//! ★ LOCK — a write to a LIVE node reaches the document, and the retained tree with it. ★
//!
//! A patched turn writes to nodes an EARLIER commit created — that is what "patch in place" means.
//! In `vdom.lg` a turn reaches a retained node two ways: it writes the node where it stands
//! (`vdPatchText`/`vdPatchValue`/`vdPatchChecked` on the node a site names, `vdSiteNode`), or it
//! paints again and the commit diffs the new tape against the retained node, writing what changed
//! (its raw HTML, its attributes). Either way the write rides the stream to the host, and the
//! retained node is updated to it.
//!
//! ★ SO THIS FILE IS A PARTITION OF THE WRITE OPS — text, value, checked, raw HTML, attribute —
//! not a test of one of them: a path that carries four of five leaves the fifth silent, and the next
//! op added arrives with the same hole. Each row hands its commits to both hosts a page runs on
//! (`support/vdom_hosts.rs`: the native recorder and `runtime.js` under node) and reads the document
//! back from each.
//!
//! ★ AND THE RETAINED TREE MOVES WITH THE WRITE. A write the retained tree did not record leaves
//! the next diff comparing against a value no longer on screen: it re-sends what is already there,
//! or decides nothing changed when something did.

mod common;
#[path = "compile_common/mod.rs"]
mod compile_common;
#[path = "support/vdom_hosts.rs"]
mod vdom_hosts;

use logicaffeine_compile::semantics::dom::{self, DomOp};
use vdom_hosts::{paint, Paint};

