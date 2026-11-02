//! ★ G6-full LOCK — the EMITTED WASM-DOM bundle runs in a REAL V8/JS DOM. ★
//!
//! The wasmi-hosted DOM tests (`wasm_dom`, `dom_reconciliation_lock`) drive a compiled module
//! against RUST host functions — they never execute the emitted JAVASCRIPT glue
//! (`web_host::browser_host_js`) in a real JS engine. This lock closes that gap: it emits the ACTUAL
//! wasm-DOM bundle for the G17 bound-input form, then runs it under **node (V8)** against a small
//! self-contained DOM shim that implements exactly the host surface the emitted module + host use,
//! and drives the G17 scenario end to end in real JavaScript.
//!
//! What the real-V8 run proves that the Rust MockDom cannot:
//!  * the emitted `browser_host_js` — the production host, byte-for-byte the file the CLI writes
//!    beside `<app>.wasm` — actually LOADS and RUNS in V8 (its `WebAssembly.instantiate`, its `env`
//!    import object, the entry an element's event reaches the page by, and the applier a paint's
//!    committed stream reaches the document by);
//!  * two `input` events typed into a `bound to` input are written IN PLACE: the page's own
//!    reconciler (`vdom.lg`, compiled into the module) keeps the input, and the host applies each
//!    stream by the ids it names, so the SAME real DOM node survives both keystrokes (a naive
//!    rebuild would recreate it and drop focus after one char — the whole G17 bug), and both
//!    characters land.
//!
//! Honesty about the shim (stated so no one mistakes it for a full browser):
//!  * It is a MINIMAL hand-rolled DOM — only the surface the emitted host touches:
//!    `document.{body,createElement,createTextNode,querySelector}`, a `window` with
//!    `history`/`location`/`addEventListener`, and per-node `appendChild`/`insertBefore`/
//!    `removeChild`/`childNodes`/`setAttribute`/`removeAttribute`/`addEventListener`/
//!    `textContent`/`value`/etc. It
//!    does NOT implement layout, real focus/selection, CSS, or event bubbling — a real
//!    browser adds all of that. What it DOES model, faithfully, is NODE IDENTITY: a
//!    reused element is the same JS object across renders, so "did the input node
//!    survive?" is a real, checkable question in V8. That identity is the exact thing
//!    reconciling must preserve, so the shim is sufficient to prove the fix runs in a
//!    real JS engine — while being explicit that it is a shim, not Chromium.
//!  * `node` is gated through `common::require_tool("node")`: absent node is an HONEST
//!    counted skip (`LOGOS_TOOL_SKIP: node`), and PANICS under `LOGOS_REQUIRE_TOOLS`.

mod common;
use common::require_tool;

use std::io::Write;
use std::process::Command;

use logicaffeine_compile::compile::compile_to_wasm;
use logicaffeine_compile::web_host::browser_host_js;

/// The G17 reactive form: a bound `<input>` re-seeded from a `State:` field on every turn,
/// plus a `<p>` echoing it and a hint shown only while the field is empty. The first keystroke
/// flips that `{if}`, so its turn repaints the whole view STRUCTURALLY — a naive rebuild would
/// recreate the input there, and the reconciler keeps its node; the second keystroke only patches
/// the echo's text in place. This is the scenario the Rust MockDom locks in
/// `dom_reconciliation_lock.rs` (`LGX_FORM_APP`) — proven here in real V8.
const FORM_APP: &str = concat!(
    "## Component Form\n",
    "State:\n",
    "    Let name be \"\".\n",
    "View:\n",
    "    <input id=\"f\" type=\"text\" bind={name}>\n",
    "    {if name is \"\":}<span id=\"hint\">type a name</span>{end}\n",
    "    <p id=\"echo\">Hello, {name}</p>\n",
    "\n## Main\n",
    "Let app be createElement(\"div\").\n",
    "Call setAttribute with app and \"id\" and \"root\".\n",
    "Call appendChild with body() and app.\n",
    "Mount Form at \"#root\".\n",
);

