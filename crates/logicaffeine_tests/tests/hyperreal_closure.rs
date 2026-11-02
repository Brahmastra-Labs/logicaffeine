//! The real closure of the hyperreals: roots of hyperreal numbers, exactly. `sqrt(dt)` is a
//! hyperreal of valuation one half, `sqrt(1 + dt)` the square root whose expansion is
//! `1 + dt/2 - dt²/8 + …`, and every such value is a real algebraic Puiseux series in `dt`, held by a
//! polynomial it is a root of and the branch that picks it. Field operations, order, equality and
//! the standard part are exact, and every value prints in one way: as a rational function of a root
//! of `dt` when it is one, as a radical when a power of it is, and otherwise by its leading orders.

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

// ── the square root of the infinitesimal ──────────────────────────────────────────────────────

#[test]
fn the_square_root_of_dt_is_a_hyperreal_of_valuation_one_half() {
    shows(
        "Let r: Hyperreal be sqrt(dt).\nShow r.\nShow r * r.\nShow r * r = dt.\nShow r > dt.\nShow r is infinitesimal.",
        "√dt\ndt\ntrue\ntrue\ntrue",
    );
}

#[test]
fn the_square_root_of_dt_lies_beyond_every_power_of_dt_below_it() {
    shows(
        "Let r: Hyperreal be sqrt(dt).\nLet tiny: Rational be 1 / 1000000.\nShow r < tiny.\nShow r > dt * 1000000.\nShow 1 / r.\nShow 1 / r is limited.",
        "true\ntrue\n1/√dt\nfalse",
    );
}

#[test]
fn a_polynomial_in_the_square_root_of_dt_prints_in_ascending_powers() {
    shows(
        "Let r: Hyperreal be sqrt(dt).\nShow (1 + r) ** 2.\nShow r ** 3.\nShow (r + dt) / r.",
        "1 + 2·√dt + dt\ndt^(3/2)\n1 + √dt",
    );
}

// ── square roots of appreciable hyperreals ────────────────────────────────────────────────────

#[test]
fn the_square_root_of_one_plus_dt_is_a_radical_whose_square_is_exact() {
    shows(
        "Let r: Hyperreal be sqrt(1 + dt).\nShow r.\nShow r * r.\nShow r * r = 1 + dt.\nShow 0 - r.",
        "√(1 + dt)\n1 + dt\ntrue\n-√(1 + dt)",
    );
}

#[test]
fn a_root_has_the_standard_part_of_the_root_of_the_standard_part() {
    shows(
        "Let r: Hyperreal be sqrt(4 + dt).\nShow the standard part of r.\nShow r > 2.\nShow r - 2 is infinitesimal.\nShow r is infinitely close to 2.",
        "2\ntrue\ntrue\ntrue",
    );
}

#[test]
fn a_radical_takes_its_constant_factor_out() {
    shows("Show sqrt(4 + 4 * dt).\nShow sqrt(2 * dt).", "2·√(1 + dt)\n√2·√dt");
}

#[test]
fn an_algebraic_number_and_a_root_of_dt_add_exactly() {
    shows("Let a: Algebraic be sqrt(2).\nLet r: Hyperreal be a + sqrt(dt).\nShow r.\nShow r * r.", "√2 + √dt\n2 + 2√2·√dt + dt");
}

#[test]
fn a_value_neither_rational_nor_radical_prints_its_leading_orders() {
    shows(
        "Let r: Hyperreal be sqrt(2 + dt) + sqrt(2 - dt).\nLet c: Algebraic be 2 * sqrt(2).\nShow r.\nShow the standard part of r.\nShow r < c.\nShow 0 - r.",
        "2√2 - (√2/16)·dt^2 + O(dt^3)\n2√2\ntrue\n-2√2 + (√2/16)·dt^2 + O(dt^3)",
    );
}

#[test]
fn roots_that_agree_to_a_high_order_are_still_told_apart() {
    shows(
        "Let a: Hyperreal be sqrt(1 + dt ** 3).\nLet b: Hyperreal be sqrt(1 + dt ** 3 + dt ** 4).\nShow a < b.\nShow b > a.\nShow b < a.\nShow a = b.\nShow b - a is infinitesimal.\nShow b - a.\nShow a - b.",
        "true\ntrue\nfalse\nfalse\ntrue\n(1/2)·dt^4 - (1/4)·dt^7 + O(dt^8)\n-(1/2)·dt^4 + (1/4)·dt^7 + O(dt^8)",
    );
}

