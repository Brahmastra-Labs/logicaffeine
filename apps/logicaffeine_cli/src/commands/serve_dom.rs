//! `largo serve [--watch]` for a DOM app — the live-reload dev loop.
//!
//! A DOM app (a program built from `## Component`/`View:` and `Mount`) has no
//! `Serve` statement to run itself, so `largo serve` BUILDS it — the same artifacts
//! as `--emit wasm-dom` (`<name>.wasm` + `<name>.js` + `<name>.html`) — and serves
//! them statically over the hand-rolled HTTP core. `--watch` adds a source watcher
//! that rebuilds on change and bumps a generation counter the browser POLLS
//! (`GET /__gen`); a tiny injected client reloads the page when it changes. It polls
//! rather than holding an SSE stream because the HTTP core answers one request per
//! connection and closes — a poll fits that model exactly.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use logicaffeine_system::http::server::{serve_on_with, Listener};
use logicaffeine_system::http::wire::{HttpRequest, HttpResponse};
use logicaffeine_system::relay::RelayHub;

use crate::commands::emit::{browser_host_html, browser_host_js, wasm_host_shim};
use crate::diagnostics::{self, Placed};
use crate::project::shape::Project;
use crate::ui;
use logicaffeine_compile::web_host::error_overlay_html;

/// The built, servable artifacts + the generation counter the reload client polls.
struct Artifacts {
    generation: u64,
    wasm: Vec<u8>,
    js: String,
    html: String,
    mjs: String,
    /// The last build error, pre-rendered as the themed overlay page (with the reload
    /// client). When set, `/` serves it so a bad edit shows a beautiful error — source
    /// line, caret, Socratic message — instead of a stale/blank page; the last-good
    /// `wasm`/`js` stay live so fixing the source reloads cleanly.
    error_overlay: Option<String>,
    /// The same error as the terminal prints it, `file:line:col` and all.
    error_text: Option<String>,
}

/// The live-reload client appended to the served JS under `--watch`. It polls `/__gen`
/// and reloads when the generation changes — but SMOOTHLY: it saves the scroll position
/// across the reload and restores it, then flashes a subtle "↻ updated" toast, so a save
/// feels intentional instead of jarring you back to the top with no feedback.
pub(crate) const RELOAD_CLIENT: &str = "\n;(function(){\
var K='__logos_scroll';\
try{var y=sessionStorage.getItem(K);if(y!==null){sessionStorage.removeItem(K);window.scrollTo(0,+y);\
var t=document.createElement('div');t.textContent='\u{21bb} updated';\
t.style.cssText='position:fixed;bottom:16px;right:16px;z-index:2147483647;padding:6px 13px;border-radius:999px;background:#cc785c;color:#fff;font:600 13px/1 ui-sans-serif,system-ui,sans-serif;box-shadow:0 6px 18px rgba(0,0,0,.2);opacity:0;transform:translateY(6px);transition:opacity .18s ease,transform .18s ease;pointer-events:none';\
document.body.appendChild(t);requestAnimationFrame(function(){t.style.opacity='1';t.style.transform='translateY(0)';});\
setTimeout(function(){t.style.opacity='0';setTimeout(function(){t.remove();},260);},1100);}}catch(e){}\
var last=null;setInterval(function(){fetch('/__gen').then(function(r){return r.text();}).then(function(g){\
if(last!==null&&g!==last){try{sessionStorage.setItem(K,String(window.scrollY));}catch(e){}location.reload();}last=g;\
}).catch(function(){});},350);})();\n";

/// The `Serving` banner text — `<name> on http://<addr>` with the watch hint. Pure, so
/// the console output is unit-tested without a live server.
fn serving_line(name: &str, addr: &std::net::SocketAddr, watch: bool) -> String {
    let hint = if watch {
        "  (watching src — the browser live-reloads)"
    } else {
        ""
    };
    format!("{name} on http://{addr}{hint}")
}

/// The per-rebuild success line, carrying the elapsed build time + the generation.
fn rebuilt_line(generation: u64, elapsed: Duration) -> String {
    format!("in {}ms — generation {generation}", elapsed.as_millis())
}

