//! Development roadmap page.
//!
//! Two tiers: the curated `roadmap_data::get_milestones` capability cards on
//! top (rendered by `MilestoneItem`), and a terse, scrollable release history
//! below, generated from `CHANGELOG.md` into
//! [`roadmap_history`](super::roadmap_history). Each release links to its news
//! article when one exists. Milestones carry a [`Status`], feature tags, and
//! interactive English → FOL examples (toggle between Simple FOL and Unicode
//! output).
//!
//! Each card draws its own status dot (a check, a hammer, a drafting compass)
//! and the stretch of timeline down to the next card, placed from one set of
//! CSS variables so the dots stay on the line at every width. The release
//! history opens with the Next release block, which the build bakes from the
//! CHANGELOG's `[Unreleased]` section ([`changelog`](super::changelog)).
//!
//! # Route
//!
//! Accessed via [`Route::Roadmap`].

use crate::ui::components::footer::Footer;
use crate::ui::components::icon::{Icon, IconSize, IconVariant};
use crate::ui::components::main_nav::{ActivePage, MainNav};
use crate::ui::pages::news::get_articles;
use crate::ui::pages::roadmap_data::{get_milestones, Milestone, Status};
use crate::ui::pages::roadmap_history::{get_history, news_slug_for, Release};
use crate::ui::router::Route;
use crate::ui::seo::{
    breadcrumb_schema, organization_schema, pages as seo_pages, roadmap_schema, BreadcrumbItem,
    JsonLdMultiple, PageHead,
};
use dioxus::prelude::*;
#[cfg(all(feature = "split", target_arch = "wasm32"))]
use dioxus::wasm_split;

const ROADMAP_STYLE: &str = r#"
.roadmap-container {
    min-height: 100vh;
    background: linear-gradient(135deg, #070a12 0%, #0b1022 50%, #070a12 100%);
    color: #e5e7eb;
    font-family: ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, sans-serif;
}

.roadmap-nav {
    position: sticky;
    top: 0;
    z-index: 50;
    backdrop-filter: blur(18px);
    background: linear-gradient(180deg, rgba(7,10,18,0.72), rgba(7,10,18,0.44));
    border-bottom: 1px solid rgba(255,255,255,0.06);
    padding: 16px 20px;
}

.roadmap-nav-inner {
    max-width: 1000px;
    margin: 0 auto;
    display: flex;
    align-items: center;
    justify-content: space-between;
}

.roadmap-brand {
    display: flex;
    align-items: center;
    gap: 12px;
    text-decoration: none;
    color: #e5e7eb;
}

.roadmap-logo {
    width: 36px;
    height: 36px;
    border-radius: 12px;
    background:
        radial-gradient(circle at 30% 30%, rgba(96,165,250,0.85), transparent 55%),
        radial-gradient(circle at 65% 60%, rgba(167,139,250,0.85), transparent 55%),
        rgba(255,255,255,0.06);
    border: 1px solid rgba(255,255,255,0.10);
}

.roadmap-brand-name {
    font-weight: 800;
    font-size: 14px;
    letter-spacing: -0.5px;
}

.roadmap-back {
    color: #a78bfa;
    text-decoration: none;
    font-size: 14px;
    padding: 8px 16px;
    border-radius: 8px;
    border: 1px solid rgba(167,139,250,0.3);
    transition: all 0.2s ease;
}

.roadmap-back:hover {
    background: rgba(167,139,250,0.1);
    border-color: rgba(167,139,250,0.5);
}

.roadmap-hero {
    text-align: center;
    padding: 60px 20px 40px;
    max-width: 800px;
    margin: 0 auto;
}

.roadmap-hero h1 {
    font-size: 42px;
    font-weight: 800;
    letter-spacing: -1px;
    background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.78) 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    margin-bottom: 12px;
}

.roadmap-hero .version {
    display: inline-block;
    font-size: 14px;
    padding: 6px 14px;
    border-radius: 20px;
    background: rgba(167,139,250,0.15);
    border: 1px solid rgba(167,139,250,0.3);
    color: #a78bfa;
    margin-bottom: 16px;
}

.roadmap-hero p {
    color: rgba(229,231,235,0.72);
    font-size: 18px;
    line-height: 1.6;
}

