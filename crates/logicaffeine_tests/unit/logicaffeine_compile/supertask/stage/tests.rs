use super::*;
use crate::intern::Interner;

/// The first `While` of `block` in reading order, inside the statements the engine wrote around it.
fn first_loop<'s, 'a>(block: &'s [Stmt<'a>]) -> Option<&'s Stmt<'a>> {
    block.iter().find_map(|s| match s {
        Stmt::While { .. } => Some(s),
        Stmt::Splice { body } => first_loop(body),
        Stmt::If {
            then_block,
            else_block,
            ..
        } => first_loop(then_block).or_else(|| else_block.and_then(|b| first_loop(b))),
        _ => None,
    })
}

/// The pass of `program`'s outer `While` as the engine meets it (its inner supertasks resolved), split by
/// its clock `t`.
fn split_of(program: &str) -> Option<(Vec<String>, Vec<String>, usize)> {
    let source = format!("## Main\nLet dt be an infinitesimal.\nLet t be 0.\nLet total be 0.\nWhile t < 1:\n{program}");
    crate::ui_bridge::with_parsed_program_checked(&source, |parsed, interner: &Interner| {
        let (stmts, _, _) = parsed.expect("the program parses");
        let Some(Stmt::While { body, .. }) = first_loop(stmts) else {
            panic!("no loop");
        };
        let t = interner.lookup("t").expect("the clock");
        let step_at = body
            .iter()
            .position(|s| matches!(s, Stmt::Set { target, .. } if *target == t))
            .expect("the clock's step");
        let name = |s: &Stmt<'_>| match s {
            Stmt::Let { var, .. } => format!("let {}", interner.resolve(*var)),
            Stmt::Set { target, .. } => format!("set {}", interner.resolve(*target)),
            Stmt::While { .. } => "while".to_string(),
            other => format!("{other:?}").chars().take(12).collect(),
        };
        Ok(split(body, step_at, interner).map(|sp| {
            (sp.ahead.iter().map(name).collect(), sp.rest.iter().map(name).collect(), sp.kept_at)
        }))
    })
    .expect("the program parses")
}

#[test]
fn a_pass_s_own_work_runs_ahead_and_the_carried_state_stays() {
    let (ahead, rest, kept_at) = split_of(
        "    Let s be 0.\n    Let n be 0.\n    While s < 1:\n        Set n to n + 1.\n        Set s to s + dt.\n    Set total to total + n.\n    Set t to t + dt.\nShow total.\n",
    )
    .expect("the pass splits");
    assert_eq!(ahead[..2], ["let s", "let n"]);
    assert_eq!(ahead.len(), 3, "the inner supertask, resolved, runs ahead with its locals: {ahead:?}");
    assert_eq!(rest, ["set total", "set t"]);
    assert_eq!(kept_at, 1, "the clock's step keeps its place among the rest");
}

#[test]
fn work_that_reads_a_binding_left_in_the_pass_stays_after_it() {
    assert!(split_of(
        "    Let u be t * t.\n    Let s be 0.\n    Let n be 0.\n    While s < 1:\n        Set n to n + u.\n        Set s to s + dt.\n    Set total to total + n.\n    Set t to t + dt.\nShow total.\n"
    )
    .is_none());
    let (ahead, rest, _) = split_of("    Let u be t * t.\n    Let k be 3.\n    Set total to total + u + k.\n    Set t to t + dt.\nShow total.\n")
        .expect("the pass splits");
    assert_eq!(ahead, ["let k"]);
    assert_eq!(rest, ["let u", "set total", "set t"]);
}

#[test]
fn work_that_reads_the_carried_state_or_the_clock_stays_in_the_pass() {
    assert!(split_of("    Let s be t.\n    Set total to total + s.\n    Set t to t + dt.\nShow total.\n").is_none());
    assert!(split_of("    Let s be total.\n    Set total to s + 1.\n    Set t to t + dt.\nShow total.\n").is_none());
}

#[test]
fn a_local_the_rest_of_the_pass_changes_stays_with_it() {
    assert!(
        split_of("    Let n be 0.\n    Set total to total + n.\n    Set n to n + 1.\n    Set t to t + dt.\nShow total.\n").is_none(),
        "n starts afresh at every pass only while it is bound there"
    );
}

#[test]
fn a_name_read_before_the_pass_binds_it_is_not_bound_ahead() {
    assert!(split_of("    Set total to total + n.\n    Let n be 5.\n    Set t to t + dt.\nShow total.\n").is_none());
}

#[test]
fn work_with_an_effect_or_a_call_of_the_program_s_own_stays_in_the_pass() {
    assert!(split_of("    Let s be 2.\n    Show s.\n    Set total to total + s.\n    Set t to t + dt.\nShow total.\n")
        .is_some_and(|(ahead, _, _)| ahead == ["let s"]));
    assert!(split_of("    Let s be abs(3).\n    Set total to total + s.\n    Set t to t + dt.\nShow total.\n").is_none());
}
