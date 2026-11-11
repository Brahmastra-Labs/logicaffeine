//! The arena entry checks a large export across cores by building the whole environment first, then
//! re-checking the leaf declarations (theorems, opaques, axioms) over that frozen environment — out
//! of order, across forked children when THREADS > 1. The result must be a sequential replay: a leaf
//! admitted unchecked and re-checked later gives the identical verdict to checking every declaration
//! in order. These tests pin that equivalence in process (one worker, no fork); the serial path
//! (THREADS = 1) stays exactly `check_export`.

use logicaffeine_kernel::dag::check::Decl;
use logicaffeine_kernel::dag::export::{
    check_export, check_export_build_only, check_export_deferred, Observe, Reader, Verdict,
};
use logicaffeine_kernel::dag::tc::Engine;
use logicaffeine_kernel::dag::{FailKind, Kernel};
use std::io::BufReader;
use std::path::PathBuf;

fn corpus(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/arena_corpus")
        .join(rel)
}

fn open(rel: &str) -> BufReader<std::fs::File> {
    BufReader::new(std::fs::File::open(corpus(rel)).unwrap())
}

fn big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

fn tag(v: &Verdict) -> &'static str {
    match v {
        Verdict::Accept => "accept",
        Verdict::Reject(_) => "reject",
        Verdict::Decline(_) => "decline",
    }
}

/// A good export accepts whether its leaves are checked in order or deferred and re-checked over the
/// finished environment, and the two runs see the same declarations.
#[test]
fn deferring_leaves_matches_the_serial_accept() {
    big_stack(|| {
        let (serial, s1) = check_export(open("good/init-prelude.ndjson"));
        let (deferred, s2) = check_export_deferred(open("good/init-prelude.ndjson"));
        assert_eq!(tag(&serial), "accept", "the serial run should accept init-prelude");
        assert_eq!(tag(&serial), tag(&deferred), "deferred verdict must equal serial");
        assert_eq!(
            s1.declarations, s2.declarations,
            "both runs see the same number of declarations"
        );
    });
}

/// A soundness bug rejects under both paths: deferring a leaf's check never turns a reject into an
/// accept.
#[test]
fn deferring_leaves_never_hides_a_reject() {
    big_stack(|| {
        let (serial, _) = check_export(open("bad/bugs/closure-level-subst.ndjson"));
        let (deferred, _) = check_export_deferred(open("bad/bugs/closure-level-subst.ndjson"));
        assert_eq!(tag(&serial), "reject", "the fixture is an invalid proof");
        assert_eq!(tag(&serial), tag(&deferred), "deferred must also reject");
    });
}

/// A deterministic Fisher–Yates scramble (a fixed-seed LCG, so the test is reproducible): the order
/// the fork re-checks leaves in is not the file order, so the verdict must not depend on it.
fn scrambled(mut v: Vec<Decl>) -> Vec<Decl> {
    let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
    for i in (1..v.len()).rev() {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let j = ((s >> 33) as usize) % (i + 1);
        v.swap(i, j);
    }
    v
}

fn reversed(mut v: Vec<Decl>) -> Vec<Decl> {
    v.reverse();
    v
}

/// Build the whole environment, then re-check its deferred leaves in `reorder`'s order over that
/// frozen environment — exactly what a forked worker does out of order — and report the verdict.
fn recheck_in_order(rel: &str, engine: Engine, reorder: fn(Vec<Decl>) -> Vec<Decl>) -> &'static str {
    let mut k = Kernel::new();
    k.engine = engine;
    let (built, deferred, _) = check_export_build_only(&mut k, open(rel), None, Observe::default());
    let outcome = match built {
        Ok(()) => {
            let mut res = Ok(());
            for d in &reorder(deferred) {
                if let Err(e) = k.recheck_decl(d) {
                    res = Err(e);
                    break;
                }
            }
            res
        }
        Err(e) => Err(e),
    };
    match outcome {
        Ok(()) => "accept",
        Err(e) if e.kind == FailKind::Reject => "reject",
        Err(_) => "decline",
    }
}