/// A `<p>` holding `text` as a text node that carries `site`; an `<input id=f>` holding `value`,
/// and a checkbox, each an element carrying `site`; a `<div>` of raw `markup`; a `<p>` with a
/// `title` only when one is given.
const SHAPES: &str = "
## To para(text: Text, site: Int):
    vdOpen(\"p\").
    vdTextSite(text, site).
    vdClose().

## To field(value: Text, site: Int):
    vdOpen(\"input\").
    vdSite(site).
    vdSetAttribute(\"id\", \"f\").
    vdSetValue(value).
    vdClose().

## To box(on: Bool, site: Int):
    vdOpen(\"input\").
    vdSite(site).
    vdSetAttribute(\"type\", \"checkbox\").
    vdSetChecked(on).
    vdClose().

## To raw(markup: Text):
    vdOpen(\"div\").
    vdSetHtml(markup).
    vdClose().

## To titled(title: Text):
    vdOpen(\"p\").
    If title != \"\":
        vdSetAttribute(\"title\", title).
    vdText(\"x\").
    vdClose().
";

/// `main` after a root standing for the body and one fresh site.
fn painted(main: &str) -> Paint {
    let p = paint(
        SHAPES,
        &format!("Let root be vdNewRoot(0).\nLet site be vdNewSite().\n{main}"),
    );
    p.assert_hosts_follow();
    p
}

/// The node step `k`'s stream created with `made`, which must be exactly one.
fn created(p: &Paint, k: usize, made: impl Fn(&DomOp) -> Option<u32>) -> u32 {
    let ids: Vec<u32> = p.steps[k].ops.iter().filter_map(made).collect();
    assert_eq!(
        ids.len(),
        1,
        "step {k} creates exactly one such node: {:?}",
        p.steps[k].ops
    );
    ids[0]
}

/// Asserts that the `tag` nodes after step `k` are the very ones step `k - 1` left, on both hosts.
fn kept(p: &Paint, k: usize, tag: &str) {
    if tag != "#text" {
        assert_eq!(
            p.native_ids(k, tag),
            p.native_ids(k - 1, tag),
            "step {k}: the native `{tag}` is the node it was"
        );
    }
    if let (Some(before), Some(now)) = (p.browser(k - 1), p.browser(k)) {
        assert_eq!(
            now.uids(tag),
            before.uids(tag),
            "step {k}: the browser's `{tag}` is the object it was"
        );
    }
}

/// Asserts both hosts show `markup` after step `k`.
fn shows(p: &Paint, k: usize, markup: &str) {
    assert_eq!(
        p.steps[k].native, markup,
        "step {k}: the native recorder: {:?}",
        p.steps[k].ops
    );
    if let Some(b) = p.browser(k) {
        assert_eq!(
            b.markup, markup,
            "step {k}: the browser: {:?}",
            p.steps[k].ops
        );
    }
}

#[test]
fn setting_text_on_a_live_node_reaches_the_document() {
    // ★ The shape of every counter and every label: a text hole a turn rewrites where it stands.
    let p = painted("para(\"before\", site).\ncommit(root).\nvdPatchText(vdSiteNode(site), \"after\").\nflush(root).\n");
    let text = created(&p, 0, |op| match op {
        DomOp::CreateTextNode { node, text } if text == "before" => Some(*node),
        _ => None,
    });
    assert_eq!(
        p.steps[1].ops,
        vec![DomOp::SetText {
            node: text,
            text: "after".into()
        }],
        "one write, to the live text node"
    );
    shows(&p, 1, "<p>after</p>");
    kept(&p, 1, "p");
    kept(&p, 1, "#text");
}

#[test]
fn setting_a_value_on_a_live_node_reaches_the_document() {
    let p = painted("field(\"before\", site).\ncommit(root).\nvdPatchValue(vdSiteNode(site), \"typed\").\nflush(root).\n");
    let input = created(&p, 0, |op| match op {
        DomOp::CreateElement { node, tag } if tag == "input" => Some(*node),
        _ => None,
    });
    assert_eq!(
        p.steps[1].ops,
        vec![DomOp::SetValue {
            node: input,
            value: "typed".into()
        }],
        "one write, to the live input"
    );
    shows(&p, 1, "<input id=\"f\" value=\"typed\">");
    assert_eq!(
        p.native_at(1, || dom::get_value(dom::query_selector("#f"))),
        "typed",
        "the native input holds it"
    );
    kept(&p, 1, "input");
}

#[test]
fn setting_checked_on_a_live_node_reaches_the_document() {
    let p = painted(
        "box(false, site).\ncommit(root).\nvdPatchChecked(vdSiteNode(site), true).\nflush(root).\n",
    );
    let input = created(&p, 0, |op| match op {
        DomOp::CreateElement { node, tag } if tag == "input" => Some(*node),
        _ => None,
    });
    assert_eq!(
        p.steps[1].ops,
        vec![DomOp::SetChecked {
            node: input,
            checked: true
        }],
        "a checkbox patched in place is actually checked, by one write to the live input"
    );
    shows(&p, 1, "<input type=\"checkbox\" checked>");
    assert!(
        p.native_at(1, || dom::get_checked(dom::query_selector("input"))),
        "the native checkbox is checked"
    );
    kept(&p, 1, "input");
}

#[test]
fn setting_html_on_a_live_node_reaches_the_document() {
    let p = painted("raw(\"<i>a</i>\").\ncommit(root).\nraw(\"<b>x</b>\").\ncommit(root).\n");
    let div = created(&p, 0, |op| match op {
        DomOp::CreateElement { node, tag } if tag == "div" => Some(*node),
        _ => None,
    });
    assert_eq!(
        p.steps[1].ops,
        vec![DomOp::SetHtml {
            node: div,
            html: "<b>x</b>".into()
        }],
        "one write, to the live div"
    );
    shows(&p, 1, "<div><b>x</b></div>");
    kept(&p, 1, "div");
}

#[test]
fn setting_an_attribute_on_a_live_node_reaches_the_document() {
    let p = painted("titled(\"\").\ncommit(root).\ntitled(\"hi\").\ncommit(root).\n");
    let para = created(&p, 0, |op| match op {
        DomOp::CreateElement { node, tag } if tag == "p" => Some(*node),
        _ => None,
    });
    assert_eq!(
        p.steps[1].ops,
        vec![DomOp::SetAttribute {
            node: para,
            name: "title".into(),
            value: "hi".into()
        }],
        "one write, to the live p"
    );
    shows(&p, 1, "<p title=\"hi\">x</p>");
    kept(&p, 1, "p");
}

#[test]
fn a_live_write_updates_the_retained_tree_so_the_next_diff_is_honest() {
    // ★ THE HALF THAT IS NOT ABOUT THIS TURN. Each in-place write, then a paint drawing the value the
    // write already put on screen: the retained tree moved with the write, so the commit has nothing
    // to send. A stale retained node would re-send the write, or miss the next real change.
    for (write, repaint, markup) in [
        (
            "para(\"before\", site).\ncommit(root).\nvdPatchText(vdSiteNode(site), \"after\").\nflush(root).\n",
            "para(\"after\", site).\ncommit(root).\npara(\"later\", site).\ncommit(root).\n",
            "<p>later</p>",
        ),
        (
            "field(\"before\", site).\ncommit(root).\nvdPatchValue(vdSiteNode(site), \"typed\").\nflush(root).\n",
            "field(\"typed\", site).\ncommit(root).\nfield(\"retyped\", site).\ncommit(root).\n",
            "<input id=\"f\" value=\"retyped\">",
        ),
        (
            "box(false, site).\ncommit(root).\nvdPatchChecked(vdSiteNode(site), true).\nflush(root).\n",
            "box(true, site).\ncommit(root).\nbox(false, site).\ncommit(root).\n",
            "<input type=\"checkbox\">",
        ),
    ] {
        let p = painted(&format!("{write}{repaint}"));
        assert!(
            p.steps[2].ops.is_empty(),
            "painting what the in-place write already put there sends nothing — the retained tree \
             still holds the old value:\n{:?}",
            p.steps[2].ops
        );
        assert_eq!(p.steps[3].ops.len(), 1, "and the next real change is one write: {:?}", p.steps[3].ops);
        shows(&p, 3, markup);
    }
}

#[test]
fn a_fresh_node_carries_its_properties_into_its_mount() {
    // ★ THE NARROWNESS GUARD. The in-place path is for LIVE nodes: a node this commit creates is
    // assembled with its attribute, value and checked state as it mounts — one write each, never a
    // create and then a second write for something the same stream already said.
    let p =
        painted("titled(\"hi\").\nfield(\"v\", site).\nbox(true, vdNewSite()).\ncommit(root).\n");
    let ops = &p.steps[0].ops;
    let each = |pick: &dyn Fn(&DomOp) -> bool| ops.iter().filter(|op| pick(op)).count();
    assert_eq!(
        each(&|op| matches!(op, DomOp::SetAttribute { name, .. } if name == "title")),
        1,
        "{ops:?}"
    );
    assert_eq!(
        each(&|op| matches!(op, DomOp::SetAttribute { name, .. } if name == "id")),
        1,
        "{ops:?}"
    );
    assert_eq!(
        each(&|op| matches!(op, DomOp::SetAttribute { name, .. } if name == "type")),
        1,
        "{ops:?}"
    );
    assert_eq!(
        each(&|op| matches!(op, DomOp::SetValue { .. })),
        1,
        "{ops:?}"
    );
    assert_eq!(
        each(&|op| matches!(op, DomOp::SetChecked { .. })),
        1,
        "{ops:?}"
    );
    shows(
        &p,
        0,
        "<p title=\"hi\">x</p><input id=\"f\" value=\"v\"><input type=\"checkbox\" checked>",
    );
}
