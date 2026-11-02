//! ★ LOCK — an element's event reaches a page through ONE path, on every tier. ★
//!
//! A handler written in a view (`on-click={…}`, `bind={…}`, a callback prop, a `When "<sel>" is …:`
//! sentence, a page key) is DATA: the paint writes an entry into the page's listener table, keyed
//! by the element and the event, and tells the host only that the element listens there and how
//! (`listenFlags(el, event, flags)`, the `.prevent`/`.stop`/`.capture`/`.passive` bits). Every
//! event the host hears on that element goes to `__lgx_dispatch_node(node, event, value)`, the one
//! entry that reads the table — the same arm a driving verb in a `## Test` reaches.
//!
//! So nothing about an element's event is a closure: the lowering attaches none, the browser host
//! keeps no handler table of its own, and a browser's click enters the module through the node
//! dispatcher and nothing else.

#[path = "cli_common/mod.rs"]
mod common;

use logicaffeine_cli::browser::{self, Browser, CallKind, PageBundle};
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom::{self, DomOp};
use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};
use logicaffeine_compile::vm::wasm::runtime_export_names;

/// One component per way a view registers for an element's event: a plain handler, `.prevent`,
/// a keyed `.stop`, `.once`, a bound text input, a bound checkbox, a callback prop handed to a
/// keyed row, a selector sentence and the page's key sentence.
const EVERY_REGISTRATION: &str = r##"## Component Row (label: Text) and (remove: fn(Text) -> Unit)

View:
    <li class="row"><button id="remove-{label}" on-click={remove(label)}>x</button></li>

## Component App

State:
    Let n be 0.
    Let draft be "".
    Let done be false.
    Let names be ["ada", "grace"].

Actions:
    To hit:
        Set n to n + 1.
    To drop (who: Text):
        Let mutable kept be a new Seq of Text.
        Repeat for x in names:
            If x is not equal to who:
                Push x to kept.
        Set names to kept.
    When "#far" is clicked:
        Set n to n + 100.
    When the user presses "Escape":
        Set n to n + 1000.

View:
    <p id="n">{n}</p>
    <p id="echo">{draft}</p>
    <p id="ticked">{done}</p>
    <button id="plain" on-click={hit}>plain</button>
    <form id="f" on-submit.prevent={hit}><button>go</button></form>
    <input id="k" aria-label="k" on-keydown.enter.stop={hit}>
    <button id="once" on-click.once={hit}>once</button>
    <input id="draft" aria-label="draft" bind={draft}>
    <input id="done" type="checkbox" aria-label="done" bind={done}>
    <button id="far">far</button>
    <ul>{for each x in names, keyed by x:}<Row label={x} remove={drop}/>{end}</ul>
"##;

fn mounted() -> String {
    format!("{EVERY_REGISTRATION}\n## Main\nMount App.\n")
}

/// Every `(element, event, flags)` the mounted page registers, the element named by selector.
const REGISTERED: &[(&str, &str, u8)] = &[
    ("#plain", "click", 0),
    ("#f", "submit", 1),
    ("#k", "keydown", 2),
    ("#once", "click", 0),
    ("#draft", "input", 0),
    ("#done", "input", 0),
    ("#remove-ada", "click", 0),
    ("#remove-grace", "click", 0),
    ("#far", "click", 0),
    ("body", "keydown", 0),
];

/// The registrations the lowered VM's mount told the host about, as `(selector, event, flags)`,
/// plus every closure listener and bare `preventDefault` it made.
fn registrations_on_the_lowered_vm() -> (Vec<(String, String, u8)>, Vec<DomOp>) {
    dom::reset();
    let out = vm_outcome(&mounted());
    assert_eq!(out.error, None, "the page mounts on the lowered VM");
    let named: Vec<(i64, &str)> = REGISTERED
        .iter()
        .map(|(sel, _, _)| (dom::query_selector(sel), *sel))
        .collect();
    for (node, sel) in &named {
        assert!(*node >= 0, "`{sel}` is on the mounted page");
    }
    let name_of = |node: u32| {
        named
            .iter()
            .find(|(n, _)| *n == i64::from(node))
            .map(|(_, sel)| sel.to_string())
            .unwrap_or_else(|| format!("node {node}"))
    };
    let mut listens = Vec::new();
    let mut closures = Vec::new();
    for op in dom::op_log() {
        match op {
            DomOp::Listen { node, event, flags } => {
                let row = (name_of(node), event, flags);
                if !listens.contains(&row) {
                    listens.push(row);
                }
            }
            DomOp::AddEventListener { .. } | DomOp::PreventDefault { .. } => closures.push(op),
            _ => {}
        }
    }
    (listens, closures)
}

