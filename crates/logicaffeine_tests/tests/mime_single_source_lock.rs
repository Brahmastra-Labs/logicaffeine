//! ★ LOCK — every convention `largo serve` and `largo build` share is written ONCE. ★
//!
//! Three of them were written twice, and each pair had drifted or was one edit from drifting.
//! The file is named for the first one because the MIME table is where the pattern was found;
//! what it holds is the rule underneath all three — **development and production must not be two
//! implementations of the same decision**, because the difference only ever shows up in the place
//! nobody can debug.
//!
//!   1. THE MIME TABLE (below) — which content type a path gets.
//!   2. THE STYLESHEET LINKS — which `.css` files a page links.
//!   3. THE PAGE URL NAMESPACE — which URLs a page answers.
//!
//! Each is now ONE function with two callers rather than two functions that agree today.
//!
//! ★ THE MIME TABLE IS ONE TABLE, IN ONE PLACE. ★
//!
//! A content type is what a browser uses to decide whether to RUN a file, render it, or download
//! it. Getting it wrong is not cosmetic: a stylesheet served as `text/plain` is ignored, a wasm
//! module served as `application/octet-stream` fails `instantiateStreaming`, and a page served
//! without a charset renders someone's name as mojibake.
//!
//! TWO different things ask the question. A deployed LOGOS program answers it through
//! `static.lg`'s `contentTypeFor`; `largo serve` and `largo build` answer it as the tool
//! assembling a site. Both are legitimate — what is not legitimate is two ANSWERS, because then a
//! file typed correctly in development is typed wrongly in production, which is the single worst
//! place for a difference to appear.
//!
//! THEY HAD ALREADY DRIFTED when this was written: `static.lg` answered `text/css` where the tool
//! answered `text/css; charset=utf-8`, likewise `.html` and `.txt`, and it knew nothing at all
//! about `.jpg`, `.gif`, `.webp`, `.woff`, `.woff2` or `.pdf` — so a deployed site served its own
//! fonts as `application/octet-stream`.
//!
//! ★★ THE FIX IS STRUCTURAL, NOT A COMPARISON. The first version of this file held two tables
//! against each other, which makes drift *detectable*; keeping the table in ONE PLACE makes it
//! *impossible*. `static.lg` is that place — framework logic belongs in LOGOS, where every tier
//! runs it — and largo READS it (`loader::content_type_for` parses the embedded module's own
//! `contentTypeFor` chain). Adding a type is one edit, in one file, with no second place to
//! remember.
//!
//! So these tests no longer ask "do the two agree". They ask the questions that still have teeth
//! once there is one table: that largo really is READING it (a type only `static.lg` names must
//! come back), that the table says the right things, and that its default is a download rather
//! than a guess.

use std::collections::BTreeMap;

const STATIC_LG: &str = include_str!("../../../crates/logicaffeine_compile/assets/std/static.lg");

