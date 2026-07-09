//! Regression pin for Bug Report #1, BUG-032.
//!
//! Moment/Time comparison and Moment display must use Euclidean (floored)
//! time-of-day arithmetic, so a pre-epoch (negative) Moment has a correct
//! 0..86399 time-of-day rather than a negative one from truncating `%`.

use logicaffeine_compile::ast::stmt::BinaryOpKind;
use logicaffeine_compile::interpreter::RuntimeValue;
use logicaffeine_compile::semantics::compare::compare;

#[test]
fn moment_before_epoch_compares_by_true_time_of_day() {
    let hour = 3_600_000_000_000i64;

    // `1969-12-31 at 11pm`: one hour before the Unix epoch. True wall-clock
    // time-of-day = 23:00, so it is AFTER 22:00.
    let m = RuntimeValue::Moment(-hour);
    let t_2200 = RuntimeValue::Time(22 * hour);

    // 23:00 < 22:00 is FALSE.
    let r = compare(BinaryOpKind::Lt, &m, &t_2200).unwrap();
    assert!(
        matches!(r, RuntimeValue::Bool(false)),
        "pre-epoch Moment time-of-day must be 23:00 (after 22:00), not a negative time-of-day"
    );

    // Symmetric arm: 23:00 > 22:00 is TRUE.
    let r2 = compare(BinaryOpKind::Gt, &m, &t_2200).unwrap();
    assert!(
        matches!(r2, RuntimeValue::Bool(true)),
        "Time vs pre-epoch Moment must also use Euclidean time-of-day"
    );
}

#[test]
fn moment_before_epoch_displays_correct_date_and_hour() {
    let hour = 3_600_000_000_000i64;
    // 1969-12-31 at 23:00.
    let m = RuntimeValue::Moment(-hour);
    assert_eq!(
        m.to_display_string(),
        "1969-12-31 23:00",
        "pre-epoch Moment must floor the date and use Euclidean time-of-day, not print 1970-01-01 -1:00"
    );
}
