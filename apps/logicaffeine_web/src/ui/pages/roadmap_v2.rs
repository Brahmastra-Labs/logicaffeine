//! New roadmap page (preview at `/roadmap-new`).
//!
//! Two tiers: the curated [`get_milestones`](super::roadmap_data::get_milestones)
//! capability cards on top (reusing [`MilestoneItem`](super::roadmap::MilestoneItem)),
//! and a terse, scrollable release history below, generated from `CHANGELOG.md`
//! into [`roadmap_history`](super::roadmap_history). Each release links to its
//! news article when one exists. Lives alongside the current `/roadmap` so the
//! redesign can be reviewed before the swap.

use dioxus::prelude::*;
use crate::ui::components::main_nav::{MainNav, ActivePage};
use crate::ui::components::footer::Footer;
use crate::ui::pages::roadmap::{ROADMAP_STYLE, MilestoneItem};
use crate::ui::pages::roadmap_data::get_milestones;
use crate::ui::pages::roadmap_history::{get_history, news_slug_for, Release};
use crate::ui::router::Route;
use crate::ui::seo::{JsonLdMultiple, PageHead, organization_schema, roadmap_schema, breadcrumb_schema, BreadcrumbItem, pages as seo_pages};

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
    let maint = if release.maintenance { " maintenance" } else { "" };
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

#[component]
pub fn RoadmapNew() -> Element {
    let breadcrumbs = vec![
        BreadcrumbItem { name: "Home", path: "/" },
        BreadcrumbItem { name: "Roadmap", path: "/roadmap-new" },
    ];
    let schemas = vec![
        organization_schema(),
        roadmap_schema(),
        breadcrumb_schema(&breadcrumbs),
    ];

    // Releases are newest-first; the ones above the newest git tag are genuinely
    // unreleased (prepared, not yet cut). Older untagged releases predate tagging
    // and were released, so they are not badged.
    let history = get_history();
    let first_tagged = history.iter().position(|r| r.tagged).unwrap_or(history.len());
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
            MainNav { active: ActivePage::RoadmapNew, subtitle: Some("Where we're headed") }

            section { class: "roadmap-hero",
                h1 { "LOGOS Roadmap" }
                span { class: "version", "v0.9.17" }
                p { "From English sentences to a verified compilation stack: first-order logic, a native execution tier, hardware model checking, and a kernel-certified proof core." }
            }

            h2 { class: "roadmap-section-title", "Featured milestones" }
            div { class: "timeline",
                for (i, milestone) in get_milestones().iter().enumerate() {
                    MilestoneItem { key: "{i}", milestone }
                }
            }

            h2 { class: "roadmap-section-title", "Release history" }
            section { class: "release-history",
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
