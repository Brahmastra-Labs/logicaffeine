//! One page in the browser: its target, the events it has produced, and the verbs that drive it.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use logicaffeine_compile::semantics::world::JournalEntry;

use super::bundle::PageBundle;
use super::cdp::{SessionSink, Wire};
use super::server::{RelayHub, StaticServer};
use super::snapshot::Snapshot;
use super::{BrowserError, COMMAND_TIMEOUT, PAGE_WORK_TIMEOUT};

/// The recorder every page runs before its own scripts.
const RECORDER: &str = include_str!("recorder.js");

/// The binding the recorder reports through; its calls arrive as `Runtime.bindingCalled` and are
/// kept by the page's sink, so they survive the document that made them.
const REPORT_BINDING: &str = "__logosHarnessReport";

/// What the browser is served at a document address outside the page's bundle — another page
/// file's, whose content is outside the test: an empty document.
const ANOTHER_DOCUMENT: &str =
    "<!doctype html>\n<html>\n<head>\n</head>\n<body>\n</body>\n</html>\n";

/// The most virtual time a document's load may take on the page's clock — timers are held until the
/// page arrives, so this moves only what the page reads as the time of day.
const LOAD_CLOCK_MS: u64 = 1000;

/// How far the page's clock moves each time the harness lets the page run the work a paused clock
/// holds (see [`Page::run_held_tasks`]): one microsecond, the least Chrome's budget can say, below
/// every clock a page reads.
const HELD_TASKS_CLOCK_MS: f64 = 0.001;

/// How long a page may take to load and run its module's `main`.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// Setup applied before the page's own scripts run.
#[derive(Clone, Debug, Default)]
pub struct OpenOptions {
    /// Scripts evaluated in every new document after the harness recorder and before the page.
    pub init_scripts: Vec<String>,
    /// Answers in place before the page loads, so a request the page makes while it loads is
    /// answered exactly as one it makes later would be.
    pub answers: Vec<Answer>,
    /// The relay the page's `/_relay` reaches. Pages opened with clones of one hub hear each
    /// other's publishes; by default a page has a relay of its own.
    pub relay: RelayHub,
}

/// Which way a traced call crossed the host boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CallKind {
    /// The module called a host function (`env.*`).
    Import,
    /// The host called into the module.
    Export,
}

/// One call across the host boundary, as the recorder saw it. Arguments and results are the
/// decimal form of the raw wasm values (a Text argument is its handle).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostCall {
    pub kind: CallKind,
    pub name: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub result: Option<String>,
    /// How many exports were running when this call was made — 0 for an entry from the host.
    pub depth: u32,
    #[serde(default)]
    pub threw: Option<String>,
    /// The decoded Text arguments, in order, of a call whose Text is read the moment it is made
    /// (`dom_note_turn`'s action name, `dom_publish`'s topic and message) — `args` stays the raw
    /// handles either way.
    #[serde(default)]
    pub texts: Vec<String>,
}

/// One console message the page logged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleLine {
    pub level: String,
    pub text: String,
}

/// The renderer's live object counts after a full garbage collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DomCounters {
    pub documents: u64,
    pub nodes: u64,
    pub listeners: u64,
}

/// An answer for a request the page may make: `method path` answered with `status`, a
/// `content-type` and `body`. `path` is relative to the page's origin (`/api/todo`) or a full URL
/// for another origin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub method: String,
    pub path: String,
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

/// One request in the page's own request table, as the page asked for it at the host boundary:
/// its method, URL, body and headers as the page wrote them, and the timeout and abort it later
/// asked for.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct PageRequest {
    pub method: String,
    pub url: String,
    pub body: String,
    /// `[name, value, name, value, …]`, as the page hands them over.
    pub headers: Vec<String>,
    pub timeout: i64,
    pub aborted: bool,
}

/// A file a drop handed the page, as the browser's own `File` describes it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct DroppedFile {
    pub name: String,
    pub size: u64,
    #[serde(rename = "type")]
    pub mime_type: String,
}

/// A request the page made that nothing has answered yet.
struct Held {
    request_id: String,
    method: String,
    url: String,
}

/// How the browser's request for a document of the page's own frame is met.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Admission {
    /// The harness asked for the document (opening the page, reloading it): let through at once.
    Harness,
    /// The page asked for it: held until the document that asked has reported its leaving, so
    /// nothing that document reported on its way out is lost when the browser replaces it.
    Page,
}

/// What a parked document request is answered with once it is let through.
enum Reply {
    Continue,
    Fulfill(Answer),
}

/// A request for a document of the page's own frame, parked until the leaving document's report.
struct Parked {
    request_id: String,
    reply: Reply,
}

/// The default execution context of a document: its id within the session, which is how
/// `Runtime.bindingCalled` names the context a report came from, and its unique id, which is how a
/// `Runtime.evaluate` is pinned to the document.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    id: u64,
    unique_id: String,
}

/// A navigation the browser has started for the page's frame and not yet committed or abandoned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InFlight {
    /// Whether it walks the history (back, forward, a restore): the browser starts fetching such a
    /// document before the leaving document is told, so its request cannot wait on a report.
    traversal: bool,
}

/// The page's own frame, as its sink follows it: the document it holds and the navigation the
/// page has asked the browser for. Written on the reader thread as the browser reports; read by
/// the harness thread deciding what to do next.
struct Frame {
    /// The default execution context of the document the frame holds.
    context: Option<Context>,
    /// How many documents the frame has committed since the page was attached.
    commits: u32,
    /// `loads` when the last document committed: that document has loaded once `loads` passes it.
    loads_at_commit: u32,
    /// Whether the page itself asked for the document the frame holds — a link it followed, a
    /// form it submitted, a `location` it assigned — rather than the harness opening or reloading.
    page_asked: bool,
    in_flight: Option<InFlight>,
    /// Whether the document the frame holds has asked to be replaced — a link followed, a
    /// `location` assigned, a submission's navigation begun — and the browser has not yet
    /// started fetching the replacement.
    requested: bool,
    /// Whether a form's submission is scheduled on a task the paused clock holds.
    planned: bool,
    /// Leaving reports made by the document the frame holds that no document request has
    /// consumed yet.
    leaving_reports: u32,
    /// Document requests parked until the leaving report that lets each through.
    parked: Vec<Parked>,
    admission: Admission,
}

impl Default for Frame {
    fn default() -> Frame {
        Frame {
            context: None,
            commits: 0,
            loads_at_commit: 0,
            page_asked: false,
            in_flight: None,
            requested: false,
            planned: false,
            leaving_reports: 0,
            parked: Vec::new(),
            admission: Admission::Harness,
        }
    }
}

impl Frame {
    /// Whether the document the frame holds has asked to be replaced and the browser has not yet
    /// replaced it or given up.
    fn leaving(&self) -> bool {
        self.requested || self.in_flight.is_some()
    }

    /// Whether the document the frame holds has loaded.
    fn loaded(&self, loads: u32) -> bool {
        loads > self.loads_at_commit
    }

    /// Whether a request for a document of the frame is let through now: one the harness asked
    /// for, one continuing a redirect the browser is following, one walking the history, or one a
    /// leaving report has already cleared the way for.
    fn admits(&mut self, redirect: bool) -> bool {
        if self.admission == Admission::Harness
            || redirect
            || self.in_flight.as_ref().is_some_and(|n| n.traversal)
        {
            return true;
        }
        if self.leaving_reports > 0 {
            self.leaving_reports -= 1;
            return true;
        }
        false
    }

