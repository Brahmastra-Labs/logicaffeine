//! The proof for one function: its source and a compiled form of it (the VM's bytecode, the emitted Rust) paired at
//! their loop heads, an invariant relating their states at each pair of heads inferred by Houdini, and the
//! obligations every pair of paths between heads must meet.
//!
//! When the two loop nests have the same shape, loop `k` of one is paired with loop `k` of the other. When they do not
//! (the Rust emitter versions a loop on a guard, so one source loop is two Rust loops), the pairs of heads are the
//! ones the two sides reach together, found from the entry outward. At each pair of heads the candidates are every
//! equality between a target location and a source variable of the same kind, every constant a location held at any
//! head, and a lower bound on each integer variable (zero, one, and each constant it held). Houdini drops each
//! candidate some pair of paths fails to re-establish, assuming the candidates still standing at the pair it started
//! from, until none is dropped; what remains is inductive. Then, from the entry and from every pair of heads under its
//! invariant, every pair of paths whose conditions can hold together must end alike — at a pair of heads, returning
//! the same value, raising, or halting — having shown the same values. When one side reaches a head and the other
//! leaves the loop (the VM leaves a counted range before its last increment; the source tests its counter once more),
//! the side at the head is followed one segment further before the ends are compared.
//!
//! A target that stops with a loud integer overflow (the emitted Rust, where a value needs more than 64 bits) meets
//! its obligation when what it showed before stopping is what the source showed: the proof is then that it never
//! computes a wrong answer, and the verdict says the target may stop where the source goes on.
//!
//! A target with no loop where the source has some computes them in closed form; its proof is `closed`'s.

mod closed;
mod divides;
mod relation;
mod sample;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;

use z3::ast::{Ast, Bool, Dynamic, Int};
use z3::{Params, SatResult, Solver, Tactic};

use super::path::{fresh, End, LoopNode, Path, State};
use sample::{entry_terms, input_atoms, render, Fact, Samples};
use super::value::{in_i64, same, Kind, Lit, SymVal};
use crate::symexec::Unsupported;

/// How much work Z3 may do on one query, in its resource units (about a million a second on a desktop core) rather
/// than in time, so that a verdict does not depend on how loaded the machine is: the incremental engine a little, and
/// a fresh solver, when that one gives up, the rest.
const INCREMENTAL_RLIMIT: u32 = 2_000_000;
const HOUDINI_RLIMIT: u32 = 8_000_000;
const QUERY_RLIMIT: u32 = 40_000_000;

/// The wall-clock time after which a query is abandoned as unknown. Not a budget: the resource limit bounds the
/// solver's search, but Z3 simplifies some terms (a remainder by 2⁶⁴ of a nonlinear product) without counting the work,
/// and a query could otherwise run for minutes on that alone.
const HANG_MS: u32 = 60_000;

/// A solver's parameters: `rlimit` resource units of work, and the hang guard.
fn budget(rlimit: u32) -> Params {
    let mut params = Params::new();
    params.set_u32("rlimit", rlimit);
    params.set_u32("timeout", HANG_MS);
    params
}

/// How many segments past a loop head one side may be followed to meet the other.
const EXTEND: usize = 2;

/// One side of the proof: one function's runs from its entry and from each of its loop heads.
pub trait Side {
    type Loc: Ord + Clone + Debug;
    fn loops(&self) -> &[LoopNode];
    /// The state at the entry, the parameters bound to `params`.
    fn entry(&self, params: &[SymVal]) -> State<Self::Loc>;
    /// Every path from the entry (`None`) or from the head of loop `k` with `state`, to the next head or the end.
    fn explore(&self, from: Option<usize>, state: State<Self::Loc>) -> Result<Vec<Path<Self::Loc>>, Unsupported>;
    /// The name a location goes by in an invariant or a counterexample.
    fn describe(&self, l: &Self::Loc) -> String;
    /// What every input this side can be called with satisfies: a machine word's parameter holds a value in `i64`,
    /// since a caller holding a larger one stops before the call. Every query assumes it.
    fn assumes(&self, _params: &[SymVal]) -> Bool {
        Bool::from_bool(true)
    }
}

/// What the proof found for one function, before a failure is replayed on concrete inputs.
#[derive(Clone, Debug)]
pub enum Finding {
    /// Every obligation holds: the invariant per pair of heads, the callees taken as uninterpreted functions, and
    /// whether the target may stop with a loud integer overflow where the source goes on.
    Proven { invariants: Vec<Vec<String>>, assumed: Vec<String>, overflow: bool },
    /// An obligation fails on a state Z3 found — where, that state, and every integer in Z3's model (the inputs a
    /// replay tries first).
    Failed { at: String, state: String, values: Vec<String> },
    /// Z3 gave no answer in time.
    Unknown { reason: String },
}

