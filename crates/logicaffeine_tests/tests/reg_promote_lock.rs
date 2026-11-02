//! Aggregate-element register promotion (RegPromote) lock.
//!
//! RegPromote forwards an invariant-indexed read-modify-write array element
//! through a scalar across a loop (load once, use in a register, write back
//! once) — store-to-load forwarding LICM cannot do. These locks pin:
//!
//!   1. FIRING — it fires on a reduction into one fixed cell of a runtime-size
//!      array, and gates off under `## No regpromote`.
//!   2. CORRECTNESS — the OPTIMIZED run (`interpret_program`, RegPromote
//!      applied) equals the RAW run (`vm_run_source`) and the RegPromote-off
//!      optimized run, over a spread of loop counts (including the zero-trip
//!      guard case). A miscompiled forward changes the optimized run and is
//!      caught.
//!   3. NEGATIVES — a VARYING index (not loop-invariant) and an ALIASED access
//!      (the array touched at a second index) must NOT fire it.

use logicaffeine_compile::compile::{
    compile_to_rust, interpret_program, optimizations_fired_run, vm_run_source,
};
use logicaffeine_compile::optimization::decorate_source;

/// Reduction into `acc[1]` of a size-`n` array — the register-promotable shape.
/// `acc` is built by a loop (not a fixed literal), so array scalarization cannot
/// claim it; every access in the hot loop is `item 1 of acc`.
fn fixture(n: i64) -> String {
    format!(
        "## Main\nLet n be {n}.\n\
         Let mutable acc be a new Seq of Int.\nLet mutable p be 0.\n\
         While p is less than n:\n    Push 0 to acc.\n    Set p to p + 1.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n    Set item 1 of acc to item 1 of acc + i.\n    Set i to i + 1.\n\
         Show item 1 of acc.\n"
    )
}

// `interpret_program` runs the OPTIMIZING VM (RegPromote applied); `vm_run_source`
// runs the RAW VM. Their agreement is the opt-vs-raw execution differential.
fn opt(src: &str) -> String {
    interpret_program(src)
        .unwrap_or_else(|e| panic!("optimized run failed: {e:?}"))
        .trim_end()
        .to_string()
}
fn raw(src: &str) -> String {
    vm_run_source(src)
        .unwrap_or_else(|e| panic!("raw run failed: {e}"))
        .trim_end()
        .to_string()
}

#[test]
fn regpromote_fires_and_gates() {
    let src = fixture(50);
    let run = optimizations_fired_run(&src);
    assert!(
        run.contains(&"regpromote"),
        "run-path must fire regpromote on the reduction; got {run:?}"
    );
    let off = optimizations_fired_run(&decorate_source(&src, &["regpromote"]));
    assert!(
        !off.contains(&"regpromote"),
        "`## No regpromote` must remove it; got {off:?}"
    );
}

/// Correctness core: the forwarded scalar equals the in-memory element, so the
/// reduction `sum(0..n-1)` from the optimized run equals the raw run and the
/// RegPromote-off optimized run.
#[test]
fn regpromote_preserves_reduction() {
    for n in [1, 2, 5, 20, 100] {
        let src = fixture(n);
        let off = decorate_source(&src, &["regpromote"]);
        let expected = (n * (n - 1) / 2).to_string();
        assert_eq!(opt(&src), expected, "n={n}: optimized value");
        assert_eq!(raw(&src), expected, "n={n}: raw value");
        assert_eq!(
            opt(&src),
            raw(&src),
            "n={n}: RegPromote diverged from raw (miscompile!)"
        );
        assert_eq!(opt(&off), expected, "n={n}: RegPromote-off optimized value");
    }
}

/// The zero-trip guard: a loop that never runs must leave the element untouched
/// (the promoted load/write-back are skipped), byte-identical to the source.
#[test]
fn regpromote_zero_trip_guard() {
    let src = "## Main\nLet mutable acc be a new Seq of Int.\nPush 7 to acc.\nLet mutable i be 5.\nWhile i is less than 0:\n    Set item 1 of acc to item 1 of acc + i.\n    Set i to i + 1.\nShow item 1 of acc.\n";
    assert_eq!(opt(src), "7");
    assert_eq!(raw(src), "7");
    assert_eq!(opt(&decorate_source(src, &["regpromote"])), "7");
}

/// A VARYING index (`item (i+1) of acc`, not loop-invariant) must NOT fire
/// RegPromote — forwarding a different cell each iteration would be unsound.
#[test]
fn regpromote_noop_on_varying_index() {
    let src = "## Main\nLet n be 30.\n\
         Let mutable acc be a new Seq of Int.\nLet mutable p be 0.\n\
         While p is less than n:\n    Push 0 to acc.\n    Set p to p + 1.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n    Set item (i + 1) of acc to item (i + 1) of acc + i.\n    Set i to i + 1.\n\
         Show item 1 of acc.\n";
    let fired = optimizations_fired_run(src);
    assert!(
        !fired.contains(&"regpromote"),
        "must not fire on a varying index; got {fired:?}"
    );
    // And it stays correct: acc[1] = 0 (only the k=0 write touches index 1).
    assert_eq!(opt(src), raw(src));
}

/// An ALIASED access — the array also touched at a second index in the loop —
/// must NOT fire RegPromote (the other write could hit the promoted cell).
#[test]
fn regpromote_noop_on_aliased_access() {
    let src = "## Main\nLet n be 30.\n\
         Let mutable acc be a new Seq of Int.\nLet mutable p be 0.\n\
         While p is less than n:\n    Push 0 to acc.\n    Set p to p + 1.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n    Set item 1 of acc to item 1 of acc + i.\n    Set item 2 of acc to i.\n    Set i to i + 1.\n\
         Show item 1 of acc.\n";
    let fired = optimizations_fired_run(src);
    assert!(
        !fired.contains(&"regpromote"),
        "must not fire when the array is aliased at another index; got {fired:?}"
    );
    assert_eq!(opt(src), raw(src));
}

/// `## Require regpromote` compiles on the reduction fixture and errors with no site.
#[test]
fn regpromote_require_gate() {
    let required = fixture(50).replace("## Main", "## Require regpromote\n## Main");
    assert!(
        compile_to_rust(&required).is_ok(),
        "`## Require regpromote` must compile on the fixture"
    );
    let no_site = "## Require regpromote\n## Main\nShow 42.\n";
    assert!(
        compile_to_rust(no_site).is_err(),
        "`## Require regpromote` must error with no site"
    );
}
