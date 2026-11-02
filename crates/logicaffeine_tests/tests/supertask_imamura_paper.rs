//! The programs of Imamura, "On the Extreme Power of Nonstandard Programming Languages"
//! (arXiv:1811.05307), run as LOGOS programs — the specification of the supertask engine.
//!
//! Imamura shows the language While^dt (While with an infinitesimal `dt`, Suenaga–Hasuo) computes
//! beyond Turing machines through real constants, exact comparison and supertasks. LOGOS keeps the
//! infinitesimal and the supertasks and answers each one as far as a real machine can: a supertask
//! whose end state a finite witness determines runs in finite time and prints that state exactly;
//! one whose end state depends on which infinitesimal `dt` is prints the dependence; a declared
//! residue or limit rule resolves it; a search no proof settles prints a provisional answer with
//! its evidence. Every program here must print the same bytes on every tier, and finish: a
//! supertask resolved in finite time is the claim, so each run has a deadline.

mod common;
use std::time::Duration;

/// Every program here resolves its supertasks before it runs; one that spins past this has not.
const DEADLINE: Duration = Duration::from_secs(300);

fn assert_all_tiers(source: &str, expected: &str) {
    common::assert_all_tiers_within(source, expected, DEADLINE)
}

fn run_all_tiers(source: &str) -> common::TierAgreement {
    common::run_all_tiers_within(source, DEADLINE)
}

// ── the infinitesimal itself ──────────────────────────────────────────────────────────────────

#[test]
fn dt_is_a_positive_infinitesimal_with_an_infinite_reciprocal() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Show dt.
Show 1 / dt.
Show dt is infinitesimal.
Show 1 / dt is infinitesimal.
Show the standard part of 3 + 2 * dt.
Show dt > 0.
Let tiny: Rational be 1 / 1000000.
Show dt < tiny.",
        "dt\n1/dt\ntrue\nfalse\n3\ntrue\ntrue",
    );
}

// ── Lemma 1: the floor function on *R ─────────────────────────────────────────────────────────

/// The paper's program: count `n` up from `0` until `x` lies in `[n, n + 1)` or in `[-n, -n + 1)`.
const FLOOR: &str = "## To floorOf (x: Hyperreal) -> Hyperreal:
    Let n be 0.
    While not ((n <= x and x < n + 1) or (0 - n <= x and x < 0 - n + 1)):
        Set n to n + 1.
    If x >= 0:
        Return n.
    Return 0 - n.
";

#[test]
fn lemma_1_the_floor_of_a_standard_or_limited_value_counts_to_it() {
    assert_all_tiers(
        &format!(
            "{FLOOR}
## Main
Let dt be an infinitesimal.
Show floorOf(7 / 2).
Show floorOf(0 - 7 / 2).
Show floorOf(3).
Show floorOf(3 + dt).
Show floorOf(3 - dt).
Show floorOf(0 - dt)."
        ),
        "3\n-4\n3\n3\n2\n-1",
    );
}

#[test]
fn lemma_1_the_floor_of_an_infinite_value_is_the_infinite_integer_the_count_reaches() {
    assert_all_tiers(
        &format!(
            "{FLOOR}
## Main
Let dt be an infinitesimal.
Show floorOf(1 / dt).
Show floorOf(1 / dt + 1 / 2).
Show floorOf(1 / dt - dt).
Show floorOf(0 - 1 / dt).
Show floorOf(1 / (2 * dt))."
        ),
        "1/dt\n1/dt\n1/dt - 1\n-1/dt\n1/(2·dt) when ⌈1/dt⌉ is even, 1/(2·dt) - 1/2 when it is odd",
    );
}

#[test]
fn lemma_1_a_declared_residue_settles_the_floor_of_a_fraction_of_the_infinite() {
    assert_all_tiers(
        &format!(
            "{FLOOR}
## Main
Let dt be an infinitesimal whose reciprocal is odd.
Show floorOf(1 / (2 * dt))."
        ),
        "1/(2·dt) - 1/2",
    );
}

// ── Remark 7: a loop that runs infinitely many times ──────────────────────────────────────────

#[test]
fn remark_7_the_counter_of_a_dt_loop_is_the_infinite_natural_one_over_dt() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let t: Hyperreal be 0.
Let u be 0.
While t < 1:
    Set t to t + dt.
    Set u to u + 1.
Show u.
Show t.",
        "1/dt\n1",
    );
}

// ── Thomson's lamp ────────────────────────────────────────────────────────────────────────────

const LAMP: &str = "Let time: Hyperreal be 0.
Let lamp be 0.
While time < 1:
    Set time to time + dt.
    Set lamp to 1 - lamp.
Show lamp.";

