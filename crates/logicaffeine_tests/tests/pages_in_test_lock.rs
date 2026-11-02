//! ★ LOCK — `largo test` runs the tests written in a project's PAGES. ★
//!
//! `pages_in_tooling_lock` names three tools that had never heard of `pages/`, and closed
//! two of them: `largo check` now compiles every page and `largo fmt` now formats them.
//! **The third was left open**, and its own header says so — `largo test` builds its file
//! set from the manifest entry and `src/`, so a `## Test` written in a page is discovered
//! by nothing.
//!
//! ★ THE FAILURE IS THE SAME ONE, AND IT IS THE WORST OF THE THREE. A component test is
//! the only kind that mounts a page, drives it and asserts about a rendered document —
//! and on a `--web` project the page is where every one of them naturally lives. So the
//! answer to "does this project test its UI?" was: it does, and the runner never looked.
//! A green CI on an untested frontend.
//!
//! ★ AND THE ZERO-TEST GATE MADE IT LOOK DELIBERATE. A project whose only tests are in
//! its pages failed with "no tests were discovered — no `## Test` block exists in the
//! project's closure", which is a confident, specific and false statement about a project
//! holding several.

#[path = "cli_common/mod.rs"]
mod common;
use common::{largo_in, stderr, stdout};

use std::path::{Path, PathBuf};

/// A page carrying a component test: it drives the page and asserts about the rendered
/// document — the kind of test that can only be written where the page is.
///
/// No `Mount` sentence: a `## Page` mounts ITSELF (that is the framework's rule — a page
/// owns a URL and needs no entry point), so the test drives what is already there.
const TESTED_PAGE: &str = r##"# Counter

## Page /

State:
    Let clicks be 0.

Actions:
    To bump:
        Set clicks to clicks + 1.

View:
    <main>
      <button id="more" on-click={bump}>Click me</button>
      <p id="count">{clicks}</p>
    </main>

## Test: the button counts

Click "#more".
Expect that the text of "#count" is equal to "1".
"##;

/// A page whose test FAILS — the run must go red, or discovery without enforcement is
/// worse than no discovery at all.
const FAILING_PAGE: &str = r##"# Counter

## Page /

State:
    Let clicks be 0.

View:
    <main>
      <p id="count">{clicks}</p>
    </main>

## Test: the count starts at nine

Expect that the text of "#count" is equal to "9".
"##;

fn web_project(dir: &Path, name: &str) -> PathBuf {
    let out = largo_in(dir, &["new", name, "--web"]);
    assert!(out.status.success(), "scaffold: {}", stderr(&out));
    dir.join(name)
}

/// ★ THE LOAD-BEARING TEST — a test written in a page runs.
#[test]
fn a_test_written_in_a_page_is_discovered_and_run() {
    let dir = tempfile::tempdir().unwrap();
    let root = web_project(dir.path(), "tested");
    std::fs::write(root.join("pages/app.lg"), TESTED_PAGE).unwrap();

    let out = largo_in(&root, &["test"]);
    let all = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        all.contains("the button counts"),
        "the page's own test must appear in the run — every line of frontend in a `--web` \
         project lives in pages/, so a runner that cannot see them answers \"does this project \
         test its UI?\" with a green run and no evidence.\n{all}"
    );
    assert!(out.status.success(), "and it must pass:\n{all}");
}

/// ★ DISCOVERY WITHOUT ENFORCEMENT IS WORSE THAN NO DISCOVERY. A listed test that cannot
/// fail the run is a test somebody will trust.
#[test]
fn a_failing_test_in_a_page_fails_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let root = web_project(dir.path(), "broken");
    std::fs::write(root.join("pages/app.lg"), FAILING_PAGE).unwrap();

    let out = largo_in(&root, &["test"]);
    assert!(
        !out.status.success(),
        "a page whose test fails must fail `largo test`:\n{}{}",
        stdout(&out),
        stderr(&out)
    );
}

/// The entry's own tests still run beside the pages' — this widens the file set, it does
/// not replace it.
#[test]
fn the_entrys_own_tests_still_run() {
    let dir = tempfile::tempdir().unwrap();
    let root = web_project(dir.path(), "both");
    std::fs::write(root.join("pages/app.lg"), TESTED_PAGE).unwrap();
    let main = std::fs::read_to_string(root.join("src/main.lg")).unwrap();
    std::fs::write(
        root.join("src/main.lg"),
        format!("{main}\n## Test: the backend answers\n\nExpect that 1 + 1 is equal to 2.\n"),
    )
    .unwrap();

    let out = largo_in(&root, &["test"]);
    let all = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        all.contains("the backend answers"),
        "the entry's test:\n{all}"
    );
    assert!(all.contains("the button counts"), "and the page's:\n{all}");
    assert!(out.status.success(), "both pass:\n{all}");
}

/// A page with no `## Test`: it compiles, and there is nothing in it to run.
const UNTESTED_PAGE: &str = r##"# Counter

## Page /

State:
    Let clicks be 0.

View:
    <main>
      <p id="count">{clicks}</p>
    </main>
"##;

/// ★ AND A PROJECT WITH NO TESTS MUST STAY QUIET ABOUT ITS PAGES. A tool that accuses a
/// correct project teaches people to stop running it. With no `## Test` anywhere — in the entry
/// or in any page — the honest answer is the zero-test failure, about the whole project, not
/// about `pages/`.
#[test]
fn a_scaffold_with_no_tests_anywhere_still_says_so_plainly() {
    let dir = tempfile::tempdir().unwrap();
    let root = web_project(dir.path(), "pristine");
    std::fs::write(root.join("pages/app.lg"), UNTESTED_PAGE).unwrap();
    let out = largo_in(&root, &["test"]);
    let all = format!("{}{}", stdout(&out), stderr(&out));
    assert!(!out.status.success(), "no tests is not a green run:\n{all}");
    assert!(
        all.contains("no tests were discovered"),
        "and it says the plain thing, with no new noise about pages:\n{all}"
    );
}
