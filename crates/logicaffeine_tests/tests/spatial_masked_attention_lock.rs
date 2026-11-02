//! The masked attention — `Let C be attention of Q over K and V with H heads KV groups up to P.` — a decode step's
//! attention over the first `P` positions of its KV cache, as language and on the SN50.
//!
//! A decode step's cache is declared at the length it will ever reach, and only the first `P` rows are written yet.
//! The promises:
//!
//!   1. MEANING. `up to P` is the attention over a cache of exactly its first `P` rows — the tree-walker and the VM
//!      agree, and both equal the unmasked attention over the truncated cache.
//!   2. `P` IS CHECKED, ONCE. A `P` outside `1..=L` fails with one message on every tier; the bound expression runs
//!      exactly once.
//!   3. THE SN50 COMPUTES IT. For a `P` the program computes, the flash form (heads in whole tiles) masks every
//!      position from `P` on in every block, and the flat form (narrower heads) masks the scores; a literal `P`
//!      lowers as the prefix it is. Each evaluates to the VM's answer under the ledger's two-part rule (WIRING: the
//!      fp32 twin within a relative 2⁻¹⁶; CHIP: bf16 within `sections + 1` ulps of `Σ p·|v|`), and a `P` outside the
//!      cache FAULTS at its guard, named against the cache's `L` positions — the flat form's index guard; the flash
//!      form's fold runs its blocks to `P` (`spatial_trip_count_lock`), and the trip admitting exactly `1..=L` refuses
//!      it before any op — the chip never attends past the cache or over nothing.
//!   4. THE DECODE CYCLE. `Set row pos of Kc to k.` then `attention … up to pos` — the cache append, then attention
//!      over the filled positions — lowers whole on the SN50 and computes what the VM computes.
//!
//! Inputs are exact in bf16 (multiples of 1/64), so the chip check measures the lowering's arithmetic, not the
//! data's quantization.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_masked_attention_lock`
//!
//! ★ MUTATION PROBES: (run when this landed) lower a computed `P` as no limit at all (`from_attention`'s
//! `RowIndex::Named` arm returning `(l, None)`) — the flat form, the flash form and the decode cycle red on their
//! values against the VM, and a `P` outside the cache stops faulting at its guard. (2026-09-27, the flash fold, logs
//! `logs/sn50/probe_ffold_*.txt`) the fold counts a block start for a single block → the literal prefix reds: it
//! needs no runtime mask; the fold drops the mask of a cache ending mid-tile → the literal prefix and the real-width
//! and tilized-cache locks red on their values (`full t1 h2 hd128: 1386` vs VM `270000`); the fold never counts its
//! blocks → every multi-block mask reads block 0's limit, and the real-width and causal locks red. (2026-09-27, trip
//! counts, `logs/sn50/probe_guard_admits_one_more.txt`) the guard's range widened by one position → the flash form's
//! trip refuses `P = 0` against `(1, 41)`, not the cache's `(1, 40)`: the guard test reds on the range it names.
#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::spatial::eval::{eval_choreography, ChoreoFault};
use logicaffeine_compile::spatial::ir::{Phase, SpatialProgram};
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

/// Attention over the first `p` positions, in f64: per output element, the value and the magnitude `Σ p_j·|v_j|`.
fn reference(
    q: &[f64],
    k: &[f64],
    v: &[f64],
    h: usize,
    kv: usize,
    hd: usize,
    p: usize,
) -> Vec<(f64, f64)> {
    let group = h / kv;
    let mut out = Vec::new();
    for head in 0..h {
        let kvh = head / group;
        let at = |m: &[f64], j: usize, e: usize| m[j * kv * hd + kvh * hd + e];
        let scores: Vec<f64> = (0..p)
            .map(|j| {
                (0..hd).map(|e| q[head * hd + e] * at(k, j, e)).sum::<f64>() / (hd as f64).sqrt()
            })
            .collect();
        let mx = scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let ex: Vec<f64> = scores.iter().map(|s| (s - mx).exp()).collect();
        let z: f64 = ex.iter().sum();
        for e in 0..hd {
            out.push((
                (0..p).map(|j| ex[j] / z * at(v, j, e)).sum(),
                (0..p).map(|j| ex[j] / z * at(v, j, e).abs()).sum(),
            ));
        }
    }
    out
}

struct Shape {
    h: usize,
    kv: usize,
    hd: usize,
    l: usize,
}

/// A case: the source and, per `Show`, the reference value and magnitude.
struct Case {
    src: String,
    reference: Vec<(f64, f64)>,
}

