//! ★ LOCK — a file dropped by its name alone has the type a browser gives that name, on every tier. ★
//!
//! `Raise "filedropped" on "#zone" with "notes.txt".` is the drive sentence `docs/frontend.md`
//! teaches. In Chrome the drop carries a real file, and a browser names a file's type from its
//! name: `notes.txt` is `text/plain`. A page that shows `file's type` shows a visitor what
//! `largo test` saw only when every tier hands the handler that same type.
//!
//! A real browser is the reference for what a visitor's browser reports. So the tiers that
//! synthesize a file from a name derive its type exactly as a browser does, through ONE mapping
//! (`semantics::dom::browser_mime_type`, held to the same table by `browser_mime_type_lock`):
//! Chromium's own two lists. Between those lists a browser asks the visitor's desktop — on Linux
//! the shared MIME database — for an extension the primary list does not name, and that layer is
//! the machine's, not the table's. `support::dropped_names` keeps the names the table settles
//! (`DROPS`) apart from the names a desktop may settle (`DESKTOP_DROPS`), and this file holds each
//! to what is true of it: the first agree on every tier, the second agree on every in-process tier
//! and part from Chrome exactly where this desktop names a file otherwise, and never anywhere else.
//!
//! Chrome is the oracle. The expected types are written out so that a browser that starts naming a
//! file differently fails HERE, against the table, rather than silently re-teaching the tiers; and
//! every tier is then held to the reference on the document and the journal. A type the author
//! DECLARES is theirs on the tiers that synthesize the file; the browser tier hands the page only
//! what a browser names the file, and refuses a declaration Chrome does not give the name.

#[path = "cli_common/mod.rs"]
mod common;

#[path = "support/dropped_names.rs"]
mod dropped_names;

use dropped_names::{desktop_types, DESKTOP_DROPS, DROPS};
use logicaffeine_cli::browser::{self, Browser};
use logicaffeine_compile::page_host::{
    compare_tests, Axis, Divergence, PageTierKind, PageTrace, TestTierComparison, TierRun,
};

/// The browser this binary shares, or `None` (recorded as a skip) on a machine without Chrome.
/// `LOGOS_REQUIRE_TOOLS=chrome` makes a missing browser a failure instead.
fn browser() -> Option<&'static Browser> {
    let Some(chrome) = browser::locate_chrome() else {
        let spec = std::env::var("LOGOS_REQUIRE_TOOLS").unwrap_or_default();
        common::require_tool_strict("chrome", &spec);
        return None;
    };
    Some(browser::shared(&chrome).unwrap_or_else(|e| {
        panic!(
            "Chrome is installed at {} and must start: {e}",
            chrome.display()
        )
    }))
}

/// Every `## Test` of `source`, in source order, run through `page_host::compare_tests` on the
/// reference tier, the lowered VM and Chrome; `None` (recorded as a skip) without Chrome.
fn compared(source: &str) -> Option<Vec<TestTierComparison>> {
    browser()?;
    Some(
        compare_tests(
            source,
            &[PageTierKind::LoweredVm, PageTierKind::Browser],
            0,
            50_000_000,
            Some(&browser::ChromeLauncher::default()),
        )
        .unwrap_or_else(|e| panic!("the page must parse: {e}\n{source}")),
    )
}

/// `kind`'s run of `c`.
fn run(c: &TestTierComparison, kind: PageTierKind) -> &TierRun {
    c.candidates
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, run)| run)
        .unwrap_or_else(|| panic!("the {} tier ran `{}`", kind.label(), c.test))
}

/// `kind`'s trace of `c`, which it must have traced rather than refused.
fn traced(c: &TestTierComparison, kind: PageTierKind) -> &PageTrace {
    match run(c, kind) {
        TierRun::Traced(trace) => trace,
        other => panic!(
            "the {} tier must trace `{}`, not {other:?}",
            kind.label(),
            c.test
        ),
    }
}

/// Where `candidate` disagrees with `reference` on `axes`, rendered.
fn disagreements(reference: &PageTrace, candidate: &PageTrace, axes: &[Axis]) -> String {
    let d = reference.compare(candidate);
    let kept: Vec<_> = d
        .axes
        .into_iter()
        .filter(|a| axes.contains(&a.axis))
        .collect();
    Divergence { axes: kept }.render()
}

/// The page every scenario drives: a drop zone that keeps each dropped file's name, type and size.
const PAGE: &str = r##"## Page /

State:
    Let names be a new Seq of Text.
    Let types be a new Seq of Text.
    Let sizes be a new Seq of Int.

Actions:
    When a file is dropped on "#zone" (file: File):
        Push file's name to names.
        Push file's type to types.
        Push file's size to sizes.

