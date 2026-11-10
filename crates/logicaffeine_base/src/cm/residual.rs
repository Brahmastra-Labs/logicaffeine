//! Residual histograms: for each predictor of the next byte (a value the caller computes — the byte a period back, an
//! extrapolation), the distribution of `actual − predicted` (mod 256), kept per activity bucket the caller picks. A
//! byte's known high bits confine its residual to a range, and the counts on either side of the range's midpoint give
//! the next bit's probability. Unlike a context keyed on the predicted value, one distribution serves every value, so
//! "two above the prediction" learnt at one brightness applies at all of them.

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_base/cm/residual/tests.rs"]
mod tests;

use crate::entropy::stretch;

const BINS: usize = 256;

/// The prefix sums are kept with residual 0 at bin 192, so the common small residuals touch the array's tail.
const CENTER: usize = 192;

/// Histograms for `predictors` predictors, `buckets` activity buckets each.
pub struct Residuals {
    /// Per (predictor, bucket): prefix sums of residual counts, `sums[k] = count[0] + … + count[k]`.
    sums: Vec<u16>,
    buckets: usize,
    /// Per predictor, the prediction and the histogram it reads for the current byte.
    predicted: Vec<u8>,
    base: Vec<usize>,
}

impl Residuals {
    pub fn new(predictors: usize, buckets: usize) -> Self {
        Self { sums: vec![0; predictors * buckets * BINS], buckets, predicted: vec![0; predictors], base: vec![0; predictors] }
    }

    /// Predictor `i` predicts `value` for the next byte, in activity bucket `bucket`.
    #[inline]
    pub fn set(&mut self, i: usize, value: u8, bucket: usize) {
        self.predicted[i] = value;
        self.base[i] = (i * self.buckets + bucket.min(self.buckets - 1)) * BINS;
    }

    /// Residual counts in the circular bin range `[from, from + len)` of predictor `i`'s histogram.
    #[inline]
    fn count(&self, i: usize, from: usize, len: usize) -> u32 {
        let s = &self.sums[self.base[i]..self.base[i] + BINS];
        let at = |k: usize| if k == 0 { 0 } else { u32::from(s[k - 1]) };
        let end = from + len;
        if end <= BINS { at(end) - at(from) } else { u32::from(s[BINS - 1]) - at(from) + at(end - BINS) }
    }

    /// Predictor `i`'s stretched probability that the next bit is a one, the byte's bits so far being `c0` (with its
    /// leading one) at `depth` (0..=7) bits.
    #[inline]
    pub fn input(&self, i: usize, c0: u32, depth: u32) -> i32 {
        let known = (c0 << (8 - depth)) as usize & 0xFF;
        let half = 1usize << (7 - depth);
        let zero = (CENTER + known + BINS - usize::from(self.predicted[i])) % BINS;
        let n0 = self.count(i, zero, half);
        let n1 = self.count(i, (zero + half) % BINS, half);
        let p = ((u64::from(n1) * 2 + 1) * 4096 / (u64::from(n0 + n1) * 2 + 2)).clamp(1, 4095);
        stretch(p as u16)
    }

    /// Count `byte` against every predictor's prediction.
    pub fn update(&mut self, byte: u8) {
        for i in 0..self.predicted.len() {
            let bin = (CENTER + usize::from(byte) + BINS - usize::from(self.predicted[i])) % BINS;
            let s = &mut self.sums[self.base[i]..self.base[i] + BINS];
            for v in &mut s[bin..] {
                *v += 1;
            }
            if s[BINS - 1] == u16::MAX {
                for v in s.iter_mut() {
                    *v >>= 1;
                }
            }
        }
    }
}
