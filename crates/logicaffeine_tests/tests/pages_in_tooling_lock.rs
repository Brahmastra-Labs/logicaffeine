//! ★ LOCK — a project's PAGES are inside its tooling. ★
//!
//! `largo check`, `largo fmt` and `largo test` each build their file set from the manifest ENTRY
//! and `src/`. None of them has ever known that `pages/` exists — a grep for the word `pages`
//! across all three returns nothing.
//!
//! So on a `--web` project, where every line of frontend a person writes lives in `pages/`:
//!
//!   * `largo check` reports success on a site whose page does not compile,
//!   * `largo fmt --check` reports success on a page that is not formatted,
//!   * `largo test` discovers no test written in one.
//!
//! ★ THE FAILURE IS A GREEN CI ON A BROKEN SITE, which is worse than no check at all: whoever set
//! that CI up believes the frontend is covered, and the first thing that tells them otherwise is
//! a blank page in a browser. The tooling predates `--web` and nobody re-audited its file sets —
//! the same shape as every "built and not connected" defect in this campaign.
//!
//! ★ AND THE CHECK MUST NOT OVERSHOOT. A pristine scaffold has to stay silent. A tool that
//! accuses a correct project teaches people to stop running it, which costs more than the gap.

#[path = "cli_common/mod.rs"]
mod common;
use common::{largo_in, stderr};

/// A page that cannot compile: `<div>` is never closed, which the LGX template parser refuses by
/// name. Deliberately a mistake somebody actually makes — and deliberately NOT an unbound name,
/// which is a RUNTIME failure in LOGOS and compiles perfectly well (my first fixture was exactly
/// that, and it proved the check was working when it was not).
const BROKEN_PAGE: &str = "# Broken\n\n\
    ## Component App\n\
    State:\n\
    \x20   Let n be 0.\n\
    View:\n\
    \x20   <div><p>{n}</p>\n\n\
    ## Main\n\
    Mount App.\n";

/// Scaffold a `--web` project and hand back its root.
fn web_project(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let out = largo_in(dir, &["new", name, "--web"]);
    assert!(
        out.status.success(),
        "largo new --web failed: {}",
        stderr(&out)
    );
    dir.join(name)
}

/// ★ THE LOAD-BEARING TEST — `largo check` must not pass a site that does not build.
#[test]
fn largo_check_fails_when_a_page_does_not_compile() {
    let dir = tempfile::tempdir().unwrap();
    let root = web_project(dir.path(), "sitea");
    std::fs::write(root.join("pages/app.lg"), BROKEN_PAGE).unwrap();

    let out = largo_in(&root, &["check"]);
    assert!(
        !out.status.success(),
        "`largo check` reported SUCCESS on a project whose only page does not compile. Every line \
         of frontend in a --web project lives in pages/, so this is a green check on a broken \
         site — and any CI built on it is green too.\nstderr:\n{}",
        stderr(&out)
    );
    let msg = stderr(&out);
    assert!(
        msg.contains("app.lg") || msg.contains("pages"),
        "the failure must NAME the page it is about — a reader who is told only that something \
         failed will look in src/ first, which is the one place that is fine: {msg}"
    );
}

/// The pristine scaffold must stay silent: a tool that accuses a correct project gets switched off.
#[test]
fn largo_check_passes_on_the_untouched_web_scaffold() {
    let dir = tempfile::tempdir().unwrap();
    let root = web_project(dir.path(), "siteb");
    let out = largo_in(&root, &["check"]);
    assert!(
        out.status.success(),
        "`largo check` failed on an untouched `largo new --web` scaffold. Whatever this reports, \
         it is not the user's mistake.\nstderr:\n{}",
        stderr(&out)
    );
}

/// A page is source too — `largo fmt` must reach it, or a formatted repo silently is not one.
#[test]
fn largo_fmt_reaches_a_page() {
    let dir = tempfile::tempdir().unwrap();
    let root = web_project(dir.path(), "sitec");
    let page = root.join("pages/app.lg");
    let original = std::fs::read_to_string(&page).unwrap();

    // Trailing whitespace on a line the formatter owns: unambiguous, and nothing about LGX makes
    // it meaningful, so a formatter that reaches the file will remove it.
    std::fs::write(&page, original.replace("## Main", "## Main   ")).unwrap();

    let out = largo_in(&root, &["fmt"]);
    assert!(out.status.success(), "largo fmt failed: {}", stderr(&out));
    let after = std::fs::read_to_string(&page).unwrap();
    assert!(
        !after.contains("## Main   "),
        "`largo fmt` left `pages/app.lg` untouched. It collects `src/` and the manifest entry and \
         has never known pages/ exists, so `largo fmt --check` is green on an unformatted frontend."
    );
}

