//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! PARAM / RETURN × TIER COVERAGE LOCK — the pit-of-success invariant that function PARAMETERS and
//! RETURN VALUES are FIRST-CLASS for EVERY value kind, byte-identically on EVERY execution tier.
//!
//! LOGOS's promise: a value of ANY kind — Int, Real, Bool, Text, Seq (incl. nested), Set, struct, and
//! (the new surface) anonymous tuples `(Int, Int)` + named records `(quotient: Int, remainder: Int)` +
//! multiple return + call-site destructuring — can be PASSED INTO a function and RETURNED OUT of one,
//! and means the same thing on the tree-walker, the register VM, the AOT-C (gcc) backend, AND the
//! AOT-Rust backend (all four EXECUTED and compared to the oracle byte-for-byte). The WASM tier lowers
//! here (no `wasmi` in this crate) and is EXECUTION-locked in the compile crate's
//! `wasm_aot_lock::first_class_param_return_round_trips_on_wasm`. Each probe ROUND-TRIPS a value through
//! a function (`## To rt (x: T) -> T: Return x.`) and characterizes the result with `Show`.
//!
//! TWO RATCHET AXES, both shrink-only, goal = both empty:
//!   • `PENDING` — probes whose TYPE does not parse yet (the unimplemented tuple/record surface). While
//!     a probe is PENDING its oracle failure is EXPECTED. The moment it parses+runs on the oracle the
//!     lock DEMANDS it be promoted out of PENDING (a silently-landed feature must be locked in).
//!   • `KNOWN_GAPS` — (probe, tier) pairs that PARSE but do not yet AGREE on a specific backend. Each
//!     carries a reason; delete it the instant the gap closes.
//!
//! The lock FAILS on any drift: an undocumented gap (a type broke on a tier), a silently-fixed gap
//! (delete the row), or a PENDING probe that started working (promote it). Coverage can only rise.
//! ════════════════════════════════════════════════════════════════════════════════════════════════
#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::compile::{
    compile_to_c, compile_to_wasm, tw_outcome, vm_outcome, FIRST_CLASS_VALUE_KINDS,
};

/// A value kind and a self-contained program that ROUND-TRIPS it through a function (param → return)
/// and `Show`s a characterizing result. The tree-walker output is the oracle; every tier must agree.
struct Probe {
    name: &'static str,
    program: &'static str,
}

const PROBES: &[Probe] = &[
    // ── existing value kinds: param+return round-trip must work on EVERY tier (no gaps) ────────────
    Probe { name: "int", program: "## To rt (x: Int) -> Int:\n    Return x.\n## Main\n    Show rt(42).\n" },
    Probe { name: "real", program: "## To rt (x: Real) -> Real:\n    Return x.\n## Main\n    Show floor(rt(3.5) * 2.0).\n" },
    Probe { name: "bool", program: "## To rt (x: Bool) -> Bool:\n    Return x.\n## Main\n    Show rt(2 is greater than 1).\n" },
    Probe { name: "text", program: "## To rt (x: Text) -> Text:\n    Return x.\n## Main\n    Show upper(rt(\"hi\")).\n" },
    Probe { name: "seq_int", program: "## To rt (x: Seq of Int) -> Seq of Int:\n    Return x.\n## Main\n    Let r be rt([10, 20, 30]).\n    Show item 2 of r.\n" },
    Probe { name: "seq_text", program: "## To rt (x: Seq of Text) -> Seq of Text:\n    Return x.\n## Main\n    Let r be rt([\"a\", \"b\"]).\n    Show item 1 of r.\n" },
    Probe { name: "seq_seq", program: "## To rt (x: Seq of (Seq of Int)) -> Seq of (Seq of Int):\n    Return x.\n## Main\n    Let r be rt([[1, 2], [3]]).\n    Show item 2 of item 1 of r.\n" },
    Probe { name: "set_int", program: "## To rt (x: Set of Int) -> Set of Int:\n    Return x.\n## Main\n    Let s be a new Set of Int.\n    Add 7 to s.\n    Let r be rt(s).\n    Show the length of r.\n" },
    Probe { name: "struct", program: "## A Point has:\n    A x: Int.\n    A y: Int.\n## To rt (p: Point) -> Point:\n    Return p.\n## Main\n    Let r be rt(a new Point with x 3 and y 4).\n    Show r's x.\n" },

    // ── the NEW first-class surface — currently PENDING (the type does not parse yet) ──────────────
    // Anonymous tuple as a param+return type.
    Probe { name: "tuple_anon", program: "## To rt (x: (Int, Int)) -> (Int, Int):\n    Return x.\n## Main\n    Let r be rt((7, 9)).\n    Show item 1 of r.\n" },
    // Anonymous tuple as a MULTIPLE-RETURN type.
    Probe { name: "multi_return", program: "## To dm (n: Int) -> (Int, Int):\n    Return (n / 5, n mod 5).\n## Main\n    Let r be dm(17).\n    Show item 2 of r.\n" },
    // Named record return, accessed by field name.
    Probe { name: "record_named", program: "## To dm (n: Int) -> (quotient: Int, remainder: Int):\n    Return (n / 5, n mod 5).\n## Main\n    Let r be dm(17).\n    Show r's quotient.\n" },
    // Call-site destructuring of a tuple return.
    Probe { name: "destructure", program: "## To dm (n: Int) -> (Int, Int):\n    Return (n / 5, n mod 5).\n## Main\n    Let (q, rem) be dm(17).\n    Show rem.\n" },
    // A Seq of tuples as a param+return type (nested tuple in a collection).
    Probe { name: "seq_tuple", program: "## To rt (x: Seq of (Int, Int)) -> Seq of (Int, Int):\n    Return x.\n## Main\n    Let r be rt([(1, 2), (3, 4)]).\n    Let p be item 1 of r.\n    Show item 2 of p.\n" },
];

