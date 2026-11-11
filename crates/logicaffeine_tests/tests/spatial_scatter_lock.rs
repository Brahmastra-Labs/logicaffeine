//! spatial_scatter_lock — a scatter of rows, written as the VM's own loop, is lifted by what the loop means and lowered
//! on the SN50 as one owner-computes rewrite of the table, repeated indices included (KERNEL-4 of
//! `benchmarks/PREPARE_FOR_SAMBA.md`).
//!
//! The loop `Repeat for k from 1 to N: Let at be item k of I. Let r be row k of X. Set row at of T to r.` writes row `k`
//! of `X` at row `I[k]` of `T`, turn by turn, so a row named twice keeps the later write; with `Let old be row at of T.
//! Let sum be old plus r.` before the write, a row named twice takes both. The walk lifts the loop as ONE `Scatter` op —
//! never as the row read, the sum or the row write it holds, each of which runs once a turn — and the SN50 computes it
//! as the VM does. An index the program gives outside the table fails its turn's row check on every tier: the loop is
//! listed, never lifted. An index the program computes is data, checked where the scatter reads it: outside the table it
//! faults, never wrapped into another row.
//!
//!   cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test spatial_scatter_lock

#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
#[allow(dead_code)]
mod tensor_probes;
#[path = "common/written_programs.rs"]
#[allow(dead_code)]
mod written_programs;

use std::collections::BTreeMap;

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::ChoreoFault;
use logicaffeine_compile::spatial::pipeline::value_ranges_of;
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::tensor_graph::eval::{eval, Data};
use logicaffeine_compile::tensor_graph::graph::{Storage, TensorGraph};
use written_programs::{faults_on_the_sn50, filled, ints, ints_shaped, lifted, lowered, on_the_sn50, seeded, shows, shows_exact, walk};

/// The turn that keeps the later write of a row named twice.
const KEPT: &str = "    Let r be row k of X.\n    Set row at of T to r.\n";
/// The turn that adds each row into the row it lands on.
const SUMMED: &str = "    Let r be row k of X.\n    Let old be row at of T.\n    Let sum be old plus r.\n    Set row at of T to sum.\n";

/// A scatter over a `rows × cols` Real table `T` of `index.len()` rows of a Real source `X`, by the declared indices
/// `index`, its turn `turn`; every element of `T` shown.
fn scatter(rows: usize, cols: usize, index: &[i64], turn: &str) -> String {
    let n = index.len();
    format!(
        "## Main\n{}{}{}Repeat for k from 1 to {n}:\n    Let at be item k of I.\n{turn}{}",
        filled("T", rows, cols, &|i| seeded("T", i)),
        filled("X", n, cols, &|i| seeded("X", i)),
        ints("I", index),
        shows("T", rows * cols)
    )
}

/// The walk of a program that is one scatter: it lifts the loop, by its index, as one `Scatter`, and lists nothing.
fn one_scatter(src: &str, what: &str) {
    let rp = walk(src);
    assert!(rp.unrecognized.is_empty(), "{what}: the walk lists {:?}", rp.unrecognized);
    let loop_at = rp.declared.len() + src.lines().filter(|l| l.starts_with("Set item")).count();
    assert_eq!(lifted(src), vec![("Scatter", loop_at)], "{what}: the loop lifts as one scatter, by its own index");
}

#[test]
fn a_scatter_with_repeats_sums_as_the_vm() {
    // Rows 3 and 1 are named three times and twice: each takes every row scattered onto it.
    let src = scatter(8, 16, &[3, 3, 1, 8, 3, 1], SUMMED);
    one_scatter(&src, "a summed scatter");
    let sp = on_the_sn50(&src, "a summed scatter with repeated indices", false);
    assert_eq!(sp.sections.len(), 1, "the scatter is one section, not {}", sp.sections.len());
}

#[test]
fn a_scatter_keeps_the_later_write_of_a_row_named_twice() {
    let index = [3, 3, 1, 8, 3, 1];
    let src = scatter(8, 16, &index, KEPT);
    one_scatter(&src, "a kept scatter");
    let sp = on_the_sn50(&src, "a kept scatter with repeated indices", false);
    assert_eq!(sp.sections.len(), 1, "the scatter is one section, not {}", sp.sections.len());
    // The kept scatter only moves values, each exact in bf16: row 3 is X's fifth row, row 1 its sixth, row 8 its fourth,
    // every other row T's own.
    let vm = vm_outcome(&src);
    let shown: Vec<i64> = vm.output.split_whitespace().map(|t| t.parse().unwrap()).collect();
    let at = |r: usize, c: usize| shown[(r - 1) * 16 + c];
    let x = |k: usize, c: usize| (seeded("X", (k - 1) * 16 + c) * 1e6).round() as i64;
    for c in 0..16 {
        assert_eq!(at(3, c), x(5, c), "row 3 keeps the last of the three writes to it");
        assert_eq!(at(1, c), x(6, c), "row 1 keeps the later of its two writes");
        assert_eq!(at(8, c), x(4, c), "row 8 keeps its one write");
        assert_eq!(at(2, c), (seeded("T", 16 + c) * 1e6).round() as i64, "row 2 is never written");
    }
}

