//! C4 — the pure-LOGOS full-stack CROWN LOCK.
//!
//! `examples/fullstack_todo/` is a real todo web app whose backend
//! (`src/server.lg`) and frontend (`pages/app.lg`) are BOTH written in LOGOS.
//! This lock proves the two `.lg` files INTEROPERATE — the frontend's fetches
//! reach the backend's handler and the backend's responses drive the frontend's
//! DOM — entirely IN-PROCESS: no sockets, no live database, deterministic.
//!
//! The mechanism, end to end:
//!
//!   1. `pages/app.lg` is compiled to WASM (`compile_to_wasm`) and instantiated
//!      under a mock DOM — the same wasmi(MockDom) host model `wasm_dom.rs`
//!      proves the DOM tier against. `main` runs the initial mount, which fires
//!      the component's boot fetch (`GET /api/todos`).
//!   2. For every fetch the frontend issues, we read the request (method, url,
//!      body) straight off the mock's correlated fetch engine and run it through
//!      the ACTUAL backend on the TREE-WALKER: a genuine `Request` is synthesized
//!      and `server.lg`'s real `handle(request) -> Response` is called, so the
//!      routing, the response construction and the file persistence are the ones
//!      a deployed `Serve` wires — not a Rust mirror of them.
//!   3. The handler's response body + status are delivered back into the mock
//!      fetch by id, firing the frontend's `load` completion closure, which
//!      writes the `todos` signal. We then settle (`__logos_render`) exactly as
//!      the browser host does on a reactive write.
//!   4. We assert the rendered DOM op-log shows the todos — after the initial
//!      load, after an ADD, and after a DELETE.
//!
//! Two tests, two depths, and it is worth knowing which is which. The headline
//! test drives the REAL `handle` against a temp-file store. The companion test
//! (`the_backend_wire_format_is_what_the_frontend_parses`) drives `server.lg`'s
//! pure store transforms (`addToStore` / `removeFromStore` / `listBody`) over an
//! in-memory string, which is the narrower, faster check that the wire text the
//! backend builds is the wire text the frontend parses.
//!
//! Why a file store and not Postgres here? To stay HERMETIC and DETERMINISTIC.
//! The Postgres path (`pgHandle*` in `server.lg`) is proven separately, LIVE, by
//! `postgres_live.rs` against a real PostgreSQL 16 — and it is the SAME `handle`,
//! taking its other arm. This lock owns the CONTRACT between the two `.lg` files;
//! that lock owns the durable store.
//!
//! YOU DO NOT GET TO WEAKEN THIS LOCK. A divergence means the frontend and
//! backend stopped speaking the same wire language — fix the `.lg` files, never
//! the assert.
#![cfg(feature = "wasm-jit")]

use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome};
use logicaffeine_compile::semantics::dom::{self, DomOp};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

#[path = "compile_common/mod.rs"]
mod common;

/// ★ THE CLOSURE, NOT THE FILE. Both halves of the app say `Import Shared.`, and the one `## A
/// Todo has:` they agree on lives in `src/shared.lg` — which is the entire point of the example.
/// Reading either file with `include_str!` compiles a FRAGMENT: the record is undefined, so the
/// routes answering `Seq of Todo` are correctly refused and the page cannot reach a field of a row.
///
/// It passed that way only because an `Import` after a paragraph of prose used to be silently
/// dropped by the dependency scanner — so the fragment was, by accident, the whole program. Once
/// imports actually resolved, the lock was compiling something no `largo` command ever compiles.
///
/// `assemble_project` is the SAME seam `largo build`/`run`/`test`/`serve` consume, so this file now
/// stands on the artifact those commands produce. Every assertion below is untouched.
fn closure_of(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("the crate sits two levels below the repo root")
        .join("examples/fullstack_todo")
        .join(rel);
    logicaffeine_compile::analysis::assemble_project(&path)
        .unwrap_or_else(|e| panic!("{rel} assembles: {e}"))
        .source
}

fn server_lg() -> String {
    closure_of("src/server.lg")
}

fn app_lg() -> String {
    closure_of("pages/app.lg")
}

// ─────────────────────────────────────────────────────────────────────────────
// The backend oracle — the REAL server.lg store logic, on the tree-walker.
// ─────────────────────────────────────────────────────────────────────────────

/// `server.lg` with its `## Main` (the `Serve` line) stripped, so the store
/// transforms can be driven directly.
fn server_lib() -> String {
    let idx = server_lg()
        .find("\n## Main")
        .expect("server.lg has a ## Main");
    server_lg()[..idx].to_string()
}

/// Run a driver `## Main` against the server library on the tree-walker and
/// return its single line of `Show` output. The driver calls the genuine
/// `server.lg` functions — this is the backend executing, not a re-implementation.
fn run_server(driver_main: &str) -> String {
    let src = format!("{}\n## Main\n{}", server_lib(), driver_main);
    let out = tw_outcome(&src);
    assert!(
        out.error.is_none(),
        "backend oracle errored:\n{driver_main}\n\nerror: {:?}",
        out.error
    );
    out.output.trim_end_matches('\n').to_string()
}

// ─────────────────────────────────────────────────────────────────────────────
// The REAL backend: `server.lg`'s own `## Route` table, driven through the
// SYNTHESIZED DISPATCHER (`routeWith` / `routeStatusWith`) on the tree-walker.
//
// ★ IT DRIVES THE ENDPOINT, NOT THE INTERNALS. The oracle used to call hand-named
// store helpers (`listBody`, `addToStore`, `removeFromStore`) and compare an
// `id|text` wire format. Those functions no longer exist: a `## Route` block now
// says which method and path it answers and what it returns, the router is
// synthesized from the table, and the wire is JSON DERIVED from `## A Todo has:`.
// Driving `route(method, url)` is strictly stronger — it exercises the matching,
// the status, the content type and the derived encoder that a deployed server
// runs, rather than three functions a test happened to know the names of.
//
// State lives in the store FILE exactly as it does under `Serve`, so each call is
// a fresh stateless dispatch and persistence is really exercised.
// ─────────────────────────────────────────────────────────────────────────────

