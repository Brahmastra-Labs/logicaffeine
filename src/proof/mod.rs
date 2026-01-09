// =============================================================================
// LOGOS PROOF ENGINE - CORE STRUCTURES (PHASE 60)
// =============================================================================
// This module defines the "Shape of Truth."
// A proof is not a boolean; it is a recursive tree of inference rules.
//
// Curry-Howard Correspondence:
// - A Proposition is a Type
// - A Proof is a Program
// - Verification is Type Checking

pub mod certifier;
pub mod convert;
pub mod engine;
pub mod error;
pub mod hints;
pub mod unify;

#[cfg(feature = "verification")]
pub mod oracle;

pub use convert::{logic_expr_to_proof_expr, term_to_proof_term};
pub use engine::BackwardChainer;
pub use error::ProofError;
pub use hints::{suggest_hint, SocraticHint, SuggestedTactic};
pub use unify::Substitution;

use std::fmt;

// =============================================================================
// PROOF TERM - Owned representation of logical terms
// =============================================================================

/// Owned term representation for proof manipulation.
/// Decoupled from arena-allocated Term<'a> to allow proof trees to persist.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProofTerm {
    /// A constant symbol (e.g., "Socrates", "42")
    Constant(String),

    /// A variable symbol (e.g., "x", "y")
    Variable(String),

    /// A function application (e.g., "father(x)", "add(1, 2)")
    Function(String, Vec<ProofTerm>),

    /// A group/tuple of terms (e.g., "(x, y)")
    Group(Vec<ProofTerm>),

    /// Reference to a bound variable in a pattern context.
    /// Distinct from Variable (unification var) and Constant (global name).
    /// Prevents variable capture bugs during alpha-conversion.
    BoundVarRef(String),
}

impl fmt::Display for ProofTerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProofTerm::Constant(s) => write!(f, "{}", s),
            ProofTerm::Variable(s) => write!(f, "{}", s),
            ProofTerm::Function(name, args) => {
                write!(f, "{}(", name)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arg)?;
                }
                write!(f, ")")
            }
            ProofTerm::Group(terms) => {
                write!(f, "(")?;
                for (i, t) in terms.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", t)?;
                }
                write!(f, ")")
            }
            ProofTerm::BoundVarRef(s) => write!(f, "#{}", s),
        }
    }
}

// =============================================================================
// PROOF EXPRESSION - Owned representation of logical expressions
// =============================================================================

/// Owned expression representation for proof manipulation.
/// Supports all LogicExpr variants to enable full language coverage.
#[derive(Debug, Clone, PartialEq)]
pub enum ProofExpr {
    // --- Core FOL ---

    /// Atomic predicate: P(t1, t2, ...)
    Predicate {
        name: String,
        args: Vec<ProofTerm>,
        world: Option<String>,
    },

    /// Identity: t1 = t2
    Identity(ProofTerm, ProofTerm),

    /// Propositional atom
    Atom(String),

    // --- Logical Connectives ---

    /// Conjunction: P ∧ Q
    And(Box<ProofExpr>, Box<ProofExpr>),

    /// Disjunction: P ∨ Q
    Or(Box<ProofExpr>, Box<ProofExpr>),

    /// Implication: P → Q
    Implies(Box<ProofExpr>, Box<ProofExpr>),

    /// Biconditional: P ↔ Q
    Iff(Box<ProofExpr>, Box<ProofExpr>),

    /// Negation: ¬P
    Not(Box<ProofExpr>),

    // --- Quantifiers ---

    /// Universal quantification: ∀x P(x)
    ForAll {
        variable: String,
        body: Box<ProofExpr>,
    },

    /// Existential quantification: ∃x P(x)
    Exists {
        variable: String,
        body: Box<ProofExpr>,
    },

    // --- Modal Logic ---

    /// Modal operator: □P or ◇P (with world semantics)
    Modal {
        domain: String,
        force: f32,
        flavor: String,
        body: Box<ProofExpr>,
    },

    // --- Temporal Logic ---

    /// Temporal operator: Past(P) or Future(P)
    Temporal {
        operator: String,
        body: Box<ProofExpr>,
    },

    // --- Lambda Calculus (CIC extension) ---

    /// Lambda abstraction: λx.P
    Lambda {
        variable: String,
        body: Box<ProofExpr>,
    },

    /// Function application: (f x)
    App(Box<ProofExpr>, Box<ProofExpr>),

    // --- Event Semantics ---

