use super::noun::NounParsing;
use super::quantifier::QuantifierParsing;
use super::verb::LogicVerbParsing;
use super::{ParseResult, Parser};
use crate::ast::{AspectOperator, LogicExpr, ModalDomain, ModalVector, TemporalOperator, Term};
use crate::lexicon::{Aspect, Time};
use crate::token::TokenType;

pub trait QuestionParsing<'a, 'ctx, 'int> {
    fn parse_wh_question(&mut self) -> ParseResult<&'a LogicExpr<'a>>;
    fn parse_yes_no_question(&mut self) -> ParseResult<&'a LogicExpr<'a>>;
    fn aux_token_to_modal_vector(&self, token: &TokenType) -> ModalVector;
}

impl<'a, 'ctx, 'int> QuestionParsing<'a, 'ctx, 'int> for Parser<'a, 'ctx, 'int> {
    fn parse_wh_question(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        let pied_piping_prep = if self.check_preposition() {
            let prep = self.advance().kind.clone();
            Some(prep)
        } else {
            None
        };

        let wh_token = self.advance().kind.clone();
        let var_name = self.interner.intern("x");
        let var_term = Term::Variable(var_name);

        if pied_piping_prep.is_some() && self.check_auxiliary() {
            let aux_token = self.advance().clone();
            if let TokenType::Auxiliary(time) = aux_token.kind {
                self.pending_time = Some(time);
            }

            let subject = self.parse_noun_phrase(true)?;
            let verb = self.consume_verb();

            let mut args = vec![Term::Constant(subject.noun)];
            if self.check_content_word() || self.check_article() {
                let object = self.parse_noun_phrase(false)?;
                args.push(Term::Constant(object.noun));
            }
            args.push(var_term);

            let body = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice(args),
                world: None,
            });
            return Ok(self.ctx.exprs.alloc(LogicExpr::Question {
                wh_variable: var_name,
                body,
            }));
        }

        if self.check_verb() {
            let verb = self.consume_verb();
            let mut args = vec![var_term];

            if self.check_content_word() {
                let object = self.parse_noun_phrase(false)?;
                args.push(Term::Constant(object.noun));
            }

            let body = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice(args),
                world: None,
            });
            return Ok(self.ctx.exprs.alloc(LogicExpr::Question {
                wh_variable: var_name,
                body,
            }));
        }

        if self.check(&TokenType::Does) || self.check(&TokenType::Do) {
            self.advance();
            let subject = self.parse_noun_phrase(true)?;
            let verb = self.consume_verb();

            let body = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice([Term::Constant(subject.noun), var_term]),
                world: None,
            });
            return Ok(self.ctx.exprs.alloc(LogicExpr::Question {
                wh_variable: var_name,
                body,
            }));
        }

        if self.check_auxiliary() {
            let aux_token = self.advance().clone();
            if let TokenType::Auxiliary(time) = aux_token.kind {
                self.pending_time = Some(time);
            }

            self.filler_gap = Some(var_name);

            let subject = self.parse_noun_phrase(true)?;
            let body = self.parse_predicate_with_subject(subject.noun)?;

            self.filler_gap = None;

            return Ok(self.ctx.exprs.alloc(LogicExpr::Question {
                wh_variable: var_name,
                body,
            }));
        }

        let unknown = self.interner.intern(&format!("{:?}", wh_token));
        Ok(self.ctx.exprs.alloc(LogicExpr::Atom(unknown)))
    }

    fn parse_yes_no_question(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        let aux_token = self.advance().kind.clone();

        let is_modal = matches!(aux_token, TokenType::Can | TokenType::Could | TokenType::Would | TokenType::May | TokenType::Must | TokenType::Should);
        let is_copula = matches!(aux_token, TokenType::Is | TokenType::Are | TokenType::Was | TokenType::Were);
        let copula_time = if matches!(aux_token, TokenType::Was | TokenType::Were) {
            Time::Past
        } else {
            Time::Present
        };

        if self.check_quantifier() {
            self.advance();
            let quantified = self.parse_quantified()?;
            let wrapped = if is_modal {
                let vector = self.aux_token_to_modal_vector(&aux_token);
                self.ctx.exprs.alloc(LogicExpr::Modal {
                    vector,
                    operand: quantified,
                })
            } else {
                quantified
            };
            return Ok(self.ctx.exprs.alloc(LogicExpr::YesNoQuestion { body: wrapped }));
        }

        let subject_symbol = if self.check_pronoun() {
            let token = self.advance().clone();
            if let TokenType::Pronoun { gender, number, .. } = token.kind {
                let token_text = self.interner.resolve(token.lexeme);
                if token_text.eq_ignore_ascii_case("you") {
                    self.interner.intern("Addressee")
                } else {
                    let resolved = self.resolve_pronoun(gender, number)?;
                    match resolved {
                        super::ResolvedPronoun::Variable(s) | super::ResolvedPronoun::Constant(s) => s,
                    }
                }
            } else {
                self.interner.intern("?")
            }
        } else {
            self.parse_noun_phrase(true)?.noun
        };

        let please_sym = self.interner.intern("please");
        self.match_token(&[TokenType::Adverb(please_sym)]);

        if is_copula {
            let body = if self.check_verb() {
                let (verb, _, verb_aspect, _) = self.consume_verb_with_metadata();
                let predicate = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: verb,
                    args: self.ctx.terms.alloc_slice([Term::Constant(subject_symbol)]),
                    world: None,
                });
                let with_aspect = if verb_aspect == Aspect::Progressive {
                    self.ctx.exprs.alloc(LogicExpr::Aspectual {
                        operator: AspectOperator::Progressive,
                        body: predicate,
                    })
                } else {
                    predicate
                };
                if copula_time == Time::Past {
                    self.ctx.exprs.alloc(LogicExpr::Temporal {
                        operator: TemporalOperator::Past,
                        body: with_aspect,
                    })
                } else {
                    with_aspect
                }
            } else if self.check_content_word() {
                let adj = self.consume_content_word()?;
                self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: adj,
                    args: self.ctx.terms.alloc_slice([Term::Constant(subject_symbol)]),
                    world: None,
                })
            } else {
                self.ctx.exprs.alloc(LogicExpr::Atom(subject_symbol))
            };
            return Ok(self.ctx.exprs.alloc(LogicExpr::YesNoQuestion { body }));
        }

        let body = self.parse_predicate_with_subject(subject_symbol)?;

        let wrapped_body = if is_modal {
            let vector = self.aux_token_to_modal_vector(&aux_token);
            self.ctx.exprs.alloc(LogicExpr::Modal {
                vector,
                operand: body,
            })
        } else {
            body
        };

        Ok(self.ctx.exprs.alloc(LogicExpr::YesNoQuestion { body: wrapped_body }))
    }

    fn aux_token_to_modal_vector(&self, token: &TokenType) -> ModalVector {
        use crate::ast::ModalFlavor;
        match token {
            // Root modals (narrow scope)
            TokenType::Can => ModalVector {
                domain: ModalDomain::Alethic,
                force: 0.5,
                flavor: ModalFlavor::Root,
            },
            TokenType::Could => ModalVector {
                domain: ModalDomain::Alethic,
                force: 0.4,
                flavor: ModalFlavor::Root,
            },
            TokenType::Would => ModalVector {
                domain: ModalDomain::Alethic,
                force: 0.6,
                flavor: ModalFlavor::Root,
            },
            TokenType::Must => ModalVector {
                domain: ModalDomain::Alethic,
                force: 1.0,
                flavor: ModalFlavor::Root,
            },
            TokenType::Should => ModalVector {
                domain: ModalDomain::Deontic,
                force: 0.6,
                flavor: ModalFlavor::Root,
            },
            // Epistemic modals (wide scope)
            TokenType::May => ModalVector {
                domain: ModalDomain::Deontic,
                force: 0.5,
                flavor: ModalFlavor::Epistemic,
            },
            TokenType::Might => ModalVector {
                domain: ModalDomain::Alethic,
                force: 0.3,
                flavor: ModalFlavor::Epistemic,
            },
            _ => ModalVector {
                domain: ModalDomain::Alethic,
                force: 0.5,
                flavor: ModalFlavor::Root,
            },
        }
    }
}
