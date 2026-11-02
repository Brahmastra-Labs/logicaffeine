//! value_range_lock — every integer tensor's proven interval holds every value it takes, and a matmul runs at the
//! narrowest operand those intervals admit.
//!
//! `spatial::value_range` interprets a program's tensor ops over intervals: fills are exact, a gather reads its whole
//! table, a row-written table is closed over its writes, an op it does not model is bounded by nothing. The pipeline
//! hands each matmul its operands' intervals, and the operand precision is the narrowest two's-complement width that
//! holds them. This lock holds:
//!
//! - each op's rule against the op itself, on values drawn from inside the intervals and at their corners;
//! - the decode step's whole run — the prompt and the tokens after it — on the VM: every value every tensor takes at
//!   every step lies in its interval;
//! - the decode step's matmuls run on int8 operands;
//! - a table is closed over its writes, an op the analysis does not model is unbounded, and a float tensor is not
//!   ranged;
//! - the signed fit at int8's edges;
//! - a named row index holds every row its ops admit, so a step's intervals hold for every token and position the host
//!   writes;
//! - the scheduled program's intervals (`program_ranges`) against the evaluator's own run: every page a buffer holds,
//!   every element a tensor holds, every node of every integer body — both arms of a select, as hardware evaluates
//!   them — and every value a stage keeps in its accumulator, on the decode step at every position and on each op's
//!   own program;
//! - storage at the narrowest precision that holds it: on a target that stores narrow integers, every buffer a load
//!   fills from an integer tensor is the narrowest precision holding the tensor's interval, which is what lets the
//!   decode step's attention fit the RTX 3060's shared memory at all; a target that stores only 64-bit integers keeps
//!   them.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test value_range_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use std::collections::BTreeMap;

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography_every_tensor, eval_choreography_traced, Value};
use logicaffeine_compile::spatial::ir::SpatialProgram;
use logicaffeine_compile::spatial::value_range::program_ranges;
use spatial_support::{elem_map, inputs_from_fills, one_generic, one_generic_of, parallel, sum_fold};
use logicaffeine_compile::codegen_sva::systolic::{FusedBlock, Systolic};
use logicaffeine_compile::spatial::eval::Inputs;
use logicaffeine_compile::spatial::ir::{Pred, PredRhs, TensorBody, TensorRole};
use logicaffeine_compile::spatial::passes::{LV_GEN_COL, LV_GEN_K, LV_GEN_ROW};
use logicaffeine_compile::spatial::passes::from_tensor_op::from_fused_block;
use logicaffeine_compile::spatial::pipeline::run_passes;
use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::ir::{ComputeOp, StageKind};
use logicaffeine_compile::spatial::passes::from_tensor_op::choose_matmul_numerics_ranged;
use logicaffeine_compile::spatial::pipeline::{compile_program_to_spatial, value_ranges_of};
use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::spatial::value_range::{Interval, UNBOUNDED};

fn model() -> (Int8Model, Vec<usize>, Vec<usize>) {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let tokens = |k: &str| -> Vec<usize> {
        j["golden_trace"][k].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as usize).collect()
    };
    (Int8Model::from_json(&j).expect("the model"), tokens("prompt_tokens"), tokens("generated_tokens"))
}

/// A deterministic value in `[lo, hi]` for draw `i` of stream `seed`.
fn draw(seed: u64, i: u64, (lo, hi): Interval) -> i64 {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ i.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    let span = (hi as i128 - lo as i128 + 1) as u128;
    (lo as i128 + (x as u128 % span) as i128) as i64
}

fn tensor(name: &str, cols: usize, vals: &[i64]) -> String {
    let mut s = format!("Let {name} be a 1 by {cols} tensor of Int.\n");
    for (i, v) in vals.iter().enumerate() {
        if *v != 0 {
            s.push_str(&format!("Set item {} of {name} to {v}.\n", i + 1));
        }
    }
    s
}

/// The values the VM leaves in each named tensor after `src` runs — a length of zero names a scalar, an arg-max's index.
fn vm_values(src: &str, names: &[(String, usize)]) -> BTreeMap<String, Vec<i64>> {
    let mut prog = src.to_string();
    for (n, len) in names {
        if *len == 0 {
            prog.push_str(&format!("Show {n}.\n"));
        }
        for k in 1..=*len {
            prog.push_str(&format!("Show item {k} of {n}.\n"));
        }
    }
    let out = vm_outcome(&prog);
    assert!(out.error.is_none(), "VM error {:?}", out.error);
    let mut vals = out.output.split_whitespace().map(|v| v.parse::<i64>().unwrap_or_else(|_| panic!("not an integer: {v}")));
    names.iter().map(|(n, len)| (n.clone(), (0..(*len).max(1)).map(|_| vals.next().expect("a shown value")).collect())).collect()
}

fn ranges(src: &str) -> BTreeMap<String, Interval> {
    value_ranges_of(src).unwrap_or_else(|e| panic!("the program ranges: {e:?}\n{src}"))
}

