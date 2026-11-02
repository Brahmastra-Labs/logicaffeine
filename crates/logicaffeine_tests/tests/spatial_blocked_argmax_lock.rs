//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! AN ARG-MAX OVER A ROW WIDER THAN A SITE: BLOCKS, THEN A TREE — AND STILL THE FIRST MAXIMUM.
//!
//! A decode step ends in `next = argmax of logits`, and Qwen3.8's logits are 248,320 wide: 7,760 tiles against
//! a site that holds eight. So the row is reduced in BLOCKS — each block's maximum and its position in the whole
//! row — and the blocks' results merge in a tree, each level keeping the first maximum of `fan` partials. The first
//! maximum survives because every block and every partial stays in position order and every fold takes the first
//! of equals. A block past the row is masked whole and a tail block's padding is masked by the block's live
//! limit: padding reads as 0, which would beat a row of negative logits.
//!
//! Pinned: the real vocabulary lowers and certifies, the real decode head (norm, LM head, arg-max) lowers whole,
//! and single-site and blocked the result is the VM's exactly — the maximum at the start, at the end, at a block
//! boundary, tied across blocks (the first wins), in a row of negatives with a partial tail block, below every
//! finite mask, and with NaNs: the VM's scan starts from item 1 and replaces only on `>`, so a NaN is the answer
//! exactly when it is item 1.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_blocked_argmax_lock`
//!
//! ★ MUTATION PROBES (run when this landed, in `from_qwen_ops::blocked_argmax` unless the evaluator is named):
//! - the float mask `−∞` → the finite `−1e30` flash attention masks scores with: a row below `−1e30` reports
//!   5001 for 5000 — the first PADDING position, past the row (`a_row_below_any_finite_mask…`).
//! - the live-limit `Select` dropped: padding folds as 0 and beats every negative row (three tests).
//! - the position base unclamped (`b · block` for the tail block that starts early): every maximum in the
//!   overlapped tail reports the wrong position (three tests).
//! - the block table reversed (block `b` reads block `nb−1−b`): positions stay absolute, so only rows with
//!   EQUAL maxima see it — the tie test, and the negative and integer rows whose values repeat — as the first
//!   of equals becomes the last.
//! - the integer mask 0 instead of the type's minimum: a row of large negatives loses to its padding.
//! - the evaluator's (`eval.rs`) ArgMax fold unseeded (an accumulator starting at `−∞`, as it did): a NaN first is skipped —
//!   200 for the VM's 1 at width 200.
//! - every block seeded by its own first element (the first-block flag ignored): a NaN at block 1's start captures
//!   the block — 29 for the VM's 301.
//! - a block that holds its seed reporting the mask instead: block 0's first item can no longer win — 273 for 1.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_cuda::qwen38::{gen_qwen38_decode_head_lg, Qwen38Cfg};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::ir::SpatialProgram;
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;

#[path = "common/spatial.rs"]
mod spatial_support;
use spatial_support::inputs_from_fills;

fn composed(src: &str) -> SpatialProgram {
    let l = compile_program_to_spatial(src, &RduChip::Sn50.facts().spatial_target())
        .expect("a tensor program");
    l.program.unwrap_or_else(|| {
        panic!(
            "the arg-max lowers whole and certifies: {:?}",
            l.missing.first()
        )
    })
}

/// An arg-max over `values`, and the index the VM computes for it.
fn argmax_program(values: &[f64]) -> String {
    let mut s = format!(
        "## Main\nLet L be a 1 by {} tensor of Real.\n",
        values.len()
    );
    for (i, v) in values.iter().enumerate() {
        if *v != 0.0 {
            s.push_str(&format!("Set item {} of L to {v:?}.\n", i + 1));
        }
    }
    s.push_str("Let t be argmax of L.\n");
    s
}

fn chip_index(values: &[f64]) -> i64 {
    let p = composed(&argmax_program(values));
    let out = eval_choreography(&p, &inputs_from_fills(&p)).expect("the arg-max evaluates");
    let t = p.tensors.iter().find(|t| t.name == "t").expect("t").id;
    match out.tensors.get(&t).expect("t is written") {
        Value::Int(v) => v[0],
        Value::Float(v) => v[0] as i64,
    }
}

fn vm_index(values: &[f64]) -> i64 {
    let vm = vm_outcome(&format!("{}Show t.\n", argmax_program(values)));
    assert!(vm.error.is_none(), "{:?}", vm.error);
    vm.output.trim().parse().expect("an index")
}

fn agrees(what: &str, values: &[f64]) {
    let (chip, vm) = (chip_index(values), vm_index(values));
    assert_eq!(
        chip, vm,
        "{what}: the chip's arg-max is {chip}, the VM's is {vm}"
    );
}

