use super::*;

/// A tiny deterministic RNG (SplitMix64) for reproducible fuzz.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

#[test]
fn local_instant_encodes_the_wall_clock_for_zoned_component_reads() {
    // The local-as-UTC instant, read with the plain UTC calendar, gives the zone's wall clock.
    let read = |ts: &str, zone: &str| {
        let inst = parse_rfc3339(ts).unwrap();
        let local = local_instant_nanos(inst, zone).unwrap();
        civil_from_unix_nanos(local)
    };
    // 2024-07-01T12:00Z in New York = EDT (-4) → 08:00 local, same day.
    let ny = read("2024-07-01T12:00:00Z", "America/New_York");
    assert_eq!((ny.hour, ny.day, ny.month), (8, 1, 7));
    // 2024-01-01T00:00Z in Kolkata (+5:30) → 05:30 local.
    let ist = read("2024-01-01T00:00:00Z", "Asia/Kolkata");
    assert_eq!((ist.hour, ist.minute, ist.day), (5, 30, 1));
    // 2024-07-01T02:00Z in New York (-4) → previous calendar day, 22:00 local (date rollover).
    let roll = read("2024-07-01T02:00:00Z", "America/New_York");
    assert_eq!((roll.hour, roll.day, roll.month), (22, 30, 6));
    // An unknown zone yields None, never a panic.
    assert_eq!(local_instant_nanos(0, "Mars/Olympus_Mons"), None);
}

#[test]
fn calendar_months_and_years_between_count_complete_periods() {
    let at = |ts: &str| parse_rfc3339(ts).unwrap();
    let months = |a: &str, b: &str| months_between(at(a), at(b));
    let years = |a: &str, b: &str| years_between(at(a), at(b));

    // Whole and partial months.
    assert_eq!(months("2024-01-15T00:00:00Z", "2024-03-15T00:00:00Z"), 2);
    assert_eq!(months("2024-01-15T00:00:00Z", "2024-03-14T00:00:00Z"), 1); // a day short
                                                                           // End-of-month clamp: Jan 31 → Feb 28 (2023, non-leap) IS a complete month.
    assert_eq!(months("2023-01-31T00:00:00Z", "2023-02-28T00:00:00Z"), 1);
    assert_eq!(months("2023-01-31T00:00:00Z", "2023-02-27T00:00:00Z"), 0);
    // Time-of-day precision: one hour short of a month is not a complete month.
    assert_eq!(months("2024-01-15T12:00:00Z", "2024-02-15T11:00:00Z"), 0);
    assert_eq!(months("2024-01-15T12:00:00Z", "2024-02-15T12:00:00Z"), 1);
    // Signed / antisymmetric.
    assert_eq!(months("2024-03-15T00:00:00Z", "2024-01-15T00:00:00Z"), -2);
    assert_eq!(months("2020-01-01T00:00:00Z", "2020-01-01T00:00:00Z"), 0);

    // Complete years are complete 12-month periods.
    assert_eq!(years("2020-06-01T00:00:00Z", "2024-06-01T00:00:00Z"), 4);
    assert_eq!(years("2020-06-01T00:00:00Z", "2024-03-01T00:00:00Z"), 3); // 4th year not reached
    assert_eq!(years("2020-06-01T00:00:00Z", "2024-05-31T00:00:00Z"), 3);
    assert_eq!(years("2024-01-01T00:00:00Z", "2020-01-01T00:00:00Z"), -4);
}

#[test]
fn time_of_day_formats_losslessly_to_the_nanosecond() {
    let h = 3_600 * NANOS_PER_SECOND;
    let m = 60 * NANOS_PER_SECOND;
    let s = NANOS_PER_SECOND;
    assert_eq!(format_time_of_day(0), "00:00:00");
    assert_eq!(format_time_of_day(16 * h), "16:00:00"); // 4pm
    assert_eq!(format_time_of_day(7 * h + 30 * m + 45 * s), "07:30:45");
    assert_eq!(format_time_of_day(23 * h + 59 * m + 59 * s), "23:59:59");
    // Sub-second fraction only when nonzero, trailing zeros trimmed.
    assert_eq!(
        format_time_of_day(7 * h + 30 * m + 45 * s + 500_000_000),
        "07:30:45.5"
    );
    assert_eq!(format_time_of_day(123_456_789), "00:00:00.123456789");
    // Normalised modulo a day: a full day wraps to midnight, a negative is the pre-midnight clock.
    assert_eq!(format_time_of_day(NANOS_PER_DAY), "00:00:00");
    assert_eq!(format_time_of_day(-1), "23:59:59.999999999");
}

#[test]
fn gregorian_known_anchors() {
    // The Unix epoch is day 0, a Thursday.
    assert_eq!(days_from_civil(1970, 1, 1), 0);
    assert_eq!(civil_from_days(0), (1970, 1, 1));
    assert_eq!(weekday_from_days(0), 4); // Thursday
    assert_eq!(iso_weekday_from_days(0), 4); // Thursday = ISO 4
                                             // Y2K: 2000-01-01 was a Saturday, day 10957.
    assert_eq!(days_from_civil(2000, 1, 1), 10957);
    assert_eq!(weekday_from_days(10957), 6); // Saturday
                                             // A day before the epoch.
    assert_eq!(days_from_civil(1969, 12, 31), -1);
    assert_eq!(civil_from_days(-1), (1969, 12, 31));
}

