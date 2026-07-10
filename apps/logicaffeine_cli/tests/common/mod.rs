//! Shared helpers for `largo` end-to-end tests.
//!
//! Every e2e test spawns the real binary (`CARGO_BIN_EXE_largo`) against a
//! scaffolded project in a temp directory, mirroring exactly what a user runs.

use std::path::Path;
use std::process::{Command, Output};

/// A `Command` for the freshly-built `largo` binary.
///
/// `env!("CARGO_BIN_EXE_largo")` bakes the *build-time* target path into the test
/// binary; when the suite runs from a nextest archive (CI), that path doesn't exist
/// in the fresh test-job checkout. nextest re-exports the extracted binary at
/// runtime via `CARGO_BIN_EXE_largo`, so prefer the runtime value and fall back to
/// the compile-time constant for a plain `cargo test`.
pub fn largo() -> Command {
    let exe = std::env::var_os("CARGO_BIN_EXE_largo")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_largo").into());
    Command::new(exe)
}

/// Run `largo` with `args` in `dir`, capturing output.
pub fn largo_in(dir: &Path, args: &[&str]) -> Output {
    largo()
        .args(args)
        .current_dir(dir)
        .output()
        .expect("largo should spawn")
}

/// Scaffold a minimal LOGOS project (Largo.toml + src/main.lg) in `dir`.
pub fn scaffold(dir: &Path, name: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Largo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n"),
    )
    .unwrap();
    std::fs::write(dir.join("src/main.lg"), "# Main\n\n## Main\n\nShow \"Hello, world!\".\n")
        .unwrap();
}

/// Strip ANSI escape sequences from a string.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip a CSI sequence: ESC [ ... final-byte (0x40..=0x7e)
            if chars.peek() == Some(&'[') {
                chars.next();
                for f in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&f) {
                        break;
                    }
                }
                continue;
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Whether a string contains any ANSI escape sequence.
pub fn has_ansi(s: &str) -> bool {
    s.contains('\x1b')
}

/// UTF-8 stdout of an `Output`.
pub fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// UTF-8 stderr of an `Output`.
pub fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}
