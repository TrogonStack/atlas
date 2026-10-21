use std::{path::PathBuf, process::ExitCode};

use anyhow::Result;
use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use trogon_atlas_client::{
    apply, branch, client, compat, export, failure::Failure, fmt, manifest, manifest_schema,
    openslo, openslo_bindings::BindingsConfig, outcome::CommandOutcome,
    precondition::BranchEntryState, validate, view,
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
        /// for .yaml/.yml), or a board .html file saved by Studio's Export
        /// HTML. A board file's external entities are never written; apply
        /// fails if the target lacks one.
        #[arg(short = 'f', long = "filename", required = true, num_args = 1..)]
        filenames: Vec<PathBuf>,
        /// Run server-side validation for every change without persisting.
        /// Reports the request-time findings for each write, including the
        /// strategic ones (BoundedContext, Domain, Subdomain rules), which
        /// never block the apply at any severity, so it still exits 0.
        /// Findings that need the whole model in view come from `validate`
        /// and `validate-project`.
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
        /// Manifest files, directories or a board .html file, as for apply.
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
    /// Print one entity as the manifest YAML document `export` writes.
    /// Omit `@version` to read the latest version.
    Get {
        /// `<kind>/<namespace>/<slug>[@version]`, e.g. `event/shop/order.placed`,
        /// or an id that `describe` prints.
        entity: String,
    },
    /// Inspect authoritative DataProduct selections and checked model evidence.
    InspectDataProduct {
        /// Exact `data_product/<namespace>/<slug>@<version>` address.
        entity: String,
    },
    /// Query backend checked responsibility context using proto3 JSON `ListSubjectAssignmentsRequest`.
    ListSubjectAssignments {
        #[arg(long)]
        request: String,
    },
    /// Query backend checked responsibility context using proto3 JSON `ListAssigneeResponsibilitiesRequest`.
    ListAssigneeResponsibilities {
        #[arg(long)]
        request: String,
    },
    /// Query backend checked responsibility context using proto3 JSON `ListTeamMembersRequest`.
    ListTeamMembers {
        #[arg(long)]
        request: String,
    },
    /// Query backend checked responsibility context using proto3 JSON `ListPersonTeamsRequest`.
    ListPersonTeams {
        #[arg(long)]
        request: String,
    },
    /// Query backend checked responsibility context using proto3 JSON `GetTeamContactsRequest`.
    GetTeamContacts {
        #[arg(long)]
        request: String,
    },
    /// Print one entity as readable text: title, doc, schema fields, and
    /// the references it holds and receives.
    Describe {
        /// `<kind>/<namespace>/<slug>[@version]`, e.g. `command_slice/shop/place-order-slice`,
        /// or an id that `describe` prints.
        entity: String,
    },
    /// Print a namespace's structure: domains with their subdomains, then
    /// each storyboard's slices with the commands, events, read models and
    /// processors they connect.
    Tree {
        /// Namespace id, as `namespaces` lists it. Omit to print every
        /// namespace that holds entities.
        namespace: Option<String>,
    },
    /// List the namespaces the caller can see, with their ids and entity counts.
    Namespaces,
    /// Run full validation on a stored event model: the rules that need the
    /// whole model in view, which `apply --dry-run` does not run, plus the
    /// strategic chain of the model's namespace (its BoundedContext, the
    /// Subdomains it realizes, their Domains, and Domains declared in the
    /// namespace). Exits 1 when any finding is an error. With --branch,
    /// validates the branch's view of the model.
    Validate {
        /// `<namespace>/<model>[@version]`, e.g. `shop/checkout`. Omit
        /// `@version` to validate the latest version.
        model: String,
    },
    /// Validate every event model in a namespace, domain or project, plus
    /// the strategic findings (BoundedContext, Domain, Subdomain, Project
    /// rules), which print once in a store-level section. Use it after
    /// editing only problem-space entities or a BoundedContext, when there
    /// may be no event model to run `validate` on. Exits 1 when any
    /// finding is an error. With --branch, validates the branch's view.
    #[command(group(
        clap::ArgGroup::new("scope")
            .required(true)
            .multiple(false)
            .args(["namespace", "domain", "project"])
    ))]
    ValidateProject {
        /// Every event model in this namespace, problem-space namespaces
        /// (those with no event model) included.
        #[arg(long)]
        namespace: Option<String>,
        /// `<namespace>/<slug>[@version]` of a Domain; omit `@version` for
        /// the latest.
        #[arg(long)]
        domain: Option<String>,
        /// `<namespace>/<slug>[@version]` of a Project; omit `@version` for
        /// the latest.
        #[arg(long)]
        project: Option<String>,
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
        /// persisting anything, even on success. Prints how many entities
        /// would be added, changed and deleted, and the post-merge findings.
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

/// One section of a `validate-project` run: a model's findings, or the
/// store-level findings (strategic rules) that belong to no event model.
struct ProjectSection {
    /// Canonical event model id; `None` for the store-level section.
    event_model: Option<String>,
    counts: branch::SeverityCounts,
    issues: Vec<pb::ValidationIssue>,
}

struct ProjectValidation {
    scope: String,
    total: branch::SeverityCounts,
    sections: Vec<ProjectSection>,
}

impl ProjectValidation {
    fn of(scope: &str, resp: &pb::ValidateProjectResponse) -> Self {
        let sections: Vec<ProjectSection> = resp
            .reports
            .iter()
            .map(|report| ProjectSection {
                event_model: report
                    .event_model_id
                    .as_ref()
                    .filter(|id| !id.slug.is_empty())
                    .map(|id| pb::canonical::id_string(pb::EntityKind::EventModel, id)),
                counts: branch::SeverityCounts::of(&report.issues),
                issues: report.issues.clone(),
            })
            .collect();
        let total =
            sections
                .iter()
                .fold(branch::SeverityCounts::default(), |mut total, section| {
                    total.errors += section.counts.errors;
                    total.warnings += section.counts.warnings;
                    total.infos += section.counts.infos;
                    total
                });
        Self {
            scope: scope.to_string(),
            total,
            sections,
        }
    }

    fn render(&self) -> String {
        use std::fmt::Write as _;
        let mut out = format!("{}: {}\n", self.scope, self.total);
        for section in &self.sections {
            let name = section.event_model.as_deref().unwrap_or("store-level");
            let _ = writeln!(out, "{name}: {}", section.counts);
            for issue in &section.issues {
                let _ = writeln!(out, "{}", apply::format_issue(issue));
            }
        }
        out
    }

    fn to_json(&self) -> Value {
        json!({
            "scope": self.scope,
            "validation_summary": summary_json(self.total),
            "reports": self.sections.iter().map(|section| json!({
                "event_model": section.event_model,
                "validation_summary": summary_json(section.counts),
                "validation": section.issues.iter().map(apply::issue_to_json).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })
    }
}

fn summary_json(counts: branch::SeverityCounts) -> Value {
    json!({
        "errors": counts.errors,
        "warnings": counts.warnings,
        "info": counts.infos,
    })
}

/// Exit code for a command whose output reports something short of success
/// without being an error: drift a diff found, a branch's remaining
/// conflicts. Also used by `trogon-atlas fmt --check`.
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
            let files = manifest::read_paths(&filenames)?;
            let mut client = connect_planning_client(
                &endpoint,
                auth_token.as_deref(),
                rpc_timeout_secs,
                branch,
                dry_run,
            )
            .await?;
            apply::require_external(&mut client, files.external).await?;
            let resolved = manifest::load_with_tenant_types(&mut client, files.documents).await?;
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
            let data = json!({
                "lines": outcome.lines,
                "issues": outcome.issues.iter().map(apply::issue_to_json).collect::<Vec<_>>(),
            });
            Ok((ExitCode::SUCCESS, Some(data)))
        }
        Cmd::Diff { filenames } => {
            let files = manifest::read_paths(&filenames)?;
            let mut client =
                connect_client(&endpoint, auth_token.as_deref(), rpc_timeout_secs, branch).await?;
            apply::require_external(&mut client, files.external).await?;
            let resolved = manifest::load_with_tenant_types(&mut client, files.documents).await?;
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
        Cmd::InspectDataProduct { entity } => {
            let address = view::parse_address(&entity)?;
            if address.kind != pb::EntityKind::DataProduct {
                trogon_atlas_client::invalid_input!("expected a data_product address");
            }
            let version = address.version.ok_or_else(|| {
                trogon_atlas_client::failure::invalid_input("inspection requires an exact version")
            })?;
            let mut client =
                connect_client(&endpoint, auth_token.as_deref(), rpc_timeout_secs, branch).await?;
            let result = trogon_atlas_client::data_product::inspect(
                &mut client,
                pb::Id {
                    namespace: address.namespace,
                    slug: address.slug,
                    version,
                },
            )
            .await?;
            if text {
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            Ok((ExitCode::SUCCESS, Some(result)))
        }
        Cmd::ListSubjectAssignments { request } => {
            let json = serde_json::from_str(&request).map_err(|error| {
                trogon_atlas_client::failure::invalid_input(format!(
                    "invalid request JSON: {error}"
                ))
            })?;
            let request = trogon_atlas_client::responsibility::parse_request(
                "ListSubjectAssignmentsRequest",
                &json,
            )?;
            let mut client =
                connect_client(&endpoint, auth_token.as_deref(), rpc_timeout_secs, branch).await?;
            let result =
                trogon_atlas_client::responsibility::list_subject_assignments(&mut client, request)
                    .await?;
            if text {
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            Ok((ExitCode::SUCCESS, Some(result)))
        }
        Cmd::ListAssigneeResponsibilities { request } => {
            let json = serde_json::from_str(&request).map_err(|error| {
                trogon_atlas_client::failure::invalid_input(format!(
                    "invalid request JSON: {error}"
                ))
            })?;
            let request = trogon_atlas_client::responsibility::parse_request(
                "ListAssigneeResponsibilitiesRequest",
                &json,
            )?;
            let mut client =
                connect_client(&endpoint, auth_token.as_deref(), rpc_timeout_secs, branch).await?;
            let result = trogon_atlas_client::responsibility::list_assignee_responsibilities(
                &mut client,
                request,
            )
            .await?;
            if text {
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            Ok((ExitCode::SUCCESS, Some(result)))
        }
        Cmd::ListTeamMembers { request } => {
            let json = serde_json::from_str(&request).map_err(|error| {
                trogon_atlas_client::failure::invalid_input(format!(
                    "invalid request JSON: {error}"
                ))
            })?;
            let request = trogon_atlas_client::responsibility::parse_request(
                "ListTeamMembersRequest",
                &json,
            )?;
            let mut client =
                connect_client(&endpoint, auth_token.as_deref(), rpc_timeout_secs, branch).await?;
            let result =
                trogon_atlas_client::responsibility::list_team_members(&mut client, request)
                    .await?;
            if text {
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            Ok((ExitCode::SUCCESS, Some(result)))
        }
        Cmd::ListPersonTeams { request } => {
            let json = serde_json::from_str(&request).map_err(|error| {
                trogon_atlas_client::failure::invalid_input(format!(
                    "invalid request JSON: {error}"
                ))
            })?;
            let request = trogon_atlas_client::responsibility::parse_request(
                "ListPersonTeamsRequest",
                &json,
            )?;
            let mut client =
                connect_client(&endpoint, auth_token.as_deref(), rpc_timeout_secs, branch).await?;
            let result =
                trogon_atlas_client::responsibility::list_person_teams(&mut client, request)
                    .await?;
            if text {
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            Ok((ExitCode::SUCCESS, Some(result)))
        }
        Cmd::GetTeamContacts { request } => {
            let json = serde_json::from_str(&request).map_err(|error| {
                trogon_atlas_client::failure::invalid_input(format!(
                    "invalid request JSON: {error}"
                ))
            })?;
            let request = trogon_atlas_client::responsibility::parse_request(
                "GetTeamContactsRequest",
                &json,
            )?;
            let mut client =
                connect_client(&endpoint, auth_token.as_deref(), rpc_timeout_secs, branch).await?;
            let result =
                trogon_atlas_client::responsibility::get_team_contacts(&mut client, request)
                    .await?;
            if text {
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            Ok((ExitCode::SUCCESS, Some(result)))
        }
        Cmd::Get { entity } => {
            let address = view::parse_address(&entity)?;
            let mut client =
                connect_client(&endpoint, auth_token.as_deref(), rpc_timeout_secs, branch).await?;
            let entity = view::fetch_entity(&mut client, &address).await?;
            let types = view::entity_types(&mut client, &entity).await?;
            let (manifest, yaml) = view::entity_manifest(&entity, &types)?;
            if text {
                print!("{yaml}");
            }
            Ok((ExitCode::SUCCESS, Some(json!({"manifest": manifest}))))
        }
        Cmd::Describe { entity } => {
            let address = view::parse_address(&entity)?;
            let mut client =
                connect_client(&endpoint, auth_token.as_deref(), rpc_timeout_secs, branch).await?;
            let entity = view::fetch_entity(&mut client, &address).await?;
            let outgoing =
                view::fetch_references(&mut client, &entity, view::Direction::Outgoing).await?;
            let incoming =
                view::fetch_references(&mut client, &entity, view::Direction::Incoming).await?;
            let types = view::entity_types(&mut client, &entity).await?;
            let description = view::describe(&entity, &types, &outgoing, &incoming)?;
            if text {
                print!("{}", view::render_description(&description));
            }
            Ok((ExitCode::SUCCESS, Some(serde_json::to_value(&description)?)))
        }
        Cmd::Namespaces => {
            let mut client =
                connect_client(&endpoint, auth_token.as_deref(), rpc_timeout_secs, branch).await?;
            let namespaces = view::fetch_namespaces(&mut client).await?;
            if text {
                print!("{}", view::render_namespaces(&namespaces));
            }
            Ok((ExitCode::SUCCESS, Some(view::namespaces_json(&namespaces))))
        }
        Cmd::Validate { model } => {
            let address = validate::parse_model_address(&model)?;
            let mut client =
                connect_client(&endpoint, auth_token.as_deref(), rpc_timeout_secs, branch).await?;
            let (id, issues) = validate::validate_event_model(&mut client, &address).await?;
            let model = pb::canonical::id_string(pb::EntityKind::EventModel, &id);
            let severities = branch::SeverityCounts::of(&issues);
            if text {
                println!("{model}: {severities}");
                for issue in &issues {
                    println!("{}", apply::format_issue(issue));
                }
            }
            let data = json!({
                "event_model": model,
                "validation_summary": {
                    "errors": severities.errors,
                    "warnings": severities.warnings,
                    "info": severities.infos,
                },
                "validation": issues.iter().map(apply::issue_to_json).collect::<Vec<_>>(),
            });
            Ok((
                if severities.errors > 0 {
                    ExitCode::from(EXIT_FINDINGS)
                } else {
                    ExitCode::SUCCESS
                },
                Some(data),
            ))
        }
        Cmd::ValidateProject {
            namespace,
            domain,
            project,
        } => {
            let (label, scope) = match (namespace, domain, project) {
                (Some(ns), None, None) => (
                    format!("namespace {ns}"),
                    validate::ProjectScope::namespace(&ns)?,
                ),
                (None, Some(d), None) => {
                    (format!("domain {d}"), validate::ProjectScope::domain(&d)?)
                }
                (None, None, Some(p)) => {
                    (format!("project {p}"), validate::ProjectScope::project(&p)?)
                }
                _ => unreachable!("clap requires exactly one of --namespace, --domain, --project"),
            };
            let mut client =
                connect_client(&endpoint, auth_token.as_deref(), rpc_timeout_secs, branch).await?;
            let resp = validate::validate_project(&mut client, &scope).await?;
            let report = ProjectValidation::of(&label, &resp);
            if text {
                print!("{}", report.render());
            }
            Ok((
                if report.total.errors > 0 {
                    ExitCode::from(EXIT_FINDINGS)
                } else {
                    ExitCode::SUCCESS
                },
                Some(report.to_json()),
            ))
        }
        Cmd::Tree { namespace } => {
            if let Some(namespace) = &namespace {
                trogon_atlas_core::validate_id_component(namespace).map_err(|e| {
                    trogon_atlas_client::failure::invalid_input(format!("namespace {e}"))
                })?;
            }
            let mut client =
                connect_client(&endpoint, auth_token.as_deref(), rpc_timeout_secs, branch).await?;
            // Without a namespace, skip empty ones: the export fetch refuses
            // a namespace with no entities, and an empty tree says nothing.
            let namespaces = match namespace {
                Some(namespace) => vec![namespace],
                None => view::fetch_namespaces(&mut client)
                    .await?
                    .into_iter()
                    .filter(|n| n.entity_count > 0)
                    .map(|n| n.id)
                    .collect(),
            };
            let mut trees = Vec::new();
            for namespace in &namespaces {
                let entities = export::fetch_namespace_entities(&mut client, namespace).await?;
                trees.push(view::namespace_tree(namespace, &entities));
            }
            if text {
                let rendered: Vec<String> = trees.iter().map(view::render_tree).collect();
                print!("{}", rendered.join("\n"));
            }
            Ok((ExitCode::SUCCESS, Some(json!({"trees": trees}))))
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
                        let severities = branch::SeverityCounts::of(&resp.validation);
                        if text {
                            println!("{name} merged ({} entities applied)", resp.applied_count);
                            if !resp.validation.is_empty() {
                                println!("validation after merge: {severities}");
                                for issue in &resp.validation {
                                    println!("{}", apply::format_issue(issue));
                                }
                            }
                        }
                        let counts = branch::MergeCounts::of(&resp);
                        let data = json!({
                            "status": "applied",
                            "applied_count": resp.applied_count,
                            "added_count": counts.added,
                            "changed_count": counts.changed,
                            "deleted_count": counts.deleted,
                            "validation_summary": {
                                "errors": severities.errors,
                                "warnings": severities.warnings,
                                "info": severities.infos,
                            },
                            "validation": resp.validation.iter().map(apply::issue_to_json).collect::<Vec<_>>(),
                        });
                        Ok((ExitCode::SUCCESS, Some(data)))
                    }
                    Status::Validated => {
                        let counts = branch::MergeCounts::of(&resp);
                        let severities = branch::SeverityCounts::of(&resp.validation);
                        if text {
                            println!(
                                "{name} would merge cleanly (dry run): {} entities ({counts})",
                                resp.applied_count
                            );
                            println!("validation after merge: {severities}");
                            for issue in &resp.validation {
                                println!("{}", apply::format_issue(issue));
                            }
                        }
                        let data = json!({
                            "status": "validated",
                            "applied_count": resp.applied_count,
                            "added_count": counts.added,
                            "changed_count": counts.changed,
                            "deleted_count": counts.deleted,
                            "validation_summary": {
                                "errors": severities.errors,
                                "warnings": severities.warnings,
                                "info": severities.infos,
                            },
                            "validation": resp.validation.iter().map(apply::issue_to_json).collect::<Vec<_>>(),
                        });
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
                        let counts = branch::MergeCounts::of(&resp);
                        let severities = branch::SeverityCounts::of(&resp.validation);
                        if text {
                            eprintln!("{name} would leave baseline invalid, nothing merged:");
                            eprintln!("would have landed: {counts}");
                            eprintln!("validation after merge: {severities}");
                            for issue in &resp.validation {
                                eprintln!("{}", apply::format_issue(issue));
                            }
                        }
                        let data = json!({
                            "status": "invalid",
                            "added_count": counts.added,
                            "changed_count": counts.changed,
                            "deleted_count": counts.deleted,
                            "validation_summary": {
                                "errors": severities.errors,
                                "warnings": severities.warnings,
                                "info": severities.infos,
                            },
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
#[allow(clippy::unwrap_used)]
mod tests {
    use trogon_atlas_client::failure::FailureCategory as Category;
    use trogon_atlas_proto as pb;

    use super::{exit_code_for_category, ProjectValidation};

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

    fn issue(severity: pb::validation_issue::Severity, code: &str) -> pb::ValidationIssue {
        pb::ValidationIssue {
            severity: severity as i32,
            code: code.into(),
            message: "m".into(),
            ..Default::default()
        }
    }

    fn project_response() -> pb::ValidateProjectResponse {
        use pb::validation_issue::Severity;
        pb::ValidateProjectResponse {
            reports: vec![
                pb::validate_project_response::ModelReport {
                    event_model_id: Some(pb::Id {
                        namespace: "shop".into(),
                        slug: "checkout".into(),
                        version: 1,
                    }),
                    issues: vec![issue(Severity::Info, "OPEN_QUESTION_PRESENT")],
                },
                pb::validate_project_response::ModelReport {
                    event_model_id: None,
                    issues: vec![
                        issue(Severity::Warning, "DOMAIN_IN_CONTEXT_NAMESPACE"),
                        issue(Severity::Error, "DOMAIN_SLUG_MISMATCH"),
                    ],
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn project_validation_prints_per_model_and_store_level_sections() {
        let report = ProjectValidation::of("namespace shop", &project_response());
        assert_eq!(report.total.errors, 1);
        assert_eq!(report.total.warnings, 1);
        assert_eq!(report.total.infos, 1);
        let text = report.render();
        assert!(
            text.starts_with("namespace shop: 1 errors, 1 warnings, 1 info\n"),
            "{text}"
        );
        assert!(
            text.contains("event_model:shop/checkout@1: 0 errors, 0 warnings, 1 info"),
            "{text}"
        );
        assert!(
            text.contains("store-level: 1 errors, 1 warnings, 0 info"),
            "{text}"
        );
        assert!(
            text.contains("warning DOMAIN_IN_CONTEXT_NAMESPACE"),
            "{text}"
        );
    }

    #[test]
    fn project_validation_json_keeps_store_level_report_with_null_model() {
        let json = ProjectValidation::of("namespace shop", &project_response()).to_json();
        assert_eq!(json["scope"], "namespace shop");
        assert_eq!(json["validation_summary"]["errors"], 1);
        let reports = json["reports"].as_array().unwrap();
        assert_eq!(reports[0]["event_model"], "event_model:shop/checkout@1");
        assert!(reports[1]["event_model"].is_null());
        assert_eq!(
            reports[1]["validation"][0]["code"],
            "DOMAIN_IN_CONTEXT_NAMESPACE"
        );
    }
}
