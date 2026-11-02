//! fabric_postsynth_lock — the decode board as synthesis builds it, run on its serial line.
//!
//! Every other fabric lock runs iverilog over the RTL, whose memories are behavioral arrays: it holds the design, never
//! the netlist a bitstream is built from, and a block RAM mapping that loses bits is invisible to it. This lock runs the
//! image flow's own synthesis (`synth_xilinx -family xc7 -flatten -nowidelut`) over `decode_board` and the step's RTL,
//! then simulates the gate-level netlist — yosys's 7-series cell models (LUTs, LUT-RAM, carry chains, flops, DSP slices)
//! and a faithful block RAM model (`assets/ramb_behav.v`, every true and simple dual-port width over the device's data
//! and parity layout) — through the board's protocol on its serial pins: the first prompt positions stepped with `S`,
//! each next token the evaluator's and each step the twin's cycles, then tokens generated on chip with `G` from there,
//! the evaluator's, in exactly the loop's own accounting of cycles. The netlist holds the RAM and DSP slices the
//! bitstream is built from: block RAM where a memory is deep, LUT-RAM where its banks are 64 rows or fewer (every memory
//! of Board A's on its 16 banks).
//!
//! Both Nexys parts run it. The serial divider is fixed at synthesis to `CPB` clocks per bit, the one change from the
//! flashed image, so the simulation spends its time on the step rather than the line. A gate-level step is about seven
//! minutes on this box (55 cycles a second over 22,396), so the `postsynth_` test is a silicon sim: the routine baseline
//! leaves it to `scripts/run-silicon-tests.sh`.
//!
//! Run: `./scripts/run-silicon-tests.sh` (yosys- and iverilog-gated; about an hour)

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "common/fabric_rtl.rs"]
mod fabric_rtl;
#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::ir::SpatialProgram;
use logicaffeine_compile::spatial::lower::fabric::{fabric_cycle_model, input_hex_files, FabricBackend, VerilogArtifact};
use logicaffeine_compile::spatial::lower::fabric_board::{decode_board_verilog, generate_cycles, DecodeWords};
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;
use spatial_support::inputs_from_fills;

/// The faithful 7-series block RAM model the WPE's gate-level lock built.
const RAMB_BEHAV_V: &str = include_str!("assets/ramb_behav.v");

/// Clocks per serial bit, fixed in the netlist.
const CPB: u32 = 4;

/// Prompt positions stepped with `S`, and tokens generated after them with `G`.
const STEPS: usize = 2;
const GENERATED: usize = 2;

fn model() -> (Int8Model, Vec<usize>, Vec<usize>) {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let tokens = |k: &str| -> Vec<usize> {
        j["golden_trace"][k].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as usize).collect()
    };
    (Int8Model::from_json(&j).expect("the model"), tokens("prompt_tokens"), tokens("generated_tokens"))
}

fn lowered(src: &str, target: &SpatialTarget) -> (SpatialProgram, VerilogArtifact) {
    let l = compile_program_to_spatial(src, target).expect("a tensor program");
    let p = l.program.unwrap_or_else(|| panic!("the step lowers whole: {:?}", l.missing.first()));
    let art = match FabricBackend::new(target.clone()).lower(&p) {
        Ok(SpatialArtifact::Verilog(v)) => v,
        other => panic!("the fabric lowers the step: {other:?}"),
    };
    (p, art)
}

/// yosys's 7-series cell models without its two block RAM modules, which the faithful model replaces. A yosys on the
/// PATH ships them in its data directory; one that does not is an install synthesis itself would fail on.
fn cells_sim_noramb() -> String {
    const PLACES: [&str; 2] = ["/usr/share/yosys/xilinx/cells_sim.v", "/usr/local/share/yosys/xilinx/cells_sim.v"];
    let full = PLACES
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| panic!("yosys is on the PATH but its 7-series cell models are at none of {PLACES:?}"));
    let at = full.find("\nmodule RAMB18E1 (").expect("yosys's cell models end with RAMB18E1 and RAMB36E1");
    full[..at + 1].to_string()
}

/// `decode_board` over `art`'s step, synthesized as the image flow synthesizes it with the divider at `CPB`: the
/// gate-level netlist, its block RAMs initialized from the step's images.
fn synthesize(dir: &std::path::Path, board: &str, art: &VerilogArtifact, images: &[(String, String)]) -> String {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("decode_board.v"), board).unwrap();
    std::fs::write(dir.join("spatial_top.v"), &art.rtl).unwrap();
    for (name, body) in art.hex.iter().chain(images) {
        std::fs::write(dir.join(name), body).unwrap();
    }
    let script = format!(
        "read_verilog decode_board.v spatial_top.v\nchparam -set CLKS_PER_BIT {CPB} decode_board\n\
         synth_xilinx -family xc7 -flatten -nowidelut -top decode_board\nwrite_verilog -noattr netlist.v\n"
    );
    std::fs::write(dir.join("synth.ys"), script).unwrap();
    let run = std::process::Command::new("yosys").arg("-q").arg("-s").arg("synth.ys").current_dir(dir).output().expect("yosys");
    assert!(
        run.status.success(),
        "synthesis: {}\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    std::fs::read_to_string(dir.join("netlist.v")).expect("the netlist")
}

fn u16le(v: u16) -> [u8; 2] {
    v.to_le_bytes()
}

fn le(bytes: &[u8]) -> u64 {
    bytes.iter().rev().fold(0u64, |a, b| (a << 8) | *b as u64)
}

