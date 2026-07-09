//! Type-directed exact division: `/` floors for `Int` by default (the integer
//! programmer default — unchanged), but a `Rational`-typed context makes it EXACT
//! (`7 / 2 → 7/2`). The `resolve_divisions` pass rewrites `Divide → ExactDivide`
//! only where the result flows into a `Rational`, so existing floor code is untouched.

mod common;
use common::{assert_interpreter_output, assert_interpreter_output_lines};

#[test]
fn unconstrained_int_division_still_floors() {
    // The default is unchanged: int / int floors, exactly as before.
    assert_interpreter_output("## Main\nShow 7 / 2.", "3");
    assert_interpreter_output("## Main\nLet m be (5 + 4) / 2.\nShow m.", "4");
    assert_interpreter_output("## Main\nLet m be 17 / 5.\nShow m.", "3");
}

#[test]
fn rational_typed_binding_makes_division_exact() {
    // A Rational binding makes `/` exact — the result is the fraction, not floored.
    assert_interpreter_output("## Main\nLet x: Rational be 7 / 2.\nShow x.", "7/2");
    assert_interpreter_output("## Main\nLet x: Rational be 1 / 3.\nShow x.", "1/3");
    assert_interpreter_output("## Main\nLet x: Rational be 17 / 5.\nShow x.", "17/5");
}

#[test]
fn an_evenly_dividing_rational_downsizes_to_int() {
    // 6 / 2 is whole → it reduces to the Int 3 even in a Rational context.
    assert_interpreter_output("## Main\nLet x: Rational be 6 / 2.\nShow x.", "3");
    assert_interpreter_output("## Main\nLet x: Rational be 100 / 4.\nShow x.", "25");
}

#[test]
fn exact_division_propagates_across_the_arithmetic_spine() {
    // Every `/` on the arithmetic spine of a Rational value is exact: 1/3 + 1/6 = 1/2.
    assert_interpreter_output("## Main\nLet x: Rational be 1 / 3 + 1 / 6.\nShow x.", "1/2");
    // 1/2 * 2/3 = 1/3.
    assert_interpreter_output("## Main\nLet x: Rational be 1 / 2 * (2 / 3).\nShow x.", "1/3");
}

#[test]
fn negative_exact_division() {
    assert_interpreter_output("## Main\nLet x: Rational be -7 / 2.\nShow x.", "-7/2");
}

#[test]
fn rational_return_type_makes_function_division_exact() {
    // A function whose return type is Rational makes its `Return n / 2` exact.
    let src = "## To half (n: Int) -> Rational:\n    Return n / 2.\n\n\
               ## Main\nLet r: Rational be half(7).\nShow r.";
    assert_interpreter_output(src, "7/2");
}

#[test]
fn a_rational_operand_propagates_exactness_through_plain_division() {
    // `a` is a Rational at runtime, so `a / 3` is exact even with no annotation on the
    // result — the operand carries the exactness through ordinary `/`.
    let src = "## Main\nLet a: Rational be 1 / 2.\nLet b be a / 3.\nShow b.";
    assert_interpreter_output(src, "1/6");
}

#[test]
fn floor_and_exact_division_coexist_for_the_same_literals() {
    // Identical `7 / 2`, two contexts: the Int binding floors, the Rational is exact.
    let src = "## Main\nLet f be 7 / 2.\nLet x: Rational be 7 / 2.\nShow f.\nShow x.";
    assert_interpreter_output_lines(src, &["3", "7/2"]);
}

#[test]
fn rational_arithmetic_after_an_exact_division_stays_exact() {
    // Once a value is a Rational, further arithmetic with it is exact.
    let src = "## Main\nLet x: Rational be 7 / 2.\nLet y be x + 1.\nShow y.";
    assert_interpreter_output(src, "9/2");
}

// =====================================================================
// Exhaustive: the floor default is UNCHANGED for every shape
// =====================================================================

#[test]
fn floor_default_is_exhaustively_unchanged() {
    for (src, want) in [
        ("Show 7 / 2.", "3"),
        ("Show 17 / 5.", "3"),
        ("Show 99 / 100.", "0"),
        ("Show 100 / 3.", "33"),
        ("Show 0 / 5.", "0"),
        ("Show 6 / 2.", "3"),
        ("Show 10 / 2.", "5"),
        // Integer division truncates toward zero (the C/Rust default), unchanged.
        ("Let f be -7 / 2.\nShow f.", "-3"),
        ("Let f be 7 / -2.\nShow f.", "-3"),
        // Nested / chained int divisions still floor at each step.
        ("Show 100 / 3 / 2.", "16"),
        ("Let m be (3 + 4) / 2.\nShow m.", "3"),
    ] {
        assert_interpreter_output(&format!("## Main\n{src}"), want);
    }
}

