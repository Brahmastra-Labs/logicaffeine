//! Clause-level parsing: sentences, conditionals, conjunctions, and relative clauses.
//!
//! This module handles the top-level sentence structures including:
//!
//! - **Simple sentences**: Subject-verb-object patterns
//! - **Conditionals**: "If P then Q" with DRS scope handling
//! - **Counterfactuals**: "If P were/had, Q would" (subjunctive)
//! - **Disjunctions**: "Either P or Q", "P or Q"
//! - **Conjunctions**: "P and Q"
//! - **Relative clauses**: "who/that/which" attaching to noun phrases
//! - **VP ellipsis**: "John ran and Mary did too"
//!
//! The [`ClauseParsing`] trait defines the interface implemented by [`Parser`].

use super::modal::ModalParsing;
use super::noun::NounParsing;
use super::pragmatics::PragmaticsParsing;
use super::quantifier::QuantifierParsing;
use super::question::QuestionParsing;
use super::verb::LogicVerbParsing;
use super::{EventTemplate, ParseResult, Parser};
use crate::ast::{AspectOperator, LogicExpr, NeoEventData, NounPhrase, QuantifierKind, TemporalOperator, Term, ThematicRole};
use crate::lexer::Lexer;
use crate::lexicon::Time;
use crate::drs::{BoxType, Gender, Number};
use super::ParserMode;
use crate::error::{ParseError, ParseErrorKind};
use logicaffeine_base::Symbol;
use crate::lexicon::Definiteness;
use crate::token::TokenType;

/// Whether `kind` can BEGIN a clause subject (an NP head/determiner), as opposed to
/// a copula or verb. Used to tell a fronted temporal adjunct ("Every year SIMON
/// takes …") from a temporal SUBJECT ("Every year IS long").
fn starts_clause_subject(kind: &TokenType) -> bool {
    matches!(
        kind,
        TokenType::ProperName(_)
            | TokenType::Noun(_)
            | TokenType::Article(_)
            | TokenType::Pronoun { .. }
            | TokenType::All
            | TokenType::No
            | TokenType::Some
            | TokenType::Any
            | TokenType::Most
            | TokenType::Few
            | TokenType::Many
            | TokenType::Cardinal(_)
            | TokenType::Number(_)
    )
}

/// One side of an "Of A and B, …" pair. A bare proper name is a referring
/// CONSTANT (`is_var == false`); anything with a determiner, adjective,
/// possessor, PP, or relative clause is a DESCRIPTION carried by a fresh
/// existential VARIABLE plus a `restrictor` predicate over it — so two NPs that
/// share a head noun ("the red stamp" / "the blue stamp") stay distinct instead
/// of collapsing to one constant.
struct OfEntity<'a> {
    sym: Symbol,
    is_var: bool,
    term: Term<'a>,
    restrictor: Option<&'a LogicExpr<'a>>,
}

