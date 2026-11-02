//! ★ LOCK — a component with its own state can be placed more than once. ★
//!
//! A row that knows whether it is expanded, a field that knows whether it has been touched, a card
//! with a menu that is open — a component owning a little state and appearing many times on a page
//! is the most ordinary thing in this whole model, and the bytecode tier REFUSED IT:
//!
//!     "it holds changing state AND is placed more than once, so module-level state would let two
//!      instances share one record"
//!
//! The refusal was honest and it was the right call at the time — aliasing two views onto one record
//! is the precise isolation failure the component model exists to prevent, and rendering the page
//! with two rows sharing a counter would be worse than refusing it. But it means the reference tier
//! renders a page the browser cannot, so a whole class of ordinary page could not be shipped.
//!
//! ★ THE FIX IS THE REFERENCE TIER'S OWN MODEL, not a new one: an instance is addressed by its KEY
//! PATH (`/App/Row#0`), state lives in a map from that key to the instance's record, and the key is
//! an ordinary parameter threaded through the render function, the actions and the handler closures.
//! Both tiers build the SAME key, which is why they can be compared node for node below.
//!
//! ★ AND THE ASSERTION IS ISOLATION, not merely "it renders". A lowering that shared one record
//! across both instances still renders two rows, still shows numbers, and still passes any test that
//! only checks the page is there. The question is whether clicking ONE moves ONLY that one.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

/// Drive `src` on the reference tier — which defines what a page MEANS — and answer its document.
///
/// ★ THE DRIVING VERBS ARE REFERENCE-TIER ONLY HERE, and the reason is a limit in the OTHER tier's
/// event routing rather than anything about instances. The bytecode tier answers a driving verb
/// through a synthesized `__lgx_dispatch(selector, event)` built from static arms, and
/// `collect_dispatch_arms` emits an arm only where the element carries a LITERAL `id`. Every row of
/// a list has a computed one, so a selector cannot reach it — and even with a literal id, a selector
/// says nothing about WHICH of two instances rendered the element it names.
///
/// That is T5.13: route the driving verbs through the element's own listener, as the reference tier
/// and every browser do, and the dispatcher goes away. A real page is unaffected — a browser fires
/// the closure, which carries its instance — which is why `both_render` below is the assertion that
/// matters for whether this feature works.
fn driven(src: &str) -> String {
    dom::reset();
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker must run it:\n{src}");
    dom::render()
}

/// Both tiers render `src` and must produce the SAME document — the contract that says the bytecode
/// tier builds the same page, including one instance's state per placement.
fn both_render(src: &str) -> String {
    dom::reset();
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker must run it:\n{src}");
    let doc = dom::render();
    dom::reset();
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM must render it too:\n{src}");
    assert_eq!(
        doc,
        dom::render(),
        "the two tiers rendered differently:\n{src}"
    );
    doc
}

/// A row that counts its own clicks and is told which id to wear, so a test can aim at one.
const ROW: &str = "## Component Row (rid: Text)\n\
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
   \n";

#[test]
fn two_placements_of_one_stateful_component_keep_separate_state() {
    // ★★ THE LOAD-BEARING TEST. Two rows, three clicks, and the counts must be 2 and 1 — a lowering
    // that let them share a record answers 3 and 3, and renders just as plausibly.
    let doc = driven(&format!(
        "{ROW}## Component App\n\
         \n\
         View:\n\
        \x20   <ul><Row rid=\"a\"/><Row rid=\"b\"/></ul>\n\
         \n\
         ## Main\n\
         Mount App.\n\
         Click \"#a\".\n\
         Click \"#a\".\n\
         Click \"#b\".\n"
    ));
    assert!(
        doc.contains(">2<"),
        "the row clicked twice must show 2:\n{doc}"
    );
    assert!(
        doc.contains(">1<"),
        "the row clicked once must show 1:\n{doc}"
    );
    assert!(
        !doc.contains(">3<"),
        "no row saw three clicks — that is a shared record:\n{doc}"
    );
}

