//! ★ LOCK — every LGX view construct parses into the view IR (`ast::view`) before it is
//! lowered. ★
//!
//! The IR is the seam the whole framework hangs off: the checker, the server renderer,
//! the fine-grained DOM compiler and `largo expand` all read it, and the lowering that
//! feeds the execution tiers is a separate walk over it. This lock pins the tree SHAPE
//! the parser builds for each construct of the view surface (the same catalog
//! `view_surface_lock` ratchets), via the IR's deterministic one-line `shape()` printer.
//!
//! Byte-identical op-logs on every tier are locked elsewhere (`wasm_dom`, `reactive`,
//! `dom_reconciliation_lock`, …): this file owns only "the parser sees the tree".
//!
//! ★ THE INTROSPECTION POINT CHANGED WITH THE RETIRED REACTIVE-SIGNAL DIALECT. That
//! dialect's own `Render:`/`Render into <el>:` parser (`parser/view.rs`, deleted) pushed
//! every `ViewDef` it built onto a flat `Parser::views` accumulator, read back here through
//! `take_views()`. LGX's own `## Component`/`## Page` parser (`parser/component.rs`) builds
//! the identical `ast::view::ViewDef`/`ViewNode` tree, but it attaches the tree directly
//! to the definition it belongs to (`ComponentDef.view`, `.head`) instead of a side
//! accumulator, because a component's view is a named part of its OWN definition, not a
//! free-floating render call. `view_shapes` below walks the parsed `Stmt` list for
//! `Stmt::ComponentDef` and reads `.view` off each — the accumulator itself is now
//! permanently empty and no longer worth reading.
//!
//! ★ FOUR OLD-CATALOG CONSTRUCTS HAVE NO LGX TREE TO PIN, BY DESIGN, NOT BY OMISSION —
//! matching `view_surface_lock`'s own documented exclusions:
//!   - "Render into an element" — LGX mounts declaratively (`## Page` / `Mount`), never by
//!     an imperative call naming a render target; there is no tree this could produce.
//!   - "css (plain string)" / "css (raw string)" / "scoped css component" — a component's
//!     `Style:` is `ComponentDef.style`, a SIBLING of `.view`, never a `ViewNode` inside the
//!     tree this file pins. It is still covered end-to-end, just not here (see
//!     `wasm_dom.rs::component_scoped_css_preserves_at_rules_and_scopes_only_nested_selectors`).
//!   - "component call in element position" (`div: card("Hello")`) — an LGX template names a
//!     `<Component/>` instead; a bare call in element position is not LGX surface at all, and
//!     the view IR has no node for it.
//!   - "Repeat-style bare for each" — the optional `each` is a STATEMENT-level parser rule
//!     (`Repeat for <x> in …` / `Repeat for each <x> in …`), already locked at that level by
//!     `reactive.rs::repeat_for_each_statement_iterates_a_collection`; the view hole's own
//!     `{for each … in …:}` does not need a second copy of the same coverage.
//!   - the old "event handler (block)" / "event handler (inline)" pair collapses to ONE
//!     construct — `on-click={action}` has no block/inline surface distinction to preserve
//!     (the same finding already applied to this session's `wasm_dom.rs` port).

use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::analysis::TypeRegistry;
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::{Expr, LogicExpr, NounPhrase, Stmt, Term, ThematicRole, TypeExpr};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::{Lexer, Parser};

/// Parse `input` and return the `shape()` of every component's view IR tree, in source order.
fn view_shapes(input: &str) -> Vec<String> {
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
    let mut parser = Parser::new(tokens, world_state, interner, ctx, TypeRegistry::default());
    let stmts = parser
        .parse_program()
        .unwrap_or_else(|e| panic!("program should parse: {e:?}\n{input}"));
    stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::ComponentDef(def) => def.view.as_ref().map(|v| v.shape(interner)),
            _ => None,
        })
        .collect()
}

