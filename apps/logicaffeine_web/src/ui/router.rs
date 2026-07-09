//! Application routing and navigation.
//!
//! Defines all routes for the Logicaffeine web application using the Dioxus
//! router. Each route maps to a page component in [`crate::ui::pages`].
//!
//! # Routes
//!
//! | Path | Component | Description |
//! |------|-----------|-------------|
//! | `/` | [`Landing`] | Marketing homepage |
//! | `/learn` | [`Learn`] | Main learning interface with curriculum |
//! | `/studio?:file` | [`Studio`] | Playground for experimentation |
//! | `/profile` | [`Profile`] | User settings and progress |
//! | `/pricing` | [`Pricing`] | Subscription plans |
//! | `/guide` | [`Guide`] | Documentation and tutorials |
//! | `/registry?:token&:login&:error&:q` | [`Registry`] | Package browser |
//! | `/workspace/:subject` | [`Workspace`] | Subject-specific workspace |
//!
//! # Navigation
//!
//! Use the Dioxus `Link` component with `Route` variants:
//!
//! ```no_run
//! # use dioxus::prelude::*;
//! use logicaffeine_web::ui::router::Route;
//!
//! # fn Example() -> Element {
//! rsx! {
//!     Link { to: Route::Learn {}, "Start Learning" }
//!     Link { to: Route::Studio { file: None }, "Open Studio" }
//! }
//! # }
//! ```
//!
//! # Query parameters are part of the route type
//!
//! On startup the router parses the browser URL into [`Route`], serializes it
//! back, and replaces the browser URL with the serialization whenever the two
//! differ. Any URL state the route type does not model is silently destroyed
//! before page components can read it. Pages therefore MUST receive query
//! parameters as route props (declared with `?:name` in the `#[route]`
//! attribute) — never by scraping the browser location's search string; the
//! `query_scraping_is_forbidden` lock enforces this.

use dioxus::prelude::*;
use crate::ui::pages::{Landing, Learn, Pricing, Privacy, Profile, Roadmap, Success, Terms, Workspace, Studio, Guide, Crates, News, NewsArticle, Benchmarks};
use crate::ui::pages::registry::{Registry, PackageDetail};

/// Application routes.
///
/// This enum is derived with `Routable` to generate route matching logic.
/// Each variant corresponds to a URL pattern and its associated page component.
#[derive(Clone, Routable, Debug, PartialEq)]
pub enum Route {
    /// Marketing homepage at `/`.
    #[route("/")]
    Landing {},

    /// Contact and licensing at `/pricing`.
    #[route("/pricing")]
    Pricing {},

    /// Privacy policy at `/privacy`.
    #[route("/privacy")]
    Privacy {},

    /// Terms of service at `/terms`.
    #[route("/terms")]
    Terms {},

    /// Product roadmap at `/roadmap`.
    #[route("/roadmap")]
    Roadmap {},

    /// Documentation and tutorials at `/guide`.
    #[route("/guide")]
    Guide {},

    /// Crate documentation at `/crates`.
    #[route("/crates")]
    Crates {},

    /// Post-checkout success page at `/success`.
    ///
    /// Stripe redirects here with a `session_id` query parameter that the page
    /// exchanges for a license key.
    #[route("/success?:session_id")]
    Success {
        /// Stripe checkout session id from the redirect, if any.
        session_id: Option<String>,
    },

    /// Playground for experimentation at `/studio`.
    ///
    /// Deep links carry the VFS path of the file to open as a `file` query
    /// parameter (without the leading slash), e.g.
    /// `/studio?file=examples/logic/prover-demo.logic`.
    #[route("/studio?:file")]
    Studio {
        /// VFS path of the file to open, if any.
        file: Option<String>,
    },

    /// Main learning interface at `/learn`.
    ///
    /// All learning happens here - curriculum browsing, exercises, and review.
    #[route("/learn")]
    Learn {},

    /// User profile and settings at `/profile`.
    #[route("/profile")]
    Profile {},

    /// Subject-specific workspace at `/workspace/:subject`.
    #[route("/workspace/:subject")]
    Workspace {
        /// The workspace subject identifier (e.g., "logic", "proofs").
        subject: String,
    },

    /// Package registry browser at `/registry`.
    ///
    /// The GitHub OAuth callback redirects here with `token`/`login` on
    /// success or `error` on failure; `q` pre-fills the package search (the
    /// SearchAction URL advertised in the JSON-LD).
    #[route("/registry?:token&:login&:error&:q")]
    Registry {
        /// OAuth access token from the callback, if any.
        token: Option<String>,
        /// GitHub login name from the callback, if any.
        login: Option<String>,
        /// OAuth error message from the callback, if any.
        error: Option<String>,
        /// Package search query to pre-fill, if any.
        q: Option<String>,
    },

