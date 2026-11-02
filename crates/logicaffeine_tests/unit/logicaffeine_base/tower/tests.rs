//! The exact core run on a value model of its own: integers and rationals with floor and exact
//! division, booleans, text, and the exact types. Nothing here knows the interpreter, so a test
//! that passes proves the core needs only what [`TowerValue`] asks for.

use super::*;
use crate::numeric::{BigInt, Rational};

#[derive(Clone, Debug)]
pub(crate) enum Val {
    Int(BigInt),
    Rat(Rational),
    Bool(bool),
    Text(String),
    Nothing,
    Alg(Algebraic),
    Real(ExactReal),
    Hyper(HyperValue),
    Nat(NaturalValue),
    Ord(Ordinal),
    Dep(Box<DependsValue<Val>>),
    Conv(Box<Converging<Val>>),
    List(Vec<Val>),
}

impl TowerValue for Val {
    fn exact(&self) -> Option<ExactRef<'_, Self>> {
        Some(match self {
            Val::Alg(a) => ExactRef::Algebraic(a),
            Val::Real(x) => ExactRef::Real(x),
            Val::Hyper(h) => ExactRef::Hyper(h),
            Val::Nat(n) => ExactRef::Natural(n),
            Val::Ord(o) => ExactRef::Ordinal(o),
            Val::Dep(d) => ExactRef::Depends(d),
            Val::Conv(c) => ExactRef::Converging(c),
            _ => return None,
        })
    }

    fn standard_rational(&self) -> Option<Rational> {
        match self {
            Val::Int(n) => Some(Rational::from_bigint(n.clone())),
            Val::Rat(r) => Some(r.clone()),
            _ => None,
        }
    }

    fn integer(&self) -> Option<BigInt> {
        match self {
            Val::Int(n) => Some(n.clone()),
            _ => None,
        }
    }

    fn exact_rational(&self) -> Option<Rational> {
        self.standard_rational()
    }

    fn is_standard_number(&self) -> bool {
        matches!(self, Val::Int(_) | Val::Rat(_))
    }

    fn text(&self) -> Option<&str> {
        match self {
            Val::Text(t) => Some(t),
            _ => None,
        }
    }

    fn type_name(&self) -> String {
        match self {
            Val::Int(_) => "Int",
            Val::Rat(_) => "Rational",
            Val::Bool(_) => "Bool",
            Val::Text(_) => "Text",
            Val::Nothing => "Nothing",
            Val::Alg(_) => "Algebraic",
            Val::Real(_) => "ExactReal",
            Val::Hyper(_) => "Hyperreal",
            Val::Nat(_) => "Hypernatural",
            Val::Ord(_) => "Ordinal",
            Val::Dep(_) => "Depends",
            Val::Conv(_) => "Converging",
            Val::List(_) => "Seq",
        }
        .to_string()
    }

    fn display_string(&self) -> String {
        match self {
            Val::Int(n) => n.to_string(),
            Val::Rat(r) => r.to_string(),
            Val::Bool(b) => b.to_string(),
            Val::Text(t) => t.clone(),
            Val::Nothing => "nothing".to_string(),
            Val::List(items) => format!(
                "[{}]",
                items.iter().map(Val::display_string).collect::<Vec<_>>().join(", ")
            ),
            other => display(other).expect("an exact value displays"),
        }
    }

    fn values_equal(&self, other: &Self) -> bool {
        if let Some(same) = structurally_equal(self, other) {
            return same;
        }
        match (self, other) {
            (Val::Bool(a), Val::Bool(b)) => a == b,
            (Val::Text(a), Val::Text(b)) => a == b,
            (Val::Nothing, Val::Nothing) => true,
            (Val::List(a), Val::List(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.values_equal(y))
            }
            _ => match (self.standard_rational(), other.standard_rational()) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            },
        }
    }

    fn is_nothing(&self) -> bool {
        matches!(self, Val::Nothing)
    }

    fn from_rational(r: Rational) -> Self {
        match r.to_bigint() {
            Some(n) if r.is_integer() => Val::Int(n),
            _ => Val::Rat(r),
        }
    }

    fn from_bigint(b: BigInt) -> Self {
        Val::Int(b)
    }

    fn boolean(b: bool) -> Self {
        Val::Bool(b)
    }

    fn nothing() -> Self {
        Val::Nothing
    }

    fn algebraic(a: Algebraic) -> Self {
        Val::Alg(a)
    }

    fn exact_real(x: ExactReal) -> Self {
        Val::Real(x)
    }

    fn hyper(h: HyperValue) -> Self {
        Val::Hyper(h)
    }

    fn natural(n: NaturalValue) -> Self {
        Val::Nat(n)
    }

    fn ordinal(o: Ordinal) -> Self {
        Val::Ord(o)
    }

    fn depends(d: DependsValue<Self>) -> Self {
        Val::Dep(Box::new(d))
    }

    fn converging(c: Converging<Self>) -> Self {
        Val::Conv(Box::new(c))
    }
}

impl ValueAlgebra for Val {
    fn add(a: Self, b: Self) -> Result<Self, String> {
        calc(Op::Add, a, b)
    }

    fn subtract(a: Self, b: Self) -> Result<Self, String> {
        calc(Op::Sub, a, b)
    }

    fn multiply(a: Self, b: Self) -> Result<Self, String> {
        calc(Op::Mul, a, b)
    }

    fn exact_divide(a: Self, b: Self) -> Result<Self, String> {
        calc(Op::ExactDiv, a, b)
    }

    fn modulo(a: Self, b: Self) -> Result<Self, String> {
        calc(Op::Mod, a, b)
    }

    fn power(a: Self, b: Self) -> Result<Self, String> {
        calc(Op::Pow, a, b)
    }