#[test]
fn thomsons_lamp_ends_in_a_state_that_depends_on_which_infinitesimal_dt_is() {
    assert_all_tiers(
        &format!("## Main\nLet dt be an infinitesimal.\n{LAMP}"),
        "0 when ⌈1/dt⌉ is even, 1 when it is odd",
    );
}

#[test]
fn declaring_the_parity_of_one_over_dt_settles_the_lamp() {
    assert_all_tiers(
        &format!("## Main\nLet dt be an infinitesimal whose reciprocal is even.\n{LAMP}"),
        "0",
    );
    assert_all_tiers(
        &format!("## Main\nLet dt be an infinitesimal whose reciprocal is odd.\n{LAMP}"),
        "1",
    );
}

#[test]
fn each_limit_rule_gives_its_classical_answer_to_the_lamp() {
    for (rule, want) in [
        ("the limit superior", "1"),
        ("the limit inferior", "0"),
        ("the Cesàro mean", "1/2"),
        ("the Cesaro mean", "1/2"),
    ] {
        assert_all_tiers(
            &format!("## Main\nLet dt be an infinitesimal.\nAt the limit, take {rule}.\n{LAMP}"),
            want,
        );
    }
}

#[test]
fn demanding_a_limit_the_lamp_does_not_have_fails_with_the_dependence() {
    let out = run_all_tiers(&format!(
        "## Main\nLet dt be an infinitesimal.\nAt the limit, take the limit.\n{LAMP}"
    ));
    let err = out.error.expect("the lamp has no limit");
    assert!(err.contains("no limit"), "{err}");
    assert!(err.contains("0 when ⌈1/dt⌉ is even, 1 when it is odd"), "{err}");
}

#[test]
fn contradictory_facts_about_the_infinitesimal_are_refused() {
    let out = run_all_tiers(
        "## Main
Let dt be an infinitesimal whose reciprocal is even and odd.
Show dt.",
    );
    let err = out.error.expect("even and odd contradict");
    assert!(err.contains("contradict"), "{err}");
}

// ── the bouncing ball: a good supertask ───────────────────────────────────────────────────────

#[test]
fn the_bouncing_ball_comes_to_rest_at_its_zeno_time_and_stays_there() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let g be 10.
Let c: Rational be 1 / 2.
Let h: Hyperreal be 0.
Let v: Hyperreal be 10.
Let time: Hyperreal be 0.
Let last: Hyperreal be 0.
While time < 5:
    Set h to h + v * dt.
    Set v to v - g * dt.
    If h <= 0 and v < 0:
        Set h to 0.
        Set v to 0 - c * v.
        If v is not infinitesimal:
            Set last to time.
    Set time to time + dt.
Show the standard part of h.
Show the standard part of v.
Show the standard part of last.",
        "0\n0\n4",
    );
}

// ── Euler flows: the standard part is the solution of the differential equation ──────────────

#[test]
fn the_euler_flow_of_exponential_growth_has_standard_part_e() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 1.
Let t: Hyperreal be 0.
While t < 1:
    Set x to x + x * dt.
    Set t to t + dt.
Show the standard part of x.",
        "e",
    );
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 1.
Let t: Hyperreal be 0.
While t < 1:
    Set x to x - x * dt.
    Set t to t + dt.
Show the standard part of x.",
        "e^(-1)",
    );
}

#[test]
fn the_harmonic_oscillator_integrates_to_cosine_and_sine() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 1.
Let v: Hyperreal be 0.
Let t: Hyperreal be 0.
While t < 1:
    Let a be 0 - x.
    Set x to x + v * dt.
    Set v to v + a * dt.
    Set t to t + dt.
Show the standard part of x.
Show the standard part of v.",
        "cos(1)\n-sin(1)",
    );
}

// ── searches over every natural: the supertask as a quantifier ────────────────────────────────

#[test]
fn a_search_that_provably_never_succeeds_ends_false() {
    assert_all_tiers(
        "## Main
Let found be false.
Repeat for every natural n:
    If n + n = 7:
        Set found to true.
Show found.",
        "false",
    );
}

#[test]
fn a_search_that_succeeds_ends_true() {
    assert_all_tiers(
        "## Main
Let found be false.
Repeat for every natural n:
    If n * 3 = 12:
        Set found to true.
Show found.",
        "true",
    );
}

#[test]
fn a_search_no_proof_settles_ends_provisional_with_its_evidence() {
    assert_all_tiers(
        "## To isPrime (n: Int) -> Bool:
    If n < 2:
        Return false.
    Let d be 2.
    While d * d <= n:
        If n % d = 0:
            Return false.
        Set d to d + 1.
    Return true.

## To isSumOfTwoPrimes (n: Int) -> Bool:
    Let p be 2.
    While p <= n - p:
        If isPrime(p) and isPrime(n - p):
            Return true.
        Set p to p + 1.
    Return false.

## Main
Let found be false.
Repeat for every natural n, checking up to 1000 cases:
    If n > 2 and n % 2 = 0 and not isSumOfTwoPrimes(n):
        Set found to true.
Show found.",
        "false (provisional: checked 1,000 cases, 0 revisions; not proved)",
    );
}

