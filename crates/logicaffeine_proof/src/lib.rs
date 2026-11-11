#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

// Number-theory / cryptanalysis substrate — factoring, elliptic/isogeny, lattice, order-finding.
// These are prover tools (the hardness lens behind isogeny/ait/oracle), not runtime crypto; they
// depend only on `logicaffeine_base::numeric`.
pub mod cyclotomic;
pub mod elliptic;
pub mod factor;
pub mod fp2;
pub mod hyperelliptic;
pub mod lattice;
pub mod period;

pub mod arith;
// Independent replay checker for the analog lane's range-safety certificates (the PR/RUP-checker
// pattern applied to voltage bounds): dependency-free, fail-closed, trusted over the producer.
pub mod affine;
pub mod affine_gfp;
pub mod aig;
mod aig_cnf;
mod aig_edit;
pub mod ait;
pub mod atpg;
pub mod bmc;
pub mod cardinality;
pub mod cdcl;
pub mod certifier;
pub mod cipher;
pub mod cnf;
pub mod coalgebra;
pub mod complexity;
pub mod count;
pub mod counterexample;
pub mod counting_principle;
pub mod crush;
pub mod cube;
pub mod cubical;
pub mod cycle_ratio;
pub mod decide;
pub mod development;
pub mod dimacs;
pub mod discrimination;
pub mod eilenberg_maclane;
pub mod engine;
pub mod error;
pub mod families;
pub mod formula;
pub mod fraig;
pub mod gf2;
pub mod grid_solver;
pub mod grounding;
pub mod groupoid;
pub mod hints;
pub mod hornsat;
pub mod hypercube;
pub mod inprocess;
pub mod interpolate;
pub mod interval_replay;
pub mod interval_sched;
pub mod isogeny;
pub mod itp;
pub mod kan_complex;
pub mod kernel_refutation;
pub mod lemma_index;
pub mod linalg;
pub mod linalg_certify;
pub mod linarith_solve;
pub mod lll;
pub mod lrat;
pub mod lyapunov;
pub mod macaulay;
pub mod matching;
pub mod localize;
pub mod scorr;
pub mod mc;
pub mod min_cost_flow;
pub mod modm;
pub mod modp;
mod numeral;
pub mod omega_solve;
pub mod optimize;
pub mod orbit_stability;
pub mod ordering;
pub mod parity_cardinality;
pub mod pdr;
pub mod permgroup;
pub mod pigeonhole;
pub mod polycalc;
pub mod polycalc_gfp;
pub mod polycalc_zm;
pub mod postnikov;
pub mod pr;
pub mod preprocess;
pub mod portfolio;
pub mod progress_complex;
pub mod proof;
pub mod proof_emit;
pub mod proof_rewrite;
pub mod pseudo_boolean;
pub mod rainbow;
pub mod register_alloc;
pub mod relax_replay;
pub mod res_width;
pub mod revgate;
pub mod rewrite;
pub mod rule_search;
pub mod rup;
pub mod sat;
pub mod satcli;
pub mod sdcl;
pub mod seq_equiv;
pub mod simp;
pub mod solve;
pub mod sos;
pub mod steenrod;
pub mod sweep;
pub mod sym_break;
pub mod sym_certify;
pub mod sym_dynamic;
mod symbols;
pub mod symmetry;
pub mod symmetry_detect;
pub mod tactic;
pub mod tactic_cert;
pub mod tactic_script;
pub mod thermal_replay;
pub mod trace_determinism;
pub mod two_group;
pub mod two_type;
pub mod twosat;
pub mod unify;
pub mod units;
pub mod streams;
pub mod verify;
pub mod walk;
pub mod xl;
pub mod xor_drat;
pub mod xor_engine;
pub mod xorsat;

#[cfg(feature = "verification")]
mod modal_translation;
#[cfg(feature = "verification")]
pub mod oracle;

pub use engine::{BackwardChainer, PremiseVariables};
pub use error::ProofError;
pub use hints::{suggest_hint, SocraticHint, SuggestedTactic};
pub use unify::Substitution;

use std::fmt;

// =============================================================================
// PROOF TERM - Owned representation of logical terms
// =============================================================================

