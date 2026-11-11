//! Timing graphs, end to end: Verilog through yosys into the retiming graph (`codegen_sva::timing_graph`), its depth
//! where the logic is, and the bound its loops put on the clock (`logicaffeine_proof::cycle_ratio`), every bound's
//! certificate re-checked.

use logicaffeine_compile::codegen_sva::netlist::{clock_sources, ClockDriver, ClockSource};
use logicaffeine_compile::codegen_sva::timing_graph::{synthesize, Area, Boxed, Cost, Delays, End, TimingError, TimingGraph};
use logicaffeine_proof::cycle_ratio::{check, Bound};

fn graph(verilog: &str, top: &str, params: &[(&str, &str)]) -> TimingGraph {
    synthesize(verilog, top, params, &[], &Delays::LEVELS).unwrap_or_else(|e| panic!("{top}: {e}"))
}

fn certified(g: &TimingGraph) -> Bound {
    let bound = g.loop_bound().expect("a well-formed graph");
    assert!(check(g.nodes.len(), &g.edges, &bound), "the bound's certificate checks: {bound:?}");
    bound
}

const LEVEL: i64 = Delays::LEVELS.lut;

#[test]
fn an_accumulator_is_a_loop_of_one_adder_through_one_register() {
    let v = "module acc_loop(input clk, input [15:0] x, output reg [15:0] acc);
               always @(posedge clk) acc <= acc + x;
             endmodule";
    let g = graph(v, "acc_loop", &[]);
    assert_eq!(g.flops, 16);
    let bound = certified(&g);
    assert_eq!(bound.period(), Some((LEVEL, 1)), "the adder's entry level, once a clock");
    let names = g.name_loop(&bound);
    assert!(names.iter().any(|n| n.starts_with("acc[") && n.ends_with("[1 register]")), "{names:?}");
    assert!(names.iter().any(|n| n.contains(".carry[")), "the loop runs through the adder's chain: {names:?}");
}

#[test]
fn the_slowest_loop_sets_the_bound_and_is_the_one_named() {
    let v = "module two_loops(input clk, input [7:0] a, output reg [7:0] p, output reg [7:0] q);
               always @(posedge clk) begin
                 p <= p * a + 8'd3;
                 q <= q ^ a;
               end
             endmodule";
    let g = graph(v, "two_loops", &[]);
    let bound = certified(&g);
    let (delay, registers) = bound.period().unwrap();
    assert_eq!(registers, 1);
    assert_eq!(delay, Delays::LEVELS.dsp + LEVEL, "a DSP block and the adder's entry level");
    let names = g.name_loop(&bound);
    assert!(names.iter().any(|n| n.starts_with("p[")), "{names:?}");
    assert!(!names.iter().any(|n| n.starts_with("q[")), "{names:?}");
}

#[test]
fn a_register_in_a_loop_halves_its_bound_and_a_register_outside_it_does_not() {
    let once = "module ring(input clk, input [5:0] k, output reg [5:0] s);
                  wire [5:0] t = ((s ^ k) & {s[4:0], s[5]}) | (s >> 1);
                  wire [5:0] u = ((t ^ k) & {t[0], t[5:1]}) | (t << 2);
                  always @(posedge clk) s <= u;
                endmodule";
    let twice = "module ring(input clk, input [5:0] k, output reg [5:0] s);
                   reg [5:0] h;
                   wire [5:0] t = ((s ^ k) & {s[4:0], s[5]}) | (s >> 1);
                   wire [5:0] u = ((h ^ k) & {h[0], h[5:1]}) | (h << 2);
                   always @(posedge clk) begin h <= t; s <= u; end
                 endmodule";
    let (a, b) = (certified(&graph(once, "ring", &[])), certified(&graph(twice, "ring", &[])));
    let ratio = |b: &Bound| -> f64 {
        let (d, r) = b.period().unwrap();
        d as f64 / r as f64
    };
    assert!(ratio(&b) < ratio(&a), "{a:?} {b:?}");
    let piped = "module ring(input clk, input [5:0] k, output reg [5:0] s, output reg [5:0] o);
                   reg [5:0] h;
                   wire [5:0] t = ((s ^ k) & {s[4:0], s[5]}) | (s >> 1);
                   always @(posedge clk) begin s <= t; h <= t; o <= ((h ^ k) & {h[0], h[5:1]}) | (h << 2); end
                 endmodule";
    let c = certified(&graph(piped, "ring", &[]));
    assert!(ratio(&c) <= ratio(&a), "a register after the loop only shortens paths");
}

