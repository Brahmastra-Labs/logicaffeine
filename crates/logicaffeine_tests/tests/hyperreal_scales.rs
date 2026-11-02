//! Several ordered infinitesimals. `Let dx be an infinitesimal much smaller than dt.` declares `dx`
//! smaller than every positive number made from `dt`, `dt^1000` among them, so the values of the two
//! are the ordered field of rational functions of `dx` over the values of `dt`, and every comparison
//! between them is decided. `much larger than` declares an infinitesimal above another, and both bounds
//! declare one between two. A bare `smaller than` decides too little and is refused, as is any value
//! that combines infinitesimals whose order the program never stated.
//!
//! `relative to dt` asks a question from `dt`'s standpoint, where every value made from `dt` is an
//! ordinary number: `the standard part of dt + dx relative to dt` is `dt`, the inner limit with `dt`
//! held fixed.

mod common;
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(300);

const SCALES: &str = "## Main\nLet dt be an infinitesimal.\nLet dx be an infinitesimal much smaller than dt.\n";

fn shows(body: &str, expected: &str) {
    common::assert_all_tiers_within(&format!("{SCALES}{body}"), expected, DEADLINE)
}

fn fails_with(body: &str, needle: &str) {
    program_fails_with(&format!("{SCALES}{body}"), needle)
}

fn program_shows(source: &str, expected: &str) {
    common::assert_all_tiers_within(source, expected, DEADLINE)
}

fn program_fails_with(source: &str, needle: &str) {
    let out = common::run_all_tiers_within(source, DEADLINE);
    let err = out
        .error
        .unwrap_or_else(|| panic!("expected an error containing {needle:?}; the program printed {:?}", out.output));
    assert!(err.contains(needle), "expected {needle:?} in: {err}");
}

// ── declaring ─────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_much_smaller_infinitesimal_is_a_positive_infinitesimal_of_its_own_name() {
    shows(
        "Show dx.\nShow dx > 0.\nShow dx is infinitesimal.\nShow dx is limited.\nShow dx = dt.",
        "dx\ntrue\ntrue\ntrue\nfalse",
    );
}

#[test]
fn it_lies_below_every_power_of_the_larger() {
    shows(
        "Show dx < dt.\nShow dx < dt ** 2.\nShow dx < dt ** 1000.\nShow 1000000 * dx < dt ** 1000.\nShow dx > dt ** 1000.",
        "true\ntrue\ntrue\ntrue\nfalse",
    );
}

#[test]
fn its_ratio_to_any_power_of_the_larger_is_infinitesimal() {
    shows(
        "Show dx / dt ** 1000 is infinitesimal.\nShow 1 / dx > 1 / dt ** 1000.\nShow 1 / dx is limited.\nShow dt ** 1000 / dx is limited.",
        "true\ntrue\nfalse\nfalse",
    );
}

#[test]
fn a_much_larger_infinitesimal_lies_above_every_root_of_the_smaller() {
    program_shows(
        "## Main\nLet dt be an infinitesimal.\nLet h be an infinitesimal much larger than dt.\nShow dt < h ** 1000.\nShow sqrt(dt) < h.\nShow h is infinitesimal.\nShow h > 0.\nShow h + dt.\nShow dt + h.",
        "true\ntrue\ntrue\ntrue\nh + dt\nh + dt",
    );
}

#[test]
fn an_infinitesimal_declared_between_two_lies_between_them() {
    program_shows(
        "## Main\nLet dt be an infinitesimal.\nLet dx be an infinitesimal much smaller than dt.\nLet dy be an infinitesimal much smaller than dt and much larger than dx.\nShow dx < dy ** 1000.\nShow dy < dt ** 1000.\nShow dx + dy + dt.\nShow dt - dy - dx.",
        "true\ntrue\ndt + dy + dx\ndt - dy - dx",
    );
}

#[test]
fn the_bounds_of_a_between_declaration_read_in_either_order() {
    program_shows(
        "## Main\nLet dt be an infinitesimal.\nLet dx be an infinitesimal much smaller than dt.\nLet dy be an infinitesimal much larger than dx and much smaller than dt.\nShow dx < dy ** 1000.\nShow dy < dt ** 1000.",
        "true\ntrue",
    );
}

#[test]
fn an_order_runs_through_every_declaration_between() {
    program_shows(
        "## Main\nLet a be an infinitesimal.\nLet b be an infinitesimal much smaller than a.\nLet c be an infinitesimal much smaller than b.\nLet d be an infinitesimal much smaller than c.\nShow d < a ** 1000.\nShow a + d.\nShow c * d < b ** 1000 * d.",
        "true\na + d\ntrue",
    );
}

#[test]
fn an_infinitesimal_declared_above_later_orders_every_value_made_before() {
    program_shows(
        "## Main\nLet dt be an infinitesimal.\nLet x be 1 + dt.\nLet h be an infinitesimal much larger than dt.\nShow (h + x) * (h - x).\nShow x < 1 + h.",
        "-1 + h^2 - 2·dt - dt^2\ntrue",
    );
}

#[test]
fn a_much_smaller_infinitesimal_takes_facts_about_its_reciprocal() {
    shows(
        "Let dy be an infinitesimal much smaller than dx whose reciprocal is even.\nShow floor(1 / dy) % 2.\nShow dy < dx ** 1000.",
        "0\ntrue",
    );
}

// ── arithmetic ────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_value_prints_by_the_powers_of_its_smallest_infinitesimal() {
    shows(
        "Show dt + dx.\nShow dx + dt.\nShow dt - dx.\nShow dx - dt.\nShow 3 * dt * dx.\nShow (1 + dt) * dx.\nShow 0 - (1 + dt) * dx.\nShow dt * dx - dx * dx.",
        "dt + dx\ndt + dx\ndt - dx\n-dt + dx\n3·dt·dx\n(1 + dt)·dx\n-(1 + dt)·dx\ndt·dx - dx^2",
    );
}

#[test]
fn what_cancels_an_infinitesimal_leaves_the_others() {
    shows(
        "Show (dt + dx) - dx.\nShow (dt + dx) - dt.\nShow (dt + dx) - dt - dx.\nShow (dx + dt * dx) / dx.\nShow dx * dt / dx.",
        "dt\ndx\n0\n1 + dt\ndt",
    );
}

#[test]
fn a_quotient_prints_as_a_fraction_of_both() {
    shows(
        "Show dx / dt.\nShow dt / dx.\nShow 1 / (dt + dx).\nShow dx / (2 * dt).\nShow (dt + dx) / (dt - dx).\nShow dx * dx / dt.\nShow 1 / (dt * dx).\nShow (1 + dx) / (1 + dt).",
        "dx/dt\ndt/dx\n1/(dt + dx)\ndx/(2·dt)\n(dt + dx)/(dt - dx)\ndx^2/dt\n1/(dt·dx)\n(1 + dx)/(1 + dt)",
    );
}

#[test]
fn a_negative_fraction_keeps_its_denominator_positive() {
    shows(
        "Show 1 / (dx - dt).\nShow dx / (0 - dt).",
        "-1/(dt - dx)\n-dx/dt",
    );
}