/// Serializes the crown tests' use of the shared default store file WITHIN one process — which
/// is the half a mutex can do. The other half is [`RealBackend::new`]'s per-process directory.
static STORE_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct RealBackend {
    store_path: PathBuf,
    /// The private directory this backend's store lives in. Held so it outlives the store and is
    /// removed with it; never read.
    _dir: tempfile::TempDir,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl RealBackend {
    fn new() -> Self {
        // `server.lg`'s `storePath()` answers `todos.json`, RELATIVE to the process's working
        // directory. (`tw_outcome` seals the environment, so `get()` answers nothing and the
        // default is the only path there is — the env var cannot be used to point it elsewhere.)
        //
        // ★ A MUTEX SERIALIZED NOTHING HERE, AND THE TEST WAS FLAKY BY CONSTRUCTION. nextest runs
        // each test in its OWN PROCESS, so `STORE_GUARD` — a `static` inside one process — could
        // not order the two tests that share this file. They raced on one relative path and each
        // read rows the other had written: this test would fail with an extra `"milk"` row it
        // never added, while its sibling passed, and on the next run the pair swapped. It cost a
        // false red on every full-suite run, which is worse than a failure that means something,
        // because a red nobody believes is a red nobody reads.
        //
        // The fix is ISOLATION rather than ORDERING: give each process its own working directory,
        // so "relative to the working directory" resolves somewhere no other process can reach.
        // The sources are loaded through `CARGO_MANIFEST_DIR`, an absolute path, so moving the
        // working directory cannot disturb them. The mutex STAYS for the same-process runner
        // (`cargo test` uses threads, where `set_current_dir` is global and ordering is what
        // protects it) — the two together are correct under both.
        let guard = STORE_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("a private store directory");
        std::env::set_current_dir(dir.path()).expect("move into the private store directory");
        let store_path = PathBuf::from("todos.json");
        let _ = std::fs::remove_file(&store_path);
        RealBackend {
            store_path,
            _dir: dir,
            _guard: guard,
        }
    }

    /// The current persisted store text — what the real `fileHandle*` handlers
    /// wrote to disk. The observation point for "what the backend actually holds".
    fn store_text(&self) -> String {
        std::fs::read_to_string(&self.store_path).unwrap_or_default()
    }

    /// Dispatch one request through `server.lg`'s OWN router and return `(status, body)`.
    ///
    /// `routeStatusWith` and `routeWith` are the two halves the route lowering builds from the
    /// SAME walk over the `## Route` table, so a route can never be added to one and forgotten in
    /// the other. Each call is a fresh stateless dispatch; state survives through the store file,
    /// exactly as it does under `Serve on port 8080.`.
    ///
    /// ★ THE STATUS IS READ FIRST, AND ON ITS OWN LINE. The body is JSON and contains every
    /// character a separator might use, so splitting one interpolated string on a delimiter — what
    /// this did while the wire was `id|text` — would truncate the answer at the first brace or
    /// comma somebody's todo text happened to contain.
    fn handle(&mut self, method: &str, path: &str, body: &str) -> (i64, String) {
        let driver = format!(
            "Show routeStatusWith({m}, {p}, {b}).\nShow routeWith({m}, {p}, {b}).",
            m = lg_str(method),
            p = lg_str(path),
            b = lg_str(body),
        );
        let src = format!("{}\n## Main\n{driver}", server_lib());
        let out = tw_outcome(&src);
        assert!(
            out.error.is_none(),
            "the REAL router errored for {method} {path}:\n{driver}\n\nerror: {:?}",
            out.error
        );
        let text = out.output.trim_end_matches('\n');
        let (status_s, resp_body) = text.split_once('\n').unwrap_or((text, ""));
        (status_s.trim().parse().unwrap_or(0), resp_body.to_string())
    }
}

impl Drop for RealBackend {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.store_path);
    }
}

