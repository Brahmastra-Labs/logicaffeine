//! Phase 34: The Adjective System (User-Defined Generics)
//!
//! Tests for generic struct/enum definitions, type parameters, and instantiation.

use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_compile::compile::compile_to_rust;
use logicaffeine_language::analysis::{DiscoveryPass, TypeDef};
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::{Expr, LogicExpr, NounPhrase, Stmt, Term, ThematicRole, TypeExpr};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::mwe;
use logicaffeine_language::Lexer;
use logicaffeine_language::Parser;

fn make_tokens(source: &str, interner: &mut Interner) -> Vec<logicaffeine_language::token::Token> {
    let mut lexer = Lexer::new(source, interner);
    let tokens = lexer.tokenize();
    let mwe_trie = mwe::build_mwe_trie();
    mwe::apply_mwe_pipeline(tokens, &mwe_trie, interner)
}

/// The type-application half of this same phase: a user-declared generic struct/enum used as a
/// function's parameter or return type, not only as another struct's field type (this file's own
/// `test_generic_struct_discovered` and friends, which read the registry `discovery.rs`'s inline
/// field parser already resolves against). Parses `body` after [`GENERIC_DEFS`] with a REAL
/// pre-pass registry, exactly as `ui_bridge.rs`'s production pipeline builds one before the main
/// parse — `Parser::get_generic_param_count` (`parser/mod.rs`) used to answer only
/// `TypeDef::Generic` (a BUILT-IN parametric type's own shape) and fall through to `None` for a
/// user's `Struct`/`Enum` with a non-empty `generics` list, so `Box of T`/`Outcome of Int and
/// Text` in a signature never consumed the `of …` clause at all — a parse error, not a silent
/// misparse.
const GENERIC_DEFS: &str = "\
## Definition
A Box of [T] has:
    a value, which is T.

An Outcome of [S] and [E] is either:
    A Success with a value, which is S.
    A Failure with an error, which is E.

";

fn parse_type_app(body: &str) -> Vec<Stmt<'static>> {
    let source: &'static str = Box::leak(format!("{GENERIC_DEFS}{body}").into_boxed_str());
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
    let mut interner_owned = Interner::new();
    let tokens = make_tokens(source, &mut interner_owned);
    *interner = interner_owned;

    let mut discovery = DiscoveryPass::new(&tokens, interner);
    let type_registry = discovery.run();
    assert!(
        matches!(type_registry.get(interner.intern("Box")), Some(TypeDef::Struct { generics, .. }) if !generics.is_empty()),
        "fixture bug: Box must be discovered as a generic struct"
    );
    assert!(
        matches!(type_registry.get(interner.intern("Outcome")), Some(TypeDef::Enum { generics, .. }) if generics.len() == 2),
        "fixture bug: Outcome must be discovered as a two-param generic enum"
    );

    let mut parser = Parser::new(tokens, world_state, interner, ctx, type_registry);
    parser
        .parse_program()
        .expect("a user-declared generic type must parse as a type application")
}

fn first_fn_sig(
    stmts: &'static [Stmt<'static>],
) -> (
    &'static [(Symbol, &'static TypeExpr<'static>)],
    &'static TypeExpr<'static>,
) {
    for s in stmts {
        if let Stmt::FunctionDef {
            params,
            return_type,
            ..
        } = s
        {
            return (
                params,
                return_type.expect("the function declares a return type"),
            );
        }
    }
    panic!("no function def parsed");
}

#[test]
fn a_user_generic_struct_parses_as_a_type_application_in_a_parameter() {
    let stmts: &'static [Stmt<'static>] = Box::leak(
        parse_type_app("## To wrap (b: Box of Int) -> Int:\n    Return 0.\n").into_boxed_slice(),
    );
    let (params, _) = first_fn_sig(stmts);
    match params[0].1 {
        TypeExpr::Generic { base, params: args } => {
            assert_eq!(
                args.len(),
                1,
                "Box takes exactly one type argument: {args:?}"
            );
            assert!(
                matches!(args[0], TypeExpr::Primitive(_)),
                "Box of Int: {args:?}"
            );
            let _ = base;
        }
        other => panic!("`Box of Int` must parse as Generic{{Box, [Int]}}, got {other:?}"),
    }
}

