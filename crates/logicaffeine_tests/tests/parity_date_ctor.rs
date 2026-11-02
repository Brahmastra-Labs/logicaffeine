//! Date/Moment CONSTRUCTORS — `makeDate(y,m,d)` and `makeMoment(y,m,d,h,mi,s)`.
//!
//! Before this, a `Date`/`Moment` could only enter a program as a literal or via
//! `today`/`now` — the built-in temporal types were half-usable because nothing
//! could build one from computed `(year, month, day, …)` components. These
//! constructors close that gap by routing through the same `logicaffeine_base::
//! temporal` substrate (`days_from_civil` / `unix_nanos_from_civil`) the rest of
//! the temporal family already uses, so a constructed value participates in the
//! WHOLE existing date algebra (`the weekday/year/month/hour of`, `+ 1 month`,
//! `the seconds between`, zones) and cannot drift from `parity_calendar`'s
//! `timegm` lock. Locked byte-identical across every tier via `assert_all_tiers`.
#![allow(unused_imports, dead_code)]
mod common;
use common::assert_all_tiers;

// --- makeDate: construction + ISO display, including the adversarial edges ---

#[test]
fn make_date_shows_iso() {
    assert_all_tiers("## Main\nShow makeDate(2024, 3, 15).\n", "2024-03-15");
}

/// Pre-epoch: days_from_civil(1969,12,31) = -1 (negative day index must display civilly).
#[test]
fn make_date_pre_epoch() {
    assert_all_tiers("## Main\nShow makeDate(1969, 12, 31).\n", "1969-12-31");
}

/// Leap day: only valid because 2000 is a (400-rule) leap year.
#[test]
fn make_date_leap_day() {
    assert_all_tiers("## Main\nShow makeDate(2000, 2, 29).\n", "2000-02-29");
}

// --- makeDate feeds the existing extractor family (the whole point) ---

#[test]
fn make_date_year_month_day_roundtrip() {
    assert_all_tiers(
        "## Main\nLet d be makeDate(2024, 3, 15).\nShow the year of d.\nShow the month of d.\nShow the day of d.\n",
        "2024\n3\n15",
    );
}

/// 2024-03-15 is a Friday; `the weekday of` uses the ISO 8601 Monday=1..Sunday=7 convention, so Friday = 5.
#[test]
fn make_date_weekday() {
    assert_all_tiers(
        "## Main\nLet d be makeDate(2024, 3, 15).\nShow the weekday of d.\n",
        "5",
    );
}

// --- makeMoment: construction + display + extractors ---

#[test]
fn make_moment_shows() {
    assert_all_tiers(
        "## Main\nShow makeMoment(2024, 3, 15, 10, 30, 45).\n",
        "2024-03-15 10:30",
    );
}

#[test]
fn make_moment_time_extractors() {
    assert_all_tiers(
        "## Main\nLet m be makeMoment(2024, 3, 15, 10, 30, 45).\nShow the hour of m.\nShow the minute of m.\nShow the second of m.\n",
        "10\n30\n45",
    );
}

/// The constructor must AGREE with the pre-existing `timegm` lock: the seconds from
/// the epoch to 2000-02-29 12:00:00 UTC is exactly timegm(2000,2,29,12,0,0)=951825600.
#[test]
fn make_moment_agrees_with_timegm() {
    assert_all_tiers(
        "## Main\nLet a be makeMoment(1970, 1, 1, 0, 0, 0).\nLet b be makeMoment(2000, 2, 29, 12, 0, 0).\nShow the seconds between a and b.\n",
        "951825600",
    );
}

/// A constructed Moment supports civil span arithmetic (end-of-month clamp): 2024-01-31 + 1 month → 2024-02-29.
#[test]
fn make_moment_plus_one_month_clamps() {
    assert_all_tiers(
        "## Main\nLet m be makeMoment(2024, 1, 31, 0, 0, 0).\nShow the day of (m + 1 month).\n",
        "29",
    );
}

// --- Compiled tiers: AOT-Rust ≡ interpreter (covers Date + Moment, incl Show + extractors) ---

#[test]
fn make_date_aot_matches_interpreter() {
    common::assert_compiled_equals_interpreted(
        "## Main\nLet d be makeDate(2024, 3, 15).\nShow d.\nShow the year of d.\nShow the month of d.\nShow the day of d.\nShow the weekday of d.\n",
    );
}

#[test]
fn make_moment_aot_matches_interpreter() {
    common::assert_compiled_equals_interpreted(
        "## Main\nLet m be makeMoment(2024, 3, 15, 10, 30, 45).\nShow m.\nShow the hour of m.\nShow the minute of m.\nShow the day of (m + 1 month).\n",
    );
}

// --- C tier: makeMoment (the Moment extractor path) byte-exact, incl the timegm cross-check ---

#[test]
fn make_moment_c_extractors() {
    common::assert_c_output(
        "## Main\nLet m be makeMoment(2024, 3, 15, 10, 30, 45).\nShow the year of m.\nShow the hour of m.\nShow the minute of m.\nShow the second of m.\n",
        "2024\n10\n30\n45",
    );
}

