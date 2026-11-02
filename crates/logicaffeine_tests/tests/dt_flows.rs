//! Flows: a loop stepped by an infinitesimal `dt` whose body is an Euler step of a linear
//! differential equation ends, after `⌈T/dt⌉` steps, in a hyperreal infinitely close to the
//! equation's solution at `T` — its standard part is that solution, exactly — and the value itself
//! is known by its series in `dt`. Every program must finish, on every run path.

mod common;
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(300);

fn assert_all_tiers(source: &str, expected: &str) {
    common::assert_all_tiers_within(source, expected, DEADLINE)
}

#[test]
fn exponential_growth_flows_to_e_and_its_clock_to_one() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 1.
Let t: Hyperreal be 0.
While t < 1:
    Set x to x + x * dt.
    Set t to t + dt.
Show the standard part of x.
Show t.
Show x.",
        "e\n1\ne - (1/2)·e·dt + O(dt^2)",
    );
}

#[test]
fn exponential_decay_flows_to_its_rate() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 1.
Let t: Hyperreal be 0.
While t < 1:
    Set x to x - 2 * x * dt.
    Set t to t + dt.
Show the standard part of x.",
        "e^(-2)",
    );
}

#[test]
fn an_oscillator_run_for_two_units_of_time_turns_two_radians() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 1.
Let v: Hyperreal be 0.
Let t: Hyperreal be 0.
While t < 2:
    Let a be 0 - x.
    Set x to x + v * dt.
    Set v to v + a * dt.
    Set t to t + dt.
Show the standard part of x.
Show the standard part of v.",
        "cos(2)\n-sin(2)",
    );
}

#[test]
fn a_symplectic_step_has_the_same_standard_part_and_refuses_what_it_cannot_compute() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 1.
Let v: Hyperreal be 0.
Let t: Hyperreal be 0.
While t < 1:
    Set x to x + v * dt.
    Set v to v - x * dt.
    Set t to t + dt.
Show the standard part of x.
Show x.",
        "cos(1)\ncos(1) + O(dt)",
    );
}

#[test]
fn a_flow_whose_step_count_depends_on_the_residue_has_one_standard_part() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 1.
Let t: Hyperreal be 0.
While t < 1:
    Set x to x + x * 2 * dt.
    Set t to t + 2 * dt.
Show the standard part of x.
Show t.",
        "e\n1 when ⌈1/dt⌉ is even, 1 + dt when it is odd",
    );
}

#[test]
fn a_flows_value_compares_with_standard_numbers() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 1.
Let t: Hyperreal be 0.
While t < 1:
    Set x to x + x * dt.
    Set t to t + dt.
Show x > 2.
Show x < 3.
Show x is limited.
Show x is infinitesimal.
Show floor(x).",
        "true\ntrue\ntrue\nfalse\n2",
    );
}

#[test]
fn a_damped_oscillator_decays_inside_the_unit_interval() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 1.
Let v: Hyperreal be 0.
Let t: Hyperreal be 0.
While t < 1:
    Let a be 0 - x - v.
    Set x to x + v * dt.
    Set v to v + a * dt.
    Set t to t + dt.
Let s be the standard part of x.
Show s > 0.
Show s < 1.",
        "true\ntrue",
    );
}
