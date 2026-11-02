//! The static dependency graph a fine-grained DOM update is compiled from.
//!
//! Stage A repaints a component by clearing its mount and rebuilding the subtree — correct, and
//! O(the whole view) for a change to one field. Stage B emits a PATCHER instead, and a patcher
//! needs three facts known before the program runs:
//!
//!   * every SITE in the view that can change, and which state fields it reads;
//!   * every ACTION, and which state fields it writes;
//!   * every DERIVED field, and what it is derived from.
//!
//! Then a turn that writes `draft` updates exactly the sites that read `draft`.
//!
//! ★ THE READ SET IS CLOSED, which is the whole difficulty and the whole value. The names a site
//! LITERALLY MENTIONS are usually not state:
//!
//!   * `{remaining}` where `remaining` is derived reads whatever the derivation reads — nothing
//!     ever writes `remaining` itself, so a literal read set would never update it;
//!   * `{t}` inside `{for each t in todos:}` reads `todos` — `t` is a window onto state, not
//!     state, and no action writes it;
//!   * `{doubled}` after `{let doubled be n * 2}` reads `n`.
//!
//! Every one of those failures is SILENT: the page simply stops updating, on exactly the
//! constructs real pages are built from. So resolution happens here, once, and the answer a site
//! carries is already in terms of the fields an action can write.
//!
//! ★ The write set is a static OVER-approximation on purpose. A write inside an `If` counts even
//! when the branch does not run: repainting a site that did not change is a wasted patch, while
//! missing one leaves a visitor looking at a stale page. The two errors are not comparable.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use logicaffeine_base::{Interner, Symbol};

use crate::ast::component::{ComponentDef, Trigger};
use crate::ast::stmt::{Block, ClosureBody, Expr, Pattern, Stmt, StringPart};
use crate::ast::view::{Attr, ViewNode};

/// Why a component has no dependency graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DepError {
    /// Derived fields that depend on each other, in the order the walk met them.
    ///
    /// `a always be b + 1` beside `b always be a + 1` has no fixed point: recomputing them gives
    /// an answer that depends on the order, and the turn semantics forbid a second round. Better
    /// refused where it is written than settled by whichever value happened to land last.
    DerivedCycle(Vec<String>),
}

/// What kind of thing a site is — which decides what the patcher emits for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SiteKind {
    /// A text child: `{draft}`.
    Text,
    /// An attribute value: `title={name}`.
    Attr(Symbol),
    /// A conditional class: `class:done={done}`.
    Class(Symbol),
    /// The condition set of an `{if}` — any of them changing may switch which arm renders.
    If,
    /// A loop's collection.
    Each,
    /// A two-way binding, which SHOWS its field as well as writing it.
    Bind(Symbol),
    /// An `{inspect}` scrutinee.
    Inspect,
}

/// One addressable place in a view that can change, and what it depends on.
#[derive(Debug, Clone)]
pub struct SiteDeps {
    /// Dotted child indices from the view root — the patcher's address for the node.
    pub path: String,
    pub kind: SiteKind,
    /// Does this site live under an `{if}`, a `{for each}` or an `{inspect}`?
    ///
    /// A site under control flow has no single node: a loop body renders once PER ITEM, and an
    /// arm renders only while its condition holds. Its static path therefore names a shape rather
    /// than a node, so an in-place patch cannot address it and the update falls back to
    /// rebuilding. Marked here, where the walk already knows, rather than re-derived from the
    /// path string — a path is not a reliable record of how it was produced.
    pub dynamic: bool,
    /// State fields, already closed over derived fields and template bindings.
    pub reads: BTreeSet<Symbol>,
    /// Props read here. Separate because a prop cannot change without the PARENT re-rendering
    /// this component, so it is not something this instance's own turns can invalidate.
    pub prop_reads: BTreeSet<Symbol>,
}

/// The whole graph for one component.
#[derive(Debug, Clone)]
pub struct ComponentDeps {
    /// Sites in document order.
    pub sites: Vec<SiteDeps>,
    /// Each derived field and the base state fields it is computed from.
    pub derived: BTreeMap<Symbol, BTreeSet<Symbol>>,
    /// Each turn (an action by name, an event handler by its trigger) and what it writes.
    pub writes: BTreeMap<String, BTreeSet<Symbol>>,
}

