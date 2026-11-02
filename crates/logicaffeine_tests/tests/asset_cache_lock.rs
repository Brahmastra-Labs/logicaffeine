//! ★ LOCK — a built site's assets are cacheable, and the ones that can be cached forever are. ★
//!
//! Every asset a LOGOS site shipped was served with a `Content-Type` and NOTHING ELSE. No
//! `Cache-Control`, no `ETag`, anywhere — so a visitor's browser re-downloaded the WebAssembly
//! module, the host shim and every stylesheet on **every single page load**, and had no way to ask
//! "has this changed?" short of fetching the whole thing again. On the tier where the framework's
//! premise lives — LOGOS running in the browser — that is the module's entire weight paid again on
//! every navigation.
//!
//! ★ THE RULE IS THE NAME. An asset whose file name contains a hash of its own bytes can be cached
//! FOREVER, because a different body is a different name: `immutable` is not a promise about the
//! future, it is a statement about what the URL means. An asset whose name is stable — a page at
//! `/`, whose URL is the thing people link to — must NOT be cached that way, or a deploy is
//! invisible until the cache expires. Those two need opposite headers, and telling them apart by
//! the NAME is what makes the rule checkable rather than a convention someone remembers.
//!
//! ★ AND A REVALIDATION MUST BE CHEAP. A page carries an `ETag` of its bytes, so a browser asks
//! with `If-None-Match` and gets **304 Not Modified** and no body when nothing changed — the
//! difference between a repeat visit costing a round trip and costing a page.

#[path = "cli_common/mod.rs"]
mod common;

use common::{largo_in, stderr};

/// Build a `--web` project and hand back its planned asset table.
fn built_site() -> Vec<(String, String, Vec<u8>)> {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = largo_in(dir.path(), &["new", "site", "--web"]);
    assert!(out.status.success(), "largo new --web: {}", stderr(&out));
    let root = dir.path().join("site");
    logicaffeine_cli::plan_site_assets(&root.join("pages"), &root.join("static"))
        .expect("the site plans")
}

fn find<'a>(assets: &'a [(String, String, Vec<u8>)], path: &str) -> &'a (String, String, Vec<u8>) {
    assets
        .iter()
        .find(|(p, _, _)| p == path)
        .unwrap_or_else(|| {
            panic!(
                "no asset at {path}; site has {:?}",
                assets.iter().map(|(p, _, _)| p).collect::<Vec<_>>()
            )
        })
}

#[test]
fn the_module_and_the_shim_are_content_addressed() {
    // ★ THE ASSET WHOSE WEIGHT MATTERS MOST. The WebAssembly module is the largest thing a page
    // ships and the thing least likely to change between visits, so it is the one that must be
    // cacheable forever — which means its NAME must change when its bytes do.
    let assets = built_site();
    let names: Vec<&String> = assets.iter().map(|(p, _, _)| p).collect();
    let hashed_wasm = names
        .iter()
        .find(|p| p.ends_with(".wasm"))
        .expect("the site ships a module");
    assert!(
        hashed_wasm.matches('.').count() >= 2,
        "the module's name carries a hash of its bytes, so it can be cached forever: {hashed_wasm}"
    );
    let hashed_js = names
        .iter()
        .find(|p| p.ends_with(".js"))
        .expect("the site ships a host shim");
    assert!(
        hashed_js.matches('.').count() >= 2,
        "and so does the shim: {hashed_js}"
    );
}

#[test]
fn the_page_that_references_them_uses_the_hashed_names() {
    // ★ A HASH NOBODY LINKS TO IS A 404. Renaming the asset and leaving the page pointing at the
    // old name would ship a site whose module never loads — which is worse than no caching at all,
    // and exactly the failure a rename invites.
    let assets = built_site();
    let (_, _, html) = find(&assets, "/");
    let html = String::from_utf8_lossy(html);
    let wasm = assets
        .iter()
        .find(|(p, _, _)| p.ends_with(".wasm"))
        .expect("a module")
        .0
        .clone();
    let js = assets
        .iter()
        .find(|(p, _, _)| p.ends_with(".js"))
        .expect("a shim")
        .0
        .clone();
    let js_name = js.trim_start_matches('/');
    assert!(
        html.contains(js_name),
        "the page links the shim by its hashed name `{js_name}`:\n{html}"
    );
    // The module is fetched by the shim rather than named in the markup, so the SHIM must carry it.
    let (_, _, shim) = find(&assets, &js);
    let shim = String::from_utf8_lossy(shim);
    let wasm_name = wasm.trim_start_matches('/');
    assert!(
        shim.contains(wasm_name),
        "the shim fetches the module by its hashed name `{wasm_name}`"
    );
}

