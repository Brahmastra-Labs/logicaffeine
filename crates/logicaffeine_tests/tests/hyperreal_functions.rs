//! The exponential, the logarithm and roots of hyperreal numbers. `exp(dt)` is the germ
//! `1 + dt + O(dt^2)`, known by its series in `dt` with exact coefficients; `ln(1 + dt)` is
//! `dt - (1/2)·dt^2 + O(dt^3)`; a root of a hyperreal meets a flow or a computable real as a germ of
//! fractional orders. A value whose logarithm or exponential leaves the germs — `ln(dt)`, which
//! carries `ln(dt)` itself, and `exp(1/dt)`, beyond every power of `dt` — is refused by name.

mod common;
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(300);

fn shows(body: &str, expected: &str) {
    let source = format!("## Main\nLet dt be an infinitesimal.\n{body}");
    common::assert_all_tiers_within(&source, expected, DEADLINE)
}

fn fails_with(body: &str, needle: &str) {
    let source = format!("## Main\nLet dt be an infinitesimal.\n{body}");
    let out = common::run_all_tiers_within(&source, DEADLINE);
    let err = out
        .error
        .unwrap_or_else(|| panic!("expected an error containing {needle:?}; the program printed {:?}", out.output));
    assert!(err.contains(needle), "{err}");
}

// ── the exponential ───────────────────────────────────────────────────────────────────────────

#[test]
fn the_exponential_of_dt_is_its_series() {
    shows(
        "Let x: Hyperreal be exp(dt).\nShow x.\nShow the standard part of x.\nShow x > 1.\nShow x - 1 is infinitesimal.",
        "1 + dt + O(dt^2)\n1\ntrue\ntrue",
    );
}

#[test]
fn the_exponential_of_an_appreciable_hyperreal_has_the_standard_exponential_for_its_standard_part() {
    shows(
        "Let x: Hyperreal be exp(2 + dt).\nShow the standard part of x.\nShow x.",
        "e^2\ne^2 + e^2·dt + O(dt^2)",
    );
}

#[test]
fn the_exponential_of_a_root_of_dt_has_fractional_orders() {
    shows("Let x: Hyperreal be exp(sqrt(dt)).\nShow x.", "1 + √dt + O(dt)");
}

#[test]
fn an_infinite_exponent_is_beyond_every_germ() {
    fails_with("Show exp(1 / dt).", "the exponential of 1/dt is beyond every power of dt");
}

// ── the logarithm ─────────────────────────────────────────────────────────────────────────────

#[test]
fn the_logarithm_of_one_plus_dt_is_its_series() {
    shows(
        "Let x: Hyperreal be ln(1 + dt).\nShow x.\nShow x is infinitesimal.\nShow x < dt.",
        "dt - (1/2)·dt^2 + O(dt^3)\ntrue\ntrue",
    );
}

#[test]
fn the_logarithm_undoes_the_exponential_to_every_order_it_reads() {
    shows(
        "Let x: Hyperreal be ln(exp(dt)).\nShow the standard part of x / dt.\nShow x - dt is infinitesimal.",
        "1\ntrue",
    );
}

#[test]
fn the_logarithm_of_an_infinitesimal_is_no_germ() {
    fails_with("Show ln(dt).", "the logarithm of dt carries ln(dt)");
    fails_with("Show ln(0 - 1 - dt).", "logarithm of a nonpositive number");
}

// ── roots meeting computable reals and flows ──────────────────────────────────────────────────

#[test]
fn a_root_of_dt_beside_a_computable_real_is_a_germ() {
    shows(
        "Let x: Hyperreal be sqrt(dt) + pi.\nShow x.\nShow the standard part of x.\nShow x > pi.",
        "π + √dt + O(dt)\nπ\ntrue",
    );
}

#[test]
fn a_flow_s_end_state_takes_a_root_and_a_logarithm() {
    shows(
        "Let x: Hyperreal be 1.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Set x to x + x * dt.\n    Set t to t + dt.\nShow the standard part of sqrt(x).\nShow the standard part of ln(x).",
        "e^(1/2)\n1",
    );
}

#[test]
fn the_exponential_of_a_negative_infinite_value_is_below_every_power() {
    fails_with("Show exp(0 - 1 / dt).", "the exponential of -1/dt is beyond every power of dt");
}

#[test]
fn the_logarithm_of_an_infinite_value_carries_the_logarithm_of_its_reciprocal() {
    fails_with("Show ln(1 / dt).", "the logarithm of 1/dt carries ln(1/dt)");
    fails_with("Show ln(sqrt(dt)).", "the logarithm of √dt carries ln(dt)");
}

// ── comparisons read the first order where two series differ ──────────────────────────────────

#[test]
fn the_exponential_lies_where_its_series_puts_it() {
    shows(
        "Let x: Hyperreal be exp(dt).\nShow x > 1 + dt.\nShow x < 1 + dt + dt * dt.\nShow ln(1 + dt) < dt.\nShow exp(sqrt(dt)) > 1 + sqrt(dt).",
        "true\ntrue\ntrue\ntrue",
    );
}

// ── rational powers and roots of germs ────────────────────────────────────────────────────────