/// A minimal, self-contained DOM shim implementing EXACTLY the host surface the emitted
/// `browser_host_js` touches. Installed on `globalThis` (`document`, `window`) before the
/// host body runs. Every node is a plain JS object so NODE IDENTITY is observable — the
/// property the G17 reconciler must preserve. The `__uid` is a stable per-node identity
/// stamp the driver uses to assert "same node survived" from the JS side.
///
/// NOT modeled (a real browser would add these): layout, real focus/selection/scroll,
/// CSS, event bubbling/capture, and the full HTML element zoo. We model node identity,
/// the value/textContent/attribute properties, child lists, and per-node listeners —
/// enough to apply the page's streams and check that the bound input's node survives.
const DOM_SHIM: &str = r#"
// ---- Minimal DOM shim (node identity + the properties the host reads/writes) ----
let __uidCounter = 0;
function makeNode(tag, isText) {
  const n = {
    __uid: ++__uidCounter,
    nodeTag: tag,
    isText: !!isText,
    _textContent: '',
    _value: '',
    _innerHTML: '',
    attributes: new Map(),
    children: [],
    parentNode: null,
    __logosHandlers: undefined,
    _listeners: new Map(), // event -> fn (the live DOM listener the host installed)
  };
  Object.defineProperty(n, 'textContent', {
    // A real `textContent` getter concatenates every descendant text node — LGX gives an
    // interpolated hole its OWN text node (`<p>Hello, {name}</p>` is TWO children, not one
    // `setText`), so reading `_textContent` alone (the old single-assignment shortcut) would
    // read empty for any view holding a hole beside literal text.
    get() {
      if (this.isText) return this._textContent;
      return this.children.map((c) => c.textContent).join('');
    },
    set(v) { this._textContent = String(v); this.children = []; },
  });
  Object.defineProperty(n, 'value', {
    get() { return this._value; },
    set(v) { this._value = String(v); },
  });
  Object.defineProperty(n, 'innerHTML', {
    get() { return this._innerHTML; },
    set(v) { this._innerHTML = String(v); },
  });
  Object.defineProperty(n, 'firstChild', { get() { return this.children[0] || null; } });
  Object.defineProperty(n, 'childNodes', { get() { return this.children; } });
  n.appendChild = function (c) {
    if (c.parentNode) { const i = c.parentNode.children.indexOf(c); if (i >= 0) c.parentNode.children.splice(i, 1); }
    this.children.push(c); c.parentNode = this; return c;
  };
  // `insertBefore(c, null)` appends, as the DOM's does.
  n.insertBefore = function (c, before) {
    if (c.parentNode) { const i = c.parentNode.children.indexOf(c); if (i >= 0) c.parentNode.children.splice(i, 1); }
    const at = before ? this.children.indexOf(before) : -1;
    if (at < 0) this.children.push(c); else this.children.splice(at, 0, c);
    c.parentNode = this; return c;
  };
  n.removeChild = function (c) {
    const i = this.children.indexOf(c); if (i >= 0) this.children.splice(i, 1); c.parentNode = null; return c;
  };
  n.setAttribute = function (k, v) { this.attributes.set(k, String(v)); };
  n.removeAttribute = function (k) { this.attributes.delete(k); };
  n.getAttribute = function (k) { return this.attributes.has(k) ? this.attributes.get(k) : null; };
  n.addEventListener = function (ev, fn) { this._listeners.set(ev, fn); };
  n.removeEventListener = function (ev, fn) { if (this._listeners.get(ev) === fn) this._listeners.delete(ev); };
  return n;
}
const __body = makeNode('body', false);
// A tiny querySelector: `#id` walks the tree for a node with that id attribute.
function __querySelector(sel) {
  if (sel[0] === '#') {
    const id = sel.slice(1);
    const stack = [__body];
    while (stack.length) {
      const n = stack.pop();
      if (n.attributes && n.attributes.get('id') === id) return n;
      for (const c of n.children) stack.push(c);
    }
    return null;
  }
  return null;
}
globalThis.document = {
  body: __body,
  _listeners: new Map(),
  addEventListener(ev, fn) { this._listeners.set(ev, fn); },
  removeEventListener(ev, fn) { if (this._listeners.get(ev) === fn) this._listeners.delete(ev); },
  createElement: (tag) => makeNode(tag, false),
  createTextNode: (t) => { const n = makeNode('', true); n._textContent = String(t); return n; },
  querySelector: __querySelector,
};
globalThis.window = {
  location: { pathname: '/', search: '' },
  history: { pushState() {}, replaceState() {} },
  addEventListener() {},
};
// A helper the driver uses to find the first element of a tag in the live tree.
globalThis.__findByTag = (tag) => {
  const stack = [__body];
  while (stack.length) {
    const n = stack.pop();
    if (!n.isText && n.nodeTag === tag) return n;
    for (const c of n.children) stack.push(c);
  }
  return null;
};
"#;

