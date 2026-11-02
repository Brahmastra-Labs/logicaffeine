//! A checker's finding names the statement it came from — nested ones
//! included — as a trail of statement identities, innermost first. With the
//! parser's nested span index that trail resolves to the sentence at fault, so
//! a diagnostic underlines `Return "oops".` rather than the whole function, loop
//! or branch around it.

use logicaffeine_base::{Arena, Interner};
use logicaffeine_compile::analysis::{
    check_program_collect, unify::TypeError, IndexedTypeError, OwnershipChecker, OwnershipErrorKind,
    OwnershipFinding,
};
use logicaffeine_language::{
    analysis::DiscoveryPass,
    arena_ctx::AstContext,
    ast::{Expr, Stmt, StmtId, TypeExpr},
    drs::WorldState,
    parser::NestedStmtSpans,
    Lexer, Parser,
};

/// Parse and typecheck `source` like the LSP pipeline, then hand the program,
/// the nested span index and the findings to `inspect` while the AST is alive.
fn with_findings<R>(
    source: &str,
    inspect: impl FnOnce(&[Stmt<'_>], &NestedStmtSpans, &[IndexedTypeError]) -> R,
) -> R {
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
    let mut parser =
        Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, type_registry.clone());
    let stmts = parser.parse_program().expect("program must parse");
    let nested = parser.nested_stmt_spans().clone();
    let (_env, findings) = check_program_collect(&stmts, &interner, &type_registry);
    inspect(&stmts, &nested, &findings)
}

fn body_of<'a>(stmt: &Stmt<'a>) -> &'a [Stmt<'a>] {
    match stmt {
        Stmt::FunctionDef { body, .. } | Stmt::While { body, .. } | Stmt::Repeat { body, .. } => body,
        other => panic!("expected a statement with a body, got {other:?}"),
    }
}

fn then_of<'a>(stmt: &Stmt<'a>) -> &'a [Stmt<'a>] {
    match stmt {
        Stmt::If { then_block, .. } => then_block,
        other => panic!("expected an If, got {other:?}"),
    }
}

/// The source text a finding's trail resolves to through the nested index.
fn resolved(source: &str, nested: &NestedStmtSpans, finding: &IndexedTypeError) -> Option<String> {
    finding
        .trail
        .iter()
        .find_map(|id| nested.get(*id))
        .map(|span| source[span.start..span.end].to_string())
}

#[test]
fn a_type_error_in_a_function_body_names_its_statement() {
    let source = "## To f () -> Int:\n    Let a be 1.\n    Return \"oops\".\n\n## Main\nShow f().\n";
    with_findings(source, |stmts, nested, findings| {
        assert_eq!(findings.len(), 1, "{findings:?}");
        let body = body_of(&stmts[0]);
        assert_eq!(findings[0].stmt_index, Some(0));
        assert_eq!(
            findings[0].trail,
            vec![StmtId::of(&body[1]), StmtId::of(&stmts[0])],
            "the failing Return, then the function it propagated out of",
        );
        assert_eq!(resolved(source, nested, &findings[0]).as_deref(), Some("Return \"oops\"."));
    });
}

#[test]
fn a_type_error_deep_in_a_loop_names_the_innermost_statement() {
    let source = "## To f (n: Int) -> Int:\n    Let i be 0.\n    While i < n:\n        If i > 1:\n            Return \"deep\".\n        Set i to i + 1.\n    Return 0.\n\n## Main\nShow f(3).\n";
    with_findings(source, |stmts, nested, findings| {
        assert_eq!(findings.len(), 1, "{findings:?}");
        let body = body_of(&stmts[0]);
        let loop_body = body_of(&body[1]);
        let then_block = then_of(&loop_body[0]);
        assert_eq!(
            findings[0].trail,
            vec![
                StmtId::of(&then_block[0]),
                StmtId::of(&loop_body[0]),
                StmtId::of(&body[1]),
                StmtId::of(&stmts[0]),
            ],
            "innermost first: the Return, its If, the While, the function",
        );
        assert_eq!(resolved(source, nested, &findings[0]).as_deref(), Some("Return \"deep\"."));
    });
}

#[test]
fn each_failing_function_names_its_own_statement() {
    let source = "## To f () -> Int:\n    Return \"oops\".\n\n## To g () -> Int:\n    Return \"worse\".\n\n## Main\nShow 1.\n";
    with_findings(source, |_stmts, nested, findings| {
        let texts: Vec<_> = findings.iter().map(|f| resolved(source, nested, f)).collect();
        assert_eq!(
            texts,
            vec![Some("Return \"oops\".".to_string()), Some("Return \"worse\".".to_string())],
            "{findings:?}",
        );
    });
}

#[test]
fn a_dimension_mismatch_names_its_statement() {
    let source = "## Main\nLet len be quantity(3, \"meter\").\nLet t be quantity(2, \"second\").\nShow len + t.\n";
    with_findings(source, |stmts, _nested, findings| {
        let dimension: Vec<_> = findings
            .iter()
            .filter(|f| matches!(f.error, TypeError::DimensionMismatch { .. }))
            .collect();
        assert_eq!(dimension.len(), 1, "{findings:?}");
        assert_eq!(dimension[0].stmt_index, Some(2), "the Show owns the mismatch");
        assert_eq!(dimension[0].trail, vec![StmtId::of(&stmts[2])]);
    });
}

