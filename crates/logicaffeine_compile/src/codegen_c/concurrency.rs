//! The **concurrency-lowering knob** for the C-family backends.
//!
//! LOGOS's deterministic concurrency (channels / tasks / `Await the first of` / virtual-time `Sleep`)
//! needs a task to SUSPEND at a blocking op and RESUME later. There are two ways to lower that suspend,
//! and — like the CUDA [`crate::codegen_cuda::CudaKnobs`] lattice — **neither is strictly better**, so it
//! is a knob with a documented trade-off ([`ConcurrencyLowering::tradeoffs`]). Unlike most knobs, one end
//! is **capability-forced**: a target with no `ucontext`/`setjmp` (a GPU kernel, WASM, bare metal) can
//! ONLY do the stackless form. So the lattice point is chosen by [`ConcurrencyLowering::for_target`]:
//! the best-general choice each target *can* express.
//!
//! - [`ConcurrencyLowering::Ucontext`] — stackful coroutines (`getcontext`/`makecontext`/`swapcontext`).
//!   A blocking op yields mid-loop / mid-nested-call and the native stack survives, so codegen is SIMPLE
//!   (emit native code + a `logos_yield()` at each blocking point) and straight-line code runs at native
//!   speed with arbitrary nesting. But it needs `ucontext`/`setjmp` (host, RISC-V under qemu, a Tensix
//!   baby core) and a full stack per task (~256 KB). This is the RISC-V tier's approach today.
//! - [`ConcurrencyLowering::Stackless`] — a state machine: each task body is transformed into a resumable
//!   `switch` over its blocking points, with live locals hoisted into a heap frame. UNIVERSAL (CUDA
//!   device, WASM, embedded — no OS/libc), a tiny per-task frame, cheap function-return yields, and it
//!   scales to many tasks. But codegen is COMPLEX (the async/await-style transform) and each resume pays
//!   a small `switch`-dispatch cost. This is what the CUDA tier requires.

/// A C-family lowering target, classified by whether it can express stackful coroutines. The
/// concurrency knob is forced to [`ConcurrencyLowering::Stackless`] exactly when `ucontext` is absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CTarget {
    /// The host C backend (gcc/clang) — full libc, `ucontext` available.
    HostC,
    /// The RISC-V AOT tier (runs under qemu with glibc) — `ucontext` available.
    RiscV,
    /// A Tenstorrent baby RISC-V core (glibc-backed C body) — `ucontext` available.
    Tenstorrent,
    /// A single CUDA `<<<1,1>>>` kernel — device code has NO `ucontext`/`setjmp`.
    Cuda,
    /// The direct-WASM backend — no `ucontext`/`setjmp` in the sandbox.
    Wasm,
}

impl CTarget {
    /// Whether this target can express stackful coroutines (`getcontext`/`makecontext`/`swapcontext`).
    /// The single capability that forces the concurrency-lowering knob.
    pub fn has_ucontext(self) -> bool {
        match self {
            CTarget::HostC | CTarget::RiscV | CTarget::Tenstorrent => true,
            CTarget::Cuda | CTarget::Wasm => false,
        }
    }
}

/// How the C-family backend lowers a task's suspend/resume. One point in the (one-dimensional) knob
/// lattice; pick with [`ConcurrencyLowering::for_target`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConcurrencyLowering {
    /// Stackful coroutines via `ucontext`. Simple codegen, native speed, arbitrary nesting; needs
    /// `ucontext` + a full stack per task. Host / RISC-V / Tenstorrent.
    Ucontext,
    /// A stackless state machine. Universal (device / WASM / embedded), tiny frames, cheap yields;
    /// complex codegen + a small per-resume dispatch cost. CUDA / WASM.
    Stackless,
}

/// One knob's trade-off — the regime where the setting **wins** and where it **loses** (why it can't be
/// baked in unconditionally). Mirrors [`crate::codegen_cuda::KnobTradeoff`] but kept local so
/// `codegen_c` does not depend on `codegen_cuda`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConcurrencyTradeoff {
    pub setting: &'static str,
    pub wins: &'static str,
    pub loses: &'static str,
}

impl ConcurrencyLowering {
    /// The best-general lowering each target CAN express. A target without `ucontext` is FORCED to
    /// [`ConcurrencyLowering::Stackless`]; a target that has it defaults to [`ConcurrencyLowering::Ucontext`]
    /// (simpler codegen + native speed — the right general call when portability is not at stake).
    pub fn for_target(target: CTarget) -> Self {
        if target.has_ucontext() {
            ConcurrencyLowering::Ucontext
        } else {
            ConcurrencyLowering::Stackless
        }
    }

