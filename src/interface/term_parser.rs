//! Parser for Kernel Term syntax.
//!
//! Parses the following grammar:
//! ```text
//! term   ::= lambda | forall | fix | match | arrow
//! arrow  ::= app ("->" arrow)?
//! app    ::= atom+
//! atom   ::= "(" term ")" | sort | ident
//! sort   ::= "Prop" | "Type" | "Type" digit+
//! lambda ::= "fun" ident ":" term "=>" term
//! forall ::= "forall" ident ":" term "," term
//! fix    ::= "fix" ident "=>" term
//! match  ::= "match" term "return" term "with" ("|" ident+ "=>" term)+
//! ```

use crate::kernel::{Literal, Term, Universe};
use super::error::ParseError;
use std::collections::HashSet;

/// Recursive descent parser for Term syntax.
pub struct TermParser<'a> {
    input: &'a str,
    pos: usize,
    /// Variables currently in scope (bound by lambda, forall, fix, or match case)
    bound_vars: HashSet<String>,
}

impl<'a> TermParser<'a> {
    /// Parse a term from input string.
    pub fn parse(input: &'a str) -> Result<Term, ParseError> {
        let mut parser = Self {
            input,
            pos: 0,
            bound_vars: HashSet::new(),
        };
        let term = parser.parse_term()?;
        parser.skip_whitespace();
        Ok(term)
    }

    /// Parse a term (top-level).
    fn parse_term(&mut self) -> Result<Term, ParseError> {
        self.skip_whitespace();

        if self.peek_keyword("fun") {
            self.parse_lambda()
        } else if self.peek_keyword("forall") {
            self.parse_forall()
        } else if self.peek_keyword("fix") {
            self.parse_fix()
        } else if self.peek_keyword("match") {
            self.parse_match()
        } else {
            self.parse_arrow()
        }
    }

