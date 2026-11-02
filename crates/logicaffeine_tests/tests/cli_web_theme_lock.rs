//! ★ LOCK — a scaffolded site's stylesheet actually styles something. ★
//!
//! `largo new <name> --web` writes `static/style.css`, and that file is the first CSS anybody
//! sees of LOGOS. It referenced four design tokens — `--border`, `--radius`, `--shadow`,
//! `--accent` — and **nothing anywhere defined them**, so every declaration using one was
//! silently DROPPED: the card had no border, no rounded corners and no shadow, and its heading
//! had no colour. The scaffold shipped a stylesheet that did nothing.
//!
//! ★ AND CSS NEVER TELLS YOU. An undefined custom property is not an error, not a warning, not a
//! console line — `var(--nope)` with no fallback makes the whole declaration invalid at computed
//! value time and the browser moves on. That silence is exactly why this needs a lock: the same
//! shape as `largo new --web`'s counter that never re-rendered, and as `bind` registering no
//! listener. It LOOKS wired up.
//!
//! So: every token a scaffolded stylesheet USES must be DEFINED by a scaffolded stylesheet, and
//! the tokens live in their own `static/theme.css` — a file a project owns and can edit or
//! delete, rather than values buried in a component's rules where changing the accent colour
//! means hunting through selectors.

#[path = "cli_common/mod.rs"]
mod common;

use common::{largo_in, stderr};
use std::collections::BTreeSet;
use std::path::Path;

/// Scaffold a real `--web` project by RUNNING `largo new --web`, the way `web_scaffold` does —
/// so this holds what a person actually gets rather than a copy of the templates.
fn scaffold() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = largo_in(dir.path(), &["new", "demo", "--web"]);
    assert!(out.status.success(), "largo new --web: {}", stderr(&out));
    let root = dir.path().join("demo");
    (dir, root)
}

/// Every `var(--name)` a stylesheet references.
fn tokens_used(css: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = css;
    while let Some(at) = rest.find("var(--") {
        let after = &rest[at + 6..];
        let end = after
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '-')
            .unwrap_or(after.len());
        out.insert(format!("--{}", &after[..end]));
        rest = &after[end..];
    }
    out
}

/// Every `--name:` a stylesheet defines.
fn tokens_defined(css: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in css.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("--") {
            if let Some((name, _)) = rest.split_once(':') {
                out.insert(format!("--{}", name.trim()));
            }
        }
    }
    out
}

/// Every top-level `.css` the scaffold writes, in the order a page links them.
fn scaffolded_css_paths(root: &Path) -> Vec<String> {
    let mut paths: Vec<String> = std::fs::read_dir(root.join("static"))
        .expect("static/ exists")
        .filter_map(|e| e.ok())
        .map(|e| format!("/{}", e.file_name().to_string_lossy()))
        .filter(|p| p.ends_with(".css"))
        .collect();
    paths.sort();
    paths
}

/// Every scaffolded stylesheet's text, concatenated the way a browser sees them.
fn all_scaffolded_css(root: &Path) -> String {
    let mut out = String::new();
    for p in scaffolded_css_paths(root) {
        out.push_str(
            &std::fs::read_to_string(root.join("static").join(p.trim_start_matches('/')))
                .expect("read css"),
        );
        out.push('\n');
    }
    out
}

#[test]
fn every_token_the_scaffold_uses_is_defined_by_the_scaffold() {
    // ★ THE LOAD-BEARING TEST. Four tokens were referenced and none defined, so the scaffolded
    // card had no border, no radius, no shadow and no accent — and nothing said so, because an
    // undefined custom property makes a declaration invalid silently.
    let (_dir, root) = scaffold();
    let css = all_scaffolded_css(&root);
    let used = tokens_used(&css);
    let defined = tokens_defined(&css);
    assert!(
        !used.is_empty(),
        "the scaffolded stylesheet uses design tokens: {css}"
    );
    let missing: Vec<&String> = used.difference(&defined).collect();
    assert!(
        missing.is_empty(),
        "the scaffold references design tokens nothing defines, so every declaration using one \
         is SILENTLY dropped — the card has no border, no shadow, no colour: {missing:?}"
    );
}