/// The view surface, construct by construct: (label, source, expected IR shapes).
const CONSTRUCTS: &[(&str, &str, &[&str])] = &[
    ("element", "## Component App\nView:\n    <div></div>\n", &["view of App{div}"]),
    (
        "element with inline text",
        "## Component App\nView:\n    <h1>Hello</h1>\n",
        &["view of App{h1{\"Hello\"}}"],
    ),
    (
        "attributes",
        "## Component App\nView:\n    <a href=\"/x\" id=\"go\">link</a>\n",
        &["view of App{a[href=\"/x\", id=\"go\"]{\"link\"}}"],
    ),
    (
        "state interpolation",
        "## Component App\nState:\n    Let count be 1.\nView:\n    <p>n = {count}</p>\n",
        &["view of App{p{\"n = \", count}}"],
    ),
    (
        "event handler",
        "## Component App\nState:\n    Let n be 0.\nActions:\n    To act:\n        Increase n by 1.\nView:\n    <button on-click={act}>go</button>\n",
        &["view of App{button{on:click{1}, \"go\"}}"],
    ),
    (
        "bind (form binding)",
        "## Component App\nState:\n    Let title be \"\".\nView:\n    <input aria-label=\"t\" bind={title}>\n",
        &["view of App{input[aria-label=\"t\", bind=title]}"],
    ),
    (
        "raw html (hole)",
        "## Component App\nState:\n    Let markup be \"<b>bold</b>\".\nView:\n    <div>{raw markup}</div>\n",
        &["view of App{div{raw(markup)}}"],
    ),
    (
        "for each",
        "## Component App\nState:\n    Let xs be [\"a\", \"b\"].\nView:\n    <ul>{for each x in xs:}<li>{x}</li>{end}</ul>\n",
        &["view of App{ul{each x in xs{li{x}}}}"],
    ),
    (
        "for each with index",
        "## Component App\nState:\n    Let xs be [\"a\", \"b\"].\nView:\n    <ul>{for each x in xs, at index i:}<li>{i}: {x}</li>{end}</ul>\n",
        &["view of App{ul{each x in xs at index i{li{i, \": \", x}}}}"],
    ),
    (
        "keyed for each (identity)",
        "## Component Row\nView:\n    <li>r</li>\n\n## Component App\nState:\n    Let xs be [\"a\"].\nView:\n    <ul>{for each x in xs, keyed by x:}<Row/>{end}</ul>\n",
        &["view of Row{li{\"r\"}}", "view of App{ul{each x in xs keyed by x{<Row>}}}"],
    ),
    (
        "keyed for each (literal template)",
        "## Component Row\nView:\n    <li>r</li>\n\n## Component App\nState:\n    Let xs be [\"a\"].\nView:\n    <ul>{for each x in xs, keyed by \"row-{x}\":}<Row/>{end}</ul>\n",
        &["view of Row{li{\"r\"}}", "view of App{ul{each x in xs keyed by \"row-{x}\"{<Row>}}}"],
    ),
    (
        "if",
        "## Component App\nState:\n    Let ready be true.\nView:\n    <div>{if ready:}<p>yes</p>{end}</div>\n",
        &["view of App{div{when ready{p{\"yes\"}}}}"],
    ),
    (
        "if / else",
        "## Component App\nState:\n    Let ready be true.\nView:\n    <div>{if ready:}<p>yes</p>{else:}<p>no</p>{end}</div>\n",
        &["view of App{div{when ready{p{\"yes\"}} otherwise{p{\"no\"}}}}"],
    ),
    (
        "component child",
        "## Component Card\nView:\n    <p>card</p>\n\n## Component App\nView:\n    <div><Card/></div>\n",
        &["view of Card{p{\"card\"}}", "view of App{div{<Card>}}"],
    ),
    (
        "component with children",
        "## Component Wrap\nView:\n    <section>{children}</section>\n\n## Component App\nView:\n    <Wrap><p>inside</p></Wrap>\n",
        &["view of Wrap{section{children}}", "view of App{<Wrap>{p{\"inside\"}}}"],
    ),
    (
        "class toggle",
        "## Component App\nState:\n    Let on be true.\nView:\n    <div class:active={on}>x</div>\n",
        &["view of App{div[class:active=on]{\"x\"}}"],
    ),
    (
        "directive",
        "## Directive tooltip (el: Element) and (text: Text):\n    Call setAttribute with el and \"title\" and text.\n\n## Component App\nState:\n    Let hint be \"hi\".\nView:\n    <div tooltip={hint}>x</div>\n",
        &["view of App{div[tooltip!=hint]{\"x\"}}"],
    ),
    (
        "## Component + Mount (default #root)",
        "## Component App\nState:\n    Let n be 0.\nView:\n    <h1>Count: {n}</h1>\n\n## Main\nMount App.\n",
        &["view of App{h1{\"Count: \", n}}"],
    ),
    (
        "## Component + Mount at \"#sel\"",
        "## Component App\nState:\n    Let n be 0.\nView:\n    <h1>Count: {n}</h1>\n\n## Main\nMount App at \"#app\".\n",
        &["view of App{h1{\"Count: \", n}}"],
    ),
];

#[test]
fn every_view_construct_parses_into_the_view_ir() {
    let mut failures = Vec::new();
    for (label, src, expected) in CONSTRUCTS {
        let got = view_shapes(src);
        let want: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
        if got != want {
            failures.push(format!(
                "[{label}]\n  expected: {want:?}\n  got:      {got:?}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} construct(s) did not produce the expected view IR:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The catalog never shrinks: every construct of the LGX view surface that has a `ViewNode`
/// tree to pin has an IR snapshot here (see the file header for the four that provably do not).
#[test]
fn the_ir_catalog_covers_the_whole_view_surface() {
    assert!(
        CONSTRUCTS.len() >= 19,
        "the view IR catalog has {} entries; it may only grow",
        CONSTRUCTS.len()
    );
}
