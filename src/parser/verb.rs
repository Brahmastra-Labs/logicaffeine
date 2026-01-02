use super::clause::ClauseParsing;
use super::modal::ModalParsing;
use super::noun::NounParsing;
use super::pragmatics::PragmaticsParsing;
use super::{ParseResult, Parser};
use crate::ast::{
    AspectOperator, LogicExpr, NeoEventData, NounPhrase, QuantifierKind, TemporalOperator, Term,
    ThematicRole,
};
use crate::context::{Gender, Number};
use crate::error::{ParseError, ParseErrorKind};
use crate::intern::Symbol;
use crate::lexer::Lexer;
use crate::lexicon::{Aspect, Definiteness, Time};
use crate::token::{FocusKind, Span, TokenType};

use crate::ast::Stmt;

pub trait LogicVerbParsing<'a, 'ctx, 'int> {
    fn parse_predicate_with_subject(&mut self, subject_symbol: Symbol)
        -> ParseResult<&'a LogicExpr<'a>>;
    /// Try to parse a plural subject construction like "John and Mary verb".
    /// Returns Ok(Some(expr)) if successful, Ok(None) if not a plural subject (backtrack),
    /// or Err if there's a semantic error (e.g., "respectively" with mismatched lengths).
    fn try_parse_plural_subject(&mut self, first_subject: &NounPhrase<'a>)
        -> Result<Option<&'a LogicExpr<'a>>, ParseError>;
    fn parse_control_structure(
        &mut self,
        subject: &NounPhrase<'a>,
        verb: Symbol,
        verb_time: Time,
    ) -> ParseResult<&'a LogicExpr<'a>>;
    fn is_control_verb(&self, verb: Symbol) -> bool;
    /// Build a group predicate for intransitive verbs with multiple subjects
    fn build_group_predicate(
        &mut self,
        subjects: &[Symbol],
        verb: Symbol,
        verb_time: Time,
    ) -> &'a LogicExpr<'a>;
    /// Build a transitive predicate with group subject and group object
    fn build_group_transitive(
        &mut self,
        subjects: &[Symbol],
        objects: &[Symbol],
        verb: Symbol,
        verb_time: Time,
    ) -> &'a LogicExpr<'a>;
}

pub trait ImperativeVerbParsing<'a, 'ctx, 'int> {
    fn parse_statement_with_subject(&mut self, subject_symbol: Symbol)
        -> ParseResult<Stmt<'a>>;
}

