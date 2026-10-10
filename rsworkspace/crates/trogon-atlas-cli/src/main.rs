use std::{path::PathBuf, process::ExitCode};

use anyhow::Result;
use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use trogon_atlas_client::{
    apply, branch, client, compat, export, failure::Failure, fmt, manifest, manifest_schema,
    openslo, openslo_bindings::BindingsConfig, outcome::CommandOutcome,
    precondition::BranchEntryState,
};
use trogon_atlas_proto as pb;

/// kubectl-style CLI for the trogon-atlas server: declarative YAML manifests
/// in, idempotent BatchMutate out.
#[derive(Parser)]
#[command(name = "trogon-atlas", version, about)]
struct Args {
    /// gRPC endpoint of the trogon-atlas server.
    #[arg(
        long,
        global = true,
        env = "TROGON_ATLAS_ENDPOINT",
        default_value = "http://127.0.0.1:50069"
    )]
    endpoint: String,

    /// Bearer token; omit only against servers allowing anonymous access.
    #[arg(long, global = true, env = "TROGON_ATLAS_AUTH_TOKEN")]
    auth_token: Option<String>,

    #[arg(
        long,
        global = true,
        env = "TROGON_ATLAS_RPC_TIMEOUT_SECS",
        default_value_t = 30
    )]
    rpc_timeout_secs: u64,

    /// Scope every RPC to this branch via the `x-trogon-atlas-branch` metadata
    /// header (Phase 1: Isolation). Omit to operate against baseline.
    #[arg(long, global = true, env = "TROGON_ATLAS_BRANCH")]
    branch: Option<String>,

    /// Shape of stdout. `json` prints exactly one `CommandOutcome` document
    /// (schema `trogon-atlas.result.v1`) and moves everything else to stderr, so a
    /// calling agent parses one value instead of scraping text. Not `-o`:
    /// export already uses that short flag for its output directory.
    #[arg(
        long,
        global = true,
        env = "TROGON_ATLAS_FORMAT",
        value_enum,
        default_value = "text"
    )]
    format: Format,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
enum Format {
    #[default]
    Text,
    Json,
}

