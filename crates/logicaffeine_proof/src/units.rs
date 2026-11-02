//! Equivalence over units proven apart: two circuits built of units (adders, multipliers, any module already proven
//! equal to its counterpart), each unit cut out of its circuit — its results free inputs, its operands outputs — and
//! the two decided over the units as functions they know nothing of, a unit's own proof standing for what it computes.
//!
//! Each side is unrolled to its latency ([`Side`]): its own outputs (those not named under a unit's instance) read at
//! the last clock, each unit's operands at every clock, each unit's results a free input at every clock, named apart by
//! side (`left:` and `right:`) so no two units are one unless proven so. An input with no name (an undefined bit) is a
//! fresh one of its own side at every clock, and a latch with no name starts from a fresh value of its own. A unit applied at clock `t` has its results at
//! clock `t + latency`: a pipelined unit's proof says its outputs that many clocks late are the function of its
//! operands.
//!
//! A unit's function is its name: a side's units of one function compute one function, and a left function is the
//! right one `functions` pairs it with (each pair a proof made apart), so the functions fall into classes, a side's
//! own names and the declared pairs joined.
//!
//! The decision runs in rounds. Each builds one miter of both sides with every identification so far and simulates
//! it; the applications of one class whose operands simulate alike are a group, on either side or both. Each
//! application of a group is put to [`crate::fraig`] against the group's first one it has not been refuted against: a
//! pair whose operands it proves equal, bit for bit, is identified for the next round, the later application's results
//! made the earlier's (functional consistency: one function of equal operands is equal), and a refuted one tries the
//! group's next. A round that identifies nothing more is the last, and decides the circuits' own outputs. An output
//! pair that differs over units the decision knows nothing of may still agree over the real ones, so it is
//! [`Verdict::Unknown`], never a difference.
//!
//! [`Report::check`] builds every round's miter again, replays its derivation from the miter's formula, finds in it
//! the operand equalities of every identification the next round makes, and in the last the outputs' equalities; an
//! application made one twice, or made one with itself through others, does not check.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::aig::{Aig, AigLit, Init, Leaf};
use crate::fraig;
use crate::lrat::{self, Lrat};
use crate::sweep::{MiterError, Verdict};

/// A unit of a cut circuit: its instance name, the function it computes (its module), the clocks its results follow
/// its operands by, and the names of its operand bits (outputs of the cut circuit) and result bits (inputs of it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    pub name: String,
    pub function: String,
    pub latency: usize,
    pub operands: Vec<String>,
    pub results: Vec<String>,
}

/// One side: its cut circuit, the clock its own outputs are read at, and its units.
#[derive(Clone, Copy, Debug)]
pub struct Side<'a> {
    pub aig: &'a Aig,
    pub latency: usize,
    pub units: &'a [Unit],
}

/// What a side's named input is at a clock: one input both sides share by its name, a constant the environment holds
/// it at, or a fresh input of the side's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Binding {
    Shared(String),
    Const(bool),
    Fresh,
}

/// An output bit a side claims at a clock, under the name its counterpart on the other side is claimed by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claim {
    pub name: String,
    pub output: String,
    pub frame: usize,
}

/// A side unrolled `frames` clocks: what each named input is at each clock (`bind`), the output bits it claims, and its
/// units. An input with no name is a fresh one of the side's own at every clock, whatever `bind` says.
pub struct Unrolled<'a> {
    pub aig: &'a Aig,
    pub frames: usize,
    pub bind: &'a dyn Fn(&str, usize) -> Binding,
    pub claims: &'a [Claim],
    pub units: &'a [Unit],
}

/// A port of a unit's instance that is no operand (a pipelined unit's clock) is the unit's, not the circuit's.
fn of_a_unit(units: &[Unit], name: &str) -> bool {
    units.iter().any(|u| name.strip_prefix(u.name.as_str()).is_some_and(|rest| rest.starts_with('.')))
}

/// A side read at its latency as an unrolled one: its inputs at clock `k ≥ 1` named `name@k`, at clock 0 by their own
/// names; its own outputs (no unit's operand or port) claimed at the latency by their names.
fn aligned_claims(side: &Side) -> Vec<Claim> {
    let operands: std::collections::HashSet<&str> = side.units.iter().flat_map(|u| u.operands.iter().map(String::as_str)).collect();
    side.aig
        .outputs()
        .iter()
        .filter_map(|o| o.name.clone())
        .filter(|n| !operands.contains(n.as_str()) && !of_a_unit(side.units, n))
        .map(|n| Claim { name: n.clone(), output: n, frame: side.latency })
        .collect()
}

