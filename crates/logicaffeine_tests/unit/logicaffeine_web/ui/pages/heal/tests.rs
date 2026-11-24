//! Heal's pure core: the restore, the flight's states, the dock bar's words, the fairy the site
//! serves, and the pixelation schedule.
use super::*;

// ---- flight ---------------------------------------------------------------------------------

#[test]
fn the_dock_narrates_each_state() {
    assert_eq!(narration(&Flight::Idle), None);
    assert_eq!(narration(&Flight::Failed("Why.".to_string())).as_deref(), Some("Why."));
    assert_eq!(narration(&Flight::Restored { previous: "mine".to_string() }).as_deref(), Some(RESTORED));
}

#[test]
fn the_fairy_is_served() {
    let served = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("public").join(FAIRY.trim_start_matches('/'));
    assert!(served.is_file(), "the fairy is not served: {}", served.display());
}

// ---- pixels ---------------------------------------------------------------------------------

#[test]
fn the_pixel_schedule_is_a_deterministic_permutation() {
    let steps = pixel_steps(40, 7);
    let mut sorted = steps.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, (0..40).collect::<Vec<_>>(), "every character dissolves once");
    assert_eq!(steps, pixel_steps(40, 7), "the same seed gives the same order");
    assert_ne!(steps, pixel_steps(40, 8), "another seed gives another order");
    assert_ne!(steps, (0..40).collect::<Vec<_>>(), "the order is not simply left to right");
    assert!(pixel_steps(0, 1).is_empty() && pixel_steps(1, 1) == vec![0]);
}

// ---- restore (Collin, 2026-11-18: Heal puts the example back as it was before the reader changed it) ----

#[test]
fn heal_puts_an_edited_file_back_as_it_shipped() {
    let shipped = "## Main\nShow \"hello\".\n";
    let edited = "## Main\nShwo \"hello\".\n";
    let flight = restore(edited, Some(shipped));
    assert_eq!(flight, Flight::Restored { previous: edited.to_string() });
    assert_eq!(narration(&flight).as_deref(), Some(RESTORED));
}

#[test]
fn an_unchanged_file_is_already_as_it_shipped() {
    let shipped = "## Main\nShow 1.\n";
    assert_eq!(restore(shipped, Some(shipped)), Flight::Failed(ALREADY_ORIGINAL.to_string()));
    // Trailing spaces and the final newline are not a change.
    assert_eq!(restore("## Main\nShow 1.   ", Some(shipped)), Flight::Failed(ALREADY_ORIGINAL.to_string()));
}

#[test]
fn a_file_with_no_original_says_so() {
    assert_eq!(restore("## Main\nShow 1.\n", None), Flight::Failed(NO_ORIGINAL.to_string()));
}

#[test]
fn undo_gives_back_the_readers_version_only_after_a_restore() {
    let (idle, back) = Flight::Restored { previous: "mine".to_string() }.take_back();
    assert_eq!(idle, Flight::Idle);
    assert_eq!(back.as_deref(), Some("mine"));
    assert_eq!(Flight::Failed("x".to_string()).take_back(), (Flight::Idle, None));
    assert_eq!(Flight::Idle.take_back(), (Flight::Idle, None));
    // Closing the card (Done or Escape) keeps the restored text.
    assert_eq!(Flight::Restored { previous: "mine".to_string() }.undo(), Flight::Idle);
}

#[test]
fn the_restore_words_are_plain_sentences() {
    for text in [RESTORED, ALREADY_ORIGINAL, NO_ORIGINAL] {
        assert!(text.ends_with('.'), "{text:?}");
        assert!(!text.contains(';') && !text.contains('\u{2014}'), "{text:?}");
    }
}

// ---- a restore's pixels -----------------------------------------------------------------------

#[test]
fn every_character_of_a_restore_takes_one_turn() {
    for len in [0usize, 1, 2, 40, 2000] {
        for forming in [false, true] {
            let mut turns = pixel_turns(len, 11, forming);
            turns.sort_unstable();
            assert_eq!(turns, (0..len).collect::<Vec<_>>(), "{len} characters, forming {forming}");
        }
    }
}

#[test]
fn the_file_forms_in_the_mirror_of_the_order_the_reader_s_text_goes() {
    let going = pixel_turns(40, 11, false);
    let coming = pixel_turns(40, 11, true);
    for i in 0..40 {
        assert_eq!(going[i] + coming[i], 39, "character {i}: the last to go is the first to come");
    }
    assert_eq!(going, pixel_turns(40, 11, false), "the schedule is the same every time");
}

#[test]
fn the_turns_spread_over_the_restore_s_span_and_never_past_it() {
    assert_eq!(pixel_delay_ms(0, 0), 0);
    assert_eq!(pixel_delay_ms(0, 40), 0);
    for len in [1usize, 7, 40, 2000] {
        let last = pixel_delay_ms(len - 1, len);
        assert!(f64::from(last) < PIXEL_SPAN_MS, "{len} characters: the last turn starts at {last} ms");
        for turn in 1..len {
            assert!(pixel_delay_ms(turn, len) >= pixel_delay_ms(turn - 1, len), "turns go in order");
        }
    }
    assert_eq!(pixel_delay_ms(20, 40), 350, "half way through the turns is half way through the span");
}
