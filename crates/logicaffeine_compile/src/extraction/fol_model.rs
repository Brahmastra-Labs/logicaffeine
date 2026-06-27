//! FOL → runnable Rust model-checker / temporal monitor.
//!
//! Compiles a first-order (optionally temporal) formula into a self-contained,
//! reusable Rust module: a `World` you populate with individuals + facts, and a
//! `holds(...)` that evaluates the rule against it — so a rule/invariant can be
//! encoded once and *used* against your own state.
//!
//! - Quantifiers: ∀→`.all`, ∃→`.any` over `World::domain`.
//! - Predicates: table lookups (`World::pred`).
//! - Events (Neo-Davidsonian, e.g. "read"): `∃e. read(e) ∧ Agent(e,x) ∧ Theme(e,y)`
//!   → an existential over the domain treating events as first-class entities.
//! - Temporal (`always`/`eventually`/`next`/`until`): a finite-trace **monitor**
//!   over `&[World]` (Always→all states, Eventually→any, …) plus an incremental
//!   `Monitor`. Pure O(trace) evaluation — no SMT, no `std::time`, no randomness,
//!   WASM-safe. (The native automaton path is `logicaffeine_verify::automata`,
//!   which carries a Z3 crate dep; this mirrors its semantics WASM-safely.)

use logicaffeine_proof::{ProofExpr, ProofTerm};
use std::collections::BTreeSet;

/// Thematic-role priority — fixes the argument order of an event relation so the
/// emitted Rust is deterministic and reads naturally (`read(Agent, Theme)`).
const ROLE_ORDER: &[&str] = &[
    "Agent", "Patient", "Theme", "Recipient", "Goal", "Source", "Instrument",
    "Location", "Time", "Manner", "Result", "Depictive",
];

/// Emit a self-contained Rust rule module for `premises ⊨ goal` (empty `premises`
/// = check the single formula `goal`). `english`/`fol` are echoed as doc comments.
pub fn fol_to_model_checker(
    premises: &[ProofExpr],
    goal: &ProofExpr,
    english: &str,
    fol: &str,
) -> String {
    fol_to_model_checker_impl(premises, goal, english, fol, true)
}

/// Like [`fol_to_model_checker`], but emits NO demo `fn main` — the form bundled
/// into an imperative program's `mod proven` (which has its own `main`). Produces
/// only the `World` + `holds` (+ `Monitor`) library items.
pub fn fol_to_model_checker_module(
    premises: &[ProofExpr],
    goal: &ProofExpr,
    english: &str,
    fol: &str,
) -> String {
    fol_to_model_checker_impl(premises, goal, english, fol, false)
}

fn fol_to_model_checker_impl(
    premises: &[ProofExpr],
    goal: &ProofExpr,
    english: &str,
    fol: &str,
    emit_main: bool,
) -> String {
    let temporal = premises.iter().any(contains_temporal) || contains_temporal(goal);

    let mut consts: BTreeSet<String> = BTreeSet::new();
    let mut facts: BTreeSet<(String, Vec<String>)> = BTreeSet::new();
    for p in premises {
        collect(p, &mut consts, &mut facts);
    }
    collect(goal, &mut consts, &mut facts);

    let header = doc_header(english, fol);
    if temporal {
        emit_temporal_module(premises, goal, &consts, &facts, &header, emit_main)
    } else {
        emit_world_module(premises, goal, &consts, &facts, &header, emit_main)
    }
}

// --- non-temporal: a single World + holds(&World) ----------------------------

fn emit_world_module(
    premises: &[ProofExpr],
    goal: &ProofExpr,
    consts: &BTreeSet<String>,
    facts: &BTreeSet<(String, Vec<String>)>,
    header: &str,
    emit_main: bool,
) -> String {
    let mut body = entailment(premises, goal, &|e| emit_expr(e, "w"));
    // Universally close any free variable over the domain so the formula is a
    // closed, evaluable Rust expression (no dangling `v_x`).
    for f in free_vars_of(premises, goal) {
        body = format!("w.domain.iter().all(|&{}| {})", sanitize_var(&f), body);
    }
    let main = if emit_main {
        format!(
            "\nfn main() {{\n    let w = {demo};\n    println!(\"holds = {{}}\", holds(&w));\n}}\n",
            demo = demo_world(consts, facts),
        )
    } else {
        String::new()
    };
    format!(
        "{header}{world}\n\
         /// Evaluate the rule against a world.\n\
         pub fn holds(w: &World) -> bool {{\n    {body}\n}}\n{main}",
        header = header,
        world = WORLD_DEF,
        body = body,
        main = main,
    )
}

