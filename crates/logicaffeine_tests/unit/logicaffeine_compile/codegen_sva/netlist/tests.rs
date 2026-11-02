use super::*;

fn module_json(cells: &str) -> String {
    format!(r#"{{"creator": "Yosys 0.33", "modules": {{"top": {{"ports": {{}}, "cells": {{{cells}}}, "netnames": {{}}}}}}}}"#)
}

#[test]
fn the_script_flattens_frees_undefined_bits_and_maps_to_and_gates_in_order() {
    let s = script("logos_fmul_p", &[("WIDTH", "8"), ("NAME", "\"x\"")]);
    let order = [
        "read_verilog -sv design.v",
        "hierarchy -check -top logos_fmul_p -chparam WIDTH 8 -chparam NAME \"x\"",
        "proc",
        "flatten",
        "memory_map",
        "write_json design.json",
        "setundef -anyseq",
        "async2sync",
        "techmap",
        "dffunmap",
        "aigmap",
        "write_aiger -ascii -symbols design.aag",
    ];
    let mut at = 0;
    for step in order {
        let found = s[at..].find(step).unwrap_or_else(|| panic!("`{step}` after byte {at} in\n{s}"));
        at += found + step.len();
    }
    assert!(!s.contains("opt_expr") && !s.contains(" opt;") && !s.contains("abc"), "no pass that may refine an undefined bit or restructure logic: {s}");
}

#[test]
fn one_clock_and_no_latches_is_one_domain() {
    let json = module_json(r#""a": {"type": "$dff", "connections": {"CLK": [2], "D": [3], "Q": [4]}},
                              "b": {"type": "$sdffe", "connections": {"CLK": [2], "D": [4], "Q": [5]}}"#);
    assert_eq!(domains(&json), Ok(Domains { flops: 2, clocks: 1, latches: 0 }));
}

#[test]
fn two_clocks_count_twice() {
    let json = module_json(r#""a": {"type": "$dff", "connections": {"CLK": [3]}},
                              "b": {"type": "$adff", "connections": {"CLK": [2], "ARST": [7]}}"#);
    assert_eq!(domains(&json).unwrap().clocks, 2);
}

#[test]
fn a_level_latch_is_counted() {
    let json = module_json(r#""l": {"type": "$dlatch", "connections": {"EN": [2], "D": [3], "Q": [4]}}"#);
    assert_eq!(domains(&json), Ok(Domains { flops: 0, clocks: 0, latches: 1 }));
}

#[test]
fn a_combinational_module_has_no_domain() {
    let json = module_json(r#""m": {"type": "$mul", "connections": {"A": [2], "B": [3], "Y": [4]}}"#);
    assert_eq!(domains(&json), Ok(Domains { flops: 0, clocks: 0, latches: 0 }));
}

#[test]
fn json_that_is_not_a_yosys_netlist_is_refused() {
    assert!(matches!(domains("{\"modules\": 3}"), Err(NetlistError::Json(_))));
    assert!(matches!(domains("not json"), Err(NetlistError::Json(_))));
}

#[test]
fn a_design_with_two_clocks_or_a_level_latch_is_refused_by_name() {
    assert_eq!(admit(Domains { flops: 4, clocks: 2, latches: 0 }), Err(NetlistError::Clocks(2)));
    assert_eq!(admit(Domains { flops: 0, clocks: 0, latches: 3 }), Err(NetlistError::LevelLatches(3)));
    assert_eq!(admit(Domains { flops: 9, clocks: 1, latches: 0 }), Ok(()));
    assert_eq!(admit(Domains { flops: 0, clocks: 0, latches: 0 }), Ok(()));
}

#[test]
fn an_undefined_bit_is_free_by_default_and_zero_when_read_so() {
    assert!(script("t", &[]).contains("setundef -anyseq\n") && !script("t", &[]).contains("setundef -zero"));
    let zero = script_reading("t", &[], Undefined::Zero);
    assert!(zero.contains("setundef -zero\n") && !zero.contains("anyseq"));
    assert_eq!(script_reading("t", &[], Undefined::Free), script("t", &[]));
    let at = |s: &str, step: &str| s.find(step).unwrap_or_else(|| panic!("{step}"));
    assert!(at(&zero, "write_json design.json") < at(&zero, "setundef -zero") && at(&zero, "setundef -zero") < at(&zero, "techmap"));
}

#[test]
fn every_multiplier_is_lowered_by_the_one_fixed_map_before_the_rest() {
    let s = script("t", &[]);
    let (map, rest) = (s.find("techmap -map logos_mul.v\n").expect("the map"), s.find("techmap\n").expect("the rest"));
    assert!(map < rest, "{s}");
    assert!(CANONICAL_MUL.contains("(* techmap_celltype = \"$mul\" *)"));
    assert!(!CANONICAL_MUL.contains("alumacc") && !CANONICAL_MUL.contains("$macc"), "no hash-ordered lowering");
}

#[test]
fn identical_cells_are_merged_once_every_undefined_bit_is_its_own_before_anything_is_lowered() {
    for undefined in [Undefined::Free, Undefined::Zero] {
        let s = script_reading("t", &[], undefined);
        let at = |step: &str| s.find(step).unwrap_or_else(|| panic!("`{step}` in\n{s}"));
        let merge = at("opt_merge\n");
        assert!(at("setundef ") < merge, "a free bit is a cell of its own before cells are merged: {s}");
        assert!(merge < at("techmap -map logos_mul.v\n"), "one multiplier is lowered for each product: {s}");
    }
}

#[test]
fn undefined_bits_are_read_again_after_the_cells_are_lowered() {
    // A part-select at a moving offset is a shift whose bits past its operand are undefined only once techmap lowers
    // it; they are read the same way as every other undefined bit.
    for (s, read) in [(script("t", &[]), "setundef -anyseq"), (script_reading("t", &[], Undefined::Zero), "setundef -zero")] {
        let lines: Vec<&str> = s.lines().collect();
        let reads: Vec<usize> = (0..lines.len()).filter(|&i| lines[i] == read).collect();
        let lowered = lines.iter().rposition(|l| *l == "techmap").unwrap();
        let aig = lines.iter().position(|l| *l == "aigmap").unwrap();
        assert_eq!(reads.len(), 2, "{s}");
        assert!(reads[0] < lowered && lowered < reads[1] && reads[1] < aig, "{s}");
    }
}

#[test]
fn a_parameter_value_has_one_spelling_for_one_value_its_width_and_sign_kept() {
    let ok = |signed, real, text: &str| rtlil_value(signed, real, text).unwrap_or_else(|e| panic!("{text}: {e}"));
    assert_eq!(ok(true, false, "32'00000000000000000000000000000001"), "32'sd1");
    assert_eq!(ok(false, false, "32'00000000000000000000000000000001"), "32'd1");
    assert_eq!(ok(true, false, "32'11111111111111111111111111111111"), "-32'sd1");
    assert_eq!(ok(false, false, "32'11111111111111111111111111111111"), "32'd4294967295");
    assert_eq!(ok(true, false, "32'10000000000000000000000000000000"), "-32'sd2147483648");
    assert_eq!(ok(false, false, "4'0010"), "4'd2");
    assert_eq!(ok(true, false, "4'1110"), "-4'sd2");
    assert_eq!(ok(false, false, "4'0010"), ok(false, false, "4'0010"));
    assert_ne!(ok(false, false, "4'0010"), ok(false, false, "5'00010"), "a width is part of the value");
    assert_eq!(ok(false, false, "3'1x0"), "3'b1x0");
    assert_eq!(ok(true, false, "2'z1"), "2'sbz1");
    let wide = format!("130'1{}", "0".repeat(129));
    assert_eq!(ok(false, false, &wide), format!("130'b1{}", "0".repeat(129)));
    let full = format!("128'1{}", "0".repeat(127));
    assert_eq!(ok(true, false, &full), "-128'sd170141183460469231731687303715884105728");
    assert_eq!(ok(false, false, &format!("128'{}", "1".repeat(128))), format!("128'd{}", u128::MAX));
    assert_eq!(ok(false, false, "\"ab\""), "\"ab\"");
    assert_eq!(ok(false, true, "\"1.5\""), "real \"1.5\"");
    assert_eq!(ok(true, false, "-1"), "-32'sd1", "a bare integer is a 32-bit constant");
    assert_eq!(ok(false, false, "-1"), "32'd4294967295");
    assert_eq!(ok(false, false, "0'"), "0'b");
    for bad in ["4'001", "4'00201", "x'0101", "\"open", "", "1.5", "99999999999"] {
        assert!(matches!(rtlil_value(false, false, bad), Err(NetlistError::Rtlil(_))), "{bad:?} is no constant");
    }
    assert!(matches!(rtlil_value(false, true, "4'0010"), Err(NetlistError::Rtlil(_))), "a real is written as a string");
}

const RTLIL: &str = r#"# Generated by Yosys 0.33
autoidx 2
attribute \top 1
attribute \src "design.v:4.1-8.10"
module \top
  wire width 4 input 1 \a
  attribute \module_not_derived 1
  cell \u \lane[0].m.i1
    parameter signed \K 32'11111111111111111111111111111111
    parameter \M 4'0010
    connect \a \a
  end
  cell \u $flatten\i2
    parameter real \R "2.5"
    parameter \K 32'11111111111111111111111111111111
  end
  cell \u \i3
    connect \a \a
  end
  cell \other \o
    parameter \K 4'0001
  end
  cell $and $and$design.v:3$1
    parameter \A_WIDTH 1
  end
end
attribute \blackbox 1
module \u
  parameter \K 1
  cell \u \inner
    parameter \K 4'0011
  end
end
"#;

#[test]
fn each_units_instance_in_the_top_gives_the_parameters_its_cell_carries_in_name_order() {
    let found = given_parameters(RTLIL, &["u"]).unwrap();
    let mut names: Vec<&str> = found.keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(names, ["$flatten\\i2", "i3", "lane[0].m.i1"], "the top's cells of the unit module alone, named as the JSON names them");
    assert_eq!(found["lane[0].m.i1"], [("K".to_string(), "-32'sd1".to_string()), ("M".to_string(), "4'd2".to_string())]);
    assert_eq!(found["$flatten\\i2"], [("K".to_string(), "32'd4294967295".to_string()), ("R".to_string(), "real \"2.5\"".to_string())]);
    assert!(found["i3"].is_empty(), "an instance giving nothing leaves every parameter its module's");
    assert_eq!(given_parameters(RTLIL, &["u", "other"]).unwrap()["o"], [("K".to_string(), "4'd1".to_string())]);
}

#[test]
fn an_rtlil_netlist_with_no_top_or_a_cell_yosys_never_writes_is_refused() {
    let no_top = RTLIL.replace("attribute \\top 1\n", "");
    assert_ne!(no_top, RTLIL);
    assert!(matches!(given_parameters(&no_top, &["u"]), Err(NetlistError::Rtlil(_))), "no top module");
    let bad_value = RTLIL.replace("parameter \\M 4'0010", "parameter \\M 4'00");
    assert_ne!(bad_value, RTLIL);
    assert!(matches!(given_parameters(&bad_value, &["u"]), Err(NetlistError::Rtlil(_))));
    let no_value = RTLIL.replace("parameter \\M 4'0010", "parameter \\M");
    assert_ne!(no_value, RTLIL);
    assert!(matches!(given_parameters(&no_value, &["u"]), Err(NetlistError::Rtlil(_))));
    let open_at_end = RTLIL.replace("    parameter \\A_WIDTH 1\n  end\n", "    parameter \\A_WIDTH 1\n");
    assert_ne!(open_at_end, RTLIL);
    assert!(matches!(given_parameters(&open_at_end, &["u"]), Err(NetlistError::Rtlil(_))), "the top ends inside a cell");
    let nested = RTLIL.replace("  cell \\u \\i3\n    connect \\a \\a\n  end\n", "  cell \\u \\i3\n    connect \\a \\a\n");
    assert_ne!(nested, RTLIL);
    assert!(matches!(given_parameters(&nested, &["u"]), Err(NetlistError::Rtlil(_))), "a cell inside a cell");
    let unit = UnitInstance { name: "i".into(), module: "u".into(), parameters: vec![("K".into(), "-32'sd1".into()), ("M".into(), "4'd2".into())], operands: vec![], results: vec![] };
    assert_eq!(unit.function(), "u #(K=-32'sd1, M=4'd2)");
    assert_eq!(UnitInstance { parameters: vec![], ..unit }.function(), "u");
}

#[test]
fn a_cut_writes_the_rtlil_netlist_beside_the_json_before_the_units_are_cut_and_a_plain_synthesis_writes_none() {
    let cut = script_cut("t", &[], Undefined::Free, &["u"]);
    let at = |step: &str| cut.find(step).unwrap_or_else(|| panic!("`{step}` in\n{cut}"));
    assert!(at("write_json design.json") < at("write_rtlil design.il") && at("write_rtlil design.il") < at("expose -evert"), "{cut}");
    assert!(!script("t", &[]).contains("write_rtlil"));
}

const LIBRARY: &str = "`timescale 1ns/1ps
// A library: a top, what it instantiates, and what it does not.
module leaf(input a, output y); assign y = !a; endmodule
module adder (input a, output y);
  leaf l(.a(a), .y(y));
endmodule
module add(input a, output y);
  adder u(.a(a), .y(y));
endmodule
module add_more(input a, output y);
  leaf l(.a(a), .y(y));
endmodule
module unused(input a, output y);
  assign y = a;
endmodule
module mentions(input a, output y);
  // unused is named here in a comment alone
  assign y = a;
endmodule
module top #(parameter W = 1)
  (input a, output y);
  add u(.a(a), .y(y));
endmodule
";

/// The modules a text defines, in order.
fn defined(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.trim_start().strip_prefix("module "))
        .map(|rest| rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect())
        .collect()
}

#[test]
fn yosys_is_handed_the_modules_the_roots_reach_and_every_line_outside_a_module() {
    let kept = reachable_modules(LIBRARY, &["top"]);
    assert_eq!(defined(&kept), ["leaf", "adder", "add", "top"], "in the library's order; `add` never reaches `add_more`");
    assert!(kept.starts_with("`timescale 1ns/1ps\n// A library"), "{kept}");
    assert!(kept.contains("module leaf(input a, output y); assign y = !a; endmodule\n"), "a module on one line");
    assert!(kept.contains("module top #(parameter W = 1)\n  (input a, output y);\n  add u(.a(a), .y(y));\nendmodule\n"), "{kept}");
    assert_eq!(defined(&reachable_modules(LIBRARY, &["mentions"])), ["unused", "mentions"], "a module named in a comment is kept: time, never a wrong design");
    assert_eq!(defined(&reachable_modules(LIBRARY, &["top", "add_more"])), ["leaf", "adder", "add", "add_more", "top"]);
    assert_eq!(reachable_modules(LIBRARY, &["nowhere"]), LIBRARY, "a root no module is named for leaves the text whole, for yosys to refuse");
    assert_eq!(reachable_modules(LIBRARY, &["top", "nowhere"]), LIBRARY);
    let open = LIBRARY.replace("  assign y = a;\nendmodule\nmodule mentions", "  assign y = a;\nmodule mentions");
    assert_eq!(reachable_modules(&open, &["top"]), open, "a module left open leaves the text whole");
}

#[test]
fn a_cut_parts_its_units_from_the_design_and_reads_them_as_ports_alone() {
    let (rest, units) = split_modules(LIBRARY, &["add", "leaf"]).unwrap();
    assert_eq!(defined(&units), ["leaf", "add"], "the units in the text's order");
    assert_eq!(defined(&rest), ["adder", "add_more", "unused", "mentions", "top"]);
    assert!(rest.starts_with("`timescale 1ns/1ps\n// A library"), "every line outside a module stays with the design");
    assert_eq!(rest.len() + units.len(), LIBRARY.len(), "nothing lost, nothing doubled");
    let open = LIBRARY.replace("  assign y = a;\nendmodule\nmodule mentions", "  assign y = a;\nmodule mentions");
    assert_eq!(split_modules(&open, &["add"]), None, "a text whose modules cannot be told apart is not parted");
    let cut = script_cut("t", &[], Undefined::Free, &["u"]);
    let at = |step: &str| cut.find(step).unwrap_or_else(|| panic!("`{step}` in\n{cut}"));
    assert!(at("verilog_defaults -add -defer") < at("read_verilog -sv design.v") && at("read_verilog -sv design.v") < at("verilog_defaults -clear"), "{cut}");
    assert!(at("verilog_defaults -clear") < at("read_verilog -sv -lib units.v") && at("read_verilog -sv -lib units.v") < at("select -assert-any =u"), "{cut}");
    assert!(at("select -assert-any =u") < at("hierarchy -check -top t") && at("verilog_defaults -clear") < at("techmap -map logos_mul.v"), "the deferring cleared before any map is read");
    assert!(!cut.contains("blackbox"), "{cut}");
    let plain = script("t", &[]);
    assert!(plain.contains("verilog_defaults -add -defer\nread_verilog -sv design.v\nverilog_defaults -clear\n") && !plain.contains("units.v"), "{plain}");
}

#[test]
fn a_module_defined_twice_leaves_the_text_whole_and_unparted_for_yosys_to_refuse() {
    let twice = format!("{LIBRARY}module leaf(input a, output y); assign y = a; endmodule\n");
    assert_eq!(reachable_modules(&twice, &["top"]), twice.as_str(), "which leaf a top reaches is yosys's to refuse");
    assert_eq!(split_modules(&twice, &["leaf"]), None);
}
