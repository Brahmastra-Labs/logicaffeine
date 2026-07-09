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
    let tasks: Vec<Interval> = ranges.iter().map(|r| Interval::new(r.start, r.end)).collect();
    peak_concurrency(&tasks)
}

/// Allocate `registers` physical registers to a basic block's `ranges`, or certify that spilling is
/// unavoidable. O(n log n) via the interval sweep.
pub fn allocate(ranges: &[LiveRange], registers: usize) -> Allocation {
    let tasks: Vec<Interval> = ranges.iter().map(|r| Interval::new(r.start, r.end)).collect();
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
pub fn is_valid_allocation(ranges: &[LiveRange], registers: usize, reg_of: &[(usize, usize)]) -> bool {
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
mod tests {
    use super::*;

    fn lr(var: usize, start: i64, end: i64) -> LiveRange {
        LiveRange::new(var, start, end)
    }

    #[test]
    fn low_pressure_block_allocates() {
        //  v0:[0,2)  v1:[1,3)  v2:[3,5)  — peak pressure 2 (v0,v1 overlap), 2 registers fit.
        let ranges = vec![lr(0, 0, 2), lr(1, 1, 3), lr(2, 3, 5)];
        assert_eq!(register_pressure(&ranges), 2);
        match allocate(&ranges, 2) {
            Allocation::Allocated(reg_of) => {
                assert!(is_valid_allocation(&ranges, 2, &reg_of), "{reg_of:?}")
            }
            o => panic!("expected Allocated, got {o:?}"),
        }
    }

    #[test]
    fn high_pressure_block_must_spill_with_a_clique() {
        // Four variables all live across instruction 2; only 3 registers ⇒ one must spill.
        let ranges = vec![lr(0, 0, 5), lr(1, 1, 6), lr(2, 2, 7), lr(3, 2, 8)];
        assert_eq!(register_pressure(&ranges), 4);
        match allocate(&ranges, 3) {
            Allocation::Spill { pressure, must_spill } => {
                assert_eq!(pressure, 4, "needs 4 registers");
                assert!(must_spill.len() > 3, "clique must exceed register count");
                assert!(is_spill_certificate(&ranges, 3, &must_spill), "{must_spill:?}");
            }
            o => panic!("expected Spill, got {o:?}"),
        }
    }

    #[test]
    fn exactly_at_pressure_fits() {
        // Pressure 3, exactly 3 registers ⇒ allocates (no spill).
        let ranges = vec![lr(0, 0, 5), lr(1, 1, 6), lr(2, 2, 7)];
        assert_eq!(register_pressure(&ranges), 3);
        assert!(matches!(allocate(&ranges, 3), Allocation::Allocated(_)));
    }

    #[test]
    fn matches_pressure_oracle_on_random_blocks() {
        let mut s: u64 = 0xC2B2AE3D27D4EB4F;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        for _ in 0..500 {
            let n = (next() % 10) as usize + 1;
            let registers = (next() % 5) as usize + 1;
            let ranges: Vec<LiveRange> = (0..n)
                .map(|v| {
                    let a = (next() % 12) as i64;
                    let len = (next() % 6) as i64 + 1;
                    lr(v, a, a + len)
                })
                .collect();
            let pressure = register_pressure(&ranges);
            match allocate(&ranges, registers) {
                Allocation::Allocated(reg_of) => {
                    assert!(pressure <= registers, "Allocated but pressure {pressure} > {registers}");
                    assert!(is_valid_allocation(&ranges, registers, &reg_of), "invalid: {reg_of:?}");
                }
                Allocation::Spill { pressure: p, must_spill } => {
                    assert!(pressure > registers, "Spill but pressure {pressure} ≤ {registers}");
                    assert_eq!(p, pressure);
                    assert!(is_spill_certificate(&ranges, registers, &must_spill), "{must_spill:?}");
                }
            }
        }
    }

    #[test]
    fn robustness_edge_cases() {
        // Single variable fits one register.
        assert!(matches!(allocate(&[lr(0, 0, 5)], 1), Allocation::Allocated(_)));
        // Disjoint live ranges all share one register.
        let disjoint = vec![lr(0, 0, 2), lr(1, 2, 4), lr(2, 4, 6)];
        assert_eq!(register_pressure(&disjoint), 1);
        match allocate(&disjoint, 1) {
            Allocation::Allocated(a) => assert!(is_valid_allocation(&disjoint, 1, &a), "{a:?}"),
            o => panic!("disjoint ranges fit 1 register: {o:?}"),
        }
        // Six identical ranges all mutually interfere → spill over 4 registers, clique of 5.
        let same: Vec<LiveRange> = (0..6).map(|v| lr(v, 0, 10)).collect();
        assert_eq!(register_pressure(&same), 6);
        match allocate(&same, 4) {
            Allocation::Spill { pressure, must_spill } => {
                assert_eq!(pressure, 6);
                assert_eq!(must_spill.len(), 5, "the first overflow is registers+1 wide");
                assert!(is_spill_certificate(&same, 4, &must_spill));
            }
            o => panic!("6 identical ranges over 4 registers must spill: {o:?}"),
        }
        // Huge coordinates do not overflow the sweep.
        let big = vec![lr(0, 0, i64::MAX / 2), lr(1, 1, i64::MAX / 2)];
        assert_eq!(register_pressure(&big), 2);
        assert!(matches!(allocate(&big, 2), Allocation::Allocated(_)));
        // Zero variables is trivially allocatable.
        assert!(matches!(allocate(&[], 3), Allocation::Allocated(_)));
    }

    #[test]
    fn a_bad_spill_certificate_is_rejected() {
        let ranges = vec![lr(0, 0, 5), lr(1, 1, 6), lr(2, 2, 7), lr(3, 2, 8)];
        assert!(!is_spill_certificate(&ranges, 3, &[0, 1]), "two vars don't exceed 3 registers");
        // v2 and a far-future non-overlapping var would not pairwise interfere.
        assert!(is_spill_certificate(&ranges, 3, &[0, 1, 2, 3]), "all four are mutually live");
    }
}
