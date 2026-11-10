//! Temporal types for Logicaffeine. The date, moment, time-of-day, calendar span and duration values are
//! `logicaffeine-data`'s ([`logicaffeine_data::temporal`]), where every value model reaches them, re-exported
//! here beside what only a system has: the clock, and how `Show` prints them.

use std::fmt::{self, Display};

pub use logicaffeine_data::temporal::*;

/// The language's `today`: the day `LOGOS_FIXED_DAYS` pins, when the environment pins one, else the day the system
/// clock reads. Every compiled tier reads the same variable, so a differential hands each the one instant.
#[cfg(not(target_arch = "wasm32"))]
pub fn today() -> LogosDate {
    match pinned("LOGOS_FIXED_DAYS") {
        Some(days) => LogosDate(days as i32),
        None => LogosDate((since_epoch().as_secs() / 86_400) as i32),
    }
}

/// The language's `now`: the instant `LOGOS_FIXED_NANOS` pins, when the environment pins one, else the instant the
/// system clock reads.
#[cfg(not(target_arch = "wasm32"))]
pub fn now() -> LogosMoment {
    match pinned("LOGOS_FIXED_NANOS") {
        Some(nanos) => LogosMoment(nanos),
        None => LogosMoment(since_epoch().as_nanos() as i64),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn pinned(variable: &str) -> Option<i64> {
    std::env::var(variable).ok()?.trim().parse().ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn since_epoch() -> std::time::Duration {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default()
}

impl crate::io::Showable for LogosDate {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl crate::io::Showable for LogosMoment {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl crate::io::Showable for LogosDuration {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl crate::io::Showable for LogosSpan {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl crate::io::Showable for LogosTime {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_system/temporal/tests.rs"]
mod tests;
