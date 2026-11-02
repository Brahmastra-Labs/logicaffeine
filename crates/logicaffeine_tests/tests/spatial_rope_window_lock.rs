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
    let mut s = String::from("## Main\n");
    for (name, r, c) in [("Q", 1, heads * hd), ("C", ctx, half), ("S", ctx, half)] {
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
    for i in 1..=heads * hd {
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