/// The per-rebuild failure line, carrying the elapsed build time and the error itself — the
/// terminal is where a developer is looking when they save.
fn rebuild_failed_line(elapsed: Duration, error: &str) -> String {
    format!(
        "rebuild failed in {}ms — serving last good (the browser overlay shows the same error):\n{error}",
        elapsed.as_millis()
    )
}

fn response(status: u16, content_type: &str, body: Vec<u8>) -> HttpResponse {
    let mut headers = BTreeMap::new();
    headers.insert("Content-Type".to_string(), content_type.to_string());
    HttpResponse {
        status,
        headers,
        body,
    }
}

/// Build the four servable artifacts from a closure for module stem `name`. In `watch` mode the
/// reload client is appended to the JS (and embedded in the error overlay). A compile error
/// surfaces as the themed overlay page (`Artifacts::error_overlay`) and the terminal text
/// (`Artifacts::error_text`), both drawn against the file the mistake is in — the closure's
/// `SourceMap` undoes the splice of every imported module ahead of it.
fn build(
    closure: &crate::closure::Closure,
    entry: &Path,
    name: &str,
    generation: u64,
    watch: bool,
) -> Artifacts {
    let reload = if watch { RELOAD_CLIENT } else { "" };
    match logicaffeine_compile::compile::compile_to_wasm(&closure.source) {
        Ok(wasm) => {
            let mut js = browser_host_js(name);
            if watch {
                js.push_str(RELOAD_CLIENT);
            }
            // A copy-lint warning does not fail the build (it never has a fatal shape), but the
            // author still needs to hear it — the ONE surface `largo serve` prints anything to is
            // the terminal, matching `largo check`'s own treatment of the same finding.
            for f in logicaffeine_compile::ui_bridge::value_copy_findings_of(&closure.source) {
                let at = Placed::locate(f.span, &closure.source, &closure.source_map, entry);
                ui::warning(&diagnostics::render_copy_finding(&f, &at));
            }
            Artifacts {
                generation,
                wasm,
                js,
                html: browser_host_html(name),
                mjs: wasm_host_shim(name),
                error_overlay: None,
                error_text: None,
            }
        }
        Err(pe) => {
            let at = Placed::locate(pe.span, &closure.source, &closure.source_map, entry);
            Artifacts {
                generation,
                wasm: Vec::new(),
                js: String::new(),
                html: String::new(),
                mjs: String::new(),
                error_overlay: Some(diagnostics::overlay(&pe, &at, reload)),
                error_text: Some(diagnostics::render_parse_error(&pe, &at)),
            }
        }
    }
}

/// Route one request to a static artifact, the `/__gen` counter, or a 404. Pure over
/// `art`, so it is unit-tested directly.
fn route(path: &str, name: &str, art: &Artifacts) -> HttpResponse {
    let path = path.split('?').next().unwrap_or(path);
    if path == "/__gen" {
        return response(200, "text/plain", art.generation.to_string().into_bytes());
    }
    if path == "/" || path == "/index.html" {
        if let Some(overlay) = &art.error_overlay {
            return response(
                200,
                "text/html; charset=utf-8",
                overlay.clone().into_bytes(),
            );
        }
        return response(
            200,
            "text/html; charset=utf-8",
            art.html.clone().into_bytes(),
        );
    }
    if path == format!("/{name}.js") {
        return response(200, "text/javascript", art.js.clone().into_bytes());
    }
    if path == format!("/{name}.wasm") {
        return response(200, "application/wasm", art.wasm.clone());
    }
    if path == format!("/{name}.mjs") {
        return response(200, "text/javascript", art.mjs.clone().into_bytes());
    }
    response(404, "text/plain", b"Not Found".to_vec())
}

/// The newest mtime across the WHOLE watched program: every file of the
/// assembled module closure (`files`) plus every `.lg` under the project root,
/// recursively — so an edit to a nested imported module (or a brand-new module
/// about to be imported) ticks the watcher. `target/` and dot-directories
/// (`.git`, …) are never scanned.
fn source_fingerprint(root: &Path, files: &[PathBuf]) -> SystemTime {
    let mut newest = SystemTime::UNIX_EPOCH;
    for f in files {
        if let Ok(m) = std::fs::metadata(f).and_then(|m| m.modified()) {
            newest = newest.max(m);
        }
    }
    scan_lg_mtimes(root, &mut newest, 0);
    newest
}

