//! W2.1 — `test_surface_catalog`: every Wave-2 grammar surface, locked.
//!
//! Each test
//! pins one committed surface of the in-language test framework's GRAMMAR:
//!
//! - `## Test: <prose sentence>` blocks parse as CODE (an imperative body),
//!   register a `TestDef` named by the prose, and refuse duplicate names.
//! - `Expect that <condition>.` reuses the full English condition grammar and
//!   is refused outside test blocks with a Socratic hint.
//! - `Expect this to fail [with "…"]:` and `Expect output:` parse.
//! - Test-header metadata sentences: `Skip this test because "…".`, capability
//!   sentences (`This test may use the real filesystem.`), and the op-budget
//!   escalation (`This test may run up to N steps.`).
//! - `For each of these cases:` + a markdown pipe table binds columns to rows.
//! - `is approximately X within eps` is grammar-level (usable anywhere).
//! - `Advance time by …` / `Given a file …` parse in tests only.
//! - Unknown `##` headers still degrade to prose (the additive guarantee).
//!
//! Runner semantics (execution, filtering, budgets) land in Wave 3 — this file
//! locks the parse products only. Doctests in `## Example` blocks get their own
//! lock in W2.5.

use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_language::ast::stmt::{ExpectedOutput, Stmt};

/// Parse and return Ok(()) or the parse-advice error.
fn parse_result(source: &str) -> Result<(), String> {
    with_parsed_program(source, |parsed, _interner| match parsed {
        Ok(_) => Ok(()),
        Err(advice) => Err(advice),
    })
}

/// Walk the parse product, extracting an owned summary of every `TestDef`:
/// (name, skip reason, [fs, net, env, clock] capabilities, budget,
///  case-table (columns, row count), body-statement census).
#[allow(clippy::type_complexity)]
fn test_defs(
    source: &str,
) -> Vec<(
    String,
    Option<String>,
    [bool; 4],
    Option<u64>,
    Option<(Vec<String>, usize)>,
    BodyCensus,
)> {
    with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("the corpus program parses");
        stmts
            .iter()
            .filter_map(|s| match s {
                Stmt::TestDef {
                    name,
                    skip,
                    capabilities,
                    budget,
                    cases,
                    body,
                } => {
                    let mut census = BodyCensus::default();
                    for stmt in body.iter() {
                        match stmt {
                            Stmt::Expect { .. } => census.expects += 1,
                            Stmt::ExpectFail { message, .. } => {
                                census.expect_fails += 1;
                                if let Some(m) = message {
                                    census.fail_messages.push(interner.resolve(*m).to_string());
                                }
                            }
                            Stmt::ExpectOutput { expected } => {
                                census.expect_outputs += 1;
                                if let ExpectedOutput::Inline(text) = expected {
                                    census
                                        .inline_outputs
                                        .push(interner.resolve(*text).to_string());
                                }
                            }
                            Stmt::AdvanceTime { .. } => census.advances += 1,
                            Stmt::GivenFile { .. } => census.given_files += 1,
                            _ => {}
                        }
                    }
                    Some((
                        interner.resolve(*name).to_string(),
                        skip.map(|s| interner.resolve(s).to_string()),
                        [
                            capabilities.filesystem,
                            capabilities.network,
                            capabilities.environment,
                            capabilities.clock,
                        ],
                        *budget,
                        cases.as_ref().map(|t| {
                            (
                                t.columns
                                    .iter()
                                    .map(|c| interner.resolve(*c).to_string())
                                    .collect(),
                                t.rows.len(),
                            )
                        }),
                        census,
                    ))
                }
                _ => None,
            })
            .collect()
    })
}

#[derive(Default, Debug)]
struct BodyCensus {
    expects: usize,
    expect_fails: usize,
    fail_messages: Vec<String>,
    expect_outputs: usize,
    inline_outputs: Vec<String>,
    advances: usize,
    given_files: usize,
}

