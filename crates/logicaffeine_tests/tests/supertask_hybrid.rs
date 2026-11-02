//! Loops of infinitesimal steps whose state flows between jumps: a ball that bounces, a thermostat
//! that switches, a switch thrown at a set time. The supertask engine runs the ideal hybrid system
//! such a loop follows — its flights, its events, the infinitely many bounces of a Zeno tail and the
//! rest after them — exactly, and every tier prints the same bytes. A loop whose end state depends on
//! which infinitesimal `dt` is, or that the engine cannot follow, stops with the reason.

mod common;
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(300);

fn assert_all_tiers(source: &str, expected: &str) {
    common::assert_all_tiers_within(source, expected, DEADLINE)
}

fn refused(source: &str, reason: &str) {
    let agreed = common::run_all_tiers_within(source, DEADLINE);
    let error = agreed
        .error
        .unwrap_or_else(|| panic!("the program ran to its end instead of refusing:\n{}", agreed.output));
    assert!(error.contains(reason), "the refusal does not say `{reason}`: {error}");
}

/// The bouncing ball of the paper, with a `last` that each statement of `record` sets.
fn ball(h0: &str, v0: &str, stop: &str, record: &str, shows: &str) -> String {
    format!(
        "## Main
Let dt be an infinitesimal.
Let g be 10.
Let c: Rational be 1 / 2.
Let stop: Rational be {stop}.
Let h: Hyperreal be {h0}.
Let v: Hyperreal be {v0}.
Let time: Hyperreal be 0.
Let last: Hyperreal be 0.
Let bounces be 0.
While time < stop:
    Set h to h + v * dt.
    Set v to v - g * dt.
    If h <= 0 and v < 0:
        Set h to 0.
        Set v to 0 - c * v.
{record}
    Set time to time + dt.
{shows}"
    )
}

const STANDARD_BOUNCES: &str = "        If v is not infinitesimal:\n            Set last to time.";
const EVERY_BOUNCE: &str = "        Set last to time.";
const STATE: &str = "Show the standard part of h.\nShow the standard part of v.\nShow the standard part of last.";

// ── Zeno ─────────────────────────────────────────────────────────────────────────────────────

#[test]
fn the_ball_rests_after_its_zeno_time_and_its_standard_bounces_end_there() {
    assert_all_tiers(&ball("0", "10", "5", STANDARD_BOUNCES, STATE), "0\n0\n4");
}

#[test]
fn every_bounce_recorded_includes_the_resting_bounces_up_to_the_loops_end() {
    assert_all_tiers(&ball("0", "10", "5", EVERY_BOUNCE, STATE), "0\n0\n5");
}

#[test]
fn a_rebound_known_to_be_positive_counts_the_resting_bounces_too() {
    let record = "        If v > 0:\n            Set last to time.";
    assert_all_tiers(&ball("0", "10", "5", record, STATE), "0\n0\n5");
}

#[test]
fn a_ball_dropped_from_a_height_rests_after_its_own_zeno_time() {
    assert_all_tiers(&ball("5", "0", "10", STANDARD_BOUNCES, STATE), "0\n0\n3");
}

#[test]
fn an_irrational_fall_gives_an_algebraic_zeno_time() {
    assert_all_tiers(&ball("1", "0", "5", STANDARD_BOUNCES, STATE), "0\n0\n3√5/5");
}

#[test]
fn a_condition_that_turns_along_the_bounces_records_the_last_bounce_it_holds_for() {
    let above_one = "        If v > 1:\n            Set last to time.";
    assert_all_tiers(&ball("0", "10", "5", above_one, STATE), "0\n0\n7/2");
    let above_a_thousandth = "        If v > 1 / 1000:\n            Set last to time.";
    assert_all_tiers(&ball("0", "10", "5", above_a_thousandth, STATE), "0\n0\n8191/2048");
}

#[test]
fn a_loop_that_ends_before_the_zeno_time_ends_mid_flight() {
    assert_all_tiers(&ball("0", "10", "13 / 4", STANDARD_BOUNCES, STATE), "5/16\n0\n3");
}

#[test]
fn a_loop_that_ends_at_the_zeno_time_ends_at_rest() {
    assert_all_tiers(&ball("0", "10", "4", STANDARD_BOUNCES, STATE), "0\n0\n4");
}