    /// A new document has committed in the frame: the navigation that brought it is over, and the
    /// document that left has nothing more to report.
    fn committed(&mut self, loads: u32) {
        self.commits += 1;
        self.loads_at_commit = loads;
        self.page_asked = self.admission == Admission::Page;
        self.admission = Admission::Page;
        self.resolved();
    }

    /// The navigation in flight is over — committed, or abandoned by the browser with no document.
    fn resolved(&mut self) {
        self.in_flight = None;
        self.requested = false;
        self.planned = false;
        self.leaving_reports = 0;
        self.parked.clear();
    }
}

/// Whether a `Page.frameRequestedNavigation` reason is a form's submission — reported when the
/// submission is scheduled, before the task that navigates has run.
fn form_submission(reason: &Value) -> bool {
    matches!(
        reason.as_str(),
        Some("formSubmissionGet" | "formSubmissionPost")
    )
}

#[derive(Default)]
struct SinkState {
    stubs: Vec<Answer>,
    held: Vec<Held>,
    requests: Vec<(String, String)>,
    console: Vec<ConsoleLine>,
    exceptions: Vec<String>,
    navigations: Vec<String>,
    navigation_requests: Vec<String>,
    submissions: Vec<(Option<String>, bool)>,
    /// Every turn the page noted, across every document of the visit.
    turns: Vec<JournalEntry>,
    loads: u32,
    budgets_expired: u32,
    crashed: bool,
    frame: Frame,
}

struct PageSink {
    session: String,
    /// The page's own frame: the target's main frame, whose id is the target's.
    frame: String,
    origin: String,
    bundle_paths: HashSet<String>,
    state: Mutex<SinkState>,
    changed: Condvar,
}

impl PageSink {
    /// The request path relative to the page's origin, or `None` for another origin.
    fn local_path<'u>(&self, url: &'u str) -> Option<&'u str> {
        let rest = url.strip_prefix(&self.origin)?;
        Some(if rest.is_empty() { "/" } else { rest })
    }

    fn stub_matches(&self, stub: &Answer, method: &str, url: &str) -> bool {
        if !stub.method.eq_ignore_ascii_case(method) {
            return false;
        }
        match self.local_path(url) {
            Some(path) => {
                stub.path == path || stub.path == path.split(['?', '#']).next().unwrap_or(path)
            }
            None => stub.path == url,
        }
    }

    fn fulfill(&self, wire: &Wire, request_id: &str, stub: &Answer) {
        wire.send_detached(
            Some(&self.session),
            "Fetch.fulfillRequest",
            json!({
                "requestId": request_id,
                "responseCode": stub.status,
                "responseHeaders": [{ "name": "content-type", "value": stub.content_type }],
                "body": base64::engine::general_purpose::STANDARD.encode(&stub.body),
            }),
        );
    }

    fn reply(&self, wire: &Wire, request_id: &str, reply: &Reply) {
        match reply {
            Reply::Continue => wire.send_detached(
                Some(&self.session),
                "Fetch.continueRequest",
                json!({ "requestId": request_id }),
            ),
            Reply::Fulfill(answer) => self.fulfill(wire, request_id, answer),
        }
    }

    fn own_frame(&self, id: &Value) -> bool {
        id.as_str() == Some(self.frame.as_str())
    }
}

