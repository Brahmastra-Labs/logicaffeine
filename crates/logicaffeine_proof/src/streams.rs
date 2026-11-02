//! Stream equivalence over units proven apart: two circuits that take a stream of tokens through a valid-ready handshake
//! and give a stream back, each at its own width and its own pace, decided equal token for token under an arrival
//! pattern.
//!
//! The environment ([`Environment`]) offers a token at the clocks it says, takes one at the clocks it says, and holds
//! every other input as it says (a reset's level, a value both sides share for the whole run). Under it each side's
//! handshakes are a schedule ([`schedule`]): the clocks it takes its input tokens at and gives its output tokens at, each
//! proven the constant it is with the data free, so a side whose handshake depends on the data has no schedule and is
//! refused by name. The sides are then unrolled ([`crate::units::Unrolled`]) with each side's j-th input token read where
//! that side takes it ([`Ports::token`] names the symbols a token's bits are, so a side two words wide takes words 2j and
//! 2j+1), and each side's k-th output token claimed where that side gives it, its bits `out<k>.<i>`; [`decide`] decides
//! the claims over the units as [`crate::units::decide_unrolled`] does. A token one side gives past what the other gives
//! in the clocks unrolled is counted, not claimed.

use std::collections::HashSet;
use std::fmt;

use crate::aig::{Aig, AigLit, Init, Leaf};
use crate::fraig;
use crate::lrat::{self, Lrat};
use crate::sweep::{MiterError, Verdict};
use crate::units::{self, Binding, Claim, Unit, Unrolled};

/// A side's stream ports, by bit name: the input stream's valid, ready and data, the output stream's valid, ready (none
/// for a side that never waits for it) and data; and what bit `bit` of the side's j-th input token reads (`token(j,
/// bit)`, a symbol the other side's token reads too).
pub struct Ports<'a> {
    pub in_valid: String,
    pub in_ready: String,
    pub in_data: Vec<String>,
    pub out_valid: String,
    pub out_ready: Option<String>,
    pub out_data: Vec<String>,
    pub token: &'a dyn Fn(usize, &str) -> Binding,
}

/// The arrival pattern a side runs under, `frames` clocks: whether a token is offered at each clock, whether a given one
/// is taken (or, for a side that never waits, watched for) at each clock, and what every other input is at each clock.
/// Only the handshakes at those clocks are the side's schedule, so the clocks before a reset settles are left out by
/// offering and taking nothing there.
pub struct Environment<'a> {
    pub frames: usize,
    pub offer: &'a dyn Fn(usize) -> bool,
    pub accept: &'a dyn Fn(usize) -> bool,
    pub other: &'a dyn Fn(&str, usize) -> Binding,
}

/// One side: its cut circuit, its units, its ports.
pub struct StreamSide<'a> {
    pub aig: &'a Aig,
    pub units: &'a [Unit],
    pub ports: Ports<'a>,
}

/// A side's schedule under its environment: the clocks it took its input tokens at and gave its output tokens at, and
/// the derivation, from the side unrolled alone, that every handshake is the constant it was taken to be.
#[derive(Clone, Debug)]
pub struct Schedule {
    pub took: Vec<usize>,
    pub gave: Vec<usize>,
    pub proof: Lrat,
}

/// Why two streams cannot be decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamError {
    Miter(MiterError),
    /// A port the side's circuit has no bit of.
    Port(String),
    /// A handshake that is not one constant at a clock whatever the data: the side has no schedule.
    DataDependent { port: String, frame: usize },
    /// A handshake the search could not prove constant within its budget.
    Undecided { port: String, frame: usize },
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            StreamError::Miter(e) => write!(f, "{e:?}"),
            StreamError::Port(p) => write!(f, "no port {p}"),
            StreamError::DataDependent { port, frame } => write!(f, "{port} at clock {frame} depends on the data: the side has no schedule"),
            StreamError::Undecided { port, frame } => write!(f, "{port} at clock {frame} was not proven constant"),
        }
    }
}

