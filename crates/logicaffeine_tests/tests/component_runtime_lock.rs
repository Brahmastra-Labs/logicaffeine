//! ★ LOCK — an LGX component RUNS on the interpreter: it mounts, holds state, renders its
//! view into the DOM, and its `When the page loads` handler runs exactly once. ★
//!
//! The tree-walker is the reference semantics for the component model. Everything here is
//! observed through the same `DomOp` log every DOM tier is compared against
//! (`semantics::dom`), so the VM and the browser tier can be held to this record later.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom::{self, DomOp};

/// Run `src` on the tree-walker and return the DOM op-log it produced.
fn ops(src: &str) -> Vec<DomOp> {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the program must run cleanly:\n{src}");
    dom::op_log()
}

/// The rendered tree as HTML-ish text, without the recorder's `<body>` wrapper — the shape
/// assertion that reads like the view.
fn rendered(src: &str) -> String {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the program must run cleanly:\n{src}");
    let html = dom::render();
    html.strip_prefix("<body>")
        .and_then(|h| h.strip_suffix("</body>"))
        .unwrap_or(&html)
        .to_string()
}

#[test]
fn a_component_mounts_and_renders_its_view() {
    let src = "## Component App\nState:\n    Let n be 7.\nView:\n    <p>Count: {n}</p>\n\n## Main\nMount App.\n";
    let log = ops(src);
    assert!(
        log.iter()
            .any(|o| matches!(o, DomOp::CreateElement { tag, .. } if tag == "p")),
        "the view's element is created: {log:?}"
    );
    // A text run and a hole are SEPARATE text nodes, in document order — that is what lets a
    // later fine-grained update repaint the hole alone without touching its neighbours.
    let texts: Vec<&str> = log
        .iter()
        .filter_map(|o| match o {
            DomOp::CreateTextNode { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        texts,
        ["Count: ", "7"],
        "the hole renders its state value in place: {log:?}"
    );
    assert!(
        rendered(src).contains("<p>Count: 7</p>"),
        "rendered: {}",
        rendered(src)
    );
}

#[test]
fn text_and_holes_interleave_in_document_order() {
    let src = "## Component App\nState:\n    Let name be \"ada\".\nView:\n    <p>Hi <b>{name}</b>!</p>\n\n## Main\nMount App.\n";
    assert_eq!(rendered(src), "<p>Hi <b>ada</b>!</p>");
}

#[test]
fn conditionals_and_loops_render() {
    let src = "## Component App\nState:\n    Let items be [\"a\", \"b\"].\n    Let ok be true.\nView:\n    <ul>\n      {for each x in items:}<li>{x}</li>{end}\n    </ul>\n    {if ok:}<p>yes</p>{else:}<p>no</p>{end}\n\n## Main\nMount App.\n";
    assert_eq!(rendered(src), "<ul><li>a</li><li>b</li></ul><p>yes</p>");
}

#[test]
fn an_empty_collection_renders_the_else_arm() {
    let src = "## Component App\nState:\n    Let items be a new Seq of Text.\nView:\n    <ul>{for each x in items:}<li>{x}</li>{else:}<li>none</li>{end}</ul>\n\n## Main\nMount App.\n";
    assert_eq!(rendered(src), "<ul><li>none</li></ul>");
}

#[test]
fn attributes_static_and_dynamic_reach_the_dom() {
    let src = "## Component App\nState:\n    Let cls be \"warm\".\nView:\n    <div class=\"card {cls}\" id=\"root\"><span title={cls}>x</span></div>\n\n## Main\nMount App.\n";
    let log = ops(src);
    let attrs: Vec<(String, String)> = log
        .iter()
        .filter_map(|o| match o {
            DomOp::SetAttribute { name, value, .. } => Some((name.clone(), value.clone())),
            _ => None,
        })
        .collect();
    assert!(
        attrs.contains(&("class".to_string(), "card warm".to_string())),
        "{attrs:?}"
    );
    assert!(
        attrs.contains(&("id".to_string(), "root".to_string())),
        "{attrs:?}"
    );
    assert!(
        attrs.contains(&("title".to_string(), "warm".to_string())),
        "{attrs:?}"
    );
}

#[test]
fn a_child_component_composes_with_its_props() {
    let src = "## Component Row (label: Text)\nView:\n    <li>{label}</li>\n\n## Component App\nState:\n    Let items be [\"a\", \"b\"].\nView:\n    <ul>{for each x in items:}<Row label={x}/>{end}</ul>\n\n## Main\nMount App.\n";
    assert_eq!(rendered(src), "<ul><li>a</li><li>b</li></ul>");
}

/// The LGX twin of the retired dialect's `keyed_by_accepts_string_and_interpolated_keys`: a
/// `keyed by` clause takes ANY expression the template grammar parses (`lgx/template.rs`'s
/// `texpr` — not only a bare identity path), including a computed STRING TEMPLATE. Proven the
/// way `component_model_lock.rs`'s own `a_keyed_loop_carries_state_with_the_item_through_a_reorder`
/// proves a bare `keyed by x`: a reorder must carry each row's state with its COMPUTED key.
#[test]
fn a_keyed_loop_accepts_an_interpolated_string_key() {
    let src = "## Component Row (label: Text)\nState:\n    Let clicks be 0.\nActions:\n    \
        To bump:\n        Set clicks to clicks + 1.\nView:\n    <li class=\"row\" on-click={bump}>{label}{clicks}</li>\n\n\
        ## Component App\nState:\n    Let items be [\"a\", \"b\"].\nActions:\n    \
        To flip:\n        Set items to [\"b\", \"a\"].\nView:\n    \
        <ul>{for each x in items, keyed by \"row-{x}\":}<Row label={x}/>{end}</ul>\n    \
        <button id=\"go\" on-click={flip}>flip</button>\n\n\
        ## Main\nMount App.\nClick \".row\".\nClick \"#go\".\n";
    let out = rendered(src);
    assert!(
        out.contains("<li class=\"row\">b0</li>"),
        "`b` is first now and never was clicked: {out}"
    );
    assert!(
        out.contains("<li class=\"row\">a1</li>"),
        "`a` moved and kept its click, by its computed key: {out}"
    );
}

#[test]
fn when_the_page_loads_runs_once_before_the_render() {
    let src = "## Component App\nState:\n    Let n be 0.\nActions:\n    When the page loads:\n        Set n to 42.\nView:\n    <p>{n}</p>\n\n## Main\nMount App.\n";
    assert_eq!(
        rendered(src),
        "<p>42</p>",
        "the load handler's state change is visible in the first render"
    );
}

#[test]
fn a_derived_field_recomputes_from_the_rest_of_the_state() {
    let src = "## Component App\nState:\n    Let items be [\"a\", \"b\", \"c\"].\n    Let total always be length of items.\nView:\n    <p>{total}</p>\n\n## Main\nMount App.\n";
    assert_eq!(rendered(src), "<p>3</p>");
}

#[test]
fn a_hole_is_escaped_so_state_can_never_inject_markup() {
    let src = "## Component App\nState:\n    Let evil be \"<script>alert(1)</script>\".\nView:\n    <p>{evil}</p>\n\n## Main\nMount App.\n";
    let out = rendered(src);
    assert!(
        !out.contains("<script"),
        "a hole must be escaped, got {out}"
    );
    assert!(out.contains("&lt;script&gt;"), "escaped form: {out}");
}

#[test]
fn mounting_at_a_selector_renders_under_that_element() {
    let src = "## Component App\nView:\n    <p>x</p>\n\n## Main\nLet root be createElement(\"div\").\nCall setAttribute with root and \"id\" and \"side\".\nCall appendChild with body() and root.\nMount App at \"#side\".\n";
    let out = rendered(src);
    assert!(
        out.contains("<div id=\"side\"><p>x</p></div>"),
        "rendered under the named element: {out}"
    );
}

/// ★ A REAL, VERIFIED BEHAVIOR CHANGE FROM THE RETIRED DIALECT, not a redundant test.
///
/// The retired dialect's `Mount App` (no `at`) defaulted to the selector `"#root"` — the
/// generated ceremony read `querySelector("#root")` explicitly (`parser/mod.rs`'s legacy
/// `parse_ui_mount_statement` branch, dead code for an LGX component today). An LGX component's
/// bare `Mount App.` takes a completely different path (`self.lgx_components.contains(&comp_sym)`
/// is true, so the LEGACY branch above never runs): it parses to `Stmt::Ui(UiStmt::Mount {
/// selector: None, .. })`, and a `None` selector mounts directly under `body()` — it does NOT
/// look for a `#root` element at all, pre-created or not. Confirmed empirically: with a `#root`
/// div ALREADY on the page, bare `Mount App.` still paints straight into `body` (`ClearChildren
/// { node: 0 }`), never touching the `#root` div, while `Mount App at "#root"` correctly targets
/// it (`ClearChildren { node: 1 }`). So `keyed_by_accepts_string_and_interpolated_keys`'s sibling
/// suspect, `mount_without_at_defaults_to_hash_root`, has NO LGX equivalent to port — not because
/// the property is gone with the dialect, but because the property is now FALSE: LGX's bare
/// `Mount` defaults to `body()`, not `#root`.
#[test]
fn mount_without_at_mounts_into_body_not_a_pre_existing_hash_root() {
    let common = "## Component App\nState:\n    Let n be 7.\nView:\n    <p>Count: {n}</p>\n\n\
        ## Main\nLet root be createElement(\"div\").\nCall setAttribute with root and \"id\" and \"root\".\nCall appendChild with body() and root.\n";
    let bare = ops(&format!("{common}Mount App.\n"));
    let explicit = ops(&format!("{common}Mount App at \"#root\".\n"));
    assert!(
        bare.iter().any(|op| matches!(op, DomOp::ClearChildren { node: 0 })),
        "bare `Mount App.` must paint straight into body (node 0), never a pre-existing #root: {bare:?}"
    );
    assert!(
        explicit
            .iter()
            .any(|op| matches!(op, DomOp::ClearChildren { node: 1 })),
        "an explicit `at \"#root\"` must target the div, not body: {explicit:?}"
    );
    assert_ne!(
        bare, explicit,
        "the two forms are NOT the same program — bare Mount ignores #root entirely"
    );
}
