//! ★ DIMENSIONED CONTRACT LOCK ★ — units on `Assume`/`Require` range contracts.
//!
//! THE SPEC this file pins:
//!   1. FOLD — in an `Assume`/`Require that` range contract, literal quantity bounds fold at
//!      parse time to their EXACT SI magnitudes (exact-rational unit conversion, then the one
//!      nearest-f64 the bare twin would carry). The runtime condition compares plain floats on
//!      every tier, so a dimensioned program runs BYTE-EQUIVALENT to its bare-float twin.
//!   2. ANNOTATION — the contract's physical dimension rides `RuntimeAssert.dim` into
//!      `AssumedRange.dim`. Bare contracts carry `DIMENSIONLESS`.
//!   3. DISTRIBUTION — a unit stated on one bound distributes over the coordination
//!      (`between 200 and 700 millivolts` bounds v to [0.2, 0.7] volts SI).
//!   4. COHERENCE, SHIFT-LEFT — two stated units of different dimensions are analytically
//!      meaningless and fail AT PARSE, on every tier, before anything runs.
//!   5. CONTRADICTION — an inverted range (lo > hi after conversion) is a typed analog
//!      refusal, `ContradictoryContract`, for bare and dimensioned contracts alike (the
//!      runtime twin semantics — the assert simply fires — are unchanged).
//!   6. NO REACH — general `If`/`While` conditions are untouched: Quantity↔Quantity
//!      comparisons keep working exactly as before.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. The fold is contract-scoped by design; widening it
//! to general conditions would break Quantity comparison semantics (probe 6 guards this).

use logicaffeine_base::dimension::Dimension;
use logicaffeine_compile::codegen_ana::assumed_ranges;
use logicaffeine_compile::compile::{compile_to_analog, tw_outcome, vm_outcome};

// ============================================================
//  Corpus — every dimensioned program has its bare-float twin.
// ============================================================

const DIMENSIONED: &str = "## To scale (v: Float) -> Float:\n    Assume v is between 0.2 volts and 0.7 volts.\n    Return v * 0.5.\n\n## Main\n    Show scale(0.5).\n";
const BARE_TWIN: &str = "## To scale (v: Float) -> Float:\n    Assume v is between 0.2 and 0.7.\n    Return v * 0.5.\n\n## Main\n    Show scale(0.5).\n";

const DIMENSIONED_VIOLATION: &str =
    "## Main\n    Let v be 0.9.\n    Assume v is between 0.2 volts and 0.7 volts.\n    Show v.\n";
const BARE_VIOLATION: &str =
    "## Main\n    Let v be 0.9.\n    Assume v is between 0.2 and 0.7.\n    Show v.\n";

/// The stated unit distributes over the coordination — and the prefix converts.
const DISTRIBUTED: &str = "## To scale (v: Float) -> Float:\n    Assume v is between 200 and 700 millivolts.\n    Return v * 0.5.\n\n## Main\n    Show scale(0.5).\n";

/// Cross-unit, same dimension — each bound converts through its own unit.
const CROSS_UNIT: &str = "## To scale (v: Float) -> Float:\n    Assume v is between 0.2 volts and 700 millivolts.\n    Return v * 0.5.\n\n## Main\n    Show scale(0.5).\n";

/// Affine conversion — celsius offsets exactly (+273.15 as the rational 5463/20).
const AFFINE: &str = "## Main\n    Let t be 300.0.\n    Assume t is between 20 celsius and 30 celsius.\n    Show t.\n";

/// Analytically meaningless: a voltage lower bound against a mass upper bound. (Time units
/// are NOT used here on purpose: `0.7 seconds` is a CalendarUnit span literal — a different,
/// earlier parse path that already refuses on its own terms; temporal contracts belong to the
/// Span type system, not the SI fold.)
const MISMATCHED: &str = "## Main\n    Let v be 0.5.\n    Assume v is between 0.2 volts and 0.7 kilograms.\n    Show v.\n";

