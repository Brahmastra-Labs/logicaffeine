//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! ALGEBRA LOCK — the spatial compiler computes THE MATHEMATICS, not merely what it computed before.
//!
//! Every other spatial semantics lock is a *differential*: choreography ≡ VM, tier ≡ tier, printer ≡
//! golden. Differentials are load-bearing, but they share a blind spot the size of the compiler — two
//! sides of a differential are written by the same hands, from the same idea of what RMSNorm is. If
//! that idea is wrong (the mean taken over the padded tile width instead of the true column count, a
//! weight broadcast down rows instead of across columns, a residual added *after* the reduction it is
//! supposed to be inside), every differential in the suite agrees, loudly, on the wrong answer.
//!
//! ★ SO THIS FILE NEVER ASKS THE COMPILER WHAT THE ANSWER IS. For each recognized workload it emits a
//! LOGOS source program, compiles it with `compile_to_spatial`, evaluates the resulting choreography,
//! and compares against a reference written out longhand HERE from the operator's mathematical
//! definition — no `vm_outcome`, no `eval_*` helper, no `tensor_recognize` constant, nothing the
//! compiler could be wrong about in the same direction twice. The references below are the ONLY place
//! this file states what an operator means, and each one is a transcription of the definition quoted
//! in its doc comment. That is the whole point: keep them independent, or the lock evaporates.
//!
//! ★ AND WHAT IT DOES NOT CLAIM. "The machine" here is `eval_choreography` — the spatial IR's own
//! executable meaning, the golden oracle the Tensix / CUDA / fabric printers are each proven against
//! in their own locks. So this file pins THE ORACLE: it says the choreography a LOGOS normalization
//! compiles to computes the normalization. It says nothing about whether an emitted kernel matches
//! the choreography; that differential is those locks' job, and it is worth exactly as much as the
//! oracle it points at — which is why this one exists.
//!
//! Covered, with the shapes randomized over the repo's own seeded PRNG (`tests/common/spatial.rs`;
//! proptest is not a dependency and is not wanted):
//!   · elementwise  `C = A plus B`            — integer and real operands
//!   · SwiGLU       `Y = silu(G) · U`
//!   · RMSNorm      `Y = X / sqrt(mean X² + ε) · W`   (per row, weighted)
//!   · softmax      `Y = e^(X−max) / Σ e^(X−max)`
//!   · LayerNorm    `Y = (X − mean)/sqrt(var + ε) · W + B`  (per row)
//!
//! ★ WHAT THE SHAPES CAN AND CANNOT SAY. RMSNorm and LayerNorm are exercised at 2 and 3 rows, so a
//! reduction that leaked across rows fails here. Softmax is exercised at ONE row only, and that is a
//! property of the language rather than a gap in this file: `desugar_tensor_softmax` normalizes over
//! the tensor's whole `rows · cols` extent, and `recognize_fused_row` lifts exactly that back as
//! `rows = 1, cols = R·C`. A per-row softmax is not expressible at the LOGOS surface, so no test here
//! can pin one — say so rather than let `softmax 1×n` imply coverage it does not have.
//!
//! ★ THE DATA IS PART OF THE SPEC, NOT DECORATION. Column counts of 3, 7, 17 and 24 sit well inside a
//! 16×16 or 32×32 native tile, so every reduction here runs over a row that is mostly PADDING — which
//! is the point, because "reduced over the padded tile width instead of the true column count" is the
//! first error class named above. Two generators exist because two operators need opposite data to
//! see it: `spread_rows` keeps the normalizations' rows far from constant (so ε cannot masquerade as
//! algebra), and `logits` keeps softmax's inputs NARROW (so a zero-padding column is worth `e^-3` of
//! the denominator rather than `e^-14`). Both carry the arithmetic in their doc comments. Widening
//! softmax's range, or flattening a normalization's rows, silently un-tests the padding.
//!
//! Every preset target is tried for every case. A target that cannot host a workload must say so with
//! a TYPED, NAMED refusal (`MissingCompute { needed, .. }` and friends) whose claim is CHECKED against
//! the descriptor — never a skip, never a silent pass. `assert_workload` additionally refuses to be
//! vacuous PER CASE: every single case must be computed by at least one preset, so a workload cannot
//! quietly decay into "one shape still computes somewhere and the rest refuse".
//!
//! ★ NON-VACUITY IS DEMONSTRATED, NOT ASSERTED. `a_perturbed_reference_is_caught_by_a_wide_margin`
//! takes five hand-computed cases and, for each, builds a DELIBERATELY WRONG reference:
//!   · elementwise — the product `A·B` where the algebra says the sum `A+B`
//!   · SwiGLU      — operands swapped, `silu(U)·G` instead of `silu(G)·U`
//!   · RMSNorm     — the SUM of squares where the algebra says their MEAN (a √C denominator error)
//!   · softmax     — temperature 2, `e^(2X)/Σ e^(2X)`
//!   · LayerNorm   — the SAMPLE variance (÷ C−1) where the algebra says the POPULATION one (÷ C)
//! and asserts, per (case, preset), that the machine's answer sits within tolerance of the TRUE
//! reference and clear of the perturbed one by the floor `margin_floor` states for that datapath's
//! precision. It runs on EVERY preset, not one, because a tolerance is only justified where it is
//! demonstrated: fp32 clears its floor by four to five orders of magnitude, and bf16 — the loosest
//! number in `rel_tol`, and the one the RDU actually uses — clears it by as little as 3×. That
//! asymmetry is the fact this test exists to keep visible. Loosening a tolerance without re-reading
//! those margins is the failure mode it prevents — DO NOT widen `rel_tol` to make something pass.
//!
//! ★ ON ε. The stability epsilon inside `sqrt(· + ε)` is a numerical constant of the operator, not
//! part of its algebra, and this lock deliberately does not freeze one. LOGOS declares ε = 1e-6 for
//! both RMSNorm and LayerNorm (`parser/mod.rs::desugar_tensor_rmsnorm_weighted` /
//! `::desugar_tensor_layernorm`), and the spatial RMSNorm stage chain carries the same value as
//! `RowXformKind::Rsqrt { eps_nano: 1_000 }`; the fused `ComputeOp::RowReduce` arm in
//! `spatial/eval.rs` uses 1e-5 for LayerNorm. So that a disagreement over a constant can never be
//! mistaken for a disagreement over the mathematics, `spread_rows` rejects any row whose variance and
//! mean-square are below `MIN_ROW_SPREAD`, which bounds the difference between those two epsilons at
//! ≈2.8e-7 relative — a thirty-sixth of the fp32 tolerance. Unifying the constant is a real (and
//! separate) job; this file pins the algebra it does not own the constant of.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(spatial_algebra_semantics_lock)'`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use std::collections::BTreeSet;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::ir::{Phase, SpatialProgram, TensorRole};
use logicaffeine_compile::spatial::passes::SpatialReject;
use logicaffeine_compile::spatial::pipeline::compile_to_spatial;
use logicaffeine_compile::spatial::target::{precision_is_int, ComputeCap, SpatialTarget};
use spatial_support::{inputs_from_fills, output_id, prng};

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// The mathematics, written out longhand
//
// Nothing in this section may call into the compiler. Each function is a transcription of the formula
// in its own doc comment and of nothing else — if a definition changes, the transcription changes,
// and the diff makes the change visible as a change of MEANING rather than of implementation.
// ══════════════════════════════════════════════════════════════════════════════════════════════════

