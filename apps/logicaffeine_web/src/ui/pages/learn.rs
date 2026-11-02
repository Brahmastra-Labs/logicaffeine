//! The Learn Logic page: a roadmap through the curriculum, one lesson at a time.
//!
//! The curriculum (`assets/curriculum`, via [`ContentEngine::shared`]) is laid out as a route: four
//! eras, a stop for each module, a stop for each lesson. A lesson is a short teaching, two examples
//! and one question. Answering it right, or asking to see the answer after a try, lets the learner
//! move on; "More examples" and "More practice" are there for anyone who wants them. The rules live
//! in [`crate::learn_state`]; this file renders them and records progress to [`UserProgress`].
//!
//! # Layout
//!
//! - **Hero**: the page title and how many lessons are done
//! - **Lesson**: the open lesson, its examples and its question
//! - **Roadmap**: every era, module and lesson; beside the lesson on wide screens, after it on phones
//!
//! # Route
//!
//! Accessed via [`Route::Learn`](crate::ui::router::Route::Learn). `/learn#<module-id>` opens that
//! module's first lesson not yet done.

use crate::content::{ContentBlock, ContentEngine, ExerciseConfig, LessonExample, LessonQuestion};
use crate::generator::{AnswerType, Challenge, Generator};
use crate::grader::{check_answer, check_readings};
use crate::learn_state::{Journey, LessonQuiz, PracticeSession, QuizState};
use crate::progress::UserProgress;
use crate::struggle::StruggleDetector;
use crate::ui::components::footer::Footer;
use crate::ui::components::icon::{Icon, IconSize, IconVariant};
use crate::ui::components::learn_roadmap::LearnRoadmap;
use crate::ui::components::lexicon_gate::LexiconGate;
use crate::ui::components::main_nav::{ActivePage, MainNav};
use crate::ui::components::symbol_pad::SymbolPad;
use crate::ui::inline_markup::inline_markup_html;
use crate::ui::seo::{
    breadcrumb_schema, course_schema, organization_schema, pages as seo_pages, BreadcrumbItem,
    JsonLdMultiple, PageHead,
};
use dioxus::prelude::*;
#[cfg(all(feature = "split", target_arch = "wasm32"))]
use dioxus::wasm_split;
use rand::rngs::StdRng;
use rand::SeedableRng;

const LEARN_STYLE: &str = r#"
.learn-page {
    min-height: 100vh;
    color: var(--text-primary);
    background:
        radial-gradient(900px 480px at 85% -160px, rgba(var(--accent-secondary-rgb), 0.12), transparent 60%),
        linear-gradient(180deg, var(--bg-gradient-start), var(--bg-gradient-mid) 55%, var(--bg-gradient-end));
    font-family: var(--font-sans);
}

/* Hero */
.learn-hero {
    max-width: 1180px;
    margin: 0 auto;
    padding: 40px 16px 28px;
}

.learn-hero h1 {
    font-size: var(--font-display-md);
    font-weight: 800;
    letter-spacing: -0.02em;
    line-height: 1.1;
    margin: 0 0 12px;
}

.learn-intro {
    max-width: 60ch;
    margin: 0;
    font-size: var(--font-body-lg);
    line-height: 1.6;
    color: var(--text-secondary);
}

.learn-progress {
    display: flex;
    align-items: center;
    gap: var(--spacing-md);
    max-width: 420px;
    margin: 20px 0 0;
    font-size: var(--font-body-sm);
    color: var(--text-tertiary);
}

.learn-progress-bar {
    flex: 1;
    height: 4px;
    border-radius: 2px;
    background: rgba(255, 255, 255, 0.08);
    overflow: hidden;
}

.learn-progress-fill {
    height: 100%;
    background: var(--accent-secondary);
    border-radius: 2px;
}

/* Layout: the lesson first; on wide screens the roadmap sits to its left. */
.learn-layout {
    max-width: 1180px;
    margin: 0 auto;
    padding: 0 16px 80px;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 48px;
}

.learn-main {
    min-width: 0;
    max-width: 720px;
}

.learn-to-roadmap {
    display: inline-flex;
    align-items: center;
    min-height: 44px;
    margin-top: var(--spacing-xl);
    padding: 0 14px;
    border: 1px solid rgba(255, 255, 255, 0.14);
    border-radius: var(--radius-full);
    background: none;
    color: var(--text-secondary);
    font: inherit;
    font-size: var(--font-body-sm);
    cursor: pointer;
}

@media (min-width: 1024px) {
    .learn-hero {
        padding: 56px 24px 36px;
    }
    .learn-layout {
        grid-template-columns: 300px minmax(0, 1fr);
        grid-template-areas: "roadmap lesson";
        align-items: start;
        padding: 0 24px 96px;
    }
    .learn-main {
        grid-area: lesson;
    }
    .learn-roadmap {
        grid-area: roadmap;
        position: sticky;
        top: 112px;
        max-height: calc(100vh - 128px);
        overflow-y: auto;
        padding-right: var(--spacing-sm);
    }
    .learn-to-roadmap {
        display: none;
    }
}

/* A lesson */
/* Clear of the sticky site nav when a link or the roadmap scrolls a lesson into view. */
.lesson {
    scroll-margin-top: 112px;
}

.lesson-crumb {
    margin: 0 0 6px;
    font-size: var(--font-caption-md);
    color: var(--text-tertiary);
}

.lesson-title {
    margin: 0 0 20px;
    font-size: var(--font-heading-lg);
    font-weight: 800;
    line-height: 1.25;
    letter-spacing: -0.01em;
}

