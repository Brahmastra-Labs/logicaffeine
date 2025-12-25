
#[derive(Debug, Clone)]
pub struct GradeResult {
    pub correct: bool,
    pub partial: bool,
    pub score: u32,
    pub feedback: String,
}

impl GradeResult {
    pub fn correct() -> Self {
        Self {
            correct: true,
            partial: false,
            score: 100,
            feedback: "Correct!".to_string(),
        }
    }

    pub fn partial(feedback: String, score: u32) -> Self {
        Self {
            correct: false,
            partial: true,
            score,
            feedback,
        }
    }

    pub fn incorrect(feedback: String) -> Self {
        Self {
            correct: false,
            partial: false,
            score: 0,
            feedback,
        }
    }
}

pub fn check_answer(user_input: &str, expected: &str) -> GradeResult {
    let user_normalized = normalize_logic(user_input);
    let expected_normalized = normalize_logic(expected);

    if user_normalized == expected_normalized {
        return GradeResult::correct();
    }

    let user_parsed = parse_to_normalized_ast(user_input);
    let expected_parsed = parse_to_normalized_ast(expected);

    match (user_parsed, expected_parsed) {
        (Some(user_ast), Some(expected_ast)) => {
            if structural_eq(&user_ast, &expected_ast) {
                return GradeResult::correct();
            }

            let similarity = structural_similarity(&user_ast, &expected_ast);
            if similarity > 0.7 {
                GradeResult::partial(
                    "Close! Check your quantifier or connective structure.".to_string(),
                    (similarity * 50.0) as u32,
                )
            } else if similarity > 0.4 {
                GradeResult::partial(
                    "Partially correct. Review the logical structure.".to_string(),
                    (similarity * 30.0) as u32,
                )
            } else {
                GradeResult::incorrect(
                    "Not quite. Consider the relationship between subject and predicate.".to_string(),
                )
            }
        }
        (None, _) => GradeResult::incorrect(
            "Could not parse your answer. Check syntax.".to_string(),
        ),
        (_, None) => GradeResult::incorrect(
            "Internal error: could not parse expected answer.".to_string(),
        ),
    }
}

fn normalize_logic(input: &str) -> String {
    let mut result = input.to_string();

    result = result.replace("\\forall", "∀");
    result = result.replace("\\exists", "∃");
    result = result.replace("\\neg", "¬");
    result = result.replace("\\land", "∧");
    result = result.replace("\\lor", "∨");
    result = result.replace("\\supset", "→");
    result = result.replace("\\equiv", "↔");
    result = result.replace("\\Box", "□");
    result = result.replace("\\Diamond", "◇");

    // Order matters: replace <-> before ->
    result = result.replace("<->", "↔");
    result = result.replace("->", "→");
    result = result.replace("&", "∧");
    result = result.replace("|", "∨");
    result = result.replace("~", "¬");
    result = result.replace("!", "¬");

    result = result.chars().filter(|c| !c.is_whitespace()).collect();

    result
}

#[derive(Debug, Clone)]
struct NormalizedExpr {
    kind: NormalizedKind,
}

#[derive(Debug, Clone)]
enum NormalizedKind {
    Predicate { name: String, arity: usize },
    Quantifier { kind: String, body: Box<NormalizedExpr> },
    Binary { op: String, left: Box<NormalizedExpr>, right: Box<NormalizedExpr> },
    Unary { op: String, operand: Box<NormalizedExpr> },
    Atom(String),
}

fn parse_to_normalized_ast(input: &str) -> Option<NormalizedExpr> {
    let normalized = normalize_logic(input);

    if normalized.starts_with('∀') || normalized.starts_with('∃') {
        let quantifier = if normalized.starts_with('∀') { "∀" } else { "∃" };
        let rest = &normalized[quantifier.len()..];

        if let Some(paren_start) = rest.find('(') {
            let body = &rest[paren_start..];
            if let Some(inner) = extract_balanced(body) {
                return Some(NormalizedExpr {
                    kind: NormalizedKind::Quantifier {
                        kind: quantifier.to_string(),
                        body: Box::new(parse_to_normalized_ast(&inner)?),
                    },
                });
            }
        }
    }

    if let Some(impl_pos) = find_main_connective(&normalized, "→") {
        let left = &normalized[..impl_pos];
        let right = &normalized[impl_pos + "→".len()..];
        return Some(NormalizedExpr {
            kind: NormalizedKind::Binary {
                op: "→".to_string(),
                left: Box::new(parse_to_normalized_ast(left)?),
                right: Box::new(parse_to_normalized_ast(right)?),
            },
        });
    }

    if let Some(and_pos) = find_main_connective(&normalized, "∧") {
        let left = &normalized[..and_pos];
        let right = &normalized[and_pos + "∧".len()..];
        return Some(NormalizedExpr {
            kind: NormalizedKind::Binary {
                op: "∧".to_string(),
                left: Box::new(parse_to_normalized_ast(left)?),
                right: Box::new(parse_to_normalized_ast(right)?),
            },
        });
    }

    if normalized.starts_with('¬') {
        let operand = &normalized["¬".len()..];
        return Some(NormalizedExpr {
            kind: NormalizedKind::Unary {
                op: "¬".to_string(),
                operand: Box::new(parse_to_normalized_ast(operand)?),
            },
        });
    }

    if let Some(paren_pos) = normalized.find('(') {
        let name = &normalized[..paren_pos];
        let args = &normalized[paren_pos..];
        let arity = args.matches(',').count() + 1;
        return Some(NormalizedExpr {
            kind: NormalizedKind::Predicate {
                name: name.to_string(),
                arity,
            },
        });
    }

    Some(NormalizedExpr {
        kind: NormalizedKind::Atom(normalized),
    })
}