#[test]
fn products_and_powers_multiply_out() {
    shows(
        "Show (dt + dx) * (dt - dx).\nShow (dt + dx) * (dt + dx).\nShow (dt + dx) ** 3.\nShow 1 / (dt + dx) ** 2.",
        "dt^2 - dx^2\ndt^2 + 2·dt·dx + dx^2\ndt^3 + 3·dt^2·dx + 3·dt·dx^2 + dx^3\n1/(dt^2 + 2·dt·dx + dx^2)",
    );
}

#[test]
fn roots_of_each_infinitesimal_meet_the_other() {
    shows(
        "Show sqrt(dt) + dx.\nShow sqrt(dt) * dx.\nShow sqrt(dt) > dx.\nShow sqrt(dx) + dt.\nShow sqrt(dx) < dt ** 1000.\nShow sqrt(1 + dt) + dx.\nShow sqrt(1 + dt) + dx > 1.",
        "√dt + dx\n√dt·dx\ntrue\ndt + √dx\ntrue\n√(1 + dt) + dx\ntrue",
    );
}

#[test]
fn a_rational_power_of_a_product_of_powers_is_exact() {
    shows(
        "Let half: Rational be 1 / 2.\nShow (dt * dx) ** half.\nShow (4 * dt * dt * dx) ** half.\nShow ((dt * dx) ** half) ** 2.",
        "√dt·√dx\n2·dt·√dx\ndt·dx",
    );
}

#[test]
fn a_value_of_both_is_a_hyperreal() {
    shows(
        "Let y: Hyperreal be dt + dx.\nShow y.\nSet y to y * y.\nShow y.",
        "dt + dx\ndt^2 + 2·dt·dx + dx^2",
    );
}

// ── the real closure of several infinitesimals ────────────────────────────────────────────────

#[test]
fn a_root_of_a_value_of_both_is_held_exactly() {
    shows(
        "Show sqrt(dt + dx).\nShow sqrt(dt + dx) * sqrt(dt + dx).\nShow sqrt(dt + dx) > sqrt(dt).\nShow sqrt(dt + dx) is infinitesimal.",
        "√(dt + dx)\ndt + dx\ntrue\ntrue",
    );
}

#[test]
fn the_root_of_a_square_is_the_value_squared() {
    shows(
        "Show sqrt(dt * dt + 2 * dt * dx + dx * dx).\nShow sqrt(4 * dt * dt + 4 * dt * dx + dx * dx).",
        "dt + dx\n2·dt + dx",
    );
}

#[test]
fn equal_roots_reached_two_ways_are_equal() {
    shows(
        "Show sqrt(dt + dx) * sqrt(dt - dx) = sqrt(dt * dt - dx * dx).\nShow sqrt(dt + dx) = sqrt(dx + dt).\nShow sqrt(dt + dx) - sqrt(dx + dt).",
        "true\ntrue\n0",
    );
}

#[test]
fn a_root_prints_as_a_radical_of_its_power() {
    shows(
        "Let third: Rational be 1 / 3.\nShow (dt + dx) ** third.\nShow 2 * sqrt(dt + dx).\nShow 0 - sqrt(dt + dx).\nShow 1 / sqrt(dt + dx).",
        "∛(dt + dx)\n2·√(dt + dx)\n-√(dt + dx)\n1/√(dt + dx)",
    );
}

#[test]
fn a_root_of_the_smaller_beside_the_larger_is_its_expansion() {
    shows(
        "Show sqrt(1 + dx) + dt.\nShow sqrt(1 + dx) + dt > 1 + dt.\nShow (sqrt(dt + dx) + sqrt(dt - dx)) ** 2.",
        "1 + dt + (1/2)·dx + O(dx^2)\ntrue\n4·dt - (1/dt)·dx^2 + O(dx^3)",
    );
}

/// The derivative of `√t` at `t = dt`, read as the inner limit: the difference quotient with step `dx`,
/// its standard part relative to `dt`.
#[test]
fn a_derivative_is_the_standard_part_of_a_difference_quotient_relative_to_the_larger() {
    shows(
        "Let q be (sqrt(dt + dx) - sqrt(dt)) / dx.\nShow q.\nShow the standard part of q relative to dt.\nShow q is not limited.",
        "1/(2·√dt) - (1/(8·dt^(3/2)))·dx + O(dx^2)\n1/(2·√dt)\ntrue",
    );
}

#[test]
fn a_root_compares_and_reads_its_standard_parts() {
    shows(
        "Show sqrt(dt + dx) + sqrt(dt - dx) < 2 * sqrt(dt).\nShow the standard part of sqrt(1 + dt + dx).\nShow the standard part of sqrt(1 + dt + dx) relative to dt.\nShow sqrt(dx) / sqrt(dt + dx) is infinitesimal relative to dt.\nShow floor(sqrt(4 + dx) - dt).",
        "true\n1\n√(1 + dt)\ntrue\n1",
    );
}

#[test]
fn a_root_beside_a_much_larger_infinitesimal() {
    program_shows(
        "## Main\nLet dt be an infinitesimal.\nLet h be an infinitesimal much larger than dt.\nShow sqrt(h + dt).\nShow sqrt(h + dt) > sqrt(h).\nShow sqrt(1 + dt) + h.",
        "√(h + dt)\ntrue\n1 + h + (1/2)·dt + O(dt^2)",
    );
}

/// A root beside a value infinite in the smaller infinitesimal prints by its leading terms, and a power
/// of an infinite integer by its radical: a value of degree two is a radical only when its polynomial is
/// `z² − c`, and a power of a binomial is read off.
#[test]
fn a_root_beside_an_infinite_value_prints_by_its_leading_terms() {
    shows(
        "Show sqrt(dt + dx) + 1 / dx.\nShow sqrt(dt + dx) + (1 / (2 * dt) + 1 / dx - 1 / 2).\nShow sqrt(1 / dt + 1 / dx).\nLet half: Rational be 7 / 2.\nShow (1 / dt + 1 / dx) ** half > 1 / dx ** 3.",
        "1/dx + √dt + O(dx)\n1/dx + (1 - dt + 2·dt^(3/2))/(2·dt) + O(dx)\n√(1/dx + 1/dt)\ntrue",
    );
}

#[test]
fn an_even_root_of_a_negative_value_of_both_is_refused() {
    fails_with("Show sqrt(dx - dt).", "square root of a negative number");
}

// ── functions of several infinitesimals ───────────────────────────────────────────────────────

/// `exp(dt + dx) = e^dt·e^dx`: its expansion in `dx`, each coefficient the germ of `e^dt`.
#[test]
fn the_exponential_of_a_value_of_both_expands_in_the_smaller() {
    shows(
        "Let x: Hyperreal be exp(dt + dx).\nShow x.\nShow x > exp(dt).\nShow the standard part of x relative to dt.\nShow the standard part of x.\nShow x - exp(dt) is infinitesimal relative to dt.",
        "(1 + dt + O(dt^2)) + (1 + dt + O(dt^2))·dx + O(dx^2)\ntrue\n1 + dt + O(dt^2)\n1\ntrue",
    );
}

#[test]
fn the_logarithm_of_a_value_of_both_expands_in_the_smaller() {
    shows(
        "Let x: Hyperreal be ln(1 + dt + dx).\nShow x.\nShow x < ln(1 + dt) + dx.\nShow the standard part of (x - ln(1 + dt)) / dx relative to dt.",
        "(dt - (1/2)·dt^2 + O(dt^3)) + (1/(1 + dt))·dx + O(dx^2)\ntrue\n1/(1 + dt)",
    );
}

