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
//! | `/studio` | [`Studio`] | Playground for experimentation |
//! | `/profile` | [`Profile`] | User settings and progress |
//! | `/pricing` | [`Pricing`] | Subscription plans |
//! | `/guide` | [`Guide`] | Documentation and tutorials |
//! | `/registry` | [`Registry`] | Package browser |
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
//!     Link { to: Route::Studio {}, "Open Studio" }
//! }
//! # }
//! ```

use dioxus::prelude::*;
use crate::ui::pages::{Landing, Learn, Pricing, Privacy, Profile, Roadmap, Success, Terms, Workspace, Studio, Guide, Crates, News, NewsArticle};
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

    /// Subscription plans and pricing at `/pricing`.
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
    #[route("/success")]
    Success {},

    /// Playground for experimentation at `/studio`.
    #[route("/studio")]
    Studio {},

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
    #[route("/registry")]
    Registry {},

    /// Package detail page at `/registry/package/:name`.
    #[route("/registry/package/:name")]
    PackageDetail {
        /// The package name to display.
        name: String,
    },

    /// News index page at `/news`.
    #[route("/news")]
    News {},

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
