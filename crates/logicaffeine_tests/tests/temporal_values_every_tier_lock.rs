//! ★ LOCK — the language's temporal values print the tree-walker's bytes on every backend. ★
//!
//! A date, an instant, a time of day, a calendar span and a physical duration, and the clock reads `today` and `now`:
//! their display, their arithmetic either side of the operator where the language takes both sides (a duration or a span
//! plus an instant, an instant minus either), their ordering (a time of day against an instant by its time of day),
//! the calendar builtins, a sleep for a duration or for milliseconds (none at or below zero), and each value through
//! records, lists, calls and text. A duration is signed (`0ns - 5ns` is `-5ns`). Every backend reads the one instant
//! `tier_parity` pins, 2024-10-04 12:34:56: the engines through the thread's fixed clock, every compiled program through
//! `LOGOS_FIXED_DAYS` and `LOGOS_FIXED_NANOS`. A run a backend does not yet answer as the tree-walker does is listed in
//! [`KNOWN_GAPS`], shrink-only.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test temporal_values_every_tier_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "tier_parity/mod.rs"]
mod tier_parity;

use logicaffeine_compile::compile::tw_outcome;
use tier_parity::{Backend, Program};

/// Each program, and what it prints under the pinned instant.
const PROGRAMS: &[(&str, &str, &str)] = &[
    (
        "the_clock_reads_wherever_they_appear",
        "## A Stamp has:\n    a day: Date.\n    an at: Moment.\n\n## To stamp -> Moment:\n    Return now.\n\n## To day -> Date:\n    Return today.\n\n## Main\nShow today.\nShow now.\nLet d be today.\nShow d.\nShow stamp().\nShow day().\nLet s be a new Stamp with day today and at now.\nShow s's day.\nShow s's at.\nLet days be [today, today + 1 day].\nShow days.\nShow \"on {today} at {now}\".\nShow today + 1 day.\nShow now + 90 seconds.\nShow now is greater than 2024-10-04 at 9am.\nShow format_timestamp(now).\nShow date_of(now) equals today.\n",
        "2024-10-04\n2024-10-04 12:34\n2024-10-04\n2024-10-04 12:34\n2024-10-04\n2024-10-04\n2024-10-04 12:34\n[2024-10-04, 2024-10-05]\non 2024-10-04 at 2024-10-04 12:34\n2024-10-05\n2024-10-04 12:36\ntrue\n2024-10-04T12:34:56Z\ntrue\n",
    ),
    (
        "the_clock_reads_in_every_position",
        "## A Stamped is one of:\n    A Day with value Date.\n    An At with value Moment.\n\n## To later (d: Date) -> Date:\n    Return d + 1 day.\n\n## Main\nLet s be a new Day with value today.\nInspect s:\n    When Day (d): Show d.\n    When At (m): Show m.\nLet t be a new At with value now.\nInspect t:\n    When Day (d): Show d.\n    When At (m): Show m.\nLet mutable days be a new Seq of Date.\nPush today to days.\nPush later(today) to days.\nShow days.\nLet mutable last be 2000-01-01.\nSet last to today.\nShow last.\nLet mutable seen be a new Map of Text to Moment.\nSet item \"start\" of seen to now.\nShow item \"start\" of seen.\nSet item 1 of days to today + 2 days.\nShow item 1 of days.\nLet o be some today.\nShow o.\nLet p be (today, now).\nShow item 1 of p.\nIf today equals 2024-10-04:\n    Show \"pinned\".\n",
        "2024-10-04\n2024-10-04 12:34\n[2024-10-04, 2024-10-05]\n2024-10-04\n2024-10-04 12:34\n2024-10-06\n2024-10-04\n2024-10-04\npinned\n",
    ),
    (
        "durations_show_and_add_signed",
        "## Main\nShow 3 minutes.\nShow 1500 microseconds.\nShow 2 hours + 30 minutes.\nShow 0ns - 5ns.\nShow 90 seconds is less than 2 minutes.\nShow 3 minutes equals 180 seconds.\nShow 250 milliseconds - 1 seconds.\nLet d be 45 seconds.\nShow d + d + d.\nShow 1500 microseconds equals 1 milliseconds + 500 microseconds.\nShow 90 minutes is greater than 1 hours.\nShow 90 minutes equals 5400 seconds.\n",
        "3min\n1ms\n2h\n-5ns\ntrue\ntrue\n-750ms\n2min\ntrue\ntrue\ntrue\n",
    ),
    (
        "durations_through_records_calls_loops_and_lists",
        "## A Job has:\n    a name: Text.\n    a limit: Duration.\n\n## To doubled (d: Duration) -> Duration:\n    Return d + d.\n\n## Main\nLet j be a new Job with name \"build\" and limit 90 seconds.\nShow doubled(j's limit).\nLet mutable total be 0ns.\nRepeat for k in [1500 milliseconds, 0ns - 2 seconds]:\n    Set total to total + k.\nShow total.\nShow total is less than 0ns.\nShow [1 minutes, 2 hours].\n",
        "3min\n-500ms\ntrue\n[1min, 2h]\n",
    ),
    (
        "a_moment_moves_by_durations_and_spans",
        "## Main\nLet m be 2026-11-08 at 9:30am.\nShow m.\nShow m + 90 seconds.\nShow m + 1 month.\nShow m - 2 days.\nShow m - 45 minutes.\nShow m is greater than 2026-11-08 at 9am.\nShow m equals 2026-11-08 at 9:30am.\nShow 2024-01-31 at 11:59pm + 1 month.\nShow m + 1500 milliseconds is greater than m + 1 seconds.\nShow m + 1500 milliseconds equals m + 1 seconds + 500 milliseconds.\n",
        "2026-11-08 09:30\n2026-11-08 09:31\n2026-12-08 09:30\n2026-11-06 09:30\n2026-11-08 08:45\ntrue\ntrue\n2024-02-29 23:59\ntrue\ntrue\n",
    ),
    (
        "a_date_moves_by_spans",
        "## Main\nLet d be 2024-02-29.\nShow d.\nShow d + 1 year.\nShow d - 1 month.\nShow d + 2 years and 3 days.\nShow d is less than 2024-03-01.\nShow d equals 2024-02-29.\nShow the year of d.\nShow month_of(d).\nShow day_of(d + 1 day).\n",
        "2024-02-29\n2025-02-28\n2024-01-29\n2026-03-03\ntrue\ntrue\n2024\n2\n1\n",
    ),
    (
        "a_time_of_day_orders_against_a_moment",
        "## Main\nLet t be 9:30am.\nShow t.\nShow 12:00am.\nShow 11:45pm.\nShow t is less than 10am.\nShow t equals 9:30am.\nShow 2026-11-08 at 9:45am is greater than t.\nShow t is at least 2026-11-08 at 9:30am.\n",
        "09:30:00\n00:00:00\n23:45:00\ntrue\ntrue\ntrue\ntrue\n",
    ),
    (
        "either_side_of_the_operator",
        "## Main\nLet d be 2024-02-29.\nLet m be 2026-11-08 at 9:30am.\nLet t be 9:30am.\nShow 1 month + m.\nShow 90 seconds + m.\nShow m - 90 seconds.\nShow t is less than m.\nShow t is at least m.\nShow m is at most t.\nShow m is greater than 9am.\nShow t equals m.\nShow date_of(d).\nShow date_of(m).\n",
        "2026-12-08 09:30\n2026-11-08 09:31\n2026-11-08 09:28\nfalse\ntrue\ntrue\ntrue\nfalse\n2024-02-29\n2026-11-08\n",
    ),
    (
        "spans_show_in_years_months_and_days",
        "## Main\nShow 2 years and 3 days.\nShow 14 months.\nShow 1 year.\nShow 0 days.\nShow 1 month equals 1 month.\nShow 12 months equals 1 year.\n",
        "2 years and 3 days\n1 year and 2 months\n1 year\n0 days\ntrue\ntrue\n",
    ),
    (
        "the_calendar_builtins",
        "## Main\nLet m be 2024-03-15 at 10:30am + 45 seconds.\nShow m.\nShow format_timestamp(m).\nShow format_timestamp(add_seconds(m, 3600)).\nShow year_of(m).\nShow month_of(m).\nShow day_of(m).\nShow hour_of(m).\nShow minute_of(m).\nShow second_of(m).\nShow weekday_of(m).\nShow week_of(m).\nShow quarter_of(m).\nShow date_of(m).\nShow time_of(m).\nShow year_of(2024-02-29).\nShow weekday_of(date_of(m)).\nShow seconds_between(m, add_seconds(m, 90)).\nShow months_between(2024-01-15 at 12:00am, m).\nShow years_between(2020-03-15 at 12:00am, m).\nShow format_timestamp(m + 123456789ns).\n",
        "2024-03-15 10:30\n2024-03-15T10:30:45Z\n2024-03-15T11:30:45Z\n2024\n3\n15\n10\n30\n45\n5\n11\n1\n2024-03-15\n10:30:45\n2024\n5\n90\n2\n4\n2024-03-15T10:30:45.123456789Z\n",
    ),
    (
        "temporal_values_through_records_lists_and_calls",
        "## A Meeting has:\n    a start: Moment.\n    a length: Duration.\n\n## To endOf (mt: Meeting) -> Moment:\n    Return mt's start + mt's length.\n\n## To nextDay (d: Date) -> Date:\n    Return d + 1 day.\n\n## Main\nLet mt be a new Meeting with start 2026-11-08 at 2pm and length 90 minutes.\nShow endOf(mt).\nShow mt's length.\nLet mutable total be 0ns.\nLet gaps be [10 seconds, 20 seconds, 30 seconds].\nRepeat for g in gaps:\n    Set total to total + g.\nShow total.\nShow nextDay(2026-12-31).\nLet days be [2026-01-01, 2026-06-15].\nShow days.\nShow item 2 of days.\n",
        "2026-11-08 15:30\n1h\n1min\n2027-01-01\n[2026-01-01, 2026-06-15]\n2026-06-15\n",
    ),
    (
        "a_sleep_for_a_duration_or_milliseconds",
        "## Main\nShow 1.\nSleep 2 milliseconds.\nShow 2.\nSleep 1.\nShow 3.\nLet d be 3 milliseconds.\nSleep d.\nShow 4.\nSleep 0ns - 5ns.\nShow 5.\nSleep 0 - 5.\nShow 6.\n",
        "1\n2\n3\n4\n5\n6\n",
    ),
];

