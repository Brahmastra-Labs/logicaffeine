//! SVA Semantic Model
//!
//! Provides an AST for a subset of SystemVerilog Assertions, a parser
//! for that subset, and structural equivalence checking.
//!
//! This model enables the Z3 semantic equivalence pipeline:
//! FOL (from LOGOS) ↔ SVA (from LLM) checked for structural match.

/// SVA expression AST — models a useful subset of SystemVerilog Assertions.
#[derive(Debug, Clone)]
pub enum SvaExpr {
    /// Signal reference: `req`, `ack`, `data_out`
    Signal(String),
    /// Integer constant with bit width: `8'hFF`
    Const(u64, u32),
    /// Rising edge: `$rose(sig)`
    Rose(Box<SvaExpr>),
    /// Falling edge: `$fell(sig)`
    Fell(Box<SvaExpr>),
    /// Past value: `$past(sig, n)`
    Past(Box<SvaExpr>, u32),
    /// Conjunction: `a && b`
    And(Box<SvaExpr>, Box<SvaExpr>),
    /// Disjunction: `a || b`
    Or(Box<SvaExpr>, Box<SvaExpr>),
    /// Negation: `!a`
    Not(Box<SvaExpr>),
    /// Equality: `a == b`
    Eq(Box<SvaExpr>, Box<SvaExpr>),
    /// SVA implication: `a |-> b` (overlapping) or `a |=> b` (non-overlapping)
    Implication {
        antecedent: Box<SvaExpr>,
        consequent: Box<SvaExpr>,
        overlapping: bool,
    },
    /// Delay: `##[min:max] body`
    Delay {
        body: Box<SvaExpr>,
        min: u32,
        max: Option<u32>,
    },
    /// Strong eventually: `s_eventually(body)`
    SEventually(Box<SvaExpr>),
}

/// Parse error for SVA subset.
#[derive(Debug)]
pub struct SvaParseError {
    pub message: String,
}

impl std::fmt::Display for SvaParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SVA parse error: {}", self.message)
    }
}

/// Parse a subset of SVA text into an SvaExpr.
///
/// Supports: signals, `$rose()`, `$fell()`, `s_eventually()`,
/// `!()`, `&&`, `||`, `==`, `|->`, `|=>`.
pub fn parse_sva(input: &str) -> Result<SvaExpr, SvaParseError> {
    let input = input.trim();

    // Strip clock sensitivity prefix if present
    let input = if input.starts_with("@(") {
        if let Some(pos) = input.find(')') {
            input[pos + 1..].trim()
        } else {
            input
        }
    } else {
        input
    };

    parse_implication(input)
}

fn parse_implication(input: &str) -> Result<SvaExpr, SvaParseError> {
    // Check for |-> or |=>
    // Scan for |-> or |=> not inside parentheses
    let mut depth = 0i32;
    let chars: Vec<char> = input.chars().collect();
    for i in 0..chars.len().saturating_sub(2) {
        match chars[i] {
            '(' => depth += 1,
            ')' => depth -= 1,
            '|' if depth == 0 => {
                if i + 2 < chars.len() && chars[i + 1] == '-' && chars[i + 2] == '>' {
                    let lhs = input[..i].trim();
                    let rhs = input[i + 3..].trim();
                    return Ok(SvaExpr::Implication {
                        antecedent: Box::new(parse_or(lhs)?),
                        consequent: Box::new(parse_or(rhs)?),
                        overlapping: true,
                    });
                }
                if i + 2 < chars.len() && chars[i + 1] == '=' && chars[i + 2] == '>' {
                    let lhs = input[..i].trim();
                    let rhs = input[i + 3..].trim();
                    return Ok(SvaExpr::Implication {
                        antecedent: Box::new(parse_or(lhs)?),
                        consequent: Box::new(parse_or(rhs)?),
                        overlapping: false,
                    });
                }
            }
            _ => {}
        }
    }
    parse_or(input)
}

fn parse_or(input: &str) -> Result<SvaExpr, SvaParseError> {
    let mut depth = 0i32;
    let chars: Vec<char> = input.chars().collect();
    for i in 0..chars.len().saturating_sub(1) {
        match chars[i] {
            '(' => depth += 1,
            ')' => depth -= 1,
            '|' if depth == 0 && i + 1 < chars.len() && chars[i + 1] == '|' => {
                let lhs = input[..i].trim();
                let rhs = input[i + 2..].trim();
                return Ok(SvaExpr::Or(
                    Box::new(parse_and(lhs)?),
                    Box::new(parse_or(rhs)?),
                ));
            }
            _ => {}
        }
    }
    parse_and(input)
}

