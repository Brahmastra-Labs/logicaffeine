//! qor_harness_lock — the comparative-synthesis harness measures what it claims to.
//!
//! Every candidate of a kernel runs inside `qorbist_<candidate>`, and its gate is one number: the
//! MISR signature of its first N results on the seeded stimulus must equal the golden signature
//! the LOGOS VM (or the oracle the RTL generator is locked against) predicts. The tests below hold
//! that gate to its properties under iverilog:
//!
//! * every LOGOS candidate produces the golden signature, on shaped and on edge stimulus;
//! * the signature is invariant to latency, to random stalls, to the initiation interval and to a
//!   go/done restart protocol — so a candidate is never rewarded or punished for its timing here;
//! * a single wrong result, or an unknown (X) bit, fails the gate;
//! * a memory interface answers each address with the word the tool's memory holds there — a Calyx
//!   two-dimensional `seq_mem` row-major, so a transposed read fails the gate;
//! * the port binding of every LOGOS candidate fits the ports yosys reads from its netlist.

mod common;

use logicaffeine_compile::codegen_sva::qor_harness::*;
use std::path::Path;
use std::process::Command;

const OPS: usize = 600;
/// The cycles a candidate may go without folding a result before the testbench refuses it.
const IDLE: u64 = 10_000;

fn kernel_of(id: &str) -> Kernel {
    kernel(id).unwrap_or_else(|| panic!("{id} is in the catalog"))
}

fn golden_signature(k: &Kernel, mode: Mode, ops: usize) -> u64 {
    let stim = stimulus(k, mode, ops);
    let out = golden(k, &stim).unwrap_or_else(|e| panic!("{} golden: {e}", k.id));
    signature(k, &out)
}

/// Compile and run `files` (plus the harness of `c` and its testbench) under iverilog.
fn simulate(dir: &Path, k: &Kernel, c: &Candidate, files: &[(String, String)], mode: Mode, ops: usize) -> Result<Run, QorError> {
    let mut paths = Vec::new();
    for (name, text) in files {
        let p = dir.join(name);
        std::fs::write(&p, text).expect("write dut file");
        paths.push(p);
    }
    let h = dir.join(format!("{}.v", harness_module(c)));
    std::fs::write(&h, emit_harness(k, c, mode)).expect("write harness");
    let tb = dir.join("tb_qor.v");
    std::fs::write(&tb, emit_testbench(c, ops, 200 * ops as u64 + 1_000, IDLE)).expect("write tb");
    let exe = dir.join("sim.vvp");
    let build = Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&exe)
        .arg("-s")
        .arg("tb_qor")
        .args(&paths)
        .arg(&h)
        .arg(&tb)
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog refused {}:\n{}",
        c.name,
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new("vvp").arg("-n").arg(&exe).output().expect("run vvp");
    parse_run(&String::from_utf8_lossy(&run.stdout))
}

fn tmp() -> tempfile::TempDir {
    tempfile::Builder::new().prefix("qor_harness_lock").tempdir().expect("tempdir")
}

fn assert_golden(k: &Kernel, c: &Candidate, files: &[(String, String)], mode: Mode, ops: usize) -> Run {
    let dir = tmp();
    let run = simulate(dir.path(), k, c, files, mode, ops).unwrap_or_else(|e| panic!("{} ({mode:?}): {e}", c.name));
    assert_eq!(run.ops, ops, "{}: every result was folded", c.name);
    assert_eq!(
        run.misr,
        golden_signature(k, mode, ops),
        "{} ({mode:?}): the signature differs from the golden",
        c.name
    );
    run
}

#[test]
fn every_logos_candidate_produces_the_golden_signature() {
    if !common::require_tool("iverilog") {
        return;
    }
    for (id, ops, per_op) in [("K1", OPS, 1.0), ("K2", OPS, 1.0), ("K5", OPS, 1.0), ("K4", 40, 26.0)] {
        let k = kernel_of(id);
        let (c, files) = logos_candidate(id).unwrap_or_else(|e| panic!("{id}: {e}"));
        for mode in [Mode::Shaped, Mode::Edge] {
            let run = assert_golden(&k, &c, &files, mode, ops);
            assert_eq!(run.steady_cycles_per_op(), Some(per_op), "{id}: steady-state cycles per operation");
        }
    }
    let k6 = kernel_of("K6");
    let variants = logos_candidates("K6").unwrap();
    assert_eq!(variants.len(), 5, "one K6 candidate per lane multiply strategy, and the step on ports");
    for (c, files) in &variants {
        for mode in [Mode::Shaped, Mode::Edge] {
            assert_golden(&k6, c, files, mode, 60);
        }
    }
}

/// K5's pipelined butterflies — one datapath (the product as an aligned adder tree, `m` from the product's low half,
/// the reduction as `13·m + (m >>> 8)`), registered once a multiply and after every step — fold the kernel's golden
/// signature in both stimulus modes at one result per cycle, each first result three edges after its latency.
#[test]
fn the_pipelined_ntt_butterflies_produce_the_golden_signature_at_one_result_per_cycle() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K5");
    let all = logos_candidates("K5").unwrap();
    let names: Vec<&str> = all.iter().map(|(c, _)| c.name.as_str()).collect();
    assert_eq!(names, ["k5_logos", "k5_logos_pipe", "k5_logos_deep"], "the butterfly in registers, then its two pipelines");
    for ((c, files), latency) in all[1..].iter().zip([5, 11]) {
        assert_eq!(c.protocol, Protocol::Registered { latency }, "{}", c.name);
        for mode in [Mode::Shaped, Mode::Edge] {
            let run = assert_golden(&k, c, files, mode, OPS);
            assert_eq!(run.steady_cycles_per_op(), Some(1.0), "{} {mode:?}: one result per cycle", c.name);
            assert_eq!(run.first_cycles, Some(u64::from(latency) + 3), "{}: operands on edge 1, folded, seen", c.name);
        }
    }
}

/// Stream `vectors` (a, b, zeta), one a cycle, through the butterfly emitted with `cuts`; every result must be
/// `fqmul`'s butterfly at the cut set's latency. Returns the simulator's verdict line.
fn stream_fqmul(cuts: logicaffeine_compile::codegen_sva::keccak_rtl::NttCuts, vectors: &[(i16, i16, i16)]) -> String {
    use logicaffeine_compile::codegen_sva::keccak_rtl::{emit_ntt_butterfly_pipelined_verilog, fqmul};
    let rom: String = vectors
        .iter()
        .map(|&(a, b, z)| {
            let t = fqmul(z, b);
            format!("{:04x}{:04x}{:04x}{:04x}{:04x}\n", a as u16, b as u16, z as u16, a.wrapping_add(t) as u16, a.wrapping_sub(t) as u16)
        })
        .collect();
    let dir = tempfile::tempdir().unwrap();
    let (n, lat) = (vectors.len(), cuts.latency());
    std::fs::write(dir.path().join("vec.hex"), rom).unwrap();
    std::fs::write(dir.path().join("bfly.v"), emit_ntt_butterfly_pipelined_verilog("bfly", cuts)).unwrap();
    std::fs::write(
        dir.path().join("tb.v"),
        format!(
            "module tb; reg clk = 0; always #1 clk = ~clk;\n\
             reg [79:0] v [0:{last}]; integer i, err = 0, seen = 0; reg signed [15:0] a = 0, b = 0, z = 0; wire signed [15:0] hi, lo;\n\
             bfly d(.clk(clk), .a(a), .b(b), .zeta(z), .hi(hi), .lo(lo));\n\
             initial begin $readmemh(\"vec.hex\", v);\n\
             for (i = 0; i < {n} + {lat}; i = i + 1) begin\n\
               if (i < {n}) {{a, b, z}} = v[i][79:32];\n\
               @(posedge clk); #0.5;\n\
               if (i >= {lat} - 1 && i - ({lat} - 1) < {n}) begin seen = seen + 1; if ({{hi, lo}} !== v[i - ({lat} - 1)][31:0]) err = err + 1; end\n\
             end\n\
             $display(\"RESULT %0d %0d\", seen, err); $finish; end\nendmodule\n",
            last = n - 1
        ),
    )
    .unwrap();
    let built = Command::new("iverilog").args(["-g2005", "-o", "sim.vvp", "tb.v", "bfly.v"]).current_dir(dir.path()).output().unwrap();
    assert!(built.status.success(), "{cuts:?}: {}", String::from_utf8_lossy(&built.stderr));
    let run = Command::new("vvp").args(["-n", "sim.vvp"]).current_dir(dir.path()).output().unwrap();
    String::from_utf8_lossy(&run.stdout).lines().find(|l| l.starts_with("RESULT")).unwrap_or("no RESULT").to_string()
}

/// Wherever the butterfly's registers go, its emitter delays every value to the stage that reads it: random cut sets
/// (and none, and the two the bench builds), each streamed one butterfly a cycle, every result `fqmul`'s at the cut
/// set's latency — operands at the extremes and at random, a new one every cycle.
#[test]
fn every_register_placement_of_the_ntt_butterfly_streams_fqmul() {
    use logicaffeine_compile::codegen_sva::keccak_rtl::{NttCuts, NTT_STEPS};
    if !common::require_tool("iverilog") {
        return;
    }
    let mut s = 0x2545_F491_4F6C_DD1Du64;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let mut sets = vec![NttCuts([false; NTT_STEPS]), NttCuts::PER_MULTIPLY, NttCuts::EVERY_STEP];
    for _ in 0..13 {
        let bits = next();
        sets.push(NttCuts(std::array::from_fn(|i| bits >> i & 1 == 1)));
    }
    let extremes = [i16::MIN, i16::MIN + 1, -3329, -1664, -1, 0, 1, 1664, 3329, i16::MAX];
    let vectors: Vec<(i16, i16, i16)> = (0..400)
        .map(|i| {
            let r = next();
            let pick = |k: u32| if i % 3 == 0 { extremes[(r >> (8 * k)) as usize % extremes.len()] } else { (r >> (16 * k)) as i16 };
            (pick(0), pick(1), pick(2))
        })
        .collect();
    for cuts in sets {
        assert_eq!(stream_fqmul(cuts, &vectors), format!("RESULT {} 0", vectors.len()), "{cuts:?} at latency {}", cuts.latency());
    }
}

/// The every-step butterfly against `fqmul` for every 16-bit `b`, each against the extreme zetas and q: every row of
/// the product's adder tree enabled in every combination, against the widest and the sign-only multiplicands.
#[test]
fn the_every_step_ntt_butterfly_streams_fqmul_for_every_b() {
    use logicaffeine_compile::codegen_sva::keccak_rtl::NttCuts;
    if !common::require_tool("iverilog") {
        return;
    }
    let vectors: Vec<(i16, i16, i16)> = [i16::MIN, i16::MAX, -1, 3329]
        .into_iter()
        .flat_map(|z| (i16::MIN..=i16::MAX).map(move |b| (b.rotate_left(5), b, z)))
        .collect();
    assert_eq!(stream_fqmul(NttCuts::EVERY_STEP, &vectors), format!("RESULT {} 0", vectors.len()));
}

