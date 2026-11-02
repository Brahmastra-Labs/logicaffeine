//! Mixture-of-experts at REAL width reaches the SN50 on the program path, moves only the chosen experts' weights,
//! and computes what the VM computes.
//!
//! A block whose widths are whole tiles lowers through the blocked form: a streamed router matmul, the router
//! softmax and top-K routing at the accumulator's width (picks at the chip's INT32), the chosen experts' gate, up
//! and down GEMVs as TILE GATHERS of their weight rows in chunks, and a renormalized combine. The promises:
//!
//!   1. IT COMPUTES THE BLOCK. Every case evaluates to the VM's answer under the program-path ledger's two-part
//!      rule — WIRING: the fp32 twin within a relative 2⁻¹⁶; CHIP PRECISION: the bf16 program within
//!      `sections + 1` bf16 ulps of the block's magnitude `Σ_r w̃_r Σ_i |Wd[o, i]·h_r[i]|`. The inputs are exact
//!      in bf16 (multiples of 1/64), so what is measured is the lowering's own arithmetic, not the data's
//!      quantization; the VM also agrees with an f64 reference written out here.
//!   2. ONLY THE CHOSEN EXPERTS MOVE. At Qwen3-30B-A3B's geometry (d 2048, dff 768, 128 experts, top 8), and at
//!      DeepSeek-V3's with its whole choice (d 7168, dff 2048, 256 experts scored by sigmoids, chosen by their scores
//!      plus a selection bias among the top 4 of 8 groups, top 8, scaled by 2.5), the block lowers whole and
//!      certifies, every load of an expert weight is a tile gather addressed by a pick, and the weight tiles moved are
//!      exactly `K` experts' gate, up and down.
//!   3. A SITE HOLDING SEVERAL ITEMS WRITES EVERY CHUNK OF EACH. On two small sites the gate/up partials walk two
//!      chunks per item and each site holds two items; the block still computes the VM's answer. The template states
//!      its stores' sub-blocks — placement reshapes only the store of the section's result, and only on a site holding
//!      one item — so no store depends on a later pass to learn how many chunks it writes.
//!   4. A SECOND OUTPUT IS COVERED BY WHAT ITS STORE ADDRESSES. The gate/up section's result is its gate partials;
//!      its up partials are a second output, which the items' rectangles do not describe. The validator counts the
//!      tiles the up store ADDRESSES: collapsing its chunks onto one tile, or storing it two tiles wide, is refused
//!      by name (`OutputOverlap` on the up partials).
//!
//! ★ MUTATION PROBES (each run 2026-09-24 — break, observe red, restore byte-identical):
//!   • the gate chunk loading the UP weights (every address still valid) → `moe d32 dff32 e4 k2: WIRING Show #1:
//!     fp32 twin -556476 vs VM -203984`;
//!   • an expert's tile stride one tile row long → refused before any data runs: the dry run's walk of the bound
//!     ends reads past the weight (`ReadOutOfRange { tensor: TensorId(3), tile: 6 }`);
//!   • `place` narrowing EVERY program's first two inputs to the item rect (the matmul-panel rule applied without a
//!     matmul) → `WIRING Show #1: fp32 twin -195059 vs VM -203984` — a weight chunk silently cut to one tile.
//!   • (2026-09-26, `logs/sn50/probe_moe_*.txt`) the gate partials store states one sub-block → the crowded block is
//!     refused, `CountingImbalance { reserved: 2, pushed: 2, popped: 1 }`: the chain pushes two chunks per item, the
//!     store pops one. The code of the morning of 2026-09-26 — both stores stating one sub-block, placement reshaping
//!     every store, coverage read from the rectangle — refuses it the same way: the crowded block never lowered.
//!   • (2026-09-27) the validator admits a tile stored twice (`c > 2`) → the collapsed up store is refused only as
//!     `OutputUncovered`, and the second-output test reds on the overlap it names; the validator covering every tensor
//!     by the rectangle → the collapsed up store validates (`Ok(())`) and the test reds.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_moe_real_width_lock`
#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::eval_choreography;
use logicaffeine_compile::spatial::ir::{
    AccessPattern, IrViolation, Phase, SpatialProgram, StageKind,
};
use logicaffeine_compile::spatial::passes::LV_SUB_H;
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use tensor_probes::{inputs_from_fills, oracle_ints, shown_from_outputs};