/// Which kind a loc holds at a head, joined over every path reaching it: one kind, or several (`Top`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    One(Kind),
    Top,
}

/// What the paths reaching a head say about its state: the kind each loc holds on all of them, every constant a loc
/// held on any of them (each a candidate Houdini keeps only if it is inductive), and how many iterators are open.
#[derive(Clone, Debug, PartialEq)]
struct Profile<L: Ord> {
    shapes: BTreeMap<L, Shape>,
    consts: BTreeMap<L, BTreeSet<Lit>>,
    depth: Option<u8>,
}

impl<L: Ord + Clone> Profile<L> {
    fn of(state: &State<L>) -> Self {
        Profile {
            shapes: state.env.iter().map(|(l, v)| (l.clone(), Shape::One(v.kind()))).collect(),
            consts: state
                .env
                .iter()
                .filter_map(|(l, v)| v.constant().map(|c| (l.clone(), BTreeSet::from([c]))))
                .collect(),
            depth: Some(state.depth),
        }
    }

    fn join(&self, other: &Self) -> Self {
        let mut shapes = BTreeMap::new();
        for l in self.shapes.keys().chain(other.shapes.keys()) {
            let shape = match (self.shapes.get(l), other.shapes.get(l)) {
                (Some(a), Some(b)) if a == b => *a,
                _ => Shape::Top,
            };
            shapes.insert(l.clone(), shape);
        }
        let mut consts = self.consts.clone();
        for (l, cs) in &other.consts {
            consts.entry(l.clone()).or_default().extend(cs.iter().copied());
        }
        let depth = if self.depth == other.depth { self.depth } else { None };
        Profile { shapes, consts, depth }
    }

    fn havoc(&self, label: impl Fn(&L) -> String) -> Result<State<L>, Unsupported> {
        let depth = self.depth.ok_or_else(|| Unsupported("a loop head reached with different iterators open".into()))?;
        let env = self
            .shapes
            .iter()
            .map(|(l, s)| {
                let v = match s {
                    Shape::One(k) => fresh(*k, &label(l)),
                    Shape::Top => SymVal::Clobbered,
                };
                (l.clone(), v)
            })
            .collect();
        Ok(State { env, depth })
    }
}

/// A conjunct the invariant at a pair of heads may keep: a relation between the two sides, a constant, what the
/// source's sampled runs say of its head (`sample`), or a comparison over the inputs some path tested.
#[derive(Clone, Debug, PartialEq)]
enum Cand<S, T> {
    Same(T, S),
    Target(T, Lit),
    Source(S, Lit),
    /// A location holds the term over the inputs it reached the head with from the entry ([`entry_terms`]).
    TargetEntry(T, Int),
    SourceEntry(S, Int),
    AtLeast(S, i64),
    Sampled(Fact<S>),
    Guard(Bool),
}

impl<S: Ord + Clone, T: Ord> Cand<S, T> {
    fn holds(&self, t: &State<T>, s: &State<S>, inputs: &[SymVal]) -> Bool {
        let pick = |v: Option<&SymVal>| v.cloned().unwrap_or(SymVal::Clobbered);
        match self {
            Cand::Same(r, v) => same(&pick(t.env.get(r)), &pick(s.env.get(v))),
            Cand::Target(r, lit) => same(&pick(t.env.get(r)), &lit.value()),
            Cand::Source(v, lit) => same(&pick(s.env.get(v)), &lit.value()),
            Cand::TargetEntry(r, x) => same(&pick(t.env.get(r)), &SymVal::Int(x.clone())),
            Cand::SourceEntry(v, x) => same(&pick(s.env.get(v)), &SymVal::Int(x.clone())),
            Cand::AtLeast(v, n) => match s.env.get(v) {
                Some(SymVal::Int(x)) => x.ge(z3::ast::Int::from_i64(*n)),
                _ => Bool::from_bool(false),
            },
            Cand::Sampled(f) => f.holds(s, inputs),
            Cand::Guard(g) => g.clone(),
        }
    }
}

fn lit_text(l: &Lit) -> String {
    match l {
        Lit::Int(n) => n.to_string(),
        Lit::Bool(b) => b.to_string(),
        Lit::Nothing => "nothing".into(),
    }
}

