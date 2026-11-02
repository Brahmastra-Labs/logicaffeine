//! spatial_attention_window_lock — a windowed attention (`within a window of W`) lowers onto a spatial chip and keeps
//! the last `W` positions of each query row, a chunked-local one (`within chunks of C`, Llama 4's) keeps the positions
//! of each row's own chunk of `C`, and an attention with sinks (`with sinks S`) puts each head's sink logit in its
//! softmax's denominator and nowhere else, as the VM does.
//!
//! The program declares `Q` (rows by heads·hd), `K` and `V` (l by groups·hd), fills them with `Real` literals, and binds
//! `C` to an attention. Lowered whole for the SN50 (`compile_program_to_spatial`) and run through the spatial evaluator,
//! every entry of `C` is the VM's within the bf16 datapath's error. Covered: a decode row and a causal block of rows;
//! windows of one position, narrower than the history, as wide as it and wider; one head and query heads grouped over a
//! key/value head; heads 2 wide (the sections form) and 32 wide (the flash form, whose cache streams in tile-row blocks
//! — a window then falls across blocks, and blocks wholly before it are masked whole). Sinks are covered in both forms,
//! on a decode row, grouped heads and a causal block, alone and inside a window (gpt-oss's attention).
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test spatial_attention_window_lock`

#![cfg(feature = "codegen")]

#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;
use spatial_support::inputs_from_fills;

/// One attention to lower: query rows, query heads over key/value heads, the head width, the cache length, the first
/// row's position and the clause.
struct Case {
    rows: usize,
    heads: usize,
    groups: usize,
    hd: usize,
    l: usize,
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
    s.push_str(&format!("Let pos be 5.\nLet C be {}{sinks}.\n", c.op));
    let mut shown = s.clone();
    for i in 1..=c.rows * qw {
        shown.push_str(&format!("Show item {i} of C.\n"));
    }
    (s, shown)
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

/// Every case's `C` on the chip is the VM's, each entry within the bf16 datapath's error of the largest.
fn agree(cases: &[Case]) {
    let target = SpatialTarget::sn50_rdu();
    for c in cases {
        let (src, shown) = programs(c);
        let (chip, vm) = (on_the_chip(&src, &target), on_the_vm(&shown));
        assert_eq!(chip.len(), vm.len(), "{}: C's size", c.op);
        let scale = vm.iter().fold(0.0f64, |m, x| m.max(x.abs())).max(1e-9);
        for (i, (a, b)) in chip.iter().zip(&vm).enumerate() {
            assert!((a - b).abs() <= 0.02 * scale, "{} (hd {}): C[{i}] is {a} on the chip, {b} on the VM\nchip {chip:?}\nvm   {vm:?}", c.op, c.hd);
        }
    }
}

/// The decode row at position 5 of a cache of `l` over `heads` query heads and `groups` key/value heads `hd` wide,
/// windowed by `w`.
fn decode(heads: usize, groups: usize, hd: usize, l: usize, w: Option<usize>) -> Case {
    let window = w.map_or(String::new(), |w| format!(" within a window of {w}"));
    Case { rows: 1, heads, groups, hd, l, op: format!("attention of Q over K and V with {heads} heads {groups} groups up to pos{window}"), sinks: false }
}

/// A causal block of `rows` rows from position 5.
fn causal(rows: usize, hd: usize, l: usize, w: usize) -> Case {
    Case { rows, heads: 1, groups: 1, hd, l, op: format!("causal attention of Q over K and V with 1 heads 1 groups from pos within a window of {w}"), sinks: false }
}

#[test]
fn a_decode_row_keeps_the_last_w_positions_on_the_chip() {
    agree(&[1, 2, 3, 4, 5, 6, 40].map(|w| decode(1, 1, 2, 8, Some(w))));
}

#[test]
fn every_head_of_a_group_keeps_the_same_window_on_the_chip() {
    agree(&[decode(2, 1, 2, 8, Some(2)), decode(4, 2, 2, 8, Some(3))]);
}

#[test]
fn each_row_of_a_causal_block_keeps_its_own_window_on_the_chip() {
    agree(&[causal(3, 2, 8, 2), causal(3, 2, 8, 1), causal(2, 2, 8, 7)]);
}

#[test]
fn a_flash_attention_keeps_the_window_across_its_blocks() {
    // Heads of 32 take the flash form; a cache of 70 positions is three blocks of 32 (the last partial), so position 5's
    // window ends in the first block and a causal block's rows reach the second.
    agree(&[decode(1, 1, 32, 70, Some(3)), decode(2, 1, 32, 70, Some(4)), decode(1, 1, 32, 70, Some(70))]);
    let mut late = decode(1, 1, 32, 70, Some(10));
    late.op = late.op.replace("up to pos", "up to 60");
    agree(&[late]);
    agree(&[causal(3, 32, 70, 2)]);
}

#[test]
fn an_unwindowed_attention_is_unchanged() {
    agree(&[decode(1, 1, 2, 8, None), decode(1, 1, 32, 70, None)]);
}

/// `c` with each head's sink logit in its softmax.
fn with_sinks(mut c: Case) -> Case {
    c.sinks = true;
    c
}

#[test]
fn sinks_equal_the_vm() {
    // The sections form (heads 2 wide): a decode row, grouped heads, a causal block, each with sinks, one inside a window.
    agree(&[
        with_sinks(decode(1, 1, 2, 8, None)),
        with_sinks(decode(4, 2, 2, 8, None)),
        with_sinks(decode(2, 1, 2, 8, Some(3))),
        with_sinks(causal(3, 2, 8, 2)),
    ]);
}

#[test]
fn a_flash_attention_starts_its_softmax_from_the_sinks() {
    // The flash form (heads 32 wide): one head, heads grouped over a key/value head (one item per key/value head), and
    // gpt-oss's shape, a window with sinks.
    agree(&[
        with_sinks(decode(1, 1, 32, 70, None)),
        with_sinks(decode(4, 1, 32, 70, None)),
        with_sinks(decode(2, 1, 32, 70, Some(4))),
        with_sinks(causal(3, 32, 70, 2)),
    ]);
}

#[test]
fn a_sink_changes_the_attention() {
    // Not vacuous: the sinks pull weight off the positions, so the result moves.
    let target = SpatialTarget::sn50_rdu();
    for c in [decode(1, 1, 2, 8, None), decode(1, 1, 32, 70, None)] {
        let plain = on_the_chip(&programs(&c).0, &target);
        let sunk = on_the_chip(&programs(&with_sinks(c)).0, &target);
        assert!(plain.iter().zip(&sunk).any(|(a, b)| (a - b).abs() > 1e-3), "the sinks move the result: {plain:?} vs {sunk:?}");
    }
}

#[test]
fn a_window_over_a_literal_bound_keeps_its_last_w_positions_on_the_chip() {
    // No position: the query attends the whole cache, its window the cache's last `W` rows; a causal block from the
    // literal position 3. Each in the sections form (heads 2 wide) and the flash form (heads 32 wide).
    let whole = |hd: usize, l: usize, w: usize| Case {
        rows: 1,
        heads: 1,
        groups: 1,
        hd,
        l,
        op: format!("attention of Q over K and V with 1 heads 1 groups within a window of {w}"),
        sinks: false,
    };
    let from_3 = |rows: usize, hd: usize, l: usize, w: usize| Case {
        rows,
        heads: 1,
        groups: 1,
        hd,
        l,
        op: format!("causal attention of Q over K and V with 1 heads 1 groups from 3 within a window of {w}"),
        sinks: false,
    };
    agree(&[whole(2, 8, 3), whole(2, 8, 1), from_3(3, 2, 8, 2), whole(32, 70, 3), from_3(3, 32, 70, 2)]);
}

/// The decode row at position 5 of a cache of `l`, attending only its own chunk of `c` positions (Llama 4's
/// chunked-local attention). Positions count from 1 as the language counts them: a query attending `n` positions keeps,
/// within chunks of `C`, the positions from `⌊(n − 1)/C⌋·C + 1` to `n`.
fn chunked(heads: usize, groups: usize, hd: usize, l: usize, c: usize) -> Case {
    Case {
        rows: 1,
        heads,
        groups,
        hd,
        l,
        op: format!("attention of Q over K and V with {heads} heads {groups} groups up to pos within chunks of {c}"),
        sinks: false,
    }
}

/// A causal block of `rows` rows from `from`, each row within its own chunk of `c`.
fn chunked_causal(rows: usize, hd: usize, l: usize, from: &str, c: usize) -> Case {
    Case {
        rows,
        heads: 1,
        groups: 1,
        hd,
        l,
        op: format!("causal attention of Q over K and V with 1 heads 1 groups from {from} within chunks of {c}"),
        sinks: false,
    }
}

#[test]
fn a_chunked_local_layer_equals_the_vm() {
    // Position 5 is a chunk of 1 alone, opens chunks of 2 and 4 (it attends itself), sits second in a chunk of 3 (4..5),
    // ends one of 5 (1..5); a chunk of 40 holds every position. Grouped heads keep one chunk.
    agree(&[1, 2, 3, 4, 5, 40].map(|c| chunked(1, 1, 2, 8, c)));
    agree(&[chunked(4, 2, 2, 8, 3)]);
    // A causal block across chunk boundaries: rows at 5, 6, 7 in chunks of 2 attend [5], [5, 6], [7]; in chunks of 3,
    // [4, 5], [4, 5, 6], [7].
    agree(&[chunked_causal(3, 2, 8, "pos", 2), chunked_causal(3, 2, 8, "pos", 3)]);
    // The flash form (heads 32 wide, a cache of 70 in blocks of 32): a query at 60 in a chunk of 32 (33..60) that starts
    // in the second block, so the first is masked whole; small chunks inside the first block; a causal block.
    let mut late = chunked(1, 1, 32, 70, 32);
    late.op = late.op.replace("up to pos", "up to 60");
    agree(&[late, chunked(2, 1, 32, 70, 4), chunked_causal(3, 32, 70, "pos", 2)]);
}

#[test]
fn a_chunked_local_layer_over_a_literal_bound_equals_the_vm() {
    // No position: the query attends the whole cache, its chunk the one the cache's last row sits in; a causal block from
    // the literal position 3. Each in the sections form (heads 2 wide) and the flash form (heads 32 wide).
    let whole = |hd: usize, l: usize, c: usize| Case {
        rows: 1,
        heads: 1,
        groups: 1,
        hd,
        l,
        op: format!("attention of Q over K and V with 1 heads 1 groups within chunks of {c}"),
        sinks: false,
    };
    agree(&[whole(2, 8, 3), whole(2, 8, 1), chunked_causal(3, 2, 8, "3", 2), whole(32, 70, 32), chunked_causal(3, 32, 70, "3", 2)]);
}

#[test]
fn a_chunk_is_not_a_window() {
    // Not vacuous: at position 60 a window of 32 keeps 29..60 and a chunk of 32 keeps 33..60, so the results differ.
    let target = SpatialTarget::sn50_rdu();
    let mut window = decode(1, 1, 32, 70, Some(32));
    window.op = window.op.replace("up to pos", "up to 60");
    let mut chunk = chunked(1, 1, 32, 70, 32);
    chunk.op = chunk.op.replace("up to pos", "up to 60");
    let (w, c) = (on_the_vm(&programs(&window).1), on_the_vm(&programs(&chunk).1));
    assert!(w.iter().zip(&c).any(|(a, b)| (a - b).abs() > 1e-3), "a chunk keeps other positions than a window: {w:?} vs {c:?}");
    assert_eq!(on_the_chip(&programs(&chunk).0, &target).len(), c.len());
}