#[test]
fn the_same_bytes_get_the_same_name_and_different_bytes_do_not() {
    // The hash must be OF THE BYTES: a rebuild that changed nothing must produce the same URL (or
    // every deploy pointlessly busts every cache), and a rebuild that changed something must not
    // (or a visitor keeps the old module forever, which `immutable` makes permanent).
    let a = built_site();
    let b = built_site();
    let name = |s: &Vec<(String, String, Vec<u8>)>| {
        s.iter()
            .find(|(p, _, _)| p.ends_with(".wasm"))
            .expect("a module")
            .0
            .clone()
    };
    assert_eq!(
        name(&a),
        name(&b),
        "an unchanged build keeps its asset URLs"
    );

    let differing =
        logicaffeine_cli::content_hash(b"one") != logicaffeine_cli::content_hash(b"two");
    assert!(differing, "different bytes hash differently");
    assert_eq!(
        logicaffeine_cli::content_hash(b"same"),
        logicaffeine_cli::content_hash(b"same"),
        "and the same bytes hash the same, on every machine and every run"
    );
}

#[test]
fn a_content_addressed_asset_is_immutable_and_a_page_is_not() {
    // The two halves of the rule, and they must be OPPOSITE. `immutable` on a page means a deploy
    // is invisible until the cache expires — on a year-long max-age, effectively forever.
    // A REAL name, taken from the hash function itself rather than a plausible-looking literal —
    // the rule keys on a 16-hex digest, and a shorter stand-in would have tested the wrong string.
    let hashed = logicaffeine_cli::cache_control_for(&format!(
        "/app.{}.wasm",
        logicaffeine_cli::content_hash(b"module bytes")
    ));
    assert!(
        hashed.contains("max-age=31536000"),
        "a hashed asset is cached for a year: {hashed}"
    );
    assert!(
        hashed.contains("immutable"),
        "…and never revalidated: {hashed}"
    );

    for page in ["/", "/index.html", "/about.html"] {
        let plain = logicaffeine_cli::cache_control_for(page);
        assert!(
            !plain.contains("immutable"),
            "`{page}` is a URL people LINK to — caching it immutably makes a deploy invisible: {plain}"
        );
        assert!(
            plain.contains("no-cache") || plain.contains("max-age=0"),
            "…so it is revalidated on every visit: {plain}"
        );
    }
}

#[test]
fn an_unhashed_static_file_is_revalidated_rather_than_cached_forever() {
    // A file the author dropped in `static/` keeps the name they gave it, so its bytes can change
    // under a URL that does not. Treating it like a hashed asset would pin a stale logo on every
    // visitor who ever loaded the site.
    let plain = logicaffeine_cli::cache_control_for("/logo.png");
    assert!(
        !plain.contains("immutable"),
        "an author-named file is not immutable: {plain}"
    );
}

#[test]
fn every_asset_carries_an_etag_of_its_bytes() {
    // The cheap revalidation. Without an ETag a browser asking "has this changed?" can only
    // re-fetch the whole body; with one it gets 304 and nothing.
    let assets = built_site();
    assert!(!assets.is_empty(), "the site ships assets");
    for (path, _, bytes) in &assets {
        let tag = logicaffeine_cli::etag_for(bytes);
        assert!(
            tag.starts_with('"') && tag.ends_with('"'),
            "`{path}`'s ETag is quoted: {tag}"
        );
        assert!(tag.len() > 4, "`{path}`'s ETag is a real digest: {tag}");
    }
    // Different bodies, different tags — an ETag that collided would serve a stale body as fresh.
    assert_ne!(
        logicaffeine_cli::etag_for(b"a"),
        logicaffeine_cli::etag_for(b"b")
    );
    assert_eq!(
        logicaffeine_cli::etag_for(b"a"),
        logicaffeine_cli::etag_for(b"a")
    );
}