.lesson-number {
    margin-right: 0.4em;
    font-variant-numeric: tabular-nums;
    color: var(--accent-primary);
}

.lesson-teaching p {
    max-width: 65ch;
    margin: 0 0 14px;
    font-size: var(--font-body-lg);
    line-height: 1.7;
    color: var(--text-secondary);
}

.lesson-teaching strong {
    color: var(--text-primary);
}

.lesson-definition {
    margin: 0 0 14px;
    padding: 2px 0 2px 14px;
    border-left: 2px solid rgba(var(--accent-secondary-rgb), 0.55);
}

.lesson-definition p {
    margin: 0;
}

.lesson-definition .lesson-term {
    color: var(--text-primary);
    font-weight: 700;
}

.lesson-table {
    margin: 4px 0 18px;
}

.lesson-table-title {
    margin: 0 0 8px;
    font-weight: 700;
    color: var(--text-primary);
}

.lesson-table-row {
    display: grid;
    grid-template-columns: minmax(0, 11em) minmax(0, 1fr);
    gap: 4px 16px;
    padding: 8px 0;
    border-top: 1px solid rgba(255, 255, 255, 0.07);
}

.lesson-table-key {
    font-family: var(--font-mono);
    color: var(--accent-primary);
    overflow-wrap: anywhere;
}

.lesson-table-name {
    color: var(--text-primary);
    font-weight: 600;
}

.lesson-table-meaning {
    grid-column: 2;
    color: var(--text-secondary);
    font-size: var(--font-body-sm);
    line-height: 1.5;
}

.lesson-heading {
    margin: 32px 0 12px;
    font-size: var(--font-heading-sm);
    font-weight: 700;
}

.lesson-example {
    margin: 0 0 16px;
    padding: 4px 0 4px 16px;
    border-left: 2px solid rgba(var(--accent-primary-rgb), 0.4);
}

.lesson-example-sentence,
.lesson-example-title {
    margin: 0 0 8px;
    font-size: var(--font-body-lg);
    color: var(--text-primary);
}

.lesson-example-title {
    font-weight: 600;
}

.lesson-formula {
    display: block;
    font-family: var(--font-mono);
    font-size: 17px;
    line-height: 1.55;
    color: var(--accent-primary);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
}

.lesson-example-lines {
    margin: 0;
    padding-left: 1.1em;
    color: var(--text-secondary);
    line-height: 1.6;
}

.lesson-example-conclusion {
    margin: 6px 0 0;
    color: var(--text-primary);
    font-weight: 600;
    line-height: 1.5;
}

.lesson-example-note {
    max-width: 62ch;
    margin: 8px 0 0;
    font-size: var(--font-body-sm);
    line-height: 1.55;
    color: var(--text-tertiary);
}

.lesson-toggle {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-height: 44px;
    padding: 0 16px;
    border: 1px solid rgba(255, 255, 255, 0.14);
    border-radius: var(--radius-full);
    background: none;
    color: var(--text-secondary);
    font: inherit;
    font-size: var(--font-body-sm);
    cursor: pointer;
}

/* A chevron marks the toggles that open and close something; Skip is a plain button. */
.lesson-toggle[aria-expanded]::after {
    content: "";
    width: 7px;
    height: 7px;
    border-right: 2px solid currentColor;
    border-bottom: 2px solid currentColor;
    transform: rotate(45deg) translateY(-2px);
}

.lesson-toggle[aria-expanded="true"]::after {
    transform: rotate(-135deg) translateY(-1px);
}

.lesson-toggle:hover {
    color: var(--text-primary);
    border-color: rgba(255, 255, 255, 0.26);
}

.lesson-extra {
    margin-top: 16px;
}

/* The question: the one block that asks something of the learner. */
.lesson-question {
    margin-top: 36px;
    padding: 20px 16px;
    border-radius: var(--radius-lg);
    background: rgba(var(--accent-secondary-rgb), 0.06);
    border: 1px solid rgba(var(--accent-secondary-rgb), 0.24);
}

.lesson-question .lesson-heading {
    margin-top: 0;
}

.lesson-question-prompt {
    margin: 0 0 6px;
    color: var(--text-secondary);
    line-height: 1.55;
}

.lesson-question-sentence {
    margin: 0 0 14px;
    font-size: 20px;
    font-weight: 600;
    line-height: 1.4;
    color: var(--text-primary);
}

.lesson-answer-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--spacing-sm);
}

.lesson-input {
    flex: 1 1 220px;
    min-width: 0;
    height: 48px;
    padding: 0 14px;
    border: 1px solid rgba(255, 255, 255, 0.18);
    border-radius: var(--radius-md);
    background: rgba(255, 255, 255, 0.05);
    color: var(--text-primary);
    font-family: var(--font-mono);
    font-size: 17px;
}

.lesson-input:focus {
    outline: 2px solid var(--accent-primary);
    outline-offset: 1px;
}

.lesson-input.correct {
    border-color: var(--color-success);
}

.lesson-input.wrong {
    border-color: var(--color-error);
}

.lesson-check,
.lesson-next {
    min-height: 48px;
    padding: 0 22px;
    border: none;
    border-radius: var(--radius-md);
    background: linear-gradient(135deg, var(--accent-primary), var(--accent-secondary));
    color: var(--bg-gradient-start);
    font: inherit;
    font-weight: 700;
    cursor: pointer;
}

.lesson-check:disabled {
    opacity: 0.5;
    cursor: default;
}

