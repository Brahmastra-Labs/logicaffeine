//! ★ LOCK — the page-coverage model reads each driving rule correctly, and it CAN fail. ★
//!
//! `examples_lock::every_example_page_is_exercised` holds every example page to one rule: each
//! action, `on-*` handler, trigger and `bind` it declares is driven by some `## Test` step. That
//! lock is only as good as the model under it, so every rule the model applies — a selector
//! reaching an element, a submit button submitting its form, a key filter, a prop that hands an
//! action to a child, a timer that needs the clock advanced, a fetch whose continuation needs an
//! answer — is pinned here on a page built to exercise exactly that rule, in both directions: the
//! step that drives it, and the page where nothing does.

#[path = "page_coverage/mod.rs"]
mod page_coverage;

use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};
use page_coverage::{page_coverage, Coverage, Kind};

fn coverage(src: &str) -> Coverage {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Largo.toml"), "[package]\nname = \"cov\"\n").unwrap();
    std::fs::create_dir_all(dir.path().join("pages")).unwrap();
    let page = dir.path().join("pages/app.lg");
    std::fs::write(&page, src).unwrap();
    page_coverage(&page).unwrap_or_else(|e| panic!("{e}\n{src}"))
}

/// Whether the item whose label contains `needle` is driven. Panics if no item matches, so a
/// renamed label cannot turn an assertion into one about nothing.
fn driven(c: &Coverage, needle: &str) -> bool {
    let hits: Vec<_> = c
        .items
        .iter()
        .filter(|i| i.label.contains(needle))
        .collect();
    assert!(
        !hits.is_empty(),
        "no item mentions `{needle}`: {:#?}",
        c.items
    );
    hits.iter().all(|i| i.driven)
}

const COUNTER: &str = r##"# Counter

## Page /

State:
    Let n be 0.

Actions:
    To bump:
        Set n to n + 1.
    To reset:
        Set n to 0.

View:
    <main>
      <button id="more" on-click={bump}>more</button>
      <button id="again" on-click={reset}>again</button>
      <p id="n">{n}</p>
    </main>
"##;

#[test]
fn a_click_drives_the_handler_it_lands_on_and_the_action_that_handler_calls() {
    let c = coverage(&format!("{COUNTER}\n## Test: bumps\n\nClick \"#more\".\nExpect that the text of \"#n\" is equal to \"1\".\n"));
    assert_eq!(c.tests, 1);
    assert!(driven(&c, "on-click on <button#more>"));
    assert!(driven(&c, "action `bump`"));
    assert!(
        !driven(&c, "on-click on <button#again>"),
        "nothing clicked #again"
    );
    assert!(!driven(&c, "action `reset`"), "so `reset` never ran");
    let undriven: Vec<_> = c.undriven().iter().map(|i| i.kind).collect();
    assert_eq!(
        undriven,
        vec![Kind::Action, Kind::Handler],
        "{:#?}",
        c.undriven()
    );
}

#[test]
fn a_page_with_no_tests_drives_nothing() {
    let c = coverage(COUNTER);
    assert_eq!(c.tests, 0);
    assert_eq!(c.undriven().len(), c.items.len(), "{:#?}", c.items);
    assert_eq!(
        c.items.len(),
        4,
        "two actions and two handlers are declared: {:#?}",
        c.items
    );
}

#[test]
fn an_id_with_a_hole_is_reached_by_any_id_it_can_produce() {
    let src = r##"# Board

## Page /

State:
    Let cells be ["", "", ""].

Actions:
    To play (at: Int):
        Set item at of cells to "X".

View:
    <div>
      {for each cell in cells, at index i:}<button class="cell" id="c{i}" on-click={play(i)}>{cell}</button>{end}
    </div>

## Test: plays

Click "#c2".
Expect that 1 is equal to 1.
"##;
    let c = coverage(src);
    assert!(driven(&c, "on-click on <button#c{…}.cell>"));
    assert!(driven(&c, "action `play`"));
    let missed = coverage(&src.replace("Click \"#c2\".", "Click \"#d2\"."));
    assert!(
        !driven(&missed, "action `play`"),
        "`#d2` is no id the pattern can produce"
    );
}