fn parse_and(input: &str) -> Result<SvaExpr, SvaParseError> {
    let mut depth = 0i32;
    let chars: Vec<char> = input.chars().collect();
    for i in 0..chars.len().saturating_sub(1) {
        match chars[i] {
            '(' => depth += 1,
            ')' => depth -= 1,
            '&' if depth == 0 && i + 1 < chars.len() && chars[i + 1] == '&' => {
                let lhs = input[..i].trim();
                let rhs = input[i + 2..].trim();
                return Ok(SvaExpr::And(
                    Box::new(parse_eq(lhs)?),
                    Box::new(parse_and(rhs)?),
                ));
            }
            _ => {}
        }
    }
    parse_eq(input)
}

fn parse_eq(input: &str) -> Result<SvaExpr, SvaParseError> {
    let mut depth = 0i32;
    let chars: Vec<char> = input.chars().collect();
    for i in 0..chars.len().saturating_sub(1) {
        match chars[i] {
            '(' => depth += 1,
            ')' => depth -= 1,
            '=' if depth == 0 && i + 1 < chars.len() && chars[i + 1] == '=' => {
                let lhs = input[..i].trim();
                let rhs = input[i + 2..].trim();
                return Ok(SvaExpr::Eq(
                    Box::new(parse_unary(lhs)?),
                    Box::new(parse_unary(rhs)?),
                ));
            }
            _ => {}
        }
    }
    parse_unary(input)
}

fn parse_unary(input: &str) -> Result<SvaExpr, SvaParseError> {
    let input = input.trim();

    // Delay: ##N body or ##[min:max] body
    if input.starts_with("##") {
        let rest = &input[2..];
        if rest.starts_with('[') {
            // ##[min:max] body
            if let Some(bracket_end) = rest.find(']') {
                let range_str = &rest[1..bracket_end];
                let body_str = rest[bracket_end + 1..].trim();
                let parts: Vec<&str> = range_str.split(':').collect();
                if parts.len() == 2 {
                    let min = parts[0].trim().parse::<u32>().map_err(|_| SvaParseError {
                        message: format!("invalid delay min: '{}'", parts[0]),
                    })?;
                    let max_str = parts[1].trim();
                    let max = if max_str == "$" {
                        Some(u32::MAX)
                    } else {
                        Some(max_str.parse::<u32>().map_err(|_| SvaParseError {
                            message: format!("invalid delay max: '{}'", max_str),
                        })?)
                    };
                    return Ok(SvaExpr::Delay {
                        body: Box::new(parse_unary(body_str)?),
                        min,
                        max,
                    });
                }
            }
        } else {
            // ##N body — exact delay
            let mut num_end = 0;
            for c in rest.chars() {
                if c.is_ascii_digit() {
                    num_end += 1;
                } else {
                    break;
                }
            }
            if num_end > 0 {
                let n = rest[..num_end].parse::<u32>().map_err(|_| SvaParseError {
                    message: format!("invalid delay number: '{}'", &rest[..num_end]),
                })?;
                let body_str = rest[num_end..].trim();
                return Ok(SvaExpr::Delay {
                    body: Box::new(parse_unary(body_str)?),
                    min: n,
                    max: None,
                });
            }
        }
    }

    // Negation: !(...)
    if input.starts_with('!') {
        let inner = input[1..].trim();
        let inner = strip_parens(inner);
        return Ok(SvaExpr::Not(Box::new(parse_implication(inner)?)));
    }

    // $rose(...)
    if input.starts_with("$rose(") && input.ends_with(')') {
        let inner = &input[6..input.len() - 1];
        return Ok(SvaExpr::Rose(Box::new(parse_atom(inner.trim())?)));
    }

    // $fell(...)
    if input.starts_with("$fell(") && input.ends_with(')') {
        let inner = &input[6..input.len() - 1];
        return Ok(SvaExpr::Fell(Box::new(parse_atom(inner.trim())?)));
    }

    // s_eventually(...)
    if input.starts_with("s_eventually(") && input.ends_with(')') {
        let inner = &input[13..input.len() - 1];
        return Ok(SvaExpr::SEventually(Box::new(parse_atom(inner.trim())?)));
    }

    // Parenthesized expression
    if input.starts_with('(') && input.ends_with(')') {
        return parse_implication(&input[1..input.len() - 1]);
    }

    parse_atom(input)
}

fn parse_atom(input: &str) -> Result<SvaExpr, SvaParseError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(SvaParseError {
            message: "empty expression".to_string(),
        });
    }

    // Check if it's a number
    if let Ok(n) = input.parse::<u64>() {
        return Ok(SvaExpr::Const(n, 32));
    }

    // Must be a signal name
    if input
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_')
    {
        return Ok(SvaExpr::Signal(input.to_string()));
    }

    Err(SvaParseError {
        message: format!("unexpected token: '{}'", input),
    })
}

