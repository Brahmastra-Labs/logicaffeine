//! Pipelines cut by a flow, end to end: Verilog through yosys into the timing graph (`codegen_sva::timing_graph`),
//! the fewest-register cut at a period and latency (`codegen_sva::autopipe`, the dual solved by
//! `logicaffeine_proof::min_cost_flow`), written back out as Verilog, and proven equal to the original that many
//! clocks late (`logicaffeine_proof::seq_equiv`, decided by incremental SAT sweeping, `fraig`), the derivation of every
//! equality checked. The timing graph reads each undefined bit of the source as 0, so the pipeline is proven against
//! the source read so.

mod common;

use logicaffeine_compile::codegen_sva::autopipe::{check, emit, min_latency, pipeline, unpipelined, Cuts, EmitError, PipeError};
use logicaffeine_compile::codegen_sva::netlist::{self, Undefined};
use logicaffeine_compile::codegen_sva::timing_graph::{synthesize, Delays, TimingGraph};
use logicaffeine_compile::spatial::decode_engine::rtl::fp32::fp32_verilog;
use logicaffeine_proof::fraig::Options;
use logicaffeine_proof::seq_equiv::latency_equivalent_by_fraig;
use logicaffeine_proof::sweep::Verdict;

const LEVEL: i64 = Delays::LEVELS.lut;

fn graph(verilog: &str, top: &str) -> TimingGraph {
    synthesize(verilog, top, &[], &[], &Delays::LEVELS).unwrap_or_else(|e| panic!("{top}: {e}"))
}

/// That `piped` (module `piped_top`) is `original` (module `top`), its undefined bits read as 0, `latency` clocks late,
/// output by output, certified.
fn proven(original: &str, top: &str, piped: &str, piped_top: &str, latency: i64) {
    let left = netlist::synthesize(piped, piped_top, &[]).unwrap_or_else(|e| panic!("{piped_top}: {e}\n{piped}")).aig;
    let right = netlist::synthesize_reading(original, top, &[], Undefined::Zero).unwrap_or_else(|e| panic!("{top}: {e}")).aig;
    let report = latency_equivalent_by_fraig(&left, &right, latency as usize, &Options::default()).unwrap();
    assert!(report.all_equal(), "{:?}", report.verdicts.iter().filter(|(_, v)| *v != Verdict::Equal).collect::<Vec<_>>());
    assert!(report.check(), "the derivation proves every equality and every witness replays");
}

const CHAIN: &str = "module chain(input [7:0] a, input [7:0] b, input [7:0] c, input [7:0] d, output [7:0] y);
  wire [7:0] s1 = a + b;
  wire [7:0] s2 = s1 ^ c;
  wire [7:0] s3 = s2 + d;
  wire [7:0] s4 = s3 ^ {s1[3:0], s2[7:4]};
  assign y = s4 + a;
endmodule
";

#[test]
fn a_chain_of_named_stages_is_cut_on_its_named_nets_and_is_its_latency_late() {
    if !common::require_tool("yosys") {
        return;
    }
    let g = graph(CHAIN, "chain");
    let depth = g.depth().unwrap().depth;
    let period = 2 * LEVEL;
    let latency = min_latency(&g, period, Cuts::Named).unwrap();
    assert!(latency >= 1 && latency * period >= depth - period, "depth {depth}, latency {latency}");
    let plan = pipeline(&g, period, latency, Cuts::Named).unwrap();
    assert!(check(&g, &plan));
    for (net, _, _) in plan.cuts(&g) {
        assert!(["a", "b", "c", "d", "s1", "s2", "s3", "s4", "y"].contains(&net.as_str()), "a cut on a named net: {net}");
    }
    let v = emit(&g, &plan, "chain_p", None).unwrap();
    proven(CHAIN, "chain", &v, "chain_p", latency);
    let later = pipeline(&g, period, latency + 1, Cuts::Named).unwrap();
    proven(CHAIN, "chain", &emit(&g, &later, "chain_q", None).unwrap(), "chain_q", latency + 1);
    assert_eq!(pipeline(&g, period, latency - 1, Cuts::Named), Err(PipeError::LatencyTooShort { minimum: latency }));
}

#[test]
fn the_combinational_multiply_cut_anywhere_to_a_third_of_its_depth_is_the_multiply_that_many_clocks_late() {
    if !common::require_tool("yosys") {
        return;
    }
    let v = fp32_verilog();
    let g = graph(v, "logos_fmul");
    let depth = g.depth().unwrap().depth;
    let period = (depth / 3 / LEVEL + 1) * LEVEL;
    let latency = min_latency(&g, period, Cuts::Nets).unwrap();
    assert!(latency >= 2, "a third of {depth} a stage takes more than one register");
    let plan = pipeline(&g, period, latency, Cuts::Nets).unwrap();
    assert!(check(&g, &plan));
    proven(v, "logos_fmul", &emit(&g, &plan, "logos_fmul_cut", None).unwrap(), "logos_fmul_cut", latency);
}

