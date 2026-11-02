//! ★ LOCK — the browser runtime is EXECUTED, not string-matched. ★
//!
//! `assets/web/runtime.js` is the host a compiled LOGOS page runs inside: it owns the node
//! table, the DOM imports, the event trampoline, the fetch engine and the router. Until now it
//! was a Rust `format!` template checked by a handful of `contains()` assertions on the
//! generated text — which can tell you a substring is present and nothing whatsoever about
//! whether the thing runs.
//!
//! These tests run it under node against a real compiled module, with a minimal DOM standing in
//! for the browser's. What they pin is the contract between the wasm module and its host: the
//! imports the module asks for are the ones the runtime provides, and driving them produces the
//! page the other tiers produce.
//!
//! Gated on `node` through the honest skip channel, so a box without it records a SKIP rather
//! than a silent pass.

mod common;

use common::require_tool;
use std::io::Write;
use std::process::Command;

/// Write `js` to a temp file and run it under node, returning `(stdout, stderr, ok)`.
fn run_node(js: &str) -> (String, String, bool) {
    let dir = std::env::temp_dir().join(format!("logos-runtime-js-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("probe.mjs");
    let mut f = std::fs::File::create(&path).expect("write probe");
    f.write_all(js.as_bytes()).expect("write probe");
    drop(f);
    let out = Command::new("node").arg(&path).output().expect("node runs");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.success(),
    )
}

/// A DOM small enough to read and complete enough to run a LOGOS page: elements, text nodes,
/// attributes, values, a parent chain, and `querySelector` over id/class/tag. It mirrors the
/// native recorder in `semantics::dom`, which is what makes comparing their output meaningful.
const FAKE_DOM: &str = r#"
class N {
  constructor(tag) { this.tag = tag; this.children = []; this.attrs = {}; this.value = ''; this.text = ''; this.listeners = {}; this.parentNode = null; }
  get nodeType() { return this.tag === '#text' ? 3 : 1; }
  get localName() { return this.tag; }
  get data() { return this.text; }
  get firstChild() { return this.children[0] || null; }
  get nextSibling() { const p = this.parentNode; if (!p) return null; const i = p.children.indexOf(this); return p.children[i + 1] || null; }
  get isConnected() { let n = this; while (n.parentNode) n = n.parentNode; return n === body; }
  appendChild(c) { if (c.parentNode) c.parentNode.removeChild(c); this.children.push(c); c.parentNode = this; return c; }
  // `removeChild` and `insertBefore` are what a structural diff is made of: a dropped row is
  // detached, a keyed reorder moves rows. `parentNode` is kept so the host can find a node's parent.
  removeChild(c) { this.children = this.children.filter((k) => k !== c); if (c.parentNode === this) c.parentNode = null; return c; }
  insertBefore(c, ref) {
    if (c.parentNode) c.parentNode.removeChild(c);
    const at = ref ? this.children.indexOf(ref) : -1;
    if (at < 0) this.children.push(c); else this.children.splice(at, 0, c);
    c.parentNode = this;
    return c;
  }
  setAttribute(k, v) { this.attrs[k] = String(v); }
  getAttribute(k) { return k in this.attrs ? this.attrs[k] : null; }
  hasAttribute(k) { return k in this.attrs; }
  removeAttribute(k) { delete this.attrs[k]; }
  get attributes() { return Object.keys(this.attrs).map((name) => ({ name, value: this.attrs[name] })); }
  addEventListener(k, f) { const fs = (this.listeners[k] ||= []); if (!fs.includes(f)) fs.push(f); }
  removeEventListener(k, f) { if (this.listeners[k]) this.listeners[k] = this.listeners[k].filter((g) => g !== f); }
  get textContent() { return this.text + this.children.map(c => c.textContent).join(''); }
  set textContent(v) { this.text = String(v); for (const c of this.children) c.parentNode = null; this.children = []; }
  get innerHTML() { return this.children.map(c => c.render()).join(''); }
  set innerHTML(v) { this.raw = v; }
  render() {
    if (this.tag === '#text') return this.text;
    const a = Object.keys(this.attrs).sort().map(k => ` ${k}="${this.attrs[k]}"`).join('')
      + (this.value ? ` value="${this.value}"` : '');
    return `<${this.tag}${a}>${this.text}${this.raw || ''}${this.children.map(c => c.render()).join('')}</${this.tag}>`;
  }
}
const matches = (n, sel) => {
  if (n.tag === '#text') return false;
  if (sel.startsWith('#')) return n.attrs.id === sel.slice(1);
  if (sel.startsWith('.')) return (n.attrs.class || '').split(/\s+/).includes(sel.slice(1));
  return n.tag === sel;
};
const walk = (n, sel, out) => { if (matches(n, sel)) out.push(n); n.children.forEach(c => walk(c, sel, out)); return out; };
const body = new N('body');
globalThis.document = {
  body,
  listeners: {},
  addEventListener(k, f) { const fs = (this.listeners[k] ||= []); if (!fs.includes(f)) fs.push(f); },
  removeEventListener(k, f) { if (this.listeners[k]) this.listeners[k] = this.listeners[k].filter((g) => g !== f); },
  createElement: (t) => new N(t),
  createTextNode: (t) => { const n = new N('#text'); n.text = String(t); return n; },
  querySelector: (s) => walk(body, s, [])[0] || null,
  querySelectorAll: (s) => walk(body, s, []),
};
globalThis.window = { addEventListener() {}, location: { pathname: '/', search: '' }, history: { pushState() {}, replaceState() {} } };
globalThis.fetch = async () => { throw new Error('no network in this probe'); };
"#;

#[test]
fn the_runtime_is_a_real_file_that_node_can_parse() {
    if !require_tool("node") {
        return;
    }
    let js = logicaffeine_compile::web_host::BROWSER_RUNTIME_JS;
    assert!(
        !js.contains("{{") && !js.contains("}}"),
        "a real JS file has single braces — doubled ones mean it is still a format! template"
    );
    let (_, stderr, ok) = run_node(&format!("{FAKE_DOM}\nconst WASM_URL='';\n{js}\n"));
    // It will fail at INSTANTIATION (there is no module here); what must not happen is a syntax
    // or reference error, which is what a template-shaped file produces.
    assert!(
        !stderr.contains("SyntaxError") && !stderr.contains("ReferenceError"),
        "the runtime must at least parse and resolve under node: ok={ok}\n{stderr}"
    );
}

/// The runtime follows a link in place through ONE click listener on the document, made when the
/// runtime loads: the page's paint, its turns and their repaints add no other, so a click is
/// considered for client navigation once however often the page has repainted.
#[test]
fn the_runtime_hears_link_clicks_through_one_document_listener() {
    if !require_tool("node") {
        return;
    }
    const PAGE: &str = "## Component App\nState:\n    Let n be 0.\nActions:\n    To bump:\n        Set n to n + 1.\n\
        View:\n    <p id=\"out\">{n}</p>\n    <button id=\"go\" on-click={bump}>go</button>\n\n## Main\nMount App.\n";
    let wasm = logicaffeine_compile::compile::compile_to_wasm(PAGE)
        .unwrap_or_else(|e| panic!("the page must compile to wasm: {e:?}"));
    let bytes = wasm
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let js = logicaffeine_compile::web_host::BROWSER_RUNTIME_JS.replace(
        "const bytes = await (await fetch(WASM_URL)).arrayBuffer();",
        &format!("const bytes = new Uint8Array([{bytes}]);"),
    );
    let probe = format!(
        "{FAKE_DOM}\nconst WASM_URL='';\n{js}\n\
         const clicks = () => (document.listeners.click || []).length;\n\
         console.log('LOADED:' + clicks());\n\
         const go = document.querySelector('#go');\n\
         for (let i = 0; i < 3; i++) go.listeners.click[0]({{ preventDefault() {{}}, stopPropagation() {{}}, key: '' }});\n\
         console.log('TURNED:' + clicks());\n\
         console.log('TEXT:' + document.querySelector('#out').textContent);\n"
    );
    let (stdout, stderr, ok) = run_node(&probe);
    assert!(ok, "the page must run in the browser runtime:\n{stderr}");
    let line = |tag: &str| {
        stdout
            .lines()
            .find_map(|l| l.strip_prefix(tag))
            .unwrap_or_default()
            .to_string()
    };
    assert_eq!(
        line("TEXT:"),
        "3",
        "the three turns ran and repainted, or this proves nothing:\n{stdout}"
    );
    assert_eq!(
        line("LOADED:"),
        "1",
        "the loaded runtime holds one click listener on the document:\n{stdout}"
    );
    assert_eq!(
        line("TURNED:"),
        "1",
        "and three turns and their repaints added none:\n{stdout}"
    );
}

#[test]
fn a_compiled_lgx_page_runs_in_the_browser_runtime_and_renders_what_the_other_tiers_render() {
    if !require_tool("node") {
        return;
    }
    const PAGE: &str = "## Component App\nState:\n    Let n be 7.\nView:\n    <p id=\"n\">Count: {n}</p>\n\n## Main\nMount App.\n";

    // What the reference tier renders, for comparison.
    logicaffeine_compile::semantics::dom::reset();
    let tw = logicaffeine_compile::compile::tw_outcome(PAGE);
    assert_eq!(tw.error, None, "the page must run on the reference tier");
    let expected = logicaffeine_compile::semantics::dom::render();

    let wasm = match logicaffeine_compile::compile::compile_to_wasm(PAGE) {
        Ok(w) => w,
        Err(e) => panic!("the page must compile to wasm: {e:?}"),
    };
    let bytes = wasm
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let js = logicaffeine_compile::web_host::BROWSER_RUNTIME_JS.replace(
        "const bytes = await (await fetch(WASM_URL)).arrayBuffer();",
        &format!("const bytes = new Uint8Array([{bytes}]);"),
    );
    let probe = format!(
        "{FAKE_DOM}\nconst WASM_URL='';\n{js}\nconsole.log('RENDER:' + '<body>' + document.body.children.map(c => c.render()).join('') + '</body>');\n"
    );
    let (stdout, stderr, ok) = run_node(&probe);
    assert!(ok, "the page must RUN in the browser runtime:\n{stderr}");
    let rendered = stdout
        .lines()
        .find_map(|l| l.strip_prefix("RENDER:"))
        .unwrap_or_else(|| panic!("the probe must report a render:\n{stdout}\n{stderr}"));
    assert_eq!(
        rendered, expected,
        "the browser runtime must produce the page the reference tier produces"
    );
}

/// ★ THE BULK SHIM, EXECUTED. `dom_apply` decodes a whole patch stream and applies it, and until
/// this test it had never been RUN — fifty lines of decoder in a host file, reviewed and never
/// executed, which is the exact shape of every "looks wired up" defect this campaign has found.
///
/// The stream is emitted by the Rust encoder, carried through a real compiled module, and decoded
/// by the real `runtime.js`. What it must build is the document the reference tier builds.
#[test]
fn the_bulk_patch_shim_applies_a_stream_in_the_browser_runtime() {
    if !require_tool("node") {
        return;
    }
    use logicaffeine_compile::semantics::dom::{self, DomOp};
    use logicaffeine_compile::semantics::patch;

    // Deliberately awkward content: multi-byte text, an empty attribute value (how a boolean
    // attribute is written), and a checkbox presence — the three things a naive decoder breaks.
    let ops = vec![
        DomOp::CreateElement {
            node: 1,
            tag: "div".into(),
        },
        DomOp::SetAttribute {
            node: 1,
            name: "class".into(),
            value: "card".into(),
        },
        DomOp::CreateTextNode {
            node: 2,
            text: "héllo → 世界".into(),
        },
        DomOp::AppendChild {
            parent: 1,
            child: 2,
        },
        DomOp::CreateElement {
            node: 3,
            tag: "button".into(),
        },
        DomOp::SetAttribute {
            node: 3,
            name: "disabled".into(),
            value: String::new(),
        },
        DomOp::AppendChild {
            parent: 1,
            child: 3,
        },
        DomOp::AppendChild {
            parent: 0,
            child: 1,
        },
    ];
    let literal = patch::encode(&ops)
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let src = format!("## Main\nLet patches be [{literal}].\nCall applyPatches with patches.\n");

    dom::reset();
    let tw = logicaffeine_compile::compile::tw_outcome(&src);
    assert_eq!(tw.error, None, "the reference tier must apply the stream");
    let expected = dom::render();

    let wasm = logicaffeine_compile::compile::compile_to_wasm(&src)
        .unwrap_or_else(|e| panic!("the stream program must compile to wasm: {e:?}"));
    let bytes = wasm
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let js = logicaffeine_compile::web_host::BROWSER_RUNTIME_JS.replace(
        "const bytes = await (await fetch(WASM_URL)).arrayBuffer();",
        &format!("const bytes = new Uint8Array([{bytes}]);"),
    );
    let probe = format!(
        "{FAKE_DOM}\nconst WASM_URL='';\n{js}\nconsole.log('RENDER:' + '<body>' + document.body.children.map(c => c.render()).join('') + '</body>');\n"
    );
    let (stdout, stderr, ok) = run_node(&probe);
    assert!(
        ok,
        "the bulk shim must RUN in the browser runtime:\n{stderr}"
    );
    let rendered = stdout
        .lines()
        .find_map(|l| l.strip_prefix("RENDER:"))
        .unwrap_or_else(|| panic!("the probe must report a render:\n{stdout}\n{stderr}"));
    assert_eq!(
        rendered, expected,
        "the browser shim built a different document from the one the reference tier built"
    );
}

/// A malformed stream must THROW in the shim rather than half-paint, the same rule the Rust
/// decoder follows. A host that applied the records preceding the damage would leave a page that
/// looks built and is not.
///
/// EXECUTED, not string-matched — the whole point of this file. An earlier draft of this test
/// grepped `runtime.js` for the error message, which proves a substring is present and nothing
/// about whether the guard fires.
#[test]
fn the_bulk_shim_refuses_a_malformed_stream() {
    if !require_tool("node") {
        return;
    }
    use logicaffeine_compile::semantics::dom::DomOp;
    use logicaffeine_compile::semantics::patch;

    // A real stream with its first opcode corrupted to one no host knows.
    let mut bytes = patch::encode(&[
        DomOp::CreateElement {
            node: 1,
            tag: "p".into(),
        },
        DomOp::AppendChild {
            parent: 0,
            child: 1,
        },
    ]);
    bytes[0] = 0xEE;
    let literal = bytes
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let src = format!("## Main\nLet patches be [{literal}].\nCall applyPatches with patches.\n");

    // The module carries the bytes as data, so it compiles; the HOST is the only guard on this
    // tier, which is precisely why the guard has to be exercised.
    let wasm = logicaffeine_compile::compile::compile_to_wasm(&src)
        .unwrap_or_else(|e| panic!("a program carrying a bad buffer still compiles: {e:?}"));
    let b = wasm
        .iter()
        .map(|x| x.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let js = logicaffeine_compile::web_host::BROWSER_RUNTIME_JS.replace(
        "const bytes = await (await fetch(WASM_URL)).arrayBuffer();",
        &format!("const bytes = new Uint8Array([{b}]);"),
    );
    let probe = format!(
        "{FAKE_DOM}\nconst WASM_URL='';\n{js}\nconsole.log('SURVIVED:' + document.body.children.length);\n"
    );
    let (stdout, stderr, ok) = run_node(&probe);
    let all = format!("{stdout}\n{stderr}");
    assert!(
        all.contains("unknown patch opcode"),
        "the shim must REFUSE an unknown opcode, naming it. Got:\n{all}"
    );
    assert!(
        !stdout.contains("SURVIVED:1"),
        "the shim applied part of a malformed stream — a page that looks built and is not:\n{all}"
    );
    let _ = ok;
}

/// ★ A KEYED REORDER MOVES THE ROWS IN THE BROWSER.
///
/// `{for each x in items, keyed by x:}` promises a row keeps its identity across a reorder. The
/// page's own reconciler (`vdom.lg`, compiled into the module) matches each row to its retained
/// node by key and moves the ones off the longest run already in order; the browser applies those
/// moves by the ids the committed stream names. So after the flip the SAME row nodes stand in the
/// new order.
///
/// The oracle is the reference tier's own render, so this asserts the browser agrees with it —
/// which is the whole contract, not a shape this test invented.
#[test]
fn a_keyed_reorder_moves_the_rows_in_the_browser_runtime() {
    if !require_tool("node") {
        return;
    }
    use logicaffeine_compile::semantics::dom;

    const VIEW: &str = "## Component App\nState:\n\
        \x20   Let items be [\"a\", \"b\", \"c\"].\n\
        Actions:\n\
        \x20   To flip:\n\
        \x20       Set items to [\"c\", \"a\", \"b\"].\n\
        View:\n\
        \x20   <ul>{for each x in items, keyed by x:}<li>{x}</li>{end}</ul>\n\
        \x20   <button id=\"go\" on-click={flip}>go</button>\n\n";

    // What the page looks like after the flip, on the tier that defines the answer.
    dom::reset();
    let tw = logicaffeine_compile::compile::tw_outcome(&format!(
        "{VIEW}## Main\nMount App.\nClick \"#go\".\n"
    ));
    assert_eq!(tw.error, None, "the reference tier must run the page");
    let expected = dom::render();
    assert!(
        expected.find("c").unwrap_or(0) < expected.find("a").unwrap_or(usize::MAX),
        "the ORACLE itself must show the reordered list, or this test proves nothing: {expected}"
    );

    // The same page in the browser runtime, with the button pressed there.
    let wasm =
        logicaffeine_compile::compile::compile_to_wasm(&format!("{VIEW}## Main\nMount App.\n"))
            .unwrap_or_else(|e| panic!("the page must compile to wasm: {e:?}"));
    let bytes = wasm
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let js = logicaffeine_compile::web_host::BROWSER_RUNTIME_JS.replace(
        "const bytes = await (await fetch(WASM_URL)).arrayBuffer();",
        &format!("const bytes = new Uint8Array([{bytes}]);"),
    );
    // ★ THE ROWS ARE TAGGED BEFORE THE FLIP, and that is the whole assertion. The rendered TEXT
    // comes out right either way — positional reuse patches each row's text in place, so the list
    // reads correctly while every row's real node stayed where it was. What a key buys is that the
    // NODE travels with its row, which is why an input keeps what you typed into it. Reading the
    // tags back in document order is the only way to see the difference.
    let probe = format!(
        "{FAKE_DOM}\nconst WASM_URL='';\n{js}\n\
         const go = document.querySelector('#go');\n\
         const before = document.querySelectorAll('li');\n\
         before.forEach((n, i) => {{ n.__row = i; }});\n\
         if (!go) {{ console.log('ORDER:NO-BUTTON'); }} else {{\n\
         \x20 const fns = go.listeners.click || [];\n\
         \x20 if (!fns.length) {{ console.log('ORDER:NO-LISTENER'); }} else {{\n\
         \x20   fns[0]({{ preventDefault() {{}}, stopPropagation() {{}}, target: go, key: '' }});\n\
         \x20   const after = document.querySelectorAll('li');\n\
         \x20   console.log('ORDER:' + after.map(n => n.__row === undefined ? 'new' : n.__row).join(','));\n\
         \x20   console.log('TEXT:' + after.map(n => n.textContent).join(','));\n\
         \x20   console.log('RENDER:' + '<body>' + document.body.children.map(c => c.render()).join('') + '</body>');\n\
         \x20 }}\n}}\n"
    );
    let (stdout, stderr, ok) = run_node(&probe);
    assert!(ok, "the page must run in the browser runtime:\n{stderr}");
    let rendered = stdout
        .lines()
        .find_map(|l| l.strip_prefix("RENDER:"))
        .unwrap_or_else(|| panic!("the probe must report a render:\n{stdout}\n{stderr}"));
    assert_eq!(
        rendered, expected,
        "after a keyed reorder the browser runtime must show the list the reference tier shows"
    );

    let text = stdout
        .lines()
        .find_map(|l| l.strip_prefix("TEXT:"))
        .unwrap_or_default();
    assert_eq!(
        text, "c,a,b",
        "the rows must READ in the new order:\n{stdout}"
    );

    // ★ AND THE SAME THREE NODES MUST BE THERE, MOVED. `0,1,2` here would mean every row stayed
    // put and only its text was rewritten — the list reads correctly and row one's input now
    // holds row two's typing, the failure `keyed by` exists to prevent.
    let order = stdout
        .lines()
        .find_map(|l| l.strip_prefix("ORDER:"))
        .unwrap_or_else(|| panic!("the probe must report an order:\n{stdout}\n{stderr}"));
    assert_eq!(
        order, "2,0,1",
        "the rows must be the SAME nodes in the new order — `0,1,2` means they never moved and \
         only their text was rewritten, which loses everything a row was holding:\n{stdout}"
    );
}

/// ★ A COMPONENT ROW KEEPS ITS IDENTITY TOO — the shape a real list is actually made of.
///
/// `{for each t in todos, keyed by t's id:}<TodoRow todo={t}/>{end}` is what a list looks like once
/// there is more than a word in each row, and it is the shape `component_model_lock`'s own keyed
/// test uses. It is also the one the loop's key could not reach: a child is drawn through a shared
/// `__render_<Name>`, so the placement has the key and the element is created in another function.
///
/// The answer was already in the child's hands. Every render receives its per-instance key — that
/// is what gives two rows independent state — and inside a keyed loop that key IS the row's. So the
/// component's root element carries it, and a component row reconciles by identity exactly as a
/// plain `<li>` row does.
#[test]
fn a_keyed_component_row_keeps_its_node_across_a_reorder() {
    if !require_tool("node") {
        return;
    }
    use logicaffeine_compile::semantics::dom;

    const VIEW: &str = "## Component Row (label: Text)\n\
        View:\n\x20   <li class=\"row\">{label}</li>\n\n\
        ## Component App\nState:\n\
        \x20   Let items be [\"a\", \"b\", \"c\"].\n\
        Actions:\n\
        \x20   To flip:\n\
        \x20       Set items to [\"c\", \"a\", \"b\"].\n\
        View:\n\
        \x20   <ul>{for each x in items, keyed by x:}<Row label={x}/>{end}</ul>\n\
        \x20   <button id=\"go\" on-click={flip}>go</button>\n\n";

    dom::reset();
    let tw = logicaffeine_compile::compile::tw_outcome(&format!(
        "{VIEW}## Main\nMount App.\nClick \"#go\".\n"
    ));
    assert_eq!(tw.error, None, "the reference tier must run the page");
    let expected = dom::render();

    let wasm =
        logicaffeine_compile::compile::compile_to_wasm(&format!("{VIEW}## Main\nMount App.\n"))
            .unwrap_or_else(|e| panic!("the page must compile to wasm: {e:?}"));
    let bytes = wasm
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let js = logicaffeine_compile::web_host::BROWSER_RUNTIME_JS.replace(
        "const bytes = await (await fetch(WASM_URL)).arrayBuffer();",
        &format!("const bytes = new Uint8Array([{bytes}]);"),
    );
    let probe = format!(
        "{FAKE_DOM}\nconst WASM_URL='';\n{js}\n\
         const go = document.querySelector('#go');\n\
         document.querySelectorAll('li').forEach((n, i) => {{ n.__row = i; }});\n\
         if (!go || !(go.listeners.click || []).length) {{ console.log('ORDER:NO-HANDLER'); }} else {{\n\
         \x20 go.listeners.click[0]({{ preventDefault() {{}}, stopPropagation() {{}}, target: go, key: '' }});\n\
         \x20 const after = document.querySelectorAll('li');\n\
         \x20 console.log('ORDER:' + after.map(n => n.__row === undefined ? 'new' : n.__row).join(','));\n\
         \x20 console.log('TEXT:' + after.map(n => n.textContent).join(','));\n\
         \x20 console.log('RENDER:' + '<body>' + document.body.children.map(c => c.render()).join('') + '</body>');\n\
         }}\n"
    );
    let (stdout, stderr, ok) = run_node(&probe);
    assert!(ok, "the page must run in the browser runtime:\n{stderr}");
    let rendered = stdout
        .lines()
        .find_map(|l| l.strip_prefix("RENDER:"))
        .unwrap_or_else(|| panic!("the probe must report a render:\n{stdout}\n{stderr}"));
    assert_eq!(
        rendered, expected,
        "the browser must show what the reference tier shows"
    );

    let text = stdout
        .lines()
        .find_map(|l| l.strip_prefix("TEXT:"))
        .unwrap_or_default();
    assert_eq!(
        text, "c,a,b",
        "the rows must READ in the new order:\n{stdout}"
    );

    let order = stdout
        .lines()
        .find_map(|l| l.strip_prefix("ORDER:"))
        .unwrap_or_default();
    assert_eq!(
        order, "2,0,1",
        "a COMPONENT row must travel with its key — `0,1,2` means the rows never moved and only \
         their content was rewritten, so each row's own state now belongs to its neighbour:\n{stdout}"
    );
}

/// ★★ A ROW'S OWN STATE TRAVELS WITH IT — the promise `keyed by` actually makes.
///
/// A child's instance key decides which state record it draws, and inside a `{for each}` the
/// browser tier built that key from the loop's POSITION. So the row standing in slot 0 always read
/// slot 0's state: reorder the list and **every row's state moved to its neighbour** — what a
/// visitor typed into the first row appeared in whichever row took its place.
///
/// The reference tier already keyed by the item (`component_model_lock` pins a row keeping its own
/// click across a flip), so the two tiers disagreed about who a row was, and only the tier nobody
/// drove was wrong. This is that test, on the tier a person actually uses.
#[test]
fn a_keyed_component_row_keeps_its_own_state_across_a_reorder() {
    if !require_tool("node") {
        return;
    }
    use logicaffeine_compile::semantics::dom;

    const VIEW: &str = "## Component Row (label: Text)\n\
        State:\n\x20   Let clicks be 0.\n\
        Actions:\n\x20   To bump:\n\x20       Set clicks to clicks + 1.\n\
        View:\n\x20   <li class=\"row\" on-click={bump}>{label}{clicks}</li>\n\n\
        ## Component App\nState:\n\
        \x20   Let items be [\"a\", \"b\"].\n\
        Actions:\n\x20   To flip:\n\x20       Set items to [\"b\", \"a\"].\n\
        View:\n\
        \x20   <ul>{for each x in items, keyed by x:}<Row label={x}/>{end}</ul>\n\
        \x20   <button id=\"go\" on-click={flip}>go</button>\n\n";

    // Click the first row, then flip: the reference tier defines what must be on the page after.
    dom::reset();
    let tw = logicaffeine_compile::compile::tw_outcome(&format!(
        "{VIEW}## Main\nMount App.\nClick \".row\".\nClick \"#go\".\n"
    ));
    assert_eq!(tw.error, None, "the reference tier must run the page");
    let expected = dom::render();
    assert!(
        expected.contains("a1") && expected.contains("b0"),
        "the ORACLE must show `a` keeping its click after moving, or this proves nothing: {expected}"
    );

    let wasm =
        logicaffeine_compile::compile::compile_to_wasm(&format!("{VIEW}## Main\nMount App.\n"))
            .unwrap_or_else(|e| panic!("the page must compile to wasm: {e:?}"));
    let bytes = wasm
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let js = logicaffeine_compile::web_host::BROWSER_RUNTIME_JS.replace(
        "const bytes = await (await fetch(WASM_URL)).arrayBuffer();",
        &format!("const bytes = new Uint8Array([{bytes}]);"),
    );
    let probe = format!(
        "{FAKE_DOM}\nconst WASM_URL='';\n{js}\n\
         const ev = () => ({{ preventDefault() {{}}, stopPropagation() {{}}, key: '' }});\n\
         const rows = document.querySelectorAll('li');\n\
         const go = document.querySelector('#go');\n\
         if (!rows.length || !go) {{ console.log('RENDER:NO-PAGE'); }} else {{\n\
         \x20 rows[0].listeners.click[0](ev());\n\
         \x20 go.listeners.click[0](ev());\n\
         \x20 console.log('TEXT:' + document.querySelectorAll('li').map(n => n.textContent).join(','));\n\
         \x20 console.log('RENDER:' + '<body>' + document.body.children.map(c => c.render()).join('') + '</body>');\n\
         }}\n"
    );
    let (stdout, stderr, ok) = run_node(&probe);
    assert!(ok, "the page must run in the browser runtime:\n{stderr}");
    let text = stdout
        .lines()
        .find_map(|l| l.strip_prefix("TEXT:"))
        .unwrap_or_default();
    assert_eq!(
        text, "b0,a1",
        "`a` was clicked and then moved, so it must still read `a1` — `b1` would mean the click \
         stayed with the SLOT and now belongs to a row that was never clicked:\n{stdout}"
    );
    let rendered = stdout
        .lines()
        .find_map(|l| l.strip_prefix("RENDER:"))
        .unwrap_or_default();
    assert_eq!(
        rendered, expected,
        "and the browser must agree with the reference tier"
    );
}

/// ★ A TRAP INSIDE AN ACTION SAYS WHY — the seam existed and only `main()` went through it.
///
/// A compiled LOGOS program reports a runtime failure by TRAPPING, which carries no JS-visible
/// message of its own; `logos_rt_fail` runs immediately before it and leaves the compiler's own
/// words in `failMessage`, and `reportRuntimeFailure` turns that into a console line and a panel.
/// It was wired to exactly one call site — the initial `main()`.
///
/// ★ EVERY OTHER WAY BACK INTO THE MODULE WAS UNGUARDED: a queued handler, a managed listener, a
/// direct listener, the render `settle()` opens, `dom_navigate`, and `popstate`. So a trap in an
/// ACTION — a click handler, which is where a real page spends its life — printed the browser's
/// bare `RuntimeError: unreachable` and nothing else, while the sentence explaining it sat in a
/// variable one function away.
///
/// The fix is one door rather than six `try`/`catch` blocks: a `guarded` wrapper every re-entry
/// goes through, `main()` included. Six copies of a rule is five chances to add a seventh re-entry
/// without it.
#[test]
fn a_trap_inside_an_action_says_why_in_the_browser_runtime() {
    if !require_tool("node") {
        return;
    }
    // `item 99 of xs` is out of bounds, and the compiler's own words for that are
    // "Index 99 out of bounds" — the message this seam exists to surface.
    const PAGE: &str = "## Component App\nState:\n\
        \x20   Let xs be [1, 2].\n\x20   Let n be 0.\n\
        Actions:\n\x20   To boom:\n\x20       Set n to item 99 of xs.\n\
        View:\n\x20   <button id=\"go\" on-click={boom}>go</button>\n\n\
        ## Main\nMount App.\n";

    let wasm = logicaffeine_compile::compile::compile_to_wasm(PAGE)
        .unwrap_or_else(|e| panic!("the page must compile to wasm: {e:?}"));
    let bytes = wasm
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let js = logicaffeine_compile::web_host::BROWSER_RUNTIME_JS.replace(
        "const bytes = await (await fetch(WASM_URL)).arrayBuffer();",
        &format!("const bytes = new Uint8Array([{bytes}]);"),
    );
    let probe = format!(
        "{FAKE_DOM}\nconst WASM_URL='';\n{js}\n\
         const go = document.querySelector('#go');\n\
         try {{ go.listeners.click[0]({{ preventDefault() {{}}, stopPropagation() {{}}, key: '' }}); }}\n\
         catch (e) {{ console.log('THREW:' + (e && e.constructor && e.constructor.name)); }}\n"
    );
    let (stdout, stderr, _) = run_node(&probe);
    let all = format!("{stdout}\n{stderr}");
    assert!(
        all.contains("LOGOS runtime failure"),
        "a trap inside an action must be reported by the runtime, not left as a bare browser \
         error:\n{all}"
    );
    assert!(
        all.contains("out of bounds"),
        "and must carry the compiler's own words — the whole point of capturing them:\n{all}"
    );
    // ★ AND IT STILL FAILS. Reporting must not swallow the trap into a page that looks fine.
    assert!(
        all.contains("THREW:"),
        "the failure must still propagate — a caught-and-hidden trap is a page that lies:\n{all}"
    );
}

/// ★★ `When <field> changes:` RUNS IN A BROWSER — the first effect to reach the tier a person uses.
///
/// In React's terms `useState` worked everywhere and **`useEffect` worked only on the tier you test
/// on**: a stateful component refused every trigger but an element event, `When the page loads` and
/// `When this component appears`, so a page reacting to its own state changing was an honest
/// refusal rather than a working feature.
///
/// This one needed no host import at all. A turn already knows what it wrote, so the effect is
/// decided from state the tier holds — snapshot the watched field, run the action, compare, run the
/// handler. The reference tier has always answered it that way; the browser now does the same.
///
/// ★ AND IT SETTLES ONCE. The handler is INLINED into the turn rather than called as its own turn
/// function — calling it would run a second complete write-back and a second patch, so one click
/// would paint twice.
#[test]
fn a_when_field_changes_effect_runs_in_the_browser_runtime() {
    if !require_tool("node") {
        return;
    }
    use logicaffeine_compile::semantics::dom;

    const VIEW: &str = "## Component App\nState:\n\
        \x20   Let n be 0.\n\x20   Let log be \"\".\n\
        Actions:\n\
        \x20   To bump:\n\x20       Set n to n + 1.\n\
        \x20   When n changes:\n\x20       Set log to log + \"!\".\n\
        View:\n\
        \x20   <p id=\"out\">{n}{log}</p>\n\
        \x20   <button id=\"go\" on-click={bump}>go</button>\n\n";

    // The reference tier defines the answer: two clicks, two changes, two marks.
    dom::reset();
    let tw = logicaffeine_compile::compile::tw_outcome(&format!(
        "{VIEW}## Main\nMount App.\nClick \"#go\".\nClick \"#go\".\n"
    ));
    assert_eq!(tw.error, None, "the reference tier must run the page");
    let expected = dom::render();
    assert!(
        expected.contains("2!!"),
        "the ORACLE must show the effect having run twice, or this proves nothing: {expected}"
    );

    let wasm =
        logicaffeine_compile::compile::compile_to_wasm(&format!("{VIEW}## Main\nMount App.\n"))
            .unwrap_or_else(|e| panic!("a page with a change effect must compile to wasm: {e:?}"));
    let bytes = wasm
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let js = logicaffeine_compile::web_host::BROWSER_RUNTIME_JS.replace(
        "const bytes = await (await fetch(WASM_URL)).arrayBuffer();",
        &format!("const bytes = new Uint8Array([{bytes}]);"),
    );
    let probe = format!(
        "{FAKE_DOM}\nconst WASM_URL='';\n{js}\n\
         const ev = () => ({{ preventDefault() {{}}, stopPropagation() {{}}, key: '' }});\n\
         const go = document.querySelector('#go');\n\
         if (!go) {{ console.log('RENDER:NO-BUTTON'); }} else {{\n\
         \x20 go.listeners.click[0](ev());\n\
         \x20 go.listeners.click[0](ev());\n\
         \x20 console.log('TEXT:' + document.querySelector('#out').textContent);\n\
         \x20 console.log('RENDER:' + '<body>' + document.body.children.map(c => c.render()).join('') + '</body>');\n\
         }}\n"
    );
    let (stdout, stderr, ok) = run_node(&probe);
    assert!(ok, "the page must run in the browser runtime:\n{stderr}");
    let text = stdout
        .lines()
        .find_map(|l| l.strip_prefix("TEXT:"))
        .unwrap_or_default();
    assert_eq!(
        text, "2!!",
        "the effect must run once per change — `2` alone means it never ran, and more marks than \
         clicks means it retriggered on its own writes:\n{stdout}"
    );
    let rendered = stdout
        .lines()
        .find_map(|l| l.strip_prefix("RENDER:"))
        .unwrap_or_default();
    assert_eq!(
        rendered, expected,
        "and the browser must agree with the reference tier"
    );
}

#[test]
fn a_change_effect_that_writes_its_own_field_does_not_loop() {
    // ★ THE BOUNDED-TURN PROPERTY, and the reason the reference tier compares values ONCE rather
    // than settling to a fixed point. An effect that writes the field it watches is an ordinary
    // thing to write by accident; if the browser re-checked afterwards it would spin forever, and
    // a page that hangs on one click is worse than one that never reacts at all.
    if !require_tool("node") {
        return;
    }
    use logicaffeine_compile::semantics::dom;

    const VIEW: &str = "## Component App\nState:\n\x20   Let n be 0.\n\
        Actions:\n\
        \x20   To bump:\n\x20       Set n to n + 1.\n\
        \x20   When n changes:\n\x20       Set n to n + 10.\n\
        View:\n\
        \x20   <p id=\"out\">{n}</p>\n\
        \x20   <button id=\"go\" on-click={bump}>go</button>\n\n";

    dom::reset();
    let tw = logicaffeine_compile::compile::tw_outcome(&format!(
        "{VIEW}## Main\nMount App.\nClick \"#go\".\n"
    ));
    assert_eq!(
        tw.error, None,
        "the reference tier must run it without looping"
    );
    let expected = dom::render();

    let wasm =
        logicaffeine_compile::compile::compile_to_wasm(&format!("{VIEW}## Main\nMount App.\n"))
            .unwrap_or_else(|e| panic!("must compile: {e:?}"));
    let bytes = wasm
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let js = logicaffeine_compile::web_host::BROWSER_RUNTIME_JS.replace(
        "const bytes = await (await fetch(WASM_URL)).arrayBuffer();",
        &format!("const bytes = new Uint8Array([{bytes}]);"),
    );
    let probe = format!(
        "{FAKE_DOM}\nconst WASM_URL='';\n{js}\n\
         const go = document.querySelector('#go');\n\
         go.listeners.click[0]({{ preventDefault() {{}}, stopPropagation() {{}}, key: '' }});\n\
         console.log('TEXT:' + document.querySelector('#out').textContent);\n"
    );
    let (stdout, stderr, ok) = run_node(&probe);
    assert!(ok, "the page must not hang or trap:\n{stderr}");
    let text = stdout
        .lines()
        .find_map(|l| l.strip_prefix("TEXT:"))
        .unwrap_or_default();
    assert!(
        expected.contains(&format!(">{text}<")),
        "the browser answered {text:?} and the reference tier rendered {expected:?} — the two \
         tiers must agree about how many times an effect that writes its own field runs"
    );
}

/// ★★★ LOCK — a TEXT HOLE showing state updates in a browser. ★★★
///
/// `<p>{n}</p>` beside a button that changes `n` is the first page anybody writes and the shape the
/// scaffold ships. In a browser it **did not update at all**: the click ran, the state changed, and
/// the document kept its old text forever.
///
/// ★ IT IS THE IN-PLACE PATCH THAT WAS DEAD, not the turn. Everything around it worked, which is
/// exactly why nothing caught it:
///
/// | shape                                | patched or repainted | worked |
/// |--------------------------------------|----------------------|--------|
/// | `<p>{n}</p>`                         | patch (Text site)    | **no** |
/// | `<p title={n}>`                      | patch (Attr site)    | yes    |
/// | `<p>{if on:}{n}{end}</p>`            | repaint (control flow) | yes  |
/// | a `{for each}` whose collection changes | repaint            | yes    |
///
/// ★ AND EVERY BROWSER TEST IN THE SUITE SAT ON ONE OF THE WORKING ROWS. The reconciliation locks
/// drive a `bind`, which is a VALUE site; the keyed-list locks change a collection, which repaints;
/// the memory ratchet counts turns rather than reading text. The one square nothing stood on was
/// the plain text hole — the square every page occupies.
///
/// The reference tier patches it correctly, so a cross-tier comparison run anywhere but a browser
/// agrees. This test drives the real `runtime.js` over a real compiled module, which is the only
/// place the disagreement is visible.

/// Click `#go` `times` times in the browser runtime and report `#out`'s text.
fn counter_text_after_clicks(view: &str, times: usize) -> String {
    let wasm =
        logicaffeine_compile::compile::compile_to_wasm(&format!("{view}## Main\nMount App.\n"))
            .unwrap_or_else(|e| panic!("the page must compile to wasm: {e:?}\n{view}"));
    let bytes = wasm
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let js = logicaffeine_compile::web_host::BROWSER_RUNTIME_JS.replace(
        "const bytes = await (await fetch(WASM_URL)).arrayBuffer();",
        &format!("const bytes = new Uint8Array([{bytes}]);"),
    );
    let clicks = "go.listeners.click[0](ev());".repeat(times);
    let probe = format!(
        "{FAKE_DOM}\nconst WASM_URL='';\n{js}\n\
         const ev = () => ({{ preventDefault() {{}}, stopPropagation() {{}}, key: '' }});\n\
         const go = document.querySelector('#go');\n\
         if (!go || !(go.listeners.click || []).length) {{ console.log('OUT:NO-HANDLER'); }} else {{\n\
         \x20 {clicks}\n\
         \x20 console.log('OUT:' + document.querySelector('#out').textContent);\n\
         }}\n"
    );
    let (stdout, stderr, ok) = run_node(&probe);
    assert!(ok, "the page must run in the browser runtime:\n{stderr}");
    stdout
        .lines()
        .find_map(|l| l.strip_prefix("OUT:"))
        .unwrap_or_else(|| panic!("the probe must report the text:\n{stdout}\n{stderr}"))
        .to_string()
}

const COUNTER: &str = "## Component App\nState:\n\x20   Let n be 0.\n\
    Actions:\n\x20   To bump:\n\x20       Set n to n + 1.\n\
    View:\n\x20   <p id=\"out\">{n}</p>\n\
    \x20   <button id=\"go\" on-click={bump}>go</button>\n\n";

#[test]
fn a_text_hole_showing_state_updates_in_the_browser() {
    if !require_tool("node") {
        return;
    }
    assert_eq!(
        counter_text_after_clicks(COUNTER, 1),
        "1",
        "the first page anybody writes — a number and a button — did not change when the button \
         was pressed. The turn ran and the state changed; the in-place patch of the text site \
         wrote nothing."
    );
    assert_eq!(
        counter_text_after_clicks(COUNTER, 3),
        "3",
        "and it keeps up over several turns"
    );
}

#[test]
fn a_text_hole_holding_text_updates_too() {
    if !require_tool("node") {
        return;
    }
    let view = "## Component App\nState:\n\x20   Let s be \"a\".\n\
        Actions:\n\x20   To bump:\n\x20       Set s to \"b\".\n\
        View:\n\x20   <p id=\"out\">{s}</p>\n\
        \x20   <button id=\"go\" on-click={bump}>go</button>\n\n";
    assert_eq!(counter_text_after_clicks(view, 1), "b");
}

#[test]
fn the_browser_agrees_with_the_reference_tier_about_a_counter() {
    // The oracle, so this file is not asserting a number it invented.
    if !require_tool("node") {
        return;
    }
    use logicaffeine_compile::semantics::dom;
    dom::reset();
    let tw = logicaffeine_compile::compile::tw_outcome(&format!(
        "{COUNTER}## Main\nMount App.\nClick \"#go\".\nShow the text of \"#out\".\n"
    ));
    assert_eq!(tw.error, None, "the reference tier must run the counter");
    assert_eq!(
        tw.output.trim(),
        "1",
        "and it patches the text site correctly"
    );
    assert_eq!(
        counter_text_after_clicks(COUNTER, 1),
        tw.output.trim(),
        "the browser must agree with the tier that defines the answer"
    );
}

#[test]
fn the_shapes_that_already_worked_still_work() {
    // ★ THE CONTROLS, and they are the load-bearing half of this file. Without them it reads as
    // "the browser does not update", when the truth is far more specific: the PATCH of a TEXT site
    // did nothing while every neighbouring path was fine. That distinction is why it hid.
    if !require_tool("node") {
        return;
    }
    let attr = "## Component App\nState:\n\x20   Let n be 0.\n\
        Actions:\n\x20   To bump:\n\x20       Set n to n + 1.\n\
        View:\n\x20   <p id=\"out\" title={n}>fixed</p>\n\
        \x20   <button id=\"go\" on-click={bump}>go</button>\n\n";
    assert_eq!(
        counter_text_after_clicks(attr, 1),
        "fixed",
        "an attribute site leaves the text"
    );

    let repainted = "## Component App\nState:\n\x20   Let n be 0.\n\x20   Let on be true.\n\
        Actions:\n\x20   To bump:\n\x20       Set n to n + 1.\n\
        View:\n\x20   <p id=\"out\">{if on:}{n}{end}</p>\n\
        \x20   <button id=\"go\" on-click={bump}>go</button>\n\n";
    assert_eq!(
        counter_text_after_clicks(repainted, 1),
        "1",
        "a hole under control flow repaints rather than patching, and that path always worked"
    );
}

/// ★ O7 — THE PAGE STILL ANSWERS AFTER A TRAPPING TURN, in a real browser runtime.
///
/// `turn_rollback_fuzz`'s ignored twin records this as an open gap and gives the reason as "a wasm
/// trap is unrecoverable — the turn takes the page with it". ★ THAT IS NOT A WEBASSEMBLY FACT.
/// A trap unwinds to the HOST; it does not poison the instance, and the module stays callable. The
/// state half was already safe by construction — a turn reads state into locals and writes back
/// only at the END, so a trap never reaches the write-back and commits nothing — so what was never
/// checked is simply whether the NEXT turn runs.
///
/// So this is the experiment, run against the real `runtime.js` under node: trap a turn, let it be
/// reported and rethrown exactly as `a_trap_inside_an_action_says_why_in_the_browser_runtime`
/// requires, then click a DIFFERENT button and read the DOM. A page that answers is a page that
/// survived; a page that does not is the poisoning that was assumed but never demonstrated.
///
/// The first turn sets `n` to 5 before it traps. Its write-back is never reached, so the second
/// turn must find `n` at 0 and leave 3 — one assertion covering BOTH halves: recovery, and the
/// rollback that makes recovery worth having.
#[test]
fn the_page_still_answers_after_a_trapping_turn_in_the_browser_runtime() {
    if !require_tool("node") {
        return;
    }
    const PAGE: &str = "## Component App\nState:\n\
        \x20   Let n be 0.\n\
        Actions:\n\
        \x20   To bad:\n\x20       Set n to 5.\n\x20       Set n to item 99 of [1].\n\
        \x20   To good:\n\x20       Set n to 3.\n\
        View:\n\x20   <p id=\"n\">{n}</p>\
        <button id=\"b\" on-click={bad}>x</button><button id=\"g\" on-click={good}>y</button>\n\n\
        ## Main\nMount App.\n";

    let wasm = logicaffeine_compile::compile::compile_to_wasm(PAGE)
        .unwrap_or_else(|e| panic!("the page must compile to wasm: {e:?}"));
    let bytes = wasm
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let js = logicaffeine_compile::web_host::BROWSER_RUNTIME_JS.replace(
        "const bytes = await (await fetch(WASM_URL)).arrayBuffer();",
        &format!("const bytes = new Uint8Array([{bytes}]);"),
    );
    let probe = format!(
        "{FAKE_DOM}\nconst WASM_URL='';\n{js}\n\
         const ev = () => ({{ preventDefault() {{}}, stopPropagation() {{}}, key: '' }});\n\
         try {{ document.querySelector('#b').listeners.click[0](ev()); }}\n\
         catch (e) {{ console.log('TRAPPED'); }}\n\
         try {{ document.querySelector('#g').listeners.click[0](ev()); }}\n\
         catch (e) {{ console.log('SECOND-THREW:' + e); }}\n\
         console.log('AFTER:<body>' + document.body.children.map(c => c.render()).join('') + '</body>');\n"
    );
    let (stdout, stderr, _) = run_node(&probe);
    let all = format!("{stdout}\n{stderr}");
    assert!(
        all.contains("TRAPPED"),
        "the first turn must still trap and still propagate:\n{all}"
    );
    assert!(
        !all.contains("SECOND-THREW:"),
        "the SECOND turn must run — if the instance were poisoned by the first trap this is where \
         it would show, and it is the claim the ignored `turn_rollback_fuzz` twin rests on:\n{all}"
    );
    let after = stdout
        .lines()
        .find_map(|l| l.strip_prefix("AFTER:"))
        .unwrap_or_default();
    assert!(
        after.contains(">3<"),
        "the page must answer the second click — and with 3, not 5: the trapping turn set n to 5 \
         and never reached its write-back, so it committed nothing:\n{after}\n{all}"
    );
}

/// ★ `When an action fails (e: Text):` RUNS IN A BROWSER — the last tier gap in the trigger
/// catalogue, and it needed no host import.
///
/// ★ THE HOST CANNOT KNOW WHOSE TURN FAILED, AND DOES NOT HAVE TO. A trap unwinds to the host,
/// which learns only that something failed; it has no idea which component's action was mid-flight.
/// The MODULE does, because every turn writes it down before anything can fail — so the dispatch is
/// compiled LOGOS like every other trigger, and the host's whole contribution is to call
/// `__lgx_on_failure` with the message it already captured for the panel.
///
/// ★ AND THE STATE IS ALREADY SAFE WHEN THE HANDLER RUNS. A turn reads state into locals and writes
/// back only at the END, so a trap never reaches the write-back: the handler paints from the record
/// the failed turn started from, not from something half-applied. `n` must therefore still be 0.
///
/// The handler is handed the compiler's own words rather than a literal, because a failure handler
/// that cannot say WHAT failed is a worse version of the panel it replaces.
#[test]
fn an_action_that_fails_reaches_its_handler_in_the_browser_runtime() {
    if !require_tool("node") {
        return;
    }
    const PAGE: &str = "## Component App\nState:\n\
        \x20   Let n be 0.\n\x20   Let problem be \"-\".\n\
        Actions:\n\
        \x20   To boom:\n\x20       Set n to 5.\n\x20       Set n to item 99 of [1].\n\
        \x20   When an action fails (e: Text):\n\x20       Set problem to e.\n\
        View:\n\x20   <p id=\"p\">{problem}</p><p id=\"n\">{n}</p>\
        <button id=\"go\" on-click={boom}>go</button>\n\n\
        ## Main\nMount App.\n";

    let wasm = logicaffeine_compile::compile::compile_to_wasm(PAGE)
        .unwrap_or_else(|e| panic!("the page must compile to wasm: {e:?}"));
    let bytes = wasm
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let js = logicaffeine_compile::web_host::BROWSER_RUNTIME_JS.replace(
        "const bytes = await (await fetch(WASM_URL)).arrayBuffer();",
        &format!("const bytes = new Uint8Array([{bytes}]);"),
    );
    let probe = format!(
        "{FAKE_DOM}\nconst WASM_URL='';\n{js}\n\
         try {{ document.querySelector('#go').listeners.click[0]({{ preventDefault() {{}}, stopPropagation() {{}}, key: '' }}); }}\n\
         catch (e) {{ console.log('TRAPPED'); }}\n\
         console.log('AFTER:<body>' + document.body.children.map(c => c.render()).join('') + '</body>');\n"
    );
    let (stdout, stderr, _) = run_node(&probe);
    let all = format!("{stdout}\n{stderr}");
    assert!(
        all.contains("TRAPPED"),
        "the turn must still trap and still propagate:\n{all}"
    );
    let after = stdout
        .lines()
        .find_map(|l| l.strip_prefix("AFTER:"))
        .unwrap_or_default();
    assert!(
        after.contains("out of bounds"),
        "the failure handler must run AND be handed the compiler's own words — a handler that \
         cannot say what failed is a worse version of the panel it replaces:\n{after}\n{all}"
    );
    assert!(
        after.contains("<p id=\"n\">0</p>"),
        "and the trapping turn committed nothing: it set n to 5 and never reached its \
         write-back, so the handler paints from the record the turn started from:\n{after}"
    );
}

/// ★ LOCK — inserting an unkeyed element AHEAD of another unkeyed sibling must not evict that
/// sibling from the retained tree.
///
/// The non-keyed reuse search advances a cursor over the prior children looking for the next one
/// of a matching tag. A single cursor SHARED across every fresh child is wrong: when an earlier
/// fresh child (the newly-appearing `<ul>`) fails to match at the cursor's position and the cursor
/// steps past a prior child of a DIFFERENT tag, that prior child is never offered to a LATER fresh
/// child that actually holds its tag — `#flip`, one position further along, is torn down and a
/// fresh `<button>` is minted in its place even though nothing about the button changed. A cursor
/// PER TAG fixes it: the button reuses the one prior button regardless of what gets inserted ahead
/// of it, which is what keeps a visitor's focus, selection and typed-but-unsubmitted state through
/// a repaint that only reveals a sibling.
#[test]
fn an_element_inserted_ahead_of_a_sibling_does_not_evict_it() {
    if !require_tool("node") {
        return;
    }
    const VIEW: &str = "## Component App\nState:\n\
        \x20   Let on be false.\n\
        Actions:\n\
        \x20   To flip:\n\x20       Set on to not on.\n\
        View:\n\x20   <main>{if on:}<ul><li>a</li></ul>{end}<button id=\"flip\" on-click={flip}>flip</button></main>\n\n";

    logicaffeine_compile::semantics::dom::reset();
    let tw = logicaffeine_compile::compile::tw_outcome(&format!(
        "{VIEW}## Main\nMount App.\nClick \"#flip\".\n"
    ));
    assert_eq!(tw.error, None, "the reference tier must run the page");
    let expected = logicaffeine_compile::semantics::dom::render();

    let wasm =
        logicaffeine_compile::compile::compile_to_wasm(&format!("{VIEW}## Main\nMount App.\n"))
            .unwrap_or_else(|e| panic!("the page must compile to wasm: {e:?}"));
    let bytes = wasm
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let js = logicaffeine_compile::web_host::BROWSER_RUNTIME_JS.replace(
        "const bytes = await (await fetch(WASM_URL)).arrayBuffer();",
        &format!("const bytes = new Uint8Array([{bytes}]);"),
    );
    let probe = format!(
        "{FAKE_DOM}\nconst WASM_URL='';\n{js}\n\
         const before = document.querySelector('#flip');\n\
         before._survivedRepaint = 'yes';\n\
         before.listeners.click[0]({{ preventDefault() {{}}, stopPropagation() {{}}, key: '' }});\n\
         const after = document.querySelector('#flip');\n\
         console.log('SAME:' + (after === before));\n\
         console.log('MARKER:' + after._survivedRepaint);\n\
         console.log('RENDER:' + '<body>' + document.body.children.map(c => c.render()).join('') + '</body>');\n"
    );
    let (stdout, stderr, ok) = run_node(&probe);
    assert!(ok, "the page must run in the browser runtime:\n{stderr}");
    assert!(
        stdout.contains("SAME:true"),
        "the button must be the SAME node after the `<ul>` appears ahead of it — a fresh element \
         means a visitor's focus on it is dropped:\n{stdout}\n{stderr}"
    );
    assert!(
        stdout.contains("MARKER:yes"),
        "and a torn-down node loses whatever it was holding — the marker set before the repaint \
         must still be on the node found after it:\n{stdout}\n{stderr}"
    );
    let rendered = stdout
        .lines()
        .find_map(|l| l.strip_prefix("RENDER:"))
        .unwrap_or_else(|| panic!("the probe must report a render:\n{stdout}\n{stderr}"));
    assert_eq!(
        rendered, expected,
        "and the document itself still matches the reference tier"
    );
}

/// ★ `vdom.lg`'s stream, applied by the browser runtime BY THE IDS IT NAMES. A first paint and a
/// keyed repaint (a moved row, a changed text, a new row with an attribute) are committed and
/// handed over with `vdApply()` from a compiled module; `runtime.js`'s `dom_apply_words` must build
/// the document the reference tier's recorder builds from the same program.
#[test]
fn a_vdom_word_stream_applies_by_its_own_ids() {
    if !require_tool("node") {
        return;
    }
    use logicaffeine_compile::semantics::dom;
    const SRC: &str = "\
## Main
Let root be vdNewRoot(0).
vdOpen(\"ul\").
vdOpenKeyed(\"li\", \"a\").
vdText(\"a\").
vdClose().
vdOpenKeyed(\"li\", \"b\").
vdText(\"b\").
vdClose().
vdClose().
Let first be vdCommit(root).
Let bad be vdApply().
vdOpen(\"ul\").
vdOpenKeyed(\"li\", \"b\").
vdText(\"b\").
vdClose().
vdOpenKeyed(\"li\", \"c\").
vdSetAttribute(\"class\", \"new\").
vdText(\"c\").
vdClose().
vdOpenKeyed(\"li\", \"a\").
vdText(\"A\").
vdClose().
vdClose().
Let second be vdCommit(root).
Let worse be vdApply().
Show first + second + \"{bad}{worse}\".
";
    dom::reset();
    let tw = logicaffeine_compile::compile::tw_outcome(SRC);
    assert_eq!(
        (tw.error.as_deref(), tw.output.trim()),
        (None, "00"),
        "the reference tier applies both commits"
    );
    let expected = dom::render();
    assert_eq!(
        expected,
        "<body><ul><li>b</li><li class=\"new\">c</li><li>A</li></ul></body>"
    );

    let wasm = logicaffeine_compile::compile::compile_to_wasm(SRC)
        .unwrap_or_else(|e| panic!("the vdom program must compile to wasm: {e:?}"));
    let bytes = wasm
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let js = logicaffeine_compile::web_host::BROWSER_RUNTIME_JS.replace(
        "const bytes = await (await fetch(WASM_URL)).arrayBuffer();",
        &format!("const bytes = new Uint8Array([{bytes}]);"),
    );
    let probe = format!(
        "{FAKE_DOM}\nconst WASM_URL='';\n{js}\nconsole.log('RENDER:' + '<body>' + document.body.children.map(c => c.render()).join('') + '</body>');\n"
    );
    let (stdout, stderr, ok) = run_node(&probe);
    assert!(
        ok,
        "the word stream must APPLY in the browser runtime:\n{stderr}"
    );
    let rendered = stdout
        .lines()
        .find_map(|l| l.strip_prefix("RENDER:"))
        .unwrap_or_else(|| panic!("the probe must report a render:\n{stdout}\n{stderr}"));
    assert_eq!(
        rendered, expected,
        "the browser built a different document from the reference tier's"
    );
}

// ---- The host applies a committed stream; it reconciles nothing. ----

/// The identifiers of a reconciler kept in the host — its retained tree (`rc`, `committed`), its
/// staging pass (`inPass`, `staged`), its keyed diff (`diffChildren`, `patchNode`, `mountNode`,
/// `longestIncreasingRun`, `KEY_ATTR`) and its second handler table (`managedHandlers`). A page
/// reconciles in `vdom.lg`, which every tier runs, and hands the host one committed stream per
/// paint; a reconciler here would be a second implementation only a browser executes.
const HOST_RECONCILER: &[&str] = &[
    "rc",
    "committed",
    "inPass",
    "staged",
    "diffChildren",
    "patchNode",
    "mountNode",
    "longestIncreasingRun",
    "KEY_ATTR",
    "managedHandlers",
];

/// Each `HOST_RECONCILER` identifier `js`'s code names, as `line: identifier`. A line comment —
/// a line that opens with `//`, or the ` // ` that ends a line's code — is prose, not code.
fn host_reconciler_names(js: &str) -> Vec<String> {
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '$';
    let mut found = Vec::new();
    for (i, line) in js.lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        let code = line.find(" // ").map_or(line, |at| &line[..at]);
        for name in HOST_RECONCILER {
            let named = code.match_indices(name).any(|(at, _)| {
                let before = code[..at].chars().next_back();
                let after = code[at + name.len()..].chars().next();
                !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
            });
            if named {
                found.push(format!("{}: {name}", i + 1));
            }
        }
    }
    found
}

#[test]
fn runtime_js_contains_no_reconciler() {
    let named = host_reconciler_names(logicaffeine_compile::web_host::BROWSER_RUNTIME_JS);
    assert!(
        named.is_empty(),
        "runtime.js keeps a reconciler of its own; a page's paint is reconciled in `vdom.lg` and \
         reaches the host as a committed stream it applies by id:\n{}",
        named.join("\n")
    );
}

#[test]
fn the_reconciler_census_sees_a_reconciler_and_not_prose() {
    let planted = "// the committed stream, applied by id\n\
                   const rc = { committed: new Map() };\n\
                   const apply = (ops) => applyByIds(ops); // nothing staged here\n\
                   const src = srcOf(node);\n";
    assert_eq!(
        host_reconciler_names(planted),
        ["2: rc", "2: committed"],
        "the census names the reconciler's identifiers in code, and neither a comment nor a longer \
         identifier containing one"
    );
}

/// What the browser runtime prints applying `bytes` through `dom_apply` in a page whose program hands
/// it that buffer: its stdout, and stdout and stderr together.
fn shim_applying(bytes: &[u8]) -> (String, String) {
    let literal = bytes
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let src = format!("## Main\nLet patches be [{literal}].\nCall applyPatches with patches.\n");
    let wasm = logicaffeine_compile::compile::compile_to_wasm(&src)
        .unwrap_or_else(|e| panic!("a program carrying the buffer compiles: {e:?}"));
    let b = wasm
        .iter()
        .map(|x| x.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let js = logicaffeine_compile::web_host::BROWSER_RUNTIME_JS.replace(
        "const bytes = await (await fetch(WASM_URL)).arrayBuffer();",
        &format!("const bytes = new Uint8Array([{b}]);"),
    );
    let probe = format!(
        "{FAKE_DOM}\nconst WASM_URL='';\n{js}\nconsole.log('SURVIVED:' + document.body.children.length);\n"
    );
    let (stdout, stderr, _) = run_node(&probe);
    let all = format!("{stdout}\n{stderr}");
    (stdout, all)
}

/// A stream that places a `<p>` in the body, then `tail`.
fn a_paragraph_then(tail: &[u8]) -> Vec<u8> {
    use logicaffeine_compile::semantics::dom::DomOp;
    let mut bytes = logicaffeine_compile::semantics::patch::encode(&[
        DomOp::CreateElement {
            node: 1,
            tag: "p".into(),
        },
        DomOp::AppendChild {
            parent: 0,
            child: 1,
        },
    ]);
    bytes.extend_from_slice(tail);
    bytes
}

/// The retired paint boundary refused in the browser's decoder in the sentence the native decoders
/// use (`patch_stream_lock`), with nothing of the stream applied.
#[test]
fn the_bulk_shim_refuses_the_retired_paint_boundary_by_name() {
    if !require_tool("node") {
        return;
    }
    let bytes = a_paragraph_then(&[0]);
    let at = bytes.len() - 1;
    let (stdout, all) = shim_applying(&bytes);
    let sentence = format!(
        "patch opcode 0 (BeginPaint) at byte {at} is retired: a paint's boundary is the commit that \
         emits the stream, and no build since the retained tree writes it."
    );
    assert!(
        all.contains(&sentence),
        "the shim refuses the retired opcode in the native decoders' words. Got:\n{all}"
    );
    assert!(
        !stdout.contains("SURVIVED:1"),
        "the records before it were not applied either:\n{all}"
    );
}

/// Every retired opcode refused by the browser's decoder exactly as the native decoder refuses the
/// same stream, operands and all, with nothing of the stream applied.
#[test]
fn the_bulk_shim_refuses_every_retired_opcode_as_the_native_decoder_does() {
    if !require_tool("node") {
        return;
    }
    use logicaffeine_compile::semantics::patch;
    for &(code, _) in patch::RETIRED_OPCODES {
        let bytes = a_paragraph_then(&[code, 1, 0, 0, 0, 0, 0, 0, 0]);
        let native =
            patch::decode(&bytes).expect_err("the native decoder refuses a retired opcode");
        let (stdout, all) = shim_applying(&bytes);
        assert!(
            all.contains(&native),
            "the shim refuses opcode {code} as the native decoder does ({native}). Got:\n{all}"
        );
        assert!(
            !stdout.contains("SURVIVED:1"),
            "opcode {code}: the records before it were not applied either:\n{all}"
        );
    }
}

fn checked_callback_runtime(source: &str, setup: &str, probe: &str) {
    if !require_tool("node") {
        return;
    }
    let wasm = logicaffeine_compile::compile::compile_to_wasm_checked(source)
        .unwrap_or_else(|e| panic!("callback program must compile: {e:?}"));
    let bytes = wasm
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let js = logicaffeine_compile::web_host::BROWSER_RUNTIME_JS.replace(
        "const bytes = await (await fetch(WASM_URL)).arrayBuffer();",
        &format!("const bytes = new Uint8Array([{bytes}]);"),
    );
    let script = format!("{FAKE_DOM}\nObject.defineProperty(N.prototype, 'childNodes', {{ get() {{ return this.children; }} }});\n{setup}\nconst WASM_URL='';\n{js}\n{probe}");
    let (stdout, stderr, ok) = run_node(&script);
    assert!(ok, "callback host contract failed:\n{stdout}\n{stderr}");
    assert!(
        stdout.lines().any(|line| line == "CALLBACK_CONTRACT_PASS"),
        "probe did not finish: {stdout}\n{stderr}"
    );
}

const IMPERATIVE_CAPTURED_CALLBACK: &str = r##"## Main
Let parent be createElement("div").
Let button be createElement("button").
Call appendChild with body() and parent.
Call appendChild with parent and button.
Let payload be [73, 74].
On click from button:
    Call setText with button and "called".
    Show item 1 of payload.
"##;

#[test]
fn freeing_subtrees_releases_imperative_callbacks_and_captured_storage() {
    checked_callback_runtime(
        IMPERATIVE_CAPTURED_CALLBACK,
        "",
        r#"
const check = (yes, why) => { if (!yes) throw new Error(why); };
let released = 0;
const actualRelease = inst.exports.__logos_release;
check(typeof actualRelease === 'function', 'module must expose callback release');
inst = { exports: { ...inst.exports, __logos_release(h) { released++; actualRelease(h); } } };
let warm = 0;
for (let turn = 1; turn <= 1000; turn++) {
  if (turn > 1) guarded(() => instance.exports.main());
  const parent = body.children[0], button = parent.children[0];
  const listener = button.listeners.click[0];
  listener({ preventDefault() {}, stopPropagation() {} });
  check(button.textContent === 'called', 'live callback did not run');
  freeSubtree(Number(nodeId(parent)));
  inst.exports.__logos_safepoint();
  check(released === turn, 'freed subtree did not release exactly one registration');
  check(button.listeners.click.length === 0, 'freed node retains its listener');
  check(inst.exports.__logos_heap_census() === 0, 'ownership census is not exact');
  check(globalThis.__logosHostCensus().nodes === 1, 'removed subtree remains in host table');
  if (turn === 100) warm = inst.exports.__logos_heap_allocated();
}
check(inst.exports.__logos_heap_allocated() === warm, 'callback churn grows retained storage');
console.log('CALLBACK_CONTRACT_PASS');
"#,
    );
}

#[test]
fn queued_imperative_events_do_not_call_a_freed_callback() {
    checked_callback_runtime(
        IMPERATIVE_CAPTURED_CALLBACK,
        "",
        r#"
let calls = 0;
const actualCall = inst.exports.__logos_call_closure;
inst = { exports: { ...inst.exports, __logos_call_closure(...args) { calls++; return actualCall(...args); } } };
const parent = body.children[0], button = parent.children[0];
const listener = button.listeners.click[0];
guarded(() => {
  listener({ preventDefault() {}, stopPropagation() {} });
  freeSubtree(Number(nodeId(parent)));
});
if (calls !== 0) throw new Error('queued event called a removed callback');
if (button.textContent !== '') throw new Error('queued event called a removed callback');
if (inst.exports.__logos_heap_census() !== 0) throw new Error('ownership census is not exact');
console.log('CALLBACK_CONTRACT_PASS');
"#,
    );
}

#[test]
fn a_callback_registration_on_a_missing_node_retains_no_storage() {
    const SOURCE: &str = r##"## Main
Let payload be [73, 74].
Let missing be querySelector("#absent").
On click from missing:
    Show item 1 of payload.
"##;
    checked_callback_runtime(
        SOURCE,
        "",
        r#"
let warm = 0;
for (let turn = 1; turn <= 1000; turn++) {
  if (turn > 1) guarded(() => instance.exports.main());
  inst.exports.__logos_safepoint();
  if (inst.exports.__logos_heap_census() !== 0) throw new Error('ownership census is not exact');
  if (turn === 100) warm = inst.exports.__logos_heap_allocated();
}
if (inst.exports.__logos_heap_allocated() !== warm) throw new Error('invalid registrations retain callback storage');
console.log('CALLBACK_CONTRACT_PASS');
"#,
    );
}
