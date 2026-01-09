//! Socratic Hint Engine - Generates pedagogical hints for proof guidance.
//!
//! Instead of giving direct answers, this module generates leading questions
//! that help users discover the right proof steps themselves.

use super::{ProofExpr, ProofTerm};

/// Tactics that the user might try
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestedTactic {
    ModusPonens,
    UniversalElim,
    ExistentialIntro,
    AndIntro,
    AndElim,
    OrIntro,
    OrElim,
    Induction,
    Reflexivity,
    Rewrite,
    Assumption,
}

impl SuggestedTactic {
    pub fn name(&self) -> &'static str {
        match self {
            SuggestedTactic::ModusPonens => "Modus Ponens",
            SuggestedTactic::UniversalElim => "Universal Elimination",
            SuggestedTactic::ExistentialIntro => "Existential Introduction",
            SuggestedTactic::AndIntro => "And Introduction",
            SuggestedTactic::AndElim => "And Elimination",
            SuggestedTactic::OrIntro => "Or Introduction",
            SuggestedTactic::OrElim => "Or Elimination (Case Analysis)",
            SuggestedTactic::Induction => "Induction",
            SuggestedTactic::Reflexivity => "Reflexivity",
            SuggestedTactic::Rewrite => "Rewrite",
            SuggestedTactic::Assumption => "Assumption",
        }
    }
}

/// A Socratic hint - a leading question to guide the user
#[derive(Debug, Clone)]
pub struct SocraticHint {
    /// The hint text (a question or observation)
    pub text: String,
    /// The tactic this hint is suggesting
    pub suggested_tactic: Option<SuggestedTactic>,
    /// Priority (higher = more relevant)
    pub priority: u8,
}

impl SocraticHint {
    pub fn new(text: impl Into<String>, tactic: Option<SuggestedTactic>, priority: u8) -> Self {
        SocraticHint {
            text: text.into(),
            suggested_tactic: tactic,
            priority,
        }
    }
}

/// Generate a Socratic hint based on the goal and available knowledge
pub fn suggest_hint(
    goal: &ProofExpr,
    knowledge_base: &[ProofExpr],
    failed_tactics: &[SuggestedTactic],
) -> SocraticHint {
    let mut hints = Vec::new();

    // Analyze goal structure
    analyze_goal_structure(goal, &mut hints);

    // Check if goal matches any premise directly
    check_direct_match(goal, knowledge_base, &mut hints);

    // Look for implications that could prove the goal
    check_implications(goal, knowledge_base, &mut hints);

    // Look for universal statements that could be instantiated
    check_universals(goal, knowledge_base, &mut hints);

    // Check for conjunction/disjunction opportunities
    check_connectives(goal, knowledge_base, &mut hints);

    // Check for equality patterns
    check_equality(goal, knowledge_base, &mut hints);

    // Filter out hints for already-tried tactics
    hints.retain(|h| {
        h.suggested_tactic
            .map(|t| !failed_tactics.contains(&t))
            .unwrap_or(true)
    });

    // Sort by priority (highest first)
    hints.sort_by(|a, b| b.priority.cmp(&a.priority));

    // Return the best hint, or a generic one
    hints.into_iter().next().unwrap_or_else(|| {
        SocraticHint::new(
            "What logical structure does your goal have? Try breaking it down into simpler parts.",
            None,
            0,
        )
    })
}

/// Analyze the structure of the goal to suggest relevant tactics
fn analyze_goal_structure(goal: &ProofExpr, hints: &mut Vec<SocraticHint>) {
    match goal {
        ProofExpr::Implies(_, _) => {
            hints.push(SocraticHint::new(
                "Your goal is an implication P \u{2192} Q. To prove it, assume P and then prove Q.",
                None,
                7,
            ));
        }
        ProofExpr::ForAll { variable, body } => {
            hints.push(SocraticHint::new(
                format!(
                    "Your goal is a universal statement \u{2200}{}. To prove it, consider an arbitrary {} and prove the body.",
                    variable, variable
                ),
                None,
                7,
            ));
        }
        ProofExpr::Exists { variable, body } => {
            hints.push(SocraticHint::new(
                format!(
                    "Your goal is an existential statement \u{2203}{}. You need to find a specific witness.",
                    variable
                ),
                Some(SuggestedTactic::ExistentialIntro),
                7,
            ));
        }
        ProofExpr::And(_, _) => {
            hints.push(SocraticHint::new(
                "Your goal is a conjunction P \u{2227} Q. You need to prove both P and Q separately.",
                Some(SuggestedTactic::AndIntro),
                7,
            ));
        }
        ProofExpr::Or(_, _) => {
            hints.push(SocraticHint::new(
                "Your goal is a disjunction P \u{2228} Q. You only need to prove one of them.",
                Some(SuggestedTactic::OrIntro),
                7,
            ));
        }
        ProofExpr::Not(_) => {
            hints.push(SocraticHint::new(
                "Your goal is a negation \u{00AC}P. Try assuming P and deriving a contradiction.",
                None,
                6,
            ));
        }
        ProofExpr::Identity(left, right) => {
            if left == right {
                hints.push(SocraticHint::new(
                    "Both sides of the equation are identical. Try reflexivity!",
                    Some(SuggestedTactic::Reflexivity),
                    10,
                ));
            } else {
                hints.push(SocraticHint::new(
                    "Your goal is an equality. Can you rewrite one side to match the other?",
                    Some(SuggestedTactic::Rewrite),
                    6,
                ));
            }
        }
        ProofExpr::Predicate { name, .. } => {
            hints.push(SocraticHint::new(
                format!(
                    "Your goal is {}(...). Do you have this as an assumption, or can you derive it?",
                    name
                ),
                None,
                3,
            ));
        }
        _ => {}
    }
}

