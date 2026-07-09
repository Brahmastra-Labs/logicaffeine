//! Multi-error collection: the typechecker reports EVERY failing top-level
//! statement (with its index for span mapping), not just the first — the IDE
//! substrate. The strict `check_program` fail-fast contract is unchanged.

use logicaffeine_base::{Arena, Interner};
use logicaffeine_compile::analysis::check_program_collect;
use logicaffeine_language::{
    analysis::DiscoveryPass,
    arena_ctx::AstContext,
    ast::{Expr, Stmt, TypeExpr},
    drs::WorldState,
    Lexer, Parser,
};

fn with_parsed<R>(source: &str, f: impl FnOnce(&[Stmt], &Interner) -> R) -> R {
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

    let stmts = {
        let mut parser = Parser::new(
            tokens,
            &mut world_state,
            &mut interner,
            ast_ctx,
            type_registry,
        );
        parser.parse_program().expect("program must parse")
    };
    f(&stmts, &interner)
}

#[test]
fn two_independent_type_errors_both_report_with_their_indices() {
    // Returning Text from an `-> Int` function is a CHECKER error that parses
    // clean; annotated-Let mismatches are caught earlier, at parse time.
    let source = "## To f () -> Int:\n    Return \"oops\".\n\n## To g () -> Int:\n    Return \"worse\".\n\n## Main\nShow 1.\n";
    with_parsed(source, |stmts, interner| {
        let registry = logicaffeine_language::analysis::TypeRegistry::default();
        let (_env, errors) = check_program_collect(stmts, interner, &registry);
        assert_eq!(
            errors.len(),
            2,
            "both bad functions must report, got: {errors:?}"
        );
        assert_eq!(errors[0].stmt_index, Some(0));
        assert_eq!(errors[1].stmt_index, Some(1));
    });
}

#[test]
fn a_clean_program_collects_no_errors() {
    let source = "## Main\nLet x be 5.\nShow x.\n";
    with_parsed(source, |stmts, interner| {
        let registry = logicaffeine_language::analysis::TypeRegistry::default();
        let (_env, errors) = check_program_collect(stmts, interner, &registry);
        assert!(errors.is_empty(), "clean program, got: {errors:?}");
    });
}

#[test]
fn an_error_does_not_cascade_into_later_valid_statements() {
    let source = "## To f () -> Int:\n    Return \"oops\".\n\n## Main\nLet y be 7.\nShow y.\n";
    with_parsed(source, |stmts, interner| {
        let registry = logicaffeine_language::analysis::TypeRegistry::default();
        let (_env, errors) = check_program_collect(stmts, interner, &registry);
        assert_eq!(
            errors.len(),
            1,
            "only the bad function reports; Main stays checkable: {errors:?}"
        );
        assert_eq!(errors[0].stmt_index, Some(0));
    });
}
