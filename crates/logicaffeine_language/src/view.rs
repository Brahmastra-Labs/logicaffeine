//! Owned view types for AST serialization and display.
//!
//! This module provides "view" versions of AST types that replace interned symbols
//! with resolved strings. Views are useful for:
//!
//! - Serialization (JSON/Serde) without interner dependency
//! - Pretty-printing and debugging
//! - UI display where string values are needed
//!
//! The conversion functions take an [`Interner`] reference to resolve symbols.

use crate::ast::{
    AspectOperator, LogicExpr, ModalVector, NounPhrase, QuantifierKind, TemporalOperator, VoiceOperator, Term,
    ThematicRole,
};
use logicaffeine_base::Interner;
use crate::lexicon::Definiteness;
use crate::token::{FocusKind, TokenType};

/// View of a term with resolved symbol names.
#[derive(Debug, Clone, PartialEq)]
pub enum TermView<'a> {
    Constant(&'a str),
    Variable(&'a str),
    Function(&'a str, Vec<TermView<'a>>),
    Group(Vec<TermView<'a>>),
    Possessed {
        possessor: Box<TermView<'a>>,
        possessed: &'a str,
    },
    Sigma(&'a str),
    Intension(&'a str),
    Proposition(Box<ExprView<'a>>),
    Value {
        kind: NumberKindView<'a>,
        unit: Option<&'a str>,
        dimension: Option<crate::ast::Dimension>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum NumberKindView<'a> {
    Real(f64),
    Integer(i64),
    Symbolic(&'a str),
}

#[derive(Debug, Clone, PartialEq)]
pub struct NounPhraseView<'a> {
    pub definiteness: Option<Definiteness>,
    pub adjectives: Vec<&'a str>,
    pub noun: &'a str,
    pub possessor: Option<Box<NounPhraseView<'a>>>,
    pub pps: Vec<Box<ExprView<'a>>>,
    pub superlative: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprView<'a> {
    Predicate {
        name: &'a str,
        args: Vec<TermView<'a>>,
    },
    Identity {
        left: TermView<'a>,
        right: TermView<'a>,
    },
    Metaphor {
        tenor: TermView<'a>,
        vehicle: TermView<'a>,
    },
    Quantifier {
        kind: QuantifierKind,
        variable: &'a str,
        body: Box<ExprView<'a>>,
    },
    Categorical {
        quantifier: TokenType,
        subject: NounPhraseView<'a>,
        copula_negative: bool,
        predicate: NounPhraseView<'a>,
    },
    Relation {
        subject: NounPhraseView<'a>,
        verb: &'a str,
        object: NounPhraseView<'a>,
    },
    Modal {
        vector: ModalVector,
        operand: Box<ExprView<'a>>,
    },
    Temporal {
        operator: TemporalOperator,
        body: Box<ExprView<'a>>,
    },
    Aspectual {
        operator: AspectOperator,
        body: Box<ExprView<'a>>,
    },
    Voice {
        operator: VoiceOperator,
        body: Box<ExprView<'a>>,
    },
    BinaryOp {
        left: Box<ExprView<'a>>,
        op: TokenType,
        right: Box<ExprView<'a>>,
    },
    UnaryOp {
        op: TokenType,
        operand: Box<ExprView<'a>>,
    },
    Question {
        wh_variable: &'a str,
        body: Box<ExprView<'a>>,
    },
    YesNoQuestion {
        body: Box<ExprView<'a>>,
    },
    Atom(&'a str),
    Lambda {
        variable: &'a str,
        body: Box<ExprView<'a>>,
    },
    App {
        function: Box<ExprView<'a>>,
        argument: Box<ExprView<'a>>,
    },
    Intensional {
        operator: &'a str,
        content: Box<ExprView<'a>>,
    },
    Event {
        predicate: Box<ExprView<'a>>,
        adverbs: Vec<&'a str>,
    },
    NeoEvent {
        event_var: &'a str,
        verb: &'a str,
        roles: Vec<(ThematicRole, TermView<'a>)>,
        modifiers: Vec<&'a str>,
    },
    Imperative {
        action: Box<ExprView<'a>>,
    },
    SpeechAct {
        performer: &'a str,
        act_type: &'a str,
        content: Box<ExprView<'a>>,
    },
    Counterfactual {
        antecedent: Box<ExprView<'a>>,
        consequent: Box<ExprView<'a>>,
    },
    Causal {
        effect: Box<ExprView<'a>>,
        cause: Box<ExprView<'a>>,
    },
    Comparative {
        adjective: &'a str,
        subject: TermView<'a>,
        object: TermView<'a>,
        difference: Option<Box<TermView<'a>>>,
    },
    Superlative {
        adjective: &'a str,
        subject: TermView<'a>,
        domain: &'a str,
    },
    Scopal {
        operator: &'a str,
        body: Box<ExprView<'a>>,
    },
    Control {
        verb: &'a str,
        subject: TermView<'a>,
        object: Option<TermView<'a>>,
        infinitive: Box<ExprView<'a>>,
    },
    Presupposition {
        assertion: Box<ExprView<'a>>,
        presupposition: Box<ExprView<'a>>,
    },
    Focus {
        kind: FocusKind,
        focused: TermView<'a>,
        scope: Box<ExprView<'a>>,
    },
    TemporalAnchor {
        anchor: &'a str,
        body: Box<ExprView<'a>>,
    },
    Distributive {
        predicate: Box<ExprView<'a>>,
    },
    GroupQuantifier {
        group_var: &'a str,
        count: u32,
        member_var: &'a str,
        restriction: Box<ExprView<'a>>,
        body: Box<ExprView<'a>>,
    },
}

pub trait Resolve<'a> {
    type Output;
    fn resolve(&self, interner: &'a Interner) -> Self::Output;
}

impl<'a, 'b> Resolve<'a> for Term<'b> {
    type Output = TermView<'a>;

    fn resolve(&self, interner: &'a Interner) -> TermView<'a> {
        match self {
            Term::Constant(s) => TermView::Constant(interner.resolve(*s)),
            Term::Variable(s) => TermView::Variable(interner.resolve(*s)),
            Term::Function(name, args) => TermView::Function(
                interner.resolve(*name),
                args.iter().map(|a| a.resolve(interner)).collect(),
            ),
            Term::Group(members) => {
                TermView::Group(members.iter().map(|m| m.resolve(interner)).collect())
            }
            Term::Possessed {
                possessor,
                possessed,
            } => TermView::Possessed {
                possessor: Box::new(possessor.resolve(interner)),
                possessed: interner.resolve(*possessed),
            },
            Term::Sigma(predicate) => TermView::Sigma(interner.resolve(*predicate)),
            Term::Intension(predicate) => TermView::Intension(interner.resolve(*predicate)),
            Term::Proposition(expr) => {
                TermView::Proposition(Box::new(expr.resolve(interner)))
            }
            Term::Value { kind, unit, dimension } => {
                use crate::ast::NumberKind;
                let kind_view = match kind {
                    NumberKind::Real(r) => NumberKindView::Real(*r),
                    NumberKind::Integer(i) => NumberKindView::Integer(*i),
                    NumberKind::Symbolic(s) => NumberKindView::Symbolic(interner.resolve(*s)),
                };
                TermView::Value {
                    kind: kind_view,
                    unit: unit.map(|u| interner.resolve(u)),
                    dimension: *dimension,
                }
            }
        }
    }
}

impl<'a, 'b> Resolve<'a> for NounPhrase<'b> {
    type Output = NounPhraseView<'a>;

    fn resolve(&self, interner: &'a Interner) -> NounPhraseView<'a> {
        NounPhraseView {
            definiteness: self.definiteness,
            adjectives: self.adjectives.iter().map(|s| interner.resolve(*s)).collect(),
            noun: interner.resolve(self.noun),
            possessor: self.possessor.map(|p| Box::new(p.resolve(interner))),
            pps: self.pps.iter().map(|pp| Box::new(pp.resolve(interner))).collect(),
            superlative: self.superlative.map(|s| interner.resolve(s)),
        }
    }
}

impl<'a, 'b> Resolve<'a> for LogicExpr<'b> {
    type Output = ExprView<'a>;

    fn resolve(&self, interner: &'a Interner) -> ExprView<'a> {
        match self {
            LogicExpr::Predicate { name, args, .. } => ExprView::Predicate {
                name: interner.resolve(*name),
                args: args.iter().map(|a| a.resolve(interner)).collect(),
            },
            LogicExpr::Identity { left, right } => ExprView::Identity {
                left: left.resolve(interner),
                right: right.resolve(interner),
            },
            LogicExpr::Metaphor { tenor, vehicle } => ExprView::Metaphor {
                tenor: tenor.resolve(interner),
                vehicle: vehicle.resolve(interner),
            },
            LogicExpr::Quantifier { kind, variable, body, .. } => ExprView::Quantifier {
                kind: *kind,
                variable: interner.resolve(*variable),
                body: Box::new(body.resolve(interner)),
            },
            LogicExpr::Categorical(data) => ExprView::Categorical {
                quantifier: data.quantifier.clone(),
                subject: data.subject.resolve(interner),
                copula_negative: data.copula_negative,
                predicate: data.predicate.resolve(interner),
            },
            LogicExpr::Relation(data) => ExprView::Relation {
                subject: data.subject.resolve(interner),
                verb: interner.resolve(data.verb),
                object: data.object.resolve(interner),
            },
            LogicExpr::Modal { vector, operand } => ExprView::Modal {
                vector: *vector,
                operand: Box::new(operand.resolve(interner)),
            },
            LogicExpr::Temporal { operator, body } => ExprView::Temporal {
                operator: *operator,
                body: Box::new(body.resolve(interner)),
            },
            LogicExpr::Aspectual { operator, body } => ExprView::Aspectual {
                operator: *operator,
                body: Box::new(body.resolve(interner)),
            },
            LogicExpr::Voice { operator, body } => ExprView::Voice {
                operator: *operator,
                body: Box::new(body.resolve(interner)),
            },
            LogicExpr::BinaryOp { left, op, right } => ExprView::BinaryOp {
                left: Box::new(left.resolve(interner)),
                op: op.clone(),
                right: Box::new(right.resolve(interner)),
            },
            LogicExpr::UnaryOp { op, operand } => ExprView::UnaryOp {
                op: op.clone(),
                operand: Box::new(operand.resolve(interner)),
            },
            LogicExpr::Question { wh_variable, body } => ExprView::Question {
                wh_variable: interner.resolve(*wh_variable),
                body: Box::new(body.resolve(interner)),
            },
            LogicExpr::YesNoQuestion { body } => ExprView::YesNoQuestion {
                body: Box::new(body.resolve(interner)),
            },
            LogicExpr::Atom(s) => ExprView::Atom(interner.resolve(*s)),
            LogicExpr::Lambda { variable, body } => ExprView::Lambda {
                variable: interner.resolve(*variable),
                body: Box::new(body.resolve(interner)),
            },
            LogicExpr::App { function, argument } => ExprView::App {
                function: Box::new(function.resolve(interner)),
                argument: Box::new(argument.resolve(interner)),
            },
            LogicExpr::Intensional { operator, content } => ExprView::Intensional {
                operator: interner.resolve(*operator),
                content: Box::new(content.resolve(interner)),
            },
            LogicExpr::Event { predicate, adverbs } => ExprView::Event {
                predicate: Box::new(predicate.resolve(interner)),
                adverbs: adverbs.iter().map(|s| interner.resolve(*s)).collect(),
            },
            LogicExpr::NeoEvent(data) => ExprView::NeoEvent {
                event_var: interner.resolve(data.event_var),
                verb: interner.resolve(data.verb),
                roles: data.roles.iter().map(|(role, term)| (*role, term.resolve(interner))).collect(),
                modifiers: data.modifiers.iter().map(|s| interner.resolve(*s)).collect(),
            },
            LogicExpr::Imperative { action } => ExprView::Imperative {
                action: Box::new(action.resolve(interner)),
            },
            LogicExpr::SpeechAct {
                performer,
                act_type,
                content,
            } => ExprView::SpeechAct {
                performer: interner.resolve(*performer),
                act_type: interner.resolve(*act_type),
                content: Box::new(content.resolve(interner)),
            },
            LogicExpr::Counterfactual { antecedent, consequent } => ExprView::Counterfactual {
                antecedent: Box::new(antecedent.resolve(interner)),
                consequent: Box::new(consequent.resolve(interner)),
            },
            LogicExpr::Causal { effect, cause } => ExprView::Causal {
                effect: Box::new(effect.resolve(interner)),
                cause: Box::new(cause.resolve(interner)),
            },
            LogicExpr::Comparative { adjective, subject, object, difference } => ExprView::Comparative {
                adjective: interner.resolve(*adjective),
                subject: subject.resolve(interner),
                object: object.resolve(interner),
                difference: difference.map(|d| Box::new(d.resolve(interner))),
            },
            LogicExpr::Superlative { adjective, subject, domain } => ExprView::Superlative {
                adjective: interner.resolve(*adjective),
                subject: subject.resolve(interner),
                domain: interner.resolve(*domain),
            },
            LogicExpr::Scopal { operator, body } => ExprView::Scopal {
                operator: interner.resolve(*operator),
                body: Box::new(body.resolve(interner)),
            },
            LogicExpr::Control {
                verb,
                subject,
                object,
                infinitive,
            } => ExprView::Control {
                verb: interner.resolve(*verb),
                subject: subject.resolve(interner),
                object: object.map(|o| o.resolve(interner)),
                infinitive: Box::new(infinitive.resolve(interner)),
            },
            LogicExpr::Presupposition { assertion, presupposition } => ExprView::Presupposition {
                assertion: Box::new(assertion.resolve(interner)),
                presupposition: Box::new(presupposition.resolve(interner)),
            },
            LogicExpr::Focus { kind, focused, scope } => ExprView::Focus {
                kind: *kind,
                focused: focused.resolve(interner),
                scope: Box::new(scope.resolve(interner)),
            },
            LogicExpr::TemporalAnchor { anchor, body } => ExprView::TemporalAnchor {
                anchor: interner.resolve(*anchor),
                body: Box::new(body.resolve(interner)),
            },
            LogicExpr::Distributive { predicate } => ExprView::Distributive {
                predicate: Box::new(predicate.resolve(interner)),
            },
            LogicExpr::GroupQuantifier { group_var, count, member_var, restriction, body } => ExprView::GroupQuantifier {
                group_var: interner.resolve(*group_var),
                count: *count,
                member_var: interner.resolve(*member_var),
                restriction: Box::new(restriction.resolve(interner)),
                body: Box::new(body.resolve(interner)),
            },
        }
    }
}

#[cfg(test)]
mod term_view_tests {
    use super::*;
    use logicaffeine_base::Arena;

    #[test]
    fn resolve_term_constant() {
        let mut interner = Interner::new();
        let sym = interner.intern("Socrates");
        let term = Term::Constant(sym);
        assert_eq!(term.resolve(&interner), TermView::Constant("Socrates"));
    }

    #[test]
    fn resolve_term_variable() {
        let mut interner = Interner::new();
        let x = interner.intern("x");
        let term = Term::Variable(x);
        assert_eq!(term.resolve(&interner), TermView::Variable("x"));
    }

    #[test]
    fn resolve_term_function() {
        let mut interner = Interner::new();
        let term_arena: Arena<Term> = Arena::new();
        let father = interner.intern("father");
        let john = interner.intern("John");
        let term = Term::Function(father, term_arena.alloc_slice([Term::Constant(john)]));

        assert_eq!(
            term.resolve(&interner),
            TermView::Function("father", vec![TermView::Constant("John")])
        );
    }

    #[test]
    fn resolve_term_group() {
        let mut interner = Interner::new();
        let term_arena: Arena<Term> = Arena::new();
        let j = interner.intern("John");
        let m = interner.intern("Mary");
        let term = Term::Group(term_arena.alloc_slice([Term::Constant(j), Term::Constant(m)]));

        assert_eq!(
            term.resolve(&interner),
            TermView::Group(vec![
                TermView::Constant("John"),
                TermView::Constant("Mary")
            ])
        );
    }

    #[test]
    fn resolve_term_possessed() {
        let mut interner = Interner::new();
        let term_arena: Arena<Term> = Arena::new();
        let john = interner.intern("John");
        let dog = interner.intern("dog");
        let term = Term::Possessed {
            possessor: term_arena.alloc(Term::Constant(john)),
            possessed: dog,
        };

        assert_eq!(
            term.resolve(&interner),
            TermView::Possessed {
                possessor: Box::new(TermView::Constant("John")),
                possessed: "dog",
            }
        );
    }

    #[test]
    fn term_view_equality_is_bit_exact() {
        let a = TermView::Constant("test");
        let b = TermView::Constant("test");
        let c = TermView::Constant("Test");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn nested_function_resolve() {
        let mut interner = Interner::new();
        let term_arena: Arena<Term> = Arena::new();
        let f = interner.intern("f");
        let g = interner.intern("g");
        let x = interner.intern("x");

        let inner = Term::Function(g, term_arena.alloc_slice([Term::Variable(x)]));
        let outer = Term::Function(f, term_arena.alloc_slice([inner]));

        assert_eq!(
            outer.resolve(&interner),
            TermView::Function(
                "f",
                vec![TermView::Function("g", vec![TermView::Variable("x")])]
            )
        );
    }
}

#[cfg(test)]
mod expr_view_tests {
    use super::*;
    use logicaffeine_base::Arena;
    use crate::ast::ModalDomain;

    #[test]
    fn resolve_expr_predicate() {
        let mut interner = Interner::new();
        let term_arena: Arena<Term> = Arena::new();
        let mortal = interner.intern("Mortal");
        let x = interner.intern("x");
        let expr = LogicExpr::Predicate {
            name: mortal,
            args: term_arena.alloc_slice([Term::Variable(x)]),
            world: None,
        };

        assert_eq!(
            expr.resolve(&interner),
            ExprView::Predicate {
                name: "Mortal",
                args: vec![TermView::Variable("x")],
            }
        );
    }

    #[test]
    fn resolve_expr_identity() {
        let mut interner = Interner::new();
        let term_arena: Arena<Term> = Arena::new();
        let clark = interner.intern("Clark");
        let superman = interner.intern("Superman");
        let expr = LogicExpr::Identity {
            left: term_arena.alloc(Term::Constant(clark)),
            right: term_arena.alloc(Term::Constant(superman)),
        };

        assert_eq!(
            expr.resolve(&interner),
            ExprView::Identity {
                left: TermView::Constant("Clark"),
                right: TermView::Constant("Superman"),
            }
        );
    }

    #[test]
    fn resolve_expr_quantifier() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();
        let x = interner.intern("x");
        let mortal = interner.intern("Mortal");

        let body = expr_arena.alloc(LogicExpr::Predicate {
            name: mortal,
            args: term_arena.alloc_slice([Term::Variable(x)]),
            world: None,
        });
        let expr = LogicExpr::Quantifier {
            kind: QuantifierKind::Universal,
            variable: x,
            body,
            island_id: 0,
        };

        assert_eq!(
            expr.resolve(&interner),
            ExprView::Quantifier {
                kind: QuantifierKind::Universal,
                variable: "x",
                body: Box::new(ExprView::Predicate {
                    name: "Mortal",
                    args: vec![TermView::Variable("x")],
                }),
            }
        );
    }

    #[test]
    fn resolve_expr_atom() {
        let mut interner = Interner::new();
        let p = interner.intern("P");
        let expr = LogicExpr::Atom(p);

        assert_eq!(expr.resolve(&interner), ExprView::Atom("P"));
    }

    #[test]
    fn resolve_expr_binary_op() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let p = interner.intern("P");
        let q = interner.intern("Q");
        let expr = LogicExpr::BinaryOp {
            left: expr_arena.alloc(LogicExpr::Atom(p)),
            op: TokenType::And,
            right: expr_arena.alloc(LogicExpr::Atom(q)),
        };

        assert_eq!(
            expr.resolve(&interner),
            ExprView::BinaryOp {
                left: Box::new(ExprView::Atom("P")),
                op: TokenType::And,
                right: Box::new(ExprView::Atom("Q")),
            }
        );
    }

    #[test]
    fn resolve_expr_lambda() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let x = interner.intern("x");
        let p = interner.intern("P");
        let expr = LogicExpr::Lambda {
            variable: x,
            body: expr_arena.alloc(LogicExpr::Atom(p)),
        };

        assert_eq!(
            expr.resolve(&interner),
            ExprView::Lambda {
                variable: "x",
                body: Box::new(ExprView::Atom("P")),
            }
        );
    }

    #[test]
    fn resolve_expr_temporal() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let run = interner.intern("Run");
        let expr = LogicExpr::Temporal {
            operator: TemporalOperator::Past,
            body: expr_arena.alloc(LogicExpr::Atom(run)),
        };

        assert_eq!(
            expr.resolve(&interner),
            ExprView::Temporal {
                operator: TemporalOperator::Past,
                body: Box::new(ExprView::Atom("Run")),
            }
        );
    }

    #[test]
    fn resolve_expr_modal() {
        use crate::ast::ModalFlavor;
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let rain = interner.intern("Rain");
        let expr = LogicExpr::Modal {
            vector: ModalVector {
                domain: ModalDomain::Alethic,
                force: 1.0,
                flavor: ModalFlavor::Root,
            },
            operand: expr_arena.alloc(LogicExpr::Atom(rain)),
        };

        assert_eq!(
            expr.resolve(&interner),
            ExprView::Modal {
                vector: ModalVector {
                    domain: ModalDomain::Alethic,
                    force: 1.0,
                    flavor: ModalFlavor::Root,
                },
                operand: Box::new(ExprView::Atom("Rain")),
            }
        );
    }

    #[test]
    fn modal_vector_equality_is_bit_exact() {
        use crate::ast::ModalFlavor;
        let v1 = ModalVector {
            domain: ModalDomain::Alethic,
            force: 0.5,
            flavor: ModalFlavor::Root,
        };
        let v2 = ModalVector {
            domain: ModalDomain::Alethic,
            force: 0.5,
            flavor: ModalFlavor::Root,
        };
        let v3 = ModalVector {
            domain: ModalDomain::Alethic,
            force: 0.51,
            flavor: ModalFlavor::Root,
        };

        assert_eq!(v1, v2);
        assert_ne!(v1, v3);
    }

    #[test]
    fn resolve_expr_unary_op() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let p = interner.intern("P");
        let expr = LogicExpr::UnaryOp {
            op: TokenType::Not,
            operand: expr_arena.alloc(LogicExpr::Atom(p)),
        };

        assert_eq!(
            expr.resolve(&interner),
            ExprView::UnaryOp {
                op: TokenType::Not,
                operand: Box::new(ExprView::Atom("P")),
            }
        );
    }

    #[test]
    fn resolve_expr_app() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let f = interner.intern("f");
        let x = interner.intern("x");
        let expr = LogicExpr::App {
            function: expr_arena.alloc(LogicExpr::Atom(f)),
            argument: expr_arena.alloc(LogicExpr::Atom(x)),
        };

        assert_eq!(
            expr.resolve(&interner),
            ExprView::App {
                function: Box::new(ExprView::Atom("f")),
                argument: Box::new(ExprView::Atom("x")),
            }
        );
    }

    #[test]
    fn expr_view_equality_complex() {
        let a = ExprView::Quantifier {
            kind: QuantifierKind::Universal,
            variable: "x",
            body: Box::new(ExprView::Predicate {
                name: "P",
                args: vec![TermView::Variable("x")],
            }),
        };
        let b = ExprView::Quantifier {
            kind: QuantifierKind::Universal,
            variable: "x",
            body: Box::new(ExprView::Predicate {
                name: "P",
                args: vec![TermView::Variable("x")],
            }),
        };
        assert_eq!(a, b);
    }
}
