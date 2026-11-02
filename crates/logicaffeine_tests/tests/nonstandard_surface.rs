//! The English surface of infinitesimals: declaring one with what is known about its reciprocal,
//! the nonstandard predicates, the standard part, the limit rule a program takes, and the exact
//! number types a binding may declare. Each form reads as the sentence a mathematician writes and
//! runs the same on the tree-walker, the VM, and the optimized VM with and without the native tier.

mod common;
use common::{assert_all_tiers, run_all_tiers};

fn fails_with(source: &str, needle: &str) {
    let out = run_all_tiers(source);
    let err = out
        .error
        .unwrap_or_else(|| panic!("expected an error containing {needle:?}; the program printed {:?}", out.output));
    assert!(err.contains(needle), "{err}");
}

// ── declaring the infinitesimal ───────────────────────────────────────────────────────────────

#[test]
fn an_infinitesimal_is_declared_under_any_name() {
    assert_all_tiers("## Main\nLet dt be an infinitesimal.\nShow dt.", "dt");
    assert_all_tiers("## Main\nLet h be an infinitesimal.\nShow h + 1.", "1 + h");
    assert_all_tiers("## Main\nLet eps be an infinitesimal.\nShow 1 / eps.", "1/eps");
    assert_all_tiers("## Main\nLet dt be an infinitesimal.\nLet x be dt * 2.\nSet x to x + dt.\nShow x.", "3·dt");
}

#[test]
fn the_reciprocal_of_the_infinitesimal_is_an_infinite_integer_with_a_residue() {
    assert_all_tiers(
        "## Main\nLet dt be an infinitesimal.\nLet n be 1 / dt.\nShow n % 2.\nShow n > 1000000000000.",
        "0 when ⌈1/dt⌉ is even, 1 when it is odd\ntrue",
    );
}

#[test]
fn a_fact_about_the_reciprocal_fixes_its_residues() {
    let residue = |facts: &str, k: u64| {
        format!("## Main\nLet dt be an infinitesimal whose reciprocal is {facts}.\nLet n be 1 / dt.\nShow n % {k}.")
    };
    assert_all_tiers(&residue("even", 2), "0");
    assert_all_tiers(&residue("odd", 2), "1");
    assert_all_tiers(&residue("divisible by 6", 3), "0");
    assert_all_tiers(&residue("divisible by 6", 4), "0 when ⌈1/dt⌉ ≡ 0 (mod 4), 2 when ⌈1/dt⌉ ≡ 2 (mod 4)");
    assert_all_tiers(&residue("a multiple of 5", 5), "0");
    assert_all_tiers(&residue("congruent to 1 modulo 3", 3), "1");
    assert_all_tiers(&residue("congruent to 7 mod 3", 3), "1");
    assert_all_tiers(&residue("not divisible by 3", 3), "1 when ⌈1/dt⌉ ≡ 1 (mod 3), 2 when ⌈1/dt⌉ ≡ 2 (mod 3)");
    assert_all_tiers(&residue("not even", 2), "1");
}

#[test]
fn facts_joined_by_and_hold_together() {
    let residue = |facts: &str, k: u64| {
        format!("## Main\nLet dt be an infinitesimal whose reciprocal is {facts}.\nLet n be 1 / dt.\nShow n % {k}.")
    };
    assert_all_tiers(&residue("even and divisible by 3", 6), "0");
    assert_all_tiers(&residue("odd and congruent to 2 modulo 3", 6), "5");
    assert_all_tiers(&residue("even and not divisible by 4", 4), "2");
    assert_all_tiers(&residue("divisible by 2 and divisible by 3 and divisible by 5", 30), "0");
}

