//! Pragmatic inference and focus-sensitive parsing.
//!
//! This module handles linguistic phenomena that go beyond pure syntax/semantics:
//!
//! - **Focus particles**: "only", "even", "also" with alternative semantics
//! - **Presupposition triggers**: "stop", "continue", "too"
//! - **Measure phrases**: Dimensional expressions with units
//! - **Comparatives**: "taller than", "as tall as", degree semantics
//! - **Superlatives**: "the tallest", unique maximal individuals
//! - **Scopal adverbs**: "always", "never", "usually"
//!
//! Focus is represented using the `LogicExpr::Focus` variant with an alternatives
//! set derived from the focus domain.

use super::noun::NounParsing;
use super::quantifier::QuantifierParsing;
use super::{ParseResult, Parser};
use crate::ast::{LogicExpr, NounPhrase, NumberKind, QuantifierKind, TemporalOperator, Term};
use crate::error::{ParseError, ParseErrorKind};
use crate::lexicon::{self, Time};
use crate::token::{MeasureKind, PresupKind, TokenType};

/// Trait for parsing pragmatic and focus-sensitive constructions.
///
/// Provides methods for parsing linguistic phenomena that go beyond pure
/// syntax/semantics, including focus particles, presupposition triggers,
/// measure phrases, degree expressions (comparatives/superlatives), and
/// scopal adverbs.
///
/// Focus is represented using alternative semantics, where the focused
/// element evokes a set of alternatives. Presuppositions project through
/// various operators and represent background entailments.
pub trait PragmaticsParsing<'a, 'ctx, 'int> {
    /// Parses a focus particle construction: "only John runs", "even Mary left".
    ///
    /// Focus particles like "only", "even", and "also" introduce alternative
    /// semantics. The focused element is contrasted with a contextually
    /// determined set of alternatives.
    ///
    /// Returns a [`LogicExpr::Focus`] with the focus kind, focused element,
    /// and the scope predicate.
    fn parse_focus(&mut self) -> ParseResult<&'a LogicExpr<'a>>;

    /// Parses a measure construction: "much water is cold", "little food arrived".
    ///
    /// Measure expressions quantify over amounts using "much", "little", etc.
    /// The result is an existentially quantified formula binding the measured
    /// entity with a measure predicate.
    fn parse_measure(&mut self) -> ParseResult<&'a LogicExpr<'a>>;

    /// Parses a presupposition-triggering verb: "stopped running", "regrets leaving".
    ///
    /// Presupposition triggers introduce background entailments that project
    /// through negation and other operators:
    ///
    /// - "stop P" presupposes: previously P; asserts: now ¬P
    /// - "start P" presupposes: previously ¬P; asserts: now P
    /// - "regret P" presupposes: P happened; asserts: subject regrets it
    /// - "continue P" presupposes: was P; asserts: still P
    ///
    /// Returns a [`LogicExpr::Presupposition`] separating assertion from presupposition.
    fn parse_presupposition(
        &mut self,
        subject: &NounPhrase<'a>,
        presup_kind: PresupKind,
    ) -> ParseResult<&'a LogicExpr<'a>>;

    /// Parses a simple predicate for a given subject noun phrase.
    ///
    /// Handles verb phrases with optional objects and focus-marked objects
    /// like "eats only rice". Used as a helper for focus and other pragmatic
    /// constructions.
    fn parse_predicate_for_subject(&mut self, subject: &NounPhrase<'a>)
        -> ParseResult<&'a LogicExpr<'a>>;

    /// Parses a scopal adverb construction: "always runs", "never sleeps".
    ///
    /// Scopal adverbs like "always", "never", "usually", "sometimes" quantify
    /// over times, events, or situations. They create a [`LogicExpr::Scopal`]
    /// operator that scopes over the verb predicate.
    fn parse_scopal_adverb(&mut self, subject: &NounPhrase<'a>) -> ParseResult<&'a LogicExpr<'a>>;

    /// Parses a superlative construction: "is the tallest student".
    ///
    /// Superlatives identify the unique maximal individual along a gradable
    /// dimension within a comparison class. Returns a [`LogicExpr::Superlative`]
    /// with the adjective, subject, and domain restrictor.
    fn parse_superlative(&mut self, subject: &NounPhrase<'a>) -> ParseResult<&'a LogicExpr<'a>>;

    /// Parses a comparative construction: "is taller than Mary", "is greater than 0".
    ///
    /// Comparatives establish an ordering relation along a gradable dimension.
    /// Supports both NP comparisons ("taller than Mary") and numeric comparisons
    /// ("greater than 0"). The optional `difference` parameter handles differential
    /// comparatives like "3 inches taller".
    ///
    /// Returns a [`LogicExpr::Comparative`] with adjective, subject, and object.
    fn parse_comparative(
        &mut self,
        subject: &NounPhrase<'a>,
        copula_time: Time,
        difference: Option<&'a Term<'a>>,
    ) -> ParseResult<&'a LogicExpr<'a>>;

    /// Checks if the current token is a numeric literal.
    ///
    /// Used to distinguish numeric comparisons ("greater than 0") from
    /// entity comparisons ("taller than John").
    fn check_number(&self) -> bool;

    /// Parses a measure phrase: "5 meters", "100 kilograms".
    ///
    /// Measure phrases combine a numeric value with an optional unit.
    /// The unit is looked up in the lexicon to determine its dimension
    /// (length, mass, time, etc.).
    ///
    /// Returns a [`Term::Value`] with the parsed number, unit, and dimension.
    fn parse_measure_phrase(&mut self) -> ParseResult<&'a Term<'a>>;
}

impl<'a, 'ctx, 'int> PragmaticsParsing<'a, 'ctx, 'int> for Parser<'a, 'ctx, 'int> {
    fn parse_focus(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        let kind = if let TokenType::Focus(k) = self.advance().kind {
            k
        } else {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedFocusParticle,
                span: self.current_span(),
            });
        };

        if self.check_quantifier() {
            self.advance();
            let quantified = self.parse_quantified()?;
            let focus_var = self.interner.intern("focus");
            let focused = self.ctx.terms.alloc(Term::Variable(focus_var));
            return Ok(self.ctx.exprs.alloc(LogicExpr::Focus {
                kind,
                focused,
                scope: quantified,
            }));
        }

        let focused_np = self.parse_noun_phrase(true)?;
        let focused = self.ctx.terms.alloc(Term::Constant(focused_np.noun));

        let scope = self.parse_predicate_for_subject(&focused_np)?;

        Ok(self.ctx.exprs.alloc(LogicExpr::Focus {
            kind,
            focused,
            scope,
        }))
    }

    fn parse_measure(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        let kind = if let TokenType::Measure(k) = self.advance().kind {
            k
        } else {
            return Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    expected: TokenType::Measure(MeasureKind::Much),
                    found: self.peek().kind.clone(),
                },
                span: self.current_span(),
            });
        };

        let np = self.parse_noun_phrase(true)?;
        let var = self.next_var_name();

        let noun_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: np.noun,
            args: self.ctx.terms.alloc_slice([Term::Variable(var)]),
            world: None,
        });

        let measure_sym = self.interner.intern("Measure");
        let kind_sym = self.interner.intern(match kind {
            MeasureKind::Much => "Much",
            MeasureKind::Little => "Little",
        });
        let measure_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: measure_sym,
            args: self
                .ctx
                .terms
                .alloc_slice([Term::Variable(var), Term::Constant(kind_sym)]),
            world: None,
        });

        let (pred_expr, verb_time) = if self.check(&TokenType::Is) {
            let copula_time = if let TokenType::Is = self.advance().kind {
                Time::Present
            } else {
                Time::Present
            };

            // Check for comparative: "is colder than"
            if self.check_comparative() {
                let subj_np = NounPhrase {
                    noun: np.noun,
                    definiteness: None,
                    adjectives: &[],
                    possessor: None,
                    pps: &[],
                    superlative: None,
                };
                let comp_expr = self.parse_comparative(&subj_np, copula_time, None)?;

                let combined = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: noun_pred,
                    op: TokenType::And,
                    right: self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: measure_pred,
                        op: TokenType::And,
                        right: comp_expr,
                    }),
                });

                return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: QuantifierKind::Existential,
                    variable: var,
                    body: combined,
                    island_id: self.current_island,
                }));
            }

            let adj = self.consume_content_word()?;
            let adj_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: adj,
                args: self.ctx.terms.alloc_slice([Term::Variable(var)]),
                world: None,
            });
            (adj_pred, copula_time)
        } else {
            let (verb, verb_time, _, _) = self.consume_verb_with_metadata();
            let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice([Term::Variable(var)]),
                world: None,
            });
            (verb_pred, verb_time)
        };

        let combined = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
            left: noun_pred,
            op: TokenType::And,
            right: self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: measure_pred,
                op: TokenType::And,
                right: pred_expr,
            }),
        });

        let with_time = match verb_time {
            Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: TemporalOperator::Past,
                body: combined,
            }),
            Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: TemporalOperator::Future,
                body: combined,
            }),
            _ => combined,
        };

        Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Existential,
            variable: var,
            body: with_time,
            island_id: self.current_island,
        }))
    }

    fn parse_presupposition(
        &mut self,
        subject: &NounPhrase<'a>,
        presup_kind: PresupKind,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        let subject_noun = subject.noun;

        let unknown = self.interner.intern("?");
        let complement = if self.check_verb() {
            let verb = self.consume_verb();
            self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice([Term::Constant(subject_noun)]),
                world: None,
            })
        } else {
            self.ctx.exprs.alloc(LogicExpr::Atom(unknown))
        };

        let (assertion, presupposition) = match presup_kind {
            PresupKind::Stop => {
                let neg = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                    op: TokenType::Not,
                    operand: complement,
                });
                let past = self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: TemporalOperator::Past,
                    body: complement,
                });
                (neg, past)
            }
            PresupKind::Start => {
                let past = self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: TemporalOperator::Past,
                    body: complement,
                });
                let neg_past = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                    op: TokenType::Not,
                    operand: past,
                });
                (complement, neg_past)
            }
            PresupKind::Regret => {
                let regret_sym = self.interner.intern("Regret");
                let regret = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: regret_sym,
                    args: self.ctx.terms.alloc_slice([Term::Constant(subject_noun)]),
                    world: None,
                });
                let past = self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: TemporalOperator::Past,
                    body: complement,
                });
                (regret, past)
            }
            PresupKind::Continue | PresupKind::Realize | PresupKind::Know => {
                let verb_name = match presup_kind {
                    PresupKind::Continue => self.interner.intern("Continue"),
                    PresupKind::Realize => self.interner.intern("Realize"),
                    PresupKind::Know => self.interner.intern("Know"),
                    _ => unknown,
                };
                let main = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: verb_name,
                    args: self.ctx.terms.alloc_slice([Term::Constant(subject_noun)]),
                    world: None,
                });
                (main, complement)
            }
        };

        Ok(self.ctx.exprs.alloc(LogicExpr::Presupposition {
            assertion,
            presupposition,
        }))
    }

    fn parse_predicate_for_subject(
        &mut self,
        subject: &NounPhrase<'a>,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        if self.check_verb() {
            let verb = self.consume_verb();

            // Check for focused object: "eats only rice"
            if self.check_focus() {
                let focus_kind = if let TokenType::Focus(k) = self.advance().kind {
                    k
                } else {
                    crate::token::FocusKind::Only
                };

                let object_np = self.parse_noun_phrase(false)?;
                let object_term = Term::Constant(object_np.noun);

                let predicate = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: verb,
                    args: self.ctx.terms.alloc_slice([
                        Term::Constant(subject.noun),
                        object_term.clone(),
                    ]),
                    world: None,
                });

                return Ok(self.ctx.exprs.alloc(LogicExpr::Focus {
                    kind: focus_kind,
                    focused: self.ctx.terms.alloc(object_term),
                    scope: predicate,
                }));
            }

            let mut args = vec![Term::Constant(subject.noun)];

            if self.check_content_word() || self.check_article() {
                let object = self.parse_noun_phrase(false)?;
                args.push(Term::Constant(object.noun));
            }

            Ok(self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice(args),
                world: None,
            }))
        } else {
            Ok(self.ctx.exprs.alloc(LogicExpr::Atom(subject.noun)))
        }
    }

    fn parse_scopal_adverb(&mut self, subject: &NounPhrase<'a>) -> ParseResult<&'a LogicExpr<'a>> {
        let operator = if let TokenType::ScopalAdverb(adv) = self.advance().kind.clone() {
            adv
        } else {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedScopalAdverb,
                span: self.current_span(),
            });
        };

        if !self.check_verb() {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedVerb {
                    found: self.peek().kind.clone(),
                },
                span: self.current_span(),
            });
        }

        let (verb, verb_time, _verb_aspect, _) = self.consume_verb_with_metadata();

        let predicate = self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: verb,
            args: self.ctx.terms.alloc_slice([Term::Constant(subject.noun)]),
            world: None,
        });

        let with_time = match verb_time {
            Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: TemporalOperator::Past,
                body: predicate,
            }),
            Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: TemporalOperator::Future,
                body: predicate,
            }),
            _ => predicate,
        };

        Ok(self.ctx.exprs.alloc(LogicExpr::Scopal {
            operator,
            body: with_time,
        }))
    }

    fn parse_superlative(&mut self, subject: &NounPhrase<'a>) -> ParseResult<&'a LogicExpr<'a>> {
        let adj = if let TokenType::Superlative(adj) = self.advance().kind.clone() {
            adj
        } else {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedSuperlativeAdjective,
                span: self.current_span(),
            });
        };

        let domain = self.consume_content_word()?;

        Ok(self.ctx.exprs.alloc(LogicExpr::Superlative {
            adjective: adj,
            subject: self.ctx.terms.alloc(Term::Constant(subject.noun)),
            domain,
        }))
    }

    fn parse_comparative(
        &mut self,
        subject: &NounPhrase<'a>,
        _copula_time: Time,
        difference: Option<&'a Term<'a>>,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        let adj = if let TokenType::Comparative(adj) = self.advance().kind.clone() {
            adj
        } else {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedComparativeAdjective,
                span: self.current_span(),
            });
        };

        if !self.check(&TokenType::Than) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedThan,
                span: self.current_span(),
            });
        }
        self.advance();

        // Check if the comparison target is a number (e.g., "greater than 0")
        let object_term = if self.check_number() {
            // Parse number as the comparison target
            let num_sym = if let TokenType::Number(sym) = self.advance().kind {
                sym
            } else {
                unreachable!()
            };
            let num_str = self.interner.resolve(num_sym);
            let num_val = num_str.parse::<i64>().unwrap_or(0);
            self.ctx.terms.alloc(Term::Value {
                kind: crate::ast::logic::NumberKind::Integer(num_val),
                unit: None,
                dimension: None,
            })
        } else {
            // Parse noun phrase as the comparison target
            let object = self.parse_noun_phrase(false)?;
            let obj_term = self.ctx.terms.alloc(Term::Constant(object.noun));

            let result = self.ctx.exprs.alloc(LogicExpr::Comparative {
                adjective: adj,
                subject: self.ctx.terms.alloc(Term::Constant(subject.noun)),
                object: obj_term,
                difference,
            });

            let result = self.wrap_with_definiteness(subject.definiteness, subject.noun, result)?;
            return self.wrap_with_definiteness_for_object(object.definiteness, object.noun, result);
        };

        // For number comparisons, create a simple Comparative expression
        Ok(self.ctx.exprs.alloc(LogicExpr::Comparative {
            adjective: adj,
            subject: self.ctx.terms.alloc(Term::Constant(subject.noun)),
            object: object_term,
            difference,
        }))
    }

    fn check_number(&self) -> bool {
        matches!(self.peek().kind, TokenType::Number(_))
    }

    fn parse_measure_phrase(&mut self) -> ParseResult<&'a Term<'a>> {
        let num_sym = if let TokenType::Number(sym) = self.advance().kind {
            sym
        } else {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedNumber,
                span: self.current_span(),
            });
        };

        let num_str = self.interner.resolve(num_sym);
        let kind = parse_number_kind(num_str, num_sym);

        let (unit, dimension) = if self.check_content_word() {
            let unit_word = self.consume_content_word()?;
            let unit_str = self.interner.resolve(unit_word).to_lowercase();
            let dim = lexicon::lookup_unit_dimension(&unit_str);
            (Some(unit_word), dim)
        } else {
            (None, None)
        };

        Ok(self.ctx.terms.alloc(Term::Value { kind, unit, dimension }))
    }
}

fn parse_number_kind(s: &str, sym: crate::intern::Symbol) -> NumberKind {
    if s.contains('.') {
        NumberKind::Real(s.parse().unwrap_or(0.0))
    } else if s.chars().all(|c| c.is_ascii_digit() || c == '-') {
        NumberKind::Integer(s.parse().unwrap_or(0))
    } else {
        NumberKind::Symbolic(sym)
    }
}
