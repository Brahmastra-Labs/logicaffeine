//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! A DECODE STEP'S QUERY HEADS THAT SHARE A K/V HEAD STREAM IT ONCE.
//!
//! Qwen3.8's attention is 24 query heads over 4 K/V heads (grouped-query attention, groups of 6). A decode step has
//! one query row, and it attends every cached position. Folded one item per QUERY head, each K/V head's cache streams
//! past six items — 101 MB of the 102 MB the flash section reads at a 4,096-position cache, 84 MB of it re-reads.
//! Grouped, one item per K/V HEAD folds its cache ONCE for the group's queries, held as the rows of one query tile:
//!   • the queries are regrouped into a `(kv · tile) × hd` matrix — group `x`'s heads in tile row `x`, one per row,
//!     the rows past the group zero;
//!   • the fold runs one item per K/V head, every row limited by the same position;
//!   • the result goes back to the row of heads the program declares.
//!
//! Pinned:
//!   1. At real width (24 query heads over 4 K/V heads of 256, a 4,096-position cache) the fold is ONE section of 4
//!      items and reads each cache exactly once a step — `ctx · kv · hd` elements of K and of V, no more.
//!   2. It computes the attention: groups of 2, 3 and 6, one K/V head and two, a cache of whole tile rows and one
//!      ending inside a tile row, `up to pos` after an append, a literal prefix, and the whole cache — against the f64
//!      reference on the chip (within `sections + 1` bf16 ulps of the largest value) and on the fp32 twin (within
//!      `sections + 1` units of 2⁻²⁰).
//!   3. The regrouping is two sections that move only the queries and the result; where there is nothing to share —
//!      a head per K/V head, or a block of query rows (a prefill chunk) — the fold keeps an item per query head.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_grouped_decode_attention_lock`
//!
//! ★ MUTATION PROBES (run 2026-09-27 in a private copy — break, observe red; logs `logs/sn50/probe_gqa_*.txt`):
//!   • grouping never taken → 7 of 8 red (an item per query head; one section, not three);
//!   • every grouped item reads K/V head 0 → every case over two K/V heads reds on CHIP (`ct[97] = 0.00497` for
//!     `-0.00921`); one K/V head stays green — only a second head can see it;
//!   • a grouped row limited as a query position (`P + tile row + row`) → every masked case reds (head 1 sees one
//!     position more: `ct[33] = -0.00032` for `0.00604`); the literal prefix (a constant limit) stays green;
//!   • the result lifted from each group's first row → every numerics case reds at head 2 or 1 of a group;
//!   • the regrouping reads head 0 for every row → every numerics case reds.
//!   The ports it made (`spatial_causal_attention_lock` promise 5, `qwen38_decode_step_lock` real width) red with
//!   grouping never taken.
//! ════════════════════════════════════════════════════════════════════════════════════════════════
#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::ir::{ChoreoOp, Phase, SpatialProgram, TaggedOp, TensorRole};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::target::{precision_bytes, ComputeCap, SpatialTarget};

#[path = "common/spatial.rs"]
mod spatial_support;
use spatial_support::inputs_from_fills;

