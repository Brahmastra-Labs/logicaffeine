//! Attention at REAL widths reaches the SN50 on the program path and computes what the VM computes — at any
//! cache length (decode) and any number of query rows (full attention).
//!
//! A head wider than a site's accumulator budget (Qwen3.8: 24 heads × 256) lowers through the BLOCKED form,
//! one head per work item. That form is what every real attention layer of the model takes, so it must be
//! right for the lengths a model actually runs: a decode step attends over every cached position, and a
//! vision block attends every row to every row.
//!
//! Every case here must lower whole, certify, and evaluate within the program-path ledger's two-part rule:
//!   – WIRING: the same source lowered for an SN50 descriptor that also offers Fp32 agrees with the VM to a
//!     relative 2⁻¹⁶ (a wrong dataflow is off by O(1));
//!   – CHIP PRECISION: the bf16 program is within `sections + 1` bf16 ulps — ulps of the MAGNITUDE the output is
//!     summed from, `Σ_j p_j·|v_j|`, which is the forward-error model of a dot product. An output whose terms
//!     cancel toward zero is only ever as exact as the terms it cancelled, never as exact as its own ulp.
//!
//! The magnitudes come from an attention written out here in f64 — a third oracle, checked against the VM too.
//!
//! ★ MUTATION PROBE (run 2026-09-24 — break, observe red, restore byte-identical): the flash block's skip mask
//! ignored (every position of every block counted) → `blocked_decode_attention_is_right_at_every_cache_length`
//! reds at `l32`: `WIRING Show #1: fp32 twin -413435 vs VM -418277` — the padded empty blocks and the overlapping
//! last block are counted as real positions.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_attention_real_width_lock`
#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::eval_choreography;
use logicaffeine_compile::spatial::ir::{Phase, SpatialProgram};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use tensor_probes::{inputs_from_fills, oracle_ints, shown_from_outputs, shown_tensor, shows_of};

fn sn50() -> SpatialTarget {
    SpatialTarget::sn50_rdu()
}

/// SN50 with Fp32 wherever it offers a float — the compiler's own wide twin of the same lowering.
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

/// A deterministic value in `[-1, 1)` with two decimals — exact in the source text, varied enough that a
/// wrong position, head or row shows.
fn val(seed: usize) -> f64 {
    (((seed * 7919 + 13) % 200) as f64 - 100.0) / 100.0
}

fn declare(name: &str, rows: usize, cols: usize, salt: usize) -> String {
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of Real.\n");
    for p in 0..rows * cols {
        s += &format!(
            "Set item {} of {name} to {:?}.\n",
            p + 1,
            val(p * 31 + salt)
        );
    }
    s
}

/// Show every `stride`-th element of `out` (`n` of them), rounded ×1e6.
fn shows(out: &str, n: usize, stride: usize) -> String {
    (0..n)
        .step_by(stride.max(1))
        .map(|p| format!("Show round(item {} of {out} * 1000000).\n", p + 1))
        .collect()
}

/// The `n` values `declare(_, _, _, salt)` writes.
fn vals(n: usize, salt: usize) -> Vec<f64> {
    (0..n).map(|p| val(p * 31 + salt)).collect()
}

/// One attention head in f64: query `q` (`hd`), keys and values read through `at(j, e)` over `l` positions.
/// Returns, per output element `e`, the attention `Σ p_j·v_j[e]` and its magnitude `Σ p_j·|v_j[e]|`.
fn head_ref(
    q: &[f64],
    k: &dyn Fn(usize, usize) -> f64,
    v: &dyn Fn(usize, usize) -> f64,
    l: usize,
    hd: usize,
) -> Vec<(f64, f64)> {
    let scores: Vec<f64> = (0..l)
        .map(|j| (0..hd).map(|e| q[e] * k(j, e)).sum::<f64>() / (hd as f64).sqrt())
        .collect();
    let m = scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let ex: Vec<f64> = scores.iter().map(|s| (s - m).exp()).collect();
    let z: f64 = ex.iter().sum();
    (0..hd)
        .map(|e| {
            (
                (0..l).map(|j| ex[j] / z * v(j, e)).sum(),
                (0..l).map(|j| ex[j] / z * v(j, e).abs()).sum(),
            )
        })
        .collect()
}

/// A case: its source, and per `Show`, the reference attention value and its magnitude.
struct Case {
    src: String,
    reference: Vec<(f64, f64)>,
}

/// Decode attention: one query row of `h` heads over an `l`-position cache of `kv` heads.
fn decode(h: usize, kv: usize, hd: usize, l: usize) -> Case {
    let mut s = String::from("## Main\n");
    s += &declare("Q", 1, h * hd, 1);
    s += &declare("K", l, kv * hd, 2);
    s += &declare("V", l, kv * hd, 3);
    s += &format!("Let C be attention of Q over K and V with {h} heads {kv} groups.\n");
    s += &shows("C", h * hd, 7);
    let (q, k, v) = (vals(h * hd, 1), vals(l * kv * hd, 2), vals(l * kv * hd, 3));
    let group = h / kv;
    let mut c = Vec::new();
    for head in 0..h {
        let kvh = head / group;
        let at = |x: &[f64], j: usize, e: usize| x[j * kv * hd + kvh * hd + e];
        c.extend(head_ref(
            &q[head * hd..(head + 1) * hd],
            &|j, e| at(&k, j, e),
            &|j, e| at(&v, j, e),
            l,
            hd,
        ));
    }
    Case {
        src: s,
        reference: (0..h * hd).step_by(7).map(|p| c[p]).collect(),
    }
}

