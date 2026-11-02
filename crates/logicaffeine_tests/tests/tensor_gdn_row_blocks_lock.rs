//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! GATED-DELTANET ROW BLOCKS — the recurrent layer's sugars over a block of rows, as a prefill chunk writes them.
//!
//! A prefill chunk is `R` rows of one sequence at consecutive positions. A Gated-DeltaNet layer takes them IN ORDER:
//! each row enters the conv ring after the one before it, and each updates the recurrent memory the next row reads.
//! The block forms state that order once, so the chunk is one statement per sugar instead of one per row.
//!
//! Pinned, on the VM and the tree-walker (which must agree), each against its rows fed in turn:
//!   1. `causal conv of X with weight W state S over T taps` over an `R`-row `X` convolves EVERY row, row `r` against the
//!      ring after rows `1 … r − 1` entered it — its output rows and the ring it leaves are exactly the single-row convs
//!      in turn. (It convolved row 1 alone and bound a one-row result.)
//!   2. `decay of Ar with bias Dt rate Alog` over an `R`-row `Ar`, with one bias and one rate per head, decays every row
//!      by those per-head values — exactly the single-row decays.
//!   3. `gated delta of Q over K and V with gate Z beta B decay A state S norm GN with H heads G groups` over `R`-row
//!      operands runs the recurrence row after row — its output rows and the memory it leaves are exactly the
//!      single-row steps in turn.
//!
//!   4. THE SN50 COMPUTES THE BLOCKS. Each block form lowers whole and evaluates to the VM's values under the two-part
//!      rule (WIRING: the fp32 twin within a relative 2⁻¹⁶; CHIP: bf16 within `sections + 1` ulps of the value): the
//!      decay over the `rows × heads` matrix a projection writes, heads not a whole number of tiles; the conv — its
//!      output rows and the ring it leaves — for a block shorter than the ring, channels narrower than a tile, and
//!      rows spanning two tile rows (each tile row's first rows reading the one before it); and the gated delta — its
//!      output rows and the memory it leaves — over the flat memory and the blocked one, both the fp32 twin and the bf16
//!      chip held to each shown tensor's scale (2⁻¹⁶ of it; `rows + 1` bf16 ulps of it): a cancelling fold keeps its
//!      terms' rounding, and a chunked lowering associates its sums unlike the VM.
//!   5. THE BLOCK IS ITS ROWS' STEPS OR ITS CHUNKS. A short block is its rows' steps, each reading its row of the
//!      operands in place; the memory carries and the output is assembled whole (never carried). A longer one is lowered
//!      a CHUNK (a tile row of rows) at a time — its sections do not depend on a chunk's rows, each chunk adds the same,
//!      and a chunk costs fewer than its rows stepped — and computes the VM's values for model-like operands (l2-normed
//!      keys, sigmoid gates) at 12, 32 and 40 rows and a two-tile head over two key heads. `V` and `O` the program lays out
//!      dense are read and written dense, a 33-row block (a partial chunk, relayouts in two parts) included.
//!   6. THE MEMORY MOVES ONCE A STEP. A row's step reads the recurrent memory once and writes it once: its prediction
//!      `u = S·k`, its update `S' = α·S + β·(v − α·u)·kᵀ` and its output `S'·q = α·(S·q) + β·(v − α·u)·(k·q)` are one
//!      pass over each head's memory — at Qwen3.8's width (48 value heads over 16 key heads of 128) and for a block
//!      stepped row by row (once per row).
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test tensor_gdn_row_blocks_lock`
//!
//! ★ MUTATION PROBES (each run in a private copy of the tree; every one RED, 2026-09-26):
//!   • the block decay reads its bias down the rows of its tile (per element, not per head) → WIRING `decay block 3×2`
//!     Show #3: 1000000 vs 937804.
//!   • the block decay drops its rate → Show #1: 1543691 vs 940771.
//!   • the block decay's tile rows all read tile row 0 → `decay block 40×48` Show #1537: 940771 vs 956335.
//!   • the block conv's halo is 0 in every tile row (row 1 reads rows before the block, not the ring) → `conv block 3×4,
//!     4 taps` Show #1: 13672 vs −350586.
//!   • the ring the block conv leaves is shifted one row → Show #13 (the ring): −218750 vs −93750.
//!   • each tile row of the block conv reads ITSELF as the row before → `conv block 40×64` Show #2049: −370605 vs −1459961.
//!   • the flat form reads every row's operands at row 0 → `gated delta block 3 rows, d 2` WIRING Show #5: −47161 vs
//!     17563.
//!   • the blocked form reads every row's `α`/`β` at plane row 0 → `2 rows, d 32` Show #65: 44566 vs 44658.
//!   • the blocked form reads every row's `k` at row 0 → Show #65: −24441 vs 44658.
//!   • the blocked form writes every row's output at row 0 → Show #1: 44658 vs −55365.
//!   • a dense output's rows are all written at plane row 0 → `dense … 2 rows, d 32` Show #1: 13761 vs −41254.
//!   • the flat form's row write merges every row into row 1 → Show #1: 16966 vs −46305 (and the dense twin's).
//!   ONE PASS (2026-09-27, the fused step; logs `logs/sn50/probe_gd1_*.txt`):
//!   • every a-block's update reads a-block 0's values → `1 row, d 64` WIRING Show #2305: 893368 vs 733189.
//!   • the values drop their head group (`v` loaded at the k-head's step) → `2 rows, d 32` Show #33: 1497 vs 1694.
//!   • every a-block stored at a-block 0 → the step is refused (`OutputOverlap` on the memory) and promise 6 reds.
//!   • the output drops its update term (`α·w` alone) → Show #1: −46120 vs −55365.
//!   • the look reads `k` for `q` → Show #1: −4886 vs −55365.
//!   • the update not ordered after its load (the `u_j` edge dropped) → the certificate refuses every shape:
//!     `StateOrder` — the write is not downstream of the read.
//!   CHUNKED:
//!   • a log ratio counts rows past its own (`s < t + 1` dropped) → `chunked … 4 rows` Show #1: −78479 vs −41254.
//!   • a solve factor drops its `+ X` → Show #1: −92235 vs −41254.
//!   • `A` takes its diagonal (the strict mask `i < t` made `i ≤ t`) → Show #1: −57155 vs −41254.
//!   • the memory update reads the ratio row 0 instead of row `C − 1` → Show #257 (the memory): 33018 vs −72272.
//!   • a chunk's padding rows are not masked out of `G` → with a declared decay (`α = 0` past the block) Show #2049:
//!     0 vs 32547. (With a computed decay the padding holds `σ(0)` and the mask is invisible — which is why the
//!     declared-decay test exists.)
//!   • the chunked form is never chosen → one chunk of 31 rows and one of 32 differ in sections.
//!   • the certificate holds an activation to ONE writing section → the 33- and 40-row blocks do not certify
//!     (`ActivationOrder`).
//!   • a relayout of rows not whole tile rows is not split → `ReadOutOfRange` past the flat table (whole-tile rows and
//!     narrow rows alike).
//!   • the recognizer forgets the rows (a block recognized as one row) → the chip lock refuses (`O` has no element 4),
//!     and the structure lock's section count moves.
//!   • the output is left out of the step's state → compose refuses the rows' writes (`OutputOverlap`), both locks red.
//!   • the output is not counted as assembled whole → the structure lock: `carried` names it.
//! ════════════════════════════════════════════════════════════════════════════════════════════════
#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::spatial::eval::eval_choreography;
use logicaffeine_compile::spatial::ir::{Phase, SpatialProgram};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use tensor_probes::{inputs_from_fills, oracle_ints, shown_from_outputs};

