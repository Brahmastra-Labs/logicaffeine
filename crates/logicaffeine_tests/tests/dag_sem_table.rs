//! The value engine's hash tables (`dag::sem::table`) against a model: every sequence of inserts,
//! lookups and clears answers as a map would — across growth, across emptying, for every id a
//! value can be (up to one below `u32::MAX`, the one id no table ever stores), and for keys that
//! collide in their hash's top bits.

use logicaffeine_kernel::dag::sem::table::{Key, StampedTable, Table};
use std::collections::HashMap;

/// A deterministic stream of pseudo-random numbers (xorshift).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Ids at the edges and in the middle: 0 (the empty environment, the dead value), small ids, and
/// the largest id a table may hold.
fn id(r: &mut Rng) -> u32 {
    match r.below(4) {
        0 => 0,
        1 => u32::MAX - 1 - r.below(3) as u32,
        _ => r.below(1 << 20) as u32,
    }
}

trait Model<Q: Key + std::hash::Hash> {
    fn get(&self, k: &Q) -> Option<u32>;
    fn insert(&mut self, k: Q, v: u32);
    fn clear(&mut self);
}

impl<Q: Key + std::hash::Hash> Model<Q> for Table<Q> {
    fn get(&self, k: &Q) -> Option<u32> {
        Table::get(self, k)
    }
    fn insert(&mut self, k: Q, v: u32) {
        Table::insert(self, k, v)
    }
    fn clear(&mut self) {
        Table::clear(self)
    }
}

impl<Q: Key + std::hash::Hash> Model<Q> for StampedTable<Q> {
    fn get(&self, k: &Q) -> Option<u32> {
        StampedTable::get(self, k)
    }
    fn insert(&mut self, k: Q, v: u32) {
        StampedTable::insert(self, k, v)
    }
    fn clear(&mut self) {
        StampedTable::clear(self)
    }
}

/// Random operations on `t`, checked against a map after every one.
fn agrees<Q: Key + std::hash::Hash, T: Model<Q>>(
    mut t: T,
    key: impl Fn(&mut Rng) -> Q,
    seed: u64,
    ops: usize,
) {
    let mut r = Rng(seed);
    let mut model: HashMap<Q, u32> = HashMap::new();
    let mut keys: Vec<Q> = Vec::new();
    for step in 0..ops {
        match r.below(100) {
            0 => {
                t.clear();
                model.clear();
            }
            1..=45 => {
                let k = if !keys.is_empty() && r.below(4) == 0 {
                    keys[r.below(keys.len() as u64) as usize]
                } else {
                    key(&mut r)
                };
                if !model.contains_key(&k) {
                    let v = id(&mut r);
                    t.insert(k, v);
                    model.insert(k, v);
                    keys.push(k);
                }
            }
            _ => {
                let k = if !keys.is_empty() && r.below(2) == 0 {
                    keys[r.below(keys.len() as u64) as usize]
                } else {
                    key(&mut r)
                };
                assert_eq!(
                    t.get(&k),
                    model.get(&k).copied(),
                    "step {step}: lookup disagrees with the model"
                );
            }
        }
    }
    for k in &keys {
        assert_eq!(
            t.get(k),
            model.get(k).copied(),
            "final lookup disagrees with the model"
        );
    }
}

fn pair(r: &mut Rng) -> (u32, u32) {
    (id(r), r.below(64) as u32)
}

fn triple(r: &mut Rng) -> (u32, u32, u32) {
    (r.below(1000) as u32, id(r), r.below(8) as u32)
}

fn quad(r: &mut Rng) -> (u32, u32, u32, u32) {
    (r.below(3) as u32, id(r), r.below(100) as u32, id(r))
}

fn masked(r: &mut Rng) -> (u32, u64) {
    (id(r), 1u64 << r.below(64) | r.next() & 0xff)
}

#[test]
fn a_table_answers_like_a_map_through_growth_and_clearing() {
    for seed in 1..=8 {
        agrees(Table::new(), pair, seed, 200_000);
        agrees(Table::new(), triple, seed, 100_000);
        agrees(Table::new(), quad, seed, 100_000);
        agrees(Table::new(), masked, seed, 100_000);
    }
}

#[test]
fn a_stamped_table_answers_like_a_map_through_growth_and_clearing() {
    for seed in 1..=8 {
        agrees(StampedTable::new(), pair, seed, 200_000);
        agrees(StampedTable::new(), triple, seed, 100_000);
    }
}

#[test]
fn a_cleared_table_forgets_everything_and_accepts_every_key_again() {
    let mut t: Table<(u32, u32)> = Table::new();
    for round in 0..5u32 {
        for i in 0..50_000u32 {
            t.insert((i, round), i ^ round);
        }
        assert_eq!(t.len(), 50_000);
        assert_eq!(t.get(&(7, round)), Some(7 ^ round));
        t.clear();
        assert!(t.is_empty());
        for i in (0..50_000u32).step_by(997) {
            assert_eq!(
                t.get(&(i, round)),
                None,
                "cleared entries are gone (round {round})"
            );
        }
    }
}