/// Quote `s` as a LOGOS string literal.
///
/// ★ A BRACE OPENS AN INTERPOLATION HOLE, and that is why this doubles it. The wire is JSON now,
/// so every payload this feeds into a driver program starts with `{` — and a bare one makes the
/// rest of the document read as an expression, which fails as an undefined variable somewhere far
/// from the string that caused it. `{{` is the escape; `\`, `"` and newlines are the ordinary ones.
fn lg_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '{' => out.push_str("{{"),
            '}' => out.push_str("}}"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// The mock DOM + correlated fetch engine (the frontend's runtime host).
// A focused mirror of `wasm_dom.rs`'s private harness — only the hosts the todo
// page uses. Node 0 is `body`; ids mint node handles in creation order.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
struct FetchReq {
    method: String,
    url: String,
    body: String,
    closure: i32,
    done: bool,
    status: i64,
    resp_body: String,
}

#[derive(Default)]
struct MockDom {
    node_count: u32,
    log: Vec<DomOp>,
    /// Every `(element, event)` the current render registered through `listenFlags` — where the
    /// page listens; what answers is the page's own listener table, reached through the node
    /// dispatcher.
    listens: Vec<(u32, String)>,
    fetch_reqs: Vec<FetchReq>,
    current_fetch: Option<usize>,
    text_bump: usize,
    signal_values: Vec<i64>,
    signal_slots: std::collections::HashMap<(String, usize), i64>,
    signal_scopes: Vec<String>,
    signal_cursor: std::collections::HashMap<String, usize>,
    signal_dirty: bool,
    ids: Vec<(String, u32)>,
    tags: Vec<(u32, String)>,
    /// `node → class` for every element carrying a `class` attribute — lets the
    /// driver click the delete buttons, which are keyed by class, not id.
    classes: Vec<(u32, String)>,
    /// `parent → children`, from every `appendChild`, so a `clearChildren` forgets exactly the
    /// subtree it took off the page — the ids, classes, tags and listeners of nodes that are gone.
    children: std::collections::HashMap<u32, Vec<u32>>,
}

impl MockDom {
    fn new() -> Self {
        MockDom {
            node_count: 1,
            text_bump: 2_000_000,
            signal_scopes: vec![String::new()],
            ..MockDom::default()
        }
    }
    fn fresh(&mut self) -> u32 {
        let h = self.node_count;
        self.node_count += 1;
        h
    }
    fn query_selector(&self, sel: &str) -> i64 {
        if let Some(id) = sel.strip_prefix('#') {
            return self
                .ids
                .iter()
                .find(|(i, _)| i == id)
                .map(|(_, n)| *n as i64)
                .unwrap_or(-1);
        }
        self.tags
            .iter()
            .find(|(_, t)| t == sel)
            .map(|(n, _)| *n as i64)
            .unwrap_or(-1)
    }
    fn listens_for(&self, node: u32, event: &str) -> bool {
        self.listens.iter().any(|(n, e)| *n == node && e == event)
    }
    /// Takes `node` out of whichever parent lists it.
    fn detach(&mut self, node: u32) {
        for kids in self.children.values_mut() {
            kids.retain(|&k| k != node);
        }
    }
    /// One op a paint's stream carried, logged and remembered where the driver looks for it — the
    /// tag, id and class it finds an element by, where it hangs, where the page listens — exactly
    /// as the per-op import that op stands for would have. A freed node leaves with its subtree.
    fn record(&mut self, op: DomOp) {
        match &op {
            DomOp::CreateElement { node, tag } => self.tags.push((*node, tag.clone())),
            DomOp::SetAttribute { node, name, value } => {
                if name == "id" {
                    self.ids.push((value.clone(), *node));
                }
                if name == "class" {
                    self.classes.push((*node, value.clone()));
                }
            }
            DomOp::AppendChild { parent, child } | DomOp::InsertBefore { parent, child, .. } => {
                self.detach(*child);
                self.children.entry(*parent).or_default().push(*child);
            }
            DomOp::RemoveNode { node } => self.detach(*node),
            DomOp::FreeNode { node } => {
                self.detach(*node);
                self.forget_subtree(*node);
                self.ids.retain(|(_, n)| n != node);
                self.classes.retain(|(n, _)| n != node);
                self.tags.retain(|(n, _)| n != node);
                self.listens.retain(|(n, _)| n != node);
            }
            DomOp::ClearChildren { node } => self.forget_subtree(*node),
            DomOp::Listen { node, event, .. } => {
                if !self.listens_for(*node, event) {
                    self.listens.push((*node, event.clone()));
                }
            }
            DomOp::Unlisten { node, event } => {
                self.listens.retain(|(n, e)| !(n == node && e == event))
            }
            _ => {}
        }
        self.log.push(op);
    }
    fn fetch_send(&mut self, method: &str, url: &str, body: &str, closure: i32) -> i64 {
        self.fetch_reqs.push(FetchReq {
            method: method.to_string(),
            url: url.to_string(),
            body: body.to_string(),
            closure,
            ..FetchReq::default()
        });
        self.fetch_reqs.len() as i64
    }
    fn fetch_req(&self, req: i64) -> Option<&FetchReq> {
        (req >= 1)
            .then(|| self.fetch_reqs.get(req as usize - 1))
            .flatten()
    }
    fn resolve_fetch(&mut self, req: i64, status: i64, body: &str) -> Option<i32> {
        let idx = usize::try_from(req).ok()?.checked_sub(1)?;
        let q = self.fetch_reqs.get_mut(idx)?;
        if q.done {
            return None;
        }
        q.done = true;
        q.status = status;
        q.resp_body = body.to_string();
        self.current_fetch = Some(idx);
        Some(self.fetch_reqs[idx].closure)
    }
    fn signal_scope(&self) -> String {
        self.signal_scopes.last().cloned().unwrap_or_default()
    }
    /// `clearChildren(node)`: every descendant of `node` leaves the page, and with it every way
    /// the driver could still find it.
    fn forget_subtree(&mut self, node: u32) {
        let mut gone = Vec::new();
        let mut stack = self.children.remove(&node).unwrap_or_default();
        while let Some(n) = stack.pop() {
            stack.extend(self.children.remove(&n).unwrap_or_default());
            gone.push(n);
        }
        self.ids.retain(|(_, n)| !gone.contains(n));
        self.classes.retain(|(n, _)| !gone.contains(n));
        self.tags.retain(|(n, _)| !gone.contains(n));
        self.listens.retain(|(n, _)| !gone.contains(n));
    }
    fn signal_begin(&mut self) {
        self.signal_scopes = vec![String::new()];
        self.signal_cursor.clear();
        // A re-render rebuilds the subtree: forget the previous render's element
        // indices + listeners so the driver clicks the CURRENT render's nodes,
        // not stale ones from a superseded render (the browser's clearChildren).
        self.listens.clear();
        self.ids.clear();
        self.classes.clear();
        self.tags.clear();
    }
    fn signal_enter(&mut self, key: &str) {
        let child = format!("{}/{}", self.signal_scope(), key);
        self.signal_cursor.insert(child.clone(), 0);
        self.signal_scopes.push(child);
    }
    fn signal_exit(&mut self) {
        if self.signal_scopes.len() > 1 {
            self.signal_scopes.pop();
        }
    }
    fn signal_new(&mut self, init: i64) -> i64 {
        let scope = self.signal_scope();
        let idx = *self.signal_cursor.entry(scope.clone()).or_insert(0);
        self.signal_cursor.insert(scope.clone(), idx + 1);
        if let Some(&h) = self.signal_slots.get(&(scope.clone(), idx)) {
            return h;
        }
        let h = self.signal_values.len() as i64;
        self.signal_values.push(init);
        self.signal_slots.insert((scope, idx), h);
        h
    }
    fn signal_get(&self, h: i64) -> i64 {
        self.signal_values.get(h as usize).copied().unwrap_or(0)
    }
    fn signal_set(&mut self, h: i64, v: i64) {
        if let Some(slot) = self.signal_values.get_mut(h as usize) {
            *slot = v;
        }
        self.signal_dirty = true;
    }
    fn take_dirty(&mut self) -> bool {
        let d = self.signal_dirty;
        self.signal_dirty = false;
        d
    }
}

type Data = Rc<RefCell<MockDom>>;

/// A `Seq of Int` as raw bytes — the patch buffer `dom_apply` receives. Length at the handle,
/// data pointer eight bytes in, 8-byte stride, the same layout the text-seq reader walks.
fn read_byte_seq(c: &mut wasmi::Caller<'_, Data>, handle: i32) -> Vec<u8> {
    let mem = c
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .expect("memory");
    let data = mem.data(&c);
    let at = handle as usize;
    let len = i32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize;
    let ptr = i32::from_le_bytes(data[at + 8..at + 12].try_into().unwrap()) as usize;
    (0..len)
        .map(|i| {
            let e = ptr + i * 8;
            i64::from_le_bytes(data[e..e + 8].try_into().unwrap()) as u8
        })
        .collect()
}

fn read_text(c: &wasmi::Caller<'_, Data>, handle: i32) -> String {
    let mem = c.get_export("memory").unwrap().into_memory().unwrap();
    let d = mem.data(c);
    let h = handle as usize;
    let len = i32::from_le_bytes(d[h..h + 4].try_into().unwrap()) as usize;
    let dp = i32::from_le_bytes(d[h + 8..h + 12].try_into().unwrap()) as usize;
    String::from_utf8_lossy(&d[dp..dp + len]).into_owned()
}

fn write_text(data: &mut [u8], bump: usize, s: &str) -> (i32, usize) {
    let b = s.as_bytes();
    let dp = bump + 16;
    data[bump..bump + 4].copy_from_slice(&(b.len() as i32).to_le_bytes());
    data[bump + 4..bump + 8].copy_from_slice(&(b.len() as i32).to_le_bytes());
    data[bump + 8..bump + 12].copy_from_slice(&(dp as i32).to_le_bytes());
    data[dp..dp + b.len()].copy_from_slice(b);
    (bump as i32, (dp + b.len() + 7) & !7)
}

fn alloc_host_text(c: &mut wasmi::Caller<'_, Data>, s: &str) -> i32 {
    let bump = c.data().borrow().text_bump;
    let mem = c.get_export("memory").unwrap().into_memory().unwrap();
    let (handle, new_bump) = {
        let data = mem.data_mut(&mut *c);
        write_text(data, bump, s)
    };
    c.data().borrow_mut().text_bump = new_bump;
    handle
}

fn fmt_into(c: &mut wasmi::Caller<'_, Data>, buf: i32, s: &str) -> i32 {
    let mem = c.get_export("memory").unwrap().into_memory().unwrap();
    let bytes = s.as_bytes();
    let data = mem.data_mut(c);
    data[buf as usize..buf as usize + bytes.len()].copy_from_slice(bytes);
    bytes.len() as i32
}

fn install_hosts(l: &mut wasmi::Linker<Data>) {
    l.func_wrap("env", "dom_body", |_c: wasmi::Caller<'_, Data>| -> i64 {
        0
    })
    .unwrap();
    // A no-op: `the journal`/`Replay` are a `## Test`'s own concept, not this fixture's, but every
    // lowered `__act_`/event/bind now imports it (R3), so any module
    // this host instantiates that has one action needs it satisfied to link at all.
    l.func_wrap(
        "env",
        "dom_note_turn",
        |_c: wasmi::Caller<'_, Data>, _name: i32, _key: i32, _changes: i32, _failure: i32| {},
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_journal_detail",
        |_c: wasmi::Caller<'_, Data>| -> i64 { 0 },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_wake_after",
        |_c: wasmi::Caller<'_, Data>, _ms: i64| {},
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_create_element",
        |c: wasmi::Caller<'_, Data>, tag: i32| -> i64 {
            let tag = read_text(&c, tag);
            let d = c.data().clone();
            let mut m = d.borrow_mut();
            let h = m.fresh();
            m.tags.push((h, tag.clone()));
            m.log.push(DomOp::CreateElement { node: h, tag });
            h as i64
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_create_text_node",
        |c: wasmi::Caller<'_, Data>, txt: i32| -> i64 {
            let text = read_text(&c, txt);
            let d = c.data().clone();
            let mut m = d.borrow_mut();
            let h = m.fresh();
            m.log.push(DomOp::CreateTextNode { node: h, text });
            h as i64
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_set_text",
        |c: wasmi::Caller<'_, Data>, el: i64, txt: i32| {
            let text = read_text(&c, txt);
            c.data().borrow_mut().log.push(DomOp::SetText {
                node: el as u32,
                text,
            });
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_set_html",
        |c: wasmi::Caller<'_, Data>, el: i64, html: i32| {
            let html = read_text(&c, html);
            c.data().borrow_mut().log.push(DomOp::SetHtml {
                node: el as u32,
                html,
            });
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_clear_children",
        |c: wasmi::Caller<'_, Data>, el: i64| {
            let mut m = c.data().borrow_mut();
            m.forget_subtree(el as u32);
            m.log.push(DomOp::ClearChildren { node: el as u32 });
        },
    )
    .unwrap();
    // `on-<event>.prevent` — the modifier that stops a handled form ALSO posting itself natively.
    // A host that does not define it does not merely lose the modifier: the module fails to
    // INSTANTIATE, so the page never loads at all.
    l.func_wrap(
        "env",
        "dom_prevent_default",
        |_c: wasmi::Caller<'_, Data>, _el: i64, _ev: i32| {},
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_set_attribute",
        |c: wasmi::Caller<'_, Data>, el: i64, name: i32, val: i32| {
            let name = read_text(&c, name);
            let value = read_text(&c, val);
            let mut m = c.data().borrow_mut();
            if name == "id" {
                m.ids.push((value.clone(), el as u32));
            }
            if name == "class" {
                m.classes.push((el as u32, value.clone()));
            }
            m.log.push(DomOp::SetAttribute {
                node: el as u32,
                name,
                value,
            });
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_append_child",
        |c: wasmi::Caller<'_, Data>, parent: i64, child: i64| {
            let mut m = c.data().borrow_mut();
            m.children
                .entry(parent as u32)
                .or_default()
                .push(child as u32);
            m.log.push(DomOp::AppendChild {
                parent: parent as u32,
                child: child as u32,
            });
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_set_value",
        |c: wasmi::Caller<'_, Data>, el: i64, txt: i32| {
            let value = read_text(&c, txt);
            c.data().borrow_mut().log.push(DomOp::SetValue {
                node: el as u32,
                value,
            });
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_apply",
        |mut c: wasmi::Caller<'_, Data>, patches: i32| {
            // The MockDom is a shim too: it decodes the stream with the SAME codec the native
            // recorder uses and records the ops, so a page driven through the bulk call and a page
            // driven through per-op calls are comparable op-log for op-log.
            let bytes = read_byte_seq(&mut c, patches);
            let ops = logicaffeine_compile::semantics::patch::decode(&bytes)
                .unwrap_or_else(|e| panic!("the module emitted an undecodable patch stream: {e}"));
            let mut m = c.data().borrow_mut();
            for op in ops {
                m.record(op);
            }
        },
    )
    .unwrap();
    // A paint's wire-v2 stream, decoded by the shared reader and recorded op by op.
    l.func_wrap(
        "env",
        "dom_apply_words",
        |c: wasmi::Caller<'_, Data>,
         words: i32,
         word_count: i64,
         strings: i32,
         string_count: i64|
         -> i64 {
            let ops = crate::common::wasm_host::applied_words(
                &c,
                words,
                word_count,
                strings,
                string_count,
            );
            let mut m = c.data().borrow_mut();
            for op in ops {
                m.record(op);
            }
            0
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_set_checked",
        |c: wasmi::Caller<'_, Data>, el: i64, on: i64| {
            c.data().borrow_mut().log.push(DomOp::SetChecked {
                node: el as u32,
                checked: on != 0,
            });
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_query_selector",
        |c: wasmi::Caller<'_, Data>, sel: i32| -> i64 {
            let sel = read_text(&c, sel);
            c.data().borrow().query_selector(&sel)
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_get_text",
        |_c: wasmi::Caller<'_, Data>, _el: i64| -> i32 { 0 },
    )
    .unwrap();
    // This mock keeps no attributes: a mount point reads as unmarked, as on a document no build
    // pre-rendered.
    l.func_wrap(
        "env",
        "dom_get_attribute",
        |mut c: wasmi::Caller<'_, Data>, _el: i64, _name: i32| -> i32 {
            alloc_host_text(&mut c, "")
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_get_value",
        |_c: wasmi::Caller<'_, Data>, _el: i64| -> i32 { 0 },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_listen_flags",
        |c: wasmi::Caller<'_, Data>, el: i64, event: i32, flags: i64| {
            let event = read_text(&c, event);
            let mut d = c.data().borrow_mut();
            if !d.listens_for(el as u32, &event) {
                d.listens.push((el as u32, event.clone()));
            }
            d.log.push(DomOp::Listen {
                node: el as u32,
                event,
                flags: flags as u8,
            });
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_event_target",
        |_c: wasmi::Caller<'_, Data>, _ev: i64| -> i64 { -1 },
    )
    .unwrap();

    // The correlated fetch engine — the frontend's `fetchSend` rides this.
    l.func_wrap(
        "env",
        "dom_fetch_send",
        |c: wasmi::Caller<'_, Data>,
         method: i32,
         url: i32,
         body: i32,
         _headers: i32,
         handler: i32|
         -> i64 {
            let method = read_text(&c, method);
            let url = read_text(&c, url);
            let body = read_text(&c, body);
            c.data()
                .borrow_mut()
                .fetch_send(&method, &url, &body, handler)
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_fetch_start",
        |c: wasmi::Caller<'_, Data>, url: i32, handler: i32| {
            let url = read_text(&c, url);
            c.data().borrow_mut().fetch_send("GET", &url, "", handler);
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_fetch_abort",
        |_c: wasmi::Caller<'_, Data>, _req: i64| {},
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_fetch_set_timeout",
        |_c: wasmi::Caller<'_, Data>, _req: i64, _millis: i64| {},
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_response_status",
        |c: wasmi::Caller<'_, Data>, req: i64| -> i64 {
            c.data()
                .borrow()
                .fetch_req(req)
                .map(|q| q.status)
                .unwrap_or(0)
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_response_body",
        |mut c: wasmi::Caller<'_, Data>, req: i64| -> i32 {
            let body = c
                .data()
                .borrow()
                .fetch_req(req)
                .map(|q| q.resp_body.clone())
                .unwrap_or_default();
            alloc_host_text(&mut c, &body)
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_response_header",
        |mut c: wasmi::Caller<'_, Data>, _req: i64, _name: i32| -> i32 {
            alloc_host_text(&mut c, "")
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_response_error",
        |mut c: wasmi::Caller<'_, Data>, _req: i64| -> i32 { alloc_host_text(&mut c, "") },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_response_text",
        |mut c: wasmi::Caller<'_, Data>| -> i32 {
            let body = c
                .data()
                .borrow()
                .current_fetch
                .and_then(|i| {
                    c.data()
                        .borrow()
                        .fetch_reqs
                        .get(i)
                        .map(|q| q.resp_body.clone())
                })
                .unwrap_or_default();
            alloc_host_text(&mut c, &body)
        },
    )
    .unwrap();

    // Reactive signals.
    l.func_wrap(
        "env",
        "signal_new",
        |c: wasmi::Caller<'_, Data>, init: i64| -> i64 { c.data().borrow_mut().signal_new(init) },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "signal_set",
        |c: wasmi::Caller<'_, Data>, s: i64, v: i64| {
            c.data().borrow_mut().signal_set(s, v);
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "signal_get_int",
        |c: wasmi::Caller<'_, Data>, s: i64| -> i64 { c.data().borrow().signal_get(s) },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "signal_get_text",
        |c: wasmi::Caller<'_, Data>, s: i64| -> i32 { c.data().borrow().signal_get(s) as i32 },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "signal_begin_render",
        |c: wasmi::Caller<'_, Data>| {
            c.data().borrow_mut().signal_begin();
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "signal_enter_component",
        |c: wasmi::Caller<'_, Data>, key: i32| {
            let key = read_text(&c, key);
            c.data().borrow_mut().signal_enter(&key);
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "signal_exit_component",
        |c: wasmi::Caller<'_, Data>| {
            c.data().borrow_mut().signal_exit();
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "signal_new_text",
        |c: wasmi::Caller<'_, Data>, init: i32| -> i64 {
            c.data().borrow_mut().signal_new(init as i64)
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "signal_set_text",
        |c: wasmi::Caller<'_, Data>, s: i64, v: i32| {
            c.data().borrow_mut().signal_set(s, v as i64);
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "signal_new_seq",
        |c: wasmi::Caller<'_, Data>, init: i32| -> i64 {
            c.data().borrow_mut().signal_new(init as i64)
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "signal_get_seq",
        |c: wasmi::Caller<'_, Data>, s: i64| -> i32 { c.data().borrow().signal_get(s) as i32 },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "signal_set_seq",
        |c: wasmi::Caller<'_, Data>, s: i64, v: i32| {
            c.data().borrow_mut().signal_set(s, v as i64);
        },
    )
    .unwrap();

    // Format hosts for `{status}` interpolation.
}

