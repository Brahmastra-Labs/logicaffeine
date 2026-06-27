//! The work-stealing M:N native driver (Phase 7 of FINISH_INTERPRETER.md).
//!
//! Genuine multicore for the VM/tree-walker tiers, native-only (`std::thread` +
//! `std::sync`; never tokio, never `crossbeam` — this crate is dependency-free by
//! charter). The design turns on one fact: a task body (`Vm`/`Interpreter`) holds
//! `Rc` heaps and is irreducibly `!Send`, so **tasks never cross threads**. What
//! crosses is only a `Send` *descriptor*; each worker *builds* its `!Send` task
//! locally from its own clone of the program.
//!
//! ## Architecture — a central coordinator owns every decision
//!
//! ```text
//!        ┌───────────────────────────────────────────┐
//!        │ Coordinator (the authoritative Scheduler + │
//!        │   the SINGLE Chooser seed choke point):    │
//!        │   channels, timers, the work deque         │
//!        └───────────────┬───────────────────────────┘
//!          Send descriptors ↓   ↑ Send StepReport
//!        ┌─────────────────┴──┐ ┌┴────────────────────┐
//!        │ Worker 0 (thread)  │ │ Worker N-1          │
//!        │  own program clone │ │  own program clone  │
//!        │  builds !Send task │ │  builds !Send task  │
//!        │  polls body → block│ │  polls body → block │
//!        └────────────────────┘ └─────────────────────┘
//! ```
//!
//! Workers never touch channels, timers, or the `Chooser` directly: they poll a
//! task body (the expensive part — arithmetic, loops, JIT regions) and report the
//! resulting [`StepReport`] back. The coordinator applies every report through the
//! *same* [`crate::scheduler::Scheduler`] state machine and the *same* `Chooser`,
//! in a deterministic order, so for a fixed seed the trace is **identical to the
//! cooperative run** — parallelism only reorders the wall-clock arrival of reports,
//! which a logical clock + a `TaskId`-sorted apply order erase.
//!
//! Determinism is therefore preserved *by construction*: there is one choice
//! point. "Spawns are stealable; resumptions are pinned" — a freshly spawned task
//! (just `(func, args)`) can be built by any idle worker, but an already-built
//! `!Send` continuation stays on the worker that last ran it.

use crate::channel::ChanId;
use crate::payload::RtPayload;
use crate::task::{SelectArm, TaskId};

/// An opaque function index, from the runtime's view — the compile crate's
/// `FuncIdx` (a `u16`), kept local so this crate stays dependency-free.
pub type FuncIdx = u16;

/// A `Send` task descriptor: which function to run, its already-materialised
/// arguments, its scheduling priority, and whether it is the run's main task.
/// This is what a worker turns into a `!Send` task body locally.
pub struct SpawnDesc {
    pub func: FuncIdx,
    pub args: Vec<RtPayload>,
    pub priority: u8,
    pub is_main: bool,
}

/// How a worker should (re)enter a task body on its next poll. Every variant is
/// `Send` (`FuncIdx` is `u16`, `RtPayload` is `Send`, ids are `Copy`).
pub enum ResumeKind {
    /// First entry / a block that yields no value (`Send`/`Close`/`Yield`/`Sleep`).
    Nothing,
    /// Fresh spawn: build the body from this function + materialised args.
    /// `is_main` distinguishes the program's root task (whose body is the
    /// top-level program, not a spawned function) so the builder positions it
    /// correctly.
    Spawn { func: FuncIdx, args: Vec<RtPayload>, priority: u8, is_main: bool },
    /// Resume a pinned continuation with a delivered value (`Recv`/`TryRecv`/`Await`).
    Payload(RtPayload),
    /// Resume a `Select` winner: the winning arm index + its (recv) payload.
    Select { arm: usize, payload: RtPayload },
    /// Resume after `NewChan` with the freshly-created channel id.
    NewChan(ChanId),
    /// Resume after a handle-bearing spawn with the child's task id.
    SpawnedHandle(TaskId),
}

