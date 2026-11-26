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
//!
//! A first-order refutation has a third route. When the premises are a Datalog program, their least model
//! ([`least_countermodel`]) is a finite structure in which every premise is true; if the goal is false there,
//! no proof of the goal from the premises exists. [`check_countermodel`] re-verifies such a [`FiniteModel`]
//! premise by premise over its domain, with an evaluator of its own that shares nothing with the Datalog
//! engine that built the model.

use std::collections::{BTreeMap, BTreeSet, HashMap};

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
    let ProofTerm::Function(name, args) = t else {
        return None;
    };
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
        ProofTerm::Constant(s) => s.parse::<i64>().ok().or_else(|| env.get(s).copied()),
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

// --- finite countermodels ---------------------------------------------------

/// A finite first-order structure: its elements, each named by the constant that denotes it, and the atoms true
/// in it. Every other atom over the domain is false, and two names denote the same element only when they are the
/// same name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiniteModel {
    /// The elements, in order.
    pub domain: Vec<String>,
    /// The true atoms, each a predicate with its arguments.
    pub facts: BTreeSet<(String, Vec<String>)>,
}

impl FiniteModel {
    /// A one-line reading of the model's size: `24 facts over 9 elements`.
    pub fn render(&self) -> String {
        let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        format!(
            "{} over {}",
            plural(self.facts.len(), "fact", "facts"),
            plural(self.domain.len(), "element", "elements")
        )
    }
}

/// How many atoms and assignments one check may inspect before it gives up rather than run unboundedly.
const CHECK_BUDGET: u64 = 50_000_000;

/// The least model of `premises` as a countermodel to `goal`: the facts the premises derive, over the constants
/// the premises and the goal name, re-verified by [`check_countermodel`] before it is returned.
///
/// Fails when the premises are not a Datalog program (ground facts and safe, function-free Horn rules, negation
/// stratified), or when the goal holds in the least model, which then cannot refute it.
pub fn least_countermodel(premises: &[(String, ProofExpr)], goal: &ProofExpr) -> Result<FiniteModel, String> {
    use crate::datalog::{evaluate, Program};

    let program =
        Program::from_axioms(premises).map_err(|e| format!("the premises are not a Datalog program: {e}"))?;
    let least = evaluate(&program).map_err(|e| format!("the premises have no least model: {e}"))?;
    let mut domain = program.constants();
    names_of(goal, &mut Vec::new(), &mut domain);
    if domain.is_empty() {
        domain.insert("e".to_string());
    }
    let model = FiniteModel { domain: domain.into_iter().collect(), facts: least.facts().into_iter().collect() };
    if Structure::new(&model).holds(goal, &mut Vec::new())? {
        let definite = program.rules().iter().all(|r| !r.reads_closed_world());
        let follows = if definite && is_positive(goal) {
            ", and the premises are definite Horn clauses and the statement positive, so it holds in every model of \
             them: it follows"
        } else {
            ""
        };
        return Err(format!("{goal} holds in the least model of the premises, so that model does not refute it{follows}"));
    }
    check_countermodel(premises, goal, &model)?;
    Ok(model)
}

/// Re-verify that `model` refutes `goal` from `premises`: every fact names elements of the domain, every constant
/// the formulas name is an element, the goal is false in the model and every premise true. A name a formula does
/// not quantify denotes the element of that name, as it is a constant to the prover.
pub fn check_countermodel(
    premises: &[(String, ProofExpr)],
    goal: &ProofExpr,
    model: &FiniteModel,
) -> Result<(), String> {
    if model.domain.is_empty() {
        return Err("the domain is empty, and a structure has at least one element".to_string());
    }
    let domain: BTreeSet<&str> = model.domain.iter().map(String::as_str).collect();
    for (predicate, args) in &model.facts {
        if let Some(stranger) = args.iter().find(|a| !domain.contains(a.as_str())) {
            return Err(format!(
                "the fact {predicate}({}) names {stranger}, which is not in the domain",
                args.join(", ")
            ));
        }
    }
    let named = |what: &str, e: &ProofExpr| -> Result<(), String> {
        let mut names = BTreeSet::new();
        names_of(e, &mut Vec::new(), &mut names);
        match names.iter().find(|n| !domain.contains(n.as_str())) {
            Some(stranger) => Err(format!("{what} names {stranger}, which is not in the domain")),
            None => Ok(()),
        }
    };
    named("the refuted statement", goal)?;
    for (name, premise) in premises {
        named(&format!("the premise `{name}`"), premise)?;
    }

    let structure = Structure::new(model);
    if structure.holds(goal, &mut Vec::new())? {
        return Err(format!("the refuted statement holds in the model: {goal}"));
    }
    for (name, premise) in premises {
        if let Some(instance) = structure.counter_instance(premise)? {
            let at = if instance.is_empty() {
                String::new()
            } else {
                let parts: Vec<String> = instance.iter().map(|(v, e)| format!("{v} = {e}")).collect();
                format!(" at {}", parts.join(", "))
            };
            return Err(format!("the premise `{name}` is false in the model{at}"));
        }
    }
    Ok(())
}

