use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use prost::Message;
use tokio::{sync::Mutex, task};
use trogon_atlas_proto as pb;
use trogon_atlas_store::refs::{entity_id, entity_kind};

/// Default ceiling on every `git` subprocess invoked by the mirror.
/// Override at startup via the `TROGON_ATLAS_GIT_TIMEOUT_SECS` env var.
const DEFAULT_GIT_SUBPROCESS_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum number of paths passed to a single `git add` invocation.
/// Keeps the argument list within OS limits on all platforms.
const GIT_ADD_CHUNK_SIZE: usize = 512;

/// Reject any id component (namespace, slug) that could escape the mirror
/// root. Delegates to `trogon_atlas_core::validate_id_component` so the
/// predicate matches the gRPC boundary; a future relaxation on either
/// side cannot bypass the other.
pub fn validate_path_component(label: &str, value: &str) -> Result<()> {
    trogon_atlas_core::validate_id_component(value).map_err(|e| anyhow!("{label} {e}"))
}

/// How often the caller polls a running git subprocess for exit.
///
/// The child is owned by this thread alone. An earlier revision moved the
/// `wait()` onto a helper thread to avoid polling and shared the `Child`
/// through an `Arc<Mutex<_>>` so the timeout path could kill it, but that
/// helper held the mutex for the whole `wait()`: when a child actually hung,
/// the timeout fired and then blocked forever taking the lock it needed to
/// kill. A timeout that deadlocks in the one case it exists for is worse than
/// no timeout, so the handle stays unshared and this thread polls instead.
const GIT_POLL_INTERVAL: Duration = Duration::from_millis(20);

fn run_git_with_timeout(
    mut cmd: Command,
    label: &str,
    timeout: Duration,
) -> Result<std::process::Output> {
    use std::{io::Read, process::Stdio, time::Instant};

    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawning git {label}"))?;

    // Drain both pipes on their own threads, concurrently with the wait.
    // `Child::wait` does not read them, so a subcommand whose output exceeds
    // the OS pipe buffer (64 KiB) blocks in `write()` while the parent blocks
    // waiting for an exit that can never come. `git status --porcelain` over a
    // few thousand changed paths is past that buffer, which is every batch an
    // initial mirror population or an `export-git` produces.
    fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        })
    }
    let stdout_reader = drain(child.stdout.take());
    let stderr_reader = drain(child.stderr.take());

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => return Err(e).with_context(|| format!("waiting on git {label}")),
        }
        if Instant::now() >= deadline {
            tracing::error!(
                label,
                timeout_secs = %timeout.as_secs(),
                "git subprocess exceeded timeout; killing"
            );
            let _ = child.kill();
            // Reap, so the killed child does not linger as a zombie, and so
            // the readers below see EOF rather than blocking on a pipe whose
            // write end is still held.
            let _ = child.wait();
            return Err(anyhow!("git {label} timed out after {timeout:?}"));
        }
        std::thread::sleep(GIT_POLL_INTERVAL);
    };

    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

#[derive(Clone, Debug)]
pub struct MirrorAuthor {
    pub name: String,
    pub email: String,
}

impl MirrorAuthor {
    #[must_use]
    pub fn is_blank(&self) -> bool {
        self.name.trim().is_empty() || self.email.trim().is_empty()
    }
}

// `Put` carries a full `pb::Entity` (~720 bytes) while `Delete` is ~60 bytes.
// Boxing the entity would break the public API consumers build match arms
// against, so we allow the size imbalance here.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum MirrorChange {
    Put(pb::Entity),
    Delete {
        kind: pb::EntityKind,
        id: pb::Id,
    },
    /// A file at the mirror root that is not an entity. Its only user is the
    /// ownership snapshot, which has to land in the same commit as the
    /// entities it describes: an owner list read from a different point in
    /// time than the entities is a restore that hands namespaces to the wrong
    /// parents. This is not a model edit and never travels through the
    /// changeset log the way a `Put` does.
    PutRootFile {
        name: String,
        contents: Vec<u8>,
    },
}

#[derive(Clone, Debug)]
pub struct MirrorBatch {
    pub changes: Vec<MirrorChange>,
    pub author: MirrorAuthor,
    pub message: String,
}

#[derive(Debug)]
pub struct GitMirror {
    root: PathBuf,
    default_author: MirrorAuthor,
    /// Whether to write human-readable `.txt` files alongside each `.binpb`.
    /// Read once from `TROGON_ATLAS_GIT_MIRROR_WRITE_TXT` at `open` time.
    write_txt: bool,
    /// Subprocess timeout resolved once at `open` from `TROGON_ATLAS_GIT_TIMEOUT_SECS`.
    git_timeout: Duration,
    /// Serializes the full file-write + git-add + git-commit sequence so
    /// concurrent `apply` calls cannot interleave index operations and produce
    /// index-lock conflicts or mixed commits.
    apply_lock: Mutex<()>,
}

