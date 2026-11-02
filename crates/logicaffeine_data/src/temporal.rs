//! The temporal values of a compiled program: a calendar date, an instant, a time of day and a calendar span,
//! with the arithmetic and the printing every tier shares (base's `temporal`), and each as an [`OtherValue`] a
//! [`LogosValue`](crate::exact::LogosValue) holds, so the exact extension's values meet them as the
//! interpreter's do. Reading the clock is the system's (`logicaffeine_system::temporal::now`).

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

    /// Construct a date from civil `(year, month, day)` via base `days_from_civil` — the AOT mirror
    /// of the `makeDate` builtin.
    #[inline]
    pub fn from_ymd(year: i64, month: i64, day: i64) -> Self {
        Self(logicaffeine_base::temporal::days_from_civil(year, month as u32, day as u32) as i32)
    }

    /// Get the raw days value.
    #[inline]
    pub fn days(&self) -> i32 {
        self.0
    }

    /// UTC calendar components — AOT mirrors of `year_of`/`month_of`/`day_of`/`weekday_of` on a Date.
    #[inline]
    pub fn year(&self) -> i64 {
        self.to_ymd().0
    }
    #[inline]
    pub fn month(&self) -> i64 {
        self.to_ymd().1
    }
    #[inline]
    pub fn day(&self) -> i64 {
        self.to_ymd().2
    }
    #[inline]
    pub fn weekday(&self) -> i64 {
        logicaffeine_base::temporal::iso_weekday_from_days(self.0 as i64) as i64
    }
    /// The ISO-8601 week number (1..=53) — the AOT mirror of `week_of` on a Date.
    #[inline]
    pub fn iso_week(&self) -> i64 {
        logicaffeine_base::temporal::iso_week_from_days(self.0 as i64).1 as i64
    }
    /// The calendar quarter (1..=4) — the AOT mirror of `quarter_of` on a Date.
    #[inline]
    pub fn quarter(&self) -> i64 {
        (self.month() - 1) / 3 + 1
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

impl LogosDate {
    /// This date moved by a calendar span: months first (clamped to the end of the month), then
    /// days — base `date_add_span`, the arithmetic every tier's `Date + Span` shares.
    #[inline]
    pub fn add_span(self, months: i32, days: i32) -> LogosDate {
        LogosDate(logicaffeine_base::temporal::date_add_span(
            self.0, months, days,
        ))
    }
}

impl std::ops::Add<LogosSpan> for LogosDate {
    type Output = LogosDate;
    #[inline]
    fn add(self, rhs: LogosSpan) -> LogosDate {
        self.add_span(rhs.months, rhs.days)
    }
}

impl std::ops::Sub<LogosSpan> for LogosDate {
    type Output = LogosDate;
    #[inline]
    fn sub(self, rhs: LogosSpan) -> LogosDate {
        self.add_span(rhs.months.wrapping_neg(), rhs.days.wrapping_neg())
    }
}

impl Display for LogosDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (year, month, day) = self.to_ymd();
        write!(f, "{:04}-{:02}-{:02}", year, month, day)
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

    /// Construct a UTC moment from civil `(year, month, day, hour, minute, second)` — the AOT mirror
    /// of the `makeMoment` builtin. Uses the same `days_from_civil` core as `calendar.timegm`, so the
    /// two agree to the second.
    #[inline]
    pub fn from_ymd(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> Self {
        let days = logicaffeine_base::temporal::days_from_civil(year, month as u32, day as u32);
        Self(
            days * logicaffeine_base::temporal::NANOS_PER_DAY
                + (hour * 3600 + minute * 60 + second)
                    * logicaffeine_base::temporal::NANOS_PER_SECOND,
        )
    }

    /// Get the raw nanoseconds value.
    #[inline]
    pub fn nanos(&self) -> i64 {
        self.0
    }

    /// Parse an RFC 3339 / ISO 8601 timestamp into a moment — the AOT mirror of the interpreter's
    /// `parse_timestamp`, stopping the program in the same words on text that is not one.
    #[inline]
    pub fn parse_rfc3339(s: &str) -> Self {
        Self(
            logicaffeine_base::temporal::parse_rfc3339(s)
                .unwrap_or_else(|| panic!("{}", crate::failure::not_a_timestamp(s))),
        )
    }

    /// Render this moment as an RFC 3339 / ISO 8601 UTC string — the AOT mirror of `format_timestamp`.
    #[inline]
    pub fn format_rfc3339(&self) -> String {
        logicaffeine_base::temporal::format_rfc3339(self.0)
    }

    /// UTC calendar components — the AOT mirrors of `year_of`/`month_of`/`day_of`/`weekday_of`.
    #[inline]
    pub fn year(&self) -> i64 {
        logicaffeine_base::temporal::civil_from_unix_nanos(self.0).year
    }
    #[inline]
    pub fn month(&self) -> i64 {
        logicaffeine_base::temporal::civil_from_unix_nanos(self.0).month as i64
    }
    #[inline]
    pub fn day(&self) -> i64 {
        logicaffeine_base::temporal::civil_from_unix_nanos(self.0).day as i64
    }
    #[inline]
    pub fn weekday(&self) -> i64 {
        logicaffeine_base::temporal::iso_weekday_from_days(
            self.0
                .div_euclid(logicaffeine_base::temporal::NANOS_PER_DAY),
        ) as i64
    }
    #[inline]
    pub fn hour(&self) -> i64 {
        logicaffeine_base::temporal::civil_from_unix_nanos(self.0).hour as i64
    }
    #[inline]
    pub fn minute(&self) -> i64 {
        logicaffeine_base::temporal::civil_from_unix_nanos(self.0).minute as i64
    }
    #[inline]
    pub fn second(&self) -> i64 {
        logicaffeine_base::temporal::civil_from_unix_nanos(self.0).second as i64
    }

    /// The calendar day this moment falls on (UTC) — the AOT mirror of `date_of`.
    #[inline]
    pub fn date(&self) -> LogosDate {
        LogosDate(
            self.0
                .div_euclid(logicaffeine_base::temporal::NANOS_PER_DAY) as i32,
        )
    }

    /// The wall-clock time-of-day (UTC) — the AOT mirror of `time_of`.
    #[inline]
    pub fn time_of_day(&self) -> LogosTime {
        LogosTime(
            self.0
                .rem_euclid(logicaffeine_base::temporal::NANOS_PER_DAY),
        )
    }

    /// The ISO-8601 week number (1..=53) — the AOT mirror of `week_of`.
    #[inline]
    pub fn iso_week(&self) -> i64 {
        let days = self
            .0
            .div_euclid(logicaffeine_base::temporal::NANOS_PER_DAY);
        logicaffeine_base::temporal::iso_week_from_days(days).1 as i64
    }

    /// The calendar quarter (1..=4) — the AOT mirror of `quarter_of`.
    #[inline]
    pub fn quarter(&self) -> i64 {
        (self.month() - 1) / 3 + 1
    }

    /// The moment `seconds` later (AOT mirror of `add_seconds`).
    #[inline]
    pub fn add_seconds(&self, seconds: i64) -> Self {
        Self(self.0 + seconds * 1_000_000_000)
    }

    /// Whole seconds from `self` to `other` (AOT mirror of `seconds_between`).
    #[inline]
    pub fn seconds_until(&self, other: &LogosMoment) -> i64 {
        (other.0 - self.0) / 1_000_000_000
    }

    /// Complete calendar months / years from `self` to `other` — AOT mirrors of
    /// `months_between` / `years_between` (signed, end-of-month-clamping correct).
    #[inline]
    pub fn months_until(&self, other: &LogosMoment) -> i64 {
        logicaffeine_base::temporal::months_between(self.0, other.0)
    }
    #[inline]
    pub fn years_until(&self, other: &LogosMoment) -> i64 {
        logicaffeine_base::temporal::years_between(self.0, other.0)
    }

    /// The local wall-clock time (with offset) in a named zone — AOT mirror of `in_zone`. Panics on
    /// an unknown zone, which the front-end has validated (it errors cleanly in the interpreter).
    pub fn in_zone(&self, zone_name: &str) -> String {
        logicaffeine_base::temporal::format_zoned(self.0, zone_name).unwrap_or_else(|| {
            panic!(
                "{}",
                crate::failure::unknown_time_zone(zone_name)
            )
        })
    }

    /// The local-as-UTC instant in a named zone — AOT mirror of `local_instant`. Composing a UTC
    /// component accessor onto the result reads the zone's LOCAL field (`the hour of m in "<zone>"`).
    pub fn local_instant(&self, zone_name: &str) -> LogosMoment {
        LogosMoment(
            logicaffeine_base::temporal::local_instant_nanos(self.0, zone_name).unwrap_or_else(
                || {
                    panic!(
                        "{}",
                        crate::failure::unknown_time_zone(zone_name)
                    )
                },
            ),
        )
    }

}

// `Moment + Duration` / `Moment − Duration` → a shifted Moment (the AOT mirror of the tree-walker's
// arith). Duration is `std::time::Duration` on this path; the nanos add/subtract on the i64 instant.
impl std::ops::Add<std::time::Duration> for LogosMoment {
    type Output = LogosMoment;
    #[inline]
    fn add(self, rhs: std::time::Duration) -> LogosMoment {
        LogosMoment(self.0.wrapping_add(rhs.as_nanos() as i64))
    }
}

impl std::ops::Sub<std::time::Duration> for LogosMoment {
    type Output = LogosMoment;
    #[inline]
    fn sub(self, rhs: std::time::Duration) -> LogosMoment {
        LogosMoment(self.0.wrapping_sub(rhs.as_nanos() as i64))
    }
}

// `Moment + Span` / `Moment − Span` is CIVIL calendar arithmetic — months clamp at end-of-month,
// leap years are respected, the wall time is preserved — distinct from the physical Duration path
// above. Delegates to the same `base::temporal::add_span` the interpreter's `moment_add_span` uses,
// so the AOT tier stays byte-identical to the tree-walker / VM.
impl LogosMoment {
    #[inline]
    fn add_span(self, months: i32, days: i32) -> LogosMoment {
        let dt = logicaffeine_base::temporal::civil_from_unix_nanos(self.0);
        let shifted = logicaffeine_base::temporal::add_span(dt, months as i64, days as i64);
        LogosMoment(logicaffeine_base::temporal::unix_nanos_from_civil(shifted))
    }
}

impl std::ops::Add<LogosSpan> for LogosMoment {
    type Output = LogosMoment;
    #[inline]
    fn add(self, rhs: LogosSpan) -> LogosMoment {
        self.add_span(rhs.months, rhs.days)
    }
}

impl std::ops::Sub<LogosSpan> for LogosMoment {
    type Output = LogosMoment;
    #[inline]
    fn sub(self, rhs: LogosSpan) -> LogosMoment {
        self.add_span(-rhs.months, -rhs.days)
    }
}

impl Display for LogosMoment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Civil UTC "YYYY-MM-DD HH:MM" — byte-identical to the interpreter's Moment Show. Euclidean
        // division so a pre-epoch (negative) Moment yields the correct date and a 0..86399 time-of-day.
        let total_seconds = self.0.div_euclid(1_000_000_000);
        let days = total_seconds.div_euclid(86_400);
        let day_seconds = total_seconds.rem_euclid(86_400);
        let hours = day_seconds / 3600;
        let minutes = (day_seconds % 3600) / 60;
        let (year, month, day) = LogosDate(days as i32).to_ymd();
        write!(
            f,
            "{:04}-{:02}-{:02} {:02}:{:02}",
            year, month, day, hours, minutes
        )
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


/// Time-of-day stored as nanoseconds from midnight — the wall-clock face of a `Moment`, with no
/// date or zone. Renders `HH:MM:SS[.frac]`, lossless to the nanosecond (the AOT mirror of the
/// interpreter's `Time`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LogosTime(pub i64);

impl LogosTime {
    #[inline]
    pub fn new(nanos_from_midnight: i64) -> Self {
        Self(nanos_from_midnight)
    }

    /// The raw nanoseconds-from-midnight value.
    #[inline]
    pub fn nanos(&self) -> i64 {
        self.0
    }
}

impl Display for LogosTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            logicaffeine_base::temporal::format_time_of_day(self.0)
        )
    }
}