/// One op's rule against the op: for intervals around the operands' and values drawn from them — corners included —
/// the VM's result lies in the interval the analysis gives.
fn rule_holds(label: &str, ins: &[(&str, Interval)], body: &str, out: &str, n: usize, n_out: usize) {
    for seed in 0..24u64 {
        let mut src = String::from("## Main\n");
        for (k, (name, iv)) in ins.iter().enumerate() {
            let vals: Vec<i64> = (0..n as u64)
                .map(|i| match i {
                    0 => iv.0,
                    1 => iv.1,
                    _ => draw(seed * 31 + k as u64, i, *iv),
                })
                .collect();
            src.push_str(&tensor(name, n, &vals));
        }
        src.push_str(body);
        let got = vm_values(&src, &[(out.to_string(), n_out)]);
        let iv = ranges(&src)[out];
        for v in &got[out] {
            assert!(iv.0 <= *v && *v <= iv.1, "{label} (seed {seed}): {v} lies outside the proven {iv:?}\n{src}");
        }
    }
}

#[test]
fn each_integer_rule_holds_on_the_op_itself() {
    rule_holds("requant", &[("X", (-258_064, 258_064))], "Let Y be X times 104593 shifted right by 24.\n", "Y", 8, 8);
    rule_holds("requant by a negative scale", &[("X", (-5000, 7000))], "Let Y be X times -3 shifted right by 2.\n", "Y", 8, 8);
    rule_holds("clamp", &[("X", (-400, 400))], "Let Y be X clamped between -128 and 127.\n", "Y", 8, 8);
    rule_holds("relu", &[("X", (-40, 40))], "Let Y be relu of X.\n", "Y", 8, 8);
    rule_holds("add", &[("A", (-100, 50)), ("B", (-7, 90))], "Let C be A plus B.\n", "C", 8, 8);
    rule_holds("argmax", &[("X", (-40, 40))], "Let C be argmax of X.\n", "C", 8, 0);
}

#[test]
fn a_matmul_s_interval_is_k_extreme_products() {
    let src = "## Main\nLet A be a 1 by 3 tensor of Int.\nSet item 1 of A to -128.\nSet item 2 of A to 127.\n\
               Let W be a 3 by 2 tensor of Int.\nSet item 1 of W to 100.\nSet item 2 of W to -5.\nLet C be A matmul W.\n";
    let r = ranges(src);
    assert_eq!(r["A"], (-128, 127));
    assert_eq!(r["W"], (-5, 100));
    assert_eq!(r["C"], (3 * -12_800, 3 * 12_700), "k times the least and the greatest product of the operands' ends");
}

#[test]
fn a_gather_reads_its_whole_table_whichever_row_the_index_names() {
    let src = "## Main\nLet T be a 3 by 2 tensor of Int.\nSet item 1 of T to -9.\nSet item 6 of T to 40.\nLet at be 2.\nLet r be row at of T.\n";
    assert_eq!(ranges(src)["r"], (-9, 40), "row 2 holds only zeros, but the step serves every row the host names");
}

#[test]
fn a_row_written_table_is_closed_over_its_writes() {
    let src = "## Main\nLet K be a 4 by 2 tensor of Int.\nLet x be a 1 by 2 tensor of Int.\nSet item 1 of x to 300.\nSet item 2 of x to -7.\n\
               Let y be x clamped between -128 and 127.\nLet at be 1.\nSet row at of K to y.\nLet r be row at of K.\n";
    let r = ranges(src);
    assert_eq!(r["K"], (-7, 127), "the table's zeros joined with the row written into it");
    assert_eq!(r["r"], (-7, 127), "a read after the write sees what the table can hold");
}

#[test]
fn a_transpose_and_a_block_gather_carry_their_operand_s_interval() {
    let r = ranges("## Main\nLet T be a 4 by 2 tensor of Int.\nSet item 1 of T to 3.\nLet U be transpose of T.\nLet V be U plus U.\n");
    assert_eq!((r["U"], r["V"]), ((0, 3), (0, 6)));
    let r = ranges("## Main\nLet E be a 3 by 2 tensor of Int.\nSet item 4 of E to -6.\nLet I be a 1 by 2 tensor of Int.\nSet item 1 of I to 1.\nSet item 2 of I to 3.\nLet X be rows I of E.\n");
    assert_eq!(r["X"], (-6, 0), "a block gather reads its whole table");
}

