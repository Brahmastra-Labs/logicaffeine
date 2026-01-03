use crate::arena::Arena;
use crate::intern::Symbol;
use crate::lexicon::Definiteness;
use crate::token::TokenType;

// ═══════════════════════════════════════════════════════════════════
// Semantic Types (Montague Grammar)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalType {
    Entity,      // e (individuals: John, the ball)
    TruthValue,  // t (propositions)
    Property,    // <e,t> (predicates: Unicorn, Water)
    Quantifier,  // <<e,t>,t> (every man, a woman)
}

// ═══════════════════════════════════════════════════════════════════
// Degree Semantics (Prover-Ready Number System)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Length,
    Time,
    Weight,
    Temperature,
    Cardinality,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NumberKind {
    Real(f64),
    Integer(i64),
    Symbolic(Symbol),
}

// ═══════════════════════════════════════════════════════════════════
// First-Order Logic Types (FOL Upgrade)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy)]
pub enum Term<'a> {
    Constant(Symbol),
    Variable(Symbol),
    Function(Symbol, &'a [Term<'a>]),
    Group(&'a [Term<'a>]),
    Possessed { possessor: &'a Term<'a>, possessed: Symbol },
    Sigma(Symbol),
    Intension(Symbol),  // ^Predicate (Montague up-arrow for de dicto)
    Proposition(&'a LogicExpr<'a>),  // Sentential complement (embedded clause)
    Value {
        kind: NumberKind,
        unit: Option<Symbol>,
        dimension: Option<Dimension>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantifierKind {
    Universal,
    Existential,
    Most,
    Few,
    Many,
    Cardinal(u32),
    AtLeast(u32),
    AtMost(u32),
    Generic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOpKind {
    And,
    Or,
    Implies,
    Iff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOpKind {
    Not,
}

// ═══════════════════════════════════════════════════════════════════
// Temporal & Aspect Operators (Arthur Prior's Tense Logic)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemporalOperator {
    Past,
    Future,
}

// ═══════════════════════════════════════════════════════════════════
// Event Semantics (Neo-Davidsonian)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThematicRole {
    Agent,
    Patient,
    Theme,
    Recipient,
    Goal,
    Source,
    Instrument,
    Location,
    Time,
    Manner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AspectOperator {
    Progressive,
    Perfect,
    Habitual,
    Iterative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceOperator {
    Passive,
}

// ═══════════════════════════════════════════════════════════════════
// Legacy Types (kept during transition)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug)]
pub struct NounPhrase<'a> {
    pub definiteness: Option<Definiteness>,
    pub adjectives: &'a [Symbol],
    pub noun: Symbol,
    pub possessor: Option<&'a NounPhrase<'a>>,
    pub pps: &'a [&'a LogicExpr<'a>],
    pub superlative: Option<Symbol>,
}

// ═══════════════════════════════════════════════════════════════════
// Boxed Variant Data (keeps LogicExpr enum small)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug)]
pub struct CategoricalData<'a> {
    pub quantifier: TokenType,
    pub subject: NounPhrase<'a>,
    pub copula_negative: bool,
    pub predicate: NounPhrase<'a>,
}

#[derive(Debug)]
pub struct RelationData<'a> {
    pub subject: NounPhrase<'a>,
    pub verb: Symbol,
    pub object: NounPhrase<'a>,
}

