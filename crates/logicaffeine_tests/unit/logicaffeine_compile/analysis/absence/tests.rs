use super::*;
use crate::ui_bridge::with_parsed_program;

/// Where `nothing` can be stored in `source`, read by `check` with the program and its names.
fn absence(source: &str, check: impl FnOnce(&Absence, &[Stmt], &Interner)) {
    with_parsed_program(source, |parsed, interner| {
        let (stmts, registry, _) = parsed.expect("the program parses");
        let types = TypeEnv::infer_program(stmts, interner, registry);
        check(&of_program(stmts, &types, interner), stmts, interner);
    });
}

fn sym(interner: &Interner, name: &str) -> Symbol {
    interner.lookup(name).unwrap_or_else(|| panic!("`{name}` is not in the program"))
}

/// The `Let` that binds `name`, in `## Main` or any function's body.
fn binding<'s, 'a>(stmts: &'s [Stmt<'a>], interner: &Interner, name: &str) -> &'s Stmt<'a> {
    fn find<'s, 'a>(stmts: &'s [Stmt<'a>], name: Symbol) -> Option<&'s Stmt<'a>> {
        stmts.iter().find_map(|stmt| match stmt {
            Stmt::Let { var, .. } if *var == name => Some(stmt),
            Stmt::FunctionDef { body, .. } => find(body, name),
            _ => None,
        })
    }
    find(stmts, sym(interner, name)).unwrap_or_else(|| panic!("no `Let {name}`"))
}

fn depths(of: &[usize]) -> Depths {
    of.iter().copied().collect()
}

#[test]
fn a_program_that_stores_no_nothing_has_no_absence() {
    absence("## Main\nLet x be 5.\nLet xs be [1, 2].\nPush 3 to xs.\nShow x.\n", |a, _, _| assert!(a.is_empty()));
}

#[test]
fn a_binding_set_to_nothing_can_itself_be_nothing() {
    absence("## Main\nLet x be 5.\nSet x to nothing.\nLet y be 3.\nShow x.\n", |a, stmts, i| {
        assert_eq!(a.binding(binding(stmts, i, "x")), Some(&depths(&[0])));
        assert_eq!(a.binding(binding(stmts, i, "y")), None, "a binding no nothing reaches keeps its type");
    });
}

#[test]
fn an_item_set_to_nothing_makes_the_items_of_its_list_nullable() {
    absence("## Main\nLet xs be [1, 2].\nSet item 1 of xs to nothing.\nShow xs.\n", |a, stmts, i| {
        assert_eq!(a.binding(binding(stmts, i, "xs")), Some(&depths(&[1])));
    });
}

#[test]
fn an_item_two_lists_deep_is_nullable_at_depth_two() {
    absence("## Main\nLet m be [[1, 2], [3]].\nSet item 1 of item 1 of m to nothing.\nShow m.\n", |a, stmts, i| {
        assert_eq!(a.binding(binding(stmts, i, "m")), Some(&depths(&[2])));
    });
}

#[test]
fn a_pushed_nothing_makes_the_items_nullable() {
    absence("## Main\nLet xs be [1].\nPush nothing to xs.\nShow xs.\n", |a, stmts, i| {
        assert_eq!(a.binding(binding(stmts, i, "xs")), Some(&depths(&[1])));
    });
}

#[test]
fn a_list_literal_with_nothing_in_it_holds_nullable_items() {
    absence("## Main\nLet xs be [1, nothing, 3].\nShow xs.\n", |a, stmts, i| {
        assert_eq!(a.binding(binding(stmts, i, "xs")), Some(&depths(&[1])));
    });
}

#[test]
fn a_map_value_set_to_nothing_makes_the_values_nullable() {
    absence(
        "## Main\nLet m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\nSet item \"b\" of m to nothing.\nShow m.\n",
        |a, stmts, i| assert_eq!(a.binding(binding(stmts, i, "m")), Some(&depths(&[1]))),
    );
}