    /// Neo-Davidsonian event: ∃e(Verb(e) ∧ Agent(e,x) ∧ ...)
    NeoEvent {
        event_var: String,
        verb: String,
        roles: Vec<(String, ProofTerm)>,
    },

    // --- Peano / Inductive Types (Phase 61 - CIC Extension) ---

    /// Data Constructor: Zero, Succ(n), Cons(h, t), etc.
    /// The building blocks of inductive types.
    Ctor {
        name: String,
        args: Vec<ProofExpr>,
    },

    /// Pattern Matching: match n { Zero => A, Succ(k) => B }
    /// Eliminates inductive types by case analysis.
    Match {
        scrutinee: Box<ProofExpr>,
        arms: Vec<MatchArm>,
    },

    /// Recursive Function (Fixpoint): fix f. λn. ...
    /// Defines recursive functions over inductive types.
    Fixpoint {
        name: String,
        body: Box<ProofExpr>,
    },

    /// Typed Variable: n : Nat
    /// Signals to the prover that induction may be applicable.
    TypedVar {
        name: String,
        typename: String,
    },

    // --- Higher-Order Pattern Unification (Phase 67) ---

    /// Meta-variable (unification hole) to be solved during proof search.
    /// Represents an unknown expression, typically a function or predicate.
    /// Example: Hole("P") in ?P(x) = Body, to be solved as P = λx.Body
    Hole(String),

    /// Embedded term (lifts ProofTerm into ProofExpr context).
    /// Used when a term appears where an expression is expected.
    /// Example: App(Hole("P"), Term(BoundVarRef("x")))
    Term(ProofTerm),

    // --- Fallback ---

    /// Unsupported construct (with description for debugging)
    Unsupported(String),
}

// =============================================================================
// MATCH ARM - A single case in pattern matching
// =============================================================================

/// A single arm in a match expression.
/// Example: Succ(k) => Add(k, m)
#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    /// The constructor being matched: "Zero", "Succ", "Cons", etc.
    pub ctor: String,

    /// Variable bindings for constructor arguments: ["k"] for Succ(k)
    pub bindings: Vec<String>,

    /// The expression to evaluate when this arm matches
    pub body: ProofExpr,
}

impl fmt::Display for ProofExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProofExpr::Predicate { name, args, world } => {
                write!(f, "{}", name)?;
                if !args.is_empty() {
                    write!(f, "(")?;
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", arg)?;
                    }
                    write!(f, ")")?;
                }
                if let Some(w) = world {
                    write!(f, " @{}", w)?;
                }
                Ok(())
            }
            ProofExpr::Identity(left, right) => write!(f, "{} = {}", left, right),
            ProofExpr::Atom(s) => write!(f, "{}", s),
            ProofExpr::And(left, right) => write!(f, "({} ∧ {})", left, right),
            ProofExpr::Or(left, right) => write!(f, "({} ∨ {})", left, right),
            ProofExpr::Implies(left, right) => write!(f, "({} → {})", left, right),
            ProofExpr::Iff(left, right) => write!(f, "({} ↔ {})", left, right),
            ProofExpr::Not(inner) => write!(f, "¬{}", inner),
            ProofExpr::ForAll { variable, body } => write!(f, "∀{} {}", variable, body),
            ProofExpr::Exists { variable, body } => write!(f, "∃{} {}", variable, body),
            ProofExpr::Modal { domain, force, flavor, body } => {
                write!(f, "□[{}/{}/{}]{}", domain, force, flavor, body)
            }
            ProofExpr::Temporal { operator, body } => write!(f, "{}({})", operator, body),
            ProofExpr::Lambda { variable, body } => write!(f, "λ{}.{}", variable, body),
            ProofExpr::App(func, arg) => write!(f, "({} {})", func, arg),
            ProofExpr::NeoEvent { event_var, verb, roles } => {
                write!(f, "∃{}({}({})", event_var, verb, event_var)?;
                for (role, term) in roles {
                    write!(f, " ∧ {}({}, {})", role, event_var, term)?;
                }
                write!(f, ")")
            }
            ProofExpr::Ctor { name, args } => {
                write!(f, "{}", name)?;
                if !args.is_empty() {
                    write!(f, "(")?;
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", arg)?;
                    }
                    write!(f, ")")?;
                }
                Ok(())
            }
            ProofExpr::Match { scrutinee, arms } => {
                write!(f, "match {} {{ ", scrutinee)?;
                for (i, arm) in arms.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arm.ctor)?;
                    if !arm.bindings.is_empty() {
                        write!(f, "({})", arm.bindings.join(", "))?;
                    }
                    write!(f, " => {}", arm.body)?;
                }
                write!(f, " }}")
            }
            ProofExpr::Fixpoint { name, body } => write!(f, "fix {}.{}", name, body),
            ProofExpr::TypedVar { name, typename } => write!(f, "{}:{}", name, typename),
            ProofExpr::Unsupported(desc) => write!(f, "⟨unsupported: {}⟩", desc),
            ProofExpr::Hole(name) => write!(f, "?{}", name),
            ProofExpr::Term(term) => write!(f, "{}", term),
        }
    }
}