#[test]
fn an_untouched_sibling_stays_at_its_initial_value() {
    // The complement, and it fails differently: sharing shows 1 in BOTH, so a test that only checked
    // the clicked row would pass against the bug.
    let doc = driven(&format!(
        "{ROW}## Component App\n\
         \n\
         View:\n\
        \x20   <ul><Row rid=\"a\"/><Row rid=\"b\"/></ul>\n\
         \n\
         ## Main\n\
         Mount App.\n\
         Click \"#a\".\n"
    ));
    assert!(doc.contains(">1<"), "the clicked row moved:\n{doc}");
    assert!(
        doc.contains(">0<"),
        "the untouched row must still read 0:\n{doc}"
    );
}

#[test]
fn a_component_placed_once_still_works() {
    // The case that already worked, held so generalising cannot cost it.
    let doc = driven(&format!(
        "{ROW}## Component App\n\
         \n\
         View:\n\
        \x20   <ul><Row rid=\"only\"/></ul>\n\
         \n\
         ## Main\n\
         Mount App.\n\
         Click \"#only\".\n\
         Click \"#only\".\n"
    ));
    assert!(
        doc.contains(">2<"),
        "a single instance still counts:\n{doc}"
    );
}

#[test]
fn a_loop_gives_every_row_its_own_state() {
    // ★ THE SHAPE REAL PAGES HAVE. A static pair can be monomorphised into two copies of everything;
    // a loop cannot, because the count is a run-time fact. This is the case that forces a real
    // instance store rather than a clever duplication.
    let doc = driven(&format!(
        "{ROW}## Component App\n\
         \n\
         State:\n\
        \x20   Let ids be [\"a\", \"b\", \"c\"].\n\
         \n\
         View:\n\
        \x20   <ul>{{for each i in ids:}}<Row rid={{i}}/>{{end}}</ul>\n\
         \n\
         ## Main\n\
         Mount App.\n\
         Click \"#b\".\n\
         Click \"#b\".\n"
    ));
    assert!(doc.contains(">2<"), "the clicked row counted twice:\n{doc}");
    assert_eq!(
        doc.matches(">0<").count(),
        2,
        "the other two rows stayed at 0:\n{doc}"
    );
}

#[test]
fn both_tiers_build_the_same_page_from_two_placements() {
    // ★★ THE ASSERTION THAT SAYS THE FEATURE WORKS IN A BROWSER. Two placements, each with its own
    // props and its own freshly-created state record, and the two tiers must produce the SAME
    // document. Driving is a separate question (see `driven`); this is whether the page exists.
    let doc = both_render(&format!(
        "{ROW}## Component App\n\
         \n\
         View:\n\
        \x20   <ul><Row rid=\"a\"/><Row rid=\"b\"/></ul>\n\
         \n\
         ## Main\n\
         Mount App.\n"
    ));
    assert!(
        doc.contains("id=\"a\""),
        "the first row rendered with its own prop:\n{doc}"
    );
    assert!(
        doc.contains("id=\"b\""),
        "the second row rendered with its own prop:\n{doc}"
    );
    assert_eq!(
        doc.matches(">0<").count(),
        2,
        "each row got its OWN fresh state:\n{doc}"
    );
}

#[test]
fn both_tiers_build_the_same_page_from_a_loop_of_rows() {
    // The shape a real list has, held across tiers. A static pair could be monomorphised; a loop
    // cannot, so this is the case that proves a real instance store rather than a duplication.
    let doc = both_render(&format!(
        "{ROW}## Component App\n\
         \n\
         State:\n\
        \x20   Let ids be [\"a\", \"b\", \"c\"].\n\
         \n\
         View:\n\
        \x20   <ul>{{for each i in ids:}}<Row rid={{i}}/>{{end}}</ul>\n\
         \n\
         ## Main\n\
         Mount App.\n"
    ));
    assert_eq!(
        doc.matches("<li>").count(),
        3,
        "three rows rendered:\n{doc}"
    );
    assert_eq!(
        doc.matches(">0<").count(),
        3,
        "each row got its own fresh state:\n{doc}"
    );
}