#[test]
fn a_function_of_a_value_of_both_that_is_no_germ_is_refused() {
    fails_with("Show ln(dt + dx).", "the logarithm of dt + dx carries ln(dt)");
    fails_with("Show exp(1 / dx + dt).", "the exponential of (1 + dt·dx)/dx is beyond every power of dx");
}

// ── supertasks across infinitesimals ──────────────────────────────────────────────────────────

/// A loop stepped by `dx` whose rate is `dt`: `(1 + dt·dx)^⌈1/dx⌉`, whose standard part relative to
/// `dt` is `e^dt` — the inner limit taken with `dt` held fixed.
#[test]
fn a_flow_stepped_by_the_smaller_with_a_rate_of_the_larger() {
    shows(
        "Let x: Hyperreal be 1.\nLet s: Hyperreal be 0.\nWhile s < 1:\n    Set x to x + x * dt * dx.\n    Set s to s + dx.\nShow the standard part of x relative to dt.\nShow the standard part of x.\nShow x > 1 + dt.",
        "1 + dt + O(dt^2)\n1\ntrue",
    );
}

/// A loop stepped by `dt` whose rate is `dx`: `(1 + dx)^⌈1/dt⌉`, exactly the binomial series in `dx`
/// with coefficients polynomials in `1/dt`.
#[test]
fn a_flow_stepped_by_the_larger_with_a_rate_of_the_smaller() {
    shows(
        "Let x: Hyperreal be 1.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Set x to x + x * dx.\n    Set t to t + dt.\nShow x.\nShow the standard part of x relative to dt.\nShow the standard part of (x - 1) / dx relative to dt.\nShow x > 1.",
        "1 + (1/dt)·dx + O(dx^2)\n1\n1/dt\ntrue",
    );
}

/// An oscillator stepped by `dx` whose frequency is `1 + dt`: the exponential of a matrix of values
/// of `dt`, read one order of `dt` at a time, ends at `cos(1 + dt)`.
#[test]
fn a_flow_stepped_by_the_smaller_whose_rates_form_a_matrix_of_the_larger() {
    shows(
        "Let w: Hyperreal be 1 + dt.\nLet x: Hyperreal be 1.\nLet v: Hyperreal be 0.\nLet s: Hyperreal be 0.\nWhile s < 1:\n    Let a be 0 - w * w * x.\n    Set x to x + v * dx.\n    Set v to v + a * dx.\n    Set s to s + dx.\nShow the standard part of x.\nShow the standard part of x relative to dt.\nShow the standard part of v relative to dt.",
        "cos(1)\ncos(1) - sin(1)·dt + O(dt^2)\n-sin(1) + (-cos(1) - sin(1))·dt + O(dt^2)",
    );
}

/// A loop stepped by `dt` growing by `dt` and by `dx` at once: `(1 + dt + dx)^⌈1/dt⌉`, whose standard part
/// relative to `dt` is `(1 + dt)^(1/dt)`.
#[test]
fn a_flow_stepped_by_the_larger_with_rates_of_both() {
    shows(
        "Let x: Hyperreal be 1.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Set x to x + x * dt + x * dx.\n    Set t to t + dt.\nShow the standard part of x relative to dt.\nShow the standard part of x.",
        "e - (1/2)·e·dt + O(dt^2)\ne",
    );
}

#[test]
fn a_nonlinear_flow_with_a_rate_of_another_infinitesimal_is_refused() {
    fails_with(
        "Let x: Hyperreal be 1.\nLet s: Hyperreal be 0.\nWhile s < 1:\n    Set x to x + x * x * dt * dx.\n    Set s to s + dx.\nShow x.",
        "this supertask steps by dx and its update holds dt: a nonlinear flow is integrated over rates that are standard numbers",
    );
}

/// An iterated limit: each pass of a loop stepped by `dt` runs a loop stepped by `dx` that adds
/// `t·dx` for every `dx`, so a pass adds `t·dt`, and the outer loop sums `∫₀¹ t dt`.
#[test]
fn a_supertask_in_each_pass_of_a_supertask_over_a_larger_infinitesimal() {
    shows(
        "Let total: Hyperreal be 0.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Let y: Hyperreal be 0.\n    Let s: Hyperreal be 0.\n    While s < 1:\n        Set y to y + t * dx.\n        Set s to s + dx.\n    Set total to total + y * dt.\n    Set t to t + dt.\nShow the standard part of total.",
        "1/2",
    );
}

/// A pass computes a value its supertask reads: each pass sums its own `t·t` over every `dx`, and the
/// outer loop sums `∫₀¹ t² dt`.
#[test]
fn a_supertask_in_each_pass_reads_a_value_the_pass_computes() {
    shows(
        "Let total: Hyperreal be 0.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Let u: Hyperreal be t * t.\n    Let y: Hyperreal be 0.\n    Let s: Hyperreal be 0.\n    While s < 1:\n        Set y to y + u * dx.\n        Set s to s + dx.\n    Set total to total + y * dt.\n    Set t to t + dt.\nShow the standard part of total.",
        "1/3",
    );
}

/// A supertask in each pass that is a polynomial in what the passes change: each pass adds `t²·dt`,
/// and the outer loop sums `∫₀¹ t² dt`.
#[test]
fn a_supertask_in_each_pass_polynomial_in_the_outer_clock() {
    shows(
        "Let total: Hyperreal be 0.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Let y: Hyperreal be 0.\n    Let s: Hyperreal be 0.\n    While s < 1:\n        Set y to y + t * t * dx.\n        Set s to s + dx.\n    Set total to total + y * dt.\n    Set t to t + dt.\nShow the standard part of total.\nShow the standard part of total relative to dt.",
        "1/3\n1/3 - (1/2)·dt + (1/6)·dt^2",
    );
}

/// A supertask in each pass that is no affine map of its own state, but whose monomials close: from
/// `z = t` it sums `(t + s)²` over `s` in `[0, 1]`, `t² + t + 1/3`, and the outer loop sums that to `7/6`.
#[test]
fn a_supertask_in_each_pass_whose_own_square_closes() {
    shows(
        "Let total: Hyperreal be 0.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Let y: Hyperreal be 0.\n    Let z: Hyperreal be t.\n    Let s: Hyperreal be 0.\n    While s < 1:\n        Set y to y + z * z * dx.\n        Set z to z + dx.\n        Set s to s + dx.\n    Set total to total + y * dt.\n    Set t to t + dt.\nShow the standard part of total.",
        "7/6",
    );
}

/// Two supertasks in each pass, the second reading what the first leaves: `∫₀¹ 2t dt`.
#[test]
fn the_supertasks_of_one_pass_compose_in_order() {
    shows(
        "Let total: Hyperreal be 0.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Let y: Hyperreal be 0.\n    Let s: Hyperreal be 0.\n    While s < 1:\n        Set y to y + t * dx.\n        Set s to s + dx.\n    Let z: Hyperreal be 0.\n    Let r: Hyperreal be 0.\n    While r < 1:\n        Set z to z + 2 * y * dx.\n        Set r to r + dx.\n    Set total to total + z * dt.\n    Set t to t + dt.\nShow the standard part of total.",
        "1",
    );
}

