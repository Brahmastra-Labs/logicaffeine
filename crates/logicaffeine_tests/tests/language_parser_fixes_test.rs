//! Parser footgun fixes for the web-authoring layer (Wave 1).
//!
//! F0 — field access chained onto a call expression. `handle(x)'s field` and
//! `getValue(input)'s length` must parse to `FieldAccess { object: Call{..} }`,
//! not blow up because `parse_call_expr` returned early before the possessive
//! chain. Bare call statements (`greet("Alice").`) must still lower to
//! `Stmt::Call` — the terminating sentence `Period` is not a field-access `Dot`.
//!
//! F1 — `r"..."` raw strings are verbatim: braces are literal, never an
//! interpolation hole (so HTML/CSS/JSON with `{` survive intact). The
//! multi-line `"""..."""` string KEEPS interpolating — that is a distinct,
//! tested feature — and the ordinary `"..."` string is unchanged.
//!
//! Wave 4 — bare `count of xs` (no article) is refused, naming the canonical `length of xs`;
//! `the count of xs` (a length) and `the count of "sel"` (a page query) are unaffected.

use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::analysis::TypeRegistry;
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::{Expr, LogicExpr, NounPhrase, Stmt, Term, ThematicRole, TypeExpr};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::token::TokenType;
use logicaffeine_language::{Lexer, Parser};

/// Lex `src` and return the first string token as `(is_interpolated, content)`.
fn first_string_token(src: &str) -> (bool, String) {
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let toks = {
        let mut lexer = Lexer::new(src, interner);
        lexer.tokenize()
    };
    for t in &toks {
        match t.kind {
            TokenType::StringLiteral(s) => return (false, interner.resolve(s).to_string()),
            TokenType::InterpolatedString(s) => return (true, interner.resolve(s).to_string()),
            _ => {}
        }
    }
    panic!("no string token found in {src:?}");
}

#[test]
fn raw_string_keeps_braces_verbatim_and_does_not_interpolate() {
    let (interpolated, content) = first_string_token("## Main\n    Let s be r\"a {b} c\".\n");
    assert!(!interpolated, "r\"...\" must not interpolate");
    assert_eq!(content, "a {b} c", "raw braces must survive verbatim");
}

#[test]
fn raw_string_double_brace_stays_doubled() {
    // The `{{` → `{` collapse is interpolation-specific; a raw string is verbatim.
    let (interpolated, content) = first_string_token("## Main\n    Let s be r\"{{x}}\".\n");
    assert!(!interpolated);
    assert_eq!(
        content, "{{x}}",
        "raw strings do not collapse doubled braces"
    );
}

#[test]
fn ordinary_string_still_interpolates() {
    let (interpolated, _) = first_string_token("## Main\n    Let s be \"hi {name}\".\n");
    assert!(
        interpolated,
        "\"...\" with a brace hole must stay InterpolatedString"
    );
}

#[test]
fn triple_quote_still_interpolates() {
    // Regression guard: multi-line interpolation is a supported, tested feature.
    let (interpolated, _) =
        first_string_token("## Main\n    Let s be \"\"\"\n    hi {name}\n    \"\"\".\n");
    assert!(interpolated, "\"\"\"...\"\"\" must keep interpolating");
}

fn parse(input: &str) -> Vec<Stmt<'static>> {
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

    let mut lexer = Lexer::new(input, interner);
    let tokens = lexer.tokenize();
    let type_registry = TypeRegistry::default();
    let mut parser = Parser::new(tokens, world_state, interner, ctx, type_registry);
    parser.parse_program().expect("program should parse")
}

/// Like [`parse`], but hands back the parse error instead of panicking on one.
fn try_parse(input: &str) -> Result<Vec<Stmt<'static>>, logicaffeine_language::error::ParseError> {
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

    let mut lexer = Lexer::new(input, interner);
    let tokens = lexer.tokenize();
    let type_registry = TypeRegistry::default();
    let mut parser = Parser::new(tokens, world_state, interner, ctx, type_registry);
    parser.parse_program()
}

/// Return the `value` expression of the first `Let` among the parsed statements
/// (`## Main` statements land at the top level, not inside a `FunctionDef`).
fn first_let_value(stmts: &'static [Stmt<'static>]) -> &'static Expr<'static> {
    for s in stmts {
        if let Stmt::Let { value, .. } = s {
            return value;
        }
    }
    panic!("no let binding parsed");
}

#[test]
fn possessive_field_access_chains_onto_a_call() {
    let stmts = Box::leak(parse("## Main\n    Let v be handle(x)'s field.\n").into_boxed_slice());
    let value = first_let_value(stmts);
    match value {
        Expr::FieldAccess { object, .. } => assert!(
            matches!(object, Expr::Call { .. }),
            "expected FieldAccess wrapping a Call, got object {object:?}"
        ),
        other => panic!("expected FieldAccess {{ object: Call{{..}} }}, got {other:?}"),
    }
}

