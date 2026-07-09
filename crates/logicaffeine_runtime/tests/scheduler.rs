//! Phase 2b (work/FINISH_INTERPRETER.md) — scheduler core tests, against toy tasks.
//!
//! These prove determinism, replay, channel/select/timer semantics, and that
//! each scheduling policy has the right ready-order — all without any interpreter
//! coupling (the toy tasks are hand-written state machines).

use std::cell::RefCell;
use std::rc::Rc;

use logicaffeine_runtime::{
    run_with_seed, run_with_trace, ChanId, Chooser, ChoiceKind, RtPayload, RunOutcome, Scheduler,
    SchedSeed, SchedulePolicy, SchedulerConfig, SelectArm, Task, TaskCtx, TaskId, TaskStateKind,
    TaskStep,
};

type Log = Rc<RefCell<Vec<i64>>>;

// ---- toy tasks ---------------------------------------------------------------

/// Sends `0..n` into `ch`, then exits.
struct Producer {
    ch: ChanId,
    next: i64,
    n: i64,
}
impl<'t> Task<'t> for Producer {
    fn poll(&mut self, _ctx: &mut TaskCtx) -> TaskStep<'t> {
        if self.next < self.n {
            let v = self.next;
            self.next += 1;
            TaskStep::Send(self.ch, RtPayload::Int(v))
        } else {
            TaskStep::Exit(RtPayload::Nothing)
        }
    }
}

/// Sends a single fixed value into `ch`, then exits.
struct OneShot {
    ch: ChanId,
    value: i64,
    done: bool,
}
impl<'t> Task<'t> for OneShot {
    fn poll(&mut self, _ctx: &mut TaskCtx) -> TaskStep<'t> {
        if self.done {
            TaskStep::Exit(RtPayload::Nothing)
        } else {
            self.done = true;
            TaskStep::Send(self.ch, RtPayload::Int(self.value))
        }
    }
}

/// Receives `n` values from `ch`, recording each into `got`.
struct Consumer {
    ch: ChanId,
    remaining: i64,
    got: Log,
    started: bool,
}
impl<'t> Task<'t> for Consumer {
    fn poll(&mut self, ctx: &mut TaskCtx) -> TaskStep<'t> {
        if self.started {
            if let RtPayload::Int(v) = ctx.resumed_with {
                self.got.borrow_mut().push(v);
            }
        }
        self.started = true;
        if self.remaining > 0 {
            self.remaining -= 1;
            TaskStep::Recv(self.ch)
        } else {
            TaskStep::Exit(RtPayload::Nothing)
        }
    }
}

/// Selects over two channels, recording the winning value.
struct Selector {
    a: ChanId,
    b: ChanId,
    log: Log,
    done: bool,
}
impl<'t> Task<'t> for Selector {
    fn poll(&mut self, ctx: &mut TaskCtx) -> TaskStep<'t> {
        if self.done {
            if let RtPayload::Int(v) = ctx.resumed_with {
                self.log.borrow_mut().push(v);
            }
            return TaskStep::Exit(RtPayload::Nothing);
        }
        self.done = true;
        TaskStep::Select(vec![SelectArm::Recv(self.a), SelectArm::Recv(self.b)])
    }
}

/// Sleeps `delay` ticks, then logs `id` and exits.
struct Sleeper {
    delay: u64,
    id: i64,
    log: Log,
    done: bool,
}
impl<'t> Task<'t> for Sleeper {
    fn poll(&mut self, _ctx: &mut TaskCtx) -> TaskStep<'t> {
        if self.done {
            self.log.borrow_mut().push(self.id);
            TaskStep::Exit(RtPayload::Nothing)
        } else {
            self.done = true;
            TaskStep::Sleep(self.delay)
        }
    }
}

/// Logs `id` on its first poll, then exits — reveals ready-pick order.
struct Logger {
    id: i64,
    log: Log,
    prio: u8,
}
impl<'t> Task<'t> for Logger {
    fn poll(&mut self, _ctx: &mut TaskCtx) -> TaskStep<'t> {
        self.log.borrow_mut().push(self.id);
        TaskStep::Exit(RtPayload::Nothing)
    }
    fn priority(&self) -> u8 {
        self.prio
    }
}

