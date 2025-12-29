//! Phase 34: The Adjective System (User-Defined Generics)
//!
//! Tests for generic struct/enum definitions, type parameters, and instantiation.

use logos::compile::compile_to_rust;
use logos::Lexer;
use logos::intern::Interner;
use logos::analysis::discovery::DiscoveryPass;
use logos::analysis::registry::TypeDef;
use logos::mwe;

fn make_tokens(source: &str, interner: &mut Interner) -> Vec<logos::token::Token> {
    let mut lexer = Lexer::new(source, interner);
    let tokens = lexer.tokenize();
    let mwe_trie = mwe::build_mwe_trie();
    mwe::apply_mwe_pipeline(tokens, &mwe_trie, interner)
}

#[test]
fn test_generic_struct_discovered() {
    let source = r#"
## Definition
A Box of [T] has:
    a value, which is T.

## Main
Return.
"#;
    let mut interner = Interner::new();
    let tokens = make_tokens(source, &mut interner);

    // Debug: print tokens
    eprintln!("Tokens:");
    for (i, tok) in tokens.iter().enumerate() {
        eprintln!("{}: {:?} ({})", i, tok.kind, interner.resolve(tok.lexeme));
    }

    let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
    let registry = discovery.run();

    let box_sym = interner.intern("Box");
    assert!(registry.is_type(box_sym), "Box should be registered as a type");

    if let Some(TypeDef::Struct { fields, generics, .. }) = registry.get(box_sym) {
        eprintln!("Box generics: {:?}", generics.iter().map(|s| interner.resolve(*s)).collect::<Vec<_>>());
        eprintln!("Box fields: {:?}", fields);
        assert_eq!(generics.len(), 1, "Box should have 1 type parameter");
        assert_eq!(fields.len(), 1, "Box should have 1 field");
    } else {
        panic!("Box should be a Struct with generics, got: {:?}", registry.get(box_sym));
    }
}

#[test]
fn test_generic_struct_codegen() {
    let source = r#"
## Definition
A Box of [T] has:
    a value, which is T.

## Main
Return.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("struct Box<T>"), "Should have generic parameter: {}", rust);
    assert!(rust.contains("value: T"), "Field should use type parameter: {}", rust);
}

#[test]
fn test_two_param_generic() {
    let source = r#"
## Definition
A Pair of [A] and [B] has:
    a first, which is A.
    a second, which is B.

## Main
Return.
"#;

    let mut interner = Interner::new();
    let tokens = make_tokens(source, &mut interner);

    // Debug: print tokens
    eprintln!("Tokens for Pair:");
    for (i, tok) in tokens.iter().enumerate() {
        eprintln!("{}: {:?} ({})", i, tok.kind, interner.resolve(tok.lexeme));
    }

    let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
    let registry = discovery.run();

    let pair_sym = interner.intern("Pair");
    if let Some(TypeDef::Struct { fields, generics, .. }) = registry.get(pair_sym) {
        eprintln!("Pair generics: {:?}", generics.iter().map(|s| (s, interner.resolve(*s))).collect::<Vec<_>>());
        for field in fields {
            eprintln!("Field {}: {:?}", interner.resolve(field.name), field.ty);
        }
    }

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("struct Pair<A, B>"), "Should have two type parameters: {}", rust);
    assert!(rust.contains("first: A"), "First field should use A: {}", rust);
    assert!(rust.contains("second: B"), "Second field should use B: {}", rust);
}

#[test]
fn test_three_param_generic() {
    let source = r#"
## Definition
A Triplet of [A] and [B] and [C] has:
    a first, which is A.
    a second, which is B.
    a third, which is C.

## Main
Return.
"#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("struct Triplet<A, B, C>"), "Should have three type parameters: {}", rust);
    assert!(rust.contains("first: A"), "First field should use A: {}", rust);
    assert!(rust.contains("second: B"), "Second field should use B: {}", rust);
    assert!(rust.contains("third: C"), "Third field should use C: {}", rust);
}

#[test]
fn test_generic_with_builtin() {
    let source = r#"
## Definition
A Container of [T] has:
    a items, which is List of T.

## Main
Return.
"#;

    let mut interner = Interner::new();
    let tokens = make_tokens(source, &mut interner);

    // Debug: print tokens
    eprintln!("Tokens for Container:");
    for (i, tok) in tokens.iter().enumerate() {
        eprintln!("{}: {:?} ({})", i, tok.kind, interner.resolve(tok.lexeme));
    }

    let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
    let registry = discovery.run();

    let container_sym = interner.intern("Container");
    eprintln!("Container registered: {:?}", registry.get(container_sym));

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("struct Container<T>"), "Should have generic parameter: {}", rust);
    assert!(rust.contains("items: Vec<T>"), "Field should be Vec<T>: {}", rust);
}

#[test]
fn test_generic_instantiation() {
    let source = r#"
## Definition
A Box of [T] has:
    a value, which is T.

## Main
Let b be a new Box of Int.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("Box::<i64>::default()"), "Should instantiate with turbofish: {}", rust);
}

#[test]
fn test_generic_enum() {
    let source = r#"
## Definition
A Maybe of [T] is either:
    A Some with a value, which is T.
    A None.

## Main
Return.
"#;

    let mut interner = Interner::new();
    let tokens = make_tokens(source, &mut interner);

    // Debug: print tokens
    eprintln!("Tokens for Maybe:");
    for (i, tok) in tokens.iter().enumerate() {
        eprintln!("{}: {:?} ({})", i, tok.kind, interner.resolve(tok.lexeme));
    }

    let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
    let registry = discovery.run();

    let maybe_sym = interner.intern("Maybe");
    eprintln!("Maybe registered: {:?}", registry.get(maybe_sym));

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("enum Maybe<T>"), "Should have generic enum: {}", rust);
    assert!(rust.contains("Some { value: T }"), "Some should have T field: {}", rust);
    assert!(rust.contains("None,") || rust.contains("None }"), "Should have None variant: {}", rust);
}

#[test]
fn test_generic_result_pattern() {
    let source = r#"
## Definition
A Outcome of [S] and [E] is either:
    A Success with a value, which is S.
    A Failure with an error, which is E.

## Main
Return.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("enum Outcome<S, E>"), "Should have two type params: {}", rust);
    assert!(rust.contains("Success { value: S }"), "Success should use S: {}", rust);
    assert!(rust.contains("Failure { error: E }"), "Failure should use E: {}", rust);
}
