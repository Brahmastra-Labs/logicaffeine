//! decode_tensor_parallel_lock — the int8 decode step split over chips Megatron's way is the step.
//!
//! `Int8Model::decode_step_tp_lg` writes the step for `tp` chips as ordinary LOGOS. Each chip owns whole heads: its
//! columns of `Wq`, `Wk` and `Wv`, its heads' rows of `Wo`, its heads' columns of the KV cache, and its columns of `W1`
//! with their rows of `W2`. Each chip's output projection is a partial accumulator; an all-reduce, spelled as the
//! fold of `plus` it means, sums the partials before the requant. The embedding, the residuals and the head stay on
//! the replicated spine.
//!
//! Integer sums are exact in any order, so the split step must equal the step bit for bit. This lock holds:
//! - on the VM, at every position of the golden trace, the split step shows the step's next token, every logit, both
//!   residuals, and the cache row the step writes;
//! - every chip-local table in the source holds the full table's values at the coordinates its manifest slice
//!   names, the map a host loads the chips by;
//! - one all-reduce per row-parallel projection, its partials one per chip in chip order;
//! - a split the heads or the FFN columns do not divide is refused by name;
//! - for each board's fabric target, the split step lowers whole as one program over the two chips, and its evaluator
//!   gives the step's next token, logits, residuals and cache rows at every position of the trace;
//! - in that program every chip-local tensor lives on its chip, and each all-reduce is one stage on chip 0 that sums
//!   in integers the partial loaded on each chip;
//! - one board alone, with no link to cross, refuses the split by name;
//! - lowered for the boards, the split is one ordinary program a chip — chip `c`'s statements, composed on its one
//!   site — and the tensors that cross between them, in program order: `xi` to chip 1, `o_a_c1` to chip 0, `hi` to
//!   chip 1, `f_a_c1` to chip 0, each all-reduce a transfer and the fold a local add on chip 0;
//! - each chip's program, given the composed program's values for what it receives, holds the composed program's
//!   values for everything else at every position of the trace: the parts and their transfers are the whole;
//! - each chip's program halts where a tensor crosses: after the section that last writes a tensor it sends, before
//!   the first section that reads a tensor it receives, and each transfer is one send and one receive;
//! - the host's relay over those halts runs every chip to its end, copies each transfer once after its sender has
//!   written it and before its receiver reads it, and lets both chips run their heads at once;
//! - each chip's twin counts the fabric's cycles at each halt and at its end, the counter paused while it is halted;
//!   a program with no halts is one count, the one it always had;
//! - each chip's board, driven through its halts as the relay drives it — the received words written, `X` run — replies
//!   the twin's count at every halt, the composed program's values for every word it sends, and its result at its end;
//! - a board's `X` past a halt replies only once its fabric has left that halt, however many cycles the fabric takes;
//! - the split host (`scripts/split-host.py`), given a split ledger and the replies the boards give, runs the golden
//!   trace through the compiler's relay and records its tokens; with a halt, a count, a prompt step's token or a
//!   generated token changed it refuses the run by name.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test decode_tensor_parallel_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "common/fabric_rtl.rs"]
mod fabric_rtl;
#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography_every_tensor, Inputs, Value};
use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::ir::{ChipId, SpatialProgram, StageKind, TensorRole};
use logicaffeine_compile::spatial::passes::shard::ShardError;
use logicaffeine_compile::spatial::lower::fabric::{fabric_cycle_model, fabric_cycle_segments, host_word, FabricBackend};
use logicaffeine_compile::spatial::lower::fabric_board::{chip_board_verilog, chip_record, relay_record};
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::pipeline::{compile_program_to_chips, compile_program_to_spatial, compile_program_to_spatial_tp};
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::spatial::tensor_parallel::{relay_schedule, ChipTransfer, RelayStep, SliceAxis, TpManifest, TpReject};
use spatial_support::inputs_from_fills;

fn model_and_trace() -> (Int8Model, Vec<usize>) {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let m = Int8Model::from_json(&j).expect("the model");
    let toks = |k: &str| -> Vec<usize> { j["golden_trace"][k].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as usize).collect() };
    let mut trace = toks("prompt_tokens");
    let generated = toks("generated_tokens");
    trace.extend_from_slice(&generated[..generated.len() - 1]);
    (m, trace)
}

fn vm_ints(src: &str) -> Vec<i64> {
    let out = vm_outcome(src);
    assert!(out.error.is_none(), "the VM runs it: {:?}", out.error);
    out.output.split_whitespace().map(|t| t.parse().expect("an integer")).collect()
}

/// What a step shows: its next token, every logit, both residuals, then the key and the value row it writes at `pos`,
/// each `d` wide. `cache` names the caches whose columns, side by side in order, make the row: one pair for the whole
/// step, a pair per chip for a split one.
fn shown(m: &Int8Model, pos: usize, cache: &[(String, String)]) -> String {
    let mut s = String::from("Show next.\n");
    for i in 1..=m.vocab {
        s.push_str(&format!("Show item {i} of logits.\n"));
    }
    for r in ["s1", "s2"] {
        for i in 1..=m.d {
            s.push_str(&format!("Show item {i} of {r}.\n"));
        }
    }
    let width = m.d / cache.len();
    for side in 0..2 {
        for pair in cache {
            let t = if side == 0 { &pair.0 } else { &pair.1 };
            for j in 1..=width {
                s.push_str(&format!("Show item {} of {t}.\n", pos * width + j));
            }
        }
    }
    s
}

fn chip_caches(tp: usize) -> Vec<(String, String)> {
    (0..tp).map(|c| (format!("Kc_c{c}"), format!("Vc_c{c}"))).collect()
}

/// The values a source sets into `name`, by flat index; a table's unset elements are zero.
fn table_values(src: &str, name: &str, len: usize) -> Vec<i64> {
    let mut v = vec![0; len];
    let tail = format!(" of {name} to ");
    for line in src.lines() {
        let Some(rest) = line.strip_prefix("Set item ") else { continue };
        let Some((i, x)) = rest.split_once(&tail) else { continue };
        let i: usize = i.parse().expect("an item index");
        v[i - 1] = x.trim_end_matches('.').parse().expect("an integer");
    }
    v
}

/// The full model tensor a manifest slice cuts, with its shape: the source's names for the model's tables.
fn full_table(m: &Int8Model, of: &str, kc: &[i64], vc: &[i64]) -> (Vec<i64>, usize, usize) {
    let pad = |v: &[i64], rows: usize, cols: usize| -> Vec<i64> {
        let mut out = v.to_vec();
        out.resize(rows * cols, 0);
        out
    };
    match of {
        "Wq" | "Wk" | "Wv" | "Wo" => (m.weights[of].clone(), m.d, m.d),
        "W1" => (m.weights["W1"].clone(), m.d, m.d_ff),
        "W2" => (m.weights["W2"].clone(), m.d_ff, m.d),
        "E" => (m.exp_lut.clone(), 1, m.exp_lut.len()),
        "Kc" => (pad(kc, m.ctx, m.d), m.ctx, m.d),
        "Vc" => (pad(vc, m.ctx, m.d), m.ctx, m.d),
        other => panic!("a slice of {other}, which is no table of the model"),
    }
}

