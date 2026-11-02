//! The nonstandard surface: the program's infinitesimals, where each stands among the others and
//! what is known about its reciprocal, the predicates `infinitesimal`, `limited` and `infinitely
//! close to` and the standard part — each also relative to an infinitesimal — the limit rule a
//! program takes, the search over every natural number, and a real number a function the program
//! defines gives by its digits or its approximations.
//!
//! ```text
//! Let dt be an infinitesimal whose reciprocal is even and not divisible by 3.
//! Let dx be an infinitesimal much smaller than dt.
//! Let dy be an infinitesimal much smaller than dt and much larger than dx.
//! If x is limited and x is not infinitesimal: …
//! Show the standard part of 3 + 2 * dt.
//! Show the standard part of dt + dx relative to dt.
//! At the limit, take the Cesàro mean.
//! Repeat for every natural n, checking up to 1000 cases: …
//! Let r be the real number whose digits in base 3 are primeDigit.
//! Let s be the real number approximated by rootTwoWithin.
//! ```
//!
//! Each form lowers to a call of the builtin that names it (`infinitesimal`, `orderedInfinitesimal`,
//! `isInfinitesimal`, `isLimited`, `infinitelyClose`, `standardPart`, `atTheLimit`, `everyNatural`,
//! `definedReal`), so every tier that runs builtins runs these, and nothing downstream needs a
//! construct of its own.
//!
//! An infinitesimal declared `much smaller than dt` is smaller than every positive number made from
//! `dt`, which is what orders every value of the two. A bare `smaller than dt` would leave `dx`
//! against `dt^2` open, and is refused with that sentence.
//!
//! The facts about the reciprocal are decided here, before the program runs: they intersect into
//! one residue type of the infinite natural `1/dt`, and a contradiction, or a system too fine for
//! one value to hold, is refused with the sentence that caused it.

use super::{ParseResult, Parser};
use crate::ast::stmt::{ClosureBody, Pattern, TypeExpr};
use crate::ast::{Expr, Literal, Stmt};
use crate::error::{ParseError, ParseErrorKind};
use crate::token::TokenType;
use logicaffeine_base::limit_value::{ResidueError, ResidueType, MAX_MODULUS};
use logicaffeine_base::numeric::BigInt;
use logicaffeine_base::Symbol;

/// The limit rules, each as written and as the runtime names it.
const LIMIT_RULES: [(&str, &str); 6] = [
    ("the standard part", "the standard part"),
    ("the limit", "the limit"),
    ("the limit superior", "the limit superior"),
    ("the limit inferior", "the limit inferior"),
    ("the Cesàro mean", "the Cesàro mean"),
    ("the Cesaro mean", "the Cesàro mean"),
];

/// How many cases a search over every natural checks when the program does not say.
pub const DEFAULT_SEARCH_CASES: i64 = 10_000_000;

impl<'a, 'ctx, 'int> Parser<'a, 'ctx, 'int> {
    fn custom_error(&self, message: String) -> ParseError {
        ParseError {
            kind: ParseErrorKind::Custom(message),
            span: self.current_span(),
        }
    }