/// A condition on the residue of the infinite integer has no answer until a fact fixes the residue,
/// and the optimizer's reading of `x % 2 = 0` as `x & 1 = 0` does not apply to it: only the
/// remainder knows the residue.
#[test]
fn a_condition_on_the_residue_waits_for_a_fact() {
    fails_with(
        "## Main\nLet dt be an infinitesimal.\nLet n be 1 / dt.\nShow \"before\".\nIf n % 2 = 0:\n    Show \"even\".\n",
        "this condition depends on the residue of",
    );
    assert_all_tiers(
        "## Main\nLet dt be an infinitesimal whose reciprocal is even.\nLet n be 1 / dt.\nIf n % 2 = 0:\n    Show \"even\".\nIf n % 2 != 0:\n    Show \"odd\".\n",
        "even",
    );
    assert_all_tiers(
        "## Main\nLet dt be an infinitesimal whose reciprocal is odd.\nLet n be 1 / dt + 2.\nIf n % 2 = 0:\n    Show \"even\".\nIf n % 2 != 0:\n    Show \"odd\".\n",
        "odd",
    );
    fails_with(
        "## Main\nLet dt be an infinitesimal whose reciprocal is even.\nLet n be 1 / dt.\nIf n % 4 = 0:\n    Show \"a multiple of four\".\n",
        "this condition depends on the residue of",
    );
}

/// Text joins a value of the extension as it prints, as it joins every other value.
#[test]
fn text_joins_an_exact_value_as_it_prints() {
    assert_all_tiers(
        "## Main\nLet dt be an infinitesimal.\nShow \"dt is \" + dt.\nShow dt + \" units\".\nShow \"one over it: \" + 1 / dt.\nShow \"parity \" + (1 / dt) % 2.\n",
        "dt is dt\ndt units\none over it: 1/dt\nparity 0 when ⌈1/dt⌉ is even, 1 when it is odd",
    );
    assert_all_tiers(
        "## Main\nLet a: Algebraic be 1 / 3.\nLet w: Ordinal be 2.\nShow \"a = \" + a + \", w = \" + w.\n",
        "a = 1/3, w = 2",
    );
}

/// Text meets an exact value only by joining it: any other operator is refused, naming both kinds.
#[test]
fn text_meets_an_exact_value_only_by_joining() {
    fails_with(
        "## Main\nLet dt be an infinitesimal.\nShow \"ab\" - dt.\n",
        "cannot subtract Text and Hyperreal",
    );
    fails_with(
        "## Main\nLet dt be an infinitesimal.\nShow dt * \"ab\".\n",
        "cannot multiply Hyperreal and Text",
    );
    fails_with(
        "## Main\nLet dt be an infinitesimal.\nShow \"ab\" / (1 / dt).\n",
        "cannot divide Text and Hypernatural",
    );
}

#[test]
fn contradictory_facts_are_refused_before_the_program_runs() {
    fails_with(
        "## Main\nLet dt be an infinitesimal whose reciprocal is even and odd.\nShow 1.",
        "the facts about 1/dt contradict each other: no natural satisfies them all",
    );
    fails_with(
        "## Main\nLet dt be an infinitesimal whose reciprocal is divisible by 4 and congruent to 2 modulo 8.\nShow 1.",
        "contradict",
    );
    fails_with("## Main\nLet dt be an infinitesimal whose reciprocal is not divisible by 1.\nShow 1.", "contradict");
}

#[test]
fn a_fact_too_fine_for_one_value_is_refused_by_name() {
    fails_with(
        "## Main\nLet dt be an infinitesimal whose reciprocal is divisible by 1000003.\nShow 1.",
        "1/dt would split over 1000003 residue classes; one value holds at most 65,536",
    );
    fails_with(
        "## Main\nLet dt be an infinitesimal whose reciprocal is divisible by 65521 and divisible by 65519.\nShow 1.",
        "residue classes",
    );
}

#[test]
fn a_fact_names_a_positive_whole_number() {
    fails_with(
        "## Main\nLet dt be an infinitesimal whose reciprocal is divisible by 0.\nShow 1.",
        "a fact about 1/dt divides by a positive whole number",
    );
    fails_with(
        "## Main\nLet k be 3.\nLet dt be an infinitesimal whose reciprocal is divisible by k.\nShow 1.",
        "a fact about 1/dt names a whole number written out",
    );
    fails_with(
        "## Main\nLet dt be an infinitesimal whose reciprocal is prime.\nShow 1.",
        "even, odd, divisible by",
    );
}

// ── the nonstandard predicates ────────────────────────────────────────────────────────────────

#[test]
fn infinitesimal_and_limited_are_adjectives_with_negations() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Show dt is infinitesimal.
Show 3 * dt is infinitesimal.
Show 1 + dt is infinitesimal.
Show 0 is infinitesimal.
Show 1 / dt is infinitesimal.
Show 1 + dt is not infinitesimal.
Show dt is not infinitesimal.",
        "true\ntrue\nfalse\ntrue\nfalse\ntrue\nfalse",
    );
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Show 1 + dt is limited.
Show 1 / dt is limited.
Show 1 / dt is not limited.
Show 5 is limited.",
        "true\nfalse\ntrue\ntrue",
    );
}

