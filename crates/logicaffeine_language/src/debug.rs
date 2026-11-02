//! Debug display utilities for AST types with interner access.
//!
//! This module provides the [`DisplayWith`] trait for formatting AST types that
//! contain interned symbols. Since symbols are just integer IDs, displaying them
//! requires access to the interner to resolve the actual strings.
//!
//! Use the `.with(interner)` method to create a displayable wrapper:
//!
//! ```
//! use logicaffeine_base::Interner;
//! use logicaffeine_language::ast::{LogicExpr, Term};
//! use logicaffeine_language::debug::DisplayWith;
//!
//! let mut interner = Interner::new();
//! let mortal = interner.intern("Mortal");
//! let socrates = interner.intern("Socrates");
//! let args = [Term::Constant(socrates)];
//! let expr = LogicExpr::Predicate { name: mortal, args: &args, world: None };
//! assert_eq!(expr.with(&interner).to_string(), "Mortal(Socrates)");
//! ```

use std::fmt;

use crate::ast::{
    AspectOperator, BinaryTemporalOp, LogicExpr, NounPhrase, QuantifierKind, TemporalOperator,
    Term, VoiceOperator,
};
use crate::token::TokenType;
use logicaffeine_base::{Interner, Symbol};

/// Trait for formatting types that require an interner for symbol resolution.
pub trait DisplayWith {
    fn fmt_with(&self, interner: &Interner, f: &mut fmt::Formatter<'_>) -> fmt::Result;

    fn with<'a>(&'a self, interner: &'a Interner) -> WithInterner<'a, Self> {
        WithInterner {
            target: self,
            interner,
        }
    }
}

pub struct WithInterner<'a, T: ?Sized> {
    pub target: &'a T,
    pub interner: &'a Interner,
}

impl<'a, T: DisplayWith> fmt::Display for WithInterner<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.target.fmt_with(self.interner, f)
    }
}

pub struct DebugWorld<'a, T: ?Sized> {
    pub target: &'a T,
    pub interner: &'a Interner,
}

impl<'a, T: DisplayWith> fmt::Debug for DebugWorld<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.target.fmt_with(self.interner, f)
    }
}

impl DisplayWith for Symbol {
    fn fmt_with(&self, interner: &Interner, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", interner.resolve(*self))
    }
}

impl<'a> DisplayWith for Term<'a> {
    fn fmt_with(&self, interner: &Interner, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Term::Constant(s) => write!(f, "{}", interner.resolve(*s)),
            Term::Variable(s) => write!(f, "{}", interner.resolve(*s)),
            Term::Function(name, args) => {
                write!(f, "{}(", interner.resolve(*name))?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    arg.fmt_with(interner, f)?;
                }
                write!(f, ")")
            }
            Term::Group(members) => {
                write!(f, "[")?;
                for (i, m) in members.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ⊕ ")?;
                    }
                    m.fmt_with(interner, f)?;
                }
                write!(f, "]")
            }
            Term::Possessed {
                possessor,
                possessed,
            } => {
                possessor.fmt_with(interner, f)?;
                write!(f, ".{}", interner.resolve(*possessed))
            }
            Term::Sigma(predicate) => {
                write!(f, "σx.{}(x)", interner.resolve(*predicate))
            }
            Term::Intension(predicate) => {
                write!(f, "^{}", interner.resolve(*predicate))
            }
            Term::Kind(kind) => {
                write!(f, "^{}", interner.resolve(*kind))
            }
            Term::Proposition(expr) => {
                write!(f, "[{}]", expr.with(interner))
            }
            Term::Value {
                kind,
                unit,
                dimension,
            } => {
                use crate::ast::NumberKind;
                match kind {
                    NumberKind::Real(r) => write!(f, "{}", r)?,
                    NumberKind::Integer(i) => write!(f, "{}", i)?,
                    NumberKind::Symbolic(s) => write!(f, "{}", interner.resolve(*s))?,
                }
                if let Some(u) = unit {
                    write!(f, " {}", interner.resolve(*u))?;
                }
                if let Some(d) = dimension {
                    write!(f, " [{:?}]", d)?;
                }
                Ok(())
            }
        }
    }
}

impl<'a> DisplayWith for NounPhrase<'a> {
    fn fmt_with(&self, interner: &Interner, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(def) = &self.definiteness {
            write!(f, "{:?} ", def)?;
        }
        for adj in self.adjectives {
            write!(f, "{} ", interner.resolve(*adj))?;
        }
        write!(f, "{}", interner.resolve(self.noun))
    }
}

