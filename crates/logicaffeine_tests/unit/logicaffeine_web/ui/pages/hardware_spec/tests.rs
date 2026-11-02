use super::*;

#[test]
fn a_hash_then_space_or_end_opens_a_comment() {
    assert!(opens_comment(Some(' ')));
    assert!(opens_comment(Some('\t')));
    assert!(opens_comment(None));
}

#[test]
fn a_verilog_delay_opens_no_comment() {
    assert!(!opens_comment(Some('5')));
    assert!(!opens_comment(Some('(')));
    assert!(!opens_comment(Some('d')));
}

#[test]
fn a_line_loses_its_comment() {
    assert_eq!(content("registers: 3 # the budget"), "registers: 3");
    assert_eq!(content("  tmp: 1-5   #live across the loop"), "tmp: 1-5   #live across the loop");
    assert_eq!(content("# a whole-line comment"), "");
    assert_eq!(content("## Pigeonhole"), "");
    assert_eq!(content("pigeons: 6"), "pigeons: 6");
}

#[test]
fn a_field_is_named_lowercase_or_capitalised() {
    assert!(is_field("pigeons", PIGEONS));
    assert!(is_field("Pigeons", PIGEONS));
    assert!(!is_field("PIGEONS", PIGEONS));
    assert!(!is_field("pigeon", PIGEONS));
    assert!(is_field("Registers", REGISTERS));
    assert!(!is_field("tmp", REGISTERS));
}

#[test]
fn a_field_value_reads_past_the_colon_and_before_the_comment() {
    assert_eq!(field_value("pigeons: 6", PIGEONS), Some("6"));
    assert_eq!(field_value("  Pigeons:  4  ", PIGEONS), Some("4"));
    assert_eq!(field_value("registers: 3 # the budget", REGISTERS), Some("3"));
    assert_eq!(field_value("registers : 3", REGISTERS), None, "the colon follows the field directly");
    assert_eq!(field_value("pigeons: 6", REGISTERS), None);
    assert_eq!(field_value("# pigeons: 6", PIGEONS), None, "a commented-out field is no field");
}
