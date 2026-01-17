use crate::progress::SrsData;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseQuality {
    Blackout = 0,
    Incorrect = 1,
    IncorrectEasy = 2,
    CorrectDifficult = 3,
    CorrectHesitation = 4,
    Perfect = 5,
}

impl ResponseQuality {
    pub fn from_score(score: u8) -> Self {
        match score {
            0 => Self::Blackout,
            1 => Self::Incorrect,
            2 => Self::IncorrectEasy,
            3 => Self::CorrectDifficult,
            4 => Self::CorrectHesitation,
            _ => Self::Perfect,
        }
    }

    pub fn is_correct(self) -> bool {
        (self as u8) >= 3
    }
}

pub fn sm2_update(srs: &mut SrsData, quality: ResponseQuality) {
    let q = quality as u8 as f64;

    if quality.is_correct() {
        srs.repetitions += 1;
        srs.interval = match srs.repetitions {
            1 => 1,
            2 => 6,
            _ => (srs.interval as f64 * srs.ease_factor).round() as u32,
        };

        srs.ease_factor += 0.1 - (5.0 - q) * (0.08 + (5.0 - q) * 0.02);
        if srs.ease_factor < 1.3 {
            srs.ease_factor = 1.3;
        }
    } else {
        srs.repetitions = 0;
        srs.interval = 1;
    }
}

pub fn calculate_next_review(current_date: &str, interval_days: u32) -> String {
    if let Ok(date) = parse_date(current_date) {
        let next = date + interval_days as i64;
        format_date(next)
    } else {
        current_date.to_string()
    }
}

pub fn is_due(next_review: Option<&str>, today: &str) -> bool {
    match next_review {
        None => true,
        Some(review_date) => {
            if let (Ok(review), Ok(now)) = (parse_date(review_date), parse_date(today)) {
                review <= now
            } else {
                true
            }
        }
    }
}

fn parse_date(date_str: &str) -> Result<i64, ()> {
    let parts: Vec<&str> = date_str.split('-').collect();
    if parts.len() != 3 {
        return Err(());
    }

    let year: i64 = parts[0].parse().map_err(|_| ())?;
    let month: i64 = parts[1].parse().map_err(|_| ())?;
    let day: i64 = parts[2].parse().map_err(|_| ())?;

    Ok(year * 10000 + month * 100 + day)
}

fn format_date(date_num: i64) -> String {
    let year = date_num / 10000;
    let month = (date_num % 10000) / 100;
    let day = date_num % 100;

    let (year, month, day) = normalize_date(year as i32, month as i32, day as i32);
    format!("{:04}-{:02}-{:02}", year, month, day)
}

fn normalize_date(year: i32, month: i32, day: i32) -> (i32, i32, i32) {
    let days_in_month = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

    let mut y = year;
    let mut m = month;
    let mut d = day;

    while d > days_in_month[(m - 1) as usize] {
        d -= days_in_month[(m - 1) as usize];
        m += 1;
        if m > 12 {
            m = 1;
            y += 1;
        }
    }

    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sm2_first_correct() {
        let mut srs = SrsData::default();
        sm2_update(&mut srs, ResponseQuality::Perfect);

        assert_eq!(srs.repetitions, 1);
        assert_eq!(srs.interval, 1);
        assert!(srs.ease_factor > 2.5);
    }

    #[test]
    fn test_sm2_second_correct() {
        let mut srs = SrsData::default();
        sm2_update(&mut srs, ResponseQuality::Perfect);
        sm2_update(&mut srs, ResponseQuality::Perfect);

        assert_eq!(srs.repetitions, 2);
        assert_eq!(srs.interval, 6);
    }

    #[test]
    fn test_sm2_incorrect_resets() {
        let mut srs = SrsData::default();
        sm2_update(&mut srs, ResponseQuality::Perfect);
        sm2_update(&mut srs, ResponseQuality::Perfect);
        sm2_update(&mut srs, ResponseQuality::Incorrect);

        assert_eq!(srs.repetitions, 0);
        assert_eq!(srs.interval, 1);
    }

    #[test]
    fn test_sm2_ease_factor_minimum() {
        let mut srs = SrsData::default();
        srs.ease_factor = 1.3;
        sm2_update(&mut srs, ResponseQuality::CorrectDifficult);

        assert!(srs.ease_factor >= 1.3);
    }

    #[test]
    fn test_is_due_none() {
        assert!(is_due(None, "2025-01-01"));
    }

    #[test]
    fn test_is_due_past() {
        assert!(is_due(Some("2025-01-01"), "2025-01-02"));
    }

    #[test]
    fn test_is_due_future() {
        assert!(!is_due(Some("2025-01-05"), "2025-01-02"));
    }

    #[test]
    fn test_is_due_today() {
        assert!(is_due(Some("2025-01-01"), "2025-01-01"));
    }

    #[test]
    fn test_calculate_next_review() {
        let next = calculate_next_review("2025-01-15", 6);
        assert_eq!(next, "2025-01-21");
    }

    #[test]
    fn test_calculate_next_review_month_overflow() {
        let next = calculate_next_review("2025-01-28", 6);
        assert_eq!(next, "2025-02-03");
    }

    #[test]
    fn test_response_quality_is_correct() {
        assert!(!ResponseQuality::Blackout.is_correct());
        assert!(!ResponseQuality::Incorrect.is_correct());
        assert!(!ResponseQuality::IncorrectEasy.is_correct());
        assert!(ResponseQuality::CorrectDifficult.is_correct());
        assert!(ResponseQuality::CorrectHesitation.is_correct());
        assert!(ResponseQuality::Perfect.is_correct());
    }
}