impl SessionSink for PageSink {
    fn event(&self, wire: &Wire, method: &str, params: &Value) {
        let mut st = self.state.lock().unwrap();
        match method {
            "Fetch.requestPaused" => {
                let request_id = params["requestId"].as_str().unwrap_or_default().to_string();
                let url = params["request"]["url"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let verb = params["request"]["method"]
                    .as_str()
                    .unwrap_or("GET")
                    .to_string();
                let in_bundle = self
                    .local_path(&url)
                    .map(|p| p.split(['?', '#']).next().unwrap_or(p))
                    .is_some_and(|p| self.bundle_paths.contains(p));
                let document = params["resourceType"] == "Document";
                let reply = if in_bundle {
                    Some(Reply::Continue)
                } else if let Some(stub) = st
                    .stubs
                    .iter()
                    .rev()
                    .find(|s| self.stub_matches(s, &verb, &url))
                    .cloned()
                {
                    st.requests.push((verb.clone(), url.clone()));
                    Some(Reply::Fulfill(stub))
                } else if document {
                    Some(Reply::Fulfill(Answer {
                        method: verb,
                        path: url,
                        status: 200,
                        content_type: "text/html; charset=utf-8".to_string(),
                        body: ANOTHER_DOCUMENT.as_bytes().to_vec(),
                    }))
                } else {
                    st.requests.push((verb.clone(), url.clone()));
                    st.held.push(Held {
                        request_id: request_id.clone(),
                        method: verb,
                        url,
                    });
                    None
                };
                if let Some(reply) = reply {
                    let own_document = document && self.own_frame(&params["frameId"]);
                    let redirect = params["redirectedRequestId"].is_string();
                    if !own_document || st.frame.admits(redirect) {
                        self.reply(wire, &request_id, &reply);
                    } else {
                        st.frame.parked.push(Parked { request_id, reply });
                    }
                }
            }
            "Runtime.consoleAPICalled" => {
                let text = params["args"]
                    .as_array()
                    .map(|args| {
                        args.iter()
                            .map(|a| match &a["value"] {
                                Value::String(s) => s.clone(),
                                Value::Null => a["description"].as_str().unwrap_or("").to_string(),
                                v => v.to_string(),
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                let level = params["type"].as_str().unwrap_or("log").to_string();
                st.console.push(ConsoleLine { level, text });
            }
            "Runtime.exceptionThrown" => {
                let d = &params["exceptionDetails"];
                let text = d["exception"]["description"]
                    .as_str()
                    .or_else(|| d["text"].as_str())
                    .unwrap_or("an exception with no description")
                    .to_string();
                st.exceptions.push(text);
            }
            "Runtime.executionContextCreated" => {
                let context = &params["context"];
                let aux = &context["auxData"];
                if aux["isDefault"].as_bool() == Some(true) && self.own_frame(&aux["frameId"]) {
                    if let (Some(id), Some(unique_id)) =
                        (context["id"].as_u64(), context["uniqueId"].as_str())
                    {
                        st.frame.context = Some(Context {
                            id,
                            unique_id: unique_id.to_string(),
                        });
                    }
                }
            }
            "Page.frameNavigated" => {
                if !params["frame"]["parentId"].is_null() {
                    return;
                }
                st.navigations.push(
                    params["frame"]["url"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                );
                let loads = st.loads;
                st.frame.committed(loads);
            }
            "Page.frameStartedNavigating" if self.own_frame(&params["frameId"]) => {
                let kind = params["navigationType"].as_str().unwrap_or_default();
                if matches!(kind, "sameDocument" | "historySameDocument") {
                    return;
                }
                st.frame.in_flight = Some(InFlight {
                    traversal: kind.starts_with("history") || kind.starts_with("restore"),
                });
                st.frame.planned = false;
            }
            "Page.frameRequestedNavigation" => {
                st.navigation_requests
                    .push(params["url"].as_str().unwrap_or_default().to_string());
                if self.own_frame(&params["frameId"]) && params["disposition"] == "currentTab" {
                    if form_submission(&params["reason"]) {
                        st.frame.planned = true;
                    } else {
                        st.frame.requested = true;
                    }
                }
            }
            "Page.frameScheduledNavigation"
                if self.own_frame(&params["frameId"]) && form_submission(&params["reason"]) =>
            {
                st.frame.requested = true;
                st.frame.planned = false;
            }
            "Page.frameStoppedLoading"
                if self.own_frame(&params["frameId"]) && st.frame.in_flight.is_some() =>
            {
                st.frame.resolved();
            }
            "Runtime.bindingCalled" if params["name"] == REPORT_BINDING => {
                let report: Value = params["payload"]
                    .as_str()
                    .and_then(|p| serde_json::from_str(p).ok())
                    .unwrap_or(Value::Null);
                if let Some(sub) = report.get("submit") {
                    st.submissions.push((
                        sub["form"].as_str().map(str::to_string),
                        sub["prevented"].as_bool() == Some(true),
                    ));
                }
                if let Some(turn) = report.get("turn") {
                    st.turns.push(journal_entry(turn));
                }
                let from_here = report.get("leaving").is_some()
                    && st
                        .frame
                        .context
                        .as_ref()
                        .is_some_and(|c| params["executionContextId"].as_u64() == Some(c.id));
                if from_here {
                    if st.frame.parked.is_empty() {
                        st.frame.leaving_reports += 1;
                    } else {
                        let parked = st.frame.parked.remove(0);
                        self.reply(wire, &parked.request_id, &parked.reply);
                    }
                }
            }
            "Page.loadEventFired" => st.loads += 1,
            "Emulation.virtualTimeBudgetExpired" => st.budgets_expired += 1,
            "Inspector.targetCrashed" => st.crashed = true,
            _ => return,
        }
        drop(st);
        self.changed.notify_all();
    }
}

/// The document the harness has arrived at: the context its looks are pinned to, and the commit
/// that brought the document.
#[derive(Clone, Debug)]
struct Here {
    context: String,
    commits: u32,
}

/// What a look at a document answered: its value, or that the document is gone — replaced by the
/// one the page asked for, or being replaced — so the look ran nowhere.
enum Looked {
    Value(Value),
    Gone,
}

/// How a settle in one document ended: with nothing left in flight there, or with that document
/// gone under it.
enum Settled {
    Here,
    Gone,
}

/// The browser's answers to a look at a document that has been replaced: for a look it had already
/// handed the document when it readied the next one; for a look it held back until the next
/// document had committed, where the context the look is pinned to is unknown, or known and
/// destroyed; and for a look still awaiting a promise of the document when the document went.
fn document_gone(message: &str) -> bool {
    message.contains("Inspected target navigated or closed")
        || message.contains("uniqueContextId not found")
        || message.contains("Cannot find context with specified id")
        || message.contains("Execution context was destroyed")
}

/// An open page. Closed, with its browser context, when dropped.
pub struct Page {
    wire: Arc<Wire>,
    session: String,
    target: String,
    context: String,
    sink: Arc<PageSink>,
    server: StaticServer,
    /// Where the files this page is handed by a drop live on disk — the browser reads a dropped
    /// file from the file itself, whenever the page asks for its content — removed with the page.
    drops: PathBuf,
    dropped: AtomicUsize,
    here: Mutex<Option<Here>>,
}

/// A turn the recorder reported from `noteTurn`'s four arguments — `{ action, instance, changes:
/// [field, before, after, …], failure }` — as the journal entry the in-process hosts write.
fn journal_entry(turn: &Value) -> JournalEntry {
    let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let flat: Vec<String> = turn["changes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(text)
        .collect();
    let failure = text(&turn["failure"]);
    JournalEntry {
        instance: text(&turn["instance"]),
        action: text(&turn["action"]),
        changes: flat
            .chunks(3)
            .map(|c| {
                (
                    c[0].clone(),
                    c.get(1).cloned().unwrap_or_default(),
                    c.get(2).cloned().unwrap_or_default(),
                )
            })
            .collect(),
        failed: (!failure.is_empty()).then_some(failure),
    }
}

fn string_field(v: &Value, field: &str, method: &str) -> Result<String, BrowserError> {
    v[field]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| BrowserError::Protocol {
            method: method.to_string(),
            message: format!("the reply carried no `{field}`: {v}"),
        })
}

impl Page {
    pub(crate) fn open(
        wire: Arc<Wire>,
        bundle: &PageBundle,
        options: &OpenOptions,
    ) -> Result<Page, BrowserError> {
        let server = StaticServer::start(bundle.served(), options.relay.clone())
            .map_err(|e| BrowserError::Setup(format!("the page server did not start: {e}")))?;
        let reply = wire.send(
            None,
            "Target.createBrowserContext",
            json!({ "disposeOnDetach": true }),
            COMMAND_TIMEOUT,
        )?;
        let context = string_field(&reply, "browserContextId", "Target.createBrowserContext")?;
        let reply = wire.send(
            None,
            "Target.createTarget",
            json!({ "url": "about:blank", "browserContextId": context }),
            COMMAND_TIMEOUT,
        )?;
        let target = string_field(&reply, "targetId", "Target.createTarget")?;
        let reply = wire.send(
            None,
            "Target.attachToTarget",
            json!({ "targetId": target, "flatten": true }),
            COMMAND_TIMEOUT,
        )?;
        let session = string_field(&reply, "sessionId", "Target.attachToTarget")?;
        let sink = Arc::new(PageSink {
            session: session.clone(),
            frame: target.clone(),
            origin: server.origin().to_string(),
            bundle_paths: bundle.paths(),
            state: Mutex::new(SinkState {
                stubs: options
                    .answers
                    .iter()
                    .map(|a| Answer {
                        method: a.method.to_ascii_uppercase(),
                        ..a.clone()
                    })
                    .collect(),
                ..SinkState::default()
            }),
            changed: Condvar::new(),
        });
        wire.attach(&session, sink.clone());
        let drops =
            std::env::temp_dir().join(format!("logos-page-drops-{}-{target}", std::process::id()));
        let page = Page {
            wire,
            session,
            target,
            context,
            sink,
            server,
            drops,
            dropped: AtomicUsize::new(0),
            here: Mutex::new(None),
        };

        page.command("Page.enable", json!({}))?;
        page.command("Runtime.enable", json!({}))?;
        page.command(
            "Fetch.enable",
            json!({ "patterns": [{ "urlPattern": "*" }] }),
        )?;
        page.command(
            "Emulation.setFocusEmulationEnabled",
            json!({ "enabled": true }),
        )?;
        page.wire.send(
            None,
            "Browser.setDownloadBehavior",
            json!({ "behavior": "deny", "browserContextId": page.context }),
            COMMAND_TIMEOUT,
        )?;
        page.command("Runtime.addBinding", json!({ "name": REPORT_BINDING }))?;
        page.command(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": RECORDER }),
        )?;
        for script in &options.init_scripts {
            page.command(
                "Page.addScriptToEvaluateOnNewDocument",
                json!({ "source": script }),
            )?;
        }

        let url = format!("{}{}", page.server.origin(), bundle.entry());
        let nav = page.command("Page.navigate", json!({ "url": url }))?;
        if let Some(err) = nav["errorText"].as_str() {
            return Err(BrowserError::Page(format!("loading {url} failed: {err}")));
        }
        page.settle()?;
        Ok(page)
    }

    fn frame<T>(&self, read: impl FnOnce(&Frame) -> T) -> T {
        read(&self.sink.state.lock().unwrap().frame)
    }

    /// Whether the frame has committed a document the harness has not arrived at.
    fn behind(&self) -> bool {
        let commits = self.frame(|f| f.commits);
        self.here
            .lock()
            .unwrap()
            .as_ref()
            .map_or(true, |h| commits > h.commits)
    }

    /// Follow the page to where it is. While the document the harness is in has asked to be
    /// replaced, wait for the replacement ([`Page::leave`]); while a document has committed that
    /// the harness has not arrived at, arrive there ([`Page::arrive`]) — until the harness is in a
    /// document that has asked for nothing. Every act and every read starts here, so none is made
    /// in a document the page has left.
    ///
    /// Knowing is ordered, not timed. The browser reports what it starts (`Page.frameStartedNavigating`,
    /// before it holds any command back for the navigation) and the renderer reports what the
    /// page asks for (`Page.frameRequestedNavigation`, from inside the page's own task, ahead of
    /// its reply to any later command). What neither has reported when the harness looks, the
    /// look itself settles: it is pinned to the document it was meant for ([`Page::look`]), so a
    /// look the browser held back for a navigation and handed the next document answers that the
    /// document is gone, never a half-built document's contents.
    fn catch_up(&self) -> Result<(), BrowserError> {
        loop {
            if self.frame(Frame::leaving) {
                self.leave()?;
            } else if self.behind() {
                self.arrive()?;
            } else {
                return Ok(());
            }
        }
    }

    /// Wait for the document the page asked for to commit, or for the browser to abandon the
    /// navigation with no document (a `204`, a download), which leaves the page where it is. The
    /// sink lets the document in once the leaving document has reported (`PageSink::event`): the
    /// browser drops whatever a leaving document has not yet delivered when it readies the next
    /// document's commit, and here there is no next document until the last has said everything.
    fn leave(&self) -> Result<(), BrowserError> {
        let commits = self.frame(|f| f.commits);
        self.wait_for_sink(
            "the document the page asked for to commit, or the browser to abandon that navigation",
            START_TIMEOUT,
            |s| s.frame.commits > commits || !s.frame.leaving(),
        )
    }

    /// Arrive at the document the frame committed last: wait for it to load and, when it is a
    /// page, for its module to run `main`, then take the clock — the moment the document has
    /// arrived, however long the load took. A document the page went to (or reloaded) loads with
    /// its clock running only while no request is in flight, for at most [`LOAD_CLOCK_MS`]: paused,
    /// a document never reaches its load event; left running freely, its clock would move by
    /// however long the load happened to take. The first document loads before the clock is taken.
    fn arrive(&self) -> Result<(), BrowserError> {
        let first = self.here.lock().unwrap().is_none();
        let mut commits = self.frame(|f| f.commits);
        if !first {
            self.let_load()?;
        }
        let context = loop {
            self.wait_for_sink("the page's load event", START_TIMEOUT, |s| {
                s.frame.commits > commits || s.frame.loaded(s.loads)
            })?;
            let st = self.sink.state.lock().unwrap();
            if st.frame.commits > commits {
                commits = st.frame.commits;
                drop(st);
                self.let_load()?;
                continue;
            }
            let Some(context) = st.frame.context.clone() else {
                return Err(BrowserError::Page(
                    "the document committed with no execution context of its own".to_string(),
                ));
            };
            break context;
        };
        *self.here.lock().unwrap() = Some(Here {
            context: context.unique_id,
            commits,
        });
        let is_page = match self.look(
            "document.querySelector('script[type=\"module\"]') !== null",
            COMMAND_TIMEOUT,
        )? {
            Looked::Gone => return Ok(()),
            Looked::Value(v) => v.as_bool() == Some(true),
        };
        if is_page {
            let started = self
                .look("__logosHarness.settled()", START_TIMEOUT)
                .map_err(|e| {
                    self.with_page_log(e, "waiting for the page's module to run `main`")
                })?;
            let started = match started {
                Looked::Gone => return Ok(()),
                Looked::Value(v) => v,
            };
            if started.as_str() != Some("returned") {
                let why = match self.look("__logosHarness.mainError", COMMAND_TIMEOUT) {
                    Ok(Looked::Value(v)) => v,
                    _ => Value::Null,
                };
                return Err(self.with_page_log(
                    BrowserError::Page(format!(
                        "the page's module did not start: `main` {started} {why}"
                    )),
                    "starting the page",
                ));
            }
        }
        self.command(
            "Emulation.setVirtualTimePolicy",
            json!({ "policy": "pause" }),
        )?;
        self.look("__logosHarness.openGate()", COMMAND_TIMEOUT)?;
        Ok(())
    }

    /// Let the document that just committed load: its clock runs only while no request is in
    /// flight, for at most [`LOAD_CLOCK_MS`].
    fn let_load(&self) -> Result<(), BrowserError> {
        self.command(
            "Emulation.setVirtualTimePolicy",
            json!({ "policy": "pauseIfNetworkFetchesPending", "budget": LOAD_CLOCK_MS }),
        )?;
        Ok(())
    }

    /// After a look found its document gone: wait for the sink to have the navigation that
    /// replaced it — the commit, or the page's request for the replacement — before following.
    fn await_departure(&self) -> Result<(), BrowserError> {
        let commits = self.here.lock().unwrap().as_ref().map_or(0, |h| h.commits);
        self.wait_for_sink(
            "the page to leave the document it asked to leave",
            START_TIMEOUT,
            |s| s.frame.commits > commits || s.frame.leaving(),
        )
    }

    /// Carry out one of a visitor's acts on the page at rest, then settle. The page settles first,
    /// so every message already written to it is handled before the act: a click is aimed where an
    /// element is drawn and pressed a command later, and a message handled between the two could
    /// repaint the page and press whatever then stands there. Afterwards it settles in the document
    /// the page is in, and, when the act takes the browser to another document (a link followed, a
    /// form submitted, `location.assign`), in the document the page comes to rest in: the page that
    /// acted is gone.
    fn act(&self, input: impl FnOnce() -> Result<(), BrowserError>) -> Result<(), BrowserError> {
        self.settle()?;
        input()?;
        self.settle()
    }

    /// Send `method`: an input event within [`PAGE_WORK_TIMEOUT`], since it returns once the page has handled it, and
    /// any other command within [`COMMAND_TIMEOUT`].
    fn command(&self, method: &str, params: Value) -> Result<Value, BrowserError> {
        let timeout = if method.starts_with("Input.") {
            PAGE_WORK_TIMEOUT
        } else {
            COMMAND_TIMEOUT
        };
        self.wire.send(Some(&self.session), method, params, timeout)
    }

    /// Attach what the page logged and threw to an error, so a failure explains itself.
    fn with_page_log(&self, err: BrowserError, doing: &str) -> BrowserError {
        let st = self.sink.state.lock().unwrap();
        let console: Vec<String> = st
            .console
            .iter()
            .map(|c| format!("  [{}] {}", c.level, c.text))
            .collect();
        let thrown: Vec<String> = st.exceptions.iter().map(|e| format!("  {e}")).collect();
        BrowserError::Page(format!(
            "{err} ({doing})\nconsole:\n{}\nexceptions:\n{}",
            if console.is_empty() {
                "  (none)".to_string()
            } else {
                console.join("\n")
            },
            if thrown.is_empty() {
                "  (none)".to_string()
            } else {
                thrown.join("\n")
            },
        ))
    }

    fn wait_for_sink(
        &self,
        what: &str,
        timeout: Duration,
        done: impl Fn(&SinkState) -> bool,
    ) -> Result<(), BrowserError> {
        let deadline = Instant::now() + timeout;
        let mut st = self.sink.state.lock().unwrap();
        loop {
            if st.crashed {
                return Err(BrowserError::Page(format!(
                    "the page crashed while waiting for {what}"
                )));
            }
            if done(&st) {
                return Ok(());
            }
            if let Some(why) = self.wire.closed() {
                return Err(BrowserError::Closed(why));
            }
            let now = Instant::now();
            if now >= deadline {
                drop(st);
                return Err(self.with_page_log(
                    BrowserError::Timeout {
                        what: what.to_string(),
                        after: timeout,
                    },
                    "waiting on the page",
                ));
            }
            st = self
                .sink
                .changed
                .wait_timeout(st, (deadline - now).min(Duration::from_millis(250)))
                .unwrap()
                .0;
        }
    }

    /// Evaluate `js` in the page (awaiting a promise it returns) and hand back its JSON value —
    /// in the document the page is in, once it has arrived there.
    pub fn eval(&self, js: &str) -> Result<Value, BrowserError> {
        self.read(js)
    }

    /// Evaluate `js` in the document the page is in: after following the page to it, and again in
    /// the next document should this one be replaced under the look.
    fn read(&self, js: &str) -> Result<Value, BrowserError> {
        loop {
            self.catch_up()?;
            match self.look(js, COMMAND_TIMEOUT)? {
                Looked::Value(v) => return Ok(v),
                Looked::Gone => self.await_departure()?,
            }
        }
    }

    /// Evaluate `js` in the document the harness arrived at last — pinned to that document's own
    /// execution context, so that a look the browser holds back while it replaces the document,
    /// and hands the next one, answers that the document is gone rather than running there.
    fn look(&self, js: &str, timeout: Duration) -> Result<Looked, BrowserError> {
        let context = match self.here.lock().unwrap().as_ref() {
            Some(here) => here.context.clone(),
            None => {
                return Err(BrowserError::Page(
                    "the harness has arrived at no document to look at".to_string(),
                ))
            }
        };
        let reply = self.wire.send(
            Some(&self.session),
            "Runtime.evaluate",
            json!({ "expression": js, "returnByValue": true, "awaitPromise": true, "uniqueContextId": context }),
            timeout,
        );
        let reply = match reply {
            Err(BrowserError::Protocol { message, .. }) if document_gone(&message) => {
                return Ok(Looked::Gone)
            }
            other => other?,
        };
        if let Some(d) = reply.get("exceptionDetails") {
            let text = d["exception"]["description"]
                .as_str()
                .or_else(|| d["text"].as_str())
                .unwrap_or("threw");
            return Err(BrowserError::Page(format!(
                "{text}\n  (evaluating `{}`)",
                short(js)
            )));
        }
        Ok(Looked::Value(reply["result"]["value"].clone()))
    }

    /// Run one of the recorder's driving helpers in the document the page is in now; `None` when
    /// that document is gone under the act, which then has nothing left to do there. A helper can fire
    /// the page's handlers, so it waits within [`PAGE_WORK_TIMEOUT`].
    fn drive(&self, helper: &str, args: &[&str]) -> Result<Option<Value>, BrowserError> {
        let args: Vec<String> = args
            .iter()
            .map(|a| serde_json::to_string(a).expect("a string serializes"))
            .collect();
        Ok(
            match self.look(
                &format!("__logosHarness.drive.{helper}({})", args.join(", ")),
                PAGE_WORK_TIMEOUT,
            )? {
                Looked::Value(v) => Some(v),
                Looked::Gone => None,
            },
        )
    }

    /// Wait until the page has nothing left in flight that will finish: every request it made is
    /// either complete — its body read, and the completion turn that read it run — or HELD, a
    /// request nothing has answered, which stays pending as it would on a network that has not
    /// replied; and every file it offered for download has been read out of its blob.
    ///
    /// An answered request completes over real time the paused virtual clock does not model: the
    /// harness fulfils it from `Fetch.requestPaused` (see `PageSink::event`) with a command nothing
    /// acknowledges, and the response, the page's `fetch()` resolving and the completion turn then
    /// take however many of the page's own event-loop turns they take. So this reads what is in
    /// flight from the page itself (`__logosHarness.inFlight()`: the recorder's own ledger of every
    /// `fetch` and blob read) and holds each pending request against the held ones, method and URL,
    /// until nothing is pending that is not held. Its relay connections are through too: open,
    /// every message it sent read by the relay, every message the relay wrote to it handled — so
    /// once the page that published settles, a page that heard it settles to having heard it. A page
    /// that never gets there is a failure naming what it is still waiting for.
    ///
    /// Every verb that drives the page begins and ends here, following the page to every document
    /// it asks for on the way (`Page::catch_up`) — a submitted form's navigation, which Chrome
    /// plans on a task the paused clock holds, is let run first — so a settle ends in the document
    /// the page came to rest in. A page nothing drove settles to what reached it from elsewhere —
    /// another page's publish.
    pub fn settle(&self) -> Result<(), BrowserError> {
        loop {
            self.catch_up()?;
            if self.frame(|f| f.planned) {
                self.run_held_tasks()?;
                self.sink.state.lock().unwrap().frame.planned = false;
                continue;
            }
            match self.settle_in_place()? {
                Settled::Here => {
                    if self.frame(|f| f.leaving() || f.planned) || self.behind() {
                        continue;
                    }
                    return Ok(());
                }
                Settled::Gone => self.await_departure()?,
            }
        }
    }

    /// [`Page::settle`] in the document the page is in now. Each look is a task on the page's own
    /// event loop, after every task before it, so the last look — the one that finds nothing left
    /// in flight — answers only after everything the page reported until then has reached the sink.
    fn settle_in_place(&self) -> Result<Settled, BrowserError> {
        let deadline = Instant::now() + START_TIMEOUT;
        // The wait between looks doubles, so a completion still in flight is not crowded out by
        // the questions asked about it.
        let mut pause = Duration::from_millis(1);
        loop {
            let flight = match self.look("__logosHarness.inFlight()", COMMAND_TIMEOUT)? {
                Looked::Value(v) => v,
                Looked::Gone => return Ok(Settled::Gone),
            };
            let mut held = self.pending_requests();
            let mut waiting: Vec<String> = Vec::new();
            for f in flight["fetches"].as_array().into_iter().flatten() {
                let (method, url) = (f[0].as_str().unwrap_or(""), f[1].as_str().unwrap_or(""));
                match held
                    .iter()
                    .position(|(m, u)| m.eq_ignore_ascii_case(method) && u == url)
                {
                    Some(i) => {
                        held.swap_remove(i);
                    }
                    None => waiting.push(format!("{method} {url}")),
                }
            }
            let reads = flight["reads"].as_u64().unwrap_or(0);
            let (relay, opening) = self.relay_in_flight(&flight);
            if waiting.is_empty() && reads == 0 && relay.is_empty() {
                return Ok(Settled::Here);
            }
            if opening {
                self.run_held_tasks()?;
            }
            if Instant::now() >= deadline {
                return Err(self.with_page_log(
                    BrowserError::Timeout {
                        what: format!(
                            "the page's answered requests to complete ({}), {reads} download(s) to be read and its relay to be through ({})",
                            waiting.join(", "),
                            relay.join(", ")
                        ),
                        after: START_TIMEOUT,
                    },
                    "settling the page",
                ));
            }
            std::thread::sleep(pause);
            pause = (pause * 2).min(Duration::from_millis(64));
        }
    }

    /// Let the page run the work it queued on the task queues Chrome fences while the page's clock
    /// is paused — a WebSocket's handshake (a relay connection the page opens after it arrived would
    /// never open), a form's planned navigation (a submitted form would never leave). The clock is
    /// let run until it has moved [`HELD_TASKS_CLOCK_MS`], and pauses again; the clock moves only
    /// once nothing is left to run, so the queued work runs first, and a timer runs in that window
    /// only if it falls due within that microsecond. Messages arrive on a queue the paused clock
    /// does not hold.
    fn run_held_tasks(&self) -> Result<(), BrowserError> {
        let before = self.sink.state.lock().unwrap().budgets_expired;
        self.command(
            "Emulation.setVirtualTimePolicy",
            json!({ "policy": "advance", "budget": HELD_TASKS_CLOCK_MS }),
        )?;
        self.wait_for_sink("the page's clock to pause again", START_TIMEOUT, |s| {
            s.budgets_expired > before
        })
    }

    /// Every relay connection of the page's that is not through, described — one still opening, one
    /// whose messages the relay has not all read, one the relay has written messages to that the page
    /// has not yet handled — and whether one is opening. A message the page publishes reaches every
    /// other page on the relay in the step the relay reads it, so a page through with its relay has
    /// reached every page it publishes to.
    fn relay_in_flight(&self, flight: &Value) -> (Vec<String>, bool) {
        let (mut waiting, mut opening) = (Vec::new(), false);
        for r in flight["relay"].as_array().into_iter().flatten() {
            let token = r["token"].as_str().unwrap_or("");
            if r["connecting"].as_bool() == Some(true) {
                waiting.push(format!("connection {token} opening"));
                opening = true;
                continue;
            }
            let (sent, handled) = (
                r["sent"].as_u64().unwrap_or(0),
                r["handled"].as_u64().unwrap_or(0),
            );
            let (read, written) = self.server.relay().counts(token);
            if read < sent {
                waiting.push(format!(
                    "{} of {sent} sent message(s) unread by the relay",
                    sent - read
                ));
            }
            if handled < written {
                waiting.push(format!(
                    "{} of {written} relayed message(s) unhandled by the page",
                    written - handled
                ));
            }
        }
        (waiting, opening)
    }

    /// Click the element `sel` names, as a visitor would: a trusted press and release at its
    /// centre, so a submit button submits and a checkbox toggles.
    ///
    /// No `mouseMoved` precedes the press. A move is a continuous event the renderer aligns to
    /// its next frame, and with the page's clock paused no frame comes: the move is acknowledged
    /// only by the input router's timeout, seconds later. Press and release are discrete and are
    /// dispatched at once.
    pub fn click(&self, sel: &str) -> Result<(), BrowserError> {
        self.act(|| {
            let Some(at) = self.drive("point", &[sel])? else { return Ok(()) };
            let (x, y) = (at["x"].as_f64().unwrap_or(0.0), at["y"].as_f64().unwrap_or(0.0));
            self.command(
                "Input.dispatchMouseEvent",
                json!({ "type": "mousePressed", "x": x, "y": y, "button": "left", "buttons": 1, "clickCount": 1 }),
            )?;
            self.command(
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseReleased", "x": x, "y": y, "button": "left", "buttons": 0, "clickCount": 1 }),
            )?;
            Ok(())
        })
    }

    /// Type `text` into the control `sel` names, replacing what it held: focus, select all,
    /// trusted text insertion (`input`), then `change` — the events a visitor's typing raises.
    pub fn type_into(&self, sel: &str, text: &str) -> Result<(), BrowserError> {
        self.act(|| {
            let Some(_) = self.drive("focusForTyping", &[sel])? else {
                return Ok(());
            };
            if text.is_empty() {
                let Some(_) = self.drive("clear", &[sel])? else {
                    return Ok(());
                };
            } else {
                self.command("Input.insertText", json!({ "text": text }))?;
            }
            self.drive("fire", &[sel, "change"])?;
            Ok(())
        })
    }

    /// Press `key` on whatever has focus: `"Enter"`, `"Escape"`, `"Tab"`, `"Backspace"`,
    /// `"Delete"`, the arrows, `"Home"`, `"End"`, `"PageUp"`, `"PageDown"`, `"Space"`, or one
    /// printable character. Names are matched without regard to case.
    pub fn press(&self, key: &str) -> Result<(), BrowserError> {
        self.act(|| {
            let spec = KeySpec::named(key).ok_or_else(|| {
                BrowserError::Setup(format!(
                    "`{key}` is not a key this harness can press — it knows {} and single printable characters",
                    NAMED_KEYS.iter().map(|k| format!("`{}`", k.0)).collect::<Vec<_>>().join(", ")
                ))
            })?;
            let mut down = json!({
                "type": if spec.text.is_some() { "keyDown" } else { "rawKeyDown" },
                "key": spec.key,
                "code": spec.code,
                "windowsVirtualKeyCode": spec.vk,
                "nativeVirtualKeyCode": spec.vk,
            });
            if let Some(t) = &spec.text {
                down["text"] = json!(t);
                down["unmodifiedText"] = json!(t);
            }
            self.command("Input.dispatchKeyEvent", down)?;
            self.command(
                "Input.dispatchKeyEvent",
                json!({ "type": "keyUp", "key": spec.key, "code": spec.code, "windowsVirtualKeyCode": spec.vk, "nativeVirtualKeyCode": spec.vk }),
            )?;
            Ok(())
        })
    }

    /// Focus `sel`, then press `key` there.
    pub fn press_in(&self, sel: &str, key: &str) -> Result<(), BrowserError> {
        self.focus(sel)?;
        self.press(key)
    }

    /// Move focus to `sel` (the browser raises `focus`).
    pub fn focus(&self, sel: &str) -> Result<(), BrowserError> {
        self.act(|| {
            self.drive("focus", &[sel])?;
            Ok(())
        })
    }

    /// Take focus off `sel` (the browser raises `blur`).
    pub fn blur(&self, sel: &str) -> Result<(), BrowserError> {
        self.act(|| {
            self.drive("blur", &[sel])?;
            Ok(())
        })
    }

    /// The pointer arriving over `sel`: the `mouseover` a visitor's mouse raises there, dispatched
    /// on the element and bubbling from it. A trusted pointer move is a continuous input the
    /// renderer delivers on its next frame, and with the page's clock paused no frame comes — the
    /// move would reach the page later, whenever the clock next moved — so the event is raised in
    /// the page, at once, as the browser would raise it.
    pub fn hover(&self, sel: &str) -> Result<(), BrowserError> {
        self.act(|| {
            let Some(_) = self.drive("point", &[sel])? else {
                return Ok(());
            };
            self.drive("hover", &[sel])?;
            Ok(())
        })
    }

    /// Pick the option of the `<select>` at `sel` whose value (or else whose text) is `value`,
    /// raising `input` and `change`.
    pub fn choose(&self, sel: &str, value: &str) -> Result<(), BrowserError> {
        self.act(|| {
            self.drive("choose", &[sel, value])?;
            Ok(())
        })
    }

    /// Submit the form `sel` names (or the form a submit button at `sel` belongs to) through
    /// `requestSubmit`, so the page's `submit` handlers run and, unless one prevents it, the
    /// browser performs the submission.
    pub fn submit(&self, sel: &str) -> Result<(), BrowserError> {
        self.act(|| {
            self.drive("submit", &[sel])?;
            Ok(())
        })
    }

    /// Drop a file named `name` holding `content` on the element `sel` names, as a visitor dragging
    /// it there from their desktop does: a trusted drag carrying a real file on disk enters the
    /// element's centre, moves over it and drops. The drop reaches the page only when the element
    /// accepts drops — something on the page cancels `dragover` — exactly as for a visitor, and the
    /// browser then reads the name, size and content from the file itself and names its type from
    /// its name. Answers the file as the browser described it, `None` when no drop reached the
    /// document.
    pub fn drop_file(
        &self,
        sel: &str,
        name: &str,
        content: &[u8],
    ) -> Result<Option<DroppedFile>, BrowserError> {
        if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
            return Err(BrowserError::Setup(format!(
                "{name:?} is not a file name a browser can be handed — a dropped file is named by one \
                 path segment"
            )));
        }
        let dir = self
            .drops
            .join(self.dropped.fetch_add(1, Ordering::SeqCst).to_string());
        std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(dir.join(name), content))
            .map_err(|e| {
                BrowserError::Setup(format!(
                    "the file {name:?} could not be written for the drop: {e}"
                ))
            })?;
        let before = self
            .read("__logosHarness.drops.length")?
            .as_u64()
            .unwrap_or(0) as usize;
        self.act(|| {
            let Some(at) = self.drive("point", &[sel])? else {
                return Ok(());
            };
            let (x, y) = (
                at["x"].as_f64().unwrap_or(0.0),
                at["y"].as_f64().unwrap_or(0.0),
            );
            let path = dir.join(name).to_string_lossy().into_owned();
            let data = json!({ "items": [], "files": [path], "dragOperationsMask": 1 });
            for kind in ["dragEnter", "dragOver", "drop"] {
                self.command(
                    "Input.dispatchDragEvent",
                    json!({ "type": kind, "x": x, "y": y, "data": data }),
                )?;
            }
            Ok(())
        })?;
        let drops: Vec<Option<DroppedFile>> =
            serde_json::from_value(self.read("__logosHarness.drops")?).map_err(|e| {
                BrowserError::Page(format!("the page's drop record did not parse: {e}"))
            })?;
        Ok(drops.into_iter().nth(before).flatten())
    }

    /// The names the page's module exports, as its host sees them.
    pub fn exports(&self) -> Result<Vec<String>, BrowserError> {
        let v = self.read("__logosHarness.exports")?;
        Ok(serde_json::from_value(v).unwrap_or_default())
    }

    /// Mount the component `component` under the element `parent` names (the page's `body` when
    /// `None`), through the mount entry the page's host exposes for it — present only when the page
    /// was built for a host that mounts it on demand ([`PageBundle::page_with`]).
    pub fn mount(&self, component: &str, parent: Option<&str>) -> Result<(), BrowserError> {
        self.act(|| {
            let component = serde_json::to_string(component).expect("a string serializes");
            let parent = serde_json::to_string(&parent).expect("an optional string serializes");
            self.look(
                &format!("__logosMount({component}, {parent})"),
                COMMAND_TIMEOUT,
            )?;
            Ok(())
        })
    }

    /// Reload the page, as a visitor pressing reload does: the document loads again, the module
    /// runs `main` again, and everything the page kept in browser storage is still there.
    pub fn reload(&self) -> Result<(), BrowserError> {
        self.catch_up()?;
        let commits = {
            let mut st = self.sink.state.lock().unwrap();
            st.frame.admission = Admission::Harness;
            st.frame.commits
        };
        self.command("Page.reload", json!({}))?;
        self.wait_for_sink("the reloaded document to commit", START_TIMEOUT, |s| {
            s.frame.commits > commits
        })?;
        self.settle()
    }

    /// Both storage areas as the page left them, `(area, key, value)` — `local` then `session`,
    /// each sorted by key: the shape `semantics::storage` answers in.
    pub fn storage(&self) -> Result<Vec<(String, String, String)>, BrowserError> {
        let v = self.read("__logosHarness.storage()")?;
        serde_json::from_value(v.clone()).map_err(|e| {
            BrowserError::Page(format!(
                "the page's storage did not parse ({e}): {}",
                short(&v.to_string())
            ))
        })
    }

    /// Every frame the page handed its relay, `(topic, message)`, in order — read off the host
    /// boundary, where the page publishes, whatever becomes of the frame after.
    pub fn published(&self) -> Result<Vec<(String, String)>, BrowserError> {
        Ok(self
            .import_trace()?
            .into_iter()
            .filter(|c| c.kind == CallKind::Import && c.name == "dom_publish")
            .filter_map(|c| match c.texts.as_slice() {
                [topic, message] => Some((topic.clone(), message.clone())),
                _ => None,
            })
            .collect())
    }

    /// Every navigation of the document the page is in now, in the reference renderer's spelling
    /// (`push /x`, `replace /y`): the pushState/replaceState moves made in it, led by `load <path>`
    /// when the page navigated to this document itself — the log `semantics::dom` keeps per document.
    pub fn route_ops(&self) -> Result<Vec<String>, BrowserError> {
        let mut ops: Vec<String> = serde_json::from_value(self.read("__logosHarness.routeOps")?)
            .map_err(|e| {
                BrowserError::Page(format!("the page's navigation log did not parse: {e}"))
            })?;
        if self.frame(|f| f.page_asked) {
            let here = self.read("location.pathname + location.search")?;
            ops.insert(0, format!("load {}", here.as_str().unwrap_or_default()));
        }
        Ok(ops)
    }

    /// The page now.
    pub fn snapshot(&self) -> Result<Snapshot, BrowserError> {
        let v = self.read("__logosHarness.snapshot()")?;
        serde_json::from_value(v.clone()).map_err(|e| {
            BrowserError::Page(format!(
                "the page's snapshot did not parse ({e}): {}",
                short(&v.to_string())
            ))
        })
    }

    /// Every call across the host boundary since the page loaded, in order.
    pub fn import_trace(&self) -> Result<Vec<HostCall>, BrowserError> {
        let v = self.read("__logosHarness.trace")?;
        serde_json::from_value(v)
            .map_err(|e| BrowserError::Page(format!("the host-call trace did not parse: {e}")))
    }

    /// How many calls the trace holds — a mark to slice [`Page::import_trace`] from.
    pub fn trace_len(&self) -> Result<usize, BrowserError> {
        Ok(self
            .read("__logosHarness.trace.length")?
            .as_u64()
            .unwrap_or(0) as usize)
    }

    /// Move the page's clock forward `ms` virtual milliseconds, running every timer that falls
    /// due, then settle. The clock does not move otherwise.
    ///
    /// Chrome reports the budget spent as its clock reaches the end and pauses the clock there,
    /// fencing the page's timer queues, so work that falls due at that instant can be left behind
    /// the fence. The advance ends in `run_held_tasks`, whose clock moves only once nothing
    /// is left to run: every task due by the end has run when this returns. A zero advance is that
    /// alone, since Chrome reports no budget spent that it was never given.
    pub fn advance(&self, ms: u64) -> Result<(), BrowserError> {
        self.act(|| {
            if ms > 0 {
                let before = self.sink.state.lock().unwrap().budgets_expired;
                self.command(
                    "Emulation.setVirtualTimePolicy",
                    json!({ "policy": "advance", "budget": ms }),
                )?;
                self.wait_for_sink(
                    &format!("{ms}ms of virtual time to pass"),
                    START_TIMEOUT,
                    |s| s.budgets_expired > before,
                )?;
            }
            self.run_held_tasks()
        })
    }

    /// Answer `method path` with `status` and `body` — now for a request the page is waiting on,
    /// and from now on for every new one. `path` is relative to the page's origin (`/api/todo`)
    /// or a full URL for another origin. Returns once the page has read every answered body.
    pub fn answer(
        &self,
        method: &str,
        path: &str,
        status: u16,
        content_type: &str,
        body: &[u8],
    ) -> Result<(), BrowserError> {
        let stub = Answer {
            method: method.to_ascii_uppercase(),
            path: path.to_string(),
            status,
            content_type: content_type.to_string(),
            body: body.to_vec(),
        };
        self.act(|| {
            let answered: Vec<Held> = {
                let mut st = self.sink.state.lock().unwrap();
                let (now, still): (Vec<Held>, Vec<Held>) = st
                    .held
                    .drain(..)
                    .partition(|h| self.sink.stub_matches(&stub, &h.method, &h.url));
                st.held = still;
                now
            };
            for held in &answered {
                self.command(
                    "Fetch.fulfillRequest",
                    json!({
                        "requestId": held.request_id,
                        "responseCode": stub.status,
                        "responseHeaders": [{ "name": "content-type", "value": stub.content_type }],
                        "body": base64::engine::general_purpose::STANDARD.encode(&stub.body),
                    }),
                )?;
            }
            self.sink.state.lock().unwrap().stubs.push(stub.clone());
            for held in &answered {
                let url = serde_json::to_string(&held.url).expect("a string serializes");
                let read = self
                    .look(
                        &format!("__logosHarness.whenFetched({url})"),
                        COMMAND_TIMEOUT,
                    )
                    .map_err(|e| {
                        self.with_page_log(e, "waiting for the page to read its answer")
                    })?;
                if let Looked::Gone = read {
                    break;
                }
            }
            Ok(())
        })
    }

    /// Requests outside the bundle that nothing has answered, as `(method, url)`.
    pub fn pending_requests(&self) -> Vec<(String, String)> {
        let st = self.sink.state.lock().unwrap();
        st.held
            .iter()
            .map(|h| (h.method.clone(), h.url.clone()))
            .collect()
    }

    /// The page's own request table: every request it made, answered or not, in the order it made
    /// them — what `semantics::dom`'s fetch table holds for the same page in process.
    pub fn page_requests(&self) -> Result<Vec<PageRequest>, BrowserError> {
        let v = self.read("__logosHarness.requests")?;
        serde_json::from_value(v.clone()).map_err(|e| {
            BrowserError::Page(format!(
                "the page's request table did not parse ({e}): {}",
                short(&v.to_string())
            ))
        })
    }

    /// Every request the page made outside its bundle, answered or not, as `(method, url)`.
    pub fn requests(&self) -> Vec<(String, String)> {
        self.sink.state.lock().unwrap().requests.clone()
    }

    /// What the page logged to its console.
    pub fn console(&self) -> Vec<ConsoleLine> {
        self.sink.state.lock().unwrap().console.clone()
    }

    /// Every uncaught exception, as its description.
    pub fn exceptions(&self) -> Vec<String> {
        self.sink.state.lock().unwrap().exceptions.clone()
    }

    /// The URL of every document the main frame has committed, in order.
    pub fn navigations(&self) -> Vec<String> {
        self.sink.state.lock().unwrap().navigations.clone()
    }

    /// Every navigation the page asked the browser for (a link, a form submission, a script),
    /// whether or not it has committed yet.
    pub fn navigation_requests(&self) -> Vec<String> {
        self.sink.state.lock().unwrap().navigation_requests.clone()
    }

    /// Every turn the page noted (`noteTurn`), in order — the journal of the whole visit, across
    /// every document the page has loaded.
    pub fn journal(&self) -> Vec<JournalEntry> {
        self.sink.state.lock().unwrap().turns.clone()
    }

    /// Every form submission the page saw, as `(form path, whether its default action was
    /// prevented)`, in order — across every document the page has loaded.
    pub fn submissions(&self) -> Vec<(Option<String>, bool)> {
        self.sink.state.lock().unwrap().submissions.clone()
    }

    /// The text the page put on the clipboard, in order.
    pub fn clipboard(&self) -> Result<Vec<String>, BrowserError> {
        let v = self.read("__logosHarness.clipboard")?;
        Ok(serde_json::from_value(v).unwrap_or_default())
    }

    /// The title of every notification the page raised, in order.
    pub fn notifications(&self) -> Result<Vec<String>, BrowserError> {
        let v = self.read("__logosHarness.notifications.map((n) => n.title)")?;
        Ok(serde_json::from_value(v).unwrap_or_default())
    }

    /// Every file the page offered for download, as `(name, content)` — the content is `None`
    /// until the browser has read it out of its blob.
    pub fn downloads(&self) -> Result<Vec<(String, Option<String>)>, BrowserError> {
        let v = self.read("__logosHarness.downloads")?;
        Ok(v.as_array()
            .map(|a| {
                a.iter()
                    .map(|d| {
                        (
                            d["name"].as_str().unwrap_or("").to_string(),
                            d["content"].as_str().map(str::to_string),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Of the elements the document held when parsing finished — before any module ran — how
    /// many there were, and the paths of those no longer in the document.
    pub fn prerendered_fate(&self) -> Result<(usize, Vec<String>), BrowserError> {
        let v = self.read("__logosHarness.prerenderedFate()")?;
        let replaced = v["replaced"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| p.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        Ok((v["total"].as_u64().unwrap_or(0) as usize, replaced))
    }

    /// The live renderer objects after a full garbage collection.
    pub fn dom_counters(&self) -> Result<DomCounters, BrowserError> {
        self.catch_up()?;
        self.command("HeapProfiler.collectGarbage", json!({}))?;
        let c = self.command("Memory.getDOMCounters", json!({}))?;
        Ok(DomCounters {
            documents: c["documents"].as_u64().unwrap_or(0),
            nodes: c["nodes"].as_u64().unwrap_or(0),
            listeners: c["jsEventListeners"].as_u64().unwrap_or(0),
        })
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.drops);
        self.wire.detach(&self.session);
        let _ = self.wire.send(
            None,
            "Target.closeTarget",
            json!({ "targetId": self.target }),
            Duration::from_secs(5),
        );
        self.wire.send_detached(
            None,
            "Target.disposeBrowserContext",
            json!({ "browserContextId": self.context }),
        );
    }
}

fn short(s: &str) -> String {
    const MAX: usize = 400;
    if s.len() <= MAX {
        return s.to_string();
    }
    let mut end = MAX;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// The named keys, as `(name, key, code, virtual key code, text)`.
const NAMED_KEYS: &[(&str, &str, &str, u32, Option<&str>)] = &[
    ("Enter", "Enter", "Enter", 13, Some("\r")),
    ("Escape", "Escape", "Escape", 27, None),
    ("Tab", "Tab", "Tab", 9, None),
    ("Backspace", "Backspace", "Backspace", 8, None),
    ("Delete", "Delete", "Delete", 46, None),
    ("ArrowUp", "ArrowUp", "ArrowUp", 38, None),
    ("ArrowDown", "ArrowDown", "ArrowDown", 40, None),
    ("ArrowLeft", "ArrowLeft", "ArrowLeft", 37, None),
    ("ArrowRight", "ArrowRight", "ArrowRight", 39, None),
    ("Home", "Home", "Home", 36, None),
    ("End", "End", "End", 35, None),
    ("PageUp", "PageUp", "PageUp", 33, None),
    ("PageDown", "PageDown", "PageDown", 34, None),
    ("Space", " ", "Space", 32, Some(" ")),
];

/// How one key is described to the browser's input pipeline.
struct KeySpec {
    key: String,
    code: String,
    vk: u32,
    text: Option<String>,
}

impl KeySpec {
    fn named(name: &str) -> Option<KeySpec> {
        if let Some((_, key, code, vk, text)) =
            NAMED_KEYS.iter().find(|k| k.0.eq_ignore_ascii_case(name))
        {
            return Some(KeySpec {
                key: key.to_string(),
                code: code.to_string(),
                vk: *vk,
                text: text.map(str::to_string),
            });
        }
        let mut chars = name.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            return None;
        };
        if c.is_control() {
            return None;
        }
        let (code, vk) = if c.is_ascii_alphabetic() {
            (
                format!("Key{}", c.to_ascii_uppercase()),
                c.to_ascii_uppercase() as u32,
            )
        } else if c.is_ascii_digit() {
            (format!("Digit{c}"), c as u32)
        } else {
            (String::new(), 0)
        };
        Some(KeySpec {
            key: c.to_string(),
            code,
            vk,
            text: Some(c.to_string()),
        })
    }
}
