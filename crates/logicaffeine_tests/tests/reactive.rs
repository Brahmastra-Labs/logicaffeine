//! Reactive state — the `State:` cells an LGX component's view re-renders from, and the turns
//! that write them.
//!
//! Each test asserts the RENDERED DOCUMENT, held equal on both native tiers: a `## Component`'s
//! `State:` field, written by an `Actions:` turn, read back by a `View:` hole. Reading the document
//! is the only honest assertion available — state lives inside a component instance, not at a
//! top level `Show` could read directly — and it is the stronger property anyway, since it covers
//! both the state and the render that shows it.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;
use logicaffeine_compile::semantics::dom::DomOp;

/// The document the reference tier renders, and the VM's held equal to it — two engines given the
/// same page and the same clicks must land it in the same place.
fn lgx_doc(src: &str) -> String {
    dom::reset();
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker must run it:\n{src}");
    let tw_doc = dom::render();
    dom::reset();
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM must run it too:\n{src}");
    assert_eq!(
        tw_doc,
        dom::render(),
        "the two tiers rendered differently:\n{src}"
    );
    tw_doc
}

fn interp_out(src: &str) -> String {
    dom::reset();
    tw_outcome(src).output.trim().to_string()
}

fn vm_out(src: &str) -> String {
    dom::reset();
    vm_outcome(src).output.trim().to_string()
}

/// A `State:` field written by a turn and read back by a hole.
#[test]
fn lgx_state_round_trips_on_interp_and_vm() {
    let doc = lgx_doc(
        "## Component App\n\
         \n\
         State:\n\
        \x20   Let count be 0.\n\
         \n\
         Actions:\n\
        \x20   To act:\n\
        \x20       Set count to 41.\n\
         \n\
         View:\n\
        \x20   <main><p id=\"c\">{count}</p><button id=\"b\" on-click={act}>x</button></main>\n\
         \n\
         ## Main\n\
         Mount App.\n\
         Click \"#b\".\n",
    );
    assert!(
        doc.contains(">41<"),
        "the page must show the written value:\n{doc}"
    );
    // ★ AND NOT THE OLD ONE. A repaint that appended instead of replacing would contain both, and
    // the assertion above alone would pass.
    assert!(!doc.contains(">0<"), "the old value must be gone:\n{doc}");
}

/// Three TURNS, each reading the field it just wrote. This is the
/// counter the whole framework is demonstrated with, and the case where a turn that read a stale
/// copy of state would answer 1 three times over.
#[test]
fn lgx_increment_reads_and_writes_the_value() {
    let doc = lgx_doc(
        "## Component App\n\
         \n\
         State:\n\
        \x20   Let count be 0.\n\
         \n\
         Actions:\n\
        \x20   To act:\n\
        \x20       Set count to count + 1.\n\
         \n\
         View:\n\
        \x20   <main><p id=\"c\">{count}</p><button id=\"b\" on-click={act}>x</button></main>\n\
         \n\
         ## Main\n\
         Mount App.\n\
         Click \"#b\".\n\
         Click \"#b\".\n\
         Click \"#b\".\n",
    );
    assert!(doc.contains(">3<"), "three turns must compound:\n{doc}");
}

/// `Increase`/`Decrease` are state writes inside a turn, and both must be SEEN by the
/// repaint — a compiler that recorded only `Set` as a write would leave the page showing 5.
#[test]
fn lgx_increase_decrease_mutates_the_value() {
    let doc = lgx_doc(
        "## Component App\n\
         \n\
         State:\n\
        \x20   Let count be 5.\n\
         \n\
         Actions:\n\
        \x20   To act:\n\
        \x20       Increase count by 3.\n\
        \x20       Decrease count by 1.\n\
         \n\
         View:\n\
        \x20   <main><p id=\"c\">{count}</p><button id=\"b\" on-click={act}>x</button></main>\n\
         \n\
         ## Main\n\
         Mount App.\n\
         Click \"#b\".\n",
    );
    assert!(doc.contains(">7<"), "both writes must land:\n{doc}");
}

/// A `Seq` field grown by `Push` inside a turn, read back through a DERIVED length.
#[test]
fn lgx_push_grows_the_collection() {
    let doc = lgx_doc(
        "## Component App\n\
         \n\
         State:\n\
        \x20   Let todos be a new Seq of Text.\n\
         \n\
         Actions:\n\
        \x20   To act:\n\
        \x20       Push \"a\" to todos.\n\
        \x20       Push \"b\" to todos.\n\
         \n\
         View:\n\
        \x20   <main><p id=\"c\">{length of todos}</p><button id=\"b\" on-click={act}>x</button></main>\n\
         \n\
         ## Main\n\
         Mount App.\n\
         Click \"#b\".\n",
    );
    assert!(doc.contains(">2<"), "both pushes must land:\n{doc}");
}

