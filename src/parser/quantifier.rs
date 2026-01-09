use super::clause::ClauseParsing;
use super::modal::ModalParsing;
use super::noun::NounParsing;
use super::{NegativeScopeMode, ParseResult, Parser};
use crate::ast::{LogicExpr, NeoEventData, NounPhrase, QuantifierKind, Term, ThematicRole};
use crate::drs::{Gender, Number};
use crate::drs::ReferentSource;
use crate::error::{ParseError, ParseErrorKind};
use crate::intern::Symbol;
use crate::lexer::Lexer;
use crate::lexicon::{get_canonical_verb, is_subsective, lookup_verb_db, Definiteness, Feature, Time};
use crate::token::{PresupKind, TokenType};

pub trait QuantifierParsing<'a, 'ctx, 'int> {
    fn parse_quantified(&mut self) -> ParseResult<&'a LogicExpr<'a>>;
    fn parse_restriction(&mut self, var_name: Symbol) -> ParseResult<&'a LogicExpr<'a>>;
    fn parse_verb_phrase_for_restriction(&mut self, var_name: Symbol) -> ParseResult<&'a LogicExpr<'a>>;
    fn combine_with_and(&self, exprs: Vec<&'a LogicExpr<'a>>) -> ParseResult<&'a LogicExpr<'a>>;
    fn wrap_with_definiteness_full(
        &mut self,
        np: &NounPhrase<'a>,
        predicate: &'a LogicExpr<'a>,
    ) -> ParseResult<&'a LogicExpr<'a>>;
    fn wrap_with_definiteness(
        &mut self,
        definiteness: Option<Definiteness>,
        noun: Symbol,
        predicate: &'a LogicExpr<'a>,
    ) -> ParseResult<&'a LogicExpr<'a>>;
    fn wrap_with_definiteness_and_adjectives(
        &mut self,
        definiteness: Option<Definiteness>,
        noun: Symbol,
        adjectives: &[Symbol],
        predicate: &'a LogicExpr<'a>,
    ) -> ParseResult<&'a LogicExpr<'a>>;
    fn wrap_with_definiteness_and_adjectives_and_pps(
        &mut self,
        definiteness: Option<Definiteness>,
        noun: Symbol,
        adjectives: &[Symbol],
        pps: &[&'a LogicExpr<'a>],
        predicate: &'a LogicExpr<'a>,
    ) -> ParseResult<&'a LogicExpr<'a>>;
    fn wrap_with_definiteness_for_object(
        &mut self,
        definiteness: Option<Definiteness>,
        noun: Symbol,
        predicate: &'a LogicExpr<'a>,
    ) -> ParseResult<&'a LogicExpr<'a>>;
    fn substitute_pp_placeholder(&mut self, pp: &'a LogicExpr<'a>, var: Symbol) -> &'a LogicExpr<'a>;
    fn substitute_constant_with_var(
        &self,
        expr: &'a LogicExpr<'a>,
        constant_name: Symbol,
        var_name: Symbol,
    ) -> ParseResult<&'a LogicExpr<'a>>;
    fn substitute_constant_with_var_sym(
        &self,
        expr: &'a LogicExpr<'a>,
        constant_name: Symbol,
        var_name: Symbol,
    ) -> ParseResult<&'a LogicExpr<'a>>;
    fn substitute_constant_with_sigma(
        &self,
        expr: &'a LogicExpr<'a>,
        constant_name: Symbol,
        sigma_term: Term<'a>,
    ) -> ParseResult<&'a LogicExpr<'a>>;
    fn find_main_verb_name(&self, expr: &LogicExpr<'a>) -> Option<Symbol>;
    fn transform_cardinal_to_group(&mut self, expr: &'a LogicExpr<'a>) -> ParseResult<&'a LogicExpr<'a>>;
    fn build_verb_neo_event(
        &mut self,
        verb: Symbol,
        subject_var: Symbol,
        object: Option<Term<'a>>,
        modifiers: Vec<Symbol>,
    ) -> &'a LogicExpr<'a>;
}

