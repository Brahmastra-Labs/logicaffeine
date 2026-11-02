//! ★ LOCK — a driving verb reaches whatever the reference tier reaches. ★
//!
//! There are TWO event paths on the bytecode tier, and the second is a much weaker copy of the
//! first. An element's `on-click={action}` becomes a real `addEventListener` closure: it works on
//! any element, it carries whatever it captured, and it is what a browser actually runs. A driving
//! verb — `Click "#b"` in a `## Main` or a `## Test` — instead goes through a synthesized
//! `__lgx_dispatch(selector, event)` built from STATIC arms, and `collect_dispatch_arms` emits an
//! arm only where the element carries a LITERAL `id`.
//!
//! ★ SO A TEST CAN DRIVE A PAGE THE BROWSER RUNS PERFECTLY, AND CANNOT DRIVE ONE IT ALSO RUNS
//! PERFECTLY. A button carrying `class="t"` is answered by the reference tier and refused here with
//! "this page answers no click: nothing in it attaches a handler". The page is fine; the dispatcher
//! cannot see it. Every row of a list has a COMPUTED id, so every list is in that position — and
//! since a component can now be placed more than once, a selector cannot say WHICH instance
//! rendered the element it names even when the id is literal.
//!
//! ★★ THE FIX IS A DELETION, and it needs no new builtin, no host import and no ABI change. The
//! dispatcher's premise — "this tier cannot call a closure it found at run time" — is TRUE and
//! stays true: a `Map of Text to fn(…)` is refused by the wasm backend ("unsupported indirect
//! call"), measured, so a table of listener closures cannot exist. But the dispatcher does not need
//! the closure. It needs to know WHICH HANDLER and FOR WHICH INSTANCE, and both are DATA: at each
//! `addEventListener` site the lowering records `"{node}:{event}"` against the handler's id and the
//! instance key that was in scope. `__lgx_dispatch` resolves the node at RUN TIME and looks the pair
//! up, so it matches by NODE rather than by spelling.
//!
//! What that buys, all at once: any selector the reference tier accepts, any instance, and the
//! static arm-building goes away.
//!
//! Every test below drives the SAME page on both tiers and holds the rendered documents equal —
//! because the claim is precisely that the two tiers answer the same act.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

/// Drive `src` on both tiers and hold the documents equal. The reference tier defines what an act
/// MEANS, so the bytecode tier answering it identically is the whole contract.
fn both(src: &str) -> String {
    dom::reset();
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker must run it:\n{src}");
    let doc = dom::render();
    dom::reset();
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM must answer the same act:\n{src}");
    assert_eq!(
        doc,
        dom::render(),
        "the two tiers answered differently:\n{src}"
    );
    doc
}

const COUNTER: &str = "## Component App\n\
   \n\
   State:\n\
  \x20   Let n be 0.\n\
   \n\
   Actions:\n\
  \x20   To bump:\n\
  \x20       Set n to n + 1.\n\
   \n";

#[test]
fn a_class_selector_reaches_a_handler() {
    // ★ THE SIMPLEST CASE THE OLD DISPATCHER COULD NOT SEE. Nothing about a class is exotic; it is
    // the selector a page uses when the element is one of many.
    let doc = both(&format!(
        "{COUNTER}View:\n\
        \x20   <p id=\"n\">{{n}}</p><button class=\"go\" on-click={{bump}}>x</button>\n\
         \n\
         ## Main\n\
         Mount App.\n\
         Click \".go\".\n"
    ));
    assert!(doc.contains(">1<"), "the click landed:\n{doc}");
}

#[test]
fn a_literal_id_still_reaches_its_handler() {
    // The case that always worked, held so the rewrite cannot cost it.
    let doc = both(&format!(
        "{COUNTER}View:\n\
        \x20   <p id=\"n\">{{n}}</p><button id=\"b\" on-click={{bump}}>x</button>\n\
         \n\
         ## Main\n\
         Mount App.\n\
         Click \"#b\".\n\
         Click \"#b\".\n"
    ));
    assert!(doc.contains(">2<"), "both clicks landed:\n{doc}");
}

#[test]
fn a_computed_id_reaches_its_handler() {
    // ★ EVERY ROW OF EVERY LIST IS IN THIS POSITION: its id comes from the data, so the compiler
    // cannot know the string and a static arm cannot exist for it.
    let doc = both(
        "## Component Row (rid: Text)\n\
         \n\
         State:\n\
        \x20   Let n be 0.\n\
         \n\
         Actions:\n\
        \x20   To bump:\n\
        \x20       Set n to n + 1.\n\
         \n\
         View:\n\
        \x20   <li><button id={rid} on-click={bump}>{n}</button></li>\n\
         \n\
         ## Component App\n\
         \n\
         View:\n\
        \x20   <ul><Row rid=\"only\"/></ul>\n\
         \n\
         ## Main\n\
         Mount App.\n\
         Click \"#only\".\n",
    );
    assert!(doc.contains(">1<"), "a computed id is reachable:\n{doc}");
}

