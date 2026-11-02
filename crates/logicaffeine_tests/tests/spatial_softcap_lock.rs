//! spatial_softcap_lock — a soft-capped attention (`capped at C`, Gemma 2's scores) lowers onto a spatial chip and takes
//! each score `s` as `C·tanh(s/C)` after its scale and before its softmax, and a soft-capped tensor (`X capped at C`,
//! Gemma 2's logits) takes every element so, as the VM does.
//!
//! The program declares `Q` (rows by heads·hd), `K` and `V` (l by groups·hd), fills them with `Real` literals, and binds
//! `C` to a capped attention. Lowered whole for the SN50 (`compile_program_to_spatial`) and run through the spatial
//! evaluator, every entry of `C` is the VM's within the bf16 datapath's error. Covered: a decode row over its whole cache
//! and a causal block ending at the cache's last row; one head and heads grouped over a key/value head; heads 2 wide (the
//! sections form) and 32 wide (the flash form, its cache streaming in blocks, the cap applied before each block's
//! running maximum); alone, after a scale, inside a window and beside sinks. Every cap binds: a scale of 4 brings these
//! scores to about ±3 against a cap of 0.5, and the capped attention is not the uncapped one; a scale of 200 puts them
//! past ±100, far past the cap, where a cap taken after the running maximum would underflow every weight.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test spatial_softcap_lock`

#![cfg(feature = "codegen")]

#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;
use spatial_support::inputs_from_fills;

/// One attention to lower: query rows, query heads over key/value heads, the head width, the cache length, the position
/// of its first row, and the attention with its clauses.
struct Case {
    rows: usize,
    heads: usize,
    groups: usize,
    hd: usize,
    l: usize,
    pos: usize,
    op: String,
    /// The `with sinks S` clause's `S`, one logit per query head, when the case has it.
    sinks: bool,
}

fn q(r: usize, h: usize, c: usize) -> f64 {
    ((r * 7 + h * 3 + c) % 11) as f64 / 11.0 - 0.45
}
fn k(j: usize, g: usize, c: usize) -> f64 {
    ((j * 5 + g * 2 + c * 3) % 13) as f64 / 13.0 - 0.5
}
fn v(j: usize, g: usize, c: usize) -> f64 {
    ((j * 3 + g + c * 5) % 17) as f64 / 17.0 - 0.4
}

/// The case's program, and the program with `C` shown entry by entry for the VM.
fn programs(c: &Case) -> (String, String) {
    let (qw, kw) = (c.heads * c.hd, c.groups * c.hd);
    let mut s = format!("## Main\nLet Q be a {} by {qw} tensor of Real.\n", c.rows);
    for r in 0..c.rows {
        for h in 0..c.heads {
            for e in 0..c.hd {
                s.push_str(&format!("Set item {} of Q to {:?}.\n", r * qw + h * c.hd + e + 1, q(r, h, e)));
            }
        }
    }
    s.push_str(&format!("Let K be a {} by {kw} tensor of Real.\nLet V be a {} by {kw} tensor of Real.\n", c.l, c.l));
    for j in 0..c.l {
        for g in 0..c.groups {
            for e in 0..c.hd {
                s.push_str(&format!("Set item {} of K to {:?}.\n", j * kw + g * c.hd + e + 1, k(j, g, e)));
                s.push_str(&format!("Set item {} of V to {:?}.\n", j * kw + g * c.hd + e + 1, v(j, g, e)));
            }
        }
    }
    if c.sinks {
        s.push_str(&format!("Let S be a 1 by {} tensor of Real.\n", c.heads));
        for h in 0..c.heads {
            s.push_str(&format!("Set item {} of S to {:?}.\n", h + 1, 0.3 - 0.45 * h as f64));
        }
    }
    let sinks = if c.sinks { " with sinks S" } else { "" };
    s.push_str(&format!("Let pos be {}.\nLet C be {}{sinks}.\n", c.pos, c.op));
    (s.clone(), shown(&s, c.rows * qw))
}

/// `src` with each of `C`'s `n` entries shown.
fn shown(src: &str, n: usize) -> String {
    let mut out = src.to_string();
    for i in 1..=n {
        out.push_str(&format!("Show item {i} of C.\n"));
    }
    out
}

/// `C` as the VM computes it.
fn on_the_vm(shown: &str) -> Vec<f64> {
    let vm = vm_outcome(shown);
    assert!(vm.error.is_none(), "the VM runs the program: {:?}\n{shown}", vm.error);
    vm.output.split_whitespace().map(|t| t.parse().expect("a number")).collect()
}

/// `C` as the spatial evaluator computes the program lowered whole for `target`.
fn on_the_chip(src: &str, target: &SpatialTarget) -> Vec<f64> {
    let prog = compile_program_to_spatial(src, target).expect("a tensor program").into_whole().unwrap_or_else(|why| panic!("{why}"));
    let out = eval_choreography(&prog, &inputs_from_fills(&prog)).expect("the program evaluates");
    let c = prog.tensors.iter().find(|t| t.name == "C").expect("C").id;
    match out.tensors.get(&c).expect("the evaluation hands back C") {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    }
}

/// `src`'s `C` on the chip is the VM's, each entry within the bf16 datapath's error of the largest.
fn agree_on(what: &str, src: &str, shown: &str) {
    let (chip, vm) = (on_the_chip(src, &SpatialTarget::sn50_rdu()), on_the_vm(shown));
    assert_eq!(chip.len(), vm.len(), "{what}: C's size");
    let scale = vm.iter().fold(0.0f64, |m, x| m.max(x.abs())).max(1e-9);
    for (i, (a, b)) in chip.iter().zip(&vm).enumerate() {
        assert!((a - b).abs() <= 0.02 * scale, "{what}: C[{i}] is {a} on the chip, {b} on the VM\nchip {chip:?}\nvm   {vm:?}");
    }
}

