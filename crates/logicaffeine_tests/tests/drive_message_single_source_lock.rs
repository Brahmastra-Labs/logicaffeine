//! ★ LOCK — what a driving verb's failure SAYS lives in one place: `semantics::drive`. ★
//!
//! A test clicks an element that is not there, or one nothing listens to, and the sentence it
//! reads decides how fast its author finds the mistake. Every tier reports those outcomes — the
//! reference tier from its own runtime, the bytecode compiler from a lowered driver's 0/1/2 answer,
//! the page host from the same answer — and each used to carry its own copy of the words. Copies
//! drift: the compiler said "a input" where the reference said "a input or change", and "a keydown"
//! where it said "a Enter press".
//!
//! Two halves:
//! - BEHAVIOUR: for every verb, the reference tier's own failure text equals what
//!   `semantics::drive` builds, so moving a tier onto the single source changes no word.
//! - CENSUS: the SOURCE of every crate is read, and each message template is found in
//!   `semantics/drive.rs` and nowhere else — except the copies listed in `COPIES`, which only
//!   shrinks: a listed copy that is gone fails too, so the list cannot outlive its reason.

#[path = "support/rust_literals.rs"]
mod rust_literals;

use logicaffeine_compile::semantics::drive::{
    cannot_take, element_required, mount_target_missing, no_element, no_routes_to_serve,
    nothing_answers, Verb,
};
use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};
use std::path::{Path, PathBuf};

/// The single failure a one-test source reports on the reference tier.
fn reference_failure(src: &str) -> String {
    let report = run_tests(src, &TestConfig::default())
        .unwrap_or_else(|e| panic!("the program must parse:\n{src}\n=> {e}"));
    assert_eq!(report.outcomes.len(), 1, "{src}");
    let o = &report.outcomes[0];
    assert_eq!(
        o.verdict,
        Verdict::Failed,
        "the statement must fail:\n{src}"
    );
    assert_eq!(o.failures.len(), 1, "one failure: {:?}", o.failures);
    o.failures[0].clone()
}

const PAGE: &str = "## Component App\n\
    View:\n    <p id=\"m\">hi</p><form id=\"f\"><input id=\"d\"></form><select id=\"s\"><option>a</option></select>\n\n";

fn test_with(stmt: &str) -> String {
    format!("{PAGE}## Test: t\n    Mount App.\n    {stmt}\n")
}

#[test]
fn every_verb_aimed_at_nothing_says_so_in_the_single_sources_words() {
    for (verb, stmt) in [
        (Verb::Click, "Click \"#none\"."),
        (Verb::Submit, "Submit \"#none\"."),
        (Verb::Press, "Press \"Enter\" in \"#none\"."),
        (Verb::Type, "Type \"x\" into \"#none\"."),
        (Verb::Choose, "Choose \"a\" in \"#none\"."),
        (Verb::Hover, "Hover \"#none\"."),
    ] {
        assert_eq!(
            reference_failure(&test_with(stmt)),
            no_element("#none"),
            "{verb:?}"
        );
    }
}

#[test]
fn every_verb_nothing_answers_says_so_in_the_single_sources_words() {
    // Typing into a text field and choosing an option the `<select>` offers are answered by the
    // browser's own default action whether or not the page listens; their failure is a control that
    // cannot take the value (below).
    for (verb, stmt, selector, key) in [
        (Verb::Click, "Click \"#m\".", "#m", None),
        (Verb::Submit, "Submit \"#m\".", "#m", None),
        (
            Verb::Press,
            "Press \"Enter\" in \"#d\".",
            "#d",
            Some("Enter"),
        ),
        (Verb::Hover, "Hover \"#m\".", "#m", None),
    ] {
        assert_eq!(
            reference_failure(&test_with(stmt)),
            nothing_answers(verb.events(), key, selector),
            "{verb:?}"
        );
    }
}

#[test]
fn a_value_no_control_takes_is_refused_in_the_single_sources_words() {
    for (verb, stmt, selector, value) in [
        (Verb::Type, "Type \"x\" into \"#m\".", "#m", "x"),
        (Verb::Choose, "Choose \"z\" in \"#s\".", "#s", "z"),
    ] {
        assert_eq!(
            reference_failure(&test_with(stmt)),
            cannot_take(verb, selector, value),
            "{verb:?}"
        );
    }
}

#[test]
fn the_other_driving_refusals_are_the_single_sources_words() {
    assert_eq!(
        reference_failure(&test_with("Mount App at \"#none\".")),
        mount_target_missing("App", "#none")
    );
    assert_eq!(
        reference_failure(&test_with("Focus \"#none\".")),
        element_required("Focus", "#none")
    );
    assert_eq!(
        reference_failure(&test_with("Blur \"#none\".")),
        element_required("Blur", "#none")
    );
    assert_eq!(
        reference_failure(&test_with("Serve fetches with routes.")),
        no_routes_to_serve()
    );
}

// ---- The census. ----

/// A message template, by a fragment that appears in its source spelling and nowhere else.
struct Template {
    name: &'static str,
    holds: fn(&str) -> bool,
}

