//! The MATH LAYER of the logic sentence grammar — quantifier × function term.
//!
//! A theorem's premises need a quantifier over a function term — `∀ n : Nat, n + 0 = n`. The
//! English theorem surface cannot reach one through its ordinary grammar: `Given:`/`Prove:` lines
//! go through the DECLARATIVE sentence parser, which reads noun phrases and verbs, while
//! `plus`/`equals` belong to the IMPERATIVE expression grammar — a different arena, a different
//! AST, unreachable from a premise.
//!
//! This module gives the logic grammar its own term layer. It parses a MATH SENTENCE:
//!
//! ```text
//! For all n, n plus 0 equals n.
//! For all a b, a plus b equals b plus a.
//! For all a b, if a equals b then b equals a.
//! f(n) plus 0 equals f(n).
//! ```
//!
//! into ordinary [`LogicExpr::Quantifier`] / [`LogicExpr::Identity`] over
//! [`Term::Function`], so the existing `proof_convert` bridge lowers it to exactly the
//! [`ProofExpr`](logicaffeine_proof::ProofExpr) the FORMAL surface (`for all n, Eq(Add(n, 0), n)`)
//! already produces — and therefore to the same certified kernel proof. Nothing downstream
//! needed a new rule, a new axiom, or a special case.
//!
//! # Two conventions are deliberately SHARED with the formal surface
//!
//! * **Case is the binder/constant distinction.** A lowercase-leading name is a VARIABLE,
//!   an uppercase- or digit-leading name is a CONSTANT — `formula::ident_to_term`'s rule, so
//!   `n` means a variable on both surfaces and `C` means an opaque constant on both.
//!   **With one deliberate exception, and it is worth stating because it is the one place
//!   the two surfaces DISAGREE:** an English NUMBER WORD is a numeral whatever its case, so
//!   `Zero`/`One`/`Ten` are `0`/`1`/`10` here while `formula::ident_to_term` reads `Zero` as
//!   an opaque constant. English capitalises sentence-initially — `Zero plus n equals n.` is
//!   an ordinary sentence — so a surface that made a capital letter change the PROPOSITION
//!   would refuse the most natural spelling of the very theorem it exists for. The English
//!   surface is the more generous of the two here, on purpose, and
//!   `an_english_number_word_is_a_numeral_whatever_its_case` pins both halves.
//! * **Arithmetic is the ring vocabulary.** `plus`/`minus`/`times`/`divided by` intern as
//!   `add`/`sub`/`mul`/`div`, which `proof_convert::canonical_arithmetic_fn` canonicalises
//!   to the oracle's `Add`/`Sub`/`Mul`/`Div`. An unrecognised name stays an uninterpreted
//!   function, exactly as on the formal surface.
//!
//!   **`divided by` is the one that is NOT in that ring**, and saying "the ring vocabulary"
//!   without saying so is how the next person mis-reads this surface. Division is not a ring
//!   operation, so `tactic::ring` — which asks `arith::prove_int_eq` — decides `Add`, `Sub`
//!   and `Mul` and nothing about `Div`. `For all n, n times 1 equals n.` certifies;
//!   `For all n, n divided by 1 equals n.` is TRUE, is stated correctly as `∀n(Div(n, 1) = n)`,
//!   and proves on NO surface — the formal one refuses it too. The gap is in the decision
//!   procedure, not in this grammar, and it is recorded here rather than left for a user to
//!   discover as "no proof found" on a theorem they know is true.
//!
//! # A binder's SCOPE outranks every word table
//!
//! `For all one, one times one equals one.` binds `one` and means it. The numeral table —
//! the lexer's `Cardinal` token for `one`…`ten`, and this module's own `zero` row — is
//! consulted only for a name no enclosing binder bound, which is how `∀ one : Nat,
//! one * one = one` binds in Lean too. Without that rule the binder bound nothing and what
//! reached the kernel was `∀one(1 × 1 = 1)`: true, certified, and not the sentence the
//! author wrote (`∀x(x × x = x)` is false). The certificate was honest about the term it
//! was handed; the term was not honest about the sentence.
//!
//! # A numeral the ring cannot VALUE still keeps its own spelling
//!
//! An `i64` numeral is the integer it denotes. Anything else — a decimal, an exponent, or an
//! integer too large for one — travels as an opaque constant spelled exactly as it was
//! written, and that is a soundness rule rather than a style choice. Widening to `f64` and
//! rendering the float back (which is what `proof_convert::term_to_proof_term` does with
//! `NumberKind::Real`) changes the NUMBER: `99999999999999999999` and `100000000000000000000`
//! are the same `f64` and both print `100000000000000000000`, so
//! `For all n, n plus 99999999999999999999 equals n plus 100000000000000000000.` reached the
//! kernel as `t = t` and came back certified. The lexeme is the identity, so the lexeme is
//! what travels.
//!
//! # What it deliberately does NOT do
//!
//! The EXISTENTIAL binder is stated here and certified nowhere. `For some n, n plus 1
//! equals 2.` builds `∃n(Add(n, 1) = 2)` — this is the only vernacular surface that can
//! say it at all, since `formula::parse_formula` answers `for some` with "expected `all`
//! after `for`" — and the proof layer closes it no further than a derivation
//! `check_derivation` refuses: `exists 1` then `ring` reports "derivation found but NOT
//! kernel-certified". Expressibility was the claim this module was built to refute, and the
//! existential half of it is expressible; it is the ExistentialIntro certifier that has not
//! caught up.
//!
//! There is no SORT on the binder — `LogicExpr::Quantifier` carries none and neither does
//! `ProofExpr::ForAll`, so `For all n : Nat,` is REFUSED rather than accepted with the
//! `: Nat` silently dropped. Accepting a sort we cannot represent would attest a
//! proposition about `Nat` while proving the ring one; refusing is the loud answer, and the
//! residue is recorded rather than papered over.
//!
//! # Why it cannot steal an English sentence
//!
//! The production is attempted only when a cheap pre-scan of the sentence finds an equality
//! relation (`equals` / `=` / `==`) AND either a `For all`-style binder head or an infix
//! arithmetic operator — and it is accepted only if it consumes the sentence ALL the way to
//! its terminator. Anything else rewinds to the entry position and falls through to the
//! ordinary English parser, so the math layer is strictly additive.