/// The context of a LUT attention lies among the values while the position with the largest score — which weighs the
/// table's first entry — weighs at least one and no weight is negative. A first entry of zero leaves a divisor that can
/// be zero, and the context is bounded by nothing.
#[test]
fn a_context_is_bounded_while_its_largest_score_weighs_at_least_one() {
    let attention = |e1: i64, e2: i64| {
        format!(
            "## Main\n{}{}{}{}Let C be lut attention of Q over K and V with 1 heads 1 groups with table E and scale 1 shifted right by 0.\n",
            tensor("Q", 2, &[1, 2]),
            "Let K be a 2 by 2 tensor of Int.\nSet item 1 of K to 3.\n",
            "Let V be a 2 by 2 tensor of Int.\nSet item 1 of V to -9.\nSet item 4 of V to 11.\n",
            format!("Let E be a 1 by 2 tensor of Int.\nSet item 1 of E to {e1}.\nSet item 2 of E to {e2}.\n"),
        )
    };
    assert_eq!(ranges(&attention(7, 5))["C"], (-9, 11), "a positive table: the context lies among the values");
    assert_eq!(ranges(&attention(7, 0))["C"], (-9, 11), "a far position may weigh zero: the nearest still weighs seven");
    assert_eq!(ranges(&attention(0, 5))["C"], UNBOUNDED, "a first entry of zero proves nothing");
    assert_eq!(ranges(&attention(7, -1))["C"], UNBOUNDED, "a negative weight proves nothing");
}

#[test]
fn int8_s_edges_fit_int8_and_one_past_them_does_not() {
    let t = SpatialTarget::sky_fabric(16, 64);
    let op = |a: Interval, b: Interval| choose_matmul_numerics_ranged(&t, Some(a), Some(b), 16).unwrap().0;
    assert_eq!(op((-128, 127), (-128, 127)), Precision::Int8, "an int8 activation and an int8 weight");
    assert_eq!(op((-129, 127), (-128, 127)), Precision::Int16, "one below int8");
    assert_eq!(op((-128, 128), (-128, 127)), Precision::Int16, "one above int8");
    assert_eq!(op((0, 0), (-32_768, 32_767)), Precision::Int16, "int16's edges");
    assert_eq!(op((0, 0), (-32_769, 0)), Precision::Int64, "one below int16: the next width the fabric's array takes");
}

#[test]
fn the_decode_steps_matmuls_run_on_int8_operands() {
    let (m, prompt, _) = model();
    for target in [SpatialTarget::sky_fabric(16, 64), SpatialTarget::xc7a200t_fabric(16, 64)] {
        let p = compile_program_to_spatial(&m.decode_step_lg(prompt[0], 0, &[], &[]), &target)
            .unwrap()
            .program
            .expect("the step lowers whole");
        let operands: Vec<Precision> = p
            .stages
            .iter()
            .filter_map(|s| match &s.kind {
                StageKind::Compute(ComputeOp::MatmulTiles { operand, .. }) => Some(*operand),
                _ => None,
            })
            .collect();
        assert_eq!(operands.len(), 7, "{}: the step's seven matmuls", target.name);
        assert!(operands.iter().all(|o| *o == Precision::Int8), "{}: every matmul on int8 operands: {operands:?}", target.name);
    }
}

/// The whole run: every tensor the step names, at every step of the prompt and the tokens after it, on the VM — each
/// value inside the interval the analysis proves for the program every step runs.
#[test]
fn every_value_the_decode_run_takes_lies_in_its_proven_interval() {
    let (m, prompt, golden) = model();
    let first = m.decode_step_lg(prompt[0], 0, &[], &[]);
    let proven = ranges(&first);
    let (mut kc, mut vc) = (Vec::new(), Vec::new());
    let mut tok = prompt[0];
    let mut checked = 0usize;
    for pos in 0..prompt.len() + golden.len() - 1 {
        let src = m.decode_step_lg(tok, pos, &kc, &vc);
        // Every position's step is the same program, so every interval it derives is the first step's; only the two
        // values the host writes each step — the token and the position — are declared differently.
        let here = ranges(&src);
        let derived = |r: &BTreeMap<String, Interval>| -> BTreeMap<String, Interval> {
            r.iter().filter(|(n, _)| *n != "tok" && *n != "pos").map(|(n, v)| (n.clone(), *v)).collect()
        };
        assert_eq!(derived(&here), derived(&proven), "position {pos}: the step's intervals are the first step's");
        let names: Vec<(String, usize)> = [
            ("xe", m.d), ("x", m.d), ("xi", m.d), ("q", m.d), ("kt", m.d), ("vt", m.d), ("c", m.d), ("ci", m.d),
            ("o", m.d), ("s1", m.d), ("hi", m.d), ("hid", m.d_ff), ("fi", m.d_ff), ("f", m.d), ("s2", m.d),
            ("oi", m.d), ("logits", m.vocab), ("next", 0), ("Kc", m.ctx * m.d), ("Vc", m.ctx * m.d),
        ]
        .iter()
        .map(|(n, l)| (n.to_string(), *l))
        .collect();
        let vals = vm_values(&src, &names);
        for (n, vs) in &vals {
            let iv = proven.get(n).unwrap_or_else(|| panic!("`{n}` has a proven interval"));
            for v in vs {
                assert!(iv.0 <= *v && *v <= iv.1, "position {pos}: `{n}` takes {v}, outside its proven {iv:?}");
                checked += 1;
            }
        }
        kc = vals["Kc"][..(pos + 1) * m.d].to_vec();
        vc = vals["Vc"][..(pos + 1) * m.d].to_vec();
        let next = vals["next"][0] as usize - 1;
        tok = if pos + 1 < prompt.len() { prompt[pos + 1] } else { next };
    }
    assert!(checked > 10_000, "the run checked {checked} values");
}