/// K2's pipelined Horner residual — the compiler's residual of the polynomial, carry-save lowered and cut into stages
/// (`fabric_pipe::horner_seq_pipeline`, the RTL the `fabric_horner_seq` block hardens) — folds the kernel's golden
/// signature in both stimulus modes at one result per cycle, its first result three edges after its latency.
#[test]
fn the_pipelined_horner_residual_folds_the_golden_one_result_a_cycle() {
    use logicaffeine_compile::codegen_sva::fabric_pipe::horner_seq_pipeline;
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K2");
    let all = logos_candidates("K2").unwrap();
    let names: Vec<&str> = all.iter().map(|(c, _)| c.name.as_str()).collect();
    assert_eq!(names, ["k2_logos", "k2_logos_seq", "k2_logos_pipe"]);
    let (c, files) = &all[2];
    let latency = horner_seq_pipeline().1.latency() as u32;
    assert_eq!(c.protocol, Protocol::Registered { latency });
    for mode in [Mode::Shaped, Mode::Edge] {
        let run = assert_golden(&k, c, files, mode, OPS);
        assert_eq!(run.steady_cycles_per_op(), Some(1.0), "{mode:?}: one result per cycle");
        assert_eq!(run.first_cycles, Some(u64::from(latency) + 3), "{mode:?}");
    }
}

/// K1's pipelined decision — the compiled decision fabric lowered to the pipeline's op vocabulary (every compare a
/// carry-chain subtract and the mask of its outcome, every select a masked merge) and cut one carry chain to a stage
/// (`fabric_pipe::decision_ops`, `DECISION_CHAIN_BUDGET`) — folds the kernel's golden signature in both stimulus
/// modes at one result per cycle, its first result three edges after its latency.
#[test]
fn the_pipelined_decision_fabric_folds_the_golden_one_result_a_cycle() {
    use logicaffeine_compile::codegen_sva::decision_workload::{DECIDE_FN, DECIDE_NAME};
    use logicaffeine_compile::codegen_sva::fabric_pipe::{decision_ops, schedule_pipeline, DECISION_CHAIN_BUDGET};
    use logicaffeine_compile::codegen_sva::fabric_rtl::decision_fabric_from_source;
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K1");
    let all = logos_candidates("K1").unwrap();
    let names: Vec<&str> = all.iter().map(|(c, _)| c.name.as_str()).collect();
    assert_eq!(names, ["k1_logos", "k1_logos_seq", "k1_logos_pipe", "k1_logos_cells", "k1_logos_hoisted"]);
    let (c, files) = &all[2];
    let (ops, result) = decision_ops(&decision_fabric_from_source(DECIDE_FN, DECIDE_NAME)).unwrap();
    let latency = schedule_pipeline(&ops, result, DECISION_CHAIN_BUDGET).latency() as u32;
    assert_eq!(c.protocol, Protocol::Registered { latency });
    for mode in [Mode::Shaped, Mode::Edge] {
        let run = assert_golden(&k, c, files, mode, OPS);
        assert_eq!(run.steady_cycles_per_op(), Some(1.0), "{mode:?}: one result per cycle");
        assert_eq!(run.first_cycles, Some(u64::from(latency) + 3), "{mode:?}");
    }
}

/// K1 for standard cells: the decision with each compare's sums folded into it, cut at the standard-cell budget
/// (`DECISION_CELLS_BUDGET`, three stages), and the
/// decision hoisted into one clock (`select_hoist`, every sum folded into the compare that reads it) both read their
/// ports, which the harness's registers hold, and fold the kernel's golden signature in both stimulus modes at one
/// result per cycle — the hoisted decision one clock after its operands, the cut one its stages after.
#[test]
fn the_decision_for_standard_cells_folds_the_golden_one_result_a_cycle() {
    use logicaffeine_compile::codegen_sva::decision_workload::{DECIDE_FN, DECIDE_NAME};
    use logicaffeine_compile::codegen_sva::fabric_pipe::{
        carry_save_lower_with, decision_ops, emit_fabric_pipelined_verilog, schedule_pipeline, FinalAdd, Inputs, SumSharing,
        DECISION_CELLS_BUDGET,
    };
    use logicaffeine_compile::codegen_sva::fabric_rtl::decision_fabric_from_source;
    use logicaffeine_compile::codegen_sva::select_hoist::hoist_selects;
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K1");
    let all = logos_candidates("K1").unwrap();
    let df = decision_fabric_from_source(DECIDE_FN, DECIDE_NAME);
    let (ops, result) = decision_ops(&df).unwrap();
    let (folded, fores) = carry_save_lower_with(&ops, result, 64, SumSharing::Fold, FinalAdd::Native);
    let cut = schedule_pipeline(&folded, fores, DECISION_CELLS_BUDGET).reading(Inputs::Ports);
    assert_eq!(cut.latency(), 3, "the standard-cell budget cuts the decision into three stages");
    let (hops, hres) = decision_ops(&hoist_selects(&df, 64).unwrap()).unwrap();
    let (flat, fres) = carry_save_lower_with(&hops, hres, 64, SumSharing::Fold, FinalAdd::PrefixTree);
    let one = schedule_pipeline(&flat, fres, f64::INFINITY).reading(Inputs::Ports);
    let built = [
        emit_fabric_pipelined_verilog("hft_cells", df.n_in, &folded, &cut),
        emit_fabric_pipelined_verilog("hft_hoisted", df.n_in, &flat, &one),
    ];
    for ((i, latency), rtl) in [(3, 3u32), (4, 1)].into_iter().zip(built) {
        let (c, files) = &all[i];
        assert_eq!(files[0].1, rtl, "{}: the RTL is not the construction residual_pipeline_lock certifies", c.name);
        assert_eq!(c.protocol, Protocol::Registered { latency }, "{}", c.name);
        assert!(files.iter().all(|(_, v)| !v.contains("in0_r")), "{}: an operand registered twice", c.name);
        for mode in [Mode::Shaped, Mode::Edge] {
            let run = assert_golden(&k, c, files, mode, OPS);
            assert_eq!(run.steady_cycles_per_op(), Some(1.0), "{} {mode:?}: one result per cycle", c.name);
            assert_eq!(run.first_cycles, Some(u64::from(latency) + 3), "{} {mode:?}", c.name);
        }
    }
}

/// K6's streamed engine — one multiply shared by the four lanes, which each lane's `x` enters as it loads — folds
/// the kernel's golden signature in both stimulus modes, the multiplies of the first lanes hidden behind the loads:
/// an operation costs the single-cycle combinational engine's clocks plus only the last lane's way through the tree's
/// registers (its operands register in the clock its `x` loads) and the clocks that register and fold its result.
#[test]
fn the_streamed_simt_engine_folds_the_golden_with_its_multiply_behind_the_loads() {
    use logicaffeine_compile::codegen_sva::mul_tree::{mul_tree, mul_tree_levels};
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K6");
    let all = logos_candidates("K6").unwrap();
    let names: Vec<&str> = all.iter().map(|(c, _)| c.name.as_str()).collect();
    assert_eq!(names, ["k6_logos_seqfma", "k6_logos_fma2", "k6_logos_comb", "k6_logos_streamed", "k6_logos_ports"]);
    let (comb, comb_files) = &all[2];
    let (c, files) = &all[3];
    let floor = assert_golden(&k, comb, comb_files, Mode::Shaped, 60).steady_cycles_per_op().unwrap();
    let levels = mul_tree_levels(64);
    let tree = mul_tree("m", "a", "x", 64, |l| (l + 1) % K6_STREAMED_EVERY == 0 || l + 1 == levels).latency;
    for mode in [Mode::Shaped, Mode::Edge] {
        let per = assert_golden(&k, c, files, mode, 60).steady_cycles_per_op().unwrap();
        assert_eq!(per, floor + f64::from(tree + 2), "{mode:?}: the combinational engine's {floor} clocks and the last lane's multiply");
    }
}

/// K6's step on ports — every operand from the harness's registers at once, each lane's product an aligned tree of
/// adds registered every `K6_PORTS_EVERY` levels and after its last, every lane's output and the sum registered
/// together (`simt_ports`) — folds the kernel's golden signature in both stimulus modes at one step per cycle, the
/// tree's registers and two more after its operands.
#[test]
fn the_simt_step_on_ports_folds_the_golden_one_step_a_cycle() {
    use logicaffeine_compile::codegen_sva::mul_tree::mul_tree_levels;
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K6");
    let all = logos_candidates("K6").unwrap();
    let (c, files) = &all[4];
    assert_eq!(c.name, "k6_logos_ports");
    let levels = mul_tree_levels(64);
    let tree = (0..levels).filter(|l| (l + 1) % K6_PORTS_EVERY == 0 || l + 1 == levels).count() as u32;
    let latency = tree + 2;
    assert_eq!(c.protocol, Protocol::Registered { latency });
    for mode in [Mode::Shaped, Mode::Edge] {
        let run = assert_golden(&k, c, files, mode, OPS);
        assert_eq!(run.steady_cycles_per_op(), Some(1.0), "{mode:?}: one step per cycle");
        assert_eq!(run.first_cycles, Some(u64::from(latency) + 3), "{mode:?}");
    }
}

/// K4's half-round core — a register between theta and the rest of each round, so neither half carries a whole round's
/// wiring — folds the permutation's golden signature in both stimulus modes, two clocks a round: 48 and the start and
/// done edges, against the full-round core's 26.
#[test]
fn the_half_round_keccak_core_folds_the_golden_two_clocks_a_round() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K4");
    let all = logos_candidates("K4").unwrap();
    let names: Vec<&str> = all.iter().map(|(c, _)| c.name.as_str()).collect();
    assert_eq!(names, ["k4_logos", "k4_logos_half"]);
    let (c, files) = &all[1];
    assert_eq!(c.protocol, all[0].0.protocol, "the same start/done handshake");
    for mode in [Mode::Shaped, Mode::Edge] {
        let run = assert_golden(&k, c, files, mode, 40);
        assert_eq!(run.steady_cycles_per_op(), Some(50.0), "{mode:?}: two clocks a round");
    }
}

#[test]
fn a_registered_candidate_produces_one_result_per_cycle() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K5");
    let (c, files) = logos_candidate("K5").unwrap();
    let run = assert_golden(&k, &c, &files, Mode::Shaped, OPS);
    assert_eq!(run.cycles, OPS as u64 + 4, "latency 2, one result per cycle, the fold one edge later, the verdict on the next");
    assert_eq!(run.steady_cycles_per_op(), Some(1.0));
    assert_eq!(run.first_cycles, Some(5), "operands on edge 1, result on edge 3, folded on edge 4, seen on edge 5");
}

fn dp(list: &[(&str, Dir, u32)]) -> Vec<DutPort> {
    list.iter()
        .map(|(n, d, b)| DutPort { name: n.to_string(), dir: *d, bits: *b })
        .collect()
}

/// The handshaked butterfly ports every hand-written test DUT below declares.
fn handshake_ports(reset: &str, start: &str) -> Vec<DutPort> {
    dp(&[
        ("clk", Dir::In, 1),
        (reset, Dir::In, 1),
        (start, Dir::In, 1),
        ("a", Dir::In, 16),
        ("b", Dir::In, 16),
        ("zeta", Dir::In, 16),
        ("done", Dir::Out, 1),
        ("hi", Dir::Out, 16),
        ("lo", Dir::Out, 16),
    ])
}

