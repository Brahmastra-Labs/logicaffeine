//! What concrete runs of the source say about its loop heads: the candidate conjuncts the proof offers Houdini beside
//! its own. The source is run on sampled inputs ([`trace`]), the symbolic exploration on constants, where every
//! condition decides and one path stays live per segment, and the state at each head it reaches is recorded. From
//! those states come the polynomial relations among a head's integers and the inputs (`relation`), and the bounds
//! every run kept on each integer and on each difference of two. A sample is also filed under the region of inputs it
//! falls in (for the closed-form proof, the target path its inputs take); each region offers its own candidates, so a
//! bound one region keeps and another breaks is offered to both and kept only where it is inductive. Every candidate
//! holds on the runs it was read from; the proof keeps only the inductive ones.

use std::collections::{BTreeMap, BTreeSet};

use logicaffeine_base::numeric::BigInt;
use z3::ast::{Ast, Bool, Dynamic, Int};
use z3::{SatResult, Solver};

use super::relation::relations;
use super::{Profile, Shape, Side};
use crate::bytecode::path::{dead, End, Path, State};
use crate::bytecode::value::{numeral, Kind, SymVal};

/// The integers each sampled input takes, spread first so that a capped sample still varies every input.
const SAMPLE_VALUES: [i64; 24] = [0, 1, 3, 7, -1, 12, 2, 5, -3, 20, 4, 9, 6, -2, 15, 10, 8, 11, 30, -4, 25, 40, 17, 13];

/// How many input tuples are run.
const SAMPLES: usize = 128;

/// How many loop heads one sampled run may reach, and all the sampled runs together: a run of a loop with a constant
/// bound is followed to its exit, so the bound it exits at is a candidate.
const VISITS: usize = 2_000;
const TOTAL_VISITS: usize = 50_000;

/// How many inputs are found for a region no sampled tuple fell in: its runs, even cut off after a few heads, are states
/// the region reaches, and drop a candidate there that only another region keeps. Their values can be anywhere in the
/// region (`n` near 2⁶²), so relations are read from the sampled tuples' small states alone: the exact elimination's
/// cost grows with its numbers, and a relation found on enough points is the same relation.
const REGION_MODELS: usize = 3;

/// How many loop heads a run on such an input may reach: a few hundred states break what holds only in another
/// region, and the loop it runs may be billions of iterations long.
const MODEL_VISITS: usize = 200;

/// How many sampled states one search for relations reads, spread over the runs.
const ROWS: usize = 600;

/// The highest degree a relation is searched at, and the most monomials a search at one degree may take.
const DEGREE: u32 = 4;
const MONOMIALS: usize = 70;

/// An integer a candidate speaks of.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Var<L> {
    /// An integer the head holds.
    Loc(L),
    /// The first or the last value of a range the head holds.
    From(L),
    To(L),
    /// The run's `j`-th input.
    Input(usize),
}

/// A candidate conjunct of the invariant at one head.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Fact<L> {
    /// `Σ c · Π x^e = 0`.
    Poly(Vec<(BigInt, Vec<(Var<L>, u32)>)>),
    AtLeast(Var<L>, BigInt),
    AtMost(Var<L>, BigInt),
    /// `x − y ≥ c` and `x − y ≤ c`.
    GapAtLeast(Var<L>, Var<L>, BigInt),
    GapAtMost(Var<L>, Var<L>, BigInt),
    /// A truth value the head holds.
    Truth(L, bool),
}

fn int_of(n: &BigInt) -> Int {
    n.to_string().parse::<Int>().expect("a decimal numeral is an Int")
}

fn superscript(e: u32) -> String {
    match e {
        1 => String::new(),
        2 => "²".into(),
        3 => "³".into(),
        4 => "⁴".into(),
        e => format!("^{e}"),
    }
}