// ─────────────────────────────────────────────────────────────────────────────
// The driver — the browser + backend, in-process.
// ─────────────────────────────────────────────────────────────────────────────

struct App {
    dom: Data,
    store: wasmi::Store<Data>,
    inst: wasmi::Instance,
    backend: RealBackend,
}

/// The same harness, on an arbitrary page source — so a minimal reproduction of a browser-tier
/// failure can be driven by the machinery that found it rather than a second copy of it.
fn mount_source(src: &str) -> App {
    dom::reset();
    let module = compile_to_wasm(src).expect("compiles to wasm");
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, &module[..]).expect("valid wasm module");
    let dom: Data = Rc::new(RefCell::new(MockDom::new()));
    let mut linker = wasmi::Linker::<Data>::new(&engine);
    // Shared: the formatters are not this test's decision (see `wasm_host_stub_lock`).
    crate::common::wasm_host::link_value_imports::<Data>(&mut linker);
    // Every program that CAN fail declares `logos_rt_fail`, and a module importing a
    // function its host does not define FAILS TO INSTANTIATE — silently, with no
    // compile error anywhere. This host does not care why; it must simply be here.
    linker
        .func_wrap(
            "env",
            "logos_rt_fail",
            |_c: wasmi::Caller<'_, _>, _h: i32| {},
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "logos_rt_fail_at",
            |_c: wasmi::Caller<'_, _>, _code: i32, _v: i64| {},
        )
        .unwrap();
    install_hosts(&mut linker);
    let mut store = wasmi::Store::new(&engine, dom.clone());
    crate::common::wasm_host::link_unmodelled_imports(&mut linker);
    let inst = linker
        .instantiate(&mut store, &m)
        .unwrap()
        .start(&mut store)
        .unwrap();
    let backend = RealBackend::new();
    inst.get_typed_func::<(), ()>(&store, "main")
        .unwrap()
        .call(&mut store, ())
        .expect("main runs");
    App {
        dom,
        store,
        inst,
        backend,
    }
}