/// Whether truth of `e` is preserved from a structure to every structure it maps into: atoms, equations, and their
/// conjunctions, disjunctions and existentials.
fn is_positive(e: &ProofExpr) -> bool {
    match e {
        ProofExpr::Predicate { world: None, .. } | ProofExpr::Identity(..) => true,
        ProofExpr::Atom(a) => a != "False",
        ProofExpr::And(l, r) | ProofExpr::Or(l, r) => is_positive(l) && is_positive(r),
        ProofExpr::Exists { body, .. } => is_positive(body),
        _ => false,
    }
}

/// The names `e` uses as constants: every `Constant` term and every variable no enclosing quantifier binds.
fn names_of(e: &ProofExpr, bound: &mut Vec<String>, out: &mut BTreeSet<String>) {
    fn term(t: &ProofTerm, bound: &[String], out: &mut BTreeSet<String>) {
        match t {
            ProofTerm::Constant(c) => {
                out.insert(c.clone());
            }
            ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) => {
                if !bound.contains(v) {
                    out.insert(v.clone());
                }
            }
            ProofTerm::Function(_, args) | ProofTerm::Group(args) => {
                for a in args {
                    term(a, bound, out);
                }
            }
        }
    }
    match e {
        ProofExpr::Predicate { args, .. } => {
            for a in args {
                term(a, bound, out);
            }
        }
        ProofExpr::Identity(l, r) => {
            term(l, bound, out);
            term(r, bound, out);
        }
        ProofExpr::And(l, r) | ProofExpr::Or(l, r) | ProofExpr::Implies(l, r) | ProofExpr::Iff(l, r) => {
            names_of(l, bound, out);
            names_of(r, bound, out);
        }
        ProofExpr::Not(p) => names_of(p, bound, out),
        ProofExpr::ForAll { variable, body } | ProofExpr::Exists { variable, body } => {
            bound.push(variable.clone());
            names_of(body, bound, out);
            bound.pop();
        }
        _ => {}
    }
}

/// The bindings of the quantified names in scope, innermost last.
type Env = Vec<(String, String)>;

/// A [`FiniteModel`] read for evaluation: its facts grouped by predicate.
struct Structure<'m> {
    model: &'m FiniteModel,
    by_predicate: HashMap<&'m str, Vec<&'m [String]>>,
    steps: std::cell::Cell<u64>,
}

impl<'m> Structure<'m> {
    fn new(model: &'m FiniteModel) -> Self {
        let mut by_predicate: HashMap<&str, Vec<&[String]>> = HashMap::new();
        for (p, args) in &model.facts {
            by_predicate.entry(p.as_str()).or_default().push(args.as_slice());
        }
        Structure { model, by_predicate, steps: std::cell::Cell::new(0) }
    }

    fn step(&self) -> Result<(), String> {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        if n > CHECK_BUDGET {
            return Err(format!("the check gave up after {CHECK_BUDGET} steps"));
        }
        Ok(())
    }