// The notation's operator symbols — one table, read by every renderer (this Display, the
// Studio's syntax tree), so the two never spell one operator two ways.

/// A quantifier's symbol, with its count for the numeric ones (`∃=2`, `∃≥3`).
pub fn quantifier_symbol(kind: &QuantifierKind) -> String {
    match kind {
        QuantifierKind::Universal => "∀".to_string(),
        QuantifierKind::Existential => "∃".to_string(),
        QuantifierKind::Most => "MOST".to_string(),
        QuantifierKind::Few => "FEW".to_string(),
        QuantifierKind::Many => "MANY".to_string(),
        QuantifierKind::Generic => "Gen".to_string(),
        QuantifierKind::InfinitelyMany => "∃^∞".to_string(),
        QuantifierKind::FinitelyMany => "∃^<∞".to_string(),
        QuantifierKind::AllButFinitelyMany => "∀^∞".to_string(),
        QuantifierKind::Cardinal(n) => format!("∃={n}"),
        QuantifierKind::AtLeast(n) => format!("∃≥{n}"),
        QuantifierKind::AtMost(n) => format!("∃≤{n}"),
    }
}

/// A connective's symbol, from the one mapping every printer shares ([`Connective::of`](crate::formatter::Connective::of)).
pub fn connective_symbol(op: &TokenType) -> &'static str {
    use crate::formatter::Connective;
    match Connective::of(op) {
        Some(Connective::And) => "∧",
        Some(Connective::Or) => "∨",
        Some(Connective::Implies) => "→",
        Some(Connective::Iff) => "↔",
        Some(Connective::Not) => "¬",
        None => "?",
    }
}

/// A modal's operator. The boundary must match the formatter (formatter.rs) and the Kripke
/// lowering (semantics/kripke.rs): force == 0.5 (ability/epistemic can/could/may) is the
/// weak/possibility side (◇), not necessity.
pub fn modal_symbol(vector: &crate::ast::ModalVector) -> &'static str {
    match (vector.domain, vector.force > 0.5) {
        (crate::ast::ModalDomain::Alethic, true) => "□",
        (crate::ast::ModalDomain::Alethic, false) => "◇",
        (crate::ast::ModalDomain::Deontic, true) => "O",
        (crate::ast::ModalDomain::Deontic, false) => "P",
        (crate::ast::ModalDomain::Temporal, _) => "Temporal",
    }
}

/// A tense operator: `P` past, `F` future or eventually, `G` always, `X` next.
pub fn temporal_symbol(op: &TemporalOperator) -> &'static str {
    match op {
        TemporalOperator::Past => "P",
        TemporalOperator::Future => "F",
        TemporalOperator::Always => "G",
        TemporalOperator::Eventually | TemporalOperator::BoundedEventually(_) => "F",
        TemporalOperator::Next => "X",
    }
}

/// A binary temporal operator: until, release, weak until.
pub fn binary_temporal_symbol(op: &BinaryTemporalOp) -> &'static str {
    match op {
        BinaryTemporalOp::Until => "U",
        BinaryTemporalOp::Release => "R",
        BinaryTemporalOp::WeakUntil => "W",
    }
}

/// An aspect operator.
pub fn aspect_symbol(op: &AspectOperator) -> &'static str {
    match op {
        AspectOperator::Progressive => "PROG",
        AspectOperator::Perfect => "PERF",
        AspectOperator::Habitual => "HAB",
        AspectOperator::Iterative => "ITER",
    }
}

/// A voice operator.
pub fn voice_symbol(op: &VoiceOperator) -> &'static str {
    match op {
        VoiceOperator::Passive => "PASS",
    }
}

/// A focus particle.
pub fn focus_symbol(kind: &crate::token::FocusKind) -> &'static str {
    match kind {
        crate::token::FocusKind::Only => "ONLY",
        crate::token::FocusKind::Even => "EVEN",
        crate::token::FocusKind::Just => "JUST",
        crate::token::FocusKind::Cleft => "CLEFT",
    }
}