/// ★ THE DEV OVERLAY POINTS AT THE FILE THE MISTAKE IS IN.
///
/// The overlay draws its caret against the source it is handed, and that was the ASSEMBLED
/// closure — every `Import` spliced in ahead of the page's own text. So a page with
/// `Import Shared.` reported a line number offset by the whole of the imported module and drew
/// the caret under the wrong line: a confident pointer at innocent code, which is worse than no
/// pointer. Imports only began working recently, so this went from harmless to hitting every
/// real project at once.
///
/// Asserted through the REAL dev server on a REAL project, because the bug is entirely in which
/// source the overlay is given — a unit test on the formatter would pass either way.
#[test]
fn the_build_overlay_names_the_page_and_its_own_line() {
    let dir = tempfile::tempdir().unwrap();
    let root = web_project(dir.path(), "sited");
    // A module the page imports, long enough that a splice-offset line number cannot coincide.
    std::fs::write(
        root.join("src/shared.lg"),
        "# Shared\n\n## A Thing has:\n    an id, which is Int.\n\n\
         ## To one () -> Int:\n    Return 1.\n\n## To two () -> Int:\n    Return 2.\n\n\
         ## To three () -> Int:\n    Return 3.\n\n## To four () -> Int:\n    Return 4.\n",
    )
    .unwrap();
    // The page's own mistake is on ITS line 6 — an unclosed `<div>`.
    let page = "# App\n\nImport Shared.\n\n## Component App\nView:\n    <div><p>hi</p>\n\n## Main\nMount App.\n";
    std::fs::write(root.join("pages/app.lg"), page).unwrap();

    let out = largo_in(&root, &["check"]);
    assert!(
        !out.status.success(),
        "the page does not compile, so check must fail"
    );
    let msg = stderr(&out);
    assert!(
        msg.contains("app.lg"),
        "the failure must name the PAGE, not the module spliced in ahead of it — a reader told \
         only a line number opens whichever file they had last: {msg}"
    );
}

/// ★ THE DEV SERVER CAN SERVE THE NETWORK, AND SAYS SO BY DEFAULT IT DOES NOT.
///
/// The bind was hard-coded to `127.0.0.1`, so testing a page on a phone — the device half of
/// your visitors use — meant not testing it. Binding this machine only is the right DEFAULT (a
/// dev server should not appear on the network because somebody ran it); having no way to opt in
/// is not.
#[test]
fn serve_offers_a_host_flag() {
    let dir = tempfile::tempdir().unwrap();
    let out = largo_in(dir.path(), &["serve", "--help"]);
    let help = format!("{}{}", String::from_utf8_lossy(&out.stdout), stderr(&out));
    assert!(
        help.contains("--host"),
        "`largo serve` must offer a bind address, or a page can only ever be opened on the \
         machine that built it:\n{help}"
    );
    assert!(
        help.contains("0.0.0.0") || help.to_lowercase().contains("network"),
        "and the help must say what to pass — `--host` with no hint is a flag nobody uses:\n{help}"
    );
}

/// ★ THE ACCESSIBILITY WARNINGS REACH THE DEVELOPER.
///
/// `lgx/check.rs` computes them on every parse — `img` without `alt`, a button that announces
/// nothing, an input with no label — and `Parser::lgx_findings()` had exactly ONE caller, a test.
/// Four well-written warnings with spans, computed and discarded, for two phases.
///
/// They are WARNINGS, not errors: a page that is hard to use is not a page that is broken, and a
/// check that failed on them would be switched off.
#[test]
fn largo_check_reports_accessibility_warnings_without_failing() {
    let dir = tempfile::tempdir().unwrap();
    let root = web_project(dir.path(), "sitee");
    std::fs::write(
        root.join("pages/app.lg"),
        "# App\n\n## Component App\nState:\n    Let n be 0.\nView:\n    \
         <img src=\"cat.png\">\n\n## Main\nMount App.\n",
    )
    .unwrap();

    let out = largo_in(&root, &["check"]);
    let msg = stderr(&out);
    assert!(
        msg.contains("alt"),
        "an `<img>` with no `alt` must be reported — the checker has always found it and nobody \
         has ever been told:\n{msg}"
    );
    assert!(
        out.status.success(),
        "and it must stay a WARNING. A check that fails on accessibility advice is a check people \
         turn off:\n{msg}"
    );
}