fn mount_app() -> App {
    dom::reset();
    let module = compile_to_wasm(&app_lg()).expect("pages/app.lg compiles to wasm");
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, &module[..]).expect("valid wasm module");
    let dom: Data = Rc::new(RefCell::new(MockDom::new()));
    let mut linker = wasmi::Linker::<Data>::new(&engine);
    // Shared: the formatters are not this test's decision (see `wasm_host_stub_lock`).
    crate::common::wasm_host::link_value_imports::<Data>(&mut linker);
    // Every program that CAN fail declares `logos_rt_fail`, and a module importing a
    // function its host does not define FAILS TO INSTANTIATE — silently, with no
    // compile error anywhere. This host does not care why; it must simply be here.
    linker
        .func_wrap(
            "env",
            "logos_rt_fail",
            |_c: wasmi::Caller<'_, _>, _h: i32| {},
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "logos_rt_fail_at",
            |_c: wasmi::Caller<'_, _>, _code: i32, _v: i64| {},
        )
        .unwrap();
    install_hosts(&mut linker);
    let mut store = wasmi::Store::new(&engine, dom.clone());
    crate::common::wasm_host::link_unmodelled_imports(&mut linker);
    let inst = linker
        .instantiate(&mut store, &m)
        .unwrap()
        .start(&mut store)
        .unwrap();
    let backend = RealBackend::new();
    inst.get_typed_func::<(), ()>(&store, "main")
        .unwrap()
        .call(&mut store, ())
        .expect("main runs (mount + boot fetch)");
    App {
        dom,
        store,
        inst,
        backend,
    }
}