const LIST: &str = r##"# List

## Component Row (label: Text) and (remove: fn(Text) -> Unit)

View:
    <li><span>{label}</span><button class="del" on-click={remove(label)}>delete</button></li>

## Page /

State:
    Let items be ["a", "b"].

Actions:
    To remove (x: Text):
        Set items to items.

View:
    <ul>
      {for each it in items:}<Row label={it} remove={remove}/>{end}
    </ul>
"##;

#[test]
fn a_class_reaches_a_handler_inside_a_child_and_its_callback_prop_reaches_the_parents_action() {
    let c = coverage(&format!(
        "{LIST}\n## Test: removes\n\nClick \".del\".\nExpect that 1 is equal to 1.\n"
    ));
    assert!(driven(&c, "on-click on <button.del> (## Component Row)"));
    assert!(
        driven(&c, "action `remove`"),
        "the action the parent handed down as `remove`"
    );
    let c = coverage(&format!(
        "{LIST}\n## Test: nothing\n\nExpect that 1 is equal to 1.\n"
    ));
    assert!(!driven(&c, "action `remove`"));
}

const FORM: &str = r##"# Form

## Page /

State:
    Let draft be "".
    Let saved be "".

Actions:
    To save:
        Set saved to draft.

View:
    <form id="add" on-submit.prevent={save}>
      <label for="draft">Draft</label>
      <input id="draft" type="text" bind={draft}>
      <button type="submit">Save</button>
    </form>
"##;

#[test]
fn typing_drives_a_bind_and_submitting_drives_the_form() {
    let c = coverage(&format!("{FORM}\n## Test: saves\n\nType \"milk\" into \"#draft\".\nSubmit \"#add\".\nExpect that 1 is equal to 1.\n"));
    assert!(driven(&c, "bind `draft`"));
    assert!(driven(&c, "on-submit on <form#add>"));
    assert!(driven(&c, "action `save`"));
    let untyped = coverage(&format!(
        "{FORM}\n## Test: saves\n\nSubmit \"#add\".\nExpect that 1 is equal to 1.\n"
    ));
    assert!(
        !driven(&untyped, "bind `draft`"),
        "nothing typed into the bound input"
    );
}

#[test]
fn clicking_a_submit_button_submits_its_form() {
    let c = coverage(&format!(
        "{FORM}\n## Test: saves\n\nClick \"form button\".\nExpect that 1 is equal to 1.\n"
    ));
    assert!(
        driven(&c, "on-submit on <form#add>"),
        "a descendant selector reaches the button"
    );
    assert!(driven(&c, "action `save`"));
}

#[test]
fn a_key_filter_is_driven_only_by_its_key() {
    let page = |press: &str| {
        format!(
            "# Keys\n\n## Page /\n\nState:\n    Let n be 0.\n\nActions:\n    To add:\n        Set n to n + 1.\n\n\
             View:\n    <input id=\"d\" aria-label=\"d\" on-keydown.enter={{add}}>\n\n\
             ## Test: keys\n\n{press}\nExpect that 1 is equal to 1.\n"
        )
    };
    assert!(driven(
        &coverage(&page("Press \"Enter\" in \"#d\".")),
        "action `add`"
    ));
    assert!(
        !driven(&coverage(&page("Press \"a\" in \"#d\".")), "action `add`"),
        "`a` is not Enter"
    );
}

const TIMED: &str = r##"# Timed

## A Todo has:
    an id, which is Int.

## Page /

State:
    Let ticks be 0.
    Let loaded be 0.

