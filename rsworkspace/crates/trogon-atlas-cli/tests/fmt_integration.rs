#![allow(clippy::unwrap_used, clippy::expect_used)]

// `trogon-atlas fmt` needs no server, so these tests exercise it two ways:
//
// - directly through `trogon_atlas_client::fmt`, walking every manifest under
//   this crate's examples/manifests and the repository's shared examples/manifests
//   IN MEMORY, asserting
//   semantic preservation (parse(fmt(x)) == parse(x)) and idempotency
//   (fmt(fmt(x)) == fmt(x)). Nothing under those directories is ever
//   written back.
// - through the compiled `trogon-atlas` binary, on throwaway temp files, for the
//   CLI-level behavior (--check exit code, comment refusal, --write).

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use trogon_atlas_client::{fmt, manifest};
use trogon_atlas_core::transcode::entity_json_value;

fn fixture_dirs() -> Vec<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut dirs = vec![
        manifest_dir.join("examples/manifests"),
        manifest_dir.join("../../../examples/manifests"),
    ];
    if let Some(extra) = std::env::var_os("TROGON_ATLAS_EXTRA_FIXTURE_DIRS") {
        dirs.extend(std::env::split_paths(&extra));
    }
    dirs.retain(|d| d.is_dir());
    dirs
}

fn collect_yaml_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
    {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            collect_yaml_files(&path, out);
            continue;
        }
        let is_yaml = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("yaml") || e.eq_ignore_ascii_case("yml"));
        if is_yaml {
            out.push(path);
        }
    }
}

/// (kind, id, entity-as-canonical-json) is the semantic content of a
/// manifest document; the `source` label is allowed to differ between a
/// parse of the original file and a parse of its canonicalized text.
///
/// Comparing via canonical JSON rather than raw `pb::Entity` equality
/// matters here: `google.protobuf.Struct` scenario payloads round-trip
/// through `Any` with nondeterministic protobuf map encoding order (see
/// `apply::classify`'s same fallback), so two semantically identical
/// entities can disagree byte-for-byte while agreeing as JSON values.
type Semantics = (
    trogon_atlas_proto::EntityKind,
    trogon_atlas_proto::Id,
    serde_json::Value,
);

fn semantics(manifests: &[manifest::LoadedManifest]) -> Vec<Semantics> {
    manifests
        .iter()
        .map(|m| {
            let json = entity_json_value(&m.entity)
                .unwrap_or_else(|e| panic!("{}: entity_json_value: {e:#}", m.source));
            (m.kind, m.id.clone(), json)
        })
        .collect()
}

#[test]
fn fmt_preserves_semantics_and_is_idempotent_over_every_fixture_manifest() {
    let mut files = Vec::new();
    for dir in fixture_dirs() {
        collect_yaml_files(&dir, &mut files);
    }
    let min_expected = if std::env::var_os("TROGON_ATLAS_EXTRA_FIXTURE_DIRS").is_some() {
        100
    } else {
        20
    };
    assert!(
        files.len() > min_expected,
        "expected to find a sizeable fixture set, got {}",
        files.len()
    );

    for path in &files {
        let label = path.display().to_string();
        let original = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{label}: {e}"));

        let original_manifests =
            manifest::parse_str(&original, &label).unwrap_or_else(|e| panic!("{label}: {e:#}"));
        let canonical_once = fmt::canonicalize(&original, &label)
            .unwrap_or_else(|e| panic!("{label}: canonicalize: {e:#}"));
        let canonical_manifests = manifest::parse_str(&canonical_once, &label)
            .unwrap_or_else(|e| panic!("{label}: reparsing canonical output: {e:#}"));

        assert_eq!(
            semantics(&original_manifests),
            semantics(&canonical_manifests),
            "{label}: canonicalization changed the parsed entity"
        );

        let canonical_twice = fmt::canonicalize(&canonical_once, &label)
            .unwrap_or_else(|e| panic!("{label}: canonicalize twice: {e:#}"));
        assert_eq!(
            canonical_once, canonical_twice,
            "{label}: canonicalize is not idempotent"
        );
    }
}

fn trogon_atlas(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args(args)
        .output()
        .expect("trogon-atlas must spawn")
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("trogon-atlas-fmt-test-{tag}-{}", uuid_like()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{nanos}-{:?}", std::thread::current().id())
}

const NON_CANONICAL_EVENT: &str = "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: Event\nmetadata: {namespace: shop, name: order.placed}\nspec: {title: Order placed}\n";

#[test]
fn fmt_default_mode_prints_canonical_yaml_for_a_single_file() {
    let dir = tempdir("stdout");
    let path = dir.join("event.yaml");
    std::fs::write(&path, NON_CANONICAL_EVENT).unwrap();

    let out = trogon_atlas(&["fmt", path.to_str().unwrap()]);
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("apiVersion:"), "{stdout}");
    assert!(stdout.contains("namespace: shop"), "{stdout}");

    // Nothing was written: the source file is untouched.
    let unchanged = std::fs::read_to_string(&path).unwrap();
    assert_eq!(unchanged, NON_CANONICAL_EVENT);
}

