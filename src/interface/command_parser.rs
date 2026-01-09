//! Parser for Vernacular commands.
//!
//! Commands:
//! - Definition name : type := term.
//! - Definition name := term.  (type inferred)
//! - Check term.
//! - Eval term.
//! - Inductive Name := C1 : T1 | C2 : T2.

use super::command::Command;
use super::error::ParseError;
use super::term_parser::TermParser;
use crate::kernel::{Term, Universe};

/// Parse a command from input string.
pub fn parse_command(input: &str) -> Result<Command, ParseError> {
    let input = input.trim();

    // Remove trailing period if present
    let input = input.strip_suffix('.').unwrap_or(input).trim();

    if input.starts_with("Definition") {
        parse_definition(&input[10..].trim_start())
    } else if input.starts_with("Check") {
        parse_check(&input[5..].trim_start())
    } else if input.starts_with("Eval") {
        parse_eval(&input[4..].trim_start())
    } else if input.starts_with("Inductive") {
        parse_inductive(&input[9..].trim_start())
    } else {
        Err(ParseError::UnknownCommand(
            input.split_whitespace().next().unwrap_or(input).to_string(),
        ))
    }
}

/// Parse: name : type := term  OR  name := term
fn parse_definition(input: &str) -> Result<Command, ParseError> {
    // Find the := delimiter
    let assign_pos = input.find(":=").ok_or(ParseError::Missing(":=".to_string()))?;

    let before_assign = input[..assign_pos].trim();
    let body_str = input[assign_pos + 2..].trim();

    // Check if there's a type annotation (: before :=)
    if let Some(colon_pos) = before_assign.find(':') {
        // Has type annotation: name : type
        let name = before_assign[..colon_pos].trim().to_string();
        let type_str = before_assign[colon_pos + 1..].trim();

        if name.is_empty() {
            return Err(ParseError::Missing("definition name".to_string()));
        }

        let ty = TermParser::parse(type_str)?;
        let body = TermParser::parse(body_str)?;

        Ok(Command::Definition {
            name,
            ty: Some(ty),
            body,
        })
    } else {
        // No type annotation: name := term
        let name = before_assign.to_string();

        if name.is_empty() {
            return Err(ParseError::Missing("definition name".to_string()));
        }

        let body = TermParser::parse(body_str)?;

        Ok(Command::Definition {
            name,
            ty: None,
            body,
        })
    }
}

/// Parse: term
fn parse_check(input: &str) -> Result<Command, ParseError> {
    let term = TermParser::parse(input)?;
    Ok(Command::Check(term))
}

/// Parse: term
fn parse_eval(input: &str) -> Result<Command, ParseError> {
    let term = TermParser::parse(input)?;
    Ok(Command::Eval(term))
}

/// Parse: Name (params) := C1 : T1 | C2 : T2
///
/// Supports polymorphic inductives like:
/// `Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.`
fn parse_inductive(input: &str) -> Result<Command, ParseError> {
    // Find the := delimiter
    let assign_pos = input.find(":=").ok_or(ParseError::Missing(":=".to_string()))?;

    let header = input[..assign_pos].trim();
    let ctors_str = input[assign_pos + 2..].trim();

    // Parse header to separate name from parameters
    let (name, params) = parse_inductive_header(header)?;

    if name.is_empty() {
        return Err(ParseError::Missing("inductive name".to_string()));
    }

    // Parse constructors separated by |
    let mut constructors = Vec::new();
    for ctor_part in ctors_str.split('|') {
        let ctor_part = ctor_part.trim();
        if ctor_part.is_empty() {
            continue;
        }

        // Each constructor is: Name : Type
        let colon_pos = ctor_part
            .find(':')
            .ok_or(ParseError::Missing("constructor type annotation".to_string()))?;

        let ctor_name = ctor_part[..colon_pos].trim().to_string();
        let ctor_type_str = ctor_part[colon_pos + 1..].trim();

        if ctor_name.is_empty() {
            return Err(ParseError::Missing("constructor name".to_string()));
        }

        let ctor_type = TermParser::parse(ctor_type_str)?;
        constructors.push((ctor_name, ctor_type));
    }

    if constructors.is_empty() {
        return Err(ParseError::Missing("constructors".to_string()));
    }

    // Default to Type 0 for the sort
    let sort = Term::Sort(Universe::Type(0));

    Ok(Command::Inductive {
        name,
        params,
        sort,
        constructors,
    })
}