Actions:
    When the page loads:
        Get "/api/boot" as Todo, then booted.
    Every 1 second:
        Set ticks to ticks + 1.
    To refresh:
        Get "/api/todos/7" as Todo, then got.
    To booted (r: Result of Todo and HttpError):
        Set loaded to 2.
    To got (r: Result of Todo and HttpError):
        Set loaded to 1.

View:
    <p id="t">{ticks}</p>
    <button id="refresh" on-click={refresh}>refresh</button>
"##;

#[test]
fn the_page_loading_is_always_driven_and_a_timer_needs_the_clock() {
    let c = coverage(&format!(
        "{TIMED}\n## Test: idle\n\nExpect that 1 is equal to 1.\n"
    ));
    assert!(
        driven(&c, "When the page loads"),
        "a page mounts itself in every test"
    );
    assert!(!driven(&c, "Every"), "nothing advanced the clock");
    assert!(!driven(&c, "action `got`"), "nothing asked for the refresh");
    let c = coverage(&format!(
        "{TIMED}\n## Test: ticks\n\nAdvance time by 2 seconds.\nExpect that 1 is equal to 1.\n"
    ));
    assert!(driven(&c, "Every"));
}

#[test]
fn a_fetch_continuation_is_driven_only_when_a_test_answers_that_request() {
    let step = |given: &str| {
        coverage(&format!(
            "{TIMED}\n## Test: refreshes\n\n{given}\nClick \"#refresh\".\nExpect that 1 is equal to 1.\n"
        ))
    };
    assert!(driven(
        &step("Given that GET \"/api/todos/7\" answers 200 with \"ok\"."),
        "action `got`"
    ));
    let wrong_url = step("Given that GET \"/api/other\" answers 200 with \"ok\".");
    assert!(
        !driven(&wrong_url, "action `got`"),
        "a stub for another URL answers nothing here"
    );
    let wrong_method = step("Given that POST \"/api/todos/7\" answers 200 with \"ok\".");
    assert!(
        !driven(&wrong_method, "action `got`"),
        "the method is part of the match"
    );
    let routed = step("Serve fetches with routes.");
    assert!(
        driven(&routed, "action `got`"),
        "the program's own routes answer every request"
    );
}

/// ★ ORDER IS PART OF THE RULE. The reference tier answers a request when it is SENT, from the
/// answers arranged by then — so a stub written after the step that sends the request answers
/// nothing. A page's load-time request is sent when the page mounts: after the scene the test
/// opens with, and before any other step.
#[test]
fn an_answer_arranged_after_the_request_answers_nothing() {
    let late = coverage(&format!(
        "{TIMED}\n## Test: late\n\nClick \"#refresh\".\nGiven that GET \"/api/todos/7\" answers 200 with \"ok\".\nExpect that 1 is equal to 1.\n"
    ));
    assert!(
        !driven(&late, "action `got`"),
        "the request went out before its answer existed"
    );
    let boot = coverage(&format!(
        "{TIMED}\n## Test: boot\n\nGiven that GET \"/api/boot\" answers 200 with \"ok\".\nExpect that 1 is equal to 1.\n"
    ));
    assert!(
        driven(&boot, "action `booted`"),
        "the scene the test opens with is arranged before the page mounts and sends its load request"
    );
    let boot_late = coverage(&format!(
        "{TIMED}\n## Test: boot late\n\nClick \"#refresh\".\nGiven that GET \"/api/boot\" answers 200 with \"ok\".\nExpect that 1 is equal to 1.\n"
    ));
    assert!(
        !driven(&boot_late, "action `booted`"),
        "the page mounted — and sent its load request — before the click, ahead of this answer"
    );
}

