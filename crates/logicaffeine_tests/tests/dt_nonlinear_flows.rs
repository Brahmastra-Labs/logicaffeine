//! Nonlinear flows: a loop stepped by an infinitesimal `dt` whose body is an Euler step of a
//! polynomial differential equation ends, after `⌈T/dt⌉` steps, infinitely close to the
//! equation's solution at `T` (Suenaga–Hasuo). Its standard part is that solution as a computable
//! real — enclosed by a validated Taylor integration, so every printed digit is certified — and a
//! solution that leaves every bound before `T` leaves an iterate that is not limited. Every program
//! must finish, on every run path.

mod common;
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(300);

fn assert_all_tiers(source: &str, expected: &str) {
    common::assert_all_tiers_within(source, expected, DEADLINE)
}

#[test]
fn the_logistic_equation_flows_to_its_sigmoid() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 1 / 2.
Let t: Hyperreal be 0.
While t < 1:
    Set x to x + x * (1 - x) * dt.
    Set t to t + dt.
Show the standard part of x.
Show x is limited.",
        "0.731058578630004879251159241822\ntrue",
    );
}

#[test]
fn a_riccati_equation_reaches_its_exact_value() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 1.
Let t: Hyperreal be 0.
While t < 1:
    Set x to x - x * x * dt.
    Set t to t + dt.
Show the standard part of x.",
        "0.500000000000000000000000000000",
    );
}

#[test]
fn a_nonlinear_rotation_turns_one_radian() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 1.
Let y: Hyperreal be 0.
Let t: Hyperreal be 0.
While t < 1:
    Let r be x * x + y * y.
    Let dx be 0 - y * r.
    Let dy be x * r.
    Set x to x + dx * dt.
    Set y to y + dy * dt.
    Set t to t + dt.
Show the standard part of x.
Show the standard part of y.",
        "0.540302305868139717400936607443\n0.841470984807896506652502321630",
    );
}

#[test]
fn a_nonlinear_flow_prints_its_standard_part_and_its_order() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 1 / 2.
Let t: Hyperreal be 0.
While t < 1:
    Set x to x + x * (1 - x) * dt.
    Set t to t + dt.
Show x.",
        "0.731058578630004879251159241822 + O(dt)",
    );
}

#[test]
fn a_flow_that_blows_up_before_its_time_ends_unlimited() {
    let agreed = common::run_all_tiers_within(
        "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 1.
Let t: Hyperreal be 0.
While t < 2:
    Set x to x + x * x * dt.
    Set t to t + dt.
Show x is limited.
Show the standard part of x.",
        DEADLINE,
    );
    assert_eq!(agreed.output.trim(), "false");
    let error = agreed.error.expect("an unlimited value has no standard part");
    assert!(error.contains("unlimited is infinite and has no standard part"), "{error}");
}

#[test]
fn a_flow_that_blows_up_after_its_time_has_a_standard_part() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 1.
Let t: Hyperreal be 0.
Let half: Rational be 1 / 2.
While t < half:
    Set x to x + x * x * dt.
    Set t to t + dt.
Show the standard part of x.",
        "2.00000000000000000000000000000",
    );
}

#[test]
fn a_flow_whose_rate_reads_its_clock_carries_the_clock_along() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 1.
Let t: Hyperreal be 0.
While t < 1:
    Set x to x + 2 * t * x * dt.
    Set t to t + dt.
Show the standard part of x.",
        "2.71828182845904523536028747135",
    );
}