use super::{ParseResult, Parser, ParserMode};
use crate::ast::{LogicExpr, NumberKind, QuantifierKind, Term};
use crate::error::{ParseError, ParseErrorKind};
use crate::token::TokenType;
use logicaffeine_base::Symbol;

/// The math layer's entry point, called from the head of the declarative sentence parser.
pub(super) trait MathParsing<'a, 'ctx, 'int> {
    /// Attempts to read the upcoming sentence as a quantified arithmetic proposition.
    ///
    /// * `None` — not a math sentence; the parser position is RESTORED and the English
    ///   parser gets the sentence untouched.
    /// * `Some(Err(_))` — unambiguously a math binder, written in a way this surface cannot
    ///   represent. Propagated, because falling through would report the wrong construct.
    /// * `Some(Ok(_))` — the whole sentence, as a logic expression.
    fn try_parse_math_sentence(&mut self) -> Option<ParseResult<&'a LogicExpr<'a>>>;
}

impl<'a, 'ctx, 'int> MathParsing<'a, 'ctx, 'int> for Parser<'a, 'ctx, 'int> {
    fn try_parse_math_sentence(&mut self) -> Option<ParseResult<&'a LogicExpr<'a>>> {
        if self.mode == ParserMode::Imperative {
            return None;
        }
        if let Some(refusal) = self.unsupported_relation_refusal() {
            return Some(Err(refusal));
        }
        if !self.math_sentence_ahead() {
            return None;
        }
        if let Some(refusal) = self.sorted_binder_refusal() {
            return Some(Err(refusal));
        }
        let cp = self.checkpoint();
        // The binder SCOPE, innermost last. It lives here rather than on the parser so a
        // rewind cannot leave a stale binding behind for the next sentence.
        let mut scope: Vec<Symbol> = Vec::new();
        match self.parse_math_proposition(&mut scope) {
            // The production must account for the WHOLE sentence. A partial parse is a
            // mis-read, not a math sentence, and leaving the cursor mid-clause would
            // corrupt every later production — so it rewinds like any other failure.
            Ok(expr) if self.at_sentence_terminator() => Some(Ok(expr)),
            _ => {
                self.restore(cp);
                None
            }
        }
    }
}