#[test]
fn a_selector_trigger_is_driven_by_the_verb_on_its_element() {
    let page = |step: &str| {
        format!(
            "# Search\n\n## Page /\n\nState:\n    Let q be \"\".\n    Let hits be 0.\n\n\
             Actions:\n    When \"#q\" changes:\n        Set hits to hits + 1.\n\n\
             View:\n    <input id=\"q\" aria-label=\"q\" bind={{q}}>\n\n\
             ## Test: searches\n\n{step}\nExpect that 1 is equal to 1.\n"
        )
    };
    let c = coverage(&page("Type \"tea\" into \"#q\"."));
    assert!(driven(&c, "trigger `When \"#q\""), "{:#?}", c.items);
    let c = coverage(&page("Expect that 2 is equal to 2."));
    assert!(!driven(&c, "trigger `When \"#q\""));
}

/// The coverage of a page whose own `## Test`s all PASS on the runner the model stands for — the
/// reference run `largo test` makes, with no candidate tier beside it. Each row's `Expect`s state
/// what the runner did with the page (a handler ran, or it did not), so the model's verdict on the
/// same page is checked against the runner rather than against a second copy of its rules.
fn on_the_runner(src: &str) -> Coverage {
    let config = TestConfig {
        page_tiers: Vec::new(),
        ..TestConfig::default()
    };
    let report =
        run_tests(src, &config).unwrap_or_else(|e| panic!("the page must run: {e}\n{src}"));
    let failed: Vec<_> = report
        .outcomes
        .iter()
        .filter(|o| o.verdict != Verdict::Passed)
        .collect();
    assert!(
        !report.outcomes.is_empty() && failed.is_empty(),
        "the runner must pass the page's own test, whose Expects say what it did:\n{failed:#?}\n{src}"
    );
    coverage(src)
}

const BOOT: &str = r##"# Boot

## A Todo has:
    an id, which is Int.

## Page /

State:
    Let booted be "no".

Actions:
    When the page loads:
        Get "/api/boot" as Todo, then boot.
    To boot (r: Result of Todo and HttpError):
        Set booted to "yes".
    To poke:
        Set booted to booted.

View:
    <p id="booted">{booted}</p>
    <button id="poke" on-click={poke}>poke</button>
"##;

/// ★ THE PAGE MOUNTS WHERE THE RUNNER MOUNTS IT: after the scene a test opens with. `page_host::
/// arrange` places a page's own mount after the leading `Given that … answers` / `Serve fetches
/// with routes.` / `Given a file` statements, so the load-time request meets the answer arranged
/// for it; an answer arranged after any other step comes too late for that request.
#[test]
fn the_page_mounts_after_the_scene_a_test_opens_with() {
    let answered = on_the_runner(&format!(
        "{BOOT}\n## Test: arrives answered\n\nGiven that GET \"/api/boot\" answers 200 with \"{{{{\\\"id\\\": 1}}}}\".\n\
         Expect that the text of \"#booted\" is equal to \"yes\".\n"
    ));
    assert!(
        driven(&answered, "action `boot`"),
        "the scene is arranged before the page asks: {:#?}",
        answered.items
    );
    let late = on_the_runner(&format!(
        "{BOOT}\n## Test: answered too late\n\nClick \"#poke\".\nGiven that GET \"/api/boot\" answers 200 with \"{{{{\\\"id\\\": 1}}}}\".\n\
         Expect that the text of \"#booted\" is equal to \"no\".\n"
    ));
    assert!(driven(&late, "action `poke`"));
    assert!(
        !driven(&late, "action `boot`"),
        "the page mounted before the click, and asked then"
    );
}

/// [`BOOT`] with a route that answers its load-time request.
fn boot_with_a_route() -> String {
    BOOT.replacen(
        "## Page /",
        "## Route GET /api/boot -> Todo:\n    Return a new Todo with id 1.\n\n## Page /",
        1,
    )
}

