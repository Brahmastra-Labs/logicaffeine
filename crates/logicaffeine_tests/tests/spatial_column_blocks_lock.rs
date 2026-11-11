//! spatial_column_blocks_lock — a flat section wider than an accumulator page, whose widths are not whole tiles, runs an
//! item per block of its result on the SN50 and computes what the VM computes.
//!
//! A row write into a table, a join of every head's parts and a head slice are each one pointwise stage over the flat
//! result. Their whole-tile forms move tiles; at other widths the stage's maps shift by amounts no tile boundary keeps,
//! so the section keeps every operand whole and blocks only its RESULT: an item computes `blk` result tiles, a column
//! `c` of its block being result column `c + o` for the item's first column `o`, read from a compile-time table one page
//! an item — a map that walks the columns gathers through `o`, a constant predicate bound `k` reads the table's
//! `k − o`, and a bound or gather index the section computes (a named row's offset) moves by `o` in a stage of its own.
//!
//! Each case: on an fp32 twin of the chip the program is the VM's within a relative 2⁻¹⁶, on the SN50 within the bf16
//! datapath's 2% of the largest value (exactly, for integer data), and the SN50's program is column-blocked (it holds a
//! `…__columns` table).
//!
//! An elementwise op whose right operand is one row combined with every row of a block (a bias over a prefill block)
//! is covered here too, in a block of whole tiles and one of a ragged width.
//!
//! A fold over rows blocks the same way: an attention's scores (a dot product a column) and its weighted sum of values,
//! and a per-head matmul. A chain of blocked stages hands each the item's own block — a causal block's raw scores,
//! capped and then masked — read in the block's columns, and the mask's per-head thresholds move by `o` as one vector.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test spatial_column_blocks_lock`

#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::eval_choreography;
use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::ir::{Phase, SpatialProgram};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use tensor_probes::{inputs_from_fills, oracle_ints, shown_from_outputs};

/// A value in `[-1, 1]` in steps of 1/64 for element `i` of tensor `name`.
fn seeded(name: &str, i: usize) -> f64 {
    let salt = name.bytes().fold(7u64, |h, b| h.wrapping_mul(131).wrapping_add(b as u64));
    let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29);
    ((h >> 40) % 129) as f64 / 64.0 - 1.0
}

