use super::*;
use crate::ast::stmt::Literal;
use crate::intern::Interner;
use crate::ui_bridge::with_parsed_program;

/// The liveness of `source`, read by `check` with the program and its names.
fn liveness(source: &str, check: impl FnOnce(&LivenessResult, &[Stmt], &Interner)) {
    with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("the program parses");
        check(&LivenessResult::analyze(stmts), stmts, interner);
    });
}

fn sym(interner: &Interner, name: &str) -> Symbol {
    interner.lookup(name).unwrap_or_else(|| panic!("`{name}` is not in the program"))
}

/// The first statement `found` accepts, in any body of the program.
fn first<'a>(stmts: &'a [Stmt<'a>], found: &dyn Fn(&Stmt) -> bool) -> Option<&'a Stmt<'a>> {
    stmts.iter().find_map(|stmt| {
        if found(stmt) {
            return Some(stmt);
        }
        match stmt {
            Stmt::FunctionDef { body, .. }
            | Stmt::Zone { body, .. }
            | Stmt::While { body, .. }
            | Stmt::Repeat { body, .. } => first(body, found),
            Stmt::If { then_block, else_block, .. } => {
                first(then_block, found).or_else(|| else_block.and_then(|b| first(b, found)))
            }
            _ => None,
        }
    })
}

const ZONE_BREAK: &str = "## A Box has:\n    a value, which is Text.\n## To display (text: Text) -> Int:\n    Repeat for i from 1 to 1:\n        Inside a zone called \"Scratch\":\n            Let stored be a new Box with value text.\n            Show stored's value.\n            Break.\n        Show text.\n    Return 1.\n## Main\nShow display(\"kept\").\n";

#[test]
fn a_value_read_after_the_zone_a_break_leaves_is_live_at_the_break() {
    liveness(ZONE_BREAK, |l, stmts, i| {
        let at = first(stmts, &|s| matches!(s, Stmt::Break)).expect("the Break");
        assert!(l.after(at).expect("recorded").contains(&sym(i, "text")), "`Show text` follows the zone");
    });
}

#[test]
fn a_value_read_after_the_zone_a_return_leaves_is_live_at_the_return() {
    let source = "## To f (text: Text) -> Int:\n    Inside a zone called \"Z\":\n        Show \"in\".\n        Return 5.\n    Show text.\n    Return 7.\n## Main\nShow f(\"kept\").\n";
    liveness(source, |l, stmts, i| {
        let at = first(stmts, &|s| matches!(s, Stmt::Return { value: Some(Expr::Literal(Literal::Number(5))) }))
            .expect("the Return inside the zone");
        assert!(l.after(at).expect("recorded").contains(&sym(i, "text")), "the zone's exit reads `text`");
    });
}

#[test]
fn a_break_in_a_loop_sees_what_is_live_after_the_loop_not_after_itself() {
    let source = "## To f (a: Int, b: Int) -> Int:\n    While true:\n        Break.\n        Show b.\n    Show a.\n    Return 1.\n## Main\nShow f(1, 2).\n";
    liveness(source, |l, stmts, i| {
        let at = first(stmts, &|s| matches!(s, Stmt::Break)).expect("the Break");
        let live = l.after(at).expect("recorded");
        assert!(live.contains(&sym(i, "a")), "`Show a` follows the loop");
        assert!(!live.contains(&sym(i, "b")), "`Show b` never runs after the Break");
    });
}

#[test]
fn a_return_outside_every_zone_keeps_nothing_live() {
    let source = "## To f (a: Int) -> Int:\n    Show a.\n    Return 1.\n    Show a.\n## Main\nShow f(1).\n";
    liveness(source, |l, stmts, i| {
        let at = first(stmts, &|s| matches!(s, Stmt::Return { .. })).expect("the Return");
        assert!(!l.after(at).expect("recorded").contains(&sym(i, "a")));
    });
}

#[test]
fn a_call_to_a_function_of_the_program_keeps_no_name_live() {
    let source = "## To later (n: Int) -> Int:\n    Return n + 1.\n## Main\nLet x be 1.\nShow x.\nShow later(2).\n";
    liveness(source, |l, stmts, i| {
        let at = first(stmts, &|s| matches!(s, Stmt::Let { .. })).expect("the Let");
        let live = l.after(at).expect("recorded");
        assert!(live.contains(&sym(i, "x")), "`Show x` follows");
        assert!(!live.contains(&sym(i, "later")), "a call reaches the function `later`, never a binding: {live:?}");
    });
}

#[test]
fn a_call_through_a_closure_binding_keeps_the_binding_live() {
    let source = "## Main\nLet double be (n: Int) -> n * 2.\nShow 1.\nShow double(3).\n";
    liveness(source, |l, stmts, i| {
        let at = first(stmts, &|s| matches!(s, Stmt::Show { object: Expr::Literal(Literal::Number(1)), .. }))
            .expect("the first Show");
        assert!(
            l.after(at).expect("recorded").contains(&sym(i, "double")),
            "`double(3)` calls the closure the binding holds"
        );
    });
}