/// Three infinitesimals, each much smaller than the last, nest three supertasks, the innermost reading
/// the clocks of both loops around it: `∫₀¹∫₀¹ (s + t) ds dt`.
#[test]
fn three_infinitesimals_nest_three_supertasks() {
    program_shows(
        "## Main\nLet dt be an infinitesimal.\nLet dx be an infinitesimal much smaller than dt.\nLet dy be an infinitesimal much smaller than dx.\nLet total: Hyperreal be 0.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Let y: Hyperreal be 0.\n    Let s: Hyperreal be 0.\n    While s < 1:\n        Let w: Hyperreal be 0.\n        Let r: Hyperreal be 0.\n        While r < 1:\n            Set w to w + (s + t) * dy.\n            Set r to r + dy.\n        Set y to y + w * dx.\n        Set s to s + dx.\n    Set total to total + y * dt.\n    Set t to t + dt.\nShow the standard part of total.\nShow the standard part of total relative to dt.",
        "1\n1 - (1/2)·dt",
    );
}

/// The loops need not nest in the order of their infinitesimals: each pass of a loop stepped by `dx`
/// runs a supertask stepped by the larger `dt`.
#[test]
fn a_supertask_over_the_larger_in_each_pass_of_one_over_the_smaller() {
    shows(
        "Let total: Hyperreal be 0.\nLet s: Hyperreal be 0.\nWhile s < 1:\n    Let y: Hyperreal be 0.\n    Let t: Hyperreal be 0.\n    While t < 1:\n        Set y to y + s * dt.\n        Set t to t + dt.\n    Set total to total + y * dx.\n    Set s to s + dx.\nShow the standard part of total.",
        "1/2",
    );
}

/// A supertask that runs no steps leaves its pass as it was and computes nothing of its body — here a
/// rate that divides by zero.
#[test]
fn a_supertask_of_no_steps_in_each_pass_computes_nothing_of_its_body() {
    shows(
        "Let k: Hyperreal be 0.\nLet total: Hyperreal be 0.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Let y: Hyperreal be t.\n    Let s: Hyperreal be 0.\n    While s < 0:\n        Set y to y + t / k * dx.\n        Set s to s + dx.\n    Set total to total + y * dt.\n    Set t to t + dt.\nShow the standard part of total.",
        "1/2",
    );
}

/// A count that depends on the residue of `1/dx`: `⌈1/(2·dx)⌉` steps of `dx` reach `1/2` or `1/2 + dx/2`,
/// so each pass adds `t·dt/2` up to a value infinitesimal relative to `dt`.
#[test]
fn a_supertask_in_each_pass_whose_count_depends_on_the_residue() {
    shows(
        "Let total: Hyperreal be 0.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Let y: Hyperreal be 0.\n    Let s: Hyperreal be 0.\n    While s < 1 / 2:\n        Set y to y + t * dx.\n        Set s to s + dx.\n    Set total to total + y * dt.\n    Set t to t + dt.\nShow the standard part of total.",
        "1/4",
    );
}

#[test]
fn a_supertask_in_each_pass_with_no_exact_answer_of_its_own_is_refused_with_its_reason() {
    fails_with(
        "Let total: Hyperreal be 0.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Let y: Hyperreal be 0.\n    Let s: Hyperreal be 0.\n    While s < 1:\n        Set y to y + t * dx.\n        Show y.\n        Set s to s + dx.\n    Set total to total + y * dt.\n    Set t to t + dt.\nShow total.",
        "the supertask in each of its passes has no exact answer: it does more than assign variables at each step",
    );
}

#[test]
fn a_supertask_in_each_pass_counted_from_a_value_its_pass_changes_is_refused() {
    fails_with(
        "Let total: Hyperreal be 0.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Let y: Hyperreal be 0.\n    Let s: Hyperreal be 0.\n    While s < t:\n        Set y to y + dx.\n        Set s to s + dx.\n    Set total to total + y * dt.\n    Set t to t + dt.\nShow total.",
        "the supertask in each of its passes counts its steps from a value its pass changes",
    );
}

/// A second supertask that counts on from where the first left its clock reads a value the pass
/// changed, which it is not counted from: refused, never counted from the clock's first value.
#[test]
fn a_supertask_counted_from_where_another_left_its_clock_is_refused() {
    fails_with(
        "Let total: Hyperreal be 0.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Let y: Hyperreal be 0.\n    Let z: Hyperreal be 0.\n    Let s: Hyperreal be 0.\n    While s < 1:\n        Set y to y + t * dx.\n        Set s to s + dx.\n    While s < 2:\n        Set z to z + t * dx.\n        Set s to s + dx.\n    Set total to total + (y + z) * dt.\n    Set t to t + dt.\nShow total.",
        "the supertask in each of its passes counts its steps from a value its pass changes",
    );
}

#[test]
fn a_supertask_in_each_pass_whose_map_changes_from_pass_to_pass_is_refused() {
    fails_with(
        "Let total: Hyperreal be 0.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Let y: Hyperreal be 1.\n    Let s: Hyperreal be 0.\n    While s < 1:\n        Set y to y + t * y * dx.\n        Set s to s + dx.\n    Set total to total + y * dt.\n    Set t to t + dt.\nShow total.",
        "the supertask in each of its passes reads `t`, which its passes change, and its end state is no polynomial in it",
    );
}

#[test]
fn a_supertask_in_each_pass_under_a_limit_rule_is_refused() {
    fails_with(
        "At the limit, take the standard part.\nLet total: Hyperreal be 0.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Let y: Hyperreal be 0.\n    Let s: Hyperreal be 0.\n    While s < 1:\n        Set y to y + t * dx.\n        Set s to s + dx.\n    Set total to total + y * dt.\n    Set t to t + dt.\nShow total.",
        "the supertask in each of its passes takes its limit rule in every pass",
    );
}

#[test]
fn a_supertask_in_a_branch_of_each_pass_is_refused() {
    fails_with(
        "Let total: Hyperreal be 0.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Let y: Hyperreal be 0.\n    Let s: Hyperreal be 0.\n    If t < 1 / 2:\n        While s < 1:\n            Set y to y + t * dx.\n            Set s to s + dx.\n    Set total to total + y * dt.\n    Set t to t + dt.\nShow total.",
        "a supertask runs in a branch of its pass",
    );
}

// ── comparisons ───────────────────────────────────────────────────────────────────────────────

#[test]
fn every_comparison_of_values_of_both_is_decided() {
    shows(
        "Show dt - dx > 0.\nShow dx - dt * dt < 0.\nShow dt * dx < dx.\nShow dt * dx > dx * dx.\nShow (dt + dx) * (dt + dx) = dt * dt + 2 * dt * dx + dx * dx.\nShow dt + dx = dt.\nShow dt + dx > dt.\nShow dt + dx >= dt.\nShow dt + dx != dt.",
        "true\ntrue\ntrue\ntrue\ntrue\nfalse\ntrue\ntrue\ntrue",
    );
}

#[test]
fn a_value_of_the_smaller_compares_with_a_standard_number() {
    shows(
        "Show 1 + dx > 1.\nShow 1 - dx < 1.\nShow dx < 1 / 1000000.\nShow 0 - dx < 0.",
        "true\ntrue\ntrue\ntrue",
    );
}

#[test]
fn a_list_finds_a_value_of_both_by_its_value() {
    shows(
        "Let xs be [dt + dx, dt * dx].\nShow xs contains (dx + dt).\nShow xs contains (dx * dt).\nShow xs contains dt.",
        "true\ntrue\nfalse",
    );
}

