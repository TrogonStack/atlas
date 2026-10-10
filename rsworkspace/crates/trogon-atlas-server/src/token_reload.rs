//! Re-reading the token registry while the server is serving.
//!
//! Provisioning a key for a client must not cost every other client its
//! connection. Without this, `--auth-tokens-file` is read once at startup, so
//! adding a principal means a restart, and a restart drops every in-flight
//! stream on a process that exists to serve long-lived ones.
//!
//! The loop polls rather than watching inotify/FSEvents on purpose. The
//! registry and its `token_files` are routinely bind mounts, secret mounts, or
//! ConfigMap symlink swaps, and filesystem events on those are inconsistent
//! across platforms and orchestrators in a way that fails silently. A poll
//! that re-reads a handful of small files every few seconds cannot miss an
//! edit, and it costs nothing measurable.
//!
//! Change is judged by what the registry *grants*, not by mtime or bytes, so
//! `touch tokens.toml` and a reformat are both correctly no-ops.

use std::{path::PathBuf, sync::Arc, time::Duration};

use crate::auth::{LiveRegistry, TokenRegistry};

/// How often the server re-reads a source of authorization truth it does not
/// own: the token registry file here, and the namespace registry in
/// [`crate::ownership::NamespaceDirectory`].
///
/// The interval is the worst-case delay between writing a grant and that
/// grant taking effect, and equally between removing one and it ceasing to
/// apply, so it is a revocation latency rather than a mere polling knob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReloadInterval(Option<Duration>);

impl ReloadInterval {
    /// `0` disables reloading, which pins the registry to whatever startup
    /// read.
    #[must_use]
    pub fn from_secs(secs: u64) -> Self {
        Self((secs > 0).then(|| Duration::from_secs(secs)))
    }

    #[must_use]
    pub fn as_duration(self) -> Option<Duration> {
        self.0
    }

    #[must_use]
    pub fn is_disabled(self) -> bool {
        self.0.is_none()
    }
}

impl Default for ReloadInterval {
    fn default() -> Self {
        Self::from_secs(5)
    }
}

impl std::str::FromStr for ReloadInterval {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let secs: u64 = raw
            .trim()
            .parse()
            .map_err(|_| format!("expected a whole number of seconds, got {raw:?}"))?;
        Ok(Self::from_secs(secs))
    }
}

impl std::fmt::Display for ReloadInterval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.map_or(0, |d| d.as_secs()))
    }
}

/// What one pass over the registry file did.
#[derive(Debug)]
pub enum Reload {
    /// The file parses and grants exactly what is already in force.
    Unchanged,
    /// A new snapshot is now in force.
    Installed(Arc<TokenRegistry>),
    /// The file could not be read, parsed, or validated. The previous
    /// snapshot stays in force.
    Rejected(String),
}

/// Re-read `path` and install it if it grants something different.
///
/// A registry that fails to load is refused rather than applied, so a
/// half-written file, a bad edit, or a secret mount that momentarily
/// disappears cannot lock every client out. That is the whole reason this
/// returns [`Reload::Rejected`] instead of clearing the registry: the failure
/// mode of an auth reload has to be "keeps working", never "denies
/// everybody".
pub fn reload_once(path: &std::path::Path, live: &LiveRegistry) -> Reload {
    let next = match TokenRegistry::load(path) {
        Ok(next) => next,
        Err(e) => return Reload::Rejected(e),
    };
    if live.current().grants_same_as(&next) {
        return Reload::Unchanged;
    }
    let next = Arc::new(next);
    live.install(next.clone());
    Reload::Installed(next)
}