impl<'a, 'ctx, 'int> QuantifierParsing<'a, 'ctx, 'int> for Parser<'a, 'ctx, 'int> {
    fn parse_quantified(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        let quantifier_token = self.previous().kind.clone();
        let var_name = self.next_var_name();

        // Track if we're inside a "No" quantifier - referents introduced here
        // are inaccessible for cross-sentence anaphora
        let was_in_negative_quantifier = self.in_negative_quantifier;
        if matches!(quantifier_token, TokenType::No) {
            self.in_negative_quantifier = true;
        }

        let subject_pred = self.parse_restriction(var_name)?;

        if self.check_modal() {
            use crate::ast::ModalFlavor;

            self.advance();
            let vector = self.token_to_vector(&self.previous().kind.clone());
            let verb = self.consume_content_word()?;

            // Parse object if present (e.g., "can enter the room" -> room is object)
            let obj_term = if self.check_content_word() || self.check_article() {
                let obj_np = self.parse_noun_phrase(false)?;
                Some(self.noun_phrase_to_term(&obj_np))
            } else {
                None
            };

            // Collect any trailing adverbs
            let modifiers = self.collect_adverbs();
            let verb_pred = self.build_verb_neo_event(verb, var_name, obj_term, modifiers);

            // Determine quantifier kind first (shared by both branches)
            let kind = match quantifier_token {
                TokenType::All | TokenType::No => QuantifierKind::Universal,
                TokenType::Any => {
                    if self.is_negative_context() {
                        QuantifierKind::Existential
                    } else {
                        QuantifierKind::Universal
                    }
                }
                TokenType::Some => QuantifierKind::Existential,
                TokenType::Most => QuantifierKind::Most,
                TokenType::Few => QuantifierKind::Few,
                TokenType::Many => QuantifierKind::Many,
                TokenType::Cardinal(n) => QuantifierKind::Cardinal(n),
                TokenType::AtLeast(n) => QuantifierKind::AtLeast(n),
                TokenType::AtMost(n) => QuantifierKind::AtMost(n),
                _ => {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnknownQuantifier {
                            found: quantifier_token.clone(),
                        },
                        span: self.current_span(),
                    })
                }
            };

            // Branch on modal flavor for scope handling
            if vector.flavor == ModalFlavor::Root {
                // === NARROW SCOPE (De Re) ===
                // Root modals (can, must, should) attach to the predicate inside the quantifier
                // "Some birds can fly" → ∃x(Bird(x) ∧ ◇Fly(x))

                // Wrap the verb predicate in the modal
                let modal_verb = self.ctx.exprs.alloc(LogicExpr::Modal {
                    vector,
                    operand: verb_pred,
                });

                let body = match quantifier_token {
                    TokenType::All => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: subject_pred,
                        op: TokenType::If,
                        right: modal_verb,
                    }),
                    TokenType::Any => {
                        if self.is_negative_context() {
                            self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                left: subject_pred,
                                op: TokenType::And,
                                right: modal_verb,
                            })
                        } else {
                            self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                left: subject_pred,
                                op: TokenType::If,
                                right: modal_verb,
                            })
                        }
                    }
                    TokenType::Some
                    | TokenType::Most
                    | TokenType::Few
                    | TokenType::Many
                    | TokenType::Cardinal(_)
                    | TokenType::AtLeast(_)
                    | TokenType::AtMost(_) => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: subject_pred,
                        op: TokenType::And,
                        right: modal_verb,
                    }),
                    TokenType::No => {
                        let neg = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                            op: TokenType::Not,
                            operand: modal_verb,
                        });
                        self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: subject_pred,
                            op: TokenType::If,
                            right: neg,
                        })
                    }
                    _ => {
                        return Err(ParseError {
                            kind: ParseErrorKind::UnknownQuantifier {
                                found: quantifier_token.clone(),
                            },
                            span: self.current_span(),
                        })
                    }
                };

                // Build quantifier (modal is inside)
                let mut result = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind,
                    variable: var_name,
                    body,
                    island_id: self.current_island,
                });

                // Process donkey bindings for indefinites in restrictions (e.g., "who lacks a key")
                for (_noun, donkey_var, used, wide_neg) in self.donkey_bindings.iter().rev() {
                    if *used {
                        // Donkey anaphora: wrap with ∀ at outer scope
                        result = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                            kind: QuantifierKind::Universal,
                            variable: *donkey_var,
                            body: result,
                            island_id: self.current_island,
                        });
                    } else {
                        // Non-donkey: wrap with ∃ INSIDE the restriction
                        result = self.wrap_donkey_in_restriction(result, *donkey_var, *wide_neg);
                    }
                }
                self.donkey_bindings.clear();

                self.in_negative_quantifier = was_in_negative_quantifier;
                return Ok(result);

            } else {
                // === WIDE SCOPE (De Dicto) ===
                // Epistemic modals (might, may) wrap the entire quantifier
                // "Some unicorns might exist" → ◇∃x(Unicorn(x) ∧ Exist(x))

                let body = match quantifier_token {
                    TokenType::All => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: subject_pred,
                        op: TokenType::If,
                        right: verb_pred,
                    }),
                    TokenType::Any => {
                        if self.is_negative_context() {
                            self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                left: subject_pred,
                                op: TokenType::And,
                                right: verb_pred,
                            })
                        } else {
                            self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                left: subject_pred,
                                op: TokenType::If,
                                right: verb_pred,
                            })
                        }
                    }
                    TokenType::Some
                    | TokenType::Most
                    | TokenType::Few
                    | TokenType::Many
                    | TokenType::Cardinal(_)
                    | TokenType::AtLeast(_)
                    | TokenType::AtMost(_) => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: subject_pred,
                        op: TokenType::And,
                        right: verb_pred,
                    }),
                    TokenType::No => {
                        let neg = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                            op: TokenType::Not,
                            operand: verb_pred,
                        });
                        self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: subject_pred,
                            op: TokenType::If,
                            right: neg,
                        })
                    }
                    _ => {
                        return Err(ParseError {
                            kind: ParseErrorKind::UnknownQuantifier {
                                found: quantifier_token.clone(),
                            },
                            span: self.current_span(),
                        })
                    }
                };

                let mut result = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind,
                    variable: var_name,
                    body,
                    island_id: self.current_island,
                });

                // Process donkey bindings for indefinites in restrictions (e.g., "who lacks a key")
                for (_noun, donkey_var, used, wide_neg) in self.donkey_bindings.iter().rev() {
                    if *used {
                        // Donkey anaphora: wrap with ∀ at outer scope
                        result = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                            kind: QuantifierKind::Universal,
                            variable: *donkey_var,
                            body: result,
                            island_id: self.current_island,
                        });
                    } else {
                        // Non-donkey: wrap with ∃ INSIDE the restriction
                        result = self.wrap_donkey_in_restriction(result, *donkey_var, *wide_neg);
                    }
                }
                self.donkey_bindings.clear();

                // Wrap the entire quantifier in the modal
                self.in_negative_quantifier = was_in_negative_quantifier;
                return Ok(self.ctx.exprs.alloc(LogicExpr::Modal {
                    vector,
                    operand: result,
                }));
            }
        }

        if self.check_auxiliary() {
            let aux_token = self.advance();
            let aux_time = if let TokenType::Auxiliary(time) = aux_token.kind.clone() {
                time
            } else {
                Time::None
            };
            self.pending_time = Some(aux_time);

            let is_negated = self.match_token(&[TokenType::Not]);
            if is_negated {
                self.negative_depth += 1;
            }

            if self.check_verb() {
                let verb = self.consume_verb();

                // Convert aux_time to modifier
                let modifiers = match aux_time {
                    Time::Past => vec![self.interner.intern("Past")],
                    Time::Future => vec![self.interner.intern("Future")],
                    _ => vec![],
                };

                let verb_pred = self.build_verb_neo_event(verb, var_name, None, modifiers);

                let maybe_negated = if is_negated {
                    self.negative_depth -= 1;
                    self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand: verb_pred,
                    })
                } else {
                    verb_pred
                };

                let body = match quantifier_token {
                    TokenType::All | TokenType::Any => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: subject_pred,
                        op: TokenType::If,
                        right: maybe_negated,
                    }),
                    _ => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: subject_pred,
                        op: TokenType::And,
                        right: maybe_negated,
                    }),
                };

                let kind = match quantifier_token {
                    TokenType::All | TokenType::No => QuantifierKind::Universal,
                    TokenType::Some => QuantifierKind::Existential,
                    TokenType::Most => QuantifierKind::Most,
                    TokenType::Few => QuantifierKind::Few,
                    TokenType::Cardinal(n) => QuantifierKind::Cardinal(n),
                    TokenType::AtLeast(n) => QuantifierKind::AtLeast(n),
                    TokenType::AtMost(n) => QuantifierKind::AtMost(n),
                    _ => QuantifierKind::Universal,
                };

                self.in_negative_quantifier = was_in_negative_quantifier;
                return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind,
                    variable: var_name,
                    body,
                    island_id: self.current_island,
                }));
            }
        }

        // Only trigger presupposition if followed by gerund complement
        if self.check_presup_trigger() && self.is_followed_by_gerund() {
            let presup_kind = match self.advance().kind {
                TokenType::PresupTrigger(kind) => kind,
                TokenType::Verb { lemma, .. } => {
                    let s = self.interner.resolve(lemma).to_lowercase();
                    crate::lexicon::lookup_presup_trigger(&s)
                        .expect("Lexicon mismatch: Verb flagged as trigger but lookup failed")
                }
                _ => panic!("Expected presupposition trigger"),
            };

            let complement = if self.check_verb() {
                let verb = self.consume_verb();
                let modifiers = self.collect_adverbs();
                self.build_verb_neo_event(verb, var_name, None, modifiers)
            } else {
                let unknown = self.interner.intern("?");
                self.ctx.exprs.alloc(LogicExpr::Atom(unknown))
            };

            let verb_pred = match presup_kind {
                PresupKind::Stop => self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                    op: TokenType::Not,
                    operand: complement,
                }),
                PresupKind::Start | PresupKind::Continue => complement,
                PresupKind::Regret | PresupKind::Realize | PresupKind::Know => complement,
            };

            let body = match quantifier_token {
                TokenType::All | TokenType::Any => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: subject_pred,
                    op: TokenType::If,
                    right: verb_pred,
                }),
                _ => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: subject_pred,
                    op: TokenType::And,
                    right: verb_pred,
                }),
            };

            let kind = match quantifier_token {
                TokenType::All | TokenType::No => QuantifierKind::Universal,
                TokenType::Some => QuantifierKind::Existential,
                TokenType::Most => QuantifierKind::Most,
                TokenType::Few => QuantifierKind::Few,
                TokenType::Many => QuantifierKind::Many,
                TokenType::Cardinal(n) => QuantifierKind::Cardinal(n),
                TokenType::AtLeast(n) => QuantifierKind::AtLeast(n),
                TokenType::AtMost(n) => QuantifierKind::AtMost(n),
                _ => QuantifierKind::Universal,
            };

            self.in_negative_quantifier = was_in_negative_quantifier;
            return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind,
                variable: var_name,
                body,
                island_id: self.current_island,
            }));
        }

        if self.check_verb() {
            let verb = self.consume_verb();
            let mut args = vec![Term::Variable(var_name)];

            if self.check_pronoun() {
                let token = self.peek().clone();
                if let TokenType::Pronoun { gender, .. } = token.kind {
                    self.advance();
                    if let Some(donkey_var) = self.resolve_donkey_pronoun(gender) {
                        args.push(Term::Variable(donkey_var));
                    } else {
                        let resolved = self.resolve_pronoun(gender, Number::Singular)?;
                        let term = match resolved {
                            super::ResolvedPronoun::Variable(s) => Term::Variable(s),
                            super::ResolvedPronoun::Constant(s) => Term::Constant(s),
                        };
                        args.push(term);
                    }
                }
            } else if self.check_npi_object() {
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

                let npi_modifiers = self.collect_adverbs();
                let verb_with_obj = self.build_verb_neo_event(
                    verb,
                    var_name,
                    Some(Term::Variable(obj_var)),
                    npi_modifiers,
                );

                let npi_body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: obj_restriction,
                    op: TokenType::And,
                    right: verb_with_obj,
                });

                let npi_quantified = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: QuantifierKind::Existential,
                    variable: obj_var,
                    body: npi_body,
                    island_id: self.current_island,
                });

                let negated_npi = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                    op: TokenType::Not,
                    operand: npi_quantified,
                });

                let body = match quantifier_token {
                    TokenType::All | TokenType::No => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: subject_pred,
                        op: TokenType::If,
                        right: negated_npi,
                    }),
                    _ => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: subject_pred,
                        op: TokenType::And,
                        right: negated_npi,
                    }),
                };

                let kind = match quantifier_token {
                    TokenType::All | TokenType::No => QuantifierKind::Universal,
                    TokenType::Some => QuantifierKind::Existential,
                    TokenType::Most => QuantifierKind::Most,
                    TokenType::Few => QuantifierKind::Few,
                    TokenType::Many => QuantifierKind::Many,
                    TokenType::Cardinal(n) => QuantifierKind::Cardinal(n),
                    TokenType::AtLeast(n) => QuantifierKind::AtLeast(n),
                    TokenType::AtMost(n) => QuantifierKind::AtMost(n),
                    _ => QuantifierKind::Universal,
                };

                self.in_negative_quantifier = was_in_negative_quantifier;
                return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind,
                    variable: var_name,
                    body,
                    island_id: self.current_island,
                }));
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

                let object = self.parse_noun_phrase(false)?;

                if let Some(obj_q) = obj_quantifier {
                    let obj_var = self.next_var_name();

                    // Introduce object referent in DRS for cross-sentence anaphora (telescoping)
                    // BUT: If inside "No X" quantifier, mark with NegationScope to block accessibility
                    let obj_gender = Self::infer_noun_gender(self.interner.resolve(object.noun));
                    let obj_number = if Self::is_plural_noun(self.interner.resolve(object.noun)) {
                        Number::Plural
                    } else {
                        Number::Singular
                    };
                    if self.in_negative_quantifier {
                        self.drs.introduce_referent_with_source(obj_var, object.noun, obj_gender, obj_number, ReferentSource::NegationScope);
                    } else {
                        self.drs.introduce_referent(obj_var, object.noun, obj_gender, obj_number);
                    }

                    let obj_restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: object.noun,
                        args: self.ctx.terms.alloc_slice([Term::Variable(obj_var)]),
                        world: None,
                    });

                    let obj_modifiers = self.collect_adverbs();
                    let verb_with_obj = self.build_verb_neo_event(
                        verb,
                        var_name,
                        Some(Term::Variable(obj_var)),
                        obj_modifiers,
                    );

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
                            right: verb_with_obj,
                        }),
                        TokenType::No => {
                            let neg = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                                op: TokenType::Not,
                                operand: verb_with_obj,
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
                            right: verb_with_obj,
                        }),
                    };

                    let obj_quantified = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                        kind: obj_kind,
                        variable: obj_var,
                        body: obj_body,
                        island_id: self.current_island,
                    });

                    let subj_kind = match quantifier_token {
                        TokenType::All | TokenType::No => QuantifierKind::Universal,
                        TokenType::Any => {
                            if self.is_negative_context() {
                                QuantifierKind::Existential
                            } else {
                                QuantifierKind::Universal
                            }
                        }
                        TokenType::Some => QuantifierKind::Existential,
                        TokenType::Most => QuantifierKind::Most,
                        TokenType::Few => QuantifierKind::Few,
                        TokenType::Many => QuantifierKind::Many,
                        TokenType::Cardinal(n) => QuantifierKind::Cardinal(n),
                        TokenType::AtLeast(n) => QuantifierKind::AtLeast(n),
                        TokenType::AtMost(n) => QuantifierKind::AtMost(n),
                        _ => QuantifierKind::Universal,
                    };

                    let subj_body = match quantifier_token {
                        TokenType::All => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: subject_pred,
                            op: TokenType::If,
                            right: obj_quantified,
                        }),
                        TokenType::No => {
                            let neg = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                                op: TokenType::Not,
                                operand: obj_quantified,
                            });
                            self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                left: subject_pred,
                                op: TokenType::If,
                                right: neg,
                            })
                        }
                        _ => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: subject_pred,
                            op: TokenType::And,
                            right: obj_quantified,
                        }),
                    };

                    self.in_negative_quantifier = was_in_negative_quantifier;
                    return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                        kind: subj_kind,
                        variable: var_name,
                        body: subj_body,
                        island_id: self.current_island,
                    }));
                } else {
                    args.push(Term::Constant(object.noun));
                }
            } else if self.check_content_word() {
                let object = self.parse_noun_phrase(false)?;
                args.push(Term::Constant(object.noun));
            }

            // Extract object term from args if present (args[0] is subject, args[1] is object)
            let obj_term = if args.len() > 1 {
                Some(args.remove(1))
            } else {
                None
            };
            // Collect any trailing adverbs (e.g., "bark loudly")
            let modifiers = self.collect_adverbs();
            let verb_pred = self.build_verb_neo_event(verb, var_name, obj_term, modifiers);

            let body = match quantifier_token {
                TokenType::All => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: subject_pred,
                    op: TokenType::If,
                    right: verb_pred,
                }),
                TokenType::Any => {
                    if self.is_negative_context() {
                        self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: subject_pred,
                            op: TokenType::And,
                            right: verb_pred,
                        })
                    } else {
                        self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: subject_pred,
                            op: TokenType::If,
                            right: verb_pred,
                        })
                    }
                }
                TokenType::Some
                | TokenType::Most
                | TokenType::Few
                | TokenType::Many
                | TokenType::Cardinal(_)
                | TokenType::AtLeast(_)
                | TokenType::AtMost(_) => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: subject_pred,
                    op: TokenType::And,
                    right: verb_pred,
                }),
                TokenType::No => {
                    let neg = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand: verb_pred,
                    });
                    self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: subject_pred,
                        op: TokenType::If,
                        right: neg,
                    })
                }
                _ => {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnknownQuantifier {
                            found: quantifier_token.clone(),
                        },
                        span: self.current_span(),
                    })
                }
            };

            let kind = match quantifier_token {
                TokenType::All | TokenType::No => QuantifierKind::Universal,
                TokenType::Any => {
                    if self.is_negative_context() {
                        QuantifierKind::Existential
                    } else {
                        QuantifierKind::Universal
                    }
                }
                TokenType::Some => QuantifierKind::Existential,
                TokenType::Most => QuantifierKind::Most,
                TokenType::Few => QuantifierKind::Few,
                TokenType::Many => QuantifierKind::Many,
                TokenType::Cardinal(n) => QuantifierKind::Cardinal(n),
                TokenType::AtLeast(n) => QuantifierKind::AtLeast(n),
                TokenType::AtMost(n) => QuantifierKind::AtMost(n),
                _ => {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnknownQuantifier {
                            found: quantifier_token.clone(),
                        },
                        span: self.current_span(),
                    })
                }
            };

            let mut result = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind,
                variable: var_name,
                body,
                island_id: self.current_island,
            });

            for (_noun, donkey_var, used, wide_neg) in self.donkey_bindings.iter().rev() {
                if *used {
                    // Donkey anaphora: wrap with ∀ at outer scope
                    result = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                        kind: QuantifierKind::Universal,
                        variable: *donkey_var,
                        body: result,
                        island_id: self.current_island,
                    });
                } else {
                    // Non-donkey: wrap with ∃ INSIDE the restriction
                    result = self.wrap_donkey_in_restriction(result, *donkey_var, *wide_neg);
                }
            }
            self.donkey_bindings.clear();

            self.in_negative_quantifier = was_in_negative_quantifier;
            return Ok(result);
        }

        self.consume_copula()?;

        let negative = self.match_token(&[TokenType::Not]);
        let predicate_np = self.parse_noun_phrase(true)?;

        let predicate_expr = self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: predicate_np.noun,
            args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
            world: None,
        });

        let final_predicate = if negative {
            self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: predicate_expr,
            })
        } else {
            predicate_expr
        };

        let body = match quantifier_token {
            TokenType::All => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: subject_pred,
                op: TokenType::If,
                right: final_predicate,
            }),
            TokenType::Any => {
                if self.is_negative_context() {
                    self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: subject_pred,
                        op: TokenType::And,
                        right: final_predicate,
                    })
                } else {
                    self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: subject_pred,
                        op: TokenType::If,
                        right: final_predicate,
                    })
                }
            }
            TokenType::Some
            | TokenType::Most
            | TokenType::Few
            | TokenType::Many
            | TokenType::Cardinal(_)
            | TokenType::AtLeast(_)
            | TokenType::AtMost(_) => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: subject_pred,
                op: TokenType::And,
                right: final_predicate,
            }),
            TokenType::No => {
                let neg_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: predicate_np.noun,
                    args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
                    world: None,
                });
                let neg = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                    op: TokenType::Not,
                    operand: neg_pred,
                });
                self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: subject_pred,
                    op: TokenType::If,
                    right: neg,
                })
            }
            _ => {
                return Err(ParseError {
                    kind: ParseErrorKind::UnknownQuantifier {
                        found: quantifier_token.clone(),
                    },
                    span: self.current_span(),
                })
            }
        };

        let kind = match quantifier_token {
            TokenType::All | TokenType::No => QuantifierKind::Universal,
            TokenType::Any => {
                if self.is_negative_context() {
                    QuantifierKind::Existential
                } else {
                    QuantifierKind::Universal
                }
            }
            TokenType::Some => QuantifierKind::Existential,
            TokenType::Most => QuantifierKind::Most,
            TokenType::Few => QuantifierKind::Few,
            TokenType::Many => QuantifierKind::Many,
            TokenType::Cardinal(n) => QuantifierKind::Cardinal(n),
            TokenType::AtLeast(n) => QuantifierKind::AtLeast(n),
            TokenType::AtMost(n) => QuantifierKind::AtMost(n),
            _ => {
                return Err(ParseError {
                    kind: ParseErrorKind::UnknownQuantifier {
                        found: quantifier_token.clone(),
                    },
                    span: self.current_span(),
                })
            }
        };

        let mut result = self.ctx.exprs.alloc(LogicExpr::Quantifier {
            kind,
            variable: var_name,
            body,
            island_id: self.current_island,
        });

        for (_noun, donkey_var, used, wide_neg) in self.donkey_bindings.iter().rev() {
            if *used {
                // Donkey anaphora: wrap with ∀ at outer scope
                result = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: QuantifierKind::Universal,
                    variable: *donkey_var,
                    body: result,
                    island_id: self.current_island,
                });
            } else {
                // Non-donkey: wrap with ∃ INSIDE the restriction
                result = self.wrap_donkey_in_restriction(result, *donkey_var, *wide_neg);
            }
        }
        self.donkey_bindings.clear();

        self.in_negative_quantifier = was_in_negative_quantifier;
        Ok(result)
    }

    fn parse_restriction(&mut self, var_name: Symbol) -> ParseResult<&'a LogicExpr<'a>> {
        let mut conditions: Vec<&'a LogicExpr<'a>> = Vec::new();

        loop {
            if self.is_at_end() {
                break;
            }

            let is_adjective = matches!(self.peek().kind, TokenType::Adjective(_));
            if !is_adjective {
                break;
            }

            let next_is_content = if self.current + 1 < self.tokens.len() {
                matches!(
                    self.tokens[self.current + 1].kind,
                    TokenType::Noun(_) | TokenType::Adjective(_) | TokenType::ProperName(_)
                )
            } else {
                false
            };

            if next_is_content {
                if let TokenType::Adjective(adj) = self.advance().kind.clone() {
                    conditions.push(self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: adj,
                        args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
                        world: None,
                    }));
                }
            } else {
                break;
            }
        }

        let noun = self.consume_content_word()?;
        conditions.push(self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: noun,
            args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
            world: None,
        }));

        while self.check(&TokenType::That) || self.check(&TokenType::Who) {
            self.advance();
            let clause_pred = self.parse_relative_clause(var_name)?;
            conditions.push(clause_pred);
        }

        self.combine_with_and(conditions)
    }

    fn parse_verb_phrase_for_restriction(&mut self, var_name: Symbol) -> ParseResult<&'a LogicExpr<'a>> {
        let var_term = Term::Variable(var_name);
        let verb = self.consume_verb();
        let verb_str_owned = self.interner.resolve(verb).to_string();

        // Check EARLY if verb is lexically negative (e.g., "lacks" -> "Have" with negation)
        // This determines whether donkey bindings need wide scope negation
        let (canonical_verb, is_negative) = get_canonical_verb(&verb_str_owned.to_lowercase())
            .map(|(lemma, neg)| (self.interner.intern(lemma), neg))
            .unwrap_or((verb, false));

        // Determine if this binding needs wide scope negation wrapping
        let needs_wide_scope = is_negative && self.negative_scope_mode == NegativeScopeMode::Wide;

        if Lexer::is_raising_verb(&verb_str_owned) && self.check_to() {
            self.advance();
            if self.check_verb() {
                let inf_verb = self.consume_verb();
                let inf_verb_str = self.interner.resolve(inf_verb).to_lowercase();

                if inf_verb_str == "be" && self.check_content_word() {
                    let adj = self.consume_content_word()?;
                    let embedded = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: adj,
                        args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
                        world: None,
                    });
                    return Ok(self.ctx.exprs.alloc(LogicExpr::Scopal {
                        operator: verb,
                        body: embedded,
                    }));
                }

                let embedded = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: inf_verb,
                    args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
                    world: None,
                });
                return Ok(self.ctx.exprs.alloc(LogicExpr::Scopal {
                    operator: verb,
                    body: embedded,
                }));
            } else if self.check(&TokenType::Is) || self.check(&TokenType::Are) {
                self.advance();
                if self.check_content_word() {
                    let adj = self.consume_content_word()?;
                    let embedded = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: adj,
                        args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
                        world: None,
                    });
                    return Ok(self.ctx.exprs.alloc(LogicExpr::Scopal {
                        operator: verb,
                        body: embedded,
                    }));
                }
            }
        }

        let mut args = vec![var_term];
        let mut extra_conditions: Vec<&'a LogicExpr<'a>> = Vec::new();

        if self.check(&TokenType::Reflexive) {
            self.advance();
            args.push(Term::Variable(var_name));
        } else if (self.check_content_word() || self.check_article()) && !self.check_verb() {
            if matches!(
                self.peek().kind,
                TokenType::Article(Definiteness::Indefinite)
            ) {
                self.advance();
                let noun = self.consume_content_word()?;
                let donkey_var = self.next_var_name();

                if needs_wide_scope {
                    // === WIDE SCOPE MODE ===
                    // Build ¬∃y(Key(y) ∧ ∃e(Have(e) ∧ Agent(e,x) ∧ Theme(e,y))) directly
                    //
                    // We capture the binding HERE and return the complete structure.
                    // DO NOT push to donkey_bindings - that would leak y to outer scope.

                    // Build: Key(y)
                    let restriction_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: noun,
                        args: self.ctx.terms.alloc_slice([Term::Variable(donkey_var)]),
                        world: None,
                    });

                    // Build: ∃e(Have(e) ∧ Agent(e,x) ∧ Theme(e,y)) using Neo-Davidsonian semantics
                    // IMPORTANT: Use build_verb_neo_event() for consistent Full-tier formatting
                    let inner_modifiers = self.collect_adverbs();
                    let verb_pred = self.build_verb_neo_event(
                        canonical_verb,
                        var_name,
                        Some(Term::Variable(donkey_var)),
                        inner_modifiers,
                    );

                    // Build: Key(y) ∧ ∃e(Have(e) ∧ Agent(e,x) ∧ Theme(e,y))
                    let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: restriction_pred,
                        op: TokenType::And,
                        right: verb_pred,
                    });

                    // Build: ∃y(Key(y) ∧ ∃e(Have(e) ∧ ...))
                    let existential = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                        kind: QuantifierKind::Existential,
                        variable: donkey_var,
                        body,
                        island_id: self.current_island,
                    });

                    // Build: ¬∃y(Key(y) ∧ ∃e(Have(e) ∧ ...))
                    let negated_existential = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand: existential,
                    });

                    // Return the complete wide-scope structure directly
                    return Ok(negated_existential);
                }

                // === NARROW SCOPE MODE ===
                // Push binding for later processing (normal donkey binding flow)
                self.donkey_bindings.push((noun, donkey_var, false, false));

                extra_conditions.push(self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: noun,
                    args: self.ctx.terms.alloc_slice([Term::Variable(donkey_var)]),
                    world: None,
                }));

                args.push(Term::Variable(donkey_var));
            } else {
                let object = self.parse_noun_phrase(false)?;

                if self.check(&TokenType::That) || self.check(&TokenType::Who) {
                    self.advance();
                    let nested_var = self.next_var_name();
                    let nested_rel = self.parse_relative_clause(nested_var)?;

                    extra_conditions.push(self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: object.noun,
                        args: self.ctx.terms.alloc_slice([Term::Variable(nested_var)]),
                        world: None,
                    }));
                    extra_conditions.push(nested_rel);
                    args.push(Term::Variable(nested_var));
                } else {
                    args.push(Term::Constant(object.noun));
                }
            }
        }

        while self.check_preposition() {
            self.advance();
            if self.check(&TokenType::Reflexive) {
                self.advance();
                args.push(Term::Variable(var_name));
            } else if self.check_content_word() || self.check_article() {
                let object = self.parse_noun_phrase(false)?;

                if self.check(&TokenType::That) || self.check(&TokenType::Who) {
                    self.advance();
                    let nested_var = self.next_var_name();
                    let nested_rel = self.parse_relative_clause(nested_var)?;
                    extra_conditions.push(self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: object.noun,
                        args: self.ctx.terms.alloc_slice([Term::Variable(nested_var)]),
                        world: None,
                    }));
                    extra_conditions.push(nested_rel);
                    args.push(Term::Variable(nested_var));
                } else {
                    args.push(Term::Constant(object.noun));
                }
            }
        }

        // Use the canonical verb determined at top of function
        // Extract object term from args if present (args[0] is subject, args[1] is object)
        let obj_term = if args.len() > 1 {
            Some(args.remove(1))
        } else {
            None
        };
        let final_modifiers = self.collect_adverbs();
        let base_pred = self.build_verb_neo_event(canonical_verb, var_name, obj_term, final_modifiers);

        // Wrap in negation only for NARROW scope mode (de re reading)
        // Wide scope mode: negation handled via donkey binding flag in wrap_donkey_in_restriction
        // - Narrow: ∃y(Key(y) ∧ ¬Have(x,y)) - "missing ANY key"
        // - Wide:   ¬∃y(Key(y) ∧ Have(x,y)) - "has NO keys"
        let verb_pred = if is_negative && self.negative_scope_mode == NegativeScopeMode::Narrow {
            self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: base_pred,
            })
        } else {
            base_pred
        };

        if extra_conditions.is_empty() {
            Ok(verb_pred)
        } else {
            extra_conditions.push(verb_pred);
            self.combine_with_and(extra_conditions)
        }
    }

    fn combine_with_and(&self, mut exprs: Vec<&'a LogicExpr<'a>>) -> ParseResult<&'a LogicExpr<'a>> {
        if exprs.is_empty() {
            return Err(ParseError {
                kind: ParseErrorKind::EmptyRestriction,
                span: self.current_span(),
            });
        }
        if exprs.len() == 1 {
            return Ok(exprs.remove(0));
        }
        let mut root = exprs.remove(0);
        for expr in exprs {
            root = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: root,
                op: TokenType::And,
                right: expr,
            });
        }
        Ok(root)
    }

    fn wrap_with_definiteness_full(
        &mut self,
        np: &NounPhrase<'a>,
        predicate: &'a LogicExpr<'a>,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        let result = self.wrap_with_definiteness_and_adjectives_and_pps(
            np.definiteness,
            np.noun,
            np.adjectives,
            np.pps,
            predicate,
        )?;

        // If NP has a superlative, add the superlative constraint
        if let Some(adj) = np.superlative {
            let superlative_expr = self.ctx.exprs.alloc(LogicExpr::Superlative {
                adjective: adj,
                subject: self.ctx.terms.alloc(Term::Constant(np.noun)),
                domain: np.noun,
            });
            Ok(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: result,
                op: TokenType::And,
                right: superlative_expr,
            }))
        } else {
            Ok(result)
        }
    }

    fn wrap_with_definiteness(
        &mut self,
        definiteness: Option<Definiteness>,
        noun: Symbol,
        predicate: &'a LogicExpr<'a>,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        self.wrap_with_definiteness_and_adjectives_and_pps(definiteness, noun, &[], &[], predicate)
    }

    fn wrap_with_definiteness_and_adjectives(
        &mut self,
        definiteness: Option<Definiteness>,
        noun: Symbol,
        adjectives: &[Symbol],
        predicate: &'a LogicExpr<'a>,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        self.wrap_with_definiteness_and_adjectives_and_pps(
            definiteness,
            noun,
            adjectives,
            &[],
            predicate,
        )
    }

    fn wrap_with_definiteness_and_adjectives_and_pps(
        &mut self,
        definiteness: Option<Definiteness>,
        noun: Symbol,
        adjectives: &[Symbol],
        pps: &[&'a LogicExpr<'a>],
        predicate: &'a LogicExpr<'a>,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        match definiteness {
            Some(Definiteness::Indefinite) => {
                let var = self.next_var_name();

                // Introduce referent into DRS for cross-sentence anaphora
                // If inside a "No" quantifier, mark as NegationScope (inaccessible)
                let gender = Self::infer_noun_gender(self.interner.resolve(noun));
                let number = if Self::is_plural_noun(self.interner.resolve(noun)) {
                    Number::Plural
                } else {
                    Number::Singular
                };
                if self.in_negative_quantifier {
                    self.drs.introduce_referent_with_source(var, noun, gender, number, ReferentSource::NegationScope);
                } else {
                    self.drs.introduce_referent(var, noun, gender, number);
                }

                let mut restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: noun,
                    args: self.ctx.terms.alloc_slice([Term::Variable(var)]),
                    world: None,
                });

                for adj in adjectives {
                    let adj_str = self.interner.resolve(*adj).to_lowercase();
                    let adj_pred = if is_subsective(&adj_str) {
                        self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: *adj,
                            args: self.ctx.terms.alloc_slice([
                                Term::Variable(var),
                                Term::Intension(noun),
                            ]),
                            world: None,
                        })
                    } else {
                        self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: *adj,
                            args: self.ctx.terms.alloc_slice([Term::Variable(var)]),
                            world: None,
                        })
                    };
                    restriction = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: restriction,
                        op: TokenType::And,
                        right: adj_pred,
                    });
                }

                for pp in pps {
                    let substituted_pp = self.substitute_pp_placeholder(pp, var);
                    restriction = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: restriction,
                        op: TokenType::And,
                        right: substituted_pp,
                    });
                }

                let substituted = self.substitute_constant_with_var_sym(predicate, noun, var)?;
                let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: restriction,
                    op: TokenType::And,
                    right: substituted,
                });
                Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: QuantifierKind::Existential,
                    variable: var,
                    body,
                    island_id: self.current_island,
                }))
            }
            Some(Definiteness::Definite) => {
                let noun_str = self.interner.resolve(noun).to_string();

                if Self::is_plural_noun(&noun_str) {
                    let singular = Self::singularize_noun(&noun_str);
                    let singular_sym = self.interner.intern(&singular);
                    let sigma_term = Term::Sigma(singular_sym);

                    let substituted =
                        self.substitute_constant_with_sigma(predicate, noun, sigma_term)?;

                    let verb_name = self.find_main_verb_name(predicate);
                    let is_collective = verb_name
                        .map(|v| {
                            let lemma = self.interner.resolve(v);
                            Lexer::is_collective_verb(lemma)
                                || (Lexer::is_mixed_verb(lemma) && self.collective_mode)
                        })
                        .unwrap_or(false);

                    // Introduce definite plural referent to DRS for cross-sentence pronoun resolution
                    // E.g., "The dogs ran. They barked." - "they" refers to "dogs"
                    // Definite descriptions presuppose existence, so they should be globally accessible.
                    let gender = Gender::Unknown;  // Plural entities have unknown gender
                    self.drs.introduce_referent_with_source(singular_sym, singular_sym, gender, Number::Plural, ReferentSource::MainClause);

                    if is_collective {
                        Ok(substituted)
                    } else {
                        Ok(self.ctx.exprs.alloc(LogicExpr::Distributive {
                            predicate: substituted,
                        }))
                    }
                } else {
                    let x = self.next_var_name();
                    let y = self.next_var_name();

                    let mut restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: noun,
                        args: self.ctx.terms.alloc_slice([Term::Variable(x)]),
                        world: None,
                    });

                    for adj in adjectives {
                        let adj_str = self.interner.resolve(*adj).to_lowercase();
                        let adj_pred = if is_subsective(&adj_str) {
                            self.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: *adj,
                                args: self.ctx.terms.alloc_slice([
                                    Term::Variable(x),
                                    Term::Intension(noun),
                                ]),
                                world: None,
                            })
                        } else {
                            self.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: *adj,
                                args: self.ctx.terms.alloc_slice([Term::Variable(x)]),
                                world: None,
                            })
                        };
                        restriction = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: restriction,
                            op: TokenType::And,
                            right: adj_pred,
                        });
                    }

                    for pp in pps {
                        let substituted_pp = self.substitute_pp_placeholder(pp, x);
                        restriction = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: restriction,
                            op: TokenType::And,
                            right: substituted_pp,
                        });
                    }

                    // Bridging anaphora: check if this noun is a part of a previously mentioned whole
                    // E.g., "I bought a car. The engine smoked." - engine is part of car
                    let has_prior_antecedent = self.drs.resolve_definite(
                        self.drs.current_box_index(),
                        noun
                    ).is_some();

                    if !has_prior_antecedent {
                        if let Some((whole_var, _whole_name)) = self.drs.resolve_bridging(self.interner, noun) {
                            let part_of_sym = self.interner.intern("PartOf");
                            let part_of_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: part_of_sym,
                                args: self.ctx.terms.alloc_slice([
                                    Term::Variable(x),
                                    Term::Constant(whole_var),
                                ]),
                                world: None,
                            });
                            restriction = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                left: restriction,
                                op: TokenType::And,
                                right: part_of_pred,
                            });
                        }
                    }

                    // Introduce definite referent to DRS for cross-sentence pronoun resolution
                    // E.g., "The engine smoked. It broke." - "it" refers to "engine"
                    // Definite descriptions presuppose existence, so they should be globally
                    // accessible even when introduced inside conditional antecedents.
                    let gender = Self::infer_noun_gender(self.interner.resolve(noun));
                    let number = if Self::is_plural_noun(self.interner.resolve(noun)) {
                        Number::Plural
                    } else {
                        Number::Singular
                    };
                    self.drs.introduce_referent_with_source(x, noun, gender, number, ReferentSource::MainClause);

                    let mut y_restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: noun,
                        args: self.ctx.terms.alloc_slice([Term::Variable(y)]),
                        world: None,
                    });
                    for adj in adjectives {
                        let adj_str = self.interner.resolve(*adj).to_lowercase();
                        let adj_pred = if is_subsective(&adj_str) {
                            self.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: *adj,
                                args: self.ctx.terms.alloc_slice([
                                    Term::Variable(y),
                                    Term::Intension(noun),
                                ]),
                                world: None,
                            })
                        } else {
                            self.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: *adj,
                                args: self.ctx.terms.alloc_slice([Term::Variable(y)]),
                                world: None,
                            })
                        };
                        y_restriction = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: y_restriction,
                            op: TokenType::And,
                            right: adj_pred,
                        });
                    }

                    for pp in pps {
                        let substituted_pp = self.substitute_pp_placeholder(pp, y);
                        y_restriction = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: y_restriction,
                            op: TokenType::And,
                            right: substituted_pp,
                        });
                    }

                    let identity = self.ctx.exprs.alloc(LogicExpr::Identity {
                        left: self.ctx.terms.alloc(Term::Variable(y)),
                        right: self.ctx.terms.alloc(Term::Variable(x)),
                    });
                    let uniqueness_body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: y_restriction,
                        op: TokenType::If,
                        right: identity,
                    });
                    let uniqueness = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                        kind: QuantifierKind::Universal,
                        variable: y,
                        body: uniqueness_body,
                        island_id: self.current_island,
                    });

                    let main_pred = self.substitute_constant_with_var_sym(predicate, noun, x)?;

                    let inner = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: restriction,
                        op: TokenType::And,
                        right: uniqueness,
                    });
                    let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: inner,
                        op: TokenType::And,
                        right: main_pred,
                    });

                    Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                        kind: QuantifierKind::Existential,
                        variable: x,
                        body,
                        island_id: self.current_island,
                    }))
                }
            }
            Some(Definiteness::Proximal) | Some(Definiteness::Distal) => {
                let var = self.next_var_name();

                let mut restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: noun,
                    args: self.ctx.terms.alloc_slice([Term::Variable(var)]),
                    world: None,
                });

                let deictic_name = if matches!(definiteness, Some(Definiteness::Proximal)) {
                    self.interner.intern("Proximal")
                } else {
                    self.interner.intern("Distal")
                };
                let deictic_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: deictic_name,
                    args: self.ctx.terms.alloc_slice([Term::Variable(var)]),
                    world: None,
                });
                restriction = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: restriction,
                    op: TokenType::And,
                    right: deictic_pred,
                });

                for adj in adjectives {
                    let adj_str = self.interner.resolve(*adj).to_lowercase();
                    let adj_pred = if is_subsective(&adj_str) {
                        self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: *adj,
                            args: self.ctx.terms.alloc_slice([
                                Term::Variable(var),
                                Term::Intension(noun),
                            ]),
                            world: None,
                        })
                    } else {
                        self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: *adj,
                            args: self.ctx.terms.alloc_slice([Term::Variable(var)]),
                            world: None,
                        })
                    };
                    restriction = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: restriction,
                        op: TokenType::And,
                        right: adj_pred,
                    });
                }

                for pp in pps {
                    let substituted_pp = self.substitute_pp_placeholder(pp, var);
                    restriction = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: restriction,
                        op: TokenType::And,
                        right: substituted_pp,
                    });
                }

                let substituted = self.substitute_constant_with_var_sym(predicate, noun, var)?;
                let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: restriction,
                    op: TokenType::And,
                    right: substituted,
                });
                Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: QuantifierKind::Existential,
                    variable: var,
                    body,
                    island_id: self.current_island,
                }))
            }
            None => Ok(predicate),
        }
    }

    fn wrap_with_definiteness_for_object(
        &mut self,
        definiteness: Option<Definiteness>,
        noun: Symbol,
        predicate: &'a LogicExpr<'a>,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        match definiteness {
            Some(Definiteness::Indefinite) => {
                let var = self.next_var_name();

                // Introduce referent into DRS for cross-sentence anaphora
                // If inside a "No" quantifier, mark as NegationScope (inaccessible)
                let gender = Self::infer_noun_gender(self.interner.resolve(noun));
                let number = if Self::is_plural_noun(self.interner.resolve(noun)) {
                    Number::Plural
                } else {
                    Number::Singular
                };
                if self.in_negative_quantifier {
                    self.drs.introduce_referent_with_source(var, noun, gender, number, ReferentSource::NegationScope);
                } else {
                    self.drs.introduce_referent(var, noun, gender, number);
                }

                let type_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: noun,
                    args: self.ctx.terms.alloc_slice([Term::Variable(var)]),
                    world: None,
                });
                let substituted = self.substitute_constant_with_var(predicate, noun, var)?;
                let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: type_pred,
                    op: TokenType::And,
                    right: substituted,
                });
                Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: QuantifierKind::Existential,
                    variable: var,
                    body,
                    island_id: self.current_island,
                }))
            }
            Some(Definiteness::Definite) => {
                let x = self.next_var_name();
                let y = self.next_var_name();

                let type_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: noun,
                    args: self.ctx.terms.alloc_slice([Term::Variable(x)]),
                    world: None,
                });

                let identity = self.ctx.exprs.alloc(LogicExpr::Identity {
                    left: self.ctx.terms.alloc(Term::Variable(y)),
                    right: self.ctx.terms.alloc(Term::Variable(x)),
                });
                let inner_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: noun,
                    args: self.ctx.terms.alloc_slice([Term::Variable(y)]),
                    world: None,
                });
                let uniqueness_body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: inner_pred,
                    op: TokenType::If,
                    right: identity,
                });
                let uniqueness = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: QuantifierKind::Universal,
                    variable: y,
                    body: uniqueness_body,
                    island_id: self.current_island,
                });

                let main_pred = self.substitute_constant_with_var(predicate, noun, x)?;

                let type_and_unique = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: type_pred,
                    op: TokenType::And,
                    right: uniqueness,
                });
                let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: type_and_unique,
                    op: TokenType::And,
                    right: main_pred,
                });

                Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: QuantifierKind::Existential,
                    variable: x,
                    body,
                    island_id: self.current_island,
                }))
            }
            Some(Definiteness::Proximal) | Some(Definiteness::Distal) => {
                let var = self.next_var_name();

                let mut restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: noun,
                    args: self.ctx.terms.alloc_slice([Term::Variable(var)]),
                    world: None,
                });

                let deictic_name = if matches!(definiteness, Some(Definiteness::Proximal)) {
                    self.interner.intern("Proximal")
                } else {
                    self.interner.intern("Distal")
                };
                let deictic_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: deictic_name,
                    args: self.ctx.terms.alloc_slice([Term::Variable(var)]),
                    world: None,
                });
                restriction = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: restriction,
                    op: TokenType::And,
                    right: deictic_pred,
                });

                let substituted = self.substitute_constant_with_var(predicate, noun, var)?;
                let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: restriction,
                    op: TokenType::And,
                    right: substituted,
                });
                Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: QuantifierKind::Existential,
                    variable: var,
                    body,
                    island_id: self.current_island,
                }))
            }
            None => Ok(predicate),
        }
    }

    fn substitute_pp_placeholder(&mut self, pp: &'a LogicExpr<'a>, var: Symbol) -> &'a LogicExpr<'a> {
        let placeholder = self.interner.intern("_PP_SELF_");
        match pp {
            LogicExpr::Predicate { name, args, .. } => {
                let new_args: Vec<Term<'a>> = args
                    .iter()
                    .map(|arg| match arg {
                        Term::Variable(v) if *v == placeholder => Term::Variable(var),
                        other => *other,
                    })
                    .collect();
                self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: *name,
                    args: self.ctx.terms.alloc_slice(new_args),
                    world: None,
                })
            }
            _ => pp,
        }
    }

    fn substitute_constant_with_var(
        &self,
        expr: &'a LogicExpr<'a>,
        constant_name: Symbol,
        var_name: Symbol,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        match expr {
            LogicExpr::Predicate { name, args, .. } => {
                let new_args: Vec<Term<'a>> = args
                    .iter()
                    .map(|arg| match arg {
                        Term::Constant(c) if *c == constant_name => Term::Variable(var_name),
                        Term::Constant(c) => Term::Constant(*c),
                        Term::Variable(v) => Term::Variable(*v),
                        Term::Function(n, a) => Term::Function(*n, *a),
                        Term::Group(m) => Term::Group(*m),
                        Term::Possessed { possessor, possessed } => Term::Possessed {
                            possessor: *possessor,
                            possessed: *possessed,
                        },
                        Term::Sigma(p) => Term::Sigma(*p),
                        Term::Intension(p) => Term::Intension(*p),
                        Term::Proposition(e) => Term::Proposition(*e),
                        Term::Value { kind, unit, dimension } => Term::Value {
                            kind: *kind,
                            unit: *unit,
                            dimension: *dimension,
                        },
                    })
                    .collect();
                Ok(self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: *name,
                    args: self.ctx.terms.alloc_slice(new_args),
                    world: None,
                }))
            }
            LogicExpr::Temporal { operator, body } => Ok(self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: *operator,
                body: self.substitute_constant_with_var(body, constant_name, var_name)?,
            })),
            LogicExpr::Aspectual { operator, body } => Ok(self.ctx.exprs.alloc(LogicExpr::Aspectual {
                operator: *operator,
                body: self.substitute_constant_with_var(body, constant_name, var_name)?,
            })),
            LogicExpr::UnaryOp { op, operand } => Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: op.clone(),
                operand: self.substitute_constant_with_var(operand, constant_name, var_name)?,
            })),
            LogicExpr::BinaryOp { left, op, right } => Ok(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: self.substitute_constant_with_var(left, constant_name, var_name)?,
                op: op.clone(),
                right: self.substitute_constant_with_var(right, constant_name, var_name)?,
            })),
            LogicExpr::Event { predicate, adverbs } => Ok(self.ctx.exprs.alloc(LogicExpr::Event {
                predicate: self.substitute_constant_with_var(predicate, constant_name, var_name)?,
                adverbs: *adverbs,
            })),
            LogicExpr::TemporalAnchor { anchor, body } => {
                Ok(self.ctx.exprs.alloc(LogicExpr::TemporalAnchor {
                    anchor: *anchor,
                    body: self.substitute_constant_with_var(body, constant_name, var_name)?,
                }))
            }
            LogicExpr::NeoEvent(data) => {
                // Substitute constants in thematic roles (Agent, Theme, etc.)
                let new_roles: Vec<(crate::ast::ThematicRole, Term<'a>)> = data
                    .roles
                    .iter()
                    .map(|(role, term)| {
                        let new_term = match term {
                            Term::Constant(c) if *c == constant_name => Term::Variable(var_name),
                            Term::Constant(c) => Term::Constant(*c),
                            Term::Variable(v) => Term::Variable(*v),
                            Term::Function(n, a) => Term::Function(*n, *a),
                            Term::Group(m) => Term::Group(*m),
                            Term::Possessed { possessor, possessed } => Term::Possessed {
                                possessor: *possessor,
                                possessed: *possessed,
                            },
                            Term::Sigma(p) => Term::Sigma(*p),
                            Term::Intension(p) => Term::Intension(*p),
                            Term::Proposition(e) => Term::Proposition(*e),
                            Term::Value { kind, unit, dimension } => Term::Value {
                                kind: *kind,
                                unit: *unit,
                                dimension: *dimension,
                            },
                        };
                        (*role, new_term)
                    })
                    .collect();
                Ok(self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(crate::ast::NeoEventData {
                    event_var: data.event_var,
                    verb: data.verb,
                    roles: self.ctx.roles.alloc_slice(new_roles),
                    modifiers: data.modifiers,
                    suppress_existential: data.suppress_existential,
                    world: None,
                }))))
            }
            // Recurse into nested quantifiers to substitute constants in their bodies
            LogicExpr::Quantifier { kind, variable, body, island_id } => {
                let new_body = self.substitute_constant_with_var(body, constant_name, var_name)?;
                Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: *kind,
                    variable: *variable,
                    body: new_body,
                    island_id: *island_id,
                }))
            }
            _ => Ok(expr),
        }
    }

    fn substitute_constant_with_var_sym(
        &self,
        expr: &'a LogicExpr<'a>,
        constant_name: Symbol,
        var_name: Symbol,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        self.substitute_constant_with_var(expr, constant_name, var_name)
    }

    fn substitute_constant_with_sigma(
        &self,
        expr: &'a LogicExpr<'a>,
        constant_name: Symbol,
        sigma_term: Term<'a>,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        match expr {
            LogicExpr::Predicate { name, args, .. } => {
                let new_args: Vec<Term<'a>> = args
                    .iter()
                    .map(|arg| match arg {
                        Term::Constant(c) if *c == constant_name => sigma_term.clone(),
                        Term::Constant(c) => Term::Constant(*c),
                        Term::Variable(v) => Term::Variable(*v),
                        Term::Function(n, a) => Term::Function(*n, *a),
                        Term::Group(m) => Term::Group(*m),
                        Term::Possessed { possessor, possessed } => Term::Possessed {
                            possessor: *possessor,
                            possessed: *possessed,
                        },
                        Term::Sigma(p) => Term::Sigma(*p),
                        Term::Intension(p) => Term::Intension(*p),
                        Term::Proposition(e) => Term::Proposition(*e),
                        Term::Value { kind, unit, dimension } => Term::Value {
                            kind: *kind,
                            unit: *unit,
                            dimension: *dimension,
                        },
                    })
                    .collect();
                Ok(self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: *name,
                    args: self.ctx.terms.alloc_slice(new_args),
                    world: None,
                }))
            }
            LogicExpr::Temporal { operator, body } => Ok(self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: *operator,
                body: self.substitute_constant_with_sigma(body, constant_name, sigma_term)?,
            })),
            LogicExpr::Aspectual { operator, body } => Ok(self.ctx.exprs.alloc(LogicExpr::Aspectual {
                operator: *operator,
                body: self.substitute_constant_with_sigma(body, constant_name, sigma_term)?,
            })),
            LogicExpr::UnaryOp { op, operand } => Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: op.clone(),
                operand: self.substitute_constant_with_sigma(operand, constant_name, sigma_term)?,
            })),
            LogicExpr::BinaryOp { left, op, right } => Ok(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: self.substitute_constant_with_sigma(
                    left,
                    constant_name,
                    sigma_term.clone(),
                )?,
                op: op.clone(),
                right: self.substitute_constant_with_sigma(right, constant_name, sigma_term)?,
            })),
            LogicExpr::Event { predicate, adverbs } => Ok(self.ctx.exprs.alloc(LogicExpr::Event {
                predicate: self.substitute_constant_with_sigma(
                    predicate,
                    constant_name,
                    sigma_term,
                )?,
                adverbs: *adverbs,
            })),
            LogicExpr::TemporalAnchor { anchor, body } => {
                Ok(self.ctx.exprs.alloc(LogicExpr::TemporalAnchor {
                    anchor: *anchor,
                    body: self.substitute_constant_with_sigma(body, constant_name, sigma_term)?,
                }))
            }
            LogicExpr::NeoEvent(data) => {
                let new_roles: Vec<(crate::ast::ThematicRole, Term<'a>)> = data
                    .roles
                    .iter()
                    .map(|(role, term)| {
                        let new_term = match term {
                            Term::Constant(c) if *c == constant_name => sigma_term.clone(),
                            Term::Constant(c) => Term::Constant(*c),
                            Term::Variable(v) => Term::Variable(*v),
                            Term::Function(n, a) => Term::Function(*n, *a),
                            Term::Group(m) => Term::Group(*m),
                            Term::Possessed { possessor, possessed } => Term::Possessed {
                                possessor: *possessor,
                                possessed: *possessed,
                            },
                            Term::Sigma(p) => Term::Sigma(*p),
                            Term::Intension(p) => Term::Intension(*p),
                            Term::Proposition(e) => Term::Proposition(*e),
                            Term::Value { kind, unit, dimension } => Term::Value {
                                kind: *kind,
                                unit: *unit,
                                dimension: *dimension,
                            },
                        };
                        (*role, new_term)
                    })
                    .collect();
                Ok(self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(crate::ast::NeoEventData {
                    event_var: data.event_var,
                    verb: data.verb,
                    roles: self.ctx.roles.alloc_slice(new_roles),
                    modifiers: data.modifiers,
                    suppress_existential: data.suppress_existential,
                    world: None,
                }))))
            }
            LogicExpr::Distributive { predicate } => Ok(self.ctx.exprs.alloc(LogicExpr::Distributive {
                predicate: self.substitute_constant_with_sigma(predicate, constant_name, sigma_term)?,
            })),
            _ => Ok(expr),
        }
    }

    fn find_main_verb_name(&self, expr: &LogicExpr<'a>) -> Option<Symbol> {
        match expr {
            LogicExpr::Predicate { name, .. } => Some(*name),
            LogicExpr::NeoEvent(data) => Some(data.verb),
            LogicExpr::Temporal { body, .. } => self.find_main_verb_name(body),
            LogicExpr::Aspectual { body, .. } => self.find_main_verb_name(body),
            LogicExpr::Event { predicate, .. } => self.find_main_verb_name(predicate),
            LogicExpr::TemporalAnchor { body, .. } => self.find_main_verb_name(body),
            LogicExpr::UnaryOp { operand, .. } => self.find_main_verb_name(operand),
            LogicExpr::BinaryOp { left, .. } => self.find_main_verb_name(left),
            _ => None,
        }
    }

    fn transform_cardinal_to_group(&mut self, expr: &'a LogicExpr<'a>) -> ParseResult<&'a LogicExpr<'a>> {
        match expr {
            LogicExpr::Quantifier { kind: QuantifierKind::Cardinal(n), variable, body, .. } => {
                let group_var = self.interner.intern("g");
                let member_var = *variable;

                // Extract the restriction (first conjunct) and the body (rest)
                // The structure is: restriction ∧ body_rest
                let (restriction, body_rest) = match body {
                    LogicExpr::BinaryOp { left, op: TokenType::And, right } => (*left, *right),
                    _ => return Ok(expr),
                };

                // Substitute the member variable with the group variable in the body
                let transformed_body = self.substitute_constant_with_var_sym(body_rest, member_var, group_var)?;

                Ok(self.ctx.exprs.alloc(LogicExpr::GroupQuantifier {
                    group_var,
                    count: *n,
                    member_var,
                    restriction,
                    body: transformed_body,
                }))
            }
            // Recursively transform nested expressions
            LogicExpr::Temporal { operator, body } => {
                let transformed = self.transform_cardinal_to_group(body)?;
                Ok(self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: *operator,
                    body: transformed,
                }))
            }
            LogicExpr::Aspectual { operator, body } => {
                let transformed = self.transform_cardinal_to_group(body)?;
                Ok(self.ctx.exprs.alloc(LogicExpr::Aspectual {
                    operator: *operator,
                    body: transformed,
                }))
            }
            LogicExpr::UnaryOp { op, operand } => {
                let transformed = self.transform_cardinal_to_group(operand)?;
                Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                    op: op.clone(),
                    operand: transformed,
                }))
            }
            LogicExpr::BinaryOp { left, op, right } => {
                let transformed_left = self.transform_cardinal_to_group(left)?;
                let transformed_right = self.transform_cardinal_to_group(right)?;
                Ok(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: transformed_left,
                    op: op.clone(),
                    right: transformed_right,
                }))
            }
            LogicExpr::Distributive { predicate } => {
                let transformed = self.transform_cardinal_to_group(predicate)?;
                Ok(self.ctx.exprs.alloc(LogicExpr::Distributive {
                    predicate: transformed,
                }))
            }
            LogicExpr::Quantifier { kind, variable, body, island_id } => {
                let transformed = self.transform_cardinal_to_group(body)?;
                Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: kind.clone(),
                    variable: *variable,
                    body: transformed,
                    island_id: *island_id,
                }))
            }
            _ => Ok(expr),
        }
    }

    fn build_verb_neo_event(
        &mut self,
        verb: Symbol,
        subject_var: Symbol,
        object: Option<Term<'a>>,
        modifiers: Vec<Symbol>,
    ) -> &'a LogicExpr<'a> {
        let event_var = self.get_event_var();

        // Check if verb is unaccusative (intransitive subject is Theme, not Agent)
        let verb_str = self.interner.resolve(verb).to_lowercase();
        let is_unaccusative = lookup_verb_db(&verb_str)
            .map(|meta| meta.features.contains(&Feature::Unaccusative))
            .unwrap_or(false);

        // Determine subject role: unaccusative verbs without object use Theme
        let has_object = object.is_some();
        let subject_role = if is_unaccusative && !has_object {
            ThematicRole::Theme
        } else {
            ThematicRole::Agent
        };

        // Build roles vector
        let mut roles = vec![(subject_role, Term::Variable(subject_var))];
        if let Some(obj_term) = object {
            roles.push((ThematicRole::Theme, obj_term));
        }

        // Create NeoEventData with suppress_existential: false
        // Each quantified individual gets their own event (distributive reading)
        self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
            event_var,
            verb,
            roles: self.ctx.roles.alloc_slice(roles),
            modifiers: self.ctx.syms.alloc_slice(modifiers),
            suppress_existential: false,
            world: None,
        })))
    }
}

