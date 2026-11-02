//! ★ OPTION × TIER LOCK — one Option, one shape, on every tier. ★
//!
//! `Option` is the type LOGOS models everywhere except where it counts. The type checker has it
//! (`InferType::Option`, `LogosType::Option`, arity-1 in the registry), the proof/extraction tier
//! encodes it as an honest two-constructor inductive (`compile.rs`'s `COptionSome`/`COptionNone`),
//! and AOT-Rust emits real `std::option::Option`. The BYTECODE does not: `vm/compiler.rs` lowers
//! `Expr::OptionSome { value }` to `compile_expr_into(value, dst)` — the `Some` is discarded — and
//! `none` to the pre-existing `Nothing` sentinel.
//!
//! ★ SO ONE TYPE HAS THREE REPRESENTATIONS, and this file is where they are held to each other.
//!
//!   1. `some x`                    → transparent: the payload itself
//!   2. `a new Some with value x`   → a RECORD named "Some" (`Option` is registered as a
//!                                     `TypeDef::Generic`, not an enum, so `find_variant("Some")`
//!                                     misses and the parser falls through to `Expr::New`)
//!   3. AOT-Rust                    → real `Some(x)` / `None`
//!
//! `stdlib_result_interp.rs` already asserts the invariant these must satisfy — *"a literal-built
//! Option must match `When Some (v):` / `When None:` arms identically to `a new Some with value
//! x`. One Option, one shape."* — but only on the interpreter, which is precisely why the two
//! forms were free to diverge from each other on the VM.
//!
//! Every test here is written against the CONTRACT, not against today's behaviour. Where a tier
//! does not meet it yet, that is a gap to close, never an assertion to soften.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome, vm_outcome, vm_outcome_concurrent};

/// Run a program on the tree-walker (the reference tier) and the VM, and return both verdicts as
/// `Ok(output)` / `Err(message)` so a divergence in EITHER direction is visible.
fn both_tiers(src: &str) -> (Result<String, String>, Result<String, String>) {
    both_tiers_with(src, false)
}

/// [`both_tiers`] for a program that uses channels — the VM needs its scheduler driver for those,
/// and the plain runner answers "concurrency op requires the scheduler driver" rather than running.
fn both_tiers_concurrent(src: &str) -> (Result<String, String>, Result<String, String>) {
    both_tiers_with(src, true)
}

fn both_tiers_with(
    src: &str,
    concurrent: bool,
) -> (Result<String, String>, Result<String, String>) {
    let tw = tw_outcome(src);
    let vm = if concurrent {
        vm_outcome_concurrent(src)
    } else {
        vm_outcome(src)
    };
    let as_result = |out: String, err: Option<String>| match err {
        Some(e) => Err(e),
        None => Ok(out.trim().to_string()),
    };
    (
        as_result(tw.output, tw.error),
        as_result(vm.output, vm.error),
    )
}

/// ★ THE ONE-SHAPE LOCK. The two ways to build a present Option must be the same value.
///
/// This is the invariant `stdlib_result_interp.rs:66` states in prose and tests on one tier. Held
/// across two tiers it fails, because the forms take different paths: `some 7` is transparent, and
/// `a new Some with value 7` builds a struct named "Some". The tree-walker's `Some` arm then looks
/// for a field to bind and finds none; the VM's `SOME_BINDING_MARKER` binds the whole struct.
///
/// Two tiers, two different wrong answers, for a form the codebase claims is equivalent.
#[test]
fn the_boxed_and_literal_option_forms_are_the_same_value_on_every_tier() {
    const LITERAL: &str = "## Main\nLet v be some 7.\nInspect v:\n    When Some (n):\n        Show n.\n    When None:\n        Show \"none\".\n";
    const BOXED: &str = "## Main\nLet v be a new Some with value 7.\nInspect v:\n    When Some (n):\n        Show n.\n    When None:\n        Show \"none\".\n";

    let (lit_tw, lit_vm) = both_tiers(LITERAL);
    let (box_tw, box_vm) = both_tiers(BOXED);

    assert_eq!(
        lit_tw, lit_vm,
        "the LITERAL form must already agree across tiers (this is the baseline)"
    );
    assert_eq!(
        box_tw, box_vm,
        "`a new Some with value 7` answers differently on the tree-walker than on the VM.\n  \
         tree-walker: {box_tw:?}\n  VM:          {box_vm:?}\n\
         The two tiers disagree about what the boxed form even IS — one looks for a field to bind, \
         the other binds the whole struct. Converge the two SURFACE forms at the AST so no tier \
         has to guess."
    );
    assert_eq!(
        lit_tw, box_tw,
        "`some 7` and `a new Some with value 7` must be the same value — `stdlib_result_interp` \
         states this invariant outright (\"One Option, one shape\") and tests it on one tier.\n  \
         some 7:                    {lit_tw:?}\n  a new Some with value 7:   {box_tw:?}"
    );
}

