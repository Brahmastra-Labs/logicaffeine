use super::modal::ModalParsing;
use super::noun::NounParsing;
use super::pragmatics::PragmaticsParsing;
use super::quantifier::QuantifierParsing;
use super::question::QuestionParsing;
use super::verb::LogicVerbParsing;
use super::{ParseResult, Parser};
use crate::ast::{LogicExpr, NeoEventData, NounPhrase, Term, ThematicRole};
use crate::error::{ParseError, ParseErrorKind};
use crate::intern::Symbol;
use crate::token::TokenType;

pub trait ClauseParsing<'a, 'ctx, 'int> {
    fn parse_sentence(&mut self) -> ParseResult<&'a LogicExpr<'a>>;
    fn parse_conditional(&mut self) -> ParseResult<&'a LogicExpr<'a>>;
    fn parse_disjunction(&mut self) -> ParseResult<&'a LogicExpr<'a>>;
    fn parse_conjunction(&mut self) -> ParseResult<&'a LogicExpr<'a>>;
    fn parse_relative_clause(&mut self, gap_var: Symbol) -> ParseResult<&'a LogicExpr<'a>>;
    fn parse_gapped_clause(&mut self, borrowed_verb: Symbol) -> ParseResult<&'a LogicExpr<'a>>;
    fn parse_counterfactual_antecedent(&mut self) -> ParseResult<&'a LogicExpr<'a>>;
    fn parse_counterfactual_consequent(&mut self) -> ParseResult<&'a LogicExpr<'a>>;
    fn check_wh_word(&self) -> bool;
    fn is_counterfactual_context(&self) -> bool;
    fn is_complete_clause(&self, expr: &LogicExpr<'a>) -> bool;
    fn extract_verb_from_expr(&self, expr: &LogicExpr<'a>) -> Option<Symbol>;
    fn try_parse_ellipsis(&mut self) -> Option<ParseResult<&'a LogicExpr<'a>>>;
    fn check_ellipsis_auxiliary(&self) -> bool;
    fn check_ellipsis_terminator(&self) -> bool;
}

impl<'a, 'ctx, 'int> ClauseParsing<'a, 'ctx, 'int> for Parser<'a, 'ctx, 'int> {
    fn parse_sentence(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        // Check for ellipsis pattern: "Mary does too." / "Mary can too."
        if let Some(result) = self.try_parse_ellipsis() {
            return result;
        }

        if self.check_verb() {
            let verb_pos = self.current;
            let mut temp_pos = self.current + 1;
            while temp_pos < self.tokens.len() {
                if matches!(self.tokens[temp_pos].kind, TokenType::Exclamation) {
                    self.current = verb_pos;
                    let verb = self.consume_verb();
                    while !matches!(self.peek().kind, TokenType::Exclamation | TokenType::EOF) {
                        self.advance();
                    }
                    if self.check(&TokenType::Exclamation) {
                        self.advance();
                    }
                    let addressee = self.interner.intern("addressee");
                    let action = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: verb,
                        args: self.ctx.terms.alloc_slice([Term::Variable(addressee)]),
                    });
                    return Ok(self.ctx.exprs.alloc(LogicExpr::Imperative { action }));
                }
                if matches!(self.tokens[temp_pos].kind, TokenType::Period | TokenType::EOF) {
                    break;
                }
                temp_pos += 1;
            }
        }

        if self.check_wh_word() {
            return self.parse_wh_question();
        }

        if self.check(&TokenType::Does)
            || self.check(&TokenType::Do)
            || self.check(&TokenType::Is)
            || self.check(&TokenType::Are)
            || self.check(&TokenType::Was)
            || self.check(&TokenType::Were)
            || self.check(&TokenType::Would)
            || self.check(&TokenType::Could)
            || self.check(&TokenType::Can)
        {
            return self.parse_yes_no_question();
        }

        if self.match_token(&[TokenType::If]) {
            return self.parse_conditional();
        }

        if self.check_modal() {
            self.advance();
            return self.parse_modal();
        }

        if self.match_token(&[TokenType::Not]) {
            self.negative_depth += 1;
            let inner = self.parse_sentence()?;
            self.negative_depth -= 1;
            return Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: inner,
            }));
        }

        self.parse_disjunction()
    }

    fn check_wh_word(&self) -> bool {
        if matches!(
            self.peek().kind,
            TokenType::Who
                | TokenType::What
                | TokenType::Where
                | TokenType::When
                | TokenType::Why
        ) {
            return true;
        }
        if self.check_preposition() && self.current + 1 < self.tokens.len() {
            matches!(
                self.tokens[self.current + 1].kind,
                TokenType::Who
                    | TokenType::What
                    | TokenType::Where
                    | TokenType::When
                    | TokenType::Why
            )
        } else {
            false
        }
    }

    fn parse_conditional(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        let is_counterfactual = self.is_counterfactual_context();

        let antecedent = self.parse_counterfactual_antecedent()?;

        if self.check(&TokenType::Comma) {
            self.advance();
        }

        if self.check(&TokenType::Then) {
            self.advance();
        }

        let consequent = self.parse_counterfactual_consequent()?;

        Ok(if is_counterfactual {
            self.ctx.exprs.alloc(LogicExpr::Counterfactual {
                antecedent,
                consequent,
            })
        } else {
            self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: antecedent,
                op: TokenType::If,
                right: consequent,
            })
        })
    }

    fn is_counterfactual_context(&self) -> bool {
        for i in 0..5 {
            if self.current + i >= self.tokens.len() {
                break;
            }
            let token = &self.tokens[self.current + i];
            if matches!(token.kind, TokenType::Were | TokenType::Had) {
                return true;
            }
            if matches!(token.kind, TokenType::Comma | TokenType::Period) {
                break;
            }
        }
        false
    }

    fn parse_counterfactual_antecedent(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        let unknown = self.interner.intern("?");
        if self.check_content_word() || self.check_pronoun() {
            let subject = if self.check_pronoun() {
                let token = self.advance().clone();
                if let TokenType::Pronoun { gender, number, .. } = token.kind {
                    self.resolve_pronoun(gender, number).unwrap_or(unknown)
                } else {
                    unknown
                }
            } else {
                self.parse_noun_phrase(true)?.noun
            };

            // Handle presupposition triggers in antecedent: "If John stopped smoking, ..."
            // Only trigger if followed by gerund complement
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
                let np = NounPhrase {
                    noun: subject,
                    definiteness: None,
                    adjectives: &[],
                    possessor: None,
                    pps: &[],
                    superlative: None,
                };
                return self.parse_presupposition(&np, presup_kind);
            }

            if self.check(&TokenType::Were) {
                self.advance();
                let predicate = if self.check_pronoun() {
                    let token = self.advance().clone();
                    if let TokenType::Pronoun { gender, number, .. } = token.kind {
                        let token_text = self.interner.resolve(token.lexeme);
                        if token_text.eq_ignore_ascii_case("i") {
                            self.interner.intern("Speaker")
                        } else if token_text.eq_ignore_ascii_case("you") {
                            self.interner.intern("Addressee")
                        } else {
                            self.resolve_pronoun(gender, number).unwrap_or(unknown)
                        }
                    } else {
                        unknown
                    }
                } else {
                    self.consume_content_word()?
                };
                let be = self.interner.intern("Be");
                return Ok(self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: be,
                    args: self.ctx.terms.alloc_slice([
                        Term::Constant(subject),
                        Term::Constant(predicate),
                    ]),
                }));
            }

            if self.check(&TokenType::Had) {
                self.advance();
                let verb = self.consume_content_word()?;
                let main_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: verb,
                    args: self.ctx.terms.alloc_slice([Term::Constant(subject)]),
                });

                // Handle "because" causal clause in antecedent
                // Phase 35: Do NOT consume if followed by string literal (Trust justification)
                if self.check(&TokenType::Because) && !self.peek_next_is_string_literal() {
                    self.advance();
                    let cause = self.parse_atom()?;
                    return Ok(self.ctx.exprs.alloc(LogicExpr::Causal {
                        effect: main_pred,
                        cause,
                    }));
                }

                return Ok(main_pred);
            }

            return self.parse_predicate_with_subject(subject);
        }

        self.parse_sentence()
    }

    fn parse_counterfactual_consequent(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        let unknown = self.interner.intern("?");
        if self.check_content_word() || self.check_pronoun() {
            let subject = if self.check_pronoun() {
                let token = self.advance().clone();
                if let TokenType::Pronoun { gender, number, .. } = token.kind {
                    self.resolve_pronoun(gender, number).unwrap_or(unknown)
                } else {
                    unknown
                }
            } else {
                self.parse_noun_phrase(true)?.noun
            };

            if self.check(&TokenType::Would) {
                self.advance();
                if self.check_content_word() {
                    let next_word = self.interner.resolve(self.peek().lexeme).to_lowercase();
                    if next_word == "have" {
                        self.advance();
                    }
                }
                let verb = self.consume_content_word()?;
                return Ok(self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: verb,
                    args: self.ctx.terms.alloc_slice([Term::Constant(subject)]),
                }));
            }

            return self.parse_predicate_with_subject(subject);
        }

        self.parse_sentence()
    }

    fn extract_verb_from_expr(&self, expr: &LogicExpr<'a>) -> Option<Symbol> {
        match expr {
            LogicExpr::Predicate { name, .. } => Some(*name),
            LogicExpr::NeoEvent(data) => Some(data.verb),
            LogicExpr::BinaryOp { right, .. } => self.extract_verb_from_expr(right),
            LogicExpr::Modal { operand, .. } => self.extract_verb_from_expr(operand),
            LogicExpr::Presupposition { assertion, .. } => self.extract_verb_from_expr(assertion),
            LogicExpr::Control { verb, .. } => Some(*verb),
            LogicExpr::Temporal { body, .. } => self.extract_verb_from_expr(body),
            LogicExpr::TemporalAnchor { body, .. } => self.extract_verb_from_expr(body),
            LogicExpr::Aspectual { body, .. } => self.extract_verb_from_expr(body),
            LogicExpr::Quantifier { body, .. } => self.extract_verb_from_expr(body),
            _ => None,
        }
    }

    fn parse_gapped_clause(&mut self, borrowed_verb: Symbol) -> ParseResult<&'a LogicExpr<'a>> {
        let subject = self.parse_noun_phrase(true)?;

        if self.check(&TokenType::Comma) {
            self.advance();
        }

        let subject_term = self.noun_phrase_to_term(&subject);
        let event_var = self.get_event_var();

        // Check if next token is temporal adverb (gapping with adjunct only)
        if self.check_temporal_adverb() {
            let adv_sym = if let TokenType::TemporalAdverb(sym) = self.advance().kind {
                sym
            } else {
                self.interner.intern("?")
            };

            return Ok(self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                event_var,
                verb: borrowed_verb,
                roles: self.ctx.roles.alloc_slice(vec![
                    (ThematicRole::Agent, subject_term),
                ]),
                modifiers: self.ctx.syms.alloc_slice(vec![adv_sym]),
            }))));
        }

        // Standard gapping: subject + object
        let object = self.parse_noun_phrase(false)?;
        let object_term = self.noun_phrase_to_term(&object);

        let roles = vec![
            (ThematicRole::Agent, subject_term),
            (ThematicRole::Theme, object_term),
        ];

        Ok(self
            .ctx
            .exprs
            .alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                event_var,
                verb: borrowed_verb,
                roles: self.ctx.roles.alloc_slice(roles),
                modifiers: self.ctx.syms.alloc_slice(vec![]),
            }))))
    }

    fn is_complete_clause(&self, expr: &LogicExpr<'a>) -> bool {
        match expr {
            LogicExpr::Atom(_) => false,
            LogicExpr::Predicate { .. } => true,
            LogicExpr::Quantifier { .. } => true,
            LogicExpr::Modal { .. } => true,
            LogicExpr::Temporal { .. } => true,
            LogicExpr::Aspectual { .. } => true,
            LogicExpr::BinaryOp { .. } => true,
            LogicExpr::UnaryOp { .. } => true,
            LogicExpr::Control { .. } => true,
            LogicExpr::Presupposition { .. } => true,
            LogicExpr::Categorical(_) => true,
            LogicExpr::Relation(_) => true,
            _ => true,
        }
    }

    /// Parse disjunction (Or/Iff) - lowest precedence logical connectives.
    /// Calls parse_conjunction for operands to ensure And binds tighter.
    fn parse_disjunction(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        let mut expr = self.parse_conjunction()?;

        while self.check(&TokenType::Comma)
            || self.check(&TokenType::Or)
            || self.check(&TokenType::Iff)
        {
            if self.check(&TokenType::Comma) {
                self.advance();
            }
            if !self.match_token(&[TokenType::Or, TokenType::Iff]) {
                break;
            }
            let operator = self.previous().kind.clone();
            self.current_island += 1;

            let saved_pos = self.current;
            let standard_attempt = self.try_parse(|p| p.parse_conjunction());

            let use_gapping = match &standard_attempt {
                Some(right) => {
                    !self.is_complete_clause(right)
                        && (self.check(&TokenType::Comma) || self.check_content_word())
                }
                None => true,
            };

            if !use_gapping {
                if let Some(right) = standard_attempt {
                    expr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: expr,
                        op: operator,
                        right,
                    });
                }
            } else {
                self.current = saved_pos;

                let borrowed_verb = self.extract_verb_from_expr(expr).ok_or(ParseError {
                    kind: ParseErrorKind::GappingResolutionFailed,
                    span: self.current_span(),
                })?;

                let right = self.parse_gapped_clause(borrowed_verb)?;

                expr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: expr,
                    op: operator,
                    right,
                });
            }
        }

        Ok(expr)
    }

    /// Parse conjunction (And) - higher precedence than Or.
    /// Calls parse_atom for operands.
    fn parse_conjunction(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        let mut expr = self.parse_atom()?;

        // Handle causal "because" at conjunction level
        // Phase 35: Do NOT consume if followed by string literal (Trust justification)
        if self.check(&TokenType::Because) && !self.peek_next_is_string_literal() {
            self.advance();
            let cause = self.parse_atom()?;
            return Ok(self.ctx.exprs.alloc(LogicExpr::Causal {
                effect: expr,
                cause,
            }));
        }

        while self.check(&TokenType::Comma) || self.check(&TokenType::And) {
            if self.check(&TokenType::Comma) {
                self.advance();
            }
            if !self.match_token(&[TokenType::And]) {
                break;
            }
            let operator = self.previous().kind.clone();
            self.current_island += 1;

            let saved_pos = self.current;
            let standard_attempt = self.try_parse(|p| p.parse_atom());

            let use_gapping = match &standard_attempt {
                Some(right) => {
                    !self.is_complete_clause(right)
                        && (self.check(&TokenType::Comma) || self.check_content_word())
                }
                None => true,
            };

            if !use_gapping {
                if let Some(right) = standard_attempt {
                    expr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: expr,
                        op: operator,
                        right,
                    });
                }
            } else {
                self.current = saved_pos;

                let borrowed_verb = self.extract_verb_from_expr(expr).ok_or(ParseError {
                    kind: ParseErrorKind::GappingResolutionFailed,
                    span: self.current_span(),
                })?;

                let right = self.parse_gapped_clause(borrowed_verb)?;

                expr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: expr,
                    op: operator,
                    right,
                });
            }
        }

        Ok(expr)
    }

    fn parse_relative_clause(&mut self, gap_var: Symbol) -> ParseResult<&'a LogicExpr<'a>> {
        if self.check_verb() {
            return self.parse_verb_phrase_for_restriction(gap_var);
        }

        if self.check_content_word() || self.check_article() {
            let rel_subject = self.parse_noun_phrase_for_relative()?;

            let nested_relative = if matches!(self.peek().kind, TokenType::Article(_)) {
                let nested_var = self.next_var_name();
                Some((nested_var, self.parse_relative_clause(nested_var)?))
            } else {
                None
            };

            if self.check_verb() {
                let verb = self.consume_verb();

                let mut roles: Vec<(ThematicRole, Term<'a>)> = vec![
                    (ThematicRole::Agent, Term::Constant(rel_subject.noun)),
                    (ThematicRole::Theme, Term::Variable(gap_var)),
                ];

                while self.check_to_preposition() {
                    self.advance();
                    if self.check_content_word() || self.check_article() {
                        let recipient = self.parse_noun_phrase(false)?;
                        roles.push((ThematicRole::Recipient, Term::Constant(recipient.noun)));
                    }
                }

                let event_var = self.get_event_var();
                let this_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var,
                    verb,
                    roles: self.ctx.roles.alloc_slice(roles),
                    modifiers: self.ctx.syms.alloc_slice(vec![]),
                })));

                if let Some((nested_var, nested_clause)) = nested_relative {
                    let type_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: rel_subject.noun,
                        args: self.ctx.terms.alloc_slice([Term::Variable(nested_var)]),
                    });

                    let inner = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: type_pred,
                        op: TokenType::And,
                        right: nested_clause,
                    });

                    let combined = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: inner,
                        op: TokenType::And,
                        right: this_event,
                    });

                    return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                        kind: crate::ast::QuantifierKind::Existential,
                        variable: nested_var,
                        body: combined,
                        island_id: self.current_island,
                    }));
                }

                return Ok(this_event);
            }
        }

        if self.check_verb() {
            return self.parse_verb_phrase_for_restriction(gap_var);
        }

        let unknown = self.interner.intern("?");
        Ok(self.ctx.exprs.alloc(LogicExpr::Atom(unknown)))
    }

    fn check_ellipsis_auxiliary(&self) -> bool {
        matches!(
            self.peek().kind,
            TokenType::Does | TokenType::Do |
            TokenType::Can | TokenType::Could | TokenType::Would |
            TokenType::May | TokenType::Must | TokenType::Should
        )
    }

    fn check_ellipsis_terminator(&self) -> bool {
        if self.is_at_end() || self.check(&TokenType::Period) {
            return true;
        }
        if self.check_content_word() {
            let word = self.interner.resolve(self.peek().lexeme).to_lowercase();
            return word == "too" || word == "also";
        }
        false
    }

    fn try_parse_ellipsis(&mut self) -> Option<ParseResult<&'a LogicExpr<'a>>> {
        // Need a stored template to reconstruct from
        if self.last_event_template.is_none() {
            return None;
        }

        let saved_pos = self.current;

        // Pattern: Subject + Auxiliary + (not)? + Terminator
        // Subject must be proper name or pronoun
        let subject_sym = if matches!(self.peek().kind, TokenType::ProperName(_)) {
            if let TokenType::ProperName(sym) = self.advance().kind {
                sym
            } else {
                self.current = saved_pos;
                return None;
            }
        } else if self.check_pronoun() {
            let token = self.advance().clone();
            if let TokenType::Pronoun { gender, number, .. } = token.kind {
                self.resolve_pronoun(gender, number)
                    .unwrap_or_else(|| self.interner.intern("?"))
            } else {
                self.current = saved_pos;
                return None;
            }
        } else {
            return None;
        };

        // Must be followed by ellipsis auxiliary
        if !self.check_ellipsis_auxiliary() {
            self.current = saved_pos;
            return None;
        }
        let aux_token = self.advance().kind.clone();

        // Check for negation
        let is_negated = self.match_token(&[TokenType::Not]);

        // Must end with terminator
        if !self.check_ellipsis_terminator() {
            self.current = saved_pos;
            return None;
        }

        // Consume "too"/"also" if present
        if self.check_content_word() {
            let word = self.interner.resolve(self.peek().lexeme).to_lowercase();
            if word == "too" || word == "also" {
                self.advance();
            }
        }

        // Reconstruct from template
        let template = self.last_event_template.clone().unwrap();
        let event_var = self.get_event_var();

        // Build roles with new subject as Agent
        let mut roles: Vec<(ThematicRole, Term<'a>)> = vec![
            (ThematicRole::Agent, Term::Constant(subject_sym))
        ];
        roles.extend(template.non_agent_roles.iter().cloned());

        let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
            event_var,
            verb: template.verb,
            roles: self.ctx.roles.alloc_slice(roles),
            modifiers: self.ctx.syms.alloc_slice(template.modifiers.clone()),
        })));

        // Apply modal if auxiliary is modal
        let with_modal = match aux_token {
            TokenType::Can | TokenType::Could => {
                let vector = self.token_to_vector(&aux_token);
                self.ctx.modal(vector, neo_event)
            }
            TokenType::Would | TokenType::May | TokenType::Must | TokenType::Should => {
                let vector = self.token_to_vector(&aux_token);
                self.ctx.modal(vector, neo_event)
            }
            _ => neo_event,
        };

        // Apply negation if present
        let result = if is_negated {
            self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: with_modal,
            })
        } else {
            with_modal
        };

        Some(Ok(result))
    }
}
