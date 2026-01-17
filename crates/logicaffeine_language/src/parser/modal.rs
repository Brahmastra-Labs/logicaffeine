//! Modal verb parsing with Kripke semantics support.
//!
//! This module handles modal auxiliaries (can, could, may, might, must, should, would)
//! and their semantic interpretation using modal vectors that encode:
//!
//! - **Domain**: Alethic (possibility/necessity) vs Deontic (permission/obligation)
//! - **Flavor**: Root (circumstantial) vs Epistemic (knowledge-based)
//! - **Force**: Possibility (◇) vs Necessity (□)
//!
//! # Modal Vector Examples
//!
//! | Modal | Default Reading | Alternative Reading |
//! |-------|-----------------|---------------------|
//! | can   | Ability (Root)  | Permission (Deontic) |
//! | may   | Permission (Deontic) | Possibility (Epistemic) |
//! | must  | Necessity (Root) | Obligation (Deontic) |
//! | might | Possibility (Epistemic) | - |
//!
//! The module also handles aspect chains (perfect "have", progressive "be -ing").

use super::clause::ClauseParsing;
use super::noun::NounParsing;
use super::{ParseResult, Parser};
use crate::ast::{AspectOperator, LogicExpr, ModalDomain, ModalFlavor, ModalVector, NeoEventData, ThematicRole, VoiceOperator, Term};
use crate::drs::TimeRelation;
use crate::error::{ParseError, ParseErrorKind};
use logicaffeine_base::Symbol;
use crate::lexicon::{Time, Aspect};
use crate::token::TokenType;

/// Trait for parsing modal verbs and aspect chains.
///
/// Provides methods for interpreting modal auxiliaries (can, must, etc.)
/// with Kripke semantics and handling aspect markers (perfect, progressive).
pub trait ModalParsing<'a, 'ctx, 'int> {
    /// Parses a modal verb and its scope content.
    fn parse_modal(&mut self) -> ParseResult<&'a LogicExpr<'a>>;
    /// Parses perfect/progressive aspect chain with a symbol subject.
    fn parse_aspect_chain(&mut self, subject_symbol: Symbol) -> ParseResult<&'a LogicExpr<'a>>;
    /// Parses perfect/progressive aspect chain with a term subject.
    fn parse_aspect_chain_with_term(&mut self, subject_term: Term<'a>) -> ParseResult<&'a LogicExpr<'a>>;
    /// Converts a modal token to its semantic vector (domain, force, flavor).
    fn token_to_vector(&self, token: &TokenType) -> ModalVector;
}