/* The timeline's geometry, set once. Each card draws its own dot and the stretch of line from
   that dot down to the next card's, both placed from these values inside the card, so the dots
   sit on the line and the line runs from the first dot to the last at every width. */
.timeline {
    --rail-x: 28px;
    --rail-w: 3px;
    --dot: 28px;
    --card-x: 70px;
    --card-border: 1px;
    --card-pad: 24px;
    --title-line: 28px;
    --gap: 40px;
    --dot-center: calc(var(--card-border) + var(--card-pad) + var(--title-line) / 2);
    --rail-done: #22c55e;
    --rail-progress: #a78bfa;
    --rail-planned: rgba(148, 163, 184, 0.45);
    max-width: 700px;
    margin: 0 auto;
    padding: 0 20px 80px;
}

.milestone {
    position: relative;
    padding-left: var(--card-x);
    margin-bottom: var(--gap);
}

.milestone:last-child {
    margin-bottom: 0;
}

/* The line from this card's dot to the next card's, in this card's status colour fading into
   the next one's. */
.milestone::before {
    content: "";
    position: absolute;
    left: calc(var(--rail-x) - var(--rail-w) / 2);
    top: var(--dot-center);
    width: var(--rail-w);
    height: calc(100% + var(--gap));
    border-radius: 2px;
    background: linear-gradient(var(--rail-from), var(--rail-to));
}

.milestone:last-child::before {
    display: none;
}

.milestone.from-done { --rail-from: var(--rail-done); }
.milestone.from-progress { --rail-from: var(--rail-progress); }
.milestone.from-planned { --rail-from: var(--rail-planned); }
.milestone.to-done { --rail-to: var(--rail-done); }
.milestone.to-progress { --rail-to: var(--rail-progress); }
.milestone.to-planned { --rail-to: var(--rail-planned); }

.milestone-dot {
    position: absolute;
    z-index: 1;
    left: calc(var(--rail-x) - var(--dot) / 2);
    top: calc(var(--dot-center) - var(--dot) / 2);
    width: var(--dot);
    height: var(--dot);
    box-sizing: border-box;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
}

.milestone-dot.done {
    background: linear-gradient(135deg, #22c55e, #16a34a);
    box-shadow: 0 0 20px rgba(34,197,94,0.4);
}

.milestone-dot.progress {
    background: linear-gradient(135deg, #a78bfa, #8b5cf6);
    box-shadow: 0 0 20px rgba(167,139,250,0.4);
    animation: pulse 2s ease-in-out infinite;
}

/* Planned work is a sketch: an opaque disc, so the line stops at its rim, drawn in dashes. */
.milestone-dot.planned {
    background: #151b2e;
    border: 2px dashed rgba(148, 163, 184, 0.6);
}

@keyframes pulse {
    0%, 100% { transform: scale(1); }
    50% { transform: scale(1.1); }
}

.milestone-content {
    background: rgba(255,255,255,0.03);
    border: var(--card-border) solid rgba(255,255,255,0.08);
    border-radius: 16px;
    padding: var(--card-pad);
    transition: all 0.2s ease;
}

.milestone-content:hover {
    background: rgba(255,255,255,0.05);
    border-color: rgba(255,255,255,0.12);
}

.milestone-header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 12px;
    min-height: var(--title-line);
    margin-bottom: 12px;
}

/* A fixed first line box: the dot centres on it, whether or not the header wraps. */
.milestone-title {
    font-size: 20px;
    font-weight: 700;
    line-height: var(--title-line);
    color: #fff;
}

.milestone-badge {
    font-size: 11px;
    font-weight: 600;
    padding: 4px 10px;
    border-radius: 12px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
}

.milestone-badge.done {
    background: rgba(34,197,94,0.15);
    color: #22c55e;
    border: 1px solid rgba(34,197,94,0.3);
}

.milestone-badge.progress {
    background: rgba(167,139,250,0.15);
    color: #a78bfa;
    border: 1px solid rgba(167,139,250,0.3);
}

.milestone-badge.planned {
    background: rgba(255,255,255,0.05);
    color: rgba(255,255,255,0.5);
    border: 1px solid rgba(255,255,255,0.1);
}

.milestone-desc {
    color: rgba(229,231,235,0.72);
    font-size: 14px;
    line-height: 1.6;
    margin-bottom: 16px;
}

.milestone-features {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
}

.feature-tag {
    font-size: 12px;
    padding: 6px 12px;
    border-radius: 8px;
    background: rgba(255,255,255,0.04);
    border: 1px solid rgba(255,255,255,0.08);
    color: rgba(229,231,235,0.8);
}

.feature-tag.done {
    background: rgba(34,197,94,0.08);
    border-color: rgba(34,197,94,0.2);
    color: #86efac;
}

.roadmap-footer {
    border-top: 1px solid rgba(255,255,255,0.06);
    padding: 24px 20px;
    text-align: center;
    color: rgba(229,231,235,0.56);
    font-size: 13px;
}

.roadmap-footer a {
    color: rgba(229,231,235,0.72);
    text-decoration: none;
    margin: 0 8px;
}

.roadmap-footer a:hover {
    color: #a78bfa;
}

.roadmap-nav-links {
    display: flex;
    align-items: center;
    gap: 12px;
}

.roadmap-github {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    border-radius: 8px;
    border: 1px solid rgba(255,255,255,0.1);
    background: rgba(255,255,255,0.03);
    color: rgba(229,231,235,0.72);
    transition: all 0.2s ease;
}

.roadmap-github:hover {
    background: rgba(255,255,255,0.08);
    color: #e5e7eb;
    border-color: rgba(255,255,255,0.2);
}

.roadmap-github svg {
    width: 18px;
    height: 18px;
    fill: currentColor;
}

.github-link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
}

