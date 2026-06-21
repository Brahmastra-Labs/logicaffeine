//! Verb phrase parsing with event semantics and thematic roles.
//!
//! This module handles the core verbal predication including:
//!
//! - **Intransitive verbs**: "John runs" → `∃e(Run(e) ∧ Agent(e,John))`
//! - **Transitive verbs**: "John loves Mary" → `∃e(Love(e) ∧ Agent(e,John) ∧ Theme(e,Mary))`
//! - **Ditransitive verbs**: "John gives Mary a book" → with Goal role
//! - **Copula constructions**: "John is tall", "John is a doctor"
//! - **Control verbs**: "John wants to run" → raising/control structures
//! - **Plural subjects**: "John and Mary run", "John and Mary love each other"
//! - **VP respectively**: "John and Mary love Sue and Bill respectively"
//!
//! # Neo-Davidsonian Event Semantics
//!
//! Verbs introduce event variables with thematic roles:
//! - **Agent**: The doer of the action
//! - **Theme/Patient**: The entity affected
//! - **Goal/Recipient**: The target of transfer
//! - **Instrument**: The tool used
//!
//! Events are represented using `LogicExpr::NeoEvent` with a verb symbol and
//! a list of (ThematicRole, Term) pairs.

use super::clause::ClauseParsing;
use super::modal::ModalParsing;
use super::noun::NounParsing;
use super::pragmatics::PragmaticsParsing;
use super::quantifier::QuantifierParsing;
use super::{ParseResult, Parser};
use crate::ast::{
    AspectOperator, LogicExpr, NeoEventData, NounPhrase, QuantifierKind, TemporalOperator, Term,
    ThematicRole,
};
use crate::drs::{Gender, Number, ReferentSource};
use crate::error::{ParseError, ParseErrorKind};
use logicaffeine_base::Symbol;
use crate::lexer::Lexer;
use crate::lexicon::{Aspect, Definiteness, Time};
use crate::token::{FocusKind, Span, Token, TokenType};

use crate::ast::Stmt;

/// Trait for parsing verb phrases in declarative (logic) mode.
///
/// Provides methods for parsing predicates with subjects, control verbs,
/// and plural/group constructions with Neo-Davidsonian event semantics.
pub trait LogicVerbParsing<'a, 'ctx, 'int> {
    /// Parses a verb phrase given the subject as a constant symbol.
    fn parse_predicate_with_subject(&mut self, subject_symbol: Symbol)
        -> ParseResult<&'a LogicExpr<'a>>;
    /// Parses a verb phrase with subject as a bound variable.
    fn parse_predicate_with_subject_as_var(&mut self, subject_symbol: Symbol)
        -> ParseResult<&'a LogicExpr<'a>>;
    /// Attempts to parse a plural subject ("John and Mary verb").
    /// Returns `Ok(Some(expr))` on success, `Ok(None)` if not plural, `Err` on semantic error.
    fn try_parse_plural_subject(&mut self, first_subject: &NounPhrase<'a>)
        -> Result<Option<&'a LogicExpr<'a>>, ParseError>;
    /// Parses control verb structures: "wants to VP", "persuaded X to VP".
    fn parse_control_structure(
        &mut self,
        subject: &NounPhrase<'a>,
        verb: Symbol,
        verb_time: Time,
    ) -> ParseResult<&'a LogicExpr<'a>>;
    /// Checks if a verb is a control verb (want, try, persuade, etc.).
    fn is_control_verb(&self, verb: Symbol) -> bool;
    /// Builds a predicate for intransitive verbs with multiple subjects.
    fn build_group_predicate(
        &mut self,
        subjects: &[Symbol],
        verb: Symbol,
        verb_time: Time,
    ) -> &'a LogicExpr<'a>;
    /// Builds a transitive predicate with group subject and group object.
    fn build_group_transitive(
        &mut self,
        subjects: &[Symbol],
        objects: &[Symbol],
        verb: Symbol,
        verb_time: Time,
    ) -> &'a LogicExpr<'a>;
}

/// Trait for parsing verb phrases in imperative (LOGOS) mode.
///
/// Provides methods for parsing statements rather than logical propositions.
pub trait ImperativeVerbParsing<'a, 'ctx, 'int> {
    /// Parses a statement with the given subject symbol.
    fn parse_statement_with_subject(&mut self, subject_symbol: Symbol)
        -> ParseResult<Stmt<'a>>;
}

