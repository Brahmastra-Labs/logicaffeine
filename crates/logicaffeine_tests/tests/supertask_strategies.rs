//! The supertask engine's strategies, one program each: a loop stepped by an infinitesimal clock
//! resolved by its affine closed form or by the cycle its state enters, a search over every natural
//! decided by its linear structure or answered provisionally, the limit rule a block takes, and the
//! loops the engine leaves exactly as they were. Every program must finish, on every run path.

mod common;
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(300);

fn assert_all_tiers(source: &str, expected: &str) {
    common::assert_all_tiers_within(source, expected, DEADLINE)
}

fn fails_with(source: &str, needle: &str) {
    let out = common::run_all_tiers_within(source, DEADLINE);
    let err = out
        .error
        .unwrap_or_else(|| panic!("expected an error containing {needle:?}; the program printed {:?}", out.output));
    assert!(err.contains(needle), "{err}");
}

// ── a clock stepped by the infinitesimal: the affine closed form ──────────────────────────────

#[test]
fn a_counter_beside_the_clock_counts_one_over_dt() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let n be 0.
While t < 1:
    Set t to t + dt.
    Set n to n + 1.
Show n.
Show t.",
        "1/dt\n1",
    );
}

#[test]
fn a_running_sum_is_the_triangular_number_of_the_step_count() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let k be 0.
Let s be 0.
While t < 1:
    Set s to s + k.
    Set k to k + 1.
    Set t to t + dt.
Show k.
Show s.",
        "1/dt\n1/(2·dt^2) - 1/(2·dt)",
    );
}

/// A polynomial map whose monomials close under it is an affine map of them: `t²` steps to
/// `t² + 2·dt·t + dt²`, so the map acts on `[total, t, t², 1]` and its sum of `t²·dt` is exact,
/// `(1 - dt)(2 - dt)/6`.
#[test]
fn a_sum_of_squares_of_the_clock_is_exact() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let total be 0.
Let t be 0.
While t < 1:
    Set total to total + t * t * dt.
    Set t to t + dt.
Show total.
Show the standard part of total.",
        "1/3 - (1/2)·dt + (1/6)·dt^2\n1/3",
    );
}

#[test]
fn a_sum_of_cubes_of_the_clock_is_exact() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let total be 0.
Let t be 0.
While t < 1:
    Set total to total + t * t * t * dt.
    Set t to t + dt.
Show total.
Show the standard part of total.",
        "1/4 - (1/2)·dt + (1/4)·dt^2\n1/4",
    );
}

/// `Σ k²` over the `1/dt` steps: Faulhaber's cubic, an infinite value held exactly.
#[test]
fn a_sum_of_squares_of_the_step_count_is_its_cubic() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let k be 0.
Let x be 0.
While t < 1:
    Set x to x + k * k.
    Set k to k + 1.
    Set t to t + dt.
Show x.
Show k.",
        "1/(3·dt^3) - 1/(2·dt^2) + 1/(6·dt)\n1/dt",
    );
}

/// Two clocks multiplied close on their product: `b·c` steps to `b·c + 2·dt·b + dt·c + 2·dt²`.
#[test]
fn a_product_of_two_clocks_closes_on_their_product() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let a be 0.
Let b be 0.
Let c be 0.
Let t be 0.
While t < 1:
    Set a to a + b * c * dt.
    Set b to b + dt.
    Set c to c + 2 * dt.
    Set t to t + dt.
Show a.
Show the standard part of a.",
        "2/3 - dt + (1/3)·dt^2\n2/3",
    );
}

/// Growth forced by the square of the clock: `x' = x + t²` from `0` ends at `∫₀¹ e^(1-s)·s² ds = 2e - 5`,
/// the map over `[x, t, t², 1]` having the eigenvalue `1 + dt`.
#[test]
fn a_growth_forced_by_the_square_of_the_clock_ends_at_its_exponential_polynomial() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 0.
Let t: Hyperreal be 0.
While t < 1:
    Set x to x + x * dt + t * t * dt.
    Set t to t + dt.