/// The stability ε both normalizations declare at the LOGOS surface (`parser/mod.rs`, eps = 1e-6). See
/// the "★ ON ε" note in the header for why the references take it as a parameter rather than a law.
const SURFACE_EPS: f64 = 1.0e-6;

/// A row is only used as normalization input once its variance AND its mean-square clear this. It puts
/// the radicands of `sqrt(var + ε)` / `sqrt(mean X² + ε)` six orders of magnitude above the largest ε
/// the repo declares (1e-5), so swapping one declared ε for the other moves the denominator by at most
/// `9e-6 / (2·16) ≈ 2.8e-7` relative — a thirty-sixth of the fp32 tolerance. The reference therefore
/// cannot be sensitive to which constant the datapath used, only to the algebra around it.
const MIN_ROW_SPREAD: f64 = 16.0;

/// `C[p] = A[p] + B[p]` — the elementwise sum.
fn ref_add(a: &[f64], b: &[f64]) -> Vec<f64> {
    (0..a.len()).map(|p| a[p] + b[p]).collect()
}

/// SwiGLU: `Y[p] = silu(G[p]) · U[p]`, where `silu(g) = g / (1 + e^(−g))` — the gated MLP activation.
fn ref_swiglu(g: &[f64], u: &[f64]) -> Vec<f64> {
    (0..g.len())
        .map(|p| (g[p] / (1.0 + (-g[p]).exp())) * u[p])
        .collect()
}

/// Weighted RMSNorm, per row: `Y[r,i] = X[r,i] / sqrt( (1/C)·Σ_j X[r,j]² + ε ) · W[i]`.
///
/// The reduction runs over the row's TRUE `cols` entries — a padded tile width would divide by the
/// wrong count, which is precisely the class of error a differential against the same compiler cannot
/// see. Note the shape too: one scalar per ROW, one weight per COLUMN.
fn ref_rmsnorm(x: &[f64], w: &[f64], rows: usize, cols: usize, eps: f64) -> Vec<f64> {
    let mut y = vec![0.0f64; rows * cols];
    for r in 0..rows {
        let mut sum_sq = 0.0f64;
        for j in 0..cols {
            let v = x[r * cols + j];
            sum_sq += v * v;
        }
        let mean_sq = sum_sq / cols as f64;
        let denom = (mean_sq + eps).sqrt();
        for i in 0..cols {
            y[r * cols + i] = x[r * cols + i] / denom * w[i];
        }
    }
    y
}

/// Softmax over one row: `Y[i] = e^(X[i] − max_j X[j]) / Σ_j e^(X[j] − max_k X[k])`.
///
/// The max subtraction is a numerical-stability shift that cancels exactly in exact arithmetic, so it
/// is written here for the same reason the operator writes it — it is not what the test detects.
fn ref_softmax(x: &[f64]) -> Vec<f64> {
    let mut max = f64::NEG_INFINITY;
    for v in x {
        if *v > max {
            max = *v;
        }
    }
    let exps: Vec<f64> = x.iter().map(|v| (v - max).exp()).collect();
    let total: f64 = exps.iter().sum();
    exps.iter().map(|e| e / total).collect()
}

/// LayerNorm, per row: `Y[r,i] = (X[r,i] − mean_r) / sqrt(var_r + ε) · W[i] + B[i]`, where `mean_r` and
/// `var_r` are the row's mean and its POPULATION variance (`Σ (x − mean)² / C`, not `/ (C−1)`).
fn ref_layernorm(x: &[f64], w: &[f64], b: &[f64], rows: usize, cols: usize, eps: f64) -> Vec<f64> {
    let mut y = vec![0.0f64; rows * cols];
    for r in 0..rows {
        let mut mean = 0.0f64;
        for j in 0..cols {
            mean += x[r * cols + j];
        }
        mean /= cols as f64;
        let mut var = 0.0f64;
        for j in 0..cols {
            let d = x[r * cols + j] - mean;
            var += d * d;
        }
        var /= cols as f64;
        let denom = (var + eps).sqrt();
        for i in 0..cols {
            y[r * cols + i] = (x[r * cols + i] - mean) / denom * w[i] + b[i];
        }
    }
    y
}

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// Data — seeded, and exactly representable in every float datapath under test
// ══════════════════════════════════════════════════════════════════════════════════════════════════

/// One pseudo-random value, a multiple of ¼ with magnitude below 16.
///
/// ★ SIX SIGNIFICANT BITS, SO EXACT IN bf16, fp16 AND fp32 ALIKE. A datapath rounds its inputs on the
/// way into the tiles; if the generated data were not exactly representable, the reference and the
/// machine would already disagree before the first arithmetic operation and the tolerance below would
/// be silently absorbing an input mismatch instead of measuring the compute. Keep the ¼ grid.
fn quarter(seed: u64, i: usize) -> f64 {
    let k = prng(seed, i, -63, 63);
    assert!(
        k.abs() <= 63,
        "the PRNG must stay inside the ¼-grid range that keeps values bf16-exact"
    );
    k as f64 / 4.0
}

/// `n` seeded ¼-grid values.
fn quarters(seed: u64, n: usize) -> Vec<f64> {
    (0..n).map(|i| quarter(seed, i)).collect()
}