    fn holds(rel: Rel, a: &Self, b: &Self) -> Result<bool, String> {
        let answer = compare(rel, a.clone(), b.clone())?;
        if let Some(why) = undecided_branch(&answer) {
            return Err(why);
        }
        match answer {
            Val::Bool(b) => Ok(b),
            Val::Conv(c) => Ok(matches!(c.value(), Val::Bool(true))),
            other => Err(format!("a comparison answered {other:?}")),
        }
    }

    fn floor(v: Self) -> Result<Self, String> {
        if let Some(r) = integer_part(&v, Rounding::Floor, &|x| Self::floor(x)) {
            return r;
        }
        Ok(Val::from_bigint(v.standard_rational().ok_or("floor of a number")?.floor()))
    }

    fn ceil(v: Self) -> Result<Self, String> {
        if let Some(r) = integer_part(&v, Rounding::Ceil, &|x| Self::ceil(x)) {
            return r;
        }
        Ok(Val::from_bigint(v.standard_rational().ok_or("ceiling of a number")?.ceil()))
    }

    fn list(values: Vec<Self>) -> Self {
        Val::List(values)
    }

    fn shape(&self) -> Shape<Self> {
        match self {
            Val::Bool(_) => Shape::Bool,
            Val::Int(n) => match n.to_i64() {
                Some(k) => Shape::Int(k),
                None => Shape::BigInt(n.clone()),
            },
            Val::Rat(r) => Shape::Rational(r.clone()),
            Val::Text(t) => Shape::Text(t.len()),
            Val::List(items) => Shape::List(items.clone()),
            _ => Shape::Other,
        }
    }
}

/// The model's whole arithmetic: the exact core first, then integers and rationals, with `/` of two
/// integers flooring and `ExactDiv` exact.
pub(crate) fn calc(op: Op, a: Val, b: Val) -> Result<Val, String> {
    if let Some(r) = arith(op, &a, &b, &calc) {
        return r;
    }
    let (Some(x), Some(y)) = (a.standard_rational(), b.standard_rational()) else {
        return Err(format!("cannot combine {} and {}", a.type_name(), b.type_name()));
    };
    let both_integers = a.integer().is_some() && b.integer().is_some();
    Ok(match op {
        Op::Add => Val::from_rational(x.add(&y)),
        Op::Sub => Val::from_rational(x.sub(&y)),
        Op::Mul => Val::from_rational(x.mul(&y)),
        Op::Div if both_integers => Val::from_bigint(
            x.div(&y).ok_or_else(|| "Division by zero".to_string())?.floor(),
        ),
        Op::Div | Op::ExactDiv => {
            Val::from_rational(x.div(&y).ok_or_else(|| "Division by zero".to_string())?)
        }
        Op::FloorDiv => Val::from_bigint(x.div(&y).ok_or_else(|| "Division by zero".to_string())?.floor()),
        Op::Mod => {
            let q = x.div(&y).ok_or_else(|| "Division by zero".to_string())?.floor();
            Val::from_rational(x.sub(&y.mul(&Rational::from_bigint(q))))
        }
        Op::Pow => match b.integer().and_then(|k| k.to_i64()).and_then(|k| i32::try_from(k).ok()) {
            Some(k) => Val::from_rational(x.pow(k).ok_or_else(|| "zero cannot be raised to a negative power".to_string())?),
            None => return Err(format!("cannot raise {} to the {} power", a.type_name(), b.type_name())),
        },
    })
}

pub(crate) fn compare(rel: Rel, a: Val, b: Val) -> Result<Val, String> {
    if let Some(r) = relation(rel, &a, &b, &compare) {
        return r;
    }
    let (Some(x), Some(y)) = (a.standard_rational(), b.standard_rational()) else {
        return Ok(Val::Bool(match rel {
            Rel::Eq => a.values_equal(&b),
            Rel::NotEq => !a.values_equal(&b),
            _ => return Err("cannot order these values".to_string()),
        }));
    };
    let o = x.cmp(&y);
    Ok(Val::Bool(match rel {
        Rel::Lt => o.is_lt(),
        Rel::Gt => o.is_gt(),
        Rel::LtEq => o.is_le(),
        Rel::GtEq => o.is_ge(),
        Rel::Eq => o.is_eq(),
        Rel::NotEq => o.is_ne(),
    }))
}

pub(crate) fn int(n: i64) -> Val {
    Val::Int(BigInt::from_i64(n))
}

pub(crate) fn dt() -> Val {
    new_infinitesimal("dt", 1, &[0]).unwrap()
}

pub(crate) fn even_dt() -> Val {
    new_infinitesimal("dt", 2, &[0]).unwrap()
}

pub(crate) fn show(v: &Val) -> String {
    v.display_string()
}

#[test]
fn the_infinitesimal_prints_by_its_name_and_combines_with_numbers() {
    let d = dt();
    assert_eq!(show(&d), "dt");
    assert_eq!(show(&calc(Op::Add, d.clone(), int(3)).unwrap()), "3 + dt");
    assert_eq!(show(&calc(Op::Div, int(1), d.clone()).unwrap()), "1/dt");
    assert_eq!(show(&calc(Op::Mul, d.clone(), d).unwrap()), "dt^2");
}

#[test]
fn a_value_with_no_infinitesimal_part_is_its_standard_form() {
    let d = dt();
    let back = calc(Op::Sub, calc(Op::Add, d.clone(), int(1)).unwrap(), d.clone()).unwrap();
    assert!(matches!(&back, Val::Int(n) if *n == BigInt::from_i64(1)), "{back:?}");
    let half = calc(Op::Div, d.clone(), calc(Op::Mul, d, int(2)).unwrap()).unwrap();
    assert!(matches!(half, Val::Rat(_)));
    assert_eq!(show(&half), "1/2");
}