#[test]
fn a_matching_if_none_match_is_a_304_with_no_body() {
    // The whole point of the ETag: the repeat visit costs a round trip instead of a page. A 304
    // that carried a body would be a wasted header rather than a saving.
    let body = b"the page";
    let tag = logicaffeine_cli::etag_for(body);
    let (status, len) = logicaffeine_cli::conditional_response(&tag, Some(&tag), body.len());
    assert_eq!(status, 304, "a matching ETag means Not Modified");
    assert_eq!(len, 0, "and a 304 carries no body");

    let (status, len) = logicaffeine_cli::conditional_response(&tag, Some("\"stale\""), body.len());
    assert_eq!(
        status, 200,
        "a different ETag means the body really is needed"
    );
    assert_eq!(len, body.len());

    let (status, len) = logicaffeine_cli::conditional_response(&tag, None, body.len());
    assert_eq!(status, 200, "a first visit sends the body");
    assert_eq!(len, body.len());
}

#[test]
fn a_client_sending_several_etags_matches_any_of_them() {
    // `If-None-Match` is a LIST — a browser holding several variants sends them all, and a server
    // that only compared the whole header against one tag would re-send a body the client has.
    let tag = logicaffeine_cli::etag_for(b"x");
    let header = format!("\"other\", {tag}, \"more\"");
    let (status, _) = logicaffeine_cli::conditional_response(&tag, Some(&header), 8);
    assert_eq!(status, 304, "any tag in the list matches: {header}");
}

#[test]
fn a_wildcard_if_none_match_matches_anything_that_exists() {
    // `*` means "if you have any version of this at all" — the spec's own shape, and a client
    // really does send it.
    let tag = logicaffeine_cli::etag_for(b"x");
    let (status, _) = logicaffeine_cli::conditional_response(&tag, Some("*"), 8);
    assert_eq!(status, 304);
}

#[test]
fn the_generated_table_carries_the_headers_and_the_serve_hook_sends_them() {
    // ★ THE HALF THAT WOULD HAVE SHIPPED INERT. Computing a cache rule proves nothing if the bytes
    // go out with only a `Content-Type` — which is what every LOGOS site did. This holds the whole
    // path: the table BAKED INTO THE BINARY carries the header for each asset, and the emitted
    // serve hook reads them and answers a conditional request.
    let dir = tempfile::tempdir().expect("temp dir");
    let out = largo_in(dir.path(), &["new", "site", "--web"]);
    assert!(out.status.success(), "largo new --web: {}", stderr(&out));
    let root = dir.path().join("site");
    let module = logicaffeine_cli::assets_module(&root.join("pages"), &root.join("static"))
        .expect("the site plans");
    assert!(
        module.contains("max-age=31536000, immutable"),
        "the baked table carries the immutable rule for its hashed assets:\n{module}"
    );
    assert!(
        module.contains("no-cache"),
        "…and the revalidate rule for its pages:\n{module}"
    );
    assert!(
        module.contains("pub fn fresh("),
        "…and the conditional check:\n{module}"
    );
    // A tag really is a digest of the bytes, not a placeholder.
    assert!(
        module.matches('"').count() > 8 && !module.contains("\"etag\""),
        "the tags are real digests"
    );
}

#[test]
fn the_baked_headers_are_not_double_escaped() {
    // ★ A LATENT BUG THIS SURFACED. The generated table wrote each field as
    // `format!("{:?}", rust_str_escape(field))` — and `{:?}` ALREADY writes a Rust string literal
    // with its escapes, so escaping first escaped TWICE. It survived unnoticed because a path and
    // a content-type contain no quotes; an ETag is quoted BY THE HEADER SYNTAX, so it went out as
    // `ETag: \"857c…\"` — with literal backslashes — into a real release binary.
    //
    // A browser comparing `If-None-Match` would still have matched (the same wrong string both
    // ways), which is what makes this the kind of bug that ships: it looks like it works.
    let dir = tempfile::tempdir().expect("temp dir");
    let out = largo_in(dir.path(), &["new", "site", "--web"]);
    assert!(out.status.success(), "largo new --web: {}", stderr(&out));
    let root = dir.path().join("site");
    let module = logicaffeine_cli::assets_module(&root.join("pages"), &root.join("static"))
        .expect("the site plans");
    assert!(
        !module.contains("\\\\\""),
        "no field is double-escaped — an ETag must reach the wire as `\"digest\"`, not \
         `\\\"digest\\\"`:\n{module}"
    );
    // And the tags really are the quoted form the header syntax wants.
    assert!(
        module.contains("\\\""),
        "the ETag literals carry their quotes:\n{module}"
    );
}
