//! `page_host::RemotePage`/`RemotePageLauncher` over this crate's Chrome harness — the browser
//! page tier `largo test --browser` drives, and the tier every `examples/*/pages/*.lg` test
//! must agree with (`browser_tier_lock`'s `every_example_page_agrees_on_the_tree_walker_the_vm_and_chrome_or_is_ledgered`,
//! tree-walker ≡ lowered VM ≡ Chrome).
//!
//! A page arrives the way a visitor finds it ([`page_host::Arrival`]): served at its own route,
//! with the test's load-time answers already in place, its module's `main` run — which is where a
//! `## Page` mounts itself. A component the test mounts at a statement of its own is mounted at that
//! statement through the host's `__logosMount`, whose `__mount_<C>` export the page was built with
//! because the arrival named it.
//!
//! [`page_host::Arrival`]: logicaffeine_compile::page_host::Arrival

use logicaffeine_compile::page_host::{
    fetch_record, Arrival, DocSnapshot, Drive, DriveError, RemotePage, RemotePageLauncher,
};
use logicaffeine_compile::semantics::dom::DroppedFile;
use logicaffeine_compile::vm::reach::HostEntries;

use super::{
    locate_chrome, shared, Answer, BrowserError, DomNode, OpenOptions, Page, PageBundle, Snapshot,
};

/// Launches a page in the shared, process-wide Chrome for one scenario, as its project serves it:
/// the project's `static/` files beside the page and its stylesheets linked from the document's
/// head, by the rule `largo serve` applies (`serve_web::stylesheet_links_for`). A page's layout is
/// its stylesheet's as much as its markup's — which element a click lands on depends on it — so a
/// page tested bare is not the page a visitor gets. A launcher with no project serves the page alone.
#[derive(Default)]
pub struct ChromeLauncher {
    statics: Vec<(String, Vec<u8>)>,
}

impl ChromeLauncher {
    /// A launcher for the pages of the project whose static files are in `static_dir`.
    pub fn for_project(static_dir: &std::path::Path) -> ChromeLauncher {
        let statics = crate::commands::web_bundle::collect_static_assets(static_dir)
            .into_iter()
            .map(|asset| (asset.path, asset.bytes))
            .collect();
        ChromeLauncher { statics }
    }
}

impl RemotePageLauncher for ChromeLauncher {
    fn launch(&self, arrival: &Arrival) -> Result<Box<dyn RemotePage>, String> {
        let chrome = locate_chrome()
            .ok_or_else(|| "no Chrome/Chromium binary found (set CHROME_BIN, or install one at a name `largo test --browser` looks for)".to_string())?;
        let browser = shared(&chrome).map_err(|e| e.to_string())?;
        let mut bundle =
            PageBundle::page_with(arrival.source, &HostEntries::mounting(&arrival.mounts))?
                .at(&arrival.route);
        let shell_stylesheets: Vec<String> = crate::commands::serve_web::linked_stylesheets(
            self.statics.iter().map(|(p, _)| p.as_str()),
        )
        .into_iter()
        .map(str::to_string)
        .collect();
        if !self.statics.is_empty() {
            let links = crate::commands::serve_web::stylesheet_links_for(
                self.statics.iter().map(|(p, _)| p.as_str()),
            );
            bundle = bundle.with_head_markup(&links);
            for (path, bytes) in &self.statics {
                bundle = bundle.with_file(path, bytes.clone());
            }
        }
        let answers = arrival
            .answers
            .iter()
            .map(|a| Answer {
                method: a.method.clone(),
                path: a.url.clone(),
                status: a.status.clamp(100, 599) as u16,
                content_type: JSON.to_string(),
                body: a.body.clone().into_bytes(),
            })
            .collect();
        let options = OpenOptions {
            answers,
            ..OpenOptions::default()
        };
        let page = browser
            .open_with(&bundle, &options)
            .map_err(|e| e.to_string())?;
        Ok(Box::new(ChromePage {
            page,
            shell_stylesheets,
        }))
    }
}

/// The content type a `Given that … answers …` body is served with — every tier's.
const JSON: &str = logicaffeine_compile::semantics::dom::STUB_CONTENT_TYPE;