#[test]
fn the_generator_s_parity_depends_on_its_residue_until_one_is_declared() {
    let h = generator_of(&dt()).unwrap();
    assert!(matches!(h, Val::Nat(_)));
    let parity = calc(Op::Mod, h, int(2)).unwrap();
    assert_eq!(show(&parity), "0 when ⌈1/dt⌉ is even, 1 when it is odd");
    let even = calc(Op::Mod, generator_of(&even_dt()).unwrap(), int(2)).unwrap();
    assert!(matches!(&even, Val::Int(n) if *n == BigInt::from_i64(0)), "{even:?}");
}

#[test]
fn an_operation_lifts_over_residue_branches() {
    let parity = calc(Op::Mod, generator_of(&dt()).unwrap(), int(2)).unwrap();
    let flipped = calc(Op::Sub, int(1), parity.clone()).unwrap();
    assert_eq!(show(&flipped), "1 when ⌈1/dt⌉ is even, 0 when it is odd");
    let sum = calc(Op::Add, parity, flipped).unwrap();
    assert!(matches!(&sum, Val::Int(n) if *n == BigInt::from_i64(1)), "{sum:?}");
}

#[test]
fn a_comparison_of_the_infinitesimal_is_decided_by_transfer() {
    let d = dt();
    assert!(matches!(compare(Rel::Lt, d.clone(), int(1)).unwrap(), Val::Bool(true)));
    assert!(matches!(compare(Rel::Gt, d.clone(), int(0)).unwrap(), Val::Bool(true)));
    let big = calc(Op::Div, int(1), d).unwrap();
    assert!(matches!(compare(Rel::Gt, big, int(1_000_000)).unwrap(), Val::Bool(true)));
}

#[test]
fn a_parity_comparison_depends_on_the_residue() {
    let parity = calc(Op::Mod, generator_of(&dt()).unwrap(), int(2)).unwrap();
    let is_zero = compare(Rel::Eq, parity, int(0)).unwrap();
    assert_eq!(show(&is_zero), "true when ⌈1/dt⌉ is even, false when it is odd");
    assert_eq!(
        undecided_branch(&is_zero).as_deref(),
        Some("this condition depends on the residue of ⌈1/dt⌉: it is true when ⌈1/dt⌉ is even, false when it is odd; declare what is known about 1/dt or take a limit")
    );
}

#[test]
fn the_standard_part_of_a_limited_value_and_the_refusal_of_an_infinite_one() {
    let d = dt();
    let x = calc(Op::Add, int(3), calc(Op::Mul, int(2), d.clone()).unwrap()).unwrap();
    assert_eq!(show(&standard_part(&x).unwrap()), "3");
    let infinite = calc(Op::Div, int(1), d).unwrap();
    assert_eq!(
        standard_part(&infinite).unwrap_err(),
        "1/dt is infinite and has no standard part"
    );
    assert_eq!(show(&standard_part(&int(7)).unwrap()), "7");
    assert!(matches!(is_infinitesimal(&dt()).unwrap(), Val::Bool(true)));
    assert!(matches!(is_limited(&calc(Op::Div, int(1), dt()).unwrap()).unwrap(), Val::Bool(false)));
}

#[test]
fn a_limit_rule_resolves_a_dependence() {
    let parity = calc(Op::Mod, generator_of(&dt()).unwrap(), int(2)).unwrap();
    assert_eq!(show(&apply_limit(&parity, LimitRule::LimitSuperior).unwrap()), "1");
    assert_eq!(show(&apply_limit(&parity, LimitRule::LimitInferior).unwrap()), "0");
    assert_eq!(show(&apply_limit(&parity, LimitRule::CesaroMean).unwrap()), "1/2");
    assert_eq!(
        apply_limit(&parity, LimitRule::Limit).unwrap_err(),
        "no limit: the value is 0 when ⌈1/dt⌉ is even, 1 when it is odd"
    );
}

#[test]
fn a_provisional_answer_carries_its_evidence_through_arithmetic() {
    let guess = provisional(int(4), 100, 2);
    let more = calc(Op::Add, guess, int(1)).unwrap();
    let Val::Conv(c) = &more else {
        panic!("{more:?}");
    };
    assert!(matches!(c.value(), Val::Int(n) if *n == BigInt::from_i64(5)));
    assert_eq!((c.steps(), c.revisions(), c.is_settled()), (100, 2, false));
}

/// The evidence an answer carries, with its value; `None` for a value that is not an answer.
fn evidence(v: &Val) -> Option<(&Val, u64, u64, Option<&str>)> {
    match v {
        Val::Conv(c) => Some((c.value(), c.steps(), c.revisions(), c.certificate())),
        _ => None,
    }
}

#[test]
fn two_provisional_answers_make_one_answer_on_the_fewest_cases_and_every_revision() {
    let sum = calc(Op::Add, provisional(int(4), 100, 2), provisional(int(5), 50, 3)).unwrap();
    let (value, steps, revisions, certificate) = evidence(&sum).expect("an answer");
    assert!(evidence(value).is_none(), "an answer never holds another: {sum:?}");
    assert_eq!(show(value), "9");
    assert_eq!((steps, revisions, certificate), (50, 5, None));
    assert_eq!(show(&sum), "9 (provisional: checked 50 cases, 5 revisions; not proved)");
}

#[test]
fn a_provisional_guess_marked_provisional_again_keeps_one_evidence() {
    let again = provisional(provisional(int(1), 200, 1), 200, 0);
    let (value, steps, revisions, _) = evidence(&again).expect("an answer");
    assert!(evidence(value).is_none(), "{again:?}");
    assert_eq!((steps, revisions), (200, 1));
    let fewer = provisional(provisional(int(1), 200, 1), 30, 4);
    assert_eq!(show(&fewer), "1 (provisional: checked 30 cases, 5 revisions; not proved)");
}

