//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! Q8_0 WEIGHTS STAY Q8_0 ON THE CHIP — a decode step reads each weight as one int8 and one scale per 32 rows.
//!
//! A decode step is weight-bound: every weight is read once per token, and a bf16 copy of a Q8_0 checkpoint moves
//! two bytes per weight where the checkpoint holds about one. A weight the deployment stores Q8_0
//! (`compile_program_to_spatial_stored`) is lowered as what it is — `W__q8`, its int8 values, `K × N`, and
//! `W__q8_scale`, one fp32 scale per 32 rows of each column, `K/32 × N` — and a one-row product `x · W` FOLDS its K
//! blocks of 1,024 rows (32 scale rows) into an fp32 carry:
//!   `y[n] += Σ_b s[b, n] · Σ_i x[32b + i] · q[32b + i, n]` — each block's int8 dot rescaled once, exactly.
//! `x` is read as a `K/32 × 32` matrix (one tile per block) — its VIEW, reshaped once per value and shared by every
//! product that reads it (the projections of one normalized row all read one view).
//!
//! Pinned:
//!   1. It computes the VM's product under the ledger's two-part rule (WIRING: the fp32 twin within a relative 2⁻¹⁶;
//!      CHIP: within `sections + 1` bf16 ulps of `Σ |x·w|`).
//!   2. The weight moves as int8 and fp32 scales — `K·N + 4·(K/32)·N` bytes, read once — and at a width where the
//!      weight dominates the product moves at most 0.65× the bytes of the same product over a bf16 weight (1.125
//!      bytes a weight against 2, and the activation's own reads).
//!   3. A width that is not whole 32-column tiles folds over its column tiles, the last padded with zero columns as every
//!      tensor's last tile is (a tensor-parallel chip's 24 of a gate's 48 heads): 40 columns, and 33 — whose pad is
//!      nearly a tile, which the dense weight moves too — each compute the VM's product and read every padded int8 and
//!      every scale once, the bf16 weight never on the chip.
//!   4. A weight declared Q8_0 whose values are not an int8 multiple of a per-32-row scale is refused by name.
//!   5. Two products of one `x` write its view once and both compute; `x` written again between them (a row write,
//!      `Set row pos of x to v`) is a new value, viewed again — two views, and the second product computes `v · U`.
//!   6. A BATCH of rows — `x` an `m × K` matrix, `m` up to a tile's 32 rows (a batched decode step's stacked hidden
//!      states) — folds over the same stored blocks: it computes the VM's product for every row and reads every int8
//!      and every scale ONCE for all its rows. Each column group reads the batch's view once, so at a batch of 8 the
//!      product moves at most 0.75× the bytes over a bf16 weight. Where the view's re-reads would outweigh what the
//!      int8 values save (16 rows over 32 columns, 32 rows over 64) and past a tile's rows (33), the weight is read
//!      dense — and still computes.
//!   7. A K that is not whole blocks of 1,024 — a tensor-parallel chip's rows of a row-split weight (768, 2,176) —
//!      folds its whole blocks and then its TAIL (the last `K mod 1024` rows, whole Q8_0 blocks): it computes the VM's
//!      product for one row and a batch, and still reads every int8 and every scale once.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_q8_weights_lock`
//!
//! ★ MUTATION PROBES (each run 2026-09-27 in a private copy — break, observe red; logs `logs/sn50/probe_q8_*.txt`):
//!   • the quantizer accepts a block that is not an int8 multiple of its scale → the perturbed weight lowers, and the
//!     refusal test reds;
//!   • a block's stage drops the scale (`y += Σ_b Σ_i x·q`; re-run on the one-stage-a-row form, same figures) → the
//!     product reds on WIRING (`808968750` vs VM `15433594`);
//!   • every K block reads the scales' first tile row → the two-block product reds on WIRING (`15373108` vs VM
//!     `17652100`); the one-block product stays green — only a second block can see it;
//!   • the stored set ignored → the weight is read dense (`W` on the chip, no `W__q8`) and the perturbed weight lowers:
//!     the traffic and refusal tests red;
//!   • the view never shared (every product reshapes `x` itself) → two writers of one activation, refused at
//!     composition (`OutputOverlap` on the view) — the sharing test reds;
//!   • the view named for its variable, not its value → the row-written `x` is never viewed again (no `x__q8_rows1`)
//!     — the sharing test reds;
//!   • every batch row's block stage reads row 0's view tile → the batch product reds on WIRING (`1554871` vs VM
//!     `4043701`, row 1);
//!   • every row's block sum folded into every row of the carry → the batch product reds on WIRING (`8509277` vs VM
//!     `1554871`, row 0);
//!   • the batch view's lane table drops the row (`1024·ρ`) → every row is row 0 — the batch product reds;
//!   • the fold taken whatever it moves → the batch of 16 folds (`W__q8` on the chip) — the dense-fallback test reds;
//!   • a batch never folded → the traffic and product tests red (the bf16 weight on the chip);
//!   • the tail's values read at the first block's rows → `2176×64` WIRING Show #1: `-2891418` vs VM `2734375`; its
//!     scales at the first scale tile row → `3199158`; its view at the first K block → `-9532471`;
//!   • the tail left out of the result → a tail-only product (768) has nothing to compute and the lowering stops;
//!   • only whole K blocks fold (the tail's applicability dropped) → the 768 product reads its weight dense;
//!   • a batch view written in two parts planned as whole-view rows (`grid: (blocks·chunks, 1)`) → the two-block batch
//!     tail is refused (`GridMismatch { expected: (1, 2), found: (2, 1) }`) — the served deployment's down projection
//!     failed so before this case existed (one whole block made both conventions `(1, 1)`).
//! ════════════════════════════════════════════════════════════════════════════════════════════════
#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use std::collections::BTreeSet;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::spatial::eval::eval_choreography;
use logicaffeine_compile::spatial::ir::{ChoreoOp, Phase, SpatialProgram, TaggedOp};
use logicaffeine_compile::spatial::pipeline::{
    compile_program_to_spatial, compile_program_to_spatial_stored,
};
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use logicaffeine_compile::spatial::work::work_of;
use tensor_probes::{inputs_from_fills, oracle_ints, shown_from_outputs};

