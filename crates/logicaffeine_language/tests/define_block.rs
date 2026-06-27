//! Rung 0a, Stride 1 — `## Define` block parses to `Stmt::Definition`.
//!
//! The canonical definitional surface is a biconditional sentence:
//!
//! ```text
//! ## Define
//! x is a glorp if and only if x is shiny and x is round.
//! ```
//!
//! whose top connective is `Iff`; the LHS `Predicate` is the definiendum
//! (its name + parameter symbols), the RHS is the definiens. Made-up words
//! keep the test clear of lexicon entailment expansion.

use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::analysis::TypeRegistry;
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::{LogicExpr, NounPhrase, Stmt, Term, ThematicRole};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::token::TokenType;
use logicaffeine_language::{Lexer, Parser};

fn parse_program(input: &str) -> Vec<Stmt<'static>> {
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let world_state: &'static mut WorldState = Box::leak(Box::new(WorldState::new()));
    let expr_arena: &'static Arena<LogicExpr> = Box::leak(Box::new(Arena::new()));
    let term_arena: &'static Arena<Term> = Box::leak(Box::new(Arena::new()));
    let np_arena: &'static Arena<NounPhrase> = Box::leak(Box::new(Arena::new()));
    let sym_arena: &'static Arena<Symbol> = Box::leak(Box::new(Arena::new()));
    let role_arena: &'static Arena<(ThematicRole, Term)> = Box::leak(Box::new(Arena::new()));
    let pp_arena: &'static Arena<&'static LogicExpr> = Box::leak(Box::new(Arena::new()));
    let ctx = AstContext::new(expr_arena, term_arena, np_arena, sym_arena, role_arena, pp_arena);

    let mut lexer = Lexer::new(input, interner);
    let tokens = lexer.tokenize();
    let type_registry = TypeRegistry::default();
    let mut parser = Parser::new(tokens, world_state, interner, ctx, type_registry);
    parser.parse_program().expect("program should parse")
}

#[test]
fn define_block_parses_to_definition_stmt() {
    let stmts = parse_program(
        "## Define\nx is a glorp if and only if x is shiny and x is round.\n",
    );

    let def = stmts
        .iter()
        .find_map(|s| if let Stmt::Definition(d) = s { Some(d) } else { None })
        .expect("## Define should parse to a Stmt::Definition");

    assert!(
        def.name.eq_ignore_ascii_case("glorp"),
        "definiendum name should be 'glorp', got {:?}",
        def.name
    );
    assert_eq!(def.params.len(), 1, "glorp takes one parameter");

    // The definiens is the RHS of the biconditional: a conjunction.
    assert!(
        matches!(def.definiens, LogicExpr::BinaryOp { op: TokenType::And, .. }),
        "definiens should be a conjunction, got {:?}",
        def.definiens
    );
}

#[test]
fn define_block_rejects_non_biconditional() {
    // A `## Define` body that is not a biconditional is a malformed definition;
    // it must NOT silently parse as a definition. (Parsed via parse_program,
    // which should surface the error rather than mint a bogus Stmt::Definition.)
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let world_state: &'static mut WorldState = Box::leak(Box::new(WorldState::new()));
    let expr_arena: &'static Arena<LogicExpr> = Box::leak(Box::new(Arena::new()));
    let term_arena: &'static Arena<Term> = Box::leak(Box::new(Arena::new()));
    let np_arena: &'static Arena<NounPhrase> = Box::leak(Box::new(Arena::new()));
    let sym_arena: &'static Arena<Symbol> = Box::leak(Box::new(Arena::new()));
    let role_arena: &'static Arena<(ThematicRole, Term)> = Box::leak(Box::new(Arena::new()));
    let pp_arena: &'static Arena<&'static LogicExpr> = Box::leak(Box::new(Arena::new()));
    let ctx = AstContext::new(expr_arena, term_arena, np_arena, sym_arena, role_arena, pp_arena);
    let mut lexer = Lexer::new("## Define\nx is a glorp.\n", interner);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(tokens, world_state, interner, ctx, TypeRegistry::default());
    assert!(
        parser.parse_program().is_err(),
        "a non-biconditional `## Define` body must be a parse error"
    );
}
