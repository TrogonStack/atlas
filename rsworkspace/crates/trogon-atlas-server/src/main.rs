use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tonic::{
    service::interceptor::InterceptedService,
    transport::{Identity, Server, ServerTlsConfig},
};
use trogon_atlas_core::WriterRole;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelService;
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    git_mirror::{GitMirror, MirrorAuthor},
};
use trogon_atlas_store::{
    nats::{NatsStore, NatsStoreConfig},
    refs::{entity_id, entity_kind},
    ConflictResolution, LegacyFieldsShape, MigrationMode, Store, StoreOpenMode,
};

#[derive(Parser, Debug, Clone)]
#[command(name = "trogon-atlas-server")]
struct Args {
    #[command(subcommand)]
    cmd: Option<Cmd>,

    #[arg(long, env = "TROGON_ATLAS_LISTEN", default_value = "0.0.0.0:50069")]
    listen: String,
    /// Storage backend. Only `nats` is supported (durable, clustered; requires `JetStream`).
    #[arg(long, env = "TROGON_ATLAS_STORE", default_value = "nats")]
    store: String,
    /// This process's posture in the single-writer lease (see
    /// `docs/explanation/single-writer.md`): `writer` or `standby` both
    /// compete for the lease and may end up holding it; `reader` never
    /// competes and only ever serves reads.
    #[arg(long, env = "TROGON_ATLAS_ROLE", default_value = "writer")]
    role: WriterRole,
    #[arg(
        long,
        env = "TROGON_ATLAS_NATS_URL",
        default_value = "nats://127.0.0.1:4222"
    )]
    nats_url: String,
    /// KV bucket holding entities.
    #[arg(
        long,
        env = "TROGON_ATLAS_NATS_BUCKET",
        default_value = "trogon-atlas-entities"
    )]
    nats_bucket: String,
    /// `JetStream` stream holding the change log.
    #[arg(
        long,
        env = "TROGON_ATLAS_NATS_STREAM",
        default_value = "TROGON_ATLAS_CHANGES"
    )]
    nats_stream: String,
    /// Subject root for change events; the stream listens on `{root}.>`.
    #[arg(
        long,
        env = "TROGON_ATLAS_NATS_SUBJECT_ROOT",
        default_value = "atlas.changes"
    )]
    nats_subject_root: String,
    /// Discard change records older than this many days. 0 keeps them forever.
    #[arg(long, env = "TROGON_ATLAS_CHANGES_MAX_AGE_DAYS", default_value_t = 90)]
    changes_max_age_days: u64,
    /// Cap the change log at this many records. 0 means unlimited.
    #[arg(
        long,
        env = "TROGON_ATLAS_CHANGES_MAX_MSGS",
        default_value_t = 1_000_000
    )]
    changes_max_msgs: i64,
    /// Cap the change log at this many bytes. 0 means unlimited.
    #[arg(long, env = "TROGON_ATLAS_CHANGES_MAX_BYTES", default_value_t = 0)]
    changes_max_bytes: i64,
    /// How long a settled operation receipt (idempotency claim for a mutation
    /// carrying an `operation_id`) stays readable via `GetOperation`.
    #[arg(
        long = "operations-retention",
        env = "TROGON_ATLAS_OPERATIONS_RETENTION_SECS",
        default_value_t = 7 * 24 * 60 * 60
    )]
    operations_retention_secs: u64,
    #[arg(long, env = "TROGON_ATLAS_GIT_MIRROR")]
    git_mirror: Option<PathBuf>,
    #[arg(
        long,
        env = "TROGON_ATLAS_GIT_AUTHOR_NAME",
        default_value = "trogon-atlas-server"
    )]
    git_author_name: String,
    #[arg(
        long,
        env = "TROGON_ATLAS_GIT_AUTHOR_EMAIL",
        default_value = "trogon-atlas@local"
    )]
    git_author_email: String,
    /// `host:port` to bind the Prometheus exporter on. Omit to disable metrics.
    /// Binding a non-loopback address also requires --metrics-allow-external.
    #[arg(long, env = "TROGON_ATLAS_METRICS_LISTEN")]
    metrics_listen: Option<String>,
    /// Explicitly allow the Prometheus /metrics endpoint to bind on a
    /// non-loopback interface. Required when --metrics-listen resolves to
    /// a public or private-network address. Loopback binding is always
    /// permitted and does not require this flag. Set
    /// `TROGON_ATLAS_METRICS_ALLOW_EXTERNAL=true` to enable.
    #[arg(
        long,
        env = "TROGON_ATLAS_METRICS_ALLOW_EXTERNAL",
        default_value_t = false
    )]
    metrics_allow_external: bool,
    /// Shared bearer token required on every RPC. Deprecated in favor of
    /// `--auth-tokens-file`. When set without a tokens file, the token is
    /// treated as belonging to a synthetic principal named "legacy-admin" with
    /// Admin role. Omit both this and `--auth-tokens-file` only when
    /// `--insecure-allow-anonymous` is also set.
    #[arg(long, env = "TROGON_ATLAS_AUTH_TOKEN")]
    auth_token: Option<String>,
    /// Path to a TOML token registry file. When set, overrides `--auth-token`.
    /// See tokens.example.toml for the file format.
    #[arg(long, env = "TROGON_ATLAS_AUTH_TOKENS_FILE")]
    auth_tokens_file: Option<PathBuf>,
    /// Seconds between re-reads of the token registry, so a key can be issued
    /// or revoked without restarting the server. `0` pins the registry to
    /// whatever startup read.
    ///
    /// This is a revocation latency, not just a polling knob: it bounds how
    /// long a key deleted from the file keeps working.
    #[arg(
        long,
        env = "TROGON_ATLAS_AUTH_TOKENS_RELOAD_INTERVAL",
        default_value_t = trogon_atlas_server::token_reload::ReloadInterval::default()
    )]
    auth_tokens_reload_interval: trogon_atlas_server::token_reload::ReloadInterval,
    /// Seconds a cached namespace registry view stays usable before it is
    /// re-read. `0` pins it until the next registry write this process
    /// serves, which is the behaviour before this flag existed.
    ///
    /// This bounds how long one replica may keep serving an ownership
    /// answer that another replica has already changed, so leave it non-zero
    /// on any deployment with more than one server.
    #[arg(
        long,
        env = "TROGON_ATLAS_NAMESPACE_REGISTRY_RELOAD_INTERVAL",
        default_value_t = trogon_atlas_server::token_reload::ReloadInterval::default()
    )]
    namespace_registry_reload_interval: trogon_atlas_server::token_reload::ReloadInterval,
    /// Explicit opt-in to running without auth. Required when neither
    /// `--auth-token` nor `--auth-tokens-file` is set; otherwise the server
    /// refuses to start. Intended for local development and CI.
    #[arg(
        long,
        env = "TROGON_ATLAS_INSECURE_ALLOW_ANONYMOUS",
        default_value_t = false
    )]
    insecure_allow_anonymous: bool,
    /// Reject mutating RPCs (PutEntity, DeleteEntity, BatchMutate,
    /// RetargetReferences, DeleteByQuery) that carry no
    /// `x-trogon-atlas-branch` context, unless the caller has the Admin role.
    /// Merging a branch remains the way baseline changes under protection.
    /// Default false: byte-identical to today's behavior. See
    /// `docs/explanation/branching.md`, "Baseline protection".
    #[arg(long, env = "TROGON_ATLAS_PROTECT_BASELINE", default_value_t = false)]
    protect_baseline: bool,
    /// Protect baseline in these namespaces only, instead of all of them.
    /// Accepts exact names and a trailing wildcard (`orders`, `shop-*`), and
    /// narrows `--protect-baseline` when both are set. Namespaces outside the
    /// list keep accepting direct baseline writes, which is what lets a
    /// reviewed context and a sandbox live in one store. See
    /// `docs/explanation/authorization.md`, "Namespace scope".
    #[arg(
        long,
        env = "TROGON_ATLAS_PROTECT_BASELINE_NAMESPACES",
        value_delimiter = ','
    )]
    protect_baseline_namespaces: Vec<String>,
    /// PEM certificate chain for serving TLS. Requires --tls-key.
    #[arg(long, env = "TROGON_ATLAS_TLS_CERT")]
    tls_cert: Option<PathBuf>,
    /// PEM private key for serving TLS. Requires --tls-cert.
    #[arg(long, env = "TROGON_ATLAS_TLS_KEY")]
    tls_key: Option<PathBuf>,
    /// Maximum decoded gRPC message size in bytes.
    #[arg(
        long,
        env = "TROGON_ATLAS_MAX_MESSAGE_BYTES",
        default_value_t = 4 * 1024 * 1024
    )]
    max_message_bytes: usize,
    /// Maximum concurrent in-flight RPCs per connection.
    #[arg(long, env = "TROGON_ATLAS_CONCURRENCY_LIMIT", default_value_t = 32)]
    concurrency_limit: usize,
    /// Server-side RPC timeout in seconds. The server aborts any RPC that
    /// runs longer than this, returning `DEADLINE_EXCEEDED`. Clients can
    /// still impose a tighter deadline via the `grpc-timeout` header.
    /// `0` disables the server-side timeout entirely.
    #[arg(long, env = "TROGON_ATLAS_RPC_TIMEOUT_SECS", default_value_t = 60)]
    rpc_timeout_secs: u64,
    /// Generative analyzer: `disabled` or `anthropic`. Independent of Jev.
    #[arg(long, env = "TROGON_ATLAS_LLM", default_value = "disabled")]
    llm: String,
    /// API key for the selected LLM provider. Prefers `TROGON_ATLAS_LLM_API_KEY`
    /// so the variable is provider-agnostic; falls back to
    /// `ANTHROPIC_API_KEY` for compatibility with existing tooling.
    #[arg(long, env = "TROGON_ATLAS_LLM_API_KEY")]
    llm_api_key: Option<String>,
    /// Override the default model for the selected provider. The default
    /// is a cost-balanced Sonnet-class model; see llm.rs.
    #[arg(long, env = "TROGON_ATLAS_LLM_MODEL")]
    llm_model: Option<String>,
    /// Override the provider base URL (e.g. to route through a proxy).
    /// Prefers `TROGON_ATLAS_LLM_BASE_URL`; falls back to `ANTHROPIC_BASE_URL`.
    #[arg(long, env = "TROGON_ATLAS_LLM_BASE_URL")]
    llm_base_url: Option<String>,
    /// Character budget for analysis prompts before doc strings are dropped.
    #[arg(long, env = "TROGON_ATLAS_LLM_PROMPT_BUDGET", default_value_t = 24_000)]
    llm_prompt_budget: usize,
    /// Maximum tokens the LLM provider may return per request.
    #[arg(long, env = "TROGON_ATLAS_LLM_MAX_TOKENS", default_value_t = 4096)]
    llm_max_tokens: u32,
    /// Per-request timeout for LLM calls, in seconds.
    #[arg(
        long,
        env = "TROGON_ATLAS_LLM_REQUEST_TIMEOUT_SECS",
        default_value_t = 60
    )]
    llm_request_timeout_secs: u64,
    /// Maximum concurrent in-flight LLM calls per process.
    #[arg(long, env = "TROGON_ATLAS_LLM_CONCURRENCY", default_value_t = 4)]
    llm_concurrency: usize,
    /// Enable Jev field-source evaluation before the generative analyzer.
    #[arg(long, env = "TROGON_ATLAS_JEV", default_value_t = false)]
    jev: bool,
    /// Gateway API key. Falls back to AI_GATEWAY_API_KEY.
    #[arg(long, env = "TROGON_ATLAS_JEV_API_KEY", hide_env_values = true)]
    jev_api_key: Option<String>,
    #[arg(
        long,
        env = "TROGON_ATLAS_JEV_BASE_URL",
        default_value = "https://ai-gateway.vercel.sh"
    )]
    jev_base_url: String,
    #[arg(long, env = "TROGON_ATLAS_JEV_TIMEOUT_SECS", default_value_t = 8)]
    jev_timeout_secs: u64,
    #[arg(long, env = "TROGON_ATLAS_JEV_CONCURRENCY", default_value_t = 4)]
    jev_concurrency: usize,
    #[arg(long, env = "TROGON_ATLAS_JEV_CACHE_CAPACITY", default_value_t = 128)]
    jev_cache_capacity: usize,
    #[arg(long, env = "TROGON_ATLAS_JEV_CACHE_TTL_SECS", default_value_t = 300)]
    jev_cache_ttl_secs: u64,
    /// Minimum selected-option probability; calibrate against labeled cases.
    #[arg(long, env = "TROGON_ATLAS_JEV_MIN_PROBABILITY", default_value_t = 0.95)]
    jev_min_probability: f64,
    /// SpiceDB endpoint, e.g. `http://spicedb:50051`. When set, namespace
    /// visibility is decided by SpiceDB rather than by the registry's own
    /// `parent` column, which is what makes a namespace shared between two
    /// owners expressible at all.
    ///
    /// The registry stays the source of truth for whether a namespace exists
    /// and what its id is; only the permission question moves.
    #[arg(long, env = "TROGON_ATLAS_SPICEDB_ENDPOINT")]
    spicedb_endpoint: Option<String>,

    /// Preshared key SpiceDB was started with. Required whenever
    /// `--spicedb-endpoint` is set.
    #[arg(long, env = "TROGON_ATLAS_SPICEDB_PRESHARED_KEY")]
    spicedb_preshared_key: Option<String>,

    /// How fresh a permission check has to be.
    ///
    /// `minimize-latency` reads from whichever replica answers first, floored
    /// by this process's own writes so it always sees the grants it just
    /// made. `fully-consistent` quorum-reads every check, which is the
    /// setting for a deployment where a grant revoked through another process
    /// must stop working immediately rather than at the end of SpiceDB's
    /// revision quantization window.
    #[arg(
        long,
        env = "TROGON_ATLAS_SPICEDB_FRESHNESS",
        default_value = "minimize-latency"
    )]
    spicedb_freshness: trogon_atlas_server::spicedb::Freshness,

    /// Skip the startup reconcile that installs the SpiceDB schema and
    /// re-derives every grant from the registry and the token file.
    ///
    /// The reconcile is idempotent and cheap, and running it is what keeps
    /// the token file authoritative for membership. Skip it only when this
    /// process must not write to SpiceDB, and run `sync-spicedb` out of band
    /// instead.
    #[arg(
        long,
        env = "TROGON_ATLAS_SPICEDB_SKIP_STARTUP_SYNC",
        default_value_t = false
    )]
    spicedb_skip_startup_sync: bool,

    /// Seconds to wait for in-flight RPCs to drain during graceful shutdown
    /// before aborting them. 0 exits immediately without draining.
    #[arg(long, env = "TROGON_ATLAS_SHUTDOWN_DRAIN_SECS", default_value_t = 30)]
    shutdown_drain_secs: u64,
    /// Standard-base64, 32-byte key sealing `ListChanges` pagination cursors
    /// with ChaCha20-Poly1305. Required whenever more than one
    /// trogon-atlas-server process can serve the same poll (a standby, a
    /// reader role, or horizontal scale behind one logical service):
    /// without it, each process generates its own random key at startup, so
    /// a cursor minted by one process fails to open on another, and that
    /// caller's poll is refused with `CURSOR_REJECTED` and must restart.
    /// Safe to leave unset for a single-process deployment.
    #[arg(long, env = "TROGON_ATLAS_CHANGE_CURSOR_KEY", hide_env_values = true)]
    change_cursor_key: Option<String>,
}