fn sn50() -> SpatialTarget {
    SpatialTarget::sn50_rdu()
}

/// The SN50 with fp32 datapaths: fp32 in place of bf16 on the matmul, fp32 beside it on the pointwise and row units.
fn fp32_twin() -> SpatialTarget {
    let mut t = sn50();
    for c in &mut t.compute {
        match c {
            ComputeCap::MatmulTile { precisions, .. } => {
                if precisions.remove(&Precision::Bf16) {
                    precisions.insert(Precision::Fp32);
                }
            }
            ComputeCap::Elementwise { precisions } | ComputeCap::RowReduce { precisions } => {
                if precisions.contains(&Precision::Bf16) {
                    precisions.insert(Precision::Fp32);
                }
            }
            _ => {}
        }
    }
    t.precisions.insert(Precision::Fp32);
    t
}

fn stored() -> BTreeSet<String> {
    BTreeSet::from(["W".to_string()])
}

/// An activation value exact in bf16: a multiple of 1/64 in `[-1, 1]`.
fn xval(p: usize) -> f64 {
    (((p * 7919 + 13) % 129) as f64 - 64.0) / 64.0
}

/// A Q8_0 weight, `k × n`: per column and 32-row block a scale `d` (a small multiple of 1/256) and int8 values with
/// one of them ±127, so the per-block scale `max|w| / 127` recovers `d` exactly.
fn q8_weight(k: usize, n: usize) -> Vec<f64> {
    q8_weight_seeded(k, n, 0)
}

