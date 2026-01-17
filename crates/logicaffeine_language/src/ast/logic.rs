//! Logic expression AST types for first-order logic with modal and event extensions.
//!
//! This module defines the core logical expression types including:
//!
//! - **[`LogicExpr`]**: The main expression enum with all logical constructs
//! - **[`Term`]**: Terms (constants, variables, function applications)
//! - **[`NounPhrase`]**: Parsed noun phrase structure
//! - **Semantic types**: Montague-style type markers
//! - **Event roles**: Neo-Davidsonian thematic roles (Agent, Theme, Goal, etc.)
//! - **Modal vectors**: Kripke semantics parameters (domain, flavor, force)
//! - **Temporal operators**: Past, future, perfect, progressive
//!
//! All types use arena allocation with the `'a` lifetime parameter.

use logicaffeine_base::Arena;
use logicaffeine_base::Symbol;
use crate::lexicon::Definiteness;
use crate::token::TokenType;

// ═══════════════════════════════════════════════════════════════════
// Semantic Types (Montague Grammar)
// ═══════════════════════════════════════════════════════════════════

/// Montague semantic types for compositional interpretation.
///
/// These types classify expressions according to their denotation in
/// a model-theoretic semantics, following Montague's "Universal Grammar".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalType {
    /// Type `e`: Individuals (entities) like "John" or "the ball".
    Entity,
    /// Type `t`: Truth values (propositions) like "John runs".
    TruthValue,
    /// Type `<e,t>`: Properties (one-place predicates) like "is a unicorn".
    Property,
    /// Type `<<e,t>,t>`: Generalized quantifiers like "every man" or "a woman".
    Quantifier,
}

// ═══════════════════════════════════════════════════════════════════
// Degree Semantics (Prover-Ready Number System)
// ═══════════════════════════════════════════════════════════════════

/// Physical dimension for degree semantics and unit tracking.
///
/// Used with [`NumberKind`] to enable dimensional analysis and prevent
/// nonsensical comparisons (e.g., adding meters to seconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    /// Spatial extent (meters, feet, inches).
    Length,
    /// Temporal duration (seconds, minutes, hours).
    Time,
    /// Mass (kilograms, pounds).
    Weight,
    /// Thermal measure (Celsius, Fahrenheit, Kelvin).
    Temperature,
    /// Count of discrete items.
    Cardinality,
}

/// Numeric literal representation for degree semantics.
///
/// Supports exact integers, floating-point reals, and symbolic constants
/// (e.g., π, e) for prover integration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NumberKind {
    /// Floating-point real number (e.g., 3.14, 0.5).
    Real(f64),
    /// Exact integer (e.g., 42, -1, 0).
    Integer(i64),
    /// Symbolic constant (e.g., π, e, ∞).
    Symbolic(Symbol),
}

// ═══════════════════════════════════════════════════════════════════
// First-Order Logic Types (FOL Upgrade)
// ═══════════════════════════════════════════════════════════════════

/// First-order logic term representing entities or values.
///
/// Terms denote individuals, groups, or computed values in the domain
/// of discourse. They serve as arguments to predicates.
#[derive(Debug, Clone, Copy)]
pub enum Term<'a> {
    /// Named individual constant (e.g., `john`, `paris`).
    Constant(Symbol),
    /// Bound or free variable (e.g., `x`, `y`).
    Variable(Symbol),
    /// Function application: `f(t1, t2, ...)` (e.g., `mother(john)`).
    Function(Symbol, &'a [Term<'a>]),
    /// Plural group for collective readings (e.g., `john ⊕ mary`).
    Group(&'a [Term<'a>]),
    /// Possessive construction: `john's book` → `Poss(john, book)`.
    Possessed { possessor: &'a Term<'a>, possessed: Symbol },
    /// Definite description σ-term: `σx.P(x)` ("the unique x such that P").
    Sigma(Symbol),
    /// Intensional term (Montague up-arrow `^P`) for de dicto readings.
    Intension(Symbol),
    /// Sentential complement (embedded clause as propositional argument).
    Proposition(&'a LogicExpr<'a>),
    /// Numeric value with optional unit and dimension.
    Value {
        kind: NumberKind,
        unit: Option<Symbol>,
        dimension: Option<Dimension>,
    },
}