#[test]
fn equal_values_reached_two_ways_are_equal() {
    shows(
        "Let a: Hyperreal be sqrt(2 + 2 * dt).\nLet b: Hyperreal be sqrt(2) * sqrt(1 + dt).\nShow a = b.\nShow a - b.",
        "true\n0",
    );
}

// ── rational powers ──────────────────────────────────────────────────────────────────────────

#[test]
fn a_rational_power_is_a_root_raised_to_a_power() {
    shows(
        "Let half: Rational be 1 / 2.\nLet third: Rational be 1 / 3.\nLet three_halves: Rational be 3 / 2.\nShow dt ** half.\nShow dt ** three_halves.\nShow (8 + 8 * dt) ** third.\nShow (dt ** third) ** 3.",
        "√dt\ndt^(3/2)\n2·∛(1 + dt)\ndt",
    );
}

#[test]
fn a_radical_takes_out_the_powers_of_its_factors() {
    shows(
        "Let three_halves: Rational be 3 / 2.\nLet minus_third: Rational be -1 / 3.\nShow (3 + dt) ** three_halves.\nShow (3 + dt) ** minus_third.\nShow sqrt(3 + dt).\nShow sqrt(2 + 2 * dt).",
        "(3 + dt)·√(3 + dt)\n1/∛(3 + dt)\n√(3 + dt)\n√(2 + 2·dt)",
    );
}

#[test]
fn an_algebraic_number_takes_a_rational_power() {
    shows(
        "Let half: Rational be 1 / 2.\nLet a: Algebraic be 2 ** half.\nShow a.\nLet b: Algebraic be 8 ** (half * 2 / 3).\nShow b.",
        "√2\n2",
    );
}

#[test]
fn a_float_base_keeps_the_float_power() {
    shows(
        "Let half: Rational be 1 / 2.\nShow 2.0 ** half.\nLet f be 2.0.\nShow f ** half.",
        "1.4142135623730951\n1.4142135623730951",
    );
}

// ── integer parts ────────────────────────────────────────────────────────────────────────────

#[test]
fn a_root_s_integer_part_reads_which_side_of_an_integer_it_lies() {
    shows(
        "Let r: Hyperreal be sqrt(dt).\nShow floor(sqrt(1 + dt)).\nShow floor(1 - r).\nShow ceil(r).\nShow floor(sqrt(4 - dt)).\nShow ceil(sqrt(4 - dt)).",
        "1\n0\n1\n1\n2",
    );
}

#[test]
fn an_infinite_root_has_no_integer_part_a_program_holds() {
    fails_with(
        "Let r: Hyperreal be 1 / sqrt(dt).\nShow floor(r).",
        "the integer part of 1/√dt is no polynomial in ⌈1/dt⌉",
    );
}

// ── what has no real root ─────────────────────────────────────────────────────────────────────

#[test]
fn a_negative_hyperreal_has_no_square_root() {
    fails_with("Show sqrt(0 - dt).", "square root of a negative number");
    fails_with("Show sqrt(dt - 1).", "square root of a negative number");
}

#[test]
fn an_odd_root_of_a_negative_hyperreal_is_negative() {
    shows("Let third: Rational be 1 / 3.\nShow (0 - dt) ** third.", "-∛dt");
}

// ── roots inside a supertask ──────────────────────────────────────────────────────────────────

#[test]
fn a_supertask_s_end_state_takes_a_root() {
    shows(
        "Let t be 0.\nLet n be 0.\nWhile t < 1:\n    Set n to n + 1.\n    Set t to t + dt.\nLet r: Hyperreal be sqrt(n).\nShow r.\nShow r * sqrt(dt).",
        "1/√dt\n1",
    );
}

#[test]
fn a_power_past_the_degree_bound_is_refused_in_the_root_it_counts() {
    fails_with(
        "Let r: Hyperreal be 1 + sqrt(dt).\nShow r ** 5000.",
        "the power would have degree 5000 in √dt; one power reaches at most degree 4,096",
    );
}

/// Rational functions of a root of `dt` compute in their own field: six hundred operations on values of
/// degree up to 900 in `√dt` end within the deadline, where a resultant for each operation takes minutes.
#[test]
fn rational_functions_of_a_root_of_dt_compute_in_their_own_field() {
    shows(
        "Let r: Hyperreal be 1 + sqrt(dt).\nLet x: Hyperreal be 1.\nRepeat for i from 1 to 300:\n    Set x to x * r.\n    Set x to x - dt * x.\nShow x > 1.\nShow the standard part of x.",
        "true\n1",
    );
}