#[derive(Subcommand, Debug, Clone)]
enum Cmd {
    /// Write every entity in the configured store into a git mirror tree,
    /// plus the namespace registry, and commit the result.
    ///
    /// The inverse of `import-git`, and the only way to produce a mirror
    /// without having run a server with `--git-mirror` since the store was
    /// created. Deletes mirror files whose entity is no longer in the store,
    /// so the commit is a snapshot rather than an accumulation.
    ExportGit {
        /// Path to the git mirror working tree. Created and `git init`ed if
        /// it does not exist.
        path: PathBuf,
    },
    /// Walk a git mirror tree and re-import every entity into the
    /// configured store (`--store=nats` to restore a live deployment).
    ImportGit {
        /// Path to the git mirror working tree (root with kind/namespace/slug/version.binpb).
        path: PathBuf,
        /// Also put back the namespace registry from the mirror's
        /// `namespaces.json`, restoring who owns what.
        ///
        /// Off by default because it decides what every caller can see. An
        /// import without it leaves the entities unowned and says so; an
        /// import with it trusts the snapshot on disk. Run it against an
        /// empty registry: rows already present are left exactly as they are,
        /// so this never re-owns a namespace out from under its current
        /// parent.
        #[arg(long)]
        restore_namespaces: bool,
    },
    /// Apply a `BatchMutateRequest` fixture against the configured store.
    /// Accepts prost-encoded `.binpb` fixtures.
    ///
    /// Encode a textproto fixture first via:
    ///   protoc --encode=trogonatlas.api.eventmodel.v1alpha1.BatchMutateRequest \
    ///     -I proto \
    ///     proto/trogonatlas/api/eventmodel/v1alpha1/service.proto \
    ///     < fixture.textproto > fixture.binpb
    Seed {
        /// Path to the prost-encoded `BatchMutateRequest` (.binpb).
        #[arg(long)]
        fixture: PathBuf,
    },
    /// Compare the configured store against a git mirror working tree
    /// and report every entity that diverges between the two. Exits
    /// non-zero if any drift is found.
    ///
    /// Use after direct NATS KV surgery to confirm the mirror is back
    /// in sync, or on a schedule to catch silent divergence.
    Reconcile {
        /// Path to the git mirror working tree
        /// (root containing kind/namespace/slug/version.binpb files).
        #[arg(long)]
        mirror: PathBuf,
    },
    /// Install the SpiceDB schema and write the grant implied by every
    /// namespace registry row and every principal binding in the token file.
    ///
    /// Idempotent, and the repair path for a namespace whose registry row
    /// landed but whose grant did not. The server does this at startup too;
    /// this subcommand is for doing it without starting a server.
    SyncSpicedb {
        /// Report what would be written and exit without writing it.
        #[arg(long)]
        dry_run: bool,
    },
    /// Give every namespace that predates the namespace registry a registry
    /// row, adopting its name as its id so no entity key changes.
    ///
    /// Idempotent, and safe to run against a live store: it only ever adds
    /// rows for namespaces that have none. Run it once after upgrading a
    /// deployment that was created before ownership existed. Until it has
    /// run, an owner-scoped principal sees nothing, because a namespace with
    /// no registry row has no owner.
    BackfillNamespaces {
        /// Owner every backfilled namespace is assigned to. Namespaces can be
        /// redistributed afterwards with `MoveNamespace`, which is one
        /// registry row and no entity writes.
        #[arg(long, default_value = trogon_atlas_server::ownership::DEFAULT_OWNER)]
        parent: String,
        /// Report what would be written and exit without writing it.
        #[arg(long)]
        dry_run: bool,
    },
    /// Find baseline batches that stopped part way and roll each one back
    /// or forward until it is fully applied or fully absent. Prints a JSON
    /// report and exits non-zero unless every batch found converged.
    ///
    /// The server runs the same repair at startup and on a timer; this is
    /// for checking or repairing without starting a server.
    RecoverBatches {
        /// Report what would be repaired and exit without repairing it.
        #[arg(long)]
        report_only: bool,
        /// Leave journal entries younger than this alone, so a pass run
        /// beside a live server does not act on a batch still in flight.
        #[arg(long, default_value_t = 0)]
        min_age_secs: u64,
    },
    /// Move the retired `fields` list on every Event, Command and ReadModel
    /// into `schema`, in baseline entities and branch deltas. Reports what
    /// it would change unless `--apply` is given, and exits non-zero while
    /// an entity carries both and they disagree.
    ///
    /// A server refuses to open the store until an `--apply` run leaves no
    /// row in the retired layout. Idempotent. Run it with no server writing
    /// to the store.
    MigrateLegacyFields {
        /// Write the changes instead of only reporting them.
        #[arg(long)]
        apply: bool,
        /// With `--apply`, resolve a disagreement by keeping `schema` and
        /// discarding the retired list.
        #[arg(long, requires = "apply")]
        keep_schema_on_conflict: bool,
    },
}