/// A `git` invocation isolated from user/system configuration. The mirror
/// repo is fully machine-managed; inherited config (gpg signing, hooks,
/// prompts) can hang the daemon or corrupt the mirror's history shape.
fn git_command(root: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(root)
        .arg("-c")
        .arg("commit.gpgsign=false")
        .arg("-c")
        .arg("tag.gpgsign=false")
        .arg("-c")
        .arg("init.defaultBranch=main")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

impl GitMirror {
    pub fn open(root: PathBuf, default_author: MirrorAuthor) -> Result<Arc<Self>> {
        let write_txt = std::env::var_os("TROGON_ATLAS_GIT_MIRROR_WRITE_TXT").is_some();

        let git_timeout = match std::env::var("TROGON_ATLAS_GIT_TIMEOUT_SECS") {
            Ok(s) => match s.parse::<u64>() {
                Ok(n) if n > 0 => Duration::from_secs(n),
                Ok(_) => {
                    tracing::warn!(
                        value = %s,
                        default_secs = DEFAULT_GIT_SUBPROCESS_TIMEOUT.as_secs(),
                        "TROGON_ATLAS_GIT_TIMEOUT_SECS is zero; using default"
                    );
                    DEFAULT_GIT_SUBPROCESS_TIMEOUT
                }
                Err(_) => {
                    tracing::warn!(
                        value = %s,
                        default_secs = DEFAULT_GIT_SUBPROCESS_TIMEOUT.as_secs(),
                        "TROGON_ATLAS_GIT_TIMEOUT_SECS is not a valid integer; using default"
                    );
                    DEFAULT_GIT_SUBPROCESS_TIMEOUT
                }
            },
            Err(_) => DEFAULT_GIT_SUBPROCESS_TIMEOUT,
        };

        std::fs::create_dir_all(&root)
            .with_context(|| format!("creating git mirror dir at {}", root.display()))?;
        if !root.join(".git").exists() {
            let mut cmd = git_command(&root);
            cmd.arg("init").arg("--quiet");
            let out = run_git_with_timeout(cmd, "init", git_timeout)?;
            if !out.status.success() {
                return Err(anyhow!(
                    "git init failed: {}",
                    String::from_utf8_lossy(&out.stderr)
                ));
            }
        }
        Ok(Arc::new(Self {
            root,
            default_author,
            write_txt,
            git_timeout,
            apply_lock: Mutex::new(()),
        }))
    }

    pub async fn apply(self: &Arc<Self>, batch: MirrorBatch) -> Result<()> {
        // Acquire the async-aware mutex before entering the blocking pool so
        // concurrent callers queue here rather than racing on the git index.
        // The guard is held for the lifetime of the spawn_blocking call.
        let _guard = self.apply_lock.lock().await;
        let me = Arc::clone(self);
        task::spawn_blocking(move || me.apply_blocking(batch))
            .await
            .context("git mirror task join")?
    }

    fn apply_blocking(&self, batch: MirrorBatch) -> Result<()> {
        let mut staged_paths: Vec<PathBuf> = Vec::new();
        for change in &batch.changes {
            match change {
                MirrorChange::Put(entity) => {
                    let paths = self.write_put(entity)?;
                    staged_paths.extend(paths);
                }
                MirrorChange::Delete { kind, id } => {
                    let paths = self.write_delete(*kind, id)?;
                    staged_paths.extend(paths);
                }
                MirrorChange::PutRootFile { name, contents } => {
                    validate_path_component("root file", name)?;
                    let path = self.root.join(name);
                    std::fs::write(&path, contents)
                        .with_context(|| format!("writing {}", path.display()))?;
                    staged_paths.push(path);
                }
            }
        }

        // Stage only the paths touched by this batch. Chunking keeps the
        // argument list under ARG_MAX on platforms with tight limits.
        for chunk in staged_paths.chunks(GIT_ADD_CHUNK_SIZE) {
            let mut add_cmd = git_command(&self.root);
            add_cmd.arg("add").arg("--").args(chunk);
            let added = run_git_with_timeout(add_cmd, "add", self.git_timeout)?;
            if !added.status.success() {
                return Err(anyhow!(
                    "git add failed: {}",
                    String::from_utf8_lossy(&added.stderr)
                ));
            }
        }

        let mut status_cmd = git_command(&self.root);
        status_cmd.arg("status").arg("--porcelain");
        let status = run_git_with_timeout(status_cmd, "status", self.git_timeout)?;
        if status.stdout.is_empty() {
            return Ok(());
        }

        let author = if batch.author.is_blank() {
            self.default_author.clone()
        } else {
            batch.author
        };

        let mut commit_cmd = git_command(&self.root);
        commit_cmd
            .arg("commit")
            .arg("--quiet")
            .arg("--no-verify")
            .arg("-m")
            .arg(&batch.message)
            .env("GIT_AUTHOR_NAME", &author.name)
            .env("GIT_AUTHOR_EMAIL", &author.email)
            .env("GIT_COMMITTER_NAME", &author.name)
            .env("GIT_COMMITTER_EMAIL", &author.email);
        let out = run_git_with_timeout(commit_cmd, "commit", self.git_timeout)?;
        if !out.status.success() {
            return Err(anyhow!(
                "git commit failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        Ok(())
    }

    fn write_put(&self, entity: &pb::Entity) -> Result<Vec<PathBuf>> {
        let kind = entity_kind(entity).ok_or_else(|| anyhow!("entity has no kind"))?;
        let id = entity_id(entity).ok_or_else(|| anyhow!("entity has no id"))?;
        let dir = self.dir_for(kind, &id.namespace, &id.slug)?;
        std::fs::create_dir_all(&dir)?;
        let stem = format!("{}", id.version);
        let binpb = dir.join(format!("{stem}.binpb"));
        std::fs::write(&binpb, entity.encode_to_vec())?;
        let mut touched = vec![binpb];
        // Human-readable debug dump alongside the binary. Gated behind
        // TROGON_ATLAS_GIT_MIRROR_WRITE_TXT (default off) because the .txt
        // files double the mirror size and add noise to git diffs.
        if self.write_txt {
            let txt = dir.join(format!("{stem}.txt"));
            std::fs::write(&txt, format!("{entity:#?}\n"))?;
            touched.push(txt);
        }
        Ok(touched)
    }

    fn write_delete(&self, kind: pb::EntityKind, id: &pb::Id) -> Result<Vec<PathBuf>> {
        let dir = self.dir_for(kind, &id.namespace, &id.slug)?;
        let stem = format!("{}", id.version);
        let binpb = dir.join(format!("{stem}.binpb"));
        let txt = dir.join(format!("{stem}.txt"));
        // Check existence before removal so we can build the staging list
        // accurately. `git add -- <path>` fails if the path was never tracked
        // and no longer exists on disk.
        let binpb_tracked = binpb.exists();
        let txt_tracked = txt.exists();
        remove_unless_missing(&binpb)?;
        remove_unless_missing(&txt)?;
        let mut touched = Vec::new();
        if binpb_tracked {
            touched.push(binpb);
        }
        if txt_tracked {
            touched.push(txt);
        }
        Ok(touched)
    }

    fn dir_for(&self, kind: pb::EntityKind, namespace: &str, slug: &str) -> Result<PathBuf> {
        validate_path_component("namespace", namespace)?;
        validate_path_component("slug", slug)?;
        let mut p = self.root.clone();
        p.push(kind_dir(kind));
        p.push(namespace);
        p.push(slug);
        Ok(p)
    }
}

fn remove_unless_missing(path: &std::path::Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow!("removing {}: {e}", path.display())),
    }
}

fn kind_dir(kind: pb::EntityKind) -> &'static str {
    match kind {
        pb::EntityKind::Event => "events",
        pb::EntityKind::Command => "commands",
        pb::EntityKind::ReadModel => "read_models",
        pb::EntityKind::Processor => "processors",
        pb::EntityKind::Ui => "uis",
        pb::EntityKind::Persona => "personas",
        pb::EntityKind::Swimlane => "swimlanes",
        pb::EntityKind::CommandSlice => "command_slices",
        pb::EntityKind::ReadModelSlice => "read_model_slices",
        pb::EntityKind::AutomationSlice => "automation_slices",
        pb::EntityKind::UiSlice => "ui_slices",
        pb::EntityKind::Storyboard => "storyboards",
        pb::EntityKind::EventModel => "event_models",
        pb::EntityKind::Component => "components",
        pb::EntityKind::ExternalSystem => "external_systems",
        pb::EntityKind::Tracker => "trackers",
        pb::EntityKind::BoundedContext => "bounded_contexts",
        pb::EntityKind::Domain => "domains",
        pb::EntityKind::Subdomain => "subdomains",
        pb::EntityKind::Schema => "schemas",
        pb::EntityKind::Project => "projects",
        pb::EntityKind::Screen => "screens",
        pb::EntityKind::Term => "terms",
        pb::EntityKind::Ambiguity => "ambiguities",
        pb::EntityKind::ServiceLevelIndicator => "service_level_indicators",
        pb::EntityKind::ServiceLevelObjective => "service_level_objectives",
        pb::EntityKind::AlertPolicy => "alert_policies",
        pb::EntityKind::AlertNotificationTarget => "alert_notification_targets",
        pb::EntityKind::TypeLibrary => "type_libraries",
        pb::EntityKind::Unspecified => "unspecified",
    }
}

#[must_use]
pub fn commit_message(
    kind: pb::EntityKind,
    namespace: &str,
    slug: &str,
    version: u64,
    verb: &str,
) -> String {
    format!(
        "{}({}/{}@{}): {}",
        trogon_atlas_proto::canonical::kind_short(kind),
        namespace,
        slug,
        version,
        verb,
    )
}

#[must_use]
pub fn batch_commit_message(count: usize) -> String {
    format!("batch_mutate: {count} op(s)")
}

pub fn walk_repo(root: &Path) -> Result<Vec<pb::Entity>> {
    let mut out: Vec<pb::Entity> = Vec::new();
    walk_dir(root, &mut out)?;
    Ok(out)
}

/// Namespaces the given entities live in that the registry does not name.
///
/// The mirror is entity-only. Ownership lives in its own bucket, which no
/// commit here has ever touched, so a store restored from a mirror comes back
/// with every entity and no owner. That is not a quiet state: an unregistered
/// namespace is invisible to every caller bound to a parent, and the first
/// caller to write into it claims it, which after a restore is whoever
/// happens to reconnect first rather than whoever owned it. Naming them is
/// the difference between a restore that looks clean and one an operator can
/// act on.
pub fn unregistered_namespaces(
    entities: &[pb::Entity],
    registered: &[trogon_atlas_store::NamespaceRecord],
) -> Vec<String> {
    let known: std::collections::BTreeSet<&str> =
        registered.iter().map(|r| r.id.as_str()).collect();
    let mut missing: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for entity in entities {
        let Some(id) = entity_id(entity) else {
            continue;
        };
        if !known.contains(id.namespace.as_str()) {
            missing.insert(id.namespace.clone());
        }
    }
    missing.into_iter().collect()
}

/// Name of the ownership snapshot at the mirror root.
///
/// Deliberately not under a kind directory and deliberately not a `.binpb`:
/// [`walk_repo`] only collects entities, so the snapshot travels with the
/// mirror without ever being mistaken for one.
pub const NAMESPACE_SNAPSHOT_FILE: &str = "namespaces.json";

/// Schema version of [`NamespaceSnapshot`]. A reader that meets a version it
/// does not know refuses rather than guessing which fields it is missing.
pub const NAMESPACE_SNAPSHOT_VERSION: u32 = 1;

/// One registry row as it appears in the snapshot.
///
/// JSON rather than the protobuf the store keeps, because the mirror is a git
/// repository an operator reads with `git log -p`. Ownership is the part of a
/// restore that decides who can see what, so a change to it has to be legible
/// in a diff; the entities can afford to be opaque, this cannot.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NamespaceSnapshotRow {
    pub id: String,
    pub name: String,
    pub parent: String,
    pub created_at: String,
    pub created_by: String,
}