/// Does `kind` end a sentence (or a document block), so the math production has consumed
/// everything it was offered?
fn terminates_sentence(kind: &TokenType) -> bool {
    matches!(
        kind,
        TokenType::Period
            | TokenType::Exclamation
            | TokenType::EOF
            | TokenType::Dedent
            | TokenType::BlockHeader { .. }
    )
}

/// Is `lexeme` shaped like a mathematical NAME — a variable, a constant, or a function
/// symbol? Leading letter or `_`, then letters/digits/`_`/`'` (so `x'`, `a_1`, `Succ` all
/// qualify and no punctuation ever does).
fn is_math_name(lexeme: &str) -> bool {
    let mut chars = lexeme.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '_' || c == '\'')
}

/// The formal surface's rule, shared verbatim: an uppercase- or digit-leading name is a
/// CONSTANT, a lowercase-leading one is a VARIABLE (`formula::ident_to_term`).
fn leads_constant(lexeme: &str) -> bool {
    lexeme
        .chars()
        .next()
        .map(|c| c.is_uppercase() || c.is_ascii_digit())
        .unwrap_or(false)
}

/// How a sentence with no binder prefix OPENS, as far as the O(1) half of the pre-scan can
/// tell. The three answers carry different burdens of proof, because English and arithmetic
/// overlap only in one of them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MathStart {
    /// Not a term at all. The sentence is English and the math layer never looks again.
    No,
    /// A shape English does not write: a numeral, a parenthesis, a leading `-`, or a
    /// function APPLICATION (`f(`). An equality relation is then enough to make the
    /// sentence a term equation — `2 equals 2.` and `f(3) equals 3.` are statements of
    /// arithmetic whether or not an operator appears anywhere in them.
    Unambiguous,
    /// A bare NAME whose next token is an operator or the relation. `a equals b` reads as
    /// English as readily as it reads as algebra, so this shape must ALSO carry infix
    /// arithmetic before the math layer claims the sentence.
    Name,
}

/// Is `lexeme` one of the English infix arithmetic operators the math layer reads? Shared by
/// the pre-scan gate and the term grammar so the two can never disagree about what makes a
/// sentence arithmetic.
fn is_infix_arithmetic_word(lexeme: &str) -> bool {
    ["plus", "minus", "times", "divided"]
        .iter()
        .any(|w| lexeme.eq_ignore_ascii_case(w))
}

/// The English name of a small number, as a term-level numeral. `zero` is included here
/// because the lexicon's own table starts at `one` — and "adding zero" is the sentence this
/// whole layer exists to state.
fn number_word(lexeme: &str) -> Option<i64> {
    let lower = lexeme.to_ascii_lowercase();
    if lower == "zero" {
        return Some(0);
    }
    crate::lexicon::word_to_number(&lower).map(i64::from)
}

impl<'a, 'ctx, 'int> Parser<'a, 'ctx, 'int> {
    // ───────────────────────── the pre-scan ─────────────────────────

