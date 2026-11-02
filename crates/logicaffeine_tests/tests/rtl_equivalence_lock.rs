//! RTL equivalence, end to end: the engine's Verilog through yosys into and-inverter graphs
//! (`codegen_sva::netlist`), pipelines unrolled to their latency (`logicaffeine_proof::seq_equiv`), and every pair of
//! outputs decided by certified SAT sweeping (`logicaffeine_proof::sweep`) — every equality's LRAT proof replayed,
//! every difference's witness replayed. Incremental SAT sweeping (`logicaffeine_proof::fraig`) decides the same
//! pipelines output for output, its one derivation checked.

mod common;

use logicaffeine_compile::codegen_sva::fp_hard::{hard_verilog, hard_verilog_ce, hard_verilog_for, Block, HardError, Op as HardOp};
use logicaffeine_compile::codegen_sva::fp_hypotheses::{family, fmul_verilog, Semantics};
use logicaffeine_compile::codegen_sva::fp_oracle::{hard_model_verilog, Depths};
use logicaffeine_compile::codegen_sva::netlist::{synthesize, synthesize_cut, NetlistError, Undefined};
use logicaffeine_compile::spatial::decode_engine::numerics::{fadd, fmul};
use logicaffeine_compile::spatial::decode_engine::rtl::act_quant::{act_quant_verilog, WIDE_SCALE_P_LATENCY};
use logicaffeine_compile::spatial::decode_engine::rtl::fp32::{fp32_verilog, F32_TO_F16_LATENCY, FADD_LATENCY, FMUL_LATENCY, TREE16_LATENCY};
use logicaffeine_compile::spatial::decode_engine::rtl::gdn::gdn_stream_verilog;
use logicaffeine_compile::spatial::decode_engine::rtl::lane::{lane_verilog, ROUND88_P_LATENCY};
use logicaffeine_compile::spatial::decode_engine::rtl::simd::{simd_p_verilog, simd_verilog, LANE_FUNC_P_LATENCY, LANE_FUNC_STAGES};
use logicaffeine_compile::spatial::decode_engine::rtl::vector::{vector_verilog, EXP_LATENCY, I2F_LATENCY, LN_LATENCY, RECIP_LATENCY, RSQRT_LATENCY};
use logicaffeine_proof::aig::Aig;
use logicaffeine_proof::fraig;
use logicaffeine_proof::streams::{self, Environment, Ports, StreamSide};
use logicaffeine_proof::units::{self, Binding};
use logicaffeine_proof::seq_equiv::{latency_equivalent, latency_equivalent_by_fraig};
use logicaffeine_proof::sweep::{equivalent, Options, Report, Verdict};

fn aig_of(verilog: &str, top: &str, params: &[(&str, &str)]) -> Aig {
    synthesize(verilog, top, params).unwrap_or_else(|e| panic!("{top}: {e}")).aig
}

/// The word `name[0..width]` read from a named assignment.
fn word(assign: &[(String, bool)], name: &str, width: usize) -> u32 {
    (0..width).fold(0, |acc, i| {
        let bit = assign.iter().find(|(k, _)| *k == format!("{name}[{i}]")).map_or(false, |&(_, v)| v);
        acc | (bit as u32) << i
    })
}

/// The right-hand circuit's output word `y` under a difference's assignment.
fn right_output(report: &Report, inputs: &[bool], latches: &[bool]) -> u32 {
    let values = report.miter.aig.eval(inputs, latches);
    report.miter.pairs.iter().fold(0u32, |acc, (name, _, r)| {
        let i: u32 = name.trim_start_matches("y[").trim_end_matches(']').parse().unwrap();
        acc | (Aig::value(&values, *r) as u32) << i
    })
}

/// Both engines on `pipeline` against `reference` at `latency`: each certificate checking, the same outputs told apart,
/// and none left open by `fraig`.
fn decided(pipeline: &Aig, reference: &Aig, latency: usize) -> (Report, fraig::Report) {
    let sweep = latency_equivalent(pipeline, reference, latency, &Options::default()).unwrap();
    let fraig = latency_equivalent_by_fraig(pipeline, reference, latency, &fraig::Options::default()).unwrap();
    assert!(sweep.check() && fraig.check(), "both certificates check");
    let differ = |verdicts: &[(String, Verdict)]| -> Vec<(String, bool)> {
        verdicts.iter().map(|(n, v)| (n.clone(), matches!(v, Verdict::Differ { .. }))).collect()
    };
    assert_eq!(differ(&sweep.verdicts), differ(&fraig.verdicts), "the engines tell the same outputs apart");
    assert!(fraig.verdicts.iter().all(|(_, v)| !matches!(v, Verdict::Unknown(_))), "fraig decides every output");
    (sweep, fraig)
}

fn left_output(report: &Report, inputs: &[bool], latches: &[bool]) -> u32 {
    let values = report.miter.aig.eval(inputs, latches);
    report.miter.pairs.iter().fold(0u32, |acc, (name, l, _)| {
        let i: u32 = name.trim_start_matches("y[").trim_end_matches(']').parse().unwrap();
        acc | (Aig::value(&values, *l) as u32) << i
    })
}

#[test]
fn the_pipelined_multiply_is_the_combinational_multiply_three_clocks_late() {
    if !common::require_tool("yosys") {
        return;
    }
    let v = fp32_verilog();
    let (report, by_fraig) = decided(&aig_of(v, "logos_fmul_p", &[]), &aig_of(v, "logos_fmul", &[]), FMUL_LATENCY);
    assert_eq!(report.verdicts.len(), 32);
    assert!(report.all_equal(), "{:?}", report.verdicts.iter().filter(|(_, v)| *v != Verdict::Equal).collect::<Vec<_>>());
    assert!(by_fraig.all_equal());
}

#[test]
fn the_pipelined_add_is_the_combinational_add_four_clocks_late() {
    if !common::require_tool("yosys") {
        return;
    }
    let v = fp32_verilog();
    let (report, by_fraig) = decided(&aig_of(v, "logos_fadd_p", &[]), &aig_of(v, "logos_fadd", &[]), FADD_LATENCY);
    assert_eq!(report.verdicts.len(), 32);
    assert!(report.all_equal(), "{:?}", report.verdicts.iter().filter(|(_, v)| *v != Verdict::Equal).collect::<Vec<_>>());
    assert!(by_fraig.all_equal());
}

#[test]
fn a_clock_early_is_told_apart_with_witnesses_that_replay() {
    if !common::require_tool("yosys") {
        return;
    }
    let v = fp32_verilog();
    let (report, _) = decided(&aig_of(v, "logos_fmul_p", &[]), &aig_of(v, "logos_fmul", &[]), FMUL_LATENCY - 1);
    assert!(report.verdicts.iter().any(|(_, v)| matches!(v, Verdict::Differ { .. })));
}

