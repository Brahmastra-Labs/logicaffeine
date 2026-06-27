//! The VM task driver (T11).
//!
//! [`VmTask`] wraps one resumable [`Vm`] as a scheduler [`Task`]. On each `poll`
//! it delivers the scheduler's resume value into the VM's reserved register, runs
//! the VM until its next concurrency op (`run_until_block`), drains any output the
//! slice produced into a shared sink, and maps the VM's [`VmBlock`] request to a
//! scheduler [`TaskStep`]. It mirrors the tree-walker's
//! [`super::driver::InterpreterTask`].

use std::cell::RefCell;
use std::rc::Rc;

use logicaffeine_runtime::{RtPayload, Task, TaskCtx, TaskStep};

use crate::interpreter::RuntimeValue;
use crate::vm::{Value, Vm, VmBlock, VmStep};

use super::driver::ErrSink;
use super::marshal;

/// Where a VM task's `Show` output goes — the one place the cooperative and
/// work-stealing drivers diverge.
enum OutputMode {
    /// Cooperative (M:1): every task drains into one shared sink on the single
    /// thread, so per-task slices interleave in pick order directly.
    Shared(Rc<RefCell<Vec<String>>>),
    /// Work-stealing (M:N): a worker polls task bodies off-thread, so output is
    /// buffered locally and reported per-slice via [`Task::take_output`]; the
    /// coordinator re-orders the slices into pick order on its thread.
    Buffered(Vec<String>),
}

/// Drives one VM task on the scheduler. `err_sink` (first writer wins) receives
/// the task's error if it fails.
pub struct VmTask<'p> {
    vm: Vm<'p>,
    output: OutputMode,
    err_sink: Option<ErrSink>,
}

impl<'p> VmTask<'p> {
    /// A cooperative task draining into the shared `output` sink.
    pub fn new(vm: Vm<'p>, output: Rc<RefCell<Vec<String>>>, err_sink: Option<ErrSink>) -> Self {
        VmTask { vm, output: OutputMode::Shared(output), err_sink }
    }

    /// A work-stealing task: output buffers locally (reported via `take_output`),
    /// and a spawn forwards a `SpawnDesc` the worker rebuilds — never a boxed body
    /// (which is `!Send` and could not cross a worker boundary anyway).
    pub fn work_stealing(vm: Vm<'p>, err_sink: Option<ErrSink>) -> Self {
        VmTask { vm, output: OutputMode::Buffered(Vec::new()), err_sink }
    }

    fn block_to_step(&self, req: VmBlock) -> TaskStep<'p> {
        match req {
            VmBlock::NewChan(cap) => TaskStep::NewChan(cap),
            VmBlock::Send(ch, p) => TaskStep::Send(ch, p),
            VmBlock::Recv(ch) => TaskStep::Recv(ch),
            VmBlock::TrySend(ch, p) => TaskStep::TrySend(ch, p),
            VmBlock::TryRecv(ch) => TaskStep::TryRecv(ch),
            VmBlock::Close(ch) => TaskStep::Close(ch),
            VmBlock::SpawnDesc { func, args, want_handle } => match &self.output {
                // Cooperative: build the child VM inline over the shared program +
                // sink, and hand the scheduler the boxed body.
                OutputMode::Shared(sink) => {
                    let child_vm = self.vm.spawn_task_vm(func, &args);
                    let child = VmTask::new(child_vm, sink.clone(), self.err_sink.clone());
                    TaskStep::Spawn(Box::new(child))
                }
                // Work-stealing: forward the `Send` descriptor; the receiving
                // worker builds the child locally over its own program borrow.
                OutputMode::Buffered(_) => TaskStep::SpawnDesc { func, args, want_handle },
            },
            VmBlock::Await(t) => TaskStep::Await(t),
            VmBlock::Abort(t) => TaskStep::Abort(t),
            VmBlock::Select(arms) => TaskStep::Select(arms),
            VmBlock::Sleep(d) => TaskStep::Sleep(d),
        }
    }
}

impl<'p> Task<'p> for VmTask<'p> {
    fn poll(&mut self, ctx: &mut TaskCtx) -> TaskStep<'p> {
        // Deliver whatever the scheduler resumed us with. A resolved `Select`
        // routes the received value into the winning arm's var and the arm index
        // into the `SelectWait` destination; every other block delivers a single
        // value into the reserved register.
        if let Some(arm) = ctx.selected_arm.take() {
            let payload = std::mem::replace(&mut ctx.resumed_with, RtPayload::Nothing);
            self.vm.deliver_select(arm, Value::from_runtime(marshal::rebuild(payload)));
        } else {
            let resume = ctx_to_value(ctx);
            self.vm.deliver_resume(resume);
        }

        let step = self.vm.run_until_block();

        // Stream this slice's output. Cooperative drains straight into the shared
        // sink; work-stealing buffers locally for the coordinator to flush in pick
        // order (see [`Task::take_output`]).
        let lines = self.vm.drain_lines();
        if !lines.is_empty() {
            match &mut self.output {
                OutputMode::Shared(sink) => sink.borrow_mut().extend(lines),
                OutputMode::Buffered(buf) => buf.extend(lines),
            }
        }

        match step {
            Ok(VmStep::Done(result)) => {
                let payload = marshal::materialize(&result).unwrap_or(RtPayload::Nothing);
                TaskStep::Exit(payload)
            }
            Ok(VmStep::Blocked) => {
                let req = self
                    .vm
                    .take_pending()
                    .expect("a blocked VM slice must leave a pending request");
                self.block_to_step(req)
            }
            Ok(VmStep::Paused) => {
                unreachable!("the driver uses run_until_block; the debug stepper is never driven here")
            }
            Err(e) => {
                if let Some(sink) = &self.err_sink {
                    let mut slot = sink.borrow_mut();
                    if slot.is_none() {
                        *slot = Some(e);
                    }
                }
                TaskStep::Exit(RtPayload::Nothing)
            }
        }
    }

    /// Hand the work-stealing coordinator this slice's buffered output so it can
    /// be flushed in pick order. A no-op for cooperative tasks (they drain into
    /// the shared sink directly).
    fn take_output(&mut self) -> Vec<String> {
        match &mut self.output {
            OutputMode::Buffered(buf) => std::mem::take(buf),
            OutputMode::Shared(_) => Vec::new(),
        }
    }
}

/// Translate the scheduler's resume context into the VM resume value: a freshly
/// created channel id, a spawned task handle, or a received/awaited payload.
fn ctx_to_value(ctx: &mut TaskCtx) -> Value {
    if let Some(id) = ctx.new_chan {
        Value::from_runtime(RuntimeValue::Chan(id))
    } else if let Some(id) = ctx.spawned {
        Value::from_runtime(RuntimeValue::TaskHandle(id))
    } else {
        let payload = std::mem::replace(&mut ctx.resumed_with, RtPayload::Nothing);
        Value::from_runtime(marshal::rebuild(payload))
    }
}
