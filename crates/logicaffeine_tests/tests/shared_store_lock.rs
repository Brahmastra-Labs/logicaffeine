//! ★ LOCK — a `shared on "topic"` store is identified by its TOPIC. ★
//!
//! `## Store Board shared on "board"` parsed, was carried through the AST, and then behaved
//! exactly like a local store: the sentence claimed the state was shared and nothing shared it.
//! A declaration the runtime does not honour is the failure this campaign keeps finding — the
//! author is told their intent was understood and finds out otherwise in production.
//!
//! The semantic core is that **the topic is the identity**. Two stores on the same topic are one
//! store: a replica joining a topic ADOPTS what is already there rather than resetting it, and
//! every action publishes back. That is what makes the state shared, and it is what a relay
//! transports rather than what a relay decides.
//!
//! HONEST SCOPE: this locks convergence between replicas in ONE sealed world. The relay leg (two
//! processes over a socket) is transport on top of these semantics, and `shared_store_lock` grows
//! when it lands rather than this file pretending to cover it.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

fn rendered(src: &str) -> String {
    // The whole world. `dom::reset()` clears the document; the browser's storage holds the
    // topic-keyed records, and a shared topic legitimately OUTLIVES a mount — which is the
    // feature, and which means a test that reset only the DOM would adopt whatever a previous
    // test in the same thread had left on the topic.
    dom::reset();
    logicaffeine_compile::semantics::world::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the program must run:\n{src}");
    dom::render()
}

/// Two stores, same topic — the shape that says whether the topic is the identity.
const TWO_REPLICAS: &str = "\
## Store Alpha shared on \"room\"
State:
    Let notes be 0.
Actions:
    To add:
        Set notes to notes + 1.

## Store Beta shared on \"room\"
State:
    Let notes be 0.
Actions:
    To add:
        Set notes to notes + 1.

## Component App
Use Alpha.
Use Beta.
Actions:
    To viaAlpha:
        Call Alpha's add.
View:
    <p id=\"a\">{Alpha's notes}</p>
    <p id=\"b\">{Beta's notes}</p>
    <button id=\"go\" on-click={viaAlpha}>x</button>

## Main
Mount App.
";

#[test]
fn a_replica_sees_what_another_replica_wrote() {
    // The whole claim of `shared on`. Alpha's action must be visible through Beta, because they
    // are the same store under two names.
    let out = rendered(&format!("{TWO_REPLICAS}Click \"#go\".\n"));
    assert!(
        out.contains("<p id=\"a\">1</p>"),
        "the writer sees its own write: {out}"
    );
    assert!(
        out.contains("<p id=\"b\">1</p>"),
        "and the other replica on the same topic sees it too — this is what `shared on` means: {out}"
    );
}

#[test]
fn a_replica_joining_a_topic_adopts_what_is_already_there() {
    // A late joiner must converge onto existing state rather than reset the topic to its own
    // initializers — which would make the LAST client to load erase everyone else's work.
    let src = "\
## Store Early shared on \"room\"
State:
    Let notes be 7.

## Store Late shared on \"room\"
State:
    Let notes be 0.

## Component App
Use Early.
Use Late.
View:
    <p id=\"e\">{Early's notes}</p>
    <p id=\"n\">{Late's notes}</p>

## Main
Mount App.
";
    assert!(
        rendered(src).contains("<p id=\"n\">7</p>"),
        "the late replica adopted the topic's state: {}",
        rendered(src)
    );
}

#[test]
fn different_topics_do_not_leak_into_each_other() {
    // The other half, and the one that makes sharing safe: two rooms are two rooms.
    let src = "\
## Store Here shared on \"room-a\"
State:
    Let notes be 0.
Actions:
    To add:
        Set notes to notes + 1.

## Store There shared on \"room-b\"
State:
    Let notes be 0.

## Component App
Use Here.
Use There.
Actions:
    To go:
        Call Here's add.
View:
    <p id=\"a\">{Here's notes}</p>
    <p id=\"b\">{There's notes}</p>
    <button id=\"go\" on-click={go}>x</button>

## Main
Mount App.
Click \"#go\".
";
    let out = rendered(src);
    assert!(
        out.contains("<p id=\"a\">1</p>"),
        "the written room moved: {out}"
    );
    assert!(
        out.contains("<p id=\"b\">0</p>"),
        "the other room did NOT: {out}"
    );
}