/// The term `v` stands for in `state`, the run's inputs being `inputs`.
fn term<L: Ord>(v: &Var<L>, state: &State<L>, inputs: &[SymVal]) -> Option<Int> {
    match v {
        Var::Loc(l) => match state.env.get(l)? {
            SymVal::Int(i) => Some(i.clone()),
            _ => None,
        },
        Var::From(l) | Var::To(l) => match state.env.get(l)? {
            SymVal::Range(a, b) => Some((if matches!(v, Var::From(_)) { a } else { b }).clone()),
            _ => None,
        },
        Var::Input(j) => match inputs.get(*j)? {
            SymVal::Int(i) => Some(i.clone()),
            _ => None,
        },
    }
}

impl<L: Ord + Clone> Fact<L> {
    /// The fact of `state`, the run's inputs being `inputs`; false when the state lacks a variable it speaks of.
    pub(super) fn holds(&self, state: &State<L>, inputs: &[SymVal]) -> Bool {
        let at = |v: &Var<L>| term(v, state, inputs);
        let built = match self {
            Fact::Truth(l, b) => match state.env.get(l) {
                Some(SymVal::Bool(x)) => Some(x.eq(Bool::from_bool(*b))),
                _ => None,
            },
            Fact::Poly(terms) => terms
                .iter()
                .try_fold(Int::from_i64(0), |acc, (c, m)| {
                    let product = m.iter().try_fold(int_of(c), |p, (v, e)| {
                        let x = at(v)?;
                        Some((0..*e).fold(p, |p, _| &p * &x))
                    })?;
                    Some(&acc + &product)
                })
                .map(|sum| sum.eq(Int::from_i64(0))),
            Fact::AtLeast(v, c) => at(v).map(|x| x.ge(int_of(c))),
            Fact::AtMost(v, c) => at(v).map(|x| x.le(int_of(c))),
            Fact::GapAtLeast(a, b, c) => at(a).zip(at(b)).map(|(x, y)| (&x - &y).ge(int_of(c))),
            Fact::GapAtMost(a, b, c) => at(a).zip(at(b)).map(|(x, y)| (&x - &y).le(int_of(c))),
        };
        built.unwrap_or_else(|| Bool::from_bool(false))
    }

    /// The fact on a sampled row of `vars`, decided exactly.
    pub(super) fn on(&self, vars: &[Var<L>], row: &[BigInt]) -> bool {
        let at = |v: &Var<L>| vars.iter().position(|w| w == v).map(|i| &row[i]);
        let value = |v: &Var<L>| at(v).cloned().unwrap_or_else(BigInt::zero);
        match self {
            Fact::Poly(terms) => terms
                .iter()
                .fold(BigInt::zero(), |acc, (c, m)| {
                    acc.add(&m.iter().fold(c.clone(), |p, (v, e)| (0..*e).fold(p, |p, _| p.mul(&value(v)))))
                })
                .is_zero(),
            Fact::AtLeast(v, c) => value(v) >= *c,
            Fact::AtMost(v, c) => value(v) <= *c,
            Fact::GapAtLeast(a, b, c) => value(a).sub(&value(b)) >= *c,
            Fact::GapAtMost(a, b, c) => value(a).sub(&value(b)) <= *c,
            Fact::Truth(..) => true,
        }
    }

    /// The quantity this fact bounds from above, as a term of `state`: the variable or difference an upper bound caps,
    /// or the negation of one a lower bound floors. A loop whose every iteration raises it by at least one ends.
    pub(super) fn capped(&self, state: &State<L>, inputs: &[SymVal]) -> Option<Int> {
        let at = |v: &Var<L>| term(v, state, inputs);
        match self {
            Fact::AtMost(v, _) => at(v),
            Fact::AtLeast(v, _) => at(v).map(|x| -x),
            Fact::GapAtMost(a, b, _) => Some(&at(a)? - &at(b)?),
            Fact::GapAtLeast(a, b, _) => Some(&at(b)? - &at(a)?),
            Fact::Poly(_) | Fact::Truth(..) => None,
        }
    }

    /// A bound rather than a relation: what pruning tries to drop first.
    pub(super) fn is_bound(&self) -> bool {
        !matches!(self, Fact::Poly(_))
    }

    /// Linear in its variables: a bound, a truth value, or a relation of degree one.
    pub(super) fn is_linear(&self) -> bool {
        match self {
            Fact::Poly(terms) => terms.iter().all(|(_, m)| m.iter().map(|(_, e)| e).sum::<u32>() <= 1),
            _ => true,
        }
    }

