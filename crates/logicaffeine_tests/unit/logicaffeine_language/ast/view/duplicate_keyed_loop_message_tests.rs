use super::duplicate_keyed_loop_message;

/// Pins the exact wording, including that positions 1 and 3 — not 0 and 2 — name items one
/// and three: every position an author sees is 1-based (Tristen's decision), and a caller
/// converting a 0-based internal index with a bare `+ 1` is exactly the kind of edit a future
/// refactor could silently drop. If this ever prints "item 0"/"item 2" again, a caller stopped
/// converting.
#[test]
fn the_message_names_the_loop_the_key_and_both_1_based_positions() {
    let got = duplicate_keyed_loop_message("todos", "7", 1, 3);
    assert_eq!(
        got,
        "the loop over todos keys item 1 and item 3 both \"7\" — a `keyed by` loop's keys \
             must be unique among its own siblings; give the duplicate its own key, or drop \
             `keyed by` if position alone already identifies each row here."
    );
}
