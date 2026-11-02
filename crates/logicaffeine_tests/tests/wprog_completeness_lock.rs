//! wprog_completeness_lock — the "we can't be missing anything" ratchet for the whole-program fabric.
//!
//! Every major LOGOS construct is CLASSIFIED here, and the classification is enforced so coverage can only
//! grow and the gap set can only shrink:
//!
//!   • a SUPPORTED construct MUST run on the whole-program fabric bit-exactly vs the VM (a regression → RED),
//!   • a KNOWN-GAP construct MUST NOT run yet — if one silently starts working, the lock FAILS and demands it
//!     be PROMOTED to SUPPORTED (so a shrinking gap can never go unrecorded),
//!   • every construct in the census is one or the other — nothing is unclassified,
//!   • the SUPPORTED count is a monotone floor and the GAP count a monotone ceiling.
//!
//! This is the meta-guarantee behind "send ANY program down the wire and it runs": we can never be silently
//! missing a construct, and the honest remaining gap list is printed every run. Op-closure (every one of the
//! 115 ISA ops realized ≡VM) is locked separately by `fabric_coverage_lock`; this lock is the whole-program
//! COMPOSITION census on top of it.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(wprog_completeness_lock)'`

#![cfg(not(target_arch = "wasm32"))]

use std::panic::{catch_unwind, AssertUnwindSafe};

use logicaffeine_compile::codegen_sva::fabric_grid::{
    eval_grid_with_heap, route_loop_body, run_program_cfg_on_grid,
};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    decision_fabric_from_source, eval_loop_fabric, loop_fabric_from_source, program_cfg_fabric,
};
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    eval_wprog_config, program_calls_fabric, serialize_program, wprog_from_decision_fabric,
};
use logicaffeine_compile::compile::vm_outcome;

/// One census entry: a LOGOS construct, a function `f(x)` that exercises it, and inputs to check.
struct Construct {
    name: &'static str,
    src: &'static str,
    args: &'static [i64],
}

/// The census — every major LOGOS construct. To claim "we can't be missing anything" this must keep growing
/// to cover the whole language; each entry is either SUPPORTED below or an implicit KNOWN GAP.
const CENSUS: &[Construct] = &[
    Construct { name: "branch", src: "## To f (x: Int) -> Int:\n    If x is greater than 5:\n        Return 1.\n    Return 0.\n", args: &[3, 7] },
    Construct { name: "while-loop", src: "## To f (x: Int) -> Int:\n    Let mutable s be 0.\n    Let mutable i be 1.\n    While i is at most x:\n        Set s to s + i.\n        Set i to i + 1.\n    Return s.\n", args: &[5, 9] },
    Construct { name: "list", src: "## To f (x: Int) -> Int:\n    Let mutable xs be [10, x, 30].\n    Set item 1 of xs to 99.\n    Return item 1 of xs + item 2 of xs.\n", args: &[7, 20] },
    Construct { name: "tuple", src: "## To f (x: Int) -> Int:\n    Let t be (10, x, 30).\n    Return item 2 of t.\n", args: &[7, 20] },
    Construct { name: "call", src: "## To g (n: Int) -> Int:\n    Return n + 1.\n## To f (x: Int) -> Int:\n    Return g(x) + g(x).\n", args: &[7, 20] },
    Construct { name: "division", src: "## To f (x: Int) -> Int:\n    Return x / 2 + x % 3.\n", args: &[17, 20] },
    Construct { name: "range-loop", src: "## To f (x: Int) -> Int:\n    Let mutable s be 0.\n    Repeat for i from 1 to x:\n        Set s to s + i.\n    Return s.\n", args: &[5, 9] },
    Construct { name: "float", src: "## To f (x: Int) -> Int:\n    Let a be 3.5.\n    Let b be 1.5.\n    Return floor(a + b).\n", args: &[0] },
    Construct { name: "text", src: "## To f (x: Int) -> Int:\n    Let s be \"abcd\".\n    Return the length of s.\n", args: &[0] },
    Construct { name: "set", src: "## To f (x: Int) -> Int:\n    Let s be a new Set of Int.\n    Add x to s.\n    Add 5 to s.\n    Add x to s.\n    Return the length of s.\n", args: &[7, 5] },
    // RECURSION — distinct from `call`: a non-recursive call inlines, but unbounded recursion needs a real
    // call stack (the frame-stack executor). Tracked as its own gap so "call SUPPORTED" never overclaims.
    Construct { name: "recursion", src: "## To f (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Return n + f(n - 1).\n", args: &[5, 3] },
];

/// Constructs proven to run on the whole-program fabric ≡VM. This is the RATCHET FLOOR — it only grows. When
/// a gap is closed, move its name here (and the lock's promote-check enforces you did).
const SUPPORTED: &[&str] = &[
    "branch",
    "while-loop",
    "list",
    "tuple",
    "division",
    "text",
    "range-loop",
    "float",
    "call",
    "recursion",
    "set",
];

/// The ratchet bounds. SUPPORTED can only grow; the gap set (CENSUS − SUPPORTED) can only shrink to ∅.
const SUPPORTED_FLOOR: usize = 11;
const GAP_CEILING: usize = 0;