#[test]
fn values_connected_by_a_copy_share_one_representation_both_ways() {
    absence("## Main\nLet xs be [1].\nLet ys be xs.\nPush nothing to ys.\nShow xs.\n", |a, stmts, i| {
        assert_eq!(a.binding(binding(stmts, i, "ys")), Some(&depths(&[1])));
        assert_eq!(a.binding(binding(stmts, i, "xs")), Some(&depths(&[1])), "the source of a copy holds what the copy can");
    });
}

#[test]
fn an_item_read_out_of_a_nullable_list_can_be_nothing() {
    absence("## Main\nLet xs be [1, nothing].\nLet x be item 2 of xs.\nShow x.\n", |a, stmts, i| {
        assert_eq!(a.binding(binding(stmts, i, "x")), Some(&depths(&[0])));
    });
}

#[test]
fn arithmetic_makes_a_value_of_its_own() {
    absence("## Main\nLet x be 5.\nSet x to nothing.\nLet y be x + 1.\nShow y.\n", |a, stmts, i| {
        assert_eq!(a.binding(binding(stmts, i, "x")), Some(&depths(&[0])));
        assert_eq!(a.binding(binding(stmts, i, "y")), None, "a sum is a number or a failure, never nothing");
    });
}

#[test]
fn nothing_passed_to_a_parameter_reaches_the_parameter_and_what_it_returns() {
    absence(
        "## To echo (a: Int) -> Int:\n    Return a.\n## To double (b: Int) -> Int:\n    Return b * 2.\n## Main\nShow echo(nothing).\nShow double(4).\n",
        |a, _, i| {
            assert_eq!(a.parameter(sym(i, "echo"), sym(i, "a")), Some(&depths(&[0])));
            assert_eq!(a.returned(sym(i, "echo")), Some(&depths(&[0])));
            assert_eq!(a.parameter(sym(i, "double"), sym(i, "b")), None);
            assert_eq!(a.returned(sym(i, "double")), None);
        },
    );
}

#[test]
fn a_returned_nothing_reaches_the_binding_the_call_fills() {
    absence(
        "## To missing () -> Int:\n    Return nothing.\n## Main\nLet x be missing().\nShow x.\n",
        |a, stmts, i| {
            assert_eq!(a.returned(sym(i, "missing")), Some(&depths(&[0])));
            assert_eq!(a.binding(binding(stmts, i, "x")), Some(&depths(&[0])));
        },
    );
}

#[test]
fn a_field_set_to_nothing_is_nullable_for_every_value_of_its_struct() {
    absence(
        "## A Box has:\n    a value, which is Int.\n    a size, which is Int.\n## Main\nLet b be a new Box with value 4 and size 2.\nSet b's value to nothing.\nShow b.\n",
        |a, _, i| {
            assert_eq!(a.field(sym(i, "Box"), sym(i, "value")), Some(depths(&[0])));
            assert_eq!(a.field(sym(i, "Box"), sym(i, "size")), None, "another field of the struct keeps its type");
        },
    );
}

#[test]
fn a_popped_item_of_a_nullable_list_can_be_nothing() {
    absence("## Main\nLet xs be [1, nothing].\nPop from xs into y.\nLet z be y.\nShow z.\n", |a, stmts, i| {
        assert_eq!(a.binding(binding(stmts, i, "z")), Some(&depths(&[0])), "what is popped is an item");
    });
}

#[test]
fn a_list_pushed_into_itself_is_bounded_by_the_depth_limit() {
    absence("## Main\nLet xs be [nothing].\nPush xs to xs.\nShow length of xs.\n", |a, stmts, i| {
        let found = a.binding(binding(stmts, i, "xs")).expect("its items can be nothing");
        assert!(found.contains(&1));
        assert!(found.iter().all(|d| *d <= DEPTH_LIMIT), "{found:?} passes the depth limit");
    });
}
