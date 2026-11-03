#![allow(clippy::unwrap_used, clippy::expect_used)]

// clap prints the live value of an env-backed argument in `--help` as
// `[env: NAME=value]` unless the argument sets `hide_env_values = true`. These
// tests run the compiled binary with obvious sentinel values in every
// secret-bearing environment variable and require that the help text names the
// variable without ever echoing the value.

use std::process::Command;

const SENTINEL: &str = "ATLAS-HELP-LEAK-SENTINEL-NOT-A-REAL-VALUE";

/// Environment variables of `trogon-atlas` that carry a secret or a
/// connection string that may embed credentials.
const SECRET_ENV: &[&str] = &["TROGON_ATLAS_AUTH_TOKEN", "TROGON_ATLAS_ENDPOINT"];

fn help(args: &[&str]) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"));
    command.args(args).env_clear();
    for name in SECRET_ENV {
        command.env(name, format!("{SENTINEL}-{name}"));
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn assert_hidden(args: &[&str]) {
    let text = help(args);
    for name in SECRET_ENV {
        assert!(
            text.contains(name),
            "{args:?} help should still name {name}:\n{text}"
        );
    }
    assert!(
        !text.contains(SENTINEL),
        "{args:?} help leaked a secret environment value:\n{text}"
    );
}

#[test]
fn long_help_hides_secret_env_values() {
    assert_hidden(&["--help"]);
}

#[test]
fn short_help_hides_secret_env_values() {
    assert_hidden(&["-h"]);
}

#[test]
fn subcommand_help_hides_secret_env_values() {
    assert_hidden(&["validate", "--help"]);
}