.lesson-check:focus-visible,
.lesson-next:focus-visible,
.lesson-toggle:focus-visible,
.lesson-option:focus-visible,
.lesson-show-answer:focus-visible,
.learn-to-roadmap:focus-visible {
    outline: 2px solid var(--accent-primary);
    outline-offset: 2px;
}

.lesson-options {
    display: grid;
    gap: var(--spacing-sm);
}

.lesson-option {
    min-height: 48px;
    padding: 12px 16px;
    border: 1px solid rgba(255, 255, 255, 0.14);
    border-radius: var(--radius-md);
    background: rgba(255, 255, 255, 0.03);
    color: var(--text-secondary);
    font: inherit;
    text-align: left;
    line-height: 1.45;
    cursor: pointer;
}

.lesson-option:hover:not(:disabled) {
    border-color: rgba(var(--accent-primary-rgb), 0.5);
    color: var(--text-primary);
}

.lesson-option:disabled {
    cursor: default;
}

.lesson-option.wrong {
    border-color: var(--color-error);
    color: var(--color-error);
}

.lesson-option.correct {
    border-color: var(--color-success);
    color: var(--color-success);
}

.lesson-verdict {
    margin: 14px 0 0;
    font-weight: 600;
}

.lesson-verdict.correct {
    color: var(--color-success);
}

.lesson-verdict.wrong {
    color: var(--color-error);
}

.lesson-hint {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: 10px;
    align-items: start;
    margin-top: 12px;
    padding: 12px 14px;
    border-left: 3px solid var(--accent-secondary);
    border-radius: 0 var(--radius-md) var(--radius-md) 0;
    background: rgba(var(--accent-secondary-rgb), 0.08);
    color: var(--text-secondary);
    line-height: 1.6;
}

.lesson-hint p {
    margin: 0;
}

.lesson-hint p + p {
    margin-top: 6px;
}

.lesson-show-answer {
    min-height: 44px;
    margin-top: 12px;
    padding: 0 16px;
    border: 1px solid rgba(255, 255, 255, 0.18);
    border-radius: var(--radius-md);
    background: none;
    color: var(--text-secondary);
    font: inherit;
    cursor: pointer;
}

.lesson-answer {
    margin-top: 16px;
    padding-top: 14px;
    border-top: 1px solid rgba(255, 255, 255, 0.08);
}

.lesson-answer-label {
    margin: 0 0 6px;
    font-size: var(--font-caption-md);
    color: var(--text-tertiary);
}

.lesson-answer-option {
    margin: 0;
    color: var(--text-primary);
    font-weight: 600;
}

.lesson-explanation {
    margin: 10px 0 0;
    color: var(--text-secondary);
    line-height: 1.6;
}

.lesson-next {
    margin-top: 16px;
}

.lesson-practice {
    margin-top: 28px;
}

/* Practice */
.practice {
    margin-top: 16px;
}

.practice-progress {
    margin: 0 0 10px;
    font-size: var(--font-caption-md);
    color: var(--text-tertiary);
}

.practice-card {
    padding: 18px 16px;
    border-radius: var(--radius-lg);
    border: 1px solid rgba(255, 255, 255, 0.1);
    background: rgba(255, 255, 255, 0.02);
}

.practice-prompt {
    margin: 0 0 6px;
    color: var(--text-tertiary);
    font-size: var(--font-body-sm);
}

.practice-sentence {
    margin: 0 0 14px;
    font-size: var(--font-body-lg);
    font-weight: 600;
    line-height: 1.45;
}

.practice-options {
    display: grid;
    gap: var(--spacing-sm);
}

.practice-readings {
    display: grid;
    gap: var(--spacing-sm);
}

.practice-readings label {
    font-size: var(--font-caption-md);
    color: var(--text-tertiary);
}

.practice-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--spacing-sm);
    margin-top: 14px;
}

.practice-key {
    display: block;
    margin: 6px 0;
    font-family: var(--font-mono);
    color: var(--accent-primary);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
}

@media (min-width: 640px) {
    .lesson-question {
        padding: 24px;
    }
    .practice-card {
        padding: 20px 22px;
    }
}

@media (prefers-reduced-motion: reduce) {
    .learn-page * {
        transition: none !important;
        animation: none !important;
    }
}
"#;

// ---------------------------------------------------------------------------------------
// Browser glue: scrolling and the URL fragment. `<body>` is the scroll container site-wide,
// so nothing here reads or writes `window.scrollY`.
// ---------------------------------------------------------------------------------------

/// Bring the element with `id` to the top of the page (under the nav), waiting for it to exist.
fn scroll_to(id: &'static str) {
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(async move {
        for _ in 0..30 {
            gloo_timers::future::TimeoutFuture::new(16).await;
            let Some(window) = web_sys::window() else {
                return;
            };
            let Some(element) = window.document().and_then(|d| d.get_element_by_id(id)) else {
                continue;
            };
            let reduced = window
                .match_media("(prefers-reduced-motion: reduce)")
                .ok()
                .flatten()
                .is_some_and(|m| m.matches());
            let options = web_sys::ScrollIntoViewOptions::new();
            options.set_behavior(if reduced {
                web_sys::ScrollBehavior::Instant
            } else {
                web_sys::ScrollBehavior::Smooth
            });
            options.set_block(web_sys::ScrollLogicalPosition::Start);
            element.scroll_into_view_with_scroll_into_view_options(&options);
            return;
        }
    });
    #[cfg(not(target_arch = "wasm32"))]
    let _ = id;
}

