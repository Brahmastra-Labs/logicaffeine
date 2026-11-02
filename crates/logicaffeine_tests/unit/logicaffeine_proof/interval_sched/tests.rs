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
                assert!(
                    feasible_oracle,
                    "we said Feasible but peak > m: {tasks:?} m={machines}"
                );
                assert!(
                    is_valid_schedule(&tasks, machines, &a),
                    "invalid schedule {a:?}"
                );
            }
            ScheduleOutcome::Infeasible(w) => {
                assert!(
                    !feasible_oracle,
                    "we said Infeasible but peak ≤ m: {tasks:?} m={machines}"
                );
                assert!(
                    is_overflow_witness(&tasks, machines, &w),
                    "bogus witness {w:?}"
                );
            }
        }
    }
}

#[test]
fn empty_is_feasible() {
    assert!(matches!(
        schedule_or_overflow(&[], 3),
        ScheduleOutcome::Feasible(_)
    ));
}