#[test]
fn gregorian_leap_years_and_month_lengths() {
    assert!(is_gregorian_leap(2000)); // divisible by 400
    assert!(!is_gregorian_leap(1900)); // century, not by 400
    assert!(is_gregorian_leap(2024));
    assert!(!is_gregorian_leap(2023));
    assert_eq!(last_day_of_month(2024, 2), 29);
    assert_eq!(last_day_of_month(2023, 2), 28);
    assert_eq!(last_day_of_month(2024, 1), 31);
    assert_eq!(last_day_of_month(2024, 4), 30);
    // Day-of-year: 2024-12-31 is day 366 (leap), 2023-12-31 is day 365.
    assert_eq!(day_of_year(2024, 12, 31), 366);
    assert_eq!(day_of_year(2023, 12, 31), 365);
    assert_eq!(day_of_year(2024, 3, 1), 61); // 31 + 29 + 1
}

#[test]
fn iso_week_date_edge_cases() {
    // 2021-01-01 (Friday) belongs to ISO week 53 of 2020.
    assert_eq!(
        iso_week_from_days(days_from_civil(2021, 1, 1)),
        (2020, 53, 5)
    );
    // 2020-12-31 (Thursday) is ISO 2020-W53-4.
    assert_eq!(
        iso_week_from_days(days_from_civil(2020, 12, 31)),
        (2020, 53, 4)
    );
    // 2023-01-01 (Sunday) is ISO 2022-W52-7.
    assert_eq!(
        iso_week_from_days(days_from_civil(2023, 1, 1)),
        (2022, 52, 7)
    );
    // 2024-01-01 (Monday) starts ISO 2024-W01-1 cleanly.
    assert_eq!(
        iso_week_from_days(days_from_civil(2024, 1, 1)),
        (2024, 1, 1)
    );
    // A 53-week ISO year: 2020 has 53 weeks; 2020-W53 exists.
    let (y, w, _) = iso_week_from_days(days_from_civil(2020, 12, 28)); // Monday of W53
    assert_eq!((y, w), (2020, 53));
}

#[test]
fn julian_gregorian_divergence() {
    // The Gregorian reform: 1582-10-15 Gregorian = 1582-10-05 Julian (the 10-day jump).
    let reform = days_from_civil(1582, 10, 15);
    assert_eq!(julian_from_days(reform), (1582, 10, 5));
    // In the 20th/21st century the Gregorian calendar runs 13 days ahead of Julian:
    // Gregorian 1970-01-01 is Julian 1969-12-19.
    assert_eq!(
        julian_from_days(days_from_civil(1970, 1, 1)),
        (1969, 12, 19)
    );
    // Round the other way: Julian 1969-12-19 maps back to the same day number.
    assert_eq!(days_from_julian(1969, 12, 19), 0);
    // Julian leap rule differs: 1900 is a leap year in Julian, not in Gregorian.
    assert!(is_julian_leap(1900));
    assert!(!is_gregorian_leap(1900));
}

#[test]
fn gregorian_round_trips_over_a_wide_range_under_fuzz() {
    // Property: civil_from_days ∘ days_from_civil = identity for every day across ±400k days
    // (~±1100 years around the epoch), and the day number is strictly monotonic in date.
    let mut prev = i64::MIN;
    let mut rng = Rng(0x_DA7E_C0DE_1234_5678);
    for _ in 0..50_000 {
        let z = (rng.next() % 800_001) as i64 - 400_000;
        let (y, m, d) = civil_from_days(z);
        assert!((1..=12).contains(&m), "month in range at z={z}");
        assert!(
            (1..=last_day_of_month(y, m)).contains(&d),
            "day in range at z={z}"
        );
        assert_eq!(days_from_civil(y, m, d), z, "round-trip at z={z}");
        // Weekday cycles correctly: consecutive days differ by one weekday.
        assert_eq!(weekday_from_days(z + 1), (weekday_from_days(z) + 1) % 7);
        let _ = prev;
        prev = z;
    }
}

#[test]
fn smooth_utc_instant_civil_decomposition() {
    let dt = |y, mo, d, h, mi, s, n| CivilDateTime {
        year: y,
        month: mo,
        day: d,
        hour: h,
        minute: mi,
        second: s,
        nanosecond: n,
    };
    // The epoch.
    assert_eq!(civil_from_unix_nanos(0), dt(1970, 1, 1, 0, 0, 0, 0));
    assert_eq!(unix_nanos_from_civil(dt(1970, 1, 1, 0, 0, 0, 0)), 0);
    // One full day.
    assert_eq!(
        unix_nanos_from_civil(dt(1970, 1, 2, 0, 0, 0, 0)),
        NANOS_PER_DAY
    );
    // A precise instant with sub-second nanos.
    let stamp = dt(2024, 2, 29, 13, 45, 30, 123_456_789); // a leap-day afternoon
    assert_eq!(civil_from_unix_nanos(unix_nanos_from_civil(stamp)), stamp);
    // Pre-epoch: −1 ns is the last nanosecond of 1969.
    assert_eq!(
        civil_from_unix_nanos(-1),
        dt(1969, 12, 31, 23, 59, 59, 999_999_999)
    );
    // Time-of-day extraction: noon = 12:00:00.
    assert_eq!(civil_from_unix_nanos(12 * 3600 * NANOS_PER_SECOND).hour, 12);
}