/// The `Send` projection of a [`crate::task::TaskStep`] a worker reports after
/// polling one task slice (the `tid` lives on [`WorkerReport`]). The crucial
/// difference from `TaskStep`: the spawn variant carries a *descriptor*, not a
/// `Box<dyn Task>` — so the `!Send` body is built and consumed on one worker.
enum ReportStep {
    Yield,
    Recv(ChanId),
    Send(ChanId, RtPayload),
    TrySend(ChanId, RtPayload),
    TryRecv(ChanId),
    Select(Vec<SelectArm>),
    Sleep(u64),
    NewChan(Option<usize>),
    Spawn { func: FuncIdx, args: Vec<RtPayload>, want_handle: bool },
    Await(TaskId),
    Abort(TaskId),
    Close(ChanId),
    Exit(RtPayload),
}

/// What one worker sends back after a poll slice: which task, the output that
/// slice produced (flushed by the coordinator in pick order), and the projected
/// step.
struct WorkerReport {
    tid: TaskId,
    output: Vec<String>,
    step: ReportStep,
}

/// A unit of work the coordinator sends to a worker.
enum WorkMsg {
    Run { tid: TaskId, resume: ResumeKind },
    Shutdown,
}

/// The result of a work-stealing run.
pub struct WsOutcome {
    pub outcome: crate::scheduler::RunOutcome,
    pub output: Vec<String>,
    pub trace: crate::seed::SchedTrace,
}

/// Compile-time proof that the entire cross-thread boundary is `Send`. The task
/// bodies (`Vm`/`Interpreter`, `Box<dyn Task>`, any `Rc`) never cross.
#[allow(dead_code)]
fn _assert_boundary_send() {
    fn s<T: Send>() {}
    s::<SpawnDesc>();
    s::<ResumeKind>();
    s::<WorkerReport>();
    s::<WorkMsg>();
    s::<RtPayload>();
}

use std::collections::HashMap;
use std::sync::mpsc;

use crate::scheduler::{RunOutcome, Scheduler};
use crate::seed::{Chooser, SchedSeed};
use crate::config::SchedulerConfig;
use crate::task::{TaskCtx, TaskStep};

