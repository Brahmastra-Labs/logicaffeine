//! `logicaffeine_system::replay` IS `logicaffeine_runtime`'s scheduler, step for step.
//!
//! A Mode-B binary runs its tasks on `replay` — the interpreter's scheduler ported, since the runtime
//! crate never links into a compiled program (invariant I6). A port is only as good as its agreement,
//! so this lock runs the same concurrent programs on both and demands the same observations: every
//! line in the same order, the same values through every pipe and select, the same deadlocks, at every
//! seed. The programs are toy scripts — sends, receives, try-operations, sleeps, launches, stops,
//! awaits, closes and selects over a handful of tasks — generated from a seed, so the corpus reaches
//! interleavings nobody would think to write by hand, and each failure names the program that found it.
//!
//! The runtime side drives each script as a hand-written `Task` state machine (`common/sched_scripts`);
//! the replay side runs it as an `async fn` over the public operations a compiled program calls,
//! drawing a select's winner with `seeded_pick` exactly as the lowering does.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use logicaffeine_system::replay::{self, Arm, Failure, Pipe};

#[path = "common/sched_scripts.rs"]
mod sched_scripts;
use sched_scripts::{generate, program, run_on_runtime, Log, Op, Program, Sel, SCHEDULING_SEEDS};

// ─── The replay side: each script as an `async fn` over the compiled program's operations ───

fn script(
    program: Program,
    which: usize,
    mut chans: Vec<Pipe<i64>>,
    log: Log,
) -> Pin<Box<dyn Future<Output = ()>>> {
    Box::pin(async move {
        let tag = which;
        let mut handles: Vec<replay::Task> = Vec::new();
        for op in program[which].clone() {
            match op {
                Op::Show(n) => log.borrow_mut().push(format!("t{tag} show {n}")),
                Op::NewChan(capacity) => chans.push(Pipe::<i64>::open(capacity).await),
                Op::Send(c, v) if !chans.is_empty() => chans[c % chans.len()].send(v).await,
                Op::TrySend(c, v) if !chans.is_empty() => {
                    let i = c % chans.len();
                    let ok = chans[i].try_send(v).await;
                    log.borrow_mut().push(format!("t{tag} try-send#{i} {ok}"));
                }
                Op::Recv(c) if !chans.is_empty() => {
                    let i = c % chans.len();
                    let got = chans[i]
                        .recv_or_closed()
                        .await
                        .map_or("nothing".to_string(), |v| v.to_string());
                    log.borrow_mut().push(format!("t{tag} recv#{i} {got}"));
                }
                Op::TryRecv(c) if !chans.is_empty() => {
                    let i = c % chans.len();
                    let got = chans[i]
                        .try_recv()
                        .await
                        .map_or("nothing".to_string(), |v| v.to_string());
                    log.borrow_mut().push(format!("t{tag} try-recv#{i} {got}"));
                }
                Op::Sleep(ticks) => replay::sleep(ticks as i64).await,
                Op::Spawn(k) => handles.push(
                    replay::spawn(script(program.clone(), k, chans.clone(), log.clone())).await,
                ),
                Op::Stop(h) if !handles.is_empty() => handles[h % handles.len()].stop().await,
                Op::Await(h) if !handles.is_empty() => {
                    handles[h % handles.len()].join().await;
                }
                Op::Close(c) if !chans.is_empty() => chans[c % chans.len()].close().await,
                Op::Select(arms) => {
                    let arms: Vec<Sel> = arms
                        .into_iter()
                        .filter(|a| matches!(a, Sel::After(_)) || !chans.is_empty())
                        .collect();
                    if arms.is_empty() {
                        continue;
                    }
                    let pipes: Vec<Option<Pipe<i64>>> = arms
                        .iter()
                        .map(|a| match a {
                            Sel::Recv(c) => Some(chans[c % chans.len()]),
                            Sel::After(_) => None,
                        })
                        .collect();
                    let select_arms: Vec<Arm> = arms
                        .iter()
                        .zip(&pipes)
                        .map(|(a, p)| match a {
                            Sel::Recv(_) => p.expect("a receive arm has its pipe").arm(),
                            Sel::After(t) => Arm::after(*t as i64),
                        })
                        .collect();
                    let ready = replay::ready_arms(&select_arms);
                    let pick = (!ready.is_empty())
                        .then(|| ready[logicaffeine_system::concurrency::seeded_pick(ready.len())]);
                    let (arm, got) = replay::select(&select_arms, pick).await;
                    let shown = match pipes[arm] {
                        Some(pipe) => pipe
                            .received_or_closed(got)
                            .map_or("nothing".to_string(), |v| v.to_string()),
                        None => "timeout".to_string(),
                    };
                    log.borrow_mut()
                        .push(format!("t{tag} select arm {arm}: {shown}"));
                }
                _ => {}
            }
        }
    })
}

