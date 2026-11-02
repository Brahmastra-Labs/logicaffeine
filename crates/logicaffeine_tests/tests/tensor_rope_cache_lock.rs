//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! ROPE FROM A POSITION — a rotary embedding that reads its angles out of a position-indexed cache.
//!
//! `Let Y be rope of Q with cos C and sin S from position P [across H heads]` turns row `r` of `Q` (1-based) by row
//! `P + r − 1` of the cos/sin caches `C` and `S` — a table of angles per POSITION, `ctx` rows of `half`. A decode step
//! turns its one row by row `pos`; a prefill chunk turns its `R` rows by rows `pos … pos + R − 1`. The caches are model
//! constants, so the host hands in a position and nothing else.
//!
//! Pinned, on the VM and the tree-walker (which must agree):
//!   1. Row `r` of the block turns exactly as row `r` alone turns by the tables `row P + r − 1 of C` / `of S` — for a
//!      full and a partial rotary, one row and a block, a literal and a computed position, at every position the cache
//!      admits; and the position moves the answer.
//!   2. A block leaving the cache (`P < 1`, or `P + R − 1` past its last row) fails at run time with one message on
//!      every tier; the position expression runs once; a cache of fewer rows than the block, and cos/sin caches of
//!      different shapes, are refused by name.
//!   3. THE SN50 COMPUTES IT IN THE ROPE'S OWN SECTION. The flat form (heads within the site's row budget) — cache rows
//!      narrower than a tile and whole tiles — and the blocked form (a head per item), one row and a block, lower whole
//!      to ONE section — no gather of the cache rows before it — and evaluate to the VM's values: the fp32 twin exactly
//!      (every input a multiple of 1/64), the bf16 chip within two bf16 ulps. A real model's cache (4,096 positions)
//!      lowers without coming on chip: a section's pages hold the block's angles, not the cache.
//!   4. THE POSITION IS DATA: one compiled program, evaluated at other positions, gives the VM's values at those
//!      positions; a position whose block leaves the cache faults at the guard, against the positions the block admits.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test tensor_rope_cache_lock`
//!
//! ★ MUTATION PROBES (private copy, every one RED, 2026-09-26):
//!   • the desugar reads the cache from row 1 whatever the position (`(at − 1)` → `at·0`) → the VM/tree-walker tests,
//!     and the recognizer no longer lifts the statement.
//!   • the sub-tile guard admits the whole cache (`(cache − rows + 1)·half` → `cache·half`), and the whole-tile guards,
//!     flat and blocked, admit every row (`cache − rows + 1` → `cache`) → each `… faults at its guard` extent.
//!   • a literal position is dropped from the whole-tile load's base → `FLAT_WHOLE from the literal 3: WIRING`.
//!   • the cache rows' tile stride doubles → the certificate's dry run reads past the cache (`ReadOutOfRange`); a zero
//!     stride is malformed (`IndirectMalformed`).
//! ════════════════════════════════════════════════════════════════════════════════════════════════
#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::spatial::eval::{eval_choreography, ChoreoFault};
use logicaffeine_compile::spatial::ir::{Phase, SpatialProgram, TensorInit};
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

fn ran(src: &str, what: &str) -> Vec<String> {
    let (out, err) = tiers(src);
    assert_eq!(err, None, "{what}: {err:?}\n{src}");
    out
}

fn show_all(name: &str, n: usize) -> String {
    (1..=n)
        .map(|i| format!("Show item {i} of {name}.\n"))
        .collect()
}

/// A rope shape: `rows` query rows of `heads` heads of `hd`, rotating the leading `2·half` of each head.
#[derive(Clone, Copy, Debug)]
struct Rope {
    rows: usize,
    heads: usize,
    hd: usize,
    half: usize,
}

const FULL: Rope = Rope {
    rows: 3,
    heads: 2,
    hd: 4,
    half: 2,
};
const PARTIAL: Rope = Rope {
    rows: 4,
    heads: 3,
    hd: 6,
    half: 1,
};
/// Heads streamed one per item: a row's heads are more tiles than a site's row budget, a head is not.
const BLOCKED: Rope = Rope {
    rows: 2,
    heads: 3,
    hd: 64,
    half: 32,
};
const BLOCKED_PARTIAL: Rope = Rope {
    rows: 3,
    heads: 2,
    hd: 128,
    half: 32,
};
/// The flat form with cache rows of whole tiles: the block's rows are fetched alone.
const FLAT_WHOLE: Rope = Rope {
    rows: 2,
    heads: 1,
    hd: 64,
    half: 32,
};
/// A tensor-parallel chip's key rope: one head of Qwen3.8's width, one row.
const KEY_HEAD: Rope = Rope {
    rows: 1,
    heads: 1,
    hd: 256,
    half: 32,
};

