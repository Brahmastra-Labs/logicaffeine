//! ★ LOCK — a shipped site's assets travel compressed. ★
//!
//! There was no `Content-Encoding` anywhere in the serving stack, so the WebAssembly
//! module — the largest thing a LOGOS page ships and the whole reason the page can be
//! written in LOGOS at all — went out raw on every first visit.
//!
//! ★ COMPRESSED AT BUILD TIME, NOT PER REQUEST. The bytes are already frozen and already
//! hashed; compressing them once at build costs the server nothing per request and cannot
//! be skipped under load. It also means the site ships one artifact whose behaviour does
//! not depend on how busy it is.
//!
//! ★ A SEPARATE TABLE, NOT A WIDER ROW. `asset_cache_lock` pins the shipped table's shape
//! — path, content type, cache control, ETag, bytes — because those five are what every
//! response is built from. The compressed variant is a DIFFERENT REPRESENTATION of the
//! same resource, so it is answered by its own lookup rather than by making every row
//! carry a column most of them do not use.
//!
//! ★ AND A DIFFERENT REPRESENTATION NEEDS A DIFFERENT ETAG. Two bodies under one entity
//! tag is how a shared cache comes to hand a gzip body to a client that cannot read one.
//! `Vary: Accept-Encoding` says the response depends on the request header; the distinct
//! ETag is what makes revalidation of each variant correct.

#[path = "cli_common/mod.rs"]
mod common;
use common::{largo_in, stderr};

use std::path::{Path, PathBuf};

fn site(dir: &Path, name: &str) -> PathBuf {
    let out = largo_in(dir, &["new", name, "--web"]);
    assert!(out.status.success(), "scaffold: {}", stderr(&out));
    let root = dir.join(name);
    // A stylesheet big enough that compressing it is unambiguously worth it.
    let css = (0..200)
        .map(|i| format!(".row-{i} {{ color: #cc785c; padding: 0.5rem; border-radius: 6px; }}\n"))
        .collect::<String>();
    std::fs::write(root.join("static/big.css"), css.as_bytes()).unwrap();
    root
}

fn assets_module(root: &Path) -> String {
    logicaffeine_cli::assets_module(&root.join("pages"), &root.join("static"))
        .expect("the table generates")
}

fn planned(root: &Path) -> Vec<(String, Vec<u8>)> {
    logicaffeine_cli::plan_site_assets(&root.join("pages"), &root.join("static"))
        .expect("the site plans")
        .into_iter()
        .map(|(p, _, b)| (p, b))
        .collect()
}

/// ★ THE LOAD-BEARING TEST — the module a visitor downloads has a compressed form.
#[test]
fn the_wasm_module_ships_a_compressed_form() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "sitea");
    let module = assets_module(&root);
    assert!(
        module.contains("pub fn encoded("),
        "the generated table must answer \"is there a compressed form of this?\":\n{module}"
    );
    let wasm = planned(&root)
        .into_iter()
        .find(|(p, _)| p.ends_with(".wasm"))
        .expect("the site ships a module");
    assert!(
        module.contains(&format!("\"{}\"", wasm.0)),
        "and the module's own path must be among the compressed entries — it is the largest \
         thing the page downloads and the point of the exercise:\n{module}"
    );
}

/// The compressed form must actually be smaller, or shipping it costs bytes and CPU for
/// nothing. Checked against the REAL compressor, on the REAL asset.
#[test]
fn a_compressed_asset_is_smaller_than_the_original() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "siteb");
    let table = planned(&root);
    let (path, raw) = table
        .iter()
        .find(|(p, _)| p == "/big.css")
        .expect("the stylesheet ships");
    let gz = logicaffeine_cli::gzip_asset(raw).expect("a 200-rule stylesheet compresses");
    assert!(
        gz.len() * 2 < raw.len(),
        "{path}: {} compressed to {} — a stylesheet of repeated rules must compress hard, and a \
         variant that is not meaningfully smaller is not worth a second representation",
        raw.len(),
        gz.len()
    );
    // It is real gzip, not raw deflate: the two magic bytes are what a browser dispatches on.
    assert_eq!(
        &gz[..2],
        &[0x1f, 0x8b],
        "the container must be gzip, which is what the header names"
    );
}

/// ★ A TINY FILE MUST NOT BE "COMPRESSED" INTO A BIGGER ONE. gzip's own header is 18
/// bytes before a single byte of content, so below a threshold the variant is pure loss.
#[test]
fn an_incompressible_asset_gets_no_variant() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "sitec");
    std::fs::write(root.join("static/tiny.css"), b"a{}").unwrap();
    let module = assets_module(&root);
    let entries: Vec<&str> = module
        .split("ENCODED")
        .nth(1)
        .unwrap_or("")
        .split('\n')
        .take_while(|l| !l.contains("];"))
        .collect();
    let joined = entries.join("\n");
    assert!(
        !joined.contains("/tiny.css"),
        "a 3-byte stylesheet must not ship a gzip variant — the container alone is larger than \
         the file:\n{joined}"
    );
}

/// ★ AND THE SERVER MUST ONLY SEND IT TO A CLIENT THAT ASKED. The whole contract is three
/// headers: negotiate on `Accept-Encoding`, declare `Content-Encoding`, and say
/// `Vary: Accept-Encoding` so a shared cache does not hand the compressed body to a client
/// that cannot read it.
#[test]
fn the_serve_hook_negotiates_and_declares_the_encoding() {
    let rust = logicaffeine_compile::codegen::with_embed_assets(|| {
        logicaffeine_compile::compile_to_rust(
            "# Site\n\n## To handle (request: Request) -> Response:\n\
             \x20   Return plain(\"hi\").\n\n## Main\n\nServe on port 8080 with handle.\n",
        )
        .expect("the server compiles")
    });
    for needle in [
        "accept-encoding",
        "Content-Encoding",
        "Vary",
        "logos_assets::encoded",
    ] {
        assert!(
            rust.contains(needle),
            "the embedded-asset lowering must carry `{needle}` — negotiation, declaration and \
             the cache directive are one contract, and a response missing any of the three is \
             a body somebody cannot read:\n{rust}"
        );
    }
}

/// ★ A DIFFERENT BODY IS A DIFFERENT ENTITY TAG. Serving two representations under one
/// ETag is exactly how a shared cache comes to answer a plain client with a gzip body.
#[test]
fn the_compressed_variant_carries_its_own_etag() {
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), "sited");
    let module = assets_module(&root);
    assert!(
        module.contains("-gz\""),
        "the compressed variant must carry a distinct ETag, so each representation \
         revalidates as itself:\n{module}"
    );
}