/// [`q8_weight`] with its values and scales shifted by `seed` — another weight of the same kind.
fn q8_weight_seeded(k: usize, n: usize, seed: usize) -> Vec<f64> {
    let mut w = vec![0.0; k * n];
    for b in 0..k / 32 {
        for col in 0..n {
            let d = (1 + (b + col + seed) % 3) as f64 / 256.0;
            for i in 0..32 {
                let q = if i == (b + col) % 32 {
                    if (b + col) % 2 == 0 {
                        127
                    } else {
                        -127
                    }
                } else {
                    ((i * 37 + b * 13 + col * 5 + seed * 11) % 255) as i64 - 127
                };
                w[(32 * b + i) * n + col] = q as f64 * d;
            }
        }
    }
    w
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

/// A shown sample of the product's columns.
const STRIDE: usize = 7;

struct Case {
    src: String,
    /// Per `Show`: the f64 value and its magnitude `Σ_k |x_k·w_kn|`.
    reference: Vec<(f64, f64)>,
}

fn gemv(k: usize, n: usize, w: &[f64]) -> Case {
    let x: Vec<f64> = (0..k).map(xval).collect();
    let mut src = String::from("## Main\n");
    src += &declare("x", 1, k, &x);
    src += &declare("W", k, n, w);
    src += "Let y be x matmul W.\n";
    let shown: Vec<usize> = (0..n).step_by(STRIDE).collect();
    for c in &shown {
        src += &format!("Show round(item {} of y * 1000000).\n", c + 1);
    }
    let reference = shown
        .iter()
        .map(|c| {
            (
                (0..k).map(|r| x[r] * w[r * n + c]).sum(),
                (0..k).map(|r| (x[r] * w[r * n + c]).abs()).sum(),
            )
        })
        .collect();
    Case { src, reference }
}

fn lines(s: &str) -> Vec<String> {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

fn composed(
    src: &str,
    target: &SpatialTarget,
    q8: &BTreeSet<String>,
    what: &str,
) -> (SpatialProgram, usize) {
    let pl = compile_program_to_spatial_stored(src, target, q8)
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

fn verdict(c: &Case, q8: &BTreeSet<String>, what: &str) {
    let src = c.src.as_str();
    let (tw, vm) = (tw_outcome(src), vm_outcome(src));
    assert_eq!(
        (lines(&tw.output), &tw.error),
        (lines(&vm.output), &vm.error),
        "{what}: the VM and the tree-walker agree"
    );
    assert_eq!(vm.error, None, "{what}: {:?}", vm.error);
    let want = oracle_ints(&vm.output).expect("ints");
    for (i, (v, (r, _))) in want.iter().zip(&c.reference).enumerate() {
        assert!(
            ((*v as f64) - (r * 1e6).round()).abs() <= 1.0,
            "{what}: Show #{}: VM {v} vs f64 reference {}",
            i + 1,
            r * 1e6
        );
    }
    let (twin, _) = composed(src, &fp32_twin(), q8, what);
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
    let (sp, sections) = composed(src, &sn50(), q8, what);
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
            "{what}: CHIP Show #{}: {g} vs VM {v} (magnitude {m:.4}, bound {bound:.0})",
            i + 1
        );
    }
}

/// Bytes of `tensor` the schedule reads, every loop's count multiplied in.
fn bytes_read(p: &SpatialProgram, tensor: &str) -> u64 {
    bytes_moved(p, tensor, false)
}

/// Bytes of `tensor` the schedule writes, every loop's count multiplied in.
fn bytes_written(p: &SpatialProgram, tensor: &str) -> u64 {
    bytes_moved(p, tensor, true)
}

fn bytes_moved(p: &SpatialProgram, tensor: &str, writes: bool) -> u64 {
    fn walk(
        p: &SpatialProgram,
        ops: &[TaggedOp],
        mult: u64,
        tensor: &str,
        writes: bool,
        n: &mut u64,
    ) {
        let tile = (p.tile.rows * p.tile.cols) as u64;
        for t in ops {
            let moved = match &t.op {
                ChoreoOp::Loop { count, body, .. } => {
                    walk(p, body, mult * count.bound() as u64, tensor, writes, n);
                    None
                }
                ChoreoOp::ReadTiles {
                    tensor: id,
                    block_h,
                    block_w,
                    ..
                } if !writes => Some((*id, *block_h * *block_w)),
                ChoreoOp::WriteTiles {
                    tensor: id,
                    block_h,
                    block_w,
                    ..
                } if writes => Some((*id, *block_h * *block_w)),
                _ => None,
            };
            if let Some((id, tiles)) = moved.filter(|(id, _)| p.tensor(*id).name == tensor) {
                *n += mult
                    * tiles as u64
                    * tile
                    * logicaffeine_compile::spatial::target::precision_bytes(p.tensor(id).dtype)
                        as u64;
            }
        }
    }
    let mut n = 0;
    for prog in &p.schedule.as_ref().expect("scheduled").programs {
        walk(p, &prog.ops, 1, tensor, writes, &mut n);
    }
    n
}

// ───────────────────────────── 1. it computes the product ─────────────────────────────

#[test]
fn a_q8_weight_gemv_computes_the_vms_product() {
    for (k, n) in [(1024usize, 64usize), (2048, 128)] {
        verdict(
            &gemv(k, n, &q8_weight(k, n)),
            &stored(),
            &format!("x·W, {k}×{n}, W stored Q8_0"),
        );
    }
}

// ───────────────────────────── 2. it moves int8 and scales ─────────────────────────────

#[test]
fn the_weight_moves_as_int8_and_one_scale_per_32_rows() {
    // A weight the program never sets is all zeros — Q8_0 with every scale 0 — so a wide product costs no source.
    let (k, n) = (2048usize, 1024usize);
    let c = gemv(k, n, &vec![0.0; k * n]);
    let (q, _) = composed(&c.src, &sn50(), &stored(), "stored");
    let tensor = |name: &str| {
        q.tensors
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| {
                panic!(
                    "a tensor {name}: {:?}",
                    q.tensors.iter().map(|t| &t.name).collect::<Vec<_>>()
                )
            })
    };
    let (values, scales) = (tensor("W__q8"), tensor("W__q8_scale"));
    assert_eq!(
        (values.elems.rows, values.elems.cols, values.dtype),
        (k, n, Precision::Int8),
        "the int8 values"
    );
    assert_eq!(
        (scales.elems.rows, scales.elems.cols, scales.dtype),
        (k / 32, n, Precision::Fp32),
        "one fp32 scale per 32 rows of each column"
    );
    assert!(
        q.tensors.iter().all(|t| t.name != "W"),
        "the bf16 weight is never on the chip"
    );
    assert_eq!(
        bytes_read(&q, "W__q8"),
        (k * n) as u64,
        "every int8 read once"
    );
    assert_eq!(
        bytes_read(&q, "W__q8_scale"),
        (4 * (k / 32) * n) as u64,
        "every scale read once"
    );
    let dense = compile_program_to_spatial(&c.src, &sn50())
        .expect("walk")
        .whole()
        .expect("the dense product lowers")
        .clone();
    let (stored_bytes, dense_bytes) = (
        work_of(&q, &sn50()).dram_bytes,
        work_of(&dense, &sn50()).dram_bytes,
    );
    assert!(
        100 * stored_bytes <= 65 * dense_bytes,
        "the stored product moves {stored_bytes} bytes against {dense_bytes} over a bf16 weight"
    );
}

