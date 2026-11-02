//! Learn page state: where the learner stands on the roadmap, the question that closes a lesson,
//! and the optional practice behind it.
//!
//! Everything here is pure (no Dioxus, no browser), so every rule is pinned by unit tests. The page
//! renders these types and forwards clicks and keystrokes to them; it never keeps a second copy of
//! the rules.
//!
//! - [`Journey`] walks the roadmap: which lesson is open and which are done. Any lesson can be
//!   opened; finishing one and moving on opens the next in roadmap order.
//! - [`LessonQuiz`] is a lesson's one question: answer it right, or try once and then ask to see the
//!   answer, and the learner may move on.
//! - [`PracticeSession`] walks a lesson's extra questions in order, bringing a missed one back after
//!   [`RETRY_GAP`] others.

use std::collections::{BTreeSet, HashSet, VecDeque};

/// Where a learner stands on the roadmap: every lesson in order, the ones done, the one open.
#[derive(Debug, Clone, PartialEq)]
pub struct Journey {
    order: Vec<String>,
    done: BTreeSet<String>,
    open: usize,
}

impl Journey {
    /// `order` is every lesson id in roadmap order; `done` the lessons finished before (an id not on
    /// the roadmap is dropped). `remembered` is the lesson open last time: it opens again while it is
    /// on the roadmap, else the first lesson not done opens, else the first lesson.
    pub fn new(
        order: Vec<String>,
        done: impl IntoIterator<Item = String>,
        remembered: Option<&str>,
    ) -> Self {
        let on_roadmap: HashSet<&str> = order.iter().map(String::as_str).collect();
        let done: BTreeSet<String> = done
            .into_iter()
            .filter(|id| on_roadmap.contains(id.as_str()))
            .collect();
        let open = remembered
            .and_then(|id| order.iter().position(|o| o == id))
            .or_else(|| order.iter().position(|o| !done.contains(o)))
            .unwrap_or(0);
        Self { order, done, open }
    }

    /// The open lesson, or `None` for an empty roadmap.
    pub fn open(&self) -> Option<&str> {
        self.order.get(self.open).map(String::as_str)
    }

    /// The lesson after the open one.
    pub fn next(&self) -> Option<&str> {
        self.order.get(self.open + 1).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn done_count(&self) -> usize {
        self.done.len()
    }

    pub fn is_done(&self, id: &str) -> bool {
        self.done.contains(id)
    }

    /// Whether every lesson on a non-empty roadmap is done.
    pub fn is_finished(&self) -> bool {
        !self.order.is_empty() && self.done.len() == self.order.len()
    }

    /// The lessons done, in id order.
    pub fn done_ids(&self) -> impl Iterator<Item = &str> + '_ {
        self.done.iter().map(String::as_str)
    }

    /// Mark the open lesson done. It stays open.
    pub fn finish(&mut self) {
        if let Some(id) = self.order.get(self.open) {
            self.done.insert(id.clone());
        }
    }

    /// Open the next lesson in roadmap order. `false` at the end of the roadmap.
    pub fn advance(&mut self) -> bool {
        let moved = self.open + 1 < self.order.len();
        if moved {
            self.open += 1;
        }
        moved
    }

    /// Open `id`. `false` when it is not on the roadmap.
    pub fn open_lesson(&mut self, id: &str) -> bool {
        match self.order.iter().position(|o| o == id) {
            Some(at) => {
                self.open = at;
                true
            }
            None => false,
        }
    }

    /// Open the first of `ids` not done, or the first of them when all are done: how a link to a
    /// module opens it. `false` when none of `ids` is on the roadmap.
    pub fn open_first_unfinished(&mut self, ids: &[&str]) -> bool {
        let on_roadmap: Vec<&str> = ids
            .iter()
            .copied()
            .filter(|id| self.order.iter().any(|o| o == id))
            .collect();
        match on_roadmap
            .iter()
            .find(|id| !self.done.contains(**id))
            .or(on_roadmap.first())
        {
            Some(id) => self.open_lesson(id),
            None => false,
        }
    }
}

/// Where a lesson's question stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QuizState {
    /// Waiting for a right answer.
    #[default]
    Answering,
    /// Answered right.
    Correct,
    /// The learner asked to see the answer after a try.
    Shown,
}

/// A lesson's one question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LessonQuiz {
    state: QuizState,
    wrong: u32,
}

impl LessonQuiz {
    pub fn state(&self) -> QuizState {
        self.state
    }

    /// Wrong answers so far.
    pub fn wrong_attempts(&self) -> u32 {
        self.wrong
    }

    /// Grade an answer. Ignored once the question is settled.
    pub fn answer(&mut self, correct: bool) {
        if self.state != QuizState::Answering {
            return;
        }
        if correct {
            self.state = QuizState::Correct;
        } else {
            self.wrong += 1;
        }
    }

    /// Whether the answer may be shown: after a wrong try, before the question is settled.
    pub fn can_show_answer(&self) -> bool {
        self.state == QuizState::Answering && self.wrong > 0
    }