/// A named row index is data the host may write at every step: its interval is every row its ops admit — the rows of
/// the table a gather or a row write names, the positions of the cache an attention reads — not the one value its
/// declaration starts it at. An index two ops name holds only the rows both admit; a declared value outside them still
/// lies in the interval, since the program holds it until the op refuses it.
#[test]
fn an_index_holds_every_row_its_ops_admit() {
    let r = ranges("## Main\nLet T be a 5 by 2 tensor of Int.\nLet at be 3.\nLet x be row at of T.\n");
    assert_eq!(r["at"], (1, 5), "a gather's index: every row of its table");
    let r = ranges("## Main\nLet K be a 6 by 2 tensor of Int.\nLet x be a 1 by 2 tensor of Int.\nLet at be 2.\nSet row at of K to x.\n");
    assert_eq!(r["at"], (1, 6), "a row write's index: every row of its table");
    let r = ranges(
        "## Main\nLet T be a 5 by 2 tensor of Int.\nLet K be a 3 by 2 tensor of Int.\nLet x be a 1 by 2 tensor of Int.\n\
         Let at be 2.\nSet row at of K to x.\nLet y be row at of T.\n",
    );
    assert_eq!(r["at"], (1, 3), "an index two ops name: the rows both admit");
    let r = ranges("## Main\nLet T be a 5 by 2 tensor of Int.\nLet at be 9.\nLet x be row at of T.\n");
    assert_eq!(r["at"], (1, 9), "a declared row past the table: the program holds it until the gather refuses it");
    let r = ranges(
        "## Main\nLet E be a 3 by 2 tensor of Int.\nLet I be a 1 by 2 tensor of Int.\nSet item 1 of I to 1.\nSet item 2 of I to 2.\nLet X be rows I of E.\n",
    );
    assert_eq!(r["I"], (1, 3), "a block gather's indices: every row of its table");
    let attention = format!(
        "## Main\n{}Let K be a 4 by 2 tensor of Int.\nLet V be a 4 by 2 tensor of Int.\n{}Let at be 2.\n\
         Let C be lut attention of Q over K and V with 1 heads 1 groups up to at with table E and scale 1 shifted right by 0.\n",
        tensor("Q", 2, &[1, 2]),
        tensor("E", 2, &[7, 5]),
    );
    assert_eq!(ranges(&attention)["at"], (1, 4), "an attention's bound: every position of its cache");
    let (m, prompt, _) = model();
    let r = ranges(&m.decode_step_lg(prompt[0], 0, &[], &[]));
    assert_eq!(r["tok"], (1, m.vocab as i64), "the decode step's token: every row of the embedding");
    assert_eq!(r["pos"], (1, m.ctx as i64), "the decode step's position: every row of the cache");
}

fn within(iv: Interval, held: Interval) -> bool {
    iv.0 <= held.0 && held.1 <= iv.1
}

/// The evaluator's run of `p` from its fills, against `program_ranges(p)`: every buffer's pages, every tensor's
/// elements, every node of every integer body and every value a stage keeps lie in their proven intervals. The number
/// of observations checked.
fn scheduled_run_holds(label: &str, p: &SpatialProgram) -> usize {
    scheduled_run_holds_on(label, p, &inputs_from_fills(p))
}

/// [`scheduled_run_holds`] from `inputs`.
fn scheduled_run_holds_on(label: &str, p: &SpatialProgram, inputs: &Inputs) -> usize {
    let proven = program_ranges(p);
    let (_, trace) = eval_choreography_traced(p, inputs).unwrap_or_else(|e| panic!("{label}: the run: {e:?}"));
    let mut checked = 0usize;
    for (b, held) in &trace.held {
        let iv = proven.buffers[b.idx()].unwrap_or_else(|| panic!("{label}: buffer {} holds {held:?} and has no interval", p.buffer(*b).name));
        assert!(within(iv, *held), "{label}: buffer {} holds {held:?}, outside its proven {iv:?}", p.buffer(*b).name);
        checked += 1;
    }
    for (s, nodes) in &trace.nodes {
        let proven_nodes = &proven.stages[s.idx()].nodes;
        assert_eq!(nodes.len(), proven_nodes.len(), "{label}: stage {}: one interval per body node", p.stage(*s).name);
        for (j, (held, iv)) in nodes.iter().zip(proven_nodes).enumerate() {
            if let Some(held) = held {
                assert!(within(*iv, *held), "{label}: stage {} node {j} computes {held:?}, outside its proven {iv:?}", p.stage(*s).name);
                checked += 1;
            }
        }
    }
    for (s, held) in &trace.stage_held {
        let iv = proven.stages[s.idx()].held.unwrap_or_else(|| panic!("{label}: stage {} keeps {held:?} and has no interval", p.stage(*s).name));
        assert!(within(iv, *held), "{label}: stage {} keeps {held:?}, outside its proven {iv:?}", p.stage(*s).name);
        checked += 1;
    }
    let every = eval_choreography_every_tensor(p, inputs).unwrap_or_else(|e| panic!("{label}: the run: {e:?}"));
    for (t, v) in &every.tensors {
        let Value::Int(vs) = v else { continue };
        let iv = proven.tensors[t.idx()].unwrap_or_else(|| panic!("{label}: tensor {} holds values and has no interval", p.tensor(*t).name));
        for x in vs {
            assert!(iv.0 <= *x && *x <= iv.1, "{label}: tensor {} holds {x}, outside its proven {iv:?}", p.tensor(*t).name);
            checked += 1;
        }
    }
    assert!(!trace.held.is_empty() && !trace.stage_held.is_empty(), "{label}: the run observed its buffers and stages");
    checked
}