#[test]
fn the_clock_ends_exactly_where_the_loop_stops_it() {
    let shows = "Show time.\nShow the standard part of time.\nShow time - stop is infinitesimal.";
    assert_all_tiers(&ball("0", "10", "5", STANDARD_BOUNCES, shows), "5\n5\ntrue");
}

#[test]
fn a_passenger_that_flows_on_its_own_crosses_the_zeno_time_unhindered() {
    let source = "## Main
Let dt be an infinitesimal.
Let h: Hyperreal be 0.
Let v: Hyperreal be 10.
Let x: Hyperreal be 0.
Let time: Hyperreal be 0.
While time < 5:
    Set h to h + v * dt.
    Set v to v - 10 * dt.
    Set x to x + 3 * dt.
    If h <= 0 and v < 0:
        Set h to 0.
        Set v to 0 - v / 2.
    Set time to time + dt.
Show the standard part of x.
Show the standard part of h.";
    assert_all_tiers(source, "15\n0");
}

// ── periods and switches ─────────────────────────────────────────────────────────────────────

#[test]
fn an_elastic_ball_bounces_on_a_period_and_counts_exactly() {
    let source = "## Main
Let dt be an infinitesimal.
Let h: Hyperreal be 0.
Let v: Hyperreal be 10.
Let time: Hyperreal be 0.
Let bounces be 0.
While time < 999999:
    Set h to h + v * dt.
    Set v to v - 10 * dt.
    If h <= 0 and v < 0:
        Set h to 0.
        Set v to 0 - v.
        Set bounces to bounces + 1.
    Set time to time + dt.
Show bounces.
Show the standard part of h.
Show the standard part of v.";
    assert_all_tiers(source, "499999\n5\n0");
}

#[test]
fn a_thermostat_switches_on_its_cycle_for_a_million_seconds() {
    let source = "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 20.
Let rate be 1.
Let switches be 0.
Let time: Hyperreal be 0.
While time < 1000001:
    Set x to x + rate * dt.
    If x >= 22 and rate > 0:
        Set rate to 0 - 1.
        Set switches to switches + 1.
    If x <= 18 and rate < 0:
        Set rate to 1.
        Set switches to switches + 1.
    Set time to time + dt.
Show switches.
Show rate.
Show the standard part of x.";
    assert_all_tiers(source, "250000\n1\n21");
}

#[test]
fn a_switch_thrown_at_a_set_time_changes_the_flow_after_it() {
    let source = "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 0.
Let speed be 1.
Let switched be false.
Let time: Hyperreal be 0.
While time < 3:
    Set x to x + speed * dt.
    If time >= 1 and not switched:
        Set speed to 2.
        Set switched to true.
    Set time to time + dt.
Show switched.
Show speed.
Show the standard part of x.";
    assert_all_tiers(source, "true\n2\n5");
}

#[test]
fn a_jump_checked_before_the_flow_fires_on_the_starting_state() {
    let source = "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 0.
Let fired be 0.
Let time: Hyperreal be 0.
While time < 2:
    If x <= 0:
        Set x to 5.
        Set fired to fired + 1.
    Set x to x + dt.
    Set time to time + dt.
Show fired.
Show the standard part of x.";
    assert_all_tiers(source, "1\n7");
}

#[test]
fn a_guard_that_holds_on_records_until_the_loop_ends() {
    let source = "## Main
Let dt be an infinitesimal.
Let h: Hyperreal be 5.
Let v: Hyperreal be 0.
Let landed be false.
Let when: Hyperreal be 0.
Let time: Hyperreal be 0.
While time < 4:
    Set h to h + v * dt.
    Set v to v - 10 * dt.
    If h <= 0:
        Set landed to true.
        Set when to time.
    Set time to time + dt.
Show landed.
Show the standard part of when.
Show the standard part of h.";
    assert_all_tiers(source, "true\n4\n-75");
}

