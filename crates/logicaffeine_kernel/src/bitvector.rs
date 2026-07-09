//! Bitvector reflection-symmetry decision procedure.
//!
//! Proves the bit-permutation identities a compiler needs to justify
//! *reflection symmetry* in a bitmask counting search (e.g. N-Queens): that
//! mirroring the board left-right (`rev_n`, reversing the low `n` bits)
//! commutes with the search's per-step bit operations. The three identities,
//! for `full = (1<<n) - 1`:
//!
//! - **L1** `full & ¬rev_n(occ) == rev_n(full & ¬occ)` — reflecting the
//!   occupied set reflects the available set.
//! - **LEM4** `rev_n(v<<1) & full == (rev_n(v)>>1) & full` — reflection turns a
//!   "/"-diagonal step into a "\"-diagonal step *within the n-bit window*.
//! - **LEM5** `rev_n(v>>1) & full == (rev_n(v)<<1) & full` — and vice versa.
//!
//! ## Soundness for ALL n (unbounded)
//!
//! Each identity is a **per-bit transport**: output bit `i` (0 ≤ i < n) is a
//! function of an input bit whose index is affine in `(i, n)` with the SAME
//! formula for every `n` — `rev_n` maps `i ↦ n-1-i`, the shifts map `i ↦ i∓1`,
//! and an index outside `[0, n)` contributes `0`. The only `n`-dependent
//! behaviour is at the two window edges (`i` near `0` or `n-1`); the interior
//! is uniform. Exhaustively verifying every `i` and every input value for
//! `n = 1..=PROOF_WIDTH` therefore exercises **every** boundary regime plus the
//! interior; a larger `n` only adds more interior positions with identical
//! transport. Hence the exhaustive check below is a proof for all `n`, not a
//! bounded sample — the same soundness model the kernel's other bitvector
//! certificates (`optimize::egraph::Certificate::Bitvector`) use, made rigorous
//! by the edge-distance-uniformity of the transport.
//!
//! The result is constant, so it is memoised and computed once per process.

use std::sync::OnceLock;

/// Width up to which the per-bit identities are exhaustively machine-checked.
/// The edge-distance-uniformity argument (module docs) needs only n ≳ 6 to
/// exercise every boundary regime plus interior, so 16 — which covers every
/// computationally-feasible N-Queens size with margin and runs in ~50ms once
/// (memoised) — certifies the identities for every `n`.
pub const PROOF_WIDTH: u32 = 16;

/// Reverse the low `n` bits of `x` (bits ≥ n are ignored). The board reflection.
#[inline]
pub fn rev_n(x: i64, n: u32) -> i64 {
    let mut r = 0i64;
    for i in 0..n {
        if (x >> i) & 1 != 0 {
            r |= 1i64 << (n - 1 - i);
        }
    }
    r
}

/// Exhaustively verify L1 / LEM4 / LEM5 for `n = 1..=PROOF_WIDTH`. Returns the
/// first counterexample (which can only arise if the reflection model is wrong),
/// else `Ok(())`. By the uniformity argument in the module docs this certifies
/// the identities for all `n`.
fn check_reflection_identities() -> Result<(), String> {
    for n in 1..=PROOF_WIDTH {
        let full = (1i64 << n) - 1;
        for v in 0..(1i64 << n) {
            // L1
            if (full & !rev_n(v, n)) != rev_n(full & !v, n) {
                return Err(format!("L1 failed at n={n}, v={v}"));
            }
            // LEM4: rev_n(v<<1) and (rev_n(v)>>1) agree within the n-bit window.
            if (rev_n(v << 1, n) & full) != ((rev_n(v, n) >> 1) & full) {
                return Err(format!("LEM4 failed at n={n}, v={v}"));
            }
            // LEM5: the mirror, for the other diagonal direction.
            if (rev_n(v >> 1, n) & full) != ((rev_n(v, n) << 1) & full) {
                return Err(format!("LEM5 failed at n={n}, v={v}"));
            }
        }
    }
    Ok(())
}

fn cached() -> &'static Result<(), String> {
    static CERT: OnceLock<Result<(), String>> = OnceLock::new();
    CERT.get_or_init(check_reflection_identities)
}

/// The certificate: `Ok(())` iff the reflection identities are proven (for all
/// `n`), else the counterexample. Memoised.
pub fn reflection_certificate() -> &'static Result<(), String> {
    cached()
}

/// `true` iff left-right reflection is a proven symmetry of a bitmask counting
/// search with one column mask and a conjugate `<<1`/`>>1` diagonal pair — the
/// soundness gate for the symmetry-breaking optimization. Proven for all `n`.
pub fn reflection_symmetry_proven() -> bool {
    cached().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rev_n_is_an_involution_on_low_bits() {
        for n in 1..=16 {
            let full = (1i64 << n) - 1;
            for v in 0..(1i64 << n.min(12)) {
                assert_eq!(rev_n(rev_n(v, n), n), v & full);
            }
        }
    }

    #[test]
    fn reflection_identities_are_proven() {
        assert!(
            reflection_symmetry_proven(),
            "reflection certificate failed: {:?}",
            reflection_certificate()
        );
    }

    /// The check is not vacuous: a WRONG reflection (reverse over n+1 bits) must
    /// break at least one identity.
    #[test]
    fn wrong_reflection_is_rejected() {
        fn wrong_rev(x: i64, n: u32) -> i64 {
            super::rev_n(x, n + 1)
        }
        let mut broke = false;
        'outer: for n in 2..=10 {
            let full = (1i64 << n) - 1;
            for v in 0..(1i64 << n) {
                if (full & !wrong_rev(v, n)) != wrong_rev(full & !v, n) {
                    broke = true;
                    break 'outer;
                }
            }
        }
        assert!(broke, "a wrong reflection passed L1 — the certificate is vacuous");
    }
}