/// How a browser produces an event a test raises.
#[derive(Debug, PartialEq, Eq)]
enum Realized {
    /// An event of the page itself (`offline`, `online`, `resize`): raised on `window`.
    Window,
    /// `scrolledtobottom`: the element — or, raised with no target, the page — scrolled to its end.
    ScrollToEnd,
    /// Any other event without a value: raised on the element as a bare DOM event.
    Element,
    /// No visitor's act in a browser carries it; why.
    Unrealized(String),
}

impl Realized {
    fn of(event: &str, target: &str, value: &str) -> Realized {
        if event == "scrolledtobottom" && value.is_empty() {
            return Realized::ScrollToEnd;
        }
        if target.is_empty() || target == "window" {
            return match value.is_empty() {
                true => Realized::Window,
                false => Realized::Unrealized(format!(
                    "a window event carries no value in a browser, and `{event}` was raised with one"
                )),
            };
        }
        match event {
            _ if event.starts_with("message:") => Realized::Unrealized(format!(
                "`{event}` arrives from the relay, a peer this harness does not run; the page's \
                 own `Publish` delivers to it"
            )),
            _ if value.is_empty() => Realized::Element,
            _ => Realized::Unrealized(format!(
                "no act of a visitor raises `{event}` carrying a value in a browser"
            )),
        }
    }
}

/// A page open in the shared Chrome, adapted to [`RemotePage`].
struct ChromePage {
    page: Page,
    /// The project stylesheets the launcher linked — the serving shell's, not the page's head.
    shell_stylesheets: Vec<String>,
}

fn to_message(e: BrowserError) -> DriveError {
    DriveError::Failed(e.to_string())
}

impl ChromePage {
    /// `Mount C [at S]` through the host's own mount entry. A component the page's module does not
    /// export a mount for is one this tier cannot mount (the walk asks the lowering why); a parent
    /// the page does not hold is the reference tier's own failure.
    fn mount(&mut self, component: &str, at: Option<&str>) -> Result<(), DriveError> {
        let entry = logicaffeine_language::parser::lgx_lower::mount_fn_name(component);
        if !self.page.exports().map_err(to_message)?.contains(&entry) {
            return Err(DriveError::Refused(format!(
                "the page's module exports no `{entry}` for its host to call"
            )));
        }
        if let Some(sel) = at {
            let found = self
                .page
                .eval(&format!("document.querySelector({sel:?}) !== null"))
                .map_err(to_message)?;
            if found.as_bool() != Some(true) {
                return Err(DriveError::Failed(
                    logicaffeine_compile::semantics::drive::mount_target_missing(component, sel),
                ));
            }
        }
        self.page.mount(component, at).map_err(to_message)
    }

    /// `Raise "<event>" on "<target>" [with "<value>"]` as a browser meets it: the platform event
    /// the visitor's own act produces, dispatched where the browser dispatches it.
    fn raise(&mut self, event: &str, target: &str, value: &str) -> Result<(), DriveError> {
        let js = match Realized::of(event, target, value) {
            Realized::Window => format!("window.dispatchEvent(new Event({event:?}))"),
            Realized::ScrollToEnd => {
                let scrolled = if target.is_empty() || target == "window" {
                    "body"
                } else {
                    target
                };
                format!("__logosHarness.drive.scrollToBottom({scrolled:?})")
            }
            Realized::Element => format!("__logosHarness.drive.fire({target:?}, {event:?})"),
            Realized::Unrealized(why) => return Err(DriveError::Refused(why)),
        };
        self.page.eval(&js).map_err(to_message)?;
        self.page.settle().map_err(to_message)
    }