#[derive(Debug)]
pub struct NeoEventData<'a> {
    pub event_var: Symbol,
    pub verb: Symbol,
    pub roles: &'a [(ThematicRole, Term<'a>)],
    pub modifiers: &'a [Symbol],
    /// When true, suppress local ∃e quantification (DRT: event var will be bound by outer ∀)
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModalDomain {
    Alethic,
    Deontic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalFlavor {
    /// Root modals (ability, obligation): can, must, should, shall, could, would
    /// These get NARROW scope (de re) - modal attaches to the predicate inside quantifier
    Root,
    /// Epistemic modals (possibility, deduction): might, may
    /// These get WIDE scope (de dicto) - modal wraps the entire quantifier
    Epistemic,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModalVector {
    pub domain: ModalDomain,
    pub force: f32,
    pub flavor: ModalFlavor,
}

// ═══════════════════════════════════════════════════════════════════
// Expression Enum (hybrid: old + new variants)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug)]
pub enum LogicExpr<'a> {
    Predicate {
        name: Symbol,
        args: &'a [Term<'a>],
        /// World argument for Kripke semantics. None = implicit actual world (w₀).
        world: Option<Symbol>,
    },

    Identity {
        left: &'a Term<'a>,
        right: &'a Term<'a>,
    },

    Metaphor {
        tenor: &'a Term<'a>,
        vehicle: &'a Term<'a>,
    },

    Quantifier {
        kind: QuantifierKind,
        variable: Symbol,
        body: &'a LogicExpr<'a>,
        island_id: u32,
    },

    Categorical(Box<CategoricalData<'a>>),

    Relation(Box<RelationData<'a>>),

    Modal {
        vector: ModalVector,
        operand: &'a LogicExpr<'a>,
    },

    Temporal {
        operator: TemporalOperator,
        body: &'a LogicExpr<'a>,
    },

    Aspectual {
        operator: AspectOperator,
        body: &'a LogicExpr<'a>,
    },

    Voice {
        operator: VoiceOperator,
        body: &'a LogicExpr<'a>,
    },

    BinaryOp {
        left: &'a LogicExpr<'a>,
        op: TokenType,
        right: &'a LogicExpr<'a>,
    },

    UnaryOp {
        op: TokenType,
        operand: &'a LogicExpr<'a>,
    },

    Question {
        wh_variable: Symbol,
        body: &'a LogicExpr<'a>,
    },

    YesNoQuestion {
        body: &'a LogicExpr<'a>,
    },

    Atom(Symbol),

    Lambda {
        variable: Symbol,
        body: &'a LogicExpr<'a>,
    },

    App {
        function: &'a LogicExpr<'a>,
        argument: &'a LogicExpr<'a>,
    },

    Intensional {
        operator: Symbol,
        content: &'a LogicExpr<'a>,
    },

    Event {
        predicate: &'a LogicExpr<'a>,
        adverbs: &'a [Symbol],
    },

    NeoEvent(Box<NeoEventData<'a>>),

    Imperative {
        action: &'a LogicExpr<'a>,
    },

    SpeechAct {
        performer: Symbol,
        act_type: Symbol,
        content: &'a LogicExpr<'a>,
    },

    Counterfactual {
        antecedent: &'a LogicExpr<'a>,
        consequent: &'a LogicExpr<'a>,
    },

    Causal {
        effect: &'a LogicExpr<'a>,
        cause: &'a LogicExpr<'a>,
    },

    Comparative {
        adjective: Symbol,
        subject: &'a Term<'a>,
        object: &'a Term<'a>,
        difference: Option<&'a Term<'a>>,
    },

    Superlative {
        adjective: Symbol,
        subject: &'a Term<'a>,
        domain: Symbol,
    },

    Scopal {
        operator: Symbol,
        body: &'a LogicExpr<'a>,
    },

    Control {
        verb: Symbol,
        subject: &'a Term<'a>,
        object: Option<&'a Term<'a>>,
        infinitive: &'a LogicExpr<'a>,
    },

    Presupposition {
        assertion: &'a LogicExpr<'a>,
        presupposition: &'a LogicExpr<'a>,
    },

    Focus {
        kind: crate::token::FocusKind,
        focused: &'a Term<'a>,
        scope: &'a LogicExpr<'a>,
    },

    TemporalAnchor {
        anchor: Symbol,
        body: &'a LogicExpr<'a>,
    },

    Distributive {
        predicate: &'a LogicExpr<'a>,
    },

    /// Group existential for collective readings of cardinals
    /// ∃g(Group(g) ∧ Count(g,n) ∧ ∀x(Member(x,g) → Restriction(x)) ∧ Body(g))
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
