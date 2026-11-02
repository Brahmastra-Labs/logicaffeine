//! The LGX component model — the first-class AST for a page, a component, a store, and the
//! sections they own: `State:` (the record), `Actions:` (the only mutators, plus the
//! English event sentences that trigger them), `View:` (the template, already parsed into
//! the view IR), and `Style:` (the component's CSS).
//!
//! Nothing here is desugared: the definition is kept as it was written, and the lowering
//! (`parser::lgx_lower`) turns it into the page's LOGOS, which every tier runs — the tree-walker,
//! the VM for WASM and the compiled backends, AOT-Rust for server rendering.

use crate::ast::stmt::{Block, Expr, Stmt, TypeExpr};
use crate::ast::view::{expr_shape, ViewDef};
use crate::token::Span;
use logicaffeine_base::{Interner, Symbol};

/// One `## Page` / `## Component` / `## Store` / `## Error page` definition.
#[derive(Debug, Clone)]
pub struct ComponentDef<'a> {
    pub kind: ComponentKind<'a>,
    pub name: Symbol,
    /// Header parameters — a component's props (`## Component Row (todo: Todo)`), read-only
    /// inside it. A page's typed path parameters live in its kind.
    pub props: Vec<(Symbol, &'a TypeExpr<'a>)>,
    /// `Use Session.` — the stores this component reads and calls.
    pub uses: Vec<Symbol>,
    pub state: Vec<StateField<'a>>,
    pub events: Vec<EventHandler<'a>>,
    pub actions: Vec<ActionDef<'a>>,
    pub view: Option<ViewDef<'a>>,
    /// `Head:` — the head HTML this page declares for its own document: its title,
    /// description, social card, canonical URL. Rendered at BUILD time into the
    /// document the server sends, and merged over the default shell tag by tag, so
    /// a page that declares nothing keeps the document it has today.
    ///
    /// Only a `## Page` can carry one: a component is placed INSIDE a document and
    /// does not own it, and two components on one page declaring a title would make
    /// the answer depend on render order.
    pub head: Option<ViewDef<'a>>,
    pub style: Option<StyleBlock>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ComponentKind<'a> {
    Component,
    /// `## Page /todos/:id (id: Int)` — mounted at `path`; each `:name` binds a typed param.
    Page {
        path: Symbol,
        params: Vec<(Symbol, &'a TypeExpr<'a>)>,
    },
    /// `## Store Name [persisted as "key" | shared on "topic"]`.
    Store {
        mode: StoreMode<'a>,
    },
    /// `## Error page 404`.
    ErrorPage {
        status: u16,
    },
}

#[derive(Debug, Clone)]
pub enum StoreMode<'a> {
    Local,
    Persisted {
        key: &'a Expr<'a>,
    },
    Shared {
        topic: &'a Expr<'a>,
    },
    /// `preserved across reloads` — state that survives a HOT RELOAD but not a fresh session.
    ///
    /// Distinct from `persisted as "<key>"`, which survives everything and is a decision about
    /// the PRODUCT. This is a decision about the EDIT LOOP: a rebuild must not empty the form you
    /// were filling in, and a cold start must not resurrect yesterday's draft. Between those two
    /// there is no key to choose, because the state never leaves the session.
    Preserved,
}

/// `Let x be <init>.` or `Let x always be <expr>.` (derived) in a `State:` section.
#[derive(Debug, Clone)]
pub struct StateField<'a> {
    pub name: Symbol,
    pub ty: Option<&'a TypeExpr<'a>>,
    pub init: &'a Expr<'a>,
    pub derived: bool,
    pub span: Span,
}

