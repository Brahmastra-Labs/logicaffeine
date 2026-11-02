//! The block-scale lane: a weight block's integer sums as the array and the activation side deliver them, turned into
//! the block's f32 value and added into its row, exactly as [`super::numerics::dot`] does under
//! [`ActScaling::PerBlock`](super::numerics::ActScaling::PerBlock).
//!
//! The array never sees a signed code. A weight code `q` (a decoder's unsigned code, one or two 4-bit digits) meets
//! an activation word in excess form: a one-word code `a` as `a + 128`, a two-word code's signed high byte as
//! `hi + 128` and its unsigned low byte as itself. Per 16-element unit the array delivers `P′ = Σ q·a′` for each word,
//! the weight image carries `Q = Σ q`, and the activation side carries `S′ = Σ a′`. The unit's sums against the signed
//! codes follow exactly:
//!
//! `Σ (q − z)·a = P′ − 128·Q − z·S′ + 128·16·z` and `Σ a = S′ − 128·16` for an excess word, `P′ − z·S′` and `S′`
//! for the low byte, the high word weighted by 256;
//!
//! the coefficient sum is `Σ_u c_u·Σ(q − z)·a` and the offset sum `Σ_u n_u·Σ a`. The lane multiplies them by the
//! block's scales times the activation scale, sums the two terms exactly, rounds once and adds into the row.

use super::numerics::{exact_sum, fadd, ActRow, ActScale, Scale, Words};
use super::rtl::weights::{Units, UNIT};

/// What the array and the activation side hand a lane for one unit of one word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitSums {
    /// `Σ q·a′` over the unit, the digits recombined.
    pub products: i64,
    /// `Σ a′` over the unit.
    pub act: i64,
}

/// One word's unit sums: whether the word crossed in excess form, and its weight in the code (1, or 256 for a
/// two-word code's high byte).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WordSums {
    pub units: Vec<UnitSums>,
    pub excess: bool,
    pub weight: i64,
}

/// One weight block as a lane reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneBlock {
    /// Per unit: the sub-block multiplier, the offset multiplier, and `Σ q` from the weight image.
    pub c: Vec<i64>,
    pub n: Vec<i64>,
    pub q: Vec<i64>,
    /// The format's code offset `z`.
    pub offset: i64,
    pub a: Scale,
    pub b: Scale,
    pub act: ActScale,
    pub words: Vec<WordSums>,
}

/// The block's two exact sums: `Σ coef·a` and `Σ offs·a`.
pub fn block_sums(blk: &LaneBlock) -> (i128, i128) {
    let unit = UNIT as i64;
    let (mut ia, mut ib) = (0i128, 0i128);
    for w in &blk.words {
        let excess = if w.excess { 128 } else { 0 };
        for (u, s) in w.units.iter().enumerate() {
            let products = s.products - excess * blk.q[u] - blk.offset * s.act + excess * unit * blk.offset;
            let act = s.act - excess * unit;
            ia += (w.weight * blk.c[u]) as i128 * products as i128;
            ib += (w.weight * blk.n[u]) as i128 * act as i128;
        }
    }
    (ia, ib)
}

/// The block's value rounded once and added into `acc`.
pub fn lane(blk: &LaneBlock, acc: f32) -> f32 {
    let (ia, ib) = block_sums(blk);
    let mut terms = [(0i128, 0i32); 2];
    let mut count = 0;
    for (s, sum) in [(blk.a, ia), (blk.b, ib)] {
        if s.sig != 0 && sum != 0 {
            terms[count] = (s.sig as i128 * blk.act.m as i128 * sum, s.exp + blk.act.e);
            count += 1;
        }
    }
    fadd(acc, exact_sum(&terms[..count]))
}

/// What the array, the weight image and the activation side deliver a lane for block `b` of a row: the row's decoded
/// units `units` (the block's own), its block scales `a`, `b_scale`, and the block's activations of `act` in `words`.
pub fn deliver(units: &Units, scales: (Scale, Scale), offset: u32, act: &ActRow, words: Words, b: usize) -> LaneBlock {
    let len = units.code.len();
    let start = b * len;
    let count = len / UNIT;
    let unit_sums = |a: &dyn Fn(usize) -> i64| -> Vec<UnitSums> {
        (0..count)
            .map(|u| {
                let range = u * UNIT..(u + 1) * UNIT;
                UnitSums {
                    products: range.clone().map(|i| units.code[i] as i64 * a(start + i)).sum(),
                    act: range.map(|i| a(start + i)).sum(),
                }
            })
            .collect()
    };
    let code = |i: usize| act.codes[i] as i64;
    let word_sums = match words {
        Words::One => vec![WordSums { units: unit_sums(&|i| code(i) + 128), excess: true, weight: 1 }],
        Words::Two => vec![
            WordSums { units: unit_sums(&|i| code(i).div_euclid(256) + 128), excess: true, weight: 256 },
            WordSums { units: unit_sums(&|i| code(i).rem_euclid(256)), excess: false, weight: 1 },
        ],
    };
    LaneBlock {
        c: units.c.iter().map(|&v| v as i64).collect(),
        n: units.n.iter().map(|&v| v as i64).collect(),
        q: (0..count).map(|u| units.code[u * UNIT..(u + 1) * UNIT].iter().map(|&q| q as i64).sum()).collect(),
        offset: offset as i64,
        a: scales.0,
        b: scales.1,
        act: act.scales[b],
        words: word_sums,
    }
}

// ── The per-32 lane ─────────────────────────────────────────────────────────────────────────────────

/// Elements a per-32 activation scale covers: a sub-block of the lane.
pub const SUB_BLOCK: usize = 32;

/// What the array, the weight image and the activation side deliver the per-32 lane for the 32-element sub-block `s`
/// of a row's weight block `b`: the block's decoded units `units` (the sub-block's two among them), the block's scales,
/// and the sub-block's activations of `act`, scaled per 32 elements, in `words`.
pub fn deliver32(units: &Units, scales: (Scale, Scale), offset: u32, act: &ActRow, words: Words, b: usize, s: usize) -> LaneBlock {
    let block = deliver(units, scales, offset, act, words, b);
    let first = s * SUB_BLOCK / UNIT;
    let count = SUB_BLOCK / UNIT;
    let start = b * units.code.len() + s * SUB_BLOCK;
    LaneBlock {
        c: block.c[first..first + count].to_vec(),
        n: block.n[first..first + count].to_vec(),
        q: block.q[first..first + count].to_vec(),
        offset: block.offset,
        a: block.a,
        b: block.b,
        act: act.scales[start / SUB_BLOCK],
        words: block.words.into_iter().map(|w| WordSums { units: w.units[first..first + count].to_vec(), ..w }).collect(),
    }
}

/// A group of the row — [`super::numerics::GROUP`] elements, its sub-blocks `subs` — rounded once and added into
/// `acc`: every sub-block's two exact sums times its block's scales and its own activation scale, the group's terms
/// summed exactly, as [`super::numerics::dot`] does under [`ActScaling::Sub32`](super::numerics::ActScaling::Sub32).
pub fn group(subs: &[LaneBlock], acc: f32) -> f32 {
    let mut terms = Vec::with_capacity(2 * subs.len());
    for blk in subs {
        let (ia, ib) = block_sums(blk);
        for (s, sum) in [(blk.a, ia), (blk.b, ib)] {
            if s.sig != 0 && sum != 0 {
                terms.push((s.sig as i128 * blk.act.m as i128 * sum, s.exp + blk.act.e));
            }
        }
    }
    fadd(acc, exact_sum(&terms))
}