fn scheduled(src: &str, target: &SpatialTarget) -> SpatialProgram {
    let l = compile_program_to_spatial(src, target).unwrap_or_else(|e| panic!("a tensor program: {e:?}\n{src}"));
    l.program.unwrap_or_else(|| panic!("the program lowers whole: {:?}\n{src}", l.missing.first()))
}

/// Each op's own program, scheduled, on operands drawn across their intervals and at their corners — on the sky130
/// fabric, whose tiles are one element, and on the RTX 3060, whose 16 × 16 tiles pad every one of these tensors: the
/// padding zeros the folds read and the planes write are values the intervals hold too.
#[test]
fn each_op_s_scheduled_run_lies_in_its_proven_intervals() {
    for target in [SpatialTarget::sky_fabric(16, 64), SpatialTarget::rtx3060()] {
        op_programs_hold(&target);
    }
}

fn op_programs_hold(target: &SpatialTarget) {
    let target = target.clone();
    let programs: Vec<(&str, Vec<(&str, Interval)>, &str)> = vec![
        ("requant", vec![("X", (-258_064, 258_064))], "Let Y be X times 104593 shifted right by 24.\n"),
        ("requant by a negative scale", vec![("X", (-5000, 7000))], "Let Y be X times -3 shifted right by 2.\n"),
        ("clamp", vec![("X", (-400, 400))], "Let Y be X clamped between -128 and 127.\n"),
        ("relu", vec![("X", (-40, 40))], "Let Y be relu of X.\n"),
        ("add", vec![("A", (-100, 50)), ("B", (-7, 90))], "Let C be A plus B.\n"),
        ("argmax", vec![("X", (-40, 40))], "Let C be argmax of X.\n"),
    ];
    let mut checked = 0usize;
    for (label, ins, body) in &programs {
        for seed in 0..6u64 {
            let mut src = String::from("## Main\n");
            for (k, (name, iv)) in ins.iter().enumerate() {
                let vals: Vec<i64> = (0..8u64)
                    .map(|i| match i {
                        0 => iv.0,
                        1 => iv.1,
                        _ => draw(seed * 31 + k as u64, i, *iv),
                    })
                    .collect();
                src.push_str(&tensor(name, 8, &vals));
            }
            src.push_str(body);
            checked += scheduled_run_holds(&format!("{label} (seed {seed}) on {}", target.name), &scheduled(&src, &target));
        }
    }
    let matmul = "## Main\nLet A be a 1 by 3 tensor of Int.\nSet item 1 of A to -128.\nSet item 2 of A to 127.\n\
                  Let W be a 3 by 2 tensor of Int.\nSet item 1 of W to 100.\nSet item 2 of W to -5.\nLet C be A matmul W.\n";
    checked += scheduled_run_holds(&format!("matmul on {}", target.name), &scheduled(matmul, &target));
    let gather = "## Main\nLet T be a 3 by 2 tensor of Int.\nSet item 1 of T to -9.\nSet item 6 of T to 40.\nLet at be 3.\nLet r be row at of T.\n";
    checked += scheduled_run_holds(&format!("gather on {}", target.name), &scheduled(gather, &target));
    let write = "## Main\nLet K be a 4 by 2 tensor of Int.\nLet x be a 1 by 2 tensor of Int.\nSet item 1 of x to 300.\nSet item 2 of x to -7.\n\
                 Let y be x clamped between -128 and 127.\nLet at be 2.\nSet row at of K to y.\nLet r be row at of K.\n";
    checked += scheduled_run_holds(&format!("row write on {}", target.name), &scheduled(write, &target));
    for (e1, e2) in [(7, 5), (7, 0), (32768, 1)] {
        let attention = format!(
            "## Main\n{}Let K be a 3 by 2 tensor of Int.\nSet item 1 of K to 3.\nSet item 4 of K to -100.\n\
             Let V be a 3 by 2 tensor of Int.\nSet item 1 of V to -9.\nSet item 4 of V to 11.\nSet item 6 of V to -128.\n{}Let at be 2.\n\
             Let C be lut attention of Q over K and V with 1 heads 1 groups up to at with table E and scale 21076 shifted right by 6.\n",
            tensor("Q", 2, &[100, -2]),
            tensor("E", 2, &[e1, e2]),
        );
        checked += scheduled_run_holds(&format!("lut attention [{e1}, {e2}] on {}", target.name), &scheduled(&attention, &target));
    }
    assert!(checked > 500, "{}: the op programs checked {checked} observations", target.name);
}

