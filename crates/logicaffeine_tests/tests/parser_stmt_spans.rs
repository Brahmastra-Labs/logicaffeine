//! The parser's statement-span side-table: one `Span` per parsed top-level
//! statement, in push order. This is the keystone that gives typechecker
//! errors, ownership cause-links, and the rustc sourcemap real spans without
//! threading a span through every `Stmt` variant.

use logicaffeine_base::{Arena, Interner};
use logicaffeine_language::{
    analysis::DiscoveryPass,
    arena_ctx::AstContext,
    ast::{Expr, Stmt, StmtId, TypeExpr},
    drs::WorldState,
    parser::NestedStmtSpans,
    token::Span,
    Lexer, Parser,
};

/// Parse `source` exactly like the production pipeline (lex → MWE →
/// discovery → parse) and return (statement count, spans, span texts).
fn parse_spans(source: &str) -> (usize, Vec<Span>, Vec<String>) {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();
    let mwe_trie = logicaffeine_language::mwe::build_mwe_trie();
    let tokens = logicaffeine_language::mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    let type_registry = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        discovery.run_full().types
    };

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();
    let ast_ctx = AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_expr_arena,
    );

    // No process_block_headers(): parse_program handles blocks itself (the
    // compile path's contract — see logicaffeine_compile::compile).
    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
    );
    let stmts = parser.parse_program().expect("program must parse");

    let spans = parser.stmt_spans().to_vec();
    let texts = spans
        .iter()
        .map(|span| source[span.start..span.end].to_string())
        .collect();
    (stmts.len(), spans, texts)
}

#[test]
fn every_top_level_statement_gets_a_span_in_order() {
    let source = "## Main\nLet x be 5.\nSet x to 6.\nShow x.\n";
    let (count, _, texts) = parse_spans(source);

    assert_eq!(count, 3);
    assert_eq!(
        texts.len(),
        count,
        "stmt_spans must align 1:1 with parsed statements"
    );
    assert!(texts[0].starts_with("Let x"), "got {:?}", texts[0]);
    assert!(
        texts[0].ends_with('.'),
        "spans include the period: {:?}",
        texts[0]
    );
    assert!(texts[1].starts_with("Set x"), "got {:?}", texts[1]);
    assert!(texts[2].starts_with("Show x"), "got {:?}", texts[2]);
}

#[test]
fn a_nested_block_is_one_top_level_span() {
    let source = "## Main\nIf 1 < 2:\n    Show 1.\nShow 2.\n";
    let (count, _, texts) = parse_spans(source);

    assert_eq!(count, 2, "If-block and trailing Show");
    assert!(texts[0].starts_with("If 1"), "got {:?}", texts[0]);
    assert!(
        texts[0].contains("Show 1."),
        "the If span covers its body: {:?}",
        texts[0]
    );
    assert!(texts[1].starts_with("Show 2"), "got {:?}", texts[1]);
}

#[test]
fn function_definitions_get_spans_too() {
    let source = "## To double (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nShow double(21).\n";
    let (count, _, texts) = parse_spans(source);

    assert_eq!(count, 2, "function def + Show");
    assert!(
        texts[0].contains("Return n * 2."),
        "the function span covers its body: {:?}",
        texts[0]
    );
    assert!(texts[1].starts_with("Show double"), "got {:?}", texts[1]);
}

#[test]
fn spans_are_ascending_and_non_overlapping() {
    let source = "## Main\nLet a be 1.\nLet b be 2.\nLet c be a + b.\nShow c.\n";
    let (count, spans, _) = parse_spans(source);

    assert_eq!(spans.len(), count);
    for pair in spans.windows(2) {
        assert!(
            pair[0].end <= pair[1].start,
            "spans must be ascending and disjoint: {:?}",
            pair
        );
    }
}

// ---------------------------------------------------------------------------
// Nested statements: every statement read inside a block has a span of its own,
// found by identity through `Parser::nested_stmt_spans`.
// ---------------------------------------------------------------------------

