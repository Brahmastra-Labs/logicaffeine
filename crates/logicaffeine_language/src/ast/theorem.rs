//! Theorem and proof block AST types.
//!
//! This module defines the AST for theorem blocks in the vernacular proof language:
//!
//! ```text
//! ## Theorem: Socrates_Mortality
//! Given: All men are mortal.
//! Given: Socrates is a man.
//! Prove: Socrates is mortal.
//! Proof: Auto.
//! ```
//!
//! # Key Types
//!
//! - **[`TheoremBlock`]**: Contains premises, goal, and proof strategy
//! - **[`ProofStrategy`]**: How to prove (Auto, Manual, By lemmas)

use super::logic::LogicExpr;

/// A theorem block containing premises, goal, and proof strategy.
#[derive(Debug, Clone)]
pub struct TheoremBlock<'a> {
    /// The name of the theorem (e.g., "Socrates_Mortality")
    pub name: String,

    /// Premises (Given statements) - logical expressions to assume true
    pub premises: Vec<&'a LogicExpr<'a>>,

    /// Optional names for the premises, parallel to `premises`. A `Given (h): …`
    /// names that premise `h`, so a `Proof:` script can refer to it as `cases h`
    /// rather than the positional `hp0`. `None` for an unnamed `Given:`.
    pub premise_names: Vec<Option<String>>,

    /// The goal to prove (Prove statement)
    pub goal: &'a LogicExpr<'a>,

    /// The proof strategy to use
    pub strategy: ProofStrategy,
}

/// Proof strategies for theorem verification.
#[derive(Debug, Clone, PartialEq)]
pub enum ProofStrategy {
    /// Automatic proof search using backward chaining.
    /// The prover will try all available inference rules.
    Auto,

    /// Induction on a variable (for inductive types like Nat, List).
    /// Example: `Proof: By induction on n.`
    ///
    /// Driven by lowering to the tactic script `induction <variable>`, so there is one
    /// proof engine rather than a second, divergent one — and the bare
    /// [`Script`](ProofStrategy::Script) spelling `Proof: Induction on n.` reaches exactly
    /// the same proof by the same route. What the structured variant adds is that the METHOD
    /// and its ARGUMENT are readable without re-parsing prose.
    ///
    /// Only the WHOLE line qualifies: `By induction on n. Automatically.` is a two-step
    /// script and stays one.
    Induction(String),

    /// A one-word `Proof:` line: the proof is a SINGLE named step.
    /// Example: `Proof: Assumption.`
    ///
    /// The classification is by SHAPE — one word is one step, more than one word is a
    /// [`Script`](ProofStrategy::Script) — so it cannot drift against the inference-rule
    /// enum or the tactic table. The vernacular has exactly ONE proof-step namespace (the
    /// tactics), and the step runs through the same engine a script does.
    ///
    /// A word that names a kernel INFERENCE RULE rather than a tactic — `Proof:
    /// ModusPonens.` — is reported as exactly that and is NOT applied here. A rule is not a
    /// step, exactly as Lean answers `by modus_ponens` with "unknown tactic" and wants
    /// `apply`; the diagnostic names the spelling that DOES apply it, `Proof: By ModusPonens.`,
    /// which lowers to the `by_rule` tactic and is certified before it reports success.
    ByRule(String),

    /// An explicit tactic-script proof, written in the English-esque vernacular and run by
    /// the tactic framework. Over the block
    ///
    /// ```text
    /// ## Theorem: Or_Commutes
    /// Given (h): Bob is happy or Bob is sad.
    /// Prove: Bob is sad or Bob is happy.
    /// Proof: By cases on h. Right, by assumption. Left, by assumption.
    /// ```
    ///
    /// the script is `Proof: By cases on h. Right, by assumption. Left, by assumption.`
    ///
    /// The example names its BLOCK because a proof step is only meaningful against a goal:
    /// the spelling taught here before, which opened with `Assume h.`, wanted a goal
    /// that was BOTH an implication (for `Assume`) and a disjunction (for `cases`), a shape
    /// this surface has no sentence for, so the one worked example of the one open-ended
    /// variant could not be run on anything. That is the same defect as T12-005 one variant
    /// over. Every `Proof:` spelling quoted in this file is now extracted from the file and
    /// run against the surface by
    /// `logicaffeine-tests::L_language_extra::every_proof_spelling_our_own_doc_comments_teach_is_checked_against_the_surface`,
    /// so a doc comment cannot teach a spelling the surface does not accept.
    Script(String),
}

impl Default for ProofStrategy {
    fn default() -> Self {
        ProofStrategy::Auto
    }
}