fn kindly(s: &Shape) -> Option<Kind> {
    match s {
        Shape::One(k @ (Kind::Int | Kind::Bool | Kind::Range)) => Some(*k),
        _ => None,
    }
}

/// The candidates at one pair of heads. `seen_s` and `seen_t` are the constants each loc held at any head: a loop
/// reached only from inside another sees its constants only as the outer head's havoc values, so a constant is offered
/// wherever its loc lives and kept only where it is inductive.
fn candidates<S: Ord + Clone, T: Ord + Clone>(
    sp: &Profile<S>,
    tp: &Profile<T>,
    seen_s: &BTreeMap<S, BTreeSet<Lit>>,
    seen_t: &BTreeMap<T, BTreeSet<Lit>>,
) -> Vec<Cand<S, T>> {
    let mut out = Vec::new();
    for (r, rs) in &tp.shapes {
        let Some(rk) = kindly(rs) else { continue };
        for (v, vs) in &sp.shapes {
            if kindly(vs) == Some(rk) {
                out.push(Cand::Same(r.clone(), v.clone()));
            }
        }
        for lit in seen_t.get(r).into_iter().flatten() {
            if matches!(lit, Lit::Int(_) | Lit::Bool(_)) {
                out.push(Cand::Target(r.clone(), *lit));
            }
        }
    }
    for (v, vs) in &sp.shapes {
        if kindly(vs).is_none() {
            continue;
        }
        let lits = seen_s.get(v).into_iter().flatten();
        for lit in lits.clone() {
            if matches!(lit, Lit::Int(_) | Lit::Bool(_)) {
                out.push(Cand::Source(v.clone(), *lit));
            }
        }
        if *vs == Shape::One(Kind::Int) {
            let mut bounds: BTreeSet<i64> = BTreeSet::from([0, 1]);
            bounds.extend(lits.filter_map(|l| match l {
                Lit::Int(n) => Some(*n),
                _ => None,
            }));
            out.extend(bounds.into_iter().map(|n| Cand::AtLeast(v.clone(), n)));
        }
    }
    out
}

fn cut(end: &End) -> Option<usize> {
    match end {
        End::Cut(k) => Some(*k),
        _ => None,
    }
}

fn describe_end(e: &End) -> String {
    match e {
        End::Cut(k) => format!("reaches the head of loop {}", k + 1),
        End::Return(v) => format!("returns a {:?}", v.kind()),
        End::Raise => "raises".into(),
        End::Overflow => "stops with an integer overflow".into(),
        End::Halt => "finishes".into(),
    }
}

/// The two sides of one function. `what` names the target in a failure ("the bytecode", "the Rust").
pub struct Prover<'a, S: Side, T: Side> {
    pub source: &'a S,
    pub target: &'a T,
    pub what: &'static str,
}

/// One segment's paths, paired.
type Pairs<S, T> = Vec<(Path<S>, Path<T>)>;

/// The pairs of heads the proof reasons at: segment 0 is the entry, segment `i + 1` starts at `pairs[i]`, a pair
/// `(source head, target head)`. With loop nests of one shape the pairs are `(k, k)`; otherwise they are the pairs the
/// two sides reach together.
struct Joints {
    strict: bool,
    pairs: Vec<(usize, usize)>,
}

impl Joints {
    fn alike(&self, a: &End, b: &End) -> bool {
        match (a, b) {
            (End::Cut(x), End::Cut(y)) => !self.strict || x == y,
            (End::Return(_), End::Return(_)) | (End::Raise, End::Raise) | (End::Halt, End::Halt) => true,
            _ => false,
        }
    }

    fn index(&self, a: usize, b: usize) -> Option<usize> {
        self.pairs.iter().position(|&p| p == (a, b))
    }
}

/// A query decided: the answer, and the fresh solver that gave it when the incremental one gave up. Z3's incremental
/// engine abandons some nonlinear queries as incomplete that the same SMT core decides in milliseconds when it is given
/// the assertions at once, after simplification and the elimination of solved equations. A model is read from the
/// solver that decided.
struct Decided {
    answer: SatResult,
    fresh: Option<Solver>,
}

impl Decided {
    fn by<'s>(&'s self, solver: &'s Solver) -> &'s Solver {
        self.fresh.as_ref().unwrap_or(solver)
    }
}

/// A non-incremental solver: simplification, the elimination of solved equations, then the SMT core. Built from those
/// tactics rather than Z3's default for the logic, whose nonlinear integer tactic (`qfnia`) ignores the resource limit
/// and runs for minutes on a goal the SMT core decides in seconds.
fn fresh_solver(rlimit: u32) -> Solver {
    let tactic = Tactic::new("simplify").and_then(&Tactic::new("solve-eqs")).and_then(&Tactic::new("smt"));
    let solver = tactic.solver();
    solver.set_params(&budget(rlimit));
    solver
}

