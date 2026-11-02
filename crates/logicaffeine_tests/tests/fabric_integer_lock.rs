//! fabric_integer_lock — an int8 model's integer forms, and its whole decode step, as synthesizable Verilog on our
//! fabric, running under iverilog to exactly the spatial evaluator's answer in exactly the cycle twin's cycles.
//!
//! The spatial evaluator is the contract every lowering answers to; its integer arm evaluates a `Generic` in `i64`. The
//! fabric's GENERAL Generic engine is that arm in hardware: it walks the iteration space with address counters (the
//! parallel axes outermost, a fold's axis inside them, a contraction innermost), reads each operand slot through its
//! affine map — and a gather's slot through the value its index slot read — evaluates the body as a 64-bit datapath,
//! folds, and writes the point where the evaluator's output convention puts it. A composed program's sections run in
//! order, each under its own plan.
//!
//! Each case lowers through `compile_program_to_spatial` onto the decode fabric of three machines — sky130, whose flop
//! memories answer a read in the cycle of its address, and the two Nexys parts, whose block RAM answers the cycle after
//! and whose PE arrays are capped — so every read path is held at both latencies, and every matmul whole and in
//! blocks. On each the RTL is the same IR the evaluator ran, and holds four things:
//! the design finishes; every output tensor and every state tensor it carries to the next step is the evaluator's; its
//! cycle count IS the twin's (`fabric_cycle_model`); and its instance census IS the IR's.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test fabric_integer_lock` (iverilog-gated)

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "common/fabric_rtl.rs"]
mod fabric_rtl;
#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::ir::{SpatialProgram, TensorRole};
use logicaffeine_compile::spatial::lower::fabric::{fabric_cycle_model, host_word, FabricBackend};
use logicaffeine_compile::spatial::lower::{Census, SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;
use fabric_rtl::HostStep;
use spatial_support::inputs_from_fills;

/// The decode fabric on sky130 — its PE array as large as a matmul's block — and on the two Nexys parts, whose arrays
/// are capped (16×16 on the XC7A200T, 8×8 on the XC7A100T), so a matmul runs as blocks: across its contraction,
/// reloading the partial, and across its output columns, a loop over sub-blocks.
fn decode_fabrics() -> [SpatialTarget; 3] {
    [SpatialTarget::sky_fabric(16, 64), SpatialTarget::xc7a200t_fabric(16, 64), SpatialTarget::xc7a100t_fabric(16, 64)]
}

fn lowered(src: &str, target: &SpatialTarget) -> SpatialProgram {
    let l = compile_program_to_spatial(src, target).expect("a tensor program");
    l.program
        .unwrap_or_else(|| panic!("the program lowers whole: {:?}\n{src}", l.missing.first()))
}

/// Lower `src` to each decode fabric, simulate it, and hold every output and state tensor, the cycle count and the
/// census.
fn holds(label: &str, src: &str) {
    if !common::require_tool("iverilog") {
        return;
    }
    for target in decode_fabrics() {
        holds_on(&target, &format!("{label}@{}", target.name), src);
    }
}

fn holds_on(target: &SpatialTarget, label: &str, src: &str) {
    let p = lowered(src, target);
    let inputs = inputs_from_fills(&p);
    let want = eval_choreography(&p, &inputs).expect("the program evaluates");
    let art = match FabricBackend::new(target.clone()).lower(&p) {
        Ok(SpatialArtifact::Verilog(v)) => v,
        other => panic!("{label}: the fabric lowers the program: {other:?}"),
    };
    let dir = std::env::temp_dir().join(format!("fabric_integer_{label}_{}", std::process::id()));
    let run = fabric_rtl::simulate(&dir, &p, &art, &inputs);
    assert!(run.done, "{label}: the design finishes\n{}", run.log);
    let carried: Vec<_> = p.tensors.iter().filter(|t| matches!(t.role, TensorRole::Output | TensorRole::State)).collect();
    assert!(!carried.is_empty(), "{label}: the program hands back a tensor");
    for t in carried {
        let want = match &want.tensors[&t.id] {
            Value::Int(v) => v.clone(),
            Value::Float(v) => panic!("{label}: {} evaluated through floats: {v:?}", t.name),
        };
        let got = run
            .out
            .get(&t.id.idx())
            .unwrap_or_else(|| panic!("{label}: the testbench prints `{}`\n{}", t.name, run.log));
        assert_eq!(got, &want, "{label}: `{}` in RTL is the evaluator's", t.name);
    }
    assert_eq!(
        run.cycles,
        fabric_cycle_model(&p, target),
        "{label}: the RTL's cycle count IS the twin's"
    );
    assert_eq!(Census::of_verilog(&art.rtl), Census::of(&p), "{label}: the RTL census IS the IR's");
    let _ = std::fs::remove_dir_all(&dir);
}

fn tensor(name: &str, rows: usize, cols: usize, vals: &[i64]) -> String {
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of Int.\n");
    for (i, v) in vals.iter().enumerate() {
        if *v != 0 {
            s.push_str(&format!("Set item {} of {name} to {v}.\n", i + 1));
        }
    }
    s
}

fn spread(n: usize, lo: i64, hi: i64, seed: u64) -> Vec<i64> {
    (0..n as u64)
        .map(|i| lo + ((i.wrapping_add(seed).wrapping_mul(2_654_435_761) >> 7) % (hi - lo + 1) as u64) as i64)
        .collect()
}

// ── ONE FORM AT A TIME ───────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_requant_runs_in_rtl() {
    let src = format!(
        "## Main\n{}Let Y be X times 104593 shifted right by 24.\n",
        tensor("X", 1, 8, &spread(8, -258_064, 258_064, 1))
    );
    holds("requant", &src);
}

