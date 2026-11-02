//! Sequential equivalence by unrolling: a pipelined circuit proven equal to its combinational reference at its
//! latency, or two pipelines of different depths proven to compute the same transaction.
//!
//! [`at_latency`] unrolls a circuit's transition relation `latency + 1` times into one combinational graph. The
//! inputs of clock 0 keep their names, the inputs of clock `k ≥ 1` are named `name@k`, a latch with a free initial
//! value becomes an input `latch@init`, and an initialized latch becomes its constant; the outputs read at clock
//! `latency` keep their names. Equality of those outputs with a reference over the clock-0 inputs, for every initial
//! state and every later input, says exactly that the circuit is a pipeline of that latency for that function: an
//! output that leans on a stale state, on a later input, or on feedback is told apart by a witness.
//! [`aligned`] does this for both sides at once, each side's initial state its own (a state shared between two
//! designs would hide a difference), and decides the pair with [`crate::sweep`], certificate and all;
//! [`aligned_by_fraig`] decides it by incremental SAT sweeping ([`crate::fraig`]), one derivation for every equality.
//!
//! [`always_equal`] decides two sequential circuits for every clock from reset, by register correspondence (van
//! Eijk, *Sequential equivalence checking based on structural similarities*, TCAD 2000): the two run side by side in
//! one [`Product`]; simulated from reset, their registers fall into classes of equal (or complementary) values; the
//! classes are refined until, assumed of a state, they hold of the next state for every input, each step a refuted
//! difference with an LRAT proof ([`crate::sweep::query`] with the classes as its assumptions); and the outputs are
//! proven equal under them. The [`InductionCertificate`] is that invariant, its steps' proofs and the outputs' proofs;
//! [`check_always`] replays it, including the invariant's truth at reset. When the correspondence is not strong
//! enough, bounded model checking looks for the shortest difference from reset ([`Trace`], replayed by
//! [`replays`]); finding none, the verdict is unknown, never equal.

use std::collections::HashMap;

use crate::aig::{Aig, AigLit, Init, Leaf, Node};
use crate::fraig;
use crate::lrat::{self, Lrat};
use crate::sweep::{self, MiterError, Options, Outcome, Report};

/// The graph unrolled over `frames` clocks, the latches' starting values from their initial values, the outputs of
/// every clock named `name@k`.
pub fn unroll(aig: &Aig, frames: usize) -> Aig {
    let mut out = Aig::new();
    let mut frame = Frames::new(aig, &mut out, "@init");
    for k in 0..frames {
        let map = frame.step(aig, &mut out, k);
        for o in aig.outputs() {
            let lit = at(&map, o.lit);
            out.add_output(lit, o.name.as_ref().map(|n| format!("{n}@{k}")));
        }
        frame.advance(aig, &map);
    }
    out
}

/// The outputs at clock `latency` as a combinational graph over the clock-0 inputs (by their own names), the later
/// clocks' inputs and the free initial values.
pub fn at_latency(aig: &Aig, latency: usize) -> Aig {
    at_latency_with(aig, latency, "@init")
}

fn at_latency_with(aig: &Aig, latency: usize, init: &str) -> Aig {
    let mut out = Aig::new();
    let mut frame = Frames::new(aig, &mut out, init);
    for k in 0..=latency {
        let map = frame.step(aig, &mut out, k);
        if k == latency {
            for o in aig.outputs() {
                let lit = at(&map, o.lit);
                out.add_output(lit, o.name.clone());
            }
        } else {
            frame.advance(aig, &map);
        }
    }
    out
}

fn at(map: &[AigLit], l: AigLit) -> AigLit {
    map[l.node() as usize].with_phase(l.is_complemented())
}

/// The unrolling's running state: the clock-0 inputs, created first so a combinational circuit keeps its order, and
/// each latch's value at the current clock.
struct Frames {
    first: Vec<AigLit>,
    state: Vec<AigLit>,
}

impl Frames {
    fn new(aig: &Aig, out: &mut Aig, init: &str) -> Frames {
        let first = aig.inputs().iter().map(|p| out.add_input(p.name.clone())).collect();
        let state = aig
            .latches()
            .iter()
            .map(|l| match l.init {
                Init::Zero => AigLit::FALSE,
                Init::One => AigLit::TRUE,
                Init::Free => out.add_input(l.name.as_ref().map(|n| format!("{n}{init}"))),
            })
            .collect();
        Frames { first, state }
    }

