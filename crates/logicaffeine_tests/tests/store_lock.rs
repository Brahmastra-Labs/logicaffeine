//! ★ LOCK — a `## Store` is the shared model: many components read it, only its own actions
//! change it, and a change reaches every reader. ★
//!
//! This is the Redux half of the model, in LOGOS terms. A store has state and actions and no
//! view; a component `Use`s it, reads `Store's field`, and calls `Call Store's action`. There
//! is no other door to that state.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom;

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

const SESSION: &str = "\
## Store Session
State:
    Let name be \"guest\".
Actions:
    To login (who: Text):
        Set name to who.
    To logout:
        Set name to \"guest\".
";

#[test]
fn a_component_reads_a_store_field() {
    let src = format!("{SESSION}\n## Component App\nUse Session.\nView:\n    <p>{{Session's name}}</p>\n\n## Main\nMount App.\n");
    assert_eq!(rendered(&src), "<p>guest</p>");
}

#[test]
fn calling_a_store_action_changes_what_every_reader_shows() {
    let src = format!(
        "{SESSION}\n## Component App\nUse Session.\nActions:\n    To signIn:\n        Call Session's login with \"ada\".\nView:\n    <p>{{Session's name}}</p>\n    <button id=\"go\" on-click={{signIn}}>in</button>\n\n## Main\nMount App.\nClick \"#go\".\n"
    );
    assert!(
        rendered(&src).contains("<p>ada</p>"),
        "rendered: {}",
        rendered(&src)
    );
}

#[test]
fn two_components_see_the_same_store() {
    let src = format!(
        "{SESSION}\n## Component Greeting\nUse Session.\nView:\n    <em>{{Session's name}}</em>\n\n## Component App\nUse Session.\nActions:\n    To signIn:\n        Call Session's login with \"ada\".\nView:\n    <p>{{Session's name}}</p>\n    <Greeting/>\n    <button id=\"go\" on-click={{signIn}}>in</button>\n\n## Main\nMount App.\nClick \"#go\".\n"
    );
    let out = rendered(&src);
    assert!(
        out.contains("<p>ada</p>") && out.contains("<em>ada</em>"),
        "both readers see it: {out}"
    );
}

#[test]
fn a_store_action_with_no_arguments_runs() {
    let src = format!(
        "{SESSION}\n## Component App\nUse Session.\nActions:\n    To out:\n        Call Session's logout.\nView:\n    <p>{{Session's name}}</p>\n    <button id=\"go\" on-click={{out}}>out</button>\n\n## Main\nMount App.\nClick \"#go\".\n"
    );
    assert!(
        rendered(&src).contains("<p>guest</p>"),
        "rendered: {}",
        rendered(&src)
    );
}

#[test]
fn calling_an_action_a_store_does_not_have_is_a_clean_error() {
    let src = format!(
        "{SESSION}\n## Component App\nUse Session.\nActions:\n    To bad:\n        Call Session's vanish.\nView:\n    <button id=\"go\" on-click={{bad}}>x</button>\n\n## Main\nMount App.\nClick \"#go\".\n"
    );
    let msg = failure(&src);
    assert!(
        msg.contains("vanish"),
        "the message names the action: {msg}"
    );
    assert!(msg.contains("Session"), "and the store: {msg}");
}

#[test]
fn a_change_handler_runs_when_its_field_changes() {
    let src = "\
## Component App
State:
    Let n be 0.
    Let seen be \"no\".
Actions:
    To bump:
        Set n to n + 1.
    When n changes:
        Set seen to \"yes\".
View:
    <p>{n}{seen}</p>
    <button id=\"go\" on-click={bump}>+</button>

## Main
Mount App.
";
    assert!(
        rendered(src).contains("<p>0no</p>"),
        "before the click: {}",
        rendered(src)
    );
    let clicked = format!("{src}Click \"#go\".\n");
    assert!(
        rendered(&clicked).contains("<p>1yes</p>"),
        "after the click: {}",
        rendered(&clicked)
    );
}

#[test]
fn a_change_handler_does_not_run_when_nothing_changed() {
    let src = "\
## Component App
State:
    Let n be 0.
    Let runs be 0.
Actions:
    To touch:
        Set n to 0.
    When n changes:
        Set runs to runs + 1.
View:
    <p>{runs}</p>
    <button id=\"go\" on-click={touch}>x</button>

## Main
Mount App.
Click \"#go\".
Click \"#go\".
";
    assert!(
        rendered(src).contains("<p>0</p>"),
        "setting a field to what it already held is not a change: {}",
        rendered(src)
    );
}

#[test]
fn a_change_handler_settles_in_one_round() {
    // The handler's own write does NOT start another round: a turn is one pass, so two fields
    // that each watch the other cannot loop.
    let src = "\
## Component App
State:
    Let a be 0.
    Let b be 0.
Actions:
    To bump:
        Set a to a + 1.
    When a changes:
        Set b to b + 1.
    When b changes:
        Set a to a + 100.
View:
    <p>{a}-{b}</p>
    <button id=\"go\" on-click={bump}>+</button>

## Main
Mount App.
Click \"#go\".
";
    assert!(
        rendered(src).contains("<p>1-1</p>"),
        "one round only: {}",
        rendered(src)
    );
}

