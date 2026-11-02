use super::*;
use crate::ast::stmt::BinaryOpKind;
use crate::semantics::arith;

fn int(n: i64) -> RuntimeValue {
    RuntimeValue::Int(n)
}

fn show(v: &RuntimeValue) -> String {
    v.to_display_string()
}

fn op(kind: BinaryOpKind, a: &RuntimeValue, b: &RuntimeValue) -> Result<RuntimeValue, String> {
    arith::binary_op(kind, a.clone(), b.clone())
}

fn dt() -> RuntimeValue {
    new_infinitesimal("dt", 1, &[0]).unwrap()
}

fn even_dt() -> RuntimeValue {
    new_infinitesimal("dt", 2, &[0]).unwrap()
}

/// `H mod 2` flipped: the lamp after `H` toggles from off is `H mod 2`; `1 − H mod 2` is its
/// complement.
fn lamp(d: &RuntimeValue) -> RuntimeValue {
    let h = generator_of(d).unwrap();
    op(BinaryOpKind::Modulo, &h, &int(2)).unwrap()
}

// ── the infinitesimal ─────────────────────────────────────────────────────────────────────────

#[test]
fn the_infinitesimal_prints_by_its_name_and_combines_with_numbers() {
    let d = dt();
    assert_eq!(show(&d), "dt");
    assert_eq!(show(&op(BinaryOpKind::Add, &d, &int(3)).unwrap()), "3 + dt");
    assert_eq!(show(&op(BinaryOpKind::Divide, &int(1), &d).unwrap()), "1/dt");
    assert_eq!(show(&op(BinaryOpKind::Multiply, &d, &d).unwrap()), "dt^2");
    let h = new_infinitesimal("h", 1, &[0]).unwrap();
    assert_eq!(show(&op(BinaryOpKind::Add, &h, &int(1)).unwrap()), "1 + h");
}

#[test]
fn a_value_with_no_infinitesimal_part_is_an_ordinary_number() {
    let d = dt();
    let back = op(BinaryOpKind::Subtract, &op(BinaryOpKind::Add, &d, &int(1)).unwrap(), &d).unwrap();
    assert!(matches!(back, RuntimeValue::Int(1)), "{back:?}");
    let zero = op(BinaryOpKind::Multiply, &d, &int(0)).unwrap();
    assert!(matches!(zero, RuntimeValue::Int(0)), "{zero:?}");
    let half = op(BinaryOpKind::Divide, &op(BinaryOpKind::Multiply, &d, &int(1)).unwrap(), &op(BinaryOpKind::Multiply, &d, &int(2)).unwrap()).unwrap();
    assert_eq!(show(&half), "1/2");
}

#[test]
fn a_float_meets_the_infinitesimal_as_the_exact_value_it_denotes() {
    let r = op(BinaryOpKind::Add, &dt(), &RuntimeValue::Float(0.5)).unwrap();
    assert_eq!(show(&r), "1/2 + dt");
    let tenth = op(BinaryOpKind::Add, &dt(), &RuntimeValue::Float(0.1)).unwrap();
    assert_eq!(show(&tenth), "3602879701896397/36028797018963968 + dt", "0.1 is a dyadic, not a tenth");
}

#[test]
fn the_order_is_decided_by_transfer() {
    let d = dt();
    let tiny = RuntimeValue::Rational(Rc::new(Rational::from_ratio_i64(1, 1_000_000).unwrap()));
    let lt = arith::binary_op(BinaryOpKind::Lt, d.clone(), tiny).unwrap();
    assert!(matches!(lt, RuntimeValue::Bool(true)));
    let gt = arith::binary_op(BinaryOpKind::Gt, d.clone(), int(0)).unwrap();
    assert!(matches!(gt, RuntimeValue::Bool(true)));
    let eq = arith::binary_op(BinaryOpKind::Eq, d.clone(), int(0)).unwrap();
    assert!(matches!(eq, RuntimeValue::Bool(false)));
}

