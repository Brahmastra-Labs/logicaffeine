//! ★ LOCK — a Date/Moment calendar component runs under EVERY host that can run a standalone
//! wasm module, not just the `wasmi` test stub. ★
//!
//! `vm/wasm/module.rs` imports `temporal_component`/`temporal_component_date` the moment a
//! program reads `the weekday of`/`the year of`/… a `Date` or `Moment`. Before this lock, only a
//! test file's own hand-rolled `wasmi` linker answered them — `assets/web/runtime.js` (the
//! browser host) and `web_host::wasm_host_shim` (the `node` shim `largo build --emit wasm`
//! writes) had no implementation, so a page or a headless program reading a date component failed
//! to INSTANTIATE anywhere real: not in a browser, not under `node`.
//!
//! These tests run the ACTUAL, unmodified host bodies — `browser_host_js` and `wasm_host_shim` —
//! under real V8 (`node`), against a real compiled module, and hold the output to the tree-walker
//! reference. A host that answers the wrong weekday, or does not answer at all, fails here loudly.

mod common;
use common::require_tool;

use std::io::Write;
use std::process::Command;

use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome};
use logicaffeine_compile::web_host::{browser_host_js, wasm_host_shim};

/// A Date and a Moment, every component `the_year_of`'s family can extract, including the
/// weekday (ISO 8601: Monday = 1 … Sunday = 7) — 2024-03-11 is a Monday, so the census also pins
/// the convention, not only "does it answer at all".
const SRC: &str = concat!(
    "## Main\n",
    "Let d be 2024-03-11.\n",
    "Show the weekday of d.\n",
    "Show the year of d.\n",
    "Show the month of d.\n",
    "Show the day of d.\n",
    "Show the week of d.\n",
    "Show the quarter of d.\n",
    "Let m be parse_timestamp(\"2024-03-10T07:30:45Z\").\n",
    "Show the weekday of m.\n",
    "Show the hour of m.\n",
    "Show the minute of m.\n",
);

/// Globals sufficient for `runtime.js` to LOAD under `node` with no real DOM: this program never
/// touches the document, so only the module-load-time references (`document.body`, the document and
/// the `window` the runtime attaches its own listeners to) need answering, not a whole element tree.
const MIN_GLOBALS: &str = r#"
globalThis.document = { body: {}, addEventListener() {} };
globalThis.window = { addEventListener() {}, history: { pushState() {}, replaceState() {} }, location: { pathname: '/', search: '' } };
"#;

/// Write `harness` to a temp `.mjs` and run it under `node`, returning `(stdout, stderr, ok)`.
fn run_module_under_node(harness: &str) -> (String, String, bool) {
    let dir = std::env::temp_dir().join(format!("logos-temporal-host-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("probe.mjs");
    std::fs::write(&path, harness).expect("write probe");
    let out = Command::new("node")
        .arg(&path)
        .output()
        .expect("node preflighted present but failed to spawn");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

#[test]
fn a_date_and_moment_component_program_runs_under_the_browser_host_in_real_v8() {
    if !require_tool("node") {
        return;
    }
    let reference = tw_outcome(SRC);
    assert_eq!(
        reference.error, None,
        "the reference tier must run this program"
    );
    let wasm = compile_to_wasm(SRC).unwrap_or_else(|e| panic!("must compile: {e:?}\n{SRC}"));
    let bytes_csv = wasm
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let host = browser_host_js("__temporal_probe__");
    let from = "const bytes = await (await fetch(WASM_URL)).arrayBuffer();";
    assert!(
        host.contains(from),
        "the wasm loader shape in browser_host_js drifted"
    );
    let host_inlined = host.replace(
        from,
        &format!("const bytes = new Uint8Array([{bytes_csv}]);"),
    );
    let harness = format!("{MIN_GLOBALS}\n{host_inlined}\n");
    let (stdout, stderr, ok) = run_module_under_node(&harness);
    assert!(
        ok,
        "the browser host must run a Date/Moment component program under real V8:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(
        stdout.trim_end(),
        reference.output.trim_end(),
        "the browser host must display exactly the components the reference tier does"
    );
}

#[test]
fn a_date_and_moment_component_program_runs_under_the_node_shim() {
    if !require_tool("node") {
        return;
    }
    let reference = tw_outcome(SRC);
    assert_eq!(
        reference.error, None,
        "the reference tier must run this program"
    );
    let wasm = compile_to_wasm(SRC).unwrap_or_else(|e| panic!("must compile: {e:?}\n{SRC}"));
    let dir = std::env::temp_dir().join(format!("logos-temporal-shim-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(dir.join("probe.wasm"), &wasm).expect("write wasm");
    let mut f = std::fs::File::create(dir.join("probe.mjs")).expect("write shim");
    f.write_all(wasm_host_shim("probe").as_bytes())
        .expect("write shim");
    drop(f);
    let out = Command::new("node")
        .arg(dir.join("probe.mjs"))
        .output()
        .expect("node preflighted present but failed to spawn");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "the node shim must run a Date/Moment component program:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(
        stdout.trim_end(),
        reference.output.trim_end(),
        "the node shim must display exactly the components the reference tier does"
    );
}