impl<'a, 'ctx, 'int> LogicVerbParsing<'a, 'ctx, 'int> for Parser<'a, 'ctx, 'int> {
    fn parse_predicate_with_subject(
        &mut self,
        subject_symbol: Symbol,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        let subject_term = Term::Constant(subject_symbol);

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
                    let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                        event_var,
                        verb,
                        roles: self.ctx.roles.alloc_slice(vec![]), // No thematic roles
                        modifiers: self.ctx.syms.alloc_slice(vec![]),
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

        if self.check(&TokenType::Never) {
            self.advance();
            let verb = self.consume_verb();
            let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice([subject_term]),
            });
            return Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: verb_pred,
            }));
        }

        if self.check_modal() {
            return self.parse_aspect_chain(subject_symbol);
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
                            let reconstructed = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                                event_var,
                                verb: template.verb,
                                roles: self.ctx.roles.alloc_slice(roles),
                                modifiers: self.ctx.syms.alloc_slice(template.modifiers.clone()),
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

                let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var,
                    verb,
                    roles: self.ctx.roles.alloc_slice(roles),
                    modifiers: self.ctx.syms.alloc_slice(modifiers),
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

        if self.check_auxiliary() {
            let aux_time = if let TokenType::Auxiliary(time) = self.advance().kind {
                time
            } else {
                Time::None
            };
            self.pending_time = Some(aux_time);

            if self.match_token(&[TokenType::Not]) {
                self.negative_depth += 1;

                if self.check_verb() {
                    let verb = self.consume_verb();

                    if self.check_quantifier() {
                        let quantifier_token = self.advance().kind.clone();
                        let object_np = self.parse_noun_phrase(false)?;
                        let obj_var = self.next_var_name();

                        let obj_restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: object_np.noun,
                            args: self.ctx.terms.alloc_slice([Term::Variable(obj_var)]),
                        });

                        let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: verb,
                            args: self
                                .ctx
                                .terms
                                .alloc_slice([subject_term, Term::Variable(obj_var)]),
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
                                            op: TokenType::If,
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
                                    op: TokenType::If,
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
                        });

                        let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: verb,
                            args: self.ctx.terms.alloc_slice([subject_term, Term::Variable(obj_var)]),
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

                    if self.check_content_word() || self.check_article() {
                        let object = self.parse_noun_phrase(false)?;
                        let object_term = self.noun_phrase_to_term(&object);
                        roles.push((ThematicRole::Theme, object_term));
                    }

                    let event_var = self.get_event_var();
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

                return Ok(if copula_time == Time::Past {
                    self.ctx.exprs.alloc(LogicExpr::Temporal {
                        operator: TemporalOperator::Past,
                        body: with_aspect,
                    })
                } else {
                    with_aspect
                });
            }

            let predicate = self.consume_content_word()?;
            return Ok(self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: predicate,
                args: self.ctx.terms.alloc_slice([subject_term]),
            }));
        }

        if self.check_verb() {
            let (mut verb, verb_time, verb_aspect, verb_class) = self.consume_verb_with_metadata();
            let mut args = vec![subject_term.clone()];

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
                        let reconstructed = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                            event_var,
                            verb: template.verb,
                            roles: self.ctx.roles.alloc_slice(roles),
                            modifiers: self.ctx.syms.alloc_slice(template.modifiers.clone()),
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

                let know_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var: self.get_event_var(),
                    verb,
                    roles: self.ctx.roles.alloc_slice(vec![
                        (ThematicRole::Agent, subject_term),
                        (ThematicRole::Theme, Term::Proposition(question)),
                    ]),
                    modifiers: self.ctx.syms.alloc_slice(vec![]),
                })));

                return Ok(know_event);
            }

            let mut object_term: Option<Term<'a>> = None;
            let mut second_object_term: Option<Term<'a>> = None;
            if self.check(&TokenType::Reflexive) {
                self.advance();
                let term = Term::Constant(subject_symbol);
                object_term = Some(term);
                args.push(term);
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

                let resolved = self
                    .resolve_pronoun(gender, number)
                    .unwrap_or_else(|| self.interner.intern("?"));
                let term = Term::Constant(resolved);
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
            } else if self.check_quantifier() || self.check_article() {
                let obj_quantifier = if self.check_quantifier() {
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
                    let obj_restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: object_np.noun,
                        args: self.ctx.terms.alloc_slice([Term::Variable(obj_var)]),
                    });

                    let event_var = self.get_event_var();
                    let mut modifiers = self.collect_adverbs();
                    let effective_time = self.pending_time.take().unwrap_or(verb_time);
                    match effective_time {
                        Time::Past => modifiers.push(self.interner.intern("Past")),
                        Time::Future => modifiers.push(self.interner.intern("Future")),
                        _ => {}
                    }

                    let roles = vec![
                        (ThematicRole::Agent, subject_term),
                        (ThematicRole::Theme, Term::Variable(obj_var)),
                    ];

                    let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                        event_var,
                        verb,
                        roles: self.ctx.roles.alloc_slice(roles),
                        modifiers: self.ctx.syms.alloc_slice(modifiers),
                    })));

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
                            op: TokenType::If,
                            right: neo_event,
                        }),
                        TokenType::No => {
                            let neg = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                                op: TokenType::Not,
                                operand: neo_event,
                            });
                            self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                left: obj_restriction,
                                op: TokenType::If,
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
                    let term = Term::Constant(object_np.noun);
                    object_term = Some(term);
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
                    let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                        event_var,
                        verb,
                        roles: self.ctx.roles.alloc_slice(roles),
                        modifiers: self.ctx.syms.alloc_slice(modifiers),
                    })));

                    let pp_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: prep_name,
                        args: self.ctx.terms.alloc_slice([Term::Variable(event_var), pp_obj_term]),
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

                let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var,
                    verb,
                    roles: self.ctx.roles.alloc_slice(roles),
                    modifiers: self.ctx.syms.alloc_slice(modifiers),
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

                if self.check_verb() && self.filler_gap.is_some() {
                    let embedded_subject = potential_object.noun;
                    let embedded_pred = self.parse_predicate_with_subject(embedded_subject)?;

                    let embedded_term = Term::Proposition(embedded_pred);
                    let main_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: verb,
                        args: self.ctx.terms.alloc_slice([subject_term, embedded_term]),
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
                    let resolved = self.resolve_pronoun(gender, number).unwrap_or(unknown);
                    Term::Constant(resolved)
                } else if self.check_content_word() || self.check_article() {
                    let prep_obj = self.parse_noun_phrase(false)?;
                    Term::Constant(prep_obj.noun)
                } else {
                    continue;
                };

                if self.pp_attach_to_noun {
                    if let Some(obj) = object_term {
                        let pp_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: prep_name,
                            args: self.ctx.terms.alloc_slice([obj, pp_obj_term]),
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
            roles.push((ThematicRole::Agent, subject_term));
            if let Some(second_obj) = second_object_term {
                if let Some(first_obj) = object_term {
                    roles.push((ThematicRole::Recipient, first_obj));
                }
                roles.push((ThematicRole::Theme, second_obj));
            } else if let Some(obj) = object_term {
                roles.push((ThematicRole::Theme, obj));
            }

            let event_var = self.get_event_var();
            let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                event_var,
                verb,
                roles: self.ctx.roles.alloc_slice(roles.clone()),
                modifiers: self.ctx.syms.alloc_slice(modifiers.clone()),
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

            // Apply aspectual operators based on verb class
            let with_aspect = if verb_aspect == Aspect::Simple && effective_time == Time::Present {
                // Non-state verbs in simple present get Habitual reading
                if !verb_class.is_stative() {
                    self.ctx.exprs.alloc(LogicExpr::Aspectual {
                        operator: AspectOperator::Habitual,
                        body: with_pps,
                    })
                } else {
                    with_pps
                }
            } else if verb_aspect == Aspect::Progressive {
                // Semelfactive + Progressive → Iterative
                if verb_class == crate::lexicon::VerbClass::Semelfactive {
                    self.ctx.exprs.alloc(LogicExpr::Aspectual {
                        operator: AspectOperator::Iterative,
                        body: with_pps,
                    })
                } else {
                    with_pps
                }
            } else {
                with_pps
            };

            Ok(with_aspect)
        } else {
            Ok(self.ctx.exprs.alloc(LogicExpr::Atom(subject_symbol)))
        }
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

        if !self.check_verb() {
            self.current = saved_pos;
            return Ok(None);
        }

        // Register the coordinated subjects as a plural entity for pronoun resolution
        {
            use crate::context::{Gender, Number};
            let group_name = subjects.iter()
                .map(|s| self.interner.resolve(*s))
                .collect::<Vec<_>>()
                .join("⊕");
            self.register_entity(&group_name, "group", Gender::Unknown, Number::Plural);
        }

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
            });
            let pred2 = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice([
                    Term::Constant(subjects[1]),
                    Term::Constant(subjects[0]),
                ]),
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
        if self.check(&TokenType::Respectively) {
            let respectively_span = self.peek().span;
            self.advance(); // consume "respectively"

            if subjects.len() != objects.len() {
                return Err(ParseError {
                    kind: ParseErrorKind::RespectivelyLengthMismatch {
                        subject_count: subjects.len(),
                        object_count: objects.len(),
                    },
                    span: respectively_span,
                });
            }

            // Build pairwise predicates: See(J,T) ∧ See(M,J) ∧ ...
            let mut conjuncts: Vec<&'a LogicExpr<'a>> = Vec::new();
            for (subj, obj) in subjects.iter().zip(objects.iter()) {
                let event_var = self.get_event_var();
                let roles = vec![
                    (ThematicRole::Agent, Term::Constant(*subj)),
                    (ThematicRole::Theme, Term::Constant(*obj)),
                ];
                let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var,
                    verb,
                    roles: self.ctx.roles.alloc_slice(roles),
                    modifiers: self.ctx.syms.alloc_slice(vec![]),
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
                }));
            }
            self.advance();

            if !self.check_verb() {
                return Ok(self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: verb,
                    args: self.ctx.terms.alloc_slice([Term::Constant(subject_sym)]),
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
            }));
        }
        self.advance();

        if !self.check_verb() {
            return Ok(self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice([Term::Constant(subject_sym)]),
            }));
        }

        let inf_verb = self.consume_verb();
        let inf_verb_str = self.interner.resolve(inf_verb).to_lowercase();

        let infinitive = if inf_verb_str == "be" && self.check_verb() {
            let passive_verb = self.consume_verb();
            let passive_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: passive_verb,
                args: self
                    .ctx
                    .terms
                    .alloc_slice([Term::Constant(pro_controller_sym)]),
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
