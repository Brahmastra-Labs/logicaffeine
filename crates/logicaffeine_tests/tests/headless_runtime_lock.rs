//! ★ LOCK — the headless runtime delivers everything a browser delivers. ★
//!
//! A page does not only answer clicks. A fetch comes back, a timer fires, the URL changes, another
//! component's store moves — and each of those starts a TURN exactly as a click does: the action
//! runs, derived fields recompute, the page repaints. If the headless runtime delivers only clicks
//! then every test written against it is testing a quarter of the framework, and the other
//! three-quarters are covered by nothing at all.
//!
//! ★ THE ASSERTION IS THE RENDERED DOCUMENT, on both tiers where both tiers run the page. A page
//! query (`the text of "#n"`) is a reference-tier form the VM does not have, so asking that way
//! compares one tier against a refusal — the mistake this file's sibling `finegrained_differential_lock`
//! records. Both tiers paint into the same recorder, so the tree is the thing to compare, and it is
//! also what a browser would show.
//!
//! ★ WHAT IS DELIBERATELY REFERENCE-TIER ONLY is stated here rather than left as a silence. The
//! virtual clock lives in `## Test` blocks, which the runner executes on the interpreter tier by
//! design — so a timer is driven there and nowhere else. Saying so in a test means the next reader
//! learns the boundary from the suite instead of from a surprise.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;
use logicaffeine_compile::testrun::{run_tests, TestConfig};

/// The document the reference tier renders.
fn tw_doc(src: &str) -> String {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the tree-walker must run it:\n{src}");
    dom::render()
}

/// The document both tiers render, held equal — the runtime delivering the same event to two
/// engines must land the page in the same place.
fn both_docs(src: &str) -> String {
    let tw = tw_doc(src);
    dom::reset();
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM must run it too:\n{src}");
    assert_eq!(
        tw,
        dom::render(),
        "the two tiers settled differently:\n{src}"
    );
    tw
}

const PAGE: &str = "## Component App\n\
   \n\
   State:\n\
  \x20   Let n be 0.\n\
  \x20   Let note be \"idle\".\n\
   \n\
   Actions:\n\
  \x20   To bump:\n\
  \x20       Set n to n + 1.\n\
   \n\
   View:\n\
  \x20   <main><p id=\"n\">{n}</p><p id=\"s\">{note}</p><button id=\"b\" on-click={bump}>x</button></main>\n\
   \n\
   ## Main\n\
   Mount App.\n";

#[test]
fn a_click_is_delivered_and_the_page_settles() {
    // The baseline every other delivery is compared against: one event, one turn, one repaint.
    let doc = both_docs(&format!("{PAGE}Click \"#b\".\n"));
    assert!(
        doc.contains(">1<"),
        "the click must have run its turn: {doc}"
    );
}

#[test]
fn typing_is_delivered_to_a_bound_field() {
    // A keystroke is a delivery with a VALUE attached — the channel a click does not exercise.
    let src = "## Component App\n\
       \n\
       State:\n\
      \x20   Let draft be \"\".\n\
       \n\
       View:\n\
      \x20   <main><input id=\"d\" bind={draft}><p id=\"e\">{draft}</p></main>\n\
       \n\
       ## Main\n\
       Mount App.\n\
       Type \"milk\" into \"#d\".\n";
    let doc = both_docs(src);
    assert!(
        doc.contains("milk"),
        "the keystroke must have reached the field: {doc}"
    );
}

#[test]
fn a_route_change_starts_a_turn() {
    // `Navigate to` is a delivery like any other: the sentence that answers it runs, and the page
    // repaints. A runtime that only delivered DOM events would leave this handler silent.
    let src = "## Component App\n\
       \n\
       State:\n\
      \x20   Let note be \"idle\".\n\
       \n\
       Actions:\n\
      \x20   When the route changes:\n\
      \x20       Set note to \"routed\".\n\
       \n\
       View:\n\
      \x20   <main><p id=\"s\">{note}</p></main>\n\
       \n\
       ## Main\n\
       Mount App.\n\
       Navigate to \"/other\".\n";
    assert!(
        tw_doc(src).contains("routed"),
        "the route change must have started a turn"
    );
}

#[test]
fn a_store_action_moves_every_component_that_reads_it() {
    // ★ A STORE MESSAGE IS A DELIVERY TO SOMEBODY ELSE'S PAGE. The action belongs to the store, so
    // a component that merely READS it has no write of its own to notice — the runtime has to
    // settle it anyway, which is why T5.2's "an empty dirty set means I know of no change" rule
    // exists. Two components reading one store is the shape that proves it.
    let src = "## Store Session\n\
       \n\
       State:\n\
      \x20   Let name be \"guest\".\n\
       \n\
       Actions:\n\
      \x20   To login (who: Text):\n\
      \x20       Set name to who.\n\
       \n\
       ## Component Greeting\n\
       Use Session.\n\
       View:\n\
      \x20   <em id=\"g\">{Session's name}</em>\n\
       \n\
       ## Component App\n\
       Use Session.\n\
       Actions:\n\
      \x20   To signIn:\n\
      \x20       Call Session's login with \"ada\".\n\
       View:\n\
      \x20   <main><p id=\"p\">{Session's name}</p><Greeting/>\
       <button id=\"go\" on-click={signIn}>in</button></main>\n\
       \n\
       ## Main\n\
       Mount App.\n\
       Click \"#go\".\n";
    let doc = tw_doc(src);
    assert_eq!(
        doc.matches("ada").count(),
        2,
        "BOTH readers must have settled: {doc}"
    );
    assert!(
        !doc.contains("guest"),
        "nothing may still show the old value: {doc}"
    );
}