/// The caches' positions.
const CACHE: usize = 7;

fn one_row(r: Rope) -> Rope {
    Rope { rows: 1, ..r }
}

fn across(r: &Rope) -> String {
    if 2 * r.half == r.hd {
        String::new()
    } else {
        format!(" across {} heads", r.heads)
    }
}

fn caches(r: &Rope, cache: usize) -> String {
    declare("C", cache, r.half, 2) + &declare("S", cache, r.half, 3)
}

/// `Y` = rope of the whole `Q` from position `from` (`bind` declares `Let P be …` first).
fn rope_at(r: &Rope, from: &str, bind: Option<usize>) -> String {
    let w = r.heads * r.hd;
    let mut s = String::from("## Main\n");
    s += &declare("Q", r.rows, w, 1);
    s += &caches(r, CACHE);
    if let Some(p) = bind {
        s += &format!("Let P be {p}.\n");
    }
    s += &format!(
        "Let Y be rope of Q with cos C and sin S from position {from}{}.\n",
        across(r)
    );
    s += &show_all("Y", r.rows * w);
    s
}

/// The block as its rows: row `k` of `Q` turned alone by row `p + k − 1` of the caches.
fn rope_at_rows(r: &Rope, p: usize) -> String {
    let w = r.heads * r.hd;
    let mut s = String::from("## Main\n");
    s += &declare("Q", r.rows, w, 1);
    s += &caches(r, CACHE);
    for k in 1..=r.rows {
        let at = p + k - 1;
        s += &format!(
            "Let q{k} be row {k} of Q.\nLet c{k} be row {at} of C.\nLet s{k} be row {at} of S.\n"
        );
        s += &format!(
            "Let y{k} be rope of q{k} with cos c{k} and sin s{k}{}.\n",
            across(r)
        );
    }
    for k in 1..=r.rows {
        s += &show_all(&format!("y{k}"), w);
    }
    s
}

// ───────────────────────────── 1. the rows turn by their positions' angles ─────────────────────────────

#[test]
fn row_r_turns_by_cache_row_p_plus_r_minus_one() {
    for r in [FULL, PARTIAL, one_row(FULL), one_row(PARTIAL)] {
        for p in 1..=CACHE + 1 - r.rows {
            let want = ran(&rope_at_rows(&r, p), &format!("{r:?} rows from {p}"));
            assert_eq!(
                want.len(),
                r.rows * r.heads * r.hd,
                "{r:?}: every element of every row is shown"
            );
            assert_eq!(
                ran(
                    &rope_at(&r, &p.to_string(), None),
                    &format!("{r:?} literal from {p}")
                ),
                want,
                "{r:?} from {p}: row k turns by cache row {p} + k − 1"
            );
            assert_eq!(
                ran(&rope_at(&r, "P", Some(p)), &format!("{r:?} named from {p}")),
                want,
                "{r:?} from a computed {p}"
            );
        }
        let (first, next) = (
            ran(&rope_at(&r, "1", None), "from 1"),
            ran(&rope_at(&r, "2", None), "from 2"),
        );
        assert_ne!(first, next, "{r:?}: the position moves the angles");
    }
}

// ───────────────────────────── 2. what is refused ─────────────────────────────

#[test]
fn a_block_leaving_the_cache_fails_with_one_message_on_every_tier() {
    for r in [FULL, one_row(FULL)] {
        for p in [0usize, CACHE + 2 - r.rows] {
            for (src, what) in [
                (rope_at(&r, &p.to_string(), None), "literal"),
                (rope_at(&r, "P", Some(p)), "named"),
            ] {
                let err = tiers(&src).1.unwrap_or_else(|| {
                    panic!("{r:?}: a block from {p} ({what}) leaves the cache and must fail")
                });
                assert!(
                    err.contains("out of range") && err.contains(&format!("C has {CACHE} rows")),
                    "{r:?} from {p} ({what}): {err}"
                );
            }
        }
    }
}