#[test]
fn an_enable_steps_every_register_and_the_module_elaborates() {
    if !common::require_tool("yosys") {
        return;
    }
    let g = graph(CHAIN, "chain");
    let latency = min_latency(&g, 2 * LEVEL, Cuts::Named).unwrap();
    let plan = pipeline(&g, 2 * LEVEL, latency, Cuts::Named).unwrap();
    let v = emit(&g, &plan, "chain_en", Some("step_en")).unwrap();
    assert!(v.contains("input step_en") && v.contains("always @(posedge clk) if (step_en) begin"), "{v}");
    let regs = v.lines().filter(|l| l.trim_start().starts_with("reg ")).count() as i64;
    assert_eq!(regs, plan.bits, "a register a planned bit");
    let n = netlist::synthesize(&v, "chain_en", &[]).unwrap();
    assert_eq!(n.domains.flops as i64, plan.bits);
}

const CLOCKED: &str = "module clocked(input clk, input rst, input [7:0] a, input [7:0] b, output [7:0] y);
  wire [7:0] s1 = a + b;
  wire [7:0] s2 = s1 ^ {b[3:0], a[7:4]};
  assign y = s2 + a;
endmodule
";

#[test]
fn a_clock_the_source_declares_and_never_reads_clocks_the_pipeline_whose_outputs_end_on_registers() {
    if !common::require_tool("yosys") {
        return;
    }
    let g = graph(CLOCKED, "clocked");
    let latency = min_latency(&g, 2 * LEVEL, Cuts::Nets).unwrap();
    let plan = pipeline(&g, 2 * LEVEL, latency, Cuts::Nets).unwrap().with_outputs_registered(&g);
    assert_eq!(plan.latency, latency + 1);
    assert!(check(&g, &plan));
    let v = emit(&g, &plan, "clocked_p", None).unwrap();
    assert_eq!(v.matches("input clk").count(), 1, "the source's clock is the pipeline's: {v}");
    assert!(v.contains("input [0:0] rst"), "the source's other ports are kept: {v}");
    for bit in 0..8 {
        let line = v.lines().find(|l| l.trim_start().starts_with(&format!("assign y[{bit}] = "))).unwrap_or_else(|| panic!("y[{bit}] in {v}"));
        assert!(line.contains("_d"), "y[{bit}] read straight from a register: {line}");
    }
    proven(CLOCKED, "clocked", &v, "clocked_p", latency + 1);
    let reads = "module reads(input clk, input [7:0] a, output [7:0] y); assign y = (a + 8'd3) ^ {8{clk}}; endmodule";
    let g = graph(reads, "reads");
    let plan = pipeline(&g, 2 * LEVEL, 1, Cuts::Nets).unwrap();
    assert_eq!(emit(&g, &plan, "reads_p", None), Err(EmitError::Control { port: "clk".into() }), "a clock the logic reads is data");
}