/// `n` seeded small integers — the operands of the integer elementwise case.
fn small_ints(seed: u64, n: usize) -> Vec<i64> {
    (0..n).map(|i| prng(seed, i, -9, 9)).collect()
}

/// `n` seeded ¼-grid logits inside ±3 — softmax's operands, and NARROW ON PURPOSE.
///
/// ★ WIDE LOGITS MAKE THE PADDING INVISIBLE, AND THE PADDING IS THE POINT. Softmax's answer is
/// `e^(X−max) / Σ e^(X−max)`, so a zero-padding column of the tile contributes `e^(0−max)` to the
/// denominator. At the ±15.75 range `quarters` draws, a row's max is around 14 and a padding column
/// is worth ~1e-6 of the total: a reduction that ran over the tile's PADDED WIDTH instead of the
/// row's true `cols` — the very first error class this file's header names — would then move every
/// output by less than the fp32 tolerance, and this test would wave it through. (Worked through on
/// the seeds below: 0.17× to 0.99× the tolerance. It would have passed.) Inside ±3 a padding column
/// is worth at least `e^-3` of the denominator, and the same mutation moves the answer by 5·10³ to
/// 7·10⁴ tolerances. Keep the range narrow; `quarters` is the wrong generator here.
///
/// The values stay on the ¼ grid, so they remain exactly representable in every float datapath.
fn logits(seed: u64, n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| {
            let k = prng(seed, i, -12, 12);
            assert!(k.abs() <= 12, "softmax logits must stay inside ±3 so a padding column is not worth 1e-6 of the denominator");
            k as f64 / 4.0
        })
        .collect()
}

