//! A `vdom.lg` paint handed to both hosts a page runs on, and each host's document read back after
//! every step.
//!
//! A scenario is a LOGOS program that paints through `vdom.lg` (`vdOpen`/`vdText`/`vdClose`/…) and
//! ends each paint with `commit(root)`, or ends an in-place write (`vdPatchText`/`vdPatchValue`/
//! `vdPatchChecked`) with `flush(root)`. A step prints the stream it hands over — its words, then
//! each of its strings as UTF-8 bytes, so no string can pass for a separator — and the markup the
//! retained tree serializes, then gives the stream to the page's host with `vdApply()`, the one door
//! a paint reaches a host by.
//!
//! The program runs on the tree-walker and on the VM (and, under `wasm-jit`, compiled to wasm on
//! wasmi), which must print the same steps and leave the same document behind. Its steps then meet
//! the two hosts:
//!
//! - the native recorder (`semantics::dom`), each stream applied in order through
//!   `patch::decode_words` and `patch::apply` — what `vdApply` runs on the tree-walker and the VM;
//! - `runtime.js` under node, where the program compiled to wasm hands each stream to
//!   `dom_apply_words`: the browser's own applier, in V8, over a DOM whose nodes are plain objects,
//!   so whether a node SURVIVED a step is a question about object identity (`__uid`).
//!
//! Node is preflighted through `common::require_tool`: without it the browser half is an honest
//! counted skip (`Step::browser` is `None`), and `LOGOS_REQUIRE_TOOLS=node` makes it a failure.

#![allow(dead_code)]

use logicaffeine_compile::semantics::dom::{self, DomOp};
use logicaffeine_compile::semantics::patch;
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};

/// The two helpers a scenario ends its steps with.
const STEP_HELPERS: &str = "
## To flush(root: Int):
    Show vdPatchWords().
    For each s in vdPatchStrings():
        Show text_bytes(s).
    Show \"--\".
    Show text_bytes(vdSerializeChildren(root)).
    Show vdApply().
    Show \"@step\".

## To commit(root: Int):
    Let problem be vdCommit(root).
    If problem != \"\":
        Show \"REFUSED \" + problem.
    flush(root).
";

/// One step of a scenario, as each side saw it.
pub struct Step {
    /// The stream the step handed over, decoded through the wire format's own decoder.
    pub ops: Vec<DomOp>,
    /// The markup `vdom.lg`'s retained tree serializes the root's children to after the step.
    pub retained: String,
    /// The native recorder's `<body>` content after the step.
    pub native: String,
    /// The browser's, when node runs.
    pub browser: Option<BrowserDocument>,
}

/// The browser's document after a step: its body's content, and every node under the body in
/// document order.
#[derive(serde::Deserialize)]
pub struct BrowserDocument {
    pub markup: String,
    pub nodes: Vec<BrowserNode>,
}

/// One node of the browser's document: its tag (`#text` for text), its identity, and how many
/// listeners are attached to it for each event.
#[derive(serde::Deserialize)]
pub struct BrowserNode {
    pub tag: String,
    pub uid: u64,
    pub listeners: BTreeMap<String, usize>,
}

impl BrowserDocument {
    /// The identities of the `tag` nodes, in document order.
    pub fn uids(&self, tag: &str) -> Vec<u64> {
        self.nodes
            .iter()
            .filter(|n| n.tag == tag)
            .map(|n| n.uid)
            .collect()
    }

    /// How many `event` listeners each `tag` element carries, in document order.
    pub fn listeners(&self, tag: &str, event: &str) -> Vec<usize> {
        self.nodes
            .iter()
            .filter(|n| n.tag == tag)
            .map(|n| n.listeners.get(event).copied().unwrap_or(0))
            .collect()
    }
}

/// A scenario's steps, and the streams that let the native recorder be stood at any of them.
pub struct Paint {
    pub steps: Vec<Step>,
    streams: Vec<Vec<DomOp>>,
    source: String,
}

impl Paint {
    /// Runs `query` against the native recorder standing where step `k` left it: a fresh document
    /// given steps `0..=k` in order.
    pub fn native_at<T>(&self, k: usize, query: impl FnOnce() -> T) -> T {
        dom::reset();
        for ops in &self.streams[..=k] {
            patch::apply(ops).expect("a stream the recorder honoured once is honoured again");
        }
        query()
    }

    /// The native recorder's `tag` elements after step `k`, in document order: the ids the streams
    /// name them by, which is their identity on that host.
    pub fn native_ids(&self, k: usize, tag: &str) -> Vec<i64> {
        self.native_at(k, || dom::query_selector_all(tag))
    }

    /// The browser's document after step `k`, or `None` when node is absent.
    pub fn browser(&self, k: usize) -> Option<&BrowserDocument> {
        self.steps[k].browser.as_ref()
    }