async fn build_store(args: &Args) -> Result<Arc<dyn Store>> {
    Ok(Arc::new(
        build_nats_store(args, StoreOpenMode::Serve).await?,
    ))
}

async fn build_nats_store(args: &Args, open_mode: StoreOpenMode) -> Result<NatsStore> {
    match args.store.as_str() {
        "nats" => {
            let mut config = NatsStoreConfig::new(&args.nats_url);
            config.bucket = args.nats_bucket.clone();
            config.stream = args.nats_stream.clone();
            config.subject_root = args.nats_subject_root.clone();
            config.changes_max_age = match args.changes_max_age_days {
                0 => None,
                days => Some(Duration::from_secs(days * 24 * 60 * 60)),
            };
            config.changes_max_msgs = match args.changes_max_msgs {
                0 => None,
                n => Some(n),
            };
            config.changes_max_bytes = match args.changes_max_bytes {
                0 => None,
                n => Some(n),
            };
            config.operations_retention = Duration::from_secs(args.operations_retention_secs);
            config.role = args.role;
            config.open_mode = open_mode;
            NatsStore::connect_with(config)
                .await
                .map_err(|e| anyhow::anyhow!("nats connect {}: {e}", args.nats_url))
        }
        other => anyhow::bail!(
            "--store={other} is not supported; only `nats` is accepted. \
             Set TROGON_ATLAS_STORE=nats and ensure NATS JetStream is available."
        ),
    }
}

/// Wait for either SIGINT (`ctrl_c`) or SIGTERM and resolve. On non-unix
/// platforms only `ctrl_c` is wired; SIGTERM is unix-specific.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(err) => {
                tracing::error!(%err, "failed to install SIGTERM handler; relying on SIGINT only");
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("SIGINT received; beginning graceful shutdown");
            }
            _ = term.recv() => {
                tracing::info!("SIGTERM received; beginning graceful shutdown");
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("SIGINT received; beginning graceful shutdown");
    }
}

