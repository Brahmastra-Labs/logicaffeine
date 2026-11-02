//! ★ LOCK — a View is a value on every tier: placed, passed, kept, placed again. ★
//!
//! On a page's lowered code a View value is DATA — the template call, one variant of the program's
//! `__View` enum carrying that template's arguments (`parser::lgx_lower::views`) — and placing one
//! draws the template's nodes afresh, in the scope of the component placing it. So a view held in
//! state is placed again by every repaint, a view handed through a prop is drawn where the child
//! puts it, and a hole holding a view is rebuilt, never written as text.
//!
//! Each row runs the page on the tree-walker and the bytecode VM and holds both documents to the
//! page the template means; the browser rows build the page and run it in Chrome.

#[path = "cli_common/mod.rs"]
mod common;

use logicaffeine_cli::browser::{self, Browser, PageBundle};
use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

const BADGE: &str = "## To badge (label: Text) -> View:\n    <span>{label}</span>\n\n";

/// `src`'s document on the tree-walker and on the VM, which must agree.
fn drawn(src: &str) -> String {
    dom::reset();
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker runs it:\n{src}");
    let tw_doc = dom::render();
    dom::reset();
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM runs it:\n{src}");
    let vm_doc = dom::render();
    assert_eq!(tw_doc, vm_doc, "the two tiers draw one page:\n{src}");
    tw_doc
        .strip_prefix("<body>")
        .and_then(|d| d.strip_suffix("</body>"))
        .unwrap_or(&tw_doc)
        .to_string()
}

/// `(shape, page, the document it draws)`.
fn shapes() -> Vec<(&'static str, String, &'static str)> {
    vec![
        (
            "a template called in a hole",
            format!("{BADGE}## Component App\nView:\n    <p>{{badge(\"done\")}}</p>\n\n## Main\nMount App.\n"),
            "<p><span>done</span></p>",
        ),
        (
            "two calls, two views",
            format!("{BADGE}## Component App\nView:\n    <p>{{badge(\"a\")}}{{badge(\"b\")}}</p>\n\n## Main\nMount App.\n"),
            "<p><span>a</span><span>b</span></p>",
        ),
        (
            "a template called in a loop",
            format!("{BADGE}## Component App\nState:\n    Let items be [\"x\", \"y\"].\nView:\n    <ul>{{for each i in items:}}{{badge(i)}}{{end}}</ul>\n\n## Main\nMount App.\n"),
            "<ul><span>x</span><span>y</span></ul>",
        ),
        (
            "a view handed through a prop",
            format!("{BADGE}## Component Slot (content: View)\nView:\n    <div>{{content}}</div>\n\n## Component App\nView:\n    <Slot content={{badge(\"in\")}}/>\n\n## Main\nMount App.\n"),
            "<div><span>in</span></div>",
        ),
        (
            "a template taking a view",
            format!("{BADGE}## To frame (inner: View) -> View:\n    <div class=\"frame\">{{inner}}</div>\n\n## Component App\nView:\n    <main>{{frame(badge(\"x\"))}}</main>\n\n## Main\nMount App.\n"),
            "<main><div class=\"frame\"><span>x</span></div></main>",
        ),
        (
            "a template of two parameters",
            "## To pair (a: Text) and (b: Int) -> View:\n    <i>{a}{b}</i>\n\n## Component App\nView:\n    <p>{pair(\"n\", 3)}</p>\n\n## Main\nMount App.\n".to_string(),
            "<p><i>n3</i></p>",
        ),
        (
            "a view kept in state, placed again after a turn",
            format!("{BADGE}## Component App\nState:\n    Let badges be [badge(\"a\")].\nActions:\n    To more:\n        Push badge(\"b\") to badges.\nView:\n    <p>{{for each b in badges:}}{{b}}{{end}}</p><button id=\"go\" on-click={{more}}>+</button>\n\n## Main\nMount App.\nClick \"#go\".\n"),
            "<p><span>a</span><span>b</span></p><button id=\"go\">+</button>",
        ),
        (
            "a hole whose view a turn replaces",
            format!("{BADGE}## Component App\nState:\n    Let shown be badge(\"a\").\nActions:\n    To swap:\n        Set shown to badge(\"b\").\nView:\n    <p>{{shown}}</p><button id=\"go\" on-click={{swap}}>+</button>\n\n## Main\nMount App.\nClick \"#go\".\n"),
            "<p><span>b</span></p><button id=\"go\">+</button>",
        ),
        (
            "a template placed by a styled component takes its scope",
            format!("{BADGE}## Component Slot (content: View)\nView:\n    <div>{{content}}</div>\nStyle:\n    span {{ color: red; }}\n\n## Component App\nView:\n    <Slot content={{badge(\"in\")}}/>\n\n## Main\nMount App.\n"),
            "<style>span.lgx-slot { color: red; }</style><div class=\"lgx-slot\"><span class=\"lgx-slot\">in</span></div>",
        ),
    ]
}

#[test]
fn every_placed_view_draws_the_same_page_on_both_tiers() {
    for (shape, src, expected) in shapes() {
        assert_eq!(drawn(&src), expected, "{shape}:\n{src}");
    }
}

