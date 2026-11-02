use super::*;
use crate::ast::stmt::{Expr, Stmt, TypeExpr};
use crate::ast::NounPhrase;
use crate::drs::WorldState;
use crate::lint::FindingKind;
use logicaffeine_base::Arena;

// ─── The value-semantics copy lint (`crate::lint`) ─────────────────────────────────────────────
//
// A direct `Parser::parse_program` + `Parser::value_copy_findings` round trip, bypassing the
// stdlib prelude/discovery pipeline entirely (`TypeRegistry::default()`, no `Uses`): the lint is
// purely syntactic (it never resolves a field or a construction against the registry), so a bare
// parameter name is all a fixture needs.

fn copy_lint_findings(source: &str) -> Vec<crate::lint::Finding> {
    let mut interner = Interner::new();
    let mut world_state = WorldState::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();
    let np_arena: Arena<NounPhrase> = Arena::new();
    let sym_arena: Arena<Symbol> = Arena::new();
    let role_arena: Arena<(ThematicRole, Term)> = Arena::new();
    let pp_arena: Arena<&LogicExpr> = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_arena: Arena<TypeExpr> = Arena::new();

    let ctx = AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_arena,
    );

    let types = crate::analysis::TypeRegistry::with_primitives(&mut interner);
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, types);
    let result = parser.parse_program();
    assert!(
        result.is_ok(),
        "fixture failed to parse: {:?}\n{source}",
        result.err()
    );
    parser.value_copy_findings().to_vec()
}

#[test]
fn pop_from_a_copied_field_and_never_writing_back_warns() {
    let findings = copy_lint_findings(
        "## To useOpens (b: Box) -> Int:\n\
        \x20   Let opens be b's opens.\n\
        \x20   Pop from opens into at.\n\
        \x20   Return at.\n",
    );
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].kind, FindingKind::OrphanedCopy);
    assert_eq!(findings[0].severity, crate::lint::Severity::Warning);
    assert!(
        findings[0].message.contains("opens"),
        "{}",
        findings[0].message
    );
    assert!(
        findings[0].message.contains("b's opens"),
        "{}",
        findings[0].message
    );
    assert!(
        findings[0].message.contains("Set b's opens to opens"),
        "{}",
        findings[0].message
    );
}

#[test]
fn push_to_a_copied_field_and_never_writing_back_warns() {
    let findings = copy_lint_findings(
        "## To useOpens (b: Box) -> Int:\n\
        \x20   Let opens be b's opens.\n\
        \x20   Push 1 to opens.\n\
        \x20   Return 0.\n",
    );
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0].message.contains("b's opens"),
        "{}",
        findings[0].message
    );
}

#[test]
fn set_item_of_a_copied_field_and_never_writing_back_warns() {
    let findings = copy_lint_findings(
        "## To useOpens (b: Box) -> Int:\n\
        \x20   Let opens be b's opens.\n\
        \x20   Set item 1 of opens to 5.\n\
        \x20   Return 0.\n",
    );
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0].message.contains("b's opens"),
        "{}",
        findings[0].message
    );
}

#[test]
fn a_loop_that_re_copies_a_field_every_iteration_without_writing_back_warns() {
    let findings = copy_lint_findings(
        "## To drainAll (b: Box, xs: Seq of Int) -> Int:\n\
        \x20   Repeat for x in xs:\n\
        \x20       Let opens be b's opens.\n\
        \x20       Pop from opens into at.\n\
        \x20   Return 0.\n",
    );
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0].message.contains("b's opens"),
        "{}",
        findings[0].message
    );
}

#[test]
fn a_field_of_a_field_copied_and_mutated_names_the_whole_chain() {
    let findings = copy_lint_findings(
        "## To useNested (a: Box) -> Int:\n\
        \x20   Let c be a's b's c.\n\
        \x20   Pop from c into at.\n\
        \x20   Return at.\n",
    );
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0].message.contains("a's b's c"),
        "{}",
        findings[0].message
    );
    assert!(
        findings[0].message.contains("Set a's b's c to c"),
        "{}",
        findings[0].message
    );
}

#[test]
fn a_list_copied_from_another_local_not_from_a_field_warns() {
    let findings = copy_lint_findings(
        "## To drainInto (src: Seq of Int) -> Int:\n\
        \x20   Let dst be src.\n\
        \x20   Pop from dst into at.\n\
        \x20   Return at.\n",
    );
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0].message.contains("dst"),
        "{}",
        findings[0].message
    );
    assert!(
        findings[0].message.contains("Set src to dst"),
        "{}",
        findings[0].message
    );
    assert!(
        !findings[0].message.contains("'s "),
        "not a field copy: {}",
        findings[0].message
    );
}

