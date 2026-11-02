//! ★ SMOKE — a built LOGOS site in a real browser. ★
//!
//! Every other lock in this campaign checks a tier against another tier. This one checks the
//! thing itself: a page built by `largo`, served over loopback HTTP the way it ships, painted by a
//! real WebAssembly module through the real host runtime in headless Chrome, and clicked.
//!
//! TWO HALVES. The first needs NO browser and runs everywhere: a built page must already carry its
//! content, because that is what a reader with a slow connection, a crawler, or scripts disabled
//! receives. The second needs Chrome and is gated through the honest skip channel — a box without
//! it records a SKIP, and `LOGOS_REQUIRE_TOOLS=chrome` (the full runner's default, and CI's) makes
//! that skip a LOUD FAILURE.

#[path = "cli_common/mod.rs"]
mod common;

use common::{largo_in, stderr};
use logicaffeine_cli::browser::{self, Browser, PageBundle};

/// Build the scaffolded site and hand back its front page's HTML.
fn built_site(dir: &std::path::Path) -> (std::path::PathBuf, String) {
    let out = largo_in(dir, &["new", "site", "--web"]);
    assert!(
        out.status.success(),
        "scaffold must succeed: {}",
        stderr(&out)
    );
    let root = dir.join("site");
    let page = std::fs::read_to_string(root.join("pages/app.lg")).unwrap();
    (root, logicaffeine_cli::prerender_page_html(&page, "app"))
}

// ---- The half that needs no browser. ----

#[test]
fn a_built_page_is_readable_before_any_script_runs() {
    // What a crawler, a slow connection, and a reader with scripts off actually get.
    let dir = tempfile::tempdir().unwrap();
    let (_, html) = built_site(dir.path());
    assert!(
        html.contains("is alive"),
        "the heading is in the document:\n{html}"
    );
    assert!(
        html.contains("Clicks so far: 0"),
        "and the initial state:\n{html}"
    );
}

#[test]
fn the_built_page_asks_for_exactly_the_module_it_ships() {
    // A page referencing a script that is not built is a blank page in production and a green
    // test suite — the two together are the worst possible combination.
    let dir = tempfile::tempdir().unwrap();
    let (_, html) = built_site(dir.path());
    assert!(
        html.contains("./app.js"),
        "it loads its own module:\n{html}"
    );
    assert!(
        !html.contains(".mjs"),
        "and asks for nothing that is not shipped — a page once linked a node shim no browser \
         could use:\n{html}"
    );
}

#[test]
fn the_pages_wasm_module_exists_and_is_a_wasm_module() {
    let dir = tempfile::tempdir().unwrap();
    let root = {
        let out = largo_in(dir.path(), &["new", "site", "--web"]);
        assert!(
            out.status.success(),
            "scaffold must succeed: {}",
            stderr(&out)
        );
        dir.path().join("site")
    };
    let page = std::fs::read_to_string(root.join("pages/app.lg")).unwrap();
    let wasm = logicaffeine_compile::compile::compile_to_wasm(&page)
        .expect("the front page must compile to wasm");
    assert_eq!(
        &wasm[..4],
        b"\0asm",
        "the served module must carry the WebAssembly magic — a browser refuses anything else"
    );
}

// ---- The half that needs Chrome. ----

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

/// Serve the scaffolded site's front page from loopback HTTP, load it in headless Chrome, click
/// its button, and read the count back.
///
/// The bundle is built by the functions `largo build` calls — the pre-rendered document, the
/// page's module and the host runtime — so a second implementation of the build cannot pass
/// while the real one is broken. Chrome refuses a module script from `file://`, so the page is
/// served over HTTP exactly as it ships.
#[test]
fn a_real_browser_paints_the_page_and_its_button_counts() {
    let Some(b) = browser() else { return };
    let dir = tempfile::tempdir().unwrap();
    let (root, _) = built_site(dir.path());
    let page = std::fs::read_to_string(root.join("pages/app.lg")).unwrap();
    let bundle = PageBundle::prerendered(&page).unwrap_or_else(|e| panic!("{e}"));
    let p = b
        .open(&bundle)
        .unwrap_or_else(|e| panic!("the built page must load: {e}"));

    let painted = p.snapshot().unwrap().body_markup();
    assert!(
        painted.contains("is alive"),
        "a real browser must paint the page; got:\n{painted}"
    );
    assert_eq!(
        painted.matches("is alive").count(),
        1,
        "and paint it ONCE — the module adopts the pre-rendered markup:\n{painted}"
    );

    // `is alive` is in the PRE-RENDERED markup, so on its own it passes whether or not the module
    // ever ran. The one thing only a running module can produce is a page that has CHANGED.
    p.click("#more").unwrap();
    let after = p.snapshot().unwrap().body_markup();
    assert!(
        after.contains("Clicks so far: 1"),
        "the button must count in a real browser — `Clicks so far: 0` means the document was \
         painted from the pre-rendered markup and the module never took over:\n{after}"
    );
    assert!(
        p.exceptions().is_empty(),
        "the page must not throw: {:#?}",
        p.exceptions()
    );
}

