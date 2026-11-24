//! ★ LOCK — a parameterized page is BUILT once per value it declares. ★
//!
//! `## Page /learn/:chapter (chapter: Text)` reads its parameter from its own address, which is
//! how one file serves a whole family of URLs. A static export had no way to know which members of
//! the family exist, so it published one document at `/learn/chapter`, pre-rendered for an EMPTY
//! parameter, and a rewrite rule pointing every chapter at it: a crawler, a link preview and a
//! reader with scripts off all saw the same blank chapter, and the right one appeared only once the
//! module ran.
//!
//! ★ A PAGE SAYS WHERE IT IS BUILT, IN LOGOS. A `Paths:` section returns the page's URLs as a
//! `Seq of Text`, usually computed from the same data the page renders, so a chapter added to the
//! data is a page added to the site. Each URL is pre-rendered AT that URL — the page binds its
//! parameter exactly as it does in a browser — and published as an exact page, joining the
//! sitemap. The pattern document and its rewrite stay as the fallback for a value the list did not
//! name, which is the routing rule every delivery path already follows: an exact page first, then
//! the pattern.
//!
//! Every URL is checked at build time against the page's own pattern, for duplicates, for an `Int`
//! parameter that would not parse, and for a collision with another page — each a loud error that
//! names the URL, because a page built at the wrong address is invisible until someone links it.

#[path = "cli_common/mod.rs"]
mod common;

use common::{largo_in, stderr};
use std::path::{Path, PathBuf};

/// A static site with `pages` (file name → source) and, optionally, a declared site URL.
fn site(dir: &Path, pages: &[(&str, &str)], site_url: Option<&str>) -> PathBuf {
    let root = dir.join("site");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let site_line = site_url.map(|u| format!("site = \"{u}\"\n")).unwrap_or_default();
    std::fs::write(
        root.join("Largo.toml"),
        format!(
            "[package]\nname = \"site\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n{site_line}"
        ),
    )
    .unwrap();
    std::fs::write(root.join("src/main.lg"), "## Main\n\nShow \"ok\".\n").unwrap();
    for (name, src) in pages {
        std::fs::write(root.join("pages").join(name), src).unwrap();
    }
    root
}

/// `largo build --static`, answering `dist/` or the build's stderr.
fn export(root: &Path) -> Result<PathBuf, String> {
    let out = largo_in(root, &["build", "--static"]);
    if out.status.success() {
        Ok(root.join("dist"))
    } else {
        Err(stderr(&out))
    }
}

fn read(dist: &Path, rel: &str) -> String {
    std::fs::read_to_string(dist.join(rel)).unwrap_or_else(|e| {
        panic!(
            "dist/{rel} must exist: {e}\ndist holds:\n  {}",
            walk(dist).join("\n  ")
        )
    })
}

fn walk(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(dir) {
                out.push(rel.to_string_lossy().to_string());
            }
        }
    }
    out.sort();
    out
}

/// The chapter page with its URLs written out.
const LITERAL: &str = "## Page /learn/:chapter (chapter: Text)\n\n\
Paths:\n    Return [\"/learn/intro\", \"/learn/state\"].\n\n\
View:\n    <main><h1 id=\"t\">chapter {chapter}</h1></main>\n";

/// The same page with its URLs computed from the data it renders.
const DATA: &str = "## A Chapter has:\n    A slug: Text.\n    A title: Text.\n\n\
## To chapters -> Seq of Chapter:\n    Return [(a new Chapter with slug \"intro\" and title \"Intro\"), (a new Chapter with slug \"state\" and title \"State\")].\n\n\
## To titleOf (slug: Text) -> Text:\n    Repeat for c in chapters():\n        If c's slug equals slug:\n            Return c's title.\n    Return \"Not found\".\n\n\
## Page /learn/:chapter (chapter: Text)\n\n\
Paths:\n    Let out be a new Seq of Text.\n    Repeat for c in chapters():\n        Push \"/learn/{c's slug}\" to out.\n    Return out.\n\n\
View:\n    <main><h1 id=\"t\">{titleOf(chapter)}</h1></main>\n";

/// The attribute a pre-rendered document's `<body>` carries, which its mount adopts by.
fn prerendered_mark() -> String {
    format!(
        "<body {}=\"1\">",
        logicaffeine_language::parser::lgx_lower::PRERENDERED_ATTR
    )
}