/// `term` with every wrap the solver's assertions rule out replaced by the exact value. A machine word's wrap is
/// `ite(in_i64(x), x, (x + 2⁶³) mod 2⁶⁴ − 2⁶³)` (`value::wrap64`); where the path's guard keeps `x` in `i64`, it is `x`,
/// and saying so spares Z3 the remainder of a nonlinear product, which it can spend minutes simplifying even when the
/// branch is dead. Innermost first, each replacement checked by the incremental engine; an undecided one is kept.
fn discharge_wraps(solver: &Solver, term: &Bool) -> Bool {
    let mut term = term.clone();
    loop {
        let mut wraps: Vec<(Dynamic, Int)> = Vec::new();
        let mut seen = BTreeSet::new();
        let mut work = vec![Dynamic::from_ast(&term)];
        while let Some(t) = work.pop() {
            if !t.is_app() || !seen.insert(t.to_string()) {
                continue;
            }
            let kids = t.children();
            if let [c, x, _] = kids.as_slice() {
                if t.decl().name() == "if" {
                    if let (Some(c), Some(x)) = (c.as_bool(), x.as_int()) {
                        if c == in_i64(&x) {
                            wraps.push((t.clone(), x));
                        }
                    }
                }
            }
            work.extend(kids);
        }
        wraps.sort_by_key(|(t, _)| t.to_string().len());
        let Some((wrap, exact)) = wraps
            .into_iter()
            .find(|(_, x)| solver.check_assumptions(&[in_i64(x).not()]) == SatResult::Unsat)
        else {
            return term;
        };
        term = term.substitute(&[(&wrap, &Dynamic::from_ast(&exact))]);
    }
}

/// No product of two terms that are not numerals: a linear term.
fn is_linear(t: &Dynamic) -> bool {
    let kids = t.children();
    let variable = |k: &Dynamic| !(k.children().is_empty() && super::value::numeral(&k.to_string()).is_some());
    (t.decl().name() != "*" || kids.iter().filter(|k| variable(k)).count() <= 1) && kids.iter().all(is_linear)
}

/// `solver`'s assertions and `extra`, decided: incrementally first, and afresh with `QUERY_RLIMIT` when that gives up.
fn decide(solver: &Solver, extra: &[Bool]) -> Decided {
    decide_within(solver, extra, QUERY_RLIMIT)
}

/// As [`decide`], a fresh solver given `budget`. Houdini asks with less: a candidate dropped on an unknown answer is
/// dropped soundly, and only weakens the invariant.
fn decide_within(solver: &Solver, extra: &[Bool], rlimit: u32) -> Decided {
    let answer = solver.check_assumptions(extra);
    if answer != SatResult::Unknown {
        return Decided { answer, fresh: None };
    }
    decide_fresh(solver, extra, rlimit)
}

/// `solver`'s assertions and `extra`, decided by a fresh solver given `rlimit`.
fn decide_fresh(solver: &Solver, extra: &[Bool], rlimit: u32) -> Decided {
    let fresh = fresh_solver(rlimit);
    for a in solver.get_assertions() {
        fresh.assert(&a);
    }
    for e in extra {
        fresh.assert(e);
    }
    Decided { answer: fresh.check(), fresh: Some(fresh) }
}

impl<'a, S: Side, T: Side> Prover<'a, S, T> {
    fn absorb<L: Ord + Clone>(paths: &[Path<L>], into: &mut [Option<Profile<L>>]) -> bool {
        let mut changed = false;
        for p in paths {
            if let End::Cut(k) = p.end {
                let seen = Profile::of(&p.state);
                let next = match &into[k] {
                    Some(prev) => prev.join(&seen),
                    None => seen,
                };
                if into[k].as_ref() != Some(&next) {
                    into[k] = Some(next);
                    changed = true;
                }
            }
        }
        changed
    }

    /// Every constant each loc held at any head.
    fn constants<L: Ord + Clone>(profiles: &[Option<Profile<L>>]) -> BTreeMap<L, BTreeSet<Lit>> {
        let mut out: BTreeMap<L, BTreeSet<Lit>> = BTreeMap::new();
        for p in profiles.iter().flatten() {
            for (l, cs) in &p.consts {
                out.entry(l.clone()).or_default().extend(cs.iter().copied());
            }
        }
        out
    }

