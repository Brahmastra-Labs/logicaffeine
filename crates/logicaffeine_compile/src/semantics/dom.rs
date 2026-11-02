//! An in-process mock DOM — the shared oracle that gives the DOM builtins
//! identical semantics on the tree-walker and the register VM (they both run
//! `call_builtin`), so the sacred WASM==VM==tree-walker invariant holds even
//! though the DOM is browser-only.
//!
//! A DOM-driving LOGOS program builds a node tree and mutates it; on the native
//! tiers those operations land here (a thread-local handle table + an ordered
//! op-log), and on the wasm tier the SAME operations land in the emitted module's
//! `dom_*` host imports — a `MockDom` linker in tests, the real browser DOM in
//! production. Because DOM ops emit no `Show`, the existing output differential
//! passes trivially; the DOM EFFECTS are compared via [`op_log`]/[`render`], keyed
//! by node **creation ordinal** (raw handles legitimately differ per tier), which
//! the wasm `MockDom` reproduces because it creates nodes in the same order.
//!
//! Modeled on `logicaffeine_base::money`'s `AMBIENT_RATES` thread-local.

use crate::interpreter::RuntimeValue;
use std::cell::RefCell;
use std::collections::BTreeMap;

/// The largest request body `fetchSend` will put on the wire — exceeding it fails the
/// request immediately with the `"toolarge"` transport error (never a silent truncation).
/// Shared by every tier (native recorder, MockDom, browser host) so behavior cannot drift.
pub const FETCH_REQUEST_BODY_CAP: usize = 4 * 1024 * 1024;

/// The largest response body a fetch will deliver — a larger body is dropped and the
/// request fails with `"toolarge"` (the callback still runs and can read the error).
pub const FETCH_RESPONSE_BODY_CAP: usize = 4 * 1024 * 1024;

/// The request headers `fetchSend` allows (lowercase). A header outside this list fails
/// the request immediately with `"network: header not allowed: <name>"` — the browser
/// would silently strip or reject forbidden headers per-engine; the contract makes the
/// denial explicit and identical on every tier.
pub const FETCH_ALLOWED_REQUEST_HEADERS: &[&str] = &[
    "accept",
    "accept-language",
    "authorization",
    "cache-control",
    "content-type",
    "idempotency-key",
    "if-match",
    "if-none-match",
    "x-csrf-token",
    "x-expected-version",
    "x-requested-with",
];

/// The `<meta name=…>` a document carries its CSRF token in — written by the server that composes
/// a page under a `## Session`, and read by every tier's host when the page makes a request.
pub const CSRF_META_NAME: &str = logicaffeine_system::csrf::CSRF_META_NAME;

/// Whether a request changes state on the server, and so must prove it came from the page the
/// server handed out: `POST`, `PUT`, `PATCH`, `DELETE`.
pub fn changes_state(method: &str) -> bool {
    ["POST", "PUT", "PATCH", "DELETE"]
        .iter()
        .any(|m| m.eq_ignore_ascii_case(method))
}

/// Whether `url` goes to the page's own server: a URL with no scheme and no host of its own.
/// Decided on the URL as the page wrote it, identically in every host.
pub fn to_own_server(url: &str) -> bool {
    !url.contains("://") && !url.starts_with("//")
}

/// One recorded DOM mutation. Nodes are referenced by their **creation ordinal**
/// (the handle, assigned sequentially from 0 = `body`), so op-logs compare
/// equal across tiers regardless of how each tier numbers its handles internally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DomOp {
    CreateElement {
        node: u32,
        tag: String,
    },
    CreateTextNode {
        node: u32,
        text: String,
    },
    SetText {
        node: u32,
        text: String,
    },
    SetHtml {
        node: u32,
        html: String,
    },
    ClearChildren {
        node: u32,
    },
    SetAttribute {
        node: u32,
        name: String,
        value: String,
    },
    AppendChild {
        parent: u32,
        child: u32,
    },
    SetValue {
        node: u32,
        value: String,
    },
    /// Attach an opaque host media stream (a `MediaStream` handle) to an element's `srcObject` —
    /// the primitive that renders a peer's live video/audio in a `<video>`/`<audio>` tile.
    SetStream {
        node: u32,
        stream: i64,
    },
    // --- WebRTC: the browser P2P leg. `peerConnection`/`dataChannel` mint opaque handles (an
    // `RTCPeerConnection` / `RTCDataChannel` in the browser); the rest drive the SDP + ICE
    // handshake and the DataChannel send.
    //
    // ★ THESE LAND IN THIS OP-LOG AND NOWHERE ELSE. This comment used to say they "lower to `rtc_*`
    // host imports (real WebRTC in the browser, a mock in tests)" — there is no `rtc_*` import in
    // the ABI, no mock, and nothing in `runtime.js`, so a reader was told a browser leg exists that
    // does not. `wasm_host_abi_lock` writes the whole import surface down; `rtc_` appears nowhere
    // in it. The ops are real on the native tiers and a browser page cannot reach them yet. ---
    RtcPeerConnection {
        pc: u32,
    },
    RtcDataChannel {
        pc: u32,
        channel: u32,
        label: String,
    },
    RtcCreateOffer {
        pc: u32,
    },
    RtcCreateAnswer {
        pc: u32,
    },
    RtcSetLocal {
        pc: u32,
        sdp: String,
    },
    RtcSetRemote {
        pc: u32,
        sdp: String,
    },
    RtcAddIce {
        pc: u32,
        candidate: String,
    },
    RtcSend {
        channel: u32,
        bytes: u64,
    },
    AddEventListener {
        node: u32,
        event: String,
    },
    /// `setChecked(el, on)` — a checkbox's or radio's CHECKED state.
    ///
    /// Deliberately not a `SetAttribute`. In HTML `checked` is a PRESENCE, and in the DOM it is
    /// two different things: the ATTRIBUTE is `defaultChecked` (what the box returns to on a form
    /// reset) while the live state a visitor sees and a form submits is the PROPERTY. Writing the
    /// attribute therefore leaves a box the page believes is checked drawn unchecked. Carrying it
    /// as a boolean op means both tiers agree on presence and absence rather than on a string, and
    /// `checked="false"` — which every browser reads as CHECKED — is unspellable.
    SetChecked {
        node: u32,
        checked: bool,
    },
    /// ★ THIS LISTENER CANCELS THE BROWSER'S OWN ACTION for `event` on `node`.
    ///
    /// `on-submit.prevent={save}` means the page handles the submit and the browser must not ALSO
    /// post the form — otherwise the request goes twice, once to the handler and once to the
    /// server, which is the duplicate-submission bug every form library exists to avoid. It is a
    /// property of the REGISTRATION, not of the event, so it is recorded where the listener is
    /// attached rather than discovered when one fires.
    PreventDefault {
        node: u32,
        event: String,
    },
    /// Detach a node from wherever it currently lives — the single-node counterpart to
    /// `ClearChildren`, so a keyed diff drops exactly the rows it stopped rendering instead of
    /// tearing down and rebuilding the whole parent.
    RemoveNode {
        node: u32,
    },
    /// Move `child` under `parent`, immediately before `before` — or last, when `before` is
    /// `u32::MAX` (the same "no such element" sentinel documented in `patch.rs`; handle `0` is
    /// `body` here, so it cannot double as the "append" marker). The op a keyed reorder's minimal
    /// move set is built from: `child` keeps its identity, so a reused row keeps whatever focus,
    /// selection or typed value it already carries.
    InsertBefore {
        parent: u32,
        child: u32,
        before: u32,
    },
    /// Remove an attribute a new render dropped — the deletion `SetAttribute` cannot express.
    RemoveAttribute {
        node: u32,
        name: String,
    },
    /// Bind `node` to the next element under `parent` that the markup a [`DomOp::Hydrate`] opened
    /// already holds, when its tag is `tag`; an element of another tag, or none, is a mismatch the
    /// stream's apply counts, and `node` stays unbound.
    Adopt {
        parent: u32,
        node: u32,
        tag: String,
    },
    /// Bind `node` to its place in the run of text the markup holds under `parent` before the next
    /// adopted element, holding `text`. The run is split along its claims when they spell it and
    /// rebuilt when they do not: a text node carries nothing a visitor did to it.
    AdoptText {
        parent: u32,
        node: u32,
        text: String,
    },
    /// Register `event` on `node` with a bitmask of behavioural modifiers (`.prevent` / `.stop`
    /// / `.once` / `.capture` / `.passive`) — the generalisation `AddEventListener` /
    /// `PreventDefault` predate, carrying every modifier in one registration rather than one op
    /// per behaviour.
    Listen {
        node: u32,
        event: String,
        flags: u8,
    },
    /// Remove a listener a new render dropped.
    Unlisten {
        node: u32,
        event: String,
    },
    /// Detach `node` from its parent AND release the id of `node` and EVERY descendant — the
    /// document-side half of heap reclamation, and ONE op per removed subtree root regardless of
    /// its size: a 1,000-row list clear is 1,000 `FreeNode`s, not a `FreeNode` per node. The host
    /// walks the (about to be detached) subtree once, deleting each id from its own node table,
    /// rather than the emitter naming every descendant — a descendant has no `RemoveNode`/
    /// `FreeNode` of its own; removing/freeing its ancestor is what removes/frees it.
    FreeNode {
        node: u32,
    },
    /// Open a hydration of `root`: the markup it holds is the page's first paint, drawn before the
    /// program ran, and the stream's `Adopt`/`AdoptText` claim its nodes instead of creating them.
    /// When the stream ends, whatever the markup holds that no claim bound is removed.
    Hydrate {
        root: u32,
    },
}

/// One recorded navigation — the router's history op-log entry: a pushState (`replace: false`, a
/// new history entry) or replaceState (`replace: true`, swap the current one) within the page, or a
/// document load. Compares equal across tiers; the wasm `MockDom` records the same shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteOp {
    Navigate {
        path: String,
        replace: bool,
    },
    /// A document load at `path` (`location.assign`) — the page that asked is gone.
    Load {
        path: String,
    },
}

#[derive(Clone, Debug, Default)]
struct Node {
    tag: String,
    text: String,
    /// Raw innerHTML set via `setHtml` — verbatim markup, rendered after `text`.
    html: String,
    value: String,
    /// A checkbox/radio's live checked PROPERTY. Separate from `attrs` because it is a presence,
    /// not a string: see [`DomOp::SetChecked`].
    checked: bool,
    attrs: BTreeMap<String, String>,
    children: Vec<u32>,
    is_text: bool,
    /// Whether the control's value was last changed by the visitor's own editing — what makes
    /// `minlength`/`maxlength` apply ([`Validity`]).
    user_edited: bool,
    /// Whether the visitor typed something the control cannot hold (a `type="number"` field given
    /// text that is no number), so its value is empty — `badInput`.
    bad_input: bool,
}

impl Node {
    /// See [`focusable`].
    fn focusable(&self) -> bool {
        if self.is_text {
            return false;
        }
        let has = |name: &str| self.attrs.contains_key(name);
        let enabled = !has("disabled");
        if has("tabindex") {
            return true;
        }
        if self
            .attrs
            .get("contenteditable")
            .is_some_and(|v| !v.eq_ignore_ascii_case("false"))
        {
            return true;
        }
        match self.tag.to_ascii_lowercase().as_str() {
            "button" | "select" | "textarea" => enabled,
            "input" => {
                enabled
                    && !self
                        .attrs
                        .get("type")
                        .is_some_and(|t| t.eq_ignore_ascii_case("hidden"))
            }
            "a" | "area" => has("href"),
            "iframe" | "summary" => true,
            "audio" | "video" => has("controls"),
            _ => false,
        }
    }
}

/// The selector engine reads the ONE parse the whole language shares: the static check that
/// refuses a `When "#draft" is clicked:` naming an element no view renders reads the same
/// grammar this answers against the live page, so they cannot drift apart.
use logicaffeine_language::lgx::selector::{self, Compound};

/// The mock DOM: a handle table of nodes + the ordered op-log. Handle 0 is always
/// `body`, the render root.
#[derive(Clone, Debug)]
pub struct DomRecorder {
    nodes: Vec<Node>,
    /// The nodes a patch stream created, at `id - STREAM_ID_BASE`; `None` is an id free to rebind.
    streamed: Vec<Option<Node>>,
    log: Vec<DomOp>,
    /// Registered handlers, `(node, event) → closure`. On the native tiers the
    /// closure is a `RuntimeValue::Function` we later invoke to simulate an event.
    listeners: Vec<(u32, String, RuntimeValue)>,
    /// Every listener registration the page made, `(node, event) → flags` (the `LISTEN_*` bits) —
    /// what [`dispatch_path`] walks. A closure the host can fire is not needed for a registration
    /// to count: the page's own dispatcher answers for the node, and the browser needs only to
    /// know that something listens there, and how.
    registrations: BTreeMap<(u32, String), u8>,
    /// The document's `<head>`, element by element — what a browser holds after the page's shell
    /// arrived and the page merged its own `Head:` over it. `the title` reads its first `<title>`
    /// and `Set the title to …` writes it, exactly as `document.title` does. Recorder state rather
    /// than ops: the head is not painted, so it is not in the op-log the body is compared through.
    head: Vec<HeadTag>,
    /// What the page put on the clipboard, offered as a download, and raised as a notification.
    ///
    /// ★ RECORDED, NOT PERFORMED, ON A TIER WITH NO BROWSER — and recorded as a LIST for the two
    /// that can happen more than once, because "did the page notify" and "how many times" are
    /// different questions and a page that notifies twice is usually a bug. The clipboard holds one
    /// thing by definition, so it is the last write.
    clipboard: String,
    downloads: Vec<(String, String)>,
    notifications: Vec<(String, String)>,
    /// Every frame the page published on a relay topic, `(topic, message)`, in order — see
    /// [`publish`].
    published: Vec<(String, String)>,
    /// The element most recently brought into view (`Scroll to "#card".`) and the one whose text
    /// was selected (`Select the text in "#draft".`), or the `None` handle.
    ///
    /// ★ RECORDER STATE, NOT AN OP. Neither is a change to the document — scrolling and selecting
    /// are things a browser does TO a document that stays the same — so neither belongs in the
    /// op-log the tiers are compared through. Held here so a test can still ask whether the page
    /// asked, which is the only part any tier without a viewport can honestly answer.
    scrolled_to: u32,
    selected: u32,
    /// The focused element (`Focus "#draft".`), or the `None` handle. Recorder state for the
    /// same reason `title` is.
    focused: u32,
    /// Focus and blur the page's own element commands raised and the host has not delivered yet —
    /// see [`take_raised_events`].
    raised: Vec<(String, &'static str)>,
    /// The file the drop being delivered carries, on a tier with no real transfer to read it from —
    /// see [`set_dropped_file`]. Belongs to the document: a reload starts without one.
    dropped_file: Option<DroppedFile>,
    /// Whether the browser has stepped back through the history since the host last looked
    /// ([`take_popped`]) — the `popstate` a `history.back()` raises once the step is taken.
    popped: bool,
    /// Whether the page asked the browser to load it again (`location.reload()`) — a request the
    /// browser carries out once the running script is done ([`take_reload_request`]).
    reload_requested: bool,
    /// Where the page asked the browser to load another document (`location.assign`) — the last
    /// ask wins, and the browser carries it out once the running script is done
    /// ([`take_load_request`]); until then the page that asked is the one on screen.
    load_requested: Option<String>,
    /// The event currently being dispatched — what `eventValue`/`eventTarget` read.
    current_event: Option<CurrentEvent>,
    /// Outstanding fetches keyed by REQUEST ID (assigned sequentially from 1) —
    /// the correlation handle that makes concurrent fetches unconfusable. Resolved
    /// on command (never real IO) so callback order is deterministic across tiers.
    fetches: BTreeMap<i64, PendingFetch>,
    /// The next request id `fetchSend` mints.
    next_fetch: i64,
    /// Answers a test has arranged for requests it expects the page to send: `(method, url) →
    /// (status, body)`. The sealed-world rule the rest of the suite follows — a page test must
    /// not depend on a network, and a page whose data never arrives proves nothing.
    ///
    /// Keyed on method AND url together, because a stub that answered the wrong request is how a
    /// test ends up proving something nobody asked for.
    fetch_stubs: Vec<((String, String), (i64, String))>,
    /// Delivered (or failed) responses, PER REQUEST — what `responseStatus`/
    /// `responseBody`/`responseHeader`/`responseError` read. Once present, a
    /// request is FINAL: later resolves/failures are suppressed (no double
    /// delivery, and an aborted request can never deliver).
    fetch_responses: BTreeMap<i64, FetchResponse>,
    /// The request currently being delivered — what the `responseText` sugar reads.
    current_fetch: Option<i64>,
    /// The router — the location model shared by the tree-walker and the VM (the
    /// wasm tier mirrors it host-side). `route_path` is `window.location.pathname`,
    /// `route_query` its query string, `route_ops` the ordered pushState/replaceState
    /// log, and `route_handlers` every LOGOS closure registered by `onRoute`, each of which a
    /// popstate (back/forward) fires, in the order they were registered. Deterministic:
    /// navigation records ops; a test `deliver_pop(path)` hands the handlers over (never a real
    /// browser event).
    route_path: String,
    route_query: String,
    route_ops: Vec<RouteOp>,
    /// ★ THE BACK STACK, which `route_ops` is NOT. The op log records what a page DID (including
    /// every `replaceState`, which creates no entry); going back needs to know where the page
    /// would land, which is a different question. Index 0 is where the app booted, so `Go back.`
    /// from the first page has nowhere to go and says so rather than inventing a path.
    route_history: Vec<String>,
    route_handlers: Vec<RuntimeValue>,
    /// The hydration the stream being applied opened ([`DomOp::Hydrate`]), until the stream ends.
    hydration: Option<Hydration>,
    /// Ids that named a node a hydration renumbered: an adopted node answers to the id its stream
    /// names for it, and the id this recorder minted for it before still reaches it, as a browser's
    /// node table keeps every id it handed out for an element.
    forwarded: BTreeMap<u32, u32>,
}

/// A stream adopting the markup a host node already holds — see [`DomOp::Hydrate`].
#[derive(Clone, Debug, Default)]
struct Hydration {
    /// Every node whose children the stream claims — the root, then each adopted element — with the
    /// child bound under it last, `None` before its first.
    cursors: BTreeMap<u32, Option<u32>>,
    /// The text claims waiting for the next element claim under the same parent, in stream order:
    /// `(parent, node, text)`.
    texts: Vec<(u32, u32, String)>,
    /// Adopted elements whose content is raw markup (`SetHtml`), which no claim walks.
    raw: std::collections::BTreeSet<u32>,
    /// The attributes the stream wrote on each adopted element; the markup's others are dropped
    /// when the stream ends — among them the `value` and `checked` a pre-render spells a control's
    /// state with, which the stream writes as the properties they are.
    written: BTreeMap<u32, std::collections::BTreeSet<String>>,
    /// Element claims that found no element of their tag.
    mismatches: i64,
}

#[derive(Clone, Debug)]
struct CurrentEvent {
    target: u32,
    value: String,
    /// The key that was pressed, for a keyboard event — what `eventKey` reads. Empty for
    /// every other event, which is what a browser reports too.
    key: String,
}

/// One outstanding `fetchSend` request as the recorder holds it.
#[derive(Clone, Debug)]
struct PendingFetch {
    method: String,
    url: String,
    body: String,
    headers: Vec<(String, String)>,
    closure: RuntimeValue,
    timeout_ms: i64,
    aborted: bool,
}

/// A delivered response (or a structured transport failure: `status == 0` and
/// `error` one of `"timeout"` | `"abort"` | `"toolarge"` | `"network: <detail>"`).
#[derive(Clone, Debug, Default)]
struct FetchResponse {
    status: i64,
    headers: Vec<(String, String)>,
    body: String,
    error: String,
}

/// One recorded fetch request — the tw≡VM parity surface for the request side
/// (what was asked for, with what method/body/headers, and its control state).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchRequest {
    pub req: i64,
    pub method: String,
    pub url: String,
    pub body: String,
    pub headers: Vec<(String, String)>,
    pub timeout_ms: i64,
    pub aborted: bool,
}

impl Default for DomRecorder {
    fn default() -> Self {
        // Node 0 is `document.body`, the render root — pre-created so `body()` is a
        // stable handle and the ordinals of user-created nodes start at 1.
        DomRecorder {
            head: Vec::new(),
            focused: u32::MAX,
            clipboard: String::new(),
            downloads: Vec::new(),
            notifications: Vec::new(),
            published: Vec::new(),
            scrolled_to: u32::MAX,
            selected: u32::MAX,
            raised: Vec::new(),
            dropped_file: None,
            popped: false,
            reload_requested: false,
            load_requested: None,
            fetch_stubs: Vec::new(),
            nodes: vec![Node {
                tag: "body".to_string(),
                ..Node::default()
            }],
            streamed: Vec::new(),
            log: Vec::new(),
            listeners: Vec::new(),
            registrations: BTreeMap::new(),
            current_event: None,
            fetches: BTreeMap::new(),
            next_fetch: 1,
            fetch_responses: BTreeMap::new(),
            current_fetch: None,
            // The SPA root is the default location — a fresh run boots at "/".
            route_path: "/".to_string(),
            route_query: String::new(),
            route_ops: Vec::new(),
            route_history: vec!["/".to_string()],
            route_handlers: Vec::new(),
            hydration: None,
            forwarded: BTreeMap::new(),
        }
    }
}

