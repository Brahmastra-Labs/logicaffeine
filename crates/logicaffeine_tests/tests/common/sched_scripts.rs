//! Scheduler scripts: toy concurrent programs — sends, receives, try-operations, sleeps, launches,
//! stops, awaits, closes and selects over a handful of tasks — generated from a seed, and the
//! runtime scheduler driving each as a hand-written `Task` state machine. Every port of the scheduler
//! (`logicaffeine_system::replay`, the C family's `sched_core.c`) is held to what these produce on
//! `logicaffeine_runtime`, observation for observation.

use std::cell::RefCell;
use std::rc::Rc;

use logicaffeine_runtime::{
    ChanId, Chooser, RtPayload, RunOutcome, SchedSeed, Scheduler, SchedulerConfig, SelectArm, Task,
    TaskCtx, TaskId, TaskStep,
};

#[derive(Clone, Debug)]
pub enum Op {
    Show(u32),
    NewChan(Option<usize>),
    Send(usize, i64),
    TrySend(usize, i64),
    Recv(usize),
    TryRecv(usize),
    Sleep(u64),
    Spawn(usize),
    Stop(usize),
    Await(usize),
    Close(usize),
    Select(Vec<Sel>),
}

#[derive(Clone, Debug)]
pub enum Sel {
    Recv(usize),
    After(u64),
}

/// A program: script 0 is `main`; a script only launches scripts after its own index, so every run
/// ends.
pub type Program = Rc<Vec<Vec<Op>>>;
pub type Log = Rc<RefCell<Vec<String>>>;

// ─── The runtime side: each script as a `Task` state machine ─────────────────

enum Waiting {
    Nothing,
    Chan,
    Handle,
    Recv(usize),
    TryRecv(usize),
    TrySend(usize),
    Select(Vec<Sel>),
}

struct ScriptTask {
    program: Program,
    which: usize,
    pc: usize,
    chans: Vec<ChanId>,
    handles: Vec<TaskId>,
    log: Log,
    waiting: Waiting,
}

fn int_of(p: &RtPayload) -> String {
    match p {
        RtPayload::Int(n) => n.to_string(),
        RtPayload::Nothing => "nothing".to_string(),
        other => format!("{other:?}"),
    }
}

impl<'t> Task<'t> for ScriptTask {
    fn poll(&mut self, ctx: &mut TaskCtx) -> TaskStep<'t> {
        let tag = self.which;
        match std::mem::replace(&mut self.waiting, Waiting::Nothing) {
            Waiting::Nothing => {}
            Waiting::Chan => self.chans.push(ctx.new_chan.expect("a new channel")),
            Waiting::Handle => self.handles.push(ctx.spawned.expect("a spawned task")),
            Waiting::Recv(i) => self
                .log
                .borrow_mut()
                .push(format!("t{tag} recv#{i} {}", int_of(&ctx.resumed_with))),
            Waiting::TryRecv(i) => self
                .log
                .borrow_mut()
                .push(format!("t{tag} try-recv#{i} {}", int_of(&ctx.resumed_with))),
            Waiting::TrySend(i) => {
                let ok = matches!(ctx.resumed_with, RtPayload::Bool(true));
                self.log
                    .borrow_mut()
                    .push(format!("t{tag} try-send#{i} {ok}"));
            }
            Waiting::Select(arms) => {
                let arm = ctx.selected_arm.expect("a select resumes with its arm");
                let got = match arms[arm] {
                    Sel::Recv(_) => int_of(&ctx.resumed_with),
                    Sel::After(_) => "timeout".to_string(),
                };
                self.log
                    .borrow_mut()
                    .push(format!("t{tag} select arm {arm}: {got}"));
            }
        }
        while let Some(op) = self.program[self.which].get(self.pc).cloned() {
            self.pc += 1;
            match op {
                Op::Show(n) => self.log.borrow_mut().push(format!("t{tag} show {n}")),
                Op::NewChan(capacity) => {
                    self.waiting = Waiting::Chan;
                    return TaskStep::NewChan(capacity);
                }
                Op::Send(c, v) if !self.chans.is_empty() => {
                    return TaskStep::Send(self.chans[c % self.chans.len()], RtPayload::Int(v));
                }
                Op::TrySend(c, v) if !self.chans.is_empty() => {
                    self.waiting = Waiting::TrySend(c % self.chans.len());
                    return TaskStep::TrySend(self.chans[c % self.chans.len()], RtPayload::Int(v));
                }
                Op::Recv(c) if !self.chans.is_empty() => {
                    self.waiting = Waiting::Recv(c % self.chans.len());
                    return TaskStep::Recv(self.chans[c % self.chans.len()]);
                }
                Op::TryRecv(c) if !self.chans.is_empty() => {
                    self.waiting = Waiting::TryRecv(c % self.chans.len());
                    return TaskStep::TryRecv(self.chans[c % self.chans.len()]);
                }
                Op::Sleep(ticks) => {
                    if ticks > 0 {
                        return TaskStep::Sleep(ticks);
                    }
                }
                Op::Spawn(k) => {
                    self.waiting = Waiting::Handle;
                    return TaskStep::Spawn(Box::new(ScriptTask {
                        program: self.program.clone(),
                        which: k,
                        pc: 0,
                        chans: self.chans.clone(),
                        handles: Vec::new(),
                        log: self.log.clone(),
                        waiting: Waiting::Nothing,
                    }));
                }
                Op::Stop(h) if !self.handles.is_empty() => {
                    return TaskStep::Abort(self.handles[h % self.handles.len()]);
                }
                Op::Await(h) if !self.handles.is_empty() => {
                    return TaskStep::Await(self.handles[h % self.handles.len()]);
                }
                Op::Close(c) if !self.chans.is_empty() => {
                    return TaskStep::Close(self.chans[c % self.chans.len()]);
                }
                Op::Select(arms) => {
                    let arms: Vec<Sel> = arms
                        .into_iter()
                        .filter(|a| matches!(a, Sel::After(_)) || !self.chans.is_empty())
                        .collect();
                    if arms.is_empty() {
                        continue;
                    }
                    let steps = arms
                        .iter()
                        .map(|a| match a {
                            Sel::Recv(c) => SelectArm::Recv(self.chans[c % self.chans.len()]),
                            Sel::After(t) => SelectArm::Timeout(*t),
                        })
                        .collect();
                    self.waiting = Waiting::Select(arms);
                    return TaskStep::Select(steps);
                }
                _ => {}
            }
        }
        TaskStep::Exit(RtPayload::Nothing)
    }
}

