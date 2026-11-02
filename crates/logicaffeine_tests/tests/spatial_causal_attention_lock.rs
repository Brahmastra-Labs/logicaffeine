//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! CAUSAL ATTENTION — a block of query rows against a cache in ONE op, each row up to its own position.
//!
//! `Let C be causal attention of Q over K and V with H heads KV groups from P.` — `Q` is `T × (H·HD)`, `K` and `V` are
//! `L × (KV·HD)`, `C` is `T × (H·HD)`. Row `r` (1-based) is the query at position `P + r − 1` and attends cache
//! positions `1 … P + r − 1`: a prefill chunk's queries against a cache the chunk has just appended its own keys and
//! values to. It is T single-query attentions (`attention of … up to P + r − 1`), written as the one statement a
//! compiler can lower as one — the block's query tiles × heads as work items of one flash section, not T sections.
//!
//! Pinned:
//!   1. MEANING. On the VM and the tree-walker, row `r` of `C` is exactly the single-query attention of row `r` of `Q`
//!      up to `P + r − 1` — GQA and MHA, a literal `P` and one the program computes — and equals the f64 reference.
//!   2. REFUSALS. The single-query sugar refuses a query block by name (it would attend the block's first row alone);
//!      a `P` that puts a row outside the cache (`P < 1` or `P + T − 1 > L`) fails with one message on every tier; the
//!      bound expression runs once.
//!   3. THE SN50 COMPUTES IT. The block lowers whole — heads narrower than a tile (the flat form) and heads of whole
//!      tiles (the flash form, including rows whose positions straddle two cache blocks) — for a literal `P` and a
//!      computed one, and evaluates to the VM's answer under the ledger's two-part rule (WIRING: the fp32 twin within
//!      a relative 2⁻¹⁶; CHIP: bf16 within `sections + 1` ulps of `Σ p·|v|`). A computed `P` outside `1 ..= L − T + 1`
//!      FAULTS at its guard, whose extent is exactly that range — the flat form's index guard; the flash fold runs its
//!      blocks to `P` (`spatial_trip_count_lock`), and its trip, admitting exactly that range, refuses it before any op.
//!   4. THE PREFILL CYCLE. Every row's key and value appended at its own position, the rows' queries stacked, then ONE
//!      causal attention from the first position — lowers whole on the SN50 and computes what the VM computes.
//!   5. ONE ATTENTION. The block's section count does not grow with its rows: the flash block section runs the block's
//!      query TILES × heads as its work items (`⌈T/tile⌉·H`), rows within a tile adding neither items nor stages, and one
//!      row through the causal sugar lowers to exactly the decode form's stages — for heads that share a K/V head, the
//!      grouped decode form: an item per K/V head, between two regroupings.
//!   6. THE BLOCK TAKES THE LAYOUT OF THE MATRIX OPS AROUND IT. Queries a matmul wrote (a dense matrix) and a result a
//!      pointwise op reads as a matrix lower whole — the flash tile form takes the matrix layout at both edges with NO
//!      relayout, the flat form moves the block at once through one relayout per edge — and compute what the VM
//!      computes, for a block inside one tile row and a block of two whole tile rows.
//!   7. MORE QUERY HEADS THAN SITES. A site holding several items runs each through the same stages: on an SN50 whose
//!      mesh is smaller than the block's query heads, the causal block and a full attention — each over two cache
//!      blocks, so their partials are stored per item — lower and compute what the VM computes.
//!   8. EVERY BACKEND. The executing tiers (AOT-C, RISC-V under qemu) print the VM's answer and the codegen tiers
//!      (WASM, CUDA, Tenstorrent) lower the block; the single-op CUDA path refuses it rather than lower a fragment.
//!      (`tensor_probes` maps the sugar's desugar function to the `attention` probe, which runs it with one row; the
//!      multi-row block's cells live here.)
//!
//! Inputs are exact in bf16 (multiples of 1/64), so the chip check measures the lowering's arithmetic, not the data's
//! quantization.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_causal_attention_lock`
//!
//! ★ MUTATION PROBES (run 2026-09-26/27 in a private copy — break, observe red; logs `logs/sn50/probe_causal2_*.txt`,
//!   `probe_relayout3_*.txt` and `probe_ffold_*.txt`):
//!   • the flash fold's row limit drops the row's own index → the flash form, the prefill cycle, the layout test and a
//!     site holding several query heads red on WIRING (`-406250` vs VM `505931`);
//!   • the flat mask drops its row offset (threshold `v·l + P`) → the flat form, the flat prefill cycle and the flat
//!     layout case red on WIRING (`-406250` vs VM `-373037`);
//!   • the computed bound's guard admits the whole cache (`bound_max` = `L`) → the guard fault reds on its extent
//!     (`40` vs `38`); (2026-09-27, trip counts) widened by one → the flash fold's trip refuses `P = 0` against
//!     `(1, 39)`, not `(1, 38)`: the guard test reds on the range it names;
//!   • the recognizer reports a block as one row (`rows: 1`) → six tests red: `C` short of elements, the guard's extent,
//!     a tensor-shape conflict on `C` when the block composes;
//!   • every flash item reads K/V head 0 → the GQA flash form and the GQA prefill cycle red on WIRING (`296741` vs
//!     VM `-10819`);
//!   • the flat rows→matrix relayout reads row 0 for every row → both flat layout cases red on WIRING (`-123229` vs VM
//!     `-149602`);
//!   • the flat matrix→rows relayout reads every row at row 0's offset → the flat block between matrix ops reds on
//!     WIRING (`-23438` vs VM `-149602`);
//!   • the layout pass leaves a section's written matrix unbridged → both flat layout cases red on a
//!     `TensorShapeConflict` (`C`: `(1, 48)` vs `(3, 16)`); left unbridged on a read → the flat block between matrix ops
//!     reds the same way on `Q`;
//!   • the fold's block start frozen at 0, or its running maximum never updated → every flash case reds (`40751` vs VM
//!     `67692`; `-687500` vs VM `318442`);
//!   • (2026-09-25) the layout pass bridges a carry one section updates and the next reads → `qwen38_real_dims_lock`
//!     reds: the conv ring is never stored (`TensorRoleDeferred`) and the gated-delta memory's relayout overruns the
//!     site budget.
//!   The whole-tile relayouts no longer run here — a flash block of rows takes the matrix layout at both edges — and
//!   `tensor_gdn_row_blocks_lock` holds them: segment 0 for every row, in-tile row 0 for every row, the site budget
//!   ignored, each red there.
//! ════════════════════════════════════════════════════════════════════════════════════════════════
#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

