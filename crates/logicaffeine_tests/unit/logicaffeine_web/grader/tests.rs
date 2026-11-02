use super::*;

#[test]
fn test_exact_match() {
    let result = check_answer("∀x(D(x) → B(x))", "∀x(D(x) → B(x))");
    assert!(result.correct, "Exact match should be correct");
}

#[test]
fn test_whitespace_normalization() {
    let result = check_answer("∀x( D(x) → B(x) )", "∀x(D(x)→B(x))");
    assert!(result.correct, "Whitespace should be normalized");
}

#[test]
fn test_latex_to_unicode() {
    let result = check_answer("\\forall x(D(x) \\supset B(x))", "∀x(D(x) → B(x))");
    assert!(result.correct, "LaTeX should normalize to Unicode");
}

#[test]
fn test_ascii_shortcuts() {
    let result = check_answer("D(x) & B(x)", "D(x) ∧ B(x)");
    assert!(result.correct, "ASCII & should match ∧");
}

#[test]
fn test_commutative_conjunction() {
    let result = check_answer("∃x(B(x) ∧ D(x))", "∃x(D(x) ∧ B(x))");
    assert!(result.correct, "Conjunction should be commutative");
}

#[test]
fn test_wrong_quantifier() {
    let result = check_answer("∃x(D(x) → B(x))", "∀x(D(x) → B(x))");
    assert!(!result.correct, "Wrong quantifier should not match");
    assert!(result.partial, "Should get partial credit");
}

#[test]
fn test_wrong_connective() {
    let result = check_answer("∀x(D(x) ∧ B(x))", "∀x(D(x) → B(x))");
    assert!(!result.correct, "Wrong connective should not match");
    assert!(result.partial, "Should get partial credit for structure");
}

#[test]
fn test_completely_wrong() {
    let result = check_answer("P(a)", "∀x(D(x) → B(x))");
    assert!(!result.correct);
    assert!(!result.partial);
}

#[test]
fn test_normalize_arrow() {
    let normalized = normalize_logic("A -> B");
    assert_eq!(normalized, "A→B");
}

