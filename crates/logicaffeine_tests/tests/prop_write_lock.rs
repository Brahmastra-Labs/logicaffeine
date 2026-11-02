//! LOCK — no sentence in a component may change one of its props, and the refusal points at the
//! prop where the sentence names it.
//!
//! What a body writes is decided by the one write walk (`lgx::deps`), the same walk that decides
//! which sites a turn repaints; where it was written comes from the write recorded as the sentence
//! was parsed. Every sentence that writes a place is a row here, and each must be refused with
//! the caret on the prop's own name and the sentence quoted back.

#[path = "language_common/mod.rs"]
mod common;

use common::{accepted, refusal};

/// `Row` with the props `label: Text`, `tags: Seq of Text`, `todo: Todo`, `count: Int`, `done: Bool`,
/// whose one action runs `body`.
fn row(body: &str) -> String {
    format!(
        "## A Todo has:\n    a text, which is Text.\n    a points, which is Int.\n\n\
         ## Component Row (label: Text) and (tags: Seq of Text) and (todo: Todo) and (count: Int) and (done: Bool)\n\
         Actions:\n    To go:\n        {body}\n\
         View:\n    <li on-click={{go}}>{{label}}</li>\n\n\
         ## Component App\nState:\n    Let t be a new Todo with text \"a\" and points 1.\n    Let tags be [\"x\"].\n\
         View:\n    <ul><Row label=\"a\" tags={{tags}} todo={{t}} count={{1}} done={{true}}/></ul>\n\n## Main\nMount App.\n"
    )
}

/// Every place-writing sentence, the prop it writes, and how the refusal quotes it.
const WRITES: &[(&str, &str, &str)] = &[
    ("Set label to \"b\".", "label", "`Set label to …`"),
    ("Set todo's text to \"b\".", "todo", "`Set todo's … to …`"),
    (
        "Set item 1 of tags to \"b\".",
        "tags",
        "`Set item … of tags to …`",
    ),
    ("count += 1.", "count", "`count … = …`"),
    ("Toggle done.", "done", "`Toggle done.`"),
    ("Push \"b\" to tags.", "tags", "`Push … to tags`"),
    ("Add \"b\" to tags.", "tags", "`Add … to tags`"),
    ("Remove \"x\" from tags.", "tags", "`Remove … from tags`"),
    ("Pop from tags.", "tags", "`Pop from tags`"),
    ("Increase count by 1.", "count", "`Increase count by …`"),
    ("Decrease count by 1.", "count", "`Decrease count by …`"),
    ("Load \"k\" into label.", "label", "`Load … into label`"),
];

#[test]
fn every_sentence_that_writes_a_prop_is_refused_at_the_prop() {
    for (sentence, prop, quoted) in WRITES {
        let src = row(sentence);
        let (message, covered) = refusal(&src);
        assert_eq!(
            &covered, prop,
            "`{sentence}`: the caret covers the prop's own name: {message}"
        );
        assert!(
            message.contains(quoted),
            "`{sentence}`: the sentence is quoted back as {quoted}: {message}"
        );
        assert!(
            message.contains(&format!(
                "`{prop}` is a prop of `Row`, so this component cannot change it"
            )),
            "`{sentence}`: {message}"
        );
    }
}

#[test]
fn a_write_inside_a_branch_or_a_loop_is_still_a_write() {
    let (message, covered) = refusal(&row("If true:\n            Set label to \"b\"."));
    assert_eq!(covered, "label", "{message}");
    let (message, covered) = refusal(&row("Repeat for x in tags:\n            Push x to tags."));
    assert_eq!(covered, "tags", "{message}");
}

#[test]
fn a_local_or_a_parameter_of_the_same_name_is_not_the_prop() {
    accepted(&row(
        "Let mutable label be \"local\".\n        Set label to \"b\".",
    ));
    accepted(
        "## Component Row (label: Text)\nActions:\n    To rename (label: Text):\n        Set label to label.\n\
         View:\n    <li on-click={rename(\"z\")}>{label}</li>\n\n\
         ## Component App\nView:\n    <ul><Row label=\"a\"/></ul>\n\n## Main\nMount App.\n",
    );
}

#[test]
fn a_write_in_an_event_sentence_is_refused_too() {
    let src = "## Component Badge (count: Int)\nActions:\n    When the page loads:\n        Increase count by 1.\nView:\n    <b>{count}</b>\n\n\
               ## Component App\nView:\n    <Badge count={1}/>\n\n## Main\nMount App.\n";
    let (message, covered) = refusal(src);
    assert_eq!(covered, "count", "{message}");
    assert!(message.contains("`Increase count by …`"), "{message}");
}

#[test]
fn a_state_field_is_writable() {
    accepted(
        "## Component App\nState:\n    Let n be 0.\n    Let on be false.\n    Let tags be [\"a\"].\n\
         Actions:\n    To go:\n        Increase n by 1.\n        Push \"b\" to tags.\n        Toggle on.\n\
         View:\n    <p on-click={go}>{n}</p>\n\n## Main\nMount App.\n",
    );
}