fn sn50() -> SpatialTarget {
    SpatialTarget::sn50_rdu()
}

fn fp32_twin() -> SpatialTarget {
    fp32_of(sn50())
}

/// A chip with fp32 beside bf16 on the matmul, pointwise and row units.
fn fp32_of(mut t: SpatialTarget) -> SpatialTarget {
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

/// Whether weight position `p` of the tensor salted `salt` is non-zero: about one in three, spread over every
/// expert, row and chunk, so a wrong expert, row or chunk reads different numbers.
fn live(p: usize, salt: usize) -> bool {
    (p.wrapping_mul(2654435761).wrapping_add(salt * 97) >> 5) % 3 == 0
}

struct Moe {
    d: usize,
    dff: usize,
    ne: usize,
    k: usize,
}

struct Case {
    src: String,
    /// Per `Show`: the f64 reference and the magnitude it is summed from.
    reference: Vec<(f64, f64)>,
}

fn dense(n: usize, salt: usize) -> Vec<f64> {
    (0..n).map(|p| val(p * 31 + salt)).collect()
}

fn sparse(n: usize, salt: usize) -> Vec<f64> {
    (0..n)
        .map(|p| {
            if live(p, salt) {
                val(p * 17 + salt)
            } else {
                0.0
            }
        })
        .collect()
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

fn silu(g: f64) -> f64 {
    g / (1.0 + (-g).exp())
}

fn case(m: &Moe) -> Case {
    case_with(m, dense(m.d, 1), dense(m.d * m.ne, 2))
}

/// A case whose router decides a row of any width: `X[0] = 4` and only the router's first row non-zero, `−1` for
/// every expert but `K + 1` spread over the row's blocks (the first in the last block), whose weights fall from `1` by
/// eighths — logits `4, 3.5, 3, …` against every other expert's `−4`, so each kept score clears the next by far more
/// than any rounding. (`dense`'s values repeat every 129 positions, so its router ties experts 129 apart.)
fn wide_case(m: &Moe) -> Case {
    let (d, ne, k) = (m.d, m.ne, m.k);
    let mut x = dense(d, 1);
    x[0] = 4.0;
    let mut wr = vec![0.0; d * ne];
    for e in 0..ne {
        wr[e] = -1.0;
    }
    for j in 0..=k {
        let e = (ne - 1 - j * 37) % ne;
        wr[e] = 1.0 - j as f64 / 8.0;
    }
    case_with(m, x, wr)
}

fn case_with(m: &Moe, x: Vec<f64>, wr: Vec<f64>) -> Case {
    let (d, dff, ne, k) = (m.d, m.dff, m.ne, m.k);
    let (wg, wu, wd) = (
        sparse(ne * dff * d, 3),
        sparse(ne * dff * d, 4),
        sparse(ne * d * dff, 5),
    );
    // The router, as the desugar computes it.
    let logits: Vec<f64> = (0..ne)
        .map(|e| (0..d).map(|t| x[t] * wr[t * ne + e]).sum())
        .collect();
    let mx = logits.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let z: f64 = logits.iter().map(|l| (l - mx).exp()).sum();
    let p: Vec<f64> = logits.iter().map(|l| (l - mx).exp() / z).collect();
    let mut order: Vec<usize> = (0..ne).collect();
    order.sort_by(|a, b| p[*b].partial_cmp(&p[*a]).unwrap().then(a.cmp(b)));
    // ★ THE CASE MUST DECIDE ITS EXPERTS UNAMBIGUOUSLY: every kept probability clear of the next by far more than
    // any rounding, or a correct lowering could pick differently from the VM.
    for w in order.windows(2).take(k) {
        assert!(
            p[w[0]] - p[w[1]] > 1e-3,
            "the case's router is ambiguous between experts {} and {} ({} vs {})",
            w[0],
            w[1],
            p[w[0]],
            p[w[1]]
        );
    }
    let chosen = &order[..k];
    let wsum: f64 = chosen.iter().map(|e| p[*e]).sum();
    let mut out = vec![0.0; d];
    let mut mag = vec![0.0; d];
    for &e in chosen {
        let w = p[e] / wsum;
        let h: Vec<f64> = (0..dff)
            .map(|o| {
                let g: f64 = (0..d).map(|t| x[t] * wg[(e * dff + o) * d + t]).sum();
                let u: f64 = (0..d).map(|t| x[t] * wu[(e * dff + o) * d + t]).sum();
                silu(g) * u
            })
            .collect();
        for o in 0..d {
            let row = (e * d + o) * dff;
            out[o] += w * (0..dff).map(|i| wd[row + i] * h[i]).sum::<f64>();
            mag[o] += w * (0..dff).map(|i| (wd[row + i] * h[i]).abs()).sum::<f64>();
        }
    }
    let mut src = String::from("## Main\n");
    src += &declare("X", 1, d, &x);
    src += &declare("Wr", d, ne, &wr);
    src += &declare("Wg", ne * dff, d, &wg);
    src += &declare("Wu", ne * dff, d, &wu);
    src += &declare("Wd", ne * d, dff, &wd);
    src +=
        &format!("Let H be moe of X with router Wr gate Wg up Wu down Wd choosing {k} of {ne}.\n");
    let shown: Vec<usize> = (0..d).step_by(3).collect();
    for o in &shown {
        src += &format!("Show round(item {} of H * 1000000).\n", o + 1);
    }
    Case {
        src,
        reference: shown.iter().map(|o| (out[*o], mag[*o])).collect(),
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

fn verdict(c: &Case, what: &str) {
    verdict_on(c, what, &sn50(), &fp32_twin());
}

/// [`verdict`] on `chip`, its wiring checked on `twin`.
fn verdict_on(c: &Case, what: &str, chip: &SpatialTarget, twin: &SpatialTarget) {
    let src = c.src.as_str();
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "{what}: VM: {:?}", vm.error);
    let want = oracle_ints(&vm.output).expect("vm ints");
    assert_eq!(want.len(), c.reference.len());
    for (i, (v, (r, _))) in want.iter().zip(&c.reference).enumerate() {
        assert!(
            ((*v as f64) - (r * 1e6).round()).abs() <= 1.0,
            "{what}: Show #{}: VM {v} vs the f64 reference {}",
            i + 1,
            r * 1e6
        );
    }
    // WIRING: the fp32 twin to a relative 2⁻¹⁶.
    let (twin, _) = composed(src, twin, what);
    let out = eval_choreography(&twin, &inputs_from_fills(&twin).expect("fills"))
        .unwrap_or_else(|e| panic!("{what}: fp32 eval: {e:?}"));
    let got = shown_from_outputs(src, &twin, &out).expect("shown");
    for (i, (g, v)) in got.iter().zip(&want).enumerate() {
        let bound = ((*v as f64).abs() / 65536.0).max(1.0);
        assert!(
            ((g - v).abs() as f64) <= bound,
            "{what}: WIRING Show #{}: fp32 twin {g} vs VM {v}",
            i + 1
        );
    }
    // CHIP PRECISION: the bf16 program within `sections + 1` ulps of the magnitude.
    let (sp, sections) = composed(src, chip, what);
    let out = eval_choreography(&sp, &inputs_from_fills(&sp).expect("fills"))
        .unwrap_or_else(|e| panic!("{what}: bf16 eval: {e:?}"));
    let got = shown_from_outputs(src, &sp, &out).expect("shown");
    for (i, ((g, v), (_, m))) in got.iter().zip(&want).zip(&c.reference).enumerate() {
        let bound = ((sections + 1) as f64 * ulp_bf16(*m) * 1e6).max(1.0);
        assert!(
            ((g - v).abs() as f64) <= bound,
            "{what}: CHIP Show #{}: bf16 {g} vs VM {v} (magnitude {m:.4}, bound {bound:.0})",
            i + 1
        );
    }
}

#[test]
fn a_blocked_moe_computes_the_block_at_every_shape() {
    for m in [
        Moe {
            d: 32,
            dff: 32,
            ne: 4,
            k: 2,
        },
        Moe {
            d: 64,
            dff: 96,
            ne: 8,
            k: 3,
        },
        Moe {
            d: 32,
            dff: 64,
            ne: 4,
            k: 4,
        },
    ] {
        let what = format!("moe d{} dff{} e{} k{}", m.d, m.dff, m.ne, m.k);
        verdict(&case(&m), &what);
    }
}

/// Every load of `tensor`: whether it is a gather, and the tiles it moves over the whole work.
fn weight_loads(sp: &SpatialProgram, tensor: &str) -> (usize, usize) {
    let mut direct = 0;
    let mut moved = 0;
    for s in &sp.stages {
        if let StageKind::Load {
            tensor: t, pattern, ..
        } = &s.kind
        {
            if sp.tensor(*t).name != tensor {
                continue;
            }
            if pattern.origin.indirect.is_none() {
                direct += 1;
                continue;
            }
            let section = sp.section_of_stage(s.id);
            let items = s
                .site
                .map(|site| sp.items_at_in(section, site).len())
                .unwrap_or(1);
            let passes = sp
                .plan_of_stage(s.id)
                .map(|p| (p.per_site_m / p.block.osh.max(1)) * (p.per_site_n / p.block.osw.max(1)))
                .unwrap_or(1);
            moved += pattern.block_h * pattern.block_w * items * passes;
        }
    }
    (direct, moved)
}

#[test]
fn at_qwen3_moe_geometry_only_the_chosen_experts_weights_move() {
    let (d, dff, ne, k) = (2048usize, 768usize, 128usize, 8usize);
    let src = format!(
        "## Main\nLet X be a 1 by {d} tensor of Real.\nLet Wr be a {d} by {ne} tensor of Real.\nLet Wg be a {} by {d} tensor of Real.\nLet Wu be a {} by {d} tensor of Real.\nLet Wd be a {} by {dff} tensor of Real.\nLet H be moe of X with router Wr gate Wg up Wu down Wd choosing {k} of {ne}.\nShow round(item 1 of H * 1000000).\n",
        ne * dff,
        ne * dff,
        ne * d
    );
    let (sp, sections) = composed(&src, &sn50(), "qwen3-30b-a3b moe");
    assert_eq!(
        sections, 7,
        "router, softmax, routing, gate/up, SwiGLU, down, combine"
    );
    let expert_tiles = (dff / 32) * (d / 32);
    for w in ["Wg", "Wu", "Wd"] {
        let (direct, moved) = weight_loads(&sp, w);
        assert_eq!(direct, 0, "{w}: every load of an expert weight is a gather");
        assert_eq!(
            moved,
            k * expert_tiles,
            "{w}: exactly {k} experts' tiles move, of {ne}"
        );
    }
}

#[test]
fn at_deepseek_v3_geometry_the_whole_choice_lowers_and_only_the_chosen_experts_move() {
    let (d, dff, ne, k) = (7168usize, 2048usize, 256usize, 8usize);
    let src = format!(
        "## Main\nLet X be a 1 by {d} tensor of Real.\nLet Wr be a {d} by {ne} tensor of Real.\nLet Wg be a {} by {d} tensor of Real.\nLet Wu be a {} by {d} tensor of Real.\nLet Wd be a {} by {dff} tensor of Real.\nLet Bs be a 1 by {ne} tensor of Real.\nLet H be moe of X with router Wr gate Wg up Wu down Wd choosing {k} of {ne} by sigmoid biased by Bs within the top 4 of 8 groups scaled by 2.5.\nShow round(item 1 of H * 1000000).\n",
        ne * dff,
        ne * dff,
        ne * d
    );
    let (sp, sections) = composed(&src, &sn50(), "deepseek-v3 moe");
    assert_eq!(sections, 7, "router, sigmoid, routing, gate/up, SwiGLU, down, combine");
    let expert_tiles = (dff / 32) * (d / 32);
    for w in ["Wg", "Wu", "Wd"] {
        let (direct, moved) = weight_loads(&sp, w);
        assert_eq!(direct, 0, "{w}: every load of an expert weight is a gather");
        assert_eq!(moved, k * expert_tiles, "{w}: exactly {k} experts' tiles move, of {ne}");
    }
}

/// ★ A ROUTER ROW PAST ONE ACCUMULATOR (more than 256 experts: Kimi K2's 384, the 512 of Qwen3-Next, the Qwen 3.5
/// mixtures and qwen4_exp) is routed in blocks: each block's top K candidates, then the top K of the candidates — the
/// row's own top K, since each of them is among its own block's — the lowest expert first among equal scores, as the
/// row's scan picks. The block computes the VM's answer where the last block is short (300 experts), at two whole
/// blocks with K 8 (384), and at the top 10 of 512, more picks than an accumulator page holds a tile each.
#[test]
fn a_router_row_past_one_accumulator_computes_the_block() {
    for m in [
        Moe { d: 32, dff: 32, ne: 300, k: 2 },
        Moe { d: 32, dff: 32, ne: 384, k: 8 },
        Moe { d: 32, dff: 32, ne: 512, k: 10 },
    ] {
        let what = format!("moe d{} dff{} e{} k{}", m.d, m.dff, m.ne, m.k);
        verdict(&wide_case(&m), &what);
    }
}

#[test]
fn at_kimi_k2_geometry_the_choice_lowers_and_only_the_chosen_experts_move() {
    let (d, dff, ne, k) = (7168usize, 2048usize, 384usize, 8usize);
    let src = format!(
        "## Main\nLet X be a 1 by {d} tensor of Real.\nLet Wr be a {d} by {ne} tensor of Real.\nLet Wg be a {} by {d} tensor of Real.\nLet Wu be a {} by {d} tensor of Real.\nLet Wd be a {} by {dff} tensor of Real.\nLet Bs be a 1 by {ne} tensor of Real.\nLet H be moe of X with router Wr gate Wg up Wu down Wd choosing {k} of {ne} by sigmoid biased by Bs scaled by 2.827.\nShow round(item 1 of H * 1000000).\n",
        ne * dff,
        ne * dff,
        ne * d
    );
    let (sp, _) = composed(&src, &sn50(), "kimi-k2 moe");
    let expert_tiles = (dff / 32) * (d / 32);
    for w in ["Wg", "Wu", "Wd"] {
        let (direct, moved) = weight_loads(&sp, w);
        assert_eq!(direct, 0, "{w}: every load of an expert weight is a gather");
        assert_eq!(moved, k * expert_tiles, "{w}: exactly {k} experts' tiles move, of {ne}");
    }
}

#[test]
fn at_qwen4_exp_geometry_the_top_ten_of_512_lowers_and_only_the_chosen_experts_move() {
    let (d, dff, ne, k) = (2560usize, 640usize, 512usize, 10usize);
    let src = format!(
        "## Main\nLet X be a 1 by {d} tensor of Real.\nLet Wr be a {d} by {ne} tensor of Real.\nLet Wg be a {} by {d} tensor of Real.\nLet Wu be a {} by {d} tensor of Real.\nLet Wd be a {} by {dff} tensor of Real.\nLet H be moe of X with router Wr gate Wg up Wu down Wd choosing {k} of {ne}.\nShow round(item 1 of H * 1000000).\n",
        ne * dff,
        ne * dff,
        ne * d
    );
    let (sp, _) = composed(&src, &sn50(), "qwen4_exp moe");
    let expert_tiles = (dff / 32) * (d / 32);
    for w in ["Wg", "Wu", "Wd"] {
        let (direct, moved) = weight_loads(&sp, w);
        assert_eq!(direct, 0, "{w}: every load of an expert weight is a gather");
        assert_eq!(moved, k * expert_tiles, "{w}: exactly {k} experts' tiles move, of {ne}");
    }
}

/// The SN50 as two sites with small memories: the expert chunks split each item's row into sub-blocks, and each site
/// holds several items of a section.
fn crowded() -> SpatialTarget {
    let mut t = sn50();
    t.sites = logicaffeine_compile::spatial::target::MeshShape { x: 2, y: 1 };
    t.site.local_mem_bytes = CROWDED_SITE_BYTES;
    t.site.local_mem_opt_in_bytes = None;
    t
}

/// Small enough that a gate/up chunk holds half of a four-tile row, large enough for every other section.
const CROWDED_SITE_BYTES: usize = 64 * 1024;

#[test]
fn a_site_holding_several_items_writes_every_chunk_of_each() {
    let m = Moe {
        d: 128,
        dff: 64,
        ne: 4,
        k: 2,
    };
    let c = case(&m);
    let (sp, _) = composed(&c.src, &crowded(), "crowded moe");
    // The precondition, checked: the gate/up partials store walks two or more chunks per item, on a site holding two or
    // more of the section's items — the shape a store that states one sub-block cannot write.
    let chunked: Vec<(
        Option<usize>,
        usize,
        logicaffeine_compile::spatial::ir::Site,
    )> = sp
        .stages
        .iter()
        .filter_map(|s| match (&s.kind, s.site) {
            (StageKind::Store { sub_blocks, .. }, Some(site)) if s.name.contains("store_up") => {
                Some((sp.section_of_stage(s.id), sub_blocks.0, site))
            }
            _ => None,
        })
        .collect();
    assert!(
        !chunked.is_empty(),
        "the crowded block has a gate/up partials store: {:?}",
        sp.stages
            .iter()
            .filter(|s| matches!(s.kind, StageKind::Store { .. }))
            .map(|s| &s.name)
            .collect::<Vec<_>>()
    );
    assert!(
        chunked.iter().all(|(_, n, _)| *n >= 2),
        "every gate/up store walks two or more chunks: {chunked:?}"
    );
    assert!(
        chunked
            .iter()
            .any(|(sec, _, site)| sp.items_at_in(*sec, *site).len() >= 2),
        "some site holds several gate/up items: {chunked:?}"
    );
    // The fp32 twin's pages are twice as wide, so its sites are twice as large.
    let mut twin = fp32_of(crowded());
    twin.site.local_mem_bytes *= 2;
    verdict_on(&c, "crowded moe d128 dff64 e4 k2", &crowded(), &twin);
}

#[test]
fn a_second_partial_store_is_checked_by_the_tiles_it_addresses() {
    let c = case(&Moe {
        d: 128,
        dff: 64,
        ne: 4,
        k: 2,
    });
    let (p, _) = composed(&c.src, &crowded(), "crowded moe");
    assert!(
        p.validate().is_ok(),
        "the composed program validates: {:?}",
        p.validate()
    );
    // The gate/up section's result is its GATE partials; its UP partials are a second output of another name, which
    // the items' rectangles do not describe — the validator covers them by the tiles their store addresses.
    let up = p
        .tensors
        .iter()
        .find(|t| t.name == "H__moe_up_part")
        .expect("the up partials")
        .id;
    let misaddressed = |edit: &dyn Fn(&mut AccessPattern)| {
        let mut bad = p.clone();
        let store = bad
            .stages
            .iter_mut()
            .find(|s| matches!(&s.kind, StageKind::Store { tensor, .. } if *tensor == up))
            .expect("an up store");
        if let StageKind::Store { pattern, .. } = &mut store.kind {
            edit(pattern);
        }
        bad.validate()
    };
    // Every chunk of the item writing the first chunk's partial: two writers of one tile.
    let collapsed = misaddressed(&|pat| {
        for (v, step) in pat.origin.steps.iter_mut() {
            if *v == LV_SUB_H {
                *step = 0;
            }
        }
    });
    assert!(
        matches!(collapsed, Err(IrViolation::OutputOverlap { tensor, .. }) if tensor == up),
        "chunks sharing one partial tile: {collapsed:?}"
    );
    // A partial stored two tiles wide runs past a tensor one tile wide.
    let widened = misaddressed(&|pat| pat.block_w = 2);
    assert!(
        matches!(widened, Err(IrViolation::OutputOverlap { tensor, .. }) if tensor == up),
        "a partial stored two tiles wide: {widened:?}"
    );
}