/// Probes whose TYPE does not parse yet (the unimplemented first-class-tuple surface). SHRINK-ONLY to
/// ∅ — as each phase lands, the probe starts parsing and MUST be removed here (the lock forces it).
const PENDING: &[&str] = &[
    // Phase 1 LANDED — anonymous tuple types `(Int, Int)`, multiple-return, `Seq of (Int, Int)`.
    // Phase 2 LANDED — named records `(quotient: Int, remainder: Int)` with `r's field` access.
    // Phase 4 LANDED — `Let (a, b) be f()` destructuring (desugar to `item N of temp`).
];

/// (probe, tier) pairs that PARSE but do not yet AGREE on a backend. Each carries a reason. DELETE an
/// entry the instant its gap closes — the lock forces it. Goal: empty.
const KNOWN_GAPS: &[(&str, &str)] = &[
    // ∅ — EMPTY. Every value kind now round-trips (param → return) on ALL FIVE tiers: the tree-walker,
    // the VM, AOT-Rust, AOT-C, and the SELF-CONTAINED WASM backend (`compile_to_wasm`, no linked
    // runtime). The C tier was closed in Phase 5 (synthetic `LTuple_…` / `Seq_<LTuple>` / `Seq_<inner>`
    // families + `CType::Record`/`SeqSeq` + the `Stmt::Splice` destructure arm). The FINAL WASM gaps
    // closed this session, each a small, principled fix in the AOT type bridge:
    //   • `real`     — `boundary_of_name` gained the `Real` → `Float` alias (every other resolver already
    //                  accepts both; the WASM/AOT bridge alone knew only `Float`, so a `-> Real` return
    //                  seeded no kind → "unknown-kind return").
    //   • `seq_seq`  — `boundary_to_kind` maps a nested `SeqInt` element to `Kind::SeqSeqInt` (the one
    //                  self-describing nested-sequence kind, which already existed).
    //   • `set_int`  — new `BoundaryType::Set(Box<BoundaryType>)` (added LAST for bincode index stability)
    //                  + a `Set of …` arm in `boundary_of_type_expr` + a `Set`→`Kind::Set`/`SetText` arm
    //                  in `boundary_to_kind`. A set param/return now seeds its self-describing handle kind.
    //   • `struct`   — a struct RETURNED AS A PARAMETER (never built in the callee) had its resolved field
    //                  layout only in `call_result`, not `reg_layout`, so `fn_return_struct_layout` bailed
    //                  and the caller's `r's field` had no static slot. It now falls back to `call_result`.
    //   • `seq_tuple`— lowered once the boundary resolutions above were in place.
    // EXECUTION (not merely lowering) is proven byte-for-byte WASM == tree-walker == VM by the compile
    // crate's `wasm_aot_lock::first_class_param_return_round_trips_on_wasm`. KNOWN_GAPS is now terminal:
    // it may only ever stay empty — a new (probe, tier) disagreement is a REGRESSION to fix at the backend.
];

