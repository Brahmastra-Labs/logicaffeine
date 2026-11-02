//! C2 — the pure-LOGOS static-serving helpers (`static.lg`).
//!
//! A web server needs three things beyond `http.lg`'s response constructors to
//! serve a frontend bundle: a MIME map from a path's extension
//! (`contentTypeFor`), a byte response read straight off disk with that type
//! (`fileResponse`), and a rooted-directory server with a HARD path-traversal
//! guard (`staticUnder`). All three are written in LOGOS over `readBytes` +
//! `http.lg`'s `bytesResponse`/`notFound`, so they run on every executing tier.
//!
//! These tests are HANDLER-LEVEL — no sockets. We call the helper with an
//! ordinary value and inspect the `Response` it returns, exactly as
//! `http_stdlib.rs` does with `plain`/`json`/`notFound`. The MIME + traversal
//! logic is pure text, so it runs cross-tier via `assert_all_tiers` (TW + VM,
//! the oracle path). The one test that touches the filesystem (serving a real
//! temp `.wasm`) also runs cross-tier, mirroring how `file_io_cross_tier.rs`
//! scopes `readBytes` to the executing tiers.
//!
//! YOU DO NOT GET TO WEAKEN THE TRAVERSAL LOCK TO MAKE IT PASS. A `..` segment
//! reaching disk is a directory-traversal vulnerability; every traversal shape
//! below MUST resolve to `notFound()` (404). Fix the helper, never the assert.

mod common;
use common::assert_all_tiers;

// ─────────────────────────────────────────────────────────────────────────────
// contentTypeFor — the extension → MIME map.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn content_type_for_maps_known_extensions() {
    // ★ A TEXT TYPE NAMES ITS CHARSET. Without one the browser GUESSES the encoding, and a page
    // that was served correctly renders a visitor's name as mojibake. `text/javascript` is the
    // deliberate exception — its own specification defines the type as always UTF-8 and obsoletes
    // the parameter (RFC 9239), so writing one is noise the spec asks us not to write.
    //
    // These expectations changed when this table became the SINGLE source of truth: `largo serve`
    // and `largo build` now read it rather than keeping a second copy (`loader::content_type_for`,
    // locked by `mime_single_source_lock`). The tool had always been right about the charset and
    // this module had not, so the two disagreed — a file typed correctly in development and
    // wrongly in production. Merging them meant one of the two answers had to go, and dropping
    // the charset would have made every served page LESS correct.
    assert_all_tiers(
        r#"## Main
Show contentTypeFor("index.html").
Show contentTypeFor("app.js").
Show contentTypeFor("app.wasm").
Show contentTypeFor("style.css").
Show contentTypeFor("icon.svg").
Show contentTypeFor("logo.png").
Show contentTypeFor("data.json").
"#,
        "text/html; charset=utf-8\ntext/javascript\napplication/wasm\ntext/css; charset=utf-8\nimage/svg+xml\nimage/png\napplication/json",
    );
}

#[test]
fn content_type_for_types_the_files_a_real_site_ships() {
    // The types this module did NOT know before it became the single source: a deployed site
    // served its own fonts, photographs and documents as `application/octet-stream` while
    // `largo serve` typed them correctly — so a page worked in development and had no fonts in
    // production. Every one of these is a file an ordinary site ships.
    assert_all_tiers(
        r#"## Main
Show contentTypeFor("photo.jpg").
Show contentTypeFor("photo.jpeg").
Show contentTypeFor("spin.gif").
Show contentTypeFor("hero.webp").
Show contentTypeFor("body.woff").
Show contentTypeFor("body.woff2").
Show contentTypeFor("terms.pdf").
Show contentTypeFor("notes.txt").
"#,
        "image/jpeg\nimage/jpeg\nimage/gif\nimage/webp\nfont/woff\nfont/woff2\napplication/pdf\ntext/plain; charset=utf-8",
    );
}

#[test]
fn content_type_for_unknown_extension_is_octet_stream() {
    // The default: an unknown or absent extension is the generic binary type,
    // never a guess. `README` (no dot) and `data.bin` both fall through.
    assert_all_tiers(
        r#"## Main
Show contentTypeFor("data.bin").
Show contentTypeFor("README").
Show contentTypeFor("archive.tar.gz").
"#,
        "application/octet-stream\napplication/octet-stream\napplication/octet-stream",
    );
}

