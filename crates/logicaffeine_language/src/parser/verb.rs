//! Verb phrase parsing with event semantics and thematic roles.
//!
//! This module handles the core verbal predication including:
//!
//! - **Intransitive verbs**: "John runs" → `∃e(Run(e) ∧ Agent(e,John))`
//! - **Transitive verbs**: "John loves Mary" → `∃e(Love(e) ∧ Agent(e,John) ∧ Theme(e,Mary))`
//! - **Ditransitive verbs**: "John gives Mary a book" → with Goal role
//! - **Copula constructions**: "John is tall", "John is a doctor"
//! - **Control verbs**: "John wants to run" → raising/control structures
//! - **Plural subjects**: "John and Mary run", "John and Mary love each other"
//! - **VP respectively**: "John and Mary love Sue and Bill respectively"
//!
//! # Neo-Davidsonian Event Semantics
//!
//! Verbs introduce event variables with thematic roles:
//! - **Agent**: The doer of the action
//! - **Theme/Patient**: The entity affected
//! - **Goal/Recipient**: The target of transfer
//! - **Instrument**: The tool used
//!
//! Events are represented using `LogicExpr::NeoEvent` with a verb symbol and
//! a list of (ThematicRole, Term) pairs.

use super::clause::ClauseParsing;
use super::modal::ModalParsing;
use super::noun::NounParsing;
use super::pragmatics::PragmaticsParsing;
use super::quantifier::QuantifierParsing;
use super::{ParseResult, Parser};
use crate::ast::{
    AspectOperator, LogicExpr, NeoEventData, NounPhrase, QuantifierKind, TemporalOperator, Term,
    ThematicRole,
};
use crate::drs::{Gender, Number, ReferentSource};
use crate::error::{ParseError, ParseErrorKind};
use logicaffeine_base::Symbol;
use crate::lexer::Lexer;
use crate::lexicon::{Aspect, Definiteness, Time};
use crate::token::{FocusKind, Span, TokenType};

use crate::ast::Stmt;

/// Trait for parsing verb phrases in declarative (logic) mode.
///
/// Provides methods for parsing predicates with subjects, control verbs,
/// and plural/group constructions with Neo-Davidsonian event semantics.
pub trait LogicVerbParsing<'a, 'ctx, 'int> {
    /// Parses a verb phrase given the subject as a constant symbol.
    fn parse_predicate_with_subject(&mut self, subject_symbol: Symbol)
        -> ParseResult<&'a LogicExpr<'a>>;
    /// Parses a verb phrase with subject as a bound variable.
    fn parse_predicate_with_subject_as_var(&mut self, subject_symbol: Symbol)
        -> ParseResult<&'a LogicExpr<'a>>;
    /// Attempts to parse a plural subject ("John and Mary verb").
    /// Returns `Ok(Some(expr))` on success, `Ok(None)` if not plural, `Err` on semantic error.
    fn try_parse_plural_subject(&mut self, first_subject: &NounPhrase<'a>)
        -> Result<Option<&'a LogicExpr<'a>>, ParseError>;
    /// Parses control verb structures: "wants to VP", "persuaded X to VP".
    fn parse_control_structure(
        &mut self,
        subject: &NounPhrase<'a>,
        verb: Symbol,
        verb_time: Time,
    ) -> ParseResult<&'a LogicExpr<'a>>;
    /// Checks if a verb is a control verb (want, try, persuade, etc.).
    fn is_control_verb(&self, verb: Symbol) -> bool;
    /// Builds a predicate for intransitive verbs with multiple subjects.
    fn build_group_predicate(
        &mut self,
        subjects: &[Symbol],
        verb: Symbol,
        verb_time: Time,
    ) -> &'a LogicExpr<'a>;
    /// Builds a transitive predicate with group subject and group object.
    fn build_group_transitive(
        &mut self,
        subjects: &[Symbol],
        objects: &[Symbol],
        verb: Symbol,
        verb_time: Time,
    ) -> &'a LogicExpr<'a>;
}

/// Trait for parsing verb phrases in imperative (LOGOS) mode.
///
/// Provides methods for parsing statements rather than logical propositions.
pub trait ImperativeVerbParsing<'a, 'ctx, 'int> {
    /// Parses a statement with the given subject symbol.
    fn parse_statement_with_subject(&mut self, subject_symbol: Symbol)
        -> ParseResult<Stmt<'a>>;
}

impl<'a, 'ctx, 'int> Parser<'a, 'ctx, 'int> {
    fn parse_predicate_impl(
        &mut self,
        subject_symbol: Symbol,
        as_variable: bool,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        let subject_term = if as_variable {
            Term::Variable(subject_symbol)
        } else {
            Term::Constant(subject_symbol)
        };

        // Weather verb + expletive "it" detection: "it rains" → ∃e(Rain(e))
        let subject_str = self.interner.resolve(subject_symbol).to_lowercase();
        if subject_str == "it" && self.check_verb() {
            if let TokenType::Verb { lemma, time, .. } = &self.peek().kind {
                let lemma_str = self.interner.resolve(*lemma);
                if Lexer::is_weather_verb(lemma_str) {
                    let verb = *lemma;
                    let verb_time = *time;
                    self.advance(); // consume the weather verb

                    let event_var = self.get_event_var();
                    let suppress_existential = self.drs.in_conditional_antecedent();
                    if suppress_existential {
                        let event_class = self.interner.intern("Event");
                        self.drs.introduce_referent(event_var, event_class, Gender::Neuter, Number::Singular);
                    }
                    let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                        event_var,
                        verb,
                        roles: self.ctx.roles.alloc_slice(vec![]), // No thematic roles
                        modifiers: self.ctx.syms.alloc_slice(vec![]),
                        suppress_existential,
                        world: None,
                    })));

