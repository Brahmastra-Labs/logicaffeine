//! Quantifier parsing and scope management.
//!
//! This module handles determiners with quantificational force:
//!
//! - **Universal**: every, all, each → `∀x`
//! - **Existential**: a, an, some → `∃x`
//! - **Negative**: no, neither → `¬∃x` or `∀x(... → ¬...)`
//! - **Proportional**: most, few, many → generalized quantifiers
//! - **Definite**: the → uniqueness presupposition (ιx)
//!
//! # Quantifier Scope
//!
//! Quantifiers are assigned to scope islands during parsing. The `island_id` field
//! tracks which island a quantifier belongs to, preventing illicit scope inversions
//! (e.g., extracting from relative clauses).
//!
//! # Donkey Anaphora
//!
//! Indefinites in conditional antecedents or relative clauses receive universal
//! force when bound by a pronoun in the main clause:
//!
//! "If a farmer owns a donkey, he beats it" → `∀x∀y((Farmer(x) ∧ Donkey(y) ∧ Owns(x,y)) → Beats(x,y))`

use super::clause::ClauseParsing;
use super::modal::ModalParsing;
use super::noun::NounParsing;
use super::pragmatics::PragmaticsParsing;
use super::{NegativeScopeMode, ParseResult, Parser};
use crate::ast::{LogicExpr, NeoEventData, NounPhrase, QuantifierKind, Term, ThematicRole};
use crate::drs::{Gender, Number};
use crate::drs::ReferentSource;
use crate::error::{ParseError, ParseErrorKind};
use logicaffeine_base::Symbol;
use crate::lexer::Lexer;
use crate::lexicon::{
    get_canonical_verb, is_subsective, lookup_relational_adjective, lookup_verb_db, Definiteness,
    Feature, Time,
};
use crate::token::{PresupKind, TokenType};

/// Trait for parsing quantified expressions and managing scope.
///
/// Provides methods for parsing quantifiers (every, some, no, most),
/// their restrictions, and wrapping expressions with appropriate scope.
pub trait QuantifierParsing<'a, 'ctx, 'int> {
    /// Parses a quantified expression from a quantifier determiner.
    fn parse_quantified(&mut self) -> ParseResult<&'a LogicExpr<'a>>;

    /// The quantifier-parsing body; `parse_quantified` wraps its result with any pending
    /// partitive-superset presupposition (§5.3).
    fn parse_quantified_core(&mut self) -> ParseResult<&'a LogicExpr<'a>>;
    /// Parses the restrictor clause for a quantifier.
    fn parse_restriction(&mut self, var_name: Symbol) -> ParseResult<&'a LogicExpr<'a>>;
    /// Builds the restriction conjunct for one pre-nominal adjective, dispatching
    /// on its lexical class (relational/subsective/intersective). Shared by every
    /// NP-restriction path so all paths model adjective classes identically.
    fn adjective_restriction(&mut self, adj: Symbol, var: Symbol, noun: Symbol) -> &'a LogicExpr<'a>;
    /// Parses a verb phrase as the nuclear scope of a quantifier.
    fn parse_verb_phrase_for_restriction(&mut self, var_name: Symbol) -> ParseResult<&'a LogicExpr<'a>>;
    /// Combines multiple expressions with conjunction.
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
    /// Rewrite a relativized gap variable into a constant, so a subject's
    /// relative-clause restriction (built over `from_var`) can be folded into a
    /// predicate keyed on `to_const` and then re-bound uniformly by
    /// `wrap_with_definiteness`.
    fn substitute_variable_with_constant(
        &self,
        expr: &'a LogicExpr<'a>,
        from_var: Symbol,
        to_const: Symbol,
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
    /// Parse a copula PP complement under a quantifier — "is in Florida or in
    /// Maine" → In(subj,Florida) ∨ In(subj,Maine), `subj_var` being the bound
    /// variable. Captures the or-coordination ("or in B" repeats the preposition,
    /// "or B" reuses it) that the backtracking fallback path otherwise drops.
    fn parse_copula_pp_complement(
        &mut self,
        subj_var: Symbol,
    ) -> ParseResult<&'a LogicExpr<'a>>;
}