/// ★ THE LOAD-BEARING TEST — each declared URL is its own pre-rendered document.
#[test]
fn each_declared_url_is_built_as_its_own_prerendered_page() {
    let dir = tempfile::tempdir().unwrap();
    let dist = export(&site(dir.path(), &[("chapter.lg", LITERAL)], None)).expect("the site builds");
    let intro = read(&dist, "learn/intro/index.html");
    let state = read(&dist, "learn/state/index.html");
    assert!(
        intro.contains("<h1 id=\"t\">chapter intro</h1>"),
        "rendered AT /learn/intro:\n{intro}"
    );
    assert!(
        state.contains("<h1 id=\"t\">chapter state</h1>"),
        "rendered AT /learn/state:\n{state}"
    );
    assert!(
        intro.contains(&prerendered_mark()),
        "marked for its mount to adopt, like any pre-rendered page:\n{intro}"
    );
    assert_eq!(
        read(&dist, "learn/intro.html"),
        intro,
        "and served at its `.html` twin too, the rule every page follows"
    );
}

#[test]
fn a_page_builds_the_urls_its_own_data_names() {
    let dir = tempfile::tempdir().unwrap();
    let dist = export(&site(dir.path(), &[("chapter.lg", DATA)], None)).expect("the site builds");
    assert!(read(&dist, "learn/intro/index.html").contains("<h1 id=\"t\">Intro</h1>"));
    assert!(read(&dist, "learn/state/index.html").contains("<h1 id=\"t\">State</h1>"));
}

#[test]
fn every_declared_page_loads_the_one_shared_module() {
    // One page, one module: the built documents differ in their markup, not in their code.
    let dir = tempfile::tempdir().unwrap();
    let dist = export(&site(dir.path(), &[("chapter.lg", LITERAL)], None)).expect("the site builds");
    let module_of = |doc: &str| {
        doc.split("<script type=\"module\" src=\"")
            .nth(1)
            .and_then(|r| r.split('"').next())
            .map(str::to_string)
            .unwrap_or_else(|| panic!("the page loads a module:\n{doc}"))
    };
    let a = module_of(&read(&dist, "learn/intro/index.html"));
    let b = module_of(&read(&dist, "learn/state/index.html"));
    assert_eq!(a, b, "both documents load the same shim");
    let shim = a.rsplit('/').next().unwrap().to_string();
    assert!(
        walk(&dist).iter().any(|f| f.ends_with(&shim)),
        "and the shim they name is published: {shim}\n{:?}",
        walk(&dist)
    );
}

#[test]
fn the_pattern_fallback_and_its_rewrite_still_ship() {
    // A value the list did not name still reaches the page — through the pattern, as before.
    let dir = tempfile::tempdir().unwrap();
    let dist = export(&site(dir.path(), &[("chapter.lg", LITERAL)], None)).expect("the site builds");
    assert!(
        dist.join("learn/chapter/index.html").is_file(),
        "the fallback document: {:?}",
        walk(&dist)
    );
    let rules = read(&dist, "_redirects");
    assert!(rules.contains("/learn/:chapter"), "the rewrite for undeclared values:\n{rules}");
}

#[test]
fn the_release_binary_serves_each_declared_url_exactly() {
    // The binary serves from the same planned table, so a declared URL is an exact row there — the
    // routing rule's first step — and the pattern stays for everything else.
    let dir = tempfile::tempdir().unwrap();
    let root = site(dir.path(), &[("chapter.lg", LITERAL)], None);
    let assets = logicaffeine_cli::plan_site_assets(&root.join("pages"), &root.join("static"))
        .expect("the site plans");
    let row = |p: &str| {
        assets
            .iter()
            .find(|(path, _, _)| path == p)
            .map(|(_, _, bytes)| String::from_utf8_lossy(bytes).to_string())
    };
    let intro = row("/learn/intro").expect("an exact row for /learn/intro");
    assert!(intro.contains("chapter intro"), "{intro}");
    assert!(row("/learn/intro.html").is_some(), "and its twin");
    assert!(row("/learn/chapter").is_some(), "and the pattern's own document");
}

#[test]
fn the_sitemap_lists_each_declared_url_and_not_the_placeholder() {
    let dir = tempfile::tempdir().unwrap();
    let dist = export(&site(
        dir.path(),
        &[("chapter.lg", LITERAL)],
        Some("https://acme.example"),
    ))
    .expect("the site builds");
    let map = read(&dist, "sitemap.xml");
    assert!(map.contains("<loc>https://acme.example/learn/intro</loc>"), "{map}");
    assert!(map.contains("<loc>https://acme.example/learn/state</loc>"), "{map}");
    assert!(
        !map.contains("/learn/chapter"),
        "the placeholder is not a page anybody can link — listing it asks a crawler to index a \
         chapter called `chapter`:\n{map}"
    );
}