/// The LOGOS butterfly with `extra` more output registers.
fn delayed_butterfly(extra: u32) -> (Candidate, Vec<(String, String)>) {
    let (mut c, mut files) = logos_candidate("K5").unwrap();
    let mut v = String::from(
        "module qor_k5_delayed(input clk, input signed [15:0] a, input signed [15:0] b, input signed [15:0] zeta, output [15:0] hi, output [15:0] lo);\n  wire [15:0] h0, l0;\n  qor_k5_logos u(.clk(clk), .a(a), .b(b), .zeta(zeta), .hi(h0), .lo(l0));\n",
    );
    for i in 1..=extra {
        v.push_str(&format!("  reg [15:0] h{i}, l{i};\n  always @(posedge clk) begin h{i} <= h{}; l{i} <= l{}; end\n", i - 1, i - 1));
    }
    v.push_str(&format!("  assign hi = h{extra};\n  assign lo = l{extra};\nendmodule\n"));
    files.push(("qor_k5_delayed.v".into(), v));
    c.name = format!("k5_delayed_{extra}");
    c.top = "qor_k5_delayed".into();
    c.protocol = Protocol::Registered { latency: 2 + extra };
    c.ports = dp(&[
        ("clk", Dir::In, 1),
        ("a", Dir::In, 16),
        ("b", Dir::In, 16),
        ("zeta", Dir::In, 16),
        ("hi", Dir::Out, 16),
        ("lo", Dir::Out, 16),
    ]);
    (c, files)
}

#[test]
fn latency_does_not_move_the_signature() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K5");
    for extra in 1..=4 {
        let (c, files) = delayed_butterfly(extra);
        assert_golden(&k, &c, &files, Mode::Shaped, OPS);
    }
}

/// The butterfly behind a start/done handshake whose busy time is drawn from an internal LFSR
/// (1 to 8 cycles): a stalling candidate.
fn stalling_butterfly() -> (Candidate, Vec<(String, String)>) {
    let v = "module qor_k5_stall(input clk, input rst, input start, input signed [15:0] a, input signed [15:0] b, input signed [15:0] zeta, output reg done, output reg [15:0] hi, output reg [15:0] lo);\n\
  wire signed [15:0] h, l;\n\
  ntt_butterfly u(.a(a), .b(b), .zeta(zeta), .hi(h), .lo(l));\n\
  reg [15:0] r;\n\
  reg [3:0] wait_n;\n\
  reg busy;\n\
  always @(posedge clk) begin\n\
    if (rst) begin r <= 16'hACE1; busy <= 1'b0; done <= 1'b0; end\n\
    else begin\n\
      done <= 1'b0;\n\
      r <= {r[14:0], r[15] ^ r[13] ^ r[12] ^ r[10]};\n\
      if (start && !busy) begin busy <= 1'b1; wait_n <= {1'b0, r[2:0]}; hi <= h; lo <= l; end\n\
      else if (busy) begin\n\
        if (wait_n == 4'd0) begin busy <= 1'b0; done <= 1'b1; end\n\
        else wait_n <= wait_n - 4'd1;\n\
      end\n\
    end\n\
  end\n\
endmodule\n";
    let c = Candidate {
        kernel: "K5".into(),
        name: "k5_stall".into(),
        top: "qor_k5_stall".into(),
        clock: "clk".into(),
        reset: Some(Reset { port: "rst".into(), active_high: true }),
        protocol: Protocol::StartDone { start: "start".into(), done: "done".into() },
        inputs: vec![Bind::whole("a", "a"), Bind::whole("b", "b"), Bind::whole("zeta", "zeta")],
        outputs: vec![Bind::whole("hi", "hi"), Bind::whole("lo", "lo")],
        mems: vec![],
        batch: 1,
        ports: handshake_ports("rst", "start"),
    };
    let bfly = logicaffeine_compile::codegen_sva::keccak_rtl::emit_ntt_butterfly_verilog();
    (c, vec![("ntt_butterfly.v".into(), bfly), ("qor_k5_stall.v".into(), v.to_string())])
}

/// The butterfly behind a start/done handshake that answers five operations and then never raises
/// `done` again.
fn stuck_butterfly() -> (Candidate, Vec<(String, String)>) {
    let (mut c, mut files) = stalling_butterfly();
    c.name = "k5_stuck".into();
    c.top = "qor_k5_stuck".into();
    files[1] = ("qor_k5_stuck.v".into(), "module qor_k5_stuck(input clk, input rst, input start, input signed [15:0] a, input signed [15:0] b, input signed [15:0] zeta, output reg done, output reg [15:0] hi, output reg [15:0] lo);\n\
  wire signed [15:0] h, l;\n\
  ntt_butterfly u(.a(a), .b(b), .zeta(zeta), .hi(h), .lo(l));\n\
  reg [3:0] answered;\n\
  always @(posedge clk) begin\n\
    if (rst) begin answered <= 4'd0; done <= 1'b0; end\n\
    else begin\n\
      done <= 1'b0;\n\
      if (start && !done && answered < 4'd5) begin hi <= h; lo <= l; done <= 1'b1; answered <= answered + 4'd1; end\n\
    end\n\
  end\n\
endmodule\n".into());
    (c, files)
}

#[test]
fn a_candidate_that_stops_answering_is_refused_once_it_has_been_idle_too_long() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K5");
    let (c, files) = stuck_butterfly();
    let dir = tmp();
    match simulate(dir.path(), &k, &c, &files, Mode::Shaped, OPS) {
        Err(QorError::Testbench(line)) => {
            assert!(line.starts_with(&format!("QOR TIMEOUT idle {IDLE} cycles ops 5 ")), "{line}");
            let cycle: u64 = line.rsplit(' ').next().unwrap().parse().unwrap();
            assert!(cycle < 2 * IDLE, "refused at cycle {cycle}, long before the {}-cycle budget", 200 * OPS + 1_000);
        }
        other => panic!("a stuck candidate must be refused as idle, got {other:?}"),
    }
}

#[test]
fn stalls_do_not_move_the_signature() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K5");
    let (c, files) = stalling_butterfly();
    let run = assert_golden(&k, &c, &files, Mode::Shaped, OPS);
    assert!(run.cycles > 3 * OPS as u64, "the stalls really happened ({} cycles)", run.cycles);
}

/// A three-stage pipelined butterfly that accepts on `start` and raises `done` three cycles later.
fn pipelined_butterfly() -> (Candidate, Vec<(String, String)>) {
    let v = "module qor_k5_pipe(input clk, input rst, input start, input signed [15:0] a, input signed [15:0] b, input signed [15:0] zeta, output done, output [15:0] hi, output [15:0] lo);\n\
  wire signed [15:0] h, l;\n\
  ntt_butterfly u(.a(a), .b(b), .zeta(zeta), .hi(h), .lo(l));\n\
  reg [15:0] h1, l1, h2, l2, h3, l3;\n\
  reg v1, v2, v3;\n\
  always @(posedge clk) begin\n\
    if (rst) begin v1 <= 1'b0; v2 <= 1'b0; v3 <= 1'b0; end\n\
    else begin v1 <= start; v2 <= v1; v3 <= v2; end\n\
    h1 <= h; l1 <= l; h2 <= h1; l2 <= l1; h3 <= h2; l3 <= l2;\n\
  end\n\
  assign done = v3;\n\
  assign hi = h3;\n\
  assign lo = l3;\n\
endmodule\n";
    let c = Candidate {
        kernel: "K5".into(),
        name: "k5_pipe".into(),
        top: "qor_k5_pipe".into(),
        clock: "clk".into(),
        reset: Some(Reset { port: "rst".into(), active_high: true }),
        protocol: Protocol::Pipelined { start: "start".into(), done: "done".into(), ii: 1 },
        inputs: vec![Bind::whole("a", "a"), Bind::whole("b", "b"), Bind::whole("zeta", "zeta")],
        outputs: vec![Bind::whole("hi", "hi"), Bind::whole("lo", "lo")],
        mems: vec![],
        batch: 1,
        ports: handshake_ports("rst", "start"),
    };
    let bfly = logicaffeine_compile::codegen_sva::keccak_rtl::emit_ntt_butterfly_verilog();
    (c, vec![("ntt_butterfly.v".into(), bfly), ("qor_k5_pipe.v".into(), v.to_string())])
}

#[test]
fn the_initiation_interval_does_not_move_the_signature() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K5");
    for ii in [1u32, 3] {
        let (mut c, files) = pipelined_butterfly();
        c.name = format!("k5_pipe_ii{ii}");
        c.protocol = Protocol::Pipelined { start: "start".into(), done: "done".into(), ii };
        let run = assert_golden(&k, &c, &files, Mode::Shaped, OPS);
        let expect = ii as u64 * (OPS as u64 - 1) + 6;
        assert_eq!(run.cycles, expect, "II {ii}: one result every {ii} cycles after the fill");
        assert_eq!(run.steady_cycles_per_op(), Some(ii as f64));
    }
}

/// A go/done component in the style of a Calyx toplevel: while `go` is high it computes for two
/// cycles, pulses `done` for one, and restarts.
fn go_done_butterfly() -> (Candidate, Vec<(String, String)>) {
    let v = "module qor_k5_godone(input clk, input reset, input go, input signed [15:0] a, input signed [15:0] b, input signed [15:0] zeta, output done, output reg [15:0] hi, output reg [15:0] lo);\n\
  wire signed [15:0] h, l;\n\
  ntt_butterfly u(.a(a), .b(b), .zeta(zeta), .hi(h), .lo(l));\n\
  reg [1:0] fsm;\n\
  always @(posedge clk) begin\n\
    if (reset) fsm <= 2'd0;\n\
    else if (go) begin\n\
      if (fsm == 2'd0) begin hi <= h; lo <= l; end\n\
      fsm <= (fsm == 2'd2) ? 2'd0 : fsm + 2'd1;\n\
    end\n\
  end\n\
  assign done = go && fsm == 2'd2;\n\
endmodule\n";
    let c = Candidate {
        kernel: "K5".into(),
        name: "k5_godone".into(),
        top: "qor_k5_godone".into(),
        clock: "clk".into(),
        reset: Some(Reset { port: "reset".into(), active_high: true }),
        protocol: Protocol::GoDone { go: "go".into(), done: "done".into() },
        inputs: vec![Bind::whole("a", "a"), Bind::whole("b", "b"), Bind::whole("zeta", "zeta")],
        outputs: vec![Bind::whole("hi", "hi"), Bind::whole("lo", "lo")],
        mems: vec![],
        batch: 1,
        ports: handshake_ports("reset", "go"),
    };
    let bfly = logicaffeine_compile::codegen_sva::keccak_rtl::emit_ntt_butterfly_verilog();
    (c, vec![("ntt_butterfly.v".into(), bfly), ("qor_k5_godone.v".into(), v.to_string())])
}

#[test]
fn a_go_done_restart_does_not_move_the_signature() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K5");
    let (c, files) = go_done_butterfly();
    let run = assert_golden(&k, &c, &files, Mode::Shaped, OPS);
    assert_eq!(run.cycles, 3 * OPS as u64 + 2, "three cycles per operation");
    assert_eq!(run.steady_cycles_per_op(), Some(3.0));
}