impl SiteDeps {
    /// `kind@path` — the site's identity, and the KEY a renderer records its node under.
    ///
    /// The dependency walk and the render walk are two walks over one tree, and two walks drift.
    /// Making them agree on a STRING means the agreement can be asserted directly rather than
    /// hoped for: a renderer that numbered a child differently records a key the graph never
    /// names, and the lock says so.
    pub fn key(&self, interner: &Interner) -> String {
        let name = |s: &Symbol| interner.resolve(*s).to_string();
        let kind = match &self.kind {
            SiteKind::Text => "text".to_string(),
            SiteKind::Attr(a) => format!("attr:{}", name(a)),
            SiteKind::Class(c) => format!("class:{}", name(c)),
            SiteKind::If => "if".to_string(),
            SiteKind::Each => "each".to_string(),
            SiteKind::Bind(f) => format!("bind:{}", name(f)),
            SiteKind::Inspect => "inspect".to_string(),
        };
        format!("{kind}@{}", self.path)
    }

    /// Can a turn update this site IN PLACE, without rebuilding anything?
    ///
    /// Only a value-shaped site under no control flow: it names exactly one node that exists for
    /// as long as the component is mounted. An `{if}` or a `{for each}` MOUNTS and UNMOUNTS
    /// subtrees, which is a different operation from writing a value into a node that is already
    /// there.
    pub fn patchable(&self) -> bool {
        // ★ ONE PREDICATE. Every walk must agree about which turns patch and which repaint, or
        // their op-logs diverge on the same page — and the op-log is the contract, not the
        // rendered tree. So the set is defined here once rather than decided twice.
        //
        // Text and a binding are values written into one node. An ATTRIBUTE and a CLASS TOGGLE
        // are not here yet: a class toggle recomposes the element's whole class list (`class=`
        // and every `class:x` compose into one attribute, so writing one would clobber its
        // neighbours), which the bytecode tier builds inline during the paint. Both tiers repaint
        // for those, which is correct and merely slower; widening this set widens both together.
        !self.dynamic && matches!(self.kind, SiteKind::Text | SiteKind::Bind(_))
    }
}

impl ComponentDeps {
    /// The sites a write to `field` invalidates.
    pub fn sites_reading(&self, field: Symbol) -> impl Iterator<Item = &SiteDeps> {
        self.sites.iter().filter(move |s| s.reads.contains(&field))
    }

    /// A deterministic line per fact: sites in document order, then the derived graph, then the
    /// write sets. Emitting from the maps' iteration order would make every downstream lock a
    /// coin flip and every generated patcher differ between builds of one source.
    pub fn describe(&self, interner: &Interner) -> Vec<String> {
        let name = |s: &Symbol| interner.resolve(*s).to_string();
        let list = |set: &BTreeSet<Symbol>| {
            let mut v: Vec<String> = set.iter().map(name).collect();
            v.sort();
            v
        };
        let mut out = Vec::new();
        for site in &self.sites {
            let key = site.key(interner);
            let mut parts: Vec<String> = Vec::new();
            if !site.reads.is_empty() {
                parts.push(list(&site.reads).join(", "));
            }
            if !site.prop_reads.is_empty() {
                parts.push(format!("props {}", list(&site.prop_reads).join(", ")));
            }
            let reads = if parts.is_empty() {
                "—".to_string()
            } else {
                parts.join(" ")
            };
            out.push(format!("{key} reads {reads}"));
        }
        for (field, from) in &self.derived {
            let from = if from.is_empty() {
                "—".to_string()
            } else {
                list(from).join(", ")
            };
            out.push(format!("derived {} <- {from}", name(field)));
        }
        for (turn, fields) in &self.writes {
            let f = if fields.is_empty() {
                "—".to_string()
            } else {
                list(fields).join(", ")
            };
            out.push(format!("writes {turn} -> {f}"));
        }
        out
    }
}

/// What a name resolves to once template bindings and derivations are followed through.
#[derive(Debug, Clone, Default)]
struct Resolved {
    state: BTreeSet<Symbol>,
    props: BTreeSet<Symbol>,
}

impl Resolved {
    fn merge(&mut self, other: &Resolved) {
        self.state.extend(other.state.iter().copied());
        self.props.extend(other.props.iter().copied());
    }
}

