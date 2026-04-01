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
    /// Sequence repetition: `body[*N]` or `body[*min:max]`
    Repetition {
        body: Box<SvaExpr>,
        min: u32,
        max: Option<u32>, // None = unbounded ($)
    },
    /// Strong eventually: `s_eventually(body)`
    SEventually(Box<SvaExpr>),
    /// Strong always: `s_always(body)`
    SAlways(Box<SvaExpr>),
    /// Stable: `$stable(sig)` — signal unchanged from previous cycle
    Stable(Box<SvaExpr>),
    /// Changed: `$changed(sig)` — signal changed from previous cycle
    Changed(Box<SvaExpr>),
    /// Disable condition: `disable iff (cond) body`
    DisableIff {
        condition: Box<SvaExpr>,
        body: Box<SvaExpr>,
    },
    /// Next time: `nexttime(body)` or `nexttime[N](body)`
    Nexttime(Box<SvaExpr>, u32),
    /// Conditional property: `if (cond) P else Q`
    IfElse {
        condition: Box<SvaExpr>,
        then_expr: Box<SvaExpr>,
        else_expr: Box<SvaExpr>,
    },
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

    parse_toplevel(input)
}

fn parse_toplevel(input: &str) -> Result<SvaExpr, SvaParseError> {
    let input = input.trim();

    // disable iff (cond) body — must be checked before implication
    if input.starts_with("disable iff") {
        let rest = input["disable iff".len()..].trim();
        if rest.starts_with('(') {
            if let Some(close) = find_balanced_close(rest, 0) {
                let cond = &rest[1..close];
                let body = rest[close + 1..].trim();
                return Ok(SvaExpr::DisableIff {
                    condition: Box::new(parse_implication(cond)?),
                    body: Box::new(parse_implication(body)?),
                });
            }
        }
    }

    // if (cond) P else Q — must be checked before implication
    if input.starts_with("if ") || input.starts_with("if(") {
        let rest = input[2..].trim();
        if rest.starts_with('(') {
            if let Some(close) = find_balanced_close(rest, 0) {
                let cond = &rest[1..close];
                let after_cond = rest[close + 1..].trim();
                if let Some(else_pos) = find_else_keyword(after_cond) {
                    let then_part = after_cond[..else_pos].trim();
                    let else_part = after_cond[else_pos + 4..].trim();
                    return Ok(SvaExpr::IfElse {
                        condition: Box::new(parse_implication(cond)?),
                        then_expr: Box::new(parse_implication(then_part)?),
                        else_expr: Box::new(parse_implication(else_part)?),
                    });
                } else {
                    return Ok(SvaExpr::IfElse {
                        condition: Box::new(parse_implication(cond)?),
                        then_expr: Box::new(parse_implication(after_cond)?),
                        else_expr: Box::new(SvaExpr::Signal("1".to_string())),
                    });
                }
            }
        }
    }

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

    // $rose(...), $fell(...), $stable(...), $changed(...), s_eventually(...), $nexttime(...)
    // Use balanced paren matching so "$fell(sda) && scl" correctly parses $fell(sda) only
    if let Some(result) = try_parse_function_call(input, "$rose", |inner| {
        Ok(SvaExpr::Rose(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    if let Some(result) = try_parse_function_call(input, "$fell", |inner| {
        Ok(SvaExpr::Fell(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    if let Some(result) = try_parse_function_call(input, "$stable", |inner| {
        Ok(SvaExpr::Stable(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    if let Some(result) = try_parse_function_call(input, "$changed", |inner| {
        Ok(SvaExpr::Changed(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    if let Some(result) = try_parse_function_call(input, "s_eventually", |inner| {
        Ok(SvaExpr::SEventually(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    if let Some(result) = try_parse_function_call(input, "s_always", |inner| {
        Ok(SvaExpr::SAlways(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    // nexttime[N](body) — with explicit count
    if input.starts_with("nexttime[") {
        if let Some(bracket_end) = input.find(']') {
            let n_str = &input[9..bracket_end];
            if let Ok(n) = n_str.parse::<u32>() {
                let rest = input[bracket_end + 1..].trim();
                if rest.starts_with('(') {
                    if let Some(close) = find_balanced_close(rest, 0) {
                        let inner = &rest[1..close];
                        return Ok(SvaExpr::Nexttime(
                            Box::new(parse_implication(inner.trim())?),
                            n,
                        ));
                    }
                }
            }
        }
    }

    // nexttime(body) — default count = 1
    if let Some(result) = try_parse_function_call(input, "nexttime", |inner| {
        Ok(SvaExpr::Nexttime(Box::new(parse_implication(inner)?), 1))
    })? { return Ok(result); }

    if let Some(result) = try_parse_function_call(input, "$nexttime", |inner| {
        Ok(SvaExpr::Nexttime(Box::new(parse_implication(inner)?), 1))
    })? { return Ok(result); }

    if let Some(result) = try_parse_function_call(input, "$past", |inner| {
        // $past(sig, n) — parse the signal and count
        if let Some(comma) = inner.find(',') {
            let sig = inner[..comma].trim();
            let n_str = inner[comma + 1..].trim();
            let n = n_str.parse::<u32>().unwrap_or(1);
            Ok(SvaExpr::Past(Box::new(parse_atom(sig)?), n))
        } else {
            Ok(SvaExpr::Past(Box::new(parse_atom(inner)?), 1))
        }
    })? { return Ok(result); }

    // Parenthesized expression
    if input.starts_with('(') && input.ends_with(')') {
        return parse_implication(&input[1..input.len() - 1]);
    }

    parse_atom(input)
}

/// Find the closing paren that balances the opening paren at `start`.
/// Returns the index of the closing ')' relative to the input string.
fn find_balanced_close(input: &str, start: usize) -> Option<usize> {
    let chars: Vec<char> = input.chars().collect();
    let mut depth = 0i32;
    for i in start..chars.len() {
        match chars[i] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Try to parse a function call like `$rose(expr)` with balanced parens.
/// If the input starts with `prefix(`, extracts the balanced inner expression,
/// parses it with the provided closure, and returns the result.
/// If there's content after the closing paren, this returns None so the caller
/// can try parsing at a higher level (e.g., `$rose(sig) && other` should be
/// parsed as And($rose(sig), other) at the And level, not here).
fn try_parse_function_call<F>(
    input: &str,
    prefix: &str,
    parse_inner: F,
) -> Result<Option<SvaExpr>, SvaParseError>
where
    F: FnOnce(&str) -> Result<SvaExpr, SvaParseError>,
{
    let full_prefix = format!("{}(", prefix);
    if !input.starts_with(&full_prefix) {
        return Ok(None);
    }
    let paren_start = full_prefix.len() - 1; // index of '('
    if let Some(close) = find_balanced_close(input, paren_start) {
        let inner = &input[full_prefix.len()..close];
        let remaining = input[close + 1..].trim();
        if remaining.is_empty() {
            // Simple case: $rose(sig) with nothing after
            return Ok(Some(parse_inner(inner.trim())?));
        }
        // There's stuff after the closing paren (e.g., "$rose(sig) && other")
        // Parse just the function call, then let the caller handle the rest
        // We can't handle this at the unary level — return None so the
        // expression gets reparsed at the binary operator level.
        // But we need to handle it: wrap as atom.
        // Actually, re-parse the entire input through the binary operators:
        // The issue is that "$fell(sda) && scl" is at the AND level, not unary.
        // So we parse just "$fell(sda)" as the left side of AND.
        return Ok(None);
    }
    Err(SvaParseError {
        message: format!("unbalanced parens in {}", prefix),
    })
}

/// Find the position of the top-level "else" keyword (not inside parens).
fn find_else_keyword(input: &str) -> Option<usize> {
    let mut depth = 0i32;
    let bytes = input.as_bytes();
    for i in 0..input.len().saturating_sub(3) {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'e' if depth == 0 => {
                if input[i..].starts_with("else") {
                    // Check word boundary
                    let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
                    let after_ok = i + 4 >= input.len() || !bytes[i + 4].is_ascii_alphanumeric();
                    if before_ok && after_ok {
                        return Some(i);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_atom(input: &str) -> Result<SvaExpr, SvaParseError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(SvaParseError {
            message: "empty expression".to_string(),
        });
    }

    // Check for repetition: signal[*N] or signal[*min:max]
    if let Some(bracket_pos) = input.find("[*") {
        let signal_part = input[..bracket_pos].trim();
        let rep_part = &input[bracket_pos + 2..];
        if let Some(close_bracket) = rep_part.find(']') {
            let range_str = &rep_part[..close_bracket];
            let body = parse_atom(signal_part)?;
            if let Some(colon) = range_str.find(':') {
                let min_str = range_str[..colon].trim();
                let max_str = range_str[colon + 1..].trim();
                let min = min_str.parse::<u32>().map_err(|_| SvaParseError {
                    message: format!("invalid repetition min: '{}'", min_str),
                })?;
                let max = if max_str == "$" {
                    None
                } else {
                    Some(max_str.parse::<u32>().map_err(|_| SvaParseError {
                        message: format!("invalid repetition max: '{}'", max_str),
                    })?)
                };
                return Ok(SvaExpr::Repetition {
                    body: Box::new(body),
                    min,
                    max,
                });
            } else {
                // Exact repetition: [*N]
                let n = range_str.trim().parse::<u32>().map_err(|_| SvaParseError {
                    message: format!("invalid repetition count: '{}'", range_str),
                })?;
                return Ok(SvaExpr::Repetition {
                    body: Box::new(body),
                    min: n,
                    max: Some(n),
                });
            }
        }
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
        SvaExpr::Repetition { body, min, max } => {
            let body_str = sva_expr_to_string(body);
            match max {
                Some(m) if *m == *min => format!("{}[*{}]", body_str, min),
                Some(m) => format!("{}[*{}:{}]", body_str, min, m),
                None => format!("{}[*{}:$]", body_str, min),
            }
        }
        SvaExpr::SEventually(inner) => format!("s_eventually({})", sva_expr_to_string(inner)),
        SvaExpr::SAlways(inner) => format!("s_always({})", sva_expr_to_string(inner)),
        SvaExpr::Stable(inner) => format!("$stable({})", sva_expr_to_string(inner)),
        SvaExpr::Changed(inner) => format!("$changed({})", sva_expr_to_string(inner)),
        SvaExpr::Nexttime(inner, n) => {
            if *n == 1 {
                format!("nexttime({})", sva_expr_to_string(inner))
            } else {
                format!("nexttime[{}]({})", n, sva_expr_to_string(inner))
            }
        }
        SvaExpr::DisableIff { condition, body } => {
            format!("disable iff ({}) {}", sva_expr_to_string(condition), sva_expr_to_string(body))
        }
        SvaExpr::IfElse { condition, then_expr, else_expr } => {
            format!(
                "if ({}) {} else {}",
                sva_expr_to_string(condition),
                sva_expr_to_string(then_expr),
                sva_expr_to_string(else_expr),
            )
        }
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
        (
            SvaExpr::Repetition { body: ba, min: mna, max: mxa },
            SvaExpr::Repetition { body: bb, min: mnb, max: mxb },
        ) => mna == mnb && mxa == mxb && sva_exprs_structurally_equivalent(ba, bb),
        (SvaExpr::SEventually(ia), SvaExpr::SEventually(ib)) => {
            sva_exprs_structurally_equivalent(ia, ib)
        }
        (SvaExpr::SAlways(ia), SvaExpr::SAlways(ib)) => {
            sva_exprs_structurally_equivalent(ia, ib)
        }
        (SvaExpr::Stable(ia), SvaExpr::Stable(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::Changed(ia), SvaExpr::Changed(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::Nexttime(ia, na), SvaExpr::Nexttime(ib, nb)) => {
            na == nb && sva_exprs_structurally_equivalent(ia, ib)
        }
        (
            SvaExpr::DisableIff { condition: ca, body: ba },
            SvaExpr::DisableIff { condition: cb, body: bb },
        ) => {
            sva_exprs_structurally_equivalent(ca, cb)
                && sva_exprs_structurally_equivalent(ba, bb)
        }
        (
            SvaExpr::IfElse { condition: ca, then_expr: ta, else_expr: ea },
            SvaExpr::IfElse { condition: cb, then_expr: tb, else_expr: eb },
        ) => {
            sva_exprs_structurally_equivalent(ca, cb)
                && sva_exprs_structurally_equivalent(ta, tb)
                && sva_exprs_structurally_equivalent(ea, eb)
        }
        _ => false,
    }
}