#[test]
fn a_component_watches_a_store_field() {
    let src = format!(
        "{SESSION}\n## Component App\nUse Session.\nState:\n    Let greeted be \"no\".\nActions:\n    To signIn:\n        Call Session's login with \"ada\".\n    When Session's name changes:\n        Set greeted to \"yes\".\nView:\n    <p>{{greeted}}</p>\n    <button id=\"go\" on-click={{signIn}}>in</button>\n\n## Main\nMount App.\nClick \"#go\".\n"
    );
    assert!(
        rendered(&src).contains("<p>yes</p>"),
        "the watcher ran: {}",
        rendered(&src)
    );
}

#[test]
fn a_store_watcher_does_not_run_for_an_untouched_field() {
    let src = "\
## Store Counter
State:
    Let hits be 0.
    Let misses be 0.
Actions:
    To hit:
        Set hits to hits + 1.

## Component App
Use Counter.
State:
    Let saw be 0.
Actions:
    To go:
        Call Counter's hit.
    When Counter's misses changes:
        Set saw to saw + 1.
View:
    <p>{saw}</p>
    <button id=\"go\" on-click={go}>x</button>

## Main
Mount App.
Click \"#go\".
";
    assert!(
        rendered(src).contains("<p>0</p>"),
        "only the changed field wakes a watcher: {}",
        rendered(src)
    );
}

#[test]
fn a_persisted_store_survives_a_remount() {
    let src = "\
## Store Session persisted as \"session\"
State:
    Let name be \"guest\".
Actions:
    To login (who: Text):
        Set name to who.

## Component App
Use Session.
Actions:
    To signIn:
        Call Session's login with \"ada\".
View:
    <p>{Session's name}</p>
    <button id=\"go\" on-click={signIn}>in</button>

## Main
Mount App.
Click \"#go\".
Reload the page.
Mount App.
";
    assert!(
        rendered(src).contains("<p>ada</p>"),
        "a persisted store reloads its saved record: {}",
        rendered(src)
    );
}

#[test]
fn a_local_store_does_not_survive_a_remount() {
    let src = format!(
        "{SESSION}\n## Component App\nUse Session.\nActions:\n    To signIn:\n        Call Session's login with \"ada\".\nView:\n    <p>{{Session's name}}</p>\n    <button id=\"go\" on-click={{signIn}}>in</button>\n\n## Main\nMount App.\nClick \"#go\".\nReload the page.\nMount App.\n"
    );
    assert!(
        rendered(&src).contains("<p>guest</p>"),
        "a local store starts fresh: {}",
        rendered(&src)
    );
}

#[test]
fn saving_and_loading_a_value_round_trips() {
    let src = "\
## Component App
State:
    Let draft be \"\".
    Let back be \"\".
Actions:
    To keep:
        Set draft to \"milk\".
        Save draft as \"draft\".
        Set draft to \"\".
        Load \"draft\" into back.
View:
    <p>{back}</p>
    <button id=\"go\" on-click={keep}>x</button>

## Main
Mount App.
Click \"#go\".
";
    assert!(
        rendered(src).contains("<p>milk</p>"),
        "Save then Load round-trips: {}",
        rendered(src)
    );
}

#[test]
fn forgetting_a_key_clears_it() {
    let src = "\
## Component App
State:
    Let back be \"kept\".
Actions:
    To go:
        Save back as \"k\".
        Forget \"k\".
        Load \"k\" into back.
View:
    <p>{back}</p>
    <button id=\"go\" on-click={go}>x</button>

## Main
Mount App.
Click \"#go\".
";
    assert!(
        rendered(src).contains("<p></p>"),
        "a forgotten key loads as nothing: {}",
        rendered(src)
    );
}

#[test]
fn a_shared_store_behaves_like_a_local_one_until_the_relay_lands() {
    let src = "\
## Store Board shared on \"board\"
State:
    Let notes be 0.
Actions:
    To add:
        Set notes to notes + 1.

## Component App
Use Board.
Actions:
    To go:
        Call Board's add.
View:
    <p>{Board's notes}</p>
    <button id=\"go\" on-click={go}>x</button>

## Main
Mount App.
Click \"#go\".
";
    assert!(
        rendered(src).contains("<p>1</p>"),
        "a shared store still works locally: {}",
        rendered(src)
    );
}

#[test]
fn calling_a_component_as_if_it_were_a_store_is_a_clean_error() {
    let src = "\
## Component Other
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.

## Component App
Actions:
    To go:
        Call Other's bump.
View:
    <button id=\"go\" on-click={go}>x</button>

## Main
Mount App.
";
    let msg = failure(src);
    assert!(msg.contains("Other"), "the message names it: {msg}");
    assert!(
        msg.contains("store"),
        "and says a store is what `Call …'s …` reaches: {msg}"
    );
}