#[test]
fn standard_parts_and_the_infinitesimal_predicates() {
    let d = dt();
    let x = op(BinaryOpKind::Add, &int(3), &op(BinaryOpKind::Multiply, &int(2), &d).unwrap()).unwrap();
    assert!(matches!(standard_part(&x).unwrap(), RuntimeValue::Int(3)));
    assert!(matches!(is_infinitesimal(&d).unwrap(), RuntimeValue::Bool(true)));
    assert!(matches!(is_infinitesimal(&x).unwrap(), RuntimeValue::Bool(false)));
    let inf = op(BinaryOpKind::Divide, &int(1), &d).unwrap();
    assert!(matches!(is_limited(&inf).unwrap(), RuntimeValue::Bool(false)));
    let err = standard_part(&inf).unwrap_err();
    assert!(err.contains("infinite") && err.contains("1/dt"), "{err}");
    assert!(matches!(standard_part(&int(5)).unwrap(), RuntimeValue::Int(5)));
}

// ── the infinite generator and the lamp ───────────────────────────────────────────────────────

#[test]
fn the_generator_is_an_infinite_integer_written_in_the_infinitesimal() {
    let h = generator_of(&dt()).unwrap();
    assert_eq!(show(&h), "1/dt");
    assert_eq!(show(&op(BinaryOpKind::Add, &h, &int(1)).unwrap()), "1/dt + 1");
    assert_eq!(show(&op(BinaryOpKind::Multiply, &h, &int(2)).unwrap()), "2/dt");
    let big = arith::binary_op(BinaryOpKind::Gt, h.clone(), int(i64::MAX)).unwrap();
    assert!(matches!(big, RuntimeValue::Bool(true)));
    let times_dt = op(BinaryOpKind::Multiply, &h, &dt()).unwrap();
    assert!(matches!(times_dt, RuntimeValue::Int(1)), "⌈1/dt⌉·dt = 1, got {times_dt:?}");
    assert!(matches!(is_limited(&h).unwrap(), RuntimeValue::Bool(false)));
    assert!(generator_of(&int(3)).is_err());
}

#[test]
fn thomsons_lamp_depends_on_the_parity_of_the_generator() {
    let v = lamp(&dt());
    assert_eq!(show(&v), "0 when ⌈1/dt⌉ is even, 1 when it is odd");
    let flipped = op(BinaryOpKind::Subtract, &int(1), &v).unwrap();
    assert_eq!(show(&flipped), "1 when ⌈1/dt⌉ is even, 0 when it is odd");
    let settled = lamp(&even_dt());
    assert!(matches!(settled, RuntimeValue::Int(0)), "{settled:?}");
    let odd = lamp(&new_infinitesimal("dt", 2, &[1]).unwrap());
    assert!(matches!(odd, RuntimeValue::Int(1)));
    let parity_test = arith::binary_op(BinaryOpKind::Eq, v.clone(), int(0)).unwrap();
    assert_eq!(show(&parity_test), "true when ⌈1/dt⌉ is even, false when it is odd");
}

#[test]
fn the_limit_rules_resolve_the_lamp() {
    let v = lamp(&dt());
    assert_eq!(show(&apply_limit(&v, LimitRule::LimitSuperior).unwrap()), "1");
    assert_eq!(show(&apply_limit(&v, LimitRule::LimitInferior).unwrap()), "0");
    assert_eq!(show(&apply_limit(&v, LimitRule::CesaroMean).unwrap()), "1/2");
    let err = apply_limit(&v, LimitRule::Limit).unwrap_err();
    assert!(err.contains("no limit") && err.contains("0 when ⌈1/dt⌉ is even, 1 when it is odd"), "{err}");
    assert!(matches!(apply_limit(&int(7), LimitRule::Limit).unwrap(), RuntimeValue::Int(7)));
    assert_eq!(limit_rule("the Cesàro mean").unwrap(), LimitRule::CesaroMean);
    assert_eq!(limit_rule("the Cesaro mean").unwrap(), LimitRule::CesaroMean);
    assert!(limit_rule("the average").is_err());
}

#[test]
fn contradictory_facts_and_two_infinitesimals_are_refused() {
    let err = new_infinitesimal("dt", 2, &[]).unwrap_err();
    assert!(err.contains("contradict"), "{err}");
    let other = new_infinitesimal("h", 1, &[0]).unwrap();
    let err = op(BinaryOpKind::Add, &dt(), &other).unwrap_err();
    assert!(err.contains("two infinitesimals"), "{err}");
}

// ── algebraic and computable reals ────────────────────────────────────────────────────────────