/// Bf16-exact values in (−0.5, 0.5), none at the extremes the cases plant.
fn row(n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| (((i * 37) % 61) as f64 - 30.0) / 64.0)
        .collect()
}

/// ── AT A WIDTH THAT BLOCKS, THE VM'S INDEX: start, end, a block boundary.
#[test]
fn the_blocked_argmax_finds_the_maximum_wherever_it_is() {
    let n = 5000;
    for at in [0, n - 1, 255, 256, 257, 2048, 4095, 4096] {
        let mut v = row(n);
        v[at] = 1.0;
        agrees(&format!("the maximum at {at}"), &v);
    }
}

/// ── A TIE ACROSS BLOCKS: the first of equals wins, as the VM's `>` scan keeps it.
#[test]
fn a_tie_across_blocks_keeps_the_first() {
    let n = 5000;
    for (a, b) in [(10, 4000), (255, 256), (1000, 1001), (300, 4999)] {
        let mut v = row(n);
        v[a] = 1.0;
        v[b] = 1.0;
        agrees(&format!("equal maxima at {a} and {b}"), &v);
    }
}

/// ── A ROW OF NEGATIVES WITH A PARTIAL TAIL BLOCK: the tail's padding reads as 0 and must never win.
#[test]
fn padding_never_beats_a_row_of_negatives() {
    for n in [5000, 4099, 300] {
        let v: Vec<f64> = (0..n)
            .map(|i| -1.0 - ((i * 13) % 17) as f64 / 64.0)
            .collect();
        agrees(&format!("all negative, width {n}"), &v);
    }
}

/// ── A ROW BELOW EVERY FINITE NUMBER A MASK COULD BE: a masked position folds as the maximum's identity, not as a
/// large negative. Values of `−2¹⁰⁴·(1 + k/128)` are bf16 logits like any other. Width 5000 masks the tail block's
/// padding; width 2816 is 88 tiles, 11 blocks under a 4 × 3 merge tree, so one PAD block rides the tree too.
#[test]
fn a_row_below_any_finite_mask_still_finds_its_maximum() {
    let floor = -(2f64.powi(104));
    for (n, at) in [
        (5000, 4999),
        (5000, 1234),
        (300, 299),
        (300, 17),
        (2816, 2000),
        (2816, 0),
    ] {
        let mut v: Vec<f64> = (0..n)
            .map(|i| floor * (1.0 + (1 + i % 61) as f64 / 128.0))
            .collect();
        v[at] = floor;
        agrees(
            &format!("width {n}, every value below −1e30, the maximum at {at}"),
            &v,
        );
    }
}

/// The VM's arg-max of `values` with a NaN at each of `nans` — made the one way the language can, `∞ − ∞`.
fn vm_index_with_nans(values: &[f64], nans: &[usize]) -> i64 {
    let src = argmax_program(values);
    let at = src.find("Let t be argmax").expect("the arg-max statement");
    let mut s = format!(
        "{}Let big be 1.0e308.\nLet inf be big * 10.0.\nLet nan be inf - inf.\n",
        &src[..at]
    );
    for i in nans {
        s.push_str(&format!("Set item {} of L to nan.\n", i + 1));
    }
    s.push_str(&src[at..]);
    s.push_str("Show t.\n");
    let vm = vm_outcome(&s);
    assert!(vm.error.is_none(), "{:?}", vm.error);
    vm.output.trim().parse().expect("an index")
}

/// The chip's arg-max of the same row: the program lowered from the literal row, and the NaNs placed in its input —
/// no fill can spell a computed NaN, so the differential hands both sides the same row directly.
fn chip_index_with_nans(values: &[f64], nans: &[usize]) -> i64 {
    let p = composed(&argmax_program(values));
    let mut inputs = inputs_from_fills(&p);
    let l = p.tensors.iter().find(|t| t.name == "L").expect("L").id;
    match inputs.tensors.get_mut(&l).expect("L is an input") {
        Value::Float(v) => nans.iter().for_each(|i| v[*i] = f64::NAN),
        Value::Int(_) => panic!("a Real row"),
    }
    let out = eval_choreography(&p, &inputs).expect("the arg-max evaluates");
    let t = p.tensors.iter().find(|t| t.name == "t").expect("t").id;
    match out.tensors.get(&t).expect("t is written") {
        Value::Int(v) => v[0],
        Value::Float(v) => v[0] as i64,
    }
}