impl<'a, 'ctx, 'int> ModalParsing<'a, 'ctx, 'int> for Parser<'a, 'ctx, 'int> {
    fn parse_modal(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        use crate::drs::BoxType;

        let vector = self.token_to_vector(&self.previous().kind.clone());

        // Enter modal box in parser's DRS (not world_state - that's swapped at sentence boundaries)
        self.drs.enter_box(BoxType::ModalScope);

        if self.check(&TokenType::That) {
            self.advance();
        }

        let content = self.parse_sentence()?;

        // Exit modal box
        self.drs.exit_box();

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
            || self.check(&TokenType::Cannot) || self.check(&TokenType::Might) {
            let modal_token = self.peek().kind.clone();
            self.advance();
            has_modal = true;
            let vector = self.token_to_vector(&modal_token);
            modal_vector = Some(vector.clone());
            // Enter modal box in DRS so any new referents are marked as hypothetical
            // This ensures "A wolf might enter" puts the wolf in a modal scope
            self.drs.enter_box(crate::drs::BoxType::ModalScope);
            // Also set modal context on WorldState for cross-sentence tracking
            // This is used by end_sentence() to mark telescope candidates as modal-sourced
            let is_epistemic = matches!(vector.flavor, crate::ast::ModalFlavor::Epistemic);
            self.world_state.enter_modal_context(is_epistemic, vector.force);
        }

        if self.check(&TokenType::Not) {
            self.advance();
            has_negation = true;
        }

        // Check for "be able to" periphrastic modal (= can)
        // This creates a nested modal: "might be able to fly" → ◇◇Fly(x)
        let mut nested_modal_vector = None;
        if self.check_content_word() {
            let word = self.interner.resolve(self.peek().lexeme).to_lowercase();
            if word == "be" {
                // Look ahead for "able to"
                if let Some(next1) = self.tokens.get(self.current + 1) {
                    let next1_word = self.interner.resolve(next1.lexeme).to_lowercase();
                    if next1_word == "able" {
                        if let Some(next2) = self.tokens.get(self.current + 2) {
                            if matches!(next2.kind, TokenType::To) {
                                // Consume "be able to" - it's a modal meaning "can" (ability)
                                self.advance(); // consume "be"
                                self.advance(); // consume "able"
                                self.advance(); // consume "to"
                                nested_modal_vector = Some(ModalVector {
                                    domain: ModalDomain::Alethic,
                                    force: 0.5, // ability = possibility
                                    flavor: ModalFlavor::Root, // "be able to" = Root modal (ability)
                                });
                            }
                        }
                    }
                }
            }
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
            let r_var = self.world_state.next_reference_time();
            self.world_state.add_time_constraint(r_var, TimeRelation::Precedes, "S".to_string());
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
            world: None,
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

            // Check pending_time to set up reference time for tense
            if let Some(pending) = self.pending_time.take() {
                match pending {
                    Time::Future => {
                        // Future perfect: S < R
                        let r_var = self.world_state.next_reference_time();
                        self.world_state.add_time_constraint("S".to_string(), TimeRelation::Precedes, r_var);
                    }
                    Time::Past => {
                        // Past perfect fallback (if not already set by "had")
                        if self.world_state.current_reference_time() == "S" {
                            let r_var = self.world_state.next_reference_time();
                            self.world_state.add_time_constraint(r_var, TimeRelation::Precedes, "S".to_string());
                        }
                    }
                    _ => {}
                }
            }

            // Perfect: E < R (event before reference)
            let e_var = format!("e{}", self.world_state.event_history().len().max(1));
            let r_var = self.world_state.current_reference_time();
            self.world_state.add_time_constraint(e_var, TimeRelation::Precedes, r_var);
        }

        if has_negation {
            result = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: result,
            });
        }

        // Apply nested modal first (from "be able to" = ability)
        if let Some(vector) = nested_modal_vector {
            result = self.ctx.modal(vector, result);
        }

        // Then apply outer modal (e.g., "might")
        if has_modal {
            // Exit modal box in DRS (matches enter_box above)
            self.drs.exit_box();
            // Note: We do NOT exit_modal_context() here because we want the modal flag
            // to persist until end_sentence() so telescope candidates are marked as modal.
            // The modal context is cleared by end_sentence() → prior_modal_context.take()
            if let Some(vector) = modal_vector {
                result = self.ctx.modal(vector, result);
            }
        }

        Ok(result)
    }

    fn token_to_vector(&self, token: &TokenType) -> ModalVector {
        use crate::ast::ModalFlavor;
        use super::ModalPreference;

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

            // Polysemous modal: CAN
            // Default: Ability (Alethic, Root/Narrow)
            // Deontic: Permission (Deontic, Root/Narrow)
            TokenType::Can => {
                match self.modal_preference {
                    ModalPreference::Deontic => {
                        // Permission: "You can go" (Deontic, Narrow Scope)
                        ModalVector {
                            domain: ModalDomain::Deontic,
                            force: 0.5,
                            flavor: ModalFlavor::Root,
                        }
                    }
                    _ => {
                        // Ability: "Birds can fly" (Alethic, Narrow Scope)
                        ModalVector {
                            domain: ModalDomain::Alethic,
                            force: 0.5,
                            flavor: ModalFlavor::Root,
                        }
                    }
                }
            },

            // Polysemous modal: COULD
            // Default: Past Ability (Alethic, Root/Narrow)
            // Epistemic: Conditional Possibility (Alethic, Epistemic/Wide)
            TokenType::Could => {
                match self.modal_preference {
                    ModalPreference::Epistemic => {
                        // Conditional Possibility: "It could rain" (Alethic, Wide Scope)
                        ModalVector {
                            domain: ModalDomain::Alethic,
                            force: 0.5,
                            flavor: ModalFlavor::Epistemic,
                        }
                    }
                    _ => {
                        // Past Ability: "She could swim" (Alethic, Narrow Scope)
                        ModalVector {
                            domain: ModalDomain::Alethic,
                            force: 0.5,
                            flavor: ModalFlavor::Root,
                        }
                    }
                }
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

            // Polysemous modal: MAY
            // Default: Permission (Deontic, Root/Narrow)
            // Epistemic: Possibility (Alethic, Epistemic/Wide)
            TokenType::May => {
                match self.modal_preference {
                    ModalPreference::Epistemic => {
                        // Possibility: "It may rain" (Alethic, Wide Scope)
                        ModalVector {
                            domain: ModalDomain::Alethic,
                            force: 0.5,
                            flavor: ModalFlavor::Epistemic,
                        }
                    }
                    _ => {
                        // Permission: "Students may leave" (Deontic, Narrow Scope)
                        ModalVector {
                            domain: ModalDomain::Deontic,
                            force: 0.5,
                            flavor: ModalFlavor::Root,
                        }
                    }
                }
            },

            _ => panic!("Unknown modal token: {:?}", token),
        }
    }
}