#[test]
fn calendar_span_arithmetic_clamps_and_rolls_over() {
    let dt = |y, mo, d, h, mi, s, n| CivilDateTime {
        year: y,
        month: mo,
        day: d,
        hour: h,
        minute: mi,
        second: s,
        nanosecond: n,
    };
    // End-of-month clamping: Jan 31 + 1 month = Feb 28 (2023) / Feb 29 (2024).
    assert_eq!(
        add_span(dt(2023, 1, 31, 9, 0, 0, 0), 1, 0),
        dt(2023, 2, 28, 9, 0, 0, 0)
    );
    assert_eq!(
        add_span(dt(2024, 1, 31, 9, 0, 0, 0), 1, 0),
        dt(2024, 2, 29, 9, 0, 0, 0)
    );
    // Time-of-day is preserved across a span.
    assert_eq!(
        add_span(dt(2024, 3, 15, 13, 45, 30, 7), 0, 1),
        dt(2024, 3, 16, 13, 45, 30, 7)
    );
    // Year rollover: Dec + 1 month = next Jan.
    assert_eq!(
        add_span(dt(2024, 12, 10, 0, 0, 0, 0), 1, 0),
        dt(2025, 1, 10, 0, 0, 0, 0)
    );
    // Days roll over months: Jan 15 + 20 days = Feb 4.
    assert_eq!(
        add_span(dt(2024, 1, 15, 0, 0, 0, 0), 0, 20),
        dt(2024, 2, 4, 0, 0, 0, 0)
    );
    // Negative months: Mar 31 − 1 month = Feb 29 (clamped, leap year).
    assert_eq!(
        add_span(dt(2024, 3, 31, 0, 0, 0, 0), -1, 0),
        dt(2024, 2, 29, 0, 0, 0, 0)
    );
    // +12 months = same date next year (a clean year).
    assert_eq!(
        add_span(dt(2024, 6, 15, 0, 0, 0, 0), 12, 0),
        dt(2025, 6, 15, 0, 0, 0, 0)
    );
}

#[test]
fn business_day_math() {
    let d = |y, mo, day| days_from_civil(y, mo, day);
    // 2024-03-11 is a Monday; 2024-03-18 the next Monday → 5 business days between.
    assert_eq!(business_days_between(d(2024, 3, 11), d(2024, 3, 18)), 5);
    // Within one week, Mon→Fri is 4 business days; the weekend adds none.
    assert_eq!(business_days_between(d(2024, 3, 11), d(2024, 3, 15)), 4);
    assert_eq!(business_days_between(d(2024, 3, 15), d(2024, 3, 18)), 1); // Fri→Mon (skips weekend)
                                                                          // Direction is signed.
    assert_eq!(business_days_between(d(2024, 3, 18), d(2024, 3, 11)), -5);
    // Friday + 1 business day = Monday; Monday − 1 = previous Friday.
    assert_eq!(add_business_days(d(2024, 3, 15), 1), d(2024, 3, 18)); // Fri → Mon
    assert_eq!(add_business_days(d(2024, 3, 18), -1), d(2024, 3, 15)); // Mon → Fri
                                                                       // Five business days from Monday lands on the next Monday.
    assert_eq!(add_business_days(d(2024, 3, 11), 5), d(2024, 3, 18));
    // Weekends are weekends.
    assert!(is_weekend(d(2024, 3, 16))); // Saturday
    assert!(is_weekend(d(2024, 3, 17))); // Sunday
    assert!(!is_weekend(d(2024, 3, 15))); // Friday
}

#[test]
fn business_day_invariants_under_fuzz() {
    // Properties (from a WEEKDAY anchor, where business-day counting is unambiguous):
    // advancing n business days never lands on a weekend, and the business-day count from the
    // anchor to its n-later image is exactly n. `n > 0` advances strictly into the future.
    let mut rng = Rng(0x_B17A_DA75_C0DE_2468);
    for _ in 0..20_000 {
        let z0 = (rng.next() % 200_001) as i64 - 100_000;
        let z = if is_weekend(z0) {
            add_business_days(z0, 1)
        } else {
            z0
        }; // a weekday
        let n = (rng.next() % 39) as i64 + 1; // 1..=39 business days
        let fwd = add_business_days(z, n);
        assert!(!is_weekend(fwd), "lands on a business day at z={z}, n={n}");
        assert!(fwd > z, "advancing moves strictly forward at z={z}, n={n}");
        assert_eq!(
            business_days_between(z, fwd),
            n,
            "count matches at z={z}, n={n}"
        );
    }
}

/// A hand-built US-Eastern-style zone for 2024: EST = UTC−5h, EDT = UTC−4h.
/// Spring forward 2024-03-10 07:00 UTC (02:00 EST → 03:00 EDT); fall back 2024-11-03 06:00 UTC
/// (02:00 EDT → 01:00 EST).
fn eastern_2024() -> (Vec<ZoneTransition>, i32) {
    let spring = days_from_civil(2024, 3, 10) * SECONDS_PER_DAY + 7 * 3600;
    let fall = days_from_civil(2024, 11, 3) * SECONDS_PER_DAY + 6 * 3600;
    (
        vec![
            ZoneTransition {
                at_unix_seconds: spring,
                offset_seconds: -4 * 3600,
            },
            ZoneTransition {
                at_unix_seconds: fall,
                offset_seconds: -5 * 3600,
            },
        ],
        -5 * 3600, // base: EST
    )
}

fn utc(y: i64, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
    unix_nanos_from_civil(CivilDateTime {
        year: y,
        month: mo,
        day: d,
        hour: h,
        minute: mi,
        second: 0,
        nanosecond: 0,
    })
}