/// ★ A RELOAD MOUNTS THE PAGE AGAIN, and its load-time request meets what answers requests by
/// then — here the page's own routes, turned on after the first mount had already asked.
#[test]
fn a_reload_mounts_the_page_again() {
    let page = boot_with_a_route();
    let reloaded = on_the_runner(&format!(
        "{page}\n## Test: reloaded\n\nClick \"#poke\".\nServe fetches with routes.\nReload the page.\n\
         Expect that the text of \"#booted\" is equal to \"yes\".\n"
    ));
    assert!(driven(&reloaded, "action `boot`"), "{:#?}", reloaded.items);
    let not_reloaded = on_the_runner(&format!(
        "{page}\n## Test: not reloaded\n\nClick \"#poke\".\nServe fetches with routes.\n\
         Expect that the text of \"#booted\" is equal to \"no\".\n"
    ));
    assert!(
        !driven(&not_reloaded, "action `boot`"),
        "the one mount asked before the routes answered"
    );
}

const RAISED: &str = r##"# Raised

## Page /

State:
    Let scrolled be 0.
    Let feedScrolled be 0.
    Let dropped be "".
    Let online be 0.
    Let offline be 0.
    Let resized be 0.
    Let heard be "".

Actions:
    When the user scrolls to the bottom:
        Set scrolled to scrolled + 1.
    When the user scrolls to the bottom of "#feed":
        Set feedScrolled to feedScrolled + 1.
    When a file is dropped on "#zone" (name: Text):
        Set dropped to name.
    When the app comes back online:
        Set online to online + 1.
    When the app goes offline:
        Set offline to offline + 1.
    When the window resizes:
        Set resized to resized + 1.
    When a message arrives on "room" (m: Text):
        Set heard to m.

View:
    <div id="feed"><p id="scrolled">{scrolled}</p><p id="feed-scrolled">{feedScrolled}</p></div>
    <div id="zone">drop here</div>
    <p id="dropped">{dropped}</p>
    <p id="online">{online}</p>
    <p id="offline">{offline}</p>
    <p id="resized">{resized}</p>
    <p id="heard">{heard}</p>
"##;

/// Each trigger of [`RAISED`]: the label it carries, the element that shows whether it ran, the
/// text there when it has not, and the text after the `Raise` in the same row has run.
const RAISABLE: [(&str, &str, &str, &str, &str); 7] = [
    (
        "When the user scrolls to the bottom`",
        "#scrolled",
        "0",
        "Raise \"scrolledtobottom\".",
        "1",
    ),
    (
        "When the user scrolls to the bottom of \"#feed\"",
        "#feed-scrolled",
        "0",
        "Raise \"scrolledtobottom\" on \"#feed\".",
        "1",
    ),
    (
        "When a file is dropped on \"#zone\"",
        "#dropped",
        "",
        "Raise \"filedropped\" on \"#zone\" with \"notes.txt\".",
        "notes.txt",
    ),
    (
        "When the app comes back online",
        "#online",
        "0",
        "Raise \"online\".",
        "1",
    ),
    (
        "When the app goes offline",
        "#offline",
        "0",
        "Raise \"offline\".",
        "1",
    ),
    (
        "When the window resizes",
        "#resized",
        "0",
        "Raise \"resize\".",
        "1",
    ),
    (
        "When a message arrives on \"room\"",
        "#heard",
        "",
        "Raise \"message:room\" with \"hi\".",
        "hi",
    ),
];

/// [`RAISED`] with one test: `step`, then an `Expect` per trigger — `ran` names the one row whose
/// trigger the step reaches, if any.
fn raised_with(step: &str, ran: Option<usize>) -> String {
    let mut test = format!("{RAISED}\n## Test: raised\n\n{step}\n");
    for (i, (_, shows, idle, _, after)) in RAISABLE.iter().enumerate() {
        let want = if ran == Some(i) { after } else { idle };
        test.push_str(&format!(
            "Expect that the text of \"{shows}\" is equal to \"{want}\".\n"
        ));
    }
    test
}