    fn expect_nonstandard_word(&mut self, word: &str) -> ParseResult<()> {
        if !self.check_word(word) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword {
                    keyword: word.to_string(),
                },
                span: self.current_span(),
            });
        }
        self.advance();
        Ok(())
    }

    fn builtin_call(&mut self, name: &str, args: Vec<&'a Expr<'a>>) -> &'a Expr<'a> {
        let function = self.interner.intern(name);
        self.ctx.alloc_imperative_expr(Expr::Call { function, args })
    }

    fn number_literal(&mut self, n: i64) -> &'a Expr<'a> {
        self.ctx.alloc_imperative_expr(Expr::Literal(Literal::Number(n)))
    }

    fn text_literal(&mut self, text: &str) -> &'a Expr<'a> {
        let sym = self.interner.intern(text);
        self.ctx.alloc_imperative_expr(Expr::Literal(Literal::Text(sym)))
    }

    // ── the infinitesimal ─────────────────────────────────────────────────────────────────────

    /// After `Let <name> be`: `an infinitesimal [much smaller than <dt>] [and much larger than <dx>]
    /// [whose reciprocal is <fact> [and <fact>]…]`, as the value `infinitesimal("<name>", modulus,
    /// residue…)`, or `orderedInfinitesimal("<name>", larger, smaller, modulus, residue…)` with
    /// `nothing` for a bound not named; `None` when the value is anything else.
    pub(super) fn try_parse_infinitesimal(&mut self, var: Symbol) -> ParseResult<Option<&'a Expr<'a>>> {
        if !(self.check_article() && self.peek_word_at(1, "infinitesimal")) {
            return Ok(None);
        }
        self.advance(); // the article
        self.advance(); // "infinitesimal"
        let name = self.interner.resolve(var).to_string();
        let (larger, smaller) = self.parse_infinitesimal_bounds(&name)?;
        let mut residues = ResidueType::any();
        if self.check_word("whose") {
            self.advance();
            self.expect_nonstandard_word("reciprocal")?;
            self.expect_nonstandard_word("is")?;
            loop {
                let fact = self.parse_residue_fact(&name)?;
                residues = residues
                    .and(&fact)
                    .map_err(|e| self.residue_refusal(e, &name))?;
                if !(self.check(&TokenType::And) || self.check_word("and")) {
                    break;
                }
                self.advance(); // "and"
                if self.check_word("is") {
                    self.advance();
                }
            }
        }
        let mut args = vec![self.text_literal(&name)];
        let ordered = larger.is_some() || smaller.is_some();
        if ordered {
            let mut bound = |b: Option<&'a Expr<'a>>| {
                b.unwrap_or_else(|| self.ctx.alloc_imperative_expr(Expr::Literal(Literal::Nothing)))
            };
            let (larger, smaller) = (bound(larger), bound(smaller));
            args.push(larger);
            args.push(smaller);
        }
        args.push(self.number_literal(residues.modulus() as i64));
        for r in residues.residues() {
            args.push(self.number_literal(r as i64));
        }
        let builtin = if ordered { "orderedInfinitesimal" } else { "infinitesimal" };
        Ok(Some(self.builtin_call(builtin, args)))
    }

    /// `[much smaller than <larger>] [and much larger than <smaller>]`, in either order. A bare
    /// `smaller than` or `larger than` is refused with what it leaves undecided, and any other
    /// comparative is refused naming the two this language reads.
    #[allow(clippy::type_complexity)]
    fn parse_infinitesimal_bounds(
        &mut self,
        name: &str,
    ) -> ParseResult<(Option<&'a Expr<'a>>, Option<&'a Expr<'a>>)> {
        const COMPARATIVES: [&str; 8] = ["smaller", "larger", "less", "greater", "bigger", "lower", "higher", "tinier"];
        let comparative_at = |p: &Self, at: usize| {
            COMPARATIVES.iter().any(|w| p.peek_word_at(at, w)) && p.peek_word_at(at + 1, "than")
        };
        let (mut larger, mut smaller) = (None, None);
        loop {
            let much = self.check_word("much");
            let at = usize::from(much);
            if !comparative_at(self, at) {
                if much {
                    return Err(self.custom_error(
                        "an infinitesimal is declared `much smaller than` or `much larger than` another".to_string(),
                    ));
                }
                break;
            }
            let word = self.interner.resolve(self.tokens[self.current + at].lexeme).to_ascii_lowercase();
            let bound_text = self
                .tokens
                .get(self.current + at + 2)
                .map(|t| self.interner.resolve(t.lexeme).to_string())
                .unwrap_or_default();
            if !matches!(word.as_str(), "smaller" | "larger") {
                return Err(self.custom_error(
                    "an infinitesimal is declared `much smaller than` or `much larger than` another".to_string(),
                ));
            }
            if !much {
                return Err(self.custom_error(if word == "smaller" {
                    format!(
                        "{name} smaller than {bound_text} is not enough to compare {name} with {bound_text}^2, \
                         {bound_text}^3 and every other value made from {bound_text}; say `much smaller than \
                         {bound_text}` for an infinitesimal smaller than all of them, or make {name} from \
                         {bound_text}, like `Let {name} be {bound_text} ** 2.`"
                    )
                } else {
                    format!(
                        "{name} larger than {bound_text} is not enough to compare {name} with √{bound_text}, \
                         ∛{bound_text} and every other root of {bound_text}; say `much larger than \
                         {bound_text}` for an infinitesimal larger than all of them, or make {name} from \
                         {bound_text}, like `Let {name} be sqrt({bound_text}).`"
                    )
                }));
            }
            for _ in 0..3 {
                self.advance(); // "much <comparative> than"
            }
            let bound = self.parse_xor_expr()?;
            let slot = if word == "smaller" { &mut larger } else { &mut smaller };
            if slot.is_some() {
                return Err(self.custom_error(format!(
                    "an infinitesimal is declared much {word} than one other infinitesimal, not two"
                )));
            }
            *slot = Some(bound);
            let and = self.check(&TokenType::And) || self.check_word("and");
            if and && self.peek_word_at(1, "much") {
                self.advance(); // "and"
                continue;
            }
            break;
        }
        Ok((larger, smaller))
    }

    fn residue_refusal(&self, e: ResidueError, name: &str) -> ParseError {
        self.custom_error(e.describe(&format!("1/{name}")))
    }

    /// One fact about the reciprocal: `even`, `odd`, `divisible by <k>`, `a multiple of <k>` or
    /// `congruent to <r> modulo <k>`, each optionally after `not`.
    fn parse_residue_fact(&mut self, name: &str) -> ParseResult<ResidueType> {
        let negated = self.check_word("not") || self.check(&TokenType::Not);
        if negated {
            self.advance();
        }
        let (modulus, residue) = if self.check_word("even") || self.check_word("odd") {
            let odd = self.check_word("odd");
            self.advance();
            (BigInt::from_u64(2), BigInt::from_u64(u64::from(odd)))
        } else if self.check_word("divisible") {
            self.advance();
            self.expect_nonstandard_word("by")?;
            (self.residue_modulus(name)?, BigInt::from_u64(0))
        } else if self.check_article() && self.peek_word_at(1, "multiple") && self.peek_word_at(2, "of") {
            self.advance();
            self.advance();
            self.advance();
            (self.residue_modulus(name)?, BigInt::from_u64(0))
        } else if self.check_word("congruent") {
            self.advance();
            self.expect_nonstandard_word("to")?;
            let residue = self.residue_integer(name)?;
            if self.check_word("modulo") || self.check_word("mod") {
                self.advance();
            } else {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword {
                        keyword: "modulo".to_string(),
                    },
                    span: self.current_span(),
                });
            }
            (self.residue_modulus(name)?, residue)
        } else {
            let word = self.interner.resolve(self.peek().lexeme).to_string();
            return Err(self.custom_error(format!(
                "`{word}` is not a fact about 1/{name} this language reads; say even, odd, divisible by \
                 <k>, a multiple of <k> or congruent to <r> modulo <k>, each optionally after `not`"
            )));
        };
        let m = match modulus.to_u64().filter(|m| *m <= MAX_MODULUS) {
            Some(m) => m,
            None => {
                return Err(self.residue_refusal(ResidueError::TooFine { classes: modulus }, name));
            }
        };
        let r = residue
            .div_rem(&BigInt::from_u64(m))
            .expect("a positive modulus")
            .1;
        let r = if r.is_negative() { r.add(&BigInt::from_u64(m)) } else { r };
        let r = r.to_u64().expect("a residue below the modulus");
        let fact = ResidueType::from_allowed(m, &[r]).map_err(|e| self.residue_refusal(e, name))?;
        if !negated {
            return Ok(fact);
        }
        let kept = fact.residues();
        let complement: Vec<u64> = (0..fact.modulus()).filter(|x| !kept.contains(x)).collect();
        ResidueType::from_allowed(fact.modulus(), &complement).map_err(|e| self.residue_refusal(e, name))
    }

    /// A whole number written out, optionally negative.
    fn residue_integer(&mut self, name: &str) -> ParseResult<BigInt> {
        let negative = self.check(&TokenType::Minus);
        let at = usize::from(negative);
        let literal = self.tokens.get(self.current + at).and_then(|t| match t.kind {
            TokenType::Number(sym) => {
                BigInt::parse_decimal(&self.interner.resolve(sym).replace('_', ""))
            }
            _ => None,
        });
        let Some(n) = literal else {
            return Err(self.custom_error(format!(
                "a fact about 1/{name} names a whole number written out, like `divisible by 6`"
            )));
        };
        for _ in 0..=at {
            self.advance();
        }
        Ok(if negative { n.negated() } else { n })
    }

    /// A modulus: a positive whole number written out.
    fn residue_modulus(&mut self, name: &str) -> ParseResult<BigInt> {
        let k = self.residue_integer(name)?;
        if !k.is_positive() {
            return Err(self.custom_error(format!(
                "a fact about 1/{name} divides by a positive whole number"
            )));
        }
        Ok(k)
    }

    // ── the predicates ────────────────────────────────────────────────────────────────────────

    /// After `<left> is`: `[not] infinitesimal`, `[not] limited` or `[not] infinitely close to
    /// <right>`. A program's own binding of the adjective's name wins (`x is limited` with a
    /// variable `limited` is the equality it has always been), so `None` there.
    pub(super) fn try_parse_nonstandard_predicate(
        &mut self,
        left: &'a Expr<'a>,
    ) -> ParseResult<Option<&'a Expr<'a>>> {
        let negated = self.check_word("not") || self.check(&TokenType::Not);
        let at = usize::from(negated);
        let unbound = self
            .tokens
            .get(self.current + at)
            .is_some_and(|t| !self.user_bound.contains(&t.lexeme));
        if !unbound {
            return Ok(None);
        }
        let skip = |p: &mut Self, n: usize| {
            for _ in 0..n {
                p.advance();
            }
        };
        let call = if self.peek_word_at(at, "infinitesimal") {
            skip(self, at + 1);
            let args = self.relative_to(vec![left])?;
            self.builtin_call("isInfinitesimal", args)
        } else if self.peek_word_at(at, "limited") {
            skip(self, at + 1);
            let args = self.relative_to(vec![left])?;
            self.builtin_call("isLimited", args)
        } else if self.tokens.get(self.current + at).is_some_and(|t| {
            matches!(t.kind, TokenType::Preposition(sym) if self.interner.resolve(sym) == "InfinitelyClose")
        }) {
            skip(self, at + 1);
            let right = self.parse_xor_expr()?;
            let args = self.relative_to(vec![left, right])?;
            self.builtin_call("infinitelyClose", args)
        } else {
            return Ok(None);
        };
        Ok(Some(if negated {
            self.ctx.alloc_imperative_expr(Expr::Not { operand: call })
        } else {
            call
        }))
    }

    /// `args` and, after `relative to <infinitesimal>`, that infinitesimal: the standpoint a question
    /// is asked from.
    fn relative_to(&mut self, mut args: Vec<&'a Expr<'a>>) -> ParseResult<Vec<&'a Expr<'a>>> {
        if self.check_word("relative") && self.peek_word_at(1, "to") {
            self.advance(); // "relative"
            self.advance(); // "to"
            args.push(self.parse_xor_expr()?);
        }
        Ok(args)
    }

    /// `the standard part of <expr> [relative to <infinitesimal>]` — the operand runs to the end of
    /// the arithmetic, so `the standard part of 3 + 2 * dt` is `3`, and a comparison after it stays
    /// outside.
    #[inline(never)]
    pub(super) fn parse_standard_part(&mut self) -> ParseResult<&'a Expr<'a>> {
        for _ in 0..4 {
            self.advance(); // "the standard part of"
        }
        let operand = self.parse_additive_expr()?;
        let args = self.relative_to(vec![operand])?;
        Ok(self.builtin_call("standardPart", args))
    }

    // ── limit rules ───────────────────────────────────────────────────────────────────────────

    /// True at `At the limit, take …`.
    pub(super) fn at_limit_rule(&self) -> bool {
        self.lexeme_is(0, "At")
            && self.lexeme_is(1, "the")
            && self.lexeme_is(2, "limit")
            && self
                .tokens
                .get(self.current + 3)
                .is_some_and(|t| matches!(t.kind, TokenType::Comma))
            && self.lexeme_is(4, "take")
    }

    /// `At the limit, take <rule>.` as the statement `atTheLimit("<rule>")`.
    pub(super) fn parse_limit_rule(&mut self) -> ParseResult<Stmt<'a>> {
        for _ in 0..5 {
            self.advance(); // "At the limit , take"
        }
        let mut words = Vec::new();
        while !self.is_at_end()
            && !matches!(
                self.peek().kind,
                TokenType::Period | TokenType::Newline | TokenType::EOF
            )
        {
            words.push(self.interner.resolve(self.peek().lexeme).to_string());
            self.advance();
        }
        let written = words.join(" ");
        let Some((_, rule)) = LIMIT_RULES
            .iter()
            .find(|(spelled, _)| spelled.eq_ignore_ascii_case(&written))
        else {
            return Err(self.custom_error(format!(
                "`{written}` is not a limit rule; take the standard part, the limit, the limit \
                 superior, the limit inferior or the Cesàro mean"
            )));
        };
        let rule = self.text_literal(rule);
        let function = self.interner.intern("atTheLimit");
        Ok(Stmt::Call {
            function,
            args: vec![rule],
        })
    }

    // ── real numbers a program defines ────────────────────────────────────────────────────────

    /// At `the real number`: `whose digits in base <k> are <function>` or `approximated by
    /// <function>`, as `definedReal("digits", k, "<function>", (n: Int) -> <function>(n))` or
    /// `definedReal("approximations", 2, "<function>", …)`; `None` when the phrase is anything else.
    /// The function is one the program defines, taking the digit's place or the precision `n` and
    /// answering the digit or a rational within `1/2^n` of the number.
    pub(super) fn try_parse_defined_real(&mut self) -> ParseResult<Option<&'a Expr<'a>>> {
        let opens = self.check_word("the")
            && self.peek_word_at(1, "real")
            && self.peek_word_at(2, "number")
            && (self.peek_word_at(3, "whose") || self.peek_word_at(3, "approximated"));
        if !opens {
            return Ok(None);
        }
        for _ in 0..3 {
            self.advance(); // "the real number"
        }
        let (kind, base) = if self.check_word("whose") {
            self.advance();
            self.expect_nonstandard_word("digits")?;
            self.expect_nonstandard_word("in")?;
            self.expect_nonstandard_word("base")?;
            let written = match self.peek().kind {
                TokenType::Number(sym) => BigInt::parse_decimal(&self.interner.resolve(sym).replace('_', "")),
                _ => None,
            };
            let base = written
                .and_then(|b| b.to_u64())
                .filter(|b| (2..=u64::from(u32::MAX)).contains(b))
                .ok_or_else(|| {
                    self.custom_error(
                        "a real number's digits are in a base from 2 up, written out, like `base 10`".to_string(),
                    )
                })?;
            self.advance();
            self.expect_nonstandard_word("are")?;
            ("digits", base as i64)
        } else {
            self.advance(); // "approximated"
            self.expect_nonstandard_word("by")?;
            ("approximations", 2)
        };
        let function = self.expect_identifier()?;
        let name = self.interner.resolve(function).to_string();
        let index = self.interner.intern("n");
        let int = self.interner.intern("Int");
        let int_type = self.ctx.alloc_type_expr(TypeExpr::Primitive(int));
        let read = self.ctx.alloc_imperative_expr(Expr::Identifier(index));
        let body = self.ctx.alloc_imperative_expr(Expr::Call {
            function,
            args: vec![read],
        });
        let closure = self.ctx.alloc_imperative_expr(Expr::Closure {
            params: vec![(index, int_type)],
            body: ClosureBody::Expression(body),
            return_type: None,
        });
        let args = vec![
            self.text_literal(kind),
            self.number_literal(base),
            self.text_literal(&name),
            closure,
        ];
        Ok(Some(self.builtin_call("definedReal", args)))
    }

    // ── searches over every natural ───────────────────────────────────────────────────────────

    /// After `Repeat for`: `every natural [number] <n>[, checking up to <k> cases]:` and its body,
    /// as a loop over `everyNatural(k)`; `None` when the header is any other loop.
    pub(super) fn try_parse_every_natural_loop(&mut self) -> ParseResult<Option<Stmt<'a>>> {
        if !(self.check_word("every") && self.peek_word_at(1, "natural")) {
            return Ok(None);
        }
        self.advance(); // "every"
        self.advance(); // "natural"
        if self.check_word("number") {
            self.advance();
        }
        let var = self.expect_identifier()?;
        let mut cases = DEFAULT_SEARCH_CASES;
        if self.check(&TokenType::Comma) && self.peek_word_at(1, "checking") {
            self.advance(); // ","
            self.advance(); // "checking"
            self.expect_nonstandard_word("up")?;
            self.expect_nonstandard_word("to")?;
            cases = match self.peek().kind {
                TokenType::Number(sym) => self
                    .interner
                    .resolve(sym)
                    .replace('_', "")
                    .parse::<i64>()
                    .ok()
                    .filter(|k| *k > 0)
                    .ok_or_else(|| {
                        self.custom_error(
                            "a search checks a positive whole number of cases".to_string(),
                        )
                    })?,
                _ => {
                    return Err(self.custom_error(
                        "a search checks a number of cases written out, like `checking up to 1000 cases`"
                            .to_string(),
                    ))
                }
            };
            self.advance();
            self.expect_nonstandard_word("cases")?;
        }
        if !self.check(&TokenType::Colon) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword {
                    keyword: ":".to_string(),
                },
                span: self.current_span(),
            });
        }
        self.advance(); // ":"
        let cases = self.number_literal(cases);
        let iterable = self.builtin_call("everyNatural", vec![cases]);
        let body = self.parse_indented_block()?;
        Ok(Some(Stmt::Repeat {
            pattern: Pattern::Identifier(var),
            iterable,
            body,
        }))
    }
}
