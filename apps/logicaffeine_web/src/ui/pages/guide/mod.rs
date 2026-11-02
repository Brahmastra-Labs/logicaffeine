//! Programmer's Guide page.
//!
//! A beautiful, interactive guide to the LOGOS programming language with:
//! - 22 sections, from the basics to the full language
//! - Sticky sidebar navigation
//! - Interactive code examples with Run/Copy/Reset
//! - Dual mode: Logic (FOL output) and Imperative (WASM execution)
//! - Every example, exercise and quoted snippet in the Studio's editor, with the language server

pub mod content;

use crate::progress::UserProgress;
use crate::ui::components::code_editor::Language;
use crate::ui::components::exercise_block::ExerciseBlock;
use crate::ui::components::footer::Footer;
use crate::ui::components::guide_code_block::GuideCodeBlock;
use crate::ui::components::guide_sidebar::{GuideSidebar, SectionInfo};
use crate::ui::components::guide_snippet::GuideSnippet;
use crate::ui::components::main_nav::{ActivePage, MainNav};
use crate::ui::components::misconception_box::MisconceptionBox;
use crate::ui::components::recall_block::RecallBlock;
use crate::ui::inline_markup::inline_markup_html;
use crate::ui::markdown_blocks::{blocks, Block};
use crate::ui::seo::{
    breadcrumb_schema, organization_schema, pages as seo_pages, tech_article_schema,
    BreadcrumbItem, JsonLdMultiple, PageHead,
};
use content::SECTIONS;
use dioxus::prelude::*;
#[cfg(all(feature = "split", target_arch = "wasm32"))]
use dioxus::wasm_split;

/// Today as `YYYY-MM-DD` — the date the SM-2 deck schedules against.
pub fn today_string() -> String {
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

/// The canonical rendering of a theorem run — shared by the Run button and
/// the test harness, so the locked expected output IS what the reader sees.
pub fn theorem_verdict(r: &logicaffeine_compile::TheoremCompileResult) -> Result<String, String> {
    if let Some(e) = &r.error {
        return Err(e.clone());
    }
    let mut out = if r.verified {
        format!("Proved: {} — kernel-certified.", r.name)
    } else {
        format!("Not proved: {}.", r.name)
    };
    if let Some(goal) = &r.goal_string {
        out.push_str(&format!("\nGoal: {goal}"));
    }
    Ok(out)
}

/// The verdict on a web page's own `## Test` blocks — shared by the web exercise grader and the
/// test harness, so what the reader is told is what the suite checked. `Ok` carries how many tests
/// passed; a page with no test, or with any failing one, is an `Err` naming what went wrong.
///
/// Host-only because the test runner it reads (`logicaffeine_compile::testrun`) is: the runner
/// seals each test's world with the in-memory filesystem, which `logicaffeine_system` builds for
/// native targets only.
#[cfg(not(target_arch = "wasm32"))]
pub fn page_verdict(source: &str) -> Result<usize, String> {
    use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};
    let report = run_tests(source, &TestConfig::default())?;
    if report.outcomes.is_empty() {
        return Err(
            "the page has no `## Test` — a test is how a page proves what it does".to_string(),
        );
    }
    let failed: Vec<String> = report
        .outcomes
        .iter()
        .filter(|o| o.verdict != Verdict::Passed)
        .map(|o| format!("`{}`: {}", o.name, o.failures.join("; ")))
        .collect();
    if failed.is_empty() {
        Ok(report.passed)
    } else {
        Err(failed.join("\n"))
    }
}

/// Case-insensitive full-text search over section titles, prose, hooks,
/// objectives, misconceptions, recap questions, and example labels/code, in
/// display order. Blank queries match nothing (no noise).
pub fn search_sections(query: &str) -> Vec<&'static content::Section> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    SECTIONS
        .iter()
        .filter(|s| {
            s.title.to_lowercase().contains(&q)
                || s.content.to_lowercase().contains(&q)
                || s.hook.to_lowercase().contains(&q)
                || s.objectives.iter().any(|o| o.to_lowercase().contains(&q))
                || s.misconceptions.iter().any(|m| {
                    m.expectation.to_lowercase().contains(&q)
                        || m.reality.to_lowercase().contains(&q)
                })
                || s.recap.iter().any(|r| {
                    r.question.to_lowercase().contains(&q) || r.answer.to_lowercase().contains(&q)
                })
                || s.examples.iter().any(|e| {
                    e.label.to_lowercase().contains(&q) || e.code.to_lowercase().contains(&q)
                })
        })
        .collect()
}