#[test]
fn every_index_the_same_lands_every_row_on_one() {
    for (turn, what) in [(KEPT, "kept"), (SUMMED, "summed")] {
        let src = scatter(4, 32, &[2, 2, 2, 2, 2], turn);
        on_the_sn50(&src, &format!("a {what} scatter whose every index is 2"), false);
    }
}

#[test]
fn one_turn_and_a_scatter_onto_every_row() {
    for (turn, what) in [(KEPT, "kept"), (SUMMED, "summed")] {
        on_the_sn50(&scatter(4, 8, &[4], turn), &format!("one {what} turn"), false);
        on_the_sn50(&scatter(4, 8, &[4, 1, 3, 2], turn), &format!("a {what} permutation of every row"), false);
    }
}

#[test]
fn a_loop_of_fewer_turns_reads_the_first_of_longer_operands() {
    // Three turns over five indices and five rows of the source: the last two of each are never read.
    for (turn, what) in [(KEPT, "kept"), (SUMMED, "summed")] {
        let src = format!(
            "## Main\n{}{}{}Repeat for k from 1 to 3:\n    Let at be item k of I.\n{turn}{}",
            filled("T", 6, 16, &|i| seeded("T", i)),
            filled("X", 5, 16, &|i| seeded("X", i)),
            ints("I", &[4, 2, 4, 9, 0]),
            shows("T", 96)
        );
        assert_eq!(lifted(&src).iter().map(|l| l.0).collect::<Vec<_>>(), vec!["Scatter"], "{what}: the indices past the third are never read");
        on_the_sn50(&src, &format!("a {what} scatter of three turns"), false);
    }
}

#[test]
fn an_integer_table_scatters_exactly() {
    for (turn, what) in [(KEPT, "kept"), (SUMMED, "summed")] {
        let index = [2, 4, 2, 1];
        let table: Vec<i64> = (0..4 * 8).map(|i| (i as i64 * 37) % 23 - 11).collect();
        let source: Vec<i64> = (0..4 * 8).map(|i| (i as i64 * 53) % 29 - 14).collect();
        let src = format!(
            "## Main\n{}{}{}Repeat for k from 1 to 4:\n    Let at be item k of I.\n{turn}{}",
            ints_shaped("T", 4, 8, &table),
            ints_shaped("X", 4, 8, &source),
            ints("I", &index),
            shows_exact("T", 32)
        );
        on_the_sn50(&src, &format!("a {what} scatter of Ints"), true);
    }
}

#[test]
fn an_integer_table_is_ranged_over_every_value_the_turns_leave_in_it() {
    // The interval the front end proves for `T` holds every value the VM leaves there: a kept scatter's own values and its
    // source's, bounded; a summed one's sums, which the table carries from step to step, bounded by nothing. The indices
    // hold every row of the table, as a gather's do.
    let table: Vec<i64> = (0..4 * 8).map(|i| (i as i64 * 37) % 23 - 11).collect();
    let source: Vec<i64> = (0..4 * 8).map(|i| (i as i64 * 53) % 29 - 14).collect();
    for (turn, what) in [(KEPT, "kept"), (SUMMED, "summed")] {
        let src = format!(
            "## Main\n{}{}{}Repeat for k from 1 to 4:\n    Let at be item k of I.\n{turn}{}",
            ints_shaped("T", 4, 8, &table),
            ints_shaped("X", 4, 8, &source),
            ints("I", &[2, 2, 1, 2]),
            shows_exact("T", 32)
        );
        let vm = vm_outcome(&src);
        assert!(vm.error.is_none(), "{what}: the VM: {:?}", vm.error);
        let values: Vec<i64> = vm.output.split_whitespace().map(|t| t.parse().unwrap()).collect();
        let ranges = value_ranges_of(&src).unwrap_or_else(|e| panic!("{what}: {e:?}"));
        let (lo, hi) = *ranges.get("T").unwrap_or_else(|| panic!("{what}: `T` is ranged"));
        let (least, most) = (*values.iter().min().unwrap(), *values.iter().max().unwrap());
        assert!(lo <= least && most <= hi, "{what}: `T` is proven in [{lo}, {hi}], and the VM leaves values in [{least}, {most}]");
        let bounded = hi < i64::MAX && lo > i64::MIN;
        assert_eq!(bounded, turn == KEPT, "{what}: `T` is proven in [{lo}, {hi}]");
        assert_eq!(ranges.get("I"), Some(&(1, 4)), "{what}: the indices hold every row of the table");
    }
}