    /// Show the answer. `false` when it may not be shown yet.
    pub fn show_answer(&mut self) -> bool {
        let allowed = self.can_show_answer();
        if allowed {
            self.state = QuizState::Shown;
        }
        allowed
    }

    /// Whether the question is answered or shown: it takes no more answers.
    pub fn is_settled(&self) -> bool {
        self.state != QuizState::Answering
    }

    /// Whether the learner may move on to the next lesson.
    pub fn can_continue(&self) -> bool {
        self.is_settled()
    }
}

/// How many other questions come between a missed one and its retry.
pub const RETRY_GAP: usize = 3;

/// A walk through a lesson's practice questions.
#[derive(Debug, Clone, PartialEq)]
pub struct PracticeSession {
    total: usize,
    current: usize,
    /// Where the in-order walk stands; a retry interrupts the walk without moving it.
    cursor: usize,
    served: usize,
    retries: VecDeque<Retry>,
    completed: HashSet<usize>,
    revealed: HashSet<usize>,
    solved_current: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Retry {
    index: usize,
    due: usize,
}

impl PracticeSession {
    /// A session over `total` questions. `completed` are questions answered right before: the walk
    /// starts at the first one not in it.
    pub fn new(total: usize, completed: impl IntoIterator<Item = usize>) -> Self {
        let completed: HashSet<usize> = completed.into_iter().filter(|&i| i < total).collect();
        let current = (0..total).find(|i| !completed.contains(i)).unwrap_or(0);
        Self {
            total,
            current,
            cursor: current,
            served: 0,
            retries: VecDeque::new(),
            completed,
            revealed: HashSet::new(),
            solved_current: false,
        }
    }

    /// The question on screen, or `None` when there are none.
    pub fn current(&self) -> Option<usize> {
        (self.total > 0).then_some(self.current)
    }

    pub fn total(&self) -> usize {
        self.total
    }

    /// Questions answered right, in this session or an earlier one.
    pub fn completed_count(&self) -> usize {
        self.completed.len()
    }

    pub fn is_completed(&self, index: usize) -> bool {
        self.completed.contains(&index)
    }

    /// Counts arrivals: every Next or Skip starts a new visit, even when it lands back on the same
    /// question, so a view keyed by it always opens a question with a blank answer.
    pub fn visit(&self) -> usize {
        self.served
    }

    /// Whether the question on screen has been answered right since it came up.
    pub fn solved_current(&self) -> bool {
        self.solved_current
    }

    /// Whether the answer to the question on screen has been shown.
    pub fn is_revealed(&self) -> bool {
        self.revealed.contains(&self.current)
    }

    /// Missed questions waiting to come back, in the order they will.
    pub fn pending_retries(&self) -> Vec<usize> {
        self.retries.iter().map(|r| r.index).collect()
    }

    /// Grade the question on screen. A wrong answer to a question not yet completed queues it to come
    /// back after [`RETRY_GAP`] others.
    pub fn answer(&mut self, correct: bool) {
        if self.total == 0 {
            return;
        }
        let index = self.current;
        if correct {
            self.solved_current = true;
            self.completed.insert(index);
        } else if !self.completed.contains(&index) && !self.retries.iter().any(|r| r.index == index)
        {
            self.retries.push_back(Retry {
                index,
                due: self.served + RETRY_GAP + 1,
            });
        }
    }

    /// Show the answer to the question on screen.
    pub fn reveal_answer(&mut self) {
        if self.total > 0 {
            self.revealed.insert(self.current);
        }
    }

    /// Move on (Next or Skip). A missed question whose retry is due comes first; otherwise the next
    /// question not yet completed, wrapping around; once everything is completed, the walk simply
    /// continues in order.
    pub fn advance(&mut self) {
        if self.total == 0 {
            return;
        }
        self.served += 1;
        self.solved_current = false;

        let mut still_waiting = VecDeque::new();
        let mut next_retry = None;
        while let Some(retry) = self.retries.pop_front() {
            if self.completed.contains(&retry.index) {
                continue;
            }
            if next_retry.is_none() && retry.due <= self.served && retry.index != self.current {
                next_retry = Some(retry.index);
            } else {
                still_waiting.push_back(retry);
            }
        }
        self.retries = still_waiting;

        self.current = match next_retry {
            Some(index) => index,
            None => {
                self.cursor = self.next_in_order();
                self.cursor
            }
        };
    }

    fn next_in_order(&self) -> usize {
        let ahead = || (1..=self.total).map(|step| (self.cursor + step) % self.total);
        ahead()
            .find(|i| !self.completed.contains(i) && *i != self.current)
            .or_else(|| ahead().find(|i| *i != self.current))
            .unwrap_or(self.current)
    }
}

#[cfg(test)]
#[path = "../../../crates/logicaffeine_tests/unit/logicaffeine_web/learn_state/tests.rs"]
mod tests;
