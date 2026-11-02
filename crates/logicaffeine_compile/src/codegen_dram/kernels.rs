//! DRAM application kernels — the workloads in-memory compute is lightning-fast at: bitwise + COUNT
//! over massive data, at zero data movement. Popcount is the linchpin — the reduction that turns the
//! bitwise/XNOR machinery into counts, distances, and dot products:
//!
//!   • [`popcount`] — per-lane population count, folded bit-serially into an in-DRAM counter;
//!   • [`hamming_distance`] = popcount(a XOR b) — vector similarity search (ANN over binary codes),
//!     binary-net inference, bioinformatics distance;
//!   • [`agreement_count`] = popcount(a XNOR b) — the 1-bit / binary neural-net DOT PRODUCT (for ±1
//!     codes, dot = 2·agreements − K). This is how a BitNet-style matmul runs with NO multiply.
//!
//! All are composed from the `{And, Or, Not}` basis over a bit-serial (bitplane) layout: bitplane j
//! is a row whose bit `i` is bit `j` of lane `i`'s value, so one kernel processes all lanes in
//! parallel — exactly the data-parallel, bandwidth-bound shape the DRAM plane wins on.

use super::dataflow::{bit_serial_sub, DramDataflow};

/// Add a single-bit plane `bit` (0/1 per lane) into a per-lane counter `counter` (W planes, LSB
/// first) — a half-adder ripple. Overflow beyond W is dropped, so size W ≥ ⌈log2(max_count+1)⌉.
fn increment_by_bit(df: &mut DramDataflow, counter: &[usize], bit: usize) -> Vec<usize> {
    let mut carry = bit;
    let mut out = Vec::with_capacity(counter.len());
    for &c in counter {
        out.push(df.xor(c, carry)); // sum bit
        carry = df.and(c, carry); // carry out
    }
    out
}

/// Per-lane POPCOUNT of `planes`: for each lane, the number of set bits across the planes, as an
/// `out_width`-plane per-lane integer (LSB first). A reduction realized ENTIRELY in the DRAM plane by
/// folding each plane into a bit-serial counter. `zero` is a zero row (the initial counter value).
pub fn popcount(
    df: &mut DramDataflow,
    planes: &[usize],
    out_width: usize,
    zero: usize,
) -> Vec<usize> {
    let mut counter: Vec<usize> = vec![zero; out_width];
    for &p in planes {
        counter = increment_by_bit(df, &counter, p);
    }
    counter
}

/// Per-lane HAMMING DISTANCE between two bit-serial codes `a`/`b` (K planes each): popcount(a XOR b).
/// The core kernel of vector similarity search (ANN over binary codes) and binary-net inference.
pub fn hamming_distance(
    df: &mut DramDataflow,
    a: &[usize],
    b: &[usize],
    out_width: usize,
    zero: usize,
) -> Vec<usize> {
    let xored: Vec<usize> = a.iter().zip(b).map(|(&ai, &bi)| df.xor(ai, bi)).collect();
    popcount(df, &xored, out_width, zero)
}

/// Per-lane AGREEMENT COUNT popcount(a XNOR b) — the binary / 1-bit neural-net DOT PRODUCT. For ±1
/// codes packed as bits, the true dot product is `2·agreements − K`; the agreement count is the
/// in-DRAM half (XNOR + popcount, no multiply), the final affine map is a scalar host step.
pub fn agreement_count(
    df: &mut DramDataflow,
    a: &[usize],
    b: &[usize],
    out_width: usize,
    zero: usize,
) -> Vec<usize> {
    let xnored: Vec<usize> = a.iter().zip(b).map(|(&ai, &bi)| df.xnor(ai, bi)).collect();
    popcount(df, &xnored, out_width, zero)
}

/// BitNet-style TERNARY·BINARY dot product — one matmul row, entirely in-DRAM, NO multiply. Ternary
/// weights `w[k]` ∈ {−1,0,+1} are carried as two disjoint bitplane sets: `wp[k]`=1 iff w=+1, `wn[k]`=1
/// iff w=−1. Binary activations `x[k]` ∈ {0,1}. Then, per lane (per output neuron),
///   `dot = Σ w[k]·x[k] = popcount(Wp AND X) − popcount(Wn AND X)`
/// as a two's-complement `out_width`-plane integer. `zero` is a zero row, `ones` an all-ones carry
/// row (for the subtract). This is how a 1.58-bit LLM layer runs where the weights live: AND +
/// popcount + subtract, no multiplier.
pub fn ternary_binary_dot(
    df: &mut DramDataflow,
    wp: &[usize],
    wn: &[usize],
    x: &[usize],
    out_width: usize,
    zero: usize,
    ones: usize,
) -> Vec<usize> {
    let wpx: Vec<usize> = wp.iter().zip(x).map(|(&w, &xi)| df.and(w, xi)).collect();
    let wnx: Vec<usize> = wn.iter().zip(x).map(|(&w, &xi)| df.and(w, xi)).collect();
    let pos = popcount(df, &wpx, out_width, zero);
    let neg = popcount(df, &wnx, out_width, zero);
    bit_serial_sub(df, &pos, &neg, ones)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_dram/kernels/tests.rs"]
mod tests;