/// The list RENDERS, in insertion order. Order is the whole assertion: a loop that rendered its
/// items by set iteration would pass a count check and show the page backwards.
#[test]
fn lgx_iterates_pushed_items_in_order() {
    let doc = lgx_doc(
        "## Component App\n\
         \n\
         State:\n\
        \x20   Let todos be a new Seq of Text.\n\
         \n\
         Actions:\n\
        \x20   To act:\n\
        \x20       Push \"first\" to todos.\n\
        \x20       Push \"second\" to todos.\n\
         \n\
         View:\n\
        \x20   <main><ul>{for each t in todos:}<li>{t}</li>{end}</ul><button id=\"b\" on-click={act}>x</button></main>\n\
         \n\
         ## Main\n\
         Mount App.\n\
         Click \"#b\".\n",
    );
    let first = doc.find("first").expect("the first item must render");
    let second = doc.find("second").expect("the second item must render");
    assert!(
        first < second,
        "insertion order must survive the render:\n{doc}"
    );
}

/// The statement-level `Repeat for each <var> in <collection>` reads exactly like the
/// view's `for each …`: the optional `each` is accepted so the two loop surfaces stay
/// consistent (a bare `Repeat for <var> in …` works too). Locks the parser fix that
/// made `each` optional in `parse_repeat_statement`/`parse_for_statement`.
#[test]
fn repeat_for_each_statement_iterates_a_collection() {
    let with_each = "## Main\nLet xs be [\"a\", \"b\"].\nRepeat for each t in xs:\n    Show t.\n";
    let bare = "## Main\nLet xs be [\"a\", \"b\"].\nRepeat for t in xs:\n    Show t.\n";
    assert_eq!(interp_out(with_each), "a\nb", "tree-walker `for each`");
    assert_eq!(vm_out(with_each), "a\nb", "VM `for each`");
    assert_eq!(interp_out(bare), "a\nb", "tree-walker bare `for`");
    assert_eq!(
        interp_out(with_each),
        interp_out(bare),
        "`each` is pure optional sugar"
    );
}

/// A hole sits between literal text on both sides, which is where a renderer that
/// created one text node instead of three loses a character.
#[test]
fn lgx_state_reads_transparently_beside_literal_text() {
    let doc = lgx_doc(
        "## Component App\n\
         \n\
         State:\n\
        \x20   Let count be 7.\n\
         \n\
         Actions:\n\
        \x20   To act:\n\
        \x20       Set count to count + 1.\n\
         \n\
         View:\n\
        \x20   <main><p id=\"c\">Clicks: {count} so far</p><button id=\"b\" on-click={act}>x</button></main>\n\
         \n\
         ## Main\n\
         Mount App.\n",
    );
    assert!(
        doc.contains("Clicks: 7 so far"),
        "the hole must read in place:\n{doc}"
    );
}

fn interp_dom_log(src: &str) -> Vec<DomOp> {
    dom::reset();
    let _ = tw_outcome(src);
    dom::op_log()
}

fn vm_dom_log(src: &str) -> Vec<DomOp> {
    dom::reset();
    let _ = vm_outcome(src);
    dom::op_log()
}

/// Immediate-mode re-render, with no component model at all: a plain top-level function named
/// `render` is exported as `__logos_render` for a host-driven re-render, and calling it twice —
/// with a `## Main`-level mutable field changed in between — must show the NEW value the second
/// time, proving the field survives the re-render and the rebuilt tree reflects it. Identical on
/// both native tiers.
const RERENDER_SRC: &str = concat!(
    "## Main\n",
    "Let mutable count be 0.\n",
    "Let root be createElement(\"div\").\n",
    "Call setAttribute with root and \"id\" and \"root\".\n",
    "Call appendChild with body() and root.\n",
    "Call render.\n",
    "Call render.\n",
    "## To render ():\n",
    "    Call clearChildren with root.\n",
    "    Let label be createElement(\"span\").\n",
    "    Set the text of label to \"Count: {count}\".\n",
    "    Call appendChild with root and label.\n",
    "    Set count to count + 1.\n",
);

fn rerender_expected() -> Vec<DomOp> {
    vec![
        DomOp::CreateElement {
            node: 1,
            tag: "div".into(),
        },
        DomOp::SetAttribute {
            node: 1,
            name: "id".into(),
            value: "root".into(),
        },
        DomOp::AppendChild {
            parent: 0,
            child: 1,
        },
        // first render → "Count: 0"
        DomOp::ClearChildren { node: 1 },
        DomOp::CreateElement {
            node: 2,
            tag: "span".into(),
        },
        DomOp::SetText {
            node: 2,
            text: "Count: 0".into(),
        },
        DomOp::AppendChild {
            parent: 1,
            child: 2,
        },
        // second render → the field persisted and now reads "Count: 1"
        DomOp::ClearChildren { node: 1 },
        DomOp::CreateElement {
            node: 3,
            tag: "span".into(),
        },
        DomOp::SetText {
            node: 3,
            text: "Count: 1".into(),
        },
        DomOp::AppendChild {
            parent: 1,
            child: 3,
        },
    ]
}

#[test]
fn re_render_reflects_the_new_field_value_on_interp() {
    assert_eq!(interp_dom_log(RERENDER_SRC), rerender_expected());
}

#[test]
fn re_render_reflects_the_new_field_value_on_vm() {
    assert_eq!(vm_dom_log(RERENDER_SRC), rerender_expected());
}