    /// Worth attempting the math production at all?
    ///
    /// Two O(1) gates come first, because this runs on EVERY declarative sentence the parser
    /// sees. A binder head (`For all …`) is one token pair; a bare equation must begin with a
    /// TERM whose following token is an operator, an opening paren, or the relation — which
    /// rejects essentially every English sentence on its second token, since English puts a
    /// copula or a verb there. Only then is the sentence scanned, allocation-free and
    /// bounded by its terminator, for the equality relation (and, without a binder, for the
    /// arithmetic that makes it a term equation rather than English).
    fn math_sentence_ahead(&self) -> bool {
        if self.at_math_binder().is_some() {
            return self.sentence_has_relation(false);
        }
        match self.math_term_start() {
            MathStart::No => false,
            MathStart::Unambiguous => self.sentence_has_relation(false),
            MathStart::Name => self.sentence_has_relation(true),
        }
    }

    /// The O(1) half of the gate for a bare equation — see [`MathStart`] for why the answer
    /// is three-valued rather than a yes/no.
    fn math_term_start(&self) -> MathStart {
        let kind = &self.peek().kind;
        if matches!(
            kind,
            TokenType::LParen | TokenType::Minus | TokenType::Number(_) | TokenType::Cardinal(_)
        ) {
            return MathStart::Unambiguous;
        }
        if !is_math_name(self.interner.resolve(self.peek().lexeme)) {
            return MathStart::No;
        }
        let Some(next) = self.tokens.get(self.current + 1) else {
            return MathStart::No;
        };
        if matches!(next.kind, TokenType::LParen) {
            return MathStart::Unambiguous;
        }
        if matches!(
            next.kind,
            TokenType::Plus
                | TokenType::Minus
                | TokenType::Star
                | TokenType::Slash
                | TokenType::Equals
                | TokenType::Identity
                | TokenType::Assign
                | TokenType::EqEq
        ) || is_infix_arithmetic_word(self.interner.resolve(next.lexeme))
        {
            return MathStart::Name;
        }
        MathStart::No
    }

    /// Does the upcoming sentence carry an equality relation (and, when `require_arithmetic`
    /// is set, an infix arithmetic operator)? Bounded by the sentence terminator, no allocation, and
    /// it stops as soon as both answers are known.
    fn sentence_has_relation(&self, require_arithmetic: bool) -> bool {
        let mut found_relation = false;
        let mut found_arithmetic = !require_arithmetic;
        let mut i = self.current;
        while let Some(token) = self.tokens.get(i) {
            if terminates_sentence(&token.kind) {
                break;
            }
            match token.kind {
                TokenType::Equals | TokenType::Identity | TokenType::Assign | TokenType::EqEq => {
                    found_relation = true
                }
                TokenType::Plus | TokenType::Minus | TokenType::Star | TokenType::Slash => {
                    found_arithmetic = true
                }
                _ => {
                    if !found_arithmetic
                        && is_infix_arithmetic_word(self.interner.resolve(token.lexeme))
                    {
                        found_arithmetic = true;
                    }
                }
            }
            if found_relation && found_arithmetic {
                return true;
            }
            i += 1;
        }
        found_relation && found_arithmetic
    }

    /// Is the cursor on a math binder head, and which quantifier is it?
    ///
    /// `For all` / `For every` / `For each` / `For any` bind universally; `For some` binds
    /// existentially. Both halves of the quantifier are here on purpose: a surface that can say
    /// `∀ n, n + 0 = n` but not `∃ n, n + 1 = 2` expresses only half of arithmetic's statements.
    /// `For infinitely many`, `For [only] finitely many` and `For all but finitely many` bind
    /// the infinitary quantifiers, each one token.
    fn at_math_binder(&self) -> Option<QuantifierKind> {
        let heads_for = matches!(self.peek().kind, TokenType::For)
            || self
                .interner
                .resolve(self.peek().lexeme)
                .eq_ignore_ascii_case("for");
        if !heads_for {
            return None;
        }
        let token = self.tokens.get(self.current + 1)?;
        if matches!(
            token.kind,
            TokenType::InfinitelyMany | TokenType::FinitelyMany | TokenType::AllButFinitelyMany
        ) {
            return QuantifierKind::of_determiner(&token.kind);
        }
        let lexeme = self.interner.resolve(token.lexeme);
        if matches!(token.kind, TokenType::All | TokenType::Any)
            || ["all", "every", "each", "any"]
                .iter()
                .any(|w| lexeme.eq_ignore_ascii_case(w))
        {
            return Some(QuantifierKind::Universal);
        }
        if matches!(token.kind, TokenType::Some) || lexeme.eq_ignore_ascii_case("some") {
            return Some(QuantifierKind::Existential);
        }
        None
    }