/// A pipelined butterfly whose two results leave at different depths, each marked by its own
/// one-cycle strobe: `hi` two cycles after the operands, `lo` five.
fn staggered_butterfly() -> (Candidate, Vec<(String, String)>) {
    let v = "module qor_k5_stagger(input clk, input rst, input start, input signed [15:0] a, input signed [15:0] b, input signed [15:0] zeta, output done, output [15:0] hi, output hi_vld, output [15:0] lo, output lo_vld);\n\
  wire signed [15:0] h, l;\n\
  ntt_butterfly u(.a(a), .b(b), .zeta(zeta), .hi(h), .lo(l));\n\
  reg [15:0] h1, h2, l1, l2, l3, l4, l5;\n\
  reg [4:0] v;\n\
  always @(posedge clk) begin\n\
    if (rst) v <= 5'd0; else v <= {v[3:0], start};\n\
    h1 <= h; h2 <= h1;\n\
    l1 <= l; l2 <= l1; l3 <= l2; l4 <= l3; l5 <= l4;\n\
  end\n\
  assign hi = h2;\n\
  assign hi_vld = v[1];\n\
  assign lo = l5;\n\
  assign lo_vld = v[4];\n\
  assign done = v[4];\n\
endmodule\n";
    let mut ports = handshake_ports("rst", "start");
    ports.push(DutPort { name: "hi_vld".into(), dir: Dir::Out, bits: 1 });
    ports.push(DutPort { name: "lo_vld".into(), dir: Dir::Out, bits: 1 });
    let c = Candidate {
        kernel: "K5".into(),
        name: "k5_stagger".into(),
        top: "qor_k5_stagger".into(),
        clock: "clk".into(),
        reset: Some(Reset { port: "rst".into(), active_high: true }),
        protocol: Protocol::Pipelined { start: "start".into(), done: "done".into(), ii: 1 },
        inputs: vec![Bind::whole("a", "a"), Bind::whole("b", "b"), Bind::whole("zeta", "zeta")],
        outputs: vec![Bind::whole("hi", "hi").valid_on("hi_vld"), Bind::whole("lo", "lo").valid_on("lo_vld")],
        mems: vec![],
        batch: 1,
        ports,
    };
    let bfly = logicaffeine_compile::codegen_sva::keccak_rtl::emit_ntt_butterfly_verilog();
    (c, vec![("ntt_butterfly.v".into(), bfly), ("qor_k5_stagger.v".into(), v.to_string())])
}

#[test]
fn outputs_that_leave_at_different_depths_are_realigned_by_their_strobes() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K5");
    let (c, files) = staggered_butterfly();
    check_ports(&k, &c).unwrap();
    let run = assert_golden(&k, &c, &files, Mode::Shaped, OPS);
    assert_eq!(run.steady_cycles_per_op(), Some(1.0), "the queues never stall a one-per-cycle pipeline");
}

/// A go/done component that, like a Calyx toplevel, starts its next run in the cycle it raises
/// `done`: it samples its operands at the end of the `done` cycle of the run before (and, for the
/// first run, at the end of its first cycle).
fn eager_restart_butterfly() -> (Candidate, Vec<(String, String)>) {
    let v = "module qor_k5_eager(input clk, input reset, input go, input signed [15:0] a, input signed [15:0] b, input signed [15:0] zeta, output done, output reg [15:0] hi, output reg [15:0] lo);\n\
  wire signed [15:0] h, l;\n\
  ntt_butterfly u(.a(a), .b(b), .zeta(zeta), .hi(h), .lo(l));\n\
  reg [15:0] ha, la;\n\
  reg [1:0] fsm;\n\
  reg first;\n\
  always @(posedge clk) begin\n\
    if (reset) begin fsm <= 2'd0; first <= 1'b1; end\n\
    else if (go) begin\n\
      first <= 1'b0;\n\
      if (first || fsm == 2'd2) begin ha <= h; la <= l; end\n\
      if (fsm == 2'd1) begin hi <= ha; lo <= la; end\n\
      fsm <= (fsm == 2'd2) ? 2'd0 : fsm + 2'd1;\n\
    end\n\
  end\n\
  assign done = go && fsm == 2'd2;\n\
endmodule\n";
    let (mut c, _) = go_done_butterfly();
    c.name = "k5_eager".into();
    c.top = "qor_k5_eager".into();
    let bfly = logicaffeine_compile::codegen_sva::keccak_rtl::emit_ntt_butterfly_verilog();
    (c, vec![("ntt_butterfly.v".into(), bfly), ("qor_k5_eager.v".into(), v.to_string())])
}

#[test]
fn a_component_that_restarts_in_its_done_cycle_sees_the_next_operands() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K5");
    let (c, files) = eager_restart_butterfly();
    assert_golden(&k, &c, &files, Mode::Shaped, OPS);
}

/// A butterfly controlled as Spatial's accelerator is: it runs while `io_enable` is high, writes each result with a
/// one-cycle strobe (its value undefined otherwise), and raises a `done` that stays up until its own `io_reset` clears
/// it; its run counter clears only while it is disabled, so one left enabled through the clear starts its next run
/// late. It also declares inputs its binding never names — an `echo` it XORs into `hi`, as a floating input would
/// make it unknown, and a `ready` it ignores.
fn enable_done_butterfly() -> (Candidate, Vec<(String, String)>) {
    let v = "module qor_k5_enable(input clock, input reset, input io_enable, input io_reset, input [15:0] io_echo, input io_ready, input signed [15:0] a, input signed [15:0] b, input signed [15:0] zeta, output io_done, output [15:0] hi, output hi_valid, output [15:0] lo, output lo_valid);\n\
  wire signed [15:0] h, l;\n\
  ntt_butterfly u(.a(a), .b(b), .zeta(zeta), .hi(h), .lo(l));\n\
  reg [1:0] fsm;\n\
  reg done_r, wr;\n\
  always @(posedge clock) begin\n\
    if (reset) begin fsm <= 2'd0; done_r <= 1'b0; wr <= 1'b0; end\n\
    else begin\n\
      wr <= io_enable && !done_r && fsm == 2'd2;\n\
      if (io_reset) done_r <= 1'b0;\n\
      else if (io_enable && !done_r && fsm == 2'd2) done_r <= 1'b1;\n\
      if (!io_enable) fsm <= 2'd0;\n\
      else if (!done_r) fsm <= fsm + 2'd1;\n\
    end\n\
  end\n\
  assign hi = wr ? (h ^ io_echo) : 16'bx;\n\
  assign lo = wr ? l : 16'bx;\n\
  assign hi_valid = wr;\n\
  assign lo_valid = wr;\n\
  assign io_done = done_r;\n\
endmodule\n";
    let c = Candidate {
        kernel: "K5".into(),
        name: "k5_enable".into(),
        top: "qor_k5_enable".into(),
        clock: "clock".into(),
        reset: Some(Reset { port: "reset".into(), active_high: true }),
        protocol: Protocol::EnableDone { enable: "io_enable".into(), done: "io_done".into(), clear: "io_reset".into() },
        inputs: vec![Bind::whole("a", "a"), Bind::whole("b", "b"), Bind::whole("zeta", "zeta")],
        outputs: vec![Bind::whole("hi", "hi").valid_on("hi_valid"), Bind::whole("lo", "lo").valid_on("lo_valid")],
        mems: vec![],
        batch: 1,
        ports: dp(&[
            ("clock", Dir::In, 1),
            ("reset", Dir::In, 1),
            ("io_enable", Dir::In, 1),
            ("io_reset", Dir::In, 1),
            ("io_echo", Dir::In, 16),
            ("io_ready", Dir::In, 1),
            ("a", Dir::In, 16),
            ("b", Dir::In, 16),
            ("zeta", Dir::In, 16),
            ("io_done", Dir::Out, 1),
            ("hi", Dir::Out, 16),
            ("hi_valid", Dir::Out, 1),
            ("lo", Dir::Out, 16),
            ("lo_valid", Dir::Out, 1),
        ]),
    };
    let bfly = logicaffeine_compile::codegen_sva::keccak_rtl::emit_ntt_butterfly_verilog();
    (c, vec![("ntt_butterfly.v".into(), bfly), ("qor_k5_enable.v".into(), v.to_string())])
}

#[test]
fn an_enable_done_candidate_runs_while_enabled_and_is_cleared_between_operations() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K5");
    let (c, files) = enable_done_butterfly();
    check_ports(&k, &c).unwrap();
    for mode in [Mode::Shaped, Mode::Edge] {
        let run = assert_golden(&k, &c, &files, mode, OPS);
        assert_eq!(
            run.steady_cycles_per_op(),
            Some(5.0),
            "three cycles running, the strobed results and the sticky done on the fourth, the clear on the fifth"
        );
    }
    assert!(Protocol::EnableDone { enable: "e".into(), done: "d".into(), clear: "r".into() }.label().contains("enable"));
    let mut swapped = c.clone();
    swapped.protocol = Protocol::EnableDone { enable: "io_done".into(), done: "io_enable".into(), clear: "io_reset".into() };
    assert!(check_ports(&k, &swapped).is_err(), "the enable is an input and the done an output");
}

/// Two butterflies a call, controlled as [`enable_done_butterfly`]: each slot's results written with their own
/// strobes, slot 1's a cycle before slot 0's — as an accelerator's ArgOuts are written in whatever order its
/// schedule reaches them.
fn batched_enable_done_butterfly() -> (Candidate, Vec<(String, String)>) {
    let v = "module qor_k5_enable2(input clock, input reset, input io_enable, input io_reset, input signed [15:0] a0, input signed [15:0] b0, input signed [15:0] z0, input signed [15:0] a1, input signed [15:0] b1, input signed [15:0] z1, output io_done, output [15:0] hi0, output hi0_valid, output [15:0] lo0, output lo0_valid, output [15:0] hi1, output hi1_valid, output [15:0] lo1, output lo1_valid);\n\
  wire signed [15:0] h0, l0, h1, l1;\n\
  ntt_butterfly u0(.a(a0), .b(b0), .zeta(z0), .hi(h0), .lo(l0));\n\
  ntt_butterfly u1(.a(a1), .b(b1), .zeta(z1), .hi(h1), .lo(l1));\n\
  reg [2:0] fsm;\n\
  reg done_r;\n\
  always @(posedge clock) begin\n\
    if (reset || io_reset) begin fsm <= 3'd0; done_r <= 1'b0; end\n\
    else if (io_enable && !done_r) begin\n\
      if (fsm == 3'd4) done_r <= 1'b1;\n\
      fsm <= fsm + 3'd1;\n\
    end\n\
  end\n\
  wire w1 = io_enable && !done_r && fsm == 3'd2;\n\
  wire w0 = io_enable && !done_r && fsm == 3'd3;\n\
  assign hi1 = w1 ? h1 : 16'bx;\n\
  assign lo1 = w1 ? l1 : 16'bx;\n\
  assign hi1_valid = w1;\n\
  assign lo1_valid = w1;\n\
  assign hi0 = w0 ? h0 : 16'bx;\n\
  assign lo0 = w0 ? l0 : 16'bx;\n\
  assign hi0_valid = w0;\n\
  assign lo0_valid = w0;\n\
  assign io_done = done_r;\n\
endmodule\n";
    let slot = |j: u32| {
        let (a, b, z) = (format!("a{j}"), format!("b{j}"), format!("z{j}"));
        (
            vec![Bind::whole(&format!("a@{j}"), &a), Bind::whole(&format!("b@{j}"), &b), Bind::whole(&format!("zeta@{j}"), &z)],
            vec![
                Bind::whole(&format!("hi@{j}"), &format!("hi{j}")).valid_on(&format!("hi{j}_valid")),
                Bind::whole(&format!("lo@{j}"), &format!("lo{j}")).valid_on(&format!("lo{j}_valid")),
            ],
        )
    };
    let ((i0, o0), (i1, o1)) = (slot(0), slot(1));
    let mut ports = dp(&[("clock", Dir::In, 1), ("reset", Dir::In, 1), ("io_enable", Dir::In, 1), ("io_reset", Dir::In, 1), ("io_done", Dir::Out, 1)]);
    for j in 0..2 {
        for n in ["a", "b", "z"] {
            ports.push(DutPort { name: format!("{n}{j}"), dir: Dir::In, bits: 16 });
        }
        for n in ["hi", "lo"] {
            ports.push(DutPort { name: format!("{n}{j}"), dir: Dir::Out, bits: 16 });
            ports.push(DutPort { name: format!("{n}{j}_valid"), dir: Dir::Out, bits: 1 });
        }
    }
    let c = Candidate {
        kernel: "K5".into(),
        name: "k5_enable2".into(),
        top: "qor_k5_enable2".into(),
        clock: "clock".into(),
        reset: Some(Reset { port: "reset".into(), active_high: true }),
        protocol: Protocol::EnableDone { enable: "io_enable".into(), done: "io_done".into(), clear: "io_reset".into() },
        inputs: [i0, i1].concat(),
        outputs: [o0, o1].concat(),
        mems: vec![],
        batch: 2,
        ports,
    };
    let bfly = logicaffeine_compile::codegen_sva::keccak_rtl::emit_ntt_butterfly_verilog();
    (c, vec![("ntt_butterfly.v".into(), bfly), ("qor_k5_enable2.v".into(), v.to_string())])
}