#[test]
fn a_settled_answer_joins_a_provisional_one_on_the_provisional_evidence() {
    let settled = Val::Conv(Box::new(Converging::settled(int(2), "a kernel proof")));
    let guess = provisional(int(3), 40, 6);
    for joined in [
        calc(Op::Mul, settled.clone(), guess.clone()).unwrap(),
        calc(Op::Mul, guess.clone(), settled.clone()).unwrap(),
    ] {
        let (value, steps, revisions, certificate) = evidence(&joined).expect("an answer");
        assert_eq!(show(value), "6");
        assert_eq!((steps, revisions, certificate), (40, 6, None));
    }
    let both = calc(Op::Add, settled.clone(), settled.clone()).unwrap();
    let (value, _, _, certificate) = evidence(&both).expect("an answer");
    assert_eq!((show(value).as_str(), certificate), ("4", Some("a kernel proof")));
    let other = Val::Conv(Box::new(Converging::settled(int(1), "a witness")));
    let two = calc(Op::Add, settled, other).unwrap();
    assert_eq!(evidence(&two).unwrap().3, Some("a kernel proof; a witness"));
}

#[test]
fn ordinals_add_on_the_left_absorbingly() {
    let omega: Val = from_ordinal(Ordinal::omega());
    let one_plus = calc(Op::Add, int(1), omega.clone()).unwrap();
    assert_eq!(show(&one_plus), "ω");
    let plus_one = calc(Op::Add, omega, int(1)).unwrap();
    assert_eq!(show(&plus_one), "ω + 1");
    assert_eq!(
        calc(Op::Sub, plus_one, int(1)).unwrap_err(),
        "ordinals add, multiply and exponentiate; they do not subtract"
    );
}

#[test]
fn two_infinitesimals_of_no_stated_order_do_not_mix() {
    let h = new_infinitesimal::<Val>("h", 1, &[0]).unwrap();
    assert_eq!(
        calc(Op::Add, dt(), h).unwrap_err(),
        "cannot combine values of two infinitesimals, dt and h, whose order the program does not state; declare one much smaller than the other"
    );
}

fn scale(name: &str) -> Arc<Scale> {
    Arc::new(Scale::new(name, ResidueType::any()))
}

fn ordered(name: &str, larger: Option<&Arc<Scale>>, smaller: Option<&Arc<Scale>>) -> Result<Arc<Scale>, String> {
    Scale::ordered(name, ResidueType::any(), larger.cloned(), smaller.cloned()).map(Arc::new)
}

/// The order of two infinitesimals is what the declarations give by transitivity, through
/// infinitesimals declared above and below alike.
#[test]
fn the_order_of_infinitesimals_runs_through_their_declarations() {
    let dt = scale("dt");
    let dx = ordered("dx", Some(&dt), None).unwrap();
    let dy = ordered("dy", Some(&dx), None).unwrap();
    let h = ordered("h", None, Some(&dt)).unwrap();
    let between = ordered("dz", Some(&dt), Some(&dx)).unwrap();
    assert_eq!(dx.order(&dt), Some(Ordering::Less));
    assert_eq!(dt.order(&dx), Some(Ordering::Greater));
    assert_eq!(dy.order(&dt), Some(Ordering::Less), "dy below dx below dt");
    assert_eq!(dy.order(&h), Some(Ordering::Less), "dy below dx below dt below h");
    assert_eq!(h.order(&dy), Some(Ordering::Greater));
    assert_eq!(between.order(&dt), Some(Ordering::Less));
    assert_eq!(between.order(&dx), Some(Ordering::Greater));
    assert_eq!(between.order(&dy), Some(Ordering::Greater));
    assert_eq!(between.order(&h), Some(Ordering::Less));
    assert_eq!(dt.order(&dt), Some(Ordering::Equal));
}

/// Two infinitesimals the declarations leave unordered stay so: both below a third, both above one,
/// or declared on their own.
#[test]
fn infinitesimals_the_declarations_do_not_order_stay_unordered() {
    let dt = scale("dt");
    let dx = ordered("dx", Some(&dt), None).unwrap();
    let dy = ordered("dy", Some(&dt), None).unwrap();
    let h = ordered("h", None, Some(&dt)).unwrap();
    let k = ordered("k", None, Some(&dt)).unwrap();
    let between = ordered("dz", Some(&dt), Some(&dx)).unwrap();
    assert_eq!(dx.order(&dy), None);
    assert_eq!(h.order(&k), None);
    assert_eq!(between.order(&dy), None, "dz and dy both lie below dt, dz above dx only");
    assert_eq!(scale("u").order(&dt), None);
}

#[test]
fn a_declaration_between_two_needs_them_ordered() {
    let dt = scale("dt");
    let h = scale("h");
    let dx = ordered("dx", Some(&dt), None).unwrap();
    assert_eq!(
        ordered("dy", Some(&dt), Some(&h)).unwrap_err(),
        "dy is declared much smaller than dt and much larger than h, but the program does not state that h is much smaller than dt"
    );
    assert_eq!(
        ordered("dy", Some(&dx), Some(&dt)).unwrap_err(),
        "dy is declared much smaller than dx and much larger than dt, but dt is much larger than dx"
    );
    assert_eq!(
        ordered("dy", Some(&dt), Some(&dt)).unwrap_err(),
        "dy cannot be both much smaller and much larger than dt"
    );
    assert_eq!(
        ordered("dt", Some(&dx), None).unwrap_err(),
        "dt is ordered with the infinitesimal dt, so it needs a name of its own"
    );
}

/// An infinitesimal declared on its own writes its residue type alone, as it always has; one declared
/// against others writes them too, and reads back as itself.
#[test]
fn an_infinitesimal_travels_with_where_it_stands() {
    let dt = Arc::new(Scale::new("dt", ResidueType::congruent(0, 2)));
    assert_eq!(dt.to_bytes(), ResidueType::congruent(0, 2).to_bytes());
    let dx = ordered("dx", Some(&dt), None).unwrap();
    let h = ordered("h", None, Some(&dt)).unwrap();
    for s in [&dt, &dx, &h] {
        let back = Scale::from_bytes(&s.name, &s.to_bytes()).unwrap();
        assert_eq!(*back, **s);
        assert!(back.same_infinitesimal(s));
    }
    let mut longer = dx.to_bytes();
    longer.push(0);
    assert!(Scale::from_bytes("dx", &longer).is_none());
    assert!(Scale::from_bytes("dt", &dx.to_bytes()).is_none(), "a name its own bound has");
}