/// The driver appended AFTER the emitted host body (which ends with `main()`). It types two
/// characters into the bound input by firing the SAME real DOM listener the host installed
/// (`el._listeners.get('input')`) — the exact call a browser makes on an `input` event — with the
/// node's `.value` pre-set to the typed text (as the browser does before dispatching). It captures
/// the input node's identity (`__uid`) before and after each keystroke and prints machine-readable
/// `RESULT:` lines the Rust side parses. If the page's reconciler is working, the `__uid` never
/// changes (same node) and both characters land in `.value` and the echo `<p>`.
fn driver_js() -> String {
    r#"
// ---- Drive two keystrokes into the bound input, in real V8 ----
function fireInput(node, typed) {
  node._value = typed;                       // the browser sets .value before dispatch
  const fn = node._listeners.get('input');
  if (!fn) { throw new Error('no input listener on the bound node'); }
  fn({ target: node });                       // exactly what a browser input event does
}
const input0 = globalThis.__findByTag('input');
if (!input0) { console.log('RESULT: FAIL no-input-node'); throw new Error('no input'); }
const uid0 = input0.__uid;
// Keystroke 1: "H"
fireInput(input0, 'H');
const input1 = globalThis.__findByTag('input');
const uid1 = input1 ? input1.__uid : -1;
// Keystroke 2: "He"
fireInput(input1, 'He');
const input2 = globalThis.__findByTag('input');
const uid2 = input2 ? input2.__uid : -1;
const echo = globalThis.__findByTag('p');
const echoText = echo ? echo.textContent : '<no-echo>';
console.log('RESULT: uid0=' + uid0 + ' uid1=' + uid1 + ' uid2=' + uid2);
console.log('RESULT: value=' + JSON.stringify(input2 ? input2._value : '<no-input>'));
console.log('RESULT: echo=' + JSON.stringify(echoText));
const sameNode = (uid0 === uid1 && uid1 === uid2 && uid2 !== -1);
console.log('RESULT: sameNode=' + sameNode);
"#
    .to_string()
}

/// Assemble the full `.mjs`: the DOM shim, then the emitted browser host (its `fetch(...)`
/// wasm loader swapped for the inline bytes — the SAME swap `web_host::assemble_preview`
/// does for the Studio preview, so the host body is byte-for-byte the emitted production
/// host except for how the bytes are fetched), then the keystroke driver.
fn assemble_harness(wasm: &[u8]) -> String {
    let bytes_csv = wasm
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let host = browser_host_js("__browser_lock__");
    // The loader is now ONE FIXED LINE reading a `const WASM_URL` the emitter prepends, rather
    // than a per-page `fetch("./<name>.wasm")` — so the swap no longer depends on the page name.
    let from = "const bytes = await (await fetch(WASM_URL)).arrayBuffer();";
    let to = format!("const bytes = new Uint8Array([{bytes_csv}]);");
    assert!(
        host.contains(from),
        "the emitted host's wasm loader shape drifted — update the inline-bytes swap"
    );
    let host_inlined = host.replace(from, &to);
    format!("{DOM_SHIM}\n{host_inlined}\n{}", driver_js())
}