/// Wait until the in-flight counter drops to zero or the deadline elapses.
/// Wakes immediately on each counter update via a watch channel instead of
/// polling with fixed sleeps.
async fn await_inflight_drain(
    counter: &std::sync::atomic::AtomicI64,
    mut drain_rx: tokio::sync::watch::Receiver<i64>,
    deadline: std::time::Duration,
) {
    let start = std::time::Instant::now();
    loop {
        let inflight = counter.load(std::sync::atomic::Ordering::Relaxed);
        if inflight <= 0 {
            tracing::info!(
                drain_ms = %start.elapsed().as_millis(),
                "in-flight RPCs drained"
            );
            return;
        }
        let remaining_deadline = deadline.saturating_sub(start.elapsed());
        if remaining_deadline.is_zero() {
            tracing::warn!(
                inflight,
                drain_ms = %start.elapsed().as_millis(),
                "drain deadline elapsed; aborting outstanding RPCs"
            );
            return;
        }
        tokio::select! {
            _ = drain_rx.changed() => {}
            () = tokio::time::sleep(remaining_deadline) => {
                let inflight = counter.load(std::sync::atomic::Ordering::Relaxed);
                if inflight > 0 {
                    tracing::warn!(
                        inflight,
                        drain_ms = %start.elapsed().as_millis(),
                        "drain deadline elapsed; aborting outstanding RPCs"
                    );
                } else {
                    tracing::info!(
                        drain_ms = %start.elapsed().as_millis(),
                        "in-flight RPCs drained"
                    );
                }
                return;
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let tracing_guard = trogon_atlas_server::telemetry::init_tracing()?;

    let args = Args::parse();

    match args.cmd.clone() {
        Some(Cmd::ExportGit { path }) => {
            let store = build_store(&args).await?;
            return run_export(&args, path, store).await;
        }
        Some(Cmd::ImportGit {
            path,
            restore_namespaces,
        }) => {
            let store = build_store(&args).await?;
            return run_import(path, restore_namespaces, store).await;
        }
        Some(Cmd::Seed { fixture }) => {
            let store = build_store(&args).await?;
            return run_seed(fixture, store).await;
        }
        Some(Cmd::Reconcile { mirror }) => {
            let store = build_store(&args).await?;
            return run_reconcile(mirror, store).await;
        }
        Some(Cmd::BackfillNamespaces { parent, dry_run }) => {
            let store = build_store(&args).await?;
            return run_backfill_namespaces(&parent, dry_run, store).await;
        }
        Some(Cmd::SyncSpicedb { dry_run }) => {
            let store = build_store(&args).await?;
            return run_sync_spicedb(&args, dry_run, store).await;
        }
        Some(Cmd::RecoverBatches {
            report_only,
            min_age_secs,
        }) => {
            let store = build_store(&args).await?;
            return run_recover_batches(report_only, min_age_secs, store.as_ref()).await;
        }
        Some(Cmd::MigrateLegacyFields {
            apply,
            keep_schema_on_conflict,
        }) => {
            let store = build_nats_store(&args, StoreOpenMode::MigrateLegacyFields).await?;
            let mode = if apply {
                MigrationMode::Apply
            } else {
                MigrationMode::DryRun
            };
            let conflicts = if keep_schema_on_conflict {
                ConflictResolution::KeepSchema
            } else {
                ConflictResolution::Refuse
            };
            return run_migrate_legacy_fields(mode, conflicts, &store).await;
        }
        None => {}
    }

    let store = build_store(&args).await?;
    trogon_atlas_server::batch_recovery::recover_at_startup(store.as_ref()).await;
    let _batch_recovery_sweep = trogon_atlas_server::batch_recovery::spawn_sweep(store.clone());

    let mirror = match args.git_mirror.as_ref() {
        Some(path) => Some(GitMirror::open(
            path.clone(),
            MirrorAuthor {
                name: args.git_author_name.clone(),
                email: args.git_author_email.clone(),
            },
        )?),
        None => None,
    };

    let baseline_protection = trogon_atlas_server::scope::BaselineProtection::configure(
        args.protect_baseline,
        &args.protect_baseline_namespaces,
    )
    .map_err(|e| anyhow::anyhow!("invalid --protect-baseline-namespaces: {e}"))?;
    let directory = Arc::new(
        trogon_atlas_server::ownership::NamespaceDirectory::with_freshness(
            args.namespace_registry_reload_interval,
        ),
    );
    let mut svc = trogon_atlas_server::service::EventModelServiceImpl::try_new_with_directory(
        store.clone(),
        directory,
    )
    .context("constructing event modeling service")?
    .with_baseline_protection(baseline_protection.clone());
    match args.change_cursor_key.as_deref() {
        Some(encoded) => {
            let key = trogon_atlas_server::change_cursor::ChangeCursorKey::from_base64(encoded)
                .context("invalid --change-cursor-key")?;
            svc = svc.with_change_cursor_key(&key);
        }
        None => {
            tracing::warn!(
                "no TROGON_ATLAS_CHANGE_CURSOR_KEY configured; this process generated a random \
                 per-process key instead. A deployment running more than one \
                 trogon-atlas-server process serving the same ListChanges poll (a standby, a \
                 reader role, or horizontal scale) must set this explicitly, or a cursor \
                 minted by one process will fail to open on another."
            );
        }
    }
    if !baseline_protection.is_off() {
        tracing::info!(
            protected = %baseline_protection,
            "baseline write protection enabled: mutating RPCs without an \
             x-trogon-atlas-branch header are rejected unless the caller has the Admin role"
        );
    }
    let inflight_counter = svc.inflight_requests.clone();
    // Pre-warm the in-RAM search index so the first SearchEntities caller
    // doesn't pay the full `list + replace_all` cost. The lazy path inside
    // the RPC handler is the fallback if this background task fails.
    {
        let (warm_store, warm_index, warm_ready) = svc.search_warmup_handles();
        trogon_atlas_server::search::spawn_warmup(
            warm_store,
            warm_index,
            warm_ready,
            trogon_atlas_server::service::MAX_PROJECTION_ENTITIES,
        );
    }
    if let Some(m) = mirror {
        svc = svc.with_git_mirror(m);
    }
    let llm_provider: trogon_atlas_server::llm::LlmProvider = args
        .llm
        .parse()
        .with_context(|| format!("parsing --llm={}", args.llm))?;
    // Provider-neutral env vars take precedence; legacy Anthropic-specific
    // names are honored as a fallback so existing deployments keep working.
    let llm_api_key = args.llm_api_key.clone().or_else(|| {
        let legacy = std::env::var("ANTHROPIC_API_KEY").ok();
        if legacy.is_some() {
            tracing::warn!("ANTHROPIC_API_KEY is deprecated; set TROGON_ATLAS_LLM_API_KEY instead");
        }
        legacy
    });
    let llm_base_url = args.llm_base_url.clone().or_else(|| {
        let legacy = std::env::var("ANTHROPIC_BASE_URL").ok();
        if legacy.is_some() {
            tracing::warn!(
                "ANTHROPIC_BASE_URL is deprecated; set TROGON_ATLAS_LLM_BASE_URL instead"
            );
        }
        legacy
    });
    let llm_client = trogon_atlas_server::llm::build(
        llm_provider,
        llm_api_key,
        args.llm_model.clone(),
        llm_base_url,
        Some(args.llm_max_tokens),
        Some(args.llm_request_timeout_secs),
        Some(args.llm_concurrency),
    )?;
    if let Some(client) = llm_client.as_ref() {
        tracing::info!(
            provider = client.provider(),
            model = client.model(),
            "llm client ready"
        );
        // Entity titles, slugs, and doc strings written by workspace authors reach
        // the LLM prompt. A malicious or compromised author could embed
        // prompt-injection text. The wrap_user_prompt function in llm_analysis.rs
        // mitigates this with fencing, but defense-in-depth requires operators to
        // be aware of the surface area.
        tracing::warn!(
            "llm analysis enabled: entity authors have indirect access to the LLM prompt; \
             review the prompt injection mitigations in llm_analysis.rs before exposing \
             this server to untrusted content contributors"
        );
        let analyzer = Arc::new(
            trogon_atlas_server::llm_analysis::LlmAnalyzer::new(client.clone())
                .with_prompt_budget(args.llm_prompt_budget),
        );
        svc = svc.with_llm_analyzer(analyzer);
    }

    if args.jev {
        let api_key = args
            .jev_api_key
            .clone()
            .or_else(|| std::env::var("AI_GATEWAY_API_KEY").ok())
            .filter(|key| !key.trim().is_empty())
            .context("Jev requires TROGON_ATLAS_JEV_API_KEY or AI_GATEWAY_API_KEY")?;
        let mut config = trogon_atlas_server::jev::JevConfig::new(api_key);
        config.base_url.clone_from(&args.jev_base_url);
        config.timeout = std::time::Duration::from_secs(args.jev_timeout_secs);
        config.max_concurrency = args.jev_concurrency;
        config.cache_capacity = args.jev_cache_capacity;
        config.cache_ttl = std::time::Duration::from_secs(args.jev_cache_ttl_secs);
        config.confidence_threshold = args.jev_min_probability;
        let analyzer = trogon_atlas_server::jev::JevAnalyzer::new(config)
            .context("configuring Jev evaluation")?;
        svc = svc.with_jev_analyzer(Arc::new(analyzer));
        tracing::info!(model = "typesafe-ai/jev", "Jev field inference enabled");
    }

    let addr = args
        .listen
        .parse()
        .with_context(|| format!("parsing --listen={}", args.listen))?;

    if let Some(metrics_addr) = args.metrics_listen.as_deref() {
        let parsed = metrics_addr
            .parse()
            .with_context(|| format!("parsing --metrics-listen={metrics_addr}"))?;
        trogon_atlas_server::telemetry::install_prometheus_exporter(
            parsed,
            args.metrics_allow_external,
        )?;
        tracing::info!(metrics_addr = %parsed, "prometheus exporter started");
        trogon_atlas_server::telemetry::spawn_change_stream_stats_publisher(
            store.clone(),
            Duration::from_secs(30),
        );
    }

    let mut builder = Server::builder();
    match (args.tls_cert.as_ref(), args.tls_key.as_ref()) {
        (Some(cert), Some(key)) => {
            let cert_pem = std::fs::read(cert)
                .with_context(|| format!("reading --tls-cert={}", cert.display()))?;
            let key_pem = std::fs::read(key)
                .with_context(|| format!("reading --tls-key={}", key.display()))?;
            builder = builder
                .tls_config(ServerTlsConfig::new().identity(Identity::from_pem(cert_pem, key_pem)))
                .context("configuring TLS")?;
            tracing::info!("TLS enabled");
        }
        (None, None) => {
            tracing::warn!(
                "serving plaintext gRPC; set TROGON_ATLAS_TLS_CERT/TROGON_ATLAS_TLS_KEY or terminate \
                 TLS in front of this process"
            );
        }
        _ => anyhow::bail!("--tls-cert and --tls-key must be set together"),
    }

    let registry = if let Some(path) = args.auth_tokens_file.as_ref() {
        // Tokens file takes precedence over legacy --auth-token.
        let reg = TokenRegistry::load(path)
            .map_err(|e| anyhow::anyhow!("loading auth tokens file {}: {}", path.display(), e))?;
        tracing::info!(path = %path.display(), "loaded token registry");
        Arc::new(reg)
    } else if let Some(token) = args.auth_token.as_deref() {
        tracing::warn!(
            "TROGON_ATLAS_AUTH_TOKEN is deprecated; migrate to --auth-tokens-file / \
             TROGON_ATLAS_AUTH_TOKENS_FILE for per-principal roles"
        );
        Arc::new(TokenRegistry::from_legacy_token(token))
    } else if args.insecure_allow_anonymous {
        tracing::warn!(
            "no auth token configured; every client with network access can mutate the model \
             (--insecure-allow-anonymous)"
        );
        Arc::new(TokenRegistry::anonymous())
    } else {
        anyhow::bail!(
            "no TROGON_ATLAS_AUTH_TOKENS_FILE or TROGON_ATLAS_AUTH_TOKEN configured. Set one, or \
             pass --insecure-allow-anonymous (or TROGON_ATLAS_INSECURE_ALLOW_ANONYMOUS=true) to \
             explicitly opt in to anonymous access (local development only)."
        );
    };
    let authorizer = build_spicedb(&args, svc.directory(), store.clone())?;
    if let Some(authorizer) = authorizer.clone() {
        if args.spicedb_skip_startup_sync {
            tracing::warn!(
                "--spicedb-skip-startup-sync is set: this process will not install the schema \
                 or publish principal memberships, so callers stay invisible until \
                 `sync-spicedb` has run against the same token file"
            );
        } else {
            let report = authorizer
                .reconcile(&registry.memberships())
                .await
                .map_err(|e| anyhow::anyhow!("synchronizing SpiceDB: {e}"))?;
            tracing::info!(
                namespaces = report.namespaces,
                memberships = report.memberships,
                "synchronized SpiceDB with the namespace registry and token file"
            );
        }
        svc = svc.with_authorizer(authorizer);
        tracing::info!(
            endpoint = %args.spicedb_endpoint.as_deref().unwrap_or_default(),
            freshness = %args.spicedb_freshness,
            "namespace visibility delegated to SpiceDB"
        );
    }

    let auth = BearerAuth::new(registry.clone());

    if let Some(path) = args.auth_tokens_file.clone() {
        if let Some(interval) = args.auth_tokens_reload_interval.as_duration() {
            // Honors --spicedb-skip-startup-sync: a process told not to
            // publish memberships at startup must not start publishing them
            // later either, or the flag means nothing.
            let authorizer = (!args.spicedb_skip_startup_sync)
                .then_some(authorizer)
                .flatten();
            let live = auth.live();
            tokio::spawn(async move {
                trogon_atlas_server::token_reload::watch(
                    path,
                    live,
                    interval,
                    move |memberships| {
                        let authorizer = authorizer.clone();
                        async move {
                            let Some(authorizer) = authorizer else {
                                return Ok(());
                            };
                            authorizer
                                .reconcile(&memberships)
                                .await
                                .map(|_| ())
                                .map_err(|e| e.to_string())
                        }
                    },
                )
                .await;
            });
            tracing::info!(
                interval_secs = interval.as_secs(),
                "watching the token registry; keys can be issued and revoked without a restart"
            );
        } else {
            tracing::warn!(
                "token registry reloading is disabled; issuing or revoking a key requires a \
                 restart, which drops every connected client"
            );
        }
    }

    if args.namespace_registry_reload_interval.is_disabled() {
        tracing::warn!(
            "namespace registry caching never expires; a namespace moved by another replica              stays with its previous owner on this one until it restarts"
        );
    }

    type ServiceServer = trogon_atlas_proto::event_model_service_server::EventModelServiceServer<
        trogon_atlas_server::service::EventModelServiceImpl,
    >;
    let grpc_service = ServiceServer::new(svc)
        .max_decoding_message_size(args.max_message_bytes)
        .max_encoding_message_size(args.max_message_bytes);

    let (mut health_reporter, health_service) = tonic_health::server::health_reporter();

    // Verify store connectivity before advertising SERVING. A cheap list
    // with limit=1 is enough to confirm the backend is reachable.
    match store
        .list(
            trogon_atlas_store::store::ListFilter::default(),
            Some(1),
            None,
        )
        .await
    {
        Ok(_) => {
            health_reporter.set_serving::<ServiceServer>().await;
            tracing::info!("store connectivity verified; health=SERVING");
        }
        Err(err) => {
            tracing::error!(
                error = %err,
                "store connectivity check failed; health remains NOT_SERVING"
            );
            // Keep NOT_SERVING so load balancers withhold traffic until the
            // store is reachable. The server still starts so it can be
            // probed and will become SERVING once a mutation succeeds.
        }
    }

    // Background health probe: re-checks the store every 15 seconds and
    // toggles the gRPC health status accordingly. The task stops when the
    // shutdown channel fires.
    let (health_shutdown_tx, mut health_shutdown_rx) = tokio::sync::watch::channel(false);
    {
        let store_for_health = store.clone();
        let mut health_reporter_bg = health_reporter.clone();
        tokio::spawn(async move {
            const HEALTH_INTERVAL: Duration = Duration::from_secs(15);
            let mut interval = tokio::time::interval(HEALTH_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = interval.tick() => {}
                    _ = health_shutdown_rx.changed() => { break; }
                }
                match store_for_health
                    .list(
                        trogon_atlas_store::store::ListFilter::default(),
                        Some(1),
                        None,
                    )
                    .await
                {
                    Ok(_) => {
                        health_reporter_bg.set_serving::<ServiceServer>().await;
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "health probe: store unreachable; health=NOT_SERVING");
                        health_reporter_bg.set_not_serving::<ServiceServer>().await;
                    }
                }
            }
        });
    }

    tracing::info!(%addr, "trogon-atlas-server starting");

    let drain_deadline = std::time::Duration::from_secs(args.shutdown_drain_secs);

    // Watch channel for drain notification: InflightGuard::drop sends the
    // current count; await_inflight_drain wakes on each send.
    let (drain_tx, drain_rx) = tokio::sync::watch::channel(0i64);
    let drain_tx = std::sync::Arc::new(drain_tx);

    let inflight_for_layer = inflight_counter.clone();
    let mut builder = builder.concurrency_limit_per_connection(args.concurrency_limit);
    if args.rpc_timeout_secs > 0 {
        // An LLM-backed RPC needs headroom above the LLM's own timeout,
        // otherwise the gRPC deadline fires first and turns a slow-but-valid
        // LLM response into DEADLINE_EXCEEDED. Widen with a fixed buffer
        // rather than failing startup so existing configs keep working.
        let mut rpc_timeout_secs = args.rpc_timeout_secs;
        let llm_enabled = llm_provider != trogon_atlas_server::llm::LlmProvider::Disabled;
        let analysis_timeout_secs = if llm_enabled {
            args.llm_request_timeout_secs
        } else {
            0
        }
        .saturating_add(if args.jev { args.jev_timeout_secs } else { 0 });
        if analysis_timeout_secs > 0 && rpc_timeout_secs <= analysis_timeout_secs {
            rpc_timeout_secs = analysis_timeout_secs.saturating_add(30);
            tracing::warn!(
                configured = args.rpc_timeout_secs,
                effective = rpc_timeout_secs,
                analysis_timeout_secs,
                "widening RPC timeout to allow analysis and fallback to complete"
            );
        }
        builder = builder.timeout(std::time::Duration::from_secs(rpc_timeout_secs));
    }
    builder
        .layer(trogon_atlas_server::telemetry::RpcMetricsLayer::new())
        .layer(trogon_atlas_server::telemetry::InflightLayer::new(
            inflight_for_layer,
            drain_tx,
        ))
        .trace_fn(trogon_atlas_server::telemetry::rpc_span_from_request)
        // The health service is mounted OUTSIDE the auth stack so
        // liveness/readiness probes (kube-proxy, GCP HC, etc.) can reach it
        // without holding a bearer token. It exposes only liveness -- no
        // entity data -- so the bypass is intentional and bounded. Anything
        // touching the EventModelService data plane must stay inside the
        // auth stack below.
        //
        // Request flow for the data-plane service:
        //   InterceptedService/BearerAuth (token -> Principal in extensions, authn)
        //     -> AuthzLayer (required_role(uri.path()) vs principal.role, authz)
        //       -> handler
        //
        // Ordering is critical: InterceptedService is the OUTERMOST wrapper so
        // BearerAuth runs first and populates extensions before AuthzLayer reads
        // them. Inverting the nesting puts AuthzLayer before authn and denies
        // every request.
        .add_service(health_service)
        .add_service(InterceptedService::new(
            tower::ServiceBuilder::new()
                .layer(AuthzLayer)
                .service(grpc_service),
            auth,
        ))
        .serve_with_shutdown(addr, async {
            shutdown_signal().await;
            // Stop the background health probe before toggling the status so
            // the probe cannot race and flip back to SERVING after we set
            // NOT_SERVING here.
            let _ = health_shutdown_tx.send(true);
            health_reporter.set_not_serving::<ServiceServer>().await;
            await_inflight_drain(&inflight_counter, drain_rx, drain_deadline).await;
        })
        .await?;

    // Flush any spans the batch OTLP exporter still has queued. Must run
    // before the process exits or the last few seconds of trace data are
    // silently dropped.
    tracing_guard.shutdown();
    Ok(())
}

/// Maximum number of entities fetched in a single admin scan. The `Store::list`
/// API only accepts a limit (no cursor), so we use this cap to bound memory.
/// When the store exceeds this many entities the admin subcommands will warn.
const ADMIN_SCAN_CAP: usize = 100_000;

/// Write the store out as a git mirror: every entity, plus the ownership
/// snapshot, in one commit.
///
/// A mirror only ever existed as a side effect of running a server with
/// `--git-mirror`, which meant a deployment that had not been doing that had
/// no way to produce one, and a deployment that started doing it late had a
/// mirror missing everything written before. Both of those are backups an
/// operator believes in until the day they need one.
async fn run_export(args: &Args, root: PathBuf, store: Arc<dyn Store>) -> Result<()> {
    use trogon_atlas_server::git_mirror::{
        MirrorChange, NamespaceSnapshot, NAMESPACE_SNAPSHOT_FILE,
    };

    let stored = store
        .list(
            trogon_atlas_store::store::ListFilter::default(),
            Some(ADMIN_SCAN_CAP),
            None,
        )
        .await
        .map_err(|e| anyhow::anyhow!("listing store: {e}"))?;
    // A truncated listing would export as a mirror that looks complete, and
    // the deletion pass below would then remove every entity past the cap
    // from a mirror that already held it. Refuse instead.
    anyhow::ensure!(
        stored.len() < ADMIN_SCAN_CAP,
        "store listing hit the scan cap of {ADMIN_SCAN_CAP}; an export from a truncated \
         listing would silently drop entities and delete them from the mirror",
    );

    let mirror = GitMirror::open(
        root.clone(),
        MirrorAuthor {
            name: args.git_author_name.clone(),
            email: args.git_author_email.clone(),
        },
    )?;

    // Anything the mirror holds that the store no longer does. Without this
    // an export is an accumulation: entities deleted since the last one stay
    // in the tree and come back on the next import.
    let mut live: std::collections::BTreeSet<(i32, String, String, u64)> =
        std::collections::BTreeSet::new();
    let mut changes: Vec<MirrorChange> = Vec::new();
    for record in &stored {
        let (Some(kind), Some(id)) = (entity_kind(&record.entity), entity_id(&record.entity))
        else {
            tracing::warn!("skipping an entity with no kind or id");
            continue;
        };
        live.insert((
            kind as i32,
            id.namespace.clone(),
            id.slug.clone(),
            id.version,
        ));
        changes.push(MirrorChange::Put(record.entity.clone()));
    }
    let exported = changes.len();

    let existing = trogon_atlas_server::git_mirror::walk_repo(&root)
        .with_context(|| format!("walking the existing mirror at {}", root.display()))?;
    let mut removed = 0usize;
    for entity in &existing {
        let (Some(kind), Some(id)) = (entity_kind(entity), entity_id(entity)) else {
            continue;
        };
        let key = (
            kind as i32,
            id.namespace.clone(),
            id.slug.clone(),
            id.version,
        );
        if !live.contains(&key) {
            changes.push(MirrorChange::Delete {
                kind,
                id: id.clone(),
            });
            removed += 1;
        }
    }

    let registered = store
        .list_namespaces()
        .await
        .map_err(|e| anyhow::anyhow!("listing the namespace registry: {e}"))?;
    let exported_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let (snapshot, provisional) = NamespaceSnapshot::build(&registered, exported_at);
    if provisional > 0 {
        tracing::info!(
            count = provisional,
            "left provisional namespaces out of the snapshot; they belong to branches, which \
             the mirror does not carry"
        );
    }
    changes.push(MirrorChange::PutRootFile {
        name: NAMESPACE_SNAPSHOT_FILE.to_owned(),
        contents: snapshot.to_bytes()?,
    });

    let owners = snapshot.namespaces.len();
    mirror
        .apply(trogon_atlas_server::git_mirror::MirrorBatch {
            changes,
            author: MirrorAuthor {
                name: args.git_author_name.clone(),
                email: args.git_author_email.clone(),
            },
            message: format!("export-git: {exported} entities, {owners} namespaces"),
        })
        .await?;

    tracing::info!(
        exported,
        removed,
        namespaces = owners,
        provisional_skipped = provisional,
        mirror = %root.display(),
        "export-git complete"
    );
    Ok(())
}

/// Put the mirror's ownership snapshot back into the registry.
///
/// Returns how many rows it created. Rows already in the registry are left
/// alone rather than overwritten: a restore run against a store that is not
/// actually empty must not re-own a live namespace from a file.
async fn restore_namespace_snapshot(
    snapshot: &trogon_atlas_server::git_mirror::NamespaceSnapshot,
    store: &Arc<dyn Store>,
) -> Result<usize> {
    let records = snapshot.to_records()?;
    let mut created = 0usize;
    let mut conflicted: Vec<String> = Vec::new();
    for record in &records {
        match store.restore_namespace(record).await {
            Ok(claim) if claim.created => created += 1,
            Ok(_) => {}
            Err(trogon_atlas_store::error::StoreError::AlreadyExists) => {
                conflicted.push(record.id.to_string());
            }
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "restoring namespace {}: {err}",
                    record.id.as_str()
                ))
            }
        }
    }
    if !conflicted.is_empty() {
        // Not fatal, and not silent. Each of these is a namespace whose id the
        // live registry already hands to a different owner or a different
        // name, so the snapshot and the store disagree about who owns entities
        // that are about to be imported under that id.
        tracing::error!(
            count = conflicted.len(),
            namespaces = %conflicted.join(", "),
            "the registry already holds these ids under a different owner or name, so the \
             snapshot was not applied to them; entities imported under these ids belong to \
             whoever the live registry says, not to whoever the mirror says"
        );
    }
    tracing::info!(
        restored = created,
        in_snapshot = records.len(),
        conflicted = conflicted.len(),
        "namespace registry restore complete"
    );
    Ok(created)
}