impl DomRecorder {
    fn get(&self, h: u32) -> Option<&Node> {
        let h = self.resolve(h);
        match h.checked_sub(STREAM_ID_BASE) {
            Some(i) => self.streamed.get(i as usize).and_then(Option::as_ref),
            None => self.nodes.get(h as usize),
        }
    }
    fn get_mut(&mut self, h: u32) -> Option<&mut Node> {
        let h = self.resolve(h);
        match h.checked_sub(STREAM_ID_BASE) {
            Some(i) => self.streamed.get_mut(i as usize).and_then(Option::as_mut),
            None => self.nodes.get_mut(h as usize),
        }
    }

    /// The id the node `h` names answers to now — `h` itself, unless a hydration renumbered it.
    fn resolve(&self, h: u32) -> u32 {
        self.forwarded.get(&h).copied().unwrap_or(h)
    }

    fn body(&self) -> u32 {
        0
    }

    /// Binds `node` to `fresh`: a stream id that is not live, or exactly the ordinal this recorder
    /// would mint next — the one id below [`STREAM_ID_BASE`] a stream can name for a new node.
    fn create_as(&mut self, node: u32, fresh: Node) -> Result<(), String> {
        match node.checked_sub(STREAM_ID_BASE) {
            Some(i) => {
                let i = i as usize;
                if self.streamed.len() <= i {
                    self.streamed.resize_with(i + 1, || None);
                }
                if self.streamed[i].is_some() {
                    return Err(format!(
                        "the stream creates node {node}, which is still live"
                    ));
                }
                self.streamed[i] = Some(fresh);
            }
            None if node as usize == self.nodes.len() => self.nodes.push(fresh),
            None => {
                return Err(format!(
                    "the stream names node {node} for a new node, and this host mints its own ids \
                     below {STREAM_ID_BASE}: the next is {}",
                    self.nodes.len()
                ))
            }
        }
        Ok(())
    }

    fn create_element_as(&mut self, node: u32, tag: &str) -> Result<(), String> {
        self.create_as(
            node,
            Node {
                tag: tag.to_string(),
                ..Node::default()
            },
        )?;
        self.log.push(DomOp::CreateElement {
            node,
            tag: tag.to_string(),
        });
        Ok(())
    }

    fn create_text_node_as(&mut self, node: u32, text: &str) -> Result<(), String> {
        self.create_as(
            node,
            Node {
                text: text.to_string(),
                is_text: true,
                ..Node::default()
            },
        )?;
        self.log.push(DomOp::CreateTextNode {
            node,
            text: text.to_string(),
        });
        Ok(())
    }

    fn create_element(&mut self, tag: &str) -> u32 {
        let h = self.nodes.len() as u32;
        self.nodes.push(Node {
            tag: tag.to_string(),
            ..Node::default()
        });
        self.log.push(DomOp::CreateElement {
            node: h,
            tag: tag.to_string(),
        });
        h
    }

    fn create_text_node(&mut self, text: &str) -> u32 {
        let h = self.nodes.len() as u32;
        self.nodes.push(Node {
            text: text.to_string(),
            is_text: true,
            ..Node::default()
        });
        self.log.push(DomOp::CreateTextNode {
            node: h,
            text: text.to_string(),
        });
        h
    }

    fn set_text(&mut self, node: u32, text: &str) {
        if let Some(n) = self.get_mut(node) {
            n.text = text.to_string();
        }
        self.log.push(DomOp::SetText {
            node,
            text: text.to_string(),
        });
    }

    fn set_html(&mut self, node: u32, html: &str) {
        if self.adopting(node) {
            self.hydration.as_mut().map(|h| h.raw.insert(node));
        }
        if let Some(n) = self.get_mut(node) {
            n.html = html.to_string();
        }
        self.log.push(DomOp::SetHtml {
            node,
            html: html.to_string(),
        });
    }

    /// Remove every child of `node` (and any raw innerHTML) — the immediate-mode
    /// re-render primitive: the view clears its root, then rebuilds it.
    fn clear_children(&mut self, node: u32) {
        if let Some(n) = self.get_mut(node) {
            n.children.clear();
            n.html.clear();
        }
        self.log.push(DomOp::ClearChildren { node });
    }

    fn set_attribute(&mut self, node: u32, name: &str, value: &str) {
        if let Some(written) = self
            .hydration
            .as_mut()
            .and_then(|h| h.written.get_mut(&node))
        {
            written.insert(name.to_string());
        }
        if let Some(n) = self.get_mut(node) {
            n.attrs.insert(name.to_string(), value.to_string());
        }
        self.log.push(DomOp::SetAttribute {
            node,
            name: name.to_string(),
            value: value.to_string(),
        });
    }

    fn append_child(&mut self, parent: u32, child: u32) {
        let child = self.resolve(child);
        if let Some(n) = self.get_mut(parent) {
            n.children.push(child);
        }
        self.log.push(DomOp::AppendChild { parent, child });
    }

    fn set_value(&mut self, node: u32, value: &str) {
        if self.adopting(node) {
            if self.get(node).is_some_and(|n| n.user_edited) {
                self.keep_the_visitors(node);
                self.log.push(DomOp::SetValue {
                    node,
                    value: value.to_string(),
                });
                return;
            }
        }
        if let Some(n) = self.get_mut(node) {
            if n.value != value {
                n.user_edited = false;
                n.bad_input = false;
            }
            n.value = value.to_string();
        }
        self.log.push(DomOp::SetValue {
            node,
            value: value.to_string(),
        });
    }

    /// The `<option>`s under a `<select>`, in document order, as `(value, selected-by-markup)` —
    /// an option's value is its `value` attribute, or else its text.
    fn options_of(&self, select: u32) -> Vec<(String, bool)> {
        let mut out = Vec::new();
        let mut stack: Vec<u32> = self
            .get(select)
            .map(|n| n.children.iter().rev().copied().collect())
            .unwrap_or_default();
        while let Some(h) = stack.pop() {
            let Some(n) = self.get(h) else { continue };
            if n.tag.eq_ignore_ascii_case("option") {
                let value = n
                    .attrs
                    .get("value")
                    .cloned()
                    .unwrap_or_else(|| self.text_content(h));
                out.push((value, n.attrs.contains_key("selected")));
            } else {
                stack.extend(n.children.iter().rev().copied());
            }
        }
        out
    }

    /// The value a `<select>` shows before anyone chooses: its markup's `selected` option, or else
    /// its first.
    fn default_choice(&self, select: u32) -> Option<String> {
        let options = self.options_of(select);
        options
            .iter()
            .find(|(_, selected)| *selected)
            .or(options.first())
            .map(|(v, _)| v.clone())
    }

    /// The value of the option of `select` whose value — or else whose text — is `wanted`.
    fn option_offering(&self, select: u32, wanted: &str) -> Option<String> {
        if !self
            .get(select)
            .is_some_and(|n| n.tag.eq_ignore_ascii_case("select"))
        {
            return None;
        }
        let options = self.options_of(select);
        if options.iter().any(|(v, _)| v == wanted) {
            return Some(wanted.to_string());
        }
        let mut stack: Vec<u32> = self
            .get(select)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        while let Some(h) = stack.pop() {
            let Some(n) = self.get(h) else { continue };
            if n.tag.eq_ignore_ascii_case("option") && self.text_content(h) == wanted {
                return Some(
                    n.attrs
                        .get("value")
                        .cloned()
                        .unwrap_or_else(|| wanted.to_string()),
                );
            }
            stack.extend(n.children.iter().copied());
        }
        None
    }

    fn set_checked(&mut self, node: u32, checked: bool) {
        if self.adopting(node) {
            if self
                .get(node)
                .is_some_and(|n| n.checked != n.attrs.contains_key("checked"))
            {
                self.keep_the_visitors(node);
                self.log.push(DomOp::SetChecked { node, checked });
                return;
            }
        }
        if let Some(n) = self.get_mut(node) {
            n.checked = checked;
        }
        self.log.push(DomOp::SetChecked { node, checked });
    }

    fn set_stream(&mut self, node: u32, stream: i64) {
        self.log.push(DomOp::SetStream { node, stream });
    }

    fn rtc_peer_connection(&mut self) -> u32 {
        let h = self.nodes.len() as u32;
        self.nodes.push(Node::default());
        self.log.push(DomOp::RtcPeerConnection { pc: h });
        h
    }

    fn rtc_data_channel(&mut self, pc: u32, label: &str) -> u32 {
        let h = self.nodes.len() as u32;
        self.nodes.push(Node::default());
        self.log.push(DomOp::RtcDataChannel {
            pc,
            channel: h,
            label: label.to_string(),
        });
        h
    }

    fn rtc_create_offer(&mut self, pc: u32) {
        self.log.push(DomOp::RtcCreateOffer { pc });
    }

    fn rtc_create_answer(&mut self, pc: u32) {
        self.log.push(DomOp::RtcCreateAnswer { pc });
    }

    fn rtc_set_local(&mut self, pc: u32, sdp: &str) {
        self.log.push(DomOp::RtcSetLocal {
            pc,
            sdp: sdp.to_string(),
        });
    }

    fn rtc_set_remote(&mut self, pc: u32, sdp: &str) {
        self.log.push(DomOp::RtcSetRemote {
            pc,
            sdp: sdp.to_string(),
        });
    }

    fn rtc_add_ice(&mut self, pc: u32, candidate: &str) {
        self.log.push(DomOp::RtcAddIce {
            pc,
            candidate: candidate.to_string(),
        });
    }

    fn rtc_send(&mut self, channel: u32, bytes: u64) {
        self.log.push(DomOp::RtcSend { channel, bytes });
    }

    fn get_text(&self, node: u32) -> String {
        self.get(node).map(|n| n.text.clone()).unwrap_or_default()
    }

    fn get_value(&self, node: u32) -> String {
        self.get(node).map(|n| n.value.clone()).unwrap_or_default()
    }

    fn get_checked(&self, node: u32) -> bool {
        self.get(node).is_some_and(|n| n.checked)
    }

    /// Which toggle control, if any, this element is — what a CLICK on it means. A browser's
    /// default action for a click on a checkbox is to flip `checked`; on a radio it is to select
    /// it (a second click leaves it selected). Anything else has no such default, and a click on
    /// it is only a click.
    fn toggle_control(&self, node: u32) -> Option<ToggleControl> {
        let n = self.get(node)?;
        if n.tag != "input" {
            return None;
        }
        match n.attrs.get("type").map(String::as_str) {
            Some("checkbox") => Some(ToggleControl::Checkbox),
            Some("radio") => Some(ToggleControl::Radio),
            _ => None,
        }
    }

    /// The element's TEXT CONTENT: its own text plus every descendant text node, in document
    /// order — what a reader sees, and what `the text of "#n"` means. A hole renders as its own
    /// text node, so an element's visible string is only ever the concatenation.
    fn text_content(&self, node: u32) -> String {
        let mut out = String::new();
        fn go(dom: &DomRecorder, h: u32, out: &mut String) {
            let Some(n) = dom.get(h) else { return };
            out.push_str(&n.text);
            for &c in &n.children {
                go(dom, c, out);
            }
        }
        go(self, node, &mut out);
        out
    }

    /// Every node matching `selector`, in DOCUMENT order.
    ///
    /// The walk starts at the body root, so a node detached by a re-render is no longer in the
    /// document and cannot match — exactly as in a browser. Scanning the node table instead
    /// would keep handing back the previous render's stale elements.
    fn query_selector_all(&self, selector: &str) -> Vec<u32> {
        if let Some(place) = selector.strip_prefix(PLACE) {
            return self.at_place(place).into_iter().collect();
        }
        let Some(parts) = selector::parse(selector) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut ancestors = Vec::new();
        self.collect(0, &parts, &mut ancestors, &mut out);
        out
    }

    /// Depth-first walk collecting matches. `ancestors` is the chain from the root, which is
    /// what makes the descendant combinator (`.card h1`) mean what it says.
    fn collect(&self, h: u32, parts: &[Compound], ancestors: &mut Vec<u32>, out: &mut Vec<u32>) {
        let Some(n) = self.get(h) else { return };
        if !n.is_text && self.matches_chain(h, parts, ancestors) {
            out.push(h);
        }
        ancestors.push(h);
        for &c in &n.children {
            self.collect(c, parts, ancestors, out);
        }
        ancestors.pop();
    }

    /// The chain of ancestors from the root down to (but not including) `target`.
    fn ancestors_of(&self, target: u32) -> Option<Vec<u32>> {
        fn walk(d: &DomRecorder, target: u32, h: u32, path: &mut Vec<u32>) -> bool {
            if h == target {
                return true;
            }
            let Some(n) = d.get(h) else { return false };
            path.push(h);
            for &c in &n.children {
                if walk(d, target, c, path) {
                    return true;
                }
            }
            path.pop();
            false
        }
        let mut path = Vec::new();
        walk(self, target, 0, &mut path).then_some(path)
    }

    /// Where `target` stands in the page — its child index under each ancestor from `body` down —
    /// and its tag, or `None` when it is not an attached element.
    fn place_of(&self, target: u32) -> Option<(Vec<usize>, String)> {
        let node = self.get(target).filter(|n| !n.is_text && target != 0)?;
        let chain = self.ancestors_of(target)?;
        let mut indices = Vec::with_capacity(chain.len());
        for (i, &parent) in chain.iter().enumerate() {
            let child = chain.get(i + 1).copied().unwrap_or(target);
            indices.push(
                self.get(parent)?
                    .children
                    .iter()
                    .position(|&c| c == child)?,
            );
        }
        Some((indices, node.tag.to_ascii_lowercase()))
    }

    /// The element of `tag` standing at `indices` (child indices from `body` down), if any.
    fn element_at(&self, indices: &[usize], tag: &str) -> Option<u32> {
        let mut at = 0u32;
        for &i in indices {
            at = *self.get(at)?.children.get(i)?;
        }
        self.get(at)
            .filter(|n| !n.is_text && n.tag.eq_ignore_ascii_case(tag))
            .map(|_| at)
    }

    /// A place-form [`node_address`] (without its mark) resolved: `1/0/3:button`.
    fn at_place(&self, place: &str) -> Option<u32> {
        let (path, tag) = place.rsplit_once(':')?;
        let indices: Vec<usize> = match path {
            "" => Vec::new(),
            path => path
                .split('/')
                .map(str::parse)
                .collect::<Result<_, _>>()
                .ok()?,
        };
        self.element_at(&indices, tag)
    }

    /// A focused element no longer in the document took focus with it, as a browser's focus fixup
    /// leaves nothing focused once the focused node is gone.
    fn drop_detached_focus(&mut self) {
        if self.focused != u32::MAX && self.ancestors_of(self.focused).is_none() {
            self.focused = u32::MAX;
        }
    }

    /// Whether `inner` is `outer` or lies inside it.
    fn holds(&self, outer: u32, inner: u32) -> bool {
        let mut stack = vec![outer];
        while let Some(h) = stack.pop() {
            if h == inner {
                return true;
            }
            if let Some(n) = self.get(h) {
                stack.extend(n.children.iter().copied());
            }
        }
        false
    }

    /// `node` is leaving where it stands: when focus is on it or inside it, focus leaves the page, as
    /// a browser's focus fixup takes it — a move is a removal and an insertion, so moving the
    /// focused element loses focus too — and nothing a later paint builds takes it back. The
    /// browser's `blur` on the way out reaches no handler: the host has already dropped the
    /// element's listeners.
    fn leaving(&mut self, node: u32) {
        if self.focused != u32::MAX && self.holds(node, self.focused) {
            self.focused = u32::MAX;
        }
    }

    /// Whether a visitor would SEE `target`.
    ///
    /// ★ THIS USED TO BE `query_selector(sel) >= 0` — present in the document — and `is hidden`
    /// its negation. So an element that exists and is not shown answered "visible", which makes
    /// the one question a page test asks about a toggle unable to fail: `hidden={flag}` and
    /// `class:hidden={flag}` are LGX's own ways of showing and hiding, and neither changes whether
    /// the element is in the document. `Expect "#error" to be visible.` passed in both states.
    ///
    /// Decided: the `hidden` attribute, an inline `display:none`/`visibility:hidden`, a `Style:`
    /// rule declaring either whose selector matches, and all of those on any ANCESTOR — hiding a
    /// container hides what is inside it. Not decided: anything needing a full cascade. This is a
    /// page test's question, not a rendering engine, and the boundary is written down rather than
    /// implied.
    fn shown(&self, target: u32) -> bool {
        let Some(chain) = self.ancestors_of(target) else {
            return false;
        };
        let hiding = self.hiding_rules();
        for h in chain.iter().copied().chain(std::iter::once(target)) {
            if h == 0 {
                continue;
            }
            let Some(n) = self.get(h) else { return false };
            if n.is_text {
                continue;
            }
            if n.attrs.contains_key("hidden") {
                return false;
            }
            if n.attrs.get("style").is_some_and(|v| declares_hidden(v)) {
                return false;
            }
            if hiding.iter().any(|parts| {
                self.ancestors_of(h)
                    .is_some_and(|a| self.matches_chain(h, parts, &a))
            }) {
                return false;
            }
        }
        true
    }

    /// Every selector in the document's stylesheets whose rule declares an element invisible.
    ///
    /// This is what `class:hidden={flag}` toggles — the class moves, the rule does the hiding — so
    /// leaving it out would leave the framework's own idiom for showing and hiding uncovered.
    fn hiding_rules(&self) -> Vec<Vec<Compound>> {
        let mut out = Vec::new();
        let mut sheets = Vec::new();
        self.collect_stylesheets(0, &mut sheets);
        for css in sheets {
            let Ok(items) = logicaffeine_language::lgx::css::parse_items(&css) else {
                continue;
            };
            for item in items {
                let logicaffeine_language::lgx::css::CssItem::Rule { prelude, body } = item else {
                    continue;
                };
                if !declares_hidden(&body) {
                    continue;
                }
                // A stylesheet may legitimately group selectors with a comma; the page-query
                // engine refuses one, so the group is split here rather than handed to it.
                for one in prelude.split(',') {
                    if let Some(parts) = selector::parse(one.trim()) {
                        out.push(parts);
                    }
                }
            }
        }
        out
    }

    /// Every `<style>` sheet under `h`, each its element's text content — the sheet a browser
    /// reads, however its text is split into nodes.
    fn collect_stylesheets(&self, h: u32, out: &mut Vec<String>) {
        let Some(n) = self.get(h) else { return };
        if n.tag == "style" {
            out.push(self.text_content(h));
        }
        for &c in &n.children {
            self.collect_stylesheets(c, out);
        }
    }

    /// Whether `h` matches the last compound and some ancestor chain matches the ones before it.
    fn matches_chain(&self, h: u32, parts: &[Compound], ancestors: &[u32]) -> bool {
        let Some((last, rest)) = parts.split_last() else {
            return false;
        };
        if !self.matches_compound(h, last) {
            return false;
        }
        // Walk the required ancestors right-to-left; each must appear, in order, above `h`.
        let mut i = ancestors.len();
        for want in rest.iter().rev() {
            let mut found = false;
            while i > 0 {
                i -= 1;
                if self.matches_compound(ancestors[i], want) {
                    found = true;
                    break;
                }
            }
            if !found {
                return false;
            }
        }
        true
    }

    fn matches_compound(&self, h: u32, c: &Compound) -> bool {
        let Some(n) = self.get(h) else { return false };
        if let Some(tag) = &c.tag {
            if &n.tag != tag {
                return false;
            }
        }
        if let Some(id) = &c.id {
            if n.attrs.get("id") != Some(id) {
                return false;
            }
        }
        for want in &c.classes {
            let has = n
                .attrs
                .get("class")
                .is_some_and(|v| v.split_whitespace().any(|k| k == want));
            if !has {
                return false;
            }
        }
        for (name, value) in &c.attrs {
            match (n.attrs.get(name), value) {
                (None, _) => return false,
                (Some(_), None) => {}
                (Some(have), Some(want)) if have == want => {}
                _ => return false,
            }
        }
        true
    }

    /// The first match, or the `None` handle (`u32::MAX`).
    fn query_selector(&self, selector: &str) -> u32 {
        self.query_selector_all(selector)
            .first()
            .copied()
            .unwrap_or(u32::MAX)
    }