// --- temporal: a trace of Worlds + holds(&[World]) + Monitor -----------------

fn emit_temporal_module(
    premises: &[ProofExpr],
    goal: &ProofExpr,
    consts: &BTreeSet<String>,
    facts: &BTreeSet<(String, Vec<String>)>,
    header: &str,
    emit_main: bool,
) -> String {
    let mut body = entailment(premises, goal, &|e| emit_expr_trace(e, "0", 0));
    // Close free variables over the first state's domain.
    for f in free_vars_of(premises, goal) {
        body = format!("trace.first().map_or(true, |w| w.domain.iter().all(|&{}| {}))", sanitize_var(&f), body);
    }
    let demo = demo_world(consts, facts);
    let main = if emit_main {
        format!(
            "fn main() {{\n    \
             let trace = vec![{demo}, {demo}];\n    \
             println!(\"holds = {{}}\", holds(&trace));\n}}\n",
            demo = demo,
        )
    } else {
        String::new()
    };
    format!(
        "{header}{world}\n\
         /// Evaluate the temporal rule over a finite trace of worlds.\n\
         pub fn holds(trace: &[World]) -> bool {{\n    {body}\n}}\n\n\
         /// Incremental monitor: feed worlds as they happen.\n\
         #[derive(Default)]\n\
         pub struct Monitor {{ trace: Vec<World> }}\n\n\
         impl Monitor {{\n    \
         pub fn new() -> Self {{ Self::default() }}\n    \
         /// Append a world and report whether the rule still holds over the trace so far.\n    \
         pub fn step(&mut self, w: World) -> bool {{ self.trace.push(w); holds(&self.trace) }}\n}}\n\n\
         {main}",
        header = header,
        world = WORLD_DEF,
        body = body,
        main = main,
    )
}

/// `premises ⊨ goal` as a Rust bool expression, given a per-formula emitter.
fn entailment(premises: &[ProofExpr], goal: &ProofExpr, emit: &dyn Fn(&ProofExpr) -> String) -> String {
    let g = emit(goal);
    if premises.is_empty() {
        g
    } else {
        let prem: Vec<String> = premises.iter().map(|p| emit(p)).collect();
        format!("(!({}) || {})", prem.join(" && "), g)
    }
}

/// The reusable `World` type + ergonomic builder + `pred` lookup.
const WORLD_DEF: &str = "\
use std::collections::HashSet;\n\n\
/// A finite world: named individuals (including events) and the facts that hold.\n\
#[derive(Default, Clone)]\n\
pub struct World {\n    \
pub domain: Vec<&'static str>,\n    \
pub facts: HashSet<(&'static str, Vec<&'static str>)>,\n}\n\n\
impl World {\n    \
pub fn new() -> Self { Self::default() }\n    \
/// Add a named individual to the domain.\n    \
pub fn individual(mut self, name: &'static str) -> Self {\n        \
if !self.domain.contains(&name) { self.domain.push(name); }\n        \
self\n    }\n    \
/// Assert `name(args...)`; any new individuals are added to the domain.\n    \
pub fn fact(mut self, name: &'static str, args: &[&'static str]) -> Self {\n        \
for &a in args { if !self.domain.contains(&a) { self.domain.push(a); } }\n        \
self.facts.insert((name, args.to_vec()));\n        \
self\n    }\n    \
/// True iff `name(args...)` holds in this world.\n    \
pub fn pred(&self, name: &'static str, args: &[&'static str]) -> bool {\n        \
self.facts.contains(&(name, args.to_vec()))\n    }\n}\n";

