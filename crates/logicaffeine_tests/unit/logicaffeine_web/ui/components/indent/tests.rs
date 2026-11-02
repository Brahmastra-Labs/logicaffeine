use super::*;

const RTL: &str = "module m(input clk);\nendmodule\n";

fn logos() -> Indentation {
    Indentation::of(Language::Logos, "")
}
fn math() -> Indentation {
    Indentation::of(Language::Vernacular, "")
}
fn verilog() -> Indentation {
    Indentation::of(Language::Hardware, RTL)
}
fn spec() -> Indentation {
    Indentation::of(Language::Hardware, "Always, if request is high, then acknowledge is high.")
}
fn logic() -> Indentation {
    Indentation::of(Language::Logic, "")
}

fn enter_at_end(ind: Indentation, line: &str) -> Enter {
    ind.enter(line, line.len())
}

#[test]
fn each_language_has_its_step() {
    assert_eq!(logos().step, "    ");
    assert_eq!(math().step, "    ");
    assert_eq!(logic().step, "    ");
    assert_eq!(spec().step, "    ");
    assert_eq!(verilog().step, "  ", "Verilog is written two spaces a step");
}

#[test]
fn enter_keeps_the_lines_indentation() {
    assert_eq!(enter_at_end(logos(), "    Let x be 1.").insert, "\n    ");
    assert_eq!(enter_at_end(logos(), "Let x be 1.").insert, "\n");
    assert_eq!(enter_at_end(spec(), "  a is high.").insert, "\n  ");
    assert_eq!(enter_at_end(logic(), "Every cat sleeps.").insert, "\n");
}

#[test]
fn a_logos_line_ending_in_a_colon_opens_a_block() {
    assert_eq!(enter_at_end(logos(), "If x is greater than 5:").insert, "\n    ");
    assert_eq!(enter_at_end(logos(), "    Repeat for i from 1 to 5:").insert, "\n        ");
    assert_eq!(enter_at_end(logos(), "## To double (n: Int) -> Int:").insert, "\n    ");
    assert_eq!(enter_at_end(logos(), "If x is 1:  -- the one case").insert, "\n    ", "a trailing comment does not hide the colon");
    assert_eq!(enter_at_end(logos(), "Show \"a: b\".").insert, "\n");
}

#[test]
fn enter_in_the_middle_of_a_line_carries_the_rest_at_the_lines_indentation() {
    let line = "    Let x be 1. Show x.";
    let at = line.find("Show").unwrap();
    assert_eq!(logos().enter(line, at).insert, "\n    ");
    let opener = "If x is 1: Show x.";
    assert_eq!(logos().enter(opener, opener.find(" Show").unwrap()).insert, "\n    ", "split after the colon");
}

#[test]
fn enter_inside_the_leading_space_adds_no_indentation() {
    assert_eq!(logos().enter("        Show x.", 4).insert, "\n    ");
    assert_eq!(logos().enter("    If x:", 0).insert, "\n");
}

#[test]
fn math_opens_blocks_the_ways_its_sources_do() {
    for line in [
        "## Theorem: FxRefl",
        "## Lemma: Small",
        "## To check_parity (n: Nat) -> Decision:",
        "    Consider n:",
        "Definition nuclear : Syntax -> Derivation :=",
        "  match n with",
        "Proof.",
    ] {
        assert!(math().opens_block(line), "{line:?} opens a block");
    }
    for line in ["Check FxRefl.", "        When Zero: Yield Yes.", "## Main", "Definition one : Nat := Succ Zero.", "-- a comment:"] {
        assert!(!math().opens_block(line), "{line:?} does not open a block");
    }
    assert_eq!(enter_at_end(math(), "## Theorem: FxRefl").insert, "\n    ");
}

#[test]
fn a_math_closer_moves_back_a_step_on_enter() {
    let e = enter_at_end(math(), "        end.");
    assert_eq!(e.reindent.as_deref(), Some("    "));
    assert_eq!(e.insert, "\n    ");
    assert_eq!(enter_at_end(math(), "Qed.").reindent, None, "already at the margin");
}

#[test]
fn verilog_opens_and_closes_its_blocks() {
    assert_eq!(enter_at_end(verilog(), "module arbiter(input clk, input r0);").insert, "\n  ");
    assert_eq!(enter_at_end(verilog(), "  always @(posedge clk) begin").insert, "\n    ");
    assert_eq!(enter_at_end(verilog(), "    case (state)").insert, "\n      ");
    assert_eq!(enter_at_end(verilog(), "  reg g0;").insert, "\n  ");
    let end = enter_at_end(verilog(), "    end");
    assert_eq!((end.reindent.as_deref(), end.insert.as_str()), (Some("  "), "\n  "));
    assert!(verilog().closes_block("  endmodule"));
    assert!(verilog().closes_block("  end else begin"));
    assert!(!verilog().closes_block("  ending <= 1;"), "a name that starts with end is not a closer");
    assert!(!verilog().opens_block("  // begin"), "a comment opens nothing");
}

#[test]
fn a_line_that_closes_and_reopens_stays_at_its_depth_and_opens() {
    let e = enter_at_end(verilog(), "    end else begin");
    assert_eq!(e.reindent.as_deref(), Some("  "), "`end` moves back to its block's depth");
    assert_eq!(e.insert, "\n    ", "and the next line sits inside the `else` block it opens");
}

#[test]
fn logic_and_specs_never_deepen() {
    assert_eq!(enter_at_end(logic(), "## Theorem: Socrates").insert, "\n");
    assert_eq!(enter_at_end(logic(), "Given: All men are mortal:").insert, "\n");
    assert_eq!(enter_at_end(spec(), "Movements: ns-through, ew-through.").insert, "\n");
}

#[test]
fn tab_fills_to_the_next_stop() {
    assert_eq!(logos().tab_fill(0), "    ");
    assert_eq!(logos().tab_fill(1), "   ");
    assert_eq!(logos().tab_fill(4), "    ");
    assert_eq!(logos().tab_fill(6), "  ");
    assert_eq!(verilog().tab_fill(3), " ");
}

#[test]
fn tab_over_lines_deepens_each_and_leaves_blank_lines_empty() {
    assert_eq!(logos().indent_lines("Let x be 1.\n\n    Show x."), "    Let x be 1.\n\n        Show x.");
}

#[test]
fn shift_tab_moves_each_line_back_as_far_as_its_spaces_go() {
    assert_eq!(logos().outdent_lines("        a\n  b\nc\n\td"), "    a\nb\nc\nd");
    assert_eq!(verilog().outdent_lines("    x\n y"), "  x\ny");
}

#[test]
fn indenting_then_outdenting_gives_the_lines_back() {
    let text = "## Main\n    Let x be 1.\n\nShow x.";
    assert_eq!(logos().outdent_lines(&logos().indent_lines(text)), text);
}
