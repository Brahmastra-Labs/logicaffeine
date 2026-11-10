//! # Binary arithmetic coding and the adaptive models that drive it
//!
//! Every lossless compressor is a model plus a coder: the model says how likely the next bit is, the
//! coder spends `−log₂ p` bits on what actually happened. This module is the coder and the model
//! toolkit — a carry-less binary arithmetic coder, the logistic `squash`/`stretch` pair, adaptive
//! probability cells, a gated logistic mixer, and an adaptive probability map (secondary estimation).
//! The column models in [`crate::describe`] are built from these parts.
//!
//! Everything is integer arithmetic, so an encoder on one machine and a decoder on another (or in
//! wasm) make bit-identical predictions; a float anywhere on the prediction path would let the two
//! disagree in the last ulp and desynchronize the stream.
//!
//! Probabilities are 12-bit: `p12 = P(bit = 1) · 4096`, always in `1..=4095`.

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/entropy/tests.rs"]
mod tests;

const PROB_MIN: u16 = 1;
const PROB_MAX: u16 = 4095;

#[inline]
fn clamp_prob(p12: u16) -> u32 {
    u32::from(p12.clamp(PROB_MIN, PROB_MAX))
}

// ---- The coder -----------------------------------------------------------------------------

/// The binary arithmetic encoder. The live interval is `[x1, x2]`; each bit narrows it in proportion
/// to its probability, and leading bytes are shifted out as soon as both ends agree on them.
pub struct Encoder {
    x1: u32,
    x2: u32,
    out: Vec<u8>,
}

impl Default for Encoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Encoder {
    pub fn new() -> Self {
        Self { x1: 0, x2: u32::MAX, out: Vec::new() }
    }

    /// Code `bit` under the model's `p12 = P(bit = 1)`: [`Self::encode_fine`] at `p12 · 2²⁰`, the same bytes.
    #[inline]
    pub fn encode(&mut self, bit: bool, p12: u16) {
        self.encode_fine(bit, clamp_prob(p12) << 20);
    }

    /// Code `bit` under `P(bit = 1) = p32 / 2³²`.
    #[inline]
    pub fn encode_fine(&mut self, bit: bool, p32: u32) {
        let range = u64::from(self.x2 - self.x1);
        let xmid = self.x1 + ((range * u64::from(p32.max(1))) >> 32) as u32;
        if bit {
            self.x2 = xmid;
        } else {
            self.x1 = xmid + 1;
        }
        while (self.x1 ^ self.x2) & 0xFF00_0000 == 0 {
            self.out.push((self.x2 >> 24) as u8);
            self.x1 <<= 8;
            self.x2 = (self.x2 << 8) | 0xFF;
        }
    }

    /// Flush the one byte that pins a point inside the final interval: `x1`'s top byte plus one lies
    /// strictly above `x1` and, because the two ends differ in their top byte, no higher than `x2`.
    /// The decoder pads past the end with zero bytes, so that byte alone identifies the interval.
    pub fn finish(mut self) -> Vec<u8> {
        self.out.push(((self.x1 >> 24) + 1) as u8);
        self.out
    }

    /// Bytes emitted so far (the flush adds one more).
    pub fn len(&self) -> usize {
        self.out.len()
    }

    pub fn is_empty(&self) -> bool {
        self.out.is_empty()
    }
}

/// The binary arithmetic decoder — the encoder's mirror. Reading past the end yields zero bytes, so a
/// truncated or arbitrary stream decodes to *some* bits and never panics.
pub struct Decoder<'a> {
    x1: u32,
    x2: u32,
    x: u32,
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        let mut d = Self { x1: 0, x2: u32::MAX, x: 0, buf, pos: 0 };
        for _ in 0..4 {
            d.x = (d.x << 8) | u32::from(d.next_byte());
        }
        d
    }

    #[inline]
    fn next_byte(&mut self) -> u8 {
        let b = self.buf.get(self.pos).copied().unwrap_or(0);
        self.pos += 1;
        b
    }

    /// Decode one bit that was coded under `p12 = P(bit = 1)`.
    #[inline]
    pub fn decode(&mut self, p12: u16) -> bool {
        self.decode_fine(clamp_prob(p12) << 20)
    }

    /// Decode one bit that was coded under `P(bit = 1) = p32 / 2³²`.
    #[inline]
    pub fn decode_fine(&mut self, p32: u32) -> bool {
        let range = u64::from(self.x2 - self.x1);
        let xmid = self.x1 + ((range * u64::from(p32.max(1))) >> 32) as u32;
        let bit = self.x <= xmid;
        if bit {
            self.x2 = xmid;
        } else {
            self.x1 = xmid + 1;
        }
        while (self.x1 ^ self.x2) & 0xFF00_0000 == 0 {
            self.x1 <<= 8;
            self.x2 = (self.x2 << 8) | 0xFF;
            self.x = (self.x << 8) | u32::from(self.next_byte());
        }
        bit
    }
}

