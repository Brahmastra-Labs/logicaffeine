//! ★ LOCK — a bad `Style:` block points at the character that broke it. ★
//!
//! `CssError` has carried a byte offset (`at`) since it was written, and `StyleBlock` has carried
//! the offset of its own body in the file (`content_start`). Adding the two is a file position —
//! and nothing added them. An unbalanced brace in a forty-line stylesheet was reported as "The
//! `Style:` of App is not valid CSS", with no line, no column, and no caret.
//!
//! ★ WORSE, IT WAS REPORTED FROM THE WRONG PLACE. The check ran when the component RENDERED, so a
//! stylesheet that could not be scoped was a runtime failure on the reference tier and, on the
//! browser tier, a page that compiled and rendered UNSTYLED with nothing saying why. The mistake
//! is in the source text and is knowable at parse time, which is where a person can act on it.

use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::analysis::{DiscoveryPass, TypeRegistry};
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::{Expr, LogicExpr, NounPhrase, Stmt, Term, ThematicRole, TypeExpr};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::error::ParseError;
use logicaffeine_language::{Lexer, Parser};

fn parse_err(src: &str) -> ParseError {
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
    let mut parser = Parser::new(tokens, world_state, interner, ctx, registry);
    match parser.parse_program() {
        Ok(_) => panic!("this program must be refused:\n{src}"),
        Err(e) => e,
    }
}

/// A component whose `Style:` block is `css`.
fn page(css: &str) -> String {
    format!(
        "## Component App\nState:\n\x20   Let n be 0.\n\
         Actions:\n\x20   To go:\n\x20       Set n to n + 1.\n\
         View:\n\x20   <p id=\"p\">{{n}}</p>\n\
         Style:\n\x20   {css}\n\n\
         ## Main\nMount App.\n"
    )
}

#[test]
fn an_unbalanced_brace_is_refused_where_it_was_written() {
    let src = page("p { color: red;");
    let e = parse_err(&src);
    let at = src.get(e.span.start..e.span.end).unwrap_or("");
    assert!(
        e.span.start > 0,
        "the refusal points at the top of the file. `CssError` knows the offset inside the \
         stylesheet and `StyleBlock` knows where the stylesheet starts; adding them is the whole \
         fix. message: {:?}",
        e.kind
    );
    // The span must land inside the Style: block, not somewhere earlier in the component.
    let style_at = src
        .find("p { color")
        .expect("the fixture has a Style block");
    assert!(
        e.span.start >= style_at,
        "the refusal points at byte {} but the stylesheet starts at {style_at} — it is pointing \
         at something the author did not write wrong. it covers: {at:?}",
        e.span.start
    );
}

#[test]
fn a_valid_stylesheet_still_parses() {
    // ★ THE CONTROL. A check that refused every `Style:` block would satisfy the test above and
    // break every styled page in the repository.
    let src = page("p { color: red; }");
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
    let mut lexer = Lexer::new(&src, interner);
    let tokens = lexer.tokenize();
    let registry: TypeRegistry = DiscoveryPass::new(&tokens, interner).run_full().types;
    let mut parser = Parser::new(tokens, world_state, interner, ctx, registry);
    assert!(
        parser.parse_program().is_ok(),
        "a valid stylesheet must parse:\n{src}"
    );
}
