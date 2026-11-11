//! Kernel-checked refutations: an LRAT proof turned into a development the kernel checks declaration by declaration.
//!
//! A refutation of clauses `C₁ … C_m` over variables `0..n` is checked in the context
//!
//! ```text
//! SAT.p0 … SAT.p{n−1} : Prop,   SAT.h0 : C₁, …, SAT.h{m−1} : C_m
//! ```
//!
//! where a positive literal of variable `v` is `SAT.p{v}`, a negative one `SAT.p{v} → False`, a clause the
//! right-nested `Or` of its literals, a unit clause its literal, and the empty clause `False`. [`check`] builds that
//! context from the formula alone, so a refutation cannot state its own hypotheses. Every clause the proof uses, then
//! every step, is a definition `SAT.c{id} : ¬l₁ → … → ¬l_k → False` (`¬l` the proposition that `l` fails): the
//! kernel checks its proof against that statement ([`logicaffeine_kernel::Context::declare`]), and the independent de
//! Bruijn kernel ([`logicaffeine_kernel::double_check`]) must not disagree. The last definition is the empty clause,
//! of type `False`: the hypotheses entail `False`, which no assignment of the atoms satisfying every clause allows.
//! That puts a SAT verdict on the same footing as every kernel-certified theorem, with no soundness proof of an LRAT
//! checker needed: the proof *is* the resolution derivation.
//!
//! The development is a sequence, not one closed term `∀ p. C₁ → … → C_m → False`, because the kernel refuses a term
//! nesting deeper than its stack bound: a closed term nests once per variable, clause and step, a definition only as
//! deep as its own lemma.
//!
//! An original clause becomes its lemma by `Or` elimination. An LRAT step for `C` is a λ over its literals'
//! negations; each hint is unit or the conflict under them. A unit hint with free literal `u` gives
//! `λ x : ¬u. hint … x …`, a proof of `¬¬u`. For a positive `u = p_v` that is already the negation of `¬u`; for a
//! negative `u` the negation of `¬u` is `¬p_v`, obtained from `¬¬¬p_v` by `λ q. h (λ r. r q)`. The conflict hint
//! applied to every argument is `False`. Propagated literals are bound by β-redexes, so a step's proof is linear in its
//! hints, each costing the length of the clause it names. Everything is intuitionistic: no excluded middle is used.
//!
//! [`refute`] solves the formula and refutes it from the solver's learned clauses. [`certify`] takes a refutation
//! already in hand, such as the proof [`crate::solve::certified_proof`] gives a route's verdict, so the 2-SAT and
//! Horn verdicts and the parity and GF(p) refutations compiled to resolution are kernel theorems too.

use std::collections::{HashMap, HashSet};

use logicaffeine_kernel::{double_check, prelude::StandardLibrary, DoubleCheck, Term, Universe};

use crate::cdcl::{Lit, SolveResult, Solver};
use crate::lrat::Lrat;
use crate::proof::ProofStep;

/// One definition of a refutation: a clause in lemma form and its proof.
#[derive(Clone, Debug)]
pub struct KernelLemma {
    /// `SAT.c{id}`, the clause's LRAT id.
    pub name: String,
    /// `¬l₁ → … → ¬l_k → False`; `False` for the empty clause.
    pub statement: Term,
    /// A term of that statement in the context of the hypotheses and the lemmas before it.
    pub proof: Term,
}

/// A refutation as the kernel sees it: the formula it refutes, and the development that refutes it.
#[derive(Clone, Debug)]
pub struct KernelRefutation {
    /// The atoms `SAT.p0 … SAT.p{num_vars − 1}`.
    pub num_vars: usize,
    /// The hypotheses `SAT.h{i}`, one per clause.
    pub clauses: Vec<Vec<Lit>>,
    /// The definitions, in order; the last proves `False`.
    pub lemmas: Vec<KernelLemma>,
}

/// What the kernels said about a refutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KernelCheck {
    /// The main kernel checked every lemma, and the independent kernel agreed on every one.
    TwoKernels,
    /// The main kernel checked every lemma; the independent kernel reached no verdict on at least one.
    MainKernel,
}

/// The verdict of [`refute`].
#[derive(Clone, Debug)]
pub enum KernelVerdict {
    /// Unsatisfiable, with a refutation the kernels checked.
    Refuted(KernelRefutation, KernelCheck),
    /// Satisfiable: a model of every clause.
    Satisfiable(Vec<bool>),
    /// The solver's refutation did not become a checked development; the reason.
    Unknown(String),
}