const GUIDE_STYLE: &str = r#"
.guide-page {
    min-height: 100vh;
    color: var(--text-primary);
    background:
        radial-gradient(1200px 600px at 50% -120px, rgba(167,139,250,0.14), transparent 60%),
        radial-gradient(900px 500px at 15% 30%, rgba(96,165,250,0.14), transparent 60%),
        radial-gradient(800px 450px at 90% 45%, rgba(34,197,94,0.08), transparent 62%),
        linear-gradient(180deg, #070a12, #0b1022 55%, #070a12);
    font-family: var(--font-sans);
}

/* Navigation - now handled by MainNav component */

/* Hero */
.guide-hero {
    max-width: 1280px;
    margin: 0 auto;
    padding: 60px var(--spacing-xl) 40px;
}

.guide-hero h1 {
    font-size: var(--font-display-lg);
    font-weight: 900;
    letter-spacing: -1.5px;
    line-height: 1.1;
    background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.78) 65%, rgba(229,231,235,0.62) 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    margin: 0 0 var(--spacing-lg);
}

.guide-hero p {
    font-size: var(--font-body-lg);
    color: var(--text-secondary);
    max-width: 600px;
    line-height: 1.6;
    margin: 0;
}

.guide-hero-badge {
    display: inline-flex;
    align-items: center;
    gap: var(--spacing-sm);
    padding: var(--spacing-sm) 14px;
    border-radius: var(--radius-full);
    background: rgba(255,255,255,0.06);
    border: 1px solid rgba(255,255,255,0.10);
    font-size: var(--font-caption-md);
    font-weight: 600;
    color: var(--text-primary);
    margin-bottom: var(--spacing-xl);
}

.guide-hero-badge .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--color-success);
    box-shadow: 0 0 0 4px rgba(34,197,94,0.15);
}

/* Layout */
.guide-layout {
    max-width: 1280px;
    margin: 0 auto;
    display: flex;
    gap: 48px;
    padding: 0 var(--spacing-xl) 80px;
}

/* Main content */
.guide-content {
    flex: 1;
    min-width: 0;
    max-width: 800px;
    padding-right: var(--spacing-lg);
}

@media (max-width: 1024px) {
    .guide-content {
        padding-right: 0;
    }
}

/* Section styling */
.guide-section {
    margin-bottom: 64px;
    scroll-margin-top: 100px;
}

.guide-section h2 {
    font-size: var(--font-display-md);
    font-weight: 800;
    letter-spacing: -0.8px;
    line-height: 1.2;
    background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.85) 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    margin: 0 0 var(--spacing-xl);
    padding-bottom: var(--spacing-lg);
    border-bottom: 1px solid rgba(255,255,255,0.08);
}

.guide-section h3 {
    font-size: var(--font-heading-sm);
    font-weight: 700;
    color: var(--text-primary);
    margin: var(--spacing-xxl) 0 var(--spacing-lg);
}

.guide-section p {
    color: var(--text-secondary);
    font-size: var(--font-body-sm);
    line-height: 1.75;
    margin: 0 0 var(--spacing-lg);
}

.guide-section ul,
.guide-section ol {
    color: var(--text-secondary);
    font-size: var(--font-body-sm);
    line-height: 1.75;
    padding-left: var(--spacing-xl);
    margin: 0 0 var(--spacing-lg);
}

.guide-section li {
    margin-bottom: var(--spacing-sm);
}

.guide-section hr {
    border: 0;
    border-top: 1px solid rgba(255,255,255,0.08);
    margin: var(--spacing-xxl) 0;
}

.guide-section code {
    font-family: var(--font-mono);
    background: rgba(255,255,255,0.08);
    padding: 3px 7px;
    border-radius: var(--radius-sm);
    font-size: 0.9em;
    color: var(--color-accent-purple);
}

/* Plain fences (install commands, config) — copyable, one command per block. A class of their
   own, apart from the live example component's. */
.guide-section .guide-fence {
    position: relative;
    margin: 0 0 var(--spacing-lg);
}

.guide-section .guide-fence pre {
    font-family: var(--font-mono);
    background: rgba(255,255,255,0.05);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: var(--radius-lg);
    padding: var(--spacing-lg);
    padding-right: 72px;
    overflow-x: auto;
    font-size: 0.9em;
    line-height: 1.6;
    margin: 0;
}

.guide-section .guide-fence code {
    background: none;
    padding: 0;
    color: var(--text-primary);
}

.guide-section .guide-code-copy {
    position: absolute;
    top: 10px;
    right: 10px;
    font-size: 12px;
    color: var(--text-secondary);
    background: rgba(255,255,255,0.06);
    border: 1px solid rgba(255,255,255,0.1);
    border-radius: var(--radius-sm);
    padding: 3px 10px;
    cursor: pointer;
}