impl App {
    /// Raise `event` on `#id`, which the page must listen on.
    fn dispatch(&mut self, id: &str, event: &str) {
        let node = self.element(id);
        assert!(
            self.dom.borrow().listens_for(node, event),
            "no {event} handler on #{id}"
        );
        self.deliver(node, event, "");
    }

    fn element(&self, id: &str) -> u32 {
        let d = self.dom.borrow();
        d.ids
            .iter()
            .find(|(i, _)| i == id)
            .map(|(_, n)| *n)
            .unwrap_or_else(|| panic!("no element with id #{id}"))
    }

    /// Fire the click handler registered on the element with the given `#id`.
    fn click(&mut self, id: &str) {
        self.dispatch(id, "click");
    }

    /// ★ SUBMIT THE FORM WITH THAT ID — how a todo is added now. The old app's "add" was a BUTTON
    /// and this drove its click; the rewritten one is a `<form on-submit.prevent={add}>`, which is
    /// what makes it work with no script at all (the browser posts it) AND handle its own submit
    /// when the module is loaded. Same scenario, driven the way the page is actually built.
    fn submit(&mut self, id: &str) {
        self.dispatch(id, "submit");
    }

    /// Click the first element carrying `class` that has a click listener — the
    /// delete buttons are keyed by class (there is one per row, so an id would
    /// not be unique). Uses the earliest-created such node in the current render.
    fn click_class(&mut self, class: &str) {
        let node = {
            let d = self.dom.borrow();
            d.classes
                .iter()
                // ★ A CLASS ATTRIBUTE IS A LIST OF TOKENS, NOT A STRING. This compared the whole
                // attribute for equality, so it matched `class="del"` and missed `class="del
                // lgx-todorow"` — which is what a component with a `Style:` block renders, because
                // scoping appends the component's scope class. The effect was that giving a child
                // component its own styles made its buttons unclickable TO THIS TEST ONLY, with a
                // message ("no clickable element with class .del") that described the page rather
                // than the matcher. A browser matches `.del` by token; so does this now.
                .filter(|(_, c)| c.split_whitespace().any(|t| t == class))
                .map(|(n, _)| *n)
                .find(|n| d.listens_for(*n, "click"))
        }
        .unwrap_or_else(|| panic!("no clickable element with class .{class}"));
        self.deliver(node, "click", "");
    }

    /// Hand `event` on `node`, carrying `value`, to the page's node dispatcher, as `runtime.js`
    /// hands a browser's event over.
    fn deliver(&mut self, node: u32, event: &str, value: &str) {
        let answered = crate::common::wasm_host::dispatch_node(
            &mut self.store,
            &self.inst,
            i64::from(node),
            event,
            value,
        )
        .expect("the handler fires without trapping");
        assert!(
            answered,
            "the page listens for {event} on node {node}, so its dispatcher answers it"
        );
    }

    /// Drive EVERY outstanding fetch the frontend has issued: route each through
    /// the backend oracle and deliver its response, firing the frontend's
    /// completion closure. Returns the number of fetches serviced.
    fn service_fetches(&mut self) -> usize {
        let mut serviced = 0;
        loop {
            let next = {
                let d = self.dom.borrow();
                d.fetch_reqs.iter().position(|q| !q.done).map(|i| {
                    (
                        i as i64 + 1,
                        d.fetch_reqs[i].method.clone(),
                        d.fetch_reqs[i].url.clone(),
                        d.fetch_reqs[i].body.clone(),
                    )
                })
            };
            let Some((req_id, method, url, body)) = next else {
                break;
            };
            let (status, resp_body) = self.backend.handle(&method, &url, &body);
            let handle = self
                .dom
                .borrow_mut()
                .resolve_fetch(req_id, status, &resp_body);
            if let Some(h) = handle {
                self.inst
                    .get_typed_func::<(i32, i64), i64>(&self.store, "__logos_call_closure")
                    .expect("trampoline exported")
                    .call(&mut self.store, (h, req_id))
                    .expect("fetch completion fires without trapping");
            }
            serviced += 1;
        }
        serviced
    }

