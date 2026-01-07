use super::clause::ClauseParsing;
use super::{ParseResult, Parser};
use crate::ast::{LogicExpr, NounPhrase, Term};
use crate::drs::{Case, Gender, Number};
use crate::intern::SymbolEq;
use crate::lexicon::Definiteness;
use crate::token::TokenType;
use crate::transpile::capitalize_first;

pub trait NounParsing<'a, 'ctx, 'int> {
    fn parse_noun_phrase(&mut self, greedy: bool) -> ParseResult<NounPhrase<'a>>;
    fn parse_noun_phrase_for_relative(&mut self) -> ParseResult<NounPhrase<'a>>;
    fn noun_phrase_to_term(&self, np: &NounPhrase<'a>) -> Term<'a>;
    fn check_possessive(&self) -> bool;
    fn check_of_preposition(&self) -> bool;
    fn check_proper_name_or_label(&self) -> bool;
    fn check_possessive_pronoun(&self) -> bool;
}

impl<'a, 'ctx, 'int> NounParsing<'a, 'ctx, 'int> for Parser<'a, 'ctx, 'int> {
    fn parse_noun_phrase(&mut self, greedy: bool) -> ParseResult<NounPhrase<'a>> {
        let mut definiteness = None;
        let mut adjectives = Vec::new();
        let mut non_intersective_prefix: Option<crate::intern::Symbol> = None;
        let mut possessor_from_pronoun: Option<&'a NounPhrase<'a>> = None;
        let mut superlative_adj: Option<crate::intern::Symbol> = None;

        // Phase 35: Support numeric literals as noun phrases (e.g., "equal to 42")
        if let TokenType::Number(sym) = self.peek().kind {
            self.advance();
            return Ok(NounPhrase {
                definiteness: None,
                adjectives: &[],
                noun: sym,
                possessor: None,
                pps: &[],
                superlative: None,
            });
        }

        if self.check_possessive_pronoun() {
            let token = self.advance().clone();
            let (gender, number) = match &token.kind {
                TokenType::Pronoun { gender, number, case: Case::Possessive } => (*gender, *number),
                TokenType::Ambiguous { primary, alternatives } => {
                    let mut found = None;
                    if let TokenType::Pronoun { gender, number, case: Case::Possessive } = **primary {
                        found = Some((gender, number));
                    }
                    if found.is_none() {
                        for alt in alternatives {
                            if let TokenType::Pronoun { gender, number, case: Case::Possessive } = alt {
                                found = Some((*gender, *number));
                                break;
                            }
                        }
                    }
                    found.unwrap_or((Gender::Unknown, Number::Singular))
                }
                _ => (Gender::Unknown, Number::Singular),
            };

            let resolved = self.resolve_pronoun(gender, number)?;
            let resolved_sym = match resolved {
                super::ResolvedPronoun::Variable(s) | super::ResolvedPronoun::Constant(s) => s,
            };

            let possessor_np = NounPhrase {
                definiteness: None,
                adjectives: &[],
                noun: resolved_sym,
                possessor: None,
                pps: &[],
                superlative: None,
            };
            possessor_from_pronoun = Some(self.ctx.nps.alloc(possessor_np));
            definiteness = Some(Definiteness::Definite);
        } else if let TokenType::Article(def) = self.peek().kind {
            // Phase 35: Disambiguate "a" as variable vs article
            // If "a" or "an" is followed by a verb/copula/modal, it's a variable name, not an article
            let is_variable_a = {
                let lexeme = self.interner.resolve(self.peek().lexeme).to_lowercase();
                if lexeme == "a" || lexeme == "an" {
                    if let Some(next) = self.tokens.get(self.current + 1) {
                        matches!(next.kind,
                            TokenType::Is | TokenType::Are | TokenType::Was | TokenType::Were | // Copula
                            TokenType::Verb { .. } | // Main verb
                            TokenType::Auxiliary(_) | // will, did
                            TokenType::Must | TokenType::Can | TokenType::Should | TokenType::May | // Modals
                            TokenType::Could | TokenType::Would | TokenType::Shall | TokenType::Might |
                            TokenType::Identity | TokenType::Equals // "a = b"
                        )
                    } else {
                        false
                    }
                } else {
                    false
                }
            };

            if !is_variable_a {
                definiteness = Some(def);
                self.advance();
            }
        }

        if self.check_superlative() {
            if let TokenType::Superlative(adj) = self.advance().kind {
                superlative_adj = Some(adj);
            }
        }

        if self.check_non_intersective_adjective() {
            if let TokenType::NonIntersectiveAdjective(adj) = self.advance().kind {
                non_intersective_prefix = Some(adj);
            }
        }

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
                    TokenType::Noun(_)
                        | TokenType::Adjective(_)
                        | TokenType::ProperName(_)
                )
            } else {
                false
            };

            if next_is_content {
                if let TokenType::Adjective(adj) = self.advance().kind {
                    adjectives.push(adj);
                }
            } else {
                break;
            }
        }

        let base_noun = self.consume_content_word()?;

        let noun = if let Some(prefix) = non_intersective_prefix {
            let prefix_str = self.interner.resolve(prefix);
            let base_str = self.interner.resolve(base_noun);
            let compound = format!("{}-{}", prefix_str, base_str);
            self.interner.intern(&compound)
        } else {
            base_noun
        };

        let noun = if self.check_proper_name_or_label() {
            let label = self.consume_content_word()?;
            let label_str = self.interner.resolve(label);
            let base_str = self.interner.resolve(noun);
            let compound = format!("{}_{}", base_str, label_str);
            self.interner.intern(&compound)
        } else {
            noun
        };

        if self.check_possessive() {
            self.advance();

            let possessor = self.ctx.nps.alloc(NounPhrase {
                definiteness,
                adjectives: self.ctx.syms.alloc_slice(adjectives.clone()),
                noun,
                possessor: None,
                pps: &[],
                superlative: superlative_adj,
            });

            let possessed_noun = self.consume_content_word()?;

            return Ok(NounPhrase {
                definiteness: None,
                adjectives: &[],
                noun: possessed_noun,
                possessor: Some(possessor),
                pps: &[],
                superlative: None,
            });
        }

        let should_attach_pps = greedy || self.pp_attach_to_noun;

        let mut pps: Vec<&'a LogicExpr<'a>> = Vec::new();
        if should_attach_pps {
            while self.check_preposition() && !self.check_of_preposition() {
                let prep_token = self.advance().clone();
                let prep_name = if let TokenType::Preposition(sym) = prep_token.kind {
                    sym
                } else {
                    break;
                };

                if self.check_content_word() || matches!(self.peek().kind, TokenType::Article(_)) {
                    let pp_object = self.parse_noun_phrase(true)?;
                    let placeholder_var = self.interner.intern("_PP_SELF_");
                    let pp_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: prep_name,
                        args: self.ctx.terms.alloc_slice([
                            Term::Variable(placeholder_var),
                            Term::Constant(pp_object.noun),
                        ]),
                        world: None,
                    });
                    pps.push(pp_pred);
                }
            }
        }
        let pps_slice = self.ctx.pps.alloc_slice(pps);

        if self.check_of_preposition() {
            // Two-Pass Type Disambiguation:
            // If the noun is a known generic type (e.g., "Stack", "List"),
            // then "X of Y" is a type instantiation, not a possessive.
            // For now, we still parse it as possessive structurally, but
            // the type_registry enables future AST extensions for type annotations.
            let is_generic = self.is_generic_type(noun);

            if !is_generic {
                // Standard possessive: "owner of house" → possessor relationship
                self.advance();

                let possessor_np = self.parse_noun_phrase(true)?;
                let possessor = self.ctx.nps.alloc(possessor_np);

                return Ok(NounPhrase {
                    definiteness,
                    adjectives: self.ctx.syms.alloc_slice(adjectives),
                    noun,
                    possessor: Some(possessor),
                    pps: pps_slice,
                    superlative: superlative_adj,
                });
            }
            // If generic type, fall through to regular noun phrase handling.
            // The "of [Type]" will be left unparsed for now.
            // Future: Parse as GenericType { base: noun, params: [...] }
        }

        // Register ALL noun phrases as discourse entities, not just definite ones.
        // This is needed for bridging anaphora: "I bought a car. The engine smoked."
        // The indefinite "a car" must be in discourse history for "the engine" to link to it.
        let noun_str = self.interner.resolve(noun);
        let first_char = noun_str.chars().next().unwrap_or('X');
        if first_char.is_alphabetic() {
            // Use full noun name as symbol for consistent output in Full mode
            let symbol = capitalize_first(noun_str);
            let number = if noun_str.ends_with('s') && !noun_str.ends_with("ss") {
                Number::Plural
            } else {
                Number::Singular
            };
        }

        Ok(NounPhrase {
            definiteness,
            adjectives: self.ctx.syms.alloc_slice(adjectives),
            noun,
            possessor: possessor_from_pronoun,
            pps: pps_slice,
            superlative: superlative_adj,
        })
    }

    fn parse_noun_phrase_for_relative(&mut self) -> ParseResult<NounPhrase<'a>> {
        let mut definiteness = None;
        let mut adjectives = Vec::new();

        if let TokenType::Article(def) = self.peek().kind {
            definiteness = Some(def);
            self.advance();
        }

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
                    TokenType::Noun(_)
                        | TokenType::Adjective(_)
                        | TokenType::Verb { .. }
                        | TokenType::ProperName(_)
                )
            } else {
                false
            };

            if next_is_content {
                if let TokenType::Adjective(adj) = self.advance().kind.clone() {
                    adjectives.push(adj);
                }
            } else {
                break;
            }
        }

        let noun = self.consume_content_word_for_relative()?;

        if self.check(&TokenType::That) || self.check(&TokenType::Who) {
            self.advance();
            let var_name = self.interner.intern(&format!("r{}", self.var_counter));
            self.var_counter += 1;
            let _nested_clause = self.parse_relative_clause(var_name)?;
        }

        Ok(NounPhrase {
            definiteness,
            adjectives: self.ctx.syms.alloc_slice(adjectives),
            noun,
            possessor: None,
            pps: &[],
            superlative: None,
        })
    }

    fn noun_phrase_to_term(&self, np: &NounPhrase<'a>) -> Term<'a> {
        if let Some(possessor) = np.possessor {
            let possessor_term = self.noun_phrase_to_term(possessor);
            Term::Possessed {
                possessor: self.ctx.terms.alloc(possessor_term),
                possessed: np.noun,
            }
        } else {
            Term::Constant(np.noun)
        }
    }

    fn check_possessive(&self) -> bool {
        matches!(self.peek().kind, TokenType::Possessive)
    }

    fn check_of_preposition(&self) -> bool {
        if let TokenType::Preposition(p) = self.peek().kind {
            p.is(self.interner, "of")
        } else {
            false
        }
    }

    fn check_proper_name_or_label(&self) -> bool {
        match &self.peek().kind {
            TokenType::ProperName(_) => true,
            TokenType::Noun(s) => {
                let str_val = self.interner.resolve(*s);
                str_val.len() == 1 && str_val.chars().next().unwrap().is_uppercase()
            }
            _ => false,
        }
    }

    fn check_possessive_pronoun(&self) -> bool {
        match &self.peek().kind {
            TokenType::Pronoun { case: Case::Possessive, .. } => true,
            TokenType::Ambiguous { primary, alternatives } => {
                if self.noun_priority_mode {
                    if let TokenType::Pronoun { case: Case::Possessive, .. } = **primary {
                        return true;
                    }
                    for alt in alternatives {
                        if let TokenType::Pronoun { case: Case::Possessive, .. } = alt {
                            return true;
                        }
                    }
                }
                false
            }
            _ => false,
        }
    }
}