/// Record the open lesson's module in the address bar without adding a history entry.
fn set_fragment(module: &str) {
    #[cfg(target_arch = "wasm32")]
    if let Some(history) = web_sys::window().and_then(|w| w.history().ok()) {
        let _ = history.replace_state_with_url(
            &wasm_bindgen::JsValue::NULL,
            "",
            Some(&format!("#{module}")),
        );
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = module;
}

fn new_seed() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() as u64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        42
    }
}

/// Every lesson id, in roadmap order.
fn lesson_order() -> Vec<String> {
    ContentEngine::shared()
        .lessons()
        .iter()
        .map(|l| l.section.id.clone())
        .collect()
}

/// Change where the learner stands on the roadmap, and save it.
fn steer(mut progress: Signal<UserProgress>, change: impl FnOnce(&mut Journey)) {
    let mut journey = {
        let p = progress.peek();
        Journey::new(
            lesson_order(),
            p.lessons.iter().cloned(),
            p.open_lesson.as_deref(),
        )
    };
    change(&mut journey);
    let mut p = progress.write();
    p.lessons = journey.done_ids().map(str::to_string).collect();
    p.open_lesson = journey.open().map(str::to_string);
    p.save();
}

/// The module a lesson belongs to.
fn module_of(lesson: &str) -> Option<String> {
    ContentEngine::shared()
        .lesson(lesson)
        .map(|l| l.module.meta.id.clone())
}

// ---------------------------------------------------------------------------------------
// The page
// ---------------------------------------------------------------------------------------

#[component(lazy)]
pub fn Learn(module: String) -> Element {
    let progress = use_context_provider(|| Signal::new(UserProgress::load()));

    // `/learn#<module-id>` opens that module at its first lesson not done, unless the lesson
    // already open belongs to it; on arrival and on any later link to one.
    use_effect(use_reactive!(|module| {
        let Some(found) = ContentEngine::shared()
            .eras()
            .iter()
            .flat_map(|e| e.modules.iter())
            .find(|m| m.meta.id == module)
        else {
            return;
        };
        let ids: Vec<&str> = found.sections.iter().map(|s| s.id.as_str()).collect();
        steer(progress, |journey| {
            if !journey.open().is_some_and(|open| ids.contains(&open)) {
                journey.open_first_unfinished(&ids);
            }
        });
        scroll_to("lesson");
    }));

    let journey = {
        let p = progress.read();
        Journey::new(
            lesson_order(),
            p.lessons.iter().cloned(),
            p.open_lesson.as_deref(),
        )
    };
    let open = journey.open().map(str::to_string);
    let done: Vec<String> = journey.done_ids().map(str::to_string).collect();
    let (finished, total) = (journey.done_count(), journey.len());
    let percent = if total == 0 {
        0
    } else {
        finished * 100 / total
    };
    let count = if journey.is_finished() {
        format!("All {total} lessons done")
    } else {
        format!("{finished} of {total} lessons done")
    };

    let breadcrumbs = vec![
        BreadcrumbItem {
            name: "Home",
            path: "/",
        },
        BreadcrumbItem {
            name: "Learn",
            path: "/learn",
        },
    ];
    let schemas = vec![
        organization_schema(),
        course_schema(),
        breadcrumb_schema(&breadcrumbs),
    ];

    rsx! {
        PageHead {
            title: seo_pages::LEARN.title,
            description: seo_pages::LEARN.description,
            canonical_path: seo_pages::LEARN.canonical_path,
        }
        style { "{LEARN_STYLE}" }
        JsonLdMultiple { schemas }

        div { class: "learn-page",
            MainNav { active: ActivePage::Learn, subtitle: Some("Master formal reasoning") }

            header { class: "learn-hero",
                h1 { "Learn Logic" }
                p { class: "learn-intro",
                    "Learn to read and write formal logic, one short lesson at a time. Each lesson shows two examples and asks one question. Answer it to move on."
                }
                p { class: "learn-progress",
                    span { "{count}" }
                    span { class: "learn-progress-bar", aria_hidden: "true",
                        span { class: "learn-progress-fill", style: "width: {percent}%" }
                    }
                }
            }

            div { class: "learn-layout",
                main { class: "learn-main",
                    if let Some(id) = open.clone() {
                        LessonPanel {
                            key: "{id}",
                            lesson: id.clone(),
                            done: journey.is_done(&id),
                            on_next: move |_| {
                                steer(progress, |journey| {
                                    journey.finish();
                                    journey.advance();
                                });
                                if let Some(module) = progress.peek().open_lesson.as_deref().and_then(module_of) {
                                    set_fragment(&module);
                                }
                                scroll_to("lesson");
                            },
                        }
                    }
                    button {
                        class: "learn-to-roadmap",
                        r#type: "button",
                        onclick: move |_| scroll_to("roadmap"),
                        "See the whole roadmap"
                    }
                }
                LearnRoadmap {
                    open,
                    done,
                    on_open: move |id: String| {
                        steer(progress, |journey| {
                            journey.open_lesson(&id);
                        });
                        if let Some(module) = module_of(&id) {
                            set_fragment(&module);
                        }
                        scroll_to("lesson");
                    },
                }
            }

            Footer {}
        }
    }
}

// ---------------------------------------------------------------------------------------
// One lesson: its teaching, two examples, the question, and the extras behind their toggles
// ---------------------------------------------------------------------------------------

