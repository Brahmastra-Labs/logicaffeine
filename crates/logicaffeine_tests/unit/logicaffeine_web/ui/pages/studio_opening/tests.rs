//! The Studio's opening: decided from the route alone, before the file system answers, and the
//! one rule for what that answer may change.

use super::*;
use crate::ui::examples::{ExampleSpec, ALL_CODE_EXAMPLES, ALL_HARDWARE_SPECS, ALL_LOGIC_EXAMPLES, ALL_MATH_EXAMPLES};

/// The first Code example the file tree shows today. A bare `/studio` opens it (the user,
/// 2026-10-01: "lets start it off on code mode since its the first example").
const FIRST_CODE_EXAMPLE: &str = "/examples/code/basics/control-flow.logos";

fn every_example() -> impl Iterator<Item = &'static ExampleSpec> {
    ALL_LOGIC_EXAMPLES.iter().chain(ALL_CODE_EXAMPLES).chain(ALL_MATH_EXAMPLES).chain(ALL_HARDWARE_SPECS)
}

fn example(path: &str) -> &'static ExampleSpec {
    every_example().find(|spec| spec.vfs_path == path).unwrap_or_else(|| panic!("no shipped example at {path}"))
}

/// The mode the file browser opens a file in once its text is read: the path decides, and a
/// `.logos` file the path leaves to Code is a page when it declares one.
fn opened_by_the_file_browser(path: &str, text: &str) -> Option<StudioMode> {
    match StudioMode::for_path(path) {
        Some(StudioMode::Code) => Some(StudioMode::for_logos_source(text)),
        other => other,
    }
}

#[test]
fn a_bare_studio_opens_the_first_code_example_in_code_mode() {
    let opening = StudioOpening::for_route(None);
    assert_eq!(opening.path, FIRST_CODE_EXAMPLE);
    assert_eq!(opening.mode, StudioMode::Code);
    assert_eq!(opening.text, Some(example(FIRST_CODE_EXAMPLE).source));
}

#[test]
fn the_first_example_is_the_bare_studios_opening() {
    assert_eq!(StudioOpening::first_example(), StudioOpening::for_route(None));
}

#[test]
fn an_empty_file_parameter_is_no_file() {
    assert_eq!(StudioOpening::for_route(Some("")), StudioOpening::for_route(None));
}

#[test]
fn a_deep_link_to_any_example_opens_it_in_the_mode_the_file_browser_would() {
    let mut wrong = Vec::new();
    for spec in every_example() {
        let route = spec.vfs_path.trim_start_matches('/');
        let opening = StudioOpening::for_route(Some(route));
        let expected = StudioOpening {
            path: spec.vfs_path.to_string(),
            mode: opened_by_the_file_browser(spec.vfs_path, spec.source).expect("every example names its mode"),
            text: Some(spec.source),
        };
        if opening != expected {
            wrong.push(format!("{route}: {:?} {:?} (text {}), not {:?}", opening.path, opening.mode, opening.text.is_some(), expected.mode));
        }
    }
    assert!(wrong.is_empty(), "{} deep link(s) open wrong:\n{}", wrong.len(), wrong.join("\n"));
}

#[test]
fn a_deep_link_opens_each_kind_of_file_in_its_mode() {
    for (path, mode) in [
        ("examples/logic/quantifiers.logic", StudioMode::Logic),
        ("examples/logic/prover-demo.logic", StudioMode::Logic),
        ("examples/code/algorithms/fibonacci.logos", StudioMode::Code),
        ("examples/code/web/counter.logos", StudioMode::Web),
        ("examples/math/natural-numbers.logos", StudioMode::Math),
        ("examples/hardware/handshake.hw", StudioMode::Hardware),
        ("examples/hardware/arbiter.v", StudioMode::Hardware),
    ] {
        let opening = StudioOpening::for_route(Some(path));
        assert_eq!(opening.mode, mode, "{path} opens in {:?}", opening.mode);
        assert_eq!(opening.text, Some(example(&format!("/{path}")).source), "{path} opens without its text");
    }
}

#[test]
fn a_deep_link_reads_the_same_with_or_without_its_leading_slash() {
    for path in ["examples/logic/quantifiers.logic", "workspace/untitled.logos", "examples/code/fibonacci.logos"] {
        assert_eq!(StudioOpening::for_route(Some(path)), StudioOpening::for_route(Some(&format!("/{path}"))));
        assert_eq!(StudioOpening::for_route(Some(path)).path, format!("/{path}"));
    }
}

