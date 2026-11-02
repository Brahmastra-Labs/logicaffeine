#![doc = include_str!("../README.md")]

pub mod browser;
pub mod cli;
pub(crate) mod closure;
pub(crate) mod commands;

/// The content type largo serves a path as — the table `largo serve` and `largo build` both use.
///
/// Public so `mime_single_source_lock` asks the REAL table rather than a copy, and can hold it
/// against `static.lg`'s `contentTypeFor`, which is what a DEPLOYED LOGOS program answers with.
/// The two had already drifted (`text/css` against `text/css; charset=utf-8`, and six types
/// `static.lg` did not know at all), which means a file typed correctly in development and
/// wrongly in production.
pub fn content_type_for(path: &str) -> &'static str {
    commands::serve_web::content_type_for(path)
}

/// Which stylesheets a page links — the ONE rule `largo serve` and `largo build` both apply.
///
/// Public so `mime_single_source_lock` holds the real convention rather than a copy: it was
/// written twice before, so a page could be styled in development and unstyled in production.
pub fn stylesheet_links_for<'a>(paths: impl Iterator<Item = &'a str>) -> String {
    commands::serve_web::stylesheet_links_for(paths)
}

/// The URLs a page answers — the ONE rule `largo serve` and `largo build` both follow.
///
/// Public so `mime_single_source_lock` holds the real namespace rather than a copy. Written
/// twice, they had already diverged: the dev server answered `/index.html` and the built site
/// did not publish it, so a link that worked all through development 404'd the moment the site
/// shipped.
pub fn page_html_paths(stem: &str) -> Vec<String> {
    commands::web_bundle::page_html_paths(stem)
}

/// The document `largo serve` sends for a page — the same pre-rendered markup `largo build`
/// ships, so what a developer looks at is what their visitors get.
///
/// Public so `mime_single_source_lock` holds dev and prod against each other through the REAL
/// functions. They diverged before: the build pre-rendered the content and the dev server served
/// the bare shell.
pub fn dev_page_html(source: &str, stem: &str) -> String {
    commands::web_bundle::prerendered_html_pub(source, stem)
}

/// A built site's planned assets as `(path, content type, bytes)` — the REAL table `largo build`
/// bakes into the binary, exposed so `asset_cache_lock` holds the shipped names and headers rather
/// than a copy of the rule.
pub fn plan_site_assets(
    pages_dir: &std::path::Path,
    static_dir: &std::path::Path,
) -> Result<Vec<(String, String, Vec<u8>)>, String> {
    commands::web_bundle::plan_site(pages_dir, static_dir)
        .map(|assets| {
            assets
                .into_iter()
                .map(|a| (a.path, a.content_type, a.bytes))
                .collect()
        })
        .map_err(|e| e.to_string())
}

/// Does a concrete request path match a page's declared `## Page` path? A `:name` segment
/// matches any one segment — the same rule `web.lg`'s `matchRoute` applies inside the page,
/// exposed so `page_pattern_routing_lock` holds the REAL matcher the router uses.
pub fn page_path_matches(pattern: &str, request: &str) -> bool {
    commands::web_bundle::page_path_matches(pattern, request)
}

/// The URL-and-filesystem-safe form of a page path (`/posts/:id` → `/posts/id`) — the shipped
/// name, exposed so the lock holds the published bytes rather than a copy of the rule.
pub fn page_asset_base(path: &str) -> String {
    commands::web_bundle::page_asset_base(path)
}

/// gzip some bytes the way `largo build` does, or `None` when a compressed form is not
/// worth shipping — exposed so `asset_compression_lock` measures the REAL compressor on a
/// REAL asset rather than a copy of the rule.
pub fn gzip_asset(bytes: &[u8]) -> Option<Vec<u8>> {
    commands::web_bundle::gzip_asset(bytes)
}

/// The generated `logos_assets` module a built site bakes into its binary — the REAL table, so a
/// lock holds the headers that actually ship rather than the rule that computes them.
pub fn assets_module(
    pages_dir: &std::path::Path,
    static_dir: &std::path::Path,
) -> Result<String, String> {
    let patterns = commands::web_bundle::page_patterns(pages_dir);
    commands::web_bundle::plan_site(pages_dir, static_dir)
        .map(|assets| commands::web_bundle::site_assets_module(pages_dir, &assets, &patterns))
        .map_err(|e| e.to_string())
}