#[test]
fn a_saturation_runs_in_rtl() {
    let src = format!(
        "## Main\n{}Let Y be X clamped between -128 and 127.\n",
        tensor("X", 1, 8, &spread(8, -400, 400, 2))
    );
    holds("clamp", &src);
}

#[test]
fn an_integer_relu_runs_in_rtl() {
    let src = format!("## Main\n{}Let Y be relu of X.\n", tensor("X", 1, 8, &spread(8, -40, 40, 3)));
    holds("relu", &src);
}

#[test]
fn the_saturating_requant_composes_in_rtl() {
    let src = format!(
        "## Main\n{}Let Y be X times 16777216 shifted right by 24.\nLet Z be Y clamped between -128 and 127.\n",
        tensor("X", 1, 8, &spread(8, -258_064, 258_064, 4))
    );
    holds("to8", &src);
}

/// An integer matmul, then its requant: a matrix-engine section beside a Generic one, each under its own plan.
#[test]
fn a_matmul_then_its_requant_runs_in_rtl() {
    let src = format!(
        "## Main\n{}{}Let Y be X matmul W.\nLet Z be Y times 104593 shifted right by 24.\n",
        tensor("X", 1, 4, &spread(4, -128, 127, 5)),
        tensor("W", 4, 4, &spread(16, -128, 127, 6))
    );
    holds("matmul_requant", &src);
}

/// A matmul of several rows runs on the engine's PE array, and one shallower than the array loads only the array's first
/// rows: the rest multiply the zero activations it streams through them. Run before any deeper matmul has loaded those
/// rows, they hold what their weight registers start with: zero, never the unknown a register powers up with, which
/// turns every sum it reaches unknown.
#[test]
fn a_matmul_shallower_than_its_array_runs_in_rtl_before_any_deeper_one() {
    let src = format!(
        "## Main\n{}{}{}{}Let Y be X matmul W.\nLet Z be U matmul V.\n",
        tensor("X", 4, 4, &spread(16, -128, 127, 21)),
        tensor("W", 4, 8, &spread(32, -128, 127, 22)),
        tensor("U", 4, 16, &spread(64, -128, 127, 23)),
        tensor("V", 16, 8, &spread(128, -128, 127, 24))
    );
    holds("shallow_first", &src);
}

/// A row gathered by an index the program computes: the tile address adds `(index − 1) · row` at run time. The first row
/// alone cannot tell that apart from an address that ignores the index, so the rows past it are what this holds.
#[test]
fn a_row_gather_by_a_runtime_index_runs_in_rtl() {
    for at in [1usize, 2, 4, 6] {
        let src = format!(
            "## Main\n{}Let at be {at}.\nLet r be row at of T.\nLet y be r times 3 shifted right by 0.\n",
            tensor("T", 6, 4, &spread(24, -100, 100, 10))
        );
        holds(&format!("row_gather_{at}"), &src);
    }
}

// ── THE LUT ATTENTION: folds, gathers, masks ─────────────────────────────────────────────────────────────────────────

