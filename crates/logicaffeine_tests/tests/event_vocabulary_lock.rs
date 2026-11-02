//! ★ LOCK — every English event sentence fires exactly when it says, and at no other time. ★
//!
//! `Actions:` is written in sentences, not callbacks. This file holds each sentence to its
//! literal reading: `When "#add" is submitted:` answers a submit and not a click, `When the
//! user presses Escape in "#draft":` answers Escape and not Enter, and a lifecycle sentence
//! runs at its moment in the page's life. If a trigger ever drifts from its words, this goes
//! red.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::{dom, world};

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

fn failure(src: &str) -> String {
    dom::reset();
    tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("expected a failure:\n{src}"))
}

/// A component with one counter, whose `Actions:` holds `trigger`, driven by `driver`.
fn drive(trigger: &str, driver: &str) -> String {
    let src = format!(
        "## Component App\nState:\n    Let n be 0.\nActions:\n{trigger}\n        Set n to n + 1.\nView:\n    <p id=\"n\">{{n}}</p>\n    <button id=\"go\">go</button>\n    <input id=\"draft\">\n\n## Main\nMount App.\n{driver}"
    );
    rendered(&src)
}

#[test]
fn a_click_sentence_answers_a_click() {
    assert!(drive("    When \"#go\" is clicked:", "Click \"#go\".\n").contains("<p id=\"n\">1</p>"));
}

#[test]
fn a_click_sentence_does_not_answer_typing() {
    let msg = failure(&format!(
        "## Component App\nState:\n    Let n be 0.\nActions:\n    When \"#go\" is clicked:\n        Set n to n + 1.\nView:\n    <button id=\"go\">go</button>\n\n## Main\nMount App.\nType \"x\" into \"#go\".\n"
    ));
    assert!(
        msg.contains("input"),
        "nothing answers an input on it: {msg}"
    );
}

#[test]
fn a_typing_sentence_answers_typing() {
    let out = drive(
        "    When the user types in \"#draft\":",
        "Type \"x\" into \"#draft\".\n",
    );
    assert!(out.contains("<p id=\"n\">1</p>"), "rendered: {out}");
}

#[test]
fn a_changes_sentence_answers_a_change() {
    let out = drive(
        "    When \"#draft\" changes:",
        "Type \"x\" into \"#draft\".\n",
    );
    assert!(
        out.contains("<p id=\"n\">1</p>"),
        "an input's change is its typing: {out}"
    );
}

#[test]
fn a_page_loads_sentence_runs_before_the_first_paint() {
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    When the page loads:
        Set n to 41.
View:
    <p id=\"n\">{n}</p>

## Main
Mount App.
";
    assert!(
        rendered(src).contains("<p id=\"n\">41</p>"),
        "what it sets is what the page shows"
    );
}

#[test]
fn a_field_change_sentence_answers_that_field_only() {
    let src = "\
## Component App
State:
    Let a be 0.
    Let b be 0.
    Let saw be \"\".
Actions:
    To bumpB:
        Set b to b + 1.
    When a changes:
        Set saw to \"a\".
View:
    <p id=\"n\">{saw}</p>
    <button id=\"go\" on-click={bumpB}>x</button>

## Main
Mount App.
Click \"#go\".
";
    assert!(
        rendered(src).contains("<p id=\"n\"></p>"),
        "b changed, not a: {}",
        rendered(src)
    );
}

#[test]
fn an_action_fails_sentence_answers_a_trap_and_nothing_else() {
    let src = "\
## Component App
State:
    Let n be 0.
    Let error be \"\".
Actions:
    To fine:
        Set n to 1.
    To bad:
        Set n to item 9 of [1].
    When an action fails (e: Text):
        Set error to \"caught\".
View:
    <p id=\"n\">{n}-{error}</p>
    <button id=\"ok\" on-click={fine}>ok</button>
    <button id=\"no\" on-click={bad}>no</button>

## Main
Mount App.
Click \"#ok\".
";
    assert!(
        rendered(src).contains("<p id=\"n\">1-</p>"),
        "a clean turn does not call it: {}",
        rendered(src)
    );
    let with_trap = src.replace("Click \"#ok\".", "Click \"#ok\".\nClick \"#no\".");
    assert!(
        rendered(&with_trap).contains("<p id=\"n\">1-caught</p>"),
        "a trapping turn does, and rolls back: {}",
        rendered(&with_trap)
    );
}

