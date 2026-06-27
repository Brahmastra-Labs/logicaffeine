//! # logicaffeine-runtime — the deterministic concurrency runtime
//!
//! **Charter.** This crate is the *operational semantics* of Logos concurrency
//! for the interpreter and VM — the task scheduler, the channels (as language
//! semantics), `Select`, the timer wheel, and the seed/trace machinery that makes
//! every scheduling decision deterministic and exactly replayable.
//!
//! It is deliberately **pure, WASM-safe, and tokio-free**: the cooperative (M:1)
//! driver runs on a single thread/event loop, and the work-stealing (M:N) driver
//! uses `std::thread`, never tokio. It is **never linked into AOT-compiled
//! binaries** (invariant I6 of `FINISH_INTERPRETER.md`) — the compiled path uses
//! `logicaffeine-system`'s platform services instead. Keeping this crate
//! dependency-free is what *enforces* that boundary at the type/dependency level.
//!
//! ## Milestones
//! - **Phase 2a (this milestone):** the seed/trace determinism contract
//!   ([`seed`]) and the `Send`-able value subset that crosses task boundaries
//!   ([`payload`]).
//! - **Phase 2b:** the scheduler core (executors, channels, select, timers) built
//!   on top of these.

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
