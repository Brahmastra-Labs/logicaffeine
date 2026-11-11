//! spatial_rope_window_lock — a rotation that turns a head per item by a table row that is not whole tiles reads the
//! row through a window, on the SN50 as on the VM.
//!
//! `rope of Q with cos C and sin S from position P across H heads` turns every head of `Q` by row `P` of the tables. A
//! row wider than an accumulator page streams a head per item, and each item reads its row of the tables. When the
//! row's `half` is whole tiles the row is a tile run; otherwise row `ρ` starts at element `ρ·half`, mid-tile, so the
//! item computes that element, loads the whole tiles around it — a window clamped to end inside the table — and reads
//! the row at its offset in the page. Partial rotations (Qwen3.5's 64 of 256, Phi-2's 32 of 80, the exam's 24 of 256)
//! have such rows.
//!
//! Each case: on an fp32 twin of the chip the result is the VM's within a relative 2⁻¹⁶, on the SN50 within the bf16
//! datapath's 2% of the largest value, over a named position (checked as a row index is) and a literal one, at the
//! table's first and last rows.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test spatial_rope_window_lock`

#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::eval_choreography;
use logicaffeine_compile::spatial::ir::{Phase, SpatialProgram};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use tensor_probes::{inputs_from_fills, oracle_ints, shown_from_outputs};

fn seeded(name: &str, i: usize) -> f64 {
    let salt = name.bytes().fold(7u64, |h, b| h.wrapping_mul(131).wrapping_add(b as u64));
    let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29);
    ((h >> 40) % 129) as f64 / 64.0 - 1.0
}

/// `Y` the rotation of `heads` heads of `hd` by `half`-wide rows of `ctx`-row tables, from `position` (a name bound to
/// `p`, or the literal itself).
fn program(heads: usize, hd: usize, half: usize, ctx: usize, p: usize, named: bool) -> String {
    block_program(1, heads, hd, half, ctx, p, named)
}

/// [`program`] over a block of `rows` rows of `heads` heads each, row `r` turned by table row `p + r`.
fn block_program(rows: usize, heads: usize, hd: usize, half: usize, ctx: usize, p: usize, named: bool) -> String {
    let mut s = String::from("## Main\n");
    for (name, r, c) in [("Q", rows, heads * hd), ("C", ctx, half), ("S", ctx, half)] {
        s.push_str(&format!("Let {name} be a {r} by {c} tensor of Real.\n"));
        for i in 0..r * c {
            s.push_str(&format!("Set item {} of {name} to {:?}.\n", i + 1, seeded(name, i)));
        }
    }
    let from = if named {
        s.push_str(&format!("Let P be {p}.\n"));
        "P".to_string()
    } else {
        p.to_string()
    };
    s.push_str(&format!("Let Y be rope of Q with cos C and sin S from position {from} across {heads} heads.\n"));
    for i in 1..=rows * heads * hd {
        s.push_str(&format!("Show round(item {i} of Y * 1000000).\n"));
    }
    s
}

fn fp32_twin() -> SpatialTarget {
    let mut t = SpatialTarget::sn50_rdu();
    for c in &mut t.compute {
        if let ComputeCap::MatmulTile { precisions, .. } | ComputeCap::Elementwise { precisions } | ComputeCap::RowReduce { precisions } = c {
            if precisions.contains(&Precision::Bf16) {
                precisions.insert(Precision::Fp32);
            }
        }
    }
    t.precisions.insert(Precision::Fp32);
    t
}

fn composed(src: &str, target: &SpatialTarget, what: &str) -> SpatialProgram {
    let pl = compile_program_to_spatial(src, target).unwrap_or_else(|e| panic!("{what}: the walk: {e:?}"));
    let sp = pl.whole().unwrap_or_else(|| panic!("{what}: not lowered whole: {}", pl.refusal())).clone();
    assert_eq!(sp.phase(), Phase::Verified, "{what}: the program certifies");
    sp
}