    fn at_sentence_terminator(&self) -> bool {
        terminates_sentence(&self.peek().kind)
    }

    /// A quantified ARITHMETIC sentence whose relation is an ORDER rather than an equality:
    /// `For all n, n plus 1 is greater than n.` The term layer relates terms with `equals`
    /// only, so this is a real residue — and it is refused HERE, by name, instead of being
    /// handed to the English parser which reports a missing content word and never mentions
    /// the construct that actually broke.
    ///
    /// The gate is deliberately narrow: a math binder prefix AND infix arithmetic AND no
    /// equality anywhere. "For all cats, the score is greater than five" carries no
    /// arithmetic, so English keeps it.
    fn unsupported_relation_refusal(&self) -> Option<ParseError> {
        if self.at_math_binder().is_none() {
            return None;
        }
        let mut arithmetic = false;
        let mut equality = false;
        let mut order = false;
        let mut i = self.current;
        while let Some(token) = self.tokens.get(i) {
            if terminates_sentence(&token.kind) {
                break;
            }
            match token.kind {
                TokenType::Equals | TokenType::Identity | TokenType::Assign | TokenType::EqEq => {
                    equality = true
                }
                TokenType::Plus | TokenType::Minus | TokenType::Star | TokenType::Slash => {
                    arithmetic = true
                }
                TokenType::Lt | TokenType::Gt | TokenType::LtEq | TokenType::GtEq => order = true,
                _ => {
                    let lexeme = self.interner.resolve(token.lexeme);
                    if is_infix_arithmetic_word(lexeme) {
                        arithmetic = true;
                    } else if ["greater", "less", "exceeds", "smaller", "larger"]
                        .iter()
                        .any(|w| lexeme.eq_ignore_ascii_case(w))
                    {
                        order = true;
                    }
                }
            }
            i += 1;
        }
        (arithmetic && order && !equality).then(|| ParseError {
            kind: ParseErrorKind::Custom(
                "the English theorem surface relates TERMS with `equals` only — an ORDER \
                 between arithmetic terms (`is greater than`, `is less than`) has no term \
                 production here yet. State it as an equality, or use the formal surface"
                    .to_string(),
            ),
            span: self.current_span(),
        })
    }

    /// `For all n : Nat,` — a SORTED binder, which this surface cannot represent (see the
    /// module docs). Detected here, before any token is consumed, so the refusal names the
    /// construct that actually broke instead of whatever the English parser makes of a
    /// stray colon. Returns `None` for every other shape.
    fn sorted_binder_refusal(&self) -> Option<ParseError> {
        if self.at_math_binder().is_none() {
            return None;
        }
        let mut i = self.current + 2;
        while let Some(token) = self.tokens.get(i) {
            let lexeme = self.interner.resolve(token.lexeme);
            if lexeme.eq_ignore_ascii_case("and") {
                i += 1;
                continue;
            }
            if !is_math_name(lexeme) || leads_constant(lexeme) {
                break;
            }
            i += 1;
        }
        matches!(self.tokens.get(i).map(|t| &t.kind), Some(TokenType::Colon)).then(|| ParseError {
            kind: ParseErrorKind::Custom(
                "a quantifier on the English theorem surface carries no SORT: write \
                     `For all n,` — the arithmetic is the ring's. Accepting `: Nat` and \
                     dropping it would attest a proposition about `Nat` while proving a \
                     different one"
                    .to_string(),
            ),
            span: self.tokens.get(i).map(|t| t.span).unwrap_or_default(),
        })
    }

