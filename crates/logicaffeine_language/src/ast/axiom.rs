//! AST nodes for the formal-logic vernacular: `## Axiom` and `## Theory` blocks.
//!
//! Both store their formal body as RAW TEXT (the same strategy as
//! [`ProofStrategy::Script`](super::theorem::ProofStrategy), which keeps proof scripts as
//! text for a downstream parser). The formal-formula grammar lives in the proof crate
//! (`logicaffeine_proof::formula`), so the language crate does not parse it here — it only
//! captures the text, which the compile layer parses into the prover's `ProofExpr`.

/// A `## Axiom` block: a named first-order axiom in formal notation.
///
/// `## Axiom flip: for all a b, Cong(a, b, b, a).` registers `flip` as a shared premise
/// available to every later theorem in the program — the seam for an axiomatic base like
/// Tarski geometry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AxiomBlock {
    /// The axiom's name (`flip`), for citation and diagnostics.
    pub name: String,
    /// The axiom formula as surface text, parsed downstream by the formal-formula parser.
    pub formula: String,
}

/// A `## Theory` block: a named development that groups the axioms and theorems that
/// follow it (`## Theory Tarski`). Its body is the formal development text — a sequence of
/// `Axiom …` and `Theorem …` declarations parsed downstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TheoryBlock {
    /// The theory's name (`Tarski`).
    pub name: String,
    /// The development body as surface text: `Axiom …` / `Theorem …` declarations, parsed
    /// downstream by the formal-development parser. Empty when the theory is just a header
    /// grouping standalone `## Axiom` / `## Theorem` blocks.
    pub body: String,
}
