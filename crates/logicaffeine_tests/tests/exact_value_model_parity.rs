//! The interpreter's runtime value and a compiled program's `LogosValue` are two models of one
//! language. On every value the exact extension can meet — integers at their edges, big integers,
//! rationals, floats with their specials, decimals, booleans, text, lists, nothing, the
//! infinitesimal and what it generates, residue-dependent and provisional values, algebraic numbers,
//! computable reals, ordinals and a value of a kind the tower does not mix with — every operator
//! answers the same in both: the same value printed the same way and of the same type, or the same
//! refusal word for word.

use logicaffeine_base::algebraic::Algebraic;
use logicaffeine_base::exact_real::ExactReal;
use logicaffeine_base::numeric::{BigInt, Decimal, Rational};
use logicaffeine_base::ordinal::Ordinal;
use logicaffeine_base::tower::{self, Rel};
use logicaffeine_compile::ast::stmt::BinaryOpKind;
use logicaffeine_compile::interpreter::RuntimeValue;
use logicaffeine_compile::semantics::{arith, builtins, compare, exact};
use logicaffeine_data::LogosValue;
use logicaffeine_system::temporal::{LogosDate, LogosMoment, LogosSpan, LogosTime};

/// One value in both models.
struct Pair {
    name: String,
    rt: RuntimeValue,
    lv: LogosValue,
}

fn pair(name: &str, rt: RuntimeValue, lv: LogosValue) -> Pair {
    Pair {
        name: name.to_string(),
        rt,
        lv,
    }
}

fn int(n: i64) -> Pair {
    pair(&n.to_string(), RuntimeValue::Int(n), LogosValue::Int(n))
}

fn big(n: BigInt) -> Pair {
    pair(
        &n.to_string(),
        RuntimeValue::from_bigint(n.clone()),
        LogosValue::from_bigint(n),
    )
}

fn rat(n: i64, d: i64) -> Pair {
    let r = Rational::from_ratio_i64(n, d).unwrap();
    pair(
        &format!("{n}/{d}"),
        RuntimeValue::from_rational(r.clone()),
        LogosValue::from_rational(r),
    )
}

fn float(f: f64) -> Pair {
    pair(&format!("{f:?}"), RuntimeValue::Float(f), LogosValue::Float(f))
}

fn decimal(s: &str) -> Pair {
    let d = Decimal::parse(s).unwrap();
    pair(
        &format!("decimal {s}"),
        RuntimeValue::Decimal(std::rc::Rc::new(d.clone())),
        LogosValue::Decimal(std::sync::Arc::new(d)),
    )
}

fn text(s: &str) -> Pair {
    pair(
        &format!("{s:?}"),
        RuntimeValue::Text(std::rc::Rc::new(s.to_string())),
        LogosValue::text(s),
    )
}

fn list(items: &[i64]) -> Pair {
    let rt = RuntimeValue::List(std::rc::Rc::new(std::cell::RefCell::new(
        logicaffeine_compile::interpreter::ListRepr::from_values(
            items.iter().map(|n| RuntimeValue::Int(*n)).collect(),
        ),
    )));
    pair(
        &format!("{items:?}"),
        rt,
        LogosValue::list(items.iter().map(|n| LogosValue::Int(*n)).collect()),
    )
}

/// `op` applied in both models to two pairs, under a name.
fn derived(name: &str, a: &Pair, b: &Pair, op: BinaryOpKind) -> Pair {
    let rt = arith::binary_op(op, a.rt.clone(), b.rt.clone())
        .unwrap_or_else(|e| panic!("{name}: the interpreter refused: {e}"));
    let lv = lv_op(op, &a.lv, &b.lv).unwrap_or_else(|e| panic!("{name}: the compiled model refused: {e}"));
    pair(name, rt, lv)
}

