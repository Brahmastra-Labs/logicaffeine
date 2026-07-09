//! A self-contained parser from formal first-order-logic surface text to [`ProofExpr`].
//!
//! The natural-language front end (`logicaffeine_language`) is excellent at English —
//! unary predicates, simple binary relations from transitive verbs — but a geometry
//! axiom base needs *formal* relations of higher arity (`Cong(a,b,c,d)`, `Bet(a,b,c)`)
//! and explicit quantifier prefixes that no English sentence expresses. This module is
//! the seam: a small recursive-descent parser, deliberately independent of the 12k-line
//! NL parser (the same self-containment that `tactic_script` has), turning a line like
//!
//! ```text
//! for all a b c d e f, if Cong(a, b, c, d) and Cong(a, b, e, f) then Cong(c, d, e, f)
//! ```
//!
//! directly into the [`ProofExpr`] the prover and the multi-theorem driver already
//! consume. It is the surface for `## Axiom` / `## Theory` (the seam for Tarski).
//!
//! # Conventions
//!
//! * An identifier that leads with an uppercase letter or a digit is a CONSTANT
//!   (`ProofTerm::Constant`) — a fixed point like `P`, `Q`, `A`; one that leads with a
//!   lowercase letter is a VARIABLE (`ProofTerm::Variable`) — a bound point like `a`,
//!   `b`. This matches the verifier's standard FOL reading (a lowercase-leading symbol
//!   is a variable).
//! * Connectives, lowest precedence first: quantifiers (`for all`, `there exists`) <
//!   `iff` < implication (`if … then …`, `implies`, `->`) < `or` < `and` < `not` <
//!   atoms. Implication is right-associative.
//! * Both an English-flavoured spelling and a symbolic one are accepted: `and`/`∧`,
//!   `or`/`∨`, `not`/`¬`, `implies`/`->`/`→`, `iff`/`<->`/`↔`, `for all`/`forall`/`∀`,
//!   `there exists`/`exists`/`∃`.

use crate::{ProofExpr, ProofTerm};

/// A failure to parse formal-logic surface text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormulaError {
    pub message: String,
}

impl FormulaError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        FormulaError { message: message.into() }
    }
}

impl std::fmt::Display for FormulaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "formula parse error: {}", self.message)
    }
}

impl std::error::Error for FormulaError {}

/// Parse formal first-order-logic surface text into a [`ProofExpr`].
///
/// See the module docs for the grammar and the constant/variable convention. Returns a
/// [`FormulaError`] on malformed input (the whole string must be consumed).
pub fn parse_formula(input: &str) -> Result<ProofExpr, FormulaError> {
    let tokens = tokenize(input)?;
    let mut p = Parser { tokens: &tokens, pos: 0 };
    let expr = p.parse_formula()?;
    if p.pos != p.tokens.len() {
        return Err(FormulaError::new(format!(
            "unexpected trailing input near token {:?}",
            p.tokens.get(p.pos)
        )));
    }
    Ok(expr)
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Ident(String),
    LParen,
    RParen,
    Comma,
    Eq,
    Arrow,    // -> →   (implication)
    Iff,      // <-> ↔
    And,      // ∧
    Or,       // ∨
    Not,      // ¬
    Forall,   // ∀
    Exists,   // ∃
}

fn tokenize(input: &str) -> Result<Vec<Tok>, FormulaError> {
    let chars: Vec<char> = input.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        match c {
            '(' => {
                toks.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                toks.push(Tok::RParen);
                i += 1;
            }
            ',' => {
                toks.push(Tok::Comma);
                i += 1;
            }
            // A bare '.' is a benign body/clause terminator (e.g. ending a sentence);
            // treated as whitespace so `forall a b. P` and `forall a b, P` both parse.
            '.' | ';' => {
                i += 1;
            }
            '=' => {
                toks.push(Tok::Eq);
                i += 1;
            }
            '∧' => {
                toks.push(Tok::And);
                i += 1;
            }
            '∨' => {
                toks.push(Tok::Or);
                i += 1;
            }
            '¬' => {
                toks.push(Tok::Not);
                i += 1;
            }
            '∀' => {
                toks.push(Tok::Forall);
                i += 1;
            }
            '∃' => {
                toks.push(Tok::Exists);
                i += 1;
            }
            '→' => {
                toks.push(Tok::Arrow);
                i += 1;
            }
            '↔' => {
                toks.push(Tok::Iff);
                i += 1;
            }
            '-' if i + 1 < chars.len() && chars[i + 1] == '>' => {
                toks.push(Tok::Arrow);
                i += 2;
            }
            '<' if i + 2 < chars.len() && chars[i + 1] == '-' && chars[i + 2] == '>' => {
                toks.push(Tok::Iff);
                i += 3;
            }
            c if c.is_alphanumeric() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                toks.push(Tok::Ident(chars[start..i].iter().collect()));
            }
            other => {
                return Err(FormulaError::new(format!("unexpected character '{other}'")));
            }
        }
    }
    Ok(toks)
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

