//! Interval scheduling in O(n log n) via a sweep line — the matching/Hall reasoner reaching into
//! resource allocation over time.
//!
//! Given `n` tasks, each an interval `[start, end)`, and `m` interchangeable machines (or green
//! windows, or registers), can every task run with no two overlapping tasks on one machine? This is
//! colouring the tasks' **interval graph** (overlap = edge), and interval graphs are *perfect*, so
//! the chromatic number equals the largest clique — here, the maximum number of tasks overlapping at
//! any instant. Hence it is feasible **iff peak overlap ≤ m**, decided by one sweep over the
//! endpoints. When it overflows, the `m+1` tasks active at the peak are a clique that needs `m+1`
//! machines — a Hall/pigeonhole certificate, exactly the structure Z3 grinds in the colouring
//! encoding but we settle in near-linear time. Both outcomes are independently re-checkable.

/// A half-open task interval `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    /// Inclusive start.
    pub start: i64,
    /// Exclusive end.
    pub end: i64,
}

impl Interval {
    /// Construct `[start, end)`.
    pub fn new(start: i64, end: i64) -> Self {
        Interval { start, end }
    }
}

/// The outcome of scheduling onto `m` machines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScheduleOutcome {
    /// Every task placed: `assignment[i]` is task `i`'s machine in `0..m` (re-checkable via
    /// [`is_valid_schedule`]).
    Feasible(Vec<usize>),
    /// No schedule exists, witnessed by a set of mutually-overlapping tasks larger than `m`
    /// (re-checkable via [`is_overflow_witness`]).
    Infeasible(Vec<usize>),
}

/// Two half-open intervals overlap iff each starts before the other ends.
#[inline]
fn overlaps(a: &Interval, b: &Interval) -> bool {
    a.start < b.end && b.start < a.end
}

/// Decide whether `tasks` can be scheduled on `machines` machines. Sweeps the endpoints to find the
/// peak overlap: feasible iff that peak is ≤ `machines`, with a greedy interval colouring as the
/// assignment, else the overflowing overlap-clique as the certificate.
pub fn schedule_or_overflow(tasks: &[Interval], machines: usize) -> ScheduleOutcome {
    // Endpoint events; at equal times an end (-1) precedes a start (+1) so touching intervals
    // [a,b) and [b,c) are never counted as simultaneously active.
    let mut events: Vec<(i64, i8, usize)> = Vec::with_capacity(tasks.len() * 2);
    for (i, t) in tasks.iter().enumerate() {
        if t.start < t.end {
            events.push((t.start, 1, i));
            events.push((t.end, -1, i));
        }
    }
    events.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut active: Vec<usize> = Vec::new();
    for (_, delta, idx) in &events {
        if *delta == 1 {
            active.push(*idx);
            if active.len() > machines {
                // Every active task overlaps every other (all contain this instant) — a clique.
                return ScheduleOutcome::Infeasible(active.clone());
            }
        } else {
            active.retain(|&x| x != *idx);
        }
    }
    ScheduleOutcome::Feasible(greedy_assign(tasks, machines))
}

/// The peak number of tasks active simultaneously — the interval graph's clique number / chromatic
/// number, i.e. the fewest machines (or registers) that suffice. One sweep, O(n log n).
pub fn peak_concurrency(tasks: &[Interval]) -> usize {
    let mut events: Vec<(i64, i8)> = Vec::with_capacity(tasks.len() * 2);
    for t in tasks {
        if t.start < t.end {
            events.push((t.start, 1));
            events.push((t.end, -1));
        }
    }
    events.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let (mut cur, mut peak) = (0i64, 0i64);
    for (_, d) in events {
        cur += d as i64;
        peak = peak.max(cur);
    }
    peak as usize
}

/// Greedy interval colouring: tasks in start order each take the lowest machine free by their start.
/// Uses at most the peak-overlap many machines, so when peak ≤ `machines` it always succeeds.
fn greedy_assign(tasks: &[Interval], machines: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..tasks.len()).collect();
    order.sort_by_key(|&i| tasks[i].start);
    let mut free_at = vec![i64::MIN; machines.max(1)];
    let mut assignment = vec![0usize; tasks.len()];
    for &i in &order {
        let m = (0..free_at.len())
            .find(|&m| free_at[m] <= tasks[i].start)
            .unwrap_or(0);
        free_at[m] = tasks[i].end;
        assignment[i] = m;
    }
    assignment
}

