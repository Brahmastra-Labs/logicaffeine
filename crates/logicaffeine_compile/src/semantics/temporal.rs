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

/// The `today` builtin identifier.
pub fn today() -> RuntimeValue {
    if let Some((days, _)) = FIXED_CLOCK.with(|c| c.get()) {
        return RuntimeValue::Date(days);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        let duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
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
        let duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        RuntimeValue::Moment(duration.as_nanos() as i64)
    }
    #[cfg(target_arch = "wasm32")]
    {
        RuntimeValue::Moment(0) // Placeholder for WASM
    }
}

/// Add months and days to a date (calendar-aware).
/// Uses Howard Hinnant's date algorithms for correct month-end handling
/// (e.g. Jan 31 + 1 month → Feb 28/29).
pub fn date_add_span(days_since_epoch: i32, months: i32, days: i32) -> i32 {
    // Convert days since epoch to (year, month, day).
    let z = days_since_epoch + 719468;
    let era = if z >= 0 { z / 146097 } else { (z - 146096) / 146097 };
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let mut year = y + if m <= 2 { 1 } else { 0 };
    let mut month = m as i32;
    let mut day = d as i32;

    // Add months (wrapping i32 — the Int spec applies to span math too).
    let total_months = year.wrapping_mul(12).wrapping_add(month - 1).wrapping_add(months);
    year = total_months / 12;
    month = total_months % 12 + 1;
    if month <= 0 {
        month += 12;
        year -= 1;
    }

    // Clamp day to valid range for the new month.
    let dim = days_in_month(year, month);
    if day > dim {
        day = dim;
    }

    // Convert back to days since epoch.
    let yp = year - if month <= 2 { 1 } else { 0 };
    let era2 = if yp >= 0 { yp / 400 } else { (yp - 399) / 400 };
    let yoe2 = (yp - era2 * 400) as u32;
    let mp2 = if month > 2 { month as u32 - 3 } else { month as u32 + 9 };
    let doy2 = (153 * mp2 + 2) / 5 + day as u32 - 1;
    let doe2 = yoe2 * 365 + yoe2 / 4 - yoe2 / 100 + doy2;
    let result = era2 * 146097 + doe2 as i32 - 719468;

    // Add days.
    result.wrapping_add(days)
}

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
mod tests {
    use super::*;

    #[test]
    fn month_end_clamps() {
        // 2024-01-31 is day 19753 since epoch; +1 month clamps to 2024-02-29 (leap).
        let jan31_2024 = 19753;
        let feb29_2024 = date_add_span(jan31_2024, 1, 0);
        assert_eq!(feb29_2024 - jan31_2024, 29);
    }

    #[test]
    fn days_in_month_handles_leap_years() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2024, 4), 30);
        assert_eq!(days_in_month(2024, 12), 31);
    }
}
