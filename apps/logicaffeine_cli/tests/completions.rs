//! `largo completions <shell>` — shell completion generation.

mod common;

use common::*;

/// Every supported shell generates non-empty completions mentioning `largo`.
#[test]
fn all_shells_generate_completions() {
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let out = largo()
            .args(["completions", shell])
            .output()
            .expect("largo should spawn");
        assert_eq!(
            out.status.code(),
            Some(0),
            "completions {shell} should succeed: {}",
            stderr(&out)
        );
        let script = stdout(&out);
        assert!(!script.trim().is_empty(), "{shell} completions must be non-empty");
        assert!(
            script.contains("largo"),
            "{shell} completions must mention largo"
        );
    }
}

/// The generated bash script knows the actual subcommands.
#[test]
fn bash_completions_mention_subcommands() {
    let out = largo()
        .args(["completions", "bash"])
        .output()
        .expect("largo should spawn");
    let script = stdout(&out);
    for cmd in ["build", "run", "check", "publish"] {
        assert!(script.contains(cmd), "bash completions must mention `{cmd}`");
    }
}

/// An unsupported shell is a usage error.
#[test]
fn bogus_shell_is_a_usage_error() {
    let out = largo()
        .args(["completions", "tcsh"])
        .output()
        .expect("largo should spawn");
    assert_eq!(out.status.code(), Some(2));
}