.guide-section .guide-code-copy:hover {
    color: var(--text-primary);
    background: rgba(255,255,255,0.12);
}

/* LOGOS quoted in the prose: the shared editor, read-only, on a command block's surface */
.guide-section .guide-snippet {
    position: relative;
    margin: 0 0 var(--spacing-lg);
    background: rgba(255,255,255,0.05);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: var(--radius-lg);
}

.guide-section .guide-snippet .code-editor-highlight,
.guide-section .guide-snippet .code-editor-textarea {
    padding-right: 72px;
}

/* Above the editor's text area, which lies over the whole snippet */
.guide-section .guide-snippet .guide-code-copy {
    z-index: 3;
}

.guide-section strong {
    color: var(--text-primary);
    font-weight: 600;
}

/* Tables - wrapped in scrollable container */
.guide-section .table-wrapper {
    width: 100%;
    overflow-x: auto;
    margin: var(--spacing-xl) 0;
    border-radius: var(--radius-lg);
    border: 1px solid rgba(255,255,255,0.08);
    -webkit-overflow-scrolling: touch;
}

.guide-section table {
    width: 100%;
    min-width: 400px;
    border-collapse: collapse;
    font-size: var(--font-body-md);
}

.guide-section th {
    text-align: left;
    padding: 14px var(--spacing-lg);
    background: rgba(255,255,255,0.05);
    color: var(--text-primary);
    font-weight: 600;
    border-bottom: 1px solid rgba(255,255,255,0.08);
    white-space: nowrap;
}

.guide-section td {
    padding: var(--spacing-md) var(--spacing-lg);
    color: var(--text-secondary);
    border-bottom: 1px solid rgba(255,255,255,0.05);
}

.guide-section tr:last-child td {
    border-bottom: none;
}

.guide-section tr:hover td {
    background: rgba(255,255,255,0.02);
}

/* Collapsible examples container */
.guide-examples-collapsible {
    margin-top: var(--spacing-xl);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: var(--radius-lg);
    overflow: hidden;
}

.guide-examples-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--spacing-lg) var(--spacing-xl);
    background: rgba(255,255,255,0.03);
    cursor: pointer;
    user-select: none;
    transition: background 0.15s ease;
}

.guide-examples-header:hover {
    background: rgba(255,255,255,0.05);
}

.guide-examples-header-left {
    display: flex;
    align-items: center;
    gap: var(--spacing-md);
}

.guide-examples-title {
    font-size: var(--font-body-md);
    font-weight: 600;
    color: var(--text-primary);
}

.guide-examples-count {
    font-size: var(--font-caption-md);
    color: var(--text-tertiary);
    background: rgba(255,255,255,0.06);
    padding: 4px 10px;
    border-radius: var(--radius-full);
}

.guide-examples-chevron {
    font-size: 12px;
    color: var(--text-tertiary);
    transition: transform 0.2s ease;
}

.guide-examples-collapsible.expanded .guide-examples-chevron {
    transform: rotate(90deg);
}

.guide-examples-content {
    max-height: 0;
    overflow: hidden;
    transition: max-height 0.3s ease;
}

.guide-examples-collapsible.expanded .guide-examples-content {
    max-height: 10000px;
}

.guide-examples-inner {
    padding: var(--spacing-lg);
    display: flex;
    flex-direction: column;
    gap: var(--spacing-lg);
}

/* Part dividers */
.guide-part-divider {
    margin: 80px 0 48px;
    padding: var(--spacing-xl) 0;
    border-top: 1px solid rgba(255,255,255,0.08);
}

.guide-part-divider h2 {
    font-size: var(--font-body-md);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 1.5px;
    color: var(--text-tertiary);
    margin: 0;
    background: none;
    -webkit-text-fill-color: currentColor;
    border-bottom: none;
    padding-bottom: 0;
}

/* Section number */
.section-number {
    display: inline-block;
    font-size: var(--font-body-md);
    font-weight: 700;
    color: var(--color-accent-purple);
    margin-right: var(--spacing-sm);
    opacity: 0.8;
}

/* Examples container */
.guide-examples {
    margin-top: var(--spacing-xl);
    display: flex;
    flex-direction: column;
    gap: var(--spacing-lg);
}

/* Responsive */
@media (max-width: 1024px) {
    .guide-layout {
        flex-direction: column;
    }

    .guide-hero h1 {
        font-size: var(--font-display-md);
    }

    .guide-hero {
        padding: 40px var(--spacing-xl) var(--spacing-xxl);
    }
}

@media (max-width: 640px) {
    .guide-hero h1 {
        font-size: var(--font-heading-lg);
    }

    .guide-hero p {
        font-size: var(--font-body-md);
    }

    .guide-section h2 {
        font-size: var(--font-heading-lg);
    }
}