/// The step and the split step, both from the same cache, at every position of the trace: the caches the step writes
/// carried from one position to the next.
#[test]
fn the_two_chip_step_is_the_step_on_the_vm_at_every_position() {
    let (m, trace) = model_and_trace();
    let (mut kc, mut vc) = (Vec::new(), Vec::new());
    for (pos, tok) in trace.iter().enumerate() {
        let whole = vm_ints(&format!(
            "{}{}",
            m.decode_step_lg(*tok, pos, &kc, &vc),
            shown(&m, pos, &[("Kc".into(), "Vc".into())])
        ));
        let (src, manifest) = m.decode_step_tp_lg(*tok, pos, &kc, &vc, 2).expect("the model splits over two chips");
        assert_eq!(manifest.chips, 2);
        let split = vm_ints(&format!("{src}{}", shown(&m, pos, &chip_caches(2))));
        assert_eq!(split.len(), whole.len(), "position {pos}: the same values are shown");
        assert_eq!(split, whole, "position {pos}: the split step is the step, bit for bit");
        let rows = &whole[1 + m.vocab + 2 * m.d..];
        kc.extend_from_slice(&rows[..m.d]);
        vc.extend_from_slice(&rows[m.d..]);
    }
}

/// Every chip-local table holds the full table's values where its slice says, so a host loads each chip from the
/// model's own tensors by the manifest alone.
#[test]
fn every_chip_table_is_its_slice_of_the_model() {
    let (m, trace) = model_and_trace();
    let pos = 5;
    let kc: Vec<i64> = (0..pos * m.d).map(|i| (i as i64 * 7) % 23 - 11).collect();
    let vc: Vec<i64> = (0..pos * m.d).map(|i| (i as i64 * 5) % 19 - 9).collect();
    let (src, manifest) = m.decode_step_tp_lg(trace[pos], pos, &kc, &vc, 2).unwrap();
    let mut sliced = std::collections::BTreeSet::new();
    for (name, s) in &manifest.slices {
        let (full, rows, cols) = full_table(&m, &s.of, &kc, &vc);
        assert_eq!(s.of_shape, (rows, cols), "{name}: the slice names {}'s shape", s.of);
        let (r, c) = s.shape();
        let got = table_values(&src, name, r * c);
        for i in 0..r * c {
            assert_eq!(got[i], full[s.full_index(i / c, i % c)], "{name} element {i}: the full {} at its slice's coordinate", s.of);
        }
        sliced.insert(s.of.clone());
    }
    let want: std::collections::BTreeSet<String> = ["Wq", "Wk", "Wv", "Wo", "W1", "W2", "E", "Kc", "Vc"].iter().map(|s| s.to_string()).collect();
    assert_eq!(sliced, want, "every table a chip reads is a slice of the model's");
}

/// Wq, Wk, Wv and W1 split by column, Wo and W2 by row, the caches by column; the exp table whole on every chip. Each
/// chip's slices are its own heads' and its own FFN columns', contiguous and disjoint.
#[test]
fn each_chip_owns_whole_heads_and_whole_ffn_columns() {
    let (m, trace) = model_and_trace();
    let (_, manifest) = m.decode_step_tp_lg(trace[0], 0, &[], &[], 2).unwrap();
    let hd = m.d / m.heads;
    let per_chip_heads = m.heads / 2;
    for c in 0..2usize {
        let s = |name: &str| manifest.slices.get(&format!("{name}_c{c}")).unwrap_or_else(|| panic!("chip {c}'s {name}"));
        let heads = vec![(c * per_chip_heads * hd, per_chip_heads * hd)];
        for (name, axis) in [("Wq", SliceAxis::Cols), ("Wk", SliceAxis::Cols), ("Wv", SliceAxis::Cols), ("Kc", SliceAxis::Cols), ("Vc", SliceAxis::Cols), ("Wo", SliceAxis::Rows)] {
            assert_eq!((s(name).axis, &s(name).ranges), (axis, &heads), "chip {c}'s {name}: its heads");
        }
        let cols = vec![(c * m.d_ff / 2, m.d_ff / 2)];
        assert_eq!((s("W1").axis, &s("W1").ranges), (SliceAxis::Cols, &cols), "chip {c}'s W1: its FFN columns");
        assert_eq!((s("W2").axis, &s("W2").ranges), (SliceAxis::Rows, &cols), "chip {c}'s W2: its FFN rows");
        assert_eq!(s("E").ranges, vec![(0, m.exp_lut.len())], "chip {c}'s exp table: the whole table");
        for t in ["Wq", "Wk", "Wv", "Wo", "W1", "W2", "E", "Kc", "Vc"] {
            assert_eq!(manifest.chip_of.get(&format!("{t}_c{c}")), Some(&(c as u16)), "{t}_c{c} lives on chip {c}");
        }
    }
}

/// One all-reduce per row-parallel projection, of the accumulator before its requant: its partials one per chip, in
/// chip order, each on its chip.
#[test]
fn each_row_parallel_projection_is_one_all_reduce_of_its_accumulators() {
    let (m, trace) = model_and_trace();
    let (src, manifest) = m.decode_step_tp_lg(trace[0], 0, &[], &[], 2).unwrap();
    let outs: Vec<&str> = manifest.collectives.iter().map(|c| c.out.as_str()).collect();
    assert_eq!(outs, ["o_a", "f_a"], "the attention's and the FFN's output accumulators");
    for col in &manifest.collectives {
        let want: Vec<String> = (0..2).map(|c| format!("{}_c{c}", col.out)).collect();
        assert_eq!(col.partials, want, "{}: a partial per chip, in chip order", col.out);
        for (c, p) in col.partials.iter().enumerate() {
            assert_eq!(manifest.chip_running(p), c as u16, "{p} is computed on chip {c}");
        }
        assert_eq!(col.folds.last().map(String::as_str), Some(col.out.as_str()), "{}: the fold's last statement binds it", col.out);
        let requant = format!("Let {} be {} times ", if col.out == "o_a" { "o" } else { "f" }, col.out);
        assert!(src.contains(&requant), "{}: requanted after the sum, never per partial", col.out);
    }
}

/// A split that cuts a head or an FFN column in two is refused, naming what does not divide.
#[test]
fn a_split_the_heads_or_columns_do_not_divide_is_refused() {
    let (m, trace) = model_and_trace();
    let three = m.decode_step_tp_lg(trace[0], 0, &[], &[], 3).expect_err("2 heads over 3 chips");
    assert!(three.contains("heads"), "the refusal names the heads: {three}");
    let mut odd = m.clone();
    odd.d_ff = 33;
    let cols = odd.decode_step_tp_lg(trace[0], 0, &[], &[], 2).expect_err("33 FFN columns over 2 chips");
    assert!(cols.contains("FFN"), "the refusal names the FFN columns: {cols}");
    let none = m.decode_step_tp_lg(trace[0], 0, &[], &[], 0).expect_err("no chips");
    assert!(none.contains("chip"), "the refusal names the chips: {none}");
}

/// One chip is the step itself: no all-reduce, every table its whole model table.
#[test]
fn one_chip_is_the_step() {
    let (m, trace) = model_and_trace();
    let (src, manifest): (String, TpManifest) = m.decode_step_tp_lg(trace[0], 0, &[], &[], 1).unwrap();
    assert!(manifest.collectives.is_empty(), "one chip's accumulators are whole: {:?}", manifest.collectives);
    for s in manifest.slices.values() {
        assert_eq!(s.shape(), s.of_shape, "{}: the whole table on the one chip", s.of);
    }
    let whole = vm_ints(&format!("{}{}", m.decode_step_lg(trace[0], 0, &[], &[]), shown(&m, 0, &[("Kc".into(), "Vc".into())])));
    let split = vm_ints(&format!("{src}{}", shown(&m, 0, &chip_caches(1))));
    assert_eq!(split, whole);
}

/// Each board's fabric target, as its images are printed, on two boards joined by a link.
fn board_targets() -> [SpatialTarget; 2] {
    [SpatialTarget::xc7a100t_fabric(16, 64).linked(), SpatialTarget::xc7a200t_fabric(16, 64).linked()]
}

