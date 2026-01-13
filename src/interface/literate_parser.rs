//! Literate Specification Parser for the Kernel.
//!
//! This module parses English-like "Literate" syntax and emits the same
//! `Command` and `Term` structures as the Coq-style parser.
//!
//! # Supported Syntax
//!
//! ## Inductive Types
//! ```text
//! A Bool is either Yes or No.
//! A Nat is either:
//!     Zero.
//!     a Successor with pred: Nat.
//! ```
//!
//! ## Function Definitions (with implicit recursion)
//! ```text
//! ## To add (n: Nat) and (m: Nat) -> Nat:
//!     Consider n:
//!         When Zero: Yield m.
//!         When Successor k: Yield Successor (add k m).
//! ```
//!
//! ## Pattern Matching
//! ```text
//! Consider x:
//!     When Zero: Yield 0.
//!     When Successor k: Yield k.
//! ```
//!
//! ## Lambda Expressions
//! ```text
//! given x: Nat yields Successor x
//! |x: Nat| -> Successor x
//! ```

use crate::kernel::{Literal, Term, Universe};
use super::command::Command;
use super::error::ParseError;
use std::collections::HashSet;

// ============================================================
// PUBLIC API (called by command_parser.rs dispatcher)
// ============================================================

/// Parse "A [Name] is either..." into Command::Inductive
///
/// Supports:
/// - `A Bool is either Yes or No.`
/// - `A Nat is either: Zero. a Successor with pred: Nat.`
/// - `A List of (T: Type) is either: Empty. a Node with head: T and tail: List of T.`
pub fn parse_inductive(input: &str) -> Result<Command, ParseError> {
    let mut parser = LiterateParser::new(input);
    parser.parse_literate_inductive()
}

/// Parse "## To [name] (params) -> RetType:" into Command::Definition
///
/// Handles implicit fixpoints: if the function name appears in the body,
/// the body is automatically wrapped in Term::Fix.
pub fn parse_definition(input: &str) -> Result<Command, ParseError> {
    let mut parser = LiterateParser::new(input);
    parser.parse_literate_definition()
}

/// Parse "Let [name] be [term]." into Command::Definition (constant, not function)
///
/// This is for simple constant bindings like:
/// - `Let T be Apply(Name "Not", Variable 0).`
/// - `Let G be the diagonalization of T.`
pub fn parse_let_definition(input: &str) -> Result<Command, ParseError> {
    let mut parser = LiterateParser::new(input);
    parser.parse_literate_let()
}

/// Parse "## Theorem: [name]\n    Statement: [proposition]." into Command::Definition
///
/// This is for theorem declarations like:
/// - `## Theorem: Godel_First_Incompleteness\n    Statement: Consistent implies Not(Provable(G)).`
pub fn parse_theorem(input: &str) -> Result<Command, ParseError> {
    let mut parser = LiterateParser::new(input);
    parser.parse_literate_theorem()
}

// ============================================================
// PARSER STATE
// ============================================================

struct LiterateParser<'a> {
    input: &'a str,
    pos: usize,
    bound_vars: HashSet<String>,
    current_function: Option<String>,
    /// The return type of the current function (used as motive in Consider)
    return_type: Option<Term>,
}

