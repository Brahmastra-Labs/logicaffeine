//! Calendar arithmetic (Howard Hinnant's algorithms) and the clock.

use std::cell::Cell;

use crate::interpreter::RuntimeValue;

thread_local! {
    /// Test-clock override: (days since epoch, nanos since epoch). When set,
    /// `today`/`now` read it instead of the system clock — differential tests
    /// inject it so both engines see the same instant.
    static FIXED_CLOCK: Cell<Option<(i32, i64)>> = const { Cell::new(None) };
}

/// Pin `today`/`now` to a fixed instant (tests).
pub fn set_fixed_clock(days_since_epoch: i32, nanos_since_epoch: i64) {
    FIXED_CLOCK.with(|c| c.set(Some((days_since_epoch, nanos_since_epoch))));
}

/// Restore the system clock.
pub fn clear_fixed_clock() {
    FIXED_CLOCK.with(|c| c.set(None));
}

/// The injected fixed clock `(days, nanos)`, or `None` when the system clock is live. The spatial fabric
/// reads this so a `now`/`today` tile produces the SAME instant the VM does — a combinational circuit has
/// no wall clock, so a deterministic differential requires an injected one (tests set it around both runs).
pub fn fixed_clock() -> Option<(i32, i64)> {
    FIXED_CLOCK.with(|c| c.get())
}

/// A clock read the language names as a value: `today` reads the date, `now` the instant. The name reads the clock
/// wherever it appears, whatever a binding of the same name holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockRead {
    Today,
    Now,
}

impl ClockRead {
    /// Every clock read.
    pub const ALL: [ClockRead; 2] = [ClockRead::Today, ClockRead::Now];

    /// The name the read is written as.
    pub fn name(self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::Now => "now",
        }
    }

    /// The clock read `name` names, if it names one.
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|read| read.name() == name)
    }

    /// The type of what the read reads.
    pub fn logos_type(self) -> crate::analysis::types::LogosType {
        match self {
            Self::Today => crate::analysis::types::LogosType::Date,
            Self::Now => crate::analysis::types::LogosType::Moment,
        }
    }
}

/// The `today` builtin identifier.
pub fn today() -> RuntimeValue {
    if let Some((days, _)) = FIXED_CLOCK.with(|c| c.get()) {
        return RuntimeValue::Date(days);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        RuntimeValue::Date((duration.as_secs() / 86400) as i32)
    }
    #[cfg(target_arch = "wasm32")]
    {
        RuntimeValue::Date(0) // Placeholder for WASM
    }
}

/// The `now` builtin identifier.
pub fn now() -> RuntimeValue {
    if let Some((_, nanos)) = FIXED_CLOCK.with(|c| c.get()) {
        return RuntimeValue::Moment(nanos);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        RuntimeValue::Moment(duration.as_nanos() as i64)
    }
    #[cfg(target_arch = "wasm32")]
    {
        RuntimeValue::Moment(0) // Placeholder for WASM
    }
}

pub use logicaffeine_base::temporal::date_add_span;

/// Add a **calendar span** (`months` then `days`) to a SmoothUTC instant — the *civil* (wall-clock)
/// operation: months clamp at end-of-month and respect leap years, the time-of-day rides along
/// untouched. Distinct from adding a physical `Duration` (which is just nanosecond arithmetic).
/// Delegates to the same `base::temporal::add_span` the AOT mirror uses, so the tiers cannot diverge.
pub fn moment_add_span(nanos_since_epoch: i64, months: i32, days: i32) -> i64 {
    let dt = logicaffeine_base::temporal::civil_from_unix_nanos(nanos_since_epoch);
    let shifted = logicaffeine_base::temporal::add_span(dt, months as i64, days as i64);
    logicaffeine_base::temporal::unix_nanos_from_civil(shifted)
}

/// The number of days in a given month (1-indexed).
pub fn days_in_month(year: i32, month: i32) -> i32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
            if is_leap {
                29
            } else {
                28
            }
        }
        _ => 30, // Fallback
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/semantics/temporal/tests.rs"]
mod tests;