// =============================================================================
// INFERENCE RULE - The logical moves available to the prover
// =============================================================================

/// The "Lever" - The specific logical move made at each proof step.
/// This enum captures HOW we moved from Premises to Conclusion.
#[derive(Debug, Clone, PartialEq)]
pub enum InferenceRule {
    // --- Basic FOL ---

    /// Direct match with a known fact in the Context/KnowledgeBase.
    /// Logic: Γ, P ⊢ P
    PremiseMatch,

    /// Logic: P → Q, P ⊢ Q
    ModusPonens,

    /// Logic: ¬Q, P → Q ⊢ ¬P
    ModusTollens,

    /// Logic: P, Q ⊢ P ∧ Q
    ConjunctionIntro,

    /// Logic: P ∧ Q ⊢ P (or Q)
    ConjunctionElim,

    /// Logic: P ⊢ P ∨ Q
    DisjunctionIntro,

    /// Logic: P ∨ Q, P → R, Q → R ⊢ R
    DisjunctionElim,

    /// Logic: ¬¬P ⊢ P (and P ⊢ ¬¬P)
    DoubleNegation,

    // --- Quantifiers ---

    /// Logic: ∀x P(x) ⊢ P(c)
    /// Stores the specific term 'c' used to instantiate the universal.
    UniversalInst(String),

    /// Logic: Γ, x:T ⊢ P(x) implies Γ ⊢ ∀x:T. P(x)
    /// Stores variable name and type name for Lambda construction.
    UniversalIntro { variable: String, var_type: String },

    /// Logic: P(w) ⊢ ∃x.P(x)
    /// Carries the witness and its type for kernel certification.
    ExistentialIntro {
        witness: String,
        witness_type: String,
    },

    // --- Modal Logic (World Moves) ---

    /// Logic: □P (in w0), Accessible(w0, w1) ⊢ P (in w1)
    /// "Necessity Elimination" / "Distribution"
    ModalAccess,

    /// Logic: If P is true in ALL accessible worlds ⊢ □P
    /// "Necessity Introduction"
    ModalGeneralization,

    // --- Temporal Logic ---

    /// Logic: t1 < t2, t2 < t3 ⊢ t1 < t3
    TemporalTransitivity,

    // --- Peano / Inductive Reasoning (CIC seed) ---

    /// Logic: P(0), ∀k(P(k) → P(S(k))) ⊢ ∀n P(n)
    /// Stores the variable name, its inductive type, and the step variable used.
    StructuralInduction {
        variable: String,  // "n" - the induction variable
        ind_type: String,  // "Nat" - the inductive type
        step_var: String,  // "k" - the predecessor variable (for IH matching)
    },

    // --- Equality ---

    /// Leibniz's Law / Substitution of Equals
    /// Logic: a = b, P(a) ⊢ P(b)
    /// The equality proof is in premise[0], the P(a) proof is in premise[1].
    /// Carries the original term and replacement term for certification.
    Rewrite {
        from: ProofTerm,
        to: ProofTerm,
    },

    /// Symmetry of Equality: a = b ⊢ b = a
    EqualitySymmetry,

    /// Transitivity of Equality: a = b, b = c ⊢ a = c
    EqualityTransitivity,

    /// Reflexivity of Equality: a = a (after normalization)
    /// Used when both sides of an identity reduce to the same normal form.
    Reflexivity,

    // --- Fallbacks ---

    /// "The User Said So." Used for top-level axioms.
    Axiom,

    /// "The Machine Said So." (Z3 Oracle - Phase 2)
    /// The string contains the solver's justification.
    OracleVerification(String),

    /// Proof by Contradiction (Reductio ad Absurdum)
    /// Logic: Assume ¬C, derive P ∧ ¬P (contradiction), conclude C
    /// Or: Assume P, derive Q ∧ ¬Q, conclude ¬P
    ReductioAdAbsurdum,

    /// Contradiction detected in premises: P and ¬P both hold
    /// Logic: P, ¬P ⊢ ⊥ (ex falso quodlibet)
    Contradiction,