#[test]
fn timezone_utc_to_local_picks_the_offset_in_effect() {
    let (tz, base) = eastern_2024();
    let local = |ns| {
        let c = to_local(&tz, base, ns);
        (c.month, c.day, c.hour, c.minute)
    };
    // Winter → EST (−5): 2024-01-01 12:00 UTC = 07:00 local.
    assert_eq!(local(utc(2024, 1, 1, 12, 0)), (1, 1, 7, 0));
    // Summer → EDT (−4): 2024-07-01 12:00 UTC = 08:00 local.
    assert_eq!(local(utc(2024, 7, 1, 12, 0)), (7, 1, 8, 0));
    // Just after fall-back → back to EST: 2024-12-01 12:00 UTC = 07:00 local.
    assert_eq!(local(utc(2024, 12, 1, 12, 0)), (12, 1, 7, 0));
    // The offset function directly.
    assert_eq!(offset_at(&tz, base, utc(2024, 7, 1, 0, 0)), -4 * 3600);
    assert_eq!(offset_at(&tz, base, utc(2024, 1, 1, 0, 0)), -5 * 3600);
}

#[test]
fn timezone_local_to_utc_round_trip_and_dst_fold() {
    let (tz, base) = eastern_2024();
    let cdt = |y, mo, d, h, mi| CivilDateTime {
        year: y,
        month: mo,
        day: d,
        hour: h,
        minute: mi,
        second: 0,
        nanosecond: 0,
    };
    // Unambiguous local → UTC round-trips: 08:00 EDT on 2024-07-01 = 12:00 UTC.
    let summer_local = cdt(2024, 7, 1, 8, 0);
    let summer_utc = from_local(&tz, base, summer_local, Fold::Later);
    assert_eq!(summer_utc, utc(2024, 7, 1, 12, 0));
    assert_eq!(to_local(&tz, base, summer_utc), summer_local);
    // DST FOLD: 2024-11-03 01:30 local occurs twice. Earlier = 05:30 UTC (still EDT −4),
    // Later = 06:30 UTC (now EST −5).
    let ambiguous = cdt(2024, 11, 3, 1, 30);
    assert_eq!(
        from_local(&tz, base, ambiguous, Fold::Earlier),
        utc(2024, 11, 3, 5, 30)
    );
    assert_eq!(
        from_local(&tz, base, ambiguous, Fold::Later),
        utc(2024, 11, 3, 6, 30)
    );
    // DST GAP: 2024-03-10 02:30 local never occurs (clocks jump 02:00→03:00). Resolution is
    // deterministic and the two folds differ (boundary instants of the skipped hour).
    let nonexistent = cdt(2024, 3, 10, 2, 30);
    let gap_earlier = from_local(&tz, base, nonexistent, Fold::Earlier);
    let gap_later = from_local(&tz, base, nonexistent, Fold::Later);
    assert_ne!(gap_earlier, gap_later, "the two gap resolutions differ");
}

#[test]
fn dst_rule_evaluator_generates_correct_transitions() {
    // Nth-weekday anchors.
    assert_eq!(nth_weekday_of_month(2024, 3, 2, 0), 10); // 2nd Sunday of March 2024
    assert_eq!(nth_weekday_of_month(2024, 11, 1, 0), 3); // 1st Sunday of November 2024
    assert_eq!(nth_weekday_of_month(2024, 10, 5, 0), 27); // last Sunday of October 2024 (EU rule)
    assert_eq!(nth_weekday_of_month(2024, 2, 5, 4), 29); // last Thursday of Feb 2024 (leap)
                                                         // The US Eastern POSIX rule `EST5EDT,M3.2.0,M11.1.0` generates exactly the hand-built 2024
                                                         // transitions (spring 2024-03-10 07:00Z → EDT, fall 2024-11-03 06:00Z → EST).
    let start = DstRule {
        month: 3,
        week: 2,
        weekday: 0,
        time_seconds: 2 * 3600,
    };
    let end = DstRule {
        month: 11,
        week: 1,
        weekday: 0,
        time_seconds: 2 * 3600,
    };
    let gen = dst_transitions_for_year(-5 * 3600, -4 * 3600, start, end, 2024);
    let (hand, _) = eastern_2024();
    assert_eq!(
        gen.to_vec(),
        hand,
        "generated transitions match the hand-built zone"
    );
    // The generated zone localizes correctly: summer = EDT (−4), winter = EST (−5).
    let tz = gen.to_vec();
    let hour = |y, mo, d| to_local(&tz, -5 * 3600, utc(y, mo, d, 12, 0)).hour;
    assert_eq!(hour(2024, 7, 1), 8); // 12:00Z → 08:00 EDT
    assert_eq!(hour(2024, 1, 1), 7); // 12:00Z → 07:00 EST
}

#[test]
fn nth_weekday_invariants_under_fuzz() {
    // Property: the result is a valid day of the month, has the requested weekday, and lies in
    // the requested week band (or is the last such weekday when week = 5).
    let mut rng = Rng(0x_4EE7_DA42_C0DE_9001);
    for _ in 0..20_000 {
        let year = 1900 + (rng.next() % 300) as i64;
        let month = (rng.next() % 12) as u32 + 1;
        let weekday = (rng.next() % 7) as u32;
        let week = (rng.next() % 5) as u32 + 1; // 1..=5
        let day = nth_weekday_of_month(year, month, week, weekday);
        assert!(
            (1..=last_day_of_month(year, month)).contains(&day),
            "valid day {year}-{month} w{week}"
        );
        assert_eq!(
            weekday_from_days(days_from_civil(year, month, day)),
            weekday,
            "weekday matches"
        );
        // It is indeed the week-th (or last) such weekday: one week earlier is a different month
        // for week 1, or the prior occurrence; one week later overflows the month for "last".
        if week < 5 {
            assert_eq!((day - 1) / 7 + 1, week, "in the requested week band");
        } else {
            assert!(
                day + 7 > last_day_of_month(year, month),
                "week 5 is the last occurrence"
            );
        }
    }
}

