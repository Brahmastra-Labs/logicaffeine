use crate::progress::UserProgress;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Achievement {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub xp_reward: u64,
    pub unlocks_title: Option<&'static str>,
    pub grants_freeze: bool,
}

pub const ACHIEVEMENTS: &[Achievement] = &[
    Achievement {
        id: "first_blood",
        title: "First Blood",
        description: "Answer your first question correctly",
        xp_reward: 50,
        unlocks_title: None,
        grants_freeze: false,
    },
    Achievement {
        id: "combo_5",
        title: "On Fire",
        description: "Get a 5-answer combo",
        xp_reward: 100,
        unlocks_title: None,
        grants_freeze: false,
    },
    Achievement {
        id: "combo_10",
        title: "Unstoppable",
        description: "Get a 10-answer combo",
        xp_reward: 250,
        unlocks_title: Some("Logic Machine"),
        grants_freeze: false,
    },
    Achievement {
        id: "combo_25",
        title: "Terminator",
        description: "Get a 25-answer combo",
        xp_reward: 500,
        unlocks_title: Some("Automaton"),
        grants_freeze: false,
    },
    Achievement {
        id: "streak_3",
        title: "Getting Started",
        description: "Maintain a 3-day streak",
        xp_reward: 75,
        unlocks_title: None,
        grants_freeze: false,
    },
    Achievement {
        id: "streak_7",
        title: "Week Warrior",
        description: "Maintain a 7-day streak",
        xp_reward: 200,
        unlocks_title: Some("Dedicated"),
        grants_freeze: true,
    },
    Achievement {
        id: "streak_14",
        title: "Fortnight Fighter",
        description: "Maintain a 14-day streak",
        xp_reward: 400,
        unlocks_title: None,
        grants_freeze: true,
    },
    Achievement {
        id: "streak_30",
        title: "Monthly Master",
        description: "Maintain a 30-day streak",
        xp_reward: 1000,
        unlocks_title: Some("Logician"),
        grants_freeze: true,
    },
    Achievement {
        id: "perfect_module",
        title: "Flawless",
        description: "Complete a module with no mistakes",
        xp_reward: 300,
        unlocks_title: None,
        grants_freeze: false,
    },
    Achievement {
        id: "century",
        title: "Century",
        description: "Answer 100 questions correctly",
        xp_reward: 500,
        unlocks_title: Some("Scholar"),
        grants_freeze: false,
    },
    Achievement {
        id: "millennium",
        title: "Millennium",
        description: "Answer 1000 questions correctly",
        xp_reward: 2000,
        unlocks_title: Some("Sage"),
        grants_freeze: false,
    },
    Achievement {
        id: "level_10",
        title: "Double Digits",
        description: "Reach level 10",
        xp_reward: 300,
        unlocks_title: None,
        grants_freeze: false,
    },
    Achievement {
        id: "level_25",
        title: "Quarter Century",
        description: "Reach level 25",
        xp_reward: 750,
        unlocks_title: Some("Adept"),
        grants_freeze: false,
    },
    Achievement {
        id: "level_50",
        title: "Half Century",
        description: "Reach level 50",
        xp_reward: 1500,
        unlocks_title: Some("Grandmaster"),
        grants_freeze: false,
    },
];

pub fn get_achievement(id: &str) -> Option<&'static Achievement> {
    ACHIEVEMENTS.iter().find(|a| a.id == id)
}

pub fn check_achievements(progress: &UserProgress) -> Vec<&'static Achievement> {
    let mut newly_unlocked = Vec::new();

    for achievement in ACHIEVEMENTS {
        if progress.achievements.contains(achievement.id) {
            continue;
        }

        let earned = match achievement.id {
            "first_blood" => total_correct(progress) >= 1,
            "combo_5" => progress.best_combo >= 5,
            "combo_10" => progress.best_combo >= 10,
            "combo_25" => progress.best_combo >= 25,
            "streak_3" => progress.streak_days >= 3,
            "streak_7" => progress.streak_days >= 7,
            "streak_14" => progress.streak_days >= 14,
            "streak_30" => progress.streak_days >= 30,
            "century" => total_correct(progress) >= 100,
            "millennium" => total_correct(progress) >= 1000,
            "level_10" => progress.level >= 10,
            "level_25" => progress.level >= 25,
            "level_50" => progress.level >= 50,
            "perfect_module" => false, // Checked separately in lesson completion
            _ => false,
        };

        if earned {
            newly_unlocked.push(achievement);
        }
    }

    newly_unlocked
}

fn total_correct(progress: &UserProgress) -> u32 {
    progress.exercises.values().map(|e| e.correct_count).sum()
}

pub fn unlock_achievement(progress: &mut UserProgress, achievement: &Achievement) {
    progress.achievements.insert(achievement.id.to_string());
    progress.xp += achievement.xp_reward;
    progress.level = crate::progress::calculate_level(progress.xp);

    if let Some(title) = achievement.unlocks_title {
        if progress.title.is_none() {
            progress.title = Some(title.to_string());
        }
    }

    if achievement.grants_freeze && progress.streak_freezes < 3 {
        progress.streak_freezes += 1;
    }

    progress.save();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_achievement() {
        let achievement = get_achievement("first_blood");
        assert!(achievement.is_some());
        assert_eq!(achievement.unwrap().title, "First Blood");
    }

    #[test]
    fn test_achievement_not_found() {
        let achievement = get_achievement("nonexistent");
        assert!(achievement.is_none());
    }

    #[test]
    fn test_check_achievements_first_blood() {
        let mut progress = UserProgress::new();
        progress.record_attempt("test", true);

        let newly_unlocked = check_achievements(&progress);
        assert!(newly_unlocked.iter().any(|a| a.id == "first_blood"));
    }

    #[test]
    fn test_check_achievements_combo() {
        let mut progress = UserProgress::new();
        progress.best_combo = 5;

        let newly_unlocked = check_achievements(&progress);
        assert!(newly_unlocked.iter().any(|a| a.id == "combo_5"));
        assert!(!newly_unlocked.iter().any(|a| a.id == "combo_10"));
    }

    #[test]
    fn test_streak_achievements_grant_freeze() {
        let streak_7 = get_achievement("streak_7").unwrap();
        assert!(streak_7.grants_freeze);

        let first_blood = get_achievement("first_blood").unwrap();
        assert!(!first_blood.grants_freeze);
    }
}
