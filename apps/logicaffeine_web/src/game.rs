use crate::progress::UserProgress;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XpReward {
    pub base: u64,
    pub combo_bonus: u64,
    pub streak_bonus: u64,
    pub critical_bonus: u64,
    pub first_try_bonus: u64,
    pub total: u64,
    pub is_critical: bool,
}

pub fn calculate_xp_reward(
    difficulty: u32,
    combo: u32,
    streak_days: u32,
    first_try: bool,
    rng_seed: u64,
) -> XpReward {
    let base = 10 + (difficulty.saturating_sub(1) * 5) as u64;

    let combo_mult = 1.0 + (combo.min(10) as f64 * 0.1);
    let combo_bonus = ((base as f64) * (combo_mult - 1.0)) as u64;

    let streak_bonus = (streak_days.min(7) * 2) as u64;

    let first_try_bonus = if first_try { 5 } else { 0 };

    let is_critical = (rng_seed % 10) == 0;
    let critical_bonus = if is_critical { base } else { 0 };

    let total = base + combo_bonus + streak_bonus + first_try_bonus + critical_bonus;

    XpReward {
        base,
        combo_bonus,
        streak_bonus,
        critical_bonus,
        first_try_bonus,
        total,
        is_critical,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreakStatus {
    Active { days: u32 },
    AtRisk,
    Frozen,
    Lost { was: u32 },
}

pub fn update_streak(progress: &mut UserProgress, today: &str) -> StreakStatus {
    match &progress.last_streak_date {
        None => {
            progress.streak_days = 1;
            progress.last_streak_date = Some(today.to_string());
            StreakStatus::Active { days: 1 }
        }
        Some(last) if last == today => {
            StreakStatus::Active { days: progress.streak_days }
        }
        Some(last) if is_yesterday(last, today) => {
            progress.streak_days += 1;
            progress.last_streak_date = Some(today.to_string());
            StreakStatus::Active { days: progress.streak_days }
        }
        Some(last) if is_two_days_ago(last, today) && progress.streak_freezes > 0 => {
            progress.streak_freezes -= 1;
            progress.last_streak_date = Some(today.to_string());
            StreakStatus::Frozen
        }
        Some(_) => {
            let was = progress.streak_days;
            progress.streak_days = 1;
            progress.last_streak_date = Some(today.to_string());
            StreakStatus::Lost { was }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComboResult {
    pub new_combo: u32,
    pub is_new_record: bool,
    pub multiplier: f64,
}

pub fn update_combo(progress: &mut UserProgress, correct: bool) -> ComboResult {
    if correct {
        progress.combo += 1;
        let is_new_record = progress.combo > progress.best_combo;
        if is_new_record {
            progress.best_combo = progress.combo;
        }
        let multiplier = 1.0 + (progress.combo.min(10) as f64 * 0.1);
        ComboResult { new_combo: progress.combo, is_new_record, multiplier }
    } else {
        progress.combo = 0;
        ComboResult { new_combo: 0, is_new_record: false, multiplier: 1.0 }
    }
}

pub fn level_title(level: u32) -> &'static str {
    match level {
        1 => "Novice",
        2..=4 => "Apprentice",
        5..=9 => "Student",
        10..=14 => "Scholar",
        15..=19 => "Adept",
        20..=29 => "Expert",
        30..=39 => "Master",
        40..=49 => "Sage",
        _ => "Grandmaster",
    }
}

pub fn xp_progress_to_next_level(xp: u64, level: u32) -> (u64, u64, f64) {
    let current_threshold = crate::progress::xp_for_level(level);
    let next_threshold = crate::progress::xp_for_level(level + 1);
    let progress = xp.saturating_sub(current_threshold);
    let needed = next_threshold - current_threshold;
    let percentage = if needed > 0 {
        (progress as f64) / (needed as f64)
    } else {
        0.0
    };
    (progress, needed, percentage)
}

pub struct FreezeGrant {
    pub freezes: u8,
    pub reason: &'static str,
}

pub fn check_level_up_freeze_grants(old_level: u32, new_level: u32) -> Option<FreezeGrant> {
    let freeze_count = (old_level + 1..=new_level)
        .filter(|l| l % 5 == 0)
        .count() as u8;

    if freeze_count > 0 {
        Some(FreezeGrant {
            freezes: freeze_count,
            reason: "Level milestone reward",
        })
    } else {
        None
    }
}

pub fn is_sunday(date: &str) -> bool {
    if let Ok(num) = parse_date_to_days(date) {
        (num + 4) % 7 == 0
    } else {
        false
    }
}

fn is_yesterday(last: &str, today: &str) -> bool {
    if let (Ok(l), Ok(t)) = (parse_date_to_days(last), parse_date_to_days(today)) {
        t - l == 1
    } else {
        false
    }
}

fn is_two_days_ago(last: &str, today: &str) -> bool {
    if let (Ok(l), Ok(t)) = (parse_date_to_days(last), parse_date_to_days(today)) {
        t - l == 2
    } else {
        false
    }
}

fn parse_date_to_days(date: &str) -> Result<i64, ()> {
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3 {
        return Err(());
    }

    let year: i64 = parts[0].parse().map_err(|_| ())?;
    let month: i64 = parts[1].parse().map_err(|_| ())?;
    let day: i64 = parts[2].parse().map_err(|_| ())?;

    let days = year * 365 + (year / 4) - (year / 100) + (year / 400)
        + (month * 30) + day;
    Ok(days)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xp_reward_base() {
        let reward = calculate_xp_reward(1, 0, 0, false, 1);
        assert_eq!(reward.base, 10);
        assert_eq!(reward.combo_bonus, 0);
        assert_eq!(reward.total, 10);
        assert!(!reward.is_critical);
    }

    #[test]
    fn test_xp_reward_with_combo() {
        let reward = calculate_xp_reward(1, 5, 0, false, 1);
        assert_eq!(reward.base, 10);
        assert_eq!(reward.combo_bonus, 5); // 10 * 0.5 = 5
        assert_eq!(reward.total, 15);
    }

    #[test]
    fn test_xp_reward_critical() {
        let reward = calculate_xp_reward(1, 0, 0, false, 10); // seed % 10 == 0
        assert!(reward.is_critical);
        assert_eq!(reward.critical_bonus, 10);
        assert_eq!(reward.total, 20);
    }

    #[test]
    fn test_xp_reward_full() {
        // difficulty 3, combo 10, streak 7, first try, non-crit
        let reward = calculate_xp_reward(3, 10, 7, true, 1);
        // base = 10 + (2 * 5) = 20
        // combo = 20 * 1.0 = 20
        // streak = 14
        // first_try = 5
        // total = 20 + 20 + 14 + 5 = 59
        assert_eq!(reward.base, 20);
        assert_eq!(reward.combo_bonus, 20);
        assert_eq!(reward.streak_bonus, 14);
        assert_eq!(reward.first_try_bonus, 5);
        assert_eq!(reward.total, 59);
    }

    #[test]
    fn test_combo_increment() {
        let mut progress = UserProgress::new();

        let result = update_combo(&mut progress, true);
        assert_eq!(result.new_combo, 1);
        assert!(result.is_new_record);

        let result = update_combo(&mut progress, true);
        assert_eq!(result.new_combo, 2);
        assert!(result.is_new_record);

        let result = update_combo(&mut progress, false);
        assert_eq!(result.new_combo, 0);
        assert!(!result.is_new_record);
    }

    #[test]
    fn test_combo_multiplier() {
        let mut progress = UserProgress::new();

        for _ in 0..10 {
            update_combo(&mut progress, true);
        }

        let result = update_combo(&mut progress, true);
        assert!((result.multiplier - 2.0).abs() < 0.01);
    }

    #[test]
    fn test_level_titles() {
        assert_eq!(level_title(1), "Novice");
        assert_eq!(level_title(5), "Student");
        assert_eq!(level_title(10), "Scholar");
        assert_eq!(level_title(50), "Grandmaster");
    }

    #[test]
    fn test_level_up_freeze_grants() {
        assert!(check_level_up_freeze_grants(1, 4).is_none());

        let grant = check_level_up_freeze_grants(4, 5).unwrap();
        assert_eq!(grant.freezes, 1);

        let grant = check_level_up_freeze_grants(1, 10).unwrap();
        assert_eq!(grant.freezes, 2); // levels 5 and 10
    }

    #[test]
    fn test_is_yesterday() {
        assert!(is_yesterday("2025-01-01", "2025-01-02"));
        assert!(!is_yesterday("2025-01-01", "2025-01-03"));
    }
}