    /// Copy the circuit at clock `k`; returns the literal each of its nodes became.
    fn step(&mut self, aig: &Aig, out: &mut Aig, k: usize) -> Vec<AigLit> {
        let inputs: Vec<AigLit> = if k == 0 {
            self.first.clone()
        } else {
            aig.inputs().iter().map(|p| out.add_input(p.name.as_ref().map(|n| format!("{n}@{k}")))).collect()
        };
        let state = &self.state;
        out.import(aig, &mut |leaf| match leaf {
            Leaf::Input(i) => inputs[i],
            Leaf::Latch(j) => state[j],
        })
    }

    fn advance(&mut self, aig: &Aig, map: &[AigLit]) {
        self.state = aig.latches().iter().map(|l| at(map, l.next)).collect();
    }
}

/// Whether `left`'s outputs at clock `left_latency` equal `right`'s at clock `right_latency`, as functions of the
/// clock-0 inputs they share by name, for every initial state of each and every later input.
pub fn aligned(
    left: &Aig,
    left_latency: usize,
    right: &Aig,
    right_latency: usize,
    opts: &Options,
) -> Result<Report, MiterError> {
    let (l, r) = aligned_pair(left, left_latency, right, right_latency);
    sweep::equivalent(&l, &r, opts)
}

/// The two combinational circuits [`aligned`] decides between: each side at its latency, its free initial values
/// named apart from the other's (`latch@init.left`, `latch@init.right`).
pub fn aligned_pair(left: &Aig, left_latency: usize, right: &Aig, right_latency: usize) -> (Aig, Aig) {
    (at_latency_with(left, left_latency, "@init.left"), at_latency_with(right, right_latency, "@init.right"))
}

/// Whether `pipeline`'s outputs at clock `latency` equal the combinational `reference` over the clock-0 inputs.
pub fn latency_equivalent(pipeline: &Aig, reference: &Aig, latency: usize, opts: &Options) -> Result<Report, MiterError> {
    aligned(pipeline, latency, reference, 0, opts)
}

/// [`aligned`], decided by [`crate::fraig`].
pub fn aligned_by_fraig(
    left: &Aig,
    left_latency: usize,
    right: &Aig,
    right_latency: usize,
    opts: &fraig::Options,
) -> Result<fraig::Report, MiterError> {
    let (l, r) = aligned_pair(left, left_latency, right, right_latency);
    fraig::equivalent(&l, &r, opts)
}

/// [`latency_equivalent`], decided by [`crate::fraig`].
pub fn latency_equivalent_by_fraig(
    pipeline: &Aig,
    reference: &Aig,
    latency: usize,
    opts: &fraig::Options,
) -> Result<fraig::Report, MiterError> {
    aligned_by_fraig(pipeline, latency, reference, 0, opts)
}

/// Two sequential circuits run side by side: inputs of the same name shared, every register of each with its initial
/// value, each named output of the left paired with the right's, and an output a pair's difference.
#[derive(Clone, Debug)]
pub struct Product {
    pub aig: Aig,
    pub pairs: Vec<(String, AigLit, AigLit)>,
}