#[test]
fn a_view_is_never_drawn_as_its_text() {
    for (shape, src, _) in shapes() {
        let doc = drawn(&src);
        assert!(
            !doc.contains("&lt;") && !doc.contains("__View"),
            "{shape}: a view was written as text, its markup or its data: {doc}"
        );
    }
}

/// In the statements the parse seam hands every engine (`ui_bridge::with_parsed_program`), how many
/// View expressions are still markup, and how many `__View` variants the templates answer instead.
fn template_bodies(src: &str) -> (usize, usize) {
    use logicaffeine_language::ast::stmt::Expr;
    use logicaffeine_language::ast_depth::{for_each_node, Visited};
    use logicaffeine_language::parser::lgx_lower::view_type_name;
    logicaffeine_compile::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _, _) = parsed.unwrap_or_else(|e| panic!("it parses: {e}\n{src}"));
        let (mut markup, mut data) = (0, 0);
        for_each_node(stmts, |node| match node {
            Visited::Expr(Expr::View(_)) => markup += 1,
            Visited::Expr(Expr::NewVariant { enum_name, .. })
                if interner.resolve(*enum_name) == view_type_name() =>
            {
                data += 1
            }
            _ => {}
        });
        (markup, data)
    })
}

#[test]
fn no_template_reaches_an_engine_as_markup() {
    // A `-> View` body is markup only between the parser and the lowering, which makes every one
    // its `__View` variant before any engine sees the program — so no tier compiles a View
    // expression, and the bytecode compiler has no arm for one.
    let shown_by_main = format!("{BADGE}## Main\nShow badge(\"a\").\n");
    let pages = shapes()
        .into_iter()
        .map(|(shape, src, _)| (shape, src))
        .chain([("a template Main shows", shown_by_main)]);
    for (shape, src) in pages {
        let (markup, data) = template_bodies(&src);
        assert!(
            data > 0,
            "{shape}: its templates answer their variants:\n{src}"
        );
        assert_eq!(
            markup, 0,
            "{shape}: a template reached the engines as markup:\n{src}"
        );
    }
}

#[test]
fn every_template_page_builds_for_the_browser() {
    for (shape, src, _) in shapes() {
        if let Err(e) = compile_to_wasm(&src) {
            panic!("{shape}: the page must build for the browser: {e:?}\n{src}");
        }
    }
}

/// What a template may not hold, each with the words its refusal names it by.
const REFUSED: &[(&str, &str, &str)] = &[
    (
        "an event handler",
        "<span on-click={go}>x</span>",
        "an event handler",
    ),
    ("a bind", "<input bind={t}>", "a `bind`"),
    ("a component", "<Inner/>", "a component"),
    (
        "its placer's children",
        "<div>{children}</div>",
        "`{children}`",
    ),
];

#[test]
fn a_template_holding_what_only_a_component_answers_is_refused_by_name() {
    for (what, markup, words) in REFUSED {
        let src = format!(
            "## To badge (label: Text) -> View:\n    {markup}\n\n## Component Inner\nView:\n    <b>i</b>\n\n\
             ## Component App\nState:\n    Let t be \"\".\nActions:\n    To go:\n        Set t to \"x\".\n\
             View:\n    <p>{{badge(\"x\")}}</p>\n\n## Main\nMount App.\n"
        );
        for (tier, error) in [
            ("tree-walker", tw_outcome(&src).error),
            ("VM", vm_outcome(&src).error),
        ] {
            let error =
                error.unwrap_or_else(|| panic!("{what}: the {tier} must refuse it:\n{src}"));
            assert!(
                error.contains("`## To badge`") && error.contains(words),
                "{what}: the {tier}'s refusal names the template and what it holds: {error}"
            );
        }
        let built = compile_to_wasm(&src).expect_err("the browser build refuses it too");
        assert!(
            format!("{built:?}").contains("`## To badge`"),
            "{what}: {built:?}"
        );
    }
}

fn chrome() -> Option<&'static Browser> {
    let Some(chrome) = browser::locate_chrome() else {
        let spec = std::env::var("LOGOS_REQUIRE_TOOLS").unwrap_or_default();
        common::require_tool_strict("chrome", &spec);
        return None;
    };
    Some(browser::shared(&chrome).unwrap_or_else(|e| {
        panic!(
            "Chrome is installed at {} and must start: {e}",
            chrome.display()
        )
    }))
}

#[test]
fn every_template_page_draws_the_same_page_in_a_browser() {
    let Some(b) = chrome() else { return };
    for (shape, src, expected) in shapes() {
        let bundle =
            PageBundle::page(&src).unwrap_or_else(|e| panic!("{shape}: the page bundles: {e}"));
        let p = b
            .open(&bundle)
            .unwrap_or_else(|e| panic!("{shape}: the page opens: {e}"));
        let body = p
            .snapshot()
            .unwrap_or_else(|e| panic!("snapshot: {e}"))
            .body_markup();
        assert_eq!(
            body,
            format!("<body>{expected}</body>"),
            "{shape}: the browser draws the page the tiers draw"
        );
        assert!(
            p.exceptions().is_empty(),
            "{shape}: the page does not throw: {:?}",
            p.exceptions()
        );
    }
}
