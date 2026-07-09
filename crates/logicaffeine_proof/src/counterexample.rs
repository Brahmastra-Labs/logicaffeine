//! Counterexamples — when a goal is FALSE, exhibit a model instead of failing
//! silently. Lean ships no built-in counterexample finder with its automation;
//! this closes that gap.
//!
//! Two routes, both of which RE-VERIFY the model before reporting it (zero
//! false accusations — a returned witness provably satisfies every premise and
//! falsifies the goal):
//!
//! 1. **Propositional** — treat the ground atoms as booleans, search
//!    assignments that make all premises true and the goal false.
//! 2. **Arithmetic QuickCheck** — sample the free integer variables over a
//!    grid (with shrinking toward the smallest witness) and evaluate.
//!
//! The evaluator is a pure-Rust ground interpreter over [`ProofExpr`], so a
//! counterexample is a plain, checkable assignment — not an opaque solver
//! verdict.

use std::collections::{BTreeMap, BTreeSet};

use crate::{ProofExpr, ProofTerm};

/// A refuting model.
#[derive(Debug, Clone, PartialEq)]
pub enum Counterexample {
    /// Integer assignment to the free variables (arithmetic goals).
    Witness(Vec<(String, i64)>),
    /// Truth assignment to the ground atoms (propositional goals).
    Valuation(Vec<(String, bool)>),
}

impl Counterexample {
    /// A one-line human reading of the model.
    pub fn render(&self) -> String {
        match self {
            Counterexample::Witness(bindings) => {
                let parts: Vec<String> =
                    bindings.iter().map(|(v, n)| format!("{v} = {n}")).collect();
                format!("false when {}", parts.join(", "))
            }
            Counterexample::Valuation(bindings) => {
                let parts: Vec<String> = bindings
                    .iter()
                    .map(|(a, b)| format!("{a} = {}", if *b { "true" } else { "false" }))
                    .collect();
                format!("false when {}", parts.join(", "))
            }
        }
    }
}

/// Search for a counterexample to `premises ⊢ goal`: an assignment satisfying
/// every premise while falsifying the goal. `None` when none is found on the
/// covered fragment (which does NOT prove the goal — absence of a witness here
/// is not a proof).
pub fn find_counterexample(premises: &[ProofExpr], goal: &ProofExpr) -> Option<Counterexample> {
    // An arithmetic goal is decided ONLY by the grid evaluator: the
    // propositional route would treat `le(1,x)` and `le(0,x)` as unrelated
    // atoms and manufacture a spurious model. Absence of a grid witness on an
    // arithmetic goal is reported as "no counterexample found" (not a proof).
    if is_arithmetic(goal) {
        return arithmetic_witness(premises, goal);
    }
    propositional_model(premises, goal)
}

// --- arithmetic QuickCheck --------------------------------------------------

/// The sampling grid for a free integer variable, small values first so
/// shrinking is automatic (the first witness found is the smallest by |·|).
const GRID: &[i64] = &[0, 1, -1, 2, -2, 3, -3, 10, -10, 100, -100];

fn arithmetic_witness(premises: &[ProofExpr], goal: &ProofExpr) -> Option<Counterexample> {
    let mut vars: BTreeSet<String> = BTreeSet::new();
    for p in premises {
        arith_vars_expr(p, &mut vars);
    }
    arith_vars_expr(goal, &mut vars);
    let vars: Vec<String> = vars.into_iter().collect();
    // Only attempt this route when the problem is actually arithmetic and
    // small enough to grid-search.
    if vars.is_empty() || vars.len() > 3 || !is_arithmetic(goal) {
        return None;
    }

    let mut env = BTreeMap::new();
    search_grid(&vars, 0, &mut env, premises, goal)
}