/// The two circuits as one [`Product`].
pub fn product(left: &Aig, right: &Aig) -> Result<Product, MiterError> {
    for g in [left, right] {
        let mut seen = std::collections::HashSet::new();
        for o in g.outputs() {
            let name = o.name.as_ref().ok_or(MiterError::Unnamed)?;
            if !seen.insert(name) {
                return Err(MiterError::Duplicate(name.clone()));
            }
        }
    }
    let mut aig = Aig::new();
    let mut by_name: HashMap<String, AigLit> = HashMap::new();
    let left_inputs: Vec<AigLit> = left
        .inputs()
        .iter()
        .map(|p| {
            let l = aig.add_input(p.name.clone());
            if let Some(n) = &p.name {
                by_name.insert(n.clone(), l);
            }
            l
        })
        .collect();
    let right_inputs: Vec<AigLit> = right
        .inputs()
        .iter()
        .map(|p| match p.name.as_ref().and_then(|n| by_name.get(n)) {
            Some(&l) => l,
            None => aig.add_input(p.name.clone()),
        })
        .collect();
    let side = |g: &Aig, inputs: &[AigLit], tag: &str, aig: &mut Aig| -> Vec<AigLit> {
        let base = aig.latches().len();
        let states: Vec<AigLit> = g
            .latches()
            .iter()
            .map(|l| {
                let k = aig.add_latch(l.name.as_ref().map(|n| format!("{tag}.{n}")), l.init);
                aig.latch_lit(k)
            })
            .collect();
        let map = aig.import(g, &mut |leaf| match leaf {
            Leaf::Input(i) => inputs[i],
            Leaf::Latch(j) => states[j],
        });
        for (j, l) in g.latches().iter().enumerate() {
            aig.set_next(base + j, at(&map, l.next));
        }
        map
    };
    let lmap = side(left, &left_inputs, "left", &mut aig);
    let rmap = side(right, &right_inputs, "right", &mut aig);
    let mut pairs = Vec::with_capacity(left.outputs().len());
    for o in left.outputs() {
        let name = o.name.clone().expect("named above");
        let k = right.output_index(&name).ok_or_else(|| MiterError::Unmatched(name.clone()))?;
        pairs.push((name, at(&lmap, o.lit), at(&rmap, right.outputs()[k].lit)));
    }
    if let Some(o) = right.outputs().iter().find(|o| left.output_index(o.name.as_deref().unwrap()).is_none()) {
        return Err(MiterError::Unmatched(o.name.clone().unwrap()));
    }
    for (name, a, b) in &pairs {
        let d = aig.xor(*a, *b);
        aig.add_output(d, Some(name.clone()));
    }
    Ok(Product { aig, pairs })
}

/// How [`always_equal`] searches.
#[derive(Clone, Debug)]
pub struct SeqOptions {
    /// Clocks simulated from reset to draw the register classes from.
    pub sim_cycles: usize,
    /// Words of 64 parallel input streams the simulation runs.
    pub sim_words: usize,
    pub seed: u64,
    /// The deepest difference from reset bounded model checking looks for.
    pub bmc_depth: usize,
    /// Conflicts a query may spend; 0 is unlimited.
    pub budget: u64,
}

impl Default for SeqOptions {
    fn default() -> SeqOptions {
        SeqOptions { sim_cycles: 64, sim_words: 4, seed: 0x5eed_c0de_f00d_a16e, bmc_depth: 32, budget: 20_000 }
    }
}

/// An assignment from reset under which a pair of outputs differs at the last clock: the registers that start free,
/// by index of the product's registers (the others are read from their initial values), and the inputs of each clock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trace {
    pub init: Vec<bool>,
    pub frames: Vec<Vec<bool>>,
}

/// The proof that two circuits agree at every clock from reset: an invariant of register literal pairs true at reset,
/// each pair's next values proven equal assuming the invariant (one proof a pair, in order), and each output pair
/// proven equal assuming it (one proof an output, in the product's order, absent for an output pair that is one
/// literal).
#[derive(Clone, Debug)]
pub struct InductionCertificate {
    pub invariant: Vec<(AigLit, AigLit)>,
    pub steps: Vec<Lrat>,
    pub outputs: Vec<Option<Lrat>>,
}

#[derive(Clone, Debug)]
pub enum SeqVerdict {
    Equal(InductionCertificate),
    Differ(Trace),
    Unknown(String),
}

#[derive(Clone, Debug)]
pub struct SeqReport {
    pub product: Product,
    pub verdict: SeqVerdict,
}

/// The value a register literal (or a constant) holds before the first clock, or `None` when the register starts free.
fn reset_value(aig: &Aig, l: AigLit) -> Option<bool> {
    let v = match aig.node(l.node()) {
        Node::False => false,
        Node::Latch(k) => match aig.latches()[k].init {
            Init::Zero => false,
            Init::One => true,
            Init::Free => return None,
        },
        _ => return None,
    };
    Some(v ^ l.is_complemented())
}

/// The next value of a register literal (a constant's is itself).
fn next(aig: &Aig, l: AigLit) -> AigLit {
    match aig.node(l.node()) {
        Node::Latch(k) => aig.latches()[k].next.with_phase(l.is_complemented()),
        _ => l,
    }
}

