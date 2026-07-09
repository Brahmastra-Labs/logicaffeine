#![doc = include_str!("../README.md")]

pub mod channel;
pub mod config;
// The work-stealing M:N driver is a standard part of the native runtime — it is
// always available off-WASM. It is excluded only on `wasm32`, where there are no
// OS threads (the cooperative M:1 driver is the browser's only option). This is a
// target gate, NOT an opt-in Cargo feature: genuine multicore is not optional.
#[cfg(not(target_arch = "wasm32"))]
pub mod executor;
pub mod payload;
pub mod scheduler;
pub mod seed;
pub mod task;

pub use channel::ChanId;
pub use config::{ClockMode, SchedulePolicy, SchedulerConfig};
pub use payload::RtPayload;
pub use scheduler::{
    run_with_seed, run_with_trace, ChanView, RunOutcome, SchedSnapshot, Scheduler, TaskStateKind,
    TaskView,
};
pub use seed::{ChoiceKind, ChoicePoint, Chooser, SchedSeed, SchedTrace, SeededRng};
pub use task::{SelectArm, Task, TaskCtx, TaskId, TaskStep};

#[cfg(not(target_arch = "wasm32"))]
pub use executor::{run_workstealing_seeded, FuncIdx, SpawnDesc, WsOutcome};
