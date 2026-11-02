use super::*;

#[test]
fn abbreviation_detector_precision() {
    for w in ["EEG", "ERPs", "BSc", "MSc", "PhD", "N2pc"] {
        assert!(
            looks_like_abbreviation(w),
            "{w} should look like an abbreviation"
        );
    }
    for w in ["Mary", "Paris", "cat", "lecture", "the"] {
        assert!(
            !looks_like_abbreviation(w),
            "{w} should NOT be an abbreviation"
        );
    }
}

#[test]
fn pos_guess_by_suffix() {
    assert_eq!(guess_pos("cognitive"), "Adjective");
    assert_eq!(guess_pos("neural"), "Adjective");
    assert_eq!(guess_pos("publications"), "Noun");
    assert_eq!(guess_pos("stimulation"), "Noun");
    assert_eq!(guess_pos("characterize"), "Verb");
}

#[test]
fn function_words_are_never_gaps() {
    for w in ["is", "are", "from", "where", "of", "the", "than"] {
        assert!(
            !is_actionable_gap(w, &TokenCategory::Other),
            "{w} is a function word, never a lexicon gap"
        );
    }
}
