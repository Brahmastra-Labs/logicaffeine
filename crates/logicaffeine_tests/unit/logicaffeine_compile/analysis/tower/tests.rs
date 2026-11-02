use super::*;
use crate::ui_bridge::with_parsed_program;

/// The answer for `source` as every tier sees it: parsed and resolved.
fn reached(source: &str, check: impl FnOnce(Option<&TowerTypes>, &Interner)) {
    with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("the program parses");
        check(TowerTypes::of_program(stmts, interner).as_ref(), interner);
    });
}

fn sym(interner: &Interner, name: &str) -> Symbol {
    interner.lookup(name).unwrap_or_else(|| panic!("`{name}` is not in the program"))
}

#[test]
fn a_program_the_extension_never_reaches_has_no_answer() {
    reached("## Main\nLet x be 7 / 2.\nShow x + 1.\n", |t, _| assert!(t.is_none()));
}

#[test]
fn the_infinitesimal_and_what_is_computed_from_it_are_reached() {
    reached(
        "## Main\nLet dt be an infinitesimal.\nLet x be 3 + 2 * dt.\nLet below be x < 4.\nLet n be 7.\nLet both be below and true.\nShow x.\n",
        |t, i| {
            let t = t.expect("the program reaches the extension");
            for name in ["dt", "x", "below"] {
                assert!(t.binding(None, sym(i, name)), "{name} holds a value of the extension");
            }
            assert!(!t.binding(None, sym(i, "n")), "a standard integer no reached value meets is not reached");
            assert!(!t.binding(None, sym(i, "both")), "`and` decides its operands");
        },
    );
}

#[test]
fn a_supertask_reaches_the_state_it_writes() {
    reached(
        "## Main\nLet dt be an infinitesimal.\nLet t be 0.\nLet lamp be 0.\nWhile t < 1:\n    Set lamp to 1 - lamp.\n    Set t to t + dt.\nShow lamp.\n",
        |t, i| {
            let t = t.expect("a supertask reaches the extension");
            assert!(t.binding(None, sym(i, "lamp")));
            assert!(t.binding(None, sym(i, "t")));
        },
    );
}

#[test]
fn a_declared_exact_type_is_reached_without_any_builtin() {
    reached("## Main\nLet x: Hyperreal be 1 / 2.\nLet y be x + 1.\nShow y.\n", |t, i| {
        let t = t.expect("a declared exact type reaches the extension");
        assert!(t.binding(None, sym(i, "x")));
        assert!(t.binding(None, sym(i, "y")));
    });
}

#[test]
fn a_search_s_function_returns_a_reached_value() {
    reached(
        "## To isSquare (m: Int) -> Bool:\n    Repeat for every natural n, checking up to 100 cases:\n        If n * n = m:\n            Return true.\n    Return false.\n\n## Main\nLet answer be isSquare(50).\nShow answer.\n",
        |t, i| {
            let t = t.expect("a provisional answer reaches the extension");
            assert!(t.returns(sym(i, "isSquare")));
            assert!(t.binding(None, sym(i, "answer")));
        },
    );
}

#[test]
fn a_parameter_a_call_passes_a_reached_value_is_reached() {
    reached(
        "## To twice (x: Int) -> Int:\n    Return x * 2.\n\n## To plain (k: Int) -> Int:\n    Return k + 1.\n\n## Main\nLet dt be an infinitesimal.\nShow twice(dt).\nShow plain(3).\n",
        |t, i| {
            let t = t.expect("the program reaches the extension");
            let twice = sym(i, "twice");
            assert!(t.binding(Some(twice), sym(i, "x")));
            assert!(t.returns(twice), "what a reached parameter computes is reached");
            assert!(!t.binding(Some(sym(i, "plain")), sym(i, "k")));
            assert!(!t.returns(sym(i, "plain")));
        },
    );
}

#[test]
fn an_item_of_a_reached_list_is_reached() {
    reached(
        "## Main\nLet dt be an infinitesimal.\nLet xs be a new Seq of Int.\nPush dt to xs.\nLet first be item 1 of xs.\nShow first.\n",
        |t, i| {
            let t = t.expect("the program reaches the extension");
            assert!(t.binding(None, sym(i, "xs")));
            assert!(t.binding(None, sym(i, "first")));
        },
    );
}