/// The side unrolled alone under its environment, its input data and every unit's result free at every clock, and its
/// handshakes at every clock: the input stream's ready, the output stream's valid. Only what the handshakes read
/// through any number of clocks is unrolled ([`Aig::sequential_cone`]): a datapath they never read costs nothing.
struct Alone {
    aig: Aig,
    ready: Vec<AigLit>,
    valid: Vec<AigLit>,
}

fn alone(side: &StreamSide, env: &Environment) -> Result<Alone, StreamError> {
    let p = &side.ports;
    for port in [&p.in_valid].into_iter().chain(&p.in_data).chain(p.out_ready.iter()) {
        side.aig.input_index(port).ok_or_else(|| StreamError::Port(port.clone()))?;
    }
    for port in [&p.in_ready, &p.out_valid].into_iter().chain(&p.out_data) {
        side.aig.output_index(port).ok_or_else(|| StreamError::Port(port.clone()))?;
    }
    let handshakes = [side.aig.output_index(&p.in_ready).expect("checked"), side.aig.output_index(&p.out_valid).expect("checked")];
    let cone = side.aig.sequential_cone(&handshakes);
    let g = &cone;
    let results: HashSet<&str> = side.units.iter().flat_map(|u| u.results.iter().map(String::as_str)).collect();
    let data: HashSet<&str> = p.in_data.iter().map(String::as_str).collect();
    let mut m = Aig::new();
    let mut shared: std::collections::HashMap<String, AigLit> = std::collections::HashMap::new();
    let mut state: Vec<AigLit> = g
        .latches()
        .iter()
        .map(|l| match l.init {
            Init::Zero => AigLit::FALSE,
            Init::One => AigLit::TRUE,
            Init::Free => m.add_input(None),
        })
        .collect();
    let (mut ready, mut valid) = (Vec::new(), Vec::new());
    let (ri, vi) = (0, 1);
    for k in 0..env.frames {
        let frame_inputs: Vec<AigLit> = g
            .inputs()
            .iter()
            .map(|port| {
                let Some(name) = port.name.as_deref() else { return m.add_input(None) };
                let constant = |b: bool| if b { AigLit::TRUE } else { AigLit::FALSE };
                if name == p.in_valid {
                    constant((env.offer)(k))
                } else if p.out_ready.as_deref() == Some(name) {
                    constant((env.accept)(k))
                } else if data.contains(name) || results.contains(name) {
                    m.add_input(None)
                } else {
                    match (env.other)(name, k) {
                        Binding::Const(b) => constant(b),
                        Binding::Shared(symbol) => *shared.entry(symbol.clone()).or_insert_with(|| m.add_input(Some(symbol))),
                        Binding::Fresh => m.add_input(None),
                    }
                }
            })
            .collect();
        let map = m.import(g, &mut |leaf| match leaf {
            Leaf::Input(i) => frame_inputs[i],
            Leaf::Latch(j) => state[j],
        });
        let at = |l: AigLit| map[l.node() as usize].with_phase(l.is_complemented());
        ready.push(at(g.outputs()[ri].lit));
        valid.push(at(g.outputs()[vi].lit));
        state = g.latches().iter().map(|l| at(l.next)).collect();
    }
    Ok(Alone { aig: m, ready, valid })
}

/// The constant each handshake is taken to be, from random simulation of the side alone: `None` when the simulation
/// already shows it moving with the data.
fn guesses(a: &Alone, seed: u64) -> Vec<Option<bool>> {
    let words = 8;
    let mut state = seed | 1;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let inputs: Vec<u64> = (0..a.aig.inputs().len() * words).map(|_| next()).collect();
    let values = a.aig.simulate(words, &inputs, &[]);
    a.ready
        .iter()
        .chain(&a.valid)
        .map(|&l| {
            let word = |w: usize| Aig::word(&values, words, l, w);
            match (0..words).map(word).collect::<Vec<_>>() {
                v if v.iter().all(|&x| x == 0) => Some(false),
                v if v.iter().all(|&x| x == !0) => Some(true),
                _ => None,
            }
        })
        .collect()
}

/// The handshakes paired with the constants they are taken to be.
fn handshake_pairs(a: &Alone, guessed: &[bool]) -> Vec<(AigLit, AigLit)> {
    a.ready.iter().chain(&a.valid).zip(guessed).map(|(&l, &b)| (l, if b { AigLit::TRUE } else { AigLit::FALSE })).collect()
}