#[test]
fn an_every_sentence_answers_the_clock_and_not_a_click() {
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    Every 10 seconds:
        Set n to n + 1.
View:
    <p id=\"n\">{n}</p>
    <button id=\"go\">x</button>

## Main
Mount App.
";
    assert!(
        rendered(src).contains("<p id=\"n\">0</p>"),
        "no clock, no tick: {}",
        rendered(src)
    );
}

#[test]
fn an_on_directive_and_a_when_sentence_lower_to_the_same_thing() {
    let by_sentence = drive("    When \"#go\" is clicked:", "Click \"#go\".\n");
    let by_directive = rendered(
        "## Component App\nState:\n    Let n be 0.\nActions:\n    To bump:\n        Set n to n + 1.\nView:\n    <p id=\"n\">{n}</p>\n    <button id=\"go\" on-click={bump}>go</button>\n    <input id=\"draft\">\n\n## Main\nMount App.\nClick \"#go\".\n",
    );
    assert!(by_sentence.contains("<p id=\"n\">1</p>"));
    assert!(by_directive.contains("<p id=\"n\">1</p>"));
}

#[test]
fn a_sentence_naming_a_selector_the_view_does_not_render_says_so() {
    let msg = failure(
        "## Component App\nState:\n    Let n be 0.\nActions:\n    When \"#missing\" is clicked:\n        Set n to 1.\nView:\n    <p>x</p>\n\n## Main\nMount App.\nClick \"#missing\".\n",
    );
    assert!(
        msg.contains("#missing"),
        "the message names the selector: {msg}"
    );
}

#[test]
fn a_handler_naming_an_action_the_component_does_not_have_says_so() {
    let src = "\
## Component App
State:
    Let n be 0.
    Let error be \"\".
Actions:
    To host:
        Set n to n + 1.
    When an action fails (e: Text):
        Set error to e.
View:
    <p id=\"e\">{error}</p>
    <button id=\"go\" on-click={ghost}>x</button>

## Main
Mount App.
Click \"#go\".
";
    let reference = failure(src);
    assert!(
        reference.contains("`ghost` is not an action of `App`"),
        "the page is refused before it runs, naming the handler and its component: {reference}"
    );
    assert!(
        reference.contains("did you mean `host`"),
        "the refusal offers the action it resembles: {reference}"
    );
    let vm = vm_outcome(src)
        .error
        .unwrap_or_else(|| panic!("the bytecode tier must refuse it too:\n{src}"));
    assert_eq!(
        vm, reference,
        "both tiers refuse the misnamed handler in the same words"
    );
}