/// The core parallel-soundness invariant: a good export accepts no matter what order its leaves are
/// re-checked in — file order, reversed, or scrambled — on both checking engines. If any order
/// changed the verdict, a forked worker could accept what the serial check rejects.
#[test]
fn reordering_the_leaves_never_changes_an_accept() {
    big_stack(|| {
        for engine in [Engine::Classic, Engine::Sem] {
            let file = recheck_in_order("good/init-prelude.ndjson", engine, |v| v);
            let rev = recheck_in_order("good/init-prelude.ndjson", engine, reversed);
            let scr = recheck_in_order("good/init-prelude.ndjson", engine, scrambled);
            assert_eq!(file, "accept", "{engine:?}: file order should accept");
            assert_eq!(rev, "accept", "{engine:?}: reversed order must also accept");
            assert_eq!(scr, "accept", "{engine:?}: scrambled order must also accept");
        }
    });
}

/// The same invariant on the reject side: an invalid proof is rejected no matter the order, so no
/// re-checking order can hide a soundness bug.
#[test]
fn reordering_the_leaves_never_hides_a_reject() {
    big_stack(|| {
        for engine in [Engine::Classic, Engine::Sem] {
            let rev = recheck_in_order("bad/bugs/closure-level-subst.ndjson", engine, reversed);
            let scr = recheck_in_order("bad/bugs/closure-level-subst.ndjson", engine, scrambled);
            assert_eq!(rev, "reject", "{engine:?}: reversed order must reject the invalid proof");
            assert_eq!(scr, "reject", "{engine:?}: scrambled order must reject the invalid proof");
        }
    });
}

/// Build the environment with definitions *also* deferred (`defer_defns`), then re-check every
/// deferred declaration — theorems and non-primitive definitions alike — in `reorder`'s order over
/// the frozen environment. A deferred definition's body is δ-unfolded by other checks meanwhile, so
/// this exercises the widened deferral the serial build never does.
fn recheck_with_defn_deferral(rel: &str, engine: Engine, reorder: fn(Vec<Decl>) -> Vec<Decl>) -> &'static str {
    let mut k = Kernel::new();
    k.engine = engine;
    let (built, deferred) = {
        let mut r = Reader::new(&mut k);
        r.defer_leaves = true;
        r.defer_defns = true;
        let built = r.run(open(rel));
        let deferred = std::mem::take(&mut r.deferred);
        (built, deferred)
    };
    let outcome = match built {
        Ok(()) => {
            let mut res = Ok(());
            for d in &reorder(deferred) {
                if let Err(e) = k.recheck_decl(d) {
                    res = Err(e);
                    break;
                }
            }
            res
        }
        Err(e) => Err(e),
    };
    match outcome {
        Ok(()) => "accept",
        Err(e) if e.kind == FailKind::Reject => "reject",
        Err(_) => "decline",
    }
}

/// Deferring non-primitive definitions too is sound: a definition admitted unchecked records the
/// same environment entry a checked one would (body, height, type), so a later unfold sees identical
/// state and the verdict is a sequential replay — accepting the good export in any order, on either
/// engine, even though many more declarations are now deferred.
#[test]
fn deferring_definitions_too_matches_the_serial_accept() {
    big_stack(|| {
        for engine in [Engine::Classic, Engine::Sem] {
            let file = recheck_with_defn_deferral("good/init-prelude.ndjson", engine, |v| v);
            let rev = recheck_with_defn_deferral("good/init-prelude.ndjson", engine, reversed);
            let scr = recheck_with_defn_deferral("good/init-prelude.ndjson", engine, scrambled);
            assert_eq!(file, "accept", "{engine:?}: definitions deferred, file order should accept");
            assert_eq!(rev, "accept", "{engine:?}: definitions deferred, reversed must accept");
            assert_eq!(scr, "accept", "{engine:?}: definitions deferred, scrambled must accept");
        }
    });
}

/// Deferring definitions never hides a reject: an invalid declaration is caught by its own re-check
/// whatever order it runs in, so widening the deferral cannot turn a reject into an accept.
#[test]
fn deferring_definitions_too_never_hides_a_reject() {
    big_stack(|| {
        for engine in [Engine::Classic, Engine::Sem] {
            let rev = recheck_with_defn_deferral("bad/bugs/closure-level-subst.ndjson", engine, reversed);
            let scr = recheck_with_defn_deferral("bad/bugs/closure-level-subst.ndjson", engine, scrambled);
            assert_eq!(rev, "reject", "{engine:?}: definitions deferred, reversed must reject");
            assert_eq!(scr, "reject", "{engine:?}: definitions deferred, scrambled must reject");
        }
    });
}