#[test]
fn a_computed_index_is_data_the_scatter_reads() {
    // `I` is a sum the program computes, so the walk cannot read its values: the scatter checks each where it reads it.
    for (turn, what) in [(KEPT, "kept"), (SUMMED, "summed")] {
        let src = format!(
            "## Main\n{}{}{}{}Let I be J plus Z.\nRepeat for k from 1 to 5:\n    Let at be item k of I.\n{turn}{}",
            filled("T", 6, 16, &|i| seeded("T", i)),
            filled("X", 5, 16, &|i| seeded("X", i)),
            ints("J", &[5, 1, 5, 6, 1]),
            ints("Z", &[0, 0, 0, 0, 0]),
            shows("T", 96)
        );
        assert_eq!(lifted(&src).iter().map(|l| l.0).collect::<Vec<_>>(), vec!["Elementwise", "Scatter"], "{what}");
        on_the_sn50(&src, &format!("a {what} scatter by a computed index"), false);
    }
}

#[test]
fn a_computed_index_outside_the_table_faults_never_wraps() {
    // Row 9 of a table of 6: the VM fails the turn's row check, and the SN50 faults where it reads the index — the write is
    // never wrapped into another row, nor silently dropped.
    for (turn, what) in [(KEPT, "kept"), (SUMMED, "summed")] {
        for bad in [9i64, 0, 7] {
            let src = format!(
                "## Main\n{}{}{}{}Let I be J plus Z.\nRepeat for k from 1 to 3:\n    Let at be item k of I.\n{turn}{}",
                filled("T", 6, 16, &|i| seeded("T", i)),
                filled("X", 3, 16, &|i| seeded("X", i)),
                ints("J", &[2, bad, 1]),
                ints("Z", &[0, 0, 0]),
                shows("T", 96)
            );
            assert!(vm_outcome(&src).error.is_some(), "{what}, index {bad}: the VM fails the row check");
            match faults_on_the_sn50(&src, &format!("a {what} scatter to row {bad} of 6")) {
                ChoreoFault::GatherOutOfRange { .. } => {}
                other => panic!("{what}, index {bad}: the SN50 stops at {other:?}, not at the index's range check"),
            }
        }
    }
}

#[test]
fn an_index_the_program_gives_outside_the_table_is_listed_never_lifted() {
    // The fourth turn names row 9 of 8: it fails its row check on every tier, so the loop is no scatter. A sum before it
    // lifts, so the lowering is partial and its refusal names the loop.
    for (turn, what) in [(KEPT, "kept"), (SUMMED, "summed")] {
        let src = scatter(8, 16, &[3, 3, 1, 9, 3, 1], turn).replacen("Repeat for k", "Let D be X plus X.\nRepeat for k", 1);
        assert!(vm_outcome(&src).error.is_some(), "{what}: the VM fails the turn naming row 9");
        let rp = walk(&src);
        assert_eq!(lifted(&src).iter().map(|l| l.0).collect::<Vec<_>>(), vec!["Elementwise"], "{what}: only the sum lifts");
        assert_eq!(rp.unrecognized.len(), 1, "{what}: the loop is listed: {:?}", rp.unrecognized);
        assert_eq!(rp.unrecognized[0].1, "Repeat", "{what}");
        let why = lowered(&src, &SpatialTarget::sn50_rdu()).err().unwrap_or_else(|| panic!("{what}: lowered without its loop"));
        assert!(why.contains("Repeat"), "{what}: the refusal names the loop: {why}");
    }
}