/// Check if the goal matches any premise directly
fn check_direct_match(goal: &ProofExpr, kb: &[ProofExpr], hints: &mut Vec<SocraticHint>) {
    for premise in kb {
        if premise == goal {
            hints.push(SocraticHint::new(
                "Look carefully at your assumptions. One of them is exactly what you need!",
                Some(SuggestedTactic::Assumption),
                10,
            ));
            return;
        }
    }
}

/// Check for implications P \u{2192} goal in the knowledge base
fn check_implications(goal: &ProofExpr, kb: &[ProofExpr], hints: &mut Vec<SocraticHint>) {
    for premise in kb {
        if let ProofExpr::Implies(antecedent, consequent) = premise {
            // Check if consequent matches goal
            if consequent.as_ref() == goal {
                hints.push(SocraticHint::new(
                    format!(
                        "You have an implication that concludes your goal. Can you prove its antecedent?"
                    ),
                    Some(SuggestedTactic::ModusPonens),
                    9,
                ));
            }
            // Check if antecedent is also in KB
            if consequent.as_ref() == goal && kb.iter().any(|p| p == antecedent.as_ref()) {
                hints.push(SocraticHint::new(
                    "You have both P and P \u{2192} Q where Q is your goal. Try Modus Ponens!",
                    Some(SuggestedTactic::ModusPonens),
                    10,
                ));
            }
        }
    }
}

/// Check for universal statements that could be instantiated
fn check_universals(goal: &ProofExpr, kb: &[ProofExpr], hints: &mut Vec<SocraticHint>) {
    for premise in kb {
        if let ProofExpr::ForAll { variable, body } = premise {
            // Check if goal could be an instance of the body
            if could_be_instance(goal, body) {
                hints.push(SocraticHint::new(
                    format!(
                        "You have a universal statement \u{2200}{}. What value should you substitute for {}?",
                        variable, variable
                    ),
                    Some(SuggestedTactic::UniversalElim),
                    8,
                ));
            }
        }
    }
}

/// Check for conjunction/disjunction opportunities
fn check_connectives(goal: &ProofExpr, kb: &[ProofExpr], hints: &mut Vec<SocraticHint>) {
    // Check if we have both parts of a conjunction goal
    if let ProofExpr::And(left, right) = goal {
        let have_left = kb.iter().any(|p| p == left.as_ref());
        let have_right = kb.iter().any(|p| p == right.as_ref());
        if have_left && have_right {
            hints.push(SocraticHint::new(
                "You have both parts of the conjunction in your assumptions!",
                Some(SuggestedTactic::AndIntro),
                10,
            ));
        } else if have_left {
            hints.push(SocraticHint::new(
                "You have the left part of the conjunction. Now prove the right part.",
                Some(SuggestedTactic::AndIntro),
                5,
            ));
        } else if have_right {
            hints.push(SocraticHint::new(
                "You have the right part of the conjunction. Now prove the left part.",
                Some(SuggestedTactic::AndIntro),
                5,
            ));
        }
    }

    // Check for disjunctions in premises (case analysis)
    for premise in kb {
        if let ProofExpr::Or(_, _) = premise {
            hints.push(SocraticHint::new(
                "You have a disjunction in your assumptions. Consider case analysis!",
                Some(SuggestedTactic::OrElim),
                6,
            ));
        }
    }

    // Check for conjunctions in premises (can extract parts)
    for premise in kb {
        if let ProofExpr::And(left, right) = premise {
            if left.as_ref() == goal || right.as_ref() == goal {
                hints.push(SocraticHint::new(
                    "Your goal is part of a conjunction you have. Extract it!",
                    Some(SuggestedTactic::AndElim),
                    9,
                ));
            }
        }
    }
}