/// Logs `id` on every poll, yielding `yields` times before exiting — reveals rotation.
struct YieldLogger {
    id: i64,
    log: Log,
    yields: u8,
}
impl<'t> Task<'t> for YieldLogger {
    fn poll(&mut self, _ctx: &mut TaskCtx) -> TaskStep<'t> {
        self.log.borrow_mut().push(self.id);
        if self.yields > 0 {
            self.yields -= 1;
            TaskStep::Yield
        } else {
            TaskStep::Exit(RtPayload::Nothing)
        }
    }
}

/// Receives once from `ch` (which never gets a value) — deadlocks.
struct Recver {
    ch: ChanId,
}
impl<'t> Task<'t> for Recver {
    fn poll(&mut self, _ctx: &mut TaskCtx) -> TaskStep<'t> {
        TaskStep::Recv(self.ch)
    }
}

// ---- helpers -----------------------------------------------------------------

fn run(config: SchedulerConfig, setup: impl FnOnce(&mut Scheduler)) -> RunOutcome {
    let (outcome, _) = run_with_seed(config, SchedSeed(0), setup);
    outcome
}

fn fifo() -> SchedulerConfig {
    SchedulerConfig::default()
}

// ---- tests -------------------------------------------------------------------

#[test]
fn sched_producer_consumer_fifo() {
    let got: Log = Rc::new(RefCell::new(Vec::new()));
    let g = got.clone();
    let outcome = run(fifo(), move |s| {
        let ch = s.new_default_chan();
        s.spawn(Box::new(Consumer { ch, remaining: 5, got: g, started: false }));
        s.spawn(Box::new(Producer { ch, next: 0, n: 5 }));
    });
    assert_eq!(outcome, RunOutcome::Done(RtPayload::Nothing));
    assert_eq!(*got.borrow(), vec![0, 1, 2, 3, 4], "FIFO order, fully drained");
}

#[test]
fn sched_bounded_channel_blocks_sender() {
    // Capacity 1 forces the producer to block until the consumer drains.
    let got: Log = Rc::new(RefCell::new(Vec::new()));
    let g = got.clone();
    let cfg = SchedulerConfig::default().with_channel_capacity(1);
    let outcome = run(cfg, move |s| {
        let ch = s.new_default_chan(); // capacity 1
        s.spawn(Box::new(Producer { ch, next: 0, n: 4 }));
        s.spawn(Box::new(Consumer { ch, remaining: 4, got: g, started: false }));
    });
    assert_eq!(outcome, RunOutcome::Done(RtPayload::Nothing));
    assert_eq!(*got.borrow(), vec![0, 1, 2, 3], "bounded channel preserves order under backpressure");
}

#[test]
fn sched_select_winner_is_seeded() {
    // Both arms ready; the winner is the seed's choice and is reproducible.
    let scenario = |log: Log| {
        move |s: &mut Scheduler| {
            let a = s.new_default_chan();
            let b = s.new_default_chan();
            s.spawn(Box::new(OneShot { ch: a, value: 1, done: false }));
            s.spawn(Box::new(OneShot { ch: b, value: 2, done: false }));
            s.spawn(Box::new(Selector { a, b, log, done: false }));
        }
    };

    let log1: Log = Rc::new(RefCell::new(Vec::new()));
    let (_, trace) = run_with_seed(fifo(), SchedSeed(3), scenario(log1.clone()));
    let won = log1.borrow()[0];
    assert!(won == 1 || won == 2, "winner is one of the two channel values");
    assert!(
        trace.choices.iter().any(|c| c.kind == ChoiceKind::SelectWinner),
        "the select's winner went through the seeded chooser"
    );

    // Same seed -> same winner.
    let log2: Log = Rc::new(RefCell::new(Vec::new()));
    let _ = run_with_seed(fifo(), SchedSeed(3), scenario(log2.clone()));
    assert_eq!(*log1.borrow(), *log2.borrow(), "same seed reproduces the winner");
}