#[test]
fn algebraic_numbers_downsize_and_order_exactly() {
    let root2 = RuntimeValue::Algebraic(Rc::new(Algebraic::from_i64(2).sqrt().unwrap()));
    assert_eq!(show(&root2), "√2");
    let two = op(BinaryOpKind::Multiply, &root2, &root2).unwrap();
    assert!(matches!(two, RuntimeValue::Int(2)), "{two:?}");
    let lt = arith::binary_op(BinaryOpKind::Lt, root2.clone(), RuntimeValue::Float(1.4142135623730951)).unwrap();
    assert!(matches!(lt, RuntimeValue::Bool(true)), "√2 is just below its nearest double");
    let sum = op(BinaryOpKind::Add, &root2, &dt()).unwrap();
    assert_eq!(show(&sum), "√2 + dt");
}

#[test]
fn computable_reals_compare_exactly_or_fail_loudly() {
    let pi = RuntimeValue::ExactReal(Rc::new(ExactReal::pi()));
    let e = RuntimeValue::ExactReal(Rc::new(ExactReal::e()));
    assert_eq!(show(&pi), "π");
    let lt = arith::binary_op(BinaryOpKind::Lt, pi.clone(), int(4)).unwrap();
    assert!(matches!(lt, RuntimeValue::Bool(true)));
    let a = op(BinaryOpKind::Add, &pi, &e).unwrap();
    let b = op(BinaryOpKind::Add, &e, &pi).unwrap();
    let err = arith::binary_op(BinaryOpKind::Eq, a, b).unwrap_err();
    assert!(err.contains("cannot separate"), "{err}");
    let near = op(BinaryOpKind::Add, &pi, &dt()).unwrap();
    assert_eq!(show(&near), "π + dt + O(dt^2)", "a computable real meets the infinitesimal as a germ");
}

// ── ordinals and provisional values ───────────────────────────────────────────────────────────

#[test]
fn ordinals_add_without_commuting() {
    let w = RuntimeValue::Ordinal(Rc::new(Ordinal::omega()));
    assert_eq!(show(&op(BinaryOpKind::Add, &w, &int(1)).unwrap()), "ω + 1");
    assert_eq!(show(&op(BinaryOpKind::Add, &int(1), &w).unwrap()), "ω");
    assert_eq!(show(&op(BinaryOpKind::Multiply, &w, &int(2)).unwrap()), "ω·2");
    assert!(op(BinaryOpKind::Subtract, &w, &int(1)).is_err());
    let lt = arith::binary_op(BinaryOpKind::Lt, int(1_000_000), w).unwrap();
    assert!(matches!(lt, RuntimeValue::Bool(true)));
}

#[test]
fn provisional_values_print_their_evidence_and_carry_it_through_arithmetic() {
    let p = provisional(RuntimeValue::Bool(false), 1000, 0);
    assert_eq!(show(&p), "false (provisional: checked 1,000 cases, 0 revisions; not proved)");
    let n = provisional(int(41), 1000, 2);
    assert_eq!(show(&op(BinaryOpKind::Add, &n, &int(1)).unwrap()), "42 (provisional: checked 1,000 cases, 2 revisions; not proved)");
}

// ── bounds ────────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_residue_or_a_power_past_its_bound_is_refused_in_the_programs_words() {
    let h = generator_of(&dt()).unwrap();
    assert_eq!(
        op(BinaryOpKind::Modulo, &h, &int(1_000_003)).unwrap_err(),
        "⌈1/dt⌉ would split over 1000003 residue classes; one value holds at most 65,536"
    );
    assert_eq!(
        op(BinaryOpKind::Pow, &h, &int(5000)).unwrap_err(),
        "the power would have degree 5000 in dt; one power reaches at most degree 4,096"
    );
    assert_eq!(
        op(BinaryOpKind::Pow, &dt(), &int(-5000)).unwrap_err(),
        "the power would have degree 5000 in dt; one power reaches at most degree 4,096"
    );
    assert_eq!(
        new_infinitesimal("dt", 1 << 20, &[0]).unwrap_err(),
        "1/dt would split over 1048576 residue classes; one value holds at most 65,536"
    );
    let cube = op(BinaryOpKind::Pow, &h, &int(3)).unwrap();
    assert_eq!(show(&cube), "1/dt^3");
    assert_eq!(show(&op(BinaryOpKind::Pow, &dt(), &int(-2)).unwrap()), "1/dt^2");
}