// ───────────────────────────── 3. a width of part tiles folds over its padded tiles ─────────────────────────────

#[test]
fn a_width_of_part_tiles_folds_over_its_padded_tiles_and_computes() {
    for n in [40usize, 33] {
        let k = 1024usize;
        let padded = n.div_ceil(32) * 32;
        let c = gemv(k, n, &q8_weight(k, n));
        let what = format!("N of {n}");
        let (q, _) = composed(&c.src, &sn50(), &stored(), &what);
        let tensor = |name: &str| q.tensors.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("{what}: a tensor {name}"));
        let (values, scales) = (tensor("W__q8"), tensor("W__q8_scale"));
        assert_eq!((values.elems.rows, values.elems.cols, values.tiles.cols_t), (k, n, padded / 32), "{what}: the int8 values over their padded tiles");
        assert_eq!((scales.elems.rows, scales.elems.cols, scales.tiles.cols_t), (k / 32, n, padded / 32), "{what}: the scales over theirs");
        assert!(q.tensors.iter().all(|t| t.name != "W"), "{what}: the bf16 weight is never on the chip");
        assert_eq!(bytes_read(&q, "W__q8"), (k * padded) as u64, "{what}: every padded int8 read once");
        assert_eq!(bytes_read(&q, "W__q8_scale"), (4 * (k / 32) * padded) as u64, "{what}: every padded scale read once");
        verdict(&c, &stored(), &format!("x·W, {k}×{n}, W stored Q8_0 over padded tiles"));
    }
}