impl<'a> DisplayWith for LogicExpr<'a> {
    fn fmt_with(&self, interner: &Interner, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogicExpr::Predicate { name, args, .. } => {
                write!(f, "{}(", interner.resolve(*name))?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    arg.fmt_with(interner, f)?;
                }
                write!(f, ")")
            }
            LogicExpr::Identity { left, right } => {
                left.fmt_with(interner, f)?;
                write!(f, " = ")?;
                right.fmt_with(interner, f)
            }
            LogicExpr::Metaphor { tenor, vehicle } => {
                write!(f, "Metaphor(")?;
                tenor.fmt_with(interner, f)?;
                write!(f, ", ")?;
                vehicle.fmt_with(interner, f)?;
                write!(f, ")")
            }
            LogicExpr::Quantifier {
                kind,
                variable,
                body,
                ..
            } => {
                write!(
                    f,
                    "{}{}.{}",
                    quantifier_symbol(kind),
                    interner.resolve(*variable),
                    body.with(interner)
                )
            }
            LogicExpr::Categorical(data) => {
                let q = match &data.quantifier {
                    TokenType::All => "All",
                    TokenType::Some => "Some",
                    TokenType::No => "No",
                    _ => "?",
                };
                let cop = if data.copula_negative {
                    "are not"
                } else {
                    "are"
                };
                write!(
                    f,
                    "{} {} {} {}",
                    q,
                    data.subject.with(interner),
                    cop,
                    data.predicate.with(interner)
                )
            }
            LogicExpr::Relation(data) => {
                write!(
                    f,
                    "{}({}, {})",
                    interner.resolve(data.verb),
                    data.subject.with(interner),
                    data.object.with(interner)
                )
            }
            LogicExpr::Modal { vector, operand } => {
                write!(f, "{}({})", modal_symbol(vector), operand.with(interner))
            }
            LogicExpr::Temporal { operator, body } => {
                write!(f, "{}({})", temporal_symbol(operator), body.with(interner))
            }
            LogicExpr::Aspectual { operator, body } => {
                write!(f, "{}({})", aspect_symbol(operator), body.with(interner))
            }
            LogicExpr::Voice { operator, body } => {
                write!(f, "{}({})", voice_symbol(operator), body.with(interner))
            }
            LogicExpr::BinaryOp { left, op, right } => {
                write!(
                    f,
                    "({} {} {})",
                    left.with(interner),
                    connective_symbol(op),
                    right.with(interner)
                )
            }
            LogicExpr::UnaryOp { op, operand } => {
                write!(f, "{}({})", connective_symbol(op), operand.with(interner))
            }
            LogicExpr::Question { wh_variable, body } => {
                write!(
                    f,
                    "?{}.{}",
                    interner.resolve(*wh_variable),
                    body.with(interner)
                )
            }
            LogicExpr::YesNoQuestion { body } => {
                write!(f, "?{}", body.with(interner))
            }
            LogicExpr::Atom(s) => write!(f, "{}", interner.resolve(*s)),
            LogicExpr::Lambda { variable, body } => {
                write!(
                    f,
                    "λ{}.{}",
                    interner.resolve(*variable),
                    body.with(interner)
                )
            }
            LogicExpr::App { function, argument } => {
                write!(
                    f,
                    "({})({})",
                    function.with(interner),
                    argument.with(interner)
                )
            }
            LogicExpr::Intensional { operator, content } => {
                write!(
                    f,
                    "{}({})",
                    interner.resolve(*operator),
                    content.with(interner)
                )
            }
            LogicExpr::Event { predicate, adverbs } => {
                predicate.fmt_with(interner, f)?;
                for adv in *adverbs {
                    write!(f, "[{}]", interner.resolve(*adv))?;
                }
                Ok(())
            }
            LogicExpr::NeoEvent(data) => {
                write!(
                    f,
                    "∃{}({}({})",
                    interner.resolve(data.event_var),
                    interner.resolve(data.verb),
                    interner.resolve(data.event_var)
                )?;
                for (role, term) in data.roles.iter() {
                    write!(
                        f,
                        " ∧ {:?}({}, {})",
                        role,
                        interner.resolve(data.event_var),
                        term.with(interner)
                    )?;
                }
                for mod_sym in data.modifiers.iter() {
                    write!(
                        f,
                        " ∧ {}({})",
                        interner.resolve(*mod_sym),
                        interner.resolve(data.event_var)
                    )?;
                }
                write!(f, ")")
            }
            LogicExpr::Imperative { action } => {
                write!(f, "!({})", action.with(interner))
            }
            LogicExpr::Exclamative { degree_var, body } => {
                write!(
                    f,
                    "Exclaim(∃{}({} ∧ {} ≫ θ))",
                    interner.resolve(*degree_var),
                    body.with(interner),
                    interner.resolve(*degree_var)
                )
            }
            LogicExpr::Optative { wish } => {
                write!(f, "Wish(Speaker, ⟨{}⟩)", wish.with(interner))
            }
            LogicExpr::Implicature {
                assertion,
                implicature,
            } => {
                write!(
                    f,
                    "{} +> Implicature({})",
                    assertion.with(interner),
                    implicature.with(interner)
                )
            }
            LogicExpr::SpeechAct {
                performer,
                act_type,
                content,
            } => {
                write!(
                    f,
                    "{}:{}({})",
                    interner.resolve(*performer),
                    interner.resolve(*act_type),
                    content.with(interner)
                )
            }
            LogicExpr::Counterfactual {
                antecedent,
                consequent,
            } => {
                write!(
                    f,
                    "({} □→ {})",
                    antecedent.with(interner),
                    consequent.with(interner)
                )
            }
            LogicExpr::Causal { effect, cause } => {
                write!(
                    f,
                    "Cause({}, {})",
                    cause.with(interner),
                    effect.with(interner)
                )
            }
            LogicExpr::Concessive { main, concession } => {
                write!(
                    f,
                    "{} ∧ Concessive({})",
                    main.with(interner),
                    concession.with(interner)
                )
            }
            LogicExpr::Comparative {
                adjective,
                subject,
                object,
                difference,
                ..
            } => {
                if let Some(diff) = difference {
                    write!(
                        f,
                        "{}({}, {}, by: {})",
                        interner.resolve(*adjective),
                        subject.with(interner),
                        object.with(interner),
                        diff.with(interner)
                    )
                } else {
                    write!(
                        f,
                        "{}({}, {})",
                        interner.resolve(*adjective),
                        subject.with(interner),
                        object.with(interner)
                    )
                }
            }
            LogicExpr::Superlative {
                adjective,
                subject,
                domain,
            } => {
                write!(
                    f,
                    "MOST-{}({}, {})",
                    interner.resolve(*adjective),
                    subject.with(interner),
                    interner.resolve(*domain)
                )
            }
            LogicExpr::Scopal { operator, body } => {
                write!(
                    f,
                    "{}({})",
                    interner.resolve(*operator),
                    body.with(interner)
                )
            }
            LogicExpr::Control {
                verb,
                subject,
                object,
                infinitive,
            } => {
                write!(f, "{}(", interner.resolve(*verb))?;
                subject.fmt_with(interner, f)?;
                if let Some(obj) = object {
                    write!(f, ", ")?;
                    obj.fmt_with(interner, f)?;
                }
                write!(f, ", {})", infinitive.with(interner))
            }
            LogicExpr::Presupposition {
                assertion,
                presupposition,
            } => {
                write!(
                    f,
                    "[{} | {}]",
                    assertion.with(interner),
                    presupposition.with(interner)
                )
            }
            LogicExpr::Focus {
                kind,
                focused,
                scope,
            } => {
                write!(f, "{}[", focus_symbol(kind))?;
                focused.fmt_with(interner, f)?;
                write!(f, "]({})", scope.with(interner))
            }
            LogicExpr::TemporalAnchor { anchor, body } => {
                write!(f, "@{}({})", interner.resolve(*anchor), body.with(interner))
            }
            LogicExpr::Distributive { predicate } => {
                write!(f, "*")?;
                predicate.fmt_with(interner, f)
            }
            LogicExpr::GroupQuantifier {
                group_var,
                count,
                member_var,
                restriction,
                body,
            } => {
                write!(
                    f,
                    "∃{}(Group({}) ∧ Count({}, {}) ∧ ∀{}(Member({}, {}) → ",
                    interner.resolve(*group_var),
                    interner.resolve(*group_var),
                    interner.resolve(*group_var),
                    count,
                    interner.resolve(*member_var),
                    interner.resolve(*member_var),
                    interner.resolve(*group_var)
                )?;
                restriction.fmt_with(interner, f)?;
                write!(f, ") ∧ ")?;
                body.fmt_with(interner, f)?;
                write!(f, ")")
            }
            LogicExpr::TemporalBinary {
                operator,
                left,
                right,
            } => {
                write!(
                    f,
                    "({} {} {})",
                    left.with(interner),
                    binary_temporal_symbol(operator),
                    right.with(interner)
                )
            }
        }
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_language/debug/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_language/debug/connective_symbol_tests.rs"]
mod connective_symbol_tests;