@media (max-width: 600px) {
    .timeline {
        --rail-x: 14px;
        --dot: 24px;
        --card-x: 40px;
        --card-pad: 18px;
        --title-line: 26px;
    }
    .milestone-title {
        font-size: 18px;
    }
}

.milestone-examples {
    margin-top: 16px;
    border-top: 1px solid rgba(255,255,255,0.06);
    padding-top: 16px;
}

.milestone-tabs {
    display: flex;
    gap: 6px;
    margin-bottom: 12px;
    flex-wrap: wrap;
}

.milestone-tab {
    padding: 6px 12px;
    border-radius: 6px;
    border: 1px solid rgba(255,255,255,0.1);
    background: rgba(255,255,255,0.03);
    color: #94a3b8;
    cursor: pointer;
    font-size: 12px;
    font-weight: 500;
    transition: all 0.2s ease;
}

.milestone-tab:hover {
    background: rgba(255,255,255,0.08);
    color: #e8e8e8;
}

.milestone-tab.active {
    background: linear-gradient(135deg, #667eea, #764ba2);
    color: white;
    border-color: transparent;
}

.milestone-code {
    background: rgba(0,0,0,0.25);
    border-radius: 8px;
    padding: 16px;
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
    font-size: 13px;
}

.milestone-english {
    color: #e8e8e8;
    font-style: italic;
    margin-bottom: 8px;
    white-space: pre-wrap;
    line-height: 1.5;
}

.milestone-english.source {
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
    font-style: normal;
    font-size: 12px;
    color: #cbd5e1;
}

.milestone-arrow {
    color: #667eea;
    margin: 8px 0;
    font-size: 16px;
}

.milestone-output {
    color: #98c379;
    white-space: pre-wrap;
    line-height: 1.4;
}

.format-toggle {
    display: flex;
    gap: 4px;
    margin-bottom: 8px;
}

.format-btn {
    padding: 3px 8px;
    border-radius: 4px;
    border: 1px solid rgba(255,255,255,0.1);
    background: rgba(255,255,255,0.03);
    color: #64748b;
    cursor: pointer;
    font-size: 10px;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    transition: all 0.15s ease;
}

.format-btn:hover {
    background: rgba(255,255,255,0.06);
    color: #94a3b8;
}

.format-btn.active {
    background: rgba(102,126,234,0.2);
    border-color: rgba(102,126,234,0.4);
    color: #a5b4fc;
}
"#;

#[component]
fn MilestoneExamples(examples: &'static [crate::ui::pages::roadmap_data::Example]) -> Element {
    use crate::ui::pages::roadmap_data::Output;
    let mut active = use_signal(|| 0usize);
    let mut use_unicode = use_signal(|| false);

    let ex = examples[active()];

    rsx! {
        div { class: "milestone-examples",
            div { class: "milestone-tabs",
                for (i, e) in examples.iter().enumerate() {
                    button {
                        key: "{i}",
                        class: if active() == i { "milestone-tab active" } else { "milestone-tab" },
                        onclick: move |_| active.set(i),
                        "{e.label}"
                    }
                }
            }
            div { class: "milestone-code",
                {match ex.output {
                    Output::Fol { .. } => rsx! {
                        div { class: "milestone-english", "\"{ex.english}\"" }
                    },
                    _ => rsx! {
                        div { class: "milestone-english source", "{ex.english}" }
                    },
                }}
                div { class: "milestone-arrow", "↓" }
                {match ex.output {
                    Output::Fol { simple, unicode } => rsx! {
                        div { class: "format-toggle",
                            button {
                                class: if !use_unicode() { "format-btn active" } else { "format-btn" },
                                onclick: move |_| use_unicode.set(false),
                                "Simple"
                            }
                            button {
                                class: if use_unicode() { "format-btn active" } else { "format-btn" },
                                onclick: move |_| use_unicode.set(true),
                                "Unicode"
                            }
                        }
                        div { class: "milestone-output",
                            if use_unicode() { "{unicode}" } else { "{simple}" }
                        }
                    },
                    Output::Rust(code) => rsx! {
                        div { class: "format-toggle",
                            span { class: "format-btn active", "Rust" }
                        }
                        div { class: "milestone-output", "{code}" }
                    },
                    Output::Sva(code) => rsx! {
                        div { class: "format-toggle",
                            span { class: "format-btn active", "SVA" }
                        }
                        div { class: "milestone-output", "{code}" }
                    },
                }}
            }
        }
    }
}

/// The line that closes the Next release block: how many other changes and fixes `[Unreleased]`
/// holds beside its additions. `None` when it holds none.
pub fn next_release_summary(changed: usize, fixed: usize, since: &str) -> Option<String> {
    let counts: Vec<String> = [(changed, "change", "changes"), (fixed, "fix", "fixes")]
        .into_iter()
        .filter(|&(n, _, _)| n > 0)
        .map(|(n, one, many)| format!("{n} {}", if n == 1 { one } else { many }))
        .collect();
    if counts.is_empty() {
        return None;
    }
    Some(format!("Plus {} since v{since}.", counts.join(" and ")))
}

/// Each status's dot: done, being built, on the drawing board.
fn milestone_dot_icon(status: Status) -> IconVariant {
    match status {
        Status::Complete => IconVariant::Check,
        Status::InProgress => IconVariant::Hammer,
        Status::Planned => IconVariant::DraftingCompass,
    }
}

/// The glyph's colour: white on the filled dots, a muted slate on a planned card's sketch.
fn milestone_dot_color(status: Status) -> &'static str {
    match status {
        Status::Planned => "#cbd5e1",
        _ => "#fff",
    }
}

