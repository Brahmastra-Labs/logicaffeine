use super::*;
use crate::arena::Arena;
use crate::ast::NounPhrase;
use crate::drs::WorldState;

#[test]
fn guard_restores_all_fields_on_drop() {
    let mut interner = Interner::new();
    let mut world_state = WorldState::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();
    let np_arena: Arena<NounPhrase> = Arena::new();
    let sym_arena: Arena<Symbol> = Arena::new();
    let role_arena: Arena<(ThematicRole, Term)> = Arena::new();
    let pp_arena: Arena<&LogicExpr> = Arena::new();

    let ctx = AstContext::new(
        &expr_arena, &term_arena, &np_arena, &sym_arena, &role_arena, &pp_arena,
    );

    let mut lexer = Lexer::new("a b c d e", &mut interner);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, crate::analysis::TypeRegistry::default());

    let initial_pos = parser.current;
    let initial_var_counter = parser.var_counter;
    let initial_bindings_len = parser.donkey_bindings.len();
    let initial_island = parser.current_island;
    let initial_time = parser.pending_time;

    {
        let mut guard = parser.guard();
        guard.current = 3;
        guard.var_counter = 99;
        guard.donkey_bindings.push((Symbol::EMPTY, Symbol::EMPTY, false, false));
        guard.current_island = 42;
        guard.pending_time = Some(Time::Past);
    }

    assert_eq!(parser.current, initial_pos, "position not restored");
    assert_eq!(parser.var_counter, initial_var_counter, "var_counter not restored");
    assert_eq!(parser.donkey_bindings.len(), initial_bindings_len, "bindings not restored");
    assert_eq!(parser.current_island, initial_island, "island not restored");
    assert_eq!(parser.pending_time, initial_time, "time not restored");
}

#[test]
fn guard_preserves_state_on_commit() {
    let mut interner = Interner::new();
    let mut world_state = WorldState::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();
    let np_arena: Arena<NounPhrase> = Arena::new();
    let sym_arena: Arena<Symbol> = Arena::new();
    let role_arena: Arena<(ThematicRole, Term)> = Arena::new();
    let pp_arena: Arena<&LogicExpr> = Arena::new();

    let ctx = AstContext::new(
        &expr_arena, &term_arena, &np_arena, &sym_arena, &role_arena, &pp_arena,
    );

    let mut lexer = Lexer::new("a b c", &mut interner);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, crate::analysis::TypeRegistry::default());

    {
        let mut guard = parser.guard();
        guard.current = 2;
        guard.var_counter = 50;
        guard.commit();
    }

    assert_eq!(parser.current, 2, "position should be preserved after commit");
    assert_eq!(parser.var_counter, 50, "var_counter should be preserved after commit");
}

#[test]
fn check_any_matches_wh_words() {
    let mut interner = Interner::new();
    let mut world_state = WorldState::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();
    let np_arena: Arena<NounPhrase> = Arena::new();
    let sym_arena: Arena<Symbol> = Arena::new();
    let role_arena: Arena<(ThematicRole, Term)> = Arena::new();
    let pp_arena: Arena<&LogicExpr> = Arena::new();

    let ctx = AstContext::new(
        &expr_arena, &term_arena, &np_arena, &sym_arena, &role_arena, &pp_arena,
    );

    let mut lexer = Lexer::new("who what where", &mut interner);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, crate::analysis::TypeRegistry::default());

    assert!(parser.check_any(TokenType::WH_WORDS));
    parser.current = 1;
    assert!(parser.check_any(TokenType::WH_WORDS));
    parser.current = 2;
    assert!(parser.check_any(TokenType::WH_WORDS));
}

#[test]
fn check_any_rejects_non_matching() {
    let mut interner = Interner::new();
    let mut world_state = WorldState::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();
    let np_arena: Arena<NounPhrase> = Arena::new();
    let sym_arena: Arena<Symbol> = Arena::new();
    let role_arena: Arena<(ThematicRole, Term)> = Arena::new();
    let pp_arena: Arena<&LogicExpr> = Arena::new();

    let ctx = AstContext::new(
        &expr_arena, &term_arena, &np_arena, &sym_arena, &role_arena, &pp_arena,
    );

    let mut lexer = Lexer::new("if then", &mut interner);
    let tokens = lexer.tokenize();
    let parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, crate::analysis::TypeRegistry::default());

    assert!(!parser.check_any(TokenType::WH_WORDS));
    assert!(!parser.check_any(TokenType::MODALS));
}