impl<'a, 'ctx, 'int> Parser<'a, 'ctx, 'int> {
    /// Nominal copula predication: the body of "SUBJ is (the) PRED-NP".
    ///
    /// The subject is identified with the predicate nominal, so every property
    /// of the predicate NP is predicated of the SUBJECT term:
    /// `Pred(subj) ∧ Adj_i(subj) ∧ [Possesses(possessor, subj)]`, recursing
    /// through the possessor's own possessor chain. This keeps the genitive
    /// constraint — "X is the Alvarado family's house" entails the Alvarado
    /// family possesses X — instead of dropping it.
    pub(super) fn nominal_predication(
        &mut self,
        subject_term: Term<'a>,
        pred_np: &NounPhrase<'a>,
    ) -> &'a LogicExpr<'a> {
        let mut result = self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: pred_np.noun,
            args: self.ctx.terms.alloc_slice([subject_term]),
            world: None,
        });

        for &adj in pred_np.adjectives {
            let adj_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: adj,
                args: self.ctx.terms.alloc_slice([subject_term]),
                world: None,
            });
            result = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: result,
                op: TokenType::And,
                right: adj_pred,
            });
        }

        if let Some(possessor) = pred_np.possessor {
            let poss_logic = self.possessor_predication(possessor, subject_term);
            result = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: result,
                op: TokenType::And,
                right: poss_logic,
            });
        }

        result
    }

    /// THE single decision point for whether a possessor / passive-by-agent NP
    /// refers by a bare constant or must become its own restrictor-carrying entity.
    /// Returns the term that stands for the possessor/agent in the relation, plus
    /// (when descriptive) the fresh variable and the restrictor that scopes it:
    ///   - a **bare** NP (no adjectives, no nested genitive, no PPs — "Agnes",
    ///     "His", "John's") → `(Term::Constant(np.noun), None)`, the referring
    ///     constant, so its output is byte-identical to the historical form;
    ///   - a **descriptive** NP ("the old captain", "the Woodard family", "the red
    ///     team") → `(Term::Variable(pvar), Some((pvar, restrictor)))` where the
    ///     restrictor is `nominal_predication(Variable(pvar), np)` conjoined with
    ///     each of the NP's PPs (substituted onto `pvar`), so EVERY modifier
    ///     survives instead of collapsing to the bare head.
    ///
    /// The restrictor recurses through [`Self::nominal_predication`] (which re-enters
    /// [`Self::possessor_entity`] for a nested genitive), so an arbitrarily deep
    /// "X's B's C" is handled uniformly. Callers wrap the relation they build over
    /// the returned term with [`Self::wrap_in_possessor_entity`].
    pub(super) fn possessor_entity(
        &mut self,
        np: &NounPhrase<'a>,
    ) -> (Term<'a>, Option<(Symbol, &'a LogicExpr<'a>)>) {
        let is_descriptive = !np.adjectives.is_empty()
            || np.possessor.is_some()
            || !np.pps.is_empty();
        if !is_descriptive {
            return (Term::Constant(np.noun), None);
        }
        let pvar = self.next_var_name();
        let mut restr = self.nominal_predication(Term::Variable(pvar), np);
        for pp in np.pps {
            let pp_sub = self.substitute_pp_placeholder(pp, pvar);
            restr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: restr,
                op: TokenType::And,
                right: pp_sub,
            });
        }
        (Term::Variable(pvar), Some((pvar, restr)))
    }

    /// Scope a relation built over a possessor/agent term inside that entity's
    /// existential, when [`Self::possessor_entity`] produced a restrictor:
    ///   - `None` (bare possessor/agent) → `relation` unchanged;
    ///   - `Some((pvar, restrictor))` → `∃pvar(restrictor ∧ relation)`.
    pub(super) fn wrap_in_possessor_entity(
        &mut self,
        restr: Option<(Symbol, &'a LogicExpr<'a>)>,
        relation: &'a LogicExpr<'a>,
    ) -> &'a LogicExpr<'a> {
        match restr {
            None => relation,
            Some((pvar, restrictor)) => {
                let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: restrictor,
                    op: TokenType::And,
                    right: relation,
                });
                self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: QuantifierKind::Existential,
                    variable: pvar,
                    body,
                    island_id: self.current_island,
                })
            }
        }
    }

    /// Lower a possessor noun phrase to its logical contribution, predicating it of
    /// `possessed_term`. THE single place a genitive becomes logic, so a possessor's
    /// adjectives / nested genitive / PPs survive in EVERY syntactic position rather
    /// than each construction re-deriving (and dropping) them:
    ///   - a **descriptive** possessor ("the old captain", "X's B") → its own
    ///     existential entity carrying its full restrictor, recursively lowered:
    ///     `∃p(Restrictor(p) ∧ Possesses(p, possessed))`;
    ///   - a **bare** proper name ("Agnes") → the referring constant:
    ///     `Possesses(Agnes, possessed)`.
    pub(super) fn possessor_predication(
        &mut self,
        possessor: &NounPhrase<'a>,
        possessed_term: Term<'a>,
    ) -> &'a LogicExpr<'a> {
        let (poss_term, restr) = self.possessor_entity(possessor);
        let possesses = self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: self.interner.intern("Possesses"),
            args: self.ctx.terms.alloc_slice([poss_term, possessed_term]),
            world: None,
        });
        self.wrap_in_possessor_entity(restr, possesses)
    }

    /// Like [`Self::nominal_predication`] but ALSO conjoins the NP's PPs / reduced
    /// relatives (the `_PP_SELF_` placeholder substituted to `subject_term`,
    /// whether a constant or a variable). Use at sites that predicate a full NP of
    /// a term and would otherwise drop "the medicine sourced from a fig" down to
    /// `Medicine(x)`. Does NOT double-count at sites that already loop the PPs.
    pub(super) fn nominal_predication_with_pps(
        &mut self,
        subject_term: Term<'a>,
        pred_np: &NounPhrase<'a>,
    ) -> &'a LogicExpr<'a> {
        let mut result = self.nominal_predication(subject_term, pred_np);
        for pp in pred_np.pps {
            // Recurse through the restrictor's structure so a COMPLEX pp — a reduced
            // relative built as a NeoEvent / quantified conjunction, not just a flat
            // Predicate — has its `_PP_SELF_` gap bound and is NOT silently dropped.
            let pp_sub = self.substitute_pp_self_term(pp, subject_term);
            result = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: result,
                op: TokenType::And,
                right: pp_sub,
            });
        }
        result
    }

    /// THE single place a PP object's own modifiers are recovered: its ADJECTIVES
    /// ("the BIG table") and nested Predicate PPs ("a range OF 650 ft"), each
    /// predicated of the object constant (nested PPs' `_PP_SELF_` rebound to it).
    /// Every PP position (NP-internal, copula, passive-agent, modal) routes through
    /// this so "on the BIG table" / "by the LOCAL police" never drop the modifier.
    pub(super) fn pp_object_modifier_preds(
        &mut self,
        pp_object: &NounPhrase<'a>,
    ) -> Vec<&'a LogicExpr<'a>> {
        let mut out: Vec<&'a LogicExpr<'a>> = Vec::new();
        for &adj in pp_object.adjectives {
            out.push(self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: adj,
                args: self.ctx.terms.alloc_slice([Term::Constant(pp_object.noun)]),
                world: None,
            }));
        }
        let placeholder = self.interner.intern("_PP_SELF_");
        for nested in pp_object.pps {
            if let LogicExpr::Predicate { name, args, world } = nested {
                let new_args: Vec<Term<'a>> = args
                    .iter()
                    .map(|a| match a {
                        Term::Variable(v) if *v == placeholder => Term::Constant(pp_object.noun),
                        other => *other,
                    })
                    .collect();
                out.push(self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: *name,
                    args: self.ctx.terms.alloc_slice(new_args),
                    world: *world,
                }));
            }
        }
        out
    }

    /// Conjoin a PP object's recovered modifiers (see [`Self::pp_object_modifier_preds`])
    /// onto `pred` — for PP sites that build a single conjoined relation (copula,
    /// passive-agent, modal) rather than a `pps` list.
    pub(super) fn attach_pp_object_modifiers(
        &mut self,
        mut pred: &'a LogicExpr<'a>,
        pp_object: &NounPhrase<'a>,
    ) -> &'a LogicExpr<'a> {
        for m in self.pp_object_modifier_preds(pp_object) {
            pred = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: pred,
                op: TokenType::And,
                right: m,
            });
        }
        pred
    }

    /// Consume trailing EVENT PP-adjuncts ("takes a holiday WITH a friend TO some
    /// location") and conjoin each as `Prep(event, obj)` onto `event`. The
    /// non-quantified object path consumes these inline in its main PP loop; the
    /// QUANTIFIED-object path wraps the event in `∃`, so without this the PPs strand
    /// as trailing tokens. A bare preposition with no object is handed back for the
    /// sentence-level parse to report rather than silently dropped.
    pub(super) fn attach_trailing_event_pps(
        &mut self,
        mut event: &'a LogicExpr<'a>,
        event_var: Symbol,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        while self.check_preposition() || self.check_to() {
            // "within N <unit>" is a temporal bound, not a PP.
            if self.check_preposition_is("within")
                && matches!(
                    self.tokens.get(self.current + 1).map(|t| &t.kind),
                    Some(TokenType::Cardinal(_)) | Some(TokenType::Number(_))
                )
            {
                break;
            }
            // An NP object must follow; otherwise leave the preposition in place.
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
                    | Some(TokenType::Pronoun { .. })
            );
            if !object_follows {
                break;
            }
            let prep_token = self.advance().clone();
            let prep_name = match prep_token.kind {
                TokenType::Preposition(sym) => sym,
                TokenType::To => self.interner.intern("To"),
                _ => break,
            };
            // A quantified PP object ("to SOME new location", "with ANY friend")
            // leads with a bare quantifier token that parse_noun_phrase does not
            // consume; drop it so the head noun reads as the object referent. The
            // PP object is represented by its noun constant, matching the indefinite
            // ("with a friend" → With(e, Friend)) convention.
            if matches!(
                self.peek().kind,
                TokenType::Some | TokenType::Any | TokenType::All | TokenType::No
            ) {
                self.advance();
            }
            let pp_np = self.parse_noun_phrase(false)?;
            let pp_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: prep_name,
                args: self
                    .ctx
                    .terms
                    .alloc_slice([Term::Variable(event_var), Term::Constant(pp_np.noun)]),
                world: None,
            });
            event = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: event,
                op: TokenType::And,
                right: pp_pred,
            });
        }
        Ok(event)
    }

    /// Substitute the `_PP_SELF_` placeholder with `term` (constant OR variable)
    /// throughout a restrictor, recursing through connectives / quantifiers / events.
    /// Generalizes [`QuantifierParsing::substitute_pp_placeholder`] (which targets a
    /// variable) so a reduced-relative restrictor binds its gap wherever it sits.
    pub(super) fn substitute_pp_self_term(
        &mut self,
        pp: &'a LogicExpr<'a>,
        term: Term<'a>,
    ) -> &'a LogicExpr<'a> {
        let placeholder = self.interner.intern("_PP_SELF_");
        match pp {
            LogicExpr::Predicate { name, args, .. } => {
                let new_args: Vec<Term<'a>> = args
                    .iter()
                    .map(|a| match a {
                        Term::Variable(v) if *v == placeholder => term,
                        other => *other,
                    })
                    .collect();
                self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: *name,
                    args: self.ctx.terms.alloc_slice(new_args),
                    world: None,
                })
            }
            LogicExpr::BinaryOp { left, op, right } => {
                let l = self.substitute_pp_self_term(left, term);
                let r = self.substitute_pp_self_term(right, term);
                self.ctx.exprs.alloc(LogicExpr::BinaryOp { left: l, op: op.clone(), right: r })
            }
            LogicExpr::UnaryOp { op, operand } => {
                let o = self.substitute_pp_self_term(operand, term);
                self.ctx.exprs.alloc(LogicExpr::UnaryOp { op: op.clone(), operand: o })
            }
            LogicExpr::Quantifier { kind, variable, body, island_id } => {
                let b = self.substitute_pp_self_term(body, term);
                self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: *kind,
                    variable: *variable,
                    body: b,
                    island_id: *island_id,
                })
            }
            LogicExpr::Temporal { operator, body } => {
                let b = self.substitute_pp_self_term(body, term);
                self.ctx.exprs.alloc(LogicExpr::Temporal { operator: *operator, body: b })
            }
            LogicExpr::NeoEvent(data) => {
                let new_roles: Vec<(ThematicRole, Term<'a>)> = data
                    .roles
                    .iter()
                    .map(|(role, t)| {
                        let nt = match t {
                            Term::Variable(v) if *v == placeholder => term,
                            other => *other,
                        };
                        (*role, nt)
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

    /// If a relative clause ("WHO played", "THAT won 9 games") trails a predicate
    /// nominal, conjoin it onto `pred`, predicated of the subject — "X is the
    /// player who played" entails X played. The gap binds to the subject term
    /// (constant or variable). A no-op when no relative clause follows. Shared by
    /// every copula-complement site (plain `is the Y`, `either A or B`,
    /// neither/nor, quantified subjects) so the feature composes once.
    /// If a relativizer follows, parse the relative clause over `term` and return it.
    /// One place so who/that (argument gap), where (locative), and whose (possessive)
    /// compose UNIFORMLY at every NP attachment site (subject, predicate nominal,
    /// either-or / of-pair member, comparison/temporal standard) instead of each site
    /// re-checking who/that inline and silently stranding where/whose. The where/whose
    /// helpers consume their own relativizer; parse_relative_clause expects who/that
    /// already consumed (matching its existing callers).
    pub(super) fn try_attach_relative(
        &mut self,
        term: Term<'a>,
    ) -> ParseResult<Option<&'a LogicExpr<'a>>> {
        if let Term::Constant(s) | Term::Variable(s) = term {
            if self.check(&TokenType::Who) || self.check(&TokenType::That) {
                self.advance();
                return Ok(Some(self.parse_relative_clause(s)?));
            } else if self.check(&TokenType::Where) {
                return Ok(Some(self.parse_where_relative(s)?));
            } else if self.check(&TokenType::Whose) {
                return Ok(Some(self.parse_whose_relative(s)?));
            }
        }
        Ok(None)
    }

    pub(super) fn conjoin_trailing_relative(
        &mut self,
        pred: &'a LogicExpr<'a>,
        subject_term: Term<'a>,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        let mut pred = pred;
        // who/that/where/whose relative on the complement.
        if let Some(rc) = self.try_attach_relative(subject_term.clone())? {
            pred = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: pred,
                op: TokenType::And,
                right: rc,
            });
        }
        // A REDUCED relative ("is the mountain FIRST CLIMBED in 1845", "is the island
        // SEEN by Captain Norris") — every caller is a copula complement / predicate
        // nominal / disjunct, where the copula is already consumed, so a trailing
        // participle is a reduced relative restricting the subject, never a matrix verb.
        if let Some(rr) = self.try_consume_reduced_relative(subject_term)? {
            pred = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: pred,
                op: TokenType::And,
                right: rr,
            });
        }
        Ok(pred)
    }

    /// Conjoin a temporal adverb consumed after a relative-clause copula ("who is NOW
    /// with the Tigers") over the gap, or return the predication unchanged when there
    /// was no adverb. Shared by the progressive and the predicate-complement exits of
    /// the copular relative branch.
    pub(super) fn conjoin_relative_temporal_adverb(
        &mut self,
        pred: &'a LogicExpr<'a>,
        gap_var: Symbol,
        adv: Option<Symbol>,
    ) -> &'a LogicExpr<'a> {
        match adv {
            Some(a) => {
                let adv_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: a,
                    args: self.ctx.terms.alloc_slice([Term::Variable(gap_var)]),
                    world: None,
                });
                self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: pred,
                    op: TokenType::And,
                    right: adv_pred,
                })
            }
            None => pred,
        }
    }

    /// Copula complement led by a temporal/ordinal adverb: "was FIRST" (ranked
    /// first), "is NOW the leader", "was THEN the champion". Returns the base
    /// predication over `subject_term` — the adverb conjoined with any following
    /// predicate nominal/adjective ("is now THE LEADER" → Leader(x) ∧ Now(x)), or the
    /// bare adverb when it is the whole complement ("was first" → First(x)). Returns
    /// None when the next token is not a temporal adverb, so the caller falls through
    /// to its other complement branches. The caller applies its own tense / negation /
    /// definiteness wrapping. Restricted to TemporalAdverb so a degree adverb ("is
    /// very tall") is not mis-attached to the subject. Shared by parse_atom and
    /// parse_predicate so both copula paths gain the reading from one place.
    pub(super) fn copula_temporal_adverb_complement(
        &mut self,
        subject_term: Term<'a>,
    ) -> ParseResult<Option<&'a LogicExpr<'a>>> {
        let adv = match self.peek().kind {
            TokenType::TemporalAdverb(s) => s,
            _ => return Ok(None),
        };
        self.advance();
        let adv_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: adv,
            args: self.ctx.terms.alloc_slice([subject_term.clone()]),
            world: None,
        });
        if self.check_article() || self.check_content_word() {
            let saved_ctx = self.nominal_np_context;
            self.nominal_np_context = true;
            let comp_np_result = self.parse_noun_phrase(true);
            self.nominal_np_context = saved_ctx;
            let comp_np = comp_np_result?;
            let comp_pred = self.nominal_predication_with_pps(subject_term.clone(), &comp_np);
            let comp_pred = self.conjoin_trailing_relative(comp_pred, subject_term.clone())?;
            return Ok(Some(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: comp_pred,
                op: TokenType::And,
                right: adv_pred,
            })));
        }
        Ok(Some(adv_pred))
    }

    /// Conjoin an NP's restrictions — possessor (`Possesses(possessor, entity)`)
    /// and PPs (with the `_PP_SELF_` placeholder substituted to the entity) —
    /// onto a predication. Used wherever an NP is reduced to its head symbol
    /// (comparative standards, disjuncts, list members) so the possessor / PP
    /// constraints are not silently dropped — zero meaning loss.
    pub(super) fn augment_with_np_restrictions(
        &mut self,
        expr: &'a LogicExpr<'a>,
        np: &NounPhrase<'a>,
    ) -> &'a LogicExpr<'a> {
        let entity = Term::Constant(np.noun);
        let mut result = expr;
        if let Some(possessor) = np.possessor {
            let possesses = self.interner.intern("Possesses");
            let poss = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: possesses,
                args: self
                    .ctx
                    .terms
                    .alloc_slice([Term::Constant(possessor.noun), entity]),
                world: None,
            });
            result = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: result,
                op: TokenType::And,
                right: poss,
            });
        }
        if !np.pps.is_empty() {
            let placeholder = self.interner.intern("_PP_SELF_");
            for pp in np.pps {
                let pp_sub = match pp {
                    LogicExpr::Predicate { name, args, world } => {
                        let new_args: Vec<Term<'a>> = args
                            .iter()
                            .map(|a| match a {
                                Term::Variable(v) if *v == placeholder => entity,
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
                result = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: result,
                    op: TokenType::And,
                    right: pp_sub,
                });
            }
        }
        result
    }

    /// Apply the copula's tense, negation, and temporal-adverb wrappers to a
    /// finished predication body, in the same order the bare-predicate path uses.
    pub(super) fn finish_copula(
        &self,
        base: &'a LogicExpr<'a>,
        copula_time: Time,
        is_negated: bool,
        copula_temporal: Option<super::CopulaTemporal>,
    ) -> &'a LogicExpr<'a> {
        let with_time = if copula_time == Time::Past {
            self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: TemporalOperator::Past,
                body: base,
            })
        } else {
            base
        };
        let with_neg = if is_negated {
            self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: with_time,
            })
        } else {
            with_time
        };
        match copula_temporal {
            Some(super::CopulaTemporal::Always) => self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: TemporalOperator::Always,
                body: with_neg,
            }),
            Some(super::CopulaTemporal::Never) => {
                let negated = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                    op: TokenType::Not,
                    operand: with_time,
                });
                self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: TemporalOperator::Always,
                    body: negated,
                })
            }
            Some(super::CopulaTemporal::Eventually) => self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: TemporalOperator::Eventually,
                body: with_neg,
            }),
            None => with_neg,
        }
    }

    /// Arithmetic / vague verbal comparative after a just-consumed verb.
    ///
    /// Matches `[MEASURE] [DEGREE] COMPARATIVE "than" STANDARD`, where the verb
    /// names the graded dimension (its event is asserted), a measure phrase
    /// ("3 points") is the exact offset, and a degree modifier ("somewhat")
    /// marks a strict-but-imprecise inequality. The bare "faster than Bob" case
    /// (no measure, no degree modifier) is left for the caller. Returns `None`
    /// without consuming when the pattern is absent.
    pub(super) fn try_arithmetic_comparative(
        &mut self,
        verb: Symbol,
        subject_term: Term<'a>,
        verb_time: Time,
    ) -> ParseResult<Option<&'a LogicExpr<'a>>> {
        let is_unit = |k: &TokenType| {
            matches!(
                k,
                TokenType::Noun(_)
                    | TokenType::Adjective(_)
                    | TokenType::NonIntersectiveAdjective(_)
                    | TokenType::Verb { .. }
                    | TokenType::ProperName(_)
                    | TokenType::Performative(_)
                    | TokenType::Ambiguous { .. }
                    // Measure-unit tokens ("10 more MINUTES than", "3 ounces more
                    // gold than") — a calendar/clock unit or duration literal heads
                    // the comparison dimension just like a plain unit noun.
                    | TokenType::CalendarUnit(_)
                    | TokenType::DurationLiteral { .. }
            )
        };
        let cp = self.checkpoint();

        // 0. A price/measure comparative may be introduced by "for"/"at"/"with"
        // ("sold for $25,000 less than Y", "sold for somewhat less than Y",
        // "finished with 3 ounces more gold than Y"). Tentatively drop it; the
        // checkpoint restores it if no comparative follows, so a plain "sold for
        // $105" / "finished with a medal" still flows to the PP handler.
        if matches!(self.peek().kind, TokenType::Preposition(s)
            if matches!(self.interner.resolve(s).to_lowercase().as_str(), "for" | "at" | "with"))
        {
            self.advance();
        }

        // 0b. A possessed-quality comparative is introduced by an indefinite
        // article ("has a narrower wingspan than Y", "has a 4 inches narrower
        // wingspan than Y"). Tentatively drop it; the checkpoint restores it if no
        // comparative follows, so a plain "has a wingspan" still flows onward.
        if matches!(self.peek().kind, TokenType::Article(crate::lexicon::Definiteness::Indefinite)) {
            self.advance();
        }

        // 1. Optional numeric offset ("3", "190").
        let count_kind: Option<crate::ast::NumberKind> = match self.peek().kind {
            TokenType::Number(s) => {
                self.advance();
                let raw = self.interner.resolve(s);
                Some(if let Ok(n) = raw.parse::<i64>() {
                    crate::ast::NumberKind::Integer(n)
                } else if raw.contains('.') {
                    crate::ast::NumberKind::Real(raw.parse().unwrap_or(0.0))
                } else {
                    crate::ast::NumberKind::Symbolic(s)
                })
            }
            TokenType::Cardinal(n) => {
                self.advance();
                Some(crate::ast::NumberKind::Integer(n as i64))
            }
            _ => None,
        };

        // 2. Optional unit noun BEFORE the comparative ("3 points lower").
        let mut unit_sym: Option<Symbol> = None;
        if count_kind.is_some()
            && is_unit(&self.peek().kind)
            && matches!(
                self.tokens.get(self.current + 1).map(|t| &t.kind),
                Some(TokenType::Comparative(_))
            )
        {
            unit_sym = Some(self.consume_content_word()?);
        }

        // 3. A dimension noun and/or degree modifier between here and the comparative.
        // A NOUN ("a wingspan LONGER than", "a face WIDER than") is the comparison
        // DIMENSION → unit_sym, so the measure is Wingspan(x)/Wingspan(y) and not the
        // verb (the prenominal form "a LONGER wingspan than" puts the comparative first
        // and is captured at step 5). A non-noun degree word ("somewhat", "much") is
        // discarded. Both may appear ("a wingspan SOMEWHAT longer than"). Only the
        // count-less case — the numeric "4 inches narrower" path is handled at step 2.
        let mut has_vague = false;
        if unit_sym.is_none() && count_kind.is_none() {
            // Pure lookahead (no checkpoint side effects): scan dimension noun(s), an
            // optional degree adverb, and require a Comparative to follow. Only then
            // consume — so a bare object ("won her prize") with no comparative is
            // untouched.
            let mut k = self.current;
            while self.tokens.get(k).map_or(false, |t| {
                matches!(t.kind, TokenType::Noun(_) | TokenType::Ambiguous { .. })
                    && !crate::lexicon::is_degree_adverb(
                        &self.interner.resolve(t.lexeme).to_lowercase(),
                    )
            }) {
                k += 1;
            }
            let dim_end = k;
            let has_dim = dim_end > self.current;
            let degree = !matches!(
                self.tokens.get(k).map(|t| &t.kind),
                Some(TokenType::Comparative(_))
            ) && matches!(
                self.tokens.get(k + 1).map(|t| &t.kind),
                Some(TokenType::Comparative(_))
            );
            let comp_at = if degree { k + 1 } else { k };
            let comp_here = matches!(
                self.tokens.get(comp_at).map(|t| &t.kind),
                Some(TokenType::Comparative(_))
            );
            if has_dim && comp_here {
                let mut dim: Option<Symbol> = None;
                while self.current < dim_end {
                    let n = self.consume_content_word()?;
                    dim = Some(match dim {
                        Some(d) => self.interner.intern(&format!(
                            "{}_{}",
                            self.interner.resolve(d),
                            self.interner.resolve(n)
                        )),
                        None => n,
                    });
                }
                if degree {
                    self.advance(); // degree adverb (discarded)
                    has_vague = true;
                }
                unit_sym = dim;
            }
        }
        // 3b. A bare degree modifier with no dimension noun ("somewhat higher").
        if unit_sym.is_none()
            && !matches!(self.peek().kind, TokenType::Comparative(_))
            && matches!(
                self.tokens.get(self.current + 1).map(|t| &t.kind),
                Some(TokenType::Comparative(_))
            )
        {
            self.advance(); // degree modifier
            has_vague = true;
        }

        // 4. The comparative itself.
        let comp_adj = match self.peek().kind {
            TokenType::Comparative(a) => {
                self.advance();
                a
            }
            _ => {
                self.restore(cp);
                return Ok(None);
            }
        };

        // 5. A unit / dimension noun phrase AFTER the comparative, immediately before
        // "than" ("1 more game than", "less baking time than", "received 7 votes more
        // votes than Ken"). Consume the WHOLE (possibly multi-word) dimension up to
        // "than"; join multi-word into one measure symbol ("baking time" → Baking_time)
        // so the prover relates the same dimension. A unit already captured before the
        // comparative makes a post-comparative noun redundant — consumed but discarded.
        {
            let mut k = self.current;
            while self.tokens.get(k).map_or(false, |t| is_unit(&t.kind)) {
                k += 1;
            }
            let unit_end = k;
            // An infinitive purpose modifier on the dimension ("10 more minutes TO
            // PRINT than") specifies what the measured amount is FOR; fold the verb
            // into the dimension so the prover relates the same measure on both sides
            // ("minutes to print" → Minute_print).
            let infinitive: Option<Symbol> = if unit_end > self.current
                && matches!(self.tokens.get(unit_end).map(|t| &t.kind), Some(TokenType::To))
            {
                match self.tokens.get(unit_end + 1).map(|t| t.kind.clone()) {
                    Some(TokenType::Verb { lemma, .. }) => Some(lemma),
                    _ => None,
                }
            } else {
                None
            };
            let after = if infinitive.is_some() { unit_end + 2 } else { unit_end };
            if unit_end > self.current
                && matches!(self.tokens.get(after).map(|t| &t.kind), Some(TokenType::Than))
            {
                let mut dim = self.consume_content_word()?;
                while self.current < unit_end {
                    let next = self.consume_content_word()?;
                    dim = self.interner.intern(&format!(
                        "{}_{}",
                        self.interner.resolve(dim),
                        self.interner.resolve(next)
                    ));
                }
                if let Some(vlemma) = infinitive {
                    self.advance(); // "to"
                    self.advance(); // the infinitive verb
                    dim = self.interner.intern(&format!(
                        "{}_{}",
                        self.interner.resolve(dim),
                        self.interner.resolve(vlemma)
                    ));
                }
                if unit_sym.is_none() {
                    unit_sym = Some(dim);
                }
            }
        }

        // 5b. A per-unit RATE between the comparative and "than" ("less per gallon
        // than", "5 dollars less per pound than", "10 dollars less per month than")
        // — the comparison is on a RATE, not a raw amount. The basis is folded into
        // the measure name (Charge → Charge_per_Gallon) so the prover relates
        // per-gallon prices; the count's noun ("dollars") becomes the offset unit.
        let mut rate_unit: Option<Symbol> = None;
        if matches!(self.peek().kind, TokenType::Preposition(s)
            if self.interner.resolve(s).eq_ignore_ascii_case("per"))
        {
            let cp_rate = self.checkpoint();
            self.advance(); // "per"
            if is_unit(&self.peek().kind) {
                rate_unit = Some(self.consume_content_word()?);
            } else {
                self.restore(cp_rate);
            }
        }

        // 6. "than" must follow. A measure prefix (count or degree) makes this an
        // exact/vague arithmetic comparative. A BARE comparative ("cost less than
        // the potatoes") is also a genuine comparison — handled here for a
        // solver-ready `Less`/`Greater` over a DISTINCT standard entity — EXCEPT
        // when the standard is a number ("ate more than 5 apples" is a quantity,
        // not an entity comparison), which is left for the quantity path.
        if !matches!(self.peek().kind, TokenType::Than) {
            self.restore(cp);
            return Ok(None);
        }
        if count_kind.is_none() && !has_vague {
            // A QUANTIFIED standard ("more than 5 apples" = quantity; "faster than
            // all cats" = a universally-quantified comparison) is not a bare
            // entity comparison — leave it for the quantity / verbal-comparative
            // quantifier paths.
            let standard_quantified = matches!(
                self.tokens.get(self.current + 1).map(|t| &t.kind),
                Some(TokenType::Number(_))
                    | Some(TokenType::Cardinal(_))
                    | Some(TokenType::All)
                    | Some(TokenType::No)
                    | Some(TokenType::Some)
                    | Some(TokenType::Any)
                    | Some(TokenType::Most)
                    | Some(TokenType::Few)
                    | Some(TokenType::Many)
                    | Some(TokenType::AtLeast(_))
                    | Some(TokenType::AtMost(_))
            );
            if standard_quantified {
                self.restore(cp);
                return Ok(None);
            }
        }
        self.advance(); // "than"

        let event_var = self.get_event_var();
        let mut modifiers = Vec::new();
        let effective_time = self.pending_time.take().unwrap_or(verb_time);
        match effective_time {
            Time::Past => modifiers.push(self.interner.intern("Past")),
            Time::Future => modifiers.push(self.interner.intern("Future")),
            _ => {}
        }
        let suppress_existential = self.drs.in_conditional_antecedent();
        let event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
            event_var,
            verb,
            roles: self
                .ctx
                .roles
                .alloc_slice(vec![(ThematicRole::Agent, subject_term)]),
            modifiers: self.ctx.syms.alloc_slice(modifiers),
            suppress_existential,
            world: None,
        })));

        // Solver-ready arithmetic: a measure function over the entity, an
        // arithmetic offset (add/sub — the names the LIA oracle recognises), and
        // an equality (exact) or strict inequality (vague). The measure is named
        // by the unit ("points" → Points) or, lacking one, by the verb ("scored"
        // → Score) — stable across both sides so the prover can relate them.
        let cap = |s: &str| -> String {
            let mut c = s.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        };
        let measure_name = match (rate_unit, unit_sym) {
            // A rate names the measure by the VERB per the rate unit; the count's
            // noun ("dollars") is the offset's unit, not the measure name.
            (Some(r), _) => format!(
                "{}_per_{}",
                cap(&self.interner.resolve(verb).to_string()),
                cap(&self.interner.resolve(r).to_string())
            ),
            (None, Some(u)) => cap(&self.interner.resolve(u).to_string()),
            (None, None) => cap(&self.interner.resolve(verb).to_string()),
        };
        let measure_sym = self.interner.intern(&measure_name);
        // `comp_adj` is the base adjective lemma (e.g. "narrow", "short"); its scale
        // polarity decides the direction. Negative-pole adjectives subtract.
        let comp_str = self.interner.resolve(comp_adj).to_lowercase();
        let subtract = crate::lexicon::is_decreasing_adjective(&comp_str);
        let op_sym = self.interner.intern(if subtract { "sub" } else { "add" });
        let dir_sym = self.interner.intern(if subtract { "Less" } else { "Greater" });
        // For a rate ("5 dollars less per gallon"), the count's noun is the
        // offset's currency unit; otherwise the count is implicitly in the measure.
        let offset_unit = if rate_unit.is_some() { unit_sym } else { None };
        let offset_term: Option<Term<'a>> = count_kind.map(|kind| Term::Value {
            kind,
            unit: offset_unit,
            dimension: None,
        });
        let measure_x = Term::Function(measure_sym, self.ctx.terms.alloc_slice([subject_term]));
        let build_constraint = move |p: &mut Self, y_term: Term<'a>| -> &'a LogicExpr<'a> {
            let measure_y = Term::Function(measure_sym, p.ctx.terms.alloc_slice([y_term]));
            match offset_term {
                Some(off) => {
                    let rhs =
                        Term::Function(op_sym, p.ctx.terms.alloc_slice([measure_y, off]));
                    p.ctx.exprs.alloc(LogicExpr::Identity {
                        left: p.ctx.terms.alloc(measure_x),
                        right: p.ctx.terms.alloc(rhs),
                    })
                }
                None => p.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: dir_sym,
                    args: p.ctx.terms.alloc_slice([measure_x, measure_y]),
                    world: None,
                }),
            }
        };

        // The standard of comparison. A DESCRIPTION (determiner / adjective /
        // possessor / PP / relative clause) becomes its own existentially
        // quantified entity carrying a restrictor — so "less than Quinn Quade's
        // stamp" does NOT collapse onto the subject's "Stamp" constant (which
        // would make `Less(Sell(Stamp), Sell(Stamp))` — the subject compared to
        // itself — and misattach the possessor to the subject). A bare proper
        // name stays a referring constant. Greedy parse so a PP standard ("than
        // the perfume from Spain") attaches its PP. A comparative standard is a
        // nominal position — "than" rules out the matrix verb — so a verb-word
        // head in it is a deverbal noun ("than the orange PACK", "the investing
        // SHOW").
        let saved_ctx = self.nominal_np_context;
        self.nominal_np_context = true;
        let std_np_result = self.parse_noun_phrase(true);
        self.nominal_np_context = saved_ctx;
        let std_np = std_np_result?;
        let has_rel = self.check(&TokenType::Who) || self.check(&TokenType::That);
        let is_desc = has_rel
            || std_np.definiteness.is_some()
            || !std_np.adjectives.is_empty()
            || std_np.possessor.is_some()
            || !std_np.pps.is_empty();
        let result = if is_desc {
            let std_var = self.next_var_name();
            // Head noun + adjectives + possessor over the standard's variable.
            let mut restr = self.nominal_predication(Term::Variable(std_var), &std_np);
            for pp in std_np.pps {
                let pp_sub = self.substitute_pp_placeholder(pp, std_var);
                restr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: restr,
                    op: TokenType::And,
                    right: pp_sub,
                });
            }
            if has_rel {
                self.advance(); // "who" / "that"
                let rel = self.parse_relative_clause(std_var)?;
                restr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: restr,
                    op: TokenType::And,
                    right: rel,
                });
            }
            let comparison = build_constraint(self, Term::Variable(std_var));
            let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: restr,
                op: TokenType::And,
                right: comparison,
            });
            let quantified = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind: QuantifierKind::Existential,
                variable: std_var,
                body,
                island_id: self.current_island,
            });
            self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: event,
                op: TokenType::And,
                right: quantified,
            })
        } else {
            let comparison = build_constraint(self, Term::Constant(std_np.noun));
            self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: event,
                op: TokenType::And,
                right: comparison,
            })
        };
        Ok(Some(result))
    }

    /// Temporal offset after a just-consumed verb: `COUNT CALENDAR-UNIT
    /// (after|before) STANDARD` ("performed 2 weeks after Bessie", "will launch
    /// 2 months before the graduate who will be studying radiation"). The verb
    /// event is asserted and a directed offset relates the subject to the
    /// standard. Returns `None` without consuming when the pattern is absent.
    /// Bare temporal ordering "(sometime) before/after STANDARD" relating
    /// `subject_term` DIRECTLY to a distinct standard entity by time — used by the
    /// PASSIVE path ("the photo was taken sometime before the photo of the red
    /// panda", "was bought sometime before Faye's pet"), which has no NeoEvent var.
    /// Returns `Before/After(subject_term, std)` with the standard a distinct
    /// ∃-entity (descriptive) or a constant (bare name). `None` (no consumption) if
    /// the pattern is absent. A leading vague adverb ("sometime") is skipped.
    pub(super) fn parse_bare_temporal_constraint(
        &mut self,
        subject_term: Term<'a>,
    ) -> ParseResult<Option<&'a LogicExpr<'a>>> {
        let j = self.current;
        let std_at = |k: Option<&TokenType>| {
            matches!(
                k,
                Some(TokenType::Article(_))
                    | Some(TokenType::Noun(_))
                    | Some(TokenType::ProperName(_))
            )
        };
        let lead_adverb = match self.tokens.get(j).map(|t| &t.kind) {
            Some(TokenType::Adverb(_)) => true,
            Some(_) => matches!(
                self.interner.resolve(self.tokens[j].lexeme).to_lowercase().as_str(),
                "sometime" | "shortly" | "soon" | "immediately" | "long" | "right" | "just"
            ),
            None => false,
        };
        let dj = if lead_adverb { j + 1 } else { j };
        let next_kind = self.tokens.get(dj + 1).map(|t| &t.kind);
        // A YEAR or clock time is also a valid temporal reference ("won a prize BEFORE
        // 1989", "started AFTER 2010") — the prover orders the value against other
        // years/times. Accept a numeric/time-literal object alongside the NP object.
        let temporal_at = |k: Option<&TokenType>| {
            std_at(k)
                || matches!(
                    k,
                    Some(TokenType::Number(_)) | Some(TokenType::TimeLiteral { .. })
                )
        };
        let bare_dir = match self.tokens.get(dj).map(|t| &t.kind) {
            Some(TokenType::Before) if temporal_at(next_kind) => Some("Before"),
            Some(TokenType::Preposition(s))
                if self.interner.resolve(*s).eq_ignore_ascii_case("before")
                    && temporal_at(next_kind) =>
            {
                Some("Before")
            }
            Some(TokenType::Preposition(s))
                if self.interner.resolve(*s).eq_ignore_ascii_case("after")
                    && temporal_at(next_kind) =>
            {
                Some("After")
            }
            _ => None,
        };
        let dir = match bare_dir {
            Some(d) => d,
            None => return Ok(None),
        };
        if lead_adverb {
            self.advance();
        }
        self.advance(); // "before" / "after"
        let rel_sym = self.interner.intern(dir);
        // Numeric temporal reference ("before 1989", "after 2010", "before 9:30am"):
        // the year/clock-time is a value, so relate the subject to it directly.
        if matches!(
            self.peek().kind,
            TokenType::Number(_) | TokenType::TimeLiteral { .. }
        ) {
            let year = self.parse_measure_phrase()?;
            return Ok(Some(self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: rel_sym,
                args: self.ctx.terms.alloc_slice([subject_term, *year]),
                world: None,
            })));
        }
        let std_np = self.parse_noun_phrase(true)?;
        let has_rel = self.check(&TokenType::Who) || self.check(&TokenType::That);
        let is_desc = has_rel
            || std_np.definiteness.is_some()
            || !std_np.adjectives.is_empty()
            || std_np.possessor.is_some()
            || !std_np.pps.is_empty();
        let result = if is_desc {
            let v = self.next_var_name();
            let mut restr = self.nominal_predication(Term::Variable(v), &std_np);
            for pp in std_np.pps {
                let pp_sub = self.substitute_pp_placeholder(pp, v);
                restr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: restr,
                    op: TokenType::And,
                    right: pp_sub,
                });
            }
            if has_rel {
                self.advance();
                let rel = self.parse_relative_clause(v)?;
                restr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: restr,
                    op: TokenType::And,
                    right: rel,
                });
            }
            let rel = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: rel_sym,
                args: self.ctx.terms.alloc_slice([subject_term, Term::Variable(v)]),
                world: None,
            });
            let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: restr,
                op: TokenType::And,
                right: rel,
            });
            self.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind: QuantifierKind::Existential,
                variable: v,
                body,
                island_id: self.current_island,
            })
        } else {
            self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: rel_sym,
                args: self.ctx.terms.alloc_slice([subject_term, Term::Constant(std_np.noun)]),
                world: None,
            })
        };
        Ok(Some(result))
    }

    pub(super) fn try_temporal_offset(
        &mut self,
        verb: Symbol,
        subject_term: Term<'a>,
        verb_time: Time,
    ) -> ParseResult<Option<&'a LogicExpr<'a>>> {
        let j = self.current;

        // Bare temporal ordering (no count/unit): "<verb> before STANDARD" or
        // "<verb> after THE/A STANDARD" — relate the subject's event to a
        // DISTINCT standard entity. "after <proper name>" is intentionally left
        // to the PP-adjunct path (which already yields After(e, Name)); only the
        // gaps ("before …", "after the/a …") are filled here.
        let std_at = |k: Option<&TokenType>| {
            matches!(
                k,
                Some(TokenType::Article(_))
                    | Some(TokenType::Noun(_))
                    | Some(TokenType::ProperName(_))
            )
        };
        // A vague temporal adverb may precede the direction ("starts SOMETIME
        // after …", "arrived SHORTLY before …"); it only emphasises the absence
        // of a precise offset, which the bare relation already captures, so skip
        // it. `dj` indexes the after/before once it is skipped.
        let lead_adverb = match self.tokens.get(j).map(|t| &t.kind) {
            Some(TokenType::Adverb(_)) => true,
            Some(_) => matches!(
                self.interner.resolve(self.tokens[j].lexeme).to_lowercase().as_str(),
                "sometime" | "shortly" | "soon" | "immediately" | "long" | "right" | "just"
            ),
            None => false,
        };
        let dj = if lead_adverb { j + 1 } else { j };
        let next_kind = self.tokens.get(dj + 1).map(|t| &t.kind);
        // A YEAR or clock time is a temporal reference the prover can order ("happened
        // BEFORE 1989", "started AFTER 2010"). "after <name>" still defers to the
        // PP-adjunct path, but "after <year>" has no such path, so accept it here.
        let num_at = |k: Option<&TokenType>| {
            matches!(
                k,
                Some(TokenType::Number(_)) | Some(TokenType::TimeLiteral { .. })
            )
        };
        let bare_dir = match self.tokens.get(dj).map(|t| &t.kind) {
            Some(TokenType::Before) if std_at(next_kind) || num_at(next_kind) => Some("Before"),
            Some(TokenType::Preposition(s))
                if self.interner.resolve(*s).eq_ignore_ascii_case("before")
                    && (std_at(next_kind) || num_at(next_kind)) =>
            {
                Some("Before")
            }
            Some(TokenType::Preposition(s))
                if self.interner.resolve(*s).eq_ignore_ascii_case("after")
                    && (matches!(next_kind, Some(TokenType::Article(_))) || num_at(next_kind)) =>
            {
                Some("After")
            }
            _ => None,
        };
        if let Some(dir) = bare_dir {
            if lead_adverb {
                self.advance(); // skip the vague temporal adverb
            }
            self.advance(); // "before" / "after"
            let event_var = self.get_event_var();
            let mut modifiers = Vec::new();
            let effective_time = self.pending_time.take().unwrap_or(verb_time);
            match effective_time {
                Time::Past => modifiers.push(self.interner.intern("Past")),
                Time::Future => modifiers.push(self.interner.intern("Future")),
                _ => {}
            }
            let suppress_existential = self.drs.in_conditional_antecedent();
            let event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                event_var,
                verb,
                roles: self
                    .ctx
                    .roles
                    .alloc_slice(vec![(ThematicRole::Agent, subject_term)]),
                modifiers: self.ctx.syms.alloc_slice(modifiers),
                suppress_existential,
                world: None,
            })));
            let rel_sym = self.interner.intern(dir);
            // Numeric temporal reference ("happened before 1989", "started after
            // 2010"): relate the EVENT to the year/clock-time value directly.
            if matches!(
                self.peek().kind,
                TokenType::Number(_) | TokenType::TimeLiteral { .. }
            ) {
                let year = self.parse_measure_phrase()?;
                let rel = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: rel_sym,
                    args: self
                        .ctx
                        .terms
                        .alloc_slice([Term::Variable(event_var), *year]),
                    world: None,
                });
                return Ok(Some(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: event,
                    op: TokenType::And,
                    right: rel,
                })));
            }
            // A temporal standard ("after the skydiving TRIP") is a nominal
            // position — a verb-word head there is a deverbal noun.
            let saved_ctx = self.nominal_np_context;
            self.nominal_np_context = true;
            let std_np_result = self.parse_noun_phrase(true);
            self.nominal_np_context = saved_ctx;
            let std_np = std_np_result?;
            let is_desc = std_np.definiteness.is_some()
                || !std_np.adjectives.is_empty()
                || std_np.possessor.is_some()
                || !std_np.pps.is_empty();
            let result = if is_desc {
                let v = self.next_var_name();
                let mut restr = self.nominal_predication(Term::Variable(v), &std_np);
                for pp in std_np.pps {
                    let pp_sub = self.substitute_pp_placeholder(pp, v);
                    restr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: restr,
                        op: TokenType::And,
                        right: pp_sub,
                    });
                }
                // A relative clause on the standard ("before the winner WHO won in
                // chemistry", "after the person WHO took the cruise") restricts the
                // standard entity; without this it was stranded (TrailingTokens at
                // Who/That). Mirrors parse_bare_temporal_constraint and the offset path.
                if self.check(&TokenType::Who) || self.check(&TokenType::That) {
                    self.advance();
                    let rc = self.parse_relative_clause(v)?;
                    restr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: restr,
                        op: TokenType::And,
                        right: rc,
                    });
                }
                let rel = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: rel_sym,
                    args: self
                        .ctx
                        .terms
                        .alloc_slice([Term::Variable(event_var), Term::Variable(v)]),
                    world: None,
                });
                let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: restr,
                    op: TokenType::And,
                    right: rel,
                });
                let quant = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                    kind: QuantifierKind::Existential,
                    variable: v,
                    body,
                    island_id: self.current_island,
                });
                self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: event,
                    op: TokenType::And,
                    right: quant,
                })
            } else {
                let rel = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: rel_sym,
                    args: self
                        .ctx
                        .terms
                        .alloc_slice([Term::Variable(event_var), Term::Constant(std_np.noun)]),
                    world: None,
                });
                self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: event,
                    op: TokenType::And,
                    right: rel,
                })
            };
            return Ok(Some(result));
        }

        // "N <unit> after/before STANDARD" — a solver-ready measure offset. The
        // event is asserted and a constraint relates the two positions. The
        // constraint builder is shared with the PASSIVE path ("was taken 1 month
        // after Y"), which already has its own event, so it is factored out into
        // `parse_temporal_offset_constraint`.
        let has_count = matches!(
            self.tokens.get(j).map(|t| &t.kind),
            Some(TokenType::Number(_)) | Some(TokenType::Cardinal(_))
        );
        let has_unit = matches!(
            self.tokens.get(j + 1).map(|t| &t.kind),
            Some(TokenType::CalendarUnit(_))
        );
        let has_dir = matches!(self.tokens.get(j + 2).map(|t| &t.kind), Some(TokenType::Before))
            || matches!(self.tokens.get(j + 2).map(|t| &t.kind),
                Some(TokenType::Preposition(s))
                    if matches!(self.interner.resolve(*s).to_lowercase().as_str(), "after" | "before"));
        if !(has_count && has_unit && has_dir) {
            return Ok(None);
        }

        // Capture tense BEFORE parsing the standard, whose relative clause could
        // otherwise overwrite pending_time.
        let effective_time = self.pending_time.take().unwrap_or(verb_time);
        let constraint = match self.parse_temporal_offset_constraint(subject_term)? {
            Some(c) => c,
            None => return Ok(None),
        };
        let event_var = self.get_event_var();
        let mut modifiers = Vec::new();
        match effective_time {
            Time::Past => modifiers.push(self.interner.intern("Past")),
            Time::Future => modifiers.push(self.interner.intern("Future")),
            _ => {}
        }
        let suppress_existential = self.drs.in_conditional_antecedent();
        let event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
            event_var,
            verb,
            roles: self
                .ctx
                .roles
                .alloc_slice(vec![(ThematicRole::Agent, subject_term)]),
            modifiers: self.ctx.syms.alloc_slice(modifiers),
            suppress_existential,
            world: None,
        })));
        Ok(Some(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
            left: event,
            op: TokenType::And,
            right: constraint,
        })))
    }

    /// Parses a calendar-unit measure offset `N <unit> (after|before) STANDARD`
    /// positioned at the count, returning ONLY the solver-ready constraint
    /// `Unit(subject) = add|sub(Unit(STANDARD), N)` (after → add, before → sub).
    /// The caller supplies the event (active VP) or passive predicate it conjoins
    /// to. A descriptive standard becomes a distinct existential entity carrying
    /// its restrictor (head + adjectives + possessor + PPs + relative clause); a
    /// bare proper name stays a constant. `None` (no consumption) if the offset
    /// pattern is absent.
    pub(super) fn parse_temporal_offset_constraint(
        &mut self,
        subject_term: Term<'a>,
    ) -> ParseResult<Option<&'a LogicExpr<'a>>> {
        let j = self.current;
        let has_count = matches!(
            self.tokens.get(j).map(|t| &t.kind),
            Some(TokenType::Number(_)) | Some(TokenType::Cardinal(_))
        );
        let has_unit = matches!(
            self.tokens.get(j + 1).map(|t| &t.kind),
            Some(TokenType::CalendarUnit(_))
        );
        let direction = match self.tokens.get(j + 2).map(|t| &t.kind) {
            Some(TokenType::Before) => Some("Before"),
            Some(TokenType::Preposition(s)) => {
                match self.interner.resolve(*s).to_lowercase().as_str() {
                    "after" => Some("After"),
                    "before" => Some("Before"),
                    _ => None,
                }
            }
            _ => None,
        };
        if !(has_count && has_unit && direction.is_some()) {
            return Ok(None);
        }
        let direction = direction.unwrap();

        // Offset count (Number or Cardinal) and calendar unit.
        let count_kind = match self.advance().kind {
            TokenType::Number(s) => {
                let raw = self.interner.resolve(s);
                if let Ok(n) = raw.parse::<i64>() {
                    crate::ast::NumberKind::Integer(n)
                } else {
                    crate::ast::NumberKind::Symbolic(s)
                }
            }
            TokenType::Cardinal(n) => crate::ast::NumberKind::Integer(n as i64),
            _ => unreachable!("guarded by has_count"),
        };
        let unit_lexeme = self.peek().lexeme;
        self.advance(); // calendar unit
        self.advance(); // "after" / "before"

        let cap = |s: &str| -> String {
            let mut c = s.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        };
        let measure_sym = self.interner.intern(&cap(&self.interner.resolve(unit_lexeme).to_string()));
        let op_sym = self.interner.intern(if direction == "After" { "add" } else { "sub" });
        let offset_term = Term::Value {
            kind: count_kind,
            unit: None,
            dimension: None,
        };

        let measure_x = Term::Function(measure_sym, self.ctx.terms.alloc_slice([subject_term]));
        let build_constraint = move |p: &mut Self, y_term: Term<'a>| -> &'a LogicExpr<'a> {
            let measure_y = Term::Function(measure_sym, p.ctx.terms.alloc_slice([y_term]));
            let rhs = Term::Function(op_sym, p.ctx.terms.alloc_slice([measure_y, offset_term]));
            p.ctx.exprs.alloc(LogicExpr::Identity {
                left: p.ctx.terms.alloc(measure_x),
                right: p.ctx.terms.alloc(rhs),
            })
        };

        // Distinct-identity treatment: a descriptive standard ("2 weeks after
        // Quinn Quade's debut") becomes its own existential entity so it never
        // collapses onto the subject and its possessor/PP/relative clause bind to
        // IT; a bare name stays a constant. The standard is nominal ("after"
        // rules out the matrix verb) → a verb-word head is a deverbal noun
        // ("1 month after the goblin shark PROJECT").
        let saved_ctx = self.nominal_np_context;
        self.nominal_np_context = true;
        let std_np_result = self.parse_noun_phrase(true);
        self.nominal_np_context = saved_ctx;
        let std_np = std_np_result?;
        let has_rel = self.check(&TokenType::Who) || self.check(&TokenType::That);
        let is_desc = has_rel
            || std_np.definiteness.is_some()
            || !std_np.adjectives.is_empty()
            || std_np.possessor.is_some()
            || !std_np.pps.is_empty();
        let result = if is_desc {
            let std_var = self.next_var_name();
            let mut restr = self.nominal_predication(Term::Variable(std_var), &std_np);
            for pp in std_np.pps {
                let pp_sub = self.substitute_pp_placeholder(pp, std_var);
                restr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: restr,
                    op: TokenType::And,
                    right: pp_sub,
                });
            }
            if has_rel {
                self.advance(); // "who" / "that"
                let rel = self.parse_relative_clause(std_var)?;
                restr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: restr,
                    op: TokenType::And,
                    right: rel,
                });
            }
            let relation = build_constraint(self, Term::Variable(std_var));
            let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: restr,
                op: TokenType::And,
                right: relation,
            });
            self.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind: QuantifierKind::Existential,
                variable: std_var,
                body,
                island_id: self.current_island,
            })
        } else {
            build_constraint(self, Term::Constant(std_np.noun))
        };
        Ok(Some(result))
    }

    /// Ordinal-position offset after a just-consumed verb: `COUNT
    /// (place|places|spot|spots) (ahead of | behind | before | after) STANDARD`
    /// ("finished 2 places ahead of Bob", "performed 1 spot before Violet"). The
    /// verb event is asserted and a directed offset orders the two positions:
    /// `Place(X) = add|sub(Place(Y), N)` (ahead/before → sub, behind/after → add),
    /// solver-ready, with the standard as a DISTINCT entity. `None` if absent.
    pub(super) fn try_positional_offset(
        &mut self,
        verb: Symbol,
        subject_term: Term<'a>,
        verb_time: Time,
    ) -> ParseResult<Option<&'a LogicExpr<'a>>> {
        let j = self.current;
        let has_count = matches!(
            self.tokens.get(j).map(|t| &t.kind),
            Some(TokenType::Number(_)) | Some(TokenType::Cardinal(_))
        );
        let is_pos_unit = self
            .tokens
            .get(j + 1)
            .map(|t| {
                matches!(
                    self.interner.resolve(t.lexeme).to_lowercase().as_str(),
                    "place" | "places" | "spot" | "spots"
                )
            })
            .unwrap_or(false);
        if !(has_count && is_pos_unit) {
            return Ok(None);
        }
        // Direction word at j+2: ahead(/of) / before → sub; behind / after → add.
        let (subtract, ahead_of) = match self.tokens.get(j + 2).map(|t| &t.kind) {
            Some(TokenType::Before) => (true, false),
            Some(_) => match self
                .interner
                .resolve(self.tokens[j + 2].lexeme)
                .to_lowercase()
                .as_str()
            {
                "ahead" => (true, true),
                "before" => (true, false),
                "behind" => (false, false),
                "after" => (false, false),
                _ => return Ok(None),
            },
            None => return Ok(None),
        };

        let count_kind = match self.advance().kind {
            TokenType::Number(s) => {
                let raw = self.interner.resolve(s);
                if let Ok(n) = raw.parse::<i64>() {
                    crate::ast::NumberKind::Integer(n)
                } else {
                    crate::ast::NumberKind::Symbolic(s)
                }
            }
            TokenType::Cardinal(n) => crate::ast::NumberKind::Integer(n as i64),
            _ => unreachable!("guarded by has_count"),
        };
        self.advance(); // positional unit
        self.advance(); // direction word
        if ahead_of
            && matches!(self.peek().kind, TokenType::Preposition(s)
                if self.interner.resolve(s).eq_ignore_ascii_case("of"))
        {
            self.advance(); // "of" after "ahead"
        }

        let measure_sym = self.interner.intern("Place");
        let op_sym = self.interner.intern(if subtract { "sub" } else { "add" });
        let offset_term = Term::Value { kind: count_kind, unit: None, dimension: None };

        let event_var = self.get_event_var();
        let mut modifiers = Vec::new();
        let effective_time = self.pending_time.take().unwrap_or(verb_time);
        match effective_time {
            Time::Past => modifiers.push(self.interner.intern("Past")),
            Time::Future => modifiers.push(self.interner.intern("Future")),
            _ => {}
        }
        let suppress_existential = self.drs.in_conditional_antecedent();
        let event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
            event_var,
            verb,
            roles: self.ctx.roles.alloc_slice(vec![(ThematicRole::Agent, subject_term)]),
            modifiers: self.ctx.syms.alloc_slice(modifiers),
            suppress_existential,
            world: None,
        })));

        let measure_x = Term::Function(measure_sym, self.ctx.terms.alloc_slice([subject_term]));
        let build_constraint = move |p: &mut Self, y_term: Term<'a>| -> &'a LogicExpr<'a> {
            let measure_y = Term::Function(measure_sym, p.ctx.terms.alloc_slice([y_term]));
            let rhs = Term::Function(op_sym, p.ctx.terms.alloc_slice([measure_y, offset_term]));
            p.ctx.exprs.alloc(LogicExpr::Identity {
                left: p.ctx.terms.alloc(measure_x),
                right: p.ctx.terms.alloc(rhs),
            })
        };

        let std_np = self.parse_noun_phrase(true)?;
        let has_rel = self.check(&TokenType::Who) || self.check(&TokenType::That);
        let is_desc = has_rel
            || std_np.definiteness.is_some()
            || !std_np.adjectives.is_empty()
            || std_np.possessor.is_some()
            || !std_np.pps.is_empty();
        let result = if is_desc {
            let std_var = self.next_var_name();
            let mut restr = self.nominal_predication(Term::Variable(std_var), &std_np);
            for pp in std_np.pps {
                let pp_sub = self.substitute_pp_placeholder(pp, std_var);
                restr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: restr,
                    op: TokenType::And,
                    right: pp_sub,
                });
            }
            if has_rel {
                self.advance();
                let rel = self.parse_relative_clause(std_var)?;
                restr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: restr,
                    op: TokenType::And,
                    right: rel,
                });
            }
            let relation = build_constraint(self, Term::Variable(std_var));
            let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: restr,
                op: TokenType::And,
                right: relation,
            });
            let quantified = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind: QuantifierKind::Existential,
                variable: std_var,
                body,
                island_id: self.current_island,
            });
            self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: event,
                op: TokenType::And,
                right: quantified,
            })
        } else {
            let relation = build_constraint(self, Term::Constant(std_np.noun));
            self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: event,
                op: TokenType::And,
                right: relation,
            })
        };
        Ok(Some(result))
    }

    fn parse_predicate_impl(
        &mut self,
        subject_symbol: Symbol,
        as_variable: bool,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        let subject_term = if as_variable {
            Term::Variable(subject_symbol)
        } else {
            Term::Constant(subject_symbol)
        };

        // Weather verb + expletive "it" detection: "it rains" → ∃e(Rain(e))
        let subject_str = self.interner.resolve(subject_symbol).to_lowercase();
        if subject_str == "it" && self.check_verb() {
            if let TokenType::Verb { lemma, time, .. } = &self.peek().kind {
                let lemma_str = self.interner.resolve(*lemma);
                if Lexer::is_weather_verb(lemma_str) {
                    let verb = *lemma;
                    let verb_time = *time;
                    self.advance(); // consume the weather verb

                    let event_var = self.get_event_var();
                    let suppress_existential = self.drs.in_conditional_antecedent();
                    if suppress_existential {
                        let event_class = self.interner.intern("Event");
                        self.drs.introduce_referent(event_var, event_class, Gender::Neuter, Number::Singular);
                    }
                    let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                        event_var,
                        verb,
                        roles: self.ctx.roles.alloc_slice(vec![]), // No thematic roles
                        modifiers: self.ctx.syms.alloc_slice(vec![]),
                        suppress_existential,
                        world: None,
                    })));

                    return Ok(match verb_time {
                        Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                            operator: TemporalOperator::Past,
                            body: neo_event,
                        }),
                        Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                            operator: TemporalOperator::Future,
                            body: neo_event,
                        }),
                        _ => neo_event,
                    });
                }
            }
        }

        // Weather adjective + expletive "it" detection: "it is wet" → Wet
        // Also handle "it's wet" where 's is Possessive token
        if subject_str == "it" && (self.check(&TokenType::Is) || self.check(&TokenType::Was) || self.check(&TokenType::Possessive)) {
            let saved_pos = self.current;
            self.advance(); // consume copula

            if self.check_content_word() {
                let adj_lexeme = self.peek().lexeme;
                let adj_str = self.interner.resolve(adj_lexeme).to_lowercase();

                if let Some(meta) = crate::lexicon::lookup_adjective_db(&adj_str) {
                    if meta.features.contains(&crate::lexicon::Feature::Weather) {
                        let adj_sym = self.consume_content_word().unwrap_or(adj_lexeme);
                        // Atmospheric predicate: "it is wet" → Wet
                        return Ok(self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: adj_sym,
                            args: self.ctx.terms.alloc_slice([]),
                            world: None,
                        }));
                    }
                }
            }
            // Not a weather adjective, restore position
            self.current = saved_pos;
        }

        if self.check(&TokenType::Never) {
            self.advance();
            let verb = self.consume_verb();
            let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice([subject_term]),
                world: None,
            });
            return Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: verb_pred,
            }));
        }

        if self.check_modal() {
            return self.parse_aspect_chain_with_term(subject_term.clone());
        }

        if self.check_content_word() {
            let next_word = self.interner.resolve(self.peek().lexeme).to_lowercase();
            if next_word == "has" || next_word == "have" || next_word == "had" {
                // Look ahead to distinguish perfect aspect ("has eaten") from possession ("has 3 children")
                // Perfect aspect: has/have/had + verb
                // Possession: has/have/had + number/noun
                let is_perfect_aspect = if self.current + 1 < self.tokens.len() {
                    let next_token = &self.tokens[self.current + 1].kind;
                    matches!(
                        next_token,
                        TokenType::Verb { .. } | TokenType::Not
                    ) && !matches!(next_token, TokenType::Number(_))
                } else {
                    false
                };
                if is_perfect_aspect {
                    return self.parse_aspect_chain(subject_symbol);
                }
                // Otherwise, treat "has" as a main verb (possession) and continue below
            }
        }

        if self.check(&TokenType::Had) {
            return self.parse_aspect_chain(subject_symbol);
        }

        // Handle do-support: "I do/don't know who"
        if self.check(&TokenType::Does) || self.check(&TokenType::Do) {
            self.advance();
            let is_negated = self.match_token(&[TokenType::Not]);

            if self.check(&TokenType::Ever) {
                self.advance();
            }

            if self.check_verb() {
                let (verb, verb_time, verb_aspect, verb_class) =
                    self.consume_verb_with_metadata();

                // Check for embedded wh-clause with sluicing: "I don't know who"
                if self.check_wh_word() {
                    let wh_token = self.advance().kind.clone();
                    let is_who = matches!(wh_token, TokenType::Who);
                    let is_what = matches!(wh_token, TokenType::What);

                    let is_sluicing = self.is_at_end() ||
                        self.check(&TokenType::Period) ||
                        self.check(&TokenType::Comma);

                    if is_sluicing {
                        if let Some(template) = self.last_event_template.clone() {
                            let wh_var = self.next_var_name();

                            let roles: Vec<_> = if is_who {
                                std::iter::once((ThematicRole::Agent, Term::Variable(wh_var)))
                                    .chain(template.non_agent_roles.iter().cloned())
                                    .collect()
                            } else if is_what {
                                vec![
                                    (ThematicRole::Agent, subject_term.clone()),
                                    (ThematicRole::Theme, Term::Variable(wh_var)),
                                ]
                            } else {
                                std::iter::once((ThematicRole::Agent, Term::Variable(wh_var)))
                                    .chain(template.non_agent_roles.iter().cloned())
                                    .collect()
                            };

                            let event_var = self.get_event_var();
                            let suppress_existential = self.drs.in_conditional_antecedent();
                            let reconstructed = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                                event_var,
                                verb: template.verb,
                                roles: self.ctx.roles.alloc_slice(roles),
                                modifiers: self.ctx.syms.alloc_slice(template.modifiers.clone()),
                                suppress_existential,
                                world: None,
                            })));

                            let question = self.ctx.exprs.alloc(LogicExpr::Question {
                                wh_variable: wh_var,
                                body: reconstructed,
                            });

                            let know_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                                event_var: self.get_event_var(),
                                verb,
                                roles: self.ctx.roles.alloc_slice(vec![
                                    (ThematicRole::Agent, subject_term.clone()),
                                    (ThematicRole::Theme, Term::Proposition(question)),
                                ]),
                                modifiers: self.ctx.syms.alloc_slice(vec![]),
                                suppress_existential,
                                world: None,
                            })));

                            let result = if is_negated {
                                self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                                    op: TokenType::Not,
                                    operand: know_event,
                                })
                            } else {
                                know_event
                            };

                            return Ok(result);
                        }
                    }
                }

                // Regular do-support ("does/do/don't VERB …"): delegate the whole
                // VP — object, measure phrase, PPs, aspect — to the shared builder
                // so every complement form folds exactly as in the positive path,
                // wrapping in ¬ when "not"/"never" was present. The negative scope
                // is open across the complement so NPIs inside it are licensed.
                if is_negated {
                    self.negative_depth += 1;
                }
                let vp = self.build_verb_vp(
                    subject_symbol,
                    subject_term,
                    as_variable,
                    verb,
                    verb_time,
                    verb_aspect,
                    verb_class,
                )?;
                if is_negated {
                    self.negative_depth -= 1;
                    return Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand: vp,
                    }));
                }
                return Ok(vp);
            }
        }

        // Check for auxiliary (like "did" in "did not bark")
        // BUT: "did it" should be parsed as verb "do" with object "it"
        // We lookahead to check if this is truly an auxiliary usage
        if self.check_auxiliary() && self.is_true_auxiliary_usage() {
            let aux_time = if let TokenType::Auxiliary(time) = self.advance().kind {
                time
            } else {
                Time::None
            };
            self.pending_time = Some(aux_time);

            if self.match_token(&[TokenType::Not]) {
                self.negative_depth += 1;

                // A bare verb the lexicon ALSO lists as a performative ("didn't ORDER",
                // "didn't CALL") is the clause's main verb after do-support, not a
                // speech act — re-tag it so check_verb consumes it (the tense comes from
                // the "did"/"does" auxiliary).
                if let TokenType::Performative(_) = self.peek().kind {
                    let lemma = self
                        .interner
                        .intern(&self.interner.resolve(self.peek().lexeme).to_lowercase());
                    self.tokens[self.current].kind = TokenType::Verb {
                        lemma,
                        time: Time::None,
                        aspect: Aspect::Simple,
                        class: crate::lexicon::VerbClass::Activity,
                    };
                }
                // Check for verb or "do" (TokenType::Do is separate from TokenType::Verb)
                if self.check_verb() || self.check(&TokenType::Do) {
                    let (verb, verb_time, verb_aspect, verb_class) =
                        if self.check(&TokenType::Do) {
                            self.advance(); // consume "do"
                            (
                                self.interner.intern("Do"),
                                Time::None,
                                Aspect::Simple,
                                crate::lexicon::VerbClass::Activity,
                            )
                        } else {
                            self.consume_verb_with_metadata()
                        };

                    if self.check_quantifier() {
                        let quantifier_token = self.advance().kind.clone();
                        let object_np = self.parse_noun_phrase(false)?;
                        let obj_var = self.next_var_name();

                        let obj_restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: object_np.noun,
                            args: self.ctx.terms.alloc_slice([Term::Variable(obj_var)]),
                            world: None,
                        });

                        let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: verb,
                            args: self
                                .ctx
                                .terms
                                .alloc_slice([subject_term, Term::Variable(obj_var)]),
                            world: None,
                        });

                        let (kind, body) = match quantifier_token {
                            TokenType::Any => {
                                if self.is_negative_context() {
                                    (
                                        QuantifierKind::Existential,
                                        self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                            left: obj_restriction,
                                            op: TokenType::And,
                                            right: verb_pred,
                                        }),
                                    )
                                } else {
                                    (
                                        QuantifierKind::Universal,
                                        self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                            left: obj_restriction,
                                            op: TokenType::Implies,
                                            right: verb_pred,
                                        }),
                                    )
                                }
                            }
                            TokenType::Some => (
                                QuantifierKind::Existential,
                                self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                    left: obj_restriction,
                                    op: TokenType::And,
                                    right: verb_pred,
                                }),
                            ),
                            TokenType::All => (
                                QuantifierKind::Universal,
                                self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                    left: obj_restriction,
                                    op: TokenType::Implies,
                                    right: verb_pred,
                                }),
                            ),
                            _ => (
                                QuantifierKind::Existential,
                                self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                    left: obj_restriction,
                                    op: TokenType::And,
                                    right: verb_pred,
                                }),
                            ),
                        };

                        let quantified = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                            kind,
                            variable: obj_var,
                            body,
                            island_id: self.current_island,
                        });

                        let effective_time = self.pending_time.take().unwrap_or(Time::None);
                        let with_time = match effective_time {
                            Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                                operator: TemporalOperator::Past,
                                body: quantified,
                            }),
                            Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                                operator: TemporalOperator::Future,
                                body: quantified,
                            }),
                            _ => quantified,
                        };

                        self.negative_depth -= 1;
                        return Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                            op: TokenType::Not,
                            operand: with_time,
                        }));
                    }

                    if self.check_npi_object() {
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

                        let verb_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: verb,
                            args: self.ctx.terms.alloc_slice([subject_term, Term::Variable(obj_var)]),
                            world: None,
                        });

                        let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: obj_restriction,
                            op: TokenType::And,
                            right: verb_pred,
                        });

                        let quantified = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                            kind: QuantifierKind::Existential,
                            variable: obj_var,
                            body,
                            island_id: self.current_island,
                        });

                        let effective_time = self.pending_time.take().unwrap_or(Time::None);
                        let with_time = match effective_time {
                            Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                                operator: TemporalOperator::Past,
                                body: quantified,
                            }),
                            Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                                operator: TemporalOperator::Future,
                                body: quantified,
                            }),
                            _ => quantified,
                        };

                        self.negative_depth -= 1;
                        return Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                            op: TokenType::Not,
                            operand: with_time,
                        }));
                    }

                    // Delegate the VP complement (object/measure/PP/aspect) to the
                    // shared builder so negated do-support folds every complement
                    // form exactly as the positive path does. pending_time still
                    // carries the auxiliary's tense ("did" → Past) into the build.
                    let vp = self.build_verb_vp(
                        subject_symbol,
                        subject_term,
                        as_variable,
                        verb,
                        verb_time,
                        verb_aspect,
                        verb_class,
                    )?;

                    self.negative_depth -= 1;
                    return Ok(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand: vp,
                    }));
                }

                self.negative_depth -= 1;
            }
        }

        if self.check(&TokenType::Is)
            || self.check(&TokenType::Are)
            || self.check(&TokenType::Was)
            || self.check(&TokenType::Were)
        {
            let copula_time = if self.check(&TokenType::Was) || self.check(&TokenType::Were) {
                Time::Past
            } else {
                Time::Present
            };
            self.advance();

            // Check for negation: "was not caught", "is not happy"
            let is_negated = self.check(&TokenType::Not);
            if is_negated {
                self.advance(); // consume "not"
            }

            // Check for temporal adverbs after copula: "is eventually Y", "is always Y", "is never Y"
            let mut copula_temporal: Option<super::CopulaTemporal> = None;
            if !is_negated {
                if self.check(&TokenType::Never) {
                    self.advance();
                    copula_temporal = Some(super::CopulaTemporal::Never);
                } else if let TokenType::Adverb(sym) | TokenType::ScopalAdverb(sym) | TokenType::TemporalAdverb(sym) = &self.peek().kind {
                    let resolved = self.interner.resolve(*sym).to_string();
                    if resolved == "Always" || resolved == "always" {
                        self.advance();
                        copula_temporal = Some(super::CopulaTemporal::Always);
                    } else if resolved == "Eventually" || resolved == "eventually" {
                        self.advance();
                        copula_temporal = Some(super::CopulaTemporal::Eventually);
                    }
                }
            }

            if self.check_verb() {
                let (verb, _verb_time, verb_aspect, verb_class) = self.consume_verb_with_metadata();

                // Stative verbs cannot be progressive
                if verb_class.is_stative() && verb_aspect == Aspect::Progressive {
                    return Err(crate::error::ParseError {
                        kind: crate::error::ParseErrorKind::StativeProgressiveConflict,
                        span: self.current_span(),
                    });
                }

                let mut predicate: &'a LogicExpr<'a> = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: verb,
                    args: self.ctx.terms.alloc_slice([subject_term]),
                    world: None,
                });

                // Passive by-phrase: the NP after `by` is the AGENT and must fill
                // the predicate's agent (FIRST) slot — `See(Mary, John)` for
                // "John was seen by Mary" — matching the main passive path. Handle
                // it BEFORE the generic locative-PP loop below, which would
                // otherwise demote the agent into a spurious `by(theme, agent)`.
                if self.check_by_preposition() {
                    self.advance(); // consume "by"
                    if self.check_content_word()
                        || matches!(self.peek().kind, TokenType::Article(_))
                    {
                        let agent = self.parse_noun_phrase(true)?;
                        // A DESCRIPTIVE by-agent becomes its own restrictor-carrying
                        // entity scoping the relation; a bare one keeps the constant.
                        let (agent_term, agent_restr) = self.possessor_entity(&agent);
                        let core = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: verb,
                            args: self.ctx.terms.alloc_slice([agent_term, subject_term]),
                            world: None,
                        });
                        predicate = self.wrap_in_possessor_entity(agent_restr, core);
                    }
                }

                // Trailing PP adjuncts on the passive participle ("was found in
                // Spain", "was taken on May 12", "was at 88.2 W") and a calendar-unit
                // offset ("was taken 1 month after Y") — predicated of the theme.
                // This makes the embedded VP parser (of-pair members, delegated
                // relative clauses) as capable as parse_atom's main passive path.
                while self.check_preposition() && !self.check_of_preposition()
                    && !self.pp_is_cycle_temporal()
                {
                    let prep = match self.advance().kind {
                        TokenType::Preposition(s) => s,
                        _ => break,
                    };
                    let adjunct = if self.check_number() {
                        let m = self.parse_measure_phrase()?;
                        self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: prep,
                            args: self.ctx.terms.alloc_slice([subject_term, *m]),
                            world: None,
                        })
                    } else if self.check_content_word()
                        || matches!(self.peek().kind, TokenType::Article(_))
                    {
                        let obj = self.parse_noun_phrase(true)?;
                        self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: prep,
                            args: self.ctx.terms.alloc_slice([subject_term, Term::Constant(obj.noun)]),
                            world: None,
                        })
                    } else {
                        break;
                    };
                    predicate = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: predicate,
                        op: TokenType::And,
                        right: adjunct,
                    });
                }
                if let Some(constraint) = self.parse_temporal_offset_constraint(subject_term)? {
                    predicate = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: predicate,
                        op: TokenType::And,
                        right: constraint,
                    });
                }

                let with_aspect = if verb_aspect == Aspect::Progressive {
                    // Semelfactive + Progressive → Iterative
                    let operator = if verb_class == crate::lexicon::VerbClass::Semelfactive {
                        AspectOperator::Iterative
                    } else {
                        AspectOperator::Progressive
                    };
                    self.ctx.exprs.alloc(LogicExpr::Aspectual {
                        operator,
                        body: predicate,
                    })
                } else {
                    predicate
                };

                let with_time = if copula_time == Time::Past {
                    self.ctx.exprs.alloc(LogicExpr::Temporal {
                        operator: TemporalOperator::Past,
                        body: with_aspect,
                    })
                } else {
                    with_aspect
                };

                let with_neg = if is_negated {
                    self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand: with_time,
                    })
                } else {
                    with_time
                };

                let result = match copula_temporal {
                    Some(super::CopulaTemporal::Always) => {
                        self.ctx.exprs.alloc(LogicExpr::Temporal {
                            operator: TemporalOperator::Always,
                            body: with_neg,
                        })
                    }
                    Some(super::CopulaTemporal::Never) => {
                        let negated = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                            op: TokenType::Not,
                            operand: with_time,
                        });
                        self.ctx.exprs.alloc(LogicExpr::Temporal {
                            operator: TemporalOperator::Always,
                            body: negated,
                        })
                    }
                    Some(super::CopulaTemporal::Eventually) => {
                        self.ctx.exprs.alloc(LogicExpr::Temporal {
                            operator: TemporalOperator::Eventually,
                            body: with_neg,
                        })
                    }
                    None => with_neg,
                };

                return Ok(result);
            }

            // "is 14 inches tall" / "is 5 meters long" — a measure phrase + a
            // dimensional adjective: Adj(subject, measure). Also bare "is 98.6
            // degrees" → Identity. parse_atom handles this for its subjects; the
            // of-pair / quantified-subject copula VPs ("the other is 14 inches
            // tall") reach here. A measure-OFFSET comparative ("is 2 inches
            // taller than X") is left to fall through (not mis-read as Identity).
            if self.check_number() {
                let after_measure_is_comparative = {
                    let mut i = self.current + 1; // past the number
                    if matches!(
                        self.tokens.get(i).map(|t| &t.kind),
                        Some(TokenType::Noun(_)) | Some(TokenType::CalendarUnit(_))
                    ) {
                        i += 1; // past an optional unit word
                    }
                    matches!(
                        self.tokens.get(i).map(|t| &t.kind),
                        Some(TokenType::Comparative(_))
                    )
                };
                if !after_measure_is_comparative {
                    let measure = self.parse_measure_phrase()?;
                    let pred = if self.check_content_word() {
                        let adj = self.consume_content_word()?;
                        self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: adj,
                            args: self.ctx.terms.alloc_slice([subject_term, *measure]),
                            world: None,
                        })
                    } else {
                        self.ctx.exprs.alloc(LogicExpr::Identity {
                            left: self.ctx.terms.alloc(subject_term),
                            right: measure,
                        })
                    };
                    return Ok(self.finish_copula(pred, copula_time, is_negated, copula_temporal));
                }
            }

            // "is on Rosewood Street" / "is from Australia" — locative/origin PP in copula position.
            // Excludes "by" which is handled as passive-agent above.
            if self.check_preposition() && !self.check_by_preposition() {
                let prep_token = self.advance().clone();
                let prep_sym = match prep_token.kind {
                    TokenType::Preposition(s) => s,
                    _ => unreachable!("guarded by check_preposition()"),
                };
                // A coordinate / measure object ("is at 88.2 W", "is at 40.5 N")
                // routes through parse_measure_phrase, which takes the trailing
                // direction/unit; otherwise a plain NP object.
                let base = if self.check_number() {
                    let m = self.parse_measure_phrase()?;
                    self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: prep_sym,
                        args: self.ctx.terms.alloc_slice([subject_term, *m]),
                        world: None,
                    })
                } else {
                    let pp_obj = self.parse_noun_phrase(true)?;
                    self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: prep_sym,
                        args: self.ctx.terms.alloc_slice([subject_term, Term::Constant(pp_obj.noun)]),
                        world: None,
                    })
                };
                let with_time = if copula_time == Time::Past {
                    self.ctx.exprs.alloc(LogicExpr::Temporal {
                        operator: TemporalOperator::Past,
                        body: base,
                    })
                } else {
                    base
                };
                return Ok(if is_negated {
                    self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand: with_time,
                    })
                } else {
                    with_time
                });
            }

            // "is Tara" — identity with a proper name (X = Tara), enabling Leibniz's
            // Law. But "is Kerry's project" is a POSSESSIVE NP complement
            // (Project(x) ∧ Possesses(Kerry, x)), so a following "'s" diverts to NP
            // predication instead of the bare identity.
            if let TokenType::ProperName(pname) = self.peek().kind {
                if matches!(
                    self.tokens.get(self.current + 1).map(|t| &t.kind),
                    Some(TokenType::Possessive)
                ) {
                    // "is Ginger's." — an ELIDED possessed noun (a clause boundary
                    // follows "'s") → Possesses(Ginger, subject). parse_atom's
                    // copula handles this; the of-pair / quantified-subject VPs
                    // route here, where it previously failed (ExpectedContentWord
                    // at the period). "is Ginger's PROJECT" still parses the full
                    // possessive NP below.
                    let elided = matches!(
                        self.tokens.get(self.current + 2).map(|t| &t.kind),
                        Some(TokenType::Period) | Some(TokenType::EOF)
                            | Some(TokenType::Comma) | Some(TokenType::And)
                            | Some(TokenType::Or) | None
                    );
                    if elided {
                        self.advance(); // proper name
                        self.advance(); // possessive
                        let pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: self.interner.intern("Possesses"),
                            args: self.ctx.terms.alloc_slice([
                                Term::Constant(pname),
                                subject_term,
                            ]),
                            world: None,
                        });
                        return Ok(self.finish_copula(pred, copula_time, is_negated, copula_temporal));
                    }
                    let saved_ctx = self.nominal_np_context;
                    self.nominal_np_context = true;
                    let np_result = self.parse_noun_phrase(true);
                    self.nominal_np_context = saved_ctx;
                    let np = np_result?;
                    let pred = self.nominal_predication_with_pps(subject_term, &np);
                    return Ok(self.finish_copula(pred, copula_time, is_negated, copula_temporal));
                }

                // A possessive after a MULTI-WORD proper name ("is Tim Tucker's
                // film") — the single-word check above misses it (the second name
                // sits where the "'s" would be). Absorb the full possessor, then
                // predicate the possessed noun: Possesses(Tim_Tucker, x) ∧ Film(x).
                let multiword_poss = {
                    let mut k = self.current + 1;
                    while matches!(self.tokens.get(k).map(|t| &t.kind), Some(TokenType::ProperName(_))) {
                        k += 1;
                    }
                    if k > self.current + 1
                        && matches!(self.tokens.get(k).map(|t| &t.kind), Some(TokenType::Possessive))
                    {
                        Some(k)
                    } else {
                        None
                    }
                };
                if let Some(poss_pos) = multiword_poss {
                    let elided = matches!(
                        self.tokens.get(poss_pos + 1).map(|t| &t.kind),
                        Some(TokenType::Period) | Some(TokenType::EOF)
                            | Some(TokenType::Comma) | Some(TokenType::And)
                            | Some(TokenType::Or) | None
                    );
                    self.advance(); // first proper name
                    let possessor = self.absorb_multiword_proper_name(pname);
                    self.advance(); // possessive
                    let poss_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: self.interner.intern("Possesses"),
                        args: self.ctx.terms.alloc_slice([Term::Constant(possessor), subject_term]),
                        world: None,
                    });
                    let pred = if elided {
                        poss_pred
                    } else {
                        let saved_ctx = self.nominal_np_context;
                        self.nominal_np_context = true;
                        let np_result = self.parse_noun_phrase(true);
                        self.nominal_np_context = saved_ctx;
                        let np = np_result?;
                        let np_pred = self.nominal_predication_with_pps(subject_term, &np);
                        self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: np_pred,
                            op: TokenType::And,
                            right: poss_pred,
                        })
                    };
                    return Ok(self.finish_copula(pred, copula_time, is_negated, copula_temporal));
                }

                self.advance();
                // Absorb subsequent capitalized words into one multi-word proper
                // name ("Porcher Place", "Highland Drive") — a place/title name is
                // a single entity, so the identity must not strand the second word.
                let pname = self.absorb_multiword_proper_name(pname);
                let identity = self.ctx.exprs.alloc(LogicExpr::Identity {
                    left: self.ctx.terms.alloc(subject_term),
                    right: self.ctx.terms.alloc(Term::Constant(pname)),
                });
                return Ok(self.finish_copula(identity, copula_time, is_negated, copula_temporal));
            }

            // "is either A or B" — disjunctive predication: A(x) ∨ B(x).
            if self.check(&TokenType::Either) {
                self.advance(); // consume "either"
                let saved_ctx = self.nominal_np_context;
                self.nominal_np_context = true;
                let np1_result = self.parse_noun_phrase(true);
                self.nominal_np_context = saved_ctx;
                let np1 = np1_result?;
                // Keep the disjunct's PP restrictors ("either the vegetables FROM
                // JESUP or …") — bare nominal_predication drops them, the same
                // meaning-loss the plain copula complement avoids with _with_pps.
                let pred1 = self.nominal_predication_with_pps(subject_term, &np1);
                // A relative clause on the disjunct ("either the one WHO won or …")
                // attaches before "or"; the second disjunct's after np2.
                let pred1 = self.conjoin_trailing_relative(pred1, subject_term)?;
                if self.check(&TokenType::Or) {
                    self.advance(); // consume "or"
                    let saved_ctx2 = self.nominal_np_context;
                    self.nominal_np_context = true;
                    let np2_result = self.parse_noun_phrase(true);
                    self.nominal_np_context = saved_ctx2;
                    let np2 = np2_result?;
                    let pred2 = self.nominal_predication_with_pps(subject_term, &np2);
                    let pred2 = self.conjoin_trailing_relative(pred2, subject_term)?;
                    let disj = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: pred1,
                        op: TokenType::Or,
                        right: pred2,
                    });
                    return Ok(self.finish_copula(disj, copula_time, is_negated, copula_temporal));
                }
                return Ok(self.finish_copula(pred1, copula_time, is_negated, copula_temporal));
            }

            // "is the mansion" / "is the frat on Holly Street" / "is the Alvarado
            // family's house" — NP predication keeping the genitive AND the
            // predicate NP's PP restrictors ("on Holly Street"); dropping the PP
            // is a meaning-loss parse.
            if self.check_article() {
                let saved_ctx = self.nominal_np_context;
                self.nominal_np_context = true;
                let pred_np_result = self.parse_noun_phrase(true);
                self.nominal_np_context = saved_ctx;
                let pred_np = pred_np_result?;
                let pred = self.nominal_predication_with_pps(subject_term, &pred_np);
                // A relative clause on the predicate nominal ("was the player WHO
                // played", "is the one THAT won") is predicated of the subject —
                // being that player entails the subject played. neither/nor and
                // quantified subjects route their copula complement through here.
                let pred = self.conjoin_trailing_relative(pred, subject_term)?;
                return Ok(self.finish_copula(pred, copula_time, is_negated, copula_temporal));
            }

            // "is older" / "is faster than the cobra" — a comparative copula
            // complement. parse_atom routes its subjects to parse_comparative;
            // quantified / of-pair subjects (parse_predicate_with_subject) reach
            // here, so the comparative must be handled too. Use the COMPARATIVE
            // surface ("older" → Older), not the base lemma, so the degree
            // survives. Bare (no "than") → unary property ("one is older"); with
            // "than" → binary comparison.
            if let TokenType::Comparative(_) = self.peek().kind {
                let comp_tok = self.advance().clone();
                let comp_surface = self.interner.resolve(comp_tok.lexeme).to_string();
                let comp_name = {
                    let mut c = comp_surface.chars();
                    match c.next() {
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                        None => comp_surface.clone(),
                    }
                };
                let name = self.interner.intern(&comp_name);
                let pred = if self.check(&TokenType::Than) {
                    self.advance(); // than
                    let std_np = self.parse_noun_phrase(true)?;
                    let std = self.nominal_predication(Term::Constant(std_np.noun), &std_np);
                    let cmp = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name,
                        args: self.ctx.terms.alloc_slice([
                            subject_term,
                            Term::Constant(std_np.noun),
                        ]),
                        world: None,
                    });
                    // Keep the standard's own restrictors (a bare proper name's
                    // std is vacuous Predicate, harmless).
                    if matches!(std, LogicExpr::Predicate { args, .. } if args.len() == 1) {
                        cmp
                    } else {
                        self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: cmp,
                            op: TokenType::And,
                            right: std,
                        })
                    }
                } else {
                    self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name,
                        args: self.ctx.terms.alloc_slice([subject_term]),
                        world: None,
                    })
                };
                return Ok(self.finish_copula(pred, copula_time, is_negated, copula_temporal));
            }

            // Copula complement led by a temporal/ordinal adverb: "was FIRST", "is
            // NOW the leader". Shared with parse_atom's copula path.
            if let Some(base) = self.copula_temporal_adverb_complement(subject_term.clone())? {
                return Ok(self.finish_copula(base, copula_time, is_negated, copula_temporal));
            }

            let predicate = self.consume_content_word()?;

            // Coordinated predicate adjectives — "is black and red" (or "black &
            // red", where "&" lexes to "and") → Adj1(subj) ∧ Adj2(subj). Mirrors
            // parse_atom; of-pair / neither / quantified subjects reach this shared
            // VP parser. Requires "and" before each extra adjective.
            {
                let mut coord_adjs: Vec<Symbol> = vec![predicate];
                while self.check(&TokenType::And) {
                    let saved = self.current;
                    self.advance();
                    if let TokenType::Adjective(a) = self.peek().kind {
                        // "and ADJ is/are/verb …" — ADJ is the SUBJECT of a new
                        // clause, not a coordinated predicate adjective.
                        if matches!(
                            self.tokens.get(self.current + 1).map(|t| &t.kind),
                            Some(TokenType::Is) | Some(TokenType::Are)
                                | Some(TokenType::Was) | Some(TokenType::Were)
                                | Some(TokenType::Verb { .. })
                        ) {
                            self.current = saved;
                            break;
                        }
                        self.advance();
                        coord_adjs.push(a);
                    } else {
                        self.current = saved;
                        break;
                    }
                }
                if coord_adjs.len() > 1 {
                    let mut conj: &'a LogicExpr<'a> = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: coord_adjs[0],
                        args: self.ctx.terms.alloc_slice([subject_term.clone()]),
                        world: None,
                    });
                    for &a in &coord_adjs[1..] {
                        let p = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: a,
                            args: self.ctx.terms.alloc_slice([subject_term.clone()]),
                            world: None,
                        });
                        conj = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: conj,
                            op: TokenType::And,
                            right: p,
                        });
                    }
                    return Ok(self.finish_copula(conj, copula_time, is_negated, copula_temporal));
                }
            }

            // Postposed measure complement on a predicate adjective — "is worth
            // $26 billion", "is worth 5 dollars" → Worth(subject, $26 billion).
            // Mirrors parse_atom's copula path; of-pair / neither / quantified
            // subjects reach this shared VP parser, so the complement lives here
            // too. A number after a predicate adjective is never a separate clause.
            if self.check_number() {
                let measure = self.parse_measure_phrase()?;
                let pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: predicate,
                    args: self.ctx.terms.alloc_slice([subject_term.clone(), *measure]),
                    world: None,
                });
                return Ok(self.finish_copula(pred, copula_time, is_negated, copula_temporal));
            }

            let base_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: predicate,
                args: self.ctx.terms.alloc_slice([subject_term]),
                world: None,
            });

            let with_time = if copula_time == Time::Past {
                self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: TemporalOperator::Past,
                    body: base_pred,
                })
            } else {
                base_pred
            };

            let with_neg = if is_negated {
                self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                    op: TokenType::Not,
                    operand: with_time,
                })
            } else {
                with_time
            };

            let result = match copula_temporal {
                Some(super::CopulaTemporal::Always) => {
                    self.ctx.exprs.alloc(LogicExpr::Temporal {
                        operator: TemporalOperator::Always,
                        body: with_neg,
                    })
                }
                Some(super::CopulaTemporal::Never) => {
                    let negated = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand: with_time,
                    });
                    self.ctx.exprs.alloc(LogicExpr::Temporal {
                        operator: TemporalOperator::Always,
                        body: negated,
                    })
                }
                Some(super::CopulaTemporal::Eventually) => {
                    self.ctx.exprs.alloc(LogicExpr::Temporal {
                        operator: TemporalOperator::Eventually,
                        body: with_neg,
                    })
                }
                None => with_neg,
            };

            return Ok(result);
        }

        // Handle "did it" - when Auxiliary(Past) is used as a transitive verb (past of "do")
        // This happens when we bypassed auxiliary handling because of lookahead
        if self.check_auxiliary_as_main_verb() {
            return self.parse_do_as_main_verb(subject_term);
        }

        self.parse_finite_verb_vp(subject_symbol, subject_term, as_variable)
    }

    /// Parses a finite verb's VP body: the verb, its object/measure/coordinated/
    /// ditransitive complement, trailing PPs, object-internal adjectives/PPs, and
    /// aspectual operators — building the NeoEvent. Shared by the positive predicate
    /// path and the do-support paths ("does/do/did not VERB …"), which wrap the
    /// result in negation; unifying them so every complement form folds uniformly.
    pub(super) fn parse_finite_verb_vp(
        &mut self,
        subject_symbol: Symbol,
        subject_term: Term<'a>,
        as_variable: bool,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        if self.check_verb() {
            let (verb, verb_time, verb_aspect, verb_class) = self.consume_verb_with_metadata();
            self.build_verb_vp(
                subject_symbol,
                subject_term,
                as_variable,
                verb,
                verb_time,
                verb_aspect,
                verb_class,
            )
        } else {
            Ok(self.ctx.exprs.alloc(LogicExpr::Atom(subject_symbol)))
        }
    }

    /// The VP body shared by the positive predicate path and the do-support paths
    /// ("does/do/did not VERB …"): the object/measure/coordinated/ditransitive
    /// complement, trailing PPs, object-internal adjectives/PPs, and aspectual
    /// operators — building and returning the NeoEvent. The caller consumes the
    /// verb (with metadata); negated do-support wraps the result in ¬.
    ///
    /// Coordinated OBJECT LISTS ("Determine each trip's activity, state and year, as
    /// well as the friend …") are handled here: the first object's predication is
    /// built by [`build_verb_vp_single`], then each comma/and-separated additional
    /// object yields its own predication of the SAME verb, all conjoined. A leading
    /// determiner/possessor ("each trip's") distributes over the bare coordinate
    /// heads (it is replayed before each), while a coordinate carrying its own
    /// determiner ("the friend …") is parsed as a fresh NP. No member is dropped.
    pub(super) fn build_verb_vp(
        &mut self,
        subject_symbol: Symbol,
        subject_term: Term<'a>,
        as_variable: bool,
        verb: Symbol,
        verb_time: Time,
        verb_aspect: Aspect,
        verb_class: crate::lexicon::VerbClass,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        // The leading determiner/possessor that distributes over a coordinate head
        // list ("each trip's" in "each trip's activity, state and year"): a
        // quantifier optionally followed by a possessive NP, captured as raw tokens
        // to replay before each bare coordinate. Empty when the object opens with no
        // shared determiner.
        let shared_prefix = self.capture_distributive_prefix();

        let first = self.build_verb_vp_single(
            subject_symbol,
            subject_term.clone(),
            as_variable,
            verb,
            verb_time,
            verb_aspect,
            verb_class,
        )?;

        let mut combined = first;
        loop {
            // Consume a coordinator: "," / "and" / ", and" / ", as well as" (the MWE
            // collapses "as well as" to And, so ", as well as X" arrives as Comma
            // And X). A trailing comma with no following object is not a coordinator.
            let mut k = self.current;
            let mut saw_coordinator = false;
            if matches!(self.tokens.get(k).map(|t| &t.kind), Some(TokenType::Comma)) {
                k += 1;
                saw_coordinator = true;
            }
            if matches!(self.tokens.get(k).map(|t| &t.kind), Some(TokenType::And)) {
                k += 1;
                saw_coordinator = true;
            }
            if !saw_coordinator {
                break;
            }
            // An object must actually follow the coordinator — a determiner, a
            // content word, or a number. Otherwise this is sentential/VP "and" (not
            // ours to consume), so leave it for the surrounding parser.
            let opens_object = matches!(
                self.tokens.get(k).map(|t| &t.kind),
                Some(TokenType::Article(_))
                    | Some(TokenType::All)
                    | Some(TokenType::Some)
                    | Some(TokenType::No)
                    | Some(TokenType::Any)
                    | Some(TokenType::Most)
                    | Some(TokenType::Few)
                    | Some(TokenType::Many)
                    | Some(TokenType::Noun(_))
                    | Some(TokenType::ProperName(_))
                    | Some(TokenType::CalendarUnit(_))
                    | Some(TokenType::Ambiguous { .. })
                    | Some(TokenType::Number(_))
                    | Some(TokenType::Cardinal(_))
            );
            if !opens_object {
                break;
            }
            // CLAUSE-BOUNDARY GUARD: a determiner/adjective-led NP whose COMMON-NOUN
            // head is immediately followed by a finite verb is the SUBJECT of a new
            // clause, not a coordinated object — "enters the room, the alarm TRIGGERS"
            // must not coordinate "the room, the alarm". A pure-noun head must be
            // crossed first, so a bare ambiguous noun/verb coordinate head ("…, STATE
            // and year") is still coordinated and a proper-name reduced relative ("the
            // friend SIMON went with") is not mistaken for a clause.
            {
                let mut p = k;
                if matches!(
                    self.tokens.get(p).map(|t| &t.kind),
                    Some(TokenType::Article(_)) | Some(TokenType::All) | Some(TokenType::Some)
                        | Some(TokenType::No) | Some(TokenType::Any) | Some(TokenType::Most)
                        | Some(TokenType::Few) | Some(TokenType::Many)
                ) {
                    p += 1;
                }
                while matches!(
                    self.tokens.get(p).map(|t| &t.kind),
                    Some(TokenType::Adjective(_)) | Some(TokenType::NonIntersectiveAdjective(_))
                ) {
                    p += 1;
                }
                let head_start = p;
                while matches!(self.tokens.get(p).map(|t| &t.kind), Some(TokenType::Noun(_))) {
                    p += 1;
                }
                let saw_noun_head = p > head_start;
                let verb_follows = self.tokens.get(p).map_or(false, |t| {
                    self.kind_is_verb(&t.kind)
                        || matches!(
                            t.kind,
                            TokenType::Auxiliary(_)
                                | TokenType::Is | TokenType::Are | TokenType::Was | TokenType::Were
                                | TokenType::Must | TokenType::Can | TokenType::Should
                                | TokenType::Could | TokenType::Would | TokenType::May
                                | TokenType::Might | TokenType::Shall | TokenType::Cannot
                        )
                });
                if saw_noun_head && verb_follows {
                    break;
                }
            }
            // Commit to consuming the coordinator tokens.
            self.current = k;
            // A bare coordinate head (no determiner of its own) inherits the shared
            // distributive prefix; one with its own determiner is parsed as-is.
            let has_own_determiner = matches!(
                self.tokens.get(self.current).map(|t| &t.kind),
                Some(TokenType::Article(_))
                    | Some(TokenType::All)
                    | Some(TokenType::Some)
                    | Some(TokenType::No)
                    | Some(TokenType::Any)
                    | Some(TokenType::Most)
                    | Some(TokenType::Few)
                    | Some(TokenType::Many)
            );
            if !has_own_determiner && !shared_prefix.is_empty() {
                self.splice_tokens(self.current, &shared_prefix);
            }
            let next = self.build_verb_vp_single(
                subject_symbol,
                subject_term.clone(),
                as_variable,
                verb,
                verb_time,
                verb_aspect,
                verb_class,
            )?;
            combined = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: combined,
                op: TokenType::And,
                right: next,
            });
        }
        Ok(combined)
    }

    /// Capture the leading distributive determiner/possessor of the object at the
    /// cursor as cloned tokens to be replayed before bare coordinate heads — a
    /// quantifier (`each`/`every`/…) optionally followed by a possessive NP
    /// (`trip 's`). Returns an empty vector and leaves the cursor unchanged when the
    /// object opens with no such prefix (a definite article, a bare noun, …).
    fn capture_distributive_prefix(&self) -> Vec<Token> {
        let mut p = self.current;
        let mut prefix: Vec<Token> = Vec::new();
        let is_quantifier = matches!(
            self.tokens.get(p).map(|t| &t.kind),
            Some(TokenType::All)
                | Some(TokenType::Some)
                | Some(TokenType::No)
                | Some(TokenType::Any)
                | Some(TokenType::Most)
                | Some(TokenType::Few)
                | Some(TokenType::Many)
        );
        if !is_quantifier {
            return prefix;
        }
        prefix.push(self.tokens[p].clone());
        p += 1;
        // An optional possessive NP head ("trip 's"): one or more nouns then "'s".
        let mut q = p;
        let mut saw_noun = false;
        while matches!(
            self.tokens.get(q).map(|t| &t.kind),
            Some(TokenType::Noun(_)) | Some(TokenType::Ambiguous { .. })
        ) {
            saw_noun = true;
            q += 1;
        }
        if saw_noun
            && matches!(
                self.tokens.get(q).map(|t| &t.kind),
                Some(TokenType::Possessive)
            )
        {
            for t in &self.tokens[p..=q] {
                prefix.push(t.clone());
            }
        }
        prefix
    }

    /// Insert `toks` into the token stream at index `at`, shifting the tail right.
    fn splice_tokens(&mut self, at: usize, toks: &[Token]) {
        for (i, t) in toks.iter().enumerate() {
            self.tokens.insert(at + i, t.clone());
        }
    }

    /// Builds the predication for a SINGLE object (see [`build_verb_vp`], which wraps
    /// this to coordinate object lists). The caller has consumed the verb.
    pub(super) fn build_verb_vp_single(
        &mut self,
        subject_symbol: Symbol,
        subject_term: Term<'a>,
        as_variable: bool,
        mut verb: Symbol,
        verb_time: Time,
        verb_aspect: Aspect,
        verb_class: crate::lexicon::VerbClass,
    ) -> ParseResult<&'a LogicExpr<'a>> {
            let mut args = vec![subject_term.clone()];

            // Control/raising verb with infinitival complement ("wants to
            // play"): route through the canonical control machinery, then
            // restore the subject's variable-ness so quantified subjects bind
            // into the complement ("Every child wants to play." → W(x, Play(x))).
            if self.is_control_verb(verb) && self.check_to() {
                let subject_np = NounPhrase {
                    noun: subject_symbol,
                    definiteness: None,
                    adjectives: &[],
                    possessor: None,
                    pps: &[],
                    superlative: None,
                };
                let control = self.parse_control_structure(&subject_np, verb, verb_time)?;
                return if as_variable {
                    self.substitute_constant_with_var(control, subject_symbol, subject_symbol)
                } else {
                    Ok(control)
                };
            }

            // Arithmetic / vague verbal comparative ("scored 3 points lower
            // than Bessie", "scored somewhat higher than Shirley"). Shared
            // with parse_atom's inline VP path via try_arithmetic_comparative.
            if let Some(cmp) = self.try_arithmetic_comparative(verb, subject_term.clone(), verb_time)? {
                return Ok(cmp);
            }

            // Temporal offset ("performed 2 weeks after Bessie").
            if let Some(off) = self.try_temporal_offset(verb, subject_term.clone(), verb_time)? {
                return Ok(off);
            }

            // Ordinal-position offset ("finished 2 places ahead of Bob").
            if let Some(off) = self.try_positional_offset(verb, subject_term.clone(), verb_time)? {
                return Ok(off);
            }

            // Verbal comparative ("runs faster than Bob", "run faster than all
            // cats"): the comparative grades the event participants — the verb
            // event is asserted and the subject compared to the standard.
            if let TokenType::Comparative(comp_adj) = self.peek().kind.clone() {
                if matches!(
                    self.tokens.get(self.current + 1).map(|t| t.kind.clone()),
                    Some(TokenType::Than)
                ) {
                    self.advance(); // comparative
                    self.advance(); // than

                    let event_var = self.get_event_var();
                    let mut modifiers = Vec::new();
                    let effective_time = self.pending_time.take().unwrap_or(verb_time);
                    match effective_time {
                        Time::Past => modifiers.push(self.interner.intern("Past")),
                        Time::Future => modifiers.push(self.interner.intern("Future")),
                        _ => {}
                    }
                    let suppress_existential = self.drs.in_conditional_antecedent();
                    let event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                        event_var,
                        verb,
                        roles: self
                            .ctx
                            .roles
                            .alloc_slice(vec![(ThematicRole::Agent, subject_term.clone())]),
                        modifiers: self.ctx.syms.alloc_slice(modifiers),
                        suppress_existential,
                        world: None,
                    })));

                    let result = if self.check_quantifier() {
                        let q = self.advance().kind.clone();
                        let std_np = self.parse_noun_phrase(false)?;
                        let std_var = self.next_var_name();
                        let restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: std_np.noun,
                            args: self.ctx.terms.alloc_slice([Term::Variable(std_var)]),
                            world: None,
                        });
                        let comparison = self.ctx.exprs.alloc(LogicExpr::Comparative {
                            adjective: comp_adj,
                            subject: self.ctx.terms.alloc(subject_term.clone()),
                            object: self.ctx.terms.alloc(Term::Variable(std_var)),
                            difference: None,
                            relation: crate::ast::logic::ComparisonRelation::Greater,
                        });
                        let (std_kind, std_op) = match q {
                            TokenType::All => (QuantifierKind::Universal, TokenType::Implies),
                            TokenType::Most => (QuantifierKind::Most, TokenType::And),
                            TokenType::Few => (QuantifierKind::Few, TokenType::And),
                            TokenType::Many => (QuantifierKind::Many, TokenType::And),
                            _ => (QuantifierKind::Existential, TokenType::And),
                        };
                        let std_body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: restriction,
                            op: std_op,
                            right: comparison,
                        });
                        let quantified = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                            kind: std_kind,
                            variable: std_var,
                            body: std_body,
                            island_id: self.current_island,
                        });
                        self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: event,
                            op: TokenType::And,
                            right: quantified,
                        })
                    } else {
                        // A descriptive standard ("faster than Quinn Quade's
                        // stamp", "faster than the cat that runs") becomes its own
                        // existential entity carrying its possessor/PP/relative
                        // clause, so it never collapses onto the subject; a bare
                        // name stays a constant.
                        let std_np = self.parse_noun_phrase(true)?;
                        let has_rel = self.check(&TokenType::Who) || self.check(&TokenType::That);
                        let is_desc = has_rel
                            || std_np.definiteness.is_some()
                            || !std_np.adjectives.is_empty()
                            || std_np.possessor.is_some()
                            || !std_np.pps.is_empty();
                        if is_desc {
                            let std_var = self.next_var_name();
                            let mut restr =
                                self.nominal_predication(Term::Variable(std_var), &std_np);
                            for pp in std_np.pps {
                                let pp_sub = self.substitute_pp_placeholder(pp, std_var);
                                restr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                    left: restr,
                                    op: TokenType::And,
                                    right: pp_sub,
                                });
                            }
                            if has_rel {
                                self.advance(); // "who" / "that"
                                let rel = self.parse_relative_clause(std_var)?;
                                restr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                    left: restr,
                                    op: TokenType::And,
                                    right: rel,
                                });
                            }
                            let comparison = self.ctx.exprs.alloc(LogicExpr::Comparative {
                                adjective: comp_adj,
                                subject: self.ctx.terms.alloc(subject_term.clone()),
                                object: self.ctx.terms.alloc(Term::Variable(std_var)),
                                difference: None,
                                relation: crate::ast::logic::ComparisonRelation::Greater,
                            });
                            let body = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                left: restr,
                                op: TokenType::And,
                                right: comparison,
                            });
                            let quantified = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                                kind: QuantifierKind::Existential,
                                variable: std_var,
                                body,
                                island_id: self.current_island,
                            });
                            self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                left: event,
                                op: TokenType::And,
                                right: quantified,
                            })
                        } else {
                            let comparison = self.ctx.exprs.alloc(LogicExpr::Comparative {
                                adjective: comp_adj,
                                subject: self.ctx.terms.alloc(subject_term.clone()),
                                object: self.ctx.terms.alloc(Term::Constant(std_np.noun)),
                                difference: None,
                                relation: crate::ast::logic::ComparisonRelation::Greater,
                            });
                            self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                left: event,
                                op: TokenType::And,
                                right: comparison,
                            })
                        }
                    };
                    return Ok(result);
                }
            }

            // Perception small clause ("saw her duck", "watched the bird fly"):
            // a perception verb takes "NP bare-VP" naming the PERCEIVED event.
            // Gated on an actual Verb token so a Noun-variant parse of the same
            // word yields the distinct NP-object reading instead.
            if crate::lexicon::is_perception_verb(&self.interner.resolve(verb).to_lowercase()) {
                let mut vp_idx = None;
                let mut i = self.current;
                while i < self.tokens.len()
                    && !matches!(
                        self.tokens[i].kind,
                        TokenType::Period | TokenType::EOF | TokenType::Comma
                    )
                {
                    // Mode-aware verb reading: under noun priority an
                    // Ambiguous token takes its noun reading, so the small
                    // clause does not fire and the NP-object parse runs.
                    let is_verb_reading = match &self.tokens[i].kind {
                        TokenType::Verb { .. } => true,
                        TokenType::Ambiguous { primary, .. } if !self.noun_priority_mode => {
                            matches!(**primary, TokenType::Verb { .. })
                        }
                        _ => false,
                    };
                    if is_verb_reading {
                        vp_idx = Some(i);
                    }
                    i += 1;
                }
                if let Some(vp_i) = vp_idx {
                    if vp_i > self.current {
                        let psubj = match self.tokens[vp_i - 1].kind.clone() {
                            TokenType::Noun(n) | TokenType::ProperName(n) => Some(n),
                            TokenType::Pronoun { .. } | TokenType::Ambiguous { .. } => {
                                let lx = self
                                    .interner
                                    .resolve(self.tokens[vp_i - 1].lexeme)
                                    .to_lowercase();
                                let cap = lx
                                    .chars()
                                    .next()
                                    .map(|c| c.to_uppercase().collect::<String>() + &lx[1..])
                                    .unwrap_or(lx);
                                Some(self.interner.intern(&cap))
                            }
                            _ => None,
                        };
                        if let Some(psubj) = psubj {
                            let inner_verb = match &self.tokens[vp_i].kind {
                                TokenType::Verb { lemma, .. } => *lemma,
                                TokenType::Ambiguous { primary, .. } => {
                                    if let TokenType::Verb { lemma, .. } = **primary {
                                        lemma
                                    } else {
                                        unreachable!("gated on verb reading")
                                    }
                                }
                                _ => unreachable!("gated on verb reading"),
                            };
                            self.current = vp_i + 1; // consume through the VP head
                            let perceived = self.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: inner_verb,
                                args: self.ctx.terms.alloc_slice([Term::Constant(psubj)]),
                                world: None,
                            });
                            let perceived_advs = self.collect_adverbs();
                            let perceived = if perceived_advs.is_empty() {
                                perceived
                            } else {
                                self.ctx.exprs.alloc(LogicExpr::Event {
                                    predicate: perceived,
                                    adverbs: self.ctx.syms.alloc_slice(perceived_advs),
                                })
                            };
                            let mut modifiers: Vec<Symbol> = Vec::new();
                            match verb_time {
                                Time::Past => modifiers.push(self.interner.intern("Past")),
                                Time::Future => modifiers.push(self.interner.intern("Future")),
                                _ => {}
                            }
                            let event_var = self.get_event_var();
                            let suppress_existential = self.drs.in_conditional_antecedent();
                            return Ok(self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(
                                NeoEventData {
                                    event_var,
                                    verb,
                                    roles: self.ctx.roles.alloc_slice(vec![
                                        (ThematicRole::Agent, subject_term.clone()),
                                        (ThematicRole::Theme, Term::Proposition(perceived)),
                                    ]),
                                    modifiers: self.ctx.syms.alloc_slice(modifiers),
                                    suppress_existential,
                                    world: None,
                                },
                            ))));
                        }
                    }
                }
            }

            // Check for embedded wh-clause: "I know who/what"
            if self.check_wh_word() {
                let wh_token = self.advance().kind.clone();

                let is_who = matches!(wh_token, TokenType::Who);
                let is_what = matches!(wh_token, TokenType::What);

                // Check for sluicing: wh-word followed by terminator
                let is_sluicing = self.is_at_end() ||
                    self.check(&TokenType::Period) ||
                    self.check(&TokenType::Comma);

                if is_sluicing {
                    if let Some(template) = self.last_event_template.clone() {
                        let wh_var = self.next_var_name();

                        let roles: Vec<_> = if is_who {
                            std::iter::once((ThematicRole::Agent, Term::Variable(wh_var)))
                                .chain(template.non_agent_roles.iter().cloned())
                                .collect()
                        } else if is_what {
                            vec![
                                (ThematicRole::Agent, subject_term.clone()),
                                (ThematicRole::Theme, Term::Variable(wh_var)),
                            ]
                        } else {
                            std::iter::once((ThematicRole::Agent, Term::Variable(wh_var)))
                                .chain(template.non_agent_roles.iter().cloned())
                                .collect()
                        };

                        let event_var = self.get_event_var();
                        let suppress_existential = self.drs.in_conditional_antecedent();
                        let reconstructed = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                            event_var,
                            verb: template.verb,
                            roles: self.ctx.roles.alloc_slice(roles),
                            modifiers: self.ctx.syms.alloc_slice(template.modifiers.clone()),
                            suppress_existential,
                            world: None,
                        })));

                        let question = self.ctx.exprs.alloc(LogicExpr::Question {
                            wh_variable: wh_var,
                            body: reconstructed,
                        });

                        let know_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                            event_var: self.get_event_var(),
                            verb,
                            roles: self.ctx.roles.alloc_slice(vec![
                                (ThematicRole::Agent, subject_term),
                                (ThematicRole::Theme, Term::Proposition(question)),
                            ]),
                            modifiers: self.ctx.syms.alloc_slice(vec![]),
                            suppress_existential,
                            world: None,
                        })));

                        return Ok(know_event);
                    }
                }

                // Non-sluicing: "I know who runs"
                let embedded = self.parse_embedded_wh_clause()?;
                let question = self.ctx.exprs.alloc(LogicExpr::Question {
                    wh_variable: self.interner.intern("x"),
                    body: embedded,
                });

                let suppress_existential = self.drs.in_conditional_antecedent();
                let know_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var: self.get_event_var(),
                    verb,
                    roles: self.ctx.roles.alloc_slice(vec![
                        (ThematicRole::Agent, subject_term),
                        (ThematicRole::Theme, Term::Proposition(question)),
                    ]),
                    modifiers: self.ctx.syms.alloc_slice(vec![]),
                    suppress_existential,
                    world: None,
                })));

                return Ok(know_event);
            }

            // Opaque attitude verbs take a finite clausal complement as a STRUCTURED
            // PROPOSITION (P3), not an extensional object: "John believes Mary left."
            // → Believe(John, ⟨Left(Mary)⟩). A pure-token lookahead detects an
            // embedded proper-name/pronoun subject directly followed by a verb
            // (optionally after the complementizer "that"); article-headed embedded
            // clauses ("a spy exists") are already handled downstream and untouched.
            if crate::lexicon::is_opaque_verb(&self.interner.resolve(verb).to_lowercase()) {
                let mut i = self.current;
                if i < self.tokens.len() && matches!(self.tokens[i].kind, TokenType::That) {
                    i += 1;
                }
                let subj_is_name_or_pronoun = i < self.tokens.len()
                    && matches!(
                        self.tokens[i].kind,
                        TokenType::ProperName(_) | TokenType::Pronoun { .. }
                    );
                let verb_follows = subj_is_name_or_pronoun
                    && i + 1 < self.tokens.len()
                    && matches!(
                        self.tokens[i + 1].kind,
                        TokenType::Verb { .. } | TokenType::Auxiliary(_)
                    );
                // Article-headed embedded subject with a finite clause:
                // "believes that THE TEACHER wants …". (Indefinite objects
                // without a following verb keep the de re/de dicto path.)
                let definite_np_clause = i + 2 < self.tokens.len()
                    && matches!(self.tokens[i].kind, TokenType::Article(_))
                    && matches!(self.tokens[i + 1].kind, TokenType::Noun(_))
                    && matches!(
                        self.tokens[i + 2].kind,
                        TokenType::Verb { .. } | TokenType::Auxiliary(_)
                    );
                if verb_follows || definite_np_clause {
                    if self.check(&TokenType::That) {
                        self.advance();
                    }
                    let embedded_subject = match self.peek().kind {
                        TokenType::ProperName(s) => {
                            self.advance();
                            s
                        }
                        TokenType::Pronoun { gender, number, .. } => {
                            self.advance();
                            match self.resolve_pronoun(gender, number)? {
                                super::ResolvedPronoun::Variable(s)
                                | super::ResolvedPronoun::Constant(s) => s,
                            }
                        }
                        TokenType::Article(_) => {
                            let np = self.parse_noun_phrase(false)?;
                            np.noun
                        }
                        _ => unreachable!("guarded by subj_is_name_or_pronoun"),
                    };
                    let embedded_pred = self.parse_predicate_with_subject(embedded_subject)?;
                    let embedded_term = Term::Proposition(embedded_pred);
                    let main_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: verb,
                        args: self
                            .ctx
                            .terms
                            .alloc_slice([subject_term.clone(), embedded_term]),
                        world: None,
                    });
                    let effective_time = self.pending_time.take().unwrap_or(verb_time);
                    return Ok(if effective_time == Time::Past {
                        self.ctx.exprs.alloc(LogicExpr::Temporal {
                            operator: TemporalOperator::Past,
                            body: main_pred,
                        })
                    } else {
                        main_pred
                    });
                }
            }

            let mut object_term: Option<Term<'a>> = None;
            let mut second_object_term: Option<Term<'a>> = None;
            // "X has Y as [its] ROLE" — a predicative secondary naming the role Y
            // fills (set after the object is parsed; conjoined as Role(Y) below).
            let mut as_role: Option<Symbol> = None;
            // A filler-gap object licenses a stranded preposition ("Who did John talk to?").
            let mut gap_object = false;
            let mut object_pps: &[&LogicExpr<'a>] = &[];  // PPs attached to object NP (for NP-attachment mode)
            let mut object_adjectives: &[Symbol] = &[];   // adjectives on a definite/constant object ("the GREEN shirt")
            if self.check(&TokenType::Reflexive) {
                self.advance();
                // The reflexive binds the subject TERM, preserving its
                // variable-ness under a quantified subject ("Every man loves
                // himself." → Theme(e, x), not a constant).
                let term = subject_term.clone();
                object_term = Some(term.clone());
                args.push(term);
            } else if self.check_pronoun()
                && !(self.check_possessive_pronoun()
                    && match self.tokens.get(self.current + 1).map(|t| t.kind.clone()) {
                        Some(TokenType::Noun(_)) => true,
                        // Under noun priority an Ambiguous next token reads as
                        // a noun, so "her duck" is a possessive NP object.
                        Some(TokenType::Ambiguous { .. }) => self.noun_priority_mode,
                        _ => false,
                    })
            {
                let token = self.advance().clone();
                let (gender, number) = match &token.kind {
                    TokenType::Pronoun { gender, number, .. } => (*gender, *number),
                    TokenType::Ambiguous { primary, alternatives } => {
                        if let TokenType::Pronoun { gender, number, .. } = **primary {
                            (gender, number)
                        } else {
                            alternatives.iter().find_map(|t| {
                                if let TokenType::Pronoun { gender, number, .. } = t {
                                    Some((*gender, *number))
                                } else {
                                    None
                                }
                            }).unwrap_or((Gender::Unknown, Number::Singular))
                        }
                    }
                    _ => (Gender::Unknown, Number::Singular),
                };

                // Person deictics (§8.4) resolve to the discourse roles in any
                // position: object "you" → Addressee, "me" → Speaker.
                let plex = self.interner.resolve(token.lexeme).to_lowercase();
                let term = match plex.as_str() {
                    "you" | "yourself" => Term::Constant(self.interner.intern("Addressee")),
                    "me" | "myself" | "i" => Term::Constant(self.interner.intern("Speaker")),
                    // A donkey antecedent (indefinite from a quantifier's
                    // restriction) outranks discourse resolution: "Every man
                    // who owns a book reads it." → Theme(e, y).
                    _ => match self.resolve_donkey_pronoun(gender) {
                        Some(donkey_var) => Term::Variable(donkey_var),
                        None => match self.resolve_pronoun(gender, number)? {
                            super::ResolvedPronoun::Variable(s) => Term::Variable(s),
                            super::ResolvedPronoun::Constant(s) => Term::Constant(s),
                        },
                    },
                };
                object_term = Some(term);
                args.push(term);

                let verb_str = self.interner.resolve(verb);
                if Lexer::is_ditransitive_verb(verb_str)
                    && (self.check_content_word() || self.check_article())
                {
                    let second_np = self.parse_noun_phrase(false)?;
                    let second_term = Term::Constant(second_np.noun);
                    second_object_term = Some(second_term);
                    args.push(second_term);
                }
            } else if self.peek_definite_reduced_relative_object() {
                // A definite object whose head carries a REDUCED OBJECT RELATIVE
                // ("Determine the friend Simon went with"). Parse the WHOLE NP —
                // article included — so `parse_noun_phrase` runs its reduced-relative
                // machinery and returns the relative as a `_PP_SELF_` PP over the
                // head. Pre-consuming the article (the generic definite path) would
                // hide the determiner and strand the relative's verb. The head noun
                // and the relative both survive: the PP is rebound to the object
                // constant by the shared object-PP application below. Parsed GREEDILY
                // so the reduced relative — a genuine head restrictor, gated above the
                // non-greedy PP cutoff — attaches; the precise dispatch guard keeps
                // this from over-consuming any other construction.
                self.nominal_np_context = true;
                let object_np_result = self.parse_noun_phrase(true);
                self.nominal_np_context = false;
                let object_np = object_np_result?;

                let obj_gender = Self::infer_noun_gender(self.interner.resolve(object_np.noun));
                let obj_number = if Self::is_plural_noun(self.interner.resolve(object_np.noun)) {
                    Number::Plural
                } else {
                    Number::Singular
                };
                self.drs.introduce_referent_with_source(
                    object_np.noun,
                    object_np.noun,
                    obj_gender,
                    obj_number,
                    ReferentSource::MainClause,
                );

                let term = Term::Constant(object_np.noun);
                object_term = Some(term);
                object_pps = object_np.pps;
                object_adjectives = object_np.adjectives;
                args.push(term);
            } else if self.counting_np_lookahead().is_some()
                || self.check_quantifier()
                || self.check_article()
                || self.check_possessive_pronoun()
            {
                let obj_quantifier = if let Some(n) = self.counting_np_lookahead() {
                    // A digit-led counting NP object ("saw 6 brown manatees"):
                    // consume the integer and quantify the remaining
                    // "(adjective)+ noun" as ∃=n, reusing the canonical
                    // quantified-object construction below. Without this the
                    // adjective is mis-read as a measure unit (→ a bogus
                    // Recipient role and a dropped count).
                    self.advance();
                    Some(TokenType::Cardinal(n))
                } else if self.check_possessive_pronoun() {
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

                // The quantifier/article/cardinal just established this as an NP,
                // so a verb-word head after an adjective is a deverbal noun.
                self.nominal_np_context = true;
                let object_np_result = self.parse_noun_phrase(false);
                self.nominal_np_context = false;
                let object_np = object_np_result?;

                if let Some(obj_q) = obj_quantifier {
                    let obj_var = self.next_var_name();

                    // Introduce object referent in DRS for cross-sentence anaphora
                    let obj_gender = Self::infer_noun_gender(self.interner.resolve(object_np.noun));
                    let obj_number = if Self::is_plural_noun(self.interner.resolve(object_np.noun)) {
                        Number::Plural
                    } else {
                        Number::Singular
                    };
                    // Definite descriptions presuppose existence, so they should be globally accessible
                    if object_np.definiteness == Some(Definiteness::Definite) {
                        self.drs.introduce_referent_with_source(obj_var, object_np.noun, obj_gender, obj_number, ReferentSource::MainClause);
                    } else {
                        self.drs.introduce_referent(obj_var, object_np.noun, obj_gender, obj_number);
                    }

                    let mut obj_restriction: &'a LogicExpr<'a> =
                        self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: object_np.noun,
                            args: self.ctx.terms.alloc_slice([Term::Variable(obj_var)]),
                            world: None,
                        });
                    // A quantified object's own restrictors — adjectives ("a RED
                    // book") and PPs ("a maximum range OF 475 ft" → Range(o) ∧
                    // Of(o, 475 ft)) — must survive; dropping them is meaning loss.
                    for &adj in object_np.adjectives {
                        let adj_pred = self.adjective_restriction(adj, obj_var, object_np.noun);
                        obj_restriction = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: obj_restriction,
                            op: TokenType::And,
                            right: adj_pred,
                        });
                    }
                    for pp in object_np.pps {
                        let pp_sub = self.substitute_pp_placeholder(pp, obj_var);
                        obj_restriction = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                            left: obj_restriction,
                            op: TokenType::And,
                            right: pp_sub,
                        });
                    }

                    // Continuations inside the object quantifier's scope: a
                    // second object ("gave some student a book"), a recipient
                    // ("gave a book to Mary", "… to some teacher"), or an
                    // object-control infinitive ("caused all flowers to bloom").
                    let verb_str = self.interner.resolve(verb).to_string();
                    let mut second_object: Option<Term<'a>> = None;
                    let mut recipient: Option<Term<'a>> = None;
                    let mut recipient_quant: Option<(TokenType, Symbol, Symbol)> = None;
                    let mut control_infinitive: Option<Symbol> = None;

                    if Lexer::is_ditransitive_verb(&verb_str)
                        && (self.check_content_word() || self.check_article())
                    {
                        let second_np = self.parse_noun_phrase(false)?;
                        second_object = Some(Term::Constant(second_np.noun));
                    } else if self.check_to_marker() {
                        let after_to = self.tokens.get(self.current + 1).map(|t| t.kind.clone());
                        match after_to {
                            Some(TokenType::Verb { lemma, .. }) => {
                                self.advance(); // to
                                self.advance(); // infinitive verb
                                control_infinitive = Some(lemma);
                            }
                            // After a preposition the lexer may classify the
                            // infinitive as a noun ("to bloom"); the lexicon
                            // recovers the verb reading.
                            Some(TokenType::Noun(word))
                                if crate::lexicon::lookup_verb_db(
                                    &self.interner.resolve(word).to_lowercase(),
                                )
                                .is_some() =>
                            {
                                let lemma_str = crate::lexicon::lookup_verb_db(
                                    &self.interner.resolve(word).to_lowercase(),
                                )
                                .map(|m| m.lemma)
                                .unwrap();
                                self.advance(); // to
                                self.advance(); // infinitive verb (noun-classified)
                                control_infinitive = Some(self.interner.intern(lemma_str));
                            }
                            Some(kind)
                                if Lexer::is_ditransitive_verb(&verb_str)
                                    && matches!(
                                        kind,
                                        TokenType::All
                                            | TokenType::Some
                                            | TokenType::No
                                            | TokenType::Most
                                            | TokenType::Few
                                            | TokenType::Many
                                            | TokenType::Cardinal(_)
                                            | TokenType::AtLeast(_)
                                            | TokenType::AtMost(_)
                                    ) =>
                            {
                                self.advance(); // to
                                let r_quant = self.advance().kind.clone();
                                let r_np = self.parse_noun_phrase(false)?;
                                let r_var = self.next_var_name();
                                recipient_quant = Some((r_quant, r_var, r_np.noun));
                            }
                            Some(kind)
                                if Lexer::is_ditransitive_verb(&verb_str)
                                    && matches!(
                                        kind,
                                        TokenType::ProperName(_)
                                            | TokenType::Noun(_)
                                            | TokenType::Article(_)
                                    ) =>
                            {
                                self.advance(); // to
                                let r_np = self.parse_noun_phrase(false)?;
                                recipient = Some(Term::Constant(r_np.noun));
                            }
                            _ => {}
                        }
                    }

                    let event_var = self.get_event_var();
                    let mut modifiers = self.collect_adverbs();
                    let effective_time = self.pending_time.take().unwrap_or(verb_time);
                    match effective_time {
                        Time::Past => modifiers.push(self.interner.intern("Past")),
                        Time::Future => modifiers.push(self.interner.intern("Future")),
                        _ => {}
                    }

                    let mut roles = vec![(ThematicRole::Agent, subject_term.clone())];
                    if let Some(second) = second_object {
                        roles.push((ThematicRole::Recipient, Term::Variable(obj_var)));
                        roles.push((ThematicRole::Theme, second));
                    } else {
                        roles.push((ThematicRole::Theme, Term::Variable(obj_var)));
                        if let Some(r) = recipient {
                            roles.push((ThematicRole::Recipient, r));
                        } else if let Some((_, r_var, _)) = recipient_quant {
                            roles.push((ThematicRole::Recipient, Term::Variable(r_var)));
                        }
                    }

                    let suppress_existential = self.drs.in_conditional_antecedent();
                    let neo_event = if let Some(inf) = control_infinitive {
                        let inf_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: inf,
                            args: self.ctx.terms.alloc_slice([Term::Variable(obj_var)]),
                            world: None,
                        });
                        let control = self.ctx.exprs.alloc(LogicExpr::Control {
                            verb,
                            subject: self.ctx.terms.alloc(subject_term.clone()),
                            object: Some(&*self.ctx.terms.alloc(Term::Variable(obj_var))),
                            infinitive: inf_pred,
                        });
                        match effective_time {
                            Time::Past => &*self.ctx.exprs.alloc(LogicExpr::Temporal {
                                operator: TemporalOperator::Past,
                                body: control,
                            }),
                            Time::Future => &*self.ctx.exprs.alloc(LogicExpr::Temporal {
                                operator: TemporalOperator::Future,
                                body: control,
                            }),
                            _ => control,
                        }
                    } else {
                        let plain = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                            event_var,
                            verb,
                            roles: self.ctx.roles.alloc_slice(roles),
                            modifiers: self.ctx.syms.alloc_slice(modifiers),
                            suppress_existential,
                            world: None,
                        })));
                        if let Some((r_quant, r_var, r_noun)) = recipient_quant {
                            let r_restriction = self.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: r_noun,
                                args: self.ctx.terms.alloc_slice([Term::Variable(r_var)]),
                                world: None,
                            });
                            let r_kind = match r_quant {
                                TokenType::All => QuantifierKind::Universal,
                                TokenType::Most => QuantifierKind::Most,
                                TokenType::Few => QuantifierKind::Few,
                                TokenType::Many => QuantifierKind::Many,
                                TokenType::Cardinal(n) => QuantifierKind::Cardinal(n),
                                TokenType::AtLeast(n) => QuantifierKind::AtLeast(n),
                                TokenType::AtMost(n) => QuantifierKind::AtMost(n),
                                _ => QuantifierKind::Existential,
                            };
                            let r_body = if matches!(r_kind, QuantifierKind::Universal) {
                                self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                    left: r_restriction,
                                    op: TokenType::Implies,
                                    right: plain,
                                })
                            } else {
                                self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                    left: r_restriction,
                                    op: TokenType::And,
                                    right: plain,
                                })
                            };
                            &*self.ctx.exprs.alloc(LogicExpr::Quantifier {
                                kind: r_kind,
                                variable: r_var,
                                body: r_body,
                                island_id: self.current_island,
                            })
                        } else {
                            plain
                        }
                    };

                    // Trailing event-PP adjuncts ("takes a holiday WITH a friend TO
                    // some location") attach to the event INSIDE the object's
                    // existential scope — the non-quantified path does this inline.
                    let neo_event = self.attach_trailing_event_pps(neo_event, event_var)?;

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
                            right: neo_event,
                        }),
                        TokenType::No => {
                            let neg = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                                op: TokenType::Not,
                                operand: neo_event,
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
                            right: neo_event,
                        }),
                    };

                    return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
                        kind: obj_kind,
                        variable: obj_var,
                        body: obj_body,
                        island_id: self.current_island,
                    }));
                } else {
                    // Definite object NP (e.g., "the house")
                    // Introduce to DRS for cross-sentence bridging anaphora
                    // E.g., "John entered the house. The door was open." - door bridges to house
                    if object_np.definiteness == Some(Definiteness::Definite) {
                        let obj_gender = Self::infer_noun_gender(self.interner.resolve(object_np.noun));
                        let obj_number = if Self::is_plural_noun(self.interner.resolve(object_np.noun)) {
                            Number::Plural
                        } else {
                            Number::Singular
                        };
                        // Definite descriptions presuppose existence, so they should be globally accessible
                        self.drs.introduce_referent_with_source(object_np.noun, object_np.noun, obj_gender, obj_number, ReferentSource::MainClause);
                    }

                    let term = Term::Constant(object_np.noun);
                    object_term = Some(term);
                    // Store the definite object's adjectives + PPs so they are
                    // predicated of the object ("ate the RED apple" → Red(Apple));
                    // dropping them is a meaning-loss parse.
                    object_pps = object_np.pps;
                    object_adjectives = object_np.adjectives;
                    args.push(term);

                    // Ditransitive with a DEFINITE indirect object: "gave the
                    // winner the prize" — the definite IO took this non-quantified
                    // branch (a definite article carries no obj_quantifier), so the
                    // direct object must still be picked up here, mirroring the
                    // proper-name/quantified IO paths. The double-object builder
                    // then assigns Recipient(IO) ∧ Theme(DO); without this the DO
                    // strands as a trailing token.
                    let verb_str = self.interner.resolve(verb);
                    if Lexer::is_ditransitive_verb(verb_str)
                        && (self.check_content_word() || self.check_article())
                    {
                        let second_np = self.parse_noun_phrase(false)?;
                        let second_term = Term::Constant(second_np.noun);
                        second_object_term = Some(second_term);
                        args.push(second_term);
                    }
                }
            } else if self.check_focus() {
                let focus_kind = if let TokenType::Focus(k) = self.advance().kind {
                    k
                } else {
                    FocusKind::Only
                };

                let event_var = self.get_event_var();
                let mut modifiers = self.collect_adverbs();
                let effective_time = self.pending_time.take().unwrap_or(verb_time);
                match effective_time {
                    Time::Past => modifiers.push(self.interner.intern("Past")),
                    Time::Future => modifiers.push(self.interner.intern("Future")),
                    _ => {}
                }

                if self.check_preposition() {
                    let prep_token = self.advance().clone();
                    let prep_name = if let TokenType::Preposition(sym) = prep_token.kind {
                        sym
                    } else {
                        self.interner.intern("to")
                    };
                    let pp_obj = self.parse_noun_phrase(false)?;
                    let pp_obj_term = Term::Constant(pp_obj.noun);

                    let roles = vec![(ThematicRole::Agent, subject_term)];
                    let suppress_existential = self.drs.in_conditional_antecedent();
                    let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                        event_var,
                        verb,
                        roles: self.ctx.roles.alloc_slice(roles),
                        modifiers: self.ctx.syms.alloc_slice(modifiers),
                        suppress_existential,
                        world: None,
                    })));

                    let pp_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: prep_name,
                        args: self.ctx.terms.alloc_slice([Term::Variable(event_var), pp_obj_term]),
                        world: None,
                    });

                    let with_pp = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: neo_event,
                        op: TokenType::And,
                        right: pp_pred,
                    });

                    let focused_ref = self.ctx.terms.alloc(pp_obj_term);
                    return Ok(self.ctx.exprs.alloc(LogicExpr::Focus {
                        kind: focus_kind,
                        focused: focused_ref,
                        scope: with_pp,
                    }));
                }

                let focused_np = self.parse_noun_phrase(false)?;
                let focused_term = Term::Constant(focused_np.noun);
                args.push(focused_term);

                let roles = vec![
                    (ThematicRole::Agent, subject_term),
                    (ThematicRole::Theme, focused_term),
                ];

                let suppress_existential = self.drs.in_conditional_antecedent();
                let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var,
                    verb,
                    roles: self.ctx.roles.alloc_slice(roles),
                    modifiers: self.ctx.syms.alloc_slice(modifiers),
                    suppress_existential,
                    world: None,
                })));

                let focused_ref = self.ctx.terms.alloc(focused_term);
                return Ok(self.ctx.exprs.alloc(LogicExpr::Focus {
                    kind: focus_kind,
                    focused: focused_ref,
                    scope: neo_event,
                }));
            } else if self.check(&TokenType::Either) {
                // "danced either the hustle or the lindy" → Dance(S,Hustle) ∨ Dance(S,Lindy)
                self.advance(); // consume "either"
                let np1 = self.parse_noun_phrase(true)?;
                if self.check(&TokenType::Or) {
                    self.advance(); // consume "or"
                    let np2 = self.parse_noun_phrase(true)?;
                    let effective_time = self.pending_time.take().unwrap_or(verb_time);
                    let placeholder = self.interner.intern("_PP_SELF_");
                    let possesses = self.interner.intern("Possesses");
                    // Build a disjunct's predication, KEEPING the object NP's
                    // possessor and PPs ("either the hustle from Spain or …"
                    // must not drop "from Spain"; "either Tara's routine or …"
                    // must not drop the possessor).
                    let mut build = |p: &mut Self, np: &NounPhrase<'a>| -> &'a LogicExpr<'a> {
                        let obj = Term::Constant(np.noun);
                        let mut pred: &'a LogicExpr<'a> = p.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: verb,
                            args: p.ctx.terms.alloc_slice([subject_term, obj]),
                            world: None,
                        });
                        if let Some(possessor) = np.possessor {
                            let poss = p.ctx.exprs.alloc(LogicExpr::Predicate {
                                name: possesses,
                                args: p
                                    .ctx
                                    .terms
                                    .alloc_slice([Term::Constant(possessor.noun), obj]),
                                world: None,
                            });
                            pred = p.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                left: pred,
                                op: TokenType::And,
                                right: poss,
                            });
                        }
                        for pp in np.pps {
                            let pp_sub = match pp {
                                LogicExpr::Predicate { name, args, world } => {
                                    let new_args: Vec<Term<'a>> = args
                                        .iter()
                                        .map(|a| match a {
                                            Term::Variable(v) if *v == placeholder => obj,
                                            other => *other,
                                        })
                                        .collect();
                                    p.ctx.exprs.alloc(LogicExpr::Predicate {
                                        name: *name,
                                        args: p.ctx.terms.alloc_slice(new_args),
                                        world: *world,
                                    })
                                }
                                other => *other,
                            };
                            pred = p.ctx.exprs.alloc(LogicExpr::BinaryOp {
                                left: pred,
                                op: TokenType::And,
                                right: pp_sub,
                            });
                        }
                        pred
                    };
                    let pred1 = build(self, &np1);
                    let pred2 = build(self, &np2);
                    let mut wrap_time = |p: &mut Self, e: &'a LogicExpr<'a>| -> &'a LogicExpr<'a> {
                        if effective_time == Time::Past {
                            p.ctx.exprs.alloc(LogicExpr::Temporal {
                                operator: TemporalOperator::Past,
                                body: e,
                            })
                        } else {
                            e
                        }
                    };
                    let pred1 = wrap_time(self, pred1);
                    let pred2 = wrap_time(self, pred2);
                    return Ok(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: pred1,
                        op: TokenType::Or,
                        right: pred2,
                    }));
                }
                // No "or" — fall through with np1 as plain object
                let term = Term::Constant(np1.noun);
                object_term = Some(term);
                args.push(term);
            } else if self.check_number() {
                let measure = self.parse_measure_phrase()?;
                // The measured quantity is the verb's Theme ("scored 190 points"
                // → Theme(e, 190 points)); without this the count is parsed but
                // never reaches a thematic role and the number is lost.
                object_term = Some(*measure);
                if self.check_content_word() {
                    let noun_sym = self.consume_content_word()?;
                    args.push(*measure);
                    args.push(Term::Constant(noun_sym));
                } else {
                    args.push(*measure);
                }
            } else if self.check_content_word() {
                let potential_object = self.parse_noun_phrase(false)?;
                // Store the object's adjectives + PPs for NP-attachment mode
                object_pps = potential_object.pps;
                object_adjectives = potential_object.adjectives;

                // A finite clausal complement (the NP is followed by a verb) is taken
                // as a structured proposition (P3) when the matrix verb is an opaque
                // attitude verb ("John believes Mary left." → Believe(John, ⟨Left(Mary)⟩))
                // or in a filler-gap context. The complement keeps its own structure
                // so co-intensional complements stay distinct and substitution into it
                // is blocked.
                let verb_is_opaque =
                    crate::lexicon::is_opaque_verb(&self.interner.resolve(verb).to_lowercase());
                if self.check_verb() && (self.filler_gap.is_some() || verb_is_opaque) {
                    let embedded_subject = potential_object.noun;
                    let embedded_pred = self.parse_predicate_with_subject(embedded_subject)?;

                    let embedded_term = Term::Proposition(embedded_pred);
                    let main_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: verb,
                        args: self.ctx.terms.alloc_slice([subject_term, embedded_term]),
                        world: None,
                    });

                    let effective_time = self.pending_time.take().unwrap_or(verb_time);
                    return Ok(if effective_time == Time::Past {
                        self.ctx.exprs.alloc(LogicExpr::Temporal {
                            operator: TemporalOperator::Past,
                            body: main_pred,
                        })
                    } else {
                        main_pred
                    });
                }

                // Collect all objects for potential "respectively" handling
                let mut all_objects: Vec<Symbol> = vec![potential_object.noun];

                // Check for coordinated objects: "Tom and Jerry and Bob"
                while self.check(&TokenType::And) {
                    let saved = self.current;
                    self.advance(); // consume "and"
                    if self.check_content_word() || self.check_article() {
                        let next_obj = match self.parse_noun_phrase(false) {
                            Ok(np) => np,
                            Err(_) => {
                                self.current = saved;
                                break;
                            }
                        };
                        all_objects.push(next_obj.noun);
                    } else {
                        self.current = saved;
                        break;
                    }
                }

                // Check for "respectively" with single subject
                if self.check(&TokenType::Respectively) {
                    let respectively_span = self.peek().span;
                    // Single subject with multiple objects + respectively = error
                    if all_objects.len() > 1 {
                        return Err(ParseError {
                            kind: ParseErrorKind::RespectivelyLengthMismatch {
                                subject_count: 1,
                                object_count: all_objects.len(),
                            },
                            span: respectively_span,
                        });
                    }
                    // Single subject, single object + respectively is valid (trivially pairwise)
                    self.advance(); // consume "respectively"
                }

                // Use the first object (or only object) for normal processing
                let term = Term::Constant(all_objects[0]);
                object_term = Some(term);
                args.push(term);

                // For multiple objects without "respectively", use group semantics
                if all_objects.len() > 1 {
                    let obj_members: Vec<Term<'a>> = all_objects.iter()
                        .map(|o| Term::Constant(*o))
                        .collect();
                    let obj_group = Term::Group(self.ctx.terms.alloc_slice(obj_members));
                    // Replace the single object with the group — both in the
                    // predicate args AND as the Theme term, so every coordinate
                    // survives the neo-event role assignment (the Theme reads
                    // `object_term`; leaving it on the first member silently drops
                    // the rest, e.g. "year" in "the activity, state and year").
                    args.pop();
                    args.push(obj_group);
                    object_term = Some(obj_group);
                }

                let verb_str = self.interner.resolve(verb);
                if Lexer::is_ditransitive_verb(verb_str)
                    && (self.check_content_word() || self.check_article())
                {
                    let second_np = self.parse_noun_phrase(false)?;
                    let second_term = Term::Constant(second_np.noun);
                    second_object_term = Some(second_term);
                    args.push(second_term);
                }
            } else if self.filler_gap.is_some() && !self.check_content_word() && !self.check_pronoun()
            {
                let gap_var = self.filler_gap.take().unwrap();
                let term = Term::Variable(gap_var);
                object_term = Some(term);
                args.push(term);
                gap_object = true;
            }

            let unknown = self.interner.intern("?");
            let mut pp_predicates: Vec<&'a LogicExpr<'a>> = Vec::new();

            // Check for distanced phrasal verb particle: "gave the book up"
            if let TokenType::Particle(particle_sym) = self.peek().kind {
                let verb_str = self.interner.resolve(verb).to_lowercase();
                let particle_str = self.interner.resolve(particle_sym).to_lowercase();
                if let Some((phrasal_lemma, _class)) = crate::lexicon::lookup_phrasal_verb(&verb_str, &particle_str) {
                    self.advance(); // consume the particle
                    verb = self.interner.intern(phrasal_lemma);
                } else {
                    // A particle with no phrasal-verb table entry ("came OUT",
                    // "went UP") — keep it as a particle predicate over the event so
                    // a trailing PP still attaches ("came out IN 1995"). The
                    // clause-final case is handled in the PP loop; this covers the
                    // particle-then-PP case it misses.
                    self.advance(); // consume the particle
                    let event_sym = self.get_event_var();
                    let cap = {
                        let p = self.interner.resolve(particle_sym);
                        let mut chs = p.chars();
                        match chs.next() {
                            Some(f) => f.to_uppercase().collect::<String>() + chs.as_str(),
                            None => String::new(),
                        }
                    };
                    pp_predicates.push(self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: self.interner.intern(&cap),
                        args: self.ctx.terms.alloc_slice([Term::Variable(event_sym)]),
                        world: None,
                    }));
                }
            }
            while self.check_preposition() || self.check_to() {
                // "within N cycles" is a temporal bound, not a PP — leave for try_wrap_bounded_delay
                if self.check_preposition_is("within") && self.current + 1 < self.tokens.len()
                    && matches!(self.tokens[self.current + 1].kind, TokenType::Cardinal(_) | TokenType::Number(_))
                {
                    break;
                }
                let prep_token = self.advance().clone();
                let prep_name = if let TokenType::Preposition(sym) = prep_token.kind {
                    sym
                } else if matches!(prep_token.kind, TokenType::To) {
                    self.interner.intern("To")
                } else {
                    continue;
                };

                let pp_obj_term = if self.check(&TokenType::Reflexive) {
                    self.advance();
                    Term::Constant(subject_symbol)
                } else if self.check_pronoun() {
                    let token = self.advance().clone();
                    let (gender, number) = match &token.kind {
                        TokenType::Pronoun { gender, number, .. } => (*gender, *number),
                        TokenType::Ambiguous { primary, alternatives } => {
                            if let TokenType::Pronoun { gender, number, .. } = **primary {
                                (gender, number)
                            } else {
                                alternatives.iter().find_map(|t| {
                                    if let TokenType::Pronoun { gender, number, .. } = t {
                                        Some((*gender, *number))
                                    } else {
                                        None
                                    }
                                }).unwrap_or((Gender::Unknown, Number::Singular))
                            }
                        }
                        _ => (Gender::Unknown, Number::Singular),
                    };
                    let resolved = self.resolve_pronoun(gender, number)?;
                    match resolved {
                        super::ResolvedPronoun::Variable(s) => Term::Variable(s),
                        super::ResolvedPronoun::Constant(s) => Term::Constant(s),
                    }
                } else if self.check_content_word() || self.check_article() {
                    let prep_obj = self.parse_noun_phrase(false)?;
                    Term::Constant(prep_obj.noun)
                } else if self.check_number() {
                    // "N unit NOUN" is a measure-premodified noun ("brew with 190
                    // degree WATER") — ONE folded entity, not a measure with the
                    // head stranded. A noun/ambiguous head after the unit signals
                    // it; else the PP object is the bare measure ("sold for $105").
                    let premodified = matches!(
                        self.tokens.get(self.current + 2).map(|t| &t.kind),
                        Some(TokenType::Noun(_)) | Some(TokenType::Ambiguous { .. })
                    );
                    if premodified {
                        let saved_ctx = self.nominal_np_context;
                        self.nominal_np_context = true;
                        let r = self.parse_noun_phrase(false);
                        self.nominal_np_context = saved_ctx;
                        Term::Constant(r?.noun)
                    } else {
                        *self.parse_measure_phrase()?
                    }
                } else if gap_object {
                    // Preposition stranding: the object position was a wh-gap,
                    // so the bare preposition is licensed ("Who did John talk to?").
                    continue;
                } else if self.at_clause_boundary()
                    && crate::lexicon::is_particle(
                        &self.interner.resolve(prep_name).to_lowercase(),
                    )
                {
                    // A clause-final object-less PARTICLE preposition is an
                    // intransitive directional ("walked in", "sat down") — a
                    // lexically listed class; "of"/"to" cannot end a clause.
                    let event_sym = self.get_event_var();
                    pp_predicates.push(self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: prep_name,
                        args: self.ctx.terms.alloc_slice([Term::Variable(event_sym)]),
                        world: None,
                    }));
                    continue;
                } else {
                    // A mid-clause preposition with no object is not a PP —
                    // hand it back so the sentence-level parse reports it
                    // instead of silently dropping it.
                    self.current -= 1;
                    break;
                };

                if self.pp_attach_to_noun {
                    if let Some(obj) = object_term {
                        let pp_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                            name: prep_name,
                            args: self.ctx.terms.alloc_slice([obj, pp_obj_term]),
                            world: None,
                        });
                        pp_predicates.push(pp_pred);
                    } else {
                        args.push(pp_obj_term);
                    }
                } else {
                    let event_sym = self.get_event_var();
                    let pp_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: prep_name,
                        args: self
                            .ctx
                            .terms
                            .alloc_slice([Term::Variable(event_sym), pp_obj_term]),
                        world: None,
                    });
                    pp_predicates.push(pp_pred);
                }
            }

            if self.check(&TokenType::That) || self.check(&TokenType::Who) {
                self.advance();
                let rel_var = self.next_var_name();
                let rel_pred = self.parse_relative_clause(rel_var)?;
                pp_predicates.push(rel_pred);
            }

            // "X has Y as [its/his/her] ROLE" — a predicative secondary on the
            // object: Y fills ROLE for the subject ("the city has Al Acosta as its
            // mayor" → Have(City, Al_Acosta) ∧ Mayor(Al_Acosta)). The Have-link
            // already binds Y to the subject, so the possessive determiner is
            // redundant and dropped. "as" lexes as a Noun (function-word fallback),
            // hence the lexeme test; the noun-compound loop leaves it unbundled.
            if object_term.is_some()
                && matches!(self.peek().kind, TokenType::Noun(_))
                && self.interner.resolve(self.peek().lexeme).eq_ignore_ascii_case("as")
            {
                self.advance(); // consume "as"
                if self.check_possessive_pronoun() {
                    self.advance(); // consume "its" / "his" / "her"
                }
                if self.check_content_word() {
                    as_role = Some(self.consume_content_word()?);
                }
            }

            let mut modifiers = self.collect_adverbs();

            let effective_time = self.pending_time.take().unwrap_or(verb_time);
            match effective_time {
                Time::Past => modifiers.push(self.interner.intern("Past")),
                Time::Future => modifiers.push(self.interner.intern("Future")),
                _ => {}
            }

            if verb_aspect == Aspect::Progressive {
                modifiers.push(self.interner.intern("Progressive"));
            } else if verb_aspect == Aspect::Perfect {
                modifiers.push(self.interner.intern("Perfect"));
            }

            let mut roles: Vec<(ThematicRole, Term<'a>)> = Vec::new();

            // Check if verb is unaccusative (intransitive subject is Theme, not Agent)
            let verb_str = self.interner.resolve(verb).to_lowercase();
            let is_unaccusative = crate::lexicon::lookup_verb_db(&verb_str)
                .map(|meta| meta.features.contains(&crate::lexicon::Feature::Unaccusative))
                .unwrap_or(false);

            // Unaccusative verbs used intransitively: subject is Theme
            // E.g., "The alarm triggers" → Theme(e, Alarm), not Agent(e, Alarm)
            let has_object = object_term.is_some() || second_object_term.is_some();
            let subject_role = if is_unaccusative && !has_object {
                ThematicRole::Theme
            } else {
                ThematicRole::Agent
            };

            roles.push((subject_role, subject_term));
            if let Some(second_obj) = second_object_term {
                if let Some(first_obj) = object_term {
                    roles.push((ThematicRole::Recipient, first_obj));
                }
                roles.push((ThematicRole::Theme, second_obj));
            } else if let Some(obj) = object_term {
                roles.push((ThematicRole::Theme, obj));
            }

            let event_var = self.get_event_var();
            let suppress_existential = self.drs.in_conditional_antecedent();
            if suppress_existential {
                let event_class = self.interner.intern("Event");
                self.drs.introduce_referent(event_var, event_class, Gender::Neuter, Number::Singular);
            }
            let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                event_var,
                verb,
                roles: self.ctx.roles.alloc_slice(roles.clone()),
                modifiers: self.ctx.syms.alloc_slice(modifiers.clone()),
                suppress_existential,
                world: None,
            })));

            // Capture template for ellipsis reconstruction
            self.capture_event_template(verb, &roles, &modifiers);

            let with_pps = if pp_predicates.is_empty() {
                neo_event
            } else {
                let mut combined = neo_event;
                for pp in pp_predicates {
                    combined = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: combined,
                        op: TokenType::And,
                        right: pp,
                    });
                }
                combined
            };

            // Include PPs attached to object NP (for NP-attachment mode)
            // These have _PP_SELF_ placeholder that needs to be replaced with the object term
            let with_object_pps = if object_pps.is_empty() {
                with_pps
            } else if let Some(obj_term) = object_term {
                let mut combined = with_pps;
                for pp in object_pps {
                    // Rebind the `_PP_SELF_` gap to the object term, recursing
                    // through connectives / quantifiers / events so a reduced
                    // relative restrictor ("the friend Simon went WITH" →
                    // ∃e(Go(e) ∧ Agent(e,Simon) ∧ With(e, _PP_SELF_))) binds its
                    // stranded-preposition gap to the head, not just a flat PP.
                    let substituted = self.substitute_pp_self_term(pp, obj_term);
                    combined = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: combined,
                        op: TokenType::And,
                        right: substituted,
                    });
                }
                combined
            } else {
                with_pps
            };

            // Predicate the definite/constant object's adjectives of it ("ate the
            // RED apple" → Red(Apple)); like the PPs above, dropping them loses a
            // constraint.
            let with_object_pps = if object_adjectives.is_empty() {
                with_object_pps
            } else if let Some(obj_term) = object_term {
                let mut combined = with_object_pps;
                for &adj in object_adjectives {
                    let adj_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                        name: adj,
                        args: self.ctx.terms.alloc_slice([obj_term]),
                        world: None,
                    });
                    combined = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                        left: combined,
                        op: TokenType::And,
                        right: adj_pred,
                    });
                }
                combined
            } else {
                with_object_pps
            };

            // Apply aspectual operators based on verb class
            let with_aspect = if verb_aspect == Aspect::Simple && effective_time == Time::Present {
                // Non-state verbs in simple present get Habitual reading
                if !verb_class.is_stative() {
                    self.ctx.exprs.alloc(LogicExpr::Aspectual {
                        operator: AspectOperator::Habitual,
                        body: with_object_pps,
                    })
                } else {
                    with_object_pps
                }
            } else if verb_aspect == Aspect::Progressive {
                // Semelfactive + Progressive → Iterative
                if verb_class == crate::lexicon::VerbClass::Semelfactive {
                    self.ctx.exprs.alloc(LogicExpr::Aspectual {
                        operator: AspectOperator::Iterative,
                        body: with_object_pps,
                    })
                } else {
                    with_object_pps
                }
            } else {
                with_object_pps
            };

            // Conjoin the predicative-secondary role ("as its mayor" → Mayor(Y)).
            let with_aspect = if let (Some(role), Some(obj)) = (as_role, object_term) {
                let role_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: role,
                    args: self.ctx.terms.alloc_slice([obj]),
                    world: None,
                });
                self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: with_aspect,
                    op: TokenType::And,
                    right: role_pred,
                })
            } else {
                with_aspect
            };

            Ok(with_aspect)
    }
}