    // ───────────────────────── the grammar ─────────────────────────

    /// `proposition := ('For all' | 'For some') var+ ',' proposition | implication`
    fn parse_math_proposition(
        &mut self,
        scope: &mut Vec<Symbol>,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        self.enter_recursion()?;
        let result = self.parse_math_proposition_inner(scope);
        self.leave_recursion();
        result
    }

    fn parse_math_proposition_inner(
        &mut self,
        scope: &mut Vec<Symbol>,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        let Some(kind) = self.at_math_binder() else {
            return self.parse_math_implication(scope);
        };
        self.advance(); // `For`
        self.advance(); // `all` / `every` / `each` / `any` / `some` / an infinitary determiner

        let mut variables = Vec::new();
        loop {
            let symbol = self.peek().lexeme;
            let binds = {
                let lexeme = self.interner.resolve(symbol);
                is_math_name(lexeme) && !leads_constant(lexeme)
            };
            if !binds {
                break;
            }
            variables.push(symbol);
            self.advance();
            // `and` between binders is decoration (`For all a and b,`). A COMMA is not
            // consumed here — it is what ends the prefix and opens the body.
            if self.check_word("and") {
                self.advance();
            }
        }
        if variables.is_empty() {
            return Err(
                self.math_error("a math binder needs at least one lowercase variable to bind")
            );
        }
        // A SORT is not representable — neither `LogicExpr::Quantifier` nor
        // `ProofExpr::ForAll` carries one — so `For all n : Nat,` is refused rather than
        // accepted with the sort silently dropped.
        if self.check(&TokenType::Colon) {
            return Err(self.math_error(
                "a quantifier on this surface carries no SORT: write `For all n,` \
                 (the arithmetic is the ring's). Dropping `: Nat` would attest a \
                 different proposition than the one written",
            ));
        }
        if !self.check(&TokenType::Comma) {
            return Err(self.math_error("a math binder prefix ends with a comma"));
        }
        self.advance(); // `,`

        // The prefix is in scope for the body and for nothing after it. A name this
        // prefix bound is that VARIABLE wherever the body mentions it — the numeral table
        // does not reach inside a binder's scope, any more than it does in Lean.
        let outer = scope.len();
        scope.extend(variables.iter().copied());
        let body = self.parse_math_proposition(scope);
        scope.truncate(outer);
        let mut body = body?;
        for variable in variables.into_iter().rev() {
            body = self.ctx.exprs.alloc(LogicExpr::Quantifier {
                kind,
                variable,
                body,
                island_id: self.current_island,
            });
        }
        Ok(body)
    }

    /// `implication := 'if' proposition 'then' proposition | disjunction`
    fn parse_math_implication(
        &mut self,
        scope: &mut Vec<Symbol>,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        if !self.check(&TokenType::If) {
            return self.parse_math_disjunction(scope);
        }
        self.advance(); // `if`
        let antecedent = self.parse_math_proposition(scope)?;
        if self.check(&TokenType::Comma) {
            self.advance();
        }
        if !self.check(&TokenType::Then) {
            return Err(self.math_error("`if` in a math sentence needs a matching `then`"));
        }
        self.advance(); // `then`
        let consequent = self.parse_math_proposition(scope)?;
        Ok(self.ctx.exprs.alloc(LogicExpr::BinaryOp {
            left: antecedent,
            op: TokenType::Implies,
            right: consequent,
        }))
    }