// ── Proposition 3 and Corollary 4: one real number holds a whole set ──────────────────────────

/// Primality, the set the real number's base-3 digits spell out.
const PRIMES: &str = "## To isPrime (n: Int) -> Bool:
    If n < 2:
        Return false.
    Let d be 2.
    While d * d <= n:
        If n % d = 0:
            Return false.
        Set d to d + 1.
    Return true.

## To primeDigit (i: Int) -> Int:
    If isPrime(i):
        Return 1.
    Return 0.
";

/// The paper's program: shift the real number left `x` places in base 3 and read the digit there.
const MEMBER_OF: &str = "## To memberOf (x: Int) and (r: ExactReal) -> Int:
    Let a be r.
    Let k be x.
    While k != 0:
        Set a to 3 * a.
        Set k to k - 1.
    If a - 3 * floor(a / 3) >= 1:
        Return 1.
    Return 0.
";

#[test]
fn proposition_3_one_real_number_decides_a_set() {
    assert_all_tiers(
        &format!(
            "{PRIMES}
{MEMBER_OF}
## Main
Let r be the real number whose digits in base 3 are primeDigit.
Show memberOf(2, r).
Show memberOf(4, r).
Show memberOf(97, r).
Show memberOf(91, r).
Show r."
        ),
        "1\n0\n1\n0\n0.152726902725722471528175418744",
    );
}

/// `f(x) = x·x` as the set of Cantor-paired points of its graph, `J(x, y) = (x + y)(x + y + 1)/2 + y`.
const GRAPH: &str = "## To pairOf (x: Int) and (y: Int) -> Int:
    Return (x + y) * (x + y + 1) / 2 + y.

## To graphDigit (i: Int) -> Int:
    Let w be 0.
    While (w + 1) * (w + 2) / 2 <= i:
        Set w to w + 1.
    Let y be i - w * (w + 1) / 2.
    Let x be w - y.
    If y = x * x:
        Return 1.
    Return 0.
";

#[test]
fn corollary_4_one_real_number_computes_a_function() {
    assert_all_tiers(
        &format!(
            "{GRAPH}
{MEMBER_OF}
## To valueAt (x: Int) and (r: ExactReal) -> Int:
    Let y be 0.
    While memberOf(pairOf(x, y), r) = 0:
        Set y to y + 1.
    Return y.

## Main
Let r be the real number whose digits in base 3 are graphDigit.
Show valueAt(0, r).
Show valueAt(2, r).
Show valueAt(3, r)."
        ),
        "0\n4\n9",
    );
}

// ── Proposition 6: the limit lemma ────────────────────────────────────────────────────────────

/// `F(s, x)`: whether the Collatz walk from `x` reaches `1` within `s` steps. Its limit over `s`
/// asks whether a search halts, and the program computes it by guessing and revising.
const LIMIT_LEMMA: &str = "## To reachesOneWithin (x: Int) and (s: Int) -> Int:
    Let n be x.
    Let k be 0.
    While k < s:
        If n = 1:
            Return 1.
        If n % 2 = 0:
            Set n to n / 2.
        Otherwise:
            Set n to 3 * n + 1.
        Set k to k + 1.
    If n = 1:
        Return 1.
    Return 0.

## To limitOf (x: Int) -> Int:
    Let y be 0.
    Repeat for every natural s, checking up to 200 cases:
        Set y to reachesOneWithin(x, s).
    Return y.
";

#[test]
fn proposition_6_the_limit_lemma_answers_with_its_last_guess_and_its_evidence() {
    assert_all_tiers(
        &format!(
            "{LIMIT_LEMMA}
## Main
Show limitOf(1).
Show limitOf(27).
Show limitOf(77031)."
        ),
        "1 (provisional: checked 200 cases, 1 revision; not proved)\n\
         1 (provisional: checked 200 cases, 1 revision; not proved)\n\
         0 (provisional: checked 200 cases, 0 revisions; not proved)",
    );
}

#[test]
fn an_oracle_is_a_function_whose_body_runs_a_supertask() {
    assert_all_tiers(
        "## To hasMultipleOf (k: Int) and (target: Int) -> Bool:
    Repeat for every natural n:
        If n * k = target:
            Return true.
    Return false.

## Main
Show hasMultipleOf(3, 12).
Show hasMultipleOf(2, 7).",
        "true\nfalse",
    );
}
