use super::*;

fn uses_the_relay(source: &str) -> bool {
    crate::ui_bridge::with_parsed_program(source, |parsed, _| {
        let (stmts, _, _) = parsed.unwrap_or_else(|e| panic!("the program parses: {e}\n{source}"));
        pages_use_the_relay(stmts)
    })
}

#[test]
fn a_publish_a_message_handler_or_a_shared_store_uses_the_relay() {
    for (what, source) in [
        (
            "a publish in an action",
            "## Page /\nState:\n    Let n be 0.\nActions:\n    To ping:\n        Publish \"hi\" on \"room\".\nView:\n    <button on-click={ping}>ping</button>\n\n## Main\nShow 1.\n",
        ),
        (
            "a publish nested in a condition",
            "## Page /\nState:\n    Let n be 0.\nActions:\n    To ping:\n        If n is greater than 0:\n            Publish \"hi\" on \"room\".\nView:\n    <button on-click={ping}>ping</button>\n\n## Main\nShow 1.\n",
        ),
        (
            "a message handler",
            "## Page /\nState:\n    Let last be \"\".\nActions:\n    When a message arrives on \"room\" (m: Text):\n        Set last to m.\nView:\n    <p>{last}</p>\n\n## Main\nShow 1.\n",
        ),
        (
            "a shared store",
            "## Store Board shared on \"board\"\nState:\n    Let n be 0.\n\n## Main\nShow 1.\n",
        ),
    ] {
        assert!(uses_the_relay(source), "{what} talks to the relay:\n{source}");
    }
}

#[test]
fn a_page_that_never_publishes_or_listens_does_not_use_the_relay() {
    for (what, source) in [
        ("no pages at all", "## Main\nShow 1.\n"),
        (
            "a page with local state",
            "## Page /\nState:\n    Let n be 0.\nActions:\n    To bump:\n        Set n to n + 1.\nView:\n    <button on-click={bump}>{n}</button>\n\n## Main\nShow 1.\n",
        ),
        (
            "a persisted store",
            "## Store Prefs persisted as \"prefs\"\nState:\n    Let theme be \"dark\".\n\n## Main\nShow 1.\n",
        ),
    ] {
        assert!(!uses_the_relay(source), "{what} never reaches the relay:\n{source}");
    }
}