#[test]
fn a_user_generic_struct_parses_as_a_type_application_in_a_return_type() {
    let stmts: &'static [Stmt<'static>] = Box::leak(
        parse_type_app("## To make () -> Box of Int:\n    Return 0.\n").into_boxed_slice(),
    );
    let (_, ret) = first_fn_sig(stmts);
    assert!(
        matches!(ret, TypeExpr::Generic { params, .. } if params.len() == 1),
        "`-> Box of Int` must parse as Generic{{Box, [Int]}}, got {ret:?}"
    );
}

#[test]
fn a_user_generic_struct_nests_inside_a_builtin_generic() {
    let stmts: &'static [Stmt<'static>] = Box::leak(
        parse_type_app("## To flatten (xs: Seq of Box of Int) -> Int:\n    Return 0.\n")
            .into_boxed_slice(),
    );
    let (params, _) = first_fn_sig(stmts);
    match params[0].1 {
        TypeExpr::Generic { base: seq, params: seq_args } => {
            assert_eq!(seq_args.len(), 1, "Seq takes exactly one type argument: {seq_args:?}");
            match &seq_args[0] {
                TypeExpr::Generic { params: box_args, .. } => {
                    assert_eq!(box_args.len(), 1, "the nested Box of Int: {box_args:?}");
                }
                other => panic!(
                    "`Seq of Box of Int`'s element must itself be Generic{{Box, [Int]}}, got {other:?}"
                ),
            }
            let _ = seq;
        }
        other => panic!(
            "`Seq of Box of Int` must parse as Generic{{Seq, [Generic{{Box,[Int]}}]}}, got {other:?}"
        ),
    }
}

#[test]
fn a_user_generic_enum_takes_two_type_arguments() {
    let stmts: &'static [Stmt<'static>] = Box::leak(
        parse_type_app("## To try () -> Outcome of Int and Text:\n    Return 0.\n")
            .into_boxed_slice(),
    );
    let (_, ret) = first_fn_sig(stmts);
    match ret {
        TypeExpr::Generic { params: args, .. } => {
            assert_eq!(args.len(), 2, "Outcome takes exactly two type arguments: {args:?}");
            assert!(matches!(args[0], TypeExpr::Primitive(_)), "Outcome's first argument, Int: {args:?}");
            assert!(matches!(args[1], TypeExpr::Primitive(_)), "Outcome's second argument, Text: {args:?}");
        }
        other => panic!("`Outcome of Int and Text` must parse as Generic{{Outcome, [Int, Text]}}, got {other:?}"),
    }
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
    assert!(
        registry.is_type(box_sym),
        "Box should be registered as a type"
    );

    if let Some(TypeDef::Struct {
        fields, generics, ..
    }) = registry.get(box_sym)
    {
        eprintln!(
            "Box generics: {:?}",
            generics
                .iter()
                .map(|s| interner.resolve(*s))
                .collect::<Vec<_>>()
        );
        eprintln!("Box fields: {:?}", fields);
        assert_eq!(generics.len(), 1, "Box should have 1 type parameter");
        assert_eq!(fields.len(), 1, "Box should have 1 field");
    } else {
        panic!(
            "Box should be a Struct with generics, got: {:?}",
            registry.get(box_sym)
        );
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
    assert!(
        rust.contains("struct Box<T>"),
        "Should have generic parameter: {}",
        rust
    );
    assert!(
        rust.contains("value: T"),
        "Field should use type parameter: {}",
        rust
    );
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
    if let Some(TypeDef::Struct {
        fields, generics, ..
    }) = registry.get(pair_sym)
    {
        eprintln!(
            "Pair generics: {:?}",
            generics
                .iter()
                .map(|s| (s, interner.resolve(*s)))
                .collect::<Vec<_>>()
        );
        for field in fields {
            eprintln!("Field {}: {:?}", interner.resolve(field.name), field.ty);
        }
    }

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("struct Pair<A, B>"),
        "Should have two type parameters: {}",
        rust
    );
    assert!(
        rust.contains("first: A"),
        "First field should use A: {}",
        rust
    );
    assert!(
        rust.contains("second: B"),
        "Second field should use B: {}",
        rust
    );
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
    assert!(
        rust.contains("struct Triplet<A, B, C>"),
        "Should have three type parameters: {}",
        rust
    );
    assert!(
        rust.contains("first: A"),
        "First field should use A: {}",
        rust
    );
    assert!(
        rust.contains("second: B"),
        "Second field should use B: {}",
        rust
    );
    assert!(
        rust.contains("third: C"),
        "Third field should use C: {}",
        rust
    );
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
    assert!(
        rust.contains("struct Container<T>"),
        "Should have generic parameter: {}",
        rust
    );
    assert!(
        rust.contains("items: LogosSeq<T>"),
        "Field should be LogosSeq<T>: {}",
        rust
    );
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
    assert!(
        rust.contains("Box::<i64>::default()"),
        "Should instantiate with turbofish: {}",
        rust
    );
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
    assert!(
        rust.contains("enum Maybe<T>"),
        "Should have generic enum: {}",
        rust
    );
    assert!(
        rust.contains("Some { value: T }"),
        "Some should have T field: {}",
        rust
    );
    assert!(
        rust.contains("None,") || rust.contains("None }"),
        "Should have None variant: {}",
        rust
    );
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
    assert!(
        rust.contains("enum Outcome<S, E>"),
        "Should have two type params: {}",
        rust
    );
    assert!(
        rust.contains("Success { value: S }"),
        "Success should use S: {}",
        rust
    );
    assert!(
        rust.contains("Failure { error: E }"),
        "Failure should use E: {}",
        rust
    );
}

