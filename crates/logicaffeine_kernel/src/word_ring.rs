//! Kernel ring proofs for the word ring ℤ/2ⁿ (`Word8`/`Word16`/`Word32`/`Word64`).
//!
//! The [`crate::ring`] procedure proves a polynomial identity by reducing both sides to a
//! canonical multivariate polynomial over ℤ and comparing. An identity that canonicalizes equal
//! holds in **every** commutative ring — so it holds in the word ring ℤ/2ⁿ, whose `+`/`*` are
//! `wrapping_add`/`wrapping_mul` (the quotient of ℤ by 2ⁿℤ; the ring axioms survive the quotient).
//!
//! This is the soundness certificate behind every reassociation and product reshaping the
//! optimizer performs on wrapping arithmetic: associativity, commutativity, distributivity, and
//! the Karatsuba/gauss product form are all kernel-certified here, at FULL word width with no
//! bound on the operands. The bit-logic side of the crypto (`xor`, `rotl`) is not polynomial and
//! is certified separately by `bitblast→CDCL`.
//!
//! Each lemma returns `true` iff the kernel certifies it; the tests pin both the positive proofs
//! and their non-vacuity (a WRONG identity must NOT canonicalize equal).

use crate::ring::Polynomial;

#[inline]
fn a() -> Polynomial {
    Polynomial::var(0)
}
#[inline]
fn b() -> Polynomial {
    Polynomial::var(1)
}
#[inline]
fn c() -> Polynomial {
    Polynomial::var(2)
}
#[inline]
fn d() -> Polynomial {
    Polynomial::var(3)
}

/// `(a + b) + c = a + (b + c)` — additive associativity in ℤ/2ⁿ.
pub fn add_associative() -> bool {
    a().add(&b())
        .add(&c())
        .canonical_eq(&a().add(&b().add(&c())))
}

/// `a + b = b + a` — additive commutativity in ℤ/2ⁿ.
pub fn add_commutative() -> bool {
    a().add(&b()).canonical_eq(&b().add(&a()))
}

/// `(a · b) · c = a · (b · c)` — multiplicative associativity in ℤ/2ⁿ.
pub fn mul_associative() -> bool {
    a().mul(&b())
        .mul(&c())
        .canonical_eq(&a().mul(&b().mul(&c())))
}

/// `a · b = b · a` — multiplicative commutativity in ℤ/2ⁿ.
pub fn mul_commutative() -> bool {
    a().mul(&b()).canonical_eq(&b().mul(&a()))
}

/// `a · (b + c) = a · b + a · c` — left distributivity in ℤ/2ⁿ.
pub fn left_distributive() -> bool {
    a().mul(&b().add(&c()))
        .canonical_eq(&a().mul(&b()).add(&a().mul(&c())))
}

/// `a + 0 = a` — additive identity in ℤ/2ⁿ.
pub fn additive_identity() -> bool {
    a().add(&Polynomial::constant(0)).canonical_eq(&a())
}

/// `a · 1 = a` — multiplicative identity in ℤ/2ⁿ.
pub fn multiplicative_identity() -> bool {
    a().mul(&Polynomial::constant(1)).canonical_eq(&a())
}

/// `(a + b)(c + d) = ac + ad + bc + bd` — the Karatsuba/gauss product expansion, the identity the
/// 3-multiply NTT butterfly and complex-multiply reshaping rely on, certified at full word width.
pub fn karatsuba_expand() -> bool {
    let lhs = a().add(&b()).mul(&c().add(&d()));
    let rhs = a()
        .mul(&c())
        .add(&a().mul(&d()))
        .add(&b().mul(&c()))
        .add(&b().mul(&d()));
    lhs.canonical_eq(&rhs)
}

/// Every word-ring law the optimizer leans on, kernel-certified — a single gate.
pub fn all_word_ring_laws_certified() -> bool {
    add_associative()
        && add_commutative()
        && mul_associative()
        && mul_commutative()
        && left_distributive()
        && additive_identity()
        && multiplicative_identity()
        && karatsuba_expand()
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/word_ring/tests.rs"]
mod tests;