fn atom(v: u32) -> Term {
    Term::Global(format!("SAT.p{v}"))
}

fn hypothesis(i: usize) -> Term {
    Term::Global(format!("SAT.h{i}"))
}

fn lemma_name(id: u64) -> String {
    format!("SAT.c{id}")
}

fn falsum() -> Term {
    Term::Global("False".to_string())
}

fn arrow(a: Term, b: Term) -> Term {
    Term::Pi { param: "_".to_string(), param_type: Box::new(a), body_type: Box::new(b) }
}

fn lam(name: &str, ty: Term, body: Term) -> Term {
    Term::Lambda { param: name.to_string(), param_type: Box::new(ty), body: Box::new(body) }
}

fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}

/// The proposition that literal `l` holds.
fn holds(l: Lit) -> Term {
    if l.is_positive() {
        atom(l.var())
    } else {
        arrow(atom(l.var()), falsum())
    }
}

/// The proposition that literal `l` fails.
fn fails(l: Lit) -> Term {
    arrow(holds(l), falsum())
}

/// A clause as its hypothesis states it: the right-nested `Or` of its literals.
fn disjunction(clause: &[Lit]) -> Term {
    match clause {
        [] => falsum(),
        [l] => holds(*l),
        [l, rest @ ..] => app(app(Term::Global("Or".to_string()), holds(*l)), disjunction(rest)),
    }
}

/// A clause in lemma form: `¬l₁ → … → ¬l_k → False`.
fn lemma_type(clause: &[Lit]) -> Term {
    clause.iter().rev().fold(falsum(), |acc, &l| arrow(fails(l), acc))
}