/// Owned term representation for proof manipulation.
///
/// Decoupled from arena-allocated `Term<'a>` to allow proof trees to persist
/// beyond the lifetime of the parsing arena.
///
/// # See Also
///
/// * [`ProofExpr`] - Expression-level representation containing `ProofTerm`s
/// * [`unify::unify_terms`] - Unification algorithm for terms
/// * [`unify::Substitution`] - Maps variables to terms
/// * [`certifier::certify`] - Converts proof trees to kernel terms
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
///
/// Supports all `LogicExpr` variants to enable full language coverage.
/// This is the "proposition" side of the Curry-Howard correspondence.
///
/// # See Also
///
/// * [`ProofTerm`] - Terms embedded within expressions (predicate arguments)
/// * [`unify::unify_exprs`] - Expression-level unification with alpha-equivalence
/// * [`unify::beta_reduce`] - Normalizes lambda applications
/// * [`certifier::certify`] - Converts expressions to kernel types
/// * [`DerivationTree`] - Proof trees conclude with a `ProofExpr`
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

    /// Counterfactual conditional: P □→ Q. Quantifies the consequent over the
    /// closest antecedent-worlds (a similarity ordering), so it is strictly
    /// distinct from material implication in both directions.
    Counterfactual {
        antecedent: Box<ProofExpr>,
        consequent: Box<ProofExpr>,
    },

    // --- Temporal Logic ---
    /// Temporal operator: Past(P), Future(P), Always(P), Eventually(P), Next(P)
    Temporal {
        operator: String,
        body: Box<ProofExpr>,
    },

    /// Binary temporal operator: Until(P,Q), Release(P,Q), WeakUntil(P,Q)
    TemporalBinary {
        operator: String,
        left: Box<ProofExpr>,
        right: Box<ProofExpr>,
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

    // --- Peano / Inductive Types (CIC Extension) ---
    /// Data Constructor: Zero, Succ(n), Cons(h, t), etc.
    /// The building blocks of inductive types.
    Ctor { name: String, args: Vec<ProofExpr> },

    /// Pattern Matching: match n { Zero => A, Succ(k) => B }
    /// Eliminates inductive types by case analysis.
    Match {
        scrutinee: Box<ProofExpr>,
        arms: Vec<MatchArm>,
    },

    /// Recursive Function (Fixpoint): fix f. λn. ...
    /// Defines recursive functions over inductive types.
    Fixpoint { name: String, body: Box<ProofExpr> },

    /// Typed Variable: n : Nat
    /// Signals to the prover that induction may be applicable.
    TypedVar { name: String, typename: String },

    // --- Higher-Order Pattern Unification ---
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
            ProofExpr::Modal {
                domain,
                force,
                flavor,
                body,
            } => {
                write!(f, "□[{}/{}/{}]{}", domain, force, flavor, body)
            }
            ProofExpr::Counterfactual {
                antecedent,
                consequent,
            } => {
                write!(f, "({} □→ {})", antecedent, consequent)
            }
            ProofExpr::Temporal { operator, body } => write!(f, "{}({})", operator, body),
            ProofExpr::TemporalBinary {
                operator,
                left,
                right,
            } => {
                write!(f, "TemporalBinary({}, {}, {})", operator, left, right)
            }
            ProofExpr::Lambda { variable, body } => write!(f, "λ{}.{}", variable, body),
            ProofExpr::App(func, arg) => write!(f, "({} {})", func, arg),
            ProofExpr::NeoEvent {
                event_var,
                verb,
                roles,
            } => {
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
///
/// This enum captures HOW we moved from premises to conclusion. Each variant
/// corresponds to a different proof strategy or logical rule.
///
/// # See Also
///
/// * [`DerivationTree`] - Each node contains an `InferenceRule`
/// * [`certifier::certify`] - Maps inference rules to kernel terms
/// * [`hints::suggest_hint`] - Suggests applicable rules when stuck
/// * [`BackwardChainer`] - The engine that selects and applies rules
///
/// One constructor's case in a generic induction scheme
/// ([`InferenceRule::InductionScheme`]). The derivation's premises align 1:1 with
/// the cases, in the inductive's constructor-registration order.
#[derive(Debug, Clone, PartialEq)]
pub struct InductionCase {
    /// The constructor this case eliminates (e.g. `"Succ"`, `"Node"`).
    pub constructor: String,
    /// The constructor's arguments, in order — each bound in the case body.
    pub args: Vec<InductionArg>,
}

/// One argument of a constructor in an [`InductionCase`].
#[derive(Debug, Clone, PartialEq)]
pub struct InductionArg {
    /// The name bound for this argument in the case body.
    pub name: String,
    /// Whether this argument is itself of the inductive type — a recursive
    /// position, which therefore carries an induction hypothesis.
    pub recursive: bool,
}

/// `ModusPonens` → `modus_ponens`. The script spelling of a rule name: a capital starts a new
/// word, and a run of capitals (`EqualityTransitivity` has none, but a future `LHS` would) stays
/// one word, so the transformation is a bijection on the names it is applied to.
fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let chars: Vec<char> = name.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            let prev_lower = chars[i - 1].is_ascii_lowercase() || chars[i - 1].is_ascii_digit();
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_ascii_lowercase());
            if prev_lower || next_lower {
                out.push('_');
            }
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

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

    /// Logic: ⊥ ⊢ anything (ex falso quodlibet). The single premise concludes `False`.
    ExFalso,

    /// Logic: assume P, derive Q ⊢ P → Q (implication introduction / →I). The
    /// single premise proves Q with P bound as a local hypothesis.
    ImpliesIntro,

    /// Logic: prove P → Q and Q → P ⊢ P ↔ Q (biconditional introduction / ↔I).
    BicondIntro,

    /// Logic: ¬¬P ⊢ P (and P ⊢ ¬¬P)
    DoubleNegation,

    /// Logic: classical reductio (proof by contradiction) — assume ¬G, derive ⊥ ⊢ G,
    /// discharged through the `dne` axiom. The single premise concludes `False` with
    /// ¬G bound as a local hypothesis.
    ClassicalReductio,

    // --- Quantifiers ---
    /// Logic: ∀x P(x) ⊢ P(c)
    /// Stores the specific term 'c' used to instantiate the universal.
    UniversalInst(String),

    /// Logic: ∀x P(x) ⊢ P(t) at an arbitrary witness TERM (a compound like
    /// `add(a, Zero)`, not just a name). `UniversalInst` keeps the name-only
    /// fast path; this carries the full term for instantiations that
    /// `simp`/`crush` produce by matching.
    UniversalInstTerm(ProofTerm),

    /// Logic: Γ, x:T ⊢ P(x) implies Γ ⊢ ∀x:T. P(x)
    /// Stores variable name and type name for Lambda construction.
    UniversalIntro { variable: String, var_type: String },

    /// Logic: P(w) ⊢ ∃x.P(x)
    /// Carries the witness and its type for kernel certification.
    ExistentialIntro {
        witness: String,
        witness_type: String,
    },

    /// Logic: P(t) ⊢ ∃x.P(x) at an arbitrary witness TERM — `use (n + 1)`, `use (f x)`,
    /// the compound witnesses Lean's `exists`/`use` takes. [`InferenceRule::ExistentialIntro`]
    /// carries its witness as a `String`, which the certifier can only resolve as one NAME, so
    /// `add(n, 1)` certified to a global literally called `"add(n, 1)"` that no context
    /// declares. This carries the term, exactly as [`InferenceRule::UniversalInstTerm`] sits
    /// beside [`InferenceRule::UniversalInst`] for the dual quantifier.
    ///
    /// `witness_type` is the ∃-binder's DOMAIN, not the witness's own type name: it must be
    /// the domain `certifier::proof_expr_to_type` gives the same `Exists`, or the certified
    /// term's type and the goal type disagree. Both sides read it from one function,
    /// `certifier::existential_domain`.
    ExistentialIntroTerm {
        witness: ProofTerm,
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

    /// Logic: P(s₀), ∀s(P(s) → P(next(s))) ⊢ G(P)
    /// Standard k-induction for hardware safety properties.
    TemporalInduction,

    /// Logic: G(P) ⊢ P ∧ X(G(P))
    /// Fixpoint unfolding of Always.
    TemporalUnfolding,

    /// Logic: P(w) ⊢ F(P) for witness world w
    /// Prove Eventually by exhibiting a reachable witness.
    EventualityProgress,

    /// Logic: Induction on trace length for P U Q
    UntilInduction,

    // --- Peano / Inductive Reasoning (CIC seed) ---
    /// Logic: P(0), ∀k(P(k) → P(S(k))) ⊢ ∀n P(n)
    /// Stores the variable name, its inductive type, and the step variable used.
    StructuralInduction {
        variable: String, // "n" - the induction variable
        ind_type: String, // "Nat" - the inductive type
        step_var: String, // "k" - the predecessor variable (for IH matching)
    },

    /// Logic: generic structural induction over ANY inductive type. Generalizes
    /// [`InferenceRule::StructuralInduction`] (fixed to the nullary-base + unary-step
    /// Nat shape) to an arbitrary constructor set — one premise per constructor, in
    /// registration order, each recursive argument carrying its own induction
    /// hypothesis. Certifies to a `Fix` over an N-ary `Match`: the dependent
    /// eliminator the kernel re-checks for coverage, case types, and termination.
    InductionScheme {
        variable: String,          // the induction variable, e.g. "t"
        ind_type: String,          // the inductive type, e.g. "Tree"
        cases: Vec<InductionCase>, // one per constructor, in registration order
    },

    // --- Linear arithmetic (order) ---
    /// `a ≤ b`, `b ≤ c` ⊢ `a ≤ c` over `Int`. The middle term is recovered from
    /// the first premise's conclusion. Certifies to `le_trans a b c p₀ p₁`.
    /// Inequalities are encoded as the Prop `Eq Bool (le a b) true`.
    LeTrans,

    /// `⊢ a ≤ a` over `Int`. Certifies to `le_refl a`.
    LeRefl,

    /// `a ≤ b`, `c ≤ d` ⊢ `a + c ≤ b + d` over `Int`. The four operands are read
    /// from the conclusion `le(add a c, add b d) = true`; `premise[0]` proves
    /// `a ≤ b`, `premise[1]` proves `c ≤ d`. Certifies to `le_add_mono a b c d p₀ p₁`.
    LeAddMono,

    /// Linear contradiction: `premise[0]` proves `le(m, n) = true` for ground `m > n`
    /// (so `le m n ⇝ false`, the Prop is `Eq Bool false true`). Concludes `⊥` via the
    /// `Bool` no-confusion discriminator. Lets contradictory bounds prove anything.
    LinFalse,

    /// `0 ≤ k`, `a ≤ b` ⊢ `k·a ≤ k·b` — scale an inequality by a non-negative `k`.
    /// Operands from the conclusion `le(mul k a, mul k b) = true`; `premise[0]` proves
    /// `0 ≤ k`, `premise[1]` proves `a ≤ b`. Certifies to `le_mul_nonneg k a b p₀ p₁`.
    /// A Farkas-reconstruction primitive.
    LeMulNonneg,

    /// `a ≤ b` ⊢ `0 ≤ b - a` — move an inequality to one side. Operands from the
    /// conclusion `le(0, sub b a) = true`; `premise[0]` proves `a ≤ b`. Certifies to
    /// `le_sub a b p₀`. The Farkas "collect to a single side" primitive.
    LeSub,

    /// `a < b` ⊢ `(a + 1) ≤ b` — integer DISCRETENESS. Operands from the conclusion
    /// `le(add a 1, b) = true`; `premise[0]` proves `a < b` (`lt(a,b) = true`).
    /// Certifies to `lt_succ_le a b p₀`. This is the one step rational Fourier-Motzkin
    /// lacks — the `omega` primitive that refutes strict systems the rational solver
    /// reports satisfiable.
    LtSuccLe,

    /// `a < (b + 1)` ⊢ `a ≤ b` — the upper-side discreteness companion. Operands
    /// from the conclusion `le(a, b) = true`; `premise[0]` proves `a < b+1`
    /// (`lt(a, add b 1) = true`). Certifies to `lt_add1_le a b p₀`. Preferred over
    /// `LtSuccLe` when the strict bound is already `b+1`, since it cancels the
    /// constant instead of propagating it into the Farkas reconstruction.
    LtAdd1Le,

    /// `(a + 1) ≤ b` ⊢ `a < b` — the CONVERSE of [`InferenceRule::LtSuccLe`], and the only
    /// way to CONCLUDE a strict inequality about symbolic operands. Operands are read from the
    /// conclusion `lt(a, b) = true`; `premise[0]` proves `le(add a 1, b) = true`. Certifies to
    /// `le_succ_lt a b p₀`.
    ///
    /// **It needs a prelude axiom that is not registered yet.** Of the `Int` order axioms in
    /// the trusted base, NONE concludes a `lt`: `lt_succ_le` and `lt_add1_le` both take one as
    /// a PREMISE, so a relation chain can compose a strict step but can only state its result
    /// in the tightened form `a + 1 ≤ d`. Adding `le_succ_lt : Π a b. (a + 1) ≤ b → a < b`
    /// closes that, and it is a ledger decision (`tcb_ledger_lock`'s `DEFINABLE_CEILING`), not
    /// a tactic one — Lean PROVES this, because its `Int` is an inductive.
    ///
    /// The strictly better move, which costs no axiom at all and REMOVES two, is to make `lt`
    /// a DEFINITION (`lt a b := le (add a 1) b`) instead of an opaque primitive: `lt_succ_le`
    /// and `lt_add1_le` then hold by conversion and leave the ledger, and `a < b` becomes
    /// definitionally `a + 1 ≤ b`, so this rule is not needed either.
    ///
    /// Until one of those lands, nothing CONSTRUCTS this rule: every producer asks
    /// `engine::strict_conclusions_available` first, so no tree is ever built that the
    /// kernel would have to refuse.
    LeSuccLt,

    /// `⊢ (a ≤ b) ∨ (b ≤ a)` over `Int` — LINEAR-ORDER TOTALITY, a leaf (no premises).
    /// Certifies to `le_total a b`, whose operands are read from the conclusion
    /// `Or (le(a,b) = true) (le(b,a) = true)`.
    ///
    /// This is the GOAL-DIRECTED half of `omega`. A refutation engine only ever proves `⊥`;
    /// a decision procedure proves the goal, and the way it does that is to assume the goal
    /// false and refute. Totality is the sound, axiom-free approximation of that assumption
    /// for a `≤` goal: split on `le_total a b`, take the goal itself in the left branch, and
    /// refute the hypotheses together with `b ≤ a` in the right. It reaches every goal whose
    /// negation-by-totality is refutable; the strictly larger fragment (`¬(a ≤ b) ⟹ b+1 ≤ a`)
    /// needs an axiom the trusted base does not have, and is deliberately NOT assumed here.
    LeTotal,

    // --- Equality ---
    /// Leibniz's Law / Substitution of Equals
    /// Logic: a = b, P(a) ⊢ P(b)
    /// The equality proof is in `premise\[0\]`, the P(a) proof is in `premise\[1\]`.
    /// Carries the original term and replacement term for certification.
    Rewrite { from: ProofTerm, to: ProofTerm },

    /// POSITIONAL Leibniz: like [`InferenceRule::Rewrite`], but abstracting exactly ONE
    /// occurrence of `to` in the conclusion — the `occurrence`-th, counted left-to-right in
    /// the same pre-order walk the tactic layer uses.
    ///
    /// `Rewrite` carries no position, and the `Eq_rec` motive it builds abstracts EVERY
    /// occurrence, so `P(a, a)` rewritten by `a = b` could only ever become `P(b, b)`. That
    /// is why Lean has `conv` and occurrence arguments at all: `P(b, a)` is a different (and
    /// frequently the only provable) goal. A caller that rewrote one occurrence had nowhere
    /// to record which, so the trust door could not follow it.
    ///
    /// The kernel is the check on the index: `Eq_rec` demands the source premise prove
    /// `P from`, so an index that abstracted the wrong occurrence yields a motive the source
    /// subgoal does not inhabit and the proof is REFUSED. A wrong position cannot be unsound,
    /// only unprovable.
    RewriteAt {
        from: ProofTerm,
        to: ProofTerm,
        occurrence: usize,
    },

    /// Symmetry of Equality: a = b ⊢ b = a
    EqualitySymmetry,

    /// Transitivity of Equality: a = b, b = c ⊢ a = c
    EqualityTransitivity,

    /// Reflexivity of Equality: a = a (after normalization)
    /// Used when both sides of an identity reduce to the same normal form.
    Reflexivity,

    /// Arithmetic decision: an `Int` equality discharged by the proof-producing
    /// arithmetic oracle ([`crate::arith::prove_int_eq`]) into a kernel-checked
    /// proof (computation + the ring axioms). The conclusion is the `Identity`.
    ArithDecision,

    /// Proof by kernel evaluation: a closed decidable proposition (a ground
    /// comparison or Bool/Nat equality) discharged via `native_decide`. The
    /// leaf carries only the claim; certification re-runs the evaluator and
    /// the kernel checks the resulting `of_decide_eq_true`/`ofReduceBool`
    /// term, so a lying leaf is rejected.
    NativeDecide,

    /// Proof by ground REFUTATION: the conclusion is `¬P` for a closed `P` that COMPUTES
    /// FALSE. A leaf — like [`InferenceRule::NativeDecide`], it carries only the claim, and
    /// certification re-runs the computation, so a lying leaf certifies to nothing.
    ///
    /// The dual `NativeDecide` lacks is the reason `decide` had no `ProofExpr::Not` arm at
    /// all: a `Decidable` instance decides `P` true through `of_decide_eq_true`, and the
    /// false direction needs `of_decide_eq_false`, which the trusted base does not register.
    /// No axiom is needed, because `Bool` is an inductive with two constructors and its
    /// no-confusion discriminator is already how a ground-false inequality becomes `⊥`
    /// ([`InferenceRule::LinFalse`]): a ground `Int` disequality transports `le a a` (true by
    /// computation) along the assumed equality into `le a b` (false by computation) with
    /// `Eq_rec`, and a ground `Bool` disequality is that contradiction already.
    GroundRefutation,

    // --- Fallbacks ---
    /// "The User Said So." Used for top-level axioms.
    Axiom,

    /// "The Machine Said So." (Z3 Oracle)
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

    /// Case Analysis on a formula `C` whose two cases both reach absurdity.
    /// Logic: (C → ⊥), (¬C → ⊥) ⊢ ⊥ — note this is the *intuitionistic* form
    /// (build `¬C` and `¬¬C`, then apply), so certifying it needs no excluded
    /// middle. Used for self-referential paradoxes like the Barber Paradox.
    ///
    /// `case_formula` carries the actual proposition `C` (not a rendered string)
    /// so the certifier can build the case lambdas' parameter types and bind `C`
    /// / `¬C` as local hypotheses in each branch.
    CaseAnalysis { case_formula: Box<ProofExpr> },

    /// Logic: `A ∨ B`, `A ⊢ C`, `B ⊢ C` ⊢ `C` — disjunction elimination to a common
    /// conclusion (here always `⊥`, for the grounded-grid contradiction prover).
    /// Premises: `[A∨B, left-branch (C assuming A), right-branch (C assuming B)]`.
    /// Unlike `DisjunctionElim` (disjunctive syllogism, which needs a refuted
    /// disjunct) this eliminates BOTH disjuncts by case analysis — the move a grid's
    /// of-pair / either-or / closure clause needs. The disjuncts (and, when a disjunct
    /// is a conjunction, each of its conjuncts) are bound as local hypotheses in the
    /// respective branch, so a branch may reference them directly.
    DisjunctionCases,
}

impl InferenceRule {
    /// Why [`certifier::certify`] cannot turn this rule into a kernel term —
    /// `None` when it can.
    ///
    /// **This is the list a proof PRODUCER must consult before it emits a tree.**
    /// A [`DerivationTree`] is an ordinary Rust value: it can be built with any
    /// variant, it passes every Rust type check, and a rule with no certifier arm
    /// fails only at [`verify::check_derivation`] — surfacing as a
    /// `verification_error` that reads like a kernel limitation rather than what
    /// it is, a tree that was never certifiable. Selecting a rule by NAME is how
    /// that happens: `TemporalTransitivity` looks exactly right for `t₁ < t₂`,
    /// `t₂ < t₃` ⊢ `t₁ < t₃`, and it is dead.
    ///
    /// Existence in this enum is not evidence of implementation. The match below
    /// is EXHAUSTIVE and so is `certify`'s — adding a variant without deciding
    /// both is a compile error, which is the only thing that keeps the enum and
    /// the certifier from drifting apart.
    pub fn uncertifiable_reason(&self) -> Option<&'static str> {
        match self {
            // --- Modal and temporal: DECLARED, never implemented. -------------
            // Nothing in any engine constructs these, `certify` has no arm for
            // any of them, and each names a proof shape with a perfectly good
            // translation into live rules. Each reason states that translation,
            // so a producer that reached for the name can rebuild the tree.
            InferenceRule::ModalAccess => Some(
                "no certifier arm: modal necessity-elimination is unimplemented. Express \
                 `□P` as `∀v. Accessible(w, v) → P(v)` and use UniversalInst + ModusPonens",
            ),
            InferenceRule::ModalGeneralization => Some(
                "no certifier arm: modal necessity-introduction is unimplemented. Express \
                 `□P` as `∀v. Accessible(w, v) → P(v)` and use UniversalIntro",
            ),
            InferenceRule::TemporalTransitivity => Some(
                "no certifier arm: temporal-order transitivity is unimplemented. A time \
                 order is an ordinary order — use LeTrans on `≤`, or the `<` chain it \
                 tightens to",
            ),
            InferenceRule::TemporalInduction => Some(
                "no certifier arm: k-induction over a trace is unimplemented. Induct on \
                 the trace index with StructuralInduction / InductionScheme",
            ),
            InferenceRule::TemporalUnfolding => Some(
                "no certifier arm: fixpoint unfolding of `Always` is unimplemented. \
                 Express `G P` as `∀n. P n` and use ConjunctionIntro over its \
                 instantiations",
            ),
            InferenceRule::EventualityProgress => Some(
                "no certifier arm: `Eventually` by witness is unimplemented. Express \
                 `F P` as `∃n. P n` and use ExistentialIntro",
            ),
            InferenceRule::UntilInduction => Some(
                "no certifier arm: induction for `P U Q` is unimplemented. Express it as \
                 `∃k. Q k ∧ ∀j < k. P j` and use ExistentialIntro + ConjunctionIntro",
            ),

            // --- Deliberately outside the trusted base. ----------------------
            InferenceRule::OracleVerification(_) => Some(
                "refused by design: an SMT oracle attests satisfiability but hands back no \
                 checkable proof term, so it can never become a kernel certificate. For \
                 arithmetic, ArithDecision is the proof-producing route",
            ),

            // --- Everything below has a real arm in `certifier::certify`. ----
            InferenceRule::PremiseMatch
            | InferenceRule::ModusPonens
            | InferenceRule::ModusTollens
            | InferenceRule::ConjunctionIntro
            | InferenceRule::ConjunctionElim
            | InferenceRule::DisjunctionIntro
            | InferenceRule::DisjunctionElim
            | InferenceRule::ExFalso
            | InferenceRule::ImpliesIntro
            | InferenceRule::BicondIntro
            | InferenceRule::DoubleNegation
            | InferenceRule::ClassicalReductio
            | InferenceRule::UniversalInst(_)
            | InferenceRule::UniversalInstTerm(_)
            | InferenceRule::UniversalIntro { .. }
            | InferenceRule::ExistentialIntro { .. }
            | InferenceRule::ExistentialIntroTerm { .. }
            | InferenceRule::StructuralInduction { .. }
            | InferenceRule::InductionScheme { .. }
            | InferenceRule::LeTrans
            | InferenceRule::LeRefl
            | InferenceRule::LeAddMono
            | InferenceRule::LinFalse
            | InferenceRule::LeMulNonneg
            | InferenceRule::LeSub
            | InferenceRule::LtSuccLe
            | InferenceRule::LtAdd1Le
            | InferenceRule::LeSuccLt
            | InferenceRule::LeTotal
            | InferenceRule::Rewrite { .. }
            | InferenceRule::RewriteAt { .. }
            | InferenceRule::EqualitySymmetry
            | InferenceRule::EqualityTransitivity
            | InferenceRule::Reflexivity
            | InferenceRule::ArithDecision
            | InferenceRule::NativeDecide
            | InferenceRule::GroundRefutation
            | InferenceRule::Axiom
            | InferenceRule::ReductioAdAbsurdum
            | InferenceRule::Contradiction
            | InferenceRule::ExistentialElim { .. }
            | InferenceRule::CaseAnalysis { .. }
            | InferenceRule::DisjunctionCases => None,
        }
    }

    /// Rewrite every NAME, TERM and PROPOSITION this rule carries in its PAYLOAD.
    ///
    /// Two places in this crate rename a rigid eigen-CONSTANT back to the variable it stands
    /// for, and a renaming that reaches a node's conclusion but not the payload of the rule
    /// that justifies it produces a tree whose motive, witness or case formula is about a name
    /// the conclusion no longer mentions — the certifier then computes against one proposition
    /// and checks another, and refuses a correct proof. Each of the two walkers had its own
    /// `match` with an `other => other.clone()` catch-all, and each caught a DIFFERENT subset:
    /// `engine::rewrite_tree_const_to_var` covered `UniversalInst`, `ExistentialIntro`,
    /// `ExistentialElim`, `Rewrite` and `CaseAnalysis`; `tactic::eigen_to_var_tree` covered
    /// `Rewrite` alone. NEITHER covered `RewriteAt`, `UniversalInstTerm` or
    /// `ExistentialIntroTerm` — the three payload-carrying rules added after they were written,
    /// which is exactly what a catch-all guarantees will happen again.
    ///
    /// This is that decision, once, and the match is EXHAUSTIVE: a rule variant added later
    /// cannot silently keep a stale name in its payload, because adding one without deciding
    /// its arm here is a compile error.
    ///
    /// Binder names a rule INTRODUCES (`UniversalIntro`'s variable, `StructuralInduction`'s
    /// induction and step variables, `InductionScheme`'s case arguments) are deliberately left
    /// alone: they are not references to anything outside the node, and renaming one would
    /// change which variable the eliminator abstracts.
    pub fn map_payload(
        &self,
        name: &dyn Fn(&str) -> String,
        term: &dyn Fn(&ProofTerm) -> ProofTerm,
        expr: &dyn Fn(&ProofExpr) -> ProofExpr,
    ) -> InferenceRule {
        match self {
            // --- Payloads that NAME a term the surrounding proof also mentions. ----------
            InferenceRule::UniversalInst(witness) => InferenceRule::UniversalInst(name(witness)),
            InferenceRule::ExistentialIntro { witness, witness_type } => {
                InferenceRule::ExistentialIntro {
                    witness: name(witness),
                    witness_type: witness_type.clone(),
                }
            }
            InferenceRule::ExistentialElim { witness } => {
                InferenceRule::ExistentialElim { witness: name(witness) }
            }

            // --- Payloads that ARE terms. ------------------------------------------------
            InferenceRule::UniversalInstTerm(witness) => {
                InferenceRule::UniversalInstTerm(term(witness))
            }
            InferenceRule::ExistentialIntroTerm { witness, witness_type } => {
                InferenceRule::ExistentialIntroTerm {
                    witness: term(witness),
                    witness_type: witness_type.clone(),
                }
            }
            InferenceRule::Rewrite { from, to } => {
                InferenceRule::Rewrite { from: term(from), to: term(to) }
            }
            InferenceRule::RewriteAt { from, to, occurrence } => InferenceRule::RewriteAt {
                from: term(from),
                to: term(to),
                occurrence: *occurrence,
            },

            // --- The one payload that is a PROPOSITION. ----------------------------------
            InferenceRule::CaseAnalysis { case_formula } => {
                InferenceRule::CaseAnalysis { case_formula: Box::new(expr(case_formula)) }
            }

            // --- Binders the rule introduces, type NAMES, and the oracle's justification
            //     string: nothing here refers outward, so nothing here is renamed.
            InferenceRule::UniversalIntro { .. }
            | InferenceRule::StructuralInduction { .. }
            | InferenceRule::InductionScheme { .. }
            | InferenceRule::OracleVerification(_)
            // --- …and every payload-free rule.
            | InferenceRule::PremiseMatch
            | InferenceRule::ModusPonens
            | InferenceRule::ModusTollens
            | InferenceRule::ConjunctionIntro
            | InferenceRule::ConjunctionElim
            | InferenceRule::DisjunctionIntro
            | InferenceRule::DisjunctionElim
            | InferenceRule::ExFalso
            | InferenceRule::ImpliesIntro
            | InferenceRule::BicondIntro
            | InferenceRule::DoubleNegation
            | InferenceRule::ClassicalReductio
            | InferenceRule::ModalAccess
            | InferenceRule::ModalGeneralization
            | InferenceRule::TemporalTransitivity
            | InferenceRule::TemporalInduction
            | InferenceRule::TemporalUnfolding
            | InferenceRule::EventualityProgress
            | InferenceRule::UntilInduction
            | InferenceRule::LeTrans
            | InferenceRule::LeRefl
            | InferenceRule::LeAddMono
            | InferenceRule::LinFalse
            | InferenceRule::LeMulNonneg
            | InferenceRule::LeSub
            | InferenceRule::LtSuccLe
            | InferenceRule::LtAdd1Le
            | InferenceRule::LeSuccLt
            | InferenceRule::LeTotal
            | InferenceRule::EqualitySymmetry
            | InferenceRule::EqualityTransitivity
            | InferenceRule::Reflexivity
            | InferenceRule::ArithDecision
            | InferenceRule::NativeDecide
            | InferenceRule::GroundRefutation
            | InferenceRule::Axiom
            | InferenceRule::ReductioAdAbsurdum
            | InferenceRule::Contradiction
            | InferenceRule::DisjunctionCases => self.clone(),
        }
    }

    /// Every rule that can be applied from a NAME alone — the payload-free variants, whose
    /// application is fixed by the goal and the facts in scope.
    ///
    /// A rule carrying an argument is deliberately absent: `Rewrite` needs its equation,
    /// `UniversalInstTerm` its witness, `StructuralInduction` its variable, and a name cannot
    /// supply any of them. Those have their own tactics (`rewrite`, `exists`, `induction`),
    /// and reporting the absence is more useful than guessing the argument.
    ///
    /// The seven declared-but-unimplemented rules ARE here. `by_rule` looks them up, finds
    /// [`InferenceRule::uncertifiable_reason`] and reports that reason — which is the whole
    /// point of the list: a producer that reached for a dead name learns it is dead, instead
    /// of watching a search fail for no stated cause.
    pub const NAMEABLE: &'static [InferenceRule] = &[
        InferenceRule::PremiseMatch,
        InferenceRule::ModusPonens,
        InferenceRule::ModusTollens,
        InferenceRule::ConjunctionIntro,
        InferenceRule::ConjunctionElim,
        InferenceRule::DisjunctionIntro,
        InferenceRule::DisjunctionElim,
        InferenceRule::DisjunctionCases,
        InferenceRule::ExFalso,
        InferenceRule::ImpliesIntro,
        InferenceRule::BicondIntro,
        InferenceRule::DoubleNegation,
        InferenceRule::ClassicalReductio,
        InferenceRule::ReductioAdAbsurdum,
        InferenceRule::Contradiction,
        InferenceRule::Axiom,
        InferenceRule::EqualitySymmetry,
        InferenceRule::EqualityTransitivity,
        InferenceRule::Reflexivity,
        InferenceRule::LeTrans,
        InferenceRule::LeRefl,
        InferenceRule::LeAddMono,
        InferenceRule::LeMulNonneg,
        InferenceRule::LeSub,
        InferenceRule::LinFalse,
        InferenceRule::LtSuccLe,
        InferenceRule::LtAdd1Le,
        InferenceRule::LeSuccLt,
        InferenceRule::LeTotal,
        InferenceRule::ArithDecision,
        InferenceRule::NativeDecide,
        InferenceRule::GroundRefutation,
        InferenceRule::ModalAccess,
        InferenceRule::ModalGeneralization,
        InferenceRule::TemporalTransitivity,
        InferenceRule::TemporalInduction,
        InferenceRule::TemporalUnfolding,
        InferenceRule::EventualityProgress,
        InferenceRule::UntilInduction,
    ];

    /// Resolve a rule NAME, as a proof script or a translator spells it: the variant's own
    /// name (`ModusPonens`) or its snake_case form (`modus_ponens`). Nothing else — a
    /// case-insensitive or punctuation-insensitive match would quietly accept `MODUSPONENS`
    /// and make a typo indistinguishable from a rule that does not exist.
    pub fn from_name(name: &str) -> Option<InferenceRule> {
        InferenceRule::NAMEABLE
            .iter()
            .find(|r| {
                let canonical = r.canonical_name();
                name == canonical || name == snake_case(&canonical)
            })
            .cloned()
    }

    /// This rule's canonical name. A payload-free variant renders as its bare variant name —
    /// the spelling [`InferenceRule::from_name`] accepts; a payload-carrying one renders with
    /// its payload, which is what a derivation dump wants and what `from_name` deliberately
    /// cannot read back.
    pub fn canonical_name(&self) -> String {
        format!("{self:?}")
    }

    /// Whether [`certifier::certify`] has an arm that builds a kernel term for
    /// this rule. The negation of [`InferenceRule::uncertifiable_reason`].
    pub fn is_certifiable(&self) -> bool {
        self.uncertifiable_reason().is_none()
    }
}