#[test]
fn logic_without_a_loop_is_acyclic_and_its_depth_follows_its_width() {
    let v = "module wide_and #(parameter W = 4) (input [W-1:0] x, output y);
               assign y = &x;
             endmodule";
    let narrow = graph(v, "wide_and", &[("W", "6")]);
    assert_eq!(certified(&narrow), Bound::Acyclic);
    assert_eq!(narrow.depth().unwrap().depth, LEVEL, "six inputs are one LUT");
    let wide = graph(v, "wide_and", &[("W", "216")]);
    assert_eq!(wide.depth().unwrap().depth, 3 * LEVEL, "216 = 6^3 inputs are three levels of LUTs");
}

#[test]
fn a_product_by_a_power_of_two_is_wiring_and_any_other_is_a_dsp_block() {
    let v = "module scale #(parameter K = 8) (input [11:0] x, output [15:0] y);
               assign y = x * K;
             endmodule";
    let shift = graph(v, "scale", &[("K", "8")]);
    assert_eq!(shift.depth().unwrap().depth, 0);
    assert!(!shift.nodes.iter().any(|n| n.ends_with(".dsp")));
    let product = graph(v, "scale", &[("K", "7")]);
    assert!(product.nodes.iter().any(|n| n.ends_with(".dsp")), "{:?}", product.nodes);
    assert_eq!(product.depth().unwrap().depth, Delays::LEVELS.dsp);
}

#[test]
fn a_clocked_memory_holds_its_read_and_an_unclocked_one_becomes_logic() {
    let ram = "module ram(input clk, input we, input [5:0] wa, input [5:0] ra, input [7:0] d, output reg [7:0] q);
                 reg [7:0] m [0:63];
                 always @(posedge clk) begin if (we) m[wa] <= d; q <= m[ra]; end
               endmodule";
    let g = graph(ram, "ram", &[]);
    assert!(g.nodes.iter().any(|n| n.ends_with(".contents")), "{:?}", g.nodes);
    assert_eq!(certified(&g), Bound::Acyclic, "nothing read feeds a write");
    let rom = "module rom(input [3:0] a, output [7:0] y);
                 reg [7:0] t [0:15];
                 integer i;
                 initial for (i = 0; i < 16; i = i + 1) t[i] = i * 37;
                 assign y = t[a];
               endmodule";
    let g = graph(rom, "rom", &[]);
    assert!(!g.nodes.iter().any(|n| n.ends_with(".contents")), "an unclocked read is LUTs: {:?}", g.nodes);
    assert_eq!(g.depth().unwrap().depth, LEVEL, "sixteen words of a four-bit address are one LUT a bit");
}

#[test]
fn a_memory_cleared_at_once_is_registers_and_a_two_port_one_stays_a_block_ram() {
    // Cleared by a loop, one write port a word: no block RAM has seventeen write ports, so the fabric holds it in
    // registers, and the read's register stays its own.
    let cleared = "module many(input clk, input clear, input we, input [3:0] wa, input [7:0] wd, input [3:0] ra, output reg [7:0] y);
                     reg [7:0] mem [0:15];
                     integer k;
                     always @(posedge clk) begin
                       if (clear) for (k = 0; k < 16; k = k + 1) mem[k] <= 8'd0;
                       else if (we) mem[wa] <= wd;
                       y <= mem[ra];
                     end
                   endmodule";
    let g = graph(cleared, "many", &[]);
    assert!(!g.nodes.iter().any(|n| n.ends_with(".contents")), "registers, not a block RAM: {:?}", g.nodes.iter().filter(|n| n.contains("mem")).take(4).collect::<Vec<_>>());
    assert_eq!(g.flops, 16 * 8 + 8);
    let dual = "module dual(input clk, input wa, input wb, input [3:0] aa, input [3:0] ab, input [7:0] da, input [7:0] db, output reg [7:0] qa, output reg [7:0] qb);
                  reg [7:0] mem [0:15];
                  always @(posedge clk) begin
                    if (wa) mem[aa] <= da;
                    if (wb) mem[ab] <= db;
                    qa <= mem[aa];
                    qb <= mem[ab];
                  end
                endmodule";
    let g = graph(dual, "dual", &[]);
    assert!(g.nodes.iter().any(|n| n.ends_with(".contents")), "two write ports and clocked reads: a block RAM");
    assert_eq!(g.flops, 0, "its read registers are the RAM's own");
}

