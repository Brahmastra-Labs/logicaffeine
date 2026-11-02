//! The row ids of Qwen3.8-Flash-Next's hashed n-gram embedding (`qwen4_exp`'s per-layer embedding), as transformers'
//! `Qwen4ExpTextNGramEmbedding` computes them.
//!
//! For each position and each n-gram order n from 2 to `ngram_size`, the token and the n − 1 before it are each
//! multiplied by the layer's multiplier for their place and the products exclusive-ored; the result, reduced by each of
//! the order's `heads_per_ngram` heads' primes and offset into that head's block, names one table row per head. A
//! token before the start of its segment (the end-of-sequence token ends a segment) reads as the end-of-sequence
//! token. The multipliers are odd, drawn by splitmix64 from the layer's seed, and small enough that every product of a
//! token and a multiplier fits in a signed 64-bit integer; the heads' primes are the consecutive primes after
//! `ngram_vocab_size_base`, and the table is padded to a multiple of `make_ngram_vocab_size_divisible_by` rows.

use serde_json::Value;

const MASK64: u128 = (1u128 << 64) - 1;
const SPLITMIX_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
const SPLITMIX_M1: u64 = 0xBF58_476D_1CE4_E5B9;
const SPLITMIX_M2: u64 = 0x94D0_49BB_1331_11EB;
const PRIME_1: u64 = 10007;

fn splitmix64(value: u64) -> u64 {
    let mut v = value.wrapping_add(SPLITMIX_GAMMA);
    v = (v ^ (v >> 30)).wrapping_mul(SPLITMIX_M1);
    v = (v ^ (v >> 27)).wrapping_mul(SPLITMIX_M2);
    v ^ (v >> 31)
}

fn is_prime(n: u64) -> bool {
    if n < 2 {
        return false;
    }
    if n % 2 == 0 {
        return n == 2;
    }
    let mut d = 3;
    while d * d <= n {
        if n % d == 0 {
            return false;
        }
        d += 2;
    }
    true
}

/// The `count`-th prime after `start`.
fn nth_prime_after(start: u64, count: u64) -> u64 {
    let mut p = start;
    for _ in 0..count {
        p += 1;
        while !is_prime(p) {
            p += 1;
        }
    }
    p
}

/// One layer's n-gram hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NgramHash {
    ngram_size: usize,
    heads_per_ngram: usize,
    eos: u32,
    multipliers: Vec<u64>,
    primes: Vec<u64>,
    offsets: Vec<u64>,
    rows_before_padding: u64,
    rows: u64,
}

impl NgramHash {
    /// The hash of the per-layer embedding at place `ple_layer_index` among the layers that carry one.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        vocab_size: u64,
        ngram_size: usize,
        heads_per_ngram: usize,
        ngram_vocab_size_base: u64,
        divisible_by: u64,
        ple_layer_index: u64,
        seed: u64,
        eos: u32,
    ) -> NgramHash {
        let max_long = (1u64 << 63) - 1;
        let half_bound = (max_long / vocab_size.max(1) / 2).max(1);
        let base_seed = seed.wrapping_add(PRIME_1.wrapping_mul(ple_layer_index));
        let multipliers = (0..ngram_size as u64)
            .map(|i| {
                let value = ((base_seed as u128 + SPLITMIX_GAMMA as u128 * (i as u128 + 1)) & MASK64) as u64;
                2 * (splitmix64(value) % half_bound) + 1
            })
            .collect();
        let heads = (ngram_size - 1) * heads_per_ngram;
        let mut primes = Vec::with_capacity(heads);
        let mut offsets = Vec::with_capacity(heads);
        let mut total = 0u64;
        for h in 0..heads as u64 {
            let global = ple_layer_index * heads as u64 + h;
            let p = nth_prime_after(ngram_vocab_size_base - 1, global + 1);
            primes.push(p);
            offsets.push(total);
            total += p;
        }
        let rows = total.div_ceil(divisible_by) * divisible_by;
        NgramHash { ngram_size, heads_per_ngram, eos, multipliers, primes, offsets, rows_before_padding: total, rows }
    }

    /// The hash a checkpoint's text configuration describes (its `seed` defaulting to 1234 and its end-of-sequence
    /// token to the first listed, as the configuration class has them).
    pub fn from_config(config: &Value, ple_layer_index: u64) -> Option<NgramHash> {
        let u = |k: &str| config.get(k).and_then(Value::as_u64);
        let eos = match config.get("eos_token_id")? {
            Value::Array(a) => a.first()?.as_u64()?,
            v => v.as_u64()?,
        };
        Some(NgramHash::new(
            u("vocab_size")?,
            u("ngram_size")? as usize,
            u("heads_per_ngram")? as usize,
            u("ngram_vocab_size_base")?,
            u("make_ngram_vocab_size_divisible_by")?,
            ple_layer_index,
            u("seed").unwrap_or(1234),
            eos as u32,
        ))
    }

    pub fn multipliers(&self) -> &[u64] {
        &self.multipliers
    }

    pub fn head_primes(&self) -> &[u64] {
        &self.primes
    }

    /// Where each head's block starts in the table.
    pub fn head_offsets(&self) -> &[u64] {
        &self.offsets
    }

    pub fn rows_before_padding(&self) -> u64 {
        self.rows_before_padding
    }

    /// The table's rows, padded.
    pub fn rows(&self) -> u64 {
        self.rows
    }

    /// The row ids of each of `tokens`, one per head, after the `context` the previous call left (its last
    /// `ngram_size − 1` tokens; empty on the first call, which reads as end-of-sequence tokens).
    pub fn ids(&self, context: &[u32], tokens: &[u32]) -> Vec<Vec<u64>> {
        let keep = self.ngram_size - 1;
        let mut history: Vec<u32> = if context.is_empty() { vec![self.eos; keep] } else { context.to_vec() };
        history.extend_from_slice(tokens);
        // Where each position's segment starts: just after the last end-of-sequence token before it.
        let mut segment_start = Vec::with_capacity(history.len());
        let mut start = 0usize;
        for (p, &t) in history.iter().enumerate() {
            segment_start.push(start);
            if t == self.eos {
                start = p + 1;
            }
        }
        let shifted = |p: usize, s: usize| -> u64 {
            if p >= s && p - segment_start[p] >= s {
                u64::from(history[p - s])
            } else {
                u64::from(self.eos)
            }
        };
        let first = history.len() - tokens.len();
        (first..history.len())
            .map(|p| {
                let mut row = Vec::with_capacity(self.primes.len());
                for n in 2..=self.ngram_size {
                    let mut mixed = shifted(p, 0).wrapping_mul(self.multipliers[0]);
                    for place in 1..n {
                        mixed ^= shifted(p, place).wrapping_mul(self.multipliers[place]);
                    }
                    for h in 0..self.heads_per_ngram {
                        let head = (n - 2) * self.heads_per_ngram + h;
                        row.push(mixed % self.primes[head] + self.offsets[head]);
                    }
                }
                row
            })
            .collect()
    }
}