/// The ownership half of a mirror, as of the export that wrote it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NamespaceSnapshot {
    pub version: u32,
    pub exported_at: String,
    pub namespaces: Vec<NamespaceSnapshotRow>,
}

impl NamespaceSnapshot {
    /// Build a snapshot from live registry rows.
    ///
    /// Provisional rows are left out. Such a row exists only to hold a name
    /// open for one branch, and branches do not live in the mirror; restoring
    /// one would reserve a name on behalf of a branch that no longer exists,
    /// which is the reservation [`trogon_atlas_store::NamespaceTenure`] was
    /// split in two to prevent. The caller is told how many were dropped so
    /// the count in the log is not mistaken for the registry's size.
    #[must_use]
    pub fn build(
        records: &[trogon_atlas_store::NamespaceRecord],
        exported_at: String,
    ) -> (Self, usize) {
        let mut namespaces: Vec<NamespaceSnapshotRow> = records
            .iter()
            .filter(|r| r.tenure == trogon_atlas_store::NamespaceTenure::Permanent)
            .map(|r| NamespaceSnapshotRow {
                id: r.id.to_string(),
                name: r.name.to_string(),
                parent: r.parent.to_string(),
                created_at: r.created_at.clone(),
                created_by: r.created_by.clone(),
            })
            .collect();
        // Sorted so re-exporting an unchanged registry produces no diff, and
        // a real ownership change shows up as exactly the lines that moved.
        namespaces.sort_by(|a, b| a.id.cmp(&b.id));
        let skipped = records.len() - namespaces.len();
        (
            Self {
                version: NAMESPACE_SNAPSHOT_VERSION,
                exported_at,
                namespaces,
            },
            skipped,
        )
    }

