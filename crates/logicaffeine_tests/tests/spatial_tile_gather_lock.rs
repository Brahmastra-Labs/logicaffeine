//! The TILE-level gather and scatter — an `IndirectTile` address on a Load or a Store — on the SN50's program
//! path: a data-indexed row moves as its own tiles and nothing else, so a table far wider than a site (an
//! embedding table, a KV cache) is never brought on chip whole.
//!
//! `IndirectTile { index, tile_stride, elems_per_tile, bound }` adds `(v div elems_per_tile)·tile_stride` tiles
//! to the affine address, `v` the integer at the front of the `index` buffer (an operand of the stage). The
//! promises:
//!
//!   1. ONLY THE ROW MOVES. `row i of T` with `T` wider than a site lowers to indirect Loads of exactly the
//!      row's tiles (one per work item — a row wider than the accumulator is spread over several); `Set row i
//!      of T to S` to indirect Stores of exactly the row's tiles, the rest of the carried table untouched.
//!   2. AN INDEX OUTSIDE ITS BOUND FAULTS (`ChoreoFault::TileGatherOutOfBound`) — never a wrapped row, never a
//!      silent no-op write.
//!   3. THE CERTIFICATE CHECKS THE BOUND. The token-only dry run cannot read the index, so it walks the tile
//!      block at BOTH ends of the declared bound: a bound that reaches past the table is refused before any
//!      data runs.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_tile_gather_lock`
#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography, ChoreoFault, Outputs};
use logicaffeine_compile::spatial::ir::{ChoreoOp, Phase, SpatialProgram, StageKind, TaggedOp};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;
use tensor_probes::{inputs_from_fills, oracle_ints, shown_from_outputs};

const ROWS: usize = 64;
const COLS: usize = 256;

fn composed(src: &str) -> SpatialProgram {
    let pl = compile_program_to_spatial(src, &SpatialTarget::sn50_rdu())
        .unwrap_or_else(|e| panic!("walk refused: {e:?}"));
    let sp = pl
        .whole()
        .unwrap_or_else(|| panic!("not lowered whole: {}", pl.refusal()))
        .clone();
    assert_eq!(
        sp.phase(),
        Phase::Verified,
        "the composed program certifies"
    );
    sp
}

fn eval(sp: &SpatialProgram) -> Result<Outputs, ChoreoFault> {
    eval_choreography(sp, &inputs_from_fills(sp).expect("deterministic fills"))
}

/// The value placed at flat position `p` (0-based) of the table — distinct per row and column, exact in Int.
fn marker(p: usize) -> i64 {
    (p as i64) * 7 + 3
}

/// Positions of the table that carry a marker: the first, a middle and the last element of every row the
/// tests touch, so a wrong row or a shifted column shows.
fn marked_positions() -> Vec<usize> {
    let mut v = Vec::new();
    for r in [0, 16, ROWS - 1] {
        for c in [0, COLS / 2 + 5, COLS - 1] {
            v.push(r * COLS + c);
        }
    }
    v
}

fn declare_table(name: &str) -> String {
    let mut s = format!("Let {name} be a {ROWS} by {COLS} tensor of Int.\n");
    for p in marked_positions() {
        s += &format!("Set item {} of {name} to {}.\n", p + 1, marker(p));
    }
    s
}

fn gather_src(tok: i64) -> String {
    let mut s = String::from("## Main\n");
    s += &declare_table("T");
    s += &format!("Let tok be {tok}.\nLet x be row tok of T.\n");
    for c in [1, COLS / 2 + 6, COLS] {
        s += &format!("Show item {c} of x.\n");
    }
    s
}

fn scatter_src(p: i64) -> String {
    let mut s = String::from("## Main\n");
    s += &declare_table("Kc");
    s += &format!("Let src be a 1 by {COLS} tensor of Int.\n");
    for (i, v) in [(1, 11), (COLS / 2 + 6, 22), (COLS, 33)] {
        s += &format!("Set item {i} of src to {v}.\n");
    }
    s += &format!("Let p be {p}.\nSet row p of Kc to src.\n");
    for r in [1, 17, ROWS] {
        for c in [1, COLS / 2 + 6, COLS] {
            s += &format!("Show item {} of Kc.\n", (r - 1) * COLS + c);
        }
    }
    s
}

/// Every Load/Store stage of a program that carries an indirect tile address, with the tiles it moves.
fn indirect_movements(sp: &SpatialProgram) -> Vec<(&'static str, usize)> {
    sp.stages
        .iter()
        .filter_map(|s| match &s.kind {
            StageKind::Load { pattern, .. } if pattern.origin.indirect.is_some() => {
                Some(("load", pattern.block_h * pattern.block_w))
            }
            StageKind::Store { pattern, .. } if pattern.origin.indirect.is_some() => {
                Some(("store", pattern.block_h * pattern.block_w))
            }
            _ => None,
        })
        .collect()
}

/// The tiles of `tensor` the indirect Loads move over the WHOLE work: each placed Load stage runs once per work
/// item of its site, and moves its block each time.
fn indirect_tiles_moved(sp: &SpatialProgram, tensor: &str) -> usize {
    sp.stages
        .iter()
        .filter_map(|s| match &s.kind {
            StageKind::Load {
                tensor: t, pattern, ..
            } if sp.tensor(*t).name == tensor && pattern.origin.indirect.is_some() => {
                let items = match (s.item, s.site) {
                    (Some(_), _) => 1,
                    (None, Some(site)) => sp.items_at_in(sp.section_of_stage(s.id), site).len(),
                    (None, None) => 1,
                };
                Some(pattern.block_h * pattern.block_w * items)
            }
            _ => None,
        })
        .sum()
}

fn tile_cols() -> usize {
    SpatialTarget::sn50_rdu().tile.cols
}

