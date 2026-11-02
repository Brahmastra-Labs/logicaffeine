use super::{tokenize, Language, TokenKind};

fn kinds(source: &str) -> Vec<(String, bool, bool)> {
    tokenize(source, Language::Hardware)
        .into_iter()
        .filter(|t| !t.text.trim().is_empty())
        .map(|t| (t.text, t.kind == TokenKind::Keyword, t.kind == TokenKind::Comment))
        .collect()
}

fn is_keyword(source: &str, word: &str) -> bool {
    kinds(source).iter().any(|(t, kw, _)| t == word && *kw)
}

#[test]
fn an_english_spec_highlights_its_temporal_words() {
    let spec = "Always, if request is high, then acknowledge is high in the next cycle.";
    for word in ["Always", "if", "then", "is", "high", "next", "cycle"] {
        assert!(is_keyword(spec, word), "{word} is not highlighted in {spec:?}");
    }
    assert!(!is_keyword(spec, "request"), "a signal name is not a keyword");
}

#[test]
fn a_signal_plan_highlights_its_structure() {
    let plan = "Movements: ns-through, ew-through.\nns-through conflicts with ew-through.";
    for word in ["Movements", "conflicts", "with"] {
        assert!(is_keyword(plan, word), "{word} is not highlighted in {plan:?}");
    }
}

#[test]
fn verilog_highlights_its_keywords_and_comments() {
    let rtl = "// a flip-flop\nmodule m(input clk);\n  reg q;\n  always @(posedge clk) q <= ~q;\nendmodule";
    for word in ["module", "input", "reg", "always", "posedge", "endmodule"] {
        assert!(is_keyword(rtl, word), "{word} is not highlighted in the Verilog");
    }
    assert!(kinds(rtl).iter().any(|(t, _, comment)| *comment && t == "// a flip-flop"), "the // comment is not highlighted");
}

fn kind_of(source: &str, text: &str) -> Option<TokenKind> {
    tokenize(source, Language::Hardware).into_iter().find(|t| t.text == text).map(|t| t.kind)
}

/// The `.hw` specs (pigeonhole, register allocation) comment with `#`, as the Studio's
/// Hardware parsers read them: the prose is a comment, not code with a stray `is` coloured.
#[test]
fn a_hash_line_is_a_comment_in_a_hardware_spec() {
    let spec = "# Pigeonhole \u{2014} N pigeons into N-1 holes is impossible.\npigeons: 6\n";
    assert_eq!(kind_of(spec, "# Pigeonhole \u{2014} N pigeons into N-1 holes is impossible."), Some(TokenKind::Comment));
    assert!(!is_keyword(spec, "is"), "a word inside the comment is coloured as a keyword");
}

#[test]
fn a_hash_after_a_spec_line_is_a_comment() {
    let spec = "registers: 3 # the physical budget\n";
    assert_eq!(kind_of(spec, "# the physical budget"), Some(TokenKind::Comment));
}

/// Verilog spells a delay `#5` or `#(…)`: that is code, not a comment.
#[test]
fn a_verilog_delay_is_not_a_comment() {
    let rtl = "initial begin\n  #5 a = 1;\n  #(10) b = 0;\nend\n";
    assert!(!kinds(rtl).iter().any(|(_, _, comment)| *comment), "a Verilog delay reads as a comment: {:?}", kinds(rtl));
}

/// A spec's fields (`pigeons:`, `registers:`) are its keywords; the values are numbers.
#[test]
fn a_spec_field_reads_as_a_keyword_and_its_value_as_a_number() {
    for (spec, field, value) in [("pigeons: 6\n", "pigeons", "6"), ("registers: 3\n", "registers", "3")] {
        assert_eq!(kind_of(spec, field), Some(TokenKind::Keyword), "{field} in {spec:?}");
        assert_eq!(kind_of(spec, value), Some(TokenKind::Number), "{value} in {spec:?}");
    }
}

/// A variable's live range (`a: 1-5`) names the variable; it is not a field of the spec.
#[test]
fn a_live_range_names_its_variable() {
    let spec = "registers: 3\ntmp: 1-5\n";
    assert_eq!(kind_of(spec, "tmp"), Some(TokenKind::Identifier));
    assert_eq!(kind_of(spec, "1"), Some(TokenKind::Number));
    assert_eq!(kind_of(spec, "5"), Some(TokenKind::Number));
}
