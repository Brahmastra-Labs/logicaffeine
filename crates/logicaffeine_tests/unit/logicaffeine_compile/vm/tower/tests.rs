use super::*;

/// The rule over registers named by which hold a value of the extension (`exact`), text, and a map or a record.
fn write(op: Op, exact: &[u16], text: &[u16], keyed: &[u16]) -> Option<u16> {
    exact_write(&op, |r| exact.contains(&r), |r| text.contains(&r), |r| keyed.contains(&r))
}

#[test]
fn a_builtin_of_the_extension_writes_one_whatever_it_reads() {
    let op = Op::CallBuiltin {
        dst: 4,
        builtin: BuiltinId::Infinitesimal,
        args_start: 1,
        arg_count: 3,
    };
    assert_eq!(write(op, &[], &[], &[]), Some(4));
}

#[test]
fn a_lifting_builtin_writes_one_only_over_an_argument_that_is_one() {
    for builtin in [
        BuiltinId::Floor,
        BuiltinId::Ceil,
        BuiltinId::Round,
        BuiltinId::Abs,
        BuiltinId::Sqrt,
        BuiltinId::Min,
        BuiltinId::Max,
        BuiltinId::Pow,
    ] {
        let op = Op::CallBuiltin {
            dst: 9,
            builtin,
            args_start: 2,
            arg_count: 2,
        };
        assert_eq!(write(op, &[], &[], &[]), None, "{builtin:?} over machine values");
        assert_eq!(write(op, &[3], &[], &[]), Some(9), "{builtin:?} over a second argument that is one");
        assert!(lifts(builtin).is_some());
    }
    let length = Op::CallBuiltin {
        dst: 9,
        builtin: BuiltinId::Length,
        args_start: 2,
        arg_count: 1,
    };
    assert_eq!(write(length, &[2], &[], &[]), None, "a builtin that does not lift keeps its own kind");
}

#[test]
fn an_operator_over_one_operand_that_is_one_writes_one() {
    let ops = [
        Op::Sub { dst: 0, lhs: 1, rhs: 2 },
        Op::Mul { dst: 0, lhs: 1, rhs: 2 },
        Op::Div { dst: 0, lhs: 1, rhs: 2 },
        Op::ExactDiv { dst: 0, lhs: 1, rhs: 2 },
        Op::FloorDiv { dst: 0, lhs: 1, rhs: 2 },
        Op::Mod { dst: 0, lhs: 1, rhs: 2 },
        Op::Pow { dst: 0, lhs: 1, rhs: 2 },
        Op::Lt { dst: 0, lhs: 1, rhs: 2 },
        Op::Gt { dst: 0, lhs: 1, rhs: 2 },
        Op::LtEq { dst: 0, lhs: 1, rhs: 2 },
        Op::GtEq { dst: 0, lhs: 1, rhs: 2 },
        Op::Eq { dst: 0, lhs: 1, rhs: 2 },
        Op::NotEq { dst: 0, lhs: 1, rhs: 2 },
        Op::ApproxEq { dst: 0, lhs: 1, rhs: 2 },
        Op::BitAnd { dst: 0, lhs: 1, rhs: 2 },
        Op::BitOr { dst: 0, lhs: 1, rhs: 2 },
        Op::BitXor { dst: 0, lhs: 1, rhs: 2 },
        Op::Shl { dst: 0, lhs: 1, rhs: 2 },
        Op::Shr { dst: 0, lhs: 1, rhs: 2 },
        Op::SeqConcat { dst: 0, lhs: 1, rhs: 2 },
    ];
    for op in ops {
        assert!(binary(&op).is_some(), "{op:?} has a runtime operation");
        assert_eq!(write(op, &[], &[], &[]), None, "{op:?} over machine values");
        assert_eq!(write(op, &[1], &[], &[]), Some(0), "{op:?} over a left operand that is one");
        assert_eq!(write(op, &[2], &[], &[]), Some(0), "{op:?} over a right operand that is one");
    }
}

#[test]
fn plus_beside_text_stays_the_text_it_joins() {
    let add = Op::Add { dst: 0, lhs: 1, rhs: 2 };
    assert_eq!(write(add, &[1], &[], &[]), Some(0));
    assert_eq!(write(add, &[1], &[2], &[]), None);
    assert_eq!(write(add, &[2], &[1], &[]), None);
    let add_assign = Op::AddAssign { dst: 3, src: 4 };
    assert_eq!(write(add_assign, &[4], &[], &[]), Some(3));
    assert_eq!(write(add_assign, &[3], &[], &[]), Some(3));
    assert_eq!(write(add_assign, &[4], &[3], &[]), None);
}

#[test]
fn a_list_holding_one_is_one_and_so_is_what_comes_out_of_it() {
    assert_eq!(write(Op::NewList { dst: 5, start: 1, count: 3 }, &[3], &[], &[]), Some(5));
    assert_eq!(write(Op::NewList { dst: 5, start: 1, count: 3 }, &[4], &[], &[]), None, "past the last item");
    assert_eq!(write(Op::ListPush { list: 5, value: 6 }, &[6], &[], &[]), Some(5));
    assert_eq!(write(Op::ListPush { list: 5, value: 6 }, &[5], &[], &[]), Some(5));
    assert_eq!(write(Op::ListPop { list: 5, dst: 7 }, &[5], &[], &[]), Some(7));
    assert_eq!(write(Op::ListPop { list: 5, dst: 7 }, &[], &[], &[]), None);
    assert_eq!(write(Op::Index { dst: 8, collection: 5, index: 1 }, &[5], &[], &[]), Some(8));
    assert_eq!(write(Op::IndexUnchecked { dst: 8, collection: 5, index: 1 }, &[5], &[], &[]), Some(8));
    assert_eq!(write(Op::Index { dst: 8, collection: 5, index: 1 }, &[1], &[], &[]), None, "an exact index of a machine list");
    assert_eq!(write(Op::Not { dst: 2, src: 1 }, &[1], &[], &[]), Some(2));
}