/// The split step at `pos` lowered for `target` as one program over its chips, with its manifest.
fn lowered_split(m: &Int8Model, tok: usize, pos: usize, kc: &[i64], vc: &[i64], target: &SpatialTarget) -> (SpatialProgram, TpManifest) {
    let (src, manifest) = m.decode_step_tp_lg(tok, pos, kc, vc, 2).unwrap();
    let pl = compile_program_to_spatial_tp(&src, target, &manifest).expect("the walk lifts the split step");
    let p = pl.whole().unwrap_or_else(|| panic!("{}: the split step at position {pos} lowers whole: {}", target.name, pl.refusal())).clone();
    (p, manifest)
}

/// Each named tensor's values after the evaluator runs `p` from its fills.
fn evaluated(p: &SpatialProgram, names: &[&str]) -> Vec<Vec<i64>> {
    let out = eval_choreography_every_tensor(p, &inputs_from_fills(p)).expect("the program evaluates");
    names
        .iter()
        .map(|name| {
            let t = p.tensors.iter().find(|t| t.name == *name).unwrap_or_else(|| panic!("{}: a tensor {name}", p.name));
            match &out.tensors[&t.id] {
                Value::Int(v) => v.clone(),
                Value::Float(v) => panic!("{name} through floats: {v:?}"),
            }
        })
        .collect()
}

/// For each board, at every position of the trace: the split step's program, lowered as one program over both chips,
/// evaluates to the step's own program's next token, logits and residuals, and writes the step's cache row across its
/// chips' caches.
#[test]
fn the_two_chip_step_lowers_whole_for_each_board_and_evaluates_to_the_step() {
    let (m, trace) = model_and_trace();
    let w = m.d / 2;
    for target in board_targets() {
        let (mut kc, mut vc) = (Vec::new(), Vec::new());
        for (pos, tok) in trace.iter().enumerate() {
            let whole = compile_program_to_spatial(&m.decode_step_lg(*tok, pos, &kc, &vc), &target)
                .unwrap()
                .program
                .unwrap_or_else(|| panic!("{}: the step at position {pos} lowers whole", target.name));
            let want = evaluated(&whole, &["next", "logits", "s1", "s2", "Kc", "Vc"]);
            let (split, _) = lowered_split(&m, *tok, pos, &kc, &vc, &target);
            let got = evaluated(&split, &["next", "logits", "s1", "s2", "Kc_c0", "Kc_c1", "Vc_c0", "Vc_c1"]);
            assert_eq!(got[..4], want[..4], "{} position {pos}: next, logits and residuals", target.name);
            for (cache, halves) in [(&want[4], [&got[4], &got[5]]), (&want[5], [&got[6], &got[7]])] {
                let row: Vec<i64> = halves.iter().flat_map(|h| h[pos * w..(pos + 1) * w].to_vec()).collect();
                assert_eq!(row, cache[pos * m.d..(pos + 1) * m.d], "{} position {pos}: the cache row across the chips", target.name);
            }
            kc.extend_from_slice(&want[4][pos * m.d..(pos + 1) * m.d]);
            vc.extend_from_slice(&want[5][pos * m.d..(pos + 1) * m.d]);
        }
    }
}

/// In the lowered split every tensor the manifest names lives on its chip, and each all-reduce is one stage on chip 0,
/// summing in integers, fed by a load of each partial on that partial's chip.
#[test]
fn the_lowered_split_keeps_each_tensor_on_its_chip_and_sums_on_the_root() {
    let (m, trace) = model_and_trace();
    for target in board_targets() {
        let rows = vec![0; 3 * m.d];
        let (p, manifest) = lowered_split(&m, trace[3], 3, &rows, &rows, &target);
        for t in &p.tensors {
            if let Some(c) = manifest.chip_of.get(&t.name) {
                assert_eq!(t.chip, Some(ChipId(*c)), "{}: {} lives on chip {c}", target.name, t.name);
            }
        }
        let reduces: Vec<_> = p.stages.iter().filter(|s| matches!(s.kind, StageKind::AllReduce { .. })).collect();
        assert_eq!(reduces.len(), manifest.collectives.len(), "{}: one all-reduce stage per collective", target.name);
        for r in &reduces {
            assert_eq!(r.site.map(|s| s.chip), Some(0), "{}: {} sums on chip 0", target.name, r.name);
            for b in &r.ins {
                let buf = &p.buffers[b.idx()];
                assert_eq!(buf.dtype, Precision::Int64, "{}: {} sums integers", target.name, r.name);
            }
        }
        for col in &manifest.collectives {
            for (c, part) in col.partials.iter().enumerate() {
                let t = p.tensors.iter().find(|t| t.name == *part).unwrap_or_else(|| panic!("the partial {part}"));
                let load = p
                    .stages
                    .iter()
                    .find(|s| matches!(&s.kind, StageKind::Load { tensor, .. } if *tensor == t.id) && p.edges.iter().any(|e| e.from == s.id && reduces.iter().any(|r| e.to.contains(&r.id))))
                    .unwrap_or_else(|| panic!("{}: a load of {part} feeding its all-reduce", target.name));
                assert_eq!(load.site.map(|s| s.chip), Some(c as u16), "{}: {part} is loaded on chip {c}", target.name);
            }
        }
    }
}

/// A board alone has no link for a partial to cross: the split's all-reduces are refused, naming the missing hand-off.
#[test]
fn one_board_alone_refuses_the_split_for_want_of_a_link() {
    let (m, trace) = model_and_trace();
    let (src, manifest) = m.decode_step_tp_lg(trace[0], 0, &[], &[], 2).unwrap();
    for target in [SpatialTarget::xc7a100t_fabric(16, 64), SpatialTarget::xc7a200t_fabric(16, 64)] {
        let pl = compile_program_to_spatial_tp(&src, &target, &manifest).expect("the walk lifts the split step");
        assert!(pl.whole().is_none(), "{}: no program over two chips without a link", target.name);
        let refused: Vec<&str> = pl.missing.iter().map(|x| x.binder.as_str()).collect();
        assert_eq!(refused, ["o_a", "f_a"], "{}: exactly the all-reduces are refused", target.name);
        for x in &pl.missing {
            let want = format!("{:?}", TpReject::NoCrossing(ShardError::NoCrossChipHandshake { target: target.name }));
            assert!(x.why.contains(&want), "{}: {} names the missing crossing: {}", target.name, x.binder, x.why);
        }
    }
}

/// The split step at `pos` lowered for the boards: a program per chip and the transfers between them.
fn lowered_chips(m: &Int8Model, tok: usize, pos: usize, kc: &[i64], vc: &[i64], target: &SpatialTarget) -> (Vec<SpatialProgram>, Vec<ChipTransfer>) {
    let (src, manifest) = m.decode_step_tp_lg(tok, pos, kc, vc, 2).unwrap();
    let lowered = compile_program_to_chips(&src, target, &manifest).expect("the walk lifts the split step");
    let chips = lowered
        .chips
        .iter()
        .enumerate()
        .map(|(c, pl)| pl.whole().unwrap_or_else(|| panic!("{}: chip {c}'s program at position {pos} lowers whole: {}", target.name, pl.refusal())).clone())
        .collect();
    (chips, lowered.transfers)
}