async fn run_import(path: PathBuf, restore_namespaces: bool, store: Arc<dyn Store>) -> Result<()> {
    let entities = trogon_atlas_server::git_mirror::walk_repo(&path)
        .with_context(|| format!("walking {}", path.display()))?;

    // Ownership first. Claim-on-first-write means an entity landing in an
    // unregistered namespace registers it to nobody in particular, so the
    // registry has to be in place before the entities are, not after.
    let snapshot = trogon_atlas_server::git_mirror::read_namespace_snapshot(&path)?;
    if restore_namespaces {
        match &snapshot {
            Some(snapshot) => {
                restore_namespace_snapshot(snapshot, &store).await?;
            }
            None => anyhow::bail!(
                "--restore-namespaces was given but {} holds no {}; this mirror was written \
                 by a server's live mirroring rather than by export-git, and carries no \
                 ownership at all",
                path.display(),
                trogon_atlas_server::git_mirror::NAMESPACE_SNAPSHOT_FILE,
            ),
        }
    }
    let mut imported = 0usize;
    let mut skipped = 0usize;
    for entity in &entities {
        let Some(kind) = entity_kind(entity) else {
            continue;
        };
        let Some(_id) = entity_id(entity) else {
            continue;
        };
        // Import runs outside any RPC, so there is no authenticated principal
        // to attribute the write to and no changeset is minted. The resulting
        // change events carry an empty author, which is the documented
        // "unattributed write" case.
        match store
            .create(kind, entity, trogon_atlas_store::WriteContext::baseline())
            .await
        {
            Ok(_) => imported += 1,
            Err(trogon_atlas_store::error::StoreError::AlreadyExists) => skipped += 1,
            Err(err) => {
                tracing::warn!(error = %err, "skipping entity that failed to import");
                skipped += 1;
            }
        }
    }
    let total = store
        .list(
            trogon_atlas_store::store::ListFilter::default(),
            Some(ADMIN_SCAN_CAP),
            None,
        )
        .await
        .map_err(|e| anyhow::anyhow!("listing store after import: {e}"))?;
    if total.len() >= ADMIN_SCAN_CAP {
        tracing::warn!(
            cap = ADMIN_SCAN_CAP,
            "store total hit the scan cap; actual count may be higher"
        );
    }

    // What the mirror could not bring back. Ownership was never in it, so a
    // restore that reports only its entity count reports success on half a
    // store: the namespaces are there and unowned, which reads as empty to
    // every bound caller and is claimable by whichever one writes first.
    let registered = store
        .list_namespaces()
        .await
        .map_err(|e| anyhow::anyhow!("listing the namespace registry after import: {e}"))?;
    let unregistered =
        trogon_atlas_server::git_mirror::unregistered_namespaces(&entities, &registered);
    if !unregistered.is_empty() {
        let remedy = if snapshot.is_some() && !restore_namespaces {
            "This mirror carries an ownership snapshot: re-run with --restore-namespaces"
        } else {
            "Restore the registry from a store backup, or assign them with \
             backfill-namespaces and move-namespace"
        };
        tracing::warn!(
            count = unregistered.len(),
            namespaces = %unregistered.join(", "),
            remedy,
            "imported entities whose namespaces have no registry row -- these are invisible to \
             every caller bound to a parent, and the first write into one claims it",
        );
    }

    tracing::info!(
        imported,
        skipped,
        unregistered = unregistered.len(),
        store_total = total.len(),
        "import-git complete"
    );
    Ok(())
}