// =====================================================================
// Exhaustive: a Rational context makes `/` exact (fractions, reductions, signs)
// =====================================================================

#[test]
fn rational_context_is_exhaustively_exact() {
    for (expr, want) in [
        ("7 / 2", "7/2"),
        ("1 / 3", "1/3"),
        ("17 / 5", "17/5"),
        ("22 / 7", "22/7"),
        ("6 / 8", "3/4"),   // reduces
        ("10 / 4", "5/2"),  // reduces
        ("100 / 8", "25/2"),
        ("-7 / 2", "-7/2"),
        ("7 / -2", "-7/2"), // sign normalizes onto the numerator
        ("-6 / -8", "3/4"),
        ("0 / 5", "0"),     // zero
        ("6 / 2", "3"),     // even → downsizes to Int
        ("100 / 4", "25"),  // even → downsizes
        ("9 / 3", "3"),
    ] {
        let src = format!("## Main\nLet x: Rational be {expr}.\nShow x.");
        assert_interpreter_output(&src, want);
    }
}

// =====================================================================
// Operand propagation: a Rational operand makes ordinary `/` exact, everywhere
// =====================================================================

#[test]
fn a_rational_in_a_show_position_division_does_not_crash_and_is_exact() {
    // `a / 3` lives in a Show (a statement the resolver does not rewrite), so the
    // abstract-interp defense (ExactDivide / Rational → not "proven Int") is what keeps
    // the integer strength reducer from misfiring; the runtime divides exactly.
    let src = "## Main\nLet a: Rational be 1 / 2.\nShow a / 3.";
    assert_interpreter_output(src, "1/6");
}

#[test]
fn exactness_chains_through_reassignment() {
    let src = "## Main\n\
               Let mutable x: Rational be 1 / 2.\n\
               Set x to x + 1 / 2.\n\
               Show x.";
    assert_interpreter_output(src, "1");
}

#[test]
fn exactness_chains_through_several_operations() {
    // ((1/2 / 3) * 4) - 1/6  =  (1/6 * 4) - 1/6  =  2/3 - 1/6  =  1/2.
    let src = "## Main\n\
               Let a: Rational be 1 / 2.\n\
               Let b be a / 3.\n\
               Let c be b * 4.\n\
               Let d be c - 1 / 6.\n\
               Show d.";
    assert_interpreter_output(src, "1/2");
}

// =====================================================================
// Functions: Rational params and return types
// =====================================================================

#[test]
fn rational_parameter_division_is_exact_inside_the_function() {
    let src = "## To third (r: Rational) -> Rational:\n    Return r / 3.\n\n\
               ## Main\nLet q: Rational be 1 / 2.\nLet r: Rational be third(q).\nShow r.";
    assert_interpreter_output(src, "1/6");
}

#[test]
fn rational_returning_function_composes() {
    let src = "## To half (n: Int) -> Rational:\n    Return n / 2.\n\n\
               ## Main\nLet r: Rational be half(7) + half(3).\nShow r.";
    // 7/2 + 3/2 = 10/2 = 5 (even → Int).
    assert_interpreter_output(src, "5");
}

// =====================================================================
// The key SAFETY property: an index `/` always floors, even amid Rationals
// =====================================================================

#[test]
fn an_index_division_floors_even_when_rationals_are_present() {
    // The index `i = 7 / 2` floors to 3 (1-based → the 3rd element). `i` is NOT a
    // Rational (no annotation, no Rational operand) even though a Rational `r` is in
    // scope — algorithms that index by `(lo+hi)/2` keep working.
    let src = "## Main\n\
               Let r: Rational be 1 / 3.\n\
               Let xs be [10, 20, 30, 40, 50].\n\
               Let i be 7 / 2.\n\
               Show item i of xs.";
    assert_interpreter_output(src, "30");
}

// =====================================================================
// Modulo is unchanged; mixed Float; the anti-JSON exactness headline
// =====================================================================

#[test]
fn modulo_is_unchanged() {
    assert_interpreter_output("## Main\nShow 7 % 2.", "1");
    assert_interpreter_output("## Main\nShow 17 % 5.", "2");
}

