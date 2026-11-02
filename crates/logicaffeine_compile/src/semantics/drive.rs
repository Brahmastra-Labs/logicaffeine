//! Driving a page from outside it: what a host's driving verb answers, what that answer says, and
//! the clock a page arms.
//!
//! A `## Test` works a page the way a visitor would — `Click`, `Type … into`, `Press`, `Submit`,
//! `Choose`, `Hover` — and every tier reports the outcome in the same three words: the element the
//! selector names is not there, it is there and nothing on the page answers what was done to it,
//! or something answered. A lowered page answers through its node dispatcher, which says whether
//! anything heard each event the verb raised; the reference tier knows it directly. Either way the
//! sentence a test author reads comes from HERE, so the two tiers cannot word one failure two ways.
//!
//! The page clock is the other half of what a host does to a page it drives. A browser page arms
//! `setInterval(ms, handler)` and the browser calls the handler every `ms` milliseconds; a host
//! with a virtual clock does the same arithmetic when a test says `Advance time by …`. The armed
//! intervals are recorded here, beside the answers, because they are the host's to fire.

use crate::interpreter::RuntimeValue;
use std::cell::RefCell;

/// What a driving verb found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// No element matches the selector.
    NoElement,
    /// The element is there and nothing on the page answered.
    Unanswered,
    /// Something answered.
    Answered,
}

/// A driving verb a test aims at an element, and the events it raises there — the events a
/// browser raises for the same act, in the order it raises them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    /// `Click SEL.`
    Click,
    /// `Submit SEL.`
    Submit,
    /// `Press KEY [in SEL].` — a key goes down and comes up; either answering is an answer.
    Press,
    /// `Type TEXT into SEL.` — an `input` and then a `change`.
    Type,
    /// `Choose VALUE in SEL.` — a selection raises what typing raises.
    Choose,
    /// `Hover SEL.`
    Hover,
}

impl Verb {
    /// Every verb, for a census.
    pub const ALL: [Verb; 6] = [
        Verb::Click,
        Verb::Submit,
        Verb::Press,
        Verb::Type,
        Verb::Choose,
        Verb::Hover,
    ];

    /// The events the verb raises on its element.
    pub fn events(self) -> &'static [&'static str] {
        match self {
            Verb::Click => &["click"],
            Verb::Submit => &["submit"],
            Verb::Press => &["keydown", "keyup"],
            Verb::Type | Verb::Choose => &["input", "change"],
            Verb::Hover => &["mouseover"],
        }
    }

    /// Where the browser's own default action of the verb leaves keyboard focus — moved, with its
    /// `blur` and `focus` raised, before any of the verb's [`Verb::events`]: a click's press focuses
    /// what was pressed, a control is focused to be typed into or chosen in, and a key pressed in an
    /// element goes to it. Submitting and hovering move nothing.
    pub fn focus_target(self) -> FocusTarget {
        match self {
            Verb::Click => FocusTarget::PressedAncestor,
            Verb::Type | Verb::Choose | Verb::Press => FocusTarget::Element,
            Verb::Submit | Verb::Hover => FocusTarget::Unmoved,
        }
    }
}

/// Where a verb's default action sends keyboard focus ([`Verb::focus_target`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusTarget {
    /// A pointer press: focus goes to the pressed element's nearest focusable ancestor-or-self, and
    /// off every element when there is none — pressing on plain text blurs the field that had it.
    PressedAncestor,
    /// The element itself takes focus when it can (`element.focus()`); focus stays put when it
    /// cannot.
    Element,
    /// Focus does not move.
    Unmoved,
}

/// The failure an answer means for `events` aimed at `selector`, or `None` when it is no failure.
///
/// `key` names the key of a press, which is how its refusal names what was pressed.
pub fn answer_failure(
    answer: Answer,
    events: &[&str],
    key: Option<&str>,
    selector: &str,
    must_answer: bool,
) -> Option<String> {
    match answer {
        Answer::Answered => None,
        Answer::NoElement => Some(no_element(selector)),
        Answer::Unanswered if must_answer => Some(nothing_answers(events, key, selector)),
        Answer::Unanswered => None,
    }
}

/// `No element matches "#x". …` — the selector names nothing on the page.
pub fn no_element(selector: &str) -> String {
    format!(
        "No element matches {selector:?}. Mount the component first, and check the selector \
         against the ids and classes its view renders."
    )
}

