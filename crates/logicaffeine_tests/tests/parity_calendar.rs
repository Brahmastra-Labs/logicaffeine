#![allow(unused_imports, dead_code)]
mod common;
use common::assert_all_tiers;
#[test]
fn parity_calendar_isleap() {
    assert_all_tiers(
        "## Main\nShow isLeapYear(2000).\nShow isLeapYear(1900).\nShow isLeapYear(2024).\n",
        "true\nfalse\ntrue",
    );
}
#[test]
fn parity_calendar_days_in_month() {
    assert_all_tiers("## Main\nShow daysInMonth(2024, 2).\nShow daysInMonth(2023, 2).\nShow daysInMonth(2024, 4).\n", "29\n28\n30");
}
#[test]
fn parity_calendar_leapdays() {
    assert_all_tiers("## Main\nShow leapDaysBetween(2000, 2025).\n", "7");
}
#[test]
fn parity_calendar_month_name() {
    assert_all_tiers(
        "## Main\nShow monthName(3).\nShow monthName(12).\n",
        "March\nDecember",
    );
}
#[test]
fn parity_calendar_day_name() {
    assert_all_tiers(
        "## Main\nShow dayName(1).\nShow dayName(2).\nShow dayName(7).\n",
        "Monday\nTuesday\nSunday",
    );
}
/// Out of range (ISO weekdays are 1-7) answers "" rather than falling through to a neighbor.
#[test]
fn parity_calendar_day_name_out_of_range() {
    assert_all_tiers(
        "## Main\nShow length of dayName(0).\nShow length of dayName(8).\n",
        "0\n0",
    );
}
/// Abbreviated month name. py: calendar.month_abbr
#[test]
fn parity_calendar_month_abbr() {
    assert_all_tiers(
        "## Main\nShow monthAbbr(1).\nShow monthAbbr(9).\nShow monthAbbr(12).\n",
        "Jan\nSep\nDec",
    );
}
/// Abbreviated weekday name. py: calendar.day_abbr
#[test]
fn parity_calendar_day_abbr() {
    assert_all_tiers(
        "## Main\nShow dayAbbr(1).\nShow dayAbbr(2).\nShow dayAbbr(7).\n",
        "Mon\nTue\nSun",
    );
}
/// Out of range (ISO weekdays are 1-7) answers "" rather than falling through to a neighbor.
#[test]
fn parity_calendar_day_abbr_out_of_range() {
    assert_all_tiers(
        "## Main\nShow length of dayAbbr(0).\nShow length of dayAbbr(8).\n",
        "0\n0",
    );
}
/// The convention unification: `dayName`/`dayAbbr` share `the weekday of`'s ISO 8601 numbering
/// (Monday=1 .. Sunday=7), so composing them names the actual day. 2024-03-15 is a Friday.
#[test]
fn parity_calendar_dayname_of_weekday() {
    assert_all_tiers("## Main\nLet d be makeDate(2024, 3, 15).\nLet w be the weekday of d.\nShow dayName(w).\nShow dayAbbr(w).\n", "Friday\nFri");
}
/// 1-based day of the year, built on the (now O(1), built-in-backed) `daysBeforeMonth`.
/// 2024-03-15 = 31+29+15 = 75 (leap); 2024-12-31 = 366 (leap); 2023-12-31 = 365.
#[test]
fn parity_calendar_day_of_year() {
    assert_all_tiers("## Main\nShow dayOfYear(2024, 1, 1).\nShow dayOfYear(2024, 3, 15).\nShow dayOfYear(2023, 3, 1).\nShow dayOfYear(2024, 12, 31).\nShow dayOfYear(2023, 12, 31).\n", "1\n75\n60\n366\n365");
}
/// Seconds since the Unix epoch for a UTC calendar tuple. py: calendar.timegm
#[test]
fn parity_calendar_timegm() {
    assert_all_tiers(
        "## Main\nShow timegm(1970, 1, 1, 0, 0, 0).\nShow timegm(2024, 3, 15, 10, 30, 45).\nShow timegm(2000, 2, 29, 12, 0, 0).\nShow timegm(1969, 12, 31, 23, 59, 59).\n",
        "0\n1710498645\n951825600\n-1",
    );
}
