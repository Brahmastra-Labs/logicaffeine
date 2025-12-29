use dioxus::prelude::*;
use crate::ui::router::Route;
use crate::ui::components::mixed_text::MixedText;
use crate::ui::components::xp_popup::XpPopup;
use crate::ui::components::combo_indicator::ComboIndicator;
use crate::ui::components::achievement_toast::AchievementToast;
use crate::ui::components::app_navbar::AppNavbar;
use crate::content::ContentEngine;
use crate::generator::{Generator, Challenge, AnswerType};
use crate::grader::{check_answer, GradeResult};
use crate::progress::UserProgress;
use crate::game::{XpReward, ComboResult, calculate_xp_reward, update_combo};
use crate::achievements::{Achievement, check_achievements, unlock_achievement};
use crate::audio::{SoundEffect, play_sound};

#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub enum SessionMode {
    Textbook,
    #[default]
    Learning,
    Testing,
}

impl SessionMode {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "textbook" | "read" => SessionMode::Textbook,
            "testing" | "test" => SessionMode::Testing,
            _ => SessionMode::Learning,
        }
    }

    pub fn to_str(&self) -> &'static str {
        match self {
            SessionMode::Textbook => "textbook",
            SessionMode::Learning => "learning",
            SessionMode::Testing => "testing",
        }
    }

    pub fn shows_hints(&self) -> bool {
        matches!(self, SessionMode::Learning)
    }

    pub fn shows_immediate_feedback(&self) -> bool {
        matches!(self, SessionMode::Learning)
    }

    pub fn shows_explanation(&self) -> bool {
        matches!(self, SessionMode::Learning)
    }

    pub fn xp_multiplier(&self) -> f64 {
        match self {
            SessionMode::Textbook => 0.0,
            SessionMode::Learning => 0.5,
            SessionMode::Testing => 1.0,
        }
    }
}

const LESSON_STYLE: &str = r#"
.lesson-container {
    min-height: 100vh;
    background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%);
    color: #e8e8e8;
    display: flex;
    flex-direction: column;
}

.lesson-header {
    padding: 16px 24px;
    background: rgba(0, 0, 0, 0.2);
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    display: flex;
    justify-content: space-between;
    align-items: center;
}

.breadcrumb {
    display: flex;
    align-items: center;
    gap: 8px;
    color: #888;
    font-size: 14px;
}

.breadcrumb a {
    color: #667eea;
    text-decoration: none;
}

.progress-info {
    display: flex;
    align-items: center;
    gap: 16px;
}

.progress-bar {
    width: 200px;
    height: 8px;
    background: rgba(255, 255, 255, 0.1);
    border-radius: 4px;
    overflow: hidden;
}