/// The handshakes a schedule rests on, as indices into `ready` then `valid`: the input's ready where a token is
/// offered, the output's valid where one is taken.
fn relevant(env: &Environment, n: usize) -> Vec<usize> {
    (0..n).filter(|&k| (env.offer)(k)).chain((0..n).filter(|&k| (env.accept)(k)).map(|k| n + k)).collect()
}

/// A side's schedule under its environment, every handshake it rests on proven the constant it is.
pub fn schedule(side: &StreamSide, env: &Environment, opts: &fraig::Options) -> Result<Schedule, StreamError> {
    let a = alone(side, env)?;
    let n = a.ready.len();
    let name = |i: usize| if i < n { (side.ports.in_ready.clone(), i) } else { (side.ports.out_valid.clone(), i - n) };
    let all = guesses(&a, opts.seed);
    let mut guessed = vec![false; 2 * n];
    let used = relevant(env, n);
    for &i in &used {
        guessed[i] = all[i].ok_or_else(|| {
            let (port, frame) = name(i);
            StreamError::DataDependent { port, frame }
        })?;
    }
    let pairs = handshake_pairs(&a, &guessed);
    let asked: Vec<(AigLit, AigLit)> = used.iter().map(|&i| pairs[i]).collect();
    let (verdicts, proof, _) = fraig::decide(&a.aig, &asked, opts);
    for (&i, v) in used.iter().zip(&verdicts) {
        let (port, frame) = name(i);
        match v {
            Verdict::Equal => {}
            Verdict::Differ { .. } => return Err(StreamError::DataDependent { port, frame }),
            Verdict::Unknown(_) => return Err(StreamError::Undecided { port, frame }),
        }
    }
    let took = (0..n).filter(|&k| (env.offer)(k) && guessed[k]).collect();
    let gave = (0..n).filter(|&k| (env.accept)(k) && guessed[n + k]).collect();
    Ok(Schedule { took, gave, proof })
}

impl Schedule {
    /// The clocks are in the order the tokens pass (token j the j-th taken, the j-th given), and the derivation replays
    /// from the side unrolled alone and proves every handshake the constant the schedule says.
    pub fn check(&self, side: &StreamSide, env: &Environment) -> bool {
        if !self.took.windows(2).all(|w| w[0] < w[1]) || !self.gave.windows(2).all(|w| w[0] < w[1]) {
            return false;
        }
        let Ok(a) = alone(side, env) else { return false };
        let n = a.ready.len();
        let took: HashSet<usize> = self.took.iter().copied().collect();
        let gave: HashSet<usize> = self.gave.iter().copied().collect();
        // The constants the schedule says, where it rests on them: taken where a token was offered, given where one was
        // taken.
        let guessed: Vec<bool> = (0..n).map(|k| (env.offer)(k) && took.contains(&k)).chain((0..n).map(|k| (env.accept)(k) && gave.contains(&k))).collect();
        if self.took.iter().any(|&k| k >= n || !(env.offer)(k)) || self.gave.iter().any(|&k| k >= n || !(env.accept)(k)) {
            return false;
        }
        let known = relevant(env, n);
        let pairs = handshake_pairs(&a, &guessed);
        let claims: Vec<Vec<crate::cdcl::Lit>> =
            known.iter().filter(|&&i| pairs[i].0 != pairs[i].1).flat_map(|&i| fraig::claim(pairs[i].0, pairs[i].1)).collect();
        lrat::check_derivation(a.aig.num_nodes(), &fraig::formula(&a.aig), &self.proof, &claims)
    }
}

/// A decision of two streams: a verdict per claimed output bit (`out<k>.<i>`), each side's schedule, the units' rounds,
/// and how many tokens each side gave in the clocks unrolled.
#[derive(Clone, Debug)]
pub struct Report {
    pub verdicts: Vec<(String, Verdict)>,
    pub schedules: [Schedule; 2],
    pub units: units::Report,
    pub gave: [usize; 2],
}