/// Programs whose runs reach the ends of their stages' intervals, where a rule's bound one step too narrow is caught: a
/// contraction whose terms all take their greatest product, so its sum is twice any one term; a requant of an operand
/// that excludes zero, on a target whose tiles pad it with zeros the body evaluates; and a fused block — its matmul,
/// requant, bias and ReLU stages — whose columns of weights are each one value, +100 or −100, and whose activations sit
/// at int8's ends, so every one of those stages reaches both ends of its interval. Two hand-built `Generic`s reach what no
/// front-end program does yet: a contraction over `k`, each of its terms the greatest product, and a select whose
/// unselected arm reads past its page — zero, as the hardware reads it — from a tensor that holds no zero.
#[test]
fn each_stage_rule_holds_where_its_runs_reach_its_ends() {
    let mut checked = 0usize;
    let sky = SpatialTarget::sky_fabric(16, 64);
    let contraction = one_generic(
        &sky,
        &[("a", 4, vec![100; 4]), ("b", 8, vec![3; 8])],
        2,
        vec![parallel(LV_GEN_ROW, 1), parallel(LV_GEN_COL, 2), sum_fold(LV_GEN_K, 4)],
        vec![elem_map(0, &[(LV_GEN_K, 1)]), elem_map(0, &[(LV_GEN_K, 2), (LV_GEN_COL, 1)])],
        elem_map(0, &[(LV_GEN_COL, 1)]),
        TensorBody::Mul(Box::new(TensorBody::Operand(0)), Box::new(TensorBody::Operand(1))),
    );
    checked += scheduled_run_holds("a contraction of four greatest products", &contraction);
    let select = one_generic(
        &sky,
        &[("a", 4, vec![-3, 4, -5, 6]), ("b", 4, vec![5, 9, 7, 8])],
        4,
        vec![parallel(LV_GEN_ROW, 1), parallel(LV_GEN_COL, 4)],
        vec![elem_map(0, &[(LV_GEN_COL, 1)]), elem_map(2, &[(LV_GEN_COL, 1)])],
        elem_map(0, &[(LV_GEN_COL, 1)]),
        TensorBody::Select {
            cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Const(2)),
            then: Box::new(TensorBody::Operand(1)),
            otherwise: Box::new(TensorBody::Operand(0)),
        },
    );
    checked += scheduled_run_holds("a select whose unselected arm reads past its page", &select);
    for target in [SpatialTarget::sky_fabric(16, 64), SpatialTarget::rtx3060()] {
        let attention = format!(
            "## Main\n{}Let K be a 3 by 2 tensor of Int.\n{}Let V be a 3 by 2 tensor of Int.\nSet item 1 of V to -9.\nSet item 4 of V to 11.\n\
             {}Let at be 2.\nLet C be lut attention of Q over K and V with 1 heads 1 groups up to at with table E and scale 21076 shifted right by 6.\n",
            tensor("Q", 2, &[100, 100]),
            (1..=6).map(|i| format!("Set item {i} of K to 3.\n")).collect::<String>(),
            tensor("E", 2, &[7, 5]),
        );
        checked += scheduled_run_holds(&format!("aligned attention on {}", target.name), &scheduled(&attention, &target));
        let requant = format!("## Main\n{}Let Y be X times 104593 shifted right by 24.\n", tensor("X", 8, &[5000, 7000, 5001, 6999, 6000, 5500, 6500, 5000]));
        checked += scheduled_run_holds(&format!("requant of a positive operand on {}", target.name), &scheduled(&requant, &target));
    }
    let target = SpatialTarget::sky_fabric(16, 64);
    let (k, n, m) = (4usize, 4usize, 6usize);
    let weights: Vec<i64> = (0..k * n).map(|i| if (i % n) % 2 == 0 { 100 } else { -100 }).collect();
    let fb = FusedBlock { sys: Systolic::new(k, n, weights), requant_m: 181, requant_s: 8, bias: vec![-400, 0, 7, 400] };
    let p = run_passes(from_fused_block(&fb, m, &target).expect("the fused block"), &target).expect("the fused block schedules");
    let acts: Vec<i64> = (0..m * k).map(|i| if (i / k) % 2 == 0 { -128 } else { 127 }).collect();
    let mut inputs = Inputs::default();
    for t in &p.tensors {
        let len = t.elems.rows * t.elems.cols;
        match t.role {
            TensorRole::Output => {}
            TensorRole::Input => {
                inputs.tensors.insert(t.id, Value::Int(acts.clone()));
            }
            _ => {
                inputs.tensors.insert(t.id, Value::Int((0..len).map(|i| t.init.element(i).expect("a parameter's values")).collect()));
            }
        }
    }
    checked += scheduled_run_holds_on("fused block at int8's ends", &p, &inputs);
    let proven = program_ranges(&p);
    let (_, trace) = eval_choreography_traced(&p, &inputs).unwrap();
    let kinds: [(&str, fn(&ComputeOp) -> bool); 4] = [
        ("matmul", |op| matches!(op, ComputeOp::MatmulTiles { .. })),
        ("requant", |op| matches!(op, ComputeOp::Requant { .. })),
        ("bias", |op| matches!(op, ComputeOp::Bias { .. })),
        ("relu", |op| matches!(op, ComputeOp::Relu)),
    ];
    for (what, is) in kinds {
        let stage = p.stages.iter().find(|s| matches!(&s.kind, StageKind::Compute(op) if is(op))).unwrap_or_else(|| panic!("the fused block has its {what} stage"));
        assert_eq!(trace.stage_held[&stage.id], proven.stages[stage.id.idx()].held.unwrap(), "the {what} stage's run reaches both ends of its interval");
    }
    assert!(checked > 300, "the stage programs checked {checked} observations");
}