#[test]
fn a_batch_whose_results_are_strobed_captures_each_slot_by_its_own_strobes() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K5");
    let (c, files) = batched_enable_done_butterfly();
    check_ports(&k, &c).unwrap();
    for mode in [Mode::Shaped, Mode::Edge] {
        let run = assert_golden(&k, &c, &files, mode, OPS);
        assert_eq!(run.steady_cycles_per_op(), Some(3.5), "seven cycles a call of two");
    }
    let mut spanning = c.clone();
    spanning.outputs[1].dut = "hi1".into();
    spanning.outputs[1].valid = Some("hi1_valid".into());
    assert!(check_ports(&k, &spanning).is_err(), "one strobed port feeding two slots");
}

#[test]
fn an_input_the_binding_does_not_name_is_held_at_zero() {
    if !common::require_tool("iverilog") {
        return;
    }
    let (c, _) = enable_done_butterfly();
    let k = kernel_of("K5");
    let h = emit_harness(&k, &c, Mode::Shaped);
    for tied in [".io_echo(16'd0)", ".io_ready(1'd0)"] {
        assert!(h.contains(tied), "{tied}\n{h}");
    }
    for bound in [".a(", ".io_enable(", ".io_reset(", ".clock(clk)"] {
        assert_eq!(h.matches(bound).count(), 1, "{bound} is connected once");
    }
}

#[test]
fn the_weight_stationary_gemm_tile_streams_on_its_beat_schedule() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K3");
    let (c, files) = gemm_ws_candidate(gemm_ws_period());
    check_ports(&k, &c).unwrap();
    for mode in [Mode::Shaped, Mode::Edge] {
        let run = assert_golden(&k, &c, &files, mode, 64);
        assert_eq!(
            run.steady_cycles_per_op(),
            Some(gemm_ws_period() as f64 / 8.0),
            "eight rows per {}-cycle call",
            gemm_ws_period()
        );
    }
}

#[test]
fn the_output_stationary_gemm_array_streams_a_row_per_cycle() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K3");
    let (c, files) = gemm_os_candidate();
    check_ports(&k, &c).unwrap();
    for mode in [Mode::Shaped, Mode::Edge] {
        let run = assert_golden(&k, &c, &files, mode, 64);
        assert_eq!(run.steady_cycles_per_op(), Some(1.0), "a tile of eight rows every eight beats");
    }
}

#[test]
fn a_weight_stationary_call_one_beat_shorter_overwrites_weights_still_in_use() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K3");
    let (mut c, files) = gemm_ws_candidate(gemm_ws_period() - 1);
    c.name = "k3_logos_ws_short".into();
    let dir = tmp();
    let run = simulate(dir.path(), &k, &c, &files, Mode::Shaped, 64).expect("the run finishes");
    assert_ne!(
        run.misr,
        golden_signature(&k, Mode::Shaped, 64),
        "the drain is part of the period: loading the next tile's weights a beat early corrupts the last row"
    );
}

#[test]
fn a_batched_start_done_candidate_folds_its_slots_in_operation_order() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K5");
    let mut v = String::from("module qor_k5_batch4(input clk, input rst, input start");
    for j in 0..4 {
        v.push_str(&format!(", input signed [15:0] a{j}, input signed [15:0] b{j}, input signed [15:0] zeta{j}, output reg [15:0] hi{j}, output reg [15:0] lo{j}"));
    }
    v.push_str(", output reg done);\n  reg [2:0] n;\n  reg busy;\n");
    for j in 0..4 {
        v.push_str(&format!("  wire signed [15:0] h{j}, l{j};\n  ntt_butterfly u{j}(.a(a{j}), .b(b{j}), .zeta(zeta{j}), .hi(h{j}), .lo(l{j}));\n"));
    }
    v.push_str("  always @(posedge clk) begin\n    done <= 1'b0;\n    if (rst) begin busy <= 1'b0; n <= 3'd0; end\n    else if (start && !busy) begin busy <= 1'b1; n <= 3'd3;\n");
    for j in 0..4 {
        v.push_str(&format!("      hi{j} <= h{j}; lo{j} <= l{j};\n"));
    }
    v.push_str("    end else if (busy) begin if (n == 3'd0) begin busy <= 1'b0; done <= 1'b1; end else n <= n - 3'd1; end\n  end\nendmodule\n");
    let mut ports = vec![
        DutPort { name: "clk".into(), dir: Dir::In, bits: 1 },
        DutPort { name: "rst".into(), dir: Dir::In, bits: 1 },
        DutPort { name: "start".into(), dir: Dir::In, bits: 1 },
        DutPort { name: "done".into(), dir: Dir::Out, bits: 1 },
    ];
    let (mut inputs, mut outputs) = (Vec::new(), Vec::new());
    for j in 0..4 {
        for p in ["a", "b", "zeta"] {
            ports.push(DutPort { name: format!("{p}{j}"), dir: Dir::In, bits: 16 });
            inputs.push(Bind::whole(&format!("{p}@{j}"), &format!("{p}{j}")));
        }
        for p in ["hi", "lo"] {
            ports.push(DutPort { name: format!("{p}{j}"), dir: Dir::Out, bits: 16 });
            outputs.push(Bind::whole(&format!("{p}@{j}"), &format!("{p}{j}")));
        }
    }
    let c = Candidate {
        kernel: "K5".into(),
        name: "k5_batch4".into(),
        top: "qor_k5_batch4".into(),
        clock: "clk".into(),
        reset: Some(Reset { port: "rst".into(), active_high: true }),
        protocol: Protocol::StartDone { start: "start".into(), done: "done".into() },
        inputs,
        outputs,
        mems: vec![],
        batch: 4,
        ports,
    };
    check_ports(&k, &c).unwrap();
    let bfly = logicaffeine_compile::codegen_sva::keccak_rtl::emit_ntt_butterfly_verilog();
    let files = vec![("ntt_butterfly.v".to_string(), bfly), ("qor_k5_batch4.v".to_string(), v)];
    for mode in [Mode::Shaped, Mode::Edge] {
        assert_golden(&k, &c, &files, mode, OPS);
    }
}

#[test]
fn one_wrong_result_fails_the_gate() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K5");
    let (mut c, mut files) = logos_candidate("K5").unwrap();
    let bad = files[0].1.replace("assign lo = a - t;", "assign lo = (a[3:0] == 4'd5) ? a - t + 16'sd1 : a - t;");
    assert_ne!(bad, files[0].1, "the planted defect is in the emitted butterfly");
    files[0].1 = bad;
    c.name = "k5_planted".into();
    let stim = stimulus(&k, Mode::Shaped, OPS);
    assert!(stim.iter().any(|op| op[0][0] & 0xf == 5), "the stimulus reaches the planted operand");
    let ops = OPS;
    let dir = tmp();
    let run = simulate(dir.path(), &k, &c, &files, Mode::Shaped, ops).expect("the run finishes");
    assert_ne!(run.misr, golden_signature(&k, Mode::Shaped, ops), "a single wrong result moves the signature");
}

#[test]
fn an_unknown_result_bit_is_refused() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K5");
    let (mut c, mut files) = logos_candidate("K5").unwrap();
    files[0].1 = files[0].1.replace("assign hi = a + t;", "assign hi = {a[15:1] + t[15:1], 1'bx};");
    c.name = "k5_unknown".into();
    let dir = tmp();
    match simulate(dir.path(), &k, &c, &files, Mode::Shaped, 50) {
        Err(QorError::Unknown { .. }) => {}
        other => panic!("an X result must be refused, got {other:?}"),
    }
}

/// The ports of `top` as yosys reads them from `files`.
fn yosys_ports(dir: &Path, files: &[(String, String)], top: &str) -> Vec<DutPort> {
    let mut reads = String::new();
    for (name, text) in files {
        let p = dir.join(name);
        std::fs::write(&p, text).unwrap();
        reads.push_str(&format!("read_verilog -sv {}; ", p.display()));
    }
    let json = dir.join("ports.json");
    let out = Command::new("yosys")
        .arg("-q")
        .arg("-p")
        .arg(format!("{reads}hierarchy -top {top}; proc; write_json {}", json.display()))
        .output()
        .expect("run yosys");
    assert!(out.status.success(), "yosys: {}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    let ports = v["modules"][top]["ports"].as_object().expect("the top module's ports");
    ports
        .iter()
        .map(|(name, p)| DutPort {
            name: name.clone(),
            dir: if p["direction"] == "output" { Dir::Out } else { Dir::In },
            bits: p["bits"].as_array().map(|b| b.len() as u32).unwrap_or(0),
        })
        .collect()
}

fn sorted(mut ports: Vec<DutPort>) -> Vec<DutPort> {
    ports.sort_by(|a, b| a.name.cmp(&b.name));
    ports
}

#[test]
fn every_candidate_declares_the_ports_yosys_reads_and_its_binding_fits_them() {
    if !common::require_tool("yosys") {
        return;
    }
    let mut all: Vec<(Candidate, Vec<(String, String)>)> = ["K1", "K2", "K3", "K4", "K5", "K6"]
        .iter()
        .flat_map(|id| logos_candidates(id).unwrap())
        .collect();
    all.push(delayed_butterfly(2));
    all.push(stalling_butterfly());
    all.push(pipelined_butterfly());
    all.push(go_done_butterfly());
    all.push(staggered_butterfly());
    for (c, files) in all {
        let k = kernel_of(&c.kernel);
        let dir = tmp();
        let read = yosys_ports(dir.path(), &files, &c.top);
        assert_eq!(sorted(c.ports.clone()), sorted(read), "{}: declared ports vs the netlist", c.name);
        check_ports(&k, &c).unwrap_or_else(|e| panic!("{}: {e}", c.name));
    }
}

#[test]
fn the_harness_itself_elaborates_with_the_dut_kept_whole() {
    if !common::require_tool("yosys") {
        return;
    }
    let k = kernel_of("K5");
    let (c, mut files) = logos_candidate("K5").unwrap();
    files.push((format!("{}.v", harness_module(&c)), emit_harness(&k, &c, Mode::Shaped)));
    let dir = tmp();
    let mut reads = String::new();
    for (name, text) in &files {
        let p = dir.path().join(name);
        std::fs::write(&p, text).unwrap();
        reads.push_str(&format!("read_verilog -sv {}; ", p.display()));
    }
    let out = Command::new("yosys")
        .arg("-p")
        .arg(format!("{reads}synth -flatten -top {}; stat", harness_module(&c)))
        .output()
        .expect("run yosys");
    let log = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "yosys: {}", String::from_utf8_lossy(&out.stderr));
    assert!(
        log.contains("=== qor_k5_logos ==="),
        "the kept DUT survives flattening as its own module:\n{log}"
    );
}