#[test]
fn a_url_the_pattern_does_not_match_is_refused_by_name() {
    let page = LITERAL.replace("\"/learn/state\"", "\"/blog/state\"");
    let dir = tempfile::tempdir().unwrap();
    let err = export(&site(dir.path(), &[("chapter.lg", &page)], None))
        .expect_err("a URL outside the page's own pattern is a build error");
    assert!(err.contains("/blog/state"), "names the URL: {err}");
    assert!(err.contains("/learn/:chapter"), "and the pattern it had to match: {err}");
    assert!(err.contains("pages/chapter.lg"), "and the file: {err}");
}

#[test]
fn a_url_declared_twice_is_refused() {
    let page = LITERAL.replace("\"/learn/state\"", "\"/learn/intro\"");
    let dir = tempfile::tempdir().unwrap();
    let err = export(&site(dir.path(), &[("chapter.lg", &page)], None))
        .expect_err("one URL, one page — declared twice is a mistake to say out loud");
    assert!(err.contains("/learn/intro") && err.contains("twice"), "{err}");
}

#[test]
fn an_int_parameter_must_read_as_a_whole_number() {
    let page = "## Page /posts/:id (id: Int)\n\nPaths:\n    Return [\"/posts/7\", \"/posts/seven\"].\n\n\
        View:\n    <main><h1>post {id}</h1></main>\n";
    let dir = tempfile::tempdir().unwrap();
    let err = export(&site(dir.path(), &[("post.lg", page)], None))
        .expect_err("`seven` is not an Int, and the page would trap reading it");
    assert!(err.contains("/posts/seven") && err.contains("Int"), "{err}");
}

#[test]
fn an_int_parameter_page_is_built_at_each_number() {
    let page = "## Page /posts/:id (id: Int)\n\nPaths:\n    Return [\"/posts/7\", \"/posts/8\"].\n\n\
        View:\n    <main><h1>post {id + 1}</h1></main>\n";
    let dir = tempfile::tempdir().unwrap();
    let dist = export(&site(dir.path(), &[("post.lg", page)], None)).expect("the site builds");
    assert!(
        read(&dist, "posts/7/index.html").contains("<h1>post 8</h1>"),
        "the parameter is an Int"
    );
    assert!(read(&dist, "posts/8/index.html").contains("<h1>post 9</h1>"));
}

#[test]
fn a_declared_url_another_page_owns_is_a_loud_collision() {
    let dir = tempfile::tempdir().unwrap();
    let err = export(&site(
        dir.path(),
        &[
            ("chapter.lg", LITERAL),
            ("intro.lg", "## Page /learn/intro\n\nView:\n    <main>literal</main>\n"),
        ],
        None,
    ))
    .expect_err("two pages at one URL");
    assert!(
        err.contains("pages/chapter.lg") && err.contains("pages/intro.lg") && err.contains("/learn/intro"),
        "names both files and the URL: {err}"
    );
}

#[test]
fn a_paths_section_needs_a_page_with_parameters() {
    let dir = tempfile::tempdir().unwrap();
    let err = export(&site(
        dir.path(),
        &[(
            "about.lg",
            "## Page /about\n\nPaths:\n    Return [\"/about\"].\n\nView:\n    <main>about</main>\n",
        )],
        None,
    ))
    .expect_err("a page with no `:name` segment has exactly one URL to be built at");
    assert!(
        err.contains("`Paths:`") && err.contains("/about") && err.contains(":name"),
        "the refusal names the section, the page, and what it would need: {err}"
    );
}

#[test]
fn a_paths_section_that_returns_no_text_is_refused() {
    let page = "## Page /posts/:id (id: Int)\n\nPaths:\n    Return 7.\n\n\
        View:\n    <main><h1>post {id}</h1></main>\n";
    let dir = tempfile::tempdir().unwrap();
    let err = export(&site(dir.path(), &[("post.lg", page)], None))
        .expect_err("a `Paths:` answers addresses");
    assert!(err.contains("`Paths:`") && err.contains("pages/post.lg"), "{err}");
}

/// A page with parameters and no `Paths:` builds exactly as before: one document at the pattern's
/// safe form, answering every value through the rewrite.
#[test]
fn a_parameterized_page_without_paths_builds_as_before() {
    let page = "## Page /posts/:id (id: Int)\n\nView:\n    <main><h1>post {id}</h1></main>\n";
    let dir = tempfile::tempdir().unwrap();
    let dist = export(&site(dir.path(), &[("post.lg", page)], None)).expect("the site builds");
    assert!(dist.join("posts/id/index.html").is_file(), "{:?}", walk(&dist));
    assert!(read(&dist, "_redirects").contains("/posts/:id"));
}