#[test]
fn format_zoned_shows_local_time_with_offset() {
    let u = |y, mo, d, h| {
        unix_nanos_from_civil(CivilDateTime {
            year: y,
            month: mo,
            day: d,
            hour: h,
            minute: 0,
            second: 0,
            nanosecond: 0,
        })
    };
    // Summer → EDT (−4); winter → EST (−5).
    assert_eq!(
        format_zoned(u(2024, 7, 1, 12), "America/New_York").as_deref(),
        Some("2024-07-01T08:00:00-04:00")
    );
    assert_eq!(
        format_zoned(u(2024, 1, 1, 12), "America/New_York").as_deref(),
        Some("2024-01-01T07:00:00-05:00")
    );
    // UTC and a half-hour positive offset.
    assert_eq!(
        format_zoned(u(2024, 1, 1, 12), "UTC").as_deref(),
        Some("2024-01-01T12:00:00+00:00")
    );
    assert_eq!(
        format_zoned(u(2024, 1, 1, 0), "Asia/Kolkata").as_deref(),
        Some("2024-01-01T05:30:00+05:30")
    );
    // Unknown zone → None.
    assert_eq!(format_zoned(0, "Mars/Base"), None);
}

#[test]
fn named_zone_registry_localizes_with_dst() {
    let h = |z: &ZoneSpec, y, mo, d, hh| z.to_local(utc(y, mo, d, hh, 0)).hour;
    // America/New_York: EDT (−4) in summer, EST (−5) in winter.
    let ny = zone_by_name("America/New_York").unwrap();
    assert_eq!(h(&ny, 2024, 7, 1, 12), 8); // 12:00Z → 08:00 EDT
    assert_eq!(h(&ny, 2024, 1, 1, 12), 7); // 12:00Z → 07:00 EST
                                           // UTC: fixed, no DST.
    let u = zone_by_name("UTC").unwrap();
    assert!(u.dst.is_none());
    assert_eq!(h(&u, 2024, 7, 1, 12), 12);
    // Europe/London: BST (+1) in summer, GMT (0) in winter.
    let lon = zone_by_name("Europe/London").unwrap();
    assert_eq!(h(&lon, 2024, 7, 1, 12), 13); // BST
    assert_eq!(h(&lon, 2024, 1, 1, 12), 12); // GMT
                                             // Asia/Kolkata: fixed +5:30.
    let ist = zone_by_name("Asia/Kolkata").unwrap();
    let c = ist.to_local(utc(2024, 1, 1, 0, 0));
    assert_eq!((c.hour, c.minute), (5, 30));
    // Australia/Sydney (southern hemisphere): AEDT (+11) in January, AEST (+10) in July.
    let syd = zone_by_name("Australia/Sydney").unwrap();
    assert_eq!(h(&syd, 2024, 1, 1, 0), 11); // 00:00Z → 11:00 AEDT (summer down under)
    assert_eq!(h(&syd, 2024, 7, 1, 0), 10); // 00:00Z → 10:00 AEST (winter)
                                            // Unknown zone → None.
    assert!(zone_by_name("Mars/Olympus_Mons").is_none());
}

#[test]
fn timezone_round_trips_unambiguous_locals_under_fuzz() {
    // Property: outside the fall-back fold hour, to_local ∘ from_local = identity. We test by
    // generating UTC instants (each maps to exactly one local), converting to local and back.
    let (tz, base) = eastern_2024();
    let mut rng = Rng(0x_72ED_C0DE_8888_4321);
    let year_start = utc(2024, 1, 1, 0, 0);
    for _ in 0..20_000 {
        let ns = year_start + (rng.next() % (300 * NANOS_PER_DAY as u64)) as i64;
        let local = to_local(&tz, base, ns);
        // from_local with the matching fold recovers the instant: pick whichever fold matches.
        let back_e = from_local(&tz, base, local, Fold::Earlier);
        let back_l = from_local(&tz, base, local, Fold::Later);
        assert!(
            back_e == ns || back_l == ns,
            "instant recovered from its local time (ns={ns}, e={back_e}, l={back_l})"
        );
    }
}

#[test]
fn rfc3339_format_and_parse_round_trip() {
    let at = |y, mo, d, h, mi, s, n| CivilDateTime {
        year: y,
        month: mo,
        day: d,
        hour: h,
        minute: mi,
        second: s,
        nanosecond: n,
    };
    // The epoch, no fraction.
    assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
    assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
    // Full nanosecond precision, round-trip.
    let ns = unix_nanos_from_civil(at(2024, 3, 10, 7, 30, 0, 123_456_789));
    assert_eq!(format_rfc3339(ns), "2024-03-10T07:30:00.123456789Z");
    assert_eq!(parse_rfc3339("2024-03-10T07:30:00.123456789Z"), Some(ns));
    // Trailing zeros trimmed: half a second is `.5`.
    let half = unix_nanos_from_civil(at(2000, 1, 1, 0, 0, 0, 500_000_000));
    assert_eq!(format_rfc3339(half), "2000-01-01T00:00:00.5Z");
    assert_eq!(parse_rfc3339("2000-01-01T00:00:00.5Z"), Some(half));
    // Numeric offsets normalize to the same UTC instant as 07:30Z.
    let utc_730 = unix_nanos_from_civil(at(2024, 3, 10, 7, 30, 0, 0));
    assert_eq!(parse_rfc3339("2024-03-10T03:30:00-04:00"), Some(utc_730));
    assert_eq!(parse_rfc3339("2024-03-10T13:00:00+05:30"), Some(utc_730));
    // Pre-epoch.
    assert_eq!(format_rfc3339(-NANOS_PER_SECOND), "1969-12-31T23:59:59Z");
    assert_eq!(
        parse_rfc3339("1969-12-31T23:59:59Z"),
        Some(-NANOS_PER_SECOND)
    );
    // Malformed inputs are a clean None (never a panic).
    assert_eq!(parse_rfc3339("not a date"), None);
    assert_eq!(parse_rfc3339("2024-13-01T00:00:00Z"), None); // month out of range
    assert_eq!(parse_rfc3339("2024-02-30T00:00:00Z"), None); // day out of range
    assert_eq!(parse_rfc3339("2024-03-10 07:30:00Z"), None); // missing 'T'
    assert_eq!(parse_rfc3339("2024-03-10T07:30:00"), None); // missing zone
}

