//! Which parts of a page its `## Test`s DRIVE — read from the parsed page, never from its text.
//!
//! A page declares things a visitor can set in motion: actions, `on-*` handlers on elements,
//! English triggers (`When the page loads:`), and `bind`s. A test drives them through its steps:
//! `Click`, `Submit`, `Type … into`, `Choose … in`, `Press`, `Advance time`, `Replay`, `Raise`,
//! `Publish`, `Reload the page.`, and the page mounting itself. This module computes, per test,
//! which declared items those steps reach, and reports every item no test reaches.
//!
//! The steps are read in the order the runner executes them: `page_host::arrange` — the runner's
//! own arrangement — places the page's FIRST mount after the leading scene the test opens with,
//! and every `Reload the page.` step is itself the page mounting again (`Drive::Reload` re-runs
//! `## Main` and re-mounts the self-mounting page — `arrange` places nothing extra for it), so a
//! mount happens at the arrival point AND at every `Reload` step; each mount is where the
//! load-time triggers run. A request is answered by the answers arranged when it is sent; an
//! answer outside 2xx is a failed fetch, which the sending component's `When a fetch fails` hears
//! after the continuation has its `Err`.
//!
//! The element model is static: the page's view tree with every component use inlined at the
//! place it renders (props that pass an action carried through, `{children}` rendered in the
//! caller's scope), each element described by its tag, its `id` (literal, a pattern with holes,
//! or dynamic), its possible classes and its literal attributes. A step's selector is matched
//! against that model — a compound selector and its descendant/child chain — so `#c1` reaches
//! `<button id="c{i}">` and `.del` reaches the delete button of every row.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use logicaffeine_language::ast::component::{ComponentDef, ComponentKind, Trigger};
use logicaffeine_language::ast::stmt::{Expr, Literal, Stmt, StringPart, UiStmt};
use logicaffeine_language::ast::view::{Attr, ViewNode};
use logicaffeine_language::lgx::html::EventModifier;
use logicaffeine_language::Interner;

/// What kind of thing a page declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Action,
    Handler,
    Trigger,
    Bind,
}

/// One declared item and whether some test step drives it.
#[derive(Debug, Clone)]
pub struct Item {
    pub kind: Kind,
    /// How a reader finds it: `action `reset` (Page /)`, `on-click on <button#again>`, …
    pub label: String,
    pub driven: bool,
}

/// A page's coverage: its tests, and every item it declares.
#[derive(Debug, Clone)]
pub struct Coverage {
    pub tests: usize,
    pub items: Vec<Item>,
    /// The file holds an `## Error page` and no `## Page`: it has no URL, and a test runner does not
    /// mount it — what it shows is checked where it is served.
    pub error_page_only: bool,
}

impl Coverage {
    /// The items no test drives.
    pub fn undriven(&self) -> Vec<&Item> {
        self.items.iter().filter(|i| !i.driven).collect()
    }
}

/// How an element's `id` is written.
#[derive(Debug, Clone)]
enum Id {
    None,
    /// `id="again"`.
    Exact(String),
    /// `id="c{i}"` — literal pieces around holes that match any text.
    Pattern(Vec<Option<String>>),
    /// `id={someName}` — anything.
    Dynamic,
}

/// One element of the static model.
#[derive(Debug, Clone)]
struct Element {
    tag: String,
    id: Id,
    /// Every class the element can carry: its literal `class` tokens and every `class:name`.
    classes: BTreeSet<String>,
    /// Whether its `class` attribute is computed, so it may carry any class.
    dynamic_class: bool,
    /// Literal attributes, lowercased names (`type`, `name`, …); `None` = computed.
    attrs: BTreeMap<String, Option<String>>,
    /// Indices of its ancestors in the model, outermost first.
    ancestors: Vec<usize>,
    /// `(event, the key it waits for, handler index)` — the key as a browser reports it, from the
    /// language's own modifier catalogue; `None` answers every key.
    handlers: Vec<(String, Option<String>, usize)>,
    /// The bound field, with its item index.
    bind: Option<usize>,
}

/// A handler's calls resolved to action item indices.
#[derive(Debug, Clone, Default)]
struct Reach {
    actions: BTreeSet<usize>,
}

struct Model {
    elements: Vec<Element>,
    items: Vec<Item>,
    /// Item index → the action item indices it sets in motion when driven.
    reaches: BTreeMap<usize, Reach>,
    /// Action item → its own body's calls, fetch continuations and written fields.
    action_facts: BTreeMap<usize, ActionFacts>,
    /// Trigger item index → what fires it.
    triggers: Vec<(usize, Fires)>,
}

#[derive(Debug, Clone, Default)]
struct ActionFacts {
    /// The component whose instance runs the body — the one that hears its requests fail.
    component: String,
    calls: BTreeSet<usize>,
    /// `(method, url pattern, continuation action)`.
    fetches: Vec<(String, Vec<Option<String>>, Option<usize>)>,
    writes: BTreeSet<String>,
    /// The events the body delivers to the page itself: `Publish … on "<topic>"` is heard by the
    /// page's own `When a message arrives on "<topic>"` as `message:<topic>`.
    raises: BTreeSet<String>,
}

