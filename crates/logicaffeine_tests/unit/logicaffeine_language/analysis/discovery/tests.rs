use super::*;
use crate::mwe;
use crate::Lexer;

fn make_tokens(source: &str, interner: &mut Interner) -> Vec<Token> {
    let mut lexer = Lexer::new(source, interner);
    let tokens = lexer.tokenize();
    let mwe_trie = mwe::build_mwe_trie();
    mwe::apply_mwe_pipeline(tokens, &mwe_trie, interner)
}

#[test]
fn discovery_finds_generic_in_definition_block() {
    let source = "## Definition\nA Stack is a generic collection.";
    let mut interner = Interner::new();
    let tokens = make_tokens(source, &mut interner);

    let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
    let registry = discovery.run();

    let stack = interner.intern("Stack");
    assert!(
        registry.is_generic(stack),
        "Stack should be discovered as generic"
    );
}

#[test]
fn discovery_parses_struct_with_fields() {
    let source = r#"## Definition
A Point has:
    an x, which is Int.
    a y, which is Int.
"#;
    let mut interner = Interner::new();
    let tokens = make_tokens(source, &mut interner);

    let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
    let registry = discovery.run();

    let point = interner.intern("Point");
    assert!(registry.is_type(point), "Point should be registered");

    if let Some(TypeDef::Struct {
        fields, generics, ..
    }) = registry.get(point)
    {
        assert_eq!(
            fields.len(),
            2,
            "Point should have 2 fields, got {:?}",
            fields
        );
        assert_eq!(interner.resolve(fields[0].name), "x");
        assert_eq!(interner.resolve(fields[1].name), "y");
        assert!(generics.is_empty(), "Point should have no generics");
    } else {
        panic!("Point should be a struct with fields");
    }
}

#[test]
fn discovery_works_with_markdown_header() {
    // Phase 36: LOGOS files have `# Header` before `## Definition`
    let source = r#"# Geometry

## Definition
A Point has:
    an x, which is Int.
"#;
    let mut interner = Interner::new();
    let tokens = make_tokens(source, &mut interner);

    // Debug: print tokens to see what we're getting
    for (i, tok) in tokens.iter().enumerate() {
        eprintln!("Token {}: {:?}", i, tok.kind);
    }

    let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
    let registry = discovery.run();
    let point = interner.intern("Point");
    assert!(
        registry.is_type(point),
        "Point should be discovered even with # header"
    );
}

#[test]
fn discovery_parses_portable_enum() {
    let source = r#"## Definition
A Command is Portable and is either:
    a Start.
    a Stop.
    a Pause.
"#;
    let mut interner = Interner::new();
    let tokens = make_tokens(source, &mut interner);

    // Debug: print tokens to see what we're getting
    eprintln!("Tokens for portable enum:");
    for (i, tok) in tokens.iter().enumerate() {
        eprintln!(
            "Token {}: {:?} ({})",
            i,
            tok.kind,
            interner.resolve(tok.lexeme)
        );
    }

    let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
    let registry = discovery.run();

    let command = interner.intern("Command");
    assert!(
        registry.is_type(command),
        "Command should be registered as type"
    );

    if let Some(TypeDef::Enum {
        variants,
        is_portable,
        ..
    }) = registry.get(command)
    {
        eprintln!("Command is_portable: {}", is_portable);
        eprintln!(
            "Variants: {:?}",
            variants
                .iter()
                .map(|v| interner.resolve(v.name))
                .collect::<Vec<_>>()
        );
        assert!(*is_portable, "Command should be portable");
        assert_eq!(variants.len(), 3, "Command should have 3 variants");
    } else {
        panic!(
            "Command should be an enum, got: {:?}",
            registry.get(command)
        );
    }
}

#[test]
fn discovery_parses_lww_int_field() {
    let source = r#"## Definition
A Setting is Shared and has:
    a volume, which is LastWriteWins of Int.
"#;
    let mut interner = Interner::new();
    let tokens = make_tokens(source, &mut interner);

    // Debug: print tokens
    eprintln!("Tokens for LWW of Int:");
    for (i, tok) in tokens.iter().enumerate() {
        eprintln!("{:3}: {:?} ({})", i, tok.kind, interner.resolve(tok.lexeme));
    }

    let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
    let registry = discovery.run();

    let setting = interner.intern("Setting");
    assert!(registry.is_type(setting), "Setting should be registered");

    if let Some(TypeDef::Struct {
        fields, is_shared, ..
    }) = registry.get(setting)
    {
        eprintln!("is_shared: {}", is_shared);
        eprintln!("Fields: {:?}", fields.len());
        for f in fields {
            eprintln!("  field: {} = {:?}", interner.resolve(f.name), f.ty);
        }
        assert!(*is_shared, "Setting should be shared");
        assert_eq!(fields.len(), 1, "Setting should have 1 field");
    } else {
        panic!(
            "Setting should be a struct, got: {:?}",
            registry.get(setting)
        );
    }
}