/// `Nothing answers a click on "#x". …` — the element is there and nothing listens.
pub fn nothing_answers(events: &[&str], key: Option<&str>, selector: &str) -> String {
    let what = match key {
        Some(k) => format!("a {k} press"),
        None => format!("a {}", events.join(" or ")),
    };
    let first = events.first().copied().unwrap_or("click");
    format!(
        "Nothing answers {what} on {selector:?}. Give the element an `on-{first}={{action}}` \
         directive, or write a `When {selector:?} is …:` sentence in the component's `Actions:`."
    )
}

/// `Typing into "#x" enters nothing. …` / `Choosing "v" in "#x" selects nothing. …` — the element
/// is there and cannot take the verb, so the browser's own default action does nothing and raises
/// no `input` or `change`.
pub fn cannot_take(verb: Verb, selector: &str, value: &str) -> String {
    match verb {
        Verb::Choose => format!(
            "Choosing {value:?} in {selector:?} selects nothing: it is not a `<select>` a visitor can \
             reach that offers that option, so the browser raises no `input` or `change`. Check the \
             option's value or text, and that the control is not disabled."
        ),
        _ => format!(
            "Typing into {selector:?} enters nothing: it takes no typed text — it is read-only, \
             disabled, or not a text field — so the browser raises no `input` or `change`. Type into \
             a `<textarea>` or a text `<input>` a visitor can edit."
        ),
    }
}

/// `` `Focus "#x"` found no such element. … `` — a command that acts on one element (focus, blur)
/// aimed at an element that is not there.
pub fn element_required(verb: &str, selector: &str) -> String {
    format!(
        "`{verb} {selector:?}` found no such element. Mount the component first, and check the \
         selector against the ids and classes its view renders."
    )
}

/// `Serve fetches with routes.` in a program that declares no route.
pub fn no_routes_to_serve() -> String {
    "`Serve fetches with routes.` needs routes to serve, and this program declares none. Add a \
     `## Route METHOD /path -> T:` block, or answer the page's requests with `Given that METHOD \
     \"/url\" answers …`."
        .to_string()
}

/// `Mount App at "#root": no element matches that selector. …`
pub fn mount_target_missing(component: &str, selector: &str) -> String {
    format!(
        "Mount {component} at {selector:?}: no element matches that selector. Create the mount \
         point before mounting, or mount without `at` to render under the page body."
    )
}

/// A form's fields as the body a browser submits: `a=1&b=2`, each name and value percent-encoded
/// with the unreserved characters kept and a space as `+` (`application/x-www-form-urlencoded`).
pub fn form_urlencoded(fields: &[(String, String)]) -> String {
    fn enc(s: &str) -> String {
        let mut out = String::new();
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(b as char)
                }
                b' ' => out.push('+'),
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }
    fields
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// One interval a page armed: its period and the closure the host fires each period.
#[derive(Clone, Debug)]
pub struct Interval {
    pub period_ms: i64,
    pub handler: RuntimeValue,
}

thread_local! {
    static INTERVALS: RefCell<Vec<Interval>> = const { RefCell::new(Vec::new()) };
}

/// `setInterval(ms, handler)` — record an interval the host fires every `ms` milliseconds.
pub fn arm_interval(period_ms: i64, handler: RuntimeValue) {
    INTERVALS.with(|i| i.borrow_mut().push(Interval { period_ms, handler }));
}

/// Every interval the page has armed, in the order it armed them.
pub fn intervals() -> Vec<Interval> {
    INTERVALS.with(|i| i.borrow().clone())
}

/// A fresh page: no intervals armed.
pub fn reset_clock() {
    INTERVALS.with(|i| i.borrow_mut().clear());
}

/// The intervals a page has armed — what [`take`] saves and [`restore`] puts back.
#[derive(Default)]
pub struct ClockState {
    intervals: Vec<Interval>,
}

/// Take the armed intervals, leaving none in their place — like [`reset_clock`], but they come
/// back to the caller instead of being dropped. A nested run (a candidate page tier compared
/// against the reference) takes the caller's own armed intervals before it arms its own, and
/// [`restore`]s them when it is done.
pub fn take() -> ClockState {
    ClockState {
        intervals: INTERVALS.with(RefCell::take),
    }
}

/// Put back intervals a matching [`take`] returned.
pub fn restore(state: ClockState) {
    INTERVALS.with(|i| *i.borrow_mut() = state.intervals);
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/semantics/drive/tests.rs"]
mod tests;