fn aligned_bind(name: &str, k: usize) -> Binding {
    if k == 0 {
        Binding::Shared(name.to_string())
    } else {
        Binding::Shared(format!("{name}@{k}"))
    }
}

/// A unit of a side applied at a clock: the side (0 left, 1 right), the unit's index, the clock of its operands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Application {
    pub side: usize,
    pub unit: usize,
    pub frame: usize,
}

/// A round: the identifications its derivation justifies (an application kept, and one whose results become the kept
/// one's), and the derivation, from the round's miter, of their operands' equalities — in the last round, of the
/// outputs' equalities.
#[derive(Clone, Debug)]
pub struct Round {
    pub substituted: Vec<(Application, Application)>,
    pub proof: Lrat,
}

/// A decision over units: a verdict per output name, the rounds that made it, and the last round's cost.
#[derive(Clone, Debug)]
pub struct Report {
    pub verdicts: Vec<(String, Verdict)>,
    pub rounds: Vec<Round>,
    pub stats: fraig::Stats,
}

/// The miter of both sides under the identifications `subst` (each right result bit's name, the left one it is).
struct Miter {
    aig: Aig,
    outputs: Vec<(String, AigLit, AigLit)>,
    /// Each application's operand literals, by application.
    operands: BTreeMap<Application, Vec<AigLit>>,
}

const TAGS: [&str; 2] = ["left", "right"];

/// The name of side `side`'s result bit `bit` at clock `k`.
fn result_name(side: usize, bit: &str, k: usize) -> String {
    format!("{}:{bit}@{k}", TAGS[side])
}

fn build(sides: [&Unrolled; 2], subst: &HashMap<String, String>) -> Result<Miter, MiterError> {
    let mut aig = Aig::new();
    let mut inputs: HashMap<String, AigLit> = HashMap::new();
    let mut input = |aig: &mut Aig, name: String| -> AigLit { *inputs.entry(name.clone()).or_insert_with(|| aig.add_input(Some(name))) };
    let mut own: [BTreeMap<String, AigLit>; 2] = [BTreeMap::new(), BTreeMap::new()];
    let mut operands: BTreeMap<Application, Vec<AigLit>> = BTreeMap::new();
    for (s, side) in sides.iter().enumerate() {
        let g = side.aig;
        let result_bits: HashMap<&str, ()> = side.units.iter().flat_map(|u| u.results.iter().map(|r| (r.as_str(), ()))).collect();
        let operand_of: HashMap<&str, (usize, usize)> = side
            .units
            .iter()
            .enumerate()
            .flat_map(|(u, unit)| unit.operands.iter().enumerate().map(move |(i, o)| (o.as_str(), (u, i))))
            .collect();
        for unit in side.units {
            for name in unit.operands.iter().chain(&unit.results) {
                if g.input_index(name).is_none() && g.output_index(name).is_none() {
                    return Err(MiterError::Unmatched(format!("{}: {name}", TAGS[s])));
                }
            }
        }
        let mut claimed: HashMap<(&str, usize), Vec<&str>> = HashMap::new();
        for c in side.claims {
            if g.output_index(&c.output).is_none() || c.frame >= side.frames {
                return Err(MiterError::Unmatched(format!("{}: {} at clock {}", TAGS[s], c.output, c.frame)));
            }
            claimed.entry((c.output.as_str(), c.frame)).or_default().push(c.name.as_str());
        }
        let mut state: Vec<AigLit> = g
            .latches()
            .iter()
            .map(|l| match (l.init, &l.name) {
                (Init::Zero, _) => AigLit::FALSE,
                (Init::One, _) => AigLit::TRUE,
                (Init::Free, Some(name)) => input(&mut aig, format!("{}:{name}@init", TAGS[s])),
                (Init::Free, None) => aig.add_input(None),
            })
            .collect();
        for k in 0..side.frames {
            let frame_inputs: Vec<AigLit> = g
                .inputs()
                .iter()
                .map(|p| {
                    let Some(name) = p.name.clone() else { return aig.add_input(None) };
                    if result_bits.contains_key(name.as_str()) {
                        return input(&mut aig, resolve(subst, &result_name(s, &name, k)));
                    }
                    match (side.bind)(&name, k) {
                        Binding::Shared(symbol) => input(&mut aig, symbol),
                        Binding::Const(b) => if b { AigLit::TRUE } else { AigLit::FALSE },
                        Binding::Fresh => aig.add_input(None),
                    }
                })
                .collect();
            let map = aig.import(g, &mut |leaf| match leaf {
                Leaf::Input(i) => frame_inputs[i],
                Leaf::Latch(j) => state[j],
            });
            let at = |l: AigLit| map[l.node() as usize].with_phase(l.is_complemented());
            for o in g.outputs() {
                let name = o.name.clone().unwrap_or_default();
                if let Some(&(u, i)) = operand_of.get(name.as_str()) {
                    let app = Application { side: s, unit: u, frame: k };
                    let width = side.units[u].operands.len();
                    operands.entry(app).or_insert_with(|| vec![AigLit::FALSE; width])[i] = at(o.lit);
                }
                for claim in claimed.get(&(name.as_str(), k)).into_iter().flatten() {
                    own[s].insert(claim.to_string(), at(o.lit));
                }
            }
            state = g.latches().iter().map(|l| at(l.next)).collect();
        }
        operands.retain(|app, _| app.side != s || app.frame + side.units[app.unit].latency < side.frames);
    }
    let mut outputs = Vec::new();
    for (name, &l) in &own[0] {
        let &r = own[1].get(name).ok_or_else(|| MiterError::Unmatched(name.clone()))?;
        outputs.push((name.clone(), l, r));
    }
    if let Some(name) = own[1].keys().find(|n| !own[0].contains_key(*n)) {
        return Err(MiterError::Unmatched(name.clone()));
    }
    Ok(Miter { aig, outputs, operands })
}