/// Trait for parsing clause-level structures.
///
/// Defines methods for parsing sentences, conditionals, conjunctions,
/// and other clause-level constructs.
pub trait ClauseParsing<'a, 'ctx, 'int> {
    /// Parses a complete sentence, handling imperatives, ellipsis, and questions.
    fn parse_sentence(&mut self) -> ParseResult<&'a LogicExpr<'a>>;
    /// Parses "if P then Q" conditionals with DRS scope handling.
    fn parse_conditional(&mut self) -> ParseResult<&'a LogicExpr<'a>>;
    /// Parses "either P or Q" exclusive disjunctions.
    fn parse_either_or(&mut self) -> ParseResult<&'a LogicExpr<'a>>;
    /// Parses "P or Q" disjunctions.
    fn parse_disjunction(&mut self) -> ParseResult<&'a LogicExpr<'a>>;
    /// Parses "P and Q" conjunctions with scope coordination.
    fn parse_conjunction(&mut self) -> ParseResult<&'a LogicExpr<'a>>;
    /// Extracts the subject of a copular predication, for non-parallel coordination.
    fn extract_copular_subject(&self, expr: &'a LogicExpr<'a>) -> Option<Symbol>;
    /// Parses a bare copular-predicate remnant ("wealthy" / "a philanthropist").
    fn try_parse_copular_predicate(&mut self, subject: Symbol) -> ParseResult<Option<&'a LogicExpr<'a>>>;
    /// Parses "who/that/which" relative clauses attaching to noun phrases.
    fn parse_relative_clause(&mut self, gap_var: Symbol) -> ParseResult<&'a LogicExpr<'a>>;
    /// Parses a clause with a gap filled by borrowed verb (for VP coordination).
    fn parse_gapped_clause(&mut self, borrowed_verb: Symbol) -> ParseResult<&'a LogicExpr<'a>>;
    /// Parses "if P were/had" counterfactual antecedent (subjunctive).
    fn parse_counterfactual_antecedent(&mut self) -> ParseResult<&'a LogicExpr<'a>>;
    /// Parses "Q would" counterfactual consequent.
    fn parse_counterfactual_consequent(&mut self) -> ParseResult<&'a LogicExpr<'a>>;
    /// Checks if current token is a wh-word (who, what, which, etc.).
    fn check_wh_word(&self) -> bool;
    /// Returns true if parsing a counterfactual context.
    fn is_counterfactual_context(&self) -> bool;
    /// Returns true if expression is a complete clause.
    fn is_complete_clause(&self, expr: &LogicExpr<'a>) -> bool;
    /// Extracts the main verb from an expression.
    fn extract_verb_from_expr(&self, expr: &LogicExpr<'a>) -> Option<Symbol>;
    /// Attempts to parse an English imperative ("Close the door.", "Don't touch
    /// that.", "Let's leave."). Returns `None` (restoring position) when the input
    /// is not verb-initial / not a hortative or negative command.
    fn try_parse_imperative(&mut self) -> ParseResult<Option<&'a LogicExpr<'a>>>;
    /// True if a finite verb (Verb/Auxiliary/copula/have/modal) appears at or after
    /// `from`, before the clause terminator. Used to distinguish an imperative
    /// (command verb is the only finite verb) from a declarative whose initial word
    /// is a subject ("Set A has cardinality 5.").
    fn clause_has_later_finite_verb(&self, from: usize) -> bool;
    /// True when the verb at `vp` heads a reduced object relative (its overt subject
    /// sits at `vp - 1`, the relativized head is the determiner-headed noun before
    /// it), so it is NOT the clause's main verb.
    fn is_reduced_relative_verb(&self, vp: usize) -> bool;
    /// Attempts to parse an it-cleft "It was X who/that VP." → focus on X plus
    /// exhaustivity (only X did it). Returns `None` (restoring position) otherwise.
    fn try_parse_cleft(&mut self) -> ParseResult<Option<&'a LogicExpr<'a>>>;
    /// Attempts to parse an exclamative "How tall she is!" / "What a fool he is!"
    /// (how/what, no subject-aux inversion, "!"-terminated). Returns `None` otherwise.
    fn try_parse_exclamative(&mut self) -> ParseResult<Option<&'a LogicExpr<'a>>>;
    /// Attempts to parse an optative wish "May you prosper!", "Long live the king!",
    /// "If only it were Friday!". Returns `None` (restoring position) otherwise.
    fn try_parse_optative(&mut self) -> ParseResult<Option<&'a LogicExpr<'a>>>;
    /// Attempts to parse correlative coordination "Neither X nor Y VP" / "Either X
    /// or Y VP" — a shared predicate scoped over two subjects. `None` otherwise.
    fn try_parse_correlative(&mut self) -> ParseResult<Option<&'a LogicExpr<'a>>>;
    /// Attempts to parse "Of NP₁ and NP₂, one VP₁ and the other VP₂" →
    /// (VP₁(NP₁) ∧ VP₂(NP₂)) ∨ (VP₁(NP₂) ∧ VP₂(NP₁)). Returns `None` otherwise.
    fn try_parse_of_pair_xor(&mut self) -> ParseResult<Option<&'a LogicExpr<'a>>>;
    /// Attempts to parse a sentence-initial temporal NP that FRAMES the clause
    /// ("Every year Simon takes a holiday" → HAB), not its subject. `None` otherwise.
    fn try_parse_fronted_temporal_adjunct(&mut self) -> ParseResult<Option<&'a LogicExpr<'a>>>;
    /// Attempts to parse an inverted conditional "Had/Were/Should SUBJECT …, …" by
    /// un-inverting to the canonical "If SUBJECT aux …" form and reusing the conditional
    /// parser. Handles multi-word subjects and `Should`-fronting. `None` otherwise.
    fn try_parse_inverted_conditional(&mut self) -> ParseResult<Option<&'a LogicExpr<'a>>>;
    /// Attempts to parse VP ellipsis ("Mary did too").
    fn try_parse_ellipsis(&mut self) -> Option<ParseResult<&'a LogicExpr<'a>>>;
    /// Checks for ellipsis auxiliary (did, does, can, etc.).
    fn check_ellipsis_auxiliary(&self) -> bool;
    /// Checks for ellipsis terminator (too, also, as well).
    fn check_ellipsis_terminator(&self) -> bool;
}

impl<'a, 'ctx, 'int> ClauseParsing<'a, 'ctx, 'int> for Parser<'a, 'ctx, 'int> {
    fn try_parse_imperative(&mut self) -> ParseResult<Option<&'a LogicExpr<'a>>> {
        let start = self.current;
        let mut negated = false;
        // The covert subject of an imperative is the addressee (the hearer); the
        // hortative "let's" makes it the inclusive group (speaker + addressee).
        let mut agent_name = "Addressee";

        if self.check(&TokenType::Let) {
            // Hortative: "Let's leave." / "Let us leave." (Let + 's|us + verb)
            let n1 = self.current + 1;
            if n1 >= self.tokens.len() {
                return Ok(None);
            }
            let next = &self.tokens[n1];
            let next_text = self.interner.resolve(next.lexeme);
            let is_lets = matches!(next.kind, TokenType::Possessive)
                || next_text.eq_ignore_ascii_case("us")
                || next_text.eq_ignore_ascii_case("'s")
                || next_text.eq_ignore_ascii_case("s");
            if !is_lets {
                return Ok(None);
            }
            self.advance(); // Let
            self.advance(); // 's / us
            agent_name = "Us";
        } else if self.check(&TokenType::Do) {
            // Negative imperative: "Don't touch that." → Do + Not + verb
            let n1 = self.current + 1;
            if n1 < self.tokens.len() && matches!(self.tokens[n1].kind, TokenType::Not) {
                self.advance(); // Do
                self.advance(); // Not
                negated = true;
            } else {
                // "Do you ...?" is a yes/no question, handled elsewhere.
                return Ok(None);
            }
        }

        // Sentence-initial imperatives capitalize the command verb, so the lexer
        // may have tagged it as a ProperName ("Close" in "Close the door."). Retag
        // a capitalized known base verb as a Verb — but ONLY when no other finite
        // verb appears later in the clause. A real imperative has the command verb
        // as its only finite verb; a later finite verb means the initial capitalized
        // word is actually a subject ("Bill ran.", "Set A has cardinality 5.").
        if let TokenType::ProperName(sym) = self.peek().kind {
            let lemma = self.interner.resolve(sym).to_lowercase();
            if crate::lexicon::is_base_verb(&lemma)
                && !self.clause_has_later_finite_verb(self.current + 1)
            {
                let class = crate::lexicon::lookup_verb_class(&lemma);
                self.tokens[self.current].kind = TokenType::Verb {
                    lemma: sym,
                    time: Time::Present,
                    aspect: crate::lexicon::Aspect::Simple,
                    class,
                };
            }
        }

        // An imperative is verb-initial. English has no bare-verb declaratives, and
        // yes/no questions begin with an auxiliary (Do/Is/...) rather than a Verb
        // token, so a Verb in initial position is the command verb.
        if !self.check_verb() {
            self.current = start;
            return Ok(None);
        }

        // Even when the initial word is a Verb token, a later finite verb means it is
        // really a subject (e.g. "Set" in "Set A has cardinality 5."), not a command.
        if self.clause_has_later_finite_verb(self.current + 1) {
            self.current = start;
            return Ok(None);
        }

        let agent = self.interner.intern(agent_name);
        let core = self.parse_predicate_with_subject(agent)?;
        let action = if negated {
            self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: core,
            })
        } else {
            core
        };
        Ok(Some(self.ctx.exprs.alloc(LogicExpr::Imperative { action })))
    }

    fn clause_has_later_finite_verb(&self, from: usize) -> bool {
        let mut j = from;
        while j < self.tokens.len() {
            // A finite verb that heads a REDUCED OBJECT RELATIVE ("the friend Simon
            // WENT with", "the waterfall Derrick PHOTOGRAPHED") is not the clause's
            // main verb — its presence must NOT veto the imperative reading of a
            // sentence-initial command verb. The relative's signature is a
            // determiner-headed noun head followed by a fresh subject (ProperName /
            // Pronoun) and then this verb. Skip past such a verb (and a trailing
            // stranded preposition) and keep scanning for a genuine main verb.
            if matches!(self.tokens[j].kind, TokenType::Verb { .. })
                && self.is_reduced_relative_verb(j)
            {
                let after = j + 1;
                if matches!(
                    self.tokens.get(after).map(|t| &t.kind),
                    Some(TokenType::Preposition(_))
                ) {
                    j = after + 1;
                } else {
                    j = after;
                }
                continue;
            }
            match self.tokens[j].kind {
                TokenType::Period | TokenType::EOF | TokenType::Exclamation => return false,
                TokenType::Verb { .. }
                | TokenType::Auxiliary(_)
                | TokenType::Is
                | TokenType::Are
                | TokenType::Was
                | TokenType::Were
                | TokenType::Do
                | TokenType::Does => return true,
                _ => {
                    // Some finite verbs (have/has/had, modals) are lexed as other
                    // token kinds; catch them by lexeme.
                    let lex = self.interner.resolve(self.tokens[j].lexeme).to_lowercase();
                    if matches!(
                        lex.as_str(),
                        "has" | "have" | "had" | "is" | "are" | "was" | "were"
                            | "do" | "does" | "did" | "will" | "would" | "can"
                            | "could" | "should" | "shall" | "may" | "might" | "must"
                    ) {
                        return true;
                    }
                }
            }
            j += 1;
        }
        false
    }

    /// True when the verb at `vp` heads a reduced object relative — i.e. it is the
    /// finite verb of a relativizer-dropped clause modifying a preceding noun head,
    /// not a main-clause verb. The relative's overt subject (a ProperName or
    /// Pronoun) sits at `vp - 1`, and the relativized head is the determiner-headed
    /// common noun that immediately precedes that subject ("the friend [Simon] went",
    /// "the waterfall [Derrick] photographed"). The determiner requirement is what
    /// distinguishes this from a true main clause whose initial word is a subject
    /// ("Set A has …" — "A" has no determiner-headed noun before it).
    fn is_reduced_relative_verb(&self, vp: usize) -> bool {
        if vp == 0 {
            return false;
        }
        let subj = vp - 1;
        if !matches!(
            self.tokens[subj].kind,
            TokenType::ProperName(_) | TokenType::Pronoun { .. }
        ) {
            return false;
        }
        if subj == 0 {
            return false;
        }
        // The relativized head must be a common noun (the gap filler).
        let head = subj - 1;
        if !matches!(
            self.tokens[head].kind,
            TokenType::Noun(_)
                | TokenType::CalendarUnit(_)
                | TokenType::Ambiguous { .. }
        ) {
            return false;
        }
        // Walk back across nouns/adjectives in the head NP to find a determiner —
        // an article, possessive, or quantifier opening the NP that the relative
        // modifies. Without one, "X Y verb" is a bare main clause, not a relative.
        let mut k = head;
        loop {
            match self.tokens[k].kind {
                TokenType::Article(_)
                | TokenType::Possessive
                | TokenType::All
                | TokenType::Some
                | TokenType::No
                | TokenType::Any
                | TokenType::Most
                | TokenType::Few
                | TokenType::Many => return true,
                TokenType::Noun(_)
                | TokenType::CalendarUnit(_)
                | TokenType::Adjective(_)
                | TokenType::Ambiguous { .. } => {
                    if k == 0 {
                        return false;
                    }
                    k -= 1;
                }
                _ => return false,
            }
        }
    }

    fn try_parse_cleft(&mut self) -> ParseResult<Option<&'a LogicExpr<'a>>> {
        let start = self.current;
        // "It was/is X who/that VP." — the expletive "it" + copula + focus + relative.
        if !self.interner.resolve(self.peek().lexeme).eq_ignore_ascii_case("it") {
            return Ok(None);
        }
        if self.current + 1 >= self.tokens.len()
            || !matches!(self.tokens[self.current + 1].kind, TokenType::Is | TokenType::Was)
        {
            return Ok(None);
        }
        self.advance(); // it
        self.advance(); // is/was

        // The focused constituent (a proper name or NP).
        let focus_np = match self.parse_noun_phrase(false) {
            Ok(np) => np,
            Err(_) => {
                self.current = start;
                return Ok(None);
            }
        };
        if !self.check(&TokenType::Who) && !self.check(&TokenType::That) {
            self.current = start;
            return Ok(None);
        }
        self.advance(); // who/that

        let focus_sym = focus_np.noun;
        // The cleft clause "broke the vase" with the focus as subject — the core
        // predication.
        let core = self.parse_predicate_with_subject(focus_sym)?;

        // Exhaustivity: ∀z( core[focus→z] → z = focus ) — no one but the focus did it.
        let z = self.next_var_name();
        let core_z = self.substitute_constant_with_var_sym(core, focus_sym, z)?;
        let identity = self.ctx.exprs.alloc(LogicExpr::Identity {
            left: self.ctx.terms.alloc(Term::Variable(z)),
            right: self.ctx.terms.alloc(Term::Constant(focus_sym)),
        });
        let implies = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
            left: core_z,
            op: TokenType::Implies,
            right: identity,
        });
        let exhaustivity = self.ctx.exprs.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Universal,
            variable: z,
            body: implies,
            island_id: self.current_island,
        });

        // Focus marker over the core, conjoined with the exhaustivity claim.
        let focused_term = self.ctx.terms.alloc(Term::Constant(focus_sym));
        let focus_expr = self.ctx.exprs.alloc(LogicExpr::Focus {
            kind: crate::token::FocusKind::Cleft,
            focused: focused_term,
            scope: core,
        });
        Ok(Some(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
            left: focus_expr,
            op: TokenType::And,
            right: exhaustivity,
        })))
    }

    fn try_parse_exclamative(&mut self) -> ParseResult<Option<&'a LogicExpr<'a>>> {
        let start = self.current;
        let lead = self.interner.resolve(self.peek().lexeme).to_lowercase();
        if lead != "how" && lead != "what" {
            return Ok(None);
        }
        // An exclamative is "!"-terminated and has NO subject-aux inversion. A
        // wh-question ("How tall is she?") inverts and ends with "?"; require "!".
        let ends_with_exclamation = self.tokens[start..]
            .iter()
            .take_while(|t| !matches!(t.kind, TokenType::EOF))
            .any(|t| matches!(t.kind, TokenType::Exclamation));
        if !ends_with_exclamation {
            return Ok(None);
        }
        let is_what = lead == "what";
        self.advance(); // How / What
        // optional "a"/"an"
        if self.check_article() {
            self.advance();
        }
        // The gradable adjective (How) or the noun (What).
        let pred_sym = match self.consume_content_word() {
            Ok(s) => s,
            Err(_) => {
                self.current = start;
                return Ok(None);
            }
        };
        // The subject (a pronoun or proper name).
        let subj_sym = if let TokenType::ProperName(s) = self.peek().kind {
            self.advance();
            s
        } else if self.check_pronoun() {
            let lx = self.interner.resolve(self.peek().lexeme).to_string();
            self.advance();
            let cap = lx
                .chars()
                .next()
                .map(|c| c.to_uppercase().collect::<String>() + &lx[1..])
                .unwrap_or(lx);
            self.interner.intern(&cap)
        } else {
            match self.consume_content_word() {
                Ok(s) => s,
                Err(_) => {
                    self.current = start;
                    return Ok(None);
                }
            }
        };
        // optional copula + "!"
        if matches!(
            self.peek().kind,
            TokenType::Is | TokenType::Are | TokenType::Was | TokenType::Were
        ) {
            self.advance();
        }
        if self.check(&TokenType::Exclamation) {
            self.advance();
        }

        let degree_var = self.next_var_name();
        // "How tall she is!" → Tall(she, d); "What a fool he is!" → Fool(he).
        let body = if is_what {
            self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: pred_sym,
                args: self.ctx.terms.alloc_slice([Term::Constant(subj_sym)]),
                world: None,
            })
        } else {
            self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: pred_sym,
                args: self
                    .ctx
                    .terms
                    .alloc_slice([Term::Constant(subj_sym), Term::Variable(degree_var)]),
                world: None,
            })
        };
        Ok(Some(self.ctx.exprs.alloc(LogicExpr::Exclamative { degree_var, body })))
    }

    fn try_parse_optative(&mut self) -> ParseResult<Option<&'a LogicExpr<'a>>> {
        let start = self.current;
        // Optatives are "!"-terminated wishes.
        let ends_with_exclamation = self.tokens[start..]
            .iter()
            .take_while(|t| !matches!(t.kind, TokenType::EOF))
            .any(|t| matches!(t.kind, TokenType::Exclamation));
        if !ends_with_exclamation {
            return Ok(None);
        }

        // "May SUBJ VP!" — may-fronting (a wish, not the deontic modal). "May"
        // collides with the month proper-name; in some contexts (e.g. theorem
        // premises) the lexer emits it as a `ProperName("May")`, so accept that
        // spelling too — the `!` terminator and the SUBJ-VP shape below keep a
        // genuine month reading ("May 3 is a holiday.") from matching.
        let is_may_fronting = self.check(&TokenType::May)
            || matches!(self.peek().kind, TokenType::ProperName(_))
                && self.interner.resolve(self.peek().lexeme).eq_ignore_ascii_case("may");
        if is_may_fronting {
            self.advance(); // May
            let subj_sym = if let TokenType::ProperName(s) = self.peek().kind {
                self.advance();
                s
            } else if self.check_pronoun() {
                let lx = self.interner.resolve(self.peek().lexeme).to_lowercase();
                self.advance();
                match lx.as_str() {
                    "you" => self.interner.intern("Addressee"),
                    "i" | "me" => self.interner.intern("Speaker"),
                    other => self.interner.intern(
                        &(other.chars().next().map(|c| c.to_uppercase().collect::<String>() + &other[1..]).unwrap_or_default()),
                    ),
                }
            } else {
                match self.parse_noun_phrase(false) {
                    Ok(np) => np.noun,
                    Err(_) => {
                        self.current = start;
                        return Ok(None);
                    }
                }
            };
            // The wish verb may be a rare/unlisted word ("prosper"), so capture it
            // by lexeme rather than requiring a recognized verb token.
            if self.is_at_end() || self.check(&TokenType::Exclamation) {
                self.current = start;
                return Ok(None);
            }
            let vlex = self.interner.resolve(self.peek().lexeme).to_string();
            let vname = vlex
                .chars()
                .next()
                .map(|c| c.to_uppercase().collect::<String>() + &vlex[1..])
                .unwrap_or(vlex);
            let verb_sym = self.interner.intern(&vname);
            self.advance(); // consume the wish verb
            // Optional object (a pronoun / proper name).
            let wish = if let TokenType::ProperName(o) = self.peek().kind {
                self.advance();
                self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: verb_sym,
                    args: self
                        .ctx
                        .terms
                        .alloc_slice([Term::Constant(subj_sym), Term::Constant(o)]),
                    world: None,
                })
            } else {
                self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: verb_sym,
                    args: self.ctx.terms.alloc_slice([Term::Constant(subj_sym)]),
                    world: None,
                })
            };
            return Ok(Some(self.ctx.exprs.alloc(LogicExpr::Optative { wish })));
        }

        // "Long live NP!" — fixed optative construction.
        let lead = self.interner.resolve(self.peek().lexeme).to_lowercase();
        if lead == "long"
            && self.current + 1 < self.tokens.len()
            && self.interner.resolve(self.tokens[self.current + 1].lexeme).eq_ignore_ascii_case("live")
        {
            self.advance(); // Long
            self.advance(); // live
            let np = self.parse_noun_phrase(false)?;
            let live_sym = self.interner.intern("Live");
            let wish = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: live_sym,
                args: self.ctx.terms.alloc_slice([Term::Constant(np.noun)]),
                world: None,
            });
            return Ok(Some(self.ctx.exprs.alloc(LogicExpr::Optative { wish })));
        }

        // "If only S!" — counterfactual wish.
        if self.check(&TokenType::If)
            && self.current + 1 < self.tokens.len()
            && self.interner.resolve(self.tokens[self.current + 1].lexeme).eq_ignore_ascii_case("only")
        {
            self.advance(); // If
            self.advance(); // only
            let wish = self.parse_sentence()?;
            return Ok(Some(self.ctx.exprs.alloc(LogicExpr::Optative { wish })));
        }

        Ok(None)
    }

    fn try_parse_correlative(&mut self) -> ParseResult<Option<&'a LogicExpr<'a>>> {
        let start = self.current;
        let lead = self.interner.resolve(self.peek().lexeme).to_lowercase();
        let is_neither = lead == "neither";
        let is_either = lead == "either";
        if !is_neither && !is_either {
            return Ok(None);
        }
        self.advance(); // Neither / Either

        // Parse each disjunct as a FULL noun phrase so multi-word proper names
        // ("Belle Grove"), possessives ("Pam's client"), and descriptive NPs with
        // PPs / relative clauses ("the person who paid $150") are preserved with
        // ZERO meaning loss. A bare proper name OR a bare definite (head only,
        // nothing to lose) stays a referring CONSTANT; a description carrying
        // RESTRICTIONS (adjectives / possessor / PPs / relative clause) becomes a
        // fresh existential VARIABLE with a restrictor, so the shared predicate
        // distributes over the right entity.
        fn build_disjunct<'a, 'ctx, 'int>(
            p: &mut Parser<'a, 'ctx, 'int>,
        ) -> ParseResult<OfEntity<'a>> {
            let np = p.parse_noun_phrase(true)?;
            let has_rel = p.check(&TokenType::Who)
                || p.check(&TokenType::That)
                || p.check(&TokenType::Where)
                || p.check(&TokenType::Whose);
            let is_desc = !np.adjectives.is_empty()
                || np.possessor.is_some()
                || !np.pps.is_empty()
                || has_rel;
            let (sym, is_var) = if is_desc {
                (p.next_var_name(), true)
            } else {
                (np.noun, false)
            };
            let term = if is_var {
                Term::Variable(sym)
            } else {
                Term::Constant(sym)
            };
            let rel = p.try_attach_relative(term)?;
            let restrictor = if is_var {
                let mut r = p.nominal_predication(term, &np);
                for pp in np.pps {
                    let pp_sub = p.substitute_pp_placeholder(pp, sym);
                    r = p.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: r,
                        op: TokenType::And,
                        right: pp_sub,
                    });
                }
                if let Some(rc) = rel {
                    r = p.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: r,
                        op: TokenType::And,
                        right: rc,
                    });
                }
                Some(r)
            } else {
                None
            };
            Ok(OfEntity { sym, is_var, term, restrictor })
        }

        // A disjunct's predicate becomes its branch: a description asserts its
        // restrictor and binds the predicate under a fresh existential; a bare
        // constant predicates directly. (For a constant this is exactly the old
        // ¬pred / pred form, so proper-name correlatives are byte-identical.)
        fn wrap_branch<'a, 'ctx, 'int>(
            p: &mut Parser<'a, 'ctx, 'int>,
            e: &OfEntity<'a>,
            body: &'a LogicExpr<'a>,
        ) -> &'a LogicExpr<'a> {
            if !e.is_var {
                return body;
            }
            let inner = match e.restrictor {
                Some(r) => p.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: r,
                    op: TokenType::And,
                    right: body,
                }),
                None => body,
            };
            p.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind: QuantifierKind::Existential,
                variable: e.sym,
                body: inner,
                island_id: p.current_island,
            })
        }

        let e1 = match self.try_parse(|p| build_disjunct(p)) {
            Some(e) => e,
            None => {
                self.current = start;
                return Ok(None);
            }
        };

        let conj = self.interner.resolve(self.peek().lexeme).to_lowercase();
        if conj != "nor" && conj != "or" {
            self.current = start;
            return Ok(None);
        }
        self.advance(); // nor / or

        let e2 = match self.try_parse(|p| build_disjunct(p)) {
            Some(e) => e,
            None => {
                self.current = start;
                return Ok(None);
            }
        };

        // The shared predicate is parsed once per subject by re-parsing from the
        // same position (parallel structure), so "Neither X nor Y VP" distributes VP.
        let vp_start = self.current;
        let pred1 = if e1.is_var {
            self.parse_predicate_with_subject_as_var(e1.sym)?
        } else {
            self.parse_predicate_with_subject(e1.sym)?
        };
        self.current = vp_start;
        let pred2 = if e2.is_var {
            self.parse_predicate_with_subject_as_var(e2.sym)?
        } else {
            self.parse_predicate_with_subject(e2.sym)?
        };

        let result = if is_neither {
            // ¬pred1 ∧ ¬pred2 — a description binds its (asserted) restrictor and
            // the negated predicate under its own existential.
            let n1 = self.ctx.exprs.alloc(LogicExpr::UnaryOp { op: TokenType::Not, operand: pred1 });
            let b1 = wrap_branch(self, &e1, n1);
            let n2 = self.ctx.exprs.alloc(LogicExpr::UnaryOp { op: TokenType::Not, operand: pred2 });
            let b2 = wrap_branch(self, &e2, n2);
            self.ctx.exprs.alloc(LogicExpr::BinaryOp { left: b1, op: TokenType::And, right: b2 })
        } else {
            // "either…or" is the inclusive disjunction by default (so the proof engine
            // and existing tests see a plain ∨); its EXCLUSIVITY implicature
            // `∧ ¬(branch1 ∧ branch2)` is a pragmatic enrichment, added only in that mode.
            let b1 = wrap_branch(self, &e1, pred1);
            let b2 = wrap_branch(self, &e2, pred2);
            let disj = self.ctx.exprs.alloc(LogicExpr::BinaryOp { left: b1, op: TokenType::Or, right: b2 });
            if self.pragmatic {
                let both = self.ctx.exprs.alloc(LogicExpr::BinaryOp { left: b1, op: TokenType::And, right: b2 });
                let not_both = self.ctx.exprs.alloc(LogicExpr::UnaryOp { op: TokenType::Not, operand: both });
                self.ctx.exprs.alloc(LogicExpr::BinaryOp { left: disj, op: TokenType::And, right: not_both })
            } else {
                disj
            }
        };
        Ok(Some(result))
    }

    fn try_parse_inverted_conditional(&mut self) -> ParseResult<Option<&'a LogicExpr<'a>>> {
        // A fronted auxiliary (Had / Were / Should) stands in for "if".
        if !matches!(
            self.peek().kind,
            TokenType::Had | TokenType::Were | TokenType::Should
        ) {
            return Ok(None);
        }

        // Require "antecedent, consequent" — a comma before the clause terminator — so an
        // inverted yes/no question ("Had you eaten?") is not mistaken for a conditional.
        let has_comma = self.tokens[self.current..]
            .iter()
            .take_while(|t| !matches!(t.kind, TokenType::EOF | TokenType::Period))
            .any(|t| matches!(t.kind, TokenType::Comma));
        if !has_comma {
            return Ok(None);
        }

        // Where the fronted aux un-inverts to: before the antecedent's first verb when
        // there is one ("Had the soldiers KNOWN" → "the soldiers had known"), else after
        // the subject-NP head (copular "Were I rich" → "I were rich"). Scanning stops at
        // the clause comma so the aux never lands in the consequent.
        let start = self.current + 1;
        let comma_at = self.tokens[start..]
            .iter()
            .position(|t| matches!(t.kind, TokenType::Comma))
            .map(|p| start + p)
            .unwrap_or(self.tokens.len());

        let first_verb = (start..comma_at)
            .find(|&j| matches!(self.tokens[j].kind, TokenType::Verb { .. }));

        // Subject-NP head: optional determiners/adjectives then a nominal head (or, after
        // a determiner, any content word — "soldiers" is verb/noun-ambiguous here).
        let mut i = start;
        while i < comma_at
            && matches!(
                self.tokens[i].kind,
                TokenType::Article(_)
                    | TokenType::Adjective(_)
                    | TokenType::Cardinal(_)
                    | TokenType::Possessive
            )
        {
            i += 1;
        }
        let head_is_nominal = i < comma_at
            && matches!(
                self.tokens[i].kind,
                TokenType::Noun(_) | TokenType::ProperName(_) | TokenType::Pronoun { .. }
            );
        let head_after_determiner =
            i > start && i < comma_at && Self::is_content_word_type(&self.tokens[i].kind);

        let insert_at = match first_verb {
            Some(v) => v,
            None if head_is_nominal || head_after_determiner => i + 1,
            None => return Ok(None), // no subject NP / no predicate — not a conditional
        };

        // Un-invert to canonical order: lift the fronted aux to just after the subject NP
        // and prepend a synthesized "If", then reuse the conditional parser. Reusing it
        // keeps one antecedent grammar (weather verbs, conjunction, counterfactual
        // detection) rather than duplicating it for the inverted word order.
        let aux = self.tokens.remove(self.current);
        self.tokens.insert(insert_at - 1, aux);
        let mut if_tok = self.tokens[self.current].clone();
        if_tok.kind = TokenType::If;
        if_tok.lexeme = self.interner.intern("if");
        self.tokens.insert(self.current, if_tok);
        self.advance(); // consume the synthesized "If"
        Ok(Some(self.parse_conditional()?))
    }

    /// A sentence-initial temporal NP that FRAMES the clause rather than serving as
    /// its subject: "Every year Simon takes a holiday" → HAB over the whole clause.
    /// Fires only for "Every/All <calendar-unit>" FOLLOWED BY a clause subject; a
    /// time NP that is itself the subject ("Every year is long") has a copula/verb
    /// next and is left to the ordinary quantifier path.
    fn try_parse_fronted_temporal_adjunct(&mut self) -> ParseResult<Option<&'a LogicExpr<'a>>> {
        let is_universal_det = matches!(self.peek().kind, TokenType::All);
        let unit_next = matches!(
            self.tokens.get(self.current + 1).map(|t| &t.kind),
            Some(TokenType::CalendarUnit(_))
        );
        let subject_after = self
            .tokens
            .get(self.current + 2)
            .map_or(false, |t| starts_clause_subject(&t.kind));
        if !(is_universal_det && unit_next && subject_after) {
            return Ok(None);
        }
        self.advance(); // Every / All
        self.advance(); // <calendar-unit>
        if self.check(&TokenType::Comma) {
            self.advance();
        }
        let clause = self.parse_sentence()?;
        // A present-tense clause ("Simon takes …") already carries HAB; the fronted
        // "every <unit>" then adds nothing — don't double-wrap.
        if matches!(clause, LogicExpr::Aspectual { operator: AspectOperator::Habitual, .. }) {
            return Ok(Some(clause));
        }
        Ok(Some(self.ctx.exprs.alloc(LogicExpr::Aspectual {
            operator: AspectOperator::Habitual,
            body: clause,
        })))
    }

    fn parse_sentence(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        // In imperative mode, handle Let statements by converting to LogicExpr
        // This supports declarative parser being called after process_block_headers()
        // Let x is/= value -> returns the value expression (the test just checks parsing succeeds)
        if self.mode == ParserMode::Imperative && self.check(&TokenType::Let) {
            self.advance(); // consume "Let"
            let _var = self.expect_identifier()?;
            // Accept "is", "be", "=" as assignment operators
            if self.check(&TokenType::Is) || self.check(&TokenType::Be) || self.check(&TokenType::Equals) || self.check(&TokenType::Identity) || self.check(&TokenType::Assign) {
                self.advance(); // consume the operator
            }
            // Parse the value and return it (test just checks parsing succeeds)
            return self.parse_disjunction();
        }

        // Check for ellipsis pattern: "Mary does too." / "Mary can too."
        if let Some(result) = self.try_parse_ellipsis() {
            return result;
        }

        // Optatives: "May you prosper!", "Long live the king!", "If only …!".
        if self.mode != ParserMode::Imperative {
            if let Some(opt) = self.try_parse_optative()? {
                return Ok(opt);
            }
        }

        // Correlative coordination: "Neither X nor Y VP" / "Either X or Y VP".
        if self.mode != ParserMode::Imperative {
            if let Some(corr) = self.try_parse_correlative()? {
                return Ok(corr);
            }
        }

        // "Of NP₁ and NP₂, one VP₁ and the other VP₂" binary XOR partition.
        if self.mode != ParserMode::Imperative {
            if let Some(xor) = self.try_parse_of_pair_xor()? {
                return Ok(xor);
            }
        }

        // Sentence-initial temporal adjunct: "Every year Simon takes a holiday"
        // (habitual) — the fronted time NP frames the whole clause.
        if self.mode != ParserMode::Imperative {
            if let Some(framed) = self.try_parse_fronted_temporal_adjunct()? {
                return Ok(framed);
            }
        }

        // "Whoever VP₁ VP₂" → ∀x(VP₁(x) → VP₂(x)).
        // "Whoever" is not in the lexicon so it arrives as Noun/ProperName; detect by text.
        if self.mode != ParserMode::Imperative {
            let lead_text = self.interner.resolve(self.peek().lexeme).to_lowercase();
            if lead_text == "whoever" {
                self.advance(); // consume "whoever"
                let var = self.next_var_name();
                let restrictor = self.parse_predicate_with_subject(var)?;
                let scope = self.parse_predicate_with_subject(var)?;
                let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: restrictor,
                    op: TokenType::Implies,
                    right: scope,
                });
                return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: QuantifierKind::Universal,
                    variable: var,
                    body,
                    island_id: self.current_island,
                }));
            }
        }

        // Exclamatives: "How tall she is!" / "What a fool he is!" — before the
        // wh-question path, since they share how/what but are "!"-terminated.
        if self.mode != ParserMode::Imperative {
            if let Some(excl) = self.try_parse_exclamative()? {
                return Ok(excl);
            }
        }

        // it-clefts: "It was John who broke the vase." → focus + exhaustivity.
        if self.mode != ParserMode::Imperative {
            if let Some(cleft) = self.try_parse_cleft()? {
                return Ok(cleft);
            }
        }

        // English imperatives: bare-verb-initial commands ("Close the door."),
        // negatives ("Don't touch that."), and hortatives ("Let's leave."). Only in
        // declarative (English) mode — code mode has its own verb-initial handling.
        if self.mode != ParserMode::Imperative {
            if let Some(imp) = self.try_parse_imperative()? {
                return Ok(imp);
            }
        }

        // "Although/Though X, Y" concessive subordinator: Y holds despite X (a
        // defeated expectation). → Concessive{ main: Y, concession: X }.
        if self.check(&TokenType::Although) {
            self.advance(); // consume "Although"/"Though"
            let concession = self.parse_sentence()?;
            if self.check(&TokenType::Comma) {
                self.advance();
            }
            let main = self.parse_sentence()?;
            return Ok(self.ctx.exprs.alloc(LogicExpr::Concessive { main, concession }));
        }

        // "While X, Y" as temporal duration subordinator
        // Duration semantics: Y holds for the entire interval where X is true.
        // Lowered as implication checked globally: G(X → Y)
        if self.check(&TokenType::While) {
            self.advance(); // consume "While"
            let condition = self.parse_sentence()?;
            if self.check(&TokenType::Comma) {
                self.advance();
            }
            let consequent = self.parse_sentence()?;
            return Ok(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: condition,
                op: TokenType::Implies,
                right: consequent,
            }));
        }

        // "When X, Y" as temporal subordinator (before wh-question check)
        // Disambiguate: subordinator has comma-separated clauses, question does not
        if self.check(&TokenType::When) {
            let saved = self.current;
            let mut found_comma = false;
            for i in (self.current + 1)..self.tokens.len() {
                match &self.tokens[i].kind {
                    TokenType::Comma => { found_comma = true; break; }
                    TokenType::Period => break,
                    _ => {}
                }
            }
            if found_comma {
                self.advance(); // consume "When"
                let condition = self.parse_sentence()?;
                if self.check(&TokenType::Comma) {
                    self.advance();
                }
                let consequent = self.parse_sentence()?;
                return Ok(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: condition,
                    op: TokenType::Implies,
                    right: consequent,
                }));
            }
            self.current = saved;
        }

        // "Whenever X, Y" → same as "When X, Y"
        if self.check_content_word() {
            let word = self.interner.resolve(self.peek().lexeme).to_string();
            if word == "Whenever" || word == "whenever" {
                self.advance(); // consume "Whenever"
                let condition = self.parse_sentence()?;
                if self.check(&TokenType::Comma) {
                    self.advance();
                }
                let consequent = self.parse_sentence()?;
                return Ok(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: condition,
                    op: TokenType::Implies,
                    right: consequent,
                }));
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

        // Inverted conditional (§4.1): "Had I known, …" / "Were I rich, …" /
        // "Should it rain, …" — subject-aux inversion stands in for "if".
        if let Some(expr) = self.try_parse_inverted_conditional()? {
            return Ok(expr);
        }

        if self.match_token(&[TokenType::If]) {
            return self.parse_conditional();
        }

        // Handle "Either X or Y" disjunction
        // Special case: "Either NP1 or NP2 is/are PRED" should apply PRED to both
        if self.match_token(&[TokenType::Either]) {
            return self.parse_either_or();
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

        // "not both every request is valid and every grant is valid" → ¬(X ∧ Y)
        // Only triggers for clausal conjunction: "both" + quantifier/determiner
        // NOT for conjoined NP: "both Socrates and Plato are men"
        if self.check(&TokenType::Both) {
            // Peek at token after "both" — if it's a quantifier, this is clausal
            let next_is_clausal = if self.current + 1 < self.tokens.len() {
                matches!(self.tokens[self.current + 1].kind,
                    TokenType::All | TokenType::No | TokenType::Some | TokenType::Any
                    | TokenType::Most | TokenType::Few | TokenType::Many
                    | TokenType::Cardinal(_) | TokenType::AtLeast(_) | TokenType::AtMost(_)
                    | TokenType::Article(_)
                )
            } else {
                false
            };
            if next_is_clausal {
                self.advance(); // consume "both"
                let first = self.parse_atom()?;
                if self.check(&TokenType::And) {
                    self.advance(); // consume "and"
                }
                let second = self.parse_atom()?;
                return Ok(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: first,
                    op: TokenType::And,
                    right: second,
                }));
            }
        }

        // Sentence-initial temporal operators for hardware verification:
        // "Always, P" → Temporal { Always, P }
        // "Eventually, P" → Temporal { Eventually, P }
        // "Next, P" → Temporal { Next, P }
        // "Never P" → Temporal { Always, ¬P }
        {
            let temporal_op = match &self.peek().kind {
                TokenType::Adverb(sym) | TokenType::ScopalAdverb(sym) | TokenType::TemporalAdverb(sym) => {
                    let resolved = self.interner.resolve(*sym).to_string();
                    match resolved.as_str() {
                        "Always" => Some(crate::ast::logic::TemporalOperator::Always),
                        "Eventually" => Some(crate::ast::logic::TemporalOperator::Eventually),
                        "Next" => Some(crate::ast::logic::TemporalOperator::Next),
                        _ => None,
                    }
                }
                // Handle "next" as an adjective token (common fallback)
                TokenType::Adjective(sym) => {
                    let resolved = self.interner.resolve(*sym).to_string();
                    if resolved == "Next" {
                        Some(crate::ast::logic::TemporalOperator::Next)
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(op) = temporal_op {
                self.advance(); // consume the token
                // Optionally consume comma: "Always, P"
                if self.check(&TokenType::Comma) {
                    self.advance();
                }
                let body = self.parse_sentence()?;
                return Ok(self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: op,
                    body,
                }));
            }
        }
        // "Never P" → G(¬P): Always { Not { P } }
        if self.check(&TokenType::Never) {
            self.advance(); // consume "Never"
            // Optionally consume comma
            if self.check(&TokenType::Comma) {
                self.advance();
            }
            let body = self.parse_sentence()?;
            let negated = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: body,
            });
            return Ok(self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: crate::ast::logic::TemporalOperator::Always,
                body: negated,
            }));
        }

        // "After X, Y" → X → Y (temporal sequence)
        // "Before X, Y" → Y → X
        // Handles both "After reset is deasserted, ..." (full clause)
        // and "After request, ..." (bare signal/event noun)
        if self.check_preposition_is("after") || self.check_preposition_is("After") {
            self.advance(); // consume "after"

            // Check for bare noun/signal + comma pattern: "After request, ..."
            // Also handle Performative tokens (e.g., "request" when not after determiner)
            let is_bare_noun_comma = self.current + 1 < self.tokens.len()
                && matches!(self.tokens[self.current + 1].kind, TokenType::Comma)
                && (self.check_content_word()
                    || matches!(self.peek().kind, TokenType::Performative(_)));
            let antecedent = if is_bare_noun_comma {
                let noun = match self.advance().kind.clone() {
                    TokenType::Performative(s) => s,
                    TokenType::Noun(s) | TokenType::Adjective(s) | TokenType::ProperName(s) => s,
                    TokenType::Verb { lemma, .. } => lemma,
                    _ => return Err(crate::error::ParseError {
                        kind: crate::error::ParseErrorKind::ExpectedContentWord { found: self.peek().kind.clone() },
                        span: self.current_span(),
                    }),
                };
                self.ctx.exprs.alloc(LogicExpr::Atom(noun))
            } else {
                self.parse_sentence()?
            };

            if self.check(&TokenType::Comma) {
                self.advance();
            }
            let consequent = self.parse_sentence()?;
            let consequent = self.try_wrap_bounded_delay(consequent);
            return Ok(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: antecedent,
                op: TokenType::Implies,
                right: consequent,
            }));
        }
        if self.check_preposition_is("before") || self.check_preposition_is("Before") {
            self.advance(); // consume "before"
            let first_clause = self.parse_sentence()?;
            if self.check(&TokenType::Comma) {
                self.advance();
            }
            let second_clause = self.parse_sentence()?;
            return Ok(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: second_clause,
                op: TokenType::Implies,
                right: first_clause,
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

        // Biscuit / relevance conditional (§4.2): "If you WANT tea, the kettle is
        // hot." — an "if you <relevance-verb> …" antecedent restricts RELEVANCE, not
        // truth; the consequent is asserted unconditionally.
        let is_biscuit = self.check_pronoun()
            && self.interner.resolve(self.peek().lexeme).eq_ignore_ascii_case("you")
            && self
                .tokens
                .get(self.current + 1)
                .map(|t| {
                    crate::lexicon::is_relevance_verb(
                        &self.interner.resolve(t.lexeme).to_lowercase(),
                    )
                })
                .unwrap_or(false);

        // Enter DRS antecedent box - indefinites here get universal force
        self.drs.enter_box(BoxType::ConditionalAntecedent);
        let mut antecedent = self.parse_counterfactual_antecedent()?;

        // Handle conjunction of clauses in antecedent: "If X is Y and Z is W, ..."
        while self.check(&TokenType::And) {
            self.advance(); // consume "and"
            let second = self.parse_counterfactual_antecedent()?;
            antecedent = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: antecedent,
                op: TokenType::And,
                right: second,
            });
        }
        self.drs.exit_box();

        if self.check(&TokenType::Comma) {
            self.advance();
        }

        if self.check(&TokenType::Then) {
            self.advance();
        }

        // Enter DRS consequent box - can access antecedent referents
        self.drs.enter_box(BoxType::ConditionalConsequent);
        let mut consequent = self.parse_counterfactual_consequent()?;

        // Conjunction of consequent clauses: "…, he would have passed and he
        // would have celebrated." A non-clausal "and" (NP coordination left
        // unconsumed) rolls back and is left for the caller.
        while self.check(&TokenType::And) {
            let cp = self.checkpoint();
            self.advance(); // consume "and"
            match self.parse_counterfactual_consequent() {
                Ok(second) => {
                    consequent = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: consequent,
                        op: TokenType::And,
                        right: second,
                    });
                }
                Err(_) => {
                    self.restore(cp);
                    break;
                }
            }
        }

        // Trailing temporal operators on the consequent — the conditional's
        // consequent path does not route through parse_disjunction, so the
        // same handlers apply here: "…, P until Q." / "…, P in the next
        // cycle." / "…, P within N cycles."
        if self.check(&TokenType::Until)
            || self.check(&TokenType::Release)
            || self.check(&TokenType::WeakUntil)
        {
            let op = match self.peek().kind {
                TokenType::Release => crate::ast::logic::BinaryTemporalOp::Release,
                TokenType::WeakUntil => crate::ast::logic::BinaryTemporalOp::WeakUntil,
                _ => crate::ast::logic::BinaryTemporalOp::Until,
            };
            self.advance();
            let right = self.parse_counterfactual_consequent()?;
            consequent = self.ctx.exprs.alloc(LogicExpr::TemporalBinary {
                operator: op,
                left: consequent,
                right,
            });
        }
        consequent = self.try_wrap_next_cycle(consequent);
        consequent = self.try_wrap_bounded_delay(consequent);
        self.drs.exit_box();

        // Biscuit conditional: assert the consequent and mark the antecedent as a
        // relevance condition — `consequent ∧ Relevance(⟨antecedent⟩)`.
        if is_biscuit {
            let relevance = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: self.interner.intern("Relevance"),
                args: self.ctx.terms.alloc_slice([Term::Proposition(antecedent)]),
                world: None,
            });
            return Ok(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: consequent,
                op: TokenType::And,
                right: relevance,
            }));
        }

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

    /// Parse "Either NP1 or NP2 is/are PRED" or "Either S1 or S2"
    ///
    /// Handles coordination: "Either Alice or Bob is guilty" should become
    /// guilty(Alice) ∨ guilty(Bob), not Alice ∨ guilty(Bob)
    fn parse_either_or(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
        // Save position for potential backtracking
        let start_pos = self.current;

        // Try to parse as "Either NP1 or NP2 VP"
        // First, try to parse just a proper name (not a full clause)
        if let TokenType::ProperName(name1) = self.peek().kind {
            self.advance(); // consume first proper name

            if self.check(&TokenType::Or) {
                self.advance(); // consume "or"

                if let TokenType::ProperName(name2) = self.peek().kind {
                    self.advance(); // consume second proper name

                    // Check for shared predicate: "is/are ADJECTIVE"
                    let is_copula = matches!(
                        self.peek().kind,
                        TokenType::Is | TokenType::Are | TokenType::Was | TokenType::Were
                    );
                    if is_copula {
                        self.advance(); // consume copula

                        // Check for negation: "is not"
                        let is_negated = self.match_token(&[TokenType::Not]);

                        // Try to get an adjective
                        if let TokenType::Adjective(adj) = self.peek().kind {
                            self.advance(); // consume adjective

                            // Create predicate for each NP
                            let pred1 = self.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: adj,
                                args: self.ctx.terms.alloc_slice(vec![
                                    Term::Constant(name1)
                                ]),
                                world: None,
                            });
                            let pred2 = self.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: adj,
                                args: self.ctx.terms.alloc_slice(vec![
                                    Term::Constant(name2)
                                ]),
                                world: None,
                            });

                            // Apply negation if needed
                            let left = if is_negated {
                                self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                                    op: TokenType::Not,
                                    operand: pred1,
                                })
                            } else {
                                pred1
                            };
                            let right = if is_negated {
                                self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                                    op: TokenType::Not,
                                    operand: pred2,
                                })
                            } else {
                                pred2
                            };

                            return Ok(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                left,
                                op: TokenType::Or,
                                right,
                            }));
                        }
                    }
                }
            }

            // Backtrack if the special case didn't match
            self.current = start_pos;
        }

        // Fall back to general disjunction parsing
        // Enter disjunct box for left side - referents here are inaccessible outward
        self.drs.enter_box(BoxType::Disjunct);
        let left = self.parse_conjunction()?;
        self.drs.exit_box();

        if !self.check(&TokenType::Or) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword { keyword: "or".to_string() },
                span: self.current_span(),
            });
        }
        self.advance(); // consume "or"

        // Enter disjunct box for right side - referents here are also inaccessible outward
        self.drs.enter_box(BoxType::Disjunct);
        let right = self.parse_conjunction()?;
        self.drs.exit_box();

        Ok(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
            left,
            op: TokenType::Or,
            right,
        }))
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
                    // Look ahead for weather verb: "it rains" or "it is raining"
                    if self.current + 1 < self.tokens.len() {
                        // Check for "it + verb" pattern
                        if let TokenType::Verb { lemma, time, .. } = &self.tokens[self.current + 1].kind {
                            let lemma_str = self.interner.resolve(*lemma);
                            if Lexer::is_weather_verb(lemma_str) {
                                let verb = *lemma;
                                let verb_time = *time;
                                self.advance(); // consume "it"
                                self.advance(); // consume weather verb

                                let event_var = self.get_event_var();

                                // Weather verbs are impersonal - no pronoun resolution needed
                                // Event var gets universal force from transpiler when suppress_existential=true
                                let suppress_existential = self.drs.in_conditional_antecedent();

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
                        // Check for "it + is/are + verb" pattern: "it is raining"
                        else if self.current + 2 < self.tokens.len() {
                            let is_copula = matches!(
                                self.tokens[self.current + 1].kind,
                                TokenType::Is | TokenType::Are | TokenType::Was | TokenType::Were
                            );
                            if is_copula {
                                if let TokenType::Verb { lemma, .. } = &self.tokens[self.current + 2].kind {
                                    let lemma_str = self.interner.resolve(*lemma);
                                    if Lexer::is_weather_verb(lemma_str) {
                                        let verb = *lemma;
                                        let verb_time = if matches!(
                                            self.tokens[self.current + 1].kind,
                                            TokenType::Was | TokenType::Were
                                        ) {
                                            Time::Past
                                        } else {
                                            Time::Present
                                        };
                                        self.advance(); // consume "it"
                                        self.advance(); // consume "is/are/was/were"
                                        self.advance(); // consume weather verb

                                        let event_var = self.get_event_var();
                                        // Weather verbs are impersonal - no pronoun resolution needed
                                        // Event var gets universal force from transpiler when suppress_existential=true
                                        let suppress_existential = self.drs.in_conditional_antecedent();

                                        let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                                            event_var,
                                            verb,
                                            roles: self.ctx.roles.alloc_slice(vec![]),
                                            modifiers: self.ctx.syms.alloc_slice(vec![]),
                                            suppress_existential,
                                            world: None,
                                        })));

                                        // Progressive aspect for "is raining"
                                        let with_aspect = self.ctx.exprs.alloc(LogicExpr::Aspectual {
                                            operator: AspectOperator::Progressive,
                                            body: neo_event,
                                        });

                                        return Ok(match verb_time {
                                            Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                                                operator: TemporalOperator::Past,
                                                body: with_aspect,
                                            }),
                                            _ => with_aspect,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Track if subject is an indefinite that needs DRS registration
            let (subject, subject_type_pred) = if self.check_pronoun() {
                let token = self.advance().clone();
                let token_text = self.interner.resolve(token.lexeme);
                // Handle first/second person pronouns as constants (deictic reference)
                let resolved = if token_text.eq_ignore_ascii_case("i") {
                    self.interner.intern("Speaker")
                } else if token_text.eq_ignore_ascii_case("you") {
                    self.interner.intern("Addressee")
                } else if let TokenType::Pronoun { gender, number, .. } = token.kind {
                    let resolved_pronoun = self.resolve_pronoun(gender, number)?;
                    match resolved_pronoun {
                        super::ResolvedPronoun::Variable(s) | super::ResolvedPronoun::Constant(s) => s,
                    }
                } else {
                    unknown
                };
                (resolved, None)
            } else {
                let np = self.parse_noun_phrase(true)?;

                // Check if this NP should introduce a DRS referent
                // Both indefinites ("a dog") and definites ("the dog") introduce referents
                // For definites without antecedent, this implements "global accommodation"
                if np.definiteness == Some(Definiteness::Indefinite)
                    || np.definiteness == Some(Definiteness::Definite)
                    || np.definiteness == Some(Definiteness::Distal) {
                    let gender = Self::infer_noun_gender(self.interner.resolve(np.noun));
                    let number = if Self::is_plural_noun(self.interner.resolve(np.noun)) {
                        Number::Plural
                    } else {
                        Number::Singular
                    };

                    // Register in DRS using noun as variable (for pronoun resolution)
                    // For DEFINITES ("the X"), use MainClause source to avoid universal force
                    // This ensures "the butler" in conditionals is treated as a constant
                    // For INDEFINITES ("a X"), use default source (gets universal force in antecedent)
                    if np.definiteness == Some(Definiteness::Definite) || np.definiteness == Some(Definiteness::Distal) {
                        self.drs.introduce_referent_with_source(np.noun, np.noun, gender, number, crate::drs::ReferentSource::MainClause);
                    } else {
                        self.drs.introduce_referent(np.noun, np.noun, gender, number);
                    }

                    // Create type predicate: Farmer(noun)
                    let type_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: np.noun,
                        args: self.ctx.terms.alloc_slice([Term::Variable(np.noun)]),
                        world: None,
                    });

                    (np.noun, Some(type_pred))
                } else {
                    // Proper name - use as constant (proper names have their own registration)
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
                return self.parse_presupposition(&np, presup_kind, false);
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
                            let resolved_pronoun = self.resolve_pronoun(gender, number)?;
                            match resolved_pronoun {
                                super::ResolvedPronoun::Variable(s) | super::ResolvedPronoun::Constant(s) => s,
                            }
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
                // "If John had NOT studied, …" — negated antecedent.
                let negated = self.check(&TokenType::Not);
                if negated {
                    self.advance();
                }
                let verb = self.consume_content_word()?;
                let mut main_pred: &'a LogicExpr<'a> =
                    self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: verb,
                        args: self.ctx.terms.alloc_slice([subject_term]),
                        world: None,
                    });
                if negated {
                    main_pred = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand: main_pred,
                    });
                }

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
                let token_text = self.interner.resolve(token.lexeme);
                // Handle first/second person pronouns as constants (deictic reference)
                if token_text.eq_ignore_ascii_case("i") {
                    self.interner.intern("Speaker")
                } else if token_text.eq_ignore_ascii_case("you") {
                    self.interner.intern("Addressee")
                } else if let TokenType::Pronoun { gender, number, .. } = token.kind {
                    let resolved_pronoun = self.resolve_pronoun(gender, number)?;
                    match resolved_pronoun {
                        super::ResolvedPronoun::Variable(s) | super::ResolvedPronoun::Constant(s) => s,
                    }
                } else {
                    unknown
                }
            } else {
                let np = self.parse_noun_phrase(true)?;
                if np.definiteness == Some(crate::lexicon::Definiteness::Definite) {
                    // A definite presupposes existence: accommodate the
                    // referent GLOBALLY (highest box) so later mentions BIND
                    // to it ("…, the kettle is hot." then "The kettle is
                    // hot." reuses the same individual).
                    self.drs.introduce_referent_global(
                        np.noun,
                        np.noun,
                        Gender::Unknown,
                        Number::Singular,
                        crate::drs::ReferentSource::MainClause,
                    );
                }
                np.noun
            };

            if self.check(&TokenType::Would) {
                self.advance();
                // "…, he would NOT have failed." — negated consequent.
                let negated = self.check(&TokenType::Not);
                if negated {
                    self.advance();
                }
                if self.check_content_word() {
                    let next_word = self.interner.resolve(self.peek().lexeme).to_lowercase();
                    if next_word == "have" {
                        self.advance();
                    }
                }
                // A bare verb keeps the simple predication shape; anything
                // after it ("would buy a boat") takes the full VP grammar.
                let clause_ends_after_verb = matches!(
                    self.tokens.get(self.current + 1).map(|t| t.kind.clone()),
                    Some(
                        TokenType::Period
                            | TokenType::Exclamation
                            | TokenType::EOF
                            | TokenType::And
                            | TokenType::Comma
                    ) | None
                );
                let mut pred: &'a LogicExpr<'a> = if clause_ends_after_verb {
                    let verb = self.consume_content_word()?;
                    self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: verb,
                        args: self.ctx.terms.alloc_slice([Term::Constant(subject)]),
                        world: None,
                    })
                } else {
                    self.parse_predicate_with_subject(subject)?
                };
                if negated {
                    pred = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand: pred,
                    });
                }
                return Ok(pred);
            }

            return self.parse_predicate_with_subject(subject);
        }

        self.parse_sentence()
    }

    fn extract_verb_from_expr(&self, expr: &LogicExpr<'a>) -> Option<Symbol> {
        match expr {
            // NeoEvent directly contains the verb
            LogicExpr::NeoEvent(data) => Some(data.verb),
            // Control structures directly contain the verb
            LogicExpr::Control { verb, .. } => Some(*verb),
            // Phase 46: For BinaryOp, try to find NeoEvent first (either side),
            // then fall back to Predicate. This handles both:
            // - Transitive: Apple(x) ∧ ∃e(Eat(e)...) - NeoEvent on right
            // - Motion PP: ∃e(Walk(e)...) ∧ To(e, Park) - NeoEvent on left
            LogicExpr::BinaryOp { left, right, .. } => {
                // First check if left contains a NeoEvent (motion PP case)
                if let Some(verb) = self.extract_neo_event_verb(left) {
                    return Some(verb);
                }
                // Then check right (transitive case with type predicate on left)
                if let Some(verb) = self.extract_neo_event_verb(right) {
                    return Some(verb);
                }
                // Fall back to any extractable verb
                self.extract_verb_from_expr(left)
                    .or_else(|| self.extract_verb_from_expr(right))
            }
            // Plain predicate - last resort (might be type predicate or PP)
            LogicExpr::Predicate { name, .. } => Some(*name),
            LogicExpr::Modal { operand, .. } => self.extract_verb_from_expr(operand),
            LogicExpr::Presupposition { assertion, .. } => self.extract_verb_from_expr(assertion),
            LogicExpr::Temporal { body, .. } => self.extract_verb_from_expr(body),
            LogicExpr::TemporalAnchor { body, .. } => self.extract_verb_from_expr(body),
            LogicExpr::Aspectual { body, .. } => self.extract_verb_from_expr(body),
            LogicExpr::Quantifier { body, .. } => self.extract_verb_from_expr(body),
            _ => None,
        }
    }

    /// Phase 46: Generalized gapping with template-guided reconstruction.
    /// Handles NPs, PPs, temporal adverbs, and preserves roles from EventTemplate.
    fn parse_gapped_clause(&mut self, borrowed_verb: Symbol) -> ParseResult<&'a LogicExpr<'a>> {
        let subject = self.parse_noun_phrase(true)?;

        if self.check(&TokenType::Comma) {
            self.advance();
        }

        let subject_term = self.noun_phrase_to_term(&subject);
        let event_var = self.get_event_var();
        let suppress_existential = self.drs.in_conditional_antecedent();

        // Get template for role guidance
        let template = self.last_event_template.clone();

        // Collect arguments (NPs, PPs, temporal adverbs) from gapped clause
        let mut np_args: Vec<Term<'a>> = Vec::new();
        let mut pp_args: Vec<(Symbol, Term<'a>)> = Vec::new();
        let mut override_adverb: Option<Symbol> = None;

        loop {
            if self.check_temporal_adverb() {
                // Temporal adverb: override template modifier
                if let TokenType::TemporalAdverb(sym) = self.advance().kind {
                    override_adverb = Some(sym);
                }
            } else if self.check_preposition() {
                // PP argument: "to the school", "on the table"
                let prep = if let TokenType::Preposition(sym) = self.advance().kind {
                    sym
                } else {
                    continue;
                };
                let np = self.parse_noun_phrase(false)?;
                pp_args.push((prep, self.noun_phrase_to_term(&np)));
            } else if self.check_content_word() || self.check_article() {
                // NP argument
                let np = self.parse_noun_phrase(false)?;
                np_args.push(self.noun_phrase_to_term(&np));
                if self.check(&TokenType::Comma) {
                    self.advance();
                }
            } else {
                break;
            }
        }

        // Build roles using template guidance
        let roles = self.build_gapped_roles(subject_term, &np_args, &pp_args, &template);

        // Handle modifiers: override if adverb provided, else inherit from template
        let modifiers = match (override_adverb, &template) {
            (Some(adv), Some(tmpl)) => {
                // Filter out temporal modifiers from template, add new one
                let mut mods: Vec<Symbol> = tmpl
                    .modifiers
                    .iter()
                    .filter(|m| !self.is_temporal_modifier(**m))
                    .cloned()
                    .collect();
                mods.push(adv);
                mods
            }
            (Some(adv), None) => vec![adv],
            (None, Some(tmpl)) => tmpl.modifiers.clone(),
            (None, None) => vec![],
        };

        Ok(self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
            event_var,
            verb: borrowed_verb,
            roles: self.ctx.roles.alloc_slice(roles),
            modifiers: self.ctx.syms.alloc_slice(modifiers),
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
        {
            if self.check(&TokenType::Comma) {
                self.advance();
            }
            // Iff is handled at a LOOSER precedence tier below (standard
            // precedence ∨ > ↔); only Or folds here.
            if !self.match_token(&[TokenType::Or]) {
                break;
            }
            let operator = self.previous().kind.clone();
            self.current_island += 1;

            let saved_pos = self.current;
            let standard_attempt = self.try_parse(|p| p.parse_conjunction());

            // Gapping in disjunction: only for Or, not Iff. Use original (non-expanded) trigger.
            // Expanded gapping (with Period/is_at_end) only applies in parse_conjunction.
            let use_gapping = match &standard_attempt {
                Some(right) => {
                    !self.is_complete_clause(right)
                        && (self.check(&TokenType::Comma) || self.check_content_word())
                        && operator != TokenType::Iff // Don't gap on biconditional
                }
                None => operator != TokenType::Iff, // For Iff, require successful parse
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

        // Handle binary temporal connectives (lowest precedence temporal)
        // "P until Q" → TemporalBinary { Until, P, Q }
        // "P release Q" → TemporalBinary { Release, P, Q }
        // "P weak-until Q" → TemporalBinary { WeakUntil, P, Q }
        if self.check(&TokenType::Until) || self.check(&TokenType::Release) || self.check(&TokenType::WeakUntil) {
            let op = match self.peek().kind {
                TokenType::Release => crate::ast::logic::BinaryTemporalOp::Release,
                TokenType::WeakUntil => crate::ast::logic::BinaryTemporalOp::WeakUntil,
                _ => crate::ast::logic::BinaryTemporalOp::Until,
            };
            self.advance();
            let right = self.parse_conjunction()?;
            expr = self.ctx.exprs.alloc(LogicExpr::TemporalBinary {
                operator: op,
                left: expr,
                right,
            });
        }

        // Check for trailing "within N cycles" bounded temporal delay
        let expr = self.try_wrap_bounded_delay(expr);

        // Trailing "in the next cycle" → X(P)
        let mut expr = self.try_wrap_next_cycle(expr);

        // Sentence-final temporal anchor ("…eat Bill first.", "…left
        // yesterday.") for clauses whose path didn't consume it in the VP.
        if let TokenType::TemporalAdverb(anchor) = self.peek().kind {
            if matches!(
                self.tokens.get(self.current + 1).map(|t| &t.kind),
                Some(TokenType::Period) | Some(TokenType::Exclamation) | Some(TokenType::EOF) | None
            ) {
                self.advance();
                expr = self.ctx.exprs.alloc(LogicExpr::TemporalAnchor { anchor, body: expr });
            }
        }

        // Postposed necessary condition: "Y only when X." / "Y only if X." ⇔ Y → X (X is
        // *necessary* for Y — the textbook reading of "only if"). This is the converse direction
        // of the sufficient "Y when X" below, so it must be matched first.
        if self.interner.resolve(self.peek().lexeme).eq_ignore_ascii_case("only")
            && matches!(
                self.tokens.get(self.current + 1).map(|t| &t.kind),
                Some(TokenType::When) | Some(TokenType::If)
            )
        {
            self.advance(); // only
            self.advance(); // when | if
            let condition = self.parse_conjunction()?;
            expr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: expr,
                op: TokenType::If,
                right: condition,
            });
        }
        // Postposed "when": "Y when X." ⇔ "When X, Y." → X → Y
        else if self.check(&TokenType::When) {
            self.advance();
            let condition = self.parse_conjunction()?;
            expr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: condition,
                op: TokenType::Implies,
                right: expr,
            });
        }

        // Biconditional binds LOOSER than disjunction (standard precedence
        // ∨ > ↔). Fold any trailing `iff` with a FULL disjunction as its right
        // operand, so "P if and only if Q or R" is P ↔ (Q ∨ R), not (P ↔ Q) ∨ R.
        while self.check(&TokenType::Iff) {
            self.advance();
            self.current_island += 1;
            let right = self.parse_disjunction()?;
            expr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: expr,
                op: TokenType::Iff,
                right,
            });
        }

        Ok(expr)
    }

    /// Parse conjunction (And) - higher precedence than Or.
    /// Calls parse_atom for operands.
    /// Extracts the subject of a copular predication (the first `Constant` argument
    /// of a copular `Predicate`), digging through degree/aspect/boolean wrappers.
    /// Returns `None` for event predications (NeoEvent) and variable subjects, so
    /// only true copular clauses ("X is ADJ/NP") trigger predicate coordination.
    fn extract_copular_subject(&self, expr: &'a LogicExpr<'a>) -> Option<Symbol> {
        match expr {
            LogicExpr::Predicate { args, .. } => match args.first() {
                Some(Term::Constant(s)) => Some(*s),
                _ => None,
            },
            LogicExpr::Quantifier { body, .. } => self.extract_copular_subject(body),
            LogicExpr::Aspectual { body, .. } => self.extract_copular_subject(body),
            LogicExpr::UnaryOp { operand, .. } => self.extract_copular_subject(operand),
            LogicExpr::BinaryOp { left, .. } => self.extract_copular_subject(left),
            _ => None,
        }
    }

    /// Parses a bare copular-predicate remnant — an adjective ("wealthy") or a
    /// predicate nominal ("a philanthropist") — as `Predicate(subject)`. Returns
    /// `None` (consuming nothing) if the next tokens are not such a remnant.
    fn try_parse_copular_predicate(
        &mut self,
        subject: Symbol,
    ) -> ParseResult<Option<&'a LogicExpr<'a>>> {
        let pred_sym = if let TokenType::Adjective(adj) = self.peek().kind {
            // An adjective-classified word followed by a copula is the
            // SUBJECT of a new clause ("…and ready is not asserted"), not a
            // predicate remnant of the previous one.
            if matches!(
                self.tokens.get(self.current + 1).map(|t| &t.kind),
                Some(TokenType::Is)
                    | Some(TokenType::Are)
                    | Some(TokenType::Was)
                    | Some(TokenType::Were)
            ) {
                return Ok(None);
            }
            self.advance();
            adj
        } else if self.check_article() {
            // "a/an N" predicate nominal — parse the NP and use its head noun.
            let np = self.parse_noun_phrase(false)?;
            np.noun
        } else {
            return Ok(None);
        };
        Ok(Some(self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: pred_sym,
            args: self.ctx.terms.alloc_slice([Term::Constant(subject)]),
            world: None,
        })))
    }

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

            // Non-parallel copular coordination (§2.2): "X is wealthy and a
            // philanthropist" — the remnant after "and" is a bare predicate (an
            // adjective or a predicate nominal) attributed to the SAME copular
            // subject, not a gapped event verb.
            if let Some(subj) = self.extract_copular_subject(expr) {
                let cop_pos = self.current;
                if let Some(p2) = self.try_parse_copular_predicate(subj)? {
                    expr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: expr,
                        op: operator,
                        right: p2,
                    });
                    continue;
                }
                self.current = cop_pos;
            }

            let saved_pos = self.current;
            let standard_attempt = self.try_parse(|p| p.parse_atom());

            // Phase 46: Expanded gapping trigger to support PP gapping, temporal override,
            // and intransitive gapping (bare subject at clause boundary)
            let use_gapping = match &standard_attempt {
                Some(right) => {
                    !self.is_complete_clause(right)
                        && (self.check(&TokenType::Comma)
                            || self.check_content_word()
                            || self.check_preposition()
                            || self.check_temporal_adverb()
                            || self.check(&TokenType::Period)
                            || self.is_at_end())
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
        // A clause-initial adverb ("who FIRST started in 1983", "who ORIGINALLY came
        // out in 1866") modifies the relative clause's event. It hid the verb from the
        // dispatch below, stranding the clue (TrailingTokens) or dropping the clause to
        // `?`. Consume it, parse the rest of the clause, then conjoin the adverb over
        // the gap so nothing is lost. Only when a clause predicate (verb / perfect /
        // modal / auxiliary / copula / negation) actually follows the adverb.
        if matches!(
            self.peek().kind,
            TokenType::Adverb(_) | TokenType::TemporalAdverb(_)
        ) {
            let next_opens_predicate = self.tokens.get(self.current + 1).map_or(false, |t| {
                self.kind_is_verb(&t.kind)
                    || matches!(
                        t.kind,
                        TokenType::Had
                            | TokenType::Auxiliary(_)
                            | TokenType::Not
                            | TokenType::Is
                            | TokenType::Are
                            | TokenType::Was
                            | TokenType::Were
                            | TokenType::Can
                            | TokenType::Could
                            | TokenType::Must
                            | TokenType::Should
                            | TokenType::May
                            | TokenType::Might
                            | TokenType::Would
                            | TokenType::Shall
                            | TokenType::Cannot
                    )
            });
            if next_opens_predicate {
                let adv = match self.peek().kind {
                    TokenType::Adverb(s) | TokenType::TemporalAdverb(s) => s,
                    _ => unreachable!(),
                };
                self.advance();
                let rest = self.parse_relative_clause(gap_var)?;
                let adv_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: adv,
                    args: self.ctx.terms.alloc_slice([Term::Variable(gap_var)]),
                    world: None,
                });
                return Ok(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: rest,
                    op: TokenType::And,
                    right: adv_pred,
                }));
            }
        }

        // "who had the port" — possessive HAVE (past) as the clause verb, NOT a
        // perfect auxiliary (no participle follows); re-tag so check_verb handles it.
        if self.check(&TokenType::Had)
            && !matches!(
                self.tokens.get(self.current + 1).map(|t| &t.kind),
                Some(TokenType::Verb { .. })
            )
        {
            let have_lemma = self.interner.intern("Have");
            self.tokens[self.current].kind = TokenType::Verb {
                lemma: have_lemma,
                time: Time::Past,
                aspect: crate::lexicon::Aspect::Simple,
                class: crate::lexicon::VerbClass::State,
            };
        }

        // "who did 49 jumps" / "who does the dishes" — main verb "do" (performed),
        // NOT do-support (no verb follows); re-tag so check_verb handles it.
        if matches!(self.peek().kind, TokenType::Auxiliary(_))
            && !matches!(
                self.tokens.get(self.current + 1).map(|t| &t.kind),
                Some(TokenType::Verb { .. })
            )
        {
            let lex = self.interner.resolve(self.peek().lexeme).to_lowercase();
            if matches!(lex.as_str(), "did" | "do" | "does") {
                let do_lemma = self.interner.intern("Do");
                let time = if lex == "did" { Time::Past } else { Time::Present };
                self.tokens[self.current].kind = TokenType::Verb {
                    lemma: do_lemma,
                    time,
                    aspect: crate::lexicon::Aspect::Simple,
                    class: crate::lexicon::VerbClass::Activity,
                };
            }
        }

        // Perfect-aspect relative clause: "who HAS done 49 jumps", "who HAVE won",
        // "that HAD been issued in 1868". The perfect auxiliary + a participle is an
        // aspect chain over the gap, NOT possessive HAVE — which the `check_verb` below
        // would greedily consume, stranding the participle (TrailingTokens). Mirror the
        // main-clause perfect dispatch (`parse_aspect_chain`). The participle may be an
        // Ambiguous noun/verb ("done"), so judge it with `kind_is_verb`, and allow an
        // intervening negation ("who has not won").
        let head_word = self.interner.resolve(self.peek().lexeme).to_lowercase();
        let is_perfect_head =
            matches!(head_word.as_str(), "has" | "have") || self.check(&TokenType::Had);
        let next_opens_participle = self.tokens.get(self.current + 1).map_or(false, |t| {
            self.kind_is_verb(&t.kind) || matches!(t.kind, TokenType::Not)
        });
        if is_perfect_head && next_opens_participle {
            return self.parse_aspect_chain_with_term(Term::Variable(gap_var));
        }

        if self.check_verb() {
            return self.parse_verb_phrase_for_restriction(gap_var);
        }

        // Modal-headed relative clause: "that can fly for 40 minutes", "who must
        // attend", "that should win". The modal scopes the event over the gap
        // variable — "the device that can fly" → ◇ ∃e(Fly(e) ∧ Agent(e, x)).
        if self.check_modal() {
            return self.parse_aspect_chain_with_term(Term::Variable(gap_var));
        }

        // Auxiliary-headed relative clause: "who will be studying radiation",
        // "who would win". Record the modality/tense, drop an optional "be" of
        // the progressive, then parse the verb phrase as the restriction.
        if let TokenType::Auxiliary(time) = self.peek().kind {
            self.advance(); // "will" / "would" / "did"
            // Drop the progressive's "be" ("will be studying"). It tokenizes
            // either as TokenType::Be or as a verb with lemma "be".
            let is_be = self.check(&TokenType::Be)
                || matches!(self.peek().kind, TokenType::Verb { lemma, .. }
                    if self.interner.resolve(lemma).eq_ignore_ascii_case("be"));
            if is_be {
                self.advance();
            }
            if self.check_verb() {
                let restriction = self.parse_verb_phrase_for_restriction(gap_var)?;
                return Ok(if time == Time::Future {
                    self.ctx.exprs.alloc(LogicExpr::Temporal {
                        operator: TemporalOperator::Future,
                        body: restriction,
                    })
                } else {
                    restriction
                });
            }
            // "who will be ready" — copular future with an adjective/noun.
            if self.check_content_word() || self.check_article() {
                let pred_np = self.parse_noun_phrase(false)?;
                let base = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: pred_np.noun,
                    args: self.ctx.terms.alloc_slice([Term::Variable(gap_var)]),
                    world: None,
                });
                return Ok(if time == Time::Future {
                    self.ctx.exprs.alloc(LogicExpr::Temporal {
                        operator: TemporalOperator::Future,
                        body: base,
                    })
                } else {
                    base
                });
            }
        }

        // Copular relative: "that is on the table", "that is red".
        if matches!(
            self.peek().kind,
            TokenType::Is | TokenType::Are | TokenType::Was | TokenType::Were
        ) {
            let copula_past = matches!(self.peek().kind, TokenType::Was | TokenType::Were);
            self.advance(); // copula
            let negated = self.check(&TokenType::Not);
            if negated {
                self.advance();
            }
            // A temporal adverb after the copula ("who is NOW with the Tigers", "that
            // was THEN the leader") frames the predication; consume it and conjoin it
            // over the gap once the complement is built, so it is not stranded
            // (ExpectedContentWord at the adverb).
            let rel_temporal_adv = if let TokenType::TemporalAdverb(s) = self.peek().kind {
                self.advance();
                Some(s)
            } else {
                None
            };
            // "that is printing 100 pages", "that is paying the rent" — a
            // PROGRESSIVE verb after the copula is a verb phrase (∃e(Print(e) ∧
            // Agent(e,x) ∧ Theme(e,…))), not a predicate adjective. Gate strictly
            // on Progressive aspect so a PASSIVE past participle ("that was ISSUED
            // in 1868") is NOT mis-read as an active VP — that stays the
            // passive/PP path below.
            if matches!(
                self.peek().kind,
                TokenType::Verb { aspect: crate::lexicon::Aspect::Progressive, .. }
            ) {
                let vp = self.parse_verb_phrase_for_restriction(gap_var)?;
                let vp = self.conjoin_relative_temporal_adverb(vp, gap_var, rel_temporal_adv);
                let vp = if negated {
                    self.ctx.exprs.alloc(LogicExpr::UnaryOp { op: TokenType::Not, operand: vp })
                } else {
                    vp
                };
                return Ok(if copula_past {
                    self.ctx.exprs.alloc(LogicExpr::Temporal {
                        operator: TemporalOperator::Past,
                        body: vp,
                    })
                } else {
                    vp
                });
            }
            let pred: &'a LogicExpr<'a> = if self.check_preposition() {
                let prep = if let TokenType::Preposition(sym) = self.advance().kind {
                    sym
                } else {
                    self.interner.intern("At")
                };
                let obj = self.parse_noun_phrase(false)?;
                self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: prep,
                    args: self
                        .ctx
                        .terms
                        .alloc_slice([Term::Variable(gap_var), Term::Constant(obj.noun)]),
                    world: None,
                })
            } else if self.check_number() {
                // Measure complement: "that is 30 inches long" → Long(x, 30 inches);
                // bare "that is 30 inches" → Measure(x, 30 inches).
                let measure = self.parse_measure_phrase()?;
                let dim = if self.check_content_word() {
                    self.consume_content_word()?
                } else {
                    self.interner.intern("Measure")
                };
                self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: dim,
                    args: self
                        .ctx
                        .terms
                        .alloc_slice([Term::Variable(gap_var), *measure]),
                    world: None,
                })
            } else {
                let adj = self.consume_content_word()?;
                self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: adj,
                    args: self.ctx.terms.alloc_slice([Term::Variable(gap_var)]),
                    world: None,
                })
            };
            // Trailing PPs on a copular relative ("that was issued in 1868",
            // "that is from Spain") — conjoin each as a predicate over the gap.
            let mut pred = pred;
            while self.check_preposition() {
                let prep = if let TokenType::Preposition(s) = self.advance().kind {
                    s
                } else {
                    break;
                };
                let obj_term = if self.check_number() {
                    *self.parse_measure_phrase()?
                } else if self.check_content_word() || self.check_article() {
                    Term::Constant(self.parse_noun_phrase(false)?.noun)
                } else {
                    self.current -= 1;
                    break;
                };
                let pp = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: prep,
                    args: self.ctx.terms.alloc_slice([Term::Variable(gap_var), obj_term]),
                    world: None,
                });
                pred = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: pred,
                    op: TokenType::And,
                    right: pp,
                });
            }
            let pred = self.conjoin_relative_temporal_adverb(pred, gap_var, rel_temporal_adv);
            let pred = if copula_past {
                &*self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: TemporalOperator::Past,
                    body: pred,
                })
            } else {
                pred
            };
            return Ok(if negated {
                self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                    op: TokenType::Not,
                    operand: pred,
                })
            } else {
                pred
            });
        }

        // Handle "do/does (not)" in relative clauses: "who do not shave themselves"
        if self.check(&TokenType::Do) || self.check(&TokenType::Does) {
            self.advance(); // consume "do/does"

            let is_negated = self.check(&TokenType::Not);
            if is_negated {
                self.advance(); // consume "not"
            }

            if self.check_verb() {
                let verb = self.consume_verb();

                // Check for reflexive object: "shave themselves"
                let roles = if self.check(&TokenType::Reflexive) {
                    self.advance(); // consume "themselves/himself"
                    vec![
                        (ThematicRole::Agent, Term::Variable(gap_var)),
                        (ThematicRole::Theme, Term::Variable(gap_var)),
                    ]
                } else if self.check_content_word() || self.check_article() {
                    // Parse object NP
                    let obj = self.parse_noun_phrase(false)?;
                    vec![
                        (ThematicRole::Agent, Term::Variable(gap_var)),
                        (ThematicRole::Theme, Term::Constant(obj.noun)),
                    ]
                } else {
                    // Intransitive
                    vec![(ThematicRole::Agent, Term::Variable(gap_var))]
                };

                let event_var = self.get_event_var();
                let suppress_existential = self.drs.in_conditional_antecedent();
                let event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var,
                    verb,
                    roles: self.ctx.roles.alloc_slice(roles),
                    modifiers: self.ctx.syms.alloc_slice(vec![]),
                    suppress_existential,
                    world: None,
                })));

                if is_negated {
                    return Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand: event,
                    }));
                }
                return Ok(event);
            }
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

                // A STRANDED preposition ("the animal Eva works WITH", "the case Bob
                // paid FOR") makes the GAP the object of that preposition, not the
                // verb's direct Theme. Detect a preposition with NO overt object (the
                // matrix clause follows immediately) and bind the gap to it below.
                let stranded_prep: Option<Symbol> = if self.check_preposition()
                    && !self.check_to_preposition()
                {
                    let has_overt_object = matches!(
                        self.tokens.get(self.current + 1).map(|t| &t.kind),
                        Some(TokenType::Article(_))
                            | Some(TokenType::Noun(_))
                            | Some(TokenType::ProperName(_))
                            | Some(TokenType::Number(_))
                            | Some(TokenType::Cardinal(_))
                            | Some(TokenType::Pronoun { .. })
                            | Some(TokenType::Possessive)
                    );
                    let prep_sym = match &self.peek().kind {
                        TokenType::Preposition(s) => Some(*s),
                        _ => None,
                    };
                    if !has_overt_object && prep_sym.is_some() {
                        self.advance();
                        prep_sym
                    } else {
                        None
                    }
                } else {
                    None
                };

                let mut roles: Vec<(ThematicRole, Term<'a>)> =
                    vec![(ThematicRole::Agent, Term::Constant(rel_subject.noun))];
                if stranded_prep.is_none() {
                    roles.push((ThematicRole::Theme, Term::Variable(gap_var)));
                }

                while self.check_to_preposition() {
                    self.advance();
                    if self.check_content_word() || self.check_article() {
                        let recipient = self.parse_noun_phrase(false)?;
                        roles.push((ThematicRole::Recipient, Term::Constant(recipient.noun)));
                    }
                }

                let event_var = self.get_event_var();

                // Absorb the embedded clause's trailing PP complements onto the
                // EVENT ("photographed in 1989" → In(e, 1989), "issued at the depot"
                // → At(e, depot)) so an object-gap reduced relative does not strand
                // them. They attach over the event var, inside its existential scope.
                let mut pp_preds: Vec<&'a LogicExpr<'a>> = Vec::new();
                // The stranded preposition's object IS the gap ("Eva works with [x]" →
                // With(e, x)); attach it over the event so it falls in the event scope.
                if let Some(prep) = stranded_prep {
                    pp_preds.push(self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: prep,
                        args: self
                            .ctx
                            .terms
                            .alloc_slice([Term::Variable(event_var), Term::Variable(gap_var)]),
                        world: None,
                    }));
                }
                while self.check_preposition() {
                    let prep = if let TokenType::Preposition(s) = self.advance().kind {
                        s
                    } else {
                        break;
                    };
                    let obj_term = if self.check_number() {
                        *self.parse_measure_phrase()?
                    } else if self.check_content_word() || self.check_article() {
                        Term::Constant(self.parse_noun_phrase(false)?.noun)
                    } else {
                        self.current -= 1;
                        break;
                    };
                    pp_preds.push(self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: prep,
                        args: self.ctx.terms.alloc_slice([Term::Variable(event_var), obj_term]),
                        world: None,
                    }));
                }
                let has_pps = !pp_preds.is_empty();

                // With PPs, suppress the NeoEvent's own ∃e and wrap an explicit one so
                // the PP conjuncts fall inside the event's scope; without PPs, the
                // NeoEvent emits its own ∃e exactly as before (byte-identical).
                let suppress_existential = self.drs.in_conditional_antecedent() || has_pps;
                let neo = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var,
                    verb,
                    roles: self.ctx.roles.alloc_slice(roles),
                    modifiers: self.ctx.syms.alloc_slice(vec![]),
                    suppress_existential,
                    world: None,
                })));
                let mut event_body = neo;
                for pp in pp_preds {
                    event_body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: event_body,
                        op: TokenType::And,
                        right: pp,
                    });
                }
                let this_event: &'a LogicExpr<'a> = if has_pps {
                    self.ctx.exprs.alloc(LogicExpr::Quantifier {
                        kind: crate::ast::QuantifierKind::Existential,
                        variable: event_var,
                        body: event_body,
                        island_id: self.current_island,
                    })
                } else {
                    event_body
                };

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
                match self.resolve_pronoun(gender, number) {
                    Ok(resolved) => match resolved {
                        super::ResolvedPronoun::Variable(s) | super::ResolvedPronoun::Constant(s) => s,
                    },
                    Err(e) => return Some(Err(e)),
                }
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

    fn try_parse_of_pair_xor(&mut self) -> ParseResult<Option<&'a LogicExpr<'a>>> {
        let start = self.current;

        // Must start with the preposition "of"
        let is_of = {
            let text = self.interner.resolve(self.peek().lexeme).to_lowercase();
            text == "of" && matches!(self.peek().kind, TokenType::Preposition(_))
        };
        if !is_of {
            return Ok(None);
        }

        // Scan for the structural boundaries: the comma ending "Of NP₁ and NP₂,"
        // and the last "and" before it (the NP₁/NP₂ separator). These give a
        // robust fallback when full-NP parsing misaligns with the boundary.
        let scan_start = self.current + 1; // one past "of"
        let comma_pos = {
            let mut found = None;
            for i in scan_start..self.tokens.len() {
                match &self.tokens[i].kind {
                    TokenType::Period | TokenType::EOF => break,
                    TokenType::Comma => {
                        found = Some(i);
                        break;
                    }
                    _ => {}
                }
            }
            match found {
                Some(p) => p,
                None => return Ok(None),
            }
        };
        let and_pos = {
            let mut found = None;
            for i in scan_start..comma_pos {
                if self.interner.resolve(self.tokens[i].lexeme).to_lowercase() == "and" {
                    found = Some(i);
                }
            }
            match found {
                Some(p) => p,
                None => return Ok(None),
            }
        };
        let one_ok = self
            .tokens
            .get(comma_pos + 1)
            .map(|t| self.interner.resolve(t.lexeme).to_lowercase() == "one")
            .unwrap_or(false);
        if !one_ok {
            return Ok(None);
        }

        // Last content head in [lo, hi) (compounding a preceding cardinal) — the
        // robust fallback when parse_noun_phrase misaligns. A verb-only word
        // ("stamp", "print") or an ambiguous noun/verb token can head a puzzle NP
        // too, so they count as heads here; the fallback yields a bare constant
        // (its modifiers are lost — only reached when the full parse failed).
        fn scan_head<'a, 'ctx, 'int>(p: &mut Parser<'a, 'ctx, 'int>, lo: usize, hi: usize) -> Option<Symbol> {
            let mut i = hi;
            while i > lo {
                i -= 1;
                let head = match p.tokens[i].kind {
                    TokenType::ProperName(s) | TokenType::Noun(s) => Some(s),
                    TokenType::Verb { lemma, .. } => Some(lemma),
                    TokenType::Ambiguous { ref primary, ref alternatives } => {
                        match **primary {
                            TokenType::Noun(s) | TokenType::ProperName(s) => Some(s),
                            TokenType::Verb { lemma, .. } => Some(lemma),
                            _ => alternatives.iter().find_map(|t| match t {
                                TokenType::Noun(s) | TokenType::ProperName(s) => Some(*s),
                                TokenType::Verb { lemma, .. } => Some(*lemma),
                                _ => None,
                            }),
                        }
                    }
                    _ => None,
                };
                if let Some(s) = head {
                    if i > lo {
                        if let TokenType::Cardinal(n) = p.tokens[i - 1].kind {
                            return Some(
                                p.interner
                                    .intern(&format!("{}_{}", n, p.interner.resolve(s))),
                            );
                        }
                    }
                    return Some(s);
                }
            }
            None
        }

        // Build one side of the pair, parsing the full NP for correctness but
        // committing only if it lands exactly at `boundary` without erroring;
        // otherwise fall back to the robust scan head (a bare constant). A
        // descriptive NP (determiner / adjective / possessor / PP / relative
        // clause) becomes a fresh existential variable carrying a restrictor so
        // two NPs sharing a head noun stay distinct; a bare proper name stays a
        // referring constant. Parsing is non-fatal so a malformed NP never breaks
        // an otherwise-valid clue.
        fn build_entity<'a, 'ctx, 'int>(
            p: &mut Parser<'a, 'ctx, 'int>,
            boundary: usize,
        ) -> ParseResult<OfEntity<'a>> {
            // An of-pair member is a nominal description ("the skydiving trip"),
            // so a verb-ambiguous head ("trip", "place") folds its modifier into
            // the head noun instead of being read as a verb — without this the NP
            // misaligns at the following "and" and the lossy scan_head fallback
            // drops the modifier ("the skydiving trip" → bare `Trip`).
            let saved_ctx = p.nominal_np_context;
            p.nominal_np_context = true;
            let np_result = p.parse_noun_phrase(true);
            p.nominal_np_context = saved_ctx;
            let np = np_result?;
            // who/that/where/whose relative, or a REDUCED relative ("the island first
            // seen by Captain Norris", "the well cut through chalk") — both restrict
            // the member and run up to the structural boundary ("and"/comma).
            let has_rel = (p.check(&TokenType::Who)
                || p.check(&TokenType::That)
                || p.check(&TokenType::Where)
                || p.check(&TokenType::Whose))
                && p.current < boundary;
            let has_reduced = p.peek_heads_reduced_relative_participle() && p.current < boundary;
            let is_desc = np.definiteness.is_some()
                || !np.adjectives.is_empty()
                || np.possessor.is_some()
                || !np.pps.is_empty()
                || has_rel
                || has_reduced;
            let (sym, is_var) = if is_desc {
                (p.next_var_name(), true)
            } else {
                (np.noun, false)
            };
            let term = if is_var {
                Term::Variable(sym)
            } else {
                Term::Constant(sym)
            };
            let rel = if has_rel {
                p.try_attach_relative(term)?
            } else {
                None
            };
            let reduced = if has_reduced {
                p.try_consume_reduced_relative(term)?
            } else {
                None
            };
            if p.current != boundary {
                return Err(ParseError {
                    kind: ParseErrorKind::Custom("of-pair NP misaligned".into()),
                    span: p.current_span(),
                });
            }
            let restrictor = if is_var {
                // Head noun + adjectives + possessor, all over the fresh variable.
                let mut r = p.nominal_predication(term, &np);
                for pp in np.pps {
                    let pp_sub = p.substitute_pp_placeholder(pp, sym);
                    r = p.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: r,
                        op: TokenType::And,
                        right: pp_sub,
                    });
                }
                for rc in rel.into_iter().chain(reduced) {
                    r = p.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: r,
                        op: TokenType::And,
                        right: rc,
                    });
                }
                Some(r)
            } else {
                None
            };
            Ok(OfEntity { sym, is_var, term, restrictor })
        }

        self.advance(); // consume "of"

        // NP₁, bounded by the "and".
        let e1 = match self.try_parse(|p| build_entity(p, and_pos)) {
            Some(e) => e,
            None => {
                self.current = and_pos;
                match scan_head(self, scan_start, and_pos) {
                    Some(h) => OfEntity { sym: h, is_var: false, term: Term::Constant(h), restrictor: None },
                    None => {
                        self.current = start;
                        return Ok(None);
                    }
                }
            }
        };
        self.advance(); // "and" separating NP₁ from NP₂

        // NP₂, bounded by the comma.
        let e2 = match self.try_parse(|p| build_entity(p, comma_pos)) {
            Some(e) => e,
            None => {
                self.current = comma_pos;
                match scan_head(self, and_pos + 1, comma_pos) {
                    Some(h) => OfEntity { sym: h, is_var: false, term: Term::Constant(h), restrictor: None },
                    None => {
                        self.current = start;
                        return Ok(None);
                    }
                }
            }
        };
        self.advance(); // ","
        self.advance(); // "one" (validated above)
        // "one TYPE is …" / "one PERSON did …" — a redundant classifier noun
        // after "one" (the of-pair already binds the entity); skip it so VP₁
        // starts at the real predicate. Only when a Noun is directly followed by
        // a copula / auxiliary / verb (the predicate), never a bare object NP.
        {
            // A classifier is a generic noun ("one PERSON …") or a noun-or-verb
            // ambiguous word the lexicon also reads as a verb ("one TYPE …", "one
            // KIND …") — matched by lexeme so the verb tagging doesn't hide it. A
            // genuine aspectual verb ("one STARTED running") is never a classifier.
            let cur_is_classifier = matches!(
                self.tokens.get(self.current).map(|t| &t.kind),
                Some(TokenType::Noun(_))
            ) || self
                .tokens
                .get(self.current)
                .map(|t| {
                    matches!(
                        self.interner.resolve(t.lexeme).to_lowercase().as_str(),
                        "type" | "kind" | "sort" | "variety" | "category" | "version"
                    )
                })
                .unwrap_or(false);
            if cur_is_classifier
                && matches!(
                    self.tokens.get(self.current + 1).map(|t| &t.kind),
                    Some(TokenType::Is | TokenType::Are | TokenType::Was | TokenType::Were
                        | TokenType::Verb { .. } | TokenType::Auxiliary(_))
                )
            {
                self.advance(); // skip the classifier noun
            }
        }

        // Bound VP₁ at the "and the other" marker so a verb-object VP ("one teaches
        // yoga and the other is …") does not swallow "and the other …" as
        // coordinated objects. Scan for And + (article) + "other" and temporarily
        // turn that "and" into a clause terminator (a Period the VP parser stops
        // at); it is RESTORED on every exit path (errors, misalign, success).
        let other_and_pos = {
            let mut found = None;
            let mut i = self.current;
            while i + 2 < self.tokens.len() {
                if matches!(self.tokens[i].kind, TokenType::Period | TokenType::EOF) {
                    break;
                }
                if matches!(self.tokens[i].kind, TokenType::And)
                    && matches!(self.tokens[i + 1].kind, TokenType::Article(_))
                    && self
                        .interner
                        .resolve(self.tokens[i + 2].lexeme)
                        .eq_ignore_ascii_case("other")
                {
                    found = Some(i);
                    break;
                }
                i += 1;
            }
            found
        };
        let saved_and_tok = other_and_pos.map(|p| self.tokens[p].clone());
        if let Some(p) = other_and_pos {
            let mut t = self.tokens[p].clone();
            t.kind = TokenType::Period;
            self.tokens[p] = t;
        }

        macro_rules! restore_and {
            () => {
                if let (Some(p), Some(tok)) = (other_and_pos, saved_and_tok.as_ref()) {
                    self.tokens[p] = tok.clone();
                }
            };
        }

        // A verb phrase parsed from `start`, with `e` as its subject — restoring the
        // bounded "and" before propagating any parse error.
        macro_rules! vp_with {
            ($start:expr, $e:expr) => {{
                self.current = $start;
                let __r = if $e.is_var {
                    self.parse_predicate_with_subject_as_var($e.sym)
                } else {
                    self.parse_predicate_with_subject($e.sym)
                };
                match __r {
                    Ok(v) => v,
                    Err(err) => {
                        restore_and!();
                        return Err(err);
                    }
                }
            }};
        }

        // VP₁ with e1 as subject (stops at the bounded marker).
        let vp1_start = self.current;
        let vp1_e1 = vp_with!(vp1_start, e1);

        // Expect the "and the other" marker. When bounded, VP₁ stopped at the
        // Period that replaced "and"; an optional comma may precede it.
        if self.check(&TokenType::Comma) { self.advance(); }
        if let Some(marker) = other_and_pos {
            if self.current != marker || !self.check(&TokenType::Period) {
                restore_and!();
                self.current = start;
                return Ok(None);
            }
            self.advance(); // the bounded "and"
        } else {
            if self.interner.resolve(self.peek().lexeme).to_lowercase() != "and" {
                self.current = start;
                return Ok(None);
            }
            self.advance(); // consume "and"
        }

        // Expect "the"
        if !self.check_article() {
            restore_and!();
            self.current = start;
            return Ok(None);
        }
        self.advance(); // consume "the"

        // Expect "other"
        if self.interner.resolve(self.peek().lexeme).to_lowercase() != "other" {
            restore_and!();
            self.current = start;
            return Ok(None);
        }
        self.advance(); // consume "other"
        // "the other TYPE is …" — skip the redundant classifier noun (mirrors the
        // "one TYPE" skip above) so VP₂ starts at the real predicate.
        {
            // A classifier is a generic noun ("one PERSON …") or a noun-or-verb
            // ambiguous word the lexicon also reads as a verb ("one TYPE …", "one
            // KIND …") — matched by lexeme so the verb tagging doesn't hide it. A
            // genuine aspectual verb ("one STARTED running") is never a classifier.
            let cur_is_classifier = matches!(
                self.tokens.get(self.current).map(|t| &t.kind),
                Some(TokenType::Noun(_))
            ) || self
                .tokens
                .get(self.current)
                .map(|t| {
                    matches!(
                        self.interner.resolve(t.lexeme).to_lowercase().as_str(),
                        "type" | "kind" | "sort" | "variety" | "category" | "version"
                    )
                })
                .unwrap_or(false);
            if cur_is_classifier
                && matches!(
                    self.tokens.get(self.current + 1).map(|t| &t.kind),
                    Some(TokenType::Is | TokenType::Are | TokenType::Was | TokenType::Were
                        | TokenType::Verb { .. } | TokenType::Auxiliary(_))
                )
            {
                self.advance(); // skip the classifier noun
            }
        }

        // VP₂ with e2 as subject, then re-parse each VP with the other entity.
        let vp2_start = self.current;
        let vp2_e2 = vp_with!(vp2_start, e2);
        let end_pos = self.current;
        let vp1_e2 = vp_with!(vp1_start, e2);
        let vp2_e1 = vp_with!(vp2_start, e1);
        self.current = end_pos;

        // All VP parses done — the bounded "and" is no longer needed; restore it
        // before building the result so the token stream is left pristine.
        restore_and!();

        // Build: (VP₁(e1) ∧ VP₂(e2)) ∨ (VP₁(e2) ∧ VP₂(e1))
        let branch1 = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
            left: vp1_e1,
            op: TokenType::And,
            right: vp2_e2,
        });
        let branch2 = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
            left: vp1_e2,
            op: TokenType::And,
            right: vp2_e1,
        });
        let mut result: &'a LogicExpr<'a> = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
            left: branch1,
            op: TokenType::Or,
            right: branch2,
        });

        // "Of A and B" presents two DISTINCT entities. A variable entity could
        // otherwise co-refer with the other side and collapse the XOR, so assert
        // the inequality whenever either side is a variable; two proper-name
        // constants are distinct by the unique-name assumption already.
        if e1.is_var || e2.is_var {
            let identity = self.ctx.exprs.alloc(LogicExpr::Identity {
                left: self.ctx.terms.alloc(e1.term),
                right: self.ctx.terms.alloc(e2.term),
            });
            let ineq = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: identity,
            });
            result = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: ineq,
                op: TokenType::And,
                right: result,
            });
        }

        // A description's restrictor (head noun, adjectives, possessor, PPs,
        // relative clause) holds in BOTH XOR branches, so it conjoins outside the
        // disjunction and is bound by the existential opened below.
        if let Some(r2) = e2.restrictor {
            result = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: r2,
                op: TokenType::And,
                right: result,
            });
        }
        if let Some(r1) = e1.restrictor {
            result = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: r1,
                op: TokenType::And,
                right: result,
            });
        }
        if e2.is_var {
            result = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind: QuantifierKind::Existential,
                variable: e2.sym,
                body: result,
                island_id: self.current_island,
            });
        }
        if e1.is_var {
            result = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind: QuantifierKind::Existential,
                variable: e1.sym,
                body: result,
                island_id: self.current_island,
            });
        }
        Ok(Some(result))
    }
}