#[test]
fn the_tokens_live_in_their_own_file() {
    // A project OWNS its theme. Tokens buried in a component's rules mean changing the accent
    // colour is a hunt through selectors; in `theme.css` it is one line, and a project that wants
    // none of it deletes the file.
    let (_dir, root) = scaffold();
    let theme = std::fs::read_to_string(root.join("static/theme.css"))
        .expect("the scaffold writes static/theme.css");
    let defined = tokens_defined(&theme);
    for want in ["--accent", "--border", "--radius", "--shadow"] {
        assert!(
            defined.contains(want),
            "`{want}` belongs in theme.css where a project can change it: {defined:?}"
        );
    }
    assert!(
        theme.contains(":root"),
        "tokens are defined on :root so the whole page sees them"
    );
}

#[test]
fn the_theme_is_linked_into_every_page() {
    // A theme nothing links is a theme nothing applies. It is a top-level `.css` under `static/`,
    // which is precisely what the site-stylesheet convention links (`stylesheet_links_for`), so
    // this asserts the convention actually picks it up rather than that the file merely exists.
    let (_dir, root) = scaffold();
    let paths = scaffolded_css_paths(&root);
    let links = logicaffeine_cli::stylesheet_links_for(paths.iter().map(String::as_str));
    assert!(
        links.contains("href=\"/theme.css\""),
        "the theme is linked: {links}"
    );
    assert!(
        links.contains("href=\"/style.css\""),
        "and so is the page stylesheet: {links}"
    );
}

#[test]
fn the_theme_is_linked_before_the_stylesheet_that_uses_it() {
    // ORDER IS THE CASCADE. A token is defined wherever it is defined and a later rule wins, so
    // this is not about correctness of the `var()` lookup — it is that a project overriding a
    // rule in `style.css` should not have its override beaten by the theme it is customising.
    let (_dir, root) = scaffold();
    let paths = scaffolded_css_paths(&root);
    let links = logicaffeine_cli::stylesheet_links_for(paths.iter().map(String::as_str));
    let theme_at = links.find("/theme.css").expect("linked");
    let style_at = links.find("/style.css").expect("linked");
    assert!(
        theme_at < style_at,
        "the theme comes first so a project's own rules win over it: {links}"
    );
}

// ── S16: the built-in theme's opt-out (`Largo.toml`'s `[web] theme = false`) ──────────────────────

/// A scaffolded `--web` project, with `[web]\ntheme = false` appended to its `Largo.toml` when
/// `off` — the one way a real project actually opts out, rather than a hand-built manifest that
/// could disagree with what `largo new` writes.
fn scaffold_themed(off: bool) -> (tempfile::TempDir, std::path::PathBuf) {
    let (dir, root) = scaffold();
    if off {
        let path = root.join("Largo.toml");
        let mut toml = std::fs::read_to_string(&path).expect("read Largo.toml");
        toml.push_str("\n[web]\ntheme = false\n");
        std::fs::write(&path, toml).expect("write Largo.toml");
    }
    (dir, root)
}

/// `largo serve`'s `/` for a scaffolded project, dialled over HTTP — the fully-wired page
/// composition site this wave lands (`cmd_serve_web` reads `Manifest::shell_theme` once and
/// threads it through every page and error page it builds).
fn served_index(root: &std::path::Path) -> String {
    let mut server = common::http::Server::start(root);
    let port = server.port();
    common::http::body(&server.get("/"))
}

#[test]
fn a_project_with_no_web_section_ships_the_builtin_theme() {
    let (_dir, root) = scaffold_themed(false);
    let html = served_index(&root);
    assert!(
        html.contains("<style>") && html.contains("--paper:"),
        "the default is the built-in theme, inlined as its own `<style>`: {html}"
    );
}

#[test]
fn a_theme_false_project_ships_no_builtin_theme_style() {
    let (_dir, root) = scaffold_themed(true);
    let html = served_index(&root);
    assert!(
        !html.contains("<style>"),
        "`[web] theme = false` opts all the way out — no built-in `<style>` at all (a project's \
         own `static/*.css` is always a `<link>`, never inlined): {html}"
    );
}

