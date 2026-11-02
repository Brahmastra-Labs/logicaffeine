//! The questions every pass that moves a body or carries what it knows of a name asks, on parsed programs: a
//! callee's free names against a call site's frame, a block's own names against the statements a splice would
//! put them in front of, and the names a statement binds where it stands or a nested block binds on entry.
use super::*;
use crate::intern::Interner;

/// Names, spelled, so a row reads as the program it states.
fn spelled(names: &HashSet<Symbol>, interner: &Interner) -> Vec<String> {
    let mut out: Vec<String> = names
        .iter()
        .map(|s| interner.resolve(*s).to_string())
        .collect();
    out.sort();
    out
}

/// Run `f` on the parsed program's function `name` (its parameters and body) and Main.
fn with_program<R>(src: &str, f: impl FnOnce(&[Stmt], &Interner) -> R) -> R {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("the program parses");
        f(stmts, interner)
    })
}

fn function<'s, 'a>(
    stmts: &'s [Stmt<'a>],
    name: &str,
    interner: &Interner,
) -> (Vec<Symbol>, &'s [Stmt<'a>]) {
    stmts
        .iter()
        .find_map(|s| match s {
            Stmt::FunctionDef {
                name: n,
                params,
                body,
                ..
            } if interner.resolve(*n) == name => {
                Some((params.iter().map(|(p, _)| *p).collect(), &body[..]))
            }
            _ => None,
        })
        .expect("the function is defined")
}

fn main_of<'s, 'a>(stmts: &'s [Stmt<'a>]) -> Vec<Stmt<'a>> {
    stmts
        .iter()
        .filter(|s| !matches!(s, Stmt::FunctionDef { .. }))
        .cloned()
        .collect()
}

#[test]
fn a_callees_free_names_are_what_it_reads_that_it_does_not_bind() {
    let src = "## To f (p: Int) -> Int:\n    Let q be p + x.\n    If q is greater than 0:\n        Let r be y.\n        \
               Return r + q.\n    Set z to 1.\n    Return q.\n\n## Main\nLet x be 1.\nLet y be 2.\nLet mutable z be 0.\nShow f(1).\n";
    with_program(src, |stmts, it| {
        let (params, body) = function(stmts, "f", it);
        assert_eq!(spelled(&free_names(&params, body), it), ["x", "y", "z"]);
    });
}

#[test]
fn a_name_a_nested_block_declares_is_free_after_the_block() {
    let src = "## To f -> Int:\n    If true:\n        Let x be 1.\n        Let y be x.\n    Return x.\n\n\
               ## Main\nLet x be 2.\nShow f().\n";
    with_program(src, |stmts, it| {
        let (params, body) = function(stmts, "f", it);
        assert_eq!(spelled(&free_names(&params, body), it), ["x"]);
    });
}

#[test]
fn a_name_read_before_the_body_declares_it_is_free() {
    let src = "## To f -> Int:\n    Let y be x.\n    Let x be 1.\n    Return x.\n\n## Main\nLet x be 2.\nShow f().\n";
    with_program(src, |stmts, it| {
        let (params, body) = function(stmts, "f", it);
        assert_eq!(spelled(&free_names(&params, body), it), ["x"]);
    });
}

#[test]
fn a_name_declared_before_every_read_of_it_is_not_free() {
    let src = "## To f -> Int:\n    Let x be 1.\n    If true:\n        Let y be x.\n    Return x.\n\n\
               ## Main\nLet x be 2.\nShow f().\n";
    with_program(src, |stmts, it| {
        let (params, body) = function(stmts, "f", it);
        assert!(free_names(&params, body).is_empty());
    });
}

#[test]
fn a_functions_frame_is_its_parameters_and_everything_it_declares() {
    let src = "## To f (p: Int) -> Int:\n    Let q be p.\n    Repeat for i from 1 to 3:\n        Let r be i.\n    \
               Let g be (c: Int) -> c + 1.\n    Return q.\n\n## Main\nShow f(1).\n";
    with_program(src, |stmts, it| {
        let (params, body) = function(stmts, "f", it);
        assert_eq!(
            spelled(&frame_names(&params, body), it),
            ["c", "g", "i", "p", "q", "r"]
        );
    });
}

#[test]
fn mains_frame_is_what_its_blocks_bind_never_its_root() {
    let src = "## Main\nLet x be 1.\nIf x is greater than 0:\n    Let inner be 5.\nRepeat for i from 1 to 2:\n    Show i.\n\
               Let f be (c: Int) -> c.\n";
    with_program(src, |stmts, it| {
        let main = main_of(stmts);
        assert_eq!(spelled(&main_frame_names(&main), it), ["c", "i", "inner"]);
    });
}