/// Build the dependency graph for one component.
pub fn component_deps(
    def: &ComponentDef<'_>,
    interner: &Interner,
) -> Result<ComponentDeps, DepError> {
    let plain: HashSet<Symbol> = def
        .state
        .iter()
        .filter(|f| !f.derived)
        .map(|f| f.name)
        .collect();
    let derived_fields: Vec<Symbol> = def
        .state
        .iter()
        .filter(|f| f.derived)
        .map(|f| f.name)
        .collect();
    let derived_set: HashSet<Symbol> = derived_fields.iter().copied().collect();
    let props: HashSet<Symbol> = def.props.iter().map(|(n, _)| *n).collect();

    // The DIRECT reads of each derivation: the state and derived names its initializer mentions.
    let mut direct: HashMap<Symbol, Vec<Symbol>> = HashMap::new();
    for field in def.state.iter().filter(|f| f.derived) {
        let mut names = Vec::new();
        collect_names(field.init, &mut names);
        let deps: Vec<Symbol> = names
            .into_iter()
            .filter(|n| plain.contains(n) || derived_set.contains(n))
            .collect();
        direct.insert(field.name, deps);
    }

    // A fixpoint over the derivations, with a cycle refused rather than settled.
    let mut base: HashMap<Symbol, BTreeSet<Symbol>> = HashMap::new();
    for field in &derived_fields {
        let mut stack = Vec::new();
        let mut visiting = HashSet::new();
        close_derived(
            *field,
            &direct,
            &plain,
            &mut base,
            &mut visiting,
            &mut stack,
            interner,
        )?;
    }

    let ctx = Ctx {
        plain,
        derived: derived_set,
        props,
        base,
    };

    // The view's sites.
    let mut sites = Vec::new();
    if let Some(view) = &def.view {
        let mut env: HashMap<Symbol, Resolved> = HashMap::new();
        walk_nodes(&view.nodes, "", &ctx, &mut env, &mut sites, false);
    }

    // The turns.
    let mut writes: BTreeMap<String, BTreeSet<Symbol>> = BTreeMap::new();
    for action in &def.actions {
        let mut shadowed: HashSet<Symbol> = action.params.iter().map(|(n, _)| *n).collect();
        let mut set = BTreeSet::new();
        writes_of_block(&action.body, &ctx, &mut shadowed, &mut set);
        set.retain(|n| ctx.plain.contains(n));
        writes.insert(interner.resolve(action.name).to_string(), set);
    }
    for handler in &def.events {
        let mut shadowed = HashSet::new();
        let mut set = BTreeSet::new();
        writes_of_block(&handler.body, &ctx, &mut shadowed, &mut set);
        set.retain(|n| ctx.plain.contains(n));
        writes.insert(trigger_label(&handler.trigger, interner), set);
    }

    let derived_out: BTreeMap<Symbol, BTreeSet<Symbol>> = derived_fields
        .iter()
        .map(|f| (*f, ctx.base.get(f).cloned().unwrap_or_default()))
        .collect();

    Ok(ComponentDeps {
        sites,
        derived: derived_out,
        writes,
    })
}

/// The state fields one block of statements writes — the same walk `component_deps` uses for an
/// action, exposed so a lowering can ask about a body it is about to emit.
///
/// Shared rather than re-derived: a second copy of "what counts as a write" drifts the first time
/// a verb is added, and the two answers disagreeing means one tier patches while the other
/// repaints — a divergence on the same page.
pub fn block_writes(def: &ComponentDef<'_>, block: Block<'_>, _interner: &Interner) -> Vec<Symbol> {
    let plain: HashSet<Symbol> = def
        .state
        .iter()
        .filter(|f| !f.derived)
        .map(|f| f.name)
        .collect();
    let ctx = Ctx {
        plain,
        derived: def
            .state
            .iter()
            .filter(|f| f.derived)
            .map(|f| f.name)
            .collect(),
        props: def.props.iter().map(|(n, _)| *n).collect(),
        base: HashMap::new(),
    };
    let mut shadowed = HashSet::new();
    let mut out = BTreeSet::new();
    writes_of_block(block, &ctx, &mut shadowed, &mut out);
    out.into_iter().filter(|n| ctx.plain.contains(n)).collect()
}

