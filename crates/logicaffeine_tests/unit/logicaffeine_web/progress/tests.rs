use super::*;

/// What the scored Learn page saved under `logos_user_progress` before the roadmap: XP, a level,
/// streaks, combos, achievements, module stars. A returning visitor's browser still holds it.
const SCORED_BLOB: &str = r#"{
  "xp": 340,
  "level": 2,
  "streak_days": 4,
  "last_session": "2026-09-30",
  "exercises": {
    "A_1.1": {
      "exercise_id": "A_1.1",
      "attempts": 3,
      "correct_count": 2,
      "last_attempt": null,
      "srs": { "ease_factor": 2.6, "interval": 6, "repetitions": 2, "next_review": "2026-10-06" }
    }
  },
  "modules": {
    "syllogistic": { "module_id": "syllogistic", "unlocked": true, "completed": false, "stars": 1, "best_score": 70, "attempts": 2 }
  },
  "combo": 3,
  "best_combo": 7,
  "streak_freezes": 1,
  "last_streak_date": "2026-09-30",
  "achievements": ["first_blood", "combo_5"],
  "title": "Logic Learner",
  "last_weekly_freeze_date": "2026-09-28"
}"#;

#[test]
fn test_user_progress_record() {
    let mut progress = UserProgress::new();
    progress.record_attempt("test_q1", true);
    progress.record_attempt("test_q1", false);

    let ex = progress.get_exercise_progress("test_q1").unwrap();
    assert_eq!(ex.attempts, 2);
    assert_eq!(ex.correct_count, 1);
}

#[test]
fn a_blob_from_the_scored_page_still_loads_with_its_exercises() {
    let progress: UserProgress = serde_json::from_str(SCORED_BLOB).expect("an old blob must load");
    let ex = progress.get_exercise_progress("A_1.1").expect("its exercise history is kept");
    assert_eq!((ex.attempts, ex.correct_count, ex.srs.interval), (3, 2, 6));
    assert!(progress.lessons.is_empty());
    assert_eq!(progress.open_lesson, None);
}

#[test]
fn saving_an_old_blob_again_drops_its_score() {
    let progress: UserProgress = serde_json::from_str(SCORED_BLOB).unwrap();
    let saved: serde_json::Value = serde_json::to_value(&progress).unwrap();
    let mut keys: Vec<&str> = saved.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["exercises", "lessons", "open_lesson"], "nothing but the exercises and the roadmap is kept");
}

#[test]
fn a_blob_with_only_exercises_loads() {
    let progress: UserProgress = serde_json::from_str(r#"{"exercises":{}}"#).expect("loads");
    assert!(progress.exercises.is_empty());
}

#[test]
fn an_empty_blob_loads() {
    let progress: UserProgress = serde_json::from_str("{}").expect("loads");
    assert!(progress.exercises.is_empty() && progress.lessons.is_empty());
}

#[test]
fn a_finished_lesson_is_remembered_through_a_save() {
    let mut progress = UserProgress::new();
    progress.finish_lesson("1.1");
    progress.open_lesson = Some("1.2".to_string());
    let back: UserProgress = serde_json::from_str(&serde_json::to_string(&progress).unwrap()).unwrap();
    assert!(back.is_lesson_done("1.1"));
    assert!(!back.is_lesson_done("1.2"));
    assert_eq!(back.open_lesson.as_deref(), Some("1.2"));
}

#[test]
fn finishing_a_lesson_twice_records_it_once() {
    let mut progress = UserProgress::new();
    progress.finish_lesson("8.2");
    progress.finish_lesson("8.2");
    assert_eq!(progress.lessons.len(), 1);
}
