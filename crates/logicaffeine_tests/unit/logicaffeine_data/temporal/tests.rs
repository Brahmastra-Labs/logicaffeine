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

#[test]
fn a_moment_and_a_time_of_day_order_by_the_moments_time_of_day_either_way_round() {
    let nine_thirty = LogosTime(9 * 3_600_000_000_000 + 30 * 60_000_000_000);
    let m = LogosMoment::from_ymd(2026, 11, 8, 9, 30, 0);
    let later = LogosMoment::from_ymd(2026, 11, 8, 9, 45, 0);
    let earlier_day_later_time = LogosMoment::from_ymd(1999, 1, 1, 23, 0, 0);
    assert!(!(m < nine_thirty) && !(m > nine_thirty) && m <= nine_thirty && m >= nine_thirty);
    assert!(!(nine_thirty < m) && !(nine_thirty > m) && nine_thirty <= m && nine_thirty >= m);
    assert!(later > nine_thirty && nine_thirty < later);
    assert!(earlier_day_later_time > nine_thirty, "the date takes no part: only the time of day orders");
    let before_epoch = LogosMoment::from_ymd(1969, 12, 31, 23, 0, 0);
    assert!(before_epoch > nine_thirty, "an instant before the epoch reads its time of day by Euclid, 23:00");
    assert!(m != nine_thirty && nine_thirty != m, "values of two kinds are never equal, whatever their order says");
}

#[test]
fn a_duration_or_a_span_plus_a_moment_is_the_moment_moved_either_way_round() {
    let m = LogosMoment::from_ymd(2024, 1, 31, 23, 59, 0);
    let d = LogosDuration(90_000_000_000);
    assert_eq!(d + m, m + d);
    assert_eq!(m + d - d, m);
    let month = LogosSpan::new(1, 0);
    assert_eq!(month + m, m + month);
    assert_eq!((month + m).to_string(), "2024-02-29 23:59", "a month from the 31st clamps to the month's last day");
}

#[test]
fn a_temporal_slot_starts_at_the_epoch_at_midnight_or_at_no_span() {
    assert_eq!(LogosDate::default().to_string(), "1970-01-01");
    assert_eq!(LogosMoment::default(), LogosMoment(0));
    assert_eq!(LogosTime::default().to_string(), "00:00:00");
    assert_eq!(LogosSpan::default(), LogosSpan::new(0, 0));
}

#[test]
fn the_date_of_a_date_is_the_date() {
    for days in [i32::MIN, -1, 0, 19753, 20000, i32::MAX] {
        assert_eq!(LogosDate(days).date(), LogosDate(days));
    }
}

#[test]
fn a_duration_is_signed_shows_as_the_language_shows_it_and_waits_none_below_zero() {
    let five = LogosDuration(5);
    let none = LogosDuration::default();
    assert_eq!((none - five).to_string(), "-5ns");
    assert_eq!((LogosDuration(250_000_000) - LogosDuration(1_000_000_000)).to_string(), "-750ms");
    assert_eq!((LogosDuration(45_000_000_000) + LogosDuration(45_000_000_000) + LogosDuration(45_000_000_000)).to_string(), "2min");
    assert!(none - five < none && none < five);
    assert_eq!((none - five).to_std(), std::time::Duration::ZERO, "a wait at or below zero waits not at all");
    assert_eq!(LogosDuration(1_500_000).to_std(), std::time::Duration::from_micros(1_500));
    assert_eq!(LogosDuration(i64::MIN) - LogosDuration(1), LogosDuration(i64::MAX), "durations wrap as the tree-walker's do");
}

#[test]
fn a_moment_moves_by_a_signed_duration_either_way() {
    let m = LogosMoment::from_ymd(2026, 11, 8, 9, 30, 0);
    let back = LogosDuration(0) - LogosDuration(45 * 60 * 1_000_000_000);
    assert_eq!((m + back).to_string(), "2026-11-08 08:45");
    assert_eq!(m - back, m + LogosDuration(45 * 60 * 1_000_000_000));
    assert_eq!(back + m, m + back);
}