    /// Parse arrow type: A -> B -> C (right associative)
    fn parse_arrow(&mut self) -> Result<Term, ParseError> {
        let left = self.parse_app()?;

        self.skip_whitespace();
        if self.try_consume("->") {
            let right = self.parse_arrow()?;
            // A -> B is sugar for Π(_:A). B
            Ok(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(left),
                body_type: Box::new(right),
            })
        } else {
            Ok(left)
        }
    }

    /// Parse application: f x y z (left associative)
    fn parse_app(&mut self) -> Result<Term, ParseError> {
        let mut func = self.parse_atom()?;

        loop {
            self.skip_whitespace();
            // Check if we can parse another atom
            // Stop at: ), ->, =>, ,, |, with, return, end, ., EOF, or keywords
            if self.at_end()
                || self.peek_char(')')
                || self.peek_char('.')
                || self.peek_char(',')
                || self.peek_char('|')
                || self.peek_str("->")
                || self.peek_str("=>")
                || self.peek_keyword("with")
                || self.peek_keyword("return")
                || self.peek_keyword("end")
            {
                break;
            }

            let arg = self.parse_atom()?;
            func = Term::App(Box::new(func), Box::new(arg));
        }

        Ok(func)
    }

    /// Parse an atom: (term), number, string, sort, or identifier
    fn parse_atom(&mut self) -> Result<Term, ParseError> {
        self.skip_whitespace();

        // Check for negative number
        if self.peek_char('-') {
            if let Some(lit) = self.try_parse_negative_number() {
                return Ok(lit);
            }
        }

        // Check for positive number
        if let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                return self.parse_number_literal();
            }
        }

        // Check for string literal
        if self.peek_char('"') {
            return self.parse_string_literal();
        }

        if self.peek_char('(') {
            self.parse_parens()
        } else {
            self.parse_ident_or_sort()
        }
    }

    /// Parse a positive integer literal
    fn parse_number_literal(&mut self) -> Result<Term, ParseError> {
        let start = self.pos;

        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.advance();
            } else {
                break;
            }
        }

        let num_str = &self.input[start..self.pos];
        let value: i64 = num_str
            .parse()
            .map_err(|_| ParseError::InvalidNumber(num_str.to_string()))?;

        Ok(Term::Lit(Literal::Int(value)))
    }

    /// Parse a string literal: "contents"
    fn parse_string_literal(&mut self) -> Result<Term, ParseError> {
        self.expect_char('"')?;

        // Collect characters until closing quote
        let mut content = String::new();
        loop {
            match self.peek() {
                Some('"') => {
                    self.advance();
                    break;
                }
                Some('\\') => {
                    // Handle escape sequences
                    self.advance();
                    match self.peek() {
                        Some('n') => {
                            content.push('\n');
                            self.advance();
                        }
                        Some('t') => {
                            content.push('\t');
                            self.advance();
                        }
                        Some('\\') => {
                            content.push('\\');
                            self.advance();
                        }
                        Some('"') => {
                            content.push('"');
                            self.advance();
                        }
                        Some(c) => {
                            content.push(c);
                            self.advance();
                        }
                        None => return Err(ParseError::UnexpectedEof),
                    }
                }
                Some(c) => {
                    content.push(c);
                    self.advance();
                }
                None => return Err(ParseError::UnexpectedEof),
            }
        }

        Ok(Term::Lit(Literal::Text(content)))
    }

    /// Try to parse a negative number, returning None if not a number
    fn try_parse_negative_number(&mut self) -> Option<Term> {
        // Look ahead: - followed by digit
        if !self.peek_char('-') {
            return None;
        }
        let after_dash = self.pos + 1;
        if after_dash >= self.input.len() {
            return None;
        }
        let next = self.input[after_dash..].chars().next()?;
        if !next.is_ascii_digit() {
            return None;
        }

        // Consume the dash
        self.advance();

        // Parse the number
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.advance();
            } else {
                break;
            }
        }

        let num_str = &self.input[start..self.pos];
        let value: i64 = num_str.parse().ok()?;

        Some(Term::Lit(Literal::Int(-value)))
    }

    /// Parse parenthesized term
    fn parse_parens(&mut self) -> Result<Term, ParseError> {
        self.expect_char('(')?;
        let term = self.parse_term()?;
        self.skip_whitespace();
        self.expect_char(')')?;
        Ok(term)
    }

    /// Parse identifier or sort (Prop, Type, Type0, etc.)
    fn parse_ident_or_sort(&mut self) -> Result<Term, ParseError> {
        let ident = self.parse_ident()?;

        match ident.as_str() {
            "Prop" => Ok(Term::Sort(Universe::Prop)),
            "Type" => {
                // Check for Type followed by a number
                self.skip_whitespace();
                if let Some(n) = self.try_parse_number() {
                    Ok(Term::Sort(Universe::Type(n)))
                } else {
                    Ok(Term::Sort(Universe::Type(0)))
                }
            }
            _ => {
                // Check for TypeN (e.g., Type0, Type1)
                if ident.starts_with("Type") {
                    if let Ok(n) = ident[4..].parse::<u32>() {
                        return Ok(Term::Sort(Universe::Type(n)));
                    }
                }
                // Check if this is a bound variable
                if self.bound_vars.contains(&ident) {
                    Ok(Term::Var(ident))
                } else {
                    Ok(Term::Global(ident))
                }
            }
        }
    }

    /// Parse lambda: fun x : T => body
    fn parse_lambda(&mut self) -> Result<Term, ParseError> {
        self.consume_keyword("fun")?;
        self.skip_whitespace();
        let param = self.parse_ident()?;
        self.skip_whitespace();
        self.expect_char(':')?;
        let param_type = self.parse_term()?;
        self.skip_whitespace();
        self.expect_str("=>")?;

        // Add param to bound variables, parse body, then remove
        let was_bound = self.bound_vars.contains(&param);
        self.bound_vars.insert(param.clone());
        let body = self.parse_term()?;
        if !was_bound {
            self.bound_vars.remove(&param);
        }

        Ok(Term::Lambda {
            param,
            param_type: Box::new(param_type),
            body: Box::new(body),
        })
    }

    /// Parse forall: forall x : T, body
    fn parse_forall(&mut self) -> Result<Term, ParseError> {
        self.consume_keyword("forall")?;
        self.skip_whitespace();
        let param = self.parse_ident()?;
        self.skip_whitespace();
        self.expect_char(':')?;
        let param_type = self.parse_term()?;
        self.skip_whitespace();
        self.expect_char(',')?;

        // Add param to bound variables, parse body, then remove
        let was_bound = self.bound_vars.contains(&param);
        self.bound_vars.insert(param.clone());
        let body_type = self.parse_term()?;
        if !was_bound {
            self.bound_vars.remove(&param);
        }

        Ok(Term::Pi {
            param,
            param_type: Box::new(param_type),
            body_type: Box::new(body_type),
        })
    }

    /// Parse fix: fix f => body
    fn parse_fix(&mut self) -> Result<Term, ParseError> {
        self.consume_keyword("fix")?;
        self.skip_whitespace();
        let name = self.parse_ident()?;
        self.skip_whitespace();
        self.expect_str("=>")?;

        // Add name to bound variables, parse body, then remove
        let was_bound = self.bound_vars.contains(&name);
        self.bound_vars.insert(name.clone());
        let body = self.parse_term()?;
        if !was_bound {
            self.bound_vars.remove(&name);
        }

        Ok(Term::Fix {
            name,
            body: Box::new(body),
        })
    }

    /// Parse match: match e return M with | C1 x => t1 | C2 y z => t2 end
    fn parse_match(&mut self) -> Result<Term, ParseError> {
        self.consume_keyword("match")?;
        self.skip_whitespace();
        let discriminant = self.parse_app()?;
        self.skip_whitespace();
        self.consume_keyword("return")?;
        self.skip_whitespace();
        let motive = self.parse_app()?;
        self.skip_whitespace();
        self.consume_keyword("with")?;

        let mut cases = Vec::new();
        loop {
            self.skip_whitespace();
            if !self.try_consume("|") {
                break;
            }
            self.skip_whitespace();

            // Parse pattern: C x y z (constructor with binders)
            let case_term = self.parse_case_body()?;
            cases.push(case_term);
        }

        // Consume optional 'end' keyword
        self.skip_whitespace();
        let _ = self.try_consume_keyword("end");

        Ok(Term::Match {
            discriminant: Box::new(discriminant),
            motive: Box::new(motive),
            cases,
        })
    }

    /// Parse a match case body: C x y => term becomes λx. λy. term
    ///
    /// The first identifier is the constructor name (skipped), the rest are binders.
    fn parse_case_body(&mut self) -> Result<Term, ParseError> {
        // First identifier is the constructor name (skip it)
        self.skip_whitespace();
        let _ctor_name = self.parse_ident()?;

        // Collect binders until =>
        let mut binders = Vec::new();
        loop {
            self.skip_whitespace();
            if self.peek_str("=>") {
                break;
            }
            let ident = self.parse_ident()?;
            binders.push(ident);
        }
        self.expect_str("=>")?;

        // Add all binders to bound vars
        let mut previously_bound = Vec::new();
        for binder in &binders {
            previously_bound.push(self.bound_vars.contains(binder));
            self.bound_vars.insert(binder.clone());
        }

        let mut body = self.parse_term()?;

        // Remove binders that weren't previously bound
        for (i, binder) in binders.iter().enumerate() {
            if !previously_bound[i] {
                self.bound_vars.remove(binder);
            }
        }

        // Wrap in lambdas from right to left
        // We don't know the types, so we use a placeholder
        for binder in binders.into_iter().rev() {
            body = Term::Lambda {
                param: binder,
                param_type: Box::new(Term::Global("_".to_string())), // Placeholder type
                body: Box::new(body),
            };
        }

        Ok(body)
    }

    // =========================================================================
    // Low-level parsing utilities
    // =========================================================================

    /// Parse an identifier (alphanumeric + underscore, starting with letter/underscore)
    fn parse_ident(&mut self) -> Result<String, ParseError> {
        self.skip_whitespace();
        let start = self.pos;

        // First character must be letter or underscore
        if let Some(c) = self.peek() {
            if !c.is_alphabetic() && c != '_' {
                return Err(ParseError::Expected {
                    expected: "identifier".to_string(),
                    found: format!("{}", c),
                });
            }
        } else {
            return Err(ParseError::UnexpectedEof);
        }

        // Consume alphanumeric and underscore
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                self.advance();
            } else {
                break;
            }
        }

        let ident = self.input[start..self.pos].to_string();
        if ident.is_empty() {
            Err(ParseError::InvalidIdent("empty".to_string()))
        } else {
            Ok(ident)
        }
    }

    /// Try to parse a number, returning None if not present
    fn try_parse_number(&mut self) -> Option<u32> {
        self.skip_whitespace();
        let start = self.pos;

        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.advance();
            } else {
                break;
            }
        }

        if self.pos > start {
            self.input[start..self.pos].parse().ok()
        } else {
            None
        }
    }

    /// Skip whitespace
    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    /// Peek at current character
    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    /// Check if we're at a specific character
    fn peek_char(&self, c: char) -> bool {
        self.peek() == Some(c)
    }

    /// Check if input starts with a string at current position
    fn peek_str(&self, s: &str) -> bool {
        self.input[self.pos..].starts_with(s)
    }

    /// Check if input starts with a keyword (followed by non-alphanumeric)
    fn peek_keyword(&self, keyword: &str) -> bool {
        if !self.peek_str(keyword) {
            return false;
        }
        // Check that keyword is not part of a longer identifier
        let after = self.pos + keyword.len();
        if after >= self.input.len() {
            return true;
        }
        let next_char = self.input[after..].chars().next();
        !next_char.map(|c| c.is_alphanumeric() || c == '_').unwrap_or(false)
    }

    /// Advance position by one character
    fn advance(&mut self) {
        if let Some(c) = self.peek() {
            self.pos += c.len_utf8();
        }
    }

    /// Check if at end of input
    fn at_end(&self) -> bool {
        self.pos >= self.input.len()
    }

    /// Try to consume a string, returning true if successful
    fn try_consume(&mut self, s: &str) -> bool {
        if self.peek_str(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }

    /// Try to consume a keyword, returning true if successful
    fn try_consume_keyword(&mut self, keyword: &str) -> bool {
        self.skip_whitespace();
        if self.peek_keyword(keyword) {
            self.pos += keyword.len();
            true
        } else {
            false
        }
    }

    /// Expect a specific character
    fn expect_char(&mut self, expected: char) -> Result<(), ParseError> {
        self.skip_whitespace();
        if self.peek_char(expected) {
            self.advance();
            Ok(())
        } else {
            Err(ParseError::Expected {
                expected: format!("'{}'", expected),
                found: self.peek().map(|c| c.to_string()).unwrap_or("EOF".to_string()),
            })
        }
    }

    /// Expect a specific string
    fn expect_str(&mut self, expected: &str) -> Result<(), ParseError> {
        self.skip_whitespace();
        if self.try_consume(expected) {
            Ok(())
        } else {
            let found: String = self.input[self.pos..].chars().take(expected.len()).collect();
            Err(ParseError::Expected {
                expected: format!("'{}'", expected),
                found: if found.is_empty() { "EOF".to_string() } else { found },
            })
        }
    }

    /// Consume a keyword (must be followed by non-alphanumeric)
    fn consume_keyword(&mut self, keyword: &str) -> Result<(), ParseError> {
        self.skip_whitespace();
        if self.peek_keyword(keyword) {
            self.pos += keyword.len();
            Ok(())
        } else {
            Err(ParseError::Expected {
                expected: keyword.to_string(),
                found: self.peek().map(|c| c.to_string()).unwrap_or("EOF".to_string()),
            })
        }
    }

    /// Get remaining input (for debugging)
    #[allow(dead_code)]
    fn remaining(&self) -> &str {
        &self.input[self.pos..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_global() {
        let term = TermParser::parse("Zero").unwrap();
        assert!(matches!(term, Term::Global(ref s) if s == "Zero"));
    }

    #[test]
    fn test_parse_sort() {
        let term = TermParser::parse("Type").unwrap();
        assert!(matches!(term, Term::Sort(Universe::Type(0))));

        let term2 = TermParser::parse("Prop").unwrap();
        assert!(matches!(term2, Term::Sort(Universe::Prop)));
    }

    #[test]
    fn test_parse_app() {
        let term = TermParser::parse("Succ Zero").unwrap();
        if let Term::App(func, arg) = term {
            assert!(matches!(*func, Term::Global(ref s) if s == "Succ"));
            assert!(matches!(*arg, Term::Global(ref s) if s == "Zero"));
        } else {
            panic!("Expected App");
        }
    }

    #[test]
    fn test_parse_parens() {
        let term = TermParser::parse("(Succ Zero)").unwrap();
        assert!(matches!(term, Term::App(..)));
    }

    #[test]
    fn test_parse_arrow() {
        let term = TermParser::parse("Nat -> Nat").unwrap();
        if let Term::Pi { param, param_type, body_type } = term {
            assert_eq!(param, "_");
            assert!(matches!(*param_type, Term::Global(ref s) if s == "Nat"));
            assert!(matches!(*body_type, Term::Global(ref s) if s == "Nat"));
        } else {
            panic!("Expected Pi");
        }
    }

    #[test]
    fn test_parse_lambda() {
        let term = TermParser::parse("fun x : Nat => Succ x").unwrap();
        if let Term::Lambda { param, body, .. } = term {
            assert_eq!(param, "x");
            // Body should use Var for bound x
            if let Term::App(_, arg) = *body {
                assert!(matches!(*arg, Term::Var(ref s) if s == "x"));
            } else {
                panic!("Expected App in lambda body");
            }
        } else {
            panic!("Expected Lambda");
        }
    }

    #[test]
    fn test_parse_lambda_bound_var() {
        let term = TermParser::parse("fun n : Nat => Succ n").unwrap();
        if let Term::Lambda { body, .. } = term {
            if let Term::App(_, arg) = *body {
                assert!(
                    matches!(*arg, Term::Var(ref s) if s == "n"),
                    "Expected Var(n), got {:?}",
                    arg
                );
            } else {
                panic!("Expected App in lambda body");
            }
        } else {
            panic!("Expected Lambda");
        }
    }
}