fn lv_op(op: BinaryOpKind, a: &LogosValue, b: &LogosValue) -> Result<LogosValue, String> {
    match op {
        BinaryOpKind::Add => a.add(b),
        BinaryOpKind::Subtract => a.sub(b),
        BinaryOpKind::Multiply => a.mul(b),
        BinaryOpKind::Divide => a.div(b),
        BinaryOpKind::ExactDivide => a.exact_div(b),
        BinaryOpKind::FloorDivide => a.floor_div(b),
        BinaryOpKind::Modulo => a.rem(b),
        BinaryOpKind::Pow => a.pow(b),
        BinaryOpKind::Eq => a.relation(Rel::Eq, b),
        BinaryOpKind::NotEq => a.relation(Rel::NotEq, b),
        BinaryOpKind::Lt => a.relation(Rel::Lt, b),
        BinaryOpKind::Gt => a.relation(Rel::Gt, b),
        BinaryOpKind::LtEq => a.relation(Rel::LtEq, b),
        BinaryOpKind::GtEq => a.relation(Rel::GtEq, b),
        BinaryOpKind::ApproxEq => a.approx_eq(b),
        BinaryOpKind::SeqConcat => a.seq_concat(b),
        BinaryOpKind::BitAnd => a.bit_and(b),
        BinaryOpKind::BitOr => a.bit_or(b),
        BinaryOpKind::BitXor => a.bit_xor(b),
        BinaryOpKind::Shl => a.shl(b),
        BinaryOpKind::Shr => a.shr(b),
        other => panic!("not an operator of the differential: {other:?}"),
    }
}