    /// Package detail page at `/registry/package/:name`.
    #[route("/registry/package/:name")]
    PackageDetail {
        /// The package name to display.
        name: String,
    },

    /// Performance benchmarks at `/benchmarks`.
    #[route("/benchmarks")]
    Benchmarks {},

    /// News index page at `/news`.
    ///
    /// An optional `tag` query parameter pre-applies a tag filter.
    #[route("/news?:tag")]
    News {
        /// Tag to filter articles by, if any.
        tag: Option<String>,
    },

    /// News article page at `/news/:slug`.
    #[route("/news/:slug")]
    NewsArticle {
        /// The article slug.
        slug: String,
    },

    /// Catch-all for unknown routes, renders 404 page.
    #[route("/:..route")]
    NotFound {
        /// Path segments that didn't match any route.
        route: Vec<String>,
    },
}

/// The canonical shareable URL for a studio file.
///
/// Built from [`Route::Studio`] itself so link writers can never drift from
/// what the router parses and re-serializes at boot. Accepts the VFS path
/// with or without its leading slash.
pub fn studio_file_url(vfs_path: &str) -> String {
    Route::Studio {
        file: Some(vfs_path.trim_start_matches('/').to_string()),
    }
    .to_string()
}

/// Replace the address-bar URL in place — no history entry, no router
/// navigation. Pages use this to keep the bar on the canonical form of the
/// page they are showing (e.g. after consuming one-shot query params, or
/// because route serialization leaves a bare `?` when all params are absent).
#[cfg(target_arch = "wasm32")]
pub fn replace_bar_url(url: &str) {
    if let Some(window) = web_sys::window() {
        if let Ok(history) = window.history() {
            let _ = history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(url));
        }
    }
}

