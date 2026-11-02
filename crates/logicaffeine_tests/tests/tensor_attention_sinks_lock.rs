//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! AN ATTENTION'S SINKS JOIN ITS SOFTMAX AND TAKE NO VALUE.
//!
//! gpt-oss gives each attention head a learned sink: a logit that joins the softmax's maximum and its denominator and
//! contributes no value, so a head can attend to nothing (ggml's `soft_max_ext` with sinks). The sugar takes
//! `with sinks S` last — `S` one row of a logit per query head — on every attention form, windowed or not.
//!
//!   • the VM, the tree-walker, the C backend and AOT-Rust compute the `f64` formula, on inputs where the sinks move the
//!     answer;
//!   • every head reads its own sink, each row of a causal block too;
//!   • the tensor front end lifts the sinks into the op's spec (`AttentionSpec::sinks`) and keeps `Q, K, V` its operands;
//!   • sinks that are not one row of a logit per head are refused by name;
//!   • the accelerator paths with no sink in their kernels refuse the op by name.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;

use logicaffeine_compile::compile::{compile_to_cuda_forward, tw_outcome, vm_outcome};
use logicaffeine_compile::spatial::passes::recognize_stages::{recognize_stages, TensorOpSpec};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::tensor_recognize::qwen::Window;

const L: usize = 6;
const HD: usize = 2;

fn key(j: usize, c: usize) -> f64 {
    [0.3 * j as f64 - 1.0, 0.2 * (j % 3) as f64 - 0.1][c]
}

fn value(j: usize, c: usize) -> f64 {
    [0.5 * j as f64 - 1.0, 0.25 * (7 - j) as f64][c]
}

fn query(r: usize, head: usize, c: usize) -> f64 {
    [0.8, -0.6][c] + 0.15 * r as f64 - 0.35 * head as f64
}

/// Head `h`'s sink: large enough to take a visible share of the softmax.
fn sink(h: usize) -> f64 {
    [0.9, -0.4][h]
}

/// `Q` (`rows` by `heads·2`), `K` and `V` (6 by 2) and `S` (1 by `heads`) filled with `Real` literals, `C` bound by
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
    s.push_str(&format!("Let S be a 1 by {heads} tensor of Real.\n"));
    for h in 0..heads {
        s.push_str(&format!("Set item {} of S to {:?}.\n", h + 1, sink(h)));
    }
    s.push_str(&format!("Let pos be 5.\nLet C be {op}.\n"));
    for i in 1..=rows * heads * HD {
        s.push_str(&format!("Show round(item {i} of C * 1000000000).\n"));
    }
    s
}