Show the standard part of x.
Show x.",
        "2·e - 5\n2·e - 5 + O(dt)",
    );
}

#[test]
fn a_clock_with_a_bound_twice_as_far_runs_twice_as_long() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let n be 0.
While t < 2:
    Set t to t + dt.
    Set n to n + 1.
Show n.",
        "2/dt",
    );
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let n be 0.
While t <= 1:
    Set t to t + dt.
    Set n to n + 1.
Show n.",
        "1/dt + 1",
    );
}

#[test]
fn a_clock_counting_down_is_a_clock() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let t be 1.
Let n be 0.
While t > 0:
    Set t to t - dt.
    Set n to n + 1.
Show n.
Show t.",
        "1/dt\n0",
    );
}

#[test]
fn local_temporaries_inside_the_body_are_read_through() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let lamp be 0.
While t < 1:
    Let flipped be 1 - lamp.
    Set lamp to flipped.
    Set t to t + dt.
Show lamp.",
        "0 when ⌈1/dt⌉ is even, 1 when it is odd",
    );
}

#[test]
fn a_quarter_turn_each_step_ends_on_one_of_four_points() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal whose reciprocal is divisible by 4.
Let t be 0.
Let x be 1.
Let y be 0.
While t < 1:
    Let nx be 0 - y.
    Set y to x.
    Set x to nx.
    Set t to t + dt.
Show x.
Show y.",
        "1\n0",
    );
}

#[test]
fn a_supertask_inside_a_function_resolves_on_every_call() {
    assert_all_tiers(
        "## To flips (start: Int) -> Int:
    Let dt be an infinitesimal whose reciprocal is odd.
    Let t be 0.
    Let lamp be start.
    While t < 1:
        Set lamp to 1 - lamp.
        Set t to t + dt.
    Return lamp.

## Main
Show flips(0).
Show flips(1).",
        "1\n0",
    );
}

// ── a clock stepped by the infinitesimal: the cycle its state enters ──────────────────────────

#[test]
fn a_boolean_toggled_every_step_depends_on_the_parity() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let on be false.
While t < 1:
    Set on to not on.
    Set t to t + dt.
Show on.",
        "false when ⌈1/dt⌉ is even, true when it is odd",
    );
}

#[test]
fn a_state_that_cycles_through_a_branch_is_read_at_its_step() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal whose reciprocal is divisible by 3.
Let t be 0.
Let phase be 0.
While t < 1:
    If phase = 2:
        Set phase to 0.
    Otherwise:
        Set phase to phase + 1.
    Set t to t + dt.
Show phase.",
        "0",
    );
}

#[test]
fn a_remainder_walk_enters_its_cycle_after_a_tail() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal whose reciprocal is even.
Let t be 0.
Let x be 2.
While t < 1:
    Set x to (x * x) % 10.
    Set t to t + dt.
Show x.",
        "6",
    );
}

#[test]
fn the_clock_of_a_cycle_resolved_loop_ends_at_its_bound() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let on be false.
While t < 1:
    Set on to not on.
    Set t to t + dt.
Show t.
Show t = 1.",
        "1\ntrue",
    );
}

// ── a counter stepped toward a value that can be infinite: its first exit ─────────────────────

#[test]
fn a_counter_stepped_by_a_constant_toward_an_infinite_bound_reaches_it() {
    assert_all_tiers(
        "## To countTo (x: Hyperreal) -> Hyperreal:
    Let n be 0.
    Let s be 0.
    While n < x:
        Set s to s + 2.
        Set n to n + 1.
    Return s.

## Main
Let dt be an infinitesimal.
Show countTo(5).
Show countTo(1 / dt).
Show countTo(1 / dt + 1 / 2).
Show countTo(0 - 1 / dt).",
        "10\n2/dt\n2/dt + 2\n0",
    );
}

