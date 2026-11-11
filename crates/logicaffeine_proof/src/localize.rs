//! **Localization abstraction** around property-directed reachability (McMillan & Amla, *Automatic abstraction without
//! counterexamples*, TACAS 2003; Eén, Mishchenko & Amla, *A single-instance incremental SAT formulation of proof- and
//! counterexample-based abstraction*, FMCAD 2010). A large circuit is model-checked on the part of it a proof needs:
//! the latches outside the abstraction become inputs, free at every clock, so every run of the circuit is a run of the
//! abstraction and an invariant or a k-induction of the abstraction is one of the circuit.
//!
//! The abstraction is found by bounded model checking with every latch tied to its next state by a literal of its own
//! ([`crate::itp`]'s tied unrolling): each clock refuted under every tie names the latches its refutation used, and the
//! abstraction keeps them. Once the latches kept stay the same over [`Options::stable`] clocks, or a clock outruns
//! [`Options::conflicts`], [`crate::pdr`] decides the abstraction. A cone of at most [`Options::direct`] latches is
//! decided by PDR whole. A run of the abstraction may break the circuit's own
//! latches: the tied unrolling then asks the circuit as many clocks as the run took, and either finds the circuit's own
//! run or names the latches that refute it, the abstraction growing by them before it is decided again.
//!
//! Neither verdict is taken on the procedure's word: an invariant of the abstraction is taken back to the whole circuit
//! and held to [`crate::itp::check_invariant`], a k-induction to [`crate::itp::check_induction`], and a run of the
//! circuit to [`crate::itp::replays`].

use crate::aig::{Aig, AigLit, Leaf, Node};
use crate::itp::{check_induction, Bmc, Invariant, Query, Trace, CUT_LEAVES};
use crate::pdr::{self, cone, whole, Stats, Verdict};
use crate::scorr;

/// How [`prove_safe`] abstracts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// The search on each abstraction.
    pub pdr: pdr::Options,
    /// The clocks the latches kept must stay the same over before the abstraction is decided.
    pub stable: usize,
    /// The conflicts a clock of the tied unrolling may take, while the abstraction is being found, before the
    /// abstraction is decided as it stands (0: no limit).
    pub conflicts: u64,
    /// The most latches a cone of influence may have and still be decided by PDR whole, with no abstraction.
    pub direct: usize,
    /// Whether the cone is first reduced by its signal correspondence ([`crate::scorr`]), every signal that equals
    /// another or a constant at every clock merged into it, and the reduced cone decided when the reduction leaves at
    /// most [`CORRESPOND_KEEPS`] percent of its latches: a reduction smaller than that saves PDR less than the search
    /// a changed circuit takes it may cost.
    pub correspond: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options { pdr: pdr::Options::default(), stable: 8, conflicts: 100_000, direct: 1000, correspond: false }
    }
}

/// The most of a cone's latches, in percent, that a reduction by signal correspondence may leave for the reduced cone
/// to be the one decided ([`Options::correspond`]).
pub const CORRESPOND_KEEPS: usize = 90;

/// A verdict, what its last search cost, the latches the last abstraction kept of those in the cone of influence, and
/// how many times a run of the abstraction made it grow.
#[derive(Clone, Debug)]
pub struct Report {
    pub verdict: Verdict,
    pub stats: Stats,
    pub kept: usize,
    pub refinements: usize,
}

/// Decide whether a bad state of `aig` is reachable from its initial states, on abstractions of it. See the module
/// documentation.
pub fn prove_safe(aig: &Aig, opts: &Options) -> Report {
    let (cut, kept) = cone(aig);
    if opts.correspond {
        let corr = scorr::find(&cut, &scorr::Options::default()).filter(|c| c.merged() > 0);
        let reduced = corr.as_ref().map(|c| scorr::reduce(&cut, c));
        let worth = |r: &Aig| cone(r).1.len() * 100 <= cut.latches().len() * CORRESPOND_KEEPS;
        if let (Some(corr), Some(reduced)) = (corr, reduced.filter(worth)) {
            let inner = Options { correspond: false, ..*opts };
            let mut report = prove_safe(&reduced, &inner);
            if let Verdict::Induction(k) = report.verdict {
                if !check_induction(&cut, k) {
                    report = prove_safe(&reduced, &Options { pdr: pdr::Options { induction: 0, ..opts.pdr }, ..inner });
                }
            }
            let verdict = match report.verdict {
                Verdict::Safe(inv) => Verdict::Safe(scorr::invariant(&cut, &corr, &inv)),
                other => other,
            };
            return Report { verdict: whole(aig, &kept, verdict), ..report };
        }
    }
    if cut.latches().len() <= opts.direct {
        let report = pdr::prove_safe(aig, &opts.pdr);
        let kept = report.stats.latches;
        return Report { verdict: report.verdict, stats: report.stats, kept, refinements: 0 };
    }
    let mut bmc = Bmc::tied(&cut, CUT_LEAVES);
    let mut keep = vec![false; cut.latches().len()];
    let done = |verdict: Verdict, stats: Stats, keep: &[bool], refinements: usize| Report {
        verdict: whole(aig, &kept, verdict),
        stats,
        kept: keep.iter().filter(|&&k| k).count(),
        refinements,
    };
    let mut stable = 0;
    while stable < opts.stable {
        match settle(&mut bmc, &mut keep, opts.conflicts) {
            Settled::Refuted(grew) => stable = if grew { 0 } else { stable + 1 },
            Settled::Run(trace) => return done(Verdict::Unsafe(trace), Stats::default(), &keep, 0),
            Settled::Stopped => break,
        }
    }
    let mut refinements = 0;
    loop {
        let (abstraction, latches) = abstraction(&cut, &keep);
        let report = pdr::prove_safe(&abstraction, &opts.pdr);
        let verdict = match report.verdict {
            Verdict::Safe(inv) => Verdict::Safe(lifted(&inv, &latches, cut.latches().len())),
            Verdict::Unsafe(run) => {
                let mut grew = false;
                while bmc.clocks < run.inputs.len() {
                    match settle(&mut bmc, &mut keep, 0) {
                        Settled::Refuted(more) => grew |= more,
                        Settled::Run(trace) => return done(Verdict::Unsafe(trace), report.stats, &keep, refinements),
                        Settled::Stopped => break,
                    }
                }
                if !grew {
                    keep.iter_mut().for_each(|k| *k = true);
                }
                refinements += 1;
                continue;
            }
            other => other,
        };
        return done(verdict, report.stats, &keep, refinements);
    }
}