    /// Asserts that after every step both hosts hold exactly what the retained tree serializes: the
    /// tree the next diff is computed against is the page on screen.
    pub fn assert_hosts_follow(&self) {
        for (k, s) in self.steps.iter().enumerate() {
            assert_eq!(
                s.native, s.retained,
                "step {k}: the native recorder is not the retained tree\n{}",
                self.source
            );
            if let Some(b) = &s.browser {
                assert_eq!(
                    b.markup, s.retained,
                    "step {k}: the browser is not the retained tree\n{}",
                    self.source
                );
            }
        }
    }
}

/// `defs` (any `## To` helpers the scenario draws with) and `main` (the `## Main` body) as one
/// program with the step helpers.
fn program(defs: &str, main: &str) -> String {
    format!("{STEP_HELPERS}\n{defs}\n## Main\n{main}")
}

/// The ints of a `Show`n `Seq of Int` line: `[1, 2, 3]`, or `[]`.
fn ints(line: &str) -> Vec<i64> {
    let body = line.trim().trim_start_matches('[').trim_end_matches(']');
    if body.trim().is_empty() {
        return Vec::new();
    }
    body.split(',')
        .map(|w| {
            w.trim()
                .parse()
                .unwrap_or_else(|_| panic!("an Int word in {line:?}"))
        })
        .collect()
}

/// The text whose UTF-8 bytes a `Show`n `text_bytes` line lists.
fn utf8(line: &str) -> String {
    String::from_utf8(ints(line).into_iter().map(|b| b as u8).collect()).expect("UTF-8")
}

/// One step as the program printed it: the stream's words and strings, and the retained markup.
struct Printed {
    words: Vec<i64>,
    strings: Vec<String>,
    retained: String,
}

/// Reads the program's output back into its steps; a line that belongs to no step is a failure.
fn printed_steps(output: &str, source: &str) -> Vec<Printed> {
    assert!(
        !output.contains("REFUSED"),
        "a commit was refused:\n{output}\n{source}"
    );
    let mut lines = output.lines().filter(|l| !l.is_empty()).peekable();
    let mut out = Vec::new();
    while lines.peek().is_some() {
        let words = ints(lines.next().unwrap());
        let mut strings = Vec::new();
        for line in lines.by_ref() {
            if line == "--" {
                break;
            }
            strings.push(utf8(line));
        }
        let retained = utf8(lines.next().expect("the retained markup"));
        assert_eq!(
            lines.next(),
            Some("0"),
            "a step's stream opens no hydration, so nothing mismatches\n{source}"
        );
        assert_eq!(
            lines.next(),
            Some("@step"),
            "a step ends with its marker\n{output}\n{source}"
        );
        out.push(Printed {
            words,
            strings,
            retained,
        });
    }
    out
}

/// The native recorder's body content: `dom::render()` without the `<body>` it is wrapped in.
fn body_content() -> String {
    let doc = dom::render();
    doc.strip_prefix("<body>")
        .and_then(|d| d.strip_suffix("</body>"))
        .unwrap_or_else(|| panic!("the recorder renders one body: {doc}"))
        .to_string()
}

/// Runs the program on every in-process tier from a fresh document; they must print the same steps
/// and leave the same document. Answers the agreed output and document.
fn run_tiers(source: &str) -> (String, String) {
    use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
    dom::reset();
    let tw = tw_outcome(source);
    assert!(
        tw.error.is_none(),
        "the scenario runs on the tree-walker: {:?}\n{source}",
        tw.error
    );
    let tw_doc = body_content();
    dom::reset();
    let vm = vm_outcome(source);
    assert_eq!(
        (vm.output.trim(), vm.error.as_deref()),
        (tw.output.trim(), None),
        "TIER DIVERGENCE — the VM disagrees with the tree-walker\n{source}"
    );
    assert_eq!(
        body_content(),
        tw_doc,
        "the VM's vdApply left another document than the tree-walker's\n{source}"
    );
    #[cfg(feature = "wasm-jit")]
    {
        use crate::compile_common::wasm_host::{link_native_stream, run_outcome_hosted, Capped};
        dom::reset();
        let wasm = run_outcome_hosted(source, None, link_native_stream::<Capped>)
            .unwrap_or_else(|why| panic!("the wasm module does not run: {why}\n{source}"));
        assert_eq!(
            (wasm.output.trim(), wasm.failure.as_deref()),
            (tw.output.trim(), None),
            "TIER DIVERGENCE — the wasm module disagrees with the tree-walker\n{source}"
        );
        assert_eq!(
            body_content(),
            tw_doc,
            "the wasm module's vdApply left another document\n{source}"
        );
    }
    (tw.output, tw_doc)
}

