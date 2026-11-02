//! ★ LOCK — a program's own `Type` and `Choose` are the browser's default action, on the bytecode
//! tier as on the tree-walker. ★
//!
//! Typing enters text into a control that takes typed text, and choosing selects an option the
//! `<select>` offers, whether or not the page listens — so neither needs a listener, the value lands
//! in the control, and a control that cannot take the verb (a paragraph, a disabled field, an option
//! the select does not offer) is refused with `drive::cannot_take`'s words. The bytecode tier reaches
//! all of it through `DomEnterValue`; every row runs the same program on both tiers and holds the
//! bytecode tier's output and failure to the tree-walker's.

const PAGE: &str = "## Component App\n\
    State:\n    Let draft be \"\".\n\
    View:\n    <p id=\"m\">hi</p>\n    <input id=\"d\" aria-label=\"d\" bind={draft}>\n    <p id=\"echo\">{draft}</p>\n    \
    <input id=\"off\" aria-label=\"off\" disabled>\n    <input id=\"ro\" aria-label=\"ro\" readonly>\n    \
    <select id=\"s\" aria-label=\"s\"><option>a</option><option value=\"bee\">b</option></select>\n\n";

fn both(main: &str) -> ((String, Option<String>), (String, Option<String>)) {
    let src = format!("{PAGE}## Main\nMount App.\n{main}\n");
    let tw = logicaffeine_compile::compile::tw_outcome(&src);
    let vm = logicaffeine_compile::compile::vm_outcome(&src);
    (
        (tw.output.trim().to_string(), tw.error),
        (vm.output.trim().to_string(), vm.error),
    )
}

fn agree(main: &str) -> (String, Option<String>) {
    let (tw, vm) = both(main);
    assert_eq!(
        vm, tw,
        "the bytecode tier and the tree-walker disagree on:\n{main}"
    );
    tw
}

#[test]
fn typing_into_a_field_goes_on_whether_or_not_the_page_listens() {
    for main in [
        "Type \"milk\" into \"#d\".\nShow \"after\".",
        "Type \"milk\" into \"#d\".\nType \"oat\" into \"#d\".\nShow \"after\".",
    ] {
        let (out, err) = agree(main);
        assert_eq!((out.as_str(), err), ("after", None), "{main}");
    }
}

#[test]
fn choosing_an_offered_option_needs_no_listener() {
    for chosen in ["a", "bee", "b"] {
        let (out, err) = agree(&format!("Choose \"{chosen}\" in \"#s\".\nShow \"after\"."));
        assert_eq!((out.as_str(), err), ("after", None), "choosing {chosen:?}");
    }
}

#[test]
fn a_control_that_cannot_take_the_verb_is_refused_in_the_references_words() {
    use logicaffeine_compile::semantics::drive::{cannot_take, Verb};
    // Each ends in a statement after the verb, so the bytecode tier compiles the verb into `main`
    // rather than handing a trailing driving script to the page host.
    for (main, verb, selector, value) in [
        (
            "Type \"x\" into \"#m\".\nShow \"after\".",
            Verb::Type,
            "#m",
            "x",
        ),
        (
            "Type \"x\" into \"#off\".\nShow \"after\".",
            Verb::Type,
            "#off",
            "x",
        ),
        (
            "Type \"x\" into \"#ro\".\nShow \"after\".",
            Verb::Type,
            "#ro",
            "x",
        ),
        (
            "Choose \"zed\" in \"#s\".\nShow \"after\".",
            Verb::Choose,
            "#s",
            "zed",
        ),
        (
            "Choose \"a\" in \"#d\".\nShow \"after\".",
            Verb::Choose,
            "#d",
            "a",
        ),
    ] {
        let (_, err) = agree(main);
        assert_eq!(err, Some(cannot_take(verb, selector, value)), "{main}");
    }
}

/// A `## Main` that ENDS by driving the page hands that script to the page host rather than compiling
/// it into `main` (`compile::vm_outcome`'s gate); the refusal is the same there.
#[test]
fn a_trailing_driving_script_refuses_the_same_way() {
    use logicaffeine_compile::semantics::drive::{cannot_take, Verb};
    let (_, err) = agree("Type \"x\" into \"#m\".");
    assert_eq!(err, Some(cannot_take(Verb::Type, "#m", "x")));
}

#[test]
fn a_selector_and_a_value_known_only_at_run_time_are_named_in_the_refusal() {
    use logicaffeine_compile::semantics::drive::{cannot_take, Verb};
    let (_, err) = agree("Let s be \"#m\".\nLet v be \"typed\".\nType v into s.");
    assert_eq!(err, Some(cannot_take(Verb::Type, "#m", "typed")));
}