/// The result bit a name stands for: each identified bit's name followed to the bit it was made.
fn resolve(subst: &HashMap<String, String>, name: &str) -> String {
    let mut at = name;
    while let Some(next) = subst.get(at) {
        at = next;
    }
    at.to_string()
}

fn unit<'a>(sides: [&'a Unrolled; 2], app: Application) -> &'a Unit {
    &sides[app.side].units[app.unit]
}

/// The identification of `made`'s application with `kept`'s: each of `made`'s result bits named the bit `kept`'s
/// stands for. An application made one already, or one `kept` already stands for, is refused, so the names stay a
/// forest and every name resolves.
fn identify(sides: [&Unrolled; 2], subst: &mut HashMap<String, String>, kept: Application, made: Application) -> bool {
    let (ku, mu) = (unit(sides, kept), unit(sides, made));
    let bits: Vec<(String, String)> = ku
        .results
        .iter()
        .zip(&mu.results)
        .map(|(k, m)| (result_name(made.side, m, made.frame + mu.latency), resolve(subst, &result_name(kept.side, k, kept.frame + ku.latency))))
        .collect();
    if bits.iter().any(|(from, to)| from == to || subst.contains_key(from)) {
        return false;
    }
    subst.extend(bits);
    true
}

/// The functions of both sides in classes: a side's units of one name are one function, and each pair `functions`
/// declares makes its left function and its right one one.
struct Classes(HashMap<(usize, String), usize>);

impl Classes {
    fn new(sides: [&Unrolled; 2], functions: &[(String, String)]) -> Classes {
        let mut index: HashMap<(usize, String), usize> = HashMap::new();
        for (s, side) in sides.iter().enumerate() {
            for u in side.units {
                let next = index.len();
                index.entry((s, u.function.clone())).or_insert(next);
            }
        }
        let mut parent: Vec<usize> = (0..index.len()).collect();
        fn root(parent: &mut [usize], mut x: usize) -> usize {
            while parent[x] != x {
                parent[x] = parent[parent[x]];
                x = parent[x];
            }
            x
        }
        for (l, r) in functions {
            if let (Some(&a), Some(&b)) = (index.get(&(0, l.clone())), index.get(&(1, r.clone()))) {
                let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
                parent[ra.max(rb)] = ra.min(rb);
            }
        }
        Classes(index.into_iter().map(|(k, i)| (k, root(&mut parent, i))).collect())
    }