#[test]
fn a_local_store_shares_nothing() {
    // Sharing is opt-in. Two local stores with the same shape must stay separate, or `shared on`
    // would mean nothing and every store would leak into every other.
    let src = "\
## Store Solo
State:
    Let notes be 0.
Actions:
    To add:
        Set notes to notes + 1.

## Store Duo
State:
    Let notes be 0.

## Component App
Use Solo.
Use Duo.
Actions:
    To go:
        Call Solo's add.
View:
    <p id=\"a\">{Solo's notes}</p>
    <p id=\"b\">{Duo's notes}</p>
    <button id=\"go\" on-click={go}>x</button>

## Main
Mount App.
Click \"#go\".
";
    let out = rendered(src);
    assert!(out.contains("<p id=\"a\">1</p>"), "{out}");
    assert!(
        out.contains("<p id=\"b\">0</p>"),
        "a local store shares nothing: {out}"
    );
}

#[test]
fn only_fields_a_replica_declares_are_adopted() {
    // Replicas may be different versions of the same app. A field the joiner does not declare
    // must not be forced into it, or deploying a new version breaks the old one.
    let src = "\
## Store Old shared on \"room\"
State:
    Let notes be 3.
    Let extra be 9.

## Store New shared on \"room\"
State:
    Let notes be 0.

## Component App
Use Old.
Use New.
View:
    <p id=\"o\">{Old's notes}</p>
    <p id=\"n\">{New's notes}</p>

## Main
Mount App.
";
    assert!(
        rendered(src).contains("<p id=\"n\">3</p>"),
        "the shared field converged: {}",
        rendered(src)
    );
}

/// The bytecode tier SHARES a shared store, exactly as the reference does.
///
/// The lowered page keeps a shared store as a store persisted to its TOPIC: a replica adopts the
/// topic's state when it is first read, publishes what it writes, and every other replica that has
/// joined the topic is refreshed from it — so another replica's write is visible, as it must be on
/// the tier a browser runs. A page that lowered a shared store as a plain local record would
/// SILENTLY NOT SHARE, which is the failure `shared on` exists to remove, so the replica that did
/// not write is asserted directly.
#[test]
fn the_bytecode_tier_shares_a_store_like_the_reference() {
    let src = format!("{TWO_REPLICAS}Click \"#go\".\n");
    dom::reset();
    logicaffeine_compile::semantics::world::reset();
    logicaffeine_compile::semantics::storage::reset();
    assert_eq!(tw_outcome(&src).error, None, "the reference tier runs it");
    let reference = dom::render();

    dom::reset();
    logicaffeine_compile::semantics::world::reset();
    logicaffeine_compile::semantics::storage::reset();
    let vm = vm_outcome(&src);
    assert_eq!(vm.error, None, "the VM runs a shared store");
    let lowered = dom::render();
    assert!(
        lowered.contains("<p id=\"b\">1</p>"),
        "the replica that did not write sees the write on the VM too: {lowered}"
    );
    assert_eq!(
        lowered, reference,
        "and the VM draws the page the reference draws"
    );
}

#[test]
fn a_local_store_still_runs_on_both_tiers() {
    // The refusal must be narrow: only SHARING is beyond that tier, and a local store — which is
    // most stores — has to keep working there.
    let src = "\
## Store Counter
State:
    Let n be 0.
Actions:
    To add:
        Set n to n + 1.

## Component App
Use Counter.
Actions:
    To go:
        Call Counter's add.
View:
    <p id=\"n\">{Counter's n}</p>
    <button id=\"go\" on-click={go}>x</button>

## Main
Mount App.
Click \"#go\".
";
    dom::reset();
    logicaffeine_compile::semantics::world::reset();
    assert_eq!(tw_outcome(src).error, None);
    let tw = dom::render();
    dom::reset();
    logicaffeine_compile::semantics::world::reset();
    assert_eq!(
        vm_outcome(src).error,
        None,
        "a local store still runs on the VM"
    );
    assert_eq!(dom::render(), tw, "and renders identically");
}

