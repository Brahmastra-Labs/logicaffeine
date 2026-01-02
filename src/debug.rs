use std::fmt;

use crate::ast::{
    AspectOperator, LogicExpr, NounPhrase, QuantifierKind, TemporalOperator, VoiceOperator, Term,
};
use crate::intern::{Interner, Symbol};
use crate::token::TokenType;

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
            Term::Possessed { possessor, possessed } => {
                possessor.fmt_with(interner, f)?;
                write!(f, ".{}", interner.resolve(*possessed))
            }
            Term::Sigma(predicate) => {
                write!(f, "σx.{}(x)", interner.resolve(*predicate))
            }
            Term::Intension(predicate) => {
                write!(f, "^{}", interner.resolve(*predicate))
            }
            Term::Proposition(expr) => {
                write!(f, "[{:?}]", expr)
            }
            Term::Value { kind, unit, dimension } => {
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

impl<'a> DisplayWith for LogicExpr<'a> {
    fn fmt_with(&self, interner: &Interner, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogicExpr::Predicate { name, args } => {
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
            LogicExpr::Quantifier { kind, variable, body, .. } => {
                let q = match kind {
                    QuantifierKind::Universal => "∀",
                    QuantifierKind::Existential => "∃",
                    QuantifierKind::Most => "MOST",
                    QuantifierKind::Few => "FEW",
                    QuantifierKind::Many => "MANY",
                    QuantifierKind::Generic => "Gen",
                    QuantifierKind::Cardinal(n) => return write!(f, "∃={}{}.{}", n, interner.resolve(*variable), body.with(interner)),
                    QuantifierKind::AtLeast(n) => return write!(f, "∃≥{}{}.{}", n, interner.resolve(*variable), body.with(interner)),
                    QuantifierKind::AtMost(n) => return write!(f, "∃≤{}{}.{}", n, interner.resolve(*variable), body.with(interner)),
                };
                write!(f, "{}{}.{}", q, interner.resolve(*variable), body.with(interner))
            }
            LogicExpr::Categorical(data) => {
                let q = match &data.quantifier {
                    TokenType::All => "All",
                    TokenType::Some => "Some",
                    TokenType::No => "No",
                    _ => "?",
                };
                let cop = if data.copula_negative { "are not" } else { "are" };
                write!(f, "{} {} {} {}", q, data.subject.with(interner), cop, data.predicate.with(interner))
            }
            LogicExpr::Relation(data) => {
                write!(f, "{}({}, {})", interner.resolve(data.verb), data.subject.with(interner), data.object.with(interner))
            }
            LogicExpr::Modal { vector, operand } => {
                let op = match (vector.domain, vector.force >= 0.5) {
                    (crate::ast::ModalDomain::Alethic, true) => "□",
                    (crate::ast::ModalDomain::Alethic, false) => "◇",
                    (crate::ast::ModalDomain::Deontic, true) => "O",
                    (crate::ast::ModalDomain::Deontic, false) => "P",
                };
                write!(f, "{}({})", op, operand.with(interner))
            }
            LogicExpr::Temporal { operator, body } => {
                let op = match operator {
                    TemporalOperator::Past => "P",
                    TemporalOperator::Future => "F",
                };
                write!(f, "{}({})", op, body.with(interner))
            }
            LogicExpr::Aspectual { operator, body } => {
                let op = match operator {
                    AspectOperator::Progressive => "PROG",
                    AspectOperator::Perfect => "PERF",
                    AspectOperator::Habitual => "HAB",
                    AspectOperator::Iterative => "ITER",
                };
                write!(f, "{}({})", op, body.with(interner))
            }
            LogicExpr::Voice { operator, body } => {
                let op = match operator {
                    VoiceOperator::Passive => "PASS",
                };
                write!(f, "{}({})", op, body.with(interner))
            }
            LogicExpr::BinaryOp { left, op, right } => {
                let sym = match op {
                    TokenType::And => "∧",
                    TokenType::Or => "∨",
                    TokenType::If => "→",
                    TokenType::Iff => "↔",
                    _ => "?",
                };
                write!(f, "({} {} {})", left.with(interner), sym, right.with(interner))
            }
            LogicExpr::UnaryOp { op, operand } => {
                let sym = match op {
                    TokenType::Not => "¬",
                    _ => "?",
                };
                write!(f, "{}({})", sym, operand.with(interner))
            }
            LogicExpr::Question { wh_variable, body } => {
                write!(f, "?{}.{}", interner.resolve(*wh_variable), body.with(interner))
            }
            LogicExpr::YesNoQuestion { body } => {
                write!(f, "?{}", body.with(interner))
            }
            LogicExpr::Atom(s) => write!(f, "{}", interner.resolve(*s)),
            LogicExpr::Lambda { variable, body } => {
                write!(f, "λ{}.{}", interner.resolve(*variable), body.with(interner))
            }
            LogicExpr::App { function, argument } => {
                write!(f, "({})({})", function.with(interner), argument.with(interner))
            }
            LogicExpr::Intensional { operator, content } => {
                write!(f, "{}({})", interner.resolve(*operator), content.with(interner))
            }
            LogicExpr::Event { predicate, adverbs } => {
                predicate.fmt_with(interner, f)?;
                for adv in *adverbs {
                    write!(f, "[{}]", interner.resolve(*adv))?;
                }
                Ok(())
            }
            LogicExpr::NeoEvent(data) => {
                write!(f, "∃{}({}({})", interner.resolve(data.event_var), interner.resolve(data.verb), interner.resolve(data.event_var))?;
                for (role, term) in data.roles.iter() {
                    write!(f, " ∧ {:?}({}, {})", role, interner.resolve(data.event_var), term.with(interner))?;
                }
                for mod_sym in data.modifiers.iter() {
                    write!(f, " ∧ {}({})", interner.resolve(*mod_sym), interner.resolve(data.event_var))?;
                }
                write!(f, ")")
            }
            LogicExpr::Imperative { action } => {
                write!(f, "!({})", action.with(interner))
            }
            LogicExpr::SpeechAct { performer, act_type, content } => {
                write!(f, "{}:{}({})", interner.resolve(*performer), interner.resolve(*act_type), content.with(interner))
            }
            LogicExpr::Counterfactual { antecedent, consequent } => {
                write!(f, "({} □→ {})", antecedent.with(interner), consequent.with(interner))
            }
            LogicExpr::Causal { effect, cause } => {
                write!(f, "Cause({}, {})", cause.with(interner), effect.with(interner))
            }
            LogicExpr::Comparative { adjective, subject, object, difference } => {
                if let Some(diff) = difference {
                    write!(f, "{}({}, {}, by: {})", interner.resolve(*adjective), subject.with(interner), object.with(interner), diff.with(interner))
                } else {
                    write!(f, "{}({}, {})", interner.resolve(*adjective), subject.with(interner), object.with(interner))
                }
            }
            LogicExpr::Superlative { adjective, subject, domain } => {
                write!(f, "MOST-{}({}, {})", interner.resolve(*adjective), subject.with(interner), interner.resolve(*domain))
            }
            LogicExpr::Scopal { operator, body } => {
                write!(f, "{}({})", interner.resolve(*operator), body.with(interner))
            }
            LogicExpr::Control { verb, subject, object, infinitive } => {
                write!(f, "{}(", interner.resolve(*verb))?;
                subject.fmt_with(interner, f)?;
                if let Some(obj) = object {
                    write!(f, ", ")?;
                    obj.fmt_with(interner, f)?;
                }
                write!(f, ", {})", infinitive.with(interner))
            }
            LogicExpr::Presupposition { assertion, presupposition } => {
                write!(f, "[{} | {}]", assertion.with(interner), presupposition.with(interner))
            }
            LogicExpr::Focus { kind, focused, scope } => {
                let k = match kind {
                    crate::token::FocusKind::Only => "ONLY",
                    crate::token::FocusKind::Even => "EVEN",
                    crate::token::FocusKind::Just => "JUST",
                };
                write!(f, "{}[", k)?;
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
            LogicExpr::GroupQuantifier { group_var, count, member_var, restriction, body } => {
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::Arena;

    #[test]
    fn symbol_display_with_interner() {
        let mut interner = Interner::new();
        let sym = interner.intern("Socrates");
        assert_eq!(sym.with(&interner).to_string(), "Socrates");
    }

    #[test]
    fn symbol_empty_displays_empty() {
        let interner = Interner::new();
        assert_eq!(Symbol::EMPTY.with(&interner).to_string(), "");
    }

    #[test]
    fn term_constant_display() {
        let mut interner = Interner::new();
        let sym = interner.intern("John");
        let term = Term::Constant(sym);
        assert_eq!(term.with(&interner).to_string(), "John");
    }

    #[test]
    fn term_variable_display() {
        let mut interner = Interner::new();
        let x = interner.intern("x");
        let term = Term::Variable(x);
        assert_eq!(term.with(&interner).to_string(), "x");
    }

    #[test]
    fn term_function_display() {
        let mut interner = Interner::new();
        let term_arena: Arena<Term> = Arena::new();
        let f = interner.intern("father");
        let j = interner.intern("John");
        let term = Term::Function(f, term_arena.alloc_slice([Term::Constant(j)]));
        assert_eq!(term.with(&interner).to_string(), "father(John)");
    }

    #[test]
    fn term_group_display() {
        let mut interner = Interner::new();
        let term_arena: Arena<Term> = Arena::new();
        let j = interner.intern("John");
        let m = interner.intern("Mary");
        let term = Term::Group(term_arena.alloc_slice([Term::Constant(j), Term::Constant(m)]));
        assert_eq!(term.with(&interner).to_string(), "[John ⊕ Mary]");
    }

    #[test]
    fn term_possessed_display() {
        let mut interner = Interner::new();
        let term_arena: Arena<Term> = Arena::new();
        let j = interner.intern("John");
        let dog = interner.intern("dog");
        let term = Term::Possessed {
            possessor: term_arena.alloc(Term::Constant(j)),
            possessed: dog,
        };
        assert_eq!(term.with(&interner).to_string(), "John.dog");
    }

    #[test]
    fn expr_predicate_display() {
        let mut interner = Interner::new();
        let term_arena: Arena<Term> = Arena::new();
        let mortal = interner.intern("Mortal");
        let x = interner.intern("x");
        let expr = LogicExpr::Predicate {
            name: mortal,
            args: term_arena.alloc_slice([Term::Variable(x)]),
        };
        assert_eq!(expr.with(&interner).to_string(), "Mortal(x)");
    }

    #[test]
    fn expr_quantifier_display() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();
        let x = interner.intern("x");
        let mortal = interner.intern("Mortal");
        let body = expr_arena.alloc(LogicExpr::Predicate {
            name: mortal,
            args: term_arena.alloc_slice([Term::Variable(x)]),
        });
        let expr = LogicExpr::Quantifier {
            kind: QuantifierKind::Universal,
            variable: x,
            body,
            island_id: 0,
        };
        assert_eq!(expr.with(&interner).to_string(), "∀x.Mortal(x)");
    }

    #[test]
    fn expr_atom_display() {
        let mut interner = Interner::new();
        let p = interner.intern("P");
        let expr = LogicExpr::Atom(p);
        assert_eq!(expr.with(&interner).to_string(), "P");
    }

    #[test]
    fn expr_binary_op_display() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let p = interner.intern("P");
        let q = interner.intern("Q");
        let expr = LogicExpr::BinaryOp {
            left: expr_arena.alloc(LogicExpr::Atom(p)),
            op: TokenType::And,
            right: expr_arena.alloc(LogicExpr::Atom(q)),
        };
        assert_eq!(expr.with(&interner).to_string(), "(P ∧ Q)");
    }

    #[test]
    fn expr_lambda_display() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let x = interner.intern("x");
        let p = interner.intern("P");
        let expr = LogicExpr::Lambda {
            variable: x,
            body: expr_arena.alloc(LogicExpr::Atom(p)),
        };
        assert_eq!(expr.with(&interner).to_string(), "λx.P");
    }

    #[test]
    fn debug_world_works_with_dbg_pattern() {
        let mut interner = Interner::new();
        let sym = interner.intern("test");
        let term = Term::Constant(sym);
        let debug_str = format!("{:?}", DebugWorld { target: &term, interner: &interner });
        assert!(debug_str.contains("test"));
    }

    #[test]
    fn expr_temporal_display() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let p = interner.intern("Run");
        let expr = LogicExpr::Temporal {
            operator: TemporalOperator::Past,
            body: expr_arena.alloc(LogicExpr::Atom(p)),
        };
        assert_eq!(expr.with(&interner).to_string(), "P(Run)");
    }

    #[test]
    fn expr_modal_display() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let p = interner.intern("Rain");
        let expr = LogicExpr::Modal {
            vector: crate::ast::ModalVector {
                domain: crate::ast::ModalDomain::Alethic,
                force: 1.0,
                flavor: crate::ast::ModalFlavor::Root,
            },
            operand: expr_arena.alloc(LogicExpr::Atom(p)),
        };
        assert_eq!(expr.with(&interner).to_string(), "□(Rain)");
    }
}