/// Whether `cert` proves the product's outputs equal at every clock from reset.
pub fn check_always(product: &Product, cert: &InductionCertificate) -> bool {
    let aig = &product.aig;
    let n = aig.num_nodes();
    let register = |l: AigLit| (l.node() as usize) < n && matches!(aig.node(l.node()), Node::Latch(_) | Node::False);
    if cert.steps.len() != cert.invariant.len() || cert.outputs.len() != product.pairs.len() {
        return false;
    }
    for &(a, b) in &cert.invariant {
        if !register(a) || !register(b) {
            return false;
        }
        match (reset_value(aig, a), reset_value(aig, b)) {
            (Some(x), Some(y)) if x == y => {}
            _ => return false,
        }
    }
    for (&(a, b), proof) in cert.invariant.iter().zip(&cert.steps) {
        let (na, nb) = (next(aig, a), next(aig, b));
        if na == nb {
            continue;
        }
        let q = sweep::query(aig, na, nb, &cert.invariant);
        if !lrat::check(q.num_vars, &q.clauses, proof) {
            return false;
        }
    }
    for ((_, l, r), proof) in product.pairs.iter().zip(&cert.outputs) {
        match proof {
            None if l == r => {}
            None => return false,
            Some(proof) => {
                let q = sweep::query(aig, *l, *r, &cert.invariant);
                if !lrat::check(q.num_vars, &q.clauses, proof) {
                    return false;
                }
            }
        }
    }
    true
}

/// Whether `trace`, run on the product from reset, ends at a clock where some output pair differs.
pub fn replays(product: &Product, trace: &Trace) -> bool {
    let aig = &product.aig;
    if trace.frames.is_empty() || trace.init.len() != aig.latches().len() {
        return false;
    }
    let mut state: Vec<bool> = aig
        .latches()
        .iter()
        .zip(&trace.init)
        .map(|(l, &free)| match l.init {
            Init::Zero => false,
            Init::One => true,
            Init::Free => free,
        })
        .collect();
    for (f, inputs) in trace.frames.iter().enumerate() {
        if inputs.len() != aig.inputs().len() {
            return false;
        }
        let v = aig.eval(inputs, &state);
        if f + 1 == trace.frames.len() {
            return product.pairs.iter().any(|(_, l, r)| Aig::value(&v, *l) != Aig::value(&v, *r));
        }
        state = aig.latches().iter().map(|l| Aig::value(&v, l.next)).collect();
    }
    false
}

