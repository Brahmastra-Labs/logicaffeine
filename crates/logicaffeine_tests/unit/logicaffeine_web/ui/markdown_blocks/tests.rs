use super::*;

fn p(text: &str) -> Block {
    Block::Paragraph(text.to_string())
}

fn list(ordered: bool, items: &[&str]) -> Block {
    Block::List { ordered, items: items.iter().map(|s| s.to_string()).collect() }
}

#[test]
fn a_paragraph_wrapped_across_lines_is_one_paragraph() {
    assert_eq!(blocks("LOGOS reads\nlike English."), vec![p("LOGOS reads like English.")]);
    assert_eq!(blocks("  indented\n  continuation  "), vec![p("indented continuation")]);
}

#[test]
fn a_blank_line_ends_a_paragraph() {
    assert_eq!(blocks("One.\n\nTwo.\n   \nThree."), vec![p("One."), p("Two."), p("Three.")]);
}

#[test]
fn a_line_that_starts_with_a_number_is_text() {
    assert_eq!(blocks("16 new test files cover it."), vec![p("16 new test files cover it.")]);
    assert_eq!(blocks("512 tests pass. 6,035 in all."), vec![p("512 tests pass. 6,035 in all.")]);
    assert_eq!(blocks("It rose\n12 of 31 times."), vec![p("It rose 12 of 31 times.")]);
}

#[test]
fn a_numbered_list_counts_past_nine() {
    assert_eq!(blocks("9. nine\n10. ten\n11. eleven"), vec![list(true, &["nine", "ten", "eleven"])]);
}

#[test]
fn bullets_and_numbers_are_different_lists() {
    assert_eq!(
        blocks("- a\n* b\n1. one\n2. two"),
        vec![list(false, &["a", "b"]), list(true, &["one", "two"])]
    );
}

#[test]
fn a_list_item_continues_across_its_wrapped_lines() {
    assert_eq!(blocks("- one\n  more\n- two"), vec![list(false, &["one more", "two"])]);
    assert_eq!(blocks("1. first\nstill first\n2. second"), vec![list(true, &["first still first", "second"])]);
}

#[test]
fn a_blank_line_ends_a_list() {
    assert_eq!(blocks("- a\n\nAfter."), vec![list(false, &["a"]), p("After.")]);
}

#[test]
fn a_list_or_heading_or_quote_can_follow_a_paragraph_directly() {
    assert_eq!(blocks("Intro:\n- a"), vec![p("Intro:"), list(false, &["a"])]);
    assert_eq!(
        blocks("Intro.\n### Next\nBody."),
        vec![p("Intro."), Block::Heading { level: 3, text: "Next".to_string() }, p("Body.")]
    );
    assert_eq!(blocks("Intro.\n> said"), vec![p("Intro."), Block::Quote(vec!["said".to_string()])]);
}

#[test]
fn bold_is_not_a_bullet() {
    assert_eq!(blocks("**Bold** start."), vec![p("**Bold** start.")]);
}

#[test]
fn a_heading_names_its_level() {
    assert_eq!(
        blocks("# One\n## Two\n###### Six\n####### Seven"),
        vec![
            Block::Heading { level: 1, text: "One".to_string() },
            Block::Heading { level: 2, text: "Two".to_string() },
            Block::Heading { level: 6, text: "Six".to_string() },
            p("####### Seven"),
        ]
    );
}

#[test]
fn a_quote_continues_across_its_wrapped_lines() {
    assert_eq!(
        blocks("> **Mental model:** a\n> b.\n>\n> Second."),
        vec![Block::Quote(vec!["**Mental model:** a b.".to_string(), "Second.".to_string()])]
    );
}

#[test]
fn a_blank_line_ends_a_quote() {
    assert_eq!(
        blocks("> a\n\n> b"),
        vec![Block::Quote(vec!["a".to_string()]), Block::Quote(vec!["b".to_string()])]
    );
}

#[test]
fn a_line_of_three_or_more_dashes_stars_or_underscores_is_a_rule() {
    assert_eq!(blocks("One.\n\n---\n\nTwo."), vec![p("One."), Block::Rule, p("Two.")]);
    assert_eq!(blocks("***\n___\n- - -\n-----"), vec![Block::Rule, Block::Rule, Block::Rule, Block::Rule]);
}

#[test]
fn a_rule_ends_the_block_before_it() {
    assert_eq!(blocks("One.\n---\nTwo."), vec![p("One."), Block::Rule, p("Two.")]);
    assert_eq!(blocks("- a\n---"), vec![list(false, &["a"]), Block::Rule]);
}

#[test]
fn fewer_than_three_or_mixed_marks_are_text() {
    assert_eq!(blocks("--"), vec![p("--")]);
    assert_eq!(blocks("-*-"), vec![p("-*-")]);
    assert_eq!(blocks("--- x"), vec![p("--- x")]);
}

#[test]
fn a_fence_keeps_its_lines_exactly() {
    assert_eq!(
        blocks("Before.\n```logos\n## Main\n    Show 1.\n\n- not a list\n```\nAfter."),
        vec![
            p("Before."),
            Block::Fence { tag: "logos".to_string(), code: "## Main\n    Show 1.\n\n- not a list".to_string() },
            p("After."),
        ]
    );
}

#[test]
fn a_fence_without_a_language_or_an_end_still_holds_its_lines() {
    assert_eq!(blocks("```\nx\n"), vec![Block::Fence { tag: String::new(), code: "x".to_string() }]);
}

#[test]
fn a_table_reads_its_header_and_rows() {
    assert_eq!(
        blocks("| A | B |\n|---|:-:|\n| 1 | 2 |\n| 3 | 4 |"),
        vec![Block::Table {
            header: Some(vec!["A".to_string(), "B".to_string()]),
            rows: vec![vec!["1".to_string(), "2".to_string()], vec!["3".to_string(), "4".to_string()]],
        }]
    );
}

#[test]
fn a_table_without_a_separator_has_no_header() {
    assert_eq!(
        blocks("| a | b |\n| c | d |"),
        vec![Block::Table {
            header: None,
            rows: vec![vec!["a".to_string(), "b".to_string()], vec!["c".to_string(), "d".to_string()]],
        }]
    );
}

#[test]
fn a_cell_that_holds_dashes_is_not_a_separator() {
    assert_eq!(
        blocks("| a | b |\n| --- is text | x |"),
        vec![Block::Table {
            header: None,
            rows: vec![vec!["a".to_string(), "b".to_string()], vec!["--- is text".to_string(), "x".to_string()]],
        }]
    );
}