#[test]
fn a_finite_clock_with_jumps_runs_as_written() {
    let source = "## Main
Let dt be an infinitesimal.
Let x be 0.
Let flips be 0.
Let time be 0.
While time < 10:
    Set x to x + 1.
    If x >= 3:
        Set x to 0.
        Set flips to flips + 1.
    Set time to time + 1.
Show flips.
Show x.";
    assert_all_tiers(source, "3\n1");
}

#[test]
fn a_jump_that_sets_a_value_to_itself_while_its_guard_holds_leaves_the_flow_alone() {
    let source = "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 0.
Let seen: Hyperreal be 0.
Let time: Hyperreal be 0.
While time < 3:
    Set x to x + dt.
    If x >= 1:
        Set x to x.
        Set seen to x.
    Set time to time + dt.
Show the standard part of x.
Show the standard part of seen.";
    assert_all_tiers(source, "3\n3");
}

// ── refusals ─────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_bounce_in_the_loops_last_step_is_refused() {
    refused(&ball("0", "10", "3", STANDARD_BOUNCES, STATE), "last step");
}

#[test]
fn counting_infinitely_many_bounces_is_refused() {
    let record = "        Set bounces to bounces + 1.";
    refused(&ball("0", "10", "5", record, "Show bounces."), "grows without bound");
}

#[test]
fn a_condition_on_values_infinitely_close_to_each_other_is_refused() {
    let source = "## Main
Let dt be an infinitesimal.
Let h: Hyperreal be 0.
Let v: Hyperreal be 10.
Let below be 0.
Let time: Hyperreal be 0.
While time < 5:
    Set h to h + v * dt.
    Set v to v - 10 * dt.
    If h <= 0 and v < 0:
        If h < 0:
            Set below to below + 1.
        Set h to 0.
        Set v to 0 - v / 2.
    Set time to time + dt.
Show below.";
    refused(source, "infinitely close");
}

#[test]
fn a_jump_with_an_otherwise_is_refused() {
    let source = "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 0.
Let time: Hyperreal be 0.
While time < 1:
    Set x to x + dt.
    If x > 1 / 2:
        Set x to 0.
    Otherwise:
        Set x to x + dt.
    Set time to time + dt.
Show x.";
    refused(source, "Otherwise");
}

#[test]
fn a_jump_that_shows_a_value_is_refused() {
    let source = "## Main
Let dt be an infinitesimal.
Let h: Hyperreal be 0.
Let v: Hyperreal be 10.
Let time: Hyperreal be 0.
While time < 5:
    Set h to h + v * dt.
    Set v to v - 10 * dt.
    If h <= 0 and v < 0:
        Show \"bounce\".
        Set h to 0.
        Set v to 0 - v / 2.
    Set time to time + dt.
Show h.";
    refused(source, "does more than set values");
}

#[test]
fn a_jump_that_pushes_the_state_it_holds_away_is_refused() {
    let source = "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 0.
Let time: Hyperreal be 0.
While time < 3:
    Set x to x + dt.
    If x >= 1:
        Set x to 2 * x - 1.
    Set time to time + dt.
Show x.";
    refused(source, "does not pull the state back");
}

#[test]
fn a_flow_that_is_not_a_polynomial_in_time_is_refused() {
    let source = "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 1.
Let hits be 0.
Let time: Hyperreal be 0.
While time < 2:
    Set x to x + x * dt.
    If x >= 2 and hits = 0:
        Set hits to 1.
    Set time to time + dt.
Show hits.";
    refused(source, "not a polynomial in time");
}

#[test]
fn a_zeno_tail_whose_guard_reads_the_clock_is_refused_by_the_event_budget() {
    let source = "## Main
Let dt be an infinitesimal.
Let h: Hyperreal be 0.
Let v: Hyperreal be 10.
Let time: Hyperreal be 0.
While time < 5:
    Set h to h + v * dt.
    Set v to v - 10 * dt.
    If h <= 0 and v < 0 and time < 100:
        Set h to 0.
        Set v to 0 - v / 2.
    Set time to time + dt.
Show the standard part of h.";
    let budget = logicaffeine_base::hybrid::EVENT_BUDGET.to_string();
    let budget = budget
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|c| std::str::from_utf8(c).expect("digits"))
        .collect::<Vec<_>>()
        .join(",");
    refused(source, &format!("fire more than {budget} times before its end, each time sooner after the last,"));
}