#[test]
fn a_blocks_own_names_are_what_it_declares_at_its_own_level() {
    let src = "## Main\nIf true:\n    Let a be 1.\n    If a is greater than 0:\n        Let b be 2.\n    Let c be a.\n";
    with_program(src, |stmts, it| {
        let main = main_of(stmts);
        let Stmt::If { then_block, .. } = &main[0] else {
            panic!("the program opens with an If")
        };
        assert_eq!(spelled(&own_names(then_block), it), ["a", "c"]);
    });
}

#[test]
fn a_block_splices_before_statements_that_never_read_its_names() {
    let src = "## Main\nLet x be 1.\nIf true:\n    Let t be 2.\n    Show t.\nShow x.\nIf true:\n    Let x be 3.\n    Show x.\nShow x.\n";
    with_program(src, |stmts, _| {
        let main = main_of(stmts);
        let (
            Stmt::If {
                then_block: temp, ..
            },
            Stmt::If {
                then_block: shadow, ..
            },
        ) = (&main[1], &main[3])
        else {
            panic!("the program's two branches")
        };
        assert!(
            splices_before(temp, &main[2..3]),
            "`t` is read nowhere after its block"
        );
        assert!(
            !splices_before(shadow, &main[4..]),
            "`x` is read after the block that shadows it"
        );
    });
}

#[test]
fn a_free_name_bound_in_the_frame_is_captured_and_one_that_is_not_is_free() {
    let src = "## To peek -> Int:\n    Return x.\n\n## To outer -> Int:\n    Let x be 7.\n    Return peek().\n\n\
               ## To other -> Int:\n    Let y be 7.\n    Return peek().\n\n## Main\nLet x be 1.\nShow outer().\n";
    with_program(src, |stmts, it| {
        let (pp, pb) = function(stmts, "peek", it);
        let free = free_names(&pp, pb);
        let (op, ob) = function(stmts, "outer", it);
        let (yp, yb) = function(stmts, "other", it);
        assert!(
            captures(&free, &frame_names(&op, ob)),
            "outer's `x` would capture peek's"
        );
        assert!(
            !captures(&free, &frame_names(&yp, yb)),
            "other binds no `x`"
        );
        assert!(
            !captures(&free, &main_frame_names(&main_of(stmts))),
            "Main's root `x` is the program's own"
        );
    });
}

#[test]
fn a_statements_own_names_are_the_ones_it_binds_where_it_stands() {
    let src = "## Main\nLet ch be a Pipe of Int.\nSend 1 into ch.\nReceive x from ch.\nRepeat for i from 1 to 2:\n    \
               Let inner be i.\nIf x is greater than 0:\n    Let b be 2.\n";
    with_program(src, |stmts, it| {
        let main = main_of(stmts);
        let own = |at: usize| spelled(&own_names(std::slice::from_ref(&main[at])), it);
        assert_eq!(own(0), ["ch"], "a pipe's `Let`");
        assert!(own(1).is_empty(), "a send binds nothing");
        assert_eq!(own(2), ["x"], "a receive binds where it stands");
        assert!(
            own(3).is_empty(),
            "a loop's pattern and body bind inside the loop"
        );
        assert!(own(4).is_empty(), "a branch binds inside the branch");
    });
}

#[test]
fn a_nested_block_binds_its_loop_pattern_or_arm_bindings_on_entry() {
    let src = "## A Shape is either:\n    a Circle with radius Int.\n    a Square with side Int.\n\n\
               ## Main\nLet s be a new Circle with radius 9.\nRepeat for i from 1 to 2:\n    Show i.\n\
               Inspect s:\n    When Circle (radius):\n        Show radius.\n    When Square (side):\n        Show side.\n\
               If true:\n    Let t be 1.\n";
    with_program(src, |stmts, it| {
        let main = main_of(stmts);
        let names = |syms: &[Symbol]| {
            syms.iter()
                .map(|s| it.resolve(*s).to_string())
                .collect::<Vec<_>>()
        };
        let Stmt::Repeat { body, .. } = &main[1] else {
            panic!("the loop")
        };
        assert_eq!(names(EntryBinders::of(&main[1]).of_block(body)), ["i"]);
        let Stmt::Inspect { arms, .. } = &main[2] else {
            panic!("the match")
        };
        let entry = EntryBinders::of(&main[2]);
        assert_eq!(names(entry.of_block(arms[0].body)), ["radius"]);
        assert_eq!(names(entry.of_block(arms[1].body)), ["side"]);
        let Stmt::If { then_block, .. } = &main[3] else {
            panic!("the branch")
        };
        assert!(
            EntryBinders::of(&main[3]).of_block(then_block).is_empty(),
            "a branch binds nothing on entry"
        );
    });
}