/// What fires a trigger.
#[derive(Debug, Clone)]
enum Fires {
    Mount,
    Time,
    Key(String),
    Selector {
        selector: String,
        event: String,
        key: Option<String>,
    },
    FieldChange(String),
    RouteChange,
    /// An event delivered by name — by `Raise "<event>"`, or by a `Publish` for `message:<topic>` —
    /// to the node the trigger listens on: its own selector, or the page (`"body"`) when `None`.
    Raised {
        event: String,
        on: Option<String>,
    },
    /// A request the named component sent was answered outside 2xx.
    FetchFailure {
        component: String,
    },
    /// No test verb delivers it; named so the report says why.
    Nothing(&'static str),
}

/// The page, as a raise names it: every page-level listener is registered on `"body"`, and
/// `Raise "<event>".` without `on` is delivered there.
const PAGE_NODE: &str = "body";

/// Compute the coverage of the page at `path`: its assembled closure, parsed.
pub fn page_coverage(path: &Path) -> Result<Coverage, String> {
    let assembled = logicaffeine_compile::analysis::assemble_project(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    logicaffeine_compile::ui_bridge::with_parsed_program(&assembled.source, |parsed, interner| {
        let (stmts, _types, _policies) =
            parsed.map_err(|e| format!("{} does not parse: {e}", path.display()))?;
        Ok(coverage_of(stmts, interner))
    })
}

fn coverage_of(stmts: &[Stmt<'_>], interner: &Interner) -> Coverage {
    let components: BTreeMap<String, &ComponentDef<'_>> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::ComponentDef(def) => Some((interner.resolve(def.name).to_string(), def.as_ref())),
            _ => None,
        })
        .collect();
    // What the file mounts: its pages (a `## Page` mounts itself), and any component a `Mount`
    // sentence names.
    let mut mounted: BTreeSet<String> = BTreeSet::new();
    for_each_stmt(stmts, &mut |stmt| {
        if let Stmt::Ui(UiStmt::Mount { component, .. }) = stmt {
            mounted.insert(interner.resolve(*component).to_string());
        }
    });
    let roots: Vec<&ComponentDef<'_>> = components
        .iter()
        .filter(|(name, d)| {
            matches!(
                d.kind,
                ComponentKind::Page { .. } | ComponentKind::ErrorPage { .. }
            ) || mounted.contains(name.as_str())
        })
        .map(|(_, d)| *d)
        .collect();

    let mut model = Model {
        elements: Vec::new(),
        items: Vec::new(),
        reaches: BTreeMap::new(),
        action_facts: BTreeMap::new(),
        triggers: Vec::new(),
    };
    // The components the page renders, each declared once: `(name → action name → item)`.
    let mut declared: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for root in &roots {
        let name = interner.resolve(root.name).to_string();
        declare(&mut model, &components, &name, interner, &mut declared);
        let env = Env {
            component: name.clone(),
            label: owner(root, interner),
            props: BTreeMap::new(),
        };
        if let Some(view) = &root.view {
            walk_view(
                &mut model,
                &components,
                &declared,
                &view.nodes,
                &env,
                &[],
                None,
                interner,
                0,
            );
        }
    }

    let tests: Vec<&[Stmt<'_>]> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::TestDef { body, .. } => Some(*body),
            _ => None,
        })
        .collect();
    // The program every test runs against — everything but the tests, as the runner builds it.
    let defs: Vec<Stmt<'_>> = stmts
        .iter()
        .filter(|s| !matches!(s, Stmt::TestDef { .. }))
        .cloned()
        .collect();
    let mut driven: BTreeSet<usize> = BTreeSet::new();
    for body in &tests {
        driven.extend(drive(&model, &defs, body, interner, &declared));
    }
    let mut items = model.items;
    for i in &driven {
        items[*i].driven = true;
    }
    let error_page_only = components
        .values()
        .any(|d| matches!(d.kind, ComponentKind::ErrorPage { .. }))
        && !components
            .values()
            .any(|d| matches!(d.kind, ComponentKind::Page { .. }));
    Coverage {
        tests: tests.len(),
        items,
        error_page_only,
    }
}

/// Declare a component's actions and triggers as items, and recurse into the components it
/// renders — once each.
fn declare(
    model: &mut Model,
    components: &BTreeMap<String, &ComponentDef<'_>>,
    name: &str,
    interner: &Interner,
    declared: &mut BTreeMap<String, BTreeMap<String, usize>>,
) {
    if declared.contains_key(name) {
        return;
    }
    let Some(def) = components.get(name) else {
        return;
    };
    let mut actions = BTreeMap::new();
    for action in &def.actions {
        let a = interner.resolve(action.name).to_string();
        let i = push(
            model,
            Kind::Action,
            format!("action `{a}` ({})", owner(def, interner)),
        );
        actions.insert(a, i);
    }
    declared.insert(name.to_string(), actions.clone());
    for action in &def.actions {
        let i = actions[interner.resolve(action.name)];
        let facts = body_facts(name, action.body, &actions, interner);
        model.action_facts.insert(i, facts);
    }
    for handler in &def.events {
        let (label, fires) = trigger(&handler.trigger, name, interner);
        let why = match &fires {
            Fires::Nothing(why) => format!(" — {why}"),
            _ => String::new(),
        };
        let i = push(
            model,
            Kind::Trigger,
            format!("trigger `{label}` ({}){why}", owner(def, interner)),
        );
        let facts = body_facts(name, handler.body, &actions, interner);
        model.reaches.insert(
            i,
            Reach {
                actions: facts.calls.clone(),
            },
        );
        model.action_facts.insert(i, facts);
        model.triggers.push((i, fires));
    }
    if let Some(view) = &def.view {
        let mut used = BTreeSet::new();
        components_used(&view.nodes, interner, &mut used);
        for c in used {
            declare(model, components, &c, interner, declared);
        }
    }
}