fn is_pending(name: &str) -> bool {
    PENDING.contains(&name)
}
fn is_known_gap(name: &str, tier: &str) -> bool {
    KNOWN_GAPS.contains(&(name, tier))
}

/// The four-case ratchet verdict, as a PURE function of (does the tier agree?, is this a documented
/// gap?). Kept independent of the live `KNOWN_GAPS` set so the guard-on-the-guard can exercise EVERY
/// branch — including the `SilentFix` path — even now that `KNOWN_GAPS` is terminally empty.
#[derive(PartialEq, Debug)]
enum Verdict {
    Ok,
    Regression,
    SilentFix,
}

fn classify(agrees: bool, is_gap: bool) -> Verdict {
    match (agrees, is_gap) {
        (true, false) => Verdict::Ok,
        (false, true) => Verdict::Ok,
        (false, false) => Verdict::Regression,
        (true, true) => Verdict::SilentFix,
    }
}

/// Record one tier's verdict against the (probe, tier) ratchet.
fn judge(name: &str, tier: &str, agrees: bool, violations: &mut Vec<String>) {
    match classify(agrees, is_known_gap(name, tier)) {
        Verdict::Ok => {}
        Verdict::Regression => violations.push(format!(
            "REGRESSION: `{name}` no longer round-trips (param→return) on tier `{tier}`. A value kind \
             broke as a parameter/return on a backend. Fix the backend — do NOT add it to KNOWN_GAPS."
        )),
        Verdict::SilentFix => violations.push(format!(
            "FIXED: `{name}` now round-trips on tier `{tier}` — DELETE (\"{name}\", \"{tier}\") from \
             KNOWN_GAPS so the ratchet locks the win in."
        )),
    }
}

/// Compile through the AOT-C backend, gcc it, run it, return trimmed stdout — or `None` on any failure
/// (a gap). Mirrors `type_tier_coverage_lock`'s C runner.
fn c_backend_output(program: &str) -> Option<String> {
    let c = compile_to_c(program).ok()?;
    let dir = std::env::temp_dir().join(format!("logos_pr_{}_{:x}", std::process::id(), c.len()));
    std::fs::create_dir_all(&dir).ok()?;
    let src = dir.join("out.c");
    let bin = dir.join("out");
    std::fs::write(&src, &c).ok()?;
    let compiled = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&src)
        .arg("-lm")
        .output()
        .ok()?;
    let out = if compiled.status.success() {
        std::process::Command::new(&bin)
            .output()
            .ok()
            .filter(|r| r.status.success())
            .map(|r| String::from_utf8_lossy(&r.stdout).trim().to_string())
    } else {
        None
    };
    let _ = std::fs::remove_dir_all(&dir);
    out
}

fn have_gcc() -> bool {
    common::tool_available("gcc")
}

#[test]
fn param_return_is_first_class_across_tiers() {
    let gcc = have_gcc();
    let mut violations = Vec::new();

    for p in PROBES {
        let oracle = tw_outcome(p.program);
        if let Some(err) = &oracle.error {
            // The oracle (tree-walker) could not parse/run this probe.
            if is_pending(p.name) {
                continue; // expected — this is the unimplemented RED spec, tracked in PENDING
            }
            violations.push(format!(
                "REGRESSION: probe `{}` broke on the ORACLE (tree-walker) — it must always run: {}",
                p.name,
                &err[..err.len().min(90)]
            ));
            continue;
        }
        // The oracle runs. A PENDING probe that now parses is a LANDED FEATURE → force its promotion.
        if is_pending(p.name) {
            violations.push(format!(
                "PROMOTE: probe `{}` now parses + runs on the oracle — the feature landed. REMOVE it from \
                 PENDING so the ratchet demands it on every tier (and add any not-yet-working tier to \
                 KNOWN_GAPS with a reason).",
                p.name
            ));
        }
        let want = oracle.output.trim();

        let vm = vm_outcome(p.program);
        judge(
            p.name,
            "vm",
            vm.error.is_none() && vm.output.trim() == want,
            &mut violations,
        );

        if gcc {
            judge(
                p.name,
                "c",
                c_backend_output(p.program).as_deref() == Some(want),
                &mut violations,
            );
        }
    }

    assert!(
        violations.is_empty(),
        "PARAM/RETURN × TIER coverage drift ({} issue(s)):\n  - {}",
        violations.len(),
        violations.join("\n  - ")
    );
}