    pub(super) fn describe(&self, name: &dyn Fn(&Var<L>) -> String, loc: &dyn Fn(&L) -> String) -> String {
        let monomial = |m: &[(Var<L>, u32)]| -> String {
            m.iter().map(|(v, e)| format!("{}{}", name(v), superscript(*e))).collect::<Vec<_>>().join("·")
        };
        match self {
            Fact::Poly(terms) => {
                let side = |positive: bool| -> String {
                    let parts: Vec<String> = terms
                        .iter()
                        .filter(|(c, _)| c.is_negative() != positive)
                        .map(|(c, m)| {
                            let c = c.abs();
                            match (m.is_empty(), c == BigInt::from_i64(1)) {
                                (true, _) => c.to_string(),
                                (false, true) => monomial(m),
                                (false, false) => format!("{c}·{}", monomial(m)),
                            }
                        })
                        .collect();
                    if parts.is_empty() {
                        "0".into()
                    } else {
                        parts.join(" + ")
                    }
                };
                format!("{} = {}", side(true), side(false))
            }
            Fact::AtLeast(v, c) => format!("{} ≥ {c}", name(v)),
            Fact::AtMost(v, c) => format!("{} ≤ {c}", name(v)),
            Fact::GapAtLeast(a, b, c) => format!("{} − {} ≥ {c}", name(a), name(b)),
            Fact::GapAtMost(a, b, c) => format!("{} − {} ≤ {c}", name(a), name(b)),
            Fact::Truth(l, b) => format!("{} = {b}", loc(l)),
        }
    }
}

/// The candidates sampled rows of `vars` offer: the relations `small` satisfies, and the bounds `rows` kept on each
/// variable and on each difference of two.
fn offered<L: Clone>(vars: &[Var<L>], small: &[Vec<BigInt>], rows: &[Vec<BigInt>]) -> Vec<Fact<L>> {
    let mut out: Vec<Fact<L>> = Vec::new();
    if rows.is_empty() {
        return out;
    }
    let step = small.len().div_ceil(ROWS).max(1);
    let picked: Vec<Vec<BigInt>> = small.iter().step_by(step).cloned().collect();
    for r in relations(&picked, DEGREE, MONOMIALS) {
        let terms = r
            .terms
            .into_iter()
            .map(|(c, m)| (c, m.iter().enumerate().filter(|(_, &e)| e > 0).map(|(v, &e)| (vars[v].clone(), e)).collect()))
            .collect();
        out.push(Fact::Poly(terms));
    }
    let extreme = |f: &dyn Fn(&Vec<BigInt>) -> BigInt| -> (BigInt, BigInt) {
        let values: Vec<BigInt> = rows.iter().map(f).collect();
        (values.iter().min().cloned().expect("rows"), values.iter().max().cloned().expect("rows"))
    };
    for (i, v) in vars.iter().enumerate() {
        let (lo, hi) = extreme(&|r| r[i].clone());
        out.push(Fact::AtLeast(v.clone(), lo));
        out.push(Fact::AtMost(v.clone(), hi));
    }
    for a in 0..vars.len() {
        for b in a + 1..vars.len() {
            let (lo, hi) = extreme(&|r| r[a].sub(&r[b]));
            out.push(Fact::GapAtLeast(vars[a].clone(), vars[b].clone(), lo.clone()));
            out.push(Fact::GapAtMost(vars[a].clone(), vars[b].clone(), hi));
        }
    }
    out
}

/// The states a run of `side` on constant `inputs` reaches its loop heads in, in order, up to `visits` heads.
fn trace<X: Side>(side: &X, inputs: &[SymVal], visits: usize) -> Vec<(usize, State<X::Loc>)> {
    let mut out = Vec::new();
    let Ok(mut paths) = side.explore(None, side.entry(inputs)) else {
        return out;
    };
    while out.len() < visits {
        let mut live = paths.into_iter().filter(|p| !dead(&p.cond));
        let (Some(p), None) = (live.next(), live.next()) else {
            break;
        };
        let End::Cut(k) = p.end else {
            break;
        };
        let state = State { env: p.state.env.into_iter().map(|(l, v)| (l, v.settled())).collect(), depth: p.state.depth };
        out.push((k, state.clone()));
        match side.explore(Some(k), state) {
            Ok(next) => paths = next,
            Err(_) => break,
        }
    }
    out
}