#[test]
fn sched_timer_wheel_orders_logically() {
    // Sleepers wake in delay order regardless of spawn order; no wall clock.
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let outcome = run(fifo(), move |s| {
        s.spawn(Box::new(Sleeper { delay: 5, id: 5, log: l.clone(), done: false }));
        s.spawn(Box::new(Sleeper { delay: 2, id: 2, log: l.clone(), done: false }));
        s.spawn(Box::new(Sleeper { delay: 8, id: 8, log: l.clone(), done: false }));
    });
    assert_eq!(outcome, RunOutcome::Done(RtPayload::Nothing));
    assert_eq!(*log.borrow(), vec![2, 5, 8], "timers fire in armed (delay) order");
}

#[test]
fn replay_roundtrip_is_bit_identical() {
    let scenario = |log: Log| {
        move |s: &mut Scheduler| {
            let a = s.new_default_chan();
            let b = s.new_default_chan();
            s.spawn(Box::new(OneShot { ch: a, value: 10, done: false }));
            s.spawn(Box::new(OneShot { ch: b, value: 20, done: false }));
            s.spawn(Box::new(Selector { a, b, log, done: false }));
        }
    };

    let log1: Log = Rc::new(RefCell::new(Vec::new()));
    let (o1, trace) = run_with_seed(fifo(), SchedSeed(7), scenario(log1.clone()));

    let log2: Log = Rc::new(RefCell::new(Vec::new()));
    let o2 = run_with_trace(fifo(), trace, scenario(log2.clone()));

    assert_eq!(o1, o2, "replay reproduces the outcome");
    assert_eq!(*log1.borrow(), *log2.borrow(), "replay reproduces the observable output");
}

#[test]
fn seed_sweep_is_reproducible() {
    let scenario = |log: Log| {
        move |s: &mut Scheduler| {
            let a = s.new_default_chan();
            let b = s.new_default_chan();
            s.spawn(Box::new(OneShot { ch: a, value: 1, done: false }));
            s.spawn(Box::new(OneShot { ch: b, value: 2, done: false }));
            s.spawn(Box::new(Selector { a, b, log, done: false }));
        }
    };
    for seed in [0u64, 1, 2, 7, 42] {
        let la: Log = Rc::new(RefCell::new(Vec::new()));
        let _ = run_with_seed(fifo(), SchedSeed(seed), scenario(la.clone()));
        let lb: Log = Rc::new(RefCell::new(Vec::new()));
        let _ = run_with_seed(fifo(), SchedSeed(seed), scenario(lb.clone()));
        assert_eq!(*la.borrow(), *lb.borrow(), "seed {seed} is reproducible");
    }
}

#[test]
fn deadlock_is_deterministic() {
    let outcome = run(fifo(), |s| {
        let ch = s.new_default_chan();
        s.spawn(Box::new(Recver { ch }));
    });
    assert_eq!(outcome, RunOutcome::Deadlock, "a receive with no sender deadlocks");
}

// ---- policy ordering ---------------------------------------------------------

fn loggers(log: &Log, prios: [u8; 3]) -> impl FnOnce(&mut Scheduler) + '_ {
    let log = log.clone();
    move |s: &mut Scheduler| {
        for (i, prio) in prios.iter().enumerate() {
            s.spawn(Box::new(Logger { id: (i as i64) + 1, log: log.clone(), prio: *prio }));
        }
    }
}

#[test]
fn sched_fifo_runs_in_spawn_order() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    run(SchedulerConfig::default().with_policy(SchedulePolicy::Fifo), loggers(&log, [0, 0, 0]));
    assert_eq!(*log.borrow(), vec![1, 2, 3]);
}

#[test]
fn sched_lifo_runs_newest_first() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    run(SchedulerConfig::default().with_policy(SchedulePolicy::Lifo), loggers(&log, [0, 0, 0]));
    assert_eq!(*log.borrow(), vec![3, 2, 1]);
}

#[test]
fn sched_priority_orders_by_priority() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    // ids 1,2,3 with priorities 1,5,2 -> highest first: 2, 3, 1.
    run(SchedulerConfig::default().with_policy(SchedulePolicy::Priority), loggers(&log, [1, 5, 2]));
    assert_eq!(*log.borrow(), vec![2, 3, 1]);
}