#[derive(Subcommand)]
enum Cmd {
    /// Apply manifests: create missing entities, overwrite drifted ones,
    /// skip unchanged ones. The whole change set is one atomic BatchMutate.
    /// Exits 3 without writing anything if another writer changed an entity
    /// after it was read; run apply again to plan from fresh state.
    Apply {
        /// Manifest files or directories (directories are walked recursively
        /// for .yaml/.yml).
        #[arg(short = 'f', long = "filename", required = true, num_args = 1..)]
        filenames: Vec<PathBuf>,
        /// Run server-side validation for every change without persisting.
        #[arg(long)]
        dry_run: bool,
        /// Idempotency key for the batch this apply sends; omit to mint a
        /// fresh one. Printed to stderr either way, so a retry (or
        /// `trogon-atlas operation get`) can reuse the same id.
        #[arg(long)]
        operation_id: Option<String>,
    },
    /// Show what apply would change, as a unified diff of canonical proto
    /// JSON. Exits 1 when differences exist (kubectl diff semantics).
    Diff {
        #[arg(short = 'f', long = "filename", required = true, num_args = 1..)]
        filenames: Vec<PathBuf>,
    },
    /// Export a namespace from the live store as manifest YAML. The
    /// migration path away from push scripts: export, then `trogon-atlas diff`
    /// must report no drift.
    Export {
        #[arg(long)]
        namespace: String,
        /// Directory to write the manifest files into (created if missing).
        #[arg(short = 'o', long = "output")]
        output: PathBuf,
        /// `single`: one multi-document file per entity kind (default).
        /// `slices`: one self-contained file per slice, named from the
        /// slice's id, plus `unsliced.yaml` for everything no slice
        /// reaches. Entities referenced by more than one slice are
        /// duplicated across files so each file stands alone.
        #[arg(long, value_enum, default_value = "single")]
        layout: ExportLayoutArg,
        /// With `--layout slices`, write into a non-empty output directory
        /// anyway. Generated files are overwritten; stale ones are kept.
        #[arg(long)]
        force: bool,
    },
    /// Manage branch lifecycle (Phase 1: Isolation). Branch scope for
    /// Apply/Diff/Export is set via the top-level --branch flag, not here.
    Branch {
        #[command(subcommand)]
        cmd: BranchCmd,
    },
    /// Export a namespace's reliability entities (SLIs, SLOs, AlertPolicies,
    /// AlertNotificationTargets, and the Components they reference) as a
    /// vendor-neutral OpenSLO v1 multi-document YAML stream.
    Openslo {
        #[command(subcommand)]
        cmd: OpensloCmd,
    },
    /// Print the manifest JSON Schema (draft 2020-12), derived from the
    /// proto descriptors. No server connection is needed; wire the output
    /// into yaml-language-server for editor completion/validation.
    Schema,
    /// Canonicalize manifest YAML offline: same parser, same canonical
    /// emission `export` uses, no server connection. Prints to stdout by
    /// default (single file only); use --write or --check for more than
    /// one.
    /// Look up a previously used operation_id's durable status: whether a
    /// mutating command it guarded is still pending, applied, rejected, or
    /// abandoned (not_applied), or unknown to the server (never claimed, or
    /// it guarded a branch-scoped write, which keeps no durable receipt).
    Operation {
        #[command(subcommand)]
        cmd: OperationCmd,
    },
    Fmt {
        /// Manifest files or directories (directories are walked
        /// recursively for .yaml/.yml).
        #[arg(required = true, num_args = 1..)]
        paths: Vec<PathBuf>,
        /// Rewrite changed files in place instead of printing to stdout.
        #[arg(short = 'w', long, conflicts_with = "check")]
        write: bool,
        /// List files that would change and exit non-zero if any would
        /// (CI gate); nothing is written.
        #[arg(long, conflicts_with = "write")]
        check: bool,
        /// Canonicalize files that contain YAML comments anyway, dropping
        /// them (comments cannot survive the proto round trip).
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum OperationCmd {
    /// Report the status of one operation_id.
    Get { operation_id: String },
}

#[derive(Subcommand)]
enum OpensloCmd {
    /// Write a namespace's OpenSLO v1 export to a file.
    Export {
        #[arg(long)]
        namespace: String,
        /// File to write the rendered YAML into.
        #[arg(short = 'o', long = "output")]
        output: PathBuf,
        /// `openslo-bindings.yaml`: maps each SLI's `Signal` to the
        /// `DataSource` and the per-role `metricSource` templates
        /// (`threshold`/`good`/`bad`/`total`) OpenSLO needs but the
        /// model does not carry.
        #[arg(long)]
        bindings: PathBuf,
        /// Emit a documented placeholder `metricSource` (type `Unbound`)
        /// for any SLI whose signal has no entry in `bindings`, instead
        /// of failing the export.
        #[arg(long)]
        allow_placeholder_metrics: bool,
    },
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum ExportLayoutArg {
    Single,
    Slices,
}

impl From<ExportLayoutArg> for export::ExportLayout {
    fn from(arg: ExportLayoutArg) -> Self {
        match arg {
            ExportLayoutArg::Single => export::ExportLayout::Single,
            ExportLayoutArg::Slices => export::ExportLayout::Slices,
        }
    }
}

#[derive(Subcommand)]
enum BranchCmd {
    /// Create a new branch.
    Create {
        name: String,
        /// Free-form description stored with the branch.
        #[arg(long, default_value = "")]
        doc: String,
    },
    /// List every registered branch.
    List,
    /// Delete a branch and every delta it holds.
    Delete { name: String },
    /// Show a branch's live deltas against baseline, as a unified diff of
    /// canonical proto JSON. Exits 1 when any entry is a conflict class.
    Diff { name: String },
    /// Merge a branch onto baseline: all-or-nothing, three-way conflict
    /// detection, post-merge world validation. Exits 1 on conflicts or
    /// validation failure (nothing persisted in either case); exits 2 on
    /// dry-run failure so it is distinguishable from a real blocked merge.
    Merge {
        name: String,
        /// Run full conflict detection and post-merge validation without
        /// persisting anything, even on success.
        #[arg(long)]
        dry_run: bool,
        /// Keep the branch (now empty of deltas) after a successful merge
        /// instead of deleting it.
        #[arg(long)]
        keep_branch: bool,
        /// Land the combined result for edit/edit conflicts whose two sides
        /// changed different fields, instead of blocking on them. Conflicts
        /// where both sides changed the same field still block.
        #[arg(long)]
        auto_merge: bool,
        /// Idempotency key for this merge; omit to mint a fresh one. Printed
        /// to stderr either way.
        #[arg(long)]
        operation_id: Option<String>,
    },
    /// Rebase a branch's non-conflicting deltas forward to the current
    /// baseline. Exits 1 when conflicts remain (see the printed list).
    Update { name: String },
    /// Resolve a single conflicting entry on a branch.
    Resolve {
        name: String,
        /// Entity kind of the conflicting entry, e.g. `event`.
        #[arg(long)]
        kind: String,
        /// Namespace of the conflicting entry.
        #[arg(long)]
        namespace: String,
        /// Slug of the conflicting entry.
        #[arg(long)]
        slug: String,
        /// Version of the conflicting entry.
        #[arg(long, default_value_t = 1)]
        version: u64,
        /// Take the current baseline content, discarding the branch's edit
        /// (or remove the delta entirely if baseline no longer has the key).
        #[arg(long, conflicts_with_all = ["keep_ours", "take_merged"])]
        take_theirs: bool,
        /// Keep the branch's current value, rebasing forward so the
        /// conflict clears.
        #[arg(long, conflicts_with_all = ["take_theirs", "take_merged"])]
        keep_ours: bool,
        /// Take both sides' edits combined, rebasing forward so the conflict
        /// clears. Only available when the two sides changed different
        /// fields.
        #[arg(long, conflicts_with_all = ["take_theirs", "keep_ours"])]
        take_merged: bool,
        /// The entry state this decision was made from, exactly as
        /// `branch diff` printed it (`base=<rev>,ours=<rev>,theirs=<rev>`).
        /// The resolution is refused if any side has moved since.
        #[arg(long)]
        expect_state: BranchEntryState,
        /// Idempotency key for this resolution; omit to mint a fresh one.
        /// Printed to stderr either way.
        #[arg(long)]
        operation_id: Option<String>,
    },
}

/// Exit code for a command whose output reports something short of success
/// without being an error: drift a diff found, issues apply reported, a
/// branch's remaining conflicts. Also used by `trogon-atlas fmt --check`.
const EXIT_FINDINGS: u8 = 1;

/// A clap usage error (missing/conflicting flags) exits 2 on its own,
/// before `main` ever runs; nothing here needs to produce it.
///
/// Every other exit code is a direct function of a classified `Failure`'s
/// category, so the two can never drift apart (`docs/reference/failure-reasons.md`
/// and `docs/reference/trogon-atlas-exit-codes.md` both read from
/// `trogon_atlas_client::failure::FailureCategory`).
fn exit_code_for_category(category: trogon_atlas_client::failure::FailureCategory) -> ExitCode {
    use trogon_atlas_client::failure::FailureCategory as Category;
    ExitCode::from(match category {
        Category::StaleState => 3,
        Category::Precondition => 4,
        Category::Validation => 5,
        Category::NotFound => 6,
        Category::Unauthorized => 7,
        Category::Incompatible => 8,
        Category::OperationConflict => 9,
        Category::PartialApply => 10,
        Category::Internal => 70,
        Category::Unavailable => 75,
    })
}

/// Print the one `CommandOutcome` document a `--format json` run owes
/// stdout, as a single line so a caller never has to buffer past the first
/// newline to parse it.
fn print_outcome(outcome: &CommandOutcome) {
    let line = serde_json::to_string(outcome).unwrap_or_else(|e| {
        format!(
            r#"{{"schema":"{}","ok":false,"error":"{e}"}}"#,
            outcome.schema
        )
    });
    println!("{line}");
}

/// Resolve the `--operation-id` a mutating command was given (validating it
/// the same way the server does) or mint a fresh one when omitted, and
/// print it to stderr either way, so a caller that only watches stdout can
/// still recover it for a retry or `trogon-atlas operation get`.
fn resolve_operation_id(given: Option<String>) -> Result<String> {
    let id = match given {
        Some(id) => {
            trogon_atlas_core::validate_operation_id(&id).map_err(|e| {
                trogon_atlas_client::failure::invalid_input(format!("--operation-id {e}"))
            })?;
            id
        }
        None => trogon_atlas_core::new_operation_id(),
    };
    eprintln!("operation_id: {id}");
    Ok(id)
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let format = args.format;
    let mut operation_id = None;
    match run(args, &mut operation_id).await {
        Ok((code, data)) => {
            if format == Format::Json {
                let mut outcome = data.map_or_else(CommandOutcome::ok_empty, CommandOutcome::ok);
                if let Some(id) = operation_id {
                    outcome = outcome.with_operation_id(id);
                }
                print_outcome(&outcome);
            }
            code
        }
        Err(err) => {
            let failure = Failure::classify(&err);
            let code = exit_code_for_category(failure.category);
            if format == Format::Json {
                let mut outcome = CommandOutcome::failed(failure);
                if let Some(id) = operation_id {
                    outcome = outcome.with_operation_id(id);
                }
                print_outcome(&outcome);
            } else if failure.category == trogon_atlas_client::failure::FailureCategory::StaleState
            {
                eprintln!("stale, replan: {err:#}");
            } else {
                eprintln!("error: {err:#}");
            }
            code
        }
    }
}

fn directory_has_entries(path: &std::path::Path) -> Result<bool> {
    match std::fs::read_dir(path) {
        Ok(mut entries) => Ok(entries.next().is_some()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

async fn connect_client(
    endpoint: &str,
    auth_token: Option<&str>,
    rpc_timeout_secs: u64,
    branch: Option<String>,
) -> Result<client::Client> {
    client::connect_with_options(
        endpoint,
        auth_token,
        rpc_timeout_secs,
        client::ConnectOptions {
            request_id_prefix: Some("trogon-atlas"),
            max_message_bytes: None,
            branch,
        },
    )
    .await
}

async fn connect_mutating_client(
    endpoint: &str,
    auth_token: Option<&str>,
    rpc_timeout_secs: u64,
    branch: Option<String>,
    validate_only: bool,
) -> Result<client::Client> {
    let intent = compat::MutationIntent {
        validate_only,
        branch_scoped: branch.is_some(),
        state_preconditions: false,
    };
    connect_checked_client(endpoint, auth_token, rpc_timeout_secs, branch, intent).await
}

async fn connect_planning_client(
    endpoint: &str,
    auth_token: Option<&str>,
    rpc_timeout_secs: u64,
    branch: Option<String>,
    validate_only: bool,
) -> Result<client::Client> {
    let intent = compat::MutationIntent {
        validate_only,
        branch_scoped: branch.is_some(),
        state_preconditions: true,
    };
    connect_checked_client(endpoint, auth_token, rpc_timeout_secs, branch, intent).await
}

async fn connect_checked_client(
    endpoint: &str,
    auth_token: Option<&str>,
    rpc_timeout_secs: u64,
    branch: Option<String>,
    intent: compat::MutationIntent,
) -> Result<client::Client> {
    let mut client = connect_client(endpoint, auth_token, rpc_timeout_secs, branch).await?;
    compat::ensure_can_mutate(&mut client, intent).await?;
    Ok(client)
}

async fn run(
    args: Args,
    operation_id_out: &mut Option<String>,
) -> Result<(ExitCode, Option<Value>)> {
    let Args {
        endpoint,
        auth_token,
        rpc_timeout_secs,
        branch,
        format,
        cmd,
    } = args;
    let text = format == Format::Text;
    // Local validation (paths, flags, kind parse) runs before connect so an
    // unreachable --endpoint cannot mask authoring errors.
    match cmd {
        Cmd::Schema => {
            let schema = manifest_schema::generate()?;
            if text {
                println!("{}", serde_json::to_string_pretty(&schema)?);
            }
            Ok((ExitCode::SUCCESS, Some(schema)))
        }
        Cmd::Fmt {
            paths,
            write,
            check,
            force,
        } => {
            let mode = if write {
                fmt::OutputMode::Write
            } else if check {
                fmt::OutputMode::Check
            } else {
                fmt::OutputMode::Stdout
            };
            let outcomes = fmt::format_paths(&paths, force)?;
            match mode {
                fmt::OutputMode::Stdout => {
                    if outcomes.len() != 1 {
                        trogon_atlas_client::invalid_input!(
                            "stdout mode formats exactly one file at a time ({} resolved); \
                             use -w/--write or --check for multiple files",
                            outcomes.len()
                        );
                    }
                    if text {
                        print!("{}", outcomes[0].canonical);
                    }
                    let data = json!({"canonical": outcomes[0].canonical});
                    Ok((ExitCode::SUCCESS, Some(data)))
                }
                fmt::OutputMode::Write => {
                    let mut rewritten = Vec::new();
                    for outcome in &outcomes {
                        if outcome.changed() {
                            std::fs::write(&outcome.path, &outcome.canonical)?;
                            if text {
                                println!("{}", outcome.path.display());
                            }
                            rewritten.push(outcome.path.display().to_string());
                        }
                    }
                    if text {
                        eprintln!(
                            "{} file(s) reformatted, {} unchanged",
                            rewritten.len(),
                            outcomes.len() - rewritten.len()
                        );
                    }
                    let data = json!({
                        "rewritten": rewritten,
                        "unchanged": outcomes.len() - rewritten.len(),
                    });
                    Ok((ExitCode::SUCCESS, Some(data)))
                }
                fmt::OutputMode::Check => {
                    let changed: Vec<_> = outcomes.iter().filter(|o| o.changed()).collect();
                    if text {
                        for outcome in &changed {
                            println!("{}", outcome.path.display());
                        }
                    }
                    let would_change: Vec<_> = changed
                        .iter()
                        .map(|o| o.path.display().to_string())
                        .collect();
                    let data = json!({"would_change": would_change});
                    if changed.is_empty() {
                        Ok((ExitCode::SUCCESS, Some(data)))
                    } else {
                        if text {
                            eprintln!("{} file(s) would be reformatted", changed.len());
                        }
                        Ok((ExitCode::from(EXIT_FINDINGS), Some(data)))
                    }
                }
            }
        }
        Cmd::Apply {
            filenames,
            dry_run,
            operation_id,
        } => {
            let operation_id = resolve_operation_id(operation_id)?;
            *operation_id_out = Some(operation_id.clone());
            let documents = manifest::read_paths(&filenames)?;
            let mut client = connect_planning_client(
                &endpoint,
                auth_token.as_deref(),
                rpc_timeout_secs,
                branch,
                dry_run,
            )
            .await?;
            let resolved = manifest::load_with_tenant_types(&mut client, documents).await?;
            let plan = apply::plan(&mut client, resolved.manifests).await?;
            let outcome = apply::apply(&mut client, plan, dry_run, Some(&operation_id)).await?;
            if text {
                for line in &outcome.lines {
                    println!("{line}");
                }
                if !outcome.issues.is_empty() {
                    eprintln!("validation:");
                    for issue in &outcome.issues {
                        eprintln!("{}", apply::format_issue(issue));
                    }
                }
            }
            let has_errors = outcome.issues.iter().any(|issue| {
                matches!(
                    pb::validation_issue::Severity::try_from(issue.severity),
                    Ok(pb::validation_issue::Severity::Error)
                )
            });
            let data = json!({
                "lines": outcome.lines,
                "issues": outcome.issues.iter().map(apply::issue_to_json).collect::<Vec<_>>(),
            });
            Ok((
                if has_errors {
                    ExitCode::from(EXIT_FINDINGS)
                } else {
                    ExitCode::SUCCESS
                },
                Some(data),
            ))
        }
        Cmd::Diff { filenames } => {
            let documents = manifest::read_paths(&filenames)?;
            let mut client =
                connect_client(&endpoint, auth_token.as_deref(), rpc_timeout_secs, branch).await?;
            let resolved = manifest::load_with_tenant_types(&mut client, documents).await?;
            let plan = apply::plan(&mut client, resolved.manifests).await?;
            let (diff, changed) = apply::render_diff(&plan, &resolved.types)?;
            if text {
                print!("{diff}");
            }
            let data = json!({"diff": diff, "changed": changed});
            Ok((
                if changed {
                    ExitCode::from(EXIT_FINDINGS)
                } else {
                    ExitCode::SUCCESS
                },
                Some(data),
            ))
        }
        Cmd::Export {
            namespace,
            output,
            layout,
            force,
        } => {
            if namespace.is_empty() {
                trogon_atlas_client::invalid_input!("namespace must not be empty");
            }
            trogon_atlas_core::validate_id_component(&namespace).map_err(|e| {
                trogon_atlas_client::failure::invalid_input(format!("namespace {e}"))
            })?;
            let layout: export::ExportLayout = layout.into();
            if layout == export::ExportLayout::Slices && !force && directory_has_entries(&output)? {
                trogon_atlas_client::invalid_input!(
                    "{} is not empty; pass --force to export into it anyway (files this export does not regenerate are left in place, not deleted)",
                    output.display()
                );
            }
            let mut client =
                connect_client(&endpoint, auth_token.as_deref(), rpc_timeout_secs, branch).await?;
            let outcome = match layout {
                export::ExportLayout::Single => {
                    export::export_namespace(&mut client, &namespace).await?
                }
                export::ExportLayout::Slices => {
                    export::export_namespace_slices(&mut client, &namespace).await?
                }
            };
            std::fs::create_dir_all(&output)?;
            let mut written = Vec::new();
            for (name, content) in &outcome.files {
                let path = output.join(name);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, content)?;
                let docs = content.matches("apiVersion:").count();
                if text {
                    println!("{} ({docs} entities)", path.display());
                }
                written.push(json!({"path": path.display().to_string(), "entities": docs}));
            }
            if text && !outcome.skipped.is_empty() {
                eprintln!("NOT exported ({} entities):", outcome.skipped.len());
                for line in &outcome.skipped {
                    eprintln!("  {line}");
                }
            }
            let data = json!({"files": written, "skipped": outcome.skipped});
            Ok((
                if outcome.skipped.is_empty() {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(EXIT_FINDINGS)
                },
                Some(data),
            ))
        }
        Cmd::Openslo {
            cmd:
                OpensloCmd::Export {
                    namespace,
                    output,
                    bindings,
                    allow_placeholder_metrics,
                },
        } => {
            if namespace.is_empty() {
                trogon_atlas_client::invalid_input!("namespace must not be empty");
            }
            trogon_atlas_core::validate_id_component(&namespace).map_err(|e| {
                trogon_atlas_client::failure::invalid_input(format!("namespace {e}"))
            })?;
            let bindings_config = BindingsConfig::load(&bindings)?;
            let mut client =
                connect_client(&endpoint, auth_token.as_deref(), rpc_timeout_secs, branch).await?;
            let outcome = openslo::export_namespace_openslo(
                &mut client,
                &namespace,
                &bindings_config,
                allow_placeholder_metrics,
            )
            .await?;
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&output, &outcome.yaml)?;
            let docs = outcome.yaml.matches("apiVersion:").count();
            if text {
                println!("{} ({docs} documents)", output.display());
                if !outcome.skipped.is_empty() {
                    eprintln!("NOT exported ({} entities):", outcome.skipped.len());
                    for line in &outcome.skipped {
                        eprintln!("  {line}");
                    }
                }
            }
            let data = json!({
                "path": output.display().to_string(),
                "documents": docs,
                "skipped": outcome.skipped,
            });
            Ok((
                if outcome.skipped.is_empty() {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(EXIT_FINDINGS)
                },
                Some(data),
            ))
        }
        Cmd::Operation {
            cmd: OperationCmd::Get { operation_id },
        } => {
            trogon_atlas_core::validate_operation_id(&operation_id).map_err(|e| {
                trogon_atlas_client::failure::invalid_input(format!("operation_id {e}"))
            })?;
            let mut client =
                connect_client(&endpoint, auth_token.as_deref(), rpc_timeout_secs, branch).await?;
            let resp =
                trogon_atlas_client::operation::get_operation(&mut client, &operation_id).await?;
            let status = trogon_atlas_client::operation::status_label(resp.status);
            if text {
                println!("{operation_id}: {status}");
                if !resp.changeset_id.is_empty() {
                    println!("  changeset_id: {}", resp.changeset_id);
                }
                if !resp.rejection_code.is_empty() {
                    println!(
                        "  rejection: {} {}",
                        resp.rejection_code, resp.rejection_message
                    );
                }
            }
            let data = json!({
                "operation_id": operation_id,
                "status": status,
                "changeset_id": resp.changeset_id,
                "rejection_code": resp.rejection_code,
                "rejection_message": resp.rejection_message,
            });
            Ok((ExitCode::SUCCESS, Some(data)))
        }
        Cmd::Branch { cmd } => match cmd {
            BranchCmd::Create { name, doc } => {
                branch::validate_branch_name(&name)?;
                let mut client = connect_mutating_client(
                    &endpoint,
                    auth_token.as_deref(),
                    rpc_timeout_secs,
                    branch,
                    false,
                )
                .await?;
                let info = branch::create_branch(&mut client, &name, &doc).await?;
                if text {
                    println!("{} created at {}", info.name, info.created_at);
                }
                let data = json!({"name": info.name, "created_at": info.created_at});
                Ok((ExitCode::SUCCESS, Some(data)))
            }
            BranchCmd::List => {
                let mut client =
                    connect_client(&endpoint, auth_token.as_deref(), rpc_timeout_secs, branch)
                        .await?;
                let branches = branch::list_branches(&mut client).await?;
                if text {
                    if branches.is_empty() {
                        println!("no branches");
                    } else {
                        for b in &branches {
                            println!("{}\t{} deltas\t{}", b.name, b.delta_count, b.created_at);
                        }
                    }
                }
                let data = json!({
                    "branches": branches.iter().map(|b| json!({
                        "name": b.name,
                        "delta_count": b.delta_count,
                        "created_at": b.created_at,
                    })).collect::<Vec<_>>(),
                });
                Ok((ExitCode::SUCCESS, Some(data)))
            }
            BranchCmd::Delete { name } => {
                branch::validate_branch_name(&name)?;
                let mut client = connect_mutating_client(
                    &endpoint,
                    auth_token.as_deref(),
                    rpc_timeout_secs,
                    branch,
                    false,
                )
                .await?;
                branch::delete_branch(&mut client, &name).await?;
                if text {
                    println!("{name} deleted");
                }
                Ok((ExitCode::SUCCESS, Some(json!({"name": name}))))
            }
            BranchCmd::Diff { name } => {
                branch::validate_branch_name(&name)?;
                let mut client =
                    connect_client(&endpoint, auth_token.as_deref(), rpc_timeout_secs, branch)
                        .await?;
                let entries = branch::diff_branch(&mut client, &name).await?;
                let has_conflict = entries.iter().any(|e| {
                    matches!(
                        pb::branch_diff_entry::Status::try_from(e.status),
                        Ok(pb::branch_diff_entry::Status::ConflictEditEdit
                            | pb::branch_diff_entry::Status::ConflictEditDelete
                            | pb::branch_diff_entry::Status::ConflictDeleteEdit)
                    )
                });
                let diff = branch::render_diff(&entries)?;
                if text {
                    print!("{diff}");
                }
                let data = json!({"diff": diff, "has_conflict": has_conflict});
                Ok((
                    if has_conflict {
                        ExitCode::from(EXIT_FINDINGS)
                    } else {
                        ExitCode::SUCCESS
                    },
                    Some(data),
                ))
            }
            BranchCmd::Merge {
                name,
                dry_run,
                keep_branch,
                auto_merge,
                operation_id,
            } => {
                let operation_id = resolve_operation_id(operation_id)?;
                *operation_id_out = Some(operation_id.clone());
                branch::validate_branch_name(&name)?;
                let mut client = connect_mutating_client(
                    &endpoint,
                    auth_token.as_deref(),
                    rpc_timeout_secs,
                    branch,
                    dry_run,
                )
                .await?;
                let options = branch::MergeOptions {
                    dry_run,
                    keep_branch,
                    auto_merge,
                };
                let resp =
                    branch::merge_branch(&mut client, &name, options, Some(&operation_id)).await?;
                use pb::merge_branch_response::Status;
                match Status::try_from(resp.status).unwrap_or(Status::Unspecified) {
                    Status::Applied => {
                        if text {
                            println!("{name} merged ({} entities applied)", resp.applied_count);
                        }
                        let data =
                            json!({"status": "applied", "applied_count": resp.applied_count});
                        Ok((ExitCode::SUCCESS, Some(data)))
                    }
                    Status::Validated => {
                        if text {
                            println!(
                                "{name} would merge cleanly ({} entities, dry run)",
                                resp.applied_count
                            );
                        }
                        let data =
                            json!({"status": "validated", "applied_count": resp.applied_count});
                        Ok((ExitCode::SUCCESS, Some(data)))
                    }
                    Status::Conflicts => {
                        let diff = branch::render_diff(&resp.conflicts)?;
                        if text {
                            eprintln!("{name} has conflicts, nothing merged:");
                            print!("{diff}");
                        }
                        let data = json!({"status": "conflicts", "diff": diff});
                        Ok((ExitCode::from(EXIT_FINDINGS), Some(data)))
                    }
                    Status::Invalid => {
                        if text {
                            eprintln!("{name} would leave baseline invalid, nothing merged:");
                            for issue in &resp.validation {
                                eprintln!("{}", apply::format_issue(issue));
                            }
                        }
                        let data = json!({
                            "status": "invalid",
                            "validation": resp.validation.iter().map(apply::issue_to_json).collect::<Vec<_>>(),
                        });
                        Ok((ExitCode::from(EXIT_FINDINGS), Some(data)))
                    }
                    Status::Unspecified => {
                        anyhow::bail!("{name} merge returned an unspecified status")
                    }
                }
            }
            BranchCmd::Update { name } => {
                branch::validate_branch_name(&name)?;
                let mut client = connect_mutating_client(
                    &endpoint,
                    auth_token.as_deref(),
                    rpc_timeout_secs,
                    branch,
                    false,
                )
                .await?;
                let resp = branch::update_branch(&mut client, &name).await?;
                let conflicts_empty = resp.conflicts.is_empty();
                let diff = branch::render_diff(&resp.conflicts)?;
                if text {
                    println!("{name}: {} entries rebased", resp.rebased_count);
                    if !conflicts_empty {
                        eprintln!("{} conflicts remain:", resp.conflicts.len());
                        print!("{diff}");
                    }
                }
                let data = json!({"rebased_count": resp.rebased_count, "diff": diff});
                Ok((
                    if conflicts_empty {
                        ExitCode::SUCCESS
                    } else {
                        ExitCode::from(EXIT_FINDINGS)
                    },
                    Some(data),
                ))
            }
            BranchCmd::Resolve {
                name,
                kind,
                namespace,
                slug,
                version,
                take_theirs,
                keep_ours,
                take_merged,
                expect_state,
                operation_id,
            } => {
                let operation_id = resolve_operation_id(operation_id)?;
                *operation_id_out = Some(operation_id.clone());
                if usize::from(take_theirs) + usize::from(keep_ours) + usize::from(take_merged) != 1
                {
                    trogon_atlas_client::invalid_input!(
                        "specify exactly one of --take-theirs, --keep-ours or --take-merged"
                    );
                }
                if version < 1 {
                    trogon_atlas_client::invalid_input!(
                        "version must be a positive integer, got {version}"
                    );
                }
                branch::validate_branch_name(&name)?;
                let kind = pb::canonical::parse_kind(&kind).ok_or_else(|| {
                    trogon_atlas_client::failure::invalid_input(format!(
                        "unknown entity kind: {kind}"
                    ))
                })?;
                let entity_ref = pb::EntityRef {
                    kind: kind as i32,
                    id: Some(pb::Id {
                        namespace,
                        slug,
                        version,
                    }),
                };
                let resolution = if take_theirs {
                    branch::Resolution::TakeTheirs
                } else if take_merged {
                    branch::Resolution::TakeMerged
                } else {
                    branch::Resolution::KeepOurs
                };
                let mut client = connect_planning_client(
                    &endpoint,
                    auth_token.as_deref(),
                    rpc_timeout_secs,
                    branch,
                    false,
                )
                .await?;
                branch::resolve_branch_entry(
                    &mut client,
                    &name,
                    entity_ref,
                    resolution,
                    &expect_state,
                    Some(&operation_id),
                )
                .await?;
                if text {
                    println!("{name} resolved");
                }
                Ok((ExitCode::SUCCESS, Some(json!({"name": name}))))
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use trogon_atlas_client::failure::FailureCategory as Category;

    use super::exit_code_for_category;

    // `ExitCode` has no public accessor for the raw code, so compare
    // through `Debug` formatting against `ExitCode::from(expected)` built
    // the same way the production code builds it, rather than reaching
    // into a private field.
    #[test]
    fn every_category_maps_to_its_documented_exit_code() {
        use std::process::ExitCode;

        let cases = [
            (Category::StaleState, 3),
            (Category::Precondition, 4),
            (Category::Validation, 5),
            (Category::NotFound, 6),
            (Category::Unauthorized, 7),
            (Category::Incompatible, 8),
            (Category::OperationConflict, 9),
            (Category::PartialApply, 10),
            (Category::Internal, 70),
            (Category::Unavailable, 75),
        ];
        for (category, expected) in cases {
            assert_eq!(
                format!("{:?}", exit_code_for_category(category)),
                format!("{:?}", ExitCode::from(expected)),
                "{category:?} should exit {expected}"
            );
        }
    }
}