/// The PROPS a block writes — every one a mistake, since a prop is the parent's value. `bound`
/// are the names the block's own header binds (an action's parameters, a trigger's values), which
/// shadow a prop of the same name. The same walk as [`block_writes`], so a verb that writes state
/// is a verb that writes a prop.
pub fn block_prop_writes(
    def: &ComponentDef<'_>,
    block: Block<'_>,
    bound: &[Symbol],
) -> Vec<Symbol> {
    let ctx = Ctx {
        plain: def
            .state
            .iter()
            .filter(|f| !f.derived)
            .map(|f| f.name)
            .collect(),
        derived: def
            .state
            .iter()
            .filter(|f| f.derived)
            .map(|f| f.name)
            .collect(),
        props: def.props.iter().map(|(n, _)| *n).collect(),
        base: HashMap::new(),
    };
    let mut shadowed: HashSet<Symbol> = bound.iter().copied().collect();
    let mut out = BTreeSet::new();
    writes_of_block(block, &ctx, &mut shadowed, &mut out);
    out.into_iter().filter(|n| ctx.props.contains(n)).collect()
}

/// Everything the walks need to decide what a name means.
struct Ctx {
    plain: HashSet<Symbol>,
    derived: HashSet<Symbol>,
    props: HashSet<Symbol>,
    base: HashMap<Symbol, BTreeSet<Symbol>>,
}

impl Ctx {
    /// A single NAME, resolved to base state fields and props.
    fn resolve_name(&self, name: Symbol, env: &HashMap<Symbol, Resolved>) -> Resolved {
        // A template binding shadows everything: `{let n be …}` beside a field `n` means the
        // binding, exactly as an action's own `Let` shadows the field for a write.
        if let Some(found) = env.get(&name) {
            return found.clone();
        }
        if self.derived.contains(&name) {
            let state = self.base.get(&name).cloned().unwrap_or_default();
            return Resolved {
                state,
                props: BTreeSet::new(),
            };
        }
        if self.plain.contains(&name) {
            return Resolved {
                state: [name].into_iter().collect(),
                props: BTreeSet::new(),
            };
        }
        if self.props.contains(&name) {
            return Resolved {
                state: BTreeSet::new(),
                props: [name].into_iter().collect(),
            };
        }
        Resolved::default()
    }

    /// A whole expression.
    fn resolve_expr(&self, expr: &Expr<'_>, env: &HashMap<Symbol, Resolved>) -> Resolved {
        let mut names = Vec::new();
        collect_names(expr, &mut names);
        let mut out = Resolved::default();
        for n in names {
            let r = self.resolve_name(n, env);
            out.merge(&r);
        }
        out
    }
}

/// Close one derived field over the derivations it reads, refusing a cycle by name.
fn close_derived(
    field: Symbol,
    direct: &HashMap<Symbol, Vec<Symbol>>,
    plain: &HashSet<Symbol>,
    base: &mut HashMap<Symbol, BTreeSet<Symbol>>,
    visiting: &mut HashSet<Symbol>,
    stack: &mut Vec<Symbol>,
    interner: &Interner,
) -> Result<BTreeSet<Symbol>, DepError> {
    if let Some(done) = base.get(&field) {
        return Ok(done.clone());
    }
    if !visiting.insert(field) {
        // The cycle names every field on it, in the order the walk met them — a message naming
        // one field would leave the reader looking for the other half.
        let mut names: Vec<String> = stack
            .iter()
            .map(|s| interner.resolve(*s).to_string())
            .collect();
        names.push(interner.resolve(field).to_string());
        return Err(DepError::DerivedCycle(names));
    }
    stack.push(field);
    let mut out = BTreeSet::new();
    for dep in direct.get(&field).map(|v| v.as_slice()).unwrap_or(&[]) {
        if plain.contains(dep) {
            out.insert(*dep);
        } else {
            let inner = close_derived(*dep, direct, plain, base, visiting, stack, interner)?;
            out.extend(inner);
        }
    }
    stack.pop();
    visiting.remove(&field);
    base.insert(field, out.clone());
    Ok(out)
}

