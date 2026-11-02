//! The extension's builtins on base's own value model (`tower::tests::Val`): every one is read and
//! run by [`call`], generic over the value model, so these pass without the interpreter.

use super::*;
use crate::tower::tests::{calc, dt, int, show, Val};
use crate::tower::{Op, TowerValue};

fn text(t: &str) -> Val {
    Val::Text(t.to_string())
}

fn refusal(builtin: Builtin, args: Vec<Val>) -> String {
    match call(builtin, args) {
        Err(e) => e,
        Ok(v) => panic!("{} answered {} where it should refuse", builtin.name(), show(&v)),
    }
}

#[test]
fn every_builtin_is_called_by_its_own_name() {
    let mut seen = std::collections::HashSet::new();
    for b in Builtin::ALL {
        assert!(seen.insert(b.name()), "two builtins share the name {}", b.name());
        assert_eq!(Builtin::named(b.name()), Some(b));
    }
    assert_eq!(Builtin::named("sqrt"), None);
    assert_eq!(Builtin::named("StandardPart"), None, "names are matched as a program spells them");
}

#[test]
fn an_infinitesimal_is_named_by_text_and_constrained_by_naturals() {
    let dt = call(Builtin::Infinitesimal, vec![text("dt"), int(1), int(0)]).unwrap();
    assert_eq!(show(&dt), "dt");
    let h = call(Builtin::Infinitesimal, vec![text("h"), int(2), int(0)]).unwrap();
    assert_eq!(show(&call(Builtin::GeneratorOf, vec![h]).unwrap()), "1/h");
    assert_eq!(
        refusal(Builtin::Infinitesimal, vec![int(3), int(1), int(0)]),
        "an infinitesimal's name is Text, got Int"
    );
    assert_eq!(
        refusal(Builtin::Infinitesimal, vec![text("dt"), int(-2), int(0)]),
        "an infinitesimal's residue facts are natural numbers, got -2"
    );
    assert_eq!(
        refusal(Builtin::Infinitesimal, vec![text("dt"), int(2), text("odd")]),
        "an infinitesimal's residue facts are natural numbers, got odd"
    );
}

#[test]
fn the_predicates_and_the_standard_part_read_the_value() {
    let three_and_a_bit = calc(Op::Add, int(3), dt()).unwrap();
    assert_eq!(show(&call(Builtin::StandardPart, vec![three_and_a_bit.clone()]).unwrap()), "3");
    assert_eq!(show(&call(Builtin::IsInfinitesimal, vec![dt()]).unwrap()), "true");
    assert_eq!(show(&call(Builtin::IsInfinitesimal, vec![three_and_a_bit.clone()]).unwrap()), "false");
    assert_eq!(show(&call(Builtin::IsLimited, vec![three_and_a_bit.clone()]).unwrap()), "true");
    assert_eq!(
        show(&call(Builtin::InfinitelyClose, vec![three_and_a_bit.clone(), int(3)]).unwrap()),
        "true"
    );
    assert_eq!(
        show(&call(Builtin::InfinitelyClose, vec![three_and_a_bit, int(4)]).unwrap()),
        "false"
    );
}

#[test]
fn a_limit_rule_is_named_by_text() {
    assert_eq!(show(&call(Builtin::AtTheLimit, vec![text("the Cesàro mean")]).unwrap()), "nothing");
    assert_eq!(
        refusal(Builtin::AtTheLimit, vec![int(1)]),
        "a limit rule is named by Text, got Int"
    );
    assert_eq!(
        refusal(Builtin::LimitOf, vec![int(1), int(1)]),
        "a limit rule is named by Text, got Int"
    );
    assert_eq!(
        refusal(Builtin::LimitOf, vec![int(1), text("the median")]),
        crate::tower::limit_rule("the median").unwrap_err()
    );
}

#[test]
fn a_provisional_answer_counts_with_naturals() {
    let answer = call(Builtin::Provisional, vec![Val::Bool(false), int(100), int(0)]).unwrap();
    assert!(show(&answer).contains("provisional"), "{}", show(&answer));
    assert_eq!(
        refusal(Builtin::Provisional, vec![Val::Bool(false), int(-1), int(0)]),
        "a provisional answer counts with natural numbers, got -1"
    );
    assert_eq!(
        refusal(Builtin::Provisional, vec![Val::Bool(false), int(1), text("x")]),
        "a provisional answer counts with natural numbers, got x"
    );
}

#[test]
fn a_clock_s_relation_is_named_by_text() {
    let n = call(Builtin::TripCount, vec![int(0), int(1), int(5), text("<")]).unwrap();
    assert_eq!(show(&n), "5");
    assert_eq!(
        refusal(Builtin::TripCount, vec![int(0), int(1), int(5), int(0)]),
        "a clock's relation is named by Text, got Int"
    );
}

#[test]
fn a_state_s_size_is_an_integer_and_an_unresolved_search_is_refused() {
    let size = call(Builtin::StateSize, vec![int(12345)]).unwrap();
    assert!(size.integer().is_some(), "{}", show(&size));
    assert!(refusal(Builtin::EveryNatural, vec![]).starts_with("a search over every natural number reached the runtime unresolved"));
}

#[test]
fn the_exact_constructors_answer_through_the_one_dispatcher() {
    assert_eq!(show(&call(Builtin::ExactSqrt, vec![int(2)]).unwrap()), "√2");
    assert_eq!(show(&call(Builtin::ExactSqrt, vec![int(16)]).unwrap()), "4");
    assert_eq!(show(&call(Builtin::ExactExp, vec![int(0)]).unwrap()), "1");
    let e = call(Builtin::ExactE, vec![]).unwrap();
    assert_eq!(show(&e), "e");
    assert_eq!(show(&call(Builtin::ExactLn, vec![e]).unwrap()), "1");
    assert_eq!(show(&call(Builtin::ExactPi, vec![]).unwrap()), "π");
    assert_eq!(show(&call(Builtin::ExactOmega, vec![]).unwrap()), "ω");
    assert_eq!(refusal(Builtin::ExactSqrt, vec![int(-1)]), "square root of a negative number");
    assert_eq!(refusal(Builtin::ExactLn, vec![int(0)]), "logarithm of a nonpositive number");
}