View:
    <main>
      <div id="zone">drop here</div>
      <ul id="names">{for each n in names:}<li>{n}</li>{end}</ul>
      <ul id="types">{for each t in types:}<li>{t}</li>{end}</ul>
      <ul id="sizes">{for each s in sizes:}<li>{s}</li>{end}</ul>
    </main>
"##;

/// A `## Test` named `title` that drops each of `files` — a drive operand as written — on the zone.
fn dropping<'a>(title: &str, files: impl Iterator<Item = &'a str>) -> String {
    let mut test = format!("## Test: {title}\n");
    for file in files {
        test.push_str(&format!(
            "    Raise \"filedropped\" on \"#zone\" with {file}.\n"
        ));
    }
    test
}

/// The name `name` as a drive operand.
fn quoted(name: &str) -> String {
    format!("\"{name}\"")
}

/// The list `id` shows once every item has arrived, as the page draws it.
fn list(id: &str, items: impl Iterator<Item = String>) -> String {
    let items: String = items.map(|item| format!("<li>{item}</li>")).collect();
    format!("<ul id=\"{id}\">{items}</ul>")
}

/// The items of the list `id` in `document`, in order.
fn items(document: &str, id: &str) -> Vec<String> {
    let open = format!("<ul id=\"{id}\">");
    let start = document
        .find(&open)
        .unwrap_or_else(|| panic!("the document shows #{id}:\n{document}"))
        + open.len();
    let end = document[start..].find("</ul>").expect("the list closes") + start;
    document[start..end]
        .split("<li>")
        .skip(1)
        .map(|item| item.trim_end_matches("</li>").to_string())
        .collect()
}

/// The last document `trace` shows.
fn last_document(trace: &PageTrace) -> &str {
    &trace
        .steps
        .last()
        .expect("the arrival and every drop")
        .snapshot
        .document
}

#[test]
fn every_name_only_drop_has_the_type_chrome_gives_it_on_every_tier() {
    let names: Vec<String> = DROPS.iter().map(|(name, _)| quoted(name)).collect();
    let source = format!(
        "{PAGE}\n{}",
        dropping(
            "a file dropped by its name alone has the type a browser gives that name",
            names.iter().map(String::as_str)
        )
    );
    let Some(all) = compared(&source) else { return };
    let [c] = all.as_slice() else {
        panic!("one scenario, not {}", all.len())
    };
    let chrome = traced(c, PageTierKind::Browser);
    let lowered = traced(c, PageTierKind::LoweredVm);
    let last = last_document(chrome);
    assert!(
        last.contains(&list("names", DROPS.iter().map(|(n, _)| n.to_string()))),
        "Chrome delivered every drop, in order:\n{last}"
    );
    assert!(
        last.contains(&list("types", DROPS.iter().map(|(_, t)| t.to_string()))),
        "Chrome is the oracle: a browser names a dropped file's type from its name, by the table \
         written in support/dropped_names.rs:\n{last}"
    );
    assert!(
        last.contains(&list("sizes", DROPS.iter().map(|_| "0".to_string()))),
        "a file known by its name alone has no content, so its size is 0:\n{last}"
    );
    for (tier, trace) in [
        ("the reference", &c.reference),
        ("the lowered VM", lowered),
        ("Chrome", chrome),
    ] {
        assert!(
            trace.stopped.is_none(),
            "{tier}: every drop is delivered: {:?}",
            trace.stopped
        );
    }
    for (tier, candidate) in [("the lowered VM", lowered), ("Chrome", chrome)] {
        let off = disagreements(&c.reference, candidate, &[Axis::Document, Axis::Journal]);
        assert!(
            off.is_empty(),
            "{tier} must show what the reference shows:\n{off}"
        );
    }
}