#[test]
fn values_of_ordered_infinitesimals_combine() {
    let dt = dt();
    let dx = new_ordered_infinitesimal::<Val>("dx", &dt, &Val::Nothing, 1, &[0]).unwrap();
    let sum = calc(Op::Add, dt.clone(), dx.clone()).unwrap();
    assert_eq!(show(&sum), "dt + dx");
    let back = calc(Op::Sub, sum, dx.clone()).unwrap();
    assert_eq!(show(&back), "dt");
    assert!(matches!(back, Val::Hyper(_)));
    assert_eq!(
        new_ordered_infinitesimal::<Val>("dy", &int(2), &Val::Nothing, 1, &[0]).unwrap_err(),
        "an infinitesimal is declared much smaller than another infinitesimal itself, not than 2"
    );
}

#[test]
fn every_exact_value_travels_by_its_canonical_bytes() {
    let x = calc(Op::Add, int(3), dt()).unwrap();
    let (kind, name, residues, bytes) = scaled_bytes(&x).unwrap();
    let back: Val = from_scaled_bytes(kind, &name, &residues, &bytes).unwrap();
    assert_eq!(show(&back), "3 + dt");
    let root2: Val = from_algebraic(Algebraic::from_i64(2).sqrt().unwrap());
    let (kind, bytes) = exact_bytes(&root2).unwrap();
    assert_eq!(show(&from_exact_bytes::<Val>(kind, &bytes).unwrap()), "√2");
    let parity = calc(Op::Mod, generator_of(&dt()).unwrap(), int(2)).unwrap();
    let Val::Dep(d) = &parity else {
        panic!("{parity:?}");
    };
    let (name, residues) = scale_parts(&d.scale);
    let branches = d.value.raw_branches().into_iter().map(|b| b.map(|b| b.0)).collect();
    let rebuilt: Val = depends_from_parts(&name, &residues, d.value.modulus(), branches).unwrap();
    assert_eq!(show(&rebuilt), show(&parity));
}

#[test]
fn a_dependence_of_text_prints_its_branches_and_compares_branch_by_branch() {
    let Val::Hyper(h) = dt() else {
        panic!("the infinitesimal is a hyperreal");
    };
    let scale = h.scale.clone();
    let lamp: Val = from_depends(
        Depends::from_fn(2, &scale.residues, |r| {
            Branch(Val::Text(if r == 0 { "off" } else { "on" }.to_string()))
        })
        .unwrap(),
        &scale,
    );
    assert_eq!(show(&lamp), "off when ⌈1/dt⌉ is even, on when it is odd");
    let off = compare(Rel::Eq, lamp.clone(), Val::Text("off".to_string())).unwrap();
    assert_eq!(show(&off), "true when ⌈1/dt⌉ is even, false when it is odd");
    let differs = compare(Rel::NotEq, lamp.clone(), lamp).unwrap();
    assert!(matches!(differs, Val::Bool(false)), "{differs:?}");
}

#[test]
fn a_function_of_the_reals_answers_in_its_smallest_exact_form() {
    let sqrt = |v: Val| real_fn(RealFn::Sqrt, &v).unwrap();
    assert!(matches!(sqrt(int(4)), Val::Int(_)), "a rational answer is a standard number");
    assert!(matches!(real_fn(RealFn::Exp, &int(0)).unwrap(), Val::Int(_)));
    assert!(matches!(real_fn(RealFn::Ln, &euler::<Val>()).unwrap(), Val::Int(_)));
    assert!(matches!(sqrt(int(2)), Val::Alg(_)), "an irrational root is an algebraic number");
    assert!(matches!(real_fn(RealFn::Exp, &int(1)).unwrap(), Val::Real(_)));
    assert_eq!(show(&sqrt(int(4))), "2");
    assert_eq!(show(&sqrt(Val::Rat(Rational::from_ratio_i64(9, 4).unwrap()))), "3/2");
    assert_eq!(show(&sqrt(int(2))), "√2");
    assert_eq!(show(&sqrt(int(0))), "0");
    let root2 = sqrt(int(2));
    assert_eq!(show(&calc(Op::Mul, root2.clone(), root2).unwrap()), "2", "√2·√2 is the integer 2");
    assert_eq!(show(&real_fn(RealFn::Exp, &int(0)).unwrap()), "1");
    assert_eq!(show(&real_fn(RealFn::Ln, &int(1)).unwrap()), "0");
    assert_eq!(show(&real_fn(RealFn::Ln, &euler::<Val>()).unwrap()), "1");
    let e2 = real_fn(RealFn::Exp, &int(2)).unwrap();
    assert_eq!(show(&real_fn(RealFn::Ln, &e2).unwrap()), "2");
    assert_eq!(show(&pi::<Val>()), "π");
    assert_eq!(show(&euler::<Val>()), "e");
    assert_eq!(show(&omega::<Val>()), "ω");
    let w = omega::<Val>();
    assert_eq!(show(&calc(Op::Add, int(1), w.clone()).unwrap()), "ω", "1 + ω = ω");
    assert_eq!(show(&calc(Op::Add, w, int(1)).unwrap()), "ω + 1");
}