/// A list finds a root of both by its value: built another way it is found, and a root that differs
/// from it — at the first order, or only at the tenth — is not.
#[test]
fn a_list_finds_a_root_of_both_by_its_value() {
    shows(
        "Let r: Hyperreal be sqrt(dt + dx).\nLet same: Hyperreal be sqrt(dx + dt).\nLet doubled: Hyperreal be sqrt(dt + 2 * dx).\nLet xs be [r, r + dx ** 10].\nShow xs contains same.\nShow xs contains doubled.\nShow xs contains (r + dx ** 11).\nShow xs contains ((r + 1) - 1).",
        "true\nfalse\nfalse\ntrue",
    );
}

// ── the standard part and the predicates ──────────────────────────────────────────────────────

#[test]
fn the_standard_part_reads_through_both() {
    shows(
        "Show the standard part of 3 + dt + dx.\nShow the standard part of (dx + dt * dx) / dx.\nShow the standard part of dx / dt.\nShow the standard part of (2 * dt + dx) / (dt - dx).",
        "3\n1\n0\n2",
    );
}

#[test]
fn the_predicates_read_through_both() {
    shows(
        "Show dx / dt is infinitesimal.\nShow (dt + dx) / dt is limited.\nShow dt / dx is limited.\nShow dt / dx is infinitesimal.\nShow dt + dx is infinitely close to dt.\nShow 1 + dx is infinitely close to 1.\nShow 1 + dt is infinitely close to 1 + dx.",
        "true\ntrue\nfalse\nfalse\ntrue\ntrue\ntrue",
    );
}

#[test]
fn an_infinite_value_of_both_has_no_standard_part() {
    fails_with("Show the standard part of dt / dx.", "dt/dx is infinite and has no standard part");
}

// ── relative to an infinitesimal ──────────────────────────────────────────────────────────────

#[test]
fn the_standard_part_relative_to_the_larger_keeps_it() {
    shows(
        "Show the standard part of dt + dx relative to dt.\nShow the standard part of (dt + dx) / dt relative to dt.\nShow the standard part of 3 + dt * dt + dt * dx relative to dt.\nShow the standard part of dt relative to dt.\nShow the standard part of 5 relative to dt.",
        "dt\n1\n3 + dt^2\ndt\n5",
    );
}

#[test]
fn infinitesimal_relative_to_the_larger() {
    shows(
        "Show dx is infinitesimal relative to dt.\nShow dt * dt is infinitesimal relative to dt.\nShow dx / dt ** 1000 is infinitesimal relative to dt.\nShow dt + dx is infinitesimal relative to dt.\nShow dt is infinitesimal relative to dt.\nShow dx is not infinitesimal relative to dt.",
        "true\nfalse\ntrue\nfalse\nfalse\nfalse",
    );
}

#[test]
fn limited_relative_to_the_larger() {
    shows(
        "Show 1 / dt is limited relative to dt.\nShow dt / dx is limited relative to dt.\nShow (dt + dx) / dx is limited relative to dt.\nShow 1 / dt ** 1000 + dx is limited relative to dt.\nShow dt / dx is not limited relative to dt.",
        "true\nfalse\nfalse\ntrue\ntrue",
    );
}

#[test]
fn infinitely_close_relative_to_the_larger() {
    shows(
        "Show dt + dx is infinitely close to dt relative to dt.\nShow dt is infinitely close to dt + dt * dt relative to dt.\nShow dt is infinitely close to dt + dt * dt.\nShow dt is not infinitely close to dt + dt * dt relative to dt.",
        "true\nfalse\ntrue\ntrue",
    );
}

#[test]
fn relative_to_the_smaller_every_value_of_both_is_ordinary() {
    shows(
        "Show the standard part of dt + dx relative to dx.\nShow dx is infinitesimal relative to dx.\nShow 1 / dx is limited relative to dx.\nShow 0 is infinitesimal relative to dx.",
        "dt + dx\nfalse\ntrue\ntrue",
    );
}

#[test]
fn relative_to_a_much_larger_infinitesimal_the_smaller_ones_vanish() {
    program_shows(
        "## Main\nLet dt be an infinitesimal.\nLet h be an infinitesimal much larger than dt.\nShow the standard part of h + dt relative to h.\nShow dt is infinitesimal relative to h.\nShow h is infinitesimal relative to dt.",
        "h\ntrue\nfalse",
    );
}

#[test]
fn an_infinite_value_has_no_standard_part_relative_to_the_larger() {
    fails_with(
        "Show the standard part of dt / dx relative to dt.",
        "dt/dx is infinite relative to dt and has no standard part relative to it",
    );
}

#[test]
fn relative_to_names_an_infinitesimal_itself() {
    fails_with(
        "Show dx is infinitesimal relative to 2 * dt.",
        "`relative to` names an infinitesimal itself, not 2·dt",
    );
}

#[test]
fn relative_to_an_unordered_infinitesimal_is_refused() {
    program_fails_with(
        "## Main\nLet dt be an infinitesimal.\nLet h be an infinitesimal.\nShow the standard part of 1 + dt relative to h.",
        "cannot read a value of dt relative to h, whose order the program does not state; declare one much smaller than the other",
    );
}

// ── integer parts ─────────────────────────────────────────────────────────────────────────────

#[test]
fn the_integer_part_of_a_limited_value_of_both() {
    shows(
        "Show floor(1 + dt + dx).\nShow floor(1 - dx).\nShow ceil(1 - dx).\nShow ceil(1 + dx).\nShow round(1 / 2 - dx).\nShow round(1 / 2 + dx).\nShow floor(dt - dx).",
        "1\n0\n1\n2\n0\n1\n0",
    );
}

/// The integer part of a value infinite in both is an infinite integer of both, `⌈1/dt⌉ + ⌈1/dx⌉` and
/// `⌈1/dt⌉·⌈1/dx⌉` among them, printed as an infinite integer of one is: a sum of reciprocals.
#[test]
fn the_integer_part_of_a_value_infinite_in_both() {
    shows(
        "Show floor(1 / dt + 1 / dx).\nShow floor(1 / (dt * dx)).\nShow floor(1 / dx + dt).\nShow floor(1 / dx - dt).\nShow floor(1 / dt + dx).\nShow floor(1 / dt - dx).\nShow ceil(1 / dx - dt).",
        "1/dx + 1/dt\n1/(dt·dx)\n1/dx\n1/dx - 1\n1/dt\n1/dt - 1\n1/dx",
    );
}

/// Integer parts read on different classes of `⌈1/dt⌉` add on the classes both read:
/// `⌊1/(2·dt) + 1/dx⌋ + ⌊1/(3·dt) + 1/dx⌋` is `2/dx + 5/(6·dt)` less what each floor drops on its class,
/// `(h mod 2)/2 + (h mod 3)/3` on the class `h` modulo 6 — an infinite integer on each.
#[test]
fn integer_parts_on_two_classes_add_on_the_classes_both_read() {
    shows(
        "Show floor(1 / (2 * dt) + 1 / dx) + floor(1 / (3 * dt) + 1 / dx).",
        "2/dx + 5/(6·dt) when ⌈1/dt⌉ ≡ 0 (mod 6), 2/dx + 5/(6·dt) - 5/6 when ⌈1/dt⌉ ≡ 1 (mod 6), \
         2/dx + 5/(6·dt) - 2/3 when ⌈1/dt⌉ ≡ 2 (mod 6), 2/dx + 5/(6·dt) - 1/2 when ⌈1/dt⌉ ≡ 3 (mod 6), \
         2/dx + 5/(6·dt) - 1/3 when ⌈1/dt⌉ ≡ 4 (mod 6), 2/dx + 5/(6·dt) - 7/6 when ⌈1/dt⌉ ≡ 5 (mod 6)",
    );
}

