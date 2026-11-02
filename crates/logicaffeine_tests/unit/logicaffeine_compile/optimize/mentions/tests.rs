//! Every name a statement touches, pinned at the unit level: the names `Swap` and `Check` carry as fields, an
//! identifier inside any expression form, what a closure captures, and foreign code touching everything; a nested
//! function body is not entered.
use super::*;
use crate::ui_bridge::with_parsed_program;

/// The `## Main` statements of `src`, with its interner.
fn with_main<R>(src: &str, f: impl for<'a> FnOnce(&'a [Stmt<'a>], &Interner) -> R) -> R {
    with_parsed_program(src, |parsed, interner| match parsed {
        Ok((stmts, _, _)) => {
            let first = stmts
                .iter()
                .position(|s| !matches!(s, Stmt::FunctionDef { .. } | Stmt::StructDef { .. }))
                .unwrap_or(stmts.len());
            f(&stmts[first..], interner)
        }
        Err(advice) => panic!("test program must parse: {advice}"),
    })
}

/// The statement of `## Main` at `index` touches exactly the listed names among `probe`.
fn touched(src: &str, index: usize, probe: &[&str]) -> Vec<String> {
    with_main(src, |main, interner| {
        let m = Mentions::of_stmt(&main[index]);
        probe
            .iter()
            .filter(|n| m.touches(n, interner))
            .map(|n| n.to_string())
            .collect()
    })
}

#[test]
fn a_swap_touches_the_two_names_it_exchanges() {
    let src = "## Main\nLet mutable a be 1.\nLet mutable b be 2.\nLet c be 3.\nSwap a and b.\n";
    assert_eq!(touched(src, 3, &["a", "b", "c"]), ["a", "b"]);
}

#[test]
fn a_capability_check_touches_its_subject_and_its_object() {
    let src = "## Definition\nA User has:\n    a name: Text.\n    a role: Text.\n\nA Document has:\n    an owner: Text.\n\n\
               ## Policy\nA User is admin if the user's role equals \"admin\".\nA User can edit the Document if:\n    \
               The user is admin, OR\n    The user's name equals the document's owner.\n\n\
               ## Main\nLet alice be a new User with name \"Alice\" and role \"guest\".\n\
               Let doc be a new Document with owner \"Alice\".\nCheck that alice can edit doc.\n";
    with_main(src, |main, interner| {
        let check = main
            .iter()
            .find(|s| matches!(s, Stmt::Check { .. }))
            .expect("the program has a Check");
        let m = Mentions::of_stmt(check);
        assert!(m.touches("alice", interner), "the subject is read");
        assert!(m.touches("doc", interner), "the object is read");
    });
}

#[test]
fn an_identifier_inside_a_record_constructor_is_touched() {
    let src = "## A Point has:\n    An x: Int.\n\n## Main\nLet mutable a be a new Seq of Int.\nPush 5 to a.\n\
               Let p be a new Point with x (item 1 of a).\n";
    assert_eq!(touched(src, 2, &["a", "p"]), ["a", "p"]);
}

#[test]
fn a_closure_touches_what_its_body_captures() {
    let src =
        "## Main\nLet mutable total be 0.\nLet bump be (e: Int) ->:\n    Set total to total + e.\n";
    assert_eq!(
        touched(src, 1, &["total", "bump", "e"]),
        ["total", "bump", "e"]
    );
}

#[test]
fn a_nested_function_body_is_its_own_scope() {
    let src =
        "## Main\nLet mutable a be 1.\n## To f (n: Int) -> Int:\n    Let a be n.\n    Return a.\n";
    with_parsed_program(src, |parsed, interner| match parsed {
        Ok((stmts, _, _)) => {
            let def = stmts
                .iter()
                .find(|s| matches!(s, Stmt::FunctionDef { .. }))
                .expect("a function definition");
            assert!(
                !Mentions::of_stmt(def).touches("a", interner),
                "the definition runs nothing where it stands"
            );
        }
        Err(advice) => panic!("test program must parse: {advice}"),
    });
}

#[test]
fn foreign_code_touches_every_name() {
    let src = "## Main\nLet x be 42.\nEscape to Rust:\n    println!(\"{}\", x);\n";
    with_main(src, |main, interner| {
        let m = Mentions::of_stmt(&main[1]);
        assert!(m.is_opaque());
        assert!(m.touches("x", interner));
        assert!(m.touches("anything_at_all", interner));
    });
}

#[test]
fn a_block_touches_what_any_statement_in_it_touches() {
    let src =
        "## Main\nLet mutable a be 1.\nLet mutable b be 2.\nIf a equals 1:\n    Swap a and b.\n";
    with_main(src, |main, interner| {
        let m = Mentions::of_block(&main[2..]);
        assert!(m.touches("a", interner) && m.touches("b", interner));
        assert!(!m.touches("c", interner));
    });
}