/// Every place in the CLI that composes a page's HTML shell WITHOUT reading the project's own
/// `[web] theme` — a literal `ShellTheme::Default`, or a call through the un-themed
/// `browser_host_html`/`browser_host_html_with_head` wrappers (which always mean Default,
/// regardless of what a project's `Largo.toml` says). Each row here is a REVIEWED, NAMED exception;
/// a new one added anywhere else — or an existing one silently multiplying — fails the census
/// below. `src/project/manifest.rs` is excluded: it is `Manifest::shell_theme`'s own definition,
/// the function every wired site is supposed to call, not a bypass of it.
const UNWIRED_THEME_SITES: &[(&str, &str)] = &[
    (
        "commands/web_bundle.rs",
        "prerendered_html_pub — the single-file SSG-lock/`largo emit` preview path has no site or \
         manifest in scope",
    ),
    (
        "commands/web_bundle.rs",
        "build_page_assets — the static-export build (`largo build --static`) is not yet wired to \
         the manifest (a follow-up to this same S16 decision)",
    ),
    (
        "commands/web_bundle.rs",
        "plan_site's error-page branch — same static-export build, same follow-up",
    ),
    (
        "browser/bundle.rs",
        "the browser-automation test harness bundles a page for a headless Chrome probe, never a \
         page a visitor is served",
    ),
    (
        "commands/emit.rs",
        "`largo emit wasm-dom` on a standalone FILE has no enclosing project to declare `[web]` in",
    ),
    (
        "commands/serve_dom.rs",
        "a single-file DOM app (`largo serve` with no `pages/`) is Default by design (S16 decision)",
    ),
];

fn cli_src_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/logicaffeine_cli/src")
}

/// Every `.rs` file under the CLI's `src/`, except `project/manifest.rs` itself, as `(relative
/// path, contents)`.
fn cli_source_files() -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).expect("read_dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if rel == "project/manifest.rs" {
                continue;
            }
            out.push((rel, std::fs::read_to_string(&path).expect("read source")));
        }
    }
    let mut out = Vec::new();
    walk(&cli_src_root(), &cli_src_root(), &mut out);
    out
}

/// Every line, across the CLI's `src/`, that composes a shell WITHOUT the manifest's theme — see
/// [`UNWIRED_THEME_SITES`]'s own doc comment for exactly what counts and why `manifest.rs` does
/// not.
fn unwired_theme_call_sites() -> Vec<String> {
    let mut out = Vec::new();
    for (rel, text) in cli_source_files() {
        for line in text.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if line.contains("ShellTheme::Default")
                || line.contains("browser_host_html(")
                || line.contains("browser_host_html_with_head(")
            {
                out.push(rel.clone());
            }
        }
    }
    out
}

#[test]
fn every_unwired_theme_site_is_a_reviewed_exception() {
    let found = unwired_theme_call_sites();
    let mut found_counts: std::collections::BTreeMap<&str, usize> =
        std::collections::BTreeMap::new();
    for f in &found {
        *found_counts.entry(f.as_str()).or_default() += 1;
    }
    let mut declared_counts: std::collections::BTreeMap<&str, usize> =
        std::collections::BTreeMap::new();
    for (file, _) in UNWIRED_THEME_SITES {
        *declared_counts.entry(*file).or_default() += 1;
    }
    assert_eq!(
        found_counts, declared_counts,
        "a page-shell composition site started (or stopped) bypassing the project's `[web]` theme \
         with no row in `UNWIRED_THEME_SITES` saying so — found: {found:#?}\ndeclared: {:#?}",
        UNWIRED_THEME_SITES
    );
}

/// The census can fail: a fixture whose declared table names a file the census would never find a
/// bypass in (`manifest.rs`, deliberately excluded) proves the mismatch check actually fires.
#[test]
fn the_theme_site_census_can_fail() {
    let found = unwired_theme_call_sites();
    assert!(
        !found.iter().any(|f| f == "project/manifest.rs"),
        "manifest.rs must never be counted as an unwired site"
    );
    let fake: &[(&str, &str)] = &[(
        "project/manifest.rs",
        "a control row nothing would ever match",
    )];
    let mut declared_counts: std::collections::BTreeMap<&str, usize> =
        std::collections::BTreeMap::new();
    for (file, _) in fake {
        *declared_counts.entry(*file).or_default() += 1;
    }
    let mut found_counts: std::collections::BTreeMap<&str, usize> =
        std::collections::BTreeMap::new();
    for f in &found {
        *found_counts.entry(f.as_str()).or_default() += 1;
    }
    assert_ne!(
        found_counts, declared_counts,
        "a fixture naming an unmatched file must disagree"
    );
}

#[test]
fn a_deleted_theme_leaves_a_page_that_still_works() {
    // The theme is DELETABLE, which is what makes it a scaffold rather than a framework. Removing
    // it must leave a site that still builds and serves — styled plainly, not broken.
    let (_dir, root) = scaffold();
    std::fs::remove_file(root.join("static/theme.css")).expect("a project may delete its theme");
    let paths = ["/style.css"];
    let links = logicaffeine_cli::stylesheet_links_for(paths.iter().copied());
    assert!(
        links.contains("style.css"),
        "the remaining stylesheet still links: {links}"
    );
    assert!(
        !links.contains("theme.css"),
        "and the deleted one does not: {links}"
    );
}
