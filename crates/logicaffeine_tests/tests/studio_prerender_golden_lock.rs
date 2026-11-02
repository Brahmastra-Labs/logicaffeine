//! The Studio's server render, byte for byte.
//!
//! The Studio page is being cut in two: the workbench (its toolbar, Files tree, editor and output
//! columns) becomes a component the front page renders too, and the page keeps its head, nav, the
//! reader's store and the address bar. That move must change nothing `/studio` serves. This holds the
//! server render of a bare `/studio`, of a deep link in every mode and of a reader's own file to the
//! document recorded in `goldens/studio_prerender.txt`, by length and a stable FNV-1a hash. Each run
//! writes what it rendered under the test's target tmp dir (`studio-prerender/`), so a red names the
//! file to diff against an earlier run's.
//!
//! A deliberate change to the Studio's render rerecords the golden: `LOGOS_REGEN=1 cargo nextest run
//! -p logicaffeine-tests --features web-tests --test studio_prerender_golden_lock`.

#![cfg(feature = "web-tests")]

use dioxus::prelude::*;
use dioxus_history::{History, MemoryHistory};
use logicaffeine_web::App;
use std::path::PathBuf;
use std::rc::Rc;

/// Every route the lock renders, with the name its rendered copy is written under.
const ROUTES: &[(&str, &str)] = &[
    ("studio", "/studio"),
    ("logic-sentences", "/studio?file=examples/logic/simple-sentences.logic"),
    ("logic-theorem", "/studio?file=examples/logic/syllogism.logic"),
    ("code", "/studio?file=examples/code/hello-world.logos"),
    ("math", "/studio?file=examples/math/auto-tactic.logos"),
    ("hardware-spec", "/studio?file=examples/hardware/handshake.hw"),
    ("hardware-verilog", "/studio?file=examples/hardware/arbiter.v"),
    ("web", "/studio?file=examples/code/web/counter.logos"),
    ("readers-file", "/studio?file=workspace/untitled.logos"),
];

fn render_route(path: &str) -> String {
    let mut vdom = VirtualDom::new(App);
    vdom.provide_root_context(Rc::new(MemoryHistory::with_initial_path(path)) as Rc<dyn History>);
    vdom.rebuild_in_place();
    without_editor_numbers(&dioxus_ssr::render(&vdom))
}

/// `html` with each code editor's number taken out of its element id. Every editor mints
/// `code-editor-<n>` from one counter for the whole process, so the number says how many editors the
/// process drew before it, not what this page is.
fn without_editor_numbers(html: &str) -> String {
    const ID: &str = "code-editor-";
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find(ID) {
        let after = &rest[at + ID.len()..];
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        out.push_str(&rest[..at + ID.len()]);
        if digits > 0 {
            out.push('#');
        }
        rest = &after[digits..];
    }
    out.push_str(rest);
    out
}

#[test]
fn an_editors_number_is_taken_out_of_its_id_and_nothing_else() {
    assert_eq!(without_editor_numbers(r#"<textarea id="code-editor-12">"#), r#"<textarea id="code-editor-#">"#);
    assert_eq!(without_editor_numbers(".code-editor-textarea { }"), ".code-editor-textarea { }");
    assert_eq!(without_editor_numbers("code-editor-3 code-editor-45x"), "code-editor-# code-editor-#x");
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn golden_path() -> PathBuf {
    repo_root().join("crates/logicaffeine_tests/tests/goldens/studio_prerender.txt")
}

/// Where each run leaves the documents it rendered.
fn rendered_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("studio-prerender")
}

/// FNV-1a over 64 bits: stable across Rust releases, unlike the standard library's hasher.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3))
}

#[test]
fn the_hash_is_fnv1a() {
    assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(fnv1a(b"foobar"), 0x8594_4171_f739_67e8);
}

/// The lock means something only if one route renders the same document every time.
#[test]
fn a_route_renders_the_same_document_twice() {
    let unstable: Vec<String> = ROUTES
        .iter()
        .filter_map(|(name, path)| {
            let (first, second) = (render_route(path), render_route(path));
            let at = first.bytes().zip(second.bytes()).position(|(a, b)| a != b).or((first.len() != second.len()).then(|| first.len().min(second.len())))?;
            let around = |doc: &str| doc.get(at.saturating_sub(120)..(at + 120).min(doc.len())).unwrap_or("").to_string();
            Some(format!("{name} ({path}) parts at byte {at}:\n  first  {:?}\n  second {:?}", around(&first), around(&second)))
        })
        .collect();
    assert!(unstable.is_empty(), "a route renders a different document each time:\n{}", unstable.join("\n"));
}

#[test]
fn the_studio_renders_its_recorded_documents() {
    let dir = rendered_dir();
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("create {}: {e}", dir.display()));
    let mut lines = Vec::new();
    for (name, path) in ROUTES {
        let html = render_route(path);
        assert!(html.contains("studio-toolbar"), "{path} did not render the Studio, so its golden would prove nothing");
        std::fs::write(dir.join(format!("{name}.html")), &html).unwrap();
        lines.push(format!("{:016x} {} {name} {path}", fnv1a(html.as_bytes()), html.len()));
    }
    let now = lines.join("\n") + "\n";
    if std::env::var_os("LOGOS_REGEN").is_some() {
        std::fs::write(golden_path(), &now).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(golden_path()).unwrap_or_else(|e| panic!("read the golden ({e}); record it with LOGOS_REGEN=1"));
    let changed: Vec<String> = golden
        .lines()
        .zip(now.lines())
        .filter(|(was, is)| was != is)
        .map(|(was, is)| format!("  was {was}\n  now {is}"))
        .collect();
    assert!(
        changed.is_empty() && golden == now,
        "the Studio renders different documents (this run's copies are in {}):\n{}\n(golden {} lines, now {})",
        dir.display(),
        changed.join("\n"),
        golden.lines().count(),
        now.lines().count()
    );
}
