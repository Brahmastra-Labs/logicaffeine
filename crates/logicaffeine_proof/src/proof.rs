//! Shared proof-step vocabulary for the certified checkers.
//!
//! A refutation is an ordered stream of clause *additions*, each carrying a justification a
//! tiny independent checker can replay:
//!
//! - [`ProofStep::Rup`] — the clause follows by reverse unit propagation (the DRAT/RUP unit).
//! - [`ProofStep::Pr`] — the clause is **propagation-redundant** with an explicit [`Witness`]
//!   (Heule, Kiesl & Biere, *Short Proofs Without New Variables*, CADE 2017). This is the only
//!   form that can certify a *model-removing* addition — exactly what a symmetry-breaking
//!   predicate is — so it is the seam through which certified symmetry breaking flows.
//!
//! The witness may be an explicit partial [`Witness::Assignment`] or a literal
//! [`Witness::Substitution`] (a permutation / automorphism). The substitution form is mere
//! sugar: against a clause `C` whose falsifying assignment is `α = ¬C`, the substitution `σ`
//! denotes the assignment witness `ω = { ¬σ(l) : l ∈ C }` — the image of the "bad" corner of
//! the cube under the symmetry, which is the canonical PR witness for a lex-leader SBP. The
//! checker ([`crate::pr`]) reduces every substitution to that assignment, so there is a single
//! well-specified propagation-redundancy criterion to trust.

use crate::cdcl::{Lit, Var};

/// A literal permutation that respects negation: `σ(¬l) = ¬σ(l)`. Represented by the image of
/// each variable's positive literal, so the negation invariant holds by construction.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Perm {
    /// `pos_image[v] = σ(+v)`. The negative literal's image is its negation.
    pos_image: Vec<Lit>,
}

impl Perm {
    /// The identity permutation over `num_vars` variables.
    pub fn identity(num_vars: usize) -> Perm {
        Perm { pos_image: (0..num_vars as Var).map(Lit::pos).collect() }
    }

    /// Build from the image of each positive literal: `images[v] = σ(+v)`.
    pub fn from_images(images: Vec<Lit>) -> Perm {
        Perm { pos_image: images }
    }

    /// Apply `σ` to a literal, respecting sign: `σ(¬l) = ¬σ(l)`.
    pub fn apply(&self, l: Lit) -> Lit {
        let img = self.pos_image[l.var() as usize];
        if l.is_positive() {
            img
        } else {
            img.negated()
        }
    }

    /// Apply `σ` to every literal of a clause.
    pub fn apply_clause(&self, clause: &[Lit]) -> Vec<Lit> {
        clause.iter().map(|&l| self.apply(l)).collect()
    }

    /// The number of variables the permutation is defined over.
    pub fn num_vars(&self) -> usize {
        self.pos_image.len()
    }

    /// Whether this is the identity permutation.
    pub fn is_identity(&self) -> bool {
        self.pos_image.iter().enumerate().all(|(v, &img)| img == Lit::pos(v as Var))
    }

    /// Compose: `(self ∘ other)(l) = self(other(l))` — apply `other` first, then `self`.
    pub fn compose(&self, other: &Perm) -> Perm {
        Perm {
            pos_image: (0..self.pos_image.len() as Var)
                .map(|v| self.apply(other.apply(Lit::pos(v))))
                .collect(),
        }
    }
}

/// A redundancy witness for a [`ProofStep::Pr`] clause.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Witness {
    /// An explicit partial assignment (the set of literals it sets true) — the classic PR
    /// witness.
    Assignment(Vec<Lit>),
    /// A literal permutation / automorphism. Against the clause `C` it certifies, it denotes
    /// the assignment `ω = { ¬σ(l) : l ∈ C }`.
    Substitution(Perm),
}

/// One step of a refutation: add a clause with the justification a checker replays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProofStep {
    /// Add a clause that is RUP w.r.t. the current database.
    Rup(Vec<Lit>),
    /// Add a clause that is propagation-redundant w.r.t. the current database, certified by
    /// `witness`. May remove models; preserves satisfiability.
    Pr { clause: Vec<Lit>, witness: Witness },
}

impl ProofStep {
    /// The clause this step adds.
    pub fn clause(&self) -> &[Lit] {
        match self {
            ProofStep::Rup(c) => c,
            ProofStep::Pr { clause, .. } => clause,
        }
    }
}