/// The SpiceDB-backed authorizer, or `None` when no endpoint is configured
/// and ownership stays a registry question.
fn build_spicedb(
    args: &Args,
    directory: Arc<trogon_atlas_server::ownership::NamespaceDirectory>,
    store: Arc<dyn Store>,
) -> Result<Option<Arc<trogon_atlas_server::spicedb::SpiceDbAuthorizer>>> {
    let Some(endpoint) = args.spicedb_endpoint.as_deref() else {
        // Refused rather than ignored: a key with no endpoint is somebody who
        // believes SpiceDB is on when it is not, and silently serving every
        // request through the registry instead is the wrong way to find out.
        anyhow::ensure!(
            args.spicedb_preshared_key.is_none(),
            "--spicedb-preshared-key is set without --spicedb-endpoint",
        );
        return Ok(None);
    };
    let key = args
        .spicedb_preshared_key
        .as_deref()
        .context("--spicedb-endpoint requires --spicedb-preshared-key")?;
    let key = trogon_atlas_authzed::PresharedKey::parse(key)
        .map_err(|e| anyhow::anyhow!("invalid --spicedb-preshared-key: {e}"))?;

    let config = trogon_atlas_authzed::SpiceDbConfig::new(endpoint.to_owned(), key);
    Ok(Some(Arc::new(
        trogon_atlas_server::spicedb::SpiceDbAuthorizer::new(
            &config,
            directory,
            store,
            args.spicedb_freshness,
        )
        .with_context(|| format!("connecting to SpiceDB at {endpoint}"))?,
    )))
}

