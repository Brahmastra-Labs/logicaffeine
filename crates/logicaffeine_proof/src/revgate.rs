//! Certify reversible logic gates as bijections.
//!
//! The Silicon tier maps each VM stencil family to a functional unit,
//! and the adiabatic datapath demands those units be reversible. A unit over `n` bits is reversible
//! iff its truth table is a permutation of `{0..2ⁿ}` — a bijection whose inverse is the inverse
//! permutation, i.e. a real constructive reverse gate. A purely-linear (CNOT/XOR) block is reversible
//! iff its GF(2) transfer matrix is invertible (`∈ GL(n,2)`) — reused from [`crate::gf2`].
//!
//! This is the hardware-gate complement to the software-level Z3 bijectivity proofs in
//! `logicaffeine_synth::reversible`: there the abelian `Rev*` opcodes are proved bijective as BV64
//! maps; here the universal reversible generators they compile to — CNOT (the `BitXor→CNOT`
//! embedding), Toffoli (the `BitAnd→Toffoli` embedding), and Fredkin — are certified as gates.

use crate::gf2::is_invertible_gf2;

/// A reversibility certificate for a functional unit.
#[derive(Debug, PartialEq, Eq)]
pub enum GateCert {
    /// Reversible: `inverse[output] = input`, the constructive reverse gate (itself a truth table).
    Reversible { inverse: Vec<usize> },
    /// Irreversible: two distinct inputs collide to the same output — information is erased (the
    /// Landauer cost the adiabatic tier forbids). Carries the witnessing collision.
    Collision {
        input_a: usize,
        input_b: usize,
        output: usize,
    },
}

/// Certify an `n`-bit functional unit from its truth table (`table[input] = output`, each a state
/// index `0..2ⁿ`). Reversible iff the table is a permutation; returns the inverse gate, or the first
/// erasing collision that proves it is not.
pub fn certify_gate(n: u32, table: &[usize]) -> GateCert {
    let size = 1usize << n;
    assert_eq!(
        table.len(),
        size,
        "an n-bit truth table must have exactly 2^n rows"
    );
    let mut inverse = vec![usize::MAX; size];
    for (input, &output) in table.iter().enumerate() {
        assert!(
            output < size,
            "truth-table output {output} out of range for {n} bits"
        );
        if inverse[output] != usize::MAX {
            // A second input mapping to `output` means the map is not injective — it erases.
            return GateCert::Collision {
                input_a: inverse[output],
                input_b: input,
                output,
            };
        }
        inverse[output] = input;
    }
    GateCert::Reversible { inverse }
}

/// CNOT (2-bit): `(a,b) ↦ (a, a⊕b)`, state encoded `a | b<<1`. The universal reversible XOR
/// generator — `RevXor` lifted to a gate.
pub fn cnot() -> Vec<usize> {
    (0..4)
        .map(|s| {
            let (a, b) = (s & 1, (s >> 1) & 1);
            a | ((a ^ b) << 1)
        })
        .collect()
}

/// Toffoli / CCNOT (3-bit): `(a,b,c) ↦ (a, b, c⊕(a∧b))`. The universal reversible AND generator.
pub fn toffoli() -> Vec<usize> {
    (0..8)
        .map(|s| {
            let (a, b, c) = (s & 1, (s >> 1) & 1, (s >> 2) & 1);
            a | (b << 1) | ((c ^ (a & b)) << 2)
        })
        .collect()
}

/// Fredkin / CSWAP (3-bit): swap `b` and `c` when `a = 1`. Conserves 1-count; self-inverse.
pub fn fredkin() -> Vec<usize> {
    (0..8)
        .map(|s| {
            let (a, b, c) = (s & 1, (s >> 1) & 1, (s >> 2) & 1);
            let (b2, c2) = if a == 1 { (c, b) } else { (b, c) };
            a | (b2 << 1) | (c2 << 2)
        })
        .collect()
}

/// Certify a purely-LINEAR `n`-bit block reversible: its GF(2) transfer matrix (`rows[v]` a bitmask
/// of which input bits XOR into output bit `v`) must be invertible (`∈ GL(n,2)`), giving a linear
/// inverse. This is the fast path for CNOT-only (XOR) datapaths.
pub fn certify_linear(n: u32, rows: &[u64]) -> bool {
    is_invertible_gf2(n, rows)
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/revgate/tests.rs"]
mod tests;
