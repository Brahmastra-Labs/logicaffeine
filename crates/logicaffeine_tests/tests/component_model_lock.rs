//! ★ LOCK — the component model itself: props down, callbacks up, state that belongs to an
//! INSTANCE rather than to a render. ★
//!
//! This is the file that says LGX is not hooks in disguise. State identity is the instance's
//! key path, so adding a field cannot shift anyone else's data and two siblings of the same
//! component never share a cell. A prop is read-only. `When the page loads` runs once per
//! instance, however many times the page repaints. And a child talks back to its parent by
//! calling a prop, which runs on the PARENT's record — the only direction data flows.

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

fn output(src: &str) -> Vec<String> {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the program must run cleanly:\n{src}");
    out.output.lines().map(str::to_string).collect()
}

fn failure(src: &str) -> String {
    dom::reset();
    tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("expected a failure:\n{src}"))
}

#[test]
fn a_prop_flows_down_to_the_child() {
    let src = "\
## Component Greeting (who: Text)
View:
    <p>Hi {who}</p>

## Component App
State:
    Let name be \"ada\".
View:
    <Greeting who={name}/>

## Main
Mount App.
";
    assert_eq!(rendered(src), "<p>Hi ada</p>");
}

#[test]
fn a_state_initializer_may_read_a_prop() {
    let src = "\
## Component Greeting (who: Text)
State:
    Let shout always be who + \"!\".
View:
    <p>{shout}</p>

## Component App
View:
    <Greeting who={\"ada\"}/>

## Main
Mount App.
";
    assert_eq!(rendered(src), "<p>ada!</p>");
}

#[test]
fn two_siblings_of_one_component_keep_separate_state() {
    // No keys, no discipline to remember: identity is the instance's path in the tree.
    let src = "\
## Component Counter
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <li class=\"n\" on-click={bump}>{n}</li>

## Component App
View:
    <ul><Counter/><Counter/></ul>

## Main
Mount App.
Click \"li\".
";
    assert_eq!(
        rendered(src),
        "<ul><li class=\"n\">1</li><li class=\"n\">0</li></ul>"
    );
}

#[test]
fn adding_a_state_field_does_not_shift_another_components_data() {
    let src = "\
## Component Counter
State:
    Let label be \"c\".
    Let n be 7.
View:
    <li>{label}{n}</li>

## Component Other
State:
    Let n be 1.
View:
    <li>{n}</li>

## Component App
View:
    <ul><Counter/><Other/></ul>

## Main
Mount App.
";
    assert_eq!(rendered(src), "<ul><li>c7</li><li>1</li></ul>");
}

#[test]
fn when_the_page_loads_runs_once_however_often_the_page_repaints() {
    let src = "\
## Component App
State:
    Let loads be 0.
    Let clicks be 0.
Actions:
    When the page loads:
        Set loads to loads + 1.
    To bump:
        Set clicks to clicks + 1.
View:
    <p id=\"n\">{loads}-{clicks}</p>
    <button id=\"go\" on-click={bump}>+</button>

## Main
Mount App.
Click \"#go\".
Click \"#go\".
Click \"#go\".
";
    assert_eq!(
        rendered(src),
        "<p id=\"n\">1-3</p><button id=\"go\">+</button>"
    );
}

#[test]
fn state_survives_a_re_render() {
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <p id=\"n\">{n}</p>
    <button id=\"go\" on-click={bump}>+</button>

## Main
Mount App.
Click \"#go\".
Click \"#go\".
";
    assert!(
        rendered(src).contains("<p id=\"n\">2</p>"),
        "rendered: {}",
        rendered(src)
    );
}

#[test]
fn a_derived_field_recomputes_after_every_turn() {
    let src = "\
## Component App
State:
    Let n be 1.
    Let doubled always be n * 2.
Actions:
    To bump:
        Set n to n + 1.
View:
    <p id=\"n\">{doubled}</p>
    <button id=\"go\" on-click={bump}>+</button>

## Main
Mount App.
Click \"#go\".
";
    assert!(
        rendered(src).contains("<p id=\"n\">4</p>"),
        "rendered: {}",
        rendered(src)
    );
}

#[test]
fn a_child_calls_back_into_its_parent_through_a_prop() {
    // The ONE direction data flows: a child never reaches into a parent's state, it calls an
    // action the parent handed it, which runs on the PARENT's record.
    let src = "\
## Component Row (label: Text) and (remove: Action)
View:
    <li class=\"row\" on-click={remove(label)}>{label}</li>

## Component App
State:
    Let items be [\"a\", \"b\"].
    Let gone be \"\".
Actions:
    To drop (what: Text):
        Set gone to what.
View:
    <p id=\"gone\">{gone}</p>
    <ul>{for each x in items:}<Row label={x} remove={drop}/>{end}</ul>

## Main
Mount App.
Click \".row\".
";
    assert!(
        rendered(src).contains("<p id=\"gone\">a</p>"),
        "the parent's action ran: {}",
        rendered(src)
    );
}

#[test]
fn a_callback_runs_against_the_parents_record_not_the_childs() {
    let src = "\
## Component Row (hit: Action)
State:
    Let n be 100.
View:
    <li class=\"row\" on-click={hit}>{n}</li>

## Component App
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <p id=\"n\">{n}</p>
    <Row hit={bump}/>

## Main
Mount App.
Click \".row\".
";
    let out = rendered(src);
    assert!(
        out.contains("<p id=\"n\">1</p>"),
        "the parent's n moved: {out}"
    );
    assert!(
        out.contains("<li class=\"row\">100</li>"),
        "the child's n did not: {out}"
    );
}

#[test]
fn setting_a_prop_is_refused_with_a_message_that_says_why() {
    let src = "\
## Component Greeting (who: Text)
Actions:
    To rename:
        Set who to \"bob\".
View:
    <p id=\"go\" on-click={rename}>{who}</p>

## Component App
View:
    <Greeting who={\"ada\"}/>

## Main
Mount App.
";
    let msg = failure(src);
    assert!(msg.contains("who"), "the message names the prop: {msg}");
    assert!(msg.contains("prop"), "and says it is a prop: {msg}");
}

#[test]
fn an_unknown_component_tag_names_itself() {
    let src = "## Component App\nView:\n    <Ghost/>\n\n## Main\nMount App.\n";
    let msg = failure(src);
    assert!(msg.contains("Ghost"), "the message names the tag: {msg}");
}

#[test]
fn a_component_that_disappears_and_returns_is_a_new_instance() {
    // Instance identity is the path in the tree, so a path that stopped rendering holds no
    // state to carry: what comes back is a NEW component and says so by mounting again. The
    // alternative — a component that looks fresh but is not — is the bug this rule prevents.
    let src = "\
## Component Counter
State:
    Let n be 0.
Actions:
    When the page loads:
        Show \"mounted\".
View:
    <li>{n}</li>

## Component App
State:
    Let showing be true.
Actions:
    To toggle:
        Toggle showing.
View:
    <button id=\"go\" on-click={toggle}>x</button>
    {if showing:}<Counter/>{end}

## Main
Mount App.
Click \"#go\".
Click \"#go\".
";
    assert_eq!(
        output(src),
        ["mounted", "mounted"],
        "it left and came back, so it mounted twice — identity died with the path"
    );
}

// ── keyed loops: identity follows the ITEM, not the position ─────────────────────────────

#[test]
fn an_unkeyed_loop_gives_each_position_its_own_state() {
    // Without a key, identity is the position — which is the right default for a list whose
    // rows do not move, and exactly the wrong one for a list that reorders. That is why the key
    // exists, and why this test and the next one differ only in one clause.
    let src = "\
## Component Row (label: Text)
State:
    Let clicks be 0.
Actions:
    To bump:
        Set clicks to clicks + 1.
View:
    <li class=\"row\" on-click={bump}>{label}{clicks}</li>

## Component App
State:
    Let items be [\"a\", \"b\"].
Actions:
    To flip:
        Set items to [\"b\", \"a\"].
View:
    <ul>{for each x in items:}<Row label={x}/>{end}</ul>
    <button id=\"go\" on-click={flip}>flip</button>

## Main
Mount App.
Click \".row\".
Click \"#go\".
";
    let out = rendered(src);
    assert!(
        out.contains("<li class=\"row\">b1</li>"),
        "the FIRST POSITION keeps the click: {out}"
    );
}

#[test]
fn a_keyed_loop_carries_state_with_the_item_through_a_reorder() {
    let src = "\
## Component Row (label: Text)
State:
    Let clicks be 0.
Actions:
    To bump:
        Set clicks to clicks + 1.
View:
    <li class=\"row\" on-click={bump}>{label}{clicks}</li>

## Component App
State:
    Let items be [\"a\", \"b\"].
Actions:
    To flip:
        Set items to [\"b\", \"a\"].
View:
    <ul>{for each x in items, keyed by x:}<Row label={x}/>{end}</ul>
    <button id=\"go\" on-click={flip}>flip</button>

## Main
Mount App.
Click \".row\".
Click \"#go\".
";
    let out = rendered(src);
    assert!(
        out.contains("<li class=\"row\">b0</li>"),
        "`b` is first now and never was clicked: {out}"
    );
    assert!(
        out.contains("<li class=\"row\">a1</li>"),
        "`a` moved and kept its click: {out}"
    );
}

#[test]
fn a_keyed_loop_starts_a_new_row_fresh() {
    let src = "\
## Component Row (label: Text)
State:
    Let clicks be 0.
Actions:
    To bump:
        Set clicks to clicks + 1.
View:
    <li class=\"row\" on-click={bump}>{label}{clicks}</li>

## Component App
State:
    Let items be [\"a\"].
Actions:
    To add:
        Set items to [\"c\", \"a\"].
View:
    <ul>{for each x in items, keyed by x:}<Row label={x}/>{end}</ul>
    <button id=\"go\" on-click={add}>add</button>

## Main
Mount App.
Click \".row\".
Click \"#go\".
";
    let out = rendered(src);
    assert!(
        out.contains("<li class=\"row\">c0</li>"),
        "a row that never existed starts at zero: {out}"
    );
    assert!(
        out.contains("<li class=\"row\">a1</li>"),
        "and `a` keeps what it had: {out}"
    );
}

#[test]
fn a_loop_index_names_the_position() {
    let src = "\
## Component App
State:
    Let items be [\"a\", \"b\", \"c\"].
View:
    <ul>{for each x in items, at index i:}<li>{i}:{x}</li>{end}</ul>

## Main
Mount App.
";
    assert_eq!(
        rendered(src),
        "<ul><li>1:a</li><li>2:b</li><li>3:c</li></ul>"
    );
}

#[test]
fn a_row_that_leaves_a_keyed_loop_takes_its_state_with_it() {
    let src = "\
## Component Row (label: Text)
State:
    Let clicks be 0.
Actions:
    To bump:
        Set clicks to clicks + 1.
View:
    <li class=\"row\" on-click={bump}>{label}{clicks}</li>

## Component App
State:
    Let items be [\"a\", \"b\"].
Actions:
    To drop:
        Set items to [\"b\"].
    To restore:
        Set items to [\"a\", \"b\"].
View:
    <ul>{for each x in items, keyed by x:}<Row label={x}/>{end}</ul>
    <button id=\"d\" on-click={drop}>d</button>
    <button id=\"r\" on-click={restore}>r</button>

## Main
Mount App.
Click \".row\".
Click \"#d\".
Click \"#r\".
";
    let out = rendered(src);
    assert!(
        out.contains("<li class=\"row\">a0</li>"),
        "a row that left and came back is new: {out}"
    );
}
