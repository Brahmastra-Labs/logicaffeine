//! Every Lean Kernel Arena case, checked by the value engine (`Engine::Sem`), against the verdict
//! the Arena expects — the same list and the same expectations as `arena_corpus`, which holds
//! the substitution engine to them.

mod common;

use logicaffeine_kernel::dag::tc::Engine;
use logicaffeine_kernel::dag::{check_export_with, Verdict};
use std::io::BufReader;
use std::path::PathBuf;

enum Expect {
    Accept,
    Reject,
    Either,
}

fn run(path: &'static str, expect: Expect) {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/arena_corpus")
        .join(path);
    run_file(file, path, expect)
}

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
            let input = std::fs::File::open(file).expect("Arena case");
            check_export_with(BufReader::with_capacity(1 << 20, input), |k| {
                k.engine = Engine::Sem
            })
            .0
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