#[test]
fn a_guard_written_either_way_round_with_a_remainder_exits_at_its_first_failure() {
    let past = "## To past (x: Hyperreal) -> Hyperreal:
    Let n be 0.
    While x > n or 0 < n % 3:
        Set n to n + 1.
    Return n.
";
    assert_all_tiers(
        &format!(
            "{past}
## Main
Let dt be an infinitesimal whose reciprocal is divisible by 3.
Show past(10).
Show past(1 / dt).
Show past(1 / dt + 1)."
        ),
        "12\n1/dt\n1/dt + 3",
    );
    assert_all_tiers(
        &format!(
            "{past}
## Main
Let dt be an infinitesimal.
Show past(1 / dt)."
        ),
        "1/dt when ⌈1/dt⌉ ≡ 0 (mod 3), 1/dt + 2 when ⌈1/dt⌉ ≡ 1 (mod 3), 1/dt + 1 when ⌈1/dt⌉ ≡ 2 (mod 3)",
    );
}

#[test]
fn a_counter_whose_guard_never_fails_runs_as_written_and_an_infinitesimal_one_is_refused() {
    assert_all_tiers(
        "## To escape (x: Hyperreal) -> Hyperreal:
    Let n be 0.
    While n >= 0 or n < x:
        Set n to n + 1.
        If n > 3:
            Return n.
    Return n.

## Main
Let dt be an infinitesimal.
Show escape(1 / dt).
Show escape(dt).",
        "4\n4",
    );
    fails_with(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
While t >= 0 or t < 1:
    Set t to t + dt.
Show t.",
        "this loop never ends",
    );
}

// ── a supertask inside a supertask: the stage past every stage ω·k ────────────────────────────

#[test]
fn a_supertask_in_each_pass_of_a_supertask_counts_to_the_square_of_one_over_dt() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let total be 0.
While t < 1:
    Let s be 0.
    Let n be 0.
    While s < 1:
        Set n to n + 1.
        Set s to s + dt.
    Set total to total + n.
    Set t to t + dt.
Show total.
Show t.",
        "1/dt^2\n1",
    );
}

#[test]
fn a_lamp_switched_in_each_pass_of_a_supertask_leaves_a_count_that_depends_on_the_parity() {
    let lamps = |fact: &str| {
        format!(
            "## Main
Let dt be an infinitesimal{fact}.
Let t be 0.
Let lit be 0.
While t < 1:
    Let s be 0.
    Let lamp be 0.
    While s < 1:
        Set lamp to 1 - lamp.
        Set s to s + dt.
    Set lit to lit + lamp.
    Set t to t + dt.
Show lit."
        )
    };
    assert_all_tiers(&lamps(""), "0 when ⌈1/dt⌉ is even, 1/dt when it is odd");
    assert_all_tiers(&lamps(" whose reciprocal is even"), "0");
    assert_all_tiers(&lamps(" whose reciprocal is odd"), "1/dt");
}

/// Each class's answer holds the program's own `dt`: a lamp worth `dt²` in each of `⌈1/dt⌉`
/// passes sums to `dt` on the odd classes, and that sum adds to `dt` as one infinitesimal.
#[test]
fn a_parity_dependent_sum_of_infinitesimals_is_in_the_program_s_own_dt() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let x be 0.
While t < 1:
    Let s be 0.
    Let lamp be 0.
    While s < 1:
        Set lamp to 1 - lamp.
        Set s to s + dt.
    Set x to x + lamp * dt * dt.
    Set t to t + dt.
Show x.
Show x + dt.
Show x < dt.",
        "0 when ⌈1/dt⌉ is even, dt when it is odd\ndt when ⌈1/dt⌉ is even, 2·dt when it is odd\ntrue when ⌈1/dt⌉ is even, false when it is odd",
    );
}