// ───────────────────────────── 4. a weight that is not Q8_0 is refused ─────────────────────────────

#[test]
fn a_weight_declared_q8_whose_values_are_not_is_refused_by_name() {
    let (k, n) = (1024usize, 64usize);
    let mut w = q8_weight(k, n);
    w[5 * n + 3] += 1.0 / 1024.0;
    let c = gemv(k, n, &w);
    let pl = compile_program_to_spatial_stored(&c.src, &sn50(), &stored()).expect("walk");
    assert!(
        pl.whole().is_none(),
        "a weight that is not Q8_0 is not lowered as one"
    );
    let why = pl.refusal();
    assert!(
        why.contains("Q8_0") && why.contains('W'),
        "the refusal names the weight and its storage: {why}"
    );
}

// ───────────────────────────── the stored tensors name their weight ─────────────────────────────

#[test]
fn the_stored_tensors_name_their_weight_and_the_part_the_host_repacks() {
    use logicaffeine_compile::spatial::passes::from_q8::{stored_part, Q8Part};
    let (k, n) = (1024usize, 64usize);
    let (q, _) = composed(
        &gemv(k, n, &vec![0.0; k * n]).src,
        &sn50(),
        &stored(),
        "stored",
    );
    let parts: Vec<(String, (&str, Q8Part))> = q
        .tensors
        .iter()
        .filter_map(|t| stored_part(&t.name).map(|p| (t.name.clone(), p)))
        .collect();
    assert_eq!(
        parts.iter().map(|(_, p)| *p).collect::<Vec<_>>(),
        vec![("W", Q8Part::Values), ("W", Q8Part::Scales)],
        "the values and the scales of `W`: {parts:?}"
    );
    assert_eq!(
        (Q8Part::Values.repack(), Q8Part::Scales.repack()),
        ("q8_0.values", "q8_0.scales"),
        "the manifest's names for the two repacks"
    );
    assert!(
        q.tensors
            .iter()
            .filter(|t| stored_part(&t.name).is_none())
            .all(|t| t.name != "W"),
        "no dense copy beside them"
    );
}

// ───────────────────────────── 5. one view of a value, however many products read it ─────────────────────────────

/// `y = x·W` and `z = x·U`, both weights stored Q8_0 — with `rewrite`, `x`'s one row is written with `v` between
/// them, so `z` is `v · U`.
fn two_products(k: usize, n: usize, rewrite: bool) -> Case {
    let x: Vec<f64> = (0..k).map(xval).collect();
    let v: Vec<f64> = (0..k).map(|p| xval(p + 17)).collect();
    let (w, u) = (q8_weight_seeded(k, n, 0), q8_weight_seeded(k, n, 1));
    let mut src = String::from("## Main\n");
    src += &declare("x", 1, k, &x);
    src += &declare("v", 1, k, &v);
    src += &declare("W", k, n, &w);
    src += &declare("U", k, n, &u);
    src += "Let pos be 1.\nLet y be x matmul W.\n";
    if rewrite {
        src += "Set row pos of x to v.\n";
    }
    src += "Let z be x matmul U.\n";
    let later = if rewrite { &v } else { &x };
    let shown: Vec<usize> = (0..n).step_by(STRIDE).collect();
    let mut reference = Vec::new();
    for (out, a, wt) in [("y", &x, &w), ("z", later, &u)] {
        for c in &shown {
            src += &format!("Show round(item {} of {out} * 1000000).\n", c + 1);
            let term = |r: usize| a[r] * wt[r * n + c];
            reference.push(((0..k).map(term).sum(), (0..k).map(|r| term(r).abs()).sum()));
        }
    }
    Case { src, reference }
}

fn both_stored() -> BTreeSet<String> {
    BTreeSet::from(["W".to_string(), "U".to_string()])
}

/// Bytes of every tile of `tensor`, once.
fn every_tile_once(p: &SpatialProgram, tensor: &str) -> u64 {
    let t = p
        .tensors
        .iter()
        .find(|t| t.name == tensor)
        .unwrap_or_else(|| panic!("a tensor {tensor}"));
    let (tr, tc) = (p.tile.rows, p.tile.cols);
    (t.elems.rows.div_ceil(tr) * t.elems.cols.div_ceil(tc) * tr * tc) as u64
        * logicaffeine_compile::spatial::target::precision_bytes(t.dtype) as u64
}

