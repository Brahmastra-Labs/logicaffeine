//! A literate `Consider` read as it is LAID OUT.
//!
//! The Studio flattens a `## To` block onto one line before it reaches the kernel; the document
//! door reads the file as written. Two layout facts the flattened line cannot show, and the
//! document door must:
//!
//! * a clause's body may begin on the line after its `When …:` — the shape a nested `Consider`
//!   takes whenever it is not squeezed onto its clause's line;
//! * a `When` belongs to the innermost `Consider` whose clauses it is indented with. Lean reads a
//!   nested `match` the same way (`matchAlts` is `many1Indent`: an alternative that starts left of
//!   the first one ends the inner match). Without it a nested `Consider` swallows every clause
//!   after it, and an outer clause written after the nested one lands in the inner match.
//!
//! On one line there is no layout, and a nested `Consider` keeps reading every `When` that
//! follows it — so the flattened form is unchanged.

use logicaffeine_kernel::interface::Repl;

fn document(text: &str) -> Repl {
    let mut repl = Repl::new();
    repl.execute_document(text)
        .unwrap_or_else(|e| panic!("the document must be accepted: {e}\n{text}"));
    repl
}

fn eval(repl: &mut Repl, term: &str) -> String {
    repl.execute(&format!("Eval {term}."))
        .unwrap_or_else(|e| panic!("`Eval {term}`: {e}"))
}

#[test]
fn a_clause_body_may_begin_on_the_next_line() {
    let mut repl = document(
        "## To pred (n: Nat) -> Nat:
    Consider n:
        When Zero:
            Yield Zero.
        When Succ k:
            Yield k.
",
    );
    assert_eq!(eval(&mut repl, "pred (Succ (Succ Zero))"), "(Succ Zero)");
    assert_eq!(eval(&mut repl, "pred Zero"), "Zero");
}

#[test]
fn a_nested_consider_on_its_own_lines_is_read_as_written() {
    // The Studio's `halve`, laid out as the Studio file lays it out.
    let mut repl = document(
        "A Decision is either Yes or No.

## To negate (d: Decision) -> Decision:
    Consider d:
        When Yes: Yield No.
        When No: Yield Yes.

## To check_parity (n: Nat) -> Decision:
    Consider n:
        When Zero: Yield Yes.
        When Succ k: Yield negate(check_parity(k)).

## To is_odd (n: Nat) -> Decision:
    Yield negate(check_parity(n)).

## To halve (n: Nat) -> Nat:
    Consider n:
        When Zero: Yield Zero.
        When Succ k:
            Consider is_odd(k):
                When Yes: Yield Succ (halve(k)).
                When No: Yield halve(k).
",
    );
    assert_eq!(
        eval(&mut repl, "halve (Succ (Succ (Succ (Succ Zero))))"),
        "(Succ (Succ Zero))"
    );
    assert_eq!(
        eval(&mut repl, "halve (Succ (Succ (Succ Zero)))"),
        "(Succ Zero)"
    );
}

#[test]
fn an_outer_clause_after_a_nested_consider_belongs_to_the_outer_one() {
    let mut repl = document(
        "## To pick (a: Nat) and (b: Nat) -> Nat:
    Consider a:
        When Succ k:
            Consider b:
                When Zero: Yield Succ (Succ Zero).
                When Succ j: Yield Succ Zero.
        When Zero: Yield Zero.
",
    );
    assert_eq!(eval(&mut repl, "pick Zero (Succ Zero)"), "Zero");
    assert_eq!(
        eval(&mut repl, "pick (Succ Zero) Zero"),
        "(Succ (Succ Zero))"
    );
    assert_eq!(
        eval(&mut repl, "pick (Succ Zero) (Succ Zero)"),
        "(Succ Zero)"
    );
}

#[test]
fn a_clause_written_beside_its_consider_sets_the_indentation_its_siblings_continue() {
    // The first clause sits on the `Consider` line; the rest continue beneath it, indented past
    // that line. They are one match.
    let mut repl = document(
        "## To pred (n: Nat) -> Nat:
    Consider n: When Zero: Yield Zero.
        When Succ k: Yield k.
",
    );
    assert_eq!(eval(&mut repl, "pred (Succ Zero)"), "Zero");

    // Nested, and written the same way: the inner match's clauses are the ones indented past its
    // line; the outer clause at the outer indentation is the outer match's.
    let mut repl = document(
        "## To pick (a: Nat) and (b: Nat) -> Nat:
    Consider a:
        When Succ k: Consider b: When Zero: Yield Succ (Succ Zero).
            When Succ j: Yield Succ Zero.
        When Zero: Yield Zero.
",
    );
    assert_eq!(eval(&mut repl, "pick Zero Zero"), "Zero");
    assert_eq!(
        eval(&mut repl, "pick (Succ Zero) Zero"),
        "(Succ (Succ Zero))"
    );
    assert_eq!(
        eval(&mut repl, "pick (Succ Zero) (Succ Zero)"),
        "(Succ Zero)"
    );
}

#[test]
fn the_flattened_one_line_form_reads_as_before() {
    // What the Studio's splitter hands the REPL for `halve`: one line, the nested `Consider` last.
    let mut repl = Repl::new();
    for line in [
        "A Decision is either Yes or No.",
        "## To negate (d: Decision) -> Decision: Consider d: When Yes: Yield No. When No: Yield Yes.",
        "## To check_parity (n: Nat) -> Decision: Consider n: When Zero: Yield Yes. When Succ k: Yield negate(check_parity(k)).",
        "## To is_odd (n: Nat) -> Decision: Yield negate(check_parity(n)).",
        "## To halve (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. When Succ k: Consider is_odd(k): When Yes: Yield Succ (halve(k)). When No: Yield halve(k).",
    ] {
        repl.execute(line).unwrap_or_else(|e| panic!("`{line}`: {e}"));
    }
    assert_eq!(
        eval(&mut repl, "halve (Succ (Succ (Succ (Succ Zero))))"),
        "(Succ (Succ Zero))"
    );
}

#[test]
fn a_clause_left_of_every_open_consider_ends_the_definition() {
    // `When Succ k` starts left of `Consider n`'s clauses: it is not one of them, so the match is
    // missing `Succ` and the definition is refused — never silently completed by a stray line.
    let mut repl = Repl::new();
    let refused = repl.execute_document(
        "## To f (n: Nat) -> Nat:
    Consider n:
        When Zero: Yield Zero.
  When Succ k: Yield k.
",
    );
    assert!(refused.is_err(), "{refused:?}");
    assert!(repl.execute("Check f.").is_err(), "nothing was registered");
}