/// Every `(extension, content type)` pair `static.lg`'s `contentTypeFor` answers, read out of its
/// own source: the body is a flat `If endsWith(p, ".ext") …: Return "type".` chain, which is the
/// shape a LOGOS reader sees too.
fn logos_table() -> BTreeMap<String, String> {
    let body = STATIC_LG
        .split("## To contentTypeFor")
        .nth(1)
        .expect("static.lg declares contentTypeFor")
        .split("\n## ")
        .next()
        .expect("the function has a body");
    let mut out = BTreeMap::new();
    let mut pending: Vec<String> = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("If endsWith(p, \".") {
            if let Some(ext) = rest.split('"').next() {
                pending.push(ext.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("Return \"") {
            if let Some(ty) = rest.split('"').next() {
                // The trailing bare `Return "application/octet-stream".` is the DEFAULT, not an
                // extension's answer — it has no `If` in front of it.
                for ext in pending.drain(..) {
                    out.insert(ext, ty.to_string());
                }
            }
        }
    }
    out
}

/// The tool's answer, asked the way it is asked in production — through a path.
fn tool(ext: &str) -> String {
    logicaffeine_cli::content_type_for(&format!("/asset.{ext}")).to_string()
}

#[test]
fn largo_reads_the_table_out_of_static_lg() {
    // ★ THE LOAD-BEARING TEST. Every other assertion here would pass just as happily against a
    // second Rust table that happened to agree today. This one only passes if largo is really
    // reading `static.lg`: it asks for a type that appears NOWHERE in largo's own crate, so the
    // only way to answer it is from the LOGOS module.
    let logos = logos_table();
    let woff2 = logos.get("woff2").expect("static.lg types .woff2");
    assert_eq!(
        &tool("woff2"),
        woff2,
        "largo must answer from `static.lg`'s table, not a copy of it"
    );
    // And a type that is NOT in static.lg must fall through, even though it is a perfectly real
    // MIME type — which is what proves the table is the module's rather than the tool's idea.
    assert!(
        !logos.contains_key("mp4"),
        "the fixture assumes static.lg does not type .mp4"
    );
    assert_eq!(
        tool("mp4"),
        "application/octet-stream",
        "largo must not know a type `static.lg` does not — that would be a second table"
    );
}

#[test]
fn every_extension_in_the_table_is_answered() {
    // The parse is only trustworthy if what it reads is what is served. A line `static.lg` names
    // and largo does not answer means the reader has silently stopped matching the file's shape.
    let logos = logos_table();
    assert!(
        logos.len() >= 18,
        "the table was read in full, not partially: {}",
        logos.len()
    );
    let mut wrong = Vec::new();
    for (ext, ty) in &logos {
        let theirs = tool(ext);
        if &theirs != ty {
            wrong.push(format!(
                ".{ext}: static.lg says {ty:?}, largo answered {theirs:?}"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "largo did not answer from the table:\n  {}",
        wrong.join("\n  ")
    );
}

#[test]
fn the_types_a_web_page_actually_needs_are_all_there() {
    // A never-shrink floor on the table's CONTENT. Every one of these is a file a real site
    // ships, and any of them falling back to `application/octet-stream` breaks the page: a wasm
    // module will not stream-instantiate, a stylesheet is ignored, a font silently does not load.
    let logos = logos_table();
    for ext in [
        "html", "htm", "css", "js", "mjs", "wasm", "json", "svg", "png", "jpg", "jpeg", "gif",
        "webp", "ico", "txt", "woff", "woff2", "pdf",
    ] {
        assert!(
            logos.contains_key(ext),
            ".{ext} is a file a real site ships and the table does not type it — it would be \
             served as `application/octet-stream`"
        );
    }
}

#[test]
fn a_text_type_carries_a_charset_and_a_binary_type_does_not() {
    // A text type without a charset lets the browser GUESS the encoding, which renders a
    // visitor's name as mojibake on a page that was otherwise served correctly; a charset on a
    // binary type is meaningless noise that some proxies mishandle.
    for (ext, ty) in logos_table() {
        let has_charset = ty.contains("charset=");
        // ★ `text/javascript` IS THE EXCEPTION, and it is deliberate rather than an oversight.
        // Its own specification defines the type as always UTF-8 and OBSOLETES the charset
        // parameter (RFC 9239), so writing one is noise the spec asks us not to write. Asserting
        // the general rule here would have demanded a change that made the header LESS correct —
        // this test's first version did exactly that, and the table was right.
        if ty.starts_with("text/") && ty != "text/javascript" {
            assert!(
                has_charset,
                ".{ext} is {ty:?} — a text type must name its charset"
            );
        }
        if !ty.starts_with("text/") {
            assert!(
                !has_charset,
                ".{ext} is {ty:?} — a binary type must not carry a charset"
            );
        }
    }
}

#[test]
fn an_unknown_extension_is_a_download_and_never_a_guess() {
    // The default matters as much as the table: guessing `text/html` for an unknown file would
    // let an uploaded file be served as a PAGE, which is a stored-XSS delivery mechanism.
    assert_eq!(tool("unheardof"), "application/octet-stream");
    assert_eq!(tool(""), "application/octet-stream");
    assert!(
        STATIC_LG.contains("Return \"application/octet-stream\"."),
        "static.lg's fallback is the same download-never-guess default"
    );
}

#[test]
fn the_extension_match_is_case_insensitive() {
    // `IMAGE.PNG` off a camera and `Style.CSS` off Windows are ordinary file names, and a type
    // that depended on their capitalisation would work on one machine and not another.
    assert_eq!(tool("PNG"), tool("png"));
    assert_eq!(tool("Css"), tool("css"));
    assert_eq!(tool("WOFF2"), tool("woff2"));
    assert!(
        STATIC_LG.contains("Let p be lower(path)."),
        "static.lg lowercases the path before matching, so both sides agree about case"
    );
}

#[test]
fn a_longer_suffix_wins_over_the_one_it_ends_with() {
    // `.woff2` ends with `2`, not with `.woff` — but `.jpeg`/`.jpg` and any future
    // `.tar.gz`-shaped pair make suffix ORDER load-bearing, and the tool walks the table in the
    // module's own source order for exactly that reason.
    assert_eq!(tool("woff2"), "font/woff2");
    assert_eq!(tool("woff"), "font/woff");
    assert_eq!(tool("jpeg"), "image/jpeg");
    assert_eq!(tool("jpg"), "image/jpeg");
}

// ---------------------------------------------------------------------------------------------
// The other convention `largo serve` and `largo build` must not implement twice.
// ---------------------------------------------------------------------------------------------

#[test]
fn the_dev_server_and_the_build_link_the_same_stylesheets() {
    // ★ THE SAME SHAPE AS THE MIME TABLE, caught next door to it. Which stylesheets a page links
    // was written twice — identically, differing only in the container each caller happened to
    // hold — so a page could end up styled in development and unstyled in production, with
    // nothing saying why. It is one function now; this asks the questions that still have teeth.
    //
    // TOP-LEVEL ONLY is the convention, not an accident: a `.css` at the site root is a SITE
    // stylesheet, while one nested under a directory belongs to whatever put it there and is
    // opted into by the page that wants it. Linking everything would leak a component's private
    // stylesheet onto every page on the site.
    let paths = [
        "/style.css",
        "/extra.css",
        "/deep/nested.css",
        "/logo.png",
        "/theme.CSS",
    ];
    let links = logicaffeine_cli::stylesheet_links_for(paths.iter().copied());
    assert!(
        links.contains("href=\"/style.css\""),
        "a site stylesheet is linked: {links}"
    );
    assert!(
        links.contains("href=\"/extra.css\""),
        "and so is a second one: {links}"
    );
    assert!(
        !links.contains("nested.css"),
        "a nested stylesheet is the page's own business, not the site's: {links}"
    );
    assert!(
        !links.contains("logo.png"),
        "only stylesheets are linked: {links}"
    );
    // Order follows the caller's, so a site controls its cascade — a later rule beating an
    // earlier one is the whole point of a stylesheet, and a set that reordered them would change
    // which rule wins. The ONE exception is `theme.css`, which is a base layer and is asserted
    // below.
    let style_at = links.find("/style.css").expect("linked");
    let extra_at = links.find("/extra.css").expect("linked");
    assert!(
        style_at < extra_at,
        "the caller's order is the cascade's order: {links}"
    );
}

#[test]
fn a_site_with_no_stylesheets_links_none() {
    // The empty case, because it is what a fresh scaffold is: an empty string, not a stray
    // `<link>` pointing at a file that does not exist (which is a 404 on every page load).
    let links = logicaffeine_cli::stylesheet_links_for(["/app.wasm", "/index.html"].into_iter());
    assert!(links.is_empty(), "no stylesheets means no links: {links:?}");
}

#[test]
fn a_built_site_serves_every_url_the_dev_server_does() {
    // ★ A REAL DIVERGENCE THIS CAUGHT. The dev server answered `/index.html` for the front page
    // and the BUILT site did not publish it — its own comment claimed it did — so a link to
    // `/index.html` worked all the way through development and 404'd the moment the site
    // shipped. That is the worst shape a divergence takes: invisible until production, and
    // visible there to everyone.
    //
    // `/index.html` is not an exotic URL. It is what a static host serves for a directory, what
    // a person types when a link goes stale, and what an old bookmark holds.
    // ★ THE FRONT PAGE'S URL IS `/`. It used to ALSO answer `/app` and `/app.html`, which was an
    // artifact of deriving URLs from the file STEM — `pages/app.lg` is where the source lives, not
    // an address anybody chose. Now that `## Page /path` is the source of truth (and the file name
    // is only the default), a page has ONE canonical URL plus its `.html` twin, and the front
    // page's twin is `/index.html` because that is what a static host serves for a directory.
    let front = logicaffeine_cli::page_html_paths("app");
    for want in ["/", "/index.html"] {
        assert!(
            front.iter().any(|p| p == want),
            "the front page must answer {want} — it is one rule now, so dev and prod both do: \
             {front:?}"
        );
    }
}

#[test]
fn only_the_front_page_owns_the_site_root() {
    // A second page must not claim `/` or `/index.html`: two pages answering the root is a site
    // where which one you get depends on enumeration order, which is not a decision anybody made.
    let other = logicaffeine_cli::page_html_paths("about");
    assert!(
        !other.iter().any(|p| p == "/"),
        "only the front page owns the root: {other:?}"
    );
    assert!(
        !other.iter().any(|p| p == "/index.html"),
        "…and its index: {other:?}"
    );
    for want in ["/about", "/about.html"] {
        assert!(
            other.iter().any(|p| p == want),
            "but it owns its own URLs: {other:?}"
        );
    }
}

#[test]
fn a_pages_urls_are_distinct() {
    // The list is used to publish assets, and a repeat would push the same path twice — one
    // shadowing the other under first-match-wins, which is a silent way to serve stale bytes.
    for stem in ["app", "about"] {
        let paths = logicaffeine_cli::page_html_paths(stem);
        let unique: std::collections::BTreeSet<&String> = paths.iter().collect();
        assert_eq!(
            unique.len(),
            paths.len(),
            "`{stem}` publishes a path twice: {paths:?}"
        );
    }
}

#[test]
fn the_dev_server_serves_the_same_document_the_build_ships() {
    // ★ WHAT YOU SEE IN DEVELOPMENT MUST BE WHAT SHIPS. `largo build` pre-renders a page's markup
    // into its document (T6.8: a built page carries its own content, so a crawler, a slow
    // connection and a reader with scripts off all see something). `largo serve` shipped the bare
    // SHELL — one `<script>` tag — so a developer NEVER saw the document their visitors get, and
    // a page the server renderer cannot draw revealed that only at `largo build`, long after the
    // edit that caused it.
    //
    // The dev server is where you look at your page. If it shows you something the build does not
    // produce, it is not showing you your page.
    let page = "## Component App\nState:\n    Let who be \"ada\".\nView:\n    <p id=\"g\">Hi {who}</p>\n\n## Main\nMount App.\n";
    let dev = logicaffeine_cli::dev_page_html(page, "app");
    let built = logicaffeine_cli::prerender_page_html(page, "app");
    assert!(
        built.contains("Hi ada"),
        "the build pre-renders the page's content (T6.8): {built}"
    );
    assert!(
        dev.contains("Hi ada"),
        "and so must the dev server, or a developer never sees the document their visitors get:\n\
         dev: {dev}"
    );
}

#[test]
fn the_dev_server_still_loads_the_module_that_takes_over() {
    // Pre-rendering is what a reader sees FIRST, not instead. The script must still be there, or
    // the page would be a static picture of itself: no clicks, no typing, no reactivity.
    let page = "## Component App\nView:\n    <p>hi</p>\n\n## Main\nMount App.\n";
    let dev = logicaffeine_cli::dev_page_html(page, "app");
    assert!(dev.contains("app.js"), "the host shim still loads: {dev}");
}

#[test]
fn a_page_the_server_renderer_cannot_draw_still_serves_in_dev() {
    // T6.8's rule, held on this side too: pre-rendering is an IMPROVEMENT, never a new way to
    // fail. A page using a construct the server walk does not cover ships its shell and works in
    // the browser exactly as it did — a dev server that went blank on it would be a bad trade.
    let page = "## Component App\nState:\n    Let t be \"x\".\nView:\n    <input id=\"i\" bind={t}>\n\n## Main\nMount App.\n";
    let dev = logicaffeine_cli::dev_page_html(page, "app");
    assert!(
        dev.contains("app.js"),
        "the page still serves and still loads its module: {dev}"
    );
}

#[test]
fn the_theme_is_a_base_layer_whatever_order_it_arrives_in() {
    // Design tokens are the layer a project customises, so they go FIRST and a project's own
    // rules win. Alphabetical order alone puts `style.css` before `theme.css`, which is exactly
    // backwards — the theme would override the rules written to customise it. Handed over in the
    // wrong order on purpose, so this tests the rule rather than the caller's sorting.
    let links = logicaffeine_cli::stylesheet_links_for(
        ["/style.css", "/theme.css", "/zebra.css"].into_iter(),
    );
    let theme_at = links.find("/theme.css").expect("linked");
    let style_at = links.find("/style.css").expect("linked");
    let zebra_at = links.find("/zebra.css").expect("linked");
    assert!(theme_at < style_at, "the theme is the base layer: {links}");
    assert!(
        style_at < zebra_at,
        "and everything else keeps the caller's order behind it: {links}"
    );
}