/// A go/done GEMM tile in the shape Calyx gives a Dahlia `decl x: bit<N>[8][8]`: every operand and
/// result memory is a `seq_mem` addressed by row (`addr0`) and column (`addr1`). The component reads
/// `a[i][r]` and `w[r][c]` together, accumulates the eight products, and writes `o[i][c]`.
fn two_dimensional_gemm() -> (Candidate, Vec<(String, String)>) {
    let v = "module qor_k3_mem2d(input clk, input reset, input go, output done,\n\
  output [3:0] a_addr0, output [3:0] a_addr1, output a_content_en, output a_write_en, output [7:0] a_write_data, input [7:0] a_read_data, input a_done,\n\
  output [3:0] w_addr0, output [3:0] w_addr1, output w_content_en, output w_write_en, output [7:0] w_write_data, input [7:0] w_read_data, input w_done,\n\
  output [3:0] o_addr0, output [3:0] o_addr1, output o_content_en, output o_write_en, output [31:0] o_write_data, input [31:0] o_read_data, input o_done);\n\
  localparam IDLE = 3'd0, RD = 3'd1, WT = 3'd2, WR = 3'd3, WW = 3'd4, FIN = 3'd5;\n\
  reg [2:0] st;\n\
  reg [3:0] i, c, r;\n\
  reg signed [31:0] acc;\n\
  assign a_addr0 = i; assign a_addr1 = r; assign w_addr0 = r; assign w_addr1 = c;\n\
  assign a_content_en = st == RD; assign w_content_en = st == RD;\n\
  assign a_write_en = 1'b0; assign w_write_en = 1'b0; assign a_write_data = 8'd0; assign w_write_data = 8'd0;\n\
  assign o_addr0 = i; assign o_addr1 = c; assign o_content_en = st == WR; assign o_write_en = st == WR; assign o_write_data = acc;\n\
  assign done = st == FIN;\n\
  always @(posedge clk) begin\n\
    if (reset) st <= IDLE;\n\
    else case (st)\n\
      IDLE: if (go) begin i <= 4'd0; c <= 4'd0; r <= 4'd0; acc <= 32'sd0; st <= RD; end\n\
      RD: st <= WT;\n\
      WT: if (a_done && w_done) begin\n\
            acc <= acc + $signed(a_read_data) * $signed(w_read_data);\n\
            if (r == 4'd7) st <= WR; else begin r <= r + 4'd1; st <= RD; end\n\
          end\n\
      WR: st <= WW;\n\
      WW: if (o_done) begin\n\
            r <= 4'd0; acc <= 32'sd0;\n\
            if (c == 4'd7) begin c <= 4'd0; if (i == 4'd7) st <= FIN; else begin i <= i + 4'd1; st <= RD; end end\n\
            else begin c <= c + 4'd1; st <= RD; end\n\
          end\n\
      FIN: st <= IDLE;\n\
      default: st <= IDLE;\n\
    endcase\n\
  end\n\
endmodule\n";
    let mut ports = vec![
        DutPort { name: "clk".into(), dir: Dir::In, bits: 1 },
        DutPort { name: "reset".into(), dir: Dir::In, bits: 1 },
        DutPort { name: "go".into(), dir: Dir::In, bits: 1 },
        DutPort { name: "done".into(), dir: Dir::Out, bits: 1 },
    ];
    for (p, width) in [("a", 8), ("w", 8), ("o", 32)] {
        for (suffix, dir, bits) in [
            ("addr0", Dir::Out, 4),
            ("addr1", Dir::Out, 4),
            ("content_en", Dir::Out, 1),
            ("write_en", Dir::Out, 1),
            ("write_data", Dir::Out, width),
            ("read_data", Dir::In, width),
            ("done", Dir::In, 1),
        ] {
            ports.push(DutPort { name: format!("{p}_{suffix}"), dir, bits });
        }
    }
    let rows = |p: &str| (0..8).map(|j| format!("{p}@{j}")).collect::<Vec<_>>();
    let c = Candidate {
        kernel: "K3".into(),
        name: "k3_mem2d".into(),
        top: "qor_k3_mem2d".into(),
        clock: "clk".into(),
        reset: Some(Reset { port: "reset".into(), active_high: true }),
        protocol: Protocol::GoDone { go: "go".into(), done: "done".into() },
        inputs: vec![],
        outputs: vec![],
        mems: vec![
            MemBind::seq_mem_rows(&rows("a"), "a", 8).shaped(&[8, 8]),
            MemBind::seq_mem_rows(&["w@0".to_string()], "w", 64).shaped(&[8, 8]),
            MemBind::seq_mem_rows(&rows("o"), "o", 8).shaped(&[8, 8]),
        ],
        batch: 8,
        ports,
    };
    (c, vec![("qor_k3_mem2d.v".into(), v.to_string())])
}

#[test]
fn a_two_dimensional_calyx_memory_is_addressed_row_major() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K3");
    let (c, files) = two_dimensional_gemm();
    check_ports(&k, &c).unwrap();
    for mode in [Mode::Shaped, Mode::Edge] {
        assert_golden(&k, &c, &files, mode, 64);
    }
}

#[test]
fn a_two_dimensional_memory_read_column_major_fails_the_gate() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K3");
    let (c, files) = two_dimensional_gemm();
    let swapped: Vec<(String, String)> = files
        .into_iter()
        .map(|(n, t)| (n, t.replace("assign w_addr0 = r; assign w_addr1 = c;", "assign w_addr0 = c; assign w_addr1 = r;")))
        .collect();
    assert!(swapped[0].1.contains("assign w_addr0 = c;"), "the mutation applies");
    let dir = tmp();
    let run = simulate(dir.path(), &k, &c, &swapped, Mode::Shaped, 64).unwrap();
    assert_eq!(run.ops, 64);
    assert_ne!(run.misr, golden_signature(&k, Mode::Shaped, 64), "reading W transposed is a different GEMM");
}

/// A butterfly on a two-beat call whose `b` and `zeta` latch with their own call and whose `a`
/// arrives a call later — the shape of a design that loads one call's weights while it computes on
/// the previous call's activations. Call n-1's result is registered at the end of call n's beat 0.
fn lagged_butterfly(lag_a: bool) -> (Candidate, Vec<(String, String)>) {
    let v = "module qor_k5_lag(input clk, input load, input signed [15:0] a_prev, input signed [15:0] b_now, input signed [15:0] zeta_now, output reg [15:0] hi, output reg [15:0] lo);\n\
  reg signed [15:0] b_q, z_q;\n\
  wire signed [15:0] h, l;\n\
  ntt_butterfly u(.a(a_prev), .b(b_q), .zeta(z_q), .hi(h), .lo(l));\n\
  always @(posedge clk) if (load) begin hi <= h; lo <= l; b_q <= b_now; z_q <= zeta_now; end\n\
endmodule\n";
    let a = if lag_a {
        DriveValue::Previous { contract: "a".into(), lsb: 0, width: 16 }
    } else {
        DriveValue::Slice { contract: "a".into(), lsb: 0, width: 16 }
    };
    let c = Candidate {
        kernel: "K5".into(),
        name: "k5_lag".into(),
        top: "qor_k5_lag".into(),
        clock: "clk".into(),
        reset: None,
        protocol: Protocol::Beats {
            period: 2,
            drives: vec![
                Drive { beat: 0, dut: "load".into(), value: DriveValue::Const(1) },
                Drive { beat: 0, dut: "a_prev".into(), value: a },
                Drive { beat: 0, dut: "b_now".into(), value: DriveValue::Slice { contract: "b".into(), lsb: 0, width: 16 } },
                Drive { beat: 0, dut: "zeta_now".into(), value: DriveValue::Slice { contract: "zeta".into(), lsb: 0, width: 16 } },
            ],
            captures: vec![3],
        },
        inputs: vec![],
        outputs: vec![Bind::whole("hi", "hi"), Bind::whole("lo", "lo")],
        mems: vec![],
        batch: 1,
        ports: vec![
            DutPort { name: "clk".into(), dir: Dir::In, bits: 1 },
            DutPort { name: "load".into(), dir: Dir::In, bits: 1 },
            DutPort { name: "a_prev".into(), dir: Dir::In, bits: 16 },
            DutPort { name: "b_now".into(), dir: Dir::In, bits: 16 },
            DutPort { name: "zeta_now".into(), dir: Dir::In, bits: 16 },
            DutPort { name: "hi".into(), dir: Dir::Out, bits: 16 },
            DutPort { name: "lo".into(), dir: Dir::Out, bits: 16 },
        ],
    };
    let bfly = logicaffeine_compile::codegen_sva::keccak_rtl::emit_ntt_butterfly_verilog();
    (c, vec![("ntt_butterfly.v".into(), bfly), ("qor_k5_lag.v".into(), v.to_string())])
}

#[test]
fn a_drive_of_the_previous_calls_operands_meets_a_call_that_overlaps_the_next() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K5");
    let (c, files) = lagged_butterfly(true);
    for mode in [Mode::Shaped, Mode::Edge] {
        let run = assert_golden(&k, &c, &files, mode, OPS);
        assert_eq!(run.steady_cycles_per_op(), Some(2.0), "one call every two cycles");
    }
    let (c, files) = lagged_butterfly(false);
    let dir = tmp();
    let run = simulate(dir.path(), &k, &c, &files, Mode::Shaped, OPS).unwrap();
    assert_ne!(run.misr, golden_signature(&k, Mode::Shaped, OPS), "the same `a` driven without the lag pairs it with the wrong call");
}

/// A butterfly on a four-beat call of four operations that hands each call's results back last first,
/// during the next call: slot j of a call is on the outputs `8 - j` cycles after its beat 0.
fn reversing_butterfly() -> (Candidate, Vec<(String, String)>) {
    let v = "module qor_k5_rev(input clk, input rst, input first, input signed [15:0] a, input signed [15:0] b, input signed [15:0] zeta, output reg [15:0] hi, output reg [15:0] lo);\n\
  wire signed [15:0] h, l;\n\
  ntt_butterfly u(.a(a), .b(b), .zeta(zeta), .hi(h), .lo(l));\n\
  reg [1:0] t;\n\
  reg bank;\n\
  reg [15:0] hs [0:7];\n\
  reg [15:0] ls [0:7];\n\
  wire [1:0] beat = first ? 2'd0 : t + 2'd1;\n\
  wire wb = first ? !bank : bank;\n\
  always @(posedge clk) begin\n\
    t <= beat;\n\
    bank <= rst ? 1'b0 : wb;\n\
    hs[{wb, beat}] <= h;\n\
    ls[{wb, beat}] <= l;\n\
    hi <= hs[{!wb, 2'd3 - beat}];\n\
    lo <= ls[{!wb, 2'd3 - beat}];\n\
  end\n\
endmodule\n";
    let mut drives = vec![Drive { beat: 0, dut: "first".into(), value: DriveValue::Const(1) }];
    for t in 0..4u32 {
        for p in ["a", "b", "zeta"] {
            drives.push(Drive { beat: t, dut: p.into(), value: DriveValue::Slice { contract: format!("{p}@{t}"), lsb: 0, width: 16 } });
        }
    }
    let mut ports = vec![
        DutPort { name: "clk".into(), dir: Dir::In, bits: 1 },
        DutPort { name: "rst".into(), dir: Dir::In, bits: 1 },
        DutPort { name: "first".into(), dir: Dir::In, bits: 1 },
    ];
    for p in ["a", "b", "zeta"] {
        ports.push(DutPort { name: p.into(), dir: Dir::In, bits: 16 });
    }
    ports.push(DutPort { name: "hi".into(), dir: Dir::Out, bits: 16 });
    ports.push(DutPort { name: "lo".into(), dir: Dir::Out, bits: 16 });
    let c = Candidate {
        kernel: "K5".into(),
        name: "k5_rev".into(),
        top: "qor_k5_rev".into(),
        clock: "clk".into(),
        reset: Some(Reset { port: "rst".into(), active_high: true }),
        protocol: Protocol::Beats { period: 4, drives, captures: (0..4).map(|j| 8 - j).collect() },
        inputs: vec![],
        outputs: (0..4).flat_map(|j| [Bind::whole(&format!("hi@{j}"), "hi"), Bind::whole(&format!("lo@{j}"), "lo")]).collect(),
        mems: vec![],
        batch: 4,
        ports,
    };
    let bfly = logicaffeine_compile::codegen_sva::keccak_rtl::emit_ntt_butterfly_verilog();
    (c, vec![("ntt_butterfly.v".into(), bfly), ("qor_k5_rev.v".into(), v.to_string())])
}

