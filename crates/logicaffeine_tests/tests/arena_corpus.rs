//! Every Lean Kernel Arena case, one test each, against the verdict the Arena expects.
//!
//! The corpus is the pinned upstream manifest's test set (`fixtures/arena_corpus`), vendored by
//! `scripts/lean/arena-crown/vendor-corpus.py`. A valid proof must be accepted, an invalid one
//! rejected, and a corner case (`either`) decided one way or the other; a decline is never a pass.

mod common;

use logicaffeine_kernel::dag::{check_export, Verdict};
use std::io::BufReader;
use std::path::PathBuf;

enum Expect {
    Accept,
    Reject,
    Either,
}

fn corpus(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/arena_corpus")
        .join(path)
}

fn run(path: &'static str, expect: Expect) {
    run_file(corpus(path), path, expect)
}

/// A large input kept out of the repository (`reference-kernels/` is gitignored), so a clone does not
/// have it. Its absence goes through the input seam: a counted `LOGOS_TOOL_SKIP: reference-kernels`
/// where the tree is optional, a failure wherever `LOGOS_REQUIRE_TOOLS` names it (the full runner, the
/// Arena runner). A tree that is present but lacks this input fails.
fn run_heavy(path: &'static str, expect: Expect) {
    if !common::require_tool("reference-kernels") {
        return;
    }
    let full = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path);
    assert!(full.exists(), "heavy Arena input {path} is missing; run scripts/lean/arena-crown/vendor-corpus.py on a box with reference-kernels/");
    run_file(full, path, expect)
}

fn run_file(file: PathBuf, path: &'static str, expect: Expect) {
    let verdict = std::thread::Builder::new()
        .stack_size(8 << 30)
        .spawn(move || {
            let file = std::fs::File::open(file).expect("Arena case");
            check_export(BufReader::with_capacity(1 << 20, file)).0
        })
        .unwrap()
        .join()
        .expect("checker thread panicked");
    match (expect, verdict) {
        (Expect::Accept, Verdict::Accept) | (Expect::Reject, Verdict::Reject(_)) => {}
        (Expect::Either, Verdict::Accept | Verdict::Reject(_)) => {}
        (Expect::Accept, Verdict::Reject(e)) => {
            panic!("{path}: a valid proof was REJECTED (incomplete): {e}")
        }
        (Expect::Reject, Verdict::Accept) => {
            panic!("{path}: an invalid proof was ACCEPTED (unsound)")
        }
        (_, Verdict::Decline(e)) => panic!("{path}: declined, which is never a pass: {e}"),
    }
}

macro_rules! arena_case {
    ($(#[$m:meta])* $name:ident, $path:expr, $expect:ident) => {
        #[test]
        $(#[$m])*
        fn $name() {
            run($path, Expect::$expect)
        }
    };
}

macro_rules! arena_heavy_case {
    ($(#[$m:meta])* $name:ident, $path:expr, $expect:ident) => {
        #[test]
        $(#[$m])*
        fn $name() {
            run_heavy($path, Expect::$expect)
        }
    };
}

include!("arena_corpus/cases.rs");
