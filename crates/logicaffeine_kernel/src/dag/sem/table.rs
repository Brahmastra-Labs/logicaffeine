//! The value engine's hash tables: open addressing with linear probing over small `Copy` keys
//! and `u32` values, one slot per cache line access, Fibonacci-hashed.
//!
//! Every value the engine builds is interned and every cache is keyed on ids, so these tables
//! are on the hottest paths — an environment extension, a neutral application, an evaluation
//! lookup each probe one. A general hash map spends several times the instructions of a probe
//! here on hashing, metadata and growth policy.

use crate::dag::slot;

/// A key a table can hold: its hash must be well mixed in the TOP bits.
pub trait Key: Copy + Eq {
    /// Every bit set: what a vacant slot holds, so that emptying a table is one byte fill.
    const VACANT: Self;
    fn hash(&self) -> u64;
}

const K: u64 = 0x9E37_79B9_7F4A_7C15;

#[inline]
fn mix2(a: u32, b: u32) -> u64 {
    (((a as u64) << 32) | b as u64).wrapping_mul(K)
}

impl Key for (u32, u32) {
    const VACANT: Self = (u32::MAX, u32::MAX);

    #[inline]
    fn hash(&self) -> u64 {
        mix2(self.0, self.1)
    }
}

impl Key for (u32, u32, u32) {
    const VACANT: Self = (u32::MAX, u32::MAX, u32::MAX);

    #[inline]
    fn hash(&self) -> u64 {
        (mix2(self.0, self.1) ^ (self.2 as u64)).wrapping_mul(K)
    }
}

impl Key for (u32, u32, u32, u32) {
    const VACANT: Self = (u32::MAX, u32::MAX, u32::MAX, u32::MAX);

    #[inline]
    fn hash(&self) -> u64 {
        (mix2(self.0, self.1) ^ (((self.2 as u64) << 32) | self.3 as u64)).wrapping_mul(K)
    }
}

impl Key for (u32, u64) {
    const VACANT: Self = (u32::MAX, u64::MAX);

    #[inline]
    fn hash(&self) -> u64 {
        ((self.0 as u64).wrapping_mul(K) ^ self.1).wrapping_mul(K)
    }
}

const EMPTY: u32 = u32::MAX;

#[derive(Copy, Clone)]
struct Slot<Q> {
    key: Q,
    /// [`EMPTY`] marks a vacant slot.
    val: u32,
}

/// A table that lives for a session: emptied rarely, by filling its slots with the vacant
/// pattern (all bits set — a `memset`).
#[derive(Clone)]
pub struct Table<Q: Key> {
    slots: Vec<Slot<Q>>,
    shift: u32,
    len: usize,
}

impl<Q: Key> Default for Table<Q> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Q: Key> Table<Q> {
    const VACANT: Slot<Q> = Slot {
        key: Q::VACANT,
        val: EMPTY,
    };

    pub fn new() -> Table<Q> {
        let cap = 1 << 10;
        Table {
            slots: vec![Self::VACANT; cap],
            shift: 64 - 10,
            len: 0,
        }
    }

    #[inline]
    pub fn get(&self, k: &Q) -> Option<u32> {
        let mask = self.slots.len() - 1;
        let mut i = (k.hash() >> self.shift) as usize;
        loop {
            let s = slot::at(&self.slots, i);
            if s.val == EMPTY {
                return None;
            }
            if s.key == *k {
                return Some(s.val);
            }
            i = (i + 1) & mask;
        }
    }

    /// Inserts a key that is not present (`v` is never `EMPTY`: ids stay below it).
    #[inline]
    pub fn insert(&mut self, k: Q, v: u32) {
        debug_assert!(v != EMPTY);
        if (self.len + 1) * 2 > self.slots.len() {
            self.grow();
        }
        self.place(k, v);
        self.len += 1;
    }

    #[inline]
    fn place(&mut self, k: Q, v: u32) {
        let mask = self.slots.len() - 1;
        let mut i = (k.hash() >> self.shift) as usize;
        while slot::at(&self.slots, i).val != EMPTY {
            i = (i + 1) & mask;
        }
        *slot::at_mut(&mut self.slots, i) = Slot { key: k, val: v };
    }

    fn grow(&mut self) {
        let cap = self.slots.len() * 2;
        let old = std::mem::replace(&mut self.slots, vec![Self::VACANT; cap]);
        self.shift = 64 - cap.trailing_zeros();
        for s in old {
            if s.val != EMPTY {
                self.place(s.key, s.val);
            }
        }
    }

    pub fn contains(&self, k: &Q) -> bool {
        self.get(k).is_some()
    }

    /// Empties the table, keeping its storage.
    pub fn clear(&mut self) {
        if self.len > 0 {
            self.slots.fill(Self::VACANT);
            self.len = 0;
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn bytes(&self) -> usize {
        self.slots.capacity() * std::mem::size_of::<Slot<Q>>()
    }
}

#[derive(Copy, Clone)]
struct StampedSlot<Q> {
    key: Q,
    val: u32,
    /// The table generation that wrote this slot; older generations read as vacant.
    generation: u32,
}

/// A table emptied often (at every declaration): emptying it is one increment.
#[derive(Clone)]
pub struct StampedTable<Q: Key> {
    slots: Vec<StampedSlot<Q>>,
    shift: u32,
    len: usize,
    generation: u32,
}

impl<Q: Key> Default for StampedTable<Q> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Q: Key> StampedTable<Q> {
    const VACANT: StampedSlot<Q> = StampedSlot {
        key: Q::VACANT,
        val: 0,
        generation: 0,
    };

    pub fn new() -> StampedTable<Q> {
        let cap = 1 << 10;
        StampedTable {
            slots: vec![Self::VACANT; cap],
            shift: 64 - 10,
            len: 0,
            generation: 1,
        }
    }

    #[inline]
    pub fn get(&self, k: &Q) -> Option<u32> {
        let mask = self.slots.len() - 1;
        let mut i = (k.hash() >> self.shift) as usize;
        loop {
            let s = slot::at(&self.slots, i);
            if s.generation != self.generation {
                return None;
            }
            if s.key == *k {
                return Some(s.val);
            }
            i = (i + 1) & mask;
        }
    }

    /// Inserts a key that is not present.
    #[inline]
    pub fn insert(&mut self, k: Q, v: u32) {
        if (self.len + 1) * 2 > self.slots.len() {
            self.grow();
        }
        self.place(k, v);
        self.len += 1;
    }

    #[inline]
    fn place(&mut self, k: Q, v: u32) {
        let mask = self.slots.len() - 1;
        let mut i = (k.hash() >> self.shift) as usize;
        while slot::at(&self.slots, i).generation == self.generation {
            i = (i + 1) & mask;
        }
        *slot::at_mut(&mut self.slots, i) = StampedSlot {
            key: k,
            val: v,
            generation: self.generation,
        };
    }

    fn grow(&mut self) {
        let cap = self.slots.len() * 2;
        let old = std::mem::replace(&mut self.slots, vec![Self::VACANT; cap]);
        self.shift = 64 - cap.trailing_zeros();
        let g = self.generation;
        self.generation = 1;
        for s in old {
            if s.generation == g {
                self.place(s.key, s.val);
            }
        }
    }

    /// Empties the table in O(1), keeping its storage.
    pub fn clear(&mut self) {
        self.len = 0;
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            for s in self.slots.iter_mut() {
                s.generation = 0;
            }
            self.generation = 1;
        }
    }

    pub fn bytes(&self) -> usize {
        self.slots.capacity() * std::mem::size_of::<StampedSlot<Q>>()
    }
}