    /// The trade-off table — one row per setting, each naming where it wins and where it loses. The
    /// concurrency-knob lock asserts this stays complete and honest (neither setting is strictly better,
    /// which is exactly why this is a knob and not a baked-in choice).
    pub fn tradeoffs() -> &'static [ConcurrencyTradeoff] {
        &[
            ConcurrencyTradeoff {
                setting: "Ucontext",
                wins: "targets WITH ucontext (host / RISC-V / Tenstorrent) — simple codegen (native code + \
                       a yield at each blocking op), native straight-line speed, arbitrary loop/call nesting \
                       across a yield",
                loses: "no ucontext (a GPU kernel, WASM, bare metal) — cannot be expressed at all; and a full \
                        ~256 KB stack per task, so it scales poorly to many tasks",
            },
            ConcurrencyTradeoff {
                setting: "Stackless",
                wins: "UNIVERSAL — the only option a GPU kernel / WASM / embedded target can express; a tiny \
                       per-task frame (just the live locals), cheap function-return yields, scales to many tasks",
                loses: "complex codegen (the async-style state-machine transform of every task body) and a \
                        small per-resume switch-dispatch cost vs native stackful code",
            },
        ]
    }
}

// ============================================================================
//  The SCHEDULER knobs — the emitted deterministic scheduler's tunables.
// ============================================================================

pub use logicaffeine_runtime::{ClockMode, SchedulePolicy, SchedulerConfig};

/// The knobs that parameterize the **emitted** deterministic scheduler. This is the SAME
/// [`SchedulerConfig`] the oracle runtime (`logicaffeine_runtime`) runs — the ready-queue `policy`
/// (FIFO / LIFO / round-robin / seeded-random / priority), the `clock` (virtual `Logical` vs `Wall`),
/// the default channel capacity, and the M:N `workers` count (1 = cooperative single core, N =
/// work-stealing) — plus the `seed` that drives the seeded chooser for the random-choice points
/// (`TaskPick` under `Random`, `SelectWinner`, `TimerTieBreak`). Emitting C straight from a
/// [`SchedulerConfig`] guarantees the AOT scheduler's knobs are ONE-TO-ONE with the oracle's, so a
/// program run at `knobs` on the VM and the same `knobs` in AOT-C agree.
///
/// The DEFAULT — `Fifo` / `workers = 1` / `Logical` / `seed = 0` — is the exact point every
/// determinism lock pins against `run_vm_concurrent` (which uses `SchedulerConfig::default()`), and is
/// the fixed policy the hand-written scheduler already implements (child-first spawn, FIFO ready queue,
/// earliest-timer-first virtual clock).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerKnobs {
    /// The oracle-shared scheduler config: `policy` / `clock` / `default_channel_capacity` /
    /// `preempt_every` / `workers`.
    pub config: SchedulerConfig,
    /// Seed for the emitted SplitMix64 chooser (mirrors `logicaffeine_runtime::seed::SeededRng`). Only
    /// observable under `SchedulePolicy::Random` or on a genuine multi-ready `Select` / timer tie.
    pub seed: u64,
}

impl Default for SchedulerKnobs {
    fn default() -> Self {
        SchedulerKnobs {
            config: SchedulerConfig::default(),
            seed: 0,
        }
    }
}

impl SchedulerKnobs {
    /// The C `#define` block the emitted scheduler reads for its knobs. Prepended to the stepped tier's C.
    /// The policy / clock enums match the emitted scheduler's `LOGOS_SCHED_*` / `LOGOS_CLOCK_*` constants
    /// one-to-one; `logos_ready_pop` branches on `LOGOS_SCHED_POLICY`, the timer/select tie-breaks draw
    /// from a SplitMix64 seeded by `LOGOS_SCHED_SEED`, and channels default to `LOGOS_CHAN_DEFAULT_CAP`.
    pub fn emit_defines(&self) -> String {
        let policy = match self.config.policy {
            SchedulePolicy::Fifo => "LOGOS_SCHED_FIFO",
            SchedulePolicy::Lifo => "LOGOS_SCHED_LIFO",
            SchedulePolicy::RoundRobin => "LOGOS_SCHED_ROUNDROBIN",
            SchedulePolicy::Random => "LOGOS_SCHED_RANDOM",
            SchedulePolicy::Priority => "LOGOS_SCHED_PRIORITY",
        };
        let clock = match self.config.clock {
            ClockMode::Logical => "LOGOS_CLOCK_LOGICAL",
            ClockMode::Wall => "LOGOS_CLOCK_WALL",
        };
        format!(
            "/* --- LOGOS scheduler knobs (mirror logicaffeine_runtime::SchedulerConfig) --------------- */\n\
             #define LOGOS_SCHED_FIFO 0\n\
             #define LOGOS_SCHED_LIFO 1\n\
             #define LOGOS_SCHED_ROUNDROBIN 2\n\
             #define LOGOS_SCHED_RANDOM 3\n\
             #define LOGOS_SCHED_PRIORITY 4\n\
             #define LOGOS_CLOCK_LOGICAL 0\n\
             #define LOGOS_CLOCK_WALL 1\n\
             #define LOGOS_SCHED_POLICY {policy}\n\
             #define LOGOS_SCHED_CLOCK {clock}\n\
             #define LOGOS_SCHED_WORKERS {workers}\n\
             #define LOGOS_SCHED_SEED {seed}ULL\n\
             #define LOGOS_CHAN_DEFAULT_CAP {cap}LL\n",
            workers = self.config.workers.max(1),
            seed = self.seed,
            cap = self.config.default_channel_capacity,
        )
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_c/concurrency/tests.rs"]
mod tests;