    /// Pretty-printed bytes, newline-terminated, for the mirror commit.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut bytes = serde_json::to_vec_pretty(self).context("encoding namespace snapshot")?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Parse rows back into registry records.
    ///
    /// Every field is re-parsed through its value type rather than trusted:
    /// the snapshot is a file on disk that anything could have edited between
    /// export and restore, and it names the owner of every namespace.
    pub fn to_records(&self) -> Result<Vec<trogon_atlas_store::NamespaceRecord>> {
        use trogon_atlas_core::{NamespaceId, NamespaceName, OwnerId};
        if self.version != NAMESPACE_SNAPSHOT_VERSION {
            return Err(anyhow!(
                "{NAMESPACE_SNAPSHOT_FILE} is version {}, but this build only reads version {NAMESPACE_SNAPSHOT_VERSION}",
                self.version
            ));
        }
        let mut out = Vec::with_capacity(self.namespaces.len());
        for row in &self.namespaces {
            out.push(trogon_atlas_store::NamespaceRecord {
                id: NamespaceId::parse(&row.id)
                    .map_err(|e| anyhow!("namespace snapshot id {:?}: {e}", row.id))?,
                name: NamespaceName::parse(&row.name)
                    .map_err(|e| anyhow!("namespace snapshot name {:?}: {e}", row.name))?,
                parent: OwnerId::parse(&row.parent)
                    .map_err(|e| anyhow!("namespace snapshot parent {:?}: {e}", row.parent))?,
                created_at: row.created_at.clone(),
                created_by: row.created_by.clone(),
                tenure: trogon_atlas_store::NamespaceTenure::Permanent,
            });
        }
        Ok(out)
    }
}