/// A supertask in each pass that reads the outer clock: each pass adds `t·dt`, and the outer loop
/// sums `∫₀¹ t dt` exactly, `(1 - dt)/2`.
#[test]
fn a_supertask_in_each_pass_that_reads_the_outer_clock_sums_its_integral() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let total be 0.
While t < 1:
    Let s be 0.
    Let y be 0.
    While s < 1:
        Set y to y + t * dt.
        Set s to s + dt.
    Set total to total + y * dt.
    Set t to t + dt.
Show total.
Show the standard part of total.
Show t.",
        "1/2 - (1/2)·dt\n1/2\n1",
    );
}

/// The supertask in each pass may change what the passes carry: here it adds to the outer total
/// itself.
#[test]
fn a_supertask_in_each_pass_may_change_what_the_passes_carry() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let total be 0.
While t < 1:
    Let s be 0.
    While s < 1:
        Set total to total + t * dt * dt.
        Set s to s + dt.
    Set t to t + dt.
Show total.",
        "1/2 - (1/2)·dt",
    );
}

/// A pass computes a value its supertask reads: `u = t·t` is fixed through the inner loop and changes
/// from pass to pass, and the outer loop sums `∫₀¹ t² dt`.
#[test]
fn a_supertask_in_each_pass_reads_a_value_the_pass_computes_before_it() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let total be 0.
While t < 1:
    Let u be t * t.
    Let s be 0.
    Let y be 0.
    While s < 1:
        Set y to y + u * dt.
        Set s to s + dt.
    Set total to total + y * dt.
    Set t to t + dt.
Show the standard part of total.",
        "1/3",
    );
}

/// The inner supertask reads a value its pass computes from the clock, so it runs after that value in
/// every pass: it is never computed once ahead of the passes, before the value exists.
#[test]
fn a_supertask_that_reads_a_value_its_pass_computes_is_never_run_ahead_of_it() {
    fails_with(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let total be 0.
While t < 1:
    Let u be t * t.
    Let s be 0.
    Let y be 1.
    While s < 1:
        Set y to y + u * y * dt.
        Set s to s + dt.
    Set total to total + y * dt.
    Set t to t + dt.
Show total.",
        "the supertask in each of its passes reads `u`, which its passes change, and its end state is no polynomial in it",
    );
}

#[test]
fn a_pass_s_own_work_is_read_before_a_name_it_binds_later_in_the_pass() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let n be 2.
Let t be 0.
Let total be 0.
While t < 1:
    Set total to total + n.
    Let n be 5.
    Set t to t + dt.
Show total.",
        "2/dt",
    );
}

#[test]
fn an_inner_supertask_that_reads_the_outer_state_is_refused_with_the_reason() {
    fails_with(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let total be 0.
While t < 1:
    Let s be t.
    Let n be 0.
    While s < 1:
        Set n to n + 1.
        Set s to s + dt.
    Set total to total + n.
    Set t to t + dt.
Show total.",
        "the supertask engine has no exact answer for its body",
    );
}

/// The work a pass runs once lives in the scope of the loop's resolution: a name it binds afresh
/// leaves the program's own name as it was.
#[test]
fn a_name_the_pass_binds_afresh_leaves_the_program_s_own_untouched() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let n be 2.
Let t be 0.
Let total be 0.
While t < 1:
    Let n be 5.
    Set total to total + n.
    Set t to t + dt.
Show n.
Show total.",
        "2\n5/dt",
    );
}

/// `Let` in a pass binds a new name for the rest of the pass: before it the pass changes the
/// program's `total`, after it its own.
#[test]
fn a_pass_that_binds_a_name_again_after_changing_it_keeps_the_two_apart() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let total be 0.
While t < 1:
    Set total to total + 1.
    Let total be 7.
    Set total to total + 2.
    Set t to t + dt.
Show total.",
        "1/dt",
    );
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let total be 0.
Let acc be 0.
While t < 1:
    Let total be total + 1.
    Set acc to acc + total.
    Set t to t + dt.
Show total.
Show acc.",
        "0\n1/dt",
    );
}