/// One routine drives both directions: on the encoder `code` writes `bit` and returns it; on the
/// decoder it ignores `bit` and returns what the stream says. A model written against `BitIo` is
/// therefore symmetric by construction — the encoder and decoder walk the same contexts in the same
/// order and update on the same bits.
pub trait BitIo {
    /// Code `bit` under `P(bit = 1) = p32 / 2³²`.
    fn code_fine(&mut self, bit: bool, p32: u32) -> bool;

    /// Code `bit` under the 12-bit `p12 = P(bit = 1)`.
    #[inline]
    fn code(&mut self, bit: bool, p12: u16) -> bool {
        self.code_fine(bit, clamp_prob(p12) << 20)
    }
}

impl BitIo for Encoder {
    #[inline]
    fn code_fine(&mut self, bit: bool, p32: u32) -> bool {
        self.encode_fine(bit, p32);
        bit
    }
}

impl BitIo for Decoder<'_> {
    #[inline]
    fn code_fine(&mut self, _bit: bool, p32: u32) -> bool {
        self.decode_fine(p32)
    }
}

// ---- The logistic domain -------------------------------------------------------------------

/// Knots of `4096 / (1 + e^(−x/256))` at `x = −2048, −1920, …, 2048`; `squash` interpolates between them.
const SQUASH_KNOTS: [i32; 33] = [
    1, 2, 3, 6, 10, 16, 27, 45, 73, 120, 194, 310, 488, 747, 1101, 1546, 2047, 2549, 2994, 3348, 3607, 3785, 3901,
    3975, 4022, 4050, 4068, 4079, 4085, 4089, 4092, 4093, 4094,
];

/// The logistic function on the fixed-point logit `d = 256·ln(p / (1 − p))`, returning a 12-bit
/// probability. Saturates outside `±2047`.
#[inline]
pub fn squash(d: i32) -> u16 {
    if d > 2047 {
        return PROB_MAX;
    }
    if d < -2047 {
        return PROB_MIN;
    }
    let w = d & 127;
    let i = ((d >> 7) + 16) as usize;
    (((SQUASH_KNOTS[i] * (128 - w) + SQUASH_KNOTS[i + 1] * w + 64) >> 7) as u16).clamp(PROB_MIN, PROB_MAX)
}

fn stretch_table() -> &'static [i16; 4096] {
    static TABLE: std::sync::OnceLock<[i16; 4096]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0i16; 4096];
        let mut next = 0usize;
        for x in -2047..=2047i32 {
            let v = squash(x) as usize;
            for slot in t.iter_mut().take(v + 1).skip(next) {
                *slot = x as i16;
            }
            next = next.max(v + 1);
        }
        for slot in t.iter_mut().skip(next) {
            *slot = 2047;
        }
        t
    })
}

/// The logit of a 12-bit probability — `squash`'s inverse, in the same fixed point.
#[inline]
pub fn stretch(p12: u16) -> i32 {
    i32::from(stretch_table()[usize::from(p12.min(4095))])
}

// ---- Adaptive probabilities ----------------------------------------------------------------

/// The adaptation-count cap a cell uses unless told otherwise: the rate settles at `1/31.5`, quick
/// enough to follow a drifting source.
pub const DEFAULT_LIMIT: u32 = 30;

/// One adaptive probability: a 22-bit `P(1)` and a hit count packed in a `u32`. The update rate is
/// `1 / (n + 1.5)` — the Krichevsky–Trofimov estimate while the count is small, settling at
/// `1 / (limit + 1.5)` so the cell keeps tracking a source whose statistics drift.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdaptiveProb(u32);

impl Default for AdaptiveProb {
    fn default() -> Self {
        Self::new()
    }
}

impl AdaptiveProb {
    pub const fn new() -> Self {
        Self(1 << 31)
    }