fn doc_header(english: &str, fol: &str) -> String {
    let one_line = |s: &str| {
        s.lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" | ")
    };
    let mut h = String::from("// Auto-generated rule from logic — a reusable World + `holds`.\n");
    let e = one_line(english);
    if !e.is_empty() {
        h.push_str(&format!("// English: {}\n", e));
    }
    let f = one_line(fol);
    if !f.is_empty() {
        h.push_str(&format!("// FOL:     {}\n", f));
    }
    h.push('\n');
    h
}

/// A demo `World` (builder chain) seeded with the formula's ground facts.
fn demo_world(consts: &BTreeSet<String>, facts: &BTreeSet<(String, Vec<String>)>) -> String {
    let mut chain = String::from("World::new()");
    for (name, args) in facts {
        let a: Vec<String> = args.iter().map(|x| format!("{:?}", x)).collect();
        chain.push_str(&format!(".fact({:?}, &[{}])", name, a.join(", ")));
    }
    // Individuals that never appear in a fact still belong to the domain.
    let in_fact: BTreeSet<&String> = facts.iter().flat_map(|(_, a)| a.iter()).collect();
    for c in consts {
        if !in_fact.contains(c) {
            chain.push_str(&format!(".individual({:?})", c));
        }
    }
    chain
}

// --- single-world FOL evaluator (predicates read from `world`) ----------------

fn emit_expr(e: &ProofExpr, world: &str) -> String {
    match e {
        ProofExpr::Predicate { name, args, .. } => {
            let a: Vec<String> = args.iter().map(emit_term).collect();
            format!("{}.pred({:?}, &[{}])", world, name, a.join(", "))
        }
        ProofExpr::Atom(s) => format!("{}.pred({:?}, &[])", world, s),
        ProofExpr::Identity(a, b) => format!("({} == {})", emit_term(a), emit_term(b)),
        ProofExpr::And(l, r) => format!("({} && {})", emit_expr(l, world), emit_expr(r, world)),
        ProofExpr::Or(l, r) => format!("({} || {})", emit_expr(l, world), emit_expr(r, world)),
        ProofExpr::Implies(l, r) => format!("(!{} || {})", emit_expr(l, world), emit_expr(r, world)),
        ProofExpr::Iff(l, r) => format!("({} == {})", emit_expr(l, world), emit_expr(r, world)),
        ProofExpr::Not(x) => format!("(!{})", emit_expr(x, world)),
        ProofExpr::ForAll { variable, body } => format!(
            "{}.domain.iter().all(|&{}| {})",
            world,
            sanitize_var(variable),
            emit_expr(body, world)
        ),
        ProofExpr::Exists { variable, body } => format!(
            "{}.domain.iter().any(|&{}| {})",
            world,
            sanitize_var(variable),
            emit_expr(body, world)
        ),
        // Neo-Davidsonian event: ∃e. verb(e) ∧ Role(e, arg) ∧ … — events are
        // domain entities, so this is an existential over the world.
        ProofExpr::NeoEvent { event_var, verb, roles } => {
            let e = sanitize_var(event_var);
            let mut sorted = roles.clone();
            sorted.sort_by_key(|(r, _)| {
                ROLE_ORDER.iter().position(|x| x == r).unwrap_or(ROLE_ORDER.len())
            });
            let mut parts = vec![format!("{}.pred({:?}, &[{}])", world, verb, e)];
            for (role, term) in &sorted {
                parts.push(format!(
                    "{}.pred({:?}, &[{}, {}])",
                    world,
                    role,
                    e,
                    emit_term(term)
                ));
            }
            format!("{}.domain.iter().any(|&{}| {})", world, e, parts.join(" && "))
        }
        other => format!("true /* unsupported: {} */", variant_name(other)),
    }
}

// --- temporal skeleton over a trace (delegates per-state to emit_expr) --------

