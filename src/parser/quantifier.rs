use super::clause::ClauseParsing;
use super::modal::ModalParsing;
use super::noun::NounParsing;
use super::{ParseResult, Parser};
use crate::ast::{LogicExpr, NounPhrase, QuantifierKind, Term};
use crate::context::Number;
use crate::error::{ParseError, ParseErrorKind};
use crate::intern::Symbol;
use crate::lexer::Lexer;
use crate::lexicon::{is_subsective, Definiteness, Time};
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
}

impl<'a, 'ctx, 'int> QuantifierParsing<'a, 'ctx, 'int> for Parser<'a, 'ctx, 'int> {
    fn parse_quantified(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        let quantifier_token = self.previous().kind.clone();
        let var_name = self.next_var_name();

        let subject_pred = self.parse_restriction(var_name)?;

        if self.check_modal() {
            use crate::ast::ModalFlavor;

            self.advance();
            let vector = self.token_to_vector(&self.previous().kind.clone());
            let verb = self.consume_content_word()?;

            // Parse object if present (e.g., "can enter the room" -> room is object)
            let verb_args = if self.check_content_word() || self.check_article() {
                let obj_np = self.parse_noun_phrase(false)?;
                let obj_term = self.noun_phrase_to_term(&obj_np);
                self.ctx.terms.alloc_slice([Term::Variable(var_name), obj_term])
            } else {
                self.ctx.terms.alloc_slice([Term::Variable(var_name)])
            };

            let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: verb_args,
            });

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

                // Return quantifier directly (modal is inside)
                return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind,
                    variable: var_name,
                    body,
                    island_id: self.current_island,
                }));

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

                let quantified = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind,
                    variable: var_name,
                    body,
                    island_id: self.current_island,
                });

                // Wrap the entire quantifier in the modal
                return Ok(self.ctx.exprs.alloc(LogicExpr::Modal {
                    vector,
                    operand: quantified,
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
                let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: verb,
                    args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
                });

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
                self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: verb,
                    args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
                })
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
                        let unknown = self.interner.intern("?");
                        let resolved = self
                            .resolve_pronoun(gender, Number::Singular)
                            .unwrap_or(unknown);
                        args.push(Term::Constant(resolved));
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
                });

                let verb_with_obj = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: verb,
                    args: self.ctx.terms.alloc_slice([
                        Term::Variable(var_name),
                        Term::Variable(obj_var),
                    ]),
                });

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
                    let obj_restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: object.noun,
                        args: self.ctx.terms.alloc_slice([Term::Variable(obj_var)]),
                    });

                    let verb_with_obj = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: verb,
                        args: self.ctx.terms.alloc_slice([
                            Term::Variable(var_name),
                            Term::Variable(obj_var),
                        ]),
                    });

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

            let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice(args),
            });

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

            for (_noun, donkey_var, used) in self.donkey_bindings.iter().rev() {
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
                    result = self.wrap_donkey_in_restriction(result, *donkey_var);
                }
            }
            self.donkey_bindings.clear();

            return Ok(result);
        }

        self.consume_copula()?;

        let negative = self.match_token(&[TokenType::Not]);
        let predicate_np = self.parse_noun_phrase(true)?;

        let predicate_expr = self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: predicate_np.noun,
            args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
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

        for (_noun, donkey_var, used) in self.donkey_bindings.iter().rev() {
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
                result = self.wrap_donkey_in_restriction(result, *donkey_var);
            }
        }
        self.donkey_bindings.clear();

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
        let verb_str = self.interner.resolve(verb);

        if Lexer::is_raising_verb(verb_str) && self.check_to() {
            self.advance();
            if self.check_verb() {
                let inf_verb = self.consume_verb();
                let inf_verb_str = self.interner.resolve(inf_verb).to_lowercase();

                if inf_verb_str == "be" && self.check_content_word() {
                    let adj = self.consume_content_word()?;
                    let embedded = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: adj,
                        args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
                    });
                    return Ok(self.ctx.exprs.alloc(LogicExpr::Scopal {
                        operator: verb,
                        body: embedded,
                    }));
                }

                let embedded = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: inf_verb,
                    args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
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

                self.donkey_bindings.push((noun, donkey_var, false));

                extra_conditions.push(self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: noun,
                    args: self.ctx.terms.alloc_slice([Term::Variable(donkey_var)]),
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
                    }));
                    extra_conditions.push(nested_rel);
                    args.push(Term::Variable(nested_var));
                } else {
                    args.push(Term::Constant(object.noun));
                }
            }
        }

        let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: verb,
            args: self.ctx.terms.alloc_slice(args),
        });

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

                let mut restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: noun,
                    args: self.ctx.terms.alloc_slice([Term::Variable(var)]),
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
                        })
                    } else {
                        self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: *adj,
                            args: self.ctx.terms.alloc_slice([Term::Variable(var)]),
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
                            })
                        } else {
                            self.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: *adj,
                                args: self.ctx.terms.alloc_slice([Term::Variable(x)]),
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
                    if let Some(ref mut ctx) = self.context {
                        let our_symbol = noun_str.chars().next().unwrap().to_uppercase().to_string();
                        let has_prior_antecedent = ctx.resolve_definite(&noun_str)
                            .map(|e| e.symbol != our_symbol)
                            .unwrap_or(false);

                        if !has_prior_antecedent {
                            let bridging_matches = ctx.resolve_bridging(&noun_str);
                            if let Some((entity, _whole_name)) = bridging_matches.first() {
                                let whole_sym = self.interner.intern(&entity.symbol);
                                let part_of_sym = self.interner.intern("PartOf");
                                let part_of_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                                    name: part_of_sym,
                                    args: self.ctx.terms.alloc_slice([
                                        Term::Variable(x),
                                        Term::Constant(whole_sym),
                                    ]),
                                });
                                restriction = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                    left: restriction,
                                    op: TokenType::And,
                                    right: part_of_pred,
                                });
                            }
                        }
                    }

                    let mut y_restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: noun,
                        args: self.ctx.terms.alloc_slice([Term::Variable(y)]),
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
                            })
                        } else {
                            self.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: *adj,
                                args: self.ctx.terms.alloc_slice([Term::Variable(y)]),
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
                });

                let deictic_name = if matches!(definiteness, Some(Definiteness::Proximal)) {
                    self.interner.intern("Proximal")
                } else {
                    self.interner.intern("Distal")
                };
                let deictic_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: deictic_name,
                    args: self.ctx.terms.alloc_slice([Term::Variable(var)]),
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
                        })
                    } else {
                        self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: *adj,
                            args: self.ctx.terms.alloc_slice([Term::Variable(var)]),
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
                let type_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: noun,
                    args: self.ctx.terms.alloc_slice([Term::Variable(var)]),
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
                });

                let identity = self.ctx.exprs.alloc(LogicExpr::Identity {
                    left: self.ctx.terms.alloc(Term::Variable(y)),
                    right: self.ctx.terms.alloc(Term::Variable(x)),
                });
                let inner_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: noun,
                    args: self.ctx.terms.alloc_slice([Term::Variable(y)]),
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
                });

                let deictic_name = if matches!(definiteness, Some(Definiteness::Proximal)) {
                    self.interner.intern("Proximal")
                } else {
                    self.interner.intern("Distal")
                };
                let deictic_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: deictic_name,
                    args: self.ctx.terms.alloc_slice([Term::Variable(var)]),
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
            LogicExpr::Predicate { name, args } => {
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
            LogicExpr::Predicate { name, args } => {
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
            LogicExpr::Predicate { name, args } => {
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

    /// Wrap unused donkey bindings inside the restriction of an implication
    /// Transform: ∀x((P(x) ∧ Q(y)) → R(x)) with unused y
    /// Into:      ∀x((P(x) ∧ ∃y(Q(y))) → R(x))
    fn wrap_donkey_in_restriction(
        &self,
        body: &'a LogicExpr<'a>,
        donkey_var: Symbol,
    ) -> &'a LogicExpr<'a> {
        // Handle Quantifier wrapping first
        if let LogicExpr::Quantifier { kind, variable, body: inner_body, island_id } = body {
            let transformed = self.wrap_donkey_in_restriction(inner_body, donkey_var);
            return self.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind: kind.clone(),
                variable: *variable,
                body: transformed,
                island_id: *island_id,
            });
        }

        // Must be an implication
        let (restriction, consequent) = match body {
            LogicExpr::BinaryOp { left, op: TokenType::If, right } => (*left, *right),
            _ => return body, // Not an implication, return unchanged
        };

        // Collect all conjuncts in the restriction
        let conjuncts = self.collect_conjuncts(restriction);

        // Partition into those mentioning the donkey var and those not
        let (with_var, without_var): (Vec<_>, Vec<_>) = conjuncts
            .into_iter()
            .partition(|c| self.expr_mentions_var(c, donkey_var));

        if with_var.is_empty() {
            // Variable not found in restriction, return unchanged
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

        // Combine with "without var" conjuncts
        let new_restriction = if without_var.is_empty() {
            existential
        } else {
            let without_combined = self.combine_conjuncts(&without_var);
            self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: without_combined,
                op: TokenType::And,
                right: existential,
            })
        };

        // Rebuild the implication
        self.ctx.exprs.alloc(LogicExpr::BinaryOp {
            left: new_restriction,
            op: TokenType::If,
            right: consequent,
        })
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