/// Run a concurrent program on the **work-stealing M:N driver** under `seed`.
///
/// `build` turns a `Send` [`SpawnDesc`] into a worker-local `!Send` task body; it
/// is cloned once per worker, so each worker constructs its tasks over its own
/// resources (e.g. its own program clone) and no body ever crosses a thread. The
/// coordinator owns the single [`Scheduler`] + `Chooser`, so the seeded trace is
/// identical to the cooperative run; workers only poll task *bodies* in parallel.
pub fn run_workstealing_seeded<'env, B>(
    config: SchedulerConfig,
    seed: SchedSeed,
    main: SpawnDesc,
    build: B,
) -> WsOutcome
where
    B: Fn(SpawnDesc) -> Box<dyn crate::task::Task<'env> + 'env> + Sync,
{
    let n = config.workers.max(1);
    // The coordinator's `Scheduler` only ever holds *metadata* slots (task bodies
    // live worker-side), so its task lifetime is vacuous — `'static`.
    let mut sched: Scheduler<'static> = Scheduler::new(config, Chooser::record(seed));
    let (report_tx, report_rx) = mpsc::channel::<WorkerReport>();

    // The main task is a metadata slot; its body is built worker-side from `main`.
    let main_tid = sched.spawn_meta(main.priority, true);
    let mut fresh: HashMap<TaskId, SpawnDesc> = HashMap::new();
    fresh.insert(main_tid, main);

    // Scoped threads let workers BORROW `&build` (and through it the program), so
    // the bodies need not be `'static` — no leak, no unsafe.
    let build_ref = &build;
    let (outcome, output, trace) = std::thread::scope(|scope| {
        // coordinator→worker work (one channel each, so a pinned resume routes to
        // its owning worker).
        let mut work_txs = Vec::with_capacity(n);
        for _ in 0..n {
            let (work_tx, work_rx) = mpsc::channel::<WorkMsg>();
            work_txs.push(work_tx);
            let rtx = report_tx.clone();
            scope.spawn(move || worker_main(work_rx, rtx, build_ref));
        }
        drop(report_tx); // so `report_rx` closes once every worker's clone drops

        let mut owner: HashMap<TaskId, usize> = HashMap::new();
        let mut next_w = 0usize;
        let mut output: Vec<String> = Vec::new();

        let outcome = loop {
            // The ready batch, in the SAME pick order the cooperative scheduler uses.
            let mut batch: Vec<crate::scheduler::WsDispatch> = Vec::new();
            while let Some(d) = sched.ws_next_dispatch() {
                batch.push(d);
            }
            if batch.is_empty() {
                if sched.ws_all_done() {
                    break RunOutcome::Done(sched.ws_main_result());
                }
                if sched.ws_has_timers() {
                    sched.ws_advance_timers();
                    continue;
                }
                break RunOutcome::Deadlock;
            }

            // Dispatch all ready tasks (workers poll their bodies in parallel). A
            // fresh task is assigned round-robin (deterministic); a resume goes to
            // its owning worker (the body is pinned there).
            let order: Vec<TaskId> = batch.iter().map(|d| d.tid).collect();
            for d in batch {
                let w = if fresh.contains_key(&d.tid) {
                    let w = next_w;
                    next_w = (next_w + 1) % n;
                    owner.insert(d.tid, w);
                    w
                } else {
                    owner[&d.tid]
                };
                let resume = if let Some(desc) = fresh.remove(&d.tid) {
                    ResumeKind::Spawn {
                        func: desc.func,
                        args: desc.args,
                        priority: desc.priority,
                        is_main: desc.is_main,
                    }
                } else if let Some(arm) = d.selected_arm {
                    ResumeKind::Select { arm, payload: d.resume }
                } else if let Some(ch) = d.new_chan {
                    ResumeKind::NewChan(ch)
                } else if let Some(t) = d.spawned {
                    ResumeKind::SpawnedHandle(t)
                } else {
                    match d.resume {
                        RtPayload::Nothing => ResumeKind::Nothing,
                        p => ResumeKind::Payload(p),
                    }
                };
                work_txs[w].send(WorkMsg::Run { tid: d.tid, resume }).expect("worker alive");
            }

            // Barrier: collect this batch's reports (arrival order is irrelevant).
            let mut reports: HashMap<TaskId, WorkerReport> = HashMap::new();
            for _ in 0..order.len() {
                let r = report_rx.recv().expect("worker report");
                reports.insert(r.tid, r);
            }

            // Apply in PICK ORDER: flush each slice's output, then apply its step
            // through the SAME Scheduler methods the cooperative driver uses — this
            // is what makes output + channel resolution identical to cooperative.
            for tid in &order {
                let WorkerReport { tid: rt, output: slice_out, step } =
                    reports.remove(tid).expect("report for dispatched task");
                output.extend(slice_out);
                apply_step(&mut sched, &mut fresh, rt, step);
            }
        };

        for tx in &work_txs {
            let _ = tx.send(WorkMsg::Shutdown);
        }
        drop(work_txs);
        (outcome, output, sched.into_trace())
    });

    WsOutcome { outcome, output, trace }
}

/// Apply one worker's report through the authoritative `Scheduler` (reusing its
/// channel/decision methods), turning a spawn report into a metadata child +
/// recorded descriptor.
fn apply_step(
    sched: &mut Scheduler<'static>,
    fresh: &mut HashMap<TaskId, SpawnDesc>,
    tid: TaskId,
    step: ReportStep,
) {
    match step {
        ReportStep::Yield => sched.process_step(tid, TaskStep::Yield),
        ReportStep::Recv(ch) => sched.process_step(tid, TaskStep::Recv(ch)),
        ReportStep::Send(ch, p) => sched.process_step(tid, TaskStep::Send(ch, p)),
        ReportStep::TrySend(ch, p) => sched.process_step(tid, TaskStep::TrySend(ch, p)),
        ReportStep::TryRecv(ch) => sched.process_step(tid, TaskStep::TryRecv(ch)),
        ReportStep::Select(arms) => sched.process_step(tid, TaskStep::Select(arms)),
        ReportStep::Sleep(t) => sched.process_step(tid, TaskStep::Sleep(t)),
        ReportStep::NewChan(cap) => sched.process_step(tid, TaskStep::NewChan(cap)),
        ReportStep::Await(t) => sched.process_step(tid, TaskStep::Await(t)),
        ReportStep::Abort(t) => sched.process_step(tid, TaskStep::Abort(t)),
        ReportStep::Close(ch) => sched.process_step(tid, TaskStep::Close(ch)),
        ReportStep::Exit(v) => sched.process_step(tid, TaskStep::Exit(v)),
        ReportStep::Spawn { func, args, want_handle: _ } => {
            // Mirror the cooperative `Spawn` step: create the child + hand the
            // parent the child id; record the child's build descriptor.
            let child = sched.ws_spawn_child(tid, 0);
            fresh.insert(child, SpawnDesc { func, args, priority: 0, is_main: false });
        }
    }
}