/// One card on the timeline. `next` is the status of the card below it, which the stretch of line
/// from this card's dot down to that card's fades into; the last card has none and draws no line.
#[component]
fn MilestoneItem(milestone: &'static Milestone, next: Option<Status>) -> Element {
    let status = milestone.status;
    let card_class = match next {
        Some(next) => format!(
            "milestone from-{} to-{}",
            status.css_class(),
            next.css_class()
        ),
        None => format!("milestone from-{}", status.css_class()),
    };
    let dot_class = format!("milestone-dot {}", status.css_class());
    let badge_class = format!("milestone-badge {}", status.css_class());
    let tag_class = match status {
        Status::Complete => "feature-tag done",
        _ => "feature-tag",
    };

    rsx! {
        div { class: "{card_class}",
            div { class: "{dot_class}",
                Icon { variant: milestone_dot_icon(status), size: IconSize::Small, color: milestone_dot_color(status) }
            }
            div { class: "milestone-content",
                div { class: "milestone-header",
                    span { class: "milestone-title", "{milestone.title}" }
                    span { class: "{badge_class}", "{status.label()}" }
                }
                p { class: "milestone-desc", "{milestone.description}" }
                div { class: "milestone-features",
                    for (i, feature) in milestone.features.iter().enumerate() {
                        span { key: "{i}", class: "{tag_class}", "{feature}" }
                    }
                }
                if !milestone.examples.is_empty() {
                    MilestoneExamples { examples: milestone.examples }
                }
            }
        }
    }
}

