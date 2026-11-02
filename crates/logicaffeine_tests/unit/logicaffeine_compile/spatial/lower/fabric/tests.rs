//! The row pitch's arithmetic, at its edges: the word a row moves in, a tile offset placed at a pitch, and which walks keep
//! within a row — each term of a walk at the largest value it takes, a walk that reaches exactly the next row's first
//! column, and the terms that take no value at all.

use super::{pitched, row_word, walk_splits};
use crate::spatial::ir::{IndirectTile, LoopVar, TileAddr};
use crate::spatial::passes::{LV_ITEM_X, LV_ITEM_Y};
use std::collections::BTreeMap;

const K: LoopVar = LoopVar(2);

fn addr(base: usize, steps: &[(LoopVar, usize)]) -> TileAddr {
    TileAddr { base, steps: steps.to_vec(), indirect: None }
}

fn loops(of: &[(LoopVar, usize)]) -> BTreeMap<LoopVar, usize> {
    of.iter().copied().collect()
}

#[test]
fn a_row_moves_in_the_narrowest_word_that_holds_it_no_wider_than_the_banks() {
    for (width, banks, word) in [(14, 16, 16), (14, 8, 8), (5, 16, 8), (6, 2, 2), (1, 16, 1), (16, 16, 16), (17, 16, 16), (3, 1, 1)] {
        assert_eq!(row_word(width, banks), word, "a row of {width} at {banks} banks");
    }
}

#[test]
fn a_tile_offset_keeps_its_column_and_places_its_rows_at_the_pitch() {
    for (s, cols, pitch, at) in [(0, 14, 16, 0), (13, 14, 16, 13), (14, 14, 16, 16), (16, 14, 16, 18), (45, 14, 16, 51), (30, 7, 8, 34), (5, 5, 5, 5)] {
        assert_eq!(pitched(s, cols, pitch), at, "offset {s} of {cols} columns at a pitch of {pitch}");
    }
}

#[test]
fn a_row_read_keeps_within_its_row_and_one_from_past_its_start_does_not() {
    let none = loops(&[]);
    assert!(walk_splits(&addr(14, &[]), (1, 14, 1, 14), &none, &[], 14), "row 1 of a 14-column tensor");
    assert!(walk_splits(&addr(0, &[]), (3, 14, 1, 14), &none, &[], 14), "three whole rows");
    assert!(!walk_splits(&addr(1, &[]), (1, 14, 1, 14), &none, &[], 14), "fourteen from column 1 reach the next row");
    assert!(walk_splits(&addr(1, &[]), (1, 13, 1, 14), &none, &[], 14), "thirteen from column 1 end at the last column");
    assert!(!walk_splits(&addr(0, &[]), (2, 14, 1, 15), &none, &[], 14), "whole rows fifteen apart: the second runs across");
    assert!(walk_splits(&addr(0, &[]), (2, 13, 1, 15), &none, &[], 14), "rows of thirteen fifteen apart end at the last column");
}

#[test]
fn a_loop_term_reaches_as_far_as_its_last_trip_and_an_unbound_one_not_at_all() {
    // Blocks of four along a row of twelve: three trips end at column 11, four run into the next row.
    assert!(walk_splits(&addr(0, &[(K, 4)]), (1, 4, 1, 12), &loops(&[(K, 3)]), &[], 12));
    assert!(!walk_splits(&addr(0, &[(K, 4)]), (1, 4, 1, 12), &loops(&[(K, 4)]), &[], 12));
    // A term of whole rows moves no column, however many trips.
    assert!(walk_splits(&addr(0, &[(K, 12)]), (1, 12, 1, 12), &loops(&[(K, 9)]), &[], 12));
    // A variable no loop binds adds nothing to the address.
    assert!(walk_splits(&addr(0, &[(K, 4)]), (1, 12, 1, 12), &loops(&[]), &[], 12));
}

#[test]
fn an_item_term_reaches_as_far_as_the_largest_coordinate() {
    let none = loops(&[]);
    assert!(walk_splits(&addr(0, &[(LV_ITEM_X, 3)]), (1, 3, 1, 9), &none, &[(0, 0), (0, 1), (0, 2)], 9));
    assert!(!walk_splits(&addr(0, &[(LV_ITEM_X, 3)]), (1, 3, 1, 9), &none, &[(0, 0), (0, 3)], 9));
    assert!(walk_splits(&addr(0, &[(LV_ITEM_Y, 9)]), (1, 9, 1, 9), &none, &[(5, 0)], 9), "rows of items move no column");
    assert!(!walk_splits(&addr(0, &[(LV_ITEM_Y, 4)]), (1, 6, 1, 9), &none, &[(1, 0)], 9), "an item four columns on runs across");
}

#[test]
fn a_gather_reaches_as_far_as_its_bound() {
    let none = loops(&[]);
    let gather = |hi: usize, tile_stride: usize| TileAddr {
        base: 0,
        steps: Vec::new(),
        indirect: Some(IndirectTile { index: crate::spatial::ir::BufferId::new(0), tile_stride, elems_per_tile: 1, bound: (0, hi) }),
    };
    assert!(walk_splits(&gather(4, 7), (1, 7, 1, 7), &none, &[], 7), "a row gather moves whole rows");
    assert!(walk_splits(&gather(5, 1), (1, 2, 1, 7), &none, &[], 7), "two from at most column 5 end at the last column");
    assert!(!walk_splits(&gather(6, 1), (1, 2, 1, 7), &none, &[], 7), "two from column 6 reach the next row");
}