/// The values every operator is tried on.
fn pool() -> Vec<Pair> {
    let mut pool = vec![
        int(0),
        int(1),
        int(-1),
        int(2),
        int(7),
        int(-7),
        int(i64::MAX),
        int(i64::MIN),
        big(BigInt::from_i64(1).shl_bits(70)),
        big(BigInt::from_i64(1).shl_bits(70).negated()),
        rat(7, 2),
        rat(-1, 3),
        float(0.0),
        float(-0.0),
        float(1.5),
        float(-2.5),
        float(0.1),
        float(f64::NAN),
        float(f64::INFINITY),
        float(1e300),
        decimal("4.50"),
        decimal("-0.10"),
        pair("true", RuntimeValue::Bool(true), LogosValue::Bool(true)),
        pair("false", RuntimeValue::Bool(false), LogosValue::Bool(false)),
        text("ab"),
        text(""),
        text("héllo"),
        text("é"),
        list(&[1, 2]),
        list(&[]),
        pair("nothing", RuntimeValue::Nothing, LogosValue::Nothing),
    ];
    let dt = pair(
        "dt",
        exact::new_infinitesimal("dt", 1, &[0]).unwrap(),
        LogosValue::infinitesimal("dt", 1, &[0]).unwrap(),
    );
    let three = int(3);
    let two = int(2);
    let three_plus_dt = derived("3 + dt", &three, &dt, BinaryOpKind::Add);
    let h = pair(
        "1/dt",
        exact::generator_of(&dt.rt).unwrap(),
        tower::generator_of(&dt.lv).unwrap(),
    );
    let parity = derived("(1/dt) mod 2", &h, &two, BinaryOpKind::Modulo);
    let even = pair(
        "even dt",
        exact::new_infinitesimal("dt", 2, &[0]).unwrap(),
        LogosValue::infinitesimal("dt", 2, &[0]).unwrap(),
    );
    let root2 = Algebraic::from_i64(2).sqrt().unwrap();
    let sqrt2 = pair(
        "√2",
        exact::from_algebraic(root2.clone()),
        tower::from_algebraic(root2),
    );
    let pi = pair(
        "π",
        exact::from_exact_real(ExactReal::pi()),
        tower::from_exact_real(ExactReal::pi()),
    );
    let omega = pair(
        "ω",
        exact::from_ordinal(Ordinal::omega()),
        tower::from_ordinal(Ordinal::omega()),
    );
    let guess = pair(
        "provisional 4",
        exact::provisional(RuntimeValue::Int(4), 100, 2),
        tower::provisional(LogosValue::Int(4), 100, 2),
    );
    let date = pair(
        "a date",
        RuntimeValue::Date(19000),
        LogosValue::other(LogosDate::new(19000)),
    );
    let later = pair(
        "a later date",
        RuntimeValue::Date(19031),
        LogosValue::other(LogosDate::new(19031)),
    );
    let span = pair(
        "a span",
        RuntimeValue::Span { months: 1, days: 14 },
        LogosValue::other(LogosSpan::new(1, 14)),
    );
    let moment = pair(
        "a moment",
        RuntimeValue::Moment(1_700_000_000_000_000_000),
        LogosValue::other(LogosMoment::new(1_700_000_000_000_000_000)),
    );
    let noon = pair(
        "noon",
        RuntimeValue::Time(43_200_000_000_000),
        LogosValue::other(LogosTime::new(43_200_000_000_000)),
    );
    let mixed = pair(
        "[dt, 1]",
        RuntimeValue::List(std::rc::Rc::new(std::cell::RefCell::new(
            logicaffeine_compile::interpreter::ListRepr::from_values(vec![dt.rt.clone(), RuntimeValue::Int(1)]),
        ))),
        LogosValue::list(vec![dt.lv.clone(), LogosValue::Int(1)]),
    );
    let tuple = pair(
        "(1, ab)",
        RuntimeValue::Tuple(std::rc::Rc::new(vec![
            RuntimeValue::Int(1),
            RuntimeValue::Text(std::rc::Rc::new("ab".to_string())),
        ])),
        LogosValue::Tuple(std::sync::Arc::new(vec![LogosValue::Int(1), LogosValue::text("ab")])),
    );
    let dx = pair(
        "dx much smaller than dt",
        exact::new_ordered_infinitesimal("dx", &dt.rt, &RuntimeValue::Nothing, 1, &[0]).unwrap(),
        tower::new_ordered_infinitesimal("dx", &dt.lv, &LogosValue::Nothing, 1, &[0]).unwrap(),
    );
    let dt_plus_dx = derived("dt + dx", &dt, &dx, BinaryOpKind::Add);
    let dx_over_dt = derived("dx/dt", &dx, &dt, BinaryOpKind::ExactDivide);
    let root = pair(
        "√(dt + dx)",
        tower::real_fn(tower::RealFn::Sqrt, &dt_plus_dx.rt).unwrap(),
        tower::real_fn(tower::RealFn::Sqrt, &dt_plus_dx.lv).unwrap(),
    );
    let k = pair(
        "k of no stated order",
        exact::new_infinitesimal("k", 1, &[0]).unwrap(),
        LogosValue::infinitesimal("k", 1, &[0]).unwrap(),
    );
    let germ = pair(
        "exp(dt + dx)",
        tower::real_fn(tower::RealFn::Exp, &dt_plus_dx.rt).unwrap(),
        tower::real_fn(tower::RealFn::Exp, &dt_plus_dx.lv).unwrap(),
    );
    let hx = pair(
        "1/dx",
        exact::generator_of(&dx.rt).unwrap(),
        tower::generator_of(&dx.lv).unwrap(),
    );
    let dx_parity = derived("(1/dx) mod 2", &hx, &two, BinaryOpKind::Modulo);
    let both_parities = derived("(1/dt) mod 2 + (1/dx) mod 2", &parity, &dx_parity, BinaryOpKind::Add);
    let integer_of_both = derived("1/dt + 1/dx", &h, &hx, BinaryOpKind::Add);
    let half_integer_of_both = derived("1/(2·dt) + 1/dx", &derived("1/(2·dt)", &h, &two, BinaryOpKind::FloorDivide), &hx, BinaryOpKind::Add);
    pool.extend([
        dt, three_plus_dt, h, parity, even, sqrt2, pi, omega, guess, date, later, span, moment, noon,
        mixed, tuple, dx, dt_plus_dx, dx_over_dt, root, k, germ, hx, dx_parity, both_parities, integer_of_both,
        half_integer_of_both,
    ]);
    pool
}