/// Seeded tensors `(name, rows, cols, of Int)`, then `body`, then every entry of `show` (`n` of them) as
/// `round(v·1e6)`.
fn program(tensors: &[(&str, usize, usize, bool)], body: &str, show: &str, n: usize) -> String {
    let mut s = String::from("## Main\n");
    for &(name, r, c, int) in tensors {
        let kind = if int { "Int" } else { "Real" };
        s.push_str(&format!("Let {name} be a {r} by {c} tensor of {kind}.\n"));
        for i in 0..r * c {
            let v = seeded(name, i);
            if int {
                s.push_str(&format!("Set item {} of {name} to {}.\n", i + 1, (v * 64.0) as i64));
            } else {
                s.push_str(&format!("Set item {} of {name} to {v:?}.\n", i + 1));
            }
        }
    }
    s.push_str(body);
    for i in 1..=n {
        s.push_str(&format!("Show round(item {i} of {show} * 1000000).\n"));
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

fn column_blocked(sp: &SpatialProgram) -> bool {
    sp.tensors.iter().any(|t| t.name.ends_with("__columns"))
}

/// The case on the fp32 twin and on the SN50 against the VM, the SN50's program column-blocked.
fn verdict(src: &str, what: &str) {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "{what}: the VM: {:?}", vm.error);
    let want = oracle_ints(&vm.output).expect("the VM shows integers");
    for (target, chip) in [(fp32_twin(), false), (SpatialTarget::sn50_rdu(), true)] {
        let sp = composed(src, &target, what);
        assert!(column_blocked(&sp), "{what}: the program on {} is column-blocked", target.name);
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
fn a_row_write_into_a_table_wider_than_a_page_is_the_vms() {
    // 24 rows of 20: 15 tiles, rows that straddle tiles; a literal row, a row at the table's end, and a named row.
    for (what, body) in [
        ("row 7", "Set row 7 of T to S.\n"),
        ("the last row", "Set row 24 of T to S.\n"),
        ("a named row", "Let r be 13.\nSet row r of T to S.\n"),
    ] {
        verdict(&program(&[("T", 24, 20, false), ("S", 1, 20, false)], body, "T", 480), what);
    }
}

#[test]
fn an_integer_row_write_wider_than_a_page_is_the_vms() {
    for (what, body) in [("row 5 of an Int table", "Set row 5 of T to S.\n"), ("a named row of an Int table", "Let r be 22.\nSet row r of T to S.\n")] {
        verdict(&program(&[("T", 24, 20, true), ("S", 1, 20, true)], body, "T", 480), what);
    }
}

#[test]
fn a_join_whose_parts_are_not_whole_tiles_is_the_vms() {
    // Four heads of 64 + 8: 288 wide, nine tiles.
    verdict(&program(&[("A", 1, 256, false), ("B", 1, 32, false)], "Let J be A joined with B across 4 heads.\n", "J", 288), "64 + 8 across 4 heads");
}

#[test]
fn a_head_slice_whose_parts_are_not_whole_tiles_is_the_vms() {
    // Five groups of 72, the first 64 of each: 320 wide, ten tiles.
    verdict(&program(&[("X", 1, 360, false)], "Let P be the first 64 of every 72 of X.\n", "P", 320), "the first 64 of every 72");
}

/// `C = causal attention of Q over K and V` for a block of `rows` query rows of 4 heads of 16 over a cache of 12 rows of
/// 2 K/V heads: `4·rows` query heads, so the weighted sum is `64·rows` values and the scores `4·rows` a position attended.
fn causal_block(rows: usize, position: &str, clauses: &str) -> String {
    program(
        &[("Q", rows, 64, false), ("K", 12, 32, false), ("V", 12, 32, false)],
        &format!("{position}Let C be causal attention of Q over K and V with 4 heads 2 groups from {clauses}.\n"),
        "C",
        64 * rows,
    )
}

#[test]
fn a_causal_attention_whose_heads_are_not_whole_tiles_is_the_vms() {
    // 7 rows, 28 query heads: from a literal 4 the block attends 4 to 10 positions, 280 scores (nine tiles); from a
    // named 6 every row's scores span the cache's 12, 336 of them (eleven tiles), each row masked past its own bound;
    // the weighted sum is 448 values (fourteen tiles).
    verdict(&causal_block(7, "", "4"), "from a literal position");
    verdict(&causal_block(7, "Let p be 6.\n", "p"), "from a named position");
    // The soft cap takes the raw scores' block and the mask the capped block: two block-local hand-offs.
    verdict(&causal_block(7, "Let p be 6.\n", "p capped at 0.5"), "from a named position, capped");
}

#[test]
fn a_causal_attention_of_more_query_rows_than_a_tile_is_the_vms() {
    // 9 rows, 36 query heads: the probabilities' rows are more than a tile holds, so the softmax's statistics run 32
    // rows and then 4, each run in pages of its own.
    verdict(&causal_block(9, "Let p be 4.\n", "p"), "36 query rows from a named position");
}

#[test]
fn a_per_head_matmul_wider_than_a_page_is_the_vms() {
    // 7 rows of 4 heads of 16 inputs, each head times its own 16 × 12 block of W: 336 results, eleven tiles.
    verdict(&program(&[("X", 7, 64, false), ("W", 64, 12, false)], "Let Y be X matmul W across 4 heads.\n", "Y", 336), "4 heads of 16 → 12");
}

#[test]
fn the_int8_decode_step_lowers_whole_on_the_sn50_and_is_the_vms() {
    // The tiny int8 transformer's step writes a 24 × 16 cache: twelve tiles a row of sixteen.
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(repo.join("benchmarks/silicon/tiny_transformer.json")).unwrap()).unwrap();
    let m = Int8Model::from_json(&j).expect("the int8 model");
    let src = m.decode_step_lg(j["golden_trace"]["prompt_tokens"][0].as_u64().unwrap() as usize, 0, &[], &[]);
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "the VM: {:?}", vm.error);
    let want = oracle_ints(&vm.output).expect("the VM shows integers");
    let sp = composed(&src, &SpatialTarget::sn50_rdu(), "the int8 step");
    assert!(column_blocked(&sp), "the step's cache write is column-blocked");
    let out = eval_choreography(&sp, &inputs_from_fills(&sp).expect("fills")).expect("the step evaluates");
    let got = shown_from_outputs(&src, &sp, &out).expect("shown");
    assert_eq!(got, want, "the int8 step on the SN50 shows exactly what the VM shows");
}

#[test]
fn a_row_combined_with_every_row_of_a_block_is_the_vms() {
    // A bias over a block of rows — a prefill's projection plus its bias — added and multiplied, the block a matrix of
    // whole tiles and one of a ragged width.
    for (rows, w, op) in [(3, 64, "plus"), (4, 96, "hadamard"), (3, 40, "plus")] {
        let src = program(&[("A", rows, w, false), ("B", 1, w, false)], &format!("Let C be A {op} B.\n"), "C", rows * w);
        let vm = vm_outcome(&src);
        assert!(vm.error.is_none(), "{rows} rows of {w} {op} a row: the VM: {:?}", vm.error);
        let want = oracle_ints(&vm.output).expect("the VM shows integers");
        for target in [fp32_twin(), SpatialTarget::sn50_rdu()] {
            let sp = composed(&src, &target, &format!("{rows} rows of {w} {op} a row"));
            let out = eval_choreography(&sp, &inputs_from_fills(&sp).expect("fills")).expect("evaluates");
            let got = shown_from_outputs(&src, &sp, &out).expect("shown");
            let scale = want.iter().fold(1i64, |m, v| m.max(v.abs())) as f64;
            for (i, (g, v)) in got.iter().zip(&want).enumerate() {
                assert!(((g - v).abs() as f64) <= 0.02 * scale, "{rows} rows of {w} {op} a row: Show #{} on {}: {g} vs the VM's {v}", i + 1, target.name);
            }
        }
    }
}

#[test]
fn a_row_combined_with_every_row_of_a_computed_block_is_the_vms() {
    // A prefill's projection plus its bias: the block is a product the program computes, not a tensor it declares, so
    // the bias goes over it one row a band — each band reading the bias from its own row's start. Blocks of whole tiles
    // and of a ragged width, added and multiplied.
    for (rows, k, w, op) in [(3, 32, 64, "plus"), (4, 32, 96, "hadamard"), (3, 8, 40, "plus"), (2, 8, 5, "plus")] {
        let what = format!("{rows} computed rows of {w} {op} a row");
        let src = program(&[("X", rows, k, false), ("W", k, w, false), ("B", 1, w, false)], &format!("Let P be X matmul W.\nLet C be P {op} B.\n"), "C", rows * w);
        let vm = vm_outcome(&src);
        assert!(vm.error.is_none(), "{what}: the VM: {:?}", vm.error);
        let want = oracle_ints(&vm.output).expect("the VM shows integers");
        for target in [fp32_twin(), SpatialTarget::sn50_rdu()] {
            let sp = composed(&src, &target, &what);
            let out = eval_choreography(&sp, &inputs_from_fills(&sp).expect("fills")).expect("evaluates");
            let got = shown_from_outputs(&src, &sp, &out).expect("shown");
            let scale = want.iter().fold(1i64, |m, v| m.max(v.abs())) as f64;
            for (i, (g, v)) in got.iter().zip(&want).enumerate() {
                assert!(((g - v).abs() as f64) <= 0.02 * scale, "{what}: Show #{} on {}: {g} vs the VM's {v}", i + 1, target.name);
            }
        }
    }
}

#[test]
fn two_column_blocked_sections_of_one_program_each_read_their_own_table() {
    // Each column-blocked section names its table after its stage, so two cache writes of different widths in one
    // program (24 rows of 20, fifteen tiles; 30 rows of 12, twelve tiles) both hold a `write_row__columns`, with
    // different blocks. Each reads its own.
    let tables = [("T", 24, 20, false), ("S", 1, 20, false), ("U", 30, 12, false), ("R", 1, 12, false)];
    let body = "Set row 7 of T to S.\nSet row 30 of U to R.\n";
    verdict(&program(&tables, body, "T", 480), "the first of two writes");
    verdict(&program(&tables, body, "U", 360), "the second of two writes");
    // Two rotations of different head counts: six heads of 80 (fifteen tiles) and four (ten tiles), one table.
    let tables = [("Q", 1, 480, false), ("K", 1, 320, false), ("C", 6, 40, false), ("S", 6, 40, false)];
    let body = "Let P be 4.\nLet Qr be rope of Q with cos C and sin S from position P across 6 heads.\nLet Kr be rope of K with cos C and sin S from position P across 4 heads.\n";
    verdict(&program(&tables, body, "Qr", 480), "the rotation of six heads beside one of four");
    verdict(&program(&tables, body, "Kr", 320), "the rotation of four heads beside one of six");
}