/// A lesson as the page draws it. `on_next` moves the learner on once its question is settled.
#[component]
pub fn LessonPanel(lesson: String, done: bool, on_next: EventHandler<()>) -> Element {
    let mut more_examples = use_signal(|| false);
    let mut more_practice = use_signal(|| false);
    let engine = ContentEngine::shared();
    let Some(here) = engine.lesson(&lesson) else {
        return rsx! { p { class: "lesson-missing", "This lesson is not on the roadmap." } };
    };
    let (section, module) = (here.section, here.module);
    let position = module
        .sections
        .iter()
        .position(|s| s.id == section.id)
        .unwrap_or(0)
        + 1;
    let last = engine
        .lessons()
        .last()
        .is_some_and(|l| l.section.id == section.id);
    let practice = module
        .exercises
        .iter()
        .filter(|e| e.section.as_deref() == Some(section.id.as_str()))
        .count();
    let extra = section.examples.get(2..).unwrap_or_default();
    let finished = if done { ", done" } else { "" };

    rsx! {
        article { class: "lesson", id: "lesson", "data-lesson": "{section.id}", aria_labelledby: "lesson-title",
            p { class: "lesson-crumb", "{module.meta.title}, lesson {position} of {module.sections.len()}{finished}" }
            h2 { class: "lesson-title", id: "lesson-title",
                span { class: "lesson-number", "{section.id}" }
                "{section.title}"
            }

            div { class: "lesson-teaching",
                for (i, block) in section.content.iter().enumerate() {
                    {teaching_block(i, block)}
                }
            }

            section { class: "lesson-examples", aria_labelledby: "lesson-examples-title",
                h3 { class: "lesson-heading", id: "lesson-examples-title", "Examples" }
                for (i, example) in section.examples.iter().take(2).enumerate() {
                    {example_view(i, example)}
                }
                if !extra.is_empty() {
                    button {
                        class: "lesson-toggle lesson-more-examples",
                        r#type: "button",
                        aria_expanded: "{more_examples}",
                        aria_controls: "lesson-extra-examples",
                        onclick: move |_| more_examples.toggle(),
                        "More examples ({extra.len()})"
                    }
                    div { class: "lesson-extra", id: "lesson-extra-examples", hidden: !more_examples(),
                        if more_examples() {
                            for (i, example) in extra.iter().enumerate() {
                                {example_view(i + 2, example)}
                            }
                        }
                    }
                }
            }

            LessonQuestionView { lesson: section.id.clone(), last, on_next }

            if practice > 0 {
                div { class: "lesson-practice",
                    button {
                        class: "lesson-toggle lesson-more-practice",
                        r#type: "button",
                        aria_expanded: "{more_practice}",
                        aria_controls: "lesson-practice",
                        onclick: move |_| more_practice.toggle(),
                        "More practice ({practice})"
                    }
                    div { id: "lesson-practice", hidden: !more_practice(),
                        if more_practice() {
                            LexiconGate {
                                PracticeArea { lesson: section.id.clone() }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// One block of a lesson's teaching.
fn teaching_block(i: usize, block: &'static ContentBlock) -> Element {
    match block {
        ContentBlock::Paragraph { text } => rsx! {
            p { key: "{i}", class: "lesson-paragraph", dangerous_inner_html: inline_markup_html(text) }
        },
        ContentBlock::Definition { term, definition } => rsx! {
            div { key: "{i}", class: "lesson-definition",
                p { span { class: "lesson-term", "{term}" } }
                p { "{definition}" }
            }
        },
        ContentBlock::Symbols { title, symbols } => rsx! {
            div { key: "{i}", class: "lesson-table",
                p { class: "lesson-table-title", "{title}" }
                for (j, row) in symbols.iter().enumerate() {
                    div { key: "{j}", class: "lesson-table-row",
                        span { class: "lesson-table-key", "{row.symbol}" }
                        span { class: "lesson-table-name", "{row.name}" }
                        span { class: "lesson-table-meaning", "{row.meaning}" }
                    }
                }
            }
        },
    }
}

/// One example: a sentence and the formula LOGOS prints for it, or a worked case.
fn example_view(i: usize, example: &'static LessonExample) -> Element {
    match example {
        LessonExample::Formula {
            sentence,
            formula,
            note,
        } => rsx! {
            div { key: "{i}", class: "lesson-example",
                p { class: "lesson-example-sentence", "{sentence}" }
                code { class: "lesson-formula", "{formula}" }
                if let Some(note) = note {
                    p { class: "lesson-example-note", "{note}" }
                }
            }
        },
        LessonExample::Worked {
            title,
            lines,
            conclusion,
            note,
        } => rsx! {
            div { key: "{i}", class: "lesson-example worked",
                p { class: "lesson-example-title", "{title}" }
                ul { class: "lesson-example-lines",
                    for (j, line) in lines.iter().enumerate() {
                        li { key: "{j}", "{line}" }
                    }
                }
                if let Some(conclusion) = conclusion {
                    p { class: "lesson-example-conclusion", "{conclusion}" }
                }
                if let Some(note) = note {
                    p { class: "lesson-example-note", "{note}" }
                }
            }
        },
    }
}

// ---------------------------------------------------------------------------------------
// The lesson's question
// ---------------------------------------------------------------------------------------

#[component]
fn LessonQuestionView(lesson: String, last: bool, on_next: EventHandler<()>) -> Element {
    let mut quiz = use_signal(LessonQuiz::default);
    let mut answer = use_signal(String::new);
    let mut tried = use_signal(Vec::<usize>::new);
    let mut verdict = use_signal(|| None::<String>);
    let mut struggle = use_signal(StruggleDetector::default);
    let Some(question) = ContentEngine::shared()
        .lesson(&lesson)
        .and_then(|l| l.section.question.as_ref())
    else {
        return rsx! {};
    };

    let mut grade = move |correct: bool, why: Option<String>| {
        quiz.write().answer(correct);
        if correct {
            struggle.write().record_correct_attempt();
            verdict.set(None);
        } else {
            struggle.write().record_wrong_attempt();
            verdict.set(why);
        }
    };

    let state = quiz.read().state();
    let settled = quiz.read().is_settled();
    let wrong = quiz.read().wrong_attempts();
    let (hint, explanation, kind) = match question {
        LessonQuestion::Translate {
            hint, explanation, ..
        } => (hint, explanation, "typed"),
        LessonQuestion::Choice {
            hint, explanation, ..
        } => (hint, explanation, "choice"),
    };

    rsx! {
        section { class: "lesson-question", "data-kind": kind, aria_labelledby: "lesson-question-title",
            h3 { class: "lesson-heading", id: "lesson-question-title", "Your turn" }
            match question {
                LessonQuestion::Translate { sentence, answer: key, .. } => {
                    let key = key.clone();
                    let check = move || {
                        if quiz.peek().is_settled() {
                            return;
                        }
                        let given = answer.peek().clone();
                        if given.trim().is_empty() {
                            verdict.set(Some("Write a formula first.".to_string()));
                            return;
                        }
                        let result = check_answer(&given, &key);
                        grade(result.correct, Some(result.feedback));
                    };
                    let field = match state {
                        QuizState::Correct => "lesson-input correct",
                        QuizState::Answering if wrong > 0 => "lesson-input wrong",
                        _ => "lesson-input",
                    };
                    rsx! {
                        p { class: "lesson-question-prompt", "Write this sentence in logic." }
                        p { class: "lesson-question-sentence", "{sentence}" }
                        div { class: "lesson-answer-row",
                            input {
                                id: "lesson-answer",
                                class: "{field}",
                                r#type: "text",
                                aria_label: "Your formula",
                                autocomplete: "off",
                                autocapitalize: "off",
                                spellcheck: "false",
                                placeholder: "Type, or tap the symbols below",
                                readonly: settled,
                                value: "{answer}",
                                oninput: move |e: Event<FormData>| {
                                    answer.set(e.value());
                                    struggle.write().record_activity();
                                },
                                onkeydown: {
                                    let mut check = check.clone();
                                    move |e: Event<KeyboardData>| {
                                        if e.key() == Key::Enter {
                                            e.prevent_default();
                                            check();
                                        }
                                    }
                                },
                            }
                            button {
                                class: "lesson-check",
                                r#type: "button",
                                disabled: settled,
                                onclick: {
                                    let mut check = check.clone();
                                    move |_| check()
                                },
                                "Check"
                            }
                        }
                        if !settled {
                            SymbolPad { target: "lesson-answer".to_string(), on_input: move |text: String| answer.set(text) }
                        }
                    }
                }
                LessonQuestion::Choice { prompt, options, correct, .. } => {
                    let right = *correct;
                    rsx! {
                        p { class: "lesson-question-prompt", "{prompt}" }
                        div { class: "lesson-options", role: "group", aria_label: "Options",
                            for (i, option) in options.iter().enumerate() {
                                {
                                    let missed = tried.read().contains(&i) && i != right;
                                    let class = if missed {
                                        "lesson-option wrong"
                                    } else if settled && i == right {
                                        "lesson-option correct"
                                    } else {
                                        "lesson-option"
                                    };
                                    rsx! {
                                        button {
                                            key: "{i}",
                                            class: "{class}",
                                            r#type: "button",
                                            disabled: settled || missed,
                                            onclick: move |_| {
                                                if quiz.peek().is_settled() {
                                                    return;
                                                }
                                                tried.write().push(i);
                                                grade(i == right, Some("Not quite.".to_string()));
                                            },
                                            "{option}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            div { class: "lesson-feedback", role: "status", aria_live: "polite",
                if state == QuizState::Correct {
                    p { class: "lesson-verdict correct", "Correct." }
                } else if let Some(why) = verdict() {
                    p { class: "lesson-verdict wrong", "{why}" }
                }
                if state == QuizState::Answering && struggle.read().should_show_hints() {
                    SocraticHint { hint: hint.clone() }
                }
            }

            if quiz.read().can_show_answer() {
                button {
                    class: "lesson-show-answer",
                    r#type: "button",
                    onclick: move |_| {
                        quiz.write().show_answer();
                    },
                    "Show the answer"
                }
            }

            if settled {
                div { class: "lesson-answer",
                    p { class: "lesson-answer-label", "The answer" }
                    match question {
                        LessonQuestion::Translate { answer: key, .. } => rsx! { code { class: "lesson-formula", "{key}" } },
                        LessonQuestion::Choice { options, correct, .. } => rsx! {
                            p { class: "lesson-answer-option", "{options[*correct]}" }
                        },
                    }
                    p { class: "lesson-explanation", "{explanation}" }
                }
                button {
                    class: "lesson-next",
                    r#type: "button",
                    onclick: move |_| on_next.call(()),
                    if last { "Finish the roadmap" } else { "Next lesson" }
                }
            }
        }
    }
}

/// The hint a learner gets once the struggle detector sees them stuck: a question that points the
/// way without giving the answer.
#[component]
fn SocraticHint(hint: String) -> Element {
    rsx! {
        div { class: "lesson-hint",
            span { class: "lesson-hint-mark", aria_hidden: "true",
                Icon { variant: IconVariant::Owl, size: IconSize::Medium, color: "var(--accent-secondary)" }
            }
            p { "{hint}" }
        }
    }
}

// ---------------------------------------------------------------------------------------
// More practice: the lesson's exercises, one at a time (behind the word bank)
// ---------------------------------------------------------------------------------------

/// The exercises that practise `lesson`, in the order they play.
fn practice_set(lesson: &str) -> Vec<&'static ExerciseConfig> {
    ContentEngine::shared()
        .lesson(lesson)
        .map(|l| {
            l.module
                .exercises
                .iter()
                .filter(|e| e.section.as_deref() == Some(lesson))
                .collect()
        })
        .unwrap_or_default()
}

#[component]
fn PracticeArea(lesson: String) -> Element {
    let set = practice_set(&lesson);
    let fallback = use_signal(UserProgress::new);
    let progress = try_consume_context::<Signal<UserProgress>>().unwrap_or(fallback);
    let total = set.len();
    let practice = use_signal(|| {
        let p = progress.peek();
        let solved = set
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                p.get_exercise_progress(&e.id)
                    .is_some_and(|x| x.correct_count > 0)
            })
            .map(|(i, _)| i);
        PracticeSession::new(total, solved.collect::<Vec<_>>())
    });
    let seed = use_hook(new_seed);

    let session = practice.read();
    let Some(index) = session.current() else {
        return rsx! { p { class: "practice-progress", "No practice questions for this lesson yet." } };
    };
    let completed = session.completed_count();
    rsx! {
        div { class: "practice",
            p { class: "practice-progress", "Question {index + 1} of {total}. {completed} answered right." }
            // A keyed list of one: a new visit remounts the card, so it opens with a blank answer.
            for visit in std::iter::once(session.visit()) {
                PracticeCard { key: "{visit}", lesson: lesson.clone(), index, practice, progress, seed }
            }
        }
    }
}

fn generate(exercise: &ExerciseConfig, seed: u64, index: usize) -> Option<Challenge> {
    let mut rng = StdRng::seed_from_u64(seed.wrapping_add((index as u64).wrapping_mul(1000)));
    Generator::new().generate(exercise, &mut rng)
}

/// Which of a practice question's aids are showing.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Reveal {
    hint: bool,
    answer: bool,
}

#[component]
fn PracticeCard(
    lesson: String,
    index: usize,
    mut practice: Signal<PracticeSession>,
    mut progress: Signal<UserProgress>,
    seed: u64,
) -> Element {
    let config = practice_set(&lesson).get(index).copied();
    let challenge = use_hook(|| config.and_then(|config| generate(config, seed, index)));
    let mut answer = use_signal(String::new);
    let readings_count = match challenge.as_ref().map(|c| &c.answer) {
        Some(AnswerType::Ambiguity { readings }) => readings.len(),
        _ => 0,
    };
    let mut readings = use_signal(|| vec![String::new(); readings_count]);
    let mut reading_focus = use_signal(|| 0usize);
    let mut selected = use_signal(|| None::<usize>);
    let mut feedback = use_signal(|| None::<(bool, String)>);
    let mut reveal = use_signal(Reveal::default);
    let mut struggle = use_signal(StruggleDetector::default);

    let (Some(config), Some(challenge)) = (config, challenge) else {
        return rsx! {
            div { class: "practice-card",
                p { "This question could not be made." }
                button { class: "lesson-toggle practice-skip", r#type: "button", onclick: move |_| practice.write().advance(), "Skip" }
            }
        };
    };
    let exercise_id: &'static str = config.id.as_str();

    let is_correct = {
        let challenge = challenge.clone();
        move || -> (bool, String) {
            match &challenge.answer {
                AnswerType::FreeForm { golden_logic } => {
                    let given = answer.peek().clone();
                    if given.trim().is_empty() {
                        return (false, "Write a formula first.".to_string());
                    }
                    let result = check_answer(&given, golden_logic);
                    (result.correct, result.feedback)
                }
                AnswerType::MultipleChoice { correct_index, .. } => (
                    *selected.peek() == Some(*correct_index),
                    "Not quite.".to_string(),
                ),
                AnswerType::Ambiguity { readings: expected } => {
                    let grade = check_readings(&readings.peek(), expected);
                    (
                        grade.all_correct(),
                        format!("{} of {} readings right.", grade.matched, grade.total),
                    )
                }
            }
        }
    };
    let check = move || {
        let (correct, why) = is_correct();
        practice.write().answer(correct);
        progress.write().record_attempt(exercise_id, correct);
        if correct {
            struggle.write().record_correct_attempt();
            feedback.set(Some((true, "Correct.".to_string())));
        } else {
            struggle.write().record_wrong_attempt();
            feedback.set(Some((false, why)));
        }
    };
    let solved = practice.read().solved_current();
    let shown = reveal();
    let field = match feedback() {
        Some((true, _)) => "lesson-input correct",
        Some((false, _)) => "lesson-input wrong",
        None => "lesson-input",
    };

    rsx! {
        div { class: "practice-card",
            if challenge.prompt.trim() != challenge.sentence.trim() {
                p { class: "practice-prompt", "{challenge.prompt}" }
            }
            p { class: "practice-sentence", "{challenge.sentence}" }

            match &challenge.answer {
                AnswerType::FreeForm { .. } => rsx! {
                    div { class: "lesson-answer-row",
                        input {
                            id: "practice-answer",
                            class: "{field}",
                            r#type: "text",
                            aria_label: "Your formula",
                            autocomplete: "off",
                            autocapitalize: "off",
                            spellcheck: "false",
                            value: "{answer}",
                            oninput: move |e: Event<FormData>| {
                                answer.set(e.value());
                                struggle.write().record_activity();
                            },
                            onkeydown: {
                                let mut check = check.clone();
                                move |e: Event<KeyboardData>| {
                                    if e.key() == Key::Enter {
                                        e.prevent_default();
                                        check();
                                    }
                                }
                            },
                        }
                        button { class: "lesson-check", r#type: "button", onclick: { let mut check = check.clone(); move |_| check() }, "Check" }
                    }
                    SymbolPad { target: "practice-answer".to_string(), on_input: move |text: String| answer.set(text) }
                },
                AnswerType::MultipleChoice { options, correct_index } => {
                    let right = *correct_index;
                    let answered = feedback().is_some();
                    rsx! {
                        div { class: "practice-options", role: "group", aria_label: "Options",
                            for (i, option) in options.iter().enumerate() {
                                {
                                    let chosen = selected() == Some(i);
                                    let class = match (answered && chosen, i == right) {
                                        (true, true) => "lesson-option correct",
                                        (true, false) => "lesson-option wrong",
                                        _ => "lesson-option",
                                    };
                                    let mut check = check.clone();
                                    rsx! {
                                        button {
                                            key: "{i}",
                                            class: "{class}",
                                            r#type: "button",
                                            aria_pressed: "{chosen}",
                                            onclick: move |_| {
                                                selected.set(Some(i));
                                                check();
                                            },
                                            "{option}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
                AnswerType::Ambiguity { readings: expected } => {
                    let n = expected.len();
                    let target = format!("practice-reading-{}", reading_focus());
                    rsx! {
                        div { class: "practice-readings", "data-answer-kind": "readings",
                            p { class: "practice-prompt", "Give all {n} readings, in any order." }
                            for i in 0..n {
                                label { key: "{i}", r#for: "practice-reading-{i}", "Reading {i + 1}" }
                                input {
                                    id: "practice-reading-{i}",
                                    class: "{field}",
                                    r#type: "text",
                                    "data-reading": "{i}",
                                    autocomplete: "off",
                                    autocapitalize: "off",
                                    spellcheck: "false",
                                    value: "{readings.read().get(i).cloned().unwrap_or_default()}",
                                    onfocus: move |_| reading_focus.set(i),
                                    oninput: move |e: Event<FormData>| {
                                        if let Some(slot) = readings.write().get_mut(i) {
                                            *slot = e.value();
                                        }
                                        struggle.write().record_activity();
                                    },
                                }
                            }
                        }
                        SymbolPad {
                            target,
                            on_input: move |text: String| {
                                let at = *reading_focus.peek();
                                if let Some(slot) = readings.write().get_mut(at) {
                                    *slot = text;
                                }
                            },
                        }
                        div { class: "practice-actions",
                            button { class: "lesson-check", r#type: "button", onclick: { let mut check = check.clone(); move |_| check() }, "Check" }
                        }
                    }
                },
            }

            div { role: "status", aria_live: "polite",
                if let Some((correct, message)) = feedback() {
                    p { class: if correct { "lesson-verdict correct" } else { "lesson-verdict wrong" }, "{message}" }
                    if !correct && struggle.read().should_show_hints() {
                        if let Some(hint) = challenge.hint.clone() {
                            SocraticHint { hint }
                        }
                    }
                }
            }

            div { class: "practice-actions",
                if challenge.hint.is_some() && feedback().is_none() {
                    button {
                        class: "lesson-toggle practice-hint",
                        r#type: "button",
                        aria_expanded: "{shown.hint}",
                        onclick: move |_| reveal.write().hint ^= true,
                        "Hint"
                    }
                }
                button {
                    class: "lesson-toggle practice-show",
                    r#type: "button",
                    aria_expanded: "{shown.answer}",
                    onclick: move |_| {
                        if !reveal.peek().answer {
                            practice.write().reveal_answer();
                        }
                        reveal.write().answer ^= true;
                    },
                    "Show the answer"
                }
                if solved {
                    button { class: "lesson-check practice-next", r#type: "button", onclick: move |_| practice.write().advance(), "Next question" }
                } else {
                    button { class: "lesson-toggle practice-skip", r#type: "button", onclick: move |_| practice.write().advance(), "Skip" }
                }
            }

            if shown.hint && feedback().is_none() {
                if let Some(hint) = challenge.hint.clone() {
                    SocraticHint { hint }
                }
            }
            if shown.answer {
                div { class: "lesson-answer",
                    p { class: "lesson-answer-label", "The answer" }
                    match &challenge.answer {
                        AnswerType::FreeForm { golden_logic } => rsx! { code { class: "practice-key", "{golden_logic}" } },
                        AnswerType::MultipleChoice { options, correct_index } => rsx! {
                            p { class: "lesson-answer-option practice-key-option", "{options[*correct_index]}" }
                        },
                        AnswerType::Ambiguity { readings: expected } => rsx! {
                            for (i, reading) in expected.iter().enumerate() {
                                code { key: "{i}", class: "practice-key", "{reading}" }
                            }
                        },
                    }
                    if let Some(explanation) = challenge.explanation.as_ref() {
                        p { class: "lesson-explanation", "{explanation}" }
                    }
                }
            }
        }
    }
}