#[test]
fn an_item_stored_into_a_list_makes_it_one_but_a_map_or_a_record_keeps_its_kind() {
    let store = Op::SetIndex { collection: 5, index: 1, value: 6 };
    assert_eq!(write(store, &[6], &[], &[]), Some(5));
    assert_eq!(write(store, &[6], &[], &[5]), None);
    let unchecked = Op::SetIndexUnchecked { collection: 5, index: 1, value: 6 };
    assert_eq!(write(unchecked, &[6], &[], &[]), Some(5));
    assert_eq!(write(unchecked, &[5], &[], &[5]), None);
}

#[test]
fn an_op_outside_the_rule_writes_none() {
    assert_eq!(write(Op::Move { dst: 1, src: 2 }, &[2], &[], &[]), None, "a copy is the backend's own");
    assert_eq!(write(Op::Length { dst: 1, collection: 2 }, &[2], &[], &[]), None);
    assert_eq!(write(Op::Show { src: 2 }, &[2], &[], &[]), None);
}

#[test]
fn the_runtime_is_handed_every_argument_of_its_builtins_and_the_items_of_a_list_that_lands_as_one() {
    let call = Op::CallBuiltin {
        dst: 0,
        builtin: BuiltinId::TripCount,
        args_start: 10,
        arg_count: 4,
    };
    assert_eq!(demands(&call, false), vec![10, 11, 12, 13]);
    let list = Op::NewList { dst: 0, start: 4, count: 2 };
    assert_eq!(demands(&list, true), vec![4, 5]);
    assert!(demands(&list, false).is_empty());
    let lifting = Op::CallBuiltin {
        dst: 0,
        builtin: BuiltinId::Sqrt,
        args_start: 1,
        arg_count: 1,
    };
    assert!(demands(&lifting, true).is_empty(), "a lifting builtin converts what it reads");
}

#[test]
fn a_declared_type_of_the_extension_is_one_of_its_names_or_a_list_of_one() {
    use crate::vm::BoundaryType as B;
    for name in crate::analysis::tower::TOWER_TYPES {
        assert!(declared(&B::Builtin(name.to_string())), "{name}");
        assert!(declared(&B::Seq(Box::new(B::Builtin(name.to_string())))), "Seq of {name}");
        assert!(declared(&B::Seq(Box::new(B::Seq(Box::new(B::Builtin(name.to_string())))))));
    }
    for other in [B::Int, B::Float, B::Text, B::Builtin("Rational".to_string()), B::Builtin("Complex".to_string())] {
        assert!(!declared(&other), "{other:?}");
        assert!(!declared(&B::Seq(Box::new(other.clone()))), "Seq of {other:?}");
    }
}

#[test]
fn a_program_reaches_the_extension_by_a_builtin_or_a_declaration() {
    let compiled = |source: &str| crate::compile::compile_to_program_optimized(source).expect("compiles");
    assert!(reaches(&compiled("## Main\nLet dt be an infinitesimal.\nShow dt * 2.\n")));
    assert!(reaches(&compiled(
        "## No Inline\n## To half (x: Hyperreal) -> Hyperreal:\n    Return x / 2.\n\n## Main\nShow half(3).\n"
    )));
    assert!(reaches(&compiled(
        "## A Box has:\n    a v, which is Ordinal.\n\n## Main\nLet b be a new Box with v 3.\nShow b's v.\n"
    )));
    assert!(!reaches(&compiled(
        "## Main\nLet xs be [3, 1, 2].\nShow length of xs.\nLet r: Rational be 1 / 3.\nShow r.\n"
    )));
}

/// `definedReal` answers a value of the extension, yet reads its kind, base, name and closure as the machine values
/// they are: the runtime is handed none of them as one, and a program that only defines a real reaches the extension.
#[test]
fn a_real_a_closure_defines_is_one_made_from_machine_values() {
    let op = Op::CallBuiltin {
        dst: 7,
        builtin: BuiltinId::DefinedReal,
        args_start: 3,
        arg_count: 4,
    };
    assert!(BuiltinId::DefinedReal.answers_exact() && BuiltinId::DefinedReal.extension().is_none());
    assert_eq!(write(op, &[], &[3, 5], &[]), Some(7));
    assert!(demands(&op, true).is_empty(), "the kind, base, name and closure stay machine values");
    let compiled = |source: &str| crate::compile::compile_to_program_optimized(source).expect("compiles");
    assert!(reaches(&compiled(
        "## To seventh (i: Int) -> Int:\n    Return i % 7.\n\n## Main\nLet r be the real number whose digits in base 10 are seventh.\nShow r > 0.\n"
    )));
    assert!(reaches(&compiled(
        "## To half (n: Int) -> Rational:\n    Return 1 / 2.\n\n## Main\nLet r be the real number approximated by half.\nShow r.\n"
    )));
}