    /// The 12-bit `P(1)`.
    #[inline]
    pub fn p(self) -> u16 {
        ((self.0 >> 20) as u16).clamp(PROB_MIN, PROB_MAX)
    }

    /// How many updates the cell has seen (saturating at its limit) — its confidence.
    #[inline]
    pub fn count(self) -> u32 {
        self.0 & 1023
    }

    #[inline]
    pub fn update(&mut self, bit: bool) {
        self.update_with_limit(bit, DEFAULT_LIMIT);
    }

    #[inline]
    pub fn update_with_limit(&mut self, bit: bool, limit: u32) {
        let n = self.0 & 1023;
        let p = i64::from(self.0 >> 10);
        let target: i64 = if bit { (1 << 22) - 1 } else { 0 };
        let p = (p + (target - p) * 2 / (2 * i64::from(n) + 3)).clamp(0, (1 << 22) - 1) as u32;
        self.0 = (p << 10) | (n + 1).min(limit.min(1023));
    }
}

/// A table of adaptive probabilities addressed by a context hash (collisions are allowed; the table
/// is sized so they are rare).
pub struct HashedProbs {
    cells: Vec<AdaptiveProb>,
    mask: usize,
}

impl HashedProbs {
    /// A table of `2^bits` cells.
    pub fn new(bits: u32) -> Self {
        Self { cells: vec![AdaptiveProb::new(); 1 << bits], mask: (1 << bits) - 1 }
    }

    #[inline]
    pub fn cell(&mut self, h: u64) -> &mut AdaptiveProb {
        let i = (h >> 32) as usize ^ h as usize;
        &mut self.cells[i & self.mask]
    }
}

/// Adaptive probabilities in 64-byte buckets of 16: a model addresses one bucket per context per group of
/// decisions (a nibble of a tree walk) and indexes inside it by the decisions made so far, so a context
/// costs one cache line per group rather than one random access per decision.
pub struct ProbBuckets {
    cells: Vec<AdaptiveProb>,
    mask: usize,
}

impl ProbBuckets {
    /// A table of `2^bits` buckets.
    pub fn new(bits: u32) -> Self {
        Self { cells: vec![AdaptiveProb::new(); 16 << bits], mask: (1 << bits) - 1 }
    }

    /// The first cell of the bucket `h` addresses; cells `base + 1 ..= base + 15` are its slots.
    #[inline]
    pub fn bucket(&self, h: u64) -> usize {
        (((h >> 32) as usize ^ h as usize) & self.mask) << 4
    }

    #[inline]
    pub fn cell(&mut self, index: usize) -> &mut AdaptiveProb {
        &mut self.cells[index]
    }
}

// ---- Mixing --------------------------------------------------------------------------------

/// A gated logistic mixer: a weighted sum of input logits, one weight vector per selector context,
/// trained online by gradient descent on coding cost. Weights are fixed point with 16 fraction bits.
pub struct Mixer {
    n: usize,
    x: Vec<i32>,
    w: Vec<i32>,
    set: usize,
    pr: u16,
    lr: i32,
}

impl Mixer {
    /// `inputs` logits per prediction; `sets` independent weight vectors chosen by [`Mixer::mix`]'s
    /// context. Every weight starts at `1 / inputs`, so an untrained mixer averages its inputs.
    pub fn new(inputs: usize, sets: usize) -> Self {
        let init = (1 << 16) / inputs.max(1) as i32;
        Self { n: inputs, x: vec![0; inputs], w: vec![init; inputs * sets.max(1)], set: 0, pr: 2048, lr: 6 }
    }

    /// The learning-rate multiplier (`Δw = err · x · lr / 2^14`).
    pub fn with_learning_rate(mut self, lr: i32) -> Self {
        self.lr = lr;
        self
    }

    #[inline]
    pub fn set(&mut self, i: usize, logit: i32) {
        self.x[i] = logit.clamp(-2047, 2047);
    }

    /// Mix the current inputs under weight vector `ctx`, returning a 12-bit probability.
    #[inline]
    pub fn mix(&mut self, ctx: usize) -> u16 {
        self.set = ctx * self.n;
        let w = &self.w[self.set..self.set + self.n];
        let dot: i64 = self.x.iter().zip(w).map(|(&x, &w)| i64::from(x) * i64::from(w)).sum();
        self.pr = squash((dot >> 16).clamp(-2047, 2047) as i32);
        self.pr
    }