/// Full attention: `t` rows of `h` heads, every row attending every row.
fn full(t: usize, h: usize, hd: usize) -> Case {
    let mut s = String::from("## Main\n");
    s += &declare("Q", t, h * hd, 4);
    s += &declare("K", t, h * hd, 5);
    s += &declare("V", t, h * hd, 6);
    s += &format!("Let C be full attention of Q over K and V with {h} heads.\n");
    s += &shows("C", t * h * hd, 11);
    let (q, k, v) = (
        vals(t * h * hd, 4),
        vals(t * h * hd, 5),
        vals(t * h * hd, 6),
    );
    let mut c = Vec::new();
    for r in 0..t {
        for x in 0..h {
            let at = |m: &[f64], j: usize, e: usize| m[j * h * hd + x * hd + e];
            let base = r * h * hd + x * hd;
            c.extend(head_ref(
                &q[base..base + hd],
                &|j, e| at(&k, j, e),
                &|j, e| at(&v, j, e),
                t,
                hd,
            ));
        }
    }
    Case {
        src: s,
        reference: (0..t * h * hd).step_by(11).map(|p| c[p]).collect(),
    }
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

/// The two-part verdict of the program-path ledger, on one case.
fn verdict(case: &Case, what: &str) {
    let src = case.src.as_str();
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "{what}: VM: {:?}", vm.error);
    let want = oracle_ints(&vm.output).expect("vm ints");
    // The third oracle: the VM computes the attention written out above.
    assert_eq!(
        want.len(),
        case.reference.len(),
        "{what}: every shown value has a reference"
    );
    for (i, (v, (r, _))) in want.iter().zip(&case.reference).enumerate() {
        assert!(
            ((*v as f64) - (r * 1e6).round()).abs() <= 1.0,
            "{what}: Show #{}: VM {v} vs the f64 reference {}",
            i + 1,
            r * 1e6
        );
    }
    // WIRING: the Fp32 twin to a relative 2⁻¹⁶.
    let (twin, _) = composed(src, &fp32_twin(), what);
    let out = eval_choreography(&twin, &inputs_from_fills(&twin).expect("fills"))
        .unwrap_or_else(|e| panic!("{what}: fp32 eval: {e:?}"));
    let got = shown_from_outputs(src, &twin, &out).expect("shown");
    assert_eq!(got.len(), want.len(), "{what}: every shown value");
    for (i, (g, v)) in got.iter().zip(&want).enumerate() {
        let bound = ((*v as f64).abs() / 65536.0).max(1.0);
        assert!(
            ((g - v).abs() as f64) <= bound,
            "{what}: WIRING Show #{}: fp32 twin {g} vs VM {v}",
            i + 1
        );
    }
    // CHIP PRECISION: bf16 within `sections + 1` ulps.
    let (sp, sections) = composed(src, &sn50(), what);
    let out = eval_choreography(&sp, &inputs_from_fills(&sp).expect("fills"))
        .unwrap_or_else(|e| panic!("{what}: bf16 eval: {e:?}"));
    let got = shown_from_outputs(src, &sp, &out).expect("shown");
    let dtype = shown_tensor(&sp, &shows_of(src)[0].tensor)
        .expect("shown tensor")
        .dtype;
    assert_eq!(
        dtype,
        Precision::Bf16,
        "{what}: the chip program carries bf16"
    );
    for (i, ((g, v), (_, magnitude))) in got.iter().zip(&want).zip(&case.reference).enumerate() {
        let bound = ((sections + 1) as f64 * ulp_bf16(*magnitude) * 1e6).max(1.0);
        assert!(((g - v).abs() as f64) <= bound, "{what}: CHIP Show #{}: bf16 {g} vs VM {v} (magnitude {magnitude:.4}, bound {bound:.0})", i + 1);
    }
}

#[test]
fn blocked_decode_attention_is_right_at_every_cache_length() {
    // 4 heads × 128 is wider than a site's accumulator budget: the blocked form.
    for l in [1, 2, 4, 32, 33, 100] {
        verdict(&decode(4, 2, 128, l), &format!("decode h4 kv2 hd128 l{l}"));
    }
}

#[test]
fn full_attention_at_blocking_width_is_right_for_every_row_count() {
    for t in [1, 3, 40] {
        verdict(&full(t, 2, 128), &format!("full t{t} h2 hd128"));
    }
}

#[test]
fn qwen38_attention_geometry_over_a_real_cache() {
    // Qwen3.8's attention layer: 24 query heads, 4 KV groups, head dim 256 — over a 40-position cache.
    verdict(
        &decode(24, 4, 256, 40),
        "qwen38 attention h24 kv4 hd256 l40",
    );
}