/// Each board's program is an ordinary program on one site, and the tensors that cross are exactly the four the split
/// makes, in the order the step makes them.
#[test]
fn the_split_lowers_as_a_program_a_board_and_four_transfers() {
    let (m, trace) = model_and_trace();
    for target in board_targets() {
        let (chips, transfers) = lowered_chips(&m, trace[2], 2, &vec![0; 2 * m.d], &vec![0; 2 * m.d], &target);
        assert_eq!(chips.len(), 2, "{}: a program a chip", target.name);
        for (c, p) in chips.iter().enumerate() {
            let sites: std::collections::BTreeSet<_> = p.stages.iter().filter_map(|s| s.site).collect();
            assert_eq!(sites.len(), 1, "{}: chip {c}'s program runs on one site: {sites:?}", target.name);
            assert!(sites.iter().all(|s| s.chip == 0), "{}: chip {c}'s program is a one-chip program", target.name);
            assert!(!p.stages.iter().any(|s| matches!(s.kind, StageKind::AllReduce { .. })), "{}: chip {c} folds locally, no collective stage", target.name);
            assert!(p.certificate.as_ref().is_some_and(|cert| cert.recheck(p)), "{}: chip {c}'s program carries a certificate that rechecks, its halts on it", target.name);
        }
        let got: Vec<(&str, u16, u16)> = transfers.iter().map(|t| (t.tensor.as_str(), t.from, t.to)).collect();
        assert_eq!(got, [("xi", 0, 1), ("o_a_c1", 1, 0), ("hi", 0, 1), ("f_a_c1", 1, 0)], "{}: the tensors that cross", target.name);
        for t in &transfers {
            let reads = |p: &SpatialProgram| p.tensors.iter().any(|x| x.name == t.tensor);
            assert!(reads(&chips[t.to as usize]), "{}: chip {} holds {} to read it", target.name, t.to, t.tensor);
        }
    }
}

/// At every position, for each board: the composed program's values of the tensors a chip receives, put into that
/// chip's program as it starts, leave every tensor the chip's program holds at the composed program's value.
#[test]
fn each_board_s_program_given_what_it_receives_holds_the_composed_values() {
    let (m, trace) = model_and_trace();
    for target in board_targets() {
        let (mut kc, mut vc) = (Vec::new(), Vec::new());
        for (pos, tok) in trace.iter().enumerate() {
            let (whole, _) = lowered_split(&m, *tok, pos, &kc, &vc, &target);
            let composed = eval_choreography_every_tensor(&whole, &inputs_from_fills(&whole)).expect("the composed program evaluates");
            let value = |name: &str| -> Vec<i64> {
                let t = whole.tensors.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("the composed program's {name}"));
                match &composed.tensors[&t.id] {
                    Value::Int(v) => v.clone(),
                    Value::Float(v) => panic!("{name} through floats: {v:?}"),
                }
            };
            let (chips, transfers) = lowered_chips(&m, *tok, pos, &kc, &vc, &target);
            for (c, p) in chips.iter().enumerate() {
                // What the chip starts from: each tensor it receives as the composed program holds it, every other tensor
                // its own fill.
                let received: Vec<&str> = transfers.iter().filter(|t| t.to as usize == c).map(|t| t.tensor.as_str()).collect();
                let mut inputs = Inputs::default();
                for t in p.tensors.iter().filter(|t| t.role != TensorRole::Output) {
                    let v = if received.contains(&t.name.as_str()) {
                        value(&t.name)
                    } else {
                        (0..t.elems.rows * t.elems.cols).map(|i| t.init.element(i).unwrap_or_else(|| panic!("chip {c}'s {} has a fill", t.name))).collect()
                    };
                    inputs.tensors.insert(t.id, Value::Int(v));
                }
                let out = eval_choreography_every_tensor(p, &inputs).expect("the chip's program evaluates");
                let mut held = 0;
                for t in &p.tensors {
                    if !whole.tensors.iter().any(|w| w.name == t.name) {
                        continue;
                    }
                    let Some(Value::Int(got)) = out.tensors.get(&t.id) else { continue };
                    assert_eq!(got, &value(&t.name), "{} position {pos}: chip {c}'s {}", target.name, t.name);
                    held += 1;
                }
                assert!(held > 10, "{} position {pos}: chip {c} is held to the composed program on {held} tensors", target.name);
            }
            let (k, v) = (value("Kc_c0"), value("Vc_c0"));
            let (k1, v1) = (value("Kc_c1"), value("Vc_c1"));
            let w = m.d / 2;
            kc.extend(k[pos * w..(pos + 1) * w].iter().chain(&k1[pos * w..(pos + 1) * w]));
            vc.extend(v[pos * w..(pos + 1) * w].iter().chain(&v1[pos * w..(pos + 1) * w]));
        }
    }
}

/// A program's tensors by id, as names.
fn names(p: &SpatialProgram, ids: &[logicaffeine_compile::spatial::ir::TensorId]) -> Vec<String> {
    ids.iter().map(|t| p.tensor(*t).name.clone()).collect()
}

/// The sections of `p` that store into, and that load from, the tensor named `name`.
fn sections_touching(p: &SpatialProgram, name: &str) -> (Vec<usize>, Vec<usize>) {
    let (mut stores, mut loads) = (Vec::new(), Vec::new());
    for (k, sec) in p.sections.iter().enumerate() {
        for st in &p.stages[sec.stages.0..sec.stages.1] {
            match &st.kind {
                StageKind::Store { tensor, .. } if p.tensor(*tensor).name == name => stores.push(k),
                StageKind::Load { tensor, .. } if p.tensor(*tensor).name == name => loads.push(k),
                _ => {}
            }
        }
    }
    stores.dedup();
    loads.dedup();
    (stores, loads)
}

/// Each board halts exactly where a tensor crosses: chip 0 after `xi`, before the attention's fold, after `hi`, before
/// the FFN's fold; chip 1 before its first section, after its attention partial (where it also takes `hi`), and at its
/// end. A send halts after the last section that stores the tensor; a receive halts right before the first that loads
/// it.
#[test]
fn each_board_halts_where_a_tensor_crosses() {
    let (m, trace) = model_and_trace();
    for target in board_targets() {
        let (chips, transfers) = lowered_chips(&m, trace[4], 4, &vec![0; 4 * m.d], &vec![0; 4 * m.d], &target);
        let shown: Vec<Vec<(Vec<String>, Vec<String>)>> = chips
            .iter()
            .map(|p| p.links.iter().map(|l| (names(p, &l.sends), names(p, &l.receives))).collect())
            .collect();
        let v = |xs: &[&str]| -> Vec<String> { xs.iter().map(|x| x.to_string()).collect() };
        assert_eq!(
            shown[0],
            [(v(&["xi"]), v(&[])), (v(&[]), v(&["o_a_c1"])), (v(&["hi"]), v(&[])), (v(&[]), v(&["f_a_c1"]))],
            "{}: chip 0's halts",
            target.name
        );
        assert_eq!(shown[1], [(v(&[]), v(&["xi"])), (v(&["o_a_c1"]), v(&["hi"])), (v(&["f_a_c1"]), v(&[]))], "{}: chip 1's halts", target.name);
        assert_eq!(chips[1].links[0].after, None, "{}: chip 1 takes xi before its first section", target.name);
        assert_eq!(chips[1].links[2].after, Some(chips[1].sections.len() - 1), "{}: chip 1 sends f_a_c1 at its end", target.name);
        for (c, p) in chips.iter().enumerate() {
            for l in &p.links {
                for t in names(p, &l.sends) {
                    let (stores, _) = sections_touching(p, &t);
                    assert_eq!(l.after, stores.last().copied(), "{} chip {c}: {t} is sent after the last section that stores it", target.name);
                }
                for t in names(p, &l.receives) {
                    let (stores, loads) = sections_touching(p, &t);
                    assert!(stores.is_empty(), "{} chip {c}: {t}, which it receives, is no section's to store", target.name);
                    let first = *loads.first().unwrap_or_else(|| panic!("{} chip {c}: {t} is read", target.name));
                    assert_eq!(l.after, first.checked_sub(1), "{} chip {c}: {t} is received right before the first section that loads it", target.name);
                }
            }
        }
        for t in &transfers {
            let count = |c: u16, sends: bool| -> usize {
                let p = &chips[c as usize];
                p.links.iter().map(|l| names(p, if sends { &l.sends } else { &l.receives }).iter().filter(|n| **n == t.tensor).count()).sum()
            };
            assert_eq!((count(t.from, true), count(t.to, false)), (1, 1), "{}: {} is sent once and received once", target.name, t.tensor);
        }
    }
}