    // --- Quantifier Elimination ---

    /// Existential Elimination (Skolemization in a proof context)
    /// Logic: ∃x.P(x), [c fresh] P(c) ⊢ Goal implies ∃x.P(x) ⊢ Goal
    /// The witness c must be fresh (not appearing in Goal).
    ExistentialElim { witness: String },

    /// Case Analysis (Tertium Non Datur / Law of Excluded Middle)
    /// Logic: (P → ⊥), (¬P → ⊥) ⊢ ⊥
    /// Used for self-referential paradoxes like the Barber Paradox.
    CaseAnalysis { case_formula: String },
}

// =============================================================================
// DERIVATION TREE - The recursive proof structure
// =============================================================================

/// The "Euclidean Structure" - A recursive tree representing the proof.
///
/// This is the object returned by the Prover. It allows the UI to render
/// a step-by-step explanation (Natural Language Generation).
#[derive(Debug, Clone)]
pub struct DerivationTree {
    /// The logical statement proved at this node.
    pub conclusion: ProofExpr,

    /// The rule applied to justify this conclusion.
    pub rule: InferenceRule,

    /// The sub-proofs that validate the requirements of the rule.
    /// Example: ModusPonens will have 2 children (The Implication, The Antecedent).
    pub premises: Vec<DerivationTree>,

    /// The depth of the tree (used for complexity limits).
    pub depth: usize,

    /// Substitution applied at this step (for unification-based rules).
    pub substitution: unify::Substitution,
}

impl DerivationTree {
    /// Constructs a new Proof Node.
    /// Automatically calculates depth based on children.
    pub fn new(
        conclusion: ProofExpr,
        rule: InferenceRule,
        premises: Vec<DerivationTree>,
    ) -> Self {
        let max_depth = premises.iter().map(|p| p.depth).max().unwrap_or(0);
        Self {
            conclusion,
            rule,
            premises,
            depth: max_depth + 1,
            substitution: unify::Substitution::new(),
        }
    }

    /// A leaf node (usually a Premise, Axiom, or Oracle result).
    pub fn leaf(conclusion: ProofExpr, rule: InferenceRule) -> Self {
        Self::new(conclusion, rule, vec![])
    }

    /// Set the substitution for this derivation step.
    pub fn with_substitution(mut self, subst: unify::Substitution) -> Self {
        self.substitution = subst;
        self
    }

    /// Renders the proof as a text-based tree (for debugging/CLI).
    pub fn display_tree(&self) -> String {
        self.display_recursive(0)
    }

    fn display_recursive(&self, indent: usize) -> String {
        let prefix = "  ".repeat(indent);

        let rule_name = match &self.rule {
            InferenceRule::UniversalInst(var) => format!("UniversalInst({})", var),
            InferenceRule::UniversalIntro { variable, var_type } => {
                format!("UniversalIntro({}:{})", variable, var_type)
            }
            InferenceRule::ExistentialIntro { witness, witness_type } => {
                format!("∃Intro({}:{})", witness, witness_type)
            }
            InferenceRule::StructuralInduction { variable, ind_type, step_var } => {
                format!("Induction({}:{}, step={})", variable, ind_type, step_var)
            }
            InferenceRule::OracleVerification(s) => format!("Oracle({})", s),
            InferenceRule::Rewrite { from, to } => format!("Rewrite({} → {})", from, to),
            InferenceRule::EqualitySymmetry => "EqSymmetry".to_string(),
            InferenceRule::EqualityTransitivity => "EqTransitivity".to_string(),
            r => format!("{:?}", r),
        };

        let mut output = format!("{}└─ [{}] {}\n", prefix, rule_name, self.conclusion);

        for premise in &self.premises {
            output.push_str(&premise.display_recursive(indent + 1));
        }
        output
    }
}

impl fmt::Display for DerivationTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_tree())
    }
}

// =============================================================================
// PROOF GOAL - The target state for backward chaining
// =============================================================================

/// The Goal State for the Backward Chainer.
/// Represents a "hole" in the proof that needs to be filled.
#[derive(Debug, Clone)]
pub struct ProofGoal {
    /// What we are trying to prove right now.
    pub target: ProofExpr,

    /// The local assumptions available (e.g., inside an implication).
    pub context: Vec<ProofExpr>,
}

impl ProofGoal {
    pub fn new(target: ProofExpr) -> Self {
        Self {
            target,
            context: Vec::new(),
        }
    }

    pub fn with_context(target: ProofExpr, context: Vec<ProofExpr>) -> Self {
        Self { target, context }
    }
}