#[test]
fn ufcs_method_chains_onto_a_call() {
    // `foo(x).bar(y)` — the receiver `foo(x)` becomes `bar`'s first argument, so
    // the whole thing lowers to a plain `Call { bar, [foo(x), y] }`.
    let stmts = Box::leak(parse("## Main\n    Let n be foo(x).bar(y).\n").into_boxed_slice());
    let value = first_let_value(stmts);
    match value {
        Expr::Call { args, .. } => assert!(
            matches!(args.first(), Some(Expr::Call { .. })),
            "expected the receiver call as bar's first argument, got {args:?}"
        ),
        other => panic!("expected a UFCS Call, got {other:?}"),
    }
}

/// Parse `src` and return the first `## To` function's `(param types, return type)`
/// as resolved type-name strings ("Named:X" for undefined user types).
fn first_fn_sig_names(src: &str) -> (Vec<String>, Option<String>) {
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
    let mut lexer = Lexer::new(src, &mut *interner);
    let tokens = lexer.tokenize();
    let type_registry = TypeRegistry::default();
    let mut parser = Parser::new(tokens, world_state, &mut *interner, ctx, type_registry);
    let stmts = parser.parse_program().expect("program should parse");
    let name = |t: &TypeExpr<'static>| -> String {
        match t {
            TypeExpr::Primitive(s) => interner.resolve(*s).to_string(),
            TypeExpr::Named(s) => format!("Named:{}", interner.resolve(*s)),
            other => format!("{other:?}"),
        }
    };
    for s in &stmts {
        if let Stmt::FunctionDef {
            params,
            return_type,
            ..
        } = s
        {
            let ps = params.iter().map(|(_, t)| name(t)).collect();
            return (ps, return_type.map(|t| name(t)));
        }
    }
    panic!("no function def parsed in {src:?}");
}

#[test]
fn element_and_event_reach_the_ast_as_their_own_nominal_types() {
    // MIGRATED (LGX campaign, T1.5): the parser used to REWRITE these names to `Int`, so
    // `Let x: Element be 3.` type-checked and a `-> Element` function could `Return 0`. They
    // now reach the AST as themselves and are distinct types in the checker
    // (`logicaffeine_tests/tests/nominal_dom_types_lock.rs`), while their runtime and AOT
    // representation stays the i64 handle every DOM builtin returns — which that same lock
    // pins, so the concern this test used to carry (codegen must never emit the LOGOS type
    // name verbatim) is still covered.
    let (params, ret) = first_fn_sig_names("## To mk (e: Event) -> Element:\n    Return e.\n");
    assert_eq!(
        params,
        vec!["Named:Event"],
        "an Event parameter keeps its own type"
    );
    assert_eq!(
        ret.as_deref(),
        Some("Named:Element"),
        "an Element return keeps its own type"
    );
}

#[test]
fn undefined_type_still_parses_as_named() {
    // The alias must DISTINGUISH — a genuinely unknown type stays Named.
    let (params, _) = first_fn_sig_names("## To g (a: Glorp) -> Glorp:\n    Return a.\n");
    assert_eq!(params, vec!["Named:Glorp"]);
}

#[test]
fn bare_call_statement_still_lowers_to_stmt_call() {
    let stmts = Box::leak(parse("## Main\n    greet(\"Alice\").\n").into_boxed_slice());
    assert!(
        stmts.iter().any(|s| matches!(s, Stmt::Call { .. })),
        "a bare call statement must remain a Stmt::Call, got {stmts:?}"
    );
}

// Wave 4 — `count of xs` (no article) is refused, naming the canonical `length of xs`;
// `the count of xs` (a length) and `the count of "sel"` (a page query) are unaffected.

#[test]
fn bare_count_of_is_refused_naming_length_of() {
    let err = try_parse("## Main\nShow count of xs.\n").expect_err("`count of xs` is not a form");
    let message = format!("{err:?}");
    assert!(
        message.contains("length of xs"),
        "the refusal must name the canonical `length of xs`, got {message}"
    );
}

#[test]
fn the_count_of_a_collection_still_reads_as_its_length() {
    let stmts = Box::leak(parse("## Main\n    Show the count of xs.\n").into_boxed_slice());
    assert!(
        stmts.iter().any(|s| matches!(
            s,
            Stmt::Show {
                object: Expr::Length { .. },
                ..
            }
        )),
        "`the count of xs` must still read as `Expr::Length`, got {stmts:?}"
    );
}

#[test]
fn the_count_of_a_selector_still_reads_as_a_page_query() {
    let stmts = Box::leak(parse("## Main\n    Show the count of \".row\".\n").into_boxed_slice());
    assert!(
        stmts.iter().any(|s| matches!(
            s,
            Stmt::Show {
                object: Expr::Dom(logicaffeine_language::ast::stmt::DomQuery::Count { .. }),
                ..
            }
        )),
        "`the count of \".row\"` must still read as a page query, got {stmts:?}"
    );
}