pub fn run_on_runtime(program: &Program, seed: u64) -> (Vec<String>, &'static str) {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut sched = Scheduler::new(SchedulerConfig::default(), Chooser::record(SchedSeed(seed)));
    sched.spawn_main(Box::new(ScriptTask {
        program: program.clone(),
        which: 0,
        pc: 0,
        chans: Vec::new(),
        handles: Vec::new(),
        log: log.clone(),
        waiting: Waiting::Nothing,
    }));
    let outcome = match sched.run() {
        RunOutcome::Done(_) => "done",
        RunOutcome::Deadlock => "deadlock",
        RunOutcome::WaitingForIo => "waiting-for-io",
    };
    let lines = log.borrow().clone();
    (lines, outcome)
}

// ─── The corpus ─────────────────────────────────────────────────────────────

/// SplitMix64, for generating programs — deliberately not the scheduler's chooser, whose draws the
/// programs themselves consume.
pub struct Gen(pub u64);

impl Gen {
    pub fn below(&mut self, n: u64) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)) % n.max(1)
    }
}

pub fn generate(seed: u64) -> Program {
    let mut g = Gen(seed);
    let scripts = 1 + g.below(4) as usize;
    let mut program = Vec::with_capacity(scripts);
    for i in 0..scripts {
        let len = g.below(11) as usize;
        let mut ops = Vec::with_capacity(len);
        for pc in 0..len {
            let value = (i * 1000 + pc) as i64;
            let op = match g.below(13) {
                0 => Op::Show(value as u32),
                1 | 2 => Op::NewChan(match g.below(4) {
                    0 => None,
                    k => Some(k as usize - 1),
                }),
                3 => Op::Send(g.below(3) as usize, value),
                4 => Op::TrySend(g.below(3) as usize, value),
                5 => Op::Recv(g.below(3) as usize),
                6 => Op::TryRecv(g.below(3) as usize),
                7 => Op::Sleep(g.below(4)),
                8 | 9 if i + 1 < scripts => {
                    Op::Spawn(i + 1 + g.below((scripts - i - 1) as u64) as usize)
                }
                8 => Op::Show(value as u32),
                9 => Op::Close(g.below(3) as usize),
                10 => Op::Stop(g.below(3) as usize),
                11 => Op::Await(g.below(3) as usize),
                _ => Op::Select(
                    (0..1 + g.below(3))
                        .map(|_| {
                            if g.below(3) == 0 {
                                Sel::After(g.below(4))
                            } else {
                                Sel::Recv(g.below(3) as usize)
                            }
                        })
                        .collect(),
                ),
            };
            ops.push(op);
        }
        program.push(ops);
    }
    Rc::new(program)
}

pub const SCHEDULING_SEEDS: &[u64] = &[0, 1, 2, 7, 42, 1_000_003];

pub fn program(scripts: Vec<Vec<Op>>) -> Program {
    Rc::new(scripts)
}