const INVERTED_DIMENSIONED: &str = "## To scale (v: Float) -> Float:\n    Assume v is between 0.7 volts and 0.2 volts.\n    Return v * 0.5.\n\n## Main\n    Show scale(0.5).\n";
const INVERTED_BARE: &str = "## To scale (v: Float) -> Float:\n    Assume v is between 0.7 and 0.2.\n    Return v * 0.5.\n\n## Main\n    Show scale(0.5).\n";

/// The `Require that` spelling folds identically (same statement, same desugar).
const REQUIRE_SPELLING: &str = "## To scale (v: Float) -> Float:\n    Require that v is between 0.2 volts and 0.7 volts.\n    Return v * 0.5.\n\n## Main\n    Show scale(0.5).\n";

/// Quantity variables in GENERAL conditions — the fold must never reach here.
const QUANTITY_IF: &str = "## Main\n    Let q be 0.5 volts.\n    If q is between 0.2 volts and 0.7 volts:\n        Show 1.\n";

// ============================================================
//  1 + 2. The fold: identical runtime, annotated extraction.
// ============================================================

#[test]
fn a_dimensioned_contract_runs_byte_equivalent_to_its_bare_twin() {
    let (dt, bt) = (tw_outcome(DIMENSIONED), tw_outcome(BARE_TWIN));
    assert_eq!(dt.error, None, "the dimensioned contract must EXECUTE (the fold makes the condition float-vs-float): {dt:?}");
    assert_eq!(dt.output, bt.output);
    let (dv, bv) = (vm_outcome(DIMENSIONED), vm_outcome(BARE_TWIN));
    assert_eq!(dv.error, None, "{dv:?}");
    assert_eq!(dv.output, bv.output);
    assert_eq!(
        dt.output.trim_end(),
        dv.output.trim_end(),
        "tw ≡ vm (modulo the runners' trailing-newline convention)"
    );
}

#[test]
fn a_violated_dimensioned_contract_fails_exactly_like_the_twin() {
    let (dt, bt) = (
        tw_outcome(DIMENSIONED_VIOLATION),
        tw_outcome(BARE_VIOLATION),
    );
    assert!(
        dt.error.is_some(),
        "0.9 is outside [0.2, 0.7] — the contract must fire"
    );
    assert_eq!(
        dt.error, bt.error,
        "same failure as the bare twin, not a different one"
    );
    let (dv, bv) = (
        vm_outcome(DIMENSIONED_VIOLATION),
        vm_outcome(BARE_VIOLATION),
    );
    assert!(dv.error.is_some());
    assert_eq!(dv.error, bv.error);
}

#[test]
fn assumed_ranges_recover_si_magnitudes_and_the_dimension() {
    let rs = assumed_ranges(DIMENSIONED);
    assert_eq!(rs.len(), 1);
    assert_eq!(rs[0].var, "v");
    assert_eq!(rs[0].lo, 0.2);
    assert_eq!(rs[0].hi, 0.7);
    assert_eq!(rs[0].dim, Dimension::voltage());

    let bare = assumed_ranges(BARE_TWIN);
    assert_eq!(bare.len(), 1);
    assert_eq!((bare[0].lo, bare[0].hi), (0.2, 0.7));
    assert_eq!(
        bare[0].dim,
        Dimension::DIMENSIONLESS,
        "bare contracts stay dimensionless"
    );

    let req = assumed_ranges(REQUIRE_SPELLING);
    assert_eq!(req.len(), 1);
    assert_eq!(
        req[0].dim,
        Dimension::voltage(),
        "`Require that` folds identically"
    );
}

// ============================================================
//  3. Distribution and cross-unit conversion.
// ============================================================

