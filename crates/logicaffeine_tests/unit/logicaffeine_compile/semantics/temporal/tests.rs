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