                    return Ok(match verb_time {
                        Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                            operator: TemporalOperator::Past,
                            body: neo_event,
                        }),
                        Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                            operator: TemporalOperator::Future,
                            body: neo_event,
                        }),
                        _ => neo_event,
                    });
                }
            }
        }

        // Weather adjective + expletive "it" detection: "it is wet" → Wet
        // Also handle "it's wet" where 's is Possessive token
        if subject_str == "it" && (self.check(&TokenType::Is) || self.check(&TokenType::Was) || self.check(&TokenType::Possessive)) {
            let saved_pos = self.current;
            self.advance(); // consume copula

            if self.check_content_word() {
                let adj_lexeme = self.peek().lexeme;
                let adj_str = self.interner.resolve(adj_lexeme).to_lowercase();

                if let Some(meta) = crate::lexicon::lookup_adjective_db(&adj_str) {
                    if meta.features.contains(&crate::lexicon::Feature::Weather) {
                        let adj_sym = self.consume_content_word().unwrap_or(adj_lexeme);
                        // Atmospheric predicate: "it is wet" → Wet
                        return Ok(self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: adj_sym,
                            args: self.ctx.terms.alloc_slice([]),
                            world: None,
                        }));
                    }
                }
            }
            // Not a weather adjective, restore position
            self.current = saved_pos;
        }

        if self.check(&TokenType::Never) {
            self.advance();
            let verb = self.consume_verb();
            let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice([subject_term]),
                world: None,
            });
            return Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: verb_pred,
            }));
        }

        if self.check_modal() {
            return self.parse_aspect_chain_with_term(subject_term.clone());
        }

        if self.check_content_word() {
            let next_word = self.interner.resolve(self.peek().lexeme).to_lowercase();
            if next_word == "has" || next_word == "have" || next_word == "had" {
                // Look ahead to distinguish perfect aspect ("has eaten") from possession ("has 3 children")
                // Perfect aspect: has/have/had + verb
                // Possession: has/have/had + number/noun
                let is_perfect_aspect = if self.current + 1 < self.tokens.len() {
                    let next_token = &self.tokens[self.current + 1].kind;
                    matches!(
                        next_token,
                        TokenType::Verb { .. } | TokenType::Not
                    ) && !matches!(next_token, TokenType::Number(_))
                } else {
                    false
                };
                if is_perfect_aspect {
                    return self.parse_aspect_chain(subject_symbol);
                }
                // Otherwise, treat "has" as a main verb (possession) and continue below
            }
        }

        if self.check(&TokenType::Had) {
            return self.parse_aspect_chain(subject_symbol);
        }

        // Handle do-support: "I do/don't know who"
        if self.check(&TokenType::Does) || self.check(&TokenType::Do) {
            self.advance();
            let is_negated = self.match_token(&[TokenType::Not]);

            if self.check(&TokenType::Ever) {
                self.advance();
            }

            if self.check_verb() {
                let verb = self.consume_verb();

                // Check for embedded wh-clause with sluicing: "I don't know who"
                if self.check_wh_word() {
                    let wh_token = self.advance().kind.clone();
                    let is_who = matches!(wh_token, TokenType::Who);
                    let is_what = matches!(wh_token, TokenType::What);

                    let is_sluicing = self.is_at_end() ||
                        self.check(&TokenType::Period) ||
                        self.check(&TokenType::Comma);

                    if is_sluicing {
                        if let Some(template) = self.last_event_template.clone() {
                            let wh_var = self.next_var_name();

                            let roles: Vec<_> = if is_who {
                                std::iter::once((ThematicRole::Agent, Term::Variable(wh_var)))
                                    .chain(template.non_agent_roles.iter().cloned())
                                    .collect()
                            } else if is_what {
                                vec![
                                    (ThematicRole::Agent, subject_term.clone()),
                                    (ThematicRole::Theme, Term::Variable(wh_var)),
                                ]
                            } else {
                                std::iter::once((ThematicRole::Agent, Term::Variable(wh_var)))
                                    .chain(template.non_agent_roles.iter().cloned())
                                    .collect()
                            };

                            let event_var = self.get_event_var();
                            let suppress_existential = self.drs.in_conditional_antecedent();
                            let reconstructed = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                                event_var,
                                verb: template.verb,
                                roles: self.ctx.roles.alloc_slice(roles),
                                modifiers: self.ctx.syms.alloc_slice(template.modifiers.clone()),
                                suppress_existential,
                                world: None,
                            })));

                            let question = self.ctx.exprs.alloc(LogicExpr::Question {
                                wh_variable: wh_var,
                                body: reconstructed,
                            });

                            let know_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                                event_var: self.get_event_var(),
                                verb,
                                roles: self.ctx.roles.alloc_slice(vec![
                                    (ThematicRole::Agent, subject_term.clone()),
                                    (ThematicRole::Theme, Term::Proposition(question)),
                                ]),
                                modifiers: self.ctx.syms.alloc_slice(vec![]),
                                suppress_existential,
                                world: None,
                            })));

                            let result = if is_negated {
                                self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                                    op: TokenType::Not,
                                    operand: know_event,
                                })
                            } else {
                                know_event
                            };

                            return Ok(result);
                        }
                    }
                }

                // Regular do-support: "I do run" or "I don't run"
                let roles: Vec<(ThematicRole, Term<'a>)> = vec![(ThematicRole::Agent, subject_term.clone())];
                let modifiers: Vec<Symbol> = vec![];
                let event_var = self.get_event_var();
                let suppress_existential = self.drs.in_conditional_antecedent();

                let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var,
                    verb,
                    roles: self.ctx.roles.alloc_slice(roles),
                    modifiers: self.ctx.syms.alloc_slice(modifiers),
                    suppress_existential,
                    world: None,
                })));

                if is_negated {
                    return Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand: neo_event,
                    }));
                }
                return Ok(neo_event);
            }
        }

        // Check for auxiliary (like "did" in "did not bark")
        // BUT: "did it" should be parsed as verb "do" with object "it"
        // We lookahead to check if this is truly an auxiliary usage
        if self.check_auxiliary() && self.is_true_auxiliary_usage() {
            let aux_time = if let TokenType::Auxiliary(time) = self.advance().kind {
                time
            } else {
                Time::None
            };
            self.pending_time = Some(aux_time);

            if self.match_token(&[TokenType::Not]) {
                self.negative_depth += 1;

                // Check for verb or "do" (TokenType::Do is separate from TokenType::Verb)
                if self.check_verb() || self.check(&TokenType::Do) {
                    let verb = if self.check(&TokenType::Do) {
                        self.advance(); // consume "do"
                        self.interner.intern("Do")
                    } else {
                        self.consume_verb()
                    };

                    if self.check_quantifier() {
                        let quantifier_token = self.advance().kind.clone();
                        let object_np = self.parse_noun_phrase(false)?;
                        let obj_var = self.next_var_name();

                        let obj_restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: object_np.noun,
                            args: self.ctx.terms.alloc_slice([Term::Variable(obj_var)]),
                            world: None,
                        });

                        let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: verb,
                            args: self
                                .ctx
                                .terms
                                .alloc_slice([subject_term, Term::Variable(obj_var)]),
                            world: None,
                        });

                        let (kind, body) = match quantifier_token {
                            TokenType::Any => {
                                if self.is_negative_context() {
                                    (
                                        QuantifierKind::Existential,
                                        self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                            left: obj_restriction,
                                            op: TokenType::And,
                                            right: verb_pred,
                                        }),
                                    )
                                } else {
                                    (
                                        QuantifierKind::Universal,
                                        self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                            left: obj_restriction,
                                            op: TokenType::Implies,
                                            right: verb_pred,
                                        }),
                                    )
                                }
                            }
                            TokenType::Some => (
                                QuantifierKind::Existential,
                                self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                    left: obj_restriction,
                                    op: TokenType::And,
                                    right: verb_pred,
                                }),
                            ),
                            TokenType::All => (
                                QuantifierKind::Universal,
                                self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                    left: obj_restriction,
                                    op: TokenType::Implies,
                                    right: verb_pred,
                                }),
                            ),
                            _ => (
                                QuantifierKind::Existential,
                                self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                    left: obj_restriction,
                                    op: TokenType::And,
                                    right: verb_pred,
                                }),
                            ),
                        };

                        let quantified = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                            kind,
                            variable: obj_var,
                            body,
                            island_id: self.current_island,
                        });

                        let effective_time = self.pending_time.take().unwrap_or(Time::None);
                        let with_time = match effective_time {
                            Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                                operator: TemporalOperator::Past,
                                body: quantified,
                            }),
                            Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                                operator: TemporalOperator::Future,
                                body: quantified,
                            }),
                            _ => quantified,
                        };

                        self.negative_depth -= 1;
                        return Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                            op: TokenType::Not,
                            operand: with_time,
                        }));
                    }

                    if self.check_npi_object() {
                        let npi_token = self.advance().kind.clone();
                        let obj_var = self.next_var_name();

                        let restriction_name = match npi_token {
                            TokenType::Anything => "Thing",
                            TokenType::Anyone => "Person",
                            _ => "Thing",
                        };

                        let restriction_sym = self.interner.intern(restriction_name);
                        let obj_restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: restriction_sym,
                            args: self.ctx.terms.alloc_slice([Term::Variable(obj_var)]),
                            world: None,
                        });

                        let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: verb,
                            args: self.ctx.terms.alloc_slice([subject_term, Term::Variable(obj_var)]),
                            world: None,
                        });

                        let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: obj_restriction,
                            op: TokenType::And,
                            right: verb_pred,
                        });

                        let quantified = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                            kind: QuantifierKind::Existential,
                            variable: obj_var,
                            body,
                            island_id: self.current_island,
                        });

                        let effective_time = self.pending_time.take().unwrap_or(Time::None);
                        let with_time = match effective_time {
                            Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                                operator: TemporalOperator::Past,
                                body: quantified,
                            }),
                            Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                                operator: TemporalOperator::Future,
                                body: quantified,
                            }),
                            _ => quantified,
                        };

                        self.negative_depth -= 1;
                        return Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                            op: TokenType::Not,
                            operand: with_time,
                        }));
                    }

                    let mut roles: Vec<(ThematicRole, Term<'a>)> =
                        vec![(ThematicRole::Agent, subject_term)];

                    // Check for object: NP, article+NP, or pronoun (like "it")
                    if self.check_content_word() || self.check_article() || self.check_pronoun() {
                        if self.check_pronoun() {
                            // Handle pronoun object like "it" in "did not do it"
                            let pronoun_token = self.advance().clone();
                            let term = if let TokenType::Pronoun { gender, number, .. } = pronoun_token.kind {
                                let resolved = self.resolve_pronoun(gender, number)?;
                                match resolved {
                                    super::ResolvedPronoun::Variable(s) => Term::Variable(s),
                                    super::ResolvedPronoun::Constant(s) => Term::Constant(s),
                                }
                            } else {
                                // Fallback to lexeme if somehow not a pronoun token
                                Term::Constant(pronoun_token.lexeme)
                            };
                            roles.push((ThematicRole::Theme, term));
                        } else {
                            let object = self.parse_noun_phrase(false)?;
                            let object_term = self.noun_phrase_to_term(&object);
                            roles.push((ThematicRole::Theme, object_term));
                        }
                    }

                    let event_var = self.get_event_var();
                    let suppress_existential = self.drs.in_conditional_antecedent();
                    let effective_time = self.pending_time.take().unwrap_or(Time::None);
                    let mut modifiers = Vec::new();
                    match effective_time {
                        Time::Past => modifiers.push(self.interner.intern("Past")),
                        Time::Future => modifiers.push(self.interner.intern("Future")),
                        _ => {}
                    }

                    let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                        event_var,
                        verb,
                        roles: self.ctx.roles.alloc_slice(roles),
                        modifiers: self.ctx.syms.alloc_slice(modifiers),
                        suppress_existential,
                        world: None,
                    })));

                    self.negative_depth -= 1;
                    return Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand: neo_event,
                    }));
                }

                self.negative_depth -= 1;
            }
        }

        if self.check(&TokenType::Is)
            || self.check(&TokenType::Are)
            || self.check(&TokenType::Was)
            || self.check(&TokenType::Were)
        {
            let copula_time = if self.check(&TokenType::Was) || self.check(&TokenType::Were) {
                Time::Past
            } else {
                Time::Present
            };
            self.advance();

            // Check for negation: "was not caught", "is not happy"
            let is_negated = self.check(&TokenType::Not);
            if is_negated {
                self.advance(); // consume "not"
            }

            // Check for temporal adverbs after copula: "is eventually Y", "is always Y", "is never Y"
            let mut copula_temporal: Option<super::CopulaTemporal> = None;
            if !is_negated {
                if self.check(&TokenType::Never) {
                    self.advance();
                    copula_temporal = Some(super::CopulaTemporal::Never);
                } else if let TokenType::Adverb(sym) | TokenType::ScopalAdverb(sym) | TokenType::TemporalAdverb(sym) = &self.peek().kind {
                    let resolved = self.interner.resolve(*sym).to_string();
                    if resolved == "Always" || resolved == "always" {
                        self.advance();
                        copula_temporal = Some(super::CopulaTemporal::Always);
                    } else if resolved == "Eventually" || resolved == "eventually" {
                        self.advance();
                        copula_temporal = Some(super::CopulaTemporal::Eventually);
                    }
                }
            }

            if self.check_verb() {
                let (verb, _verb_time, verb_aspect, verb_class) = self.consume_verb_with_metadata();

                // Stative verbs cannot be progressive
                if verb_class.is_stative() && verb_aspect == Aspect::Progressive {
                    return Err(crate::error::ParseError {
                        kind: crate::error::ParseErrorKind::StativeProgressiveConflict,
                        span: self.current_span(),
                    });
                }

                let predicate = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: verb,
                    args: self.ctx.terms.alloc_slice([subject_term]),
                    world: None,
                });

                let with_aspect = if verb_aspect == Aspect::Progressive {
                    // Semelfactive + Progressive → Iterative
                    let operator = if verb_class == crate::lexicon::VerbClass::Semelfactive {
                        AspectOperator::Iterative
                    } else {
                        AspectOperator::Progressive
                    };
                    self.ctx.exprs.alloc(LogicExpr::Aspectual {
                        operator,
                        body: predicate,
                    })
                } else {
                    predicate
                };

                let with_time = if copula_time == Time::Past {
                    self.ctx.exprs.alloc(LogicExpr::Temporal {
                        operator: TemporalOperator::Past,
                        body: with_aspect,
                    })
                } else {
                    with_aspect
                };

                let with_neg = if is_negated {
                    self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand: with_time,
                    })
                } else {
                    with_time
                };

                let result = match copula_temporal {
                    Some(super::CopulaTemporal::Always) => {
                        self.ctx.exprs.alloc(LogicExpr::Temporal {
                            operator: TemporalOperator::Always,
                            body: with_neg,
                        })
                    }
                    Some(super::CopulaTemporal::Never) => {
                        let negated = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                            op: TokenType::Not,
                            operand: with_time,
                        });
                        self.ctx.exprs.alloc(LogicExpr::Temporal {
                            operator: TemporalOperator::Always,
                            body: negated,
                        })
                    }
                    Some(super::CopulaTemporal::Eventually) => {
                        self.ctx.exprs.alloc(LogicExpr::Temporal {
                            operator: TemporalOperator::Eventually,
                            body: with_neg,
                        })
                    }
                    None => with_neg,
                };

                return Ok(result);
            }

            let predicate = self.consume_content_word()?;
            let base_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: predicate,
                args: self.ctx.terms.alloc_slice([subject_term]),
                world: None,
            });

            let with_time = if copula_time == Time::Past {
                self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: TemporalOperator::Past,
                    body: base_pred,
                })
            } else {
                base_pred
            };

            let with_neg = if is_negated {
                self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                    op: TokenType::Not,
                    operand: with_time,
                })
            } else {
                with_time
            };

            let result = match copula_temporal {
                Some(super::CopulaTemporal::Always) => {
                    self.ctx.exprs.alloc(LogicExpr::Temporal {
                        operator: TemporalOperator::Always,
                        body: with_neg,
                    })
                }
                Some(super::CopulaTemporal::Never) => {
                    let negated = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand: with_time,
                    });
                    self.ctx.exprs.alloc(LogicExpr::Temporal {
                        operator: TemporalOperator::Always,
                        body: negated,
                    })
                }
                Some(super::CopulaTemporal::Eventually) => {
                    self.ctx.exprs.alloc(LogicExpr::Temporal {
                        operator: TemporalOperator::Eventually,
                        body: with_neg,
                    })
                }
                None => with_neg,
            };

            return Ok(result);
        }

        // Handle "did it" - when Auxiliary(Past) is used as a transitive verb (past of "do")
        // This happens when we bypassed auxiliary handling because of lookahead
        if self.check_auxiliary_as_main_verb() {
            return self.parse_do_as_main_verb(subject_term);
        }

        if self.check_verb() {
            let (mut verb, verb_time, verb_aspect, verb_class) = self.consume_verb_with_metadata();
            let mut args = vec![subject_term.clone()];

            // Control/raising verb with infinitival complement ("wants to
            // play"): route through the canonical control machinery, then
            // restore the subject's variable-ness so quantified subjects bind
            // into the complement ("Every child wants to play." → W(x, Play(x))).
            if self.is_control_verb(verb) && self.check_to() {
                let subject_np = NounPhrase {
                    noun: subject_symbol,
                    definiteness: None,
                    adjectives: &[],
                    possessor: None,
                    pps: &[],
                    superlative: None,
                };
                let control = self.parse_control_structure(&subject_np, verb, verb_time)?;
                return if as_variable {
                    self.substitute_constant_with_var(control, subject_symbol, subject_symbol)
                } else {
                    Ok(control)
                };
            }

            // Verbal comparative ("runs faster than Bob", "run faster than all
            // cats"): the comparative grades the event participants — the verb
            // event is asserted and the subject compared to the standard.
            if let TokenType::Comparative(comp_adj) = self.peek().kind.clone() {
                if matches!(
                    self.tokens.get(self.current + 1).map(|t| t.kind.clone()),
                    Some(TokenType::Than)
                ) {
                    self.advance(); // comparative
                    self.advance(); // than

                    let event_var = self.get_event_var();
                    let mut modifiers = Vec::new();
                    let effective_time = self.pending_time.take().unwrap_or(verb_time);
                    match effective_time {
                        Time::Past => modifiers.push(self.interner.intern("Past")),
                        Time::Future => modifiers.push(self.interner.intern("Future")),
                        _ => {}
                    }
                    let suppress_existential = self.drs.in_conditional_antecedent();
                    let event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                        event_var,
                        verb,
                        roles: self
                            .ctx
                            .roles
                            .alloc_slice(vec![(ThematicRole::Agent, subject_term.clone())]),
                        modifiers: self.ctx.syms.alloc_slice(modifiers),
                        suppress_existential,
                        world: None,
                    })));

                    let result = if self.check_quantifier() {
                        let q = self.advance().kind.clone();
                        let std_np = self.parse_noun_phrase(false)?;
                        let std_var = self.next_var_name();
                        let restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: std_np.noun,
                            args: self.ctx.terms.alloc_slice([Term::Variable(std_var)]),
                            world: None,
                        });
                        let comparison = self.ctx.exprs.alloc(LogicExpr::Comparative {
                            adjective: comp_adj,
                            subject: self.ctx.terms.alloc(subject_term.clone()),
                            object: self.ctx.terms.alloc(Term::Variable(std_var)),
                            difference: None,
                            relation: crate::ast::logic::ComparisonRelation::Greater,
                        });
                        let (std_kind, std_op) = match q {
                            TokenType::All => (QuantifierKind::Universal, TokenType::Implies),
                            TokenType::Most => (QuantifierKind::Most, TokenType::And),
                            TokenType::Few => (QuantifierKind::Few, TokenType::And),
                            TokenType::Many => (QuantifierKind::Many, TokenType::And),
                            _ => (QuantifierKind::Existential, TokenType::And),
                        };
                        let std_body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: restriction,
                            op: std_op,
                            right: comparison,
                        });
                        let quantified = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                            kind: std_kind,
                            variable: std_var,
                            body: std_body,
                            island_id: self.current_island,
                        });
                        self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: event,
                            op: TokenType::And,
                            right: quantified,
                        })
                    } else {
                        let std_np = self.parse_noun_phrase(false)?;
                        let comparison = self.ctx.exprs.alloc(LogicExpr::Comparative {
                            adjective: comp_adj,
                            subject: self.ctx.terms.alloc(subject_term.clone()),
                            object: self.ctx.terms.alloc(Term::Constant(std_np.noun)),
                            difference: None,
                            relation: crate::ast::logic::ComparisonRelation::Greater,
                        });
                        self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: event,
                            op: TokenType::And,
                            right: comparison,
                        })
                    };
                    return Ok(result);
                }
            }

            // Perception small clause ("saw her duck", "watched the bird fly"):
            // a perception verb takes "NP bare-VP" naming the PERCEIVED event.
            // Gated on an actual Verb token so a Noun-variant parse of the same
            // word yields the distinct NP-object reading instead.
            if crate::lexicon::is_perception_verb(&self.interner.resolve(verb).to_lowercase()) {
                let mut vp_idx = None;
                let mut i = self.current;
                while i < self.tokens.len()
                    && !matches!(
                        self.tokens[i].kind,
                        TokenType::Period | TokenType::EOF | TokenType::Comma
                    )
                {
                    // Mode-aware verb reading: under noun priority an
                    // Ambiguous token takes its noun reading, so the small
                    // clause does not fire and the NP-object parse runs.
                    let is_verb_reading = match &self.tokens[i].kind {
                        TokenType::Verb { .. } => true,
                        TokenType::Ambiguous { primary, .. } if !self.noun_priority_mode => {
                            matches!(**primary, TokenType::Verb { .. })
                        }
                        _ => false,
                    };
                    if is_verb_reading {
                        vp_idx = Some(i);
                    }
                    i += 1;
                }
                if let Some(vp_i) = vp_idx {
                    if vp_i > self.current {
                        let psubj = match self.tokens[vp_i - 1].kind.clone() {
                            TokenType::Noun(n) | TokenType::ProperName(n) => Some(n),
                            TokenType::Pronoun { .. } | TokenType::Ambiguous { .. } => {
                                let lx = self
                                    .interner
                                    .resolve(self.tokens[vp_i - 1].lexeme)
                                    .to_lowercase();
                                let cap = lx
                                    .chars()
                                    .next()
                                    .map(|c| c.to_uppercase().collect::<String>() + &lx[1..])
                                    .unwrap_or(lx);
                                Some(self.interner.intern(&cap))
                            }
                            _ => None,
                        };
                        if let Some(psubj) = psubj {
                            let inner_verb = match &self.tokens[vp_i].kind {
                                TokenType::Verb { lemma, .. } => *lemma,
                                TokenType::Ambiguous { primary, .. } => {
                                    if let TokenType::Verb { lemma, .. } = **primary {
                                        lemma
                                    } else {
                                        unreachable!("gated on verb reading")
                                    }
                                }
                                _ => unreachable!("gated on verb reading"),
                            };
                            self.current = vp_i + 1; // consume through the VP head
                            let perceived = self.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: inner_verb,
                                args: self.ctx.terms.alloc_slice([Term::Constant(psubj)]),
                                world: None,
                            });
                            let perceived_advs = self.collect_adverbs();
                            let perceived = if perceived_advs.is_empty() {
                                perceived
                            } else {
                                self.ctx.exprs.alloc(LogicExpr::Event {
                                    predicate: perceived,
                                    adverbs: self.ctx.syms.alloc_slice(perceived_advs),
                                })
                            };
                            let mut modifiers: Vec<Symbol> = Vec::new();
                            match verb_time {
                                Time::Past => modifiers.push(self.interner.intern("Past")),
                                Time::Future => modifiers.push(self.interner.intern("Future")),
                                _ => {}
                            }
                            let event_var = self.get_event_var();
                            let suppress_existential = self.drs.in_conditional_antecedent();
                            return Ok(self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(
                                NeoEventData {
                                    event_var,
                                    verb,
                                    roles: self.ctx.roles.alloc_slice(vec![
                                        (ThematicRole::Agent, subject_term.clone()),
                                        (ThematicRole::Theme, Term::Proposition(perceived)),
                                    ]),
                                    modifiers: self.ctx.syms.alloc_slice(modifiers),
                                    suppress_existential,
                                    world: None,
                                },
                            ))));
                        }
                    }
                }
            }

            // Check for embedded wh-clause: "I know who/what"
            if self.check_wh_word() {
                let wh_token = self.advance().kind.clone();

                let is_who = matches!(wh_token, TokenType::Who);
                let is_what = matches!(wh_token, TokenType::What);

                // Check for sluicing: wh-word followed by terminator
                let is_sluicing = self.is_at_end() ||
                    self.check(&TokenType::Period) ||
                    self.check(&TokenType::Comma);

                if is_sluicing {
                    if let Some(template) = self.last_event_template.clone() {
                        let wh_var = self.next_var_name();

                        let roles: Vec<_> = if is_who {
                            std::iter::once((ThematicRole::Agent, Term::Variable(wh_var)))
                                .chain(template.non_agent_roles.iter().cloned())
                                .collect()
                        } else if is_what {
                            vec![
                                (ThematicRole::Agent, subject_term.clone()),
                                (ThematicRole::Theme, Term::Variable(wh_var)),
                            ]
                        } else {
                            std::iter::once((ThematicRole::Agent, Term::Variable(wh_var)))
                                .chain(template.non_agent_roles.iter().cloned())
                                .collect()
                        };

                        let event_var = self.get_event_var();
                        let suppress_existential = self.drs.in_conditional_antecedent();
                        let reconstructed = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                            event_var,
                            verb: template.verb,
                            roles: self.ctx.roles.alloc_slice(roles),
                            modifiers: self.ctx.syms.alloc_slice(template.modifiers.clone()),
                            suppress_existential,
                            world: None,
                        })));

                        let question = self.ctx.exprs.alloc(LogicExpr::Question {
                            wh_variable: wh_var,
                            body: reconstructed,
                        });

                        let know_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                            event_var: self.get_event_var(),
                            verb,
                            roles: self.ctx.roles.alloc_slice(vec![
                                (ThematicRole::Agent, subject_term),
                                (ThematicRole::Theme, Term::Proposition(question)),
                            ]),
                            modifiers: self.ctx.syms.alloc_slice(vec![]),
                            suppress_existential,
                            world: None,
                        })));

                        return Ok(know_event);
                    }
                }

                // Non-sluicing: "I know who runs"
                let embedded = self.parse_embedded_wh_clause()?;
                let question = self.ctx.exprs.alloc(LogicExpr::Question {
                    wh_variable: self.interner.intern("x"),
                    body: embedded,
                });

                let suppress_existential = self.drs.in_conditional_antecedent();
                let know_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var: self.get_event_var(),
                    verb,
                    roles: self.ctx.roles.alloc_slice(vec![
                        (ThematicRole::Agent, subject_term),
                        (ThematicRole::Theme, Term::Proposition(question)),
                    ]),
                    modifiers: self.ctx.syms.alloc_slice(vec![]),
                    suppress_existential,
                    world: None,
                })));

                return Ok(know_event);
            }

            // Opaque attitude verbs take a finite clausal complement as a STRUCTURED
            // PROPOSITION (P3), not an extensional object: "John believes Mary left."
            // → Believe(John, ⟨Left(Mary)⟩). A pure-token lookahead detects an
            // embedded proper-name/pronoun subject directly followed by a verb
            // (optionally after the complementizer "that"); article-headed embedded
            // clauses ("a spy exists") are already handled downstream and untouched.
            if crate::lexicon::is_opaque_verb(&self.interner.resolve(verb).to_lowercase()) {
                let mut i = self.current;
                if i < self.tokens.len() && matches!(self.tokens[i].kind, TokenType::That) {
                    i += 1;
                }
                let subj_is_name_or_pronoun = i < self.tokens.len()
                    && matches!(
                        self.tokens[i].kind,
                        TokenType::ProperName(_) | TokenType::Pronoun { .. }
                    );
                let verb_follows = subj_is_name_or_pronoun
                    && i + 1 < self.tokens.len()
                    && matches!(
                        self.tokens[i + 1].kind,
                        TokenType::Verb { .. } | TokenType::Auxiliary(_)
                    );
                // Article-headed embedded subject with a finite clause:
                // "believes that THE TEACHER wants …". (Indefinite objects
                // without a following verb keep the de re/de dicto path.)
                let definite_np_clause = i + 2 < self.tokens.len()
                    && matches!(self.tokens[i].kind, TokenType::Article(_))
                    && matches!(self.tokens[i + 1].kind, TokenType::Noun(_))
                    && matches!(
                        self.tokens[i + 2].kind,
                        TokenType::Verb { .. } | TokenType::Auxiliary(_)
                    );
                if verb_follows || definite_np_clause {
                    if self.check(&TokenType::That) {
                        self.advance();
                    }
                    let embedded_subject = match self.peek().kind {
                        TokenType::ProperName(s) => {
                            self.advance();
                            s
                        }
                        TokenType::Pronoun { gender, number, .. } => {
                            self.advance();
                            match self.resolve_pronoun(gender, number)? {
                                super::ResolvedPronoun::Variable(s)
                                | super::ResolvedPronoun::Constant(s) => s,
                            }
                        }
                        TokenType::Article(_) => {
                            let np = self.parse_noun_phrase(false)?;
                            np.noun
                        }
                        _ => unreachable!("guarded by subj_is_name_or_pronoun"),
                    };
                    let embedded_pred = self.parse_predicate_with_subject(embedded_subject)?;
                    let embedded_term = Term::Proposition(embedded_pred);
                    let main_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: verb,
                        args: self
                            .ctx
                            .terms
                            .alloc_slice([subject_term.clone(), embedded_term]),
                        world: None,
                    });
                    let effective_time = self.pending_time.take().unwrap_or(verb_time);
                    return Ok(if effective_time == Time::Past {
                        self.ctx.exprs.alloc(LogicExpr::Temporal {
                            operator: TemporalOperator::Past,
                            body: main_pred,
                        })
                    } else {
                        main_pred
                    });
                }
            }

            let mut object_term: Option<Term<'a>> = None;
            let mut second_object_term: Option<Term<'a>> = None;
            // A filler-gap object licenses a stranded preposition ("Who did John talk to?").
            let mut gap_object = false;
            let mut object_pps: &[&LogicExpr<'a>] = &[];  // PPs attached to object NP (for NP-attachment mode)
            if self.check(&TokenType::Reflexive) {
                self.advance();
                // The reflexive binds the subject TERM, preserving its
                // variable-ness under a quantified subject ("Every man loves
                // himself." → Theme(e, x), not a constant).
                let term = subject_term.clone();
                object_term = Some(term.clone());
                args.push(term);
            } else if self.check_pronoun()
                && !(self.check_possessive_pronoun()
                    && match self.tokens.get(self.current + 1).map(|t| t.kind.clone()) {
                        Some(TokenType::Noun(_)) => true,
                        // Under noun priority an Ambiguous next token reads as
                        // a noun, so "her duck" is a possessive NP object.
                        Some(TokenType::Ambiguous { .. }) => self.noun_priority_mode,
                        _ => false,
                    })
            {
                let token = self.advance().clone();
                let (gender, number) = match &token.kind {
                    TokenType::Pronoun { gender, number, .. } => (*gender, *number),
                    TokenType::Ambiguous { primary, alternatives } => {
                        if let TokenType::Pronoun { gender, number, .. } = **primary {
                            (gender, number)
                        } else {
                            alternatives.iter().find_map(|t| {
                                if let TokenType::Pronoun { gender, number, .. } = t {
                                    Some((*gender, *number))
                                } else {
                                    None
                                }
                            }).unwrap_or((Gender::Unknown, Number::Singular))
                        }
                    }
                    _ => (Gender::Unknown, Number::Singular),
                };

                // Person deictics (§8.4) resolve to the discourse roles in any
                // position: object "you" → Addressee, "me" → Speaker.
                let plex = self.interner.resolve(token.lexeme).to_lowercase();
                let term = match plex.as_str() {
                    "you" | "yourself" => Term::Constant(self.interner.intern("Addressee")),
                    "me" | "myself" | "i" => Term::Constant(self.interner.intern("Speaker")),
                    // A donkey antecedent (indefinite from a quantifier's
                    // restriction) outranks discourse resolution: "Every man
                    // who owns a book reads it." → Theme(e, y).
                    _ => match self.resolve_donkey_pronoun(gender) {
                        Some(donkey_var) => Term::Variable(donkey_var),
                        None => match self.resolve_pronoun(gender, number)? {
                            super::ResolvedPronoun::Variable(s) => Term::Variable(s),
                            super::ResolvedPronoun::Constant(s) => Term::Constant(s),
                        },
                    },
                };
                object_term = Some(term);
                args.push(term);

                let verb_str = self.interner.resolve(verb);
                if Lexer::is_ditransitive_verb(verb_str)
                    && (self.check_content_word() || self.check_article())
                {
                    let second_np = self.parse_noun_phrase(false)?;
                    let second_term = Term::Constant(second_np.noun);
                    second_object_term = Some(second_term);
                    args.push(second_term);
                }
            } else if self.check_quantifier() || self.check_article() || self.check_possessive_pronoun() {
                let obj_quantifier = if self.check_possessive_pronoun() {
                    // Possessive NP object ("his dog"): parse_noun_phrase
                    // consumes the possessor; no quantifier wrapper.
                    None
                } else if self.check_quantifier() {
                    Some(self.advance().kind.clone())
                } else {
                    let art = self.advance().kind.clone();
                    if let TokenType::Article(def) = art {
                        if def == Definiteness::Indefinite {
                            Some(TokenType::Some)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                let object_np = self.parse_noun_phrase(false)?;

                if let Some(obj_q) = obj_quantifier {
                    let obj_var = self.next_var_name();

                    // Introduce object referent in DRS for cross-sentence anaphora
                    let obj_gender = Self::infer_noun_gender(self.interner.resolve(object_np.noun));
                    let obj_number = if Self::is_plural_noun(self.interner.resolve(object_np.noun)) {
                        Number::Plural
                    } else {
                        Number::Singular
                    };
                    // Definite descriptions presuppose existence, so they should be globally accessible
                    if object_np.definiteness == Some(Definiteness::Definite) {
                        self.drs.introduce_referent_with_source(obj_var, object_np.noun, obj_gender, obj_number, ReferentSource::MainClause);
                    } else {
                        self.drs.introduce_referent(obj_var, object_np.noun, obj_gender, obj_number);
                    }

                    let obj_restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: object_np.noun,
                        args: self.ctx.terms.alloc_slice([Term::Variable(obj_var)]),
                        world: None,
                    });

                    // Continuations inside the object quantifier's scope: a
                    // second object ("gave some student a book"), a recipient
                    // ("gave a book to Mary", "… to some teacher"), or an
                    // object-control infinitive ("caused all flowers to bloom").
                    let verb_str = self.interner.resolve(verb).to_string();
                    let mut second_object: Option<Term<'a>> = None;
                    let mut recipient: Option<Term<'a>> = None;
                    let mut recipient_quant: Option<(TokenType, Symbol, Symbol)> = None;
                    let mut control_infinitive: Option<Symbol> = None;

                    if Lexer::is_ditransitive_verb(&verb_str)
                        && (self.check_content_word() || self.check_article())
                    {
                        let second_np = self.parse_noun_phrase(false)?;
                        second_object = Some(Term::Constant(second_np.noun));
                    } else if self.check_to_marker() {
                        let after_to = self.tokens.get(self.current + 1).map(|t| t.kind.clone());
                        match after_to {
                            Some(TokenType::Verb { lemma, .. }) => {
                                self.advance(); // to
                                self.advance(); // infinitive verb
                                control_infinitive = Some(lemma);
                            }
                            // After a preposition the lexer may classify the
                            // infinitive as a noun ("to bloom"); the lexicon
                            // recovers the verb reading.
                            Some(TokenType::Noun(word))
                                if crate::lexicon::lookup_verb_db(
                                    &self.interner.resolve(word).to_lowercase(),
                                )
                                .is_some() =>
                            {
                                let lemma_str = crate::lexicon::lookup_verb_db(
                                    &self.interner.resolve(word).to_lowercase(),
                                )
                                .map(|m| m.lemma)
                                .unwrap();
                                self.advance(); // to
                                self.advance(); // infinitive verb (noun-classified)
                                control_infinitive = Some(self.interner.intern(lemma_str));
                            }
                            Some(kind)
                                if Lexer::is_ditransitive_verb(&verb_str)
                                    && matches!(
                                        kind,
                                        TokenType::All
                                            | TokenType::Some
                                            | TokenType::No
                                            | TokenType::Most
                                            | TokenType::Few
                                            | TokenType::Many
                                            | TokenType::Cardinal(_)
                                            | TokenType::AtLeast(_)
                                            | TokenType::AtMost(_)
                                    ) =>
                            {
                                self.advance(); // to
                                let r_quant = self.advance().kind.clone();
                                let r_np = self.parse_noun_phrase(false)?;
                                let r_var = self.next_var_name();
                                recipient_quant = Some((r_quant, r_var, r_np.noun));
                            }
                            Some(kind)
                                if Lexer::is_ditransitive_verb(&verb_str)
                                    && matches!(
                                        kind,
                                        TokenType::ProperName(_)
                                            | TokenType::Noun(_)
                                            | TokenType::Article(_)
                                    ) =>
                            {
                                self.advance(); // to
                                let r_np = self.parse_noun_phrase(false)?;
                                recipient = Some(Term::Constant(r_np.noun));
                            }
                            _ => {}
                        }
                    }

                    let event_var = self.get_event_var();
                    let mut modifiers = self.collect_adverbs();
                    let effective_time = self.pending_time.take().unwrap_or(verb_time);
                    match effective_time {
                        Time::Past => modifiers.push(self.interner.intern("Past")),
                        Time::Future => modifiers.push(self.interner.intern("Future")),
                        _ => {}
                    }

                    let mut roles = vec![(ThematicRole::Agent, subject_term.clone())];
                    if let Some(second) = second_object {
                        roles.push((ThematicRole::Recipient, Term::Variable(obj_var)));
                        roles.push((ThematicRole::Theme, second));
                    } else {
                        roles.push((ThematicRole::Theme, Term::Variable(obj_var)));
                        if let Some(r) = recipient {
                            roles.push((ThematicRole::Recipient, r));
                        } else if let Some((_, r_var, _)) = recipient_quant {
                            roles.push((ThematicRole::Recipient, Term::Variable(r_var)));
                        }
                    }

                    let suppress_existential = self.drs.in_conditional_antecedent();
                    let neo_event = if let Some(inf) = control_infinitive {
                        let inf_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: inf,
                            args: self.ctx.terms.alloc_slice([Term::Variable(obj_var)]),
                            world: None,
                        });
                        let control = self.ctx.exprs.alloc(LogicExpr::Control {
                            verb,
                            subject: self.ctx.terms.alloc(subject_term.clone()),
                            object: Some(&*self.ctx.terms.alloc(Term::Variable(obj_var))),
                            infinitive: inf_pred,
                        });
                        match effective_time {
                            Time::Past => &*self.ctx.exprs.alloc(LogicExpr::Temporal {
                                operator: TemporalOperator::Past,
                                body: control,
                            }),
                            Time::Future => &*self.ctx.exprs.alloc(LogicExpr::Temporal {
                                operator: TemporalOperator::Future,
                                body: control,
                            }),
                            _ => control,
                        }
                    } else {
                        let plain = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                            event_var,
                            verb,
                            roles: self.ctx.roles.alloc_slice(roles),
                            modifiers: self.ctx.syms.alloc_slice(modifiers),
                            suppress_existential,
                            world: None,
                        })));
                        if let Some((r_quant, r_var, r_noun)) = recipient_quant {
                            let r_restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: r_noun,
                                args: self.ctx.terms.alloc_slice([Term::Variable(r_var)]),
                                world: None,
                            });
                            let r_kind = match r_quant {
                                TokenType::All => QuantifierKind::Universal,
                                TokenType::Most => QuantifierKind::Most,
                                TokenType::Few => QuantifierKind::Few,
                                TokenType::Many => QuantifierKind::Many,
                                TokenType::Cardinal(n) => QuantifierKind::Cardinal(n),
                                TokenType::AtLeast(n) => QuantifierKind::AtLeast(n),
                                TokenType::AtMost(n) => QuantifierKind::AtMost(n),
                                _ => QuantifierKind::Existential,
                            };
                            let r_body = if matches!(r_kind, QuantifierKind::Universal) {
                                self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                    left: r_restriction,
                                    op: TokenType::Implies,
                                    right: plain,
                                })
                            } else {
                                self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                    left: r_restriction,
                                    op: TokenType::And,
                                    right: plain,
                                })
                            };
                            &*self.ctx.exprs.alloc(LogicExpr::Quantifier {
                                kind: r_kind,
                                variable: r_var,
                                body: r_body,
                                island_id: self.current_island,
                            })
                        } else {
                            plain
                        }
                    };

                    let obj_kind = match obj_q {
                        TokenType::All => QuantifierKind::Universal,
                        TokenType::Some => QuantifierKind::Existential,
                        TokenType::No => QuantifierKind::Universal,
                        TokenType::Most => QuantifierKind::Most,
                        TokenType::Few => QuantifierKind::Few,
                        TokenType::Many => QuantifierKind::Many,
                        TokenType::Cardinal(n) => QuantifierKind::Cardinal(n),
                        TokenType::AtLeast(n) => QuantifierKind::AtLeast(n),
                        TokenType::AtMost(n) => QuantifierKind::AtMost(n),
                        _ => QuantifierKind::Existential,
                    };

                    let obj_body = match obj_q {
                        TokenType::All => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: obj_restriction,
                            op: TokenType::Implies,
                            right: neo_event,
                        }),
                        TokenType::No => {
                            let neg = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                                op: TokenType::Not,
                                operand: neo_event,
                            });
                            self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                left: obj_restriction,
                                op: TokenType::Implies,
                                right: neg,
                            })
                        }
                        _ => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: obj_restriction,
                            op: TokenType::And,
                            right: neo_event,
                        }),
                    };

                    return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                        kind: obj_kind,
                        variable: obj_var,
                        body: obj_body,
                        island_id: self.current_island,
                    }));
                } else {
                    // Definite object NP (e.g., "the house")
                    // Introduce to DRS for cross-sentence bridging anaphora
                    // E.g., "John entered the house. The door was open." - door bridges to house
                    if object_np.definiteness == Some(Definiteness::Definite) {
                        let obj_gender = Self::infer_noun_gender(self.interner.resolve(object_np.noun));
                        let obj_number = if Self::is_plural_noun(self.interner.resolve(object_np.noun)) {
                            Number::Plural
                        } else {
                            Number::Singular
                        };
                        // Definite descriptions presuppose existence, so they should be globally accessible
                        self.drs.introduce_referent_with_source(object_np.noun, object_np.noun, obj_gender, obj_number, ReferentSource::MainClause);
                    }

                    let term = Term::Constant(object_np.noun);
                    object_term = Some(term);
                    // Store PPs attached to object NP for NP-attachment mode
                    object_pps = object_np.pps;
                    args.push(term);
                }
            } else if self.check_focus() {
                let focus_kind = if let TokenType::Focus(k) = self.advance().kind {
                    k
                } else {
                    FocusKind::Only
                };

                let event_var = self.get_event_var();
                let mut modifiers = self.collect_adverbs();
                let effective_time = self.pending_time.take().unwrap_or(verb_time);
                match effective_time {
                    Time::Past => modifiers.push(self.interner.intern("Past")),
                    Time::Future => modifiers.push(self.interner.intern("Future")),
                    _ => {}
                }

                if self.check_preposition() {
                    let prep_token = self.advance().clone();
                    let prep_name = if let TokenType::Preposition(sym) = prep_token.kind {
                        sym
                    } else {
                        self.interner.intern("to")
                    };
                    let pp_obj = self.parse_noun_phrase(false)?;
                    let pp_obj_term = Term::Constant(pp_obj.noun);

                    let roles = vec![(ThematicRole::Agent, subject_term)];
                    let suppress_existential = self.drs.in_conditional_antecedent();
                    let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                        event_var,
                        verb,
                        roles: self.ctx.roles.alloc_slice(roles),
                        modifiers: self.ctx.syms.alloc_slice(modifiers),
                        suppress_existential,
                        world: None,
                    })));

                    let pp_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: prep_name,
                        args: self.ctx.terms.alloc_slice([Term::Variable(event_var), pp_obj_term]),
                        world: None,
                    });

                    let with_pp = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: neo_event,
                        op: TokenType::And,
                        right: pp_pred,
                    });

                    let focused_ref = self.ctx.terms.alloc(pp_obj_term);
                    return Ok(self.ctx.exprs.alloc(LogicExpr::Focus {
                        kind: focus_kind,
                        focused: focused_ref,
                        scope: with_pp,
                    }));
                }

                let focused_np = self.parse_noun_phrase(false)?;
                let focused_term = Term::Constant(focused_np.noun);
                args.push(focused_term);

                let roles = vec![
                    (ThematicRole::Agent, subject_term),
                    (ThematicRole::Theme, focused_term),
                ];

                let suppress_existential = self.drs.in_conditional_antecedent();
                let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var,
                    verb,
                    roles: self.ctx.roles.alloc_slice(roles),
                    modifiers: self.ctx.syms.alloc_slice(modifiers),
                    suppress_existential,
                    world: None,
                })));

                let focused_ref = self.ctx.terms.alloc(focused_term);
                return Ok(self.ctx.exprs.alloc(LogicExpr::Focus {
                    kind: focus_kind,
                    focused: focused_ref,
                    scope: neo_event,
                }));
            } else if self.check_number() {
                let measure = self.parse_measure_phrase()?;
                if self.check_content_word() {
                    let noun_sym = self.consume_content_word()?;
                    args.push(*measure);
                    args.push(Term::Constant(noun_sym));
                } else {
                    args.push(*measure);
                }
            } else if self.check_content_word() {
                let potential_object = self.parse_noun_phrase(false)?;
                // Store PPs attached to object NP for NP-attachment mode
                object_pps = potential_object.pps;

                // A finite clausal complement (the NP is followed by a verb) is taken
                // as a structured proposition (P3) when the matrix verb is an opaque
                // attitude verb ("John believes Mary left." → Believe(John, ⟨Left(Mary)⟩))
                // or in a filler-gap context. The complement keeps its own structure
                // so co-intensional complements stay distinct and substitution into it
                // is blocked.
                let verb_is_opaque =
                    crate::lexicon::is_opaque_verb(&self.interner.resolve(verb).to_lowercase());
                if self.check_verb() && (self.filler_gap.is_some() || verb_is_opaque) {
                    let embedded_subject = potential_object.noun;
                    let embedded_pred = self.parse_predicate_with_subject(embedded_subject)?;

                    let embedded_term = Term::Proposition(embedded_pred);
                    let main_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: verb,
                        args: self.ctx.terms.alloc_slice([subject_term, embedded_term]),
                        world: None,
                    });

                    let effective_time = self.pending_time.take().unwrap_or(verb_time);
                    return Ok(if effective_time == Time::Past {
                        self.ctx.exprs.alloc(LogicExpr::Temporal {
                            operator: TemporalOperator::Past,
                            body: main_pred,
                        })
                    } else {
                        main_pred
                    });
                }

                // Collect all objects for potential "respectively" handling
                let mut all_objects: Vec<Symbol> = vec![potential_object.noun];

                // Check for coordinated objects: "Tom and Jerry and Bob"
                while self.check(&TokenType::And) {
                    let saved = self.current;
                    self.advance(); // consume "and"
                    if self.check_content_word() || self.check_article() {
                        let next_obj = match self.parse_noun_phrase(false) {
                            Ok(np) => np,
                            Err(_) => {
                                self.current = saved;
                                break;
                            }
                        };
                        all_objects.push(next_obj.noun);
                    } else {
                        self.current = saved;
                        break;
                    }
                }

                // Check for "respectively" with single subject
                if self.check(&TokenType::Respectively) {
                    let respectively_span = self.peek().span;
                    // Single subject with multiple objects + respectively = error
                    if all_objects.len() > 1 {
                        return Err(ParseError {
                            kind: ParseErrorKind::RespectivelyLengthMismatch {
                                subject_count: 1,
                                object_count: all_objects.len(),
                            },
                            span: respectively_span,
                        });
                    }
                    // Single subject, single object + respectively is valid (trivially pairwise)
                    self.advance(); // consume "respectively"
                }

                // Use the first object (or only object) for normal processing
                let term = Term::Constant(all_objects[0]);
                object_term = Some(term);
                args.push(term);

                // For multiple objects without "respectively", use group semantics
                if all_objects.len() > 1 {
                    let obj_members: Vec<Term<'a>> = all_objects.iter()
                        .map(|o| Term::Constant(*o))
                        .collect();
                    let obj_group = Term::Group(self.ctx.terms.alloc_slice(obj_members));
                    // Replace the single object with the group
                    args.pop();
                    args.push(obj_group);
                }

                let verb_str = self.interner.resolve(verb);
                if Lexer::is_ditransitive_verb(verb_str)
                    && (self.check_content_word() || self.check_article())
                {
                    let second_np = self.parse_noun_phrase(false)?;
                    let second_term = Term::Constant(second_np.noun);
                    second_object_term = Some(second_term);
                    args.push(second_term);
                }
            } else if self.filler_gap.is_some() && !self.check_content_word() && !self.check_pronoun()
            {
                let gap_var = self.filler_gap.take().unwrap();
                let term = Term::Variable(gap_var);
                object_term = Some(term);
                args.push(term);
                gap_object = true;
            }

            // Check for distanced phrasal verb particle: "gave the book up"
            if let TokenType::Particle(particle_sym) = self.peek().kind {
                let verb_str = self.interner.resolve(verb).to_lowercase();
                let particle_str = self.interner.resolve(particle_sym).to_lowercase();
                if let Some((phrasal_lemma, _class)) = crate::lexicon::lookup_phrasal_verb(&verb_str, &particle_str) {
                    self.advance(); // consume the particle
                    verb = self.interner.intern(phrasal_lemma);
                }
            }

            let unknown = self.interner.intern("?");
            let mut pp_predicates: Vec<&'a LogicExpr<'a>> = Vec::new();
            while self.check_preposition() || self.check_to() {
                // "within N cycles" is a temporal bound, not a PP — leave for try_wrap_bounded_delay
                if self.check_preposition_is("within") && self.current + 1 < self.tokens.len()
                    && matches!(self.tokens[self.current + 1].kind, TokenType::Cardinal(_) | TokenType::Number(_))
                {
                    break;
                }
                let prep_token = self.advance().clone();
                let prep_name = if let TokenType::Preposition(sym) = prep_token.kind {
                    sym
                } else if matches!(prep_token.kind, TokenType::To) {
                    self.interner.intern("To")
                } else {
                    continue;
                };

                let pp_obj_term = if self.check(&TokenType::Reflexive) {
                    self.advance();
                    Term::Constant(subject_symbol)
                } else if self.check_pronoun() {
                    let token = self.advance().clone();
                    let (gender, number) = match &token.kind {
                        TokenType::Pronoun { gender, number, .. } => (*gender, *number),
                        TokenType::Ambiguous { primary, alternatives } => {
                            if let TokenType::Pronoun { gender, number, .. } = **primary {
                                (gender, number)
                            } else {
                                alternatives.iter().find_map(|t| {
                                    if let TokenType::Pronoun { gender, number, .. } = t {
                                        Some((*gender, *number))
                                    } else {
                                        None
                                    }
                                }).unwrap_or((Gender::Unknown, Number::Singular))
                            }
                        }
                        _ => (Gender::Unknown, Number::Singular),
                    };
                    let resolved = self.resolve_pronoun(gender, number)?;
                    match resolved {
                        super::ResolvedPronoun::Variable(s) => Term::Variable(s),
                        super::ResolvedPronoun::Constant(s) => Term::Constant(s),
                    }
                } else if self.check_content_word() || self.check_article() {
                    let prep_obj = self.parse_noun_phrase(false)?;
                    Term::Constant(prep_obj.noun)
                } else if gap_object {
                    // Preposition stranding: the object position was a wh-gap,
                    // so the bare preposition is licensed ("Who did John talk to?").
                    continue;
                } else if self.at_clause_boundary()
                    && crate::lexicon::is_particle(
                        &self.interner.resolve(prep_name).to_lowercase(),
                    )
                {
                    // A clause-final object-less PARTICLE preposition is an
                    // intransitive directional ("walked in", "sat down") — a
                    // lexically listed class; "of"/"to" cannot end a clause.
                    let event_sym = self.get_event_var();
                    pp_predicates.push(self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: prep_name,
                        args: self.ctx.terms.alloc_slice([Term::Variable(event_sym)]),
                        world: None,
                    }));
                    continue;
                } else {
                    // A mid-clause preposition with no object is not a PP —
                    // hand it back so the sentence-level parse reports it
                    // instead of silently dropping it.
                    self.current -= 1;
                    break;
                };

                if self.pp_attach_to_noun {
                    if let Some(obj) = object_term {
                        let pp_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: prep_name,
                            args: self.ctx.terms.alloc_slice([obj, pp_obj_term]),
                            world: None,
                        });
                        pp_predicates.push(pp_pred);
                    } else {
                        args.push(pp_obj_term);
                    }
                } else {
                    let event_sym = self.get_event_var();
                    let pp_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: prep_name,
                        args: self
                            .ctx
                            .terms
                            .alloc_slice([Term::Variable(event_sym), pp_obj_term]),
                        world: None,
                    });
                    pp_predicates.push(pp_pred);
                }
            }

            if self.check(&TokenType::That) || self.check(&TokenType::Who) {
                self.advance();
                let rel_var = self.next_var_name();
                let rel_pred = self.parse_relative_clause(rel_var)?;
                pp_predicates.push(rel_pred);
            }

            let mut modifiers = self.collect_adverbs();

            let effective_time = self.pending_time.take().unwrap_or(verb_time);
            match effective_time {
                Time::Past => modifiers.push(self.interner.intern("Past")),
                Time::Future => modifiers.push(self.interner.intern("Future")),
                _ => {}
            }

            if verb_aspect == Aspect::Progressive {
                modifiers.push(self.interner.intern("Progressive"));
            } else if verb_aspect == Aspect::Perfect {
                modifiers.push(self.interner.intern("Perfect"));
            }

            let mut roles: Vec<(ThematicRole, Term<'a>)> = Vec::new();

            // Check if verb is unaccusative (intransitive subject is Theme, not Agent)
            let verb_str = self.interner.resolve(verb).to_lowercase();
            let is_unaccusative = crate::lexicon::lookup_verb_db(&verb_str)
                .map(|meta| meta.features.contains(&crate::lexicon::Feature::Unaccusative))
                .unwrap_or(false);

            // Unaccusative verbs used intransitively: subject is Theme
            // E.g., "The alarm triggers" → Theme(e, Alarm), not Agent(e, Alarm)
            let has_object = object_term.is_some() || second_object_term.is_some();
            let subject_role = if is_unaccusative && !has_object {
                ThematicRole::Theme
            } else {
                ThematicRole::Agent
            };

            roles.push((subject_role, subject_term));
            if let Some(second_obj) = second_object_term {
                if let Some(first_obj) = object_term {
                    roles.push((ThematicRole::Recipient, first_obj));
                }
                roles.push((ThematicRole::Theme, second_obj));
            } else if let Some(obj) = object_term {
                roles.push((ThematicRole::Theme, obj));
            }

            let event_var = self.get_event_var();
            let suppress_existential = self.drs.in_conditional_antecedent();
            if suppress_existential {
                let event_class = self.interner.intern("Event");
                self.drs.introduce_referent(event_var, event_class, Gender::Neuter, Number::Singular);
            }
            let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                event_var,
                verb,
                roles: self.ctx.roles.alloc_slice(roles.clone()),
                modifiers: self.ctx.syms.alloc_slice(modifiers.clone()),
                suppress_existential,
                world: None,
            })));

            // Capture template for ellipsis reconstruction
            self.capture_event_template(verb, &roles, &modifiers);

            let with_pps = if pp_predicates.is_empty() {
                neo_event
            } else {
                let mut combined = neo_event;
                for pp in pp_predicates {
                    combined = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: combined,
                        op: TokenType::And,
                        right: pp,
                    });
                }
                combined
            };

            // Include PPs attached to object NP (for NP-attachment mode)
            // These have _PP_SELF_ placeholder that needs to be replaced with the object term
            let with_object_pps = if object_pps.is_empty() {
                with_pps
            } else if let Some(obj_term) = object_term {
                let placeholder = self.interner.intern("_PP_SELF_");
                let mut combined = with_pps;
                for pp in object_pps {
                    // Substitute _PP_SELF_ placeholder with the object term
                    let substituted = match pp {
                        LogicExpr::Predicate { name, args, .. } => {
                            let new_args: Vec<Term<'a>> = args
                                .iter()
                                .map(|arg| match arg {
                                    Term::Variable(v) if *v == placeholder => obj_term,
                                    other => *other,
                                })
                                .collect();
                            self.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: *name,
                                args: self.ctx.terms.alloc_slice(new_args),
                                world: None,
                            })
                        }
                        _ => *pp,
                    };
                    combined = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: combined,
                        op: TokenType::And,
                        right: substituted,
                    });
                }
                combined
            } else {
                with_pps
            };

            // Apply aspectual operators based on verb class
            let with_aspect = if verb_aspect == Aspect::Simple && effective_time == Time::Present {
                // Non-state verbs in simple present get Habitual reading
                if !verb_class.is_stative() {
                    self.ctx.exprs.alloc(LogicExpr::Aspectual {
                        operator: AspectOperator::Habitual,
                        body: with_object_pps,
                    })
                } else {
                    with_object_pps
                }
            } else if verb_aspect == Aspect::Progressive {
                // Semelfactive + Progressive → Iterative
                if verb_class == crate::lexicon::VerbClass::Semelfactive {
                    self.ctx.exprs.alloc(LogicExpr::Aspectual {
                        operator: AspectOperator::Iterative,
                        body: with_object_pps,
                    })
                } else {
                    with_object_pps
                }
            } else {
                with_object_pps
            };

            Ok(with_aspect)
        } else {
            Ok(self.ctx.exprs.alloc(LogicExpr::Atom(subject_symbol)))
        }
    }
}

