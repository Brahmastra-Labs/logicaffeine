//! `Date + Span` and `Date - Span` on every tier.
//!
//! Calendar arithmetic applies the span's months first, clamped to the end of the month
//! (`Jan 31 + 1 month = Feb 28/29`), then its days. The tree-walker and the VM compute it with
//! `base::temporal::date_add_span`; the compiled tier's `LogosDate` implements `Add`/`Sub` of a
//! `LogosSpan` through the same function, so the arithmetic exists once. Each program must print the
//! same on the tree-walker, the VM, AOT-Rust and C.

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

fn on_every_tier(src: &str, expected: &str) {
    assert_all_tiers(src, expected);
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, expected);
}

#[test]
fn a_span_of_days_moves_a_date_both_ways() {
    on_every_tier(
        "## Main\nLet liftoff be 2026-05-20.\nLet prep be liftoff - 3 days.\nLet after be liftoff + 12 days.\nShow prep.\nShow after.\n",
        "2026-05-17\n2026-06-01",
    );
}

#[test]
fn months_clamp_to_the_end_of_the_month() {
    on_every_tier(
        "## Main\nShow 2026-01-31 + 1 month.\nShow 2024-01-31 + 1 month.\nShow 2026-03-31 - 1 month.\nShow 2026-05-31 + 1 month.\n",
        "2026-02-28\n2024-02-29\n2026-02-28\n2026-06-30",
    );
}

#[test]
fn months_roll_over_the_year() {
    on_every_tier(
        "## Main\nShow 2026-12-15 + 1 month.\nShow 2026-01-10 - 2 months.\nShow 2026-05-20 + 2 months.\n",
        "2027-01-15\n2025-11-10\n2026-07-20",
    );
}

#[test]
fn a_compound_span_applies_months_then_days() {
    on_every_tier(
        "## Main\nShow 2026-01-10 + 1 month and 5 days.\nLet sprint be 2 weeks and 3 days.\nShow 2026-05-01 + sprint.\nShow 2026-03-01 - sprint.\n",
        "2026-02-15\n2026-05-18\n2026-02-12",
    );
}

#[test]
fn arithmetic_crosses_the_epoch() {
    on_every_tier(
        "## Main\nShow 1970-01-01 - 1 day.\nShow 1969-12-31 + 1 day.\nShow 1969-03-31 - 1 month.\n",
        "1969-12-31\n1970-01-01\n1969-02-28",
    );
}

#[test]
fn a_shifted_date_compares_as_a_date() {
    on_every_tier(
        "## Main\nLet liftoff be 2026-05-20.\nIf liftoff - 3 days < liftoff:\n    Show \"earlier\".\nIf 2026-01-31 + 1 month equals 2026-02-28:\n    Show \"clamped\".\n",
        "earlier\nclamped",
    );
}