#[test]
fn a_function_of_the_reals_refuses_what_has_no_exact_answer() {
    let refused = |f: RealFn, v: Val| match real_fn(f, &v) {
        Err(e) => e,
        Ok(x) => panic!("{f:?} answered {}", show(&x)),
    };
    assert_eq!(refused(RealFn::Sqrt, int(-4)), "square root of a negative number");
    assert_eq!(refused(RealFn::Ln, int(0)), "logarithm of a nonpositive number");
    assert_eq!(refused(RealFn::Ln, int(-1)), "logarithm of a nonpositive number");
    assert_eq!(refused(RealFn::Sqrt, omega::<Val>()), "the square root of ω has no exact form");
    assert_eq!(refused(RealFn::Exp, omega::<Val>()), "the exponential of ω has no exact form");
    assert_eq!(refused(RealFn::Ln, Val::Text("x".to_string())), "a Text is not a number");
}

fn rat(n: i64, d: i64) -> Val {
    Val::from_rational(Rational::from_ratio_i64(n, d).unwrap())
}

/// A root of a hyperreal travels by its scale and bytes, as every value of the infinitesimal does, and
/// is read back the value it was; its predicates and integer part read its expansion.
#[test]
fn a_root_of_a_hyperreal_travels_and_answers_like_any_hyperreal() {
    let root = real_fn(RealFn::Sqrt, &calc(Op::Add, int(2), dt()).unwrap()).unwrap();
    let (kind, name, residues, bytes) = scaled_bytes(&root).expect("a root travels");
    assert_eq!(kind, 3);
    let back: Val = from_scaled_bytes(kind, &name, &residues, &bytes).expect("reads back");
    assert!(matches!(compare(Rel::Eq, back.clone(), root.clone()).unwrap(), Val::Bool(true)));
    assert_eq!(show(&back), show(&root));
    assert!(transmissible(&root));
    assert!(matches!(is_limited(&root).unwrap(), Val::Bool(true)));
    assert!(matches!(is_infinitesimal(&root).unwrap(), Val::Bool(false)));
    assert_eq!(show(&standard_part(&root).unwrap()), "√2");
    assert_eq!(show(&<Val as ValueAlgebra>::floor(root.clone()).unwrap()), "1");
    assert_eq!(show(&<Val as ValueAlgebra>::ceil(root).unwrap()), "2");
    let small = real_fn(RealFn::Sqrt, &dt()).unwrap();
    assert!(matches!(is_infinitesimal(&small).unwrap(), Val::Bool(true)));
    assert!(matches!(compare(Rel::Gt, small, dt()).unwrap(), Val::Bool(true)));
}

/// A root of a hyperreal beside a computable transcendental is a germ, read on the root's own grid.
#[test]
fn a_root_of_a_hyperreal_beside_a_computable_real_is_a_germ() {
    let root = real_fn(RealFn::Sqrt, &dt()).unwrap();
    let sum = calc(Op::Add, root, pi::<Val>()).unwrap();
    assert_eq!(show(&sum), "π + √dt + O(dt)");
    assert!(matches!(compare(Rel::Gt, sum.clone(), pi::<Val>()).unwrap(), Val::Bool(true)));
    assert_eq!(show(&standard_part(&sum).unwrap()), "π");
    assert_eq!(show(&calc(Op::Sub, sum, pi::<Val>()).unwrap()), "√dt + O(dt)");
}

/// The exponential and the logarithm of a hyperreal are its germs; what is no germ is refused in the
/// value's own words and the infinitesimal's name.
#[test]
fn the_exponential_and_logarithm_of_a_hyperreal_are_germs() {
    assert_eq!(show(&real_fn(RealFn::Exp, &dt()).unwrap()), "1 + dt + O(dt^2)");
    let one_plus = calc(Op::Add, int(1), dt()).unwrap();
    assert_eq!(show(&real_fn(RealFn::Ln, &one_plus).unwrap()), "dt - (1/2)·dt^2 + O(dt^3)");
    let root = real_fn(RealFn::Sqrt, &dt()).unwrap();
    assert_eq!(show(&real_fn(RealFn::Exp, &root).unwrap()), "1 + √dt + O(dt)");
    let germ = real_fn(RealFn::Exp, &dt()).unwrap();
    assert_eq!(show(&real_fn(RealFn::Ln, &germ).unwrap()), "dt + O(dt^2)");
    let h = generator_of(&dt()).unwrap();
    let refused = |f: RealFn, v: &Val| real_fn(f, v).unwrap_err();
    assert_eq!(refused(RealFn::Exp, &h), "the exponential of 1/dt is beyond every power of dt");
    assert_eq!(refused(RealFn::Ln, &dt()), "the logarithm of dt carries ln(dt)");
    assert_eq!(refused(RealFn::Ln, &h), "the logarithm of 1/dt carries ln(1/dt)");
    assert_eq!(refused(RealFn::Ln, &root), "the logarithm of √dt carries ln(dt)");
    assert_eq!(refused(RealFn::Ln, &calc(Op::Sub, int(0), dt()).unwrap()), "logarithm of a nonpositive number");
    assert_eq!(refused(RealFn::Exp, &calc(Op::Div, germ, dt()).unwrap()), "the exponential of 1/dt + 1 + O(dt) is beyond every power of dt");
}