#[test]
fn the_stated_unit_distributes_and_prefixes_convert_exactly() {
    let rs = assumed_ranges(DISTRIBUTED);
    assert_eq!(rs.len(), 1);
    assert_eq!(
        (rs[0].lo, rs[0].hi),
        (0.2, 0.7),
        "200..700 mV = 0.2..0.7 V, exactly"
    );
    assert_eq!(rs[0].dim, Dimension::voltage());

    let cross = assumed_ranges(CROSS_UNIT);
    assert_eq!(cross.len(), 1);
    assert_eq!((cross[0].lo, cross[0].hi), (0.2, 0.7));
    assert_eq!(cross[0].dim, Dimension::voltage());

    // And the distributed program still RUNS like the twin.
    let out = tw_outcome(DISTRIBUTED);
    assert_eq!(out.error, None, "{out:?}");
    assert_eq!(out.output, tw_outcome(BARE_TWIN).output);
}

#[test]
fn affine_units_convert_through_the_exact_offset() {
    let rs = assumed_ranges(AFFINE);
    assert_eq!(rs.len(), 1);
    assert_eq!(
        (rs[0].lo, rs[0].hi),
        (293.15, 303.15),
        "20..30 °C = 293.15..303.15 K"
    );
    assert_eq!(rs[0].dim, Dimension::temperature());
    let out = tw_outcome(AFFINE);
    assert_eq!(out.error, None, "300.0 K is inside the band: {out:?}");
}

// ============================================================
//  4. Mismatched units fail at parse — on every consumer.
// ============================================================

#[test]
fn mismatched_units_fail_at_parse_before_anything_runs() {
    let tw = tw_outcome(MISMATCHED);
    let msg = tw
        .error
        .expect("volts vs kilograms is analytically meaningless — a parse error");
    assert!(
        msg.contains("volt") && msg.contains("kilogram"),
        "the error names both units so the fix is obvious: {msg}"
    );
    assert!(vm_outcome(MISMATCHED).error.is_some());
    assert!(compile_to_analog(MISMATCHED).is_err());
}

// ============================================================
//  5. Inverted ranges: typed ContradictoryContract refusal.
// ============================================================

#[test]
fn an_inverted_range_is_a_contradictory_contract_refusal_bare_and_dimensioned() {
    for (name, src) in [
        ("dimensioned", INVERTED_DIMENSIONED),
        ("bare", INVERTED_BARE),
    ] {
        let err = compile_to_analog(src).expect_err("an empty range can never certify");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("Contradictory") || msg.contains("contradictory"),
            "{name}: the refusal must be the TYPED ContradictoryContract, got: {msg}"
        );
    }
    // The runtime twin semantics are unchanged: the impossible assert simply fires.
    assert!(tw_outcome(INVERTED_BARE).error.is_some());
    assert!(tw_outcome(INVERTED_DIMENSIONED).error.is_some());
}

// ============================================================
//  6. The fold never reaches general conditions.
// ============================================================

#[test]
fn quantity_variables_keep_their_comparison_semantics_in_general_conditions() {
    let out = tw_outcome(QUANTITY_IF);
    assert_eq!(
        out.error, None,
        "Quantity↔Quantity `between` in an If must keep working: {out:?}"
    );
    assert_eq!(out.output.trim(), "1");
    let vm = vm_outcome(QUANTITY_IF);
    assert_eq!(vm.error, None, "{vm:?}");
    assert_eq!(vm.output.trim(), "1");
}

// ============================================================
//  The artifact: byte-identical to the bare twin.
// ============================================================

#[test]
fn the_analog_artifact_is_identical_to_the_bare_twins() {
    let d = compile_to_analog(DIMENSIONED).expect("the dimensioned program lowers");
    let b = compile_to_analog(BARE_TWIN).expect("the bare twin lowers");
    assert_eq!(d.netlist, b.netlist);
    assert_eq!(d.schedule, b.schedule);
    assert_eq!(d.cert, b.cert);
    assert_eq!(d.budget, b.budget);
    assert_eq!(d.placement, b.placement);
    assert_eq!(
        d.spice, b.spice,
        "dimension is metadata in this phase — the artifact bits do not move"
    );
}
