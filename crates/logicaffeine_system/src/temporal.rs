//! Temporal types for Logicaffeine.
//!
//! Provides Date and Moment types that complement std::time::Duration.

use std::fmt::{self, Display};

/// Date stored as days since Unix epoch (1970-01-01).
///
/// Range: ±5.8 million years from epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LogosDate(pub i32);

impl LogosDate {
    /// Create a new date from days since Unix epoch.
    #[inline]
    pub fn new(days: i32) -> Self {
        Self(days)
    }

    /// Get the raw days value.
    #[inline]
    pub fn days(&self) -> i32 {
        self.0
    }

    /// Convert to year, month, day using Howard Hinnant's algorithm.
    pub fn to_ymd(&self) -> (i64, i64, i64) {
        let z = self.0 as i64 + 719468; // shift epoch
        let era = if z >= 0 { z } else { z - 146096 } / 146097;
        let doe = z - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = mp + if mp < 10 { 3 } else { -9 };
        let year = y + if m <= 2 { 1 } else { 0 };
        (year, m, d)
    }
}

impl Display for LogosDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (year, month, day) = self.to_ymd();
        write!(f, "{:04}-{:02}-{:02}", year, month, day)
    }
}

impl crate::io::Showable for LogosDate {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

/// Moment stored as nanoseconds since Unix epoch.
///
/// Provides nanosecond precision for timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LogosMoment(pub i64);

impl LogosMoment {
    /// Create a new moment from nanoseconds since epoch.
    #[inline]
    pub fn new(nanos: i64) -> Self {
        Self(nanos)
    }

    /// Get the raw nanoseconds value.
    #[inline]
    pub fn nanos(&self) -> i64 {
        self.0
    }

    /// Get current moment (now).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn now() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Self(duration.as_nanos() as i64)
    }
}

impl Display for LogosMoment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // For now, just show as ISO-ish format with nanosecond precision
        let nanos = self.0;
        let seconds = nanos / 1_000_000_000;
        let remainder = nanos % 1_000_000_000;
        write!(f, "Moment({}s + {}ns)", seconds, remainder)
    }
}

impl crate::io::Showable for LogosMoment {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

/// Calendar span with separate month and day components.
///
/// Months and days are kept separate because they're **incommensurable**:
/// - "1 month" is 28-31 days depending on the month
/// - You can't convert months to days without knowing the reference date
///
/// Years fold into months (1 year = 12 months).
/// Weeks fold into days (1 week = 7 days).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LogosSpan {
    /// Total months (years * 12 + months)
    pub months: i32,
    /// Total days (weeks * 7 + days)
    pub days: i32,
}

impl LogosSpan {
    /// Create a new span from months and days.
    pub fn new(months: i32, days: i32) -> Self {
        Self { months, days }
    }

    /// Create a span from years, months, and days.
    /// Years are folded into months (1 year = 12 months).
    pub fn from_years_months_days(years: i32, months: i32, days: i32) -> Self {
        Self {
            months: years * 12 + months,
            days,
        }
    }

    /// Create a span from weeks and days.
    /// Weeks are folded into days (1 week = 7 days).
    pub fn from_weeks_days(weeks: i32, days: i32) -> Self {
        Self {
            months: 0,
            days: weeks * 7 + days,
        }
    }

    /// Negate the span (for "ago" operator).
    pub fn negate(&self) -> Self {
        Self {
            months: -self.months,
            days: -self.days,
        }
    }
}

impl Display for LogosSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();

        // Extract years from months
        let years = self.months / 12;
        let remaining_months = self.months % 12;

        if years != 0 {
            parts.push(if years.abs() == 1 {
                format!("{} year", years)
            } else {
                format!("{} years", years)
            });
        }

        if remaining_months != 0 {
            parts.push(if remaining_months.abs() == 1 {
                format!("{} month", remaining_months)
            } else {
                format!("{} months", remaining_months)
            });
        }

        if self.days != 0 || parts.is_empty() {
            parts.push(if self.days.abs() == 1 {
                format!("{} day", self.days)
            } else {
                format!("{} days", self.days)
            });
        }

        write!(f, "{}", parts.join(" and "))
    }
}

impl crate::io::Showable for LogosSpan {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_to_ymd_epoch() {
        let date = LogosDate(0);
        let (y, m, d) = date.to_ymd();
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn date_display() {
        let date = LogosDate(20593); // 2026-05-20
        assert_eq!(date.to_string(), "2026-05-20");
    }
}