/// Quantifier types for first-order and generalized quantifiers.
///
/// Extends standard FOL with generalized quantifiers that cannot be
/// expressed with ∀ and ∃ alone (e.g., "most", "few", "at least 3").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantifierKind {
    /// Universal: ∀x ("every", "all", "each").
    Universal,
    /// Existential: ∃x ("some", "a", "an").
    Existential,
    /// Proportional: "most X are Y" (>50% of domain).
    Most,
    /// Proportional: "few X are Y" (<expected proportion).
    Few,
    /// Vague large quantity: "many X are Y".
    Many,
    /// Exact count: "exactly n X are Y".
    Cardinal(u32),
    /// Lower bound: "at least n X are Y".
    AtLeast(u32),
    /// Upper bound: "at most n X are Y".
    AtMost(u32),
    /// Generic: "cats meow" (characterizing generalization).
    Generic,
}

/// Binary logical connectives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOpKind {
    /// Conjunction: P ∧ Q.
    And,
    /// Disjunction: P ∨ Q.
    Or,
    /// Material implication: P → Q.
    Implies,
    /// Biconditional: P ↔ Q.
    Iff,
}

/// Unary logical operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOpKind {
    /// Negation: ¬P.
    Not,
}

// ═══════════════════════════════════════════════════════════════════
// Temporal & Aspect Operators (Arthur Prior's Tense Logic)
// ═══════════════════════════════════════════════════════════════════

/// Prior-style tense logic operators.
///
/// Based on Arthur Prior's tense logic where P ("it was the case that")
/// and F ("it will be the case that") are modal operators over time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemporalOperator {
    /// Past tense: P(φ) — "it was the case that φ".
    Past,
    /// Future tense: F(φ) — "it will be the case that φ".
    Future,
}

// ═══════════════════════════════════════════════════════════════════
// Event Semantics (Neo-Davidsonian)
// ═══════════════════════════════════════════════════════════════════

/// Neo-Davidsonian thematic roles for event semantics.
///
/// Following Parsons' neo-Davidsonian analysis, events are reified and
/// participants are related to events via thematic role predicates:
/// `∃e(Run(e) ∧ Agent(e, john) ∧ Location(e, park))`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThematicRole {
    /// Animate initiator of action (e.g., "John" in "John ran").
    Agent,
    /// Entity affected by action (e.g., "the window" in "broke the window").
    Patient,
    /// Entity involved without change (e.g., "the ball" in "saw the ball").
    Theme,
    /// Animate entity receiving something (e.g., "Mary" in "gave Mary a book").
    Recipient,
    /// Destination or endpoint (e.g., "Paris" in "went to Paris").
    Goal,
    /// Origin or starting point (e.g., "London" in "came from London").
    Source,
    /// Tool or means (e.g., "a knife" in "cut with a knife").
    Instrument,
    /// Spatial setting (e.g., "the park" in "ran in the park").
    Location,
    /// Temporal setting (e.g., "yesterday" in "arrived yesterday").
    Time,
    /// How action was performed (e.g., "quickly" in "ran quickly").
    Manner,
}

/// Grammatical aspect operators for event structure.
///
/// Aspect describes the internal temporal structure of events,
/// distinct from tense which locates events in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AspectOperator {
    /// Ongoing action: "is running" → PROG(Run(e)).
    Progressive,
    /// Completed with present relevance: "has eaten" → PERF(Eat(e)).
    Perfect,
    /// Characteristic pattern: "smokes" (habitually) → HAB(Smoke(e)).
    Habitual,
    /// Repeated action: "kept knocking" → ITER(Knock(e)).
    Iterative,
}

