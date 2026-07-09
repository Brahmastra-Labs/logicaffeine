//! Phase 7 — the work-stealing M:N executor, against hand-written toy tasks.
//!
//! Proves the coordinator + worker threads end-to-end: a main task creates a
//! channel, spawns a producer and a consumer (via `SpawnDesc`, passing the channel
//! as a `Send` argument), the producer sends across a worker boundary, and the
//! consumer's received values come back in deterministic order. This exercises
//! every cross-thread mechanism (NewChan, SpawnDesc, Send, Recv, Exit,
//! take_output) over a real multi-worker pool — no interpreter involved.

use logicaffeine_runtime::{
    run_workstealing_seeded, ChanId, RtPayload, RunOutcome, SchedSeed, SchedulerConfig, SpawnDesc,
    Task, TaskCtx, TaskStep,
};

const FUNC_MAIN: u16 = 0;
const FUNC_PRODUCER: u16 = 1;
const FUNC_CONSUMER: u16 = 2;

/// Creates a channel, then spawns a producer and a consumer that share it.
struct ToyMain {
    step: u8,
    ch: Option<ChanId>,
}
impl Task<'static> for ToyMain {
    fn poll(&mut self, ctx: &mut TaskCtx) -> TaskStep<'static> {
        match self.step {
            0 => {
                self.step = 1;
                TaskStep::NewChan(None)
            }
            1 => {
                self.ch = ctx.new_chan;
                self.step = 2;
                TaskStep::SpawnDesc {
                    func: FUNC_PRODUCER,
                    args: vec![RtPayload::Chan(self.ch.unwrap())],
                    want_handle: false,
                }
            }
            2 => {
                self.step = 3;
                TaskStep::SpawnDesc {
                    func: FUNC_CONSUMER,
                    args: vec![RtPayload::Chan(self.ch.unwrap())],
                    want_handle: false,
                }
            }
            _ => TaskStep::Exit(RtPayload::Nothing),
        }
    }
}

/// Sends `1..=n` into `ch`, then exits.
struct ToyProducer {
    ch: ChanId,
    next: i64,
    n: i64,
}
impl Task<'static> for ToyProducer {
    fn poll(&mut self, _ctx: &mut TaskCtx) -> TaskStep<'static> {
        if self.next <= self.n {
            let v = self.next;
            self.next += 1;
            TaskStep::Send(self.ch, RtPayload::Int(v))
        } else {
            TaskStep::Exit(RtPayload::Nothing)
        }
    }
}

