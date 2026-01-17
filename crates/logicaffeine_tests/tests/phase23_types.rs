// Phase 23: Two-Pass Compilation - Type Disambiguation
//
// Implements TypeRegistry and DiscoveryPass to distinguish:
// - "Stack of Integers" → Generic Type instantiation
// - "Owner of House" → Possessive field access

use logicaffeine_language::analysis::{TypeRegistry, TypeDef, DiscoveryPass};
use logicaffeine_base::Interner;
use logicaffeine_language::Lexer;
use logicaffeine_language::mwe;

// =============================================================================
// Step 1: TypeRegistry Unit Tests
// =============================================================================

#[test]
fn type_registry_new() {
    let mut interner = Interner::new();
    let registry = TypeRegistry::new();
    let unknown = interner.intern("UnknownType");
    assert!(!registry.is_type(unknown), "New registry should have no types");
}

#[test]
fn type_registry_with_primitives_has_nat() {
    let mut interner = Interner::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let nat = interner.intern("Nat");
    assert!(registry.is_type(nat), "Nat should be a primitive type");
}

#[test]
fn type_registry_with_primitives_has_int() {
    let mut interner = Interner::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let int = interner.intern("Int");
    assert!(registry.is_type(int), "Int should be a primitive type");
}

#[test]
fn type_registry_with_primitives_has_text() {
    let mut interner = Interner::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let text = interner.intern("Text");
    assert!(registry.is_type(text), "Text should be a primitive type");
}

#[test]
fn type_registry_with_primitives_has_bool() {
    let mut interner = Interner::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let bool_sym = interner.intern("Bool");
    assert!(registry.is_type(bool_sym), "Bool should be a primitive type");
}

#[test]
fn type_registry_register_type() {
    let mut interner = Interner::new();
    let mut registry = TypeRegistry::new();
    let stack = interner.intern("Stack");
    registry.register(stack, TypeDef::Generic { param_count: 1 });
    assert!(registry.is_type(stack), "Stack should be registered as a type");
}

#[test]
fn type_registry_is_generic() {
    let mut interner = Interner::new();
    let mut registry = TypeRegistry::new();

    let stack = interner.intern("Stack");
    let user = interner.intern("User");

    registry.register(stack, TypeDef::Generic { param_count: 1 });
    registry.register(user, TypeDef::Struct { fields: vec![], generics: vec![], is_portable: false, is_shared: false });

    assert!(registry.is_generic(stack), "Stack should be generic");
    assert!(!registry.is_generic(user), "User should not be generic");
}

#[test]
fn type_registry_get_returns_type_def() {
    let mut interner = Interner::new();
    let mut registry = TypeRegistry::new();

    let list = interner.intern("List");
    registry.register(list, TypeDef::Generic { param_count: 1 });

    let def = registry.get(list);
    assert!(matches!(def, Some(TypeDef::Generic { param_count: 1 })));
}

#[test]
fn type_registry_intrinsic_generics() {
    let mut interner = Interner::new();
    let registry = TypeRegistry::with_primitives(&mut interner);

    let list = interner.intern("List");
    let option = interner.intern("Option");
    let result = interner.intern("Result");

    assert!(registry.is_generic(list), "List should be intrinsic generic");
    assert!(registry.is_generic(option), "Option should be intrinsic generic");
    assert!(registry.is_generic(result), "Result should be intrinsic generic");
}

// =============================================================================
// Step 2: DiscoveryPass Unit Tests
// =============================================================================

fn make_tokens(source: &str, interner: &mut Interner) -> Vec<logicaffeine_language::Token> {
    let mut lexer = Lexer::new(source, interner);
    let tokens = lexer.tokenize();
    let mwe_trie = mwe::build_mwe_trie();
    mwe::apply_mwe_pipeline(tokens, &mwe_trie, interner)
}

