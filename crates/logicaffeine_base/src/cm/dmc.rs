//! Dynamic Markov coding: bit-level state machines that start as 256 byte trees — one per previous byte, so an
//! order-1 model — and grow longer contexts by cloning a state when one incoming transition carries most of its
//! traffic. They learn which histories matter from the data, with no hashing and no fixed orders, so their
//! predictions are a different kind of input to the mixer from the hashed contexts'.
//!
//! A [`Forest`] runs machines with different cloning thresholds and sizes: a low threshold grows deep contexts
//! fast and fills early, a high one grows slowly and suits steady data. The fast machines start over once full
//! (each at its own time); the slow ones never do.

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_base/cm/dmc/tests.rs"]
mod tests;

use super::{next_state, seeded_maps};
use crate::entropy::{prefetch, stretch, AdaptiveProb};

/// One count of a bit, in the fixed point the counts are kept in.
const ONE: u32 = 512;

/// The highest a cloning threshold climbs as its machine fills.
const THRESHOLD_MAX: u32 = 8 * ONE;

/// The 256 byte trees of 255 states each.
const TREES: usize = 256 * 255;

/// The low 28 bits of a link: the state it leads to.
const INDEX: u32 = (1 << 28) - 1;

/// A state in 12 bytes: where each bit leads (a link's low 28 bits), the state's bit history (its two nibbles in the
/// links' top four bits), and a decaying count of each bit seen here.
#[derive(Clone, Copy, Default)]
struct Node {
    links: [u32; 2],
    count: [u16; 2],
}

impl Node {
    fn new(next: [u32; 2], count: [u16; 2], history: u8) -> Self {
        let mut n = Self { links: next, count };
        n.set_history(history);
        n
    }

    #[inline]
    fn next(&self, bit: usize) -> u32 {
        self.links[bit] & INDEX
    }

    #[inline]
    fn set_next(&mut self, bit: usize, to: u32) {
        self.links[bit] = self.links[bit] & !INDEX | to;
    }

    #[inline]
    fn history(&self) -> u8 {
        (self.links[0] >> 28 << 4 | self.links[1] >> 28) as u8
    }

    #[inline]
    fn set_history(&mut self, h: u8) {
        self.links[0] = self.links[0] & INDEX | u32::from(h >> 4) << 28;
        self.links[1] = self.links[1] & INDEX | u32::from(h & 15) << 28;
    }
}

/// One machine. A transition clones its target once it has carried more than `threshold` and the target has
/// seen more than `threshold` beyond it; the clone takes the transition's share of the target's counts and its
/// transitions, so every state keeps one bit position in the byte.
pub struct Dmc {
    nodes: Vec<Node>,
    capacity: usize,
    at: u32,
    threshold: u32,
    /// The threshold in 1/4096 steps: it rises one step with each clone.
    fine: u32,
    start: u32,
    /// Traffic that would have cloned with room to spare: once it passes the machine's size, the structure has
    /// stopped fitting the data.
    stalled: u64,
    maps: [AdaptiveProb; 256],
}

impl Dmc {
    /// A machine of at most `capacity` states (at least the byte trees) cloning above `threshold` (in 1/512 counts).
    pub fn new(capacity: usize, threshold: u32) -> Self {
        let capacity = capacity.clamp(TREES, 1 << 28);
        let mut m = Self { nodes: Vec::new(), capacity, at: 0, threshold, fine: 0, start: threshold, stalled: 0, maps: *seeded_maps() };
        m.reset(0);
        m
    }

    /// Back to the byte trees, at the root of `last`'s: state `t · 255 + k − 1` is tree `t`'s partial byte `k`
    /// (1..=255), and the eighth bit leads to the root of the completed byte's tree. A low threshold's counts
    /// start at two of each bit, a high one's at half, so a clone splits from a first guess of the right size.
    fn reset(&mut self, last: u8) {
        let initial = if self.start < ONE { 2 * ONE } else { ONE / 2 } as u16;
        self.nodes.clear();
        self.nodes.reserve(self.capacity.min(TREES * 4));
        for t in 0..256u32 {
            for k in 1..256u32 {
                let next = |bit: u32| if k < 128 { t * 255 + 2 * k + bit - 1 } else { ((2 * k + bit) & 255) * 255 };
                self.nodes.push(Node::new([next(0), next(1)], [initial; 2], 0));
            }
        }
        self.at = u32::from(last) * 255;
        self.threshold = self.start;
        self.fine = self.start << 12;
        self.stalled = 0;
    }