#[test]
fn a_remainder_of_a_high_power_reads_a_short_period() {
    let h = generator_of(&dt()).unwrap();
    let p = op(BinaryOpKind::Pow, &h, &int(64)).unwrap();
    let parity = op(BinaryOpKind::Modulo, &p, &int(2)).unwrap();
    assert_eq!(show(&parity), "0 when ⌈1/dt⌉ is even, 1 when it is odd");
    let even = generator_of(&even_dt()).unwrap();
    let settled = op(BinaryOpKind::Modulo, &op(BinaryOpKind::Pow, &even, &int(64)).unwrap(), &int(4)).unwrap();
    assert!(matches!(settled, RuntimeValue::Int(0)), "{settled:?}");
}

// ── one representation per value ──────────────────────────────────────────────────────────────

#[test]
fn the_reciprocal_of_the_infinitesimal_is_the_infinite_integer_it_generates() {
    let inv = op(BinaryOpKind::Divide, &int(1), &dt()).unwrap();
    assert!(matches!(inv, RuntimeValue::Hypernatural(_)), "{inv:?}");
    assert_eq!(show(&inv), "1/dt");
    let h = generator_of(&dt()).unwrap();
    assert_eq!(structurally_equal(&inv, &h), Some(true));
    assert!(super::super::compare::values_equal(&inv, &h));
    assert_eq!(show(&op(BinaryOpKind::Modulo, &inv, &int(2)).unwrap()), "0 when ⌈1/dt⌉ is even, 1 when it is odd");
    let triangular = op(
        BinaryOpKind::Divide,
        &op(BinaryOpKind::Multiply, &inv, &op(BinaryOpKind::Add, &inv, &int(1)).unwrap()).unwrap(),
        &int(2),
    )
    .unwrap();
    assert!(matches!(triangular, RuntimeValue::Hypernatural(_)), "H(H+1)/2 is an integer for every H: {triangular:?}");
}

#[test]
fn a_reciprocal_polynomial_that_is_not_always_an_integer_stays_a_hyperreal() {
    let half = op(BinaryOpKind::Divide, &int(1), &op(BinaryOpKind::Multiply, &int(2), &dt()).unwrap()).unwrap();
    assert!(matches!(half, RuntimeValue::Hyperreal(_)), "H/2 is fractional when H is odd: {half:?}");
    let even_half = op(BinaryOpKind::Divide, &int(1), &op(BinaryOpKind::Multiply, &int(2), &even_dt()).unwrap()).unwrap();
    assert!(matches!(even_half, RuntimeValue::Hypernatural(_)), "H/2 is an integer when H is even: {even_half:?}");
    assert_eq!(show(&op(BinaryOpKind::Modulo, &even_half, &int(2)).unwrap()), "0 when ⌈1/dt⌉ ≡ 0 (mod 4), 1 when ⌈1/dt⌉ ≡ 2 (mod 4)");
    let shifted = op(BinaryOpKind::Add, &op(BinaryOpKind::Divide, &int(1), &dt()).unwrap(), &RuntimeValue::Float(0.5)).unwrap();
    assert!(matches!(shifted, RuntimeValue::Hyperreal(_)));
}

// ── integer parts ─────────────────────────────────────────────────────────────────────────────

fn builtin(id: crate::semantics::builtins::BuiltinId, v: &RuntimeValue) -> Result<RuntimeValue, String> {
    crate::semantics::builtins::call_builtin(id, vec![v.clone()])
}

fn floor_of(v: &RuntimeValue) -> RuntimeValue {
    builtin(crate::semantics::builtins::BuiltinId::Floor, v).unwrap()
}

fn ceil_of(v: &RuntimeValue) -> RuntimeValue {
    builtin(crate::semantics::builtins::BuiltinId::Ceil, v).unwrap()
}

fn round_of(v: &RuntimeValue) -> RuntimeValue {
    builtin(crate::semantics::builtins::BuiltinId::Round, v).unwrap()
}