/// ★ R1 — THE DISPLAY RULE. Changing the representation must not change a single byte of output.
///
/// This test exists BEFORE the representation moves, and that ordering is the point: it is the
/// invariant that keeps the change invisible to every program anyone has already written. An
/// `Inductive` renders as `Some(7)` today (`interpreter.rs`'s `to_display_string`), so a naive
/// "make Option a real inductive" would silently rewrite the output of every program that shows
/// one. An Option renders its PAYLOAD when present and the word `nothing` when absent — which is
/// exactly what all five tiers print right now, and what `semantic_probes_lock`'s
/// `option_some_is_transparent` probe pins across tw / VM / opt-VM / CLI / AOT-Rust.
///
/// Tristen's ruling, recorded: printing does not change.
#[test]
fn showing_an_option_prints_its_payload_not_its_wrapper() {
    for (label, src, want) in [
        ("present int", "## Main\nShow some 42.\n", "42"),
        ("present text", "## Main\nShow some \"hi\".\n", "hi"),
        ("absent", "## Main\nShow none.\n", "nothing"),
        (
            "present through a binding",
            "## Main\nLet v be some 42.\nShow v.\n",
            "42",
        ),
    ] {
        let (tw, vm) = both_tiers(src);
        assert_eq!(
            tw,
            Ok(want.to_string()),
            "`{label}` must print {want:?} on the reference tier. If this went to `Some(42)`, the \
             representation moved without its display rule and every program that shows an \
             Option just changed its output."
        );
        assert_eq!(
            tw, vm,
            "`{label}`: the VM must print exactly what the reference tier prints"
        );
    }
}

/// ★ R2 — ABSENCE IS ONE THING. `nothing` and `none` must stay interchangeable.
///
/// `nothing` and `none` are DIFFERENT AST nodes (`Literal::Nothing` vs `Expr::OptionNone`) that
/// converge on the dynamic tiers. Whatever `none` becomes, comparing it against `nothing` has to
/// keep answering true, or every `x is equal to nothing` in the wild changes meaning — including
/// the ones the wasm tier compiles to a handle test today.
#[test]
fn an_absent_option_still_equals_nothing() {
    for (label, src, want) in [
        ("none is nothing", "## Main\nLet v be none.\nIf v is equal to nothing:\n    Show \"absent\".\nOtherwise:\n    Show \"present\".\n", "absent"),
        ("is none", "## Main\nLet v be none.\nIf v is none:\n    Show \"absent\".\nOtherwise:\n    Show \"present\".\n", "absent"),
        ("a present option is not none", "## Main\nLet v be some 5.\nIf v is none:\n    Show \"absent\".\nOtherwise:\n    Show \"present\".\n", "present"),
    ] {
        let (tw, vm) = both_tiers(src);
        assert_eq!(tw, Ok(want.to_string()), "`{label}` on the reference tier");
        assert_eq!(tw, vm, "`{label}`: and the VM must agree");
    }
}