#[test]
fn the_position_expression_runs_once() {
    let mut s = String::from("## To count (c: Int) -> Int:\n    Show c.\n    Return c.\n\n");
    s += &rope_at(&FULL, "(count(2))", None);
    let out = ran(&s, "counted");
    assert_eq!(
        out.iter().filter(|l| l.as_str() == "2").count(),
        1,
        "count(2) runs exactly once: {out:?}"
    );
}

#[test]
fn a_cache_that_cannot_hold_the_block_or_caches_of_two_shapes_are_refused_by_name() {
    let r = FULL;
    let w = r.heads * r.hd;
    let mut short = String::from("## Main\n");
    short += &declare("Q", r.rows, w, 1);
    short += &caches(&r, r.rows - 1);
    short += "Let Y be rope of Q with cos C and sin S from position 1.\nShow item 1 of Y.\n";
    let err = tiers(&short).1.unwrap_or_else(|| {
        panic!(
            "a {}-row cache for a {}-row block must be refused",
            r.rows - 1,
            r.rows
        )
    });
    assert!(
        err.contains("rope") && err.contains('C') && err.contains("rows"),
        "the refusal names the rope, the cache and its rows: {err}"
    );

    let mut two = String::from("## Main\n");
    two += &declare("Q", r.rows, w, 1);
    two += &declare("C", CACHE, r.half, 2);
    two += &declare("S", CACHE - 1, r.half, 3);
    two += "Let Y be rope of Q with cos C and sin S from position 1.\nShow item 1 of Y.\n";
    let err = tiers(&two)
        .1
        .unwrap_or_else(|| panic!("cos and sin caches of different shapes must be refused"));
    assert!(
        err.contains("rope") && err.contains('S'),
        "the refusal names the mismatched cache: {err}"
    );
}

// ───────────────────────────── 3. the SN50 computes it in the rope's own section ─────────────────────────────

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

/// The program lowered whole, certified, its certificate rechecked; and its section count.
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

/// The program's shown values, evaluated with `P` set to `p` when given.
fn shown(src: &str, sp: &SpatialProgram, p: Option<usize>, what: &str) -> Vec<i64> {
    let mut sp = sp.clone();
    if let Some(p) = p {
        let t = sp
            .tensors
            .iter_mut()
            .find(|t| t.name == "P")
            .unwrap_or_else(|| panic!("{what}: the position is a tensor of the program"));
        t.init = TensorInit::Values(vec![p as i64]);
    }
    let out = eval_choreography(&sp, &inputs_from_fills(&sp).expect("fills"))
        .unwrap_or_else(|e| panic!("{what}: eval: {e:?}"));
    shown_from_outputs(src, &sp, &out).expect("shown")
}

fn within_two_bf16_ulps(got: &[i64], want: &[i64], what: &str) {
    for (i, (g, v)) in got.iter().zip(want).enumerate() {
        let x = (*v as f64 / 1e6).abs();
        let ulp = if x == 0.0 {
            0.0
        } else {
            2f64.powi(x.log2().floor() as i32 - 7)
        };
        assert!(
            ((g - v).abs() as f64) <= (2.0 * ulp * 1e6).max(1.0),
            "{what}: CHIP Show #{}: bf16 {g} vs VM {v}",
            i + 1
        );
    }
}

#[test]
fn the_rope_from_a_position_lowers_on_sn50_to_one_section_and_computes_the_vms_values() {
    let named = [
        FULL,
        PARTIAL,
        FLAT_WHOLE,
        KEY_HEAD,
        BLOCKED,
        BLOCKED_PARTIAL,
        one_row(FULL),
        one_row(BLOCKED),
    ]
    .map(|r| (r, true));
    let literal = [FULL, FLAT_WHOLE, BLOCKED].map(|r| (r, false));
    for (r, is_named) in named.into_iter().chain(literal) {
        let what = format!(
            "{r:?} from {} 3",
            if is_named { "P =" } else { "the literal" }
        );
        let src = rounded(&if is_named {
            rope_at(&r, "P", Some(3))
        } else {
            rope_at(&r, "3", None)
        });
        let want = oracle_ints(&ran(&src, &what).join("\n")).expect("ints");
        let (twin, sections) = composed(&src, &fp32_twin(), &what);
        assert_eq!(
            sections, 1,
            "{what}: the rope reads its rows of the cache in its own section — no gather before it"
        );
        assert_eq!(
            shown(&src, &twin, None, &what),
            want,
            "{what}: WIRING — the fp32 twin computes the VM's rotation exactly"
        );
        let (chip, _) = composed(&src, &sn50(), &what);
        within_two_bf16_ulps(&shown(&src, &chip, None, &what), &want, &what);
    }
}