    fn add_event_listener(&mut self, node: u32, event: &str, closure: RuntimeValue) {
        self.register(node, event);
        self.listeners.push((node, event.to_string(), closure));
        self.log.push(DomOp::AddEventListener {
            node,
            event: event.to_string(),
        });
    }

    fn listener_for(&self, node: u32, event: &str) -> Option<RuntimeValue> {
        self.listeners
            .iter()
            .find(|(n, e, _)| *n == node && e == event)
            .map(|(_, _, c)| c.clone())
    }

    // --- The correlated fetch engine (shared by the tree-walker and the VM; the
    // wasm MockDom and the browser host mirror it host-side). ---

    /// Register a request and mint its id. Contract violations (an oversized body,
    /// a header outside the allowlist) FAIL the request immediately — it is final
    /// before it would ever touch the network.
    fn fetch_send(
        &mut self,
        method: &str,
        url: &str,
        body: &str,
        headers: Vec<(String, String)>,
        closure: RuntimeValue,
    ) -> i64 {
        let headers = self.request_headers(method, url, headers);
        let req = self.next_fetch;
        self.next_fetch += 1;
        let mut prefail: Option<String> = None;
        if body.len() > FETCH_REQUEST_BODY_CAP {
            prefail = Some("toolarge".to_string());
        } else if let Some((name, _)) = headers.iter().find(|(n, _)| {
            !FETCH_ALLOWED_REQUEST_HEADERS.contains(&n.to_ascii_lowercase().as_str())
        }) {
            prefail = Some(format!(
                "network: header not allowed: {}",
                name.to_ascii_lowercase()
            ));
        }
        self.fetches.insert(
            req,
            PendingFetch {
                method: method.to_string(),
                url: url.to_string(),
                body: body.to_string(),
                headers,
                closure,
                timeout_ms: 0,
                aborted: false,
            },
        );
        if let Some(error) = prefail {
            self.fetch_responses.insert(
                req,
                FetchResponse {
                    error,
                    ..FetchResponse::default()
                },
            );
        }
        req
    }

    /// The headers a request goes out with: the page's own, and — for a state-changing request
    /// ([`changes_state`]) to the page's own server ([`to_own_server`]) that sets none — the CSRF
    /// token the document's `<meta name="csrf-token">` holds. `runtime.js`'s `requestHeaders` is the
    /// same rule in a browser.
    fn request_headers(
        &self,
        method: &str,
        url: &str,
        mut headers: Vec<(String, String)>,
    ) -> Vec<(String, String)> {
        let header = logicaffeine_language::parser::lgx_lower::CSRF_HEADER;
        let sets_one = headers.iter().any(|(n, _)| n.eq_ignore_ascii_case(header));
        if !changes_state(method) || !to_own_server(url) || sets_one {
            return headers;
        }
        let token = self.head.iter().find(|t| {
            t.tag == "meta"
                && t.attrs
                    .get("name")
                    .is_some_and(|n| n.eq_ignore_ascii_case(CSRF_META_NAME))
        });
        if let Some(token) = token.and_then(|t| t.attrs.get("content")) {
            headers.push((header.to_string(), token.clone()));
        }
        headers
    }

    /// Abort a request: if nothing was delivered yet it becomes FINAL with the
    /// `"abort"` error, so a late network response can never fire its callback.
    fn fetch_abort(&mut self, req: i64) {
        if let Some(p) = self.fetches.get_mut(&req) {
            p.aborted = true;
        }
        self.fetch_responses
            .entry(req)
            .or_insert_with(|| FetchResponse {
                error: "abort".to_string(),
                ..FetchResponse::default()
            });
    }

    /// Arm (or with `0` clear) the request's timeout. The recorder holds the value;
    /// firing is the driver's job (`fail_fetch(req, "timeout")` — the browser host
    /// arms a real timer that aborts the underlying fetch).
    fn fetch_set_timeout(&mut self, req: i64, millis: i64) {
        if let Some(p) = self.fetches.get_mut(&req) {
            p.timeout_ms = millis.max(0);
        }
    }

    /// Deliver a response. `None` if the request is unknown or already FINAL
    /// (aborted / failed / delivered — no double delivery, ever). An oversized
    /// body is converted into a delivered `"toolarge"` failure: the callback
    /// still runs, and reads the error instead of a truncated body.
    fn fetch_resolve(
        &mut self,
        req: i64,
        status: i64,
        headers: &[(String, String)],
        body: &str,
    ) -> Option<RuntimeValue> {
        if self.fetch_responses.contains_key(&req) {
            return None;
        }
        let closure = self.fetches.get(&req).map(|p| p.closure.clone())?;
        let response = if body.len() > FETCH_RESPONSE_BODY_CAP {
            FetchResponse {
                error: "toolarge".to_string(),
                ..FetchResponse::default()
            }
        } else {
            FetchResponse {
                status,
                headers: headers.to_vec(),
                body: body.to_string(),
                error: String::new(),
            }
        };
        self.fetch_responses.insert(req, response);
        self.current_fetch = Some(req);
        Some(closure)
    }

    /// Deliver a structured transport failure (`"timeout"` | `"abort"` |
    /// `"network: <detail>"` | `"toolarge"`). Same finality rule as
    /// [`Self::fetch_resolve`].
    fn fetch_fail(&mut self, req: i64, error: &str) -> Option<RuntimeValue> {
        if self.fetch_responses.contains_key(&req) {
            return None;
        }
        let closure = self.fetches.get(&req).map(|p| p.closure.clone())?;
        self.fetch_responses.insert(
            req,
            FetchResponse {
                error: error.to_string(),
                ..FetchResponse::default()
            },
        );
        self.current_fetch = Some(req);
        Some(closure)
    }

    fn fetch_response(&self, req: i64) -> Option<&FetchResponse> {
        self.fetch_responses.get(&req)
    }

    fn fetch_requests(&self) -> Vec<FetchRequest> {
        self.fetches
            .iter()
            .map(|(&req, p)| FetchRequest {
                req,
                method: p.method.clone(),
                url: p.url.clone(),
                body: p.body.clone(),
                headers: p.headers.clone(),
                timeout_ms: p.timeout_ms,
                aborted: p.aborted,
            })
            .collect()
    }

    // --- The router (shared by the tree-walker and the VM; the wasm MockDom and the
    // browser host mirror it). ---

    /// Record a navigation and move the current path. `replace` is the pushState
    /// (a new history entry) vs replaceState (swap the current one) distinction.
    fn navigate(&mut self, path: &str, replace: bool) {
        self.route_ops.push(RouteOp::Navigate {
            path: path.to_string(),
            replace,
        });
        self.route_path = path.to_string();
        // pushState adds an entry; replaceState swaps the one you are standing on. That is the
        // whole difference between them, and it is what decides where Back lands.
        if replace {
            let last = self.route_history.len() - 1;
            self.route_history[last] = path.to_string();
        } else {
            self.route_history.push(path.to_string());
        }
    }

    /// Step back one history entry, answering the path landed on — `None` at the first entry,
    /// where a browser's Back button leaves the page alone.
    fn go_back(&mut self) -> Option<String> {
        if self.route_history.len() <= 1 {
            return None;
        }
        self.route_history.pop();
        let path = self
            .route_history
            .last()
            .cloned()
            .unwrap_or_else(|| "/".to_string());
        self.route_path = path.clone();
        self.popped = true;
        Some(path)
    }

    /// A query param by name (`""` when absent — never a trap) via a tiny `k=v&k=v`
    /// scan of the current query string. The SAME parse the browser host uses.
    fn route_query_value(&self, name: &str) -> String {
        for pair in self.route_query.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                if k == name {
                    return v.to_string();
                }
            }
        }
        String::new()
    }

    /// A deterministic serialization of the tree rooted at `body` — the render the
    /// parity assertion compares alongside the op-log.
    fn render(&self) -> String {
        let mut out = String::new();
        self.render_node(0, &mut out, Spelling::Recorded);
        out
    }

    fn render_served(&self) -> String {
        let mut out = String::new();
        self.render_node(0, &mut out, Spelling::Served { chosen: None });
        out
    }

    /// The value an `<option>` submits: its `value` attribute, or else its text.
    fn option_value(&self, h: u32) -> String {
        self.get(h)
            .and_then(|n| n.attrs.get("value").cloned())
            .unwrap_or_else(|| self.text_content(h))
    }

    fn render_node(&self, h: u32, out: &mut String, spelling: Spelling) {
        let Some(n) = self.get(h) else { return };
        if n.is_text {
            escape_text(&n.text, out);
            return;
        }
        let served = matches!(spelling, Spelling::Served { .. });
        let option = n.tag.eq_ignore_ascii_case("option");
        let chosen = match spelling {
            Spelling::Served { chosen: Some(v) } if option => Some(self.option_value(h) == v),
            _ => None,
        };
        out.push('<');
        out.push_str(&n.tag);
        for (k, v) in &n.attrs {
            if chosen.is_some() && k.eq_ignore_ascii_case("selected") {
                continue;
            }
            out.push_str(&format!(" {k}=\""));
            escape_attr(v, out);
            out.push('"');
        }
        if chosen == Some(true) {
            out.push_str(" selected");
        }
        if n.checked {
            // Bare, because it is a presence. `checked="false"` is CHECKED in every browser
            // there is, so the only honest encoding of false is nothing at all.
            out.push_str(" checked");
        }
        // A `<select>`'s value is written only where it differs from the option its markup already
        // selects — the one rule a browser's document says it by, since the markup alone shows that
        // choice; every other control's value is written whenever it holds one.
        let select = n.tag.eq_ignore_ascii_case("select");
        let textarea = n.tag.eq_ignore_ascii_case("textarea");
        let shown = match select {
            true => self.default_choice(h).as_deref() != Some(n.value.as_str()),
            false => true,
        };
        // Served, a control's state is spelled the way HTML's parser reads it back: a `<select>`'s
        // value as the `selected` option, a `<textarea>`'s as its content.
        if shown && !n.value.is_empty() && !(served && (select || textarea)) {
            out.push_str(" value=\"");
            escape_attr(&n.value, out);
            out.push('"');
        }
        out.push('>');
        let inner = match spelling {
            Spelling::Served { .. } if select && shown && !n.value.is_empty() => Spelling::Served {
                chosen: Some(n.value.as_str()),
            },
            other => other,
        };
        // ★ A VOID ELEMENT HAS NO CLOSING TAG. `<input>`, `<br>`, `<img>` and their siblings hold
        // no content, and HTML's own parser treats a `</input>` as a stray tag — dropping it, or
        // (the `<br></br>` case) reading it as a SECOND element. This serializer closed everything
        // it wrote, so `largo build`'s pre-rendered page — the markup a crawler and a scripts-off
        // reader actually receive — served malformed HTML for every form on it. The server-side
        // `-> View` walk already consulted the same list, so the two renderers of one view
        // disagreed about the same element.
        if logicaffeine_language::lgx::html::is_void(&n.tag) {
            return;
        }
        // A raw-text element's content — `<style>`, `<script>` — is not markup: HTML's parser reads
        // it verbatim, so it is written verbatim, whether the element holds it as its own text or
        // as a text child. Escaped, `.list > li` would reach the browser as `.list &gt; li`, which
        // is no selector.
        let raw = logicaffeine_language::lgx::html::is_raw_text(&n.tag);
        // HTML's parser drops one newline straight after `<pre>`, `<listing>` and `<textarea>`, so
        // served content that begins with one is given a second for it to drop.
        let drops_newline = served
            && ["pre", "listing", "textarea"]
                .iter()
                .any(|t| n.tag.eq_ignore_ascii_case(t));
        if served && textarea && !n.value.is_empty() {
            if drops_newline && n.value.starts_with('\n') {
                out.push('\n');
            }
            escape_text(&n.value, out);
            out.push_str(&format!("</{}>", n.tag));
            return;
        }
        let leading = if n.text.is_empty() {
            n.children
                .first()
                .and_then(|&c| self.get(c))
                .filter(|t| t.is_text)
                .map(|t| t.text.as_str())
        } else {
            Some(n.text.as_str())
        };
        if drops_newline && leading.is_some_and(|t| t.starts_with('\n')) {
            out.push('\n');
        }
        if raw {
            out.push_str(&n.text);
        } else {
            escape_text(&n.text, out);
        }
        out.push_str(&n.html);
        for &c in &n.children {
            match self.get(c) {
                Some(t) if raw && t.is_text => out.push_str(&t.text),
                _ => self.render_node(c, out, inner),
            }
        }
        out.push_str(&format!("</{}>", n.tag));
    }
}

/// How [`DomRecorder::render_node`] spells a document: as the recorder holds it — every tier's
/// comparison surface — or as a browser must be sent it to build the same page.
#[derive(Clone, Copy)]
enum Spelling<'v> {
    Recorded,
    /// `chosen`: the value of the `<select>` being written, when it is not the option its markup
    /// selects.
    Served {
        chosen: Option<&'v str>,
    },
}

/// Escape text content so a value can never be read as markup — the guarantee that makes a
/// `{hole}` safe to fill with anything a user typed.
/// Whether a block of CSS declarations makes an element invisible.
///
/// `display:none` removes it from the layout; `visibility:hidden` keeps its space and paints
/// nothing. A visitor cannot see either, which is the question being asked.
fn declares_hidden(css: &str) -> bool {
    let flat: String = css
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    flat.contains("display:none") || flat.contains("visibility:hidden")
}

fn escape_text(s: &str, out: &mut String) {
    out.push_str(&logicaffeine_language::lgx::html::escape_text(s));
}

/// Escape an attribute value: the text escapes plus the quote that delimits it.
fn escape_attr(s: &str, out: &mut String) {
    out.push_str(&logicaffeine_language::lgx::html::escape_attr(s));
}

thread_local! {
    /// The DOM the current native-tier run is building. Reset per run via [`reset`].
    static DOM: RefCell<DomRecorder> = RefCell::new(DomRecorder::default());
    /// The head of the shell this world's page is served in ([`arrive`]) — what every fresh load
    /// of the page starts from. Empty when no shell was named: a run that is not a page load.
    static SHELL: RefCell<Vec<HeadTag>> = const { RefCell::new(Vec::new()) };
}

/// Start a fresh DOM (call before each tier's run so op-logs are comparable) — a fresh load of the
/// page: its head is the shell's again, whatever the last load merged into it.
pub fn reset() {
    DOM.with(|d| {
        *d.borrow_mut() = DomRecorder {
            head: SHELL.with(|s| s.borrow().clone()),
            ..DomRecorder::default()
        }
    });
    // The declared head belongs to the document being built, so it is reset with it. Left
    // behind, the previous page's title would ride onto the next one — and the CLI
    // renders every page of a site in one process.
    HEAD_HTML.with(|h| *h.borrow_mut() = None);
}

/// `Reload the page.` as a browser reloads it: a fresh document loaded from the SAME server at the
/// SAME address, arriving in the same shell (`DomRecorder::next_document`).
pub fn reload() {
    let shell = SHELL.with(|s| s.borrow().clone());
    DOM.with(|d| {
        let mut d = d.borrow_mut();
        let before = std::mem::take(&mut *d);
        *d = before.next_document(shell);
    });
    HEAD_HTML.with(|h| *h.borrow_mut() = None);
}

/// Another document loads at `url` — a link followed, or a [`request_load`] carried out. The page
/// that asked is gone; the location and the back stack move to `url`; the load is the first thing
/// the new document's navigation log records ([`RouteOp::Load`]); and the document is empty, since
/// what `url` serves is another page's content, outside the page under test. The test's own
/// answers for requests survive, as they do across a reload.
pub fn assign_location(url: &str) {
    DOM.with(|d| {
        let mut d = d.borrow_mut();
        let before = std::mem::take(&mut *d);
        *d = before.next_document(Vec::new());
    });
    HEAD_HTML.with(|h| *h.borrow_mut() = None);
    move_location(url);
    DOM.with(|d| {
        d.borrow_mut().route_ops.push(RouteOp::Load {
            path: url.to_string(),
        })
    });
}

/// The browser moves to `url` to load a document there: the location, its query string and the
/// back stack move.
fn move_location(url: &str) {
    DOM.with(|d| {
        let mut d = d.borrow_mut();
        d.route_history.push(url.to_string());
        d.route_path = url.to_string();
        d.route_query = url
            .split_once('?')
            .map(|(_, q)| q.split('#').next().unwrap_or("").to_string())
            .unwrap_or_default();
    });
}

