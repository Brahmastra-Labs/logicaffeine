//! Noun phrase parsing with determiners, adjectives, and possessives.
//!
//! This module handles the full complexity of English noun phrases including:
//!
//! - **Determiners**: Articles (a, the), quantifiers (every, some, no)
//! - **Adjectives**: Pre-nominal modifiers, intersective vs subsective
//! - **Possessives**: "John's", "his", genitive constructions
//! - **Proper names**: Capitalized constants
//! - **Numeric literals**: Numbers as noun phrases for comparisons
//! - **Prepositional phrases**: Post-nominal "of" constructions
//! - **Superlatives**: "the tallest", "the most interesting"
//!
//! The parsed [`NounPhrase`] struct carries definiteness, adjectives, the head
//! noun, optional possessor, and attached prepositional phrases.

use super::clause::ClauseParsing;
use super::pragmatics::PragmaticsParsing;
use super::{ParseResult, Parser};
use crate::ast::{LogicExpr, NounPhrase, Term};
use crate::drs::{Case, Gender, Number};
use logicaffeine_base::SymbolEq;
use crate::lexicon::Definiteness;
use crate::token::TokenType;
use crate::transpile::capitalize_first;

/// The preposition a declared CATEGORY contributes to a definite-description
/// label, so the label un-fuses to the SAME relation the prepositional-phrase
/// form produces. TEMPORAL ("the 2003 holiday" ↔ "in 2003") and LOCATIVE ("the
/// Florida trip" ↔ "in Florida") both → `In`; PERSONAL ("the Bill trip" ↔ "with
/// Bill") → `With`.
///
/// The lexicon SORT is consulted first where it is decisive (`Place`/`Time` →
/// `In`, `Human` → `With`), but several calendar/region nouns carry an
/// `Abstract` sort in the lexicon (year, month, week, decade, century, state),
/// so an explicit lemma set backs the sort up. A category that maps to neither
/// dimension (e.g. "vegetable") returns `None` and the label stays fused.
fn category_preposition(category_lemma_lower: &str) -> Option<&'static str> {
    const TEMPORAL: &[&str] = &["year", "month", "day", "date", "week", "decade", "century"];
    const LOCATIVE: &[&str] =
        &["state", "city", "country", "province", "region", "town", "place"];
    const PERSONAL: &[&str] =
        &["friend", "person", "man", "woman", "colleague", "companion"];

    if TEMPORAL.contains(&category_lemma_lower) || LOCATIVE.contains(&category_lemma_lower) {
        return Some("In");
    }
    if PERSONAL.contains(&category_lemma_lower) {
        return Some("With");
    }
    match crate::lexicon::lookup_sort(category_lemma_lower) {
        Some(crate::lexicon::Sort::Place) | Some(crate::lexicon::Sort::Time) => Some("In"),
        Some(crate::lexicon::Sort::Human) => Some("With"),
        _ => None,
    }
}

/// Trait for parsing noun phrases.
///
/// Provides methods for parsing determiners, adjectives, possessives,
/// and converting noun phrases to first-order terms.
pub trait NounParsing<'a, 'ctx, 'int> {
    /// Parses a full noun phrase with optional greedy PP attachment.
    fn parse_noun_phrase(&mut self, greedy: bool) -> ParseResult<NounPhrase<'a>>;
    /// Parses a noun phrase suitable for relative clause antecedent.
    fn parse_noun_phrase_for_relative(&mut self) -> ParseResult<NounPhrase<'a>>;
    /// Converts a parsed noun phrase to a first-order term.
    fn noun_phrase_to_term(&self, np: &NounPhrase<'a>) -> Term<'a>;
    /// Checks for possessive marker ('s).
    fn check_possessive(&self) -> bool;
    /// Checks for "of" preposition (possessive or partitive).
    fn check_of_preposition(&self) -> bool;
    /// Checks for proper name or label (capitalized).
    fn check_proper_name_or_label(&self) -> bool;
    /// Whether the cursor opens an object-gap reduced relative (subject + transitive
    /// verb + empty object slot), e.g. "Tara won" in "the prize Tara won".
    fn peek_reduced_object_relative(&self) -> bool;
    /// Whether the cursor sits on a DEFINITE article that opens a noun phrase whose
    /// head is modified by a reduced object relative ("the friend Simon went with",
    /// "the waterfall Derrick photographed"). Used by the object-NP dispatcher to
    /// route such an object through the full `parse_noun_phrase` machinery instead
    /// of pre-consuming the article (which would hide the relative).
    fn peek_definite_reduced_relative_object(&self) -> bool;
    /// Checks for possessive pronoun (his, her, its, their).
    fn check_possessive_pronoun(&self) -> bool;
    /// Resolves a numeric LABEL ("the 2003 holiday", "the 1850 stamp"): returns
    /// the head symbol, FUSED (`2003_holiday`) by default, but UN-FUSED to the
    /// bare head plus a category relation restrictor (pushed onto
    /// `measure_restrictors`) when `n` names a DRS-declared item whose category
    /// maps to a preposition.
    fn numeric_label_head(
        &mut self,
        n: i64,
        head: crate::intern::Symbol,
        definiteness: Option<Definiteness>,
        measure_restrictors: &mut Vec<&'a LogicExpr<'a>>,
    ) -> crate::intern::Symbol;
    /// Consume a numeric-label HEAD preferring the NOUN reading of a verb-ambiguous
    /// word, so the fused symbol matches the noun-compound form ("the 2001 trip" →
    /// `2001_trip`, not the verb lemma `2001_Trip`). A verb-only head ("stamp")
    /// has no noun reading and keeps its lemma; a plain noun ("holiday") is already
    /// the noun. (The un-fused predicate is capitalized downstream regardless.)
    fn consume_label_head_noun_first(&mut self) -> ParseResult<crate::intern::Symbol>;
}