/// A germ takes a rational power as the power of its root, and its square root is its germ root.
#[test]
fn a_germ_takes_roots_and_rational_powers() {
    let e_germ = real_fn(RealFn::Exp, &dt()).unwrap();
    assert_eq!(show(&exact_power(&e_germ, &rat(1, 2)).unwrap()), "1 + (1/2)·dt + O(dt^2)", "exp(dt)^(1/2) = exp(dt/2)");
    assert_eq!(show(&exact_power(&e_germ, &rat(-1, 3)).unwrap()), "1 - (1/3)·dt + O(dt^2)");
    assert_eq!(show(&real_fn(RealFn::Sqrt, &e_germ).unwrap()), "1 + (1/2)·dt + O(dt^2)");
    let near_pi = calc(Op::Add, pi::<Val>(), dt()).unwrap();
    let power = germ_of(&exact_power(&near_pi, &rat(3, 2)).unwrap()).expect("a germ");
    let dec = |k: i64| -> f64 { power.coefficient(k).unwrap().to_decimal(25).parse().unwrap() };
    let p = std::f64::consts::PI;
    assert!((dec(0) - p.powf(1.5)).abs() < 1e-12, "(π + dt)^(3/2) starts at π^(3/2)");
    assert!((dec(1) - 1.5 * p.sqrt()).abs() < 1e-12, "and grows at (3/2)·√π");
    let negative = calc(Op::Sub, calc(Op::Sub, int(0), pi::<Val>()).unwrap(), dt()).unwrap();
    assert_eq!(exact_power(&negative, &rat(1, 2)).unwrap_err(), "square root of a negative number");
    assert_eq!(real_fn(RealFn::Sqrt, &negative).unwrap_err(), "square root of a negative number");
    let cube = germ_of(&exact_power(&negative, &rat(1, 3)).unwrap()).expect("a germ");
    assert!((cube.coefficient(0).unwrap().to_decimal(25).parse::<f64>().unwrap() + p.cbrt()).abs() < 1e-12, "∛(−π − dt) starts at −∛π");
}

/// The square root of a hyperreal is its root in the real closure: `√dt`, of valuation one half, whose
/// square is `dt` again; an infinite integer's is the reciprocal root of the infinitesimal.
#[test]
fn the_square_root_of_a_hyperreal_is_its_root_in_the_real_closure() {
    let root = real_fn(RealFn::Sqrt, &dt()).unwrap();
    assert_eq!(show(&root), "√dt");
    assert_eq!(show(&calc(Op::Mul, root.clone(), root).unwrap()), "dt");
    let h = generator_of(&dt()).unwrap();
    assert_eq!(show(&real_fn(RealFn::Sqrt, &h).unwrap()), "1/√dt");
    let one_plus = calc(Op::Add, int(1), dt()).unwrap();
    assert_eq!(show(&real_fn(RealFn::Sqrt, &one_plus).unwrap()), "√(1 + dt)");
    let negative = calc(Op::Sub, int(0), dt()).unwrap();
    assert_eq!(real_fn(RealFn::Sqrt, &negative).unwrap_err(), "square root of a negative number");
}

/// `exact_power`: a fraction takes a root on an exact base or a standard one, and any other exponent is the
/// value model's own `**`.
#[test]
fn an_exact_power_takes_a_root_for_a_fractional_exponent() {
    let half = rat(1, 2);
    assert_eq!(show(&exact_power(&int(2), &half).unwrap()), "√2");
    assert_eq!(show(&exact_power(&int(8), &rat(2, 3)).unwrap()), "4");
    assert_eq!(show(&exact_power(&dt(), &half).unwrap()), "√dt");
    assert_eq!(show(&exact_power(&dt(), &rat(3, 2)).unwrap()), "dt^(3/2)");
    assert_eq!(show(&exact_power(&int(3), &int(2)).unwrap()), "9", "an integer exponent is the model's own power");
    assert_eq!(exact_power(&int(-4), &half).unwrap_err(), "square root of a negative number");
    assert_eq!(show(&exact_power(&int(-8), &rat(1, 3)).unwrap()), "-2");
    assert_eq!(exact_power(&int(0), &rat(-1, 2)).unwrap_err(), "zero cannot be raised to a negative power");
}

#[test]
fn a_function_of_the_reals_maps_a_dependence_and_keeps_evidence() {
    let parity = calc(Op::Mod, calc(Op::Div, int(1), dt()).unwrap(), int(2)).unwrap();
    assert_eq!(show(&parity), "0 when ⌈1/dt⌉ is even, 1 when it is odd");
    assert_eq!(
        show(&real_fn(RealFn::Exp, &parity).unwrap()),
        "1 when ⌈1/dt⌉ is even, e when it is odd"
    );
    let guess = provisional(int(2), 5, 0);
    let root = real_fn(RealFn::Sqrt, &guess).unwrap();
    assert_eq!(show(&root), show(&provisional(real_fn(RealFn::Sqrt, &int(2)).unwrap(), 5, 0)));
    assert!(show(&root).starts_with("√2 (provisional"), "{}", show(&root));
}

/// A computable real takes a fractional power as the power of its root: exactly on a single
/// exponential term, to any precision on any other value.
#[test]
fn a_computable_real_takes_a_fractional_power_as_the_power_of_its_root() {
    let e = euler::<Val>();
    assert_eq!(show(&exact_power(&e, &rat(1, 2)).unwrap()), "e^(1/2)");
    assert_eq!(show(&exact_power(&e, &rat(-3, 2)).unwrap()), "e^(-3/2)");
    let root = exact_power(&pi::<Val>(), &rat(2, 3)).unwrap();
    let cubed = calc(Op::Mul, calc(Op::Mul, root.clone(), root.clone()).unwrap(), root).unwrap();
    let squared_pi = calc(Op::Mul, pi::<Val>(), pi::<Val>()).unwrap();
    let gap = calc(Op::Sub, cubed, squared_pi).unwrap();
    let Some(ExactRef::Real(x)) = gap.exact() else { panic!("a computable real") };
    let (lo, hi) = x.enclosure(80).unwrap();
    assert!(!lo.is_positive() && !hi.is_negative(), "(π^(2/3))³ = π²: [{lo}, {hi}]");
}

/// `dt` and `dx` much smaller than it, in the test value model.
fn both() -> (Val, Val) {
    let dt = dt();
    let dx = new_ordered_infinitesimal::<Val>("dx", &dt, &Val::Nothing, 1, &[0]).unwrap();
    (dt, dx)
}

fn parity(of: &Val) -> Val {
    calc(Op::Mod, generator_of(of).unwrap(), int(2)).unwrap()
}

