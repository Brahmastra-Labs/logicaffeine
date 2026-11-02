//! LOCK — a `Mount` sentence that names a component is a UI mount, whichever way it is spelt, and
//! a name no component answers to is refused with the names the file does declare.
//!
//! `Mount <var> at "path"` is also the CRDT-journal sentence. The two are told apart by SHAPE: a
//! capitalised name (components are capitalised) or a selector-shaped target (`#id`, `.class`, an
//! HTML tag) is a UI mount; a lowercase variable mounted at a file path is a journal.

#[path = "language_common/mod.rs"]
mod common;

use common::{accepted, refusal};
use logicaffeine_language::ast::stmt::{Expr, Literal, UiStmt};
use logicaffeine_language::ast::Stmt;

const COUNTER: &str = "## Component Counter\nView:\n    <p>hi</p>\n\n";

fn main_with(mount: &str) -> String {
    format!("{COUNTER}## Main\n{mount}\n")
}

/// The UI mount a program parses to: `(component, selector)`.
fn ui_mount(src: &str) -> (String, Option<String>) {
    let parsed = accepted(src);
    for s in &parsed.stmts {
        if let Stmt::Ui(UiStmt::Mount {
            component,
            selector,
        }) = s
        {
            let sel = selector.map(|e| match e {
                Expr::Literal(Literal::Text(t)) => parsed.interner.resolve(*t).to_string(),
                other => format!("{other:?}"),
            });
            return (parsed.interner.resolve(*component).to_string(), sel);
        }
    }
    panic!("no UI mount in:\n{src}");
}

#[test]
fn a_misspelt_component_at_a_selector_is_refused_with_did_you_mean() {
    let src = main_with("Mount Countr at \"#root\".");
    let (message, covered) = refusal(&src);
    assert_eq!(covered, "Countr", "the caret covers the name: {message}");
    assert!(message.contains("no component named `Countr`"), "{message}");
    assert!(message.contains("did you mean `Counter`?"), "{message}");
    assert!(
        message.contains("Counter"),
        "it lists what the file declares: {message}"
    );
}

#[test]
fn a_misspelt_component_with_no_target_is_refused_the_same_way() {
    let (message, covered) = refusal(&main_with("Mount Countr."));
    assert_eq!(covered, "Countr");
    assert!(
        message.contains("no component named `Countr` — did you mean `Counter`?"),
        "{message}"
    );
}

#[test]
fn a_lowercase_name_at_a_selector_is_a_ui_mount() {
    for target in ["#root", ".app", "main", "body"] {
        let (message, covered) = refusal(&main_with(&format!("Mount counter at \"{target}\".")));
        assert_eq!(covered, "counter", "{target}: {message}");
        assert!(
            message.contains("did you mean `Counter`?"),
            "{target}: {message}"
        );
    }
}

#[test]
fn a_capitalised_name_at_a_file_path_is_a_ui_mount_and_says_what_a_journal_looks_like() {
    let (message, _) = refusal(&main_with("Mount Ledger at \"data/ledger.journal\"."));
    assert!(message.contains("no component named `Ledger`"), "{message}");
    assert!(
        message.contains("Mount ledger at \"data/ledger.journal\""),
        "a capitalised name is never a journal variable, so the message shows the journal form: {message}"
    );
}

#[test]
fn a_lowercase_variable_at_a_file_path_is_still_a_journal() {
    let parsed = accepted("## Main\nMount counter at \"data/counter.journal\".\n");
    assert!(
        parsed.stmts.iter().any(|s| matches!(s, Stmt::Mount { .. })),
        "the CRDT journal sentence keeps its meaning"
    );
}

#[test]
fn a_component_declared_below_its_mount_is_mounted() {
    let src = "## Main\nMount App at \"#root\".\n\n## Component App\nView:\n    <p>hi</p>\n";
    assert_eq!(
        ui_mount(src),
        ("App".to_string(), Some("#root".to_string()))
    );
    let src = "## Main\nMount App.\n\n## Component App\nView:\n    <p>hi</p>\n";
    assert_eq!(ui_mount(src), ("App".to_string(), None));
}

#[test]
fn a_declared_component_mounts_as_before() {
    assert_eq!(
        ui_mount(&main_with("Mount Counter at \"#root\".")),
        ("Counter".to_string(), Some("#root".to_string()))
    );
    assert_eq!(
        ui_mount(&main_with("Mount Counter.")),
        ("Counter".to_string(), None)
    );
}

#[test]
fn a_file_with_no_components_says_so() {
    let (message, _) = refusal("## Main\nMount Countr at \"#root\".\n");
    assert!(message.contains("no component named `Countr`"), "{message}");
    assert!(message.contains("declares no components"), "{message}");
}

#[test]
fn every_declared_component_is_listed_when_nothing_is_near() {
    let src = "## Component Alpha\nView:\n    <p>a</p>\n\n## Component Beta\nView:\n    <p>b</p>\n\n## Main\nMount Zzzzzz.\n";
    let (message, _) = refusal(src);
    assert!(
        !message.contains("did you mean"),
        "nothing is near `Zzzzzz`: {message}"
    );
    assert!(
        message.contains("Alpha") && message.contains("Beta"),
        "{message}"
    );
}