/// A pass that binds its clock again steps its own `t`, never the one the guard reads.
#[test]
fn a_pass_that_binds_its_clock_again_is_refused_with_the_reason() {
    fails_with(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let n be 0.
While t < 1:
    Let t be 0.
    Set t to t + dt.
    Set n to n + 1.
Show n.",
        "its pass binds its clock `t` again",
    );
}

/// A pass that binds a name again where the renaming apart cannot follow it — here a loop's own
/// variable — is refused rather than read with the two bindings mixed.
#[test]
fn a_pass_whose_bindings_cannot_be_kept_apart_is_refused_with_the_reason() {
    fails_with(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let x be 0.
Let y be 0.
While t < 1:
    Set x to x + 1.
    Let x be 0.
    Repeat for x in [1, 2]:
        Set y to y + x.
    Set t to t + dt.
Show y.",
        "its pass binds `x` again where the engine cannot keep the two apart",
    );
}

// ── what the engine leaves alone, and what it refuses ─────────────────────────────────────────

#[test]
fn a_clock_with_a_standard_step_runs_exactly_as_written() {
    assert_all_tiers(
        "## Main
Let t be 0.
Let n be 0.
While t < 10:
    Set t to t + 1.
    Set n to n + 2.
Show n.
Show t.",
        "20\n10",
    );
    assert_all_tiers(
        "## Main
Let t be 0.0.
Let n be 0.
While t < 1.0:
    Set t to t + 0.1.
    Set n to n + 1.
Show n.",
        "11",
    );
}

#[test]
fn a_loop_stepping_something_other_than_a_number_runs_as_written_beside_an_infinitesimal() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let d be 2026-01-01.
Let step be 2 weeks.
While d < 2026-02-01:
    Set d to d + step.
Show d.",
        "2026-02-12",
    );
}

#[test]
fn a_standard_step_away_from_its_bound_is_left_to_the_loop_as_written() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let i be 0.
Let step be -1.
Let n be 0.
While i < 5:
    Set i to i + step.
    Set n to n + 1.
    If n = 3:
        Break.
Show n.",
        "3",
    );
}

#[test]
fn a_supertask_whose_state_grows_geometrically_is_refused_with_its_reason() {
    fails_with(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let x be 1.
While t < 1:
    Set x to x * 2.
    Set t to t + dt.
Show x.",
        "not a root of unity",
    );
}

/// A state whose size squares each step makes each step cost the square of the last one's, so the cycle
/// search gives it up while one more step is still cheap: a polynomial in `dt` whose degree and
/// coefficients double each step is refused on every tier within seconds, not minutes.
#[test]
fn a_state_whose_steps_grow_quadratically_is_refused_while_a_step_is_cheap() {
    let source = "## Main
Let dt be an infinitesimal.
Let k: Hyperreal be 1 + dt.
Let x: Hyperreal be 1 / 2.
Let t: Hyperreal be 0.
While t < 1:
    Set x to x + x * x * k.
    Set t to t + dt.
Show x.";
    let out = common::run_all_tiers_within(source, Duration::from_secs(60));
    let err = out.error.unwrap_or_else(|| panic!("expected a refusal; the program printed {:?}", out.output));
    assert!(err.contains("grows without repeating"), "{err}");
}

#[test]
fn a_state_that_grows_without_repeating_is_refused_before_it_fills_memory() {
    fails_with(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let x be 2.
While t < 1:
    Set x to x * x.
    Set t to t + dt.
Show x.",
        "grows without repeating",
    );
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let x be 1.
While t < 1:
    Set x to x * x.
    Set t to t + dt.
Show x.",
        "1",
    );
}

#[test]
fn a_clock_written_on_the_right_of_its_bound_is_a_clock() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let n be 0.
While 1 > t:
    Set n to n + 1.
    Set t to t + dt.
Show n.",
        "1/dt",
    );
}

