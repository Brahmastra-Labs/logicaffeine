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
use super::pragmatics::PragmaticsParsing;
use super::{ParseResult, Parser};
use crate::ast::{AspectOperator, LogicExpr, ModalDomain, ModalFlavor, ModalVector, NeoEventData, QuantifierKind, ThematicRole, VoiceOperator, Term};
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
            || self.check(&TokenType::Cannot) || self.check(&TokenType::Might)
            || self.check(&TokenType::Shall) {
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

        // "shall never send" / "must never fail" — treat Never as negation
        if self.check(&TokenType::Never) && !has_negation {
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
                                    modal_base: None,
                                    ordering_source: None,
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

        // Presupposition trigger under the modal/aspect chain: "Mary might
        // regret lying." The presupposition PROJECTS out of the modal (Van
        // der Sandt global accommodation) — the modal wraps only the
        // assertion: Presup(◇Regret(m), ⟨Lied(m)⟩).
        if self.check_presup_trigger()
            && !self.is_followed_by_np_object()
            && self.is_followed_by_gerund()
        {
            if let Term::Constant(subj_sym) = subject_term {
                let presup_kind = match self.advance().kind {
                    TokenType::PresupTrigger(kind) => kind,
                    TokenType::Verb { lemma, .. } => {
                        let s = self.interner.resolve(lemma).to_lowercase();
                        crate::lexicon::lookup_presup_trigger(&s).expect(
                            "Lexicon mismatch: Verb flagged as trigger but lookup failed",
                        )
                    }
                    _ => unreachable!("guarded by check_presup_trigger"),
                };
                let subject_np = crate::ast::NounPhrase::simple(subj_sym);
                let parsed =
                    self.parse_presupposition(&subject_np, presup_kind, has_negation)?;
                if has_modal {
                    self.drs.exit_box();
                }
                if let LogicExpr::Presupposition {
                    assertion,
                    presupposition,
                } = parsed
                {
                    let mut asserted: &'a LogicExpr<'a> = assertion;
                    if has_modal {
                        if let Some(vector) = modal_vector {
                            asserted = self.ctx.modal(vector, asserted);
                        }
                    }
                    return Ok(self.ctx.exprs.alloc(LogicExpr::Presupposition {
                        assertion: asserted,
                        presupposition,
                    }));
                }
                let mut result: &'a LogicExpr<'a> = parsed;
                if has_modal {
                    if let Some(vector) = modal_vector {
                        result = self.ctx.modal(vector, result);
                    }
                }
                return Ok(result);
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
        let mut object_quant: Option<(QuantifierKind, Symbol, Symbol)> = None;

        if has_passive && self.check_preposition() {
            if let TokenType::Preposition(sym) = self.peek().kind {
                if self.interner.resolve(sym) == "by" {
                    self.advance();
                    let agent_np = self.parse_noun_phrase(true)?;
                    let agent_term = self.noun_phrase_to_term(&agent_np);
                    roles.push((ThematicRole::Agent, agent_term));
                }
            }
        } else if !has_passive
            && matches!(
                self.peek().kind,
                TokenType::All | TokenType::Some | TokenType::Cardinal(_)
            )
        {
            // Quantified object under a modal ("shall acknowledge every
            // request", "shall never drop two consecutive bytes"): the
            // object raises past the root modal —
            // ∀x(Request(x) → MODAL(event with Theme x)).
            let kind = match self.advance().kind {
                TokenType::All => QuantifierKind::Universal,
                TokenType::Cardinal(n) => QuantifierKind::Cardinal(n),
                _ => QuantifierKind::Existential,
            };
            let obj_np = self.parse_noun_phrase(false)?;
            let var = self.next_var_name();
            roles.push((ThematicRole::Theme, Term::Variable(var)));
            object_quant = Some((kind, var, obj_np.noun));
        } else if !has_passive && (self.check_content_word() || self.check_article()) {
            let obj_np = self.parse_noun_phrase(false)?;
            let obj_term = self.noun_phrase_to_term(&obj_np);
            roles.push((ThematicRole::Theme, obj_term));
        } else if !has_passive && self.check_pronoun() {
            // Pronoun object ("A wolf would eat you."): person deictics resolve
            // to discourse roles, third person to the discourse referent.
            let token = self.advance().clone();
            let plex = self.interner.resolve(token.lexeme).to_lowercase();
            let obj_term = match plex.as_str() {
                "you" | "yourself" => Term::Constant(self.interner.intern("Addressee")),
                "i" | "me" | "myself" => Term::Constant(self.interner.intern("Speaker")),
                _ => {
                    let (gender, number) = match &token.kind {
                        TokenType::Pronoun { gender, number, .. } => (*gender, *number),
                        _ => (crate::drs::Gender::Unknown, crate::drs::Number::Singular),
                    };
                    match self.resolve_pronoun(gender, number)? {
                        super::ResolvedPronoun::Variable(s) => Term::Variable(s),
                        super::ResolvedPronoun::Constant(s) => Term::Constant(s),
                    }
                }
            };
            roles.push((ThematicRole::Theme, obj_term));
        }

        let event_var = self.get_event_var();
        let mut modifiers: Vec<Symbol> = Vec::new();
        // Manner adverbs under the modal ("would spread quickly").
        modifiers.extend(self.collect_adverbs());
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

        // PPs and clause-final particles under the modal ("might walk in",
        // "would go to the store") modify the same event.
        let mut base_pred: &'a LogicExpr<'a> = base_pred;
        while self.check_preposition() {
            let prep_name = if let TokenType::Preposition(sym) = self.peek().kind {
                sym
            } else {
                break;
            };
            let np_follows = match self.tokens.get(self.current + 1).map(|t| &t.kind) {
                Some(TokenType::Noun(_) | TokenType::ProperName(_) | TokenType::Article(_)) => true,
                // A noun READING suffices ("during transfer" — "transfer"
                // lexes Ambiguous{Verb|Noun}); the NP parse commits it.
                Some(TokenType::Ambiguous { primary, alternatives }) => {
                    matches!(**primary, TokenType::Noun(_))
                        || alternatives.iter().any(|t| matches!(t, TokenType::Noun(_)))
                }
                _ => false,
            };
            let pp_pred = if np_follows {
                self.advance(); // preposition
                let pp_np = self.parse_noun_phrase(false)?;
                self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: prep_name,
                    args: self.ctx.terms.alloc_slice([
                        Term::Variable(event_var),
                        Term::Constant(pp_np.noun),
                    ]),
                    world: None,
                })
            } else {
                self.advance(); // preposition
                if !self.at_clause_boundary()
                    || !crate::lexicon::is_particle(
                        &self.interner.resolve(prep_name).to_lowercase(),
                    )
                {
                    // Not a lexical particle at a clause end — hand it back.
                    self.current -= 1;
                    break;
                }
                // Intransitive particle/directional: event modifier.
                self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: prep_name,
                    args: self.ctx.terms.alloc_slice([Term::Variable(event_var)]),
                    world: None,
                })
            };
            base_pred = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: base_pred,
                op: TokenType::And,
                right: pp_pred,
            });
        }

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

        if let Some((kind, var, noun)) = object_quant {
            let restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: noun,
                args: self.ctx.terms.alloc_slice([Term::Variable(var)]),
                world: None,
            });
            let connective = if matches!(kind, QuantifierKind::Universal) {
                TokenType::Implies
            } else {
                TokenType::And
            };
            let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: restriction,
                op: connective,
                right: result,
            });
            result = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind,
                variable: var,
                body,
                island_id: self.current_island,
            });
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
                flavor: ModalFlavor::Root, modal_base: None, ordering_source: None
            },
            TokenType::Cannot => ModalVector {
                domain: ModalDomain::Alethic,
                force: 0.0,
                flavor: ModalFlavor::Root, modal_base: None, ordering_source: None
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
                            flavor: ModalFlavor::Root, modal_base: None, ordering_source: None
                        }
                    }
                    _ => {
                        // Ability: "Birds can fly" (Alethic, Narrow Scope)
                        ModalVector {
                            domain: ModalDomain::Alethic,
                            force: 0.5,
                            flavor: ModalFlavor::Root, modal_base: None, ordering_source: None
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
                            flavor: ModalFlavor::Epistemic, modal_base: None, ordering_source: None
                        }
                    }
                    _ => {
                        // Past Ability: "She could swim" (Alethic, Narrow Scope)
                        ModalVector {
                            domain: ModalDomain::Alethic,
                            force: 0.5,
                            flavor: ModalFlavor::Root, modal_base: None, ordering_source: None
                        }
                    }
                }
            },

            TokenType::Would => ModalVector {
                domain: ModalDomain::Alethic,
                force: 0.5,
                flavor: ModalFlavor::Root, modal_base: None, ordering_source: None
            },
            TokenType::Shall => ModalVector {
                domain: ModalDomain::Deontic,
                force: 0.9,
                flavor: ModalFlavor::Root, modal_base: None, ordering_source: None
            },
            TokenType::Should => ModalVector {
                domain: ModalDomain::Deontic,
                force: 0.6,
                flavor: ModalFlavor::Root, modal_base: None, ordering_source: None
            },

            // Epistemic modals → Wide Scope (De Dicto)
            // These wrap the entire quantifier in the modal
            TokenType::Might => ModalVector {
                domain: ModalDomain::Alethic,
                force: 0.3,
                flavor: ModalFlavor::Epistemic, modal_base: None, ordering_source: None
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
                            flavor: ModalFlavor::Epistemic, modal_base: None, ordering_source: None
                        }
                    }
                    _ => {
                        // Permission: "Students may leave" (Deontic, Narrow Scope)
                        ModalVector {
                            domain: ModalDomain::Deontic,
                            force: 0.5,
                            flavor: ModalFlavor::Root, modal_base: None, ordering_source: None
                        }
                    }
                }
            },

            _ => panic!("Unknown modal token: {:?}", token),
        }
    }
}