/// Every identifier an expression mentions, including through interpolation and closures.
///
/// Deliberately EXHAUSTIVE over `Expr`: a new variant must be considered here, because a variant
/// the walk silently ignores is a read the patcher never learns about, and the symptom is a page
/// that stops updating rather than anything that looks like a compiler bug.
fn collect_names(expr: &Expr<'_>, out: &mut Vec<Symbol>) {
    match expr {
        Expr::Identifier(s) => out.push(*s),
        Expr::Literal(_) | Expr::OptionNone | Expr::Escape { .. } => {}
        Expr::BinaryOp { left, right, .. }
        | Expr::Contains {
            collection: left,
            value: right,
        }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range {
            start: left,
            end: right,
        }
        | Expr::Index {
            collection: left,
            index: right,
        }
        | Expr::WithCapacity {
            value: left,
            capacity: right,
        }
        | Expr::ChunkAt {
            index: left,
            zone: right,
        } => {
            collect_names(left, out);
            collect_names(right, out);
        }
        Expr::Not { operand }
        | Expr::Copy { expr: operand }
        | Expr::Give { value: operand }
        | Expr::Length {
            collection: operand,
        }
        | Expr::ManifestOf { zone: operand }
        | Expr::OptionSome { value: operand } => collect_names(operand, out),
        Expr::Slice {
            collection,
            start,
            end,
        } => {
            collect_names(collection, out);
            collect_names(start, out);
            collect_names(end, out);
        }
        // A function NAME is not a read of state — only its arguments are.
        Expr::Call { args, .. } => args.iter().for_each(|a| collect_names(a, out)),
        Expr::CallExpr { callee, args } => {
            collect_names(callee, out);
            args.iter().for_each(|a| collect_names(a, out));
        }
        Expr::List(items) | Expr::Tuple(items) => items.iter().for_each(|i| collect_names(i, out)),
        // `t's text` depends on whatever `t` is.
        Expr::FieldAccess { object, .. } => collect_names(object, out),
        Expr::New { init_fields, .. } => {
            init_fields.iter().for_each(|(_, e)| collect_names(e, out))
        }
        Expr::NewVariant { fields, .. } => fields.iter().for_each(|(_, e)| collect_names(e, out)),
        Expr::Closure { body, .. } => match body {
            ClosureBody::Expression(e) => collect_names(e, out),
            ClosureBody::Block(b) => b.iter().for_each(|s| collect_names_stmt(s, out)),
        },
        Expr::InterpolatedString(parts) => {
            for part in parts {
                if let StringPart::Expr { value, .. } = part {
                    collect_names(value, out);
                }
            }
        }
        // A nested view's own dependencies belong to that view, not to the expression holding it.
        Expr::View(_) => {}
        // A page question reads the DOCUMENT, not this component's state; its selector may still
        // be computed from state.
        Expr::Dom(q) => {
            if let Some(sel) = q.selector() {
                collect_names(sel, out);
            }
        }
    }
}

/// The identifiers a statement mentions — used for closure bodies inside expressions.
fn collect_names_stmt(stmt: &Stmt<'_>, out: &mut Vec<Symbol>) {
    let mut sink = |e: &Expr<'_>| collect_names(e, out);
    match stmt {
        Stmt::Let { value, .. } => sink(value),
        Stmt::Show { object, recipient } => {
            sink(object);
            sink(recipient);
        }
        Stmt::Set { target, value } => {
            out.push(*target);
            collect_names(value, out);
        }
        Stmt::Return { value } => {
            if let Some(v) = value {
                collect_names(v, out);
            }
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            collect_names(cond, out);
            then_block.iter().for_each(|s| collect_names_stmt(s, out));
            if let Some(b) = else_block {
                b.iter().for_each(|s| collect_names_stmt(s, out));
            }
        }
        Stmt::While { cond, body, .. } => {
            collect_names(cond, out);
            body.iter().for_each(|s| collect_names_stmt(s, out));
        }
        Stmt::Repeat { iterable, body, .. } => {
            collect_names(iterable, out);
            body.iter().for_each(|s| collect_names_stmt(s, out));
        }
        _ => {}
    }
}

/// The state fields a block writes, honouring shadowing by the block's own `Let`s.
fn writes_of_block(
    block: Block<'_>,
    ctx: &Ctx,
    shadowed: &mut HashSet<Symbol>,
    out: &mut BTreeSet<Symbol>,
) {
    for stmt in block {
        writes_of_stmt(stmt, ctx, shadowed, out);
    }
}

