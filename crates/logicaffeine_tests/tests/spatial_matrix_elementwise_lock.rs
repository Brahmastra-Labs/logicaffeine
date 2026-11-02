//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! A POINTWISE OP OVER A MATRIX IS A MATRIX — AND COMPOSES WITH WHAT READS IT AS ONE.
//!
//! A pointwise map (`sigmoid`, `silu`, `gelu`, `plus`, `hadamard`) has no cross-element dependence, so it can be
//! lowered in any tiling of its operand. Lowered as a flat `1 × n` row it tiles its operand into one-row strips;
//! a matmul reads the same tensor as `r × c` in `32 × 32` blocks. Those are different tensors to the IR, and a
//! composition that joins them is refused `TensorShapeConflict`. So a pointwise section declares its tensors in
//! the shape the PROGRAM gives them — the parser's own table, read by the walk — and a matrix is mapped tile by
//! tile over its 2-D tile grid. A `1 × n` row keeps the row form every activation of the forward uses.
//!
//! Pinned against an f64 reference with the campaign's CHIP rule — bf16 within (sections + 1) ulps of Σ|terms| —
//! in both directions: a matrix pointwise op feeding a matmul, and a matmul feeding a matrix pointwise op.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_matrix_elementwise_lock`
//!
//! ★ MUTATION PROBE (run when this landed): make `RecognizedProgram::shape_of` ignore the parser's table (always
//! `1 × n`) — both matrix tests red on `TensorShapeConflict { name: "Y", first: (1, 6144, …), second: (64, 96, …) }`
//! while the row test stays green: the row form was never the problem.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::ir::SpatialProgram;
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;

#[path = "common/spatial.rs"]
mod spatial_support;
use spatial_support::inputs_from_fills;

const R: usize = 64;
const K: usize = 96;
const N: usize = 32;