#[allow(dead_code)]
mod common;

use logicaffeine_compile::codegen_riscv::RISCV_RUNTIME_C;
use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::{
    compile_to_c, compile_to_cuda, compile_to_cuda_tensor_op, compile_to_riscv,
    compile_to_tenstorrent, compile_to_wasm, tw_outcome, vm_outcome,
};
use logicaffeine_compile::spatial::eval::{eval_choreography, ChoreoFault};
use logicaffeine_compile::spatial::ir::{Phase, SpatialProgram, TensorInit};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use tensor_probes::{inputs_from_fills, oracle_ints, shown_from_outputs};

fn sn50() -> SpatialTarget {
    SpatialTarget::sn50_rdu()
}

fn fp32_twin() -> SpatialTarget {
    let mut t = sn50();
    for c in &mut t.compute {
        if let ComputeCap::MatmulTile { precisions, .. }
        | ComputeCap::Elementwise { precisions }
        | ComputeCap::RowReduce { precisions } = c
        {
            if precisions.contains(&Precision::Bf16) {
                precisions.insert(Precision::Fp32);
            }
        }
    }
    t.precisions.insert(Precision::Fp32);
    t
}

/// A block attention's shape: `t` query rows of `h` heads over `kv` K/V heads of `hd`, a cache of `l` rows.
#[derive(Clone, Copy, Debug)]
struct Shape {
    t: usize,
    h: usize,
    kv: usize,
    hd: usize,
    l: usize,
}

impl Shape {
    /// The largest `P` the block admits: its last row attends the whole cache.
    fn p_max(&self) -> usize {
        self.l + 1 - self.t
    }
    fn width(&self) -> usize {
        self.h * self.hd
    }
}

/// GQA, heads narrower than a tile: the flat form.
const FLAT_GQA: Shape = Shape {
    t: 3,
    h: 4,
    kv: 2,
    hd: 4,
    l: 8,
};
/// MHA, heads narrower than a tile: every query head reads its own K/V head.
const FLAT_MHA: Shape = Shape {
    t: 4,
    h: 2,
    kv: 2,
    hd: 4,
    l: 6,
};
/// Heads of one whole tile on the SN50 — the flash form. The cache is two tile rows of positions.
const FLASH: Shape = Shape {
    t: 3,
    h: 2,
    kv: 1,
    hd: 32,
    l: 40,
};
/// GQA heads of four tiles, a cache just past one tile row.
const FLASH_GQA: Shape = Shape {
    t: 4,
    h: 4,
    kv: 2,
    hd: 128,
    l: 33,
};

/// A value in `[-1, 1]` in steps of 1/64 — exact in bf16, in f32 and in the VM.
fn val(seed: usize) -> f64 {
    (((seed.wrapping_mul(7919).wrapping_add(13)) % 129) as f64 - 64.0) / 64.0
}

fn vals(n: usize, salt: usize) -> Vec<f64> {
    (0..n).map(|p| val(p * 31 + salt)).collect()
}

fn declare(name: &str, rows: usize, cols: usize, v: &[f64]) -> String {
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of Real.\n");
    for (p, x) in v.iter().enumerate() {
        if *x != 0.0 {
            s += &format!("Set item {} of {name} to {:?}.\n", p + 1, x);
        }
    }
    s
}