    fn extend<X: Side>(side: &X, p: &Path<X::Loc>, k: usize) -> Result<Vec<Path<X::Loc>>, Unsupported> {
        Ok(side
            .explore(Some(k), p.state.clone())?
            .into_iter()
            .map(|n| Path {
                cond: Bool::and(&[&p.cond, &n.cond]),
                end: n.end,
                out: p.out.iter().cloned().chain(n.out).collect(),
                state: n.state,
            })
            .collect())
    }

    fn pair(
        &self,
        joints: &Joints,
        p: Path<S::Loc>,
        q: Path<T::Loc>,
        depth: usize,
        out: &mut Pairs<S::Loc, T::Loc>,
    ) -> Result<(), Unsupported> {
        if joints.alike(&p.end, &q.end) || matches!(q.end, End::Overflow) || depth >= EXTEND {
            out.push((p, q));
            return Ok(());
        }
        match (cut(&p.end), cut(&q.end)) {
            (Some(k), _) => {
                for p2 in Self::extend(self.source, &p, k)? {
                    self.pair(joints, p2, q.clone(), depth + 1, out)?;
                }
            }
            (None, Some(k)) => {
                for q2 in Self::extend(self.target, &q, k)? {
                    self.pair(joints, p.clone(), q2, depth + 1, out)?;
                }
            }
            (None, None) => out.push((p, q)),
        }
        Ok(())
    }

    fn pairs(
        &self,
        joints: &Joints,
        sp: &[Path<S::Loc>],
        tp: &[Path<T::Loc>],
    ) -> Result<Pairs<S::Loc, T::Loc>, Unsupported> {
        let mut out = Vec::new();
        for p in sp {
            for q in tp {
                self.pair(joints, p.clone(), q.clone(), 0, &mut out)?;
            }
        }
        Ok(out)
    }