#[test]
fn make_moment_c_agrees_with_timegm() {
    common::assert_c_output(
        "## Main\nLet a be makeMoment(1970, 1, 1, 0, 0, 0).\nLet b be makeMoment(2000, 2, 29, 12, 0, 0).\nShow the seconds between a and b.\n",
        "951825600",
    );
}

/// makeDate construction + ISO display on the C tier.
#[test]
fn make_date_c_shows_iso() {
    common::assert_c_output("## Main\nShow makeDate(2024, 3, 15).\n", "2024-03-15");
}

/// C-tier calendar-component extractors on a constructed `Date`. These were latently WRONG: the C
/// extractor treated a Date's days-since-epoch as nanoseconds. The fix converts a Date to midnight-UTC
/// nanos first. 2024-03-15 → year 2024, month 3, day 15, weekday 5 (Friday, ISO 8601 convention).
#[test]
fn make_date_c_extractors() {
    common::assert_c_output(
        "## Main\nLet d be makeDate(2024, 3, 15).\nShow the year of d.\nShow the month of d.\nShow the day of d.\nShow the weekday of d.\n",
        "2024\n3\n15\n5",
    );
}

// --- calendar.lg now derives its former O(n)-loop functions from the built-ins (O(1)) ---
// timegm/daysBeforeYear/daysBeforeMonth/leapDaysBetween call makeMoment + `the seconds between`
// instead of looping over years/months. These lock the rewrite (parity_calendar already pins
// timegm + leapDaysBetween; daysBefore* were previously untested).

#[test]
fn days_before_year_via_builtins() {
    assert_all_tiers(
        "## Main\nShow daysBeforeYear(1970).\nShow daysBeforeYear(1971).\nShow daysBeforeYear(2000).\nShow daysBeforeYear(1969).\n",
        "0\n365\n10957\n-365",
    );
}

#[test]
fn days_before_month_via_builtins() {
    assert_all_tiers(
        "## Main\nShow daysBeforeMonth(2024, 1).\nShow daysBeforeMonth(2024, 3).\nShow daysBeforeMonth(2023, 3).\n",
        "0\n60\n59",
    );
}

/// The old loop identity must hold: daysBeforeYear + daysBeforeMonth + (day-1) = the day index of the date.
#[test]
fn days_before_compose_to_epoch_day() {
    assert_all_tiers(
        "## Main\nShow daysBeforeYear(2024) + daysBeforeMonth(2024, 3) + 14.\n",
        "19797",
    );
}

/// The built-in-backed calendar functions run on the compiled tiers too (AOT-Rust ≡ interp; C byte-exact).
#[test]
fn calendar_via_builtins_compiled_tiers() {
    common::assert_compiled_equals_interpreted(
        "## Main\nShow timegm(2024, 3, 15, 10, 30, 45).\nShow daysBeforeYear(2000).\nShow leapDaysBetween(2000, 2025).\n",
    );
    common::assert_c_output(
        "## Main\nShow timegm(2024, 3, 15, 10, 30, 45).\nShow leapDaysBetween(2000, 2025).\n",
        "1710498645\n7",
    );
}

// --- Direct-WASM tier (linker mode): the constructors lower via base::temporal host shims ---

/// The linked-WASM lowering must handle `makeDate`/`makeMoment` — this exercises the `MakeDateRt`/
/// `MakeMomentRt` HostFn noting, registration, argument/return signatures, and the lowering arm. A real
/// lowering fault (an unregistered host fn, a bad signature, an unhandled builtin) fails here; the
/// environment-dependent wasm32 base-runtime build is skipped exactly as the in-crate linked test does.
// GATED, because `compile_to_wasm_linked` is: it lives behind `feature = "wasm-jit"` (it links a
// wasm32 build of `logicaffeine_base` with `rust-lld`). Calling it ungated does not fail THIS
// test — it fails to COMPILE, and a test target that does not compile takes the whole
// `logicaffeine-tests` crate down with it, so every other binary in the suite goes dark at once.
// `wasm_gated_suite_lock` exists for the mirror image of this mistake; the rule is the same in
// both directions: a gated surface is reached only from behind its gate.
#[cfg(feature = "wasm-jit")]
#[test]
fn make_date_moment_lower_to_wasm_linked() {
    // PREFLIGHT, not post-hoc excuse. This used to attempt the link and then READ THE ERROR to
    // decide whether the toolchain was missing — printing `SKIP …` and reporting PASS, which is
    // the false-green shape `skip_budget_lock` exists to refuse: a real lowering fault whose
    // message merely mentioned `wasm32` would have been filed as an environment gap and the
    // failure would never have been seen. Asking FIRST means the test either runs for real or
    // records an honest skip marker — and under `LOGOS_REQUIRE_TOOLS=wasm32` a missing target is
    // a loud failure rather than a quiet pass.
    if !common::require_tool("wasm32") {
        return;
    }
    let src = "## Main\nLet d be makeDate(2024, 3, 15).\nShow d.\nLet m be makeMoment(2024, 3, 15, 10, 30, 45).\nShow m.\n";
    // With the target present, ANY error is a genuine lowering fault: an unregistered host fn, a
    // bad signature, an unhandled builtin. There is nothing left for it to be excused as.
    if let Err(e) = logicaffeine_compile::compile::compile_to_wasm_linked(src) {
        panic!("makeDate/makeMoment failed to LOWER on the linked WASM tier: {e:?}");
    }
}