#[test]
fn a_readers_own_file_opens_in_its_paths_mode_and_waits_for_its_text() {
    for (path, mode) in [
        ("workspace/untitled.logos", StudioMode::Code),
        ("workspace/untitled-1.logic", StudioMode::Logic),
        ("workspace/spec.hw", StudioMode::Hardware),
        ("workspace/proof.math", StudioMode::Math),
        ("workspace/math/proof.logos", StudioMode::Math),
        ("workspace/web/page.logos", StudioMode::Web),
    ] {
        let opening = StudioOpening::for_route(Some(path));
        assert_eq!(opening.path, format!("/{path}"));
        assert_eq!(opening.mode, mode, "{path} opens in {:?}", opening.mode);
        assert_eq!(opening.text, None, "{path} is not shipped, so its text must come from the file system");
    }
}

#[test]
fn a_logos_file_never_opens_in_logic() {
    for path in [
        "workspace/untitled.logos",
        "workspace/deep/folder/notes.logos",
        "examples/code/fibonacci.logos",
        "examples/code/no-such-example.logos",
    ] {
        assert_ne!(StudioOpening::for_route(Some(path)).mode, StudioMode::Logic, "{path} opens in Logic");
    }
}

#[test]
fn a_file_with_no_logos_extension_opens_in_the_first_examples_mode() {
    let first = StudioOpening::first_example().mode;
    assert_eq!(first, StudioMode::Code);
    for path in ["workspace/notes.txt", "workspace/README", "examples/code"] {
        let opening = StudioOpening::for_route(Some(path));
        assert_eq!(opening.mode, first, "{path} opens in {:?}", opening.mode);
        assert_eq!(opening.text, None);
    }
}

#[test]
fn a_link_to_a_retired_example_waits_for_the_file_system() {
    let opening = StudioOpening::for_route(Some("examples/code/fibonacci.logos"));
    assert_eq!(opening.path, "/examples/code/fibonacci.logos");
    assert_eq!(opening.mode, StudioMode::Code);
    assert_eq!(opening.text, None);
}

#[test]
fn the_opening_text_starts_in_its_own_modes_editor_and_nowhere_else() {
    let shown = |opening: &StudioOpening| -> Vec<(StudioMode, String)> {
        StudioMode::ALL.iter().map(|&mode| (mode, opening.editor_text(mode))).collect()
    };
    let code = StudioOpening::for_route(Some("examples/code/algorithms/fibonacci.logos"));
    let text = code.text.expect("a shipped example opens with its text").to_string();
    assert_eq!(
        shown(&code),
        [
            (StudioMode::Logic, String::new()),
            (StudioMode::Code, text.clone()),
            (StudioMode::Math, String::new()),
            (StudioMode::Hardware, String::new()),
            (StudioMode::Web, text.clone()),
        ],
        "Code and Web share one editor; the others start empty"
    );

    let page = StudioOpening::for_route(Some("examples/code/web/counter.logos"));
    let text = page.text.expect("a shipped example opens with its text").to_string();
    assert_eq!(page.editor_text(StudioMode::Code), text);
    assert_eq!(page.editor_text(StudioMode::Web), text);
    assert_eq!(page.editor_text(StudioMode::Logic), "");

    for (path, mode) in [
        ("examples/logic/quantifiers.logic", StudioMode::Logic),
        ("examples/math/natural-numbers.logos", StudioMode::Math),
        ("examples/hardware/handshake.hw", StudioMode::Hardware),
    ] {
        let opening = StudioOpening::for_route(Some(path));
        for other in StudioMode::ALL {
            let expected = if other == mode { example(&format!("/{path}")).source.to_string() } else { String::new() };
            assert_eq!(opening.editor_text(other), expected, "{path}: the {other:?} editor");
        }
    }

    let own = StudioOpening::for_route(Some("workspace/untitled.logos"));
    for mode in StudioMode::ALL {
        assert_eq!(own.editor_text(mode), "", "a reader's file shows nothing until it loads");
    }
}

// ---------------------------------------------------------------------------------------------
// What the file system's answer changes
// ---------------------------------------------------------------------------------------------

/// The text a bare `/studio` shows from its first frame.
fn first_text() -> &'static str {
    example(FIRST_CODE_EXAMPLE).source
}

#[test]
fn the_file_systems_copy_of_the_example_on_screen_changes_nothing() {
    let opening = StudioOpening::for_route(None);
    assert_eq!(opening.arrival(opening.current_file().as_deref(), first_text(), Some(first_text().to_string())), Arrival::Keep);
}

