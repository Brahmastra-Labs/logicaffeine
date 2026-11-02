//! `the weekday of` / `BuiltinId::WeekdayOf` — ISO 8601 (Monday=1 .. Sunday=7) on every tier
//! (a language decision). A full calendar week PLUS
//! the Unix epoch (1970-01-01, a Thursday — `days = 0`, the anchor every tier's formula is built
//! from) pinned across tree-walker, VM, AOT-Rust and C. `dayName`/`dayAbbr` follow the same
//! numbering (`parity_calendar.rs`); the fabric/RTL side is the differential, no-hardcoded-literal
//! `fabric_date_lock.rs`/`fabric_coverage_lock.rs`, which self-validate once
//! `codegen_sva/fabric_rtl.rs`'s two `WeekdayOf` lowering sites agree with the VM.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// 1970-01-01 (the epoch) is a Thursday; 2024-03-11..17 is one full Monday-to-Sunday week
/// (2024-03-10, a day earlier, is independently pinned as a Sunday elsewhere — `parity_date_ctor.rs`,
/// `e2e_codegen_timestamp.rs`). ISO: Monday=1 .. Sunday=7.
const DATE_SRC: &str = "## Main\n\
    Show the weekday of makeDate(1970, 1, 1).\n\
    Show the weekday of makeDate(2024, 3, 11).\n\
    Show the weekday of makeDate(2024, 3, 12).\n\
    Show the weekday of makeDate(2024, 3, 13).\n\
    Show the weekday of makeDate(2024, 3, 14).\n\
    Show the weekday of makeDate(2024, 3, 15).\n\
    Show the weekday of makeDate(2024, 3, 16).\n\
    Show the weekday of makeDate(2024, 3, 17).\n";
const DATE_EXPECTED: &str = "4\n1\n2\n3\n4\n5\n6\n7";

#[test]
fn weekday_of_a_date_is_iso_8601_on_tree_walker_and_vm() {
    assert_all_tiers(DATE_SRC, DATE_EXPECTED);
}

#[test]
fn weekday_of_a_date_is_iso_8601_on_aot_rust() {
    assert_compiled_equals_interpreted(DATE_SRC);
}

#[test]
fn weekday_of_a_date_is_iso_8601_on_c() {
    assert_c_output(DATE_SRC, DATE_EXPECTED);
}

/// The SAME week, through the `Moment` arm (`WeekdayOf` on a Moment and on a Date are two
/// independent match arms on every tier — `semantics/builtins.rs`, `codegen_c/emit.rs`'s Date-vs-
/// Moment nanos scaling, `codegen_sva/fabric_rtl.rs`'s `TimeKind::Moment | TimeKind::Date` split —
/// so an epoch/week check confined to `Date` cannot see an arm-specific regression in `Moment`).
const MOMENT_SRC: &str = "## Main\n\
    Show the weekday of makeMoment(1970, 1, 1, 0, 0, 0).\n\
    Show the weekday of makeMoment(2024, 3, 11, 23, 59, 59).\n\
    Show the weekday of makeMoment(2024, 3, 12, 0, 0, 0).\n\
    Show the weekday of makeMoment(2024, 3, 13, 12, 0, 0).\n\
    Show the weekday of makeMoment(2024, 3, 14, 6, 30, 0).\n\
    Show the weekday of makeMoment(2024, 3, 15, 18, 0, 0).\n\
    Show the weekday of makeMoment(2024, 3, 16, 3, 0, 0).\n\
    Show the weekday of makeMoment(2024, 3, 17, 23, 59, 59).\n";
const MOMENT_EXPECTED: &str = "4\n1\n2\n3\n4\n5\n6\n7";

#[test]
fn weekday_of_a_moment_is_iso_8601_on_tree_walker_and_vm() {
    assert_all_tiers(MOMENT_SRC, MOMENT_EXPECTED);
}

#[test]
fn weekday_of_a_moment_is_iso_8601_on_aot_rust() {
    assert_compiled_equals_interpreted(MOMENT_SRC);
}

#[test]
fn weekday_of_a_moment_is_iso_8601_on_c() {
    assert_c_output(MOMENT_SRC, MOMENT_EXPECTED);
}

/// `dayName`/`dayAbbr` compose with `the weekday of` under the SAME ISO numbering across the whole
/// week (the per-value cases already live in `parity_calendar.rs`; this is the cross-tier,
/// full-week version of the same "convention unification" property).
const NAMED_SRC: &str = "## Main\n\
    Show dayName(the weekday of makeDate(2024, 3, 11)).\n\
    Show dayName(the weekday of makeDate(2024, 3, 12)).\n\
    Show dayName(the weekday of makeDate(2024, 3, 13)).\n\
    Show dayName(the weekday of makeDate(2024, 3, 14)).\n\
    Show dayName(the weekday of makeDate(2024, 3, 15)).\n\
    Show dayName(the weekday of makeDate(2024, 3, 16)).\n\
    Show dayName(the weekday of makeDate(2024, 3, 17)).\n\
    Show dayAbbr(the weekday of makeDate(1970, 1, 1)).\n";
const NAMED_EXPECTED: &str = "Monday\nTuesday\nWednesday\nThursday\nFriday\nSaturday\nSunday\nThu";

#[test]
fn day_name_and_abbr_compose_with_weekday_of_across_the_week() {
    assert_all_tiers(NAMED_SRC, NAMED_EXPECTED);
    assert_compiled_equals_interpreted(NAMED_SRC);
    assert_c_output(NAMED_SRC, NAMED_EXPECTED);
}