/// A DOM for `runtime.js` under node, small enough to read: elements and text nodes as plain
/// objects (so a node that survives a step is the same object, stamped with the same `__uid`),
/// child lists with `appendChild`/`insertBefore`/`removeChild`, attributes, the `value`/`checked`
/// properties, raw `innerHTML`, and listeners counted per event. `__serialize` writes a body the
/// way the native recorder renders one; the escapes and void elements are spliced in from the
/// tables the recorder reads (`lgx::html`), so the two serializers share their rules.
fn dom_shim() -> String {
    use logicaffeine_language::lgx::html::{ATTR_ESCAPES, TEXT_ESCAPES, VOID_ELEMENTS};
    let table = |t: &[(char, &str)]| {
        serde_json::to_string(
            &t.iter()
                .map(|(c, r)| (c.to_string(), r.to_string()))
                .collect::<Vec<_>>(),
        )
        .unwrap()
    };
    format!(
        "const __TEXT_ESCAPES = new Map({text});\nconst __ATTR_ESCAPES = new Map({attr});\nconst __VOID = new Set({void});\n{SHIM}",
        text = table(TEXT_ESCAPES),
        attr = table(ATTR_ESCAPES),
        void = serde_json::to_string(VOID_ELEMENTS).unwrap(),
    )
}

const SHIM: &str = r#"
let __uids = 0;
const __escape = (s, table) => [...String(s)].map((c) => table.get(c) ?? c).join('');
class __Node {
  constructor(tag, isText) {
    this.__uid = ++__uids;
    this.nodeTag = tag;
    this.isText = isText;
    this._text = '';
    this._html = '';
    this.value = '';
    this.checked = false;
    this.attributes = new Map();
    this.children = [];
    this.parentNode = null;
    this._listeners = new Map();
  }
  get nodeType() { return this.isText ? 3 : 1; }
  get localName() { return this.nodeTag; }
  get tagName() { return this.nodeTag.toUpperCase(); }
  get childNodes() { return this.children; }
  get firstChild() { return this.children[0] || null; }
  get nextSibling() { const p = this.parentNode; if (!p) return null; return p.children[p.children.indexOf(this) + 1] || null; }
  get isConnected() { let n = this; while (n.parentNode) n = n.parentNode; return n === __body; }
  get data() { return this._text; }
  get textContent() { return this.isText ? this._text : this._text + this.children.map((c) => c.textContent).join(''); }
  set textContent(v) { this._text = String(v); if (!this.isText) { for (const c of this.children) c.parentNode = null; this.children = []; } }
  get innerHTML() { return this._html; }
  set innerHTML(v) { this._html = String(v); }
  detach() { const p = this.parentNode; if (p) { p.children.splice(p.children.indexOf(this), 1); this.parentNode = null; } }
  appendChild(c) { c.detach(); this.children.push(c); c.parentNode = this; return c; }
  insertBefore(c, before) {
    c.detach();
    const at = before ? this.children.indexOf(before) : -1;
    if (at < 0) this.children.push(c); else this.children.splice(at, 0, c);
    c.parentNode = this;
    return c;
  }
  removeChild(c) { if (c.parentNode === this) c.detach(); return c; }
  remove() { this.detach(); }
  setAttribute(k, v) { this.attributes.set(k, String(v)); }
  getAttribute(k) { return this.attributes.has(k) ? this.attributes.get(k) : null; }
  hasAttribute(k) { return this.attributes.has(k); }
  removeAttribute(k) { this.attributes.delete(k); }
  getAttributeNames() { return [...this.attributes.keys()]; }
  addEventListener(ev, fn) { const fs = this._listeners.get(ev) || []; if (!fs.includes(fn)) fs.push(fn); this._listeners.set(ev, fs); }
  removeEventListener(ev, fn) { const fs = (this._listeners.get(ev) || []).filter((g) => g !== fn); if (fs.length) this._listeners.set(ev, fs); else this._listeners.delete(ev); }
  dispatchEvent() { return true; }
}
const __body = new __Node('body', false);
const __walk = (n, found, keep) => { for (const c of n.children) { if (keep(c)) found.push(c); __walk(c, found, keep); } return found; };
const __matches = (n, sel) => !n.isText && (sel[0] === '#' ? n.getAttribute('id') === sel.slice(1) : n.nodeTag === sel);
globalThis.document = {
  body: __body,
  documentElement: __body,
  createElement: (tag) => new __Node(tag, false),
  createTextNode: (t) => { const n = new __Node('#text', true); n._text = String(t); return n; },
  querySelector: (sel) => __walk(__body, [], (n) => __matches(n, sel))[0] || null,
  querySelectorAll: (sel) => __walk(__body, [], (n) => __matches(n, sel)),
  addEventListener() {},
  removeEventListener() {},
};
globalThis.window = { location: { pathname: '/', search: '' }, history: { pushState() {}, replaceState() {} }, addEventListener() {}, removeEventListener() {} };
const __serialize = (n) => {
  if (n.isText) return __escape(n._text, __TEXT_ESCAPES);
  const attrs = [...n.attributes.keys()].sort().map((k) => ` ${k}="${__escape(n.attributes.get(k), __ATTR_ESCAPES)}"`).join('');
  const state = (n.checked ? ' checked' : '') + (n.value !== '' ? ` value="${__escape(n.value, __ATTR_ESCAPES)}"` : '');
  if (__VOID.has(n.nodeTag)) return `<${n.nodeTag}${attrs}${state}>`;
  return `<${n.nodeTag}${attrs}${state}>${__escape(n._text, __TEXT_ESCAPES)}${n._html}${n.children.map(__serialize).join('')}</${n.nodeTag}>`;
};
const __document = () => ({
  markup: __body.children.map(__serialize).join(''),
  nodes: __walk(__body, [], () => true).map((n) => ({
    tag: n.nodeTag,
    uid: n.__uid,
    listeners: Object.fromEntries([...n._listeners].map(([ev, fs]) => [ev, fs.length])),
  })),
});
const __print = console.log;
console.log = (line) => { if (line === '@step') __print('@step ' + JSON.stringify(__document())); else __print(line); };
"#;