// ───────────────────────────── 4. the position is data ─────────────────────────────

#[test]
fn one_compiled_rope_turns_by_whatever_position_it_is_handed() {
    for r in [
        FULL,
        PARTIAL,
        FLAT_WHOLE,
        KEY_HEAD,
        BLOCKED,
        one_row(BLOCKED),
    ] {
        let src = rounded(&rope_at(&r, "P", Some(2)));
        let (twin, _) = composed(&src, &fp32_twin(), &format!("{r:?}"));
        for p in [1, CACHE + 1 - r.rows] {
            let what = format!("{r:?} compiled at 2, run from {p}");
            let want = oracle_ints(&ran(&rounded(&rope_at(&r, "P", Some(p))), &what).join("\n"))
                .expect("ints");
            assert_eq!(
                shown(&src, &twin, Some(p), &what),
                want,
                "{what}: the position is read at run time, not folded"
            );
        }
    }
}

#[test]
fn a_computed_position_whose_block_leaves_the_cache_faults_at_its_guard() {
    // Cache rows narrower than a tile are guarded by the block's first ELEMENT offset `(P − 1)·half`; rows of whole tiles,
    // flat or blocked, by its first ROW `P − 1`.
    for (r, extent) in [
        (FULL, (CACHE + 1 - FULL.rows) * FULL.half),
        (FLAT_WHOLE, CACHE + 1 - FLAT_WHOLE.rows),
        (BLOCKED, CACHE + 1 - BLOCKED.rows),
    ] {
        let src = rounded(&rope_at(&r, "P", Some(1)));
        let (sp, _) = composed(&src, &sn50(), &format!("{r:?}"));
        for p in [0usize, CACHE + 2 - r.rows] {
            let mut bad = sp.clone();
            bad.tensors
                .iter_mut()
                .filter(|t| t.name == "P")
                .for_each(|t| t.init = TensorInit::Values(vec![p as i64]));
            match eval_choreography(&bad, &inputs_from_fills(&bad).expect("fills")) {
                Err(ChoreoFault::GatherOutOfRange { extent: e, .. }) => assert_eq!(e, extent, "{r:?} from {p}: checked against the positions the block admits"),
                other => panic!("{r:?}: a block from {p} leaves the cache and must fault at its guard, not {other:?}"),
            }
        }
    }
}

#[test]
fn a_real_models_cache_lowers_without_coming_on_chip() {
    use logicaffeine_compile::spatial::ir::BufferKind;
    // Qwen3.8's cache: 4,096 positions of 32 angles. Declared, not filled — the lowering is the question here.
    for r in [KEY_HEAD, one_row(BLOCKED), FLAT_WHOLE] {
        let w = r.heads * r.hd;
        let mut src = String::from("## Main\n");
        src += &declare("Q", r.rows, w, 1);
        src += &format!("Let C be a 4096 by {h} tensor of Real.\nLet S be a 4096 by {h} tensor of Real.\nLet P be 4000.\n", h = r.half);
        src += &format!(
            "Let Y be rope of Q with cos C and sin S from position P{}.\n",
            across(&r)
        );
        src += &show_all("Y", r.rows * w);
        let (sp, sections) = composed(&src, &sn50(), &format!("{r:?} over 4096 positions"));
        assert_eq!(sections, 1, "{r:?}: one section");
        let block_tiles = r.rows * r.half / sp.tile.cols;
        for b in &sp.buffers {
            let tiles = match b.kind {
                BufferKind::Fifo { page_tiles, .. } | BufferKind::Interm { page_tiles, .. } => {
                    page_tiles
                }
                _ => 0,
            };
            if b.name.contains("rc")
                || b.name.contains("rs")
                || b.name.contains("in_C")
                || b.name.contains("in_S")
            {
                assert!(tiles <= block_tiles.max(1), "{r:?}: buffer `{}` holds {tiles} tiles of the cache — only the block's {block_tiles} come on chip", b.name);
            }
        }
    }
}
