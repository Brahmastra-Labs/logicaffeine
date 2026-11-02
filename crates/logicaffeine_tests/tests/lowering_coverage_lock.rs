//! lowering_coverage_lock — the MASTER pit-of-success ratchet for the whole-program fabric lowering.
//!
//! The fabric proves all 115 ISA ops *reachable*, but op-reachability ≠ arbitrary-program-lowerability: the
//! lowering (`program_cfg_fabric`) accepts a subset and historically `panic!`ed on the rest, so a realistic
//! program that did not lower turned NO lock RED — it just aborted. This lock closes that hole.
//!
//! Every construct CLASS is tagged `Supported` or `KnownGap(reason)` and driven through the TYPED entry point
//! `try_program_cfg_fabric`, giving one of four honest outcomes:
//!   • `LowersEqVm`  — lowers AND runs ≡ the VM over every input,
//!   • `TypedErr`    — refused cleanly with a `LoweringError` (the intended shape of a not-yet-lowered class),
//!   • `Panicked`    — an unconverted `panic!`/`unwrap` fired (a bug: convert it to a typed error),
//!   • `WrongResult` — lowered but diverged from the VM (a correctness bug).
//!
//! Two ratchets, both RED on violation:
//!   1. every `Supported` class must be `LowersEqVm` (a regression is RED),
//!   2. every `KnownGap` class must be `TypedErr` — NOT `Panicked`/`WrongResult` (an unconverted panic or a
//!      silent miscompile is RED), and NOT `LowersEqVm` (a gap that now works must be PROMOTED to `Supported`).
//!
//! So: the supported frontier can only grow, gaps fail gracefully and can only shrink, and the 171 lowering
//! panics become one explicit, shrinking set. As each later phase (bounds prover, for-each, builtins) lands, a
//! class moves `KnownGap` → `Supported` here.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(lowering_coverage_lock)'`

use std::panic::{catch_unwind, AssertUnwindSafe};

use logicaffeine_compile::codegen_sva::fabric_rtl::try_program_cfg_fabric;
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    eval_wprog_config, program_calls_fabric, serialize_program,
};
use logicaffeine_compile::compile::vm_outcome;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Status {
    Supported,
    KnownGap(&'static str),
}
use Status::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Outcome {
    LowersEqVm,
    TypedErr,
    Panicked,
    WrongResult,
}
use Outcome::*;

struct Class {
    name: &'static str,
    status: Status,
    src: &'static str,
    args: &'static [&'static [i64]],
}