/// Recursive `.lg` mtime sweep under `dir`, skipping `target/` and
/// dot-directories; depth-capped as a symlink-cycle backstop.
fn scan_lg_mtimes(dir: &Path, newest: &mut SystemTime, depth: usize) {
    if depth > 32 {
        return;
    }
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for f in read.flatten() {
        let Ok(kind) = f.file_type() else { continue };
        if kind.is_dir() {
            let name = f.file_name();
            let name = name.to_string_lossy();
            if name == "target" || name.starts_with('.') {
                continue;
            }
            scan_lg_mtimes(&f.path(), newest, depth + 1);
        } else if f.path().extension().and_then(|e| e.to_str()) == Some("lg") {
            if let Ok(m) = f.metadata().and_then(|m| m.modified()) {
                *newest = (*newest).max(m);
            }
        }
    }
}

/// The artifacts served when the project cannot even ASSEMBLE (an unresolvable
/// import): keep nothing but the themed overlay carrying the assembler's
/// message, with the reload client so a fix snaps back to green.
fn assemble_failure_artifacts(message: &str, generation: u64) -> Artifacts {
    let pe = logicaffeine_language::error::ParseError {
        kind: logicaffeine_language::error::ParseErrorKind::Custom(message.to_string()),
        span: Default::default(),
    };
    Artifacts {
        generation,
        wasm: Vec::new(),
        js: String::new(),
        html: String::new(),
        mjs: String::new(),
        error_overlay: Some(error_overlay_html(&pe, "", RELOAD_CLIENT)),
        error_text: Some(message.to_string()),
    }
}

/// The watcher thread: poll the whole-closure fingerprint; on change, RE-ASSEMBLE
/// the closure (imports may have changed) and rebuild from it, swapping in the new
/// artifacts (bumping the generation the browser polls). A failed rebuild keeps the
/// last-good `wasm`/`js` live but records the error + bumps the generation, so the
/// page reloads into the error overlay and back to green once the source is fixed.
fn watch_loop(
    entry: PathBuf,
    root: PathBuf,
    name: String,
    state: Arc<Mutex<Artifacts>>,
    mut files: Vec<PathBuf>,
) {
    let mut last = source_fingerprint(&root, &files);
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let now = source_fingerprint(&root, &files);
        if now == last {
            continue;
        }
        last = now;
        std::thread::sleep(Duration::from_millis(60)); // settle a burst of saves
        let started = Instant::now();
        let assembled = crate::closure::assemble_closure(&entry);
        let mut st = state.lock().unwrap();
        let generation = st.generation + 1;
        let fresh = match &assembled {
            Ok(closure) => {
                files = closure.files.clone();
                build(closure, &entry, &name, generation, true)
            }
            Err(message) => assemble_failure_artifacts(message, generation),
        };
        let elapsed = started.elapsed();
        if fresh.error_overlay.is_some() {
            // Keep the last-good bundle live; only flip the error + generation.
            st.generation = generation;
            st.error_overlay = fresh.error_overlay;
            st.error_text = fresh.error_text;
        } else {
            *st = fresh;
        }
        match &st.error_text {
            Some(error) => ui::warning(rebuild_failed_line(elapsed, error)),
            None => ui::phase("Rebuilt", rebuilt_line(generation, elapsed)),
        }
    }
}

/// The OS command that opens a URL in the default browser — `(program, args)`. Pure, so
/// the platform choice is unit-tested; the actual spawn is best-effort and never fatal.
pub(crate) fn open_command(url: &str) -> (&'static str, Vec<String>) {
    if cfg!(target_os = "macos") {
        ("open", vec![url.to_string()])
    } else if cfg!(target_os = "windows") {
        // `cmd /C start "" <url>` — the empty title avoids `start` eating the URL as a title.
        (
            "cmd",
            vec!["/C".into(), "start".into(), String::new(), url.to_string()],
        )
    } else {
        ("xdg-open", vec![url.to_string()])
    }
}