/// A form the browser submitted itself (a `POST` no listener cancelled) is answered with a new
/// document: the browser is at `url`, the address it posted to, and the document is the response.
/// The document is the one a browser builds for the response's type: a `text/html` response is
/// the page it holds — its `<head>` and `<body>`, or the markup as the body when it is a fragment; a
/// JSON response is shown in the browser's JSON viewer, the raw text in a preformatted block; any
/// other response is shown as plain text. The test's answers for requests survive, as they survive
/// a reload.
pub fn follow_response(url: &str, content_type: &str, body: &str) {
    let media = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let lower = body.to_ascii_lowercase();
    let section = |tag: &str| {
        let open = lower
            .find(&format!("<{tag}"))
            .and_then(|at| lower[at..].find('>').map(|e| at + e + 1))?;
        let close = lower.find(&format!("</{tag}>")).filter(|c| *c >= open)?;
        Some(body[open..close].to_string())
    };
    let head = match media.as_str() {
        "text/html" => section("head").map(|h| parse_head(&h)).unwrap_or_default(),
        "application/json" => {
            parse_head(r#"<meta name="color-scheme" content="light dark"><meta charset="utf-8">"#)
        }
        _ => parse_head(r#"<meta name="color-scheme" content="light dark">"#),
    };
    DOM.with(|d| {
        let mut d = d.borrow_mut();
        let before = std::mem::take(&mut *d);
        *d = before.next_document(head);
    });
    HEAD_HTML.with(|h| *h.borrow_mut() = None);
    move_location(url);
    let root = self::body();
    let preformatted = |style: Option<&str>| {
        let pre = create_element("pre");
        if let Some(style) = style {
            set_attribute(pre, "style", style);
        }
        let text = create_text_node(body);
        append_child(pre, text);
        append_child(root, pre);
    };
    match media.as_str() {
        "text/html" => set_html(root, section("body").as_deref().unwrap_or(body).trim()),
        "application/json" => {
            preformatted(None);
            let viewer = create_element("div");
            set_attribute(viewer, "class", "json-formatter-container");
            append_child(root, viewer);
        }
        _ => preformatted(Some("word-wrap: break-word; white-space: pre-wrap;")),
    }
}

/// `assignLocation(url)` — `location.assign`: ask the browser to load the document at `url` once
/// the running script is done. Everything the page draws until then is drawn into the page that
/// asked, which the browser keeps on screen until the new document replaces it; a second ask
/// before then replaces the first.
pub fn request_load(url: &str) {
    DOM.with(|d| d.borrow_mut().load_requested = Some(url.to_string()));
}

/// Where the page asked the browser to go since the last look ([`request_load`]) — taken, so the
/// host carries each request out once, by [`assign_location`].
pub fn take_load_request() -> Option<String> {
    DOM.with(|d| d.borrow_mut().load_requested.take())
}

/// `location.reload()`: ask the browser to load the page again once the running script is done.
pub fn request_reload() {
    DOM.with(|d| d.borrow_mut().reload_requested = true);
}

/// Whether the page asked to be loaded again since the last look ([`request_reload`]) — taken, so
/// the host carries each request out once.
pub fn take_reload_request() -> bool {
    DOM.with(|d| std::mem::take(&mut d.borrow_mut().reload_requested))
}

/// Whether the browser stepped back through the history since the last look — the `popstate` the
/// host then delivers to every route handler, as a browser raises it after `history.back()`.
/// Taken, so each step is delivered once.
pub fn take_popped() -> bool {
    DOM.with(|d| std::mem::take(&mut d.borrow_mut().popped))
}

impl DomRecorder {
    /// The recorder for the next document this browser loads, its head `head`. What the test
    /// arranged beyond the page — the answers it gave for requests (`Given that … answers …`) — and
    /// where the browser is — the location and its back stack — carry over; the document and
    /// everything the page did in it start over, exactly as [`reset`] starts them.
    ///
    /// ★ EVERY RECORDER FIELD IS CLASSIFIED HERE. The destructuring names each field of
    /// [`DomRecorder`] as kept or started over, so a field added to the recorder does not compile
    /// until someone decides which a new document does to it.
    fn next_document(self, head: Vec<HeadTag>) -> DomRecorder {
        let DomRecorder {
            // Kept: the test's server, and where the browser is.
            fetch_stubs,
            route_path,
            route_query,
            route_history,
            // Started over: the document and everything the page did in it.
            nodes: _,
            streamed: _,
            log: _,
            listeners: _,
            registrations: _,
            head: _,
            clipboard: _,
            downloads: _,
            notifications: _,
            published: _,
            scrolled_to: _,
            selected: _,
            focused: _,
            raised: _,
            current_event: _,
            fetches: _,
            next_fetch: _,
            fetch_responses: _,
            current_fetch: _,
            route_ops: _,
            route_handlers: _,
            dropped_file: _,
            popped: _,
            reload_requested: _,
            load_requested: _,
            hydration: _,
            forwarded: _,
        } = self;
        DomRecorder {
            fetch_stubs,
            route_path,
            route_query,
            route_history,
            head,
            ..DomRecorder::default()
        }
    }
}

/// The file the drop being delivered carries: what `eventFileName`/`eventFileSize`/`eventFileType`/
/// `eventFileBytes` answer on a tier with no real transfer to read. The host that delivers a drop
/// sets it first (`page_host::Drive::DropFile`), exactly as a browser's drop event carries its
/// file into the listener it reaches.
pub fn set_dropped_file(file: DroppedFile) {
    DOM.with(|d| d.borrow_mut().dropped_file = Some(file));
}

/// The file the drop being delivered carries ([`set_dropped_file`]) — no file (no name, no type,
/// no content) when no drop has carried one.
pub fn dropped_file() -> DroppedFile {
    DOM.with(|d| d.borrow().dropped_file.clone())
        .unwrap_or_default()
}

/// A browser's own table for naming a file's type from its extension: Chromium 154.0.8037.57's
/// `net/base/mime_util.cc`, its `kPrimaryMappings` and then its `kSecondaryMappings`, searched in
/// this order for the first entry whose extensions name the file's, ASCII case-insensitively —
/// mirrored entry for entry, so the two can be read against each other. Between the two lists a
/// browser asks the visitor's desktop (Linux's shared MIME database, Windows' registry, macOS's
/// type system) for an extension the primary list does not name, so a secondary-list answer is
/// the browser's where the desktop is silent or agrees; that layer is the machine's, and the
/// portable part of the browser's rule is these two lists.
const BROWSER_PRIMARY_TYPES: &[(&str, &str)] = &[
    ("video/webm", "webm"),
    ("audio/mpeg", "mp3"),
    ("application/wasm", "wasm"),
    ("application/x-chrome-extension", "crx"),
    ("application/xhtml+xml", "xhtml,xht,xhtm"),
    ("audio/flac", "flac"),
    ("audio/matroska", "mka"),
    ("audio/mp3", "mp3"),
    ("audio/ogg", "ogg,oga,opus"),
    ("audio/wav", "wav"),
    ("audio/webm", "webm"),
    ("audio/x-m4a", "m4a"),
    ("image/avif", "avif"),
    ("image/gif", "gif"),
    ("image/jpeg", "jpeg,jpg,jpe"),
    ("image/jxl", "jxl"),
    ("image/png", "png"),
    ("image/apng", "png,apng"),
    ("image/svg+xml", "svg,svgz"),
    ("image/webp", "webp"),
    ("multipart/related", "mht,mhtml"),
    ("text/css", "css"),
    ("text/html", "html,htm,shtml,shtm"),
    ("text/javascript", "js,mjs"),
    ("text/xml", "xml"),
    ("video/matroska", "mkv"),
    ("video/mp4", "mp4,m4v"),
    ("video/ogg", "ogv,ogm"),
    ("text/csv", "csv"),
];

const BROWSER_SECONDARY_TYPES: &[(&str, &str)] = &[
    ("image/x-icon", "ico"),
    ("application/epub+zip", "epub"),
    ("application/font-woff", "woff"),
    ("application/gzip", "gz,tgz"),
    ("application/javascript", "js"),
    ("application/json", "json"),
    ("application/msword", "doc,dot"),
    ("application/octet-stream", "bin,exe,com"),
    ("application/pdf", "pdf"),
    ("application/pkcs7-mime", "p7m,p7c,p7z"),
    ("application/pkcs7-signature", "p7s"),
    ("application/postscript", "ps,eps,ai"),
    ("application/rdf+xml", "rdf"),
    ("application/rss+xml", "rss"),
    ("application/rtf", "rtf"),
    ("application/vnd.android.package-archive", "apk"),
    ("application/vnd.mozilla.xul+xml", "xul"),
    ("application/vnd.ms-excel", "xls"),
    ("application/vnd.ms-powerpoint", "ppt"),
    (
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "pptx",
    ),
    (
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "xlsx",
    ),
    (
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "docx",
    ),
    ("application/x-gzip", "gz,tgz"),
    ("application/x-mpegurl", "m3u8"),
    ("application/x-shockwave-flash", "swf,swl"),
    ("application/x-tar", "tar"),
    ("application/x-x509-ca-cert", "cer,crt"),
    ("application/zip", "zip"),
    ("audio/webm", "weba"),
    ("image/bmp", "bmp"),
    ("image/jpeg", "jfif,pjpeg,pjp"),
    ("image/tiff", "tiff,tif"),
    ("image/vnd.microsoft.icon", "ico"),
    ("image/x-png", "png"),
    ("image/x-xbitmap", "xbm"),
    ("message/rfc822", "eml"),
    ("text/calendar", "ics"),
    ("text/html", "ehtml"),
    ("text/markdown", "md"),
    ("text/plain", "txt,text"),
    ("text/vtt", "vtt"),
    ("text/x-sh", "sh"),
    ("text/xml", "xsl,xbl,xslt"),
    ("video/mpeg", "mpeg,mpg,mpe"),
];

/// The type a browser gives a file named `name`: by the extension after its last `.`, from the
/// browser's own table; `""` for a name with no extension, or with one the table does not name.
///
/// The ONE mapping every tier that synthesizes a file from its name reads — a drop driven by a
/// name alone, a `File` record that declares no type — so `file's type` on the tree-walker and the
/// lowered VM is what a visitor's browser reports for that name.
pub fn browser_mime_type(name: &str) -> &'static str {
    let Some(dot) = name.rfind('.') else {
        return "";
    };
    let ext = &name[dot + 1..];
    if ext.is_empty() {
        return "";
    }
    BROWSER_PRIMARY_TYPES
        .iter()
        .chain(BROWSER_SECONDARY_TYPES)
        .find(|(_, extensions)| {
            extensions
                .split(',')
                .any(|known| known.eq_ignore_ascii_case(ext))
        })
        .map_or("", |(mime_type, _)| mime_type)
}

/// A file a drop carries: its name, the type its author declared for it when they declared one,
/// and its content. Its size is always its content's length, as a browser's `File.size` is; its
/// type ([`DroppedFile::mime_type`]) is the declared one, and otherwise what a browser names the
/// file from its name ([`browser_mime_type`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DroppedFile {
    pub name: String,
    pub declared_type: Option<String>,
    pub bytes: Vec<u8>,
}

impl DroppedFile {
    /// A file known by its name alone: no declared type, no content.
    pub fn named(name: &str) -> DroppedFile {
        DroppedFile {
            name: name.to_string(),
            declared_type: None,
            bytes: Vec::new(),
        }
    }

    /// The file `a new File with name … and size … and type … and bytes …` declares. Its content is
    /// `bytes` when they are declared; a `size` declared without them is that many zero bytes. A
    /// declared size that is not the count of the declared bytes describes no file a browser can
    /// hold, and is refused, as is a byte outside `0..=255`. A type left undeclared is the one a
    /// browser gives the name, exactly as a file known by its name alone gets it.
    pub fn declared(
        name: String,
        size: Option<i64>,
        declared_type: Option<String>,
        bytes: Option<Vec<i64>>,
    ) -> Result<DroppedFile, String> {
        let content = match (bytes, size) {
            (Some(bytes), size) => {
                if let Some(bad) = bytes.iter().find(|b| !(0..=255).contains(*b)) {
                    return Err(format!(
                        "the file {name:?} declares the byte {bad}, and a byte is a number from 0 \
                         to 255"
                    ));
                }
                if let Some(size) = size.filter(|s| *s != 0 && *s != bytes.len() as i64) {
                    return Err(format!(
                        "the file {name:?} declares size {size} and {} bytes; a file's size is the \
                         count of its bytes, so declare one or the other",
                        bytes.len()
                    ));
                }
                bytes.into_iter().map(|b| b as u8).collect()
            }
            (None, Some(size)) if size < 0 => {
                return Err(format!(
                    "the file {name:?} declares size {size}, and a size is never negative"
                ))
            }
            (None, Some(size)) => vec![0; size as usize],
            (None, None) => Vec::new(),
        };
        Ok(DroppedFile {
            name,
            declared_type,
            bytes: content,
        })
    }

    /// The file's type as a handler reads it (`file's type`): the one its author declared, or the
    /// one a browser gives its name.
    pub fn mime_type(&self) -> &str {
        self.declared_type
            .as_deref()
            .unwrap_or_else(|| browser_mime_type(&self.name))
    }

    /// The file a raised value names: a `File` record, read by [`DroppedFile::declared`]'s rule
    /// (a field the record leaves out, or holds as `nothing`, is undeclared), or a Text naming a
    /// file by its name alone ([`DroppedFile::named`]).
    pub fn from_value(value: &RuntimeValue) -> Result<DroppedFile, String> {
        let RuntimeValue::Struct(record) = value else {
            return Ok(DroppedFile::named(&value.to_display_string()));
        };
        if record.type_name != "File" {
            return Err(format!(
                "a dropped file is a `File` or its name as Text, and this is a {}",
                record.type_name
            ));
        }
        let field = |name: &str| {
            record
                .fields
                .get(name)
                .filter(|v| !matches!(v, RuntimeValue::Nothing))
        };
        let text = |name: &str| field(name).map(RuntimeValue::to_display_string);
        let size = match field("size") {
            Some(RuntimeValue::Int(n)) => Some(*n),
            Some(other) => {
                return Err(format!(
                    "a `File`'s size is an Int, and this one is {}",
                    other.type_name()
                ))
            }
            None => None,
        };
        let bytes = match field("bytes") {
            Some(RuntimeValue::List(list)) => {
                let list = list.borrow();
                let mut out = Vec::with_capacity(list.len());
                for i in 0..list.len() {
                    match list.get(i) {
                        Some(RuntimeValue::Int(n)) => out.push(n),
                        other => {
                            return Err(format!(
                                "a `File`'s bytes are Ints, and byte {} is {}",
                                i + 1,
                                other.map_or("missing".to_string(), |v| v.type_name().to_string())
                            ))
                        }
                    }
                }
                Some(out)
            }
            Some(other) => {
                return Err(format!(
                    "a `File`'s bytes are a Seq of Int, and these are {}",
                    other.type_name()
                ))
            }
            None => None,
        };
        DroppedFile::declared(text("name").unwrap_or_default(), size, text("type"), bytes)
    }

    /// The file's size in bytes.
    pub fn size(&self) -> i64 {
        self.bytes.len() as i64
    }

    /// The `File` record a handler binds: `name`, `size`, `type` and `bytes`.
    pub fn to_value(&self) -> RuntimeValue {
        let mut fields = std::collections::HashMap::new();
        fields.insert(
            "name".to_string(),
            RuntimeValue::Text(std::rc::Rc::new(self.name.clone())),
        );
        fields.insert("size".to_string(), RuntimeValue::Int(self.size()));
        fields.insert(
            "type".to_string(),
            RuntimeValue::Text(std::rc::Rc::new(self.mime_type().to_string())),
        );
        fields.insert(
            "bytes".to_string(),
            crate::semantics::builtins::bytes_to_seq(&self.bytes),
        );
        RuntimeValue::Struct(Box::new(crate::interpreter::StructValue {
            type_name: "File".to_string(),
            fields,
        }))
    }
}

/// The page arrives, served in `document` — the shell HTML a browser loads it in: from now on this
/// world's document starts from that shell's head, this load and every reload after it, so a page
/// the host drives in process arrives in exactly the document a browser holds. The shell's body is
/// not read: the page paints its body itself.
pub fn arrive(document: &str) {
    let lower = document.to_ascii_lowercase();
    let head = match (lower.find("<head>"), lower.find("</head>")) {
        (Some(open), Some(close)) if open < close => {
            parse_head(&document[open + "<head>".len()..close])
        }
        _ => Vec::new(),
    };
    SHELL.with(|s| *s.borrow_mut() = head);
    reset();
}

/// Everything a document holds: its tree, its once-per-mount declared head, and the shell it is
/// served in. What [`take`] saves and [`restore`] puts back.
#[derive(Default)]
pub struct DomState {
    dom: DomRecorder,
    head: Option<String>,
    shell: Vec<HeadTag>,
}

/// Take the current document, leaving a fresh one in its place — [`reset`], but the document it
/// replaces comes back to the caller instead of being dropped. A nested run (a candidate page
/// tier compared against the reference) takes the caller's document before it paints its own, and
/// [`restore`]s it when it is done, so the caller's own document survives the comparison.
pub fn take() -> DomState {
    DomState {
        dom: DOM.with(RefCell::take),
        head: HEAD_HTML.with(RefCell::take),
        shell: SHELL.with(RefCell::take),
    }
}

/// Put back a document a matching [`take`] returned.
pub fn restore(state: DomState) {
    DOM.with(|d| *d.borrow_mut() = state.dom);
    HEAD_HTML.with(|h| *h.borrow_mut() = state.head);
    SHELL.with(|s| *s.borrow_mut() = state.shell);
}

/// Runs `change` against the document and keeps what it did only if it succeeds: on an `Err` the
/// document is exactly as it was before, op log included.
pub fn atomically<T>(change: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let before = DOM.with(|d| d.borrow().clone());
    let result = change();
    if result.is_err() {
        DOM.with(|d| *d.borrow_mut() = before);
    }
    result
}

/// The ordered op-log recorded so far — the parity assertion surface.
pub fn op_log() -> Vec<DomOp> {
    DOM.with(|d| d.borrow().log.clone())
}

/// A deterministic render of the current tree rooted at `body`.
/// The HTML of a node's CHILDREN — what a view value renders as. The fragment that holds a
/// rendered view is scaffolding, never markup, so it never appears in the output.
pub fn render_children(node: i64) -> String {
    DOM.with(|d| {
        let dom = d.borrow();
        let mut out = String::new();
        let kids = dom
            .get(node as u32)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        for child in kids {
            dom.render_node(child, &mut out, Spelling::Recorded);
        }
        out
    })
}

pub fn render() -> String {
    DOM.with(|d| d.borrow().render())
}

/// [`render`] as a browser must be sent it to build the same page: a control's state is spelled
/// the way HTML's parser reads it back — a `<select>`'s value as its `selected` option, a
/// `<textarea>`'s as its content — and content a parser would lose a leading newline of carries
/// one more. What `largo build` puts in a pre-rendered page's body.
pub fn render_served() -> String {
    DOM.with(|d| d.borrow().render_served())
}

thread_local! {
    /// The HTML a page's `Head:` rendered to, set once at mount.
    ///
    /// A thread-local beside the document recorder rather than a node inside it: the
    /// recorder models what a browser paints, and the head is not painted — it is part
    /// of the document the SERVER sends, read by `largo build` and `largo serve` when
    /// they compose the page. Putting it in the tree would put it in `render()`, which
    /// every op-log assertion in the suite reads as the body.
    static HEAD_HTML: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Record the head HTML a page rendered, and merge it into the document's head the way a browser
/// host does (`runtime.js`'s `mergeIntoHead`): each element with an identity replaces every element
/// of that identity already there, and joins the head at its end; one without is simply added.
/// Called once, at mount.
///
/// What is recorded ([`head_html`]) is the page's declared head after that same merge — a layout's
/// `Head:` and the page's own arrive as one string, and a server composing the document from it
/// must see the page's title, not both — each surviving tag exactly as it was written.
pub fn set_head_html(html: &str) {
    let mut declared: Vec<(Option<String>, &str)> = Vec::new();
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        let tag = &rest[open..];
        let end = if tag.to_ascii_lowercase().starts_with("<title") {
            tag.to_ascii_lowercase()
                .find("</title>")
                .map(|e| e + "</title>".len())
        } else {
            tag.find('>').map(|e| e + 1)
        };
        let Some(end) = end else { break };
        let markup = &tag[..end];
        let identity = parse_head(markup).first().and_then(HeadTag::identity);
        if let Some(identity) = &identity {
            declared.retain(|(held, _)| held.as_ref() != Some(identity));
        }
        declared.push((identity, markup));
        rest = &tag[end..];
    }
    let merged: String = declared.iter().map(|(_, markup)| *markup).collect();
    HEAD_HTML.with(|h| *h.borrow_mut() = Some(merged));
    DOM.with(|d| {
        let head = &mut d.borrow_mut().head;
        for tag in parse_head(html) {
            if let Some(identity) = tag.identity() {
                head.retain(|t| t.identity().as_deref() != Some(identity.as_str()));
            }
            head.push(tag);
        }
    });
}

/// The head HTML the last mounted page declared, if it declared one. `None` means the
/// page has no `Head:` — which is not the same as an EMPTY head, and the difference is
/// whether the default shell's title stands. What a server composes the page's document from.
pub fn head_html() -> Option<String> {
    HEAD_HTML.with(|h| h.borrow().clone())
}

/// The document's head as it stands — the shell it arrived in with the page's own `Head:` merged
/// over it — element by element in the reference renderer's serialization. `None` when the head
/// holds no element.
pub fn document_head() -> Option<String> {
    DOM.with(|d| {
        let markup: String = d.borrow().head.iter().map(HeadTag::markup).collect();
        (!markup.is_empty()).then_some(markup)
    })
}

/// One element of a document's `<head>`: its tag, its attributes, and the text it holds — a
/// `<title>`'s, a `<style>`'s or `<script>`'s raw content, nothing for a void element.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeadTag {
    pub tag: String,
    /// Attribute name → decoded value, in name order.
    pub attrs: BTreeMap<String, String>,
    pub text: String,
}

impl HeadTag {
    /// What makes two head elements THE SAME element — the identity a page's own version
    /// replaces; `None` for one that replaces nothing and is simply added. The keys of
    /// `runtime.js`'s `headIdentity`, read off the same attributes: an attribute counts by its
    /// presence and its own value, never by text that happens to appear inside another's value. A
    /// `<meta>` named with a `media` (`theme-color` for light and for dark) keys on both, so one
    /// never replaces the other.
    pub fn identity(&self) -> Option<String> {
        let attr = |name: &str| self.attrs.get(name).map(|v| v.to_ascii_lowercase());
        match self.tag.as_str() {
            "title" => Some("title".to_string()),
            "base" => Some("base".to_string()),
            "meta" if self.attrs.contains_key("charset") => Some("meta:charset".to_string()),
            "meta" => match (attr("name"), attr("property")) {
                (Some(name), _) => Some(match attr("media") {
                    Some(media) => format!("meta:name={name}@{media}"),
                    None => format!("meta:name={name}"),
                }),
                (None, Some(property)) => Some(format!("meta:property={property}")),
                (None, None) => None,
            },
            "link" => attr("rel").map(|rel| format!("link:rel={rel}")),
            _ => None,
        }
    }

    /// The element in the reference renderer's serialization: attributes in name order, a void
    /// element unclosed, a raw-text element's content verbatim, any other text escaped.
    pub fn markup(&self) -> String {
        use logicaffeine_language::lgx::html::{is_raw_text, is_void};
        let mut out = format!("<{}", self.tag);
        for (name, value) in &self.attrs {
            out.push_str(&format!(" {name}=\""));
            escape_attr(value, &mut out);
            out.push('"');
        }
        out.push('>');
        if is_void(&self.tag) {
            return out;
        }
        if is_raw_text(&self.tag) {
            out.push_str(&self.text);
        } else {
            escape_text(&self.text, &mut out);
        }
        out.push_str(&format!("</{}>", self.tag));
        out
    }
}