fn emit_expr_trace(e: &ProofExpr, idx: &str, depth: usize) -> String {
    // A subformula with no temporal operator is evaluated at the current state.
    if !contains_temporal(e) {
        return emit_expr(e, &format!("(&trace[{}])", idx));
    }
    match e {
        ProofExpr::Temporal { operator, body } => {
            let v = format!("i{}", depth);
            let inner = emit_expr_trace(body, &v, depth + 1);
            match operator.as_str() {
                "Always" => format!("({idx}..trace.len()).all(|{v}| {inner})", idx = idx, v = v, inner = inner),
                "Eventually" | "Future" => {
                    format!("({idx}..trace.len()).any(|{v}| {inner})", idx = idx, v = v, inner = inner)
                }
                "Past" => format!("(0..={idx}).any(|{v}| {inner})", idx = idx, v = v, inner = inner),
                "Next" => format!(
                    "{{ let {v} = {idx} + 1; {v} < trace.len() && {inner} }}",
                    v = v,
                    idx = idx,
                    inner = inner
                ),
                _ => format!("({idx}..trace.len()).all(|{v}| {inner})", idx = idx, v = v, inner = inner),
            }
        }
        ProofExpr::TemporalBinary { operator, left, right } => {
            let k = format!("k{}", depth);
            let j = format!("j{}", depth);
            let r_at_k = emit_expr_trace(right, &k, depth + 1);
            let l_at_j = emit_expr_trace(left, &j, depth + 1);
            match operator.as_str() {
                "Release" => format!(
                    "({idx}..trace.len()).all(|{k}| {r} || ({idx}..{k}).any(|{j}| {l}))",
                    idx = idx, k = k, j = j, r = r_at_k, l = l_at_j
                ),
                "WeakUntil" => {
                    let l_all = emit_expr_trace(left, &j, depth + 1);
                    format!(
                        "(({idx}..trace.len()).all(|{j}| {l_all})) || (({idx}..trace.len()).any(|{k}| {r} && ({idx}..{k}).all(|{j}| {l})))",
                        idx = idx, k = k, j = j, l_all = l_all, r = r_at_k, l = l_at_j
                    )
                }
                // Until (and default): ∃k≥idx. right@k ∧ ∀idx≤j<k. left@j
                _ => format!(
                    "({idx}..trace.len()).any(|{k}| {r} && ({idx}..{k}).all(|{j}| {l}))",
                    idx = idx, k = k, j = j, r = r_at_k, l = l_at_j
                ),
            }
        }
        ProofExpr::And(l, r) => {
            format!("({} && {})", emit_expr_trace(l, idx, depth), emit_expr_trace(r, idx, depth))
        }
        ProofExpr::Or(l, r) => {
            format!("({} || {})", emit_expr_trace(l, idx, depth), emit_expr_trace(r, idx, depth))
        }
        ProofExpr::Implies(l, r) => {
            format!("(!{} || {})", emit_expr_trace(l, idx, depth), emit_expr_trace(r, idx, depth))
        }
        ProofExpr::Iff(l, r) => {
            format!("({} == {})", emit_expr_trace(l, idx, depth), emit_expr_trace(r, idx, depth))
        }
        ProofExpr::Not(x) => format!("(!{})", emit_expr_trace(x, idx, depth)),
        // Temporal nested deeper than the skeleton handles (e.g. under a
        // quantifier) → evaluate at the current state (deep operators degrade).
        _ => emit_expr(e, &format!("(&trace[{}])", idx)),
    }
}

/// Free variables of `premises ⊨ goal` — variables referenced but not bound by an
/// enclosing quantifier or event. Sorted for deterministic closure order.
fn free_vars_of(premises: &[ProofExpr], goal: &ProofExpr) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut bound: Vec<String> = Vec::new();
    for p in premises {
        free_vars(p, &mut bound, &mut out);
    }
    free_vars(goal, &mut bound, &mut out);
    out
}

fn free_vars(e: &ProofExpr, bound: &mut Vec<String>, out: &mut BTreeSet<String>) {
    match e {
        ProofExpr::Predicate { args, .. } => {
            for a in args {
                free_term(a, bound, out);
            }
        }
        ProofExpr::Identity(a, b) => {
            free_term(a, bound, out);
            free_term(b, bound, out);
        }
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => {
            free_vars(l, bound, out);
            free_vars(r, bound, out);
        }
        ProofExpr::Not(x) => free_vars(x, bound, out),
        ProofExpr::ForAll { variable, body } | ProofExpr::Exists { variable, body } => {
            bound.push(variable.clone());
            free_vars(body, bound, out);
            bound.pop();
        }
        ProofExpr::Temporal { body, .. } => free_vars(body, bound, out),
        ProofExpr::TemporalBinary { left, right, .. } => {
            free_vars(left, bound, out);
            free_vars(right, bound, out);
        }
        ProofExpr::NeoEvent { event_var, roles, .. } => {
            bound.push(event_var.clone());
            for (_, t) in roles {
                free_term(t, bound, out);
            }
            bound.pop();
        }
        _ => {}
    }
}