fn val(seed: usize) -> f64 {
    (((seed.wrapping_mul(7919).wrapping_add(13)) % 129) as f64 - 64.0) / 64.0
}

fn declare(name: &str, rows: usize, cols: usize, salt: usize) -> String {
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of Real.\n");
    for i in 0..rows * cols {
        s += &format!(
            "Set item {} of {name} to {:?}.\n",
            i + 1,
            val(i * 31 + salt)
        );
    }
    s
}

fn lines(s: &str) -> Vec<String> {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Tree-walker and VM agree; the output, or a panic naming the error.
fn ran(src: &str, what: &str) -> Vec<String> {
    let (tw, vm) = (tw_outcome(src), vm_outcome(src));
    assert_eq!(
        (lines(&tw.output), &tw.error),
        (lines(&vm.output), &vm.error),
        "{what}: the VM and the tree-walker disagree"
    );
    assert_eq!(vm.error, None, "{what}: {:?}", vm.error);
    lines(&vm.output)
}

fn show_all(name: &str, n: usize) -> String {
    (1..=n)
        .map(|i| format!("Show item {i} of {name}.\n"))
        .collect()
}

// ───────────────────────────── 1. causal conv ─────────────────────────────

fn conv_block(rows: usize, chan: usize, taps: usize) -> String {
    let mut s = String::from("## Main\n");
    s += &declare("X", rows, chan, 1);
    s += &declare("W", taps, chan, 2);
    s += &declare("S", taps, chan, 3);
    s += &format!("Let Y be causal conv of X with weight W state S over {taps} taps.\n");
    s += &show_all("Y", rows * chan);
    s += &show_all("S", taps * chan);
    s
}

fn conv_rows(rows: usize, chan: usize, taps: usize) -> String {
    let mut s = String::from("## Main\n");
    s += &declare("X", rows, chan, 1);
    s += &declare("W", taps, chan, 2);
    s += &declare("S", taps, chan, 3);
    for r in 1..=rows {
        s += &format!("Let x{r} be row {r} of X.\nLet y{r} be causal conv of x{r} with weight W state S over {taps} taps.\n");
    }
    for r in 1..=rows {
        s += &show_all(&format!("y{r}"), chan);
    }
    s += &show_all("S", taps * chan);
    s
}

#[test]
fn a_causal_conv_over_rows_convolves_each_row_after_the_one_before() {
    for (rows, chan, taps) in [(3, 4, 2), (5, 3, 4), (1, 6, 4), (2, 5, 3)] {
        let block = ran(
            &conv_block(rows, chan, taps),
            &format!("conv block {rows}×{chan}, {taps} taps"),
        );
        assert_eq!(
            block.len(),
            rows * chan + taps * chan,
            "every output row and the whole ring are shown"
        );
        assert_eq!(
            block,
            ran(&conv_rows(rows, chan, taps), "conv rows"),
            "{rows}×{chan}, {taps} taps: the block is its rows in turn, output and ring"
        );
    }
}

// ───────────────────────────── 2. decay ─────────────────────────────

fn decay_block(rows: usize, n: usize) -> String {
    let mut s = String::from("## Main\n");
    s += &declare("Ar", rows, n, 4);
    s += &declare("Dt", 1, n, 5);
    s += &declare("Al", 1, n, 6);
    s += "Let D be decay of Ar with bias Dt rate Al.\n";
    s += &show_all("D", rows * n);
    s
}

fn decay_rows(rows: usize, n: usize) -> String {
    let mut s = String::from("## Main\n");
    s += &declare("Ar", rows, n, 4);
    s += &declare("Dt", 1, n, 5);
    s += &declare("Al", 1, n, 6);
    for r in 1..=rows {
        s += &format!(
            "Let a{r} be row {r} of Ar.\nLet d{r} be decay of a{r} with bias Dt rate Al.\n"
        );
    }
    for r in 1..=rows {
        s += &show_all(&format!("d{r}"), n);
    }
    s
}

#[test]
fn a_decay_over_rows_decays_each_row_by_the_per_head_bias_and_rate() {
    for (rows, n) in [(3, 2), (4, 5), (1, 3)] {
        let block = ran(&decay_block(rows, n), &format!("decay block {rows}×{n}"));
        assert_eq!(block.len(), rows * n);
        assert_eq!(
            block,
            ran(&decay_rows(rows, n), "decay rows"),
            "{rows}×{n}: the block is its rows"
        );
    }
}

#[test]
fn a_decay_whose_bias_is_neither_per_head_nor_per_element_is_refused() {
    let mut s = String::from("## Main\n");
    s += &declare("Ar", 3, 4, 4);
    s += &declare("Dt", 1, 3, 5);
    s += &declare("Al", 1, 3, 6);
    s += "Let D be decay of Ar with bias Dt rate Al.\nShow item 1 of D.\n";
    let out = vm_outcome(&s);
    let err = format!("{:?}", out.error);
    assert!(
        out.error.is_some() && err.contains("decay"),
        "a 3-value bias for 4 heads is refused by name: {err}"
    );
}

// ───────────────────────────── 3. gated delta ─────────────────────────────

/// A gated-delta shape: `rows` rows, `nk` key heads, `nv` value heads, head dimension `d`.
#[derive(Clone, Copy, Debug)]
struct Gd {
    rows: usize,
    nk: usize,
    nv: usize,
    d: usize,
}

fn gd_operands(g: &Gd) -> String {
    let (qk, vw) = (g.nk * g.d, g.nv * g.d);
    let mut s = String::new();
    s += &declare("Q", g.rows, qk, 7);
    s += &declare("K", g.rows, qk, 8);
    s += &declare("V", g.rows, vw, 9);
    s += &declare("Z", g.rows, vw, 10);
    s += &declare("B", g.rows, g.nv, 11);
    s += &declare("A", g.rows, g.nv, 12);
    s += &declare("S", vw, g.d, 13);
    s += &declare("GN", 1, g.d, 14);
    s
}

fn gd_block(g: &Gd) -> String {
    let mut s = String::from("## Main\n");
    s += &gd_operands(g);
    s += &format!("Let O be gated delta of Q over K and V with gate Z beta B decay A state S norm GN with {} heads {} groups.\n", g.nv, g.nk);
    s += &show_all("O", g.rows * g.nv * g.d);
    s += &show_all("S", g.nv * g.d * g.d);
    s
}

fn gd_rows(g: &Gd) -> String {
    let mut s = String::from("## Main\n");
    s += &gd_operands(g);
    for r in 1..=g.rows {
        for (x, m) in [
            ("q", "Q"),
            ("k", "K"),
            ("v", "V"),
            ("z", "Z"),
            ("b", "B"),
            ("a", "A"),
        ] {
            s += &format!("Let {x}{r} be row {r} of {m}.\n");
        }
        s += &format!("Let o{r} be gated delta of q{r} over k{r} and v{r} with gate z{r} beta b{r} decay a{r} state S norm GN with {} heads {} groups.\n", g.nv, g.nk);
    }
    for r in 1..=g.rows {
        s += &show_all(&format!("o{r}"), g.nv * g.d);
    }
    s += &show_all("S", g.nv * g.d * g.d);
    s
}

#[test]
fn a_gated_delta_over_rows_runs_the_recurrence_row_after_row() {
    for g in [
        Gd {
            rows: 3,
            nk: 1,
            nv: 2,
            d: 2,
        },
        Gd {
            rows: 4,
            nk: 2,
            nv: 4,
            d: 2,
        },
        Gd {
            rows: 1,
            nk: 1,
            nv: 2,
            d: 2,
        },
    ] {
        let block = ran(&gd_block(&g), &format!("gated delta block {g:?}"));
        assert_eq!(
            block.len(),
            g.rows * g.nv * g.d + g.nv * g.d * g.d,
            "every output row and the whole memory are shown"
        );
        assert_eq!(
            block,
            ran(&gd_rows(&g), "gated delta rows"),
            "{g:?}: the block is its rows in turn, output and memory"
        );
    }
}

// ───────────────────────────── 4. the SN50 computes the blocks ─────────────────────────────

fn sn50() -> SpatialTarget {
    SpatialTarget::sn50_rdu()
}

/// The SN50 with fp32 datapaths. A matmul takes the narrowest float operand its datapath offers, so the twin's matmul
/// offers fp32 in place of bf16; the pointwise and row datapaths take the widest, so fp32 beside it suffices.
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

fn composed(src: &str, target: &SpatialTarget, what: &str) -> (SpatialProgram, usize) {
    let pl = compile_program_to_spatial(src, target)
        .unwrap_or_else(|e| panic!("{what}: walk refused: {e:?}"));
    let sp = pl
        .whole()
        .unwrap_or_else(|| panic!("{what}: not lowered whole: {}", pl.refusal()))
        .clone();
    assert_eq!(sp.phase(), Phase::Verified, "{what}: the program certifies");
    assert!(
        sp.certificate.as_ref().is_some_and(|c| c.recheck(&sp)),
        "{what}: its certificate rechecks"
    );
    (sp, pl.sections)
}

/// Every `Show item i of X.` as `Show round(item i of X * 1000000).`.
fn rounded(src: &str) -> String {
    src.lines()
        .map(|l| {
            match l
                .strip_prefix("Show item ")
                .and_then(|r| r.strip_suffix('.'))
            {
                Some(item) => format!("Show round(item {item} * 1000000).\n"),
                None => format!("{l}\n"),
            }
        })
        .collect()
}

fn ulp_bf16(x: f64) -> f64 {
    let e = if x == 0.0 {
        -126
    } else {
        (x.abs().log2().floor() as i32).max(-126)
    };
    2f64.powi(e - 7)
}

/// How far the bf16 chip may sit from the VM.
#[derive(Clone, Copy)]
enum Chip<'a> {
    /// `sections + 1` ulps of each value: each section rounds once — maps and short folds.
    PerValue,
    /// `depth + 1` ulps of the largest value of each shown tensor, the shows split into tensors by `spans`: a `d`-term
    /// fold that cancels toward zero keeps the rounding of its terms, not of its result, and a recurrence's memory
    /// rounds once more with each of its `depth` rows. The fp32 twin is held to 2⁻¹⁶ of the same scales.
    Normwise { spans: &'a [usize], depth: usize },
}

/// The two-part verdict against the VM: the fp32 twin within a relative 2⁻¹⁶, the bf16 chip within the `Chip` bound.
fn chip_verdict(src: &str, what: &str) {
    chip_verdict_within(src, what, Chip::PerValue);
}

fn chip_verdict_within(src: &str, what: &str, bound: Chip) {
    let src = rounded(src);
    let want = oracle_ints(&ran(&src, what).join("\n")).expect("ints");
    let (twin, _) = composed(&src, &fp32_twin(), what);
    let got = shown_from_outputs(
        &src,
        &twin,
        &eval_choreography(&twin, &inputs_from_fills(&twin).expect("fills"))
            .unwrap_or_else(|e| panic!("{what}: fp32 eval: {e:?}")),
    )
    .expect("shown");
    // The fp32 twin within 2⁻¹⁶ of each value — or, for a recurrence whose folds cancel and whose lowering may
    // associate its sums unlike the VM, of each shown tensor's scale.
    let wiring: Vec<f64> = match bound {
        Chip::PerValue => want.iter().map(|v| (*v as f64).abs() / 65536.0).collect(),
        Chip::Normwise { spans, .. } => {
            let mut at = 0;
            spans
                .iter()
                .flat_map(|&n| {
                    let scale = want[at..at + n].iter().map(|v| v.abs()).max().unwrap_or(0) as f64;
                    at += n;
                    std::iter::repeat_n(scale / 65536.0, n)
                })
                .collect()
        }
    };
    for (i, ((g, v), b)) in got.iter().zip(&want).zip(&wiring).enumerate() {
        assert!(
            ((g - v).abs() as f64) <= b.max(1.0),
            "{what}: WIRING Show #{}: fp32 twin {g} vs VM {v} (bound {:.0})",
            i + 1,
            b.max(1.0)
        );
    }
    let (chip, sections) = composed(&src, &sn50(), what);
    let got = shown_from_outputs(
        &src,
        &chip,
        &eval_choreography(&chip, &inputs_from_fills(&chip).expect("fills"))
            .unwrap_or_else(|e| panic!("{what}: bf16 eval: {e:?}")),
    )
    .expect("shown");
    let bounds: Vec<f64> = match bound {
        Chip::PerValue => want
            .iter()
            .map(|v| (sections + 1) as f64 * ulp_bf16(*v as f64 / 1e6) * 1e6)
            .collect(),
        Chip::Normwise { spans, depth } => {
            assert_eq!(
                spans.iter().sum::<usize>(),
                want.len(),
                "{what}: the spans cover every show"
            );
            let mut at = 0;
            spans
                .iter()
                .flat_map(|&n| {
                    let scale =
                        want[at..at + n].iter().map(|v| v.abs()).max().unwrap_or(0) as f64 / 1e6;
                    at += n;
                    std::iter::repeat_n((depth + 1) as f64 * ulp_bf16(scale) * 1e6, n)
                })
                .collect()
        }
    };
    for (i, ((g, v), b)) in got.iter().zip(&want).zip(&bounds).enumerate() {
        let b = b.max(1.0);
        assert!(
            ((g - v).abs() as f64) <= b,
            "{what}: CHIP Show #{}: bf16 {g} vs VM {v} (bound {b:.0})",
            i + 1
        );
    }
}

#[test]
fn the_block_decay_lowers_on_sn50_and_computes_the_vms_values() {
    for (rows, n) in [(3, 2), (40, 48)] {
        chip_verdict(&decay_block(rows, n), &format!("decay block {rows}×{n}"));
    }
}

#[test]
fn the_block_conv_lowers_on_sn50_and_computes_the_vms_values() {
    // A block shorter than the ring; channels narrower than a tile; two tile rows of a flat ring; a ring wider than a
    // site's row (a `taps × chan` matrix, as the real width's is) over one and over two column blocks.
    for (rows, chan, taps) in [
        (3, 4, 4),
        (5, 3, 2),
        (40, 64, 4),
        (40, 128, 4),
        (33, 512, 4),
    ] {
        chip_verdict(
            &conv_block(rows, chan, taps),
            &format!("conv block {rows}×{chan}, {taps} taps"),
        );
    }
}

#[test]
fn the_block_gated_delta_lowers_on_sn50_and_computes_the_vms_values() {
    // The flat memory (every head's `d × d` in one tile) with one and with two key heads; the blocked memory with `d` one
    // tile (two v-heads sharing a key head); and with `d` two tiles — a head's memory walked a-block by a-block — over
    // two key heads each shared by two v-heads, one row (the decode step) and a block.
    for g in [
        Gd {
            rows: 3,
            nk: 1,
            nv: 2,
            d: 2,
        },
        Gd {
            rows: 4,
            nk: 2,
            nv: 4,
            d: 2,
        },
        Gd {
            rows: 2,
            nk: 1,
            nv: 2,
            d: 32,
        },
        Gd {
            rows: 1,
            nk: 2,
            nv: 4,
            d: 64,
        },
        Gd {
            rows: 2,
            nk: 2,
            nv: 4,
            d: 64,
        },
    ] {
        let spans = [g.rows * g.nv * g.d, g.nv * g.d * g.d];
        chip_verdict_within(
            &gd_block(&g),
            &format!("gated delta block {g:?}"),
            Chip::Normwise {
                spans: &spans,
                depth: g.rows,
            },
        );
    }
}

#[test]
fn a_gated_delta_whose_heads_are_not_whole_tiles_lowers_on_sn50_and_computes_the_vms_values() {
    // Heads narrower than a tile, or not a whole number of tiles, as tiny and odd configurations carry them, each memory
    // past the flat form's one tile: six heads of 16 over two key heads (Qwen3-Next's tiny twin), a decode step and a
    // block; 32 heads of 8 over four; eight heads of 48 over two; two heads of 80 over one in a two-row block. Each
    // head is widened to whole tiles on the way in and moved back to the program's layout on the way out, so the
    // memory the program shows is its own.
    for g in [
        Gd {
            rows: 1,
            nk: 2,
            nv: 6,
            d: 16,
        },
        Gd {
            rows: 3,
            nk: 2,
            nv: 6,
            d: 16,
        },
        Gd {
            rows: 1,
            nk: 4,
            nv: 32,
            d: 8,
        },
        Gd {
            rows: 1,
            nk: 2,
            nv: 8,
            d: 48,
        },
        Gd {
            rows: 2,
            nk: 1,
            nv: 2,
            d: 80,
        },
    ] {
        let spans = [g.rows * g.nv * g.d, g.nv * g.d * g.d];
        chip_verdict_within(
            &gd_block(&g),
            &format!("gated delta block {g:?}"),
            Chip::Normwise {
                spans: &spans,
                depth: g.rows,
            },
        );
    }
}

#[test]
fn a_gated_delta_whose_memory_is_more_than_a_page_of_its_row_lowers_on_sn50_and_computes_the_vms_values() {
    // Memories a 32 × 32 tile would hold but a row of one accumulator page (eight tiles of 32) would not, which the flat
    // form keeps as one row: four heads of 16 over two key heads (Qwen 3.5's tiny twin, 1,024 elements), a decode step
    // and a block; two heads of 16 over one (512); three heads of 12 over one (432); one head of 24 (576). Each is the
    // memory of a step the flat form cannot hold, so it is blocked by head.
    for g in [
        Gd {
            rows: 1,
            nk: 2,
            nv: 4,
            d: 16,
        },
        Gd {
            rows: 3,
            nk: 2,
            nv: 4,
            d: 16,
        },
        Gd {
            rows: 1,
            nk: 1,
            nv: 2,
            d: 16,
        },
        Gd {
            rows: 1,
            nk: 1,
            nv: 3,
            d: 12,
        },
        Gd {
            rows: 2,
            nk: 1,
            nv: 1,
            d: 24,
        },
    ] {
        let spans = [g.rows * g.nv * g.d, g.nv * g.d * g.d];
        chip_verdict_within(
            &gd_block(&g),
            &format!("gated delta block {g:?}"),
            Chip::Normwise {
                spans: &spans,
                depth: g.rows,
            },
        );
    }
}

#[test]
fn a_block_gated_delta_is_its_rows_steps_carrying_the_memory_and_assembling_the_output() {
    use logicaffeine_compile::spatial::pipeline::ProgramLowering;
    let lowered = |src: &str, what: &str| -> ProgramLowering {
        let pl = compile_program_to_spatial(src, &sn50())
            .unwrap_or_else(|e| panic!("{what}: walk refused: {e:?}"));
        assert!(
            pl.whole().is_some(),
            "{what}: lowered whole: {}",
            pl.refusal()
        );
        pl
    };
    for g in [
        Gd {
            rows: 3,
            nk: 1,
            nv: 2,
            d: 2,
        },
        Gd {
            rows: 2,
            nk: 1,
            nv: 2,
            d: 32,
        },
    ] {
        let block = lowered(&gd_block(&g), &format!("block {g:?}"));
        let rows = lowered(&gd_rows(&g), &format!("rows {g:?}"));
        assert!(block.sections <= rows.sections + g.rows, "{g:?}: the block is at most its rows' gathers and steps and one row write each — {} sections against {}", block.sections, rows.sections + g.rows);
        assert_eq!(
            block.state,
            ["S", "O"],
            "{g:?}: the memory, and the output each row writes in place, are the step's state"
        );
        assert_eq!(
            block.carried,
            ["S"],
            "{g:?}: every output row is written before anything reads it — only the memory carries"
        );
    }
    let one = lowered(
        &gd_block(&Gd {
            rows: 1,
            nk: 1,
            nv: 2,
            d: 2,
        }),
        "one row",
    );
    assert_eq!(
        one.state,
        ["S"],
        "a one-row step's output is its result, not state"
    );
}

/// The block as the model feeds it — `Q` and `K` l2-normalized per head, `β` and `α` through a sigmoid, so the memory
/// update contracts and a long block stays bounded — with `V` and `O` matrices the program lays out dense (`V` a
/// pointwise op's result, `O` a pointwise op's operand): the step reads a dense operand a row at a time and writes each
/// output row into a dense table.
fn gd_dense(g: &Gd) -> String {
    let (qk, vw) = (g.nk * g.d, g.nv * g.d);
    let mut s = String::from("## Main\n");
    s += &declare("Q0", g.rows, qk, 7);
    s += &declare("K0", g.rows, qk, 8);
    s += &declare("V0", g.rows, vw, 9);
    s += &declare("Z", g.rows, vw, 10);
    s += &declare("B0", g.rows, g.nv, 11);
    s += &declare("A0", g.rows, g.nv, 12);
    s += &declare("S", vw, g.d, 13);
    s += &declare("GN", 1, g.d, 14);
    let heads = g.rows * g.nk;
    s += &format!("Let Q be l2norm of Q0 across {heads} heads.\nLet K be l2norm of K0 across {heads} heads.\n");
    s += "Let B be sigmoid of B0.\nLet A be sigmoid of A0.\nLet V be V0 plus V0.\n";
    s += &format!("Let O be gated delta of Q over K and V with gate Z beta B decay A state S norm GN with {} heads {} groups.\n", g.nv, g.nk);
    s += "Let P be O plus O.\n";
    s += &show_all("P", g.rows * vw);
    s += &show_all("S", vw * g.d);
    s
}

#[test]
fn the_block_gated_delta_reads_and_writes_dense_tables_on_sn50() {
    // The flat memory (its rows merged into the dense output by the row write); the blocked memory, whose rows write
    // plane row `r mod tile` of the output's tile row in place; a block of 33 rows, whose last row starts the output's
    // second tile row; and a head two tiles wide over two key heads.
    for g in [
        Gd {
            rows: 3,
            nk: 1,
            nv: 2,
            d: 2,
        },
        Gd {
            rows: 2,
            nk: 1,
            nv: 2,
            d: 32,
        },
        Gd {
            rows: 33,
            nk: 1,
            nv: 2,
            d: 32,
        },
        Gd {
            rows: 2,
            nk: 2,
            nv: 4,
            d: 64,
        },
    ] {
        let src = gd_dense(&g);
        let rp =
            logicaffeine_compile::spatial::pipeline::dense_tables_of(&src, &sn50()).expect("walks");
        assert!(
            rp.contains("V") && rp.contains("O"),
            "{g:?}: V and O are matrices the program lays out dense: {rp:?}"
        );
        let spans = [g.rows * g.nv * g.d, g.nv * g.d * g.d];
        chip_verdict_within(
            &src,
            &format!("dense gated delta block {g:?}"),
            Chip::Normwise {
                spans: &spans,
                depth: g.rows,
            },
        );
    }
}

#[test]
fn a_long_block_gated_delta_is_lowered_a_chunk_at_a_time() {
    // A chunk is a tile row of the block's rows: its sections do not depend on how many rows it holds, each further
    // chunk adds the same sections, and a chunk costs fewer sections than its rows stepped one at a time.
    let sections = |rows: usize| {
        let pl = compile_program_to_spatial(
            &gd_dense(&Gd {
                rows,
                nk: 1,
                nv: 2,
                d: 32,
            }),
            &sn50(),
        )
        .unwrap_or_else(|e| panic!("{rows} rows: {e:?}"));
        assert!(
            pl.whole().is_some(),
            "{rows} rows: lowered whole: {}",
            pl.refusal()
        );
        pl.sections
    };
    let (s31, s32, s64, s96) = (sections(31), sections(32), sections(64), sections(96));
    assert_eq!(
        s31, s32,
        "one chunk of 31 rows and one of 32 have the same sections"
    );
    assert_eq!(
        s64 - s32,
        s96 - s64,
        "each further chunk adds the same sections"
    );
    assert!(
        s64 - s32 < 32,
        "a chunk ({} sections) costs fewer than its 32 rows stepped one at a time (32)",
        s64 - s32
    );
}

#[test]
fn the_chunked_block_gated_delta_computes_the_vms_values_on_sn50() {
    // A short block (a partial chunk of 12 rows — from 11 rows a chunk costs fewer sections than its rows stepped),
    // one full chunk, a full chunk and a partial one, and a head two tiles wide over two key heads.
    for g in [
        Gd {
            rows: 12,
            nk: 1,
            nv: 2,
            d: 32,
        },
        Gd {
            rows: 32,
            nk: 1,
            nv: 2,
            d: 32,
        },
        Gd {
            rows: 40,
            nk: 1,
            nv: 2,
            d: 32,
        },
        Gd {
            rows: 40,
            nk: 2,
            nv: 4,
            d: 64,
        },
    ] {
        let spans = [g.rows * g.nv * g.d, g.nv * g.d * g.d];
        chip_verdict_within(
            &gd_dense(&g),
            &format!("chunked gated delta block {g:?}"),
            Chip::Normwise {
                spans: &spans,
                depth: g.rows,
            },
        );
    }
}

/// [`gd_dense`] with the decay DECLARED — values in `(0.1, 0.9)` — rather than computed: a declared tensor's padding
/// rows are zero, so a chunk past the block's last row holds `α = 0`, whose `ln α` must never reach a row of the block.
fn gd_declared_decay(g: &Gd) -> String {
    let n = g.rows * g.nv;
    let mut decay = format!("Let A be a {} by {} tensor of Real.\n", g.rows, g.nv);
    for i in 0..n {
        decay += &format!(
            "Set item {} of A to {:?}.\n",
            i + 1,
            0.5 + 0.4 * val(i * 31 + 12)
        );
    }
    gd_dense(g).replace("Let A be sigmoid of A0.\n", &decay)
}

#[test]
fn a_chunk_past_the_blocks_last_row_never_reaches_it() {
    // 40 rows: a full chunk and one of 8, whose 24 padding rows hold a zero decay.
    let g = Gd {
        rows: 40,
        nk: 1,
        nv: 2,
        d: 32,
    };
    let src = gd_declared_decay(&g);
    assert!(
        src.contains("Set item 80 of A") && !src.contains("sigmoid of A0"),
        "the decay is declared, row for row"
    );
    let spans = [g.rows * g.nv * g.d, g.nv * g.d * g.d];
    chip_verdict_within(
        &src,
        &format!("chunked gated delta, declared decay {g:?}"),
        Chip::Normwise {
            spans: &spans,
            depth: g.rows,
        },
    );
}

// ───────────────────────────── 6. the memory moves once a step ─────────────────────────────

/// Bytes of `tensor` the schedule reads (`writes = false`) or writes, every loop's count multiplied in.
fn moved(p: &SpatialProgram, tensor: &str, writes: bool) -> u64 {
    use logicaffeine_compile::spatial::ir::{ChoreoOp, TaggedOp};
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
            let hit = match &t.op {
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
            if let Some((id, tiles)) = hit.filter(|(id, _)| p.tensor(*id).name == tensor) {
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

/// The block's operands declared and never set — what moves is the shape's, not the data's.
fn gd_declared(g: &Gd) -> String {
    let (qk, vw) = (g.nk * g.d, g.nv * g.d);
    let mut s = String::from("## Main\n");
    for (name, rows, cols) in [
        ("Q", g.rows, qk),
        ("K", g.rows, qk),
        ("V", g.rows, vw),
        ("Z", g.rows, vw),
        ("B", g.rows, g.nv),
        ("A", g.rows, g.nv),
        ("S", vw, g.d),
        ("GN", 1, g.d),
    ] {
        s += &format!("Let {name} be a {rows} by {cols} tensor of Real.\n");
    }
    s += &format!("Let O be gated delta of Q over K and V with gate Z beta B decay A state S norm GN with {} heads {} groups.\n", g.nv, g.nk);
    s
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
fn a_real_width_step_fits_a_site_with_fp32_beside_its_bf16_products() {
    // Qwen3.8's 48 value heads over 16 key heads of 128, a decode step, its memory fp32: a head's four a-blocks, their
    // operands and its output at fp32 leave no room for a second page of the buffers an item writes once, so those take
    // one. The step lowers whole, certifies, and still reads and writes the memory once.
    let g = Gd {
        rows: 1,
        nk: 16,
        nv: 48,
        d: 128,
    };
    let (p, _) = composed(&gd_declared(&g), &fp32_pointwise(), &format!("{g:?} with fp32 pointwise"));
    let s = p.tensors.iter().find(|t| t.name == "S").expect("the memory");
    assert_eq!(s.dtype, Precision::Fp32, "the memory is fp32");
    let once = (g.nv * g.d * g.d) as u64 * logicaffeine_compile::spatial::target::precision_bytes(s.dtype) as u64;
    assert_eq!(moved(&p, "S", false), once, "the step reads the memory once");
    assert_eq!(moved(&p, "S", true), once, "the step writes the memory once");
}

#[test]
fn each_step_reads_and_writes_the_memory_once() {
    for g in [
        Gd {
            rows: 1,
            nk: 16,
            nv: 48,
            d: 128,
        },
        Gd {
            rows: 1,
            nk: 2,
            nv: 4,
            d: 64,
        },
        Gd {
            rows: 2,
            nk: 2,
            nv: 4,
            d: 64,
        },
    ] {
        let (p, _) = composed(&gd_declared(&g), &sn50(), &format!("{g:?}"));
        let s = p
            .tensors
            .iter()
            .find(|t| t.name == "S")
            .expect("the memory");
        let once = (g.nv * g.d * g.d) as u64
            * logicaffeine_compile::spatial::target::precision_bytes(s.dtype) as u64;
        assert_eq!(
            moved(&p, "S", false),
            g.rows as u64 * once,
            "{g:?}: each row's step reads the memory once"
        );
        assert_eq!(
            moved(&p, "S", true),
            g.rows as u64 * once,
            "{g:?}: each row's step writes the memory once"
        );
    }
}