/// The context a refutation of `clauses` is checked in, as `(name, type)` declarations in order: an atom
/// `SAT.p{v} : Prop` per variable, then a hypothesis `SAT.h{i}` per clause, stating it.
pub fn hypotheses(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<(String, Term)> {
    let atoms = (0..num_vars as u32).map(|v| (format!("SAT.p{v}"), Term::Sort(Universe::Prop)));
    let hyps = clauses.iter().enumerate().map(|(i, c)| (format!("SAT.h{i}"), disjunction(c)));
    atoms.chain(hyps).collect()
}

/// Original clause `i` (the hypothesis `SAT.h{i}`) in lemma form, by `Or` elimination.
fn original_lemma(i: usize, clause: &[Lit]) -> Term {
    let k = clause.len();
    if k == 0 {
        return hypothesis(i);
    }
    let neg = |j: usize| format!("o{i}_{j}");
    // `elim(t, j)`: from `t : Pos(l_j) ∨ … ∨ Pos(l_{k−1})`, with every negation bound, `False`.
    fn elim(i: usize, clause: &[Lit], t: Term, j: usize, neg: &dyn Fn(usize) -> String) -> Term {
        if j + 1 == clause.len() {
            return app(Term::Var(neg(j)), t);
        }
        let here = format!("a{i}_{j}");
        let rest = format!("r{i}_{j}");
        Term::Match {
            discriminant: Box::new(t),
            motive: Box::new(falsum()),
            cases: vec![
                lam(&here, holds(clause[j]), app(Term::Var(neg(j)), Term::Var(here.clone()))),
                lam(&rest, disjunction(&clause[j + 1..]), elim(i, clause, Term::Var(rest.clone()), j + 1, neg)),
            ],
        }
    }
    let body = elim(i, clause, hypothesis(i), 0, &neg);
    (0..k).rev().fold(body, |acc, j| lam(&neg(j), fails(clause[j]), acc))
}

/// One LRAT step in lemma form: a λ over the negations of its clause's literals whose body, through the hints in
/// order, is `False`. `None` when a hint names no clause present or is neither unit nor the conflict, or no hint
/// conflicts.
fn step_lemma(id: u64, clause: &[Lit], hints: &[u64], lemmas: &HashMap<u64, Vec<Lit>>) -> Option<Term> {
    if clause.iter().any(|l| clause.contains(&l.negated())) {
        return None;
    }
    let mut failing: HashMap<Lit, Term> = HashMap::new();
    let mut binders: Vec<(String, Lit)> = Vec::new();
    for (j, &l) in clause.iter().enumerate() {
        let name = format!("n{id}_{j}");
        failing.entry(l).or_insert_with(|| Term::Var(name.clone()));
        binders.push((name, l));
    }
    // Propagated literals, each bound to a proof that its complement fails.
    let mut lets: Vec<(String, Term, Term)> = Vec::new();
    let mut conclusion: Option<Term> = None;
    for (k, &h) in hints.iter().enumerate() {
        let lits = lemmas.get(&h)?;
        let mut open: Option<Lit> = None;
        for &d in lits {
            if failing.contains_key(&d) {
                continue;
            }
            if failing.contains_key(&d.negated()) {
                return None;
            }
            match open {
                None => open = Some(d),
                Some(u) if u == d => {}
                Some(_) => return None,
            }
        }
        let lemma = Term::Global(lemma_name(h));
        match open {
            None => {
                conclusion = Some(lits.iter().fold(lemma, |acc, d| app(acc, failing[d].clone())));
                break;
            }
            Some(u) => {
                let x = format!("x{id}_{k}");
                let core = lam(
                    &x,
                    fails(u),
                    lits.iter().fold(lemma, |acc, &d| app(acc, if d == u { Term::Var(x.clone()) } else { failing[&d].clone() })),
                );
                let value = if u.is_positive() {
                    core
                } else {
                    let q = format!("q{id}_{k}");
                    let r = format!("w{id}_{k}");
                    lam(
                        &q,
                        atom(u.var()),
                        app(core, lam(&r, holds(u), app(Term::Var(r.clone()), Term::Var(q.clone())))),
                    )
                };
                let name = format!("u{id}_{k}");
                lets.push((name.clone(), fails(u.negated()), value));
                failing.insert(u.negated(), Term::Var(name));
            }
        }
    }
    let mut body = conclusion?;
    for (name, ty, value) in lets.into_iter().rev() {
        body = app(lam(&name, ty, body), value);
    }
    Some(binders.into_iter().rev().fold(body, |acc, (name, l)| lam(&name, fails(l), acc)))
}

/// The development of the refutation `proof` of `clauses` (ids `1..=m` for the clauses, in order): a lemma for each
/// clause some step's hints name, then one per step, through the first that derives the empty clause. `None` when a
/// literal lies at or above `num_vars`, a step's id does not exceed every id before it, a step's hints do not
/// replay, or no step derives the empty clause.
pub fn lemmas(num_vars: usize, clauses: &[Vec<Lit>], proof: &Lrat) -> Option<Vec<KernelLemma>> {
    if clauses.iter().flatten().any(|l| l.var() as usize >= num_vars) {
        return None;
    }
    let named: HashSet<u64> = proof.steps.iter().flat_map(|s| s.hints.iter().copied()).collect();
    let mut known: HashMap<u64, Vec<Lit>> = HashMap::new();
    let mut out: Vec<KernelLemma> = Vec::new();
    for (i, c) in clauses.iter().enumerate() {
        let id = i as u64 + 1;
        known.insert(id, c.clone());
        if named.contains(&id) {
            out.push(KernelLemma { name: lemma_name(id), statement: lemma_type(c), proof: original_lemma(i, c) });
        }
    }
    let mut last = clauses.len() as u64;
    for step in &proof.steps {
        if step.id <= last || step.clause.iter().any(|l| l.var() as usize >= num_vars) {
            return None;
        }
        let proof = step_lemma(step.id, &step.clause, &step.hints, &known)?;
        out.push(KernelLemma { name: lemma_name(step.id), statement: lemma_type(&step.clause), proof });
        if step.clause.is_empty() {
            return Some(out);
        }
        known.insert(step.id, step.clause.clone());
        last = step.id;
    }
    None
}

/// Check a refutation: declare the atoms and hypotheses its formula gives, then each lemma in order — the main
/// kernel checks the lemma's proof against its statement and admits it, and the independent kernel, given the
/// declarations before it, must not disagree. The last lemma must state `False`, and no name may be declared twice.
/// Runs on a thread with a deep stack. `Err` carries the kernel's reason.
pub fn check(refutation: &KernelRefutation) -> Result<KernelCheck, String> {
    let r = refutation.clone();
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(move || {
            if r.clauses.iter().flatten().any(|l| l.var() as usize >= r.num_vars) {
                return Err("a clause names a variable beyond the atoms".to_string());
            }
            match r.lemmas.last() {
                Some(last) if last.statement == falsum() => {}
                _ => return Err("the last lemma does not state False".to_string()),
            }
            let mut ctx = StandardLibrary::context().clone();
            for (name, ty) in hypotheses(r.num_vars, &r.clauses) {
                if ctx.get_global(&name).is_some() {
                    return Err(format!("{name} is declared already"));
                }
                ctx.add_declaration(&name, ty);
                if ctx.get_global(&name).is_none() {
                    return Err(format!("the kernel refuses the hypothesis {name}"));
                }
            }
            let mut how = KernelCheck::TwoKernels;
            for lemma in &r.lemmas {
                if ctx.get_global(&lemma.name).is_some() {
                    return Err(format!("{} is declared already", lemma.name));
                }
                match double_check(&ctx, &lemma.proof) {
                    DoubleCheck::Agreed => {}
                    DoubleCheck::MainOnlyReCheckerIncomplete(_) => how = KernelCheck::MainKernel,
                    DoubleCheck::Disagree(why) => return Err(format!("the kernels disagree on {}: {why}", lemma.name)),
                }
                ctx.declare(&lemma.name, lemma.statement.clone(), lemma.proof.clone())
                    .map_err(|e| format!("the kernel rejects {}: {e}", lemma.name))?;
                // A definition naming a constant not yet declared is owed, not refused: it waits, invisible to lookup,
                // for that name. A refutation is checked only once every lemma is in.
                if ctx.get_global(&lemma.name).is_none() {
                    return Err(format!("{} names a constant the context does not declare", lemma.name));
                }
            }
            Ok(how)
        })
        .map_err(|e| format!("the checking thread did not start: {e}"))?
        .join()
        .unwrap_or_else(|_| Err("the kernel check panicked".to_string()))
}

/// Decide `clauses` and, when they are unsatisfiable, refute them with a kernel-checked development: solve, trim the
/// learned clauses to LRAT ([`crate::lrat::refutation`]), build the lemmas ([`lemmas`]), and [`check`] them.
pub fn refute(num_vars: usize, clauses: &[Vec<Lit>]) -> KernelVerdict {
    let mut s = Solver::new(num_vars);
    for c in clauses {
        s.add_clause(c.clone());
    }
    match s.solve() {
        SolveResult::Sat(model) => KernelVerdict::Satisfiable(model),
        SolveResult::Unsat => {
            let learned: Vec<Vec<Lit>> = s.learned().iter().map(|c| c.lits.clone()).collect();
            match checked(num_vars, clauses, &learned) {
                Ok((refutation, how)) => KernelVerdict::Refuted(refutation, how),
                Err(why) => KernelVerdict::Unknown(why),
            }
        }
    }
}

/// Kernel-check a refutation already in hand rather than solving afresh — the proof
/// [`crate::solve::certified_proof`] gives a route's verdict: its additions trimmed to LRAT
/// ([`crate::lrat::refutation`]), the lemmas built ([`lemmas`]) and [`check`]ed. The 2-SAT and Horn verdicts and the
/// parity and GF(p) refutations compiled to resolution become kernel theorems this way. As in DRAT and LRAT, the
/// empty clause need not be written: a stream after which unit propagation conflicts is closed by it. Deletions are
/// skipped. A stream with a PR step is refused: a PR clause preserves satisfiability but is not implied by the
/// formula, so no term proves it. `Err` carries the reason.
pub fn certify(num_vars: usize, clauses: &[Vec<Lit>], steps: &[ProofStep]) -> Result<(KernelRefutation, KernelCheck), String> {
    let mut additions: Vec<Vec<Lit>> = Vec::with_capacity(steps.len());
    for step in steps {
        match step {
            ProofStep::Rup(c) => additions.push(c.clone()),
            ProofStep::Delete(_) => {}
            ProofStep::Pr { .. } => {
                return Err("a PR step preserves satisfiability but is no consequence of the formula".to_string());
            }
        }
    }
    checked(num_vars, clauses, &additions)
}

/// The additions, each RUP in turn, as a kernel-checked refutation of `clauses`.
fn checked(num_vars: usize, clauses: &[Vec<Lit>], additions: &[Vec<Lit>]) -> Result<(KernelRefutation, KernelCheck), String> {
    let lrat = crate::lrat::refutation(num_vars, clauses, additions)
        .ok_or_else(|| "the additions do not trim to an LRAT refutation".to_string())?;
    let lemmas = lemmas(num_vars, clauses, &lrat).ok_or_else(|| "the LRAT refutation does not replay as lemmas".to_string())?;
    let refutation = KernelRefutation { num_vars, clauses: clauses.to_vec(), lemmas };
    let how = check(&refutation)?;
    Ok((refutation, how))
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/kernel_refutation/tests.rs"]
mod tests;