// ---------------------------------------------------------------------------------------------
// ★ THE GATE ITSELF — a smoke test that can go dark is a smoke test that stops being run.
// ---------------------------------------------------------------------------------------------

/// Everything the smoke needs in order to actually RUN, held against the files that provide it.
///
/// This is the same lesson `wasm_gated_suite_lock` records: the browser half of this file skips
/// without a browser, which is right on a developer's machine and WRONG in CI — a gate that can
/// quietly skip is a gate nobody notices losing. So three things have to stay true, and none of
/// them is visible from inside a test that skipped:
///
///   1. CI runs the script at all.
///   2. The script makes a missing browser a FAILURE there (`LOGOS_REQUIRE_TOOLS`), not a skip.
///   3. The script drives THIS test rather than assembling a site of its own — a second
///      implementation of the build would be free to drift from it and to pass while the real
///      build was broken.
#[test]
fn the_browser_smoke_is_wired_into_ci_and_cannot_skip_there() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let script_path = root.join("scripts/web/test-web-smoke.sh");
    let script =
        std::fs::read_to_string(&script_path).expect("scripts/web/test-web-smoke.sh exists");
    let workflow = std::fs::read_to_string(root.join(".github/workflows/test.yml"))
        .expect(".github/workflows/test.yml exists");

    assert!(
        workflow.contains("scripts/web/test-web-smoke.sh"),
        "CI must run the browser smoke, or the only check that loads a real page in a real \
         browser runs nowhere"
    );
    assert!(
        script.contains("LOGOS_REQUIRE_TOOLS=chrome"),
        "the script must make a missing browser a LOUD failure — a smoke that can silently skip \
         in CI has stopped being a smoke test:\n{script}"
    );
    assert!(
        script.contains("--test browser_smoke"),
        "the script must drive THIS test rather than assemble a site of its own — a second \
         implementation of the build passes while the real build is broken:\n{script}"
    );
    // And it must be executable, or CI's `run:` fails in a way that reads like a missing file.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&script_path)
            .expect("stat")
            .permissions()
            .mode();
        assert!(
            mode & 0o111 != 0,
            "the script is executable (mode {mode:o})"
        );
    }
}

#[test]
fn each_browser_smoke_runs_in_the_workflow_that_can_afford_it() {
    // They test different things and BOTH matter: the SSG smoke drives the marketing site's
    // prerender + CSR takeover, this one drives what `largo new --web` produces. They run in
    // DIFFERENT workflows, and that is deliberate rather than an oversight — the SSG smoke needs a
    // full release build of the web app to have something to serve, which belongs with the deploy
    // that builds it. Asserting both in `test.yml` would have been asserting the wrong thing, and
    // "wire it in" would have put a multi-minute build in the test job to no benefit.
    //
    // What must hold is that NEITHER runs nowhere, which is the failure a check quietly stops
    // being run by.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let read = |rel: &str| std::fs::read_to_string(root.join(rel)).unwrap_or_default();
    let test_wf = read(".github/workflows/test.yml");
    let deploy_wf = read(".github/workflows/deploy-frontend.yml");

    assert!(
        test_wf.contains("scripts/web/test-web-smoke.sh"),
        "the PAGE smoke runs in the test workflow — it only needs `largo build`"
    );
    assert!(
        deploy_wf.contains("scripts/web/test-ssg-smoke.sh"),
        "the SSG smoke runs where the site it drives is built"
    );
    for script in [
        "scripts/web/test-web-smoke.sh",
        "scripts/web/test-ssg-smoke.sh",
    ] {
        assert!(
            root.join(script).exists(),
            "`{script}` exists — a workflow step naming a missing script fails obscurely"
        );
    }
}