#[test]
fn content_type_for_is_case_insensitive_on_the_extension() {
    // A path from the wire may carry any casing; the extension match must not.
    assert_all_tiers(
        r#"## Main
Show contentTypeFor("INDEX.HTML").
Show contentTypeFor("App.Js").
Show contentTypeFor("Module.WASM").
"#,
        "text/html; charset=utf-8\ntext/javascript\napplication/wasm",
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// staticUnder — the traversal guard (the security core).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn static_under_rejects_every_dot_dot_traversal_shape_with_404() {
    // The HARD lock. Any `..` path segment — leading, interior, or one that
    // would climb above the served root — must never reach `readBytes`; it
    // resolves to 404, full stop. We assert the STATUS (404) for a battery of
    // traversal shapes; a served file would be 200.
    assert_all_tiers(
        r#"## Main
Let a be staticUnder("web", "../etc/passwd").
Let b be staticUnder("web", "a/../../x").
Let c be staticUnder("web", "..").
Let d be staticUnder("web", "foo/../../bar").
Let e be staticUnder("web", "../../../../etc/shadow").
Let f be staticUnder("web", "ok/../../../secret").
Show a's status.
Show b's status.
Show c's status.
Show d's status.
Show e's status.
Show f's status.
"#,
        "404\n404\n404\n404\n404\n404",
    );
}

#[test]
fn static_under_rejects_a_lone_dot_dot_segment_even_amid_valid_names() {
    // A single `..` anywhere in the segment list is fatal, regardless of the
    // other segments looking benign.
    assert_all_tiers(
        r#"## Main
Let r be staticUnder("assets", "css/../../../../root/.ssh/id_rsa").
Show r's status.
"#,
        "404",
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// fileResponse / staticUnder — the happy path over a REAL file (file IO tier).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn file_response_serves_the_files_bytes_with_the_right_content_type() {
    // Write a real 4-byte "wasm" file, serve it through `fileResponse`, and read
    // back the Response: status 200, `application/wasm`, empty text body, and the
    // raw bytes in `bytes`. This exercises `readBytes` → `bytesResponse` on every
    // executing tier (mirrors `file_io_cross_tier.rs`).
    assert_all_tiers(
        r#"## Main
Let w be write("/tmp/logos_static_c2_app.wasm", "AB\ncd").
Inspect w:
    When Ok (u):
        Let r be fileResponse("/tmp/logos_static_c2_app.wasm").
        Show r's status.
        Show r's contentType.
        Show r's body.
        Show r's bytes.
    When Err (e):
        Show e.
"#,
        "200\napplication/wasm\n\n[65, 66, 10, 99, 100]",
    );
}

#[test]
fn static_under_serves_a_safe_file_under_the_root() {
    // The happy path of the guarded server: a `..`-free path joins under the root
    // and serves the file's bytes with the extension's content type.
    assert_all_tiers(
        r#"## Main
Let w be write("/tmp/logos_static_c2_index.html", "<h1>hi</h1>").
Inspect w:
    When Ok (u):
        Let r be staticUnder("/tmp", "logos_static_c2_index.html").
        Show r's status.
        Show r's contentType.
        Show r's bytes.
    When Err (e):
        Show e.
"#,
        "200\ntext/html; charset=utf-8\n[60, 104, 49, 62, 104, 105, 60, 47, 104, 49, 62]",
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// The absent file — a clean 404, never an abort.
// ─────────────────────────────────────────────────────────────────────────────
//
// `readBytes` ABORTS on a missing path, so a naive `fileResponse` core-dumped the
// server on the first mistyped asset URL. `fileResponse` now probes with the
// recoverable `read` and answers `notFound()` instead. That is the behavior a
// deployed site depends on and nothing pinned it, so a future rewrite of the
// helper could silently restore the abort. These two lock it on every executing
// tier: the direct call, and the rooted server that delegates to it.
//
// DO NOT relax these to an abort or a 500. A missing static asset is a 404.

#[test]
fn file_response_answers_404_for_a_file_that_is_not_there() {
    assert_all_tiers(
        r#"## Main
Let r be fileResponse("/tmp/logos_static_c2_definitely_absent.html").
Show r's status.
Show r's body.
"#,
        "404\nNot Found",
    );
}

#[test]
fn static_under_answers_404_for_a_safe_path_with_no_file_behind_it() {
    // A `..`-free path that simply does not exist reaches the filesystem probe and
    // comes back 404 — the same answer the traversal guard gives, so an attacker
    // learns nothing from the difference between "refused" and "absent".
    assert_all_tiers(
        r#"## Main
Let r be staticUnder("/tmp", "logos_static_c2_definitely_absent.wasm").
Show r's status.
Show r's body.
"#,
        "404\nNot Found",
    );
}