/// The integers tried for a region no sampled tuple fell in: both sides of the widths compiled code guards on, powers
/// of ten, and the rims of `i64`.
fn probe_values() -> Vec<i64> {
    let mut out = Vec::new();
    for k in [8u32, 16, 20, 31, 32, 40, 48, 52, 61, 62] {
        let p = 1i64 << k;
        out.extend([p, p + 1, p - 1, -p, -p - 1, -p + 1]);
    }
    for k in [3u32, 6, 9, 10, 12, 15, 18] {
        let p = 10i64.pow(k);
        out.extend([p, p + 1, -p]);
    }
    out.extend([i64::MAX, i64::MAX - 1, i64::MIN, i64::MIN + 1, 3_037_000_499, 3_037_000_500]);
    out
}

/// Up to `count` inputs satisfying `cond`, found among [`probe_values`] by evaluating `cond` at each: no solver, so a
/// condition Z3 would take seconds to simplify symbolically costs a substitution per input here.
fn probes(cond: &Bool, params: &[SymVal], count: usize) -> Vec<Vec<SymVal>> {
    let values = probe_values();
    let choices: Vec<Vec<SymVal>> = params
        .iter()
        .map(|p| match p.kind() {
            Kind::Bool => vec![SymVal::Bool(Bool::from_bool(false)), SymVal::Bool(Bool::from_bool(true))],
            _ => values.iter().chain(&SAMPLE_VALUES).map(|&n| SymVal::Int(Int::from_i64(n))).collect(),
        })
        .collect();
    crate::bytecode::tuples(&choices, PROBES)
        .into_iter()
        .filter(|sample| evaluate(cond, params, sample) == Some(true))
        .take(count)
        .collect()
}

/// How many input tuples a region's probe tries.
const PROBES: usize = 2_000;

/// `cond` with every input replaced by its value in `sample`, decided.
pub(super) fn evaluate(cond: &Bool, params: &[SymVal], sample: &[SymVal]) -> Option<bool> {
    let mut ints: Vec<(&Int, &Int)> = Vec::new();
    let mut bools: Vec<(&Bool, &Bool)> = Vec::new();
    for (p, s) in params.iter().zip(sample) {
        match (p, s) {
            (SymVal::Int(a), SymVal::Int(b)) => ints.push((a, b)),
            (SymVal::Bool(a), SymVal::Bool(b)) => bools.push((a, b)),
            _ => return None,
        }
    }
    cond.substitute(&ints).substitute(&bools).simplify().as_bool()
}

/// A condition on the inputs as a reader writes it: infix, every input by its source name with a `₀`.
pub(super) fn render(t: &Dynamic, names: &BTreeMap<String, String>) -> String {
    let kids = t.children();
    if kids.is_empty() {
        let text = t.to_string();
        return names.get(&text).cloned().or_else(|| numeral(&text)).unwrap_or(text);
    }
    let all = |sep: &str| kids.iter().map(|k| render(k, names)).collect::<Vec<_>>().join(sep);
    let two = |op: &str| format!("{} {op} {}", render(&kids[0], names), render(&kids[1], names));
    let op = t.decl().name();
    match (op.as_str(), kids.len()) {
        ("and", _) => all(" ∧ "),
        ("or", _) => format!("({})", all(" ∨ ")),
        ("not", 1) => {
            let inner = &kids[0];
            let flipped = match inner.decl().name().as_str() {
                "<=" => Some(">"),
                ">=" => Some("<"),
                "<" => Some("≥"),
                ">" => Some("≤"),
                "=" => Some("≠"),
                _ => None,
            };
            match (flipped, inner.children().as_slice()) {
                (Some(f), [a, b]) => format!("{} {f} {}", render(a, names), render(b, names)),
                _ => format!("¬({})", render(inner, names)),
            }
        }
        (">=", 2) => two("≥"),
        ("<=", 2) => two("≤"),
        ("<", 2) => two("<"),
        (">", 2) => two(">"),
        ("=", 2) => two("="),
        ("+", _) => format!("({})", all(" + ")),
        ("-", 1) => format!("−{}", render(&kids[0], names)),
        ("-", 2) => format!("({})", two("−")),
        ("*", _) => all("·"),
        ("div", 2) => format!("({})", two("div")),
        ("mod", 2) => format!("({})", two("mod")),
        ("if" | "ite", 3) => format!(
            "(if {} then {} else {})",
            render(&kids[0], names),
            render(&kids[1], names),
            render(&kids[2], names)
        ),
        _ => t.to_string(),
    }
}

