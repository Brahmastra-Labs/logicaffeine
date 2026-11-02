//! The region-entry guards a hoisted loop gets are a function of the loop, not of the order its array
//! accesses were collected in. `record_hoist_speculation` collects them in a map keyed by array; the
//! guards it emits must come out the same whichever order that map yields its entries.

use super::*;

fn symbols(names: &[&str]) -> Vec<Symbol> {
    let mut interner = crate::intern::Interner::new();
    names.iter().map(|n| interner.intern(n)).collect()
}

#[test]
fn every_order_of_the_collected_accesses_gives_the_same_guards() {
    let s = symbols(&["n", "i", "xs", "ys", "zs", "ws"]);
    let (bound, iv) = (s[0], s[1]);
    let accesses = [(s[2], (0, 0)), (s[3], (-1, 2)), (s[4], (0, 1)), (s[5], (2, 3))];
    let forward = hoist_descs(accesses.iter().copied(), bound, iv, 1);
    let backward = hoist_descs(accesses.iter().rev().copied(), bound, iv, 1);
    let rotated = hoist_descs(accesses.iter().cycle().skip(2).take(accesses.len()).copied(), bound, iv, 1);
    assert_eq!(forward, backward, "the guards do not depend on the order the accesses arrive in");
    assert_eq!(forward, rotated, "the guards do not depend on the order the accesses arrive in");
}

#[test]
fn the_guards_come_in_array_order() {
    let s = symbols(&["n", "i", "a", "b", "c"]);
    let (bound, iv) = (s[0], s[1]);
    let guards = hoist_descs([(s[4], (0, 0)), (s[2], (0, 0)), (s[3], (0, 0))], bound, iv, 0);
    let arrays: Vec<Symbol> = guards.iter().map(|d| d.array).collect();
    assert_eq!(arrays, vec![s[2], s[3], s[4]], "one guard per array, ordered by the array's symbol");
}
