//! B5 — `largo emit wasm-dom` produces a browser-ready LOGOS→WASM DOM app.
//!
//! A DOM program emits a `.wasm` plus a browser host (`.html` + `.js`) that wires
//! the `dom_*` imports to the real DOM and events/fetch to the exported
//! `__logos_call_closure` trampoline. We can't run a browser here, so we verify the
//! artifacts are written and the host `.js` is valid ES-module syntax (node --check).

#[path = "cli_common/mod.rs"]
mod common;
use common::{require_tool, scaffold};

use std::process::Command;

fn largo() -> Command {
    common::largo()
}

/// Check `src` under `node --input-type=module --check`, returning whether node
/// accepted it as valid ES-module syntax. The caller has already preflighted node's
/// presence (via `require_tool`), so a spawn failure here is a real error, not a
/// silent skip.
fn node_checks_module(src: &str) -> Result<(), String> {
    use std::io::Write;
    let mut child = Command::new("node")
        .args(["--input-type=module", "--check"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("node preflighted present but failed to spawn");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(src.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

const APP: &str = "# App\n\n\
    ## Main\n\
    Let h be createElement(\"h1\").\n\
    Call setText with h and \"Hello from LOGOS\".\n\
    Call appendChild with body() and h.\n\
    Let onClick be (e: Int) -> setText(h, \"clicked!\").\n\
    Call addEventListener with h and \"click\" and onClick.\n";

#[test]
fn emit_wasm_dom_writes_wasm_html_and_js() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path(), "app");
    std::fs::write(dir.path().join("src/main.lg"), APP).unwrap();

    let out = largo()
        .args(["emit", "wasm-dom"])
        .current_dir(dir.path())
        .output()
        .expect("largo emit spawns");
    assert!(
        out.status.success(),
        "largo emit wasm-dom failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let target = dir.path().join("target");
    let wasm = target.join("app.wasm");
    let html = target.join("app.html");
    let js = target.join("app.js");
    assert!(wasm.exists(), "app.wasm not written");
    assert!(html.exists(), "app.html not written");
    assert!(js.exists(), "app.js not written");

    let js_src = std::fs::read_to_string(&js).unwrap();
    // The host must wire the DOM + the LOGOS trampoline.
    assert!(
        js_src.contains("dom_add_event_listener"),
        "js missing DOM event wiring"
    );
    assert!(
        js_src.contains("__logos_call_closure"),
        "js missing the trampoline call"
    );
    assert!(
        js_src.contains("document.createElement"),
        "js missing real DOM impl"
    );
    // The reactive runtime: the import a paint's committed stream reaches the document by, the
    // settle() re-render loop run after every re-entry, the __logos_render export call, and the
    // interpolation format host (so `"{count}"` renders the value, not a MissingDefinition trap).
    assert!(
        js_src.contains("dom_apply_words:"),
        "js missing the committed-stream applier"
    );
    assert!(
        js_src.contains("settle()"),
        "js missing the settle() auto re-render loop"
    );
    assert!(
        js_src.contains("__logos_render"),
        "js missing the re-render export call"
    );
    assert!(
        js_src.contains("fmt_i64_into"),
        "js missing the interpolation format host"
    );
    let html_src = std::fs::read_to_string(&html).unwrap();
    assert!(html_src.contains("app.js"), "html doesn't load the host js");

    // Validate the host js is well-formed ES-module syntax. When node is present this
    // ACTUALLY runs `node --check` and fails on invalid JS; when node is absent it
    // records a machine-readable `LOGOS_TOOL_SKIP: node` marker (and PANICS under
    // LOGOS_REQUIRE_TOOLS) — never a silent skip-as-pass.
    if !require_tool("node") {
        return;
    }
    node_checks_module(&js_src)
        .unwrap_or_else(|err| panic!("emitted browser host is not valid JS:\n{err}"));
}

/// PROOF THE VALIDATOR HAS TEETH: node must REJECT deliberately-broken JS. If node
/// accepted this, the `node --check` gate above would be theater — passing a green
/// on any emitted string. This pins that the validator actually validates.
#[test]
fn node_check_rejects_broken_javascript() {
    if !require_tool("node") {
        return;
    }
    let broken = "export const x = ;\nfunction (\n"; // a syntax error node must catch
    assert!(
        node_checks_module(broken).is_err(),
        "node --check ACCEPTED syntactically-broken JS — the JS validator is toothless \
         (so the emitted-host check proves nothing)"
    );
    // And it accepts well-formed ES-module syntax, so the gate is not merely always-red.
    assert!(
        node_checks_module("export const x = 1;\n").is_ok(),
        "node --check rejected valid ES-module syntax"
    );
}