impl<'a> LiterateParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            pos: 0,
            bound_vars: HashSet::new(),
            current_function: None,
            return_type: None,
        }
    }

    // ============================================================
    // INDUCTIVE TYPE PARSING
    // ============================================================

    /// Parse: A [Name] (params)? is either [variants]
    fn parse_literate_inductive(&mut self) -> Result<Command, ParseError> {
        self.skip_whitespace();

        // Consume "A" or "An"
        if self.try_consume_keyword("An") || self.try_consume_keyword("A") {
            // Good
        } else {
            return Err(ParseError::Expected {
                expected: "'A' or 'An'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }

        self.skip_whitespace();

        // Parse the type name
        let name = self.parse_ident()?;

        self.skip_whitespace();

        // Check for type parameters: "of (T: Type)"
        let params = if self.try_consume_keyword("of") {
            self.skip_whitespace();
            self.parse_param_list()?
        } else {
            vec![]
        };

        self.skip_whitespace();

        // Expect "is either"
        if !self.try_consume_keyword("is") {
            return Err(ParseError::Expected {
                expected: "'is'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        self.skip_whitespace();
        if !self.try_consume_keyword("either") {
            return Err(ParseError::Expected {
                expected: "'either'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }

        self.skip_whitespace();

        // Parse variants - either inline with "or" or block with indentation
        let constructors = if self.peek_char(':') {
            // Block format: "is either:\n    Zero.\n    Successor with pred: Nat."
            self.advance(); // consume ':'
            self.parse_indented_variants(&name, &params)?
        } else {
            // Inline format: "is either Yes or No."
            self.parse_inline_variants(&name, &params)?
        };

        if constructors.is_empty() {
            return Err(ParseError::Missing("constructors".to_string()));
        }

        Ok(Command::Inductive {
            name,
            params,
            sort: Term::Sort(Universe::Type(0)),
            constructors,
        })
    }

    /// Parse inline variants: "Yes or No" or "Yes or No."
    fn parse_inline_variants(
        &mut self,
        inductive_name: &str,
        params: &[(String, Term)],
    ) -> Result<Vec<(String, Term)>, ParseError> {
        let mut constructors = Vec::new();

        loop {
            self.skip_whitespace();

            if self.at_end() || self.peek_char('.') {
                break;
            }

            // Parse a single variant
            let (ctor_name, ctor_type) = self.parse_variant(inductive_name, params)?;
            constructors.push((ctor_name, ctor_type));

            self.skip_whitespace();

            // Check for "or" separator
            if !self.try_consume_keyword("or") {
                break;
            }
        }

        // Consume trailing period if present
        self.skip_whitespace();
        let _ = self.try_consume(".");

        Ok(constructors)
    }

    /// Parse indented variants (after "is either:")
    fn parse_indented_variants(
        &mut self,
        inductive_name: &str,
        params: &[(String, Term)],
    ) -> Result<Vec<(String, Term)>, ParseError> {
        let mut constructors = Vec::new();

        loop {
            self.skip_whitespace_and_newlines();

            if self.at_end() {
                break;
            }

            // Check for end of block (non-indented line or empty)
            if !self.peek_char(' ') && !self.peek_char('\t') && !self.peek_char('a') && !self.peek_char('A') {
                // Check if this is a variant starting with capital letter
                if let Some(c) = self.peek() {
                    if !c.is_uppercase() && c != 'a' && c != 'A' {
                        break;
                    }
                }
            }

            // Skip leading whitespace for indentation
            self.skip_whitespace();

            // Check for "a/an" prefix or capital letter
            if self.at_end() {
                break;
            }

            // Parse variant
            let (ctor_name, ctor_type) = self.parse_variant(inductive_name, params)?;
            constructors.push((ctor_name, ctor_type));

            // Consume trailing period
            self.skip_whitespace();
            let _ = self.try_consume(".");
        }

        Ok(constructors)
    }

    /// Parse a single variant: "Zero" or "a Successor with pred: Nat"
    fn parse_variant(
        &mut self,
        inductive_name: &str,
        params: &[(String, Term)],
    ) -> Result<(String, Term), ParseError> {
        self.skip_whitespace();

        // Skip optional "a" or "an" prefix (for readability)
        let _ = self.try_consume_keyword("an") || self.try_consume_keyword("a");
        self.skip_whitespace();

        // Parse constructor name
        let ctor_name = self.parse_ident()?;

        self.skip_whitespace();

        // Build the result type (possibly with parameters applied)
        let result_type = self.build_applied_type(inductive_name, params);

        // Check for "with" clause (fields)
        if self.try_consume_keyword("with") {
            // Parse fields: "field: Type and field2: Type2"
            let fields = self.parse_field_list()?;

            // Build constructor type: field1_type -> field2_type -> ... -> ResultType
            let mut ctor_type = result_type;
            for (_field_name, field_type) in fields.into_iter().rev() {
                ctor_type = Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(field_type),
                    body_type: Box::new(ctor_type),
                };
            }

            Ok((ctor_name, ctor_type))
        } else {
            // Unit constructor (no fields)
            Ok((ctor_name, result_type))
        }
    }

    /// Parse field list: "field: Type" or "field: Type and field2: Type2"
    fn parse_field_list(&mut self) -> Result<Vec<(String, Term)>, ParseError> {
        let mut fields = Vec::new();

        loop {
            self.skip_whitespace();

            // Parse field name
            let field_name = self.parse_ident()?;

            self.skip_whitespace();

            // Expect ":"
            if !self.try_consume(":") {
                return Err(ParseError::Expected {
                    expected: "':'".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }

            self.skip_whitespace();

            // Parse field type
            let field_type = self.parse_type()?;

            fields.push((field_name, field_type));

            self.skip_whitespace();

            // Check for "and" separator
            if !self.try_consume_keyword("and") {
                break;
            }
        }

        Ok(fields)
    }

    /// Build type with parameters applied: List T
    fn build_applied_type(&self, name: &str, params: &[(String, Term)]) -> Term {
        let mut result = Term::Global(name.to_string());
        for (param_name, _) in params {
            result = Term::App(Box::new(result), Box::new(Term::Var(param_name.clone())));
        }
        result
    }

    // ============================================================
    // DEFINITION PARSING
    // ============================================================

    /// Parse: ## To [name] (params) -> RetType: body
    fn parse_literate_definition(&mut self) -> Result<Command, ParseError> {
        self.skip_whitespace();

        // Consume "## To "
        if !self.try_consume("## To ") && !self.try_consume("##To ") {
            return Err(ParseError::Expected {
                expected: "'## To'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }

        self.skip_whitespace();

        // Parse function name
        let mut name = self.parse_ident()?;

        // Handle predicate syntax: "## To be Provable" means define "Provable", not "be"
        // This allows "## To be X (params) -> Prop:" for predicate definitions
        if name == "be" {
            self.skip_whitespace();
            name = self.parse_ident()?;
        }

        self.current_function = Some(name.clone());

        self.skip_whitespace();

        // Parse parameter groups: (x: T) and (y: U) or (x: T) (y: U)
        // Note: For nullary predicates like "## To be Consistent -> Prop:", params will be empty
        let all_params = self.parse_function_params()?;

        self.skip_whitespace();

        // Parse optional return type: -> RetType
        let return_type = if self.try_consume("->") {
            self.skip_whitespace();
            let ret = self.parse_type()?;
            // Store return type for use as motive in Consider blocks
            self.return_type = Some(ret.clone());
            Some(ret)
        } else {
            None
        };

        self.skip_whitespace();

        // Expect ":"
        if !self.try_consume(":") {
            return Err(ParseError::Expected {
                expected: "':'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }

        // Add parameters to bound vars
        for (param_name, _) in &all_params {
            self.bound_vars.insert(param_name.clone());
        }

        // Parse body
        self.skip_whitespace_and_newlines();
        let body = self.parse_body()?;

        // Check for self-reference (implicit fixpoint)
        let needs_fix = self.contains_self_reference(&name, &body);

        // Build the function body with lambdas
        let mut func_body = body;
        for (param_name, param_type) in all_params.iter().rev() {
            func_body = Term::Lambda {
                param: param_name.clone(),
                param_type: Box::new(param_type.clone()),
                body: Box::new(func_body),
            };
        }

        // Wrap in Fix if recursive
        if needs_fix {
            func_body = Term::Fix {
                name: name.clone(),
                body: Box::new(func_body),
            };
        }

        // Build the full type annotation if we have a return type
        let ty = if let Some(ret) = return_type {
            let mut full_type = ret;
            for (_, param_type) in all_params.iter().rev() {
                full_type = Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(param_type.clone()),
                    body_type: Box::new(full_type),
                };
            }
            Some(full_type)
        } else {
            None
        };

        self.current_function = None;

        Ok(Command::Definition {
            name,
            ty,
            body: func_body,
            is_hint: false,
        })
    }

    // ============================================================
    // LET DEFINITION PARSING
    // ============================================================

    /// Parse: Let [name] be [term].
    fn parse_literate_let(&mut self) -> Result<Command, ParseError> {
        self.skip_whitespace();

        // Consume "Let "
        if !self.try_consume_keyword("Let") {
            return Err(ParseError::Expected {
                expected: "'Let'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }

        self.skip_whitespace();

        // Parse the name
        let name = self.parse_ident()?;

        self.skip_whitespace();

        // Expect "be"
        if !self.try_consume_keyword("be") {
            return Err(ParseError::Expected {
                expected: "'be'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }

        self.skip_whitespace();

        // Parse the body term
        let body = self.parse_term()?;

        // Consume trailing period if present
        self.skip_whitespace();
        let _ = self.try_consume(".");

        Ok(Command::Definition {
            name,
            ty: None,
            body,
            is_hint: false,
        })
    }

    /// Parse: ## Theorem: [name]\n    Statement: [proposition].\n    Proof: [tactic].
    ///
    /// The Proof section is optional. When provided, it applies a tactic like `ring.`
    /// to automatically prove the statement.
    fn parse_literate_theorem(&mut self) -> Result<Command, ParseError> {
        self.skip_whitespace();

        // Consume "## Theorem:"
        if !self.try_consume("## Theorem:") {
            return Err(ParseError::Expected {
                expected: "'## Theorem:'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }

        self.skip_whitespace();

        // Parse the theorem name
        let name = self.parse_ident()?;

        self.skip_whitespace_and_newlines();

        // Expect "Statement:"
        if !self.try_consume_keyword("Statement") {
            return Err(ParseError::Expected {
                expected: "'Statement'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        self.skip_whitespace();
        if !self.try_consume(":") {
            return Err(ParseError::Expected {
                expected: "':'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }

        self.skip_whitespace();

        // Parse the statement (proposition)
        let statement = self.parse_term()?;

        // Consume trailing period if present
        self.skip_whitespace();
        let _ = self.try_consume(".");

        self.skip_whitespace_and_newlines();

        // Check for optional "Proof:" section
        let (body, ty) = if self.try_consume_keyword("Proof") {
            self.skip_whitespace();
            if !self.try_consume(":") {
                return Err(ParseError::Expected {
                    expected: "':'".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }
            self.skip_whitespace();

            // Parse proof tactic and apply it to the statement
            let proof = self.parse_proof_tactic(&statement)?;

            // Consume trailing period if present
            self.skip_whitespace();
            let _ = self.try_consume(".");

            (proof, Some(Term::Global("Derivation".to_string())))
        } else {
            // No proof - existing behavior (statement as body, Prop as type)
            (statement, Some(Term::Sort(Universe::Prop)))
        };

        // Check for optional "Attribute: hint." section
        self.skip_whitespace_and_newlines();
        let is_hint = if self.try_consume_keyword("Attribute") {
            self.skip_whitespace();
            if !self.try_consume(":") {
                return Err(ParseError::Expected {
                    expected: "':'".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }
            self.skip_whitespace();

            let hint = self.try_consume_keyword("hint");

            // Consume trailing period
            self.skip_whitespace();
            let _ = self.try_consume(".");

            hint
        } else {
            false
        };

        Ok(Command::Definition {
            name,
            ty,
            body,
            is_hint,
        })
    }

    /// Parse a proof tactic like `ring.` and return the proof term.
    ///
    /// Supported tactics:
    /// - `ring.` - Proves polynomial equalities by normalization
    /// - `refl.` - Proves reflexivity goals
    /// - `lia.` - Proves linear integer arithmetic (inequalities)
    fn parse_proof_tactic(&mut self, statement: &Term) -> Result<Term, ParseError> {
        if self.try_consume_keyword("ring") {
            // Consume optional trailing period
            self.skip_whitespace();
            let _ = self.try_consume(".");

            // Convert statement to Syntax and apply try_ring
            let goal_syntax = self.term_to_syntax(statement, &[]);
            Ok(Term::App(
                Box::new(Term::Global("try_ring".to_string())),
                Box::new(goal_syntax),
            ))
        } else if self.try_consume_keyword("refl") {
            // Refl tactic
            self.skip_whitespace();
            let _ = self.try_consume(".");

            let goal_syntax = self.term_to_syntax(statement, &[]);
            Ok(Term::App(
                Box::new(Term::Global("try_refl".to_string())),
                Box::new(goal_syntax),
            ))
        } else if self.try_consume_keyword("lia") {
            // LIA tactic: linear integer arithmetic
            self.skip_whitespace();
            let _ = self.try_consume(".");

            let goal_syntax = self.term_to_syntax(statement, &[]);
            Ok(Term::App(
                Box::new(Term::Global("try_lia".to_string())),
                Box::new(goal_syntax),
            ))
        } else if self.try_consume_keyword("cc") {
            // CC tactic: congruence closure
            self.skip_whitespace();
            let _ = self.try_consume(".");

            let goal_syntax = self.term_to_syntax(statement, &[]);
            Ok(Term::App(
                Box::new(Term::Global("try_cc".to_string())),
                Box::new(goal_syntax),
            ))
        } else if self.try_consume_keyword("simp") {
            // Simp tactic: simplification and arithmetic
            self.skip_whitespace();
            let _ = self.try_consume(".");

            let goal_syntax = self.term_to_syntax(statement, &[]);
            Ok(Term::App(
                Box::new(Term::Global("try_simp".to_string())),
                Box::new(goal_syntax),
            ))
        } else if self.try_consume_keyword("omega") {
            // Omega tactic: true integer arithmetic (Omega Test)
            self.skip_whitespace();
            let _ = self.try_consume(".");

            let goal_syntax = self.term_to_syntax(statement, &[]);
            Ok(Term::App(
                Box::new(Term::Global("try_omega".to_string())),
                Box::new(goal_syntax),
            ))
        } else if self.try_consume_keyword("auto") {
            // Auto tactic: tries all decision procedures in sequence
            self.skip_whitespace();
            let _ = self.try_consume(".");

            let goal_syntax = self.term_to_syntax(statement, &[]);
            Ok(Term::App(
                Box::new(Term::Global("try_auto".to_string())),
                Box::new(goal_syntax),
            ))
        } else if self.try_consume_keyword("induction") {
            // Induction tactic: structural induction on a variable
            self.skip_whitespace();
            let var_name = self.parse_ident()?;
            self.skip_whitespace();
            let _ = self.try_consume(".");

            // Parse bullet cases
            let cases = self.parse_bullet_cases(statement)?;

            // Build induction derivation
            self.build_induction_derivation(&var_name, statement, cases)
        } else {
            Err(ParseError::Expected {
                expected: "proof tactic (ring, refl, lia, cc, simp, omega, auto, induction)".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            })
        }
    }

    /// Check if the next non-whitespace character is a bullet point
    fn peek_bullet(&mut self) -> bool {
        // Save position
        let saved_pos = self.pos;
        self.skip_whitespace_and_newlines();
        let result = matches!(self.peek(), Some('-') | Some('*') | Some('+'));
        self.pos = saved_pos;
        result
    }

    /// Consume a bullet point character
    fn consume_bullet(&mut self) {
        self.skip_whitespace_and_newlines();
        if matches!(self.peek(), Some('-') | Some('*') | Some('+')) {
            self.advance();
        }
    }

    /// Parse bullet-pointed cases for induction
    fn parse_bullet_cases(&mut self, statement: &Term) -> Result<Vec<Term>, ParseError> {
        let mut cases = Vec::new();

        while self.peek_bullet() {
            self.consume_bullet();
            self.skip_whitespace();

            // Parse tactics for this case
            let case_proof = self.parse_tactic_sequence(statement)?;
            cases.push(case_proof);
        }

        Ok(cases)
    }

    /// Parse a sequence of tactics (simp. auto.) on a single line
    fn parse_tactic_sequence(&mut self, statement: &Term) -> Result<Term, ParseError> {
        let mut tactics = Vec::new();

        loop {
            self.skip_whitespace();

            // Stop at newline followed by bullet, end of input, or Attribute keyword
            if self.peek_bullet() || self.at_end() || self.peek_keyword("Attribute") {
                break;
            }

            // Try to parse a single tactic
            match self.parse_single_tactic(statement) {
                Ok(tactic) => tactics.push(tactic),
                Err(_) => break,
            }
        }

        if tactics.is_empty() {
            return Err(ParseError::Missing("tactic in bullet case".to_string()));
        }

        // Combine tactics: if multiple, wrap with tact_seq (right-associative)
        let mut result = tactics.pop().unwrap();
        while let Some(prev) = tactics.pop() {
            result = Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("tact_seq".to_string())),
                    Box::new(prev),
                )),
                Box::new(result),
            );
        }

        Ok(result)
    }

    /// Parse a single tactic (ring, auto, etc.) without consuming trailing period
    fn parse_single_tactic(&mut self, statement: &Term) -> Result<Term, ParseError> {
        let goal_syntax = self.term_to_syntax(statement, &[]);

        if self.try_consume_keyword("ring") {
            self.skip_whitespace();
            let _ = self.try_consume(".");
            Ok(Term::App(
                Box::new(Term::Global("try_ring".to_string())),
                Box::new(goal_syntax),
            ))
        } else if self.try_consume_keyword("refl") {
            self.skip_whitespace();
            let _ = self.try_consume(".");
            Ok(Term::App(
                Box::new(Term::Global("try_refl".to_string())),
                Box::new(goal_syntax),
            ))
        } else if self.try_consume_keyword("lia") {
            self.skip_whitespace();
            let _ = self.try_consume(".");
            Ok(Term::App(
                Box::new(Term::Global("try_lia".to_string())),
                Box::new(goal_syntax),
            ))
        } else if self.try_consume_keyword("cc") {
            self.skip_whitespace();
            let _ = self.try_consume(".");
            Ok(Term::App(
                Box::new(Term::Global("try_cc".to_string())),
                Box::new(goal_syntax),
            ))
        } else if self.try_consume_keyword("simp") {
            self.skip_whitespace();
            let _ = self.try_consume(".");
            Ok(Term::App(
                Box::new(Term::Global("try_simp".to_string())),
                Box::new(goal_syntax),
            ))
        } else if self.try_consume_keyword("omega") {
            self.skip_whitespace();
            let _ = self.try_consume(".");
            Ok(Term::App(
                Box::new(Term::Global("try_omega".to_string())),
                Box::new(goal_syntax),
            ))
        } else if self.try_consume_keyword("auto") {
            self.skip_whitespace();
            let _ = self.try_consume(".");
            Ok(Term::App(
                Box::new(Term::Global("try_auto".to_string())),
                Box::new(goal_syntax),
            ))
        } else if self.try_consume_keyword("intro") {
            self.skip_whitespace();
            // Optional variable name after intro
            let _var = if !self.peek_char('.') && !self.at_end() {
                self.parse_ident().ok()
            } else {
                None
            };
            self.skip_whitespace();
            let _ = self.try_consume(".");
            Ok(Term::App(
                Box::new(Term::Global("try_intro".to_string())),
                Box::new(goal_syntax),
            ))
        } else {
            Err(ParseError::Expected {
                expected: "tactic".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            })
        }
    }

    /// Build an induction derivation from variable name and case proofs
    fn build_induction_derivation(
        &self,
        _var_name: &str,
        statement: &Term,
        cases: Vec<Term>,
    ) -> Result<Term, ParseError> {
        // Build the inductive type (for now assume Nat if not specified)
        let ind_type = Term::App(
            Box::new(Term::Global("SName".to_string())),
            Box::new(Term::Lit(Literal::Text("Nat".to_string()))),
        );

        // Build the motive from the statement
        let motive = self.term_to_syntax(statement, &[]);

        // Build the cases as DCase chain: DCase c1 (DCase c2 ... DCaseEnd)
        let mut case_chain = Term::Global("DCaseEnd".to_string());
        for case_proof in cases.into_iter().rev() {
            case_chain = Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("DCase".to_string())),
                    Box::new(case_proof),
                )),
                Box::new(case_chain),
            );
        }

        // Build: try_induction ind_type motive cases
        Ok(Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("try_induction".to_string())),
                    Box::new(ind_type),
                )),
                Box::new(motive),
            )),
            Box::new(case_chain),
        ))
    }

    /// Parse function parameters: (x: T) and (y: U) or (x: T) (y: U)
    fn parse_function_params(&mut self) -> Result<Vec<(String, Term)>, ParseError> {
        let mut params = Vec::new();

        loop {
            self.skip_whitespace();

            // Check for opening paren
            if !self.peek_char('(') {
                break;
            }

            // Parse one parameter group
            self.advance(); // consume '('
            self.skip_whitespace();

            let param_name = self.parse_ident()?;
            self.skip_whitespace();

            if !self.try_consume(":") {
                return Err(ParseError::Expected {
                    expected: "':'".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }
            self.skip_whitespace();

            let param_type = self.parse_type()?;
            self.skip_whitespace();

            if !self.try_consume(")") {
                return Err(ParseError::Expected {
                    expected: "')'".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }

            params.push((param_name, param_type));

            self.skip_whitespace();

            // Check for "and" separator
            let _ = self.try_consume_keyword("and");
        }

        Ok(params)
    }

    /// Parse optional type parameters: (T: Type)
    fn parse_param_list(&mut self) -> Result<Vec<(String, Term)>, ParseError> {
        let mut params = Vec::new();

        if !self.peek_char('(') {
            return Ok(params);
        }

        self.advance(); // consume '('
        self.skip_whitespace();

        loop {
            let param_name = self.parse_ident()?;
            self.skip_whitespace();

            if !self.try_consume(":") {
                return Err(ParseError::Expected {
                    expected: "':'".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }
            self.skip_whitespace();

            let param_type = self.parse_type()?;
            params.push((param_name, param_type));

            self.skip_whitespace();

            if self.try_consume(")") {
                break;
            }

            if !self.try_consume(",") && !self.try_consume_keyword("and") {
                return Err(ParseError::Expected {
                    expected: "')' or ','".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }
            self.skip_whitespace();
        }

        Ok(params)
    }

    // ============================================================
    // BODY / TERM PARSING
    // ============================================================

    /// Parse function body (indented block after ":")
    fn parse_body(&mut self) -> Result<Term, ParseError> {
        self.skip_whitespace();

        // Check for Consider (pattern matching)
        if self.peek_keyword("Consider") {
            return self.parse_consider();
        }

        // Check for Yield (direct return)
        if self.peek_keyword("Yield") {
            return self.parse_yield();
        }

        // Check for "given" lambda
        if self.peek_keyword("given") {
            return self.parse_given_lambda();
        }

        // Check for pipe lambda |x: T| -> body
        if self.peek_char('|') {
            return self.parse_pipe_lambda();
        }

        // Otherwise, parse as a term expression
        self.parse_term()
    }

    /// Parse: Consider x: When C1: body1. When C2: body2.
    fn parse_consider(&mut self) -> Result<Term, ParseError> {
        self.consume_keyword("Consider")?;
        self.skip_whitespace();

        // Parse the discriminant
        let discriminant = self.parse_term()?;

        self.skip_whitespace();

        // Expect ":"
        if !self.try_consume(":") {
            return Err(ParseError::Expected {
                expected: "':'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }

        // Parse cases
        let mut cases = Vec::new();
        // Use return type as motive if available, otherwise a sort placeholder
        // The type checker handles constant motives by wrapping them
        let motive = self.return_type.clone().unwrap_or_else(|| Term::Sort(Universe::Type(0)));

        loop {
            self.skip_whitespace_and_newlines();

            if !self.peek_keyword("When") {
                break;
            }

            self.consume_keyword("When")?;
            self.skip_whitespace();

            // Parse constructor name
            let ctor_name = self.parse_ident()?;
            self.skip_whitespace();

            // Parse optional binders (constructor arguments)
            let mut binders = Vec::new();
            while !self.peek_char(':') && !self.at_end() {
                let binder = self.parse_ident()?;
                binders.push(binder);
                self.skip_whitespace();
            }

            // Expect ":"
            if !self.try_consume(":") {
                return Err(ParseError::Expected {
                    expected: "':'".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }

            self.skip_whitespace();

            // Add binders to scope
            for binder in &binders {
                self.bound_vars.insert(binder.clone());
            }

            // Parse case body
            let case_body = self.parse_body()?;

            // Remove binders from scope
            for binder in &binders {
                self.bound_vars.remove(binder);
            }

            // Wrap body in lambdas for each binder (from right to left)
            let mut wrapped_body = case_body;
            for binder in binders.into_iter().rev() {
                wrapped_body = Term::Lambda {
                    param: binder,
                    param_type: Box::new(Term::Global("_".to_string())), // Placeholder
                    body: Box::new(wrapped_body),
                };
            }

            cases.push(wrapped_body);

            // Consume trailing period if present
            self.skip_whitespace();
            let _ = self.try_consume(".");
        }

        if cases.is_empty() {
            return Err(ParseError::Missing("When clauses".to_string()));
        }

        // Try to infer motive from discriminant if it's a simple variable
        if let Term::Var(ref _v) = discriminant {
            // We leave motive as placeholder; kernel will infer
        }

        Ok(Term::Match {
            discriminant: Box::new(discriminant),
            motive: Box::new(motive),
            cases,
        })
    }

    /// Parse: Yield expression
    fn parse_yield(&mut self) -> Result<Term, ParseError> {
        self.consume_keyword("Yield")?;
        self.skip_whitespace();
        self.parse_term()
    }

    /// Parse: given x: Type yields body
    fn parse_given_lambda(&mut self) -> Result<Term, ParseError> {
        self.consume_keyword("given")?;
        self.skip_whitespace();

        let param = self.parse_ident()?;
        self.skip_whitespace();

        if !self.try_consume(":") {
            return Err(ParseError::Expected {
                expected: "':'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        self.skip_whitespace();

        let param_type = self.parse_type()?;
        self.skip_whitespace();

        self.consume_keyword("yields")?;
        self.skip_whitespace();

        self.bound_vars.insert(param.clone());
        let body = self.parse_term()?;
        self.bound_vars.remove(&param);

        Ok(Term::Lambda {
            param,
            param_type: Box::new(param_type),
            body: Box::new(body),
        })
    }

    /// Parse: |x: Type| -> body
    fn parse_pipe_lambda(&mut self) -> Result<Term, ParseError> {
        self.advance(); // consume '|'
        self.skip_whitespace();

        let param = self.parse_ident()?;
        self.skip_whitespace();

        if !self.try_consume(":") {
            return Err(ParseError::Expected {
                expected: "':'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        self.skip_whitespace();

        let param_type = self.parse_type()?;
        self.skip_whitespace();

        if !self.try_consume("|") {
            return Err(ParseError::Expected {
                expected: "'|'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        self.skip_whitespace();

        if !self.try_consume("->") {
            return Err(ParseError::Expected {
                expected: "'->'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        self.skip_whitespace();

        self.bound_vars.insert(param.clone());
        let body = self.parse_term()?;
        self.bound_vars.remove(&param);

        Ok(Term::Lambda {
            param,
            param_type: Box::new(param_type),
            body: Box::new(body),
        })
    }

    /// Parse a term expression (handles infix operators like `equals` and `implies`)
    fn parse_term(&mut self) -> Result<Term, ParseError> {
        let lhs = self.parse_comparison()?;

        self.skip_whitespace();

        // Check for "equals" infix operator: X equals Y → Eq Hole X Y
        if self.peek_keyword("equals") {
            self.consume_keyword("equals")?;
            self.skip_whitespace();
            let rhs = self.parse_comparison()?; // Parse RHS as comparison (not full term to avoid recursion issues)
            return Ok(Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::Global("Eq".to_string())),
                        Box::new(Term::Hole), // Type placeholder (implicit argument)
                    )),
                    Box::new(lhs),
                )),
                Box::new(rhs),
            ));
        }

        // Check for "implies" infix operator at term level too: X implies Y → Pi _ : X, Y
        if self.peek_keyword("implies") {
            self.consume_keyword("implies")?;
            self.skip_whitespace();
            let rhs = self.parse_term()?; // Full term for right-associativity
            return Ok(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(lhs),
                body_type: Box::new(rhs),
            });
        }

        Ok(lhs)
    }

    // ============================================================
    // INFIX OPERATOR PARSING (comparison, additive, multiplicative)
    // ============================================================

    /// Parse comparison operators: x <= y, x < y, x >= y, x > y
    /// Precedence: lower than arithmetic, higher than equals/implies
    /// Non-associative: a < b < c is not valid
    fn parse_comparison(&mut self) -> Result<Term, ParseError> {
        let lhs = self.parse_additive()?;
        self.skip_whitespace();

        // Check for comparison operators (order matters: >= before >, <= before <)
        let op_name = if self.try_consume("<=") || self.try_consume("≤") {
            Some("le")
        } else if self.try_consume(">=") || self.try_consume("≥") {
            Some("ge")
        } else if self.try_consume("<") {
            Some("lt")
        } else if self.try_consume(">") {
            Some("gt")
        } else {
            None
        };

        if let Some(op) = op_name {
            self.skip_whitespace();
            let rhs = self.parse_additive()?; // Non-associative: parse additive, not comparison
            return Ok(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global(op.to_string())),
                    Box::new(lhs),
                )),
                Box::new(rhs),
            ));
        }

        Ok(lhs)
    }

    /// Parse additive operators: x + y, x - y
    /// Left-associative: a + b - c = (a + b) - c
    fn parse_additive(&mut self) -> Result<Term, ParseError> {
        let mut result = self.parse_multiplicative()?;

        loop {
            self.skip_whitespace();

            let op_name = if self.try_consume("+") {
                Some("add")
            } else if self.peek_char('-') && !self.peek_arrow() && !self.peek_negative_number() {
                self.advance(); // consume '-'
                Some("sub")
            } else {
                None
            };

            if let Some(op) = op_name {
                self.skip_whitespace();
                let rhs = self.parse_multiplicative()?;
                result = Term::App(
                    Box::new(Term::App(
                        Box::new(Term::Global(op.to_string())),
                        Box::new(result),
                    )),
                    Box::new(rhs),
                );
            } else {
                break;
            }
        }

        Ok(result)
    }

    /// Parse multiplicative operators: x * y
    /// Left-associative: a * b * c = (a * b) * c
    fn parse_multiplicative(&mut self) -> Result<Term, ParseError> {
        let mut result = self.parse_app()?;

        loop {
            self.skip_whitespace();

            if self.try_consume("*") {
                self.skip_whitespace();
                let rhs = self.parse_app()?;
                result = Term::App(
                    Box::new(Term::App(
                        Box::new(Term::Global("mul".to_string())),
                        Box::new(result),
                    )),
                    Box::new(rhs),
                );
            } else {
                break;
            }
        }

        Ok(result)
    }

    /// Parse application: f x y or f(x, y)
    fn parse_app(&mut self) -> Result<Term, ParseError> {
        let mut func = self.parse_atom()?;

        loop {
            self.skip_whitespace();

            // Check for tuple-style call: f(x, y)
            if self.peek_char('(') {
                self.advance(); // consume '('
                self.skip_whitespace();

                if !self.peek_char(')') {
                    loop {
                        let arg = self.parse_term()?;
                        func = Term::App(Box::new(func), Box::new(arg));
                        self.skip_whitespace();

                        if self.try_consume(",") {
                            self.skip_whitespace();
                        } else {
                            break;
                        }
                    }
                }

                if !self.try_consume(")") {
                    return Err(ParseError::Expected {
                        expected: "')'".to_string(),
                        found: self.peek_word().unwrap_or("EOF".to_string()),
                    });
                }
                continue;
            }

            // Check for curried application (next token is an atom)
            // Stop before infix operators like `equals`, `implies`, `+`, `-`, `*`, `<`, etc.
            if self.at_end()
                || self.peek_char(')')
                || self.peek_char('.')
                || self.peek_char(',')
                || self.peek_char(':')
                || self.peek_char('|')
                || self.peek_keyword("When")
                || self.peek_keyword("Yield")
                || self.peek_keyword("and")
                || self.peek_keyword("or")
                || self.peek_keyword("equals")
                || self.peek_keyword("implies")
                // Stop at infix arithmetic and comparison operators
                || self.peek_char('+')
                || self.peek_char('*')
                || self.peek_comparison_operator()
                || (self.peek_char('-') && !self.peek_arrow() && !self.peek_negative_number())
            {
                break;
            }

            // Try to parse next atom for curried application
            if let Ok(arg) = self.parse_atom() {
                func = Term::App(Box::new(func), Box::new(arg));
            } else {
                break;
            }
        }

        Ok(func)
    }

    /// Parse an atom: identifier, literal, or parenthesized term
    fn parse_atom(&mut self) -> Result<Term, ParseError> {
        self.skip_whitespace();

        // Check for number literal
        if let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                return self.parse_number();
            }
            if c == '-' {
                // Check for negative number
                let saved_pos = self.pos;
                self.advance();
                if let Some(next) = self.peek() {
                    if next.is_ascii_digit() {
                        self.pos = saved_pos;
                        return self.parse_number();
                    }
                }
                self.pos = saved_pos;
            }
        }

        // Check for string literal
        if self.peek_char('"') {
            return self.parse_string();
        }

        // Check for parenthesized term
        if self.peek_char('(') {
            self.advance();
            let term = self.parse_term()?;
            self.skip_whitespace();
            if !self.try_consume(")") {
                return Err(ParseError::Expected {
                    expected: "')'".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }
            return Ok(term);
        }

        // Check for "the" prefix (allows "the Successor of ...", "the diagonalization of ...", "the Name X")
        if self.try_consume_keyword("the") {
            self.skip_whitespace();
            // Check for "diagonalization of" special syntax
            if self.try_consume_keyword("diagonalization") {
                self.skip_whitespace();
                if !self.try_consume_keyword("of") {
                    return Err(ParseError::Expected {
                        expected: "'of'".to_string(),
                        found: self.peek_word().unwrap_or("EOF".to_string()),
                    });
                }
                self.skip_whitespace();
                let arg = self.parse_atom()?;
                return Ok(Term::App(
                    Box::new(Term::Global("syn_diag".to_string())),
                    Box::new(arg),
                ));
            }
            // Check for "the Name X" syntax → SName X
            if self.peek_keyword("Name") {
                self.consume_keyword("Name")?;
                self.skip_whitespace();
                let arg = self.parse_atom()?;
                return Ok(Term::App(
                    Box::new(Term::Global("SName".to_string())),
                    Box::new(arg),
                ));
            }
            // Otherwise continue to parse the identifier after "the"
        }

        // Check for "Name" special syntax: Name "X" → SName "X"
        if self.peek_keyword("Name") {
            self.consume_keyword("Name")?;
            self.skip_whitespace();
            let arg = self.parse_atom()?;
            return Ok(Term::App(
                Box::new(Term::Global("SName".to_string())),
                Box::new(arg),
            ));
        }

        // Check for "Variable" special syntax: Variable 0 → SVar 0
        if self.peek_keyword("Variable") {
            self.consume_keyword("Variable")?;
            self.skip_whitespace();
            let arg = self.parse_atom()?;
            return Ok(Term::App(
                Box::new(Term::Global("SVar".to_string())),
                Box::new(arg),
            ));
        }

        // Check for "Apply" special syntax: Apply(f, x) → SApp f x
        if self.peek_keyword("Apply") {
            self.consume_keyword("Apply")?;
            self.skip_whitespace();
            if !self.try_consume("(") {
                return Err(ParseError::Expected {
                    expected: "'('".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }
            self.skip_whitespace();
            let func_arg = self.parse_term()?;
            self.skip_whitespace();
            if !self.try_consume(",") {
                return Err(ParseError::Expected {
                    expected: "','".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }
            self.skip_whitespace();
            let arg_arg = self.parse_term()?;
            self.skip_whitespace();
            if !self.try_consume(")") {
                return Err(ParseError::Expected {
                    expected: "')'".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }
            return Ok(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("SApp".to_string())),
                    Box::new(func_arg),
                )),
                Box::new(arg_arg),
            ));
        }

        // Check for "there exists" special syntax
        if self.peek_keyword("there") {
            return self.parse_existential();
        }

        // Parse identifier
        let ident = self.parse_ident()?;

        // Check for "of" suffix (e.g., "Successor of x")
        self.skip_whitespace();
        if self.try_consume_keyword("of") {
            self.skip_whitespace();
            let arg = self.parse_atom()?;
            let func = if self.bound_vars.contains(&ident) {
                Term::Var(ident)
            } else {
                Term::Global(ident)
            };
            return Ok(Term::App(Box::new(func), Box::new(arg)));
        }

        // Return as Var if bound, Global otherwise
        if self.bound_vars.contains(&ident) || self.current_function.as_ref() == Some(&ident) {
            Ok(Term::Var(ident))
        } else {
            // Check for special sorts
            match ident.as_str() {
                "Prop" => Ok(Term::Sort(Universe::Prop)),
                "Type" => Ok(Term::Sort(Universe::Type(0))),
                _ => Ok(Term::Global(ident)),
            }
        }
    }

    /// Parse: there exists a [var]: [Type] such that [body]
    /// → Ex Type (fun var => body)
    fn parse_existential(&mut self) -> Result<Term, ParseError> {
        self.consume_keyword("there")?;
        self.skip_whitespace();
        self.consume_keyword("exists")?;
        self.skip_whitespace();

        // Optional "a" or "an"
        let _ = self.try_consume_keyword("an") || self.try_consume_keyword("a");
        self.skip_whitespace();

        // Parse variable name
        let var_name = self.parse_ident()?;
        self.skip_whitespace();

        // Expect ":"
        if !self.try_consume(":") {
            return Err(ParseError::Expected {
                expected: "':'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        self.skip_whitespace();

        // Parse the type
        let var_type = self.parse_type()?;
        self.skip_whitespace();

        // Expect "such that"
        if !self.try_consume_keyword("such") {
            return Err(ParseError::Expected {
                expected: "'such'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        self.skip_whitespace();
        if !self.try_consume_keyword("that") {
            return Err(ParseError::Expected {
                expected: "'that'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        self.skip_whitespace();

        // Add variable to bound vars and parse body
        self.bound_vars.insert(var_name.clone());
        let body = self.parse_term()?;
        self.bound_vars.remove(&var_name);

        // Build: Ex Type (fun var => body)
        Ok(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("Ex".to_string())),
                Box::new(var_type),
            )),
            Box::new(Term::Lambda {
                param: var_name,
                param_type: Box::new(Term::Global("_".to_string())), // Type inferred
                body: Box::new(body),
            }),
        ))
    }

    /// Parse a type expression (same as term for now)
    fn parse_type(&mut self) -> Result<Term, ParseError> {
        self.skip_whitespace();

        // Handle "List of T" style
        let base = self.parse_ident()?;

        self.skip_whitespace();

        // Check for "of" (polymorphic application)
        if self.try_consume_keyword("of") {
            self.skip_whitespace();
            let arg = self.parse_type()?;
            return Ok(Term::App(
                Box::new(Term::Global(base)),
                Box::new(arg),
            ));
        }

        // Check for special sorts
        match base.as_str() {
            "Prop" => Ok(Term::Sort(Universe::Prop)),
            "Type" => Ok(Term::Sort(Universe::Type(0))),
            _ => Ok(Term::Global(base)),
        }
    }

    /// Parse a number literal
    fn parse_number(&mut self) -> Result<Term, ParseError> {
        let mut num_str = String::new();

        // Handle negative sign
        if self.peek_char('-') {
            num_str.push('-');
            self.advance();
        }

        // Collect digits
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                num_str.push(c);
                self.advance();
            } else {
                break;
            }
        }

        let value: i64 = num_str
            .parse()
            .map_err(|_| ParseError::InvalidNumber(num_str))?;

        Ok(Term::Lit(Literal::Int(value)))
    }

    /// Parse a string literal
    fn parse_string(&mut self) -> Result<Term, ParseError> {
        self.advance(); // consume opening '"'

        let mut content = String::new();
        loop {
            match self.peek() {
                Some('"') => {
                    self.advance();
                    break;
                }
                Some('\\') => {
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

    // ============================================================
    // TERM TO SYNTAX REIFICATION
    // ============================================================

    /// Convert a high-level Term to deeply-embedded Syntax representation.
    /// This is used by proof tactics like `ring.` that operate on Syntax.
    fn term_to_syntax(&self, term: &Term, bound_vars: &[String]) -> Term {
        match term {
            Term::Var(name) => {
                // Convert to SVar with de Bruijn index if bound, else SName
                if let Some(idx) = bound_vars.iter().rev().position(|n| n == name) {
                    Term::App(
                        Box::new(Term::Global("SVar".to_string())),
                        Box::new(Term::Lit(Literal::Int(idx as i64))),
                    )
                } else {
                    // Free variable - treat as SName
                    Term::App(
                        Box::new(Term::Global("SName".to_string())),
                        Box::new(Term::Lit(Literal::Text(name.clone()))),
                    )
                }
            }
            Term::Global(name) => {
                // Global names become SName
                Term::App(
                    Box::new(Term::Global("SName".to_string())),
                    Box::new(Term::Lit(Literal::Text(name.clone()))),
                )
            }
            Term::Lit(Literal::Int(n)) => {
                // Integer literals become SLit
                Term::App(
                    Box::new(Term::Global("SLit".to_string())),
                    Box::new(Term::Lit(Literal::Int(*n))),
                )
            }
            Term::Lit(Literal::Float(_f)) => {
                // Float literals - treat as error for ring (ring doesn't handle floats)
                Term::App(
                    Box::new(Term::Global("SName".to_string())),
                    Box::new(Term::Lit(Literal::Text("Error_Float".to_string()))),
                )
            }
            Term::Lit(Literal::Text(s)) => {
                // Text literals become SLit (wrapped in SName for now)
                Term::App(
                    Box::new(Term::Global("SName".to_string())),
                    Box::new(Term::Lit(Literal::Text(s.clone()))),
                )
            }
            Term::App(f, x) => {
                // Applications become SApp
                let f_syn = self.term_to_syntax(f, bound_vars);
                let x_syn = self.term_to_syntax(x, bound_vars);
                Term::App(
                    Box::new(Term::App(
                        Box::new(Term::Global("SApp".to_string())),
                        Box::new(f_syn),
                    )),
                    Box::new(x_syn),
                )
            }
            Term::Lambda { param, param_type, body } => {
                // Lambdas become SLam (if we have that constructor)
                let ty_syn = self.term_to_syntax(param_type, bound_vars);
                let mut new_bound = bound_vars.to_vec();
                new_bound.push(param.clone());
                let body_syn = self.term_to_syntax(body, &new_bound);
                Term::App(
                    Box::new(Term::App(
                        Box::new(Term::Global("SLam".to_string())),
                        Box::new(ty_syn),
                    )),
                    Box::new(body_syn),
                )
            }
            Term::Pi { param, param_type, body_type } => {
                // Pi types become SPi
                let ty_syn = self.term_to_syntax(param_type, bound_vars);
                let mut new_bound = bound_vars.to_vec();
                new_bound.push(param.clone());
                let body_syn = self.term_to_syntax(body_type, &new_bound);
                Term::App(
                    Box::new(Term::App(
                        Box::new(Term::Global("SPi".to_string())),
                        Box::new(ty_syn),
                    )),
                    Box::new(body_syn),
                )
            }
            Term::Sort(Universe::Prop) => {
                // Prop sort becomes SSort UProp
                Term::App(
                    Box::new(Term::Global("SSort".to_string())),
                    Box::new(Term::Global("UProp".to_string())),
                )
            }
            Term::Sort(Universe::Type(n)) => {
                // Type sort becomes SSort (UType n)
                Term::App(
                    Box::new(Term::Global("SSort".to_string())),
                    Box::new(Term::App(
                        Box::new(Term::Global("UType".to_string())),
                        Box::new(Term::Lit(Literal::Int(*n as i64))),
                    )),
                )
            }
            Term::Hole => {
                // Holes default to Int type for ring proofs
                Term::App(
                    Box::new(Term::Global("SName".to_string())),
                    Box::new(Term::Lit(Literal::Text("Int".to_string()))),
                )
            }
            Term::Match { .. } | Term::Fix { .. } => {
                // Match and Fix are complex - return error marker
                Term::App(
                    Box::new(Term::Global("SName".to_string())),
                    Box::new(Term::Lit(Literal::Text("Error".to_string()))),
                )
            }
        }
    }

    // ============================================================
    // SELF-REFERENCE DETECTION
    // ============================================================

    /// Check if the function name appears in the term (for implicit fixpoint)
    fn contains_self_reference(&self, name: &str, term: &Term) -> bool {
        match term {
            Term::Var(v) => v == name,
            Term::Global(_) => false,
            Term::Sort(_) => false,
            Term::Lit(_) => false,
            Term::Pi { param_type, body_type, .. } => {
                self.contains_self_reference(name, param_type)
                    || self.contains_self_reference(name, body_type)
            }
            Term::Lambda { param_type, body, .. } => {
                self.contains_self_reference(name, param_type)
                    || self.contains_self_reference(name, body)
            }
            Term::App(f, a) => {
                self.contains_self_reference(name, f) || self.contains_self_reference(name, a)
            }
            Term::Match { discriminant, motive, cases } => {
                self.contains_self_reference(name, discriminant)
                    || self.contains_self_reference(name, motive)
                    || cases.iter().any(|c| self.contains_self_reference(name, c))
            }
            Term::Fix { body, .. } => self.contains_self_reference(name, body),
            Term::Hole => false, // Holes never contain self-references
        }
    }

    // ============================================================
    // LOW-LEVEL UTILITIES
    // ============================================================

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn skip_whitespace_and_newlines(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn peek_char(&self, c: char) -> bool {
        self.peek() == Some(c)
    }

    fn peek_keyword(&self, keyword: &str) -> bool {
        if !self.input[self.pos..].starts_with(keyword) {
            return false;
        }
        let after = self.pos + keyword.len();
        if after >= self.input.len() {
            return true;
        }
        let next_char = self.input[after..].chars().next();
        !next_char.map(|c| c.is_alphanumeric() || c == '_').unwrap_or(false)
    }

    fn peek_word(&self) -> Option<String> {
        let start = self.pos;
        let mut end = start;
        for c in self.input[start..].chars() {
            if c.is_alphanumeric() || c == '_' {
                end += c.len_utf8();
            } else {
                break;
            }
        }
        if end > start {
            Some(self.input[start..end].to_string())
        } else {
            self.peek().map(|c| c.to_string())
        }
    }

    fn advance(&mut self) {
        if let Some(c) = self.peek() {
            self.pos += c.len_utf8();
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn try_consume(&mut self, s: &str) -> bool {
        if self.input[self.pos..].starts_with(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }

    fn try_consume_keyword(&mut self, keyword: &str) -> bool {
        if self.peek_keyword(keyword) {
            self.pos += keyword.len();
            true
        } else {
            false
        }
    }

    fn consume_keyword(&mut self, keyword: &str) -> Result<(), ParseError> {
        if self.try_consume_keyword(keyword) {
            Ok(())
        } else {
            Err(ParseError::Expected {
                expected: format!("'{}'", keyword),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            })
        }
    }

    fn parse_ident(&mut self) -> Result<String, ParseError> {
        self.skip_whitespace();
        let start = self.pos;

        // First char must be alphabetic or underscore
        if let Some(c) = self.peek() {
            if !c.is_alphabetic() && c != '_' {
                return Err(ParseError::Expected {
                    expected: "identifier".to_string(),
                    found: c.to_string(),
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

    // ============================================================
    // INFIX OPERATOR HELPERS
    // ============================================================

    /// Check if current position starts a negative number (- followed by digit)
    fn peek_negative_number(&self) -> bool {
        if !self.peek_char('-') {
            return false;
        }
        self.input.get(self.pos + 1..)
            .and_then(|s| s.chars().next())
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
    }

    /// Check if current position starts an arrow (->)
    fn peek_arrow(&self) -> bool {
        self.input[self.pos..].starts_with("->")
    }

    /// Check if current position has a comparison operator
    fn peek_comparison_operator(&self) -> bool {
        let rest = &self.input[self.pos..];
        rest.starts_with("<=") || rest.starts_with(">=")
            || rest.starts_with("≤") || rest.starts_with("≥")
            || rest.starts_with('<') || rest.starts_with('>')
    }
}

// ============================================================
// UNIT TESTS
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_inductive() {
        let cmd = parse_inductive("A Bool is either Yes or No").unwrap();
        if let Command::Inductive { name, constructors, .. } = cmd {
            assert_eq!(name, "Bool");
            assert_eq!(constructors.len(), 2);
            assert_eq!(constructors[0].0, "Yes");
            assert_eq!(constructors[1].0, "No");
        } else {
            panic!("Expected Inductive command");
        }
    }

    #[test]
    fn test_parse_inductive_with_article() {
        let cmd = parse_inductive("A Decision is either a Yes or a No").unwrap();
        if let Command::Inductive { name, constructors, .. } = cmd {
            assert_eq!(name, "Decision");
            assert_eq!(constructors.len(), 2);
        } else {
            panic!("Expected Inductive command");
        }
    }

    #[test]
    fn test_parse_recursive_inductive() {
        let cmd = parse_inductive("A Nat is either Zero or a Succ with pred: Nat").unwrap();
        if let Command::Inductive { name, constructors, .. } = cmd {
            assert_eq!(name, "Nat");
            assert_eq!(constructors.len(), 2);
            assert_eq!(constructors[0].0, "Zero");
            assert_eq!(constructors[1].0, "Succ");
            // Succ should have type Nat -> Nat
            if let Term::Pi { .. } = &constructors[1].1 {
                // Good - it's a function type
            } else {
                panic!("Expected Succ to have Pi type");
            }
        } else {
            panic!("Expected Inductive command");
        }
    }

    #[test]
    fn test_parse_simple_definition() {
        let cmd = parse_definition("## To id (x: Nat) -> Nat: Yield x").unwrap();
        if let Command::Definition { name, body, .. } = cmd {
            assert_eq!(name, "id");
            // Body should be a lambda
            if let Term::Lambda { param, .. } = body {
                assert_eq!(param, "x");
            } else {
                panic!("Expected Lambda body");
            }
        } else {
            panic!("Expected Definition command");
        }
    }

    #[test]
    fn test_implicit_fixpoint_detection() {
        // This definition is recursive (add appears in body)
        let cmd = parse_definition(
            "## To add (n: Nat) and (m: Nat) -> Nat: Consider n: When Zero: Yield m. When Succ k: Yield Succ (add k m)."
        ).unwrap();

        if let Command::Definition { name, body, .. } = cmd {
            assert_eq!(name, "add");
            // Body should be wrapped in Fix
            if let Term::Fix { name: fix_name, .. } = body {
                assert_eq!(fix_name, "add");
            } else {
                panic!("Expected Fix wrapper for recursive function");
            }
        } else {
            panic!("Expected Definition command");
        }
    }

    #[test]
    fn test_parse_given_lambda() {
        let mut parser = LiterateParser::new("given x: Nat yields Succ x");
        let term = parser.parse_given_lambda().unwrap();

        if let Term::Lambda { param, .. } = term {
            assert_eq!(param, "x");
        } else {
            panic!("Expected Lambda");
        }
    }

    #[test]
    fn test_parse_pipe_lambda() {
        let mut parser = LiterateParser::new("|x: Nat| -> Succ x");
        let term = parser.parse_pipe_lambda().unwrap();

        if let Term::Lambda { param, .. } = term {
            assert_eq!(param, "x");
        } else {
            panic!("Expected Lambda");
        }
    }

    // ============================================================
    // GÖDEL SYNTAX TESTS (Phase 2)
    // ============================================================

    #[test]
    fn test_parse_let_definition() {
        let cmd = parse_let_definition("Let T be Zero").unwrap();
        if let Command::Definition { name, ty, body } = cmd {
            assert_eq!(name, "T");
            assert!(ty.is_none());
            if let Term::Global(g) = body {
                assert_eq!(g, "Zero");
            } else {
                panic!("Expected Global term");
            }
        } else {
            panic!("Expected Definition command");
        }
    }

    #[test]
    fn test_parse_name_syntax() {
        let mut parser = LiterateParser::new("Name \"Not\"");
        let term = parser.parse_term().unwrap();

        // Should be SName "Not"
        if let Term::App(f, arg) = term {
            if let Term::Global(g) = *f {
                assert_eq!(g, "SName");
            } else {
                panic!("Expected Global SName");
            }
            if let Term::Lit(Literal::Text(s)) = *arg {
                assert_eq!(s, "Not");
            } else {
                panic!("Expected Text literal");
            }
        } else {
            panic!("Expected App");
        }
    }

    #[test]
    fn test_parse_variable_syntax() {
        let mut parser = LiterateParser::new("Variable 0");
        let term = parser.parse_term().unwrap();

        // Should be SVar 0
        if let Term::App(f, arg) = term {
            if let Term::Global(g) = *f {
                assert_eq!(g, "SVar");
            } else {
                panic!("Expected Global SVar");
            }
            if let Term::Lit(Literal::Int(n)) = *arg {
                assert_eq!(n, 0);
            } else {
                panic!("Expected Int literal");
            }
        } else {
            panic!("Expected App");
        }
    }

    #[test]
    fn test_parse_apply_syntax() {
        let mut parser = LiterateParser::new("Apply(Name \"Not\", Variable 0)");
        let term = parser.parse_term().unwrap();

        // Should be (SApp (SName "Not") (SVar 0))
        if let Term::App(outer_f, _outer_arg) = term {
            if let Term::App(inner_f, _inner_arg) = *outer_f {
                if let Term::Global(g) = *inner_f {
                    assert_eq!(g, "SApp");
                } else {
                    panic!("Expected Global SApp");
                }
            } else {
                panic!("Expected inner App");
            }
        } else {
            panic!("Expected outer App");
        }
    }

    #[test]
    fn test_parse_diagonalization() {
        let mut parser = LiterateParser::new("the diagonalization of T");
        let term = parser.parse_term().unwrap();

        // Should be syn_diag T
        if let Term::App(f, arg) = term {
            if let Term::Global(g) = *f {
                assert_eq!(g, "syn_diag");
            } else {
                panic!("Expected Global syn_diag");
            }
            if let Term::Global(a) = *arg {
                assert_eq!(a, "T");
            } else {
                panic!("Expected Global T");
            }
        } else {
            panic!("Expected App");
        }
    }

    #[test]
    fn test_parse_implies() {
        let mut parser = LiterateParser::new("A implies B");
        let term = parser.parse_term().unwrap();

        // Should be forall _ : A, B (non-dependent Pi)
        if let Term::Pi { param, param_type, body_type } = term {
            assert_eq!(param, "_");
            if let Term::Global(a) = *param_type {
                assert_eq!(a, "A");
            } else {
                panic!("Expected Global A");
            }
            if let Term::Global(b) = *body_type {
                assert_eq!(b, "B");
            } else {
                panic!("Expected Global B");
            }
        } else {
            panic!("Expected Pi");
        }
    }

    #[test]
    fn test_parse_existential() {
        let mut parser = LiterateParser::new("there exists a d: Derivation such that P");
        let term = parser.parse_term().unwrap();

        // Should be Ex Derivation (fun d => P)
        if let Term::App(outer_f, lambda) = term {
            if let Term::App(ex, typ) = *outer_f {
                if let Term::Global(g) = *ex {
                    assert_eq!(g, "Ex");
                } else {
                    panic!("Expected Global Ex");
                }
                if let Term::Global(t) = *typ {
                    assert_eq!(t, "Derivation");
                } else {
                    panic!("Expected Global Derivation");
                }
            } else {
                panic!("Expected inner App for Ex");
            }
            if let Term::Lambda { param, body, .. } = *lambda {
                assert_eq!(param, "d");
                if let Term::Global(p) = *body {
                    assert_eq!(p, "P");
                } else {
                    panic!("Expected Global P in lambda body");
                }
            } else {
                panic!("Expected Lambda");
            }
        } else {
            panic!("Expected App");
        }
    }

    #[test]
    fn test_parse_complex_let_with_apply() {
        let cmd = parse_let_definition("Let T be Apply(Name \"Not\", Apply(Name \"Provable\", Variable 0)).").unwrap();
        if let Command::Definition { name, ty, body } = cmd {
            assert_eq!(name, "T");
            assert!(ty.is_none());
            // Body should be nested applications
            if let Term::App(_, _) = body {
                // Good - it's an application
            } else {
                panic!("Expected App body");
            }
        } else {
            panic!("Expected Definition command");
        }
    }

    // ============================================================
    // PREDICATE SYNTAX TESTS (Phase 2b)
    // ============================================================

    #[test]
    fn test_parse_predicate_definition() {
        // ## To be Provable (s: Syntax) -> Prop: Yield s
        let cmd = parse_definition("## To be Provable (s: Syntax) -> Prop: Yield s").unwrap();
        if let Command::Definition { name, .. } = cmd {
            assert_eq!(name, "Provable"); // NOT "be"
        } else {
            panic!("Expected Definition command");
        }
    }

    #[test]
    fn test_parse_nullary_predicate() {
        // ## To be Consistent -> Prop: Yield True
        let cmd = parse_definition("## To be Consistent -> Prop: Yield True").unwrap();
        if let Command::Definition { name, body, .. } = cmd {
            assert_eq!(name, "Consistent");
            // Body should be True (no lambda wrapper since no params)
            if let Term::Global(g) = body {
                assert_eq!(g, "True");
            } else {
                panic!("Expected Global True");
            }
        } else {
            panic!("Expected Definition command");
        }
    }

    #[test]
    fn test_parse_the_name_syntax() {
        // the Name "Not" → SName "Not"
        let mut parser = LiterateParser::new("the Name \"Not\"");
        let term = parser.parse_term().unwrap();

        if let Term::App(f, arg) = term {
            if let Term::Global(g) = *f {
                assert_eq!(g, "SName");
            } else {
                panic!("Expected Global SName");
            }
            if let Term::Lit(Literal::Text(s)) = *arg {
                assert_eq!(s, "Not");
            } else {
                panic!("Expected Text literal");
            }
        } else {
            panic!("Expected App");
        }
    }

    #[test]
    fn test_parse_theorem() {
        // ## Theorem: MyTheorem\n    Statement: True implies True.
        let cmd = parse_theorem("## Theorem: MyTheorem\n    Statement: A implies B.").unwrap();
        if let Command::Definition { name, ty, body } = cmd {
            assert_eq!(name, "MyTheorem");
            // Type should be Prop
            assert!(ty.is_some());
            if let Some(Term::Sort(Universe::Prop)) = ty {
                // Good
            } else {
                panic!("Expected Prop type");
            }
            // Body should be Pi (A implies B)
            if let Term::Pi { .. } = body {
                // Good
            } else {
                panic!("Expected Pi body (implication)");
            }
        } else {
            panic!("Expected Definition command");
        }
    }

    #[test]
    fn test_parse_theorem_with_complex_statement() {
        // ## Theorem: Godel\n    Statement: Consistent implies Not(Provable(G)).
        let cmd = parse_theorem("## Theorem: Godel\n    Statement: Consistent implies Not(Provable(G)).").unwrap();
        if let Command::Definition { name, ty, .. } = cmd {
            assert_eq!(name, "Godel");
            assert!(ty.is_some());
            if let Some(Term::Sort(Universe::Prop)) = ty {
                // Good
            } else {
                panic!("Expected Prop type");
            }
        } else {
            panic!("Expected Definition command");
        }
    }

    #[test]
    fn test_parse_equals_infix() {
        // X equals Y → Eq Hole X Y
        let mut parser = LiterateParser::new("A equals B");
        let term = parser.parse_term().unwrap();

        // Should be (Eq Hole A B) = App(App(App(Eq, Hole), A), B)
        if let Term::App(outer, rhs) = term {
            if let Term::Global(b) = *rhs {
                assert_eq!(b, "B");
            } else {
                panic!("Expected Global B");
            }
            if let Term::App(mid, lhs) = *outer {
                if let Term::Global(a) = *lhs {
                    assert_eq!(a, "A");
                } else {
                    panic!("Expected Global A");
                }
                if let Term::App(inner, placeholder) = *mid {
                    if let Term::Global(eq) = *inner {
                        assert_eq!(eq, "Eq");
                    } else {
                        panic!("Expected Global Eq");
                    }
                    if !matches!(*placeholder, Term::Hole) {
                        panic!("Expected Hole placeholder");
                    }
                } else {
                    panic!("Expected inner App");
                }
            } else {
                panic!("Expected mid App");
            }
        } else {
            panic!("Expected outer App");
        }
    }

    #[test]
    fn test_parse_equals_with_application() {
        // f(x) equals y → Eq _ (f x) y
        let mut parser = LiterateParser::new("f(x) equals y");
        let term = parser.parse_term().unwrap();

        // Should be App(App(App(Eq, _), App(f, x)), y)
        if let Term::App(outer, rhs) = term {
            if let Term::Global(y) = *rhs {
                assert_eq!(y, "y");
            } else {
                panic!("Expected Global y");
            }
            if let Term::App(mid, lhs) = *outer {
                // lhs should be App(f, x)
                if let Term::App(f_box, x_box) = *lhs {
                    if let Term::Global(f) = *f_box {
                        assert_eq!(f, "f");
                    }
                    if let Term::Global(x) = *x_box {
                        assert_eq!(x, "x");
                    }
                } else {
                    panic!("Expected lhs to be App(f, x)");
                }
            } else {
                panic!("Expected mid App");
            }
        } else {
            panic!("Expected outer App");
        }
    }

    // ============================================================
    // INFIX OPERATOR TESTS
    // ============================================================

    #[test]
    fn test_parse_infix_le() {
        let mut parser = LiterateParser::new("x <= y");
        let term = parser.parse_term().unwrap();
        // Should produce: App(App(Global("le"), Global("x")), Global("y"))
        if let Term::App(outer, rhs) = term {
            if let Term::App(inner, lhs) = *outer {
                if let Term::Global(op) = *inner {
                    assert_eq!(op, "le");
                } else {
                    panic!("Expected Global le");
                }
                if let Term::Global(l) = *lhs {
                    assert_eq!(l, "x");
                } else {
                    panic!("Expected Global x");
                }
            } else {
                panic!("Expected inner App");
            }
            if let Term::Global(r) = *rhs {
                assert_eq!(r, "y");
            } else {
                panic!("Expected Global y");
            }
        } else {
            panic!("Expected outer App");
        }
    }

    #[test]
    fn test_parse_infix_lt() {
        let mut parser = LiterateParser::new("a < b");
        let term = parser.parse_term().unwrap();
        if let Term::App(outer, _) = term {
            if let Term::App(inner, _) = *outer {
                if let Term::Global(op) = *inner {
                    assert_eq!(op, "lt");
                } else {
                    panic!("Expected Global lt");
                }
            } else {
                panic!("Expected inner App");
            }
        } else {
            panic!("Expected outer App");
        }
    }

    #[test]
    fn test_parse_infix_ge() {
        let mut parser = LiterateParser::new("x >= y");
        let term = parser.parse_term().unwrap();
        if let Term::App(outer, _) = term {
            if let Term::App(inner, _) = *outer {
                if let Term::Global(op) = *inner {
                    assert_eq!(op, "ge");
                } else {
                    panic!("Expected Global ge");
                }
            } else {
                panic!("Expected inner App");
            }
        } else {
            panic!("Expected outer App");
        }
    }

    #[test]
    fn test_parse_infix_gt() {
        let mut parser = LiterateParser::new("x > y");
        let term = parser.parse_term().unwrap();
        if let Term::App(outer, _) = term {
            if let Term::App(inner, _) = *outer {
                if let Term::Global(op) = *inner {
                    assert_eq!(op, "gt");
                } else {
                    panic!("Expected Global gt");
                }
            } else {
                panic!("Expected inner App");
            }
        } else {
            panic!("Expected outer App");
        }
    }

    #[test]
    fn test_parse_infix_add() {
        let mut parser = LiterateParser::new("x + y");
        let term = parser.parse_term().unwrap();
        if let Term::App(outer, _) = term {
            if let Term::App(inner, _) = *outer {
                if let Term::Global(op) = *inner {
                    assert_eq!(op, "add");
                } else {
                    panic!("Expected Global add");
                }
            } else {
                panic!("Expected inner App");
            }
        } else {
            panic!("Expected outer App");
        }
    }

    #[test]
    fn test_parse_infix_sub() {
        let mut parser = LiterateParser::new("x - y");
        let term = parser.parse_term().unwrap();
        if let Term::App(outer, _) = term {
            if let Term::App(inner, _) = *outer {
                if let Term::Global(op) = *inner {
                    assert_eq!(op, "sub");
                } else {
                    panic!("Expected Global sub");
                }
            } else {
                panic!("Expected inner App");
            }
        } else {
            panic!("Expected outer App");
        }
    }

    #[test]
    fn test_parse_infix_mul() {
        let mut parser = LiterateParser::new("x * y");
        let term = parser.parse_term().unwrap();
        if let Term::App(outer, _) = term {
            if let Term::App(inner, _) = *outer {
                if let Term::Global(op) = *inner {
                    assert_eq!(op, "mul");
                } else {
                    panic!("Expected Global mul");
                }
            } else {
                panic!("Expected inner App");
            }
        } else {
            panic!("Expected outer App");
        }
    }

    #[test]
    fn test_parse_precedence_mul_over_add() {
        // x + y * z should parse as add(x, mul(y, z))
        let mut parser = LiterateParser::new("x + y * z");
        let term = parser.parse_term().unwrap();
        // Outer should be add
        if let Term::App(outer, rhs) = term {
            if let Term::App(inner, _) = *outer {
                if let Term::Global(op) = *inner {
                    assert_eq!(op, "add");
                } else {
                    panic!("Expected add");
                }
            } else {
                panic!("Expected inner App");
            }
            // rhs should be mul(y, z)
            if let Term::App(mul_outer, _) = *rhs {
                if let Term::App(mul_inner, _) = *mul_outer {
                    if let Term::Global(op) = *mul_inner {
                        assert_eq!(op, "mul");
                    } else {
                        panic!("Expected mul");
                    }
                } else {
                    panic!("Expected mul inner App");
                }
            } else {
                panic!("Expected mul App");
            }
        } else {
            panic!("Expected outer App");
        }
    }

    #[test]
    fn test_parse_left_associative_add() {
        // x + y + z should parse as add(add(x, y), z)
        let mut parser = LiterateParser::new("x + y + z");
        let term = parser.parse_term().unwrap();
        // Outer add, lhs is add(x, y), rhs is z
        if let Term::App(outer, rhs) = term {
            if let Term::Global(z) = *rhs {
                assert_eq!(z, "z");
            } else {
                panic!("Expected z");
            }
            if let Term::App(mid, lhs_add) = *outer {
                if let Term::Global(op) = *mid {
                    assert_eq!(op, "add");
                }
                // lhs_add should be add(x, y)
                if let Term::App(inner_outer, _) = *lhs_add {
                    if let Term::App(inner_inner, _) = *inner_outer {
                        if let Term::Global(op2) = *inner_inner {
                            assert_eq!(op2, "add");
                        }
                    }
                }
            }
        } else {
            panic!("Expected App");
        }
    }

    #[test]
    fn test_parse_comparison_with_arithmetic() {
        // x + 1 <= y * 2 should parse as le(add(x, 1), mul(y, 2))
        let mut parser = LiterateParser::new("x + 1 <= y * 2");
        let term = parser.parse_term().unwrap();
        if let Term::App(outer, _) = term {
            if let Term::App(inner, _) = *outer {
                if let Term::Global(op) = *inner {
                    assert_eq!(op, "le");
                } else {
                    panic!("Expected le");
                }
            } else {
                panic!("Expected inner App");
            }
        } else {
            panic!("Expected outer App");
        }
    }

    #[test]
    fn test_parse_unicode_le() {
        let mut parser = LiterateParser::new("x ≤ y");
        let term = parser.parse_term().unwrap();
        if let Term::App(outer, _) = term {
            if let Term::App(inner, _) = *outer {
                if let Term::Global(op) = *inner {
                    assert_eq!(op, "le");
                } else {
                    panic!("Expected le for ≤");
                }
            }
        } else {
            panic!("Expected App");
        }
    }

    #[test]
    fn test_parse_unicode_ge() {
        let mut parser = LiterateParser::new("x ≥ y");
        let term = parser.parse_term().unwrap();
        if let Term::App(outer, _) = term {
            if let Term::App(inner, _) = *outer {
                if let Term::Global(op) = *inner {
                    assert_eq!(op, "ge");
                } else {
                    panic!("Expected ge for ≥");
                }
            }
        } else {
            panic!("Expected App");
        }
    }

    #[test]
    fn test_parse_negative_number_preserved() {
        // -5 + x should NOT parse - as subtraction; should be add(-5, x)
        let mut parser = LiterateParser::new("-5 + x");
        let term = parser.parse_term().unwrap();
        // Should be add(-5, x)
        if let Term::App(outer, _) = term {
            if let Term::App(inner, lhs) = *outer {
                if let Term::Global(op) = *inner {
                    assert_eq!(op, "add");
                }
                if let Term::Lit(Literal::Int(n)) = *lhs {
                    assert_eq!(n, -5);
                } else {
                    panic!("Expected -5 literal");
                }
            }
        } else {
            panic!("Expected App");
        }
    }

    #[test]
    fn test_parse_subtraction_not_negative() {
        // x - 5 should parse as sub(x, 5)
        let mut parser = LiterateParser::new("x - 5");
        let term = parser.parse_term().unwrap();
        if let Term::App(outer, rhs) = term {
            if let Term::App(inner, _) = *outer {
                if let Term::Global(op) = *inner {
                    assert_eq!(op, "sub");
                }
            }
            if let Term::Lit(Literal::Int(n)) = *rhs {
                assert_eq!(n, 5); // Should be positive 5, not -5
            } else {
                panic!("Expected 5 literal");
            }
        } else {
            panic!("Expected App");
        }
    }

    #[test]
    fn test_parse_infix_with_sexp_mix() {
        // Ensure S-expression syntax still works: (add x y) <= z
        let mut parser = LiterateParser::new("(add x y) <= z");
        let term = parser.parse_term().unwrap();
        if let Term::App(outer, _) = term {
            if let Term::App(inner, lhs) = *outer {
                if let Term::Global(op) = *inner {
                    assert_eq!(op, "le");
                }
                // lhs should be add(x, y) from S-expression parsing
                if let Term::App(add_outer, _) = *lhs {
                    if let Term::App(add_inner, _) = *add_outer {
                        if let Term::Global(add_op) = *add_inner {
                            assert_eq!(add_op, "add");
                        }
                    }
                }
            }
        } else {
            panic!("Expected App");
        }
    }
}