fn run_on_replay(program: &Program, seed: u64) -> (Vec<String>, &'static str) {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let outcome = futures::executor::block_on(replay::run_seeded(
        seed,
        script(program.clone(), 0, Vec::new(), log.clone()),
    ));
    let outcome = match outcome {
        Ok(()) => "done",
        Err(Failure::Deadlock) => "deadlock",
        Err(Failure::Task) => "task-failed",
    };
    let lines = log.borrow().clone();
    (lines, outcome)
}

fn assert_same(what: &str, program: &Program, seed: u64) -> (Vec<String>, &'static str) {
    let runtime = run_on_runtime(program, seed);
    let replayed = run_on_replay(program, seed);
    assert_eq!(
        runtime, replayed,
        "{what}: the replay executor diverged from the scheduler at seed {seed}.\nprogram: {program:#?}"
    );
    runtime
}

#[test]
fn every_generated_program_runs_identically_on_both() {
    let (mut deadlocks, mut selects, mut timeouts, mut closed_reads, mut lines) = (0, 0, 0, 0, 0);
    for program_seed in 0..3_000u64 {
        let program = generate(program_seed);
        for &seed in SCHEDULING_SEEDS {
            let (log, outcome) =
                assert_same(&format!("generated program {program_seed}"), &program, seed);
            deadlocks += usize::from(outcome == "deadlock");
            selects += log.iter().filter(|l| l.contains(" select arm ")).count();
            timeouts += log.iter().filter(|l| l.ends_with(": timeout")).count();
            closed_reads += log
                .iter()
                .filter(|l| l.contains(" recv#") && l.ends_with(" nothing"))
                .count();
            lines += log.len();
        }
    }
    // The corpus must keep reaching the paths it exists to compare; a generator that stopped
    // producing them would leave this lock green and empty.
    // Floors sit at four fifths of what the corpus reaches today (4,049 deadlocks, 5,550 selects,
    // 4,664 timeouts, 156 closed reads, 28,273 lines).
    assert!(deadlocks >= 3_200, "only {deadlocks} runs deadlocked");
    assert!(selects >= 4_400, "only {selects} selects resolved");
    assert!(timeouts >= 3_700, "only {timeouts} select timeouts fired");
    assert!(
        closed_reads >= 120,
        "only {closed_reads} receives read a closed, drained pipe"
    );
    assert!(
        lines >= 22_000,
        "only {lines} observations in the whole corpus"
    );
}

#[test]
fn a_launched_task_runs_before_its_parent_resumes() {
    // The shape of the Studio's `tasks` example: each launch runs the child to completion first.
    let p = program(vec![
        vec![
            Op::Spawn(1),
            Op::Show(1),
            Op::Spawn(2),
            Op::Spawn(1),
            Op::Stop(2),
            Op::Show(2),
        ],
        vec![Op::Show(10)],
        vec![Op::Show(20)],
    ]);
    let (log, outcome) = assert_same("launch order", &p, 0);
    assert_eq!(
        log,
        [
            "t1 show 10",
            "t0 show 1",
            "t2 show 20",
            "t1 show 10",
            "t0 show 2"
        ]
    );
    assert_eq!(outcome, "done");
}