/// ★ `Raise "<event>"` IS A TEST STEP. A trigger that hears something that happens TO the page —
/// scrolling to the bottom, a file dropped, connectivity, the window resizing, a message — is
/// driven by raising the event the tier delivers to it, on the node the trigger listens on (the
/// page is `"body"`). Every such trigger is a row, each raise drives exactly its own, and the
/// runner shows the same handler running.
#[test]
fn a_raised_event_drives_exactly_the_trigger_that_hears_it() {
    let idle = coverage(&format!(
        "{RAISED}\n## Test: nothing\n\nExpect that 1 is equal to 1.\n"
    ));
    let triggers: Vec<_> = idle
        .items
        .iter()
        .filter(|i| i.kind == Kind::Trigger)
        .collect();
    assert_eq!(
        triggers.len(),
        RAISABLE.len(),
        "every raisable trigger is a row: {triggers:#?}"
    );
    for (label, ..) in RAISABLE {
        assert!(!driven(&idle, label), "nothing raised `{label}`");
    }
    for (row, (label, _, _, step, _)) in RAISABLE.iter().enumerate() {
        let c = on_the_runner(&raised_with(step, Some(row)));
        assert!(
            driven(&c, label),
            "`{step}` drives `{label}`: {:#?}",
            c.items
        );
        for (other, (label2, ..)) in RAISABLE.iter().enumerate() {
            if other != row {
                assert!(
                    !driven(&c, label2),
                    "`{step}` reaches only its own trigger, not `{label2}`"
                );
            }
        }
    }
}

/// A raise reaches a trigger only through the event's own name AND the node it listens on.
#[test]
fn a_raise_with_another_name_or_on_another_node_drives_nothing() {
    for step in [
        "Raise \"scrolledtotop\".",
        "Raise \"scrolledtobottom\" on \"#zone\".",
        "Raise \"filedropped\" on \"#feed\" with \"notes.txt\".",
        "Raise \"message:lobby\" with \"hi\".",
        "Raise \"resize\" on \"#zone\".",
    ] {
        let c = on_the_runner(&raised_with(step, None));
        assert!(
            c.items
                .iter()
                .filter(|i| i.kind == Kind::Trigger)
                .all(|i| !i.driven),
            "`{step}` names no trigger's event on its node: {:#?}",
            c.items
        );
    }
}

const PUBLISHED: &str = r##"# Published

## Page /

State:
    Let heard be "".

Actions:
    To shout:
        Publish "hello" on "room".
    To mumble:
        Publish "psst" on "lobby".
    When a message arrives on "room" (m: Text):
        Set heard to m.

View:
    <button id="shout" on-click={shout}>shout</button>
    <button id="mumble" on-click={mumble}>mumble</button>
    <p id="heard">{heard}</p>
"##;

/// ★ A PAGE HEARS WHAT IT PUBLISHES. `Publish … on "<topic>"` delivers to the page's own
/// `When a message arrives on "<topic>"` as well as to the relay, so an action that publishes
/// drives that trigger — and only for its own topic.
#[test]
fn publishing_on_a_topic_drives_the_message_trigger_of_that_topic() {
    let c = on_the_runner(&format!(
        "{PUBLISHED}\n## Test: shouts\n\nClick \"#shout\".\nExpect that the text of \"#heard\" is equal to \"hello\".\n"
    ));
    assert!(
        driven(&c, "When a message arrives on \"room\""),
        "{:#?}",
        c.items
    );
    let c = on_the_runner(&format!(
        "{PUBLISHED}\n## Test: mumbles\n\nClick \"#mumble\".\nExpect that the text of \"#heard\" is equal to \"\".\n"
    ));
    assert!(driven(&c, "action `mumble`"));
    assert!(
        !driven(&c, "When a message arrives on \"room\""),
        "`lobby` is another topic"
    );
}

const FAILING: &str = r##"# Failing

## A Todo has:
    an id, which is Int.

## Page /

State:
    Let failures be 0.
    Let answered be 0.