/// Runs `source`, compiled to wasm, in `runtime.js` under node over [`dom_shim`]: the program's
/// output, each `@step` line carrying the browser's document at that moment. `None` when node is
/// absent.
fn run_in_browser(source: &str) -> Option<String> {
    if !crate::common::require_tool("node") {
        return None;
    }
    let wasm = logicaffeine_compile::compile::compile_to_wasm(source)
        .unwrap_or_else(|e| panic!("the scenario compiles to wasm: {e:?}\n{source}"));
    let bytes = wasm
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let host = logicaffeine_compile::web_host::browser_host_js("vdom_hosts");
    let loader = "const bytes = await (await fetch(WASM_URL)).arrayBuffer();";
    assert!(
        host.contains(loader),
        "runtime.js loads its module in one line, which this harness replaces"
    );
    let host = host.replace(loader, &format!("const bytes = new Uint8Array([{bytes}]);"));
    let mut child = Command::new("node")
        .arg("--input-type=module")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("node was preflighted and spawns");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{}\n{host}\n", dom_shim()).as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "the scenario threw in runtime.js:\n{stdout}\n{}\n{source}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(stdout)
}

/// Paints the scenario `main` (drawing with the `## To` helpers `defs`) on every tier and hands
/// every step to both hosts.
pub fn paint(defs: &str, main: &str) -> Paint {
    let source = program(defs, main);
    let (output, final_doc) = run_tiers(&source);
    let printed = printed_steps(&output, &source);
    assert!(
        !printed.is_empty(),
        "the scenario takes at least one step\n{source}"
    );

    let browser = run_in_browser(&source).map(|stdout| {
        let (mut plain, mut docs) = (Vec::new(), Vec::new());
        for line in stdout.lines().filter(|l| !l.is_empty()) {
            match line.strip_prefix("@step ") {
                Some(json) => {
                    plain.push("@step");
                    docs.push(
                        serde_json::from_str::<BrowserDocument>(json).unwrap_or_else(|e| {
                            panic!("the browser's document reads back: {e}\n{json}")
                        }),
                    );
                }
                None => plain.push(line),
            }
        }
        let expected: Vec<&str> = output.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(
            plain, expected,
            "runtime.js ran another program than the tree-walker\n{source}"
        );
        docs
    });
    let mut browser = browser.map(|docs| docs.into_iter().map(Some).collect::<Vec<_>>());

    dom::reset();
    let mut steps = Vec::new();
    let mut streams = Vec::new();
    for (k, p) in printed.into_iter().enumerate() {
        let ops = patch::decode_words(&p.words, &p.strings)
            .unwrap_or_else(|e| panic!("step {k}: vdom.lg's stream is not wire v2: {e}\n{source}"));
        patch::apply(&ops).unwrap_or_else(|e| {
            panic!("step {k}: the native recorder refuses the stream: {e}\n{source}")
        });
        let browser = browser
            .as_mut()
            .map(|docs| docs[k].take().expect("one browser document per step"));
        steps.push(Step {
            ops: ops.clone(),
            retained: p.retained,
            native: body_content(),
            browser,
        });
        streams.push(ops);
    }
    assert_eq!(
        steps.last().unwrap().native,
        final_doc,
        "the steps applied one by one leave another document than the program's own vdApply\n{source}"
    );
    Paint {
        steps,
        streams,
        source,
    }
}