async fn run_sync_spicedb(args: &Args, dry_run: bool, store: Arc<dyn Store>) -> Result<()> {
    let directory = Arc::new(trogon_atlas_server::ownership::NamespaceDirectory::new());
    let authorizer = build_spicedb(args, directory, store.clone())?
        .context("sync-spicedb requires --spicedb-endpoint")?;

    // Only a tokens file can carry a `parent`; the legacy single token and
    // anonymous mode have no principals to bind, so there is nothing to sync
    // for them and an absent file is not an error.
    let memberships = match args.auth_tokens_file.as_ref() {
        Some(path) => TokenRegistry::load(path)
            .map_err(|e| anyhow::anyhow!("loading auth tokens file {}: {}", path.display(), e))?
            .memberships(),
        None => Vec::new(),
    };

    if dry_run {
        let records = store.list_namespaces().await?;
        for record in &records {
            println!(
                "namespace:{} parent -> organization:{}",
                record.id, record.parent
            );
        }
        for (principal, owner) in &memberships {
            println!("organization:{owner} member -> user:{principal}");
        }
        println!(
            "dry run: would write the schema, {} namespace grants and {} memberships",
            records.len(),
            memberships.len(),
        );
        return Ok(());
    }

    let report = authorizer
        .reconcile(&memberships)
        .await
        .map_err(|e| anyhow::anyhow!("synchronizing SpiceDB: {e}"))?;
    println!(
        "wrote the schema, {} namespace grants and {} memberships",
        report.namespaces, report.memberships,
    );
    Ok(())
}

/// Backfill the namespace registry from the entities already in the store.
///
/// The namespaces are discovered from the entity keys rather than from any
/// existing list, because the entity keys are the only authority on what
/// exists. Each one is adopted under an id equal to its name, which is what
/// makes this a metadata-only operation: 3000 entities keyed by
/// `orders.event....` stay exactly where they are, and the registry simply
/// starts describing them.
async fn run_backfill_namespaces(parent: &str, dry_run: bool, store: Arc<dyn Store>) -> Result<()> {
    use std::collections::BTreeSet;

    let parent = trogon_atlas_core::OwnerId::parse(parent)
        .map_err(|e| anyhow::anyhow!("invalid --parent: {e}"))?;

    let existing: std::collections::HashMap<String, trogon_atlas_core::OwnerId> = store
        .list_namespaces()
        .await
        .map_err(|e| anyhow::anyhow!("reading namespace registry: {e}"))?
        .into_iter()
        .map(|record| (record.id.to_string(), record.parent))
        .collect();

    let loaded = trogon_atlas_server::graph::load_all(&store, None)
        .await
        .map_err(|e| anyhow::anyhow!("scanning entities: {e}"))?;
    if loaded.truncated {
        anyhow::bail!(
            "entity scan hit the snapshot cap, so the namespace list would be incomplete;              a partial backfill would leave namespaces unowned and invisible to scoped callers"
        );
    }

    let mut discovered: BTreeSet<String> = BTreeSet::new();
    for stored in &loaded.entities {
        let id = trogon_atlas_server::conv::entity_id(&stored.entity)
            .map_err(|e| anyhow::anyhow!("entity with unreadable id: {e}"))?;
        discovered.insert(id.namespace.clone());
    }

    let mut to_adopt: Vec<trogon_atlas_core::NamespaceName> = Vec::new();
    for namespace in &discovered {
        if let Some(owner) = existing.get(namespace) {
            tracing::debug!(namespace, %owner, "already registered");
            continue;
        }
        to_adopt.push(
            trogon_atlas_core::NamespaceName::parse(namespace)
                .map_err(|e| anyhow::anyhow!("namespace {namespace:?} is not a legal name: {e}"))?,
        );
    }

    println!(
        "{} namespaces in the store, {} already registered, {} to adopt under parent {parent}",
        discovered.len(),
        discovered.len() - to_adopt.len(),
        to_adopt.len(),
    );
    if dry_run {
        for name in &to_adopt {
            println!("  would adopt {name}");
        }
        println!("dry run: nothing written");
        return Ok(());
    }

    let mut adopted = 0_usize;
    for name in &to_adopt {
        let claim = store
            .adopt_namespace(
                name,
                &parent,
                &trogon_atlas_store::NamespaceTenure::Permanent,
            )
            .await
            .map_err(|e| anyhow::anyhow!("adopting namespace {name}: {e}"))?;
        if claim.created {
            adopted += 1;
        }
        println!(
            "  adopted {name} -> id {} (created={})",
            claim.record.id, claim.created
        );
    }
    println!("backfill complete: {adopted} rows written");
    Ok(())
}

async fn run_recover_batches(
    report_only: bool,
    min_age_secs: u64,
    store: &dyn Store,
) -> Result<()> {
    let policy = if report_only {
        trogon_atlas_store::RecoveryPolicy::report_all()
    } else {
        trogon_atlas_store::RecoveryPolicy::repair_all()
    }
    .older_than(Duration::from_secs(min_age_secs));
    let report = store
        .recover_batches(policy)
        .await
        .map_err(|e| anyhow::anyhow!("batch recovery: {e}"))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&trogon_atlas_server::batch_recovery::report_json(&report))?
    );
    anyhow::ensure!(
        report.is_converged(),
        "{} batch(es) not converged",
        report.batches.len() + report.skipped_in_flight + report.skipped_recent
    );
    Ok(())
}