#[test]
fn a_ram_written_a_byte_lane_at_a_time_is_one_write_port_and_stays_a_block_ram() {
    let v = "module bytes(input clk, input we, input [3:0] wm, input [3:0] wa, input [31:0] wd, input [3:0] ra, output reg [31:0] rd);
               reg [31:0] mem [0:15];
               always @(posedge clk) rd <= mem[ra];
               always @(posedge clk)
                 if (we) begin
                   if (wm[0]) mem[wa][7:0] <= wd[7:0];
                   if (wm[1]) mem[wa][15:8] <= wd[15:8];
                   if (wm[2]) mem[wa][23:16] <= wd[23:16];
                   if (wm[3]) mem[wa][31:24] <= wd[31:24];
                 end
             endmodule";
    let g = graph(v, "bytes", &[]);
    assert!(g.nodes.iter().any(|n| n.ends_with(".contents")), "four byte lanes at one address are one port with byte enables");
    assert_eq!(g.flops, 0, "its read register is the RAM's own");
}

#[test]
fn a_ram_read_under_an_enable_is_a_block_ram_holding_its_own_read_register() {
    // The engine's RAMs: the read registered on the clocks its enable is high on, held between them.
    let v = "module held(input clk, input ce, input cr, input we, input [3:0] wa, input [31:0] wd, input [3:0] ra, output reg [31:0] rd);
               reg [31:0] mem [0:15];
               always @(posedge clk) if (cr) rd <= mem[ra];
               always @(posedge clk) if (ce && we) mem[wa] <= wd;
             endmodule";
    let g = graph(v, "held", &[]);
    assert!(g.nodes.iter().any(|n| n.ends_with(".contents")), "a block RAM, not sixteen words of registers");
    assert_eq!(g.flops, 0, "the held read is the RAM's own register");
}

#[test]
fn the_engines_ram_written_by_byte_lane_and_read_under_an_enable_is_one_block_ram() {
    let v = "module ram(input clk, input ce, input cr, input [3:0] ra, output reg [31:0] rd, input we, input [3:0] wa, input [31:0] wd, input [3:0] wm);
               reg [31:0] mem [0:15];
               always @(posedge clk) if (cr) rd <= mem[ra];
               always @(posedge clk)
                 if (ce && we) begin
                   if (wm[0]) mem[wa][7:0] <= wd[7:0];
                   if (wm[1]) mem[wa][15:8] <= wd[15:8];
                   if (wm[2]) mem[wa][23:16] <= wd[23:16];
                   if (wm[3]) mem[wa][31:24] <= wd[31:24];
                 end
             endmodule";
    let g = graph(v, "ram", &[]);
    assert!(g.nodes.iter().any(|n| n.ends_with(".contents")), "one block RAM with byte enables");
    assert_eq!(g.flops, 0, "its held read is the RAM's own register");
}

