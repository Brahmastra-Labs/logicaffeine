//! Lib-unit coverage for the DIMACS parser/printer — parse reads the header + clauses, print
//! round-trips, and parsing is fail-closed on a bad header / out-of-range variable.
use super::*;

#[test]
fn parse_reads_header_and_clauses() {
    let cnf = parse("p cnf 3 2\n1 -2 0\n2 3 0\n").expect("valid dimacs");
    assert_eq!(cnf.num_vars, 3);
    assert_eq!(cnf.clauses.len(), 2);
    assert_eq!(cnf.clauses[0].len(), 2, "first clause has two literals");
    assert_eq!(cnf.clauses[1].len(), 2);
    // First clause `1 -2` = one positive, one negative literal.
    assert_eq!(cnf.clauses[0].iter().filter(|l| l.is_positive()).count(), 1);
    assert_eq!(
        cnf.clauses[0].iter().filter(|l| !l.is_positive()).count(),
        1
    );
}

#[test]
fn print_then_parse_round_trips_exactly() {
    let cnf = parse("p cnf 3 2\n1 -2 0\n2 3 0\n").unwrap();
    let reparsed = parse(&print(&cnf)).expect("printed output re-parses");
    assert_eq!(reparsed.num_vars, cnf.num_vars);
    assert_eq!(reparsed.clauses, cnf.clauses);
}

#[test]
fn parse_is_fail_closed_on_bad_header_and_out_of_range_var() {
    assert!(
        parse("this is not a dimacs file").is_err(),
        "garbled header rejected"
    );
    assert!(
        parse("p cnf 2 1\n1 5 0\n").is_err(),
        "var 5 exceeds declared num_vars 2"
    );
}

#[test]
fn parse_skips_comment_and_blank_lines() {
    // `c …` comment lines and blank lines are ignored, before AND after the header.
    let cnf = parse("c leading comment\n\np cnf 2 1\nc interior comment\n1 -2 0\n")
        .expect("comments and blanks are skipped");
    assert_eq!(cnf.num_vars, 2);
    assert_eq!(cnf.clauses.len(), 1, "only the real clause survives");
}

#[test]
fn parse_rejects_a_repeated_header() {
    // A second `p …` line is malformed — the formula has exactly one header.
    assert_eq!(
        parse("p cnf 2 1\np cnf 3 1\n1 0\n"),
        Err(DimacsError::MalformedHeader("p cnf 3 1".to_string())),
    );
}

#[test]
fn parse_rejects_a_header_that_is_not_p_cnf_or_has_a_trailing_token() {
    // The first token after `p` must be exactly "cnf" …
    assert!(matches!(
        parse("p sat 3 2\n1 0\n"),
        Err(DimacsError::MalformedHeader(_))
    ));
    // … and there must be no 4th token after `p cnf <nv> <nc>`.
    assert!(matches!(
        parse("p cnf 3 2 extra\n1 0\n"),
        Err(DimacsError::MalformedHeader(_))
    ));
}

#[test]
fn parse_rejects_non_integer_var_and_clause_counts() {
    // Both header counts must be nats (the clause count is advisory but must still parse).
    assert!(matches!(
        parse("p cnf x 2\n1 0\n"),
        Err(DimacsError::MalformedHeader(_))
    ));
    assert!(matches!(
        parse("p cnf 3 y\n1 0\n"),
        Err(DimacsError::MalformedHeader(_))
    ));
}

#[test]
fn parse_rejects_clause_body_before_any_header() {
    assert_eq!(parse("1 -2 0\n"), Err(DimacsError::MissingHeader));
}

#[test]
fn parse_rejects_a_non_integer_literal_token() {
    assert_eq!(
        parse("p cnf 3 1\n1 x 0\n"),
        Err(DimacsError::InvalidToken("x".to_string())),
    );
}

#[test]
fn parse_rejects_an_unterminated_final_clause() {
    // Literals with no terminating `0` at end-of-input is a typed error, not a silent clause.
    assert_eq!(
        parse("p cnf 3 1\n1 2\n"),
        Err(DimacsError::UnterminatedClause)
    );
}

#[test]
fn parse_reports_the_exact_out_of_range_variable() {
    // The boundary: var == num_vars is IN range; var == num_vars + 1 is out. Pins `> num_vars`.
    assert!(
        parse("p cnf 3 1\n3 0\n").is_ok(),
        "var 3 with num_vars 3 is in range"
    );
    assert_eq!(
        parse("p cnf 3 1\n4 0\n"),
        Err(DimacsError::VarOutOfRange {
            var: 4,
            num_vars: 3
        }),
    );
}

#[test]
fn parse_accepts_clauses_spanning_and_sharing_lines() {
    // A clause may span several lines; several clauses may share a line.
    let cnf = parse("p cnf 3 2\n1\n-2 0 3 0\n").expect("multi-line + shared-line clauses");
    assert_eq!(cnf.clauses.len(), 2);
    assert_eq!(
        cnf.clauses[0].len(),
        2,
        "clause 1 = {{1, -2}} across two lines"
    );
    assert_eq!(
        cnf.clauses[1].len(),
        1,
        "clause 2 = {{3}} shares the second line"
    );
}