/// Every backend that runs a program: the engines, the generated Rust, both C lowerings, both WebAssembly modules
/// and RISC-V.
const BACKENDS: &[Backend] = &[
    Backend::TreeWalker,
    Backend::Vm,
    Backend::VmOptimized,
    Backend::VmForge,
    Backend::Interpret,
    Backend::RustDebug,
    Backend::RustRelease,
    Backend::C,
    Backend::CBytecode,
    Backend::Wasm,
    Backend::WasmLinked,
    Backend::RiscV,
];

/// The runs that differ from the tree-walker today, as (program, backend name), each a gap in that backend's lowering of
/// the temporal values. Shrink-only: a listed run that answers as the tree-walker does fails this lock until its line
/// goes. The self-contained WebAssembly module refuses arithmetic and ordering on temporal values and the calendar
/// builtins that need the linked runtime.
const KNOWN_GAPS: &[(&str, &str)] = &[
    ("the_clock_reads_wherever_they_appear", "wasm"),
    ("a_moment_moves_by_durations_and_spans", "wasm"),
    ("a_date_moves_by_spans", "wasm"),
    ("a_time_of_day_orders_against_a_moment", "wasm"),
    ("either_side_of_the_operator", "wasm"),
    ("the_calendar_builtins", "wasm"),
    ("temporal_values_through_records_lists_and_calls", "wasm"),
];