    /// `Raise "filedropped" on "<target>" with <file>` as a visitor does it: the file, written to
    /// disk, dragged onto the element and dropped (`Page::drop_file`). A drop that never reached the
    /// document — the element accepts no drops — is the statement's failure, as it is the visitor's.
    /// The browser names a dropped file's type from its name, so a type the test DECLARES that the
    /// browser does not give that name is a file this tier cannot hand the page, and is refused. A
    /// file typed by its name alone is the browser's to name: the tiers' documents show where this
    /// browser's desktop names it otherwise than the table every tier reads.
    fn drop_file(&mut self, target: &str, file: &DroppedFile) -> Result<(), DriveError> {
        let got = self
            .page
            .drop_file(target, &file.name, &file.bytes)
            .map_err(to_message)?;
        let Some(got) = got else {
            return Err(DriveError::Failed(format!(
                "the file {:?} was dragged onto {target:?} and never dropped: nothing there accepts \
                 drops (a drop target cancels `dragover`)",
                file.name
            )));
        };
        if let Some(declared) = &file.declared_type {
            if got.mime_type != *declared {
                return Err(DriveError::Refused(format!(
                    "a browser names a dropped file's type from its name: {:?} is {:?}, and this \
                     test declares {declared:?} — declare the type a browser gives that name",
                    file.name, got.mime_type
                )));
            }
        }
        Ok(())
    }
}

impl RemotePage for ChromePage {
    fn drive(&mut self, d: &Drive) -> Result<(), DriveError> {
        match d {
            Drive::Mount { component, at } => self.mount(component, at.as_deref()),
            Drive::Click { selector } => self.page.click(selector).map_err(to_message),
            Drive::Submit { selector } => self.page.submit(selector).map_err(to_message),
            Drive::Press { key, selector } if selector == "body" => {
                self.page.press(key).map_err(to_message)
            }
            Drive::Press { key, selector } => self.page.press_in(selector, key).map_err(to_message),
            Drive::Type { text, selector } => self.page.type_into(selector, text).map_err(to_message),
            Drive::Choose { value, selector } => self.page.choose(selector, value).map_err(to_message),
            Drive::Hover { selector } => self.page.hover(selector).map_err(to_message),
            Drive::Focus { selector } => self.page.focus(selector).map_err(to_message),
            Drive::Blur { selector } => self.page.blur(selector).map_err(to_message),
            Drive::Raise { event, target, value } => self.raise(event, target, value),
            Drive::DropFile { target, file } => self.drop_file(target, file),
            Drive::Navigate { url } => self
                .page
                .eval(&format!(
                    "history.pushState(null, '', {url:?}); window.dispatchEvent(new PopStateEvent('popstate'))"
                ))
                .map(|_| ())
                .map_err(to_message),
            Drive::Reload => self.page.reload().map_err(to_message),
            Drive::AdvanceTime { ms } => self.page.advance(*ms).map_err(to_message),
            Drive::AnswerFetch { method, url, status, body } => self
                .page
                .answer(method, url, (*status).clamp(100, 599) as u16, JSON, body.as_bytes())
                .map_err(to_message),
            Drive::ServeFetchesWithRoutes => Err(DriveError::Refused(
                "the browser tier cannot consult the page's own route table from outside it yet \
                 — `Serve fetches with routes.` has no live entry"
                    .to_string(),
            )),
            Drive::Replay { .. } => Err(DriveError::Refused(
                "the browser tier does not replay actions by name yet — `Replay these actions:` \
                 has no live entry into a running page's actions"
                    .to_string(),
            )),
        }
    }

    fn unhandled_failure(&self) -> Option<String> {
        self.page.exceptions().into_iter().next()
    }

    fn snapshot(&mut self) -> Result<DocSnapshot, String> {
        let snap = self.page.snapshot().map_err(|e| e.to_string())?;
        Ok(DocSnapshot {
            document: snap.body_markup(),
            head: snap.head_markup_without_stylesheets(&self.shell_stylesheets),
            title: snap.title.clone(),
            url: path_and_query(&snap.url),
            route_ops: self.page.route_ops().map_err(|e| e.to_string())?,
            focus: focus_identity(&snap),
            scroll: None,
            selection: None,
            clipboard: self
                .page
                .clipboard()
                .map_err(|e| e.to_string())?
                .last()
                .cloned()
                .unwrap_or_default(),
            downloads: self
                .page
                .downloads()
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|(name, content)| (name, content.unwrap_or_default()))
                .collect(),
            notifications: self
                .page
                .notifications()
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|title| (title, String::new()))
                .collect(),
            published: self.page.published().map_err(|e| e.to_string())?,
            storage: self.page.storage().map_err(|e| e.to_string())?,
            fetches: self
                .page
                .page_requests()
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|r| {
                    let headers: Vec<(String, String)> = r
                        .headers
                        .chunks(2)
                        .map(|kv| (kv[0].clone(), kv.get(1).cloned().unwrap_or_default()))
                        .collect();
                    fetch_record(&r.method, &r.url, &r.body, &headers, r.timeout, r.aborted)
                })
                .collect(),
            journal: self.page.journal().into_iter().map(|e| e.action).collect(),
            failure: self.unhandled_failure(),
        })
    }
}