#[test]
fn duration_format_and_parse() {
    let s = NANOS_PER_SECOND;
    // Canonical forms.
    assert_eq!(format_duration(0), "0s");
    assert_eq!(format_duration(90 * 60 * s), "1h30m");
    assert_eq!(format_duration(2 * 86_400 * s), "2d");
    assert_eq!(format_duration(s + s / 2), "1.5s");
    assert_eq!(format_duration(s / 2), "500ms");
    assert_eq!(format_duration(-(90 * 60 * s)), "-1h30m");
    assert_eq!(
        format_duration(86_400 * s + 2 * 3600 * s + 3 * 60 * s + 4 * s),
        "1d2h3m4s"
    );
    assert_eq!(format_duration(1500), "1.5us");
    assert_eq!(format_duration(42), "42ns");
    // Parse the canonical forms back.
    assert_eq!(parse_duration("1h30m"), Some(90 * 60 * s));
    assert_eq!(parse_duration("90m"), Some(90 * 60 * s)); // non-canonical but valid
    assert_eq!(parse_duration("1.5s"), Some(s + s / 2));
    assert_eq!(parse_duration("500ms"), Some(s / 2));
    assert_eq!(parse_duration("-1h"), Some(-(3600 * s)));
    assert_eq!(parse_duration("2d"), Some(2 * 86_400 * s));
    assert_eq!(
        parse_duration("1d2h3m4s"),
        Some(86_400 * s + 2 * 3600 * s + 3 * 60 * s + 4 * s)
    );
    assert_eq!(parse_duration(".5s"), Some(s / 2)); // leading-dot fraction
    assert_eq!(parse_duration("1µs"), Some(1_000)); // micro sign accepted
    assert_eq!(parse_duration("250us"), Some(250_000));
    // Malformed → None (never a panic).
    assert_eq!(parse_duration(""), None);
    assert_eq!(parse_duration("abc"), None);
    assert_eq!(parse_duration("1x"), None); // unknown unit
    assert_eq!(parse_duration("h"), None); // no number
}

#[test]
fn duration_round_trips_under_fuzz() {
    // Property: parse_duration ∘ format_duration = identity for any duration.
    let mut rng = Rng(0x_D012_A710_C0DE_4444);
    for _ in 0..20_000 {
        let ns = (rng.next() as i64).wrapping_rem(9_000_000_000_000_000_000);
        assert_eq!(
            parse_duration(&format_duration(ns)),
            Some(ns),
            "round-trip {ns}"
        );
    }
}

#[test]
fn rfc3339_round_trips_under_fuzz() {
    // Property: parse_rfc3339 ∘ format_rfc3339 = identity for any instant.
    let mut rng = Rng(0x_3339_C0DE_ABCD_1234);
    for _ in 0..20_000 {
        let ns = (rng.next() as i64).wrapping_rem(6_000_000_000_000_000_000);
        let s = format_rfc3339(ns);
        assert_eq!(parse_rfc3339(&s), Some(ns), "round-trip {s}");
    }
}

#[test]
fn sync_clock_merge_is_a_crdt_join() {
    use std::collections::BTreeMap;
    let mk = |hp: i64, node: u64, l: i64, u: i64| {
        let mut k = BTreeMap::new();
        k.insert(node, EstimateInterval::new(l, u));
        SyncClock {
            hlc: Hlc {
                physical_nanos: hp,
                logical: 0,
            },
            knowledge: k,
        }
    };
    let a = mk(100, 1, 10, 50);
    let b = mk(200, 1, 20, 40);
    let c = mk(150, 2, 5, 9);
    // CRDT laws.
    assert_eq!(a.merge(&a), a, "idempotent");
    assert_eq!(a.merge(&b), b.merge(&a), "commutative");
    assert_eq!(a.merge(&b).merge(&c), a.merge(&b.merge(&c)), "associative");
    // HLC takes the max; a shared node intersects; a distinct node is carried.
    let m = a.merge(&b);
    assert_eq!(
        m.hlc,
        Hlc {
            physical_nanos: 200,
            logical: 0
        }
    );
    assert_eq!(m.knowledge[&1], EstimateInterval::new(20, 40)); // [10,50] ∩ [20,40]
    let m2 = a.merge(&c);
    assert_eq!(m2.knowledge[&1], EstimateInterval::new(10, 50));
    assert_eq!(m2.knowledge[&2], EstimateInterval::new(5, 9));
}

