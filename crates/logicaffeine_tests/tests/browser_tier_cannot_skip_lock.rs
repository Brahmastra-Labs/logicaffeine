//! ★ LOCK — the browser page tier cannot go dark in the full test runner.
//!
//! A browser test skips on a machine without Chrome, which is right on a laptop and wrong on the
//! box the baseline runs on: a tier that can quietly skip is a tier nobody notices losing. Four
//! things have to hold together, and none is visible from inside a test that skipped:
//!
//!   1. `scripts/run-all-tests-fast.sh` requires `chrome` by default (`LOGOS_REQUIRE_TOOLS`), in the
//!      list syntax the honest seam parses, beside the tools it already required.
//!   2. Every `browser_*` test binary that drives Chrome finds it through
//!      `browser::locate_chrome` and routes its absence through `common::require_tool_strict`,
//!      which panics under that requirement — never a bare `return`, never a PATH probe for a
//!      binary called `chrome` that most installs do not have.
//!   3. The locator finds a browser wherever one of its usual names is installed.
//!   4. No exclusion filter in the runner names a browser binary.

#[path = "cli_common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn runner() -> String {
    std::fs::read_to_string(repo().join("scripts/run-all-tests-fast.sh"))
        .expect("scripts/run-all-tests-fast.sh exists")
}

/// The default the runner gives `LOGOS_REQUIRE_TOOLS` when the caller set none: the text between
/// `${LOGOS_REQUIRE_TOOLS:-` and the closing `}` of the one `export` that sets it.
fn runner_default_requirement(script: &str) -> String {
    let exports: Vec<&str> = script
        .lines()
        .filter(|l| l.trim_start().starts_with("export LOGOS_REQUIRE_TOOLS="))
        .collect();
    assert_eq!(
        exports.len(),
        1,
        "the runner sets LOGOS_REQUIRE_TOOLS in exactly one place — two would make the effective \
         requirement depend on which ran last: {exports:#?}"
    );
    let line = exports[0];
    let start = line.find("${LOGOS_REQUIRE_TOOLS:-").unwrap_or_else(|| {
        panic!("the export keeps a caller's own value and supplies a default: {line}")
    }) + "${LOGOS_REQUIRE_TOOLS:-".len();
    let end = line[start..]
        .find('}')
        .expect("the default expansion closes")
        + start;
    line[start..end].to_string()
}

#[test]
fn the_full_runner_requires_chrome_by_default() {
    let default = runner_default_requirement(&runner());
    assert!(
        common::tool_required_by(&default, "chrome"),
        "the full runner must make a missing Chrome a FAILURE — its LOGOS_REQUIRE_TOOLS default is \
         {default:?}, which the seam does not read as requiring `chrome`"
    );
    assert!(
        common::tool_required_by(&default, "lean"),
        "and the tools it already required stay required: {default:?}"
    );
    assert!(
        !common::tool_required_by(&default, "a-tool-nobody-installs"),
        "the default names tools; it is not `1`/`all`, which would require every optional tool: {default:?}"
    );
}

/// Every `browser_*.rs` under `tests/`, as `(file name, source)`.
fn browser_test_sources() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut out: Vec<(String, String)> = std::fs::read_dir(&dir)
        .expect("tests/ is readable")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "rs")
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("browser_"))
        })
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            (name, std::fs::read_to_string(&p).unwrap())
        })
        .collect();
    out.sort();
    out
}

#[test]
fn every_browser_test_binary_gates_through_the_seam_the_requirement_reads() {
    let sources = browser_test_sources();
    // This file quotes the patterns it hunts, so it is not one of the binaries it audits.
    let this_file = file!().rsplit(['/', '\\']).next().unwrap_or_default();
    let drivers: Vec<&(String, String)> = sources
        .iter()
        .filter(|(name, _)| name != this_file)
        .filter(|(_, src)| src.contains("browser::shared(") || src.contains("Browser::launch("))
        .collect();
    assert!(
        drivers.len() >= 2,
        "the census must see the browser test binaries (browser_smoke, browser_tier_lock); it found {:?}",
        drivers.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
    for (name, src) in drivers {
        assert!(
            src.contains("browser::locate_chrome()"),
            "`{name}` drives Chrome and must find it with `browser::locate_chrome()` — the one \
             locator that knows `CHROME_BIN`, `google-chrome` and `chromium`"
        );
        assert!(
            src.contains("common::require_tool_strict(\"chrome\", &spec)"),
            "`{name}` must route a missing browser through `common::require_tool_strict(\"chrome\", \
             &spec)`, the seam that panics when LOGOS_REQUIRE_TOOLS names chrome"
        );
        assert!(
            !src.contains("require_tool(\"chrome\")"),
            "`{name}` probes PATH for a binary named `chrome`, which a google-chrome or chromium \
             install does not provide — the probe fails and the test skips with Chrome installed"
        );
    }
}

#[test]
fn the_locator_finds_an_installed_browser() {
    const NAMES: &[&str] = &[
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
    ];
    let on_path: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .flat_map(|dir| NAMES.iter().map(move |n| dir.join(n)))
        .filter(|p| p.is_file())
        .collect();
    let found = logicaffeine_cli::browser::locate_chrome();
    if let Some(first) = on_path.first() {
        assert!(
            found.is_some(),
            "{} is installed and the locator does not find it — the browser tier would skip on a \
             machine that has a browser",
            first.display()
        );
    }
    if let Some(path) = &found {
        assert!(
            path.is_file(),
            "the locator names an executable file: {}",
            path.display()
        );
    }
}

#[test]
fn no_runner_exclusion_names_a_browser_binary() {
    let script = runner();
    let exclusions: Vec<&str> = script
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.starts_with('#')
                && t.split('=')
                    .next()
                    .is_some_and(|name| name.ends_with("_EXCLUDE"))
        })
        .collect();
    assert!(
        !exclusions.is_empty(),
        "the census must see the runner's exclusion filters"
    );
    for line in exclusions {
        assert!(
            !line.contains("browser"),
            "a runner exclusion names a browser binary, which drops the browser tier from every \
             baseline without a failure anywhere: {line}"
        );
    }
    let nextest = std::fs::read_to_string(repo().join(".config/nextest.toml")).unwrap_or_default();
    for line in nextest.lines().filter(|l| l.contains("default-filter")) {
        assert!(
            !line.contains("browser"),
            "a nextest default filter names a browser binary: {line}"
        );
    }
}
