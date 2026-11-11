//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! AN ATTENTION ATTENDS THE WINDOW THE PROGRAM GIVES IT.
//!
//! Sliding-window attention (Gemma, gpt-oss, Mistral) keeps, for a query at position `p`, only the cached positions
//! `p − W + 1 ..= p`: the query's own and the `W − 1` before it, llama.cpp's `p − p₀ < W`. The sugar takes
//! `within a window of W` after the attended length — `attention … up to P`, `causal attention … from P`, and the
//! unbounded form, whose length is the cache's — and every tier computes it over the window alone.
//!
//!   • the VM, the tree-walker, the C backend and AOT-Rust compute the windowed softmax the `f64` formula does, on
//!     inputs where the positions outside the window change the answer;
//!   • a window as wide as the history, or wider, is the whole history;
//!   • each row of a causal block attends its own window;
//!   • the tensor front end lifts the window into the op's spec (`AttentionSpec::window`);
//!   • a window that is not a positive whole number is refused by name;
//!   • the spatial IR lowers the window (held to the VM by `spatial_attention_window_lock`), and the CUDA forward, which
//!     has no windowed kernel, refuses the op by name rather than attend the whole cache.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use logicaffeine_compile::codegen_cuda::knobs::CudaKnobs;
use logicaffeine_compile::codegen_cuda::lane::{compile, LaneOptions};
use logicaffeine_compile::compile::{compile_to_cuda_forward, tw_outcome, vm_outcome};
use logicaffeine_compile::model::spec::Mixer;
use logicaffeine_compile::spatial::passes::recognize_stages::{recognize_stages, TensorOpSpec};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::tensor_recognize::qwen::Window;
use tiny_models::{write_checkpoint, Tiny};

/// The cache: six positions of one key/value head two wide.
const L: usize = 6;
const HD: usize = 2;

fn key(j: usize, c: usize) -> f64 {
    [0.3 * j as f64 - 1.0, 0.2 * (j % 3) as f64 - 0.1][c]
}

fn value(j: usize, c: usize) -> f64 {
    [0.5 * j as f64 - 1.0, 0.25 * (7 - j) as f64][c]
}

/// Query `r` (1-based) of `rows`, `heads` heads wide.
fn query(r: usize, head: usize, c: usize) -> f64 {
    [0.8, -0.6][c] + 0.15 * r as f64 - 0.35 * head as f64
}

/// The program: `Q` (`rows` by `heads·2`), `K` and `V` (6 by 2) declared and filled with `Real` literals, `C` bound by
/// `op`, every entry of `C` shown as `round(c·1e9)`.
fn program(op: &str, rows: usize, heads: usize) -> String {
    let mut s = format!("## Main\nLet Q be a {rows} by {} tensor of Real.\n", heads * HD);
    for r in 1..=rows {
        for h in 0..heads {
            for c in 0..HD {
                s.push_str(&format!("Set item {} of Q to {:?}.\n", (r - 1) * heads * HD + h * HD + c + 1, query(r, h, c)));
            }
        }
    }
    s.push_str(&format!("Let K be a {L} by {HD} tensor of Real.\nLet V be a {L} by {HD} tensor of Real.\n"));
    for j in 1..=L {
        for c in 0..HD {
            s.push_str(&format!("Set item {} of K to {:?}.\n", (j - 1) * HD + c + 1, key(j, c)));
            s.push_str(&format!("Set item {} of V to {:?}.\n", (j - 1) * HD + c + 1, value(j, c)));
        }
    }
    s.push_str(&format!("Let pos be 5.\nLet C be {op}.\n"));
    for i in 1..=rows * heads * HD {
        s.push_str(&format!("Show round(item {i} of C * 1000000000).\n"));
    }
    s
}