/// A fraction of a reciprocal is decided by the reciprocal's residue, so the integer part depends on the
/// residues it needs, the coarser outside: on each class of both, the fraction `1/(3·dx) + 1/(2·dt)` leaves.
#[test]
fn the_integer_part_reads_the_residues_its_fraction_needs() {
    shows(
        "Show floor(1 / (2 * dt) + 1 / dx).\nShow ceil(1 / (2 * dx) + 1 / dt).\nShow round(1 / (2 * dt) + 1 / dx).\nShow floor(1 / (dt * dx) + 1 / (2 * dt)).\nShow floor(1 / (3 * dx) + 1 / (2 * dt)).",
        "1/dx + 1/(2·dt) when ⌈1/dt⌉ is even, 1/dx + 1/(2·dt) - 1/2 when it is odd\n\
         1/(2·dx) + 1/dt when ⌈1/dx⌉ is even, 1/(2·dx) + 1/dt + 1/2 when it is odd\n\
         1/dx + 1/(2·dt) when ⌈1/dt⌉ is even, 1/dx + 1/(2·dt) + 1/2 when it is odd\n\
         1/(dt·dx) + 1/(2·dt) when ⌈1/dt⌉ is even, 1/(dt·dx) + 1/(2·dt) - 1/2 when it is odd\n\
         (1/(3·dx) + 1/(2·dt) when ⌈1/dx⌉ ≡ 0 (mod 3), 1/(3·dx) + 1/(2·dt) - 1/3 when ⌈1/dx⌉ ≡ 1 (mod 3), \
         1/(3·dx) + 1/(2·dt) - 2/3 when ⌈1/dx⌉ ≡ 2 (mod 3)) when ⌈1/dt⌉ is even, \
         (1/(3·dx) + 1/(2·dt) - 1/2 when ⌈1/dx⌉ ≡ 0 (mod 3), 1/(3·dx) + 1/(2·dt) - 5/6 when ⌈1/dx⌉ ≡ 1 (mod 3), \
         1/(3·dx) + 1/(2·dt) - 1/6 when ⌈1/dx⌉ ≡ 2 (mod 3)) when it is odd",
    );
}

/// An infinite integer of both stays one through the integer operations: its remainder reads both
/// parities, what cancels one reciprocal leaves an infinite integer of the other, and its half is a
/// hyperreal unless its integer part is taken.
#[test]
fn an_infinite_integer_of_both_computes_as_an_integer() {
    shows(
        "Let a be floor(1 / dt + 1 / dx).\nShow a % 2.\nShow a - floor(1 / dx).\nShow a * a.\nShow a / 2.\nShow floor(a / 2) = (a - a % 2) / 2.\nShow floor(1 / (dt * dx)) % 2.\nShow floor(1 / (2 * dt)) + floor(1 / dx).\nShow a = floor(1 / dx) + floor(1 / dt).\nShow a > floor(1 / dx).",
        "(0 when ⌈1/dx⌉ is even, 1 when it is odd) when ⌈1/dt⌉ is even, (1 when ⌈1/dx⌉ is even, 0 when it is odd) when it is odd\n\
         1/dt\n\
         1/dx^2 + 2/(dt·dx) + 1/dt^2\n\
         (dt + dx)/(2·dt·dx)\n\
         true\n\
         0 when ⌈1/dt⌉ is even, (0 when ⌈1/dx⌉ is even, 1 when it is odd) when it is odd\n\
         1/dx + 1/(2·dt) when ⌈1/dt⌉ is even, 1/dx + 1/(2·dt) - 1/2 when it is odd\n\
         true\n\
         true",
    );
}

/// The integer part of a germ of both: the floor of what it is constant in `dx`, one less where the rest
/// lies below.
#[test]
fn the_integer_part_of_a_germ_of_both() {
    shows(
        "Show floor(exp(dt) + 1 / dx).\nShow floor(1 / dx + exp(dt) - 1).\nShow floor(1 / dx + 1 - exp(dt)).",
        "1/dx + 1\n1/dx\n1/dx - 1",
    );
}

#[test]
fn an_integer_part_that_no_residue_decides_is_refused() {
    fails_with(
        "Show floor(dt / dx).",
        "the integer part of dt/dx is no polynomial in ⌈1/dt⌉ and ⌈1/dx⌉, the infinite integers a program holds: it multiplies a power of 1/dx by dt, which is no polynomial in the reciprocals of the infinitesimals larger than dx with rational coefficients",
    );
    fails_with(
        "Show floor(sqrt(1 / dx) + 1 / dt).",
        "the integer part of (dt + √dx)/(dt·√dx) is no polynomial in ⌈1/dt⌉ and ⌈1/dx⌉, the infinite integers a program holds: it holds a root of 1/dx",
    );
}

// ── supertasks whose count is infinite in both ───────────────────────────────────────────────

/// A clock stepped by `dt·dx` runs `⌈1/dt⌉·⌈1/dx⌉` times; so does one stepped by `dx` to `1/dt`.
#[test]
fn a_clock_stepped_by_a_product_counts_the_product() {
    shows(
        "Let t: Hyperreal be 0.\nLet n be 0.\nWhile t < 1:\n    Set n to n + 1.\n    Set t to t + dt * dx.\nShow n.\nShow t.\nLet u: Hyperreal be 0.\nLet m be 0.\nWhile u < 1 / dt:\n    Set m to m + 1.\n    Set u to u + dx.\nShow m.\nShow u.",
        "1/(dt·dx)\n1\n1/(dt·dx)\n1/dt",
    );
}

/// A standard step to a bound infinite in both: `⌈1/dt⌉ + ⌈1/dx⌉` steps, and the sum of the clock over
/// them their triangular number.
#[test]
fn a_standard_step_to_a_bound_infinite_in_both() {
    shows(
        "Let t be 0.\nLet s be 0.\nWhile t < 1 / dt + 1 / dx:\n    Set s to s + t.\n    Set t to t + 1.\nShow t.\nShow s.",
        "1/dx + 1/dt\n1/(2·dx^2) + 1/(dt·dx) - 1/(2·dx) + 1/(2·dt^2) - 1/(2·dt)",
    );
}

/// A lamp and a three-phase counter over `⌈1/dt⌉·⌈1/dx⌉` steps read the residues of both.
#[test]
fn a_cycle_over_a_count_of_both_reads_both_residues() {
    shows(
        "Let t: Hyperreal be 0.\nLet lamp be 0.\nLet phase be 0.\nWhile t < 1:\n    Set lamp to 1 - lamp.\n    Set phase to (phase + 1) % 3.\n    Set t to t + dt * dx.\nShow lamp.\nShow phase.",
        "0 when ⌈1/dt⌉ is even, (0 when ⌈1/dx⌉ is even, 1 when it is odd) when it is odd\n\
         0 when ⌈1/dt⌉ ≡ 0 (mod 3), (0 when ⌈1/dx⌉ ≡ 0 (mod 3), 1 when ⌈1/dx⌉ ≡ 1 (mod 3), 2 when ⌈1/dx⌉ ≡ 2 (mod 3)) when ⌈1/dt⌉ ≡ 1 (mod 3), \
         (0 when ⌈1/dx⌉ ≡ 0 (mod 3), 2 when ⌈1/dx⌉ ≡ 1 (mod 3), 1 when ⌈1/dx⌉ ≡ 2 (mod 3)) when ⌈1/dt⌉ ≡ 2 (mod 3)",
    );
}