#[test]
fn sched_random_is_seed_reproducible() {
    let policy = SchedulePolicy::Random;
    let la: Log = Rc::new(RefCell::new(Vec::new()));
    let (_, _) = run_with_seed(SchedulerConfig::default().with_policy(policy), SchedSeed(99), loggers(&la, [0, 0, 0]));
    let lb: Log = Rc::new(RefCell::new(Vec::new()));
    let (_, _) = run_with_seed(SchedulerConfig::default().with_policy(policy), SchedSeed(99), loggers(&lb, [0, 0, 0]));
    assert_eq!(*la.borrow(), *lb.borrow(), "random policy is reproducible under a fixed seed");
    let mut sorted = la.borrow().clone();
    sorted.sort();
    assert_eq!(sorted, vec![1, 2, 3], "random policy runs each task exactly once");
}

#[test]
fn sched_roundrobin_rotates() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    run(SchedulerConfig::default().with_policy(SchedulePolicy::RoundRobin), move |s| {
        for id in 1..=3 {
            s.spawn(Box::new(YieldLogger { id, log: l.clone(), yields: 1 }));
        }
    });
    assert_eq!(*log.borrow(), vec![1, 2, 3, 1, 2, 3], "round-robin rotates through ready tasks");
}

// ---- task lifecycle: await / abort / close ----------------------------------

struct ExitWith {
    value: i64,
}
impl<'t> Task<'t> for ExitWith {
    fn poll(&mut self, _ctx: &mut TaskCtx) -> TaskStep<'t> {
        TaskStep::Exit(RtPayload::Int(self.value))
    }
}

struct Awaiter {
    target: TaskId,
    log: Log,
    done: bool,
}
impl<'t> Task<'t> for Awaiter {
    fn poll(&mut self, ctx: &mut TaskCtx) -> TaskStep<'t> {
        if self.done {
            let v = match ctx.resumed_with {
                RtPayload::Int(v) => v,
                _ => -1,
            };
            self.log.borrow_mut().push(v);
            return TaskStep::Exit(RtPayload::Nothing);
        }
        self.done = true;
        TaskStep::Await(self.target)
    }
}

struct LongRunner {
    id: i64,
    log: Log,
    ticks: u32,
}
impl<'t> Task<'t> for LongRunner {
    fn poll(&mut self, _ctx: &mut TaskCtx) -> TaskStep<'t> {
        self.log.borrow_mut().push(self.id);
        if self.ticks > 0 {
            self.ticks -= 1;
            TaskStep::Yield
        } else {
            TaskStep::Exit(RtPayload::Nothing)
        }
    }
}

struct Aborter {
    target: TaskId,
    log: Log,
    step: u8,
}
impl<'t> Task<'t> for Aborter {
    fn poll(&mut self, ctx: &mut TaskCtx) -> TaskStep<'t> {
        self.step += 1;
        match self.step {
            1 => TaskStep::Yield,
            2 => TaskStep::Abort(self.target),
            3 => TaskStep::Await(self.target),
            _ => {
                let v = match ctx.resumed_with {
                    RtPayload::Int(v) => v,
                    _ => -1,
                };
                self.log.borrow_mut().push(v);
                TaskStep::Exit(RtPayload::Nothing)
            }
        }
    }
}

struct Closer {
    ch: ChanId,
    done: bool,
}
impl<'t> Task<'t> for Closer {
    fn poll(&mut self, _ctx: &mut TaskCtx) -> TaskStep<'t> {
        if self.done {
            TaskStep::Exit(RtPayload::Nothing)
        } else {
            self.done = true;
            TaskStep::Close(self.ch)
        }
    }
}

struct RecvOnce {
    ch: ChanId,
    log: Log,
    done: bool,
}
impl<'t> Task<'t> for RecvOnce {
    fn poll(&mut self, ctx: &mut TaskCtx) -> TaskStep<'t> {
        if self.done {
            let v = match ctx.resumed_with {
                RtPayload::Int(v) => v,
                _ => -9,
            };
            self.log.borrow_mut().push(v);
            return TaskStep::Exit(RtPayload::Nothing);
        }
        self.done = true;
        TaskStep::Recv(self.ch)
    }
}