// =============================================================================
// DERIVATION TREE - The recursive proof structure
// =============================================================================

/// The "Euclidean Structure" - A recursive tree representing the proof.
///
/// This is the object returned by the Prover. It allows the UI to render
/// a step-by-step explanation (Natural Language Generation).
///
/// # Structure
///
/// Each node contains:
/// - `conclusion`: The proposition proved at this step
/// - `rule`: The logical rule applied (how we know the conclusion)
/// - `premises`: Sub-proofs justifying the rule's requirements
/// - `substitution`: Variable bindings from unification
///
/// # See Also
///
/// * [`BackwardChainer::prove`] - Creates derivation trees
/// * [`InferenceRule`] - The rules that justify each step
/// * [`ProofExpr`] - The conclusion type
/// * [`certifier::certify`] - Converts trees to kernel terms
/// * [`hints::suggest_hint`] - Suggests next steps when stuck
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

    /// The bindings unification discovered at this step — SEARCH TELEMETRY, not part of the
    /// claim.
    ///
    /// Ten sites in [`BackwardChainer`] write it and NOTHING in the repository reads it: not
    /// [`certifier::certify`], not [`verify::check_derivation`], not the derivation-document writer. Only the two
    /// eigenconstant-renaming walkers touch it, and they only carry it along. So a rule that
    /// recorded a substitution INSTEAD of instantiating its own conclusion would be believed by
    /// nobody and checked by nobody — the certified term would prove one proposition while the
    /// node claimed another, and every rule that computes from its own conclusion (a motive, a
    /// rewrite's endpoints) would compute from the stale one.
    ///
    /// What keeps that from happening is an invariant, not a checker: **a node's conclusion is a
    /// fixed point of its own substitution.** Every writer either applies it to the conclusion
    /// first or binds only variables the conclusion does not carry. It is locked by
    /// `T_tactics_extra::no_certified_node_records_a_substitution_its_conclusion_has_not_taken`,
    /// which sweeps the chainer and checks it directly — keep that green, or give the field a
    /// reader.
    pub substitution: unify::Substitution,
}

