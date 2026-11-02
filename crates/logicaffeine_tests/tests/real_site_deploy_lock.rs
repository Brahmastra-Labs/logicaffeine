//! ★ LOCK — a REAL LOGOS website builds, deploys and serves. ★
//!
//! `build_web_release` proves the SCAFFOLD ships: one page, one counter, one hand-written handler.
//! Nothing proved that a site anybody would actually write does — a shared record, several routes
//! including a path parameter, a keyed list rendering a child component, a form, persistence
//! across requests, and a page that reads before its script runs.
//!
//! ★ THE GAP THAT MOTIVATED THIS. Writing exactly such an app by hand found, in one sitting: a
//! record could not be named after a keyword, a component parameter could not be either, and a
//! server that decodes its own JSON does not compile on the deploy tier — none of which any of the
//! ~600 web tests caught, because a capability lock proves a capability and only a whole app
//! proves they meet. This file is that app, kept.
//!
//! ★ IT ASSERTS WHAT COMES BACK ON THE WIRE, not that the build returned `Ok`. `compile_to_rust`
//! answers `Ok` for a program it refused, and `largo build` can succeed on a binary that then
//! answers nothing useful — so every claim here is a real request to a real process.
//!
//! `#[ignore]`d beside the other deploy heavies: it compiles a release binary.

#[path = "cli_common/mod.rs"]
mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::time::{Duration, Instant};