#[test]
fn each_loop_handler_carries_its_own_item() {
    let src = "\
## Component App
State:
    Let items be [\"a\", \"b\", \"c\"].
    Let picked be \"\".
Actions:
    To pick (what: Text):
        Set picked to what.
View:
    <p id=\"p\">{picked}</p>
    <ul>{for each x in items:}<li class=\"row\" on-click={pick(x)}>{x}</li>{end}</ul>

## Main
Mount App.
Click \"[class=row]\".
";
    assert!(
        rendered(src).contains("<p id=\"p\">a</p>"),
        "the first row's own item: {}",
        rendered(src)
    );
}

// ── lifecycle, keys and navigation ───────────────────────────────────────────────────────

fn output(src: &str) -> Vec<String> {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the program must run cleanly:\n{src}");
    out.output.lines().map(str::to_string).collect()
}

#[test]
fn appears_runs_when_the_component_first_renders() {
    let src = "\
## Component App
Actions:
    When this component appears:
        Show \"here\".
View:
    <p>x</p>

## Main
Mount App.
";
    assert_eq!(output(src), ["here"]);
}

#[test]
fn disappears_runs_when_the_component_leaves_the_page() {
    let src = "\
## Component Row
Actions:
    When this component appears:
        Show \"in\".
    When this component disappears:
        Show \"out\".
View:
    <li>r</li>

## Component App
State:
    Let showing be true.
Actions:
    To hide:
        Toggle showing.
View:
    <button id=\"go\" on-click={hide}>x</button>
    {if showing:}<Row/>{end}

## Main
Mount App.
Click \"#go\".
";
    assert_eq!(output(src), ["in", "out"]);
}

#[test]
fn a_component_that_comes_back_appears_again_with_fresh_state() {
    let src = "\
## Component Row
State:
    Let n be 0.
Actions:
    When this component appears:
        Set n to n + 1.
View:
    <li class=\"r\">{n}</li>

## Component App
State:
    Let showing be true.
Actions:
    To toggle:
        Toggle showing.
View:
    <button id=\"go\" on-click={toggle}>x</button>
    {if showing:}<Row/>{end}

## Main
Mount App.
Click \"#go\".
Click \"#go\".
";
    let html = {
        dom::reset();
        let out = tw_outcome(src);
        assert_eq!(out.error, None, "must run cleanly");
        dom::render()
    };
    assert!(
        html.contains("<li class=\"r\">1</li>"),
        "a returning instance starts over: {html}"
    );
}

#[test]
fn a_disappearing_components_timer_stops_with_it() {
    let src = "\
## Component Row
Actions:
    Every 10 seconds:
        Show \"tick\".
View:
    <li>r</li>

## Component App
State:
    Let showing be true.
Actions:
    To hide:
        Toggle showing.
View:
    <button id=\"go\" on-click={hide}>x</button>
    {if showing:}<Row/>{end}

## Test: A timer leaves with its component
    Mount App.
    Advance time by 10 seconds.
    Click \"#go\".
    Advance time by 100 seconds.
    Expect that the count of \"li\" is 0.
";
    dom::reset();
    let report = logicaffeine_compile::testrun::run_tests(
        src,
        &logicaffeine_compile::testrun::TestConfig::default(),
    )
    .expect("the suite parses");
    assert_eq!(
        report.outcomes[0].verdict,
        logicaffeine_compile::testrun::Verdict::Passed,
        "{:?}",
        report.outcomes[0].failures
    );
    // The timer ticked once while its component was on the page, and not again after it left.
    assert_eq!(
        world::journal().len(),
        2,
        "one timer turn and one click turn, no ticks after the component left: {:?}",
        world::journal()
    );
}

#[test]
fn a_key_sentence_answers_that_key_and_no_other() {
    let src = "\
## Component App
State:
    Let n be 0.
    Let escaped be 0.
Actions:
    When the user presses Enter in \"#draft\":
        Set n to n + 1.
    When the user presses Escape in \"#draft\":
        Set escaped to escaped + 1.
View:
    <p id=\"n\">{n}-{escaped}</p>
    <input id=\"draft\">

## Main
Mount App.
Press \"Enter\" in \"#draft\".
Press \"Escape\" in \"#draft\".
Press \"Enter\" in \"#draft\".
";
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "must run cleanly");
    assert!(
        dom::render().contains("<p id=\"n\">2-1</p>"),
        "each key ran its own: {}",
        dom::render()
    );
}

#[test]
fn a_page_level_key_sentence_answers_a_press_anywhere() {
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    When the user presses Escape:
        Set n to n + 1.
View:
    <p id=\"n\">{n}</p>

## Main
Mount App.
Press \"Escape\".
";
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "must run cleanly");
    assert!(
        dom::render().contains("<p id=\"n\">1</p>"),
        "rendered: {}",
        dom::render()
    );
}

#[test]
fn a_key_modifier_on_a_directive_answers_the_same_way_as_the_sentence() {
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    To add:
        Set n to n + 1.
View:
    <p id=\"n\">{n}</p>
    <input id=\"draft\" on-keydown.enter={add}>

## Main
Mount App.
Press \"Enter\" in \"#draft\".
";
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "must run cleanly");
    assert!(
        dom::render().contains("<p id=\"n\">1</p>"),
        "rendered: {}",
        dom::render()
    );
}

#[test]
fn a_key_the_page_does_not_wait_for_says_so() {
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    When the user presses Enter in \"#draft\":
        Set n to n + 1.
View:
    <input id=\"draft\">

## Main
Mount App.
Press \"Escape\" in \"#draft\".
";
    let msg = failure(src);
    assert!(msg.contains("Escape"), "the message names the key: {msg}");
}

#[test]
fn a_route_change_sentence_answers_navigation() {
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    To go:
        Navigate to \"/todos\".
    When the route changes:
        Set n to n + 1.
View:
    <p id=\"n\">{n}</p>
    <button id=\"b\" on-click={go}>x</button>

## Main
Mount App.
Click \"#b\".
";
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "must run cleanly");
    assert!(
        dom::render().contains("<p id=\"n\">1</p>"),
        "rendered: {}",
        dom::render()
    );
}