/// Poll `path` forever, installing every change and republishing memberships.
///
/// `publish_memberships` is handed the new principal-to-owner bindings after
/// a snapshot is installed. Installation deliberately does not wait for it:
/// the registry file is the authority on who may authenticate, so revoking a
/// key must take effect even when the external authorizer is unreachable. A
/// publish that fails is retried on the next tick until it succeeds, which
/// leaves a newly added principal authenticated but not yet visible to
/// SpiceDB for at most one interval.
pub async fn watch<P, Fut>(
    path: PathBuf,
    live: LiveRegistry,
    interval: Duration,
    publish_memberships: P,
) where
    P: Fn(Vec<(Arc<str>, trogon_atlas_core::OwnerId)>) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Startup already loaded and published, so the first tick is a no-op
    // unless the file changed in between.
    ticker.tick().await;
    let mut publish_pending = false;

    loop {
        ticker.tick().await;

        match reload_once(&path, &live) {
            Reload::Unchanged => {}
            Reload::Installed(registry) => {
                metrics::counter!("auth_registry_reloads_total").increment(1);
                tracing::info!(
                    path = %path.display(),
                    memberships = registry.memberships().len(),
                    "reloaded the token registry"
                );
                publish_pending = true;
            }
            Reload::Rejected(e) => {
                metrics::counter!("auth_registry_reload_failures_total").increment(1);
                tracing::error!(
                    path = %path.display(),
                    error = %e,
                    "token registry failed to reload; the previous one stays in force"
                );
            }
        }

        if publish_pending {
            match publish_memberships(live.current().memberships()).await {
                Ok(()) => publish_pending = false,
                Err(e) => {
                    metrics::counter!("auth_registry_membership_publish_failures_total")
                        .increment(1);
                    tracing::error!(
                        error = %e,
                        "publishing principal memberships failed; retrying on the next reload tick"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use super::{reload_once, watch, Reload, ReloadInterval};
    use crate::auth::{LiveRegistry, Role, TokenRegistry};

    fn write(dir: &std::path::Path, name: &str, contents: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, contents).expect("write fixture");
        path
    }

    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "trogon-atlas-reload-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create tempdir");
        dir
    }

    const ONE: &str = r#"
[principals.ci]
role = "writer"
tokens = ["first-key"]
"#;

    #[test]
    fn a_new_key_authenticates_without_a_restart() {
        let dir = tempdir();
        let path = write(&dir, "tokens.toml", ONE);
        let live = LiveRegistry::new(Arc::new(TokenRegistry::load(&path).expect("initial load")));

        assert!(live.current().lookup("second-key").is_none());

        std::fs::write(
            &path,
            r#"
[principals.ci]
role = "writer"
tokens = ["first-key"]

[principals.acme]
role = "reader"
tokens = ["second-key"]
"#,
        )
        .expect("rewrite");

        assert!(matches!(reload_once(&path, &live), Reload::Installed(_)));

        let now = live.current();
        assert_eq!(
            now.lookup("second-key").expect("new key resolves").role,
            Role::Reader,
            "a key added to the file must authenticate with the role the file gives it"
        );
        assert!(
            now.lookup("first-key").is_some(),
            "reloading must not disturb the keys that were already valid"
        );
    }

    #[test]
    fn a_removed_key_stops_authenticating() {
        let dir = tempdir();
        let path = write(
            &dir,
            "tokens.toml",
            r#"
[principals.ci]
role = "writer"
tokens = ["first-key"]

[principals.gone]
role = "reader"
tokens = ["revoke-me"]
"#,
        );
        let live = LiveRegistry::new(Arc::new(TokenRegistry::load(&path).expect("initial load")));
        assert!(live.current().lookup("revoke-me").is_some());

        std::fs::write(&path, ONE).expect("rewrite");
        assert!(matches!(reload_once(&path, &live), Reload::Installed(_)));

        assert!(
            live.current().lookup("revoke-me").is_none(),
            "deleting a principal from the file must revoke its key"
        );
    }

    #[test]
    fn a_broken_registry_is_refused_and_the_old_one_keeps_working() {
        let dir = tempdir();
        let path = write(&dir, "tokens.toml", ONE);
        let live = LiveRegistry::new(Arc::new(TokenRegistry::load(&path).expect("initial load")));

        std::fs::write(&path, "this is not toml {{{").expect("rewrite");
        assert!(matches!(reload_once(&path, &live), Reload::Rejected(_)));
        assert!(
            live.current().lookup("first-key").is_some(),
            "a syntactically broken registry must not lock out the keys already in force"
        );

        std::fs::remove_file(&path).expect("remove");
        assert!(matches!(reload_once(&path, &live), Reload::Rejected(_)));
        assert!(
            live.current().lookup("first-key").is_some(),
            "a registry file that momentarily disappears must not lock out anybody"
        );
    }

    #[test]
    fn a_registry_that_grants_the_same_thing_is_not_a_change() {
        let dir = tempdir();
        let path = write(&dir, "tokens.toml", ONE);
        let live = LiveRegistry::new(Arc::new(TokenRegistry::load(&path).expect("initial load")));

        // Same grants, different bytes: reordered, recommented, respaced.
        std::fs::write(
            &path,
            "# a comment\n[principals.ci]\ntokens   = [ \"first-key\" ]\nrole = \"writer\"\n",
        )
        .expect("rewrite");

        assert!(
            matches!(reload_once(&path, &live), Reload::Unchanged),
            "change is judged by what the registry grants, not by its bytes"
        );
    }

    #[test]
    fn a_rotated_secret_file_is_picked_up() {
        let dir = tempdir();
        write(&dir, "ci.key", "old-secret\n");
        let path = write(
            &dir,
            "tokens.toml",
            "[principals.ci]\nrole = \"writer\"\ntoken_files = [\"ci.key\"]\n",
        );
        let live = LiveRegistry::new(Arc::new(TokenRegistry::load(&path).expect("initial load")));
        assert!(live.current().lookup("old-secret").is_some());

        // Rotation writes the key file; the registry itself never changes.
        std::fs::write(dir.join("ci.key"), "new-secret\n").expect("rotate");

        assert!(matches!(reload_once(&path, &live), Reload::Installed(_)));
        assert!(
            live.current().lookup("new-secret").is_some(),
            "rotating a mounted secret must take effect without editing the registry"
        );
        assert!(
            live.current().lookup("old-secret").is_none(),
            "the rotated-out secret must stop working"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_watcher_installs_a_change_and_publishes_memberships() {
        let dir = tempdir();
        let path = write(
            &dir,
            "tokens.toml",
            "[principals.ci]\nrole = \"writer\"\ntokens = [\"first-key\"]\nparent = \"acme\"\n",
        );
        let live = LiveRegistry::new(Arc::new(TokenRegistry::load(&path).expect("initial load")));

        let published = Arc::new(AtomicUsize::new(0));
        let attempts = Arc::new(AtomicUsize::new(0));
        let task = tokio::spawn({
            let live = live.clone();
            let path = path.clone();
            let published = published.clone();
            let attempts = attempts.clone();
            async move {
                watch(path, live, std::time::Duration::from_secs(1), move |m| {
                    let published = published.clone();
                    let attempts = attempts.clone();
                    async move {
                        // Fail once, to prove the publish is retried rather
                        // than dropped when the authorizer is unreachable.
                        if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                            return Err("spicedb unreachable".to_owned());
                        }
                        published.store(m.len(), Ordering::SeqCst);
                        Ok(())
                    }
                })
                .await;
            }
        });

        std::fs::write(
            &path,
            "[principals.ci]\nrole = \"writer\"\ntokens = [\"first-key\"]\nparent = \"acme\"\n\
             \n[principals.two]\nrole = \"reader\"\ntokens = [\"second-key\"]\nparent = \"beta\"\n",
        )
        .expect("rewrite");

        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        tokio::task::yield_now().await;

        assert!(
            live.current().lookup("second-key").is_some(),
            "the watcher must install a change it finds on disk"
        );
        assert_eq!(
            published.load(Ordering::SeqCst),
            2,
            "both memberships must reach the authorizer after the first publish failed"
        );
        assert!(
            attempts.load(Ordering::SeqCst) >= 2,
            "a failed publish must be retried on a later tick"
        );

        task.abort();
    }

    #[test]
    fn zero_seconds_disables_reloading() {
        assert!(ReloadInterval::from_secs(0).is_disabled());
        assert_eq!(
            ReloadInterval::from_secs(0).as_duration(),
            None,
            "a disabled interval must have no duration to schedule"
        );
        assert_eq!(
            "30".parse::<ReloadInterval>()
                .expect("parses")
                .as_duration(),
            Some(std::time::Duration::from_secs(30))
        );
        assert!("soon".parse::<ReloadInterval>().is_err());
        assert_eq!(ReloadInterval::default().to_string(), "5");
    }
}