.guide-community-cta {
    text-align: center;
    padding: var(--spacing-xxl) var(--spacing-xl);
    margin-top: var(--spacing-xxl);
    border-top: 1px solid rgba(255,255,255,0.06);
}

.guide-community-cta p {
    font-size: var(--font-body-lg);
    color: var(--text-secondary);
    margin-bottom: var(--spacing-md);
}

.guide-community-cta a {
    display: inline-block;
    padding: 8px 20px;
    border-radius: var(--radius-full);
    background: rgba(167,139,250,0.06);
    border: 1px solid rgba(167,139,250,0.3);
    color: var(--color-accent-purple);
    font-weight: 600;
    font-size: var(--font-body-md);
    text-decoration: none;
    transition: background 0.2s, border-color 0.2s;
}

.guide-community-cta a:hover {
    background: rgba(167,139,250,0.14);
    border-color: rgba(167,139,250,0.5);
}

/* The hook: the headache before the aspirin */
.guide-hook {
    font-size: var(--font-body-md);
    color: var(--text-primary);
    line-height: 1.7;
    padding: var(--spacing-lg) var(--spacing-xl);
    margin: 0 0 var(--spacing-xl);
    border-left: 3px solid var(--color-accent-purple);
    border-radius: 0 var(--radius-sm) var(--radius-sm) 0;
    background: rgba(167,139,250,0.07);
}

/* "After this section you can" */
.guide-objectives {
    margin: 0 0 var(--spacing-xl);
    padding: var(--spacing-lg) var(--spacing-xl);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: var(--radius-md);
    background: rgba(255,255,255,0.03);
}

.guide-objectives-title {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.6px;
    color: rgba(229,231,235,0.55);
    margin-bottom: var(--spacing-sm);
}

.guide-objectives ul {
    margin: 0;
    padding-left: var(--spacing-xl);
}

.guide-objectives li {
    color: var(--text-secondary);
    font-size: var(--font-body-sm);
    margin-bottom: 4px;
}

/* Blockquote callouts */
.guide-section blockquote {
    margin: 0 0 var(--spacing-lg);
    padding: var(--spacing-md) var(--spacing-lg);
    border-left: 3px solid rgba(255,255,255,0.2);
    border-radius: 0 var(--radius-sm) var(--radius-sm) 0;
    background: rgba(255,255,255,0.03);
    color: var(--text-secondary);
    font-size: var(--font-body-sm);
    line-height: 1.7;
}

.guide-section blockquote.guide-mental-model {
    border-left-color: #22c55e;
    background: rgba(34,197,94,0.06);
}

.guide-section blockquote.guide-look-back {
    border-left-color: #60a5fa;
    background: rgba(96,165,250,0.06);
}
"#;

