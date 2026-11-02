//! The mode a file opens in once its text is known: the one rule the file browser and the
//! Studio's opening share.

use super::StudioMode;

const PAGE: &str = "## Page /\n\nState:\n    Let count be 0.\n\nView:\n    <h1>Count: {count}</h1>\n";
const PROGRAM: &str = "## Main\nShow 1.\n";

#[test]
fn a_logos_program_opens_in_code() {
    assert_eq!(StudioMode::for_file("/workspace/a.logos", PROGRAM), Some(StudioMode::Code));
}

#[test]
fn a_logos_file_that_declares_a_page_opens_in_web() {
    assert_eq!(StudioMode::for_logos_source(PAGE), StudioMode::Web, "the fixture is not a page, so this proves nothing");
    assert_eq!(StudioMode::for_file("/workspace/a.logos", PAGE), Some(StudioMode::Web));
    assert_eq!(StudioMode::for_file("/examples/code/basics/a.logos", PAGE), Some(StudioMode::Web));
}

#[test]
fn the_path_decides_before_the_text() {
    assert_eq!(StudioMode::for_file("/workspace/math/a.logos", PAGE), Some(StudioMode::Math));
    assert_eq!(StudioMode::for_file("/workspace/web/a.logos", PROGRAM), Some(StudioMode::Web));
    assert_eq!(StudioMode::for_file("/workspace/a.logic", PAGE), Some(StudioMode::Logic));
    assert_eq!(StudioMode::for_file("/workspace/a.hw", PAGE), Some(StudioMode::Hardware));
    assert_eq!(StudioMode::for_file("/workspace/a.v", PROGRAM), Some(StudioMode::Hardware));
}

#[test]
fn a_file_with_no_logos_extension_names_no_mode() {
    assert_eq!(StudioMode::for_file("/workspace/notes.txt", PAGE), None);
    assert_eq!(StudioMode::for_file("/workspace/README", PROGRAM), None);
}