/// An affine update of period two or three over `⌈1/dt⌉·⌈1/dx⌉` steps ends by its closed form read on the
/// count's classes: a lamp is lit when both reciprocals are odd, and a light passed round three lamps is
/// back where it started when the count is divisible by 3 — when either reciprocal is.
#[test]
fn a_periodic_update_over_a_count_of_both_reads_both_residues() {
    shows(
        "Let t: Hyperreal be 0.\nLet lamp be 0.\nWhile t < 1:\n    Set lamp to 1 - lamp.\n    Set t to t + dt * dx.\nShow lamp.",
        "0 when ⌈1/dt⌉ is even, (0 when ⌈1/dx⌉ is even, 1 when it is odd) when it is odd",
    );
    shows(
        "Let t: Hyperreal be 0.\nLet a be 1.\nLet b be 0.\nLet c be 0.\nWhile t < 1:\n    Let old be a.\n    Set a to c.\n    Set c to b.\n    Set b to old.\n    Set t to t + dt * dx.\nShow a.",
        "1 when ⌈1/dt⌉ ≡ 0 (mod 3), (1 when ⌈1/dx⌉ ≡ 0 (mod 3), 0 when ⌈1/dx⌉ ≡ 1 (mod 3), 0 when ⌈1/dx⌉ ≡ 2 (mod 3)) when ⌈1/dt⌉ ≡ 1 (mod 3), \
         (1 when ⌈1/dx⌉ ≡ 0 (mod 3), 0 when ⌈1/dx⌉ ≡ 1 (mod 3), 0 when ⌈1/dx⌉ ≡ 2 (mod 3)) when ⌈1/dt⌉ ≡ 2 (mod 3)",
    );
}

/// A flow stepped by `dt·dx` over `⌈1/(dt·dx)⌉` steps: `exp(N·log A)`, `N·log A` limited in `dx` — growth
/// ends at `e`, an oscillator turns one radian, and a count one step short of the bound still ends at `e`.
#[test]
fn a_flow_over_a_count_of_both() {
    shows(
        "Let t: Hyperreal be 0.\nLet x: Hyperreal be 1.\nWhile t < 1:\n    Set x to x + x * dt * dx.\n    Set t to t + dt * dx.\nShow the standard part of x.\nShow the standard part of x relative to dt.\nLet s: Hyperreal be 0.\nLet y: Hyperreal be 1.\nWhile s < 1 - dx:\n    Set y to y + y * dt * dx.\n    Set s to s + dt * dx.\nShow the standard part of y.\nLet r: Hyperreal be 0.\nLet p: Hyperreal be 1.\nLet v: Hyperreal be 0.\nWhile r < 1:\n    Set p to p + v * dt * dx.\n    Set v to v - p * dt * dx.\n    Set r to r + dt * dx.\nShow the standard part of p.\nShow the standard part of v.",
        "e\ne\ne\ncos(1)\n-sin(1)",
    );
}

/// A count whose steps add up to an infinite change — `⌈1/(dt·dx²)⌉` steps of `x·dt·dx` — has no end
/// state the exponential of a limited matrix gives.
#[test]
fn a_count_of_both_that_outgrows_its_steps_is_refused() {
    fails_with(
        "Let t: Hyperreal be 0.\nLet x: Hyperreal be 1.\nWhile t < 1 / dx:\n    Set x to x + x * dt * dx.\n    Set t to t + dt * dx.\nShow x.",
        "this supertask's step count 1/(dt·dx^2) outgrows its steps: the change they add up to is infinite in dx",
    );
}

#[test]
fn a_nonlinear_flow_over_a_count_of_both_is_refused() {
    fails_with(
        "Let t: Hyperreal be 0.\nLet x: Hyperreal be 1.\nWhile t < 1:\n    Set x to x + x * x * dt * dx.\n    Set t to t + dt * dx.\nShow x.",
        "this supertask's step count, 1/(dt·dx), is infinite in more than one infinitesimal, which only an affine closed form or a cycle is read over; a flow is integrated over a count of one",
    );
}

// ── a supertask stepped by the smaller infinitesimal ──────────────────────────────────────────

#[test]
fn a_clock_stepped_by_the_smaller_infinitesimal_runs_its_own_count() {
    shows(
        "Let x: Hyperreal be 1.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Set x to x + x * dx.\n    Set t to t + dx.\nShow the standard part of x.\nShow x.",
        "e\ne - (1/2)·e·dx + O(dx^2)",
    );
}

// ── the residues of several infinitesimals ────────────────────────────────────────────────────

/// A value that reads the residues of both generators depends on the coarser — `⌈1/dt⌉` — whose
/// branches depend on the finer, whichever order the program combines them in.
#[test]
fn a_value_that_reads_both_residues_depends_on_the_coarser_first() {
    shows(
        "Show floor(1 / dt) % 2 + floor(1 / dx) % 2.\nShow floor(1 / dx) % 2 + floor(1 / dt) % 2.\nShow (floor(1 / dt) % 2) * (floor(1 / dx) % 3).",
        "(0 when ⌈1/dx⌉ is even, 1 when it is odd) when ⌈1/dt⌉ is even, (1 when ⌈1/dx⌉ is even, 2 when it is odd) when it is odd\n\
         (0 when ⌈1/dx⌉ is even, 1 when it is odd) when ⌈1/dt⌉ is even, (1 when ⌈1/dx⌉ is even, 2 when it is odd) when it is odd\n\
         0 when ⌈1/dt⌉ is even, (0 when ⌈1/dx⌉ ≡ 0 (mod 3), 1 when ⌈1/dx⌉ ≡ 1 (mod 3), 2 when ⌈1/dx⌉ ≡ 2 (mod 3)) when it is odd",
    );
}

#[test]
fn a_dependence_on_one_residue_that_cancels_leaves_the_other() {
    shows(
        "Show floor(1 / dt) % 2 + floor(1 / dx) % 2 - floor(1 / dx) % 2.\nShow floor(1 / dx) % 2 + floor(1 / dt) % 2 - floor(1 / dt) % 2.",
        "0 when ⌈1/dt⌉ is even, 1 when it is odd\n0 when ⌈1/dx⌉ is even, 1 when it is odd",
    );
}

#[test]
fn a_declared_residue_of_the_finer_decides_its_branches() {
    program_shows(
        "## Main\nLet dt be an infinitesimal.\nLet dx be an infinitesimal much smaller than dt whose reciprocal is even.\nShow floor(1 / dt) % 2 + floor(1 / dx) % 2.",
        "0 when ⌈1/dt⌉ is even, 1 when it is odd",
    );
}