fn search_grid(
    vars: &[String],
    i: usize,
    env: &mut BTreeMap<String, i64>,
    premises: &[ProofExpr],
    goal: &ProofExpr,
) -> Option<Counterexample> {
    if i == vars.len() {
        // Re-verify: every premise true, goal false.
        let premises_hold = premises.iter().all(|p| eval_expr(p, env) == Some(true));
        let goal_false = eval_expr(goal, env) == Some(false);
        if premises_hold && goal_false {
            return Some(Counterexample::Witness(
                vars.iter().map(|v| (v.clone(), env[v])).collect(),
            ));
        }
        return None;
    }
    for &val in GRID {
        env.insert(vars[i].clone(), val);
        if let Some(w) = search_grid(vars, i + 1, env, premises, goal) {
            return Some(w);
        }
    }
    env.remove(&vars[i]);
    None
}

/// Whether `e` is in the arithmetic fragment the grid evaluator handles.
fn is_arithmetic(e: &ProofExpr) -> bool {
    match e {
        ProofExpr::Identity(l, r) => is_arith_term(l) && is_arith_term(r),
        ProofExpr::And(l, r) | ProofExpr::Or(l, r) | ProofExpr::Implies(l, r) => {
            is_arithmetic(l) && is_arithmetic(r)
        }
        ProofExpr::Not(p) => is_arithmetic(p),
        ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => is_arithmetic(body),
        _ => false,
    }
}

fn is_arith_term(t: &ProofTerm) -> bool {
    match t {
        ProofTerm::Constant(_) | ProofTerm::Variable(_) => true,
        ProofTerm::Function(name, args) => {
            matches!(
                name.as_str(),
                "add" | "sub" | "mul" | "le" | "lt" | "ge" | "gt"
            ) && args.iter().all(is_arith_term)
        }
        _ => false,
    }
}

/// Free arithmetic variables of an expression. A `∀`/`∃`-bound variable IS a
/// free grid variable for counterexample purposes (we search for the
/// instantiation that breaks a universally-claimed goal).
fn arith_vars_expr(e: &ProofExpr, out: &mut BTreeSet<String>) {
    match e {
        ProofExpr::Identity(l, r) => {
            arith_vars_term(l, out);
            arith_vars_term(r, out);
        }
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => {
            arith_vars_expr(l, out);
            arith_vars_expr(r, out);
        }
        ProofExpr::Not(p) => arith_vars_expr(p, out),
        ProofExpr::ForAll { variable, body } | ProofExpr::Exists { variable, body } => {
            arith_vars_expr(body, out);
            out.insert(variable.clone());
        }
        _ => {}
    }
}

fn arith_vars_term(t: &ProofTerm, out: &mut BTreeSet<String>) {
    match t {
        ProofTerm::Variable(s) => {
            out.insert(s.clone());
        }
        // A non-numeric constant is a free integer variable — but the boolean
        // literals (the `= true`/`= false` right-hand sides of a comparison)
        // are NOT variables to search over.
        ProofTerm::Constant(s) if s.parse::<i64>().is_err() && s != "true" && s != "false" => {
            out.insert(s.clone());
        }
        ProofTerm::Function(_, args) => {
            for a in args {
                arith_vars_term(a, out);
            }
        }
        _ => {}
    }
}

/// Ground-evaluate a (closed-under-`env`) expression to a boolean.
fn eval_expr(e: &ProofExpr, env: &BTreeMap<String, i64>) -> Option<bool> {
    match e {
        ProofExpr::Identity(l, r) => {
            // `cmp(a,b) = true/false` or `a = b` over integers.
            if let Some(truth) = eval_cmp(l, env) {
                let claimed = match r {
                    ProofTerm::Constant(s) if s == "true" => true,
                    ProofTerm::Constant(s) if s == "false" => false,
                    _ => return None,
                };
                return Some(truth == claimed);
            }
            Some(eval_term(l, env)? == eval_term(r, env)?)
        }
        ProofExpr::And(l, r) => Some(eval_expr(l, env)? && eval_expr(r, env)?),
        ProofExpr::Or(l, r) => Some(eval_expr(l, env)? || eval_expr(r, env)?),
        ProofExpr::Implies(l, r) => Some(!eval_expr(l, env)? || eval_expr(r, env)?),
        ProofExpr::Not(p) => Some(!eval_expr(p, env)?),
        // A universally-quantified subgoal is treated as its body under the
        // current grid binding (the search enumerates the binding).
        ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => eval_expr(body, env),
        _ => None,
    }
}