    fn of(&self, sides: [&Unrolled; 2], app: Application) -> usize {
        self.0[&(app.side, unit(sides, app).function.clone())]
    }

    /// Whether `a` and `b` apply one function, with operands and results of one shape.
    fn one(&self, sides: [&Unrolled; 2], a: Application, b: Application) -> bool {
        let (au, bu) = (unit(sides, a), unit(sides, b));
        self.of(sides, a) == self.of(sides, b) && au.operands.len() == bu.operands.len() && au.results.len() == bu.results.len()
    }
}

/// Each application's operands as simulation words, over random inputs.
fn signatures(m: &Miter, words: usize, seed: u64) -> BTreeMap<Application, Vec<u64>> {
    let mut state = seed | 1;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let inputs: Vec<u64> = (0..m.aig.inputs().len() * words).map(|_| next()).collect();
    let values = m.aig.simulate(words, &inputs, &[]);
    let word = |l: AigLit, w: usize| {
        let v = values[l.node() as usize * words + w];
        if l.is_complemented() {
            !v
        } else {
            v
        }
    };
    m.operands.iter().map(|(app, lits)| (*app, lits.iter().flat_map(|&l| (0..words).map(move |w| word(l, w))).collect())).collect()
}

/// Decide `left` against `right` over their units, the functions of `functions` (each a left function, the right one
/// proven equal to it) the only ones identified.
pub fn decide(left: &Side, right: &Side, functions: &[(String, String)], opts: &fraig::Options) -> Result<Report, MiterError> {
    let (lc, rc) = (aligned_claims(left), aligned_claims(right));
    let l = Unrolled { aig: left.aig, frames: left.latency + 1, bind: &aligned_bind, claims: &lc, units: left.units };
    let r = Unrolled { aig: right.aig, frames: right.latency + 1, bind: &aligned_bind, claims: &rc, units: right.units };
    decide_unrolled(&l, &r, functions, opts)
}

/// The applications not yet made one with another, in groups of one class whose operands simulate alike, each group
/// in order and the groups ordered by their first.
fn groups(sides: [&Unrolled; 2], classes: &Classes, m: &Miter, made: &HashSet<Application>, opts: &fraig::Options) -> Vec<Vec<Application>> {
    let sig = signatures(m, opts.sim_words.max(1), opts.seed);
    let mut by: HashMap<(usize, usize, usize, &[u64]), Vec<Application>> = HashMap::new();
    for (&app, words) in &sig {
        if made.contains(&app) {
            continue;
        }
        let u = unit(sides, app);
        by.entry((classes.of(sides, app), u.operands.len(), u.results.len(), words.as_slice())).or_default().push(app);
    }
    let mut groups: Vec<Vec<Application>> = by.into_values().filter(|g| g.len() > 1).collect();
    groups.sort();
    groups
}

/// The identifications one miter proves: each application of a group put against the group's first it has not been
/// refuted against, a refuted one trying the next until one is proven or none is left; the pairs proven and their
/// derivation, or none.
fn identifications(m: &Miter, groups: &[Vec<Application>], opts: &fraig::Options) -> Option<(Vec<(Application, Application)>, Lrat)> {
    let mut apart: HashSet<(Application, Application)> = HashSet::new();
    loop {
        let mut candidates: Vec<(Application, Application)> = Vec::new();
        for group in groups {
            let mut kept: Vec<Application> = Vec::new();
            for &app in group {
                match kept.iter().find(|&&k| !apart.contains(&(k, app))) {
                    Some(&k) => candidates.push((k, app)),
                    None => kept.push(app),
                }
            }
        }
        if candidates.is_empty() {
            return None;
        }
        let pairs: Vec<(AigLit, AigLit)> =
            candidates.iter().flat_map(|(k, a)| m.operands[k].iter().copied().zip(m.operands[a].iter().copied())).collect();
        let (verdicts, proof, _) = fraig::decide(&m.aig, &pairs, opts);
        let mut proven: Vec<(Application, Application)> = Vec::new();
        let mut at = 0;
        for &(k, a) in &candidates {
            let width = m.operands[&k].len();
            if verdicts[at..at + width].iter().all(|v| *v == Verdict::Equal) {
                proven.push((k, a));
            } else {
                apart.insert((k, a));
            }
            at += width;
        }
        if !proven.is_empty() {
            return Some((proven, proof));
        }
    }
}