const RELEASE_HISTORY_STYLE: &str = r#"
.roadmap-section-title {
    max-width: 700px;
    margin: 8px auto 18px;
    padding: 0 20px;
    font-size: 13px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 1px;
    color: rgba(229,231,235,0.5);
}

.release-history {
    max-width: 700px;
    margin: 0 auto;
    padding: 0 20px 80px;
}

.next-release {
    margin: 0 0 20px;
    padding: 16px 18px;
    border-radius: 12px;
    border: 1px solid rgba(167,139,250,0.25);
    background: rgba(167,139,250,0.05);
}

.next-release-header {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 10px;
}

.next-release-title {
    font-weight: 700;
    font-size: 15px;
    color: #fff;
}

.next-release-list {
    margin: 0;
    padding-left: 18px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    color: rgba(229,231,235,0.85);
    font-size: 14px;
    line-height: 1.5;
}

.next-release-summary {
    margin: 10px 0 0;
    color: rgba(229,231,235,0.55);
    font-size: 13px;
}

.next-release-news {
    display: inline-block;
    margin-top: 10px;
    color: #a78bfa;
    font-size: 13px;
    text-decoration: none;
}

.next-release-news:hover {
    text-decoration: underline;
}

.release-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
}

.release-row {
    display: flex;
    align-items: baseline;
    gap: 14px;
    padding: 10px 14px;
    border-radius: 10px;
    border: 1px solid transparent;
    text-decoration: none;
    color: inherit;
    transition: background 0.15s ease, border-color 0.15s ease;
}

.release-row.linked {
    cursor: pointer;
}

.release-row.linked:hover {
    background: rgba(255,255,255,0.04);
    border-color: rgba(255,255,255,0.08);
}

.release-version {
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
    font-weight: 700;
    font-size: 13px;
    color: #a78bfa;
    min-width: 64px;
}

.release-date {
    color: rgba(229,231,235,0.45);
    font-size: 12px;
    min-width: 92px;
    font-variant-numeric: tabular-nums;
}

.release-title {
    flex: 1;
    color: rgba(229,231,235,0.85);
    font-size: 14px;
}

.release-unreleased {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: #a78bfa;
    border: 1px solid rgba(167,139,250,0.3);
    border-radius: 10px;
    padding: 2px 8px;
}

.release-arrow {
    color: rgba(229,231,235,0.25);
    font-size: 14px;
    transition: color 0.15s ease;
}

.release-row.linked:hover .release-arrow {
    color: #a78bfa;
}

.release-row.maintenance {
    opacity: 0.5;
}

.release-row.maintenance .release-version {
    color: rgba(167,139,250,0.6);
}

.maint-toggle {
    display: inline-block;
    margin: 0 0 14px;
    padding: 6px 12px;
    border-radius: 8px;
    border: 1px solid rgba(255,255,255,0.1);
    background: rgba(255,255,255,0.03);
    color: rgba(229,231,235,0.6);
    cursor: pointer;
    font-size: 12px;
    font-weight: 500;
    transition: background 0.15s ease, color 0.15s ease;
}

.maint-toggle:hover {
    background: rgba(255,255,255,0.06);
    color: #e5e7eb;
}

@media (max-width: 600px) {
    .release-date { display: none; }
}
"#;

#[component]
fn ReleaseRowBody(release: &'static Release, unreleased: bool) -> Element {
    rsx! {
        span { class: "release-version", "v{release.version}" }
        span { class: "release-date", "{release.date}" }
        span { class: "release-title", "{release.title}" }
        if unreleased {
            span { class: "release-unreleased", "Unreleased" }
        }
    }
}

#[component]
fn ReleaseRow(release: &'static Release, unreleased: bool) -> Element {
    let maint = if release.maintenance {
        " maintenance"
    } else {
        ""
    };
    match news_slug_for(&release.version) {
        Some(slug) => rsx! {
            Link {
                to: Route::NewsArticle { slug: slug.to_string() },
                class: "release-row linked{maint}",
                ReleaseRowBody { release, unreleased }
                span { class: "release-arrow", "→" }
            }
        },
        None => rsx! {
            div { class: "release-row{maint}",
                ReleaseRowBody { release, unreleased }
            }
        },
    }
}