#[test]
fn sync_clock_converges_regardless_of_order_and_intervals_only_tighten() {
    use std::collections::BTreeMap;
    let make = |hp: i64, node: u64, l: i64, u: i64| {
        let mut k = BTreeMap::new();
        k.insert(node, EstimateInterval::new(l, u));
        SyncClock {
            hlc: Hlc {
                physical_nanos: hp,
                logical: 0,
            },
            knowledge: k,
        }
    };
    let a = make(100, 1, 0, 100);
    let b = make(300, 1, 40, 80);
    // Two replicas exchange and converge to the SAME state, whichever order.
    assert_eq!(a.merge(&b), b.merge(&a));
    // Knowledge only tightens: the merged interval is no wider than either input.
    let merged_iv = a.merge(&b).knowledge[&1];
    assert!(merged_iv.width() <= a.knowledge[&1].width());
    assert!(merged_iv.width() <= b.knowledge[&1].width());
    // intersect itself is identity (idempotent at the interval level too).
    let iv = EstimateInterval::new(10, 90);
    assert_eq!(iv.intersect(&iv), iv);
    assert_eq!(
        iv.intersect(&EstimateInterval::new(30, 60)),
        EstimateInterval::new(30, 60)
    );
}

#[test]
fn sync_clock_tick_observe_and_light_cone_horizon() {
    let c = SyncClock::new();
    let c2 = c.tick(500);
    assert!(c2.hlc > c.hlc, "tick advances causally");
    // observe records knowledge for a node.
    let c3 = c2.observe(7, EstimateInterval::new(100, 200));
    assert_eq!(c3.knowledge[&7], EstimateInterval::new(100, 200));
    // observing again tightens (intersects) rather than overwrites.
    let c4 = c3.observe(7, EstimateInterval::new(150, 250));
    assert_eq!(c4.knowledge[&7], EstimateInterval::new(150, 200));
    // You cannot know a node 0.2 s away fresher than now − 0.2 s.
    assert_eq!(knowable_horizon(1_000_000_000, 200_000_000), 800_000_000);
}

#[test]
fn hybrid_logical_clock_tick_and_receive() {
    let hlc = |p: i64, l: u32| Hlc {
        physical_nanos: p,
        logical: l,
    };
    // First tick adopts wall time; a stalled clock advances the logical counter.
    assert_eq!(Hlc::ZERO.tick(100), hlc(100, 0));
    assert_eq!(hlc(100, 0).tick(100), hlc(100, 1)); // physical didn't move → logical++
    assert_eq!(hlc(100, 1).tick(200), hlc(200, 0)); // physical advanced → logical reset
                                                    // A backward wall clock is absorbed by the logical counter (never goes back).
    assert_eq!(hlc(200, 0).tick(150), hlc(200, 1));
    // Receive: equal physicals → max(logical)+1.
    assert_eq!(hlc(100, 5).recv(hlc(100, 3), 100), hlc(100, 6));
    // Receive: remote ahead → take remote physical, remote.logical+1.
    assert_eq!(hlc(100, 5).recv(hlc(200, 2), 100), hlc(200, 3));
    // Receive: local ahead → local.logical+1.
    assert_eq!(hlc(200, 7).recv(hlc(100, 9), 150), hlc(200, 8));
    // Receive: wall time beats both → reset logical.
    assert_eq!(hlc(100, 5).recv(hlc(50, 9), 300), hlc(300, 0));
    // The derived Ord is the HLC total order.
    assert!(hlc(100, 0) < hlc(100, 1) && hlc(100, 1) < hlc(200, 0));
}

#[test]
fn hlc_preserves_causality_under_fuzz() {
    // Property: tick and recv always produce a timestamp strictly greater than every input —
    // so a send-then-receive chain is monotonic and happens-before is never inverted.
    let mut rng = Rng(0x_C0DE_C0FF_EE17_AAAA);
    let mut local = Hlc::ZERO;
    let mut remote = Hlc::ZERO;
    for _ in 0..20_000 {
        let now = (rng.next() % 1_000_000) as i64;
        if rng.next() & 1 == 0 {
            let next = local.tick(now);
            assert!(next > local, "tick advances");
            local = next;
        } else {
            let before = local;
            let next = local.recv(remote, now);
            assert!(next > before && next > remote, "recv dominates both inputs");
            local = next;
            remote = remote.tick((rng.next() % 1_000_000) as i64); // remote evolves independently
        }
    }
}

#[test]
fn leap_second_tai_utc_offsets() {
    let at = |y, mo, d| days_from_civil(y, mo, d) * SECONDS_PER_DAY;
    // Known TAI−UTC values from the IERS table.
    assert_eq!(tai_minus_utc(at(2017, 1, 1)), 37); // latest leap second
    assert_eq!(tai_minus_utc(at(2024, 6, 1)), 37); // unchanged since 2017
    assert_eq!(tai_minus_utc(at(1972, 1, 1)), 10); // table origin
    assert_eq!(tai_minus_utc(at(2000, 1, 1)), 32);
    assert_eq!(tai_minus_utc(at(1999, 1, 1)), 32); // the 1999-01-01 step
    assert_eq!(tai_minus_utc(at(1998, 12, 31)), 31); // the day before it
    assert_eq!(tai_minus_utc(at(1970, 1, 1)), 10); // pre-1972 floor
                                                   // TAI is ahead of UTC by exactly the offset.
    assert_eq!(unix_to_tai_seconds(at(2017, 1, 1)), at(2017, 1, 1) + 37);
    assert_eq!(unix_to_tai_seconds(at(1972, 1, 1)), at(1972, 1, 1) + 10);
    // Round-trip away from a leap boundary.
    let t = at(2020, 6, 15) + 12_345;
    assert_eq!(tai_to_unix_seconds(unix_to_tai_seconds(t)), t);
    // TT = TAI + 32.184 s exactly.
    assert_eq!(TT_MINUS_TAI_NANOS, 32_184_000_000);
}