#[test]
fn a_turn_that_does_more_than_scatter_is_listed() {
    let decls = |turns: usize| {
        format!(
            "## Main\n{}{}{}",
            filled("T", 4, 8, &|i| seeded("T", i)),
            filled("X", 4, 8, &|i| seeded("X", i)),
            ints("I", &[2, 3, 2, 1][..turns])
        )
    };
    for (what, body) in [
        ("a row scaled before it lands", "    Let at be item k of I.\n    Let r be row k of X.\n    Let s be r scaled by 2.0.\n    Set row at of T to s.\n"),
        ("a row read at a fixed index", "    Let at be item 1 of I.\n    Let r be row k of X.\n    Set row at of T to r.\n"),
        ("a row of the table written back unchanged", "    Let at be item k of I.\n    Let r be row k of X.\n    Let old be row at of T.\n    Set row at of T to old.\n"),
        ("a row written at the turn itself", "    Let at be item k of I.\n    Let r be row k of X.\n    Set row k of T to r.\n"),
        ("a sum of the row with itself", "    Let at be item k of I.\n    Let r be row k of X.\n    Let old be row at of T.\n    Let sum be r plus r.\n    Set row at of T to sum.\n"),
    ] {
        let src = format!("{}Repeat for k from 1 to 4:\n{body}{}", decls(4), shows("T", 32));
        let rp = walk(&src);
        assert!(rp.stages.is_empty(), "{what}: nothing lifts: {:?}", rp.stages.iter().map(|s| s.op.kind()).collect::<Vec<_>>());
        assert_eq!(rp.unrecognized.iter().map(|u| u.1.as_str()).collect::<Vec<_>>(), vec!["Repeat"], "{what}: the loop is listed");
    }
    // More turns than the index has entries: the fifth turn reads past `I`'s end on every tier.
    let src = format!("{}Repeat for k from 1 to 5:\n    Let at be item k of I.\n{KEPT}{}", decls(4), shows("T", 32));
    assert!(walk(&src).stages.is_empty(), "five turns over four indices lift nothing");
}

#[test]
fn a_table_wider_than_a_page_is_refused_by_name() {
    // 64 rows of 64: 4,096 elements, past the 256 a site's page holds on the SN50.
    let src = scatter(64, 64, &[3, 7, 3], KEPT);
    one_scatter(&src, "a scatter into a 64 by 64 table");
    let why = lowered(&src, &SpatialTarget::sn50_rdu()).err().expect("a wide scatter is refused");
    assert!(why.contains("sorted by row"), "the refusal says what a wider scatter needs: {why}");
}

#[test]
fn a_scatter_that_multiplies_lifts_and_is_refused_by_name() {
    let turn = "    Let r be row k of X.\n    Let old be row at of T.\n    Let prod be old hadamard r.\n    Set row at of T to prod.\n";
    let src = scatter(8, 16, &[3, 3, 1], turn);
    one_scatter(&src, "a multiplying scatter");
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "the VM multiplies: {:?}", vm.error);
    let why = lowered(&src, &SpatialTarget::sn50_rdu()).err().expect("a multiplying scatter is refused");
    assert!(why.contains("multiply"), "the refusal names the fold: {why}");
}

#[test]
fn the_evaluator_scatters_as_the_vm() {
    // The graph's meaning is the VM's to the last bit: every element of the table after the loop, a Real and an Int
    // table, kept and summed, a multiplying fold too.
    let product = "    Let r be row k of X.\n    Let old be row at of T.\n    Let prod be old hadamard r.\n    Set row at of T to prod.\n";
    for (turn, what) in [(KEPT, "kept"), (SUMMED, "summed"), (product, "multiplied")] {
        let src = scatter(8, 16, &[3, 3, 1, 8, 3, 1], turn);
        let rp = walk(&src);
        let g = TensorGraph::lift(&rp, None, None, &|_| Storage::Values).unwrap_or_else(|e| panic!("{what}: the graph: {e}"));
        let env = eval(&g, &BTreeMap::new()).unwrap_or_else(|e| panic!("{what}: the evaluator: {e}"));
        let Some(Data::Real(t)) = env.get("T") else { panic!("{what}: the evaluator has no Real `T`") };
        let shown: String = (1..=128).map(|i| format!("Show item {i} of T.\n")).collect();
        let plain = src.lines().filter(|l| !l.starts_with("Show ")).map(|l| format!("{l}\n")).collect::<String>() + &shown;
        let vm = vm_outcome(&plain);
        assert!(vm.error.is_none(), "{what}: the VM: {:?}", vm.error);
        let printed: Vec<&str> = vm.output.lines().collect();
        assert_eq!(printed.len(), t.len(), "{what}: one value an element");
        for (i, (p, v)) in printed.iter().zip(t).enumerate() {
            assert_eq!(*p, format!("{v}"), "{what}: element {i}");
        }
    }
}
