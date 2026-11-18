#![allow(clippy::unwrap_used, clippy::expect_used)]

// `trogon-atlas walk-scenarios -f` walks manifest files offline, so these
// tests run the compiled binary against the client crate's walk fixtures
// without a server.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

fn walk_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../trogon-atlas-client/tests/fixtures/walk")
        .join(name)
}

fn walk_cases(cases: &Path, extra: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args(extra)
        .arg("walk-scenarios")
        .arg("-f")
        .arg(walk_fixture("model.yaml"))
        .arg("--cases")
        .arg(cases)
        // An unreachable endpoint proves the offline walk never connects.
        .env("TROGON_ATLAS_ENDPOINT", "http://127.0.0.1:1")
        .output()
        .unwrap()
}

fn walk(extra: &[&str]) -> std::process::Output {
    walk_cases(&walk_fixture("cases.yaml"), extra)
}

#[test]
fn error_findings_exit_one_with_text_lines() {
    let out = walk(&[]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("WALK_COMMAND_FIELD_UNSUPPLIED [walk-reviews/approve-review-42 step 2]"),
        "{stdout}"
    );
    assert!(stdout.contains("1/3 case(s) passed"), "{stdout}");
}

#[test]
fn json_format_prints_one_outcome_with_the_report() {
    let out = walk(&["--format", "json"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let outcome: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(outcome["schema"], "trogon-atlas.result.v1");
    let findings = outcome["data"]["findings"].as_array().unwrap();
    assert_eq!(findings[0]["code"], "WALK_COMMAND_FIELD_UNSUPPLIED");
    assert_eq!(findings[0]["severity"], "SEVERITY_ERROR");
    assert_eq!(outcome["data"]["cases"].as_array().unwrap().len(), 3);
}

#[test]
fn a_malformed_case_file_is_a_validation_failure() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.yaml");
    std::fs::write(
        &bad,
        "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: Specification\nmetadata: {namespace: n, name: c}\nspec: {stepz: []}\n",
    )
    .unwrap();
    let out = walk_cases(&bad, &[]);
    assert_eq!(out.status.code(), Some(5), "{out:?}");
}

#[test]
fn case_paths_with_no_case_are_a_validation_failure() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("empty.yaml"), "---\n").unwrap();
    let out = walk_cases(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(5), "{out:?}");
}
