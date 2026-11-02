//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! QUERY TILES — a prefill chunk's causal attention runs a TILE of query rows per work item, so the rows of a tile
//! share every block of the cache they read.
//!
//! `causal attention of Q over K and V with H heads KV groups from P` over a `T`-row block is flash attention: the cache
//! streams in blocks of one tile row of positions, each folded into a running softmax carried on chip. A work item that
//! owns ONE query row reads every cache block once per row — a 64-row chunk reads each block 64 times per head. An item that owns a TILE of query rows (FlashAttention-2's outer loop) reads each block once per tile: the
//! scores `S = Q_tile·K_blockᵀ` and the values `O = P·V_block` are one contraction stage each, and the causal limit is
//! a per-row operand — row `r` of tile `q` attends positions `1 … P + q·tile + r − 1`.
//!
//! Pinned:
//!   1. The flash block section's work items are the block's query TILES × heads: `⌈T / tile⌉·H` — and one row is `H`,
//!      the decode form's.
//!   2. The rows of a tile share every cache block: a tile's 32 rows move less than twice the DRAM bytes its first 2
//!      rows do over a cache of three blocks (items of one row would move about sixteen times as much), while a second
//!      tile of rows costs about as much again as the first — the traffic follows the tiles, not the rows.
//!   3. A tile computes the VM's attention under the ledger's two-part rule (WIRING: the fp32 twin within a relative
//!      2⁻¹⁶; CHIP: bf16 within `sections + 1` ulps of `Σ p·|v|`) — a full tile and a partial one, a cache of several
//!      blocks folded, GQA and MHA, a literal `P` and a computed one at every bound the block admits — and so does the
//!      prefill cycle (every row appended at its position, then the block attended) at two tiles.
//!   4. NOTHING BUT THE ATTENTION IS STORED. The blocks fold into a running softmax carried on chip — each row's maximum
//!      and sum, the head's weighted values (one fp32 tile row: a head of eight tiles fills the SN50's eight
//!      accumulator tiles) and the block start — so the section's only store writes the attention itself.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_query_tile_attention_lock`
//!
//! ★ MUTATION PROBES (run 2026-09-27 against the fold, in a private copy — break, observe red; logs
//!   `logs/sn50/probe_ffold_*.txt`):
//!   • the row limit drops the row's own index (every row of a tile masked at its first row's limit) → the prefill
//!     cycle, the tile verdicts and the wide head red on WIRING (`-718750` vs VM `-450745`);
//!   • every item reads tile 0's row offset (`q·tile` = 0) → the two-tile cases red on WIRING (`-750000` vs VM
//!     `322726`), the one-tile cases stay green — only a second tile can see it;
//!   • the running values folded unrescaled (`O' = O + O_b`) → the multi-tile cases red on WIRING (`-31528` vs VM
//!     `-31784`); so does the running sum folded unrescaled (`-26236`) — only a later block that raises a row's
//!     maximum can see either;
//!   • the block start frozen at 0 → every block masked as the first: every case red (`-211984` vs VM `-781250`);
//!   • the running maximum never updated → every case red (`62500` vs VM `322726`).
//!   The traffic lock's probe of 2026-09-26 restored the one-row form (a tile's 32 rows then moved sixteen times the
//!   bytes of 2); that form no longer exists — every block of rows is a fold over tiles.
//! ════════════════════════════════════════════════════════════════════════════════════════════════
#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::spatial::eval::eval_choreography;
use logicaffeine_compile::spatial::ir::{Buffer, Phase, SpatialProgram, StageKind};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
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

/// Two query tiles, the second partial; MHA over a cache of three blocks.
const TWO_TILES: Shape = Shape {
    t: 40,
    h: 2,
    kv: 2,
    hd: 32,
    l: 96,
};
/// A tile and one row more; GQA heads of two tiles over a cache of three blocks, the last partial.
const TILE_GQA: Shape = Shape {
    t: 33,
    h: 4,
    kv: 2,
    hd: 64,
    l: 70,
};
/// One whole tile over a cache of three blocks.
const ONE_TILE: Shape = Shape {
    t: 32,
    h: 2,
    kv: 1,
    hd: 32,
    l: 96,
};
/// The tile height on the SN50.
const TILE: usize = 32;

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

fn flash_items(p: &SpatialProgram) -> usize {
    let sec = p
        .sections
        .iter()
        .find(|x| x.name.contains("flash_blocks"))
        .expect("a flash block section");
    sec.items.1 - sec.items.0
}

// ───────────────────────────── 1. the work items are query tiles ─────────────────────────────