/// The boot-normalization invariant: on startup the router parses the browser
/// URL into [`Route`], serializes it back, and *replaces the browser URL* with
/// the serialization whenever the two differ. Any URL state the route type
/// does not model is silently destroyed before page components can read it —
/// these tests lock every query-carrying route to a lossless round-trip.
#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[track_caller]
    fn roundtrip(url: &str) -> String {
        let route = Route::from_str(url)
            .unwrap_or_else(|e| panic!("{url} failed to parse: {e}"));
        route.to_string()
    }

    #[test]
    fn studio_deep_link_survives_roundtrip() {
        let serialized = roundtrip("/studio?file=examples/logic/prover-demo.logic");
        assert!(
            serialized.contains("file=examples/logic/prover-demo.logic"),
            "studio file param must survive boot normalization, got {serialized}"
        );

        let route = Route::from_str("/studio?file=examples/logic/prover-demo.logic").unwrap();
        assert_eq!(
            route,
            Route::Studio {
                file: Some("examples/logic/prover-demo.logic".to_string())
            }
        );
        assert_eq!(
            Route::from_str("/studio").unwrap(),
            Route::Studio { file: None },
            "plain /studio must still parse"
        );
    }

    #[test]
    fn studio_file_url_is_canonical() {
        let url = studio_file_url("/examples/logic/prover-demo.logic");
        assert_eq!(url, "/studio?file=examples/logic/prover-demo.logic");
        assert_eq!(url, studio_file_url("examples/logic/prover-demo.logic"));

        let route = Route::from_str(&url).expect("canonical studio URL parses");
        assert_eq!(
            route,
            Route::Studio {
                file: Some("examples/logic/prover-demo.logic".to_string())
            }
        );
        assert_eq!(route.to_string(), url, "writer and router agree byte-for-byte");
    }

    /// The pit-of-success locks over the whole web app source. Read side: no
    /// page may scrape the query string from the browser location — the
    /// router destroys unmodeled query params at boot, so scraping is always
    /// a latent deep-link bug; query state must be declared on the route
    /// (`?:name`) and received as props. Write side: nobody may hand-build a
    /// query URL — links come from the `Route` type ([`studio_file_url`],
    /// `Link { to: Route::… }`) so writers can never drift from what the
    /// router parses.
    #[test]
    fn query_scraping_is_forbidden() {
        // Needles assembled at runtime so this test's own source never matches.
        // (needle, files exempt from it)
        let readers_forbidden_everywhere: &[String] = &[
            format!("location(){}", ".search()"),
            format!("location{}", ".search()"),
            format!("UrlSearch{}", "Params"),
        ];
        let writers_allowed_only_in_router: &[String] = &[
            format!("studio?{}", "file="),
            format!("news?{}", "tag="),
            format!("registry?{}", "q="),
            format!("success?{}", "session_id="),
        ];

        let src_root = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
        let mut offenders = Vec::new();
        let mut stack = vec![std::path::PathBuf::from(src_root)];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("readable src dir") {
                let path = entry.expect("readable dir entry").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !path.extension().is_some_and(|e| e == "rs") {
                    continue;
                }
                let source = std::fs::read_to_string(&path).expect("readable source file");
                for needle in readers_forbidden_everywhere {
                    if source.contains(needle.as_str()) {
                        offenders.push(format!(
                            "{} scrapes the query string (`{needle}`) — declare the \
                             parameter on the route (`?:name`) and take it as a prop",
                            path.display()
                        ));
                    }
                }
                // router.rs is the one place allowed to spell query-URL shapes
                // (route definitions, canonical builders, these tests); seo.rs
                // holds the JSON-LD SearchAction template, whose `{search_term}`
                // placeholder cannot be built from the Route type.
                if path.ends_with("router.rs") || path.ends_with("seo.rs") {
                    continue;
                }
                for needle in writers_allowed_only_in_router {
                    if source.contains(needle.as_str()) {
                        offenders.push(format!(
                            "{} hand-builds a query URL (`{needle}`) — build it from \
                             the Route type (studio_file_url / Link {{ to: Route::… }})",
                            path.display()
                        ));
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "query-string handling outside the route type:\n{}",
            offenders.join("\n")
        );
    }

    #[test]
    fn news_tag_survives_roundtrip() {
        let serialized = roundtrip("/news?tag=releases");
        assert!(
            serialized.contains("tag=releases"),
            "news tag param must survive boot normalization, got {serialized}"
        );
    }

    #[test]
    fn success_session_id_survives_roundtrip() {
        let serialized = roundtrip("/success?session_id=cs_test_123");
        assert!(
            serialized.contains("session_id=cs_test_123"),
            "Stripe session_id must survive boot normalization, got {serialized}"
        );
    }

    #[test]
    fn registry_oauth_params_survive_roundtrip() {
        let serialized = roundtrip("/registry?token=t0ken&login=octocat");
        assert!(
            serialized.contains("token=t0ken") && serialized.contains("login=octocat"),
            "registry OAuth params must survive boot normalization, got {serialized}"
        );

        let serialized = roundtrip("/registry?error=denied");
        assert!(
            serialized.contains("error=denied"),
            "registry OAuth error must survive boot normalization, got {serialized}"
        );
    }

    #[test]
    fn registry_search_action_url_works() {
        // The JSON-LD SearchAction advertises /registry?q={search_term}.
        let serialized = roundtrip("/registry?q=parser");
        assert!(
            serialized.contains("q=parser"),
            "registry search query must survive boot normalization, got {serialized}"
        );
        let route = Route::from_str("/registry?q=parser").unwrap();
        assert_eq!(
            route,
            Route::Registry {
                token: None,
                login: None,
                error: None,
                q: Some("parser".to_string())
            }
        );
    }

    #[test]
    fn percent_encoded_file_roundtrips() {
        let serialized = roundtrip("/studio?file=my%20notes.logos");
        assert!(
            serialized.contains("file=my%20notes.logos"),
            "encoded file param must survive boot normalization, got {serialized}"
        );
        let route = Route::from_str(&serialized).expect("re-serialized URL parses");
        assert_eq!(serialized, route.to_string(), "serialization is stable");
    }

    #[test]
    fn boot_normalization_is_fixpoint() {
        for url in [
            "/",
            "/studio",
            "/studio?file=examples/logic/prover-demo.logic",
            "/studio?file=examples/code/basics/hello.logos",
            "/news",
            "/news?tag=releases",
            "/success",
            "/success?session_id=cs_test_123",
            "/registry",
            "/registry?token=t0ken&login=octocat",
            "/registry?error=denied",
            "/registry?q=parser",
            "/registry/package/logicaffeine-base",
            "/news/some-article",
            "/workspace/logic",
        ] {
            let first = Route::from_str(url)
                .unwrap_or_else(|e| panic!("{url} failed to parse: {e}"));
            let s1 = first.to_string();
            let second = Route::from_str(&s1)
                .unwrap_or_else(|e| panic!("{s1} (from {url}) failed to re-parse: {e}"));
            assert_eq!(first, second, "{url}: parse → serialize → parse must be identity");
            assert_eq!(s1, second.to_string(), "{url}: serialization must be a fixpoint");
        }
    }
}

#[component]
fn NotFound(route: Vec<String>) -> Element {
    rsx! {
        div {
            style: "min-height: 100vh; display: flex; flex-direction: column; align-items: center; justify-content: center; background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%); color: #e8e8e8;",
            h1 { style: "font-size: 48px; margin-bottom: 16px;", "404" }
            p { style: "color: #888; margin-bottom: 24px;", "Page not found: /{route.join(\"/\")}" }
            Link {
                to: Route::Landing {},
                style: "padding: 12px 24px; background: linear-gradient(135deg, #667eea, #764ba2); border-radius: 8px; color: white; text-decoration: none;",
                "Go Home"
            }
        }
    }
}
