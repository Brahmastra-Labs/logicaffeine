use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::{Lexer, Parser, ParserMode, WorldState};
use logicaffeine_language::ast::{LogicExpr, NounPhrase, Term, ThematicRole};
use logicaffeine_language::arena_ctx::AstContext;

/// Step 1.2: Trait Split - Verify declarative mode still produces LogicExpr
/// and imperative mode produces Stmt

fn make_ctx<'a>() -> AstContext<'a> {
    let expr_arena: &'static Arena<LogicExpr> = Box::leak(Box::new(Arena::new()));
    let term_arena: &'static Arena<Term> = Box::leak(Box::new(Arena::new()));
    let np_arena: &'static Arena<NounPhrase> = Box::leak(Box::new(Arena::new()));
    let sym_arena: &'static Arena<Symbol> = Box::leak(Box::new(Arena::new()));
    let role_arena: &'static Arena<(ThematicRole, Term)> = Box::leak(Box::new(Arena::new()));
    let pp_arena: &'static Arena<&'static LogicExpr> = Box::leak(Box::new(Arena::new()));

    AstContext::new(expr_arena, term_arena, np_arena, sym_arena, role_arena, pp_arena)
}

#[test]
fn declarative_mode_parses_neoevent() {
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let world_state: &'static mut WorldState = Box::leak(Box::new(WorldState::new()));
    let mut lexer = Lexer::new("John runs.", interner);
    let tokens = lexer.tokenize();

    let ctx = make_ctx();
    let mut parser = Parser::new(tokens, world_state, interner, ctx, logicaffeine_language::analysis::TypeRegistry::default());

    let result = parser.parse();
    assert!(result.is_ok());

    let output = format!("{:?}", result.unwrap());
    assert!(output.contains("NeoEvent") || output.contains("Run"),
        "Expected NeoEvent in declarative mode, got: {}", output);
}

#[test]
fn parser_mode_getter_exists() {
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let world_state: &'static mut WorldState = Box::leak(Box::new(WorldState::new()));
    let mut lexer = Lexer::new("John runs.", interner);
    let tokens = lexer.tokenize();

    let ctx = make_ctx();
    let parser = Parser::new(tokens, world_state, interner, ctx, logicaffeine_language::analysis::TypeRegistry::default());

    // Parser should expose mode() getter - defaults to Declarative
    assert_eq!(parser.mode(), ParserMode::Declarative);
}

#[test]
fn parser_switches_mode_on_main_block() {
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let world_state: &'static mut WorldState = Box::leak(Box::new(WorldState::new()));
    let mut lexer = Lexer::new("## Main\nLet x be 5.", interner);
    let tokens = lexer.tokenize();

    let ctx = make_ctx();
    let mut parser = Parser::new(tokens, world_state, interner, ctx, logicaffeine_language::analysis::TypeRegistry::default());

    // After consuming BlockHeader(Main), mode should switch to Imperative
    parser.process_block_headers();
    assert_eq!(parser.mode(), ParserMode::Imperative);
}

#[test]
fn parser_stays_declarative_on_theorem_block() {
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let world_state: &'static mut WorldState = Box::leak(Box::new(WorldState::new()));
    let mut lexer = Lexer::new("## Theorem\nAll men are mortal.", interner);
    let tokens = lexer.tokenize();

    let ctx = make_ctx();
    let mut parser = Parser::new(tokens, world_state, interner, ctx, logicaffeine_language::analysis::TypeRegistry::default());

    parser.process_block_headers();
    assert_eq!(parser.mode(), ParserMode::Declarative);
}

// Step 1.3: Mode Dispatch Integration

#[test]
fn unknown_word_in_declarative_succeeds() {
    // "Zorblax runs" in declarative mode should auto-register entity and succeed
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let world_state: &'static mut WorldState = Box::leak(Box::new(WorldState::new()));
    let mut lexer = Lexer::new("Zorblax runs.", interner);
    let tokens = lexer.tokenize();

    let ctx = make_ctx();
    let mut parser = Parser::new(tokens, world_state, interner, ctx, logicaffeine_language::analysis::TypeRegistry::default());

    // Parser defaults to Declarative mode
    let result = parser.parse();
    assert!(result.is_ok(), "Declarative mode should accept unknown words: {:?}", result.err());
}

#[test]
fn unknown_word_in_imperative_errors() {
    // "Zorblax runs" in imperative mode should error - variable not defined
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let world_state: &'static mut WorldState = Box::leak(Box::new(WorldState::new()));
    let mut lexer = Lexer::new("## Main\nZorblax runs.", interner);
    let tokens = lexer.tokenize();

    let ctx = make_ctx();
    let mut parser = Parser::new(tokens, world_state, interner, ctx, logicaffeine_language::analysis::TypeRegistry::default());

    parser.process_block_headers();
    assert_eq!(parser.mode(), ParserMode::Imperative);

    let result = parser.parse();
    assert!(result.is_err(), "Imperative mode should reject unknown variables");
}