fn owner(def: &ComponentDef<'_>, interner: &Interner) -> String {
    match &def.kind {
        ComponentKind::Page { path, .. } => format!("## Page {}", interner.resolve(*path)),
        ComponentKind::ErrorPage { status } => format!("## Error page {status}"),
        ComponentKind::Store { .. } => format!("## Store {}", interner.resolve(def.name)),
        ComponentKind::Component => format!("## Component {}", interner.resolve(def.name)),
    }
}

fn push(model: &mut Model, kind: Kind, label: String) -> usize {
    model.items.push(Item {
        kind,
        label,
        driven: false,
    });
    model.items.len() - 1
}

fn components_used(nodes: &[ViewNode<'_>], interner: &Interner, out: &mut BTreeSet<String>) {
    for node in nodes {
        match node {
            ViewNode::Component { name, children, .. } => {
                out.insert(interner.resolve(*name).to_string());
                components_used(children, interner, out);
            }
            other => {
                for children in child_lists(other) {
                    components_used(children, interner, out);
                }
            }
        }
    }
}

/// Every child list a node renders, across all of its branches.
fn child_lists<'n, 'a>(node: &'n ViewNode<'a>) -> Vec<&'n [ViewNode<'a>]> {
    match node {
        ViewNode::Element { children, .. } | ViewNode::Component { children, .. } => vec![children],
        ViewNode::Each { body, empty, .. } => vec![body, empty],
        ViewNode::If {
            arms, otherwise, ..
        } => {
            let mut out: Vec<&[ViewNode<'a>]> = arms.iter().map(|(_, b)| b.as_slice()).collect();
            if let Some(o) = otherwise {
                out.push(o);
            }
            out
        }
        ViewNode::Inspect { arms, .. } => arms.iter().map(|a| a.body.as_slice()).collect(),
        ViewNode::Block {
            default: Some(d), ..
        } => vec![d],
        _ => Vec::new(),
    }
}

/// The scope a view is walked in: whose actions a bare name means, and which props carry an
/// action from the caller.
#[derive(Clone)]
struct Env {
    component: String,
    /// How a report names the component: `## Page /contact`, `## Component Row`.
    label: String,
    /// Prop name → the caller's action item it was given.
    props: BTreeMap<String, usize>,
}

impl Env {
    fn action(
        &self,
        name: &str,
        declared: &BTreeMap<String, BTreeMap<String, usize>>,
    ) -> Option<usize> {
        declared
            .get(&self.component)
            .and_then(|a| a.get(name).copied())
            .or_else(|| self.props.get(name).copied())
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_view<'a>(
    model: &mut Model,
    components: &BTreeMap<String, &ComponentDef<'a>>,
    declared: &BTreeMap<String, BTreeMap<String, usize>>,
    nodes: &[ViewNode<'a>],
    env: &Env,
    ancestors: &[usize],
    children: Option<(&[ViewNode<'a>], &Env)>,
    interner: &Interner,
    depth: usize,
) {
    if depth > 32 {
        return;
    }
    for node in nodes {
        match node {
            ViewNode::Element {
                tag,
                attrs,
                children: kids,
                ..
            } => {
                let index = model.elements.len();
                let mut el = Element {
                    tag: interner.resolve(*tag).to_ascii_lowercase(),
                    id: Id::None,
                    classes: BTreeSet::new(),
                    dynamic_class: false,
                    attrs: BTreeMap::new(),
                    ancestors: ancestors.to_vec(),
                    handlers: Vec::new(),
                    bind: None,
                };
                for attr in attrs {
                    match attr {
                        Attr::Plain { name, value } => {
                            let n = interner.resolve(*name).to_ascii_lowercase();
                            let lit = literal_text(value, interner);
                            match n.as_str() {
                                "id" => {
                                    el.id = match (&lit, pattern_of(value, interner)) {
                                        (Some(t), _) => Id::Exact(t.clone()),
                                        (None, Some(p)) => Id::Pattern(p),
                                        (None, None) => Id::Dynamic,
                                    }
                                }
                                "class" => match &lit {
                                    Some(t) => {
                                        el.classes.extend(t.split_whitespace().map(str::to_string))
                                    }
                                    None => el.dynamic_class = true,
                                },
                                _ => {}
                            }
                            el.attrs.insert(n, lit);
                        }
                        Attr::Class { name, .. } => {
                            el.classes.insert(interner.resolve(*name).to_string());
                        }
                        Attr::Bind { field } => {
                            let f = interner.resolve(*field).to_string();
                            let i = push(
                                model,
                                Kind::Bind,
                                format!("bind `{f}` on {} ({})", describe(&el), env.label),
                            );
                            el.bind = Some(i);
                        }
                        Attr::Directive { .. } => {}
                    }
                }
                for kid in kids {
                    if let ViewNode::Handler {
                        event,
                        modifiers,
                        body,
                        ..
                    } = kid
                    {
                        let ev = interner.resolve(*event).to_ascii_lowercase();
                        let key = EventModifier::key_of(modifiers);
                        let i = push(
                            model,
                            Kind::Handler,
                            format!("on-{ev} on {} ({})", describe(&el), env.label),
                        );
                        let mut reach = Reach::default();
                        for_each_call(
                            body,
                            &mut |name| {
                                if let Some(a) = env.action(name, declared) {
                                    reach.actions.insert(a);
                                }
                            },
                            interner,
                        );
                        model.reaches.insert(i, reach);
                        el.handlers.push((ev, key, i));
                    }
                }
                model.elements.push(el);
                let mut inner = ancestors.to_vec();
                inner.push(index);
                walk_view(
                    model,
                    components,
                    declared,
                    kids,
                    env,
                    &inner,
                    children,
                    interner,
                    depth + 1,
                );
            }
            ViewNode::Component {
                name,
                props,
                children: kids,
                ..
            } => {
                let cname = interner.resolve(*name).to_string();
                let Some(def) = components.get(&cname) else {
                    continue;
                };
                let mut given = BTreeMap::new();
                for (prop, value) in props {
                    if let Expr::Identifier(sym) = value {
                        if let Some(a) = env.action(interner.resolve(*sym), declared) {
                            given.insert(interner.resolve(*prop).to_string(), a);
                        }
                    }
                }
                let inner_env = Env {
                    component: cname.clone(),
                    label: owner(def, interner),
                    props: given,
                };
                if let Some(view) = &def.view {
                    walk_view(
                        model,
                        components,
                        declared,
                        &view.nodes,
                        &inner_env,
                        ancestors,
                        Some((kids.as_slice(), env)),
                        interner,
                        depth + 1,
                    );
                }
            }
            ViewNode::Children { .. } => {
                if let Some((kids, caller)) = children {
                    walk_view(
                        model,
                        components,
                        declared,
                        kids,
                        caller,
                        ancestors,
                        None,
                        interner,
                        depth + 1,
                    );
                }
            }
            other => {
                for list in child_lists(other) {
                    walk_view(
                        model,
                        components,
                        declared,
                        list,
                        env,
                        ancestors,
                        children,
                        interner,
                        depth + 1,
                    );
                }
            }
        }
    }
}

fn describe(el: &Element) -> String {
    let mut s = format!("<{}", el.tag);
    match &el.id {
        Id::Exact(id) => s.push_str(&format!("#{id}")),
        Id::Pattern(parts) => {
            s.push('#');
            for p in parts {
                s.push_str(p.as_deref().unwrap_or("{…}"));
            }
        }
        _ => {}
    }
    for c in &el.classes {
        s.push_str(&format!(".{c}"));
    }
    s.push('>');
    s
}

fn literal_text(e: &Expr<'_>, interner: &Interner) -> Option<String> {
    match e {
        Expr::Literal(Literal::Text(t)) => Some(interner.resolve(*t).to_string()),
        _ => None,
    }
}

/// An interpolated string as literal pieces and holes.
fn pattern_of(e: &Expr<'_>, interner: &Interner) -> Option<Vec<Option<String>>> {
    match e {
        Expr::InterpolatedString(parts) => Some(
            parts
                .iter()
                .map(|p| match p {
                    StringPart::Literal(t) => Some(interner.resolve(*t).to_string()),
                    _ => None,
                })
                .collect(),
        ),
        Expr::Literal(Literal::Text(t)) => Some(vec![Some(interner.resolve(*t).to_string())]),
        _ => None,
    }
}

/// Does literal text fit a pattern of literal pieces and holes?
fn fits(pattern: &[Option<String>], text: &str) -> bool {
    fn go(p: &[Option<String>], t: &str) -> bool {
        match p.split_first() {
            None => t.is_empty(),
            Some((Some(lit), rest)) => t.strip_prefix(lit.as_str()).is_some_and(|r| go(rest, r)),
            Some((None, rest)) => (0..=t.len())
                .filter(|i| t.is_char_boundary(*i))
                .any(|i| go(rest, &t[i..])),
        }
    }
    go(pattern, text)
}

/// Visit every statement in a block and its nested blocks.
fn for_each_stmt<'b, 'a>(block: &'b [Stmt<'a>], f: &mut dyn FnMut(&'b Stmt<'a>)) {
    for stmt in block {
        f(stmt);
        match stmt {
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                for_each_stmt(then_block, f);
                if let Some(e) = else_block {
                    for_each_stmt(e, f);
                }
            }
            Stmt::While { body, .. }
            | Stmt::Repeat { body, .. }
            | Stmt::Zone { body, .. }
            | Stmt::Reversibly { body }
            | Stmt::ExpectFail { body, .. } => for_each_stmt(body, f),
            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    for_each_stmt(arm.body, f);
                }
            }
            _ => {}
        }
    }
}

/// Every function name a block calls, as a statement or inside an expression it evaluates.
fn for_each_call(block: &[Stmt<'_>], f: &mut dyn FnMut(&str), interner: &Interner) {
    for_each_stmt(block, &mut |stmt| match stmt {
        Stmt::Call { function, .. } => f(interner.resolve(*function)),
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => expr_calls(value, f, interner),
        _ => {}
    });
}

fn expr_calls(e: &Expr<'_>, f: &mut dyn FnMut(&str), interner: &Interner) {
    if let Expr::Call { function, args } = e {
        f(interner.resolve(*function));
        for a in args {
            expr_calls(a, f, interner);
        }
    }
}

/// An action or trigger body's calls to actions, its fetches, the fields it writes and the events
/// it delivers to its own page.
fn body_facts(
    component: &str,
    block: &[Stmt<'_>],
    actions: &BTreeMap<String, usize>,
    interner: &Interner,
) -> ActionFacts {
    let mut facts = ActionFacts {
        component: component.to_string(),
        ..ActionFacts::default()
    };
    for_each_call(
        block,
        &mut |name| {
            if let Some(a) = actions.get(name) {
                facts.calls.insert(*a);
            }
        },
        interner,
    );
    for_each_stmt(block, &mut |stmt| match stmt {
        Stmt::Fetch {
            method, url, then, ..
        } => {
            let m = format!("{method:?}").to_ascii_uppercase();
            let pattern = pattern_of(url, interner).unwrap_or_else(|| vec![None]);
            facts
                .fetches
                .push((m, pattern, actions.get(interner.resolve(*then)).copied()));
        }
        Stmt::Replay { actions: names } => {
            for n in names {
                if let Some(a) = actions.get(interner.resolve(*n)) {
                    facts.calls.insert(*a);
                }
            }
        }
        Stmt::Set { target, .. } => {
            facts.writes.insert(interner.resolve(*target).to_string());
        }
        Stmt::Push { collection, .. }
        | Stmt::Add { collection, .. }
        | Stmt::Remove { collection, .. }
        | Stmt::SetIndex { collection, .. } => {
            if let Expr::Identifier(sym) = collection {
                facts.writes.insert(interner.resolve(*sym).to_string());
            }
        }
        Stmt::Ui(UiStmt::Toggle { field }) => {
            facts.writes.insert(interner.resolve(*field).to_string());
        }
        Stmt::Ui(UiStmt::Publish { topic, .. }) => {
            if let Some(t) = literal_text(topic, interner) {
                facts.raises.insert(message_event(&t));
            }
        }
        _ => {}
    });
    facts
}

/// The event a message on `topic` is delivered to the page as.
fn message_event(topic: &str) -> String {
    format!("message:{topic}")
}

/// A trigger's sentence and what fires it. `component` is the one that declares it.
fn trigger(t: &Trigger<'_>, component: &str, interner: &Interner) -> (String, Fires) {
    const COMPUTED: &str = "its node is computed, so no step can be matched to it";
    match t {
        Trigger::PageLoads => ("When the page loads".into(), Fires::Mount),
        Trigger::Appears => ("When this component appears".into(), Fires::Mount),
        Trigger::Every { .. } => ("Every …".into(), Fires::Time),
        Trigger::After { .. } => ("After …".into(), Fires::Time),
        Trigger::Key { key } => {
            let k = interner.resolve(*key).to_string();
            (format!("When the user presses {k}"), Fires::Key(k))
        }
        Trigger::Selector {
            selector,
            event,
            key,
        } => {
            let sel = literal_text(selector, interner).unwrap_or_default();
            let ev = interner.resolve(*event).to_ascii_lowercase();
            (
                format!("When \"{sel}\" {ev}"),
                Fires::Selector {
                    selector: sel,
                    event: ev,
                    key: key.map(|k| interner.resolve(k).to_string()),
                },
            )
        }
        Trigger::FieldChanges { field, .. } => {
            let f = interner.resolve(*field).to_string();
            (format!("When {f} changes"), Fires::FieldChange(f))
        }
        Trigger::RouteChanges => ("When the route changes".into(), Fires::RouteChange),
        Trigger::Disappears => (
            "When this component disappears".into(),
            Fires::Nothing("no test step removes a component"),
        ),
        Trigger::Online => (
            "When the app comes back online".into(),
            page_event("online"),
        ),
        Trigger::Offline => ("When the app goes offline".into(), page_event("offline")),
        Trigger::WindowResizes => ("When the window resizes".into(), page_event("resize")),
        Trigger::Message { topic, .. } => match literal_text(topic, interner) {
            Some(t) => (
                format!("When a message arrives on \"{t}\""),
                page_event(&message_event(&t)),
            ),
            None => (
                "When a message arrives".into(),
                Fires::Nothing("its topic is computed, so no step can be matched to it"),
            ),
        },
        Trigger::ActionFails { .. } => (
            "When an action fails".into(),
            Fires::Nothing("failure is not a test step"),
        ),
        Trigger::FetchFails { .. } => (
            "When a fetch fails".into(),
            Fires::FetchFailure {
                component: component.to_string(),
            },
        ),
        Trigger::ScrollsToBottom { selector: None } => (
            "When the user scrolls to the bottom".into(),
            page_event("scrolledtobottom"),
        ),
        Trigger::ScrollsToBottom {
            selector: Some(sel),
        } => match literal_text(sel, interner) {
            Some(s) => (
                format!("When the user scrolls to the bottom of \"{s}\""),
                Fires::Raised {
                    event: "scrolledtobottom".into(),
                    on: Some(s),
                },
            ),
            None => (
                "When the user scrolls to the bottom of …".into(),
                Fires::Nothing(COMPUTED),
            ),
        },
        Trigger::FileDropped { selector, .. } => match literal_text(selector, interner) {
            Some(s) => (
                format!("When a file is dropped on \"{s}\""),
                Fires::Raised {
                    event: "filedropped".into(),
                    on: Some(s),
                },
            ),
            None => (
                "When a file is dropped on …".into(),
                Fires::Nothing(COMPUTED),
            ),
        },
    }
}

/// A trigger that hears `event` delivered to the page.
fn page_event(event: &str) -> Fires {
    Fires::Raised {
        event: event.to_string(),
        on: None,
    }
}

/// One compound of a selector: `tag#id.class[attr=value]`.
#[derive(Debug, Default)]
struct Compound {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attrs: Vec<(String, Option<String>)>,
}

/// A selector as its compounds, each with whether it must be the CHILD of the one before
/// (`>`) rather than any descendant. `None` for syntax this model does not read.
fn parse_selector(sel: &str) -> Option<Vec<(Compound, bool)>> {
    let spaced = sel.replace('>', " > ");
    let mut out: Vec<(Compound, bool)> = Vec::new();
    let mut child = false;
    for word in spaced.split_whitespace() {
        if word == ">" {
            child = true;
            continue;
        }
        let mut c = Compound::default();
        let mut rest = word;
        let take = |s: &str| -> usize {
            s.find(|ch: char| matches!(ch, '#' | '.' | '[' | ':'))
                .unwrap_or(s.len())
        };
        let n = take(rest);
        if n > 0 {
            if rest[..n] != *"*" {
                c.tag = Some(rest[..n].to_ascii_lowercase());
            }
            rest = &rest[n..];
        }
        while let Some(ch) = rest.chars().next() {
            match ch {
                '#' | '.' => {
                    let body = &rest[1..];
                    let n = take(body);
                    if ch == '#' {
                        c.id = Some(body[..n].to_string());
                    } else {
                        c.classes.push(body[..n].to_string());
                    }
                    rest = &body[n..];
                }
                '[' => {
                    let close = rest.find(']')?;
                    let inner = &rest[1..close];
                    match inner.split_once('=') {
                        Some((k, v)) => c.attrs.push((
                            k.trim().to_ascii_lowercase(),
                            Some(v.trim().trim_matches('"').trim_matches('\'').to_string()),
                        )),
                        None => c.attrs.push((inner.trim().to_ascii_lowercase(), None)),
                    }
                    rest = &rest[close + 1..];
                }
                ':' => {
                    // A pseudo-class narrows which of the matched elements it means; the model
                    // keeps the element, which over-approximates only among lookalikes.
                    let body = &rest[1..];
                    let n = take(body);
                    let mut end = n;
                    if body[..n].contains('(') || body.get(n..).is_some_and(|r| r.starts_with('('))
                    {
                        end = body.find(')').map(|i| i + 1)?;
                    }
                    rest = &body[end..];
                }
                _ => return None,
            }
        }
        out.push((c, child));
        child = false;
    }
    (!out.is_empty()).then_some(out)
}

fn compound_fits(c: &Compound, el: &Element) -> bool {
    if c.tag.as_ref().is_some_and(|t| *t != el.tag) {
        return false;
    }
    if let Some(want) = &c.id {
        let ok = match &el.id {
            Id::None => false,
            Id::Exact(id) => id == want,
            Id::Pattern(p) => fits(p, want),
            Id::Dynamic => true,
        };
        if !ok {
            return false;
        }
    }
    if !c
        .classes
        .iter()
        .all(|k| el.dynamic_class || el.classes.contains(k))
    {
        return false;
    }
    c.attrs.iter().all(|(k, v)| match (el.attrs.get(k), v) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(None), Some(_)) => true,
        (Some(Some(have)), Some(want)) => have == want,
    })
}

/// The elements a selector matches in the model.
fn matching(model: &Model, sel: &str) -> Vec<usize> {
    let Some(chain) = parse_selector(sel) else {
        return Vec::new();
    };
    (0..model.elements.len())
        .filter(|&i| {
            let el = &model.elements[i];
            let (last, _) = chain.last().expect("non-empty");
            if !compound_fits(last, el) {
                return false;
            }
            // Walk the rest of the chain outward through the ancestors.
            let mut remaining = &el.ancestors[..];
            let mut must_be_parent = chain.last().map(|(_, child)| *child).unwrap_or(false);
            for (c, child) in chain[..chain.len() - 1].iter().rev() {
                let found = if must_be_parent {
                    remaining
                        .last()
                        .filter(|a| compound_fits(c, &model.elements[**a]))
                        .map(|_| remaining.len() - 1)
                } else {
                    remaining
                        .iter()
                        .rposition(|a| compound_fits(c, &model.elements[*a]))
                };
                match found {
                    Some(at) => remaining = &remaining[..at],
                    None => return false,
                }
                must_be_parent = *child;
            }
            true
        })
        .collect()
}

fn text_arg(e: &Expr<'_>, interner: &Interner) -> Option<String> {
    literal_text(e, interner)
}

/// The DOM events a test verb raises on the element it names.
fn events_of(verb: &UiStmt<'_>) -> &'static [&'static str] {
    match verb {
        UiStmt::Click { .. } => &["click"],
        UiStmt::Submit { .. } => &["submit"],
        UiStmt::TypeInto { .. } => &["input", "change", "keydown", "keyup"],
        UiStmt::Choose { .. } => &["change", "input"],
        UiStmt::Press { .. } => &["keydown", "keyup", "keypress"],
        UiStmt::Focus { .. } => &["focus", "focusin"],
        UiStmt::Blur { .. } => &["blur", "focusout"],
        UiStmt::Hover { .. } => &["mouseenter", "mouseover"],
        _ => &[],
    }
}

/// Whether a pressed key is the one a handler or trigger waits for — compared as a browser's
/// `KeyboardEvent.key`, case-insensitively, as the catalogue compares them.
fn same_key(pressed: &str, wanted: &str) -> bool {
    pressed.eq_ignore_ascii_case(wanted)
}

/// The items one test drives, stepping through it in order — the way the test runner executes
/// it: `page_host::arrange` places the page's first mount after the leading scene the test opens
/// with, and each `Reload the page.` step is read as a mount in its own right (every tier's own
/// `Drive::Reload` re-mounts the page itself); each mount runs the load-time triggers. A fetch is
/// answered only by a `Given that … answers …` already arranged when the fetch is sent (or by
/// `Serve fetches with routes.`), because that is when the reference tier answers it.
fn drive(
    model: &Model,
    defs: &[Stmt<'_>],
    body: &[Stmt<'_>],
    interner: &Interner,
    declared: &BTreeMap<String, BTreeMap<String, usize>>,
) -> BTreeSet<usize> {
    let mut run = Run {
        model,
        fired: BTreeSet::new(),
        stubs: BTreeMap::new(),
        serve_fetches: false,
    };
    let arranged = logicaffeine_compile::page_host::arrange(defs, body);
    // A component the program mounts for itself — a `Mount` sentence that is not a page's own —
    // is on screen before the test's first statement.
    if arranged
        .defs
        .iter()
        .any(|s| matches!(s, Stmt::Ui(UiStmt::Mount { .. })))
    {
        run.mount();
    }

    let mut steps: Vec<&Stmt<'_>> = Vec::new();
    for_each_stmt(&arranged.body, &mut |stmt| steps.push(stmt));
    for stmt in steps {
        match stmt {
            Stmt::Ui(UiStmt::Mount { .. }) => run.mount(),
            // `Reload the page.` is the page loading again, its own mount included —
            // `page_host::arrange` places a page's FIRST mount after the leading scene, but never
            // splices one in after a `Reload`: every tier's own `Drive::Reload` re-mounts the page
            // itself, so this reads that same meaning directly off the step rather than off
            // something `arrange` no longer emits.
            Stmt::Ui(UiStmt::Reload) => run.mount(),
            Stmt::GivenFetch {
                method,
                url,
                status,
                ..
            } => {
                if let Some(u) = text_arg(url, interner) {
                    let status = match status {
                        Expr::Literal(Literal::Number(n)) => Some(*n),
                        _ => None,
                    };
                    run.stubs
                        .insert((interner.resolve(*method).to_ascii_uppercase(), u), status);
                }
            }
            Stmt::ServeFetches => run.serve_fetches = true,
            Stmt::Ui(UiStmt::RaiseWindowEvent { event, target, .. }) => {
                if let (Some(e), Some(t)) = (text_arg(event, interner), text_arg(target, interner))
                {
                    let hit = raise_hits(model, &e, &t);
                    run.fire(hit);
                }
            }
            Stmt::Ui(UiStmt::Publish { topic, .. }) => {
                if let Some(t) = text_arg(topic, interner) {
                    let hit = raise_hits(model, &message_event(&t), PAGE_NODE);
                    run.fire(hit);
                }
            }
            Stmt::AdvanceTime { .. } => {
                let timers = model
                    .triggers
                    .iter()
                    .filter(|(_, f)| matches!(f, Fires::Time))
                    .map(|(i, _)| *i)
                    .collect();
                run.fire(timers);
            }
            Stmt::Replay { actions } => {
                let mut replayed = Vec::new();
                for a in actions {
                    for table in declared.values() {
                        if let Some(i) = table.get(interner.resolve(*a)) {
                            replayed.push(*i);
                        }
                    }
                }
                run.fire(replayed);
            }
            Stmt::Ui(verb) => {
                let hit = verb_hits(model, verb, interner);
                run.fire(hit);
            }
            _ => {}
        }
    }
    run.fired
}

/// A test in progress: what has been set in motion, and the answers arranged so far.
struct Run<'m> {
    model: &'m Model,
    fired: BTreeSet<usize>,
    /// `(METHOD, url) → status` — a later answer for the same request replaces the earlier one, as
    /// the runner's own stub table does. The status is `None` when it is computed.
    stubs: BTreeMap<(String, String), Option<i64>>,
    serve_fetches: bool,
}

impl Run<'_> {
    /// The page mounts: its load-time triggers run.
    fn mount(&mut self) {
        let mounted = self
            .model
            .triggers
            .iter()
            .filter(|(_, fires)| matches!(fires, Fires::Mount))
            .map(|(i, _)| *i)
            .collect();
        self.fire(mounted);
    }

    /// Set `items` in motion, and everything they reach: the actions they call, the actions those
    /// call, a fetch's continuation when an answer is already arranged — and the sending
    /// component's failure trigger when that answer is outside 2xx — the triggers on the fields
    /// they write, and the page's own triggers for the events they deliver to it — to a fixed
    /// point. Each call reads the scene as it stands at that step, so an item set in motion again
    /// after the answers changed reaches what the new answers reach.
    fn fire(&mut self, items: Vec<usize>) {
        let model = self.model;
        let mut pending = items;
        let mut seen: BTreeSet<usize> = BTreeSet::new();
        while let Some(i) = pending.pop() {
            if !seen.insert(i) {
                continue;
            }
            self.fired.insert(i);
            let mut written: BTreeSet<String> = BTreeSet::new();
            if let Some(r) = model.reaches.get(&i) {
                pending.extend(r.actions.iter().copied());
            }
            if let Some(f) = model.action_facts.get(&i) {
                pending.extend(f.calls.iter().copied());
                written.extend(f.writes.iter().cloned());
                for (method, pattern, then) in &f.fetches {
                    let answers: Vec<Option<i64>> = self
                        .stubs
                        .iter()
                        .filter(|((m, url), _)| m == method && fits(pattern, url))
                        .map(|(_, status)| *status)
                        .collect();
                    if let (true, Some(t)) = (self.serve_fetches || !answers.is_empty(), then) {
                        pending.push(*t);
                    }
                    let failed = answers.iter().flatten().any(|s| !(200..300).contains(s));
                    if failed {
                        pending.extend(model.triggers.iter().filter_map(
                            |(t, fires)| match fires {
                                Fires::FetchFailure { component } if *component == f.component => {
                                    Some(*t)
                                }
                                _ => None,
                            },
                        ));
                    }
                }
                for event in &f.raises {
                    pending.extend(raise_hits(model, event, PAGE_NODE));
                }
            }
            if model.items[i].kind == Kind::Bind {
                if let Some(f) = model.items[i].label.split('`').nth(1) {
                    written.insert(f.to_string());
                }
            }
            for (t, fires) in &model.triggers {
                if matches!(fires, Fires::FieldChange(f) if written.contains(f)) {
                    pending.push(*t);
                }
            }
        }
    }
}

/// What an event named `event`, delivered to the node `target` names, reaches: the listeners the
/// runner registered on that very node under that name (delivery does not bubble) — the triggers
/// that hear it there, a `When "<sel>" <event>` sentence on the same element, and the element's
/// own `on-<event>` handlers. The page's listeners sit on [`PAGE_NODE`].
fn raise_hits(model: &Model, event: &str, target: &str) -> Vec<usize> {
    let matched = matching(model, target);
    let same_node =
        |sel: &str| sel == target || matching(model, sel).iter().any(|e| matched.contains(e));
    let mut hits: Vec<usize> = model
        .triggers
        .iter()
        .filter(|(_, fires)| match fires {
            Fires::Raised { event: e, on: None } => e == event && target == PAGE_NODE,
            Fires::Raised {
                event: e,
                on: Some(sel),
            } => e == event && same_node(sel),
            Fires::Selector {
                selector,
                event: e,
                key: None,
            } => e == event && same_node(selector),
            _ => false,
        })
        .map(|(i, _)| *i)
        .collect();
    for &e in &matched {
        hits.extend(
            model.elements[e]
                .handlers
                .iter()
                .filter(|(ev, key, _)| ev == event && key.is_none())
                .map(|(_, _, h)| *h),
        );
    }
    hits
}

/// The handlers, binds, forms and triggers one driving verb reaches.
fn verb_hits(model: &Model, verb: &UiStmt<'_>, interner: &Interner) -> Vec<usize> {
    let mut hits: Vec<usize> = Vec::new();
    let events = events_of(verb);
    let (selector, key) = match verb {
        UiStmt::Click { selector }
        | UiStmt::Submit { selector }
        | UiStmt::Focus { selector }
        | UiStmt::Blur { selector }
        | UiStmt::Hover { selector }
        | UiStmt::TypeInto { selector, .. }
        | UiStmt::Choose { selector, .. } => (text_arg(selector, interner), None),
        UiStmt::Press { key, selector } => (
            selector.and_then(|s| text_arg(s, interner)),
            text_arg(key, interner),
        ),
        UiStmt::Navigate { .. } | UiStmt::GoBack | UiStmt::ReplaceUrl { .. } => {
            return model
                .triggers
                .iter()
                .filter(|(_, f)| matches!(f, Fires::RouteChange))
                .map(|(i, _)| *i)
                .collect();
        }
        _ => (None, None),
    };
    if let Some(k) = &key {
        hits.extend(
            model
                .triggers
                .iter()
                .filter(|(_, f)| matches!(f, Fires::Key(want) if same_key(k, want)))
                .map(|(i, _)| *i),
        );
    }
    let Some(sel) = selector else { return hits };
    if events.is_empty() {
        return hits;
    }
    let matched = matching(model, &sel);
    for &e in &matched {
        let el = &model.elements[e];
        for (ev, wanted, h) in &el.handlers {
            let key_ok = match (wanted, &key) {
                (None, _) => true,
                (Some(w), Some(p)) => same_key(p, w),
                (Some(_), None) => false,
            };
            if events.contains(&ev.as_str()) && key_ok {
                hits.push(*h);
            }
        }
        // A bound control is written by the verbs that change its value; a checkbox or a radio by
        // a click.
        if let Some(b) = el.bind {
            let toggles =
                matches!(el.attrs.get("type"), Some(Some(t)) if t == "checkbox" || t == "radio");
            if events.contains(&"input")
                || events.contains(&"change")
                || (toggles && events.contains(&"click"))
            {
                hits.push(b);
            }
        }
        // Clicking a submit button, or submitting a control inside a form, submits the form.
        let submits = events.contains(&"submit")
            || (events.contains(&"click")
                && el.tag == "button"
                && !matches!(el.attrs.get("type"), Some(Some(t)) if t == "button" || t == "reset"));
        if submits {
            if let Some(form) = el
                .ancestors
                .iter()
                .rev()
                .map(|a| &model.elements[*a])
                .find(|f| f.tag == "form")
            {
                hits.extend(
                    form.handlers
                        .iter()
                        .filter(|(ev, _, _)| ev == "submit")
                        .map(|(_, _, h)| *h),
                );
            }
        }
    }
    for (i, fires) in &model.triggers {
        if let Fires::Selector {
            selector: want,
            event,
            key: want_key,
        } = fires
        {
            let same = matching(model, want).iter().any(|w| matched.contains(w)) || *want == sel;
            let key_ok = match (want_key, &key) {
                (None, _) => true,
                (Some(w), Some(p)) => same_key(p, w),
                (Some(_), None) => false,
            };
            if same && events.contains(&event.as_str()) && key_ok {
                hits.push(*i);
            }
        }
    }
    hits
}