/// One statement's writes: every name it writes that the block has not bound itself. Callers
/// keep the kind of name they ask about (state fields, props).
///
/// Shadowing decides what a write means: after an action's own `Let mutable n be 5.`, `Set n` is
/// a write to the LOCAL, not to a field that happens to share the name.
fn writes_of_stmt(
    stmt: &Stmt<'_>,
    ctx: &Ctx,
    shadowed: &mut HashSet<Symbol>,
    out: &mut BTreeSet<Symbol>,
) {
    let note = |name: Symbol, shadowed: &HashSet<Symbol>, out: &mut BTreeSet<Symbol>| {
        if !shadowed.contains(&name) {
            out.insert(name);
        }
    };
    match stmt {
        Stmt::Let { var, .. } => {
            shadowed.insert(*var);
        }
        Stmt::Set { target, .. } => note(*target, shadowed, out),
        Stmt::Swap { a, b } => {
            note(*a, shadowed, out);
            note(*b, shadowed, out);
        }
        // A collection verb writes whatever identifier the collection expression bottoms out in:
        // `Push v to todos` and `Push v to it's todos` both change `todos`.
        Stmt::Push { collection, .. }
        | Stmt::Add { collection, .. }
        | Stmt::Remove { collection, .. }
        | Stmt::SetIndex { collection, .. } => {
            for n in base_identifiers(collection) {
                note(n, shadowed, out);
            }
        }
        Stmt::Pop { collection, into } => {
            for n in base_identifiers(collection) {
                note(n, shadowed, out);
            }
            if let Some(v) = into {
                note(*v, shadowed, out);
            }
        }
        Stmt::SetField { object, .. }
        | Stmt::IncreaseCrdt { object, .. }
        | Stmt::DecreaseCrdt { object, .. } => {
            for n in base_identifiers(object) {
                note(n, shadowed, out);
            }
        }
        // ★ A UI VERB CAN WRITE STATE TOO — `Toggle done.` moves a Bool field and `Load "k" into
        // x.` fills one. Both were invisible here until a test asked, which is the exact failure
        // this analysis exists to prevent: a write the compiler cannot see is a site that never
        // updates, and the page looks broken while the state is perfectly correct.
        //
        // Read through `UiStmt::writes()` rather than re-matched here, so the ONE place the
        // language decides what a UI verb writes stays the one place — a second copy would drift
        // the first time a verb is added.
        Stmt::Ui(ui) => {
            if let Some(name) = ui.writes() {
                note(name, shadowed, out);
            }
        }
        // ★ A write inside a branch counts even when the branch does not run. The analysis is a
        // static over-approximation on purpose: a spare patch is waste, a missed one is a stale
        // page, and only one of those is visible to whoever is using the app.
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            let mut inner = shadowed.clone();
            writes_of_block(then_block, ctx, &mut inner, out);
            if let Some(b) = else_block {
                let mut inner = shadowed.clone();
                writes_of_block(b, ctx, &mut inner, out);
            }
        }
        Stmt::While { body, .. } => {
            let mut inner = shadowed.clone();
            writes_of_block(body, ctx, &mut inner, out);
        }
        Stmt::Repeat { pattern, body, .. } => {
            let mut inner = shadowed.clone();
            bind_pattern(pattern, &mut inner);
            writes_of_block(body, ctx, &mut inner, out);
        }
        Stmt::Inspect { arms, .. } => {
            for arm in arms {
                let mut inner = shadowed.clone();
                for (_, binding) in &arm.bindings {
                    inner.insert(*binding);
                }
                writes_of_block(arm.body, ctx, &mut inner, out);
            }
        }
        // A desugared statement (`Push v to item 1 of xs`, `Set item i of xs to v`) is inline: its
        // temporaries are its own `Let`s, and its write-back lands on the collection it names.
        Stmt::Splice { body } => writes_of_block(body, ctx, shadowed, out),
        _ => {}
    }
}

/// Add every name a destructuring pattern binds to the shadow set.
fn bind_pattern(pattern: &Pattern, shadowed: &mut HashSet<Symbol>) {
    match pattern {
        Pattern::Identifier(s) => {
            shadowed.insert(*s);
        }
        Pattern::Tuple(parts) => {
            for p in parts {
                shadowed.insert(*p);
            }
        }
    }
}

/// The identifiers an lvalue expression bottoms out in.
fn base_identifiers(expr: &Expr<'_>) -> Vec<Symbol> {
    let mut out = Vec::new();
    collect_names(expr, &mut out);
    out
}