struct SelectOne {
    ch: ChanId,
    log: Log,
    done: bool,
}
impl<'t> Task<'t> for SelectOne {
    fn poll(&mut self, ctx: &mut TaskCtx) -> TaskStep<'t> {
        if self.done {
            self.log
                .borrow_mut()
                .push(ctx.selected_arm.map(|a| a as i64).unwrap_or(-1));
            return TaskStep::Exit(RtPayload::Nothing);
        }
        self.done = true;
        TaskStep::Select(vec![SelectArm::Recv(self.ch)])
    }
}

#[test]
fn sched_await_handle_result() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let outcome = run(fifo(), move |s| {
        let target = s.spawn(Box::new(ExitWith { value: 99 }));
        s.spawn(Box::new(Awaiter { target, log: l, done: false }));
    });
    assert_eq!(outcome, RunOutcome::Done(RtPayload::Nothing));
    assert_eq!(*log.borrow(), vec![99], "awaiting a task yields its result");
}

#[test]
fn sched_abort_stops_task_and_await_sees_aborted() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let outcome = run(fifo(), move |s| {
        let target = s.spawn(Box::new(LongRunner { id: 7, log: l.clone(), ticks: 100 }));
        s.spawn(Box::new(Aborter { target, log: l, step: 0 }));
    });
    assert_eq!(outcome, RunOutcome::Done(RtPayload::Nothing), "no deadlock — aborted task is finished");
    let entries = log.borrow();
    let runner_count = entries.iter().filter(|&&x| x == 7).count();
    assert!(runner_count >= 1 && runner_count < 100, "long-runner was stopped early, ran {runner_count} times");
    assert!(entries.contains(&-1), "the awaiter observed the aborted result (Nothing)");
}

#[test]
fn sched_recv_on_closed_channel_returns_nothing() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let outcome = run(fifo(), move |s| {
        let ch = s.new_default_chan();
        s.spawn(Box::new(Closer { ch, done: false }));
        s.spawn(Box::new(RecvOnce { ch, log: l, done: false }));
    });
    assert_eq!(outcome, RunOutcome::Done(RtPayload::Nothing), "recv on a closed channel does not deadlock");
    assert_eq!(*log.borrow(), vec![-9], "recv on a closed empty channel yields Nothing");
}

#[test]
fn sched_select_on_closed_channel_ready() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let outcome = run(fifo(), move |s| {
        let ch = s.new_default_chan();
        s.spawn(Box::new(Closer { ch, done: false }));
        s.spawn(Box::new(SelectOne { ch, log: l, done: false }));
    });
    assert_eq!(outcome, RunOutcome::Done(RtPayload::Nothing));
    assert_eq!(*log.borrow(), vec![0], "a closed channel makes its select recv arm ready");
}

// ---- non-blocking try-send / try-recv ----------------------------------------

/// Tries once to send (non-blocking) into `ch`, logging 1 on success, 0 on failure.
struct TrySender {
    ch: ChanId,
    value: i64,
    log: Log,
    done: bool,
}
impl<'t> Task<'t> for TrySender {
    fn poll(&mut self, ctx: &mut TaskCtx) -> TaskStep<'t> {
        if self.done {
            let ok = matches!(ctx.resumed_with, RtPayload::Bool(true));
            self.log.borrow_mut().push(if ok { 1 } else { 0 });
            return TaskStep::Exit(RtPayload::Nothing);
        }
        self.done = true;
        TaskStep::TrySend(self.ch, RtPayload::Int(self.value))
    }
}

/// Tries once to receive (non-blocking) from `ch`, logging the value or -1 if empty.
struct TryReceiver {
    ch: ChanId,
    log: Log,
    done: bool,
}
impl<'t> Task<'t> for TryReceiver {
    fn poll(&mut self, ctx: &mut TaskCtx) -> TaskStep<'t> {
        if self.done {
            let v = match ctx.resumed_with {
                RtPayload::Int(v) => v,
                _ => -1,
            };
            self.log.borrow_mut().push(v);
            return TaskStep::Exit(RtPayload::Nothing);
        }
        self.done = true;
        TaskStep::TryRecv(self.ch)
    }
}

