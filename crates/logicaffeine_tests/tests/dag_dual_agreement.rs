//! logos-dual: on every Arena case, both conversion engines answer every definitional-equality
//! question identically, and the verdict is the one the Arena expects.
//!
//! With `dual_conv` set, each `is_def_eq` is decided by the substitution engine and by the
//! closure (NbE) engine; a disagreement is reported as a decline. So a pass here means the two
//! independently written engines agreed on every question the case asked. Compiled only with the
//! `nbe` feature.
#![cfg(feature = "nbe")]

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
    let verdict = std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            let input = std::fs::File::open(file).expect("vendored Arena case");
            check_export_with(BufReader::new(input), |k| k.dual_conv = true).0
        })
        .unwrap()
        .join()
        .expect("checker thread panicked");
    match (expect, verdict) {
        (Expect::Accept, Verdict::Accept) | (Expect::Reject, Verdict::Reject(_)) => {}
        (Expect::Either, Verdict::Accept | Verdict::Reject(_)) => {}
        (_, Verdict::Decline(e)) if e.message.contains("disagree") => {
            panic!("{path}: the conversion engines disagree: {e}")
        }
        (Expect::Accept, Verdict::Reject(e)) => panic!("{path}: a valid proof was rejected: {e}"),
        (Expect::Reject, Verdict::Accept) => panic!("{path}: an invalid proof was ACCEPTED"),
        (_, Verdict::Decline(e)) => panic!("{path}: declined: {e}"),
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

/// The heavy inputs are checked by `arena_corpus` and the scoreboard; the dual run of them is a
/// separate, much longer measurement.
macro_rules! arena_heavy_case {
    ($($t:tt)*) => {};
}

include!("arena_corpus/cases.rs");