/// A stable label for a turn started by an event sentence.
fn trigger_label(trigger: &Trigger<'_>, interner: &Interner) -> String {
    let text_of = |e: &Expr<'_>| match e {
        Expr::Literal(crate::ast::stmt::Literal::Text(s)) => interner.resolve(*s).to_string(),
        _ => "<computed>".to_string(),
    };
    match trigger {
        Trigger::PageLoads => "when:page-loads".to_string(),
        Trigger::Appears => "when:appears".to_string(),
        Trigger::Disappears => "when:disappears".to_string(),
        Trigger::RouteChanges => "when:route-changes".to_string(),
        Trigger::Online => "when:online".to_string(),
        Trigger::Offline => "when:offline".to_string(),
        Trigger::Selector {
            selector, event, ..
        } => {
            format!("when:{}:{}", text_of(selector), interner.resolve(*event))
        }
        Trigger::Key { key } => format!("when:key:{}", interner.resolve(*key)),
        Trigger::FieldChanges { store, field } => match store {
            Some(s) => format!(
                "when:{}.{}:changes",
                interner.resolve(*s),
                interner.resolve(*field)
            ),
            None => format!("when:{}:changes", interner.resolve(*field)),
        },
        Trigger::Every { .. } => "when:every".to_string(),
        Trigger::After { .. } => "when:after".to_string(),
        Trigger::Message { topic, .. } => format!("when:message:{}", text_of(topic)),
        Trigger::ActionFails { .. } => "when:action-fails".to_string(),
        Trigger::FetchFails { .. } => "when:fetch-fails".to_string(),
        Trigger::WindowResizes => "when:resize".to_string(),
        Trigger::ScrollsToBottom { .. } => "when:scroll-bottom".to_string(),
        Trigger::FileDropped { .. } => "when:file-drop".to_string(),
    }
}