/// An extension the browser's primary list does not name is the desktop's to name first. The
/// tiers that synthesize the file answer the table on every machine and agree with each other;
/// Chrome on this machine answers its desktop's type where the desktop has one and the table's
/// where it does not, so its document parts from the reference's exactly where this desktop names
/// a file otherwise than the table — and the browser tier hands the page such a file without a
/// word, there being no declaration to refuse over.
#[test]
fn a_desktop_named_extension_is_the_tables_on_every_synthesizing_tier_and_the_desktops_in_chrome() {
    let names: Vec<String> = DESKTOP_DROPS.iter().map(|(name, _)| quoted(name)).collect();
    let source = format!(
        "{PAGE}\n{}",
        dropping(
            "an extension a desktop may name is named by the table in-process",
            names.iter().map(String::as_str)
        )
    );
    let Some(all) = compared(&source) else { return };
    let [c] = all.as_slice() else {
        panic!("one scenario, not {}", all.len())
    };
    let chrome = traced(c, PageTierKind::Browser);
    let lowered = traced(c, PageTierKind::LoweredVm);
    for (tier, trace) in [
        ("the reference", &c.reference),
        ("the lowered VM", lowered),
        ("Chrome", chrome),
    ] {
        assert!(
            trace.stopped.is_none(),
            "{tier}: every drop is delivered: {:?}",
            trace.stopped
        );
    }
    let table: Vec<String> = DESKTOP_DROPS.iter().map(|(_, t)| t.to_string()).collect();
    assert_eq!(
        items(last_document(&c.reference), "types"),
        table,
        "the reference tier types every desktop-named extension by the table"
    );
    let off = disagreements(&c.reference, lowered, &[Axis::Document, Axis::Journal]);
    assert!(
        off.is_empty(),
        "the lowered VM must show what the reference shows:\n{off}"
    );

    let chrome_types = items(last_document(chrome), "types");
    assert_eq!(
        chrome_types.len(),
        DESKTOP_DROPS.len(),
        "Chrome delivered every drop:\n{}",
        last_document(chrome)
    );
    let mut parted = Vec::new();
    for ((name, table), got) in DESKTOP_DROPS.iter().zip(&chrome_types) {
        let desktop = desktop_types(name);
        assert!(
            got.as_str() == *table || desktop.iter().any(|t| t == got),
            "{name:?}: Chrome names it {got:?}; the table says {table:?} and this desktop says {desktop:?} — a \
             browser answers one of these and nothing else"
        );
        if got.as_str() != *table {
            parted.push(*name);
        }
    }
    let off = disagreements(&c.reference, chrome, &[Axis::Document]);
    assert_eq!(
        off.is_empty(),
        parted.is_empty(),
        "Chrome's document parts from the reference's exactly where its desktop names a file otherwise than \
         the table ({parted:?}), and nowhere else:\n{off}"
    );
}

/// A type the author declares is the file's on the tiers that synthesize the file, whatever the
/// name says — a declared empty type included. The browser tier can hand the page only what a
/// browser names the file: a declared type Chrome gives the name runs there as everywhere, and one
/// it does not give is refused, the test told to declare the type a browser gives that name.
#[test]
fn a_declared_type_is_the_authors_in_process_and_refused_where_a_browser_names_the_file_otherwise()
{
    let source = format!(
        "{PAGE}\n{}\n{}",
        dropping(
            "a declared type a browser gives the name runs on every tier",
            [
                "a new File with name \"notes.txt\" and type \"text/plain\"",
                "a new File with name \"weird.qqq\" and type \"\"",
            ]
            .into_iter()
        ),
        dropping(
            "a declared type a browser does not give the name is refused by the browser tier",
            ["a new File with name \"notes.txt\" and type \"text/html\""].into_iter()
        ),
    );
    let Some(all) = compared(&source) else { return };
    let [agreed, refused] = all.as_slice() else {
        panic!("two scenarios, not {}", all.len())
    };

    assert_eq!(
        items(last_document(&agreed.reference), "types"),
        ["text/plain", ""]
    );
    for kind in [PageTierKind::LoweredVm, PageTierKind::Browser] {
        let trace = traced(agreed, kind);
        assert!(
            trace.stopped.is_none(),
            "{}: every drop is delivered: {:?}",
            kind.label(),
            trace.stopped
        );
        let off = disagreements(&agreed.reference, trace, &[Axis::Document, Axis::Journal]);
        assert!(
            off.is_empty(),
            "{} must show what the reference shows:\n{off}",
            kind.label()
        );
    }

    assert_eq!(
        items(last_document(&refused.reference), "types"),
        ["text/html"],
        "the reference tier hands the handler the declared type, whatever the name says"
    );
    let lowered = traced(refused, PageTierKind::LoweredVm);
    let off = disagreements(
        &refused.reference,
        lowered,
        &[Axis::Document, Axis::Journal],
    );
    assert!(
        off.is_empty(),
        "the lowered VM must show what the reference shows:\n{off}"
    );
    match run(refused, PageTierKind::Browser) {
        TierRun::Refused { reason, .. } => {
            for word in [
                "\"notes.txt\"",
                "\"text/plain\"",
                "\"text/html\"",
                "declare the type a browser gives that name",
            ] {
                assert!(reason.contains(word), "the refusal names the file, the browser's type, the declared type and the way out: {reason}");
            }
        }
        other => panic!(
            "the browser tier refuses a declared type it cannot hand the page, not {other:?}"
        ),
    }
}