.progress-fill {
    height: 100%;
    background: linear-gradient(90deg, #667eea, #764ba2);
    transition: width 0.3s ease;
}

.score-display {
    color: #667eea;
    font-weight: 600;
}

.xp-display {
    color: #4ade80;
    font-weight: 600;
    font-size: 14px;
}

.lesson-main {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 40px 20px;
}

.problem-card {
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 20px;
    padding: 40px;
    max-width: 700px;
    width: 100%;
    position: relative;
}

.combo-row {
    position: absolute;
    top: -40px;
    left: 50%;
    transform: translateX(-50%);
}

.problem-prompt {
    color: #888;
    font-size: 14px;
    margin-bottom: 16px;
    text-transform: uppercase;
    letter-spacing: 1px;
}

.problem-sentence {
    font-size: 28px;
    font-weight: 500;
    color: #fff;
    margin-bottom: 32px;
    line-height: 1.4;
}

.answer-input {
    width: 100%;
    padding: 16px 20px;
    font-size: 18px;
    font-family: 'SF Mono', 'Fira Code', monospace;
    background: rgba(255, 255, 255, 0.08);
    border: 2px solid rgba(255, 255, 255, 0.15);
    border-radius: 12px;
    color: #e8e8e8;
    outline: none;
    transition: border-color 0.2s ease;
}

.answer-input:focus {
    border-color: #667eea;
}

.answer-input.correct {
    border-color: #4ade80;
    background: rgba(74, 222, 128, 0.1);
}

.answer-input.incorrect {
    border-color: #f87171;
    background: rgba(248, 113, 113, 0.1);
}

.multiple-choice {
    display: flex;
    flex-direction: column;
    gap: 12px;
}

.choice-btn {
    padding: 16px 20px;
    background: rgba(255, 255, 255, 0.05);
    border: 2px solid rgba(255, 255, 255, 0.1);
    border-radius: 12px;
    color: #e8e8e8;
    font-size: 16px;
    text-align: left;
    cursor: pointer;
    transition: all 0.2s ease;
}

.choice-btn:hover {
    background: rgba(255, 255, 255, 0.08);
    border-color: #667eea;
}

.choice-btn.selected {
    background: rgba(102, 126, 234, 0.2);
    border-color: #667eea;
}

.choice-btn.correct {
    background: rgba(74, 222, 128, 0.2);
    border-color: #4ade80;
}

.choice-btn.incorrect {
    background: rgba(248, 113, 113, 0.2);
    border-color: #f87171;
}

.action-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-top: 24px;
}

.hint-btn {
    padding: 10px 20px;
    background: transparent;
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: 8px;
    color: #888;
    cursor: pointer;
    transition: all 0.2s ease;
}

.hint-btn:hover {
    border-color: #667eea;
    color: #667eea;
}

.submit-btn {
    padding: 12px 32px;
    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
    border: none;
    border-radius: 8px;
    color: white;
    font-size: 16px;
    font-weight: 600;
    cursor: pointer;
    transition: transform 0.2s ease;
}

.submit-btn:hover {
    transform: scale(1.02);
}

.submit-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
}

.feedback-box {
    margin-top: 20px;
    padding: 16px 20px;
    border-radius: 12px;
    font-size: 15px;
}

.feedback-correct {
    background: rgba(74, 222, 128, 0.15);
    border: 1px solid rgba(74, 222, 128, 0.3);
    color: #4ade80;
}

.feedback-incorrect {
    background: rgba(248, 113, 113, 0.15);
    border: 1px solid rgba(248, 113, 113, 0.3);
    color: #f87171;
}

.feedback-partial {
    background: rgba(251, 191, 36, 0.15);
    border: 1px solid rgba(251, 191, 36, 0.3);
    color: #fbbf24;
}

.hint-box {
    margin-top: 16px;
    padding: 16px 20px;
    background: rgba(102, 126, 234, 0.1);
    border: 1px solid rgba(102, 126, 234, 0.2);
    border-radius: 12px;
    color: #a5b4fc;
    font-size: 14px;
}

.explanation-box {
    margin-top: 16px;
    padding: 16px 20px;
    background: rgba(248, 113, 113, 0.08);
    border: 1px solid rgba(248, 113, 113, 0.2);
    border-radius: 12px;
    color: #fca5a5;
    font-size: 14px;
    line-height: 1.6;
}

.explanation-box strong {
    color: #f87171;
    font-weight: 600;
}

.next-btn {
    padding: 12px 32px;
    background: linear-gradient(135deg, #4ade80 0%, #22c55e 100%);
    border: none;
    border-radius: 8px;
    color: white;
    font-size: 16px;
    font-weight: 600;
    cursor: pointer;
}

.complete-message {
    text-align: center;
}

.complete-message h2 {
    font-size: 32px;
    color: #4ade80;
    margin-bottom: 16px;
}

.complete-message p {
    color: #888;
    margin-bottom: 24px;
}

.reading-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
}

.reading-item {
    padding: 12px 16px;
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 8px;
    font-family: 'SF Mono', 'Fira Code', monospace;
    font-size: 14px;
    color: #a5b4fc;
}

.mode-badge {
    padding: 4px 12px;
    border-radius: 12px;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
}