#[test]
fn a_germ_takes_a_rational_power_as_the_power_of_its_root() {
    shows(
        "Let half: Rational be 1 / 2.\nLet third: Rational be 1 / 3.\nLet x: Hyperreal be exp(dt).\nShow x ** half.\nShow sqrt(x).\nShow (0 - x) ** third.",
        "1 + (1/2)·dt + O(dt^2)\n1 + (1/2)·dt + O(dt^2)\n-1 - (1/3)·dt + O(dt^2)",
    );
}

#[test]
fn a_negative_germ_has_no_square_root() {
    fails_with("Let x: Hyperreal be exp(dt).\nShow sqrt(0 - x).", "square root of a negative number");
}

// ── flows from a root of dt ───────────────────────────────────────────────────────────────────

#[test]
fn a_flow_from_a_root_of_dt_runs_on_the_root_s_grid() {
    shows(
        "Let x: Hyperreal be sqrt(dt).\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Set x to x + x * dt.\n    Set t to t + dt.\nShow x.\nShow the standard part of x / sqrt(dt).\nShow x is infinitesimal.",
        "e·√dt - (1/2)·e·dt^(3/2) + O(dt^2)\ne\ntrue",
    );
}

#[test]
fn a_flow_whose_rate_holds_a_root_of_dt_runs_on_that_root_s_grid() {
    shows(
        "Let k: Hyperreal be 2 + sqrt(dt).\nLet x: Hyperreal be 1.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Set x to x + k * x * dt.\n    Set t to t + dt.\nShow x.\nShow the standard part of x.",
        "e^2 + e^2·√dt + O(dt)\ne^2",
    );
}

#[test]
fn a_step_of_a_fractional_order_below_dt_is_no_flow() {
    fails_with(
        "Let k: Hyperreal be sqrt(dt).\nLet x: Hyperreal be 1.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Set x to x + x * k.\n    Set t to t + dt.\nShow x.",
        "the update's entry 1 + √dt has a term of a fractional order of the infinitesimal below its first, so this supertask is not a flow",
    );
    fails_with(
        "Let k: Hyperreal be sqrt(dt).\nLet h: Hyperreal be 0.\nLet v: Hyperreal be 10.\nLet time: Hyperreal be 0.\nWhile time < 5:\n    Set h to h + v * k.\n    Set v to v - 10 * dt.\n    If h <= 0 and v < 0:\n        Set h to 0.\n        Set v to 0 - v / 2.\n    Set time to time + dt.\nShow h.",
        "the flow's coefficient √dt has a term of a fractional order of the infinitesimal below its first, so the loop is not an Euler step",
    );
}

#[test]
fn a_flow_s_coupled_components_meet_on_the_finest_grid_any_rate_lies_on() {
    shows(
        "Let k: Hyperreal be 2 + sqrt(dt).\nLet x: Hyperreal be 1.\nLet y: Hyperreal be 1.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Set x to x + (k * x + y) * dt.\n    Set y to y + k * y * dt.\n    Set t to t + dt.\nShow the standard part of x.\nShow y.",
        "2·e^2\ne^2 + e^2·√dt + O(dt)",
    );
}

#[test]
fn a_flow_whose_terms_do_not_commute_is_known_to_its_standard_part() {
    shows(
        "Let k: Hyperreal be 2 + sqrt(dt).\nLet x: Hyperreal be 1.\nLet y: Hyperreal be 1.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Set x to x + (k * x + y) * dt.\n    Set y to y + 3 * y * dt.\n    Set t to t + dt.\nShow the standard part of x.\nShow the standard part of y.\nShow y.",
        "e^3\ne^3\ne^3 + O(√dt)",
    );
}

#[test]
fn a_nonlinear_flow_reads_its_rate_on_a_root_of_dt() {
    shows(
        "Let k: Hyperreal be 2 + sqrt(dt).\nLet x: Hyperreal be 1 / 2.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Set x to x + k * x * (1 - x) * dt.\n    Set t to t + dt.\nShow the standard part of x.",
        "0.880797077977882444059729141302",
    );
}

#[test]
fn a_hybrid_loop_reads_its_rate_on_a_root_of_dt() {
    shows(
        "Let k: Hyperreal be 1 + sqrt(dt).\nLet h: Hyperreal be 0.\nLet v: Hyperreal be 10.\nLet time: Hyperreal be 0.\nLet last: Hyperreal be 0.\nWhile time < 5:\n    Set h to h + v * k * dt.\n    Set v to v - 10 * dt.\n    If h <= 0 and v < 0:\n        Set h to 0.\n        Set v to 0 - v / 2.\n        If v is not infinitesimal:\n            Set last to time.\n    Set time to time + dt.\nShow the standard part of h.\nShow the standard part of last.",
        "0\n4",
    );
}

#[test]
fn a_rate_that_is_no_polynomial_in_a_root_of_dt_is_refused() {
    fails_with(
        "Let k: Hyperreal be sqrt(1 + dt).\nLet x: Hyperreal be 1.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Set x to x + k * x * dt.\n    Set t to t + dt.\nShow x.",
        "is not a polynomial in the infinitesimal, or in a root of it, with rational coefficients",
    );
}

#[test]
fn a_nonlinear_step_with_a_term_below_dt_is_no_flow() {
    fails_with(
        "Let k: Hyperreal be 1 / sqrt(dt).\nLet x: Hyperreal be 1 / 2.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Set x to x + x * x * k * dt.\n    Set t to t + dt.\nShow x.",
        "the update's coefficient √dt has a term of a fractional order of the infinitesimal below its first, so this supertask is not a flow",
    );
}