fn request(port: u16, method: &str, path: &str, body: &str, timeout: Duration) -> Vec<u8> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) {
            let req = format!(
                "{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let _ = s.write_all(req.as_bytes());
            let mut raw = Vec::new();
            let _ = s.read_to_end(&mut raw);
            if !raw.is_empty() {
                return raw;
            }
        }
        assert!(
            Instant::now() < deadline,
            "the site never answered {method} {path}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
fn get(port: u16, path: &str) -> Vec<u8> {
    request(port, "GET", path, "", Duration::from_secs(30))
}
fn post(port: u16, path: &str, body: &str) -> Vec<u8> {
    request(port, "POST", path, body, Duration::from_secs(30))
}
fn body_of(raw: &[u8]) -> String {
    let b = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| &raw[i + 4..])
        .unwrap_or(&[]);
    String::from_utf8_lossy(b).into_owned()
}
fn has(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// The shape both halves agree on, in its own module — a real project splits this out, and doing
/// so is what makes the wire a contract rather than two hand-written serialisers.
const SHARED: &str = "# Shared\n\n\
    ## A Memo has:\n\
    \x20   an id, which is Int.\n\
    \x20   a title, which is Text.\n\
    \x20   a pinned, which is Bool.\n";

/// Four routes: a collection, a bare scalar, a create, and one with a PATH PARAMETER.
const BACKEND: &str = "# Notes backend\n\n\
    Import Shared.\n\n\
    ## To storePath () -> Text:\n\
    \x20   Return \"notes.txt\".\n\n\
    ## To loadNotes () -> Seq of Memo:\n\
    \x20   Let mutable out be a new Seq of Memo.\n\
    \x20   Let mutable raw be \"\".\n\
    \x20   Inspect read(storePath()):\n\
    \x20       When Ok (t):\n\
    \x20           Set raw to t.\n\
    \x20       When Err (e):\n\
    \x20           Set raw to \"\".\n\
    \x20   Repeat for line in lines(raw):\n\
    \x20       If length of text_bytes(line) is greater than 0:\n\
    \x20           Let parts be split(line, \"|\").\n\
    \x20           If length of parts is at least 3:\n\
    \x20               Let idText be item 1 of parts.\n\
    \x20               Let titleText be item 2 of parts.\n\
    \x20               Let pinText be item 3 of parts.\n\
    \x20               If isIntText(idText):\n\
    \x20                   Push (a new Memo with id parseInt(idText) and title titleText and pinned (pinText is equal to \"1\")) to out.\n\
    \x20   Return out.\n\n\
    ## To saveNotes (notes: Seq of Memo) -> Bool:\n\
    \x20   Let mutable out be a new Seq of Text.\n\
    \x20   Repeat for n in notes:\n\
    \x20       Let mutable flag be \"0\".\n\
    \x20       If n's pinned:\n\
    \x20           Set flag to \"1\".\n\
    \x20       Push (\"\" + n's id + \"|\" + n's title + \"|\" + flag) to out.\n\
    \x20   Let w be write(storePath(), join(out, \"\\n\")).\n\
    \x20   Return true.\n\n\
    ## To nextId (notes: Seq of Memo) -> Int:\n\
    \x20   Let mutable top be 0.\n\
    \x20   Repeat for n in notes:\n\
    \x20       If n's id is greater than top:\n\
    \x20           Set top to n's id.\n\
    \x20   Return top + 1.\n\n\
    ## Route GET /api/notes -> Seq of Memo:\n\
    \x20   Return loadNotes().\n\n\
    ## Route GET /api/notes/count -> Int:\n\
    \x20   Return length of loadNotes().\n\n\
    ## Route POST /api/notes (title: Text) -> Seq of Memo:\n\
    \x20   Let notes be loadNotes().\n\
    \x20   If title is equal to \"\":\n\
    \x20       Return notes.\n\
    \x20   Let mutable next be a new Seq of Memo.\n\
    \x20   Repeat for n in notes:\n\
    \x20       Push n to next.\n\
    \x20   Push (a new Memo with id nextId(notes) and title title and pinned false) to next.\n\
    \x20   Let saved be saveNotes(next).\n\
    \x20   Return next.\n\n\
    ## Route POST /api/notes/pin/:id (id: Int) -> Seq of Memo:\n\
    \x20   Let notes be loadNotes().\n\
    \x20   Let mutable next be a new Seq of Memo.\n\
    \x20   Repeat for n in notes:\n\
    \x20       If n's id is equal to id:\n\
    \x20           Push (a new Memo with id n's id and title n's title and pinned (not n's pinned)) to next.\n\
    \x20       If n's id is not equal to id:\n\
    \x20           Push n to next.\n\
    \x20   Let saved be saveNotes(next).\n\
    \x20   Return next.\n\n\
    ## Main\n\n\
    Serve on port 8080.\n";

/// A child component in a KEYED loop with a callback prop, a form, derived state and scoped CSS.
const PAGE: &str = "# Notes\n\n\
    Import Shared.\n\n\
    ## Component MemoRow (entry: Memo) and (pin: fn(Int) -> Unit)\n\n\
    View:\n\
    \x20   <li class:pinned={entry's pinned}>\n\
    \x20     <button id=\"pin\" on-click={pin(entry's id)} aria-label=\"pin {entry's title}\">*</button>\n\
    \x20     <span class=\"title\">{entry's title}</span>\n\
    \x20   </li>\n\n\
    Style:\n\
    \x20   li { display: flex; gap: .5rem; }\n\n\
    ## Page /\n\n\
    State:\n\
    \x20   Let notes be a new Seq of Memo.\n\
    \x20   Let draft be \"\".\n\
    \x20   Let total always be length of notes.\n\n\
    Actions:\n\
    \x20   When the page loads:\n\
    \x20       Get \"/api/notes\" as Seq of Memo, then loaded.\n\n\
    \x20   To loaded (answer: Result of Seq of Memo and HttpError):\n\
    \x20       Inspect answer:\n\
    \x20           When Ok (list):\n\
    \x20               Set notes to list.\n\
    \x20           When Err (e):\n\
    \x20               Set draft to \"\".\n\n\
    \x20   To add:\n\
    \x20       If draft is equal to \"\":\n\
    \x20           Return.\n\
    \x20       Post \"title=\" + urlEncode(draft) to \"/api/notes\" as Seq of Memo, then loaded.\n\
    \x20       Set draft to \"\".\n\n\
    \x20   To pin (id: Int):\n\
    \x20       Post \"\" to \"/api/notes/pin/\" + \"\" + id as Seq of Memo, then loaded.\n\n\
    View:\n\
    \x20   <main>\n\
    \x20     <h1>Notes <small id=\"total\">{total}</small></h1>\n\
    \x20     <form id=\"new\" on-submit.prevent={add}>\n\
    \x20       <input id=\"draft\" type=\"text\" bind={draft} aria-label=\"new note\">\n\
    \x20       <button id=\"save\" type=\"submit\">Save</button>\n\
    \x20     </form>\n\
    \x20     <ul id=\"list\">\n\
    \x20       {for each n in notes, keyed by n's id:}\n\
    \x20         <MemoRow entry={n} pin={pin}/>\n\
    \x20       {else:}\n\
    \x20         <li id=\"empty\">Nothing yet.</li>\n\
    \x20       {end}\n\
    \x20     </ul>\n\
    \x20   </main>\n";

#[test]
#[ignore = "compiles a real release binary — run with --ignored beside the other deploy heavies"]
fn a_real_logos_website_builds_deploys_and_serves() {
    let dir = tempfile::tempdir().unwrap();
    let out = common::largo_in(dir.path(), &["new", "notes", "--web"]);
    assert!(
        out.status.success(),
        "largo new --web: {}",
        common::stderr(&out)
    );
    let root = dir.path().join("notes");
    std::fs::write(root.join("src/shared.lg"), SHARED).unwrap();
    std::fs::write(root.join("src/main.lg"), BACKEND).unwrap();
    std::fs::write(root.join("pages/app.lg"), PAGE).unwrap();

    // The cheap gate first — it must agree with the expensive one.
    let checked = common::largo_in(&root, &["check"]);
    assert!(
        checked.status.success(),
        "largo check on a real site:\n{}",
        common::stderr(&checked)
    );

    let built = common::largo_in(&root, &["build", "--release"]);
    assert!(
        built.status.success(),
        "largo build --release:\n{}",
        common::stderr(&built)
    );
    let artifact = root.join("target/release/notes");
    assert!(
        artifact.exists(),
        "the build must leave a runnable binary at a stable path"
    );

    // ── Serve it, from a DIFFERENT directory, so nothing depends on the source tree ──
    let spawn_dir = tempfile::tempdir().unwrap();
    let mut site = Command::new(&artifact);
    site.env("PORT", "0").current_dir(spawn_dir.path());
    let server = common::http::Server::spawn(site);
    let port = server.port();

    // The page reads BEFORE any script runs, and loads a content-addressed module.
    let index = get(port, "/");
    assert!(
        index.starts_with(b"HTTP/1.1 200 OK\r\n"),
        "GET /:\n{}",
        String::from_utf8_lossy(&index)
    );
    assert!(
        has(&index, b"Content-Type: text/html"),
        "the front page is HTML"
    );
    let page = body_of(&index);
    assert!(page.contains(".js"), "the page loads its module:\n{page}");
    // ★ AND IT CARRIES ITS CONTENT. This assertion was written the other way round one session
    // ago — "not pre-rendered, and the build says so" — on a diagnosis that turned out to be
    // wrong. `bind` and `class:` were never the cause; the page rendered nothing because a
    // `## Page` never mounted at all, on any tier, and the tree-walker error that produced made
    // the pre-render bail. A page mounts itself now, so this is the flip that comment predicted.
    assert!(
        page.contains("Notes"),
        "the page's own heading is IN the document:\n{page}"
    );
    assert!(
        page.contains("Nothing yet."),
        "and its empty-list arm, rendered from real state:\n{page}"
    );
    assert!(
        !common::stderr(&built).contains("not pre-rendered"),
        "and nothing warns that it ships a shell:\n{}",
        common::stderr(&built)
    );

    // ── The API, on the deployed binary ──────────────────────────────────────
    assert_eq!(
        body_of(&get(port, "/api/notes")).trim(),
        "[]",
        "a fresh store lists nothing"
    );

    let one = post(port, "/api/notes", "title=buy%20milk");
    assert!(
        has(&one, b"application/json"),
        "a route answering a record answers JSON"
    );
    assert_eq!(
        body_of(&one),
        "[{\"id\":1,\"title\":\"buy milk\",\"pinned\":false}]",
        "the derived encoder puts the record on the wire"
    );

    post(port, "/api/notes", "title=write%20tests");
    assert_eq!(
        body_of(&get(port, "/api/notes")),
        "[{\"id\":1,\"title\":\"buy milk\",\"pinned\":false},{\"id\":2,\"title\":\"write tests\",\"pinned\":false}]",
        "both adds persisted ACROSS REQUESTS — the handler is stateless, the store is not"
    );

    // A path parameter, and the one route whose answer CHANGES a row.
    assert_eq!(
        body_of(&post(port, "/api/notes/pin/2", "")),
        "[{\"id\":1,\"title\":\"buy milk\",\"pinned\":false},{\"id\":2,\"title\":\"write tests\",\"pinned\":true}]",
        "`:id` bound from the PATH, and the Bool flipped on that row alone"
    );

    // A bare scalar answer — `2` is valid JSON unquoted, and must be typed as JSON.
    let count = get(port, "/api/notes/count");
    assert_eq!(
        body_of(&count).trim(),
        "2",
        "a route may answer a plain number"
    );
    assert!(
        has(&count, b"application/json"),
        "and it is typed as JSON, not prose"
    );

    // Static assets ride inside the binary; an unmatched path is a clean 404.
    let css = get(port, "/style.css");
    assert!(
        css.starts_with(b"HTTP/1.1 200 OK\r\n"),
        "the stylesheet ships inside the binary"
    );
    assert!(has(&css, b"text/css"), "and is typed");
    assert!(
        get(port, "/nope").starts_with(b"HTTP/1.1 404"),
        "an unmatched path is a 404"
    );
}