/// The `f64` attention of query row `r` at position `n` over the positions `max(1, n − w + 1) ..= n`, every head reading
/// the one key/value head.
fn oracle(r: usize, heads: usize, n: usize, w: Option<usize>) -> Vec<i64> {
    let lo = w.map_or(1, |w| n.saturating_sub(w - 1).max(1));
    let scale = 1.0 / (HD as f64).sqrt();
    (0..heads)
        .flat_map(|h| {
            let scores: Vec<f64> = (lo..=n).map(|j| (0..HD).map(|c| query(r, h, c) * key(j, c)).sum::<f64>() * scale).collect();
            let m = scores.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let den: f64 = scores.iter().map(|s| (s - m).exp()).sum();
            (0..HD)
                .map(|c| {
                    let a: f64 = (lo..=n).zip(&scores).map(|(j, s)| (s - m).exp() / den * value(j, c)).sum();
                    (a * 1e9).round() as i64
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn shown(src: &str) -> Vec<i64> {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "VM error on:\n{src}\n{:?}", vm.error);
    let tw = tw_outcome(src);
    assert!(tw.error.is_none(), "tree-walker error: {:?}", tw.error);
    assert_eq!(tw.output.trim_end(), vm.output.trim_end(), "the tree-walker agrees with the VM");
    vm.output.split_whitespace().map(|t| t.parse().expect("an integer")).collect()
}

/// Each value within one unit of the last place shown: the tiers sum in program order, the oracle in its own.
fn close(got: &[i64], want: &[i64], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: {got:?} vs {want:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g - w).abs() <= 1, "{what}: {got:?} vs the formula's {want:?}");
    }
}

#[test]
fn a_window_keeps_the_last_w_positions_of_a_query() {
    let windowed = shown(&program("attention of Q over K and V with 1 heads 1 groups up to pos within a window of 3", 1, 1));
    close(&windowed, &oracle(1, 1, 5, Some(3)), "a window of 3 at position 5 attends 3, 4 and 5");
    let whole = oracle(1, 1, 5, None);
    assert!(
        windowed.iter().zip(&whole).any(|(a, b)| (a - b).abs() > 1000),
        "the inputs make positions 1 and 2 matter, so a tier that ignored the window would show it"
    );
    close(
        &shown(&program("attention of Q over K and V with 1 heads 1 groups up to pos within a window of 1", 1, 1)),
        &oracle(1, 1, 5, Some(1)),
        "a window of 1 is the query's own position",
    );
    close(
        &shown(&program("attention of Q over K and V with 1 heads 1 groups within a window of 2", 1, 1)),
        &oracle(1, 1, L, Some(2)),
        "with no position the window ends at the cache's last row",
    );
}

#[test]
fn every_head_of_a_group_attends_the_same_window() {
    close(
        &shown(&program("attention of Q over K and V with 2 heads 1 groups up to pos within a window of 2", 1, 2)),
        &oracle(1, 2, 5, Some(2)),
        "two query heads over one key/value head",
    );
}

#[test]
fn a_window_as_wide_as_the_history_is_the_whole_history() {
    let whole = shown(&program("attention of Q over K and V with 1 heads 1 groups up to pos", 1, 1));
    close(&whole, &oracle(1, 1, 5, None), "the unwindowed attention");
    for w in [5, 6, 100] {
        let op = format!("attention of Q over K and V with 1 heads 1 groups up to pos within a window of {w}");
        assert_eq!(shown(&program(&op, 1, 1)), whole, "a window of {w} at position 5 is the whole history");
    }
}

#[test]
fn each_row_of_a_causal_block_attends_its_own_window() {
    let got = shown(&program("causal attention of Q over K and V with 1 heads 1 groups from 3 within a window of 2", 3, 1));
    let want: Vec<i64> = (1..=3).flat_map(|r| oracle(r, 1, 3 + r - 1, Some(2))).collect();
    close(&got, &want, "rows at positions 3, 4 and 5, each over itself and the position before");
    let unwindowed: Vec<i64> = (1..=3).flat_map(|r| oracle(r, 1, 3 + r - 1, None)).collect();
    assert_ne!(want, unwindowed, "the window binds on these rows");
}

#[test]
fn the_compiled_tiers_attend_the_window() {
    let src = program("causal attention of Q over K and V with 2 heads 1 groups from 3 within a window of 2", 3, 2);
    let want = vm_outcome(&src).output;
    common::assert_c_output(&src, want.trim());
    common::assert_exact_output(&src, want.trim());
}

#[test]
fn the_front_end_lifts_the_window_into_the_spec() {
    let lifted = |op: &str, rows: usize| {
        let rp = recognize_stages(&program(op, rows, 1)).expect("parses");
        assert!(rp.unrecognized.is_empty(), "{op}: {:?}", rp.unrecognized);
        match rp.stages.into_iter().map(|s| s.op).find(|op| matches!(op, TensorOpSpec::Attention(_))) {
            Some(TensorOpSpec::Attention(a)) => a,
            _ => panic!("{op} lifts as an attention"),
        }
    };
    for (op, rows, window) in [
        ("attention of Q over K and V with 1 heads 1 groups up to pos within a window of 3", 1, Some(Window::Sliding(3))),
        ("attention of Q over K and V with 1 heads 1 groups up to pos", 1, None),
        ("attention of Q over K and V with 1 heads 1 groups within a window of 2", 1, Some(Window::Sliding(2))),
        ("attention of Q over K and V with 1 heads 1 groups", 1, None),
        ("causal attention of Q over K and V with 1 heads 1 groups from 3 within a window of 2", 3, Some(Window::Sliding(2))),
        ("causal attention of Q over K and V with 1 heads 1 groups from 3", 3, None),
    ] {
        let a = lifted(op, rows);
        assert_eq!(a.window, window, "{op}");
        assert_eq!((a.h, a.kv, a.hd, a.l, a.rows), (1, 1, HD, L, rows), "{op}: the rest of the spec is unchanged");
    }
}

#[test]
fn a_window_that_is_not_a_positive_whole_number_is_refused() {
    for (clause, why) in [
        ("within a window of 0", "positive"),
        ("within a window of -2", "positive"),
        ("within a window of pos", "whole number"),
        ("within a window of 2.5", "whole number"),
        ("within a window", "window of"),
    ] {
        let op = format!("attention of Q over K and V with 1 heads 1 groups up to pos {clause}");
        let err = vm_outcome(&program(&op, 1, 1)).error.unwrap_or_else(|| panic!("`{clause}` is refused"));
        assert!(err.contains("window") && err.contains(why), "`{clause}` is refused naming the window and its {why}: {err}");
    }
}

#[test]
fn the_spatial_ir_lowers_the_window_and_the_cuda_forward_refuses_it_by_name() {
    let src = program("attention of Q over K and V with 1 heads 1 groups within a window of 2", 1, 1);
    // The spatial IR keeps the window (`spatial_attention_window_lock` holds it to the VM on the chip).
    let spatial = compile_program_to_spatial(&src, &SpatialTarget::wormhole()).expect("a tensor program");
    assert!(spatial.whole().is_some(), "the spatial IR lowers a windowed attention whole: {}", spatial.refusal());
    let forward = compile_to_cuda_forward(&src).expect_err("the CUDA forward has no windowed attention");
    assert!(forward.to_string().contains("window"), "the CUDA forward names the window it refuses: {forward}");
}

/// The `f64` attention of query row `r` at position `n` over the positions `lo ..= n`, every head reading the one
/// key/value head.
fn attend(r: usize, heads: usize, n: usize, lo: usize) -> Vec<i64> {
    let scale = 1.0 / (HD as f64).sqrt();
    (0..heads)
        .flat_map(|h| {
            let scores: Vec<f64> = (lo..=n).map(|j| (0..HD).map(|c| query(r, h, c) * key(j, c)).sum::<f64>() * scale).collect();
            let m = scores.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let den: f64 = scores.iter().map(|s| (s - m).exp()).sum();
            (0..HD)
                .map(|c| {
                    let a: f64 = (lo..=n).zip(&scores).map(|(j, s)| (s - m).exp() / den * value(j, c)).sum();
                    (a * 1e9).round() as i64
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The first position of the chunk of `c` that position `n` sits in.
fn chunk_start(n: usize, c: usize) -> usize {
    (n - 1) / c * c + 1
}

#[test]
fn a_chunk_keeps_the_positions_of_the_querys_own_chunk() {
    // Position 5: alone in a chunk of 1, opening chunks of 2 and 4, second in a chunk of 3 (4 and 5), last in one of 5.
    for (c, kept) in [(1, 5..=5), (2, 5..=5), (3, 4..=5), (4, 5..=5), (5, 1..=5), (40, 1..=5)] {
        assert_eq!(chunk_start(5, c)..=5, kept, "the formula's chunk of {c}");
        let op = format!("attention of Q over K and V with 1 heads 1 groups up to pos within chunks of {c}");
        close(&shown(&program(&op, 1, 1)), &attend(1, 1, 5, *kept.start()), &format!("chunks of {c} at position 5"));
    }
    // Not a window: in chunks of 3 position 5 keeps 4 and 5, where a window of 3 keeps 3, 4 and 5.
    let chunked = shown(&program("attention of Q over K and V with 1 heads 1 groups up to pos within chunks of 3", 1, 1));
    let windowed = shown(&program("attention of Q over K and V with 1 heads 1 groups up to pos within a window of 3", 1, 1));
    assert!(chunked.iter().zip(&windowed).any(|(a, b)| (a - b).abs() > 1000), "a chunk keeps other positions than a window");
    close(
        &shown(&program("attention of Q over K and V with 2 heads 1 groups up to pos within chunks of 2", 1, 2)),
        &attend(1, 2, 5, 5),
        "every head of a group keeps the same chunk",
    );
    close(
        &shown(&program("attention of Q over K and V with 1 heads 1 groups within chunks of 4", 1, 1)),
        &attend(1, 1, L, chunk_start(L, 4)),
        "with no position the chunk is the cache's last row's",
    );
}

#[test]
fn each_row_of_a_causal_block_attends_its_own_chunk() {
    // Rows at 3, 4 and 5 in chunks of 2 keep [3], [3, 4] and [5]: the block crosses a chunk boundary.
    let got = shown(&program("causal attention of Q over K and V with 1 heads 1 groups from 3 within chunks of 2", 3, 1));
    let want: Vec<i64> = (1..=3).flat_map(|r| attend(r, 1, 3 + r - 1, chunk_start(3 + r - 1, 2))).collect();
    close(&got, &want, "rows at positions 3, 4 and 5, each over its own chunk of 2");
    let windowed: Vec<i64> = (1..=3).flat_map(|r| oracle(r, 1, 3 + r - 1, Some(2))).collect();
    assert_ne!(want, windowed, "the chunks bind otherwise than a window of 2 on these rows");
}

#[test]
fn the_compiled_tiers_attend_the_chunk() {
    let src = program("causal attention of Q over K and V with 2 heads 1 groups from 3 within chunks of 2", 3, 2);
    let want = vm_outcome(&src).output;
    common::assert_c_output(&src, want.trim());
    common::assert_exact_output(&src, want.trim());
}

#[test]
fn a_chunk_that_is_not_a_positive_whole_number_is_refused() {
    for (clause, why) in [
        ("within chunks of 0", "positive"),
        ("within chunks of -2", "positive"),
        ("within chunks of pos", "whole number"),
        ("within chunks of 2.5", "whole number"),
        ("within chunks", "chunks of"),
    ] {
        let op = format!("attention of Q over K and V with 1 heads 1 groups up to pos {clause}");
        let err = vm_outcome(&program(&op, 1, 1)).error.unwrap_or_else(|| panic!("`{clause}` is refused"));
        assert!(err.contains("chunks") && err.contains(why), "`{clause}` is refused naming the chunks and their {why}: {err}");
    }
}

/// The CUDA lane takes a chunked-local attention: every layer's attention launch keeps its own chunk of four rows inside
/// its kernel (the values: `cuda_lane_lock::a_chunked_local_attention_computes_what_its_reference_computes`).
#[test]
fn the_cuda_lane_takes_a_chunked_local_attention() {
    use logicaffeine_compile::codegen_cuda::lane::plan::Launch;
    let mut spec = Tiny::Llama.spec(16);
    for layer in &mut spec.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            a.window = Some(Window::Chunked(4));
        }
    }
    let path = std::env::temp_dir().join(format!("chunked-lane-{}.gguf", std::process::id()));
    write_checkpoint(&spec, &path);
    let opts = LaneOptions { ctx: 16, knobs: CudaKnobs::default(), prefill: 1 };
    let lane = compile(&spec, &path, &opts).unwrap_or_else(|e| panic!("the lane takes a chunked-local attention: {e}"));
    let _ = std::fs::remove_file(&path);
    let attention: Vec<(Option<usize>, bool)> =
        lane.decode().schedule.launches.iter().filter_map(|l| if let Launch::Attention { window, chunked, .. } = l { Some((*window, *chunked)) } else { None }).collect();
    assert_eq!(attention, vec![(Some(4), true); spec.layers.len()], "every layer keeps its own chunk of four");
}
