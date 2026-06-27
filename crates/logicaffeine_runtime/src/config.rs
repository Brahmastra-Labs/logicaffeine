//! Scheduler configuration — annotatable knobs with a nice default.
//!
//! The default ([`SchedulerConfig::default`]) is the fair, predictable,
//! fully-deterministic baseline: a FIFO ready queue on a logical clock. Programs
//! opt into the other disciplines (via a `## Scheduler: <policy>` decorator, wired
//! in a later phase) or callers build a config with the fluent setters.

/// The ready-task selection discipline. Determinism holds under *every* policy:
/// `Fifo`/`Lifo`/`RoundRobin`/`Priority` are deterministic by construction, and
/// `Random` resolves its pick through the seeded `Chooser`, so it is reproducible
/// under a fixed seed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SchedulePolicy {
    /// Deterministic FIFO ready queue — fair, predictable. **The default.**
    Fifo,
    /// LIFO (stack) — depth-first; better locality, but can starve siblings.
    Lifo,
    /// Round-robin rotation with a cooperative quantum.
    RoundRobin,
    /// Seeded-random ready pick — explores interleavings (fuzzing / race-finding).
    Random,
    /// Highest task priority first; FIFO within a priority band.
    Priority,
}

impl Default for SchedulePolicy {
    fn default() -> Self {
        SchedulePolicy::Fifo
    }
}

/// How the scheduler's clock advances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClockMode {
    /// Virtual time: `Sleep`/`After` are ordered logically and elapse instantly.
    /// Deterministic and wall-clock-free — the default, used by tests and TV.
    Logical,
    /// Real time: timers map to actual sleeps (production, non-replay runs).
    Wall,
}

impl Default for ClockMode {
    fn default() -> Self {
        ClockMode::Logical
    }
}

/// All scheduler knobs, bundled. Has a `Default` and fluent setters so it is easy
/// to annotate while still having a sensible baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerConfig {
    /// Ready-task selection discipline (default `Fifo`).
    pub policy: SchedulePolicy,
    /// Clock mode (default `Logical`).
    pub clock: ClockMode,
    /// Default capacity for channels created without an explicit one
    /// (default 32 — matches the AOT `mpsc` default).
    pub default_channel_capacity: usize,
    /// Cooperative yield interval, in scheduler steps (default 10_000).
    pub preempt_every: u32,
    /// M:N worker count; 1 = cooperative single-thread (default 1).
    pub workers: usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        SchedulerConfig {
            policy: SchedulePolicy::Fifo,
            clock: ClockMode::Logical,
            default_channel_capacity: 32,
            preempt_every: 10_000,
            workers: 1,
        }
    }
}

impl SchedulerConfig {
    /// Set the scheduling policy.
    pub fn with_policy(mut self, policy: SchedulePolicy) -> Self {
        self.policy = policy;
        self
    }
    /// Set the clock mode.
    pub fn with_clock(mut self, clock: ClockMode) -> Self {
        self.clock = clock;
        self
    }
    /// Set the default channel capacity.
    pub fn with_channel_capacity(mut self, capacity: usize) -> Self {
        self.default_channel_capacity = capacity;
        self
    }
    /// Set the worker count (M:N).
    pub fn with_workers(mut self, workers: usize) -> Self {
        self.workers = workers;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_default_is_fifo() {
        assert_eq!(SchedulerConfig::default().policy, SchedulePolicy::Fifo);
        assert_eq!(SchedulePolicy::default(), SchedulePolicy::Fifo);
    }

    #[test]
    fn config_default_is_the_nice_baseline() {
        let c = SchedulerConfig::default();
        assert_eq!(c.clock, ClockMode::Logical);
        assert_eq!(c.default_channel_capacity, 32);
        assert_eq!(c.workers, 1);
    }

    #[test]
    fn builder_overrides_apply() {
        let c = SchedulerConfig::default()
            .with_policy(SchedulePolicy::Random)
            .with_channel_capacity(4)
            .with_workers(8);
        assert_eq!(c.policy, SchedulePolicy::Random);
        assert_eq!(c.default_channel_capacity, 4);
        assert_eq!(c.workers, 8);
        assert_eq!(c.clock, ClockMode::Logical, "untouched knobs keep defaults");
    }
}
