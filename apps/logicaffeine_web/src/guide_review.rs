//! The Syntax Guide's SM-2 deck: recap self-marks and graded exercise results
//! feed the same spaced-repetition engine as Learn, under a `guide/` key
//! namespace so the two decks coexist in one `UserProgress` without
//! collision. Due guide cards surface on the Review page beside Learn cards.

use crate::progress::{ExerciseProgress, SrsData, UserProgress};
use crate::srs::{calculate_next_review, is_due, sm2_update, ResponseQuality};
use crate::ui::pages::guide::content::SECTIONS;

pub const GUIDE_RECALL_PREFIX: &str = "guide/recall/";
pub const GUIDE_EX_PREFIX: &str = "guide/ex/";

/// The progress-map key for a recap question's SRS entry.
pub fn recall_key(recall_id: &str) -> String {
    format!("{GUIDE_RECALL_PREFIX}{recall_id}")
}

/// The progress-map key for a guide exercise's SRS entry.
pub fn exercise_key(exercise_id: &str) -> String {
    format!("{GUIDE_EX_PREFIX}{exercise_id}")
}

/// A due card in the guide deck. Content is looked up from `SECTIONS` at
/// render time; the card carries the ids and its home section for the
/// deep-link back into the guide.
#[derive(Debug, Clone, PartialEq)]
pub enum GuideCard {
    Recall {
        key: String,
        section_id: String,
        recall_id: String,
        question: String,
        answer: String,
    },
    Exercise {
        key: String,
        section_id: String,
        exercise_id: String,
    },
}

fn record(progress: &mut UserProgress, key: String, quality: ResponseQuality, today: &str) {
    let entry = progress
        .exercises
        .entry(key.clone())
        .or_insert_with(|| ExerciseProgress {
            exercise_id: key,
            attempts: 0,
            correct_count: 0,
            last_attempt: None,
            srs: SrsData::default(),
        });
    entry.attempts += 1;
    if quality.is_correct() {
        entry.correct_count += 1;
    }
    entry.last_attempt = Some(today.to_string());
    sm2_update(&mut entry.srs, quality);
    entry.srs.next_review = Some(calculate_next_review(today, entry.srs.interval));
    progress.save();
}

/// A recap reveal + self-mark: quality is the reader's own 0–5 judgment
/// (the reveal keeps it honest — control of error, not surveillance).
pub fn record_recall_mark(progress: &mut UserProgress, recall_id: &str, quality: u8, today: &str) {
    record(
        progress,
        recall_key(recall_id),
        ResponseQuality::from_score(quality),
        today,
    );
}

/// A graded exercise result: quality comes from the grader's verdict
/// (fail = 1, passed with hints ≈ 3–4, clean pass = 5).
pub fn record_exercise_result(
    progress: &mut UserProgress,
    exercise_id: &str,
    quality: u8,
    today: &str,
) {
    record(
        progress,
        exercise_key(exercise_id),
        ResponseQuality::from_score(quality),
        today,
    );
}

/// Every guide card that is seeded (the reader has touched it) AND due.
/// Untouched cards never appear — the deck reviews what you practiced, it
/// does not nag about what you skipped.
pub fn due_guide_cards(progress: &UserProgress, today: &str) -> Vec<GuideCard> {
    let mut due = Vec::new();
    for section in SECTIONS.iter() {
        for r in section.recap.iter() {
            let key = recall_key(r.id);
            if let Some(entry) = progress.exercises.get(&key) {
                if is_due(entry.srs.next_review.as_deref(), today) {
                    due.push(GuideCard::Recall {
                        key,
                        section_id: section.id.to_string(),
                        recall_id: r.id.to_string(),
                        question: r.question.to_string(),
                        answer: r.answer.to_string(),
                    });
                }
            }
        }
        for e in section.exercises.iter() {
            let key = exercise_key(e.id);
            if let Some(entry) = progress.exercises.get(&key) {
                if is_due(entry.srs.next_review.as_deref(), today) {
                    due.push(GuideCard::Exercise {
                        key,
                        section_id: section.id.to_string(),
                        exercise_id: e.id.to_string(),
                    });
                }
            }
        }
    }
    due
}