/// The child side of every isolated backend run. Does nothing unless `LOGOS_PARITY_JOB` is set.
#[test]
#[ignore = "the worker the parity sweep re-executes this binary into"]
fn tier_parity_worker() {
    tier_parity::worker_main();
}

fn programs() -> Vec<Program> {
    PROGRAMS.iter().map(|(id, source, _)| Program::new(format!("temporal_values/{id}"), *source)).collect()
}

#[test]
fn every_program_here_reaches_the_backends() {
    let refused: Vec<String> = programs()
        .into_iter()
        .filter(|p| tier_parity::rejection(&p.source).is_some())
        .map(|p| p.id)
        .collect();
    assert!(refused.is_empty(), "the front end refuses {refused:?}, which pin nothing");
}

#[test]
fn the_tree_walker_prints_what_each_program_says() {
    logicaffeine_compile::semantics::temporal::set_fixed_clock(
        tier_parity::FIXED_DAYS as i32,
        tier_parity::FIXED_NANOS,
    );
    for (id, source, printed) in PROGRAMS {
        let run = tw_outcome(source);
        assert_eq!(run.error, None, "{id}: the tree-walker refused:\n{source}");
        assert_eq!(run.output.trim_end(), printed.trim_end(), "{id}:\n{source}");
    }
}

#[test]
fn every_backend_prints_the_tree_walkers_bytes() {
    let rows = tier_parity::sweep(&programs(), BACKENDS, 4).unwrap_or_else(|why| panic!("the sweep could not start: {why}"));
    let known = |program: &str, backend: &str| {
        KNOWN_GAPS.iter().any(|(p, b)| format!("temporal_values/{p}") == program && *b == backend)
    };
    let unlisted: Vec<String> = rows
        .iter()
        .filter(|row| !row.matches() && !known(&row.program, row.backend.name()))
        .map(|row| format!("{} on {}: {} {}", row.program, row.backend.name(), row.status(), row.detail()))
        .collect();
    assert!(unlisted.is_empty(), "{} runs differ from the tree-walker:\n{}", unlisted.len(), unlisted.join("\n"));
    let answered: Vec<String> = KNOWN_GAPS
        .iter()
        .filter(|(p, b)| {
            rows.iter().any(|row| row.program == format!("temporal_values/{p}") && row.backend.name() == *b && row.matches())
        })
        .map(|(p, b)| format!("({p:?}, {b:?})"))
        .collect();
    assert!(answered.is_empty(), "these runs answer as the tree-walker does now — delete their lines from KNOWN_GAPS:\n{}", answered.join("\n"));
}

#[test]
fn every_known_gap_names_a_program_and_a_backend_of_this_lock() {
    for (program, backend) in KNOWN_GAPS {
        assert!(PROGRAMS.iter().any(|(id, _, _)| id == program), "KNOWN_GAPS names no program `{program}`");
        assert!(BACKENDS.iter().any(|b| b.name() == *backend), "KNOWN_GAPS names no backend `{backend}` of this lock");
    }
}