/// Head markup — a page's rendered `Head:`, or the head of the shell it is served in — as its
/// elements, in order. What a browser's parse keeps of it: element names and attribute names fold
/// to lower case, the first of two same-named attributes wins, character references decode (never
/// inside `<style>`/`<script>`), text between elements is formatting and is dropped, and an
/// `<html …>` is a statement about the document element, never an element of the head.
pub fn parse_head(markup: &str) -> Vec<HeadTag> {
    use logicaffeine_language::lgx::html::{decode_entities, is_raw_text, is_void};
    let mut out = Vec::new();
    let mut rest = markup;
    while let Some(lt) = rest.find('<') {
        rest = &rest[lt + 1..];
        if let Some(comment) = rest.strip_prefix("!--") {
            rest = comment.find("-->").map_or("", |end| &comment[end + 3..]);
            continue;
        }
        if rest.starts_with(['/', '!', '?']) {
            rest = rest.find('>').map_or("", |end| &rest[end + 1..]);
            continue;
        }
        let name_end = rest
            .find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
            .unwrap_or(rest.len());
        let tag = rest[..name_end].to_ascii_lowercase();
        rest = &rest[name_end..];
        let mut attrs = BTreeMap::new();
        loop {
            rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace());
            if let Some(after) = rest.strip_prefix('>') {
                rest = after;
                break;
            }
            if let Some(after) = rest.strip_prefix('/') {
                rest = after;
                continue;
            }
            if rest.is_empty() {
                break;
            }
            let name_end = rest
                .find(|c: char| c.is_ascii_whitespace() || c == '=' || c == '>' || c == '/')
                .unwrap_or(rest.len());
            let name = rest[..name_end].to_ascii_lowercase();
            rest = rest[name_end..].trim_start_matches(|c: char| c.is_ascii_whitespace());
            let value = match rest.strip_prefix('=') {
                None => String::new(),
                Some(after) => {
                    let after = after.trim_start_matches(|c: char| c.is_ascii_whitespace());
                    match after.chars().next() {
                        Some(quote @ ('"' | '\'')) => {
                            let body = &after[1..];
                            let end = body.find(quote).unwrap_or(body.len());
                            rest = body.get(end + 1..).unwrap_or("");
                            decode_entities(&body[..end])
                        }
                        _ => {
                            let end = after
                                .find(|c: char| c.is_ascii_whitespace() || c == '>')
                                .unwrap_or(after.len());
                            rest = &after[end..];
                            decode_entities(&after[..end])
                        }
                    }
                }
            };
            if !name.is_empty() {
                attrs.entry(name).or_insert(value);
            }
        }
        let mut text = String::new();
        if !is_void(&tag) {
            let close = format!("</{tag}");
            let end = rest.to_ascii_lowercase().find(&close).unwrap_or(rest.len());
            text = if is_raw_text(&tag) {
                rest[..end].to_string()
            } else {
                decode_entities(&rest[..end])
            };
            rest = &rest[end..];
            rest = rest.find('>').map_or("", |e| &rest[e + 1..]);
        }
        if tag != "html" && !tag.is_empty() {
            out.push(HeadTag { tag, attrs, text });
        }
    }
    out
}

// --- The operations the DOM builtins call (handles are `i64` for the value model). ---

pub fn body() -> i64 {
    DOM.with(|d| d.borrow().body() as i64)
}
pub fn create_element(tag: &str) -> i64 {
    DOM.with(|d| d.borrow_mut().create_element(tag) as i64)
}
pub fn create_text_node(text: &str) -> i64 {
    DOM.with(|d| d.borrow_mut().create_text_node(text) as i64)
}
/// A new element under the id a patch stream names for it — see [`STREAM_ID_BASE`].
pub fn create_element_as(node: i64, tag: &str) -> Result<(), String> {
    let id = u32::try_from(node)
        .map_err(|_| format!("the stream names node {node}, which is no node id"))?;
    DOM.with(|d| d.borrow_mut().create_element_as(id, tag))
}
/// A new text node under the id a patch stream names for it — see [`STREAM_ID_BASE`].
pub fn create_text_node_as(node: i64, text: &str) -> Result<(), String> {
    let id = u32::try_from(node)
        .map_err(|_| format!("the stream names node {node}, which is no node id"))?;
    DOM.with(|d| d.borrow_mut().create_text_node_as(id, text))
}
pub fn set_text(node: i64, text: &str) {
    DOM.with(|d| d.borrow_mut().set_text(node as u32, text));
}
pub fn set_html(node: i64, html: &str) {
    DOM.with(|d| d.borrow_mut().set_html(node as u32, html));
}
pub fn clear_children(node: i64) {
    DOM.with(|d| d.borrow_mut().clear_children(node as u32));
}
pub fn set_attribute(node: i64, name: &str, value: &str) {
    DOM.with(|d| d.borrow_mut().set_attribute(node as u32, name, value));
}
/// Set an attribute from a VALUE rather than from a string — the ONE place the boolean-attribute
/// rule lives, so no two tiers can render `disabled` differently.
///
/// An HTML boolean attribute's state is its PRESENCE: `disabled=""`, `disabled="disabled"` and
/// `disabled="false"` all disable the control, and the only way to say ENABLED is to leave the
/// attribute out. So a `Bool` in one of those names decides presence and nothing is written when
/// it is false; every other value is written as it always was, which is what keeps `disabled=""`
/// (present, in HTML) and `data-done={false}` (data, so it keeps its text) both correct.
///
/// The rule is keyed on the value's TYPE, not its truthiness — an empty Text is falsy and
/// `disabled=""` is disabled, so a truthiness rule would drop exactly the case HTML keeps.
pub fn set_attribute_value(node: i64, name: &str, value: &RuntimeValue) {
    // `checked` is the exception, and it is the important one. It is boolean in the MARKUP, but
    // in the DOM the attribute is `defaultChecked` — the state a form RESET returns the box to —
    // while what a visitor sees is the PROPERTY. The two coincide on a freshly built element,
    // which is what makes writing the attribute a trap: it works on the first paint and stops
    // the moment a reconciler patches an existing node instead of creating a new one.
    if name.eq_ignore_ascii_case("checked") {
        // A non-Bool means the author wrote the attribute at all, and in HTML writing it —
        // `checked`, `checked=""`, even `checked="false"` — checks the box.
        set_checked(node, !matches!(value, RuntimeValue::Bool(false)));
        return;
    }
    if logicaffeine_language::lgx::html::is_boolean_attribute(name) {
        if let RuntimeValue::Bool(on) = value {
            if *on {
                set_attribute(node, name, "");
            }
            return;
        }
    }
    let text = value.to_display_string();
    // ★ A URL ATTRIBUTE IS A NAVIGATION, AND ESCAPING DOES NOT REACH IT. Every hole in this
    // framework is escaped, and escaping is exactly the wrong tool here: `javascript:alert(1)`
    // contains no character HTML escaping touches, so it survives every guard the page has and
    // runs the moment a visitor follows the link. The danger is the SCHEME, not the syntax.
    //
    // Any page that renders a URL it did not author is in this position — a profile link, a
    // `?next=` parameter, a link inside user content. The value is replaced rather than dropped:
    // a missing `href` looks like a styling bug and sends the reader to the wrong question, while
    // `about:blank#blocked` is inert, greppable, and obviously deliberate when someone inspects it.
    if logicaffeine_language::lgx::html::is_url_attribute(name)
        && logicaffeine_language::lgx::html::is_executable_url(&text)
    {
        set_attribute(node, name, "about:blank#blocked");
        return;
    }
    set_attribute(node, name, &text);
}
pub fn append_child(parent: i64, child: i64) {
    DOM.with(|d| d.borrow_mut().append_child(parent as u32, child as u32));
}
pub fn set_value(node: i64, value: &str) {
    DOM.with(|d| d.borrow_mut().set_value(node as u32, value));
}
/// A control whose CHECKED state a click moves. See `DomRecorder::toggle_control`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToggleControl {
    /// A click FLIPS it.
    Checkbox,
    /// A click SELECTS it; clicking a selected radio leaves it selected.
    Radio,
}

/// Whether `node` carries attribute `name` at all — the PRESENCE question an HTML boolean
/// attribute is asked with (`"#b" is disabled`). Comparing its value against a string would call
/// an enabled control disabled, since `disabled=""` is the enabled-looking spelling of disabled.
pub fn has_attribute(node: i64, name: &str) -> bool {
    DOM.with(|d| {
        d.borrow()
            .get(node as u32)
            .is_some_and(|n| n.attrs.contains_key(name))
    })
}
/// An element's tag, lowercased — what the browser's DEFAULT ACTION for a click turns on. A click
/// on an `<a href>` follows the link and a submit on a `<form>` posts it, whether or not the page
/// is listening; a runtime that only fires listeners answers neither.
pub fn tag_of(node: i64) -> Option<String> {
    DOM.with(|d| {
        d.borrow()
            .get(node as u32)
            .map(|n| n.tag.to_ascii_lowercase())
    })
}

/// One of `node`'s attribute values.
pub fn attribute(node: i64, name: &str) -> Option<String> {
    DOM.with(|d| {
        d.borrow()
            .get(node as u32)
            .and_then(|n| n.attrs.get(name).cloned())
    })
}

/// The `(name, value)` of every named field inside `node` — a form's payload, in document order.
///
/// A field with no `name` is not submitted; that is HTML's own rule, and it is how a form marks a
/// control as decoration (a search box's clear button) rather than data.
pub fn form_fields(node: i64) -> Vec<(String, String)> {
    DOM.with(|d| {
        let dom = d.borrow();
        let mut out = Vec::new();
        let mut stack = vec![node as u32];
        while let Some(id) = stack.pop() {
            let Some(n) = dom.get(id) else { continue };
            if let Some(name) = n.attrs.get("name") {
                if !name.is_empty() {
                    let value = if matches!(n.tag.to_ascii_lowercase().as_str(), "input")
                        && matches!(
                            n.attrs
                                .get("type")
                                .map(|t| t.to_ascii_lowercase())
                                .as_deref(),
                            Some("checkbox") | Some("radio")
                        ) {
                        // An unchecked box submits NOTHING — a browser omits it entirely rather
                        // than sending an empty value, and a server reading "present or absent"
                        // would see every box as ticked.
                        if !dom.get_checked(id) {
                            for c in n.children.iter().rev() {
                                stack.push(*c);
                            }
                            continue;
                        }
                        n.attrs
                            .get("value")
                            .cloned()
                            .unwrap_or_else(|| "on".to_string())
                    } else {
                        dom.control_value(id)
                    };
                    out.push((name.clone(), value));
                }
            }
            for c in n.children.iter().rev() {
                stack.push(*c);
            }
        }
        out
    })
}

pub fn get_checked(node: i64) -> bool {
    DOM.with(|d| d.borrow().get_checked(node as u32))
}
pub fn toggle_control(node: i64) -> Option<ToggleControl> {
    DOM.with(|d| d.borrow().toggle_control(node as u32))
}
pub fn set_checked(node: i64, checked: bool) {
    DOM.with(|d| d.borrow_mut().set_checked(node as u32, checked));
}
/// Record that a listener on `node` for `event` cancels the browser's default action.
///
/// A declaration about the REGISTRATION, so it is recorded once when the listener is attached —
/// a host reads it and calls `preventDefault` when the event fires, and an in-process host reads
/// it to decide whether to perform the default action itself. Every tier therefore says the same
/// thing about the same page, which is what the op-log is for.
pub fn prevent_default(node: i64, event: &str) {
    DOM.with(|d| {
        let mut d = d.borrow_mut();
        let flags = d
            .registrations
            .entry((node as u32, event.to_string()))
            .or_insert(0);
        *flags |= LISTEN_PREVENT;
        d.log.push(DomOp::PreventDefault {
            node: node as u32,
            event: event.to_string(),
        })
    });
}
pub fn set_stream(node: i64, stream: i64) {
    DOM.with(|d| d.borrow_mut().set_stream(node as u32, stream));
}
pub fn rtc_peer_connection() -> i64 {
    DOM.with(|d| d.borrow_mut().rtc_peer_connection() as i64)
}
pub fn rtc_data_channel(pc: i64, label: &str) -> i64 {
    DOM.with(|d| d.borrow_mut().rtc_data_channel(pc as u32, label) as i64)
}
pub fn rtc_create_offer(pc: i64) -> String {
    DOM.with(|d| d.borrow_mut().rtc_create_offer(pc as u32));
    "offer".to_string()
}
pub fn rtc_create_answer(pc: i64) -> String {
    DOM.with(|d| d.borrow_mut().rtc_create_answer(pc as u32));
    "answer".to_string()
}
pub fn rtc_set_local(pc: i64, sdp: &str) {
    DOM.with(|d| d.borrow_mut().rtc_set_local(pc as u32, sdp));
}
pub fn rtc_set_remote(pc: i64, sdp: &str) {
    DOM.with(|d| d.borrow_mut().rtc_set_remote(pc as u32, sdp));
}
pub fn rtc_add_ice(pc: i64, candidate: &str) {
    DOM.with(|d| d.borrow_mut().rtc_add_ice(pc as u32, candidate));
}
pub fn rtc_send(channel: i64, bytes: u64) {
    DOM.with(|d| d.borrow_mut().rtc_send(channel as u32, bytes));
}
pub fn get_text(node: i64) -> String {
    DOM.with(|d| d.borrow().get_text(node as u32))
}
pub fn get_value(node: i64) -> String {
    DOM.with(|d| d.borrow().get_value(node as u32))
}
/// Whether a visitor would SEE the first element matching `selector` — see `Dom::shown`.
pub fn is_shown(selector: &str) -> bool {
    DOM.with(|d| {
        let d = d.borrow();
        match d.query_selector_all(selector).first() {
            Some(&h) => d.shown(h),
            None => false,
        }
    })
}