fn free_term(t: &ProofTerm, bound: &[String], out: &mut BTreeSet<String>) {
    match t {
        ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) => {
            if !bound.contains(v) {
                out.insert(v.clone());
            }
        }
        ProofTerm::Function(_, args) | ProofTerm::Group(args) => {
            for a in args {
                free_term(a, bound, out);
            }
        }
        ProofTerm::Constant(_) => {}
    }
}

fn contains_temporal(e: &ProofExpr) -> bool {
    match e {
        ProofExpr::Temporal { .. } | ProofExpr::TemporalBinary { .. } => true,
        ProofExpr::And(l, r) | ProofExpr::Or(l, r) | ProofExpr::Implies(l, r) | ProofExpr::Iff(l, r) => {
            contains_temporal(l) || contains_temporal(r)
        }
        ProofExpr::Not(x) => contains_temporal(x),
        ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => contains_temporal(body),
        _ => false,
    }
}

fn emit_term(t: &ProofTerm) -> String {
    match t {
        ProofTerm::Constant(c) => format!("{:?}", c),
        ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) => sanitize_var(v),
        ProofTerm::Function(name, _) => format!("{:?}", name),
        ProofTerm::Group(_) => "\"?\"".to_string(),
    }
}

fn sanitize_var(v: &str) -> String {
    let cleaned: String = v
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("v_{}", cleaned)
}

fn collect(e: &ProofExpr, consts: &mut BTreeSet<String>, facts: &mut BTreeSet<(String, Vec<String>)>) {
    match e {
        ProofExpr::Predicate { name, args, .. } => {
            for a in args {
                collect_term(a, consts);
            }
            let mut ground = Vec::new();
            let mut all_const = true;
            for a in args {
                match a {
                    ProofTerm::Constant(c) => ground.push(c.clone()),
                    _ => {
                        all_const = false;
                        break;
                    }
                }
            }
            if all_const {
                facts.insert((name.clone(), ground));
            }
        }
        ProofExpr::Atom(s) => {
            facts.insert((s.clone(), vec![]));
        }
        ProofExpr::Identity(a, b) => {
            collect_term(a, consts);
            collect_term(b, consts);
        }
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => {
            collect(l, consts, facts);
            collect(r, consts, facts);
        }
        ProofExpr::Not(x) => collect(x, consts, facts),
        ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => collect(body, consts, facts),
        ProofExpr::Temporal { body, .. } => collect(body, consts, facts),
        ProofExpr::TemporalBinary { left, right, .. } => {
            collect(left, consts, facts);
            collect(right, consts, facts);
        }
        ProofExpr::NeoEvent { roles, .. } => {
            for (_, term) in roles {
                collect_term(term, consts);
            }
        }
        _ => {}
    }
}

fn collect_term(t: &ProofTerm, consts: &mut BTreeSet<String>) {
    match t {
        ProofTerm::Constant(c) => {
            consts.insert(c.clone());
        }
        ProofTerm::Function(_, args) | ProofTerm::Group(args) => {
            for a in args {
                collect_term(a, consts);
            }
        }
        _ => {}
    }
}

fn variant_name(e: &ProofExpr) -> &'static str {
    match e {
        ProofExpr::Modal { .. } => "modal",
        ProofExpr::Counterfactual { .. } => "counterfactual",
        ProofExpr::Lambda { .. } => "lambda",
        ProofExpr::App(..) => "application",
        ProofExpr::Ctor { .. } => "constructor",
        ProofExpr::Match { .. } => "match",
        ProofExpr::Fixpoint { .. } => "fixpoint",
        ProofExpr::Temporal { .. } | ProofExpr::TemporalBinary { .. } => "temporal",
        _ => "construct",
    }
}