const BINARY: &[BinaryOpKind] = &[
    BinaryOpKind::Add,
    BinaryOpKind::Subtract,
    BinaryOpKind::Multiply,
    BinaryOpKind::Divide,
    BinaryOpKind::ExactDivide,
    BinaryOpKind::FloorDivide,
    BinaryOpKind::Modulo,
    BinaryOpKind::Pow,
    BinaryOpKind::Eq,
    BinaryOpKind::NotEq,
    BinaryOpKind::Lt,
    BinaryOpKind::Gt,
    BinaryOpKind::LtEq,
    BinaryOpKind::GtEq,
    BinaryOpKind::ApproxEq,
    BinaryOpKind::SeqConcat,
    BinaryOpKind::BitAnd,
    BinaryOpKind::BitOr,
    BinaryOpKind::BitXor,
    BinaryOpKind::Shl,
    BinaryOpKind::Shr,
];

/// What a result looks like from outside: its display and type, or its refusal.
fn seen_rt(r: Result<RuntimeValue, String>) -> Result<(String, String), String> {
    r.map(|v| (v.to_display_string(), v.type_name().to_string()))
}

fn seen_lv(r: Result<LogosValue, String>) -> Result<(String, String), String> {
    r.map(|v| (v.display_string(), v.type_name().to_string()))
}

/// A result the interpreter gives as `Ok` of a value, read as the compiled model gives an `i64` or a
/// `bool`.
fn seen_int(r: Result<i64, String>) -> Result<(String, String), String> {
    r.map(|n| (n.to_string(), "Int".to_string()))
}

fn seen_bool(r: Result<bool, String>) -> Result<(String, String), String> {
    r.map(|b| (b.to_string(), "Bool".to_string()))
}

fn seen_items_rt(r: Result<Vec<RuntimeValue>, String>) -> Result<Vec<(String, String)>, String> {
    r.map(|vs| vs.iter().map(|v| (v.to_display_string(), v.type_name().to_string())).collect())
}

fn seen_items_lv(r: Result<Vec<LogosValue>, String>) -> Result<Vec<(String, String)>, String> {
    r.map(|vs| vs.iter().map(|v| (v.display_string(), v.type_name().to_string())).collect())
}

/// A pair whose operation would build a number or a list too large to be worth computing in a test:
/// the interpreter and the model take the same path to it, so its answer says nothing either does not.
fn too_large(op: BinaryOpKind, a: &Pair, b: &Pair) -> bool {
    let huge = |p: &Pair| matches!(p.rt, RuntimeValue::Int(n) if n > 64);
    let list = |p: &Pair| matches!(p.lv, LogosValue::List(_));
    match op {
        BinaryOpKind::Pow => huge(b) && !matches!(a.rt, RuntimeValue::Int(-1..=1)),
        BinaryOpKind::Multiply => (list(a) && huge(b)) || (huge(a) && list(b)),
        _ => false,
    }
}

#[test]
fn every_operator_answers_the_same_in_both_value_models() {
    let pool = pool();
    let mut differences = Vec::new();
    let mut compared = 0usize;
    for a in &pool {
        for b in &pool {
            for &op in BINARY {
                if too_large(op, a, b) {
                    continue;
                }
                let rt = seen_rt(arith::binary_op(op, a.rt.clone(), b.rt.clone()));
                let lv = seen_lv(lv_op(op, &a.lv, &b.lv));
                compared += 1;
                if rt != lv {
                    differences.push(format!(
                        "{} {op:?} {}:\n    interpreter: {rt:?}\n    compiled:    {lv:?}",
                        a.name, b.name
                    ));
                }
            }
        }
    }
    assert!(compared > 10_000, "the differential compared only {compared} cases");
    assert!(
        differences.is_empty(),
        "{} of {compared} operations answer differently:\n{}",
        differences.len(),
        differences.join("\n")
    );
}

