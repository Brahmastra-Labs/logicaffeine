//! B4 — the embedded-assets `Serve` lowering, the codegen half (fast canary).
//!
//! `largo build` on a full-stack web project bundles every page + static asset
//! INTO the binary (a `logos_assets` module) and threads the embed hook so the
//! emitted `Serve` closure consults `logos_assets::lookup(&path)` BEFORE calling
//! the handler — an asset shadows a backend route, exactly the dev router's
//! precedence, so dev ≡ prod by construction. The hook is a thread-local
//! codegen scope (`with_embed_assets`), NOT a wire behavior change, so the
//! ground-truth live differential (`build_web_release.rs`) proves the running
//! binary actually serves the bundled site.
//!
//! The INVARIANT this canary locks: the hook is INERT by default. A plain
//! `Serve` program (no assets) must lower byte-identically to today — the HTTP
//! cross-tier differential (`crates/logicaffeine_tests/tests/http_differential.rs`)
//! depends on it. Only inside `with_embed_assets` does the `logos_assets`
//! preamble appear.

use logicaffeine_compile::codegen::with_embed_assets;
use logicaffeine_compile::compile::compile_to_rust;

const SERVE_PROGRAM: &str = "\
# Site

## To handle (request: Request) -> Response:
    If request's path is equal to \"/wasm\":
        Return bytesResponse(\"application/wasm\", [0, 97, 115, 109, 1, 0, 0, 0]).
    Return plain(\"text\").

## Main
Serve on port 8099 with handle.
";

/// By default the `Serve` lowering never mentions `logos_assets` — the asset
/// hook is INERT, so a non-web `Serve` program lowers byte-identically to today
/// (the invariant the HTTP differential rides on).
#[test]
fn the_default_serve_lowering_never_consults_embedded_assets() {
    let rust = compile_to_rust(SERVE_PROGRAM).expect("the serve program compiles");
    assert!(
        !rust.contains("logos_assets"),
        "the DEFAULT `Serve` lowering leaked an asset hook — a plain web \
         program would diverge from the interpreter tier.\n--- generated Rust ---\n{rust}"
    );
}

/// The embed scope must be perfectly restored: compiling with `with_embed_assets`
/// and then again by default yields the byte-identical default program — the
/// thread-local never leaks into a later compile on this thread.
#[test]
fn the_embed_scope_is_restored_and_the_default_stays_byte_identical() {
    let before = compile_to_rust(SERVE_PROGRAM).expect("compiles");
    let _embedded = with_embed_assets(|| compile_to_rust(SERVE_PROGRAM)).expect("compiles");
    let after = compile_to_rust(SERVE_PROGRAM).expect("compiles");
    assert_eq!(
        before, after,
        "the embed scope leaked into a later default compile"
    );
}

/// Inside `with_embed_assets`, the emitted `Serve` closure consults
/// `logos_assets::lookup(&…path)` BEFORE calling the handler, returns the
/// embedded bytes with a 200 + its content-type, and STILL honors the handler's
/// `bytes` field for the non-asset paths.
#[test]
fn the_embedded_serve_lowering_looks_up_assets_before_the_handler() {
    let rust =
        with_embed_assets(|| compile_to_rust(SERVE_PROGRAM)).expect("the serve program compiles");

    // The asset lookup is emitted.
    assert!(
        rust.contains("logos_assets::lookup"),
        "the embedded `Serve` lowering never consults logos_assets::lookup — the \
         bundled site is not served.\n--- generated Rust ---\n{rust}"
    );

    // It is consulted on the request path, BEFORE the handler is called. The
    // handler call is `let __resp = handle(__req);` — the lookup must precede it.
    let lookup_at = rust.find("logos_assets::lookup").expect("lookup present");
    let handler_at = rust
        .find("= handle(")
        .expect("the handler is still called for non-asset paths");
    assert!(
        lookup_at < handler_at,
        "assets must be consulted BEFORE the handler (dev-router precedence: an \
         asset shadows a backend route).\n--- generated Rust ---\n{rust}"
    );

    // The bytes field is STILL honored for non-asset paths (the G8 invariant is
    // not regressed by the asset hook).
    assert!(
        rust.contains("__resp.bytes"),
        "the embedded lowering dropped the handler's bytes field.\n--- generated Rust ---\n{rust}"
    );

    // The env-port override seam is still emitted (production deploys read PORT).
    assert!(
        rust.contains("env_port_override"),
        "the embedded lowering dropped the LOGOS_PORT/PORT override.\n--- generated Rust ---\n{rust}"
    );
}