/// Parse `source` like the production pipeline, then hand the program and the
/// parser's nested span index to `inspect` while the AST is alive.
fn with_program<R>(source: &str, inspect: impl FnOnce(&[Stmt<'_>], &NestedStmtSpans) -> R) -> R {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();
    let mwe_trie = logicaffeine_language::mwe::build_mwe_trie();
    let tokens = logicaffeine_language::mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);
    let type_registry = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        discovery.run_full().types
    };
    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();
    let ast_ctx = AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_expr_arena,
    );
    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
    );
    let stmts = parser.parse_program().expect("program must parse");
    let nested = parser.nested_stmt_spans().clone();
    inspect(&stmts, &nested)
}

/// The text a statement's nested span covers, or `None` when it has none.
fn spelled(source: &str, nested: &NestedStmtSpans, stmt: &Stmt<'_>) -> Option<String> {
    nested
        .get(StmtId::of(stmt))
        .map(|span| source[span.start..span.end].to_string())
}

fn all_spelled(source: &str, nested: &NestedStmtSpans, block: &[Stmt<'_>]) -> Vec<Option<String>> {
    block.iter().map(|s| spelled(source, nested, s)).collect()
}

fn body_of<'a>(stmt: &Stmt<'a>) -> &'a [Stmt<'a>] {
    match stmt {
        Stmt::FunctionDef { body, .. } | Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
            body
        }
        other => panic!("expected a statement with a body, got {other:?}"),
    }
}

fn branches_of<'a>(stmt: &Stmt<'a>) -> (&'a [Stmt<'a>], Option<&'a [Stmt<'a>]>) {
    match stmt {
        Stmt::If {
            then_block,
            else_block,
            ..
        } => (then_block, *else_block),
        other => panic!("expected an If, got {other:?}"),
    }
}

fn some(text: &str) -> Option<String> {
    Some(text.to_string())
}

#[test]
fn a_function_bodys_statements_each_get_a_span() {
    let source =
        "## To f (n: Int) -> Int:\n    Let m be n + 1.\n    Return m.\n\n## Main\nShow f(1).\n";
    with_program(source, |stmts, nested| {
        assert_eq!(
            all_spelled(source, nested, body_of(&stmts[0])),
            vec![some("Let m be n + 1."), some("Return m.")],
        );
    });
}

#[test]
fn both_branches_of_an_if_get_spans() {
    let source = "## Main\nIf 1 < 2:\n    Show 1.\n    Show 3.\nOtherwise:\n    Show 2.\n";
    with_program(source, |stmts, nested| {
        let (then_block, else_block) = branches_of(&stmts[0]);
        assert_eq!(
            all_spelled(source, nested, then_block),
            vec![some("Show 1."), some("Show 3.")]
        );
        assert_eq!(
            all_spelled(source, nested, else_block.expect("the Otherwise branch")),
            vec![some("Show 2.")],
        );
    });
}

#[test]
fn an_otherwise_if_chain_spans_each_link() {
    let source = "## Main\nLet x be 3.\nIf x < 2:\n    Show 1.\nOtherwise If x < 5:\n    Show 2.\nOtherwise:\n    Show 3.\n";
    with_program(source, |stmts, nested| {
        let (_, else_block) = branches_of(&stmts[1]);
        let else_block = else_block.expect("the Otherwise If link");
        assert_eq!(else_block.len(), 1, "the chain nests one If per link");
        let link = spelled(source, nested, &else_block[0]).expect("the nested If has a span");
        assert!(
            link.starts_with("If x < 5:"),
            "the link starts at its own If: {link:?}"
        );
        assert!(
            link.ends_with("Show 3."),
            "the link runs to the end of the chain: {link:?}"
        );
        let (inner_then, inner_else) = branches_of(&else_block[0]);
        assert_eq!(
            all_spelled(source, nested, inner_then),
            vec![some("Show 2.")]
        );
        assert_eq!(
            all_spelled(source, nested, inner_else.expect("the last Otherwise")),
            vec![some("Show 3.")]
        );
    });
}