// `lazy`: with the `split` feature + dx `--wasm-split`, this page's body (and the
// guide content strings only it references) moves into the lazily-fetched chunk.
#[component(lazy)]
pub fn Guide() -> Element {
    let mut active_section = use_signal(|| "introduction".to_string());
    let mut search_query = use_signal(String::new);
    // Example groups default to EXPANDED; this set holds the ids the reader
    // (or the collapse-all control) has closed.
    let mut collapsed_groups = use_signal(std::collections::HashSet::<String>::new);
    let mut completed = use_signal(Vec::<String>::new);
    // The SM-2 progress store, shared with every exercise block and recall
    // card on the page (they write through it, the mastery ring reads it).
    let progress = use_context_provider(|| Signal::new(UserProgress::load()));

    // Completion checkmarks persist per browser via localStorage.
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_storage::Storage;
        use_effect(move || {
            if let Ok(saved) = gloo_storage::LocalStorage::get::<Vec<String>>("guide-completed") {
                completed.set(saved);
            }
        });
    }
    let toggle_completed = move |id: String| {
        let mut list = completed.read().clone();
        if let Some(pos) = list.iter().position(|x| *x == id) {
            list.remove(pos);
        } else {
            list.push(id);
        }
        #[cfg(target_arch = "wasm32")]
        {
            use gloo_storage::Storage;
            let _ = gloo_storage::LocalStorage::set("guide-completed", &list);
        }
        completed.set(list);
    };

    // Build section info for sidebar — the mastery ring counts exercises
    // passed (correct at least once) via their namespaced SM-2 entries.
    let sections_info: Vec<SectionInfo> = {
        let prog = progress.read();
        SECTIONS
            .iter()
            .map(|s| {
                let exercise_total = s.exercises.len();
                let exercise_passed = s
                    .exercises
                    .iter()
                    .filter(|e| {
                        prog.exercises
                            .get(&crate::guide_review::exercise_key(e.id))
                            .is_some_and(|p| p.correct_count > 0)
                    })
                    .count();
                SectionInfo {
                    id: s.id.to_string(),
                    number: s.number,
                    title: s.title.to_string(),
                    part: s.part.to_string(),
                    exercise_total,
                    exercise_passed,
                }
            })
            .collect()
    };

    // Collect all section IDs for intersection observer
    #[allow(unused_variables)]
    let section_ids: Vec<String> = SECTIONS.iter().map(|s| s.id.to_string()).collect();

    // Set up scroll tracking with IntersectionObserver
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::prelude::*;
        use wasm_bindgen::JsCast;

        let section_ids_for_effect = section_ids.clone();

        use_effect(move || {
            let window = match web_sys::window() {
                Some(w) => w,
                None => return,
            };
            let document = match window.document() {
                Some(d) => d,
                None => return,
            };

            // Use RefCell to allow mutation from within Fn closure
            use std::cell::RefCell;
            use std::rc::Rc;

            let active_section_clone = Rc::new(RefCell::new(active_section.clone()));
            let active_section_for_closure = active_section_clone.clone();

            let callback = Closure::<dyn Fn(js_sys::Array, web_sys::IntersectionObserver)>::new(
                move |entries: js_sys::Array, _observer: web_sys::IntersectionObserver| {
                    // Simple approach: when a section crosses the threshold line,
                    // it becomes active
                    for i in 0..entries.length() {
                        if let Ok(entry) = entries
                            .get(i)
                            .dyn_into::<web_sys::IntersectionObserverEntry>()
                        {
                            if entry.is_intersecting() {
                                let target = entry.target();
                                let id = target.id();
                                if !id.is_empty() {
                                    active_section_for_closure.borrow_mut().set(id);
                                }
                            }
                        }
                    }
                },
            );

            // Create IntersectionObserver options
            let mut options = web_sys::IntersectionObserverInit::new();
            // Root margin creates a thin "tripwire" near the top of the screen
            options.root_margin("-100px 0px -90% 0px");
            let thresholds = js_sys::Array::new();
            thresholds.push(&JsValue::from(0.0));
            options.threshold(&thresholds);

            // Create the observer
            let observer = match web_sys::IntersectionObserver::new_with_options(
                callback.as_ref().unchecked_ref(),
                &options,
            ) {
                Ok(obs) => obs,
                Err(_) => return,
            };

            // Observe all sections
            for section_id in &section_ids_for_effect {
                if let Some(element) = document.get_element_by_id(section_id) {
                    observer.observe(&element);
                }
            }

            // Keep callback alive
            callback.forget();
        });
    }

    // Track current part for dividers
    let mut current_part = String::new();

    let breadcrumbs = vec![
        BreadcrumbItem {
            name: "Home",
            path: "/",
        },
        BreadcrumbItem {
            name: "Syntax Guide",
            path: "/guide",
        },
    ];
    let schemas = vec![
        organization_schema(),
        tech_article_schema("LOGOS Syntax Guide", "Comprehensive guide to LOGOS syntax, features, and First-Order Logic concepts with interactive examples.", "/guide"),
        breadcrumb_schema(&breadcrumbs),
    ];

    rsx! {
        PageHead {
            title: seo_pages::GUIDE.title,
            description: seo_pages::GUIDE.description,
            canonical_path: seo_pages::GUIDE.canonical_path,
        }
        style { "{GUIDE_STYLE}" }
        JsonLdMultiple { schemas }

        div { class: "guide-page",
            // Navigation
            MainNav {
                active: ActivePage::Guide,
                subtitle: Some("Programmer's Guide"),
            }

            // Hero
            header { class: "guide-hero",
                div { class: "guide-hero-badge",
                    div { class: "dot" }
                    span { "Interactive Guide" }
                }
                h1 { "LOGOS Syntax Guide" }
                p {
                    "Write English. Get Logic. Run Code. A comprehensive guide to programming in LOGOS, from basics to advanced features."
                }

                // In-guide search
                div {
                    style: "max-width: 560px; margin: 18px auto 0; position: relative;",
                    input {
                        class: "guide-search-input",
                        style: "width: 100%; padding: 10px 16px; border-radius: 10px; border: 1px solid rgba(255,255,255,0.15); background: rgba(0,0,0,0.35); color: rgba(229,231,235,0.95); font-size: 14px; outline: none;",
                        placeholder: "Search the guide — sections, prose, code…",
                        value: "{search_query}",
                        oninput: move |evt| search_query.set(evt.value()),
                    }
                    {
                        let q = search_query.read().clone();
                        let hits = search_sections(&q);
                        if !hits.is_empty() {
                            rsx! {
                                div {
                                    style: "position: absolute; z-index: 30; left: 0; right: 0; margin-top: 6px; border-radius: 10px; border: 1px solid rgba(255,255,255,0.12); background: #0b1022; box-shadow: 0 12px 40px rgba(0,0,0,0.5); text-align: left; max-height: 300px; overflow-y: auto;",
                                    for hit in hits.into_iter().take(8) {
                                        a {
                                            style: "display: block; padding: 9px 14px; color: rgba(229,231,235,0.9); text-decoration: none; font-size: 13px; border-bottom: 1px solid rgba(255,255,255,0.05);",
                                            href: "#{hit.id}",
                                            onclick: move |_| search_query.set(String::new()),
                                            span { style: "opacity: 0.55; margin-right: 8px;", "{hit.number}." }
                                            "{hit.title}"
                                        }
                                    }
                                }
                            }
                        } else {
                            rsx! {}
                        }
                    }
                }
            }

            // Main layout
            div { class: "guide-layout",
                // Sidebar
                GuideSidebar {
                    sections: sections_info,
                    active_section: active_section.read().clone(),
                    on_section_click: move |id: String| {
                        active_section.set(id);
                    },
                    completed: completed.read().clone(),
                    on_toggle_complete: toggle_completed,
                }

                // Content
                main { class: "guide-content",
                    div {
                        style: "display: flex; justify-content: flex-end; margin-bottom: 4px;",
                        button {
                            class: "guide-expand-all",
                            style: "padding: 6px 12px; border-radius: 8px; border: 1px solid rgba(255,255,255,0.15); background: rgba(255,255,255,0.05); color: rgba(229,231,235,0.75); font-size: 12px; cursor: pointer;",
                            onclick: move |_| {
                                if collapsed_groups.read().is_empty() {
                                    let all: std::collections::HashSet<String> = SECTIONS
                                        .iter()
                                        .filter(|s| s.examples.len() > 3)
                                        .map(|s| s.id.to_string())
                                        .collect();
                                    collapsed_groups.set(all);
                                } else {
                                    collapsed_groups.set(std::collections::HashSet::new());
                                }
                            },
                            if collapsed_groups.read().is_empty() { "Collapse example groups" } else { "Expand all examples" }
                        }
                    }
                    for section in SECTIONS.iter() {
                        {
                            // Check if we need a part divider
                            let show_divider = section.part != current_part && section.number > 1;
                            current_part = section.part.to_string();

                            rsx! {
                                // Part divider
                                if show_divider {
                                    div { class: "guide-part-divider",
                                        h2 { "{section.part}" }
                                    }
                                }

                                // Section
                                section {
                                    id: "{section.id}",
                                    class: "guide-section",

                                    h2 {
                                        span { class: "section-number", "{section.number}." }
                                        "{section.title}"
                                    }

                                    // The headache before the aspirin — the hook
                                    // opens the section, before any definition.
                                    if !section.hook.is_empty() {
                                        div { class: "guide-hook", "{section.hook}" }
                                    }

                                    // "After this section you can:" — the contract
                                    // the recap questions collect on.
                                    if !section.objectives.is_empty() {
                                        div { class: "guide-objectives",
                                            div { class: "guide-objectives-title", "After this section you can" }
                                            ul {
                                                for objective in section.objectives.iter() {
                                                    li { "{objective}" }
                                                }
                                            }
                                        }
                                    }

                                    // The prose, the LOGOS it quotes in the shared editor
                                    for block in prose_blocks(section.content) {
                                        {prose_block(block)}
                                    }

                                    // Imported beliefs, confronted before the
                                    // examples that settle them.
                                    for m in section.misconceptions.iter() {
                                        MisconceptionBox {
                                            id: m.id.to_string(),
                                            expectation: m.expectation.to_string(),
                                            reality: m.reality.to_string(),
                                            evidence: m.evidence.to_string(),
                                        }
                                    }

                                    // Render code examples - collapsible if many
                                    if !section.examples.is_empty() {
                                        {
                                            let example_count = section.examples.len();
                                            let should_collapse = example_count > 3;

                                            if should_collapse {
                                                rsx! {
                                                    CollapsibleExamples {
                                                        section_id: section.id.to_string(),
                                                        examples: section.examples,
                                                        is_open: !collapsed_groups.read().contains(section.id),
                                                        on_toggle: move |id: String| {
                                                            let mut set = collapsed_groups.read().clone();
                                                            if !set.remove(&id) {
                                                                set.insert(id);
                                                            }
                                                            collapsed_groups.set(set);
                                                        },
                                                    }
                                                }
                                            } else {
                                                rsx! {
                                                    div { class: "guide-examples",
                                                        for example in section.examples.iter() {
                                                            GuideCodeBlock {
                                                                id: example.id.to_string(),
                                                                label: example.label.to_string(),
                                                                mode: example.mode,
                                                                runnability: example.runnability,
                                                                expected_output: example.expected_output.map(|s| s.to_string()),
                                                                predict: example.predict_first,
                                                                initial_code: example.code.to_string(),
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    // "Now you try" exercises, graded in place
                                    for ex in section.exercises.iter() {
                                        ExerciseBlock {
                                            id: ex.id.to_string(),
                                            prompt: ex.prompt.to_string(),
                                            mode: ex.mode,
                                            expected: ex.expected.to_string(),
                                            starter: ex.starter.to_string(),
                                            kind: ex.kind,
                                            concepts: ex.concepts.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
                                            hints: ex.hints.iter().map(|h| h.to_string()).collect::<Vec<_>>(),
                                        }
                                    }

                                    // "Check yourself" — free recall with reveal;
                                    // self-marks feed the SM-2 review deck.
                                    if !section.recap.is_empty() {
                                        RecallBlock {
                                            cards: section.recap.iter().map(|r| {
                                                (r.id.to_string(), r.question.to_string(), r.answer.to_string())
                                            }).collect::<Vec<_>>(),
                                        }
                                    }
                                }
                            }
                        }
                        div { class: "guide-community-cta",
                            p { "Have questions or feedback? Join our Discord community." }
                            a {
                                href: "https://discord.gg/pwnjnXvUHH",
                                target: "_blank",
                                rel: "noopener noreferrer",
                                "Join Discord →"
                            }
                        }
                    }
                }
            }

            Footer {}
        }
    }
}

/// Collapsible examples component for sections with many examples. Controlled
/// by the page: open by default, closable per group or via collapse-all.
#[component]
fn CollapsibleExamples(
    section_id: String,
    examples: &'static [content::CodeExample],
    is_open: bool,
    on_toggle: EventHandler<String>,
) -> Element {
    let example_count = examples.len();

    rsx! {
        div {
            class: if is_open { "guide-examples-collapsible expanded" } else { "guide-examples-collapsible" },

            div {
                class: "guide-examples-header",
                onclick: {
                    let id = section_id.clone();
                    move |_| on_toggle.call(id.clone())
                },

                div { class: "guide-examples-header-left",
                    span { class: "guide-examples-title", "Code Examples" }
                    span { class: "guide-examples-count", "{example_count} examples" }
                }
                span { class: "guide-examples-chevron", "▶" }
            }

            div { class: "guide-examples-content",
                div { class: "guide-examples-inner",
                    for example in examples.iter() {
                        GuideCodeBlock {
                            id: example.id.to_string(),
                            label: example.label.to_string(),
                            mode: example.mode,
                            runnability: example.runnability,
                            expected_output: example.expected_output.map(|s| s.to_string()),
                            predict: example.predict_first,
                            initial_code: example.code.to_string(),
                        }
                    }
                }
            }
        }
    }
}

/// A section's prose as one piece of static markup, every fence a plain copyable block — the
/// form the markdown locks read. The page itself renders [`prose_blocks`], quoting LOGOS fences
/// in the shared editor.
fn render_markdown(content: &str) -> String {
    prose_blocks(content)
        .into_iter()
        .map(|block| match block {
            ProseBlock::Html(html) => html,
            ProseBlock::Code { code, .. } => code_block_html(&code),
        })
        .collect()
}

/// A block of guide prose as markup, its text marked up by the site's one inline renderer. A quote that opens with
/// `**Mental model:**` or `**Look back:**` is styled as that callout; the pedagogy battery rides
/// on those conventions.
fn prose_html(block: &Block) -> String {
    match block {
        Block::Heading { level, text } => {
            format!("<h{level}>{}</h{level}>", inline_markup_html(text))
        }
        Block::Paragraph(text) => format!("<p>{}</p>", inline_markup_html(text)),
        Block::List { ordered, items } => {
            let tag = if *ordered { "ol" } else { "ul" };
            let items: String = items
                .iter()
                .map(|item| format!("<li>{}</li>", inline_markup_html(item)))
                .collect();
            format!("<{tag}>{items}</{tag}>")
        }
        Block::Quote(paragraphs) => {
            let opener = paragraphs.first().map_or("", String::as_str);
            let class = if opener.contains("**Mental model:**") {
                " class=\"guide-callout guide-mental-model\""
            } else if opener.contains("**Look back:**") {
                " class=\"guide-callout guide-look-back\""
            } else {
                ""
            };
            let body: Vec<String> = paragraphs.iter().map(|p| inline_markup_html(p)).collect();
            format!(
                "<blockquote{class}>{}</blockquote>",
                body.join("<br/><br/>")
            )
        }
        Block::Table { header, rows } => {
            let row = |cells: &[String], tag: &str| -> String {
                let cells: String = cells
                    .iter()
                    .map(|cell| format!("<{tag}>{}</{tag}>", inline_markup_html(cell)))
                    .collect();
                format!("<tr>{cells}</tr>")
            };
            let head = header.as_ref().map_or(String::new(), |h| {
                format!("<thead>{}</thead>", row(h, "th"))
            });
            let body: String = rows.iter().map(|r| row(r, "td")).collect();
            format!("<div class=\"table-wrapper\"><table>{head}<tbody>{body}</tbody></table></div>")
        }
        Block::Fence { code, .. } => code_block_html(code),
        Block::Rule => "<hr/>".to_string(),
    }
}

/// The markdown renderer, exported for the render locks — the blockquote
/// callout conventions (`> **Mental model:**`, `> **Look back:**`) are load-
/// bearing for the pedagogy battery and must never silently stop parsing.
pub fn render_markdown_for_test(content: &str) -> String {
    render_markdown(content)
}

/// One piece of a section's prose, in page order: rendered markup, or a fenced code block.
#[derive(Clone, Debug, PartialEq)]
pub enum ProseBlock {
    Html(String),
    /// The language its opening fence names (empty when it names none) and its lines as written.
    Code {
        tag: String,
        code: String,
    },
}

/// How the page shows a fenced block, by the language its opening fence names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fence {
    /// ```` ```logos ````: LOGOS code, in the shared editor with the language server.
    Logos,
    /// ```` ```logic ````: sentences and theorems, in the same editor, read as Logic.
    Logic,
    /// Shell commands and configuration: plain, copyable text.
    Plain,
}

impl Fence {
    /// The editor language the block is read in; plain text has none.
    pub fn language(self) -> Option<Language> {
        match self {
            Fence::Logos => Some(Language::Logos),
            Fence::Logic => Some(Language::Logic),
            Fence::Plain => None,
        }
    }
}

/// The fence a tag names, or `None` for a tag the guide does not render.
pub fn fence(tag: &str) -> Option<Fence> {
    match tag {
        "logos" => Some(Fence::Logos),
        "logic" => Some(Fence::Logic),
        "bash" | "powershell" | "toml" => Some(Fence::Plain),
        _ => None,
    }
}

/// A section's prose split at its fences, in page order. A fence's lines are kept exactly as
/// written, blank lines and indentation included; whatever prose was open before it (a list, a
/// table, a quote) ends where it begins.
pub fn prose_blocks(content: &str) -> Vec<ProseBlock> {
    let mut out = Vec::new();
    let mut html = String::new();
    for block in blocks(content) {
        match block {
            Block::Fence { tag, code } => {
                if !html.is_empty() {
                    out.push(ProseBlock::Html(std::mem::take(&mut html)));
                }
                out.push(ProseBlock::Code { tag, code });
            }
            block => html.push_str(&prose_html(&block)),
        }
    }
    if !html.is_empty() {
        out.push(ProseBlock::Html(html));
    }
    out
}

/// One piece of a section's prose on the page: markup as it renders, LOGOS and Logic quoted in
/// the shared editor, and shell and config text as a plain copyable block.
fn prose_block(block: ProseBlock) -> Element {
    match block {
        ProseBlock::Html(html) => rsx! {
            div { dangerous_inner_html: html }
        },
        ProseBlock::Code { tag, code } => match fence(&tag).and_then(Fence::language) {
            Some(language) => rsx! {
                GuideSnippet { code, language }
            },
            None => rsx! {
                div { dangerous_inner_html: code_block_html(&code) }
            },
        },
    }
}

/// A fenced code block as copy-friendly HTML: a Copy button (clipboard API, inline
/// handler — the block arrives via `dangerous_inner_html`, so it can't carry a
/// Dioxus listener) over an escaped `<pre><code>`.
fn code_block_html(code: &str) -> String {
    format!(
        "<div class=\"guide-fence\">\
         <button class=\"guide-code-copy\" \
         onclick=\"navigator.clipboard.writeText(this.nextElementSibling.textContent);\
         this.textContent='Copied!';setTimeout(()=>{{this.textContent='Copy'}},1200)\">Copy</button>\
         <pre><code>{}</code></pre></div>",
        html_escape(code.trim_end_matches('\n'))
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
#[path = "../../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/guide/render_markdown_tests.rs"]
mod render_markdown_tests;

#[cfg(test)]
#[path = "../../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/guide/prose_block_tests.rs"]
mod prose_block_tests;