    /// The machine's prediction for the next bit, stretched: its counts' estimate and its bit history's, weighted.
    #[inline]
    pub fn input(&self) -> i32 {
        let n = &self.nodes[self.at as usize];
        let (n0, n1) = (u64::from(n.count[0]) + 1, u64::from(n.count[1]) + 1);
        let by_counts = stretch(((n1 << 12) / (n0 + n1)).clamp(1, 4095) as u16);
        let history = n.history();
        let by_history = if history == 0 { 0 } else { stretch(self.maps[usize::from(history)].p()) };
        (by_counts * 5 + by_history * 3) / 8
    }

    /// Ask for both states the next bit can lead to.
    #[inline]
    pub fn prefetch(&self) {
        let n = &self.nodes[self.at as usize];
        prefetch(&self.nodes[n.next(0) as usize]);
        prefetch(&self.nodes[n.next(1) as usize]);
    }

    /// Learn `bit` here and move along it, cloning the target first if this transition has earned its own state.
    pub fn update(&mut self, bit: bool) {
        let b = usize::from(bit);
        let at = self.at as usize;
        let here = self.nodes[at];
        self.maps[usize::from(here.history())].update_with_limit(bit, 255);
        let decay = |c: u16| c - (c >> 6);
        let carried = u32::from(here.count[b]);
        let node = &mut self.nodes[at];
        node.count = [decay(here.count[0]) + if bit { 0 } else { ONE as u16 }, decay(here.count[1]) + if bit { ONE as u16 } else { 0 }];
        node.set_history(next_state(here.history(), bit));
        let mut next = here.next(b);
        if carried > self.threshold {
            let target = self.nodes[next as usize];
            let total = u32::from(target.count[0]) + u32::from(target.count[1]);
            if total > carried + self.threshold {
                if self.nodes.len() < self.capacity {
                    let share = |c: u16| (u64::from(c) * u64::from(carried) / u64::from(total)) as u16;
                    let moved = [share(target.count[0]), share(target.count[1])];
                    self.nodes[next as usize].count = [target.count[0] - moved[0], target.count[1] - moved[1]];
                    let clone = self.nodes.len() as u32;
                    self.nodes.push(Node::new([target.next(0), target.next(1)], moved, target.history()));
                    self.nodes[at].set_next(b, clone);
                    next = clone;
                    if self.threshold < THRESHOLD_MAX {
                        self.fine += 1;
                        self.threshold = self.fine >> 12;
                    }
                } else {
                    self.stalled += u64::from(total / ONE);
                }
            }
        }
        self.at = next;
    }

    /// Whether the machine has been full long enough that starting over would serve the data better.
    pub fn stale(&self) -> bool {
        self.stalled >> 7 > self.capacity as u64
    }
}

/// The machines: (cloning threshold in 1/512 counts, 1 / the share of the states it gets). The fast ones come in
/// pairs of a low and a sixteen-times higher threshold, then the two slow ones.
const MACHINES: [(u32, u32); 10] = [(1, 6), (16, 10), (2, 7), (32, 11), (4, 8), (64, 12), (8, 9), (128, 13), (512, 2), (768, 2)];

/// How many of the machines (the fast ones, first) start over when stale; the slow ones keep growing.
const RESETTING: usize = 8;

/// The machines of one model and the mixer inputs they give: each slow machine its own, the fast ones a pair's
/// average, so a pair whose members start over at different times stays steady.
pub struct Forest {
    machines: Vec<Dmc>,
}

impl Forest {
    /// Machines sized for an input of `len` bytes: sixteen states a byte to share out, at most 2²⁴.
    pub fn new(len: usize) -> Self {
        let states = (len as u64).saturating_mul(16).min(1 << 24);
        Self { machines: MACHINES.iter().map(|&(threshold, share)| Dmc::new(TREES + (states / u64::from(share)) as usize, threshold)).collect() }
    }

    /// The mixer inputs the forest gives.
    pub const INPUTS: usize = RESETTING / 2 + MACHINES.len() - RESETTING;

    /// Input `i` of [`Self::INPUTS`].
    #[inline]
    pub fn input(&self, i: usize) -> i32 {
        if i < RESETTING / 2 {
            (self.machines[2 * i].input() + self.machines[2 * i + 1].input()) / 2
        } else {
            self.machines[RESETTING + i - RESETTING / 2].input()
        }
    }

    pub fn update(&mut self, bit: bool) {
        for m in &mut self.machines {
            m.update(bit);
            m.prefetch();
        }
    }

    /// At a byte boundary (after `last`), start over the fast machines that have gone stale.
    pub fn end_of_byte(&mut self, last: u8) {
        for m in &mut self.machines[..RESETTING] {
            if m.stale() {
                m.reset(last);
            }
        }
    }
}