#[test]
fn results_handed_back_last_first_are_folded_in_operation_order_at_full_rate() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K5");
    let (c, files) = reversing_butterfly();
    check_ports(&k, &c).unwrap();
    for mode in [Mode::Shaped, Mode::Edge] {
        let run = assert_golden(&k, &c, &files, mode, OPS);
        assert_eq!(run.steady_cycles_per_op(), Some(1.0), "four results every four cycles");
    }
}

/// The output-stationary array with 20-bit accumulators behind a wrapper whose result ports carry only
/// those 20 bits, the way an array's own outputs are as narrow as its accumulators.
fn narrow_os_array() -> (Candidate, Vec<(String, String)>) {
    let (mut c, mut files) = gemm_os_candidate_acc(20);
    let d = GEMM_DIM;
    let mut v = String::from("module qor_k3_os_narrow(input clk, input rst, input first");
    for i in 0..d {
        v.push_str(&format!(", input [63:0] aw{i}, input [63:0] wn{i}"));
    }
    for i in 0..d {
        for j in 0..d {
            v.push_str(&format!(", output [19:0] n{i}_{j}"));
        }
    }
    v.push_str(");\n");
    let mut conns = vec![".clk(clk)".to_string(), ".rst(rst)".into(), ".first(first)".into()];
    for i in 0..d {
        conns.push(format!(".aw{i}(aw{i})"));
        conns.push(format!(".wn{i}(wn{i})"));
        for j in 0..d {
            v.push_str(&format!("  wire [63:0] r{i}_{j};\n  assign n{i}_{j} = r{i}_{j}[19:0];\n"));
            conns.push(format!(".res{i}_{j}(r{i}_{j})"));
        }
    }
    v.push_str(&format!("  {} u({});\nendmodule\n", c.top, conns.join(", ")));
    for p in c.ports.iter_mut().filter(|p| p.name.starts_with("res")) {
        p.name = p.name.replacen("res", "n", 1);
        p.bits = 20;
    }
    for b in c.outputs.iter_mut() {
        *b = Bind::slice(&b.contract, b.lsb, b.width, &b.dut.replacen("res", "n", 1)).sign_extended(20);
    }
    c.name = "k3_os_narrow".into();
    c.top = "qor_k3_os_narrow".into();
    files.push(("qor_k3_os_narrow.v".into(), v));
    (c, files)
}

#[test]
fn twenty_signed_bits_extend_to_every_thirty_two_bit_sum() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K3");
    let (c, files) = narrow_os_array();
    check_ports(&k, &c).unwrap();
    for mode in [Mode::Shaped, Mode::Edge] {
        assert_golden(&k, &c, &files, mode, 64);
    }
}

/// A device in the shape of a GPU behind its memory bus: setup writes on its DCR port give it the line
/// of its operands and the line of its results; `start` launches it; while `busy` it reads the operand
/// line, computes the eight K2 polynomials, and — like a write-back cache — holds the results until a
/// DCR read of register 0 (the flush) writes them out and answers.
fn k2_device(flush: Option<u32>) -> (Candidate, Vec<(String, String)>) {
    let v = "module qor_k2_dev(input clk, input reset,\n\
  output reg mem_req_valid, output reg mem_req_rw, output reg [63:0] mem_req_byteen, output reg [15:0] mem_req_addr,\n\
  output reg [511:0] mem_req_data, output reg [3:0] mem_req_tag, input mem_req_ready,\n\
  input mem_rsp_valid, input [511:0] mem_rsp_data, input [3:0] mem_rsp_tag, output mem_rsp_ready,\n\
  input dcr_req_valid, input dcr_req_rw, input [3:0] dcr_req_addr, input [31:0] dcr_req_data, output reg dcr_rsp_valid,\n\
  input start, output busy);\n\
  reg [15:0] in_line, out_line;\n\
  reg [511:0] pend;\n\
  reg dirty;\n\
  reg [2:0] st;\n\
  reg [1:0] fw;\n\
  integer i;\n\
  assign mem_rsp_ready = 1'b1;\n\
  assign busy = (st == 3'd1) || (st == 3'd2) || (st == 3'd3);\n\
  function [63:0] horner(input [63:0] x);\n\
    horner = ((((x << 1) + 64'd5) * x - 64'd3) * x) + 64'd100;\n\
  endfunction\n\
  always @(posedge clk) begin\n\
    dcr_rsp_valid <= 1'b0;\n\
    if (reset) begin st <= 3'd0; dirty <= 1'b0; mem_req_valid <= 1'b0; end\n\
    else begin\n\
      if (dcr_req_valid && dcr_req_rw && dcr_req_addr == 4'd1) in_line <= dcr_req_data[15:0];\n\
      if (dcr_req_valid && dcr_req_rw && dcr_req_addr == 4'd2) out_line <= dcr_req_data[15:0];\n\
      case (st)\n\
        3'd0: begin\n\
          if (start) st <= 3'd1;\n\
          else if (dcr_req_valid && !dcr_req_rw && dcr_req_addr == 4'd0) begin\n\
            if (dirty) begin mem_req_valid <= 1'b1; mem_req_rw <= 1'b1; mem_req_addr <= out_line; mem_req_data <= pend; mem_req_byteen <= {64{1'b1}}; mem_req_tag <= 4'd3; st <= 3'd4; end\n\
            else dcr_rsp_valid <= 1'b1;\n\
          end\n\
        end\n\
        3'd1: begin mem_req_valid <= 1'b1; mem_req_rw <= 1'b0; mem_req_addr <= in_line; mem_req_tag <= 4'd9; st <= 3'd2; end\n\
        3'd2: begin\n\
          if (mem_req_valid && mem_req_ready) mem_req_valid <= 1'b0;\n\
          if (mem_rsp_valid && mem_rsp_tag == 4'd9) begin\n\
            for (i = 0; i < 8; i = i + 1) pend[i*64 +: 64] <= horner(mem_rsp_data[i*64 +: 64]);\n\
            st <= 3'd3;\n\
          end\n\
        end\n\
        3'd3: begin dirty <= 1'b1; st <= 3'd0; end\n\
        3'd4: if (mem_req_ready) begin mem_req_valid <= 1'b0; fw <= 2'd2; st <= 3'd5; end\n\
        3'd5: if (fw == 2'd0) begin dcr_rsp_valid <= 1'b1; dirty <= 1'b0; st <= 3'd0; end else fw <= fw - 2'd1;\n\
        default: st <= 3'd0;\n\
      endcase\n\
    end\n\
  end\n\
endmodule\n";
    let rows = |p: &str| (0..8).map(|j| format!("{p}@{j}")).collect::<Vec<_>>();
    let mut ports = Vec::new();
    for (name, dir, bits) in [
        ("clk", Dir::In, 1),
        ("reset", Dir::In, 1),
        ("mem_req_valid", Dir::Out, 1),
        ("mem_req_rw", Dir::Out, 1),
        ("mem_req_byteen", Dir::Out, 64),
        ("mem_req_addr", Dir::Out, 16),
        ("mem_req_data", Dir::Out, 512),
        ("mem_req_tag", Dir::Out, 4),
        ("mem_req_ready", Dir::In, 1),
        ("mem_rsp_valid", Dir::In, 1),
        ("mem_rsp_data", Dir::In, 512),
        ("mem_rsp_tag", Dir::In, 4),
        ("mem_rsp_ready", Dir::Out, 1),
        ("dcr_req_valid", Dir::In, 1),
        ("dcr_req_rw", Dir::In, 1),
        ("dcr_req_addr", Dir::In, 4),
        ("dcr_req_data", Dir::In, 32),
        ("dcr_rsp_valid", Dir::Out, 1),
        ("start", Dir::In, 1),
        ("busy", Dir::Out, 1),
    ] {
        ports.push(DutPort { name: name.into(), dir, bits });
    }
    let c = Candidate {
        kernel: "K2".into(),
        name: "k2_dev".into(),
        top: "qor_k2_dev".into(),
        clock: "clk".into(),
        reset: Some(Reset { port: "reset".into(), active_high: true }),
        protocol: Protocol::Launch {
            start: "start".into(),
            busy: "busy".into(),
            dcr: DcrBus {
                valid: "dcr_req_valid".into(),
                rw: "dcr_req_rw".into(),
                addr: "dcr_req_addr".into(),
                data: "dcr_req_data".into(),
                rsp_valid: "dcr_rsp_valid".into(),
            },
            setup: vec![DcrWrite { addr: 1, data: 0x10 }, DcrWrite { addr: 2, data: 0x20 }],
            flush,
            bus: LineBus {
                ports: 1,
                line_bytes: 64,
                addr_bits: 16,
                tag_bits: 4,
                msb_first: true,
                prefix: "mem".into(),
                image: vec![],
                regions: vec![
                    LineRegion { base_line: 0x10, contract: rows("x"), words_per_port: 1, stride_bits: 0 },
                    LineRegion { base_line: 0x20, contract: rows("y"), words_per_port: 1, stride_bits: 0 },
                ],
            },
        },
        inputs: vec![],
        outputs: vec![],
        mems: vec![],
        batch: 8,
        ports,
    };
    (c, vec![("qor_k2_dev.v".into(), v.to_string())])
}

#[test]
fn a_device_launched_over_its_memory_bus_is_flushed_before_its_results_are_read() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K2");
    let (c, files) = k2_device(Some(0));
    check_ports(&k, &c).unwrap();
    for mode in [Mode::Shaped, Mode::Edge] {
        assert_golden(&k, &c, &files, mode, 64);
    }
    let (c, files) = k2_device(None);
    let dir = tmp();
    let run = simulate(dir.path(), &k, &c, &files, Mode::Shaped, 64).unwrap();
    assert_ne!(run.misr, golden_signature(&k, Mode::Shaped, 64), "without the flush the results never reach memory");
}