#[test]
fn a_click_reaches_the_instance_that_rendered_the_element() {
    // ★★ THE LOAD-BEARING TEST, and the one a selector alone can never satisfy. Two rows, three
    // clicks: the counts must be 2 and 1. A dispatcher that resolved to the component rather than to
    // the INSTANCE answers 3 and 3 — and renders just as plausibly.
    let doc = both(
        "## Component Row (rid: Text)\n\
         \n\
         State:\n\
        \x20   Let n be 0.\n\
         \n\
         Actions:\n\
        \x20   To bump:\n\
        \x20       Set n to n + 1.\n\
         \n\
         View:\n\
        \x20   <li><button id={rid} on-click={bump}>{n}</button></li>\n\
         \n\
         ## Component App\n\
         \n\
         View:\n\
        \x20   <ul><Row rid=\"a\"/><Row rid=\"b\"/></ul>\n\
         \n\
         ## Main\n\
         Mount App.\n\
         Click \"#a\".\n\
         Click \"#a\".\n\
         Click \"#b\".\n",
    );
    assert!(
        doc.contains(">2<") && doc.contains(">1<"),
        "each row counted its own:\n{doc}"
    );
    assert!(
        !doc.contains(">3<"),
        "no row saw all three — that is a shared record:\n{doc}"
    );
}

#[test]
fn a_row_in_a_loop_answers_its_own_click() {
    // The same question where the instances come from a loop rather than from two placements.
    let doc = both(
        "## Component Row (rid: Text)\n\
         \n\
         State:\n\
        \x20   Let n be 0.\n\
         \n\
         Actions:\n\
        \x20   To bump:\n\
        \x20       Set n to n + 1.\n\
         \n\
         View:\n\
        \x20   <li><button id={rid} on-click={bump}>{n}</button></li>\n\
         \n\
         ## Component App\n\
         \n\
         State:\n\
        \x20   Let ids be [\"a\", \"b\", \"c\"].\n\
         \n\
         View:\n\
        \x20   <ul>{for each i in ids:}<Row rid={i}/>{end}</ul>\n\
         \n\
         ## Main\n\
         Mount App.\n\
         Click \"#b\".\n\
         Click \"#b\".\n",
    );
    assert!(doc.contains(">2<"), "the clicked row counted twice:\n{doc}");
    assert_eq!(
        doc.matches(">0<").count(),
        2,
        "the other rows stayed put:\n{doc}"
    );
}

#[test]
fn a_selector_nothing_renders_still_fails_loudly() {
    // ★ THE HONESTY RULE SURVIVES THE REWRITE. Matching by node rather than by spelling must not
    // turn a test's typo into a silent no-op — every later assertion would then describe a page
    // nobody touched. Both tiers must refuse, and in the same words.
    let src = format!(
        "{COUNTER}View:\n\
        \x20   <p id=\"n\">{{n}}</p><button id=\"b\" on-click={{bump}}>x</button>\n\
         \n\
         ## Main\n\
         Mount App.\n\
         Click \"#nope\".\n"
    );
    dom::reset();
    let tw = tw_outcome(&src)
        .error
        .expect("the reference tier refuses a selector nothing renders");
    dom::reset();
    let vm = vm_outcome(&src)
        .error
        .expect("the bytecode tier must refuse it too");
    assert_eq!(tw, vm, "the two tiers must refuse in the same words");
}

#[test]
fn an_element_nothing_listens_to_still_fails_loudly() {
    // The other honesty case: the element EXISTS and nothing answers a click on it. A browser would
    // do nothing; a test must be told, or it is asserting about a page it never moved.
    let src = format!(
        "{COUNTER}View:\n\
        \x20   <p id=\"n\">{{n}}</p><button id=\"b\" on-click={{bump}}>x</button><span id=\"q\">.</span>\n\
         \n\
         ## Main\n\
         Mount App.\n\
         Click \"#q\".\n"
    );
    dom::reset();
    let tw = tw_outcome(&src)
        .error
        .expect("the reference tier refuses an unanswered click");
    dom::reset();
    let vm = vm_outcome(&src)
        .error
        .expect("the bytecode tier must refuse it too");
    assert_eq!(tw, vm, "the two tiers must refuse in the same words");
}