#[test]
fn postsynth_the_synthesized_board_steps_and_generates_like_the_evaluator() {
    if !common::require_tool("yosys") || !common::require_tool("iverilog") {
        return;
    }
    let cells = cells_sim_noramb();
    let (m, prompt, _) = model();
    for target in [SpatialTarget::xc7a100t_fabric(16, 64), SpatialTarget::xc7a200t_fabric(16, 64)] {
        let label = format!("netlist@{}", target.name);
        let lat = target.site.local_mem_read_latency.unwrap() as u64;
        let (first, art) = lowered(&m.decode_step_lg(prompt[0], 0, &[], &[]), &target);
        let twin = fabric_cycle_model(&first, &target);
        let board = decode_board_verilog(DecodeWords::of(&first, "tok", "pos", "next").expect("the step's words"), lat);
        let dir = std::env::temp_dir().join(format!("fabric_postsynth_{}_{}", target.name, std::process::id()));
        let netlist = synthesize(&dir, &board, &art, &input_hex_files(&first, &inputs_from_fills(&first)));
        // The memories map onto RAM the cell models cover: block RAM, or the LUT-RAM a memory of banks up to 64 rows
        // deep takes (every one of Board A's on 16 banks).
        let rams: Vec<&str> = ["RAMB18E1", "RAM32M", "RAM64M"].into_iter().filter(|r| netlist.contains(&format!("\n  {r} "))).collect();
        assert!(!rams.is_empty(), "{label}: the netlist maps the step's memories onto RAM primitives");
        for primitive in ["DSP48E1", "CARRY4"] {
            assert!(netlist.contains(&format!("\n  {primitive} ")), "{label}: the netlist maps the step onto {primitive}s");
        }
        // The evaluator's steps over the prompt, the cache carried from step to step.
        let (mut kc, mut vc, mut want) = (Vec::new(), Vec::new(), Vec::new());
        for (pos, tok) in prompt.iter().take(STEPS).enumerate() {
            let (p, _) = lowered(&m.decode_step_lg(*tok, pos, &kc, &vc), &target);
            let out = eval_choreography(&p, &inputs_from_fills(&p)).expect("the step evaluates");
            let read = |name: &str| -> Vec<i64> {
                match &out.tensors[&p.tensors.iter().find(|t| t.name == name).unwrap().id] {
                    Value::Int(v) => v.clone(),
                    Value::Float(v) => panic!("{name} through floats: {v:?}"),
                }
            };
            kc = read("Kc")[..(pos + 1) * m.d].to_vec();
            vc = read("Vc")[..(pos + 1) * m.d].to_vec();
            want.push(read("next")[0] as u64);
        }
        let mut script = Vec::new();
        for (pos, tok) in prompt.iter().take(STEPS).enumerate() {
            let mut s = vec![b'S'];
            s.extend(u16le(*tok as u16 + 1));
            s.extend(u16le(pos as u16 + 1));
            script.push((s, 7));
        }
        let mut g = vec![b'G'];
        g.extend(u16le(GENERATED as u16));
        script.push((g, 2 * GENERATED + 5));
        let sources = vec![
            ("netlist.v".to_string(), netlist),
            ("cells_sim_noramb.v".to_string(), cells.clone()),
            ("ramb_behav.v".to_string(), RAMB_BEHAV_V.to_string()),
            ("tb.v".to_string(), fabric_rtl::board_testbench(None, CPB, &script, (twin + 2000) * (STEPS + GENERATED) as u64 * 2)),
        ];
        let replies = fabric_rtl::run_board(&dir, &sources, &script, (twin + 2000) * (STEPS + GENERATED) as u64 * 2);
        for (pos, (r, w)) in replies[..STEPS].iter().zip(&want).enumerate() {
            assert_eq!(r[0], b's', "{label}: a step reply at position {pos}");
            assert_eq!(le(&r[1..3]), *w, "{label}: the next token at position {pos} is the evaluator's");
            assert_eq!(le(&r[3..7]), twin, "{label}: the step at position {pos} takes the twin's cycles");
        }
        // The prompt continues on chip: the token after the last prompt step the board stepped, then the next.
        let gen = &replies[STEPS];
        let tokens: Vec<u64> = (0..GENERATED).map(|k| le(&gen[2 * k..2 * k + 2])).collect();
        let mut expect = Vec::new();
        let (mut kc, mut vc, mut tok) = (kc.clone(), vc.clone(), *want.last().unwrap() as usize - 1);
        for pos in STEPS..STEPS + GENERATED {
            let (p, _) = lowered(&m.decode_step_lg(tok, pos, &kc, &vc), &target);
            let out = eval_choreography(&p, &inputs_from_fills(&p)).expect("the step evaluates");
            let read = |name: &str| -> Vec<i64> {
                match &out.tensors[&p.tensors.iter().find(|t| t.name == name).unwrap().id] {
                    Value::Int(v) => v.clone(),
                    Value::Float(v) => panic!("{name} through floats: {v:?}"),
                }
            };
            kc = read("Kc")[..(pos + 1) * m.d].to_vec();
            vc = read("Vc")[..(pos + 1) * m.d].to_vec();
            let next = read("next")[0];
            expect.push(next as u64);
            tok = next as usize - 1;
        }
        assert_eq!(tokens, expect, "{label}: the board generates the evaluator's tokens after the prompt it stepped");
        assert_eq!(gen[2 * GENERATED], b'e', "{label}: the loop ends with its cycles");
        assert_eq!(
            le(&gen[2 * GENERATED + 1..]),
            generate_cycles(&vec![twin; GENERATED], lat),
            "{label}: the loop takes the steps' cycles and the protocol's own"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
