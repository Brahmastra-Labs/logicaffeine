//! Package detail page.
//!
//! Displays comprehensive information about a single package including:
//!
//! - Package metadata (name, author, description)
//! - README content (rendered markdown)
//! - Version history with changelogs
//! - Dependency graph
//! - Installation instructions
//!
//! # Route
//!
//! Accessed via [`Route::PackageDetail`]
//! with `name` and `version` parameters.

use dioxus::prelude::*;
use crate::ui::router::Route;
use crate::ui::state::PackageDetails;
use crate::ui::components::main_nav::{MainNav, ActivePage};
use crate::ui::seo::PageHead;

const REGISTRY_API_URL: &str = "https://registry.logicaffeine.com";

const DETAIL_STYLE: &str = r#"
.detail-container {
    min-height: 100vh;
    background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%);
    color: #e8e8e8;
    font-family: 'Inter', -apple-system, BlinkMacSystemFont, sans-serif;
}

.detail-header {
    padding: 24px 48px;
    border-bottom: 1px solid rgba(255,255,255,0.1);
}

.back-link {
    color: #888;
    text-decoration: none;
    font-size: 14px;
    margin-bottom: 16px;
    display: inline-block;
}

.back-link:hover {
    color: #fff;
}

.package-title {
    display: flex;
    align-items: center;
    gap: 16px;
    margin-bottom: 8px;
}

.package-title h1 {
    font-size: 32px;
    font-weight: 700;
    margin: 0;
}

.verified-badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: linear-gradient(135deg, #22c55e, #16a34a);
    color: white;
    font-size: 12px;
    font-weight: 700;
    padding: 4px 12px;
    border-radius: 999px;
}

.package-meta {
    display: flex;
    gap: 24px;
    color: #888;
    font-size: 14px;
}

.package-meta a {
    color: #667eea;
    text-decoration: none;
}

.package-meta a:hover {
    text-decoration: underline;
}

.detail-content {
    display: grid;
    grid-template-columns: 1fr 320px;
    gap: 48px;
    padding: 48px;
}

.main-content {
    min-width: 0;
}

.tab-nav {
    display: flex;
    gap: 4px;
    margin-bottom: 24px;
    border-bottom: 1px solid rgba(255,255,255,0.1);
    padding-bottom: 0;
}

.tab-btn {
    padding: 12px 24px;
    background: transparent;
    border: none;
    color: #888;
    font-size: 14px;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
    transition: color 0.2s, border-color 0.2s;
}

.tab-btn:hover {
    color: #fff;
}

.tab-btn.active {
    color: #667eea;
    border-bottom-color: #667eea;
}

.readme-content {
    background: rgba(255,255,255,0.03);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: 12px;
    padding: 32px;
    line-height: 1.7;
}

.readme-content h1, .readme-content h2, .readme-content h3 {
    color: #fff;
    margin-top: 24px;
    margin-bottom: 12px;
}

.readme-content code {
    background: rgba(255,255,255,0.1);
    padding: 2px 6px;
    border-radius: 4px;
    font-size: 0.9em;
}

.readme-content pre {
    background: rgba(0,0,0,0.3);
    padding: 16px;
    border-radius: 8px;
    overflow-x: auto;
}

.versions-list {
    background: rgba(255,255,255,0.03);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: 12px;
    overflow: hidden;
}

.version-item {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 16px 24px;
    border-bottom: 1px solid rgba(255,255,255,0.05);
}

.version-item:last-child {
    border-bottom: none;
}

.version-name {
    font-weight: 600;
    color: #fff;
}

.version-meta {
    display: flex;
    gap: 16px;
    font-size: 13px;
    color: #666;
}

.yanked-badge {
    background: rgba(239, 68, 68, 0.2);
    color: #ef4444;
    padding: 2px 8px;
    border-radius: 4px;
    font-size: 11px;
}

.sidebar {
    position: sticky;
    top: 24px;
}

.sidebar-section {
    background: rgba(255,255,255,0.03);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: 12px;
    padding: 24px;
    margin-bottom: 24px;
}

.sidebar-section h3 {
    font-size: 14px;
    font-weight: 600;
    color: #888;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin: 0 0 16px;
}

.install-cmd {
    display: block;
    background: rgba(0,0,0,0.3);
    padding: 12px 16px;
    border-radius: 8px;
    font-family: monospace;
    font-size: 14px;
    color: #22c55e;
    word-break: break-all;
}

.meta-item {
    display: flex;
    justify-content: space-between;
    padding: 8px 0;
    font-size: 14px;
    border-bottom: 1px solid rgba(255,255,255,0.05);
}

.meta-item:last-child {
    border-bottom: none;
}

.meta-label {
    color: #888;
}

.meta-value {
    color: #fff;
}

.meta-value a {
    color: #667eea;
    text-decoration: none;
}

.meta-value a:hover {
    text-decoration: underline;
}

.loading-spinner {
    text-align: center;
    padding: 48px;
    color: #888;
}

.error-message {
    text-align: center;
    padding: 48px;
    color: #f87171;
}

.keywords-list {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
}