/// Grammatical voice operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceOperator {
    /// Passive voice: "was eaten" promotes patient to subject position.
    Passive,
}

// ═══════════════════════════════════════════════════════════════════
// Legacy Types (kept during transition)
// ═══════════════════════════════════════════════════════════════════

/// Parsed noun phrase structure for compositional interpretation.
///
/// Captures the internal structure of noun phrases including determiners,
/// modifiers, and possessives for correct semantic composition.
#[derive(Debug)]
pub struct NounPhrase<'a> {
    /// Definiteness: the (definite), a/an (indefinite), or bare (none).
    pub definiteness: Option<Definiteness>,
    /// Pre-nominal adjectives (e.g., "big red" in "big red ball").
    pub adjectives: &'a [Symbol],
    /// Head noun (e.g., "ball" in "big red ball").
    pub noun: Symbol,
    /// Possessor phrase (e.g., "John's" in "John's book").
    pub possessor: Option<&'a NounPhrase<'a>>,
    /// Prepositional phrase modifiers attached to noun.
    pub pps: &'a [&'a LogicExpr<'a>],
    /// Superlative adjective if present (e.g., "tallest").
    pub superlative: Option<Symbol>,
}

// ═══════════════════════════════════════════════════════════════════
// Boxed Variant Data (keeps LogicExpr enum small)
// ═══════════════════════════════════════════════════════════════════

/// Aristotelian categorical proposition data.
///
/// Represents the four categorical forms (A, E, I, O):
/// - A: All S are P
/// - E: No S are P
/// - I: Some S are P
/// - O: Some S are not P
#[derive(Debug)]
pub struct CategoricalData<'a> {
    /// The quantifier (All, No, Some).
    pub quantifier: TokenType,
    /// Subject term (S in "All S are P").
    pub subject: NounPhrase<'a>,
    /// Whether copula is negated (for O-form: "Some S are not P").
    pub copula_negative: bool,
    /// Predicate term (P in "All S are P").
    pub predicate: NounPhrase<'a>,
}

/// Simple subject-verb-object relation data.
#[derive(Debug)]
pub struct RelationData<'a> {
    /// Subject noun phrase.
    pub subject: NounPhrase<'a>,
    /// Verb predicate.
    pub verb: Symbol,
    /// Object noun phrase.
    pub object: NounPhrase<'a>,
}

/// Neo-Davidsonian event structure with thematic roles.
///
/// Represents a verb event with its participants decomposed into
/// separate thematic role predicates: `∃e(Run(e) ∧ Agent(e, john))`.
#[derive(Debug)]
pub struct NeoEventData<'a> {
    /// The event variable (e, e1, e2, ...).
    pub event_var: Symbol,
    /// The verb predicate name.
    pub verb: Symbol,
    /// Thematic role assignments: (Role, Filler) pairs.
    pub roles: &'a [(ThematicRole, Term<'a>)],
    /// Adverbial modifiers (e.g., "quickly" → Quickly(e)).
    pub modifiers: &'a [Symbol],
    /// When true, suppress local ∃e quantification.
    /// Used in DRT for generic conditionals where event var is bound by outer ∀.
    pub suppress_existential: bool,
    /// World argument for Kripke semantics. None = implicit actual world (w₀).
    pub world: Option<Symbol>,
}

impl<'a> NounPhrase<'a> {
    pub fn simple(noun: Symbol) -> Self {
        NounPhrase {
            definiteness: None,
            adjectives: &[],
            noun,
            possessor: None,
            pps: &[],
            superlative: None,
        }
    }

    pub fn with_definiteness(definiteness: Definiteness, noun: Symbol) -> Self {
        NounPhrase {
            definiteness: Some(definiteness),
            adjectives: &[],
            noun,
            possessor: None,
            pps: &[],
            superlative: None,
        }
    }
}