/// Register signatures over `cycles` clocks from reset: each register's words, clock after clock.
fn register_signatures(aig: &Aig, opts: &SeqOptions) -> Vec<Vec<u64>> {
    let words = opts.sim_words.max(1);
    let mut rng = opts.seed;
    let mut next_word = || {
        rng = rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    let mut state: Vec<u64> = Vec::with_capacity(aig.latches().len() * words);
    for l in aig.latches() {
        for _ in 0..words {
            state.push(match l.init {
                Init::Zero => 0,
                Init::One => !0,
                Init::Free => next_word(),
            });
        }
    }
    let mut sigs = vec![Vec::with_capacity(opts.sim_cycles * words); aig.num_nodes()];
    for _ in 0..opts.sim_cycles.max(1) {
        let inputs: Vec<u64> = (0..aig.inputs().len() * words).map(|_| next_word()).collect();
        let sim = aig.simulate(words, &inputs, &state);
        for (k, l) in aig.latches().iter().enumerate() {
            sigs[l.node as usize].extend_from_slice(&state[k * words..(k + 1) * words]);
        }
        let mut next_state = Vec::with_capacity(state.len());
        for l in aig.latches() {
            for w in 0..words {
                next_state.push(Aig::word(&sim, words, l.next, w));
            }
        }
        state = next_state;
    }
    sigs
}

/// The register literal pairs simulation suggests: each register paired with the first register (or the constant)
/// whose signature matches its own or its complement, and that starts with a known value.
fn candidates(aig: &Aig, sigs: &[Vec<u64>]) -> Vec<(AigLit, AigLit)> {
    let mut classes: HashMap<Vec<u64>, AigLit> = HashMap::new();
    let len = sigs.iter().map(|s| s.len()).max().unwrap_or(0);
    classes.insert(vec![0; len], AigLit::FALSE);
    let mut pairs = Vec::new();
    for l in aig.latches() {
        if l.init == Init::Free {
            continue;
        }
        let sig = &sigs[l.node as usize];
        let phase = sig.first().is_some_and(|w| w & 1 == 1);
        let key: Vec<u64> = sig.iter().map(|&w| if phase { !w } else { w }).collect();
        let lit = AigLit::new(l.node, false);
        match classes.get(&key) {
            Some(&rep) => pairs.push((lit, rep.with_phase(phase))),
            None => {
                classes.insert(key, lit.with_phase(phase));
            }
        }
    }
    pairs
}

/// Whether the two circuits' outputs agree at every clock from reset.
pub fn always_equal(left: &Aig, right: &Aig, opts: &SeqOptions) -> Result<SeqReport, MiterError> {
    let product = product(left, right)?;
    let verdict = decide_always(&product, opts);
    Ok(SeqReport { product, verdict })
}

fn decide_always(product: &Product, opts: &SeqOptions) -> SeqVerdict {
    let aig = &product.aig;
    let sigs = register_signatures(aig, opts);
    let mut invariant = candidates(aig, &sigs);
    let steps = 'fixpoint: loop {
        let mut steps = Vec::with_capacity(invariant.len());
        for (i, &(a, b)) in invariant.iter().enumerate() {
            let (na, nb) = (next(aig, a), next(aig, b));
            if na == nb {
                steps.push(Lrat { steps: Vec::new() });
                continue;
            }
            match sweep::prove(aig, na, nb, &invariant, opts.budget) {
                Outcome::Equal(proof) => steps.push(proof),
                Outcome::Differ(..) | Outcome::Unknown(_) => {
                    invariant.remove(i);
                    continue 'fixpoint;
                }
            }
        }
        break steps;
    };
    let mut outputs = Vec::with_capacity(product.pairs.len());
    let mut inductive = true;
    for (_, l, r) in &product.pairs {
        if l == r {
            outputs.push(None);
            continue;
        }
        match sweep::prove(aig, *l, *r, &invariant, opts.budget) {
            Outcome::Equal(proof) => outputs.push(Some(proof)),
            _ => {
                inductive = false;
                break;
            }
        }
    }
    if inductive {
        let cert = InductionCertificate { invariant, steps, outputs };
        if check_always(product, &cert) {
            return SeqVerdict::Equal(cert);
        }
        return SeqVerdict::Unknown("the induction certificate did not replay".into());
    }
    bmc(product, opts)
}

/// The shortest difference from reset within `opts.bmc_depth` clocks: the product unrolled once, each clock's
/// differences joined into one literal and searched for a witness, earliest clock first.
fn bmc(product: &Product, opts: &SeqOptions) -> SeqVerdict {
    let aig = &product.aig;
    let free: Vec<usize> = (0..aig.latches().len()).filter(|&k| aig.latches()[k].init == Init::Free).collect();
    let n_in = aig.inputs().len();
    let pairs = product.pairs.len();
    let mut unrolled = unroll(aig, opts.bmc_depth);
    let frames: Vec<AigLit> = (0..opts.bmc_depth)
        .map(|f| {
            let outs: Vec<AigLit> = unrolled.outputs()[f * pairs..(f + 1) * pairs].iter().map(|o| o.lit).collect();
            outs.into_iter().fold(AigLit::FALSE, |acc, d| unrolled.or(acc, d))
        })
        .collect();
    for (f, &any) in frames.iter().enumerate() {
        match sweep::witness(&unrolled, any, opts.budget) {
            Ok(Some((inputs, _))) => {
                let mut init = vec![false; aig.latches().len()];
                for (j, &k) in free.iter().enumerate() {
                    init[k] = inputs[n_in + j];
                }
                let trace_frames = (0..=f)
                    .map(|c| {
                        let base = if c == 0 { 0 } else { n_in + free.len() + (c - 1) * n_in };
                        inputs[base..base + n_in].to_vec()
                    })
                    .collect();
                let trace = Trace { init, frames: trace_frames };
                return if replays(product, &trace) {
                    SeqVerdict::Differ(trace)
                } else {
                    SeqVerdict::Unknown("a difference the product does not replay".into())
                };
            }
            Ok(None) => {}
            Err(why) => return SeqVerdict::Unknown(why),
        }
    }
    SeqVerdict::Unknown(format!(
        "the register correspondence does not prove the outputs, and no difference shows within {} clocks",
        opts.bmc_depth
    ))
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/seq_equiv/tests.rs"]
mod tests;