// ── as values of the exact extension ─────────────────────────────────────────────────────────
//
// A date, a moment, a time of day or a span can reach a binding the exact extension also writes —
// the clock of a loop that runs beside an infinitesimal. There it rides in a `LogosValue` and still
// answers as itself: the interpreter's equalities, orders and calendar arithmetic, word for word.

use crate::exact::{LogosValue, OtherValue};
use logicaffeine_base::tower::Op;

const NANOS_PER_DAY: i64 = 86_400_000_000_000;

impl From<LogosDate> for LogosValue {
    fn from(d: LogosDate) -> Self {
        LogosValue::other(d)
    }
}

impl From<LogosMoment> for LogosValue {
    fn from(m: LogosMoment) -> Self {
        LogosValue::other(m)
    }
}

impl From<LogosTime> for LogosValue {
    fn from(t: LogosTime) -> Self {
        LogosValue::other(t)
    }
}

impl From<LogosSpan> for LogosValue {
    fn from(s: LogosSpan) -> Self {
        LogosValue::other(s)
    }
}

impl OtherValue for LogosDate {
    fn type_name(&self) -> &str {
        "Date"
    }

    fn display_string(&self) -> String {
        self.to_string()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn equals(&self, o: &dyn OtherValue) -> bool {
        o.as_any().downcast_ref::<LogosDate>() == Some(self)
    }

    fn order(&self, o: &LogosValue) -> Option<Option<std::cmp::Ordering>> {
        o.other_as::<LogosDate>().map(|d| Some(self.0.cmp(&d.0)))
    }

    fn combine(&self, op: Op, o: &LogosValue, reversed: bool) -> Option<Result<LogosValue, String>> {
        let span = *o.other_as::<LogosSpan>()?;
        match (op, reversed) {
            (Op::Add, false) => Some(Ok(LogosValue::other(*self + span))),
            (Op::Sub, false) => Some(Ok(LogosValue::other(
                self.add_span(-span.months, -span.days),
            ))),
            _ => None,
        }
    }
}

impl OtherValue for LogosMoment {
    fn type_name(&self) -> &str {
        "Moment"
    }