.mode-badge.learning {
    background: rgba(74, 222, 128, 0.2);
    color: #4ade80;
    border: 1px solid rgba(74, 222, 128, 0.3);
}

.mode-badge.testing {
    background: rgba(251, 146, 60, 0.2);
    color: #fb923c;
    border: 1px solid rgba(251, 146, 60, 0.3);
}

.mode-badge.textbook {
    background: rgba(96, 165, 250, 0.2);
    color: #60a5fa;
    border: 1px solid rgba(96, 165, 250, 0.3);
}

.test-summary {
    margin-top: 24px;
    padding: 20px;
    background: rgba(255, 255, 255, 0.03);
    border-radius: 12px;
    border: 1px solid rgba(255, 255, 255, 0.08);
}

.test-summary h3 {
    margin-bottom: 16px;
    color: #888;
    font-size: 14px;
    text-transform: uppercase;
    letter-spacing: 1px;
}

.result-item {
    padding: 12px;
    margin-bottom: 8px;
    border-radius: 8px;
    display: flex;
    align-items: flex-start;
    gap: 12px;
}

.result-item.correct {
    background: rgba(74, 222, 128, 0.1);
    border: 1px solid rgba(74, 222, 128, 0.2);
}

.result-item.incorrect {
    background: rgba(248, 113, 113, 0.1);
    border: 1px solid rgba(248, 113, 113, 0.2);
}

.result-icon {
    font-size: 18px;
}

.result-content {
    flex: 1;
}

.result-question {
    color: #e8e8e8;
    margin-bottom: 4px;
}

.result-explanation {
    color: #888;
    font-size: 13px;
}

.textbook-container {
    max-width: 700px;
    width: 100%;
}

.textbook-card {
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 20px;
    padding: 32px;
    margin-bottom: 20px;
}

.textbook-card h2 {
    color: #fff;
    font-size: 24px;
    margin-bottom: 16px;
}

.textbook-intro {
    color: #aaa;
    font-size: 16px;
    line-height: 1.6;
    margin-bottom: 24px;
}

.example-section {
    margin-top: 24px;
}

.example-section h3 {
    color: #667eea;
    font-size: 14px;
    text-transform: uppercase;
    letter-spacing: 1px;
    margin-bottom: 16px;
}

.example-item {
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 12px;
    padding: 16px;
    margin-bottom: 12px;
}

.example-sentence {
    color: #e8e8e8;
    font-size: 18px;
    margin-bottom: 12px;
}

.example-explanation {
    color: #888;
    font-size: 14px;
    line-height: 1.5;
    padding-left: 16px;
    border-left: 2px solid rgba(102, 126, 234, 0.3);
}

.textbook-nav {
    display: flex;
    justify-content: space-between;
    margin-top: 24px;
}

.textbook-nav-btn {
    padding: 12px 24px;
    background: transparent;
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: 8px;
    color: #888;
    font-size: 14px;
    cursor: pointer;
    transition: all 0.2s ease;
}

.textbook-nav-btn:hover {
    border-color: #667eea;
    color: #667eea;
}

.textbook-nav-btn.primary {
    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
    border: none;
    color: white;
}

.textbook-nav-btn.primary:hover {
    transform: scale(1.02);
}

.page-indicator {
    color: #666;
    font-size: 14px;
    align-self: center;
}
"#;