#[test]
fn sched_try_send_into_full_reports_false() {
    // A capacity-0 rendezvous channel with no waiting receiver has no room.
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let outcome = run(fifo(), move |s| {
        let ch = s.new_chan(Some(0));
        s.spawn(Box::new(TrySender { ch, value: 5, log: l, done: false }));
    });
    assert_eq!(outcome, RunOutcome::Done(RtPayload::Nothing), "try-send never blocks");
    assert_eq!(*log.borrow(), vec![0], "try-send with no room reports false");
}

#[test]
fn sched_try_send_with_room_reports_true() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let outcome = run(fifo(), move |s| {
        let ch = s.new_chan(Some(1));
        s.spawn(Box::new(TrySender { ch, value: 5, log: l, done: false }));
    });
    assert_eq!(outcome, RunOutcome::Done(RtPayload::Nothing));
    assert_eq!(*log.borrow(), vec![1], "try-send with room reports true");
}

#[test]
fn sched_try_send_to_waiting_receiver_succeeds() {
    // A blocked receiver is handed the value directly (rendezvous), so the
    // try-send succeeds even on a capacity-0 channel.
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let rl = log.clone();
    let outcome = run(fifo(), move |s| {
        let ch = s.new_chan(Some(0));
        s.spawn(Box::new(RecvOnce { ch, log: rl, done: false }));
        s.spawn(Box::new(TrySender { ch, value: 7, log: l, done: false }));
    });
    assert_eq!(outcome, RunOutcome::Done(RtPayload::Nothing));
    // The send hands the value to the waiting receiver first (it is re-enqueued
    // before the sender resumes), so the receiver logs the value `7`, then the
    // sender logs its `1` (success). Both facts are proven; the order is the
    // scheduler's deterministic rendezvous discipline.
    assert_eq!(*log.borrow(), vec![7, 1], "try-send to a waiting receiver: receiver gets the value, sender sees success");
}

#[test]
fn sched_try_recv_empty_reports_nothing() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let outcome = run(fifo(), move |s| {
        let ch = s.new_default_chan();
        s.spawn(Box::new(TryReceiver { ch, log: l, done: false }));
    });
    assert_eq!(outcome, RunOutcome::Done(RtPayload::Nothing), "try-recv never blocks");
    assert_eq!(*log.borrow(), vec![-1], "try-recv on an empty channel yields Nothing");
}

#[test]
fn sched_try_recv_with_value_returns_it() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let outcome = run(fifo(), move |s| {
        let ch = s.new_default_chan();
        s.spawn(Box::new(OneShot { ch, value: 42, done: false }));
        s.spawn(Box::new(TryReceiver { ch, log: l, done: false }));
    });
    assert_eq!(outcome, RunOutcome::Done(RtPayload::Nothing));
    assert_eq!(*log.borrow(), vec![42], "try-recv returns a queued value");
}

// ---- Phase 9: stepping + observability ---------------------------------------

/// Driving the scheduler one step at a time (`run_slice`) yields exactly the same outcome,
/// observable output, and scheduling trace as running to quiescence (`run`). This is what
/// lets the browser drive loop yield a macrotask between slices without changing semantics.
#[test]
fn sched_run_slice_equals_run_to_quiescence() {
    // Run-to-quiescence.
    let got_run: Log = Rc::new(RefCell::new(Vec::new()));
    let mut s1 = Scheduler::new(fifo(), Chooser::record(SchedSeed(0)));
    let ch1 = s1.new_default_chan();
    s1.spawn(Box::new(Consumer { ch: ch1, remaining: 5, got: got_run.clone(), started: false }));
    s1.spawn(Box::new(Producer { ch: ch1, next: 0, n: 5 }));
    let out_run = s1.run();
    let trace_run = s1.into_trace();

    // Slice-driven, one step at a time.
    let got_slice: Log = Rc::new(RefCell::new(Vec::new()));
    let mut s2 = Scheduler::new(fifo(), Chooser::record(SchedSeed(0)));
    let ch2 = s2.new_default_chan();
    s2.spawn(Box::new(Consumer { ch: ch2, remaining: 5, got: got_slice.clone(), started: false }));
    s2.spawn(Box::new(Producer { ch: ch2, next: 0, n: 5 }));
    let out_slice = loop {
        if let Some(o) = s2.run_slice(1) {
            break o;
        }
    };
    let trace_slice = s2.into_trace();

    assert_eq!(out_run, out_slice, "slice-driven outcome matches run-to-quiescence");
    assert_eq!(*got_run.borrow(), *got_slice.borrow(), "same observable output either way");
    assert_eq!(trace_run, trace_slice, "same scheduling trace either way");
}