impl Report {
    pub fn all_equal(&self) -> bool {
        self.verdicts.iter().all(|(_, v)| *v == Verdict::Equal)
    }
}

/// What a side's input is at a clock once its schedule is known: the environment's handshake, the token it takes there,
/// a fresh value where it takes none, every other input as the environment holds it.
fn bound<'a>(side: &'a StreamSide<'a>, env: &'a Environment<'a>, took: &'a [usize]) -> impl Fn(&str, usize) -> Binding + 'a {
    let data: HashSet<String> = side.ports.in_data.iter().cloned().collect();
    move |name: &str, k: usize| {
        if name == side.ports.in_valid {
            Binding::Const((env.offer)(k))
        } else if side.ports.out_ready.as_deref() == Some(name) {
            Binding::Const((env.accept)(k))
        } else if data.contains(name) {
            match took.iter().position(|&t| t == k) {
                Some(j) => (side.ports.token)(j, name),
                None => Binding::Fresh,
            }
        } else {
            (env.other)(name, k)
        }
    }
}

/// The output bits a side claims: its k-th given token's bits, for every token both sides give.
fn claims(side: &StreamSide, gave: &[usize], tokens: usize) -> Vec<Claim> {
    gave[..tokens]
        .iter()
        .enumerate()
        .flat_map(|(k, &frame)| side.ports.out_data.iter().enumerate().map(move |(i, bit)| Claim { name: format!("out{k}.{i}"), output: bit.clone(), frame }))
        .collect()
}

/// Decide `left` against `right`, each under its environment, over their units, the functions of `functions` (each a
/// left function, the right one proven equal to it) the only ones identified.
pub fn decide(
    left: &StreamSide,
    left_env: &Environment,
    right: &StreamSide,
    right_env: &Environment,
    functions: &[(String, String)],
    opts: &fraig::Options,
) -> Result<Report, StreamError> {
    if left.ports.out_data.len() != right.ports.out_data.len() {
        return Err(StreamError::Port(format!("{} output bits against {}", left.ports.out_data.len(), right.ports.out_data.len())));
    }
    let (ls, rs) = (schedule(left, left_env, opts)?, schedule(right, right_env, opts)?);
    let tokens = ls.gave.len().min(rs.gave.len());
    let (lc, rc) = (claims(left, &ls.gave, tokens), claims(right, &rs.gave, tokens));
    let (lt, rt) = (ls.took.clone(), rs.took.clone());
    let (lb, rb) = (bound(left, left_env, &lt), bound(right, right_env, &rt));
    let l = Unrolled { aig: left.aig, frames: left_env.frames, bind: &lb, claims: &lc, units: left.units };
    let r = Unrolled { aig: right.aig, frames: right_env.frames, bind: &rb, claims: &rc, units: right.units };
    let report = units::decide_unrolled(&l, &r, functions, opts).map_err(StreamError::Miter)?;
    let gave = [ls.gave.len(), rs.gave.len()];
    Ok(Report { verdicts: report.verdicts.clone(), schedules: [ls, rs], units: report, gave })
}

impl Report {
    /// Both schedules' derivations replay and the units' rounds check over the sides bound as the schedules say.
    pub fn check(&self, left: &StreamSide, left_env: &Environment, right: &StreamSide, right_env: &Environment, functions: &[(String, String)]) -> bool {
        let [ls, rs] = &self.schedules;
        if !ls.check(left, left_env) || !rs.check(right, right_env) {
            return false;
        }
        let tokens = ls.gave.len().min(rs.gave.len());
        if ls.gave.len() > left_env.frames || rs.gave.len() > right_env.frames {
            return false;
        }
        let (lc, rc) = (claims(left, &ls.gave, tokens), claims(right, &rs.gave, tokens));
        let (lb, rb) = (bound(left, left_env, &ls.took), bound(right, right_env, &rs.took));
        let l = Unrolled { aig: left.aig, frames: left_env.frames, bind: &lb, claims: &lc, units: left.units };
        let r = Unrolled { aig: right.aig, frames: right_env.frames, bind: &rb, claims: &rc, units: right.units };
        self.units.check_unrolled(&l, &r, functions)
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/streams/tests.rs"]
mod tests;