pub fn query_selector(selector: &str) -> i64 {
    DOM.with(|d| handle(d.borrow().query_selector(selector)))
}
/// A node id as the value model carries it: the id itself, or `-1` for the `u32::MAX` "no such
/// node" — never a wrap through `i32`, which would turn every stream id negative.
fn handle(node: u32) -> i64 {
    if node == u32::MAX {
        -1
    } else {
        node as i64
    }
}
/// Every node matching `selector`, in document order.
pub fn query_selector_all(selector: &str) -> Vec<i64> {
    DOM.with(|d| {
        d.borrow()
            .query_selector_all(selector)
            .into_iter()
            .map(i64::from)
            .collect()
    })
}
/// The element's text content: its own text plus every descendant's, in document order.
pub fn text_content(node: i64) -> String {
    DOM.with(|d| d.borrow().text_content(node as u32))
}
/// `Set the title to …` — `document.title = …`: the head's first `<title>` holds the new text, or,
/// when the head has none, a new `<title>` holding it joins the head.
pub fn set_title(title: &str) {
    DOM.with(|d| {
        let head = &mut d.borrow_mut().head;
        match head.iter_mut().find(|t| t.tag == "title") {
            Some(element) => element.text = title.to_string(),
            None => head.push(HeadTag {
                tag: "title".to_string(),
                text: title.to_string(),
                ..HeadTag::default()
            }),
        }
    });
}
/// `the title` — `document.title`: the text of the head's first `<title>`, its ASCII whitespace
/// stripped from both ends and collapsed to single spaces; `""` when there is none.
pub fn title() -> String {
    DOM.with(|d| {
        d.borrow()
            .head
            .iter()
            .find(|t| t.tag == "title")
            .map(|t| {
                t.text
                    .split_ascii_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default()
    })
}
/// `Focus SEL.` — the element the page's keyboard input goes to.
/// `Copy "…" to the clipboard.` — the text a browser would have put on the clipboard.
pub fn write_clipboard(text: &str) {
    DOM.with(|d| d.borrow_mut().clipboard = text.to_string());
}

/// What is on the clipboard (`""` when the page has never written to it).
pub fn clipboard() -> String {
    DOM.with(|d| d.borrow().clipboard.clone())
}

/// `Download <content> as "<name>".` — a file the page offered, `(name, content)`.
pub fn offer_download(name: &str, content: &str) {
    DOM.with(|d| {
        d.borrow_mut()
            .downloads
            .push((name.to_string(), content.to_string()))
    });
}

/// Every download the page has offered, in order.
pub fn downloads() -> Vec<(String, String)> {
    DOM.with(|d| d.borrow().downloads.clone())
}

/// `Notify "…".` — a notification the page raised, `(message, tone)`.
pub fn notify(message: &str, tone: &str) {
    DOM.with(|d| {
        d.borrow_mut()
            .notifications
            .push((message.to_string(), tone.to_string()))
    });
}

/// Every notification the page has raised, in order.
pub fn notifications() -> Vec<(String, String)> {
    DOM.with(|d| d.borrow().notifications.clone())
}

/// `Publish M on "t".` — the frame the page handed to the transport, `(topic, message)`.
///
/// ★ RECORDED ON BOTH TIERS, which is the only way the OUTBOUND half can be checked at all.
/// A publish's local delivery shows up in the page's own HTML, so parity would look complete
/// while the leg that actually leaves the page went unsent on one tier — the same
/// half-a-construct shape this verb exists to close. In a browser this is the WebSocket frame;
/// here it is the record that one was sent.
pub fn publish(topic: &str, message: &str) {
    DOM.with(|d| {
        d.borrow_mut()
            .published
            .push((topic.to_string(), message.to_string()))
    });
}

/// Every frame the page has published, in order.
pub fn published() -> Vec<(String, String)> {
    DOM.with(|d| d.borrow().published.clone())
}

/// `Scroll to "#card".` — the element a browser would have brought into view.
pub fn scroll_to(node: i64) {
    DOM.with(|d| d.borrow_mut().scrolled_to = node as u32);
}

/// The element most recently scrolled to, or `-1`.
pub fn scrolled_to() -> i64 {
    DOM.with(|d| handle(d.borrow().scrolled_to))
}

/// `Select the text in "#draft".` — the element whose text a browser would have selected.
pub fn select_text(node: i64) {
    DOM.with(|d| d.borrow_mut().selected = node as u32);
}

/// The element whose text was most recently selected, or `-1`.
pub fn selected() -> i64 {
    DOM.with(|d| handle(d.borrow().selected))
}

pub fn set_focus(node: i64) {
    DOM.with(|d| {
        let mut d = d.borrow_mut();
        d.focused = node as u32;
    });
}
/// The focused element, or a negative handle when nothing is focused.
pub fn focused() -> i64 {
    DOM.with(|d| {
        let mut d = d.borrow_mut();
        d.drop_detached_focus();
        handle(d.focused)
    })
}

/// Whether `node` can take keyboard focus, as a browser decides it: a control that is not
/// `disabled` (`button`, `select`, `textarea`, an `input` other than `type="hidden"`), a link or
/// image-map `area` with an `href`, an `iframe`, a `summary`, `audio`/`video` showing `controls`,
/// and any element carrying a `tabindex` or an editable `contenteditable`.
pub fn focusable(node: i64) -> bool {
    DOM.with(|d| d.borrow().get(node as u32).is_some_and(Node::focusable))
}

/// Where the default action of `verb` on `node` sends keyboard focus
/// ([`crate::semantics::drive::Verb::focus_target`]): `Some(element)`, `Some(-1)` for off every
/// element, `None` when focus does not move.
pub fn focus_target(verb: crate::semantics::drive::Verb, node: i64) -> Option<i64> {
    use crate::semantics::drive::FocusTarget;
    match verb.focus_target() {
        FocusTarget::Unmoved => None,
        FocusTarget::Element => focusable(node).then_some(node),
        FocusTarget::PressedAncestor => DOM.with(|d| {
            let d = d.borrow();
            let chain = d.ancestors_of(node as u32).unwrap_or_default();
            let pressed = std::iter::once(node as u32).chain(chain.into_iter().rev());
            Some(
                pressed
                    .filter(|&h| h != 0)
                    .find(|&h| d.get(h).is_some_and(Node::focusable))
                    .map_or(-1, i64::from),
            )
        }),
    }
}

/// Move keyboard focus to `target` (`-1`: off every element), as a browser's focusing steps do:
/// `(blurred, focused)` — the element that lost focus and the one that gained it, `-1` for none —
/// for the caller to raise `blur` and then `focus` on; `None` when focus is already there.
pub fn move_focus(target: i64) -> Option<(i64, i64)> {
    let before = focused();
    let target = if target < 0 { -1 } else { target };
    if before == target {
        return None;
    }
    set_focus(target);
    Some((before, target))
}

/// The default focus action of `verb` on `node`, carried out: [`focus_target`], then
/// [`move_focus`]. What a driving verb does before it raises its own events.
pub fn focus_for(verb: crate::semantics::drive::Verb, node: i64) -> Option<(i64, i64)> {
    focus_target(verb, node).and_then(move_focus)
}

/// `element.focus()`: a focusable element that does not have focus takes it, the element that had
/// it losing it first — and the `blur` and `focus` that raises wait in [`take_raised_events`] for
/// the host to deliver once the turn that asked has committed, as a browser delivers them.
/// Anything else is left alone: an element that cannot take focus, or already has it.
pub fn element_focus(node: i64) {
    if !focusable(node) {
        return;
    }
    if let Some((blurred, gained)) = move_focus(node) {
        raise_focus_events(blurred, gained);
    }
}

/// `element.blur()`: the focused element loses focus to nothing and `blur` is raised on it
/// ([`take_raised_events`]); an element that does not have focus is left alone.
pub fn element_blur(node: i64) {
    if node >= 0 && focused() == node {
        if let Some((blurred, gained)) = move_focus(-1) {
            raise_focus_events(blurred, gained);
        }
    }
}

fn raise_focus_events(blurred: i64, gained: i64) {
    let events: Vec<(String, &'static str)> = [(blurred, "blur"), (gained, "focus")]
        .into_iter()
        .filter(|(element, _)| *element >= 0)
        .map(|(element, event)| (node_address(element), event))
        .collect();
    DOM.with(|d| d.borrow_mut().raised.extend(events));
}

/// The events the page's own element commands raised ([`element_focus`], [`element_blur`]) that
/// the host has not delivered yet, `(element, event)` in the order they were raised — taken, so
/// each is delivered once. Each element is found as the page stands NOW (it was recorded by its
/// [`node_address`], which survives the turn's repaint), and an event whose element has left the
/// page is dropped: nothing is listening there any more.
pub fn take_raised_events() -> Vec<(i64, &'static str)> {
    let raised = DOM.with(|d| std::mem::take(&mut d.borrow_mut().raised));
    raised
        .into_iter()
        .filter_map(|(address, event)| {
            let element = query_selector(&address);
            (element >= 0).then_some((element, event))
        })
        .collect()
}

/// A selector naming `node` by the identity a browser's reconciler keeps it by through a repaint:
/// `#id` when the element carries an id that names it, `body` for the page's body, else its PLACE —
/// its child index under each ancestor from `body` down, and its tag — which [`query_selector`]
/// resolves to whatever element of that tag stands there now. The place form is not one any page
/// can spell: it begins with a character no selector contains, and is resolved before the selector
/// grammar is consulted. Empty for a node that is not an attached element.
pub fn node_address(node: i64) -> String {
    DOM.with(|d| d.borrow().address(node as u32))
}

/// The mark a place-form [`node_address`] begins with — a character no selector a page writes
/// contains.
const PLACE: char = '\u{0}';

/// The value a visitor's `Type`/`Choose` leaves in the control `selector` names before the verb
/// raises its events — the browser's own default action of typing (the text replaces what the
/// field held) and of choosing (the option whose value, or else whose text, is `value` becomes the
/// select's value). Recorder state rather than an op: it is the visitor's act on the document, not
/// something the page asked for. Other verbs, a control the verb cannot act on
/// ([`answers_by_itself`]), and a choice no option offers, leave it alone.
pub fn enter_value(verb: crate::semantics::drive::Verb, selector: &str, value: &str) {
    use crate::semantics::drive::Verb;
    DOM.with(|d| {
        let mut d = d.borrow_mut();
        let node = d.query_selector(selector);
        if !d.takes(verb, node, value) {
            return;
        }
        match verb {
            Verb::Type => {
                let Some(n) = d.get_mut(node) else { return };
                let (held, bad_input) = typed_value(n, value);
                n.value = held;
                n.bad_input = bad_input;
                n.user_edited = true;
            }
            Verb::Choose => {
                if let Some(chosen) = d.option_offering(node, value) {
                    if let Some(n) = d.get_mut(node) {
                        n.value = chosen;
                        n.user_edited = true;
                    }
                }
            }
            _ => {}
        }
    });
}

/// Whether the browser's own default action of `verb` on the element `selector` names changes it —
/// typing enters text into a control that takes typed text, choosing selects an option the
/// `<select>` offers — so the verb is answered whether or not the page listens: the value it
/// entered is on the page, and the control's validity with it. A click, a press, a submit and a
/// hover answer only through a listener (a checkbox's click is answered by its toggle, where the
/// click is decided).
pub fn answers_by_itself(verb: crate::semantics::drive::Verb, selector: &str, value: &str) -> bool {
    DOM.with(|d| {
        let d = d.borrow();
        d.takes(verb, d.query_selector(selector), value)
    })
}

/// The `<input>` types a visitor types text into.
const TEXT_ENTRY_TYPES: [&str; 8] = [
    "", "text", "search", "url", "tel", "email", "password", "number",
];

impl DomRecorder {
    /// Whether `verb`'s default action applies to `node`: a `<textarea>` or a text-entry `<input>`
    /// takes typing, a `<select>` takes the choice of an option it offers — each only when the
    /// visitor can reach it (not disabled, read-only or hidden: [`DomRecorder::barred`]).
    fn takes(&self, verb: crate::semantics::drive::Verb, node: u32, value: &str) -> bool {
        use crate::semantics::drive::Verb;
        let Some(n) = self.get(node) else {
            return false;
        };
        let reachable = !self.barred(node);
        match verb {
            Verb::Type => {
                reachable
                    && (n.tag.eq_ignore_ascii_case("textarea")
                        || (n.tag.eq_ignore_ascii_case("input")
                            && TEXT_ENTRY_TYPES.contains(&input_type(n).as_str())))
            }
            Verb::Choose => reachable && self.option_offering(node, value).is_some(),
            Verb::Click | Verb::Submit | Verb::Press | Verb::Hover => false,
        }
    }
}

/// What a control holds after a visitor types `text` into it, as a browser sanitizes typing: a
/// `number` field takes only the characters a number is written with; a `maxlength` cuts the text
/// off (in UTF-16 units); a single-line field drops line breaks; an `email` or `url` field drops
/// the whitespace around it (around each address, for `multiple`); and a `number` field holds only
/// a number — typed characters that make none leave it empty, with the bad input flagged.
fn typed_value(n: &Node, text: &str) -> (String, bool) {
    let tag = n.tag.to_ascii_lowercase();
    let ty = input_type(n);
    let mut held: String = match (tag.as_str(), ty.as_str()) {
        ("textarea", _) => text.to_string(),
        (_, "number") => text
            .chars()
            .filter(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E'))
            .collect(),
        _ => text.chars().filter(|c| *c != '\n' && *c != '\r').collect(),
    };
    if let Some(max) = n
        .attrs
        .get("maxlength")
        .and_then(|m| m.trim().parse::<usize>().ok())
    {
        let mut units = 0;
        held = held
            .chars()
            .take_while(|c| {
                units += c.len_utf16();
                units <= max
            })
            .collect();
    }
    let trim = |s: &str| {
        s.trim_matches(|c: char| c.is_ascii_whitespace())
            .to_string()
    };
    match ty.as_str() {
        "email" if n.attrs.contains_key("multiple") => (
            held.split(',').map(trim).collect::<Vec<_>>().join(","),
            false,
        ),
        "email" | "url" => (trim(&held), false),
        "number" if valid_float(&held) => (held, false),
        "number" => (String::new(), !held.is_empty()),
        _ => (held, false),
    }
}

/// An `<input>`'s type, lowercased (`text` when it names none); empty for any other element.
fn input_type(n: &Node) -> String {
    match n.tag.eq_ignore_ascii_case("input") {
        true => n
            .attrs
            .get("type")
            .map(|t| t.trim().to_ascii_lowercase())
            .unwrap_or_else(|| "text".to_string()),
        false => String::new(),
    }
}

/// The default focus action of `verb` on the element `selector` names, carried out, as the events
/// it raises in the order a browser raises them: `blur` on the element that lost focus, then
/// `focus` on the one that gained it — each `(element, event)`, before the verb's own events.
/// Nothing when the selector names no element (the verb reports that itself) or focus stays put.
pub fn focus_by(verb: crate::semantics::drive::Verb, selector: &str) -> Vec<(i64, &'static str)> {
    let node = query_selector(selector);
    if node < 0 {
        return Vec::new();
    }
    match focus_for(verb, node) {
        Some((blurred, gained)) => [(blurred, "blur"), (gained, "focus")]
            .into_iter()
            .filter(|(element, _)| *element >= 0)
            .collect(),
        None => Vec::new(),
    }
}
/// Where an event dispatched at one element goes: the listeners it reaches, in order, and whether one
/// of them cancels its default action — [`dispatch_path`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DispatchPath {
    /// Each node whose listener the event reaches, in the order it reaches them, named by
    /// [`node_address`] — so the host delivering to each finds it as the page stands when its turn
    /// comes, after the turns before it have repainted.
    pub stops: Vec<String>,
    /// Whether a listener the walk reaches cancels the event's default action: one registered with
    /// `.prevent` and not `.passive`.
    pub prevented: bool,
}

/// The walk `event` dispatched at `target` takes, as a browser dispatches it: the capture-phase
/// listeners of the target's ancestors from the root down, the target itself, then — when the event
/// bubbles ([`event_bubbles`]) — the other ancestors' listeners from the target back up. A listener
/// registered `.stop` ends the walk after it. The target is always on the walk: whether anything
/// there answers is the page's own dispatcher's to say.
pub fn dispatch_path(target: i64, event: &str) -> DispatchPath {
    DOM.with(|d| d.borrow().dispatch_path(target as u32, event))
}

/// The events a browser dispatches WITHOUT a bubbling phase — the ones a listener on an ancestor
/// hears only by capturing. Every other event bubbles.
pub const NON_BUBBLING_EVENTS: &[&str] = &[
    "focus",
    "blur",
    "mouseenter",
    "mouseleave",
    "pointerenter",
    "pointerleave",
    "load",
    "unload",
    "error",
    "abort",
    "scroll",
    "scrollend",
    "resize",
    "invalid",
    "toggle",
];

/// Whether `event` bubbles in a browser — see [`NON_BUBBLING_EVENTS`].
pub fn event_bubbles(event: &str) -> bool {
    !NON_BUBBLING_EVENTS.contains(&event)
}

/// A link a click lands on: the nearest `<a href>` that is the clicked element or holds it, as a
/// browser's activation behaviour finds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub href: String,
    /// Whether following it moves THIS document: no `download`, and a `target` of this browsing
    /// context (none, `_self`, `_top`, `_parent`). A download, or a link to another window, leaves
    /// the page where it is.
    pub here: bool,
}

/// The link a click on `node` lands on ([`Link`]), if any.
pub fn link_of(node: i64) -> Option<Link> {
    DOM.with(|d| {
        let d = d.borrow();
        let chain = d.ancestors_of(node as u32)?;
        let a = std::iter::once(node as u32)
            .chain(chain.into_iter().rev())
            .filter_map(|h| d.get(h))
            .find(|n| {
                !n.is_text && n.tag.eq_ignore_ascii_case("a") && n.attrs.contains_key("href")
            })?;
        let target = a
            .attrs
            .get("target")
            .map(|t| t.trim().to_ascii_lowercase())
            .unwrap_or_default();
        Some(Link {
            href: a.attrs.get("href").cloned().unwrap_or_default(),
            here: !a.attrs.contains_key("download")
                && matches!(target.as_str(), "" | "_self" | "_top" | "_parent"),
        })
    })
}

/// Add a class to an element's class list, leaving the others alone.
pub fn add_class(node: i64, class: &str) {
    let current = DOM.with(|d| {
        d.borrow()
            .get(node as u32)
            .and_then(|n| n.attrs.get("class").cloned())
            .unwrap_or_default()
    });
    if current.split_whitespace().any(|c| c == class) {
        return;
    }
    let next = if current.is_empty() {
        class.to_string()
    } else {
        format!("{current} {class}")
    };
    set_attribute(node, "class", &next);
}
/// Remove a class from an element's class list, leaving the others alone.
pub fn remove_class(node: i64, class: &str) {
    let current = DOM.with(|d| {
        d.borrow()
            .get(node as u32)
            .and_then(|n| n.attrs.get("class").cloned())
            .unwrap_or_default()
    });
    let next: Vec<&str> = current.split_whitespace().filter(|c| *c != class).collect();
    set_attribute(node, "class", &next.join(" "));
}
pub fn add_event_listener(node: i64, event: &str, closure: RuntimeValue) {
    DOM.with(|d| {
        d.borrow_mut()
            .add_event_listener(node as u32, event, closure)
    });
}

/// Record that a listener was attached, without handing the recorder a closure to fire.
///
/// A replayed stream names a listener whose handler is a closure in the program that emitted it,
/// so the listener is not the recorder's to call. The BROWSER still has to be told to listen: a
/// page reaches one as a stream of DOM calls, and a registration missing from that stream is a
/// registration the browser never hears about. The op-log is the contract between the tiers, so
/// it carries the attachment even when this recorder is not the thing that will fire it.
pub fn note_event_listener(node: i64, event: &str) {
    DOM.with(|d| {
        let mut d = d.borrow_mut();
        d.register(node as u32, event);
        d.log.push(DomOp::AddEventListener {
            node: node as u32,
            event: event.to_string(),
        })
    });
}
/// `eventKey() -> Text` — the key of the keyboard event being dispatched, `""` otherwise.
///
/// ★ THE ONE THING A KEY MODIFIER NEEDS. `on-keydown.enter` gates on which key was pressed, and
/// no accessor reported one — `eventValue` is the input's VALUE, which for a keydown is the text
/// so far, not the keystroke. So the browser tier registered a listener with no key comparison
/// while the reference tier gated correctly, and the modifier was refused rather than guessed.
pub fn event_key() -> String {
    DOM.with(|d| {
        d.borrow()
            .current_event
            .as_ref()
            .map(|e| e.key.clone())
            .unwrap_or_default()
    })
}
pub fn event_value() -> String {
    DOM.with(|d| {
        d.borrow()
            .current_event
            .as_ref()
            .map(|e| e.value.clone())
            .unwrap_or_default()
    })
}
pub fn event_target() -> i64 {
    DOM.with(|d| {
        d.borrow()
            .current_event
            .as_ref()
            .map(|e| e.target as i64)
            .unwrap_or(-1)
    })
}

/// Simulate an event dispatch on the NATIVE tiers: look up the `(node, event)`
/// handler, set the current event (its target + value), and hand the closure back
/// to the caller to invoke (the interpreter owns closure invocation). Returns the
/// closure, or `None` if no handler is registered.
pub fn begin_dispatch(node: i64, event: &str, value: &str) -> Option<RuntimeValue> {
    begin_dispatch_keyed(node, event, value, "")
}

/// [`begin_dispatch`], naming the key for a keyboard event so `eventKey` can report it.
pub fn begin_dispatch_keyed(
    node: i64,
    event: &str,
    value: &str,
    key: &str,
) -> Option<RuntimeValue> {
    DOM.with(|d| {
        let mut dom = d.borrow_mut();
        let closure = dom.listener_for(node as u32, event)?;
        dom.current_event = Some(CurrentEvent {
            target: node as u32,
            value: value.to_string(),
            key: key.to_string(),
        });
        Some(closure)
    })
}
pub fn end_dispatch() {
    DOM.with(|d| d.borrow_mut().current_event = None);
}

// --- The correlated fetch contract. `fetch_send` mints the request id every other
// call is keyed by; `fetchText` is sugar over the same engine (GET, no body, no
// headers), so the two surfaces can never behave differently. ---

/// `fetchSend(method, url, body, headers, onDone) -> Int` — register a request,
/// returning its correlation id. Contract violations (oversized body, disallowed
/// header) fail the request immediately with a structured error.
pub fn fetch_send(
    method: &str,
    url: &str,
    body: &str,
    headers: Vec<(String, String)>,
    closure: RuntimeValue,
) -> i64 {
    DOM.with(|d| {
        d.borrow_mut()
            .fetch_send(method, url, body, headers, closure)
    })
}

/// The `fetchText(url, onDone)` sugar — a GET through the correlated engine.
pub fn fetch_start(url: &str, closure: RuntimeValue) {
    DOM.with(|d| {
        d.borrow_mut()
            .fetch_send("GET", url, "", Vec::new(), closure);
    });
}

/// `fetchAbort(reqId)` — the request becomes FINAL with the `"abort"` error;
/// a late response can never deliver.
pub fn fetch_abort(req: i64) {
    DOM.with(|d| d.borrow_mut().fetch_abort(req));
}

/// `fetchSetTimeout(reqId, millis)` — arm (`> 0`) or clear (`0`) the timeout.
pub fn fetch_set_timeout(req: i64, millis: i64) {
    DOM.with(|d| d.borrow_mut().fetch_set_timeout(req, millis));
}

/// `responseStatus(reqId) -> Int` — the delivered HTTP status; `0` while pending
/// or on any transport failure (`responseError` then names the failure).
pub fn response_status(req: i64) -> i64 {
    DOM.with(|d| {
        d.borrow()
            .fetch_response(req)
            .map(|r| r.status)
            .unwrap_or(0)
    })
}

/// `responseBody(reqId) -> Text` — the delivered body; `""` while pending or failed.
pub fn response_body(req: i64) -> String {
    DOM.with(|d| {
        d.borrow()
            .fetch_response(req)
            .map(|r| r.body.clone())
            .unwrap_or_default()
    })
}

/// `responseHeader(reqId, name) -> Text` — a delivered response header,
/// case-insensitively; `""` when absent.
pub fn response_header(req: i64, name: &str) -> String {
    DOM.with(|d| {
        d.borrow()
            .fetch_response(req)
            .and_then(|r| {
                r.headers
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(name))
                    .map(|(_, v)| v.clone())
            })
            .unwrap_or_default()
    })
}

/// `responseError(reqId) -> Text` — `""` when none (pending or delivered OK);
/// otherwise `"timeout"` | `"abort"` | `"toolarge"` | `"network: <detail>"`.
pub fn response_error(req: i64) -> String {
    DOM.with(|d| {
        d.borrow()
            .fetch_response(req)
            .map(|r| r.error.clone())
            .unwrap_or_default()
    })
}

/// The `responseText()` sugar — the body of the request CURRENTLY being delivered
/// (set by [`begin_fetch_resolve`]/[`fail_fetch`], cleared by [`end_fetch_resolve`]).
pub fn response_text() -> String {
    DOM.with(|d| {
        let dom = d.borrow();
        dom.current_fetch
            .and_then(|req| dom.fetch_response(req).map(|r| r.body.clone()))
            .unwrap_or_default()
    })
}

/// Every request the run registered, in id order — the tw≡VM parity surface for
/// the REQUEST side (method, url, body, headers, timeout, abort state).
/// The content type an answer a test arranged ([`stub_fetch`]) is served with, on every tier.
pub const STUB_CONTENT_TYPE: &str = "application/json";

/// Arrange the answer to a request the page is expected to send. A later stub for the same
/// (method, url) REPLACES an earlier one, so a test can set the scene and then change it.
pub fn stub_fetch(method: &str, url: &str, status: i64, body: &str) {
    DOM.with(|d| {
        let mut d = d.borrow_mut();
        let key = (method.to_ascii_uppercase(), url.to_string());
        let value = (status, body.to_string());
        match d.fetch_stubs.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => d.fetch_stubs.push((key, value)),
        }
    });
}

/// The answer arranged for `req`, if a test arranged one.
pub fn stubbed_answer(req: i64) -> Option<(i64, String)> {
    DOM.with(|d| {
        let d = d.borrow();
        let r = d.fetch_requests().into_iter().find(|r| r.req == req)?;
        let key = (r.method.to_ascii_uppercase(), r.url);
        d.fetch_stubs
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.clone())
    })
}

pub fn fetch_requests() -> Vec<FetchRequest> {
    DOM.with(|d| d.borrow().fetch_requests())
}

/// Deliver request `req` with `status`/`headers`/`body`: record its per-request
/// response, point the current-delivery cursor at it, and hand back its completion
/// closure for the caller to invoke. `None` if the request is unknown or already
/// FINAL (aborted/failed/delivered — the anti-double-delivery rule).
pub fn begin_fetch_resolve(
    req: i64,
    status: i64,
    headers: &[(String, String)],
    body: &str,
) -> Option<RuntimeValue> {
    DOM.with(|d| d.borrow_mut().fetch_resolve(req, status, headers, body))
}

/// Deliver a structured transport failure for `req` (same finality rule).
pub fn fail_fetch(req: i64, error: &str) -> Option<RuntimeValue> {
    DOM.with(|d| d.borrow_mut().fetch_fail(req, error))
}

pub fn end_fetch_resolve() {
    DOM.with(|d| d.borrow_mut().current_fetch = None);
}

// --- The router contract. `route_path`/`route_query` read the location;
// `navigate`/`replace_route` record a navigation and move the current path;
// `on_route` registers a popstate handler closure. `init_route` seeds the initial
// location (a refresh / deep-link boots AT that path), and `deliver_pop` hands every
// registered handler over with the new path — the deterministic back/forward driver,
// mirroring `begin_fetch_resolve` (returns the closures for the caller to invoke). ---

/// `routePath() -> Text` — the current location pathname (`"/"` at boot).
pub fn route_path() -> String {
    DOM.with(|d| d.borrow().route_path.clone())
}

/// `routeQuery(name) -> Text` — a query param by name (`""` when absent).
pub fn route_query(name: &str) -> String {
    DOM.with(|d| d.borrow().route_query_value(name))
}

/// `navigate(path)` — history.pushState: record the navigation, move the path.
pub fn navigate(path: &str) {
    DOM.with(|d| d.borrow_mut().navigate(path, false));
}

/// `replaceRoute(path)` — history.replaceState: record the navigation (replace),
/// move the path.
pub fn replace_route(path: &str) {
    DOM.with(|d| d.borrow_mut().navigate(path, true));
}

/// `onRoute(handler)` — register a popstate handler closure: a back/forward fires it, and every
/// other handler registered before it, as a browser's `popstate` reaches every listener.
pub fn on_route(handler: RuntimeValue) {
    DOM.with(|d| d.borrow_mut().route_handlers.push(handler));
}

/// The recorded navigation op-log — the tw≡VM parity surface for the router.
pub fn route_ops() -> Vec<RouteOp> {
    DOM.with(|d| d.borrow().route_ops.clone())
}