/// The relay the host runs: every chip run to its end, each transfer copied once — after its sender has run past the
/// halt that sends it, before its receiver runs past the halt that receives it — and both chips' attention started
/// before the host waits on either: a run starts or continues a chip, and the host waits on a chip only when it next
/// copies from it or runs it, so both runs to the halt that receives or sends `o_a_c1` come before its copy.
#[test]
fn the_host_relay_moves_each_transfer_once_and_runs_the_chips_at_once() {
    let (m, trace) = model_and_trace();
    for target in board_targets() {
        let (chips, transfers) = lowered_chips(&m, trace[4], 4, &vec![0; 4 * m.d], &vec![0; 4 * m.d], &target);
        let relay = relay_schedule(&chips, &transfers).unwrap_or_else(|e| panic!("{}: the relay completes: {e}", target.name));
        let mut reached = vec![None::<usize>; chips.len()];
        let mut copied: Vec<&str> = Vec::new();
        for step in &relay {
            match step {
                RelayStep::Run { chip, to } => {
                    let c = *chip as usize;
                    assert!(reached[c].map_or(true, |r| *to > r), "{}: chip {c} runs forward", target.name);
                    let p = &chips[c];
                    let from = reached[c].map_or(0, |r| r);
                    for l in &p.links[from..*to] {
                        for t in names(p, &l.receives) {
                            assert!(copied.contains(&t.as_str()), "{}: chip {c} runs past its receipt of {t} only after it is copied", target.name);
                        }
                    }
                    reached[c] = Some(*to);
                }
                RelayStep::Copy { tensor, from, to } => {
                    let t = transfers.iter().find(|t| t.tensor == *tensor && t.from == *from && t.to == *to).unwrap_or_else(|| panic!("{}: {tensor} is a transfer", target.name));
                    let p = &chips[t.from as usize];
                    let sent_at = p.links.iter().position(|l| names(p, &l.sends).contains(tensor)).unwrap();
                    assert!(reached[t.from as usize].is_some_and(|r| r >= sent_at), "{}: {tensor} is copied after chip {from} reaches the halt that sends it", target.name);
                    assert!(!copied.contains(&tensor.as_str()), "{}: {tensor} is copied once", target.name);
                    copied.push(tensor);
                }
            }
        }
        for (c, p) in chips.iter().enumerate() {
            assert_eq!(reached[c], Some(p.links.len()), "{}: chip {c} runs to its end", target.name);
        }
        let mut want: Vec<&str> = transfers.iter().map(|t| t.tensor.as_str()).collect();
        want.sort();
        copied.sort();
        assert_eq!(copied, want, "{}: every transfer is copied", target.name);
        let oa = relay.iter().position(|s| matches!(s, RelayStep::Copy { tensor, .. } if tensor == "o_a_c1")).unwrap();
        for c in 0..2u16 {
            let p = &chips[c as usize];
            let halt = p.links.iter().position(|l| names(p, &l.sends).iter().chain(&names(p, &l.receives)).any(|n| n == "o_a_c1")).unwrap();
            assert!(relay[..oa].iter().any(|s| matches!(s, RelayStep::Run { chip, to } if *chip == c && *to == halt)), "{}: chip {c} runs its attention before the host waits for o_a_c1", target.name);
        }
    }
}

/// A chip program's halts: the links after a section a later section follows, where the fabric stops for the host.
fn halts(p: &SpatialProgram) -> Vec<usize> {
    (0..p.links.len()).filter(|i| p.links[*i].after.is_some_and(|k| k + 1 < p.sections.len())).collect()
}

/// The twin counts the cycles to each halt and to the end, increasing; the last is the whole program's count. The step
/// on one chip has no halts and one count.
#[test]
fn each_chip_s_twin_counts_its_halts_and_its_end() {
    let (m, trace) = model_and_trace();
    for target in board_targets() {
        let whole = compile_program_to_spatial(&m.decode_step_lg(trace[0], 0, &[], &[]), &target).unwrap().program.unwrap();
        assert!(whole.links.is_empty(), "{}: the one-chip step halts nowhere", target.name);
        assert_eq!(fabric_cycle_segments(&whole, &target), Some(vec![fabric_cycle_model(&whole, &target)]), "{}: one count, the step's", target.name);
        let (chips, _) = lowered_chips(&m, trace[4], 4, &vec![0; 4 * m.d], &vec![0; 4 * m.d], &target);
        for (c, p) in chips.iter().enumerate() {
            let seg = fabric_cycle_segments(p, &target).unwrap_or_else(|| panic!("{}: chip {c}'s twin", target.name));
            assert_eq!(seg.len(), halts(p).len() + 1, "{}: chip {c} counts each halt and its end", target.name);
            assert!(seg.windows(2).all(|w| w[0] < w[1]), "{}: chip {c}'s counts increase: {seg:?}", target.name);
            assert_eq!(*seg.last().unwrap(), fabric_cycle_model(p, &target), "{}: chip {c}'s last count is its whole run", target.name);
        }
    }
}

const CPB: u32 = 4;

/// Each element of tensor `name` in `p`, as `(word, element)` in row-major order.
fn words_of(p: &SpatialProgram, name: &str) -> Vec<usize> {
    let t = p.tensors.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("{}: a tensor {name}", p.name));
    (0..t.elems.rows * t.elems.cols).map(|i| host_word(p, t, i / t.elems.cols, i % t.elems.cols)).collect()
}

