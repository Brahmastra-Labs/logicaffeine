//! Cross-tier timestamp threading. `parse_timestamp` / `format_timestamp` (RFC 3339, delegating to
//! `base::temporal`) must behave IDENTICALLY on every execution tier — the tree-walker, the bytecode
//! VM, and the AOT-compiled-to-Rust binary. The harnesses below are chosen to FORCE that:
//!
//! - `assert_interpreter_output` runs `interpret_for_ui`, which cross-checks the **VM and the
//!   tree-walker** against each other — so it fails if either tier mishandles the type.
//! - `assert_compiled_equals_interpreted` runs the **AOT binary** and asserts byte-identical output
//!   to the interpreter — so it fails if codegen diverges.
//! - `assert_exact_output` pins the AOT value itself.
//!
//! A timestamp round-trips through a `Moment` and back to text, so the result is a `Text` (which
//! renders identically on every tier), keeping the parity check about the *logic*, not display.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::{assert_compiled_equals_interpreted, assert_exact_output, assert_interpreter_output};

#[cfg(not(target_arch = "wasm32"))]
const ROUNDTRIP: &str =
    "## Main\nShow format_timestamp(parse_timestamp(\"2024-03-10T07:30:00Z\")).";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn timestamp_roundtrip_on_interpreter_vm_and_treewalker() {
    // Forces the VM AND the tree-walker (interpret_for_ui cross-checks the two tiers).
    assert_interpreter_output(ROUNDTRIP, "2024-03-10T07:30:00Z");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn timestamp_roundtrip_on_aot() {
    // Forces the AOT compile-to-Rust tier.
    assert_exact_output(ROUNDTRIP, "2024-03-10T07:30:00Z");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn timestamp_all_tiers_agree() {
    // Forces AOT == interpreter (== VM == tree-walker): one program, every tier, byte-identical.
    assert_compiled_equals_interpreted(ROUNDTRIP);
}

// ---- Calendar component extraction, forced across every tier. 2024-03-10 is a Sunday. ----

#[cfg(not(target_arch = "wasm32"))]
const COMPONENTS: &str = "## Main\nLet m be parse_timestamp(\"2024-03-10T07:30:00Z\").\n\
Show year_of(m).\nShow month_of(m).\nShow day_of(m).\nShow weekday_of(m).";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn date_components_on_interpreter_vm_and_treewalker() {
    // 0 = Sunday. interpret_for_ui cross-checks VM vs tree-walker.
    assert_interpreter_output(COMPONENTS, "2024\n3\n10\n0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn date_components_on_aot() {
    common::assert_output_lines(COMPONENTS, &["2024", "3", "10", "0"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn date_components_all_tiers_agree() {
    assert_compiled_equals_interpreted(COMPONENTS);
}

// ---- Moment arithmetic (seconds_between / add_seconds), forced across every tier. ----

#[cfg(not(target_arch = "wasm32"))]
const ARITH: &str = "## Main\n\
Let a be parse_timestamp(\"2024-03-10T07:30:00Z\").\n\
Let b be parse_timestamp(\"2024-03-10T07:31:00Z\").\n\
Show seconds_between(a, b).\n\
Show format_timestamp(add_seconds(a, 90)).";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn moment_arithmetic_on_interpreter_vm_and_treewalker() {
    assert_interpreter_output(ARITH, "60\n2024-03-10T07:31:30Z");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn moment_arithmetic_on_aot() {
    common::assert_output_lines(ARITH, &["60", "2024-03-10T07:31:30Z"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn moment_arithmetic_all_tiers_agree() {
    assert_compiled_equals_interpreted(ARITH);
}

// ---- NATURAL sentence syntax: `the year of <moment>` (not `year_of(m)`), forced across tiers. ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL: &str = "## Main\nLet m be parse_timestamp(\"2024-03-10T07:30:00Z\").\n\
Show the year of m.\nShow the month of m.\nShow the day of m.\nShow the weekday of m.";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_date_components_on_interpreter_vm_and_treewalker() {
    assert_interpreter_output(NATURAL, "2024\n3\n10\n0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_date_components_on_aot() {
    common::assert_output_lines(NATURAL, &["2024", "3", "10", "0"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_date_components_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL);
}

// ---- NATURAL elapsed time: `the seconds between a and b` (not `seconds_between(a, b)`). ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_BETWEEN: &str = "## Main\nLet a be parse_timestamp(\"2024-03-10T07:30:00Z\").\n\
Let b be parse_timestamp(\"2024-03-10T07:31:00Z\").\n\
Show the seconds between a and b.\nShow the minutes between a and b.";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_elapsed_on_interpreter_vm_and_treewalker() {
    assert_interpreter_output(NATURAL_BETWEEN, "60\n1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_elapsed_on_aot() {
    common::assert_output_lines(NATURAL_BETWEEN, &["60", "1"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_elapsed_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL_BETWEEN);
}

// ---- NATURAL Moment + duration: `a + 90 seconds` (natural CalendarUnit literal + operator). ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_PLUS: &str = "## Main\nLet a be parse_timestamp(\"2024-03-10T07:30:00Z\").\n\
Show format_timestamp(a + 90 seconds).\nShow format_timestamp(a - 30 seconds).";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_moment_plus_duration_on_interpreter_vm_and_treewalker() {
    assert_interpreter_output(NATURAL_PLUS, "2024-03-10T07:31:30Z\n2024-03-10T07:29:30Z");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_moment_plus_duration_on_aot() {
    common::assert_output_lines(NATURAL_PLUS, &["2024-03-10T07:31:30Z", "2024-03-10T07:29:30Z"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_moment_plus_duration_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL_PLUS);
}

// ---- NATURAL construction: `timestamp "…"` literal (not parse_timestamp("…")). ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_LITERAL: &str =
    "## Main\nShow the year of timestamp \"2024-03-10T07:30:00Z\".\n\
Show format_timestamp(timestamp \"2024-03-10T07:30:00Z\" + 90 seconds).";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_timestamp_literal_on_interpreter_vm_and_treewalker() {
    assert_interpreter_output(NATURAL_LITERAL, "2024\n2024-03-10T07:31:30Z");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_timestamp_literal_on_aot() {
    common::assert_output_lines(NATURAL_LITERAL, &["2024", "2024-03-10T07:31:30Z"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_timestamp_literal_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL_LITERAL);
}

// ---- NATURAL zoned time: `<moment> in "America/New_York"` (timezone-aware relative read). ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_ZONE: &str = "## Main\n\
Show timestamp \"2024-07-01T12:00:00Z\" in \"America/New_York\".\n\
Show timestamp \"2024-01-01T12:00:00Z\" in \"America/New_York\".\n\
Show timestamp \"2024-01-01T00:00:00Z\" in \"Asia/Kolkata\".";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_zoned_time_on_interpreter_vm_and_treewalker() {
    // Summer EDT (−4), winter EST (−5), India +5:30.
    assert_interpreter_output(
        NATURAL_ZONE,
        "2024-07-01T08:00:00-04:00\n2024-01-01T07:00:00-05:00\n2024-01-01T05:30:00+05:30",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_zoned_time_on_aot() {
    common::assert_output_lines(
        NATURAL_ZONE,
        &["2024-07-01T08:00:00-04:00", "2024-01-01T07:00:00-05:00", "2024-01-01T05:30:00+05:30"],
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_zoned_time_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL_ZONE);
}

// ---- NATURAL time-of-day components: `the hour/minute/second of <moment>`. ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_HMS: &str = "## Main\nLet m be timestamp \"2024-03-10T07:30:45Z\".\n\
Show the hour of m.\nShow the minute of m.\nShow the second of m.";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_time_components_on_interpreter_vm_and_treewalker() {
    assert_interpreter_output(NATURAL_HMS, "7\n30\n45");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_time_components_on_aot() {
    common::assert_output_lines(NATURAL_HMS, &["7", "30", "45"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_time_components_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL_HMS);
}

// ---- NATURAL calendar arithmetic: `<moment> + 1 month` is CIVIL (end-of-month clamp, leap-year
//      correct, time-of-day preserved) — distinct from the physical `+ 90 seconds` Duration path. ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_SPAN: &str = "## Main\n\
Show format_timestamp(timestamp \"2024-01-31T12:30:00Z\" + 1 month).\n\
Show format_timestamp(timestamp \"2024-02-29T12:30:00Z\" + 1 year).\n\
Show format_timestamp(timestamp \"2024-01-31T12:30:00Z\" - 1 month).";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_moment_plus_span_on_interpreter_vm_and_treewalker() {
    // Jan 31 + 1 month → Feb 29 (2024 is a leap year, clamp); Feb 29 + 1 year → Feb 28 2025 (clamp);
    // Jan 31 − 1 month → Dec 31 2023. The 12:30:00 wall time rides along untouched.
    assert_interpreter_output(
        NATURAL_SPAN,
        "2024-02-29T12:30:00Z\n2025-02-28T12:30:00Z\n2023-12-31T12:30:00Z",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_moment_plus_span_on_aot() {
    common::assert_output_lines(
        NATURAL_SPAN,
        &["2024-02-29T12:30:00Z", "2025-02-28T12:30:00Z", "2023-12-31T12:30:00Z"],
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_moment_plus_span_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL_SPAN);
}

// ---- NATURAL calendar quarter: `the quarter of <moment>` (1..=4). March → Q1, July → Q3. ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_QUARTER_OF: &str = "## Main\n\
Show the quarter of 2024-03-10.\nShow the quarter of 2024-07-01.\n\
Show the quarter of timestamp \"2024-11-30T00:00:00Z\".";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_quarter_of_on_interpreter_vm_and_treewalker() {
    assert_interpreter_output(NATURAL_QUARTER_OF, "1\n3\n4");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_quarter_of_on_aot() {
    common::assert_output_lines(NATURAL_QUARTER_OF, &["1", "3", "4"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_quarter_of_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL_QUARTER_OF);
}

// ---- NATURAL ISO-8601 week number: `the week of <moment>` (1..=53). 2024-03-10 is the Sunday of
//      ISO week 10 (week 1 starts Mon 2024-01-01). Works on a Date literal too. ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_WEEK_OF: &str = "## Main\nLet m be timestamp \"2024-03-10T07:30:45Z\".\n\
Show the week of m.\nShow the week of 2024-01-01.";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_week_of_on_interpreter_vm_and_treewalker() {
    assert_interpreter_output(NATURAL_WEEK_OF, "10\n1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_week_of_on_aot() {
    common::assert_output_lines(NATURAL_WEEK_OF, &["10", "1"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_week_of_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL_WEEK_OF);
}

// ---- NATURAL date extraction: `the date of <moment>` is the calendar day (a Date), distinct from
//      `the day of <moment>` which is the day-of-month number. Renders YYYY-MM-DD on every tier. ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_DATE_OF: &str = "## Main\nLet m be timestamp \"2024-03-10T07:30:45Z\".\n\
Show the date of m.";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_date_of_on_interpreter_vm_and_treewalker() {
    assert_interpreter_output(NATURAL_DATE_OF, "2024-03-10");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_date_of_on_aot() {
    common::assert_output_lines(NATURAL_DATE_OF, &["2024-03-10"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_date_of_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL_DATE_OF);
}

// ---- NATURAL time-of-day extraction: `the time of <moment>` is the wall-clock time (a Time),
//      lossless to the second — `07:30:45`, not the lossy `07:30`. ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_TIME_OF: &str = "## Main\nLet m be timestamp \"2024-03-10T07:30:45Z\".\n\
Show the time of m.";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_time_of_on_interpreter_vm_and_treewalker() {
    assert_interpreter_output(NATURAL_TIME_OF, "07:30:45");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_time_of_on_aot() {
    common::assert_output_lines(NATURAL_TIME_OF, &["07:30:45"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_time_of_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL_TIME_OF);
}

// ---- NATURAL temporal comparison: `a is before b` / `a is after b` reads as a sentence and
//      desugars to the `<` / `>` the tiers already order Moments by. ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_CMP: &str = "## Main\n\
Let a be timestamp \"2024-01-01T00:00:00Z\".\n\
Let b be timestamp \"2024-06-01T00:00:00Z\".\n\
Show a is before b.\nShow a is after b.\nShow b is after a.";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_temporal_comparison_on_interpreter_vm_and_treewalker() {
    assert_interpreter_output(NATURAL_CMP, "true\nfalse\ntrue");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_temporal_comparison_on_aot() {
    common::assert_output_lines(NATURAL_CMP, &["true", "false", "true"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_temporal_comparison_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL_CMP);
}

// ---- NATURAL zoned components: `the <comp> of <m> in "<zone>"` reads the LOCAL component, not UTC.
//      The killer case: a UTC instant whose local date rolls to the previous day. ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_ZONED_COMPONENT: &str = "## Main\n\
Show the hour of timestamp \"2024-07-01T12:00:00Z\" in \"America/New_York\".\n\
Show the hour of timestamp \"2024-01-01T00:00:00Z\" in \"Asia/Kolkata\".\n\
Show the minute of timestamp \"2024-01-01T00:00:00Z\" in \"Asia/Kolkata\".\n\
Show the day of timestamp \"2024-07-01T02:00:00Z\" in \"America/New_York\".\n\
Show the hour of timestamp \"2024-07-01T02:00:00Z\" in \"America/New_York\".";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_zoned_component_on_interpreter_vm_and_treewalker() {
    // NY 12:00Z=08 EDT; Kolkata 00:00Z=05:30 IST; NY 02:00Z rolls to 06-30 22:00 local.
    assert_interpreter_output(NATURAL_ZONED_COMPONENT, "8\n5\n30\n30\n22");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_zoned_component_on_aot() {
    common::assert_output_lines(NATURAL_ZONED_COMPONENT, &["8", "5", "30", "30", "22"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_zoned_component_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL_ZONED_COMPONENT);
}

// ---- The zoned read composes onto EVERY extractor (date/time/weekday/year), not just hour. A
//      rollover to the previous local day even flips the weekday (UTC Mon → local Sun). ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_ZONED_ALL: &str = "## Main\nLet m be timestamp \"2024-07-01T02:00:00Z\".\n\
Show the date of m in \"America/New_York\".\nShow the time of m in \"America/New_York\".\n\
Show the weekday of m in \"America/New_York\".\nShow the weekday of m.\n\
Show the year of m in \"America/New_York\".";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_zoned_all_extractors_on_interpreter_vm_and_treewalker() {
    // Local = 2024-06-30T22:00 EDT: date 06-30, time 22:00:00, weekday Sun(0) vs UTC Mon(1).
    assert_interpreter_output(NATURAL_ZONED_ALL, "2024-06-30\n22:00:00\n0\n1\n2024");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_zoned_all_extractors_on_aot() {
    common::assert_output_lines(NATURAL_ZONED_ALL, &["2024-06-30", "22:00:00", "0", "1", "2024"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_zoned_all_extractors_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL_ZONED_ALL);
}

// ---- EXHAUSTIVE elapsed units: every `the <unit> between a and b` form. b is 30h after a. ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_BETWEEN_ALL: &str = "## Main\n\
Let a be timestamp \"2024-03-10T00:00:00Z\".\nLet b be timestamp \"2024-03-11T06:00:00Z\".\n\
Show the seconds between a and b.\nShow the minutes between a and b.\n\
Show the hours between a and b.\nShow the days between a and b.";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_between_all_units_on_interpreter_vm_and_treewalker() {
    // 30h = 108000s = 1800m = 30h = 1 day (truncated).
    assert_interpreter_output(NATURAL_BETWEEN_ALL, "108000\n1800\n30\n1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_between_all_units_on_aot() {
    common::assert_output_lines(NATURAL_BETWEEN_ALL, &["108000", "1800", "30", "1"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_between_all_units_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL_BETWEEN_ALL);
}

// ---- CALENDAR-CORRECT elapsed: `the months/years between a and b` count COMPLETE calendar periods
//      (not fixed-second division); `the weeks between` is the fixed 7-day form. ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_CAL_BETWEEN: &str = "## Main\n\
Let a be timestamp \"2020-06-01T00:00:00Z\".\nLet b be timestamp \"2024-03-01T00:00:00Z\".\n\
Show the months between a and b.\nShow the years between a and b.\n\
Show the weeks between timestamp \"2024-01-01T00:00:00Z\" and timestamp \"2024-01-29T00:00:00Z\".";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_calendar_between_on_interpreter_vm_and_treewalker() {
    // 2020-06 → 2024-03 = 45 complete months = 3 complete years; 28 days = 4 weeks.
    assert_interpreter_output(NATURAL_CAL_BETWEEN, "45\n3\n4");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_calendar_between_on_aot() {
    common::assert_output_lines(NATURAL_CAL_BETWEEN, &["45", "3", "4"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_calendar_between_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL_CAL_BETWEEN);
}

// ---- PRE-EPOCH: a Moment before 1970 must decompose by FLOOR division (no negative hour/second)
//      and round-trip through format, identically on every tier. ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_PRE_EPOCH: &str = "## Main\nLet m be timestamp \"1969-12-31T23:59:58Z\".\n\
Show the year of m.\nShow the month of m.\nShow the day of m.\nShow the hour of m.\n\
Show the second of m.\nShow format_timestamp(m).\nShow format_timestamp(m + 3 seconds).";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_pre_epoch_on_interpreter_vm_and_treewalker() {
    // 23:59:58 + 3s rolls past midnight into 1970-01-01T00:00:01Z.
    assert_interpreter_output(
        NATURAL_PRE_EPOCH,
        "1969\n12\n31\n23\n58\n1969-12-31T23:59:58Z\n1970-01-01T00:00:01Z",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_pre_epoch_on_aot() {
    common::assert_output_lines(
        NATURAL_PRE_EPOCH,
        &["1969", "12", "31", "23", "58", "1969-12-31T23:59:58Z", "1970-01-01T00:00:01Z"],
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_pre_epoch_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL_PRE_EPOCH);
}

// ---- EDGE CASES: the gnarly calendar boundaries, forced across every tier. ISO week 1 can belong
//      to the *previous* ISO year (2021-01-01 → 2020-W53); quarter flips at month boundaries. ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_EDGES: &str = "## Main\n\
Show the week of 2021-01-01.\nShow the week of 2024-12-30.\n\
Show the quarter of 2024-01-01.\nShow the quarter of 2024-03-31.\n\
Show the quarter of 2024-04-01.\nShow the quarter of 2024-12-31.";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_calendar_edges_on_interpreter_vm_and_treewalker() {
    // 2021-01-01 is ISO 2020-W53; 2024-12-30 (Mon) starts ISO 2025-W01. Quarters: Q1,Q1,Q2,Q4.
    assert_interpreter_output(NATURAL_EDGES, "53\n1\n1\n1\n2\n4");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_calendar_edges_on_aot() {
    common::assert_output_lines(NATURAL_EDGES, &["53", "1", "1", "1", "2", "4"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_calendar_edges_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL_EDGES);
}

// ---- EXHAUSTIVE: every natural-time surface in ONE program, forced byte-identical across all three
//      tiers. If ANY accessor/operator/comparison regresses on ANY tier, this fails. ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_KITCHEN_SINK: &str = "## Main\n\
Let m be timestamp \"2024-03-10T07:30:45Z\".\n\
Show the year of m.\nShow the month of m.\nShow the day of m.\nShow the weekday of m.\n\
Show the hour of m.\nShow the minute of m.\nShow the second of m.\n\
Show the week of m.\nShow the quarter of m.\n\
Show the date of m.\nShow the time of m.\n\
Show format_timestamp(m + 1 month).\nShow format_timestamp(m - 90 seconds).\n\
Show the seconds between m and (m + 1 month).\n\
Show m is before (m + 1 month).\nShow m is after (m + 1 month).\n\
Show m in \"America/New_York\".";

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_KITCHEN_SINK_EXPECTED: &str = "2024\n3\n10\n0\n7\n30\n45\n10\n1\n\
2024-03-10\n07:30:45\n2024-04-10T07:30:45Z\n2024-03-10T07:29:15Z\n2678400\ntrue\nfalse\n\
2024-03-10T03:30:45-04:00";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_time_kitchen_sink_on_interpreter_vm_and_treewalker() {
    assert_interpreter_output(NATURAL_KITCHEN_SINK, NATURAL_KITCHEN_SINK_EXPECTED);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_time_kitchen_sink_on_aot() {
    let expected: Vec<&str> = NATURAL_KITCHEN_SINK_EXPECTED.split('\n').collect();
    common::assert_output_lines(NATURAL_KITCHEN_SINK, &expected);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_time_kitchen_sink_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL_KITCHEN_SINK);
}

// ---- COLLECTIONS: a `Seq of Moment` must store Moments and let accessors read elements back — the
//      generic type must thread (`LogosSeq<LogosMoment>`) on every tier. ----

#[cfg(not(target_arch = "wasm32"))]
const SEQ_OF_MOMENT: &str = "## Main\n\
Let ts be a new Seq of Moment.\n\
Push timestamp \"2024-03-10T07:30:45Z\" to ts.\n\
Push timestamp \"2025-01-01T00:00:00Z\" to ts.\n\
Show the year of item 1 of ts.\nShow the year of item 2 of ts.\n\
Show the hour of item 1 of ts.";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn seq_of_moment_on_interpreter_vm_and_treewalker() {
    assert_interpreter_output(SEQ_OF_MOMENT, "2024\n2025\n7");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn seq_of_moment_on_aot() {
    common::assert_output_lines(SEQ_OF_MOMENT, &["2024", "2025", "7"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn seq_of_moment_all_tiers_agree() {
    assert_compiled_equals_interpreted(SEQ_OF_MOMENT);
}

// ---- `Return <moment> in "<zone>"`: the zoned postfix must work in a Return position too. ----

#[cfg(not(target_arch = "wasm32"))]
const RETURN_ZONED: &str = "## To local (m: Moment) -> Text:\n\
\x20   Return m in \"America/New_York\".\n\
## Main\n\
Show local(timestamp \"2024-07-01T12:00:00Z\").";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn return_zoned_on_interpreter_vm_and_treewalker() {
    assert_interpreter_output(RETURN_ZONED, "2024-07-01T08:00:00-04:00");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn return_zoned_on_aot() {
    common::assert_output_lines(RETURN_ZONED, &["2024-07-01T08:00:00-04:00"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn return_zoned_all_tiers_agree() {
    assert_compiled_equals_interpreted(RETURN_ZONED);
}

// ---- TYPE-SYSTEM CERTAINTY: temporal values must thread through every syntactic position, not just
//      `Show` — a function param + return, an `If` condition (accessor AND comparison), and a `Let`
//      whose Date value is read downstream. If type inference/codegen drops the type anywhere on any
//      tier, this fails. ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_IN_CONTEXT: &str = "## To season_quarter (m: Moment) -> Int:\n\
\x20   Return the quarter of m.\n\
## Main\n\
Let m be timestamp \"2024-07-15T12:00:00Z\".\n\
Let q be season_quarter(m).\nShow q.\n\
If the year of m is greater than 2000:\n\
\x20   Show \"modern\".\n\
If m is before timestamp \"2025-01-01T00:00:00Z\":\n\
\x20   Show \"before 2025\".\n\
Let d be the date of m.\nShow the month of d.";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_time_in_program_contexts_on_interpreter_vm_and_treewalker() {
    // Q3, year>2000, before 2025, month of the extracted Date = 7.
    assert_interpreter_output(NATURAL_IN_CONTEXT, "3\nmodern\nbefore 2025\n7");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_time_in_program_contexts_on_aot() {
    common::assert_output_lines(NATURAL_IN_CONTEXT, &["3", "modern", "before 2025", "7"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_time_in_program_contexts_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL_IN_CONTEXT);
}

// ---- The same accessors on a bare Date literal `2024-03-10` (no quotes). ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_DATE: &str = "## Main\n\
Show the year of 2024-03-10.\nShow the month of 2024-03-10.\n\
Show the day of 2024-03-10.\nShow the weekday of 2024-03-10.";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_date_literal_components_on_interpreter_vm_and_treewalker() {
    assert_interpreter_output(NATURAL_DATE, "2024\n3\n10\n0"); // 2024-03-10 is a Sunday
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_date_literal_components_on_aot() {
    common::assert_output_lines(NATURAL_DATE, &["2024", "3", "10", "0"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_date_literal_components_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL_DATE);
}