/// ── A NaN IS THE VM'S ANSWER EXACTLY WHEN IT IS ITEM 1. The VM's scan starts from item 1 and replaces it only on
/// `>`, which a NaN never satisfies in either direction: a NaN first is never replaced, a NaN later never replaces.
/// So a NaN at the start of a LATER block must not capture that block — the block's own maximum still competes.
/// Single-site (width 200) and blocked (width 5000; 256 starts block 1, 4768 starts the overlapping tail block).
#[test]
fn a_nan_is_the_answer_exactly_when_it_is_item_one() {
    for n in [200usize, 5000] {
        let mut cases: Vec<(Vec<usize>, usize)> = vec![
            (vec![0], n - 1),
            (vec![0], 1),
            (vec![5], 100),
            (vec![1, 2], 0),
            ((0..n).collect(), 0),
        ];
        if n == 5000 {
            cases.extend([
                (vec![256], 300),
                (vec![4768], 4900),
                (vec![1000], 1001),
                (vec![0, 256, 512], 3000),
                (vec![255, 256], 257),
            ]);
        }
        for (nans, at) in cases {
            let mut v = row(n);
            v[at] = 1.0;
            let (chip, vm) = (
                chip_index_with_nans(&v, &nans),
                vm_index_with_nans(&v, &nans),
            );
            assert_eq!(chip, vm, "width {n}, NaN at {:?}, maximum at {at}: the chip's arg-max is {chip}, the VM's is {vm}", &nans[..nans.len().min(4)]);
        }
    }
}

/// An arg-max over integer `values`, and the index the VM computes for it.
fn int_argmax_program(values: &[i64]) -> String {
    let mut s = format!("## Main\nLet L be a 1 by {} tensor of Int.\n", values.len());
    for (i, v) in values.iter().enumerate() {
        if *v != 0 {
            s.push_str(&format!("Set item {} of L to {v}.\n", i + 1));
        }
    }
    s.push_str("Let t be argmax of L.\n");
    s
}

/// ── AN INTEGER ROW BLOCKS TOO: its partials are integers, its mask the type's minimum — which a live value may
/// equal, and position order still keeps the live one.
#[test]
fn an_integer_row_blocks_too() {
    let low = i32::MIN as i64 + 1;
    for (n, at, tie) in [
        (5000usize, 4999usize, None),
        (5000, 3, Some(4000usize)),
        (300, 299, None),
        (2816, 1500, Some(2815)),
    ] {
        let mut v: Vec<i64> = (0..n).map(|i| low + (i % 7) as i64).collect();
        v[at] = low + 100;
        if let Some(b) = tie {
            v[b] = low + 100;
        }
        let src = int_argmax_program(&v);
        let p = composed(&src);
        let out = eval_choreography(&p, &inputs_from_fills(&p)).expect("the arg-max evaluates");
        let t = p.tensors.iter().find(|t| t.name == "t").expect("t").id;
        let chip = match out.tensors.get(&t).expect("t is written") {
            Value::Int(v) => v[0],
            Value::Float(v) => v[0] as i64,
        };
        let vm = vm_outcome(&format!("{src}Show t.\n"));
        assert!(vm.error.is_none(), "{:?}", vm.error);
        let want: i64 = vm.output.trim().parse().expect("an index");
        assert_eq!(chip, want, "width {n}, integer maximum at {at} (tie {tie:?}): the chip's arg-max is {chip}, the VM's is {want}");
    }
}

/// ── THE REAL VOCABULARY LOWERS AND CERTIFIES.
#[test]
fn an_argmax_over_the_real_vocabulary_lowers() {
    let vocab = Qwen38Cfg::real().vocab;
    let p = composed(&format!(
        "## Main\nLet L be a 1 by {vocab} tensor of Real.\nLet t be argmax of L.\n"
    ));
    assert!(
        p.certificate.as_ref().is_some_and(|c| c.recheck(&p)),
        "the arg-max over {vocab} logits certifies"
    );
    println!("argmax over {vocab}: {} sections", p.sections.len());
}

/// ── AND THE REAL DECODE HEAD — final norm, LM head, arg-max — lowers whole.
#[test]
fn the_real_decode_head_lowers_whole() {
    let mut cfg = Qwen38Cfg::real();
    cfg.layers = 1;
    let (src, _, _) = gen_qwen38_decode_head_lg(&cfg, "/w");
    let l = compile_program_to_spatial(&src, &RduChip::Sn50.facts().spatial_target())
        .expect("the head is a tensor program");
    assert_eq!(
        l.lowered,
        l.statements,
        "every statement of the decode head lowers: {:?}",
        l.missing.first()
    );
    assert!(
        l.whole().is_some(),
        "the decode head composes and certifies: {}",
        l.refusal()
    );
}