/// The inputs `params` stand for, as the terms that name them.
fn input_terms(params: &[SymVal]) -> BTreeSet<String> {
    params
        .iter()
        .filter_map(|p| match p {
            SymVal::Int(i) => Some(i.to_string()),
            SymVal::Bool(b) => Some(b.to_string()),
            _ => None,
        })
        .collect()
}

/// Whether `t` is a term over the inputs alone: the inputs and numerals under `+`, `−` and `·`.
fn over_inputs(t: &Dynamic, inputs: &BTreeSet<String>) -> bool {
    let kids = t.children();
    if kids.is_empty() {
        return inputs.contains(&t.to_string()) || numeral(&t.to_string()).is_some();
    }
    matches!(t.decl().name().as_str(), "+" | "-" | "*") && kids.iter().all(|k| over_inputs(k, inputs))
}

/// Whether every constant `t` names is an input or a literal: a term over the inputs alone, whatever its operators (the
/// wrap of a release-build `+` included).
fn names_only_inputs(t: &Dynamic, inputs: &BTreeSet<String>) -> bool {
    let kids = t.children();
    if kids.is_empty() {
        let text = t.to_string();
        return inputs.contains(&text) || numeral(&text).is_some() || matches!(text.as_str(), "true" | "false");
    }
    kids.iter().all(|k| names_only_inputs(k, inputs))
}

/// What each integer location of `paths`' ends holds where a path reaches a loop head from the entry, by head, when
/// that is a term over the inputs and no numeral (a numeral is a constant candidate already). The inputs do not change
/// along a run, so a location the loop never writes keeps it at every head (`for`'s bound, `a..(b + 1)`).
pub(super) fn entry_terms<L: Ord + Clone>(paths: &[Path<L>], params: &[SymVal]) -> BTreeMap<usize, Vec<(L, Int)>> {
    let inputs = input_terms(params);
    let mut out: BTreeMap<usize, Vec<(L, Int)>> = BTreeMap::new();
    for p in paths {
        let End::Cut(k) = p.end else { continue };
        for (l, v) in &p.state.env {
            let SymVal::Int(x) = v else { continue };
            let x = x.simplify();
            if x.as_i64().is_some() || !names_only_inputs(&Dynamic::from_ast(&x), &inputs) {
                continue;
            }
            let at = out.entry(k).or_default();
            if !at.iter().any(|(m, y)| m == l && y.to_string() == x.to_string()) {
                at.push((l.clone(), x));
            }
        }
    }
    out
}