/// `snapshot` reports each task's blocked/ready state and each channel's depth/capacity —
/// the read-only view the Studio's Tasks/Channels strip renders between slices.
#[test]
fn sched_snapshot_reports_task_states_and_channel_depth() {
    let mut s = Scheduler::new(fifo(), Chooser::record(SchedSeed(0)));
    let ch = s.new_default_chan();
    s.spawn(Box::new(Recver { ch })); // receives from a channel that never gets a value
    // One step: the receiver polls and parks on the empty channel.
    let progressed = s.poll_once();
    assert!(progressed.is_none(), "first step makes progress, not quiescence");

    let snap = s.snapshot();
    assert_eq!(snap.channels.len(), 1, "one channel");
    assert_eq!(snap.channels[0].depth, 0, "channel is empty");
    assert!(snap.channels[0].capacity.is_some(), "default channel has a capacity");
    assert_eq!(snap.channels[0].blocked_receivers, 1, "the receiver is parked on it");
    assert!(
        snap.tasks.iter().any(|t| t.kind == TaskStateKind::BlockedRecv),
        "the receiver task is blocked on recv: {:?}",
        snap.tasks
    );
}

// ---- external-I/O parking (IoPending / WaitingForIo / wake_io) ----------------
//
// A task that awaits external async I/O (a network op) the scheduler cannot service itself
// returns `TaskStep::IoPending`. It must park `BlockedIo` (not deadlock), and the async drive
// loop re-readies it with `wake_io` after yielding to the host reactor. These toy tasks prove
// that mechanism without any interpreter / networking coupling.

/// Parks on external I/O `n` times (returns `IoPending`), then exits.
struct IoWaiter {
    remaining: u32,
}
impl<'t> Task<'t> for IoWaiter {
    fn poll(&mut self, _ctx: &mut TaskCtx) -> TaskStep<'t> {
        if self.remaining > 0 {
            self.remaining -= 1;
            TaskStep::IoPending
        } else {
            TaskStep::Exit(RtPayload::Nothing)
        }
    }
}

#[test]
fn io_pending_parks_then_wake_io_drives_to_done() {
    let mut sched = Scheduler::new(SchedulerConfig::default(), Chooser::record(SchedSeed(0)));
    sched.spawn_main(Box::new(IoWaiter { remaining: 2 }));

    // First wait: the task parks on I/O — not ready, not done, so WaitingForIo (NOT deadlock).
    assert_eq!(sched.run_slice(64), Some(RunOutcome::WaitingForIo));
    assert!(
        sched.snapshot().tasks.iter().any(|t| t.kind == TaskStateKind::BlockedIo),
        "the task is parked BlockedIo: {:?}",
        sched.snapshot().tasks
    );

    // The async drive loop re-readies the parked task after a reactor yield.
    assert!(sched.wake_io(), "a parked task was woken");
    assert_eq!(sched.run_slice(64), Some(RunOutcome::WaitingForIo)); // second wait
    assert!(sched.wake_io());

    // Now it exits.
    assert_eq!(sched.run_slice(64), Some(RunOutcome::Done(RtPayload::Nothing)));
    assert!(!sched.wake_io(), "nothing left to wake once done");
}

#[test]
fn io_pending_alone_is_not_a_deadlock() {
    // A program whose only un-finished task is parked on I/O must report WaitingForIo, never
    // Deadlock — a false deadlock would abort a perfectly live networking program.
    let mut sched = Scheduler::new(SchedulerConfig::default(), Chooser::record(SchedSeed(0)));
    sched.spawn_main(Box::new(IoWaiter { remaining: 1 }));
    let outcome = sched.run_slice(64);
    assert_eq!(outcome, Some(RunOutcome::WaitingForIo));
    assert_ne!(outcome, Some(RunOutcome::Deadlock));
}