/// Re-check a schedule: every assigned machine is in range and no two overlapping tasks share one.
pub fn is_valid_schedule(tasks: &[Interval], machines: usize, assignment: &[usize]) -> bool {
    if assignment.len() != tasks.len() || assignment.iter().any(|&m| m >= machines) {
        return false;
    }
    for i in 0..tasks.len() {
        for j in (i + 1)..tasks.len() {
            if assignment[i] == assignment[j] && overlaps(&tasks[i], &tasks[j]) {
                return false;
            }
        }
    }
    true
}

/// Re-check an overflow witness: all the listed tasks pairwise overlap and there are more than
/// `machines` of them — a clique that cannot be coloured with `machines` colours.
pub fn is_overflow_witness(tasks: &[Interval], machines: usize, witness: &[usize]) -> bool {
    if witness.len() <= machines {
        return false;
    }
    witness.iter().enumerate().all(|(a, &i)| {
        i < tasks.len()
            && witness
                .iter()
                .skip(a + 1)
                .all(|&j| j < tasks.len() && overlaps(&tasks[i], &tasks[j]))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iv(s: i64, e: i64) -> Interval {
        Interval::new(s, e)
    }

    /// Independent O(n²) oracle: peak overlap = max over task starts of how many tasks contain it.
    fn peak_overlap(tasks: &[Interval]) -> usize {
        tasks
            .iter()
            .filter(|t| t.start < t.end)
            .map(|t| {
                tasks
                    .iter()
                    .filter(|o| o.start <= t.start && t.start < o.end)
                    .count()
            })
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn fitting_tasks_are_scheduled() {
        // [0,2),[1,3) overlap (peak 2); 2 machines suffice.
        let tasks = vec![iv(0, 2), iv(1, 3), iv(3, 4)];
        match schedule_or_overflow(&tasks, 2) {
            ScheduleOutcome::Feasible(a) => assert!(is_valid_schedule(&tasks, 2, &a), "{a:?}"),
            o => panic!("expected Feasible, got {o:?}"),
        }
    }

    #[test]
    fn overload_yields_a_clique_witness() {
        // Three intervals all overlap at t≈1; 2 machines cannot hold them.
        let tasks = vec![iv(0, 3), iv(1, 4), iv(2, 5)];
        match schedule_or_overflow(&tasks, 2) {
            ScheduleOutcome::Infeasible(w) => {
                assert!(is_overflow_witness(&tasks, 2, &w), "witness invalid: {w:?}");
                assert!(w.len() >= 3);
            }
            o => panic!("expected Infeasible, got {o:?}"),
        }
    }

    #[test]
    fn touching_intervals_do_not_overlap() {
        // [0,1) and [1,2) just touch — one machine is enough.
        let tasks = vec![iv(0, 1), iv(1, 2)];
        match schedule_or_overflow(&tasks, 1) {
            ScheduleOutcome::Feasible(a) => assert!(is_valid_schedule(&tasks, 1, &a)),
            o => panic!("touching intervals fit on 1 machine, got {o:?}"),
        }
    }

    #[test]
    fn matches_peak_overlap_oracle_on_random_instances() {
        let mut s: u64 = 0x243F6A8885A308D3;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        for _ in 0..500 {
            let n = (next() % 10) as usize + 1;
            let machines = (next() % 5) as usize + 1;
            let tasks: Vec<Interval> = (0..n)
                .map(|_| {
                    let a = (next() % 12) as i64;
                    let len = (next() % 6) as i64 + 1;
                    iv(a, a + len)
                })
                .collect();
            let feasible_oracle = peak_overlap(&tasks) <= machines;
            match schedule_or_overflow(&tasks, machines) {
                ScheduleOutcome::Feasible(a) => {
                    assert!(feasible_oracle, "we said Feasible but peak > m: {tasks:?} m={machines}");
                    assert!(is_valid_schedule(&tasks, machines, &a), "invalid schedule {a:?}");
                }
                ScheduleOutcome::Infeasible(w) => {
                    assert!(!feasible_oracle, "we said Infeasible but peak ≤ m: {tasks:?} m={machines}");
                    assert!(is_overflow_witness(&tasks, machines, &w), "bogus witness {w:?}");
                }
            }
        }
    }

    #[test]
    fn empty_is_feasible() {
        assert!(matches!(schedule_or_overflow(&[], 3), ScheduleOutcome::Feasible(_)));
    }
}
