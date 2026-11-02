//! ★ LOCK — what the dev server serves, the shipped binary serves. ★
//!
//! This campaign has now found FIVE divergences of one shape: the development
//! server and the built site answered the same request differently, and the
//! difference only ever showed up in production. `/index.html`, the pre-rendered
//! body, the page URL namespace, the MIME table, the module reference. Each was
//! found by a person, late.
//!
//! ★ THE ONE THIS FILE CLOSES: **a query string.** `largo serve` splits the query
//! off before matching (`serve_web::route_local`), and the generated
//! `logos_assets::lookup` splits it off too — but the emitted `Serve` gated the
//! whole asset table on `__raw.query.is_empty()`, so the split inside `lookup`
//! was unreachable. `/style.css?v=2` served in development and **404'd in
//! production**, and a cache-busting query on a stylesheet is one of the most
//! ordinary things anybody writes.
//!
//! ★ ON THE MEASURE. Proving this on the wire needs a release binary, which is a
//! multi-minute cargo build; that proof lives in the crown site lock beside the
//! other deploy heavies, which asks for a queried asset over a real socket. What
//! is asserted HERE is the thing that regresses: the guard itself. It is a scan,
//! and a scan is honest about a guard's *absence* in a way it is not about
//! behaviour — the guard is one literal expression, and its return would be
//! visible right here.

/// A site whose `Serve` carries an embedded asset table — the shape
/// `largo build` emits for a `--web` project.
const SERVER_LG: &str = "# Site\n\n\
    ## To handle (request: Request) -> Response:\n\
    \x20   Return plain(\"hi\").\n\n\
    ## Main\n\n\
    Serve on port 8080 with handle.\n";

/// The generated Rust for a web build (asset embedding ON, as `largo build`
/// turns it on for a project with `pages/` or `static/`).
fn embedded_serve_rust() -> String {
    logicaffeine_compile::codegen::with_embed_assets(|| {
        logicaffeine_compile::compile_to_rust(SERVER_LG).expect("the server compiles")
    })
}

/// ★ THE LOAD-BEARING TEST.
#[test]
fn a_queried_asset_is_not_gated_out_of_the_shipped_asset_table() {
    let rust = embedded_serve_rust();
    assert!(
        rust.contains("logos_assets::lookup"),
        "the embedded-asset lowering must be present for this test to mean anything:\n{rust}"
    );
    assert!(
        !rust.contains("query.is_empty()"),
        "the shipped asset table must not be gated on an empty query string. With that guard, \
         `/style.css?v=2` served in `largo serve` and 404'd in the built binary — a cache-busting \
         query is ordinary, and the split inside `logos_assets::lookup` was written for exactly \
         this and made unreachable by the guard.\n{rust}"
    );
}

/// The method gate is a DIFFERENT question and must survive: a `POST` to a path
/// an asset happens to occupy belongs to the handler, not to a static file.
#[test]
fn only_a_get_is_answered_from_the_asset_table() {
    let rust = embedded_serve_rust();
    assert!(
        rust.contains("__raw.method == \"GET\""),
        "assets answer GET only — a POST to the same path is the handler's, and dropping this \
         guard would silently shadow every form that posts to a path a file also occupies:\n{rust}"
    );
}

/// And the asset lookup keeps its own `?`-split, which is what makes the guard's
/// removal safe: the path a wire request carries is already query-free, but a
/// caller that passes a raw target still lands on the right asset.
///
/// This reads the OTHER generated artifact — the `logos_assets` module itself,
/// not the `Serve` lowering — because that is where the split lives.
#[test]
fn the_asset_lookup_still_splits_a_query_off_itself() {
    let dir = tempfile::tempdir().unwrap();
    let pages = dir.path().join("pages");
    let statics = dir.path().join("static");
    std::fs::create_dir_all(&pages).unwrap();
    std::fs::create_dir_all(&statics).unwrap();
    std::fs::write(statics.join("style.css"), b".card{}").unwrap();

    let module = logicaffeine_cli::assets_module(&pages, &statics).expect("the table generates");
    assert!(
        module.contains("/style.css"),
        "the table must hold the asset for this test to mean anything:\n{module}"
    );
    assert!(
        module.contains("split('?')"),
        "`logos_assets::lookup` must keep splitting the query off, so the table answers by PATH \
         however the caller spells the target:\n{module}"
    );
}