#[test]
fn fmt_default_mode_rejects_more_than_one_resolved_file() {
    let dir = tempdir("stdout-multi");
    std::fs::write(dir.join("a.yaml"), NON_CANONICAL_EVENT).unwrap();
    std::fs::write(
        dir.join("b.yaml"),
        NON_CANONICAL_EVENT.replace("order.placed", "order.cancelled"),
    )
    .unwrap();

    let out = trogon_atlas(&["fmt", dir.to_str().unwrap()]);
    assert!(!out.status.success(), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("exactly one file"),
        "expected a stdout-mode arity error, got: {stderr}"
    );
}

#[test]
fn fmt_write_rewrites_in_place_and_becomes_a_no_op_on_rerun() {
    let dir = tempdir("write");
    let path = dir.join("event.yaml");
    std::fs::write(&path, NON_CANONICAL_EVENT).unwrap();

    let first = trogon_atlas(&["fmt", "--write", path.to_str().unwrap()]);
    assert!(first.status.success(), "{first:?}");
    let stdout = String::from_utf8_lossy(&first.stdout);
    assert!(stdout.contains("event.yaml"), "{stdout}");

    let rewritten = std::fs::read_to_string(&path).unwrap();
    assert_ne!(rewritten, NON_CANONICAL_EVENT);
    assert!(rewritten.starts_with("apiVersion:"));

    let second = trogon_atlas(&["fmt", "--write", path.to_str().unwrap()]);
    assert!(second.status.success(), "{second:?}");
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        stderr.contains("0 file(s) reformatted"),
        "rerunning fmt --write on canonical output must be a no-op: {stderr}"
    );

    let still = std::fs::read_to_string(&path).unwrap();
    assert_eq!(still, rewritten, "second run must not touch the file");
}

#[test]
fn fmt_check_reports_files_that_would_change_and_exits_nonzero() {
    let dir = tempdir("check");
    let path = dir.join("event.yaml");
    std::fs::write(&path, NON_CANONICAL_EVENT).unwrap();

    let dirty = trogon_atlas(&["fmt", "--check", path.to_str().unwrap()]);
    assert_eq!(dirty.status.code(), Some(1), "{dirty:?}");
    let stdout = String::from_utf8_lossy(&dirty.stdout);
    assert!(stdout.contains("event.yaml"), "{stdout}");

    let written = trogon_atlas(&["fmt", "--write", path.to_str().unwrap()]);
    assert!(written.status.success(), "{written:?}");

    let clean = trogon_atlas(&["fmt", "--check", path.to_str().unwrap()]);
    assert_eq!(clean.status.code(), Some(0), "{clean:?}");
}

#[test]
fn fmt_refuses_a_file_with_a_comment_unless_forced() {
    let dir = tempdir("comment");
    let path = dir.join("event.yaml");
    let with_comment = format!("# a note about this event\n{NON_CANONICAL_EVENT}");
    std::fs::write(&path, &with_comment).unwrap();

    let refused = trogon_atlas(&["fmt", "--check", path.to_str().unwrap()]);
    assert!(!refused.status.success(), "{refused:?}");
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(stderr.contains("comment"), "{stderr}");

    let forced = trogon_atlas(&["fmt", "--force", path.to_str().unwrap()]);
    assert!(forced.status.success(), "{forced:?}");
    let stdout = String::from_utf8_lossy(&forced.stdout);
    assert!(!stdout.contains('#'), "{stdout}");
    assert!(stdout.contains("namespace: shop"), "{stdout}");
}

#[test]
fn fmt_write_and_check_agree_on_the_committed_shop_example() {
    // examples/manifests/shop.yaml carries hand-written documentation
    // comments on purpose; fmt must refuse it without --force and leave it
    // untouched either way (this test never writes to the real example).
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let shop = manifest_dir.join("examples/manifests/shop.yaml");
    let original = std::fs::read_to_string(&shop).unwrap();

    let refused = trogon_atlas(&["fmt", "--check", shop.to_str().unwrap()]);
    assert!(!refused.status.success(), "{refused:?}");
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("comment"),
        "{refused:?}"
    );

    let still = std::fs::read_to_string(&shop).unwrap();
    assert_eq!(still, original, "fmt must never write without --write");
}