#[test]
fn a_mounted_page_tells_the_host_only_where_it_listens_and_how() {
    let (listens, closures) = registrations_on_the_lowered_vm();
    assert!(
        closures.is_empty(),
        "an element's event is a table entry plus `listenFlags`; the mount attached a closure or a \
         bare prevent: {closures:?}"
    );
    let mut want: Vec<(String, String, u8)> = REGISTERED
        .iter()
        .map(|(s, e, f)| (s.to_string(), e.to_string(), *f))
        .collect();
    let mut got = listens;
    want.sort();
    got.sort();
    assert_eq!(
        got, want,
        "every registration reaches the host as ONE `listenFlags` on its element, carrying the \
         `.prevent`/`.stop` bits and nothing for `.once`"
    );
}

#[test]
fn every_registration_answers_through_the_dispatcher_on_every_tier() {
    let source = format!(
        "{EVERY_REGISTRATION}
## Test: every way a view listens answers its own event

Mount App.
Click \"#plain\".
Expect that the text of \"#n\" is equal to \"1\".
Submit \"#f\".
Expect that the text of \"#n\" is equal to \"2\".
Press \"Enter\" in \"#k\".
Expect that the text of \"#n\" is equal to \"3\".
Click \"#once\".
Expect that the text of \"#n\" is equal to \"4\".
Type \"hello\" into \"#draft\".
Expect that the text of \"#echo\" is equal to \"hello\".
Click \"#done\".
Expect that the text of \"#ticked\" is equal to \"true\".
Click \"#far\".
Expect that the text of \"#n\" is equal to \"104\".
Press \"Escape\".
Expect that the text of \"#n\" is equal to \"1104\".
Click \"#remove-ada\".
Expect that the count of \".row\" is equal to 1.
Expect that the text of \".row\" is equal to \"x\".
Click \"#remove-grace\".
Expect that the count of \".row\" is equal to 0.
"
    );
    let report = run_tests(&source, &TestConfig::default())
        .unwrap_or_else(|e| panic!("the fixture parses and runs: {e}\n{source}"));
    assert_eq!(report.outcomes.len(), 1, "one ## Test");
    for outcome in &report.outcomes {
        assert_eq!(
            outcome.verdict,
            Verdict::Passed,
            "`{}` passes on the reference tier and traces identically on the lowered VM: {:?}",
            outcome.name,
            outcome.failures
        );
    }
}

fn repo_file(rel: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The lowering's source, file by file.
fn lowering_sources() -> Vec<(String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../logicaffeine_language/src/parser/lgx_lower");
    let mut files: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension()? == "rs").then(|| {
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                (
                    name,
                    std::fs::read_to_string(&path).expect("read a lowering file"),
                )
            })
        })
        .collect();
    files.sort();
    files
}

/// Every `addEventListener` the lowering emits, as `file:line`.
fn closure_listener_sites(files: &[(String, String)]) -> Vec<String> {
    let mut sites = Vec::new();
    for (file, source) in files {
        for (i, line) in source.lines().enumerate() {
            if line.contains("intern(\"addEventListener\")") {
                sites.push(format!("{file}:{}", i + 1));
            }
        }
    }
    sites
}

#[test]
fn the_lowering_attaches_no_closure_to_an_element() {
    let sites = closure_listener_sites(&lowering_sources());
    assert!(
        sites.is_empty(),
        "an element's event — a drop's file and a scroll's geometry included — is registered with a \
         `Listen` and answered by the dispatcher; these sites attach a closure: {sites:?}"
    );
}

#[test]
fn the_closure_census_fails_on_an_element_handler_that_attaches_a_closure() {
    let planted = vec![(
        "render.rs".to_string(),
        "                ViewNode::Handler { event, .. } => {\n\
         \x20                   let listen = self.interner.intern(\"addEventListener\");\n"
            .to_string(),
    )];
    assert_eq!(
        closure_listener_sites(&planted),
        ["render.rs:2"],
        "the census sees a stray closure"
    );
}

/// The page-listener section of the browser host, and the names a second handler table would use.
const RETIRED_HOST_NAMES: &[&str] = &[
    "managedHandlers",
    "bindEvent",
    "unbindEvent",
    "releaseStaged",
];

fn dispatch_calls(runtime: &str) -> Vec<usize> {
    runtime
        .match_indices(".__lgx_dispatch_node(")
        .map(|(at, _)| at)
        .collect()
}

#[test]
fn the_browser_host_enters_the_page_for_an_element_event_in_one_place() {
    let runtime = repo_file("crates/logicaffeine_compile/assets/web/runtime.js");
    for name in RETIRED_HOST_NAMES {
        assert!(
            !runtime.contains(name),
            "`{name}` keeps an element's handler in the host; the page's listener table is the one \
             place a handler lives"
        );
    }
    let calls = dispatch_calls(&runtime);
    assert_eq!(
        calls.len(),
        1,
        "the host calls `__lgx_dispatch_node` from exactly one place"
    );
    let before = &runtime[..calls[0]];
    let owner = before
        .rfind("\nconst ")
        .map(|at| &before[at + 1..])
        .unwrap_or("");
    assert!(
        owner.starts_with("const dispatchNode = "),
        "the one call is `dispatchNode`'s, which every page listener hands its event to"
    );
    let body_of = |name: &str| {
        let at = runtime
            .find(&format!("const {name} = "))
            .unwrap_or_else(|| panic!("runtime.js defines `{name}`"));
        let body = &runtime[at..];
        &body[..body.find("\n};").unwrap_or_else(|| panic!("`{name}` ends"))]
    };
    assert!(
        body_of("pageListen").contains("hearAtNode("),
        "the one DOM listener `pageListen` attaches hands its event to `hearAtNode`"
    );
    assert!(
        body_of("hearAtNode").contains("dispatchNode("),
        "`hearAtNode` enters the page through `dispatchNode`"
    );
    let entry = runtime
        .find("dom_page_entry:")
        .expect("runtime.js takes a program's own driving verb");
    let entry = &runtime[entry
        ..entry
            + runtime[entry..]
                .find("\n  },")
                .expect("`dom_page_entry` ends")];
    assert!(
        entry.contains("dispatchNode("),
        "a program's own driving verb enters the page through `dispatchNode` too"
    );
}