/// The block in f64: query row `r` (0-based) attends the first `p + r` positions. Per output element, the value and
/// the magnitude `Σ p_j·|v_j|`.
fn reference(q: &[f64], k: &[f64], v: &[f64], s: &Shape, p: usize) -> Vec<(f64, f64)> {
    let group = s.h / s.kv;
    let mut out = Vec::new();
    for r in 0..s.t {
        let live = p + r;
        for head in 0..s.h {
            let kvh = head / group;
            let at = |m: &[f64], j: usize, e: usize| m[j * s.kv * s.hd + kvh * s.hd + e];
            let qv = |e: usize| q[r * s.width() + head * s.hd + e];
            let scores: Vec<f64> = (0..live)
                .map(|j| (0..s.hd).map(|e| qv(e) * at(k, j, e)).sum::<f64>() / (s.hd as f64).sqrt())
                .collect();
            let mx = scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let ex: Vec<f64> = scores.iter().map(|x| (x - mx).exp()).collect();
            let z: f64 = ex.iter().sum();
            for e in 0..s.hd {
                out.push((
                    (0..live).map(|j| ex[j] / z * at(v, j, e)).sum(),
                    (0..live).map(|j| ex[j] / z * at(v, j, e).abs()).sum(),
                ));
            }
        }
    }
    out
}

/// The shown elements: every `STRIDE`-th of the block, so every row and every head is sampled.
const STRIDE: usize = 3;

fn shown_indices(s: &Shape) -> impl Iterator<Item = usize> {
    (0..s.t * s.width()).step_by(STRIDE)
}

fn shows(s: &Shape) -> String {
    shown_indices(s)
        .map(|i| format!("Show round(item {} of C * 1000000).\n", i + 1))
        .collect()
}

/// A case: the source and, per `Show`, the reference value and magnitude.
struct Case {
    src: String,
    reference: Vec<(f64, f64)>,
}

/// How the program states `P`: a literal, or a name the program binds.
#[derive(Clone, Copy, Debug)]
enum Bound {
    Literal,
    Named,
}

fn operands(s: &Shape) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    (
        vals(s.t * s.width(), 1),
        vals(s.l * s.kv * s.hd, 2),
        vals(s.l * s.kv * s.hd, 3),
    )
}

fn reference_at(s: &Shape, p: usize, q: &[f64], k: &[f64], v: &[f64]) -> Vec<(f64, f64)> {
    // A block with a row outside the cache has no attention to refer to — the program fails; there is only the source.
    if (1..=s.p_max()).contains(&p) {
        let r = reference(q, k, v, s, p);
        shown_indices(s).map(|i| r[i]).collect()
    } else {
        Vec::new()
    }
}

/// `causal attention … from <P>`, the block as one statement.
fn block(s: &Shape, p: usize, bound: Bound) -> Case {
    let (q, k, v) = operands(s);
    let mut src = String::from("## Main\n");
    src += &declare("Q", s.t, s.width(), &q);
    src += &declare("K", s.l, s.kv * s.hd, &k);
    src += &declare("V", s.l, s.kv * s.hd, &v);
    let from = match bound {
        Bound::Literal => p.to_string(),
        Bound::Named => {
            src += &format!("Let P be {p}.\n");
            "P".to_string()
        }
    };
    src += &format!(
        "Let C be causal attention of Q over K and V with {} heads {} groups from {from}.\n",
        s.h, s.kv
    );
    src += &shows(s);
    Case {
        reference: reference_at(s, p, &q, &k, &v),
        src,
    }
}

/// The block as its rows: row `r` of `Q` attending up to `P + r − 1`, each row's result stacked into `C`.
fn rows(s: &Shape, p: usize) -> String {
    let (q, k, v) = operands(s);
    let mut src = String::from("## Main\n");
    src += &declare("Q", s.t, s.width(), &q);
    src += &declare("K", s.l, s.kv * s.hd, &k);
    src += &declare("V", s.l, s.kv * s.hd, &v);
    src += &format!("Let C be a {} by {} tensor of Real.\n", s.t, s.width());
    for r in 1..=s.t {
        src += &format!(
            "Let q{r} be row {r} of Q.\nLet c{r} be attention of q{r} over K and V with {} heads {} groups up to {}.\nSet row {r} of C to c{r}.\n",
            s.h,
            s.kv,
            p + r - 1
        );
    }
    src += &shows(s);
    src
}