#[test]
fn the_url_reads_back_after_navigation() {
    let src = "\
## Component App
Actions:
    To go:
        Navigate to \"/todos\".
View:
    <button id=\"b\" on-click={go}>x</button>

## Main
Mount App.
Click \"#b\".
Show the URL.
";
    assert_eq!(output(src), ["/todos"]);
}

/// ★ The honesty rule: every `Actions:` sentence parses, and any the runtime cannot yet fire
/// refuses at MOUNT rather than sitting in the page doing nothing. A silent no-op is the worst
/// outcome for the person who wrote the sentence; a loud refusal naming what it needs is the
/// best one short of delivering it.
/// ★ HELD TO `Trigger::undeliverable` BY `the_undeliverable_list_is_the_compilers_own`, because
/// this was the THIRD hand-kept copy of that table and copies rot in one direction: a trigger that
/// LANDS leaves a row here still demanding a refusal for something the tier now does.
/// `When a fetch fails …:` is exactly how that showed — it was delivered, and this file went red
/// insisting it could not be.
/// ★ EMPTY. `Message` was the only row, and this file's own instruction for the day it landed was
/// "some tier fires it now, so this row demands a refusal for a sentence that works. Delete it."
///
/// The list survives its last row because the checks around it are what give an empty list force:
/// they hold this catalogue and `Trigger::undeliverable` to each other in BOTH directions, so a
/// sentence cannot start refusing without a row appearing here, or stop refusing without one
/// going.
const UNDELIVERABLE: &[(&str, &str, &str)] = &[];

#[test]
fn a_sentence_this_tier_cannot_fire_refuses_instead_of_doing_nothing() {
    // ★ NOTHING TO CHECK, AND THE REASON IS THE POINT. Every sentence the language declares now
    // fires on some tier, so this loop is empty — and an empty loop that just passes is exactly
    // the silent no-op this file exists to prevent. The assertion states the fact that makes the
    // emptiness meaningful, so the day a row returns, the loop is doing work again and says so.
    assert!(
        UNDELIVERABLE.is_empty(),
        "`UNDELIVERABLE` has rows again — the loop below is live, which is fine; update this note"
    );
    for (_, sentence, needle) in UNDELIVERABLE {
        let src = format!(
            "## Component App\nState:\n    Let n be 0.\nActions:\n    {sentence}\n        Set n to 1.\nView:\n    <p>{{n}}</p>\n\n## Main\nMount App.\n"
        );
        let msg = failure(&src);
        assert!(
            msg.contains(needle) && msg.contains("App"),
            "`{sentence}` must refuse loudly and say what it needs: {msg}"
        );
    }
}

/// ★ AND THE LIST IS THE COMPILER'S, NOT THIS FILE'S.
///
/// `Trigger::undeliverable` is the one table the reference tier's mount refusal and the bytecode
/// tier's component refusal both read. A row here that the table no longer names is a test
/// demanding a refusal for a sentence that now WORKS — which is not a caught regression, it is a
/// lock holding the language back. A variant the table names with no row here is a refusal whose
/// wording nothing checks. Both go red now, with the reason in the message.
#[test]
fn the_undeliverable_list_is_the_compilers_own() {
    let ast = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../logicaffeine_language/src/ast/component.rs"),
    )
    .expect("the Trigger AST must be readable — it is the source of this list");
    let start = ast
        .find("pub fn undeliverable")
        .expect("`Trigger::undeliverable` must exist");
    let rest = &ast[start..];
    let body = &rest[..rest.find("\n    }").expect("its body must close")];
    let mut named: Vec<&str> = body
        .match_indices("Trigger::")
        .map(|(i, _)| {
            let tail = &body[i + "Trigger::".len()..];
            let n = tail
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(tail.len());
            &tail[..n]
        })
        .collect();
    named.sort_unstable();
    named.dedup();

    for variant in &named {
        assert!(
            UNDELIVERABLE.iter().any(|(n, _, _)| n == variant),
            "`Trigger::{variant}` refuses at mount and no row here checks its wording — the whole \
             point of this list is that the refusal says what the sentence NEEDS"
        );
    }
    for (variant, sentence, _) in UNDELIVERABLE {
        assert!(
            named.contains(variant),
            "`Trigger::{variant}` is no longer undeliverable — some tier fires `{sentence}` now, \
             so this row demands a refusal for a sentence that works. Delete it."
        );
    }
}
