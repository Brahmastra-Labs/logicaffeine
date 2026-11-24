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
//! # Integer ORDER is the proof layer's own two Props
//!
//! Besides `equals`, two terms may be ORDERED. The proof layer has exactly two order Props,
//! `le(a, b) = true` and `lt(a, b) = true` (`engine::le_eq`, `omega_solve::lt_eq`), which
//! are what `omega`, the Farkas core and the certifier read, so the English spellings lower
//! onto those and nothing else. The converse words swap the operands rather than inventing a
//! `ge`/`gt` symbol no decision procedure reads:
//!
//! ```text
//! a is at most b       a is less than or equal to b       a <= b    ⇒  le(a, b) = true
//! a is at least b      a is greater than or equal to b    a >= b    ⇒  le(b, a) = true
//! a is less than b                                        a < b     ⇒  lt(a, b) = true
//! a is greater than b                                     a > b     ⇒  lt(b, a) = true
//! ```
//!
//! `or equal to` belongs to the RELATION: `n is less than or equal to n` is one `≤`, never a
//! disjunction of `n is less than` and a dangling `equal to n`.
//!
//! # Why it cannot steal an English sentence
//!
//! The production is attempted only when a cheap pre-scan of the sentence finds a relation
//! (`equals` / `=` / `==`, or one of the order spellings above) AND one of three openings:
//! a `For all`-style binder head, an infix arithmetic operator, or a lowercase VARIABLE
//! subject directly followed by an order relation (`s is at most w`). That last opening is
//! the shared case rule at work: a lowercase-leading name is a variable on both surfaces, so
//! `x is greater than y` is arithmetic, while `Simon is taller than Mary` and
//! `Buddy's wingspan is greater than Max's` keep their degree readings. A pronoun, an
//! article or a quantifier word is never a variable, whatever its case. The production is
//! accepted only if it consumes the sentence ALL the way to its terminator. Anything else
//! rewinds to the entry position and falls through to the ordinary English parser, so the
//! math layer is strictly additive.

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

/// An integer ORDER between two terms, named the way English wrote it (left operand first).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Order {
    AtMost,
    AtLeast,
    LessThan,
    GreaterThan,
}

impl Order {
    /// The proof-layer Prop this order lowers to, and whether English's operands SWAP to
    /// reach it. Only `le` and `lt` exist below this surface, so the converse words swap.
    fn lowering(self) -> (&'static str, bool) {
        match self {
            Order::AtMost => ("le", false),
            Order::AtLeast => ("le", true),
            Order::LessThan => ("lt", false),
            Order::GreaterThan => ("lt", true),
        }
    }
}

/// An order relation found in the token stream: which order, how many tokens spell it, and
/// the numeral it already CARRIES. The lexer folds `at least five` and `at most five` into a
/// single `AtLeast(5)` / `AtMost(5)` token, so the relation and the first atom of its right
/// operand arrive together.
#[derive(Clone, Copy)]
struct OrderRelation {
    order: Order,
    len: usize,
    carried: Option<u32>,
}

/// Comparative words that ORDER two terms but are not one of the spellings above, so a
/// sentence using them is refused by name rather than handed to the English parser.
const ORDER_WORDS_WITHOUT_A_PRODUCTION: [&str; 5] =
    ["greater", "less", "exceeds", "smaller", "larger"];