/// Render an SvaExpr back to valid SVA text.
/// Closes the round-trip: parse_sva(text) → SvaExpr → sva_expr_to_string → text.
pub fn sva_expr_to_string(expr: &SvaExpr) -> String {
    match expr {
        SvaExpr::Signal(name) => name.clone(),
        SvaExpr::Const(value, width) => format!("{}'d{}", width, value),
        SvaExpr::Rose(inner) => format!("$rose({})", sva_expr_to_string(inner)),
        SvaExpr::Fell(inner) => format!("$fell({})", sva_expr_to_string(inner)),
        SvaExpr::Past(inner, n) => format!("$past({}, {})", sva_expr_to_string(inner), n),
        SvaExpr::And(left, right) => {
            format!("({} && {})", sva_expr_to_string(left), sva_expr_to_string(right))
        }
        SvaExpr::Or(left, right) => {
            format!("({} || {})", sva_expr_to_string(left), sva_expr_to_string(right))
        }
        SvaExpr::Not(inner) => format!("!({})", sva_expr_to_string(inner)),
        SvaExpr::Eq(left, right) => {
            format!("({} == {})", sva_expr_to_string(left), sva_expr_to_string(right))
        }
        SvaExpr::Implication {
            antecedent,
            consequent,
            overlapping,
        } => {
            let op = if *overlapping { "|->" } else { "|=>" };
            format!(
                "{} {} {}",
                sva_expr_to_string(antecedent),
                op,
                sva_expr_to_string(consequent)
            )
        }
        SvaExpr::Delay { body, min, max } => match max {
            Some(max_val) => format!("##[{}:{}] {}", min, max_val, sva_expr_to_string(body)),
            None => format!("##{} {}", min, sva_expr_to_string(body)),
        },
        SvaExpr::SEventually(inner) => format!("s_eventually({})", sva_expr_to_string(inner)),
    }
}

fn strip_parens(input: &str) -> &str {
    let input = input.trim();
    if input.starts_with('(') && input.ends_with(')') {
        &input[1..input.len() - 1]
    } else {
        input
    }
}

/// Check if two SvaExpr trees are structurally equivalent.
pub fn sva_exprs_structurally_equivalent(a: &SvaExpr, b: &SvaExpr) -> bool {
    match (a, b) {
        (SvaExpr::Signal(sa), SvaExpr::Signal(sb)) => sa == sb,
        (SvaExpr::Const(va, wa), SvaExpr::Const(vb, wb)) => va == vb && wa == wb,
        (SvaExpr::Rose(ia), SvaExpr::Rose(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::Fell(ia), SvaExpr::Fell(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::Past(ia, na), SvaExpr::Past(ib, nb)) => {
            na == nb && sva_exprs_structurally_equivalent(ia, ib)
        }
        (SvaExpr::And(la, ra), SvaExpr::And(lb, rb)) => {
            sva_exprs_structurally_equivalent(la, lb)
                && sva_exprs_structurally_equivalent(ra, rb)
        }
        (SvaExpr::Or(la, ra), SvaExpr::Or(lb, rb)) => {
            sva_exprs_structurally_equivalent(la, lb)
                && sva_exprs_structurally_equivalent(ra, rb)
        }
        (SvaExpr::Not(ia), SvaExpr::Not(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::Eq(la, ra), SvaExpr::Eq(lb, rb)) => {
            sva_exprs_structurally_equivalent(la, lb)
                && sva_exprs_structurally_equivalent(ra, rb)
        }
        (
            SvaExpr::Implication {
                antecedent: aa,
                consequent: ca,
                overlapping: oa,
            },
            SvaExpr::Implication {
                antecedent: ab,
                consequent: cb,
                overlapping: ob,
            },
        ) => {
            oa == ob
                && sva_exprs_structurally_equivalent(aa, ab)
                && sva_exprs_structurally_equivalent(ca, cb)
        }
        (
            SvaExpr::Delay {
                body: ba,
                min: mna,
                max: mxa,
            },
            SvaExpr::Delay {
                body: bb,
                min: mnb,
                max: mxb,
            },
        ) => mna == mnb && mxa == mxb && sva_exprs_structurally_equivalent(ba, bb),
        (SvaExpr::SEventually(ia), SvaExpr::SEventually(ib)) => {
            sva_exprs_structurally_equivalent(ia, ib)
        }
        _ => false,
    }
}