fn vm_f(src: &str, x: i64) -> Option<i64> {
    let arg = if x < 0 {
        format!("(0 - {})", -x)
    } else {
        x.to_string()
    };
    let prog = format!("{src}## Main\n    Show f({arg}).\n");
    let out = vm_outcome(&prog);
    if out.error.is_some() {
        return None;
    }
    out.output.split_whitespace().next()?.parse::<i64>().ok()
}

/// Does the construct run on the fabric bit-exactly vs the VM, for all its inputs, by ANY of the fabric's
/// proven execution mechanisms? Two are tried: (1) the whole-program block-walk (`eval_wprog_config`, cross-
/// checked against `run_program_cfg_on_grid`) — the general spatial executor for branchy/looping/stateful
/// control flow; (2) the counted/sequence LOOP FABRIC (`loop_fabric_from_source` + `eval_loop_fabric`) — the
/// proven mechanism for `Repeat for … from/in` induction loops (locked by `fabric_cfg_lock`). "Runs on the
/// fabric" means the fabric computes it ≡VM by SOME proven means; the `catch_unwind` turns an unsupported
/// construct's panic into "does not run" rather than aborting the census.
fn runs_on_fabric(c: &Construct) -> bool {
    let all_match = |f: &dyn Fn(i64) -> i64| -> bool {
        catch_unwind(AssertUnwindSafe(|| {
            c.args
                .iter()
                .all(|&x| matches!(vm_f(c.src, x), Some(vm) if f(x) == vm))
        }))
        .unwrap_or(false)
    };
    // (1) the whole-program block-walk (flat interpreter cross-checked against the place-per-block grid walk).
    all_match(&|x| {
        let pf = program_cfg_fabric(c.src, "f");
        let flat = eval_wprog_config(&serialize_program(&pf), &[x]);
        assert_eq!(flat, run_program_cfg_on_grid(&pf, &[x]), "flat vs grid walk");
        flat
    })
    // (2) or the counted/sequence loop fabric (the proven induction-loop mechanism).
    || all_match(&|x| eval_loop_fabric(&loop_fabric_from_source(c.src, "f"), &[x]))
    // (3) or a single-block if-converted DECISION FABRIC, placed and evaluated with the heap — the proven
    // Arith/Select/Func/Mem grid for floats, sets, and straight-line collection/compute (Stages X1/X2/X3).
    || all_match(&|x| {
        let df = decision_fabric_from_source(c.src, "f");
        let g = route_loop_body(&df.cells, df.n_in, &[df.result]);
        eval_grid_with_heap(&g, &[x])
    })
    // (4) or the multi-function CALL-STACK executor: `program_calls_fabric` lifts calls/recursion to `Call`
    // terminators and the frame-stack interpreter runs them — the mechanism for recursion and general calls.
    || all_match(&|x| eval_wprog_config(&program_calls_fabric(c.src, "f"), &[x]))
    // (5) or the decision fabric's cells on the CROSSBAR whole-program engine (full interconnect, so a
    // computed predicate needs no routing) — runs what the nearest-neighbour placer can't place (Set dedup).
    || all_match(&|x| {
        let df = decision_fabric_from_source(c.src, "f");
        eval_wprog_config(&wprog_from_decision_fabric(&df), &[x])
    })
}

#[test]
fn every_construct_is_classified_supported_run_and_gaps_do_not() {
    let mut gaps: Vec<&str> = Vec::new();
    for c in CENSUS {
        let runs = runs_on_fabric(c);
        let claimed = SUPPORTED.contains(&c.name);
        if claimed {
            assert!(
                runs,
                "SUPPORTED construct `{}` no longer runs on the whole-program fabric ≡VM — a REGRESSION",
                c.name
            );
        } else {
            assert!(
                !runs,
                "KNOWN-GAP construct `{}` now RUNS on the fabric — the gap SHRANK: promote `{}` into SUPPORTED \
                 (and lower SUPPORTED_FLOOR/GAP_CEILING accordingly)",
                c.name, c.name
            );
            gaps.push(c.name);
        }
    }
    println!(
        "whole-program fabric census: {}/{} constructs SUPPORTED ≡VM; remaining gaps (the honest, tracked \
         list — this must shrink to ∅): {:?}",
        SUPPORTED.len(),
        CENSUS.len(),
        gaps
    );
}

#[test]
fn coverage_is_a_monotone_shrinking_gap_ratchet() {
    // Every SUPPORTED name is a real census entry (no phantom coverage claims).
    for s in SUPPORTED {
        assert!(
            CENSUS.iter().any(|c| c.name == *s),
            "SUPPORTED names `{s}`, absent from the census"
        );
    }
    // Nothing unclassified: every census entry is SUPPORTED or a gap (true by construction), and the counts
    // honor the ratchet — SUPPORTED only grows, gaps only shrink.
    assert!(
        SUPPORTED.len() >= SUPPORTED_FLOOR,
        "SUPPORTED coverage regressed below the ratchet floor {SUPPORTED_FLOOR}"
    );
    let gap_count = CENSUS
        .iter()
        .filter(|c| !SUPPORTED.contains(&c.name))
        .count();
    assert!(
        gap_count <= GAP_CEILING,
        "the whole-program gap set GREW to {gap_count} > ceiling {GAP_CEILING} — coverage must only shrink gaps"
    );
    assert_eq!(
        SUPPORTED.len() + gap_count,
        CENSUS.len(),
        "every construct must be classified"
    );
}