struct Parser<'a> {
    tokens: &'a [Tok],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<&Tok> {
        let t = self.tokens.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    /// A lowercased keyword `Ident` at the cursor, if any (for the English spellings).
    fn peek_kw(&self) -> Option<String> {
        match self.peek() {
            Some(Tok::Ident(s)) => Some(s.to_lowercase()),
            _ => None,
        }
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.peek_kw().as_deref() == Some(kw) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// Whether a quantifier prefix begins at the cursor — `for all`/`forall`/`∀`,
    /// `there exists`/`exists`/`some`/`∃`. A quantifier may appear as an OPERAND (e.g. the
    /// consequent of `if … then ∃x. …`), not only at the top, so it extends as far right
    /// as possible from wherever it starts — standard FOL prefix-quantifier scope.
    fn peek_is_quantifier(&self) -> bool {
        matches!(self.peek(), Some(Tok::Forall) | Some(Tok::Exists))
            || matches!(
                self.peek_kw().as_deref(),
                Some("forall") | Some("for") | Some("exists") | Some("there") | Some("some")
            )
    }

    // formula := quantifier | iff
    fn parse_formula(&mut self) -> Result<ProofExpr, FormulaError> {
        // Quantifiers: `for all <vars> , body` / `forall …` / `∀ …`, and the existential
        // analogues. The variable list is space-separated idents up to `,`/`.` (consumed
        // as whitespace by the tokenizer) — the comma is optional.
        if matches!(self.peek(), Some(Tok::Forall)) {
            self.advance();
            return self.finish_quantifier(true);
        }
        if matches!(self.peek(), Some(Tok::Exists)) {
            self.advance();
            return self.finish_quantifier(false);
        }
        if self.eat_kw("forall") {
            return self.finish_quantifier(true);
        }
        if self.eat_kw("for") {
            if !self.eat_kw("all") {
                return Err(FormulaError::new("expected 'all' after 'for'"));
            }
            return self.finish_quantifier(true);
        }
        if self.eat_kw("exists") {
            return self.finish_quantifier(false);
        }
        if self.eat_kw("there") {
            if !(self.eat_kw("exists") || self.eat_kw("is") || self.eat_kw("are")) {
                return Err(FormulaError::new("expected 'exists' after 'there'"));
            }
            return self.finish_quantifier(false);
        }
        if self.eat_kw("some") {
            return self.finish_quantifier(false);
        }
        self.parse_iff()
    }

    /// After a quantifier keyword: read the bound variables, then `,` (optional), then
    /// the body, nesting so the FIRST variable is the OUTERMOST binder.
    fn finish_quantifier(&mut self, universal: bool) -> Result<ProofExpr, FormulaError> {
        let mut vars = Vec::new();
        while let Some(Tok::Ident(s)) = self.peek() {
            // Stop if this ident is actually the start of the body (a keyword), not a var.
            let low = s.to_lowercase();
            if matches!(low.as_str(), "if" | "not" | "true" | "false") {
                break;
            }
            vars.push(s.clone());
            self.advance();
        }
        if vars.is_empty() {
            return Err(FormulaError::new("quantifier with no bound variables"));
        }
        // An optional separating comma.
        if matches!(self.peek(), Some(Tok::Comma)) {
            self.advance();
        }
        let body = self.parse_formula()?;
        Ok(vars.into_iter().rev().fold(body, |acc, var| {
            if universal {
                ProofExpr::ForAll { variable: var, body: Box::new(acc) }
            } else {
                ProofExpr::Exists { variable: var, body: Box::new(acc) }
            }
        }))
    }

    // iff := implication (('iff' | '<->') implication)?
    fn parse_iff(&mut self) -> Result<ProofExpr, FormulaError> {
        let left = self.parse_implication()?;
        if matches!(self.peek(), Some(Tok::Iff)) || self.peek_kw().as_deref() == Some("iff") {
            self.advance();
            let right = self.parse_implication()?;
            return Ok(ProofExpr::Iff(Box::new(left), Box::new(right)));
        }
        Ok(left)
    }

    // implication := 'if' disjunction 'then' implication
    //              | disjunction (('implies'|'->') implication)?     (right-assoc)
    fn parse_implication(&mut self) -> Result<ProofExpr, FormulaError> {
        if self.eat_kw("if") {
            let cond = self.parse_disjunction()?;
            if !self.eat_kw("then") {
                return Err(FormulaError::new("expected 'then' to close an 'if'"));
            }
            let conseq = self.parse_implication()?;
            return Ok(ProofExpr::Implies(Box::new(cond), Box::new(conseq)));
        }
        let left = self.parse_disjunction()?;
        if matches!(self.peek(), Some(Tok::Arrow)) || self.peek_kw().as_deref() == Some("implies") {
            self.advance();
            let right = self.parse_implication()?;
            return Ok(ProofExpr::Implies(Box::new(left), Box::new(right)));
        }
        Ok(left)
    }

    // disjunction := conjunction (('or'|'∨') conjunction)*
    fn parse_disjunction(&mut self) -> Result<ProofExpr, FormulaError> {
        let mut left = self.parse_conjunction()?;
        loop {
            if matches!(self.peek(), Some(Tok::Or)) || self.peek_kw().as_deref() == Some("or") {
                self.advance();
                let right = self.parse_conjunction()?;
                left = ProofExpr::Or(Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        Ok(left)
    }

    // conjunction := negation (('and'|'∧') negation)*
    fn parse_conjunction(&mut self) -> Result<ProofExpr, FormulaError> {
        let mut left = self.parse_negation()?;
        loop {
            if matches!(self.peek(), Some(Tok::And)) || self.peek_kw().as_deref() == Some("and") {
                self.advance();
                let right = self.parse_negation()?;
                left = ProofExpr::And(Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        Ok(left)
    }

    // negation := ('not'|'¬') negation | atom
    fn parse_negation(&mut self) -> Result<ProofExpr, FormulaError> {
        if matches!(self.peek(), Some(Tok::Not)) || self.peek_kw().as_deref() == Some("not") {
            self.advance();
            let inner = self.parse_negation()?;
            return Ok(ProofExpr::Not(Box::new(inner)));
        }
        self.parse_atom()
    }

    // atom := '(' formula ')'
    //       | 'true' | 'false'
    //       | ident '(' term (',' term)* ')'        (predicate / relation)
    //       | term '=' term                          (identity)
    //       | ident                                  (0-ary propositional atom)
    fn parse_atom(&mut self) -> Result<ProofExpr, FormulaError> {
        // A quantifier as an operand (`… then ∃x. …`, `… and ∀y. …`) — delegate to the
        // full-formula entry, which consumes the prefix and grabs its body rightward.
        if self.peek_is_quantifier() {
            return self.parse_formula();
        }

        if matches!(self.peek(), Some(Tok::LParen)) {
            self.advance();
            let inner = self.parse_formula()?;
            if !matches!(self.advance(), Some(Tok::RParen)) {
                return Err(FormulaError::new("expected ')'"));
            }
            return Ok(inner);
        }

        let name = match self.advance() {
            Some(Tok::Ident(s)) => s.clone(),
            other => {
                return Err(FormulaError::new(format!(
                    "expected an atom, found {other:?}"
                )))
            }
        };

        match name.to_lowercase().as_str() {
            "true" => return Ok(ProofExpr::Atom("True".to_string())),
            "false" => return Ok(ProofExpr::Atom("False".to_string())),
            _ => {}
        }

        // Relation application: `Name(t1, t2, …)`.
        if matches!(self.peek(), Some(Tok::LParen)) {
            self.advance();
            let mut args = Vec::new();
            if !matches!(self.peek(), Some(Tok::RParen)) {
                loop {
                    args.push(self.parse_term()?);
                    if matches!(self.peek(), Some(Tok::Comma)) {
                        self.advance();
                        continue;
                    }
                    break;
                }
            }
            if !matches!(self.advance(), Some(Tok::RParen)) {
                return Err(FormulaError::new("expected ')' to close a relation's arguments"));
            }
            return Ok(ProofExpr::Predicate { name, args, world: None });
        }

        // Identity: `a = b`.
        if matches!(self.peek(), Some(Tok::Eq)) {
            self.advance();
            let rhs = self.parse_term()?;
            return Ok(ProofExpr::Identity(ident_to_term(&name), rhs));
        }

        // A bare identifier is a 0-ary propositional atom.
        Ok(ProofExpr::Atom(name))
    }

    fn parse_term(&mut self) -> Result<ProofTerm, FormulaError> {
        match self.advance() {
            Some(Tok::Ident(s)) => Ok(ident_to_term(s)),
            other => Err(FormulaError::new(format!("expected a term, found {other:?}"))),
        }
    }
}

/// An uppercase- or digit-leading identifier is a CONSTANT; a lowercase-leading one is a
/// VARIABLE — the standard FOL reading the verifier already uses.
fn ident_to_term(s: &str) -> ProofTerm {
    let leads_constant = s
        .chars()
        .next()
        .map(|c| c.is_uppercase() || c.is_ascii_digit())
        .unwrap_or(false);
    if leads_constant {
        ProofTerm::Constant(s.to_string())
    } else {
        ProofTerm::Variable(s.to_string())
    }
}
