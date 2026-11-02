//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! A KV CACHE IS DENSE TILES: 32 POSITIONS PER TILE ROW, READ A TILE ROW AT A TIME.
//!
//! A decode step reads every cached position and writes one. Laid out FLAT (`1 × ctx·akv`), each position fills
//! the first row of its own tile row and pads the other 31 — 32× the SRAM per block and 32× the bytes per read
//! — and at Qwen3.8's real geometry a site then holds 4 positions of a 256-wide head, so a 4096-position cache is
//! 1024 blocks, each moving 31 rows of padding. Laid out as the program declares it (`ctx × akv`), one tile row
//! holds 32 positions of every head: a block is one tile row of one head, `hd / 32` tiles, and the same cache is
//! 128 blocks — folded in ONE section, the running softmax carried on chip from block to block.
//!
//! The layout is decided ONCE per table, by its reader: a table an attention reads as K or V in the flash form is
//! dense, so the cache append and the attention always agree on it. The append on a dense table is a
//! read-modify-write of the tile row holding the row: row `pos − 1` of tile row `⌊(pos − 1)/32⌋`, every other row
//! of that tile row written back unchanged.
//!
//! Pinned: the density and the fold at real width; the append's tile traffic; and a decode step — append, then
//! masked attention — against an f64 reference, on a cache whose length is a whole number of tile rows and on one
//! that is not (the tail block's padding rows are masked, never attended).
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_tilized_kv_cache_lock`
//!
//! ★ MUTATION PROBES (run when this landed): write the in-tile row as a constant 0 (`PredRhs::Const(0)` in
//! `dense_set_row`) — every decode test reds on the key cache after the append. Ignore the block's live bound `hi`
//! — the decode tests stay GREEN (their `up to pos` limit masks the tail anyway), and the unmasked-tail and
//! literal-prefix tests red at `ct[1] = -0.0079` for `-0.0126`: the padding rows attended as real positions. That
//! blind spot is why those two tests exist.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::ir::{
    AccessPattern, SectionSpan, SpatialProgram, StageKind, TensorRole,
};
use logicaffeine_compile::spatial::passes::LV_KBLOCK;
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;

#[path = "common/spatial.rs"]
mod spatial_support;
use spatial_support::inputs_from_fills;

/// A decode step's attention statements: append this token's key and value as row `pos`, attend up to `pos`.
fn decode(ctx: usize, pos: usize, h: usize, kv: usize, hd: usize, filled: bool) -> String {
    let (aq, akv) = (h * hd, kv * hd);
    let mut s = format!(
        "## Main\nLet pos be {pos}.\nLet Kc be a {ctx} by {akv} tensor of Real.\nLet Vc be a {ctx} by {akv} tensor of Real.\n\
         Let qp be a 1 by {aq} tensor of Real.\nLet kp be a 1 by {akv} tensor of Real.\nLet vr be a 1 by {akv} tensor of Real.\n"
    );
    if filled {
        for (name, n) in [
            ("Kc", ctx * akv),
            ("Vc", ctx * akv),
            ("qp", aq),
            ("kp", akv),
            ("vr", akv),
        ] {
            for i in 0..n {
                s.push_str(&format!(
                    "Set item {} of {name} to {:?}.\n",
                    i + 1,
                    fill(name, i)
                ));
            }
        }
    }
    s.push_str(&format!(
        "Set row pos of Kc to kp.\nSet row pos of Vc to vr.\nLet ct be attention of qp over Kc and Vc with {h} heads {kv} groups up to pos.\n"
    ));
    s
}

/// Bf16-exact fills: small integers over 64, different per tensor.
fn fill(name: &str, i: usize) -> f64 {
    let salt = name.bytes().map(|b| b as usize).sum::<usize>();
    ((((i + 1) * 37 + salt * 11) % 29) as f64 - 14.0) / 64.0
}

fn composed(src: &str) -> SpatialProgram {
    let l = compile_program_to_spatial(src, &RduChip::Sn50.facts().spatial_target())
        .expect("a tensor program");
    l.program
        .unwrap_or_else(|| panic!("the decode statements compose and certify: {:?}", l.missing))
}

fn tensor_shape(p: &SpatialProgram, name: &str) -> (usize, usize) {
    let t = p
        .tensors
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("`{name}`"));
    (t.elems.rows, t.elems.cols)
}

/// ── AT REAL WIDTH THE CACHE IS DENSE: 32-position blocks of 8 tiles, 128 of them, folded in ONE section.
#[test]
fn a_real_width_cache_is_read_a_dense_tile_row_at_a_time() {
    let (ctx, h, kv, hd) = (4096, 24, 4, 256);
    let p = composed(&decode(ctx, 1000, h, kv, hd, false));
    for cache in ["Kc", "Vc"] {
        assert_eq!(
            tensor_shape(&p, cache),
            (ctx, kv * hd),
            "{cache} is declared as the program declares it"
        );
    }
    let head_t = hd / 32;
    let block_sections: Vec<&SectionSpan> = p
        .sections
        .iter()
        .filter(|s| s.name.contains("flash_blocks"))
        .collect();
    assert_eq!(
        block_sections.len(),
        1,
        "one block section: {:?}",
        block_sections.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    assert!(
        block_sections[0].name.ends_with("_b32"),
        "a block is a full tile row of 32 positions: {}",
        block_sections[0].name
    );
    // 128 live blocks — one per tile row of the 4096-position cache — folded into the running softmax on chip: the
    // section's K blocks are the cache's tile rows, and nothing merges after it.
    let blocks = ctx / 32;
    assert_eq!(
        block_sections[0].plan.map(|pl| pl.inner.num_blocks),
        Some(blocks),
        "the section folds {blocks} blocks"
    );
    let merges: Vec<&str> = p
        .sections
        .iter()
        .map(|s| s.name.as_str())
        .filter(|n| n.contains("merge"))
        .collect();
    assert!(
        merges.is_empty(),
        "the blocks fold on chip, so no section merges partials: {merges:?}"
    );
    let flash = p
        .sections
        .iter()
        .position(|s| s.name.contains("flash_blocks"));
    for cache in ["Kc", "Vc"] {
        let attention_reads: Vec<&AccessPattern> = p
            .stages
            .iter()
            .filter(|s| p.section_of_stage(s.id) == flash)
            .filter_map(|s| match &s.kind {
                StageKind::Load {
                    tensor, pattern, ..
                } if p.tensor(*tensor).name == cache => Some(pattern),
                _ => None,
            })
            .collect();
        assert!(
            !attention_reads.is_empty()
                && attention_reads
                    .iter()
                    .all(|pat| (pat.block_h, pat.block_w) == (1, head_t)),
            "{cache}: the attention reads one tile row of one head, {head_t} tiles, per block"
        );
        assert!(
            attention_reads
                .iter()
                .all(|pat| pat.origin.indirect.is_none()
                    && pat.origin.steps.contains(&(LV_KBLOCK, kv * head_t))),
            "{cache}: block `b` is tile row `b` — an address affine in the block, no gather"
        );
    }
}

/// ── THE APPEND READS AND WRITES THE ONE TILE ROW THAT HOLDS THE ROW.
#[test]
fn the_append_rewrites_exactly_the_tile_row_that_holds_the_row() {
    let (ctx, h, kv, hd) = (4096, 24, 4, 256);
    let p = composed(&decode(ctx, 1000, h, kv, hd, false));
    let row_t = kv * hd / 32;
    for cache in ["Kc", "Vc"] {
        let stored: usize = p
            .stages
            .iter()
            .filter_map(|s| match &s.kind {
                StageKind::Store {
                    tensor, pattern, ..
                } if p.tensor(*tensor).name == cache && pattern.origin.indirect.is_some() => {
                    let items = s
                        .site
                        .map(|site| p.items_at_in(p.section_of_stage(s.id), site).len())
                        .unwrap_or(1);
                    Some(pattern.block_h * pattern.block_w * items)
                }
                _ => None,
            })
            .sum();
        assert_eq!(
            stored, row_t,
            "{cache}: the append stores the {row_t} tiles of one tile row"
        );
        let role = p
            .tensors
            .iter()
            .find(|t| t.name == cache)
            .expect("the cache")
            .role;
        assert_eq!(
            role,
            TensorRole::State,
            "{cache} is the step's carried state"
        );
    }
}

/// The f64 reference of the decode statements: the caches after the append, and the attention up to `pos`.
fn reference(
    ctx: usize,
    pos: usize,
    h: usize,
    kv: usize,
    hd: usize,
) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let akv = kv * hd;
    let mut k: Vec<f64> = (0..ctx * akv).map(|i| fill("Kc", i)).collect();
    let mut v: Vec<f64> = (0..ctx * akv).map(|i| fill("Vc", i)).collect();
    for c in 0..akv {
        k[(pos - 1) * akv + c] = fill("kp", c);
        v[(pos - 1) * akv + c] = fill("vr", c);
    }
    let group = h / kv;
    let mut ct = vec![0.0; h * hd];
    for head in 0..h {
        let x = head / group;
        let scores: Vec<f64> = (0..pos)
            .map(|j| {
                (0..hd)
                    .map(|e| fill("qp", head * hd + e) * k[j * akv + x * hd + e])
                    .sum::<f64>()
                    / (hd as f64).sqrt()
            })
            .collect();
        let m = scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let w: Vec<f64> = scores.iter().map(|s| (s - m).exp()).collect();
        let sum: f64 = w.iter().sum();
        for e in 0..hd {
            ct[head * hd + e] = (0..pos)
                .map(|j| w[j] * v[j * akv + x * hd + e])
                .sum::<f64>()
                / sum;
        }
    }
    (k, v, ct)
}

fn values(
    p: &SpatialProgram,
    out: &logicaffeine_compile::spatial::eval::Outputs,
    name: &str,
) -> Vec<f64> {
    let id = p
        .tensors
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("`{name}`"))
        .id;
    match out
        .tensors
        .get(&id)
        .unwrap_or_else(|| panic!("`{name}` is a result"))
    {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    }
}

/// Run the decode statements on the chip's evaluator and hold them to the reference: the appended caches exactly
/// (a copy of a bf16-exact row), the attention within (sections + 1) bf16 ulps of the largest cached value.
fn decode_equals_the_reference(ctx: usize, pos: usize, h: usize, kv: usize, hd: usize) {
    let p = composed(&decode(ctx, pos, h, kv, hd, true));
    let out =
        eval_choreography(&p, &inputs_from_fills(&p)).expect("the decode statements evaluate");
    let (k, v, ct) = reference(ctx, pos, h, kv, hd);
    assert_eq!(
        values(&p, &out, "Kc"),
        k,
        "ctx {ctx}, pos {pos}: the key cache after the append, every row"
    );
    assert_eq!(
        values(&p, &out, "Vc"),
        v,
        "ctx {ctx}, pos {pos}: the value cache after the append, every row"
    );
    let got = values(&p, &out, "ct");
    let bound = (p.sections.len() as f64 + 1.0)
        * 2f64.powi(-8)
        * v.iter().fold(0.0f64, |a, x| a.max(x.abs()));
    for (i, (g, w)) in got.iter().zip(&ct).enumerate() {
        assert!(
            (g - w).abs() <= bound,
            "ctx {ctx}, pos {pos}: ct[{i}] = {g}, reference {w} (bound {bound})"
        );
    }
}

/// ── A CACHE OF WHOLE TILE ROWS: two blocks, the append in the second.
#[test]
fn a_decode_step_over_whole_tile_rows_equals_the_reference() {
    decode_equals_the_reference(64, 40, 2, 1, 32);
}

/// ── A CACHE THAT ENDS INSIDE A TILE ROW: the tail block's padding rows are never attended.
#[test]
fn a_decode_step_over_a_partial_tail_tile_row_equals_the_reference() {
    decode_equals_the_reference(50, 50, 2, 1, 32);
    decode_equals_the_reference(50, 33, 2, 1, 32);
}

/// ── THE FIRST ROW AND THE LAST ROW OF A TILE ROW — the two ends of the in-tile row select.
#[test]
fn the_append_lands_at_both_ends_of_a_tile_row() {
    decode_equals_the_reference(64, 33, 2, 1, 32);
    decode_equals_the_reference(64, 32, 2, 1, 32);
    decode_equals_the_reference(64, 1, 2, 1, 32);
}

/// An attention over a declared cache with no append: over the whole cache, or its first `prefix` positions.
fn attend(ctx: usize, h: usize, kv: usize, hd: usize, prefix: Option<usize>) -> String {
    let (aq, akv) = (h * hd, kv * hd);
    let mut s = format!("## Main\nLet Kc be a {ctx} by {akv} tensor of Real.\nLet Vc be a {ctx} by {akv} tensor of Real.\nLet qp be a 1 by {aq} tensor of Real.\n");
    for (name, n) in [("Kc", ctx * akv), ("Vc", ctx * akv), ("qp", aq)] {
        for i in 0..n {
            s.push_str(&format!(
                "Set item {} of {name} to {:?}.\n",
                i + 1,
                fill(name, i)
            ));
        }
    }
    let upto = prefix.map(|p| format!(" up to {p}")).unwrap_or_default();
    s.push_str(&format!(
        "Let ct be attention of qp over Kc and Vc with {h} heads {kv} groups{upto}.\n"
    ));
    s
}

/// Hold an attention with no append to the f64 reference over its first `l` positions.
fn attention_equals_the_reference(
    ctx: usize,
    h: usize,
    kv: usize,
    hd: usize,
    prefix: Option<usize>,
) {
    let l = prefix.unwrap_or(ctx);
    let p = composed(&attend(ctx, h, kv, hd, prefix));
    for cache in ["Kc", "Vc"] {
        assert_eq!(
            tensor_shape(&p, cache),
            (ctx, kv * hd),
            "{cache} is read dense"
        );
    }
    let out = eval_choreography(&p, &inputs_from_fills(&p)).expect("the attention evaluates");
    let akv = kv * hd;
    let group = h / kv;
    let k: Vec<f64> = (0..ctx * akv).map(|i| fill("Kc", i)).collect();
    let v: Vec<f64> = (0..ctx * akv).map(|i| fill("Vc", i)).collect();
    let got = values(&p, &out, "ct");
    let bound = (p.sections.len() as f64 + 1.0)
        * 2f64.powi(-8)
        * v.iter().fold(0.0f64, |a, x| a.max(x.abs()));
    for head in 0..h {
        let x = head / group;
        let scores: Vec<f64> = (0..l)
            .map(|j| {
                (0..hd)
                    .map(|e| fill("qp", head * hd + e) * k[j * akv + x * hd + e])
                    .sum::<f64>()
                    / (hd as f64).sqrt()
            })
            .collect();
        let m = scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let w: Vec<f64> = scores.iter().map(|s| (s - m).exp()).collect();
        let sum: f64 = w.iter().sum();
        for e in 0..hd {
            let want = (0..l).map(|j| w[j] * v[j * akv + x * hd + e]).sum::<f64>() / sum;
            let g = got[head * hd + e];
            assert!(
                (g - want).abs() <= bound,
                "ctx {ctx}, l {l}: ct[{}] = {g}, reference {want} (bound {bound})",
                head * hd + e
            );
        }
    }
}

/// ── AN UNMASKED ATTENTION OVER A CACHE THAT ENDS INSIDE A TILE ROW: the tail's padding rows hold zeros, which a
/// score of `q·0 = 0` would weigh as a real position — only the block's live bound keeps them out.
#[test]
fn an_unmasked_attention_over_a_partial_tail_never_attends_the_padding() {
    attention_equals_the_reference(50, 2, 1, 32, None);
    attention_equals_the_reference(40, 2, 1, 32, None);
}

/// ── A LITERAL PREFIX: the blocks cover the first `p` positions, the rest of the cache is never attended.
#[test]
fn a_literal_prefix_attends_exactly_its_positions() {
    attention_equals_the_reference(64, 2, 1, 32, Some(40));
    attention_equals_the_reference(64, 2, 1, 32, Some(32));
    attention_equals_the_reference(64, 2, 1, 32, Some(1));
}
