//! Temporal types for Logicaffeine. The date, moment, time-of-day and calendar span values are
//! `logicaffeine-data`'s ([`logicaffeine_data::temporal`]), where every value model reaches them, re-exported
//! here beside what only a system has: the clock, and how `Show` prints them.

use std::fmt::{self, Display};

pub use logicaffeine_data::temporal::*;

/// The instant the system clock reads now.
#[cfg(not(target_arch = "wasm32"))]
pub fn current_moment() -> LogosMoment {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    LogosMoment(duration.as_nanos() as i64)
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
