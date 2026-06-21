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

use super::clause::ClauseParsing;
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
        negated: bool,
    ) -> ParseResult<&'a LogicExpr<'a>>;

    /// Term-parametric form of [`Self::parse_presupposition`] — the subject is a
    /// TERM (constant or a relativized variable) so the same presupposition
    /// grammar composes over a relative-clause subject ("the person who won
    /// started skydiving 2 years after …").
    fn parse_presupposition_for_term(
        &mut self,
        subject_term: Term<'a>,
        presup_kind: PresupKind,
        negated: bool,
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

    /// Parses the equative "X is as ADJ as Y" → an at-least (`≥`) comparison.
    fn parse_equative(&mut self, subject: &NounPhrase<'a>) -> ParseResult<&'a LogicExpr<'a>>;

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

    /// Recognises a digit-led COUNTING noun phrase in object position —
    /// `Number (adjective)+ Noun` ("6 brown manatees", "49 previous jumps") —
    /// and returns its integer count.
    ///
    /// The discriminator is the intervening adjective. A measure phrase is
    /// `Number Unit` with no adjective ("190 points", "385 degrees"), so a bare
    /// `Number Noun` stays a measure (the count and noun are preserved in the
    /// [`Term::Value`] — no meaning loss, no regression). Only when an adjective
    /// sits between the number and the head noun is the phrase unambiguously a
    /// counting NP, which the caller routes through the canonical
    /// cardinal-quantified-object machinery as `∃=n y(Noun(y) ∧ Adj(y) ∧ …)`
    /// rather than mis-reading the adjective as a measure unit.
    fn counting_np_lookahead(&self) -> Option<u32>;
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
        negated: bool,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        // Delegate to the term-parametric form so the SAME presupposition grammar
        // ("started skydiving 2 years after …") composes over a relativized
        // variable too ("the person WHO WON started skydiving …"), not only a
        // constant subject — LIFT AND SHIFT instead of duplicating the dispatch.
        self.parse_presupposition_for_term(Term::Constant(subject.noun), presup_kind, negated)
    }

    fn parse_presupposition_for_term(
        &mut self,
        subject_term: Term<'a>,
        presup_kind: PresupKind,
        negated: bool,
    ) -> ParseResult<&'a LogicExpr<'a>> {

        let unknown = self.interner.intern("?");
        let complement_verb = if self.check_verb() {
            Some(self.consume_verb())
        } else {
            None
        };
        let complement = match complement_verb {
            Some(verb) => self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice([subject_term]),
                world: None,
            }),
            None => self.ctx.exprs.alloc(LogicExpr::Atom(unknown)),
        };
        // The presupposed clause is a real past EVENT — the same shape the
        // standalone sentence parses to ("Mary lied." →
        // ∃e(Lie(e) ∧ Agent(e, Mary) ∧ Past(e))) — so the projected content
        // is derivable by the proof engine, not just printable.
        let past_event = match complement_verb {
            Some(verb) => {
                use crate::ast::logic::NeoEventData;
                use crate::ast::ThematicRole;
                self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var: self.interner.intern("e"),
                    verb,
                    roles: self
                        .ctx
                        .roles
                        .alloc_slice(vec![(ThematicRole::Agent, subject_term)]),
                    modifiers: self.ctx.syms.alloc_slice(vec![self.interner.intern("Past")]),
                    suppress_existential: false,
                    world: None,
                })))
            }
            None => self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: TemporalOperator::Past,
                body: complement,
            }),
        };

        let (mut assertion, presupposition) = match presup_kind {
            PresupKind::Stop => {
                let neg = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                    op: TokenType::Not,
                    operand: complement,
                });
                (neg, past_event)
            }
            PresupKind::Start => {
                let neg_past = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                    op: TokenType::Not,
                    operand: past_event,
                });
                (complement, neg_past)
            }
            PresupKind::Regret => {
                let regret_sym = self.interner.intern("Regret");
                let regret = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: regret_sym,
                    args: self.ctx.terms.alloc_slice([subject_term]),
                    world: None,
                });
                (regret, past_event)
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
                    args: self.ctx.terms.alloc_slice([subject_term]),
                    world: None,
                });
                (main, complement)
            }
        };

        // Trailing temporal-offset adjunct on the complement event ("started
        // skydiving 2 YEARS AFTER Leslie", "started skydiving sometime BEFORE
        // Faye") attaches to the asserted clause; without this it strands.
        let subj_term = subject_term;
        if let Some(off) = self.parse_temporal_offset_constraint(subj_term)? {
            assertion = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: assertion,
                op: TokenType::And,
                right: off,
            });
        } else if let Some(off) = self.parse_bare_temporal_constraint(subj_term)? {
            assertion = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: assertion,
                op: TokenType::And,
                right: off,
            });
        }

        // Van der Sandt projection: under negation the assertion is negated but the
        // PRESUPPOSITION projects (survives outside the ¬). "Mary doesn't regret
        // lying." → ¬Regret(Mary) [Presup: P(Lie(Mary))] — she still lied.
        let assertion = if negated {
            self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: assertion,
            })
        } else {
            assertion
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
        } else if matches!(self.peek().kind, TokenType::Is | TokenType::Are) {
            // Copular predication under focus: "Only dogs are red." →
            // Only(D, Red(Dogs)), mirroring the verbal "Only dogs bark."
            self.advance();
            if self.check_article() {
                self.advance();
            }
            let predicate = match self.advance().kind.clone() {
                TokenType::Adjective(sym)
                | TokenType::Noun(sym)
                | TokenType::ProperName(sym) => sym,
                TokenType::Ambiguous { primary, alternatives } => {
                    let as_predicate = |t: &TokenType| match t {
                        TokenType::Adjective(sym)
                        | TokenType::Noun(sym)
                        | TokenType::ProperName(sym) => Some(*sym),
                        _ => None,
                    };
                    match as_predicate(&primary)
                        .or_else(|| alternatives.iter().find_map(as_predicate))
                    {
                        Some(sym) => sym,
                        None => {
                            return Err(ParseError {
                                kind: ParseErrorKind::ExpectedContentWord {
                                    found: *primary,
                                },
                                span: self.current_span(),
                            });
                        }
                    }
                }
                found => {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedContentWord { found },
                        span: self.current_span(),
                    });
                }
            };
            Ok(self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: predicate,
                args: self.ctx.terms.alloc_slice([Term::Constant(subject.noun)]),
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

        // A bare verb keeps the simple predication shape; anything after it
        // ("almost killed Mary") takes the full VP grammar under the operator.
        let clause_ends_after_verb = matches!(
            self.tokens.get(self.current + 1).map(|t| t.kind.clone()),
            Some(
                TokenType::Period
                    | TokenType::Exclamation
                    | TokenType::EOF
                    | TokenType::Comma
                    | TokenType::And
            ) | None
        );
        if !clause_ends_after_verb {
            use super::verb::LogicVerbParsing;
            let body = self.parse_predicate_with_subject(subject.noun)?;
            return Ok(self.ctx.exprs.alloc(LogicExpr::Scopal { operator, body }));
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
        // A degree adverb ("somewhat shorter", "slightly wider", "much taller")
        // may precede the comparative; it stresses the gap but adds no measurable
        // offset, so skip it — the strict inequality the comparative yields already
        // captures "more than, by an unspecified amount". Degree adverbs are a
        // closed lexical class (lexicon `degree_adverbs`).
        if crate::lexicon::is_degree_adverb(
            &self.interner.resolve(self.peek().lexeme).to_lowercase(),
        ) && matches!(
            self.tokens.get(self.current + 1).map(|t| &t.kind),
            Some(TokenType::Comparative(_))
        ) {
            self.advance(); // degree adverb
        }

        let comp_tok = self.advance().clone();
        let comp_surface = self.interner.resolve(comp_tok.lexeme).to_string();
        let adj = if let TokenType::Comparative(adj) = comp_tok.kind {
            adj
        } else {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedComparativeAdjective,
                span: self.current_span(),
            });
        };

        if !self.check(&TokenType::Than) {
            // A bare comparative predicate with no standard ("X is older", "one is
            // taller", "the other is faster") — the comparative adjective is a
            // unary property relative to a contextually-implied standard (the
            // other entity in a pair). Build Older(X) — the COMPARATIVE surface
            // ("older"), not the base lemma ("Old"), so the degree isn't lost —
            // rather than failing; the constraint is preserved (zero meaning
            // loss), and in an of-pair / list context the complementary
            // predicates pair up for the prover.
            let comp_name = {
                let mut c = comp_surface.chars();
                match c.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    None => comp_surface.clone(),
                }
            };
            return Ok(self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: self.interner.intern(&comp_name),
                args: self.ctx.terms.alloc_slice([Term::Constant(subject.noun)]),
                world: None,
            }));
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
            // Parse noun phrase as the comparison target — GREEDY so the standard's
            // PPs / reduced relatives attach ("shorter than the figure WITH THE
            // YELLOW HAT", "smaller than the tank GOING TO PHILO"). A "than"
            // standard is nominal — a verb-word head there is a deverbal noun
            // ("larger than the orange PACK").
            let saved_ctx = self.nominal_np_context;
            self.nominal_np_context = true;
            let object_result = self.parse_noun_phrase(true);
            self.nominal_np_context = saved_ctx;
            let object = object_result?;

            // Comparative subdeletion (§2.4): a clausal than-complement with its OWN
            // gradable dimension — "than the door is WIDE" → compare the matrix
            // length-degree to the than-clause width-degree:
            //   ∃d∃d'(Long(desk,d) ∧ Wide(door,d') ∧ d > d').
            if matches!(
                self.peek().kind,
                TokenType::Is | TokenType::Are | TokenType::Was | TokenType::Were
            ) {
                let save = self.current;
                self.advance(); // copula
                if let TokenType::Adjective(adj2) = self.peek().kind {
                    self.advance();
                    let d1 = self.next_var_name();
                    let d2 = self.next_var_name();
                    let matrix = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: adj,
                        args: self
                            .ctx
                            .terms
                            .alloc_slice([Term::Constant(subject.noun), Term::Variable(d1)]),
                        world: None,
                    });
                    let than_clause = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: adj2,
                        args: self
                            .ctx
                            .terms
                            .alloc_slice([Term::Constant(object.noun), Term::Variable(d2)]),
                        world: None,
                    });
                    let gt = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: self.interner.intern(">"),
                        args: self
                            .ctx
                            .terms
                            .alloc_slice([Term::Variable(d1), Term::Variable(d2)]),
                        world: None,
                    });
                    let conj1 = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: matrix,
                        op: TokenType::And,
                        right: than_clause,
                    });
                    let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: conj1,
                        op: TokenType::And,
                        right: gt,
                    });
                    let inner = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                        kind: crate::ast::logic::QuantifierKind::Existential,
                        variable: d2,
                        body,
                        island_id: self.current_island,
                    });
                    let outer = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                        kind: crate::ast::logic::QuantifierKind::Existential,
                        variable: d1,
                        body: inner,
                        island_id: self.current_island,
                    });
                    return Ok(outer);
                }
                // Clausal ellipsis ("taller than Bill is."): the bare copula
                // adds nothing — keep it consumed at a clause boundary,
                // otherwise restore for other readings.
                if self.at_clause_boundary() {
                    // copula consumed; comparison proceeds as phrasal
                } else {
                    self.current = save;
                }
            }

            // SUBJECT side: a descriptive subject (adjectives / possessor / PPs / a
            // reduced relative) becomes a DISTINCT existential entity carrying its
            // restrictor — mirroring the standard side — so "the fall Derrick
            // photographed in 1987 is shorter than …" keeps the reduced relative on
            // the subject instead of collapsing to the bare head constant. The
            // variable is used DIRECTLY in the comparison (no later substitution
            // needed); a bare-head subject stays a constant under the definiteness wrap.
            let subj_is_desc = !subject.adjectives.is_empty()
                || subject.possessor.is_some()
                || !subject.pps.is_empty();
            let subj_var = if subj_is_desc { Some(self.next_var_name()) } else { None };
            let subj_term = match subj_var {
                Some(sv) => Term::Variable(sv),
                None => Term::Constant(subject.noun),
            };

            // A standard carrying restrictions (adjectives, possessor, PPs, or a
            // who/that relative clause) becomes a DISTINCT existential entity with a
            // restrictor so nothing is dropped — same distinct-identity pattern as
            // the arithmetic comparative / of-pair. A bare name/definite-head stays
            // a constant.
            let has_rel = self.check(&TokenType::Who) || self.check(&TokenType::That);
            let obj_is_desc = has_rel
                || !object.adjectives.is_empty()
                || object.possessor.is_some()
                || !object.pps.is_empty();
            if obj_is_desc {
                let obj_var = self.next_var_name();
                let obj_var_term = Term::Variable(obj_var);
                let mut restrictor = self.nominal_predication_with_pps(obj_var_term, &object);
                if has_rel {
                    self.advance(); // "who" / "that"
                    let rel = self.parse_relative_clause(obj_var)?;
                    restrictor = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: restrictor,
                        op: TokenType::And,
                        right: rel,
                    });
                }
                let cmp = self.ctx.exprs.alloc(LogicExpr::Comparative {
                    adjective: adj,
                    subject: self.ctx.terms.alloc(subj_term),
                    object: self.ctx.terms.alloc(obj_var_term),
                    difference,
                    relation: crate::ast::ComparisonRelation::Greater,
                });
                let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: restrictor,
                    op: TokenType::And,
                    right: cmp,
                });
                let quant = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: crate::ast::logic::QuantifierKind::Existential,
                    variable: obj_var,
                    body,
                    island_id: self.current_island,
                });
                return match subj_var {
                    Some(sv) => {
                        let subj_restrictor =
                            self.nominal_predication_with_pps(Term::Variable(sv), subject);
                        let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: subj_restrictor,
                            op: TokenType::And,
                            right: quant,
                        });
                        Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                            kind: crate::ast::logic::QuantifierKind::Existential,
                            variable: sv,
                            body,
                            island_id: self.current_island,
                        }))
                    }
                    None => self.wrap_with_definiteness(subject.definiteness, subject.noun, quant),
                };
            }

            let obj_term = self.ctx.terms.alloc(Term::Constant(object.noun));

            let result = self.ctx.exprs.alloc(LogicExpr::Comparative {
                adjective: adj,
                subject: self.ctx.terms.alloc(subj_term),
                object: obj_term,
                difference,
                relation: crate::ast::ComparisonRelation::Greater,
            });

            let result = match subj_var {
                Some(sv) => {
                    let subj_restrictor =
                        self.nominal_predication_with_pps(Term::Variable(sv), subject);
                    let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: subj_restrictor,
                        op: TokenType::And,
                        right: result,
                    });
                    self.ctx.exprs.alloc(LogicExpr::Quantifier {
                        kind: crate::ast::logic::QuantifierKind::Existential,
                        variable: sv,
                        body,
                        island_id: self.current_island,
                    })
                }
                None => self.wrap_with_definiteness(subject.definiteness, subject.noun, result)?,
            };
            return self.wrap_with_definiteness_for_object(object.definiteness, object.noun, result);
        };

        // For number comparisons, create a simple Comparative expression
        Ok(self.ctx.exprs.alloc(LogicExpr::Comparative {
            adjective: adj,
            subject: self.ctx.terms.alloc(Term::Constant(subject.noun)),
            object: object_term,
            difference,
            relation: crate::ast::ComparisonRelation::Greater,
        }))
    }

    /// Parses the equative frame "X is as ADJ as Y" → an at-least (`≥`) degree
    /// comparison. The parser is positioned at the first "as"; the subject's
    /// definiteness wrapping is applied by the caller.
    fn parse_equative(
        &mut self,
        subject: &NounPhrase<'a>,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        self.advance(); // consume the first "as"
        // The gradable dimension (an adjective).
        let adj = match self.advance().kind.clone() {
            TokenType::Adjective(a) => a,
            TokenType::Comparative(a) => a,
            other => {
                // Use the lexeme as the dimension if it is not a plain adjective token.
                if let TokenType::Noun(a) = other {
                    a
                } else {
                    self.interner.intern(&self.interner.resolve(self.previous().lexeme).to_string())
                }
            }
        };
        // Second "as".
        if self.interner.resolve(self.peek().lexeme).eq_ignore_ascii_case("as") {
            self.advance();
        }
        let object = self.parse_noun_phrase(false)?;
        let obj_term = self.ctx.terms.alloc(Term::Constant(object.noun));
        let result = self.ctx.exprs.alloc(LogicExpr::Comparative {
            adjective: adj,
            subject: self.ctx.terms.alloc(Term::Constant(subject.noun)),
            object: obj_term,
            difference: None,
            relation: crate::ast::ComparisonRelation::GreaterEqual,
        });
        let result = self.wrap_with_definiteness(subject.definiteness, subject.noun, result)?;
        self.wrap_with_definiteness_for_object(object.definiteness, object.noun, result)
    }

    fn check_number(&self) -> bool {
        // A clock time ("9:30am", "8:15 pm") is a measure-like value too — it names
        // a point on the day's timeline that the prover can order, so it is a valid
        // measure-phrase / PP object ("is at 9:30am", "the meeting at 8:15 pm").
        matches!(
            self.peek().kind,
            TokenType::Number(_) | TokenType::TimeLiteral { .. }
        )
    }

    fn parse_measure_phrase(&mut self) -> ParseResult<&'a Term<'a>> {
        // A clock-time literal is a time-of-day VALUE the prover can order against
        // other times — represented as minutes-from-midnight (an integer on the
        // day's timeline) tagged with a `ClockTime` dimension, so "is at 9:30am"
        // and "the 8:15 pm event" compare numerically. (TODO: timezone-aware times
        // — carry an offset/zone on the value — when the corpus needs them.)
        if let TokenType::TimeLiteral { nanos_from_midnight } = self.peek().kind {
            self.advance();
            let minutes = (nanos_from_midnight / 60_000_000_000) as i64;
            return Ok(self.ctx.terms.alloc(Term::Value {
                kind: crate::ast::logic::NumberKind::Integer(minutes),
                unit: None,
                dimension: Some(crate::ast::logic::Dimension::Time),
            }));
        }
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

        // The unit noun after the number ("385 degrees", "5 dollars", "190
        // points"). An INFLECTED verb (past/future) is never a unit — it is the
        // matrix predicate ("issued in 1850 sold …" must keep "sold" as the verb,
        // not read "1850 sold" as a measure), so it is left unconsumed.
        let next_is_inflected_verb = match &self.peek().kind {
            TokenType::Verb { time, .. } => matches!(time, Time::Past | Time::Future),
            TokenType::Ambiguous { primary, .. } => {
                matches!(**primary, TokenType::Verb { time, .. } if matches!(time, Time::Past | Time::Future))
            }
            _ => false,
        };
        let (unit, dimension) = if matches!(self.peek().kind, TokenType::CalendarUnit(_)) {
            // A calendar unit ("years", "months", "days") is a measure unit too:
            // "12 years old", "3 weeks late". (The COUNT-UNIT-after/before temporal
            // OFFSET is caught earlier in try_temporal_offset, so this only fires
            // for the non-offset measure use.)
            let unit_word = self.peek().lexeme;
            self.advance();
            (Some(unit_word), None)
        } else if self.check_content_word() && !self.check_article() && !next_is_inflected_verb {
            // An ARTICLE after the number is never the unit — it starts a rate
            // denominator ("$700 A month") or a following NP; leave it unconsumed.
            let unit_word = self.consume_content_word()?;
            let unit_str = self.interner.resolve(unit_word).to_lowercase();
            let dim = lexicon::lookup_unit_dimension(&unit_str);
            (Some(unit_word), dim)
        } else {
            (None, None)
        };

        Ok(self.ctx.terms.alloc(Term::Value { kind, unit, dimension }))
    }

    fn counting_np_lookahead(&self) -> Option<u32> {
        let n = match self.peek().kind {
            TokenType::Number(sym) => self.interner.resolve(sym).parse::<u32>().ok()?,
            _ => return None,
        };
        // Scan ≥1 modifier (adjective or a ProperName premodifier), then require a
        // common-noun head. The modifier is what proves this is a count — "6 BROWN
        // manatees", "640 TWITTER followers", "78 LINKEDIN connections" — not a
        // measure ("190 points" has no modifier and stays on the measure path).
        let mut i = self.current + 1;
        let mut saw_modifier = false;
        while matches!(
            self.tokens.get(i).map(|t| &t.kind),
            Some(TokenType::Adjective(_))
                | Some(TokenType::NonIntersectiveAdjective(_))
                | Some(TokenType::ProperName(_))
        ) {
            saw_modifier = true;
            i += 1;
        }
        if !saw_modifier {
            return None;
        }
        let head_is_noun = match self.tokens.get(i).map(|t| &t.kind) {
            Some(TokenType::Noun(_)) | Some(TokenType::Item) | Some(TokenType::Items) => true,
            // A verb-word head ("49 previous JUMPS", "six previous RUNS") is a
            // DEVERBAL NOUN here — a number followed by an adjective cannot
            // precede a finite verb, so the head is nominal. The object-NP path
            // recovers it as the head (via nominal_np_context).
            Some(TokenType::Verb { .. }) => true,
            Some(TokenType::Ambiguous { primary, alternatives }) => {
                matches!(**primary, TokenType::Noun(_) | TokenType::Verb { .. })
                    || alternatives
                        .iter()
                        .any(|t| matches!(t, TokenType::Noun(_) | TokenType::Verb { .. }))
            }
            _ => false,
        };
        head_is_noun.then_some(n)
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