#[test]
fn every_product_of_one_value_reads_one_view_of_it() {
    use logicaffeine_compile::spatial::passes::from_q8::q8_rows_name;
    let (k, n) = (2048usize, 64usize);
    let view = q8_rows_name("x", 0);
    let shared = two_products(k, n, false);
    let (q, _) = composed(&shared.src, &sn50(), &both_stored(), "two products of x");
    assert!(
        q.tensors.iter().any(|t| t.name == "W__q8") && q.tensors.iter().any(|t| t.name == "U__q8"),
        "both weights stored Q8_0"
    );
    assert_eq!(
        bytes_written(&q, &view),
        every_tile_once(&q, &view),
        "the view of `x` is made once, however many products read it"
    );
    assert_eq!(
        bytes_read(&q, &view),
        2 * every_tile_once(&q, &view),
        "and each product reads it once"
    );
    verdict(&shared, &both_stored(), "x·W and x·U over one view of x");

    let rewritten = two_products(k, n, true);
    let (q, _) = composed(
        &rewritten.src,
        &sn50(),
        &both_stored(),
        "x written between the products",
    );
    let again = q8_rows_name("x", 1);
    assert_ne!(
        view, again,
        "the written value's view has a name of its own"
    );
    assert_eq!(
        bytes_written(&q, &view),
        every_tile_once(&q, &view),
        "the first value's view is made once"
    );
    assert_eq!(
        bytes_written(&q, &again),
        every_tile_once(&q, &again),
        "a value written again is viewed again"
    );
    assert_eq!(
        (bytes_read(&q, &view), bytes_read(&q, &again)),
        (every_tile_once(&q, &view), every_tile_once(&q, &again)),
        "each product reads the view of the value it multiplies"
    );
    verdict(
        &rewritten,
        &both_stored(),
        "x·W, then row 1 of x ← v, then x·U",
    );
}

// ───────────────────────────── 6. a batch of rows reads the weight once ─────────────────────────────

/// `y = x · W` for an `m × k` matrix `x` of bf16-exact rows and a declared `W`; shows a sample of each row's columns.
fn batch(m: usize, k: usize, n: usize, w: &[f64]) -> Case {
    let x: Vec<f64> = (0..m * k).map(|p| xval(p + 5)).collect();
    let mut src = String::from("## Main\n");
    src += &declare("x", m, k, &x);
    src += &declare("W", k, n, w);
    src += "Let y be x matmul W.\n";
    let mut reference = Vec::new();
    for r in 0..m {
        for c in (0..n).step_by(STRIDE) {
            src += &format!("Show round(item {} of y * 1000000).\n", r * n + c + 1);
            let term = |i: usize| x[r * k + i] * w[i * n + c];
            reference.push(((0..k).map(term).sum(), (0..k).map(|i| term(i).abs()).sum()));
        }
    }
    Case { src, reference }
}

#[test]
fn a_batch_of_rows_computes_the_vms_product() {
    for (m, k, n) in [(3usize, 1024usize, 64usize), (8, 2048, 64)] {
        let c = batch(m, k, n, &q8_weight_seeded(k, n, m));
        let (q, _) = composed(&c.src, &sn50(), &stored(), "a batch");
        assert!(
            q.tensors.iter().any(|t| t.name == "W__q8") && q.tensors.iter().all(|t| t.name != "W"),
            "{m} rows: the weight is read as stored"
        );
        verdict(
            &c,
            &stored(),
            &format!("{m} rows · W, {k}×{n}, W stored Q8_0"),
        );
    }
}