/// Modal logic domain classification.
///
/// Determines the accessibility relation in Kripke semantics:
/// what kinds of possible worlds are relevant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModalDomain {
    /// Alethic modality: logical/metaphysical possibility and necessity.
    /// "It is possible that P" = P holds in some accessible world.
    Alethic,
    /// Deontic modality: obligation and permission.
    /// "It is obligatory that P" = P holds in all deontically ideal worlds.
    Deontic,
}

/// Modal flavor affecting scope interpretation.
///
/// The distinction between root and epistemic modals affects
/// quantifier scope: root modals scope under quantifiers (de re),
/// while epistemic modals scope over quantifiers (de dicto).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalFlavor {
    /// Root modals express ability, obligation, or circumstantial possibility.
    /// Verbs: can, must, should, shall, could, would.
    /// Scope: NARROW (de re) — modal attaches inside quantifier scope.
    /// Example: "Every student can solve this" = ∀x(Student(x) → ◇Solve(x, this))
    Root,
    /// Epistemic modals express possibility or deduction based on evidence.
    /// Verbs: might, may (epistemic readings).
    /// Scope: WIDE (de dicto) — modal wraps the entire quantified formula.
    /// Example: "A student might win" = ◇∃x(Student(x) ∧ Win(x))
    Epistemic,
}

/// Modal operator parameters for Kripke semantics.
///
/// Combines domain (what kind of modality), force (necessity vs possibility),
/// and flavor (scope behavior) into a single modal specification.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModalVector {
    /// The modal domain: alethic or deontic.
    pub domain: ModalDomain,
    /// Modal force: 1.0 = necessity (□), 0.5 = possibility (◇), graded values between.
    pub force: f32,
    /// Scope flavor: root (narrow scope) or epistemic (wide scope).
    pub flavor: ModalFlavor,
}

// ═══════════════════════════════════════════════════════════════════
// Expression Enum (hybrid: old + new variants)
// ═══════════════════════════════════════════════════════════════════