#[test]
fn a_turn_in_a_component_with_props_repaints_in_place_and_keeps_its_props() {
    // ★★ A REAL BUG THIS FILE HAD NO TEST FOR, found by chasing an arity error a teammate's AOT
    // lock reported. A turn repaints by calling the render function again — and a turn has neither
    // the parent element the component painted into nor the props its parent handed it. So a
    // stateful child with a prop repainted into node 0, which is `body`, and redrew without the
    // prop:
    //
    //     reference:  <ul><li><button id="r">milk-1</button></li></ul>
    //     bytecode:   <body>1<ul><li><button id="r">milk-0</button></li></ul>
    //                      ↑ stray, in body            ↑ never updated
    //
    // ★ THE SHAPE, and why it was the fourth instance of one mistake: a render function's INPUTS
    // were treated as properties of the DEFINITION when they are properties of the PLACEMENT.
    // State, sites, the parent element and the props are all per-instance; three had been converted
    // and the fourth was always going to break exactly like this.
    //
    // ★★ WHAT IS ACTUALLY LEFT, measured rather than assumed. The per-instance parent and props
    // ARE recorded and read back — the disassembly shows the render writing every site under its
    // own key and the patcher reading the same names. What still fails is the way this test DRIVES
    // the page: `Click` goes through `__lgx_dispatch`, which runs against the ROOT instance (key
    // `""`), so a child component's turn reads the ROOT's site record — still the seed, node 0,
    // which is `body`. Hence the stray fragment: the patch writes the new text into the document
    // root instead of into the row.
    //
    // ★ AND IT IS A WRONG ANSWER RATHER THAN A REFUSAL, which is why this stays here as a RED
    // instead of being quietly deferred: the dispatcher used to be merely INCOMPLETE (it could not
    // see a class or a computed id and said so loudly), and now that a child component can hold
    // state it is INCORRECT — it finds an element, resolves to the wrong instance, and paints. That
    // is the strongest argument yet for T5.13, whose RED lock lives in `dispatch_by_node_lock`.
    //
    // Asserted on BOTH tiers, because the whole failure is the compiled tier disagreeing with the
    // reference one — and asserted on the DOCUMENT, so a stray fragment anywhere in it fails.
    let src = "## Component Row (label: Text)\n\
       \n\
       State:\n\
      \x20   Let n be 0.\n\
       \n\
       Actions:\n\
      \x20   To bump:\n\
      \x20       Set n to n + 1.\n\
       \n\
       View:\n\
      \x20   <li><button id=\"r\" on-click={bump}>{label}-{n}</button></li>\n\
       \n\
       ## Component List\n\
       \n\
       View:\n\
      \x20   <ul><Row label={\"milk\"}/></ul>\n\
       \n\
       ## Main\n\
       Mount List.\n\
       Click \"#r\".\n";

    dom::reset();
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker must run it:\n{src}");
    let doc = dom::render();
    assert_eq!(
        doc, "<body><ul><li><button id=\"r\">milk-1</button></li></ul></body>",
        "the turn repainted in place, keeping its prop"
    );
    dom::reset();
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM must run it too:\n{src}");
    assert_eq!(
        dom::render(),
        doc,
        "the two tiers answered the click differently"
    );
}

#[test]
fn the_bytecode_tier_no_longer_refuses_a_repeated_stateful_component() {
    // Stated as its own test so the REFUSAL's disappearance is pinned, not merely implied by the
    // pages above happening to run.
    let src = format!(
        "{ROW}## Component App\n\
         \n\
         View:\n\
        \x20   <ul><Row rid=\"a\"/><Row rid=\"b\"/></ul>\n\
         \n\
         ## Main\n\
         Mount App.\n"
    );
    dom::reset();
    let vm = vm_outcome(&src);
    assert_eq!(
        vm.error, None,
        "the bytecode tier must render a repeated stateful component"
    );
}

#[test]
fn a_stateful_child_inside_a_stateful_parent_keeps_both_apart() {
    // Nesting is where a key built from the wrong parent silently collides.
    let doc = driven(&format!(
        "{ROW}## Component App\n\
         \n\
         State:\n\
        \x20   Let total be 0.\n\
         \n\
         Actions:\n\
        \x20   To all:\n\
        \x20       Set total to total + 10.\n\
         \n\
         View:\n\
        \x20   <div><button id=\"p\" on-click={{all}}>{{total}}</button>\
         <ul><Row rid=\"a\"/><Row rid=\"b\"/></ul></div>\n\
         \n\
         ## Main\n\
         Mount App.\n\
         Click \"#p\".\n\
         Click \"#a\".\n"
    ));
    assert!(doc.contains(">10<"), "the parent's own state moved:\n{doc}");
    assert!(doc.contains(">1<"), "the child's own state moved:\n{doc}");
    assert!(doc.contains(">0<"), "the other child stayed put:\n{doc}");
}
