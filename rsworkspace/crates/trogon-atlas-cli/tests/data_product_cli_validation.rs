#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::{Command, Output};

use serde_json::Value;

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args([
            "--endpoint",
            "http://127.0.0.1:1",
            "--rpc-timeout-secs",
            "1",
        ])
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn describe_help_keeps_its_description() {
    let out = cli(&["describe", "--help"]);
    assert!(out.status.success());
    let help = String::from_utf8(out.stdout).unwrap();
    assert!(help.contains("Print one entity as readable text"), "{help}");
    assert!(
        help.contains("the references it holds and receives"),
        "{help}"
    );
    assert!(!help.contains("checked model evidence"), "{help}");
}

#[test]
fn inspect_help_describes_only_backend_inspection() {
    let out = cli(&["inspect-data-product", "--help"]);
    assert!(out.status.success());
    let help = String::from_utf8(out.stdout).unwrap();
    assert!(
        help.contains("Inspect authoritative DataProduct selections and checked model evidence"),
        "{help}"
    );
    assert!(
        !help.contains("Print one entity as readable text"),
        "{help}"
    );
}

fn invalid_inspection(address: &str, expected: &str) {
    let out = cli(&["--format", "json", "inspect-data-product", address]);
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(5), "{json}");
    assert_eq!(json["ok"], false);
    assert_eq!(json["failure"]["category"], "validation", "{json}");
    assert_eq!(json["failure"]["next_action"], "fix_input", "{json}");
    assert!(
        json["failure"]["message"]
            .as_str()
            .unwrap()
            .contains(expected),
        "{json}"
    );
    assert!(!String::from_utf8_lossy(&out.stderr).contains("connecting"));
}

#[test]
fn inspection_wrong_kind_is_a_local_input_error() {
    invalid_inspection("event/shop/fact@1", "expected a data_product address");
}

#[test]
fn inspection_missing_exact_version_is_a_local_input_error() {
    invalid_inspection(
        "data_product/shop/offering",
        "inspection requires an exact version",
    );
}