#[test]
fn a_batch_reads_each_weight_once_for_all_its_rows() {
    let (m, k, n) = (8usize, 2048usize, 1024usize);
    let c = batch(m, k, n, &vec![0.0; k * n]);
    let (q, _) = composed(&c.src, &sn50(), &stored(), "a batch of 8");
    assert!(
        q.tensors.iter().all(|t| t.name != "W"),
        "the bf16 weight is never on the chip"
    );
    assert_eq!(
        bytes_read(&q, "W__q8"),
        (k * n) as u64,
        "every int8 read once for all {m} rows"
    );
    assert_eq!(
        bytes_read(&q, "W__q8_scale"),
        (4 * (k / 32) * n) as u64,
        "every scale read once for all {m} rows"
    );
    let y = q
        .tensors
        .iter()
        .find(|t| t.name == "y")
        .expect("the product");
    assert_eq!(
        (y.elems.rows, y.elems.cols),
        (m, n),
        "the product is the batch's {m} rows"
    );
    let view = logicaffeine_compile::spatial::passes::from_q8::q8_rows_name("x", 0);
    let fold = q
        .sections
        .iter()
        .find(|s| s.name.starts_with("q8_gemm"))
        .unwrap_or_else(|| {
            panic!(
                "a batched fold: {:?}",
                q.sections.iter().map(|s| &s.name).collect::<Vec<_>>()
            )
        });
    let groups = (fold.items.1 - fold.items.0) as u64;
    assert_eq!(
        bytes_read(&q, &view),
        groups * every_tile_once(&q, &view),
        "each of the {groups} column groups reads the batch's view once"
    );
    let dense = compile_program_to_spatial(&c.src, &sn50())
        .expect("walk")
        .whole()
        .expect("the dense product lowers")
        .clone();
    let (stored_bytes, dense_bytes) = (
        work_of(&q, &sn50()).dram_bytes,
        work_of(&dense, &sn50()).dram_bytes,
    );
    assert!(
        100 * stored_bytes <= 75 * dense_bytes,
        "the stored batch moves {stored_bytes} bytes against {dense_bytes} over a bf16 weight"
    );
}

#[test]
fn a_batch_the_fold_would_not_win_reads_the_weight_dense_and_computes() {
    // A column group reads the batch's `m` view tiles per K block against its `1.125·32·wb` weight bytes: one group of 32
    // columns at 16 rows, and the widest group a site holds at 32 rows, read more of the view than the int8 values save.
    for (m, n, why) in [
        (
            16usize,
            32usize,
            "the view's re-reads outweigh the int8 saving",
        ),
        (32, 64, "the view's re-reads outweigh the int8 saving"),
        (33, 64, "more rows than a tile"),
    ] {
        let k = 1024usize;
        let c = batch(m, k, n, &q8_weight(k, n));
        let (q, _) = composed(&c.src, &sn50(), &stored(), why);
        assert!(
            q.tensors.iter().any(|t| t.name == "W") && q.tensors.iter().all(|t| t.name != "W__q8"),
            "{m} rows — {why}: the weight is read dense"
        );
        verdict(
            &c,
            &stored(),
            &format!("{m} rows · W, {k}×{n}, W stored Q8_0 but dense on the chip ({why})"),
        );
    }
}

// ───────────────────────────── 7. a K that is not whole blocks folds its tail ─────────────────────────────

#[test]
fn a_k_that_is_not_whole_blocks_folds_its_tail_and_computes() {
    // A tail alone (768 = 24 blocks, the per-chip rows of a row-split output projection), two whole blocks and a tail of
    // four (2,176, the per-chip rows of a row-split down projection), and under a batch one whole block and a tail of two
    // and two whole blocks and a tail (the batch's view then written in two parts — a served deployment's down
    // projection).
    for (m, k, n) in [
        (1usize, 768usize, 64usize),
        (1, 2176, 64),
        (3, 1088, 64),
        (3, 2176, 64),
    ] {
        let c = if m == 1 {
            gemv(k, n, &q8_weight_seeded(k, n, k))
        } else {
            batch(m, k, n, &q8_weight_seeded(k, n, k))
        };
        let what = format!("{m} rows · W, {k}×{n}, W stored Q8_0");
        let (q, _) = composed(&c.src, &sn50(), &stored(), &what);
        assert!(
            q.tensors.iter().any(|t| t.name == "W__q8") && q.tensors.iter().all(|t| t.name != "W"),
            "{what}: the weight is read as stored"
        );
        assert_eq!(
            bytes_read(&q, "W__q8"),
            (k * n) as u64,
            "{what}: every int8 read once"
        );
        verdict(&c, &stored(), &what);
    }
}