    pub fn prove(&self, params: &[SymVal], assumed: Vec<String>) -> Result<Finding, Unsupported> {
        let (ns, nt) = (self.source.loops().len(), self.target.loops().len());
        let shape = |ls: &[LoopNode]| ls.iter().map(|l| l.parent).collect::<Vec<_>>();
        let strict = shape(self.source.loops()) == shape(self.target.loops());

        let entry_s = self.source.explore(None, self.source.entry(params))?;
        let entry_t = self.target.explore(None, self.target.entry(params))?;

        let label_s = |k: usize| move |l: &S::Loc| format!("s.loop{}.{}", k + 1, self.source.describe(l));
        let label_t = |k: usize| move |l: &T::Loc| format!("t.loop{}.{}", k + 1, self.target.describe(l));
        let mut sprof: Vec<Option<Profile<S::Loc>>> = vec![None; ns];
        let mut tprof: Vec<Option<Profile<T::Loc>>> = vec![None; nt];
        Self::absorb(&entry_s, &mut sprof);
        Self::absorb(&entry_t, &mut tprof);
        loop {
            let mut changed = false;
            for k in 0..ns {
                if let Some(p) = sprof[k].clone() {
                    let paths = self.source.explore(Some(k), p.havoc(label_s(k))?)?;
                    changed |= Self::absorb(&paths, &mut sprof);
                }
            }
            for k in 0..nt {
                if let Some(p) = tprof[k].clone() {
                    let paths = self.target.explore(Some(k), p.havoc(label_t(k))?)?;
                    changed |= Self::absorb(&paths, &mut tprof);
                }
            }
            if !changed {
                break;
            }
        }
        if ns > 0 && nt == 0 {
            return self.prove_closed_form(params, assumed, &entry_s, &entry_t, &sprof);
        }

        let seen_s = Self::constants(&sprof);
        let seen_t = Self::constants(&tprof);
        let mut hs: Vec<Option<State<S::Loc>>> = vec![None; ns];
        let mut from_s: Vec<Option<Vec<Path<S::Loc>>>> = vec![None; ns];
        for k in 0..ns {
            if let Some(p) = &sprof[k] {
                let h = p.havoc(label_s(k))?;
                from_s[k] = Some(self.source.explore(Some(k), h.clone())?);
                hs[k] = Some(h);
            }
        }
        let mut ht: Vec<Option<State<T::Loc>>> = vec![None; nt];
        let mut from_t: Vec<Option<Vec<Path<T::Loc>>>> = vec![None; nt];
        for k in 0..nt {
            if let Some(p) = &tprof[k] {
                let h = p.havoc(label_t(k))?;
                from_t[k] = Some(self.target.explore(Some(k), h.clone())?);
                ht[k] = Some(h);
            }
        }

        let mut joints = Joints { strict, pairs: if strict { (0..ns).map(|k| (k, k)).collect() } else { Vec::new() } };
        let mut segments: Vec<Pairs<S::Loc, T::Loc>> = Vec::new();
        segments.push(self.pairs(&joints, &entry_s, &entry_t)?);
        let mut done = 0;
        loop {
            while done < segments.len() {
                let reached: Vec<(usize, usize)> = segments[done]
                    .iter()
                    .filter(|(p, q)| joints.alike(&p.end, &q.end))
                    .filter_map(|(p, q)| Some((cut(&p.end)?, cut(&q.end)?)))
                    .collect();
                for (a, b) in reached {
                    if joints.index(a, b).is_none() {
                        joints.pairs.push((a, b));
                    }
                }
                done += 1;
            }
            if segments.len() > joints.pairs.len() {
                break;
            }
            let (a, b) = joints.pairs[segments.len() - 1];
            let seg = match (&from_s[a], &from_t[b]) {
                (Some(ps), Some(qs)) => self.pairs(&joints, ps, qs)?,
                _ => Vec::new(),
            };
            segments.push(seg);
        }

        let loc = |l: &S::Loc| self.source.describe(l);
        let samples = Samples::gather(self.source, params, &sprof, &[]);
        let tested: Vec<&Bool> = entry_s
            .iter()
            .map(|p| &p.cond)
            .chain(entry_t.iter().map(|q| &q.cond))
            .chain(from_s.iter().flatten().flatten().map(|p| &p.cond))
            .chain(from_t.iter().flatten().flatten().map(|q| &q.cond))
            .collect();
        let guards = input_atoms(&tested, params);
        let (entered_s, entered_t) = (entry_terms(&entry_s, params), entry_terms(&entry_t, params));
        let mut cands: Vec<Vec<Cand<S::Loc, T::Loc>>> = joints
            .pairs
            .iter()
            .map(|&(a, b)| match (&sprof[a], &tprof[b]) {
                (Some(sp), Some(tp)) => {
                    let mut cs = candidates(sp, tp, &seen_s, &seen_t);
                    cs.extend(entered_t.get(&b).into_iter().flatten().map(|(r, x)| Cand::TargetEntry(r.clone(), x.clone())));
                    cs.extend(entered_s.get(&a).into_iter().flatten().map(|(v, x)| Cand::SourceEntry(v.clone(), x.clone())));
                    cs.extend(samples.facts(a, &loc).into_iter().map(Cand::Sampled));
                    cs.extend(guards.iter().cloned().map(Cand::Guard));
                    cs
                }
                _ => Vec::new(),
            })
            .collect();
        let starts: Vec<Option<(State<S::Loc>, State<T::Loc>)>> = joints
            .pairs
            .iter()
            .map(|&(a, b)| Some((hs[a].clone()?, ht[b].clone()?)))
            .collect();

        let solver = Solver::new();
        solver.set_params(&budget(INCREMENTAL_RLIMIT));
        solver.assert(&self.source.assumes(params));
        solver.assert(&self.target.assumes(params));

        let inv = |j: usize, cands: &[Vec<Cand<S::Loc, T::Loc>>]| -> Bool {
            if j == 0 {
                return Bool::from_bool(true);
            }
            match &starts[j - 1] {
                Some((s, t)) => Bool::and(&cands[j - 1].iter().map(|c| c.holds(t, s, params)).collect::<Vec<_>>()),
                None => Bool::from_bool(false),
            }
        };
        let joint_of = |p: &Path<S::Loc>, q: &Path<T::Loc>| -> Option<usize> {
            if !joints.alike(&p.end, &q.end) {
                return None;
            }
            joints.index(cut(&p.end)?, cut(&q.end)?)
        };

        loop {
            let mut dropped = false;
            for (j, pairs) in segments.iter().enumerate() {
                let assume = inv(j, &cands);
                for (p, q) in pairs {
                    let Some(i) = joint_of(p, q) else { continue };
                    solver.push();
                    solver.assert(&assume);
                    solver.assert(&p.cond);
                    solver.assert(&q.cond);
                    if decide(&solver, &[]).answer != SatResult::Unsat {
                        let before = cands[i].len();
                        let kept: Vec<Cand<S::Loc, T::Loc>> = cands[i]
                            .iter()
                            .filter(|c| {
                                let goal = c.holds(&q.state, &p.state, params).not();
                                decide_within(&solver, &[goal], HOUDINI_RLIMIT).answer == SatResult::Unsat
                            })
                            .cloned()
                            .collect();
                        if kept.len() != before {
                            cands[i] = kept;
                            dropped = true;
                        }
                    }
                    solver.pop(1);
                }
            }
            if !dropped {
                break;
            }
        }
        let mut overflow = false;
        for (j, pairs) in segments.iter().enumerate() {
            let assume = inv(j, &cands);
            let start = if j == 0 { None } else { starts[j - 1].as_ref() };
            let at = if j == 0 {
                "from the entry".to_string()
            } else {
                let (a, b) = joints.pairs[j - 1];
                if strict {
                    format!("from the head of loop {}", a + 1)
                } else {
                    format!("from the heads of source loop {} and target loop {}", a + 1, b + 1)
                }
            };
            for (p, q) in pairs {
                solver.push();
                solver.assert(&assume);
                solver.assert(&p.cond);
                solver.assert(&q.cond);
                let verdict = self.obligations(&solver, &joints, p, q, &at, start, params, &mut overflow);
                solver.pop(1);
                if let Some(v) = verdict {
                    return Ok(v);
                }
            }
        }

        let reported = cands
            .iter()
            .enumerate()
            .map(|(i, cs)| match &starts[i] {
                Some((s, t)) => Self::implied_dropped(&solver, cs, s, t, params),
                None => cs.clone(),
            })
            .map(|cs| cs.iter().map(|c| self.describe_cand(c, &samples, params)).collect())
            .collect();
        Ok(Finding::Proven { invariants: reported, assumed, overflow })
    }

