//! LOCK — every `##` block header whose own grammar does not already read a trailing `:`
//! accepts one anyway, silently dropped, so `## Component Counter:` is exactly LGX (item 3 of
//! the LGX-ease campaign's legacy-dialect removal: the colon used to select the retired
//! reactive dialect; now it is simply optional punctuation nobody has to remember).
//!
//! The rule is scoped to the IMPERATIVE-mode headers (`classify_with_lookahead`'s own mode
//! switch): a stray `:` is a real `TokenType::Colon` on every header regardless of mode, but
//! only an imperative body's statement list is strict enough to choke on one — a declarative/
//! prose section (`## Note`, `## Theorem`, …) never inspects its own trailing punctuation this
//! closely, so the ambiguity item 3 fixes does not exist there. Of the ten imperative headers,
//! `Function`/`Directive`/`Route`/`Middleware` already read a MANDATORY colon as part of their
//! own signature and must keep requiring it; `Test` gives its colon a different, load-bearing
//! meaning (the raw prose name that follows it) and must keep that. The remaining five —
//! `Main`, `Component`, `Page`, `Store`, `ErrorPage` — take the new rule.

use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::analysis::{DiscoveryPass, TypeRegistry};
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::{Expr, LogicExpr, NounPhrase, Stmt, Term, ThematicRole, TypeExpr};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::error::ParseError;
use logicaffeine_language::{Lexer, Parser};

fn parse(src: &str) -> Result<Vec<Stmt<'static>>, ParseError> {
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
    parser.parse_program()
}

fn accepted(src: &str) -> Vec<Stmt<'static>> {
    parse(src).unwrap_or_else(|e| {
        panic!(
            "this program must parse cleanly, and was refused with {:?}:\n{src}",
            e.kind
        )
    })
}

fn refused(src: &str) -> ParseError {
    match parse(src) {
        Ok(_) => panic!("this program must be refused (its own signature needs a colon this rule must not supply):\n{src}"),
        Err(e) => e,
    }
}

// ---- The five headers that gain the optional colon. ----

/// Both sides are compared after blanking `:`/no-`:` to the SAME single space, so a trailing
/// colon and no trailing colon produce byte-IDENTICAL source (and therefore identical spans) —
/// the honest way to assert "parses exactly like LGX" without the comparison itself tripping
/// over a one-byte length difference that has nothing to do with the feature.
fn assert_colon_is_invisible(with_colon: &str, without_colon_same_length: &str) {
    assert_eq!(
        with_colon.len(),
        without_colon_same_length.len(),
        "test bug: the two fixtures must be the same length"
    );
    assert_eq!(
        format!("{:?}", accepted(with_colon)),
        format!("{:?}", accepted(without_colon_same_length)),
        "a trailing `:` must parse identically to a space in its place"
    );
}

#[test]
fn a_component_with_a_trailing_colon_parses_exactly_like_lgx() {
    assert_colon_is_invisible(
        "## Component Counter:\nView:\n    <p>hi</p>\n",
        "## Component Counter \nView:\n    <p>hi</p>\n",
    );
}

#[test]
fn a_components_own_typed_prop_colon_survives_a_trailing_header_colon() {
    // The header's outer `:` must be the one dropped — the prop's OWN `label: Text` ascription,
    // which also ends in a colon, is a completely different token and must be untouched.
    assert_colon_is_invisible(
        "## Component Pick (label: Text):\nView:\n    <p>{label}</p>\n",
        "## Component Pick (label: Text) \nView:\n    <p>{label}</p>\n",
    );
}

#[test]
fn a_store_with_a_trailing_colon_parses() {
    assert_colon_is_invisible(
        "## Store Ledger:\nState:\n    Let total be 0.\n",
        "## Store Ledger \nState:\n    Let total be 0.\n",
    );
}

#[test]
fn a_page_with_a_trailing_colon_parses() {
    assert_colon_is_invisible(
        "## Page /home:\nView:\n    <p>hi</p>\n",
        "## Page /home \nView:\n    <p>hi</p>\n",
    );
}

#[test]
fn an_error_page_with_a_trailing_colon_parses() {
    assert_colon_is_invisible(
        "## Error page 404:\nView:\n    <p>not found</p>\n",
        "## Error page 404 \nView:\n    <p>not found</p>\n",
    );
}

#[test]
fn a_main_with_a_trailing_colon_parses() {
    assert_colon_is_invisible("## Main:\nShow \"hi\".\n", "## Main \nShow \"hi\".\n");
}

// ---- Headers that already read a colon of their own keep needing exactly that one. ----

#[test]
fn a_directive_still_requires_its_own_colon() {
    // Dropping the directive's OWN mandatory colon must still be refused — this rule adds
    // tolerance nowhere it was not already implicit, and must not make a required colon optional.
    let missing =
        "## Directive glow (el: Element) and (value: Text)\n    Set the text of el to value.\n";
    refused(missing);
    accepted(
        "## Directive glow (el: Element) and (value: Text):\n    Set the text of el to value.\n",
    );
}

#[test]
fn a_route_still_requires_its_own_colon() {
    let missing = "## Route GET /health -> Text\n    Return \"ok\".\n";
    refused(missing);
    accepted("## Route GET /health -> Text:\n    Return \"ok\".\n");
}

#[test]
fn middleware_still_requires_its_own_colon() {
    let missing = "## Before each request (r: Request) -> Option of Response\n    Return None.\n";
    refused(missing);
    accepted("## Before each request (r: Request) -> Option of Response:\n    Return None.\n");
}

#[test]
fn a_function_still_requires_its_own_colon() {
    let missing = "## To greet ()\n    Show \"hi\".\n";
    refused(missing);
    accepted("## To greet ():\n    Show \"hi\".\n");
}

/// `## Test:` gives its colon a different, load-bearing job (the raw prose name that follows
/// it): `Test` is not in the accepting set, so this colon is untouched by item 3, and the
/// header still parses into one `TestDef` exactly as it always did.
#[test]
fn a_test_headers_colon_is_unchanged_and_still_names_the_test() {
    let stmts = accepted("## Test: it adds\nExpect that 1 + 1 is 2.\n");
    assert_eq!(
        stmts.len(),
        1,
        "one `## Test:` header is one TestDef: {stmts:?}"
    );
    assert!(
        format!("{stmts:?}").starts_with("[TestDef"),
        "the `## Test:` colon must still separate the header from its prose name: {stmts:?}"
    );
}

// ---- The census: every accepting header is named in the source, by name. ----

/// `Lexer::blank_optional_header_colons`'s `ACCEPTING` list is the census: it names every header
/// that gets the rule, read from its own source rather than assumed, so a future edit that drops
/// one without touching a test is still visible here. The five behavioural tests above are the
/// enforcement; this locks the CHOICE.
#[test]
fn the_five_headers_that_gain_the_rule_are_named_in_the_source() {
    let lexer_rs = include_str!("../../logicaffeine_language/src/lexer.rs");
    let fn_start = lexer_rs
        .find("fn blank_optional_header_colons")
        .expect("Lexer::blank_optional_header_colons must exist");
    let fn_body = &lexer_rs[fn_start..];
    let fn_end = fn_body
        .find("\n    }")
        .map(|i| i + fn_start)
        .unwrap_or(lexer_rs.len());
    let body = &lexer_rs[fn_start..fn_end];
    for name in ["main", "component", "store", "page", "error"] {
        assert!(
            body.contains(&format!("\"{name}\"")),
            "`{name}` must be named in `blank_optional_header_colons`'s own source: {body}"
        );
    }
}
