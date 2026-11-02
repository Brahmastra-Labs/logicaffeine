//! A counter CRDT compares and equals as its Int total, on every tier.
//!
//! A `Tally` (a `PNCounter`) or `ConvergentCount` (a `GCounter`) is a per-replica CRDT; comparison and
//! equality see its total, as the interpreter projects it (`semantics::compare::counter_as_int`). The
//! compiled tier emitted a raw `>` on two `PNCounter`s, which has no ordering (E0369) — the Studio's
//! leaderboard example. A counter operand of a comparison or an equality now reads `.value()`; as an
//! argument, a field initializer or a `Merge` it stays a counter. Each program must build and print
//! exactly what the interpreter prints.

mod common;
use common::{assert_all_tiers, assert_compiled_equals_interpreted};

fn on_every_tier(src: &str, expected: &str) {
    assert_all_tiers(src, expected);
    assert_compiled_equals_interpreted(src);
}

const BOARD: &str =
    "## Definition\nA Board is Shared and has:\n    alice: Tally.\n    bob: Tally.\n\n";

#[test]
fn two_counters_compare_by_their_totals() {
    let src = format!(
        "{BOARD}## Main\nLet mutable b be a new Board.\nIncrease b's alice by 12.\nIncrease b's bob by 7.\nIf b's alice > b's bob:\n    Show \"alice leads\".\nIf b's bob < b's alice:\n    Show \"bob trails\".\nIf b's alice is at least b's bob:\n    Show \"alice at least bob\".\n"
    );
    on_every_tier(&src, "alice leads\nbob trails\nalice at least bob");
}

#[test]
fn a_counter_compares_with_an_int_on_either_side() {
    let src = format!(
        "{BOARD}## Main\nLet mutable b be a new Board.\nIncrease b's alice by 19.\nIf b's alice is at least 10:\n    Show \"at least ten\".\nIf 20 > b's alice:\n    Show \"below twenty\".\nIf b's alice <= 19:\n    Show \"at most nineteen\".\n"
    );
    on_every_tier(&src, "at least ten\nbelow twenty\nat most nineteen");
}

#[test]
fn a_counter_equals_its_total() {
    let src = format!(
        "{BOARD}## Main\nLet mutable b be a new Board.\nIncrease b's alice by 5.\nDecrease b's alice by 2.\nIf b's alice equals 3:\n    Show \"three\".\nIf b's alice is not 4:\n    Show \"not four\".\nIncrease b's bob by 3.\nIf b's alice equals b's bob:\n    Show \"tied\".\n"
    );
    on_every_tier(&src, "three\nnot four\ntied");
}

/// The Studio's leaderboard: three replicas merge, then the totals are compared.
#[test]
fn merged_replicas_compare_by_their_merged_totals() {
    let src = format!(
        "{BOARD}## Main\nLet mutable phone be a new Board.\nIncrease phone's alice by 12.\nIncrease phone's bob by 7.\n\nLet mutable laptop be a new Board.\nIncrease laptop's alice by 5.\nIncrease laptop's bob by 1.\n\nLet mutable tablet be a new Board.\nIncrease tablet's alice by 2.\nIncrease tablet's bob by 10.\nDecrease tablet's bob by 2.\n\nMerge laptop into phone.\nMerge tablet into phone.\n\nShow \"alice: {{phone's alice}}\".\nShow \"bob: {{phone's bob}}\".\nIf phone's alice > phone's bob:\n    Show \"alice leads\".\nOtherwise:\n    Show \"bob leads\".\n"
    );
    on_every_tier(&src, "alice: 19\nbob: 16\nalice leads");
}

/// A counter handed to a `Tally` parameter stays a counter, and compares there by its total.
#[test]
fn a_counter_parameter_compares_by_its_total() {
    let src = format!(
        "{BOARD}## To report (t: Tally):\n    If t > 3:\n        Show \"big\".\n    Show t.\n\n## Main\nLet mutable b be a new Board.\nIncrease b's alice by 4.\nreport(b's alice).\nShow \"{{b's alice}}\".\n"
    );
    on_every_tier(&src, "big\n4\n4");
}