#[test]
fn a_builtin_of_the_extension_used_only_in_place_still_reaches_the_program() {
    reached("## Main\nShow the standard part of 3.\n", |t, _| {
        assert!(t.is_some(), "a program that calls standardPart reaches the extension");
    });
}

/// A map or a record stores a value of the extension under a key and keeps its own kind, as every backend that
/// lowers the bytecode reads it (`vm::tower::exact_write`); a list an item of the extension is stored into is one.
#[test]
fn a_map_or_a_record_an_exact_value_is_stored_in_keeps_its_own_kind() {
    reached(
        "## A Box has:\n    a v, which is Int.\n\n## Main\nLet dt be an infinitesimal.\nLet mutable m be a new Map of Text to Hyperreal.\nSet item \"a\" of m to dt.\nLet mutable b be a new Box with v 1.\nSet item \"v\" of b to dt.\nLet mutable xs be a new Seq of Hyperreal.\nPush 1 to xs.\nSet item 1 of xs to dt.\nLet mutable ys be [1, 2].\nSet item 1 of ys to dt.\nShow m.\nShow xs.\nShow ys.\n",
        |t, i| {
            let t = t.expect("the program reaches the extension");
            assert!(!t.binding(None, sym(i, "m")), "a map holding an exact value stays a map");
            assert!(!t.binding(None, sym(i, "b")), "a record holding an exact value stays a record");
            assert!(t.binding(None, sym(i, "xs")), "a list an exact item is stored into is exact");
            assert!(t.binding(None, sym(i, "ys")), "a list an exact item is stored into is exact");
        },
    );
}

/// A binding that is a map on one path and a list on another is a list wherever an exact item is stored into it.
#[test]
fn a_binding_keyed_on_one_path_only_is_reached_as_a_list() {
    reached(
        "## Main\nLet dt be an infinitesimal.\nLet mutable m be a new Map of Text to Hyperreal.\nIf dt is infinitesimal:\n    Set m to [1, 2].\nSet item 1 of m to dt.\nShow m.\n",
        |t, i| {
            let t = t.expect("the program reaches the extension");
            assert!(t.binding(None, sym(i, "m")));
        },
    );
}

/// A function that assigns a name it never binds itself assigns the program's root binding: a value of the extension
/// it stores there reaches that binding, which every function reads.
#[test]
fn a_function_storing_into_a_root_binding_reaches_the_root_binding() {
    reached(
        "## To make_exact:\n    Let dt be an infinitesimal.\n    Set x to dt.\n\n## To own_binding:\n    Let y be 1.\n    Let dt be an infinitesimal.\n    Set y to dt.\n    Show y.\n\n## To show_x:\n    Show x + 1.\n\n## Main\nLet mutable x be 3.\nLet y be 2.\nCall show_x.\nCall make_exact.\nCall own_binding.\nCall show_x.\nShow y.\n",
        |t, i| {
            let t = t.expect("the program reaches the extension");
            assert!(t.binding(None, sym(i, "x")), "Main's x holds what make_exact stores into it");
            assert!(t.binding(Some(sym(i, "show_x")), sym(i, "x")), "show_x reads Main's x");
            assert!(t.binding(Some(sym(i, "own_binding")), sym(i, "y")), "own_binding's y is its own");
            assert!(!t.binding(None, sym(i, "y")), "Main's y is not the y own_binding binds");
        },
    );
}

/// A function's loop variable is its own, whatever root binding shares its name: an exact value it holds reaches the
/// function's binding, not the program's.
#[test]
fn a_loop_variable_that_shadows_a_root_name_is_the_function_s_own() {
    reached(
        "## To walk:\n    Let dt be an infinitesimal.\n    Repeat for x in [dt, 2]:\n        Show x.\n\n## Main\nLet x be 5.\nCall walk.\nShow x.\n",
        |t, i| {
            let t = t.expect("the program reaches the extension");
            assert!(t.binding(Some(sym(i, "walk")), sym(i, "x")), "walk's loop variable holds an exact value");
            assert!(!t.binding(None, sym(i, "x")), "Main's x is not walk's loop variable");
        },
    );
}