/// A `rows × cols` normalization input whose every row clears [`MIN_ROW_SPREAD`] in both variance and
/// mean-square, re-drawing the whole tensor from a shifted seed until it does.
///
/// ★ THIS IS NOT DATA MASSAGE, IT IS SCOPE. A row that is almost constant makes `sqrt(var + ε)` a
/// measurement of ε rather than of var, and ε is a constant this lock does not own (header note). The
/// spread requirement bounds the two declared epsilons' disagreement at ≈2.8e-7 relative, thirty-six
/// times under the fp32 tolerance, so a failure here is always a failure of the ALGEBRA.
fn spread_rows(seed: u64, rows: usize, cols: usize) -> Vec<f64> {
    for attempt in 0..64u64 {
        let s = seed.wrapping_add(attempt.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let x: Vec<f64> = (0..rows * cols).map(|i| quarter(s, i)).collect();
        let ok = (0..rows).all(|r| {
            let row = &x[r * cols..(r + 1) * cols];
            let mean = row.iter().sum::<f64>() / cols as f64;
            let var = row.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / cols as f64;
            let mean_sq = row.iter().map(|v| v * v).sum::<f64>() / cols as f64;
            var >= MIN_ROW_SPREAD && mean_sq >= MIN_ROW_SPREAD
        });
        if ok {
            return x;
        }
    }
    panic!("no {rows}×{cols} draw from seed {seed:#x} cleared MIN_ROW_SPREAD in 64 attempts — widen the PRNG range, never the spread floor");
}

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// LOGOS sources
// ══════════════════════════════════════════════════════════════════════════════════════════════════

/// A float literal whose text denotes EXACTLY the `f64` the reference will use.
///
/// ★ THE ROUND-TRIP IS ASSERTED, not assumed. If the source text and the reference disagreed by one
/// ulp before compilation even began, this file would be measuring its own formatter.
fn lit(v: f64) -> String {
    let s = format!("{v:?}");
    assert!(
        s.parse::<f64>().expect("a formatted f64 re-parses").to_bits() == v.to_bits(),
        "`{s}` does not denote the exact f64 the reference uses — fix the formatter here, not the tolerance"
    );
    s
}

/// `Let <name> be a R by C tensor of Real.` followed by one `Set item` per element.
fn real_tensor(name: &str, rows: usize, cols: usize, vals: &[f64]) -> String {
    assert_eq!(
        vals.len(),
        rows * cols,
        "{name}: a {rows}×{cols} tensor needs {} values",
        rows * cols
    );
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of Real.\n");
    for (i, v) in vals.iter().enumerate() {
        s.push_str(&format!("Set item {} of {name} to {}.\n", i + 1, lit(*v)));
    }
    s
}

/// `Let <name> be a R by C tensor of Int.` followed by one `Set item` per element.
fn int_tensor(name: &str, rows: usize, cols: usize, vals: &[i64]) -> String {
    assert_eq!(
        vals.len(),
        rows * cols,
        "{name}: a {rows}×{cols} tensor needs {} values",
        rows * cols
    );
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of Int.\n");
    for (i, v) in vals.iter().enumerate() {
        s.push_str(&format!("Set item {} of {name} to {v}.\n", i + 1));
    }
    s
}

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// Comparison
// ══════════════════════════════════════════════════════════════════════════════════════════════════

/// The relative tolerance a float datapath of precision `p` earns, as a multiple of its unit roundoff.
///
/// ★ WHY A TOLERANCE AT ALL, AND WHY THESE NUMBERS. The evaluator carries a stage's arithmetic in f64
/// and rounds ONCE PER STAGE BOUNDARY — at the load into tiles, and again at every accumulator pack
/// into the destination buffer's precision. The RMSNorm chain is the longest: square → reduce → rsqrt
/// → rescale → weight, five pack points, with the reduction carrying a squared operand's error, its
/// `1/cols` scaler rounded to the same precision, and the rsqrt halving what reaches it. The
/// references are exact f64, so the difference is bounded by a small multiple of the datapath's unit
/// roundoff `u`:
///   fp32 `u = 2⁻²⁴ ≈ 6e-8`  → 1e-5   is ≈ 168 u  (worst of this file's cases, worked through by hand: 2.5% of it)
///   bf16 `u = 2⁻⁹  ≈ 2e-3`  → 3.2e-2 is ≈  16 u  (worst worked through: 15% of it — the RMSNorm chain on the RDU)
/// ★ EVERY PRECISION NAMED HERE MUST BE DEMONSTRATED. A tolerance nothing probes is an unfalsifiable
/// claim, so `a_perturbed_reference_is_caught_by_a_wide_margin` asserts that the set of precisions it
/// actually exercised is exactly the set this function answers for. fp16 is deliberately ABSENT: no
/// preset selects it (`from_tensor_op::float_precision` prefers fp32, then bf16), so a number for it
/// could never be shown to have teeth. Adding an arm here means adding its demonstration there.
/// ★ DO NOT WIDEN AN ENTRY TO MAKE A FAILURE GO AWAY — a failure here means the arithmetic changed,
/// and the margin test is what proves these numbers still separate a right answer from a wrong one.
fn rel_tol(p: Precision) -> f64 {
    match p {
        Precision::Fp32 => 1.0e-5,
        Precision::Bf16 => 3.2e-2,
        other => panic!(
            "no tolerance is stated for a {other:?} float datapath. State one HERE with its unit \
             roundoff, and add a preset that exercises it to the margin demonstration — never reuse \
             a looser neighbour's number, and never state one you cannot demonstrate."
        ),
    }
}

/// The absolute tolerance for one answer: relative to the answer's own magnitude.
///
/// A normalization's error is governed by the row's scale, not by the individual element — an output
/// near zero is a cancellation, and demanding relative precision of a cancellation would be a demand
/// no datapath can meet and no mathematics implies.
fn tolerance(dtype: Precision, want: &[f64]) -> f64 {
    let scale = want.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    assert!(
        scale > 0.0,
        "the reference answer is identically zero, so this case would pass for any implementation — \
         generate data that exercises a non-zero answer"
    );
    rel_tol(dtype) * scale
}

/// Assert the machine's answer IS the mathematics, at `dtype`'s resolution.
fn assert_is_the_mathematics(got: &[f64], want: &[f64], dtype: Precision, label: &str) {
    assert_eq!(
        got.len(),
        want.len(),
        "{label}: the output has {} elements, the mathematics has {}",
        got.len(),
        want.len()
    );
    if precision_is_int(dtype) {
        // ★ AN INTEGER DATAPATH OWES EXACTNESS. There is no rounding to forgive, so forgiving any
        // would be inventing slack the hardware never needed.
        for (i, (g, w)) in got.iter().zip(want.iter()).enumerate() {
            assert!(
                (w - w.round()).abs() < 1e-12,
                "{label}: reference element {i} is {w}, not an integer — an integer datapath cannot be asked to produce it"
            );
            assert!(
                (g - g.round()).abs() < 1e-12 && (g.round() as i64) == (w.round() as i64),
                "{label}: element {i} is {g}, the mathematics says {w} — an integer datapath must be EXACT; \
                 fix the lowering or the evaluator, never this assertion"
            );
        }
        return;
    }
    let tol = tolerance(dtype, want);
    for (i, (g, w)) in got.iter().zip(want.iter()).enumerate() {
        assert!(
            g.is_finite(),
            "{label}: element {i} is {g} — a finite input can never produce a non-finite answer"
        );
        assert!(
            (g - w).abs() <= tol,
            "{label}: element {i} is {g}, the mathematics says {w} (|Δ| = {:.3e} > {tol:.3e} at {dtype:?}).\n\
             This is an ALGEBRA failure, not a rounding one: the tolerance is ≈16-168 unit roundoffs and \
             `a_perturbed_reference_is_caught_by_a_wide_margin` shows a wrong answer misses it.\n\
             Fix the compiler (`spatial/passes/from_tensor_op.rs` for the stage chain, `spatial/eval.rs` \
             for the op's meaning) — never the reference above, and never `rel_tol`.",
            (g - w).abs()
        );
    }
}

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// Harness
// ══════════════════════════════════════════════════════════════════════════════════════════════════

/// One workload instance: a LOGOS program, the operand data its source writes, and the answer its
/// mathematics demands.
struct Case {
    label: String,
    source: String,
    /// Every Input/Param tensor the source declares, by name, with the data the reference used. Checked
    /// against the fill the compiler recorded — see [`assert_the_fill_is_the_reference_data`].
    inputs: Vec<(&'static str, Vec<f64>)>,
    want: Vec<f64>,
}

/// What one (case, target) pair did. There is no third arm on purpose — "skipped" is not a verdict.
enum Verdict {
    Computed { got: Vec<f64>, dtype: Precision },
    Refused(SpatialReject),
}

/// ★ THE MACHINE MUST BE FED THE REFERENCE'S OWN DATA, AND THE FILL IS WHERE THAT CAN QUIETLY FAIL.
///
/// `inputs_from_fills` materializes each operand from the fill the COMPILER recorded off the source
/// (`tensor_recognize::recognize_fill` → `from_tensor_op::init_of`), not from the vector the reference
/// used. A recorder that dropped a `Set item` override — or read its index one off — would feed the
/// choreography different numbers, and the answer would then miss the mathematics with a message
/// blaming the ALGEBRA for what is a data bug. So the fill is checked first, bit for bit, and a
/// mismatch names the recorder instead.
fn assert_the_fill_is_the_reference_data(p: &SpatialProgram, case: &Case, label: &str) {
    for (name, vals) in &case.inputs {
        let td = p.tensors.iter().find(|t| t.name.eq_ignore_ascii_case(name)).unwrap_or_else(|| {
            let have: Vec<&str> = p.tensors.iter().map(|t| t.name.as_str()).collect();
            panic!("{label}: the compiled program declares no tensor named `{name}` — it has {have:?}. The source generator and the recognizer disagree about the operand names.")
        });
        let n = td.elems.rows * td.elems.cols;
        assert_eq!(
            n,
            vals.len(),
            "{label}: `{name}` reached the IR as {}×{} = {n} elements, the reference wrote {}",
            td.elems.rows,
            td.elems.cols,
            vals.len()
        );
        for (i, want) in vals.iter().enumerate() {
            let got = td.init.element_f64(i).unwrap_or_else(|| {
                panic!("{label}: `{name}` element {i} has no deterministic fill ({:?}) — every `Set item` must reach `TensorInit`", td.init)
            });
            assert_eq!(
                got.to_bits(),
                want.to_bits(),
                "{label}: `{name}` element {i} reached the choreography as {got}, the source wrote {want}. \
                 Fix `tensor_recognize::recognize_fill` / `from_tensor_op::init_of` — the algebra below is not at fault."
            );
        }
    }
}

/// Compile, evaluate, and hold the answer to the mathematics — or report a typed refusal.
fn evaluate_on(case: &Case, target: &SpatialTarget) -> Verdict {
    let p = match compile_to_spatial(&case.source, target) {
        Ok(p) => p,
        Err(e) => {
            // ★ A SOURCE THIS FILE GENERATED MUST BE RECOGNIZABLE. `Parse` / `Unrecognized` says the
            // generator emitted something the front end cannot see as a tensor op — a defect in THIS
            // file, and never a reason to quietly drop the case.
            if matches!(
                e,
                SpatialReject::Parse(_) | SpatialReject::Unrecognized { .. }
            ) {
                panic!(
                    "{}: the generated source must be recognized as a tensor op on {} — {e:?}\n\n{}",
                    case.label, target.name, case.source
                );
            }
            return Verdict::Refused(e);
        }
    };
    assert_eq!(
        p.phase(),
        Phase::Verified,
        "{} on {}: a compiled program must reach Verified",
        case.label,
        target.name
    );
    let label = format!("{} on {}", case.label, target.name);
    assert_the_fill_is_the_reference_data(&p, case, &label);
    let (got, dtype) = spatial_answer(&p, &label);
    assert_is_the_mathematics(&got, &case.want, dtype, &label);
    Verdict::Computed { got, dtype }
}

/// The program's output tensor, evaluated, with the precision its datapath stores it at.
fn spatial_answer(p: &SpatialProgram, label: &str) -> (Vec<f64>, Precision) {
    let out = eval_choreography(p, &inputs_from_fills(p))
        .unwrap_or_else(|f| panic!("{label}: a verified choreography must evaluate: {f:?}"));
    let id = output_id(p);
    let dtype = p
        .tensors
        .iter()
        .find(|t| t.role == TensorRole::Output)
        .expect("an output tensor")
        .dtype;
    let vals = match &out.tensors[&id] {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    };
    (vals, dtype)
}

/// A refusal has to NAME the capability the target lacks — and BE TRUE OF THE DESCRIPTOR.
///
/// ★ A TYPED REFUSAL IS ONLY EVIDENCE IF IT IS CHECKED. "This target has no semaphore" is a claim
/// about `target.handshakes`, and a pass that raised it against a target that DOES have one would be
/// bailing out for some other reason under a borrowed name — which reads, from the outside, exactly
/// like a capability gap. So each arm re-derives the claim from the descriptor the pass was handed.
///
/// ★ THE ARMS THAT ARE MISSING ARE THE POINT. `Unverified`, `Invalid`, `WrongPhase` and
/// `GroupCountMismatch` fall through to the panic because they are not capability gaps — they are the
/// compiler rejecting its own output, which is a defect and must be loud rather than tolerated as
/// "this target does not do that".
fn assert_refusal_is_named(label: &str, t: &SpatialTarget, e: &SpatialReject) {
    match e {
        SpatialReject::MissingCompute { needed, target } => assert!(
            !needed.is_empty() && *target == t.name,
            "{label}: a missing-compute refusal must name the engine and THIS target ({}), got {e:?}",
            t.name
        ),
        SpatialReject::MissingPrecision { needed, target } => assert!(
            *target == t.name && !t.precisions.contains(needed),
            "{label}: {} lists {needed:?} among its precisions, so a missing-precision refusal is a borrowed name, got {e:?}",
            t.name
        ),
        SpatialReject::MissingHandshake { needed, target } => assert!(
            *target == t.name && !t.provides_handshake(*needed),
            "{label}: {} provides {needed:?}, so a missing-handshake refusal is a borrowed name, got {e:?}",
            t.name
        ),
        SpatialReject::MissingInterconnect { needed, target } => assert!(
            *target == t.name && !t.provides_interconnect(*needed),
            "{label}: {} provides {needed:?}, so a missing-interconnect refusal is a borrowed name, got {e:?}",
            t.name
        ),
        SpatialReject::NoLegalTiling { rows_t, cols_t, sites, .. } => assert!(
            *rows_t > 0 && *cols_t > 0 && *sites > 0,
            "{label}: a no-legal-tiling refusal must describe a real geometry, got {e:?}"
        ),
        SpatialReject::LocalMemExceeded { needed_bytes, budget_bytes, .. } => assert!(
            needed_bytes > budget_bytes,
            "{label}: a local-memory refusal must be a real overrun, got {e:?}"
        ),
        SpatialReject::TooManyEngines { needed, available, .. } => assert!(
            *needed > *available as usize,
            "{label}: an engine-budget refusal must be a real overrun, got {e:?}"
        ),
        SpatialReject::AccTilesExceeded { needed, available, .. } => assert!(
            needed > available,
            "{label}: an accumulator-budget refusal must be a real overrun, got {e:?}"
        ),
        SpatialReject::NotLowered(what) => assert!(!what.is_empty(), "{label}: an unlowered refusal must name what, got {e:?}"),
        other => panic!(
            "{label}: this is the compiler rejecting its own output ({other:?}), not a capability the \
             target lacks. Fix the pipeline — do not add an arm here to tolerate it."
        ),
    }
}

/// Every preset, and the SN50 with its SIMD stages at bf16 alone: no preset's float datapath is bf16 now that the RDUs
/// hold fp32 islands, and `rel_tol`'s bf16 arm is demonstrated on the datapath it states.
fn targets() -> Vec<SpatialTarget> {
    let mut bf16 = SpatialTarget::sn50_rdu();
    for c in &mut bf16.compute {
        if let ComputeCap::Elementwise { precisions } | ComputeCap::RowReduce { precisions } = c {
            precisions.remove(&Precision::Fp32);
        }
    }
    bf16.precisions.remove(&Precision::Fp32);
    SpatialTarget::presets().into_iter().chain([bf16]).collect()
}

/// Run every case on every preset. Refusals must be typed, named and true of the descriptor; EVERY CASE
/// must have been computed by at least one target, so no shape can pass by being refused everywhere.
fn assert_workload(workload: &str, cases: &[Case]) {
    assert!(
        !cases.is_empty(),
        "{workload}: a workload with no cases is a workload nobody tests"
    );
    let mut refusals: Vec<String> = Vec::new();
    for case in cases {
        let mut computed = 0usize;
        for target in targets() {
            let label = format!("{} on {}", case.label, target.name);
            match evaluate_on(case, &target) {
                Verdict::Computed { .. } => computed += 1,
                Verdict::Refused(e) => {
                    assert_refusal_is_named(&label, &target, &e);
                    refusals.push(format!("  {label}: {e:?}"));
                }
            }
        }
        // ★ PER CASE, NOT PER WORKLOAD. A single surviving shape would otherwise cover for every other
        // one silently turning into a refusal, and "the compiler stopped compiling this shape anywhere"
        // is a regression, not a capability gap.
        assert!(
            computed > 0,
            "{workload}: EVERY preset refused `{}`, so that shape was never checked against the \
             mathematics. Fix the pipeline or the target descriptor — do not delete the case.",
            case.label
        );
    }
    if !refusals.is_empty() {
        eprintln!(
            "{workload}: {} typed refusal(s) — each names what the target lacks:\n{}",
            refusals.len(),
            refusals.join("\n")
        );
    }
}

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// The workloads
// ══════════════════════════════════════════════════════════════════════════════════════════════════

#[test]
fn elementwise_add_is_the_sum_of_its_operands() {
    let mut cases = Vec::new();
    // Integer operands: an integer datapath owes an exact answer, and a float one is exact here too
    // (|a|,|b| ≤ 9, so every partial fits the fp32 mantissa with room to spare).
    for (i, n) in [4usize, 9, 17, 32].into_iter().enumerate() {
        let seed = 0x5A1D_0001u64.wrapping_add(i as u64);
        let a = small_ints(seed, n);
        let b = small_ints(seed ^ 0xFFFF, n);
        let source = format!(
            "## Main\n{}{}Let C be A plus B.\nShow item 1 of C.\n",
            int_tensor("A", 1, n, &a),
            int_tensor("B", 1, n, &b)
        );
        let af: Vec<f64> = a.iter().map(|v| *v as f64).collect();
        let bf: Vec<f64> = b.iter().map(|v| *v as f64).collect();
        cases.push(Case {
            label: format!("elementwise int 1×{n}"),
            source,
            inputs: vec![("A", af.clone()), ("B", bf.clone())],
            want: ref_add(&af, &bf),
        });
    }
    // Real operands: the same algebra on the float datapath.
    for (i, n) in [4usize, 9, 17, 32].into_iter().enumerate() {
        let seed = 0x5A1D_0002u64.wrapping_add(i as u64);
        let a = quarters(seed, n);
        let b = quarters(seed ^ 0xFFFF, n);
        let source = format!(
            "## Main\n{}{}Let C be A plus B.\nShow item 1 of C.\n",
            real_tensor("A", 1, n, &a),
            real_tensor("B", 1, n, &b)
        );
        cases.push(Case {
            label: format!("elementwise real 1×{n}"),
            source,
            inputs: vec![("A", a.clone()), ("B", b.clone())],
            want: ref_add(&a, &b),
        });
    }
    assert_workload("elementwise add", &cases);
}

#[test]
fn swiglu_is_silu_of_the_gate_times_the_up_projection() {
    let mut cases = Vec::new();
    for (i, n) in [4usize, 9, 17, 32].into_iter().enumerate() {
        let seed = 0x5A1D_0003u64.wrapping_add(i as u64);
        let g = quarters(seed, n);
        let u = quarters(seed ^ 0xFFFF, n);
        let source = format!(
            "## Main\n{}{}Let Y be swiglu of G and U.\nShow item 1 of Y.\n",
            real_tensor("G", 1, n, &g),
            real_tensor("U", 1, n, &u)
        );
        cases.push(Case {
            label: format!("swiglu 1×{n}"),
            source,
            inputs: vec![("G", g.clone()), ("U", u.clone())],
            want: ref_swiglu(&g, &u),
        });
    }
    assert_workload("swiglu", &cases);
}

#[test]
fn weighted_rmsnorm_is_the_unit_rms_rescale() {
    let mut cases = Vec::new();
    for (i, (rows, cols)) in [(1usize, 4usize), (2, 3), (3, 7), (2, 24)]
        .into_iter()
        .enumerate()
    {
        let seed = 0x5A1D_0004u64.wrapping_add(i as u64);
        let x = spread_rows(seed, rows, cols);
        let w = quarters(seed ^ 0xFFFF, cols);
        let source = format!(
            "## Main\n{}{}Let Y be rmsnorm of X with weight W.\nShow item 1 of Y.\n",
            real_tensor("X", rows, cols, &x),
            real_tensor("W", 1, cols, &w)
        );
        cases.push(Case {
            label: format!("rmsnorm {rows}×{cols}"),
            source,
            inputs: vec![("X", x.clone()), ("W", w.clone())],
            want: ref_rmsnorm(&x, &w, rows, cols, SURFACE_EPS),
        });
    }
    assert_workload("weighted rmsnorm", &cases);
}

#[test]
fn softmax_is_the_row_probability_distribution() {
    let mut cases = Vec::new();
    for (i, n) in [4usize, 7, 12, 24].into_iter().enumerate() {
        let seed = 0x5A1D_0005u64.wrapping_add(i as u64);
        let x = logits(seed, n);
        let source = format!(
            "## Main\n{}Let Y be softmax of X.\nShow item 1 of Y.\n",
            real_tensor("X", 1, n, &x)
        );
        cases.push(Case {
            label: format!("softmax 1×{n}"),
            source,
            inputs: vec![("X", x.clone())],
            want: ref_softmax(&x),
        });
    }
    assert_workload("softmax", &cases);
}

#[test]
fn layernorm_is_the_mean_variance_normalization() {
    let mut cases = Vec::new();
    for (i, (rows, cols)) in [(1usize, 4usize), (2, 3), (3, 7), (2, 17)]
        .into_iter()
        .enumerate()
    {
        let seed = 0x5A1D_0006u64.wrapping_add(i as u64);
        let x = spread_rows(seed, rows, cols);
        let w = quarters(seed ^ 0xFFFF, cols);
        let b = quarters(seed ^ 0x0F0F, cols);
        let source = format!(
            "## Main\n{}{}{}Let Y be layernorm of X with weight W and bias B.\nShow item 1 of Y.\n",
            real_tensor("X", rows, cols, &x),
            real_tensor("W", 1, cols, &w),
            real_tensor("B", 1, cols, &b)
        );
        cases.push(Case {
            label: format!("layernorm {rows}×{cols}"),
            source,
            inputs: vec![("X", x.clone()), ("W", w.clone()), ("B", b.clone())],
            want: ref_layernorm(&x, &w, &b, rows, cols, SURFACE_EPS),
        });
    }
    assert_workload("layernorm", &cases);
}

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// Non-vacuity — the comparison can tell a right answer from a wrong one
// ══════════════════════════════════════════════════════════════════════════════════════════════════

/// How far `got` sits from a reference, in units of that reference's tolerance.
fn margin_in_tolerances(got: &[f64], reference: &[f64], dtype: Precision) -> f64 {
    let tol = tolerance(dtype, reference);
    let worst = got
        .iter()
        .zip(reference.iter())
        .fold(0.0f64, |m, (g, r)| m.max((g - r).abs()));
    worst / tol
}

/// How far a WRONG answer must sit from the machine's, in tolerances, for `rel_tol(p)` to count as
/// demonstrated on that datapath.
///
/// ★ THE FLOOR IS PER PRECISION BECAUSE THE TOLERANCE IS, AND THE TWO ARE NOT REMOTELY ALIKE. The
/// margins below were derived by working each probe's arithmetic through the evaluator's own rounding
/// points by hand; the test PRINTS the margin it actually measures for every (probe, preset), and
/// those printed numbers — not this table — are the authority:
///
/// |            | elementwise | swiglu  | rmsnorm | softmax | layernorm |
/// |------------|-------------|---------|---------|---------|-----------|
/// | fp32       |   62_500×   | 10_307× | 73_205× | 25_555× |  16_726×  |
/// | bf16       |      19×    |      3× |     22× |      8× |       5×  |
///
/// fp32's tolerance is four to five orders of magnitude too tight to admit any of these errors; bf16's
/// is 3200× looser and separates the swiglu perturbation by barely 3×. THAT IS THE HONEST NUMBER and
/// the reason bf16 is probed at all: an undemonstrated tolerance is an unfalsifiable one, and the RDU
/// is a real preset running real bf16. The floors keep a working margin under each measurement (fp32:
/// 100× against 10³-fold headroom; bf16: 2.5× against a worked-through 3.2×). Raising a floor above what
/// the run prints is how this test starts flaking; lowering one is how it stops meaning anything.
fn margin_floor(p: Precision) -> f64 {
    match p {
        Precision::Fp32 => 100.0,
        Precision::Bf16 => 2.5,
        other => panic!("no margin floor is stated for {other:?} — state one HERE with its measured margins, beside its `rel_tol` entry"),
    }
}

/// One non-vacuity probe: a program, its data, the mathematics, and a deliberately wrong mathematics.
struct Probe {
    label: &'static str,
    source: String,
    inputs: Vec<(&'static str, Vec<f64>)>,
    truth: Vec<f64>,
    wrong: Vec<f64>,
}

/// ★ THE DEMONSTRATION THIS FILE'S DOC COMMENT DESCRIBES.
///
/// Five hand-computed workloads, each compiled and evaluated on EVERY preset, each compared against
/// BOTH the true mathematics and a deliberately wrong one. The true reference must land inside the
/// tolerance (asserted inside `evaluate_on`, by the very code path the workload tests use); the
/// perturbed reference must miss it by more than `margin_floor` for that datapath's precision.
/// Without this, every tolerance in `rel_tol` would be an unfalsifiable claim — a tolerance wide
/// enough to accept a wrong answer looks exactly like a tolerance that is passing.
///
/// The perturbations are real error modes, not noise:
///   · elementwise — the product where the sum is meant (the wrong operator entirely)
///   · SwiGLU      — `silu(U)·G`, the operands transposed
///   · RMSNorm     — `sqrt(Σ X²)` instead of `sqrt(Σ X² / C)`: the sum where the algebra says the mean
///   · softmax     — temperature 2, `e^(2X) / Σ e^(2X)`
///   · LayerNorm   — the sample variance `Σ(x−μ)²/(C−1)` where the algebra says the population one
#[test]
fn a_perturbed_reference_is_caught_by_a_wide_margin() {
    // Small, hand-checkable, ¼-grid data — so the margins below are properties of the maths, not of a
    // lucky draw.
    let a = [1.0f64, 2.0, 3.0, 4.0];
    let b = [5.0f64, 6.0, 7.0, 8.0];
    let g = [-1.0f64, 0.0, 1.0, 2.0];
    let u = [1.0f64, 2.0, 3.0, 4.0];
    let x2x3 = [1.0f64, 2.0, 3.0, 4.0, 5.0, 6.0];
    // ★ THE LayerNorm PROBE GETS ITS OWN, WIDER-SPREAD ROWS. Its rows carry variances of 14 and ~38.9,
    // which is the same reason `spread_rows` exists: the LOGOS surface declares ε = 1e-6 and the fused
    // `ComputeOp::RowReduce` arm in `spatial/eval.rs` uses 1e-5, and at a variance of ⅔ that
    // disagreement is half the fp32 tolerance — this probe would then be measuring a constant instead
    // of the algebra. At these variances it is ≈3e-7 relative, thirty-six times under it.
    let xln = [2.0f64, 5.0, 11.0, -6.0, 4.0, 9.0];
    let w3 = [0.5f64, 1.0, 2.0];
    let b3 = [0.25f64, 0.5, 0.75];
    let sm = [1.0f64, 2.0, 3.0, 4.0];

    let mut probes: Vec<Probe> = Vec::new();

    probes.push(Probe {
        label: "elementwise: product instead of sum",
        source: format!(
            "## Main\n{}{}Let C be A plus B.\nShow item 1 of C.\n",
            real_tensor("A", 1, 4, &a),
            real_tensor("B", 1, 4, &b)
        ),
        inputs: vec![("A", a.to_vec()), ("B", b.to_vec())],
        truth: ref_add(&a, &b),
        wrong: (0..4).map(|p| a[p] * b[p]).collect(),
    });

    probes.push(Probe {
        label: "swiglu: operands transposed",
        source: format!(
            "## Main\n{}{}Let Y be swiglu of G and U.\nShow item 1 of Y.\n",
            real_tensor("G", 1, 4, &g),
            real_tensor("U", 1, 4, &u)
        ),
        inputs: vec![("G", g.to_vec()), ("U", u.to_vec())],
        truth: ref_swiglu(&g, &u),
        wrong: ref_swiglu(&u, &g),
    });

    // The sum of squares where the algebra says their mean — a `√C` error in the denominator.
    let rms_sum_instead_of_mean = {
        let (rows, cols) = (2usize, 3usize);
        let mut y = vec![0.0f64; rows * cols];
        for r in 0..rows {
            let sum_sq: f64 = (0..cols)
                .map(|j| x2x3[r * cols + j] * x2x3[r * cols + j])
                .sum();
            let denom = (sum_sq + SURFACE_EPS).sqrt();
            for i in 0..cols {
                y[r * cols + i] = x2x3[r * cols + i] / denom * w3[i];
            }
        }
        y
    };
    probes.push(Probe {
        label: "rmsnorm: sum of squares instead of their mean",
        source: format!(
            "## Main\n{}{}Let Y be rmsnorm of X with weight W.\nShow item 1 of Y.\n",
            real_tensor("X", 2, 3, &x2x3),
            real_tensor("W", 1, 3, &w3)
        ),
        inputs: vec![("X", x2x3.to_vec()), ("W", w3.to_vec())],
        truth: ref_rmsnorm(&x2x3, &w3, 2, 3, SURFACE_EPS),
        wrong: rms_sum_instead_of_mean,
    });

    // Temperature 2 — the same shape of formula, the wrong exponent.
    let softmax_temperature_two = {
        let doubled: Vec<f64> = sm.iter().map(|v| 2.0 * v).collect();
        ref_softmax(&doubled)
    };
    probes.push(Probe {
        label: "softmax: temperature 2",
        source: format!(
            "## Main\n{}Let Y be softmax of X.\nShow item 1 of Y.\n",
            real_tensor("X", 1, 4, &sm)
        ),
        inputs: vec![("X", sm.to_vec())],
        truth: ref_softmax(&sm),
        wrong: softmax_temperature_two,
    });

    // The sample variance (÷ C−1) where the algebra says the population variance (÷ C).
    let layernorm_sample_variance = {
        let (rows, cols) = (2usize, 3usize);
        let mut y = vec![0.0f64; rows * cols];
        for r in 0..rows {
            let mean: f64 = (0..cols).map(|j| xln[r * cols + j]).sum::<f64>() / cols as f64;
            let var: f64 = (0..cols)
                .map(|j| (xln[r * cols + j] - mean).powi(2))
                .sum::<f64>()
                / (cols - 1) as f64;
            let denom = (var + SURFACE_EPS).sqrt();
            for i in 0..cols {
                y[r * cols + i] = (xln[r * cols + i] - mean) / denom * w3[i] + b3[i];
            }
        }
        y
    };
    probes.push(Probe {
        label: "layernorm: sample variance instead of population variance",
        source: format!(
            "## Main\n{}{}{}Let Y be layernorm of X with weight W and bias B.\nShow item 1 of Y.\n",
            real_tensor("X", 2, 3, &xln),
            real_tensor("W", 1, 3, &w3),
            real_tensor("B", 1, 3, &b3)
        ),
        inputs: vec![("X", xln.to_vec()), ("W", w3.to_vec()), ("B", b3.to_vec())],
        truth: ref_layernorm(&xln, &w3, &b3, 2, 3, SURFACE_EPS),
        wrong: layernorm_sample_variance,
    });

    assert_eq!(
        probes.len(),
        5,
        "one demonstration per workload family — adding a workload above adds a probe here"
    );

    let mut demonstrated: BTreeSet<Precision> = BTreeSet::new();
    let mut refusals: Vec<String> = Vec::new();
    for probe in &probes {
        let case = Case {
            label: probe.label.to_string(),
            source: probe.source.clone(),
            inputs: probe.inputs.clone(),
            want: probe.truth.clone(),
        };
        let mut demonstrated_here = 0usize;
        for target in targets() {
            let label = format!("{} on {}", probe.label, target.name);
            match evaluate_on(&case, &target) {
                // Half one — the machine agrees with the mathematics — was asserted inside
                // `evaluate_on`, by the same code path the workload tests run through.
                Verdict::Computed { got, dtype } => {
                    // Half two: and the SAME comparison would have rejected the wrong mathematics —
                    // which is what makes half one a claim rather than a formality.
                    let margin = margin_in_tolerances(&got, &probe.wrong, dtype);
                    let floor = margin_floor(dtype);
                    assert!(
                        margin > floor,
                        "{label}: the perturbed reference is only {margin:.1}× the {dtype:?} tolerance away from \
                         the machine's answer (the floor is {floor}×), so this comparison could not reliably tell \
                         them apart. Either the perturbation is too small to be a demonstration, or \
                         `rel_tol({dtype:?})` has been widened past the point of usefulness — tighten the \
                         tolerance, do not weaken this bound."
                    );
                    eprintln!(
                        "{label}: wrong answer misses by {margin:.0}× the {dtype:?} tolerance"
                    );
                    demonstrated.insert(dtype);
                    demonstrated_here += 1;
                }
                Verdict::Refused(e) => {
                    assert_refusal_is_named(&label, &target, &e);
                    refusals.push(format!("  {label}: {e:?}"));
                }
            }
        }
        assert!(
            demonstrated_here > 0,
            "{}: EVERY preset refused the probe, so this perturbation demonstrated nothing.",
            probe.label
        );
    }
    if !refusals.is_empty() {
        eprintln!(
            "perturbation probes: {} typed refusal(s):\n{}",
            refusals.len(),
            refusals.join("\n")
        );
    }

    // ★ THE RATCHET THAT KEEPS `rel_tol` HONEST. Every precision `rel_tol` answers for must have been
    // exercised here, or its number would be a tolerance nothing can falsify — and a tolerance nothing
    // falsifies is how a wrong answer eventually walks through. If a preset stops selecting bf16, or a
    // new precision joins `rel_tol`, this is the assertion that says so instead of silently shrinking
    // the demonstration.
    let expected: BTreeSet<Precision> = [Precision::Fp32, Precision::Bf16].into_iter().collect();
    assert_eq!(
        demonstrated, expected,
        "the precisions demonstrated here must be exactly the precisions `rel_tol` states a tolerance for. \
         Add the missing arm's preset to the probe, or remove the tolerance you cannot demonstrate — \
         never leave an undemonstrated number in `rel_tol`."
    );
}