/// A `## Test` body is CODE, not prose: a garbage body must refuse to parse.
/// (Before Wave 2, `## Test` degraded to a prose block and garbage sailed
/// through silently — the exact bug class the CONSEQUENTIAL-typo guard exists
/// for, now applied to tests.)
#[test]
fn test_block_body_is_code_not_prose() {
    let src = "## Test: Garbage guard\n\
        \x20   Let let let.\n";
    assert!(
        parse_result(src).is_err(),
        "a nonsense sentence inside a ## Test block must be a parse error"
    );
}

/// The prose after the colon is the test's name and identity.
#[test]
fn test_block_registers_testdef_with_prose_name() {
    let src = "## Test: Addition carries correctly\n\
        \x20   Let x be 1 plus 1.\n\
        \x20   Expect that x is 2.\n";
    let defs = test_defs(src);
    assert_eq!(defs.len(), 1, "exactly one TestDef registers");
    assert_eq!(
        defs[0].0, "Addition carries correctly",
        "the prose name is the identity"
    );
    assert_eq!(
        defs[0].5.expects, 1,
        "the Expect statement lands in the body"
    );
}

/// Duplicate test names in one module are a compile error, not a surprise.
#[test]
fn duplicate_test_names_are_refused() {
    let src = "## Test: Same name\n\
        \x20   Expect that 1 is 1.\n\
        \n\
        ## Test: Same name\n\
        \x20   Expect that 2 is 2.\n";
    let err = parse_result(src).expect_err("duplicate test names must be refused");
    assert!(
        err.to_lowercase().contains("same name") || err.to_lowercase().contains("duplicate"),
        "the error names the duplicated test: {err}"
    );
}

/// `Expect` lives in tests; outside one, the refusal teaches (Socratic hint
/// pointing at `Require that`).
#[test]
fn expect_outside_a_test_is_refused_socratically() {
    let src = "## Main\n\
        \x20   Expect that 1 is 1.\n";
    let err = parse_result(src).expect_err("Expect outside a test must be refused");
    assert!(
        err.contains("Require"),
        "the refusal suggests `Require that` for runtime guards: {err}"
    );
}

/// `Expect that` reuses the FULL English condition grammar — no matcher DSL.
#[test]
fn expect_reuses_the_condition_grammar() {
    let src = "## Test: The condition grammar is all here\n\
        \x20   Let x be 7.\n\
        \x20   Expect that x is at least 3.\n\
        \x20   Expect that x is between 1 and 10.\n\
        \x20   Expect that x is divisible by 7.\n";
    let defs = test_defs(src);
    assert_eq!(defs.len(), 1);
    assert_eq!(
        defs[0].5.expects, 3,
        "every comparison surface parses inside Expect"
    );
}

/// `Expect this to fail:` is the ONLY error-expectation form — block-shaped,
/// with an optional expected message.
#[test]
fn expect_this_to_fail_parses() {
    let src = "## Test: Failure is expected here\n\
        \x20   Let a be 10.\n\
        \x20   Let b be 0.\n\
        \x20   Expect this to fail:\n\
        \x20       Let x be a / b.\n\
        \x20   Expect this to fail with \"Division by zero\":\n\
        \x20       Let y be a / b.\n";
    let defs = test_defs(src);
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].5.expect_fails, 2, "both forms parse");
    assert_eq!(
        defs[0].5.fail_messages,
        vec!["Division by zero".to_string()],
        "the with-message form carries its expected text"
    );
}

/// `Expect output:` — the in-document stdout snapshot, indented body.
#[test]
fn expect_output_inline_parses() {
    let src = "## Test: The answer prints\n\
        \x20   Show 42.\n\
        \x20   Expect output:\n\
        \x20       42\n";
    let defs = test_defs(src);
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].5.expect_outputs, 1);
    assert_eq!(defs[0].5.inline_outputs, vec!["42".to_string()]);
}