fn agree(cases: &[Case]) {
    for c in cases {
        let (src, shown) = programs(c);
        agree_on(&format!("{} (hd {})", c.op, c.hd), &src, &shown);
    }
}

/// The decode row at the last position of a cache of `l`, over `heads` query heads and `groups` key/value heads `hd`
/// wide, its `clauses` after its length.
fn decode(heads: usize, groups: usize, hd: usize, l: usize, clauses: &str) -> Case {
    let op = format!("attention of Q over K and V with {heads} heads {groups} groups up to pos{clauses}");
    Case { rows: 1, heads, groups, hd, l, pos: l, op, sinks: false }
}

/// A causal block of `rows` rows whose last row is the cache's last, its `clauses` after its start.
fn causal(rows: usize, hd: usize, l: usize, clauses: &str) -> Case {
    let op = format!("causal attention of Q over K and V with 1 heads 1 groups from pos{clauses}");
    Case { rows, heads: 1, groups: 1, hd, l, pos: l - rows + 1, op, sinks: false }
}

fn with_sinks(mut c: Case) -> Case {
    c.sinks = true;
    c
}

#[test]
fn capped_scores_equal_the_vm() {
    // The sections form, heads 2 wide.
    agree(&[
        decode(1, 1, 2, 8, " capped at 0.1"),
        decode(1, 1, 2, 8, " scaled by 4.0 capped at 0.5"),
        decode(4, 2, 2, 8, " scaled by 4.0 capped at 0.5"),
        causal(3, 2, 8, " scaled by 4.0 capped at 0.5"),
        decode(2, 1, 2, 8, " within a window of 3 scaled by 4.0 capped at 0.5"),
        with_sinks(decode(4, 2, 2, 8, " scaled by 4.0 capped at 0.5")),
        decode(2, 1, 2, 8, " scaled by 200.0 capped at 0.5"),
    ]);
}

#[test]
fn a_flash_attention_caps_each_block_before_its_running_maximum() {
    // The flash form, heads 32 wide: a cache of 70 positions streams in three blocks, each block's scores capped before
    // they join the running maximum and sum.
    agree(&[
        decode(1, 1, 32, 70, " scaled by 4.0 capped at 0.5"),
        decode(4, 1, 32, 70, " scaled by 4.0 capped at 0.5"),
        with_sinks(decode(2, 1, 32, 70, " within a window of 40 scaled by 4.0 capped at 0.5")),
        causal(3, 32, 70, " scaled by 4.0 capped at 0.5"),
    ]);
}

#[test]
fn scores_far_past_the_cap_equal_the_vm() {
    // Scores past ±100 against a cap of 0.5: every capped score is within a hair of ±0.5. A running maximum of the
    // uncapped scores would sit 100 above every capped one and underflow every weight to zero.
    agree(&[
        decode(1, 1, 32, 70, " scaled by 200.0 capped at 0.5"),
        decode(4, 1, 32, 70, " scaled by 200.0 capped at 0.5"),
        causal(3, 32, 70, " scaled by 200.0 capped at 0.5"),
        with_sinks(decode(2, 1, 32, 70, " scaled by 200.0 capped at 0.5")),
    ]);
}

#[test]
fn a_cap_changes_the_attention() {
    // Not vacuous: on these scores each cap binds, so the capped result is not the uncapped one by a twentieth of the
    // largest entry, past the bf16 datapath's error.
    let target = SpatialTarget::sn50_rdu();
    for (plain, capped) in [
        (decode(1, 1, 2, 8, ""), decode(1, 1, 2, 8, " capped at 0.1")),
        (decode(1, 1, 2, 8, " scaled by 4.0"), decode(1, 1, 2, 8, " scaled by 4.0 capped at 0.5")),
        (decode(1, 1, 32, 70, " scaled by 4.0"), decode(1, 1, 32, 70, " scaled by 4.0 capped at 0.5")),
        (decode(1, 1, 32, 70, " scaled by 200.0"), decode(1, 1, 32, 70, " scaled by 200.0 capped at 0.5")),
    ] {
        let (a, b) = (on_the_chip(&programs(&plain).0, &target), on_the_chip(&programs(&capped).0, &target));
        let scale = a.iter().chain(&b).fold(0.0f64, |m, x| m.max(x.abs()));
        assert!(
            a.iter().zip(&b).any(|(x, y)| (x - y).abs() > 0.05 * scale),
            "{}: the cap moves the result: {a:?} vs {b:?}",
            capped.op
        );
    }
}

#[test]
fn capped_logits_equal_the_vm() {
    // Gemma 2's logits, `30·tanh(z/30)`, over a row of them spanning the cap: small ones all but unmoved, large ones near
    // ±30, every one as the VM computes it.
    let zs: Vec<f64> = (0..96).map(|i| (i as f64 - 47.5) * 1.75).collect();
    let mut src = format!("## Main\nLet X be a 1 by {} tensor of Real.\n", zs.len());
    for (i, z) in zs.iter().enumerate() {
        src.push_str(&format!("Set item {} of X to {z:?}.\n", i + 1));
    }
    src.push_str("Let C be X capped at 30.0.\n");
    agree_on("logits capped at 30", &src, &shown(&src, zs.len()));
}
