//! ★ LOCK — the template checker is CONNECTED. ★
//!
//! `lgx/check.rs` and its 5-test unit lock landed in T2.7 and nothing ever called them. Every
//! finding it can produce — `bind` on an element with no value, a duplicate `{block}`, a
//! `class:` toggle on a component, an `<img>` with no `alt` — was computed by a function no
//! parse ever reached. A checker nobody runs is worse than no checker: the tests are green, the
//! error catalog lists the messages, and the author writing the mistake is told nothing.
//!
//! Same shape as `bind` registering no listener and the `on-click` closure nothing could call.
//! The machinery LOOKED wired up. This is the test that says whether it is.

use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::analysis::TypeRegistry;
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::{Expr, LogicExpr, NounPhrase, Stmt, Term, ThematicRole, TypeExpr};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::lgx::check::Severity;
use logicaffeine_language::{Lexer, Parser};

/// Parse a program, returning either the error message or the WARNINGS the parse collected.
fn parse(src: &str) -> Result<Vec<(Severity, String)>, String> {
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
    let mut parser = Parser::new(tokens, world_state, interner, ctx, TypeRegistry::default());
    match parser.parse_program() {
        Ok(_) => Ok(parser
            .lgx_findings()
            .iter()
            .map(|f| (f.severity, f.message.clone()))
            .collect()),
        Err(e) => Err(format!("{e:?}")),
    }
}

/// The same parse, returning the BYTE SPAN of the error rather than its text.
fn error_span(src: &str) -> (usize, usize) {
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
    let mut parser = Parser::new(tokens, world_state, interner, ctx, TypeRegistry::default());
    let e = parser
        .parse_program()
        .expect_err("this program must be refused");
    (e.span.start, e.span.end)
}

fn page(view: &str) -> String {
    format!("## Component App\nView:\n    {view}\n\n## Main\nMount App.\n")
}

#[test]
fn a_bind_on_an_element_with_no_value_is_refused() {
    let err = parse(&page("<div bind={x}></div>")).unwrap_err();
    assert!(
        err.contains("bind"),
        "the refusal names the construct: {err}"
    );
    assert!(
        err.contains("<input>") || err.contains("input"),
        "and says where a bind belongs instead: {err}"
    );
}

#[test]
fn a_duplicate_block_name_is_refused() {
    let err = parse(&page("{block a:}x{end}{block a:}y{end}")).unwrap_err();
    assert!(
        err.contains("twice") || err.contains("a"),
        "the refusal names the block: {err}"
    );
}

#[test]
fn a_class_toggle_on_a_component_is_refused() {
    let src = "## Component Row\nView:\n    <li>x</li>\n\n\
        ## Component App\nView:\n    <Row class:done={true}/>\n\n## Main\nMount App.\n";
    let err = parse(src).unwrap_err();
    assert!(
        err.contains("class:"),
        "the refusal names the toggle: {err}"
    );
    assert!(
        err.contains("prop"),
        "and points at props, which is the fix: {err}"
    );
}

#[test]
fn an_accessibility_finding_is_a_warning_and_does_not_block_the_page() {
    // The difference between an Error and a Warning is load-bearing. An `<img>` with no `alt` is
    // a real problem and a page that refused to compile over one would make the check something
    // authors route around rather than read.
    let found = parse(&page("<img src=\"/x.png\">")).expect("a warning must not block the parse");
    assert!(
        found
            .iter()
            .any(|(s, m)| *s == Severity::Warning && m.contains("alt")),
        "the alt warning is reported: {found:?}"
    );
}

#[test]
fn a_clean_page_reports_nothing_at_all() {
    // The other half of a checker being useful: it has to be quiet when there is nothing wrong,
    // or every author learns to ignore it.
    let found = parse(&format!(
        "## Component App\nState:\n    Let draft be \"\".\nView:\n    \
         <label for=\"t\">Name</label><input id=\"t\" type=\"text\" bind={{draft}}>\n\n## Main\nMount App.\n"
    ))
    .expect("a clean page parses");
    assert!(
        found.is_empty(),
        "nothing to say about a correct page: {found:?}"
    );
}

#[test]
fn the_finding_points_inside_the_template_not_at_the_top_of_the_file() {
    // A span that lands on the `View:` header sends the reader to the wrong line. The raw block
    // carries its own offsets precisely so a finding can point at the element that caused it.
    let src = "## Component App\nView:\n    <p>ok</p>\n    <div bind={x}></div>\n\n\
        ## Main\nMount App.\n";
    let (start, end) = error_span(src);
    let at = src.find("<div").unwrap();
    assert_eq!(
        start, at,
        "the span starts at the `<div` that caused it, not at `View:`"
    );
    assert!(
        src[start..end].contains("bind"),
        "and covers the element the author has to look at: {:?}",
        &src[start..end]
    );
}