const STRIDE: usize = 5;

fn shows(n: usize) -> String {
    (0..n)
        .step_by(STRIDE)
        .map(|p| format!("Show round(item {} of C * 1000000).\n", p + 1))
        .collect()
}

/// `attention … up to <bound>`, with `P` the attended count. `bound` is the source text of the bound.
fn masked(s: &Shape, p: usize, bound: &str) -> Case {
    let (q, k, v) = (
        vals(s.h * s.hd, 1),
        vals(s.l * s.kv * s.hd, 2),
        vals(s.l * s.kv * s.hd, 3),
    );
    let mut src = String::from("## Main\n");
    src += &declare("Q", 1, s.h * s.hd, &q);
    src += &declare("K", s.l, s.kv * s.hd, &k);
    src += &declare("V", s.l, s.kv * s.hd, &v);
    src += &format!("Let P be {p}.\nLet C be attention of Q over K and V with {} heads {} groups up to {bound}.\n", s.h, s.kv);
    src += &shows(s.h * s.hd);
    // A `P` outside the cache has no attention to refer to — the program fails; there is only the source.
    let reference = if (1..=s.l).contains(&p) {
        let r = reference(&q, &k, &v, s.h, s.kv, s.hd, p);
        (0..s.h * s.hd).step_by(STRIDE).map(|i| r[i]).collect()
    } else {
        Vec::new()
    };
    Case { src, reference }
}