impl DerivationTree {
    /// Constructs a new Proof Node.
    /// Automatically calculates depth based on children.
    pub fn new(conclusion: ProofExpr, rule: InferenceRule, premises: Vec<DerivationTree>) -> Self {
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

    /// Every node of this tree whose rule [`certifier::certify`] cannot build, as
    /// `(rule, why)` pairs in pre-order — empty when the whole tree is certifiable.
    ///
    /// **A proof PRODUCER runs this before handing the tree to
    /// [`verify::check_derivation`].** A `DerivationTree` is an ordinary Rust value:
    /// any rule can be put in any node, the result type-checks, and a dead rule is
    /// discovered only when certification reaches it — as a `verification_error`
    /// that reads like the kernel refusing a proof. Asking here instead makes the
    /// answer structural and keeps it where the producer can act on it: rebuild the
    /// node out of live rules, or reject the input.
    ///
    /// The dead rule is usually not at the root. A root of `ExFalso` over a subtree
    /// justified by `TemporalUnfolding` is well-formed everywhere except the one
    /// node that matters, which is why this walks the whole tree rather than
    /// answering about `self.rule`.
    pub fn uncertifiable_rules(&self) -> Vec<(String, &'static str)> {
        let mut out = Vec::new();
        self.collect_uncertifiable(&mut out);
        out
    }

    fn collect_uncertifiable(&self, out: &mut Vec<(String, &'static str)>) {
        if let Some(why) = self.rule.uncertifiable_reason() {
            out.push((format!("{:?}", self.rule), why));
        }
        for premise in &self.premises {
            premise.collect_uncertifiable(out);
        }
    }

    /// Renders the proof as a text-based tree (for debugging/CLI).
    pub fn display_tree(&self) -> String {
        self.display_recursive(0)
    }

    fn display_recursive(&self, indent: usize) -> String {
        let prefix = "  ".repeat(indent);

        let rule_name = match &self.rule {
            InferenceRule::UniversalInst(var) => format!("UniversalInst({})", var),
            InferenceRule::UniversalInstTerm(term) => format!("UniversalInst({})", term),
            InferenceRule::UniversalIntro { variable, var_type } => {
                format!("UniversalIntro({}:{})", variable, var_type)
            }
            InferenceRule::ExistentialIntro {
                witness,
                witness_type,
            } => {
                format!("∃Intro({}:{})", witness, witness_type)
            }
            InferenceRule::ExistentialIntroTerm {
                witness,
                witness_type,
            } => {
                format!("∃Intro({}:{})", witness, witness_type)
            }
            InferenceRule::StructuralInduction {
                variable,
                ind_type,
                step_var,
            } => {
                format!("Induction({}:{}, step={})", variable, ind_type, step_var)
            }
            InferenceRule::OracleVerification(s) => format!("Oracle({})", s),
            InferenceRule::Rewrite { from, to } => format!("Rewrite({} → {})", from, to),
            InferenceRule::RewriteAt {
                from,
                to,
                occurrence,
            } => {
                format!("Rewrite({} → {} @{})", from, to, occurrence)
            }
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
///
/// Represents a "hole" in the proof that needs to be filled. During backward
/// chaining, goals are decomposed into subgoals until they match known facts.
///
/// # Fields
///
/// * `target` - What we are trying to prove
/// * `context` - Local assumptions (e.g., antecedents of implications we've entered)
///
/// # See Also
///
/// * [`BackwardChainer::prove`] - Takes a goal and produces a derivation tree
/// * [`DerivationTree`] - The result of successfully proving a goal
/// * [`ProofExpr`] - The target type
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