    fn display_string(&self) -> String {
        self.to_string()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn equals(&self, o: &dyn OtherValue) -> bool {
        o.as_any().downcast_ref::<LogosMoment>() == Some(self)
    }

    fn order(&self, o: &LogosValue) -> Option<Option<std::cmp::Ordering>> {
        if let Some(m) = o.other_as::<LogosMoment>() {
            return Some(Some(self.0.cmp(&m.0)));
        }
        o.other_as::<LogosTime>()
            .map(|t| Some(self.0.rem_euclid(NANOS_PER_DAY).cmp(&t.0)))
    }

    fn combine(&self, op: Op, o: &LogosValue, reversed: bool) -> Option<Result<LogosValue, String>> {
        let span = *o.other_as::<LogosSpan>()?;
        match (op, reversed) {
            (Op::Add, _) => Some(Ok(LogosValue::other(*self + span))),
            (Op::Sub, false) => Some(Ok(LogosValue::other(self.add_span(-span.months, -span.days)))),
            _ => None,
        }
    }
}

impl OtherValue for LogosTime {
    fn type_name(&self) -> &str {
        "Time"
    }

    fn display_string(&self) -> String {
        self.to_string()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn equals(&self, o: &dyn OtherValue) -> bool {
        o.as_any().downcast_ref::<LogosTime>() == Some(self)
    }

    fn order(&self, o: &LogosValue) -> Option<Option<std::cmp::Ordering>> {
        if let Some(t) = o.other_as::<LogosTime>() {
            return Some(Some(self.0.cmp(&t.0)));
        }
        o.other_as::<LogosMoment>()
            .map(|m| Some(self.0.cmp(&m.0.rem_euclid(NANOS_PER_DAY))))
    }

    fn combine(&self, _op: Op, _o: &LogosValue, _reversed: bool) -> Option<Result<LogosValue, String>> {
        None
    }
}

impl OtherValue for LogosSpan {
    fn type_name(&self) -> &str {
        "Span"
    }

    fn display_string(&self) -> String {
        self.to_string()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn equals(&self, o: &dyn OtherValue) -> bool {
        o.as_any().downcast_ref::<LogosSpan>() == Some(self)
    }

    fn order(&self, _o: &LogosValue) -> Option<Option<std::cmp::Ordering>> {
        None
    }

    fn combine(&self, _op: Op, _o: &LogosValue, _reversed: bool) -> Option<Result<LogosValue, String>> {
        None
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_data/temporal/tests.rs"]
mod tests;
