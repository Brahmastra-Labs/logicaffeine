//! The scans every promotion decision is taken from, on parsed programs: what one scope assigns,
//! builds, returns and hands to a call — wherever in the scope it is written, and never what a
//! function defined in the scope does, which is that function's own scope.
use super::*;
use crate::intern::Interner;

fn with_program<R>(src: &str, f: impl FnOnce(&[Stmt], &Interner) -> R) -> R {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("the program parses");
        f(stmts, interner)
    })
}

fn body_of<'s, 'a>(stmts: &'s [Stmt<'a>], name: &str, interner: &Interner) -> &'s [Stmt<'a>] {
    stmts
        .iter()
        .find_map(|s| {
            let Stmt::FunctionDef { name: n, body, .. } = s else {
                return None;
            };
            (interner.resolve(*n) == name).then_some(&body[..])
        })
        .expect("the function is defined")
}

fn spelled(names: impl IntoIterator<Item = Symbol>, interner: &Interner) -> Vec<String> {
    let mut out: Vec<String> = names
        .into_iter()
        .map(|s| interner.resolve(s).to_string())
        .collect();
    out.sort();
    out
}

#[test]
fn a_scope_assigns_in_its_blocks_and_closures_and_not_in_its_functions() {
    let src = "## To f (p: Int) -> Int:\n    Let inner be p.\n    Return inner.\n\n\
               ## Main\nLet a be 1.\nIf a > 0:\n    Let b be 2.\nLet g be (n: Int) ->:\n    Let c be n.\n    Return c.\n\
               Repeat for i from 1 to 2:\n    Set a to a + i.\nShow f(a).\n";
    with_program(src, |stmts, it| {
        let mut assigns = Vec::new();
        collect_assigns(stmts, &mut assigns);
        assert_eq!(
            spelled(assigns.iter().map(|(n, _)| *n), it),
            ["a", "a", "b", "c", "g"]
        );
        let mut inside = Vec::new();
        collect_assigns(body_of(stmts, "f", it), &mut inside);
        assert_eq!(spelled(inside.iter().map(|(n, _)| *n), it), ["inner"]);
    });
}

#[test]
fn a_return_in_a_closure_answers_the_closure() {
    let src = "## To f (xs: Seq of Int) -> Int:\n    Let g be (n: Int) ->:\n        Return n * 2.\n\
               \x20   If length of xs > 0:\n        Return 1.\n    Return 0.\n\n## Main\nShow f([1]).\n";
    with_program(src, |stmts, it| {
        let mut returned = Vec::new();
        for_each_return(body_of(stmts, "f", it), |e| returned.push(format!("{e:?}")));
        assert_eq!(
            returned.len(),
            2,
            "the function's own two returns, not the closure's: {returned:?}"
        );
        assert!(
            returned.iter().all(|r| r.contains("Literal")),
            "{returned:?}"
        );
    });
}

#[test]
fn a_construction_is_found_wherever_it_is_written() {
    let src = "## A Cell has:\n    a v, which is Int.\n\n\
               ## To keep (c: Cell) -> Int:\n    Return c's v.\n\n\
               ## Main\nShow keep(a new Cell with v 1).\nLet mutable cs be a new Seq of Cell.\n\
               Push a new Cell with v 2 to cs.\nShow \"{keep(a new Cell with v 3)}\".\nLet c be a new Cell with v 4.\n";
    with_program(src, |stmts, it| {
        let mut built = Vec::new();
        for_each_new(stmts, |ty, fields| {
            for (field, value) in fields {
                if let Expr::Literal(Literal::Number(n)) = value {
                    built.push((
                        it.resolve(ty).to_string(),
                        it.resolve(*field).to_string(),
                        *n,
                    ));
                }
            }
        });
        built.sort();
        let cell = |n| ("Cell".to_string(), "v".to_string(), n);
        assert_eq!(built, [cell(1), cell(2), cell(3), cell(4)]);
    });
}

#[test]
fn a_call_hands_its_arguments_over_wherever_it_is_written() {
    let src = "## To f (a: Int) and (b: Int) -> Int:\n    Return a + b.\n\n\
               ## Main\nLet v be 1.\nShow f(v, 2).\nShow \"{f(3, v)}\".\nLet w be f(f(v, 4), 5).\nf(6, v).\n";
    with_program(src, |stmts, it| {
        let v = it.lookup("v").expect("v is a name");
        let mut handed = Vec::new();
        collect_bigint_arg_calls(stmts, &HashSet::from([v]), &mut handed);
        let mut at: Vec<usize> = handed.iter().map(|(_, at)| *at).collect();
        at.sort();
        assert_eq!(at, [0, 0, 1, 1], "{handed:?}");
    });
}