#[test]
fn loop_bodies_get_spans() {
    let source = "## Main\nLet x be 0.\nWhile x < 3:\n    Set x to x + 1.\nRepeat for i from 1 to 2:\n    Show i.\n";
    with_program(source, |stmts, nested| {
        assert_eq!(
            all_spelled(source, nested, body_of(&stmts[1])),
            vec![some("Set x to x + 1.")]
        );
        assert_eq!(
            all_spelled(source, nested, body_of(&stmts[2])),
            vec![some("Show i.")]
        );
    });
}

#[test]
fn every_level_of_nesting_gets_spans_that_end_at_their_last_full_stop() {
    let source = "## To f (n: Int) -> Int:\n    Let i be 0.\n    While i < n:\n        If i > 1:\n            Return i.\n        Set i to i + 1.\n    Return 0.\n\n## Main\nShow f(3).\n";
    with_program(source, |stmts, nested| {
        let body = body_of(&stmts[0]);
        assert_eq!(
            all_spelled(source, nested, body),
            vec![
                some("Let i be 0."),
                some("While i < n:\n        If i > 1:\n            Return i.\n        Set i to i + 1."),
                some("Return 0."),
            ],
            "a block statement stops at its last full stop, not at the next line's indentation",
        );
        let loop_body = body_of(&body[1]);
        assert_eq!(
            all_spelled(source, nested, loop_body),
            vec![
                some("If i > 1:\n            Return i."),
                some("Set i to i + 1.")
            ],
        );
        let (then_block, _) = branches_of(&loop_body[0]);
        assert_eq!(
            all_spelled(source, nested, then_block),
            vec![some("Return i.")]
        );
    });
}

#[test]
fn a_contract_rewrite_keeps_the_authors_spans_and_adds_none_of_its_own() {
    // `Requires`/`Ensures` desugar into asserts the parser writes itself: at entry, and before
    // every return, nested ones included — which rebuilds the If around the early return.
    let source = "## To pick (x: Int) -> Int:\n    Requires x is at least 0.\n    Ensures x is greater than 0.\n    If x is greater than 10:\n        Return x.\n    Return x.\n## Main\nShow pick(5).";
    with_program(source, |stmts, nested| {
        let body = body_of(&stmts[0]);
        let texts = all_spelled(source, nested, body);
        assert_eq!(
            texts,
            vec![
                None,
                some("If x is greater than 10:\n        Return x."),
                None,
                some("Return x."),
            ],
            "the Requires assert, the author's If, the Ensures assert, the author's Return: {body:?}",
        );
        let (then_block, _) = branches_of(&body[1]);
        assert_eq!(
            all_spelled(source, nested, then_block),
            vec![None, some("Return x.")],
            "inside the rebuilt If: the Ensures assert the parser wrote, then the author's Return",
        );
    });
}

#[test]
fn a_desugared_statement_is_spanned_by_what_the_author_wrote() {
    // A place write through two indexes desugars into a Splice of statements the parser writes;
    // the Splice carries the author's sentence, its parts carry nothing.
    let source = "## To f (grid: Seq of Seq of Int) -> Int:\n    Set item 1 of item 1 of grid to 5.\n    Return 0.\n\n## Main\nShow 1.\n";
    with_program(source, |stmts, nested| {
        let body = body_of(&stmts[0]);
        assert_eq!(
            spelled(source, nested, &body[0]),
            some("Set item 1 of item 1 of grid to 5.")
        );
        match &body[0] {
            Stmt::Splice { body: parts } => {
                assert!(!parts.is_empty());
                assert!(
                    parts.iter().all(|p| spelled(source, nested, p).is_none()),
                    "the parser's own statements claim no source",
                );
            }
            other => panic!("a two-index place write desugars into a Splice, got {other:?}"),
        }
    });
}

#[test]
fn top_level_statements_stay_in_the_positional_table() {
    let source = "## Main\nLet x be 5.\nIf x > 1:\n    Show x.\n";
    with_program(source, |stmts, nested| {
        assert!(stmts.iter().all(|s| spelled(source, nested, s).is_none()));
        assert_eq!(nested.len(), 1, "only the If's body statement is nested");
    });
}