#[test]
fn each_instance_is_as_deep_as_the_paths_through_it() {
    let v = "module stage(input [5:0] a, output y); assign y = ^a; endmodule
             module chain(input [5:0] a, input [4:0] b, output y);
               wire m;
               stage u0(.a(a), .y(m));
               stage u1(.a({b, m}), .y(y));
               stage u2(.a(b + 5'd1), .y());
             endmodule";
    let g = graph(v, "chain", &[]);
    let d = g.depth().unwrap();
    assert_eq!(d.depth, 2 * LEVEL);
    let at = |p: &str| d.instances.iter().find(|i| i.path == p).unwrap_or_else(|| panic!("{p} in {:?}", d.instances));
    assert_eq!((at("u0").total, at("u0").own), (2 * LEVEL, 2 * LEVEL), "u0 feeds u1");
    assert_eq!(at("u1").total, 2 * LEVEL);
    assert_eq!(at("").total, 2 * LEVEL);
    let names: Vec<&str> = d.path.iter().map(|&n| g.nodes[n].as_str()).collect();
    assert_eq!(names.last(), Some(&"y"), "{names:?}");
}

#[test]
fn each_stage_of_a_pipeline_is_as_deep_as_the_deepest_path_into_its_registers() {
    // Stage one: a six-input XOR and a six-input AND, one LUT each. Stage two: an XOR of eight bits, two LUTs.
    let v = "module two_stage(input clk, input [5:0] a, input [5:0] b, output reg y);
               reg [1:0] s;
               always @(posedge clk) begin
                 s <= {^a, &b};
                 y <= ^{s, a};
               end
             endmodule";
    let g = graph(v, "two_stage", &[]);
    let ends = g.endpoints().unwrap();
    let seen: Vec<(&str, End, usize, i64)> = ends.iter().map(|e| (e.name.as_str(), e.end, e.bits, e.depth)).collect();
    assert_eq!(seen, [("y", End::Register, 1, 2 * LEVEL), ("s", End::Register, 2, LEVEL), ("y", End::Output, 1, 0)], "a registered output adds nothing");
    let comb = "module comb(input [5:0] a, input [5:0] b, output y); assign y = (&a) ^ (&b); endmodule";
    let seen: Vec<(String, End, i64)> = graph(comb, "comb", &[]).endpoints().unwrap().into_iter().map(|e| (e.name, e.end, e.depth)).collect();
    assert_eq!(seen, [("y".to_string(), End::Output, 2 * LEVEL)], "a module without registers ends at its outputs");
}

#[test]
fn a_combinational_loop_in_the_verilog_is_named() {
    let v = "module knot(input a, input c, output y);
               wire b;
               assign y = a ^ b;
               assign b = y & c;
             endmodule";
    let g = graph(v, "knot", &[]);
    let Err(TimingError::CombinationalLoop(names)) = g.depth() else { panic!("{:?}", g.depth()) };
    assert!(names.iter().any(|n| n == "y" || n == "b"), "{names:?}");
    assert!(matches!(g.loop_bound(), Ok(Bound::CombinationalLoop(_))));
}

#[test]
fn verilog_that_yosys_refuses_comes_back_with_its_log() {
    let err = synthesize("module broken(input a, output y); assign y = ; endmodule", "broken", &[], &[], &Delays::LEVELS).unwrap_err();
    let TimingError::Netlist(e) = err else { panic!("{err:?}") };
    assert!(e.to_string().contains("yosys refused the design"), "{e}");
}

#[test]
fn every_clock_is_named_with_what_drives_it_and_the_registers_it_clocks() {
    // A divided clock: one register toggled by the port's clock, clocking three more. The engine's shell built its slow
    // side so; the fitter left that clock off every global network.
    let divided = "module divided(input clk, input [2:0] d, output reg [2:0] q);
                     reg slow;
                     always @(posedge clk) slow <= ~slow;
                     always @(posedge slow) q <= d;
                   endmodule";
    let sources = clock_sources(divided, "divided", &[]).unwrap();
    assert_eq!(
        sources,
        [
            ClockSource { net: "slow".into(), driver: ClockDriver::Register, registers: 3 },
            ClockSource { net: "clk".into(), driver: ClockDriver::Port, registers: 1 },
        ],
        "a clock no port drives first"
    );
    let gated = "module gated(input clk, input en, input d, output reg q); wire g = clk & en; always @(posedge g) q <= d; endmodule";
    assert_eq!(clock_sources(gated, "gated", &[]).unwrap(), [ClockSource { net: "g".into(), driver: ClockDriver::Logic, registers: 1 }]);
    let clean = "module clean(input clk, input [7:0] d, output reg [7:0] q); always @(posedge clk) q <= d + 8'd1; endmodule";
    assert_eq!(clock_sources(clean, "clean", &[]).unwrap(), [ClockSource { net: "clk".into(), driver: ClockDriver::Port, registers: 8 }]);
    let ram = "module ram(input clk, input we, input [3:0] a, input [7:0] d, output reg [7:0] q);
                 reg [7:0] m [0:15];
                 always @(posedge clk) begin if (we) m[a] <= d; q <= m[a]; end
               endmodule";
    let sources = clock_sources(ram, "ram", &[]).unwrap();
    assert!(sources.iter().all(|s| s.driver == ClockDriver::Port && s.net == "clk"), "a memory's ports clock on the port too: {sources:?}");
}

/// A shell around a box: a register, a six-input AND of it into the box (one LUT), and the box's output XORed with the
/// register out (one LUT more). The box's own logic is an accumulator, a loop through an adder.
const SHELL: &str = "module engine_box #(parameter W = 4) (input clk, input [W-1:0] a, output reg [W-1:0] y);
                       always @(posedge clk) y <= y + a;
                     endmodule
                     module shell(input clk, input [5:0] x, output [3:0] z);
                       reg [5:0] r;
                       wire [3:0] e;
                       always @(posedge clk) r <= x;
                       engine_box #(.W(4)) engine(.clk(clk), .a({4{&r}}), .y(e));
                       assign z = e ^ r[3:0];
                     endmodule";

fn ends(g: &TimingGraph) -> Vec<(String, End, usize, i64)> {
    g.endpoints().unwrap().into_iter().map(|e| (e.name, e.end, e.bits, e.depth)).collect()
}

fn named(g: &TimingGraph, nodes: &[usize]) -> Vec<String> {
    nodes.iter().map(|&n| g.nodes[n].clone()).collect()
}

#[test]
fn a_box_ends_the_paths_into_it_and_starts_the_paths_out_of_it_and_none_of_its_logic_is_timed() {
    let g = synthesize(SHELL, "shell", &[], &[("engine_box", Cost::NONE, Area::default())], &Delays::LEVELS).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(g.flops, 6, "the box's registers are its own");
    let [b] = g.boxes.as_slice() else { panic!("one box: {:?}", g.boxes) };
    assert_eq!((b.cell.as_str(), b.module.as_str()), ("engine", "engine_box"), "the instance and the module it is of");
    let mut inputs = named(&g, &b.inputs);
    inputs.sort();
    assert_eq!(inputs, ["engine.a[0]", "engine.a[1]", "engine.a[2]", "engine.a[3]", "engine.clk"], "a node a bit it reads");
    let mut outputs = named(&g, &b.outputs);
    outputs.sort();
    assert_eq!(outputs, ["e[0]", "e[1]", "e[2]", "e[3]"], "the nets it drives");
    for &o in &b.outputs {
        assert!(!g.edges.iter().any(|e| e.to == o), "{}: nothing here drives a box's output", g.nodes[o]);
    }
    for &i in &b.inputs {
        assert!(!g.edges.iter().any(|e| e.from == i), "{}: a box's input leads nowhere here", g.nodes[i]);
    }
    assert_eq!(
        ends(&g),
        [
            ("z".to_string(), End::Output, 4, LEVEL),
            ("engine.a".to_string(), End::Box, 4, LEVEL),
            ("r".to_string(), End::Register, 6, 0),
            ("engine.clk".to_string(), End::Box, 1, 0),
        ],
        "the AND ends at the box, and the XOR starts from it"
    );
    assert_eq!(certified(&g), Bound::Acyclic, "the box's loop is the box's own");
    let open = graph(SHELL, "shell", &[]);
    assert!(open.boxes.is_empty());
    assert_eq!(open.flops, 10, "unboxed, its accumulator is the shell's");
    assert_eq!(certified(&open).period(), Some((LEVEL, 1)), "and so is its loop");
}

#[test]
fn a_module_the_source_declares_a_black_box_is_a_box_without_being_named() {
    let v = "(* blackbox *) module ip_core(input [7:0] d, output [7:0] q); endmodule
             module wrap(input [7:0] a, output y);
               wire [7:0] q;
               ip_core u(.d({8{^a}}), .q(q));
               assign y = &q[5:0];
             endmodule";
    let g = graph(v, "wrap", &[]);
    assert_eq!(g.boxes, [Boxed { cell: "u".into(), module: "ip_core".into(), out: Cost::NONE, inputs: g.boxes[0].inputs.clone(), outputs: g.boxes[0].outputs.clone() }]);
    assert_eq!((g.boxes[0].inputs.len(), g.boxes[0].outputs.len()), (8, 8));
    assert_eq!(ends(&g), [("u.d".to_string(), End::Box, 8, 2 * LEVEL), ("y".to_string(), End::Output, 1, LEVEL)], "the XOR of eight bits is two LUTs deep");
}

#[test]
fn a_box_no_module_is_named_is_refused_by_its_name() {
    let err = synthesize(SHELL, "shell", &[], &[("engine_bx", Cost::NONE, Area::default())], &Delays::LEVELS).unwrap_err();
    let TimingError::Netlist(e) = err else { panic!("{err:?}") };
    assert!(e.to_string().contains("engine_bx"), "{e}");
}

#[test]
fn a_ram_boxed_as_a_memory_is_timed_as_the_block_ram_it_maps_to() {
    // A RAM written a byte lane at a time and read through its clock, and a user of it: an XOR of its word out, and a
    // 16-bit parity into its write enable. Boxed, yosys never elaborates the RAM, and every path is as deep as it is
    // through the block RAM the RAM is read as.
    let v = "module ram16 #(parameter D = 16, A = 4) (input clk, input [A-1:0] ra, output reg [15:0] rd, input we,
                                                    input [A-1:0] wa, input [15:0] wd, input [1:0] wm);
               reg [15:0] mem [0:D-1];
               always @(posedge clk) rd <= mem[ra];
               always @(posedge clk) if (we) begin
                 if (wm[0]) mem[wa][7:0] <= wd[7:0];
                 if (wm[1]) mem[wa][15:8] <= wd[15:8];
               end
             endmodule
             module user(input clk, input [3:0] a, input [15:0] d, output reg [15:0] q);
               wire [15:0] r;
               reg [3:0] ra;
               always @(posedge clk) ra <= a + 4'd1;
               ram16 #(.D(16), .A(4)) m(.clk(clk), .ra(ra), .rd(r), .we(^d), .wa(a), .wd(d), .wm(2'b11));
               always @(posedge clk) q <= r ^ d;
             endmodule";
    let delays = Delays { lut: 100, carry: 7, dsp: 460, memory: 330 };
    let open = synthesize(v, "user", &[], &[], &delays).unwrap_or_else(|e| panic!("{e}"));
    let boxed = synthesize(v, "user", &[], &[("ram16", Cost::MEMORY, Area::default())], &delays).unwrap_or_else(|e| panic!("{e}"));
    assert!(open.boxes.is_empty());
    assert_eq!(boxed.boxes.len(), 1);
    assert_eq!(boxed.boxes[0].out, Cost::MEMORY);
    let depth = |g: &TimingGraph| g.depth().unwrap().depth;
    assert_eq!(depth(&boxed), depth(&open), "as deep with the RAM a box as with it a block RAM");
    assert_eq!(depth(&boxed), delays.memory + delays.lut, "the word read, then the XOR");
    let into = |g: &TimingGraph, name: &str| g.endpoints().unwrap().into_iter().find(|e| e.name == name).map(|e| e.depth);
    assert_eq!(into(&boxed, "q"), into(&open, "q"), "the register after the read");
    assert_eq!(into(&boxed, "m.we"), Some(2 * delays.lut), "the parity ends at the box's write enable");
    assert_eq!(into(&boxed, "ra"), into(&open, "ra"));
}

#[test]
fn a_memory_written_and_never_read_is_no_part_of_the_design() {
    // yosys 0.33 collects a memory no port reads into a cell its own check refuses; the model drops it first, as
    // synthesis would, and times the rest.
    let v = "module dead(input clk, input [1:0] wa, input [63:0] wd, input we, input [7:0] a, output reg [7:0] y);
               reg [63:0] ropet [0:3];
               always @(posedge clk) begin
                 if (we) ropet[wa] <= wd;
                 y <= a + 8'd1;
               end
             endmodule";
    let g = synthesize(v, "dead", &[], &[], &Delays::LEVELS).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(g.area_under("").memory_bits, 0, "the write-only memory is gone");
    assert_eq!(g.flops, 8, "y alone");
    assert_eq!(g.depth().unwrap().depth, Delays::LEVELS.lut, "a + 1 into y: one LUT level into the carry chain's first bit");
}

/// A device block declared a box with its area, as a vendor primitive's stand-in is: each instance adds that area to
/// the instance holding it, and an area not five counts is refused.
const DEVICE: &str = "(* blackbox, logos_box_area = \"3,1,2,4,0\" *)
module hard_unit(input clk, input [7:0] a, output [7:0] y);
endmodule
module device_top(input clk, input [7:0] a, output [7:0] y);
  wire [7:0] m;
  hard_unit u0(.clk(clk), .a(a), .y(m));
  hard_unit u1(.clk(clk), .a(m), .y(y));
endmodule
";

#[test]
fn a_box_declared_with_its_area_adds_it_for_every_instance() {
    let g = synthesize(DEVICE, "device_top", &[], &[], &Delays::LEVELS).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(g.boxes.len(), 2, "{:?}", g.boxes);
    let area = g.area_under("");
    assert_eq!((area.luts, area.carry, area.half_dsps, area.flops, area.memory_bits), (6, 2, 4, 8, 0), "two instances' areas");
    let bare = DEVICE.replace(", logos_box_area = \"3,1,2,4,0\"", "");
    let g = synthesize(&bare, "device_top", &[], &[], &Delays::LEVELS).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(g.area_under("").half_dsps, 0, "a box with no area declared takes none");
    let bad = DEVICE.replace("3,1,2,4,0", "3,1");
    assert!(matches!(synthesize(&bad, "device_top", &[], &[], &Delays::LEVELS), Err(TimingError::Json(_))), "an area of two counts");
}

#[test]
fn a_box_given_its_area_by_its_caller_adds_it_for_every_instance_as_a_declared_one_does() {
    let bare = DEVICE.replace(", logos_box_area = \"3,1,2,4,0\"", "");
    let given = Area { luts: 3, carry: 1, half_dsps: 2, flops: 4, memory_bits: 0 };
    let g = synthesize(&bare, "device_top", &[], &[("hard_unit", Cost::NONE, given)], &Delays::LEVELS).unwrap_or_else(|e| panic!("{e}"));
    let declared = synthesize(DEVICE, "device_top", &[], &[], &Delays::LEVELS).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(g.area_under(""), declared.area_under(""), "the area the caller gives is the one the stand-in declares");
    let none = synthesize(&bare, "device_top", &[], &[("hard_unit", Cost::NONE, Area::default())], &Delays::LEVELS).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(none.area_under("").luts, 0, "a box given no area takes none");
}

/// The Stratix 10 delays are fit27's measured worst-path figures in hundredths of a level, a level a LUT's 0.139 ns and
/// the 0.41 ns hop into the next: a carry bit 0.039 ns, a DSP block 2.10 ns and a RAM read 1.40 ns, each with its hop.
#[test]
fn the_stratix10_delays_are_fit27s_measured_paths_in_levels() {
    use logicaffeine_compile::codegen_sva::timing_graph::STRATIX10_NS_PER_LEVEL;
    let (lut, hop, carry, dsp, ram) = (0.139, 0.41, 0.039, 2.10, 1.40);
    assert!((lut + hop - STRATIX10_NS_PER_LEVEL).abs() < 0.001, "a level is a LUT and its hop");
    let level = |ns: f64| (100.0 * ns / STRATIX10_NS_PER_LEVEL).round() as i64;
    assert_eq!(Delays::STRATIX10, Delays { lut: level(lut + hop), carry: level(carry), dsp: level(dsp + hop), memory: level(ram + hop) });
}