/// AOT-Rust tier — an EXECUTION gate: the round-trip must compile through the Rust backend (Logos → Rust
/// → `cargo run`, via `common::run_logos`) AND print the tree-walker oracle's output byte-for-byte. This
/// is the same standard the C tier is held to (compile → run → compare), so the AOT-Rust tier is locked
/// against the oracle by EXECUTION, not merely by lowering. A probe that does not parse on the oracle (a
/// PENDING feature) is skipped. NOTE: this rustc-compiles one tiny crate per probe (shared target cache),
/// so it is the slowest tier here — correctness over speed, matching the repo's e2e differential gates.
#[test]
fn param_return_executes_on_aot_rust() {
    let mut violations = Vec::new();
    for p in PROBES {
        let oracle = tw_outcome(p.program);
        if oracle.error.is_some() || is_pending(p.name) {
            continue;
        }
        let want = oracle.output.trim();
        let r = common::run_logos(p.program);
        judge(
            p.name,
            "aot_rust",
            r.success && r.stdout.trim() == want,
            &mut violations,
        );
    }
    assert!(
        violations.is_empty(),
        "AOT-Rust execution drift ({} issue(s)):\n  - {}",
        violations.len(),
        violations.join("\n  - ")
    );
}

/// WASM tier — a LOWERING gate: the round-trip must lower to WASM bytes without error (the direct WASM
/// backend represents the type). The tests crate has no `wasmi`, so this is a lowering check, not execution.
///
/// EXECUTION (the WASM output byte-for-byte == the tree-walker oracle == the VM, via `wasmi`) is verified
/// in the compile crate — `wasm_aot_lock::first_class_param_return_round_trips_on_wasm` — which runs EVERY
/// value kind here (plus heterogeneous tuples/records, float-in-aggregate, and mutate-through-boundary)
/// and asserts each is fully self-contained. ANTI-DRIFT: a new PROBE below must gain a matching program
/// there. Together the two locks make WASM both lower AND compute correctly, pinned to the same oracle as
/// TW/VM/C — so WASM is locked against every other tier transitively (all tiers == the tree-walker).
#[test]
fn param_return_lowers_on_wasm() {
    let mut violations = Vec::new();
    for p in PROBES {
        if tw_outcome(p.program).error.is_some() || is_pending(p.name) {
            continue;
        }
        judge(
            p.name,
            "wasm",
            compile_to_wasm(p.program).is_ok(),
            &mut violations,
        );
    }
    assert!(
        violations.is_empty(),
        "WASM lowering drift ({} issue(s)):\n  - {}",
        violations.len(),
        violations.join("\n  - ")
    );
}

/// Guard on the guard: the ratchet verdict must reject BOTH drift directions. Exercised through the
/// pure `classify` so every branch is covered regardless of whether `KNOWN_GAPS` is empty (it is now).
#[test]
fn ratchet_rejects_both_regression_and_silent_fix() {
    // A NOT-documented (probe, tier): passing is fine, failing is a REGRESSION to fix at the backend.
    assert_eq!(
        classify(true, false),
        Verdict::Ok,
        "an undocumented tier that agrees is fine"
    );
    assert_eq!(
        classify(false, false),
        Verdict::Regression,
        "an undocumented tier that broke is a regression"
    );
    // A documented gap: failing is the EXPECTED state; passing is a SilentFix that must be delisted.
    assert_eq!(
        classify(false, true),
        Verdict::Ok,
        "a documented gap that still fails is expected"
    );
    assert_eq!(
        classify(true, true),
        Verdict::SilentFix,
        "a documented gap that now passes must be delisted"
    );

    // And the message wiring through `judge` surfaces the REGRESSION text for the undocumented break.
    let mut v = Vec::new();
    judge("x", "c", true, &mut v);
    judge("x", "c", false, &mut v);
    assert_eq!(
        v.len(),
        1,
        "an undocumented gap must be flagged exactly once"
    );
    assert!(v[0].contains("REGRESSION"));
}

/// No stale ratchet rows: every PENDING / KNOWN_GAPS entry names a live probe.
#[test]
fn ratchet_rows_reference_live_probes() {
    for name in PENDING {
        assert!(
            PROBES.iter().any(|p| p.name == *name),
            "PENDING names `{name}`, but no probe exercises it — remove the stale row."
        );
    }
    for (name, _tier) in KNOWN_GAPS {
        assert!(
            PROBES.iter().any(|p| p.name == *name),
            "KNOWN_GAPS names `{name}`, but no probe exercises it — remove the stale row."
        );
    }
}