#[test]
fn a_fetch_completion_starts_its_own_turn() {
    // ★ THE DELIVERY THAT ARRIVES LATER. `Serve fetches with routes` answers the page's own request
    // from its own route table, so the completion is a real turn with a real answer rather than a
    // stub the test author typed — which is the only way the derived encoder and decoder are both
    // exercised.
    let src = "## A Row has:\n\
      \x20   an id, which is Int.\n\
      \x20   a text, which is Text.\n\
       \n\
       ## An HttpError has:\n\
      \x20   a status, which is Int.\n\
      \x20   a message, which is Text.\n\
       \n\
       ## Route GET /api/rows -> Seq of Row:\n\
      \x20   Return [a new Row with id 1 and text \"milk\"].\n\
       \n\
       ## Component App\n\
       \n\
       State:\n\
      \x20   Let rows be a new Seq of Row.\n\
      \x20   Let note be \"loading\".\n\
       \n\
       Actions:\n\
      \x20   When the page loads:\n\
      \x20       Get \"/api/rows\" as Seq of Row, then loaded.\n\
       \n\
      \x20   To loaded (answer: Result of Seq of Row and HttpError):\n\
      \x20       Inspect answer:\n\
      \x20           When Ok (list):\n\
      \x20               Set rows to list.\n\
      \x20               Set note to \"loaded\".\n\
      \x20           When Err (e):\n\
      \x20               Set note to e's message.\n\
       \n\
       View:\n\
      \x20   <main><p id=\"s\">{note}</p><ul>{for each r in rows:}<li>{r's text}</li>{end}</ul></main>\n\
       \n\
       ## Main\n\
       Serve fetches with routes.\n\
       Mount App.\n";
    let doc = tw_doc(src);
    assert!(
        doc.contains("loaded"),
        "the completion must have run its turn: {doc}"
    );
    assert!(
        doc.contains("milk"),
        "and the decoded row must have rendered: {doc}"
    );
}

#[test]
fn the_settle_loop_runs_a_completion_that_starts_another_fetch() {
    // ★ SETTLING IS A LOOP, NOT A STEP. A completion may send a request the route table answers,
    // whose completion sends another — a runtime that drained its queue once would stop halfway and
    // leave the page showing an intermediate state, which looks exactly like a slow network.
    let src = "## An HttpError has:\n\
      \x20   a status, which is Int.\n\
      \x20   a message, which is Text.\n\
       \n\
       ## Route GET /one -> Text:\n\
      \x20   Return \"first\".\n\
       \n\
       ## Route GET /two -> Text:\n\
      \x20   Return \"second\".\n\
       \n\
       ## Component App\n\
       \n\
       State:\n\
      \x20   Let note be \"start\".\n\
       \n\
       Actions:\n\
      \x20   When the page loads:\n\
      \x20       Get \"/one\" as Text, then first.\n\
       \n\
      \x20   To first (answer: Result of Text and HttpError):\n\
      \x20       Get \"/two\" as Text, then second.\n\
       \n\
      \x20   To second (answer: Result of Text and HttpError):\n\
      \x20       Inspect answer:\n\
      \x20           When Ok (v):\n\
      \x20               Set note to v.\n\
      \x20           When Err (e):\n\
      \x20               Set note to \"failed\".\n\
       \n\
       View:\n\
      \x20   <main><p id=\"s\">{note}</p></main>\n\
       \n\
       ## Main\n\
       Serve fetches with routes.\n\
       Mount App.\n";
    assert!(
        tw_doc(src).contains("second"),
        "the settle loop must drain a completion that started another fetch"
    );
}

#[test]
fn a_timer_fires_on_the_virtual_clock() {
    // ★ REFERENCE TIER BY DESIGN, and stated rather than silently absent: `Advance time by` moves a
    // TEST's virtual clock and lives in `## Test` blocks, which the runner executes on the
    // interpreter tier. A timer is therefore driven there and nowhere else — which is a boundary of
    // the test vocabulary, not a gap in the runtime.
    let src = "## Component App\n\
       \n\
       State:\n\
      \x20   Let n be 0.\n\
       \n\
       Actions:\n\
      \x20   Every 2 seconds:\n\
      \x20       Set n to n + 10.\n\
       \n\
       View:\n\
      \x20   <main><p id=\"n\">{n}</p></main>\n\
       \n\
       ## Test: a timer starts a turn\n\
      \x20   Mount App.\n\
      \x20   Advance time by 4 seconds.\n\
      \x20   Expect that the text of \"#n\" is equal to \"20\".\n";
    // ★ RUN THROUGH THE TEST RUNNER, which is what `## Test` blocks are for — and assert the count,
    // because a block that never ran reports zero passed and zero failed, which no "no errors"
    // check can tell apart from a block that passed.
    let report = run_tests(src, &TestConfig::default())
        .unwrap_or_else(|e| panic!("the timer test must parse:\n{src}\n=> {e}"));
    assert_eq!(
        (report.passed, report.failed),
        (1, 0),
        "the timer must have fired twice in 4 seconds"
    );
}
