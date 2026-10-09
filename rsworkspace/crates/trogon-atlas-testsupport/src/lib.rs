//! Test helpers that spin up ephemeral backends using testcontainers: a
//! NATS `JetStream` instance vending ready-to-use [`NatsStore`] instances
//! with isolated bucket/stream names, and a SpiceDB instance for tests
//! that exercise the authorization path against the real thing.

pub mod discovery;

use std::{
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, Once,
    },
};

use testcontainers::{
    core::{IntoContainerPort, WaitFor},
    ContainerAsync, GenericImage, ImageExt,
};
use testcontainers_modules::nats::{Nats, NatsServerCmd};
use tokio::sync::OnceCell;
use trogon_atlas_store::{NatsStore, NatsStoreConfig};

static SHARED: OnceCell<TestNats> = OnceCell::const_new();
static SHARED_SPICEDB: OnceCell<TestSpiceDb> = OnceCell::const_new();

// Containers held in the `shared()` statics never drop, and testcontainers has
// no reaper, so remove every started container when the test process exits.
static EXIT_CLEANUP_IDS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static EXIT_CLEANUP_REGISTERED: Once = Once::new();

fn register_for_exit_cleanup(id: &str) {
    EXIT_CLEANUP_REGISTERED.call_once(|| unsafe {
        libc::atexit(cleanup_containers_at_exit);
    });
    if let Ok(mut ids) = EXIT_CLEANUP_IDS.lock() {
        ids.push(id.to_string());
    }
}

extern "C" fn cleanup_containers_at_exit() {
    let ids = EXIT_CLEANUP_IDS
        .lock()
        .map(|mut ids| std::mem::take(&mut *ids))
        .unwrap_or_default();
    for id in ids {
        let _ = Command::new("docker").args(["rm", "-f", &id]).output();
    }
}

/// An ephemeral NATS server started by testcontainers.
///
/// The container lives as long as this value. In tests that call
/// [`shared`], the container lives for the duration of the process.
pub struct TestNats {
    pub url: String,
    container: ContainerAsync<Nats>,
}

impl TestNats {
    /// Start a fresh NATS container with `JetStream` enabled.
    ///
    /// Returns an error when Docker is unavailable or the container fails to
    /// bind. Callers that want a panicking convenience shim should use
    /// [`TestNats::start_or_panic`].
    pub async fn try_start() -> Result<Self, String> {
        use testcontainers::runners::AsyncRunner;
        let cmd = NatsServerCmd::default().with_jetstream();
        let container = Nats::default().with_cmd(&cmd).start().await.map_err(|e| {
            format!("testcontainers: NATS container failed to start (is Docker running?): {e}")
        })?;
        register_for_exit_cleanup(container.id());
        let port = container
            .get_host_port_ipv4(4222)
            .await
            .map_err(|e| format!("testcontainers: could not get NATS port: {e}"))?;
        let url = format!("nats://127.0.0.1:{port}");
        Ok(Self { url, container })
    }

    /// Start a fresh NATS container, panicking with a human-readable message
    /// when Docker is unavailable. Callers that need a `Result` instead of a
    /// panic should use [`TestNats::try_start`].
    pub async fn start() -> Self {
        Self::try_start().await.unwrap_or_else(|e| panic!("{e}"))
    }

    /// Build a [`NatsStore`] backed by this container with a unique
    /// bucket and stream name, so concurrent tests sharing one container
    /// are fully isolated from each other.
    ///
    /// Returns an error if the store fails to connect or configure.
    pub async fn try_store(&self) -> Result<NatsStore, String> {
        let suffix = unique_suffix();
        let mut config = NatsStoreConfig::new(&self.url);
        config.bucket = format!("test-{suffix}");
        config.branches_bucket = format!("test-branches-{suffix}");
        config.changesets_bucket = format!("test-changesets-{suffix}");
        config.revisions_bucket = format!("test-revisions-{suffix}");
        config.batches_bucket = format!("test-batches-{suffix}");
        config.namespaces_bucket = format!("test-namespaces-{suffix}");
        config.lease_bucket = format!("test-writer-lease-{suffix}");
        config.stream = format!("TEST_{}", suffix.replace('-', "_").to_uppercase());
        config.subject_root = format!("test.{suffix}");
        config.changes_max_msgs = Some(10_000);
        config.changes_max_age = None;
        NatsStore::connect_with(config)
            .await
            .map_err(|e| format!("NatsStore::connect_with failed in test: {e}"))
    }

