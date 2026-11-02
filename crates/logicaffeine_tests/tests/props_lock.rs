//! ★ LOCK — a child component's scope is its OWN: its props, its state, its program's globals,
//! and never a caller's locals. ★
//!
//! A child renders through its own `__render_<C>`, whose scope is its props, its state and the
//! program's globals, and a turn against a mounted instance runs as that instance's own action,
//! reading only its OWN fields — neither copies anything from whatever was in scope where the
//! placement or the click happened. This locks both doors: a
//! child's VIEW cannot read a name it did not declare (props, state, or its template's own
//! bindings), and neither can a child's ACTION.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom;

fn rendered(src: &str) -> String {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(
        out.error, None,
        "the program must run cleanly:\n{src}\n{out:?}"
    );
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

/// A child's ACTION cannot read a name the PARENT declared in its own `State:` — the field is
/// real, mounted, and holds a value, but it belongs to a different instance's scope entirely.
/// This is a RUNTIME check (unlike the view-hole case below): `Actions:` bodies are not walked by
/// the parser's name-resolution pass, so if a turn ever stopped reading its instance's OWN fields
/// only, this would silently start reading whatever this string happened to be.
#[test]
fn a_child_cannot_read_its_callers_variables() {
    let src = "\
## Component Child
State:
    Let seen be \"no\".
Actions:
    To peek:
        Set seen to secret.
View:
    <p id=\"c\">{seen}</p>
    <button id=\"go\" on-click={peek}>go</button>

## Component App
State:
    Let secret be \"leaked\".
View:
    <Child/>

## Main
Mount App.
Click \"#go\".
";
    let msg = failure(src);
    assert!(
        msg.contains("secret"),
        "a child's action reading an undeclared name fails naming it, rather than reading the \
         parent's own field of the same name: {msg}"
    );
}

/// A child's VIEW cannot read a name the PARENT declared, either — caught earlier, at parse time,
/// by the same name-resolution pass that refuses any other undeclared hole. Locked here too so a
/// regression in EITHER layer (parse-time resolution, or the runtime frame it would fall back to)
/// is caught by this suite.
#[test]
fn a_childs_view_cannot_read_its_callers_state() {
    let src = "\
## Component Child
View:
    <p id=\"c\">{secret}</p>

## Component App
State:
    Let secret be \"leaked\".
View:
    <Child/>

## Main
Mount App.
";
    let msg = failure(src);
    assert!(
        msg.contains("secret"),
        "a child's view reading an undeclared name fails naming it: {msg}"
    );
}

/// The positive control: a DECLARED prop reaches the child that declares it, through the very
/// same frame boundary — isolation must not cost legitimate prop-passing anything.
#[test]
fn a_declared_prop_still_reaches_the_child_that_declares_it() {
    let src = "\
## Component Child (label: Text)
View:
    <p id=\"c\">{label}</p>

## Component App
View:
    <Child label={\"hello\"}/>

## Main
Mount App.
";
    assert!(
        rendered(src).contains("<p id=\"c\">hello</p>"),
        "{}",
        rendered(src)
    );
}

/// A NESTED turn — a component's action calling `Call <Store>'s <action>.` while its own turn is
/// still open — cannot read the CALLING component's fields either. Before this scope boundary was
/// a real frame, both turns' fields lived in the same undo-logged map at once (the outer's push
/// had not yet been undone when the inner one opened), so a store action referencing a name it
/// does not declare could silently read whatever the calling component's OWN field of that name
/// held, instead of failing.
#[test]
fn a_nested_turn_cannot_read_the_calling_turns_fields() {
    let src = "\
## Store Cart
State:
    Let total be 0.
Actions:
    To addOne:
        Set total to total + secret.

## Component App
Use Cart.
State:
    Let secret be 99.
Actions:
    To go:
        Call Cart's addOne.
View:
    <p id=\"t\">{Cart's total}</p>
    <button id=\"go\" on-click={go}>go</button>

## Main
Mount App.
Click \"#go\".
";
    let msg = failure(src);
    assert!(
        msg.contains("secret"),
        "a nested turn (a store action called from a component action) must not read the \
         calling turn's own fields: {msg}"
    );
}