#[test]
fn an_edit_made_before_the_file_system_answers_is_kept() {
    let opening = StudioOpening::for_route(None);
    let edited = format!("{}\nShow \"mine\".", first_text());
    assert_eq!(opening.arrival(opening.current_file().as_deref(), &edited, Some(first_text().to_string())), Arrival::Keep);
    assert_eq!(opening.arrival(opening.current_file().as_deref(), &edited, Some("a newer copy".to_string())), Arrival::Keep);
    assert_eq!(opening.arrival(opening.current_file().as_deref(), "", Some(first_text().to_string())), Arrival::Keep, "clearing the editor is an edit too");

    let own = StudioOpening::for_route(Some("workspace/untitled.logos"));
    assert_eq!(own.arrival(own.current_file().as_deref(), "Show 1.", Some("Show 2.".to_string())), Arrival::Keep);
    assert_eq!(own.arrival(own.current_file().as_deref(), "Show 1.", None), Arrival::Keep, "a missing file never replaces what the reader typed");
}

#[test]
fn a_readers_file_opens_when_its_text_arrives() {
    let own = StudioOpening::for_route(Some("workspace/untitled.logos"));
    assert_eq!(own.arrival(own.current_file().as_deref(), "", Some("Show 1.".to_string())), Arrival::Open("Show 1.".to_string()));
    assert_eq!(own.arrival(own.current_file().as_deref(), "", Some(String::new())), Arrival::Open(String::new()), "an empty file still opens");
}

#[test]
fn a_newer_copy_of_an_example_opens_while_the_editor_is_untouched() {
    let opening = StudioOpening::for_route(Some("examples/logic/quantifiers.logic"));
    let text = example("/examples/logic/quantifiers.logic").source;
    assert_eq!(opening.arrival(opening.current_file().as_deref(), text, Some("Every cat sleeps.".to_string())), Arrival::Open("Every cat sleeps.".to_string()));
}

#[test]
fn an_example_the_file_system_cannot_read_stays_open() {
    let opening = StudioOpening::for_route(Some("examples/hardware/handshake.hw"));
    let text = example("/examples/hardware/handshake.hw").source;
    assert_eq!(opening.arrival(opening.current_file().as_deref(), text, None), Arrival::Keep, "the site carries its text, so nothing is missing");
}

#[test]
fn a_missing_file_falls_back_to_the_first_example() {
    for path in [
        "workspace/gone.logos",
        "workspace/gone.logic",
        "examples/code/fibonacci.logos",
        "examples/logic/no-such-file.logic",
        "workspace/notes.txt",
    ] {
        let opening = StudioOpening::for_route(Some(path));
        assert_eq!(opening.arrival(opening.current_file().as_deref(), "", None), Arrival::Fallback(StudioOpening::first_example()), "{path}");
    }
}

#[test]
fn the_fallback_is_the_first_code_example_with_its_text() {
    let Arrival::Fallback(fallback) = StudioOpening::for_route(Some("workspace/gone.logic")).arrival(None, "", None) else {
        panic!("a missing file does not fall back");
    };
    assert_eq!(fallback.path, FIRST_CODE_EXAMPLE);
    assert_eq!(fallback.mode, StudioMode::Code);
    assert_eq!(fallback.text, Some(example(FIRST_CODE_EXAMPLE).source));
}

#[test]
fn the_open_file_is_the_example_from_the_first_frame_and_a_readers_file_once_it_loads() {
    assert_eq!(StudioOpening::for_route(None).current_file().as_deref(), Some(FIRST_CODE_EXAMPLE));
    assert_eq!(
        StudioOpening::for_route(Some("examples/logic/quantifiers.logic")).current_file().as_deref(),
        Some("/examples/logic/quantifiers.logic")
    );
    assert_eq!(StudioOpening::for_route(Some("workspace/untitled.logos")).current_file(), None);
    assert_eq!(StudioOpening::for_route(Some("examples/code/fibonacci.logos")).current_file(), None);
}

#[test]
fn a_file_the_reader_opened_meanwhile_is_kept() {
    let opening = StudioOpening::for_route(None);
    let other = Some("/examples/logic/quantifiers.logic");
    assert_eq!(opening.arrival(other, first_text(), Some("a newer copy".to_string())), Arrival::Keep);

    let own = StudioOpening::for_route(Some("workspace/untitled.logos"));
    assert_eq!(own.arrival(other, "", Some("Show 1.".to_string())), Arrival::Keep, "the reader's file would replace the one they chose");
    assert_eq!(own.arrival(other, "", None), Arrival::Keep, "a missing file would replace the one they chose");
}