async fn run_migrate_legacy_fields(
    mode: MigrationMode,
    conflicts: ConflictResolution,
    store: &NatsStore,
) -> Result<()> {
    let report = store
        .migrate_legacy_fields(mode, conflicts)
        .await
        .map_err(|e| anyhow::anyhow!("legacy fields migration: {e}"))?;
    let verb = match mode {
        MigrationMode::DryRun => "would rewrite",
        MigrationMode::Apply => "rewrote",
    };
    for finding in &report.findings {
        if finding.shape.is_conflict() && conflicts == ConflictResolution::KeepSchema {
            println!(
                "{verb} {}: fields and schema disagree, fields dropped, schema kept",
                finding.location
            );
        } else if finding.shape.is_conflict() {
            println!(
                "CONFLICT {}: fields and schema disagree, left untouched",
                finding.location
            );
        } else {
            println!("{verb} {} ({})", finding.location, finding.shape);
        }
    }
    let counts: Vec<String> = report
        .counts
        .iter()
        .map(|(shape, n)| format!("{shape}={n}"))
        .collect();
    println!("summary: {}", counts.join(" "));
    match (mode, report.remaining()) {
        (MigrationMode::DryRun, _) => {
            println!("dry run: nothing written; pass --apply to write");
        }
        (MigrationMode::Apply, 0) => {
            println!("store marked migrated; servers can open it");
        }
        (MigrationMode::Apply, _) => {
            println!("store not marked migrated; servers still refuse to open it");
        }
    }
    anyhow::ensure!(
        report.unresolved_conflicts() == 0,
        "{} entity image(s) carry fields and schema that disagree; rerun with --apply \
         --keep-schema-on-conflict to keep schema and drop the retired list",
        report.count(LegacyFieldsShape::BothDifferent)
    );
    Ok(())
}

async fn run_reconcile(mirror: PathBuf, store: Arc<dyn Store>) -> Result<()> {
    use std::collections::BTreeMap;

    use prost::Message;
    use trogon_atlas_proto::canonical::kind_short;

    let mirror_entities = trogon_atlas_server::git_mirror::walk_repo(&mirror)
        .with_context(|| format!("walking {}", mirror.display()))?;
    let store_entities = store
        .list(
            trogon_atlas_store::store::ListFilter::default(),
            Some(ADMIN_SCAN_CAP),
            None,
        )
        .await
        .map_err(|e| anyhow::anyhow!("listing store: {e}"))?;
    if store_entities.len() >= ADMIN_SCAN_CAP {
        anyhow::bail!(
            "store listing returned {} entities, hitting the scan cap of {}; \
             reconcile result would be incomplete. Increase ADMIN_SCAN_CAP or \
             reduce store size before running reconcile.",
            store_entities.len(),
            ADMIN_SCAN_CAP
        );
    }

    let key_of = |kind: pb::EntityKind, id: &pb::Id| -> String {
        format!(
            "{}/{}/{}@{}",
            kind_short(kind),
            id.namespace,
            id.slug,
            id.version
        )
    };

    let mut mirror_map: BTreeMap<String, pb::Entity> = BTreeMap::new();
    for entity in mirror_entities {
        let (Some(kind), Some(id)) = (entity_kind(&entity), entity_id(&entity)) else {
            tracing::warn!("skipping unindexable mirror entity");
            continue;
        };
        mirror_map.insert(key_of(kind, id), entity);
    }

    let mut store_map: BTreeMap<String, pb::Entity> = BTreeMap::new();
    for stored in store_entities {
        let (Some(kind), Some(id)) = (entity_kind(&stored.entity), entity_id(&stored.entity))
        else {
            continue;
        };
        store_map.insert(key_of(kind, id), stored.entity);
    }

    let mut only_in_store: Vec<String> = Vec::new();
    let mut only_in_mirror: Vec<String> = Vec::new();
    let mut byte_diffs: Vec<String> = Vec::new();

    for (key, store_entity) in &store_map {
        match mirror_map.get(key) {
            None => only_in_store.push(key.clone()),
            Some(mirror_entity) => {
                if store_entity.encode_to_vec() != mirror_entity.encode_to_vec() {
                    byte_diffs.push(key.clone());
                }
            }
        }
    }
    for key in mirror_map.keys() {
        if !store_map.contains_key(key) {
            only_in_mirror.push(key.clone());
        }
    }

    let drift = only_in_store.len() + only_in_mirror.len() + byte_diffs.len();
    tracing::info!(
        store_total = store_map.len(),
        mirror_total = mirror_map.len(),
        only_in_store = only_in_store.len(),
        only_in_mirror = only_in_mirror.len(),
        byte_diffs = byte_diffs.len(),
        "reconcile scan complete"
    );

    let report_section = |label: &str, items: &[String]| {
        if items.is_empty() {
            return;
        }
        eprintln!("\n{} ({}):", label, items.len());
        for key in items {
            eprintln!("  {key}");
        }
    };
    report_section("only in store (mirror missing)", &only_in_store);
    report_section("only in mirror (store missing)", &only_in_mirror);
    report_section("byte-level drift (both sides present)", &byte_diffs);

    if drift > 0 {
        anyhow::bail!("{drift} drift(s) detected between store and mirror");
    }

    tracing::info!("OK: store and mirror are in sync");
    Ok(())
}

async fn run_seed(fixture: PathBuf, store: Arc<dyn Store>) -> Result<()> {
    let bytes = std::fs::read(&fixture)
        .with_context(|| format!("reading fixture {}", fixture.display()))?;
    let request = trogon_atlas_server::fixtures::decode_batch_mutate_fixture(&fixture, &bytes)?;
    let op_count = request.ops.len();

    let svc = trogon_atlas_server::service::EventModelServiceImpl::try_new(store.clone())
        .context("constructing event modeling service for seed")?;
    let response = svc
        .batch_mutate(tonic::Request::new(request))
        .await
        .map_err(|status| anyhow::anyhow!("batch_mutate failed: {status}"))?
        .into_inner();

    let status =
        pb::batch_mutate_response::Status::try_from(response.status).with_context(|| {
            format!(
                "batch_mutate returned unknown status code {}",
                response.status
            )
        })?;
    if !matches!(status, pb::batch_mutate_response::Status::Applied) {
        anyhow::bail!(
            "fixture did not apply cleanly: status={:?} failure={:?}",
            status,
            response.failure
        );
    }
    let total = store
        .list(
            trogon_atlas_store::store::ListFilter::default(),
            Some(ADMIN_SCAN_CAP),
            None,
        )
        .await
        .map_err(|e| anyhow::anyhow!("listing store after seed: {e}"))?;
    if total.len() >= ADMIN_SCAN_CAP {
        tracing::warn!(
            cap = ADMIN_SCAN_CAP,
            "store total hit the scan cap; actual count may be higher"
        );
    }
    tracing::info!(op_count, store_total = total.len(), "seed complete");
    Ok(())
}

#[cfg(test)]
mod bearer_auth_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use tonic::{metadata::MetadataValue, service::Interceptor};

    use super::*;

    fn make_auth(token: &str) -> BearerAuth {
        BearerAuth::new(Arc::new(TokenRegistry::from_legacy_token(token)))
    }

    fn make_anon_auth() -> BearerAuth {
        BearerAuth::new(Arc::new(TokenRegistry::anonymous()))
    }

    fn req_with_token(token: &str) -> tonic::Request<()> {
        let mut req = tonic::Request::new(());
        req.metadata_mut().insert(
            "authorization",
            MetadataValue::try_from(format!("Bearer {token}")).unwrap(),
        );
        req
    }

    fn req_without_header() -> tonic::Request<()> {
        tonic::Request::new(())
    }

    #[test]
    fn correct_token_passes() {
        let mut auth = make_auth("secret");
        assert!(auth.call(req_with_token("secret")).is_ok());
    }

    #[test]
    fn wrong_token_rejected() {
        let mut auth = make_auth("secret");
        let err = auth.call(req_with_token("wrong")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn missing_header_rejected() {
        let mut auth = make_auth("secret");
        let err = auth.call(req_without_header()).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn anonymous_mode_passes_everything() {
        let mut auth = make_anon_auth();
        assert!(auth.call(req_without_header()).is_ok());
        assert!(auth.call(req_with_token("any")).is_ok());
    }

    #[test]
    fn correct_token_inserts_principal_in_extensions() {
        let mut auth = make_auth("mytoken");
        let result = auth.call(req_with_token("mytoken")).unwrap();
        let principal = result
            .extensions()
            .get::<trogon_atlas_server::auth::Principal>()
            .expect("Principal must be in extensions after successful auth");
        assert_eq!(principal.name.as_ref(), "legacy-admin");
        assert_eq!(principal.role, trogon_atlas_server::auth::Role::Admin);
    }
}