/// Run `harness` under `node` as an ES module (stdin), returning its stdout. The caller
/// has already preflighted node via `require_tool`, so a spawn failure is a real error.
fn run_under_node(harness: &str) -> String {
    let mut child = Command::new("node")
        .args(["--input-type=module"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("node preflighted present but failed to spawn");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(harness.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "the emitted wasm-DOM bundle threw in real V8:\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}"
    );
    stdout
}

/// Parse a `RESULT: key=...` line's value out of the node stdout.
fn result_field<'a>(stdout: &'a str, key: &str) -> Option<&'a str> {
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("RESULT: ") {
            for tok in rest.split(' ') {
                if let Some(v) = tok.strip_prefix(&format!("{key}=")) {
                    return Some(v);
                }
            }
        }
    }
    None
}

#[test]
fn bound_input_reconciles_in_place_in_real_v8() {
    if !require_tool("node") {
        return;
    }
    let wasm = compile_to_wasm(FORM_APP).expect("the G17 form compiles to wasm-DOM");
    let harness = assemble_harness(&wasm);
    let stdout = run_under_node(&harness);

    // The bound input node must SURVIVE both keystrokes — same JS object (same __uid).
    assert_eq!(
        result_field(&stdout, "sameNode"),
        Some("true"),
        "the bound input's DOM node must be REUSED across both keystrokes in real V8 (the G17 \
         reconciliation) — a recreate would drop focus after one char.\nnode stdout:\n{stdout}"
    );
    // And the __uids are all equal + valid (belt-and-suspenders over the boolean).
    let uid0 = result_field(&stdout, "uid0");
    let uid1 = result_field(&stdout, "uid1");
    let uid2 = result_field(&stdout, "uid2");
    assert!(
        uid0.is_some() && uid0 == uid1 && uid1 == uid2,
        "same node id across renders:\n{stdout}"
    );
    assert_ne!(uid0, Some("-1"), "the input node was lost:\n{stdout}");

    // Both characters landed: the second keystroke's value is the full "He".
    let value_line = stdout
        .lines()
        .find(|l| l.starts_with("RESULT: value="))
        .unwrap_or("");
    assert!(
        value_line.contains("\"He\""),
        "both typed characters must land in the bound input's value in real V8:\n{stdout}"
    );
    // And the derived echo <p> reflects the bound signal after the second keystroke.
    let echo_line = stdout
        .lines()
        .find(|l| l.starts_with("RESULT: echo="))
        .unwrap_or("");
    assert!(
        echo_line.contains("Hello, He"),
        "the derived echo text must update to the new bound value in real V8:\n{stdout}"
    );
}

/// A form whose first keystroke moves the bound input under a new parent: the element drawn there
/// is a new one, whatever reconciles the page — an element is never carried between parents.
const MOVING_FORM_APP: &str = concat!(
    "## Component Form\n",
    "State:\n",
    "    Let name be \"\".\n",
    "View:\n",
    "    {if name is \"\":}<input id=\"f\" type=\"text\" bind={name}>\
     {else:}<div><input id=\"f\" type=\"text\" bind={name}></div>{end}\n",
    "    <p id=\"echo\">Hello, {name}</p>\n",
    "\n## Main\n",
    "Let app be createElement(\"div\").\n",
    "Call setAttribute with app and \"id\" and \"root\".\n",
    "Call appendChild with body() and app.\n",
    "Mount Form at \"#root\".\n",
);

/// TEETH: prove the assertion is not theater. The same driver, run against a page whose keystroke
/// genuinely replaces the input, must SEE the replacement — a different `__uid` — so the primary
/// test's `sameNode=true` is something the driver found, not the only answer it can give. A
/// regression back to clear-and-recreate makes the primary page look exactly like this one.
#[test]
fn a_replaced_input_is_seen_as_replaced_so_the_lock_has_teeth() {
    if !require_tool("node") {
        return;
    }
    let wasm = compile_to_wasm(MOVING_FORM_APP).expect("the moving form compiles to wasm-DOM");
    let stdout = run_under_node(&assemble_harness(&wasm));
    assert_eq!(
        result_field(&stdout, "sameNode"),
        Some("false"),
        "an input drawn under a new parent is a new node, and the driver must see it — proving \
         the primary test's same-node assertion has teeth.\nnode stdout:\n{stdout}"
    );
}