#[test]
fn discovery_pass_returns_primitives() {
    let mut interner = Interner::new();
    let tokens = make_tokens("Some text here.", &mut interner);

    let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
    let registry = discovery.run();

    let nat = interner.intern("Nat");
    assert!(registry.is_type(nat), "Discovery should return primitives");
}

#[test]
fn discovery_pass_finds_generic_definition() {
    let source = "## Definition\nA Stack is a generic collection.";
    let mut interner = Interner::new();
    let tokens = make_tokens(source, &mut interner);

    let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
    let registry = discovery.run();

    let stack = interner.intern("Stack");
    assert!(registry.is_type(stack), "Stack should be discovered as type");
    assert!(registry.is_generic(stack), "Stack should be discovered as generic");
}

#[test]
fn discovery_pass_finds_struct_definition() {
    let source = "## Definition\nA User is a structure with fields.";
    let mut interner = Interner::new();
    let tokens = make_tokens(source, &mut interner);

    let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
    let registry = discovery.run();

    let user = interner.intern("User");
    assert!(registry.is_type(user), "User should be discovered as type");
    assert!(!registry.is_generic(user), "User should not be generic");
}

#[test]
fn discovery_pass_finds_enum_definition() {
    let source = "## Definition\nA Shape is an enum.";
    let mut interner = Interner::new();
    let tokens = make_tokens(source, &mut interner);

    let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
    let registry = discovery.run();

    let shape = interner.intern("Shape");
    assert!(registry.is_type(shape), "Shape should be discovered as type");
}

#[test]
fn discovery_pass_ignores_non_definition_blocks() {
    let source = "## Main\nA Stack is a generic collection.";
    let mut interner = Interner::new();
    let tokens = make_tokens(source, &mut interner);

    let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
    let registry = discovery.run();

    let stack = interner.intern("Stack");
    // Stack should NOT be discovered because it's in ## Main, not ## Definition
    assert!(!registry.is_type(stack), "Types in ## Main should not be discovered");
}

// =============================================================================
// Step 3: Integration Tests - Parser with TypeRegistry
// =============================================================================

use logicaffeine_language::Parser;
use logicaffeine_base::Arena;
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::NounPhrase;

#[test]
fn parser_has_type_registry() {
    let mut interner = Interner::new();
    let stack = interner.intern("Stack");

    let mut registry = TypeRegistry::new();
    registry.register(stack, TypeDef::Generic { param_count: 1 });

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();

    let tokens = make_tokens("the stack", &mut interner);
    let ast_ctx = AstContext::new(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    let parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, registry);
    assert!(parser.is_generic_type(stack), "Parser should know Stack is generic");
}

#[test]
fn parser_without_registry_returns_false() {
    let mut interner = Interner::new();
    let stack = interner.intern("Stack");

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();

    let tokens = make_tokens("the stack", &mut interner);
    let ast_ctx = AstContext::new(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    let parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, TypeRegistry::default());
    assert!(!parser.is_generic_type(stack), "Without registry, nothing is generic");
}

#[test]
fn intrinsic_list_is_generic() {
    let mut interner = Interner::new();
    let registry = TypeRegistry::with_primitives(&mut interner);

    let list = interner.intern("List");
    assert!(registry.is_generic(list), "List should be intrinsic generic");
}

#[test]
fn possessive_still_works_without_type() {
    // "owner of house" should still be parsed as possessive
    // because "owner" is not a known type
    let source = "the owner of the house";
    let mut interner = Interner::new();
    let registry = TypeRegistry::with_primitives(&mut interner);

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena: Arena<NounPhrase> = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();

    let tokens = make_tokens(source, &mut interner);
    let ast_ctx = AstContext::new(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, registry);

    // This should parse successfully with possessor
    use logicaffeine_language::parser::NounParsing;
    let np = parser.parse_noun_phrase(true);
    assert!(np.is_ok(), "Should parse possessive: {:?}", np);

    let np = np.unwrap();
    assert!(np.possessor.is_some(), "owner of house should have possessor");
}