/// Seed the initial location (a refresh / deep-link: the app boots already AT this
/// path with this query string). Called by tests before a run.
pub fn init_route(path: &str, query: &str) {
    DOM.with(|d| {
        let mut dom = d.borrow_mut();
        dom.route_path = path.to_string();
        dom.route_query = query.to_string();
        // Booting AT a path means that path is where Back would stop, not "/".
        dom.route_history = vec![path.to_string()];
    });
}

/// `Go back.` — step one entry back and hand over the route handlers to fire, exactly as a real
/// popstate would. `None` when there is nowhere to go: the first page a visitor lands on has no
/// previous entry, and a browser's Back button does nothing there rather than inventing one.
pub fn go_back() -> Option<(String, Vec<RuntimeValue>)> {
    DOM.with(|d| {
        let mut dom = d.borrow_mut();
        let path = dom.go_back()?;
        Some((path, dom.route_handlers.clone()))
    })
}

/// Deliver a popstate (back/forward) at `path`: move the current path and hand back every
/// registered handler closure, in registration order, for the caller to invoke (the interpreter
/// owns closure invocation) — none when nothing is registered. Deterministic — never a real
/// browser event.
pub fn deliver_pop(path: &str) -> Vec<RuntimeValue> {
    DOM.with(|d| {
        let mut dom = d.borrow_mut();
        dom.route_path = path.to_string();
        dom.route_handlers.clone()
    })
}

// ─── Wave 7.4: the retained-tree primitives `vdom.lg` needs and `runtime.js`/`rc` did not,
// wired through `semantics::patch`'s opcodes 14-21 ────────────────────────────────────────────

/// Event-modifier bits a listener registration carries (`listenFlags`, `DomOp::Listen`) —
/// `.prevent` / `.stop` / `.once` / `.capture` / `.passive`, one bit each, in that order. Every
/// host dispatches by them as a browser does ([`dispatch_path`]), except `.once`, which the
/// lowering keeps itself: a host never retires a listener on its own.
pub const LISTEN_PREVENT: u8 = 1 << 0;
pub const LISTEN_STOP: u8 = 1 << 1;
pub const LISTEN_ONCE: u8 = 1 << 2;
pub const LISTEN_CAPTURE: u8 = 1 << 3;
pub const LISTEN_PASSIVE: u8 = 1 << 4;

/// The first id of the range a patch stream's emitter names (`vdom.lg`'s `vdWire`); every id below
/// it is a creation ordinal this recorder minted itself, so the two never collide.
pub const STREAM_ID_BASE: u32 = 1 << 31;

impl DomRecorder {
    /// The node whose `children` currently lists `node`, wherever in the tree that is. `Node`
    /// carries no parent pointer of its own — a keyed diff moves and removes nodes rarely enough,
    /// against a recorder small enough, that a search is the honest cost rather than a field kept
    /// consistent by every other mutator for one caller.
    fn parent_of(&self, node: u32) -> Option<u32> {
        let minted = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (i as u32, Some(n)));
        let streamed = self
            .streamed
            .iter()
            .enumerate()
            .map(|(i, n)| (i as u32 + STREAM_ID_BASE, n.as_ref()));
        minted
            .chain(streamed)
            .find(|(_, n)| n.is_some_and(|n| n.children.contains(&node)))
            .map(|(id, _)| id)
    }

    fn remove_node(&mut self, node: u32) {
        let node = self.resolve(node);
        self.leaving(node);
        if let Some(parent) = self.parent_of(node) {
            if let Some(p) = self.get_mut(parent) {
                p.children.retain(|&c| c != node);
            }
        }
        self.log.push(DomOp::RemoveNode { node });
    }

    fn insert_before(&mut self, parent: u32, child: u32, before: u32) {
        let (child, before) = (self.resolve(child), self.resolve(before));
        let moved = self.parent_of(child).is_some();
        let blurred = (moved && self.focused != u32::MAX && self.holds(child, self.focused))
            .then_some(self.focused);
        self.leaving(child);
        if let Some(cur) = self.parent_of(child) {
            if let Some(p) = self.get_mut(cur) {
                p.children.retain(|&c| c != child);
            }
        }
        if let Some(p) = self.get_mut(parent) {
            let at = if before == u32::MAX {
                p.children.len()
            } else {
                p.children
                    .iter()
                    .position(|&c| c == before)
                    .unwrap_or(p.children.len())
            };
            p.children.insert(at, child);
        }
        // A moved element keeps its listeners, so the `blur` its move raised reaches them once the
        // turn has committed, as a browser delivers it.
        if let Some(element) = blurred {
            let at = self.address(element);
            self.raised.push((at, "blur"));
        }
        self.log.push(DomOp::InsertBefore {
            parent,
            child,
            before,
        });
    }

    fn remove_attribute(&mut self, node: u32, name: &str) {
        if let Some(n) = self.get_mut(node) {
            n.attrs.remove(name);
        }
        self.log.push(DomOp::RemoveAttribute {
            node,
            name: name.to_string(),
        });
    }

    /// Release `node`'s slot AND every descendant's: detach `node` from its parent (if it still
    /// has one), then walk the whole subtree clearing each node's content and listeners, so
    /// `render()` and every page query see nothing of any of it — a freed id is exactly as gone
    /// as one that never existed. ONE op frees an entire subtree regardless of its size; a
    /// descendant is never freed by its own `FreeNode` — only ever as part of an ancestor's.
    fn free_node(&mut self, node: u32) {
        let node = self.resolve(node);
        self.leaving(node);
        if let Some(parent) = self.parent_of(node) {
            if let Some(p) = self.get_mut(parent) {
                p.children.retain(|&c| c != node);
            }
        }
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            if let Some(existing) = self.get(n) {
                stack.extend(existing.children.iter().copied());
            }
            self.listeners.retain(|(ln, _, _)| *ln != n);
            self.registrations.retain(|(rn, _), _| *rn != n);
            match n.checked_sub(STREAM_ID_BASE) {
                Some(i) => {
                    if let Some(slot) = self.streamed.get_mut(i as usize) {
                        *slot = None;
                    }
                }
                None => {
                    if let Some(nn) = self.get_mut(n) {
                        *nn = Node::default();
                    }
                }
            }
        }
        self.log.push(DomOp::FreeNode { node });
    }

    fn listen(&mut self, node: u32, event: &str, flags: u8) {
        let node = self.resolve(node);
        self.registrations.insert((node, event.to_string()), flags);
        self.log.push(DomOp::Listen {
            node,
            event: event.to_string(),
            flags,
        });
    }

    fn unlisten(&mut self, node: u32, event: &str) {
        let node = self.resolve(node);
        self.registrations.remove(&(node, event.to_string()));
        self.log.push(DomOp::Unlisten {
            node,
            event: event.to_string(),
        });
    }

    /// A listener for `event` on `node`, its flags kept when it is already registered.
    fn register(&mut self, node: u32, event: &str) {
        let node = self.resolve(node);
        self.registrations
            .entry((node, event.to_string()))
            .or_insert(0);
    }

    /// The flags of the listener for `event` on `node`, if one is registered.
    fn registration(&self, node: u32, event: &str) -> Option<u8> {
        self.registrations.get(&(node, event.to_string())).copied()
    }

    /// See [`dispatch_path`].
    fn dispatch_path(&self, target: u32, event: &str) -> DispatchPath {
        let ancestors = self.ancestors_of(target).unwrap_or_default();
        let mut stops = vec![];
        let mut prevented = false;
        let mut reach = |node: u32, flags: u8| {
            stops.push(node);
            prevented |= flags & LISTEN_PREVENT != 0 && flags & LISTEN_PASSIVE == 0;
            flags & LISTEN_STOP != 0
        };
        let capturing = ancestors.iter().filter_map(|&a| {
            self.registration(a, event)
                .filter(|f| f & LISTEN_CAPTURE != 0)
                .map(|f| (a, f))
        });
        let bubbling = ancestors.iter().rev().filter_map(|&a| {
            self.registration(a, event)
                .filter(|f| f & LISTEN_CAPTURE == 0)
                .map(|f| (a, f))
        });
        let at_target = std::iter::once((target, self.registration(target, event).unwrap_or(0)));
        let walk: Vec<(u32, u8)> = match event_bubbles(event) {
            true => capturing.chain(at_target).chain(bubbling).collect(),
            false => capturing.chain(at_target).collect(),
        };
        for (node, flags) in walk {
            if reach(node, flags) {
                break;
            }
        }
        let addresses = stops.into_iter().map(|n| self.address(n)).collect();
        DispatchPath {
            stops: addresses,
            prevented,
        }
    }

    /// See [`node_address`].
    fn address(&self, node: u32) -> String {
        let id = self
            .get(node)
            .and_then(|n| n.attrs.get("id"))
            .filter(|id| !id.is_empty());
        if let Some(id) = id.filter(|id| self.query_selector(&format!("#{id}")) == node) {
            return format!("#{id}");
        }
        if node == self.body() {
            return "body".to_string();
        }
        match self.place_of(node) {
            Some((indices, tag)) => {
                let path: Vec<String> = indices.iter().map(usize::to_string).collect();
                format!("{PLACE}{}:{tag}", path.join("/"))
            }
            None => String::new(),
        }
    }
}

impl DomRecorder {
    /// Whether `node` is an element the open hydration adopted — whose writes this stream makes
    /// bring the markup's own node in line rather than build a new one.
    fn adopting(&self, node: u32) -> bool {
        self.hydration
            .as_ref()
            .is_some_and(|h| h.written.contains_key(&node))
    }

    /// A control the visitor changed before the page ran keeps the change, and the page hears it as
    /// the `input` it missed once the stream's turn has committed.
    fn keep_the_visitors(&mut self, node: u32) {
        let at = self.address(node);
        self.raised.push((at, "input"));
    }

    fn hydrate(&mut self, root: u32) -> Result<(), String> {
        if self.hydration.is_some() {
            return Err(format!(
                "the stream hydrates node {root} while an earlier hydration in it is still open"
            ));
        }
        if let Some(n) = self.get_mut(root) {
            n.attrs
                .remove(logicaffeine_language::parser::lgx_lower::PRERENDERED_ATTR);
        }
        let mut h = Hydration::default();
        h.cursors.insert(root, None);
        self.hydration = Some(h);
        self.log.push(DomOp::Hydrate { root });
        Ok(())
    }

    fn open_hydration(&self, op: &str) -> Result<(), String> {
        match self.hydration {
            Some(_) => Ok(()),
            None => Err(format!("the stream sends `{op}` outside a hydration: only a stream that opened one with `Hydrate` claims nodes the markup holds")),
        }
    }

    fn mismatch(&mut self) {
        if let Some(h) = self.hydration.as_mut() {
            h.mismatches += 1;
        }
    }

    /// Where the children of `parent` no claim has reached begin: just past `last`, the child bound
    /// under it last.
    fn unclaimed_from(&self, parent: u32, last: Option<u32>) -> usize {
        let kids = self
            .get(parent)
            .map(|n| n.children.as_slice())
            .unwrap_or(&[]);
        match last {
            Some(l) => kids
                .iter()
                .position(|&c| c == l)
                .map_or(kids.len(), |i| i + 1),
            None => 0,
        }
    }

    /// An element the host put in the document for itself (`data-logos-host`), never page content.
    fn host_own(&self, h: u32) -> bool {
        self.get(h)
            .is_some_and(|n| !n.is_text && n.attrs.contains_key("data-logos-host"))
    }

    /// The index of the first child of `parent` from `from` on that is a page element.
    fn next_element(&self, parent: u32, from: usize) -> Option<usize> {
        let kids = self
            .get(parent)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        (from..kids.len())
            .find(|&i| self.get(kids[i]).is_some_and(|n| !n.is_text) && !self.host_own(kids[i]))
    }

    /// The text claims waiting under `parent`, taken in stream order.
    fn take_texts(&mut self, parent: u32) -> Vec<(u32, String)> {
        let Some(h) = self.hydration.as_mut() else {
            return Vec::new();
        };
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut h.texts)
            .into_iter()
            .partition(|(p, ..)| *p == parent);
        h.texts = rest;
        mine.into_iter()
            .map(|(_, node, text)| (node, text))
            .collect()
    }

    /// Moves the node `old` names under `parent` to the stream id `new`, which the stream addresses
    /// it by from now on; `old` keeps reaching it.
    fn rebind(&mut self, parent: u32, old: u32, new: u32) -> Result<(), String> {
        let old = self.resolve(old);
        let node = match old.checked_sub(STREAM_ID_BASE) {
            Some(i) => self.streamed.get_mut(i as usize).and_then(Option::take),
            None => self.nodes.get_mut(old as usize).map(std::mem::take),
        }
        .ok_or_else(|| format!("the stream adopts node {old}, which holds nothing"))?;
        self.create_as(new, node)?;
        if let Some(p) = self.get_mut(parent) {
            for c in p.children.iter_mut().filter(|c| **c == old) {
                *c = new;
            }
        }
        for target in self.forwarded.values_mut().filter(|t| **t == old) {
            *target = new;
        }
        self.forwarded.insert(old, new);
        for held in [&mut self.focused, &mut self.scrolled_to, &mut self.selected] {
            if *held == old {
                *held = new;
            }
        }
        let moved: Vec<(String, u8)> = self
            .registrations
            .iter()
            .filter(|((n, _), _)| *n == old)
            .map(|((_, e), f)| (e.clone(), *f))
            .collect();
        for (event, flags) in moved {
            self.registrations.remove(&(old, event.clone()));
            self.registrations.insert((new, event), flags);
        }
        for (n, ..) in self.listeners.iter_mut().filter(|(n, ..)| *n == old) {
            *n = new;
        }
        Ok(())
    }

    /// A new program's stream numbers its nodes from the first stream id again, so the nodes an
    /// earlier program's stream named — its page, which stays in the document for whoever holds it —
    /// take ids this recorder mints, and every stream id is free to bind.
    fn retire_stream(&mut self) {
        let mut renamed = BTreeMap::new();
        for (i, slot) in std::mem::take(&mut self.streamed).into_iter().enumerate() {
            if let Some(node) = slot {
                renamed.insert(STREAM_ID_BASE + i as u32, self.nodes.len() as u32);
                self.nodes.push(node);
            }
        }
        if renamed.is_empty() {
            return;
        }
        let name = |h: &mut u32| {
            if let Some(&minted) = renamed.get(h) {
                *h = minted;
            }
        };
        for n in &mut self.nodes {
            n.children.iter_mut().for_each(name);
        }
        self.forwarded.values_mut().for_each(name);
        for held in [&mut self.focused, &mut self.scrolled_to, &mut self.selected] {
            name(held);
        }
        if let Some(event) = self.current_event.as_mut() {
            name(&mut event.target);
        }
        for (n, ..) in &mut self.listeners {
            name(n);
        }
        self.registrations = std::mem::take(&mut self.registrations)
            .into_iter()
            .map(|((mut n, event), flags)| {
                name(&mut n);
                ((n, event), flags)
            })
            .collect();
    }

    /// A text node the stream names, holding `text`, made where no text of the markup could hold it.
    fn fresh_text(&mut self, node: u32, text: &str) -> Result<u32, String> {
        self.create_as(
            node,
            Node {
                text: text.to_string(),
                is_text: true,
                ..Node::default()
            },
        )?;
        Ok(node)
    }

    /// Makes the text between the children `from` and `to` of `parent` the text claims `run` name:
    /// the markup's text nodes split along the claims when the claims spell them, one node per claim
    /// otherwise; the host's own elements stay where they are. Answers where the settled run ends.
    fn settle_texts(
        &mut self,
        parent: u32,
        from: usize,
        to: usize,
        run: Vec<(u32, String)>,
    ) -> Result<usize, String> {
        let kids = self
            .get(parent)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        let region = &kids[from..to];
        let gap: Vec<u32> = region
            .iter()
            .copied()
            .filter(|&c| self.get(c).is_some_and(|n| n.is_text))
            .collect();
        let held: Vec<String> = gap
            .iter()
            .map(|&c| self.get(c).map(|n| n.text.clone()).unwrap_or_default())
            .collect();
        let wanted: Vec<&str> = run.iter().map(|(_, t)| t.as_str()).collect();
        let mut settled: Vec<u32> = region
            .iter()
            .copied()
            .filter(|&c| self.host_own(c))
            .collect();
        match text_tiles(&held, &wanted) {
            Some(tiles) => {
                for (g, tile) in gap.iter().zip(&tiles) {
                    let mut kept = false;
                    for k in tile.clone() {
                        let (node, text) = &run[k];
                        if !kept && !text.is_empty() {
                            self.rebind(parent, *g, *node)?;
                            if let Some(n) = self.get_mut(*node) {
                                n.text = text.clone();
                            }
                            kept = true;
                            settled.push(*node);
                        } else {
                            settled.push(self.fresh_text(*node, text)?);
                        }
                    }
                }
                let trailing = tiles.last().map_or(0, |t| t.end);
                for (node, text) in &run[trailing..] {
                    settled.push(self.fresh_text(*node, text)?);
                }
            }
            None => {
                for (node, text) in &run {
                    settled.push(self.fresh_text(*node, text)?);
                }
            }
        }
        let end = from + settled.len();
        if let Some(p) = self.get_mut(parent) {
            p.children.splice(from..to, settled);
        }
        Ok(end)
    }

    fn adopt(&mut self, parent: u32, node: u32, tag: &str) -> Result<(), String> {
        self.open_hydration("Adopt")?;
        self.log.push(DomOp::Adopt {
            parent,
            node,
            tag: tag.to_string(),
        });
        let Some(last) = self
            .hydration
            .as_ref()
            .and_then(|h| h.cursors.get(&parent).copied())
        else {
            self.mismatch();
            return Ok(());
        };
        let from = self.unclaimed_from(parent, last);
        let Some(at) = self.next_element(parent, from) else {
            self.mismatch();
            return Ok(());
        };
        let claimed = self.get(parent).map(|n| n.children[at]).unwrap_or(u32::MAX);
        if !self
            .get(claimed)
            .is_some_and(|n| n.tag.eq_ignore_ascii_case(tag))
        {
            self.mismatch();
            return Ok(());
        }
        let run = self.take_texts(parent);
        self.settle_texts(parent, from, at, run)?;
        self.rebind(parent, claimed, node)?;
        if let Some(h) = self.hydration.as_mut() {
            h.cursors.insert(parent, Some(node));
            h.cursors.insert(node, None);
            h.written.insert(node, Default::default());
        }
        Ok(())
    }

    fn adopt_text(&mut self, parent: u32, node: u32, text: &str) -> Result<(), String> {
        self.open_hydration("AdoptText")?;
        self.log.push(DomOp::AdoptText {
            parent,
            node,
            text: text.to_string(),
        });
        match self.hydration.as_mut() {
            Some(h) if h.cursors.contains_key(&parent) => {
                h.texts.push((parent, node, text.to_string()))
            }
            _ => self.mismatch(),
        }
        Ok(())
    }

    /// Ends the hydration the stream opened: each claimed parent's trailing text is settled, the
    /// markup no claim bound leaves, and an adopted element drops the attributes the stream did not
    /// write. Answers the mismatches; `0` when nothing hydrated.
    fn end_hydration(&mut self) -> Result<i64, String> {
        let Some(h) = self.hydration.clone() else {
            return Ok(0);
        };
        for (&parent, &last) in &h.cursors {
            if h.raw.contains(&parent) {
                continue;
            }
            let from = self.unclaimed_from(parent, last);
            let len = self.get(parent).map_or(0, |n| n.children.len());
            let to = self.next_element(parent, from).unwrap_or(len);
            let run = self.take_texts(parent);
            let end = self.settle_texts(parent, from, to, run)?;
            let rest: Vec<u32> = self
                .get(parent)
                .map(|n| n.children[end..].to_vec())
                .unwrap_or_default();
            let mut stays = Vec::new();
            for c in rest {
                if self.host_own(c) {
                    stays.push(c);
                    continue;
                }
                self.leaving(c);
            }
            if let Some(p) = self.get_mut(parent) {
                p.children.truncate(end);
                p.children.extend(stays);
            }
        }
        for (&el, written) in &h.written {
            if let Some(n) = self.get_mut(el) {
                n.attrs.retain(|name, _| written.contains(name));
            }
        }
        self.hydration = None;
        Ok(h.mismatches)
    }
}

/// How the text claims `wanted` tile the markup's text nodes `held`, in order: for each held node,
/// the claims whose texts spell it exactly (an empty claim rides in any tile), or `None` when they
/// do not spell the held text. Claims past the last tile are empty.
fn text_tiles(held: &[String], wanted: &[&str]) -> Option<Vec<std::ops::Range<usize>>> {
    let mut tiles = Vec::with_capacity(held.len());
    let mut k = 0;
    for g in held {
        let start = k;
        let mut spelled = String::new();
        while spelled.len() < g.len() && k < wanted.len() {
            spelled.push_str(wanted[k]);
            k += 1;
        }
        if spelled != *g {
            return None;
        }
        tiles.push(start..k);
    }
    wanted[k..].iter().all(|t| t.is_empty()).then_some(tiles)
}

