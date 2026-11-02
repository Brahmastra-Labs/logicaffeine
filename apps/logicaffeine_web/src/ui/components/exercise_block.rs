//! "Now you try" exercise block for the Syntax Guide.
//!
//! The reader writes code (Imperative — graded by exact program output), a
//! page (Web — graded by the page's own `## Test` blocks) or FOL (Logic —
//! graded structurally by the Learn grader) and checks it in place. The reference solution never reaches the DOM; the guide's test
//! suite proves it grades correct before it can ship.
//!
//! Pedagogy surface: the kind badge names the exercise's rung on the
//! worked→completion→free ladder (Sweller/Renkl), hints disclose one at a
//! time on demand and never spoil (Bloom's corrective tutor), and every
//! graded result feeds the SM-2 deck so the exercise resurfaces before it
//! fades — a clean pass without hints schedules further out than a rescued
//! one.

use super::inline_editor::{InlineEditor, InlineEditorStyle};
use crate::grader::check_answer;
use crate::guide_review::record_exercise_result;
use crate::progress::UserProgress;
use crate::ui::pages::guide::content::{ExampleMode, ExerciseKind};
use dioxus::prelude::*;
use logicaffeine_compile::interpret_for_ui;

#[derive(Props, Clone, PartialEq)]
pub struct ExerciseBlockProps {
    pub id: String,
    pub prompt: String,
    pub mode: ExampleMode,
    pub expected: String,
    pub starter: String,
    pub kind: ExerciseKind,
    pub concepts: Vec<String>,
    pub hints: Vec<String>,
}

fn record_grade(progress_ctx: Option<Signal<UserProgress>>, exercise_id: &str, quality: u8) {
    let today = crate::ui::pages::guide::today_string();
    match progress_ctx {
        Some(mut progress) => {
            record_exercise_result(&mut progress.write(), exercise_id, quality, &today);
        }
        None => {
            let mut progress = UserProgress::load();
            record_exercise_result(&mut progress, exercise_id, quality, &today);
        }
    }
}