.keyword-tag {
    font-size: 12px;
    padding: 4px 10px;
    background: rgba(102, 126, 234, 0.15);
    color: #667eea;
    border-radius: 4px;
}
"#;

#[derive(Clone, PartialEq)]
enum DetailTab {
    Readme,
    Versions,
}

#[component]
pub fn PackageDetail(name: String) -> Element {
    let mut details = use_signal(|| None::<PackageDetails>);
    let mut error = use_signal(|| None::<String>);
    let mut is_loading = use_signal(|| true);
    let mut active_tab = use_signal(|| DetailTab::Readme);

    // Fetch package details
    use_effect({
        let name = name.clone();
        move || {
            let name = name.clone();
            spawn(async move {
                is_loading.set(true);
                match fetch_package_details(&name).await {
                    Ok(d) => details.set(Some(d)),
                    Err(e) => error.set(Some(e)),
                }
                is_loading.set(false);
            });
        }
    });

    let page_title = format!("{} - LOGICAFFEINE Registry", name);
    let page_path = format!("/registry/package/{}", name);

    rsx! {
        PageHead {
            title: page_title,
            description: "Package details and documentation on the LOGICAFFEINE package registry.",
            canonical_path: page_path,
        }
        style { "{DETAIL_STYLE}" }

        MainNav { active: ActivePage::Registry, subtitle: Some("Package Details") }

        div { class: "detail-container",
            if *is_loading.read() {
                div { class: "loading-spinner", "Loading package..." }
            } else if let Some(err) = error.read().as_ref() {
                div { class: "error-message",
                    p { "Error: {err}" }
                }
            } else if let Some(pkg) = details.read().as_ref() {
                // Header
                header { class: "detail-header",
                    div { class: "package-title",
                        h1 { "{pkg.name}" }
                        if pkg.verified {
                            span { class: "verified-badge", "Official" }
                        }
                    }
                    div { class: "package-meta",
                        span { "by {pkg.owner}" }
                        if let Some(repo) = &pkg.repository {
                            a { href: "{repo}", target: "_blank", "Repository" }
                        }
                        if let Some(license) = &pkg.license {
                            span { "{license}" }
                        }
                    }
                }

                // Content
                div { class: "detail-content",
                    main { class: "main-content",
                        nav { class: "tab-nav",
                            button {
                                class: if *active_tab.read() == DetailTab::Readme { "tab-btn active" } else { "tab-btn" },
                                onclick: move |_| active_tab.set(DetailTab::Readme),
                                "README"
                            }
                            button {
                                class: if *active_tab.read() == DetailTab::Versions { "tab-btn active" } else { "tab-btn" },
                                onclick: move |_| active_tab.set(DetailTab::Versions),
                                "Versions ({pkg.versions.len()})"
                            }
                        }

                        match *active_tab.read() {
                            DetailTab::Readme => rsx! {
                                div { class: "readme-content",
                                    if let Some(readme) = &pkg.readme {
                                        // Note: In production, render markdown properly
                                        pre { "{readme}" }
                                    } else {
                                        p { "No README available." }
                                    }
                                }
                            },
                            DetailTab::Versions => rsx! {
                                div { class: "versions-list",
                                    for version in pkg.versions.iter() {
                                        div { class: "version-item",
                                            span { class: "version-name",
                                                "v{version.version}"
                                                if version.yanked {
                                                    span { class: "yanked-badge", "yanked" }
                                                }
                                            }
                                            div { class: "version-meta",
                                                span { "{format_size(version.size)}" }
                                                span { "{version.published_at}" }
                                            }
                                        }
                                    }
                                }
                            },
                        }
                    }

                    aside { class: "sidebar",
                        div { class: "sidebar-section",
                            h3 { "Install" }
                            code { class: "install-cmd", "largo add {pkg.name}" }
                        }

                        div { class: "sidebar-section",
                            h3 { "Details" }
                            div { class: "meta-item",
                                span { class: "meta-label", "Downloads" }
                                span { class: "meta-value", "{pkg.downloads}" }
                            }
                            if !pkg.versions.is_empty() {
                                div { class: "meta-item",
                                    span { class: "meta-label", "Latest" }
                                    span { class: "meta-value", "v{pkg.versions[0].version}" }
                                }
                            }
                            if let Some(homepage) = &pkg.homepage {
                                div { class: "meta-item",
                                    span { class: "meta-label", "Homepage" }
                                    span { class: "meta-value",
                                        a { href: "{homepage}", target: "_blank", "Link" }
                                    }
                                }
                            }
                        }

                        if !pkg.keywords.is_empty() {
                            div { class: "sidebar-section",
                                h3 { "Keywords" }
                                div { class: "keywords-list",
                                    for keyword in pkg.keywords.iter() {
                                        span { class: "keyword-tag", "{keyword}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

async fn fetch_package_details(name: &str) -> Result<PackageDetails, String> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;

        let url = format!("{}/packages/{}", REGISTRY_API_URL, name);

        let response = Request::get(&url)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !response.ok() {
            return Err("Package not found".to_string());
        }

        response.json().await.map_err(|e| e.to_string())
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        Err("Not available in non-WASM builds".to_string())
    }
}
