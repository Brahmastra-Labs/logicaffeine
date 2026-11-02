//! A select that names one pipe in two arms is registered on that pipe twice while it waits; when it
//! wins, BOTH registrations must go.
//!
//! The scheduler cancelled a woken select's registrations on every pipe except the one that woke it,
//! so the second registration on that pipe outlived the select. The task went on to block somewhere
//! else, and the next value sent into the pipe was handed to it there — a receive on one pipe resumed
//! with a value sent into another, and the value its own receiver was owed vanished. The interpreter,
//! the VM and the deterministic replay executor all ran the same scheduler, so all three did it, and
//! the C tier's port of that scheduler did the same.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use logicaffeine_compile::compile::compile_to_c;
use logicaffeine_compile::{interpret_for_ui_sync, run_vm_concurrent};
use logicaffeine_runtime::{
    ChanId, Chooser, RtPayload, RunOutcome, SchedSeed, Scheduler, SchedulerConfig, SelectArm, Task,
    TaskCtx, TaskStep,
};

/// Main selects over the same pipe twice, before anything is sent; a feeder then sends twice.
/// After its select, main waits on a second pipe nothing ever sends to.
const PROGRAM: &str = "\
## To feed (a: Int):
    Sleep 1.
    Send 1 into a.
    Send 2 into a.

## Main
Let a be a Pipe of Int.
Let b be a Pipe of Int.
Launch a task to feed with a.
Await the first of:
    Receive x from a:
        Show x.
    Receive y from a:
        Show y.
Receive z from b.
Show \"received {z} from b\".
";

/// The same select, with `b` fed after `a`'s second value: the program completes, and `b`'s receive
/// must get `b`'s value.
const FED: &str = "\
## To feed (up: Int, down: Int):
    Sleep 1.
    Send 1 into up.
    Send 2 into up.
    Send 99 into down.

## Main
Let up be a Pipe of Int.
Let down be a Pipe of Int.
Launch a task to feed with up and down.
Await the first of:
    Receive x from up:
        Show x.
    Receive y from up:
        Show y.
Receive z from down.
Show z.
";

#[test]
fn every_tier_hands_the_second_pipes_value_to_its_own_receiver() {
    let tw = interpret_for_ui_sync(FED);
    assert_eq!(
        (tw.lines.as_slice(), tw.error.as_deref()),
        (&["1".to_string(), "99".to_string()][..], None),
        "tree-walker"
    );
    let vm = run_vm_concurrent(FED);
    assert_eq!(
        (vm.lines.as_slice(), vm.error.as_deref()),
        (&["1".to_string(), "99".to_string()][..], None),
        "vm"
    );
}

#[test]
fn the_c_scheduler_hands_the_second_pipes_value_to_its_own_receiver() {
    if !common::require_tool("gcc") {
        return;
    }
    let c = compile_to_c(FED).expect("the C tier lowers the program");
    let dir = tempfile::tempdir().expect("temp dir");
    let (src, bin) = (dir.path().join("prog.c"), dir.path().join("prog"));
    std::fs::write(&src, &c).expect("write C");
    let build = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&src)
        .arg("-lm")
        .output()
        .expect("gcc runs");
    assert!(
        build.status.success(),
        "gcc:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new(&bin)
        .output()
        .expect("the binary runs");
    assert!(
        run.status.success(),
        "the C binary failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout)
            .lines()
            .collect::<Vec<_>>(),
        ["1", "99"]
    );
}

fn assert_deadlocks_after_the_select(tier: &str, lines: &[String], error: Option<&str>) {
    assert_eq!(
        lines,
        ["1"],
        "{tier}: only the select's value is shown — `b` never receives anything"
    );
    let error =
        error.unwrap_or_else(|| panic!("{tier}: main waits on `b` forever, which is a deadlock"));
    assert!(
        error.contains("deadlock"),
        "{tier}: expected a deadlock, got: {error}"
    );
}

#[test]
fn the_tree_walker_does_not_hand_the_pipes_next_value_to_a_finished_select() {
    let run = interpret_for_ui_sync(PROGRAM);
    assert_deadlocks_after_the_select("tree-walker", &run.lines, run.error.as_deref());
}

#[test]
fn the_vm_does_not_hand_the_pipes_next_value_to_a_finished_select() {
    let run = run_vm_concurrent(PROGRAM);
    assert_deadlocks_after_the_select("vm", &run.lines, run.error.as_deref());
}

/// The same shape on the scheduler alone, with the wake it must not make observed directly.
struct Selector {
    a: ChanId,
    b: ChanId,
    step: u8,
    log: Rc<RefCell<Vec<String>>>,
}

impl<'t> Task<'t> for Selector {
    fn poll(&mut self, ctx: &mut TaskCtx) -> TaskStep<'t> {
        self.step += 1;
        match self.step {
            1 => TaskStep::Select(vec![SelectArm::Recv(self.a), SelectArm::Recv(self.a)]),
            2 => {
                self.log.borrow_mut().push(format!(
                    "select arm {:?} got {:?}",
                    ctx.selected_arm, ctx.resumed_with
                ));
                TaskStep::Recv(self.b)
            }
            _ => {
                self.log
                    .borrow_mut()
                    .push(format!("b delivered {:?}", ctx.resumed_with));
                TaskStep::Exit(RtPayload::Nothing)
            }
        }
    }
}

struct Feeder {
    a: ChanId,
    sent: i64,
}

impl<'t> Task<'t> for Feeder {
    fn poll(&mut self, _ctx: &mut TaskCtx) -> TaskStep<'t> {
        self.sent += 1;
        match self.sent {
            1 => TaskStep::Yield,
            2 | 3 => TaskStep::Send(self.a, RtPayload::Int(self.sent - 1)),
            _ => TaskStep::Exit(RtPayload::Nothing),
        }
    }
}

#[test]
fn the_scheduler_cancels_every_registration_a_woken_select_made() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut sched = Scheduler::new(SchedulerConfig::default(), Chooser::record(SchedSeed(0)));
    let a = sched.new_chan(Some(4));
    let b = sched.new_chan(Some(4));
    sched.spawn_main(Box::new(Selector {
        a,
        b,
        step: 0,
        log: log.clone(),
    }));
    sched.spawn(Box::new(Feeder { a, sent: 0 }));
    let outcome = sched.run();
    assert_eq!(
        *log.borrow(),
        ["select arm Some(0) got Int(1)"],
        "`b` must not be handed `a`'s second value"
    );
    assert!(
        matches!(outcome, RunOutcome::Deadlock),
        "main waits on `b` forever: {outcome:?}"
    );
}
