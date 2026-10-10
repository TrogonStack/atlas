#![allow(clippy::unwrap_used, clippy::expect_used)]

// Local argument / path validation must fail before trogon-atlas attempts a gRPC
// connect. Otherwise an unreachable --endpoint masks the real authoring
// error (missing resolve flags, missing manifest file) with a connection
// failure. These tests deliberately point at a black-hole endpoint.

use std::process::Command;

fn trogon_atlas(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .arg("--endpoint")
        .arg("http://127.0.0.1:1")
        .arg("--rpc-timeout-secs")
        .arg("1")
        .args(args)
        .output()
        .expect("trogon-atlas must spawn")
}

#[test]
fn resolve_missing_strategy_flags_errors_without_connecting() {
    let out = trogon_atlas(&[
        "branch",
        "resolve",
        "any-branch",
        "--kind",
        "event",
        "--namespace",
        "ns",
        "--slug",
        "a",
        "--expect-state",
        "base=1,ours=2,theirs=3",
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "resolve without strategy flags must fail: {stderr}"
    );
    assert!(
        stderr.contains("specify exactly one of --take-theirs, --keep-ours or --take-merged"),
        "local flag validation must win over connect errors; got: {stderr}"
    );
    assert!(
        !stderr.contains("connecting to"),
        "must not attempt gRPC connect before local flag validation; got: {stderr}"
    );
}

#[test]
fn resolve_zero_version_errors_without_connecting() {
    let out = trogon_atlas(&[
        "branch",
        "resolve",
        "any-branch",
        "--kind",
        "event",
        "--namespace",
        "ns",
        "--slug",
        "a",
        "--expect-state",
        "base=1,ours=2,theirs=3",
        "--version",
        "0",
        "--take-theirs",
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "resolve with --version 0 must fail: {stderr}"
    );
    assert!(
        stderr.contains("version") && (stderr.contains("positive") || stderr.contains('0')),
        "local version validation must reject 0; got: {stderr}"
    );
    assert!(
        !stderr.contains("connecting to"),
        "must not attempt gRPC connect before local version validation; got: {stderr}"
    );
}

#[test]
fn branch_create_invalid_name_errors_without_connecting() {
    let out = trogon_atlas(&["branch", "create", "bad name with spaces"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "invalid branch name must fail: {stderr}"
    );
    assert!(
        stderr.contains("invalid") || stderr.contains("branch name"),
        "local branch-name validation must win over connect errors; got: {stderr}"
    );
    assert!(
        !stderr.contains("connecting to"),
        "must not attempt gRPC connect before local branch-name validation; got: {stderr}"
    );
}

#[test]
fn branch_create_reserved_meta_errors_without_connecting() {
    let out = trogon_atlas(&["branch", "create", "meta"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "reserved branch name must fail: {stderr}"
    );
    assert!(
        stderr.contains("reserved"),
        "local reserved-name validation must win over connect errors; got: {stderr}"
    );
    assert!(
        !stderr.contains("connecting to"),
        "must not attempt gRPC connect before local reserved-name validation; got: {stderr}"
    );
}

#[test]
fn export_empty_namespace_errors_without_connecting() {
    let out = trogon_atlas(&[
        "export",
        "--namespace",
        "",
        "-o",
        "/tmp/trogon-atlas-export-empty-ns",
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "empty export namespace must fail: {stderr}"
    );
    assert!(
        stderr.contains("namespace") && stderr.contains("empty"),
        "local namespace validation must win over connect errors; got: {stderr}"
    );
    assert!(
        !stderr.contains("connecting to"),
        "must not attempt gRPC connect before local namespace validation; got: {stderr}"
    );
}

#[test]
fn apply_missing_manifest_errors_without_connecting() {
    let missing = std::env::temp_dir().join(format!(
        "trogon-atlas-missing-manifest-{}.yaml",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    // Do not create the file: apply must report the read error locally.
    let path = missing.to_str().unwrap();
    let out = trogon_atlas(&["apply", "-f", path]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "apply of a missing file must fail: {stderr}"
    );
    assert!(
        stderr.contains("reading")
            || stderr.contains("No such file")
            || stderr.contains("not found"),
        "missing-manifest error must surface; got: {stderr}"
    );
    assert!(
        !stderr.contains("connecting to"),
        "must not attempt gRPC connect before local path validation; got: {stderr}"
    );
}
