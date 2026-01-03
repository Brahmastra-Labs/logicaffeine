use super::modal::ModalParsing;
use super::noun::NounParsing;
use super::pragmatics::PragmaticsParsing;
use super::quantifier::QuantifierParsing;
use super::question::QuestionParsing;
use super::verb::LogicVerbParsing;
use super::{ParseResult, Parser};
use crate::ast::{LogicExpr, NeoEventData, NounPhrase, QuantifierKind, TemporalOperator, Term, ThematicRole};
use crate::lexer::Lexer;
use crate::lexicon::Time;
use crate::drs::BoxType;
use crate::error::{ParseError, ParseErrorKind};
use crate::intern::Symbol;
use crate::lexicon::Definiteness;
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
                        world: None,
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

        // Enter DRS antecedent box - indefinites here get universal force
        self.drs.enter_box(BoxType::ConditionalAntecedent);
        let antecedent = self.parse_counterfactual_antecedent()?;
        self.drs.exit_box();

        if self.check(&TokenType::Comma) {
            self.advance();
        }

        if self.check(&TokenType::Then) {
            self.advance();
        }

        // Enter DRS consequent box - can access antecedent referents
        self.drs.enter_box(BoxType::ConditionalConsequent);
        let consequent = self.parse_counterfactual_consequent()?;
        self.drs.exit_box();

        // Get DRS referents that need universal quantification
        let universal_refs = self.drs.get_universal_referents();

        // Build the conditional expression
        let conditional = if is_counterfactual {
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
        };

        // Wrap with universal quantifiers for DRS referents
        let mut result = conditional;
        for var in universal_refs.into_iter().rev() {
            result = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind: QuantifierKind::Universal,
                variable: var,
                body: result,
                island_id: self.current_island,
            });
        }

        Ok(result)
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
        if self.check_content_word() || self.check_pronoun() || self.check_article() {
            // Weather verb detection: "if it rains" → ∃e(Rain(e))
            // Must check BEFORE pronoun resolution since "it" would resolve to "?"
            if self.check_pronoun() {
                let token = self.peek();
                let token_text = self.interner.resolve(token.lexeme);
                if token_text.eq_ignore_ascii_case("it") {
                    // Look ahead for weather verb
                    if self.current + 1 < self.tokens.len() {
                        if let TokenType::Verb { lemma, time, .. } = &self.tokens[self.current + 1].kind {
                            let lemma_str = self.interner.resolve(*lemma);
                            if Lexer::is_weather_verb(lemma_str) {
                                let verb = *lemma;
                                let verb_time = *time;
                                self.advance(); // consume "it"
                                self.advance(); // consume weather verb

                                let event_var = self.get_event_var();

                                // DRT: Register event var for universal quantification in conditionals
                                let suppress_existential = self.drs.in_conditional_antecedent();
                                if suppress_existential {
                                    let event_class = self.interner.intern("Event");
                                    self.drs.introduce_referent(event_var, event_class, crate::context::Gender::Neuter);
                                }

                                let mut result: &'a LogicExpr<'a> = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                                    event_var,
                                    verb,
                                    roles: self.ctx.roles.alloc_slice(vec![]),
                                    modifiers: self.ctx.syms.alloc_slice(vec![]),
                                    suppress_existential,
                                    world: None,
                                })));

                                // Handle coordinated weather verbs: "rains and thunders" or "rains or thunders"
                                // SHARE the same event_var for all coordinated verbs
                                while self.check(&TokenType::And) || self.check(&TokenType::Or) {
                                    let is_disjunction = self.check(&TokenType::Or);
                                    self.advance(); // consume "and" or "or"

                                    if let TokenType::Verb { lemma: lemma2, .. } = &self.peek().kind.clone() {
                                        let lemma2_str = self.interner.resolve(*lemma2);
                                        if Lexer::is_weather_verb(lemma2_str) {
                                            let verb2 = *lemma2;
                                            self.advance(); // consume second weather verb

                                            // REUSE same event_var - no new variable, no DRS registration
                                            let neo_event2 = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                                                event_var,  // Same variable as first weather verb
                                                verb: verb2,
                                                roles: self.ctx.roles.alloc_slice(vec![]),
                                                modifiers: self.ctx.syms.alloc_slice(vec![]),
                                                suppress_existential,
                                                world: None,
                                            })));

                                            let op = if is_disjunction { TokenType::Or } else { TokenType::And };
                                            result = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                                left: result,
                                                op,
                                                right: neo_event2,
                                            });
                                        } else {
                                            break; // Not a weather verb, stop coordination
                                        }
                                    } else {
                                        break;
                                    }
                                }

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
                        }
                    }
                }
            }

            // Track if subject is an indefinite that needs DRS registration
            let (subject, subject_type_pred) = if self.check_pronoun() {
                let token = self.advance().clone();
                let resolved = if let TokenType::Pronoun { gender, number, .. } = token.kind {
                    self.resolve_pronoun(gender, number).unwrap_or(unknown)
                } else {
                    unknown
                };
                (resolved, None)
            } else {
                let np = self.parse_noun_phrase(true)?;

                // Check if this is an indefinite NP that should introduce a DRS referent
                if np.definiteness == Some(Definiteness::Indefinite) {
                    let var = self.next_var_name();
                    let gender = Self::infer_noun_gender(self.interner.resolve(np.noun));

                    // Register in DRS - will get universal force from ConditionalAntecedent box
                    self.drs.introduce_referent(var, np.noun, gender);

                    // Create type predicate: Farmer(x)
                    let type_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: np.noun,
                        args: self.ctx.terms.alloc_slice([Term::Variable(var)]),
                        world: None,
                    });

                    (var, Some(type_pred))
                } else {
                    // Definite or proper name - use as constant
                    (np.noun, None)
                }
            };

            // Determine the subject term type
            let subject_term = if subject_type_pred.is_some() {
                Term::Variable(subject)
            } else {
                Term::Constant(subject)
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
                let be_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: be,
                    args: self.ctx.terms.alloc_slice([
                        subject_term,
                        Term::Constant(predicate),
                    ]),
                    world: None,
                });
                // Combine with type predicate if indefinite subject
                return Ok(if let Some(type_pred) = subject_type_pred {
                    self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: type_pred,
                        op: TokenType::And,
                        right: be_pred,
                    })
                } else {
                    be_pred
                });
            }

            if self.check(&TokenType::Had) {
                self.advance();
                let verb = self.consume_content_word()?;
                let main_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: verb,
                    args: self.ctx.terms.alloc_slice([subject_term]),
                    world: None,
                });

                // Handle "because" causal clause in antecedent
                // Phase 35: Do NOT consume if followed by string literal (Trust justification)
                if self.check(&TokenType::Because) && !self.peek_next_is_string_literal() {
                    self.advance();
                    let cause = self.parse_atom()?;
                    let causal = self.ctx.exprs.alloc(LogicExpr::Causal {
                        effect: main_pred,
                        cause,
                    });
                    // Combine with type predicate if indefinite subject
                    return Ok(if let Some(type_pred) = subject_type_pred {
                        self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: type_pred,
                            op: TokenType::And,
                            right: causal,
                        })
                    } else {
                        causal
                    });
                }

                // Combine with type predicate if indefinite subject
                return Ok(if let Some(type_pred) = subject_type_pred {
                    self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: type_pred,
                        op: TokenType::And,
                        right: main_pred,
                    })
                } else {
                    main_pred
                });
            }

            // Parse verb phrase with subject
            // Use variable term for indefinite subjects, constant for definites/proper names
            let verb_phrase = if subject_type_pred.is_some() {
                self.parse_predicate_with_subject_as_var(subject)?
            } else {
                self.parse_predicate_with_subject(subject)?
            };

            // Combine with type predicate if indefinite subject
            return Ok(if let Some(type_pred) = subject_type_pred {
                self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: type_pred,
                    op: TokenType::And,
                    right: verb_phrase,
                })
            } else {
                verb_phrase
            });
        }

        self.parse_sentence()
    }

    fn parse_counterfactual_consequent(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        let unknown = self.interner.intern("?");
        if self.check_content_word() || self.check_pronoun() {
            // Check for grammatically incorrect "its" + weather adjective
            // "its" is possessive, "it's" is contraction - common typo
            if self.check_pronoun() {
                let token = self.peek();
                let token_text = self.interner.resolve(token.lexeme).to_lowercase();
                if token_text == "its" {
                    // Check if followed by weather adjective
                    if self.current + 1 < self.tokens.len() {
                        let next_token = &self.tokens[self.current + 1];
                        let next_str = self.interner.resolve(next_token.lexeme).to_lowercase();
                        if let Some(meta) = crate::lexicon::lookup_adjective_db(&next_str) {
                            if meta.features.contains(&crate::lexicon::Feature::Weather) {
                                return Err(ParseError {
                                    kind: ParseErrorKind::GrammarError(
                                        "Did you mean 'it's' (it is)? 'its' is a possessive pronoun.".to_string()
                                    ),
                                    span: self.current_span(),
                                });
                            }
                        }
                    }
                }
            }

            // Check for expletive "it" + copula + weather adjective: "it's wet" → Wet
            if self.check_pronoun() {
                let token_text = self.interner.resolve(self.peek().lexeme).to_lowercase();
                if token_text == "it" {
                    // Look ahead for copula + weather adjective
                    // Handle both "it is wet" and "it's wet" (where 's is Possessive token)
                    if self.current + 2 < self.tokens.len() {
                        let next = &self.tokens[self.current + 1].kind;
                        if matches!(next, TokenType::Is | TokenType::Was | TokenType::Possessive) {
                            // Check if followed by weather adjective
                            let adj_token = &self.tokens[self.current + 2];
                            let adj_sym = adj_token.lexeme;
                            let adj_str = self.interner.resolve(adj_sym).to_lowercase();
                            if let Some(meta) = crate::lexicon::lookup_adjective_db(&adj_str) {
                                if meta.features.contains(&crate::lexicon::Feature::Weather) {
                                    self.advance(); // consume "it"
                                    self.advance(); // consume copula
                                    self.advance(); // consume adjective token

                                    // Use the canonical lemma from lexicon (e.g., "Wet" not "wet")
                                    let adj_lemma = self.interner.intern(meta.lemma);

                                    // Get event variable from DRS (introduced in antecedent)
                                    let event_var = self.drs.get_last_event_referent(self.interner)
                                        .unwrap_or_else(|| self.interner.intern("e"));

                                    // First weather adjective predicate
                                    let mut result: &'a LogicExpr<'a> = self.ctx.exprs.alloc(LogicExpr::Predicate {
                                        name: adj_lemma,
                                        args: self.ctx.terms.alloc_slice([Term::Variable(event_var)]),
                                        world: None,
                                    });

                                    // Handle coordinated adjectives: "wet and cold"
                                    while self.check(&TokenType::And) {
                                        self.advance(); // consume "and"
                                        if self.check_content_word() {
                                            let adj2_lexeme = self.peek().lexeme;
                                            let adj2_str = self.interner.resolve(adj2_lexeme).to_lowercase();

                                            // Check if it's also a weather adjective
                                            if let Some(meta2) = crate::lexicon::lookup_adjective_db(&adj2_str) {
                                                if meta2.features.contains(&crate::lexicon::Feature::Weather) {
                                                    self.advance(); // consume adjective token
                                                    // Use the canonical lemma from lexicon (e.g., "Cold" not "cold")
                                                    let adj2_lemma = self.interner.intern(meta2.lemma);
                                                    let pred2 = self.ctx.exprs.alloc(LogicExpr::Predicate {
                                                        name: adj2_lemma,
                                                        args: self.ctx.terms.alloc_slice([Term::Variable(event_var)]),
                                                        world: None,
                                                    });
                                                    result = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                                        left: result,
                                                        op: TokenType::And,
                                                        right: pred2,
                                                    });
                                                    continue;
                                                }
                                            }
                                        }
                                        break;
                                    }

                                    return Ok(result);
                                }
                            }
                        }
                    }
                }
            }

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
                    world: None,
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
        let suppress_existential = self.drs.in_conditional_antecedent();

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
                suppress_existential,
                world: None,
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
                suppress_existential,
                world: None,
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
                let suppress_existential = self.drs.in_conditional_antecedent();
                let this_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var,
                    verb,
                    roles: self.ctx.roles.alloc_slice(roles),
                    modifiers: self.ctx.syms.alloc_slice(vec![]),
                    suppress_existential,
                    world: None,
                })));

                if let Some((nested_var, nested_clause)) = nested_relative {
                    let type_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: rel_subject.noun,
                        args: self.ctx.terms.alloc_slice([Term::Variable(nested_var)]),
                        world: None,
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
        let suppress_existential = self.drs.in_conditional_antecedent();

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
            suppress_existential,
            world: None,
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