#[component]
pub fn ExerciseBlock(props: ExerciseBlockProps) -> Element {
    let mut attempt = use_signal(|| props.starter.clone());
    let mut feedback = use_signal(String::new);
    let mut passed = use_signal(|| false);
    let mut hints_shown = use_signal(|| 0usize);
    let mut fail_recorded = use_signal(|| false);

    let progress_ctx = try_consume_context::<Signal<UserProgress>>();

    let mode = props.mode;
    let expected = props.expected.clone();
    let exercise_id = props.id.clone();

    // Quality for the SM-2 deck: a clean unaided pass is Perfect, a hinted
    // pass is CorrectHesitation; the FIRST failure seeds the card at
    // Incorrect (later retries inside the same visit don't hammer the
    // schedule — the eventual pass sets it).
    let mut on_graded = move |correct: bool| {
        if correct {
            let quality = if *hints_shown.read() > 0 { 4 } else { 5 };
            record_grade(progress_ctx, &exercise_id, quality);
        } else if !*fail_recorded.read() {
            fail_recorded.set(true);
            record_grade(progress_ctx, &exercise_id, 1);
        }
    };

    let handle_check = move |_| {
        let current = attempt.read().clone();
        match mode {
            ExampleMode::Theorem => {
                // No theorem exercises exist yet; grading one would go through
                // compile_theorem_for_ui + verified.
                feedback.set("Theorem exercises are proved with largo prove.".to_string());
            }
            ExampleMode::Logic => {
                let grade = check_answer(&current, &expected);
                passed.set(grade.correct);
                feedback.set(if grade.correct {
                    "✓ Correct!".to_string()
                } else {
                    format!("Not yet — {}", grade.feedback)
                });
                on_graded(grade.correct);
            }
            ExampleMode::Web => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let verdict = crate::ui::pages::guide::page_verdict(&current);
                    let correct = verdict.is_ok();
                    passed.set(correct);
                    feedback.set(match verdict {
                        Ok(n) => format!("✓ Correct — {n} test(s) pass."),
                        Err(why) => format!("Not yet — {}", why.lines().next().unwrap_or("")),
                    });
                    on_graded(correct);
                }
                // The page test runner is not part of the browser build, so this build does not
                // grade — and records no grade, since nothing about the reader's answer is known.
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = &current;
                    feedback.set(
                        "A page is graded by running its `## Test` blocks, and the test runner is \
                         not part of the in-browser guide. Save the page in a project and run \
                         `largo test`."
                            .to_string(),
                    );
                }
            }
            ExampleMode::Imperative => {
                let expected = expected.clone();
                let mut on_graded = on_graded.clone();
                spawn(async move {
                    let r = interpret_for_ui(&current).await;
                    match r.error {
                        Some(e) => {
                            passed.set(false);
                            feedback.set(format!(
                                "Your program errored: {}",
                                e.lines().next().unwrap_or("")
                            ));
                            on_graded(false);
                        }
                        None => {
                            let out = r.lines.join("\n");
                            if out == expected {
                                passed.set(true);
                                feedback.set("✓ Correct!".to_string());
                                on_graded(true);
                            } else {
                                passed.set(false);
                                feedback.set(format!("Not yet — your output was:\n{out}"));
                                on_graded(false);
                            }
                        }
                    }
                });
            }
        }
    };

    let rows = attempt.read().lines().count().max(3) + 1;
    let kind_label = match props.kind {
        ExerciseKind::Completion => "Fill in the blank",
        ExerciseKind::Practice => "Practice",
        ExerciseKind::Challenge => "Challenge",
    };
    let shown = *hints_shown.read();
    let total_hints = props.hints.len();

    rsx! {
        if mode != ExampleMode::Logic {
            InlineEditorStyle {}
        }
        div {
            class: "guide-exercise",
            id: "{props.id}",
            style: "border: 1px solid rgba(96,165,250,0.25); border-radius: 12px; margin: 20px 0; background: rgba(96,165,250,0.05); overflow: hidden;",

            div {
                style: "padding: 12px 16px; font-size: 13px; font-weight: 600; color: rgba(229,231,235,0.9); border-bottom: 1px solid rgba(255,255,255,0.08);",
                span { style: "color: #60a5fa; margin-right: 8px;", "Now you try" }
                span {
                    class: "guide-exercise-kind",
                    style: "display: inline-block; margin-right: 8px; padding: 2px 8px; border-radius: 999px; font-size: 10px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.5px; background: rgba(96,165,250,0.15); color: #60a5fa; border: 1px solid rgba(96,165,250,0.3);",
                    "{kind_label}"
                }
                "{props.prompt}"
                if !props.concepts.is_empty() {
                    div {
                        class: "guide-exercise-concepts",
                        style: "margin-top: 6px; font-size: 11px; font-weight: 500; color: rgba(229,231,235,0.45);",
                        "practices: "
                        {props.concepts.join(" · ")}
                    }
                }
            }
            if mode == ExampleMode::Logic {
                textarea {
                    style: "width: 100%; padding: 14px 16px; background: transparent; border: none; color: rgba(229,231,235,0.95); font-family: ui-monospace, monospace; font-size: 14px; line-height: 1.6; resize: vertical; outline: none;",
                    rows: rows as i64,
                    value: "{attempt}",
                    oninput: move |evt| attempt.set(evt.value()),
                    spellcheck: "false",
                }
            } else {
                InlineEditor {
                    value: attempt.read().clone(),
                    on_change: move |v| attempt.set(v),
                    language: mode.language(),
                }
            }
            // Disclosed hints — one at a time, in order (orient, then narrow).
            if shown > 0 {
                div {
                    class: "guide-exercise-hints",
                    style: "padding: 8px 16px; border-top: 1px solid rgba(255,255,255,0.06); display: flex; flex-direction: column; gap: 6px;",
                    for hint in props.hints.iter().take(shown) {
                        div {
                            style: "font-size: 12px; color: #fbbf24; line-height: 1.5;",
                            "💡 {hint}"
                        }
                    }
                }
            }
            div {
                style: "display: flex; align-items: center; gap: 12px; padding: 10px 16px; border-top: 1px solid rgba(255,255,255,0.06);",
                button {
                    class: "guide-exercise-check",
                    style: "padding: 7px 14px; border-radius: 8px; border: none; background: linear-gradient(135deg, rgba(96,165,250,0.9), rgba(167,139,250,0.9)); color: #060814; font-size: 12px; font-weight: 700; cursor: pointer;",
                    onclick: handle_check,
                    "Check"
                }
                if total_hints > 0 && shown < total_hints {
                    button {
                        class: "guide-exercise-hint",
                        style: "padding: 7px 14px; border-radius: 8px; border: 1px solid rgba(251,191,36,0.4); background: none; color: #fbbf24; font-size: 12px; font-weight: 600; cursor: pointer;",
                        onclick: move |_| { let n = *hints_shown.read(); hints_shown.set(n + 1); },
                        if shown == 0 { "Hint ({total_hints})" } else { "Another hint ({shown}/{total_hints})" }
                    }
                }
                if !feedback.read().is_empty() {
                    span {
                        style: if *passed.read() { "color: #22c55e; font-size: 13px; white-space: pre-wrap;" } else { "color: #fbbf24; font-size: 13px; white-space: pre-wrap;" },
                        "{feedback}"
                    }
                }
            }
        }
    }
}