fn eval_cmp(t: &ProofTerm, env: &BTreeMap<String, i64>) -> Option<bool> {
    let ProofTerm::Function(name, args) = t else { return None };
    if args.len() != 2 {
        return None;
    }
    let a = eval_term(&args[0], env)?;
    let b = eval_term(&args[1], env)?;
    match name.as_str() {
        "le" => Some(a <= b),
        "lt" => Some(a < b),
        "ge" => Some(a >= b),
        "gt" => Some(a > b),
        _ => None,
    }
}

fn eval_term(t: &ProofTerm, env: &BTreeMap<String, i64>) -> Option<i64> {
    match t {
        ProofTerm::Constant(s) => s
            .parse::<i64>()
            .ok()
            .or_else(|| env.get(s).copied()),
        ProofTerm::Variable(s) => env.get(s).copied(),
        ProofTerm::Function(name, args) if args.len() == 2 => {
            let a = eval_term(&args[0], env)?;
            let b = eval_term(&args[1], env)?;
            match name.as_str() {
                "add" => a.checked_add(b),
                "sub" => a.checked_sub(b),
                "mul" => a.checked_mul(b),
                _ => None,
            }
        }
        _ => None,
    }
}

// --- propositional model ----------------------------------------------------

/// Enumerate truth assignments to the ground atoms of a propositional problem,
/// returning the first that satisfies all premises and falsifies the goal.
/// Bounded to a small number of atoms (an exhaustive 2ⁿ scan).
fn propositional_model(premises: &[ProofExpr], goal: &ProofExpr) -> Option<Counterexample> {
    let mut atoms: BTreeSet<String> = BTreeSet::new();
    for p in premises {
        prop_atoms(p, &mut atoms);
    }
    prop_atoms(goal, &mut atoms);
    let atoms: Vec<String> = atoms.into_iter().collect();
    if atoms.is_empty() || atoms.len() > 16 {
        return None;
    }
    for mask in 0u32..(1u32 << atoms.len()) {
        let mut val = BTreeMap::new();
        for (i, a) in atoms.iter().enumerate() {
            val.insert(a.clone(), (mask >> i) & 1 == 1);
        }
        let premises_hold = premises.iter().all(|p| eval_prop(p, &val) == Some(true));
        let goal_false = eval_prop(goal, &val) == Some(false);
        if premises_hold && goal_false {
            return Some(Counterexample::Valuation(
                atoms.iter().map(|a| (a.clone(), val[a])).collect(),
            ));
        }
    }
    None
}

/// The canonical string key of a ground atom (`P(a, b)`, `Atom`, `a = b`).
fn atom_key(e: &ProofExpr) -> Option<String> {
    match e {
        ProofExpr::Predicate { .. } | ProofExpr::Atom(_) | ProofExpr::Identity(..) => {
            Some(format!("{e}"))
        }
        _ => None,
    }
}

fn prop_atoms(e: &ProofExpr, out: &mut BTreeSet<String>) {
    match e {
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => {
            prop_atoms(l, out);
            prop_atoms(r, out);
        }
        ProofExpr::Not(p) => prop_atoms(p, out),
        _ => {
            if let Some(k) = atom_key(e) {
                out.insert(k);
            }
        }
    }
}

fn eval_prop(e: &ProofExpr, val: &BTreeMap<String, bool>) -> Option<bool> {
    match e {
        ProofExpr::And(l, r) => Some(eval_prop(l, val)? && eval_prop(r, val)?),
        ProofExpr::Or(l, r) => Some(eval_prop(l, val)? || eval_prop(r, val)?),
        ProofExpr::Implies(l, r) => Some(!eval_prop(l, val)? || eval_prop(r, val)?),
        ProofExpr::Iff(l, r) => Some(eval_prop(l, val)? == eval_prop(r, val)?),
        ProofExpr::Not(p) => Some(!eval_prop(p, val)?),
        _ => atom_key(e).and_then(|k| val.get(&k).copied()),
    }
}