/// A tensor declared at an interval its precision cannot hold loads as its precision stores it: an int8 tensor declared
/// `[100, 200]` reads its 200 as −56, so the load holds every int8 value, not the declared interval.
#[test]
fn a_load_wraps_a_tensor_whose_interval_passes_its_precision() {
    let sky = SpatialTarget::sky_fabric(16, 64);
    let p = one_generic_of(
        &sky,
        Precision::Int8,
        &[("a", 4, vec![100, 150, 200, 127])],
        4,
        vec![parallel(LV_GEN_ROW, 1), parallel(LV_GEN_COL, 4)],
        vec![elem_map(0, &[(LV_GEN_COL, 1)])],
        elem_map(0, &[(LV_GEN_COL, 1)]),
        TensorBody::Operand(0),
    );
    assert_eq!(p.tensors[0].range, Some((100, 200)));
    let checked = scheduled_run_holds("an int8 tensor declared past int8", &p);
    assert!(checked > 4, "the wrapped load checked {checked} observations");
    let load = p.stages.iter().find(|s| matches!(s.kind, StageKind::Load { .. })).expect("a load");
    assert_eq!(program_ranges(&p).buffers[load.outs[0].idx()], Some((-128, 127)), "the load holds every int8 value");
}

/// A table the host writes between steps without a declared range holds whatever the host wrote: it starts at every
/// value its precision holds. The row-written cache of a small program, its declared range taken away, run with
/// contents no store of the program writes.
#[test]
fn a_table_the_host_writes_starts_at_every_value_its_precision_holds() {
    let src = "## Main\nLet K be a 4 by 2 tensor of Int.\nLet x be a 1 by 2 tensor of Int.\nSet item 1 of x to 300.\nSet item 2 of x to -7.\n\
               Let y be x clamped between -128 and 127.\nLet at be 2.\nSet row at of K to y.\nLet r be row at of K.\n";
    let mut p = scheduled(src, &SpatialTarget::sky_fabric(16, 64));
    let state: Vec<usize> = p.tensors.iter().filter(|t| t.role == TensorRole::State).map(|t| t.id.idx()).collect();
    assert!(!state.is_empty(), "the cache is a table the host carries");
    let mut inputs = inputs_from_fills(&p);
    for &i in &state {
        p.tensors[i].range = None;
        let len = p.tensors[i].elems.rows * p.tensors[i].elems.cols;
        inputs.tensors.insert(p.tensors[i].id, Value::Int((0..len as i64).map(|j| if j % 2 == 0 { 1 << 40 } else { -(1 << 41) }).collect()));
    }
    let checked = scheduled_run_holds_on("a host-written cache with no declared range", &p, &inputs);
    assert!(checked > 10, "the cache's run checked {checked} observations");
}

/// The decode step at every position of the golden trace — the cache carried from step to step — on the sky130 fabric,
/// whose memories answer in the cycle of the address, and on the Nexys part, whose capped array blocks every matmul.
#[test]
fn every_value_a_scheduled_decode_run_holds_lies_in_its_proven_interval() {
    let (m, prompt, golden) = model();
    let mut checked = 0usize;
    for target in [SpatialTarget::sky_fabric(16, 64), SpatialTarget::xc7a200t_fabric(16, 64)] {
        let (mut kc, mut vc) = (Vec::new(), Vec::new());
        let mut tok = prompt[0];
        for pos in 0..prompt.len() + golden.len() - 1 {
            let p = scheduled(&m.decode_step_lg(tok, pos, &kc, &vc), &target);
            checked += scheduled_run_holds(&format!("{} at position {pos}", target.name), &p);
            let every = eval_choreography_every_tensor(&p, &inputs_from_fills(&p)).unwrap();
            let read = |name: &str| -> Vec<i64> {
                match &every.tensors[&p.tensors.iter().find(|t| t.name == name).unwrap().id] {
                    Value::Int(v) => v.clone(),
                    Value::Float(v) => panic!("{name} through floats: {v:?}"),
                }
            };
            kc = read("Kc")[..(pos + 1) * m.d].to_vec();
            vc = read("Vc")[..(pos + 1) * m.d].to_vec();
            let next = read("next")[0] as usize - 1;
            tok = if pos + 1 < prompt.len() { prompt[pos + 1] } else { next };
        }
    }
    assert!(checked > 50_000, "the decode run checked {checked} observations");
}