#[test]
fn a_float_operand_makes_a_rational_expression_float() {
    // 7/2 is the Rational 3.5; + 0.5 brings in a Float → Float 4.0, shown as "4".
    let src = "## Main\nLet x: Rational be 7 / 2.\nLet y be x + 0.5.\nShow y.";
    assert_interpreter_output(src, "4");
}

#[test]
fn three_thirds_sum_to_exactly_one() {
    // The anti-JSON headline: 1/3 + 1/3 + 1/3 is EXACTLY 1 (an f64 would give 0.999…).
    let src = "## Main\nLet x: Rational be 1 / 3 + 1 / 3 + 1 / 3.\nShow x.";
    assert_interpreter_output(src, "1");
}

#[test]
fn a_rational_sum_that_reduces_to_a_whole_number_shows_as_an_int() {
    // 1/6 + 1/3 + 1/2 = 1.
    let src = "## Main\nLet x: Rational be 1 / 6 + 1 / 3 + 1 / 2.\nShow x.";
    assert_interpreter_output(src, "1");
}

#[test]
fn an_integer_added_to_a_rational_variable_widens_to_rational() {
    // r is a Rational variable; the Int 3 widens to a rational → 7/2 (no explicit cast).
    assert_interpreter_output("## Main\nLet r: Rational be 1 / 2.\nLet s be r + 3.\nShow s.", "7/2");
}

#[test]
fn an_integer_literal_in_a_rational_binding_widens() {
    // 3 + 1/2 in a Rational context → 7/2 (the 3 widens up the numeric tower).
    assert_interpreter_output("## Main\nLet x: Rational be 3 + 1 / 2.\nShow x.", "7/2");
}

#[test]
fn int_and_rational_mix_across_minus_and_times() {
    // 1/2 * 4 = 2 (reduces to a whole) ; 5 - 1/2 = 9/2.
    assert_interpreter_output("## Main\nLet r: Rational be 1 / 2.\nLet a be r * 4.\nShow a.", "2");
    assert_interpreter_output("## Main\nLet r: Rational be 1 / 2.\nLet b be 5 - r.\nShow b.", "9/2");
    // Multiplication, both operand orders, non-reducing and reducing.
    assert_interpreter_output("## Main\nLet r: Rational be 1 / 2.\nLet a be r * 3.\nShow a.", "3/2");
    assert_interpreter_output("## Main\nLet r: Rational be 1 / 2.\nLet a be 6 * r.\nShow a.", "3");
}

#[test]
fn every_division_combination_of_int_and_rational_is_exact() {
    // Rational / Int :  (1/2) / 2 = 1/4
    assert_interpreter_output("## Main\nLet r: Rational be 1 / 2.\nLet q be r / 2.\nShow q.", "1/4");
    // Int / Rational :  3 / (1/2) = 6
    assert_interpreter_output("## Main\nLet r: Rational be 1 / 2.\nLet q be 3 / r.\nShow q.", "6");
    // Rational / Rational :  (1/2) / (1/3) = 3/2
    assert_interpreter_output(
        "## Main\nLet a: Rational be 1 / 2.\nLet b: Rational be 1 / 3.\nLet q be a / b.\nShow q.",
        "3/2",
    );
    // A rational over itself is exactly 1.
    assert_interpreter_output("## Main\nLet a: Rational be 3 / 4.\nLet q be a / a.\nShow q.", "1");
}

#[test]
fn floor_ceiling_round_of_a_rational_are_exact() {
    // 7/2 = 3.5 : floor 3, ceiling 4, round 4 (ties away from zero).
    assert_interpreter_output("## Main\nLet r: Rational be 7 / 2.\nShow floor(r).", "3");
    assert_interpreter_output("## Main\nLet r: Rational be 7 / 2.\nShow ceil(r).", "4");
    assert_interpreter_output("## Main\nLet r: Rational be 7 / 2.\nShow round(r).", "4");
    // -7/2 = -3.5 : floor → -4 (toward −∞), ceiling → -3 (toward +∞), round → -4.
    assert_interpreter_output("## Main\nLet r: Rational be -7 / 2.\nShow floor(r).", "-4");
    assert_interpreter_output("## Main\nLet r: Rational be -7 / 2.\nShow ceil(r).", "-3");
    assert_interpreter_output("## Main\nLet r: Rational be -7 / 2.\nShow round(r).", "-4");
}