/// [`decide`] over sides unrolled as each says: the claims of one name, one on each side, are an output pair.
pub fn decide_unrolled(left: &Unrolled, right: &Unrolled, functions: &[(String, String)], opts: &fraig::Options) -> Result<Report, MiterError> {
    let sides = [left, right];
    let classes = Classes::new(sides, functions);
    let mut subst: HashMap<String, String> = HashMap::new();
    let mut made: HashSet<Application> = HashSet::new();
    let mut rounds: Vec<Round> = Vec::new();
    loop {
        let m = build(sides, &subst)?;
        let groups = groups(sides, &classes, &m, &made, opts);
        let Some((proven, proof)) = identifications(&m, &groups, opts) else {
            let pairs: Vec<(AigLit, AigLit)> = m.outputs.iter().map(|(_, l, r)| (*l, *r)).collect();
            let (verdicts, proof, stats) = fraig::decide(&m.aig, &pairs, opts);
            let verdicts = m
                .outputs
                .iter()
                .zip(verdicts)
                .map(|((name, _, _), v)| {
                    let v = match v {
                        Verdict::Differ { .. } => Verdict::Unknown("the outputs differ over the units as functions nothing is known of".into()),
                        v => v,
                    };
                    (name.clone(), v)
                })
                .collect();
            rounds.push(Round { substituted: Vec::new(), proof });
            return Ok(Report { verdicts, rounds, stats });
        };
        for &(k, a) in &proven {
            let fresh = identify(sides, &mut subst, k, a);
            debug_assert!(fresh, "an application of a group is made one once");
            made.insert(a);
        }
        rounds.push(Round { substituted: proven, proof });
    }
}

impl Report {
    /// Every round's derivation replays from its miter's formula and proves the operand equalities of every
    /// identification it makes, each of two applications of one function (one name on a side, or a pair `functions`
    /// declares), each made one once and never with itself; the last round's proves the outputs claimed equal; and no
    /// output is claimed different.
    pub fn check(&self, left: &Side, right: &Side, functions: &[(String, String)]) -> bool {
        let (lc, rc) = (aligned_claims(left), aligned_claims(right));
        let l = Unrolled { aig: left.aig, frames: left.latency + 1, bind: &aligned_bind, claims: &lc, units: left.units };
        let r = Unrolled { aig: right.aig, frames: right.latency + 1, bind: &aligned_bind, claims: &rc, units: right.units };
        self.check_unrolled(&l, &r, functions)
    }

    /// [`Report::check`] over sides unrolled as each says.
    pub fn check_unrolled(&self, left: &Unrolled, right: &Unrolled, functions: &[(String, String)]) -> bool {
        let sides = [left, right];
        let classes = Classes::new(sides, functions);
        let mut subst: HashMap<String, String> = HashMap::new();
        let Some((last, earlier)) = self.rounds.split_last() else { return false };
        for round in earlier {
            let Ok(m) = build(sides, &subst) else { return false };
            let mut claims = Vec::new();
            for &(k, a) in &round.substituted {
                let (Some(ko), Some(ao)) = (m.operands.get(&k), m.operands.get(&a)) else { return false };
                if ko.len() != ao.len() || !classes.one(sides, k, a) {
                    return false;
                }
                for (&x, &y) in ko.iter().zip(ao) {
                    if x != y {
                        claims.extend(fraig::claim(x, y));
                    }
                }
            }
            if !lrat::check_derivation(m.aig.num_nodes(), &fraig::formula(&m.aig), &round.proof, &claims) {
                return false;
            }
            for &(k, a) in &round.substituted {
                if !identify(sides, &mut subst, k, a) {
                    return false;
                }
            }
        }
        let Ok(m) = build(sides, &subst) else { return false };
        if self.verdicts.len() != m.outputs.len() {
            return false;
        }
        let mut claims = Vec::new();
        for ((name, a, b), (vname, v)) in m.outputs.iter().zip(&self.verdicts) {
            if name != vname || matches!(v, Verdict::Differ { .. }) {
                return false;
            }
            if *v == Verdict::Equal && a != b {
                claims.extend(fraig::claim(*a, *b));
            }
        }
        last.substituted.is_empty() && lrat::check_derivation(m.aig.num_nodes(), &fraig::formula(&m.aig), &last.proof, &claims)
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/units/tests.rs"]
mod tests;