// ── 1. only the row moves ────────────────────────────────────────────────────────────────────────────

#[test]
fn a_row_of_a_table_wider_than_a_site_gathers_only_that_rows_tiles() {
    for tok in [1i64, 17, ROWS as i64] {
        let src = gather_src(tok);
        let sp = composed(&src);
        assert_eq!(
            indirect_movements(&sp),
            vec![("load", COLS / tile_cols())],
            "row {tok}: one indirect load of the row's tiles"
        );
        let out = eval(&sp).unwrap_or_else(|e| panic!("row {tok}: eval: {e:?}"));
        let vm = vm_outcome(&src);
        assert!(vm.error.is_none(), "row {tok}: VM {:?}", vm.error);
        assert_eq!(
            shown_from_outputs(&src, &sp, &out).expect("shown"),
            oracle_ints(&vm.output).expect("vm"),
            "row {tok}"
        );
    }
}

#[test]
fn a_row_written_into_a_table_wider_than_a_site_moves_only_that_rows_tiles() {
    for p in [1i64, 17, ROWS as i64] {
        let src = scatter_src(p);
        let sp = composed(&src);
        assert_eq!(
            indirect_movements(&sp),
            vec![("store", COLS / tile_cols())],
            "row {p}: one indirect store of the row's tiles"
        );
        let out = eval(&sp).unwrap_or_else(|e| panic!("row {p}: eval: {e:?}"));
        let vm = vm_outcome(&src);
        assert!(vm.error.is_none(), "row {p}: VM {:?}", vm.error);
        assert_eq!(
            shown_from_outputs(&src, &sp, &out).expect("shown"),
            oracle_ints(&vm.output).expect("vm"),
            "row {p}: the written row and every untouched marker"
        );
    }
}

// ── 2. an index outside its bound faults ─────────────────────────────────────────────────────────────

#[test]
fn a_row_outside_the_bound_faults_for_a_gather_and_a_scatter() {
    for tok in [0i64, ROWS as i64 + 1] {
        for (what, src) in [("gather", gather_src(tok)), ("scatter", scatter_src(tok))] {
            let sp = composed(&src);
            match eval(&sp) {
                Err(ChoreoFault::TileGatherOutOfBound { index, bound, .. }) => {
                    assert_eq!(
                        index,
                        (tok - 1) as f64,
                        "{what} row {tok}: the fault names the 0-based row"
                    );
                    assert_eq!(
                        bound,
                        (0, ROWS - 1),
                        "{what} row {tok}: and the table's row range"
                    );
                }
                other => {
                    panic!("{what} row {tok}: an out-of-bound tile index must fault, not {other:?}")
                }
            }
        }
    }
}

// ── 3. the certificate checks the bound ──────────────────────────────────────────────────────────────

#[test]
fn a_bound_that_reaches_past_the_table_is_refused_by_the_dry_run() {
    use logicaffeine_compile::spatial::eval::eval_choreography_dry;
    let sp = composed(&gather_src(1));
    assert!(
        eval_choreography_dry(&sp).is_ok(),
        "the lowered bound is exactly the table's rows"
    );
    // The bound is widened where the dry run EXECUTES it — the schedule's `ReadTiles` carries its own copy of the
    // address — and on the stage it was synthesized from, so the two still agree.
    fn widen(ops: &mut [TaggedOp]) -> usize {
        let mut n = 0;
        for t in ops {
            match &mut t.op {
                ChoreoOp::ReadTiles { at, .. } => {
                    if let Some(ind) = &mut at.indirect {
                        ind.bound.1 += 1;
                        n += 1;
                    }
                }
                ChoreoOp::Loop { body, .. } => n += widen(body),
                _ => {}
            }
        }
        n
    }
    let mut widened = sp.clone();
    for s in &mut widened.stages {
        if let StageKind::Load { pattern, .. } = &mut s.kind {
            if let Some(ind) = &mut pattern.origin.indirect {
                ind.bound.1 += 1;
            }
        }
    }
    let reads = widened
        .schedule
        .as_mut()
        .expect("a scheduled program")
        .programs
        .iter_mut()
        .map(|p| widen(&mut p.ops))
        .sum::<usize>();
    assert!(
        reads >= 1,
        "the schedule carries the gather's indirect read"
    );
    match eval_choreography_dry(&widened) {
        Err(ChoreoFault::ReadOutOfRange { .. }) => {}
        other => {
            panic!("a bound one row past the table must be refused by the dry run, not {other:?}")
        }
    }
}

#[test]
fn an_embedding_table_at_qwen_width_lowers_and_certifies_moving_one_row() {
    // Qwen3.8's hidden width, over a vocabulary slice: 1024 × 5120 is 163,840 tiles — hundreds of sites'
    // worth — and the gather moves 160 of them.
    let (rows, d) = (1024usize, 5120usize);
    let src = format!(
        "## Main\nLet Emb be a {rows} by {d} tensor of Real.\nLet tok be 7.\nLet x be row tok of Emb.\nShow round(item 1 of x * 1000000).\n"
    );
    let sp = composed(&src);
    // A 160-tile row cannot pass through one site's eight-tile accumulator, so the row is spread over work items;
    // what is pinned is WHAT MOVES: the row's tiles exactly, every one of them addressed through the index.
    assert_eq!(
        indirect_tiles_moved(&sp, "Emb"),
        d / tile_cols(),
        "exactly one embedding row's tiles move"
    );
    let direct = sp.stages.iter().filter(|s| matches!(&s.kind, StageKind::Load { tensor, pattern, .. } if sp.tensor(*tensor).name == "Emb" && pattern.origin.indirect.is_none())).count();
    assert_eq!(direct, 0, "no load of the table is anything but the gather");
}