fn field_names(source: &str, type_name: &str) -> Vec<String> {
    let mut interner = Interner::new();
    let tokens = make_tokens(source, &mut interner);
    let registry = DiscoveryPass::new(&tokens, &mut interner).run();
    let sym = interner.intern(type_name);
    match registry.get(sym) {
        Some(TypeDef::Struct { fields, .. }) => fields
            .iter()
            .map(|f| interner.resolve(f.name).to_string())
            .collect(),
        other => panic!("{type_name} is not a struct: {other:?}"),
    }
}

/// `name: Type.` names its field by POSITION — the word before the colon — whatever part of
/// speech the lexer gave it. `first` is an ordinal to the lexer, and the field vanished: no
/// error at the declaration, only `'Pair' has no field named 'first'` at the first read.
#[test]
fn concise_fields_named_by_ordinals_are_fields() {
    let source = "## A Podium has:\n    first: Text.\n    second: Text.\n    third: Text.\n\n## Main\nReturn.\n";
    assert_eq!(field_names(source, "Podium"), ["first", "second", "third"]);
}

#[test]
fn concise_two_param_generic_keeps_every_field() {
    let source =
        "## A Pair of [A] and [B] has:\n    first: A.\n    second: B.\n\n## Main\nReturn.\n";
    assert_eq!(field_names(source, "Pair"), ["first", "second"]);
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("struct Pair<A, B>"),
        "Should have two type parameters: {}",
        rust
    );
    assert!(rust.contains("first: A"), "first should use A: {}", rust);
    assert!(rust.contains("second: B"), "second should use B: {}", rust);
}

#[test]
fn a_two_param_generic_is_built_and_read_through_both_fields() {
    let source = "## A Pair of [A] and [B] has:\n    first: A.\n    second: B.\n\n## Main\n\
                  Let entry be a new Pair of Text and Int with first \"score\" and second 97.\n\
                  Show \"Pair: {entry's first} = {entry's second}.\".\n";
    let rust = compile_to_rust(source).expect("a Pair built with both its fields compiles");
    assert!(rust.contains("first"), "{}", rust);
}