#[test]
fn a_rendezvous_hands_each_value_over_in_order() {
    let p = program(vec![
        vec![
            Op::NewChan(Some(0)),
            Op::Spawn(1),
            Op::Recv(0),
            Op::Recv(0),
            Op::Recv(0),
        ],
        vec![
            Op::Send(0, 1),
            Op::Show(1),
            Op::Send(0, 2),
            Op::Show(2),
            Op::Send(0, 3),
            Op::Show(3),
        ],
    ]);
    for &seed in SCHEDULING_SEEDS {
        let (log, _) = assert_same("rendezvous", &p, seed);
        let received: Vec<&String> = log.iter().filter(|l| l.starts_with("t0 recv")).collect();
        assert_eq!(received, ["t0 recv#0 1", "t0 recv#0 2", "t0 recv#0 3"]);
    }
}

#[test]
fn a_select_race_is_decided_by_the_seed_alone() {
    let p = program(vec![vec![
        Op::NewChan(None),
        Op::NewChan(None),
        Op::Send(0, 1),
        Op::Send(1, 2),
        Op::Select(vec![Sel::Recv(0), Sel::Recv(1)]),
    ]]);
    let mut winners = std::collections::BTreeSet::new();
    for seed in 0..64 {
        let (log, _) = assert_same("select race", &p, seed);
        winners.insert(log.last().cloned().expect("the select logs its winner"));
    }
    assert_eq!(
        winners.len(),
        2,
        "both arms must win at some seed: {winners:?}"
    );
}

#[test]
fn timers_due_on_the_same_tick_wake_in_the_seeds_order() {
    let p = program(vec![
        vec![Op::Spawn(1), Op::Spawn(2), Op::Spawn(3)],
        vec![Op::Sleep(2), Op::Show(1)],
        vec![Op::Sleep(2), Op::Show(2)],
        vec![Op::Sleep(2), Op::Show(3)],
    ]);
    let mut orders = std::collections::BTreeSet::new();
    for seed in 0..64 {
        let (log, _) = assert_same("timer tie-break", &p, seed);
        orders.insert(log.join(","));
    }
    assert!(
        orders.len() > 1,
        "a same-tick tie must break by the seed: {orders:?}"
    );
}

#[test]
fn a_select_times_out_on_the_logical_clock() {
    let p = program(vec![
        vec![
            Op::NewChan(None),
            Op::Spawn(1),
            Op::Select(vec![Sel::Recv(0), Sel::After(3)]),
            Op::Show(9),
        ],
        vec![Op::Sleep(5), Op::Send(0, 7)],
    ]);
    let (log, outcome) = assert_same("select timeout", &p, 0);
    assert_eq!(log, ["t0 select arm 1: timeout", "t0 show 9"]);
    assert_eq!(outcome, "done");
}

#[test]
fn closing_a_pipe_wakes_its_receivers_with_nothing() {
    let p = program(vec![
        vec![Op::NewChan(None), Op::Spawn(1), Op::Spawn(2), Op::Close(0)],
        vec![Op::Recv(0)],
        vec![Op::Select(vec![Sel::Recv(0), Sel::After(9)])],
    ]);
    let (log, outcome) = assert_same("close", &p, 0);
    assert_eq!(log, ["t1 recv#0 nothing", "t2 select arm 0: nothing"]);
    assert_eq!(outcome, "done");
}

#[test]
fn a_stopped_task_never_runs_again_and_its_awaiter_wakes() {
    let p = program(vec![
        vec![
            Op::NewChan(None),
            Op::Spawn(1),
            Op::Spawn(2),
            Op::Stop(0),
            Op::Show(1),
            Op::Send(0, 5),
        ],
        vec![Op::Recv(0), Op::Show(10)],
        vec![Op::Show(20)],
    ]);
    let (log, outcome) = assert_same("stop", &p, 0);
    assert_eq!(log, ["t2 show 20", "t0 show 1"]);
    assert_eq!(outcome, "done");
}

#[test]
fn every_task_blocked_is_a_deadlock_on_both() {
    let p = program(vec![vec![Op::NewChan(None), Op::Recv(0), Op::Show(1)]]);
    let (log, outcome) = assert_same("deadlock", &p, 0);
    assert!(log.is_empty());
    assert_eq!(outcome, "deadlock");
}
