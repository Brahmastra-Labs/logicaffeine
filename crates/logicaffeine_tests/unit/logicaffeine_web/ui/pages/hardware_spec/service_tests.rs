use super::*;
use crate::ui::components::problems::{Problem, Severity};

fn at(source: &str, word: &str) -> usize {
    source.find(word).unwrap_or_else(|| panic!("{word:?} is not in {source:?}"))
}

fn under<'a>(source: &'a str, problem: &Problem) -> &'a str {
    &source[problem.start..problem.end]
}

#[test]
fn hovering_the_pigeons_field_explains_it() {
    let source = "# six birds\npigeons: 6\n";
    let text = hover(source, at(source, "pigeons") + 2).expect("the field has a hover");
    assert!(text.contains("pigeons") && text.contains("holes"), "{text}");
    assert!(text.contains('2') && text.contains("20"), "the hover names the range it accepts: {text}");
}

#[test]
fn hovering_the_registers_field_explains_it() {
    let source = "registers: 3\ntmp: 1-5\n";
    let text = hover(source, at(source, "registers")).expect("the field has a hover");
    assert!(text.contains("register") && text.contains("live range"), "{text}");
}

#[test]
fn hovering_a_live_range_reads_it() {
    let source = "registers: 3\ntmp: 1-5\n";
    let text = hover(source, at(source, "tmp")).expect("a live range has a hover");
    assert!(text.contains("tmp") && text.contains('1') && text.contains('5'), "{text}");
}

#[test]
fn a_comment_or_a_blank_has_no_hover() {
    let source = "# six birds\n\npigeons: 6\n";
    assert_eq!(hover(source, 3), None);
    assert_eq!(hover(source, at(source, "\n\n") + 1), None);
}

#[test]
fn a_count_that_is_no_number_is_an_error_under_it() {
    let source = "pigeons: lots\n";
    let found = problems(source);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].severity, Severity::Error);
    assert_eq!(under(source, &found[0]), "lots");
    assert!(found[0].message.contains("whole number"), "{}", found[0].message);
}

#[test]
fn a_count_out_of_range_is_an_error_naming_the_range() {
    for source in ["pigeons: 99\n", "pigeons: 1\n"] {
        let found = problems(source);
        assert_eq!(found.len(), 1, "{source:?}: {found:?}");
        assert!(found[0].message.contains('2') && found[0].message.contains("20"), "{}", found[0].message);
        assert_eq!(under(source, &found[0]), source["pigeons: ".len()..].trim_end());
    }
}

/// Any other line stops the parser reading a pigeonhole spec as one; the editor says so there.
#[test]
fn a_second_line_in_a_pigeonhole_spec_is_an_error() {
    let source = "pigeons: 6\nholes: 3\n";
    let found = problems(source);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(under(source, &found[0]), "holes: 3");
    assert!(crate::ui::pages::pigeonhole_viz::parse_pigeonhole_spec(source).is_none(), "the parser refuses it too");
}

#[test]
fn a_budget_that_is_no_number_is_an_error() {
    let source = "registers: many\na: 0-2\n";
    let found = problems(source);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(under(source, &found[0]), "many");
}

/// The register parser drops a live range that ends before it starts; the editor says so.
#[test]
fn a_live_range_that_ends_before_it_starts_is_an_error() {
    let source = "registers: 3\nb: 5-1\na: 0-2\n";
    let found = problems(source);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(under(source, &found[0]), "5-1");
    assert!(found[0].message.contains("b"), "the message names the variable: {}", found[0].message);
}

#[test]
fn a_live_range_that_is_no_range_is_an_error() {
    let source = "registers: 3\nb: soon\na: 0-2\n";
    let found = problems(source);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(under(source, &found[0]), "soon");
}

#[test]
fn a_spec_that_names_no_field_is_not_checked() {
    for source in [
        "Always, if request is high, then acknowledge is high.",
        "module m(input clk); reg a; endmodule",
        "Movements: ns-through, ew-through.",
    ] {
        assert!(problems(source).is_empty(), "{source:?}: {:?}", problems(source));
    }
}

#[test]
fn every_shipped_spec_has_no_problems() {
    for (name, source) in crate::ui::examples::PIGEONHOLE_EXAMPLES.iter().chain(crate::ui::examples::REGALLOC_EXAMPLES) {
        assert!(problems(source).is_empty(), "{name}: {:?}", problems(source));
    }
}