impl<'a, 'ctx, 'int> QuantifierParsing<'a, 'ctx, 'int> for Parser<'a, 'ctx, 'int> {
    fn parse_copula_pp_complement(
        &mut self,
        subj_var: Symbol,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        let prep_sym = match self.advance().kind {
            TokenType::Preposition(s) => s,
            _ => unreachable!("guarded by check_preposition"),
        };
        let saved_ctx = self.nominal_np_context;
        self.nominal_np_context = true;
        let obj_res = self.parse_noun_phrase(true);
        self.nominal_np_context = saved_ctx;
        let obj = obj_res?;
        let first: &'a LogicExpr<'a> = self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: prep_sym,
            args: self
                .ctx
                .terms
                .alloc_slice([Term::Variable(subj_var), Term::Constant(obj.noun)]),
            world: None,
        });
        let mut base = self.attach_pp_object_modifiers(first, &obj);
        while self.check(&TokenType::Or) {
            let cp = self.checkpoint();
            self.advance(); // "or"
            let disj_prep = if self.check_preposition() && !self.check_by_preposition() {
                match self.advance().kind {
                    TokenType::Preposition(s) => s,
                    _ => prep_sym,
                }
            } else {
                prep_sym
            };
            if !(self.check_content_word()
                || self.check_number()
                || matches!(self.peek().kind, TokenType::Article(_)))
            {
                self.restore(cp);
                break;
            }
            let saved = self.nominal_np_context;
            self.nominal_np_context = true;
            let disj_obj_res = self.parse_noun_phrase(true);
            self.nominal_np_context = saved;
            let disj_obj = disj_obj_res?;
            let disj: &'a LogicExpr<'a> = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: disj_prep,
                args: self
                    .ctx
                    .terms
                    .alloc_slice([Term::Variable(subj_var), Term::Constant(disj_obj.noun)]),
                world: None,
            });
            let disj = self.attach_pp_object_modifiers(disj, &disj_obj);
            base = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: base,
                op: TokenType::Or,
                right: disj,
            });
        }
        Ok(base)
    }

    fn parse_quantified(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        // The specialized quantified-VP grammar below covers many frames but
        // not all of them. A parse that stops mid-clause silently drops the
        // remainder's meaning, so when it under-consumes (or fails), re-parse
        // the clause delegating the VP to the full predicate parser. The
        // specialized result is kept whenever the delegation does no better.
        let cp = self.checkpoint();
        let core = self.parse_quantified_core();
        let core_complete = core.is_ok() && self.at_clause_boundary();
        let result = if core_complete {
            core?
        } else {
            let core_end = self.checkpoint();
            let core_partitive = self.pending_partitive.take();
            self.restore(cp);
            match self.parse_quantified_delegating() {
                Ok(r) if self.at_clause_boundary() => r,
                _ => match core {
                    Ok(r) => {
                        self.restore(core_end);
                        self.pending_partitive = core_partitive;
                        r
                    }
                    Err(e) => return Err(e),
                },
            }
        };
        // §5.3: a partitive "of the [Num]" frame presupposes a salient set of that
        // cardinality. Surface it as `assertion [Presup: ∃=n x Restriction(x)]` rather
        // than discarding the superset.
        if let Some((n, restriction, var)) = self.pending_partitive.take() {
            let superset = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind: QuantifierKind::Cardinal(n),
                variable: var,
                body: restriction,
                island_id: self.current_island,
            });
            return Ok(self.ctx.exprs.alloc(LogicExpr::Presupposition {
                assertion: result,
                presupposition: superset,
            }));
        }
        Ok(result)
    }

    fn parse_quantified_core(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        let quantifier_token = self.previous().kind.clone();
        let var_name = self.next_var_name();

        // Track if we're inside a "No" quantifier - referents introduced here
        // are inaccessible for cross-sentence anaphora
        let was_in_negative_quantifier = self.in_negative_quantifier;
        if matches!(quantifier_token, TokenType::No) {
            self.in_negative_quantifier = true;
        }

        // Partitive: "Two of the three students passed.", "Most of the students
        // passed." The "of the [Num]" frame restricts the quantifier to a salient
        // presupposed definite set. Consume the frame so the quantifier ranges over
        // the head noun normally; the leading cardinal/proportion is the count, the
        // optional inner cardinal is the presupposed superset size.
        let mut partitive_superset: Option<u32> = None;
        if matches!(
            quantifier_token,
            TokenType::Cardinal(_)
                | TokenType::Most
                | TokenType::Few
                | TokenType::Many
                | TokenType::Some
                | TokenType::AtLeast(_)
                | TokenType::AtMost(_)
        ) && self.check_preposition_is("of")
            && self.current + 1 < self.tokens.len()
            && matches!(self.tokens[self.current + 1].kind, TokenType::Article(_))
        {
            self.advance(); // consume "of"
            self.advance(); // consume "the"
            if let TokenType::Cardinal(n) = self.peek().kind {
                partitive_superset = Some(n);
                self.advance(); // consume the superset cardinal ("three")
            }
        }
        // "At most one of X, Y, and Z is P" — counting quantifier with explicit list
        if matches!(quantifier_token, TokenType::AtMost(_) | TokenType::AtLeast(_) | TokenType::Cardinal(_))
            && self.check_preposition_is("of")
        {
            self.advance(); // consume "of"

            // Parse comma-separated list of identifiers: "grant0, grant1, and grant2"
            let mut signal_names: Vec<Symbol> = Vec::new();
            loop {
                let name = self.consume_content_word()?;
                signal_names.push(name);

                if self.check(&TokenType::Comma) {
                    self.advance(); // consume ","
                    // Skip optional "and" after comma: "X, Y, and Z"
                    if self.check(&TokenType::And) {
                        self.advance();
                    }
                } else if self.check(&TokenType::And) {
                    self.advance(); // consume "and" (two-element: "X and Y")
                } else {
                    break;
                }
            }

            // Now parse the predicate: "is asserted", "is valid", etc.
            // In hardware context, the predicate is implicit — what matters
            // is each signal name being high/low. Consume but don't use.
            let mut is_negated = false;
            if self.check(&TokenType::Is) || self.check(&TokenType::Are) {
                self.advance(); // consume copula
                is_negated = self.check(&TokenType::Not);
                if is_negated {
                    self.advance();
                }
                // Consume the predicate adjective/verb (e.g., "asserted", "valid")
                let _ = self.consume_content_word();
                // Consume optional trailing "at any time"
                while self.check_preposition_is("at") {
                    self.advance();
                    if self.check(&TokenType::Any) {
                        self.advance();
                    }
                    if self.check_content_word() {
                        self.advance();
                    }
                }
            }

            // Build the body: each signal as an Atom, joined by OR
            // SVA synthesis maps Atom(sig) → signal name directly
            let mut signal_exprs: Vec<&'a LogicExpr<'a>> = Vec::new();
            for &sig in &signal_names {
                let atom: &'a LogicExpr<'a> = self.ctx.exprs.alloc(LogicExpr::Atom(sig));
                let sig_expr = if is_negated {
                    self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand: atom,
                    })
                } else {
                    atom
                };
                signal_exprs.push(sig_expr);
            }

            let body = if signal_exprs.len() == 1 {
                signal_exprs[0]
            } else {
                let mut combined = signal_exprs[0];
                for &expr in &signal_exprs[1..] {
                    combined = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: combined,
                        op: TokenType::Or,
                        right: expr,
                    });
                }
                combined
            };

            let kind = match quantifier_token {
                TokenType::AtMost(n) => QuantifierKind::AtMost(n),
                TokenType::AtLeast(n) => QuantifierKind::AtLeast(n),
                TokenType::Cardinal(n) => QuantifierKind::Cardinal(n),
                _ => unreachable!(),
            };

            self.in_negative_quantifier = was_in_negative_quantifier;

            return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind,
                variable: var_name,
                body,
                island_id: self.current_island,
            }));
        }

        let subject_pred = self.parse_restriction(var_name)?;

        // §5.3: stash the partitive superset now that the restriction predicate exists; the
        // `parse_quantified` wrapper turns it into a presupposed `∃=n x Restriction(x)`.
        if let Some(n) = partitive_superset {
            self.pending_partitive = Some((n, subject_pred, var_name));
        }

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
                        op: TokenType::Implies,
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
                                op: TokenType::Implies,
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
                            op: TokenType::Implies,
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
                        op: TokenType::Implies,
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
                                op: TokenType::Implies,
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
                            op: TokenType::Implies,
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

            // A polarity adverb between the auxiliary and the verb ("will EVER be
            // used", "would NEVER be seen") is an NPI licensed by the negative
            // quantifier — vacuous to the truth conditions here, so skip it.
            while self.check(&TokenType::Ever) {
                self.advance();
            }

            if self.check_verb() {
                let verb = self.consume_verb();

                // Passive auxiliary "will/would BE used": the copula `be`/`been` is
                // followed by a past-participle verb that is the real predicate — its
                // lemma fills the Theme ("will be USED" → Be(e) ∧ Theme(e, Use)),
                // mirroring the non-quantified passive path.
                let verb_lower = self.interner.resolve(verb).to_lowercase();
                let obj_term = if matches!(verb_lower.as_str(), "be" | "been")
                    && self.check_verb()
                {
                    Some(Term::Constant(self.consume_verb()))
                } else {
                    None
                };

                // Convert aux_time to modifier
                let mut modifiers = match aux_time {
                    Time::Past => vec![self.interner.intern("Past")],
                    Time::Future => vec![self.interner.intern("Future")],
                    _ => vec![],
                };
                // Manner adverbs after the participle ("were running quickly").
                modifiers.extend(self.collect_adverbs());

                // Frequency comparative "... used MORE THAN ONCE": Comparative +
                // "than" + count noun. Absorb it as an event modifier (the exact
                // count is immaterial to the bijection a grid eliminates).
                if self.check_comparative() {
                    self.advance(); // "more"
                    if self.check(&TokenType::Than) {
                        self.advance();
                    }
                    if self.check_content_word() || self.check_number() {
                        self.advance(); // the count word ("once", "twice", a number)
                    }
                    modifiers.push(self.interner.intern("MoreThanOnce"));
                }

                let verb_pred = self.build_verb_neo_event(verb, var_name, obj_term, modifiers);

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
                        op: TokenType::Implies,
                        right: maybe_negated,
                    }),
                    // "No N will/did V" = ∀x(N(x) → ¬V(x)). The aux branch was the
                    // sole VP path that dropped the negation for `No`, emitting the
                    // (much stronger, false) ∀x(N(x) ∧ V(x)).
                    TokenType::No => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: subject_pred,
                        op: TokenType::Implies,
                        right: self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                            op: TokenType::Not,
                            operand: maybe_negated,
                        }),
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
                    op: TokenType::Implies,
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
                        op: TokenType::Implies,
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

                    let mut obj_restriction: &'a LogicExpr<'a> = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: object.noun,
                        args: self.ctx.terms.alloc_slice([Term::Variable(obj_var)]),
                        world: None,
                    });
                    // The object's adjectives ("a RED book") and PP restrictors
                    // ("a maximum range OF 475 ft") are part of its description —
                    // dropping them is a meaning-loss parse.
                    for &adj in object.adjectives {
                        let adj_pred = self.adjective_restriction(adj, obj_var, object.noun);
                        obj_restriction = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: obj_restriction,
                            op: TokenType::And,
                            right: adj_pred,
                        });
                    }
                    for pp in object.pps {
                        let pp_sub = self.substitute_pp_placeholder(pp, obj_var);
                        obj_restriction = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: obj_restriction,
                            op: TokenType::And,
                            right: pp_sub,
                        });
                    }

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
                            op: TokenType::Implies,
                            right: verb_with_obj,
                        }),
                        TokenType::No => {
                            let neg = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                                op: TokenType::Not,
                                operand: verb_with_obj,
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
                            op: TokenType::Implies,
                            right: obj_quantified,
                        }),
                        TokenType::No => {
                            let neg = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                                op: TokenType::Not,
                                operand: obj_quantified,
                            });
                            self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                left: subject_pred,
                                op: TokenType::Implies,
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
                    let mut result = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                        kind: subj_kind,
                        variable: var_name,
                        body: subj_body,
                        island_id: self.current_island,
                    });
                    // Close any donkey-anaphora indefinites from the subject's
                    // relative clause (e.g. "a donkey" in "Every farmer who owns
                    // a donkey feeds every animal"). This quantified-object path
                    // previously returned WITHOUT the closure run by the main VP
                    // path, leaving the donkey variable FREE in the formula.
                    for (_noun, donkey_var, used, wide_neg) in self.donkey_bindings.iter().rev() {
                        if *used {
                            result = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                                kind: QuantifierKind::Universal,
                                variable: *donkey_var,
                                body: result,
                                island_id: self.current_island,
                            });
                        } else {
                            result = self.wrap_donkey_in_restriction(result, *donkey_var, *wide_neg);
                        }
                    }
                    self.donkey_bindings.clear();
                    return Ok(result);
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
                    op: TokenType::Implies,
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
                            op: TokenType::Implies,
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
                        op: TokenType::Implies,
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

        // Handle do-support: "every X does not hold" → ¬Hold(x)
        if self.check(&TokenType::Does) || self.check(&TokenType::Do) {
            self.advance(); // consume "does"/"do"
            let negative = self.match_token(&[TokenType::Not]);
            // The verb after "does not" becomes the predicate
            let verb_sym = self.consume_verb();
            let predicate_expr = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb_sym,
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
                    op: TokenType::Implies,
                    right: final_predicate,
                }),
                _ => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: subject_pred,
                    op: TokenType::And,
                    right: final_predicate,
                }),
            };

            let result = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind: match quantifier_token {
                    TokenType::All => QuantifierKind::Universal,
                    _ => QuantifierKind::Existential,
                },
                variable: var_name,
                body: body,
                island_id: self.current_island,
            });
            self.in_negative_quantifier = was_in_negative_quantifier;
            return Ok(result);
        }

        self.consume_copula()?;

        let negative = self.match_token(&[TokenType::Not]);

        // Copula PP complement under a quantifier: "Every trip is in Florida (or in
        // Maine)" → ∀x(Trip(x) → In(x,Florida) [∨ In(x,Maine)]) — the domain-closure
        // a logic-grid bijection eliminates over. A leading preposition makes
        // parse_noun_phrase fail, dropping the clause to a backtracking path that
        // loses the or-coordination AND any trailing PP, so handle the PP — with its
        // disjunction — directly here.
        let is_pp_complement = self.check_preposition() && !self.check_by_preposition();
        let mut final_predicate = if is_pp_complement {
            let pp = self.parse_copula_pp_complement(var_name)?;
            if negative {
                self.ctx.exprs.alloc(LogicExpr::UnaryOp { op: TokenType::Not, operand: pp })
            } else {
                pp
            }
        } else {
            let predicate_np = self.parse_noun_phrase(true)?;
            let predicate_expr = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: predicate_np.noun,
                args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
                world: None,
            });
            if negative {
                self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                    op: TokenType::Not,
                    operand: predicate_expr,
                })
            } else {
                predicate_expr
            }
        };

        // Disjunctive copula complement: "is red or blue", "is red, blue, or
        // green". Each disjunct is the same predicate form applied to the SAME
        // bound variable, so the `or` stays INSIDE the consequent (distributing
        // the variable) rather than being lifted to sentence level. A comma list
        // ("A, B, or C") coordinates with `or`; the trailing `or` before the
        // last item is optional after a comma. A `Comma` that is NOT part of such
        // a list (no following predicate complement) is left for the caller.
        // (The PP-complement case consumed its own or-coordination above.)
        while !is_pp_complement {
            let is_or = self.check(&TokenType::Or);
            let is_comma_list = self.check(&TokenType::Comma)
                && matches!(
                    self.tokens.get(self.current + 1).map(|t| &t.kind),
                    Some(TokenType::Adjective(_))
                        | Some(TokenType::Noun(_))
                        | Some(TokenType::ProperName(_))
                        | Some(TokenType::Or)
                );
            if !is_or && !is_comma_list {
                break;
            }
            if is_comma_list {
                self.advance(); // consume ","
                // "A, B, or C": drop the coordinating "or"/"and" after the comma.
                if self.check(&TokenType::Or) || self.check(&TokenType::And) {
                    self.advance();
                }
            } else {
                self.advance(); // consume "or"
            }

            let disj_negative = self.match_token(&[TokenType::Not]);
            let disj_np = self.parse_noun_phrase(true)?;
            let disj_pred: &'a LogicExpr<'a> = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: disj_np.noun,
                args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
                world: None,
            });
            let disj_pred = if disj_negative {
                self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                    op: TokenType::Not,
                    operand: disj_pred,
                })
            } else {
                disj_pred
            };
            final_predicate = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: final_predicate,
                op: TokenType::Or,
                right: disj_pred,
            });
        }

        let body = match quantifier_token {
            TokenType::All => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: subject_pred,
                op: TokenType::Implies,
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
                        op: TokenType::Implies,
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
                let neg = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                    op: TokenType::Not,
                    operand: final_predicate,
                });
                self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: subject_pred,
                    op: TokenType::Implies,
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

    /// Build the restriction conjunct contributed by a pre-nominal adjective,
    /// dispatching on the adjective's lexical class. This is the single shared
    /// site every NP-restriction path routes through, so the four classes are
    /// modeled identically everywhere (universal, indefinite, definite, copular):
    ///
    /// - **Relational / pertainymic** (lexicon `relational`): predicate of a
    ///   kind by default — `Rel(x, ^Base)` (no ∃) — or, at `level: Instance`,
    ///   an existential over a base-noun individual — `∃y(Base(y) ∧ Rel(x, y))`.
    ///   (McNally & Boleda 2004.)
    /// - **Subsective**: `Adj(x, ^Noun)` — graded against the head-noun kind.
    /// - **Intersective / other** (incl. NonIntersective, whose privative
    ///   meaning is supplied later by the axiom layer): flat `Adj(x)`.
    fn adjective_restriction(
        &mut self,
        adj: Symbol,
        var: Symbol,
        noun: Symbol,
    ) -> &'a LogicExpr<'a> {
        let adj_str = self.interner.resolve(adj).to_lowercase();

        if let Some((base, relation, level)) = lookup_relational_adjective(&adj_str) {
            let base_sym = self.interner.intern(base);
            let rel_sym = self.interner.intern(relation);
            if level == "Instance" {
                // ∃y( Base(y) ∧ Rel(x, y) )
                let y = self.next_var_name();
                let base_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: base_sym,
                    args: self.ctx.terms.alloc_slice([Term::Variable(y)]),
                    world: None,
                });
                let rel_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: rel_sym,
                    args: self
                        .ctx
                        .terms
                        .alloc_slice([Term::Variable(var), Term::Variable(y)]),
                    world: None,
                });
                let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: base_pred,
                    op: TokenType::And,
                    right: rel_pred,
                });
                return self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: QuantifierKind::Existential,
                    variable: y,
                    body,
                    island_id: self.current_island,
                });
            }
            // kind-level (default): Rel(x, ^Base)
            return self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: rel_sym,
                args: self
                    .ctx
                    .terms
                    .alloc_slice([Term::Variable(var), Term::Kind(base_sym)]),
                world: None,
            });
        }

        if is_subsective(&adj_str) {
            return self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: adj,
                args: self
                    .ctx
                    .terms
                    .alloc_slice([Term::Variable(var), Term::Intension(noun)]),
                world: None,
            });
        }

        self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: adj,
            args: self.ctx.terms.alloc_slice([Term::Variable(var)]),
            world: None,
        })
    }

    fn parse_restriction(&mut self, var_name: Symbol) -> ParseResult<&'a LogicExpr<'a>> {
        // Collect leading adjectives, then consume the head noun. The adjective
        // predicate forms (subsective `Adj(x, ^Noun)`, relational expansions)
        // need the head noun, so the noun is resolved before they are emitted.
        let mut adj_syms: Vec<Symbol> = Vec::new();

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
                    adj_syms.push(adj);
                }
            } else {
                break;
            }
        }

        let mut noun = self.consume_content_word()?;
        // Noun-noun compound restrictor ("Every chess game", "Each fire truck"):
        // join consecutive nouns into one symbol, mirroring parse_noun_phrase's
        // head compounding, so "Every chess game" doesn't strand "game".
        while let TokenType::Noun(next) = self.peek().kind {
            self.advance();
            noun = self.interner.intern(&format!(
                "{}_{}",
                self.interner.resolve(noun),
                self.interner.resolve(next)
            ));
        }

        let mut conditions: Vec<&'a LogicExpr<'a>> = Vec::new();
        for adj in &adj_syms {
            let adj_pred = self.adjective_restriction(*adj, var_name, noun);
            conditions.push(adj_pred);
        }
        conditions.push(self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: noun,
            args: self.ctx.terms.alloc_slice([Term::Variable(var_name)]),
            world: None,
        }));

        // PP post-modifiers on the restrictor head ("every dog IN the park", "no
        // option IN any category", "every member OF the team"): conjoin
        // `Prep(var, obj)` so the quantifier ranges over the PP-restricted set. In
        // restrictor position the VP has not begun, so a preposition with an NP
        // object is always a restrictor modifier; a following verb/modal/copula ends
        // the restriction and is left untouched.
        while self.check_preposition() {
            let object_follows = matches!(
                self.tokens.get(self.current + 1).map(|t| &t.kind),
                Some(TokenType::Article(_))
                    | Some(TokenType::Noun(_))
                    | Some(TokenType::ProperName(_))
                    | Some(TokenType::All)
                    | Some(TokenType::Some)
                    | Some(TokenType::Any)
                    | Some(TokenType::Cardinal(_))
                    | Some(TokenType::Number(_))
            );
            if !object_follows {
                break;
            }
            let prep_token = self.advance().clone();
            let prep_name = if let TokenType::Preposition(sym) = prep_token.kind {
                sym
            } else {
                self.current -= 1;
                break;
            };
            // Skip a leading quantifier determiner in the PP object ("in ANY
            // category", "in EACH group") — the restrictor PP names the sort, and
            // `parse_noun_phrase` does not consume a bare quantifier determiner.
            if matches!(
                self.peek().kind,
                TokenType::Any | TokenType::All | TokenType::Some | TokenType::No
                    | TokenType::Most | TokenType::Few | TokenType::Many
            ) && matches!(
                self.tokens.get(self.current + 1).map(|t| &t.kind),
                Some(TokenType::Noun(_)) | Some(TokenType::Adjective(_))
            ) {
                self.advance();
            }
            let pp_np = self.parse_noun_phrase(false)?;
            conditions.push(self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: prep_name,
                args: self
                    .ctx
                    .terms
                    .alloc_slice([Term::Variable(var_name), Term::Constant(pp_np.noun)]),
                world: None,
            }));
        }

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
        // A counting-NP object inside the relative clause ("who did 49 previous
        // jumps") becomes a ∃=n entity wrapping the whole restriction (see end).
        let mut object_cardinal: Option<(u32, crate::intern::Symbol)> = None;

        if self.check(&TokenType::Reflexive) {
            self.advance();
            args.push(Term::Variable(var_name));
        } else if let Some(n) = self.counting_np_lookahead() {
            // "who did 49 previous jumps" — a digit-led counting NP with an
            // adjective (or a deverbal-noun head): bind a fresh ∃=n entity that
            // is the Theme, predicating the noun + adjectives + PPs of it, so the
            // count and modifiers survive instead of collapsing to a measure that
            // eats the adjective.
            self.advance(); // the number
            let obj_var = self.next_var_name();
            self.nominal_np_context = true;
            let obj_np_result = self.parse_noun_phrase(false);
            self.nominal_np_context = false;
            let obj_np = obj_np_result?;
            let mut obj_restr: &'a LogicExpr<'a> = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: obj_np.noun,
                args: self.ctx.terms.alloc_slice([Term::Variable(obj_var)]),
                world: None,
            });
            for &adj in obj_np.adjectives {
                let adj_pred = self.adjective_restriction(adj, obj_var, obj_np.noun);
                obj_restr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: obj_restr,
                    op: TokenType::And,
                    right: adj_pred,
                });
            }
            for pp in obj_np.pps {
                let pp_sub = self.substitute_pp_placeholder(pp, obj_var);
                obj_restr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: obj_restr,
                    op: TokenType::And,
                    right: pp_sub,
                });
            }
            extra_conditions.push(obj_restr);
            args.push(Term::Variable(obj_var));
            object_cardinal = Some((n, obj_var));
        } else if self.check_number() {
            // "N unit NOUN" is a measure-PREMODIFIED noun ("requires 190 degree
            // WATER") — ONE folded entity (190_degree_water), not a measure with
            // the head stranded. A noun/ambiguous head TWO tokens past the number
            // (after the unit word) signals this; parse the whole NP (nominal
            // context so an ambiguous head folds). A pure finite VERB at +2 is the
            // MATRIX verb, not a head ("scored 190 points WON", "saw 6 manatees
            // WON") — those stay a bare measured object.
            let premodified_noun = matches!(
                self.tokens.get(self.current + 2).map(|t| &t.kind),
                Some(TokenType::Noun(_)) | Some(TokenType::Ambiguous { .. })
            );
            if premodified_noun {
                let saved_ctx = self.nominal_np_context;
                self.nominal_np_context = true;
                let np_result = self.parse_noun_phrase(false);
                self.nominal_np_context = saved_ctx;
                let np = np_result?;
                args.push(Term::Constant(np.noun));
            } else {
                // Measured object inside a relative clause ("that saw 6 manatees"):
                // the count is the Theme so the constraint isn't dropped.
                let measure = self.parse_measure_phrase()?;
                args.push(*measure);
            }
        } else if (self.check_content_word() || self.check_article())
            && (!self.check_verb() || {
                // A verb-headed object is normally NOT an object (avoids eating a
                // second verb), EXCEPT a gerund-noun compound ("used BOWLING
                // pins") where the -ing word is a pre-nominal modifier of the
                // following noun, so the whole thing is the noun object.
                matches!(
                    self.peek().kind,
                    TokenType::Verb { aspect: crate::lexicon::Aspect::Progressive, .. }
                ) && matches!(
                    self.tokens.get(self.current + 1).map(|t| &t.kind),
                    Some(TokenType::Noun(_))
                )
            })
        {
            if matches!(
                self.peek().kind,
                TokenType::Article(Definiteness::Indefinite)
            ) {
                // Parse the FULL object NP so a noun-noun compound folds ("a yoga
                // REGIMEN" → Yoga_regimen) instead of stranding the head — a single
                // consume_content_word() grabbed only the first word. Adjectives /
                // PPs are conjoined below (zero loss). NOTE: do NOT set
                // nominal_np_context here — in "Most farmers who own a donkey BEAT
                // it" the base-form NUCLEAR verb "beat" follows the object, and
                // nominal context would greedily fold it ("donkey_beat"), eating
                // the quantifier's nuclear scope. The Noun+Noun fold needs no flag.
                let obj_np = self.parse_noun_phrase(false)?;
                let noun = obj_np.noun;
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

                let mut obj_restr: &'a LogicExpr<'a> = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: noun,
                    args: self.ctx.terms.alloc_slice([Term::Variable(donkey_var)]),
                    world: None,
                });
                for &adj in obj_np.adjectives {
                    let adj_pred = self.adjective_restriction(adj, donkey_var, noun);
                    obj_restr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: obj_restr,
                        op: TokenType::And,
                        right: adj_pred,
                    });
                }
                for pp in obj_np.pps {
                    let pp_sub = self.substitute_pp_placeholder(pp, donkey_var);
                    obj_restr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: obj_restr,
                        op: TokenType::And,
                        right: pp_sub,
                    });
                }
                extra_conditions.push(obj_restr);

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
                    // The object's PP modifiers ("won the prize IN CHEMISTRY") restrict
                    // the object; lower them over the object constant so they are not
                    // dropped — the main-clause VP keeps them, this relative VP must too.
                    for pp in object.pps {
                        let pp_sub =
                            self.substitute_pp_self_term(pp, Term::Constant(object.noun));
                        extra_conditions.push(pp_sub);
                    }
                }
            }
        }

        while self.check_preposition() {
            let prep_sym = match self.peek().kind {
                TokenType::Preposition(s) => Some(s),
                _ => None,
            };
            self.advance();
            // After a direct object has been taken, a trailing PP modifies the EVENT
            // ("won the prize IN CHEMISTRY", "graduated FROM Yale"): the object stays
            // the Theme and the PP becomes a separate predicate over the event var
            // (the same one build_verb_neo_event uses). Previously these trailing PP
            // objects were pushed past args[1] and silently dropped by `obj_term =
            // args.remove(1)`, losing both the preposition and the object — the
            // main-clause VP keeps them, so the relative/quantified VP must too. With
            // no direct object yet, the legacy measured-arg behavior is preserved.
            let attach_to_event = prep_sym.is_some() && args.len() > 1;
            if self.check(&TokenType::Reflexive) {
                self.advance();
                args.push(Term::Variable(var_name));
            } else if self.check_number() {
                // Numeric PP object in a relative clause ("that cooks at 385
                // degrees", "that sell for $2.50"): keep the measured amount.
                let measure = self.parse_measure_phrase()?;
                if attach_to_event {
                    let ev = self.get_event_var();
                    extra_conditions.push(self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: prep_sym.unwrap(),
                        args: self.ctx.terms.alloc_slice([Term::Variable(ev), *measure]),
                        world: None,
                    }));
                } else {
                    args.push(*measure);
                }
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
                } else if attach_to_event {
                    let ev = self.get_event_var();
                    extra_conditions.push(self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: prep_sym.unwrap(),
                        args: self
                            .ctx
                            .terms
                            .alloc_slice([Term::Variable(ev), Term::Constant(object.noun)]),
                        world: None,
                    }));
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

        let mut restriction = if extra_conditions.is_empty() {
            verb_pred
        } else {
            extra_conditions.push(verb_pred);
            self.combine_with_and(extra_conditions)?
        };

        // A trailing positional/temporal adverb inside the relative clause ("the
        // person who went FIRST", "the racer who finished LAST") anchors the
        // event's ordinal position — the same TemporalAnchor the main clause
        // builds. Without this it strands and the relative clause fails to close.
        if self.check_temporal_adverb() {
            if let TokenType::TemporalAdverb(anchor) = self.advance().kind.clone() {
                restriction = self.ctx.exprs.alloc(LogicExpr::TemporalAnchor {
                    anchor,
                    body: restriction,
                });
            }
        }

        // A counting-NP object binds the relative-clause body as ∃=n: "the
        // skydiver who did 49 previous jumps" → Skydiver(s) ∧ ∃=49 j(Jump(j) ∧
        // Previous(j) ∧ event(s, j)). The object quantifier scopes inside the
        // subject's own binding (the caller conjoins the head-noun predicate).
        if let Some((n, obj_var)) = object_cardinal {
            restriction = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind: QuantifierKind::Cardinal(n),
                variable: obj_var,
                body: restriction,
                island_id: self.current_island,
            });
        }

        Ok(restriction)
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
        let result = if let Some(adj) = np.superlative {
            let superlative_expr = self.ctx.exprs.alloc(LogicExpr::Superlative {
                adjective: adj,
                subject: self.ctx.terms.alloc(Term::Constant(np.noun)),
                domain: np.noun,
            });
            self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: result,
                op: TokenType::And,
                right: superlative_expr,
            })
        } else {
            result
        };

        // A possessive definite carries an existence presupposition (§8.2):
        // "His children are happy." ≫ He has children. The presupposed
        // possession is a real event — the same shape "He has children."
        // parses to — so it is derivable, not just printable.
        if let Some(possessor) = np.possessor {
            use crate::ast::logic::NeoEventData;
            use crate::ast::ThematicRole;
            let possessed = Term::Constant(np.noun);
            // A DESCRIPTIVE possessor ("the Woodard family", "the red team")
            // carries its own restrictor; collapsing it to the bare head silently
            // drops the modifier ("Woodard"/"red"). `possessor_entity` lifts it to
            // its own existential entity carrying the full restrictor, so the
            // possession event's Agent is that entity, not a stripped constant:
            //   ∃p(Restrictor(p) ∧ ∃e(Have(e) ∧ Agent(e,p) ∧ Theme(e,possessed))).
            // A bare possessor ("Agnes", "His") keeps the constant-Agent form.
            let (agent_term, agent_restr) = self.possessor_entity(possessor);
            let have = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                event_var: self.interner.intern("e"),
                verb: self.interner.intern("Have"),
                roles: self.ctx.roles.alloc_slice(vec![
                    (ThematicRole::Agent, agent_term),
                    (ThematicRole::Theme, possessed),
                ]),
                modifiers: self.ctx.syms.alloc_slice(vec![]),
                suppress_existential: false,
                world: None,
            })));
            let presupposition = self.wrap_in_possessor_entity(agent_restr, have);
            return Ok(self.ctx.exprs.alloc(LogicExpr::Presupposition {
                assertion: result,
                presupposition,
            }));
        }

        Ok(result)
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
        // Fold in a stashed subject relative-clause restriction (expressed over
        // `Constant(noun)`): conjoining it onto the predicate means every
        // definiteness branch's constant→variable substitution re-binds it to
        // the same entity, so the relative clause is never silently dropped.
        let predicate = match self.pending_subject_restriction {
            Some((restr_noun, restr)) if restr_noun == noun => {
                self.pending_subject_restriction = None;
                self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: restr,
                    op: TokenType::And,
                    right: predicate,
                })
            }
            _ => predicate,
        };
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
                    let adj_pred = self.adjective_restriction(*adj, var, noun);
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

                    let mut substituted =
                        self.substitute_constant_with_sigma(predicate, noun, sigma_term)?;

                    // A definite plural's adjectives and PPs restrict the σ-term
                    // — "The species from Australia won." keeps From(σSpecies,
                    // Australia) instead of dropping the origin constraint.
                    for adj in adjectives {
                        let adj_pred = self.adjective_restriction(*adj, singular_sym, noun);
                        let adj_on_sigma =
                            self.substitute_constant_with_sigma(adj_pred, singular_sym, sigma_term)?;
                        substituted = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: adj_on_sigma,
                            op: TokenType::And,
                            right: substituted,
                        });
                    }
                    let placeholder = self.interner.intern("_PP_SELF_");
                    for pp in pps {
                        let pp_sub = match pp {
                            LogicExpr::Predicate { name, args, world } => {
                                let new_args: Vec<Term<'a>> = args
                                    .iter()
                                    .map(|a| match a {
                                        Term::Variable(v) if *v == placeholder => sigma_term,
                                        other => *other,
                                    })
                                    .collect();
                                self.ctx.exprs.alloc(LogicExpr::Predicate {
                                    name: *name,
                                    args: self.ctx.terms.alloc_slice(new_args),
                                    world: *world,
                                })
                            }
                            other => *other,
                        };
                        substituted = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: substituted,
                            op: TokenType::And,
                            right: pp_sub,
                        });
                    }

                    let verb_name = self.find_main_verb_name(predicate);
                    let is_collective = verb_name
                        .map(|v| {
                            let lemma = self.interner.resolve(v);
                            Lexer::is_collective_verb(lemma)
                                // A mixed verb ("lift", "carry") defaults to
                                // the COLLECTIVE reading for a plural definite
                                // ("the boys lifted the piano" — together);
                                // a floated "all/each" forces distribution.
                                || (Lexer::is_mixed_verb(lemma) && !self.distributive_marker)
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
                    // Van der Sandt: a definite BINDS to an accessible
                    // antecedent before accommodating — a re-mentioned "the
                    // kettle" reuses the referent instead of asserting a
                    // second Russell expansion (existence + uniqueness).
                    if adjectives.is_empty() && pps.is_empty() {
                        if let Some(prior) =
                            self.drs.resolve_definite(self.drs.current_box_index(), noun)
                        {
                            if prior == noun {
                                return Ok(predicate);
                            }
                            return self.substitute_constant_with_var_sym(
                                predicate, noun, prior,
                            );
                        }
                    }

                    // Context-driven coreference for a MODIFIED definite whose
                    // head noun did not already resolve: "the hunting trip"
                    // binds to a prior "the hunting vacation" because the
                    // distinguishing modifier (Hunt) matches and trip/vacation
                    // are sort-compatible occasions. The modifier does the
                    // referring; the head noun is a soft type. No synonymy
                    // axiom is asserted — the later description simply reuses
                    // the earlier referent's variable.
                    if !adjectives.is_empty() && pps.is_empty() {
                        if let Some(prior) = self.drs.resolve_definite_by_modifier(
                            self.interner,
                            self.drs.current_box_index(),
                            noun,
                            adjectives,
                        ) {
                            return self.substitute_constant_with_var_sym(
                                predicate, noun, prior,
                            );
                        }
                    }

                    // In DISCOURSE mode an unbound, non-bridging definite is
                    // SKOLEMIZED: Russell's uniqueness licenses a witness
                    // constant, so cross-sentence mentions stay rigidly
                    // linked — `Barber(B) ∧ ∀y(Barber(y) → y = B) ∧ P(B)`
                    // (the very form the proof engine's definite-description
                    // machinery expects) — never an ∃ whose bound variable
                    // later premises cannot reach.
                    if self.world_state.in_discourse_mode()
                        && adjectives.is_empty()
                        && pps.is_empty()
                        && self.drs.resolve_bridging(self.interner, noun).is_none()
                    {
                        let y = self.next_var_name();
                        let gender = Self::infer_noun_gender(self.interner.resolve(noun));
                        let number = if Self::is_plural_noun(self.interner.resolve(noun)) {
                            Number::Plural
                        } else {
                            Number::Singular
                        };
                        // Rigid (skolem) referent: later definites and
                        // pronouns resolve to the constant itself.
                        self.drs.introduce_referent_with_source(
                            noun,
                            noun,
                            gender,
                            number,
                            ReferentSource::ProperName,
                        );
                        let restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: noun,
                            args: self.ctx.terms.alloc_slice([Term::Constant(noun)]),
                            world: None,
                        });
                        let y_restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: noun,
                            args: self.ctx.terms.alloc_slice([Term::Variable(y)]),
                            world: None,
                        });
                        let identity = self.ctx.exprs.alloc(LogicExpr::Identity {
                            left: self.ctx.terms.alloc(Term::Variable(y)),
                            right: self.ctx.terms.alloc(Term::Constant(noun)),
                        });
                        let uniqueness = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                            kind: QuantifierKind::Universal,
                            variable: y,
                            body: self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                left: y_restriction,
                                op: TokenType::Implies,
                                right: identity,
                            }),
                            island_id: self.current_island,
                        });
                        let described = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: restriction,
                            op: TokenType::And,
                            right: uniqueness,
                        });
                        return Ok(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: described,
                            op: TokenType::And,
                            right: predicate,
                        }));
                    }

                    let x = self.next_var_name();
                    let y = self.next_var_name();

                    let mut restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: noun,
                        args: self.ctx.terms.alloc_slice([Term::Variable(x)]),
                        world: None,
                    });

                    for adj in adjectives {
                        let adj_pred = self.adjective_restriction(*adj, x, noun);
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
                    // Carry the distinguishing modifier(s) so a later definite
                    // ("the hunting trip") can corefer to this one ("the
                    // hunting vacation") through a shared modifier + compatible
                    // occasion sort.
                    self.drs.introduce_referent_with_modifiers(
                        x,
                        noun,
                        gender,
                        number,
                        ReferentSource::MainClause,
                        adjectives.to_vec(),
                    );

                    let mut y_restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: noun,
                        args: self.ctx.terms.alloc_slice([Term::Variable(y)]),
                        world: None,
                    });
                    for adj in adjectives {
                        let adj_pred = self.adjective_restriction(*adj, y, noun);
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
                        op: TokenType::Implies,
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
                    let adj_pred = self.adjective_restriction(*adj, var, noun);
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
                    op: TokenType::Implies,
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
            // Recurse through connectives / quantifiers / events so a complex
            // restrictor (a reduced relative built as a NeoEvent with its gap in a
            // thematic role, plus event complements) binds its `_PP_SELF_` gap to
            // the NP's variable wherever it sits.
            LogicExpr::BinaryOp { left, op, right } => {
                let l = self.substitute_pp_placeholder(left, var);
                let r = self.substitute_pp_placeholder(right, var);
                self.ctx.exprs.alloc(LogicExpr::BinaryOp { left: l, op: op.clone(), right: r })
            }
            LogicExpr::UnaryOp { op, operand } => {
                let o = self.substitute_pp_placeholder(operand, var);
                self.ctx.exprs.alloc(LogicExpr::UnaryOp { op: op.clone(), operand: o })
            }
            LogicExpr::Quantifier { kind, variable, body, island_id } => {
                let b = self.substitute_pp_placeholder(body, var);
                self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: *kind,
                    variable: *variable,
                    body: b,
                    island_id: *island_id,
                })
            }
            LogicExpr::Temporal { operator, body } => {
                let b = self.substitute_pp_placeholder(body, var);
                self.ctx.exprs.alloc(LogicExpr::Temporal { operator: *operator, body: b })
            }
            LogicExpr::NeoEvent(data) => {
                let new_roles: Vec<(ThematicRole, Term<'a>)> = data
                    .roles
                    .iter()
                    .map(|(role, term)| {
                        let new_term = match term {
                            Term::Variable(v) if *v == placeholder => Term::Variable(var),
                            other => *other,
                        };
                        (*role, new_term)
                    })
                    .collect();
                self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var: data.event_var,
                    verb: data.verb,
                    roles: self.ctx.roles.alloc_slice(new_roles),
                    modifiers: data.modifiers,
                    suppress_existential: data.suppress_existential,
                    world: data.world,
                })))
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
                        Term::Kind(k) => Term::Kind(*k),
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
                            Term::Kind(k) => Term::Kind(*k),
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
            LogicExpr::Control { verb, subject, object, infinitive } => {
                let sub_term = |t: &Term<'a>| -> Term<'a> {
                    match t {
                        Term::Constant(c) if *c == constant_name => Term::Variable(var_name),
                        other => other.clone(),
                    }
                };
                Ok(self.ctx.exprs.alloc(LogicExpr::Control {
                    verb: *verb,
                    subject: self.ctx.terms.alloc(sub_term(subject)),
                    object: match object {
                        Some(o) => Some(&*self.ctx.terms.alloc(sub_term(o))),
                        None => None,
                    },
                    infinitive: self.substitute_constant_with_var(infinitive, constant_name, var_name)?,
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

    fn substitute_variable_with_constant(
        &self,
        expr: &'a LogicExpr<'a>,
        from_var: Symbol,
        to_const: Symbol,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        let map_term = |t: &Term<'a>| -> Term<'a> {
            match t {
                Term::Variable(v) if *v == from_var => Term::Constant(to_const),
                other => other.clone(),
            }
        };
        match expr {
            LogicExpr::Predicate { name, args, world } => {
                let new_args: Vec<Term<'a>> = args.iter().map(&map_term).collect();
                Ok(self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: *name,
                    args: self.ctx.terms.alloc_slice(new_args),
                    world: *world,
                }))
            }
            LogicExpr::Identity { left, right } => Ok(self.ctx.exprs.alloc(LogicExpr::Identity {
                left: self.ctx.terms.alloc(map_term(left)),
                right: self.ctx.terms.alloc(map_term(right)),
            })),
            LogicExpr::Temporal { operator, body } => Ok(self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: *operator,
                body: self.substitute_variable_with_constant(body, from_var, to_const)?,
            })),
            LogicExpr::Aspectual { operator, body } => {
                Ok(self.ctx.exprs.alloc(LogicExpr::Aspectual {
                    operator: *operator,
                    body: self.substitute_variable_with_constant(body, from_var, to_const)?,
                }))
            }
            LogicExpr::UnaryOp { op, operand } => Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: op.clone(),
                operand: self.substitute_variable_with_constant(operand, from_var, to_const)?,
            })),
            LogicExpr::BinaryOp { left, op, right } => Ok(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: self.substitute_variable_with_constant(left, from_var, to_const)?,
                op: op.clone(),
                right: self.substitute_variable_with_constant(right, from_var, to_const)?,
            })),
            LogicExpr::Event { predicate, adverbs } => Ok(self.ctx.exprs.alloc(LogicExpr::Event {
                predicate: self.substitute_variable_with_constant(predicate, from_var, to_const)?,
                adverbs: *adverbs,
            })),
            LogicExpr::TemporalAnchor { anchor, body } => {
                Ok(self.ctx.exprs.alloc(LogicExpr::TemporalAnchor {
                    anchor: *anchor,
                    body: self.substitute_variable_with_constant(body, from_var, to_const)?,
                }))
            }
            LogicExpr::NeoEvent(data) => {
                let new_roles: Vec<(crate::ast::ThematicRole, Term<'a>)> =
                    data.roles.iter().map(|(role, term)| (*role, map_term(term))).collect();
                Ok(self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(crate::ast::NeoEventData {
                    event_var: data.event_var,
                    verb: data.verb,
                    roles: self.ctx.roles.alloc_slice(new_roles),
                    modifiers: data.modifiers,
                    suppress_existential: data.suppress_existential,
                    world: None,
                }))))
            }
            LogicExpr::Quantifier { kind, variable, body, island_id } => {
                // A nested quantifier that re-binds `from_var` shadows the gap;
                // leave its body untouched in that case.
                if *variable == from_var {
                    return Ok(expr);
                }
                Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: *kind,
                    variable: *variable,
                    body: self.substitute_variable_with_constant(body, from_var, to_const)?,
                    island_id: *island_id,
                }))
            }
            LogicExpr::Control { verb, subject, object, infinitive } => {
                Ok(self.ctx.exprs.alloc(LogicExpr::Control {
                    verb: *verb,
                    subject: self.ctx.terms.alloc(map_term(subject)),
                    object: object.map(|o| &*self.ctx.terms.alloc(map_term(o))),
                    infinitive: self.substitute_variable_with_constant(infinitive, from_var, to_const)?,
                }))
            }
            _ => Ok(expr),
        }
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
                        Term::Kind(k) => Term::Kind(*k),
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
                            Term::Kind(k) => Term::Kind(*k),
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
    /// Is the parser at a position where a finished clause may legitimately
    /// end? Used to detect under-consumption by specialized sub-grammars.
    pub(super) fn at_clause_boundary(&self) -> bool {
        matches!(
            self.peek().kind,
            TokenType::Period
                | TokenType::Exclamation
                | TokenType::EOF
                | TokenType::Comma
                | TokenType::RParen
                | TokenType::And
                | TokenType::Or
                | TokenType::Iff
                | TokenType::Then
        )
    }

    /// Quantified clause whose VP is parsed by the FULL predicate parser with
    /// the subject bound to the quantified variable. The under-consumption
    /// fallback for frames the specialized quantified grammar does not cover
    /// (reflexives, reciprocals, control infinitives, ditransitives,
    /// comparatives, adverbs, …).
    fn parse_quantified_delegating(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        use super::verb::LogicVerbParsing;

        let quantifier_token = self.previous().kind.clone();
        let var_name = self.next_var_name();

        let was_in_negative_quantifier = self.in_negative_quantifier;
        if matches!(quantifier_token, TokenType::No) {
            self.in_negative_quantifier = true;
        }

        let restriction = self.parse_restriction(var_name)?;

        // Reciprocal VP ("helped each other"): every pair of distinct members
        // of the restriction stands in the relation —
        // ∀y((R(y) ∧ ¬(y = x)) → ∃e(V(e) ∧ Agent(e, x) ∧ Theme(e, y))).
        let mut copula_vp: Option<&'a LogicExpr<'a>> = None;
        if matches!(self.peek().kind, TokenType::Verb { .. })
            && matches!(
                self.tokens.get(self.current + 1).map(|t| t.kind.clone()),
                Some(TokenType::Reciprocal)
            )
        {
            let (verb, recip_time, _, _) = self.consume_verb_with_metadata();
            self.advance(); // the reciprocal
            let other_var = self.next_var_name();
            let other_restriction = self.rename_var_in_expr(restriction, var_name, other_var);
            let distinct = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: self.ctx.exprs.alloc(LogicExpr::Identity {
                    left: self.ctx.terms.alloc(Term::Variable(other_var)),
                    right: self.ctx.terms.alloc(Term::Variable(var_name)),
                }),
            });
            let modifiers = match recip_time {
                Time::Past => vec![self.interner.intern("Past")],
                Time::Future => vec![self.interner.intern("Future")],
                _ => vec![],
            };
            let event_var = self.get_event_var();
            let suppress_existential = self.drs.in_conditional_antecedent();
            let event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                event_var,
                verb,
                roles: self.ctx.roles.alloc_slice(vec![
                    (ThematicRole::Agent, Term::Variable(var_name)),
                    (ThematicRole::Theme, Term::Variable(other_var)),
                ]),
                modifiers: self.ctx.syms.alloc_slice(modifiers),
                suppress_existential,
                world: None,
            })));
            let antecedent = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: other_restriction,
                op: TokenType::And,
                right: distinct,
            });
            let pair_body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: antecedent,
                op: TokenType::Implies,
                right: event,
            });
            copula_vp = Some(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind: QuantifierKind::Universal,
                variable: other_var,
                body: pair_body,
                island_id: self.current_island,
            }));
        }

        // Copula-initial VP. A progressive participle keeps the full predicate
        // parser (the copula only carries tense: "were running quickly"); a
        // passive participle predicates the subject variable as Theme with an
        // optional by-phrase Agent ("were read by some students").
        if matches!(
            self.peek().kind,
            TokenType::Is | TokenType::Are | TokenType::Was | TokenType::Were
        ) {
            let copula_past = matches!(self.peek().kind, TokenType::Was | TokenType::Were);
            if let Some(TokenType::Verb { aspect, .. }) =
                self.tokens.get(self.current + 1).map(|t| t.kind.clone())
            {
                if aspect == crate::lexicon::Aspect::Progressive {
                    self.advance(); // copula carries tense only
                    self.pending_time =
                        Some(if copula_past { Time::Past } else { Time::None });
                } else {
                    self.advance(); // copula
                    let (verb, _, _, _) = self.consume_verb_with_metadata();
                    let mut modifiers = if copula_past {
                        vec![self.interner.intern("Past")]
                    } else {
                        vec![]
                    };
                    modifiers.extend(self.collect_adverbs());

                    let mut roles = vec![(ThematicRole::Theme, Term::Variable(var_name))];
                    let mut agent_quant: Option<(TokenType, Symbol, Symbol)> = None;
                    if self.check_preposition_is("by") {
                        self.advance(); // by
                        if self.check_quantifier() {
                            let q = self.advance().kind.clone();
                            let a_np = self.parse_noun_phrase(false)?;
                            let a_var = self.next_var_name();
                            roles.push((ThematicRole::Agent, Term::Variable(a_var)));
                            agent_quant = Some((q, a_var, a_np.noun));
                        } else if self.check_content_word() || self.check_article() {
                            let a_np = self.parse_noun_phrase(false)?;
                            roles.push((ThematicRole::Agent, Term::Constant(a_np.noun)));
                        }
                    }

                    let event_var = self.get_event_var();
                    let suppress_existential = self.drs.in_conditional_antecedent();
                    let passive = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(
                        NeoEventData {
                            event_var,
                            verb,
                            roles: self.ctx.roles.alloc_slice(roles),
                            modifiers: self.ctx.syms.alloc_slice(modifiers),
                            suppress_existential,
                            world: None,
                        },
                    )));
                    copula_vp = Some(if let Some((q, a_var, a_noun)) = agent_quant {
                        let a_restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: a_noun,
                            args: self.ctx.terms.alloc_slice([Term::Variable(a_var)]),
                            world: None,
                        });
                        let (a_kind, a_op) = match q {
                            TokenType::All => (QuantifierKind::Universal, TokenType::Implies),
                            TokenType::Most => (QuantifierKind::Most, TokenType::And),
                            TokenType::Few => (QuantifierKind::Few, TokenType::And),
                            TokenType::Many => (QuantifierKind::Many, TokenType::And),
                            _ => (QuantifierKind::Existential, TokenType::And),
                        };
                        let a_body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: a_restriction,
                            op: a_op,
                            right: passive,
                        });
                        self.ctx.exprs.alloc(LogicExpr::Quantifier {
                            kind: a_kind,
                            variable: a_var,
                            body: a_body,
                            island_id: self.current_island,
                        })
                    } else {
                        passive
                    });
                }
            }
        }

        let vp = match copula_vp {
            Some(vp) => vp,
            None => self.parse_predicate_with_subject_as_var(var_name)?,
        };
        // Sentence-final temporal anchor ("studied yesterday") — the same
        // wrapping the simple-subject clause path applies.
        let vp = if self.check_temporal_adverb() {
            if let TokenType::TemporalAdverb(anchor) = self.advance().kind.clone() {
                &*self.ctx.exprs.alloc(LogicExpr::TemporalAnchor { anchor, body: vp })
            } else {
                vp
            }
        } else {
            vp
        };
        self.in_negative_quantifier = was_in_negative_quantifier;

        // "No N VP" denies the VP of every N.
        let consequent = if matches!(quantifier_token, TokenType::No) {
            self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: vp,
            })
        } else {
            vp
        };

        let universal_frame = matches!(
            quantifier_token,
            TokenType::All | TokenType::Any | TokenType::No
        );
        let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
            left: restriction,
            op: if universal_frame { TokenType::Implies } else { TokenType::And },
            right: consequent,
        });

        // Donkey closure: an indefinite introduced inside the restriction and
        // picked up by the VP ("Every man who owns a book gives it …") is a
        // free variable in both — close it at the quantifier (universal force
        // under a universal frame, existential otherwise).
        let mut restriction_vars = Vec::new();
        self.collect_unbound_vars(restriction, &mut vec![var_name], &mut restriction_vars);
        let mut body = body;
        for donkey_var in restriction_vars {
            if self.expr_mentions_var(consequent, donkey_var) {
                body = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: if universal_frame {
                        QuantifierKind::Universal
                    } else {
                        QuantifierKind::Existential
                    },
                    variable: donkey_var,
                    body,
                    island_id: self.current_island,
                });
            }
        }

        let kind = match quantifier_token {
            TokenType::All | TokenType::Any | TokenType::No => QuantifierKind::Universal,
            TokenType::Some => QuantifierKind::Existential,
            TokenType::Most => QuantifierKind::Most,
            TokenType::Few => QuantifierKind::Few,
            TokenType::Many => QuantifierKind::Many,
            TokenType::Cardinal(n) => QuantifierKind::Cardinal(n),
            TokenType::AtLeast(n) => QuantifierKind::AtLeast(n),
            TokenType::AtMost(n) => QuantifierKind::AtMost(n),
            _ => QuantifierKind::Universal,
        };

        Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
            kind,
            variable: var_name,
            body,
            island_id: self.current_island,
        }))
    }

    /// Rebuild `expr` with free occurrences of variable `from` renamed to
    /// `to`; binders that shadow `from` keep their bodies untouched.
    fn rename_var_in_expr(
        &self,
        expr: &'a LogicExpr<'a>,
        from: Symbol,
        to: Symbol,
    ) -> &'a LogicExpr<'a> {
        let rename_term = |t: &Term<'a>| -> Term<'a> {
            match t {
                Term::Variable(v) if *v == from => Term::Variable(to),
                other => other.clone(),
            }
        };
        match expr {
            LogicExpr::Predicate { name, args, world } => {
                let new_args: Vec<Term<'a>> = args.iter().map(|a| rename_term(a)).collect();
                self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: *name,
                    args: self.ctx.terms.alloc_slice(new_args),
                    world: world.clone(),
                })
            }
            LogicExpr::Identity { left, right } => self.ctx.exprs.alloc(LogicExpr::Identity {
                left: self.ctx.terms.alloc(rename_term(left)),
                right: self.ctx.terms.alloc(rename_term(right)),
            }),
            LogicExpr::NeoEvent(data) => {
                let new_roles: Vec<(ThematicRole, Term<'a>)> = data
                    .roles
                    .iter()
                    .map(|(role, term)| (*role, rename_term(term)))
                    .collect();
                self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var: data.event_var,
                    verb: data.verb,
                    roles: self.ctx.roles.alloc_slice(new_roles),
                    modifiers: data.modifiers,
                    suppress_existential: data.suppress_existential,
                    world: data.world.clone(),
                })))
            }
            LogicExpr::BinaryOp { left, op, right } => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: self.rename_var_in_expr(left, from, to),
                op: op.clone(),
                right: self.rename_var_in_expr(right, from, to),
            }),
            LogicExpr::UnaryOp { op, operand } => self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: op.clone(),
                operand: self.rename_var_in_expr(operand, from, to),
            }),
            LogicExpr::Quantifier { kind, variable, body, island_id } if *variable != from => {
                self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: *kind,
                    variable: *variable,
                    body: self.rename_var_in_expr(body, from, to),
                    island_id: *island_id,
                })
            }
            LogicExpr::Temporal { operator, body } => self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: *operator,
                body: self.rename_var_in_expr(body, from, to),
            }),
            LogicExpr::Aspectual { operator, body } => {
                self.ctx.exprs.alloc(LogicExpr::Aspectual {
                    operator: *operator,
                    body: self.rename_var_in_expr(body, from, to),
                })
            }
            LogicExpr::Event { predicate, adverbs } => self.ctx.exprs.alloc(LogicExpr::Event {
                predicate: self.rename_var_in_expr(predicate, from, to),
                adverbs: *adverbs,
            }),
            other => other,
        }
    }

    /// Collect variables that occur free in `expr` (not bound by an inner
    /// quantifier and not in `bound`), in first-occurrence order.
    fn collect_unbound_vars(
        &self,
        expr: &LogicExpr<'a>,
        bound: &mut Vec<Symbol>,
        out: &mut Vec<Symbol>,
    ) {
        fn term_vars(term: &Term<'_>, bound: &[Symbol], out: &mut Vec<Symbol>) {
            match term {
                Term::Variable(v) => {
                    if !bound.contains(v) && !out.contains(v) {
                        out.push(*v);
                    }
                }
                Term::Function(_, args) => {
                    for t in args.iter() {
                        term_vars(t, bound, out);
                    }
                }
                _ => {}
            }
        }

        match expr {
            LogicExpr::Predicate { args, .. } => {
                for t in args.iter() {
                    term_vars(t, bound, out);
                }
            }
            LogicExpr::NeoEvent(data) => {
                for (_, t) in data.roles.iter() {
                    term_vars(t, bound, out);
                }
            }
            LogicExpr::BinaryOp { left, right, .. } => {
                self.collect_unbound_vars(left, bound, out);
                self.collect_unbound_vars(right, bound, out);
            }
            LogicExpr::UnaryOp { operand, .. } => self.collect_unbound_vars(operand, bound, out),
            LogicExpr::Quantifier { variable, body, .. } => {
                bound.push(*variable);
                self.collect_unbound_vars(body, bound, out);
                bound.pop();
            }
            LogicExpr::Temporal { body, .. } => self.collect_unbound_vars(body, bound, out),
            LogicExpr::Aspectual { body, .. } => self.collect_unbound_vars(body, bound, out),
            LogicExpr::Event { predicate, .. } => self.collect_unbound_vars(predicate, bound, out),
            LogicExpr::Modal { operand, .. } => self.collect_unbound_vars(operand, bound, out),
            LogicExpr::Scopal { body, .. } => self.collect_unbound_vars(body, bound, out),
            _ => {}
        }
    }

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
        if let LogicExpr::BinaryOp { left, op: TokenType::Implies, right } = body {
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
                op: TokenType::Implies,
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
            op: TokenType::Implies,
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