/// Check for equality-related hints
fn check_equality(goal: &ProofExpr, kb: &[ProofExpr], hints: &mut Vec<SocraticHint>) {
    // Look for equations that could be used to rewrite
    for premise in kb {
        if let ProofExpr::Identity(left, right) = premise {
            // Check if either side of the equation appears in the goal
            if term_appears_in_expr(left, goal) || term_appears_in_expr(right, goal) {
                hints.push(SocraticHint::new(
                    "You have an equation that might help. Try rewriting with it.",
                    Some(SuggestedTactic::Rewrite),
                    7,
                ));
            }
        }
    }

    // Check for induction opportunities (Nat-related goals)
    if involves_nat(goal) {
        hints.push(SocraticHint::new(
            "This involves natural numbers. Have you considered induction?",
            Some(SuggestedTactic::Induction),
            6,
        ));
    }
}

/// Helper: Check if goal could be an instance of body (simple structural check)
fn could_be_instance(goal: &ProofExpr, body: &ProofExpr) -> bool {
    // Simplified check - in full implementation, would use unification
    match (goal, body) {
        (
            ProofExpr::Predicate { name: g_name, .. },
            ProofExpr::Predicate { name: b_name, .. },
        ) => g_name == b_name,
        (ProofExpr::Identity(_, _), ProofExpr::Identity(_, _)) => true,
        _ => false,
    }
}

/// Helper: Check if a term appears in an expression
fn term_appears_in_expr(term: &ProofTerm, expr: &ProofExpr) -> bool {
    match expr {
        ProofExpr::Predicate { args, .. } => args.iter().any(|a| a == term),
        ProofExpr::Identity(left, right) => left == term || right == term,
        ProofExpr::And(l, r) | ProofExpr::Or(l, r) | ProofExpr::Implies(l, r) => {
            term_appears_in_expr(term, l) || term_appears_in_expr(term, r)
        }
        ProofExpr::Not(inner) => term_appears_in_expr(term, inner),
        ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
            term_appears_in_expr(term, body)
        }
        _ => false,
    }
}

/// Helper: Check if expression involves natural numbers (Nat, Zero, Succ)
fn involves_nat(expr: &ProofExpr) -> bool {
    match expr {
        ProofExpr::Identity(left, right) => is_nat_term(left) || is_nat_term(right),
        ProofExpr::Predicate { args, .. } => args.iter().any(|a| is_nat_term(a)),
        ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => involves_nat(body),
        ProofExpr::And(l, r) | ProofExpr::Or(l, r) | ProofExpr::Implies(l, r) => {
            involves_nat(l) || involves_nat(r)
        }
        ProofExpr::Not(inner) => involves_nat(inner),
        _ => false,
    }
}

/// Helper: Check if a term looks like a Nat
fn is_nat_term(term: &ProofTerm) -> bool {
    match term {
        ProofTerm::Constant(s) => s == "Zero" || s == "Nat",
        ProofTerm::Function(name, _) => name == "Succ" || name == "add" || name == "mul",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn predicate(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
        ProofExpr::Predicate {
            name: name.to_string(),
            args,
            world: None,
        }
    }

    #[test]
    fn test_direct_match_hint() {
        let goal = predicate("Human", vec![ProofTerm::Constant("Socrates".to_string())]);
        let kb = vec![goal.clone()];

        let hint = suggest_hint(&goal, &kb, &[]);
        assert!(hint.suggested_tactic == Some(SuggestedTactic::Assumption));
    }

    #[test]
    fn test_modus_ponens_hint() {
        let p = predicate("Human", vec![ProofTerm::Constant("Socrates".to_string())]);
        let q = predicate("Mortal", vec![ProofTerm::Constant("Socrates".to_string())]);
        let implication = ProofExpr::Implies(Box::new(p.clone()), Box::new(q.clone()));

        let kb = vec![p, implication];

        let hint = suggest_hint(&q, &kb, &[]);
        assert!(hint.suggested_tactic == Some(SuggestedTactic::ModusPonens));
    }

    #[test]
    fn test_conjunction_hint() {
        let p = predicate("P", vec![]);
        let q = predicate("Q", vec![]);
        let goal = ProofExpr::And(Box::new(p.clone()), Box::new(q.clone()));

        let kb = vec![p, q];

        let hint = suggest_hint(&goal, &kb, &[]);
        assert!(hint.suggested_tactic == Some(SuggestedTactic::AndIntro));
    }

    #[test]
    fn test_reflexivity_hint() {
        let term = ProofTerm::Constant("x".to_string());
        let goal = ProofExpr::Identity(term.clone(), term);

        let hint = suggest_hint(&goal, &[], &[]);
        assert!(hint.suggested_tactic == Some(SuggestedTactic::Reflexivity));
    }
}