/// Three clocks, the second stage three times the first.
const UNEVEN: &str = "module uneven(input clk, input [7:0] a, input [7:0] b, output reg [7:0] y);
  reg [7:0] s1, s2;
  always @(posedge clk) begin
    s1 <= a ^ b;
    s2 <= ((s1 + 8'd3) ^ {s1[3:0], s1[7:4]}) + s1;
    y <= s2;
  end
endmodule
";

const UNEVEN_LOGIC: &str = "module uneven_logic(input [7:0] a, input [7:0] b, output [7:0] y);
  wire [7:0] s1 = a ^ b;
  assign y = ((s1 + 8'd3) ^ {s1[3:0], s1[7:4]}) + s1;
endmodule
";

#[test]
fn a_pipeline_cut_unevenly_is_cut_again_at_its_own_latency_and_is_still_its_logic_that_many_clocks_late() {
    if !common::require_tool("yosys") {
        return;
    }
    let g = graph(UNEVEN, "uneven");
    let (logic, latency) = unpipelined(&g).unwrap();
    assert_eq!(latency, 3);
    let period = (logic.depth().unwrap().depth + 1) / 2;
    assert!(g.depth().unwrap().depth > period, "the deep stage is past half the logic: {} vs {period}", g.depth().unwrap().depth);
    let plan = pipeline(&logic, period, latency - 1, Cuts::Nets).unwrap().with_outputs_registered(&logic);
    assert_eq!(plan.latency, latency);
    assert!(check(&logic, &plan), "every stage within {period}: {plan:?}");
    let v = emit(&logic, &plan, "uneven_recut", None).unwrap();
    assert_eq!(v.matches("input clk").count(), 1, "{v}");
    proven(UNEVEN_LOGIC, "uneven_logic", &v, "uneven_recut", latency);
    let held = "module held(input clk, input en, input [7:0] a, output reg [7:0] y); always @(posedge clk) if (en) y <= a + 8'd1; endmodule";
    assert!(matches!(unpipelined(&graph(held, "held")), Err(PipeError::Held { .. })), "a register under an enable is state");
}

#[test]
fn a_register_whose_next_value_is_sometimes_zero_is_a_plain_stage_not_a_reset() {
    if !common::require_tool("yosys") {
        return;
    }
    // The shape a saturating shift leaves: the stage's value is zero past a bound. A register with a reset pin would be
    // state; this one is a pipeline register whose logic holds a mux.
    let v = "module zeroed(input clk, input [7:0] a, input [7:0] d, output reg [7:0] y);
               reg [7:0] s;
               always @(posedge clk) begin
                 s <= (d > 8'd50) ? 8'd0 : a;
                 y <= s + 8'd1;
               end
             endmodule";
    let (logic, latency) = unpipelined(&graph(v, "zeroed")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(latency, 2);
    assert_eq!(logic.flops, 0);
}

#[test]
fn logic_around_a_box_is_not_cut_for_the_box_is_not_there_to_cut() {
    // An adder into a box and an XOR out of it: as deep as any module, but the box's own logic and its loops are not
    // the graph's, so no schedule of the graph's nodes is a pipeline of the module.
    let v = "(* blackbox *) module ip_core(input [7:0] d, output [7:0] q); endmodule
             module around(input [7:0] a, output [7:0] y);
               wire [7:0] q;
               ip_core u(.d(a + 8'd1), .q(q));
               assign y = q ^ a;
             endmodule";
    let g = graph(v, "around");
    let refused = PipeError::Boxed { cell: "u".into() };
    assert_eq!(min_latency(&g, LEVEL, Cuts::Nets), Err(refused.clone()));
    assert_eq!(pipeline(&g, LEVEL, 2, Cuts::Named), Err(refused.clone()));
    assert_eq!(unpipelined(&g).map(|(_, l)| l), Err(refused.clone()));
    assert_eq!(refused.to_string(), "u is a box, whose logic is not here to cut");
}

/// A stage whose operands are registered as it takes them, `b`'s under `en`, and their sum registered: an f32 unit's
/// shape, the operand a constant when the source hands it one, through a port, so its register outlives elaboration.
const STAGE: &str = "module stage(input clk, input en, input [7:0] a, input [7:0] b, output reg [7:0] y);
                       reg [7:0] aq, bq;
                       always @(posedge clk) begin aq <= a; if (en) bq <= b; y <= aq + bq; end
                     endmodule";

#[test]
fn a_register_holding_a_constant_operand_is_that_constant_when_the_pipeline_is_cut_afresh() {
    if !common::require_tool("yosys") {
        return;
    }
    let v = format!("{STAGE}\nmodule kreg(input clk, input [7:0] a, output [7:0] y); stage s(.clk(clk), .en(1'b1), .a(a), .b(8'd37), .y(y)); endmodule");
    let g = graph(&v, "kreg");
    assert_eq!(g.flops, 24, "the constant's register stays a register: its data is the constant only past elaboration");
    let (logic, latency) = unpipelined(&g).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(latency, 2);
    let plan = pipeline(&logic, logic.depth().unwrap().depth, latency - 1, Cuts::Nets).unwrap().with_outputs_registered(&logic);
    let piped = emit(&logic, &plan, "kreg_recut", None).unwrap_or_else(|e| panic!("{e}"));
    let sum = "module ksum(input [7:0] a, output [7:0] y); assign y = a + 8'd37; endmodule";
    proven(sum, "ksum", &piped, "kreg_recut", latency);
}

#[test]
fn a_register_stepping_under_an_enable_is_state_even_when_its_data_is_a_constant() {
    if !common::require_tool("yosys") {
        return;
    }
    let v = format!("{STAGE}\nmodule kreg_en(input clk, input en, input [7:0] a, output [7:0] y); stage s(.clk(clk), .en(en), .a(a), .b(8'd37), .y(y)); endmodule");
    match unpipelined(&graph(&v, "kreg_en")) {
        Err(PipeError::Held { register }) => assert!(register.starts_with("s.bq"), "{register}"),
        other => panic!("the enable is not the register's data: {:?}", other.map(|(_, l)| l)),
    }
}