#[test]
fn the_host_census_fails_on_a_second_handler_table() {
    let planted = "const managedHandlers = new Map();\nconst dispatchNode = (el) => { inst.exports.__lgx_dispatch_node(1n); };\nconst other = () => inst.exports.__lgx_dispatch_node(2n);\n";
    assert!(
        RETIRED_HOST_NAMES.iter().any(|n| planted.contains(n)),
        "the planted table is named"
    );
    assert_eq!(
        dispatch_calls(planted).len(),
        2,
        "the census counts every entry into the dispatcher"
    );
}

/// The browser this binary shares, or `None` (a recorded skip) on a machine without Chrome.
fn chrome() -> Option<&'static Browser> {
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

/// The reference tier's document after `steps` run against the page.
fn reference(steps: &[&str]) -> String {
    dom::reset();
    let program = format!(
        "{EVERY_REGISTRATION}\n## Main\nMount App.\n{}\n",
        steps.join("\n")
    );
    let out = tw_outcome(&program);
    assert_eq!(
        out.error, None,
        "the reference tier runs the page:\n{program}"
    );
    dom::render()
}

#[test]
fn a_browser_event_enters_the_page_only_through_the_node_dispatcher() {
    let Some(b) = chrome() else { return };
    let bundle = PageBundle::page(&mounted()).unwrap_or_else(|e| panic!("the page bundles: {e}"));
    let p = b
        .open(&bundle)
        .unwrap_or_else(|e| panic!("the page opens: {e}"));
    // What the host calls to hand an argument over (`__logos_text_new` for the event's name and
    // value) is the module's runtime, not an entry into the page; the closure trampoline is the one
    // runtime export that runs page code, so it is never marshalling.
    let runtime = runtime_export_names();
    let marshalling =
        |name: &str| name != "__logos_call_closure" && runtime.iter().any(|r| *r == name);
    let entries_during = |act: &dyn Fn()| -> Vec<String> {
        let from = p.trace_len().unwrap_or_else(|e| panic!("trace: {e}"));
        act();
        let trace = p.import_trace().unwrap_or_else(|e| panic!("trace: {e}"));
        trace[from..]
            .iter()
            .filter(|c| c.kind == CallKind::Export && c.depth == 0)
            .map(|c| c.name.clone())
            .filter(|name| !marshalling(name))
            .collect()
    };
    let acts: Vec<(&str, Box<dyn Fn()>)> = vec![
        ("click #plain", Box::new(|| p.click("#plain").unwrap())),
        ("submit #f", Box::new(|| p.submit("#f").unwrap())),
        (
            "Enter in #k",
            Box::new(|| p.press_in("#k", "Enter").unwrap()),
        ),
        (
            "type into #draft",
            Box::new(|| p.type_into("#draft", "hi").unwrap()),
        ),
        ("click #done", Box::new(|| p.click("#done").unwrap())),
        ("click #far", Box::new(|| p.click("#far").unwrap())),
        (
            "click #remove-ada",
            Box::new(|| p.click("#remove-ada").unwrap()),
        ),
    ];
    for (what, act) in &acts {
        let entered = entries_during(act.as_ref());
        assert!(
            !entered.is_empty() && entered.iter().all(|name| name == "__lgx_dispatch_node"),
            "{what}: the browser enters the page through `__lgx_dispatch_node` and nothing else, \
             entered {entered:?}"
        );
    }
    let snapshot = p
        .snapshot()
        .unwrap_or_else(|e| panic!("snapshot: {e}"))
        .body_markup();
    let expected = reference(&[
        "Click \"#plain\".",
        "Submit \"#f\".",
        "Press \"Enter\" in \"#k\".",
        "Type \"hi\" into \"#draft\".",
        "Click \"#done\".",
        "Click \"#far\".",
        "Click \"#remove-ada\".",
    ]);
    assert_eq!(
        snapshot, expected,
        "the browser paints the document the reference tier paints"
    );
    assert!(
        expected.contains("<p id=\"n\">103</p>"),
        "the oracle counted every act: {expected}"
    );
    assert!(
        p.exceptions().is_empty(),
        "the page does not throw: {:?}",
        p.exceptions()
    );
}
