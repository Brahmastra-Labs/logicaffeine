use super::*;

fn fresh(total: usize) -> PracticeSession {
    PracticeSession::new(total, [])
}

fn ids(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

fn done(names: &[&str]) -> Vec<String> {
    ids(names)
}

fn journey(order: &[&str], finished: &[&str], remembered: Option<&str>) -> Journey {
    Journey::new(ids(order), done(finished), remembered)
}

// --- The journey along the roadmap ---------------------------------------------------

#[test]
fn a_first_visit_opens_the_first_lesson() {
    let j = journey(&["1.1", "1.2", "1.3"], &[], None);
    assert_eq!(j.open(), Some("1.1"));
    assert_eq!((j.done_count(), j.len()), (0, 3));
}

#[test]
fn a_returning_learner_opens_the_lesson_they_left_open() {
    let j = journey(&["1.1", "1.2", "1.3"], &["1.1"], Some("1.3"));
    assert_eq!(j.open(), Some("1.3"));
}

#[test]
fn with_nothing_remembered_the_first_lesson_not_done_opens() {
    let j = journey(&["1.1", "1.2", "1.3"], &["1.1", "1.2"], None);
    assert_eq!(j.open(), Some("1.3"));
}

#[test]
fn a_remembered_lesson_that_left_the_roadmap_is_ignored() {
    let j = journey(&["1.1", "1.2", "1.3"], &["1.1"], Some("9.9"));
    assert_eq!(j.open(), Some("1.2"));
}

#[test]
fn when_every_lesson_is_done_the_first_one_opens() {
    let j = journey(&["1.1", "1.2"], &["1.1", "1.2"], None);
    assert_eq!(j.open(), Some("1.1"));
    assert!(j.is_finished());
}

#[test]
fn lessons_done_that_left_the_roadmap_are_not_counted() {
    let j = journey(&["1.1", "1.2"], &["1.1", "0.7", "9.9"], None);
    assert_eq!(j.done_count(), 1);
    assert!(j.is_done("1.1"));
    assert!(!j.is_done("0.7"));
    assert_eq!(j.done_ids().collect::<Vec<_>>(), ["1.1"]);
}

#[test]
fn an_empty_roadmap_opens_nothing() {
    let mut j = journey(&[], &[], Some("1.1"));
    assert_eq!(j.open(), None);
    j.finish();
    assert!(!j.advance());
    assert_eq!((j.done_count(), j.len()), (0, 0));
    assert!(!j.is_finished(), "an empty roadmap has nothing to finish");
}

#[test]
fn finishing_marks_the_open_lesson_done() {
    let mut j = journey(&["1.1", "1.2"], &[], None);
    j.finish();
    assert!(j.is_done("1.1"));
    assert_eq!(j.open(), Some("1.1"), "finishing does not move on by itself");
    j.finish();
    assert_eq!(j.done_count(), 1, "finishing twice counts once");
}

#[test]
fn advancing_opens_the_next_lesson_in_order() {
    let mut j = journey(&["1.1", "1.2", "2.1"], &[], None);
    assert_eq!(j.next(), Some("1.2"));
    assert!(j.advance());
    assert_eq!(j.open(), Some("1.2"));
    assert!(j.advance());
    assert_eq!(j.open(), Some("2.1"));
    assert_eq!(j.next(), None);
    assert!(!j.advance(), "the last lesson has no next");
    assert_eq!(j.open(), Some("2.1"));
}

#[test]
fn advancing_moves_to_the_next_lesson_even_when_it_was_done_before() {
    let mut j = journey(&["1.1", "1.2", "1.3"], &["1.2"], None);
    assert_eq!(j.open(), Some("1.1"));
    j.finish();
    j.advance();
    assert_eq!(j.open(), Some("1.2"), "the roadmap is walked in order, a review included");
}

#[test]
fn any_lesson_can_be_opened_from_the_roadmap() {
    let mut j = journey(&["1.1", "1.2", "1.3"], &[], None);
    assert!(j.open_lesson("1.3"), "no lesson is locked");
    assert_eq!(j.open(), Some("1.3"));
    assert!(!j.open_lesson("4.4"), "a lesson that is not on the roadmap");
    assert_eq!(j.open(), Some("1.3"));
}

#[test]
fn opening_a_module_opens_its_first_lesson_not_done() {
    let mut j = journey(&["1.1", "2.1", "2.2", "2.3"], &["2.1"], None);
    assert!(j.open_first_unfinished(&["2.1", "2.2", "2.3"]));
    assert_eq!(j.open(), Some("2.2"));
}

#[test]
fn opening_a_finished_module_opens_its_first_lesson() {
    let mut j = journey(&["1.1", "2.1", "2.2"], &["2.1", "2.2"], None);
    assert!(j.open_first_unfinished(&["2.1", "2.2"]));
    assert_eq!(j.open(), Some("2.1"));
}

#[test]
fn opening_a_module_with_no_lessons_on_the_roadmap_changes_nothing() {
    let mut j = journey(&["1.1", "1.2"], &[], None);
    assert!(!j.open_first_unfinished(&[]));
    assert!(!j.open_first_unfinished(&["7.1"]));
    assert_eq!(j.open(), Some("1.1"));
}

#[test]
fn the_journey_is_finished_only_when_every_lesson_is_done() {
    let mut j = journey(&["1.1", "1.2"], &[], None);
    assert!(!j.is_finished());
    j.finish();
    assert!(!j.is_finished());
    j.advance();
    j.finish();
    assert!(j.is_finished());
}

#[test]
fn walking_the_whole_roadmap_finishes_every_lesson_once() {
    let order: Vec<String> = (1..=12).map(|n| format!("{}.{}", n / 4 + 1, n % 4 + 1)).collect();
    let mut j = Journey::new(order.clone(), Vec::new(), None);
    let mut seen = Vec::new();
    loop {
        seen.push(j.open().unwrap().to_string());
        j.finish();
        if !j.advance() {
            break;
        }
    }
    assert_eq!(seen, order);
    assert_eq!(j.done_count(), order.len());
    assert!(j.is_finished());
}

// --- One lesson's question -----------------------------------------------------------

#[test]
fn a_new_question_waits_for_an_answer() {
    let q = LessonQuiz::default();
    assert_eq!(q.state(), QuizState::Answering);
    assert!(!q.can_continue());
    assert!(!q.can_show_answer(), "the answer is offered only after a try");
    assert_eq!(q.wrong_attempts(), 0);
}

#[test]
fn a_right_answer_lets_the_learner_continue() {
    let mut q = LessonQuiz::default();
    q.answer(true);
    assert_eq!(q.state(), QuizState::Correct);
    assert!(q.can_continue());
    assert!(!q.can_show_answer());
}

#[test]
fn a_wrong_answer_counts_and_offers_the_answer() {
    let mut q = LessonQuiz::default();
    q.answer(false);
    assert_eq!(q.state(), QuizState::Answering);
    assert_eq!(q.wrong_attempts(), 1);
    assert!(!q.can_continue(), "a wrong answer does not move on");
    assert!(q.can_show_answer());
    q.answer(false);
    assert_eq!(q.wrong_attempts(), 2);
}

#[test]
fn trying_again_after_a_miss_can_still_be_right() {
    let mut q = LessonQuiz::default();
    q.answer(false);
    q.answer(true);
    assert_eq!(q.state(), QuizState::Correct);
    assert!(q.can_continue());
}

#[test]
fn showing_the_answer_lets_the_learner_continue() {
    let mut q = LessonQuiz::default();
    assert!(!q.show_answer(), "nothing to show before a try");
    assert_eq!(q.state(), QuizState::Answering);
    q.answer(false);
    assert!(q.show_answer());
    assert_eq!(q.state(), QuizState::Shown);
    assert!(q.can_continue());
    assert!(!q.can_show_answer(), "shown once");
}

#[test]
fn a_settled_question_takes_no_more_answers() {
    let mut q = LessonQuiz::default();
    q.answer(true);
    q.answer(false);
    assert_eq!((q.state(), q.wrong_attempts()), (QuizState::Correct, 0));

    let mut q = LessonQuiz::default();
    q.answer(false);
    q.show_answer();
    q.answer(true);
    assert_eq!(q.state(), QuizState::Shown, "an answer typed after the reveal does not count as solved");
    assert!(q.is_settled());
}

// --- Practice: walking a lesson's extra questions ------------------------------------

#[test]
fn a_right_answer_completes_the_exercise() {
    let mut s = fresh(3);
    s.answer(true);
    assert!(s.is_completed(0));
    assert_eq!(s.completed_count(), 1);
}

#[test]
fn a_revealed_answer_still_completes_when_answered() {
    let mut s = fresh(3);
    s.reveal_answer();
    assert!(s.is_revealed());
    s.answer(true);
    assert!(s.is_completed(0));
}

#[test]
fn revealing_does_not_leak_onto_the_next_exercise() {
    let mut s = fresh(3);
    s.reveal_answer();
    s.advance();
    assert!(!s.is_revealed());
}

#[test]
fn answering_a_completed_exercise_again_counts_it_once() {
    let mut s = fresh(3);
    for _ in 0..5 {
        s.answer(true);
    }
    assert_eq!(s.completed_count(), 1);
}

#[test]
fn solved_current_tracks_the_exercise_on_screen() {
    let mut s = fresh(3);
    assert!(!s.solved_current());
    s.answer(false);
    assert!(!s.solved_current());
    s.answer(true);
    assert!(s.solved_current());
    s.advance();
    assert!(!s.solved_current());
}

#[test]
fn walk_goes_in_order_and_wraps() {
    let mut s = fresh(3);
    let mut seen = vec![s.current().unwrap()];
    for _ in 0..5 {
        s.advance();
        seen.push(s.current().unwrap());
    }
    assert_eq!(seen, [0, 1, 2, 0, 1, 2]);
}

#[test]
fn a_returning_learner_starts_at_the_first_unfinished_exercise() {
    let s = PracticeSession::new(5, [0, 1, 3]);
    assert_eq!(s.current(), Some(2));
}

#[test]
fn the_walk_skips_exercises_already_completed() {
    let mut s = PracticeSession::new(6, [1, 2, 4]);
    assert_eq!(s.current(), Some(0));
    s.advance();
    assert_eq!(s.current(), Some(3));
    s.advance();
    assert_eq!(s.current(), Some(5));
    s.advance();
    assert_eq!(s.current(), Some(0));
}

#[test]
fn once_everything_is_completed_the_walk_reviews_in_order() {
    let mut s = PracticeSession::new(3, [0, 1, 2]);
    assert_eq!(s.current(), Some(0));
    s.advance();
    assert_eq!(s.current(), Some(1));
    s.advance();
    assert_eq!(s.current(), Some(2));
    s.advance();
    assert_eq!(s.current(), Some(0));
}

#[test]
fn stale_completed_indices_are_ignored() {
    let s = PracticeSession::new(2, [0, 7, 99]);
    assert_eq!(s.completed_count(), 1);
    assert_eq!(s.current(), Some(1));
}

#[test]
fn a_skipped_miss_comes_back_after_three_others() {
    let mut s = fresh(10);
    assert_eq!(s.current(), Some(0));
    s.answer(false);
    let mut seen = Vec::new();
    for _ in 0..5 {
        s.advance();
        seen.push(s.current().unwrap());
    }
    assert_eq!(seen, [1, 2, 3, 0, 4]);
}

#[test]
fn a_miss_fixed_on_the_spot_does_not_come_back() {
    let mut s = fresh(10);
    s.answer(false);
    s.answer(true);
    let mut seen = Vec::new();
    for _ in 0..5 {
        s.advance();
        seen.push(s.current().unwrap());
    }
    assert_eq!(seen, [1, 2, 3, 4, 5]);
    assert!(s.pending_retries().is_empty());
}

#[test]
fn a_miss_is_queued_once_however_often_it_is_missed() {
    let mut s = fresh(10);
    s.answer(false);
    s.answer(false);
    s.answer(false);
    assert_eq!(s.pending_retries(), [0]);
}

#[test]
fn missing_a_retry_again_requeues_it() {
    let mut s = fresh(10);
    s.answer(false);
    for _ in 0..4 {
        s.advance();
    }
    assert_eq!(s.current(), Some(0));
    s.answer(false);
    assert_eq!(s.pending_retries(), [0]);
    let mut seen = Vec::new();
    for _ in 0..4 {
        s.advance();
        seen.push(s.current().unwrap());
    }
    assert_eq!(seen, [4, 5, 6, 0]);
}

#[test]
fn several_misses_return_in_the_order_they_were_missed() {
    let mut s = fresh(12);
    s.answer(false);
    s.advance();
    s.answer(false);
    let mut seen = Vec::new();
    for _ in 0..6 {
        s.advance();
        seen.push(s.current().unwrap());
    }
    assert_eq!(seen, [2, 3, 0, 1, 4, 5]);
}

#[test]
fn missing_a_completed_exercise_never_queues_it() {
    let mut s = fresh(5);
    s.answer(true);
    s.answer(false);
    assert!(s.pending_retries().is_empty());
    assert!(s.is_completed(0));
}

#[test]
fn the_same_exercise_never_shows_twice_in_a_row() {
    for total in 2..8 {
        let mut s = fresh(total);
        let mut last = s.current().unwrap();
        for step in 0..60 {
            if step % 3 == 0 {
                s.answer(false);
            } else if step % 5 == 0 {
                s.answer(true);
            }
            s.advance();
            let now = s.current().unwrap();
            assert_ne!(now, last, "total {total}, step {step}: {now} twice in a row");
            last = now;
        }
    }
}

#[test]
fn every_move_is_a_new_visit() {
    let mut s = fresh(1);
    assert_eq!(s.visit(), 0);
    s.answer(true);
    assert_eq!(s.visit(), 0, "answering is not a move");
    s.advance();
    assert_eq!((s.current(), s.visit()), (Some(0), 1), "the same exercise, a new visit");
    s.advance();
    assert_eq!(s.visit(), 2);
}

#[test]
fn a_single_exercise_lesson_stays_on_it() {
    let mut s = fresh(1);
    s.answer(false);
    s.advance();
    assert_eq!(s.current(), Some(0));
    s.advance();
    assert_eq!(s.current(), Some(0));
}

#[test]
fn an_empty_practice_set_has_nothing_to_show_or_complete() {
    let mut s = fresh(0);
    assert_eq!(s.current(), None);
    s.advance();
    s.reveal_answer();
    s.answer(true);
    assert_eq!((s.current(), s.completed_count()), (None, 0));
}