fn extract_balanced(s: &str) -> Option<String> {
    if !s.starts_with('(') {
        return None;
    }

    let mut depth = 0;
    let mut end = 0;

    for (i, c) in s.chars().enumerate() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
    }

    if depth == 0 && end > 0 {
        Some(s[1..end].to_string())
    } else {
        None
    }
}

fn find_main_connective(s: &str, connective: &str) -> Option<usize> {
    let mut depth = 0;
    let mut byte_idx = 0;

    for c in s.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ if depth == 0 && s[byte_idx..].starts_with(connective) => {
                return Some(byte_idx);
            }
            _ => {}
        }
        byte_idx += c.len_utf8();
    }

    None
}

fn structural_eq(a: &NormalizedExpr, b: &NormalizedExpr) -> bool {
    match (&a.kind, &b.kind) {
        (NormalizedKind::Predicate { name: n1, arity: a1 }, NormalizedKind::Predicate { name: n2, arity: a2 }) => {
            n1 == n2 && a1 == a2
        }
        (NormalizedKind::Quantifier { kind: k1, body: b1 }, NormalizedKind::Quantifier { kind: k2, body: b2 }) => {
            k1 == k2 && structural_eq(b1, b2)
        }
        (NormalizedKind::Binary { op: o1, left: l1, right: r1 }, NormalizedKind::Binary { op: o2, left: l2, right: r2 }) => {
            if o1 != o2 {
                return false;
            }
            if structural_eq(l1, l2) && structural_eq(r1, r2) {
                return true;
            }
            if o1 == "∧" || o1 == "∨" {
                structural_eq(l1, r2) && structural_eq(r1, l2)
            } else {
                false
            }
        }
        (NormalizedKind::Unary { op: o1, operand: op1 }, NormalizedKind::Unary { op: o2, operand: op2 }) => {
            o1 == o2 && structural_eq(op1, op2)
        }
        (NormalizedKind::Atom(a1), NormalizedKind::Atom(a2)) => a1 == a2,
        _ => false,
    }
}

fn structural_similarity(a: &NormalizedExpr, b: &NormalizedExpr) -> f64 {
    match (&a.kind, &b.kind) {
        (NormalizedKind::Predicate { name: n1, arity: a1 }, NormalizedKind::Predicate { name: n2, arity: a2 }) => {
            let name_match = if n1 == n2 { 0.7 } else { 0.0 };
            let arity_match = if a1 == a2 { 0.3 } else { 0.0 };
            name_match + arity_match
        }
        (NormalizedKind::Quantifier { kind: k1, body: b1 }, NormalizedKind::Quantifier { kind: k2, body: b2 }) => {
            let kind_match = if k1 == k2 { 0.4 } else { 0.0 };
            let body_sim = structural_similarity(b1, b2);
            kind_match + body_sim * 0.6
        }
        (NormalizedKind::Binary { op: o1, left: l1, right: r1 }, NormalizedKind::Binary { op: o2, left: l2, right: r2 }) => {
            let op_match = if o1 == o2 { 0.3 } else { 0.0 };
            let left_sim = structural_similarity(l1, l2);
            let right_sim = structural_similarity(r1, r2);
            op_match + (left_sim + right_sim) * 0.35
        }
        (NormalizedKind::Unary { op: o1, operand: op1 }, NormalizedKind::Unary { op: o2, operand: op2 }) => {
            let op_match = if o1 == o2 { 0.3 } else { 0.0 };
            op_match + structural_similarity(op1, op2) * 0.7
        }
        (NormalizedKind::Atom(a1), NormalizedKind::Atom(a2)) => {
            if a1 == a2 { 1.0 } else { 0.0 }
        }
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        let result = check_answer("∀x(D(x) → B(x))", "∀x(D(x) → B(x))");
        assert!(result.correct, "Exact match should be correct");
    }

    #[test]
    fn test_whitespace_normalization() {
        let result = check_answer("∀x( D(x) → B(x) )", "∀x(D(x)→B(x))");
        assert!(result.correct, "Whitespace should be normalized");
    }

    #[test]
    fn test_latex_to_unicode() {
        let result = check_answer("\\forall x(D(x) \\supset B(x))", "∀x(D(x) → B(x))");
        assert!(result.correct, "LaTeX should normalize to Unicode");
    }

    #[test]
    fn test_ascii_shortcuts() {
        let result = check_answer("D(x) & B(x)", "D(x) ∧ B(x)");
        assert!(result.correct, "ASCII & should match ∧");
    }

    #[test]
    fn test_commutative_conjunction() {
        let result = check_answer("∃x(B(x) ∧ D(x))", "∃x(D(x) ∧ B(x))");
        assert!(result.correct, "Conjunction should be commutative");
    }

    #[test]
    fn test_wrong_quantifier() {
        let result = check_answer("∃x(D(x) → B(x))", "∀x(D(x) → B(x))");
        assert!(!result.correct, "Wrong quantifier should not match");
        assert!(result.partial, "Should get partial credit");
    }

    #[test]
    fn test_wrong_connective() {
        let result = check_answer("∀x(D(x) ∧ B(x))", "∀x(D(x) → B(x))");
        assert!(!result.correct, "Wrong connective should not match");
        assert!(result.partial, "Should get partial credit for structure");
    }

    #[test]
    fn test_completely_wrong() {
        let result = check_answer("P(a)", "∀x(D(x) → B(x))");
        assert!(!result.correct);
        assert!(!result.partial);
    }

    #[test]
    fn test_normalize_arrow() {
        let normalized = normalize_logic("A -> B");
        assert_eq!(normalized, "A→B");
    }

    #[test]
    fn test_normalize_biconditional() {
        let normalized = normalize_logic("A <-> B");
        assert_eq!(normalized, "A↔B");
    }
}
