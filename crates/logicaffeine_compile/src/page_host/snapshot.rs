//! What a page looks like from outside, at one moment — the unit two tiers are compared in.
//!
//! Everything here is read from the thread-local recorders every in-process tier writes into
//! (`semantics::dom`, `semantics::storage`, the journal in `semantics::world`), rendered into a
//! tier-independent form: nodes are named by what they ARE (`p#status`, the third `li`), never by
//! the handle a tier happened to give them, because two tiers that paint the same document may
//! number its nodes differently.

use crate::semantics::{dom, storage, world};
use serde::Serialize;

/// One observable property of a page. A divergence is reported per axis, so a ledger can name
/// exactly which part of the page two tiers disagree about — and a second, unrelated divergence
/// on the same page cannot hide behind the first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    /// The rendered body.
    Document,
    /// The document's `<head>`: the shell the page arrived in, with the page's own `Head:` merged
    /// over it.
    Head,
    /// `document.title`.
    Title,
    /// The location path.
    Url,
    /// Every navigation, in order (`push /x`, `replace /y`, `load /z`).
    RouteOps,
    /// The focused element.
    Focus,
    /// The element last scrolled to.
    Scroll,
    /// The element whose text was last selected.
    Selection,
    /// What the page put on the clipboard.
    Clipboard,
    /// Files the page offered for download.
    Downloads,
    /// Notifications the page raised.
    Notifications,
    /// Frames the page published on a relay topic.
    Published,
    /// Browser storage, both areas.
    Storage,
    /// Every request the page sent (method, URL, body, headers, timeout, abort).
    Fetches,
    /// The actions that have run, in order — what `the journal` answers.
    Journal,
    /// The first action failure no handler absorbed — the program's error once the run ends.
    Failure,
    /// Where the scenario stopped, and why. Not a snapshot field: a trace carries it.
    Stopped,
}

impl Axis {
    /// Every axis a snapshot carries, in report order.
    pub const SNAPSHOT: [Axis; 16] = [
        Axis::Document,
        Axis::Head,
        Axis::Title,
        Axis::Url,
        Axis::RouteOps,
        Axis::Focus,
        Axis::Scroll,
        Axis::Selection,
        Axis::Clipboard,
        Axis::Downloads,
        Axis::Notifications,
        Axis::Published,
        Axis::Storage,
        Axis::Fetches,
        Axis::Journal,
        Axis::Failure,
    ];

    /// The axis's stable name — what a ledger line spells.
    pub fn name(self) -> &'static str {
        match self {
            Axis::Document => "document",
            Axis::Head => "head",
            Axis::Title => "title",
            Axis::Url => "url",
            Axis::RouteOps => "route_ops",
            Axis::Focus => "focus",
            Axis::Scroll => "scroll",
            Axis::Selection => "selection",
            Axis::Clipboard => "clipboard",
            Axis::Downloads => "downloads",
            Axis::Notifications => "notifications",
            Axis::Published => "published",
            Axis::Storage => "storage",
            Axis::Fetches => "fetches",
            Axis::Journal => "journal",
            Axis::Failure => "failure",
            Axis::Stopped => "stopped",
        }
    }

    /// The axis a ledger line names.
    pub fn from_name(name: &str) -> Option<Axis> {
        Axis::SNAPSHOT
            .iter()
            .copied()
            .chain([Axis::Stopped])
            .find(|a| a.name() == name)
    }
}

/// The page, observed from outside, at one moment.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DocSnapshot {
    pub document: String,
    pub head: Option<String>,
    pub title: String,
    pub url: String,
    pub route_ops: Vec<String>,
    pub focus: Option<String>,
    pub scroll: Option<String>,
    pub selection: Option<String>,
    pub clipboard: String,
    pub downloads: Vec<(String, String)>,
    pub notifications: Vec<(String, String)>,
    pub published: Vec<(String, String)>,
    /// `(area, key, value)`, sorted.
    pub storage: Vec<(String, String, String)>,
    pub fetches: Vec<String>,
    pub journal: Vec<String>,
    pub failure: Option<String>,
}