/// Walk a list of sibling nodes, emitting a site for everything that can change.
fn walk_nodes(
    nodes: &[ViewNode<'_>],
    prefix: &str,
    ctx: &Ctx,
    env: &mut HashMap<Symbol, Resolved>,
    out: &mut Vec<SiteDeps>,
    dynamic: bool,
) {
    for (i, node) in nodes.iter().enumerate() {
        let path = format!("{prefix}{i}");
        walk_node(node, &path, ctx, env, out, dynamic);
    }
}

fn push_site(
    out: &mut Vec<SiteDeps>,
    path: &str,
    kind: SiteKind,
    resolved: Resolved,
    dynamic: bool,
) {
    out.push(SiteDeps {
        path: path.to_string(),
        kind,
        dynamic,
        reads: resolved.state,
        prop_reads: resolved.props,
    });
}

fn walk_node(
    node: &ViewNode<'_>,
    path: &str,
    ctx: &Ctx,
    env: &mut HashMap<Symbol, Resolved>,
    out: &mut Vec<SiteDeps>,
    dynamic: bool,
) {
    match node {
        ViewNode::Element {
            attrs, children, ..
        } => {
            for attr in attrs {
                match attr {
                    // A LITERAL attribute never changes, so it is not an update site at all —
                    // listing it would put a constant in some field's invalidation set and make
                    // every turn touch it.
                    Attr::Plain { name, value } => {
                        if !is_static(value) {
                            push_site(
                                out,
                                path,
                                SiteKind::Attr(*name),
                                ctx.resolve_expr(value, env),
                                dynamic,
                            );
                        }
                    }
                    Attr::Class { name, cond } => {
                        push_site(
                            out,
                            path,
                            SiteKind::Class(*name),
                            ctx.resolve_expr(cond, env),
                            dynamic,
                        );
                    }
                    Attr::Bind { field } => {
                        let r = ctx.resolve_name(*field, env);
                        push_site(out, path, SiteKind::Bind(*field), r, dynamic);
                    }
                    Attr::Directive { name, value } => {
                        push_site(
                            out,
                            path,
                            SiteKind::Attr(*name),
                            ctx.resolve_expr(value, env),
                            dynamic,
                        );
                    }
                }
            }
            walk_nodes(children, &format!("{path}."), ctx, env, out, dynamic);
        }
        ViewNode::Text { text, .. } => {
            push_site(
                out,
                path,
                SiteKind::Text,
                ctx.resolve_expr(text, env),
                dynamic,
            );
        }
        ViewNode::Component {
            props, children, ..
        } => {
            // A prop expression is evaluated HERE, so its reads belong to this component: when
            // `todos` moves, the child has to be handed a new value.
            for (name, value) in props {
                push_site(
                    out,
                    path,
                    SiteKind::Attr(*name),
                    ctx.resolve_expr(value, env),
                    dynamic,
                );
            }
            walk_nodes(children, &format!("{path}."), ctx, env, out, dynamic);
        }
        ViewNode::Each {
            pattern,
            iterable,
            key,
            body,
            empty,
            ..
        } => {
            let over = ctx.resolve_expr(iterable, env);
            push_site(out, path, SiteKind::Each, over.clone(), dynamic);
            // ★ The loop variable is a WINDOW onto the collection: everything the body reads
            // through it depends on the collection, and on nothing the body could name instead.
            let mut inner = env.clone();
            bind_pattern_env(pattern, &over, &mut inner);
            if let Some(k) = key {
                let _ = ctx.resolve_expr(k, &inner);
            }
            walk_nodes(body, &format!("{path}."), ctx, &mut inner, out, true);
            walk_nodes(empty, &format!("{path}.e"), ctx, env, out, true);
        }
        ViewNode::If {
            arms, otherwise, ..
        } => {
            // ONE site for the whole `{if}`: any condition changing may switch which arm renders,
            // and the patcher's job there is to mount or unmount a subtree, not to edit a value.
            let mut conds = Resolved::default();
            for (cond, _) in arms {
                let r = ctx.resolve_expr(cond, env);
                conds.merge(&r);
            }
            push_site(out, path, SiteKind::If, conds, dynamic);
            for (k, (_, body)) in arms.iter().enumerate() {
                let mut inner = env.clone();
                walk_nodes(body, &format!("{path}.{k}."), ctx, &mut inner, out, true);
            }
            if let Some(body) = otherwise {
                let mut inner = env.clone();
                walk_nodes(body, &format!("{path}.o"), ctx, &mut inner, out, true);
            }
        }
        ViewNode::Inspect {
            scrutinee, arms, ..
        } => {
            let over = ctx.resolve_expr(scrutinee, env);
            push_site(out, path, SiteKind::Inspect, over.clone(), dynamic);
            for (k, arm) in arms.iter().enumerate() {
                let mut inner = env.clone();
                // A pattern binding is a window onto the scrutinee, exactly as a loop variable is
                // onto its collection.
                for (_, binding) in &arm.bindings {
                    inner.insert(*binding, over.clone());
                }
                walk_nodes(
                    &arm.body,
                    &format!("{path}.{k}."),
                    ctx,
                    &mut inner,
                    out,
                    true,
                );
            }
        }
        ViewNode::Let { var, value, .. } => {
            let r = ctx.resolve_expr(value, env);
            env.insert(*var, r);
        }
        ViewNode::Raw { html, .. } => {
            push_site(
                out,
                path,
                SiteKind::Text,
                ctx.resolve_expr(html, env),
                dynamic,
            );
        }
        ViewNode::Block { default, .. } => {
            if let Some(body) = default {
                walk_nodes(body, &format!("{path}."), ctx, env, out, dynamic);
            }
        }
        // A handler's body is a TURN, recorded in the write sets rather than as a read site; an
        // `{extends}` and a `{children}` slot hold nothing that can change.
        ViewNode::Handler { .. } | ViewNode::Extends { .. } | ViewNode::Children { .. } => {}
    }
}

/// Bind a loop pattern's names to what the collection resolves to.
fn bind_pattern_env(pattern: &Pattern, over: &Resolved, env: &mut HashMap<Symbol, Resolved>) {
    match pattern {
        Pattern::Identifier(s) => {
            env.insert(*s, over.clone());
        }
        Pattern::Tuple(parts) => {
            for p in parts {
                env.insert(*p, over.clone());
            }
        }
    }
}

/// Is this expression a compile-time constant — something that can never need an update?
fn is_static(expr: &Expr<'_>) -> bool {
    match expr {
        Expr::Literal(_) => true,
        Expr::InterpolatedString(parts) => {
            parts.iter().all(|p| matches!(p, StringPart::Literal(_)))
        }
        _ => false,
    }
}