/// A device holding two K5 butterflies per call in 64-bit memory words: it reads one line of a, b and
/// zeta for each (sign-extended into their words), and at the flush writes hi and lo of each,
/// zero-extended, into the result line.
fn k5_device() -> (Candidate, Vec<(String, String)>) {
    let (mut c, _) = k2_device(Some(0));
    let v = "module qor_k5_dev(input clk, input reset,\n\
  output reg mem_req_valid, output reg mem_req_rw, output reg [63:0] mem_req_byteen, output reg [15:0] mem_req_addr,\n\
  output reg [511:0] mem_req_data, output reg [3:0] mem_req_tag, input mem_req_ready,\n\
  input mem_rsp_valid, input [511:0] mem_rsp_data, input [3:0] mem_rsp_tag, output mem_rsp_ready,\n\
  input dcr_req_valid, input dcr_req_rw, input [3:0] dcr_req_addr, input [31:0] dcr_req_data, output reg dcr_rsp_valid,\n\
  input start, output busy);\n\
  reg [15:0] in_line, out_line;\n\
  reg [511:0] got, pend;\n\
  reg dirty;\n\
  reg [2:0] st;\n\
  wire [15:0] h0, l0, h1, l1;\n\
  ntt_butterfly u0(.a(got[15:0]), .b(got[79:64]), .zeta(got[143:128]), .hi(h0), .lo(l0));\n\
  ntt_butterfly u1(.a(got[207:192]), .b(got[271:256]), .zeta(got[335:320]), .hi(h1), .lo(l1));\n\
  assign mem_rsp_ready = 1'b1;\n\
  assign busy = (st == 3'd1) || (st == 3'd2) || (st == 3'd3);\n\
  always @(posedge clk) begin\n\
    dcr_rsp_valid <= 1'b0;\n\
    if (reset) begin st <= 3'd0; dirty <= 1'b0; mem_req_valid <= 1'b0; end\n\
    else begin\n\
      if (dcr_req_valid && dcr_req_rw && dcr_req_addr == 4'd1) in_line <= dcr_req_data[15:0];\n\
      if (dcr_req_valid && dcr_req_rw && dcr_req_addr == 4'd2) out_line <= dcr_req_data[15:0];\n\
      case (st)\n\
        3'd0: begin\n\
          if (start) st <= 3'd1;\n\
          else if (dcr_req_valid && !dcr_req_rw && dcr_req_addr == 4'd0) begin\n\
            if (dirty) begin mem_req_valid <= 1'b1; mem_req_rw <= 1'b1; mem_req_addr <= out_line; mem_req_data <= pend; mem_req_byteen <= {64{1'b1}}; mem_req_tag <= 4'd3; st <= 3'd4; end\n\
            else dcr_rsp_valid <= 1'b1;\n\
          end\n\
        end\n\
        3'd1: begin mem_req_valid <= 1'b1; mem_req_rw <= 1'b0; mem_req_addr <= in_line; mem_req_tag <= 4'd9; st <= 3'd2; end\n\
        3'd2: begin\n\
          if (mem_req_valid && mem_req_ready) mem_req_valid <= 1'b0;\n\
          if (mem_rsp_valid && mem_rsp_tag == 4'd9) begin got <= mem_rsp_data; st <= 3'd3; end\n\
        end\n\
        3'd3: begin pend <= {256'd0, 48'd0, l1, 48'd0, h1, 48'd0, l0, 48'd0, h0}; dirty <= 1'b1; st <= 3'd0; end\n\
        3'd4: if (mem_req_ready) begin mem_req_valid <= 1'b0; st <= 3'd5; end\n\
        3'd5: begin dcr_rsp_valid <= 1'b1; dirty <= 1'b0; st <= 3'd0; end\n\
        default: st <= 3'd0;\n\
      endcase\n\
    end\n\
  end\n\
endmodule\n";
    let ops = |names: &[&str]| (0..2).flat_map(|j| names.iter().map(move |n| format!("{n}@{j}"))).collect::<Vec<_>>();
    if let Protocol::Launch { bus, .. } = &mut c.protocol {
        bus.regions = vec![
            LineRegion { base_line: 0x10, contract: ops(&["a", "b", "zeta"]), words_per_port: 1, stride_bits: 64 },
            LineRegion { base_line: 0x20, contract: ops(&["hi", "lo"]), words_per_port: 1, stride_bits: 64 },
        ];
    }
    c.kernel = "K5".into();
    c.name = "k5_dev".into();
    c.top = "qor_k5_dev".into();
    c.batch = 2;
    let bfly = logicaffeine_compile::codegen_sva::keccak_rtl::emit_ntt_butterfly_verilog();
    (c, vec![("ntt_butterfly.v".into(), bfly), ("qor_k5_dev.v".into(), v.to_string())])
}

#[test]
fn narrow_contract_ports_take_whole_memory_words_by_the_region_stride() {
    if !common::require_tool("iverilog") {
        return;
    }
    let k = kernel_of("K5");
    let (c, files) = k5_device();
    check_ports(&k, &c).unwrap();
    for mode in [Mode::Shaped, Mode::Edge] {
        assert_golden(&k, &c, &files, mode, 64);
    }
}

#[test]
fn a_fold_wrapper_keeps_its_design_whole_and_its_signature_moves() {
    if !common::require_tool("iverilog") || !common::require_tool("yosys") {
        return;
    }
    let ctrl = "module toy_ctrl(input clk, input rst, input [7:0] x, input [40:0] y, output reg [9:0] acc, output [40:0] z);\n\
  always @(posedge clk) if (rst) acc <= 10'd0; else acc <= acc + {2'd0, x};\n\
  assign z = y ^ {31'd0, acc};\n\
endmodule\n";
    let ports = vec![
        DutPort { name: "clk".into(), dir: Dir::In, bits: 1 },
        DutPort { name: "rst".into(), dir: Dir::In, bits: 1 },
        DutPort { name: "x".into(), dir: Dir::In, bits: 8 },
        DutPort { name: "y".into(), dir: Dir::In, bits: 41 },
        DutPort { name: "acc".into(), dir: Dir::Out, bits: 10 },
        DutPort { name: "z".into(), dir: Dir::Out, bits: 41 },
    ];
    let wrap = emit_fold_wrapper("toy", "toy_ctrl", "clk", Some(&Reset { port: "rst".into(), active_high: true }), &ports);
    let dir = tmp();
    std::fs::write(dir.path().join("toy.v"), ctrl).unwrap();
    std::fs::write(dir.path().join("wrap.v"), &wrap).unwrap();
    std::fs::write(
        dir.path().join("tb.v"),
        "module tb; reg clk = 0, rst = 1; wire [15:0] sig; qorsyn_toy w(.clk(clk), .rst(rst), .sig(sig));\n\
         always #5 clk = ~clk; integer i; reg [15:0] first;\n\
         initial begin repeat (3) @(posedge clk); rst = 0; repeat (5) @(posedge clk); first = sig; repeat (20) @(posedge clk);\n\
         if (^sig === 1'bx) $display(\"FOLD X\"); else if (sig == first) $display(\"FOLD STUCK\"); else $display(\"FOLD MOVES\"); $finish; end\nendmodule\n",
    )
    .unwrap();
    let exe = dir.path().join("sim");
    let b = Command::new("iverilog")
        .args(["-g2012", "-s", "tb", "-o"])
        .arg(&exe)
        .args([dir.path().join("toy.v"), dir.path().join("wrap.v"), dir.path().join("tb.v")])
        .output()
        .unwrap();
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    let run = String::from_utf8_lossy(&Command::new("vvp").arg("-n").arg(&exe).output().unwrap().stdout).to_string();
    assert!(run.contains("FOLD MOVES"), "{run}");
    let out = Command::new("yosys")
        .arg("-p")
        .arg(format!(
            "read_verilog {} {}; synth -flatten -top qorsyn_toy; stat",
            dir.path().join("toy.v").display(),
            dir.path().join("wrap.v").display()
        ))
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&out.stdout);
    assert!(log.contains("=== toy_ctrl ==="), "the design survives flattening as its own module:\n{log}");
}

/// A harness hardened as a block of its own is named after the block: `emit_harness_as` is the harness
/// `emit_harness` writes with only its module name changed, and the testbench around that name runs the gate to the
/// same signature in the same cycles.
#[test]
fn a_harness_named_after_its_block_is_the_same_harness_under_that_name() {
    let k = kernel_of("K5");
    let (c, files) = logos_candidate("K5").unwrap();
    let block = "qorpwr_k5_logos_p7_5";
    let own = emit_harness(&k, &c, Mode::Shaped);
    let named = emit_harness_as(&k, &c, Mode::Shaped, block);
    assert!(named.contains(&format!("module {block}(")), "{named}");
    assert!(!named.contains(&harness_module(&c)), "no trace of the default name");
    assert_eq!(named.replace(block, &harness_module(&c)), own, "only the module's name differs");
    let tb = emit_testbench_as(block, OPS, 200 * OPS as u64 + 1_000, IDLE);
    assert!(tb.contains(&format!("  {block} u_h(")), "{tb}");
    assert_eq!(tb.replace(block, &harness_module(&c)), emit_testbench(&c, OPS, 200 * OPS as u64 + 1_000, IDLE));
    let trace = emit_trace_testbench_as(&k, block, 40, 10_000, IDLE);
    assert_eq!(trace.replace(block, &harness_module(&c)), emit_trace_testbench(&k, &c, 40, 10_000, IDLE));
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = tmp();
    let mut paths = Vec::new();
    for (name, text) in &files {
        std::fs::write(dir.path().join(name), text).unwrap();
        paths.push(dir.path().join(name));
    }
    std::fs::write(dir.path().join(format!("{block}.v")), &named).unwrap();
    std::fs::write(dir.path().join("tb_qor.v"), &tb).unwrap();
    let exe = dir.path().join("sim.vvp");
    let b = Command::new("iverilog")
        .args(["-g2012", "-s", "tb_qor", "-o"])
        .arg(&exe)
        .args(&paths)
        .arg(dir.path().join(format!("{block}.v")))
        .arg(dir.path().join("tb_qor.v"))
        .output()
        .unwrap();
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    let run = parse_run(&String::from_utf8_lossy(&Command::new("vvp").arg("-n").arg(&exe).output().unwrap().stdout))
        .unwrap_or_else(|e| panic!("{e}"));
    let reference = assert_golden(&k, &c, &files, Mode::Shaped, OPS);
    assert_eq!((run.ops, run.misr, run.cycles), (reference.ops, reference.misr, reference.cycles));
}

/// The fabric twice over: K1's decision and K2's polynomial each run as the compiled combinational fabric
/// (`k1_logos`, `k2_logos`) and as the sequenced datapath over the same cells (`emit_seq_datapath_verilog`): one cell
/// a cycle, started by its reset and done when the stream ends, the operands held meanwhile. Both fold the golden on
/// both stimulus modes, and a sequenced operation costs its cells plus the start and capture edges.
#[test]
fn the_sequenced_fabric_folds_the_golden_one_cell_a_cycle() {
    use logicaffeine_compile::codegen_sva::decision_workload::{DECIDE_FN, DECIDE_NAME};
    use logicaffeine_compile::codegen_sva::fabric_rtl::{decision_fabric_from_source, POLY_BODY};
    if !common::require_tool("iverilog") {
        return;
    }
    let poly = format!("## To poly (x: Int) -> Int:\n    Return {POLY_BODY}.\n");
    for (id, src, fname) in [("K1", DECIDE_FN.to_string(), DECIDE_NAME), ("K2", poly, "poly")] {
        let k = kernel_of(id);
        let cands = logos_candidates(id).unwrap();
        let names: Vec<&str> = cands.iter().map(|(c, _)| c.name.as_str()).collect();
        let seq = format!("{}_logos_seq", id.to_lowercase());
        assert_eq!(names[..2], [format!("{}_logos", id.to_lowercase()).as_str(), seq.as_str()], "{id}: the fabric and its sequenced form first");
        let (c, files) = &cands[1];
        let cells = decision_fabric_from_source(&src, fname).cells.len();
        for mode in [Mode::Shaped, Mode::Edge] {
            let run = assert_golden(&k, c, files, mode, 120);
            assert_eq!(run.steady_cycles_per_op(), Some((cells + 2) as f64), "{id}: {cells} cells, then start and capture");
        }
    }
}
