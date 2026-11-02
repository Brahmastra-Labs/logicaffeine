use super::*;
use crate::arena::Arena;
use crate::ast::stmt::Expr;

/// The first `While` of `block` in reading order, inside the statements the engine wrote around it.
fn first_loop<'s, 'a>(block: &'s [Stmt<'a>]) -> Option<(&'a Expr<'a>, &'a [Stmt<'a>])> {
    block.iter().find_map(|s| match s {
        Stmt::While { cond, body, .. } => Some((*cond, *body)),
        Stmt::Splice { body } => first_loop(body),
        Stmt::If {
            then_block,
            else_block,
            ..
        } => first_loop(then_block).or_else(|| else_block.and_then(|b| first_loop(b))),
        _ => None,
    })
}

/// One statement as `let x = …` / `set x = …` / `if …` with the names it reads, so a renaming shows
/// in where each name goes.
fn shape(s: &Stmt<'_>, interner: &Interner) -> String {
    fn reads(e: &Expr<'_>, interner: &Interner, out: &mut Vec<String>) {
        match e {
            Expr::Identifier(s) => out.push(interner.resolve(*s).to_string()),
            Expr::BinaryOp { left, right, .. } => {
                reads(left, interner, out);
                reads(right, interner, out);
            }
            Expr::List(items) => items.iter().for_each(|i| reads(i, interner, out)),
            _ => {}
        }
    }
    let read = |e: &Expr<'_>| {
        let mut out = Vec::new();
        reads(e, interner, &mut out);
        out.join(" ")
    };
    match s {
        Stmt::Let { var, value, .. } => format!("let {} = {}", interner.resolve(*var), read(value)),
        Stmt::Set { target, value } => format!("set {} = {}", interner.resolve(*target), read(value)),
        Stmt::If { cond, then_block, .. } => format!(
            "if {}: {}",
            read(cond),
            then_block.iter().map(|s| shape(s, interner)).collect::<Vec<_>>().join("; ")
        ),
        other => format!("{other:?}").chars().take(16).collect(),
    }
}

/// The pass of the program's first loop, `While t < 1:` with `pass` as its body, through
/// [`distinct`]: the body's shapes and the names renamed, `None` when nothing needed renaming, and
/// the reason when the pass is refused.
fn distinct_of(pass: &str) -> Result<Option<(Vec<String>, Vec<String>)>, String> {
    let source = format!(
        "## Main\nLet dt be an infinitesimal.\nLet t be 0.\nLet total be 0.\nLet acc be 0.\nLet x be 0.\nLet y be 0.\nWhile t < 1:\n{pass}Show total.\n"
    );
    crate::ui_bridge::with_parsed_program_checked(&source, |parsed, interner: &Interner| {
        let (stmts, _, _) = parsed.expect("the program parses");
        let (cond, body) = first_loop(stmts).expect("the loop as written");
        let mut interner = interner.clone();
        let (exprs, stmt_arena, types) = (Arena::new(), Arena::new(), Arena::new());
        let mut b = Build {
            arenas: Arenas {
                exprs: &exprs,
                stmts: &stmt_arena,
                types: &types,
            },
            interner: &mut interner,
            serial: 1,
        };
        let out = distinct(&[cond], body, &mut b);
        Ok(out.map(|o| o.map(|(body, names)| (body.iter().map(|s| shape(s, b.interner)).collect(), names))))
    })
    .expect("the program parses")
}

#[test]
fn a_pass_that_binds_nothing_it_used_comes_back_as_it_was() {
    let out = distinct_of("    Let s be 2.\n    Set total to total + s.\n    Set t to t + dt.\n");
    assert_eq!(out, Ok(None));
}

#[test]
fn a_name_bound_after_the_pass_changed_it_is_renamed_from_its_binding_on() {
    let (body, names) = distinct_of(
        "    Set total to total + 1.\n    Let total be 7.\n    Set total to total + 2.\n    Set t to t + dt.\n",
    )
    .unwrap()
    .expect("a binding to rename");
    assert_eq!(names, ["total"]);
    assert_eq!(
        body,
        [
            "set total = total",
            "let __supertask1_pass_total = ",
            "set __supertask1_pass_total = __supertask1_pass_total",
            "set t = t dt",
        ]
    );
}

#[test]
fn a_binding_s_own_value_reads_the_name_it_shadows() {
    let (body, names) = distinct_of("    Let total be total + 1.\n    Set acc to acc + total.\n    Set t to t + dt.\n")
        .unwrap()
        .expect("a binding to rename");
    assert_eq!(names, ["total"]);
    assert_eq!(
        body,
        [
            "let __supertask1_pass_total = total",
            "set acc = acc __supertask1_pass_total",
            "set t = t dt",
        ]
    );
}

#[test]
fn the_guard_s_names_count_as_used() {
    let (body, names) = distinct_of("    Let t be 0.\n    Set t to t + dt.\n")
        .unwrap()
        .expect("the clock bound again");
    assert_eq!(names, ["t"]);
    assert_eq!(body, ["let __supertask1_pass_t = ", "set __supertask1_pass_t = __supertask1_pass_t dt"]);
}

#[test]
fn a_name_bound_twice_is_renamed_twice_and_each_binding_reads_the_one_before() {
    let (body, names) = distinct_of(
        "    Set x to x + 1.\n    Let x be 2.\n    Let x be x + 3.\n    Set y to x.\n    Set t to t + dt.\n",
    )
    .unwrap()
    .expect("bindings to rename");
    assert_eq!(names, ["x", "__supertask1_pass_x"]);
    assert_eq!(
        body,
        [
            "set x = x",
            "let __supertask1_pass_x = ",
            "let __supertask1_pass___supertask1_pass_x = __supertask1_pass_x",
            "set y = __supertask1_pass___supertask1_pass_x",
            "set t = t dt",
        ]
    );
}

#[test]
fn a_binding_reaches_into_the_blocks_after_it() {
    let (body, _) = distinct_of("    Set x to 1.\n    Let x be 2.\n    If x > 1:\n        Set y to x.\n    Set t to t + dt.\n")
        .unwrap()
        .expect("a binding to rename");
    assert_eq!(body[2], "if __supertask1_pass_x: set y = __supertask1_pass_x");
}

#[test]
fn a_form_the_renaming_does_not_reach_refuses_the_pass_with_the_reason() {
    let out = distinct_of(
        "    Set x to x + 1.\n    Let x be 0.\n    Repeat for x in [1, 2]:\n        Set y to y + x.\n    Set t to t + dt.\n",
    );
    assert_eq!(out, Err("its pass binds `x` again where the engine cannot keep the two apart".to_string()));
}
