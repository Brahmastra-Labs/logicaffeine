//! ★ RATCHET: the LGX view surface is exhaustively catalogued ★
//!
//! Every view construct has a compiling example here, and common view mistakes have a
//! locked Socratic error. A construct can never be silently dropped or broken (the
//! positive catalog must compile + never shrink), and the guidance for a mistake can
//! never regress to a bare "expected token".
//!
//! Supersedes the old reactive-dialect (`Render:`) catalogue this file used to hold: two of that
//! dialect's rows have no LGX equivalent by design, not by omission — `raw "<b>bold</b>"` as a bare
//! VIEW STATEMENT is replaced by the hole-level `{raw expr}` (still catalogued below); "Render into
//! an arbitrary element" has no analogue at all, since LGX mounts declaratively (`## Page`/`Mount`),
//! never by an imperative call naming a render target.

use logicaffeine_compile::compile::compile_to_wasm;

/// One compiling example per view construct — the whole LGX view surface.
const VIEW_CONSTRUCTS: &[(&str, &str)] = &[
    ("element", "## Component App\nView:\n    <div></div>\n"),
    ("element with inline text", "## Component App\nView:\n    <h1>Hello</h1>\n"),
    ("attributes", "## Component App\nView:\n    <a href=\"/x\" id=\"go\">link</a>\n"),
    (
        "state interpolation",
        "## Component App\nState:\n    Let count be 1.\nView:\n    <p>n = {count}</p>\n",
    ),
    (
        "event handler (no argument)",
        "## Component App\nState:\n    Let n be 0.\nActions:\n    To act:\n        Increase n by 1.\nView:\n    <button on-click={act}>go</button>\n",
    ),
    (
        "event handler (with argument)",
        "## Component App\nState:\n    Let n be 0.\nActions:\n    To bump (by: Int):\n        Increase n by by.\nView:\n    <button on-click={bump(1)}>go</button>\n",
    ),
    (
        "bind (form binding)",
        "## Component App\nState:\n    Let title be \"\".\nView:\n    <input aria-label=\"t\" bind={title}>\n",
    ),
    (
        "raw html (hole)",
        "## Component App\nState:\n    Let markup be \"<b>bold</b>\".\nView:\n    <div>{raw markup}</div>\n",
    ),
    (
        "scoped style",
        "## Component App\nView:\n    <div class=\"box\">x</div>\nStyle:\n    .box { color: red }\n",
    ),
    (
        "for each",
        "## Component App\nState:\n    Let xs be [\"a\", \"b\"].\nView:\n    <ul>{for each x in xs:}<li>{x}</li>{end}</ul>\n",
    ),
    (
        "keyed for each (identity)",
        "## Component Row (r: Text)\nView:\n    <li>{r}</li>\n\n## Component App\nState:\n    Let xs be [\"a\"].\nView:\n    <ul>{for each x in xs, keyed by x:}<Row r={x}/>{end}</ul>\n",
    ),
    (
        "keyed for each (literal template)",
        "## Component Row (r: Text)\nView:\n    <li>{r}</li>\n\n## Component App\nState:\n    Let xs be [\"a\"].\nView:\n    <ul>{for each x in xs, keyed by \"row-{x}\":}<Row r={x}/>{end}</ul>\n",
    ),
    (
        "for each with index",
        "## Component App\nState:\n    Let xs be [\"a\", \"b\"].\nView:\n    <ul>{for each x in xs, at index i:}<li>{i}: {x}</li>{end}</ul>\n",
    ),
    (
        "if",
        "## Component App\nState:\n    Let ready be true.\nView:\n    <div>{if ready:}<p>yes</p>{end}</div>\n",
    ),
    (
        "if / else",
        "## Component App\nState:\n    Let ready be true.\nView:\n    <div>{if ready:}<p>yes</p>{else:}<p>no</p>{end}</div>\n",
    ),
    (
        "if / else if / else",
        "## Component App\nState:\n    Let n be 1.\nView:\n    <div>{if n is 0:}<p>zero</p>{else if n is 1:}<p>one</p>{else:}<p>many</p>{end}</div>\n",
    ),
    (
        "component child (no props)",
        "## Component Card\nView:\n    <p>card</p>\n\n## Component App\nView:\n    <div><Card/></div>\n",
    ),
    (
        "component child (with prop)",
        "## Component Card (title: Text)\nView:\n    <p>{title}</p>\n\n## Component App\nView:\n    <div><Card title=\"Notes\"/></div>\n",
    ),
    (
        "component with children",
        "## Component Wrap\nView:\n    <section>{children}</section>\n\n## Component App\nView:\n    <Wrap><p>inside</p></Wrap>\n",
    ),
    (
        "class toggle",
        "## Component App\nState:\n    Let on be true.\nView:\n    <div class:active={on}>x</div>\n",
    ),
    (
        "directive",
        "## Directive tooltip (el: Element) and (text: Text):\n    Call setAttribute with el and \"title\" and text.\n\n## Component App\nState:\n    Let hint be \"hi\".\nView:\n    <div tooltip={hint}>x</div>\n",
    ),
    (
        "## Page (mounts itself)",
        "## Page /\nState:\n    Let n be 0.\nView:\n    <h1>Count: {n}</h1>\n",
    ),
    (
        "## Component + Mount (default #root)",
        "## Component App\nState:\n    Let n be 0.\nView:\n    <h1>Count: {n}</h1>\n\n## Main\nMount App.\n",
    ),
    (
        "## Component + Mount at \"#sel\"",
        "## Component App\nState:\n    Let n be 0.\nView:\n    <h1>Count: {n}</h1>\n\n## Main\nMount App at \"#app\".\n",
    ),
];

/// Every catalogued view construct compiles to a wasm-dom module — proof the whole
/// surface is live, and a never-shrink floor so none can be silently dropped.
#[test]
fn every_view_construct_compiles() {
    for (name, src) in VIEW_CONSTRUCTS {
        let r = compile_to_wasm(src);
        assert!(
            r.is_ok(),
            "view construct `{name}` must compile:\n{src}\n=> {:?}",
            r.err()
        );
    }
    assert!(
        VIEW_CONSTRUCTS.len() >= 24,
        "the view-construct catalog shrank to {} — a construct was dropped",
        VIEW_CONSTRUCTS.len()
    );
}

/// Common view mistakes get view-aware Socratic guidance — never a bare "expected token".
/// Each pair is (source, a substring the error message must contain).
const VIEW_MISTAKES: &[(&str, &str)] = &[
    (
        "## Component App\nState:\n    Let xs be [\"a\"].\nView:\n    <ul>{for each x xs:}<li>{x}</li>{end}</ul>\n",
        "for each",
    ),
    (
        "## Component App\nView:\n    <div>{if:}<p>x</p>{end}</div>\n",
        "if",
    ),
    (
        "## Component App\nView:\n    <div\n",
        "close",
    ),
];

#[test]
fn view_mistakes_keep_their_socratic_guidance() {
    for (src, needle) in VIEW_MISTAKES {
        let err = format!(
            "{:?}",
            compile_to_wasm(src).expect_err("this should not compile")
        );
        assert!(
            err.to_lowercase().contains(&needle.to_lowercase()),
            "the error for:\n{src}\nmust mention `{needle}`, got:\n{err}"
        );
    }
}