// Helper methods for donkey binding scope handling
impl<'a, 'ctx, 'int> Parser<'a, 'ctx, 'int> {
    /// Check if an expression mentions a specific variable
    fn expr_mentions_var(&self, expr: &LogicExpr<'a>, var: Symbol) -> bool {
        match expr {
            LogicExpr::Predicate { args, .. } => {
                args.iter().any(|term| self.term_mentions_var(term, var))
            }
            LogicExpr::BinaryOp { left, right, .. } => {
                self.expr_mentions_var(left, var) || self.expr_mentions_var(right, var)
            }
            LogicExpr::UnaryOp { operand, .. } => self.expr_mentions_var(operand, var),
            LogicExpr::Quantifier { body, .. } => self.expr_mentions_var(body, var),
            LogicExpr::NeoEvent(data) => {
                data.roles.iter().any(|(_, term)| self.term_mentions_var(term, var))
            }
            LogicExpr::Temporal { body, .. } => self.expr_mentions_var(body, var),
            LogicExpr::Aspectual { body, .. } => self.expr_mentions_var(body, var),
            LogicExpr::Event { predicate, .. } => self.expr_mentions_var(predicate, var),
            LogicExpr::Modal { operand, .. } => self.expr_mentions_var(operand, var),
            LogicExpr::Scopal { body, .. } => self.expr_mentions_var(body, var),
            _ => false,
        }
    }