    /// Build a [`NatsStore`], panicking with a clear message on failure.
    /// Callers that need a `Result` instead of a panic should use
    /// [`TestNats::try_store`].
    pub async fn store(&self) -> NatsStore {
        self.try_store().await.unwrap_or_else(|e| panic!("{e}"))
    }

    /// Stop the underlying Docker container immediately (SIGKILL), severing
    /// connectivity for any client already connected to it.
    ///
    /// Intended for tests exercising backend-unavailable error paths. Never
    /// call this on the process-wide [`shared`] container: use a container
    /// obtained from [`TestNats::start`] instead, since stopping the shared
    /// one would break every other test in the binary.
    pub async fn stop(&self) -> Result<(), String> {
        self.container
            .stop_with_timeout(Some(0))
            .await
            .map_err(|e| format!("testcontainers: failed to stop NATS container: {e}"))
    }
}

/// Return a process-wide shared [`TestNats`] container, starting it on
/// first call. Subsequent calls return the same instance without additional
/// container overhead.
///
/// Use this when you want a single container shared across many test
/// functions in a binary. Each call to [`TestNats::store`] still produces
/// an isolated bucket/stream, so tests do not interfere.
pub async fn shared() -> &'static TestNats {
    SHARED.get_or_init(TestNats::start).await
}

/// Preshared key the test SpiceDB is started with. Not a secret: the
/// container is unreachable from outside the test run and dies with it.
pub const SPICEDB_PRESHARED_KEY: &str = "trogon-atlas-test-key";

/// An ephemeral SpiceDB server started by testcontainers, backed by the
/// in-memory datastore so each container starts with no schema and no
/// relationships.
pub struct TestSpiceDb {
    pub endpoint: String,
    _container: ContainerAsync<GenericImage>,
}

impl TestSpiceDb {
    /// Start a fresh SpiceDB container.
    ///
    /// The version is pinned rather than tracking `latest`, because the
    /// schema language and the permission semantics are the contract under
    /// test, so a silent upgrade turning a test red would be
    /// indistinguishable from a regression.
    ///
    /// Returns an error when Docker is unavailable or the container fails to
    /// become ready.
    pub async fn try_start() -> Result<Self, String> {
        use testcontainers::runners::AsyncRunner;
        let container = GenericImage::new("authzed/spicedb", "v1.53.0")
            .with_exposed_port(50051.tcp())
            .with_wait_for(WaitFor::message_on_stderr("grpc server started serving"))
            .with_cmd([
                "serve",
                "--grpc-preshared-key",
                SPICEDB_PRESHARED_KEY,
                "--skip-release-check",
            ])
            .start()
            .await
            .map_err(|e| {
                format!(
                    "testcontainers: SpiceDB container failed to start (is Docker running?): {e}"
                )
            })?;
        register_for_exit_cleanup(container.id());
        let port = container
            .get_host_port_ipv4(50051)
            .await
            .map_err(|e| format!("testcontainers: could not get SpiceDB port: {e}"))?;
        Ok(Self {
            endpoint: format!("http://127.0.0.1:{port}"),
            _container: container,
        })
    }

    /// Start a fresh SpiceDB container, panicking with a human-readable
    /// message when Docker is unavailable.
    pub async fn start() -> Self {
        Self::try_start().await.unwrap_or_else(|e| panic!("{e}"))
    }
}

/// Return a process-wide shared [`TestSpiceDb`] container, starting it on
/// first call.
///
/// Unlike NATS there is no per-test namespace to isolate with, so tests
/// sharing this container must use object ids unique to themselves;
/// [`unique_suffix_for_test`] is there for that.
pub async fn shared_spicedb() -> &'static TestSpiceDb {
    SHARED_SPICEDB.get_or_init(TestSpiceDb::start).await
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_suffix() -> String {
    unique_suffix_for_test()
}

/// Public for test use: generate a unique suffix using a monotonic counter.
/// The counter alone guarantees uniqueness within a process.
pub fn unique_suffix_for_test() -> String {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{n}")
}