    /// The browser settle loop.
    ///
    /// ★ AN LGX TURN REPAINTS ITSELF. The retiring dialect set a dirty flag and a host called
    /// `__logos_render` afterwards; a component's turn opens its own paint boundary and repaints —
    /// or patches the sites that changed — INSIDE the handler, before it returns. So there is no
    /// re-render entry to call and no flag to consult: the dirty bit is drained so a stale one
    /// cannot leak into the next assertion, and the painting has already happened.
    fn settle(&mut self) {
        let _ = self.dom.borrow_mut().take_dirty();
    }

    /// Deliver all pending fetches, then settle — the full "responses arrive →
    /// reactive write → re-render" cycle the browser runs.
    fn flush(&mut self) {
        self.service_fetches();
        self.settle();
    }

    /// The current length of the DOM op-log — a mark to observe "what changed
    /// since" (each render appends; the LATEST render is the live page).
    /// The texts the page SHOWS now, in document order: the op-log replayed onto a tree from
    /// `body`, and every node's own text read off it. A paint patches what changed and leaves
    /// the rest standing, so "what does the page show" is the document, never a range of the log.
    fn shown_texts(&self) -> Vec<String> {
        use std::collections::HashMap;
        let d = self.dom.borrow();
        let mut own: HashMap<u32, String> = HashMap::new();
        let mut kids: HashMap<u32, Vec<u32>> = HashMap::new();
        let detach = |kids: &mut HashMap<u32, Vec<u32>>, node: u32| {
            for siblings in kids.values_mut() {
                siblings.retain(|&c| c != node);
            }
        };
        for op in &d.log {
            match op {
                DomOp::CreateTextNode { node, text } => {
                    own.insert(*node, text.clone());
                }
                DomOp::SetText { node, text } => {
                    own.insert(*node, text.clone());
                    kids.remove(node);
                }
                DomOp::ClearChildren { node } => {
                    kids.remove(node);
                }
                DomOp::AppendChild { parent, child } => {
                    detach(&mut kids, *child);
                    kids.entry(*parent).or_default().push(*child);
                }
                DomOp::InsertBefore {
                    parent,
                    child,
                    before,
                } => {
                    detach(&mut kids, *child);
                    let siblings = kids.entry(*parent).or_default();
                    match siblings.iter().position(|c| c == before) {
                        Some(at) => siblings.insert(at, *child),
                        None => siblings.push(*child),
                    }
                }
                DomOp::RemoveNode { node } | DomOp::FreeNode { node } => detach(&mut kids, *node),
                _ => {}
            }
        }
        let mut out = Vec::new();
        let mut stack = vec![0u32];
        while let Some(n) = stack.pop() {
            if let Some(t) = own.get(&n).filter(|t| !t.is_empty()) {
                out.push(t.clone());
            }
            stack.extend(kids.get(&n).into_iter().flatten().rev());
        }
        out
    }