/// Test-header metadata: the skip sentence (reason mandatory), capability
/// sentences, and the deterministic op-budget escalation.
#[test]
fn skip_capability_and_budget_metadata() {
    let src = "## Test: A fully annotated test\n\
        \x20   Skip this test because \"the flag has not shipped\".\n\
        \x20   This test may use the real filesystem.\n\
        \x20   This test may run up to 5000000 steps.\n\
        \x20   Expect that 1 is 1.\n";
    let defs = test_defs(src);
    assert_eq!(defs.len(), 1);
    let (_, skip, caps, budget, _, _) = (
        &defs[0].0, &defs[0].1, &defs[0].2, &defs[0].3, &defs[0].4, &defs[0].5,
    );
    assert_eq!(
        skip.as_deref(),
        Some("the flag has not shipped"),
        "the reason is captured"
    );
    assert_eq!(
        *caps,
        [true, false, false, false],
        "exactly the filesystem capability is granted"
    );
    assert_eq!(
        *budget,
        Some(5_000_000),
        "the op budget escalation is captured"
    );
}

/// Table-driven tests: the markdown table IS the fixture — columns bind names,
/// each row is a sub-test.
#[test]
fn case_table_parses() {
    let src = "## Test: Addition cases\n\
        \n\
        For each of these cases:\n\
        \n\
        | a  | b | total |\n\
        |----|---|-------|\n\
        | 1  | 2 | 3     |\n\
        | 99 | 1 | 100   |\n\
        \n\
        Expect that a plus b is total.\n";
    let defs = test_defs(src);
    assert_eq!(defs.len(), 1);
    let (columns, rows) = defs[0].4.clone().expect("the case table registers");
    assert_eq!(
        columns,
        vec!["a".to_string(), "b".to_string(), "total".to_string()]
    );
    assert_eq!(rows, 2, "two data rows (the separator row is not data)");
    assert_eq!(
        defs[0].5.expects, 1,
        "the body below the table belongs to the test"
    );
}

/// `is approximately X within eps` is a GRAMMAR-level comparison — usable in
/// any condition anywhere (the float-equality footgun dies in the language,
/// not the framework).
#[test]
fn approximately_within_is_grammar_level() {
    let in_main = "## Main\n\
        \x20   Let pi be 3.14.\n\
        \x20   Require that pi is approximately 3.0 within 0.5.\n\
        \x20   Show 1.\n";
    assert!(
        parse_result(in_main).is_ok(),
        "approximately-within parses in ordinary code: {:?}",
        parse_result(in_main)
    );
    let in_test = "## Test: Floats compare honestly\n\
        \x20   Let pi be 3.14.\n\
        \x20   Expect that pi is approximately 3.0 within 0.5.\n";
    assert_eq!(test_defs(in_test).len(), 1, "and inside Expect");
}

/// `Advance time by …` is the virtual-clock control — test-only surface.
#[test]
fn advance_time_is_test_only() {
    let in_test = "## Test: Timeouts are instant\n\
        \x20   Advance time by 3 seconds.\n\
        \x20   Expect that 1 is 1.\n";
    let defs = test_defs(in_test);
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].5.advances, 1);

    let in_main = "## Main\n\
        \x20   Advance time by 3 seconds.\n";
    let err = parse_result(in_main).expect_err("Advance time outside a test must be refused");
    assert!(
        err.to_lowercase().contains("test"),
        "the refusal names tests: {err}"
    );
}

/// `Given a file "…" containing "…".` pre-seeds the sealed world — test-only.
#[test]
fn given_file_is_test_only() {
    let in_test = "## Test: The loader reads its fixture\n\
        \x20   Given a file \"data.txt\" containing \"hello\".\n\
        \x20   Expect that 1 is 1.\n";
    let defs = test_defs(in_test);
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].5.given_files, 1);

    let in_main = "## Main\n\
        \x20   Given a file \"data.txt\" containing \"hello\".\n";
    assert!(
        parse_result(in_main).is_err(),
        "Given-a-file outside a test must be refused"
    );
}

/// The additive guarantee: unknown `##` headers still degrade to prose, so a
/// program written for a NEWER toolchain still parses on this one (and prose
/// headings like `## Design` keep working forever).
#[test]
fn unknown_headers_stay_prose() {
    let src = "## Zzzz Quargle\n\
        Some literate prose that is definitely not code.\n\
        \n\
        ## Main\n\
        \x20   Show 1.\n";
    assert!(
        parse_result(src).is_ok(),
        "unknown headers are prose, never errors: {:?}",
        parse_result(src)
    );
}
