//! Package registry browse page.
//!
//! Main page for discovering and searching LOGOS packages. Features:
//!
//! - Package search with fuzzy matching
//! - Category filtering
//! - GitHub OAuth authentication for publishing
//! - Package cards with download counts and versions
//!
//! # Authentication
//!
//! Uses GitHub OAuth for publisher authentication. Users can publish packages
//! under their GitHub username namespace.
//!
//! # Route
//!
//! Accessed via [`Route::Registry`].

use dioxus::prelude::*;
use crate::ui::router::Route;
use crate::ui::state::{RegistryAuthState, RegistryPackage, GitHubUser};
use crate::ui::components::main_nav::{MainNav, ActivePage};

const REGISTRY_API_URL: &str = "https://registry.logicaffeine.com";

const REGISTRY_STYLE: &str = r#"
.registry-container {
    min-height: 100vh;
    background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%);
    color: #e8e8e8;
    font-family: 'Inter', -apple-system, BlinkMacSystemFont, sans-serif;
}

.registry-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 24px 48px;
    border-bottom: 1px solid rgba(255,255,255,0.1);
}

.header-left {
    display: flex;
    align-items: center;
    gap: 24px;
}

.header-left h1 {
    font-size: 24px;
    font-weight: 700;
    background: linear-gradient(135deg, #667eea, #764ba2);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    margin: 0;
}

.back-link {
    color: #888;
    text-decoration: none;
    font-size: 14px;
}

.back-link:hover {
    color: #fff;
}

.login-btn {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 10px 20px;
    background: #24292e;
    color: white;
    text-decoration: none;
    border-radius: 8px;
    font-size: 14px;
    font-weight: 500;
    transition: background 0.2s;
}

.login-btn:hover {
    background: #2f363d;
}

.user-menu {
    display: flex;
    align-items: center;
    gap: 12px;
}

.user-avatar {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    border: 2px solid rgba(255,255,255,0.2);
}

.user-name {
    font-size: 14px;
    color: #e8e8e8;
}

.logout-btn {
    padding: 6px 12px;
    background: transparent;
    border: 1px solid rgba(255,255,255,0.2);
    color: #888;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
}

.logout-btn:hover {
    border-color: rgba(255,255,255,0.4);
    color: #fff;
}

.search-section {
    padding: 48px;
    text-align: center;
}

.search-title {
    font-size: 32px;
    font-weight: 700;
    margin-bottom: 8px;
}

.search-subtitle {
    color: #888;
    margin-bottom: 32px;
}

.search-bar {
    width: 100%;
    max-width: 600px;
    padding: 16px 24px;
    font-size: 16px;
    background: rgba(255,255,255,0.05);
    border: 1px solid rgba(255,255,255,0.1);
    border-radius: 12px;
    color: #fff;
    outline: none;
    transition: border-color 0.2s, box-shadow 0.2s;
}

.search-bar:focus {
    border-color: #667eea;
    box-shadow: 0 0 0 3px rgba(102, 126, 234, 0.2);
}

.search-bar::placeholder {
    color: #666;
}

.package-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
    gap: 24px;
    padding: 0 48px 48px;
}

.package-card {
    background: rgba(255,255,255,0.03);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: 12px;
    padding: 24px;
    text-decoration: none;
    color: inherit;
    transition: transform 0.2s, border-color 0.2s, background 0.2s;
}

.package-card:hover {
    transform: translateY(-2px);
    border-color: rgba(102, 126, 234, 0.4);
    background: rgba(255,255,255,0.05);
}

.package-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    margin-bottom: 12px;
}

.package-name {
    font-size: 18px;
    font-weight: 600;
    color: #fff;
    margin: 0;
    display: flex;
    align-items: center;
    gap: 8px;
}

.verified-badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: linear-gradient(135deg, #22c55e, #16a34a);
    color: white;
    font-size: 11px;
    font-weight: 700;
    padding: 3px 8px;
    border-radius: 999px;
}