#[test]
fn two_cycles_of_different_lengths_meet_at_their_common_period() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal whose reciprocal is divisible by 12.
Let t be 0.
Let a be 1.
Let b be 0.
Let c be 1.
Let d be 0.
Let e be 0.
While t < 1:
    Let na be 0 - b.
    Set b to a.
    Set a to na.
    Let nc be 0 - c - d.
    Set d to c.
    Set c to nc.
    Set e to e + 1.
    Set t to t + dt.
Show a.
Show c.
Show e.",
        "1\n1\n1/dt",
    );
}

#[test]
fn a_supertask_that_shows_at_every_step_is_refused() {
    fails_with(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
While t < 1:
    Show t.
    Set t to t + dt.",
        "runs infinitely many times",
    );
}

// ── searches over every natural ───────────────────────────────────────────────────────────────

#[test]
fn a_linear_search_is_decided_exactly() {
    for (condition, want) in [
        ("n + n = 7", "false"),
        ("n * 3 = 12", "true"),
        ("n * 2 = 1000000", "true"),
        ("n > 5 and n < 3", "false"),
        ("n % 4 = 3 and n > 100", "true"),
        ("n * 6 = 20", "false"),
        ("n is even and n is odd", "false"),
    ] {
        assert_all_tiers(
            &format!(
                "## Main\nLet found be false.\nRepeat for every natural n:\n    If {condition}:\n        Set found to true.\nShow found."
            ),
            want,
        );
    }
}

#[test]
fn a_linear_search_reads_the_search_variable_on_both_sides() {
    assert_all_tiers(
        "## Main\nLet found be false.\nRepeat for every natural n:\n    If 2 * n = n + 7:\n        Set found to true.\nShow found.",
        "true",
    );
    assert_all_tiers(
        "## Main\nLet found be false.\nRepeat for every natural n:\n    If 3 * n + 1 = n + 8:\n        Set found to true.\nShow found.",
        "false",
    );
}

#[test]
fn a_general_search_ended_by_break_is_settled() {
    assert_all_tiers(
        "## Main
Let total be 0.
Repeat for every natural n:
    Set total to total + n.
    If total > 100:
        Break.
Show total.",
        "105",
    );
}

#[test]
fn a_linear_search_with_runtime_coefficients_is_decided_at_run_time() {
    assert_all_tiers(
        "## To divides (k: Int) and (target: Int) -> Bool:
    Repeat for every natural n:
        If n * k = target:
            Return true.
    Return false.

## Main
Show divides(7, 49).
Show divides(7, 50).
Show divides(5, 0).",
        "true\nfalse\ntrue",
    );
}

#[test]
fn a_search_records_its_first_witness() {
    assert_all_tiers(
        "## Main
Let first be 0.
Repeat for every natural n:
    If n * n > 50:
        Set first to n.
        Break.
Show first.",
        "8",
    );
}

#[test]
fn a_search_beyond_the_linear_reading_finds_its_witness_and_is_settled() {
    assert_all_tiers(
        "## Main
Let found be false.
Repeat for every natural n:
    If n * n = 49:
        Set found to true.
Show found.",
        "true",
    );
}

#[test]
fn a_search_no_proof_settles_is_provisional_with_its_evidence() {
    assert_all_tiers(
        "## Main
Let found be false.
Repeat for every natural n, checking up to 500 cases:
    If n * n = 2 * 361 + 1:
        Set found to true.
Show found.",
        "false (provisional: checked 500 cases, 0 revisions; not proved)",
    );
}

#[test]
fn a_function_answering_after_an_unsettled_search_answers_provisionally() {
    assert_all_tiers(
        "## To isSquare (m: Int) -> Bool:
    Repeat for every natural n, checking up to 100 cases:
        If n * n = m:
            Return true.
    Return false.

## Main
Show isSquare(49).
Show isSquare(50).",
        "true\nfalse (provisional: checked 100 cases, 0 revisions; not proved)",
    );
}