/// Two lamps, one switched `⌈1/dx⌉` times and one `⌈1/dt⌉` times: their sum reads both parities.
#[test]
fn two_lamps_on_two_infinitesimals() {
    shows(
        "Let a be 0.\nLet s be 0.\nWhile s < 1:\n    Set a to 1 - a.\n    Set s to s + dx.\nLet b be 0.\nLet t be 0.\nWhile t < 1:\n    Set b to 1 - b.\n    Set t to t + dt.\nShow a.\nShow b.\nShow b + a.\nShow a * dt + b * dx.",
        "0 when ⌈1/dx⌉ is even, 1 when it is odd\n0 when ⌈1/dt⌉ is even, 1 when it is odd\n\
         (0 when ⌈1/dx⌉ is even, 1 when it is odd) when ⌈1/dt⌉ is even, (1 when ⌈1/dx⌉ is even, 2 when it is odd) when it is odd\n\
         (0 when ⌈1/dx⌉ is even, dt when it is odd) when ⌈1/dt⌉ is even, (dx when ⌈1/dx⌉ is even, dt + dx when it is odd) when it is odd",
    );
}

#[test]
fn a_branch_on_both_residues_names_both() {
    fails_with(
        "Let a be floor(1 / dx) % 2.\nLet b be floor(1 / dt) % 2.\nIf a + b = 1:\n    Show 1.\nShow 2.",
        "this condition depends on the residues of ⌈1/dt⌉ and ⌈1/dx⌉: it is (false when ⌈1/dx⌉ is even, true when it is odd) when ⌈1/dt⌉ is even, (true when ⌈1/dx⌉ is even, false when it is odd) when it is odd; declare what is known about 1/dt and 1/dx or take a limit",
    );
}

/// A lamp switched by `a - c` over `⌈1/dt⌉` steps, `a` itself the parity of `⌈1/dx⌉`: the supertask
/// reads its classes of `⌈1/dx⌉` and its answer depends on `⌈1/dt⌉` too, coarser first; each limit
/// rule reads the finer residue within each class of the coarser.
#[test]
fn a_supertask_s_answer_on_both_residues_takes_every_limit_rule() {
    let lamp = |rule: &str| {
        format!(
            "Let a be 0.\nLet s be 0.\nWhile s < 1:\n    Set a to 1 - a.\n    Set s to s + dx.\n{rule}Let c be 0.\nLet t be 0.\nWhile t < 1:\n    Set c to a - c.\n    Set t to t + dt.\nShow c."
        )
    };
    shows(&lamp(""), "0 when ⌈1/dt⌉ is even, (0 when ⌈1/dx⌉ is even, 1 when it is odd) when it is odd");
    shows(&lamp("At the limit, take the Cesàro mean.\n"), "1/4");
    shows(&lamp("At the limit, take the limit superior.\n"), "1");
    shows(&lamp("At the limit, take the limit inferior.\n"), "0");
    fails_with(
        &lamp("At the limit, take the limit.\n"),
        "no limit: the value is 0 when ⌈1/dt⌉ is even, (0 when ⌈1/dx⌉ is even, 1 when it is odd) when it is odd",
    );
}

// ── refusals ──────────────────────────────────────────────────────────────────────────────────

#[test]
fn two_infinitesimals_of_no_stated_order_are_refused() {
    program_fails_with(
        "## Main\nLet dt be an infinitesimal.\nLet h be an infinitesimal.\nShow dt + h.",
        "cannot combine values of two infinitesimals, dt and h, whose order the program does not state; declare one much smaller than the other",
    );
    program_fails_with(
        "## Main\nLet dt be an infinitesimal.\nLet h be an infinitesimal.\nShow dt < h.",
        "cannot combine values of two infinitesimals, dt and h, whose order the program does not state; declare one much smaller than the other",
    );
}

#[test]
fn two_infinitesimals_both_much_smaller_than_a_third_are_unordered() {
    fails_with(
        "Let dy be an infinitesimal much smaller than dt.\nShow dx + dy.",
        "cannot combine values of two infinitesimals, dx and dy, whose order the program does not state; declare one much smaller than the other",
    );
    program_fails_with(
        "## Main\nLet dt be an infinitesimal.\nLet h be an infinitesimal much larger than dt.\nLet k be an infinitesimal much larger than dt.\nShow h < k.",
        "cannot combine values of two infinitesimals, h and k, whose order the program does not state",
    );
}

#[test]
fn values_depending_on_the_reciprocals_of_two_unordered_infinitesimals_are_refused() {
    program_fails_with(
        "## Main\nLet dt be an infinitesimal.\nLet h be an infinitesimal.\nShow floor(1 / dt) % 2 + floor(1 / h) % 2.",
        "cannot combine values of two infinitesimals, dt and h, whose order the program does not state",
    );
}

/// A supertask over values that depend on the reciprocals of two infinitesimals of no stated order is
/// refused by name, as their sum is: a pass adds a value read on the classes of `⌈1/h⌉` to one read on
/// those of `⌈1/dt⌉`, so its end state would read both.
#[test]
fn a_supertask_over_the_residues_of_two_unordered_infinitesimals_is_refused() {
    program_fails_with(
        "## Main\nLet dt be an infinitesimal.\nLet h be an infinitesimal.\nLet a be floor(1 / dt) % 2.\nLet b be floor(1 / h) % 2.\nLet t: Hyperreal be 0.\nWhile t < 1:\n    Set a to a + b.\n    Set t to t + dt.\nShow a.",
        "cannot combine values of two infinitesimals, h and dt, whose order the program does not state",
    );
}

#[test]
fn a_bare_smaller_than_decides_too_little() {
    program_fails_with(
        "## Main\nLet dt be an infinitesimal.\nLet dx be an infinitesimal smaller than dt.",
        "dx smaller than dt is not enough to compare dx with dt^2, dt^3 and every other value made from dt; say `much smaller than dt`",
    );
}

#[test]
fn a_bare_larger_than_decides_too_little() {
    program_fails_with(
        "## Main\nLet dt be an infinitesimal.\nLet h be an infinitesimal larger than dt.",
        "h larger than dt is not enough to compare h with √dt, ∛dt and every other root of dt; say `much larger than dt`",
    );
}

#[test]
fn another_comparative_names_the_two_this_language_reads() {
    program_fails_with(
        "## Main\nLet dt be an infinitesimal.\nLet dx be an infinitesimal much less than dt.",
        "an infinitesimal is declared `much smaller than` or `much larger than` another",
    );
}

#[test]
fn a_bound_is_an_infinitesimal_itself() {
    fails_with(
        "Let x be 2 * dt.\nLet dy be an infinitesimal much smaller than x.",
        "an infinitesimal is declared much smaller than another infinitesimal itself, not than 2·dt",
    );
    fails_with(
        "Let dy be an infinitesimal much larger than 3.",
        "an infinitesimal is declared much larger than another infinitesimal itself, not than 3",
    );
}

#[test]
fn the_bounds_of_a_between_declaration_must_be_ordered() {
    program_fails_with(
        "## Main\nLet dt be an infinitesimal.\nLet h be an infinitesimal.\nLet dx be an infinitesimal much smaller than dt and much larger than h.",
        "dx is declared much smaller than dt and much larger than h, but the program does not state that h is much smaller than dt",
    );
    fails_with(
        "Let dy be an infinitesimal much smaller than dx and much larger than dt.",
        "dy is declared much smaller than dx and much larger than dt, but dt is much larger than dx",
    );
    fails_with(
        "Let dy be an infinitesimal much smaller than dt and much larger than dt.",
        "dy cannot be both much smaller and much larger than dt",
    );
}