/// A token that can never be a math VARIABLE however it is cased: a pronoun, an article, a
/// quantifier or a negative-polarity word. `it is at least five` is English about a referent.
fn is_function_word(kind: &TokenType) -> bool {
    matches!(
        kind,
        TokenType::Pronoun { .. }
            | TokenType::Article(_)
            | TokenType::All
            | TokenType::No
            | TokenType::Some
            | TokenType::Any
            | TokenType::Both
            | TokenType::Most
            | TokenType::Few
            | TokenType::Many
            | TokenType::Anything
            | TokenType::Anyone
            | TokenType::Nothing
            | TokenType::Nobody
            | TokenType::NoOne
            | TokenType::Nowhere
    )
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
        let after = self.current + 1;
        let Some(next) = self.tokens.get(after) else {
            return MathStart::No;
        };
        if matches!(next.kind, TokenType::LParen) {
            return MathStart::Unambiguous;
        }
        // A subject directly ORDERED against something. A lowercase variable needs nothing
        // more (`s is at most w`), while a capitalised name is English's degree reading
        // (`Simon is taller than Mary`) unless the sentence also carries arithmetic.
        if self.order_relation_at(after).is_some() {
            return if self.subject_is_variable() {
                MathStart::Unambiguous
            } else {
                MathStart::Name
            };
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

    /// Does the upcoming sentence carry a relation, equality or order (and, when
    /// `require_arithmetic` is set, an infix arithmetic operator)? Bounded by the sentence
    /// terminator, no allocation, and it stops as soon as both answers are known.
    fn sentence_has_relation(&self, require_arithmetic: bool) -> bool {
        let mut found_relation = false;
        let mut found_arithmetic = !require_arithmetic;
        let mut i = self.current;
        while let Some(token) = self.tokens.get(i) {
            if terminates_sentence(&token.kind) {
                break;
            }
            if self.order_relation_at(i).is_some() {
                found_relation = true;
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

    /// A quantified ARITHMETIC sentence that orders its terms with a word this surface has
    /// no production for: `For all n, n plus 1 exceeds n.` It is refused HERE, by name and
    /// with the spellings that do work, instead of being handed to the English parser, which
    /// reports a missing content word and never mentions the construct that actually broke.
    ///
    /// The gate is deliberately narrow: a math binder prefix AND infix arithmetic AND an
    /// order word AND no relation this surface can read. "For all cats, the score is greater
    /// than five" carries no arithmetic, so English keeps it.
    fn unsupported_relation_refusal(&self) -> Option<ParseError> {
        if self.at_math_binder().is_none() {
            return None;
        }
        let mut arithmetic = false;
        let mut relation = false;
        let mut unspelled_order = false;
        let mut i = self.current;
        while let Some(token) = self.tokens.get(i) {
            if terminates_sentence(&token.kind) {
                break;
            }
            if let Some(order) = self.order_relation_at(i) {
                relation = true;
                i += order.len;
                continue;
            }
            match token.kind {
                TokenType::Equals | TokenType::Identity | TokenType::Assign | TokenType::EqEq => {
                    relation = true
                }
                TokenType::Plus | TokenType::Minus | TokenType::Star | TokenType::Slash => {
                    arithmetic = true
                }
                _ => {
                    let lexeme = self.interner.resolve(token.lexeme);
                    if is_infix_arithmetic_word(lexeme) {
                        arithmetic = true;
                    } else if ORDER_WORDS_WITHOUT_A_PRODUCTION
                        .iter()
                        .any(|w| lexeme.eq_ignore_ascii_case(w))
                    {
                        unspelled_order = true;
                    }
                }
            }
            i += 1;
        }
        (arithmetic && unspelled_order && !relation).then(|| ParseError {
            kind: ParseErrorKind::Custom(
                "the English theorem surface relates TERMS with `equals`, or ORDERS them with \
                 `is at most`, `is at least`, `is less than`, `is greater than` (either of the \
                 last two may add `or equal to`), `<=`, `>=`, `<` or `>`. This sentence orders \
                 its terms some other way, which has no term production here"
                    .to_string(),
            ),
            span: self.current_span(),
        })
    }

    /// Is the token at `i` the word `word` (never a string or char literal)?
    fn math_word_at(&self, i: usize, word: &str) -> bool {
        self.tokens.get(i).is_some_and(|t| {
            !matches!(
                t.kind,
                TokenType::StringLiteral(_) | TokenType::CharLiteral(_)
            ) && self.interner.resolve(t.lexeme).eq_ignore_ascii_case(word)
        })
    }

    /// The order relation spelled starting at token `i`, if any. Shared by the pre-scan, the
    /// refusal and the grammar, so the three can never disagree about what an order IS.
    fn order_relation_at(&self, i: usize) -> Option<OrderRelation> {
        let relation = |order, len, carried| Some(OrderRelation { order, len, carried });
        match self.tokens.get(i)?.kind {
            TokenType::LtEq => return relation(Order::AtMost, 1, None),
            TokenType::GtEq => return relation(Order::AtLeast, 1, None),
            TokenType::Lt => return relation(Order::LessThan, 1, None),
            TokenType::Gt => return relation(Order::GreaterThan, 1, None),
            _ => {}
        }
        if !self.math_word_at(i, "is") {
            return None;
        }
        match self.tokens.get(i + 1)?.kind {
            TokenType::AtMost(n) => return relation(Order::AtMost, 2, Some(n)),
            TokenType::AtLeast(n) => return relation(Order::AtLeast, 2, Some(n)),
            _ => {}
        }
        if self.math_word_at(i + 1, "at") {
            if self.math_word_at(i + 2, "most") {
                return relation(Order::AtMost, 3, None);
            }
            if self.math_word_at(i + 2, "least") {
                return relation(Order::AtLeast, 3, None);
            }
            return None;
        }
        for (word, strict, inclusive) in [
            ("less", Order::LessThan, Order::AtMost),
            ("greater", Order::GreaterThan, Order::AtLeast),
        ] {
            if self.math_word_at(i + 1, word) && self.math_word_at(i + 2, "than") {
                let or_equal_to = self.math_word_at(i + 3, "or")
                    && self.math_word_at(i + 4, "equal")
                    && self.math_word_at(i + 5, "to");
                return if or_equal_to {
                    relation(inclusive, 6, None)
                } else {
                    relation(strict, 3, None)
                };
            }
        }
        None
    }

    /// Is the subject at the cursor a math VARIABLE by the shared case rule: a lowercase-led
    /// name that is not a pronoun, article or quantifier word?
    fn subject_is_variable(&self) -> bool {
        let token = self.peek();
        !is_function_word(&token.kind) && !leads_constant(self.interner.resolve(token.lexeme))
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

    /// `conjunction := relation ('and' relation)*`
    fn parse_math_conjunction(
        &mut self,
        scope: &mut Vec<Symbol>,
    ) -> ParseResult<&'a LogicExpr<'a>> {
        let mut left = self.parse_math_relation(scope)?;
        while self.check(&TokenType::And) {
            self.advance();
            let right = self.parse_math_relation(scope)?;
            left = self.ctx.exprs.alloc(LogicExpr::BinaryOp {
                left,
                op: TokenType::And,
                right,
            });
        }
        Ok(left)
    }

    /// `relation := term ('equals' | '=' | '==') term | term order term`, where `order` is
    /// one of the spellings in the module docs and lowers to `le(·, ·) = true` or
    /// `lt(·, ·) = true`.
    fn parse_math_relation(&mut self, scope: &[Symbol]) -> ParseResult<&'a LogicExpr<'a>> {
        let left = self.parse_math_sum(scope)?;
        if matches!(
            self.peek().kind,
            TokenType::Equals | TokenType::Identity | TokenType::Assign | TokenType::EqEq
        ) {
            self.advance();
            let right = self.parse_math_sum(scope)?;
            return Ok(self.ctx.exprs.alloc(LogicExpr::Identity {
                left: self.ctx.terms.alloc(left),
                right: self.ctx.terms.alloc(right),
            }));
        }
        let Some(relation) = self.order_relation_at(self.current) else {
            return Err(self.math_error(
                "a math sentence relates two terms with `equals` (or `=`), or orders them \
                 with `is at most`, `is at least`, `is less than`, `is greater than` (or \
                 `<=`, `>=`, `<`, `>`); no other relation has a term grammar on this surface",
            ));
        };
        for _ in 0..relation.len {
            self.advance();
        }
        let right = match relation.carried {
            Some(n) => {
                let first = Self::carried_numeral(n, scope, self.interner);
                let product = self.parse_math_product_tail(scope, first)?;
                self.parse_math_sum_tail(scope, product)?
            }
            None => self.parse_math_sum(scope)?,
        };
        Ok(self.math_order(relation.order, left, right))
    }

    /// The first atom of an order's right operand when the lexer folded it into the relation
    /// token (`at least five`). The lexer read `five` as a number word, but SCOPE outranks the
    /// word table, so a binder that bound a name spelling that number keeps it a variable.
    fn carried_numeral(n: u32, scope: &[Symbol], interner: &logicaffeine_base::Interner) -> Term<'a> {
        scope
            .iter()
            .rev()
            .find(|bound| number_word(interner.resolve(**bound)) == Some(i64::from(n)))
            .map(|bound| Term::Variable(*bound))
            .unwrap_or_else(|| Self::math_integer(i64::from(n)))
    }

    /// `sum := product (('plus' | '+' | 'minus' | '-') product)*` — left-associative.
    fn parse_math_sum(&mut self, scope: &[Symbol]) -> ParseResult<Term<'a>> {
        let first = self.parse_math_product(scope)?;
        self.parse_math_sum_tail(scope, first)
    }

    /// The `(('plus' | 'minus') product)*` continuation of a sum whose first product is
    /// already in hand.
    fn parse_math_sum_tail(&mut self, scope: &[Symbol], first: Term<'a>) -> ParseResult<Term<'a>> {
        let mut left = first;
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
        let first = self.parse_math_atom(scope)?;
        self.parse_math_product_tail(scope, first)
    }

    /// The `(('times' | 'divided by') atom)*` continuation of a product whose first atom is
    /// already in hand.
    fn parse_math_product_tail(
        &mut self,
        scope: &[Symbol],
        first: Term<'a>,
    ) -> ParseResult<Term<'a>> {
        let mut left = first;
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

    /// `left ORDER right` as the proof layer's Prop: `le(a, b) = true` or `lt(a, b) = true`,
    /// with the operands swapped for the converse words.
    fn math_order(&mut self, order: Order, left: Term<'a>, right: Term<'a>) -> &'a LogicExpr<'a> {
        let (symbol, swap) = order.lowering();
        let operands = if swap { [right, left] } else { [left, right] };
        let comparison = self.math_apply(symbol, operands);
        let truth = Term::Constant(self.interner.intern("true"));
        self.ctx.exprs.alloc(LogicExpr::Identity {
            left: self.ctx.terms.alloc(comparison),
            right: self.ctx.terms.alloc(truth),
        })
    }

    fn math_error(&self, message: &str) -> ParseError {
        ParseError {
            kind: ParseErrorKind::Custom(message.to_string()),
            span: self.current_span(),
        }
    }
}