fn lut_program(upto: Option<usize>) -> String {
    let (h, hd, l) = (2usize, 2usize, 4usize);
    let table: Vec<i64> = {
        let mut t = vec![32768i64];
        while t.len() < 8 {
            t.push((t[t.len() - 1] * 7 / 8).max(1));
        }
        t
    };
    let mut s = String::from("## Main\n");
    s.push_str(&tensor("Q", 1, h * hd, &spread(h * hd, -128, 127, 7)));
    s.push_str(&tensor("K", l, h * hd, &spread(l * h * hd, -128, 127, 8)));
    s.push_str(&tensor("V", l, h * hd, &spread(l * h * hd, -128, 127, 9)));
    s.push_str(&tensor("E", 1, 8, &table));
    match upto {
        Some(p) => s.push_str(&format!(
            "Let pos be {p}.\nLet C be lut attention of Q over K and V with {h} heads {h} groups up to pos with table E and scale 21076 shifted right by 24.\n"
        )),
        None => s.push_str(&format!(
            "Let C be lut attention of Q over K and V with {h} heads {h} groups with table E and scale 21076 shifted right by 24.\n"
        )),
    }
    s
}

#[test]
fn a_lut_attention_over_the_whole_cache_runs_in_rtl() {
    holds("lut_all", &lut_program(None));
}

#[test]
fn a_lut_attention_up_to_a_position_runs_in_rtl_at_every_position() {
    for p in 1..=4 {
        holds(&format!("lut_upto_{p}"), &lut_program(Some(p)));
    }
}

// ── THE WHOLE DECODE STEP ────────────────────────────────────────────────────────────────────────────────────────────

/// The tiny transformer's decode step for the prompt's first token — every form of the model, in one composition.
#[test]
fn the_whole_decode_step_runs_in_rtl() {
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    let m = Int8Model::from_json(&j).expect("the model");
    let tok = j["golden_trace"]["prompt_tokens"][0].as_u64().unwrap() as usize;
    holds("decode_step", &m.decode_step_lg(tok, 0, &[], &[]));
}

/// The step at the generation positions — the last prompt token and the two generated after it — each over the cache
/// the run built: every position's cache rows are the evaluator's carried state after the steps before it.
#[test]
fn the_decode_step_runs_in_rtl_at_the_generation_positions() {
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    let m = Int8Model::from_json(&j).expect("the model");
    let prompt: Vec<usize> = j["golden_trace"]["prompt_tokens"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_u64().unwrap() as usize)
        .collect();
    let rows = |state: &[i64], upto: usize| state[..upto * m.d].to_vec();
    let (mut kc, mut vc) = (Vec::new(), Vec::new());
    let mut tok = prompt[0];
    for pos in 0..=prompt.len() + 1 {
        let src = m.decode_step_lg(tok, pos, &kc, &vc);
        if pos + 1 >= prompt.len() {
            holds(&format!("decode_step_{pos}"), &src);
        }
        let p = lowered(&src, &SpatialTarget::sky_fabric(16, 64));
        let out = eval_choreography(&p, &inputs_from_fills(&p)).expect("the step evaluates");
        let read = |name: &str| -> Vec<i64> {
            let id = p.tensors.iter().find(|t| t.name == name).unwrap().id;
            match &out.tensors[&id] {
                Value::Int(v) => v.clone(),
                Value::Float(v) => panic!("{name} through floats: {v:?}"),
            }
        };
        kc = rows(&read("Kc"), pos + 1);
        vc = rows(&read("Vc"), pos + 1);
        let next = read("next")[0] as usize - 1;
        tok = if pos + 1 < prompt.len() { prompt[pos + 1] } else { next };
    }
}

