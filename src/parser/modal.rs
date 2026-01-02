use super::clause::ClauseParsing;
use super::noun::NounParsing;
use super::{ParseResult, Parser};
use crate::ast::{AspectOperator, LogicExpr, ModalDomain, ModalVector, NeoEventData, ThematicRole, VoiceOperator, Term};
use crate::context::TimeRelation;
use crate::error::{ParseError, ParseErrorKind};
use crate::intern::Symbol;
use crate::lexicon::{Time, Aspect};
use crate::token::TokenType;

pub trait ModalParsing<'a, 'ctx, 'int> {
    fn parse_modal(&mut self) -> ParseResult<&'a LogicExpr<'a>>;
    fn parse_aspect_chain(&mut self, subject_symbol: Symbol) -> ParseResult<&'a LogicExpr<'a>>;
    fn parse_aspect_chain_with_term(&mut self, subject_term: Term<'a>) -> ParseResult<&'a LogicExpr<'a>>;
    fn token_to_vector(&self, token: &TokenType) -> ModalVector;
}

impl<'a, 'ctx, 'int> ModalParsing<'a, 'ctx, 'int> for Parser<'a, 'ctx, 'int> {
    fn parse_modal(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        let vector = self.token_to_vector(&self.previous().kind.clone());

        if self.check(&TokenType::That) {
            self.advance();
        }

        let content = self.parse_sentence()?;

        Ok(self.ctx.exprs.alloc(LogicExpr::Modal {
            vector,
            operand: content,
        }))
    }

    fn parse_aspect_chain(&mut self, subject_symbol: Symbol) -> ParseResult<&'a LogicExpr<'a>> {
        self.parse_aspect_chain_with_term(Term::Constant(subject_symbol))
    }

    fn parse_aspect_chain_with_term(&mut self, subject_term: Term<'a>) -> ParseResult<&'a LogicExpr<'a>> {
        let mut has_modal = false;
        let mut modal_vector = None;
        let mut has_negation = false;
        let mut has_perfect = false;
        let mut has_passive = false;
        let mut has_progressive = false;

        if self.check(&TokenType::Would) || self.check(&TokenType::Could)
            || self.check(&TokenType::Must) || self.check(&TokenType::Can)
            || self.check(&TokenType::Should) || self.check(&TokenType::May)
            || self.check(&TokenType::Cannot) {
            let modal_token = self.peek().kind.clone();
            self.advance();
            has_modal = true;
            modal_vector = Some(self.token_to_vector(&modal_token));
        }

        if self.check(&TokenType::Not) {
            self.advance();
            has_negation = true;
        }

        if self.check_content_word() {
            let word = self.interner.resolve(self.peek().lexeme).to_lowercase();
            if word == "have" || word == "has" || word == "had" {
                self.advance();
                has_perfect = true;
            }
        }

        if self.check(&TokenType::Had) {
            self.advance();
            has_perfect = true;
            // "had" = past perfect: R < S (past reference time)
            if let Some(ref mut context) = self.context {
                let r_var = context.next_reference_time();
                context.add_time_constraint(r_var, TimeRelation::Precedes, "S".to_string());
            }
        }

        if self.check_content_word() {
            let word = self.interner.resolve(self.peek().lexeme).to_lowercase();
            if word == "been" {
                self.advance();

                if self.check_verb() {
                    match &self.peek().kind {
                        TokenType::Verb { aspect: Aspect::Progressive, .. } => {
                            has_progressive = true;
                        }
                        TokenType::Verb { .. } => {
                            let next_word = self.interner.resolve(self.peek().lexeme);
                            if next_word.ends_with("ing") {
                                has_progressive = true;
                            } else {
                                has_passive = true;
                            }
                        }
                        _ => {
                            has_passive = true;
                        }
                    }
                }
            }
        }

        if self.check_content_word() {
            let word = self.interner.resolve(self.peek().lexeme).to_lowercase();
            if word == "being" {
                self.advance();
                has_progressive = true;
            }
        }

        let verb = if self.check_verb() {
            self.consume_verb()
        } else if self.check_content_word() {
            self.consume_content_word()?
        } else {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedContentWord { found: self.peek().kind.clone() },
                span: self.peek().span.clone(),
            });
        };

        let subject_role = if has_passive {
            ThematicRole::Theme
        } else {
            ThematicRole::Agent
        };
        let mut roles: Vec<(ThematicRole, Term<'a>)> = vec![(subject_role, subject_term)];

        if has_passive && self.check_preposition() {
            if let TokenType::Preposition(sym) = self.peek().kind {
                if self.interner.resolve(sym) == "by" {
                    self.advance();
                    let agent_np = self.parse_noun_phrase(true)?;
                    let agent_term = self.noun_phrase_to_term(&agent_np);
                    roles.push((ThematicRole::Agent, agent_term));
                }
            }
        } else if !has_passive && (self.check_content_word() || self.check_article()) {
            let obj_np = self.parse_noun_phrase(false)?;
            let obj_term = self.noun_phrase_to_term(&obj_np);
            roles.push((ThematicRole::Theme, obj_term));
        }

        let event_var = self.get_event_var();
        let mut modifiers: Vec<Symbol> = Vec::new();
        if let Some(pending) = self.pending_time {
            match pending {
                Time::Past => modifiers.push(self.interner.intern("Past")),
                Time::Future => modifiers.push(self.interner.intern("Future")),
                _ => {}
            }
        }
        let suppress_existential = self.drs.in_conditional_antecedent();
        let base_pred = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
            event_var,
            verb,
            roles: self.ctx.roles.alloc_slice(roles.clone()),
            modifiers: self.ctx.syms.alloc_slice(modifiers.clone()),
            suppress_existential,
        })));

        // Capture template for ellipsis reconstruction
        self.capture_event_template(verb, &roles, &modifiers);

        let mut result: &'a LogicExpr<'a> = base_pred;

        if has_progressive {
            result = self.ctx.aspectual(AspectOperator::Progressive, result);
        }

        if has_passive {
            result = self.ctx.voice(VoiceOperator::Passive, result);
        }

        if has_perfect {
            result = self.ctx.aspectual(AspectOperator::Perfect, result);
            if let Some(ref mut context) = self.context {
                // Check pending_time to set up reference time for tense
                if let Some(pending) = self.pending_time.take() {
                    match pending {
                        Time::Future => {
                            let r_var = context.next_reference_time();
                            context.add_time_constraint("S".to_string(), TimeRelation::Precedes, r_var);
                        }
                        Time::Past => {
                            // Past tense with perfect (should be handled by "had" already, but as fallback)
                            if context.current_reference_time() == "S" {
                                let r_var = context.next_reference_time();
                                context.add_time_constraint(r_var, TimeRelation::Precedes, "S".to_string());
                            }
                        }
                        _ => {}
                    }
                }
                // Perfect: E < R
                let e_var = format!("e{}", context.event_history().len().max(1));
                let r_var = context.current_reference_time();
                context.add_time_constraint(e_var, TimeRelation::Precedes, r_var);
            }
        }

        if has_negation {
            result = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: result,
            });
        }

        if has_modal {
            if let Some(vector) = modal_vector {
                result = self.ctx.modal(vector, result);
            }
        }

        Ok(result)
    }

    fn token_to_vector(&self, token: &TokenType) -> ModalVector {
        use crate::ast::ModalFlavor;

        match token {
            // Root modals → Narrow Scope (De Re)
            // These attach the modal to the predicate inside the quantifier
            TokenType::Must => ModalVector {
                domain: ModalDomain::Alethic,
                force: 1.0,
                flavor: ModalFlavor::Root,
            },
            TokenType::Cannot => ModalVector {
                domain: ModalDomain::Alethic,
                force: 0.0,
                flavor: ModalFlavor::Root,
            },
            TokenType::Can => ModalVector {
                domain: ModalDomain::Alethic,
                force: 0.5,
                flavor: ModalFlavor::Root,
            },
            TokenType::Could => ModalVector {
                domain: ModalDomain::Alethic,
                force: 0.5,
                flavor: ModalFlavor::Root,
            },
            TokenType::Would => ModalVector {
                domain: ModalDomain::Alethic,
                force: 0.5,
                flavor: ModalFlavor::Root,
            },
            TokenType::Shall => ModalVector {
                domain: ModalDomain::Deontic,
                force: 0.9,
                flavor: ModalFlavor::Root,
            },
            TokenType::Should => ModalVector {
                domain: ModalDomain::Deontic,
                force: 0.6,
                flavor: ModalFlavor::Root,
            },

            // Epistemic modals → Wide Scope (De Dicto)
            // These wrap the entire quantifier in the modal
            TokenType::Might => ModalVector {
                domain: ModalDomain::Alethic,
                force: 0.3,
                flavor: ModalFlavor::Epistemic,
            },
            TokenType::May => ModalVector {
                domain: ModalDomain::Deontic,
                force: 0.5,
                flavor: ModalFlavor::Epistemic,
            },

            _ => panic!("Unknown modal token: {:?}", token),
        }
    }
}
