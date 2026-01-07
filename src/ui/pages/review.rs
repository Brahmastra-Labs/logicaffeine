use dioxus::prelude::*;
use crate::ui::router::Route;
use crate::ui::components::mixed_text::MixedText;
use crate::ui::components::xp_popup::XpPopup;
use crate::ui::components::combo_indicator::ComboIndicator;
use crate::ui::components::streak_display::StreakDisplay;
use crate::ui::components::achievement_toast::AchievementToast;
use crate::ui::components::main_nav::{MainNav, ActivePage};
use crate::content::ContentEngine;
use crate::generator::{Generator, Challenge, AnswerType};
use crate::grader::{check_answer, GradeResult};
use crate::progress::UserProgress;
use crate::srs::{ResponseQuality, sm2_update, calculate_next_review, is_due};
use crate::game::{XpReward, ComboResult, StreakStatus, calculate_xp_reward, update_combo, update_streak};
use crate::achievements::{Achievement, check_achievements, unlock_achievement};
use crate::audio::{SoundEffect, play_sound};

const REVIEW_STYLE: &str = r#"
.review-container {
    min-height: 100vh;
    background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%);
    color: #e8e8e8;
    display: flex;
    flex-direction: column;
}

.review-header {
    padding: 16px 24px;
    background: rgba(0, 0, 0, 0.2);
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    display: flex;
    justify-content: space-between;
    align-items: center;
}

.review-title {
    font-size: 20px;
    font-weight: 600;
    color: #a5b4fc;
}

.review-count {
    color: #888;
    font-size: 14px;
}

.review-stats {
    display: flex;
    align-items: center;
    gap: 16px;
}

.xp-display {
    color: #4ade80;
    font-weight: 600;
    font-size: 14px;
}

.combo-row {
    position: absolute;
    top: -40px;
    left: 50%;
    transform: translateX(-50%);
}

.review-main {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 40px 20px;
}

.review-card {
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 20px;
    padding: 40px;
    max-width: 700px;
    width: 100%;
    position: relative;
}

.review-prompt {
    color: #888;
    font-size: 14px;
    margin-bottom: 16px;
    text-transform: uppercase;
    letter-spacing: 1px;
}

.review-sentence {
    font-size: 28px;
    font-weight: 500;
    color: #fff;
    margin-bottom: 32px;
    line-height: 1.4;
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
}

.answer-input.correct {
    border-color: #4ade80;
    background: rgba(74, 222, 128, 0.1);
}

.answer-input.incorrect {
    border-color: #f87171;
    background: rgba(248, 113, 113, 0.1);
}

.feedback-box {
    margin-top: 20px;
    padding: 16px 20px;
    border-radius: 12px;
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

.srs-buttons {
    display: flex;
    gap: 12px;
    margin-top: 24px;
    flex-wrap: wrap;
}

.srs-btn {
    padding: 12px 20px;
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: 8px;
    background: transparent;
    color: #e8e8e8;
    cursor: pointer;
    font-size: 14px;
    transition: all 0.2s ease;
}

.srs-btn:hover {
    background: rgba(255, 255, 255, 0.08);
}

.srs-btn.hard {
    border-color: rgba(248, 113, 113, 0.5);
    color: #f87171;
}

.srs-btn.good {
    border-color: rgba(251, 191, 36, 0.5);
    color: #fbbf24;
}

.srs-btn.easy {
    border-color: rgba(74, 222, 128, 0.5);
    color: #4ade80;
}

.action-row {
    display: flex;
    justify-content: flex-end;
    margin-top: 24px;
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
}

.submit-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
}

.empty-state {
    text-align: center;
    padding: 60px 40px;
}

.empty-state h2 {
    font-size: 28px;
    color: #4ade80;
    margin-bottom: 16px;
}

.empty-state p {
    color: #888;
    margin-bottom: 24px;
}