    /// Train the weight vector that made the last prediction toward the bit that occurred.
    #[inline]
    pub fn update(&mut self, bit: bool) {
        let err = (i32::from(bit) << 12) - i32::from(self.pr);
        let lr = self.lr;
        let w = &mut self.w[self.set..self.set + self.n];
        for (w, &x) in w.iter_mut().zip(&self.x) {
            *w = w.saturating_add(((i64::from(x) * i64::from(err) * i64::from(lr)) >> 14) as i32);
        }
    }
}

/// A two-layer mixer: several first-layer mixers over the same inputs, each choosing its weight vector
/// by its own selector context, and a final mixer over their logits. One selector rarely explains when an
/// input is reliable; several, combined by learned trust, do.
pub struct Mixer2 {
    /// Inputs, padded to a multiple of [`LANES`] so the dot products run whole vectors.
    n: usize,
    x: Vec<i16>,
    layers: Vec<Vec<i16>>,
    /// Per first-layer mixer, how often each weight set has been chosen (empty unless the sets settle).
    chosen_counts: Vec<Vec<u16>>,
    settle_half: u32,
    settle_floor: u32,
    chosen: Vec<usize>,
    hidden: Vec<i32>,
    hidden_pr: Vec<u16>,
    weights: Vec<i32>,
    final_set: usize,
    pr: u16,
    lr: i32,
    lr_final: i32,
}

/// The first layer's weights are 16-bit, `1.0 = 2^14` (so `±2.0` at most), and its inputs the 12-bit logits:
/// each product pair fits 32 bits, and the dot product is the sum of pair sums shifted down 8 — the shape SIMD
/// multiply-add instructions compute, so the hot loop vectorizes on every target and stays bit-exact.
const W_SHIFT: u32 = 14;
const LANES: usize = 16;
/// The first layer trains only on errors past this (of 4096).
const DEAD_ZONE: i32 = 56;