/// The case against the VM; `form` is a suffix of a tensor the SN50's program must hold (`__rope_rows`: rows read
/// through a window; `__columns`: the flat rotation blocked by its result's columns).
fn verdict_in(src: &str, what: &str, form: &str) {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "{what}: the VM: {:?}", vm.error);
    let want = oracle_ints(&vm.output).expect("the VM shows integers");
    for (target, chip) in [(fp32_twin(), false), (SpatialTarget::sn50_rdu(), true)] {
        let sp = composed(src, &target, what);
        assert!(sp.tensors.iter().any(|t| t.name.ends_with(form)), "{what}: the rotation on {} holds a `…{form}` table", target.name);
        let out = eval_choreography(&sp, &inputs_from_fills(&sp).expect("fills")).unwrap_or_else(|e| panic!("{what}: {e:?}"));
        let got = shown_from_outputs(src, &sp, &out).expect("shown");
        assert_eq!(got.len(), want.len(), "{what}: one Show a value");
        let scale = want.iter().fold(1i64, |m, v| m.max(v.abs())) as f64;
        for (i, (g, v)) in got.iter().zip(&want).enumerate() {
            let bound = if chip { 0.02 * scale } else { (v.abs() as f64 / 65536.0).max(1.0) };
            assert!(((g - v).abs() as f64) <= bound, "{what}: Show #{} on {}: {g} vs the VM's {v}", i + 1, target.name);
        }
    }
}

fn verdict(src: &str, what: &str) {
    verdict_in(src, what, "__rope_rows");
}

#[test]
fn a_partial_rotation_from_a_named_position_reads_its_row_through_a_window() {
    // Three heads of 96 (nine tiles, past an accumulator page), 24 of each turned: rows of 12, mid-tile from row 3 on.
    for p in [1, 7, 10] {
        verdict(&program(3, 96, 12, 10, p, true), &format!("half 12, row {p} of 10, named"));
    }
}

#[test]
fn a_partial_rotation_from_a_literal_position_reads_its_row_through_a_window() {
    for p in [4, 10] {
        verdict(&program(3, 96, 12, 10, p, false), &format!("half 12, row {p} of 10, literal"));
    }
}

#[test]
fn a_row_wider_than_a_tile_and_not_whole_tiles_reads_through_a_window() {
    // Rows of 40 under heads of 96: a window of three tiles.
    for p in [1, 6] {
        verdict(&program(3, 96, 40, 6, p, true), &format!("half 40, row {p} of 6, named"));
    }
}

#[test]
fn heads_that_are_not_whole_tiles_turn_as_one_flat_stage_blocked_by_columns() {
    // Phi-2's heads of 80 share tiles with their neighbours: four of them (320 wide, ten tiles) turn as the one flat
    // stage, an item per block of the result, by rows of 16 (32 of 80 turned) and of 40 (turned whole).
    for (half, p) in [(16, 1), (16, 6), (40, 3)] {
        verdict_in(&program(4, 80, half, 6, p, true), &format!("heads of 80, half {half}, row {p} of 6"), "__columns");
    }
}

#[test]
fn a_block_whose_whole_row_fits_no_site_turns_a_head_an_item() {
    // On the RTX 3060 target a tile is 16 wide, so heads of 16 are whole tiles, and a flat page holds one row of each
    // 16 × 16 tile: 7 rows of 4 heads (448 values, 28 tiles) kept whole beside their shifted reads and their result fit
    // no site, though they are well inside the accumulator's page. The rotation turns a head an item instead.
    let target = SpatialTarget::rtx3060();
    for p in [1, 6] {
        let src = block_program(7, 4, 16, 8, 12, p, true);
        let what = format!("7 rows of 4 heads of 16, from row {p} of 12, on {}", target.name);
        let vm = vm_outcome(&src);
        assert!(vm.error.is_none(), "{what}: the VM: {:?}", vm.error);
        let want = oracle_ints(&vm.output).expect("the VM shows integers");
        let sp = composed(&src, &target, &what);
        assert!(!sp.tensors.iter().any(|t| t.name.ends_with("__columns")), "{what}: the rotation is a head an item, not the flat stage");
        let out = eval_choreography(&sp, &inputs_from_fills(&sp).expect("fills")).unwrap_or_else(|e| panic!("{what}: {e:?}"));
        let got = shown_from_outputs(&src, &sp, &out).expect("shown");
        assert_eq!(got.len(), want.len(), "{what}: one Show a value");
        let scale = want.iter().fold(1i64, |m, v| m.max(v.abs())) as f64;
        for (i, (g, v)) in got.iter().zip(&want).enumerate() {
            assert!(((g - v).abs() as f64) <= 0.02 * scale, "{what}: Show #{}: {g} vs the VM's {v}", i + 1);
        }
    }
}

#[test]
fn a_block_of_rows_whose_heads_are_not_whole_tiles_turns_each_row_by_its_own_table_row() {
    // A prefill block: 7 rows of 4 heads of 16 (448 wide, fourteen tiles), row `r` turned by table row `P + r` of 12.
    // Every head's band reads both tables at the one named position, moved by the item's first column once.
    for p in [1, 6] {
        verdict_in(&block_program(7, 4, 16, 8, 12, p, true), &format!("7 rows of 4 heads of 16, from row {p} of 12"), "__columns");
    }
}