    /// The invariant at a pair of heads as it is reported: without the linear conjuncts the other linear ones imply,
    /// the weakest tried first (bounds and guards, then constants, then relations of degree one, then the equalities
    /// between the two sides); a relation of higher degree is always kept, so every query is linear (`sample::prune`).
    /// The proof assumes all of them, since an explicit bound is one the solver's nonlinear reasoning uses directly.
    fn implied_dropped(
        solver: &Solver,
        cs: &[Cand<S::Loc, T::Loc>],
        s: &State<S::Loc>,
        t: &State<T::Loc>,
        params: &[SymVal],
    ) -> Vec<Cand<S::Loc, T::Loc>> {
        let rank = |c: &Cand<S::Loc, T::Loc>| match c {
            Cand::AtLeast(..) | Cand::Guard(_) => 0,
            Cand::Sampled(f) if f.is_bound() => 0,
            Cand::Target(..) | Cand::Source(..) | Cand::TargetEntry(..) | Cand::SourceEntry(..) => 1,
            Cand::Sampled(_) => 2,
            Cand::Same(..) => 3,
        };
        let linear = |c: &Cand<S::Loc, T::Loc>| match c {
            Cand::Sampled(f) => f.is_linear(),
            Cand::Guard(g) => is_linear(&Dynamic::from_ast(g)),
            Cand::TargetEntry(_, x) | Cand::SourceEntry(_, x) => is_linear(&Dynamic::from_ast(x)),
            _ => true,
        };
        let mut order: Vec<usize> = (0..cs.len()).filter(|&k| linear(&cs[k])).collect();
        order.sort_by_key(|&k| rank(&cs[k]));
        let mut gone = vec![false; cs.len()];
        for k in order {
            let mut assumed: Vec<Bool> = (0..cs.len())
                .filter(|&j| j != k && !gone[j] && linear(&cs[j]))
                .map(|j| cs[j].holds(t, s, params))
                .collect();
            assumed.push(cs[k].holds(t, s, params).not());
            gone[k] = solver.check_assumptions(&assumed) == SatResult::Unsat;
        }
        cs.iter().zip(gone).filter(|(_, g)| !g).map(|(c, _)| c.clone()).collect()
    }

    fn describe_cand(&self, c: &Cand<S::Loc, T::Loc>, samples: &Samples<S::Loc>, params: &[SymVal]) -> String {
        let loc = |l: &S::Loc| self.source.describe(l);
        match c {
            Cand::Sampled(f) => f.describe(&|v| samples.name(v, &loc), &loc),
            Cand::Guard(g) => render(&Dynamic::from_ast(g), &samples.input_names(params)),
            Cand::Same(r, v) => format!("{} = {}", self.target.describe(r), self.source.describe(v)),
            Cand::Target(r, l) => format!("{} = {}", self.target.describe(r), lit_text(l)),
            Cand::Source(v, l) => format!("{} = {}", self.source.describe(v), lit_text(l)),
            Cand::TargetEntry(r, x) => {
                format!("{} = {}", self.target.describe(r), render(&Dynamic::from_ast(x), &samples.input_names(params)))
            }
            Cand::SourceEntry(v, x) => {
                format!("{} = {}", self.source.describe(v), render(&Dynamic::from_ast(x), &samples.input_names(params)))
            }
            Cand::AtLeast(v, n) => format!("{} ≥ {n}", self.source.describe(v)),
        }
    }