/// `http://127.0.0.1:PORT/path?query` → `/path?query` — the page's own route convention
/// (`semantics::dom::route_path`), which never carries the loopback origin the harness serves
/// from.
fn path_and_query(url: &str) -> String {
    match url.find("://").map(|i| i + 3) {
        Some(after_scheme) => match url[after_scheme..].find('/') {
            Some(slash) => url[after_scheme + slash..].to_string(),
            None => "/".to_string(),
        },
        None => url.to_string(),
    }
}

/// The element `snapshot.focus` names, in `page_host::DocSnapshot`'s own convention
/// (`tag#id`, else `tag[i]` — its position among the document's elements of that tag, 0-based,
/// document order) — the SAME rule `semantics::dom`'s recorder uses, so the two are comparable.
fn focus_identity(snapshot: &Snapshot) -> Option<String> {
    let path = snapshot.focus.as_deref()?;
    let node = resolve_path(&snapshot.document, path)?;
    let DomNode::Element { tag, attrs, .. } = node else {
        return None;
    };
    if let Some(id) = attrs
        .iter()
        .find(|(k, _)| k == "id")
        .map(|(_, v)| v.as_str())
        .filter(|v| !v.is_empty())
    {
        return Some(format!("{tag}#{id}"));
    }
    let index = document_order_index(&snapshot.document, node, tag)?;
    Some(format!("{tag}[{index}]"))
}

/// One `tag[n]` path segment: `n` is 1-based, counting only same-tag siblings — the convention
/// the harness recorder's `pathOf` writes.
fn parse_segment(seg: &str) -> Option<(&str, usize)> {
    let open = seg.find('[')?;
    if !seg.ends_with(']') {
        return None;
    }
    let tag = &seg[..open];
    let index: usize = seg[open + 1..seg.len() - 1].parse().ok()?;
    Some((tag, index))
}

/// Walk `path` (`/html[1]/body[1]/…`) from `root`, which the path's own first segment names.
fn resolve_path<'s>(root: &'s DomNode, path: &str) -> Option<&'s DomNode> {
    let mut segments = path.trim_start_matches('/').split('/');
    let (root_tag, root_index) = parse_segment(segments.next()?)?;
    if root.tag() != Some(root_tag) || root_index != 1 {
        return None;
    }
    let mut node = root;
    for seg in segments {
        let (tag, index) = parse_segment(seg)?;
        let mut seen = 0usize;
        node = node.children().iter().find(|c| {
            if c.tag() != Some(tag) {
                return false;
            }
            seen += 1;
            seen == index
        })?;
    }
    Some(node)
}

/// `target`'s 0-based position among every `tag` element under `root`, in document order — the
/// same order `semantics::dom::query_selector_all` walks, which is what `element_identity` (the
/// native tiers' side of this same rule) indexes into.
fn document_order_index(root: &DomNode, target: &DomNode, tag: &str) -> Option<usize> {
    fn walk<'s>(n: &'s DomNode, tag: &str, target: &DomNode, count: &mut usize) -> Option<usize> {
        if n.tag() == Some(tag) {
            if std::ptr::eq(n, target) {
                return Some(*count);
            }
            *count += 1;
        }
        for c in n.children() {
            if let Some(i) = walk(c, tag, target, count) {
                return Some(i);
            }
        }
        None
    }
    walk(root, tag, target, &mut 0)
}