/// The case against the VM where the rotation reads its tables through a window of a cache past a page: one load named
/// `…_window` a table (placement repeats a stage a site, as `…@x,y`) on the fp32 twin and on the SN50.
fn verdict_windowed(src: &str, what: &str) {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "{what}: the VM: {:?}", vm.error);
    let want = oracle_ints(&vm.output).expect("the VM shows integers");
    for (target, chip) in [(fp32_twin(), false), (SpatialTarget::sn50_rdu(), true)] {
        let sp = composed(src, &target, what);
        let windows: std::collections::BTreeSet<&str> =
            sp.stages.iter().filter_map(|s| s.name.split('@').next()).filter(|n| n.ends_with("_window")).collect();
        assert_eq!(windows.len(), 2, "{what}: on {} the cosine and the sine tables each load a window: {windows:?}", target.name);
        let out = eval_choreography(&sp, &inputs_from_fills(&sp).expect("fills")).unwrap_or_else(|e| panic!("{what}: {e:?}"));
        let got = shown_from_outputs(src, &sp, &out).expect("shown");
        assert_eq!(got.len(), want.len(), "{what}: one Show a value");
        let scale = want.iter().fold(1i64, |m, v| m.max(v.abs())) as f64;
        for (i, (g, v)) in got.iter().zip(&want).enumerate() {
            let bound = if chip { 0.02 * scale } else { (v.abs() as f64 / 65536.0).max(1.0) };
            assert!(((g - v).abs() as f64) <= bound, "{what}: Show #{} on {}: {g} vs the VM's {v}", i + 1, target.name);
        }
    }
}

#[test]
fn a_flat_rotation_whose_cache_is_past_a_page_reads_its_rows_through_a_window() {
    // Heads of 80 and of 112 share tiles with their neighbours, so a decode turns them as the one flat stage; turned
    // whole, their table rows (40 and 56) are never whole 32-wide tiles, and a cache of 8 positions (320 and 448 values,
    // ten and fourteen tiles) is past an accumulator page. The block's rows are read through the window of whole tiles
    // around them, clamped to end inside the table — named and literal, at the cache's first and last rows.
    let cases: [(usize, usize, usize, usize); 4] = [(2, 80, 40, 8), (2, 112, 56, 8), (4, 80, 40, 20), (2, 80, 16, 24)];
    for (heads, hd, half, ctx) in cases {
        let table_t = (ctx * half).div_ceil(32);
        assert!(hd % 32 != 0 && half % 32 != 0 && table_t > 8, "heads of {hd}, rows of {half}, {ctx} positions: past the page");
        for (p, named) in [(1, true), (ctx, true), (ctx / 2, false), (ctx, false)] {
            verdict_windowed(
                &program(heads, hd, half, ctx, p, named),
                &format!("{heads} heads of {hd}, rows of {half}, row {p} of {ctx}, {}", if named { "named" } else { "literal" }),
            );
        }
    }
    // A prefill block of 3 rows reads the window around all three.
    for p in [1, 6] {
        verdict_windowed(&block_program(3, 2, 80, 40, 8, p, true), &format!("3 rows of 2 heads of 80, from row {p} of 8"));
    }
}

#[test]
fn a_block_of_heads_that_are_not_whole_tiles_past_a_site_turns_each_head_through_its_window() {
    // A prefill block of heads of 80 and of 112 — 12 rows of 4 (3,840 values), 13 rows of 3 (3,120), 16 rows of 3 heads of
    // 112 (5,376) — kept whole as the one flat stage is more than an SN50 site holds beside its shifted reads. Each head is
    // an item: its queries read through the run of tiles around it from its offset, its turned values written into whole
    // tiles of its own and the heads packed side by side after — 39 heads of 80 leave one past the last pair.
    let cases: [(usize, usize, usize, usize, usize, usize, bool); 5] = [
        (12, 4, 80, 40, 16, 1, true),
        (12, 4, 80, 40, 16, 5, true),
        (13, 3, 80, 16, 16, 4, true),
        (16, 3, 112, 56, 20, 1, false),
        (16, 3, 112, 56, 20, 5, false),
    ];
    for (rows, heads, hd, half, ctx, p, named) in cases {
        verdict_in(
            &block_program(rows, heads, hd, half, ctx, p, named),
            &format!("{rows} rows of {heads} heads of {hd}, rows of {half}, from row {p} of {ctx}, {}", if named { "named" } else { "literal" }),
            "__rope_windows",
        );
    }
}