    fn term_mentions_var(&self, term: &Term<'a>, var: Symbol) -> bool {
        match term {
            Term::Variable(v) => *v == var,
            Term::Function(_, args) => args.iter().any(|t| self.term_mentions_var(t, var)),
            _ => false,
        }
    }

    /// Collect all conjuncts from a conjunction tree
    fn collect_conjuncts(&self, expr: &'a LogicExpr<'a>) -> Vec<&'a LogicExpr<'a>> {
        match expr {
            LogicExpr::BinaryOp { left, op: TokenType::And, right } => {
                let mut result = self.collect_conjuncts(left);
                result.extend(self.collect_conjuncts(right));
                result
            }
            _ => vec![expr],
        }
    }

    /// Wrap unused donkey bindings inside the restriction/body of a quantifier structure.
    ///
    /// For universals (implications):
    ///   Transform: ∀x((P(x) ∧ Q(y)) → R(x)) with unused y
    ///   Into:      ∀x((P(x) ∧ ∃y(Q(y))) → R(x))
    ///
    /// For existentials (conjunctions):
    ///   Transform: ∃x(P(x) ∧ Q(y) ∧ R(x)) with unused y
    ///   Into:      ∃x(P(x) ∧ ∃y(Q(y)) ∧ R(x))
    ///
    /// If wide_scope_negation is true, wrap the existential in negation:
    ///   Into:      ∀x((P(x) ∧ ¬∃y(Q(y))) → R(x))
    fn wrap_donkey_in_restriction(
        &self,
        body: &'a LogicExpr<'a>,
        donkey_var: Symbol,
        wide_scope_negation: bool,
    ) -> &'a LogicExpr<'a> {
        // Handle Quantifier wrapping first
        if let LogicExpr::Quantifier { kind, variable, body: inner_body, island_id } = body {
            let transformed = self.wrap_donkey_in_restriction(inner_body, donkey_var, wide_scope_negation);
            return self.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind: kind.clone(),
                variable: *variable,
                body: transformed,
                island_id: *island_id,
            });
        }

        // Handle implication (universal quantifiers)
        if let LogicExpr::BinaryOp { left, op: TokenType::If, right } = body {
            return self.wrap_in_implication(*left, *right, donkey_var, wide_scope_negation);
        }

        // Handle conjunction (existential quantifiers)
        if let LogicExpr::BinaryOp { left: _, op: TokenType::And, right: _ } = body {
            return self.wrap_in_conjunction(body, donkey_var, wide_scope_negation);
        }

        // Not a structure we can process
        body
    }

    /// Wrap donkey binding in an implication structure (∀x(P(x) → Q(x)))
    fn wrap_in_implication(
        &self,
        restriction: &'a LogicExpr<'a>,
        consequent: &'a LogicExpr<'a>,
        donkey_var: Symbol,
        wide_scope_negation: bool,
    ) -> &'a LogicExpr<'a> {
        // Collect all conjuncts in the restriction
        let conjuncts = self.collect_conjuncts(restriction);

        // Partition into those mentioning the donkey var and those not
        let (with_var, without_var): (Vec<_>, Vec<_>) = conjuncts
            .into_iter()
            .partition(|c| self.expr_mentions_var(c, donkey_var));

        if with_var.is_empty() {
            // Variable not found in restriction, return original implication
            return self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: restriction,
                op: TokenType::If,
                right: consequent,
            });
        }

        // Combine the "with var" conjuncts
        let with_var_combined = self.combine_conjuncts(&with_var);

        // Wrap with existential
        let existential = self.ctx.exprs.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Existential,
            variable: donkey_var,
            body: with_var_combined,
            island_id: self.current_island,
        });

        // For wide scope negation (de dicto reading of "lacks"), wrap ∃ in ¬
        let wrapped = if wide_scope_negation {
            self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: existential,
            })
        } else {
            existential
        };

        // Combine with "without var" conjuncts
        let new_restriction = if without_var.is_empty() {
            wrapped
        } else {
            let without_combined = self.combine_conjuncts(&without_var);
            self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: without_combined,
                op: TokenType::And,
                right: wrapped,
            })
        };

        // Rebuild the implication
        self.ctx.exprs.alloc(LogicExpr::BinaryOp {
            left: new_restriction,
            op: TokenType::If,
            right: consequent,
        })
    }

    /// Wrap donkey binding in a conjunction structure (∃x(P(x) ∧ Q(x)))
    fn wrap_in_conjunction(
        &self,
        body: &'a LogicExpr<'a>,
        donkey_var: Symbol,
        wide_scope_negation: bool,
    ) -> &'a LogicExpr<'a> {
        // Collect all conjuncts
        let conjuncts = self.collect_conjuncts(body);

        // Partition into those mentioning the donkey var and those not
        let (with_var, without_var): (Vec<_>, Vec<_>) = conjuncts
            .into_iter()
            .partition(|c| self.expr_mentions_var(c, donkey_var));

        if with_var.is_empty() {
            // Variable not found, return unchanged
            return body;
        }

        // Combine the "with var" conjuncts
        let with_var_combined = self.combine_conjuncts(&with_var);

        // Wrap with existential
        let existential = self.ctx.exprs.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Existential,
            variable: donkey_var,
            body: with_var_combined,
            island_id: self.current_island,
        });

        // For wide scope negation (de dicto reading of "lacks"), wrap ∃ in ¬
        let wrapped = if wide_scope_negation {
            self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: existential,
            })
        } else {
            existential
        };

        // Combine with "without var" conjuncts
        if without_var.is_empty() {
            wrapped
        } else {
            let without_combined = self.combine_conjuncts(&without_var);
            self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: without_combined,
                op: TokenType::And,
                right: wrapped,
            })
        }
    }

    fn combine_conjuncts(&self, conjuncts: &[&'a LogicExpr<'a>]) -> &'a LogicExpr<'a> {
        if conjuncts.is_empty() {
            panic!("Cannot combine empty conjuncts");
        }
        if conjuncts.len() == 1 {
            return conjuncts[0];
        }
        let mut result = conjuncts[0];
        for c in &conjuncts[1..] {
            result = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: result,
                op: TokenType::And,
                right: *c,
            });
        }
        result
    }
}
