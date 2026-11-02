//! Shared harness for the language crate's integration tests: parse a whole LOGOS program the way
//! every compile seam does (lexer → discovery → parser) and hand back the statements or the
//! refusal with its span in the source.
#![allow(dead_code)]

use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::analysis::{DiscoveryPass, TypeRegistry};
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::{Expr, LogicExpr, NounPhrase, Stmt, Term, ThematicRole, TypeExpr};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::error::ParseError;
use logicaffeine_language::{Lexer, Parser};

/// A parsed program together with the interner its symbols resolve through.
pub struct Parsed {
    pub stmts: Vec<Stmt<'static>>,
    pub interner: &'static Interner,
}

/// Parse `src` as a program.
pub fn parse(src: &str) -> Result<Parsed, ParseError> {
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let world_state: &'static mut WorldState = Box::leak(Box::new(WorldState::new()));
    let expr_arena: &'static Arena<LogicExpr> = Box::leak(Box::new(Arena::new()));
    let term_arena: &'static Arena<Term> = Box::leak(Box::new(Arena::new()));
    let np_arena: &'static Arena<NounPhrase> = Box::leak(Box::new(Arena::new()));
    let sym_arena: &'static Arena<Symbol> = Box::leak(Box::new(Arena::new()));
    let role_arena: &'static Arena<(ThematicRole, Term)> = Box::leak(Box::new(Arena::new()));
    let pp_arena: &'static Arena<&'static LogicExpr> = Box::leak(Box::new(Arena::new()));
    let stmt_arena: &'static Arena<Stmt> = Box::leak(Box::new(Arena::new()));
    let iexpr_arena: &'static Arena<Expr> = Box::leak(Box::new(Arena::new()));
    let type_arena: &'static Arena<TypeExpr> = Box::leak(Box::new(Arena::new()));
    let ctx = AstContext::with_types(
        expr_arena,
        term_arena,
        np_arena,
        sym_arena,
        role_arena,
        pp_arena,
        stmt_arena,
        iexpr_arena,
        type_arena,
    );
    let mut lexer = Lexer::new(src, interner);
    let tokens = lexer.tokenize();
    let registry: TypeRegistry = DiscoveryPass::new(&tokens, interner).run_full().types;
    let mut parser = Parser::new(tokens, world_state, &mut *interner, ctx, registry);
    let stmts = parser.parse_program()?;
    drop(parser);
    Ok(Parsed { stmts, interner })
}

/// The refusal `src` must produce: its message and the exact source text its span covers.
pub fn refusal(src: &str) -> (String, String) {
    match parse(src) {
        Ok(_) => panic!("this program must be refused:\n{src}"),
        Err(e) => {
            let covered = src.get(e.span.start..e.span.end).unwrap_or("").to_string();
            (message_of(&e), covered)
        }
    }
}

/// The program must parse.
pub fn accepted(src: &str) -> Parsed {
    parse(src).unwrap_or_else(|e| {
        panic!(
            "this program must parse, and was refused with {:?}:\n{src}",
            message_of(&e)
        )
    })
}

/// A refusal's human text.
pub fn message_of(e: &ParseError) -> String {
    match &e.kind {
        logicaffeine_language::error::ParseErrorKind::Custom(m) => m.clone(),
        other => format!("{other:?}"),
    }
}

/// `(line, column)`, both 1-based, of byte `offset` in `src`.
pub fn line_col(src: &str, offset: usize) -> (usize, usize) {
    let before = &src[..offset.min(src.len())];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, col)
}
