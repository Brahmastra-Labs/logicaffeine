use super::*;

#[test]
fn each_status_draws_its_own_glyph() {
    assert!(milestone_dot_icon(Status::Complete) == IconVariant::Check, "a finished card's dot is a check");
    assert!(milestone_dot_icon(Status::InProgress) == IconVariant::Hammer, "a card in progress is being built: its dot is a hammer");
    assert!(milestone_dot_icon(Status::Planned) == IconVariant::DraftingCompass, "a planned card is on the drawing board: its dot is a drafting compass");
}

#[test]
fn the_next_release_counts_the_rest_in_words() {
    assert_eq!(next_release_summary(3, 23, "0.10.1").as_deref(), Some("Plus 3 changes and 23 fixes since v0.10.1."));
    assert_eq!(next_release_summary(1, 1, "0.10.1").as_deref(), Some("Plus 1 change and 1 fix since v0.10.1."));
    assert_eq!(next_release_summary(0, 2, "0.10.1").as_deref(), Some("Plus 2 fixes since v0.10.1."));
    assert_eq!(next_release_summary(2, 0, "0.10.1").as_deref(), Some("Plus 2 changes since v0.10.1."));
    assert_eq!(next_release_summary(0, 0, "0.10.1"), None);
}