.package-version {
    font-size: 13px;
    color: #888;
    background: rgba(255,255,255,0.05);
    padding: 4px 8px;
    border-radius: 4px;
}

.package-description {
    color: #aaa;
    font-size: 14px;
    line-height: 1.5;
    margin-bottom: 16px;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
}

.package-meta {
    display: flex;
    gap: 16px;
    font-size: 13px;
    color: #666;
}

.package-meta span {
    display: flex;
    align-items: center;
    gap: 4px;
}

.package-keywords {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    margin-top: 12px;
}

.keyword-tag {
    font-size: 11px;
    padding: 3px 8px;
    background: rgba(102, 126, 234, 0.15);
    color: #667eea;
    border-radius: 4px;
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

.empty-state {
    text-align: center;
    padding: 48px;
    color: #888;
}

.stats-section {
    display: flex;
    justify-content: center;
    gap: 48px;
    padding: 24px 48px;
    border-bottom: 1px solid rgba(255,255,255,0.1);
}

.stat-item {
    text-align: center;
}

.stat-value {
    font-size: 24px;
    font-weight: 700;
    color: #667eea;
}

.stat-label {
    font-size: 13px;
    color: #666;
}
"#;

#[component]
pub fn Registry() -> Element {
    let mut auth_state = use_context::<RegistryAuthState>();
    let auth_state_for_check = auth_state.clone();
    let mut packages = use_signal(Vec::<RegistryPackage>::new);
    let mut search_query = use_signal(String::new);
    let mut is_loading = use_signal(|| true);
    let mut error = use_signal(|| None::<String>);

    // Check for OAuth callback params in URL
    use_effect(move || {
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(window) = web_sys::window() {
                if let Ok(search) = window.location().search() {
                    if let Ok(params) = web_sys::UrlSearchParams::new_with_str(&search) {
                        if let Some(token) = params.get("token") {
                            if let Some(login) = params.get("login") {
                                // Login successful
                                let user = GitHubUser {
                                    id: String::new(),
                                    login: login.clone(),
                                    name: None,
                                    avatar_url: None,
                                };
                                auth_state.login(token, user);

                                // Clear URL params
                                if let Ok(history) = window.history() {
                                    let _ = history.replace_state_with_url(
                                        &wasm_bindgen::JsValue::NULL,
                                        "",
                                        Some("/registry"),
                                    );
                                }
                            }
                        }

                        if let Some(err) = params.get("error") {
                            error.set(Some(err));
                            // Clear URL params
                            if let Ok(history) = window.history() {
                                let _ = history.replace_state_with_url(
                                    &wasm_bindgen::JsValue::NULL,
                                    "",
                                    Some("/registry"),
                                );
                            }
                        }
                    }
                }
            }
        }
    });

    // Fetch packages
    use_effect(move || {
        spawn(async move {
            is_loading.set(true);
            match fetch_packages(None).await {
                Ok(pkgs) => packages.set(pkgs),
                Err(e) => error.set(Some(e)),
            }
            is_loading.set(false);
        });
    });

    let filtered_packages: Vec<RegistryPackage> = {
        let query = search_query.read().to_lowercase();
        if query.is_empty() {
            packages.read().clone()
        } else {
            packages
                .read()
                .iter()
                .filter(|p| {
                    p.name.to_lowercase().contains(&query)
                        || p.description
                            .as_ref()
                            .map(|d| d.to_lowercase().contains(&query))
                            .unwrap_or(false)
                        || p.keywords.iter().any(|k| k.to_lowercase().contains(&query))
                })
                .cloned()
                .collect()
        }
    };

    rsx! {
        style { "{REGISTRY_STYLE}" }

        div { class: "registry-container",
            MainNav {
                active: ActivePage::Registry,
                subtitle: Some("Package Registry"),
            }

            // Registry auth section (kept separate from main nav)
            header { class: "registry-header",
                div { class: "header-left",
                    h1 { "Package Registry" }
                }
                div { class: "header-right",
                    if auth_state_for_check.is_authenticated() {
                        UserMenu { auth_state: auth_state_for_check.clone() }
                    } else {
                        a {
                            class: "login-btn",
                            href: "{RegistryAuthState::get_auth_url()}",
                            // GitHub icon
                            svg {
                                width: "20",
                                height: "20",
                                view_box: "0 0 24 24",
                                fill: "currentColor",
                                path {
                                    d: "M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z"
                                }
                            }
                            "Login with GitHub"
                        }
                    }
                }
            }

            div { class: "search-section",
                h2 { class: "search-title", "Find LOGOS Packages" }
                p { class: "search-subtitle", "Discover libraries to enhance your logic programs" }
                input {
                    class: "search-bar",
                    r#type: "search",
                    placeholder: "Search packages by name, description, or keyword...",
                    value: "{search_query}",
                    oninput: move |e| search_query.set(e.value()),
                }
            }

            if *is_loading.read() {
                div { class: "loading-spinner", "Loading packages..." }
            } else if let Some(err) = error.read().as_ref() {
                div { class: "error-message", "Error: {err}" }
            } else if filtered_packages.is_empty() {
                div { class: "empty-state",
                    if search_query.read().is_empty() {
                        "No packages published yet. Be the first!"
                    } else {
                        "No packages match your search."
                    }
                }
            } else {
                div { class: "package-grid",
                    for package in filtered_packages {
                        PackageCard { package: package }
                    }
                }
            }
        }
    }
}

