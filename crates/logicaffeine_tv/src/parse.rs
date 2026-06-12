//! Parse a LOGOS source string into the arena-allocated AST and hand it to a
//! callback, mirroring the front-end of `compile::compile_program_full`.
//!
//! The parser needs a bundle of bumpalo arenas whose lifetimes cannot escape the
//! parsing scope, so access is given through a callback `f(&[Stmt], &Interner)`.
//! Whatever the callback returns (typically a [`crate::symexec::SymSummary`], which
//! holds only owned `VerifyExpr`s) outlives the arenas.

use logicaffeine_compile::ast::{Expr, Stmt, TypeExpr};
use logicaffeine_compile::drs::WorldState;
use logicaffeine_compile::{Arena, AstContext, DiscoveryPass, Interner, Lexer, ParseError, Parser};

/// Parse `source`, optionally run the production optimizer, and invoke `f` with the
/// resulting statements and the interner used to intern their symbols.
///
/// When `optimize` is true the statements are passed through
/// `optimize::optimize_program` — the same 14-pass pipeline the real compiler runs —
/// so callers can validate the optimizer's output against the source.
pub fn with_program<R>(
    source: &str,
    optimize: bool,
    f: impl FnOnce(&[Stmt], &Interner) -> R,
) -> Result<R, ParseError> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

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
    let stmts = parser.parse_program()?;
    drop(parser);

    let stmts = if optimize {
        logicaffeine_compile::optimize::optimize_program(
            stmts,
            &imperative_expr_arena,
            &stmt_arena,
            &mut interner,
        )
    } else {
        stmts
    };

    Ok(f(&stmts, &interner))
}
