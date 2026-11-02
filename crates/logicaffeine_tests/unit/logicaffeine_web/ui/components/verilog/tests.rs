use super::*;
use crate::ui::examples::ALL_HARDWARE_SPECS;

fn kinds(source: &str) -> Vec<(String, TokenKind)> {
    tokens(source).into_iter().filter(|t| !t.text.trim().is_empty()).map(|t| (t.text, t.kind)).collect()
}

fn kind_of(source: &str, text: &str) -> TokenKind {
    kinds(source)
        .into_iter()
        .find(|(t, _)| t == text)
        .unwrap_or_else(|| panic!("no token {text:?} in {source:?}: {:?}", kinds(source)))
        .1
}

#[test]
fn the_tokens_spell_every_design_back() {
    let designs: Vec<_> = ALL_HARDWARE_SPECS.iter().filter(|s| s.vfs_path.ends_with(".v")).collect();
    assert!(!designs.is_empty());
    for design in designs {
        let spelled: String = tokens(design.source).into_iter().map(|t| t.text).collect();
        assert_eq!(spelled, design.source, "{}", design.vfs_path);
    }
}

#[test]
fn structure_words_and_types_are_keywords() {
    let src = "module m(input clk, output reg q);\n  wire w; logic l; integer i; reg signed [3:0] s;\n  initial begin q = 0; end\n  always @(posedge clk) if (w) q <= 1; else q <= 0;\n  assign w = 1;\nendmodule";
    for word in ["module", "input", "output", "initial", "begin", "end", "always", "posedge", "if", "else", "assign", "endmodule"] {
        assert_eq!(kind_of(src, word), TokenKind::Keyword, "{word}");
    }
    for word in ["wire", "logic", "integer", "reg", "signed"] {
        assert_eq!(kind_of(src, word), TokenKind::Keyword, "{word}");
    }
    assert_eq!(kind_of(src, "clk"), TokenKind::Identifier);
}

#[test]
fn a_word_that_starts_like_a_keyword_is_a_name() {
    let src = "reg endpoint; wire regs; wire module_a; wire begin_x;";
    for name in ["endpoint", "regs", "module_a", "begin_x"] {
        assert_eq!(kind_of(src, name), TokenKind::Identifier, "{name}");
    }
}

#[test]
fn sized_and_based_literals_are_one_number_each() {
    let src = "x = 4'd8 + 8'hFF + 1'b0 + 'b1010 + 32'sd5 + 12 + 3'b1_0_1 + 16'hx0Z;";
    for literal in ["4'd8", "8'hFF", "1'b0", "'b1010", "32'sd5", "12", "3'b1_0_1", "16'hx0Z"] {
        assert_eq!(kind_of(src, literal), TokenKind::Number, "{literal}");
    }
}

#[test]
fn comments_run_to_their_end_across_lines() {
    let src = "// a line comment\nreg q; /* a block\n   comment */ wire w;";
    assert_eq!(kind_of(src, "// a line comment"), TokenKind::Comment);
    assert_eq!(kind_of(src, "/* a block\n   comment */"), TokenKind::Comment);
    assert_eq!(kind_of(src, "wire"), TokenKind::Keyword, "code resumes after the block comment");
    let open = "reg q; /* never closed\nwire w;";
    assert_eq!(kind_of(open, "/* never closed\nwire w;"), TokenKind::Comment, "an unclosed comment runs to the end");
}

#[test]
fn strings_keep_their_escapes_and_hide_comment_marks() {
    let src = r#"$display("a \" // b /* c");"#;
    assert_eq!(kind_of(src, r#""a \" // b /* c""#), TokenKind::String);
    assert_eq!(kind_of(src, "$display"), TokenKind::Builtin);
}

#[test]
fn system_tasks_and_directives_are_builtins() {
    let src = "`timescale 1ns/1ps\n`define W 8\ninitial $finish;";
    assert_eq!(kind_of(src, "`timescale"), TokenKind::Builtin);
    assert_eq!(kind_of(src, "`define"), TokenKind::Builtin);
    assert_eq!(kind_of(src, "$finish"), TokenKind::Builtin);
}

#[test]
fn operators_are_whole() {
    let src = "q <= a == b && !c || d !== e; @(x) #5 y = ~z ^ (w << 2) >>> 1;";
    for op in ["<=", "==", "&&", "!", "||", "!==", "@", "#", "=", "~", "^", "<<", ">>>"] {
        assert_eq!(kind_of(src, op), TokenKind::Operator, "{op}");
    }
    for p in [";", "(", ")"] {
        assert_eq!(kind_of(src, p), TokenKind::Punctuation, "{p}");
    }
}

#[test]
fn an_escaped_identifier_runs_to_whitespace() {
    let src = r"wire \bus[0] ;";
    assert_eq!(kind_of(src, r"\bus[0]"), TokenKind::Identifier);
}