impl DocSnapshot {
    /// Read the page every in-process tier paints into. `failure` is the first unhandled action
    /// failure the tier reported — each tier knows its own, so the caller supplies it.
    pub fn capture(failure: Option<String>) -> DocSnapshot {
        let mut storage_rows = Vec::new();
        for area in [storage::LOCAL, storage::SESSION] {
            for key in storage::keys(area).unwrap_or_default() {
                let value = storage::get(area, &key).ok().flatten().unwrap_or_default();
                storage_rows.push((area.to_string(), key, value));
            }
        }
        DocSnapshot {
            document: dom::render(),
            head: dom::document_head(),
            title: dom::title(),
            url: dom::route_path(),
            route_ops: dom::route_ops()
                .into_iter()
                .map(|op| match op {
                    dom::RouteOp::Navigate {
                        path,
                        replace: false,
                    } => format!("push {path}"),
                    dom::RouteOp::Navigate {
                        path,
                        replace: true,
                    } => format!("replace {path}"),
                    dom::RouteOp::Load { path } => format!("load {path}"),
                })
                .collect(),
            focus: element_identity(dom::focused()),
            scroll: element_identity(dom::scrolled_to()),
            selection: element_identity(dom::selected()),
            clipboard: dom::clipboard(),
            downloads: dom::downloads(),
            notifications: dom::notifications(),
            published: dom::published(),
            storage: storage_rows,
            fetches: dom::fetch_requests()
                .into_iter()
                .map(|r| {
                    fetch_record(
                        &r.method,
                        &r.url,
                        &r.body,
                        &r.headers,
                        r.timeout_ms,
                        r.aborted,
                    )
                })
                .collect(),
            journal: world::journal().into_iter().map(|e| e.action).collect(),
            failure,
        }
    }

    /// The value on one axis, rendered for comparison and for a report.
    pub fn axis(&self, axis: Axis) -> String {
        fn pairs(v: &[(String, String)]) -> String {
            v.iter()
                .map(|(a, b)| format!("({a:?}, {b:?})"))
                .collect::<Vec<_>>()
                .join(", ")
        }
        match axis {
            Axis::Document => self.document.clone(),
            Axis::Head => self.head.clone().unwrap_or_default(),
            Axis::Title => self.title.clone(),
            Axis::Url => self.url.clone(),
            Axis::RouteOps => self.route_ops.join(", "),
            Axis::Focus => self.focus.clone().unwrap_or_default(),
            Axis::Scroll => self.scroll.clone().unwrap_or_default(),
            Axis::Selection => self.selection.clone().unwrap_or_default(),
            Axis::Clipboard => self.clipboard.clone(),
            Axis::Downloads => pairs(&self.downloads),
            Axis::Notifications => pairs(&self.notifications),
            Axis::Published => pairs(&self.published),
            Axis::Storage => self
                .storage
                .iter()
                .map(|(a, k, v)| format!("{a}:{k}={v:?}"))
                .collect::<Vec<_>>()
                .join(", "),
            Axis::Fetches => self.fetches.join("; "),
            Axis::Journal => self.journal.join(", "),
            Axis::Failure => self.failure.clone().unwrap_or_default(),
            Axis::Stopped => String::new(),
        }
    }
}

/// One request of a page's own request table as the fetches axis spells it — every tier's snapshot
/// renders its table through this, so two tiers that asked for the same request read the same.
pub fn fetch_record(
    method: &str,
    url: &str,
    body: &str,
    headers: &[(String, String)],
    timeout_ms: i64,
    aborted: bool,
) -> String {
    let headers: Vec<String> = headers.iter().map(|(n, v)| format!("{n}: {v}")).collect();
    format!(
        "{method} {url} body={body:?} headers=[{}] timeout={timeout_ms} aborted={aborted}",
        headers.join(", ")
    )
}

/// An element named by what it is: `tag#id` when it has an id, else `tag[n]`, its position among
/// the document's elements of that tag. `None` for no element.
fn element_identity(node: i64) -> Option<String> {
    if node < 0 {
        return None;
    }
    let tag = dom::tag_of(node)?;
    if let Some(id) = dom::attribute(node, "id").filter(|id| !id.is_empty()) {
        return Some(format!("{tag}#{id}"));
    }
    match dom::query_selector_all(&tag)
        .iter()
        .position(|n| *n == node)
    {
        Some(i) => Some(format!("{tag}[{i}]")),
        None => Some(format!("{tag} (detached)")),
    }
}