impl<'a, 'ctx, 'int> NounParsing<'a, 'ctx, 'int> for Parser<'a, 'ctx, 'int> {
    fn parse_noun_phrase(&mut self, greedy: bool) -> ParseResult<NounPhrase<'a>> {
        let mut definiteness = None;
        let mut adjectives = Vec::new();
        let mut non_intersective_prefix: Option<crate::intern::Symbol> = None;
        let mut possessor_from_pronoun: Option<&'a NounPhrase<'a>> = None;
        let mut superlative_adj: Option<crate::intern::Symbol> = None;
        // Attributive measure-adjective restrictors ("the 80 year old doll" →
        // Old(_PP_SELF_, 80 years)); merged into the NP's pps after the head, so a
        // degree property survives in every position the pps flow through.
        let mut measure_restrictors: Vec<&'a LogicExpr<'a>> = Vec::new();

        // Phase 35: Support numeric literals as noun phrases (e.g., "equal to 42").
        // BUT only when the number stands alone — a number FOLLOWED by a content
        // word heads a larger NP ("28 inch wingspan", "640 Twitter followers"),
        // which the numeric-head compound logic below folds; early-returning here
        // would strand the rest ("inch wingspan").
        if let TokenType::Number(sym) = self.peek().kind {
            let number_modifies_head = matches!(
                self.tokens.get(self.current + 1).map(|t| &t.kind),
                Some(TokenType::Noun(_)) | Some(TokenType::ProperName(_))
                    | Some(TokenType::Adjective(_)) | Some(TokenType::Verb { .. })
                    | Some(TokenType::Ambiguous { .. })
            );
            if !number_modifies_head {
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
                        // A PROGRESSIVE verb (an "-ing" gerund) directly before a
                        // noun head is a PRE-NOMINAL MODIFIER, not a predicate — so
                        // "a kayaking REGIMEN", "a running SHOE" are indefinite NPs
                        // and "a" is an article, not a logic variable. Only this
                        // gerund+noun shape is excluded; "a runs"/"a sees Bob"/"a =
                        // b" keep the variable reading.
                        let gerund_premodifier = matches!(
                            next.kind,
                            TokenType::Verb { aspect: crate::lexicon::Aspect::Progressive, .. }
                        ) && matches!(
                            self.tokens.get(self.current + 2).map(|t| &t.kind),
                            Some(TokenType::Noun(_)) | Some(TokenType::Ambiguous { .. })
                        );
                        !gerund_premodifier && matches!(next.kind,
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

            // A gerund (-ing form) directly before a noun head in a DEFINITE
            // description is an attributive MODIFIER, not part of the head: "the
            // hunting trip" → Trip(x) ∧ Hunt(x). Keeping the attribute as a
            // first-class predicate (un-fused) means "hunting" is the same key
            // wherever the entity is named ("the hunting vacation", "the hunting
            // trip"), so the discourse layer can resolve them to one referent.
            // Restricted to definites because reference-resolution applies to
            // definite descriptions; a bare-plural object ("used bowling pins")
            // becomes a constant with no restrictor to hold a separate predicate,
            // so its modifier must stay folded into the head symbol (Bowl_pins).
            if let TokenType::Verb {
                lemma,
                aspect: crate::lexicon::Aspect::Progressive,
                ..
            } = self.peek().kind
            {
                let next_is_head = matches!(
                    self.tokens.get(self.current + 1).map(|t| &t.kind),
                    Some(TokenType::Noun(_))
                        | Some(TokenType::ProperName(_))
                        | Some(TokenType::Ambiguous { .. })
                );
                if next_is_head && definiteness == Some(Definiteness::Definite) {
                    self.advance();
                    adjectives.push(lemma);
                    continue;
                }
            }

            // A proper name directly before a common-noun head in a DEFINITE
            // description is an attributive LABEL modifier, not part of the head:
            // "the Florida trip" → Trip(x) ∧ Florida(x); "the Woodard family" →
            // Family(x) ∧ Woodard(x). Un-fusing keeps the label a first-class key
            // so the discourse layer resolves "the Florida trip" and "the Florida
            // vacation" (or "the Woodard family" and "the Woodard estate") to one
            // referent. Definite-gated, so a bare named entity ("Lake Tahoe",
            // "Leiman Manor") stays a single constant and never un-fuses.
            if let TokenType::ProperName(label) = self.peek().kind {
                let next_is_common_head = matches!(
                    self.tokens.get(self.current + 1).map(|t| &t.kind),
                    Some(TokenType::Noun(_)) | Some(TokenType::Ambiguous { .. })
                );
                if next_is_common_head && definiteness == Some(Definiteness::Definite) {
                    self.advance();
                    // A proper name that names a DECLARED item ("Florida"
                    // registered as a state, "Bill" as a friend) un-fuses to a
                    // P(_PP_SELF_, <name>) restrictor instead of a bare predicate,
                    // converging with the prepositional-phrase form ("the trip was
                    // in Florida" → In(x, Florida); "…with Bill" → With(x, Bill)).
                    // The object term mirrors the PP form's constant EXACTLY so the
                    // two unify. An undeclared label ("the Woodard family") keeps
                    // its bare-predicate behavior.
                    let unfused = self.drs.item_category(label).and_then(|category| {
                        let cat_lemma = Self::singularize_noun(self.interner.resolve(category));
                        category_preposition(&cat_lemma.to_lowercase())
                    });
                    if let Some(prep) = unfused {
                        let prep_sym = self.interner.intern(prep);
                        let placeholder = self.interner.intern("_PP_SELF_");
                        let pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: prep_sym,
                            args: self
                                .ctx
                                .terms
                                .alloc_slice([Term::Variable(placeholder), Term::Constant(label)]),
                            world: None,
                        });
                        measure_restrictors.push(pred);
                    } else {
                        adjectives.push(label);
                    }
                    continue;
                }
            }

            // Compound color / quality term: a NOUN immediately before an
            // ADJECTIVE that is in turn followed by a HEAD NOUN ("LIME green
            // SHIRT", "MIDNIGHT blue CAR", "BLOOD red CAR") — the noun pre-modifies
            // the adjective; fold the pair into one "Lime_green" adjective. The
            // following head-noun requirement distinguishes this from a head noun
            // + a post-nominal secondary predicate ("painted the DOOR red.", where
            // "door" is the head and "red" a resultative): there no noun follows.
            let color_compound = matches!(self.peek().kind, TokenType::Noun(_))
                && matches!(
                    self.tokens.get(self.current + 1).map(|t| &t.kind),
                    Some(TokenType::Adjective(_))
                )
                && matches!(
                    self.tokens.get(self.current + 2).map(|t| &t.kind),
                    Some(TokenType::Noun(_))
                        | Some(TokenType::ProperName(_))
                        | Some(TokenType::Adjective(_))
                        | Some(TokenType::Verb { .. })
                        | Some(TokenType::Ambiguous { .. })
                );
            if color_compound {
                let n = if let TokenType::Noun(n) = self.peek().kind { n } else { unreachable!() };
                self.advance(); // the noun modifier
                let a = if let TokenType::Adjective(a) = self.peek().kind { a } else { unreachable!() };
                self.advance(); // the adjective
                let compound = self.interner.intern(&format!(
                    "{}_{}",
                    self.interner.resolve(n),
                    self.interner.resolve(a)
                ));
                adjectives.push(compound);
                continue;
            }

            // Attributive measure-adjective: "80 year old [doll]", "28 inch long
            // [wing]" — a measure phrase (Number + unit) modifying a following
            // gradable ADJECTIVE is a degree property of the head, mirroring the
            // predicative "is 80 years old" → Old(x, 80 years). Emit
            // Adj(_PP_SELF_, value) so the degree AND its unit survive rather than
            // stranding the unit token. Gated on the third token being an ADJECTIVE,
            // so "28 inch wingspan" (a measure-noun compound) is left to the head.
            let measure_num = match self.peek().kind {
                TokenType::Cardinal(n) => Some(crate::ast::logic::NumberKind::Integer(n as i64)),
                TokenType::Number(s) => {
                    let raw = self.interner.resolve(s).replace(',', "");
                    Some(
                        raw.parse::<i64>()
                            .map(crate::ast::logic::NumberKind::Integer)
                            .unwrap_or(crate::ast::logic::NumberKind::Symbolic(s)),
                    )
                }
                _ => None,
            };
            if let Some(kind) = measure_num {
                let unit_tok = self.tokens.get(self.current + 1);
                let unit_is_measure = unit_tok.map_or(false, |t| {
                    matches!(t.kind, TokenType::CalendarUnit(_))
                        || matches!(t.kind, TokenType::Noun(_)
                            if crate::lexicon::lookup_unit_dimension(
                                &self.interner.resolve(t.lexeme).to_lowercase()).is_some())
                });
                let adj_after = self.tokens.get(self.current + 2).and_then(|t| {
                    if let TokenType::Adjective(a) = t.kind { Some(a) } else { None }
                });
                if unit_is_measure {
                    if let Some(adj) = adj_after {
                        let unit_sym = unit_tok.unwrap().lexeme;
                        self.advance(); // number
                        self.advance(); // unit
                        self.advance(); // gradable adjective
                        let placeholder = self.interner.intern("_PP_SELF_");
                        let value = Term::Value { kind, unit: Some(unit_sym), dimension: None };
                        let pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: adj,
                            args: self
                                .ctx
                                .terms
                                .alloc_slice([Term::Variable(placeholder), value]),
                            world: None,
                        });
                        measure_restrictors.push(pred);
                        continue;
                    }
                }
            }

            let is_adjective = matches!(self.peek().kind, TokenType::Adjective(_));
            if !is_adjective {
                break;
            }

            // A verb-only or ambiguous word after an adjective is the head noun
            // ("the rare stamp", "the long run") — but ONLY inside a
            // determiner-headed NP. Without the article gate, "studies hard
            // pass …" would wrongly read the main verb "pass" as a noun ("hard"
            // there is an adverb). The lexicon's is_adverb misses "hard", so the
            // article is the reliable signal.
            let next_is_content = if self.current + 1 < self.tokens.len() {
                let next = &self.tokens[self.current + 1].kind;
                matches!(
                    next,
                    TokenType::Noun(_) | TokenType::Adjective(_) | TokenType::ProperName(_)
                ) || ((definiteness.is_some() || self.nominal_np_context)
                    && matches!(next, TokenType::Verb { .. } | TokenType::Ambiguous { .. }))
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

        // "the 8:15 pm event" / "the 9:30am outing" — a clock time names the head
        // noun; compound the minutes-from-midnight into the symbol so the entity is
        // identified by its time ("1215_event"), distinct from "570_outing".
        let base_noun = if let TokenType::TimeLiteral { nanos_from_midnight } = self.peek().kind {
            let minutes = nanos_from_midnight / 60_000_000_000;
            let next_is_head = matches!(
                self.tokens.get(self.current + 1).map(|t| &t.kind),
                Some(TokenType::Noun(_)) | Some(TokenType::ProperName(_))
                    | Some(TokenType::Adjective(_)) | Some(TokenType::Verb { .. })
                    | Some(TokenType::Ambiguous { .. })
            );
            if next_is_head {
                self.advance(); // consume the time literal
                let head = self.consume_content_word()?;
                let head_str = self.interner.resolve(head).to_string();
                self.interner.intern(&format!("{}_{}", minutes, head_str))
            } else {
                self.consume_content_word()?
            }
        }
        // "the 1848 home" / "the 1834 flood" — cardinal used as a year/label modifier
        // before the head noun; compound the two into a single symbol.
        else if let TokenType::Cardinal(n) = self.peek().kind {
            // The head after a count/label cardinal can tokenize as a noun, a
            // proper name, an adjective, or — for words that are also verbs
            // ("dances", "places") — a verb / ambiguous token.
            let next_is_content = self.tokens.get(self.current + 1)
                .map_or(false, |t| matches!(
                    t.kind,
                    TokenType::Noun(_)
                        | TokenType::ProperName(_)
                        | TokenType::Adjective(_)
                        | TokenType::Verb { .. }
                        | TokenType::Ambiguous { .. }
                        // "item"/"items" are nouns in declarative NL ("the $275
                        // item"); the lexer keeps them as keyword tokens.
                        | TokenType::Item
                        | TokenType::Items
                ));
            if next_is_content {
                self.advance(); // consume the cardinal
                let head = self.consume_label_head_noun_first()?;
                self.numeric_label_head(n as i64, head, definiteness, &mut measure_restrictors)
            } else if n == 1 {
                // "the one who paid $150" / "the one with 804 followers" / "the one
                // from St. Paul" — "one" used PRONOMINALLY (no following head noun)
                // is the impersonal pronoun, a generic entity, not the numeral 1.
                self.advance(); // consume "one"
                self.interner.intern("One")
            } else {
                self.consume_content_word()?
            }
        }
        // "the 2003 holiday" / "the 1850 stamp" — a YEAR/numeric label lexes as a
        // bare Number (digits), not a word-Cardinal, but plays the same role: it
        // names the head noun. Route it through the SAME label logic so a declared
        // item un-fuses to its category relation and an undeclared one stays fused.
        else if let TokenType::Number(num_sym) = self.peek().kind {
            let next_is_content = self.tokens.get(self.current + 1).map_or(false, |t| {
                matches!(
                    t.kind,
                    TokenType::Noun(_)
                        | TokenType::ProperName(_)
                        | TokenType::Adjective(_)
                        | TokenType::Verb { .. }
                        | TokenType::Ambiguous { .. }
                        | TokenType::Item
                        | TokenType::Items
                )
            });
            // Only an INTEGER label (a year / instance number) compounds with the
            // head; a unit measure ("3.5 inch") is left to the measure path, and a
            // non-integer Number never names an instance.
            let int_value = self
                .interner
                .resolve(num_sym)
                .replace(',', "")
                .parse::<i64>()
                .ok();
            match (next_is_content, int_value) {
                (true, Some(n)) => {
                    self.advance(); // consume the number
                    let head = self.consume_label_head_noun_first()?;
                    self.numeric_label_head(n, head, definiteness, &mut measure_restrictors)
                }
                _ => self.consume_content_word()?,
            }
        } else {
            self.consume_content_word()?
        };

        let noun = if let Some(prefix) = non_intersective_prefix {
            let prefix_str = self.interner.resolve(prefix);
            let base_str = self.interner.resolve(base_noun);
            let compound = format!("{}-{}", prefix_str, base_str);
            self.interner.intern(&compound)
        } else {
            base_noun
        };

        // Absorb EVERY consecutive proper-name label into the head ("Delta Gamma Pi" →
        // Delta_Gamma_Pi, "Beta Pi Omega" → Beta_Pi_Omega) — a multi-word name can be
        // 3+ words, not just two.
        //   …but in a DETERMINER-headed NP ("the prize Tara won", "the waterfall
        // Derrick photographed") a proper name that opens an object-gap reduced
        // relative is the relative's SUBJECT, NOT a label compounded into the head —
        // leave it for the reduced-relative detection below. A bare multi-word proper
        // name ("Ray Ricardo won") has no determiner, and an apposition with an overt
        // object ("the dancer Tara won the prize") has no gap, so both still compound.
        let mut noun = noun;
        while self.check_proper_name_or_label()
            && !(definiteness.is_some() && self.peek_reduced_object_relative())
        {
            let label = self.consume_content_word()?;
            noun = self.interner.intern(&format!(
                "{}_{}",
                self.interner.resolve(noun),
                self.interner.resolve(label)
            ));
        }
        let noun = noun;

        // US "City, ST" address — a place proper name followed by ", " + a
        // two-letter ALL-CAPS state abbreviation is ONE location entity
        // ("Charlestown, CT" → Charlestown_CT, "Barnstable, ME" → Barnstable_ME).
        // The all-caps gate excludes Title-case names ("Al", "Bo") and ordinary
        // list/clause commas (names are never two-letter all-caps), so this never
        // swallows a coordinator.
        let noun = if self.check(&TokenType::Comma) {
            let state_abbr = matches!(
                self.tokens.get(self.current + 1).map(|t| &t.kind),
                Some(TokenType::ProperName(s))
                    if { let l = self.interner.resolve(*s); l.len() == 2 && l.chars().all(|c| c.is_ascii_uppercase()) }
            );
            let head_is_proper = self
                .interner
                .resolve(noun)
                .chars()
                .next()
                .map_or(false, |c| c.is_ascii_uppercase());
            if state_abbr && head_is_proper {
                self.advance(); // consume the comma
                let st = self.consume_content_word()?;
                let compound = format!("{}_{}", self.interner.resolve(noun), self.interner.resolve(st));
                self.interner.intern(&compound)
            } else {
                noun
            }
        } else {
            noun
        };

        // Noun-noun compounds ("stop bit", "data bits", "grant signal"): a
        // PLAIN noun directly after the head joins it as one compound head.
        // An Ambiguous noun/verb word ("signal") joins ONLY when a copula
        // follows it — there the verb reading is impossible — so genuine
        // ambiguity ("Time flies like an arrow") keeps its verb reading and
        // the forest enumerates the rest.
        let mut noun = noun;
        loop {
            // The function word "as" lexes as a Noun (unknown-word fallback) but
            // is never a compound-noun part — it introduces a predicative
            // complement ("has Al Acosta AS its mayor"). Stop the compound here so
            // the "as"-phrase is left for the verb path, not bundled into the head.
            if matches!(self.peek().kind, TokenType::Noun(_))
                && self.interner.resolve(self.peek().lexeme).eq_ignore_ascii_case("as")
            {
                break;
            }
            let next = match &self.peek().kind {
                TokenType::Noun(next) => *next,
                TokenType::Ambiguous { primary, alternatives } => {
                    let noun_reading = if let TokenType::Noun(n) = &**primary {
                        Some(*n)
                    } else {
                        alternatives.iter().find_map(|t| {
                            if let TokenType::Noun(n) = t { Some(*n) } else { None }
                        })
                    };
                    let copula_after = matches!(
                        self.tokens.get(self.current + 1).map(|t| &t.kind),
                        Some(TokenType::Is)
                            | Some(TokenType::Are)
                            | Some(TokenType::Was)
                            | Some(TokenType::Were)
                    );
                    // Inside an OBJECT/complement NP (greedy == false the clause
                    // already has its main verb) an ambiguous noun/verb word that
                    // is NOT itself followed by its own argument cannot be a verb —
                    // it joins the head as a compound ("has a glass head.", "holds
                    // a paper clip.", "played second base WON"). The following
                    // token confirms this: a clause boundary (Period/EOF/comma/
                    // and/or) or a FINITE verb/copula (the matrix or next clause's
                    // verb — "second base" then matrix "won"/"is") both rule out a
                    // verb reading of the ambiguous word. A clause subject (greedy
                    // == true) keeps the verb reading so "The man runs." is not
                    // eaten; wh-extraction is excluded (the gap leaves the embedded
                    // verb at a boundary, "…Mary said Bill saw?").
                    let object_compound_boundary = !greedy
                        && self.filler_gap.is_none()
                        && matches!(
                            self.tokens.get(self.current + 1).map(|t| &t.kind),
                            None | Some(TokenType::Period)
                                | Some(TokenType::EOF)
                                | Some(TokenType::Comma)
                                | Some(TokenType::And)
                                | Some(TokenType::Or)
                                | Some(TokenType::Verb { .. })
                                | Some(TokenType::Is)
                                | Some(TokenType::Are)
                                | Some(TokenType::Was)
                                | Some(TokenType::Were)
                        );
                    // A nominal context (copula complement / PP object /
                    // comparative standard) likewise rules out the verb reading:
                    // "is the Russell Road PROJECT", "the $5.25 PURCHASE".
                    // After a numeric-MEASURE head ("60_gallon …"), the following
                    // words are the measured head COMPOUND ("60 gallon FISH TANK"), so
                    // an ambiguous noun/verb word ("fish") joins the head even in a
                    // greedy subject — a measure premodifier cannot be followed by a
                    // finite verb, so the ambiguity resolves to noun.
                    let head_is_numeric_measure = self
                        .interner
                        .resolve(noun)
                        .chars()
                        .next()
                        .map_or(false, |c| c.is_ascii_digit());
                    match (
                        noun_reading,
                        copula_after
                            || object_compound_boundary
                            || self.nominal_np_context
                            || head_is_numeric_measure,
                    ) {
                        (Some(n), true) => n,
                        _ => break,
                    }
                }
                // A verb-only word ("stamp", "print") after a numeric/label head
                // is a compound-noun part ("the 125000 stamp"). A bare number
                // cannot head an NP that acts, so a numeric head takes the
                // following verb-word as its real head noun whatever follows —
                // BUT only in BASE form (surface == lemma): "stamp"/"print" read
                // as nouns, while an inflected form ("in 1850 sold …", "runs") is
                // a genuine verb and must not be eaten. Otherwise we require a
                // copula after so "the man runs fast" keeps "runs" as the verb.
                TokenType::Verb { lemma, .. } => {
                    let head_is_numeric = {
                        // Accept decimal/grouped money amounts as numeric heads
                        // ("$5.25 purchase" → 5.25, "$1,800 stamp" → 1,800) so the
                        // deverbal-noun fold fires — a bare number can't head an
                        // acting NP, so the following verb-word is its real head.
                        let h = self.interner.resolve(noun);
                        !h.is_empty()
                            && h.chars().any(|c| c.is_ascii_digit())
                            && h.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ',')
                    };
                    let is_base_form = self
                        .interner
                        .resolve(self.peek().lexeme)
                        .eq_ignore_ascii_case(self.interner.resolve(*lemma));
                    let copula_after = matches!(
                        self.tokens.get(self.current + 1).map(|t| &t.kind),
                        Some(TokenType::Is)
                            | Some(TokenType::Are)
                            | Some(TokenType::Was)
                            | Some(TokenType::Were)
                    );
                    // A base-form verb-word is a deverbal noun ONLY at the NP TAIL —
                    // a clause boundary / finite verb / copula follows. If a VP
                    // continuation follows instead (adverb, object, PP), it is the
                    // MATRIX verb and must NOT be eaten: "the goods from Spain SELL
                    // quickly" (a PP-object/nominal context, base "sell" + adverb)
                    // keeps "sell" as the verb. This is the same NP-tail test the
                    // object-boundary case below uses.
                    let next_ends_np = matches!(
                        self.tokens.get(self.current + 1).map(|t| &t.kind),
                        None | Some(TokenType::Period)
                            | Some(TokenType::EOF)
                            | Some(TokenType::Comma)
                            | Some(TokenType::And)
                            | Some(TokenType::Or)
                            | Some(TokenType::Verb { .. })
                            | Some(TokenType::Is)
                            | Some(TokenType::Are)
                            | Some(TokenType::Was)
                            | Some(TokenType::Were)
                    );
                    // A BASE-FORM verb-word after a noun head, inside a NOMINAL
                    // context (a PP object / comparative standard) and at the NP tail,
                    // is a deverbal noun-noun COMPOUND: "a cork COVER", "an amber BASE",
                    // "a tataki ROLL". The base-form gate excludes inflected matrix
                    // verbs (runs/votes/sold); the context gate keeps SUBJECT NPs out.
                    let nominal_compound = self.nominal_np_context && is_base_form && next_ends_np;
                    // A CAPITALIZED verb-word after an already MULTI-WORD proper-name
                    // head ("Bald_Hill" + "Run") is the tail of a place name, not a
                    // verb. The head MUST already be compounded (contains '_') — this
                    // is what separates a place name from a SUBJECT + matrix verb
                    // ("John" + idiom lemma "Die" → must stay John kicked-the-bucket,
                    // not John_Die). Both sides capitalized + a clause boundary after.
                    let proper_name_part = self.interner.resolve(noun).contains('_')
                        && self.interner.resolve(noun).chars().next()
                            .map_or(false, |c| c.is_ascii_uppercase())
                        && self.interner.resolve(self.peek().lexeme).chars().next()
                            .map_or(false, |c| c.is_ascii_uppercase())
                        && matches!(
                            self.tokens.get(self.current + 1).map(|t| &t.kind),
                            None | Some(TokenType::Period) | Some(TokenType::EOF)
                                | Some(TokenType::Comma) | Some(TokenType::And)
                                | Some(TokenType::Or)
                        );
                    // A BASE-FORM verb-word in an OBJECT/complement NP (!greedy:
                    // the clause already has its verb), NOT followed by its own
                    // argument, is a deverbal noun joining the head ("the spicy
                    // tataki ROLL", "the onion DIP"). The following token confirms
                    // it: a clause boundary, or a finite verb/copula (the matrix or
                    // next clause's verb) — both rule out a verb reading. A PRONOUN
                    // after it ("own a donkey BEAT it") is NOT a boundary, so the
                    // nuclear verb of a quantifier is left alone. Mirrors the
                    // Ambiguous arm's object_compound_boundary.
                    let object_boundary =
                        !greedy && self.filler_gap.is_none() && is_base_form && next_ends_np;
                    // A clause-final GERUND (-ing) after a noun head in an object /
                    // nominal NP is a noun-incorporation compound ("started WEIGHT
                    // LIFTING", "enjoys BIRD WATCHING") — the noun is the gerund's
                    // incorporated object. Requires the NP tail (next_ends_np) so a
                    // reduced relative with its own object ("the man lifting WEIGHTS")
                    // is untouched. Uses the SURFACE form so the compound is
                    // weight_lifting, not the lemma weight_lift.
                    let gerund_compound = (!greedy || self.nominal_np_context)
                        && self.filler_gap.is_none()
                        && self
                            .interner
                            .resolve(self.peek().lexeme)
                            .to_lowercase()
                            .ends_with("ing")
                        && next_ends_np;
                    if (head_is_numeric && is_base_form) || copula_after || nominal_compound || proper_name_part || object_boundary {
                        *lemma
                    } else if gerund_compound {
                        self.peek().lexeme
                    } else {
                        break;
                    }
                }
                _ => break,
            };
            self.advance();
            let head_str = self.interner.resolve(noun);
            let next_str = self.interner.resolve(next);
            let compound = format!("{}_{}", head_str, next_str);
            noun = self.interner.intern(&compound);
        }

        // Head noun + numeric value: an instance/slot LABEL — "number 7",
        // "room 204", "lane 3", "version 2", "exhibit 5", "car 7". This is a
        // GENERAL grammatical pattern, not a hardcoded word list (which would
        // never generalise to unseen clues): in English a common noun is
        // otherwise never directly followed by a BARE number, so a head noun
        // immediately followed by a Cardinal/Number names a specific instance —
        // join them into one symbol. Both word-numbers ("seven") and digits ("7")
        // apply. The one exception is a MEASURE ("a box 3 FEET tall"), where the
        // number begins a unit phrase — a following CalendarUnit / registered unit
        // word vetoes the label reading. Gated to declarative (NL) parsing so it
        // never disturbs LOGOS imperative index syntax ("item 3 of arr").
        let label_value: Option<String> = match self.peek().kind {
            TokenType::Cardinal(n) => Some(n.to_string()),
            TokenType::Number(sym) => Some(self.interner.resolve(sym).to_string()),
            _ => None,
        };
        if let Some(value) = label_value {
            let number_starts_measure = match self.tokens.get(self.current + 1) {
                Some(t) => matches!(t.kind, TokenType::CalendarUnit(_))
                    || crate::lexicon::lookup_unit_dimension(
                        &self.interner.resolve(t.lexeme).to_lowercase(),
                    )
                    .is_some(),
                None => false,
            };
            let head_str = self.interner.resolve(noun).to_string();
            // A numeric head ("1850") is not a label base; only a real word is.
            let head_is_word = !head_str.is_empty() && !head_str.chars().all(|c| c.is_ascii_digit());
            // A MONTH head ("April 15") is a date, handled by the month+day rule
            // just below — never an instance label.
            let head_is_month = crate::lexicon::is_month(&head_str.to_lowercase());
            if !number_starts_measure
                && head_is_word
                && !head_is_month
                && self.mode == super::ParserMode::Declarative
            {
                self.advance();
                noun = self.interner.intern(&format!("{}_{}", head_str, value));
            }
        }

        // Month + day: "June 11", "May 3", "December 25" — the day (1–31) joins the
        // month into a single date symbol ("June_11"). Month names are a closed
        // lexical class (lexicon `months`).
        {
            let head_lower = self.interner.resolve(noun).to_lowercase();
            if crate::lexicon::is_month(&head_lower) {
                let day = match self.peek().kind {
                    TokenType::Cardinal(n) if (1..=31).contains(&n) => Some(n),
                    // A numeric day may carry an ordinal suffix ("15th", "3rd",
                    // "1st", "2nd"); strip it so "April 15th" and "April 15" name
                    // the same date ("April_15").
                    TokenType::Number(s) => {
                        let raw = self.interner.resolve(s);
                        let digits = raw.trim_end_matches(|c: char| c.is_ascii_alphabetic());
                        digits.parse::<u32>().ok().filter(|d| (1..=31).contains(d))
                    }
                    _ => None,
                };
                if let Some(day) = day {
                    self.advance();
                    let head_str = self.interner.resolve(noun);
                    noun = self.interner.intern(&format!("{}_{}", head_str, day));
                    // An attributive date modifies a following head noun ("the
                    // April 15th birthday" → April_15_birthday); absorb it so the
                    // real head isn't stranded.
                    while let TokenType::Noun(next) = self.peek().kind {
                        self.advance();
                        noun = self.interner.intern(&format!(
                            "{}_{}",
                            self.interner.resolve(noun),
                            self.interner.resolve(next)
                        ));
                    }
                }
            }
        }

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

            let mut possessed_noun = self.consume_content_word()?;
            // A multi-word possessed compound noun ("Bernard's fountain pen",
            // "Joe's yoga session") joins its consecutive nouns into one symbol,
            // mirroring the head noun-noun compounding above.
            while let TokenType::Noun(next) = self.peek().kind {
                self.advance();
                possessed_noun = self.interner.intern(&format!(
                    "{}_{}",
                    self.interner.resolve(possessed_noun),
                    self.interner.resolve(next)
                ));
            }

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
        // Attributive measure-adjectives ("80 year old") are core restrictors of the
        // head — collected before it — so they attach unconditionally, ahead of any
        // optional trailing PPs.
        pps.extend(measure_restrictors.iter().copied());

        // An "of <measure>" restrictor ("a maximum range OF 475 ft", "a book OF
        // 500 pages") names the head noun's measured value. It is UNAMBIGUOUSLY a
        // noun restrictor (never an event adjunct or a partitive), so attach it
        // even in a non-greedy object NP — otherwise "has a maximum range of 475
        // ft" silently strands the measure.
        loop {
            let of_measure = self.check_of_preposition()
                && matches!(
                    self.tokens.get(self.current + 1).map(|t| &t.kind),
                    Some(TokenType::Number(_)) | Some(TokenType::Cardinal(_))
                );
            if !of_measure {
                break;
            }
            let of_sym = match self.advance().kind {
                TokenType::Preposition(s) => s,
                _ => break,
            };
            let placeholder_var = self.interner.intern("_PP_SELF_");
            let measure = self.parse_measure_phrase()?;
            pps.push(self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: of_sym,
                args: self
                    .ctx
                    .terms
                    .alloc_slice([Term::Variable(placeholder_var), *measure]),
                world: None,
            }));
        }

        // Postposed measure-adjective "worth <measure>" ("the magnate WORTH $27
        // billion", "a stamp WORTH $50") — a postnominal adjective taking a measure
        // complement, the same Worth(x, measure) the copula complement builds. Without
        // this "worth" strands (TrailingTokens{Adjective}). Surface it as a restrictor
        // over the _PP_SELF_ placeholder so it lowers onto the NP's entity.
        if matches!(self.peek().kind, TokenType::Adjective(_))
            && self
                .interner
                .resolve(self.peek().lexeme)
                .eq_ignore_ascii_case("worth")
            && matches!(
                self.tokens.get(self.current + 1).map(|t| &t.kind),
                Some(TokenType::Number(_)) | Some(TokenType::Cardinal(_))
            )
        {
            let worth_sym = if let TokenType::Adjective(s) = self.peek().kind {
                s
            } else {
                unreachable!()
            };
            self.advance(); // "worth"
            let measure = self.parse_measure_phrase()?;
            let placeholder = self.interner.intern("_PP_SELF_");
            // Push directly to `pps` — the measure_restrictors → pps merge already ran
            // above (this is the post-head section).
            pps.push(self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: worth_sym,
                args: self
                    .ctx
                    .terms
                    .alloc_slice([Term::Variable(placeholder), *measure]),
                world: None,
            }));
        }

        if should_attach_pps {
            // "of" normally starts a partitive/genitive handled elsewhere, so it
            // is excluded here — EXCEPT "of <measure>" ("a range of 650 ft", "a
            // book of 500 pages"), where the of-phrase specifies the head noun's
            // measured value. That is a genuine restrictor, not a partitive, so
            // attach it as an Of(self, <measure>) PP.
            let of_measure_follows = |p: &Self| {
                p.check_of_preposition()
                    && matches!(
                        p.tokens.get(p.current + 1).map(|t| &t.kind),
                        Some(TokenType::Number(_)) | Some(TokenType::Cardinal(_))
                    )
            };
            while self.check_preposition() && (!self.check_of_preposition() || of_measure_follows(self)) {
                let prep_token = self.advance().clone();
                let prep_name = if let TokenType::Preposition(sym) = prep_token.kind {
                    sym
                } else {
                    break;
                };

                let placeholder_var = self.interner.intern("_PP_SELF_");
                if self.check_number()
                    && !matches!(
                        self.tokens.get(self.current + 2).map(|t| &t.kind),
                        Some(TokenType::Noun(_)) | Some(TokenType::Ambiguous { .. })
                    )
                {
                    // Numeric PP object ("with 15 people", "at 385 degrees"):
                    // keep the measured amount as the PP's object. A noun/ambiguous
                    // head after the unit ("with 205 degree WATER") is instead a
                    // measure-premodified noun, folded by the NP branch below.
                    let measure = self.parse_measure_phrase()?;
                    let pp_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: prep_name,
                        args: self
                            .ctx
                            .terms
                            .alloc_slice([Term::Variable(placeholder_var), *measure]),
                        world: None,
                    });
                    pps.push(pp_pred);
                } else if self.check_content_word()
                    || self.check_number()
                    || matches!(self.peek().kind, TokenType::Article(_))
                {
                    // A PP object is an unambiguously NOMINAL tail, so a base-form
                    // verb-word after its noun head is a deverbal compound ("with a
                    // cork COVER", "with a faux leather COVER"), not a matrix verb.
                    let saved_ctx = self.nominal_np_context;
                    self.nominal_np_context = true;
                    let pp_object_result = self.parse_noun_phrase(true);
                    self.nominal_np_context = saved_ctx;
                    let pp_object = pp_object_result?;
                    let pp_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: prep_name,
                        args: self.ctx.terms.alloc_slice([
                            Term::Variable(placeholder_var),
                            Term::Constant(pp_object.noun),
                        ]),
                        world: None,
                    });
                    pps.push(pp_pred);
                    // The PP object's own ADJECTIVES ("with the BLUE hat" →
                    // With(self, Hat) ∧ Blue(Hat)) and nested restrictors ("with a
                    // maximum range of 650 ft" → … ∧ Of(Range, 650 ft)) survive via
                    // the shared recovery (the single source of truth for every PP
                    // position); dropping them would be meaning loss.
                    pps.extend(self.pp_object_modifier_preds(&pp_object));
                } else {
                    break;
                }
            }

            // Active object-gap reduced relative ("the waterfall Derrick photographed
            // in 1989" = the waterfall [that] Derrick photographed): a determiner-
            // headed NP whose head is followed by a reduced-relative subject+verb with
            // an EMPTY object slot (the gap is THIS head; English drops the
            // relativizer). The empty-object + transitive test (peek_reduced_object_relative)
            // is the PROPER deterministic discriminator — an overt object is apposition,
            // an intransitive verb has no gap. Reuse parse_relative_clause with the
            // `_PP_SELF_` placeholder so the clause — and its event complements ("in
            // 1989") — attach as a restrictor wherever the NP flows.
            if definiteness.is_some() && self.peek_reduced_object_relative() {
                let placeholder = self.interner.intern("_PP_SELF_");
                let rel = self.parse_relative_clause(placeholder)?;
                pps.push(rel);
            }

            // Post-nominal "-ing" reduced relative ("the person arriving at
            // Paradise", "the assignment beginning in June", "the conductor
            // working on June 11"): a present participle after the noun is
            // unambiguously a reduced relative — it cannot be a finite main verb
            // without an auxiliary — so it restricts THIS noun phrase wherever the
            // NP appears (subject, standard, of-pair member, predicate nominal).
            // Attach as predicates over the `_PP_SELF_` placeholder (substituted
            // to the NP's variable when the NP is wrapped).
            if let TokenType::Verb { lemma, .. } = self.peek().kind {
                let is_ing = self
                    .interner
                    .resolve(self.peek().lexeme)
                    .to_lowercase()
                    .ends_with("ing");
                // A past participle followed by a "by"-agent is an unambiguous
                // PASSIVE reduced relative ("the bird trained BY the falconer",
                // "the photo published BY Wildzone") — a finite main verb is not
                // followed by a by-phrase in this position.
                let passive_by = matches!(
                    self.tokens.get(self.current + 1).map(|t| &t.kind),
                    Some(TokenType::Preposition(s))
                        if self.interner.resolve(*s).eq_ignore_ascii_case("by")
                );
                // A past participle of a TRANSITIVE verb immediately followed by a
                // PP (no object) is a passive reduced relative ("the medicine
                // sourced FROM a fig", "the item made OF teak", "the flower grown
                // IN Olin"). Lexical transitivity disambiguates it from an
                // intransitive main clause ("the box arrived in April"), which a
                // bare common noun + PP would otherwise look like.
                //
                // The near-dead `is_transitive_verb` table (47/2623 verbs) misses
                // most transitives ("the gator CAUGHT in Lynn" — catch has past ==
                // participle, so the distinct-form rule can't save it either). In a
                // NOMINAL complement position (`nominal_np_context`: "X is the gator
                // caught in Lynn") the NP cannot be a main-clause subject, so a
                // past-participle + PP IS a reduced relative for any verb that is not
                // marked pure-intransitive — the same transitive-capable-by-default
                // rule the object-gap relative uses. In subject position the strict
                // table still gates it (keeping "The team won in 1989" a main clause).
                let transitive_passive = matches!(
                    self.peek().kind,
                    TokenType::Verb { time: crate::lexicon::Time::Past, .. }
                ) && matches!(
                    self.tokens.get(self.current + 1).map(|t| &t.kind),
                    Some(TokenType::Preposition(_))
                ) && (crate::lexicon::is_transitive_verb(
                    &self.interner.resolve(lemma).to_lowercase(),
                ) || (self.nominal_np_context
                    && !crate::lexicon::is_intransitive_verb(
                        &self.interner.resolve(lemma).to_lowercase(),
                    )));
                // A DISTINCT past-participle form (participle ≠ past: "grown" vs
                // "grew", "taken" vs "took") immediately followed by a PP is an
                // unambiguous passive reduced relative regardless of transitivity —
                // the form alone is non-finite, like "-ing" ("the flower GROWN in
                // Hardy", "the package TAKEN to the depot").
                let distinct_participle_passive = matches!(
                    self.tokens.get(self.current + 1).map(|t| &t.kind),
                    Some(TokenType::Preposition(_))
                ) && crate::lexicon::is_distinct_past_participle(
                    &self.interner.resolve(self.peek().lexeme).to_lowercase(),
                );
                if is_ing || passive_by || transitive_passive || distinct_participle_passive {
                    self.advance();
                    let placeholder = self.interner.intern("_PP_SELF_");
                    // An ACTIVE -ing reduced relative can take a DIRECT OBJECT
                    // ("the origami DEPICTING a dragon", "the survivor BRINGING the
                    // rope") → Participle(x, object). Only -ing (a passive
                    // participle's patient is the head itself), and only a real
                    // object NP (article / content word), never a preposition or
                    // the matrix verb.
                    let direct_obj = if is_ing
                        && (matches!(self.peek().kind, TokenType::Article(_))
                            || self.check_content_word())
                        && !self.check_preposition()
                        && !self.check_verb()
                    {
                        Some(self.parse_noun_phrase(true)?)
                    } else {
                        None
                    };
                    let participle_pred = if let Some(ref obj) = direct_obj {
                        self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: lemma,
                            args: self.ctx.terms.alloc_slice([
                                Term::Variable(placeholder),
                                Term::Constant(obj.noun),
                            ]),
                            world: None,
                        })
                    } else {
                        self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: lemma,
                            args: self.ctx.terms.alloc_slice([Term::Variable(placeholder)]),
                            world: None,
                        })
                    };
                    pps.push(participle_pred);
                    // The participle's PP / directional-"to" complements. "of" is
                    // allowed here ("made OF teak", "made OF sandalwood") — it is
                    // the participle's material complement, not a possessive.
                    loop {
                        let prep = if self.check_preposition() {
                            match self.advance().kind {
                                TokenType::Preposition(s) => s,
                                _ => break,
                            }
                        } else if self.check(&TokenType::To)
                            && matches!(
                                self.tokens.get(self.current + 1).map(|t| &t.kind),
                                Some(TokenType::Article(_))
                                    | Some(TokenType::Noun(_))
                                    | Some(TokenType::ProperName(_))
                            )
                        {
                            self.advance();
                            self.interner.intern("To")
                        } else {
                            break;
                        };
                        if self.check_number() {
                            // A numeric PP object in a reduced relative: "found IN
                            // 1992", "sent in 1976", "donated in 2010" — keep the
                            // year/amount, else it strands.
                            let measure = self.parse_measure_phrase()?;
                            pps.push(self.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: prep,
                                args: self.ctx.terms.alloc_slice([
                                    Term::Variable(placeholder),
                                    *measure,
                                ]),
                                world: None,
                            }));
                        } else if self.check_content_word()
                            || matches!(self.peek().kind, TokenType::Article(_))
                        {
                            let obj = self.parse_noun_phrase(true)?;
                            pps.push(self.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: prep,
                                args: self.ctx.terms.alloc_slice([
                                    Term::Variable(placeholder),
                                    Term::Constant(obj.noun),
                                ]),
                                world: None,
                            }));
                        } else {
                            break;
                        }
                    }
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

    fn numeric_label_head(
        &mut self,
        n: i64,
        head: crate::intern::Symbol,
        definiteness: Option<Definiteness>,
        measure_restrictors: &mut Vec<&'a LogicExpr<'a>>,
    ) -> crate::intern::Symbol {
        // A numeric label that names a DECLARED item ("2003" registered as a
        // year) un-fuses: the label becomes the head noun PLUS a
        // P(_PP_SELF_, <number>) restrictor, converging with the
        // prepositional-phrase form ("the holiday was in 2003" →
        // Holiday(x) ∧ In(x, 2003)). The numeric term mirrors the PP form's
        // measure value EXACTLY so the two unify. Un-fusing is gated to definite
        // descriptions (a label refers); an undeclared number — or a category
        // that maps to no preposition — keeps the fused symbol.
        if definiteness == Some(Definiteness::Definite) {
            let item_sym = self.interner.intern(&n.to_string());
            if let Some(category) = self.drs.item_category(item_sym) {
                let cat_lemma = Self::singularize_noun(self.interner.resolve(category));
                if let Some(prep) = category_preposition(&cat_lemma.to_lowercase()) {
                    let prep_sym = self.interner.intern(prep);
                    let placeholder = self.interner.intern("_PP_SELF_");
                    let value = Term::Value {
                        kind: crate::ast::logic::NumberKind::Integer(n),
                        unit: None,
                        dimension: None,
                    };
                    let pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: prep_sym,
                        args: self
                            .ctx
                            .terms
                            .alloc_slice([Term::Variable(placeholder), value]),
                        world: None,
                    });
                    measure_restrictors.push(pred);
                    return head;
                }
            }
        }
        let head_str = self.interner.resolve(head);
        self.interner.intern(&format!("{}_{}", n, head_str))
    }

    fn consume_label_head_noun_first(&mut self) -> ParseResult<crate::intern::Symbol> {
        let noun_sym = if let TokenType::Ambiguous { primary, alternatives } = &self.peek().kind {
            let from_primary = match **primary {
                TokenType::Noun(s) | TokenType::Adjective(s) => Some(s),
                _ => None,
            };
            from_primary.or_else(|| {
                alternatives.iter().find_map(|t| match t {
                    TokenType::Noun(s) | TokenType::Adjective(s) => Some(*s),
                    _ => None,
                })
            })
        } else {
            None
        };
        if let Some(s) = noun_sym {
            self.advance();
            return Ok(s);
        }
        self.consume_content_word()
    }

    fn check_possessive(&self) -> bool {
        matches!(self.peek().kind, TokenType::Possessive)
    }

    /// Whether the cursor is at the SUBJECT of an active object-gap reduced relative
    /// ("the prize | Tara won", cursor at "Tara"). The PROPER, deterministic test
    /// (no trial-parse): a proper-name / pronoun subject, then a TRANSITIVE verb
    /// (only a transitive verb has an object slot for the head to fill — an
    /// intransitive "the dancer Tara performed" is apposition, not a relative), then
    /// an EMPTY object slot — the token after the verb does NOT start a direct object
    /// (an overt object "the dancer Tara won THE PRIZE" is apposition, not a gap).
    /// The caller additionally requires a determiner-headed NP.
    fn peek_reduced_object_relative(&self) -> bool {
        if !matches!(self.peek().kind, TokenType::ProperName(_) | TokenType::Pronoun { .. }) {
            return false;
        }
        // A token that would START a direct/prepositional object — its presence after
        // the verb means the slot is filled (apposition), so there is no gap.
        let starts_object = |kind: Option<&TokenType>| {
            matches!(
                kind,
                Some(TokenType::Article(_))
                    | Some(TokenType::Noun(_))
                    | Some(TokenType::ProperName(_))
                    | Some(TokenType::Number(_))
                    | Some(TokenType::Cardinal(_))
                    | Some(TokenType::Possessive)
                    | Some(TokenType::Pronoun { .. })
                    | Some(TokenType::All)
                    | Some(TokenType::Some)
                    | Some(TokenType::No)
                    | Some(TokenType::Any)
                    | Some(TokenType::Most)
                    | Some(TokenType::Few)
                    | Some(TokenType::Many)
            )
        };
        // Assume transitive-CAPABLE by default (English verbs overwhelmingly are);
        // only a verb marked pure-intransitive has no DIRECT object slot to fill.
        let verb_is_intransitive = self.tokens.get(self.current + 1).map_or(true, |t| {
            matches!(t.kind, TokenType::Verb { lemma, .. }
                if crate::lexicon::is_intransitive_verb(
                    &self.interner.resolve(lemma).to_lowercase()))
        });
        let verb_follows = matches!(
            self.tokens.get(self.current + 1).map(|t| &t.kind),
            Some(TokenType::Verb { .. })
        );
        if !verb_follows {
            return false;
        }
        // A STRANDED preposition after the verb ("the friend Simon went WITH", "the
        // animal Eva works WITH") makes the GAP the object of that preposition, so
        // even a pure-intransitive verb heads the relative. The preposition's own
        // object slot must be empty (the next token does not start an NP).
        let stranded_prep = matches!(
            self.tokens.get(self.current + 2).map(|t| &t.kind),
            Some(TokenType::Preposition(_))
        ) && !starts_object(self.tokens.get(self.current + 3).map(|t| &t.kind));
        if stranded_prep {
            return true;
        }
        // Otherwise a transitive verb with an EMPTY direct-object slot (the head is
        // the gap); an overt object after the verb is apposition, not a gap.
        if verb_is_intransitive {
            return false;
        }
        !starts_object(self.tokens.get(self.current + 2).map(|t| &t.kind))
    }

    fn peek_definite_reduced_relative_object(&self) -> bool {
        // The cursor must open a DEFINITE NP.
        if !matches!(self.peek().kind, TokenType::Article(Definiteness::Definite)) {
            return false;
        }
        // Skip the article, then any adjectives, to land on the noun head.
        let mut p = self.current + 1;
        while matches!(
            self.tokens.get(p).map(|t| &t.kind),
            Some(TokenType::Adjective(_)) | Some(TokenType::NonIntersectiveAdjective(_))
        ) {
            p += 1;
        }
        // The relativized head: a common noun.
        if !matches!(
            self.tokens.get(p).map(|t| &t.kind),
            Some(TokenType::Noun(_))
                | Some(TokenType::CalendarUnit(_))
                | Some(TokenType::Ambiguous { .. })
        ) {
            return false;
        }
        let subj = p + 1;
        // The relative's overt subject is a fresh ProperName / Pronoun.
        if !matches!(
            self.tokens.get(subj).map(|t| &t.kind),
            Some(TokenType::ProperName(_)) | Some(TokenType::Pronoun { .. })
        ) {
            return false;
        }
        // …immediately followed by the relative's finite verb.
        let vp = subj + 1;
        if !matches!(
            self.tokens.get(vp).map(|t| &t.kind),
            Some(TokenType::Verb { .. })
        ) {
            return false;
        }
        // Either a stranded preposition (gap = its object: "the friend Simon went
        // WITH") or a transitive verb with an empty direct-object slot (gap = the
        // head: "the waterfall Derrick photographed"). A filled slot after the verb
        // is apposition, not a gap.
        let starts_object = |kind: Option<&TokenType>| {
            matches!(
                kind,
                Some(TokenType::Article(_))
                    | Some(TokenType::Noun(_))
                    | Some(TokenType::ProperName(_))
                    | Some(TokenType::Number(_))
                    | Some(TokenType::Cardinal(_))
                    | Some(TokenType::Possessive)
                    | Some(TokenType::Pronoun { .. })
                    | Some(TokenType::All)
                    | Some(TokenType::Some)
                    | Some(TokenType::No)
                    | Some(TokenType::Any)
                    | Some(TokenType::Most)
                    | Some(TokenType::Few)
                    | Some(TokenType::Many)
            )
        };
        let after_verb = self.tokens.get(vp + 1).map(|t| &t.kind);
        if matches!(after_verb, Some(TokenType::Preposition(_)))
            && !starts_object(self.tokens.get(vp + 2).map(|t| &t.kind))
        {
            return true;
        }
        let verb_is_intransitive = self.tokens.get(vp).map_or(true, |t| {
            matches!(t.kind, TokenType::Verb { lemma, .. }
                if crate::lexicon::is_intransitive_verb(
                    &self.interner.resolve(lemma).to_lowercase()))
        });
        if verb_is_intransitive {
            return false;
        }
        !starts_object(after_verb)
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
                let is_possessive = matches!(
                    **primary,
                    TokenType::Pronoun { case: Case::Possessive, .. }
                ) || alternatives.iter().any(|alt| {
                    matches!(alt, TokenType::Pronoun { case: Case::Possessive, .. })
                });
                if !is_possessive {
                    return false;
                }
                if self.noun_priority_mode {
                    return true;
                }
                // Outside noun-priority contexts, the possessive reading of an
                // object/possessive-ambiguous pronoun ("her") applies exactly
                // when an NP head follows: "saw her dog" vs "saw her".
                self.possessive_np_head_follows()
            }
            _ => false,
        }
    }
}
