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
//! | `/learn#:module` | [`Learn`] | The Learn roadmap; `/profile` redirects here |
//! | `/studio?:file` | [`Studio`] | Playground for experimentation |
//! | `/contact#:section` | [`Contact`] | The contact page; `/pricing` redirects here |
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
//!     Link { to: Route::Learn { module: String::new() }, "Start Learning" }
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

use crate::ui::pages::registry::{PackageDetail, Registry};
use crate::ui::pages::{
    Benchmarks, Contact, Crates, Guide, Landing, Learn, News, NewsArticle, Privacy, Roadmap,
    Studio, Terms, Workspace,
};
use dioxus::prelude::*;

/// Application routes.
///
/// This enum is derived with `Routable` to generate route matching logic.
/// Each variant corresponds to a URL pattern and its associated page component.
#[derive(Clone, Routable, Debug, PartialEq)]
pub enum Route {
    /// Marketing homepage at `/`.
    #[route("/")]
    Landing {},

    /// The contact page at `/contact`.
    ///
    /// `/contact#<section>` opens at that section (`#contact` is the form); the fragment is
    /// modelled here so boot normalization keeps it. `/contact`, where people look for the team,
    /// the old `/pricing` address redirects here.
    #[redirect("/pricing", || Route::Contact { section: String::new() })]
    #[route("/contact#:section")]
    Contact {
        /// The section to open at (a section's id), or empty for the top of the page.
        section: String,
    },

    /// Privacy policy at `/privacy`.
    ///
    /// `/privacy#<section>` opens at that section of the policy; the contact form's privacy note
    /// links `#pricing-requests`.
    #[route("/privacy#:section")]
    Privacy {
        /// The section to open at (an id in the policy), or empty for the top of the page.
        section: String,
    },

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
    /// The Learn roadmap: one lesson at a time, with practice behind each.
    /// `/learn#<module-id>` opens that module's first unfinished lesson; the fragment is modelled
    /// here so boot normalization keeps it. `/profile`, the page that once kept a learner's score,
    /// redirects here.
    #[redirect("/profile", || Route::Learn { module: String::new() })]
    #[route("/learn#:module")]
    Learn {
        /// The module to open at (a curriculum module id), or empty for the top of the page.
        module: String,
    },

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

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/router/tests.rs"]
mod tests;

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