/// Each board, driven through its halts by the relay's protocol on its serial line: before it starts, the words it
/// receives first are written; each `X` runs it to its next halt and replies the halt and the cycles, the twin's; at a
/// halt each word it sends is read back, the composed program's, and each word it receives next is written; the last
/// `X` replies its end; then what it sends at its end and its result are read back. Two positions: the first, and one
/// with a cache behind it.
#[test]
fn each_board_driven_through_its_halts_replies_the_composed_values_in_the_twin_s_cycles() {
    if !common::require_tool("iverilog") {
        return;
    }
    let (m, trace) = model_and_trace();
    let le = |v: u64, n: usize| -> Vec<u8> { v.to_le_bytes()[..n].to_vec() };
    for target in board_targets() {
        let lat = target.site.local_mem_read_latency.unwrap() as u64;
        let (mut kc, mut vc) = (Vec::new(), Vec::new());
        for (pos, tok) in trace.iter().enumerate().take(5) {
            let (whole, _) = lowered_split(&m, *tok, pos, &kc, &vc, &target);
            let composed = eval_choreography_every_tensor(&whole, &inputs_from_fills(&whole)).expect("the composed program evaluates");
            let value = |name: &str| -> Vec<i64> {
                let t = whole.tensors.iter().find(|t| t.name == name).unwrap();
                match &composed.tensors[&t.id] {
                    Value::Int(v) => v.clone(),
                    Value::Float(v) => panic!("{name} through floats: {v:?}"),
                }
            };
            if pos == 0 || pos == 4 {
                let (chips, _) = lowered_chips(&m, *tok, pos, &kc, &vc, &target);
                for (c, p) in chips.iter().enumerate() {
                    let seg = fabric_cycle_segments(p, &target).unwrap();
                    let mut script: Vec<(Vec<u8>, usize)> = vec![(b"I".to_vec(), 6)];
                    let mut want: Vec<Option<Vec<u8>>> = vec![None];
                    let write = |script: &mut Vec<(Vec<u8>, usize)>, want: &mut Vec<Option<Vec<u8>>>, name: &str| {
                        let t = p.tensors.iter().find(|t| t.name == name).unwrap();
                        for (w, v) in words_of(p, name).into_iter().zip(value(name)) {
                            let mut cmd = b"W".to_vec();
                            cmd.extend(le(t.id.idx() as u64, 2));
                            cmd.extend(le(w as u64, 4));
                            cmd.extend(le(v as u64, 8));
                            script.push((cmd, 1));
                            want.push(Some(b"k".to_vec()));
                        }
                    };
                    let read = |script: &mut Vec<(Vec<u8>, usize)>, want: &mut Vec<Option<Vec<u8>>>, name: &str| {
                        let t = p.tensors.iter().find(|t| t.name == name).unwrap();
                        for (w, v) in words_of(p, name).into_iter().zip(value(name)) {
                            let mut cmd = b"R".to_vec();
                            cmd.extend(le(t.id.idx() as u64, 2));
                            cmd.extend(le(w as u64, 4));
                            script.push((cmd, 9));
                            let mut r = b"r".to_vec();
                            r.extend(le(v as u64, 8));
                            want.push(Some(r));
                        }
                    };
                    let named = |ids: &[logicaffeine_compile::spatial::ir::TensorId]| names(p, ids);
                    if let Some(l) = p.links.first().filter(|l| l.after.is_none()) {
                        for t in named(&l.receives) {
                            write(&mut script, &mut want, &t);
                        }
                    }
                    for (n, i) in halts(p).into_iter().enumerate() {
                        script.push((b"X".to_vec(), 7));
                        let mut r = b"x".to_vec();
                        r.extend(le(i as u64, 2));
                        r.extend(le(seg[n], 4));
                        want.push(Some(r));
                        for t in named(&p.links[i].sends) {
                            read(&mut script, &mut want, &t);
                        }
                        for t in named(&p.links[i].receives) {
                            write(&mut script, &mut want, &t);
                        }
                    }
                    script.push((b"X".to_vec(), 7));
                    let mut r = b"x".to_vec();
                    r.extend(le(0xffff, 2));
                    r.extend(le(*seg.last().unwrap(), 4));
                    want.push(Some(r));
                    if let Some(l) = p.links.last().filter(|l| l.after == Some(p.sections.len() - 1)) {
                        for t in named(&l.sends) {
                            read(&mut script, &mut want, &t);
                        }
                    }
                    if c == 0 {
                        read(&mut script, &mut want, "next");
                    }
                    let art = match FabricBackend::new(target.clone()).lower(p) {
                        Ok(SpatialArtifact::Verilog(v)) => v,
                        other => panic!("{}: chip {c} prints: {other:?}", target.name),
                    };
                    let dir = std::env::temp_dir().join(format!("decode_tp_board_{}_{}_{c}_{pos}", std::process::id(), target.name));
                    let bound = (seg.last().unwrap() + 2000) * 4 + script.len() as u64 * 400 * CPB as u64;
                    let replies = fabric_rtl::simulate_board_module(&dir, p, &art, &inputs_from_fills_or_zero(p), "chip_board", &chip_board_verilog(lat), CPB, &script, bound);
                    for (k, (got, want)) in replies.iter().zip(&want).enumerate() {
                        if let Some(w) = want {
                            assert_eq!(got, w, "{} position {pos} chip {c}: reply {k} to {:?}", target.name, script[k].0);
                        } else {
                            assert_eq!(got[..5], *b"iLOGS", "{} position {pos} chip {c}: it identifies itself", target.name);
                        }
                    }
                    let _ = std::fs::remove_dir_all(&dir);
                }
            }
            let (k0, v0, k1, v1) = (value("Kc_c0"), value("Vc_c0"), value("Kc_c1"), value("Vc_c1"));
            let w = m.d / 2;
            kc.extend(k0[pos * w..(pos + 1) * w].iter().chain(&k1[pos * w..(pos + 1) * w]));
            vc.extend(v0[pos * w..(pos + 1) * w].iter().chain(&v1[pos * w..(pos + 1) * w]));
        }
    }
}

/// A fabric with `links` halts, `run` cycles apart, that leaves a halt `lag` cycles after `link_go`: `spatial_top`'s ports
/// as a linked program prints them, around a counter. Its count at link `k` is `run · (k + 1)` and at its end
/// `run · (links + 1)`, paused while halted.
fn lagging_fabric(run: u32, links: u32, lag: u32) -> String {
    format!(
        "module spatial_top(input clk, input rst, input start, output reg done, output reg [31:0] cycles,
  input host_we, input host_re, input [15:0] host_site, input [15:0] host_tensor, input [31:0] host_addr,
  input [63:0] host_wdata, output [63:0] host_rdata, input link_go, output halted, output [15:0] halt_at);
  localparam RUN = {run}, LINKS = {links}, LAG = {lag};
  reg running, at_halt; reg [15:0] link; reg [15:0] leave;
  assign halted = at_halt; assign halt_at = link; assign host_rdata = 64'd0;
  always @(posedge clk) begin
    if (rst) begin running <= 1'b0; done <= 1'b0; cycles <= 32'd0; at_halt <= 1'b0; link <= 16'd0; leave <= 16'd0; end
    else if (start) begin running <= 1'b1; done <= 1'b0; cycles <= 32'd0; end
    else if (running) begin
      if (at_halt) begin
        if (leave == 16'd0 && link_go) begin
          if (LAG == 0) begin at_halt <= 1'b0; link <= link + 16'd1; end else leave <= LAG;
        end else if (leave == 16'd1) begin at_halt <= 1'b0; link <= link + 16'd1; leave <= 16'd0; end
        else if (leave != 16'd0) leave <= leave - 16'd1;
      end else if (link < LINKS && cycles + 32'd1 == RUN * (link + 32'd1)) begin cycles <= cycles + 32'd1; at_halt <= 1'b1; end
      else if (cycles + 32'd1 == RUN * (LINKS + 32'd1)) begin cycles <= cycles + 32'd1; running <= 1'b0; done <= 1'b1; end
      else cycles <= cycles + 32'd1;
    end
  end
endmodule
"
    )
}