/// ULTIMATE ANTI-DRIFT — this ratchet's PROBE set is EXACTLY the shared canonical value-kind set
/// (`logicaffeine_compile::compile::FIRST_CLASS_VALUE_KINDS`), the SINGLE SOURCE OF TRUTH the compile
/// crate's WASM execution lock also consumes. A value kind therefore cannot be covered on one lock and
/// forgotten on the other: adding a name to the shared const fails BOTH locks until each covers it, and a
/// probe here that isn't in the canonical set (or a canonical kind with no probe) fails THIS assertion.
/// Compiler-enforced across crates — the strongest anti-drift available without a shared program list
/// (programs stay tier-appropriate; only the kind SET is shared, which is what coverage drift is about).
#[test]
fn probes_are_exactly_the_shared_canonical_kind_set() {
    use std::collections::BTreeSet;
    let probe_kinds: BTreeSet<&str> = PROBES.iter().map(|p| p.name).collect();
    let canonical: BTreeSet<&str> = FIRST_CLASS_VALUE_KINDS.iter().copied().collect();
    let missing: Vec<&&str> = canonical.difference(&probe_kinds).collect();
    let extra: Vec<&&str> = probe_kinds.difference(&canonical).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "PROBE SET DRIFTED from the shared canonical kinds `FIRST_CLASS_VALUE_KINDS`.\n  \
         canonical kinds with no probe here: {missing:?}\n  \
         probes here not in the canonical set: {extra:?}\n  \
         Fix by adding/removing a Probe or editing the shared const — they must match EXACTLY, so both \
         this ratchet and the compile crate's WASM execution lock move together."
    );
}

/// ROBUSTNESS beyond the canonical single-kind probes (which only `Show` the round-tripped value): a
/// genuinely HETEROGENEOUS tuple `(Int, Text)` / mixed-type record `(Text, Int, Real)` whose element is
/// USED IN A TYPED POSITION — `upper(item 2 of r)` needs a `Text`, `r's ratio * 2.0` needs a `Real`, not
/// a boxed value. All four EXECUTING tiers must still agree: the tree-walker/VM are dynamically typed; C
/// uses real typed tuple structs (`.fN`); AOT-Rust stores tuples/records as a positional `Vec<Value>` and
/// must UNBOX the extracted element back to its scalar (the element/field types ride in the type marker).
/// A regression on any tier — an AOT-Rust `Value` left un-unboxed, a C field mismatch — trips this.
#[test]
fn heterogeneous_typed_extraction_agrees_across_tiers() {
    let gcc = have_gcc();
    let programs: &[(&str, &str)] = &[
        (
            "tuple_hetero",
            "## To rt (x: (Int, Text)) -> (Int, Text):\n    Return x.\n## Main\n\
             Let r be rt((7, \"hi\")).\n    Show item 1 of r.\n    Let t be item 2 of r.\n    Show upper(t).\n",
        ),
        (
            "record_mixed",
            "## To mk (n: Int) -> (p: Text, q: Int, w: Real):\n    Return (\"hi\", n, 1.5).\n## Main\n\
             Let r be mk(9).\n    Show r's p.\n    Show r's q.\n    Let d be r's w.\n    Show floor(d * 2.0).\n",
        ),
    ];
    let mut violations = Vec::new();
    for (name, src) in programs {
        let tw = tw_outcome(src);
        assert_eq!(
            tw.error, None,
            "`{name}` must run cleanly on the tree-walker oracle:\n{src}"
        );
        let want = tw.output.trim();

        let vm = vm_outcome(src);
        if !(vm.error.is_none() && vm.output.trim() == want) {
            violations.push(format!(
                "VM != oracle on `{name}` (got {:?}, want {want:?})",
                vm.output.trim()
            ));
        }
        let aot = common::run_logos(src);
        if !(aot.success && aot.stdout.trim() == want) {
            violations.push(format!(
                "AOT-Rust != oracle on `{name}` (got {:?}, want {want:?})",
                aot.stdout.trim()
            ));
        }
        if gcc && c_backend_output(src).as_deref() != Some(want) {
            violations.push(format!("C != oracle on `{name}` (want {want:?})"));
        }
    }
    assert!(
        violations.is_empty(),
        "HETEROGENEOUS TYPED-EXTRACTION drift ({} issue(s)) — a tuple/record element used at a typed use \
         site diverged across tiers:\n  - {}",
        violations.len(),
        violations.join("\n  - ")
    );
}