/// The `f64` attention of query row `r` at position `n` over `max(1, n − w + 1) ..= n`, each head's sink in its
/// softmax when `sinks`.
fn oracle(r: usize, heads: usize, n: usize, w: Option<usize>, sinks: bool) -> Vec<i64> {
    let lo = w.map_or(1, |w| n.saturating_sub(w - 1).max(1));
    let scale = 1.0 / (HD as f64).sqrt();
    (0..heads)
        .flat_map(|h| {
            let scores: Vec<f64> = (lo..=n).map(|j| (0..HD).map(|c| query(r, h, c) * key(j, c)).sum::<f64>() * scale).collect();
            let m = scores.iter().copied().fold(if sinks { sink(h) } else { f64::NEG_INFINITY }, f64::max);
            let den: f64 = scores.iter().map(|s| (s - m).exp()).sum::<f64>() + if sinks { (sink(h) - m).exp() } else { 0.0 };
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

fn close(got: &[i64], want: &[i64], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: {got:?} vs {want:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g - w).abs() <= 1, "{what}: {got:?} vs the formula's {want:?}");
    }
}

#[test]
fn a_sink_joins_each_heads_softmax() {
    let got = shown(&program("attention of Q over K and V with 2 heads 1 groups up to pos with sinks S", 1, 2));
    close(&got, &oracle(1, 2, 5, None, true), "two heads, each with its own sink");
    let without = oracle(1, 2, 5, None, false);
    assert!(got.iter().zip(&without).any(|(a, b)| (a - b).abs() > 1000), "the sinks move the answer on these inputs");
}

#[test]
fn sinks_compose_with_a_window_and_a_causal_block() {
    close(
        &shown(&program("attention of Q over K and V with 2 heads 1 groups up to pos within a window of 3 with sinks S", 1, 2)),
        &oracle(1, 2, 5, Some(3), true),
        "a windowed attention with sinks",
    );
    let got = shown(&program(
        "causal attention of Q over K and V with 1 heads 1 groups from 3 within a window of 2 with sinks S",
        3,
        1,
    ));
    let want: Vec<i64> = (1..=3).flat_map(|r| oracle(r, 1, 3 + r - 1, Some(2), true)).collect();
    close(&got, &want, "each row of a causal block over its window, with its head's sink");
}

#[test]
fn the_compiled_tiers_take_the_sinks() {
    let src = program("causal attention of Q over K and V with 2 heads 1 groups from 3 with sinks S", 3, 2);
    let want = vm_outcome(&src).output;
    common::assert_c_output(&src, want.trim());
    common::assert_exact_output(&src, want.trim());
}

#[test]
fn the_front_end_lifts_the_sinks_into_the_spec() {
    let lifted = |op: &str, rows: usize| {
        let src = program(op, rows, 2);
        let rp = recognize_stages(&src).expect("parses");
        assert!(rp.unrecognized.is_empty(), "{op}: {:?}", rp.unrecognized);
        let st = rp.stages.into_iter().find(|s| matches!(s.op, TensorOpSpec::Attention(_))).expect("an attention");
        let TensorOpSpec::Attention(a) = st.op else { unreachable!() };
        (a, st.ins)
    };
    let (a, ins) = lifted("attention of Q over K and V with 2 heads 1 groups up to pos with sinks S", 1);
    assert!(a.sinks.is_some(), "the sinks are lifted");
    assert_eq!(&ins[..3], ["Q", "K", "V"], "Q, K and V stay the operands, in order: {ins:?}");
    assert!(ins.contains(&"S".to_string()), "the sinks are read: {ins:?}");
    let (b, _) = lifted("attention of Q over K and V with 2 heads 1 groups up to pos", 1);
    assert!(b.sinks.is_none(), "no sinks without the clause");
    let (c, _) = lifted("causal attention of Q over K and V with 2 heads 1 groups from 3 within a window of 2 with sinks S", 3);
    assert_eq!((c.window, c.sinks.is_some(), c.rows), (Some(Window::Sliding(2)), true, 3), "a causal block with a window and sinks");
}

#[test]
fn sinks_that_are_not_a_logit_per_head_are_refused() {
    for (decl, why) in [("Let T be a 1 by 3 tensor of Real.", "1×2"), ("Let T be a 2 by 2 tensor of Real.", "1×2")] {
        let src = program("attention of Q over K and V with 2 heads 1 groups up to pos", 1, 2)
            .replace("Let pos be 5.", &format!("Let pos be 5.\n{decl}"))
            .replace("groups up to pos.", "groups up to pos with sinks T.");
        let err = vm_outcome(&src).error.unwrap_or_else(|| panic!("`{decl}` as sinks is refused"));
        assert!(err.contains("sinks") && err.contains(why), "the refusal names the sinks' shape: {err}");
    }
    let err = vm_outcome(&program("attention of Q over K and V with 2 heads 1 groups up to pos with sinks", 1, 2))
        .error
        .expect("a missing sinks tensor is refused");
    assert!(err.contains("sinks"), "{err}");
}

#[test]
fn an_accelerator_with_no_sinks_refuses_them_by_name() {
    let src = program("attention of Q over K and V with 2 heads 1 groups with sinks S", 1, 2);
    let spatial = format!("{:?}", compile_program_to_spatial(&src, &SpatialTarget::wormhole()));
    assert!(spatial.contains("sink"), "the spatial IR refuses sinks by name:\n{spatial}");
    let forward = compile_to_cuda_forward(&src).expect_err("the CUDA forward has no sinks");
    assert!(forward.contains("sink"), "the CUDA forward names the sinks it refuses: {forward}");
}