/// A board's `X` continues its fabric past a halt and replies only once the fabric has left it, however many cycles the
/// fabric takes to leave: it never answers a continue with the halt it was told to leave. Each `X` replies the next halt
/// and its count, then the end, for a fabric that leaves at once and for ones that leave a cycle or many later.
#[test]
fn a_board_replies_past_each_halt_however_long_its_fabric_takes_to_leave_it() {
    if !common::require_tool("iverilog") {
        return;
    }
    let le = |v: u64, n: usize| -> Vec<u8> { v.to_le_bytes()[..n].to_vec() };
    let (run, links) = (37u32, 3u32);
    for lag in [0u32, 1, 2, 5, 40] {
        let mut script: Vec<(Vec<u8>, usize)> = Vec::new();
        let mut want: Vec<Vec<u8>> = Vec::new();
        for k in 0..=links {
            script.push((b"X".to_vec(), 7));
            let mut r = b"x".to_vec();
            r.extend(le(if k < links { k as u64 } else { 0xffff }, 2));
            r.extend(le((run * (k + 1)) as u64, 4));
            want.push(r);
        }
        let dir = std::env::temp_dir().join(format!("decode_tp_board_lag_{}_{lag}", std::process::id()));
        let bound = (run as u64 + lag as u64 + 100) * (links as u64 + 1) + script.len() as u64 * 400 * CPB as u64;
        let sources = vec![
            ("spatial_top.v".to_string(), lagging_fabric(run, links, lag)),
            ("board.v".to_string(), chip_board_verilog(1)),
            ("tb.v".to_string(), fabric_rtl::board_testbench_of("chip_board", Some(CPB), CPB, &script, bound)),
        ];
        let replies = fabric_rtl::run_board(&dir, &sources, &script, bound);
        assert_eq!(replies, want, "a fabric that leaves its halt {lag} cycles after it is told to");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A chip program's memory images: each tensor's fill, and zero where it has none — a tensor it receives, which the
/// relay writes before any section reads it.
fn inputs_from_fills_or_zero(p: &SpatialProgram) -> Inputs {
    let mut inputs = Inputs::default();
    for t in p.tensors.iter().filter(|t| t.role != TensorRole::Output) {
        let v = (0..t.elems.rows * t.elems.cols).map(|i| t.init.element(i).unwrap_or(0)).collect();
        inputs.tensors.insert(t.id, Value::Int(v));
    }
    inputs
}

fn repo() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `scripts/split-host.py` with `args`: its exit status, what it printed and what it reported.
fn split_host(args: &[&str]) -> (bool, String, String) {
    let out = std::process::Command::new("python3").arg(repo().join("scripts/split-host.py")).args(args).output().expect("python3");
    (out.status.success(), String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string())
}

/// A split ledger as the boards' images state it: chip 0 lowered for Board A, chip 1 for Board B, each its compiler
/// record beside the fields a routed image adds (here placeholders: no bitstream is read by a replay).
fn split_ledger(m: &Int8Model, prompt: &[usize]) -> serde_json::Value {
    let targets = [SpatialTarget::xc7a200t_fabric(16, 64).linked(), SpatialTarget::xc7a100t_fabric(16, 64).linked()];
    let (src, manifest) = m.decode_step_tp_lg(prompt[0], 0, &[], &[], 2).unwrap();
    let mut chips = Vec::new();
    let mut transfers = Vec::new();
    for (c, target) in targets.iter().enumerate() {
        let lowered = compile_program_to_chips(&src, target, &manifest).unwrap();
        chips.push(lowered.chips[c].whole().unwrap().clone());
        transfers = lowered.transfers;
    }
    let relay = relay_schedule(&chips, &transfers).unwrap();
    let want = spatial_support::prompt_next_tokens(m, prompt, &targets[0]);
    let records: Vec<serde_json::Value> = chips
        .iter()
        .enumerate()
        .map(|(c, p)| {
            let mut r = chip_record(p, &targets[c]).unwrap();
            let o = r.as_object_mut().unwrap();
            o.insert("chip".into(), serde_json::json!(c));
            o.insert("board".into(), serde_json::json!(["A", "B"][c]));
            o.insert("part".into(), serde_json::json!(["xc7a200tsbg484-1", "xc7a100tcsg324-1"][c]));
            o.insert("bitstream".into(), serde_json::json!("none.bit"));
            o.insert("sha256".into(), serde_json::json!("none"));
            o.insert("read_latency".into(), serde_json::json!(1));
            o.insert("core_mhz".into(), serde_json::json!(40.0));
            o.insert("prompt_next_tokens".into(), if c == 0 { serde_json::json!(want) } else { serde_json::Value::Null });
            r
        })
        .collect();
    serde_json::json!({
        "transfers": transfers.iter().map(|t| serde_json::json!({ "tensor": t.tensor, "from": t.from, "to": t.to })).collect::<Vec<_>>(),
        "relay": relay_record(&relay),
        "chips": records,
    })
}

/// The replies each board gives the commands the host sends it, as its image's record predicts them: `I` its identity,
/// `W` an acknowledgement, `X` its next halt with the twin's count — its halts in order, then its end — and `R` of the
/// result the token at that step; every other read, a copy's, any value.
fn predicted_replies(ledger: &serde_json::Value, commands: &str, next_tokens: &[usize]) -> String {
    let chips = ledger["chips"].as_array().unwrap();
    let chip_of = |board: &str| chips.iter().find(|c| c["board"] == board).unwrap();
    let mut xs: std::collections::BTreeMap<String, usize> = Default::default();
    let mut nexts = next_tokens.iter();
    let mut out = String::new();
    for line in commands.lines() {
        let (board, hex) = line.split_once(' ').unwrap();
        let cmd: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        let chip = chip_of(board);
        let reply: Vec<u8> = match cmd[0] {
            b'I' => b"iLOGS\x01".to_vec(),
            b'W' => b"k".to_vec(),
            b'X' => {
                let links = chip["links"].as_array().unwrap();
                let segs: Vec<u64> = chip["segments"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
                let halts: Vec<usize> = (0..links.len()).filter(|i| links[*i]["halts"] == true).collect();
                let k = xs.entry(board.to_string()).or_default();
                let at = *k % (halts.len() + 1);
                *k += 1;
                let (link, cycles) = if at < halts.len() { (halts[at] as u64, segs[at]) } else { (0xffff, segs[segs.len() - 1]) };
                let mut r = b"x".to_vec();
                r.extend((link as u16).to_le_bytes());
                r.extend((cycles as u32).to_le_bytes());
                r
            }
            b'R' => {
                let tensor = u16::from_le_bytes([cmd[1], cmd[2]]) as u64;
                let value = if chip["next"]["tensor"].as_u64() == Some(tensor) { *nexts.next().unwrap() as u64 + 1 } else { 0 };
                let mut r = b"r".to_vec();
                r.extend(value.to_le_bytes());
                r
            }
            other => panic!("a command {other:#x} the split host does not send"),
        };
        out.push_str(&format!("{board} {}\n", reply.iter().map(|b| format!("{b:02x}")).collect::<String>()));
    }
    out
}

/// The split host runs the trace through the relay and records the trace's tokens; it refuses a halt at another link, a
/// count other than the twin's, a prompt step's other token and another generated token, each by name.
#[test]
fn the_split_host_runs_the_relay_and_holds_every_reply() {
    if !common::require_tool("python3") {
        return;
    }
    let path = repo().join("benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let m = Int8Model::from_json(&j).unwrap();
    let toks = |k: &str| -> Vec<usize> { j["golden_trace"][k].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as usize).collect() };
    let (prompt, generated) = (toks("prompt_tokens"), toks("generated_tokens"));
    let ledger = split_ledger(&m, &prompt);
    let dir = std::env::temp_dir().join(format!("decode_split_host_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let images = dir.join("decode_split_images.json");
    std::fs::write(&images, ledger.to_string()).unwrap();
    let (ok, commands, err) = split_host(&["--dry-run", "--images", images.to_str().unwrap()]);
    assert!(ok, "the split host's dry run: {err}");
    let want: Vec<usize> = ledger["chips"][0]["prompt_next_tokens"].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as usize).collect();
    let mut next_tokens = want.clone();
    next_tokens.extend(&generated[1..]);
    let run = |replies: &str, tag: &str| -> (bool, String, String) {
        let file = dir.join(format!("replies_{tag}.txt"));
        std::fs::write(&file, replies).unwrap();
        split_host(&["--replay", file.to_str().unwrap(), "--images", images.to_str().unwrap()])
    };
    let good = predicted_replies(&ledger, &commands, &next_tokens);
    let (ok, out, err) = run(&good, "board");
    assert!(ok, "the split host accepts the boards' predicted run: {err}");
    let rec: serde_json::Value = serde_json::from_str(&out).expect("the host's record");
    let got = |k: &str| -> Vec<usize> { rec[k].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as usize).collect() };
    assert_eq!(got("tokens"), generated, "the trace's generated tokens");
    assert_eq!(got("prompt_next_tokens"), want, "the evaluator's next token at every prompt position");
    // Tamper with one reply: the first X of board A (a halt), a count, the result at position 4, the last result.
    let lines: Vec<&str> = good.lines().collect();
    let edit = |pick: &dyn Fn(usize, &str) -> bool, change: &dyn Fn(&str) -> String| -> String {
        let mut done = false;
        lines
            .iter()
            .enumerate()
            .map(|(i, l)| {
                if !done && pick(i, l) {
                    done = true;
                    change(l)
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    };
    let flip = |l: &str, at: usize| -> String {
        let (b, h) = l.split_once(' ').unwrap();
        let mut bytes: Vec<u8> = (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect();
        bytes[at] ^= 1;
        format!("{b} {}", bytes.iter().map(|x| format!("{x:02x}")).collect::<String>())
    };
    let (ok, _, err) = run(&edit(&|_, l| l.starts_with("A 78"), &|l| flip(l, 1)), "halt");
    assert!(!ok && err.contains("halted at"), "a halt at another link is refused by name: {err}");
    let (ok, _, err) = run(&edit(&|_, l| l.starts_with("B 78"), &|l| flip(l, 3)), "cycles");
    assert!(!ok && err.contains("cycles"), "a count other than the twin's is refused by name: {err}");
    let reads: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.starts_with("A 72") && **l != "A 720000000000000000").map(|(i, _)| i).collect();
    let (ok, _, err) = run(&edit(&|i, _| i == reads[4], &|l| flip(l, 1)), "prompt");
    assert!(!ok && err.contains("position 4"), "a prompt step's other token is refused at its position: {err}");
    let (ok, _, err) = run(&edit(&|i, _| i == *reads.last().unwrap(), &|l| flip(l, 1)), "generated");
    assert!(!ok && err.contains("generated"), "another generated token is refused by name: {err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Wider than the model's own two heads: the same weights read as a four-head model, split over four chips — every
/// all-reduce a fold of four partials — are that model's step on the VM at every position of the trace.
#[test]
fn four_chips_are_the_step_of_a_four_head_model() {
    let (m, trace) = model_and_trace();
    let mut m4 = m.clone();
    m4.heads = 4;
    let (mut kc, mut vc) = (Vec::new(), Vec::new());
    for (pos, tok) in trace.iter().enumerate() {
        let whole = vm_ints(&format!("{}{}", m4.decode_step_lg(*tok, pos, &kc, &vc), shown(&m4, pos, &[("Kc".into(), "Vc".into())])));
        let (src, manifest) = m4.decode_step_tp_lg(*tok, pos, &kc, &vc, 4).expect("four heads over four chips");
        assert!(manifest.collectives.iter().all(|c| c.partials.len() == 4 && c.folds.len() == 3), "each all-reduce folds four partials in three additions");
        let split = vm_ints(&format!("{src}{}", shown(&m4, pos, &chip_caches(4))));
        assert_eq!(split, whole, "position {pos}: four chips are the four-head step, bit for bit");
        let rows = &whole[1 + m4.vocab + 2 * m4.d..];
        kc.extend_from_slice(&rows[..m4.d]);
        vc.extend_from_slice(&rows[m4.d..]);
    }
}

/// A tensor two sections store is sent after the second: chip 0 writes row 1 then row 2 of `K`, chip 1 reads row 2, and
/// chip 0's halt for `K` follows the last section that stores it, not the first.
#[test]
fn a_tensor_written_in_two_sections_is_sent_after_the_second() {
    let src = "## Main\nLet K be a 2 by 4 tensor of Int.\nLet y be a 1 by 4 tensor of Int.\nSet item 1 of y to 3.\nSet item 3 of y to -5.\n\
               Let z be a 1 by 4 tensor of Int.\nSet item 2 of z to 7.\nSet item 4 of z to 11.\nLet at be 2.\n\
               Let y2 be y plus y.\nSet row 1 of K to y2.\nLet z2 be z plus z.\nSet row 2 of K to z2.\n\
               Let r be row at of K.\nLet s be r plus r.\n";
    let mut manifest = TpManifest::new(2);
    manifest.place("r", 1, None);
    manifest.place("s", 1, None);
    for target in board_targets() {
        let lowered = compile_program_to_chips(src, &target, &manifest).expect("the walk lifts it");
        let got: Vec<(&str, u16, u16)> = lowered.transfers.iter().map(|t| (t.tensor.as_str(), t.from, t.to)).collect();
        assert_eq!(got, [("K", 0, 1)], "{}: K crosses to chip 1", target.name);
        let p = lowered.chips[0].whole().unwrap_or_else(|| panic!("{}: chip 0 lowers whole: {}", target.name, lowered.chips[0].refusal()));
        let (stores, _) = sections_touching(p, "K");
        assert!(stores.len() >= 2, "{}: two sections store K: {stores:?}", target.name);
        let send = p.links.iter().find(|l| names(p, &l.sends).iter().any(|n| n == "K")).expect("chip 0 sends K");
        assert_eq!(send.after, stores.last().copied(), "{}: K is sent after the last section that stores it, {stores:?}", target.name);
    }
}

/// The relay waits at a receipt whose sender sends it later than the next round: chip 0 sends `xi` first and receives
/// `f_a_c1` at its next halt, which chip 1 sends only after a halt of its own in between. A relay that ran a chip past a
/// receipt it has not copied would run chip 0 on before `f_a_c1` exists.
#[test]
fn the_relay_waits_for_a_receipt_sent_two_runs_later() {
    let (m, trace) = model_and_trace();
    let target = board_targets()[0].clone();
    let (mut chips, transfers) = lowered_chips(&m, trace[0], 0, &[], &[], &target);
    let id = |p: &SpatialProgram, name: &str| p.tensors.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("{name}")).id;
    let (xi0, fa0) = (id(&chips[0], "xi"), id(&chips[0], "f_a_c1"));
    let (xi1, fa1) = (id(&chips[1], "xi"), id(&chips[1], "f_a_c1"));
    use logicaffeine_compile::spatial::ir::LinkPoint;
    chips[0].links = vec![
        LinkPoint { after: Some(0), sends: vec![xi0], receives: vec![] },
        LinkPoint { after: Some(1), sends: vec![], receives: vec![fa0] },
    ];
    chips[1].links = vec![
        LinkPoint { after: None, sends: vec![], receives: vec![xi1] },
        LinkPoint { after: Some(0), sends: vec![], receives: vec![] },
        LinkPoint { after: Some(1), sends: vec![], receives: vec![] },
        LinkPoint { after: Some(2), sends: vec![fa1], receives: vec![] },
    ];
    let xfer: Vec<ChipTransfer> = transfers.into_iter().filter(|t| t.tensor == "xi" || t.tensor == "f_a_c1").collect();
    let relay = relay_schedule(&chips, &xfer).expect("the relay completes");
    let at = |pred: &dyn Fn(&RelayStep) -> bool| relay.iter().position(pred).unwrap();
    let sent = at(&|s| matches!(s, RelayStep::Run { chip: 1, to: 3 }));
    let copied = at(&|s| matches!(s, RelayStep::Copy { tensor, .. } if tensor == "f_a_c1"));
    let past = at(&|s| matches!(s, RelayStep::Run { chip: 0, to: 2 }));
    assert!(sent < copied && copied < past, "chip 1 sends f_a_c1 (step {sent}), the host copies it (step {copied}), then chip 0 runs past its receipt (step {past}): {relay:?}");
}
