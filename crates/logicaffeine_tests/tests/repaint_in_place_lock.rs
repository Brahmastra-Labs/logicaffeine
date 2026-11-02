//! ★ LOCK — a repaint keeps every node that did not change, and writes only what did. ★
//!
//! A view draws itself whole on every paint; `vdom.lg` commits the new tape against the retained
//! tree, and a node drawn with the same tag at the same place (or the same key) IS the node it was:
//! the commit writes its changed text, value, attribute or listener in place, drops the nodes that
//! were not drawn again, and touches nothing else. A node that survives keeps what the browser holds
//! in it — focus, selection, scroll, a half-typed value — which is why a bound input can be typed
//! into at all.
//!
//! Each row hands its commits to both hosts a page runs on (`support/vdom_hosts.rs`: the native
//! recorder and `runtime.js` under node): the stream must be exactly the minimal write, and the
//! surviving nodes the same nodes — the same stream id on the native recorder, the same object in
//! the browser.

mod common;
#[path = "compile_common/mod.rs"]
mod compile_common;
#[path = "support/vdom_hosts.rs"]
mod vdom_hosts;

use logicaffeine_compile::semantics::dom::{self, DomOp};
use vdom_hosts::{paint, Paint};

/// A bound text input holding `value`; a heading beside a paragraph showing `n`; a button that
/// listens for clicks; a list of `n` rows.
const VIEWS: &str = "
## To bound(value: Text):
    vdOpen(\"input\").
    vdSetAttribute(\"type\", \"text\").
    vdSetValue(value).
    vdListen(\"input\", 0).
    vdClose().

## To counter(n: Text):
    vdOpen(\"h1\").
    vdText(\"Title\").
    vdClose().
    vdOpen(\"p\").
    vdText(n).
    vdClose().

## To button():
    vdOpen(\"button\").
    vdListen(\"click\", 0).
    vdText(\"go\").
    vdClose().

## To rows(n: Int):
    vdOpen(\"ul\").
    Let mutable i be 0.
    While i < n:
        vdOpen(\"li\").
        vdText(\"item{i}\").
        vdClose().
        Set i to i + 1.
    vdClose().
";

/// `paints`, each committed, into a root standing for the body.
fn painted(paints: &[&str]) -> Paint {
    let mut main = String::from("Let root be vdNewRoot(0).\n");
    for draw in paints {
        main.push_str(&format!("{draw}.\ncommit(root).\n"));
    }
    let p = paint(VIEWS, &main);
    p.assert_hosts_follow();
    p
}

/// The one node step `k` created with `tag`, and the text node it created holding `text`.
fn made(p: &Paint, k: usize, tag: &str, text: &str) -> (Option<u32>, Option<u32>) {
    let ops = &p.steps[k].ops;
    let element = ops.iter().find_map(|op| match op {
        DomOp::CreateElement { node, tag: t } if t == tag => Some(*node),
        _ => None,
    });
    let text = ops.iter().find_map(|op| match op {
        DomOp::CreateTextNode { node, text: t } if t == text => Some(*node),
        _ => None,
    });
    (element, text)
}

/// Asserts that the `tags` nodes after step `k` are the ones step `k - 1` left, on both hosts.
fn kept(p: &Paint, k: usize, tags: &[&str]) {
    for tag in tags {
        if *tag != "#text" {
            assert_eq!(
                p.native_ids(k, tag),
                p.native_ids(k - 1, tag),
                "step {k}: the native `{tag}` nodes are the ones they were"
            );
        }
        if let (Some(before), Some(now)) = (p.browser(k - 1), p.browser(k)) {
            assert_eq!(
                now.uids(tag),
                before.uids(tag),
                "step {k}: the browser's `{tag}` nodes are the objects they were"
            );
        }
    }
}

#[test]
fn a_bound_input_keeps_its_node_across_repaints() {
    // The G17 core: a keystroke repaints the view, and the input being typed into must be the same
    // node afterwards — one in-place value write, no create, no removal.
    let p = painted(&["bound(\"a\")", "bound(\"ab\")", "bound(\"abc\")"]);
    let (input, _) = made(&p, 0, "input", "");
    let input = input.expect("the first commit creates the input");
    for (k, typed) in [(1, "ab"), (2, "abc")] {
        assert_eq!(
            p.steps[k].ops,
            vec![DomOp::SetValue {
                node: input,
                value: typed.into()
            }],
            "repaint {k} writes the value into the input it already has, and nothing else"
        );
        kept(&p, k, &["input"]);
    }
    assert_eq!(p.steps[2].native, "<input type=\"text\" value=\"abc\">");
}

#[test]
fn only_the_changed_text_is_written_and_its_sibling_is_untouched() {
    // A counter: a static heading beside a paragraph the turn changes. The heading gets no op at all.
    let p = painted(&["counter(\"n=0\")", "counter(\"n=1\")"]);
    let (_, shown) = made(&p, 0, "p", "n=0");
    let shown = shown.expect("the first commit creates the paragraph's text");
    assert_eq!(
        p.steps[1].ops,
        vec![DomOp::SetText {
            node: shown,
            text: "n=1".into()
        }],
        "only the paragraph's text is written, in place"
    );
    kept(&p, 1, &["h1", "p", "#text"]);
    assert_eq!(p.steps[1].native, "<h1>Title</h1><p>n=1</p>");
}

#[test]
fn a_listener_is_attached_once_however_often_the_view_repaints() {
    // A button's click listener, repainted four times: the listener is sent once, and each host holds
    // exactly one — a refresh that attached another each time would fire the action four times.
    let p = painted(&["button()", "button()", "button()", "button()"]);
    let listens = |k: usize| {
        p.steps[k]
            .ops
            .iter()
            .filter(|op| matches!(op, DomOp::Listen { .. }))
            .count()
    };
    assert_eq!(
        listens(0),
        1,
        "the mount sends the listener: {:?}",
        p.steps[0].ops
    );
    for k in 1..4 {
        assert!(
            p.steps[k].ops.is_empty(),
            "repaint {k} of an unchanged button sends nothing: {:?}",
            p.steps[k].ops
        );
        kept(&p, k, &["button"]);
        if let Some(b) = p.browser(k) {
            assert_eq!(
                b.listeners("button", "click"),
                vec![1],
                "repaint {k}: the browser's button has one click listener"
            );
        }
        let stops = p.native_at(k, || {
            dom::dispatch_path(dom::query_selector("button"), "click").stops
        });
        assert_eq!(
            stops.len(),
            1,
            "repaint {k}: a click reaches the native button's one listener: {stops:?}"
        );
    }
}

#[test]
fn a_dropped_row_is_removed_in_place() {
    // Two rows, then one: the stale row is freed where it stands, and the list and the row that
    // stays are the nodes they were — no clear-and-rebuild.
    let p = painted(&["rows(2)", "rows(1)"]);
    let dropped = p.native_ids(0, "li")[1];
    assert_eq!(
        p.steps[1].ops,
        vec![DomOp::FreeNode {
            node: dropped as u32
        }],
        "exactly the stale row is freed, and nothing else is written"
    );
    kept(&p, 1, &["ul"]);
    assert_eq!(
        p.native_ids(1, "li"),
        vec![p.native_ids(0, "li")[0]],
        "the row that stays is the node it was"
    );
    if let (Some(before), Some(now)) = (p.browser(0), p.browser(1)) {
        assert_eq!(
            now.uids("li"),
            vec![before.uids("li")[0]],
            "the browser's remaining row is the object it was"
        );
    }
    assert_eq!(p.steps[1].native, "<ul><li>item0</li></ul>");
}

/// The one sentence a call to the retired paint boundary is refused in, on every tier.
const BEGIN_PAINT_IS_GONE: &str =
    "`beginPaint` is gone: a repaint keeps its nodes by drawing through \
    the retained tree — `vdOpen`/`vdText`/`vdClose`, then `vdCommit` and `vdApply` — which an LGX \
    component's view does for you.";

#[test]
fn the_retired_paint_boundary_is_refused_on_every_tier_pointing_at_the_retained_tree() {
    // `beginPaint` opened a host-side staging pass that kept nodes across a repaint; that pass is
    // gone, and a boundary that looked like it kept nodes and did not would be worse than none. A
    // call to it is refused before the program runs, on every tier, naming the way a repaint keeps
    // its nodes now.
    use logicaffeine_compile::compile::{compile_to_rust, compile_to_wasm, tw_outcome, vm_outcome};
    use logicaffeine_language::error::socratic_explanation;
    let src = "## Main\nLet d be createElement(\"div\").\nCall beginPaint.\nCall appendChild with body() and d.\n";
    let words = |e: logicaffeine_language::error::ParseError| {
        socratic_explanation(&e, &logicaffeine_language::Interner::new())
    };
    dom::reset();
    let tw = tw_outcome(src);
    assert_eq!(
        tw.error.as_deref(),
        Some(BEGIN_PAINT_IS_GONE),
        "the tree-walker refuses it"
    );
    assert_eq!(
        dom::render(),
        "<body></body>",
        "refused before anything ran"
    );
    assert_eq!(
        vm_outcome(src).error.as_deref(),
        Some(BEGIN_PAINT_IS_GONE),
        "the VM refuses it"
    );
    assert_eq!(
        compile_to_wasm(src).map_err(words).err().as_deref(),
        Some(BEGIN_PAINT_IS_GONE),
        "wasm refuses it"
    );
    assert_eq!(
        compile_to_rust(src).map_err(words).err().as_deref(),
        Some(BEGIN_PAINT_IS_GONE),
        "AOT refuses it"
    );
}

#[test]
fn a_program_of_its_own_may_define_a_function_named_begin_paint() {
    // The refusal is of the retired builtin, not of the name: a program that defines `beginPaint`
    // calls its own.
    let src = "## To beginPaint():\n    Show \"painting\".\n\n## Main\nCall beginPaint.\n";
    for (tier, run) in [
        (
            "tree-walker",
            logicaffeine_compile::compile::tw_outcome
                as fn(&str) -> logicaffeine_compile::compile::RunOutcome,
        ),
        ("VM", logicaffeine_compile::compile::vm_outcome),
    ] {
        let out = run(src);
        assert_eq!(
            (out.error.as_deref(), out.output.trim()),
            (None, "painting"),
            "the {tier} calls the program's own function"
        );
    }
    assert!(
        logicaffeine_compile::compile::compile_to_wasm(src).is_ok(),
        "and it compiles to wasm"
    );
}
