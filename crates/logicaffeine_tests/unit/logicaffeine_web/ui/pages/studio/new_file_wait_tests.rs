//! New file writes to the reader's store, so it waits, and says why, until that store has answered.

use super::why_new_file_waits;

#[test]
fn new_file_is_offered_once_the_readers_files_are_ready() {
    assert_eq!(why_new_file_waits(true, false), None);
    assert_eq!(why_new_file_waits(true, true), None, "an error reading one file does not take New file away");
}

#[test]
fn new_file_says_why_it_waits() {
    assert_eq!(why_new_file_waits(false, false).as_deref(), Some("Your files are still loading"));
    assert_eq!(why_new_file_waits(false, true).as_deref(), Some("Your files could not be opened"));
}

#[test]
fn the_reasons_read_as_plain_sentences() {
    for reason in [why_new_file_waits(false, false), why_new_file_waits(false, true)].into_iter().flatten() {
        assert!(!reason.contains(';') && !reason.contains('\u{2014}'), "{reason:?} has a semicolon or an em dash");
    }
}