/// Parse the inductive header to extract name and type parameters.
///
/// Examples:
/// - `List` -> ("List", [])
/// - `List (A : Type)` -> ("List", [("A", Type)])
/// - `Either (A : Type) (B : Type)` -> ("Either", [("A", Type), ("B", Type)])
fn parse_inductive_header(header: &str) -> Result<(String, Vec<(String, Term)>), ParseError> {
    let header = header.trim();

    // If no '(' found, it's a simple name with no params
    if !header.contains('(') {
        return Ok((header.to_string(), vec![]));
    }

    // Find the first '(' to separate name from params
    let paren_pos = header.find('(').unwrap();
    let name = header[..paren_pos].trim().to_string();
    let params_str = header[paren_pos..].trim();

    // Parse all parameter bindings: (A : Type) (B : Type) ...
    let params = parse_param_bindings(params_str)?;

    Ok((name, params))
}

/// Parse a sequence of parameter bindings: (A : Type) (B : Type)
///
/// Each binding is of the form (name : type).
fn parse_param_bindings(input: &str) -> Result<Vec<(String, Term)>, ParseError> {
    let mut params = Vec::new();
    let mut remaining = input.trim();

    while !remaining.is_empty() {
        // Skip whitespace
        remaining = remaining.trim();
        if remaining.is_empty() {
            break;
        }

        // Expect '('
        if !remaining.starts_with('(') {
            return Err(ParseError::Missing("opening '(' for parameter".to_string()));
        }

        // Find matching ')'
        let close_pos = find_matching_paren(remaining)?;
        let binding = &remaining[1..close_pos]; // Contents inside parens

        // Parse name : type
        let colon_pos = binding
            .find(':')
            .ok_or(ParseError::Missing("':' in parameter binding".to_string()))?;

        let param_name = binding[..colon_pos].trim().to_string();
        let param_type_str = binding[colon_pos + 1..].trim();

        if param_name.is_empty() {
            return Err(ParseError::Missing("parameter name".to_string()));
        }

        let param_type = TermParser::parse(param_type_str)?;
        params.push((param_name, param_type));

        // Move past this binding
        remaining = remaining[close_pos + 1..].trim();
    }

    Ok(params)
}

/// Find the position of the ')' that matches the opening '(' at position 0.
fn find_matching_paren(input: &str) -> Result<usize, ParseError> {
    let mut depth = 0;
    for (i, c) in input.chars().enumerate() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(i);
                }
            }
            _ => {}
        }
    }
    Err(ParseError::Missing("closing ')' for parameter".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_definition_with_type() {
        let cmd = parse_command("Definition one : Nat := Succ Zero.").unwrap();
        if let Command::Definition { name, ty, body } = cmd {
            assert_eq!(name, "one");
            assert!(ty.is_some());
            assert!(matches!(body, Term::App(..)));
        } else {
            panic!("Expected Definition");
        }
    }

    #[test]
    fn test_parse_definition_without_type() {
        let cmd = parse_command("Definition two := Succ (Succ Zero).").unwrap();
        if let Command::Definition { name, ty, .. } = cmd {
            assert_eq!(name, "two");
            assert!(ty.is_none());
        } else {
            panic!("Expected Definition");
        }
    }

    #[test]
    fn test_parse_check() {
        let cmd = parse_command("Check Zero.").unwrap();
        assert!(matches!(cmd, Command::Check(_)));
    }

    #[test]
    fn test_parse_eval() {
        let cmd = parse_command("Eval (Succ Zero).").unwrap();
        assert!(matches!(cmd, Command::Eval(_)));
    }

    #[test]
    fn test_parse_inductive() {
        let cmd = parse_command("Inductive Bool := True : Bool | False : Bool.").unwrap();
        if let Command::Inductive {
            name, constructors, ..
        } = cmd
        {
            assert_eq!(name, "Bool");
            assert_eq!(constructors.len(), 2);
            assert_eq!(constructors[0].0, "True");
            assert_eq!(constructors[1].0, "False");
        } else {
            panic!("Expected Inductive");
        }
    }
}
