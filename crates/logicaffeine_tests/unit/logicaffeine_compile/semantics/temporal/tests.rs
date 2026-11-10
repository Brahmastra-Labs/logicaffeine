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

#[test]
fn every_clock_read_is_named_by_its_name_and_reads_its_type() {
    for read in ClockRead::ALL {
        assert_eq!(ClockRead::named(read.name()), Some(read));
    }
    assert_eq!(ClockRead::named("today").map(ClockRead::logos_type), Some(crate::analysis::types::LogosType::Date));
    assert_eq!(ClockRead::named("now").map(ClockRead::logos_type), Some(crate::analysis::types::LogosType::Moment));
    for other in ["Now", "Today", "today ", "tomorrow", "now()", "yesterday", ""] {
        assert_eq!(ClockRead::named(other), None, "`{other}` is no clock read");
    }
}

#[test]
fn the_clock_reads_are_the_names_the_tree_walker_reads_the_clock_for() {
    set_fixed_clock(20000, (20000 * 86_400 + 45_296) * 1_000_000_000);
    let run = crate::compile::tw_outcome("## Main\nShow today.\nShow now.\n");
    clear_fixed_clock();
    assert_eq!(run.error, None);
    assert_eq!(run.output.trim(), "2024-10-04\n2024-10-04 12:34");
    assert_eq!(ClockRead::ALL.map(ClockRead::name), ["today", "now"]);
}