/// One worker thread: builds (fresh) or looks up (resume) the `!Send` task body,
/// polls one slice, and reports the projected step + the slice's output.
fn worker_main<'env, B>(
    work_rx: mpsc::Receiver<WorkMsg>,
    report_tx: mpsc::Sender<WorkerReport>,
    build: &B,
) where
    B: Fn(SpawnDesc) -> Box<dyn crate::task::Task<'env> + 'env>,
{
    let mut local: HashMap<TaskId, Box<dyn crate::task::Task<'env> + 'env>> = HashMap::new();
    while let Ok(msg) = work_rx.recv() {
        let (tid, resume) = match msg {
            WorkMsg::Shutdown => break,
            WorkMsg::Run { tid, resume } => (tid, resume),
        };
        let mut ctx = TaskCtx {
            resumed_with: RtPayload::Nothing,
            selected_arm: None,
            new_chan: None,
            spawned: None,
        };
        match resume {
            ResumeKind::Nothing => {}
            ResumeKind::Spawn { func, args, priority, is_main } => {
                local.insert(tid, build(SpawnDesc { func, args, priority, is_main }));
            }
            ResumeKind::Payload(p) => ctx.resumed_with = p,
            ResumeKind::Select { arm, payload } => {
                ctx.selected_arm = Some(arm);
                ctx.resumed_with = payload;
            }
            ResumeKind::NewChan(ch) => ctx.new_chan = Some(ch),
            ResumeKind::SpawnedHandle(t) => ctx.spawned = Some(t),
        }
        let task = local.get_mut(&tid).expect("task body present on its owning worker");
        let step = task.poll(&mut ctx);
        let out = task.take_output();
        let (rstep, done) = project_step(step);
        if done {
            local.remove(&tid);
        }
        if report_tx.send(WorkerReport { tid, output: out, step: rstep }).is_err() {
            break;
        }
    }
}

/// Project a worker's `TaskStep` into the `Send` [`ReportStep`]; the `bool` is
/// whether the task finished (so the worker drops its body).
fn project_step<'env>(step: TaskStep<'env>) -> (ReportStep, bool) {
    match step {
        TaskStep::Yield => (ReportStep::Yield, false),
        TaskStep::Recv(ch) => (ReportStep::Recv(ch), false),
        TaskStep::Send(ch, p) => (ReportStep::Send(ch, p), false),
        TaskStep::TrySend(ch, p) => (ReportStep::TrySend(ch, p), false),
        TaskStep::TryRecv(ch) => (ReportStep::TryRecv(ch), false),
        TaskStep::Select(arms) => (ReportStep::Select(arms), false),
        TaskStep::Sleep(t) => (ReportStep::Sleep(t), false),
        TaskStep::NewChan(cap) => (ReportStep::NewChan(cap), false),
        TaskStep::SpawnDesc { func, args, want_handle } => {
            (ReportStep::Spawn { func, args, want_handle }, false)
        }
        TaskStep::Spawn(_) => {
            panic!("work-stealing tasks must spawn via SpawnDesc, not a boxed Spawn")
        }
        TaskStep::Await(t) => (ReportStep::Await(t), false),
        TaskStep::Abort(t) => (ReportStep::Abort(t), false),
        TaskStep::Close(ch) => (ReportStep::Close(ch), false),
        TaskStep::IoPending => {
            // Networking runs on the tree-walker (async) tier, never the work-stealing
            // compute tier, so a worker never parks on external I/O.
            panic!("work-stealing tasks do not perform external I/O (IoPending)")
        }
        TaskStep::Exit(v) => (ReportStep::Exit(v), true),
    }
}
