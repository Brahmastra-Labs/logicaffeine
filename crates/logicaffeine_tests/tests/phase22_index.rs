use logicaffeine_compile::compile::compile_to_rust;
use logicaffeine_base::Interner;
use logicaffeine_language::{Lexer, ParseError, ParseErrorKind, TokenType};

#[test]
fn item_token_recognized() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("item 1 of list", &mut interner);
    let tokens = lexer.tokenize();

    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::Item)),
        "Should have Item token: {:?}",
        tokens
    );
}

#[test]
fn items_token_recognized() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("items 2 through 5 of list", &mut interner);
    let tokens = lexer.tokenize();

    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::Items)),
        "Should have Items token: {:?}",
        tokens
    );
}

#[test]
fn item_followed_by_zero_tokenizes() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("item 0 of list", &mut interner);
    let tokens = lexer.tokenize();

    let has_item = tokens.iter().any(|t| matches!(t.kind, TokenType::Item));
    assert!(has_item, "Should have Item token");

    let has_number = tokens.iter().any(|t| matches!(t.kind, TokenType::Number(_)));
    assert!(has_number, "Should have Number token for 0: {:?}", tokens);
}

#[test]
fn zero_index_error_exists() {
    let error = ParseError {
        kind: ParseErrorKind::ZeroIndex,
        span: logicaffeine_language::token::Span::new(0, 5),
    };
    assert!(matches!(error.kind, ParseErrorKind::ZeroIndex));
}

#[test]
fn item_1_lexer_valid() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("item 1 of list", &mut interner);
    let tokens = lexer.tokenize();

    let item_count = tokens.iter().filter(|t| matches!(t.kind, TokenType::Item)).count();
    assert_eq!(item_count, 1, "Should have 1 Item token");
}

#[test]
fn slice_parses() {
    let source = "## Main\nLet x be items 2 through 5 of list.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse 'items 2 through 5 of list': {:?}", result);
}

#[test]
fn slice_codegen() {
    let source = "## Main\nLet x be items 2 through 5 of list.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should compile: {:?}", result);
    let code = result.unwrap();
    // Phase 43D: Slice codegen now uses explicit conversion with dynamic expressions
    // "items 2 through 5" becomes &list[(2 - 1) as usize..5 as usize] = &list[1..5]
    assert!(code.contains("(2 - 1) as usize..5 as usize"), "Should have 1-indexed slice: {}", code);
}

#[test]
fn slice_rejects_zero_start() {
    let source = "## Main\nLet x be items 0 through 5 of list.";
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should reject 'items 0 through 5': {:?}", result);
}

#[test]
fn slice_rejects_zero_end() {
    let source = "## Main\nLet x be items 2 through 0 of list.";
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should reject 'items 2 through 0': {:?}", result);
}

// `item`/`items` is the indexing keyword in imperative code regardless of the
// index form (a VARIABLE index `item i of arr` is the whole point of indexing),
// and in declarative text only when an explicit index number follows. Declarative
// prose nouns ("the blue item", "the item made of gold") stay nouns so the head is
// not stranded after its adjective. Mode separates code from prose; the number is
// the finer signal within prose.
#[test]
fn item_keyword_gating_by_mode_and_context() {
    let cases = [
        ("## Main\nLet x be item i of arr.", true),
        ("## Main\nLet x be item 1 of arr.", true),
        ("item 1 of list", true),
        ("items 2 through 5 of list", true),
        ("The blue item is here.", false),
        ("the item made of gold", false),
        ("each item", false),
    ];
    for (src, want_keyword) in cases {
        let mut interner = Interner::new();
        let mut lexer = Lexer::new(src, &mut interner);
        let toks = lexer.tokenize();
        let has_keyword = toks
            .iter()
            .any(|t| matches!(t.kind, TokenType::Item | TokenType::Items));
        assert_eq!(has_keyword, want_keyword, "src={src:?} tokens={toks:?}");
    }
}

#[test]
fn variable_index_compiles_in_imperative() {
    let result = compile_to_rust("## Main\nLet x be item i of arr.");
    assert!(result.is_ok(), "variable index `item i of arr` should compile: {result:?}");
}