#[component]
pub fn Lesson(era: String, module: String, mode: String) -> Element {
    let session_mode = SessionMode::from_str(&mode);

    let mut current_index = use_signal(|| 0usize);
    let mut score = use_signal(|| 0u32);
    let mut answer = use_signal(String::new);
    let mut selected_choice = use_signal(|| None::<usize>);
    let mut submitted = use_signal(|| false);
    let mut grade_result = use_signal(|| None::<GradeResult>);
    let mut show_hint = use_signal(|| false);
    let mut challenges = use_signal(Vec::<Challenge>::new);
    let mut initialized = use_signal(|| false);
    let mut test_results = use_signal(Vec::<(usize, bool, String, Option<String>)>::new);

    let mut progress = use_signal(UserProgress::load);
    let mut show_xp_popup = use_signal(|| false);
    let mut current_xp_reward = use_signal(|| None::<XpReward>);
    let mut combo_result = use_signal(|| ComboResult { new_combo: 0, is_new_record: false, multiplier: 1.0 });
    let mut show_achievement = use_signal(|| false);
    let mut current_achievement = use_signal(|| None::<&'static Achievement>);
    let mut first_try_tracker = use_signal(|| std::collections::HashSet::<usize>::new());
    let mut mistakes_in_module = use_signal(|| 0u32);

    let engine = ContentEngine::new();
    let generator = Generator::new();

    let module_data = engine.get_module(&era, &module);

    if !initialized() {
        if let Some(mod_data) = engine.get_module(&era, &module) {
            let mut rng = rand::thread_rng();
            let generated: Vec<Challenge> = mod_data.exercises.iter()
                .filter_map(|ex| generator.generate(ex, &mut rng))
                .collect();
            challenges.set(generated);
            initialized.set(true);
        }
    }

    let total_exercises = challenges.read().len();
    let progress_pct = if total_exercises > 0 {
        ((current_index() + 1) as f64 / total_exercises as f64 * 100.0) as u32
    } else {
        0
    };

    let module_title = module_data.map(|m| m.meta.title.clone()).unwrap_or_default();
    let era_title = match era.as_str() {
        "trivium" => "Basics",
        "quadrivium" => "Quantifiers",
        "metaphysics" => "Modality & Time",
        "logicaffeine" => "Practice",
        _ => "Training",
    };

    let progress_style = format!("width: {}%", progress_pct);
    let user_xp = progress.read().xp;
    let user_combo = progress.read().combo;

    rsx! {
        style { "{LESSON_STYLE}" }

        AppNavbar { title: "Lesson".to_string() }

        if show_xp_popup() {
            if let Some(reward) = current_xp_reward() {
                XpPopup {
                    reward: reward.clone(),
                    on_dismiss: move |_| show_xp_popup.set(false)
                }
            }
        }

        if show_achievement() {
            if let Some(achievement) = current_achievement() {
                AchievementToast {
                    achievement: achievement,
                    on_dismiss: move |_| show_achievement.set(false)
                }
            }
        }

        div { class: "lesson-container",
            header { class: "lesson-header",
                nav { class: "breadcrumb",
                    Link { to: Route::Learn {}, "Curriculum" }
                    span { " > " }
                    span { "{era_title}" }
                    span { " > " }
                    span { "{module_title}" }
                    span {
                        class: "mode-badge {session_mode.to_str()}",
                        style: "margin-left: 12px;",
                        "{session_mode.to_str()}"
                    }
                }
                div { class: "progress-info",
                    div { class: "progress-bar",
                        div {
                            class: "progress-fill",
                            style: "{progress_style}",
                        }
                    }
                    if session_mode.xp_multiplier() > 0.0 {
                        span { class: "xp-display", "{user_xp} XP" }
                    }
                    span { class: "score-display", "Score: {score}" }
                }
            }

            main { class: "lesson-main",
                {
                    let challenges_read = challenges.read();
                    let current = current_index();

                    if current >= total_exercises && total_exercises > 0 {
                        let correct_count = test_results.read().iter().filter(|(_, c, _, _)| *c).count();
                        let results_clone = test_results.read().clone();
                        rsx! {
                            div { class: "problem-card complete-message",
                                h2 { "Module Complete!" }
                                p { "You scored {score} points" }
                                if session_mode == SessionMode::Testing {
                                    p { style: "color: #667eea; font-size: 18px; margin-bottom: 8px;",
                                        "Test Results: {correct_count}/{total_exercises} correct"
                                    }
                                }
                                if mistakes_in_module() == 0 && total_exercises > 0 {
                                    p { style: "color: #fbbf24;", "🏆 Flawless! No mistakes!" }
                                }

                                if session_mode == SessionMode::Testing && !results_clone.is_empty() {
                                    div { class: "test-summary",
                                        h3 { "Review Your Answers" }
                                        for (idx, is_correct, sentence, explanation) in results_clone.iter() {
                                            {
                                                let item_class = if *is_correct { "result-item correct" } else { "result-item incorrect" };
                                                let icon = if *is_correct { "✓" } else { "✗" };
                                                rsx! {
                                                    div { class: "{item_class}",
                                                        span { class: "result-icon", "{icon}" }
                                                        div { class: "result-content",
                                                            p { class: "result-question", "Q{idx + 1}: {sentence}" }
                                                            if !is_correct {
                                                                if let Some(expl) = explanation {
                                                                    p { class: "result-explanation", "{expl}" }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                Link {
                                    class: "submit-btn",
                                    to: Route::Learn {},
                                    "← Back to Curriculum"
                                }
                            }
                        }
                    } else if session_mode == SessionMode::Textbook {
                        let page = current / 5;
                        let total_pages = (challenges_read.len() + 4) / 5;
                        let examples: Vec<_> = challenges_read.iter()
                            .filter_map(|c| {
                                c.explanation.as_ref().map(|e| (c.sentence.clone(), e.clone()))
                            })
                            .skip(page * 5)
                            .take(5)
                            .collect();
                        let era_clone = era.clone();
                        let module_clone = module.clone();
                        rsx! {
                            div { class: "textbook-container",
                                div { class: "textbook-card",
                                    h2 { "{module_title}" }
                                    p { class: "textbook-intro",
                                        "Study the examples below to understand how English sentences translate to first-order logic. Pay attention to the patterns and explanations."
                                    }

                                    div { class: "example-section",
                                        h3 { "Examples" }
                                        for (sentence, explanation) in examples.iter() {
                                            div { class: "example-item",
                                                div { class: "example-sentence",
                                                    MixedText { content: sentence.clone() }
                                                }
                                                div { class: "example-explanation",
                                                    MixedText { content: explanation.clone() }
                                                }
                                            }
                                        }
                                    }

                                    div { class: "textbook-nav",
                                        if page > 0 {
                                            button {
                                                class: "textbook-nav-btn",
                                                onclick: move |_| current_index.set((page - 1) * 5),
                                                "← Previous"
                                            }
                                        } else {
                                            div {}
                                        }

                                        span { class: "page-indicator", "Page {page + 1} of {total_pages}" }

                                        if page + 1 < total_pages {
                                            button {
                                                class: "textbook-nav-btn",
                                                onclick: move |_| current_index.set((page + 1) * 5),
                                                "Next →"
                                            }
                                        } else {
                                            Link {
                                                to: Route::Lesson {
                                                    era: era_clone.clone(),
                                                    module: module_clone.clone(),
                                                    mode: "learning".to_string(),
                                                },
                                                class: "textbook-nav-btn primary",
                                                "Start Practice →"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else if let Some(challenge) = challenges_read.get(current) {
                        let prompt = challenge.prompt.clone();
                        let sentence = challenge.sentence.clone();
                        let hint_text = challenge.hint.clone();
                        let explanation_text = challenge.explanation.clone();
                        let exercise_id = challenge.exercise_id.clone();

                        let input_class = if submitted() {
                            if grade_result().map(|r| r.correct).unwrap_or(false) {
                                "answer-input correct"
                            } else {
                                "answer-input incorrect"
                            }
                        } else {
                            "answer-input"
                        };

                        rsx! {
                            div { class: "problem-card",
                                if user_combo > 0 {
                                    div { class: "combo-row",
                                        ComboIndicator {
                                            combo: user_combo,
                                            multiplier: combo_result().multiplier,
                                            is_new_record: combo_result().is_new_record
                                        }
                                    }
                                }

                                div { class: "problem-prompt", MixedText { content: prompt.clone() } }
                                div { class: "problem-sentence", MixedText { content: sentence.clone() } }

                                {match &challenge.answer {
                                    AnswerType::FreeForm { .. } => rsx! {
                                        input {
                                            class: "{input_class}",
                                            r#type: "text",
                                            placeholder: "Enter your answer in FOL...",
                                            value: "{answer}",
                                            disabled: submitted(),
                                            oninput: move |e| answer.set(e.value()),
                                        }
                                    },
                                    AnswerType::MultipleChoice { options, correct_index } => {
                                        let correct_idx = *correct_index;
                                        let opts = options.clone();
                                        let show_result_colors = session_mode.shows_immediate_feedback();
                                        rsx! {
                                            div { class: "multiple-choice",
                                                for (i, option) in opts.iter().enumerate() {
                                                    {
                                                        let btn_class = if submitted() && show_result_colors {
                                                            if i == correct_idx {
                                                                "choice-btn correct"
                                                            } else if selected_choice() == Some(i) {
                                                                "choice-btn incorrect"
                                                            } else {
                                                                "choice-btn"
                                                            }
                                                        } else if selected_choice() == Some(i) {
                                                            "choice-btn selected"
                                                        } else {
                                                            "choice-btn"
                                                        };
                                                        rsx! {
                                                            button {
                                                                class: "{btn_class}",
                                                                disabled: submitted(),
                                                                onclick: move |_| selected_choice.set(Some(i)),
                                                                MixedText { content: option.clone() }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    },
                                    AnswerType::Ambiguity { readings } => {
                                        let rds = readings.clone();
                                        rsx! {
                                            div { class: "reading-list",
                                                for reading in rds.iter() {
                                                    div { class: "reading-item", "{reading}" }
                                                }
                                            }
                                        }
                                    },
                                }}

                                if session_mode.shows_immediate_feedback() {
                                    if let Some(result) = grade_result() {
                                        {
                                            let fb_class = if result.correct {
                                                "feedback-box feedback-correct"
                                            } else if result.partial {
                                                "feedback-box feedback-partial"
                                            } else {
                                                "feedback-box feedback-incorrect"
                                            };
                                            rsx! {
                                                div { class: "{fb_class}", "{result.feedback}" }
                                            }
                                        }
                                    }
                                }

                                if session_mode.shows_explanation() && submitted() && !grade_result().map(|r| r.correct).unwrap_or(true) {
                                    if let Some(ref expl) = explanation_text {
                                        div { class: "explanation-box",
                                            strong { "Explanation: " }
                                            MixedText { content: expl.clone() }
                                        }
                                    }
                                }

                                if session_mode.shows_hints() && show_hint() && hint_text.is_some() {
                                    div { class: "hint-box", "{hint_text.as_ref().unwrap()}" }
                                }

                                div { class: "action-row",
                                    if session_mode.shows_hints() && !submitted() && hint_text.is_some() {
                                        button {
                                            class: "hint-btn",
                                            onclick: move |_| show_hint.set(true),
                                            "Show Hint"
                                        }
                                    } else {
                                        div {}
                                    }

                                    if submitted() {
                                        button {
                                            class: "next-btn",
                                            onclick: move |_| {
                                                current_index.set(current_index() + 1);
                                                answer.set(String::new());
                                                selected_choice.set(None);
                                                submitted.set(false);
                                                grade_result.set(None);
                                                show_hint.set(false);
                                            },
                                            if current + 1 >= total_exercises {
                                                "Complete Module"
                                            } else {
                                                "Next Problem"
                                            }
                                        }
                                    } else {
                                        {
                                            let can_submit = match &challenge.answer {
                                                AnswerType::FreeForm { .. } => !answer.read().is_empty(),
                                                AnswerType::MultipleChoice { .. } => selected_choice().is_some(),
                                                AnswerType::Ambiguity { .. } => true,
                                            };
                                            let answer_clone = challenge.answer.clone();
                                            let ex_id = exercise_id.clone();
                                            let sentence_for_results = sentence.clone();
                                            let explanation_for_results = explanation_text.clone();
                                            rsx! {
                                                button {
                                                    class: "submit-btn",
                                                    disabled: !can_submit,
                                                    onclick: move |_| {
                                                        let is_correct = match &answer_clone {
                                                            AnswerType::FreeForm { golden_logic } => {
                                                                let result = check_answer(&answer.read(), golden_logic);
                                                                let correct = result.correct;
                                                                if result.correct {
                                                                    score.set(score() + 100);
                                                                } else if result.partial {
                                                                    score.set(score() + result.score);
                                                                }
                                                                grade_result.set(Some(result));
                                                                correct
                                                            }
                                                            AnswerType::MultipleChoice { correct_index, .. } => {
                                                                let correct = selected_choice() == Some(*correct_index);
                                                                let result = if correct {
                                                                    score.set(score() + 100);
                                                                    GradeResult {
                                                                        correct: true,
                                                                        partial: false,
                                                                        score: 100,
                                                                        feedback: "Correct!".to_string(),
                                                                    }
                                                                } else {
                                                                    GradeResult {
                                                                        correct: false,
                                                                        partial: false,
                                                                        score: 0,
                                                                        feedback: "Not quite.".to_string(),
                                                                    }
                                                                };
                                                                grade_result.set(Some(result));
                                                                correct
                                                            }
                                                            AnswerType::Ambiguity { .. } => {
                                                                grade_result.set(Some(GradeResult {
                                                                    correct: true,
                                                                    partial: false,
                                                                    score: 100,
                                                                    feedback: "Good analysis!".to_string(),
                                                                }));
                                                                score.set(score() + 100);
                                                                true
                                                            }
                                                        };

                                                        let is_first_try = !first_try_tracker.read().contains(&current);
                                                        first_try_tracker.write().insert(current);

                                                        {
                                                            let mut prog = progress.write();
                                                            prog.record_attempt(&ex_id, is_correct);

                                                            let cr = update_combo(&mut prog, is_correct);
                                                            combo_result.set(cr.clone());

                                                            if is_correct {
                                                                play_sound(SoundEffect::Correct);

                                                                let xp_mult = session_mode.xp_multiplier();
                                                                if xp_mult > 0.0 {
                                                                    let rng_seed = (prog.xp + current as u64) % 100;
                                                                    let mut reward = calculate_xp_reward(
                                                                        1,
                                                                        cr.new_combo,
                                                                        prog.streak_days,
                                                                        is_first_try,
                                                                        rng_seed,
                                                                    );

                                                                    reward.total = (reward.total as f64 * xp_mult) as u64;
                                                                    prog.xp += reward.total;
                                                                    prog.level = crate::progress::calculate_level(prog.xp);
                                                                    prog.save();

                                                                    current_xp_reward.set(Some(reward));
                                                                    show_xp_popup.set(true);

                                                                    let new_achievements = check_achievements(&prog);
                                                                    if let Some(achievement) = new_achievements.first() {
                                                                        current_achievement.set(Some(*achievement));
                                                                        show_achievement.set(true);
                                                                        unlock_achievement(&mut prog, achievement);
                                                                    }
                                                                }
                                                            } else {
                                                                play_sound(SoundEffect::Incorrect);
                                                                mistakes_in_module.set(mistakes_in_module() + 1);
                                                            }
                                                        }

                                                        if session_mode == SessionMode::Testing {
                                                            test_results.write().push((
                                                                current,
                                                                is_correct,
                                                                sentence_for_results.clone(),
                                                                explanation_for_results.clone(),
                                                            ));
                                                        }

                                                        submitted.set(true);
                                                    },
                                                    "Check Answer"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        rsx! {
                            div { class: "problem-card",
                                p { "Loading exercises..." }
                            }
                        }
                    }
                }
            }
        }
    }
}