/// Read the ownership snapshot from a mirror tree, or `None` when the mirror
/// predates `export-git` and carries no ownership at all.
pub fn read_namespace_snapshot(root: &Path) -> Result<Option<NamespaceSnapshot>> {
    let path = root.join(NAMESPACE_SNAPSHOT_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(anyhow!("reading {}: {e}", path.display())),
    };
    let snapshot: NamespaceSnapshot =
        serde_json::from_slice(&bytes).with_context(|| format!("decoding {}", path.display()))?;
    Ok(Some(snapshot))
}

fn walk_dir(dir: &Path, out: &mut Vec<pb::Entity>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            if path.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            walk_dir(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("binpb") {
            let bytes = std::fs::read(&path)?;
            let entity = pb::Entity::decode(bytes.as_slice())
                .with_context(|| format!("decoding {}", path.display()))?;
            out.push(entity);
        }
    }
    Ok(())
}

#[cfg(test)]
mod run_git_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::time::Instant;

    use super::*;

    /// `Child::wait` does not drain the child's pipes, so reading them only
    /// after it returns hangs forever once the output passes the OS pipe
    /// buffer. 1 MiB is comfortably past the 64 KiB buffer, and well under
    /// what `git status --porcelain` emits for an initial mirror population.
    #[test]
    fn output_larger_than_the_pipe_buffer_does_not_deadlock() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg("yes 0123456789012345678901234567890123456789 | head -c 1048576");

        let out = run_git_with_timeout(cmd, "bulk", Duration::from_secs(30))
            .expect("a command that outsizes the pipe buffer should still complete");

        assert!(out.status.success());
        assert_eq!(out.stdout.len(), 1_048_576);
    }

    /// The timeout exists to bound a wedged git. An earlier revision shared the
    /// `Child` through a mutex that the waiting thread held for the whole
    /// `wait()`, so the kill blocked on a lock that was never released and the
    /// timeout could not fire in the one case it was written for.
    #[test]
    fn a_hung_child_is_killed_at_the_timeout() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("sleep 120");

        let started = Instant::now();
        let err = run_git_with_timeout(cmd, "wedged", Duration::from_millis(300))
            .expect_err("a child that outlives the timeout must be reported as a timeout");

        assert!(err.to_string().contains("timed out"), "{err}");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "kill path took {:?}, so the timeout did not actually bound the child",
            started.elapsed()
        );
    }

    /// A child killed at the timeout still holds the write end of both pipes
    /// until it is reaped. Returning without reaping leaves the reader threads
    /// blocked on a pipe that never reaches EOF.
    #[test]
    fn a_hung_child_that_also_writes_is_still_bounded() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("echo started; sleep 120");

        let started = Instant::now();
        let err = run_git_with_timeout(cmd, "wedged-noisy", Duration::from_millis(300))
            .expect_err("a child that outlives the timeout must be reported as a timeout");

        assert!(err.to_string().contains("timed out"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn stderr_is_captured_alongside_a_failing_status() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("echo boom >&2; exit 3");

        let out = run_git_with_timeout(cmd, "failing", Duration::from_secs(30)).unwrap();

        assert_eq!(out.status.code(), Some(3));
        assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "boom");
    }
}

#[cfg(test)]
mod validate_path_component_tests {
    use super::validate_path_component;

    #[test]
    fn rejects_path_traversal() {
        for v in ["..", "../etc", "a/b", "."] {
            assert!(
                validate_path_component("test", v).is_err(),
                "accepted hostile {v:?}"
            );
        }
    }

    #[test]
    fn accepts_periods_in_slug() {
        assert!(validate_path_component("slug", "order.placed").is_ok());
    }
}

#[cfg(test)]
mod unregistered_namespaces_tests {
    #![allow(clippy::unwrap_used)]

    use trogon_atlas_core::namespace::{NamespaceId, NamespaceName, OwnerId};
    use trogon_atlas_store::{NamespaceRecord, NamespaceTenure};