#[test]
fn every_unary_answer_is_the_same_in_both_value_models() {
    let mut differences = Vec::new();
    for v in &pool() {
        let cases: Vec<(&str, Result<(String, String), String>, Result<(String, String), String>)> = vec![
            ("not", seen_rt(arith::not_value(v.rt.clone())), seen_lv(v.lv.not())),
            (
                "floor",
                seen_rt(builtins::call_builtin(builtins::BuiltinId::Floor, vec![v.rt.clone()])),
                seen_lv(v.lv.floor()),
            ),
            (
                "ceil",
                seen_rt(builtins::call_builtin(builtins::BuiltinId::Ceil, vec![v.rt.clone()])),
                seen_lv(v.lv.ceil()),
            ),
            (
                "round",
                seen_rt(builtins::call_builtin(builtins::BuiltinId::Round, vec![v.rt.clone()])),
                seen_lv(v.lv.round()),
            ),
            (
                "standard part",
                seen_rt(exact::standard_part(&v.rt)),
                seen_lv(tower::standard_part(&v.lv)),
            ),
            (
                "is infinitesimal",
                seen_rt(exact::is_infinitesimal(&v.rt)),
                seen_lv(tower::is_infinitesimal(&v.lv)),
            ),
            (
                "is limited",
                seen_rt(exact::is_limited(&v.rt)),
                seen_lv(tower::is_limited(&v.lv)),
            ),
            (
                "truth",
                v.rt.decided_truth().map(|b| (b.to_string(), "Bool".to_string())),
                v.lv.decided_truth().map(|b| (b.to_string(), "Bool".to_string())),
            ),
            (
                "display",
                Ok((v.rt.to_display_string(), v.rt.type_name().to_string())),
                Ok((v.lv.display_string(), v.lv.type_name().to_string())),
            ),
        ];
        for (what, rt, lv) in cases {
            if rt != lv {
                differences.push(format!(
                    "{what} of {}:\n    interpreter: {rt:?}\n    compiled:    {lv:?}",
                    v.name
                ));
            }
        }
    }
    assert!(
        differences.is_empty(),
        "{} unary answers differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
}

#[test]
fn equality_is_the_same_relation_in_both_value_models() {
    let pool = pool();
    for a in &pool {
        for b in &pool {
            assert_eq!(
                compare::values_equal(&a.rt, &b.rt),
                a.lv.values_equal(&b.lv),
                "{} = {}",
                a.name,
                b.name
            );
        }
    }
}

#[test]
fn the_differential_sees_a_wrong_answer() {
    let seven = int(7);
    let two = int(2);
    let rt = seen_rt(arith::binary_op(BinaryOpKind::Divide, seven.rt.clone(), two.rt.clone()));
    let wrong = seen_lv(seven.lv.exact_div(&two.lv));
    assert_ne!(rt, wrong, "7 / 2 truncates to 3; an exact 7/2 must read as a difference");
}

fn call(id: builtins::BuiltinId, args: &[&RuntimeValue]) -> Result<RuntimeValue, String> {
    builtins::call_builtin(id, args.iter().map(|v| (*v).clone()).collect())
}

/// The builtins a compiled program lowers onto the value model — `abs`, `sqrt`, `min`, `max`, `pow` —
/// answer as the interpreter's builtins do on every value and pair of values.
#[test]
fn every_lifted_builtin_answers_the_same_in_both_value_models() {
    use builtins::BuiltinId;
    let pool = pool();
    let mut differences = Vec::new();
    let mut note = |what: String, rt: Result<(String, String), String>, lv: Result<(String, String), String>| {
        if rt != lv {
            differences.push(format!("{what}:\n    interpreter: {rt:?}\n    compiled:    {lv:?}"));
        }
    };
    for a in &pool {
        note(format!("abs({})", a.name), seen_rt(call(BuiltinId::Abs, &[&a.rt])), seen_lv(a.lv.abs()));
        note(format!("sqrt({})", a.name), seen_rt(call(BuiltinId::Sqrt, &[&a.rt])), seen_lv(a.lv.sqrt()));
        for b in &pool {
            note(
                format!("min({}, {})", a.name, b.name),
                seen_rt(call(BuiltinId::Min, &[&a.rt, &b.rt])),
                seen_lv(a.lv.min(&b.lv)),
            );
            note(
                format!("max({}, {})", a.name, b.name),
                seen_rt(call(BuiltinId::Max, &[&a.rt, &b.rt])),
                seen_lv(a.lv.max(&b.lv)),
            );
            if !too_large(BinaryOpKind::Pow, a, b) {
                note(
                    format!("pow({}, {})", a.name, b.name),
                    seen_rt(call(BuiltinId::Pow, &[&a.rt, &b.rt])),
                    seen_lv(a.lv.power(&b.lv)),
                );
            }
        }
    }
    assert!(differences.is_empty(), "{} builtin answers differ:\n{}", differences.len(), differences.join("\n"));
}

/// A collection the exact extension reaches is read in both models alike: its length, membership,
/// the items a `Repeat` binds, an item by index, and a push.
#[test]
fn every_collection_reading_is_the_same_in_both_value_models() {
    use logicaffeine_compile::semantics::collections;
    let pool = pool();
    let mut differences = Vec::new();
    let mut note = |what: String, rt: String, lv: String| {
        if rt != lv {
            differences.push(format!("{what}:\n    interpreter: {rt}\n    compiled:    {lv}"));
        }
    };
    for c in &pool {
        note(
            format!("length of {}", c.name),
            format!("{:?}", seen_rt(collections::length_of(&c.rt))),
            format!("{:?}", seen_int(c.lv.length_of())),
        );
        let rt_pop = {
            let fresh = match &c.rt {
                RuntimeValue::List(items) => RuntimeValue::List(std::rc::Rc::new(std::cell::RefCell::new(
                    items.borrow().clone(),
                ))),
                other => other.clone(),
            };
            collections::list_pop(&fresh).map(|p| format!("{} then {}", p.to_display_string(), fresh.to_display_string()))
        };
        let lv_pop = {
            let mut fresh = c.lv.clone();
            fresh.pop_value().map(|p| format!("{} then {}", p.display_string(), fresh.display_string()))
        };
        note(format!("pop from {}", c.name), format!("{rt_pop:?}"), format!("{lv_pop:?}"));
        note(
            format!("items of {}", c.name),
            format!("{:?}", seen_items_rt(collections::iteration_snapshot(&c.rt))),
            format!("{:?}", seen_items_lv(c.lv.items())),
        );
        for v in &pool {
            note(
                format!("{} contains {}", c.name, v.name),
                format!("{:?}", seen_rt(collections::contains(&c.rt, &v.rt))),
                format!("{:?}", seen_bool(c.lv.contains_value(&v.lv))),
            );
            note(
                format!("item {} of {}", v.name, c.name),
                format!("{:?}", seen_rt(collections::index_get(&c.rt, &v.rt))),
                format!("{:?}", seen_lv(c.lv.item(&v.lv))),
            );
            let rt_after = {
                let fresh = match &c.rt {
                    RuntimeValue::List(items) => RuntimeValue::List(std::rc::Rc::new(std::cell::RefCell::new(
                        items.borrow().clone(),
                    ))),
                    other => other.clone(),
                };
                collections::list_push(&fresh, v.rt.clone()).map(|()| fresh.to_display_string())
            };
            let lv_after = {
                let mut fresh = c.lv.clone();
                fresh.push_value(v.lv.clone()).map(|()| fresh.display_string())
            };
            note(
                format!("push {} to {}", v.name, c.name),
                format!("{rt_after:?}"),
                format!("{lv_after:?}"),
            );
            let fresh_rt = || match &c.rt {
                RuntimeValue::List(items) => RuntimeValue::List(std::rc::Rc::new(std::cell::RefCell::new(
                    items.borrow().clone(),
                ))),
                other => other.clone(),
            };
            let rt_set = {
                let fresh = fresh_rt();
                collections::index_set(&fresh, &v.rt, RuntimeValue::Int(9)).map(|()| fresh.to_display_string())
            };
            let lv_set = {
                let mut fresh = c.lv.clone();
                fresh.set_item(&v.lv, LogosValue::Int(9)).map(|()| fresh.display_string())
            };
            note(format!("set item {} of {} to 9", v.name, c.name), format!("{rt_set:?}"), format!("{lv_set:?}"));
        }
    }
    assert!(differences.is_empty(), "{} collection readings differ:\n{}", differences.len(), differences.join("\n"));
}

/// The extension's builtins run through one dispatcher on either value model; on every argument the
/// pool offers, both models give the same value or the same refusal.
#[test]
fn every_extension_builtin_answers_the_same_in_both_value_models() {
    use logicaffeine_base::extension::{call as ext, Builtin};
    let pool = pool();
    let mut differences = Vec::new();
    let mut note = |what: String, rt: Result<(String, String), String>, lv: Result<(String, String), String>| {
        if rt != lv {
            differences.push(format!("{what}:\n    interpreter: {rt:?}\n    compiled:    {lv:?}"));
        }
    };
    let rt_text = |t: &str| RuntimeValue::Text(std::rc::Rc::new(t.to_string()));
    for a in &pool {
        for b in [Builtin::StandardPart, Builtin::IsInfinitesimal, Builtin::IsLimited, Builtin::GeneratorOf, Builtin::StateSize] {
            note(
                format!("{}({})", b.name(), a.name),
                seen_rt(ext(b, vec![a.rt.clone()])),
                seen_lv(ext(b, vec![a.lv.clone()])),
            );
        }
        for rule in ["the limit", "the limit superior", "the Cesàro mean", "the median"] {
            note(
                format!("limitOf({}, {rule})", a.name),
                seen_rt(ext(Builtin::LimitOf, vec![a.rt.clone(), rt_text(rule)])),
                seen_lv(ext(Builtin::LimitOf, vec![a.lv.clone(), LogosValue::text(rule)])),
            );
        }
        note(
            format!("provisional({})", a.name),
            seen_rt(ext(Builtin::Provisional, vec![a.rt.clone(), RuntimeValue::Int(10), RuntimeValue::Int(1)])),
            seen_lv(ext(Builtin::Provisional, vec![a.lv.clone(), LogosValue::Int(10), LogosValue::Int(1)])),
        );
        note(
            format!("infinitesimal named {}", a.name),
            seen_rt(ext(Builtin::Infinitesimal, vec![a.rt.clone(), RuntimeValue::Int(2), RuntimeValue::Int(0)])),
            seen_lv(ext(Builtin::Infinitesimal, vec![a.lv.clone(), LogosValue::Int(2), LogosValue::Int(0)])),
        );
        for b in &pool {
            note(
                format!("infinitelyClose({}, {})", a.name, b.name),
                seen_rt(ext(Builtin::InfinitelyClose, vec![a.rt.clone(), b.rt.clone()])),
                seen_lv(ext(Builtin::InfinitelyClose, vec![a.lv.clone(), b.lv.clone()])),
            );
        }
    }
    assert!(differences.is_empty(), "{} builtin answers differ:\n{}", differences.len(), differences.join("\n"));
}