/// The prefill cycle: the cache filled for positions before `p`, then row `r`'s key and value appended at position
/// `p + r`, the rows' queries stacked into `Q`, and ONE causal attention from `p`.
fn prefill_cycle(s: &Shape, p: usize) -> Case {
    let w = s.kv * s.hd;
    let before = (p - 1) * w;
    let mut k: Vec<f64> = vals(s.l * w, 5)
        .into_iter()
        .enumerate()
        .map(|(i, x)| if i < before { x } else { 0.0 })
        .collect();
    let mut v: Vec<f64> = vals(s.l * w, 6)
        .into_iter()
        .enumerate()
        .map(|(i, x)| if i < before { x } else { 0.0 })
        .collect();
    let mut src = String::from("## Main\n");
    src += &declare("Kc", s.l, w, &k);
    src += &declare("Vc", s.l, w, &v);
    src += &format!("Let Q be a {} by {} tensor of Real.\n", s.t, s.width());
    let mut q = Vec::new();
    for r in 0..s.t {
        let (qr, kr, vr) = (vals(s.width(), 40 + r), vals(w, 60 + r), vals(w, 80 + r));
        src += &declare(&format!("q{r}"), 1, s.width(), &qr);
        src += &declare(&format!("k{r}"), 1, w, &kr);
        src += &declare(&format!("v{r}"), 1, w, &vr);
        src += &format!("Let pos{r} be {}.\nSet row pos{r} of Kc to k{r}.\nSet row pos{r} of Vc to v{r}.\nSet row {} of Q to q{r}.\n", p + r, r + 1);
        let at = (p - 1 + r) * w;
        k[at..at + w].copy_from_slice(&kr);
        v[at..at + w].copy_from_slice(&vr);
        q.extend(qr);
    }
    src += &format!(
        "Let C be causal attention of Q over Kc and Vc with {} heads {} groups from pos0.\n",
        s.h, s.kv
    );
    src += &shows(s);
    let r = reference(&q, &k, &v, s, p);
    Case {
        src,
        reference: shown_indices(s).map(|i| r[i]).collect(),
    }
}