#[test]
fn the_flash_section_runs_a_tile_of_query_rows_per_item() {
    for s in [TWO_TILES, TILE_GQA, ONE_TILE, Shape { t: 1, ..TWO_TILES }] {
        let (p, _) = composed(&block(&s, 1, Bound::Named).src, &sn50(), &format!("{s:?}"));
        assert_eq!(
            flash_items(&p),
            s.t.div_ceil(TILE) * s.h,
            "{s:?}: one item per query tile and head"
        );
    }
}

// ───────────────────────────── 2. the rows of a tile share every cache block ─────────────────────────────

#[test]
fn the_rows_of_a_tile_share_every_cache_block_they_read() {
    let dram = |t: usize| {
        let s = Shape {
            t,
            l: 128,
            ..ONE_TILE
        };
        let (p, _) = composed(
            &block(&s, 1, Bound::Named).src,
            &sn50(),
            &format!("{t} rows"),
        );
        work_of(&p, &sn50()).dram_bytes
    };
    let (two, tile, two_tiles) = (dram(2), dram(TILE), dram(2 * TILE));
    assert!(tile < 2 * two, "a tile's 32 rows move {tile} DRAM bytes, its first 2 rows {two}: the tile's rows must share the cache blocks they read");
    assert!(two_tiles * 2 > 3 * tile, "a second tile of rows moves {two_tiles} bytes against one tile's {tile}: the traffic follows the tiles");
}

// ───────────────────────────── 3. a tile computes the VM's attention ─────────────────────────────

#[test]
fn a_query_tile_computes_the_vms_attention() {
    for s in [TWO_TILES, TILE_GQA, ONE_TILE] {
        for p in bounds(&s) {
            for bound in [Bound::Literal, Bound::Named] {
                verdict(&block(&s, p, bound), &format!("{s:?} from {p} ({bound:?})"));
            }
        }
    }
}

#[test]
fn the_prefill_cycle_at_two_tiles_computes_the_vms_attention() {
    for p in [1, 9] {
        verdict(
            &prefill_cycle(
                &Shape {
                    t: 40,
                    h: 2,
                    kv: 1,
                    hd: 32,
                    l: 96,
                },
                p,
            ),
            &format!("prefill cycle from {p}"),
        );
    }
}

// ───────────────────────────── 4. nothing but the attention is stored ─────────────────────────────

/// A tile row of one head eight tiles wide over a cache of two blocks: the head's running values alone fill the
/// SN50's eight accumulator tiles.
const WIDE: Shape = Shape {
    t: 32,
    h: 1,
    kv: 1,
    hd: 256,
    l: 40,
};

#[test]
fn nothing_but_the_attention_is_stored_and_the_running_state_stays_on_chip() {
    let (p, _) = composed(&block(&WIDE, 3, Bound::Named).src, &sn50(), "wide heads");
    let flash = p
        .sections
        .iter()
        .position(|s| s.name.contains("flash_blocks"))
        .expect("a flash block section");
    // The section's only store writes the attention itself: no partial, no statistic, ever leaves the site.
    let stored: std::collections::BTreeSet<&str> = p
        .stages
        .iter()
        .filter(|s| p.section_of_stage(s.id) == Some(flash))
        .filter_map(|s| match &s.kind {
            StageKind::Store { tensor, .. } => Some(p.tensor(*tensor).name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        stored,
        std::collections::BTreeSet::from(["C"]),
        "the fold stores the attention and nothing else"
    );
    // The running softmax is carried: each row's maximum and sum, the head's weighted values, and the block start.
    let carried: Vec<&Buffer> = p
        .carries()
        .keys()
        .map(|b| p.buffer(*b))
        .filter(|b| p.section_of_buffer(b.id) == Some(flash))
        .collect();
    for state in ["run_max", "run_sum", "run_out", "block_start"] {
        assert!(
            carried.iter().any(|b| b.name.contains(state)),
            "`{state}` is a carry of the fold: {:?}",
            carried.iter().map(|b| &b.name).collect::<Vec<_>>()
        );
    }
    let head_t = WIDE.hd / TILE;
    for b in carried.iter().filter(|b| b.name.contains("run_out")) {
        assert_eq!(
            (b.kind.page_tiles(), b.dtype),
            (head_t, Precision::Fp32),
            "{}: the running values are one fp32 tile row of the head — {head_t} accumulator tiles",
            b.name
        );
    }
    verdict(&block(&WIDE, 3, Bound::Named), "a tile row of wide heads");
}