/// First-order logic expression with modal, temporal, and event extensions.
///
/// This is the core AST type representing logical formulas. All nodes are
/// arena-allocated with the `'a` lifetime tracking the arena's scope.
///
/// # Categories
///
/// - **Core FOL**: [`Predicate`], [`Quantifier`], [`BinaryOp`], [`UnaryOp`], [`Identity`]
/// - **Lambda calculus**: [`Lambda`], [`App`], [`Atom`]
/// - **Modal logic**: [`Modal`], [`Intensional`]
/// - **Temporal/Aspect**: [`Temporal`], [`Aspectual`], [`Voice`]
/// - **Event semantics**: [`Event`], [`NeoEvent`]
/// - **Questions**: [`Question`], [`YesNoQuestion`]
/// - **Pragmatics**: [`SpeechAct`], [`Focus`], [`Presupposition`]
/// - **Comparison**: [`Comparative`], [`Superlative`]
/// - **Other**: [`Counterfactual`], [`Causal`], [`Control`], [`Imperative`]
///
/// [`Predicate`]: LogicExpr::Predicate
/// [`Quantifier`]: LogicExpr::Quantifier
/// [`BinaryOp`]: LogicExpr::BinaryOp
/// [`UnaryOp`]: LogicExpr::UnaryOp
/// [`Identity`]: LogicExpr::Identity
/// [`Lambda`]: LogicExpr::Lambda
/// [`App`]: LogicExpr::App
/// [`Atom`]: LogicExpr::Atom
/// [`Modal`]: LogicExpr::Modal
/// [`Intensional`]: LogicExpr::Intensional
/// [`Temporal`]: LogicExpr::Temporal
/// [`Aspectual`]: LogicExpr::Aspectual
/// [`Voice`]: LogicExpr::Voice
/// [`Event`]: LogicExpr::Event
/// [`NeoEvent`]: LogicExpr::NeoEvent
/// [`Question`]: LogicExpr::Question
/// [`YesNoQuestion`]: LogicExpr::YesNoQuestion
/// [`SpeechAct`]: LogicExpr::SpeechAct
/// [`Focus`]: LogicExpr::Focus
/// [`Presupposition`]: LogicExpr::Presupposition
/// [`Comparative`]: LogicExpr::Comparative
/// [`Superlative`]: LogicExpr::Superlative
/// [`Counterfactual`]: LogicExpr::Counterfactual
/// [`Causal`]: LogicExpr::Causal
/// [`Control`]: LogicExpr::Control
/// [`Imperative`]: LogicExpr::Imperative
#[derive(Debug)]
pub enum LogicExpr<'a> {
    /// Atomic predicate: `P(t1, t2, ...)` with optional world parameter.
    Predicate {
        name: Symbol,
        args: &'a [Term<'a>],
        /// World argument for Kripke semantics. None = implicit actual world (w₀).
        world: Option<Symbol>,
    },

    /// Identity statement: `t1 = t2`.
    Identity {
        left: &'a Term<'a>,
        right: &'a Term<'a>,
    },

    /// Metaphorical assertion: tenor "is" vehicle (non-literal identity).
    Metaphor {
        tenor: &'a Term<'a>,
        vehicle: &'a Term<'a>,
    },

    /// Quantified formula: `∀x.φ` or `∃x.φ` with scope island tracking.
    Quantifier {
        kind: QuantifierKind,
        variable: Symbol,
        body: &'a LogicExpr<'a>,
        /// Island ID prevents illicit scope interactions across syntactic boundaries.
        island_id: u32,
    },

    /// Aristotelian categorical proposition (boxed to keep enum small).
    Categorical(Box<CategoricalData<'a>>),

    /// Simple S-V-O relation (boxed).
    Relation(Box<RelationData<'a>>),

    /// Modal operator: □φ (necessity) or ◇φ (possibility).
    Modal {
        vector: ModalVector,
        operand: &'a LogicExpr<'a>,
    },

    /// Tense operator: PAST(φ) or FUTURE(φ).
    Temporal {
        operator: TemporalOperator,
        body: &'a LogicExpr<'a>,
    },

    /// Aspect operator: PROG(φ), PERF(φ), HAB(φ), ITER(φ).
    Aspectual {
        operator: AspectOperator,
        body: &'a LogicExpr<'a>,
    },

    /// Voice operator: PASSIVE(φ).
    Voice {
        operator: VoiceOperator,
        body: &'a LogicExpr<'a>,
    },

    /// Binary connective: φ ∧ ψ, φ ∨ ψ, φ → ψ, φ ↔ ψ.
    BinaryOp {
        left: &'a LogicExpr<'a>,
        op: TokenType,
        right: &'a LogicExpr<'a>,
    },

    /// Unary operator: ¬φ.
    UnaryOp {
        op: TokenType,
        operand: &'a LogicExpr<'a>,
    },

    /// Wh-question: λx.φ where x is the questioned variable.
    Question {
        wh_variable: Symbol,
        body: &'a LogicExpr<'a>,
    },

    /// Yes/no question: ?φ (is φ true?).
    YesNoQuestion {
        body: &'a LogicExpr<'a>,
    },

    /// Atomic symbol (variable or constant in lambda context).
    Atom(Symbol),

    /// Lambda abstraction: λx.φ.
    Lambda {
        variable: Symbol,
        body: &'a LogicExpr<'a>,
    },

    /// Function application: (φ)(ψ).
    App {
        function: &'a LogicExpr<'a>,
        argument: &'a LogicExpr<'a>,
    },

    /// Intensional context: `operator[content]` for opaque verbs (believes, seeks).
    Intensional {
        operator: Symbol,
        content: &'a LogicExpr<'a>,
    },

    /// Legacy event semantics (Davidson-style with adverb list).
    Event {
        predicate: &'a LogicExpr<'a>,
        adverbs: &'a [Symbol],
    },

    /// Neo-Davidsonian event with thematic roles (boxed).
    NeoEvent(Box<NeoEventData<'a>>),

    /// Imperative command: !φ.
    Imperative {
        action: &'a LogicExpr<'a>,
    },

    /// Speech act: performative utterance with illocutionary force.
    SpeechAct {
        performer: Symbol,
        act_type: Symbol,
        content: &'a LogicExpr<'a>,
    },

    /// Counterfactual conditional: "If P had been, Q would have been".
    Counterfactual {
        antecedent: &'a LogicExpr<'a>,
        consequent: &'a LogicExpr<'a>,
    },

    /// Causal relation: "effect because cause".
    Causal {
        effect: &'a LogicExpr<'a>,
        cause: &'a LogicExpr<'a>,
    },

    /// Comparative: "X is taller than Y (by 2 inches)".
    Comparative {
        adjective: Symbol,
        subject: &'a Term<'a>,
        object: &'a Term<'a>,
        difference: Option<&'a Term<'a>>,
    },

    /// Superlative: "X is the tallest among domain".
    Superlative {
        adjective: Symbol,
        subject: &'a Term<'a>,
        domain: Symbol,
    },

    /// Scopal adverb: "only", "always", etc. as operators.
    Scopal {
        operator: Symbol,
        body: &'a LogicExpr<'a>,
    },

    /// Control verb: "wants to VP", "persuaded X to VP".
    Control {
        verb: Symbol,
        subject: &'a Term<'a>,
        object: Option<&'a Term<'a>>,
        infinitive: &'a LogicExpr<'a>,
    },

    /// Presupposition-assertion structure.
    Presupposition {
        assertion: &'a LogicExpr<'a>,
        presupposition: &'a LogicExpr<'a>,
    },

    /// Focus particle: "only X", "even X" with alternative set.
    Focus {
        kind: crate::token::FocusKind,
        focused: &'a Term<'a>,
        scope: &'a LogicExpr<'a>,
    },

    /// Temporal anchor: "yesterday(φ)", "now(φ)".
    TemporalAnchor {
        anchor: Symbol,
        body: &'a LogicExpr<'a>,
    },

    /// Distributive operator: *P distributes P over group members.
    Distributive {
        predicate: &'a LogicExpr<'a>,
    },

    /// Group quantifier for collective cardinal readings.
    /// `∃g(Group(g) ∧ Count(g,n) ∧ ∀x(Member(x,g) → Restriction(x)) ∧ Body(g))`
    GroupQuantifier {
        group_var: Symbol,
        count: u32,
        member_var: Symbol,
        restriction: &'a LogicExpr<'a>,
        body: &'a LogicExpr<'a>,
    },
}

impl<'a> LogicExpr<'a> {
    pub fn lambda(var: Symbol, body: &'a LogicExpr<'a>, arena: &'a Arena<LogicExpr<'a>>) -> &'a LogicExpr<'a> {
        arena.alloc(LogicExpr::Lambda {
            variable: var,
            body,
        })
    }

    pub fn app(func: &'a LogicExpr<'a>, arg: &'a LogicExpr<'a>, arena: &'a Arena<LogicExpr<'a>>) -> &'a LogicExpr<'a> {
        arena.alloc(LogicExpr::App {
            function: func,
            argument: arg,
        })
    }
}

#[cfg(test)]
mod size_tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn test_ast_node_sizes() {
        println!("LogicExpr size: {} bytes", size_of::<LogicExpr>());
        println!("Term size: {} bytes", size_of::<Term>());
        println!("NounPhrase size: {} bytes", size_of::<NounPhrase>());

        assert!(
            size_of::<LogicExpr>() <= 48,
            "LogicExpr is {} bytes - consider boxing large variants",
            size_of::<LogicExpr>()
        );
        assert!(
            size_of::<Term>() <= 32,
            "Term is {} bytes",
            size_of::<Term>()
        );
    }
}