    /// `disjunction := conjunction ('or' conjunction)*`
    fn parse_math_disjunction(
        &mut self,
        scope: &mut Vec<Symbol>,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        let mut left = self.parse_math_conjunction(scope)?;
        while self.check(&TokenType::Or) {
            self.advance();
            let right = self.parse_math_conjunction(scope)?;
            left = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left,
                op: TokenType::Or,
                right,
            });
        }
        Ok(left)
    }

    /// `conjunction := equation ('and' equation)*`
    fn parse_math_conjunction(
        &mut self,
        scope: &mut Vec<Symbol>,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        let mut left = self.parse_math_equation(scope)?;
        while self.check(&TokenType::And) {
            self.advance();
            let right = self.parse_math_equation(scope)?;
            left = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left,
                op: TokenType::And,
                right,
            });
        }
        Ok(left)
    }

    /// `equation := term ('equals' | '=' | '==') term`
    fn parse_math_equation(&mut self, scope: &[Symbol]) -> ParseResult<&'a LogicExpr<'a>> {
        let left = self.parse_math_sum(scope)?;
        let is_relation = matches!(
            self.peek().kind,
            TokenType::Equals | TokenType::Identity | TokenType::Assign | TokenType::EqEq
        );
        if !is_relation {
            return Err(self.math_error(
                "a math sentence relates two terms with `equals` (or `=`); \
                 no other relation has a term grammar on this surface",
            ));
        }
        self.advance();
        let right = self.parse_math_sum(scope)?;
        Ok(self.ctx.exprs.alloc(LogicExpr::Identity {
            left: self.ctx.terms.alloc(left),
            right: self.ctx.terms.alloc(right),
        }))
    }

    /// `sum := product (('plus' | '+' | 'minus' | '-') product)*` — left-associative.
    fn parse_math_sum(&mut self, scope: &[Symbol]) -> ParseResult<Term<'a>> {
        let mut left = self.parse_math_product(scope)?;
        loop {
            let op = if self.check(&TokenType::Plus) || self.check_word("plus") {
                "add"
            } else if self.check(&TokenType::Minus) || self.check_word("minus") {
                "sub"
            } else {
                break;
            };
            self.advance();
            let right = self.parse_math_product(scope)?;
            left = self.math_apply(op, [left, right]);
        }
        Ok(left)
    }

    /// `product := atom (('times' | '*' | 'divided by' | '/') atom)*` — left-associative,
    /// binding tighter than `plus`/`minus`.
    fn parse_math_product(&mut self, scope: &[Symbol]) -> ParseResult<Term<'a>> {
        let mut left = self.parse_math_atom(scope)?;
        loop {
            let op = if self.check(&TokenType::Star) || self.check_word("times") {
                self.advance();
                "mul"
            } else if self.check(&TokenType::Slash) {
                self.advance();
                "div"
            } else if self.check_word("divided") && self.peek_word_at(1, "by") {
                self.advance();
                self.advance();
                "div"
            } else {
                break;
            };
            let right = self.parse_math_atom(scope)?;
            left = self.math_apply(op, [left, right]);
        }
        Ok(left)
    }

    /// `atom := '-' atom | bound-name | numeral | name '(' term,* ')' | name | '(' sum ')'`
    ///
    /// The term layer RECURSES — through parentheses, through unary minus, and through a
    /// function application's arguments — so it takes the same depth bound the clause
    /// grammar does. Without it a pathological `((((…n…))))` would be a stack overflow
    /// rather than a parse error.
    fn parse_math_atom(&mut self, scope: &[Symbol]) -> ParseResult<Term<'a>> {
        self.enter_recursion()?;
        let result = self.parse_math_atom_inner(scope);
        self.leave_recursion();
        result
    }

    fn parse_math_atom_inner(&mut self, scope: &[Symbol]) -> ParseResult<Term<'a>> {
        if self.check(&TokenType::LParen) {
            self.advance();
            let inner = self.parse_math_sum(scope)?;
            if !self.check(&TokenType::RParen) {
                return Err(self.math_error("a parenthesised term needs its closing `)`"));
            }
            self.advance();
            return Ok(inner);
        }
        // Unary minus is the ring's negation, `0 - x`, so it needs no `neg` symbol the
        // oracle would read as an uninterpreted function.
        if self.check(&TokenType::Minus) {
            self.advance();
            let operand = self.parse_math_atom(scope)?;
            return Ok(self.math_apply("sub", [Self::math_integer(0), operand]));
        }

        let name = self.peek().lexeme;
        let kind = self.peek().kind.clone();
        let lexeme = self.interner.resolve(name).to_string();
        // SCOPE outranks the word table. A name the enclosing binder prefix bound is that
        // VARIABLE wherever the body mentions it: `For all one, one times one equals one.`
        // is a statement about the bound `one`, never about the numeral 1 — exactly as
        // `∀ one : Nat, one * one = one` binds in Lean. A name in FUNCTION position is an
        // application of a function SYMBOL (this surface has no second-order binder), so it
        // is left to the application rule below.
        let applied = matches!(
            self.tokens.get(self.current + 1).map(|t| &t.kind),
            Some(TokenType::LParen)
        );
        if !applied && scope.contains(&name) {
            self.advance();
            return Ok(Term::Variable(name));
        }
        if let TokenType::Cardinal(n) = kind {
            self.advance();
            return Ok(Self::math_integer(i64::from(n)));
        }
        if matches!(kind, TokenType::Number(_)) {
            self.advance();
            // A numeral the ring can VALUE is an `i64`. Anything else — a decimal, an
            // exponent, or an integer too large for one — keeps its SOURCE SPELLING as an
            // opaque constant, and that is a soundness rule, not a style choice.
            //
            // Widening to `f64` and rendering the float back (which is what
            // `proof_convert::term_to_proof_term` does with `NumberKind::Real`) changes the
            // NUMBER: `99999999999999999999` and `100000000000000000000` are the same `f64`
            // and both print `100000000000000000000`, so
            // `For all n, n plus 99999999999999999999 equals n plus 100000000000000000000.`
            // reached the kernel as `t = t` and CERTIFIED. Two distinct numbers becoming one
            // term is a proof of something nobody wrote. The lexeme is the identity, so the
            // lexeme is what travels.
            return Ok(match lexeme.parse::<i64>() {
                Ok(n) => Self::math_integer(n),
                Err(_) => Term::Constant(name),
            });
        }
        if !is_math_name(&lexeme) {
            return Err(self.math_error(
                "a math term is a numeral, a name, or an application of a function symbol",
            ));
        }
        if let Some(n) = number_word(&lexeme) {
            self.advance();
            return Ok(Self::math_integer(n));
        }

        self.advance();
        if self.check(&TokenType::LParen) {
            self.advance();
            let mut args = Vec::new();
            if !self.check(&TokenType::RParen) {
                loop {
                    args.push(self.parse_math_sum(scope)?);
                    if self.check(&TokenType::Comma) {
                        self.advance();
                        continue;
                    }
                    break;
                }
            }
            if !self.check(&TokenType::RParen) {
                return Err(self.math_error("a function application needs its closing `)`"));
            }
            self.advance();
            return Ok(Term::Function(
                name,
                self.ctx.terms.alloc_slice(args.into_iter()),
            ));
        }
        Ok(if leads_constant(&lexeme) {
            Term::Constant(name)
        } else {
            Term::Variable(name)
        })
    }

    // ───────────────────────── construction helpers ─────────────────────────

    fn math_integer(n: i64) -> Term<'a> {
        Term::Value {
            kind: NumberKind::Integer(n),
            unit: None,
            dimension: None,
        }
    }

    fn math_apply(&mut self, op: &str, args: [Term<'a>; 2]) -> Term<'a> {
        let symbol = self.interner.intern(op);
        Term::Function(symbol, self.ctx.terms.alloc_slice(args))
    }

    fn math_error(&self, message: &str) -> ParseError {
        ParseError {
            kind: ParseErrorKind::Custom(message.to_string()),
            span: self.current_span(),
        }
    }
}