impl<'a, 'ctx, 'int> LogicVerbParsing<'a, 'ctx, 'int> for Parser<'a, 'ctx, 'int> {
    fn parse_predicate_with_subject(&mut self, subject_symbol: Symbol) -> ParseResult<&'a LogicExpr<'a>> {
        let result = self.parse_predicate_impl(subject_symbol, false)?;
        Ok(self.try_wrap_bounded_delay(result))
    }

    fn parse_predicate_with_subject_as_var(&mut self, subject_symbol: Symbol) -> ParseResult<&'a LogicExpr<'a>> {
        let result = self.parse_predicate_impl(subject_symbol, true)?;
        Ok(self.try_wrap_bounded_delay(result))
    }

    fn try_parse_plural_subject(
        &mut self,
        first_subject: &NounPhrase<'a>,
    ) -> Result<Option<&'a LogicExpr<'a>>, ParseError> {
        let saved_pos = self.current;

        // Consume the 'and' we already peeked
        self.advance();

        if !self.check_content_word() {
            self.current = saved_pos;
            return Ok(None);
        }

        // Collect all subjects: "John and Mary and Sue"
        let mut subjects: Vec<Symbol> = vec![first_subject.noun];

        loop {
            if !self.check_content_word() {
                break;
            }
            let next_subject = match self.parse_noun_phrase(true) {
                Ok(np) => np,
                Err(_) => {
                    self.current = saved_pos;
                    return Ok(None);
                }
            };
            subjects.push(next_subject.noun);

            if self.check(&TokenType::And) {
                self.advance();
            } else {
                break;
            }
        }

        // Check for copula (is/are/was/were) with predicate nominative
        // "Both Socrates and Plato are men" -> M(s) ∧ M(p)
        if self.check(&TokenType::Is) || self.check(&TokenType::Are)
            || self.check(&TokenType::Was) || self.check(&TokenType::Were)
        {
            let copula_time = if self.check(&TokenType::Was) || self.check(&TokenType::Were) {
                Time::Past
            } else {
                Time::Present
            };
            self.advance(); // consume the copula

            // Check for negation: "are not valid", "are not both valid"
            let is_negated = self.check(&TokenType::Not);
            if is_negated {
                self.advance(); // consume "not"
            }

            // Check for "both" modifier: "are not both valid"
            // "both" scopes negation over the conjunction: ¬(P(A) ∧ P(B))
            // Without "both": negation distributes: ¬P(A) ∧ ¬P(B)
            let has_both = self.check(&TokenType::Both);
            if has_both {
                self.advance(); // consume "both"
            }

            // Parse the predicate (e.g., "men" in "are men", "valid" in "are valid")
            if !self.check_content_word() && !self.check_article() {
                self.current = saved_pos;
                return Ok(None);
            }

            let predicate_np = match self.parse_noun_phrase(false) {
                Ok(np) => np,
                Err(_) => {
                    self.current = saved_pos;
                    return Ok(None);
                }
            };
            let predicate = predicate_np.noun;

            // Build distributed predicate: P(s1) ∧ P(s2) ∧ ...
            let mut conjuncts: Vec<&'a LogicExpr<'a>> = Vec::new();
            for subj in &subjects {
                let pred_expr = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: predicate,
                    args: self.ctx.terms.alloc_slice([Term::Constant(*subj)]),
                    world: None,
                });
                conjuncts.push(pred_expr);
            }

            if is_negated && !has_both {
                // "are not valid" → ¬P(s1) ∧ ¬P(s2) (negation distributes)
                for conjunct in &mut conjuncts {
                    *conjunct = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand: *conjunct,
                    });
                }
            }

            // Fold conjuncts into binary conjunction tree
            let mut result = conjuncts[0];
            for conjunct in &conjuncts[1..] {
                result = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: result,
                    op: TokenType::And,
                    right: *conjunct,
                });
            }

            // "are not both valid" → ¬(P(s1) ∧ P(s2)) (negation over conjunction)
            if is_negated && has_both {
                result = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                    op: TokenType::Not,
                    operand: result,
                });
            }

            // Apply temporal modifier for past tense
            let with_time = match copula_time {
                Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: TemporalOperator::Past,
                    body: result,
                }),
                _ => result,
            };

            return Ok(Some(with_time));
        }

        if !self.check_verb() {
            self.current = saved_pos;
            return Ok(None);
        }

        // Coordinated subjects registered in DRS via introduce_referent

        let (verb, verb_time, _verb_aspect, _) = self.consume_verb_with_metadata();

        // Check for reciprocal: "John and Mary kicked each other"
        if self.check(&TokenType::Reciprocal) {
            self.advance();
            if subjects.len() != 2 {
                self.current = saved_pos;
                return Ok(None);
            }
            let pred1 = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice([
                    Term::Constant(subjects[0]),
                    Term::Constant(subjects[1]),
                ]),
                world: None,
            });
            let pred2 = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice([
                    Term::Constant(subjects[1]),
                    Term::Constant(subjects[0]),
                ]),
                world: None,
            });
            let expr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: pred1,
                op: TokenType::And,
                right: pred2,
            });

            let with_time = match verb_time {
                Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: TemporalOperator::Past,
                    body: expr,
                }),
                Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: TemporalOperator::Future,
                    body: expr,
                }),
                _ => expr,
            };
            return Ok(Some(with_time));
        }

        // Check for objects (for transitive verbs with "respectively")
        let mut objects: Vec<Symbol> = Vec::new();
        if self.check_content_word() || self.check_article() {
            // Parse first object
            let first_obj = match self.parse_noun_phrase(false) {
                Ok(np) => np,
                Err(_) => {
                    // No objects, continue with intransitive
                    return Ok(Some(self.build_group_predicate(&subjects, verb, verb_time)));
                }
            };
            objects.push(first_obj.noun);

            // Parse additional objects: "Tom and Jerry and Bob"
            while self.check(&TokenType::And) {
                self.advance();
                if self.check_content_word() || self.check_article() {
                    let next_obj = match self.parse_noun_phrase(false) {
                        Ok(np) => np,
                        Err(_) => break,
                    };
                    objects.push(next_obj.noun);
                } else {
                    break;
                }
            }
        }

        // Check for "respectively" - triggers pairwise interpretation
        // Ditransitive pairing ("gave books TO TOM AND JERRY respectively"):
        // the recipients, not the shared theme, line up with the subjects.
        let mut recipients: Vec<Symbol> = Vec::new();
        let respectively_ahead = {
            let mut i = self.current;
            let mut found = false;
            while i < self.tokens.len()
                && !matches!(self.tokens[i].kind, TokenType::Period | TokenType::EOF)
            {
                if matches!(self.tokens[i].kind, TokenType::Respectively) {
                    found = true;
                    break;
                }
                i += 1;
            }
            found
        };
        if respectively_ahead && self.check_to_marker() {
            self.advance(); // to
            loop {
                let r_np = self.parse_noun_phrase(false)?;
                recipients.push(r_np.noun);
                if self.check(&TokenType::And) {
                    self.advance();
                } else {
                    break;
                }
            }
        }

        if self.check(&TokenType::Respectively) {
            let respectively_span = self.peek().span;
            self.advance(); // consume "respectively"

            let pair_targets: &[Symbol] = if recipients.is_empty() {
                &objects
            } else {
                &recipients
            };
            if subjects.len() != pair_targets.len() {
                return Err(ParseError {
                    kind: ParseErrorKind::RespectivelyLengthMismatch {
                        subject_count: subjects.len(),
                        object_count: pair_targets.len(),
                    },
                    span: respectively_span,
                });
            }

            // Build pairwise predicates: See(J,T) ∧ See(M,J) ∧ ...; with
            // recipients, the theme is shared: Give(J,Books,T) ∧ Give(M,Books,J).
            let mut conjuncts: Vec<&'a LogicExpr<'a>> = Vec::new();
            let suppress_existential = self.drs.in_conditional_antecedent();
            for (subj, target) in subjects.iter().zip(pair_targets.iter()) {
                let event_var = self.get_event_var();
                let roles = if recipients.is_empty() {
                    vec![
                        (ThematicRole::Agent, Term::Constant(*subj)),
                        (ThematicRole::Theme, Term::Constant(*target)),
                    ]
                } else {
                    let mut r = vec![(ThematicRole::Agent, Term::Constant(*subj))];
                    if let Some(theme) = objects.first() {
                        r.push((ThematicRole::Theme, Term::Constant(*theme)));
                    }
                    r.push((ThematicRole::Recipient, Term::Constant(*target)));
                    r
                };
                let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var,
                    verb,
                    roles: self.ctx.roles.alloc_slice(roles),
                    modifiers: self.ctx.syms.alloc_slice(vec![]),
                    suppress_existential,
                    world: None,
                })));
                conjuncts.push(neo_event);
            }

            // Fold conjuncts into binary conjunction tree
            let mut result = conjuncts[0];
            for conjunct in &conjuncts[1..] {
                result = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: result,
                    op: TokenType::And,
                    right: *conjunct,
                });
            }

            // Apply temporal modifier
            let with_time = match verb_time {
                Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: TemporalOperator::Past,
                    body: result,
                }),
                Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: TemporalOperator::Future,
                    body: result,
                }),
                _ => result,
            };

            return Ok(Some(with_time));
        }

        // No "respectively" - use group semantics
        if objects.is_empty() {
            // Intransitive: group subject
            Ok(Some(self.build_group_predicate(&subjects, verb, verb_time)))
        } else {
            // Transitive without "respectively": group subject, group object
            Ok(Some(self.build_group_transitive(&subjects, &objects, verb, verb_time)))
        }
    }

    /// Build a group predicate for intransitive verbs
    fn build_group_predicate(
        &mut self,
        subjects: &[Symbol],
        verb: Symbol,
        verb_time: Time,
    ) -> &'a LogicExpr<'a> {
        let group_members: Vec<Term<'a>> = subjects.iter()
            .map(|s| Term::Constant(*s))
            .collect();
        let group_members_slice = self.ctx.terms.alloc_slice(group_members);

        let expr = self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: verb,
            args: self.ctx.terms.alloc_slice([Term::Group(group_members_slice)]),
            world: None,
        });

        match verb_time {
            Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: TemporalOperator::Past,
                body: expr,
            }),
            Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: TemporalOperator::Future,
                body: expr,
            }),
            _ => expr,
        }
    }

    /// Build a transitive predicate with group subject and group object
    fn build_group_transitive(
        &mut self,
        subjects: &[Symbol],
        objects: &[Symbol],
        verb: Symbol,
        verb_time: Time,
    ) -> &'a LogicExpr<'a> {
        let subj_members: Vec<Term<'a>> = subjects.iter()
            .map(|s| Term::Constant(*s))
            .collect();
        let obj_members: Vec<Term<'a>> = objects.iter()
            .map(|o| Term::Constant(*o))
            .collect();

        let subj_group = Term::Group(self.ctx.terms.alloc_slice(subj_members));
        let obj_group = Term::Group(self.ctx.terms.alloc_slice(obj_members));

        let expr = self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: verb,
            args: self.ctx.terms.alloc_slice([subj_group, obj_group]),
            world: None,
        });

        match verb_time {
            Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: TemporalOperator::Past,
                body: expr,
            }),
            Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: TemporalOperator::Future,
                body: expr,
            }),
            _ => expr,
        }
    }

    fn parse_control_structure(
        &mut self,
        subject: &NounPhrase<'a>,
        verb: Symbol,
        verb_time: Time,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        let subject_sym = subject.noun;
        let verb_str = self.interner.resolve(verb);

        if Lexer::is_raising_verb(verb_str) {
            if !self.check_to() {
                return Ok(self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: verb,
                    args: self.ctx.terms.alloc_slice([Term::Constant(subject_sym)]),
                    world: None,
                }));
            }
            self.advance();

            if !self.check_verb() {
                return Ok(self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: verb,
                    args: self.ctx.terms.alloc_slice([Term::Constant(subject_sym)]),
                    world: None,
                }));
            }

            let inf_verb = self.consume_verb();

            let embedded = if self.is_control_verb(inf_verb) {
                let raised_np = NounPhrase {
                    noun: subject_sym,
                    definiteness: None,
                    adjectives: &[],
                    possessor: None,
                    pps: &[],
                    superlative: None,
                };
                self.parse_control_structure(&raised_np, inf_verb, Time::None)?
            } else {
                self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: inf_verb,
                    args: self.ctx.terms.alloc_slice([Term::Constant(subject_sym)]),
                    world: None,
                })
            };

            let result = self.ctx.exprs.alloc(LogicExpr::Scopal {
                operator: verb,
                body: embedded,
            });

            return Ok(match verb_time {
                Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: TemporalOperator::Past,
                    body: result,
                }),
                Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: TemporalOperator::Future,
                    body: result,
                }),
                _ => result,
            });
        }

        let is_object_control = Lexer::is_object_control_verb(self.interner.resolve(verb));
        let (object_term, pro_controller_sym) = if self.check_to() {
            (None, subject_sym)
        } else if self.check_content_word() {
            let object_np = self.parse_noun_phrase(false)?;
            let obj_sym = object_np.noun;

            let controller = if is_object_control {
                obj_sym
            } else {
                subject_sym
            };
            (
                Some(self.ctx.terms.alloc(Term::Constant(obj_sym))),
                controller,
            )
        } else {
            (None, subject_sym)
        };

        if !self.check_to() {
            return Ok(self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: match object_term {
                    Some(obj) => self.ctx.terms.alloc_slice([
                        Term::Constant(subject_sym),
                        Term::Constant(match obj {
                            Term::Constant(s) => *s,
                            _ => subject_sym,
                        }),
                    ]),
                    None => self.ctx.terms.alloc_slice([Term::Constant(subject_sym)]),
                },
                world: None,
            }));
        }
        self.advance();

        if !self.check_verb() {
            return Ok(self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice([Term::Constant(subject_sym)]),
                world: None,
            }));
        }

        let inf_verb = self.consume_verb();
        let inf_verb_str = self.interner.resolve(inf_verb).to_lowercase();

        let infinitive = if inf_verb_str == "be" && self.check_verb() {
            let passive_verb = self.consume_verb();
            // An agent by-phrase fills the first argument slot, matching the
            // finite passive ("was seen by the people" → See(People, s)).
            let mut passive_args = vec![Term::Constant(pro_controller_sym)];
            if self.check_preposition_is("by")
                && self
                    .tokens
                    .get(self.current + 1)
                    .map_or(false, |t| matches!(
                        t.kind,
                        TokenType::ProperName(_) | TokenType::Noun(_) | TokenType::Article(_)
                    ))
            {
                self.advance(); // by
                let agent_np = self.parse_noun_phrase(false)?;
                passive_args.insert(0, Term::Constant(agent_np.noun));
            }
            let passive_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: passive_verb,
                args: self.ctx.terms.alloc_slice(passive_args),
                world: None,
            });
            self.ctx.voice(crate::ast::VoiceOperator::Passive, passive_pred)
        } else if self.is_control_verb(inf_verb) {
            let controller_np = NounPhrase {
                noun: pro_controller_sym,
                definiteness: None,
                adjectives: &[],
                possessor: None,
                pps: &[],
                superlative: None,
            };
            self.parse_control_structure(&controller_np, inf_verb, Time::None)?
        } else {
            self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: inf_verb,
                args: self
                    .ctx
                    .terms
                    .alloc_slice([Term::Constant(pro_controller_sym)]),
                world: None,
            })
        };

        let control = self.ctx.exprs.alloc(LogicExpr::Control {
            verb,
            subject: self.ctx.terms.alloc(Term::Constant(subject_sym)),
            object: object_term,
            infinitive,
        });

        Ok(match verb_time {
            Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: TemporalOperator::Past,
                body: control,
            }),
            Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: TemporalOperator::Future,
                body: control,
            }),
            _ => control,
        })
    }

    fn is_control_verb(&self, verb: Symbol) -> bool {
        let lemma = self.interner.resolve(verb);
        Lexer::is_subject_control_verb(lemma)
            || Lexer::is_object_control_verb(lemma)
            || Lexer::is_raising_verb(lemma)
    }
}