/// The comparisons over the inputs alone that `conds` test. The inputs do not change along a run, so each is true of
/// every head a run reaches or of none, and a head may keep it as a conjunct: a guard the compiled code tested before
/// entering a loop is still known inside it.
pub(super) fn input_atoms(conds: &[&Bool], params: &[SymVal]) -> Vec<Bool> {
    let inputs = input_terms(params);
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let mut work: Vec<Dynamic> = conds.iter().map(|c| Dynamic::from_ast(&c.simplify())).collect();
    while let Some(t) = work.pop() {
        let kids = t.children();
        match t.decl().name().as_str() {
            "and" | "or" | "not" => work.extend(kids),
            "<=" | ">=" | "<" | ">" | "=" if kids.len() == 2 && kids.iter().all(|k| over_inputs(k, &inputs)) => {
                if let Some(b) = t.as_bool() {
                    if seen.insert(b.to_string()) {
                        out.push(b);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// `facts` without the linear ones the other linear facts imply at `state`, under what the solver already holds:
/// bounds tried before relations of degree one, a relation of higher degree always kept and never assumed. The
/// conjunction keeps its meaning, and every query is linear, so decided at once: asking whether a polynomial relation is
/// implied is a search for a model of it, which Z3 can spend minutes on without counting the work.
pub(super) fn prune<L: Ord + Clone>(solver: &Solver, facts: Vec<Fact<L>>, state: &State<L>, inputs: &[SymVal]) -> Vec<Fact<L>> {
    let linear = |i: usize| facts[i].is_linear();
    let order = (0..facts.len())
        .filter(|&i| linear(i) && facts[i].is_bound())
        .chain((0..facts.len()).filter(|&i| linear(i) && !facts[i].is_bound()));
    let mut gone = vec![false; facts.len()];
    for i in order.collect::<Vec<_>>() {
        let mut assumed: Vec<Bool> =
            (0..facts.len()).filter(|&j| j != i && !gone[j] && linear(j)).map(|j| facts[j].holds(state, inputs)).collect();
        assumed.push(facts[i].holds(state, inputs).not());
        if solver.check_assumptions(&assumed) == SatResult::Unsat {
            gone[i] = true;
        }
    }
    facts.into_iter().zip(gone).filter(|(_, g)| !g).map(|(f, _)| f).collect()
}

/// The sampled states at each head of one function, with the region of inputs each came from.
pub(super) struct Samples<L> {
    vars: Vec<Vec<Var<L>>>,
    /// Each head's states: the region of the run's inputs, whether they were a sampled tuple (not a Z3 model), the
    /// values.
    rows: Vec<Vec<(Option<usize>, bool, Vec<BigInt>)>>,
    truths: Vec<Vec<(Option<usize>, L, bool)>>,
    inputs: Vec<String>,
    regions: usize,
}

impl<L: Ord + Clone> Samples<L> {
    /// Run `side` on sampled inputs and record each head's states, filing each run under the first of `regions` its
    /// inputs satisfy.
    pub(super) fn gather<X: Side<Loc = L>>(
        side: &X,
        params: &[SymVal],
        profiles: &[Option<Profile<L>>],
        regions: &[Bool],
    ) -> Samples<L> {
        let entry = side.entry(params);
        let inputs = (0..params.len())
            .map(|j| {
                entry
                    .env
                    .iter()
                    .find(|(_, v)| match (v, &params[j]) {
                        (SymVal::Int(a), SymVal::Int(b)) => a == b,
                        (SymVal::Bool(a), SymVal::Bool(b)) => a == b,
                        _ => false,
                    })
                    .map(|(l, _)| side.describe(l))
                    .unwrap_or_else(|| format!("input {}", j + 1))
            })
            .collect();
        let vars: Vec<Vec<Var<L>>> = profiles
            .iter()
            .map(|p| {
                let mut vs = Vec::new();
                for (l, s) in p.iter().flat_map(|p| &p.shapes) {
                    match s {
                        Shape::One(Kind::Int) => vs.push(Var::Loc(l.clone())),
                        Shape::One(Kind::Range) => {
                            vs.push(Var::From(l.clone()));
                            vs.push(Var::To(l.clone()));
                        }
                        _ => {}
                    }
                }
                vs.extend(params.iter().enumerate().filter(|(_, v)| v.kind() == Kind::Int).map(|(j, _)| Var::Input(j)));
                vs
            })
            .collect();
        let choices: Vec<Vec<SymVal>> = params
            .iter()
            .map(|p| match p.kind() {
                Kind::Bool => vec![SymVal::Bool(Bool::from_bool(false)), SymVal::Bool(Bool::from_bool(true))],
                _ => SAMPLE_VALUES.iter().map(|&n| SymVal::Int(Int::from_i64(n))).collect(),
            })
            .collect();
        let mut samples = Samples {
            rows: vec![Vec::new(); profiles.len()],
            truths: vec![Vec::new(); profiles.len()],
            vars,
            inputs,
            regions: regions.len(),
        };
        if profiles.iter().all(Option::is_none) {
            return samples;
        }
        let mut visited = 0;
        let mut runs: Vec<(Vec<SymVal>, Option<usize>, bool)> = crate::bytecode::tuples(&choices, SAMPLES)
            .into_iter()
            .map(|sample| {
                let region = regions.iter().position(|c| evaluate(c, params, &sample) == Some(true));
                (sample, region, true)
            })
            .collect();
        for (r, cond) in regions.iter().enumerate() {
            if !runs.iter().any(|(_, region, _)| *region == Some(r)) {
                runs.extend(probes(cond, params, REGION_MODELS).into_iter().map(|sample| (sample, Some(r), false)));
            }
        }
        for (sample, region, tuple) in runs {
            if visited >= TOTAL_VISITS {
                break;
            }
            let states = trace(side, &sample, if tuple { VISITS } else { MODEL_VISITS });
            visited += states.len();
            for (k, state) in states {
                let row: Option<Vec<BigInt>> = samples.vars[k]
                    .iter()
                    .map(|v| BigInt::parse_decimal(&numeral(&term(v, &state, &sample)?.simplify().to_string())?))
                    .collect();
                if let Some(row) = row {
                    samples.rows[k].push((region, tuple, row));
                }
                for (l, v) in &state.env {
                    if let SymVal::Bool(b) = v {
                        if let Some(b) = b.simplify().as_bool() {
                            samples.truths[k].push((region, l.clone(), b));
                        }
                    }
                }
            }
        }
        samples
    }

    /// The name a variable goes by in a candidate: a location's own, an input's with a `₀` (the value the run began
    /// with), a range's ends by `first` and `last`.
    pub(super) fn name(&self, v: &Var<L>, loc: &dyn Fn(&L) -> String) -> String {
        match v {
            Var::Loc(l) => loc(l),
            Var::From(l) => format!("{}.first", loc(l)),
            Var::To(l) => format!("{}.last", loc(l)),
            Var::Input(j) => format!("{}₀", self.inputs[*j]),
        }
    }

    /// Each input's term, as Z3 prints it, with the name a condition is rendered with.
    pub(super) fn input_names(&self, params: &[SymVal]) -> BTreeMap<String, String> {
        params
            .iter()
            .zip(&self.inputs)
            .filter_map(|(p, name)| match p {
                SymVal::Int(i) => Some((i.to_string(), format!("{name}₀"))),
                SymVal::Bool(b) => Some((b.to_string(), format!("{name}₀"))),
                _ => None,
            })
            .collect()
    }

    /// Every candidate at head `k`: what all the runs offer, and what each region's runs offer, once each.
    pub(super) fn facts(&self, k: usize, loc: &dyn Fn(&L) -> String) -> Vec<Fact<L>> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for region in std::iter::once(None).chain((0..self.regions).map(Some)) {
            let within = |r: &Option<usize>| region.is_none_or(|want| *r == Some(want));
            let picked: Vec<Vec<BigInt>> =
                self.rows[k].iter().filter(|(r, _, _)| within(r)).map(|(_, _, row)| row.clone()).collect();
            let small: Vec<Vec<BigInt>> =
                self.rows[k].iter().filter(|(r, t, _)| *t && within(r)).map(|(_, _, row)| row.clone()).collect();
            let mut offer = offered(&self.vars[k], &small, &picked);
            let held: Vec<&(Option<usize>, L, bool)> = self.truths[k].iter().filter(|(r, _, _)| within(r)).collect();
            for (_, l, b) in &held {
                if held.iter().filter(|(_, m, _)| m == l).all(|(_, _, c)| c == b) {
                    offer.push(Fact::Truth(l.clone(), *b));
                }
            }
            for f in offer {
                if seen.insert(f.describe(&|v| self.name(v, loc), loc)) {
                    out.push(f);
                }
            }
        }
        out
    }

    /// A sampled run of `region` reached head `k` in a state `fact` is false of.
    pub(super) fn breaks(&self, k: usize, region: usize, fact: &Fact<L>) -> bool {
        self.rows[k].iter().any(|(r, _, row)| *r == Some(region) && !fact.on(&self.vars[k], row))
    }
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_tv/bytecode/product/sample/tests.rs"]
mod tests;
