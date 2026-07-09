//! Logicaffeine Web Application Entry Point
//!
//! Dual-target binary. Built with `--features web` it is the wasm SPA; built with
//! `--features server` it is the SSG prerender server that `dx build --ssg` drives
//! to write per-route static HTML (crawlers and first paint get real pages, then
//! the wasm client takes over rendering on the user's machine).
//!
//! The client deliberately does NOT hydrate: prerendered HTML is for crawlers and
//! the pre-wasm paint, and a hydrating client would hard-fail on the SPA-fallback
//! URLs that cannot be prerendered (`/registry/package/:name`). Instead the wasm
//! mounts into `#app` while the prerendered copy stays visible in `#main`; the
//! `body:has(#app > *) > #main` rule in index.html hides the copy in the same
//! paint the app's first frame lands, and the App root then removes it — an
//! atomic swap with no cleared-DOM gap and no unstyled flash.

use logicaffeine_web::App;

#[cfg(any(feature = "web", feature = "server"))]
fn main() {
    use dioxus::prelude::*;

    // The prerender pass writes into a SIBLING of public/ rather than public/
    // itself: dx finishes the client build after the prerender and rewrites
    // public/index.html (the shell), which would clobber the prerendered `/`.
    // scripts/merge-ssg.sh copies prerendered/ over public/ once dx is done.
    dioxus::LaunchBuilder::new()
        .with_cfg(web! {
            dioxus::web::Config::new().rootname("app")
        })
        .with_cfg(server_only! {
            dioxus::server::ServeConfig::builder().incremental(
                dioxus::server::IncrementalRendererConfig::new()
                    .static_dir(
                        std::env::current_exe()
                            .expect("server exe path")
                            .parent()
                            .expect("server exe dir")
                            .join("prerendered"),
                    )
                    .clear_cache(false),
            )
        })
        .launch(App);
}

#[cfg(not(any(feature = "web", feature = "server")))]
fn main() {
    panic!("logicaffeine-web builds with --features web (wasm client) or server (SSG prerender)");
}

/// The prerender manifest: dx's `--ssg` pass POSTs `/api/static_routes` and then
/// GETs every returned route, and the incremental renderer writes each page's HTML
/// beside the bundle in `public/`.
#[cfg(feature = "server")]
mod ssg {
    use dioxus::prelude::*;

    #[server(endpoint = "static_routes")]
    pub async fn static_routes() -> ServerFnResult<Vec<String>> {
        Ok(logicaffeine_web::sitemap::prerender_routes())
    }
}