#[test]
fn algebraic_and_computable_reals_have_integer_parts() {
    let root2 = RuntimeValue::Algebraic(Rc::new(Algebraic::from_i64(2).sqrt().unwrap()));
    let neg = op(BinaryOpKind::Subtract, &int(0), &root2).unwrap();
    assert!(matches!(floor_of(&root2), RuntimeValue::Int(1)));
    assert!(matches!(ceil_of(&root2), RuntimeValue::Int(2)));
    assert!(matches!(round_of(&root2), RuntimeValue::Int(1)));
    assert!(matches!(floor_of(&neg), RuntimeValue::Int(-2)));
    assert!(matches!(round_of(&neg), RuntimeValue::Int(-1)));
    let golden = op(BinaryOpKind::Divide, &op(BinaryOpKind::Add, &int(1), &RuntimeValue::Algebraic(Rc::new(Algebraic::from_i64(5).sqrt().unwrap()))).unwrap(), &int(2)).unwrap();
    assert!(matches!(round_of(&golden), RuntimeValue::Int(2)));
    let pi = RuntimeValue::ExactReal(Rc::new(ExactReal::pi()));
    assert!(matches!(floor_of(&pi), RuntimeValue::Int(3)));
    assert!(matches!(ceil_of(&pi), RuntimeValue::Int(4)));
    assert!(matches!(round_of(&pi), RuntimeValue::Int(3)));
    let e = RuntimeValue::ExactReal(Rc::new(ExactReal::e()));
    assert!(matches!(round_of(&e), RuntimeValue::Int(3)));
    let neg_e = op(BinaryOpKind::Subtract, &int(0), &e).unwrap();
    assert!(matches!(floor_of(&neg_e), RuntimeValue::Int(-3)));
    assert!(matches!(round_of(&neg_e), RuntimeValue::Int(-3)));
}

#[test]
fn a_hyperreal_rounds_by_its_standard_part_and_the_sign_of_what_is_left() {
    let d = dt();
    let neg = op(BinaryOpKind::Subtract, &int(0), &d).unwrap();
    assert!(matches!(floor_of(&d), RuntimeValue::Int(0)));
    assert!(matches!(floor_of(&neg), RuntimeValue::Int(-1)));
    assert!(matches!(ceil_of(&d), RuntimeValue::Int(1)));
    assert!(matches!(ceil_of(&neg), RuntimeValue::Int(0)));
    let half = RuntimeValue::Rational(Rc::new(Rational::from_ratio_i64(1, 2).unwrap()));
    assert!(matches!(round_of(&d), RuntimeValue::Int(0)));
    assert!(matches!(round_of(&op(BinaryOpKind::Add, &half, &d).unwrap()), RuntimeValue::Int(1)));
    assert!(matches!(round_of(&op(BinaryOpKind::Subtract, &half, &d).unwrap()), RuntimeValue::Int(0)), "just below a half");
    let minus_half = op(BinaryOpKind::Subtract, &int(0), &half).unwrap();
    assert!(matches!(round_of(&op(BinaryOpKind::Add, &minus_half, &d).unwrap()), RuntimeValue::Int(0)), "just above minus a half");
    assert!(matches!(round_of(&op(BinaryOpKind::Subtract, &minus_half, &d).unwrap()), RuntimeValue::Int(-1)));
}

#[test]
fn an_infinite_hyperreal_rounds_to_an_infinite_integer() {
    let d = dt();
    let inv = op(BinaryOpKind::Divide, &int(1), &d).unwrap();
    assert_eq!(show(&floor_of(&inv)), "1/dt");
    let half = op(BinaryOpKind::Divide, &int(1), &op(BinaryOpKind::Multiply, &int(2), &d).unwrap()).unwrap();
    assert_eq!(show(&floor_of(&half)), "1/(2·dt) when ⌈1/dt⌉ is even, 1/(2·dt) - 1/2 when it is odd");
    assert_eq!(show(&ceil_of(&half)), "1/(2·dt) when ⌈1/dt⌉ is even, 1/(2·dt) + 1/2 when it is odd");
    let shifted = op(BinaryOpKind::Subtract, &inv, &d).unwrap();
    assert_eq!(show(&floor_of(&shifted)), "1/dt - 1");
    let even_half = op(BinaryOpKind::Divide, &int(1), &op(BinaryOpKind::Multiply, &int(2), &even_dt()).unwrap()).unwrap();
    assert_eq!(show(&floor_of(&even_half)), "1/(2·dt)");
    let root2 = RuntimeValue::Algebraic(Rc::new(Algebraic::from_i64(2).sqrt().unwrap()));
    let err = builtin(crate::semantics::builtins::BuiltinId::Floor, &op(BinaryOpKind::Divide, &root2, &d).unwrap()).unwrap_err();
    assert!(err.contains("irrational multiple of a power of 1/dt"), "{err}");
}

