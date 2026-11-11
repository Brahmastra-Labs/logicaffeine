//! **Model checking by a race of certified engines**, deterministic. Property-directed reachability
//! ([`crate::pdr`]) and bounded model checking ([`crate::itp::bmc`]) search one circuit side by side on threads of
//! their own, each in slices that a word from the other can end, the first to answer stopping the other. The answer
//! never depends on which finished first: only PDR proves a circuit safe, its invariant the one PDR alone finds; and an
//! unsafe circuit is answered with BMC's run, the shortest there is, BMC asking clocks on after the race up to the
//! length of PDR's run when PDR found one first. Each certificate is held to its check, an invariant refuted three
//! ways, a run replayed. Both engines run over the circuit cut to its bad states' and constraints' cone of influence,
//! the answer taken back to the whole circuit and held to its checks, as [`crate::pdr::prove_safe`] holds its own. BMC
//! goes no deeper than PDR's frame bound, and the race answers [`Verdict::Unknown`] once PDR has stopped without a
//! verdict and BMC has nothing left to ask.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::aig::Aig;
use crate::itp::{Bmc, Trace};
use crate::pdr::{self, cone, whole, Search, Stats, Verdict};

/// How [`prove_safe`] races.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// PDR's bounds; BMC asks no more than `pdr.max_frames` clocks.
    pub pdr: pdr::Options,
    /// The obligations a PDR slice takes up, and the clocks a BMC slice asks, between looks at the other's word.
    pub pdr_slice: usize,
    pub bmc_slice: usize,
}

impl Default for Options {
    fn default() -> Options {
        Options { pdr: pdr::Options::default(), pdr_slice: 64, bmc_slice: 1 }
    }
}

/// The engine whose certificate answers: PDR for a safe circuit, BMC for an unsafe one, neither for no verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Pdr,
    Bmc,
    None,
}

/// What [`prove_safe`] found: the verdict, the engine whose certificate it is, and what PDR's search cost as far as it
/// ran (how far that was, against BMC, is the one thing timing decides).
#[derive(Clone, Debug)]
pub struct Outcome {
    pub verdict: Verdict,
    pub engine: Engine,
    pub stats: Stats,
}

/// Decide whether a bad state of `aig` is reachable from its initial states, by the race.
pub fn prove_safe(aig: &Aig, opts: &Options) -> Outcome {
    let (cut, kept) = cone(aig);
    let mut pdr = Search::new(&cut);
    let mut bmc = Bmc::new(&cut);
    let stop = AtomicBool::new(false);
    let bound = opts.pdr.max_frames;
    let (pdr_said, bmc_said) = std::thread::scope(|scope| {
        let (pdr, bmc, stop) = (&mut pdr, &mut bmc, &stop);
        let racing_pdr = scope.spawn(move || loop {
            if let Some(verdict) = pdr.resume(&opts.pdr, opts.pdr_slice.max(1)) {
                if !matches!(verdict, Verdict::Unknown(_)) {
                    stop.store(true, Ordering::Relaxed);
                }
                break Some(verdict);
            }
            if stop.load(Ordering::Relaxed) {
                break None;
            }
        });
        let racing_bmc = scope.spawn(move || loop {
            if bmc.clocks >= bound {
                break None;
            }
            let clocks = opts.bmc_slice.max(1).min(bound - bmc.clocks);
            if let Some(answer) = bmc.resume(clocks) {
                if answer.is_ok() {
                    stop.store(true, Ordering::Relaxed);
                }
                break Some(answer);
            }
            if stop.load(Ordering::Relaxed) {
                break None;
            }
        });
        (racing_pdr.join().expect("PDR's thread ends"), racing_bmc.join().expect("BMC's thread ends"))
    });
    let stats = Stats { latches: kept.len(), ..pdr.stats.clone() };
    let outcome =
        |verdict: Verdict, engine| Outcome { verdict: whole(aig, &kept, verdict), engine, stats: stats.clone() };
    let run_of_bmc = |trace: Trace| outcome(Verdict::Unsafe(trace), Engine::Bmc);
    if let Some(Ok(trace)) = bmc_said {
        return run_of_bmc(trace);
    }
    match pdr_said {
        Some(Verdict::Safe(invariant)) => outcome(Verdict::Safe(invariant), Engine::Pdr),
        Some(Verdict::Induction(k)) => outcome(Verdict::Induction(k), Engine::Pdr),
        Some(Verdict::Unsafe(found)) => match bmc.resume(found.inputs.len().saturating_sub(bmc.clocks)) {
            Some(Ok(trace)) => run_of_bmc(trace),
            _ => outcome(Verdict::Unknown("BMC finds no run as short as PDR's".into()), Engine::None),
        },
        _ => {
            while bmc.clocks < bound {
                let clocks = opts.bmc_slice.max(1).min(bound - bmc.clocks);
                match bmc.resume(clocks) {
                    Some(Ok(trace)) => return run_of_bmc(trace),
                    Some(Err(())) => break,
                    None => {}
                }
            }
            outcome(Verdict::Unknown("no verdict within the bounds".into()), Engine::None)
        }
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/mc/tests.rs"]
mod tests;
