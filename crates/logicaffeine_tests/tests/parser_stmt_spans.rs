//! The parser's statement-span side-table: one `Span` per parsed top-level
//! statement, in push order. This is the keystone that gives typechecker
//! errors, ownership cause-links, and the rustc sourcemap real spans without
//! threading a span through every `Stmt` variant.

use logicaffeine_base::{Arena, Interner};
use logicaffeine_language::{
    analysis::DiscoveryPass,
    arena_ctx::AstContext,
    ast::{Expr, Stmt, TypeExpr},
    drs::WorldState,
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
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, type_registry);
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
    assert!(texts[0].ends_with('.'), "spans include the period: {:?}", texts[0]);
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