/// The intervals are worth having: on the decode step every buffer and every tensor the program computes is bounded
/// within 32 bits, and every node of every body within 48 — none falls back to the whole of i64.
#[test]
fn the_decode_step_s_storage_fits_32_bits_and_its_arithmetic_48() {
    let (m, prompt, _) = model();
    let fits = |iv: Interval, bits: u32| iv.0 >= -(1i64 << (bits - 1)) && iv.1 < (1i64 << (bits - 1));
    for target in [SpatialTarget::sky_fabric(16, 64), SpatialTarget::xc7a200t_fabric(16, 64)] {
        let p = scheduled(&m.decode_step_lg(prompt[0], 0, &[], &[]), &target);
        let proven = program_ranges(&p);
        for b in &p.buffers {
            let iv = proven.buffers[b.id.idx()].unwrap_or_else(|| panic!("{}: buffer {} has an interval", target.name, b.name));
            assert!(fits(iv, 32), "{}: buffer {} holds {iv:?}", target.name, b.name);
        }
        for t in &p.tensors {
            let iv = proven.tensors[t.id.idx()].unwrap_or_else(|| panic!("{}: tensor {} has an interval", target.name, t.name));
            assert!(fits(iv, 32), "{}: tensor {} holds {iv:?}", target.name, t.name);
        }
        for (s, st) in p.stages.iter().zip(&proven.stages) {
            for (j, iv) in st.nodes.iter().enumerate() {
                assert!(fits(*iv, 48), "{}: stage {} node {j} computes {iv:?}", target.name, s.name);
            }
            if let Some(iv) = st.held {
                assert!(fits(iv, 48), "{}: stage {} keeps {iv:?}", target.name, s.name);
            }
        }
    }
}

/// The narrowest signed precision of `ps` that holds `iv`.
fn narrowest(ps: &[Precision], iv: Interval) -> Precision {
    *ps.iter()
        .find(|p| {
            let bits = logicaffeine_compile::spatial::target::precision_int_bits(**p).unwrap();
            bits >= 64 || (iv.0 >= -(1i64 << (bits - 1)) && iv.1 < (1i64 << (bits - 1)))
        })
        .expect("Int64 holds every interval")
}

/// Each target stores a loaded buffer at the narrowest of its own integer precisions that holds the tensor's interval —
/// and the padding zeros, where the tensor ends inside a tile. On the RTX 3060 that is what fits the step's attention
/// into a block's shared memory; on a fabric it changes no word, which the fabric sizes by width.
#[test]
fn a_loaded_buffer_is_stored_at_the_narrowest_precision_its_tensor_needs() {
    use logicaffeine_compile::spatial::ir::StageKind;
    let (m, prompt, _) = model();
    let src = m.decode_step_lg(prompt[0], 0, &[], &[]);
    let gpu = SpatialTarget::rtx3060();
    let ladder = [Precision::Int8, Precision::Int16, Precision::Int32, Precision::Int64];
    assert_eq!(gpu.storable_ints(), ladder.to_vec(), "the RTX 3060 stores every integer width, int32 its own word");
    for target in [gpu, SpatialTarget::sky_fabric(16, 64), SpatialTarget::xc7a200t_fabric(16, 64)] {
        let l = compile_program_to_spatial(&src, &target).expect("a tensor program");
        let p = l.program.unwrap_or_else(|| panic!("the step lowers whole on {}: {:?}", target.name, l.missing.first()));
        let storable = target.storable_ints();
        let mut narrowed = 0usize;
        for st in &p.stages {
            let StageKind::Load { tensor, .. } = &st.kind else { continue };
            let t = p.tensor(*tensor);
            let Some(range) = t.range else { continue };
            if logicaffeine_compile::spatial::target::precision_int_bits(t.dtype).is_none() {
                continue;
            }
            for b in &st.outs {
                let buf = p.buffer(*b);
                // A matmul's operand buffer holds its operand precision, which the numerics chose already.
                let operand = p.stages.iter().any(|s| s.ins.contains(b) && matches!(s.kind, StageKind::Compute(ComputeOp::MatmulTiles { .. })));
                if operand {
                    continue;
                }
                let padded = t.tiles.rows_t * p.tile.rows > t.elems.rows || t.tiles.cols_t * p.tile.cols > t.elems.cols;
                let held = if padded { (range.0.min(0), range.1.max(0)) } else { range };
                let want = narrowest(&storable, held);
                assert_eq!(buf.dtype, want, "{}: buffer {} loads {} ({held:?})", target.name, buf.name, t.name);
                narrowed += (buf.dtype != Precision::Int64) as usize;
            }
        }
        assert!(narrowed > 10, "{}: the step narrows its loaded buffers ({narrowed})", target.name);
    }
}