#[test]
fn integer_parts_pass_through_infinite_integers_ordinals_dependences_and_evidence() {
    let h = generator_of(&dt()).unwrap();
    assert_eq!(show(&floor_of(&h)), "1/dt");
    assert_eq!(show(&round_of(&h)), "1/dt");
    let w = RuntimeValue::Ordinal(Rc::new(Ordinal::omega()));
    assert_eq!(show(&ceil_of(&w)), "ω");
    let lamp_half = op(BinaryOpKind::ExactDivide, &lamp(&dt()), &int(2)).unwrap();
    assert_eq!(show(&ceil_of(&lamp_half)), "0 when ⌈1/dt⌉ is even, 1 when it is odd");
    let p = provisional(RuntimeValue::Rational(Rc::new(Rational::from_ratio_i64(7, 2).unwrap())), 1000, 1);
    assert_eq!(show(&floor_of(&p)), "3 (provisional: checked 1,000 cases, 1 revision; not proved)");
}

#[test]
fn exact_division_stays_exact_on_every_branch_and_floor_division_floors() {
    let v = lamp(&dt());
    assert_eq!(show(&op(BinaryOpKind::ExactDivide, &v, &int(2)).unwrap()), "0 when ⌈1/dt⌉ is even, 1/2 when it is odd");
    assert_eq!(show(&op(BinaryOpKind::Divide, &v, &int(2)).unwrap()), "0", "the integer `/` floors 1/2 to 0 on the odd branch too");
    let three = op(BinaryOpKind::Add, &v, &int(2)).unwrap();
    assert_eq!(show(&op(BinaryOpKind::ExactDivide, &int(1), &three).unwrap()), "1/2 when ⌈1/dt⌉ is even, 1/3 when it is odd");
    let p = provisional(int(7), 10, 0);
    assert_eq!(show(&op(BinaryOpKind::ExactDivide, &p, &int(2)).unwrap()), "7/2 (provisional: checked 10 cases, 0 revisions; not proved)");
}

#[test]
fn a_dependence_of_integer_polynomials_is_one_infinite_integer() {
    let h = generator_of(&dt()).unwrap();
    let v = op(BinaryOpKind::Multiply, &lamp(&dt()), &h).unwrap();
    assert!(matches!(v, RuntimeValue::Hypernatural(_)), "{v:?}");
    assert_eq!(show(&v), "0 when ⌈1/dt⌉ is even, 1/dt when it is odd");
    assert_eq!(show(&op(BinaryOpKind::Modulo, &v, &int(2)).unwrap()), "0 when ⌈1/dt⌉ is even, 1 when it is odd");
    let half = op(BinaryOpKind::ExactDivide, &h, &int(2)).unwrap();
    let floor_half = op(
        BinaryOpKind::Subtract,
        &half,
        &op(BinaryOpKind::ExactDivide, &lamp(&dt()), &int(2)).unwrap(),
    )
    .unwrap();
    assert!(matches!(floor_half, RuntimeValue::Hypernatural(_)), "(H − H mod 2)/2 is whole on every class: {floor_half:?}");
    assert_eq!(structurally_equal(&floor_half, &floor_of(&half)), Some(true));
    let shifted = op(BinaryOpKind::Add, &lamp(&dt()), &h).unwrap();
    assert_eq!(show(&op(BinaryOpKind::Modulo, &shifted, &int(2)).unwrap()), "0", "H + H mod 2 is always even");
}

#[test]
fn a_dependence_nested_in_a_dependence_is_flattened() {
    let h = generator_of(&dt()).unwrap();
    let thirds = op(BinaryOpKind::Modulo, &h, &int(3)).unwrap();
    let scale = match &dt() {
        RuntimeValue::Hyperreal(x) => x.scale.clone(),
        _ => unreachable!(),
    };
    let nested = Depends::from_fn(2, &scale.residues, |r| Branch(if r == 0 { int(5) } else { thirds.clone() })).unwrap();
    let flat = from_depends(nested, &scale);
    assert_eq!(
        show(&flat),
        "5 when ⌈1/dt⌉ ≡ 0 (mod 6), 1 when ⌈1/dt⌉ ≡ 1 (mod 6), 5 when ⌈1/dt⌉ ≡ 2 (mod 6), 0 when ⌈1/dt⌉ ≡ 3 (mod 6), 5 when ⌈1/dt⌉ ≡ 4 (mod 6), 2 when ⌈1/dt⌉ ≡ 5 (mod 6)"
    );
    let RuntimeValue::Depends(d) = &flat else { panic!("{flat:?}") };
    assert!(d.value.branches().all(|(_, b)| !matches!(b.0, RuntimeValue::Depends(_))));
}