/// Run the VM on `f(args)`; None on error / non-integer output.
fn vm_f(src: &str, args: &[i64]) -> Option<i64> {
    let al = args
        .iter()
        .map(|a| {
            if *a < 0 {
                format!("(0 - {})", -a)
            } else {
                a.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let prog = format!("{src}## Main\n    Show f({al}).\n");
    let out = vm_outcome(&prog);
    if out.error.is_some() {
        return None;
    }
    out.output.split_whitespace().next()?.parse::<i64>().ok()
}

/// Drive one construct through BOTH whole-program lowering entry points and classify the honest outcome. The
/// fabric has two: `program_cfg_fabric` (single-function CFG, TYPED refusal) and `program_calls_fabric` (the
/// multi-function / iterator / recursion path, which lowers `IterNext` for-each + `Call` recursion; panics on
/// unsupported). A construct is `LowersEqVm` if EITHER lowers it ≡ the VM — the same "try both" the whole
/// benchmark lane uses. The reported failure mode is the CFG path's (the typed one).
fn evaluate(c: &Class) -> Outcome {
    let eq_vm = |run: &dyn Fn(&[i64]) -> i64| -> bool {
        catch_unwind(AssertUnwindSafe(|| {
            c.args
                .iter()
                .all(|a| matches!(vm_f(c.src, a), Some(vm) if run(a) == vm))
        }))
        .unwrap_or(false)
    };
    // Path 1: single-function CFG lowering (typed).
    let cfg_res = catch_unwind(AssertUnwindSafe(|| try_program_cfg_fabric(c.src, "f")));
    if let Ok(Ok(pf)) = &cfg_res {
        let cfg = serialize_program(pf);
        if eq_vm(&|a| eval_wprog_config(&cfg, a)) {
            return LowersEqVm;
        }
    }
    // Path 2: the multi-function / iterator / recursion lowering (panics on unsupported → caught).
    let calls_ok = catch_unwind(AssertUnwindSafe(|| {
        let cfg = program_calls_fabric(c.src, "f");
        c.args
            .iter()
            .all(|a| matches!(vm_f(c.src, a), Some(vm) if eval_wprog_config(&cfg, a) == vm))
    }))
    .unwrap_or(false);
    if calls_ok {
        return LowersEqVm;
    }
    // Neither path produced ≡VM: report the CFG (typed) path's failure mode.
    match cfg_res {
        Err(_) => Panicked,
        Ok(Err(_)) => TypedErr,
        Ok(Ok(_)) => WrongResult,
    }
}

// ── The corpus. Honest tags: Supported = actually lowers ≡VM today; KnownGap = actually refused today. ──
const CLASSES: &[Class] = &[
    // ---- Supported: the straight-line + branch + while + collection subset that lowers today ----
    Class { name: "straight-line arith", status: Supported, src: "## To f (x: Int) -> Int:\n    Return x * 3 + 1.\n", args: &[&[0], &[5], &[42], &[-7]] },
    Class { name: "if / branch (if-convert)", status: Supported, src: "## To f (x: Int) -> Int:\n    Let mutable r be 0.\n    If x is greater than 5:\n        Set r to 1.\n    Return r.\n", args: &[&[3], &[7]] },
    Class { name: "while accumulate loop", status: Supported, src: "## To f (n: Int) -> Int:\n    Let mutable acc be 0.\n    Let mutable i be 0.\n    While i is less than n:\n        Set acc to acc + i.\n        Set i to i + 1.\n    Return acc.\n", args: &[&[0], &[1], &[5], &[10]] },
    Class { name: "static-index list read", status: Supported, src: "## To f (x: Int) -> Int:\n    Let ys be [10, 20, 30].\n    Return item 2 of ys + x.\n", args: &[&[0], &[5]] },
    Class { name: "integer division", status: Supported, src: "## To f (a: Int) -> Int:\n    Return (a * 100) / 7.\n", args: &[&[12345], &[70], &[1]] },
    Class { name: "loop-guard-bounded index", status: Supported, src: "## To f (n: Int) -> Int:\n    Let mutable xs be a new Seq of Int.\n    Let mutable i be 0.\n    While i is less than 16:\n        Push i to xs.\n        Set i to i + 1.\n    Let mutable best be 0.\n    Let mutable t be 1.\n    While t is at most 16:\n        If item t of xs is greater than best:\n            Set best to item t of xs.\n        Set t to t + 1.\n    Return best.\n", args: &[&[0]] },
    Class { name: "param-indexed read (in-bounds)", status: Supported, src: "## To f (n: Int) -> Int:\n    Let mutable xs be a new Seq of Int.\n    Let mutable i be 0.\n    While i is less than 8:\n        Push i to xs.\n        Set i to i + 1.\n    Return item (n + 1) of xs.\n", args: &[&[0], &[3]] },

    // Division lowers even with a dynamic (possibly-zero) divisor: the fabric drops the div-by-zero check, so
    // it equals the VM on every VM-ACCEPTED (nonzero) input — the same "≡VM on valid inputs" convention the
    // other fabric locks use for reads/writes. (Making that error-path divergence explicit + trap-locked is
    // Phase 1b; here it is simply Supported over valid inputs.)
    Class { name: "dynamic divisor (nonzero inputs)", status: Supported, src: "## To f (b: Int) -> Int:\n    Return 100 / b.\n", args: &[&[7], &[5], &[3]] },

    // Phase 3: integer abs/min/max wired as select-DAG tiles (a negate + a `Select` mux).
    Class { name: "builtin abs (int)", status: Supported, src: "## To f (x: Int) -> Int:\n    Return abs(0 - x).\n", args: &[&[5], &[9], &[0]] },
    Class { name: "builtin min (int)", status: Supported, src: "## To f (a: Int, b: Int) -> Int:\n    Return min(a, b).\n", args: &[&[3, 7], &[7, 3], &[5, 5]] },
    Class { name: "builtin max (int)", status: Supported, src: "## To f (a: Int, b: Int) -> Int:\n    Return max(a, b).\n", args: &[&[3, 7], &[7, 3], &[5, 5]] },

    // for-each: lowers via `program_calls_fabric`'s IterNext handling (index = heap[iptr], value =
    // heap[coll+index+1]) — the CFG path refuses IterNext, the calls path lowers it. ≡VM either way.
    Class { name: "for-each over a seq", status: Supported, src: "## To f (n: Int) -> Int:\n    Let mutable xs be a new Seq of Int.\n    Push n to xs.\n    Push n to xs.\n    Let mutable acc be 0.\n    Repeat for x in xs:\n        Set acc to acc + x.\n    Return acc.\n", args: &[&[3], &[5]] },
    // (The EXHAUSTIVE, ratcheted gap frontier over all 115 ops lives in `wprog_op_reachability_lock`; this
    // corpus is a representative sample of the lowering surface, not the whole ISA.)
    Class { name: "builtin abs (float)", status: Supported, src: "## To f (x: Int) -> Int:\n    Let v be 0.0 - 3.5.\n    Return floor(abs(v)).\n", args: &[&[0]] },
    Class { name: "builtin min (float)", status: Supported, src: "## To f (x: Int) -> Int:\n    Let a be 2.5.\n    Let b be 7.5.\n    Return floor(min(a, b)).\n", args: &[&[0]] },
    Class { name: "builtin max (float)", status: Supported, src: "## To f (x: Int) -> Int:\n    Let a be 2.5.\n    Let b be 7.5.\n    Return floor(max(a, b)).\n", args: &[&[0]] },
];

#[test]
fn the_lowering_coverage_ratchet_holds() {
    // Silence panic backtraces from the intentional catch_unwind probes — a clean report, not a wall of noise.
    std::panic::set_hook(Box::new(|_| {}));

    let mut rows: Vec<(String, Outcome)> = Vec::new();
    let mut regressions: Vec<&str> = Vec::new(); // Supported that no longer lowers ≡VM
    let mut promote: Vec<&str> = Vec::new(); // KnownGap that now lowers ≡VM
    let mut panics: Vec<&str> = Vec::new(); // KnownGap that panics instead of a typed error
    let mut miscompiles: Vec<&str> = Vec::new(); // KnownGap that lowers to a WRONG result

    for c in CLASSES {
        let o = evaluate(c);
        let tag = match c.status {
            Supported => "Supported".to_string(),
            KnownGap(r) => format!("KnownGap({r})"),
        };
        rows.push((format!("{:32} [{tag}]", c.name), o));
        match (c.status, o) {
            (Supported, LowersEqVm) => {}
            (Supported, _) => regressions.push(c.name),
            (KnownGap(_), TypedErr) => {}
            (KnownGap(_), LowersEqVm) => promote.push(c.name),
            (KnownGap(_), Panicked) => panics.push(c.name),
            (KnownGap(_), WrongResult) => miscompiles.push(c.name),
        }
    }

    let _ = std::panic::take_hook();
    let n_sup = CLASSES.iter().filter(|c| c.status == Supported).count();
    let n_gap = CLASSES.len() - n_sup;
    eprintln!("\nWHOLE-PROGRAM LOWERING COVERAGE  ({n_sup} supported, {n_gap} known-gaps):");
    for (label, o) in &rows {
        eprintln!("  {label:70} -> {o:?}");
    }

    assert!(
        regressions.is_empty(),
        "REGRESSION — Supported classes that no longer lower ≡VM: {regressions:?}"
    );
    assert!(
        promote.is_empty(),
        "PROMOTE — KnownGap classes that now lower ≡VM (mark them Supported): {promote:?}"
    );
    assert!(panics.is_empty(), "CONVERT — KnownGap classes that PANIC instead of returning a typed LoweringError: {panics:?}");
    assert!(miscompiles.is_empty(), "MISCOMPILE — KnownGap classes that lower to a WRONG result (must refuse, not lie): {miscompiles:?}");
}