/// The step as a board runs it. Every position's step lowers to ONE RTL text — the position is a tensor the step reads,
/// not a constant it is built around — so the RTL is loaded once and driven step after step through its host port, the
/// cache never leaving the fabric: before each step the host writes only the token and the position, and after it
/// reads back the next token and the two cache rows the step wrote. Through the whole prompt and the tokens generated
/// after it, every word read is the evaluator's for that position after the steps before it, every step takes the
/// twin's cycles, and the tokens generated are the trainer's golden ones.
#[test]
fn the_decode_step_runs_step_after_step_through_the_host_port_with_its_cache_resident() {
    if !common::require_tool("iverilog") {
        return;
    }
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let m = Int8Model::from_json(&j).expect("the model");
    let tokens = |k: &str| -> Vec<usize> {
        j["golden_trace"][k].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as usize).collect()
    };
    let (prompt, golden) = (tokens("prompt_tokens"), tokens("generated_tokens"));
    for target in decode_fabrics() {
        let label = format!("host_steps@{}", target.name);
        let lat = target.site.local_mem_read_latency.expect("a fabric states its read latency") as u64;
        let first = lowered(&m.decode_step_lg(prompt[0], 0, &[], &[]), &target);
        let art = match FabricBackend::new(target.clone()).lower(&first) {
            Ok(SpatialArtifact::Verilog(v)) => v,
            other => panic!("{label}: the fabric lowers the step: {other:?}"),
        };
        let twin = fabric_cycle_model(&first, &target);
        let tensor = |name: &str| first.tensors.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("`{name}`"));
        let (tok_t, pos_t, next_t, kc_t, vc_t) = (tensor("tok"), tensor("pos"), tensor("next"), tensor("Kc"), tensor("Vc"));
        let row = |t: &logicaffeine_compile::spatial::ir::TensorDecl, pos: usize| -> Vec<(usize, usize)> {
            (0..m.d).map(|c| (t.id.idx(), host_word(&first, t, 0, pos * m.d + c))).collect()
        };
        let (mut steps, mut want) = (Vec::new(), Vec::new());
        let (mut kc, mut vc) = (Vec::new(), Vec::new());
        let mut tok = prompt[0];
        for pos in 0..prompt.len() + golden.len() - 1 {
            let p = lowered(&m.decode_step_lg(tok, pos, &kc, &vc), &target);
            let at = match FabricBackend::new(target.clone()).lower(&p) {
                Ok(SpatialArtifact::Verilog(v)) => v,
                other => panic!("{label}: the step at position {pos} lowers: {other:?}"),
            };
            assert!(at.rtl == art.rtl, "{label}: the step at position {pos} is the RTL of every position");
            assert_eq!(fabric_cycle_model(&p, &target), twin, "{label}: every position takes the same cycles");
            let out = eval_choreography(&p, &inputs_from_fills(&p)).expect("the step evaluates");
            let read = |name: &str| -> Vec<i64> {
                let id = p.tensors.iter().find(|t| t.name == name).unwrap().id;
                match &out.tensors[&id] {
                    Value::Int(v) => v.clone(),
                    Value::Float(v) => panic!("{name} through floats: {v:?}"),
                }
            };
            let (kc_now, vc_now, next) = (read("Kc"), read("Vc"), read("next")[0]);
            let mut reads = vec![(next_t.id.idx(), host_word(&first, next_t, 0, 0))];
            reads.extend(row(kc_t, pos));
            reads.extend(row(vc_t, pos));
            let mut expect = vec![next];
            expect.extend_from_slice(&kc_now[pos * m.d..(pos + 1) * m.d]);
            expect.extend_from_slice(&vc_now[pos * m.d..(pos + 1) * m.d]);
            steps.push(HostStep {
                writes: vec![
                    (tok_t.id.idx(), host_word(&first, tok_t, 0, 0), tok as i64 + 1),
                    (pos_t.id.idx(), host_word(&first, pos_t, 0, 0), pos as i64 + 1),
                ],
                reads,
            });
            want.push(expect);
            kc = kc_now[..(pos + 1) * m.d].to_vec();
            vc = vc_now[..(pos + 1) * m.d].to_vec();
            tok = if pos + 1 < prompt.len() { prompt[pos + 1] } else { next as usize - 1 };
        }
        let dir = std::env::temp_dir().join(format!("fabric_integer_{label}_{}", std::process::id()));
        let run = fabric_rtl::simulate_host_steps(&dir, &first, &art, &inputs_from_fills(&first), &steps, lat, twin * 8);
        assert_eq!(run.done.len(), steps.len(), "{label}: every step ran\n{}", run.log);
        for (pos, ((done, cycles), (got, expect))) in run.done.iter().zip(&run.cycles).zip(run.reads.iter().zip(&want)).enumerate() {
            assert!(done, "{label}: the step at position {pos} finishes\n{}", run.log);
            assert_eq!(*cycles, twin, "{label}: the step at position {pos} takes the twin's cycles");
            assert_eq!(got, expect, "{label}: at position {pos}, the next token and the cache rows read through the host port are the evaluator's");
        }
        let generated: Vec<usize> = run.reads[prompt.len() - 1..].iter().map(|r| r[0] as usize - 1).collect();
        assert_eq!(generated, golden, "{label}: the tokens the fabric generates are the trainer's");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
