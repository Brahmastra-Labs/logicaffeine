//! ★ LOCK — a page is pre-rendered AT its own URL. ★
//!
//! `largo build` writes each page's first paint into its document, and a reader the module never
//! reaches (a crawler, a slow connection, scripts off) sees exactly that markup. A page declared at
//! `## Page /about` must therefore render as a visitor who navigated to `/about` finds it:
//! `routePath()` answers `/about` during the render, so a layout's active-link binding marks About,
//! not Home. The `## Test` tiers already boot a page at its declared path (`testrun`,
//! `page_host::Scenario`); this lock holds the static export to the same rule.

use std::path::Path;

/// The markup inside the pre-rendered document's `<body …>`.
fn body(document: &str) -> &str {
    let start = document.find("<body").expect("the document has a body");
    let open_end = start + document[start..].find('>').expect("the body tag closes") + 1;
    let end = document
        .rfind("</body>")
        .expect("the document closes its body");
    &document[open_end..end]
}

/// The opening tag of the element carrying `id="<id>"`.
fn tag_of<'a>(markup: &'a str, id: &str) -> &'a str {
    let at = markup
        .find(&format!("id=\"{id}\""))
        .unwrap_or_else(|| panic!("no element #{id} in:\n{markup}"));
    let open = markup[..at].rfind('<').expect("the id sits inside a tag");
    let close = at + markup[at..].find('>').expect("the tag closes");
    &markup[open..=close]
}

fn is_active(tag: &str) -> bool {
    tag.split("class=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .is_some_and(|classes| classes.split_whitespace().any(|c| c == "active"))
}

#[test]
fn route_path_answers_the_pages_own_declared_path_during_the_prerender() {
    let source = "## Page /about\n\nView:\n    <p id=\"route\">{routePath()}</p>\n";
    let document = logicaffeine_cli::prerender_page_html(source, "about");
    let markup = body(&document);
    assert!(
        markup.contains(">/about<"),
        "a page declared at /about pre-renders with routePath() = \"/about\", not the boot default \"/\":\n{markup}"
    );
}

/// Every `<a …>` in `markup`: its `href` and whether its class list holds `active`.
fn links(markup: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut rest = markup;
    while let Some(at) = rest.find("<a ") {
        let tag_end = rest[at..]
            .find('>')
            .map(|e| at + e + 1)
            .unwrap_or(rest.len());
        let tag = &rest[at..tag_end];
        if let Some(href) = tag
            .split("href=\"")
            .nth(1)
            .and_then(|r| r.split('"').next())
        {
            out.push((href.to_string(), is_active(tag)));
        }
        rest = &rest[tag_end..];
    }
    out
}

/// The URL the `## Page` of `source` declares, if the program auto-mounts one.
fn declared_path(source: &str) -> Option<String> {
    logicaffeine_compile::ui_bridge::with_parsed_program(source, |parsed, interner| {
        parsed.ok().and_then(|(stmts, ..)| {
            logicaffeine_compile::page_host::auto_mounted_page_path(stmts, interner)
        })
    })
}

/// The invariant over every example that marks its current page's link: in each page's pre-render the
/// `active` links all point at that page's own declared URL, and the page's own link, where the page
/// shows one, is among them. Discovered, not listed, so a new site example is held to it the day it
/// lands.
#[test]
fn every_example_page_marks_exactly_its_own_link_active_in_its_prerender() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut projects: Vec<_> = std::fs::read_dir(&examples)
        .expect("examples/ is readable")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("Largo.toml").is_file())
        .collect();
    projects.sort();
    let mut held = Vec::new();
    for project in projects {
        let Ok(dir) = std::fs::read_dir(project.join("pages")) else {
            continue;
        };
        let mut pages: Vec<_> = dir
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "lg"))
            .collect();
        pages.sort();
        for page in pages {
            let source = logicaffeine_compile::analysis::assemble_project(&page)
                .unwrap_or_else(|e| panic!("{} assembles: {e}", page.display()))
                .source;
            if source.contains("## Error page") {
                continue;
            }
            let Some(path) = declared_path(&source).filter(|p| !p.contains(':')) else {
                continue;
            };
            let stem = page.file_stem().unwrap().to_string_lossy().into_owned();
            let document = logicaffeine_cli::prerender_page_html(&source, &stem);
            let found = links(body(&document));
            let active: Vec<&String> = found.iter().filter(|(_, a)| *a).map(|(h, _)| h).collect();
            if active.is_empty() {
                continue;
            }
            let name = format!(
                "{}/{}",
                project.file_name().unwrap().to_string_lossy(),
                page.file_name().unwrap().to_string_lossy()
            );
            for href in &active {
                assert_eq!(**href, path, "{name}: only the page's own link ({path}) is active in its pre-render; {href} is too");
            }
            if found.iter().any(|(h, _)| *h == path) {
                assert!(
                    active.iter().any(|h| **h == path),
                    "{name}: its own link ({path}) is active in its pre-render"
                );
            }
            held.push(name);
        }
    }
    for expected in [
        "multi_page_site/about.lg",
        "marketing_site/about.lg",
        "marketing_site/writing.lg",
    ] {
        assert!(
            held.iter().any(|n| n == expected),
            "the sweep reaches {expected}; it held {held:?}"
        );
    }
}

#[test]
fn the_multi_page_example_marks_each_pages_own_link_active_in_its_prerender() {
    let pages = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/multi_page_site/pages");
    for (file, stem, active, inactive) in [
        ("app.lg", "app", "nav-home", ["nav-about", "nav-contact"]),
        (
            "about.lg",
            "about",
            "nav-about",
            ["nav-home", "nav-contact"],
        ),
        (
            "contact.lg",
            "contact",
            "nav-contact",
            ["nav-home", "nav-about"],
        ),
    ] {
        let source = logicaffeine_compile::analysis::assemble_project(&pages.join(file))
            .unwrap_or_else(|e| panic!("{file} assembles: {e}"))
            .source;
        let document = logicaffeine_cli::prerender_page_html(&source, stem);
        let markup = body(&document);
        assert!(
            is_active(tag_of(markup, active)),
            "{file}: #{active} is the active link in the pre-render:\n{markup}"
        );
        for other in inactive {
            assert!(
                !is_active(tag_of(markup, other)),
                "{file}: #{other} is not active in the pre-render:\n{markup}"
            );
        }
    }
}