/// Receives `n` values, emitting each as an output line (`take_output`).
struct ToyConsumer {
    ch: ChanId,
    remaining: i64,
    out: Vec<String>,
    started: bool,
}
impl Task<'static> for ToyConsumer {
    fn poll(&mut self, ctx: &mut TaskCtx) -> TaskStep<'static> {
        if self.started {
            if let RtPayload::Int(v) = ctx.resumed_with {
                self.out.push(v.to_string());
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
    fn take_output(&mut self) -> Vec<String> {
        std::mem::take(&mut self.out)
    }
}

fn toy_build(desc: SpawnDesc) -> Box<dyn Task<'static> + 'static> {
    let chan = |args: &[RtPayload]| match &args[0] {
        RtPayload::Chan(id) => *id,
        other => panic!("expected a channel argument, got {other:?}"),
    };
    match desc.func {
        FUNC_MAIN => Box::new(ToyMain { step: 0, ch: None }),
        FUNC_PRODUCER => Box::new(ToyProducer { ch: chan(&desc.args), next: 1, n: 3 }),
        FUNC_CONSUMER => Box::new(ToyConsumer {
            ch: chan(&desc.args),
            remaining: 3,
            out: Vec::new(),
            started: false,
        }),
        other => unreachable!("unknown toy func {other}"),
    }
}

#[test]
fn ws_executor_producer_consumer() {
    let main = SpawnDesc { func: FUNC_MAIN, args: vec![], priority: 0, is_main: true };
    let result = run_workstealing_seeded(
        SchedulerConfig::default().with_workers(4),
        SchedSeed(0),
        main,
        toy_build,
    );
    assert_eq!(result.outcome, RunOutcome::Done(RtPayload::Nothing), "ran to completion");
    assert_eq!(
        result.output,
        vec!["1".to_string(), "2".to_string(), "3".to_string()],
        "the consumer received 1,2,3 in order through the work-stealing executor"
    );
}

#[test]
fn ws_executor_is_seed_reproducible() {
    let run = || {
        let main = SpawnDesc { func: FUNC_MAIN, args: vec![], priority: 0, is_main: true };
        run_workstealing_seeded(SchedulerConfig::default().with_workers(4), SchedSeed(7), main, toy_build)
            .output
    };
    assert_eq!(run(), run(), "same seed reproduces the same output under work-stealing");
}

// ─── Genuine multicore: bodies poll SIMULTANEOUSLY on distinct OS threads ────

use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

const FUNC_PAR_MAIN: u16 = 10;
const FUNC_PAR_CHILD: u16 = 11;

/// A poll record: which OS thread polled, and the wall-clock window it occupied.
type PollLog = Arc<Mutex<Vec<(ThreadId, Instant, Instant)>>>;

/// Occupy this worker thread for a fixed window, recording the interval. If two
/// tasks' windows OVERLAP, they were polled on different threads at the same wall
/// time — i.e. genuinely in parallel. A serial executor would produce disjoint
/// windows.
fn occupy(log: &PollLog) {
    let enter = Instant::now();
    std::thread::sleep(Duration::from_millis(40));
    let exit = Instant::now();
    log.lock().unwrap().push((std::thread::current().id(), enter, exit));
}

/// Spawns one child, then (resumed in the SAME ready batch as that child) occupies
/// its worker — so main and child are polled concurrently.
struct ParMain {
    spawned: bool,
    log: PollLog,
}
impl Task<'static> for ParMain {
    fn poll(&mut self, _ctx: &mut TaskCtx) -> TaskStep<'static> {
        if !self.spawned {
            self.spawned = true;
            TaskStep::SpawnDesc { func: FUNC_PAR_CHILD, args: vec![], want_handle: false }
        } else {
            occupy(&self.log);
            TaskStep::Exit(RtPayload::Nothing)
        }
    }
}

struct ParChild {
    log: PollLog,
}
impl Task<'static> for ParChild {
    fn poll(&mut self, _ctx: &mut TaskCtx) -> TaskStep<'static> {
        occupy(&self.log);
        TaskStep::Exit(RtPayload::Nothing)
    }
}

#[test]
fn workstealing_actually_parallel() {
    let log: PollLog = Arc::new(Mutex::new(Vec::new()));
    let build_log = log.clone();
    let build = move |desc: SpawnDesc| -> Box<dyn Task<'static> + 'static> {
        match desc.func {
            FUNC_PAR_MAIN => Box::new(ParMain { spawned: false, log: build_log.clone() }),
            FUNC_PAR_CHILD => Box::new(ParChild { log: build_log.clone() }),
            other => unreachable!("unknown parallel-toy func {other}"),
        }
    };
    let main = SpawnDesc { func: FUNC_PAR_MAIN, args: vec![], priority: 0, is_main: true };
    let result = run_workstealing_seeded(
        SchedulerConfig::default().with_workers(4),
        SchedSeed(0),
        main,
        build,
    );
    assert_eq!(result.outcome, RunOutcome::Done(RtPayload::Nothing), "ran to completion");

    let records = log.lock().unwrap();
    assert_eq!(records.len(), 2, "main + child each recorded one poll window");
    let (t0, e0, x0) = records[0];
    let (t1, e1, x1) = records[1];
    assert_ne!(t0, t1, "the two bodies must poll on DISTINCT OS threads, both ran on {t0:?}");
    // Half-open interval intersection: they overlapped in wall time ⇒ ran truly
    // concurrently. A cooperative (single-thread) executor cannot produce this.
    assert!(
        e0 < x1 && e1 < x0,
        "the two poll windows must overlap (genuine parallelism): \
         [{e0:?}, {x0:?}] vs [{e1:?}, {x1:?}]",
    );
}