fn lines(s: &str) -> Vec<String> {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Tree-walker and VM agree; returns (output, error).
fn tiers(src: &str) -> (Vec<String>, Option<String>) {
    let (tw, vm) = (tw_outcome(src), vm_outcome(src));
    assert_eq!(
        (lines(&tw.output), &tw.error),
        (lines(&vm.output), &vm.error),
        "the VM and the tree-walker disagree on:\n{src}"
    );
    (lines(&tw.output), tw.error)
}

fn composed(src: &str, target: &SpatialTarget, what: &str) -> (SpatialProgram, usize) {
    let pl = compile_program_to_spatial(src, target)
        .unwrap_or_else(|e| panic!("{what}: walk refused: {e:?}"));
    let sp = pl
        .whole()
        .unwrap_or_else(|| panic!("{what}: not lowered whole: {}", pl.refusal()))
        .clone();
    assert_eq!(
        sp.phase(),
        Phase::Verified,
        "{what}: the composed program certifies"
    );
    assert!(
        sp.certificate.as_ref().is_some_and(|c| c.recheck(&sp)),
        "{what}: its certificate rechecks"
    );
    (sp, pl.sections)
}

fn ulp_bf16(x: f64) -> f64 {
    let e = if x == 0.0 {
        -126
    } else {
        (x.abs().log2().floor() as i32).max(-126)
    };
    2f64.powi(e - 7)
}

/// The VM's answer, checked against the f64 reference.
fn vm_checked(c: &Case, what: &str) -> Vec<i64> {
    let (out, err) = tiers(&c.src);
    assert_eq!(err, None, "{what}: {err:?}");
    let want = oracle_ints(&out.join("\n")).expect("ints");
    assert_eq!(
        want.len(),
        c.reference.len(),
        "{what}: every Show has a reference"
    );
    for (i, (v, (r, _))) in want.iter().zip(&c.reference).enumerate() {
        assert!(
            ((*v as f64) - (r * 1e6).round()).abs() <= 1.0,
            "{what}: Show #{}: VM {v} vs f64 reference {}",
            i + 1,
            r * 1e6
        );
    }
    want
}

/// The two-part verdict, with the VM also checked against the f64 reference.
fn verdict(c: &Case, what: &str) {
    verdict_on(c, what, &sn50(), &fp32_twin());
}

/// [`verdict`] on a given chip and its fp32 twin.
fn verdict_on(c: &Case, what: &str, chip: &SpatialTarget, twin_target: &SpatialTarget) {
    let src = c.src.as_str();
    let want = vm_checked(c, what);
    let (twin, _) = composed(src, twin_target, what);
    let got = shown_from_outputs(
        src,
        &twin,
        &eval_choreography(&twin, &inputs_from_fills(&twin).expect("fills"))
            .unwrap_or_else(|e| panic!("{what}: fp32 eval: {e:?}")),
    )
    .expect("shown");
    for (i, (g, v)) in got.iter().zip(&want).enumerate() {
        assert!(
            ((g - v).abs() as f64) <= ((*v as f64).abs() / 65536.0).max(1.0),
            "{what}: WIRING Show #{}: fp32 twin {g} vs VM {v}",
            i + 1
        );
    }
    let (sp, sections) = composed(src, chip, what);
    let got = shown_from_outputs(
        src,
        &sp,
        &eval_choreography(&sp, &inputs_from_fills(&sp).expect("fills"))
            .unwrap_or_else(|e| panic!("{what}: bf16 eval: {e:?}")),
    )
    .expect("shown");
    for (i, ((g, v), (_, m))) in got.iter().zip(&want).zip(&c.reference).enumerate() {
        let bound = ((sections + 1) as f64 * ulp_bf16(*m) * 1e6).max(1.0);
        assert!(
            ((g - v).abs() as f64) <= bound,
            "{what}: CHIP Show #{}: bf16 {g} vs VM {v} (magnitude {m:.4}, bound {bound:.0})",
            i + 1
        );
    }
}

/// The bounds a shape is exercised at: the first position, one inside, and the last the block admits.
fn bounds(s: &Shape) -> [usize; 3] {
    [1, s.p_max().div_ceil(2), s.p_max()]
}

// ───────────────────────────── 1. meaning ─────────────────────────────

#[test]
fn a_causal_block_is_its_rows_attending_in_turn() {
    for s in [FLAT_GQA, FLAT_MHA, FLASH] {
        for p in bounds(&s) {
            let (by_rows, e1) = tiers(&rows(&s, p));
            assert_eq!(e1, None, "{s:?} P={p}: the rows run");
            for bound in [Bound::Literal, Bound::Named] {
                let c = block(&s, p, bound);
                let (by_block, e2) = tiers(&c.src);
                assert_eq!(e2, None, "{s:?} P={p} {bound:?}: the block runs");
                assert_eq!(
                    by_block, by_rows,
                    "{s:?} P={p} {bound:?}: the block is its rows, exactly"
                );
                vm_checked(&c, &format!("{s:?} P={p} {bound:?}"));
            }
        }
        // The rows see different prefixes: the block is not one attention broadcast to every row.
        let c = block(&s, 1, Bound::Named);
        let (out, _) = tiers(&c.src);
        let per_row = s.width().div_ceil(STRIDE);
        assert!(
            out[..per_row] != out[out.len() - per_row..],
            "{s:?}: the first and last rows differ"
        );
    }
}

// ───────────────────────────── 2. refusals ─────────────────────────────

#[test]
fn the_single_query_sugar_refuses_a_query_block_by_name() {
    let s = FLAT_GQA;
    let (q, k, v) = operands(&s);
    let mut src = String::from("## Main\n");
    src += &declare("Q", s.t, s.width(), &q);
    src += &declare("K", s.l, s.kv * s.hd, &k);
    src += &declare("V", s.l, s.kv * s.hd, &v);
    src += &format!("Let C be attention of Q over K and V with {} heads {} groups up to 4.\nShow item 1 of C.\n", s.h, s.kv);
    for (tier, out) in [("VM", vm_outcome(&src)), ("tree-walker", tw_outcome(&src))] {
        let err = format!("{:?}", out.error);
        assert!(out.error.is_some() && err.contains("causal attention"), "{tier}: a {}-row query under the single-query sugar is refused, naming the block form: {err}", s.t);
    }
}

#[test]
fn a_bound_that_puts_a_row_outside_the_cache_fails_with_one_message_on_every_tier() {
    for s in [FLAT_GQA, FLAT_MHA] {
        for bound in [Bound::Literal, Bound::Named] {
            for p in [0usize, s.p_max() + 1] {
                let (_, err) = tiers(&block(&s, p, bound).src);
                let err = err.unwrap_or_else(|| panic!("{s:?} from {p} ({bound:?}) must fail"));
                assert!(
                    err.contains("attention length out of range"),
                    "{s:?} from {p} ({bound:?}): {err}"
                );
            }
            let (_, err) = tiers(&block(&s, s.p_max(), bound).src);
            assert_eq!(
                err, None,
                "{s:?}: a block whose last row is the cache's last position runs"
            );
        }
    }
}

#[test]
fn the_bound_expression_runs_once() {
    let s = FLAT_GQA;
    let (q, k, v) = operands(&s);
    let mut src =
        String::from("## To count (c: Int) -> Int:\n    Show c.\n    Return c.\n\n## Main\n");
    src += &declare("Q", s.t, s.width(), &q);
    src += &declare("K", s.l, s.kv * s.hd, &k);
    src += &declare("V", s.l, s.kv * s.hd, &v);
    src += &format!(
        "Let C be causal attention of Q over K and V with {} heads {} groups from (count(3)).\n",
        s.h, s.kv
    );
    let (out, err) = tiers(&src);
    assert_eq!(err, None);
    assert_eq!(
        out.iter().filter(|l| l.as_str() == "3").count(),
        1,
        "count(3) runs exactly once: {out:?}"
    );
}

// ───────────────────────────── 3. the SN50 computes it ─────────────────────────────

#[test]
fn the_flat_form_lowers_and_computes_the_block() {
    for s in [FLAT_GQA, FLAT_MHA] {
        for bound in [Bound::Literal, Bound::Named] {
            for p in bounds(&s) {
                verdict(
                    &block(&s, p, bound),
                    &format!("flat {s:?} from P={p} ({bound:?})"),
                );
            }
        }
    }
}

#[test]
fn the_flash_form_lowers_and_computes_the_block() {
    // FLASH at P = 31: the rows attend 31, 32 and 33 positions — the last row reaches into the cache's second block.
    for (s, ps) in [
        (FLASH, vec![1usize, 31, FLASH.p_max()]),
        (FLASH_GQA, vec![1, 16, FLASH_GQA.p_max()]),
    ] {
        for bound in [Bound::Literal, Bound::Named] {
            for p in &ps {
                verdict(
                    &block(&s, *p, bound),
                    &format!("flash {s:?} from P={p} ({bound:?})"),
                );
            }
        }
    }
}

#[test]
fn a_computed_bound_outside_the_block_faults_at_its_guard() {
    for (s, what) in [(FLASH, "flash"), (FLAT_GQA, "flat")] {
        let (sp, _) = composed(&block(&s, 1, Bound::Named).src, &sn50(), what);
        for p in [0usize, s.p_max() + 1] {
            let mut bad = sp.clone();
            for t in &mut bad.tensors {
                if t.name == "P" {
                    t.init = TensorInit::Values(vec![p as i64]);
                }
            }
            match eval_choreography(&bad, &inputs_from_fills(&bad).expect("fills")) {
                Err(ChoreoFault::GatherOutOfRange { extent, .. }) => assert_eq!(extent, s.p_max(), "{what} P={p}: checked against the positions the block admits"),
                Err(ChoreoFault::TripOutOfRange { value, admitted, .. }) => {
                    assert_eq!(value, p as f64, "{what} P={p}: the fault carries the bound it refused");
                    assert_eq!(admitted, (1, s.p_max() as i64), "{what} P={p}: checked against the positions the block admits");
                }
                other => panic!("{what} P={p}: a bound that puts a row outside the cache must fault, not {other:?}"),
            }
        }
    }
}

// ───────────────────────────── 4. the prefill cycle ─────────────────────────────

#[test]
fn the_prefill_cycle_appends_every_row_then_attends_the_block() {
    verdict(
        &prefill_cycle(&FLASH, 30),
        "flash prefill cycle from 30 (the rows straddle two cache blocks)",
    );
    verdict(
        &prefill_cycle(&FLASH_GQA, FLASH_GQA.p_max()),
        "flash GQA prefill cycle ending at the cache's last row",
    );
    verdict(
        &prefill_cycle(&FLAT_GQA, 1),
        "flat prefill cycle from the first position",
    );
}

// ───────────────────────────── 5. one attention ─────────────────────────────

#[test]
fn the_block_is_one_attention_whatever_its_rows() {
    for (s, what) in [(FLASH, "flash"), (FLAT_GQA, "flat")] {
        let lowered = |t: usize| {
            composed(
                &block(&Shape { t, ..s }, 1, Bound::Named).src,
                &sn50(),
                what,
            )
            .0
        };
        let (one, two, many) = (lowered(1), lowered(2), lowered(s.t));
        assert_eq!(
            two.sections.len(),
            many.sections.len(),
            "{what}: two rows and {} rows are the same sections",
            s.t
        );
        if what == "flash" {
            let block_section = |p: &SpatialProgram| {
                p.sections
                    .iter()
                    .find(|x| x.name.contains("flash_blocks"))
                    .expect("a flash block section")
                    .clone()
            };
            // ★ ONE ROW WHOSE HEADS SHARE A K/V HEAD IS THE GROUPED DECODE FORM: its fold runs an item per K/V head
            // between two regroupings (`spatial_grouped_decode_attention_lock`). A block of rows keeps an item per query
            // tile and head.
            assert_eq!(
                one.sections.len(),
                many.sections.len() + 2,
                "{what}: one row is the block's fold between two regroupings"
            );
            let grouped = block_section(&one);
            assert_eq!(
                grouped.items.1 - grouped.items.0,
                s.kv,
                "{what}: one row's fold runs an item per K/V head"
            );
            let blocks = block_section(&many);
            assert_eq!(
                blocks.items.1 - blocks.items.0,
                s.t.div_ceil(32) * s.h,
                "the flash block section's work items are the block's query tiles × heads"
            );
            // Rows fill a tile: they add neither items nor per-site stages.
            let per_site = |p: &SpatialProgram| {
                let sec = block_section(p);
                let mut count = std::collections::BTreeMap::new();
                for st in &p.stages[sec.stages.0..sec.stages.1] {
                    *count.entry(st.site).or_insert(0usize) += 1;
                }
                (count.len(), count.values().copied().max().unwrap_or(0))
            };
            assert_eq!(
                per_site(&many),
                per_site(&two),
                "rows within a tile run the same items through the same stages"
            );
        } else {
            assert_eq!(
                one.sections.len(),
                many.sections.len(),
                "{what}: one row and {} rows are the same sections",
                s.t
            );
        }
    }
}

#[test]
fn one_row_through_the_causal_sugar_is_the_decode_form() {
    for s in [Shape { t: 1, ..FLASH }, Shape { t: 1, ..FLAT_GQA }] {
        let causal = block(&s, 5, Bound::Named).src;
        let decode = causal.replace(
            &format!(
                "causal attention of Q over K and V with {} heads {} groups from P",
                s.h, s.kv
            ),
            &format!(
                "attention of Q over K and V with {} heads {} groups up to P",
                s.h, s.kv
            ),
        );
        assert_ne!(causal, decode);
        let names = |src: &str| {
            composed(src, &sn50(), "one row")
                .0
                .stages
                .iter()
                .map(|x| x.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(&causal),
            names(&decode),
            "{s:?}: one row of the block lowers to the decode form's stages"
        );
        assert_eq!(
            tiers(&causal),
            tiers(&decode),
            "{s:?}: and computes the decode form's answer"
        );
    }
}

// ───────────────────────────── 6. every backend ─────────────────────────────

fn norm(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

/// Build and run a native binary in a scratch directory; its stdout, or `None` when it does not build or run.
fn run_native(
    tag: &str,
    files: &[(&str, &str)],
    build: &[&str],
    run: &[&str],
) -> Option<Vec<String>> {
    let dir = std::env::temp_dir().join(format!("logos_causal_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    for (name, text) in files {
        std::fs::write(dir.join(name), text).ok()?;
    }
    let built = std::process::Command::new(build[0])
        .args(&build[1..])
        .current_dir(&dir)
        .output()
        .ok()?;
    let out = built
        .status
        .success()
        .then(|| {
            std::process::Command::new(run[0])
                .args(&run[1..])
                .current_dir(&dir)
                .output()
                .ok()
                .filter(|r| r.status.success())
                .map(|r| norm(&String::from_utf8_lossy(&r.stdout)))
        })
        .flatten();
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn the_block_runs_on_every_backend() {
    for (c, what) in [
        (block(&FLAT_GQA, 3, Bound::Named), "GQA from a computed P"),
        (block(&FLAT_MHA, 2, Bound::Literal), "MHA from a literal P"),
    ] {
        let oracle = norm(&vm_outcome(&c.src).output);
        assert_eq!(
            oracle.len(),
            c.reference.len(),
            "{what}: the VM shows every sampled element"
        );
        assert_eq!(
            norm(&tw_outcome(&c.src).output),
            oracle,
            "{what}: tree-walker"
        );
        if common::require_tool("gcc") {
            let src = compile_to_c(&c.src)
                .unwrap_or_else(|e| panic!("{what}: AOT-C lowers the block: {e:?}"));
            assert_eq!(
                run_native(
                    "c",
                    &[("o.c", &src)],
                    &["gcc", "-O2", "-o", "o", "o.c", "-lm"],
                    &["./o"]
                ),
                Some(oracle.clone()),
                "{what}: AOT-C prints the VM's answer"
            );
        }
        if common::require_tool("riscv64-linux-gnu-gcc") && common::require_tool("qemu-riscv64") {
            let asm = compile_to_riscv(&c.src)
                .unwrap_or_else(|e| panic!("{what}: RISC-V lowers the block: {e:?}"));
            let got = run_native(
                "rv",
                &[("m.s", &asm), ("rt.c", RISCV_RUNTIME_C)],
                &[
                    "riscv64-linux-gnu-gcc",
                    "-O2",
                    "-static",
                    "-o",
                    "m",
                    "m.s",
                    "rt.c",
                    "-lm",
                ],
                &["qemu-riscv64", "./m"],
            );
            assert_eq!(
                got,
                Some(oracle.clone()),
                "{what}: RISC-V under qemu prints the VM's answer"
            );
        }
        compile_to_wasm(&c.src).unwrap_or_else(|e| panic!("{what}: WASM lowers the block: {e:?}"));
        compile_to_cuda(&c.src).unwrap_or_else(|e| {
            panic!("{what}: CUDA lowers the block through the general path: {e:?}")
        });
        assert!(compile_to_cuda_tensor_op(&c.src).is_err(), "{what}: the single-op CUDA path refuses the composite block instead of lowering a fragment");
        compile_to_tenstorrent(&c.src)
            .unwrap_or_else(|e| panic!("{what}: Tenstorrent lowers the block: {e:?}"));
    }
}

// ───────────────────────────── 7. the layout of the ops around it ─────────────────────────────

/// A block of two whole tile rows on the SN50: 64 query rows of one tile-wide head.
const TWO_TILE_ROWS: Shape = Shape {
    t: 64,
    h: 1,
    kv: 1,
    hd: 32,
    l: 80,
};
/// A full tile row of heads eight tiles wide.
const WIDE_TILE_ROW: Shape = Shape {
    t: 32,
    h: 1,
    kv: 1,
    hd: 256,
    l: 40,
};

/// The block between matrix ops: `D = C hadamard G` reads its result as a matrix, and — `dense_q` — its queries are a
/// matmul's result, `Q = X · I` with `I` the identity, so they are exactly the declared rows.
fn between_matrix_ops(s: &Shape, p: usize, dense_q: bool) -> Case {
    let (q, k, v) = operands(s);
    let g = vals(s.t * s.width(), 9);
    let mut src = String::from("## Main\n");
    if dense_q {
        src += &declare("X", s.t, s.width(), &q);
        let identity: Vec<f64> = (0..s.width() * s.width())
            .map(|i| {
                if i / s.width() == i % s.width() {
                    1.0
                } else {
                    0.0
                }
            })
            .collect();
        src += &declare("I", s.width(), s.width(), &identity);
        src += "Let Q be X matmul I.\n";
    } else {
        src += &declare("Q", s.t, s.width(), &q);
    }
    src += &declare("K", s.l, s.kv * s.hd, &k);
    src += &declare("V", s.l, s.kv * s.hd, &v);
    src += &declare("G", s.t, s.width(), &g);
    src += &format!("Let P be {p}.\nLet C be causal attention of Q over K and V with {} heads {} groups from P.\nLet D be C hadamard G.\n", s.h, s.kv);
    src += &shown_indices(s)
        .map(|i| format!("Show round(item {} of D * 1000000).\n", i + 1))
        .collect::<String>();
    let r = reference(&q, &k, &v, s, p);
    Case {
        src,
        reference: shown_indices(s)
            .map(|i| (r[i].0 * g[i], r[i].1 * g[i].abs()))
            .collect(),
    }
}

#[test]
fn the_block_takes_the_layout_of_the_matrix_ops_around_it() {
    verdict(
        &between_matrix_ops(&FLASH, 20, false),
        "flash, the result read as a matrix",
    );
    verdict(
        &between_matrix_ops(&FLASH, 20, true),
        "flash, the queries a matmul's and the result read as a matrix",
    );
    verdict(
        &between_matrix_ops(&FLAT_GQA, 3, false),
        "flat, the result read as a matrix",
    );
    verdict(
        &between_matrix_ops(&TWO_TILE_ROWS, 5, true),
        "two whole tile rows, both edges dense",
    );
    // A tile row of 32 rows of a head eight tiles wide: the relayout's widest column block overruns a site, so it takes
    // the widest one a site holds.
    verdict(
        &between_matrix_ops(&WIDE_TILE_ROW, 3, false),
        "a full tile row of wide heads, the result read as a matrix",
    );
    let (sp, _) = composed(
        &between_matrix_ops(&TWO_TILE_ROWS, 5, true).src,
        &sn50(),
        "two tile rows",
    );
    let relayouts: Vec<&str> = sp
        .sections
        .iter()
        .map(|x| x.name.as_str())
        .filter(|n| n.starts_with("relayout"))
        .collect();
    assert!(relayouts.is_empty(), "a block of query tiles reads the matmul's matrix and writes the matrix the pointwise op reads: {relayouts:?}");
}

#[test]
fn the_flat_block_moves_through_one_relayout_at_each_edge() {
    let c = between_matrix_ops(&FLAT_GQA, 3, true);
    verdict(
        &c,
        "flat, the queries a matmul's and the result read as a matrix",
    );
    let (sp, _) = composed(&c.src, &sn50(), "flat between matrix ops");
    let relayouts: Vec<&str> = sp
        .sections
        .iter()
        .map(|x| x.name.as_str())
        .filter(|n| n.starts_with("relayout"))
        .collect();
    assert_eq!(relayouts.len(), 2, "heads narrower than a tile read and write flat rows: the matmul's queries in, the matrix out: {relayouts:?}");
}

// ───────────────────────────── more query heads than sites ─────────────────────────────

/// The SN50 on a 4 × 4 mesh: sixteen sites, so a block of eighteen query heads puts two on some of them.
fn small_mesh(mut t: SpatialTarget) -> SpatialTarget {
    t.sites = logicaffeine_compile::spatial::target::MeshShape { x: 4, y: 4 };
    t
}

#[test]
fn a_site_holding_several_query_heads_computes_each() {
    let s = Shape {
        t: 9,
        h: 2,
        kv: 1,
        hd: 32,
        l: 40,
    };
    assert!(s.t * s.h > 16);
    let (chip, twin) = (small_mesh(sn50()), small_mesh(fp32_twin()));
    verdict_on(
        &block(&s, 20, Bound::Named),
        "the causal block, 18 heads on 16 sites",
        &chip,
        &twin,
    );
    // A full attention over 33 positions — two cache blocks — with 33 query heads (one per row).
    let (t, h, hd) = (33usize, 1usize, 32usize);
    let (q, k, v) = (
        vals(t * h * hd, 1),
        vals(t * h * hd, 2),
        vals(t * h * hd, 3),
    );
    let mut src = String::from("## Main\n");
    src += &declare("Q", t, h * hd, &q);
    src += &declare("K", t, h * hd, &k);
    src += &declare("V", t, h * hd, &v);
    src += &format!("Let C be full attention of Q over K and V with {h} heads.\n");
    let full = Shape {
        t,
        h,
        kv: h,
        hd,
        l: t,
    };
    src += &shows(&full);
    // Full attention: every row sees every position, so row `r` is its own query's attention over all of them.
    let reference: Vec<(f64, f64)> = shown_indices(&full)
        .map(|i| {
            let row = i / (h * hd);
            mha_row_reference(&q[row * h * hd..(row + 1) * h * hd], &k, &v, h, hd, t)[i % (h * hd)]
        })
        .collect();
    verdict_on(
        &Case { src, reference },
        "a full attention, 33 heads on 16 sites",
        &chip,
        &twin,
    );
}

/// One query row of `h` heads attending every one of `l` positions (MHA): per element, the value and `Σ p·|v|`.
fn mha_row_reference(
    q: &[f64],
    k: &[f64],
    v: &[f64],
    h: usize,
    hd: usize,
    l: usize,
) -> Vec<(f64, f64)> {
    reference(
        q,
        k,
        v,
        &Shape {
            t: 1,
            h,
            kv: h,
            hd,
            l,
        },
        l,
    )
}
