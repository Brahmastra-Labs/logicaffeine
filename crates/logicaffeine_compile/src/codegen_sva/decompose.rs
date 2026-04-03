//! Hierarchical Spec Decomposition
//!
//! Decomposes complex properties into independently verifiable sub-properties.
//! Conjunction splitting: And(P, Q) → [P, Q]
//!
//! Temporal-aware: when G-unrolling produces And(P@0, And(P@1, P@2)),
//! these are temporal copies of the same property. decompose_conjunctive
//! groups by structural pattern and returns one full temporal copy per
//! unique structural pattern, preserving reconstruction soundness.

use logicaffeine_verify::ir::VerifyExpr;

/// Decompose a conjunctive property into independently verifiable sub-properties.
///
/// Flattens nested And trees: And(And(P, Q), R) → [P, Q, R]
/// Groups by structural pattern (modulo timestep indices) and returns
/// the conjunction of all temporal copies for each unique pattern.
/// This preserves the G-wrapping semantics while splitting structural conjunction.
pub fn decompose_conjunctive(expr: &VerifyExpr) -> Vec<VerifyExpr> {
    let mut parts = Vec::new();
    flatten_and(expr, &mut parts);
    if parts.is_empty() {
        return vec![expr.clone()];
    }

    // Group parts by structural pattern (replacing @N with @T for comparison)
    let mut groups: Vec<(String, Vec<VerifyExpr>)> = Vec::new();
    for part in parts {
        let pattern = normalize_timesteps(&part);
        if let Some(group) = groups.iter_mut().find(|(p, _)| *p == pattern) {
            group.1.push(part);
        } else {
            groups.push((pattern, vec![part]));
        }
    }

    // For each unique pattern, reconstruct the conjunction of all its temporal copies
    groups.into_iter().map(|(_, members)| {
        let mut result = members[0].clone();
        for member in &members[1..] {
            result = VerifyExpr::and(result, member.clone());
        }
        result
    }).collect()
}

/// Verify that decomposition is sound: conjunction of parts is equivalent to original.
/// Uses Z3 equivalence checking to prove the decomposition preserves semantics.
///
/// Returns true if Z3 proves equivalence at the given bound.
/// Returns false if Z3 finds a counterexample or returns unknown.
pub fn verify_decomposition_sound(
    original: &VerifyExpr,
    parts: &[VerifyExpr],
    bound: u32,
) -> bool {
    use logicaffeine_verify::equivalence::{check_equivalence, EquivalenceResult};
    use std::collections::HashSet;

    if parts.is_empty() {
        return false;
    }

    let mut reconstructed = parts[0].clone();
    for part in &parts[1..] {
        reconstructed = VerifyExpr::and(reconstructed, part.clone());
    }

    let mut vars = HashSet::new();
    logicaffeine_verify::equivalence::collect_vars_pub(original, &mut vars);
    logicaffeine_verify::equivalence::collect_vars_pub(&reconstructed, &mut vars);
    let signals: Vec<String> = vars
        .iter()
        .filter_map(|v| v.find('@').map(|pos| v[..pos].to_string()))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    matches!(
        check_equivalence(original, &reconstructed, &signals, bound as usize),
        EquivalenceResult::Equivalent
    )
}

fn flatten_and(expr: &VerifyExpr, parts: &mut Vec<VerifyExpr>) {
    match expr {
        VerifyExpr::Binary { op: logicaffeine_verify::ir::VerifyOp::And, left, right } => {
            flatten_and(left, parts);
            flatten_and(right, parts);
        }
        _ => {
            parts.push(expr.clone());
        }
    }
}

/// Normalize timestep indices in a VerifyExpr's debug representation.
fn normalize_timesteps(expr: &VerifyExpr) -> String {
    let debug = format!("{:?}", expr);
    let mut result = String::with_capacity(debug.len());
    let mut chars = debug.chars().peekable();
    while let Some(c) = chars.next() {
        result.push(c);
        if c == '@' {
            while chars.peek().map_or(false, |ch| ch.is_ascii_digit()) {
                chars.next();
            }
            result.push('T');
        }
    }
    result
}