#[test]
fn a_rounding_mutant_is_caught_on_a_tie_the_numerics_round_to_even() {
    if !common::require_tool("yosys") {
        return;
    }
    let honest = "  wire        up = guard2 && (sticky2 || keep2[0]);\n";
    let v = fp32_verilog();
    assert_eq!(v.matches(honest).count(), 1, "the mutation point is logos_fmul_p's round-up");
    let mutant = v.replace(honest, "  wire        up = guard2;\n");
    let (report, _) = decided(&aig_of(&mutant, "logos_fmul_p", &[]), &aig_of(v, "logos_fmul", &[]), FMUL_LATENCY);
    let (inputs, latches) = report
        .verdicts
        .iter()
        .find_map(|(_, v)| match v {
            Verdict::Differ { inputs, latches } => Some((inputs.clone(), latches.clone())),
            _ => None,
        })
        .expect("half-up rounding differs from half-even somewhere");
    let assign = report.named_inputs(&inputs);
    let (a, b) = (word(&assign, "a", 32), word(&assign, "b", 32));
    let reference = right_output(&report, &inputs, &latches);
    assert_eq!(reference, fmul(f32::from_bits(a), f32::from_bits(b)).to_bits(), "the reference RTL is the numerics");
    assert_ne!(left_output(&report, &inputs, &latches), reference, "the mutant is wrong on the witness");
    assert!(report.check());
}

#[test]
fn the_combinational_add_agrees_with_the_numerics_on_its_own_witness_inputs() {
    if !common::require_tool("yosys") {
        return;
    }
    let v = fp32_verilog();
    let mutant = v.replacen("else if (zero) y = 32'd0;", "else if (zero) y = {s1, 31'd0};", 1);
    assert_ne!(mutant, v, "the mutation point is logos_fadd's exact-cancellation zero");
    let report = equivalent(&aig_of(&mutant, "logos_fadd", &[]), &aig_of(v, "logos_fadd", &[]), &Options::default()).unwrap();
    let (inputs, latches) = report
        .verdicts
        .iter()
        .find_map(|(_, v)| match v {
            Verdict::Differ { inputs, latches } => Some((inputs.clone(), latches.clone())),
            _ => None,
        })
        .expect("a cancellation's sign differs");
    let assign = report.named_inputs(&inputs);
    let (a, b) = (word(&assign, "a", 32), word(&assign, "b", 32));
    assert_eq!(right_output(&report, &inputs, &latches), fadd(f32::from_bits(a), f32::from_bits(b)).to_bits());
    assert_eq!(right_output(&report, &inputs, &latches), 0, "an exact cancellation is +0");
    assert!(report.check());
}

#[test]
fn two_clocks_and_level_latches_are_refused_by_name() {
    if !common::require_tool("yosys") {
        return;
    }
    let two = "module two(input c1, input c2, input d, output reg q1, output reg q2);\n\
               always @(posedge c1) q1 <= d;\n always @(posedge c2) q2 <= d;\nendmodule\n";
    assert_eq!(synthesize(two, "two", &[]).err(), Some(NetlistError::Clocks(2)));
    let latch = "module lat(input en, input d, output reg q);\n always @* if (en) q = d;\nendmodule\n";
    assert_eq!(synthesize(latch, "lat", &[]).err(), Some(NetlistError::LevelLatches(1)));
}

#[test]
fn an_undefined_bit_is_any_value_never_a_convenient_one() {
    if !common::require_tool("yosys") {
        return;
    }
    let x = "module m(input s, input a, output y);\n assign y = s ? a : 1'bx;\nendmodule\n";
    let zero = "module m(input s, input a, output y);\n assign y = s ? a : 1'b0;\nendmodule\n";
    let report = equivalent(&aig_of(x, "m", &[]), &aig_of(zero, "m", &[]), &Options::default()).unwrap();
    assert!(matches!(report.verdicts[0].1, Verdict::Differ { .. }), "{:?}", report.verdicts);
    assert!(report.check());
}

#[test]
fn parameters_are_set_before_elaboration() {
    if !common::require_tool("yosys") {
        return;
    }
    let p = "module p #(parameter K = 1) (input [3:0] a, output [3:0] y);\n assign y = a + K;\nendmodule\n";
    let two = "module p(input [3:0] a, output [3:0] y);\n assign y = a + 4'd2;\nendmodule\n";
    let same = equivalent(&aig_of(p, "p", &[("K", "2")]), &aig_of(two, "p", &[]), &Options::default()).unwrap();
    assert!(same.all_equal() && same.check());
    let other = equivalent(&aig_of(p, "p", &[("K", "3")]), &aig_of(two, "p", &[]), &Options::default()).unwrap();
    assert!(!other.all_equal() && other.check());
}