Actions:
    To load:
        Get "/api/todo" as Todo, then loaded.
    To loaded (r: Result of Todo and HttpError):
        Set answered to answered + 1.
    When a fetch fails (url: Text) and (error: Text):
        Set failures to failures + 1.

View:
    <button id="load" on-click={load}>load</button>
    <p id="failures">{failures}</p>
    <p id="answered">{answered}</p>
"##;

/// ★ AN ANSWER OUTSIDE 2xx IS A FAILED FETCH. The action that asked receives its `Err`, and the
/// component's `When a fetch fails` runs after it; an answer in 200–299 is a success and runs only
/// the continuation. The boundary statuses are rows, and the runner agrees on each.
#[test]
fn an_answer_outside_2xx_drives_the_fetch_failure_trigger() {
    for status in [199, 200, 204, 299, 300, 404, 500] {
        let fails = !(200..300).contains(&status);
        let c = on_the_runner(&format!(
            "{FAILING}\n## Test: answered {status}\n\nGiven that GET \"/api/todo\" answers {status} with \"{{{{\\\"id\\\": 1}}}}\".\n\
             Click \"#load\".\nExpect that the text of \"#answered\" is equal to \"1\".\n\
             Expect that the text of \"#failures\" is equal to \"{}\".\n",
            u8::from(fails)
        ));
        assert!(
            driven(&c, "action `loaded`"),
            "{status}: the continuation receives every answer"
        );
        assert_eq!(
            driven(&c, "When a fetch fails"),
            fails,
            "{status}: {:#?}",
            c.items
        );
    }
    let unanswered = on_the_runner(&format!(
        "{FAILING}\n## Test: unanswered\n\nClick \"#load\".\nExpect that the text of \"#failures\" is equal to \"0\".\n"
    ));
    assert!(
        !driven(&unanswered, "When a fetch fails"),
        "a request nothing answered has not failed yet"
    );
}

const NESTED_FAILURE: &str = r##"# Nested

## A Todo has:
    an id, which is Int.

## Component Loader

State:
    Let failures be 0.

Actions:
    To load:
        Get "/api/todo" as Todo, then loaded.
    To loaded (r: Result of Todo and HttpError):
        Set failures to failures.
    When a fetch fails (url: Text) and (error: Text):
        Set failures to failures + 1.

View:
    <button id="load" on-click={load}>load</button>
    <p id="loader-failures">{failures}</p>

## Page /

State:
    Let failures be 0.

Actions:
    When a fetch fails (url: Text) and (error: Text):
        Set failures to failures + 1.

View:
    <Loader/>
    <p id="page-failures">{failures}</p>

## Test: the loader's failure

Given that GET "/api/todo" answers 500 with "down".
Click "#load".
Expect that the text of "#loader-failures" is equal to "1".
Expect that the text of "#page-failures" is equal to "0".
"##;

/// ★ A FAILURE IS HEARD BY THE COMPONENT THAT ASKED. The runner delivers it to the instance whose
/// action sent the request, so a page's own `When a fetch fails` is not driven by a child's request.
#[test]
fn a_failed_fetch_is_heard_by_the_component_that_sent_it() {
    let c = on_the_runner(NESTED_FAILURE);
    assert!(
        driven(&c, "When a fetch fails` (## Component Loader)"),
        "{:#?}",
        c.items
    );
    assert!(
        !driven(&c, "When a fetch fails` (## Page /)"),
        "the page sent nothing: {:#?}",
        c.items
    );
}

/// An `## Error page` file has no URL and no test runner mounts it, so the examples lock checks it
/// where it is served instead — which needs the model to know one when it sees one.
#[test]
fn an_error_page_file_is_known_as_one() {
    let err =
        coverage("# Gone\n\n## Error page 404\n\nView:\n    <main><h1>Nothing here</h1></main>\n");
    assert!(err.error_page_only && err.items.is_empty(), "{err:#?}");
    let page = coverage(COUNTER);
    assert!(!page.error_page_only, "a `## Page` is a page");
}