/// Bf16-exact fills.
fn x_at(i: usize) -> f64 {
    (((i * 37) % 23) as f64 - 11.0) / 64.0
}
fn w_at(i: usize) -> f64 {
    (((i * 11) % 19) as f64 - 9.0) / 64.0
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

fn fills(name: &str, n: usize, at: fn(usize) -> f64) -> String {
    (1..=n)
        .map(|i| format!("Set item {i} of {name} to {:?}.\n", at(i)))
        .collect()
}

fn composed(src: &str) -> SpatialProgram {
    let l = compile_program_to_spatial(src, &RduChip::Sn50.facts().spatial_target())
        .expect("a tensor program");
    l.program
        .unwrap_or_else(|| panic!("the program composes and certifies: {:?}", l.missing))
}

fn result(p: &SpatialProgram, name: &str) -> Vec<f64> {
    let out = eval_choreography(p, &inputs_from_fills(p)).expect("the composed program evaluates");
    let id = p
        .tensors
        .iter()
        .find(|t| t.name == name)
        .expect("the result")
        .id;
    match out.tensors.get(&id).expect("written") {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    }
}

/// The CHIP rule: every element within (sections + 1) bf16 ulps of the magnitude of the terms it sums.
fn within(got: &[f64], want: &[(f64, f64)], sections: usize, what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: one value per element");
    let ulp = 2f64.powi(-8);
    for (i, (g, (w, mag))) in got.iter().zip(want).enumerate() {
        let bound = (sections as f64 + 1.0) * ulp * mag.max(f64::MIN_POSITIVE);
        assert!(
            (g - w).abs() <= bound,
            "{what}[{i}]: {g} vs {w} (bound {bound})"
        );
    }
}

/// ── A MATRIX POINTWISE OP FEEDING A MATMUL: `Y = sigmoid(X)` read by `Z = Y · W`.
#[test]
fn a_matrix_pointwise_op_feeds_a_matmul() {
    let src = format!(
        "## Main\nLet X be a {R} by {K} tensor of Real.\nLet W be a {K} by {N} tensor of Real.\n{}{}Let Y be sigmoid of X.\nLet Z be Y matmul W.\n",
        fills("X", R * K, x_at),
        fills("W", K * N, w_at)
    );
    let p = composed(&src);
    let y = p.tensors.iter().find(|t| t.name == "Y").expect("Y");
    assert_eq!(
        (y.elems.rows, y.elems.cols),
        (R, K),
        "Y is declared in the program's shape"
    );
    let want: Vec<(f64, f64)> = (0..R * N)
        .map(|e| {
            let (r, c) = (e / N, e % N);
            (0..K).fold((0.0, 0.0), |(s, m), j| {
                let t = sigmoid(x_at(r * K + j + 1)) * w_at(j * N + c + 1);
                (s + t, m + t.abs())
            })
        })
        .collect();
    within(&result(&p, "Z"), &want, p.sections.len(), "Z");
}

/// ── A MATMUL FEEDING A MATRIX POINTWISE OP: `Z = X · W` read by `S = sigmoid(Z)`, then `T = S hadamard S`.
#[test]
fn a_matmul_feeds_matrix_pointwise_ops() {
    let src = format!(
        "## Main\nLet X be a {R} by {K} tensor of Real.\nLet W be a {K} by {N} tensor of Real.\n{}{}Let Z be X matmul W.\nLet S be sigmoid of Z.\nLet T be S hadamard S.\n",
        fills("X", R * K, x_at),
        fills("W", K * N, w_at)
    );
    let p = composed(&src);
    for name in ["Z", "S", "T"] {
        let t = p.tensors.iter().find(|t| t.name == name).expect("declared");
        assert_eq!(
            (t.elems.rows, t.elems.cols),
            (R, N),
            "{name} is declared in the program's shape"
        );
    }
    // Each element of T is s·s with s = sigmoid(z): the error of z (Σ|x·w| ulps) moves s by at most a quarter
    // of it (|σ'| ≤ 1/4), and T by twice that — within the rule's allowance at the magnitude of z.
    let want: Vec<(f64, f64)> = (0..R * N)
        .map(|e| {
            let (r, c) = (e / N, e % N);
            let (z, mag) = (0..K).fold((0.0, 0.0), |(s, m), j| {
                let t = x_at(r * K + j + 1) * w_at(j * N + c + 1);
                (s + t, m + t.abs())
            });
            let s = sigmoid(z);
            (s * s, mag.max(s * s))
        })
        .collect();
    within(&result(&p, "T"), &want, p.sections.len(), "T");
}

/// ── A ROW KEEPS THE ROW FORM. `1 × n` is the shape of every activation in the forward; its tiling is unchanged.
#[test]
fn a_row_pointwise_op_keeps_the_row_form() {
    let src = format!(
        "## Main\nLet X be a 1 by {K} tensor of Real.\n{}Let Y be sigmoid of X.\n",
        fills("X", K, x_at)
    );
    let p = composed(&src);
    let y = p.tensors.iter().find(|t| t.name == "Y").expect("Y");
    assert_eq!((y.elems.rows, y.elems.cols), (1, K));
    let want: Vec<(f64, f64)> = (0..K).map(|j| (sigmoid(x_at(j + 1)), 1.0)).collect();
    within(&result(&p, "Y"), &want, p.sections.len(), "Y");
}

/// ── A BATCHED FFN: `G = X·Wg`, `U = X·Wu`, `H = swiglu(G, U)`, `O = H·Wd` — the feed-forward block over `R` rows at
/// once, which is prefill's shape. SwiGLU is pointwise, so it maps in the matmuls' tiling.
#[test]
fn a_matrix_swiglu_sits_between_matmuls() {
    const F: usize = 64;
    let wg = |i: usize| (((i * 13) % 17) as f64 - 8.0) / 64.0;
    let wu = |i: usize| (((i * 5) % 21) as f64 - 10.0) / 64.0;
    let wd = |i: usize| (((i * 3) % 11) as f64 - 5.0) / 64.0;
    let src = format!(
        "## Main\nLet X be a {R} by {K} tensor of Real.\nLet Wg be a {K} by {F} tensor of Real.\nLet Wu be a {K} by {F} tensor of Real.\nLet Wd be a {F} by {N} tensor of Real.\n\
         {}{}{}{}Let G be X matmul Wg.\nLet U be X matmul Wu.\nLet H be swiglu of G and U.\nLet O be H matmul Wd.\n",
        fills("X", R * K, x_at),
        fills("Wg", K * F, wg),
        fills("Wu", K * F, wu),
        fills("Wd", F * N, wd)
    );
    let p = composed(&src);
    let h = p.tensors.iter().find(|t| t.name == "H").expect("H");
    assert_eq!(
        (h.elems.rows, h.elems.cols),
        (R, F),
        "H is declared in the program's shape"
    );
    // The reference, and a magnitude that carries every stage's error forward: |o| ≤ Σ |silu(g)·u|·|wd|, each g and
    // u good to its own Σ|x·w|.
    let want: Vec<(f64, f64)> = (0..R * N)
        .map(|e| {
            let (r, c) = (e / N, e % N);
            (0..F).fold((0.0, 0.0), |(s, m), f| {
                let dot = |w: &dyn Fn(usize) -> f64| {
                    (0..K).fold((0.0, 0.0), |(a, b), j| {
                        let t = x_at(r * K + j + 1) * w(j * F + f + 1);
                        (a + t, b + t.abs())
                    })
                };
                let ((g, gm), (u, um)) = (dot(&wg), dot(&wu));
                let hv = g * sigmoid(g) * u;
                let hm = gm.max(g.abs()) * um.max(u.abs());
                let d = wd(f * N + c + 1);
                (s + hv * d, m + (hm + hv.abs()) * d.abs())
            })
        })
        .collect();
    within(&result(&p, "O"), &want, p.sections.len(), "O");
}
