use super::*;

const LOG: &str = "# Changelog\n\
        \n\
        ## [Unreleased]\n\
        \n\
        ### Added\n\
        - **Shipped assets travel compressed.** There was no `Content-Encoding` anywhere.\n\
        - **`Real` reactive signals are a first-class value family.** Joining `Text`.\n\
        \x20 - a nested note, not an entry\n\
        ### Fixed\n\
        - **A missing static file is a 404.** Pinned.\n\
        ### Changed\n\
        - **`Element` is a nominal type.** No longer `Int`.\n\
        ### Removed\n\
        - **The theme picker.** Gone.\n\
        ### Fixed\n\
        - **Eight test binaries had never run.** Now they do.\n\
        - **WASM functions with more than 64 blocks no longer trap.** Signed LEB128.\n\
        \n\
        ## [0.9.16] - 2026-04-06\n\
        \n\
        ### Added\n\
        - **IEEE 1800-2023 SVA upgrade** — sequence methods.\n\
        \n\
        ## [0.9.1] - 2026-03-01\n\
        \n\
        ### Fixed\n\
        - **CI runs again.**\n\
        \n\
        ## [0.5.5] - 2026-01-01\n\
        \n\
        First public release.\n\
        \n\
        ### Core Features (v0.5.5)\n\
        - Everything.\n";

#[test]
fn a_section_runs_from_its_heading_to_the_next() {
    let s = section(LOG, "0.9.16").expect("0.9.16 is in the log");
    assert!(s.contains("IEEE 1800-2023 SVA upgrade"));
    assert!(!s.contains("CI runs again"), "the section ran into the next release");
}

#[test]
fn a_version_is_matched_whole() {
    let s = section(LOG, "0.9.1").expect("0.9.1 is in the log");
    assert!(s.contains("CI runs again"), "0.9.1 matched some other release");
    assert!(section(LOG, "0.9").is_none(), "a version prefix matched a release");
    assert!(section(LOG, "1.0.0").is_none());
}

#[test]
fn a_bold_lead_is_the_headline() {
    assert_eq!(headline("- **Shipped assets travel compressed.** There was no `Content-Encoding`."), "Shipped assets travel compressed");
    assert_eq!(headline("- **IEEE 1800-2023 SVA upgrade** — sequence methods."), "IEEE 1800-2023 SVA upgrade");
}

#[test]
fn backticks_leave_the_headline() {
    assert_eq!(headline("- **`Real` reactive signals are a first-class value family.** Joining."), "Real reactive signals are a first-class value family");
}

#[test]
fn a_line_without_bold_text_keeps_its_words() {
    assert_eq!(headline("First public release."), "First public release");
    assert_eq!(headline("(skipped — the tag was cut twice)"), "skipped — the tag was cut twice");
}

#[test]
fn whitespace_runs_become_one_space() {
    assert_eq!(headline("- **Two   words\tapart.** Body."), "Two words apart");
}

#[test]
fn a_long_headline_is_cut_at_its_last_whole_word() {
    let long = "- **The quick brown fox jumps over the lazy dog while the cat watches it from the fence.** More.";
    let got = headline(long);
    assert_eq!(got, "The quick brown fox jumps over the lazy dog while the cat watches it…");
    assert!(got.chars().count() <= 73, "{got:?} is longer than 72 characters and its ellipsis");
}

#[test]
fn a_headline_of_exactly_72_characters_is_kept_whole() {
    let exact = "a".repeat(30) + " " + &"b".repeat(41);
    assert_eq!(exact.chars().count(), 72);
    assert_eq!(headline(&format!("- **{exact}** Body.")), exact);
}

#[test]
fn the_cut_counts_characters_not_bytes() {
    let wide = "— ".repeat(36);
    let wide = wide.trim_end();
    assert_eq!(wide.chars().count(), 71);
    assert_eq!(headline(&format!("- **{wide}** Body.")), wide, "71 characters were cut as if they were their bytes");
}

#[test]
fn a_release_title_is_its_first_entrys_headline() {
    assert_eq!(release_title(section(LOG, "0.9.16").unwrap()), "IEEE 1800-2023 SVA upgrade");
}

#[test]
fn a_release_title_can_be_a_prose_preamble() {
    assert_eq!(release_title(section(LOG, "0.5.5").unwrap()), "First public release");
}

#[test]
fn unreleased_lists_every_addition_and_counts_the_rest() {
    assert_eq!(
        unreleased(LOG),
        Unreleased {
            added: vec![
                "Shipped assets travel compressed".to_string(),
                "Real reactive signals are a first-class value family".to_string(),
            ],
            changed: 2,
            fixed: 3,
        }
    );
}

#[test]
fn a_log_with_no_unreleased_section_has_nothing_in_progress() {
    assert_eq!(unreleased("# Changelog\n\n## [0.1.0] - 2026-01-01\n\n### Added\n- **A thing.**\n"), Unreleased::default());
}