/// How the clock under way was settled ([`settle`]).
enum Settled {
    /// Refuted under the ties of the latches kept, and whether they had to grow for it.
    Refuted(bool),
    /// A run of the circuit, which replays.
    Run(Trace),
    /// Neither within the budget, or no run of this length or longer is left.
    Stopped,
}

/// Settle the clock under way on the abstraction `keep` marks, within `budget` conflicts a query (0: no limit), and move
/// past it once refuted. A model of the abstraction that is no run of the circuit has the clock asked of the circuit,
/// every tie assumed: a model there is the circuit's run; a refutation names the latches it rests on, cut down by
/// asking again under the latches kept and those, while that names fewer, and the abstraction keeps them.
fn settle(bmc: &mut Bmc, keep: &mut [bool], budget: u64) -> Settled {
    let all = vec![true; keep.len()];
    let mut grew = false;
    loop {
        match bmc.query(budget, keep) {
            Query::Refuted(used) => {
                bmc.advance(&used);
                return Settled::Refuted(grew);
            }
            Query::Model(trace, true) => return Settled::Run(trace),
            Query::Model(_, false) => {}
            Query::Ended | Query::Budget => return Settled::Stopped,
        }
        let mut new: Vec<usize> = match bmc.query(budget, &all) {
            Query::Model(trace, true) => return Settled::Run(trace),
            Query::Refuted(used) => used.into_iter().filter(|&j| !keep[j]).collect(),
            Query::Model(_, false) | Query::Ended | Query::Budget => return Settled::Stopped,
        };
        loop {
            let mut trial = keep.to_vec();
            for &j in &new {
                trial[j] = true;
            }
            match bmc.query(budget, &trial) {
                Query::Refuted(used) => {
                    let fewer: Vec<usize> = used.into_iter().filter(|&j| !keep[j]).collect();
                    if fewer.len() >= new.len() {
                        break;
                    }
                    new = fewer;
                }
                _ => break,
            }
        }
        if new.is_empty() {
            return Settled::Stopped;
        }
        grew |= grow(keep, &new);
    }
}

/// Keep the latches `used` as well: whether any was not kept yet.
fn grow(keep: &mut [bool], used: &[usize]) -> bool {
    let mut grew = false;
    for &j in used {
        grew |= !std::mem::replace(&mut keep[j], true);
    }
    grew
}

/// `aig` with each latch not kept an input, free at every clock: its inputs, then one for each latch not kept, in order;
/// the latches kept, in order, each with its index in `aig` returned beside it; every node; the bad states and the
/// constraints.
fn abstraction(aig: &Aig, keep: &[bool]) -> (Aig, Vec<usize>) {
    let mut out = Aig::new();
    let mut map: Vec<AigLit> = vec![AigLit::FALSE; aig.num_nodes()];
    for p in aig.inputs() {
        map[p.lit.node() as usize] = out.add_input(p.name.clone());
    }
    for (j, latch) in aig.latches().iter().enumerate() {
        if !keep[j] {
            map[latch.node as usize] = out.add_input(latch.name.clone());
        }
    }
    let latches: Vec<usize> = (0..aig.latches().len()).filter(|&j| keep[j]).collect();
    let made: Vec<usize> =
        latches.iter().map(|&j| out.add_latch(aig.latches()[j].name.clone(), aig.latches()[j].init)).collect();
    for (&j, &m) in latches.iter().zip(&made) {
        map[aig.latches()[j].node as usize] = out.latch_lit(m);
    }
    let at = |map: &[AigLit], l: AigLit| map[l.node() as usize].with_phase(l.is_complemented());
    for n in 0..aig.num_nodes() as u32 {
        if let Node::And(a, b) = aig.node(n) {
            map[n as usize] = out.and(at(&map, a), at(&map, b));
        }
    }
    for (&j, &m) in latches.iter().zip(&made) {
        out.set_next(m, at(&map, aig.latches()[j].next));
    }
    let bads = if aig.bad().is_empty() { aig.outputs() } else { aig.bad() };
    for b in bads {
        out.add_bad(at(&map, b.lit), b.name.clone());
    }
    for c in aig.constraints() {
        out.add_constraint(at(&map, c.lit), c.name.clone());
    }
    (out, latches)
}

/// An invariant over an abstraction's latches as one over all `count` latches of the circuit it abstracts, `latches`
/// the circuit's index of each.
fn lifted(inv: &Invariant, latches: &[usize], count: usize) -> Invariant {
    let mut over = Aig::new();
    let all: Vec<AigLit> = (0..count).map(|j| over.add_input(Some(format!("l{j}")))).collect();
    let map = over.import(&inv.aig, &mut |leaf| match leaf {
        Leaf::Input(i) => all[latches[i]],
        Leaf::Latch(_) => unreachable!("an invariant has no latches"),
    });
    let root = map[inv.root.node() as usize].with_phase(inv.root.is_complemented());
    Invariant { aig: over, root }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/localize/tests.rs"]
mod tests;