fn sn50() -> SpatialTarget {
    RduChip::Sn50.facts().spatial_target()
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

/// Bf16-exact fills: small multiples of 1/64, different per tensor.
fn fill(name: &str, i: usize) -> f64 {
    let salt = name.bytes().map(|b| b as usize).sum::<usize>();
    ((((i + 1) * 37 + salt * 11) % 29) as f64 - 14.0) / 64.0
}

/// How far the attention reaches into its cache.
#[derive(Clone, Copy, Debug)]
enum Reach {
    /// Append row `pos` of each cache, then attend `up to pos`.
    AppendUpTo(usize),
    /// Attend the first `p` positions of the cache as declared.
    Prefix(usize),
    /// Attend the whole cache as declared.
    Whole,
}

struct Shape {
    ctx: usize,
    h: usize,
    kv: usize,
    hd: usize,
    reach: Reach,
}

fn source(s: &Shape, filled: bool) -> String {
    let (aq, akv) = (s.h * s.hd, s.kv * s.hd);
    let mut src = String::from("## Main\n");
    if let Reach::AppendUpTo(pos) = s.reach {
        src += &format!("Let pos be {pos}.\n");
    }
    src += &format!("Let Kc be a {} by {akv} tensor of Real.\nLet Vc be a {} by {akv} tensor of Real.\nLet qp be a 1 by {aq} tensor of Real.\n", s.ctx, s.ctx);
    if matches!(s.reach, Reach::AppendUpTo(_)) {
        src += &format!(
            "Let kp be a 1 by {akv} tensor of Real.\nLet vr be a 1 by {akv} tensor of Real.\n"
        );
    }
    if filled {
        let mut tensors = vec![("Kc", s.ctx * akv), ("Vc", s.ctx * akv), ("qp", aq)];
        if matches!(s.reach, Reach::AppendUpTo(_)) {
            tensors.extend([("kp", akv), ("vr", akv)]);
        }
        for (name, n) in tensors {
            for i in 0..n {
                src += &format!("Set item {} of {name} to {:?}.\n", i + 1, fill(name, i));
            }
        }
    }
    let (h, kv) = (s.h, s.kv);
    src += &match s.reach {
        Reach::AppendUpTo(_) => format!("Set row pos of Kc to kp.\nSet row pos of Vc to vr.\nLet ct be attention of qp over Kc and Vc with {h} heads {kv} groups up to pos.\n"),
        Reach::Prefix(p) => format!("Let ct be attention of qp over Kc and Vc with {h} heads {kv} groups up to {p}.\n"),
        Reach::Whole => format!("Let ct be attention of qp over Kc and Vc with {h} heads {kv} groups.\n"),
    };
    src
}

/// The f64 attention of the shape's filled program: each query head over the first `l` positions of its K/V head.
fn reference(s: &Shape) -> Vec<f64> {
    let akv = s.kv * s.hd;
    let mut k: Vec<f64> = (0..s.ctx * akv).map(|i| fill("Kc", i)).collect();
    let mut v: Vec<f64> = (0..s.ctx * akv).map(|i| fill("Vc", i)).collect();
    let l = match s.reach {
        Reach::AppendUpTo(pos) => {
            for c in 0..akv {
                k[(pos - 1) * akv + c] = fill("kp", c);
                v[(pos - 1) * akv + c] = fill("vr", c);
            }
            pos
        }
        Reach::Prefix(p) => p,
        Reach::Whole => s.ctx,
    };
    let group = s.h / s.kv;
    let mut ct = vec![0.0; s.h * s.hd];
    for head in 0..s.h {
        let x = head / group;
        let scores: Vec<f64> = (0..l)
            .map(|j| {
                (0..s.hd)
                    .map(|e| fill("qp", head * s.hd + e) * k[j * akv + x * s.hd + e])
                    .sum::<f64>()
                    / (s.hd as f64).sqrt()
            })
            .collect();
        let m = scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let w: Vec<f64> = scores.iter().map(|sc| (sc - m).exp()).collect();
        let sum: f64 = w.iter().sum();
        for e in 0..s.hd {
            ct[head * s.hd + e] = (0..l)
                .map(|j| w[j] * v[j * akv + x * s.hd + e])
                .sum::<f64>()
                / sum;
        }
    }
    ct
}

fn composed(src: &str, target: &SpatialTarget) -> SpatialProgram {
    let l = compile_program_to_spatial(src, target).expect("a tensor program");
    let p = l
        .program
        .unwrap_or_else(|| panic!("the attention composes and certifies: {:?}", l.missing));
    assert_eq!(p.phase(), Phase::Verified, "the composed program certifies");
    assert!(
        p.certificate.as_ref().is_some_and(|c| c.recheck(&p)),
        "its certificate rechecks"
    );
    p
}

/// The flash fold's section: the one whose name says it folds the cache's blocks.
fn fold_section(p: &SpatialProgram) -> usize {
    let folds: Vec<usize> = (0..p.sections.len())
        .filter(|k| p.sections[*k].name.contains("flash_blocks"))
        .collect();
    assert_eq!(
        folds.len(),
        1,
        "one fold section: {:?}",
        p.sections.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    folds[0]
}

fn items_of(p: &SpatialProgram, section: usize) -> usize {
    let (first, end) = p.sections[section].items;
    end - first
}

/// Bytes of `tensor` the schedule reads inside `section`, every loop's count multiplied in.
fn read_in(p: &SpatialProgram, section: usize, tensor: &str) -> u64 {
    fn walk(
        p: &SpatialProgram,
        ops: &[TaggedOp],
        mult: u64,
        section: usize,
        tensor: &str,
        n: &mut u64,
    ) {
        let tile = (p.tile.rows * p.tile.cols) as u64;
        for t in ops {
            match &t.op {
                ChoreoOp::Loop { count, body, .. } => {
                    walk(p, body, mult * count.bound() as u64, section, tensor, n)
                }
                ChoreoOp::ReadTiles {
                    tensor: id,
                    block_h,
                    block_w,
                    ..
                } if p.tensor(*id).name == tensor
                    && p.section_of_stage(t.stage) == Some(section) =>
                {
                    *n += mult
                        * (*block_h * *block_w) as u64
                        * tile
                        * precision_bytes(p.tensor(*id).dtype) as u64;
                }
                _ => {}
            }
        }
    }
    let mut n = 0;
    for prog in &p.schedule.as_ref().expect("scheduled").programs {
        walk(p, &prog.ops, 1, section, tensor, &mut n);
    }
    n
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

/// The shape's attention on the chip and on the fp32 twin against the f64 reference; the fold runs one item per K/V
/// head.
fn computes(s: &Shape) {
    let what = format!(
        "{} heads over {} K/V heads of {}, ctx {}, {:?}",
        s.h, s.kv, s.hd, s.ctx, s.reach
    );
    let want = reference(s);
    let vmax = (0..s.ctx * s.kv * s.hd)
        .map(|i| fill("Vc", i).abs())
        .chain((0..s.kv * s.hd).map(|i| fill("vr", i).abs()))
        .fold(0.0f64, f64::max);
    let src = source(s, true);
    for (target, unit, label) in [
        (sn50(), 2f64.powi(-8), "CHIP"),
        (fp32_twin(), 2f64.powi(-20), "WIRING"),
    ] {
        let p = composed(&src, &target);
        let fold = fold_section(&p);
        assert_eq!(
            items_of(&p, fold),
            s.kv,
            "{what}: the fold runs one item per K/V head"
        );
        let out = eval_choreography(&p, &inputs_from_fills(&p))
            .unwrap_or_else(|e| panic!("{what}: {label} eval: {e:?}"));
        let got = values(&p, &out, "ct");
        let bound = (p.sections.len() as f64 + 1.0) * unit * vmax;
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            assert!(
                (g - w).abs() <= bound,
                "{what}: {label} ct[{i}] (head {}) = {g}, reference {w} (bound {bound})",
                i / s.hd
            );
        }
    }
}

// ───────────────────────────── 1. at real width, each cache once ─────────────────────────────

#[test]
fn at_real_width_each_kv_head_streams_its_cache_once_a_step() {
    let s = Shape {
        ctx: 4096,
        h: 24,
        kv: 4,
        hd: 256,
        reach: Reach::AppendUpTo(1000),
    };
    let p = composed(&source(&s, false), &sn50());
    let fold = fold_section(&p);
    assert_eq!(items_of(&p, fold), s.kv, "one fold item per K/V head");
    assert_eq!(
        p.sections[fold].plan.map(|pl| pl.inner.num_blocks),
        Some(s.ctx / 32),
        "each item folds the cache's {} tile rows",
        s.ctx / 32
    );
    for cache in ["Kc", "Vc"] {
        let once = (s.ctx * s.kv * s.hd * 2) as u64;
        assert_eq!(
            read_in(&p, fold, cache),
            once,
            "{cache}: every cached element read exactly once a step ({once} bytes)"
        );
    }
}

/// The SN50 with fp32 beside bf16 in its pointwise and row units, its products bf16: NUM-1's datapath.
fn fp32_pointwise() -> SpatialTarget {
    let mut t = sn50();
    for c in &mut t.compute {
        if let ComputeCap::Elementwise { precisions } | ComputeCap::RowReduce { precisions } = c {
            precisions.insert(Precision::Fp32);
        }
    }
    t
}

#[test]
fn with_fp32_beside_its_bf16_products_each_kv_head_still_folds_once() {
    // The pointwise and row units fp32: the fold's scores, maximum, sums and running output, and the query, key and
    // value pages it streams, at fp32 leave no room for a second page of each streamed page beside them, so those take
    // one. A K/V head's fold still fits a site: one item per K/V head, the cache folded a tile row at a time.
    let s = Shape {
        ctx: 4096,
        h: 24,
        kv: 4,
        hd: 256,
        reach: Reach::AppendUpTo(1000),
    };
    let p = composed(&source(&s, false), &fp32_pointwise());
    let fold = fold_section(&p);
    assert_eq!(items_of(&p, fold), s.kv, "one fold item per K/V head");
    assert_eq!(
        p.sections[fold].plan.map(|pl| pl.inner.num_blocks),
        Some(s.ctx / 32),
        "each item folds the cache's {} tile rows",
        s.ctx / 32
    );
}

// ───────────────────────────── 2. it computes the attention ─────────────────────────────

#[test]
fn a_group_of_two_over_two_kv_heads_after_an_append_computes() {
    computes(&Shape {
        ctx: 64,
        h: 4,
        kv: 2,
        hd: 32,
        reach: Reach::AppendUpTo(40),
    });
}

#[test]
fn a_group_of_three_over_a_cache_ending_inside_a_tile_row_computes() {
    computes(&Shape {
        ctx: 50,
        h: 6,
        kv: 2,
        hd: 32,
        reach: Reach::AppendUpTo(50),
    });
    computes(&Shape {
        ctx: 50,
        h: 6,
        kv: 2,
        hd: 32,
        reach: Reach::Whole,
    });
}

#[test]
fn a_group_of_six_over_one_kv_head_computes() {
    computes(&Shape {
        ctx: 64,
        h: 6,
        kv: 1,
        hd: 32,
        reach: Reach::AppendUpTo(33),
    });
}

#[test]
fn a_group_of_six_with_two_tile_wide_heads_computes() {
    computes(&Shape {
        ctx: 96,
        h: 12,
        kv: 2,
        hd: 64,
        reach: Reach::AppendUpTo(70),
    });
}

#[test]
fn a_literal_prefix_computes() {
    computes(&Shape {
        ctx: 64,
        h: 6,
        kv: 2,
        hd: 32,
        reach: Reach::Prefix(45),
    });
    computes(&Shape {
        ctx: 64,
        h: 6,
        kv: 2,
        hd: 32,
        reach: Reach::Prefix(20),
    });
}

// ───────────────────────────── 3. the regrouping, and where there is nothing to share ─────────────────────────────

#[test]
fn the_regrouping_moves_only_the_queries_and_the_result() {
    let s = Shape {
        ctx: 64,
        h: 6,
        kv: 2,
        hd: 32,
        reach: Reach::Whole,
    };
    let p = composed(&source(&s, false), &sn50());
    let fold = fold_section(&p);
    assert_eq!(
        p.sections.len(),
        3,
        "regroup the queries, fold, regroup the result: {:?}",
        p.sections.iter().map(|x| &x.name).collect::<Vec<_>>()
    );
    assert_eq!(fold, 1, "the fold runs between the two regroupings");
    for k in [0, 2] {
        let touched: std::collections::BTreeSet<&str> = p
            .stages
            .iter()
            .filter(|st| p.section_of_stage(st.id) == Some(k))
            .filter_map(|st| match &st.kind {
                logicaffeine_compile::spatial::ir::StageKind::Load { tensor, .. }
                | logicaffeine_compile::spatial::ir::StageKind::Store { tensor, .. } => {
                    Some(p.tensor(*tensor))
                }
                _ => None,
            })
            .filter(|t| t.role != TensorRole::Param)
            .map(|t| t.name.as_str())
            .collect();
        assert!(
            !touched.contains("Kc") && !touched.contains("Vc"),
            "section {k} ({}) never touches the cache: {touched:?}",
            p.sections[k].name
        );
        assert_eq!(
            touched.len(),
            2,
            "section {k} ({}) moves one activation into another: {touched:?}",
            p.sections[k].name
        );
    }
}

#[test]
fn heads_that_share_nothing_and_blocks_of_rows_keep_an_item_per_query_head() {
    let own = Shape {
        ctx: 64,
        h: 3,
        kv: 3,
        hd: 32,
        reach: Reach::Whole,
    };
    let p = composed(&source(&own, false), &sn50());
    assert_eq!(
        p.sections.len(),
        1,
        "a head per K/V head: the fold alone, nothing to regroup"
    );
    assert_eq!(items_of(&p, fold_section(&p)), 3, "an item per query head");
    // A prefill chunk: 40 query rows of 4 heads over 2 K/V heads, causal from position 1.
    let (t, h, kv, hd) = (40usize, 4usize, 2usize, 32usize);
    let src = format!(
        "## Main\nLet pos be 1.\nLet Kc be a 64 by {} tensor of Real.\nLet Vc be a 64 by {} tensor of Real.\nLet qp be a {t} by {} tensor of Real.\n\
         Let ct be causal attention of qp over Kc and Vc with {h} heads {kv} groups from pos.\n",
        kv * hd,
        kv * hd,
        h * hd
    );
    let p = composed(&src, &sn50());
    assert_eq!(
        items_of(&p, fold_section(&p)),
        t.div_ceil(32) * h,
        "a block of rows: an item per query tile and head"
    );
}