const TEMPLATES: &[Template] = &[
    Template {
        name: "nothing_answers",
        holds: |l| l.contains("Give the element an `on-"),
    },
    Template {
        name: "no_element",
        holds: |l| {
            // `No element matches {…}. Mount the component first` — and NOT a page query's
            // `No element matches {…}, so it has no …`, which is a different sentence.
            let mut rest = l;
            while let Some(at) = rest.find("No element matches {") {
                let after = &rest[at + "No element matches {".len()..];
                if let Some(close) = after.find('}') {
                    if after[close + 1..].starts_with(". Mount the component first") {
                        return true;
                    }
                }
                rest = after;
            }
            false
        },
    },
    Template {
        name: "mount_target_missing",
        holds: |l| l.contains("no element matches that selector. Create"),
    },
    Template {
        name: "element_required",
        holds: |l| l.contains("found no such element. Mount the component first"),
    },
    Template {
        name: "no_routes_to_serve",
        holds: |l| l.contains("needs routes to serve, and this program"),
    },
];

/// The single source.
const SOURCE: &str = "crates/logicaffeine_compile/src/semantics/drive.rs";

/// Copies still to be retired, as `(file, template)`. Shrink-only: each is a request to
/// the lane that owns the file.
const COPIES: &[(&str, &str)] = &[];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if !matches!(
                p.file_name().and_then(|n| n.to_str()),
                Some("target" | "node_modules" | ".git")
            ) {
                sources(&p, out);
            }
        } else if matches!(p.extension().and_then(|x| x.to_str()), Some("rs" | "js")) {
            out.push(p);
        }
    }
}

/// Every `(file, template)` a string literal in CODE holds — comments quote messages, and a quote
/// is not a copy. Rust sources are read literal by literal (a template split over a `\`
/// continuation is one literal); JavaScript, line by line.
fn holders() -> std::collections::BTreeSet<(String, &'static str)> {
    let root = workspace();
    let mut files = Vec::new();
    for top in ["crates", "apps"] {
        let Ok(members) = std::fs::read_dir(root.join(top)) else {
            continue;
        };
        for m in members.flatten() {
            sources(&m.path().join("src"), &mut files);
            sources(&m.path().join("assets"), &mut files);
        }
    }
    let mut found = std::collections::BTreeSet::new();
    for f in files {
        let Some(text) = rust_literals::try_read_stable(&f) else {
            continue;
        };
        let rel = f
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let pieces: Vec<String> = if f.extension().is_some_and(|x| x == "rs") {
            rust_literals::literals(&text)
                .into_iter()
                .map(|l| l.value)
                .collect()
        } else {
            text.lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .map(str::to_string)
                .collect()
        };
        for piece in &pieces {
            for t in TEMPLATES {
                if (t.holds)(piece) {
                    found.insert((rel.clone(), t.name));
                }
            }
        }
    }
    found
}

#[test]
fn every_template_lives_in_the_single_source_and_nowhere_it_is_not_listed() {
    let found = holders();
    for t in TEMPLATES {
        assert!(
            found.contains(&(SOURCE.to_string(), t.name)),
            "`{}` must be spelled in {SOURCE} — the census has lost its anchor",
            t.name
        );
    }
    let listed: std::collections::BTreeSet<(String, &str)> =
        COPIES.iter().map(|(f, t)| (f.to_string(), *t)).collect();
    let unlisted: Vec<_> = found
        .iter()
        .filter(|(f, _)| f != SOURCE)
        .filter(|h| !listed.contains(*h))
        .collect();
    assert!(
        unlisted.is_empty(),
        "a driving failure is worded outside `semantics::drive` — call the single source instead:\n{unlisted:#?}"
    );
    let retired: Vec<_> = listed.iter().filter(|h| !found.contains(*h)).collect();
    assert!(
        retired.is_empty(),
        "these copies are gone — delete them from COPIES (the list only shrinks):\n{retired:#?}"
    );
}

#[test]
fn the_census_sees_a_copy_when_one_is_written() {
    // The detector, on literals shaped like the real ones — including a template split over a
    // line continuation, which is one literal to rustc and two lines to a text search.
    let src = r#"
        fn a() { let _ = format!("No element matches {named}. Mount the component first, and check"); }
        fn b() { let _ = format!("No element matches {sel:?}, so it has no {q}. Mount the component first"); }
        fn c() { let _ = format!("Nothing answers {what} on {s:?}. Give the element an \
                                  `on-{}={{action}}` directive"); }
        // "Mount {name} at {named}: no element matches that selector. Create the mount point"
    "#;
    let by = |name: &str| TEMPLATES.iter().find(|t| t.name == name).unwrap().holds;
    let lits: Vec<String> = rust_literals::literals(src)
        .into_iter()
        .map(|l| l.value)
        .collect();
    assert_eq!(lits.len(), 3, "the comment is not a literal: {lits:?}");
    assert!((by("no_element"))(&lits[0]));
    assert!(
        !(by("no_element"))(&lits[1]),
        "a page query's sentence is a different template"
    );
    assert!(
        (by("nothing_answers"))(&lits[2]),
        "the continuation is folded: {:?}",
        lits[2]
    );
    assert!(
        !lits.iter().any(|l| (by("mount_target_missing"))(l)),
        "a comment quoting a template is not a copy"
    );
}
