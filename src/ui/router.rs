use dioxus::prelude::*;
use crate::ui::pages::{Home, Landing, Learn, Lesson, Pricing, Privacy, Review, Roadmap, Success, Terms, Workspace, Studio};
use crate::ui::pages::registry::{Registry, PackageDetail};

#[derive(Clone, Routable, Debug, PartialEq)]
pub enum Route {
    #[route("/")]
    Landing {},

    #[route("/home")]
    Home {},

    #[route("/pricing")]
    Pricing {},

    #[route("/privacy")]
    Privacy {},

    #[route("/terms")]
    Terms {},

    #[route("/roadmap")]
    Roadmap {},

    #[route("/success")]
    Success {},

    #[route("/studio")]
    Studio {},

    #[route("/learn")]
    Learn {},

    #[route("/review")]
    Review {},

    #[route("/lesson/:era/:module/:mode")]
    Lesson { era: String, module: String, mode: String },

    #[route("/workspace/:subject")]
    Workspace { subject: String },

    // Phase 39: Package Registry
    #[route("/registry")]
    Registry {},

    #[route("/registry/package/:name")]
    PackageDetail { name: String },

    #[route("/:..route")]
    NotFound { route: Vec<String> },
}

#[component]
fn NotFound(route: Vec<String>) -> Element {
    rsx! {
        div {
            style: "min-height: 100vh; display: flex; flex-direction: column; align-items: center; justify-content: center; background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%); color: #e8e8e8;",
            h1 { style: "font-size: 48px; margin-bottom: 16px;", "404" }
            p { style: "color: #888; margin-bottom: 24px;", "Page not found: /{route.join(\"/\")}" }
            Link {
                to: Route::Home {},
                style: "padding: 12px 24px; background: linear-gradient(135deg, #667eea, #764ba2); border-radius: 8px; color: white; text-decoration: none;",
                "Go Home"
            }
        }
    }
}