impl<'a, 'ctx, 'int> LogicVerbParsing<'a, 'ctx, 'int> for Parser<'a, 'ctx, 'int> {
    fn parse_predicate_with_subject(&mut self, subject_symbol: Symbol) -> ParseResult<&'a LogicExpr<'a>> {
        let result = self.parse_predicate_impl(subject_symbol, false)?;
        Ok(self.try_wrap_bounded_delay(result))
    }

    fn parse_predicate_with_subject_as_var(&mut self, subject_symbol: Symbol) -> ParseResult<&'a LogicExpr<'a>> {
        let result = self.parse_predicate_impl(subject_symbol, true)?;
        Ok(self.try_wrap_bounded_delay(result))
    }

    fn try_parse_plural_subject(
        &mut self,
        first_subject: &NounPhrase<'a>,
    ) -> Result<Option<&'a LogicExpr<'a>>, ParseError> {
        let saved_pos = self.current;

        // Consume the 'and' we already peeked
        self.advance();

        if !self.check_content_word() {
            self.current = saved_pos;
            return Ok(None);
        }

        // Collect all subjects: "John and Mary and Sue"
        let mut subjects: Vec<Symbol> = vec![first_subject.noun];

        loop {
            if !self.check_content_word() {
                break;
            }
            let next_subject = match self.parse_noun_phrase(true) {
                Ok(np) => np,
                Err(_) => {
                    self.current = saved_pos;
                    return Ok(None);
                }
            };
            subjects.push(next_subject.noun);

            if self.check(&TokenType::And) {
                self.advance();
            } else {
                break;
            }
        }

        // Check for copula (is/are/was/were) with predicate nominative
        // "Both Socrates and Plato are men" -> M(s) ∧ M(p)
        if self.check(&TokenType::Is) || self.check(&TokenType::Are)
            || self.check(&TokenType::Was) || self.check(&TokenType::Were)
        {
            let copula_time = if self.check(&TokenType::Was) || self.check(&TokenType::Were) {
                Time::Past
            } else {
                Time::Present
            };
            self.advance(); // consume the copula

            // Check for negation: "are not valid", "are not both valid"
            let is_negated = self.check(&TokenType::Not);
            if is_negated {
                self.advance(); // consume "not"
            }

            // Check for "both" modifier: "are not both valid"
            // "both" scopes negation over the conjunction: ¬(P(A) ∧ P(B))
            // Without "both": negation distributes: ¬P(A) ∧ ¬P(B)
            let has_both = self.check(&TokenType::Both);
            if has_both {
                self.advance(); // consume "both"
            }

            // Parse the predicate (e.g., "men" in "are men", "valid" in "are valid")
            if !self.check_content_word() && !self.check_article() {
                self.current = saved_pos;
                return Ok(None);
            }

            let predicate_np = match self.parse_noun_phrase(false) {
                Ok(np) => np,
                Err(_) => {
                    self.current = saved_pos;
                    return Ok(None);
                }
            };
            let predicate = predicate_np.noun;

            // "A and B are DIFFERENT people" — the adjective "different" asserts
            // the members are pairwise DISTINCT (the puzzle solver's AllDifferent
            // constraint). Dropping it loses the constraint, so it is kept as
            // ¬(si = sj) below; mirrors the comma-list subject path.
            let is_different = predicate_np.adjectives.iter().any(|a| {
                self.interner.resolve(*a).eq_ignore_ascii_case("different")
            });

            // A category DECLARATION ("Bill, Lillie, … are four different friends.",
            // "2001, … are four different years.") records item→category in the
            // shared discourse DRS, so a later definite LABEL ("the 2003 holiday",
            // "the Florida trip") can recover the category and un-fuse to the same
            // relation the prepositional-phrase form produces. The predicate
            // nominal is the category noun; each coordinated subject is an item of
            // that category.
            for subj in &subjects {
                self.drs.register_item_category(*subj, predicate);
            }

            // Build distributed predicate: P(s1) ∧ P(s2) ∧ ...
            let mut conjuncts: Vec<&'a LogicExpr<'a>> = Vec::new();
            for subj in &subjects {
                let pred_expr = self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: predicate,
                    args: self.ctx.terms.alloc_slice([Term::Constant(*subj)]),
                    world: None,
                });
                conjuncts.push(pred_expr);
            }
            if is_different {
                for i in 0..subjects.len() {
                    for j in (i + 1)..subjects.len() {
                        let eq = self.ctx.exprs.alloc(LogicExpr::Identity {
                            left: self.ctx.terms.alloc(Term::Constant(subjects[i])),
                            right: self.ctx.terms.alloc(Term::Constant(subjects[j])),
                        });
                        conjuncts.push(self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                            op: TokenType::Not,
                            operand: eq,
                        }));
                    }
                }
            }

            if is_negated && !has_both {
                // "are not valid" → ¬P(s1) ∧ ¬P(s2) (negation distributes)
                for conjunct in &mut conjuncts {
                    *conjunct = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand: *conjunct,
                    });
                }
            }

            // Fold conjuncts into binary conjunction tree
            let mut result = conjuncts[0];
            for conjunct in &conjuncts[1..] {
                result = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: result,
                    op: TokenType::And,
                    right: *conjunct,
                });
            }

            // "are not both valid" → ¬(P(s1) ∧ P(s2)) (negation over conjunction)
            if is_negated && has_both {
                result = self.ctx.exprs.alloc(LogicExpr::UnaryOp {
                    op: TokenType::Not,
                    operand: result,
                });
            }

            // Apply temporal modifier for past tense
            let with_time = match copula_time {
                Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: TemporalOperator::Past,
                    body: result,
                }),
                _ => result,
            };

            return Ok(Some(with_time));
        }

        if !self.check_verb() {
            self.current = saved_pos;
            return Ok(None);
        }

        // Coordinated subjects registered in DRS via introduce_referent

        let (verb, verb_time, _verb_aspect, _) = self.consume_verb_with_metadata();

        // Check for reciprocal: "John and Mary kicked each other"
        if self.check(&TokenType::Reciprocal) {
            self.advance();
            if subjects.len() != 2 {
                self.current = saved_pos;
                return Ok(None);
            }
            let pred1 = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice([
                    Term::Constant(subjects[0]),
                    Term::Constant(subjects[1]),
                ]),
                world: None,
            });
            let pred2 = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice([
                    Term::Constant(subjects[1]),
                    Term::Constant(subjects[0]),
                ]),
                world: None,
            });
            let expr = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left: pred1,
                op: TokenType::And,
                right: pred2,
            });

            let with_time = match verb_time {
                Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: TemporalOperator::Past,
                    body: expr,
                }),
                Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: TemporalOperator::Future,
                    body: expr,
                }),
                _ => expr,
            };
            return Ok(Some(with_time));
        }

        // Check for objects (for transitive verbs with "respectively")
        let mut objects: Vec<Symbol> = Vec::new();
        if self.check_content_word() || self.check_article() {
            // Parse first object
            let first_obj = match self.parse_noun_phrase(false) {
                Ok(np) => np,
                Err(_) => {
                    // No objects, continue with intransitive
                    return Ok(Some(self.build_group_predicate(&subjects, verb, verb_time)));
                }
            };
            objects.push(first_obj.noun);

            // Parse additional objects: "Tom and Jerry and Bob"
            while self.check(&TokenType::And) {
                self.advance();
                if self.check_content_word() || self.check_article() {
                    let next_obj = match self.parse_noun_phrase(false) {
                        Ok(np) => np,
                        Err(_) => break,
                    };
                    objects.push(next_obj.noun);
                } else {
                    break;
                }
            }
        }

        // Check for "respectively" - triggers pairwise interpretation
        // Ditransitive pairing ("gave books TO TOM AND JERRY respectively"):
        // the recipients, not the shared theme, line up with the subjects.
        let mut recipients: Vec<Symbol> = Vec::new();
        let respectively_ahead = {
            let mut i = self.current;
            let mut found = false;
            while i < self.tokens.len()
                && !matches!(self.tokens[i].kind, TokenType::Period | TokenType::EOF)
            {
                if matches!(self.tokens[i].kind, TokenType::Respectively) {
                    found = true;
                    break;
                }
                i += 1;
            }
            found
        };
        if respectively_ahead && self.check_to_marker() {
            self.advance(); // to
            loop {
                let r_np = self.parse_noun_phrase(false)?;
                recipients.push(r_np.noun);
                if self.check(&TokenType::And) {
                    self.advance();
                } else {
                    break;
                }
            }
        }

        if self.check(&TokenType::Respectively) {
            let respectively_span = self.peek().span;
            self.advance(); // consume "respectively"

            let pair_targets: &[Symbol] = if recipients.is_empty() {
                &objects
            } else {
                &recipients
            };
            if subjects.len() != pair_targets.len() {
                return Err(ParseError {
                    kind: ParseErrorKind::RespectivelyLengthMismatch {
                        subject_count: subjects.len(),
                        object_count: pair_targets.len(),
                    },
                    span: respectively_span,
                });
            }

            // Build pairwise predicates: See(J,T) ∧ See(M,J) ∧ ...; with
            // recipients, the theme is shared: Give(J,Books,T) ∧ Give(M,Books,J).
            let mut conjuncts: Vec<&'a LogicExpr<'a>> = Vec::new();
            let suppress_existential = self.drs.in_conditional_antecedent();
            for (subj, target) in subjects.iter().zip(pair_targets.iter()) {
                let event_var = self.get_event_var();
                let roles = if recipients.is_empty() {
                    vec![
                        (ThematicRole::Agent, Term::Constant(*subj)),
                        (ThematicRole::Theme, Term::Constant(*target)),
                    ]
                } else {
                    let mut r = vec![(ThematicRole::Agent, Term::Constant(*subj))];
                    if let Some(theme) = objects.first() {
                        r.push((ThematicRole::Theme, Term::Constant(*theme)));
                    }
                    r.push((ThematicRole::Recipient, Term::Constant(*target)));
                    r
                };
                let neo_event = self.ctx.exprs.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var,
                    verb,
                    roles: self.ctx.roles.alloc_slice(roles),
                    modifiers: self.ctx.syms.alloc_slice(vec![]),
                    suppress_existential,
                    world: None,
                })));
                conjuncts.push(neo_event);
            }

            // Fold conjuncts into binary conjunction tree
            let mut result = conjuncts[0];
            for conjunct in &conjuncts[1..] {
                result = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                    left: result,
                    op: TokenType::And,
                    right: *conjunct,
                });
            }

            // Apply temporal modifier
            let with_time = match verb_time {
                Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: TemporalOperator::Past,
                    body: result,
                }),
                Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                    operator: TemporalOperator::Future,
                    body: result,
                }),
                _ => result,
            };

            return Ok(Some(with_time));
        }

        // No "respectively" - use group semantics
        if objects.is_empty() {
            // Intransitive: group subject
            Ok(Some(self.build_group_predicate(&subjects, verb, verb_time)))
        } else {
            // Transitive without "respectively": group subject, group object
            Ok(Some(self.build_group_transitive(&subjects, &objects, verb, verb_time)))
        }
    }

    /// Build a group predicate for intransitive verbs
    fn build_group_predicate(
        &mut self,
        subjects: &[Symbol],
        verb: Symbol,
        verb_time: Time,
    ) -> &'a LogicExpr<'a> {
        let group_members: Vec<Term<'a>> = subjects.iter()
            .map(|s| Term::Constant(*s))
            .collect();
        let group_members_slice = self.ctx.terms.alloc_slice(group_members);

        let expr = self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: verb,
            args: self.ctx.terms.alloc_slice([Term::Group(group_members_slice)]),
            world: None,
        });

        match verb_time {
            Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: TemporalOperator::Past,
                body: expr,
            }),
            Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: TemporalOperator::Future,
                body: expr,
            }),
            _ => expr,
        }
    }

    /// Build a transitive predicate with group subject and group object
    fn build_group_transitive(
        &mut self,
        subjects: &[Symbol],
        objects: &[Symbol],
        verb: Symbol,
        verb_time: Time,
    ) -> &'a LogicExpr<'a> {
        let subj_members: Vec<Term<'a>> = subjects.iter()
            .map(|s| Term::Constant(*s))
            .collect();
        let obj_members: Vec<Term<'a>> = objects.iter()
            .map(|o| Term::Constant(*o))
            .collect();

        let subj_group = Term::Group(self.ctx.terms.alloc_slice(subj_members));
        let obj_group = Term::Group(self.ctx.terms.alloc_slice(obj_members));

        let expr = self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: verb,
            args: self.ctx.terms.alloc_slice([subj_group, obj_group]),
            world: None,
        });

        match verb_time {
            Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: TemporalOperator::Past,
                body: expr,
            }),
            Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: TemporalOperator::Future,
                body: expr,
            }),
            _ => expr,
        }
    }

    fn parse_control_structure(
        &mut self,
        subject: &NounPhrase<'a>,
        verb: Symbol,
        verb_time: Time,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        let subject_sym = subject.noun;
        let verb_str = self.interner.resolve(verb);

        if Lexer::is_raising_verb(verb_str) {
            if !self.check_to() {
                return Ok(self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: verb,
                    args: self.ctx.terms.alloc_slice([Term::Constant(subject_sym)]),
                    world: None,
                }));
            }
            self.advance();

            if !self.check_verb() {
                return Ok(self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: verb,
                    args: self.ctx.terms.alloc_slice([Term::Constant(subject_sym)]),
                    world: None,
                }));
            }

            let inf_verb = self.consume_verb();

            let embedded = if self.is_control_verb(inf_verb) {
                let raised_np = NounPhrase {
                    noun: subject_sym,
                    definiteness: None,
                    adjectives: &[],
                    possessor: None,
                    pps: &[],
                    superlative: None,
                };
                self.parse_control_structure(&raised_np, inf_verb, Time::None)?
            } else {
                self.ctx.exprs.alloc(LogicExpr::Predicate {
                    name: inf_verb,
                    args: self.ctx.terms.alloc_slice([Term::Constant(subject_sym)]),
                    world: None,
                })
            };

            let result = self.ctx.exprs.alloc(LogicExpr::Scopal {
                operator: verb,
                body: embedded,
            });

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

        let is_object_control = Lexer::is_object_control_verb(self.interner.resolve(verb));
        let (object_term, pro_controller_sym) = if self.check_to() {
            (None, subject_sym)
        } else if self.check_content_word() {
            let object_np = self.parse_noun_phrase(false)?;
            let obj_sym = object_np.noun;

            let controller = if is_object_control {
                obj_sym
            } else {
                subject_sym
            };
            (
                Some(self.ctx.terms.alloc(Term::Constant(obj_sym))),
                controller,
            )
        } else {
            (None, subject_sym)
        };

        if !self.check_to() {
            return Ok(self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: match object_term {
                    Some(obj) => self.ctx.terms.alloc_slice([
                        Term::Constant(subject_sym),
                        Term::Constant(match obj {
                            Term::Constant(s) => *s,
                            _ => subject_sym,
                        }),
                    ]),
                    None => self.ctx.terms.alloc_slice([Term::Constant(subject_sym)]),
                },
                world: None,
            }));
        }
        self.advance();

        if !self.check_verb() {
            return Ok(self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: verb,
                args: self.ctx.terms.alloc_slice([Term::Constant(subject_sym)]),
                world: None,
            }));
        }

        let inf_verb = self.consume_verb();
        let inf_verb_str = self.interner.resolve(inf_verb).to_lowercase();

        let infinitive = if inf_verb_str == "be" && self.check_verb() {
            let passive_verb = self.consume_verb();
            // An agent by-phrase fills the first argument slot, matching the
            // finite passive ("was seen by the people" → See(People, s)).
            let mut passive_args = vec![Term::Constant(pro_controller_sym)];
            // A DESCRIPTIVE control-passive by-agent ("…to be fed by the old man")
            // becomes its own restrictor-carrying entity scoping the relation; a
            // bare one keeps the constant form.
            let mut agent_restr: Option<(Symbol, &'a LogicExpr<'a>)> = None;
            if self.check_preposition_is("by")
                && self
                    .tokens
                    .get(self.current + 1)
                    .map_or(false, |t| matches!(
                        t.kind,
                        TokenType::ProperName(_) | TokenType::Noun(_) | TokenType::Article(_)
                    ))
            {
                self.advance(); // by
                let agent_np = self.parse_noun_phrase(false)?;
                let (agent_term, restr) = self.possessor_entity(&agent_np);
                agent_restr = restr;
                passive_args.insert(0, agent_term);
            }
            let passive_pred = self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: passive_verb,
                args: self.ctx.terms.alloc_slice(passive_args),
                world: None,
            });
            let passive_pred = self.wrap_in_possessor_entity(agent_restr, passive_pred);
            self.ctx.voice(crate::ast::VoiceOperator::Passive, passive_pred)
        } else if self.is_control_verb(inf_verb) {
            let controller_np = NounPhrase {
                noun: pro_controller_sym,
                definiteness: None,
                adjectives: &[],
                possessor: None,
                pps: &[],
                superlative: None,
            };
            self.parse_control_structure(&controller_np, inf_verb, Time::None)?
        } else {
            self.ctx.exprs.alloc(LogicExpr::Predicate {
                name: inf_verb,
                args: self
                    .ctx
                    .terms
                    .alloc_slice([Term::Constant(pro_controller_sym)]),
                world: None,
            })
        };

        let control = self.ctx.exprs.alloc(LogicExpr::Control {
            verb,
            subject: self.ctx.terms.alloc(Term::Constant(subject_sym)),
            object: object_term,
            infinitive,
        });

        Ok(match verb_time {
            Time::Past => self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: TemporalOperator::Past,
                body: control,
            }),
            Time::Future => self.ctx.exprs.alloc(LogicExpr::Temporal {
                operator: TemporalOperator::Future,
                body: control,
            }),
            _ => control,
        })
    }

    fn is_control_verb(&self, verb: Symbol) -> bool {
        let lemma = self.interner.resolve(verb);
        Lexer::is_subject_control_verb(lemma)
            || Lexer::is_object_control_verb(lemma)
            || Lexer::is_raising_verb(lemma)
    }
}
