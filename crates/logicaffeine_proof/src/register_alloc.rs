//! Certified linear-scan register allocation — the matching/Hall reasoner as a compiler back-end.
//!
//! In a basic block (straight-line code) each variable is *live* over a contiguous range of
//! instructions, so the interference graph — variables that are live at the same time — is an
//! **interval graph**, which is perfect. Hence the minimum number of registers needed is exactly the
//! **register pressure**: the most variables simultaneously live (the largest clique). If that fits
//! the physical register count, a one-sweep `interval_sched` colouring assigns registers; if not,
//! the over-pressure point yields `R+1` mutually-live variables — a clique that provably cannot share
//! `R` registers, so at least one *must* spill. The allocation is re-checkable, and the spill is
//! certified by that clique (a Hall/pigeonhole witness) — no trusted solver, and far faster than
//! throwing the colouring at a general SAT/SMT solver.

use crate::interval_sched::{peak_concurrency, schedule_or_overflow, Interval, ScheduleOutcome};

/// A variable's live range over instruction positions `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveRange {
    /// The variable identifier.
    pub var: usize,
    /// First instruction the variable is live.
    pub start: i64,
    /// One past the last instruction the variable is live.
    pub end: i64,
}

impl LiveRange {
    /// Construct a live range for `var` over `[start, end)`.
    pub fn new(var: usize, start: i64, end: i64) -> Self {
        LiveRange { var, start, end }
    }
}

/// The result of allocating physical registers to a basic block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Allocation {
    /// Success: `(var, register)` pairs, every register in `0..registers`, no two
    /// simultaneously-live variables sharing one (re-checkable via [`is_valid_allocation`]).
    Allocated(Vec<(usize, usize)>),
    /// Spilling is unavoidable: the block needs `pressure` registers, and `must_spill` is a set of
    /// pairwise-live variables larger than the register count — a certified proof that they cannot
    /// all be kept in registers (re-checkable via [`is_spill_certificate`]).
    Spill {
        /// Minimum registers the block requires (peak simultaneous liveness).
        pressure: usize,
        /// `> registers` mutually-live variables — the spill certificate.
        must_spill: Vec<usize>,
    },
}

/// The register pressure of a block: the most variables live at once (the fewest registers needed).
pub fn register_pressure(ranges: &[LiveRange]) -> usize {
    let tasks: Vec<Interval> = ranges
        .iter()
        .map(|r| Interval::new(r.start, r.end))
        .collect();
    peak_concurrency(&tasks)
}

/// Allocate `registers` physical registers to a basic block's `ranges`, or certify that spilling is
/// unavoidable. O(n log n) via the interval sweep.
pub fn allocate(ranges: &[LiveRange], registers: usize) -> Allocation {
    let tasks: Vec<Interval> = ranges
        .iter()
        .map(|r| Interval::new(r.start, r.end))
        .collect();
    match schedule_or_overflow(&tasks, registers) {
        ScheduleOutcome::Feasible(reg) => {
            Allocation::Allocated(ranges.iter().zip(reg).map(|(r, m)| (r.var, m)).collect())
        }
        ScheduleOutcome::Infeasible(positions) => Allocation::Spill {
            pressure: peak_concurrency(&tasks),
            must_spill: positions.iter().map(|&i| ranges[i].var).collect(),
        },
    }
}

fn live_overlap(a: &LiveRange, b: &LiveRange) -> bool {
    a.start < b.end && b.start < a.end
}

/// Re-check an allocation: every variable is assigned a register `< registers`, and no two
/// simultaneously-live variables share one.
pub fn is_valid_allocation(
    ranges: &[LiveRange],
    registers: usize,
    reg_of: &[(usize, usize)],
) -> bool {
    if reg_of.len() != ranges.len() {
        return false;
    }
    let reg: std::collections::HashMap<usize, usize> = reg_of.iter().copied().collect();
    if reg.len() != ranges.len() || reg.values().any(|&r| r >= registers) {
        return false;
    }
    for i in 0..ranges.len() {
        for j in (i + 1)..ranges.len() {
            if live_overlap(&ranges[i], &ranges[j])
                && reg.get(&ranges[i].var) == reg.get(&ranges[j].var)
            {
                return false;
            }
        }
    }
    true
}

/// Re-check a spill certificate: the listed variables pairwise interfere (are mutually live) and
/// number more than `registers` — so they cannot all reside in registers at once.
pub fn is_spill_certificate(ranges: &[LiveRange], registers: usize, must_spill: &[usize]) -> bool {
    if must_spill.len() <= registers {
        return false;
    }
    let by_var: std::collections::HashMap<usize, &LiveRange> =
        ranges.iter().map(|r| (r.var, r)).collect();
    must_spill.iter().enumerate().all(|(a, v)| {
        by_var.get(v).is_some_and(|rv| {
            must_spill
                .iter()
                .skip(a + 1)
                .all(|u| by_var.get(u).is_some_and(|ru| live_overlap(rv, ru)))
        })
    })
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/register_alloc/tests.rs"]
mod tests;