#[test]
fn infinitely_close_compares_up_to_an_infinitesimal() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Show 3 + dt is infinitely close to 3.
Show 3 + dt is infinitely close to 4.
Show 3 + dt is not infinitely close to 4.
Show 1 / dt is infinitely close to 1 / dt + 1.",
        "true\nfalse\ntrue\nfalse",
    );
}

#[test]
fn the_predicates_read_inside_conditions() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Let x be 2 + dt.
If x is limited and x is not infinitesimal:
    Show \"appreciable\".
Otherwise:
    Show \"not\".",
        "appreciable",
    );
}

#[test]
fn a_bound_name_the_lexer_reads_as_a_participle_compares_by_equality() {
    assert_all_tiers("## Main\nLet closed be 5.\nShow 5 is closed.\nShow 4 is closed.", "true\nfalse");
    assert_all_tiers("## Main\nLet finished be 2.\nLet x be 2.\nIf x is finished:\n    Show \"same\".", "same");
}

#[test]
fn a_binding_named_like_the_adjective_keeps_its_equality() {
    assert_all_tiers("## Main\nLet limited be 5.\nLet x be 5.\nShow x is limited.", "true");
    assert_all_tiers("## Main\nLet infinitesimal be 2.\nLet x be 3.\nShow x is infinitesimal.", "false");
}

// ── the standard part ─────────────────────────────────────────────────────────────────────────

#[test]
fn the_standard_part_takes_the_whole_expression_after_it() {
    assert_all_tiers(
        "## Main
Let dt be an infinitesimal.
Show the standard part of 3 + 2 * dt.
Show the standard part of (1 + dt) * (1 - dt).
Show the standard part of dt.
Show the standard part of 7.
Show the standard part of 1 + dt < 2.",
        "3\n1\n0\n7\ntrue",
    );
}

#[test]
fn an_infinite_value_has_no_standard_part() {
    fails_with(
        "## Main\nLet dt be an infinitesimal.\nShow the standard part of 1 / dt.",
        "1/dt is infinite and has no standard part",
    );
}

// ── limit rules ───────────────────────────────────────────────────────────────────────────────

#[test]
fn a_limit_rule_is_a_statement_of_its_own() {
    for rule in [
        "the standard part",
        "the limit",
        "the limit superior",
        "the limit inferior",
        "the Cesàro mean",
        "the Cesaro mean",
    ] {
        assert_all_tiers(&format!("## Main\nAt the limit, take {rule}.\nShow 1."), "1");
    }
}

#[test]
fn an_unknown_limit_rule_is_refused_with_the_rules_there_are() {
    fails_with(
        "## Main\nAt the limit, take the average.\nShow 1.",
        "`the average` is not a limit rule; take the standard part, the limit, the limit superior, the limit inferior or the Cesàro mean",
    );
}

// ── declared exact types ──────────────────────────────────────────────────────────────────────

#[test]
fn a_binding_declares_an_exact_number_type() {
    assert_all_tiers(
        "## Main\nLet dt be an infinitesimal.\nLet t: Hyperreal be 0.\nSet t to t + dt.\nShow t.",
        "dt",
    );
    assert_all_tiers("## Main\nLet x: Hyperreal be 1 / 2.\nShow x.", "1/2");
    assert_all_tiers("## Main\nLet x: Algebraic be 1 / 3.\nShow x * 3.", "1");
    assert_all_tiers("## Main\nLet x: ExactReal be 2 / 7.\nShow x.", "2/7");
    assert_all_tiers("## Main\nLet n: Hypernatural be 4.\nShow n + 1.", "5");
    assert_all_tiers("## Main\nLet o: Ordinal be 3.\nShow o.", "3");
}

#[test]
fn an_exact_type_refuses_a_literal_of_another_kind() {
    fails_with("## Main\nLet t: Hyperreal be \"zero\".\nShow t.", "Hyperreal");
    fails_with("## Main\nLet o: Ordinal be true.\nShow o.", "Ordinal");
}