/// `[Unreleased]` from the root CHANGELOG, as the build baked it (see `build.rs`): each addition's
/// headline, and how many other changes and fixes there are.
mod next {
    include!(concat!(env!("OUT_DIR"), "/roadmap_next.rs"));
}

/// What the next release holds so far, read off `[Unreleased]`, with the newest news article
/// written since the last release.
#[component]
fn NextRelease() -> Element {
    let last = get_history().first();
    let since = last.map(|r| r.version.as_str()).unwrap_or_default();
    let since_date = last.map(|r| r.date.as_str()).unwrap_or_default();
    let summary = next_release_summary(next::CHANGED, next::FIXED, since);
    let article = get_articles().into_iter().find(|a| a.date > since_date);

    rsx! {
        div { class: "next-release",
            div { class: "next-release-header",
                span { class: "next-release-title", "Next release" }
                span { class: "release-unreleased", "In progress" }
            }
            ul { class: "next-release-list",
                for (i, headline) in next::ADDED.iter().enumerate() {
                    li { key: "{i}", "{headline}" }
                }
            }
            if let Some(summary) = summary {
                p { class: "next-release-summary", "{summary}" }
            }
            if let Some(article) = article {
                Link {
                    to: Route::NewsArticle { slug: article.slug.to_string() },
                    class: "next-release-news",
                    "Read: {article.title} →"
                }
            }
        }
    }
}

#[component(lazy)]
pub fn Roadmap() -> Element {
    let breadcrumbs = vec![
        BreadcrumbItem {
            name: "Home",
            path: "/",
        },
        BreadcrumbItem {
            name: "Roadmap",
            path: "/roadmap",
        },
    ];
    let schemas = vec![
        organization_schema(),
        roadmap_schema(),
        breadcrumb_schema(&breadcrumbs),
    ];

    // Releases are newest-first; the ones above the newest git tag are genuinely
    // unreleased (prepared, not yet cut). Older untagged releases predate tagging
    // and were released, so they are not badged.
    let milestones = get_milestones();
    let history = get_history();
    let first_tagged = history
        .iter()
        .position(|r| r.tagged)
        .unwrap_or(history.len());
    let maint_count = history.iter().filter(|r| r.maintenance).count();
    let mut show_maint = use_signal(|| false);

    rsx! {
        PageHead {
            title: seo_pages::ROADMAP.title,
            description: seo_pages::ROADMAP.description,
            canonical_path: seo_pages::ROADMAP.canonical_path,
        }
        style { "{ROADMAP_STYLE}" }
        style { "{RELEASE_HISTORY_STYLE}" }
        JsonLdMultiple { schemas }

        div { class: "roadmap-container",
            MainNav { active: ActivePage::Roadmap, subtitle: Some("Where we're headed") }

            section { class: "roadmap-hero",
                h1 { "LOGOS Roadmap" }
                span { class: "version", "v0.11.0" }
                p { "From English sentences to a verified compilation stack: first-order logic, full-stack websites, native, GPU and WebAssembly backends, hardware model checking, a kernel-certified proof core, and circuits compiled from the program itself." }
            }

            h2 { class: "roadmap-section-title", "Featured milestones" }
            div { class: "timeline",
                for (i, milestone) in milestones.iter().enumerate() {
                    MilestoneItem { key: "{i}", milestone, next: milestones.get(i + 1).map(|m| m.status) }
                }
            }

            h2 { class: "roadmap-section-title", "Release history" }
            section { class: "release-history",
                NextRelease {}
                if maint_count > 0 {
                    button {
                        class: "maint-toggle",
                        onclick: move |_| { let on = show_maint(); show_maint.set(!on); },
                        if show_maint() {
                            "Hide maintenance releases"
                        } else {
                            "Show {maint_count} maintenance releases (CI, benchmarks, tooling)"
                        }
                    }
                }
                div { class: "release-list",
                    for (i, release) in history.iter().enumerate() {
                        if !release.maintenance || show_maint() {
                            ReleaseRow { key: "{i}", release, unreleased: i < first_tagged }
                        }
                    }
                }
            }

            Footer {}
        }
    }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/roadmap/tests.rs"]
mod tests;