    use super::*;

    fn entity(ns: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(pb::Id {
                    namespace: ns.into(),
                    slug: "order-placed".into(),
                    version: 1,
                }),
                ..Default::default()
            })),
        }
    }

    fn record(id: &str, parent: &str) -> NamespaceRecord {
        NamespaceRecord {
            id: NamespaceId::parse(id).unwrap(),
            name: NamespaceName::parse(id).unwrap(),
            parent: OwnerId::parse(parent).unwrap(),
            created_at: "2026-01-01T00:00:00Z".into(),
            created_by: "migration".into(),
            tenure: NamespaceTenure::Permanent,
        }
    }

    #[test]
    fn names_every_namespace_the_registry_does_not() {
        let entities = vec![entity("orders"), entity("billing"), entity("orders")];
        let missing = unregistered_namespaces(&entities, &[record("orders", "acme")]);
        assert_eq!(missing, vec!["billing".to_string()]);
    }

    #[test]
    fn reports_nothing_when_every_namespace_is_owned() {
        let entities = vec![entity("orders"), entity("billing")];
        let registered = vec![record("orders", "acme"), record("billing", "globex")];
        assert_eq!(
            unregistered_namespaces(&entities, &registered),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_registry_row_under_any_owner_counts_as_restored() {
        // The question this answers is whether the row exists at all, not who
        // holds it: a namespace with a row is somebody's, and the operator has
        // to be told only about the ones that are nobody's.
        let missing = unregistered_namespaces(&[entity("orders")], &[record("orders", "globex")]);
        assert_eq!(missing, Vec::<String>::new());
    }

    #[test]
    fn an_empty_registry_leaves_every_namespace_unowned() {
        // The restore case: the mirror carried the entities and nothing else.
        let entities = vec![entity("orders"), entity("billing")];
        let missing = unregistered_namespaces(&entities, &[]);
        assert_eq!(missing, vec!["billing".to_string(), "orders".to_string()]);
    }
}

#[cfg(test)]
mod namespace_snapshot_tests {
    #![allow(clippy::unwrap_used)]

    use trogon_atlas_core::namespace::{NamespaceId, NamespaceName, OwnerId};
    use trogon_atlas_store::{NamespaceRecord, NamespaceTenure};

    use super::*;

    fn tmp_dir(label: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!(
            "trogon-atlas-snapshot-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn record(id: &str, name: &str, parent: &str, tenure: NamespaceTenure) -> NamespaceRecord {
        NamespaceRecord {
            id: NamespaceId::parse(id).unwrap(),
            name: NamespaceName::parse(name).unwrap(),
            parent: OwnerId::parse(parent).unwrap(),
            created_at: "2026-01-01T00:00:00Z".into(),
            created_by: "alice".into(),
            tenure,
        }
    }

    #[test]
    fn carries_the_id_the_entity_keys_use() {
        // A minted id cannot be rebuilt from the name, and every entity key
        // embeds it, so a snapshot that dropped it would restore ownership
        // that points at nothing.
        let minted = record(
            "ns_0198f0a1b2c3d4e5f60718293a4b5c6d",
            "orders",
            "acme",
            NamespaceTenure::Permanent,
        );
        let (snapshot, _) = NamespaceSnapshot::build(&[minted], "2026-01-02T00:00:00Z".into());
        assert_eq!(
            snapshot.namespaces[0].id,
            "ns_0198f0a1b2c3d4e5f60718293a4b5c6d"
        );
        assert_eq!(snapshot.namespaces[0].name, "orders");
    }

    #[test]
    fn leaves_provisional_rows_out_and_counts_them() {
        // A provisional row is a name held open for one branch. The mirror has
        // no branches, so restoring it would reserve a name for a branch that
        // cannot exist after the restore.
        let records = vec![
            record("orders", "orders", "acme", NamespaceTenure::Permanent),
            record(
                "spike",
                "spike",
                "acme",
                NamespaceTenure::for_branch(Some("try-it")),
            ),
        ];
        let (snapshot, skipped) = NamespaceSnapshot::build(&records, "now".into());
        assert_eq!(skipped, 1);
        assert_eq!(snapshot.namespaces.len(), 1);
        assert_eq!(snapshot.namespaces[0].id, "orders");
    }

    #[test]
    fn orders_rows_so_an_unchanged_registry_produces_no_diff() {
        let records = vec![
            record("zulu", "zulu", "acme", NamespaceTenure::Permanent),
            record("alpha", "alpha", "acme", NamespaceTenure::Permanent),
        ];
        let (a, _) = NamespaceSnapshot::build(&records, "now".into());
        let reversed: Vec<_> = records.into_iter().rev().collect();
        let (b, _) = NamespaceSnapshot::build(&reversed, "now".into());
        assert_eq!(a.to_bytes().unwrap(), b.to_bytes().unwrap());
        assert_eq!(a.namespaces[0].id, "alpha");
    }

    #[test]
    fn round_trips_through_the_file() {
        let dir = tmp_dir("roundtrip");
        let records = vec![record(
            "orders",
            "orders",
            "acme",
            NamespaceTenure::Permanent,
        )];
        let (snapshot, _) = NamespaceSnapshot::build(&records, "2026-01-02T00:00:00Z".into());
        std::fs::write(
            dir.join(NAMESPACE_SNAPSHOT_FILE),
            snapshot.to_bytes().unwrap(),
        )
        .unwrap();

        let read = read_namespace_snapshot(&dir).unwrap().unwrap();
        assert_eq!(read, snapshot);
        let restored = read.to_records().unwrap();
        assert_eq!(restored[0].id.as_str(), "orders");
        assert_eq!(restored[0].parent.as_str(), "acme");
        // Preserved rather than restamped: who claimed it and when is part of
        // what a restore is putting back.
        assert_eq!(restored[0].created_by, "alice");
        assert_eq!(restored[0].created_at, "2026-01-01T00:00:00Z");
    }

    #[test]
    fn a_mirror_with_no_snapshot_reads_as_absent_rather_than_empty() {
        // "No ownership recorded" and "ownership recorded as nobody" are
        // different answers: the first is a mirror from before export-git and
        // has to be refused, the second would silently unown everything.
        let dir = tmp_dir("absent");
        assert!(read_namespace_snapshot(&dir).unwrap().is_none());
    }

    #[test]
    fn refuses_a_snapshot_written_by_a_newer_build() {
        let snapshot = NamespaceSnapshot {
            version: NAMESPACE_SNAPSHOT_VERSION + 1,
            exported_at: "now".into(),
            namespaces: vec![],
        };
        let err = snapshot.to_records().unwrap_err().to_string();
        assert!(err.contains("only reads version"), "{err}");
    }

    #[test]
    fn refuses_a_row_whose_owner_is_not_a_valid_owner_id() {
        // The snapshot is a file on disk that names the owner of every
        // namespace, so nothing in it is trusted without re-parsing.
        let snapshot = NamespaceSnapshot {
            version: NAMESPACE_SNAPSHOT_VERSION,
            exported_at: "now".into(),
            namespaces: vec![NamespaceSnapshotRow {
                id: "orders".into(),
                name: "orders".into(),
                parent: "../etc".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
                created_by: "alice".into(),
            }],
        };
        let err = snapshot.to_records().unwrap_err().to_string();
        assert!(err.contains("parent"), "{err}");
    }
}

#[cfg(test)]
mod apply_tests {
    #![allow(clippy::unwrap_used)]

    use std::{path::Path, sync::Mutex};

    use super::*;

    // Serialize tests that mutate env vars; Rust runs tests in the same
    // process and env is global mutable state.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn default_author() -> MirrorAuthor {
        MirrorAuthor {
            name: "Test".into(),
            email: "test@example.com".into(),
        }
    }

    fn make_entity(ns: &str, slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(pb::Id {
                    namespace: ns.into(),
                    slug: slug.into(),
                    version: 1,
                }),
                ..Default::default()
            })),
        }
    }

    fn tmp_dir(label: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!("trogon-atlas-mirror-{label}-{pid}-{nanos}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn git_log_count(root: &Path) -> usize {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["log", "--oneline"])
            .output()
            .unwrap();
        let s = String::from_utf8_lossy(&out.stdout);
        s.lines().count()
    }

    fn git_show_files(root: &Path, commit: &str) -> Vec<String> {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["show", "--name-only", "--format=", commit])
            .output()
            .unwrap();
        let s = String::from_utf8_lossy(&out.stdout);
        s.lines()
            .filter(|l| !l.is_empty())
            .map(std::string::ToString::to_string)
            .collect()
    }

    fn open_mirror(dir: &Path) -> Arc<GitMirror> {
        // Hold ENV_LOCK while touching env vars and opening the mirror so
        // concurrent tests cannot observe a partially-set environment.
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::env::remove_var("TROGON_ATLAS_GIT_MIRROR_WRITE_TXT");
        std::env::remove_var("TROGON_ATLAS_GIT_TIMEOUT_SECS");
        GitMirror::open(dir.to_path_buf(), default_author()).unwrap()
    }

    #[tokio::test]
    async fn root_file_lands_in_the_same_commit_as_the_entities() {
        // The ownership snapshot has to be atomic with the entities it
        // describes. Two commits would leave a window whose checkout hands
        // namespaces to the wrong owners.
        let root = tmp_dir("rootfile");
        let mirror = open_mirror(&root);

        mirror
            .apply(MirrorBatch {
                changes: vec![
                    MirrorChange::Put(make_entity("acme", "order-placed")),
                    MirrorChange::PutRootFile {
                        name: NAMESPACE_SNAPSHOT_FILE.into(),
                        contents: b"{}\n".to_vec(),
                    },
                ],
                author: default_author(),
                message: "export".into(),
            })
            .await
            .unwrap();

        assert_eq!(git_log_count(&root), 1);
        let files = git_show_files(&root, "HEAD");
        assert!(
            files.iter().any(|f| f == NAMESPACE_SNAPSHOT_FILE),
            "snapshot missing from the commit: {files:?}"
        );
        assert!(
            files.iter().any(|f| std::path::Path::new(f)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("binpb"))),
            "entity missing from the commit: {files:?}"
        );
    }

    #[tokio::test]
    async fn a_root_file_cannot_escape_the_mirror() {
        let root = tmp_dir("escape");
        let mirror = open_mirror(&root);

        let err = mirror
            .apply(MirrorBatch {
                changes: vec![MirrorChange::PutRootFile {
                    name: "../escaped.json".into(),
                    contents: b"x".to_vec(),
                }],
                author: default_author(),
                message: "nope".into(),
            })
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("root file"), "{err}");
        assert!(!root.join("../escaped.json").exists());
    }

    #[tokio::test]
    async fn stray_file_in_mirror_root_is_not_committed() {
        let root = tmp_dir("stray");
        let mirror = open_mirror(&root);

        // Place a stray file in the mirror root before the first apply.
        let stray = root.join("STRAY.txt");
        std::fs::write(&stray, "should not be committed").unwrap();

        let entity = make_entity("acme", "order-placed");
        let batch = MirrorBatch {
            changes: vec![MirrorChange::Put(entity.clone())],
            author: default_author(),
            message: "add order-placed".into(),
        };
        mirror.apply(batch).await.unwrap();

        // The commit should exist.
        assert_eq!(git_log_count(&root), 1, "expected exactly one commit");

        // The stray file must NOT be tracked by git.
        let files = git_show_files(&root, "HEAD");
        assert!(
            !files.iter().any(|f| f.contains("STRAY")),
            "stray file was committed: {files:?}"
        );

        // The entity binpb must be present in the commit.
        assert!(
            files.iter().any(|f| {
                std::path::Path::new(f)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("binpb"))
            }),
            "entity binpb missing from commit: {files:?}"
        );
    }

    #[tokio::test]
    async fn delete_stages_removal_correctly() {
        let root = tmp_dir("delete");
        let mirror = open_mirror(&root);

        let entity = make_entity("acme", "order-placed");
        let put_batch = MirrorBatch {
            changes: vec![MirrorChange::Put(entity.clone())],
            author: default_author(),
            message: "add".into(),
        };
        mirror.apply(put_batch).await.unwrap();

        let id = pb::Id {
            namespace: "acme".into(),
            slug: "order-placed".into(),
            version: 1,
        };
        let del_batch = MirrorBatch {
            changes: vec![MirrorChange::Delete {
                kind: pb::EntityKind::Event,
                id: id.clone(),
            }],
            author: default_author(),
            message: "delete".into(),
        };
        mirror.apply(del_batch).await.unwrap();

        // Two commits total (add + delete).
        assert_eq!(git_log_count(&root), 2);

        // The file must not exist on disk.
        let dir = root.join("events").join("acme").join("order-placed");
        assert!(!dir.join("1.binpb").exists(), "binpb file should be gone");
    }

    #[tokio::test]
    async fn write_txt_env_controls_txt_files() {
        let root = tmp_dir("write-txt");

        // Enable txt output. Serialize with other env-mutating tests via ENV_LOCK.
        let mirror = {
            let _guard = ENV_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::env::set_var("TROGON_ATLAS_GIT_MIRROR_WRITE_TXT", "1");
            let m = GitMirror::open(root.clone(), default_author()).unwrap();
            std::env::remove_var("TROGON_ATLAS_GIT_MIRROR_WRITE_TXT");
            m
        };

        let entity = make_entity("acme", "order-placed");
        let batch = MirrorBatch {
            changes: vec![MirrorChange::Put(entity)],
            author: default_author(),
            message: "add".into(),
        };
        mirror.apply(batch).await.unwrap();

        let dir = root.join("events").join("acme").join("order-placed");
        assert!(dir.join("1.binpb").exists(), "binpb missing");
        assert!(
            dir.join("1.txt").exists(),
            "txt file should exist when env var was set"
        );
    }

    #[tokio::test]
    async fn no_txt_files_without_env_var() {
        let root = tmp_dir("no-txt");

        let mirror = open_mirror(&root);

        let entity = make_entity("acme", "order-placed");
        let batch = MirrorBatch {
            changes: vec![MirrorChange::Put(entity)],
            author: default_author(),
            message: "add".into(),
        };
        mirror.apply(batch).await.unwrap();

        let dir = root.join("events").join("acme").join("order-placed");
        assert!(dir.join("1.binpb").exists(), "binpb missing");
        assert!(
            !dir.join("1.txt").exists(),
            "txt file should not exist without env var"
        );
    }
}
