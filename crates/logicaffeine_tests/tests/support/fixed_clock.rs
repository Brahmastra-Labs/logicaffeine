//! The one instant every harness gives `today` and `now`: 2024-10-04 12:34:56 UTC.
//!
//! `today` and `now` read the wall clock, so a differential pins every tier to one instant: the VM and the
//! tree-walker through the thread's fixed clock ([`set_vm`]), a compiled program through the environment its
//! runtime reads ([`env`]).
#![allow(dead_code)]

/// 2024-10-04, in days since the Unix epoch.
pub const FIXED_DAYS: i64 = 20_000;

/// 2024-10-04 12:34:56 UTC, in nanoseconds since the Unix epoch.
pub const FIXED_NANOS: i64 = (FIXED_DAYS * 86_400 + 12 * 3_600 + 34 * 60 + 56) * 1_000_000_000;

/// The environment a compiled program reads the instant from.
pub fn env() -> [(&'static str, String); 2] {
    [
        ("LOGOS_FIXED_DAYS", FIXED_DAYS.to_string()),
        ("LOGOS_FIXED_NANOS", FIXED_NANOS.to_string()),
    ]
}

/// Pin this thread's VM and tree-walker to the instant.
pub fn set_vm() {
    logicaffeine_compile::semantics::temporal::set_fixed_clock(FIXED_DAYS as i32, FIXED_NANOS);
}
