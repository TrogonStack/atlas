#![allow(clippy::unwrap_used)]

use std::process::Command;

#[test]
fn missing_artifact_file_is_reported_before_connecting() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args([
            "--endpoint",
            "http://127.0.0.1:1",
            "--rpc-timeout-secs",
            "1",
            "artifacts",
            "upload",
            "--namespace",
            "sample",
            "--file",
        ])
        .arg(directory.path().join("missing.png"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("artifact file unavailable"));
    assert!(!error.contains("connecting to"));
}