/// The short content digest an asset's file name carries — see `web_bundle::content_hash`.
pub fn content_hash(bytes: &[u8]) -> String {
    commands::web_bundle::content_hash(bytes)
}

/// An asset's quoted `ETag`.
pub fn etag_for(bytes: &[u8]) -> String {
    commands::web_bundle::etag_for(bytes)
}

/// The `Cache-Control` a path is served with — immutable for a content-addressed name, revalidated
/// for a name people link to.
pub fn cache_control_for(path: &str) -> String {
    commands::web_bundle::cache_control_for(path)
}

/// Answer a conditional request: `(status, body length)` for an ETag and a client's
/// `If-None-Match`.
pub fn conditional_response(etag: &str, if_none_match: Option<&str>, len: usize) -> (u16, usize) {
    commands::web_bundle::conditional_response(etag, if_none_match, len)
}

/// A built page's document with its markup already in it — the static-render half of
/// `largo build`. Public so `ssg_lock` exercises the REAL function the build calls, rather than a
/// copy that could drift from it.
pub fn prerender_page_html(source: &str, stem: &str) -> String {
    commands::web_bundle::prerendered_html_pub(source, stem)
}
pub mod compile;
pub mod dap;
pub(crate) mod diagnostics;
pub use diagnostics::parse_error_report;
pub mod mcp;
pub mod project;
pub(crate) mod repl;
pub mod ui;

/// Re-export of the LOGOS interface types from the kernel crate.
///
/// Provides access to core types like `World` and `Entity` for
/// integration with the LOGOS runtime.
pub use logicaffeine_kernel::interface;

/// Re-export of analysis utilities from the compile crate.
///
/// Useful for tooling that needs to analyze LOGOS source without
/// performing a full build.
pub use logicaffeine_compile::analysis;

/// Entry point for the CLI.
///
/// Parses command-line arguments and executes the appropriate command.
/// See [`cli::run_cli`] for details.
pub use cli::run_cli;

/// The `largo` process: installs the native tier, runs [`run_cli`] on a worker whose stack is sized
/// from the AST depth limit, renders any error, and exits with its code. `src/main.rs` is exactly
/// this call, and so is the test crate's `largo-under-test`, so the CLI suites spawn what ships.
pub fn largo_main() {
    // The copy-and-patch JIT becomes the process-wide native tier: every
    // interpreted program the CLI runs gets hot-function and hot-loop
    // tier-up. Only x86_64 System V (Linux + macOS) links it; on Windows,
    // aarch64 and WASM the tier is absent and largo runs the bytecode VM.
    // `LOGOS_NO_JIT=1` skips installation so programs run on the VM bytecode
    // interpreter alone — a fallback when the JIT miscompiles a hot loop, and
    // the knob the differential gate uses to compare AOT against the unjitted
    // reference engine.
    #[cfg(all(target_arch = "x86_64", not(target_os = "windows")))]
    logicaffeine_jit::segv_trace_install();
    #[cfg(all(target_arch = "x86_64", not(target_os = "windows")))]
    if std::env::var_os("LOGOS_NO_JIT").is_none() {
        logicaffeine_jit::install();
    }

    // Run the CLI on a worker whose stack is SIZED FROM the AST depth limit,
    // so `LOGOS_MAX_AST_DEPTH` genuinely works: raising the limit raises the
    // stack that honors it (the main thread's stack is fixed by `ulimit -s`
    // at exec and can't follow the knob). 40 KiB/level covers the fattest
    // measured walker frame with margin; the pages are virtual until
    // touched, so a large reservation costs nothing on shallow programs.
    let stack_bytes = logicaffeine_language::ast_depth::max_ast_depth()
        .saturating_mul(40 * 1024)
        .max(16 * 1024 * 1024);
    let outcome = std::thread::Builder::new()
        .name("largo".into())
        .stack_size(stack_bytes)
        .spawn(|| {
            // Errors render inside the worker (their type isn't Send);
            // structured CLI errors carry a hint and their own exit code,
            // anything else renders as a bare `error:` line and exits 1.
            match run_cli() {
                Ok(()) => 0,
                Err(e) => ui::render_error(e.as_ref()),
            }
        })
        .expect("spawn the largo worker")
        .join();

    match outcome {
        Ok(0) => {}
        Ok(code) => std::process::exit(code),
        // A panicked worker already printed its panic message.
        Err(_) => std::process::exit(101),
    }
}