// Phase 46: Helper methods for generalized gapping (not part of trait)
impl<'a, 'ctx, 'int> Parser<'a, 'ctx, 'int> {
    /// Helper to extract verb specifically from NeoEvent structures
    fn extract_neo_event_verb(&self, expr: &LogicExpr<'a>) -> Option<Symbol> {
        match expr {
            LogicExpr::NeoEvent(data) => Some(data.verb),
            LogicExpr::Quantifier { body, .. } => self.extract_neo_event_verb(body),
            LogicExpr::BinaryOp { left, right, .. } => {
                self.extract_neo_event_verb(left)
                    .or_else(|| self.extract_neo_event_verb(right))
            }
            LogicExpr::Temporal { body, .. } => self.extract_neo_event_verb(body),
            LogicExpr::Aspectual { body, .. } => self.extract_neo_event_verb(body),
            _ => None,
        }
    }

    /// Build roles for gapped clause using template guidance.
    /// NP args map to Theme/Recipient roles, PP args map by preposition type.
    fn build_gapped_roles(
        &self,
        subject_term: Term<'a>,
        np_args: &[Term<'a>],
        pp_args: &[(Symbol, Term<'a>)],
        template: &Option<EventTemplate<'a>>,
    ) -> Vec<(ThematicRole, Term<'a>)> {
        // Agent-gapping (non-constituent coordination, §2.1): "John gave Mary a book
        // and Sue a pen" — the remnant (Sue, a pen) fills the template's non-agent
        // NP roles (Recipient, Theme) IN ORDER and the agent is SHARED from the
        // template (John). Detected when the remnant count (subject + np_args) equals
        // the number of non-agent NP roles (≥ 2), i.e. the agent is the gap.
        if let Some(tmpl) = template {
            if let Some(shared_agent) = &tmpl.agent {
                let np_template_roles: Vec<_> = tmpl
                    .non_agent_roles
                    .iter()
                    .filter(|(r, _)| {
                        matches!(
                            r,
                            ThematicRole::Theme | ThematicRole::Recipient | ThematicRole::Patient
                        )
                    })
                    .collect();
                // A LONE bare-NP remnant after a complete transitive clause is
                // object coordination ("John saw himself and Mary" — Mary is
                // a second THEME, the agent is shared), not a new agent
                // inheriting the template's object.
                if np_template_roles.len() == 1 && np_args.is_empty() && pp_args.is_empty() {
                    let (role, _) = np_template_roles[0];
                    return vec![
                        (ThematicRole::Agent, shared_agent.clone()),
                        (*role, subject_term),
                    ];
                }
                if np_template_roles.len() >= 2 && 1 + np_args.len() == np_template_roles.len() {
                    let mut roles = vec![(ThematicRole::Agent, shared_agent.clone())];
                    let remnants: Vec<Term<'a>> = std::iter::once(subject_term)
                        .chain(np_args.iter().cloned())
                        .collect();
                    for ((role, _), arg) in np_template_roles.iter().zip(remnants.iter()) {
                        roles.push((*role, arg.clone()));
                    }
                    // Inherit template PP roles when none are overt in the remnant.
                    if pp_args.is_empty() {
                        for (role, term) in tmpl.non_agent_roles.iter().filter(|(r, _)| {
                            matches!(
                                r,
                                ThematicRole::Goal
                                    | ThematicRole::Source
                                    | ThematicRole::Location
                                    | ThematicRole::Instrument
                            )
                        }) {
                            roles.push((*role, term.clone()));
                        }
                    } else {
                        for (prep, term) in pp_args {
                            roles.push((self.preposition_to_role(*prep), term.clone()));
                        }
                    }
                    return roles;
                }
            }
        }

        let mut roles = vec![(ThematicRole::Agent, subject_term)];

        match template {
            Some(tmpl) => {
                let template_roles = &tmpl.non_agent_roles;

                // Separate template roles into NP-type and PP-type
                let np_template_roles: Vec<_> = template_roles
                    .iter()
                    .filter(|(r, _)| {
                        matches!(
                            r,
                            ThematicRole::Theme | ThematicRole::Recipient | ThematicRole::Patient
                        )
                    })
                    .collect();

                let pp_template_roles: Vec<_> = template_roles
                    .iter()
                    .filter(|(r, _)| {
                        matches!(
                            r,
                            ThematicRole::Goal
                                | ThematicRole::Source
                                | ThematicRole::Location
                                | ThematicRole::Instrument
                        )
                    })
                    .collect();

                // Handle NPs by matching to template NP roles
                match (np_template_roles.len(), np_args.len()) {
                    (0, 0) => {} // Intransitive - no NP roles
                    (_, 0) => {
                        // Use all template NP roles unchanged
                        for (role, term) in &np_template_roles {
                            roles.push((*role, term.clone()));
                        }
                    }
                    (n, 1) if n > 0 => {
                        // 1 NP arg: replace LAST NP role (usually Theme), keep others
                        for (role, term) in np_template_roles.iter().take(n - 1) {
                            roles.push((*role, term.clone()));
                        }
                        if let Some((last_role, _)) = np_template_roles.last() {
                            roles.push((*last_role, np_args[0].clone()));
                        }
                    }
                    (n, m) if m == n => {
                        // Same count: replace all NP roles in order
                        for ((role, _), arg) in np_template_roles.iter().zip(np_args.iter()) {
                            roles.push((*role, arg.clone()));
                        }
                    }
                    (_, _) => {
                        // Fallback: assign Theme to each NP
                        for (i, arg) in np_args.iter().enumerate() {
                            let role = np_template_roles
                                .get(i)
                                .map(|(r, _)| *r)
                                .unwrap_or(ThematicRole::Theme);
                            roles.push((role, arg.clone()));
                        }
                    }
                }

                // Handle PPs: use parsed PPs if provided, else use template
                if pp_args.is_empty() {
                    // Use template PP roles unchanged
                    for (role, term) in &pp_template_roles {
                        roles.push((*role, term.clone()));
                    }
                } else {
                    // Use parsed PPs, map preposition to role
                    for (prep, term) in pp_args {
                        let role = self.preposition_to_role(*prep);
                        roles.push((role, term.clone()));
                    }
                }
            }
            None => {
                // No template: backward-compat hardcoded Agent + Theme
                for arg in np_args {
                    roles.push((ThematicRole::Theme, arg.clone()));
                }
                for (prep, term) in pp_args {
                    let role = self.preposition_to_role(*prep);
                    roles.push((role, term.clone()));
                }
            }
        }
        roles
    }

    /// Map preposition to thematic role
    fn preposition_to_role(&self, prep: Symbol) -> ThematicRole {
        let prep_str = self.interner.resolve(prep).to_lowercase();
        match prep_str.as_str() {
            "to" | "toward" | "towards" => ThematicRole::Goal,
            "from" => ThematicRole::Source,
            "in" | "on" | "at" => ThematicRole::Location,
            "with" | "by" => ThematicRole::Instrument,
            _ => ThematicRole::Location, // Default fallback
        }
    }

    /// Check if modifier is temporal (for override filtering)
    fn is_temporal_modifier(&self, sym: Symbol) -> bool {
        let s = self.interner.resolve(sym).to_lowercase();
        matches!(
            s.as_str(),
            "yesterday" | "today" | "tomorrow" | "now" | "then" | "past" | "future"
        )
    }
}