#[test]
fn a_verilog_error_comes_back_with_the_yosys_log() {
    if !common::require_tool("yosys") {
        return;
    }
    match synthesize("module broken(input a, output y);\n assign y = ;\nendmodule\n", "broken", &[]) {
        Err(NetlistError::Yosys { log }) => assert!(log.contains("ERROR"), "{log}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_product_is_one_circuit_however_its_source_spells_it() {
    if !common::require_tool("yosys") {
        return;
    }
    let inline = "module m(input [22:0] ma, input [22:0] mb, input [3:0] k, output [47:0] p, output [3:0] q);
                    assign p = {1'b1, ma} * {1'b1, mb};
                    assign q = k + 4'd1;
                  endmodule";
    let wired = "module m(input [22:0] ma, input [22:0] mb, input [3:0] k, output [47:0] p, output [3:0] q);
                   wire [23:0] fa = {1'b1, ma};
                   wire [23:0] fb = {1'b1, mb};
                   wire [47:0] full = fa * fb;
                   assign p = full;
                   assign q = 4'd1 + k;
                 endmodule";
    let report = equivalent(&aig_of(inline, "m", &[]), &aig_of(wired, "m", &[]), &Options::default()).unwrap();
    assert!(report.all_equal());
    let products = report.miter.pairs.iter().filter(|(n, _, _)| n.starts_with("p[")).collect::<Vec<_>>();
    assert!(products.iter().all(|(_, l, r)| l == r), "every product bit is one node of both sides");
    assert!(report.check());
}

#[test]
fn every_product_is_the_multiplication_of_its_widths_and_signedness() {
    if !common::require_tool("yosys") {
        return;
    }
    for (sa, sb) in [(false, false), (true, true), (true, false), (false, true)] {
        for (wa, wb, wy) in [(3, 2, 5), (4, 3, 4), (2, 4, 7), (3, 3, 3)] {
            let ty = |signed: bool| if signed { "signed " } else { "" };
            let v = format!(
                "module m(input {}[{}:0] a, input {}[{}:0] b, output [{}:0] y); assign y = a * b; endmodule",
                ty(sa),
                wa - 1,
                ty(sb),
                wb - 1,
                wy - 1
            );
            let g = aig_of(&v, "m", &[]);
            for x in 0..1u32 << wa {
                for z in 0..1u32 << wb {
                    let mut inputs = vec![false; g.inputs().len()];
                    for (k, p) in g.inputs().iter().enumerate() {
                        let name = p.name.as_deref().unwrap();
                        let bit: u32 = name[2..name.len() - 1].parse().unwrap();
                        inputs[k] = if name.starts_with("a[") { x >> bit & 1 == 1 } else { z >> bit & 1 == 1 };
                    }
                    let values = g.eval(&inputs, &[]);
                    let got = (0..wy).fold(0i64, |acc, i| {
                        let out = g.outputs().iter().find(|o| o.name.as_deref() == Some(&format!("y[{i}]"))).unwrap();
                        acc | (Aig::value(&values, out.lit) as i64) << i
                    });
                    // Verilog: a mixed product is unsigned; a signed operand sign-extends only when both are.
                    let signed = sa && sb;
                    let value = |v: u32, w: u32| -> i64 { if signed && v >> (w - 1) & 1 == 1 { v as i64 - (1i64 << w) } else { v as i64 } };
                    let want = (value(x, wa) * value(z, wb)).rem_euclid(1i64 << wy);
                    assert_eq!(got, want, "{v}: a={x} b={z}");
                }
            }
        }
    }
}


/// `piped` decided against `unit` at `latency` by incremental SAT sweeping, its derivation checked.
fn by_fraig(verilog: &str, piped: &str, unit: &str, latency: usize) -> fraig::Report {
    let report = latency_equivalent_by_fraig(&aig_of(verilog, piped, &[]), &aig_of(verilog, unit, &[]), latency, &fraig::Options::default()).unwrap();
    assert!(report.check(), "{piped}: the derivation proves every equality and every witness replays");
    report
}

#[test]
fn the_lanes_pipelined_rounder_is_its_rounder_five_clocks_late_and_a_tie_rounded_up_is_told_apart() {
    if !common::require_tool("yosys") {
        return;
    }
    let v = lane_verilog();
    let report = by_fraig(&v, "logos_round88_p", "logos_round88", ROUND88_P_LATENCY);
    assert_eq!(report.verdicts.len(), 32);
    assert!(report.all_equal(), "{:?}", report.verdicts.iter().filter(|(_, v)| *v != Verdict::Equal).collect::<Vec<_>>());
    let honest = "m = m4 + ((rnd4 && (sticky4 || m4[0])) ? 25'd1 : 25'd0);";
    assert_eq!(v.matches(honest).count(), 1, "the mutation point is logos_round88_p's rounding");
    let mutant = v.replace(honest, "m = m4 + (rnd4 ? 25'd1 : 25'd0);");
    let told = by_fraig(&mutant, "logos_round88_p", "logos_round88", ROUND88_P_LATENCY);
    assert!(told.verdicts.iter().any(|(_, v)| matches!(v, Verdict::Differ { .. })), "half up is not half even");
}

#[test]
fn the_pipelined_scale_is_its_scale_three_clocks_late_and_a_wrong_reciprocal_is_told_apart() {
    if !common::require_tool("yosys") {
        return;
    }
    let v = act_quant_verilog();
    let report = by_fraig(&v, "logos_wide_scale_p", "logos_wide_scale", WIDE_SCALE_P_LATENCY);
    assert_eq!(report.verdicts.len(), 20, "m and e");
    assert!(report.all_equal(), "{:?}", report.verdicts.iter().filter(|(_, v)| *v != Verdict::Equal).collect::<Vec<_>>());
    let start = v.find("module logos_wide_scale_p").expect("the pipelined scale");
    let at = start + v[start..].find("33027").expect("its reciprocal of 127");
    let mutant = format!("{}33026{}", &v[..at], &v[at + 5..]);
    let told = by_fraig(&mutant, "logos_wide_scale_p", "logos_wide_scale", WIDE_SCALE_P_LATENCY);
    assert!(told.verdicts.iter().any(|(_, v)| matches!(v, Verdict::Differ { .. })), "33026 is not 127's reciprocal");
}

#[test]
fn the_pipelined_half_codec_is_its_codec_two_clocks_late_and_a_denormal_rounded_up_is_told_apart() {
    if !common::require_tool("yosys") {
        return;
    }
    let v = fp32_verilog();
    let report = by_fraig(v, "logos_f32_to_f16_p", "logos_f32_to_f16", F32_TO_F16_LATENCY);
    assert_eq!(report.verdicts.len(), 16);
    assert!(report.all_equal(), "{:?}", report.verdicts.iter().filter(|(_, v)| *v != Verdict::Equal).collect::<Vec<_>>());
    let honest = "  wire        dup = (dr > dh) || (dr == dh && n[0]);\n";
    let start = v.find("module logos_f32_to_f16_p").expect("the pipelined codec");
    let at = start + v[start..].find(honest).expect("the mutation point is logos_f32_to_f16_p's denormal rounding");
    let mutant = format!("{}  wire        dup = dr >= dh;\n{}", &v[..at], &v[at + honest.len()..]);
    let told = by_fraig(&mutant, "logos_f32_to_f16_p", "logos_f32_to_f16", F32_TO_F16_LATENCY);
    assert!(told.verdicts.iter().any(|(_, v)| matches!(v, Verdict::Differ { .. })), "a tie rounded up is not to even");
}

const NESTED: &str = "module add(input [1:0] a, input [1:0] b, output [1:0] y);
  assign y = a + b;
endmodule
module mid(input [1:0] a, input [1:0] b, input [1:0] c, output [1:0] y);
  wire [1:0] s;
  add u(.a(a), .b(b), .y(s));
  add v(.a(s), .b(c), .y(y));
endmodule
module top(input [1:0] p, input [1:0] q, input [1:0] r, output [1:0] o);
  genvar k;
  wire [1:0] t [0:1];
  generate for (k = 0; k < 2; k = k + 1) begin : lane
    mid m(.a(p), .b(q), .c(k == 0 ? r : t[0]), .y(t[k]));
  end endgenerate
  assign o = t[1];
endmodule
";

#[test]
fn a_unit_cut_out_is_each_instances_results_as_inputs_and_its_operands_as_outputs() {
    if !common::require_tool("yosys") {
        return;
    }
    let cut = synthesize_cut(NESTED, "top", &[], Undefined::Free, &["add"]).unwrap_or_else(|e| panic!("{e}"));
    let names: Vec<String> = cut.units.iter().map(|u| u.name.clone()).collect();
    assert_eq!(names, ["lane[0].m.u", "lane[0].m.v", "lane[1].m.u", "lane[1].m.v"]);
    for u in &cut.units {
        assert_eq!(u.module, "add");
        assert_eq!(u.operands, [("a".to_string(), 2), ("b".to_string(), 2)], "{}", u.name);
        assert_eq!(u.results, [("y".to_string(), 2)], "{}", u.name);
    }
    let inputs: Vec<&str> = cut.netlist.aig.inputs().iter().filter_map(|p| p.name.as_deref()).collect();
    let outputs: Vec<&str> = cut.netlist.aig.outputs().iter().filter_map(|p| p.name.as_deref()).collect();
    for u in &names {
        for bit in 0..2 {
            assert!(inputs.contains(&format!("{u}.y[{bit}]").as_str()), "{u}'s result a free input: {inputs:?}");
            for port in ["a", "b"] {
                assert!(outputs.contains(&format!("{u}.{port}[{bit}]").as_str()), "{u}'s operand an output: {outputs:?}");
            }
        }
    }
    assert!(outputs.contains(&"o[0]") && inputs.contains(&"p[0]"), "the module's own ports stay");
    let absent = synthesize_cut(NESTED, "top", &[], Undefined::Free, &["sub"]).unwrap_err();
    assert!(absent.to_string().contains("sub"), "a unit no module is named is refused by its name: {absent}");
}

const PARAMETERED: &str = "module u #(parameter K = 1, parameter [3:0] M = 4'd2) (input [3:0] a, output [3:0] y);
  assign y = (K < 0) ? a - M : a + M;
endmodule
module defaults(input [3:0] a, output [3:0] y);
  u i(.a(a), .y(y));
endmodule
module one(input [3:0] a, output [3:0] y);
  u #(.K(1)) i(.a(a), .y(y));
endmodule
module one_spelled(input [3:0] a, output [3:0] y);
  u #(.K(32'sd1)) i(.a(a), .y(y));
endmodule
module minus(input [3:0] a, output [3:0] y);
  u #(.K(-1)) i(.a(a), .y(y));
endmodule
module all_ones(input [3:0] a, output [3:0] y);
  u #(.K(32'hffffffff)) i(.a(a), .y(y));
endmodule
";

#[test]
fn a_units_function_is_its_module_and_the_values_it_gives_so_one_module_at_two_values_is_never_one_function() {
    if !common::require_tool("yosys") {
        return;
    }
    let cut = |top: &str| synthesize_cut(PARAMETERED, top, &[], Undefined::Free, &["u"]).unwrap_or_else(|e| panic!("{top}: {e}"));
    let none = |_: &str| 0;
    let sides: Vec<(&str, _, Vec<units::Unit>)> = ["defaults", "one", "one_spelled", "minus", "all_ones"]
        .into_iter()
        .map(|top| {
            let c = cut(top);
            let u = c.units(&none, &[]);
            (top, c, u)
        })
        .collect();
    let functions: Vec<&str> = sides.iter().map(|(_, _, u)| u[0].function.as_str()).collect();
    assert_eq!(
        functions,
        ["u", "u #(K=32'sd1)", "u #(K=32'sd1)", "u #(K=-32'sd1)", "u #(K=32'd4294967295)"],
        "the module and each value the instance gives, width and sign kept; a default is the module's own"
    );
    // Every function either side names paired with itself, as a caller pairing units by name would.
    let decide = |l: usize, r: usize| {
        let ((_, lc, lu), (_, rc, ru)) = (&sides[l], &sides[r]);
        let pairs: Vec<(String, String)> = lu.iter().chain(ru.iter()).map(|u| (u.function.clone(), u.function.clone())).collect();
        let (ls, rs) = (units::Side { aig: &lc.netlist.aig, latency: 0, units: lu }, units::Side { aig: &rc.netlist.aig, latency: 0, units: ru });
        let report = units::decide(&ls, &rs, &pairs, &fraig::Options::default()).unwrap();
        assert!(report.check(&ls, &rs, &pairs));
        report
    };
    let same = decide(1, 2);
    assert!(same.verdicts.iter().all(|(_, v)| *v == Verdict::Equal), "1 and 32'sd1 are one value, one function: {:?}", same.verdicts);
    let signed_apart = decide(3, 4);
    assert!(signed_apart.verdicts.iter().all(|(_, v)| *v != Verdict::Equal), "-1 and 32'hffffffff are one bit pattern, two functions: {:?}", signed_apart.verdicts);
    let valued_apart = decide(1, 3);
    assert!(valued_apart.verdicts.iter().all(|(_, v)| *v != Verdict::Equal), "{:?}", valued_apart.verdicts);
    let defaulted = decide(0, 1);
    assert!(defaulted.verdicts.iter().all(|(_, v)| *v != Verdict::Equal), "a default is never read as a value it may not be: {:?}", defaulted.verdicts);
    let by_function = |f: &str| if f == "u #(K=32'sd1)" { 2 } else { 0 };
    let latencies: Vec<usize> = sides.iter().map(|(_, c, _)| c.units(&by_function, &[])[0].latency).collect();
    assert_eq!(latencies, [0, 2, 2, 0, 0], "a unit's latency is named by its function, parameters and all");
}

const NARROWED: &str = "module wide #(parameter W = 8) (input [W-1:0] a, output [W-1:0] y);
  assign y = ~a;
endmodule
module narrow(input [3:0] a, output [3:0] y);
  wide #(.W(4)) i(.a(a), .y(y));
endmodule
";

#[test]
fn a_unit_is_every_bit_the_cut_exposes_for_its_instance_even_past_the_bits_it_connects() {
    if !common::require_tool("yosys") {
        return;
    }
    let cut = synthesize_cut(NARROWED, "narrow", &[], Undefined::Free, &["wide"]).unwrap_or_else(|e| panic!("{e}"));
    let u = &cut.units(&|_| 0, &[])[0];
    let aig = &cut.netlist.aig;
    let exposed_in: Vec<&str> = aig.inputs().iter().filter_map(|p| p.name.as_deref()).filter(|n| n.starts_with("i.")).collect();
    let exposed_out: Vec<&str> = aig.outputs().iter().filter_map(|p| p.name.as_deref()).filter(|n| n.starts_with("i.")).collect();
    assert!(!exposed_in.is_empty() && !exposed_out.is_empty());
    for n in &exposed_in {
        assert!(u.results.iter().any(|r| r == n), "{n}: an exposed result bit is the unit's: {:?}", u.results);
    }
    for n in &exposed_out {
        assert!(u.operands.iter().any(|o| o == n), "{n}: an exposed operand bit is the unit's: {:?}", u.operands);
    }
    assert!(u.results.iter().all(|r| aig.input_index(r).is_some()) && u.operands.iter().all(|o| aig.output_index(o).is_some()), "and nothing the cut did not expose");
}

/// A circuit to decide over its units: its module and parameters, and the clock its outputs are read at.
#[derive(Clone, Copy)]
struct Over<'a> {
    top: &'a str,
    params: &'a [(&'a str, &'a str)],
    latency: usize,
}

/// A unit each side is built of, proven apart: the left's module and the clocks its results follow its operands by,
/// then the right's.
type UnitPair<'a> = (&'a str, usize, &'a str, usize);

/// `left` decided against `right` over the units of `pairs` as functions nothing is known of but that each left unit
/// computes its right counterpart's function, every round's derivation and identification checked.
fn over_units(verilog: &str, left: Over, right: Over, pairs: &[UnitPair]) -> units::Report {
    let cut = |side: Over, modules: Vec<&str>| {
        let mut modules = modules;
        modules.sort_unstable();
        modules.dedup();
        synthesize_cut(verilog, side.top, side.params, Undefined::Free, &modules).unwrap_or_else(|e| panic!("{}: {e}", side.top))
    };
    let (lc, rc) = (cut(left, pairs.iter().map(|p| p.0).collect()), cut(right, pairs.iter().map(|p| p.2).collect()));
    let left_late = |m: &str| pairs.iter().find(|p| p.0 == m).map_or(0, |p| p.1);
    let right_late = |m: &str| pairs.iter().find(|p| p.2 == m).map_or(0, |p| p.3);
    let (lu, ru) = (lc.units(&left_late, &["clk"]), rc.units(&right_late, &["clk"]));
    let functions: Vec<(String, String)> = pairs.iter().map(|p| (p.0.to_string(), p.2.to_string())).collect();
    let ls = units::Side { aig: &lc.netlist.aig, latency: left.latency, units: &lu };
    let rs = units::Side { aig: &rc.netlist.aig, latency: right.latency, units: &ru };
    let report = units::decide(&ls, &rs, &functions, &fraig::Options::default()).unwrap();
    assert!(report.check(&ls, &rs, &functions), "{}: every round's derivation and identification check", left.top);
    report
}

/// `piped` (its adders `logos_fadd_p`) decided against `reference` (its adders `logos_fadd`) at `latency` over the
/// adders, each proven apart (the pipelined add test above).
fn over_adders(verilog: &str, piped: &str, reference: &str, latency: usize) -> units::Report {
    over_units(
        verilog,
        Over { top: piped, params: &[], latency },
        Over { top: reference, params: &[], latency: 0 },
        &[("logos_fadd_p", FADD_LATENCY, "logos_fadd", 0)],
    )
}

/// The pipelined f32 units, each against the combinational one it is proven equal to.
const F32_UNITS: [UnitPair; 2] = [("logos_fmul_p", FMUL_LATENCY, "logos_fmul", 0), ("logos_fadd_p", FADD_LATENCY, "logos_fadd", 0)];

/// The verdicts that are not equalities, for a failure's message.
fn unequal(report: &units::Report) -> Vec<&(String, Verdict)> {
    report.verdicts.iter().filter(|(_, v)| *v != Verdict::Equal).collect()
}

/// `piped` decided against `reference` at `latency` over the pipelined f32 units and `more`.
fn over_f32_units(verilog: &str, piped: &str, reference: &str, latency: usize, more: &[UnitPair]) -> units::Report {
    let pairs: Vec<UnitPair> = F32_UNITS.iter().chain(more).copied().collect();
    over_units(verilog, Over { top: piped, params: &[], latency }, Over { top: reference, params: &[], latency: 0 }, &pairs)
}

/// `verilog` with the one `honest` text inside `module`'s body replaced by `mutant`.
fn mutated(verilog: &str, module: &str, honest: &str, mutant: &str) -> String {
    let head = format!("module {module}");
    let start = verilog
        .match_indices(&head)
        .map(|(i, _)| i)
        .find(|&i| verilog[i + head.len()..].starts_with([' ', '(', '#']))
        .unwrap_or_else(|| panic!("no module {module}"));
    let end = start + verilog[start..].find("endmodule").expect("its end");
    let body = &verilog[start..end];
    assert_eq!(body.matches(honest).count(), 1, "the mutation point `{honest}` is one place in {module}");
    let at = start + body.find(honest).unwrap();
    format!("{}{mutant}{}", &verilog[..at], &verilog[at + honest.len()..])
}

#[test]
fn the_pipelined_integer_to_float_is_its_conversion_two_clocks_late_and_a_wrong_bias_is_told_apart() {
    if !common::require_tool("yosys") {
        return;
    }
    let v = vector_verilog();
    let report = by_fraig(&v, "logos_i2f_p", "logos_i2f", I2F_LATENCY);
    assert_eq!(report.verdicts.len(), 32);
    assert!(report.all_equal(), "{:?}", report.verdicts.iter().filter(|(_, v)| *v != Verdict::Equal).collect::<Vec<_>>());
    let mutant = mutated(&v, "logos_i2f_p", "8'd127 + lead[7:0]", "8'd126 + lead[7:0]");
    let told = by_fraig(&mutant, "logos_i2f_p", "logos_i2f", I2F_LATENCY);
    assert!(told.verdicts.iter().any(|(_, v)| matches!(v, Verdict::Differ { .. })), "a bias of 126 is not the f32's");
}

#[test]
fn the_pipelined_exponential_is_its_exponential_over_its_proven_units_and_a_term_a_clock_short_is_not() {
    if !common::require_tool("yosys") {
        return;
    }
    let v = vector_verilog();
    let i2f = [("logos_i2f_p", I2F_LATENCY, "logos_i2f", 0)];
    let report = over_f32_units(&v, "logos_exp_p", "logos_exp", EXP_LATENCY, &i2f);
    assert_eq!(report.verdicts.len(), 32);
    assert!(unequal(&report).is_empty(), "{:?}", unequal(&report));
    let identified: usize = report.rounds.iter().map(|r| r.substituted.len()).sum();
    assert_eq!(identified, 13, "its seven multiplies, five adds and one conversion each identified with its counterpart");
    let honest = format!(".D({FADD_LATENCY})) dl(.clk(clk), .d(l), .q(l_d));");
    let short = format!(".D({})) dl(.clk(clk), .d(l), .q(l_d));", FADD_LATENCY - 1);
    let told = over_f32_units(&mutated(&v, "logos_exp_p", &honest, &short), "logos_exp_p", "logos_exp", EXP_LATENCY, &i2f);
    assert!(!unequal(&told).is_empty(), "the reduction's low term a clock early is another exponential");
}

#[test]
fn the_pipelined_reciprocal_is_its_reciprocal_over_its_proven_units_and_a_seed_a_clock_short_is_not() {
    if !common::require_tool("yosys") {
        return;
    }
    let v = vector_verilog();
    let report = over_f32_units(&v, "logos_recip_p", "logos_recip", RECIP_LATENCY, &[]);
    assert_eq!(report.verdicts.len(), 32);
    assert!(unequal(&report).is_empty(), "{:?}", unequal(&report));
    let identified: usize = report.rounds.iter().map(|r| r.substituted.len()).sum();
    assert_eq!(identified, 6, "its two Newton steps' multiplies, subtraction and multiply each identified");
    let step = FMUL_LATENCY + FADD_LATENCY;
    let honest = format!(".D({step})) ds(.clk(clk), .d(seed), .q(seed_d));");
    let short = format!(".D({})) ds(.clk(clk), .d(seed), .q(seed_d));", step - 1);
    let told = over_f32_units(&mutated(&v, "logos_recip_p", &honest, &short), "logos_recip_p", "logos_recip", RECIP_LATENCY, &[]);
    assert!(!unequal(&told).is_empty(), "the seed a clock early multiplies the next operand's correction");
}

#[test]
fn the_pipelined_reciprocal_square_root_is_its_function_over_its_proven_units_and_an_estimate_a_clock_short_is_not() {
    if !common::require_tool("yosys") {
        return;
    }
    let v = vector_verilog();
    let report = over_f32_units(&v, "logos_rsqrt_p", "logos_rsqrt", RSQRT_LATENCY, &[]);
    assert_eq!(report.verdicts.len(), 32);
    assert!(unequal(&report).is_empty(), "{:?}", unequal(&report));
    let identified: usize = report.rounds.iter().map(|r| r.substituted.len()).sum();
    assert_eq!(identified, 10, "its two Newton steps' four multiplies and subtraction each identified");
    let honest = format!(".D({FMUL_LATENCY})) dya(.clk(clk), .d(y1), .q(y1_a));");
    let short = format!(".D({})) dya(.clk(clk), .d(y1), .q(y1_a));", FMUL_LATENCY - 1);
    let told = over_f32_units(&mutated(&v, "logos_rsqrt_p", &honest, &short), "logos_rsqrt_p", "logos_rsqrt", RSQRT_LATENCY, &[]);
    assert!(!unequal(&told).is_empty(), "the first estimate a clock early squares two operands' estimates");
}

#[test]
fn the_pipelined_logarithm_is_its_logarithm_over_its_proven_units_and_a_table_entry_a_clock_late_is_not() {
    if !common::require_tool("yosys") {
        return;
    }
    let v = vector_verilog();
    let report = over_f32_units(&v, "logos_ln_p", "logos_ln", LN_LATENCY, &[]);
    assert_eq!(report.verdicts.len(), 32);
    assert!(unequal(&report).is_empty(), "{:?}", unequal(&report));
    let identified: usize = report.rounds.iter().map(|r| r.substituted.len()).sum();
    assert_eq!(identified, 14, "its seven multiplies and seven adds each identified");
    let poly = 4 * FMUL_LATENCY + 4 * FADD_LATENCY;
    let honest = format!(".D({poly})) dl(.clk(clk), .d(lnc), .q(lnc_d));");
    let late = format!(".D({})) dl(.clk(clk), .d(lnc), .q(lnc_d));", poly + 1);
    let told = over_f32_units(&mutated(&v, "logos_ln_p", &honest, &late), "logos_ln_p", "logos_ln", LN_LATENCY, &[]);
    assert!(!unequal(&told).is_empty(), "the table's logarithm a clock late is the operand before's");
}

/// The lane function unit's cores, each pipelined one against the combinational one it is proven equal to (the
/// tests above, and the pipelined f32 units' and half codec's).
const LANE_CORES: [UnitPair; 5] = [
    ("logos_exp_p", EXP_LATENCY, "logos_exp", 0),
    ("logos_recip_p", RECIP_LATENCY, "logos_recip", 0),
    ("logos_rsqrt_p", RSQRT_LATENCY, "logos_rsqrt", 0),
    ("logos_ln_p", LN_LATENCY, "logos_ln", 0),
    ("logos_f32_to_f16_p", F32_TO_F16_LATENCY, "logos_f32_to_f16", 0),
];

#[test]
fn the_pipelined_lane_function_unit_is_the_lane_function_unit_over_its_proven_cores_and_a_logarithm_a_clock_late_is_not() {
    if !common::require_tool("yosys") {
        return;
    }
    let v = simd_p_verilog();
    let at_once: &[(&str, &str)] = &[("STAGED", "0")];
    let decide = |v: &str| {
        let pairs: Vec<UnitPair> = F32_UNITS.iter().chain(&LANE_CORES).copied().collect();
        over_units(v, Over { top: "logos_lane_func_p", params: &[], latency: LANE_FUNC_P_LATENCY }, Over { top: "logos_lane_func", params: at_once, latency: 0 }, &pairs)
    };
    let report = decide(&v);
    assert_eq!(report.verdicts.len(), 32);
    assert!(unequal(&report).is_empty(), "{:?}", unequal(&report));
    let identified: usize = report.rounds.iter().map(|r| r.substituted.len()).sum();
    assert_eq!(identified, 10, "its five cores, four adds and one multiply each identified with its counterpart");
    let honest = format!(".D({FMUL_LATENCY})) dlu(.clk(clk), .d(lu_m), .q(lu_p));");
    let late = format!(".D({})) dlu(.clk(clk), .d(lu_m), .q(lu_p));", FMUL_LATENCY + 1);
    let told = decide(&mutated(&v, "logos_lane_func_p", &honest, &late));
    assert!(!unequal(&told).is_empty(), "ln1p's logarithm a clock late is the operand before's");
}

#[test]
fn the_staged_lane_function_unit_is_the_unit_at_once_its_stages_later_over_its_cores_and_a_sum_a_step_early_is_not() {
    if !common::require_tool("yosys") {
        return;
    }
    let v = simd_verilog();
    let cores: Vec<UnitPair> = ["logos_fmul", "logos_fadd", "logos_exp", "logos_recip", "logos_rsqrt", "logos_ln", "logos_f32_to_f16"]
        .iter()
        .map(|&m| (m, 0, m, 0))
        .collect();
    let decide = |v: &str| {
        over_units(
            v,
            Over { top: "logos_lane_func", params: &[("STAGED", "1")], latency: LANE_FUNC_STAGES },
            Over { top: "logos_lane_func", params: &[("STAGED", "0")], latency: 0 },
            &cores,
        )
    };
    let report = decide(&v);
    assert_eq!(report.verdicts.len(), 32);
    assert!(unequal(&report).is_empty(), "{:?}", unequal(&report));
    let identified: usize = report.rounds.iter().map(|r| r.substituted.len()).sum();
    assert_eq!(identified, 10, "its five cores, four adds and one multiply each identified with its counterpart");
    let told = decide(&mutated(&v, "logos_lane_func", "rt3 <= rt2;", "rt3 <= t;"));
    assert!(!unequal(&told).is_empty(), "ln1p's operand a step early is the next operand's");
}

#[test]
fn the_pipelined_sixteen_lane_tree_is_its_tree_of_adds_sixteen_clocks_late_over_its_proven_adders() {
    if !common::require_tool("yosys") {
        return;
    }
    let v = fp32_verilog();
    let report = over_adders(v, "logos_tree16_p", "logos_tree16", TREE16_LATENCY);
    assert_eq!(report.verdicts.len(), 32);
    assert!(report.verdicts.iter().all(|(_, v)| *v == Verdict::Equal), "{:?}", report.verdicts.iter().filter(|(_, v)| *v != Verdict::Equal).collect::<Vec<_>>());
    let identified: usize = report.rounds.iter().map(|r| r.substituted.len()).sum();
    assert_eq!(identified, 15, "each of the fifteen adders identified with its counterpart, none re-proven");
    let honest = ".b(l1[2*i + 1]), .y(l2[i]));";
    let start = v.find("module logos_tree16_p").expect("the pipelined tree");
    let at = start + v[start..].find(honest).expect("a level-two adder of the pipelined tree");
    let mutant = format!("{}.b(l1[2*i]), .y(l2[i]));{}", &v[..at], &v[at + honest.len()..]);
    let told = over_adders(&mutant, "logos_tree16_p", "logos_tree16", TREE16_LATENCY);
    assert!(told.verdicts.iter().all(|(_, v)| *v != Verdict::Equal), "a lane added twice is no tree of the sixteen: {:?}", told.verdicts.iter().take(2).collect::<Vec<_>>());
}

fn member(name: &str) -> Semantics {
    family().into_iter().find(|s| s.name() == name).unwrap_or_else(|| panic!("no member {name}"))
}

/// The engine's combinational unit and its pipelined latency for `op`.
fn engine_unit(op: HardOp) -> (&'static str, usize) {
    match op {
        HardOp::Multiply => ("logos_fmul", FMUL_LATENCY),
        HardOp::Add => ("logos_fadd", FADD_LATENCY),
    }
}

/// `wrapper` (top `hard`) with a stand-in block computing `semantics` in the device's block's place, `full` deep fully
/// registered and `least` with its output register alone, against the engine's combinational unit for `op` at the
/// pipelined unit's latency, the derivation checked.
fn against_engine(op: HardOp, wrapper: &str, semantics: &Semantics, full: usize, least: usize) -> fraig::Report {
    let (reference, latency) = engine_unit(op);
    let v = format!("{wrapper}\n{}", hard_model_verilog(semantics, semantics, Depths { full: [full; 3], least: [least; 2] }));
    let report = latency_equivalent_by_fraig(&aig_of(&v, "hard", &[]), &aig_of(fp32_verilog(), reference, &[]), latency, &fraig::Options::default()).unwrap();
    assert!(report.check(), "{}: the derivation proves every equality and every witness replays", semantics.name());
    report
}

/// The wrapper `fp_hard` writes for a block computing `semantics`, `full` and `least` deep, against the engine's unit.
fn hard_against_engine(op: HardOp, semantics: &Semantics, full: usize, least: usize) -> fraig::Report {
    let (_, latency) = engine_unit(op);
    let wrapper = hard_verilog(op, "hard", &Block { semantics: *semantics, full, least }, latency).unwrap();
    against_engine(op, &wrapper, semantics, full, least)
}

#[test]
fn the_hard_multiply_is_the_engines_multiply_on_every_block_it_accepts_padded_or_least_registered() {
    if !common::require_tool("yosys") {
        return;
    }
    for (name, full, least) in [
        ("din-flush.dout-flush.tiny-after-denormals.sign-kept.nan-7fc00000", 3, 1),
        ("din-flush.dout-flush.tiny-after-denormals.sign-kept.nan-7fc00000", 4, 1),
        ("din-keep.dout-keep.nan-prop-ffc00000", 2, 1),
        ("din-flush.dout-keep.nan-ffffffff", 3, 2),
        ("din-keep.dout-flush.tiny-after-denormals.sign-kept.nan-prop-7fc00000", 5, 3),
    ] {
        let report = hard_against_engine(HardOp::Multiply, &member(name), full, least);
        assert!(report.all_equal(), "{name} {full}/{least}: {:?}", report.verdicts.iter().filter(|(_, v)| *v != Verdict::Equal).take(4).collect::<Vec<_>>());
    }
}

#[test]
fn the_hard_add_is_the_engines_add_on_every_block_it_accepts_padded_or_least_registered() {
    if !common::require_tool("yosys") {
        return;
    }
    for (name, full, least) in [
        ("din-flush.dout-flush.tiny-after-denormals.sign-kept.nan-7fc00000", 4, 1),
        ("din-keep.dout-keep.nan-prop-ffc00000", 6, 1),
        ("din-keep.dout-keep.nan-7fffffff", 3, 1),
    ] {
        let report = hard_against_engine(HardOp::Add, &member(name), full, least);
        assert!(report.all_equal(), "{name} {full}/{least}: {:?}", report.verdicts.iter().filter(|(_, v)| *v != Verdict::Equal).take(4).collect::<Vec<_>>());
    }
}

#[test]
fn a_hard_wrapper_short_of_a_correction_or_a_register_is_told_apart_and_a_block_it_cannot_correct_is_refused() {
    if !common::require_tool("yosys") {
        return;
    }
    let ieee = member("din-keep.dout-keep.nan-prop-ffc00000");
    let wrapper = hard_verilog(HardOp::Multiply, "hard", &Block { semantics: ieee, full: 2, least: 1 }, FMUL_LATENCY).unwrap();
    let flush = "nan ? 32'h7fc0_0000 : (h[30:23] == 8'd0) ? {h[31], 31'd0} : h;";
    assert_eq!(wrapper.matches(flush).count(), 1, "the results' correction");
    let kept = wrapper.replace(flush, "nan ? 32'h7fc0_0000 : h;");
    let told = against_engine(HardOp::Multiply, &kept, &ieee, 2, 1);
    assert!(told.verdicts.iter().any(|(_, v)| matches!(v, Verdict::Differ { .. })), "a denormal product kept is not the engine's");
    let decided = "wire [32:0] side_c = {da || db, ";
    assert_eq!(wrapper.matches(decided).count(), 1, "the denormal operand's decision");
    let read = wrapper.replace(decided, "wire [32:0] side_c = {1'b0, ");
    let told = against_engine(HardOp::Multiply, &read, &ieee, 2, 1);
    assert!(told.verdicts.iter().any(|(_, v)| matches!(v, Verdict::Differ { .. })), "a denormal operand read is not the engine's");
    let short = against_engine(HardOp::Multiply, &wrapper, &ieee, 3, 1);
    assert!(short.verdicts.iter().any(|(_, v)| matches!(v, Verdict::Differ { .. })), "a block a clock deeper than the wrapper was written for is a clock late");
    let before = member("din-flush.dout-flush.tiny-before.sign-kept.nan-7fc00000");
    assert!(matches!(hard_verilog(HardOp::Multiply, "hard", &Block { semantics: before, full: 3, least: 1 }, FMUL_LATENCY), Err(HardError::Semantics { .. })));
}

#[test]
fn the_hard_wrapper_stepping_at_an_enable_held_high_is_the_engines_unit() {
    if !common::require_tool("yosys") {
        return;
    }
    let ieee = member("din-keep.dout-keep.nan-prop-ffc00000");
    let ce = hard_verilog_ce(HardOp::Add, "hard_ce", &Block { semantics: ieee, full: 6, least: 2 }, FADD_LATENCY).unwrap();
    let top = format!("{ce}\nmodule hard(input clk, input [31:0] a, input [31:0] b, output [31:0] y);\n  hard_ce u(.clk(clk), .step_en(1'b1), .a(a), .b(b), .y(y));\nendmodule\n");
    let report = against_engine(HardOp::Add, &top, &ieee, 6, 2);
    assert!(report.all_equal(), "{:?}", report.verdicts.iter().filter(|(_, v)| *v != Verdict::Equal).take(4).collect::<Vec<_>>());
}

/// The bits `prefix[0]` to `prefix[n - 1]`.
fn bits(prefix: &str, n: usize) -> Vec<String> {
    (0..n).map(|i| format!("{prefix}[{i}]")).collect()
}

/// A bit name's index: `in_beat[17]` is 17.
fn index(bit: &str) -> usize {
    bit[bit.find('[').unwrap() + 1..bit.len() - 1].parse().unwrap()
}

/// The read-out at two sixteens a clock (`logos_gdn_read_w`, W = 2) against the one at sixteen (`logos_gdn_read_p`),
/// rows of 32 elements, over their proven multiplies, adds and trees, each side offered rows as `offer` gives it per
/// side from the clock after a reset held one clock: the reset clears every valid stage of the read-outs' delay lines.
fn read_outs(verilog: &str, offer: [&dyn Fn(usize) -> bool; 2]) -> (streams::Report, bool) {
    let cut = ["logos_fmul_p", "logos_fadd_p", "logos_tree16_p"];
    let wide = synthesize_cut(verilog, "logos_gdn_read_w", &[("N", "32"), ("W", "2")], Undefined::Free, &cut).unwrap_or_else(|e| panic!("{e}"));
    let one = synthesize_cut(verilog, "logos_gdn_read_p", &[("N", "32")], Undefined::Free, &cut).unwrap_or_else(|e| panic!("{e}"));
    let late = |m: &str| match m {
        "logos_fmul_p" => FMUL_LATENCY,
        "logos_fadd_p" => FADD_LATENCY,
        _ => TREE16_LATENCY,
    };
    let (wu, ou) = (wide.units(&late, &["clk"]), one.units(&late, &["clk"]));
    let functions: Vec<(String, String)> = cut.iter().map(|m| (m.to_string(), m.to_string())).collect();
    // A row is two words of sixteen f32: one beat of the wide read-out, two of the other; its carried value comes with
    // its first beat.
    let wide_token = |j: usize, bit: &str| -> Binding {
        let i = index(bit);
        if bit.starts_with("in_beat") {
            Binding::Shared(format!("w{}[{}]", 2 * j + i / 512, i % 512))
        } else {
            Binding::Shared(format!("c{j}[{i}]"))
        }
    };
    let one_token = |j: usize, bit: &str| -> Binding {
        let i = index(bit);
        if bit.starts_with("in_beat") {
            Binding::Shared(format!("w{j}[{i}]"))
        } else if j % 2 == 0 {
            Binding::Shared(format!("c{}[{i}]", j / 2))
        } else {
            Binding::Fresh
        }
    };
    let ports = |beat: usize, token: &'static dyn Fn(usize, &str) -> Binding| Ports {
        in_valid: "in_valid".into(),
        in_ready: "in_ready".into(),
        in_data: bits("in_beat", beat).into_iter().chain(bits("in_carried", 32)).collect(),
        out_valid: "out_valid".into(),
        out_ready: None,
        out_data: bits("out_read", 32),
        token,
    };
    let wide_token: &'static dyn Fn(usize, &str) -> Binding = Box::leak(Box::new(wide_token));
    let one_token: &'static dyn Fn(usize, &str) -> Binding = Box::leak(Box::new(one_token));
    let l = StreamSide { aig: &wide.netlist.aig, units: &wu, ports: ports(1024, wide_token) };
    let r = StreamSide { aig: &one.netlist.aig, units: &ou, ports: ports(512, one_token) };
    const RESET: usize = 1;
    let held = |name: &str, k: usize| if name == "rst" { Binding::Const(k < RESET) } else { Binding::Shared(name.to_string()) };
    let (lo, ro) = (|k: usize| k >= RESET && offer[0](k), |k: usize| k >= RESET && offer[1](k));
    let after_reset = |k: usize| k >= RESET;
    let le = Environment { frames: RESET + 72, offer: &lo, accept: &after_reset, other: &held };
    let re = Environment { frames: RESET + 72, offer: &ro, accept: &after_reset, other: &held };
    let report = streams::decide(&l, &le, &r, &re, &functions, &fraig::Options::default()).unwrap_or_else(|e| panic!("{e}"));
    let tokens = report.gave[0].min(report.gave[1]);
    assert!(tokens >= 8, "a group of the wide read-out's rows, two of the other's: {:?} given", report.gave);
    assert_eq!(report.verdicts.len(), 32 * tokens);
    let checks = report.check(&l, &le, &r, &re, &functions);
    (report, checks)
}

#[test]
fn the_recurrence_read_out_at_two_sixteens_a_clock_gives_the_one_sixteen_a_clock_read_outs_rows_token_for_token() {
    if !common::require_tool("yosys") {
        return;
    }
    let v = gdn_stream_verilog();
    let every = |_: usize| true;
    let (report, checks) = read_outs(&v, [&every, &every]);
    assert!(report.all_equal(), "{:?}", report.verdicts.iter().filter(|(_, v)| *v != Verdict::Equal).take(4).collect::<Vec<_>>());
    assert!(checks, "every schedule's and every round's derivation checks");
}

#[test]
fn the_wide_read_out_offered_every_clock_gives_the_rows_the_other_gives_offered_three_clocks_of_four() {
    if !common::require_tool("yosys") {
        return;
    }
    let v = gdn_stream_verilog();
    let (every, gaps) = (|_: usize| true, |k: usize| k % 4 != 1);
    let (report, checks) = read_outs(&v, [&every, &gaps]);
    assert!(report.schedules[1].took.len() < report.schedules[0].took.len(), "the stalled side takes fewer beats: {:?}", report.schedules.iter().map(|s| s.took.len()).collect::<Vec<_>>());
    assert!(report.all_equal(), "{:?}", report.verdicts.iter().filter(|(_, v)| *v != Verdict::Equal).take(4).collect::<Vec<_>>());
    assert!(checks);
}

#[test]
fn a_wide_read_out_adding_its_carried_value_a_clock_early_is_not_the_stream() {
    if !common::require_tool("yosys") {
        return;
    }
    let v = gdn_stream_verilog();
    let honest = ".b(car_d[TREE]), .y(with_carried)";
    assert_eq!(v.matches(honest).count(), 1, "the mutation point is logos_gdn_read_w's carried value");
    let mutant = v.replace(honest, ".b(car_d[TREE - 1]), .y(with_carried)");
    let every = |_: usize| true;
    let (report, checks) = read_outs(&mutant, [&every, &every]);
    assert!(!report.all_equal(), "a row read with the next row's carried value");
    assert!(report.verdicts.iter().all(|(_, v)| !matches!(v, Verdict::Differ { .. })), "over units nothing is known of, no output is called different");
    assert!(checks);
}

/// A wrapper planned for another arithmetic computes it: the card's multiply wrapped for itself, fully registered or with
/// its output register alone, is the card's member at the engine's multiply latency; a block keeping denormal operands
/// wrapped for the card's member decides them beside it, the card's NaN signed by the operands; and one wrapped for an
/// ARM core's flushing twin decides them beside it with a NaN operand's payload, the derivation checked.
#[test]
fn a_hard_wrapper_planned_for_a_target_computes_the_targets_arithmetic() {
    if !common::require_tool("yosys") {
        return;
    }
    let card = member("din-flush.dout-flush.tiny-after-unbounded.sign-kept.nan-xor-7fc00000");
    let keeps_in = member("din-keep.dout-flush.tiny-after-unbounded.sign-kept.nan-xor-7fc00000");
    let arm = member("din-flush.dout-flush.tiny-after-denormals.sign-kept.nan-prop-7fc00000");
    let arm_keeps_in = member("din-keep.dout-flush.tiny-after-denormals.sign-kept.nan-prop-7fc00000");
    for (target, block, full, least) in [(card, card, 3, 1), (card, card, 4, 1), (card, keeps_in, 3, 1), (arm, arm_keeps_in, 3, 2)] {
        let reference = aig_of(&fmul_verilog("reference", &target), "reference", &[]);
        let wrapper = hard_verilog_for(HardOp::Multiply, "hard", &Block { semantics: block, full, least }, FMUL_LATENCY, &target).unwrap();
        let v = format!("{wrapper}\n{}", hard_model_verilog(&block, &block, Depths { full: [full; 3], least: [least; 2] }));
        let report = latency_equivalent_by_fraig(&aig_of(&v, "hard", &[]), &reference, FMUL_LATENCY, &fraig::Options::default()).unwrap();
        assert!(report.check(), "{} {full}/{least}: the derivation proves every equality and every witness replays", block.name());
        assert!(report.all_equal(), "{} {full}/{least}: {:?}", block.name(), report.verdicts.iter().filter(|(_, v)| *v != Verdict::Equal).take(4).collect::<Vec<_>>());
    }
}