/// What an English `When …:` sentence in `Actions:` waits for.
#[derive(Debug, Clone)]
pub enum Trigger<'a> {
    /// `When the page loads:`
    PageLoads,
    /// `When this component appears:`
    Appears,
    /// `When this component disappears:`
    Disappears,
    /// `When the route changes:`
    RouteChanges,
    /// `When the app comes back online:`
    Online,
    /// `When the app goes offline:`
    Offline,
    /// `When "#add" is submitted:` / `When "#draft" changes:` / `When the user types in "#q":` /
    /// `When the user presses Escape in "#draft":` (`key` set) / `When "#x" is focused:` …
    Selector {
        selector: &'a Expr<'a>,
        event: Symbol,
        key: Option<Symbol>,
    },
    /// `When the user presses Escape:` — a key anywhere on the page.
    Key { key: Symbol },
    /// `When todos change:` / `When Session's name changes:`
    FieldChanges {
        store: Option<Symbol>,
        field: Symbol,
    },
    /// `Every 60 seconds:` — repeating. `Every 60 seconds as refresh:` names it, and a named
    /// timer is the only kind `Stop the timer …` can reach.
    Every {
        seconds: &'a Expr<'a>,
        name: Option<Symbol>,
    },
    /// `After 5 seconds:` — one-shot, optionally named so it can be cancelled before it fires.
    After {
        seconds: &'a Expr<'a>,
        name: Option<Symbol>,
    },
    /// `When a message arrives on "topic" (m: T):`
    Message {
        topic: &'a Expr<'a>,
        param: (Symbol, &'a TypeExpr<'a>),
    },
    /// `When an action fails (error: Text):`
    ActionFails { param: Symbol },
    /// `When a fetch fails (url: Text) and (error: Text):`
    FetchFails { url: Symbol, error: Symbol },
    /// `When the window resizes:`
    WindowResizes,
    /// `When the user scrolls to the bottom [of "#list"]:`
    ScrollsToBottom { selector: Option<&'a Expr<'a>> },
    /// `When a file is dropped on "#zone" (file: File):` or `(name: Text)` — the declared type
    /// decides what binds: `File` the whole record, `Text` just the name, same as [`Trigger::
    /// Message`]'s own `(Symbol, &TypeExpr)` shape keeps its payload's declared type.
    FileDropped {
        selector: &'a Expr<'a>,
        param: (Symbol, &'a TypeExpr<'a>),
    },
}