/// The decode cycle: the cache filled for positions before `pos`, row `pos` appended, then attention up to `pos`.
fn decode_cycle(s: &Shape, pos: usize) -> Case {
    let w = s.kv * s.hd;
    let q = vals(s.h * s.hd, 4);
    let mut k: Vec<f64> = vals(s.l * w, 5)
        .into_iter()
        .enumerate()
        .map(|(i, x)| if i < (pos - 1) * w { x } else { 0.0 })
        .collect();
    let mut v: Vec<f64> = vals(s.l * w, 6)
        .into_iter()
        .enumerate()
        .map(|(i, x)| if i < (pos - 1) * w { x } else { 0.0 })
        .collect();
    let (kp, vp) = (vals(w, 7), vals(w, 8));
    let mut src = String::from("## Main\n");
    src += &declare("Q", 1, s.h * s.hd, &q);
    src += &declare("Kc", s.l, w, &k);
    src += &declare("Vc", s.l, w, &v);
    src += &declare("kp", 1, w, &kp);
    src += &declare("vp", 1, w, &vp);
    src += &format!("Let pos be {pos}.\nSet row pos of Kc to kp.\nSet row pos of Vc to vp.\n");
    src += &format!(
        "Let C be attention of Q over Kc and Vc with {} heads {} groups up to pos.\n",
        s.h, s.kv
    );
    src += &shows(s.h * s.hd);
    k[(pos - 1) * w..pos * w].copy_from_slice(&kp);
    v[(pos - 1) * w..pos * w].copy_from_slice(&vp);
    let r = reference(&q, &k, &v, s.h, s.kv, s.hd, pos);
    Case {
        src,
        reference: (0..s.h * s.hd).step_by(STRIDE).map(|i| r[i]).collect(),
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

/// The two-part verdict, with the VM also checked against the f64 reference.
fn verdict(c: &Case, what: &str) {
    let src = c.src.as_str();
    let (out, err) = tiers(src);
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
    let (twin, _) = composed(src, &fp32_twin(), what);
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
    let (sp, sections) = composed(src, &sn50(), what);
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

const FLASH: Shape = Shape {
    h: 2,
    kv: 1,
    hd: 32,
    l: 40,
};
const FLASH_GQA: Shape = Shape {
    h: 4,
    kv: 2,
    hd: 128,
    l: 33,
};
const FLAT: Shape = Shape {
    h: 2,
    kv: 1,
    hd: 4,
    l: 6,
};

// ── 1. meaning ───────────────────────────────────────────────────────────────────────────────────────

#[test]
fn up_to_p_is_the_attention_over_the_first_p_rows() {
    for p in [1usize, 3, 6] {
        let s = &FLAT;
        let full = masked(s, p, "P");
        let (q, k, v) = (
            vals(s.h * s.hd, 1),
            vals(s.l * s.kv * s.hd, 2),
            vals(s.l * s.kv * s.hd, 3),
        );
        let mut truncated = String::from("## Main\n");
        truncated += &declare("Q", 1, s.h * s.hd, &q);
        truncated += &declare("K", p, s.kv * s.hd, &k[..p * s.kv * s.hd]);
        truncated += &declare("V", p, s.kv * s.hd, &v[..p * s.kv * s.hd]);
        truncated += &format!(
            "Let C be attention of Q over K and V with {} heads {} groups.\n",
            s.h, s.kv
        );
        truncated += &shows(s.h * s.hd);
        let (masked_out, e1) = tiers(&full.src);
        let (trunc_out, e2) = tiers(&truncated);
        assert_eq!((e1, e2), (None, None));
        assert_eq!(
            masked_out, trunc_out,
            "up to {p} equals the attention over the first {p} rows"
        );
    }
}

// ── 2. P is checked, once ────────────────────────────────────────────────────────────────────────────

#[test]
fn a_p_outside_the_cache_fails_with_one_message_on_every_tier() {
    for p in [0usize, FLAT.l + 1] {
        let (_, err) = tiers(&masked(&FLAT, p, "P").src);
        let err = err.unwrap_or_else(|| panic!("up to {p} must fail"));
        assert!(
            err.contains("attention length out of range") && err.contains("K has 6 positions"),
            "up to {p}: {err}"
        );
    }
}

#[test]
fn the_bound_expression_runs_once() {
    let s = &FLAT;
    let mut src = String::from("## To count (c: Int) -> Int:\n    Show c.\n    Return c.\n\n");
    src += &masked(s, 3, "(count(3))").src;
    let (out, err) = tiers(&src);
    assert_eq!(err, None);
    assert_eq!(
        out.iter().filter(|l| l.as_str() == "3").count(),
        1,
        "count(3) runs exactly once: {out:?}"
    );
}

// ── 3. the SN50 computes it ──────────────────────────────────────────────────────────────────────────

#[test]
fn a_computed_p_is_masked_in_the_flash_form() {
    for (s, ps) in [(&FLASH, [1usize, 17, 40]), (&FLASH_GQA, [1, 20, 33])] {
        for p in ps {
            verdict(
                &masked(s, p, "P"),
                &format!("flash h{} kv{} hd{} l{} up to P={p}", s.h, s.kv, s.hd, s.l),
            );
        }
    }
}

#[test]
fn a_computed_p_is_masked_in_the_flat_form() {
    for p in [1usize, 3, 6] {
        verdict(&masked(&FLAT, p, "P"), &format!("flat up to P={p}"));
    }
}

#[test]
fn a_literal_p_lowers_as_the_prefix_it_is() {
    let c = masked(&FLASH, 5, "5");
    verdict(&c, "flash up to 5");
    let (sp, _) = composed(&c.src, &sn50(), "flash up to 5");
    assert!(
        !sp.stages.iter().any(|s| s.name.contains("block_limit")),
        "a literal bound needs no runtime mask"
    );
}

#[test]
fn a_computed_p_outside_the_cache_faults_at_its_guard() {
    for (s, what) in [(&FLASH, "flash"), (&FLAT, "flat")] {
        for p in [0usize, s.l + 1] {
            let (sp, _) = composed(&masked(s, 3, "P").src, &sn50(), what);
            let mut bad = sp.clone();
            for t in &mut bad.tensors {
                if t.name == "P" {
                    t.init = logicaffeine_compile::spatial::ir::TensorInit::Values(vec![p as i64]);
                }
            }
            match eval_choreography(&bad, &inputs_from_fills(&bad).expect("fills")) {
                Err(ChoreoFault::GatherOutOfRange { extent, .. }) => {
                    assert_eq!(extent, s.l, "{what} P={p}: checked against the cache")
                }
                Err(ChoreoFault::TripOutOfRange {
                    value, admitted, ..
                }) => {
                    assert_eq!(
                        value, p as f64,
                        "{what} P={p}: the fault carries the length it refused"
                    );
                    assert_eq!(
                        admitted,
                        (1, s.l as i64),
                        "{what} P={p}: checked against the cache"
                    );
                }
                other => {
                    panic!("{what} P={p}: a length outside the cache must fault, not {other:?}")
                }
            }
        }
    }
}

// ── 4. the decode cycle ──────────────────────────────────────────────────────────────────────────────

#[test]
fn the_decode_cycle_appends_then_attends_up_to_the_new_row() {
    verdict(&decode_cycle(&FLASH, 17), "flash decode cycle pos 17");
    verdict(
        &decode_cycle(&FLASH_GQA, 33),
        "flash GQA decode cycle pos 33 (the last row)",
    );
    verdict(&decode_cycle(&FLAT, 4), "flat decode cycle pos 4");
}