pub fn remove_node(node: i64) {
    DOM.with(|d| d.borrow_mut().remove_node(node as u32));
}

pub fn insert_before(parent: i64, child: i64, before: i64) {
    DOM.with(|d| {
        d.borrow_mut()
            .insert_before(parent as u32, child as u32, before as u32)
    });
}

pub fn remove_attribute(node: i64, name: &str) {
    DOM.with(|d| d.borrow_mut().remove_attribute(node as u32, name));
}

/// `listenFlags(el, event, flags)`: register `event` on `node` with the behavioural modifiers
/// `flags` names ([`LISTEN_PREVENT`] and its siblings), replacing the flags an earlier registration
/// gave it. [`dispatch_path`] walks by them.
pub fn listen(node: i64, event: &str, flags: u8) {
    DOM.with(|d| d.borrow_mut().listen(node as u32, event, flags));
}

pub fn unlisten(node: i64, event: &str) {
    DOM.with(|d| d.borrow_mut().unlisten(node as u32, event));
}

pub fn free_node(node: i64) {
    DOM.with(|d| d.borrow_mut().free_node(node as u32));
}

/// `Hydrate(root)`: the markup `root` holds is the page's first paint, and the stream's claims
/// adopt its nodes — see [`DomOp::Hydrate`]. The root's `data-logos-prerendered` mark goes with it.
pub fn hydrate(root: i64) -> Result<(), String> {
    DOM.with(|d| d.borrow_mut().hydrate(root as u32))
}

/// `Adopt(parent, node, tag)` — see [`DomOp::Adopt`].
pub fn adopt(parent: i64, node: i64, tag: &str) -> Result<(), String> {
    DOM.with(|d| d.borrow_mut().adopt(parent as u32, node as u32, tag))
}

/// `AdoptText(parent, node, text)` — see [`DomOp::AdoptText`].
pub fn adopt_text(parent: i64, node: i64, text: &str) -> Result<(), String> {
    DOM.with(|d| d.borrow_mut().adopt_text(parent as u32, node as u32, text))
}

/// A program starts: what an earlier program's stream drew keeps its place in the document under
/// ids this recorder mints, and the new program's stream binds its own from the first.
pub fn retire_stream() {
    DOM.with(|d| d.borrow_mut().retire_stream());
}

/// Ends the hydration a stream opened, as the stream ends; answers its mismatches (`0` when the
/// stream hydrated nothing).
pub fn end_hydration() -> Result<i64, String> {
    DOM.with(|d| d.borrow_mut().end_hydration())
}

// ─── Native constraint validation: what a browser checks before it submits a form ────────────

/// Why a form control is invalid, flag for flag as a browser's `ValidityState` reports it. The
/// constraints modelled are `required`, `type="email"`/`url"`/`number"`, `minlength`/`maxlength`,
/// `pattern` and a number's `min`/`max`/`step`; `min`/`max`/`step` on the date and time types are
/// not modelled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Validity {
    pub value_missing: bool,
    pub type_mismatch: bool,
    pub too_short: bool,
    pub too_long: bool,
    pub pattern_mismatch: bool,
    pub range_underflow: bool,
    pub range_overflow: bool,
    pub step_mismatch: bool,
    pub bad_input: bool,
}

impl Validity {
    /// No constraint fails.
    pub fn valid(&self) -> bool {
        *self == Validity::default()
    }
}

impl DomRecorder {
    /// A control's value as a browser reports it: a checkbox's or radio's `value` attribute (`on`
    /// when it has none), whether ticked or not; any other control's what the visitor or the page
    /// last put there, else what its markup says — a `<select>`'s chosen option, anything else's
    /// `value` attribute.
    fn control_value(&self, h: u32) -> String {
        let Some(n) = self.get(h) else {
            return String::new();
        };
        if matches!(input_type(n).as_str(), "checkbox" | "radio") {
            return n
                .attrs
                .get("value")
                .cloned()
                .unwrap_or_else(|| "on".to_string());
        }
        if n.user_edited || !n.value.is_empty() {
            return n.value.clone();
        }
        match n.tag.eq_ignore_ascii_case("select") {
            true => self.default_choice(h).unwrap_or_default(),
            false => n.attrs.get("value").cloned().unwrap_or_default(),
        }
    }

    /// Whether `h` is barred from constraint validation: disabled (itself, or by a disabled
    /// `<fieldset>` it is not in the first `<legend>` of), read-only where `readonly` applies, a
    /// hidden input, or inside a `<datalist>`.
    fn barred(&self, h: u32) -> bool {
        let Some(n) = self.get(h) else { return true };
        let has = |name: &str| n.attrs.contains_key(name);
        let ty = input_type(n);
        let readonly_applies = n.tag.eq_ignore_ascii_case("textarea")
            || matches!(
                ty.as_str(),
                "text"
                    | "search"
                    | "url"
                    | "tel"
                    | "email"
                    | "password"
                    | "date"
                    | "month"
                    | "week"
                    | "time"
                    | "datetime-local"
                    | "number"
            );
        if has("disabled") || (readonly_applies && has("readonly")) || ty == "hidden" {
            return true;
        }
        let chain = self.ancestors_of(h).unwrap_or_default();
        for (i, &a) in chain.iter().enumerate() {
            let Some(anc) = self.get(a) else { continue };
            if anc.tag.eq_ignore_ascii_case("datalist") {
                return true;
            }
            if anc.tag.eq_ignore_ascii_case("fieldset") && anc.attrs.contains_key("disabled") {
                let first_legend = anc.children.iter().copied().find(|&c| {
                    self.get(c)
                        .is_some_and(|x| !x.is_text && x.tag.eq_ignore_ascii_case("legend"))
                });
                let inside_legend = first_legend.is_some_and(|l| chain.get(i + 1) == Some(&l));
                if !inside_legend {
                    return true;
                }
            }
        }
        false
    }

    /// The `<form>` a control belongs to: the one its `form` attribute names, else its nearest
    /// form ancestor.
    fn form_owner(&self, h: u32) -> Option<u32> {
        if let Some(id) = self.get(h).and_then(|n| n.attrs.get("form")) {
            let found = self.query_selector(&format!("#{id}"));
            return self
                .get(found)
                .filter(|f| f.tag.eq_ignore_ascii_case("form"))
                .map(|_| found);
        }
        let chain = self.ancestors_of(h)?;
        chain.into_iter().rev().find(|&a| {
            self.get(a)
                .is_some_and(|n| n.tag.eq_ignore_ascii_case("form"))
        })
    }

    /// Whether some radio of `h`'s group — the same `name`, the same form owner — is ticked.
    fn radio_group_ticked(&self, h: u32) -> bool {
        let Some(n) = self.get(h) else { return false };
        let Some(name) = n.attrs.get("name").filter(|x| !x.is_empty()) else {
            return n.checked;
        };
        let owner = self.form_owner(h);
        let mut stack = vec![0u32];
        while let Some(at) = stack.pop() {
            let Some(x) = self.get(at) else { continue };
            if !x.is_text
                && input_type(x) == "radio"
                && x.attrs.get("name") == Some(name)
                && self.form_owner(at) == owner
                && x.checked
            {
                return true;
            }
            stack.extend(x.children.iter().rev().copied());
        }
        false
    }

    fn validity(&self, h: u32) -> Result<Validity, String> {
        let mut v = Validity::default();
        let Some(n) = self.get(h) else { return Ok(v) };
        let tag = n.tag.to_ascii_lowercase();
        if n.is_text || !matches!(tag.as_str(), "input" | "select" | "textarea") || self.barred(h) {
            return Ok(v);
        }
        let ty = input_type(n);
        let value = self.control_value(h);
        if n.attrs.contains_key("required") {
            v.value_missing = match ty.as_str() {
                "checkbox" => !n.checked,
                "radio" => !self.radio_group_ticked(h),
                _ => value.is_empty(),
            };
        }
        let text_like = tag == "textarea"
            || matches!(
                ty.as_str(),
                "text" | "search" | "url" | "tel" | "email" | "password"
            );
        if text_like && n.user_edited && !value.is_empty() {
            let units = value.encode_utf16().count();
            let limit = |name: &str| {
                n.attrs
                    .get(name)
                    .and_then(|x| x.trim().parse::<usize>().ok())
            };
            v.too_long = limit("maxlength").is_some_and(|max| units > max);
            v.too_short = limit("minlength").is_some_and(|min| units < min);
        }
        let values: Vec<&str> = match (ty.as_str(), n.attrs.contains_key("multiple")) {
            ("email", true) => value
                .split(',')
                .map(|x| x.trim_matches(|c: char| c.is_ascii_whitespace()))
                .collect(),
            _ => vec![value.as_str()],
        };
        if !value.is_empty() {
            match ty.as_str() {
                "email" => v.type_mismatch = values.iter().any(|x| !valid_email(x)),
                "url" => v.type_mismatch = !valid_url(&value),
                _ => {}
            }
            if tag == "input"
                && matches!(
                    ty.as_str(),
                    "text" | "search" | "url" | "tel" | "email" | "password"
                )
            {
                if let Some(pattern) = n.attrs.get("pattern") {
                    for x in &values {
                        if !pattern_matches(pattern, x)? {
                            v.pattern_mismatch = true;
                        }
                    }
                }
            }
        }
        if ty == "number" {
            v.bad_input = n.bad_input;
            let num = |name: &str| {
                n.attrs
                    .get(name)
                    .filter(|x| valid_float(x))
                    .and_then(|x| x.parse::<f64>().ok())
            };
            if let Some(x) = value.parse::<f64>().ok().filter(|_| valid_float(&value)) {
                v.range_underflow = num("min").is_some_and(|min| x < min);
                v.range_overflow = num("max").is_some_and(|max| x > max);
                let step = match n.attrs.get("step").map(|s| s.trim().to_ascii_lowercase()) {
                    Some(any) if any == "any" => None,
                    Some(s) => Some(
                        Some(s.as_str())
                            .filter(|s| valid_float(s))
                            .and_then(|s| s.parse::<f64>().ok())
                            .filter(|s| *s > 0.0)
                            .unwrap_or(1.0),
                    ),
                    None => Some(1.0),
                };
                if let Some(step) = step {
                    let base = num("min").or_else(|| num("value")).unwrap_or(0.0);
                    let q = (x - base) / step;
                    v.step_mismatch = (q - q.round()).abs() > 1e-9 * q.abs().max(1.0);
                }
            }
        }
        Ok(v)
    }
}

/// Whether `s` is an HTML valid floating-point number: an optional `-`, digits with an optional
/// fraction (or a fraction alone), and an optional exponent.
fn valid_float(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = usize::from(b.first() == Some(&b'-'));
    let digits = |i: &mut usize| {
        let start = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i > start
    };
    let whole = digits(&mut i);
    let fraction = if b.get(i) == Some(&b'.') {
        i += 1;
        if !digits(&mut i) {
            return false;
        }
        true
    } else {
        false
    };
    if !whole && !fraction {
        return false;
    }
    if matches!(b.get(i), Some(b'e') | Some(b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+') | Some(b'-')) {
            i += 1;
        }
        if !digits(&mut i) {
            return false;
        }
    }
    i == b.len()
}

/// Whether `s` is a valid e-mail address as HTML defines it: a local part of the permitted
/// characters, `@`, and a domain of dot-separated labels, each 1–63 letters, digits and inner
/// hyphens.
fn valid_email(s: &str) -> bool {
    let Some((local, domain)) = s.split_once('@') else {
        return false;
    };
    let local_ok = !local.is_empty()
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".!#$%&'*+/=?^_`{|}~-".contains(c));
    let label_ok = |l: &str| {
        let b = l.as_bytes();
        (1..=63).contains(&b.len())
            && b[0].is_ascii_alphanumeric()
            && b[b.len() - 1].is_ascii_alphanumeric()
            && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-')
    };
    local_ok && domain.split('.').all(label_ok)
}

/// Whether `s` parses as an absolute URL: a scheme, and — for the schemes a host is part of —
/// a non-empty host with no character a host cannot hold (an IPv6 literal in brackets holds hex
/// digits, `:` and `.`), and a port of digits.
fn valid_url(s: &str) -> bool {
    let Some((scheme, rest)) = s.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    let scheme_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !scheme_ok {
        return false;
    }
    if !matches!(
        scheme.to_ascii_lowercase().as_str(),
        "http" | "https" | "ws" | "wss" | "ftp"
    ) {
        return true;
    }
    let authority = rest
        .trim_start_matches(['/', '\\'])
        .split(['/', '\\', '?', '#'])
        .next()
        .unwrap_or("");
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let (host_ok, port) = match host_port.strip_prefix('[') {
        Some(inner) => match inner.split_once(']') {
            Some((addr, tail)) => (
                !addr.is_empty()
                    && addr
                        .chars()
                        .all(|c| c.is_ascii_hexdigit() || matches!(c, ':' | '.')),
                tail.strip_prefix(':'),
            ),
            None => return false,
        },
        None => {
            let (host, port) = match host_port.rsplit_once(':') {
                Some((h, p)) => (h, Some(p)),
                None => (host_port, None),
            };
            let forbidden =
                |c: char| c.is_control() || c.is_whitespace() || "#%/:<>?@[\\]^|".contains(c);
            (!host.is_empty() && !host.chars().any(forbidden), port)
        }
    };
    host_ok && port.is_none_or(|p| p.chars().all(|c| c.is_ascii_digit()))
}

/// Whether `value` matches a control's `pattern=` — an ECMAScript regular expression matched
/// against the WHOLE value (`^(?:pattern)$`), with the `v` flag a browser compiles it with: `\d`,
/// `\w` and `\s` are ASCII digits, word characters and JavaScript's whitespace, and a class may
/// nest classes and combine them with `--` and `&&`. A pattern that is not a valid expression
/// constrains nothing, as in a browser. The `v` flag's strings — `\q{…}` and the string properties
/// (`\p{RGI_Emoji}`) — are refused by name: the engine here cannot express them.
pub fn pattern_matches(pattern: &str, value: &str) -> Result<bool, String> {
    let refuse = |what: &str| {
        Err(format!(
            "`pattern=\"{pattern}\"` uses {what}, which the in-process tiers cannot check; a browser \
             can — write the constraint without it, or check the value in an action"
        ))
    };
    const STRING_PROPERTIES: &[&str] = &[
        "Basic_Emoji",
        "Emoji_Keycap_Sequence",
        "RGI_Emoji_Modifier_Sequence",
        "RGI_Emoji_Flag_Sequence",
        "RGI_Emoji_Tag_Sequence",
        "RGI_Emoji_ZWJ_Sequence",
        "RGI_Emoji",
    ];
    const JS_SPACE: &str = "\\t\\n\\x0B\\x0C\\r \\u{A0}\\u{1680}\\u{2000}-\\u{200A}\\u{2028}\\u{2029}\\u{202F}\\u{205F}\\u{3000}\\u{FEFF}";
    let mut out = String::from("^(?:");
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    let mut depth = 0usize;
    let mut class_atoms = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            let Some(&e) = chars.get(i + 1) else {
                return Ok(true);
            };
            if e == 'q' && chars.get(i + 2) == Some(&'{') {
                return refuse("a `\\q{…}` string literal (the `v` flag's set notation)");
            }
            if matches!(e, 'p' | 'P' | 'u') && chars.get(i + 2) == Some(&'{') {
                let name: String = chars[i + 3..].iter().take_while(|c| **c != '}').collect();
                if e != 'u' && STRING_PROPERTIES.contains(&name.as_str()) {
                    return refuse(&format!("the string property `\\p{{{name}}}`"));
                }
                let end = i + 3 + name.chars().count();
                if chars.get(end) != Some(&'}') {
                    return Ok(true);
                }
                out.extend(&chars[i..=end]);
                class_atoms += 1;
                i = end + 1;
                continue;
            }
            let in_class = depth > 0;
            let class = |set: &str, negated: bool| match (in_class, negated) {
                (true, false) => set.to_string(),
                (false, false) => format!("[{set}]"),
                (false, true) => format!("[^{set}]"),
                (true, true) => String::new(),
            };
            let translated = match e {
                'd' => Some(class("0-9", false)),
                'D' => Some(class("0-9", true)),
                'w' => Some(class("A-Za-z0-9_", false)),
                'W' => Some(class("A-Za-z0-9_", true)),
                's' => Some(class(JS_SPACE, false)),
                'S' => Some(class(JS_SPACE, true)),
                _ => None,
            };
            match translated {
                Some(t) if t.is_empty() => {
                    return refuse(&format!("a negated `\\{e}` inside a class"))
                }
                Some(t) => out.push_str(&t),
                None => {
                    out.push(c);
                    out.push(e);
                }
            }
            class_atoms += 1;
            i += 2;
            continue;
        }
        if depth > 0 {
            match c {
                ']' => {
                    depth -= 1;
                    class_atoms += 1;
                    out.push(c);
                }
                '[' => {
                    depth += 1;
                    class_atoms = 0;
                    out.push(c);
                    if chars.get(i + 1) == Some(&'^') {
                        out.push('^');
                        i += 1;
                    }
                }
                '-' | '&' if chars.get(i + 1) == Some(&c) => {
                    out.push(c);
                    out.push(c);
                    class_atoms = 0;
                    i += 2;
                    continue;
                }
                '-' if class_atoms == 0 || chars.get(i + 1) == Some(&']') => return Ok(true),
                '(' | ')' | '{' | '}' | '/' | '|' => return Ok(true),
                _ => {
                    out.push(c);
                    class_atoms += 1;
                }
            }
            i += 1;
            continue;
        }
        if c == '[' {
            depth = 1;
            class_atoms = 0;
            out.push(c);
            if chars.get(i + 1) == Some(&'^') {
                out.push('^');
                i += 1;
            }
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    if depth > 0 {
        return Ok(true);
    }
    out.push_str(")$");
    match fancy_regex::Regex::new(&out) {
        Ok(re) => re.is_match(value).map_err(|e| {
            format!("`pattern=\"{pattern}\"` could not be checked against {value:?}: {e}")
        }),
        Err(_) => Ok(true),
    }
}

/// A control's value as a browser reports it — see [`Validity`].
pub fn control_value(node: i64) -> String {
    DOM.with(|d| d.borrow().control_value(node as u32))
}

/// Why `node` is invalid ([`Validity`]) — every flag clear for an element with no constraints or
/// barred from them. Refused only for a `pattern=` this tier cannot check ([`pattern_matches`]).
pub fn validity(node: i64) -> Result<Validity, String> {
    DOM.with(|d| d.borrow().validity(node as u32))
}

/// The `<form>` `node` belongs to, if any: itself when it is one, else its form owner.
pub fn form_of(node: i64) -> Option<i64> {
    DOM.with(|d| {
        let d = d.borrow();
        match d.get(node as u32) {
            Some(n) if !n.is_text && n.tag.eq_ignore_ascii_case("form") => Some(node),
            Some(_) => d.form_owner(node as u32).map(i64::from),
            None => None,
        }
    })
}

/// The controls that stop `form` from being submitted, in document order: none when the form
/// says `novalidate` or its `submitter` says `formnovalidate`, else every control the form owns
/// whose [`validity`] is not valid.
pub fn form_constraints(form: i64, submitter: Option<i64>) -> Result<Vec<i64>, String> {
    DOM.with(|d| {
        let d = d.borrow();
        let has = |h: i64, name: &str| d.get(h as u32).is_some_and(|n| n.attrs.contains_key(name));
        if has(form, "novalidate") || submitter.is_some_and(|s| has(s, "formnovalidate")) {
            return Ok(Vec::new());
        }
        let mut invalid = Vec::new();
        let mut stack = vec![0u32];
        while let Some(at) = stack.pop() {
            let Some(x) = d.get(at) else { continue };
            if !x.is_text
                && matches!(
                    x.tag.to_ascii_lowercase().as_str(),
                    "input" | "select" | "textarea"
                )
                && d.form_owner(at) == Some(form as u32)
                && !d.validity(at)?.valid()
            {
                invalid.push(i64::from(at));
            }
            stack.extend(x.children.iter().rev().copied());
        }
        Ok(invalid)
    })
}

/// Whether activating `node` submits its form: a `<button>` of type `submit` (a button's type when
/// it names none), or an `<input>` of type `submit` or `image`.
pub fn submits_its_form(node: i64) -> bool {
    DOM.with(|d| {
        let d = d.borrow();
        let Some(n) = d.get(node as u32).filter(|n| !n.is_text) else {
            return false;
        };
        let ty = n.attrs.get("type").map(|t| t.trim().to_ascii_lowercase());
        match n.tag.to_ascii_lowercase().as_str() {
            "button" => {
                ty.is_none_or(|t| t == "submit" || !matches!(t.as_str(), "reset" | "button"))
            }
            "input" => matches!(ty.as_deref(), Some("submit") | Some("image")),
            _ => false,
        }
    })
}