#[test]
fn test_normalize_biconditional() {
    let normalized = normalize_logic("A <-> B");
    assert_eq!(normalized, "A↔B");
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

const SURFACE: &str = "∀x(D(x) → ∃y(C(y) ∧ Chase(x, y)))";
const INVERSE: &str = "∃y(C(y) ∧ ∀x(D(x) → Chase(x, y)))";

#[test]
fn readings_in_either_order_are_all_correct() {
    let readings = strings(&[SURFACE, INVERSE]);
    let g = check_readings(&strings(&[SURFACE, INVERSE]), &readings);
    assert_eq!(g, ReadingsGrade { matched: 2, total: 2 });
    assert!(g.all_correct());
    let g = check_readings(&strings(&[INVERSE, SURFACE]), &readings);
    assert!(g.all_correct(), "order must not matter: {g:?}");
}

#[test]
fn one_reading_given_twice_covers_only_one() {
    let g = check_readings(&strings(&[SURFACE, SURFACE]), &strings(&[SURFACE, INVERSE]));
    assert_eq!(g, ReadingsGrade { matched: 1, total: 2 });
    assert!(!g.all_correct());
}

#[test]
fn a_blank_answer_matches_nothing() {
    let g = check_readings(&strings(&[SURFACE, "   "]), &strings(&[SURFACE, INVERSE]));
    assert_eq!(g.matched, 1);
    assert!(!g.all_correct());
}

#[test]
fn readings_are_graded_with_the_same_normalisation_as_translations() {
    let spaced_ascii = "Ax(D(x) -> Ey(C(y) & Chase(x, y)))".replace("Ax", "∀x").replace("Ey", "∃y");
    let g = check_readings(&[spaced_ascii, INVERSE.to_string()], &strings(&[SURFACE, INVERSE]));
    assert!(g.all_correct(), "{g:?}");
}

#[test]
fn a_wrong_answer_leaves_its_reading_unmatched() {
    let g = check_readings(&strings(&["∀x(D(x))", INVERSE]), &strings(&[SURFACE, INVERSE]));
    assert_eq!(g, ReadingsGrade { matched: 1, total: 2 });
}

#[test]
fn more_answers_than_readings_still_needs_each_reading_once() {
    let g = check_readings(&strings(&[SURFACE, "junk", INVERSE]), &strings(&[SURFACE, INVERSE]));
    assert!(g.all_correct(), "{g:?}");
    let g = check_readings(&strings(&[SURFACE]), &strings(&[SURFACE, INVERSE]));
    assert!(!g.all_correct());
}

#[test]
fn a_sentence_with_no_readings_is_never_correct() {
    let g = check_readings(&strings(&[SURFACE]), &[]);
    assert_eq!(g, ReadingsGrade { matched: 0, total: 0 });
    assert!(!g.all_correct());
}

#[test]
fn matching_is_an_assignment_not_a_greedy_scan() {
    // "either" fits both readings, "only1" fits just r1. A greedy scan hands "either" the
    // first reading and strands "only1"; a proper matching gives "either" r2.
    let fits = |answer: &str, reading: &str| answer == "either" || (answer == "only1" && reading == "r1");
    let answers = strings(&["either", "only1"]);
    let readings = strings(&["r1", "r2"]);
    assert_eq!(match_readings(&answers, &readings, fits), 2);
    assert_eq!(match_readings(&strings(&["only1", "only1"]), &readings, fits), 1);
    assert_eq!(match_readings(&[], &readings, fits), 0);
}

#[test]
fn matching_finds_a_full_assignment_whenever_one_exists() {
    // Exhaustive over every compatibility relation between 3 answers and 3 readings: the
    // matched count must equal the best assignment found by brute force.
    let answers = strings(&["a0", "a1", "a2"]);
    let readings = strings(&["r0", "r1", "r2"]);
    for mask in 0u32..(1 << 9) {
        let fits = |a: &str, r: &str| {
            let i = a[1..].parse::<u32>().unwrap();
            let j = r[1..].parse::<u32>().unwrap();
            mask & (1 << (i * 3 + j)) != 0
        };
        let mut best = 0;
        for p in [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
            let n = (0..3).filter(|&i| mask & (1 << (i * 3 + p[i])) != 0).count();
            best = best.max(n);
        }
        assert_eq!(match_readings(&answers, &readings, fits), best, "relation mask {mask:09b}");
    }
}

#[test]
fn the_engines_own_scope_readings_grade_as_correct() {
    let readings = logicaffeine_language::compile_all_scopes("Every student reads a book.")
        .expect("the engine scopes its own sentence");
    assert!(readings.len() >= 2, "expected a scope ambiguity, got {readings:?}");
    let mut reversed = readings.clone();
    reversed.reverse();
    let g = check_readings(&reversed, &readings);
    assert!(g.all_correct(), "{g:?} for {readings:?}");
}

/// "All birds fly" as a textbook writes it, and as LOGOS shows it: the dictionary says a
/// bird is an animal, so LOGOS adds `Animal(x)` beside `Bird(x)`.
const BIRDS_FLY: &str = "∀x((Bird(x) → ∃e(Fly(e) ∧ Agent(e, x))))";
const BIRDS_FLY_AS_SHOWN: &str = "∀x(((Bird(x) ∧ Animal(x)) → ∃e(Fly(e) ∧ Agent(e, x))))";

#[test]
fn who_does_what_to_whom_matters() {
    let key = "∃e(See(e) ∧ Agent(e, Mary) ∧ Theme(e, John))";
    assert!(!check_answer("∃e(See(e) ∧ Agent(e, John) ∧ Theme(e, Mary))", key).correct);
}

#[test]
fn a_bound_variable_may_have_any_name() {
    assert!(check_answer("∀y((Dog(y) → Bark(y)))", "∀x((Dog(x) → Bark(x)))").correct);
    assert!(check_answer("∃u(Sleep(u) ∧ Agent(u, Peter))", "∃e(Sleep(e) ∧ Agent(e, Peter))").correct);
}

#[test]
fn a_free_name_is_not_a_bound_variable() {
    assert!(!check_answer("∃e(Sleep(e) ∧ Agent(e, e))", "∃e(Sleep(e) ∧ Agent(e, Peter))").correct);
}

#[test]
fn a_name_may_be_written_in_any_case() {
    assert!(check_answer("∃e(Sleep(e) ∧ Agent(e, peter))", "∃e(Sleep(e) ∧ Agent(e, Peter))").correct);
}

#[test]
fn conjuncts_may_come_in_any_order_and_grouping() {
    assert!(check_answer("∃x(C(x) ∧ (B(x) ∧ A(x)))", "∃x(((A(x) ∧ B(x)) ∧ C(x)))").correct);
    assert!(check_answer("∃e(Agent(e, Peter) ∧ Sleep(e))", "∃e(Sleep(e) ∧ Agent(e, Peter))").correct);
}

#[test]
fn a_conjunct_left_out_is_missed() {
    assert!(!check_answer("∃e(Sleep(e))", "∃e(Sleep(e) ∧ Agent(e, Peter))").correct);
}

#[test]
fn an_operator_over_a_formula_reads_the_formula() {
    let key = "HAB(∃e(Sleep(e) ∧ Agent(e, Peter)))";
    assert!(check_answer("HAB(∃e(Agent(e, Peter) ∧ Sleep(e)))", key).correct);
    assert!(!check_answer("HAB(∃e(Run(e) ∧ Agent(e, Peter)))", key).correct);
    assert!(!check_answer("¬HAB(∃e(Sleep(e) ∧ Agent(e, Peter)))", key).correct);
    assert!(!check_answer("∃e(Sleep(e) ∧ Agent(e, Peter))", key).correct);
}

#[test]
fn a_modal_is_read_with_or_without_its_degree() {
    let key = "◇_{0.3} ∃e(Wish(e) ∧ Agent(e, Amanda))";
    assert!(check_answer("◇ ∃e(Wish(e) ∧ Agent(e, Amanda))", key).correct);
    assert!(check_answer(key, "◇∃e(Wish(e) ∧ Agent(e, Amanda))").correct);
    assert!(!check_answer("□ ∃e(Wish(e) ∧ Agent(e, Amanda))", key).correct);
    assert!(!check_answer("◇ ∃e(Sleep(e) ∧ Agent(e, Amanda))", key).correct);
}

#[test]
fn a_body_in_parentheses_is_read_like_one_without() {
    let key = "∀x((Dog(x) → Bark(x)))";
    assert!(check_answer("∀x(Dog(x) → Bark(x))", key).correct);
    assert!(check_answer(key, "∀x(Dog(x) → Bark(x))").correct);
}

#[test]
fn a_different_noun_inside_parentheses_is_a_different_claim() {
    assert!(!check_answer("∀x((Dog(x) → ∃e(Fly(e) ∧ Agent(e, x))))", BIRDS_FLY).correct);
    assert!(!check_answer("∀x((Bird(x) → ∃e(Swim(e) ∧ Agent(e, x))))", BIRDS_FLY).correct);
}

#[test]
fn a_category_the_dictionary_adds_is_optional() {
    assert!(check_answer(BIRDS_FLY, BIRDS_FLY_AS_SHOWN).correct, "the textbook answer against the shown key");
    assert!(check_answer(BIRDS_FLY_AS_SHOWN, BIRDS_FLY).correct, "the shown answer against the textbook key");
}

#[test]
fn every_category_the_dictionary_adds_is_optional() {
    let key = "∃x((Cat(x) ∧ ∃e(Sleep(e) ∧ Agent(e, x))))";
    for answer in [
        "∃x((((Cat(x) ∧ Animal(x)) ∧ Mammal(x)) ∧ ∃e(Sleep(e) ∧ Agent(e, x))))",
        "∃x(((Cat(x) ∧ Mammal(x)) ∧ ∃e(Sleep(e) ∧ Agent(e, x))))",
        "∃x(((Animal(x) ∧ Cat(x)) ∧ ∃e(Sleep(e) ∧ Agent(e, x))))",
    ] {
        assert!(check_answer(answer, key).correct, "{answer} against {key}");
        assert!(check_answer(key, answer).correct, "{key} against {answer}");
    }
}

#[test]
fn a_predicate_that_is_no_category_of_the_noun_is_not_optional() {
    let answer = "∀x(((Bird(x) ∧ Red(x)) → ∃e(Fly(e) ∧ Agent(e, x))))";
    assert!(!check_answer(answer, BIRDS_FLY).correct);
    assert!(!check_answer(BIRDS_FLY, answer).correct);
}

#[test]
fn a_category_of_something_else_is_not_optional() {
    let key = "∀x(∀y((Bird(x) → See(x, y))))";
    let answer = "∀x(∀y(((Bird(x) ∧ Animal(y)) → See(x, y))))";
    assert!(!check_answer(answer, key).correct);
    assert!(!check_answer(key, answer).correct);
}

#[test]
fn a_category_does_not_stand_in_for_its_noun() {
    let answer = "∀x((Animal(x) → ∃e(Fly(e) ∧ Agent(e, x))))";
    assert!(!check_answer(answer, BIRDS_FLY).correct);
    assert!(!check_answer(answer, BIRDS_FLY_AS_SHOWN).correct);
}