/// `Σ ((x₂ᵢ·w₂ᵢ + x₂ᵢ₊₁·w₂ᵢ₊₁) >> 8) >> (W_SHIFT − 8)` over slices of a multiple of [`LANES`] — SSE2's
/// multiply-add on x86-64, the same integer arithmetic elsewhere, so every target computes the same value.
#[inline]
fn dot16(x: &[i16], w: &[i16]) -> i32 {
    debug_assert!(x.len() == w.len() && x.len() % LANES == 0);
    #[cfg(target_arch = "x86_64")]
    {
        use core::arch::x86_64::*;
        // SAFETY: SSE2 is part of every x86-64; each load reads 16 bytes inside the slices (both have a length that is
        // a multiple of 16 elements).
        unsafe {
            let mut acc = _mm_setzero_si128();
            for i in (0..x.len()).step_by(8) {
                let a = _mm_loadu_si128(x.as_ptr().add(i) as *const __m128i);
                let b = _mm_loadu_si128(w.as_ptr().add(i) as *const __m128i);
                acc = _mm_add_epi32(acc, _mm_srai_epi32(_mm_madd_epi16(a, b), 8));
            }
            let mut lanes = [0i32; 4];
            _mm_storeu_si128(lanes.as_mut_ptr() as *mut __m128i, acc);
            lanes.iter().sum::<i32>() >> (W_SHIFT - 8)
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let mut sum = 0i32;
        for (xc, wc) in x.chunks_exact(2).zip(w.chunks_exact(2)) {
            sum += (i32::from(xc[0]) * i32::from(wc[0]) + i32::from(xc[1]) * i32::from(wc[1])) >> 8;
        }
        sum >> (W_SHIFT - 8)
    }
}

/// `w += round(x · err / 2^16)`, saturating at 16 bits; `err` must fit 16 bits. On x86-64 the rounded high half
/// is built from SSE2's high and low products — the same value the scalar form computes.
#[inline]
fn train16(w: &mut [i16], x: &[i16], err: i32) {
    debug_assert!(x.len() == w.len() && x.len() % LANES == 0 && (-32768..=32767).contains(&err));
    #[cfg(target_arch = "x86_64")]
    {
        use core::arch::x86_64::*;
        // SAFETY: SSE2 is part of every x86-64; each load and store touches 16 bytes inside the slices.
        unsafe {
            let e = _mm_set1_epi16(err as i16);
            for i in (0..x.len()).step_by(8) {
                let a = _mm_loadu_si128(x.as_ptr().add(i) as *const __m128i);
                let hi = _mm_mulhi_epi16(a, e);
                let carry = _mm_srli_epi16(_mm_mullo_epi16(a, e), 15);
                let p = w.as_mut_ptr().add(i) as *mut __m128i;
                _mm_storeu_si128(p, _mm_adds_epi16(_mm_loadu_si128(p), _mm_add_epi16(hi, carry)));
            }
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    for (w, &x) in w.iter_mut().zip(x) {
        let step = (i32::from(x) * err + (1 << 15)) >> 16;
        *w = (i32::from(*w) + step).clamp(-32768, 32767) as i16;
    }
}

impl Mixer2 {
    /// `inputs` logits; one first-layer mixer per entry of `sets` (that many weight vectors each), and a
    /// final mixer with `final_sets` weight vectors over their outputs plus a bias.
    pub fn new(inputs: usize, sets: &[usize], final_sets: usize) -> Self {
        let k = sets.len();
        let n = inputs.max(1).div_ceil(LANES) * LANES;
        let init = ((1 << W_SHIFT) / inputs.max(1)) as i16;
        let row: Vec<i16> = (0..n).map(|i| if i < inputs { init } else { 0 }).collect();
        let mut final_init = vec![(1 << 16) / k.max(1) as i32; k];
        final_init.push(0);
        Self {
            n,
            x: vec![0; n],
            layers: sets.iter().map(|&m| row.iter().copied().cycle().take(n * m.max(1)).collect()).collect(),
            chosen_counts: Vec::new(),
            settle_half: 0,
            settle_floor: 16,
            chosen: vec![0; k],
            hidden: vec![0; k + 1],
            hidden_pr: vec![2048; k],
            weights: final_init.iter().copied().cycle().take((k + 1) * final_sets.max(1)).collect(),
            final_set: 0,
            pr: 2048,
            lr: 4,
            lr_final: 2,
        }
    }

    /// Learning-rate multipliers for the first layer (`Δw = err · x · lr / 2^16`, weights `1.0 = 2^14`) and the
    /// final mixer (`Δw = err · x · lr / 2^14`, weights `1.0 = 2^16`).
    pub fn with_learning_rates(mut self, lr: i32, lr_final: i32) -> Self {
        self.lr = lr;
        self.lr_final = lr_final;
        self
    }

    /// First-layer weight sets that settle: a set chosen `n` times steps at `(floor + (16 − floor)·half / (half + n))`
    /// sixteenths of the learning rate — the full rate while it is young, falling toward `floor` sixteenths as it is
    /// chosen again and again, so a busy set stops chasing noise and a rare one still learns.
    pub fn with_settling(mut self, half: u32, floor: u32) -> Self {
        let n = self.n;
        self.chosen_counts = self.layers.iter().map(|w| vec![0; w.len() / n]).collect();
        self.settle_half = half;
        self.settle_floor = floor;
        self
    }

    #[inline]
    pub fn set(&mut self, i: usize, logit: i32) {
        self.x[i] = logit.clamp(-2047, 2047) as i16;
    }

    /// The logit at input `i`, as last set.
    pub fn input(&self, i: usize) -> i32 {
        i32::from(self.x[i])
    }

    /// Mix under one selector per first-layer mixer and `final_ctx` for the final mixer.
    #[inline]
    pub fn mix(&mut self, selectors: &[usize], final_ctx: usize) -> u16 {
        for (l, &sel) in selectors.iter().enumerate() {
            let off = sel * self.n;
            let logit = dot16(&self.x, &self.layers[l][off..off + self.n]).clamp(-2047, 2047);
            self.chosen[l] = off;
            self.hidden[l] = logit;
            self.hidden_pr[l] = squash(logit);
        }
        let k = self.chosen.len();
        self.hidden[k] = 256;
        self.final_set = final_ctx * (k + 1);
        let w = &self.weights[self.final_set..self.final_set + k + 1];
        let dot: i64 = self.hidden.iter().zip(w).map(|(&x, &w)| i64::from(x) * i64::from(w)).sum();
        self.pr = squash((dot >> 16).clamp(-2047, 2047) as i32);
        self.pr
    }

    /// Train every first-layer mixer on its own prediction's error and the final mixer on the combined one.
    #[inline]
    pub fn update(&mut self, bit: bool) {
        let target = i32::from(bit) << 12;
        let n = self.n;
        for l in 0..self.chosen.len() {
            let miss = target - i32::from(self.hidden_pr[l]);
            let off = self.chosen[l];
            let sixteenths = match self.chosen_counts.get_mut(l) {
                Some(counts) => {
                    let seen = &mut counts[off / n];
                    let (half, floor) = (self.settle_half, self.settle_floor);
                    let rate = floor + (16 - floor) * half / (half + u32::from(*seen));
                    *seen = seen.saturating_add(1);
                    rate as i32
                }
                None => 16,
            };
            // A weight set already this close is left alone: the step would be noise, and skipping it is the
            // common case on predictable data.
            if miss.abs() <= DEAD_ZONE {
                continue;
            }
            let err = (miss * self.lr * sixteenths / 16).clamp(-32768, 32767);
            train16(&mut self.layers[l][off..off + n], &self.x, err);
        }
        let err = i64::from(target - i32::from(self.pr)) * i64::from(self.lr_final);
        let k = self.chosen.len();
        for (w, &x) in self.weights[self.final_set..self.final_set + k + 1].iter_mut().zip(&self.hidden) {
            *w = w.saturating_add(((i64::from(x) * err) >> 14) as i32);
        }
    }
}

// ---- Secondary estimation ------------------------------------------------------------------

/// Past this a calibration cell's counts both halve, so `2n + 1` shifted 32 places still fits 64 bits.
const COUNT_CAP: u32 = 1 << 30;

/// The last word on a probability: for each context and each 12-bit probability, the zeros and ones that followed
/// it, read back as a 32-bit `P(1)`. A prediction that comes true far more often than 4095 times in 4096 is coded
/// at what it is worth, which no 12-bit probability can say, and one that is systematically off is put right.
/// Each cell starts as the identity, as sure of it as 2,048 bits would make it.
pub struct Calibration {
    counts: Vec<(u32, u32)>,
    at: usize,
}

impl Calibration {
    pub fn new(contexts: usize) -> Self {
        let row = (0..4096u32).map(|j| ((4096 - j) >> 1, j >> 1));
        Self { counts: row.cycle().take(contexts.max(1) * 4096).collect(), at: 0 }
    }

    /// The calibrated `P(1)`, a 32-bit fraction, of `p12` in context `ctx`.
    #[inline]
    pub fn p(&mut self, p12: u16, ctx: usize) -> u32 {
        self.at = ctx * 4096 + usize::from(p12.min(4095));
        let (n0, n1) = self.counts[self.at];
        let (n0, n1) = (2 * u64::from(n0) + 1, 2 * u64::from(n1) + 1);
        ((n1 << 32) / (n0 + n1)).clamp(1, u64::from(u32::MAX)) as u32
    }

    /// Count `bit` in the cell the last [`Self::p`] read.
    #[inline]
    pub fn update(&mut self, bit: bool) {
        let cell = &mut self.counts[self.at];
        if bit {
            cell.1 += 1;
        } else {
            cell.0 += 1;
        }
        if cell.0.max(cell.1) > COUNT_CAP {
            cell.0 >>= 1;
            cell.1 >>= 1;
        }
    }
}

/// An adaptive probability map: refines a probability under a context by interpolating a learned
/// curve over 33 knots in the logit domain. It corrects whatever systematic miscalibration the
/// primary model has in that context. Its knots move at a fixed rate, or ([`Apm::with_counts`]) each
/// by the reciprocal of how often it has been the nearer knot.
pub struct Apm {
    t: Vec<i32>,
    lo: usize,
    w: i32,
    rate: u32,
    /// Per knot, its update count (empty at a fixed rate).
    n: Vec<u16>,
    limit: u16,
}

impl Apm {
    /// One learned curve per context, each initialized to the identity.
    pub fn new(contexts: usize) -> Self {
        let identity: Vec<i32> = (0..33).map(|j| i32::from(squash((j - 16) * 128)) * 16).collect();
        let t = identity.iter().copied().cycle().take(contexts.max(1) * 33).collect();
        Self { t, lo: 0, w: 0, rate: 7, n: Vec::new(), limit: 0 }
    }

    /// Knots that adapt by count: each update moves a knot `1/(n + 1.5)` of its interpolation share toward the
    /// bit, where `n` starts at `start` and grows by one each time the knot is the nearer of the two, up to
    /// `limit` — quick while a curve is young, settled on its average once it is old.
    pub fn with_counts(mut self, start: u16, limit: u16) -> Self {
        self.n = vec![start; self.t.len()];
        self.limit = limit;
        self
    }

    /// The map's adaptation rate (`2^-rate` per update).
    pub fn with_rate(mut self, rate: u32) -> Self {
        self.rate = rate;
        self
    }

    #[inline]
    pub fn refine(&mut self, p12: u16, ctx: usize) -> u16 {
        let s = stretch(p12) + 2048;
        self.lo = ctx * 33 + (s >> 7) as usize;
        self.w = s & 127;
        let p = (self.t[self.lo] * (128 - self.w) + self.t[self.lo + 1] * self.w) >> 11;
        (p as u16).clamp(PROB_MIN, PROB_MAX)
    }

    /// Move both knots around the last refined point toward the bit, each by its interpolation share.
    #[inline]
    pub fn update(&mut self, bit: bool) {
        let target = if bit { 65535 } else { 0 };
        if !self.n.is_empty() {
            for (k, share) in [(self.lo, 128 - self.w), (self.lo + 1, self.w)] {
                let n = i64::from(self.n[k]);
                let v = &mut self.t[k];
                *v += (i64::from(target - *v) * i64::from(share) * 2 / ((2 * n + 3) * 128)) as i32;
            }
            let near = if self.w < 64 { self.lo } else { self.lo + 1 };
            if self.n[near] < self.limit {
                self.n[near] += 1;
            }
            return;
        }
        let shift = self.rate + 7;
        let a = &mut self.t[self.lo];
        *a += ((target - *a) * (128 - self.w)) >> shift;
        let b = &mut self.t[self.lo + 1];
        *b += ((target - *b) * self.w) >> shift;
    }
}

// ---- Coding cost ---------------------------------------------------------------------------

/// `log₂ x` in 1/256-bit units for `x ≥ 1`, by integer arithmetic only (normalize, then square the
/// mantissa eight times, one fraction bit per squaring) — so every machine computes the same value.
fn log2_fixed(x: u32) -> u32 {
    let int = 31 - x.leading_zeros();
    // Mantissa in [1, 2) as a 1.31 fixed-point number.
    let mut m = u64::from(x) << (31 - int);
    let mut frac = 0u32;
    for bit in (0..8).rev() {
        m = (m * m) >> 31;
        if m >= 1 << 32 {
            m >>= 1;
            frac |= 1 << bit;
        }
    }
    (int << 8) | frac
}

/// What coding `bit` at the 12-bit `P(1) = p12` costs, in 1/256-bit units: `−log₂ P(bit)`. Integer-only, so
/// an encoder and a decoder that both track costs (to pick between two ways of coding) agree exactly.
#[inline]
pub fn cost(bit: bool, p12: u16) -> u32 {
    let p = u32::from(p12.clamp(PROB_MIN, PROB_MAX));
    let q = if bit { p } else { 4096 - p };
    (12 << 8) - log2_fixed(q)
}

// ---- Memory --------------------------------------------------------------------------------

/// A hint that `item`'s cache line is about to be read. It reads nothing and changes no result; on targets
/// without a prefetch instruction it is nothing at all.
#[inline(always)]
pub fn prefetch<T>(item: &T) {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: a prefetch is a hint to the cache: it performs no architectural load and cannot fault, and the
    // pointer comes from a live reference.
    unsafe {
        core::arch::x86_64::_mm_prefetch(item as *const T as *const i8, core::arch::x86_64::_MM_HINT_T0)
    };
}

// ---- Hashing -------------------------------------------------------------------------------

/// A fixed (seedless) 64-bit mix, so hashed contexts land in the same cells on every run and machine.
#[inline]
pub fn hash1(a: u64) -> u64 {
    let mut z = a.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// An order-sensitive hash of two words.
#[inline]
pub fn hash2(a: u64, b: u64) -> u64 {
    hash1(hash1(a) ^ b.rotate_left(17).wrapping_mul(0xD6E8_FEB8_6659_FD93))
}

/// An order-sensitive hash of three words.
#[inline]
pub fn hash3(a: u64, b: u64, c: u64) -> u64 {
    hash2(hash2(a, b), c)
}