#[test]
fn a_dimension_mismatch_in_a_function_names_its_statement() {
    let source = "## To f () -> Int:\n    Let len be quantity(3, \"meter\").\n    Let t be quantity(2, \"second\").\n    Show len + t.\n    Return 0.\n\n## Main\nShow f().\n";
    with_findings(source, |stmts, nested, findings| {
        let dimension: Vec<_> = findings
            .iter()
            .filter(|f| matches!(f.error, TypeError::DimensionMismatch { .. }))
            .collect();
        assert_eq!(dimension.len(), 1, "{findings:?}");
        let body = body_of(&stmts[0]);
        assert_eq!(dimension[0].stmt_index, Some(0));
        assert_eq!(dimension[0].trail, vec![StmtId::of(&body[2]), StmtId::of(&stmts[0])]);
        assert_eq!(resolved(source, nested, dimension[0]).as_deref(), Some("Show len + t."));
    });
}

#[test]
fn a_top_level_finding_names_only_its_own_statement() {
    let source = "## Main\nLet s be \"a\".\nLet n: Int be s.\nShow n.\n";
    with_findings(source, |stmts, nested, findings| {
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].stmt_index, Some(1));
        assert_eq!(findings[0].trail, vec![StmtId::of(&stmts[1])]);
        assert_eq!(
            resolved(source, nested, &findings[0]),
            None,
            "a top-level statement resolves through the positional table, not the nested index",
        );
    });
}

// ---------------------------------------------------------------------------
// Ownership findings name the use at fault and the Give that caused it.
// ---------------------------------------------------------------------------

fn with_ownership<R>(
    source: &str,
    inspect: impl FnOnce(&[Stmt<'_>], &NestedStmtSpans, &[OwnershipFinding]) -> R,
) -> R {
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
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, type_registry);
    let stmts = parser.parse_program().expect("program must parse");
    let nested = parser.nested_stmt_spans().clone();
    let findings = OwnershipChecker::new(&interner).check_program_collect(&stmts);
    inspect(&stmts, &nested, &findings)
}

fn spelled_by(source: &str, nested: &NestedStmtSpans, id: StmtId) -> Option<String> {
    nested.get(id).map(|span| source[span.start..span.end].to_string())
}

#[test]
fn a_maybe_move_in_a_function_names_the_use_and_the_give() {
    let source = "## To f (c: Bool) -> Int:\n    Let s be a new Seq of Int.\n    If c:\n        Give s to keeper.\n    Show s.\n    Return 0.\n\n## Main\nShow f(true).\n";
    with_ownership(source, |stmts, nested, findings| {
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(matches!(findings[0].error.kind, OwnershipErrorKind::UseAfterMaybeMove { .. }), "{findings:?}");
        let body = body_of(&stmts[0]);
        let give = &then_of(&body[1])[0];
        assert_eq!(findings[0].stmt_index, 0);
        assert_eq!(findings[0].trail, vec![StmtId::of(&body[2]), StmtId::of(&stmts[0])]);
        assert_eq!(spelled_by(source, nested, findings[0].trail[0]).as_deref(), Some("Show s."));
        assert_eq!(findings[0].cause, Some(StmtId::of(give)), "the Give that may have moved it");
        assert_eq!(spelled_by(source, nested, StmtId::of(give)).as_deref(), Some("Give s to keeper."));
    });
}

#[test]
fn a_top_level_maybe_move_names_the_use_and_the_nested_give() {
    let source = "## Main\nLet s be a new Seq of Int.\nLet c be true.\nIf c:\n    Give s to keeper.\nShow s.\n";
    with_ownership(source, |stmts, _nested, findings| {
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(matches!(findings[0].error.kind, OwnershipErrorKind::UseAfterMaybeMove { .. }), "{findings:?}");
        assert_eq!(findings[0].stmt_index, 3);
        assert_eq!(findings[0].trail, vec![StmtId::of(&stmts[3])]);
        assert_eq!(findings[0].cause_stmt_index, Some(2), "the If around the Give");
        assert_eq!(findings[0].cause, Some(StmtId::of(&then_of(&stmts[2])[0])));
    });
}

#[test]
fn a_move_given_in_an_expression_names_the_statement_that_gave_it() {
    let source = "## To keep (xs: Seq of Int):\n    Show length of xs.\n\n## To f () -> Int:\n    Let s be a new Seq of Int.\n    Call keep with give s.\n    Show s.\n    Return 0.\n\n## Main\nShow f().\n";
    with_ownership(source, |stmts, nested, findings| {
        assert_eq!(findings.len(), 1, "{findings:?}");
        let body = body_of(&stmts[1]);
        assert_eq!(findings[0].trail, vec![StmtId::of(&body[2]), StmtId::of(&stmts[1])]);
        assert_eq!(findings[0].cause, Some(StmtId::of(&body[1])));
        assert_eq!(spelled_by(source, nested, StmtId::of(&body[1])).as_deref(), Some("Call keep with give s."));
    });
}