/// Two parities of two generators combine as a dependence on the coarser, `⌈1/dt⌉`, whose branches
/// depend on the finer, whichever operand comes first; a dependence on one that cancels leaves the
/// other.
#[test]
fn a_value_of_two_residues_depends_on_the_coarser_first() {
    let (dt, dx) = both();
    let (a, b) = (parity(&dt), parity(&dx));
    let nested = "(0 when ⌈1/dx⌉ is even, 1 when it is odd) when ⌈1/dt⌉ is even, (1 when ⌈1/dx⌉ is even, 2 when it is odd) when it is odd";
    assert_eq!(show(&calc(Op::Add, a.clone(), b.clone()).unwrap()), nested);
    assert_eq!(show(&calc(Op::Add, b.clone(), a.clone()).unwrap()), nested);
    let Val::Dep(d) = calc(Op::Add, b.clone(), a.clone()).unwrap() else {
        panic!("a dependence");
    };
    assert!(d.scale.same_infinitesimal(&scale_of_val(&dt)), "the coarser generator is outside");
    let back = calc(Op::Sub, calc(Op::Add, a.clone(), b.clone()).unwrap(), b).unwrap();
    assert_eq!(show(&back), show(&a));
}

fn scale_of_val(v: &Val) -> Arc<Scale> {
    match v {
        Val::Hyper(h) => h.scale.clone(),
        other => panic!("{other:?}"),
    }
}

/// A limit rule reads the finer residue within each class of the coarser.
#[test]
fn a_limit_rule_resolves_a_dependence_on_two_residues() {
    let (dt, dx) = both();
    let sum = calc(Op::Add, parity(&dt), parity(&dx)).unwrap();
    let mean = apply_limit(&sum, LimitRule::CesaroMean).unwrap();
    assert!(matches!(&mean, Val::Int(n) if *n == BigInt::from_i64(1)), "{mean:?}");
    let sup = apply_limit(&sum, LimitRule::LimitSuperior).unwrap();
    assert!(matches!(&sup, Val::Int(n) if *n == BigInt::from_i64(2)), "{sup:?}");
    let inf = apply_limit(&sum, LimitRule::LimitInferior).unwrap();
    assert!(matches!(&inf, Val::Int(n) if *n == BigInt::from_i64(0)), "{inf:?}");
    assert_eq!(
        apply_limit(&sum, LimitRule::Limit).unwrap_err(),
        format!("no limit: the value is {}", show(&sum))
    );
}

/// A dependence on the finer generator whose branches read the coarser is turned inside out where it
/// is built: the coarser outside.
#[test]
fn a_dependence_whose_branches_read_a_coarser_residue_is_turned_inside_out() {
    let (dt, dx) = both();
    let Val::Dep(inner) = parity(&dx) else {
        panic!("a dependence");
    };
    let coarser = parity(&dt);
    let built = from_depends::<Val>(inner.value.map(|b| Branch(calc(Op::Add, b.0.clone(), coarser.clone()).unwrap())), &inner.scale);
    let Val::Dep(d) = &built else {
        panic!("{built:?}");
    };
    assert!(d.scale.same_infinitesimal(&scale_of_val(&dt)), "{}", show(&built));
    assert_eq!(show(&built), show(&calc(Op::Add, parity(&dt), parity(&dx)).unwrap()));
}

/// The integer part of a value infinite in both is an infinite integer of both; its remainder and its
/// half's integer part read the residues they need; one no residue decides is refused by name.
#[test]
fn the_integer_part_of_a_value_infinite_in_both() {
    let (dt, dx) = both();
    let reciprocal = |v: &Val| calc(Op::Div, int(1), v.clone()).unwrap();
    let sum = calc(Op::Add, reciprocal(&dt), reciprocal(&dx)).unwrap();
    let floor = integer_part(&sum, Rounding::Floor, &|v| integer_part(&v, Rounding::Floor, &|_| unreachable!()).unwrap()).unwrap().unwrap();
    assert_eq!(show(&floor), "1/dx + 1/dt");
    assert!(matches!(&floor, Val::Hyper(h) if matches!(h.value, HyperForm::Integer(_))), "{floor:?}");
    assert_eq!(
        show(&calc(Op::Mod, floor.clone(), int(2)).unwrap()),
        "(0 when ⌈1/dx⌉ is even, 1 when it is odd) when ⌈1/dt⌉ is even, (1 when ⌈1/dx⌉ is even, 0 when it is odd) when it is odd"
    );
    let product = reciprocal(&calc(Op::Mul, dt.clone(), dx.clone()).unwrap());
    let floor = integer_part(&product, Rounding::Floor, &|_| unreachable!()).unwrap().unwrap();
    assert_eq!(show(&floor), "1/(dt·dx)");
    let ratio = calc(Op::Div, dt, dx).unwrap();
    assert_eq!(
        integer_part(&ratio, Rounding::Floor, &|_| unreachable!()).unwrap().unwrap_err(),
        "the integer part of dt/dx is no polynomial in ⌈1/dt⌉ and ⌈1/dx⌉, the infinite integers a program holds: it multiplies a power of 1/dx by dt, which is no polynomial in the reciprocals of the infinitesimals larger than dx with rational coefficients"
    );
}

/// An infinite integer of both travels by its canonical bytes as an infinite integer of both.
#[test]
fn an_infinite_integer_of_both_travels_by_its_canonical_bytes() {
    let (dt, dx) = both();
    let product = calc(Op::Div, int(1), calc(Op::Mul, dt, dx).unwrap()).unwrap();
    let (kind, name, residues, bytes) = scaled_bytes(&product).unwrap();
    assert_eq!(kind, 6);
    let back: Val = from_scaled_bytes(kind, &name, &residues, &bytes).unwrap();
    assert_eq!(show(&back), "1/(dt·dx)");
    assert!(back.values_equal(&product));
}