#[test]
fn tt_scale_is_exact() {
    let at_ns = |y, mo, d| days_from_civil(y, mo, d) * NANOS_PER_DAY;
    let u = at_ns(2017, 1, 1);
    // TAI = UTC + 37 s; TT = TAI + 32.184 s = UTC + 69.184 s.
    assert_eq!(tai_nanos_from_unix_nanos(u), u + 37 * NANOS_PER_SECOND);
    assert_eq!(
        tt_nanos_from_unix_nanos(u),
        u + 37 * NANOS_PER_SECOND + TT_MINUS_TAI_NANOS
    );
    // 1972: TAI−UTC = 10 → TT = UTC + 42.184 s.
    let u72 = at_ns(1972, 1, 1);
    assert_eq!(
        tt_nanos_from_unix_nanos(u72),
        u72 + 10 * NANOS_PER_SECOND + TT_MINUS_TAI_NANOS
    );
    // Sub-second precision survives, round-trip.
    let t = at_ns(2020, 3, 15) + 12_345_678_900;
    assert_eq!(unix_nanos_from_tt_nanos(tt_nanos_from_unix_nanos(t)), t);
}

#[test]
fn tt_round_trips_under_fuzz() {
    let mut rng = Rng(0x_77_C0DE_5CA1_9999);
    let base = days_from_civil(1972, 6, 1) * NANOS_PER_DAY;
    for _ in 0..20_000 {
        let ns = base + (rng.next() % 1_600_000_000_000_000_000) as i64;
        assert_eq!(
            unix_nanos_from_tt_nanos(tt_nanos_from_unix_nanos(ns)),
            ns,
            "TT round-trip at {ns}"
        );
    }
}

#[test]
fn tai_round_trips_under_fuzz() {
    // Property: tai_to_unix_seconds ∘ unix_to_tai_seconds = identity (instants are not on a
    // leap-second boundary at whole-second granularity away from midnight steps).
    let mut rng = Rng(0x_7A1_C0DE_5EC0_4242);
    let base = days_from_civil(1972, 6, 1) * SECONDS_PER_DAY;
    for _ in 0..20_000 {
        let secs = base + (rng.next() % 1_700_000_000) as i64; // ~1972..2025
        assert_eq!(
            tai_to_unix_seconds(unix_to_tai_seconds(secs)),
            secs,
            "TAI round-trip at {secs}"
        );
    }
}

#[test]
fn smooth_utc_round_trips_over_a_wide_range_under_fuzz() {
    // Property: civil_from_unix_nanos ∘ unix_nanos_from_civil = identity, and the decomposed
    // fields are always in civil range. SmoothUTC = no leap seconds, so seconds never hit 60.
    let mut rng = Rng(0x_5007_8807_C0DE_1357);
    for _ in 0..50_000 {
        // Range: ~±200 years of nanoseconds around the epoch (within i64).
        let ns = (rng.next() as i64).wrapping_rem(6_000_000_000_000_000_000);
        let c = civil_from_unix_nanos(ns);
        assert!((1..=12).contains(&c.month));
        assert!((1..=last_day_of_month(c.year, c.month)).contains(&c.day));
        assert!(c.hour < 24 && c.minute < 60 && c.second < 60 && c.nanosecond < 1_000_000_000);
        assert_eq!(
            unix_nanos_from_civil(c),
            ns,
            "instant round-trip at ns={ns}"
        );
    }
}

#[test]
fn iso_week_round_trips_over_a_wide_range_under_fuzz() {
    // Property: days_from_iso_week ∘ iso_week_from_days = identity, the week is always 1..=53,
    // the ISO weekday always 1..=7, and the ISO weekday agrees with the plain weekday.
    let mut rng = Rng(0x_C0DE_FACE_8601_9999);
    for _ in 0..50_000 {
        let z = (rng.next() % 800_001) as i64 - 400_000;
        let (iy, w, dow) = iso_week_from_days(z);
        assert!((1..=53).contains(&w), "ISO week in range at z={z}: {w}");
        assert!(
            (1..=7).contains(&dow),
            "ISO weekday in range at z={z}: {dow}"
        );
        // ISO weekday (1=Mon..7=Sun) lines up with the plain weekday (0=Sun..6=Sat).
        let plain = weekday_from_days(z);
        assert_eq!(dow % 7, plain, "ISO/plain weekday agree at z={z}");
        // Reconstruct the exact day from its ISO week date.
        assert_eq!(days_from_iso_week(iy, w, dow), z, "ISO round-trip at z={z}");
    }
}

#[test]
fn julian_round_trips_over_a_wide_range_under_fuzz() {
    // Property: julian_from_days ∘ days_from_julian = identity, and the proleptic Julian
    // calendar is internally consistent (every produced (y,m,d) maps back to the day number).
    let mut rng = Rng(0x_1234_ABCD_5678_EF90);
    for _ in 0..50_000 {
        let z = (rng.next() % 800_001) as i64 - 400_000;
        let (y, m, d) = julian_from_days(z);
        assert!((1..=12).contains(&m), "julian month in range at z={z}");
        assert!((1..=31).contains(&d), "julian day in range at z={z}");
        assert_eq!(days_from_julian(y, m, d), z, "julian round-trip at z={z}");
    }
}