.back-btn {
    display: inline-block;
    padding: 12px 24px;
    background: linear-gradient(135deg, #667eea, #764ba2);
    border-radius: 8px;
    color: white;
    text-decoration: none;
}

/* Mobile touch target optimizations */
@media (max-width: 768px) {
    .review-header {
        padding: 12px 16px;
        flex-wrap: wrap;
        gap: 8px;
    }

    .review-title {
        font-size: 18px;
    }

    .review-main {
        padding: 24px 16px;
    }

    .review-card {
        padding: 24px 18px;
        border-radius: 16px;
    }

    .review-sentence {
        font-size: 22px;
        margin-bottom: 24px;
    }

    .choice-btn {
        min-height: 48px;
        padding: 14px 18px;
        -webkit-tap-highlight-color: transparent;
        touch-action: manipulation;
    }

    .answer-input {
        min-height: 48px;
        font-size: 16px;
        padding: 14px 16px;
    }

    .srs-btn {
        min-height: 44px;
        padding: 10px 18px;
        -webkit-tap-highlight-color: transparent;
        touch-action: manipulation;
    }

    .srs-buttons {
        gap: 10px;
    }

    .submit-btn {
        min-height: 48px;
        padding: 14px 28px;
        -webkit-tap-highlight-color: transparent;
        touch-action: manipulation;
    }

    .back-btn {
        min-height: 44px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        padding: 12px 24px;
        -webkit-tap-highlight-color: transparent;
        touch-action: manipulation;
    }

    .feedback-box {
        padding: 14px 16px;
    }
}

@media (max-width: 480px) {
    .review-header {
        padding: 10px 12px;
    }

    .review-title {
        font-size: 16px;
    }

    .review-count {
        font-size: 12px;
    }

    .xp-display {
        font-size: 12px;
    }

    .review-main {
        padding: 16px 12px;
    }

    .review-card {
        padding: 18px 14px;
    }

    .review-prompt {
        font-size: 12px;
    }

    .review-sentence {
        font-size: 18px;
        margin-bottom: 20px;
    }

    .choice-btn {
        font-size: 14px;
        padding: 12px 14px;
    }

    .answer-input {
        font-size: 14px;
    }

    .srs-buttons {
        flex-direction: column;
        gap: 8px;
    }

    .srs-btn {
        width: 100%;
        justify-content: center;
        display: flex;
    }

    .submit-btn {
        width: 100%;
    }

    .action-row {
        justify-content: stretch;
    }

    .empty-state {
        padding: 40px 20px;
    }

    .empty-state h2 {
        font-size: 22px;
    }

    .combo-row {
        top: -32px;
    }
}
"#;

#[component]
pub fn Review() -> Element {
    let mut current_index = use_signal(|| 0usize);
    let mut answer = use_signal(String::new);
    let mut selected_choice = use_signal(|| None::<usize>);
    let mut submitted = use_signal(|| false);
    let mut grade_result = use_signal(|| None::<GradeResult>);
    let mut due_challenges = use_signal(Vec::<(String, Challenge)>::new);
    let mut initialized = use_signal(|| false);
    let mut progress = use_signal(UserProgress::load);

    let mut show_xp_popup = use_signal(|| false);
    let mut current_xp_reward = use_signal(|| None::<XpReward>);
    let mut combo_result = use_signal(|| ComboResult { new_combo: 0, is_new_record: false, multiplier: 1.0 });
    let mut show_achievement = use_signal(|| false);
    let mut current_achievement = use_signal(|| None::<&'static Achievement>);
    let mut streak_status = use_signal(|| None::<StreakStatus>);
    let mut first_try_tracker = use_signal(|| std::collections::HashSet::<usize>::new());

    let engine = ContentEngine::new();
    let generator = Generator::new();

    if !initialized() {
        let today = get_today();

        {
            let mut prog = progress.write();
            let status = update_streak(&mut prog, &today);
            streak_status.set(Some(status.clone()));

            match &status {
                StreakStatus::Frozen => play_sound(SoundEffect::StreakSaved),
                StreakStatus::Lost { .. } => play_sound(SoundEffect::StreakLost),
                _ => {}
            }

            prog.save();
        }

        let user_progress = progress.read();
        let mut challenges = Vec::new();

        for era in engine.eras() {
            if let Some(era_data) = engine.get_era(&era.meta.id) {
                for module in &era_data.modules {
                    for exercise in &module.exercises {
                        let exercise_id = &exercise.id;
                        let srs_due = user_progress
                            .get_exercise_progress(exercise_id)
                            .map(|ep| is_due(ep.srs.next_review.as_deref(), &today))
                            .unwrap_or(true);

                        if srs_due {
                            let mut rng = rand::thread_rng();
                            if let Some(challenge) = generator.generate(exercise, &mut rng) {
                                challenges.push((exercise_id.clone(), challenge));
                            }
                        }
                    }
                }
            }
        }

        due_challenges.set(challenges);
        initialized.set(true);
    }

    let total_due = due_challenges.read().len();
    let current = current_index();

    let user_xp = progress.read().xp;
    let user_combo = progress.read().combo;
    let user_streak = progress.read().streak_days;
    let user_freezes = progress.read().streak_freezes;

    rsx! {
        style { "{REVIEW_STYLE}" }

        MainNav { active: ActivePage::Learn, subtitle: Some("Daily Review") }

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

        div { class: "review-container",
            header { class: "review-header",
                div {
                    span { class: "review-title", "Daily Review" }
                    span { class: "review-count",
                        if total_due > 0 {
                            " • {current + 1} of {total_due}"
                        } else {
                            ""
                        }
                    }
                }
                div { class: "review-stats",
                    if let Some(status) = streak_status() {
                        StreakDisplay {
                            streak: user_streak,
                            status: status,
                            freezes: user_freezes
                        }
                    }
                    span { class: "xp-display", "{user_xp} XP" }
                }
            }

            main { class: "review-main",
                {
                    let challenges_read = due_challenges.read();

                    if total_due == 0 {
                        rsx! {
                            div { class: "review-card empty-state",
                                h2 { "All caught up!" }
                                p { "No exercises are due for review right now." }
                                Link {
                                    class: "back-btn",
                                    to: Route::Landing {},
                                    "← Back to Home"
                                }
                            }
                        }
                    } else if current >= total_due {
                        rsx! {
                            div { class: "review-card empty-state",
                                h2 { "Review Complete!" }
                                p { "You reviewed {total_due} items." }
                                Link {
                                    class: "back-btn",
                                    to: Route::Landing {},
                                    "← Back to Home"
                                }
                            }
                        }
                    } else if let Some((exercise_id, challenge)) = challenges_read.get(current) {
                        let ex_id = exercise_id.clone();
                        let prompt = challenge.prompt.clone();
                        let sentence = challenge.sentence.clone();

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
                            div { class: "review-card",
                                if user_combo > 0 {
                                    div { class: "combo-row",
                                        ComboIndicator {
                                            combo: user_combo,
                                            multiplier: combo_result().multiplier,
                                            is_new_record: combo_result().is_new_record
                                        }
                                    }
                                }

                                div { class: "review-prompt", MixedText { content: prompt } }
                                div { class: "review-sentence", MixedText { content: sentence } }

                                {match &challenge.answer {
                                    AnswerType::FreeForm { .. } => rsx! {
                                        input {
                                            class: "{input_class}",
                                            r#type: "text",
                                            placeholder: "Enter your answer...",
                                            value: "{answer}",
                                            disabled: submitted(),
                                            oninput: move |e| answer.set(e.value()),
                                        }
                                    },
                                    AnswerType::MultipleChoice { options, correct_index } => {
                                        let correct_idx = *correct_index;
                                        let opts = options.clone();
                                        rsx! {
                                            div { class: "multiple-choice",
                                                for (i, option) in opts.iter().enumerate() {
                                                    {
                                                        let btn_class = if submitted() {
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

                                if let Some(result) = grade_result() {
                                    {
                                        let fb_class = if result.correct {
                                            "feedback-box feedback-correct"
                                        } else {
                                            "feedback-box feedback-incorrect"
                                        };
                                        rsx! {
                                            div { class: "{fb_class}", "{result.feedback}" }
                                        }
                                    }
                                }

                                if submitted() {
                                    {
                                        let is_correct = grade_result().map(|r| r.correct).unwrap_or(false);
                                        let ex_id_clone = ex_id.clone();
                                        rsx! {
                                            div { class: "srs-buttons",
                                                button {
                                                    class: "srs-btn hard",
                                                    onclick: move |_| {
                                                        record_srs(&mut progress, &ex_id_clone, ResponseQuality::CorrectDifficult);
                                                        advance_review(&mut current_index, &mut answer, &mut selected_choice, &mut submitted, &mut grade_result);
                                                    },
                                                    "Hard"
                                                }
                                                button {
                                                    class: "srs-btn good",
                                                    onclick: move |_| {
                                                        let quality = if is_correct { ResponseQuality::CorrectHesitation } else { ResponseQuality::Incorrect };
                                                        record_srs(&mut progress, &ex_id, quality);
                                                        advance_review(&mut current_index, &mut answer, &mut selected_choice, &mut submitted, &mut grade_result);
                                                    },
                                                    if is_correct { "Good" } else { "Again" }
                                                }
                                                if is_correct {
                                                    button {
                                                        class: "srs-btn easy",
                                                        onclick: {
                                                            let ex_id_easy = ex_id.clone();
                                                            move |_| {
                                                                record_srs(&mut progress, &ex_id_easy, ResponseQuality::Perfect);
                                                                advance_review(&mut current_index, &mut answer, &mut selected_choice, &mut submitted, &mut grade_result);
                                                            }
                                                        },
                                                        "Easy"
                                                    }
                                                }
                                            }
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
                                        let ex_id_submit = ex_id.clone();
                                        rsx! {
                                            div { class: "action-row",
                                                button {
                                                    class: "submit-btn",
                                                    disabled: !can_submit,
                                                    onclick: move |_| {
                                                        let is_correct = match &answer_clone {
                                                            AnswerType::FreeForm { golden_logic } => {
                                                                let result = check_answer(&answer.read(), golden_logic);
                                                                let correct = result.correct;
                                                                grade_result.set(Some(result));
                                                                correct
                                                            }
                                                            AnswerType::MultipleChoice { correct_index, .. } => {
                                                                let correct = selected_choice() == Some(*correct_index);
                                                                let result = if correct {
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
                                                                true
                                                            }
                                                        };

                                                        let is_first_try = !first_try_tracker.read().contains(&current);
                                                        first_try_tracker.write().insert(current);

                                                        {
                                                            let mut prog = progress.write();
                                                            prog.record_attempt(&ex_id_submit, is_correct);

                                                            let cr = update_combo(&mut prog, is_correct);
                                                            combo_result.set(cr.clone());

                                                            if is_correct {
                                                                play_sound(SoundEffect::Correct);

                                                                let rng_seed = (prog.xp + current as u64) % 100;
                                                                let reward = calculate_xp_reward(
                                                                    1,
                                                                    cr.new_combo,
                                                                    prog.streak_days,
                                                                    is_first_try,
                                                                    rng_seed,
                                                                );

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
                                                            } else {
                                                                play_sound(SoundEffect::Incorrect);
                                                            }
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
                            div { class: "review-card",
                                p { "Loading..." }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn record_srs(progress: &mut Signal<UserProgress>, exercise_id: &str, quality: ResponseQuality) {
    let today = get_today();
    let mut user_progress = progress.write();

    user_progress.record_attempt(exercise_id, quality.is_correct());

    if let Some(ep) = user_progress.exercises.get_mut(exercise_id) {
        sm2_update(&mut ep.srs, quality);
        ep.srs.next_review = Some(calculate_next_review(&today, ep.srs.interval));
    }

    user_progress.save();
}

fn advance_review(
    current_index: &mut Signal<usize>,
    answer: &mut Signal<String>,
    selected_choice: &mut Signal<Option<usize>>,
    submitted: &mut Signal<bool>,
    grade_result: &mut Signal<Option<GradeResult>>,
) {
    current_index.set(current_index() + 1);
    answer.set(String::new());
    selected_choice.set(None);
    submitted.set(false);
    grade_result.set(None);
}

fn get_today() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::prelude::*;

        #[wasm_bindgen]
        extern "C" {
            #[wasm_bindgen(js_namespace = Date, js_name = now)]
            fn date_now() -> f64;
        }

        let ms = date_now() as i64;
        let days_since_epoch = ms / 86400000;
        let year = 1970 + (days_since_epoch / 365) as i32;
        let day_of_year = (days_since_epoch % 365) as i32;
        let month = (day_of_year / 30).min(11) + 1;
        let day = (day_of_year % 30) + 1;
        format!("{:04}-{:02}-{:02}", year, month, day)
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        "2025-01-01".to_string()
    }
}