#[test]
fn reading_the_copy_after_its_last_mutation_is_legitimate_scratch_use() {
    let findings = copy_lint_findings(
        "## To useOpens (b: Box) -> Int:\n\
        \x20   Let opens be b's opens.\n\
        \x20   Pop from opens into at.\n\
        \x20   Show opens.\n\
        \x20   Return at.\n",
    );
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn writing_the_copy_back_to_its_field_silences_the_lint() {
    let findings = copy_lint_findings(
        "## To useOpens (b: Box) -> Int:\n\
        \x20   Let opens be b's opens.\n\
        \x20   Pop from opens into at.\n\
        \x20   Set b's opens to opens.\n\
        \x20   Return at.\n",
    );
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn readdressing_the_source_field_with_an_unrelated_value_still_silences_the_lint() {
    // Distinct from `writing_the_copy_back_to_its_field_silences_the_lint`: the value assigned
    // here is NOT the tracked local itself, so the generic read walk over `value` never touches
    // `opens` — only the write-back address match (`(object, field)` against the binding's own
    // `Field` source) can silence this one.
    let findings = copy_lint_findings(
        "## To useOpens (b: Box) -> Int:\n\
        \x20   Let opens be b's opens.\n\
        \x20   Pop from opens into at.\n\
        \x20   Set b's opens to 0.\n\
        \x20   Return at.\n",
    );
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn readdressing_the_source_local_with_an_unrelated_value_still_silences_the_lint() {
    // The `Local`-source twin of the test above.
    let findings = copy_lint_findings(
        "## To drainInto (src: Seq of Int) -> Int:\n\
        \x20   Let dst be src.\n\
        \x20   Pop from dst into at.\n\
        \x20   Set src to 0.\n\
        \x20   Return at.\n",
    );
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn writing_the_copy_back_to_its_local_source_silences_the_lint() {
    let findings = copy_lint_findings(
        "## To drainInto (src: Seq of Int) -> Int:\n\
        \x20   Let dst be src.\n\
        \x20   Pop from dst into at.\n\
        \x20   Set src to dst.\n\
        \x20   Return at.\n",
    );
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn mutating_the_field_directly_with_no_copy_at_all_never_warns() {
    let findings = copy_lint_findings(
        "## To useOpens (b: Box) -> Int:\n\
        \x20   Pop from b's opens into at.\n\
        \x20   Return at.\n",
    );
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn a_fresh_collection_with_no_source_to_write_back_to_never_warns() {
    let findings = copy_lint_findings(
        "## To scratch () -> Int:\n\
        \x20   Let xs be a new Seq of Int.\n\
        \x20   Push 1 to xs.\n\
        \x20   Return 0.\n",
    );
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn a_local_declared_once_and_never_mutated_never_warns() {
    let findings = copy_lint_findings(
        "## To readOnly (b: Box) -> Int:\n\
        \x20   Let opens be b's opens.\n\
        \x20   Show opens.\n\
        \x20   Return 0.\n",
    );
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn rebinding_the_local_before_any_read_still_warns_about_the_first_copy() {
    let findings = copy_lint_findings(
        "## To useOpens (b: Box, c: Box) -> Int:\n\
        \x20   Let opens be b's opens.\n\
        \x20   Pop from opens into at.\n\
        \x20   Let opens be c's opens.\n\
        \x20   Show opens.\n\
        \x20   Return at.\n",
    );
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0].message.contains("b's opens"),
        "{}",
        findings[0].message
    );
}

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
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    let mut lexer = Lexer::new("a b c d e", &mut interner);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ctx,
        crate::analysis::TypeRegistry::default(),
    );

    let initial_pos = parser.current;
    let initial_var_counter = parser.var_counter;
    let initial_bindings_len = parser.donkey_bindings.len();
    let initial_island = parser.current_island;
    let initial_time = parser.pending_time;

    {
        let mut guard = parser.guard();
        guard.current = 3;
        guard.var_counter = 99;
        guard
            .donkey_bindings
            .push((Symbol::EMPTY, Symbol::EMPTY, false, false));
        guard.current_island = 42;
        guard.pending_time = Some(Time::Past);
    }

    assert_eq!(parser.current, initial_pos, "position not restored");
    assert_eq!(
        parser.var_counter, initial_var_counter,
        "var_counter not restored"
    );
    assert_eq!(
        parser.donkey_bindings.len(),
        initial_bindings_len,
        "bindings not restored"
    );
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
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    let mut lexer = Lexer::new("a b c", &mut interner);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ctx,
        crate::analysis::TypeRegistry::default(),
    );

    {
        let mut guard = parser.guard();
        guard.current = 2;
        guard.var_counter = 50;
        guard.commit();
    }

    assert_eq!(
        parser.current, 2,
        "position should be preserved after commit"
    );
    assert_eq!(
        parser.var_counter, 50,
        "var_counter should be preserved after commit"
    );
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
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    let mut lexer = Lexer::new("who what where", &mut interner);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ctx,
        crate::analysis::TypeRegistry::default(),
    );

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
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    let mut lexer = Lexer::new("if then", &mut interner);
    let tokens = lexer.tokenize();
    let parser = Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ctx,
        crate::analysis::TypeRegistry::default(),
    );

    assert!(!parser.check_any(TokenType::WH_WORDS));
    assert!(!parser.check_any(TokenType::MODALS));
}