#[test]
fn a_replica_nobody_reads_never_joins_the_topic() {
    // ★ A SEMANTIC THE FIXTURES TAUGHT ME, worth pinning rather than working around: a store
    // mounts on first ACCESS, not at `Use`. So a replica no view ever reads does not exist yet,
    // and does not publish to its topic.
    //
    // That is defensible — an unused declaration costing nothing is the same rule the derives
    // follow — but it means `Use` is not a join. Anyone reasoning about who is on a topic needs
    // to know that, and a test is where they will find out.
    let src = "\
## Store Unread shared on \"quiet\"
State:
    Let notes be 5.

## Store Reader shared on \"quiet\"
State:
    Let notes be 0.

## Component App
Use Unread.
Use Reader.
View:
    <p id=\"n\">{Reader's notes}</p>

## Main
Mount App.
";
    assert!(
        rendered(src).contains("<p id=\"n\">0</p>"),
        "a replica nobody reads never published, so there was nothing to adopt: {}",
        rendered(src)
    );
}

/// A shared store read by a `## Page` — directly, and through a component the page places — run the
/// way `largo test` runs a page's own tests: the reference and every candidate page tier, which must
/// agree. The page mounts itself, so this is the `## Page` mount path, not a test's own `Mount`.
const PAGE_READS_A_SHARED_STORE: &str = "\
## Store Presence shared on \"chat-presence\"
State:
    Let onlineCount be 0.
    Let joins: Tally be 0.
Actions:
    To join:
        Set onlineCount to onlineCount + 1.
        Increase joins by 1.

## Component PresenceCount
Use Presence.
Actions:
    To go:
        Call Presence's join.
View:
    <span id=\"online\">{Presence's onlineCount} online</span>
    <button id=\"join\" on-click={go}>join</button>

## Page /
Use Presence.
View:
    <p id=\"joins\">{Presence's joins}</p>
    <PresenceCount/>

## Test: the page reads the shared store directly and through a placed component
Expect that the text of \"#online\" is equal to \"0 online\".
Expect that the text of \"#joins\" is equal to \"0\".
Click \"#join\".
Expect that the text of \"#online\" is equal to \"1 online\".
Expect that the text of \"#joins\" is equal to \"1\".
";

#[test]
fn a_page_reads_a_shared_store_on_every_page_tier_as_largo_test_runs_it() {
    dom::reset();
    let report = logicaffeine_compile::testrun::run_tests(
        PAGE_READS_A_SHARED_STORE,
        &logicaffeine_compile::testrun::TestConfig::default(),
    )
    .expect("the page parses");
    let outcome = &report.outcomes[0];
    assert!(
        outcome.mounted,
        "a `## Page` mounts itself, so its test is a page test"
    );
    assert_eq!(
        outcome.verdict,
        logicaffeine_compile::testrun::Verdict::Passed,
        "the reference and the lowered page tiers run it and agree: {:?}",
        outcome.failures
    );
    assert!(
        matches!(
            outcome.tier,
            Some(logicaffeine_compile::testrun::TierEvidence::Agreed)
        ),
        "the lowered VM ran the page rather than refusing it: {:?}",
        outcome.tier
    );
}

/// A page that joins its topic as it loads — the chat shape.
const JOINS_ON_LOAD: &str = "\
## Store Presence shared on \"chat-presence\"
State:
    Let onlineCount be 0.
Actions:
    To join:
        Set onlineCount to onlineCount + 1.

## Page /
Use Presence.
Actions:
    When the page loads:
        Call Presence's join.
View:
    <span id=\"online\">{Presence's onlineCount} online</span>
";

/// ★ A PROGRAM RUN IS A FRESH PROCESS ON EVERY TIER: it starts with nothing stored — a browser
/// opened for the first time — so what one run left on a shared topic is never what the next run
/// adopts. Run twice, or once per tier in one process, the page draws the same count.
#[test]
fn every_run_of_a_program_starts_from_an_empty_topic_on_every_tier() {
    use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
    let mut drawn = Vec::new();
    for (tier, run) in [
        (
            "tree-walker",
            tw_outcome as fn(&str) -> logicaffeine_compile::compile::RunOutcome,
        ),
        ("VM", vm_outcome),
        ("VM again", vm_outcome),
        ("tree-walker again", tw_outcome),
    ] {
        dom::reset();
        let out = run(JOINS_ON_LOAD);
        assert_eq!(out.error, None, "{tier} runs the page");
        drawn.push((tier, dom::render()));
    }
    for (tier, doc) in &drawn {
        assert!(
            doc.contains("<span id=\"online\">1 online</span>"),
            "{tier}: one page joined, so one is online: {doc}"
        );
    }
}
