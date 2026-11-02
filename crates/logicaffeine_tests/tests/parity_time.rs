//! Stdlib-parity — time domain (temporal builtins: parse_timestamp / *_of).
#![allow(unused_imports, dead_code)]
mod common;
use common::assert_all_tiers;

/// Parse an RFC-3339 timestamp + read its components (built-in temporal ops).
/// c: time.h.gmtime · py: datetime.fromisoformat · go: time.Parse · java: java.time.LocalDate.parse
#[test]
fn parity_time_parse() {
    assert_all_tiers(
        "## Main\nLet m be parse_timestamp(\"2024-03-15T10:30:45Z\").\nShow year_of(m).\nShow month_of(m).\nShow day_of(m).\n",
        "2024\n3\n15",
    );
}
/// Hour/minute/second components. py: datetime.hour · go: Time.Hour
#[test]
fn parity_time_clock() {
    assert_all_tiers(
        "## Main\nLet m be parse_timestamp(\"2024-03-15T10:30:45Z\").\nShow hour_of(m).\nShow minute_of(m).\nShow second_of(m).\n",
        "10\n30\n45",
    );
}

/// Day of week (built-in `weekday_of`). py: calendar.weekday · go: time.Weekday
#[test]
fn parity_time_weekday() {
    assert_all_tiers(
        "## Main\nShow weekday_of(parse_timestamp(\"2024-03-15T10:30:45Z\")).\n",
        "5",
    );
}

/// ISO week number (built-in `week_of`). go: time.ISOWeek
#[test]
fn parity_time_week() {
    assert_all_tiers(
        "## Main\nShow week_of(parse_timestamp(\"2024-03-15T10:30:45Z\")).\n",
        "11",
    );
}

/// Calendar quarter (built-in `quarter_of`).
#[test]
fn parity_time_quarter() {
    assert_all_tiers(
        "## Main\nShow quarter_of(parse_timestamp(\"2024-03-15T10:30:45Z\")).\nShow quarter_of(parse_timestamp(\"2024-11-01T00:00:00Z\")).\n",
        "1\n4",
    );
}

/// Seconds elapsed between two moments (built-in `seconds_between`). go: time.Sub / time.Since
#[test]
fn parity_time_seconds_between() {
    assert_all_tiers(
        "## Main\nLet m be parse_timestamp(\"2024-03-15T10:30:45Z\").\nLet n be parse_timestamp(\"2024-03-20T14:30:45Z\").\nShow seconds_between(m, n).\n",
        "446400",
    );
}

/// Whole months / years between moments (built-in `months_between` / `years_between`).
#[test]
fn parity_time_span() {
    assert_all_tiers(
        "## Main\nLet m be parse_timestamp(\"2020-01-15T00:00:00Z\").\nLet n be parse_timestamp(\"2024-07-15T00:00:00Z\").\nShow months_between(m, n).\nShow years_between(m, n).\n",
        "54\n4",
    );
}

/// Render a moment back to RFC-3339 (built-in `format_timestamp`). py: time.strftime · go: time.Format
#[test]
fn parity_time_format() {
    assert_all_tiers(
        "## Main\nShow format_timestamp(parse_timestamp(\"2024-03-15T10:30:45Z\")).\n",
        "2024-03-15T10:30:45Z",
    );
}

/// Advance a moment by N seconds (built-in `add_seconds`). py: datetime.timedelta · go: time.Add
#[test]
fn parity_time_add_seconds() {
    assert_all_tiers(
        "## Main\nShow format_timestamp(add_seconds(parse_timestamp(\"2024-03-15T10:30:45Z\"), 3600)).\n",
        "2024-03-15T11:30:45Z",
    );
}

/// Date and time-of-day parts (built-in `date_of` / `time_of`). go: time.Format
#[test]
fn parity_time_date_time_parts() {
    assert_all_tiers(
        "## Main\nLet m be parse_timestamp(\"2024-03-15T10:30:45Z\").\nShow date_of(m).\nShow time_of(m).\n",
        "2024-03-15\n10:30:45",
    );
}