    /// The element `t` denotes: a quantified name's binding, or the element a constant names.
    fn value<'e>(&self, t: &'e ProofTerm, env: &'e Env) -> Result<&'e str, String> {
        match t {
            ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) => {
                Ok(env.iter().rev().find(|(name, _)| name == v).map_or(v.as_str(), |(_, e)| e.as_str()))
            }
            ProofTerm::Constant(c) => Ok(c.as_str()),
            ProofTerm::Function(f, _) => {
                Err(format!("the function symbol `{f}` has no interpretation in a model of facts"))
            }
            ProofTerm::Group(_) => Err("a tuple term has no interpretation in a model of facts".to_string()),
        }
    }

    fn holds(&self, e: &ProofExpr, env: &mut Env) -> Result<bool, String> {
        self.step()?;
        Ok(match e {
            ProofExpr::Predicate { name, args, world: None } => {
                let tuple = args
                    .iter()
                    .map(|a| self.value(a, env).map(str::to_string))
                    .collect::<Result<Vec<String>, String>>()?;
                self.model.facts.contains(&(name.clone(), tuple))
            }
            ProofExpr::Predicate { world: Some(w), .. } => {
                return Err(format!("an atom at world `{w}` has no reading in a model of facts"))
            }
            ProofExpr::Atom(a) if a == "True" => true,
            ProofExpr::Atom(a) if a == "False" => false,
            ProofExpr::Atom(a) => self.model.facts.contains(&(a.clone(), Vec::new())),
            ProofExpr::Identity(l, r) => self.value(l, env)? == self.value(r, env)?,
            ProofExpr::And(l, r) => self.holds(l, env)? && self.holds(r, env)?,
            ProofExpr::Or(l, r) => self.holds(l, env)? || self.holds(r, env)?,
            ProofExpr::Implies(l, r) => !self.holds(l, env)? || self.holds(r, env)?,
            ProofExpr::Iff(l, r) => self.holds(l, env)? == self.holds(r, env)?,
            ProofExpr::Not(p) => !self.holds(p, env)?,
            ProofExpr::ForAll { .. } => {
                let mark = env.len();
                let found = self.counter_instance_in(e, env)?;
                env.truncate(mark);
                found.is_none()
            }
            ProofExpr::Exists { .. } => {
                let (names, body) = block(e, false);
                let mut guards = Vec::new();
                conjuncts(body, &mut guards);
                let mark = env.len();
                let found = self.assign(names, &guards, &|_, _| Ok(true), env)?;
                env.truncate(mark);
                found
            }
            other => return Err(format!("{other} has no reading in a model of facts")),
        })
    }

    /// An assignment to the leading universal names of `e` under which `e` is false, if there is one; for a
    /// formula with no leading universal, the empty assignment when it is false.
    fn counter_instance(&self, e: &ProofExpr) -> Result<Option<Vec<(String, String)>>, String> {
        self.counter_instance_in(e, &mut Vec::new())
    }

    fn counter_instance_in(&self, e: &ProofExpr, env: &mut Env) -> Result<Option<Vec<(String, String)>>, String> {
        let (names, body) = block(e, true);
        let (guards, refuted) = match body {
            ProofExpr::Implies(condition, conclusion) => {
                let mut guards = Vec::new();
                conjuncts(condition, &mut guards);
                (guards, conclusion.as_ref())
            }
            other => (Vec::new(), other),
        };
        let mark = env.len();
        if self.assign(names, &guards, &|s, env| Ok(!s.holds(refuted, env)?), env)? {
            Ok(Some(env[mark..].to_vec()))
        } else {
            Ok(None)
        }
    }

    /// Search for values of `names` under which every guard holds and `want` answers yes, leaving them bound in
    /// `env` when one is found. A guard that is an atom ranges its unbound names over the facts of its predicate; a
    /// name no such atom binds ranges over the domain.
    fn assign(
        &self,
        names: Vec<&str>,
        guards: &[&ProofExpr],
        want: &dyn Fn(&Self, &mut Env) -> Result<bool, String>,
        env: &mut Env,
    ) -> Result<bool, String> {
        let atom = guards.iter().position(|g| matches!(g, ProofExpr::Predicate { world: None, args, .. } if args.iter().all(is_name)));
        if let Some(i) = atom {
            let ProofExpr::Predicate { name, args, .. } = guards[i] else { unreachable!("the guard is an atom") };
            let rest: Vec<&ProofExpr> = guards.iter().enumerate().filter(|&(j, _)| j != i).map(|(_, g)| *g).collect();
            for tuple in self.by_predicate.get(name.as_str()).map(Vec::as_slice).unwrap_or_default() {
                self.step()?;
                if tuple.len() != args.len() {
                    continue;
                }
                let mark = env.len();
                let mut open = names.clone();
                if self.matches(args, tuple, &mut open, env)? && self.assign(open, &rest, want, env)? {
                    return Ok(true);
                }
                env.truncate(mark);
            }
            return Ok(false);
        }
        if let Some((first, others)) = names.split_first() {
            for element in &self.model.domain {
                self.step()?;
                env.push((first.to_string(), element.clone()));
                if self.assign(others.to_vec(), guards, want, env)? {
                    return Ok(true);
                }
                env.pop();
            }
            return Ok(false);
        }
        for g in guards {
            if !self.holds(g, env)? {
                return Ok(false);
            }
        }
        want(self, env)
    }

    /// Whether the atom's arguments `args` can denote `tuple`, binding each still-open quantified name they use.
    fn matches(&self, args: &[ProofTerm], tuple: &[String], open: &mut Vec<&str>, env: &mut Env) -> Result<bool, String> {
        for (arg, element) in args.iter().zip(tuple) {
            if let ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) = arg {
                if let Some(k) = open.iter().position(|n| *n == v.as_str()) {
                    open.remove(k);
                    env.push((v.clone(), element.clone()));
                    continue;
                }
            }
            if self.value(arg, env)? != element.as_str() {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// Whether a term is a name: a constant or a variable.
fn is_name(t: &ProofTerm) -> bool {
    matches!(t, ProofTerm::Constant(_) | ProofTerm::Variable(_) | ProofTerm::BoundVarRef(_))
}

/// The names a run of like quantifiers at the head of `e` binds, outermost first, and the formula under them.
fn block(e: &ProofExpr, universal: bool) -> (Vec<&str>, &ProofExpr) {
    let mut names = Vec::new();
    let mut body = e;
    loop {
        match body {
            ProofExpr::ForAll { variable, body: inner } if universal => {
                names.push(variable.as_str());
                body = inner;
            }
            ProofExpr::Exists { variable, body: inner } if !universal => {
                names.push(variable.as_str());
                body = inner;
            }
            _ => return (names, body),
        }
    }
}

/// The conjuncts of `e`.
fn conjuncts<'e>(e: &'e ProofExpr, out: &mut Vec<&'e ProofExpr>) {
    match e {
        ProofExpr::And(l, r) => {
            conjuncts(l, out);
            conjuncts(r, out);
        }
        other => out.push(other),
    }
}