/// Serve a program's WebAssembly bundle. Builds the entry, serves the static bundle on
/// `127.0.0.1:<port>` (falling forward to the next free port if it's taken), optionally opens the
/// browser, and — when `watch` — rebuilds + live-reloads on source change. Under `watch`, even a
/// FIRST build that fails to compile still binds the port and serves the themed overlay (the
/// located error) rather than refusing to start — the same recovery a mid-session break already
/// gets from [`watch_loop`]. Without `watch`, a build failure refuses loudly instead, since there
/// is no watcher that could ever fix it.
pub(crate) fn cmd_serve_dom(
    project: &Project,
    port: u16,
    watch: bool,
    open: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = project.root.clone();
    let entry = project.require_entry()?.to_path_buf();
    let name = project.manifest.package.name.clone();
    // The SAME assembled closure every other command compiles: imported
    // modules reach the wasm build, and their files are what `--watch` tracks.
    let closure = crate::closure::assemble_closure(&entry)
        .map_err(|e| format!("cannot serve — could not assemble the project: {e}"))?;

    let first = build(&closure, &entry, &name, 1, watch);
    if !watch {
        // No watcher to ever fix this: fail loudly with the Socratic terminal error, at the
        // author's line, rather than bind a port that can only ever answer the same overlay.
        if let Some(error) = &first.error_text {
            return Err(format!("cannot serve — the project does not build:\n{error}").into());
        }
    }
    // Under `--watch`, a build error is not a reason to refuse starting — even the FIRST one: the
    // whole point of the watch loop below is to survive exactly this, and it already does (a mid-
    // session break keeps serving the last-good bundle plus the new overlay). Binding the port
    // now, with `first`'s overlay already in `state`, means starting on a broken page gets the
    // same treatment as breaking one after the server is already up.
    let state = Arc::new(Mutex::new(first));

    if watch {
        let (entry, root, name, state) = (
            entry.clone(),
            root.clone(),
            name.clone(),
            Arc::clone(&state),
        );
        let files = closure.files;
        std::thread::spawn(move || watch_loop(entry, root, name, state, files));
    }

    let rt = logicaffeine_system::tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start async runtime: {e}"))?;
    rt.block_on(async move {
        // Bind the requested port, falling forward to the next free one if it's taken —
        // no more "address in use" deaths when a stray server is still up.
        let mut listener = None;
        let mut last_err = String::new();
        for p in port..port.saturating_add(16) {
            match Listener::bind(&format!("127.0.0.1:{p}")).await {
                Ok(l) => {
                    listener = Some(l);
                    break;
                }
                Err(e) => last_err = e.to_string(),
            }
        }
        let listener = listener.ok_or_else(|| {
            format!(
                "cannot bind any port in {port}..{} (last: {last_err})",
                port.saturating_add(16)
            )
        })?;
        let bound = listener.local_addr().map_err(|e| format!("{e}"))?;
        ui::phase("Serving", serving_line(&name, &bound, watch));
        if open {
            let url = format!("http://{bound}");
            let (prog, args) = open_command(&url);
            // Best-effort: a missing opener (headless/CI) must not fail the server.
            match std::process::Command::new(prog).args(&args).spawn() {
                Ok(_) => ui::phase("Opening", url),
                Err(_) => ui::info(format!("(could not open a browser — visit {url})")),
            }
        }
        let handler_state = Arc::clone(&state);
        let handler_name = name.clone();
        // `/_relay`: the same table `serve_web` registers, so a single-file DOM app's
        // `Raise`/`Publish` reach a real browser round-trip here too.
        let relay = RelayHub::new().upgrades();
        serve_on_with(
            listener,
            move |req: HttpRequest| {
                let art = handler_state.lock().unwrap();
                route(&req.path, &handler_name, &art)
            },
            relay,
        )
        .await
        .map_err(|e| format!("serve loop ended: {e}"))?;
        Ok::<(), String>(())
    })?;
    Ok(())
}

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_cli/commands/serve_dom/tests.rs"]
mod tests;