    /// The pair's obligations, under the assertions already on the solver; `None` when every one holds. A pair that
    /// can run with the target stopping on an integer overflow sets `overflow`.
    #[allow(clippy::too_many_arguments)]
    fn obligations(
        &self,
        solver: &Solver,
        joints: &Joints,
        p: &Path<S::Loc>,
        q: &Path<T::Loc>,
        at: &str,
        start: Option<&(State<S::Loc>, State<T::Loc>)>,
        params: &[SymVal],
        overflow: &mut bool,
    ) -> Option<Finding> {
        let feasible = decide(solver, &[]);
        match feasible.answer {
            SatResult::Unsat => return None,
            SatResult::Unknown => return Some(self.unknown(feasible.by(solver), at)),
            SatResult::Sat => {}
        }
        let refute = |what: String, by: &Solver| {
            let (state, values) = self.witness(by, start, params);
            Finding::Failed { at: format!("{at}: {what}"), state, values }
        };
        let stopped = matches!(q.end, End::Overflow);
        if !stopped && !joints.alike(&p.end, &q.end) {
            return Some(refute(
                format!(
                    "the source {} where {} {}",
                    describe_end(&p.end),
                    self.what,
                    describe_end(&q.end)
                ),
                feasible.by(solver),
            ));
        }
        let shown_ok = if stopped { q.out.len() <= p.out.len() } else { q.out.len() == p.out.len() };
        if !shown_ok {
            return Some(refute(
                format!("the source shows {} value(s) where {} shows {}", p.out.len(), self.what, q.out.len()),
                feasible.by(solver),
            ));
        }
        let mut goals: Vec<(String, Bool)> = p
            .out
            .iter()
            .zip(&q.out)
            .enumerate()
            .map(|(i, (a, b))| (format!("shown value {} differs", i + 1), same(a, b)))
            .collect();
        if let (End::Return(a), End::Return(b)) = (&p.end, &q.end) {
            goals.push(("the returned value differs".into(), same(a, b)));
        }
        for (what, goal) in goals {
            let goal = discharge_wraps(solver, &goal);
            let d = decide(solver, &[goal.not()]);
            match d.answer {
                SatResult::Unsat => {}
                SatResult::Sat => return Some(refute(what, d.by(solver))),
                SatResult::Unknown => return Some(self.unknown(d.by(solver), at)),
            }
        }
        if stopped {
            *overflow = true;
        }
        None
    }

    fn unknown(&self, solver: &Solver, at: &str) -> Finding {
        Finding::Unknown {
            reason: format!("{at}: {}", solver.get_reason_unknown().unwrap_or_default()),
        }
    }

    /// The segment's starting state in Z3's model — the parameters from the entry, the havoc states at a pair of heads
    /// — and every integer that model gives a parameter or a havoc value.
    fn witness(
        &self,
        solver: &Solver,
        start: Option<&(State<S::Loc>, State<T::Loc>)>,
        params: &[SymVal],
    ) -> (String, Vec<String>) {
        let Some(model) = solver.get_model() else {
            return (String::new(), Vec::new());
        };
        let mut values = Vec::new();
        let mut show = |v: &SymVal| -> Option<String> {
            match v {
                SymVal::Int(i) => model.eval(i, true).map(|x| {
                    values.push(x.to_string());
                    x.to_string()
                }),
                SymVal::Bool(b) => model.eval(b, true).map(|x| x.to_string()),
                _ => None,
            }
        };
        let mut parts = Vec::new();
        match start {
            None => {
                let entry = self.source.entry(params);
                for (l, val) in &entry.env {
                    if let Some(s) = show(val) {
                        parts.push(format!("{} = {s}", self.source.describe(l)));
                    }
                }
            }
            Some((s, t)) => {
                for (l, val) in &s.env {
                    if let Some(x) = show(val) {
                        parts.push(format!("{} = {x}", self.source.describe(l)));
                    }
                }
                for (l, val) in &t.env {
                    if let Some(x) = show(val) {
                        parts.push(format!("{} = {x}", self.target.describe(l)));
                    }
                }
            }
        }
        (parts.join(", "), values)
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_tv/bytecode/product/tests.rs"]
mod tests;