/// ★ THE SENTINEL LOCK — an Option must survive being an Option of an Option.
///
/// `some none` is the smallest program that cannot be represented by a transparent encoding: the
/// outer `Some` and the inner `None` collapse onto the same `Nothing`, so the outer Option reads as
/// absent. This fails on the REFERENCE TIER, which is the proof that the defect is the encoding
/// rather than any one backend — `some x` is erased in `interpreter.rs` exactly as it is in the
/// bytecode compiler.
#[test]
fn an_option_of_an_option_keeps_both_levels() {
    const SRC: &str = "## Main\nLet v be some none.\nInspect v:\n    When Some (inner):\n        Show \"outer-present\".\n    When None:\n        Show \"outer-absent\".\n";
    let (tw, vm) = both_tiers(SRC);
    assert_eq!(
        tw,
        Ok("outer-present".to_string()),
        "`some none` is a PRESENT Option whose payload happens to be absent. Reading it as absent \
         collapses two levels into one — the defining failure of a transparent encoding, and it \
         happens on the reference tier itself."
    );
    assert_eq!(tw, vm, "and the VM must agree with the reference tier");
}

/// ★ THE DATA-INTEGRITY LOCK — this one already ships in the standard library.
///
/// `postgres.lg` exposes `pgIntAtOpt`, `pgBoolAtOpt` and `pgRealAtOpt`, all returning
/// `Result of Option of <scalar> and Text`, to distinguish a SQL `NULL` from a value. Under the
/// transparent encoding a `NULL` and a `0` are the same bits, so the distinction those functions
/// exist to make cannot be made. This models that shape without needing a database.
#[test]
fn a_present_zero_is_not_the_same_as_an_absent_value() {
    const SRC: &str = "## To readCell (isNull: Bool) -> Option of Int:\n    If isNull:\n        Return none.\n    Return some 0.\n\n## Main\nInspect readCell(false):\n    When Some (n):\n        Show \"present\".\n    When None:\n        Show \"absent\".\nInspect readCell(true):\n    When Some (n):\n        Show \"present\".\n    When None:\n        Show \"absent\".\n";
    let (tw, vm) = both_tiers(SRC);
    assert_eq!(
        tw,
        Ok("present\nabsent".to_string()),
        "a stored `0` and a stored NULL must be distinguishable. `pgIntAtOpt` and its siblings \
         exist for exactly this, so a column holding 0 currently reads as NULL."
    );
    assert_eq!(tw, vm, "and both tiers must say so");
}

/// ★ THE BUILTINS THAT ANSWER AN OPTION MUST ANSWER A REAL ONE.
///
/// `storageGet` used to hand back a bare `Text` when present and `Nothing` when absent — the
/// transparent encoding — and it worked only while `Inspect`'s catch-all answered a `Some` arm
/// with "anything that is not the sentinel". Removing that catch-all made a PRESENT stored value
/// take the `None` arm: a WRONG ANSWER, not a refusal, which is the worst shape a gap can take.
/// `storage_lock` caught it because it had a test.
///
/// `Try to receive` and an over-`Pop` have exactly the same shape and NO such test, which is the
/// only reason they are not equally loud. This is that test.
#[test]
fn every_builtin_that_answers_an_option_answers_a_real_one() {
    // A received value is PRESENT. Under the transparent encoding this took the `None` arm.
    let recv = "## Main\nLet ch be a Pipe of Int.\nSend 7 into ch.\nTry to receive x from ch.\nInspect x:\n    When Some (v):\n        Show \"got \" + v.\n    When None:\n        Show \"empty\".\n";
    let (tw, vm) = both_tiers_concurrent(recv);
    assert_eq!(
        tw,
        Ok("got 7".to_string()),
        "a `Try to receive` that RECEIVED must take the `Some` arm. Reading it as absent is a \
         wrong answer rather than a refusal — the shape `storageGet` was caught in."
    );
    assert_eq!(tw, vm, "and the VM must agree with the reference tier");

    // An empty channel is ABSENT.
    let empty = "## Main\nLet ch be a Pipe of Int.\nTry to receive x from ch.\nInspect x:\n    When Some (v):\n        Show \"got \" + v.\n    When None:\n        Show \"empty\".\n";
    let (tw2, vm2) = both_tiers_concurrent(empty);
    assert_eq!(
        tw2,
        Ok("empty".to_string()),
        "an empty try-receive is absent"
    );
    assert_eq!(tw2, vm2, "and the VM must agree");
}