#[test]
fn absolute_value_of_a_rational_stays_a_rational() {
    assert_interpreter_output("## Main\nLet r: Rational be -7 / 2.\nShow abs(r).", "7/2");
}

#[test]
fn a_long_constant_rational_chain_lightning_folds_to_its_closed_form() {
    // 1/2 + 1/3 + 1/4 + 1/5 + 1/6  =  (30+20+15+12+10)/60  =  87/60  =  29/20.
    // The whole chain collapses to the single closed-form constant at compile time.
    let src = "## Main\nLet x: Rational be 1 / 2 + 1 / 3 + 1 / 4 + 1 / 5 + 1 / 6.\nShow x.";
    assert_interpreter_output(src, "29/20");
    // A chain that reduces to a whole number: 1/2 + 1/3 + 1/6 = 1.
    assert_interpreter_output("## Main\nLet x: Rational be 1 / 2 + 1 / 3 + 1 / 6.\nShow x.", "1");
    // Mixed +/-/* in one constant chain: (2/3 * 3/4) + 1/2 - 1/4 = 1/2 + 1/2 - 1/4 = 3/4.
    assert_interpreter_output(
        "## Main\nLet x: Rational be 2 / 3 * (3 / 4) + 1 / 2 - 1 / 4.\nShow x.",
        "3/4",
    );
}

#[test]
fn a_non_whole_rational_used_as_an_index_errors_gracefully() {
    // The VISIBLE footgun: a non-whole Rational is not a valid index. This must be a
    // clean error — never a panic or a silently-floored read.
    let src = "## Main\nLet i: Rational be 1 / 3.\nLet xs be [10, 20, 30].\nShow item i of xs.";
    let r = common::run_interpreter(src);
    assert!(!r.success, "a non-whole Rational index must be rejected.\nOutput: {}", r.output);
}

// =====================================================================
// Generative property tests: EVERY small rational must work — the proof that
// Logos is not "more broken than JavaScript". Each random case runs through the
// real language and is checked against the exact BigInt-backed oracle.
// =====================================================================

struct SplitMix64(u64);
impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// A nonzero value in `-range..=range`.
    fn nonzero(&mut self, range: i64) -> i64 {
        loop {
            let v = (self.next() % (2 * range as u64 + 1)) as i64 - range;
            if v != 0 {
                return v;
            }
        }
    }
    fn any(&mut self, range: i64) -> i64 {
        (self.next() % (2 * range as u64 + 1)) as i64 - range
    }
}

#[test]
fn every_small_rational_literal_matches_the_exact_oracle() {
    use logicaffeine_base::Rational;
    let mut rng = SplitMix64(0x5A7F_104E_2C19_8B63);
    for _ in 0..400 {
        let n = rng.any(250);
        let d = rng.nonzero(250);
        let want = Rational::from_ratio_i64(n, d).unwrap().to_string();
        let src = format!("## Main\nLet x: Rational be ({n}) / ({d}).\nShow x.");
        let r = common::run_interpreter(&src);
        assert!(r.success, "the language rejected {n}/{d}: {}", r.error);
        assert_eq!(r.output.trim(), want, "{n} / {d} (Rational) diverged from the exact oracle");
    }
}

#[test]
fn random_rational_arithmetic_matches_the_exact_oracle() {
    use logicaffeine_base::Rational;
    let mut rng = SplitMix64(0xC0FF_EE12_3456_789A);
    for _ in 0..400 {
        let (a, b) = (rng.any(60), rng.nonzero(60));
        let (c, d) = (rng.any(60), rng.nonzero(60));
        let x = Rational::from_ratio_i64(a, b).unwrap();
        let y = Rational::from_ratio_i64(c, d).unwrap();
        let (op, want) = match rng.next() % 3 {
            0 => ("+", x.add(&y)),
            1 => ("-", x.sub(&y)),
            _ => ("*", x.mul(&y)),
        };
        let src = format!(
            "## Main\nLet x: Rational be (({a}) / ({b})) {op} (({c}) / ({d})).\nShow x.",
        );
        let r = common::run_interpreter(&src);
        assert!(r.success, "the language rejected ({a}/{b}) {op} ({c}/{d}): {}", r.error);
        assert_eq!(
            r.output.trim(),
            want.to_string(),
            "({a}/{b}) {op} ({c}/{d}) diverged from the exact oracle"
        );
    }
}