#[component]
fn PackageCard(package: RegistryPackage) -> Element {
    rsx! {
        Link {
            to: Route::PackageDetail { name: package.name.clone() },
            class: "package-card",
            div { class: "package-header",
                h3 { class: "package-name",
                    "{package.name}"
                    if package.verified {
                        span { class: "verified-badge", "Official" }
                    }
                }
                if let Some(version) = &package.latest_version {
                    span { class: "package-version", "v{version}" }
                }
            }
            p { class: "package-description",
                "{package.description.as_deref().unwrap_or(\"No description\")}"
            }
            div { class: "package-meta",
                span { "{package.downloads} downloads" }
                span { "by {package.owner}" }
            }
            if !package.keywords.is_empty() {
                div { class: "package-keywords",
                    for keyword in package.keywords.iter().take(3) {
                        span { class: "keyword-tag", "{keyword}" }
                    }
                }
            }
        }
    }
}

#[component]
fn UserMenu(auth_state: RegistryAuthState) -> Element {
    let user = auth_state.user.read().clone();
    let auth_for_logout = auth_state.clone();

    rsx! {
        div { class: "user-menu",
            if let Some(u) = user.as_ref() {
                if let Some(avatar) = &u.avatar_url {
                    img {
                        class: "user-avatar",
                        src: "{avatar}",
                        alt: "{u.login}"
                    }
                }
                span { class: "user-name", "{u.login}" }
            }
            button {
                class: "logout-btn",
                onclick: move |_| {
                    let mut auth = auth_for_logout.clone();
                    auth.logout();
                },
                "Logout"
            }
        }
    }
}

async fn fetch_packages(search: Option<&str>) -> Result<Vec<RegistryPackage>, String> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;

        let url = match search {
            Some(q) if !q.is_empty() => format!("{}/packages?search={}", REGISTRY_API_URL, q),
            _ => format!("{}/packages", REGISTRY_API_URL),
        };

        let response = Request::get(&url)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !response.ok() {
            return Err("Failed to fetch packages".to_string());
        }

        #[derive(serde::Deserialize)]
        struct PackagesResponse {
            packages: Vec<RegistryPackage>,
        }

        let data: PackagesResponse = response.json().await.map_err(|e| e.to_string())?;
        Ok(data.packages)
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        Ok(vec![])
    }
}
