//! LITERATE CLAUSE LAYOUT — a clause body may begin on the line after its `When …:`, and a nested
//! equation set written beneath a clause ends where the set around it resumes.
//!
//! `When Succ k:` followed by a `Consider` on its own, deeper-indented line is the ordinary way to
//! write a two-level equation set, and the Studio's own examples (`collatz.logos`,
//! `termination-wall.logos`) are written that way. The document door refused it with
//! "Expected identifier, found \n". The layout rule that comes with it is Lean's offside rule: a
//! `When` that begins a line to the LEFT of the first clause of the innermost open `Consider`
//! belongs to an enclosing one — otherwise the nested set would swallow the outer function's
//! remaining cases and compute a different function.

use logicaffeine_kernel::interface::Repl;

const DECISION: &str = "A Decision is either Yes or No.

## To negate (d: Decision) -> Decision:
    Consider d:
        When Yes: Yield No.
        When No: Yield Yes.

## To is_odd (n: Nat) -> Decision:
    Consider n:
        When Zero: Yield No.
        When Succ k: Yield negate(is_odd(k)).
";

fn session(document: &str) -> Repl {
    let mut repl = Repl::new();
    repl.execute_document(document)
        .unwrap_or_else(|e| panic!("the document must be accepted: {e}\n{document}"));
    repl
}

fn eval(repl: &mut Repl, term: &str) -> String {
    repl.execute(&format!("Eval {term}."))
        .unwrap_or_else(|e| panic!("`Eval {term}.` must be accepted: {e}"))
}

fn nat(n: usize) -> String {
    (0..n).fold("Zero".to_string(), |acc, _| format!("(Succ {acc})"))
}

#[test]
fn a_clause_body_may_begin_on_the_next_line() {
    let mut repl = session(&format!(
        "{DECISION}
## To halve (n: Nat) -> Nat:
    Consider n:
        When Zero: Yield Zero.
        When Succ k:
            Consider is_odd(k):
                When Yes: Yield Succ (halve(k)).
                When No: Yield halve(k).
"
    ));
    for (n, half) in [(0, 0), (1, 0), (2, 1), (3, 1), (4, 2), (7, 3)] {
        let expected = if half == 0 {
            "Zero".to_string()
        } else {
            nat(half)
        };
        assert_eq!(
            eval(&mut repl, &format!("halve {}", nat(n))),
            expected,
            "halve {n}"
        );
    }
}

/// The nested set sits in the FIRST clause, so the `When Succ k:` beneath it is the outer
/// function's second case and not a third case of the inner match over a `Decision`.
#[test]
fn a_nested_equation_set_ends_where_the_enclosing_one_resumes() {
    let mut repl = session(&format!(
        "{DECISION}
## To pick (d: Decision) and (n: Nat) -> Nat:
    Consider n:
        When Zero:
            Consider d:
                When Yes: Yield Succ Zero.
                When No: Yield Zero.
        When Succ k: Yield Succ (Succ k).
"
    ));
    assert_eq!(eval(&mut repl, "pick Yes Zero"), nat(1));
    assert_eq!(eval(&mut repl, "pick No Zero"), "Zero");
    assert_eq!(eval(&mut repl, &format!("pick No {}", nat(1))), nat(2));
    assert_eq!(eval(&mut repl, &format!("pick Yes {}", nat(3))), nat(4));
}

/// The nested `Consider` may also follow its clause's colon on the same line, with its own clauses
/// beneath: the first of them begins a line, so it is that column the outer clause returns to the
/// left of.
#[test]
fn a_nested_set_opened_on_the_clause_line_ends_at_the_enclosing_column() {
    let mut repl = session(&format!(
        "{DECISION}
## To pick2 (d: Decision) and (n: Nat) -> Nat:
    Consider n:
        When Zero: Consider d:
            When Yes: Yield Succ Zero.
            When No: Yield Zero.
        When Succ k: Yield k.
"
    ));
    assert_eq!(eval(&mut repl, "pick2 Yes Zero"), nat(1));
    assert_eq!(eval(&mut repl, "pick2 No Zero"), "Zero");
    assert_eq!(eval(&mut repl, &format!("pick2 Yes {}", nat(3))), nat(2));
}

/// Every layout the door read before reads as it did: clauses on the `Consider` line, a nested set
/// written entirely on its clause's line, and clauses at the `Consider`'s own column.
#[test]
fn the_layouts_that_already_read_still_read_the_same() {
    let mut repl = session(&format!(
        "{DECISION}
## To halve_inline (n: Nat) -> Nat:
    Consider n:
        When Zero: Yield Zero.
        When Succ k: Consider is_odd(k): When Yes: Yield Succ (halve_inline(k)). When No: Yield halve_inline(k).

## To flat (n: Nat) -> Nat:
    Consider n: When Zero: Yield Succ Zero.
        When Succ k: Yield Zero.

## To level (n: Nat) -> Nat:
    Consider n:
    When Zero: Yield Succ Zero.
    When Succ k: Yield k.
"
    ));
    assert_eq!(eval(&mut repl, &format!("halve_inline {}", nat(5))), nat(2));
    assert_eq!(eval(&mut repl, "flat Zero"), nat(1));
    assert_eq!(eval(&mut repl, &format!("flat {}", nat(2))), "Zero");
    assert_eq!(eval(&mut repl, "level Zero"), nat(1));
    assert_eq!(eval(&mut repl, &format!("level {}", nat(2))), nat(1));
}

/// A clause with no body before the next `When` is refused as exactly that.
#[test]
fn a_clause_with_no_body_is_refused_as_one() {
    let mut repl = Repl::new();
    let refused = repl
        .execute_document(
            "## To broken (n: Nat) -> Nat:
    Consider n:
        When Zero:
        When Succ k: Yield k.
",
        )
        .expect_err("a clause with no body");
    let message = refused.to_string();
    assert!(
        message.contains("clause body"),
        "the refusal names what is missing: {message}"
    );
    assert!(
        repl.context().get_definition_body("broken").is_none(),
        "nothing was registered"
    );
}

/// A document is split into blocks before any of them runs, and each block is read for its meaning
/// when it runs — against the context the blocks before it built. An equation set over a function an
/// EARLIER block declares cannot be compiled when the document is split, and does not need to be.
#[test]
fn a_document_block_is_compiled_against_the_blocks_before_it() {
    let mut repl = session(&format!(
        "{DECISION}
## To parity_nat (n: Nat) -> Nat:
    Consider is_odd(n): When Yes: Yield Succ Zero. When No: Yield Zero.
"
    ));
    assert_eq!(eval(&mut repl, &format!("parity_nat {}", nat(3))), nat(1));
    assert_eq!(eval(&mut repl, &format!("parity_nat {}", nat(4))), "Zero");
}