impl<'a> Trigger<'a> {
    /// ★ THE TRIGGERS NO TIER DELIVERS — one table, read by every tier that has to say so.
    ///
    /// `Some((sentence, needs))` when nothing anywhere can fire this trigger yet; `None` when some
    /// tier delivers it.
    ///
    /// The distinction is not pedantic, and getting it wrong produced a false statement in a
    /// compiler message: the browser-tier mount refusal ended *"The tree-walker renders it; the
    /// bytecode tier cannot yet"*, which is true of `Every … seconds:` and **false** of
    /// `When the window resizes:` — nothing renders that one. An author would have been told
    /// their page works on the tree-walker and sent looking for a tier to run it on.
    ///
    /// It lives here because every tier has to agree about which sentences exist at all, and a
    /// second copy of a list is how they stop agreeing.
    pub fn undeliverable(&self) -> Option<(&'static str, &'static str)> {
        // ★ EMPTY, AND THAT IS THE POINT. `Message` was the last entry: it waited on "the
        // realtime relay", which turned out to be BUILT — `logicaffeine_system::relay` carries
        // `Subscribe`/`Publish`/`Event` between a native node and a browser tab over one shared
        // wire protocol. What was missing was the LGX end, not the transport. The table stays
        // because the next sentence that outruns its tiers belongs here, and because two tiers
        // agreeing about which sentences exist at all is worth a list even when it is empty.
        match self {
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EventHandler<'a> {
    pub trigger: Trigger<'a>,
    pub body: Block<'a>,
    pub span: Span,
}

/// `To name (params):` in `Actions:`.
#[derive(Debug, Clone)]
pub struct ActionDef<'a> {
    pub name: Symbol,
    pub params: Vec<(Symbol, &'a TypeExpr<'a>)>,
    pub body: Block<'a>,
    pub span: Span,
}

/// A `Style:` section: the dedented CSS text, the file offset of its first character, and the
/// indentation stripped from each of its lines.
#[derive(Debug, Clone)]
pub struct StyleBlock {
    pub css: Symbol,
    pub content_start: usize,
    pub indent: usize,
}

impl StyleBlock {
    /// The file offset of offset `at` in the stylesheet text `css` (this block's own text).
    pub fn file_offset(&self, css: &str, at: usize) -> usize {
        crate::lgx::raw::RawMap::new(css, self.indent, self.content_start).offset(at)
    }
}

/// A `## Route`'s body: ordinary statements, or a template for a `-> View` route. The template
/// is boxed: a `ViewDef` inline would make every `Stmt` as wide as a view route.
#[derive(Debug, Clone)]
pub enum RouteBody<'a> {
    Statements(Block<'a>),
    View(ViewDef<'a>),
}

fn type_shape(ty: &TypeExpr<'_>, interner: &Interner) -> String {
    match ty {
        TypeExpr::Primitive(s) | TypeExpr::Named(s) => interner.resolve(*s).to_string(),
        TypeExpr::Generic { base, params } => {
            let ps: Vec<String> = params.iter().map(|p| type_shape(p, interner)).collect();
            format!("{} of {}", interner.resolve(*base), ps.join(" and "))
        }
        other => format!("{other:?}")
            .split_whitespace()
            .next()
            .unwrap_or("_")
            .to_string(),
    }
}

fn params_shape(params: &[(Symbol, &TypeExpr<'_>)], interner: &Interner) -> String {
    if params.is_empty() {
        return String::new();
    }
    let ps: Vec<String> = params
        .iter()
        .map(|(n, t)| format!("{}: {}", interner.resolve(*n), type_shape(t, interner)))
        .collect();
    format!("({})", ps.join(", "))
}

impl<'a> Trigger<'a> {
    /// The English FORM this trigger is written in — the words an author would search their own
    /// file for.
    ///
    /// ★ DELIBERATELY WITHOUT THE VALUES, and that is the difference from [`Self::shape`]. `shape` is a
    /// stable debug rendering for snapshot tests, and it prints what the AST holds: a duration comes
    /// out as `every 60000000000ms`, which is true and is not what anybody typed. A refusal is read
    /// by the person who wrote the sentence, so it names the FORM (`Every … seconds:`) and lets them
    /// find the line. Two renderings because there are two audiences; kept adjacent so neither can
    /// be changed without seeing the other.
    pub fn form(&self) -> &'static str {
        match self {
            Trigger::PageLoads => "When the page loads:",
            Trigger::Appears => "When this component appears:",
            Trigger::Disappears => "When this component disappears:",
            Trigger::RouteChanges => "When the route changes:",
            Trigger::Online => "When the app comes back online:",
            Trigger::Offline => "When the app goes offline:",
            Trigger::Selector { .. } => "When \"…\" is …:",
            Trigger::Key { .. } => "When the user presses …:",
            // ★ TWO SENTENCES, TWO FORMS — and collapsing them is what let a whole trigger go
            // uncatalogued. `FieldChanges` carries an optional STORE, and the two readings are
            // different things: one watches this component's own field, the other watches shared
            // state across components. They reached the browser at different times, and while
            // this function returned one string for both, the refusal for the store form quoted a
            // sentence the author had not written and the tier census could not tell the two rows
            // apart. A `form()` that under-describes the language is a blind spot every reader of
            // it inherits.
            Trigger::FieldChanges { store: Some(_), .. } => "When <Store>'s … changes:",
            Trigger::FieldChanges { store: None, .. } => "When … changes:",
            Trigger::Every { .. } => "Every … seconds:",
            Trigger::After { .. } => "After … seconds:",
            Trigger::Message { .. } => "When a message arrives on …:",
            Trigger::ActionFails { .. } => "When an action fails …:",
            Trigger::FetchFails { .. } => "When a fetch fails …:",
            Trigger::WindowResizes => "When the window resizes:",
            Trigger::ScrollsToBottom { .. } => "When the user scrolls to the bottom …:",
            Trigger::FileDropped { .. } => "When a file is dropped on …:",
        }
    }

    /// A compact rendering for locks and tooling.
    pub fn shape(&self, interner: &Interner) -> String {
        match self {
            Trigger::PageLoads => "page loads".into(),
            Trigger::Appears => "appears".into(),
            Trigger::Disappears => "disappears".into(),
            Trigger::RouteChanges => "route changes".into(),
            Trigger::Online => "online".into(),
            Trigger::Offline => "offline".into(),
            Trigger::Selector {
                selector,
                event,
                key,
            } => match key {
                Some(k) => format!(
                    "{} {} {}",
                    expr_shape(selector, interner),
                    interner.resolve(*event),
                    interner.resolve(*k)
                ),
                None => format!(
                    "{} {}",
                    expr_shape(selector, interner),
                    interner.resolve(*event)
                ),
            },
            Trigger::Key { key } => format!("key {}", interner.resolve(*key)),
            Trigger::FieldChanges { store: None, field } => {
                format!("{} changes", interner.resolve(*field))
            }
            Trigger::FieldChanges {
                store: Some(s),
                field,
            } => format!(
                "{}'s {} changes",
                interner.resolve(*s),
                interner.resolve(*field)
            ),
            Trigger::Every { seconds, name } => match name {
                None => format!("every {}", expr_shape(seconds, interner)),
                Some(n) => format!(
                    "every {} as {}",
                    expr_shape(seconds, interner),
                    interner.resolve(*n)
                ),
            },
            Trigger::After { seconds, name } => match name {
                None => format!("after {}", expr_shape(seconds, interner)),
                Some(n) => format!(
                    "after {} as {}",
                    expr_shape(seconds, interner),
                    interner.resolve(*n)
                ),
            },
            Trigger::Message { topic, param } => format!(
                "message on {} ({})",
                expr_shape(topic, interner),
                interner.resolve(param.0)
            ),
            Trigger::ActionFails { param } => {
                format!("action fails ({})", interner.resolve(*param))
            }
            Trigger::FetchFails { url, error } => format!(
                "fetch fails ({}, {})",
                interner.resolve(*url),
                interner.resolve(*error)
            ),
            Trigger::WindowResizes => "window resizes".into(),
            Trigger::ScrollsToBottom { selector: None } => "scrolls to bottom".into(),
            Trigger::ScrollsToBottom { selector: Some(s) } => {
                format!("scrolls to bottom of {}", expr_shape(s, interner))
            }
            Trigger::FileDropped { selector, param } => format!(
                "file dropped on {} ({})",
                expr_shape(selector, interner),
                interner.resolve(param.0)
            ),
        }
    }
}

/// How many rounds of `When <Store>'s <field> changes:` may cascade — a watcher's write waking a
/// watcher, which wakes another — before the page is refused as one that never settles. Every tier
/// counts the same rounds and says the same sentence ([`store_cascade_message`]).
pub const STORE_CASCADE_LIMIT: usize = 16;

/// What every tier says of a store-watcher cascade deeper than [`STORE_CASCADE_LIMIT`].
pub fn store_cascade_message() -> String {
    format!(
        "a `When <Store>'s <field> changes:` handler keeps changing a store, so the page never \
         settles — {STORE_CASCADE_LIMIT} rounds and still going. A watcher that writes the store it \
         watches (directly, or through another store that watches back) has no resting point. Make \
         the write conditional on something that stops, or move it into the action that started \
         this."
    )
}

/// What every tier says of a timer whose period, as its instance was drawn, is not positive.
pub fn timer_period_not_positive() -> &'static str {
    "A timer needs a positive number of seconds — `Every 0 seconds:` would never leave the moment \
     it was scheduled in."
}

/// What every tier says of `Get … as <ty>` when the derived JSON codec cannot read `ty`: a typed
/// request decodes its answer through `fromJson_<T>` for a declared record, `fromJsonSeq_<T>` for
/// a list of records or scalars, and takes Text verbatim — nothing else.
pub fn typed_fetch_refusal(ty: &TypeExpr<'_>, interner: &Interner) -> String {
    format!(
        "`as {}` is not a type this fetch can read yet: a typed request decodes through the derived \
         `fromJson_<T>`, which covers a declared record, an Int or a Bool, a `Seq of` records or \
         scalars, and Text verbatim. Ask for the record itself, or read the body as Text.",
        type_shape(ty, interner)
    )
}

/// What every tier puts in front of a trap inside a `## Directive`'s block (`removed`: its `When
/// removed:` block): from the page's side, the directive is what went wrong.
pub fn directive_failure_prefix(directive: &str, removed: bool) -> String {
    if removed {
        format!("The `{directive}` directive's `When removed:` block failed: ")
    } else {
        format!("The `{directive}` directive failed: ")
    }
}

/// How deeply components may nest while one paint is drawn before the page is refused as one with
/// no finite rendering ([`render_depth_message`]).
///
/// ★ THE BUDGET LANDS BEFORE THE NATIVE STACK DOES, because every level is a real frame chain — a
/// component that placed itself with nothing to stop it aborted the process. The number is measured,
/// not picked: 64 still crashed; 32 and 24 both reported cleanly. It is far past any page nested by
/// hand (a deep menu or comment thread is single digits); a design that needs more wants an
/// ITERATIVE render walk, not a larger number, which walks back toward the crash.
pub const RENDER_DEPTH_LIMIT: usize = 32;

/// What every tier says when `component` is nested past [`RENDER_DEPTH_LIMIT`] while drawing.
pub fn render_depth_message(component: &str) -> String {
    format!(
        "`{component}` is nested more than {RENDER_DEPTH_LIMIT} deep while drawing the page, so it has \
         no finite rendering. A component whose view places itself needs something that stops — \
         `{{if depth is greater than 0:}}` around the placement, with a smaller value passed down \
         each time."
    )
}

impl<'a> ComponentDef<'a> {
    /// How a message names this definition: a component by its own name, a page or error page by
    /// its header — whose internal name (`__page__about`) appears nowhere in the source.
    pub fn label(&self, interner: &Interner) -> String {
        match &self.kind {
            ComponentKind::Page { path, .. } => format!("`## Page {}`", interner.resolve(*path)),
            ComponentKind::ErrorPage { status } => format!("`## Error page {status}`"),
            ComponentKind::Store { .. } => format!("`## Store {}`", interner.resolve(self.name)),
            ComponentKind::Component => format!("`{}`", interner.resolve(self.name)),
        }
    }

    /// A body's statement count as an AUTHOR would count it: `Stmt::Site` is a marker the parser
    /// inserts before each of an action or event handler's own sentences (Wave 3's runtime
    /// failure-site table, `parser/mod.rs`'s `site_tracking`) so a trap can name which one ran —
    /// invisible bookkeeping, never a sentence the author wrote. Counting it too would double
    /// every tracked body's own length and misreport it to whoever reads `largo expand`.
    fn authored_len(body: Block<'a>) -> usize {
        body.iter().filter(|s| !matches!(s, Stmt::Site(_))).count()
    }

    /// A compact, deterministic one-line rendering — the snapshot surface the parser lock
    /// pins and `largo expand` prints. Bodies print as their statement counts.
    pub fn shape(&self, interner: &Interner) -> String {
        let head = match &self.kind {
            ComponentKind::Component => format!(
                "component {}{}",
                interner.resolve(self.name),
                params_shape(&self.props, interner)
            ),
            ComponentKind::Page { path, params } => format!(
                "page {}{}",
                interner.resolve(*path),
                params_shape(params, interner)
            ),
            ComponentKind::Store { mode } => match mode {
                StoreMode::Local => format!("store {}", interner.resolve(self.name)),
                StoreMode::Persisted { key } => format!(
                    "store {} persisted as {}",
                    interner.resolve(self.name),
                    expr_shape(key, interner)
                ),
                StoreMode::Shared { topic } => format!(
                    "store {} shared on {}",
                    interner.resolve(self.name),
                    expr_shape(topic, interner)
                ),
                StoreMode::Preserved => format!(
                    "store {} preserved across reloads",
                    interner.resolve(self.name)
                ),
            },
            ComponentKind::ErrorPage { status } => format!("error page {status}"),
        };
        let mut parts = vec![head];
        if !self.uses.is_empty() {
            let u: Vec<&str> = self.uses.iter().map(|s| interner.resolve(*s)).collect();
            parts.push(format!("uses[{}]", u.join(", ")));
        }
        if !self.state.is_empty() {
            let fs: Vec<String> = self
                .state
                .iter()
                .map(|f| {
                    let ty =
                        f.ty.map(|t| format!(": {}", type_shape(t, interner)))
                            .unwrap_or_default();
                    format!(
                        "{}{} {} {}",
                        interner.resolve(f.name),
                        ty,
                        if f.derived { ":=" } else { "=" },
                        expr_shape(f.init, interner)
                    )
                })
                .collect();
            parts.push(format!("state{{{}}}", fs.join("; ")));
        }
        if !self.events.is_empty() {
            let es: Vec<String> = self
                .events
                .iter()
                .map(|e| {
                    format!(
                        "{}: {}",
                        e.trigger.shape(interner),
                        Self::authored_len(e.body)
                    )
                })
                .collect();
            parts.push(format!("events{{{}}}", es.join("; ")));
        }
        if !self.actions.is_empty() {
            let as_: Vec<String> = self
                .actions
                .iter()
                .map(|a| {
                    format!(
                        "{}{}: {}",
                        interner.resolve(a.name),
                        params_shape(&a.params, interner),
                        Self::authored_len(a.body)
                    )
                })
                .collect();
            parts.push(format!("actions{{{}}}", as_.join("; ")));
        }
        if let Some(v) = &self.view {
            parts.push(format!("view{}", v.body_shape(interner)));
        }
        if self.style.is_some() {
            parts.push("style".into());
        }
        parts.join(" ")
    }
}