#[test]
fn a_running_guess_counts_its_revisions() {
    assert_all_tiers(
        "## Main
Let best be 0.
Repeat for every natural n, checking up to 1000 cases:
    If n * n % 7 = 2 and n > best:
        Set best to n.
Show best.",
        "998 (provisional: checked 1,000 cases, 286 revisions; not proved)",
    );
}

#[test]
fn a_guess_a_function_returns_after_its_own_search_carries_that_evidence_once() {
    assert_all_tiers(
        "## To bestBelow (k: Int) -> Int:
    Let best be 0.
    Repeat for every natural n, checking up to 100 cases:
        If n * n % 7 = 2 and n > best and n < k:
            Set best to n.
    Return best.

## Main
Show bestBelow(50).",
        "46 (provisional: checked 100 cases, 14 revisions; not proved)",
    );
}

#[test]
fn a_value_made_from_two_provisional_answers_carries_the_fewest_cases_and_every_revision() {
    assert_all_tiers(
        "## Main
Let a be 0.
Repeat for every natural n, checking up to 100 cases:
    If n * n % 7 = 2 and n > a:
        Set a to n.
Let b be 0.
Repeat for every natural n, checking up to 50 cases:
    If n * n % 7 = 2 and n > b:
        Set b to n.
Show a + b.
Show a.
Show b.",
        "141 (provisional: checked 50 cases, 42 revisions; not proved)\n\
         95 (provisional: checked 100 cases, 28 revisions; not proved)\n\
         46 (provisional: checked 50 cases, 14 revisions; not proved)",
    );
}

#[test]
fn a_decided_answer_beside_a_provisional_one_leaves_the_provisional_evidence() {
    assert_all_tiers(
        "## Main
Let found be false.
Repeat for every natural n:
    If n * 3 = 12:
        Set found to true.
Let best be 0.
Repeat for every natural n, checking up to 50 cases:
    If n * n % 7 = 2 and n > best:
        Set best to n.
Show found.
If found:
    Show best + 1.",
        "true\n47 (provisional: checked 50 cases, 14 revisions; not proved)",
    );
}

// ── limit rules ───────────────────────────────────────────────────────────────────────────────

#[test]
fn a_limit_rule_resolves_the_supertasks_after_it_in_its_block() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let lamp be 0.
While t < 1:
    Set lamp to 1 - lamp.
    Set t to t + dt.
Show lamp.
At the limit, take the Cesàro mean.
Set t to 0.
Set lamp to 0.
While t < 1:
    Set lamp to 1 - lamp.
    Set t to t + dt.
Show lamp.",
        "0 when ⌈1/dt⌉ is even, 1 when it is odd\n1/2",
    );
}

#[test]
fn a_limit_rule_reaches_into_nested_blocks_but_not_out_of_them() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let t be 0.
Let lamp be 0.
If true:
    At the limit, take the limit superior.
    While t < 1:
        Set lamp to 1 - lamp.
        Set t to t + dt.
    Show lamp.
Set t to 0.
Set lamp to 0.
While t < 1:
    Set lamp to 1 - lamp.
    Set t to t + dt.
Show lamp.",
        "1\n0 when ⌈1/dt⌉ is even, 1 when it is odd",
    );
}

/// The same holds when the state is a rational function of a root of `dt`: its arithmetic is the
/// field's own, and its growth is refused as promptly.
#[test]
fn a_state_of_roots_of_dt_whose_steps_grow_quadratically_is_refused_while_a_step_is_cheap() {
    let source = "## Main
Let dt be an infinitesimal.
Let k: Hyperreal be sqrt(dt).
Let x: Hyperreal be 1 / 2.
Let t: Hyperreal be 0.
While t < 1:
    Set x to x + x * x * k.
    Set t to t + dt.
Show x.";
    let out = common::run_all_tiers_within(source, Duration::from_secs(60));
    let err = out.error.unwrap_or_else(|| panic!("expected a refusal; the program printed {:?}", out.output));
    assert!(err.contains("grows without repeating"), "{err}");
}
