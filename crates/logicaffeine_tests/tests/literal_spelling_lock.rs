//! The lexer's spelling of each literal kind (`lexer::char_literal`, `date_literal`, `time_literal`,
//! `moment_literal`, `duration_literal`, `span_literal`) reads back through the whole front end as exactly the
//! literal it spells, standing alone and as a parenthesized operand: the decompilers print a residual's literals
//! with them, and the Core IR holds a literal the self-interpreter carries no value of as that spelling.

use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_language::ast::stmt::{Expr, Literal, Stmt};
use logicaffeine_language::lexer::{
    char_literal, date_literal, duration_literal, moment_literal, span_literal, time_literal,
};

const MINUTE: i64 = 60_000_000_000;
const DAY: i64 = 24 * 60 * MINUTE;

/// The literal `Let v be <spelled>.` binds, read through the front end every engine shares.
fn bound_literal(spelled: &str) -> Result<Literal, String> {
    with_parsed_program(&format!("## Main\nLet v be {spelled}.\nShow v."), |parsed, _| {
        let (stmts, _, _) = parsed?;
        stmts
            .iter()
            .find_map(|s| match s {
                Stmt::Let { value: Expr::Literal(lit), .. } => Some(lit.clone()),
                _ => None,
            })
            .ok_or_else(|| format!("`{spelled}` binds no literal"))
    })
}

/// Every spelling in `cases` that does not read back, alone and parenthesized, as the literal it spells.
fn misread(cases: &[(Literal, String)]) -> Vec<String> {
    let mut failures = Vec::new();
    for (want, spelled) in cases {
        for form in [spelled.clone(), format!("({spelled})")] {
            match bound_literal(&form) {
                Ok(got) if got == *want => {}
                other => failures.push(format!("{want:?} as `{form}` reads back as {other:?}")),
            }
        }
    }
    failures
}

fn assert_read_back(kind: &str, cases: &[(Literal, String)]) {
    let failures = misread(cases);
    assert!(
        failures.is_empty(),
        "{} of {} {kind} spellings do not read back as their literal:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

/// The day of `year-month-day`, as the front end reads its date literal.
fn day_of(year: i32, month: u32, day: u32) -> i32 {
    match bound_literal(&format!("{year:04}-{month:02}-{day:02}")) {
        Ok(Literal::Date(days)) => days,
        other => panic!("{year:04}-{month:02}-{day:02} reads as {other:?}"),
    }
}

#[test]
fn a_char_literal_reads_back_as_its_char() {
    let cases: Vec<(Literal, String)> = ['a', 'Z', '0', ' ', '`', '\\', '\'', '"', '{', '}', '.', '\n', '\t', '\r', '\0', '\u{e9}', '\u{2713}', '\u{1f600}']
        .into_iter()
        .map(|c| (Literal::Char(c), char_literal(c)))
        .collect();
    assert_read_back("char", &cases);
}

#[test]
fn a_date_literal_reads_back_as_its_day() {
    let days = [day_of(1970, 1, 1), day_of(2026, 5, 20), day_of(2000, 2, 29), day_of(1900, 3, 1), day_of(0, 1, 1), day_of(9999, 12, 31)];
    let cases: Vec<(Literal, String)> =
        days.into_iter().map(|d| (Literal::Date(d), date_literal(d).expect("a four-digit year spells"))).collect();
    assert_read_back("date", &cases);
}

#[test]
fn a_time_literal_reads_back_as_its_minute() {
    let cases: Vec<(Literal, String)> = (0..24 * 60)
        .step_by(37)
        .chain([0, 1, 59, 60, 719, 720, 721, 1439])
        .map(|m| (Literal::Time(m * MINUTE), time_literal(m * MINUTE).expect("a minute of the day spells")))
        .collect();
    assert_read_back("time", &cases);
}

/// A moment counts nanoseconds from 1970 in an `i64`, so its days run from 1677-09-22 to 2262-04-11.
#[test]
fn a_moment_literal_reads_back_as_its_moment() {
    let days = [day_of(1970, 1, 1), day_of(2026, 5, 20), day_of(1969, 12, 31), day_of(1678, 1, 1), day_of(2262, 1, 1)];
    let minutes = [0, 1, 570, 720, 1439];
    let mut cases = Vec::new();
    for d in days {
        for m in minutes {
            let nanos = d as i64 * DAY + m * MINUTE;
            cases.push((Literal::Moment(nanos), moment_literal(nanos).expect("a spelled day at a spelled minute spells")));
        }
    }
    assert_read_back("moment", &cases);
    assert_eq!(moment_literal(day_of(2026, 5, 20) as i64 * DAY + 1), None, "a moment between minutes has no literal");
}

#[test]
fn a_duration_literal_reads_back_as_its_nanoseconds() {
    let cases: Vec<(Literal, String)> = [0, 1, 999, 1_000, 1_500_000, 60_000_000_000, 3_600_000_000_000, i64::MAX]
        .into_iter()
        .map(|n| (Literal::Duration(n), duration_literal(n).expect("a duration that is not negative spells")))
        .collect();
    assert_read_back("duration", &cases);
}

#[test]
fn a_span_literal_reads_back_as_its_months_and_days() {
    let mut cases = Vec::new();
    for months in (0..=30).chain([120, 1200]) {
        for days in (0..=40).step_by(3).chain([1, 365, 1000]) {
            let spelled = span_literal(months, days).expect("a span that is not negative spells");
            cases.push((Literal::Span { months, days }, spelled));
        }
    }
    assert_read_back("span", &cases);
    for (months, days) in [(-1, 0), (0, -1), (-3, -4)] {
        assert_eq!(span_literal(months, days), None, "a negative span part has no literal");
    }
}