    /// Set the `draft` text signal directly (the value a `bound to` input would
    /// have written) so the next Add posts it. The draft is the frontend's second
    /// signal (`todos`=0, `draft`=1) in the component's hook order.
    /// ★ TYPE INTO THE BOUND INPUT — the way a visitor does, and the way a browser reports it.
    ///
    /// The retiring app kept its draft in a SIGNAL CELL and this wrote that cell directly. The
    /// rewritten page uses `bind={draft}`, which registers the input for `input` events whose
    /// value the dispatcher writes into the field: so typing is a real dispatch carrying a real
    /// value, not a poke at the runtime's insides. Driving it this way is what makes the assertion
    /// downstream mean "a person could add a todo" rather than "a cell can be written".
    fn set_draft(&mut self, text: &str) {
        let node = self.element("draft");
        assert!(
            self.dom.borrow().listens_for(node, "input"),
            "a `bind` must register an input listener, or a browser hears nothing"
        );
        self.deliver(node, "input", text);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The crown lock.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn the_two_lg_files_interoperate_headlessly() {
    // Sanity: both halves compile on their own tiers. If either refuses, the
    // capstone is broken before the contract is even exercised.
    assert!(
        compile_to_wasm(&app_lg()).is_ok(),
        "the frontend must compile to WASM"
    );
    let server_program = logicaffeine_compile::compile::compile_to_program(&server_lg());
    assert!(
        server_program.is_ok(),
        "the backend must compile: {server_program:?}"
    );

    let mut app = mount_app();

    // (1) INITIAL LOAD. `main` fired the boot `GET /api/todos`; the store is
    // empty, so the list renders empty (no todo <li> text).
    app.flush();
    let shown = app.shown_texts();
    assert!(
        !shown.iter().any(|t| t == "milk" || t == "eggs"),
        "an empty store must render no todo rows, got: {shown:?}"
    );

    // (2) ADD "milk". Type it into the draft, click Add → POST /api/todos →
    // backend appends → returns the fresh list → frontend re-renders with the row.
    app.set_draft("milk");
    app.submit("add");
    app.flush();
    // ★ THE ROW SHOWS ITS TEXT. The retiring app rendered an `id|text` line because that was its
    // wire format; the rewritten `TodoRow` renders the todo the way a person reads it, and the id
    // lives in the JSON rather than on the screen. Same scenario, the page's own rendering.
    let shown = app.shown_texts();
    assert!(
        shown.iter().any(|t| t == "milk"),
        "after adding 'milk', the rendered list must show it, got: {shown:?}"
    );

    // (3) ADD "eggs". Two todos now.
    app.set_draft("eggs");
    app.submit("add");
    app.flush();
    let after_two = app.shown_texts();
    assert!(
        after_two.iter().any(|t| t == "milk") && after_two.iter().any(|t| t == "eggs"),
        "after adding 'eggs', both todos must render, got: {after_two:?}"
    );

    // The backend store agrees — the file the REAL handler persisted holds
    // exactly the two rows.
    // The server's PRIVATE on-disk format — `id|text|done`, one per line. It is deliberately not
    // the wire format (that is derived JSON), which is the whole point of keeping them separate.
    assert_eq!(
        app.backend.store_text(),
        "1|milk|0\n2|eggs|0\n",
        "the backend store after two adds"
    );

    // (4) DELETE milk (id 1). Click the first row's delete button → POST
    // /api/todos/delete with id "1" → backend drops it → returns the fresh list →
    // the re-render shows only eggs.
    app.click_class("del");
    app.flush();
    let after_delete = app.shown_texts();
    assert!(
        after_delete.iter().any(|t| t == "eggs"),
        "after deleting milk, eggs must remain, got: {after_delete:?}"
    );
    assert!(
        !after_delete.iter().any(|t| t == "milk"),
        "after deleting milk, it must be GONE from the page, got: {after_delete:?}"
    );
    assert_eq!(
        app.backend.store_text(),
        "2|eggs|0\n",
        "the backend store after the delete"
    );
}

/// The wire contract itself, isolated: what the backend's `## Route` ANSWERS is exactly what the
/// frontend's typed fetch DECODES. This pins the SHARED LANGUAGE independently of the DOM
/// plumbing — if the two ends' notion of a row ever diverges, this fails first.
///
/// ★ ONE DECLARATION, BOTH DIRECTIONS. Neither side writes a serialiser: the server answers
/// `Seq of Todo` and the encoder is DERIVED from `## A Todo has:` in `shared.lg`; the page asks
/// `as Seq of Todo` and the decoder is derived from the same block. That is why this test can
/// round-trip the real bytes rather than compare two hand-written formats that agree until they
/// don't — the failure this lock was written to catch is now structurally impossible to reach by
/// editing one side, and the test says so by driving BOTH.
///
/// Migrated from the retiring `id|text` line format (T6.13): the scenarios are unchanged — empty,
/// add, add, remove, add — and the answers are now the JSON a deployed server really sends.
#[test]
fn the_backend_wire_format_is_what_the_frontend_parses() {
    let mut backend = RealBackend::new();

    // Every request goes through the REAL router, so these are the bytes a client receives.
    let (status, empty) = backend.handle("GET", "/api/todos", "");
    assert_eq!(status, 200, "a served collection answers 200");
    assert_eq!(
        empty, "[]",
        "an empty store is an empty ARRAY, not an empty body"
    );

    let (_, one) = backend.handle("POST", "/api/todos", "text=buy%20milk");
    assert_eq!(one, r#"[{"id":1,"text":"buy milk","done":false}]"#);

    let (_, two) = backend.handle("POST", "/api/todos", "text=write%20tests");
    assert_eq!(
        two,
        r#"[{"id":1,"text":"buy milk","done":false},{"id":2,"text":"write tests","done":false}]"#
    );

    let (_, after_delete) = backend.handle("POST", "/api/todos/delete/1", "");
    assert_eq!(
        after_delete,
        r#"[{"id":2,"text":"write tests","done":false}]"#
    );

    let (_, three) = backend.handle("POST", "/api/todos", "text=ship%20it");
    assert_eq!(
        three,
        r#"[{"id":2,"text":"write tests","done":false},{"id":3,"text":"ship it","done":false}]"#
    );

    // A toggle changes the FLAG and nothing else — the field the old line format encoded as a
    // trailing `0`/`1`, and the one a client reads to strike a row through.
    let (_, toggled) = backend.handle("POST", "/api/todos/toggle/2", "");
    assert_eq!(
        toggled,
        r#"[{"id":2,"text":"write tests","done":true},{"id":3,"text":"ship it","done":false}]"#
    );

    // ★ AND THE FRONTEND READS EXACTLY THOSE BYTES. `fromJsonSeq_Todo` is the decoder the page's
    // `Get "/api/todos" as Seq of Todo` uses — derived from the same `## A Todo has:` the encoder
    // above came from. Run on the tree-walker (the same code the WASM frontend lowers).
    let app_lib = {
        let a = app_lg();
        let idx = a.find("\n## Component").expect("app has a component");
        a[..idx].to_string()
    };
    let driver = format!(
        "{}\n## Main\nInspect fromJsonSeq_Todo({}):\n        \x20   When Ok (rows):\n        \x20       Repeat for r in rows:\n        \x20           Show \"{{r's id}}|{{r's text}}|{{r's done}}\".\n        \x20   When Err (e):\n        \x20       Show \"decode failed: \" + e.\n",
        app_lib,
        lg_str(&toggled)
    );
    let out = tw_outcome(&driver);
    assert!(
        out.error.is_none(),
        "the derived decoder errored: {:?}",
        out.error
    );
    assert_eq!(
        out.output.trim(),
        "2|write tests|true\n3|ship it|false",
        "the frontend decodes the backend's rows field for field"
    );
}

/// ★ THE SMALLEST TYPED FETCH THAT RUNS IN A BROWSER — a page asks its own server for a list and
/// shows it. This is the sentence at the centre of every data-driven page, and the bytecode tier is
/// where it has to work: `typed_fetch_lock` drives the REFERENCE tier, so until this test existed
/// nothing had ever delivered a fetch answer into a compiled WebAssembly module.
///
/// It lives here rather than beside `typed_fetch_lock` because this file already owns the machinery
/// that can do it — a real module, a real host, a real completion closure fired through the
/// trampoline — and a second copy of that harness would be a second thing to keep true.
#[test]
fn a_typed_fetch_completion_runs_on_the_browser_tier() {
    const PAGE: &str = "## A Row has:\n\
      \x20   an id, which is Int.\n\
      \x20   a text, which is Text.\n\
       \n\
       ## An HttpError has:\n\
      \x20   a status, which is Int.\n\
      \x20   a message, which is Text.\n\
       \n\
       ## Component App\n\
       \n\
       State:\n\
      \x20   Let rows be a new Seq of Row.\n\
      \x20   Let note be \"loading\".\n\
       \n\
       Actions:\n\
      \x20   When the page loads:\n\
      \x20       Get \"/api/rows\" as Seq of Row, then loaded.\n\
       \n\
      \x20   To loaded (answer: Result of Seq of Row and HttpError):\n\
      \x20       Inspect answer:\n\
      \x20           When Ok (list):\n\
      \x20               Set rows to list.\n\
      \x20               Set note to \"loaded\".\n\
      \x20           When Err (e):\n\
      \x20               Set note to e's message.\n\
       \n\
       View:\n\
      \x20   <main><p id=\"note\">{note}</p></main>\n\
       \n\
       ## Main\n\
       Mount App.\n";

    let mut app = mount_source(PAGE);
    // The boot fetch went out.
    let outstanding = app
        .dom
        .borrow()
        .fetch_reqs
        .iter()
        .filter(|q| !q.done)
        .count();
    assert_eq!(outstanding, 1, "the page must ask for its data on load");

    // Deliver the answer the way a browser does: resolve the request, then fire its completion
    // closure through the trampoline.
    let handle = app
        .dom
        .borrow_mut()
        .resolve_fetch(1, 200, r#"[{"id":1,"text":"milk"}]"#)
        .expect("the request resolves");
    app.inst
        .get_typed_func::<(i32, i64), i64>(&app.store, "__logos_call_closure")
        .expect("trampoline exported")
        .call(&mut app.store, (handle, 1))
        .expect("the fetch completion must RUN, not trap");
}
