use super::json::JsonShapes;
use super::*;
use crate::ast::component::{ComponentDef, ComponentKind, RouteBody, Trigger};
use crate::ast::stmt::{Expr, Literal, Stmt, TypeExpr};
use crate::ast::view::{Attr, ViewNode};
use crate::lgx::html::{Behaviour, EventModifier};
use crate::token::Span;
use logicaffeine_base::{Interner, Symbol};

/// Whether a store's state can be given a record type here. A derived field or a field whose
/// type cannot be named from its annotation or its initializer means it cannot.
/// The storage AREA a store keeps its state in between page loads, or `None` for one that keeps
/// nothing there. The area is the lifetime the author asked for, spelled the way the browser
/// spells it — the one decision every tier reads, so every tier puts a persisted store in the same
/// place.
pub fn store_area(def: &ComponentDef<'_>) -> Option<&'static str> {
    match &def.kind {
        ComponentKind::Store {
            mode: crate::ast::component::StoreMode::Persisted { .. },
        } => Some("local"),
        ComponentKind::Store {
            mode: crate::ast::component::StoreMode::Preserved,
        } => Some("session"),
        ComponentKind::Store {
            mode: crate::ast::component::StoreMode::Shared { .. },
        } => Some(SHARED_AREA),
        _ => None,
    }
}

/// Whether a field of this type has a faithful TEXT form, which is what storage carries.
///
/// Refusing is the point: a record quietly stored as its display form and read back as the TEXT
/// of that display is a page that loses its state and says nothing about it.
pub(crate) fn storable_type(interner: &Interner, ty: Symbol) -> bool {
    matches!(
        interner.resolve(ty),
        "Int" | "Text" | "Bool" | "Float" | "Real"
    )
}

/// The element `Seq of <element>` names, from EITHER an explicit annotation (`Let x: Seq of Text
/// be …`) or the constructor an UNANNOTATED field was given (`Let x be a new Seq of Text.`, which
/// is how every one of this campaign's own fixtures writes one) — the one shape a store's own
/// record, and `Save`/`Load`, thread a list through. `None` for anything else, including a `Seq`
/// of more than one type parameter or of something other than a plain name.
pub(crate) fn seq_field_element<'a>(
    ty: Option<&TypeExpr<'a>>,
    init: &Expr<'a>,
    interner: &Interner,
) -> Option<Symbol> {
    let is_seq = |base: Symbol| interner.resolve(base).eq_ignore_ascii_case("Seq");
    let elem_of = |params: &[TypeExpr<'a>]| -> Option<Symbol> {
        if params.len() != 1 {
            return None;
        }
        match &params[0] {
            TypeExpr::Primitive(e) | TypeExpr::Named(e) => Some(*e),
            _ => None,
        }
    };
    match ty {
        Some(TypeExpr::Generic { base, params }) if is_seq(*base) => elem_of(params),
        Some(_) => None,
        None => match init {
            Expr::New {
                type_name,
                type_args,
                ..
            } if is_seq(*type_name) => elem_of(type_args),
            _ => None,
        },
    }
}

/// Why a store cannot be lowered, or `None` when it can.
///
/// TWO DIFFERENT PROBLEMS with two different fixes, and they must not borrow each other's words:
/// a field whose TYPE this tier cannot name is a record-typing problem, while a field with no
/// faithful TEXT form in a store that keeps its state between page loads is a persistence problem
/// — and the second is fixed either by changing the field or by dropping `persisted as`. A
/// refusal that names the wrong cause costs the reader the time it takes to rule out the thing
/// you told them, which is worse than naming no cause at all.
///
/// Asked with the same codec questions ([`JsonShapes`]) the store is emitted behind
/// ([`store_lowerable`]), so a store is never lowered by the one and refused by the other.
pub(crate) fn store_not_lowerable(
    def: &ComponentDef<'_>,
    shapes: &JsonShapes<'_>,
) -> Option<&'static str> {
    if !store_lowerable(def, shapes) {
        // A store this tier cannot type at all — the record-shape problem.
        if !store_shape_lowerable(def, shapes.interner()) {
            return Some("it uses a store whose state this tier cannot give a record type");
        }
        // It types fine; what it cannot do is CARRY those fields through text.
        return Some(
            "it uses a store that keeps its state between page loads, and one of its fields holds \
             something text cannot carry. A stored field can be a Text, an Int, a Real, a Bool, a \
             Seq of one of those, or a record this file can fully encode and decode as JSON",
        );
    }
    None
}

/// The record-shape half: a store whose fields this tier can give a named type, regardless of
/// whether it also has to survive a page load.
///
/// A `Seq of <anything named>` is accepted UNCONDITIONALLY here, scalar or record element alike —
/// sound because a store that does not persist needs no serialization at all: its `Seq` field
/// becomes an ordinary `FieldType::Generic` in the type registry, resolved by the exact same
/// per-field boundary inference every plain `## A Foo has: … Seq of X.` record's field already
/// goes through. Only a store that KEEPS its state between page loads needs the codec, and that
/// is [`store_lowerable`]'s own, separate, stricter question.
pub(crate) fn store_shape_lowerable(def: &ComponentDef<'_>, interner: &Interner) -> bool {
    matches!(
        def.kind,
        ComponentKind::Store {
            mode: crate::ast::component::StoreMode::Local
                | crate::ast::component::StoreMode::Persisted { .. }
                | crate::ast::component::StoreMode::Preserved
                | crate::ast::component::StoreMode::Shared { .. }
        }
    ) && !def.state.iter().any(|f| f.derived)
        && def.state.iter().all(|f| {
            shared_convergent_field(def, f, interner)
                || matches!(f.ty, Some(TypeExpr::Primitive(_) | TypeExpr::Named(_)))
                || seq_field_element(f.ty, f.init, interner).is_some()
                || matches!(
                    f.init,
                    Expr::Literal(
                        Literal::Number(_)
                            | Literal::Text(_)
                            | Literal::Boolean(_)
                            | Literal::Float(_)
                    )
                )
        })
}

/// Whether `field` of `def` is a convergent field of a `shared on` store — a `Tally`, a
/// `SharedSet of T`, … — whose record type is the plain value it converges to and whose topic form
/// is its CRDT's frame (`lgx_lower::shared`).
fn shared_convergent_field(
    def: &ComponentDef<'_>,
    field: &crate::ast::component::StateField<'_>,
    interner: &Interner,
) -> bool {
    matches!(
        def.kind,
        ComponentKind::Store {
            mode: crate::ast::component::StoreMode::Shared { .. }
        }
    ) && field
        .ty
        .and_then(|ty| super::shared::shared_field_kind(ty, interner))
        .is_some_and(|kind| kind.is_ok())
}

/// Whether a store fully lowers — its shape ([`store_shape_lowerable`]) AND, when it keeps its
/// state between page loads, every field's own text form: a scalar's fixed tag, or a record this
/// file can fully encode AND decode as JSON ([`JsonShapes::record_round_trips`]).
pub(crate) fn store_lowerable(def: &ComponentDef<'_>, shapes: &JsonShapes<'_>) -> bool {
    let interner = shapes.interner();
    let record_round_trips = |ty: Symbol| shapes.record_round_trips(ty);
    // A SHARED store is a module record like any other, its fields carried to and from the relay
    // topic in their TEXT form (`lgx_lower::shared`) exactly as a persisted store's are carried to
    // and from storage — so it takes the same field rule below.
    let persisted_field_ok = |f: &crate::ast::component::StateField<'_>| -> bool {
        if shared_convergent_field(def, f, interner) {
            return true;
        }
        if let Some(elem) = seq_field_element(f.ty, f.init, interner) {
            return storable_type(interner, elem) || record_round_trips(elem);
        }
        match f.ty {
            Some(TypeExpr::Primitive(t) | TypeExpr::Named(t)) => {
                storable_type(interner, *t) || record_round_trips(*t)
            }
            Some(_) => false,
            None => matches!(
                f.init,
                Expr::Literal(
                    Literal::Number(_) | Literal::Text(_) | Literal::Boolean(_) | Literal::Float(_)
                )
            ),
        }
    };
    matches!(
        def.kind,
        ComponentKind::Store {
            mode: crate::ast::component::StoreMode::Local
                | crate::ast::component::StoreMode::Persisted { .. }
                | crate::ast::component::StoreMode::Preserved
                | crate::ast::component::StoreMode::Shared { .. }
        }
    )
        && !def.state.iter().any(|f| f.derived)
        // A store that keeps its state between page loads carries every field through TEXT (a
        // scalar's own fixed tag, or the JSON codec's `j:` tag), so a field with neither a
        // faithful text form nor a verified JSON round-trip is refused HERE rather than lowered
        // into a page that silently forgets it.
        && (store_area(def).is_none() || def.state.iter().all(persisted_field_ok))
        && store_shape_lowerable(def, interner)
}

/// Why a component cannot be lowered — the message the VM refuses a `Mount` with, and WHERE.
pub struct NotLowerable {
    pub message: String,
    /// The node, handler or definition that could not be lowered.
    pub span: Span,
}

impl NotLowerable {
    /// A refusal, from a literal or from a message built around the author's own words, AT the
    /// place that could not be lowered.
    ///
    /// ★ IT CARRIES A `String` SO IT CAN NAME WHAT IT REFUSED. Every refusal here used to be a
    /// `&'static str`, which quietly forced each one to describe a CLASS ("one of its `Actions:`
    /// sentences waits for something this tier cannot deliver yet") rather than the sentence the
    /// author wrote. That is the anonymous-refusal shape this campaign has now fixed five times:
    /// the reader is told a page failed and left to find which line did it.
    ///
    /// ★ AND IT CARRIES A `Span`, WHICH IS THE OTHER HALF OF THAT SAME FIX. Naming the construct
    /// tells the reader WHAT; only a span tells them WHERE, and every one of these rendered at
    /// `Span::default()` — the top of the file — because the walk destructured each node with
    /// `..` and threw the location away. A message that says "its template uses a directive" and
    /// points at line 1 leaves the author scanning a forty-node view by hand.
    ///
    /// The span is a REQUIRED argument rather than an `Option` or a defaulted field, so a new
    /// refusal cannot be added without saying where it happened. That is the only version of this
    /// that does not rot: the previous shape made "no span" the path of least resistance, and all
    /// twenty-one sites took it.
    pub(crate) fn at(span: Span, text: impl Into<String>) -> Self {
        NotLowerable {
            message: text.into(),
            span,
        }
    }
}

/// Whether a component's state has to survive between renders — because an action writes it,
/// a sentence answers an event, or an input is bound to it. Such a component keeps its state in
/// module-level bindings; one without any of these reads its initializers each render.

/// Every component a view PLACES, at any depth — the edges of the component tree, which is what
/// the server renderer's demand rule closes over.
/// Every component this view PLACES, at any depth.
///
/// ★ PUBLIC BECAUSE TWO RENDERERS NEED THE SAME ANSWER. The server hoists stylesheets from the
/// root and the browser hoists them at the mount point — both walk this exact graph, and two
/// copies of it would be two chances to disagree about which sheets a page carries.
/// The components a view places INSIDE a `{for each}` — the ones that share a mount point.
///
/// ★ SHARING A MOUNT POINT IS WHAT MAKES A REPAINT UNSAFE, and a loop is the second way to get
/// there. `repaint_def` re-renders into `__M_<C>[key]` and CLEARS it first; for a row drawn by
/// `{for each t in tiles:}<Tile …/>{end}` that recorded mount point is the enclosing `<ul>`,
/// which every row shares. So one row's unpatchable turn cleared the list and redrew itself
/// alone — two rows in, click the first, and the second is gone from the document.
///
/// A placed WRAPPER already had this exact problem for the same reason and is already redirected;
/// this is the same fact about a different shape, which is why it joins that predicate rather than
/// growing a second one.
/// The components an instance of which can ever LEAVE the page.
///
/// ★ MACHINERY FOR SOMETHING THAT CANNOT HAPPEN IS STILL SHIPPED TO THE READER. The unmount sweep
/// answers "which instances were not drawn in the last paint", and an instance can only fail to be
/// drawn if something chose not to draw it — a `{if}` arm, a `{for each}` whose list shrank, an
/// `{inspect}` branch. A page whose components are all placed unconditionally can never sweep
/// anything, and it was carrying the sweep anyway: measured at 1,052 bytes of wasm on a page with
/// ONE state field, which is a third of what such a page costs at all.
///
/// A component under a conditional node is unmountable, and so is everything it places, however
/// unconditionally it places them — when the `{if}` closes, the whole subtree goes with it.
pub(crate) fn unmountable_components(all: &[ComponentDef<'_>]) -> Vec<Symbol> {
    fn walk(nodes: &[ViewNode<'_>], conditional: bool, out: &mut Vec<Symbol>) {
        for node in nodes {
            match node {
                ViewNode::Component { name, children, .. } => {
                    if conditional && !out.contains(name) {
                        out.push(*name);
                    }
                    walk(children, conditional, out);
                }
                ViewNode::Element { children, .. } => walk(children, conditional, out),
                // The three ways a view chooses. Every arm is conditional, including the `otherwise`
                // — it is not drawn when another arm is.
                ViewNode::If {
                    arms, otherwise, ..
                } => {
                    for (_, body) in arms {
                        walk(body, true, out);
                    }
                    if let Some(o) = otherwise {
                        walk(o, true, out);
                    }
                }
                ViewNode::Each { body, empty, .. } => {
                    walk(body, true, out);
                    walk(empty, true, out);
                }
                ViewNode::Inspect { arms, .. } => {
                    for arm in arms {
                        walk(&arm.body, true, out);
                    }
                }
                ViewNode::Block {
                    default: Some(body),
                    ..
                } => walk(body, conditional, out),
                _ => {}
            }
        }
    }
    let mut out: Vec<Symbol> = Vec::new();
    for def in all {
        if let Some(view) = def.view.as_ref() {
            walk(&view.nodes, false, &mut out);
        }
    }
    // Everything an unmountable component places goes with it, to any depth.
    let mut grew = true;
    while grew {
        grew = false;
        for name in out.clone() {
            let Some(def) = all.iter().find(|d| d.name == name) else {
                continue;
            };
            let Some(view) = def.view.as_ref() else {
                continue;
            };
            for placed in placed_components(&view.nodes) {
                if !out.contains(&placed) {
                    out.push(placed);
                    grew = true;
                }
            }
        }
    }
    out
}

pub fn loop_placed_components(nodes: &[ViewNode<'_>]) -> Vec<Symbol> {
    let mut out = Vec::new();
    fn walk(nodes: &[ViewNode<'_>], inside: bool, out: &mut Vec<Symbol>) {
        for node in nodes {
            match node {
                ViewNode::Component { name, children, .. } => {
                    if inside {
                        out.push(*name);
                    }
                    walk(children, inside, out);
                }
                ViewNode::Element { children, .. } => walk(children, inside, out),
                ViewNode::If {
                    arms, otherwise, ..
                } => {
                    for (_, body) in arms {
                        walk(body, inside, out);
                    }
                    if let Some(o) = otherwise {
                        walk(o, inside, out);
                    }
                }
                // The loop's BODY is inside; its `{empty}` markup draws once and is not.
                ViewNode::Each { body, empty, .. } => {
                    walk(body, true, out);
                    walk(empty, inside, out);
                }
                ViewNode::Inspect { arms, .. } => {
                    for arm in arms {
                        walk(&arm.body, inside, out);
                    }
                }
                ViewNode::Block {
                    default: Some(body),
                    ..
                } => walk(body, inside, out),
                _ => {}
            }
        }
    }
    walk(nodes, false, &mut out);
    out
}

/// The lowerable components with a `When this component appears:` handler, in declaration order —
/// the numbering the appears queue (`__LGX_APPEAR_WHO`) and its drain (`__lgx_appear`) share.
pub(crate) fn appearing_components(
    all: &[ComponentDef<'_>],
    types: Option<&crate::analysis::registry::TypeRegistry>,
    interner: &Interner,
) -> Vec<Symbol> {
    all.iter()
        .filter(|d| {
            matches!(
                d.kind,
                ComponentKind::Component | ComponentKind::Page { .. }
            )
        })
        .filter(|d| {
            d.events
                .iter()
                .any(|h| matches!(h.trigger, Trigger::Appears))
        })
        .filter(|d| lowerable(d, all, types, interner).is_ok())
        .map(|d| d.name)
        .collect()
}

/// Whether `def` can be drawn inside itself: its view places `def` again, directly or through the
/// components it places. Such a component has a finite rendering only if something in its view
/// stops the nesting, so its render counts how deep it is (`__LGX_RENDER_DEPTH`).
pub(crate) fn on_placement_cycle(def: &ComponentDef<'_>, all: &[ComponentDef<'_>]) -> bool {
    let mut seen: Vec<Symbol> = Vec::new();
    let mut pending: Vec<Symbol> = def
        .view
        .as_ref()
        .map(|v| placed_components(&v.nodes))
        .unwrap_or_default();
    while let Some(name) = pending.pop() {
        if name == def.name {
            return true;
        }
        if seen.contains(&name) {
            continue;
        }
        seen.push(name);
        if let Some(view) = all
            .iter()
            .find(|d| d.name == name)
            .and_then(|d| d.view.as_ref())
        {
            pending.extend(placed_components(&view.nodes));
        }
    }
    false
}

pub fn placed_components(nodes: &[ViewNode<'_>]) -> Vec<Symbol> {
    let mut out = Vec::new();
    fn walk(nodes: &[ViewNode<'_>], out: &mut Vec<Symbol>) {
        for node in nodes {
            match node {
                ViewNode::Component { name, children, .. } => {
                    out.push(*name);
                    walk(children, out);
                }
                ViewNode::Element { children, .. } => walk(children, out),
                ViewNode::If {
                    arms, otherwise, ..
                } => {
                    for (_, body) in arms {
                        walk(body, out);
                    }
                    if let Some(o) = otherwise {
                        walk(o, out);
                    }
                }
                ViewNode::Each { body, empty, .. } => {
                    walk(body, out);
                    walk(empty, out);
                }
                ViewNode::Inspect { arms, .. } => {
                    for arm in arms {
                        walk(&arm.body, out);
                    }
                }
                // ★ A LAYOUT IS A COMPONENT THIS VIEW DEPENDS ON, and it fell into the catch-all
                // below. Every reader of this walk asks the same question — which components does
                // this page reach — and `{extends Site}` reaches `Site` as surely as `<Site/>`
                // would. Dropping it meant the LAYOUT'S STYLESHEET WAS NEVER EMITTED: its elements
                // were stamped with its scope class, the scoped rule would have matched, and the
                // rule was not in the document. A page that renders with its styling silently
                // missing, which is the worst shape a styling bug can take — nothing to see, and
                // nothing said.
                ViewNode::Extends { layout, .. } => out.push(*layout),
                // And a block's DEFAULT body places components too; it is the markup used whenever
                // the page does not fill the block, so a component there is as real as any other.
                ViewNode::Block {
                    default: Some(body),
                    ..
                } => walk(body, out),
                _ => {}
            }
        }
    }
    walk(nodes, &mut out);
    out
}

/// Whether these statements call a `<Store>`'s action — asked of an action body BEFORE the store
/// rewrite runs over it, because that rewrite replaces the sentence with its lowered form.
pub(crate) fn touches_store(stmts: &[Stmt<'_>]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Ui(crate::ast::stmt::UiStmt::CallStore { .. }) => true,
        Stmt::If {
            then_block,
            else_block,
            ..
        } => touches_store(then_block) || else_block.is_some_and(touches_store),
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Splice { body } => {
            touches_store(body)
        }
        Stmt::Inspect { arms, .. } => arms.iter().any(|a| touches_store(a.body)),
        _ => false,
    })
}

/// Whether `def` has a `When <field> changes:` handler on one of its own plain fields — the handlers
/// a turn of `def` runs after its action commits.
pub(crate) fn watches_own_fields(def: &ComponentDef<'_>) -> bool {
    def.events.iter().any(|h| match h.trigger {
        Trigger::FieldChanges { store: None, field } => {
            def.state.iter().any(|f| f.name == field && !f.derived)
        }
        _ => false,
    })
}

/// The components with such handlers, in declaration order; a component's watch id (what
/// `__LGX_WATCHING` holds while its handlers run) is its position here plus one.
pub(crate) fn watching_components(all: &[ComponentDef<'_>]) -> Vec<Symbol> {
    all.iter()
        .filter(|d| watches_own_fields(d))
        .map(|d| d.name)
        .collect()
}

/// Whether these statements, at any depth, move the location with `Navigate to` — which ends the
/// turn by delivering the move to the page's route listeners.
pub(crate) fn navigates(stmts: &[Stmt<'_>]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Ui(crate::ast::stmt::UiStmt::Navigate { .. }) => true,
        Stmt::If {
            then_block,
            else_block,
            ..
        } => navigates(then_block) || else_block.is_some_and(navigates),
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Splice { body } => {
            navigates(body)
        }
        Stmt::Inspect { arms, .. } => arms.iter().any(|a| navigates(a.body)),
        _ => false,
    })
}

/// Whether any action or handler anywhere in `all` moves the location — the question the route
/// fan-out's emission asks, so a turn's epilogue never calls a fan-out that was not emitted.
pub(crate) fn any_navigates(all: &[ComponentDef<'_>]) -> bool {
    all.iter().any(|d| {
        d.actions.iter().any(|a| navigates(a.body)) || d.events.iter().any(|h| navigates(h.body))
    })
}

/// Whether `stmt` itself asks for an element event delivered after its turn commits — the
/// statements that lower through `Parser::queue_element_event`: a `Publish` (its page-local
/// delivery) and a `Hover` (the `mouseover` a browser raises for it). `Focus`/`Blur` are not
/// here: the host raises their events itself, and only on a real move of focus.
pub(crate) fn queues_element_event(stmt: &Stmt<'_>) -> bool {
    use crate::ast::stmt::UiStmt;
    matches!(
        stmt,
        Stmt::Ui(UiStmt::Publish { .. } | UiStmt::Hover { .. })
    )
}

/// Whether any statement in `stmts`, at any depth, queues an element event.
pub(crate) fn body_queues_element_event(stmts: &[Stmt<'_>]) -> bool {
    stmts.iter().any(|s| {
        queues_element_event(s)
            || match s {
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    body_queues_element_event(then_block)
                        || else_block.is_some_and(body_queues_element_event)
                }
                Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Splice { body } => {
                    body_queues_element_event(body)
                }
                Stmt::Inspect { arms, .. } => {
                    arms.iter().any(|a| body_queues_element_event(a.body))
                }
                _ => false,
            }
    })
}

/// Whether running `store`'s action `action` queues an element event — in its own body, or in any
/// store action it calls in turn.
pub(crate) fn store_action_queues(all: &[ComponentDef<'_>], store: Symbol, action: Symbol) -> bool {
    fn go(
        all: &[ComponentDef<'_>],
        store: Symbol,
        action: Symbol,
        seen: &mut Vec<(Symbol, Symbol)>,
    ) -> bool {
        if seen.contains(&(store, action)) {
            return false;
        }
        seen.push((store, action));
        let Some(body) = all
            .iter()
            .find(|d| d.name == store)
            .and_then(|d| d.actions.iter().find(|a| a.name == action))
            .map(|a| a.body)
        else {
            return false;
        };
        if body_queues_element_event(body) {
            return true;
        }
        let mut calls = Vec::new();
        store_calls_in(body, &mut calls);
        calls.into_iter().any(|(s, a)| go(all, s, a, seen))
    }
    go(all, store, action, &mut Vec::new())
}

/// Every `Call <Store>'s <action>` in `stmts`, at any depth.
fn store_calls_in(stmts: &[Stmt<'_>], out: &mut Vec<(Symbol, Symbol)>) {
    for s in stmts {
        match s {
            Stmt::Ui(crate::ast::stmt::UiStmt::CallStore { store, action, .. }) => {
                out.push((*store, *action))
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                store_calls_in(then_block, out);
                if let Some(e) = else_block {
                    store_calls_in(e, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Splice { body } => {
                store_calls_in(body, out)
            }
            Stmt::Inspect { arms, .. } => {
                for a in arms {
                    store_calls_in(a.body, out);
                }
            }
            _ => {}
        }
    }
}

/// Whether any action or handler anywhere in `all` queues an element event — the one question the
/// outbox's declaration and the dispatcher's flush arm both ask, so neither exists without the other.
pub(crate) fn uses_outbox(all: &[ComponentDef<'_>]) -> bool {
    all.iter().any(|d| {
        d.actions.iter().any(|a| body_queues_element_event(a.body))
            || d.events.iter().any(|h| body_queues_element_event(h.body))
    })
}

/// [`touches_store`], seeing through `Call <sibling>.` into every action of `def` those statements
/// reach — a sibling runs nested in the caller's turn, so a store it moves is a store the caller's
/// turn moved.
pub(crate) fn touches_store_through(def: &ComponentDef<'_>, stmts: &[Stmt<'_>]) -> bool {
    if touches_store(stmts) {
        return true;
    }
    let mut reached = Vec::new();
    own_action_calls(stmts, def, &mut reached);
    let mut next = 0;
    while next < reached.len() {
        let name = reached[next];
        next += 1;
        let Some(action) = def.actions.iter().find(|a| a.name == name) else {
            continue;
        };
        if touches_store(action.body) {
            return true;
        }
        own_action_calls(action.body, def, &mut reached);
    }
    false
}

/// Whether an `on-<event>.once` handler sits anywhere in `nodes`.
pub(crate) fn nodes_have_once_handler(nodes: &[ViewNode<'_>]) -> bool {
    nodes.iter().any(|node| {
        matches!(node, ViewNode::Handler { modifiers, .. }
            if modifiers.contains(&EventModifier::Behaviour(Behaviour::Once)))
            || view_subtrees(node).into_iter().any(nodes_have_once_handler)
    })
}

/// Whether `def`'s own markup carries an `on-<event>.once` handler — the component whose instances
/// own that listener's spent record.
pub(crate) fn has_once_handler(def: &ComponentDef<'_>) -> bool {
    def.view
        .as_ref()
        .is_some_and(|v| nodes_have_once_handler(&v.nodes))
}

/// Whether any component in the program declares a `.once` handler — the one question the spent map,
/// the listener-to-identity map and every reader of them ask, so none exists without the others.
pub(crate) fn uses_once(all: &[ComponentDef<'_>]) -> bool {
    all.iter().any(has_once_handler)
}

/// Whether this component keeps a live-instance registry (`__KEYS_<C>` / `__SEEN_<C>`).
///
/// ★ ONE PREDICATE, EVERY READER. Four places have to agree about this — the registry
/// declarations, the per-paint STAMP that records an instance as drawn, the unmount SWEEP that
/// walks for instances that were not, and the pass counter both of those read. They were three
/// separate conditions, and the moment one of them said yes while the stamp said no, the sweep
/// saw every LIVE instance as gone and cleared its state on the next paint. A predicate they all
/// ask cannot drift that way.
///
/// State is on the list because an instance that leaves must take its state with it: without a
/// registry nothing notices it vanished, `__S_<C>_<field>` kept the entry, and a remount under the
/// same key resurrected the old value.
pub fn keeps_instance_registry(def: &ComponentDef<'_>) -> bool {
    // ★ ONLY SOMETHING THAT RENDERS CAN BE SWEPT. A registry answers "was this instance drawn in
    // the last paint", and the sweep treats "not drawn" as GONE. A `## Store` has state and no
    // view, so it is never drawn and never stamped — including it here made the sweep wipe every
    // store's state on the first paint. Twenty-one tests said so at once, which is the useful
    // shape of that mistake: a predicate that is nearly right is worse than one that is obviously
    // wrong, because it fails only where the two ideas it conflates come apart.
    if !matches!(
        def.kind,
        ComponentKind::Component | ComponentKind::Page { .. }
    ) {
        return false;
    }
    // A `.once` listener's spent record belongs to its instance: the sweep forgets it when the
    // instance leaves, so a remount starts with the listener armed again.
    def.state.iter().any(|f| !f.derived)
        || has_once_handler(def)
        || def.events.iter().any(|h| {
            // ★ `Appears` IS HERE FOR THE SAME REASON `Disappears` IS. Both ask a question only
            // a registry can answer — "is this the first paint this instance was drawn in" — and
            // the answer is read from `__SEEN_<C>` at opposite ends of an instance's life. Listing
            // only one of them gave a component with `When this component appears:` and no state
            // no presence map at all, so the first-render guard had nothing to consult and the
            // handler never ran.
            // A route change and a published message are delivered to every LIVE instance, which
            // is the registry's question too. `When the page loads:` asks the first-render
            // question too: a placed instance loads the first time it is drawn.
            matches!(
                h.trigger,
                Trigger::Appears
                    | Trigger::PageLoads
                    | Trigger::Disappears
                    | Trigger::FieldChanges { store: Some(_), .. }
                    | Trigger::Every { .. }
                    | Trigger::After { .. }
                    | Trigger::RouteChanges
                    | Trigger::Message { .. }
            )
        })
}

/// A `{for each …:}` with NO `keyed by` whose body reaches a component that owns `State:`.
///
/// ★ SILENT STATE CORRUPTION, IN THE MOST ORDINARY SHAPE A PAGE HAS. Without a key an instance is
/// identified by its POSITION in the list, so the moment the list is reordered or an item is
/// removed every survivor inherits its neighbour's state. Measured on a list of three counters
/// with the second bumped twice and the third once: dropping the FIRST item left the page reading
/// `b:0 c:2` — b took a's count and c took b's — while the keyed spelling of the same page
/// correctly read `b:2 c:1`. Nothing was reported on either tier.
///
/// The framework cannot tell an append-only list (where positional identity is harmless) from one
/// that reorders, so it refuses and names the clause that settles it. That is the same judgement
/// this file makes everywhere else: refused LOUDLY beats rendered wrongly, and a page whose rows
/// quietly swap their contents is as wrong as a page gets.
///
/// TRANSITIVE, because the damage is not limited to the component named in the loop: a stateless
/// `<Panel>` placed unkeyed still carries a stateful `<Tile>` inside it, and the `<Tile>` is the
/// one that loses its identity. It answers the same question `placed_components` answers, one
/// reachability step further out.
#[allow(dead_code)]
pub fn unkeyed_stateful_loop<'a>(
    def: &ComponentDef<'a>,
    all: &[ComponentDef<'a>],
) -> Option<(Symbol, Span)> {
    fn stateful(name: Symbol, all: &[ComponentDef<'_>], seen: &mut Vec<Symbol>) -> Option<Symbol> {
        if seen.contains(&name) {
            return None;
        }
        seen.push(name);
        let def = all.iter().find(|d| d.name == name)?;
        if def.state.iter().any(|f| !f.derived) {
            return Some(name);
        }
        let view = def.view.as_ref()?;
        placed_components(&view.nodes)
            .into_iter()
            .find_map(|c| stateful(c, all, seen))
    }
    fn walk<'a>(nodes: &[ViewNode<'a>], all: &[ComponentDef<'a>]) -> Option<(Symbol, Span)> {
        for node in nodes {
            let found = match node {
                ViewNode::Each {
                    key: None,
                    body,
                    empty,
                    span,
                    ..
                } => placed_components(body)
                    .into_iter()
                    .find_map(|c| stateful(c, all, &mut Vec::new()))
                    .map(|c| (c, *span))
                    .or_else(|| walk(body, all))
                    .or_else(|| walk(empty, all)),
                ViewNode::Each { body, empty, .. } => walk(body, all).or_else(|| walk(empty, all)),
                ViewNode::Element { children, .. } => walk(children, all),
                ViewNode::Component { children, .. } => walk(children, all),
                ViewNode::If {
                    arms, otherwise, ..
                } => arms
                    .iter()
                    .find_map(|(_, b)| walk(b, all))
                    .or_else(|| otherwise.as_ref().and_then(|o| walk(o, all))),
                ViewNode::Inspect { arms, .. } => arms.iter().find_map(|a| walk(&a.body, all)),
                ViewNode::Block {
                    default: Some(b), ..
                } => walk(b, all),
                _ => None,
            };
            if found.is_some() {
                return found;
            }
        }
        None
    }
    walk(&def.view.as_ref()?.nodes, all)
}

/// Which view nodes the server-side HTML walk covers.
///
/// A page rendered short is worse than one refused: the reader sees a plausible document and no
/// error, and the missing piece is whatever the walk did not understand.
pub fn view_html_lowerable(
    nodes: &[ViewNode<'_>],
    interner: &Interner,
) -> Result<(), NotLowerable> {
    view_html_lowerable_with(nodes, &[], interner)
}

/// [`view_html_lowerable`] with the component list, so a CHILD COMPONENT can be checked: a
/// `<TodoRow todo={t}/>` is server-renderable exactly when the child's own view is.
///
/// Without the list a child is refused, which is the honest answer for a caller that cannot see
/// the other components (a `-> View` route body checked on its own).
pub fn view_html_lowerable_with(
    nodes: &[ViewNode<'_>],
    all: &[ComponentDef<'_>],
    interner: &Interner,
) -> Result<(), NotLowerable> {
    view_html_lowerable_seen(nodes, all, interner, &mut Vec::new())
}

pub(crate) fn view_html_lowerable_seen(
    nodes: &[ViewNode<'_>],
    all: &[ComponentDef<'_>],
    interner: &Interner,
    seen: &mut Vec<Symbol>,
) -> Result<(), NotLowerable> {
    for node in nodes {
        let at = node.span();
        match node {
            ViewNode::Element {
                tag,
                attrs,
                children,
                ..
            } => {
                for a in attrs {
                    match a {
                        Attr::Plain { .. } => {}
                        // A `bind` renders the field's current value as the attribute the element
                        // reads it from. Which attribute depends on the element's type, the same
                        // question the browser tier asks; a computed type leaves it undecidable
                        // and is refused.
                        Attr::Bind { .. } => {
                            if let Some(why) =
                                crate::ast::view::bound_input_kind(*tag, attrs, interner).refusal()
                            {
                                return Err(NotLowerable::at(at, why));
                            }
                        }
                        // `class:active={on}` is a class the element carries when `on` holds —
                        // which is a rendering, not a behaviour.
                        Attr::Class { .. } => {}
                        Attr::Directive { .. } => {
                            return Err(NotLowerable::at(
                                at,
                                "its template uses a directive, which is browser behaviour and \
                                 has no server-rendered form",
                            ));
                        }
                    }
                }
                view_html_lowerable_seen(children, all, interner, seen)?;
            }
            ViewNode::Text { .. } => {}
            // Control flow renders on a server exactly as it does in a browser: the same tree,
            // a different destination.
            ViewNode::If {
                arms, otherwise, ..
            } => {
                for (_, body) in arms {
                    view_html_lowerable_seen(body, all, interner, seen)?;
                }
                if let Some(o) = otherwise {
                    view_html_lowerable_seen(o, all, interner, seen)?;
                }
            }
            ViewNode::Each { body, empty, .. } => {
                view_html_lowerable_seen(body, all, interner, seen)?;
                view_html_lowerable_seen(empty, all, interner, seen)?;
            }
            ViewNode::Let { .. } => {}
            // Served markup is inert until hydration attaches the listeners, so a handler
            // contributes nothing to the string.
            ViewNode::Handler { .. } => {}
            // `{inspect}` is `{if}` with a shape to match on.
            ViewNode::Inspect { arms, .. } => {
                for arm in arms {
                    view_html_lowerable_seen(&arm.body, all, interner, seen)?;
                }
            }
            // A placed child lowers to a call to its own `__html_<Name>`, as the browser tier
            // calls its `__render_<Name>`. A wrapper's children belong to the caller: they are
            // walked in the caller's context here and rendered in the caller's scope.
            ViewNode::Children { .. } => {}
            ViewNode::Component { name, children, .. } => {
                view_html_lowerable_seen(children, all, interner, seen)?;
                let Some(child) = all.iter().find(|d| d.name == *name) else {
                    return Err(NotLowerable::at(
                        at,
                        format!(
                        "its template places `<{}/>`, which this renderer cannot see, so whether \
                         it can be server-rendered is unknown",
                        interner.resolve(*name)
                    ),
                    ));
                };
                // A component may place itself (a tree, a thread, a nested menu). A component
                // already on the path answers "can it be server-rendered" the same way the second
                // time, so it is not walked again and the walk terminates on any cycle.
                let repeat = seen.contains(name);
                if !repeat {
                    seen.push(*name);
                }
                match child.view.as_ref() {
                    Some(_) if repeat => {}
                    Some(v) => view_html_lowerable_seen(&v.nodes, all, interner, seen)?,
                    None => {
                        return Err(NotLowerable::at(
                            at,
                            format!(
                                "its template places `<{}/>`, which has no `View:` to render",
                                interner.resolve(*name)
                            ),
                        ))
                    }
                }
            }
            other => {
                return Err(NotLowerable::at(
                    at,
                    format!(
                        "its template uses {}, which the server-side renderer does not cover yet \
                     (elements, text, holes, `{{if}}`, `{{inspect}}`, `{{for each}}`, `{{let}}`, a \
                     placed component, `bind`, a `class:` toggle and an event handler all render \
                     today)",
                        other.construct()
                    ),
                ))
            }
        }
    }
    Ok(())
}

/// Does this view have a `{children}` hole anywhere in it — is this component a WRAPPER?
pub(crate) fn view_takes_children(nodes: &[ViewNode<'_>]) -> bool {
    // A slot can sit inside another placement — `View: <Box>{children}</Box>` passes this
    // component's own children straight through to the one it wraps — which `view_subtrees`
    // walks like any other nesting.
    nodes.iter().any(|n| {
        matches!(n, ViewNode::Children { .. })
            || view_subtrees(n).into_iter().any(view_takes_children)
    })
}

/// Why a `## Route` cannot be lowered to a plain function yet.
///
/// A route body IS a function body, so the ones this understands need nothing new from any tier.
/// The rest are refused BY NAME rather than lowered into something that half-works: a route that
/// answers the wrong thing is worse than one that will not start.
/// The types whose LOGOS display IS valid JSON with no quoting — a number, a boolean.
///
/// ONE definition, read by the record derive and by the router. A route answering an `Int` and a
/// record FIELD holding one have to agree about what reaches the wire, and two lists drift the
/// first time a numeric type is added.
pub fn json_bare_scalar_name(n: &str) -> bool {
    n.eq_ignore_ascii_case("Int")
        || n.eq_ignore_ascii_case("Nat")
        || n.eq_ignore_ascii_case("Bool")
        || n.eq_ignore_ascii_case("Real")
        || n.eq_ignore_ascii_case("Float")
}

/// A route's own heading, as its author wrote it — the one place a route names itself in a
/// refusal, called by both the tree-walker and the VM compiler so the wording cannot diverge.
pub fn route_heading(method: Symbol, path: Symbol, interner: &Interner) -> String {
    format!(
        "## Route {} {}",
        interner.resolve(method),
        interner.resolve(path)
    )
}

/// How an LGX construct is named in a refusal: by the header its author wrote.
///
/// A page's component is interned under a lowered name (`__page__about`) that appears nowhere in
/// the file, so a page is named by its `## Page` header, an error page by its status, and a
/// component by its declaration — called by both the tree-walker and the VM compiler so the
/// wording cannot diverge between them.
pub fn construct_heading(def: &ComponentDef<'_>, interner: &Interner) -> String {
    match &def.kind {
        ComponentKind::Page { path, .. } => format!("## Page {}", interner.resolve(*path)),
        ComponentKind::ErrorPage { status } => format!("## Error page {status}"),
        ComponentKind::Store { .. } => format!("## Store {}", interner.resolve(def.name)),
        ComponentKind::Component => format!("## Component {}", interner.resolve(def.name)),
    }
}

/// A `-> View` function whose body is a template — `## To badge (label: Text) -> View:` with markup
/// under it — as its name, its parameters and its view. The one reading of that shape: the served
/// twin (`lower_view_function_twins`), the server roots a template places, and the browser tier's own
/// lowering of a view function all ask it here.
pub(crate) fn view_template<'s, 'a>(
    stmt: &'s Stmt<'a>,
) -> Option<(
    Symbol,
    &'s [(Symbol, &'a TypeExpr<'a>)],
    &'a crate::ast::view::ViewDef<'a>,
)> {
    let Stmt::FunctionDef {
        name, params, body, ..
    } = stmt
    else {
        return None;
    };
    let body: &'a [Stmt<'a>] = body;
    match body {
        [Stmt::Return {
            value: Some(Expr::View(view)),
        }] => Some((*name, params.as_slice(), &**view)),
        _ => None,
    }
}

/// Every `## Route` this program cannot lower, and why — computed ONCE where the whole program is
/// visible, for the tiers whose per-statement walk cannot see it.
///
/// A route's lowerability depends on the COMPONENTS beside it (a `-> View` template may place
/// one), so it is not a property of the `Stmt::RouteDef` alone. The VM's statement walk and the
/// AOT emitter each reach a route with no program in hand; asking there with an empty component
/// list refuses every route that places a component, which is a loud wrong answer where the parser
/// already knew the right one. Both read this instead.
pub fn route_refusals(
    stmts: &[Stmt<'_>],
    interner: &Interner,
    is_record: &dyn Fn(Symbol) -> bool,
) -> Vec<(Symbol, Symbol, NotLowerable)> {
    let defs: Vec<ComponentDef<'_>> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::ComponentDef(d) => Some((**d).clone()),
            _ => None,
        })
        .collect();
    let mut out = Vec::new();
    for s in stmts {
        let Stmt::RouteDef {
            method,
            path,
            params,
            return_type,
            body,
            span,
        } = s
        else {
            continue;
        };
        if let Err(why) = route_lowerable(
            params,
            *return_type,
            body,
            interner,
            is_record,
            &defs,
            *span,
        ) {
            out.push((*method, *path, why));
        }
    }
    out
}

pub fn route_lowerable(
    params: &[(Symbol, &TypeExpr<'_>)],
    return_type: Option<&TypeExpr<'_>>,
    body: &RouteBody<'_>,
    interner: &Interner,
    is_record: &dyn Fn(Symbol) -> bool,
    components: &[ComponentDef<'_>],
    // Where the route was declared. A route is not a node, so there is nothing to walk for a
    // location — the caller holds the only one, and passing it is what keeps these two refusals
    // from being the pair that still reports at the top of the file.
    at: Span,
) -> Result<(), NotLowerable> {
    if let RouteBody::View(view) = body {
        // A template body renders to HTML TEXT rather than to DOM nodes — the same tree, a
        // different destination. Only the node kinds this walk covers may lower; the rest are
        // refused by name rather than rendered as a page with a piece missing.
        return view_html_lowerable_with(&view.nodes, components, interner);
    }
    for (_, ty) in params {
        // ★ `Request` IS NOT PARSED FROM ANYWHERE — it is the request itself, handed over whole.
        // The other two are read out of the path, the query or the body by name.
        let ok = matches!(ty, TypeExpr::Primitive(s) | TypeExpr::Named(s)
            if interner.resolve(*s).eq_ignore_ascii_case("Text")
                || interner.resolve(*s).eq_ignore_ascii_case("Int")
                || interner.resolve(*s).eq_ignore_ascii_case("Request"));
        if !ok {
            return Err(NotLowerable::at(
                at,
                "one of its parameters is a type the path matcher cannot bind yet (Text, Int and \
                 Request bind today)",
            ));
        }
    }
    // Text answers verbatim; a RECORD answers as JSON through its derived encoder; a `Seq of
    // <record>` answers as a JSON ARRAY through the list encoder emitted beside it — which is what
    // a collection endpoint always is, and the plan's own headline example. Anything else — an
    // Option, an enum — has no encoder yet, and answering something plausible would be worse than
    // saying so.
    match return_type {
        Some(TypeExpr::Primitive(sym) | TypeExpr::Named(sym))
            if interner.resolve(*sym).eq_ignore_ascii_case("Text") =>
        {
            Ok(())
        }
        // A view answers as the markup it renders (`lower_view_function_twins`).
        Some(TypeExpr::Primitive(sym) | TypeExpr::Named(sym))
            if interner.resolve(*sym) == "View" =>
        {
            Ok(())
        }
        Some(TypeExpr::Primitive(sym) | TypeExpr::Named(sym)) if is_record(*sym) => Ok(()),
        // A number or a boolean IS valid JSON printed bare — `42`, `true` — so a count endpoint,
        // a health check or an id lookup answers without a record to wrap it in.
        Some(TypeExpr::Primitive(sym) | TypeExpr::Named(sym))
            if json_bare_scalar_name(interner.resolve(*sym)) =>
        {
            Ok(())
        }
        Some(TypeExpr::Generic { base, params })
            if interner.resolve(*base).eq_ignore_ascii_case("Seq")
                && params.len() == 1
                && matches!(&params[0], TypeExpr::Primitive(s) | TypeExpr::Named(s) if is_record(*s)) =>
        {
            Ok(())
        }
        _ => Err(NotLowerable::at(
            at,
            "it answers with a type the derived JSON codecs do not cover yet (Text, a View, a \
             record, and a Seq of records answer today)",
        )),
    }
}

/// Whether some `## Store` has an action — something that can commit a store while a turn runs,
/// which is what `__LGX_STORES_DIRTY` tracks.
pub(crate) fn commits_stores(all: &[ComponentDef<'_>]) -> bool {
    all.iter()
        .any(|d| matches!(d.kind, ComponentKind::Store { .. }) && !d.actions.is_empty())
}

pub fn stateful(def: &ComponentDef<'_>) -> bool {
    !def.actions.is_empty()
        || !def.events.is_empty()
        || def.view.as_ref().is_some_and(|v| has_bind(&v.nodes))
}

pub(crate) fn has_bind(nodes: &[ViewNode<'_>]) -> bool {
    nodes.iter().any(|node| {
        matches!(node, ViewNode::Element { attrs, .. } if attrs.iter().any(|a| matches!(a, Attr::Bind { .. })))
            || view_subtrees(node).into_iter().any(has_bind)
    })
}

/// Whether this view registers ANY listener (`register_listener_with`'s own gate, read
/// backwards): a `bind={field}` (`ListenerSite::Bind`/`DefaultToggle`) or a real `ViewNode::
/// Handler` (`on-click={…}`/`on click:` — `ListenerSite::Action`, already its own view-tree node
/// by the time this runs, per the parser's own desugaring). A component with neither pays nothing
/// for `__LISTENER_SITES_<C>`'s ledger or the retire/write-back blocks `lower_component_render`
/// would otherwise emit into every render unconditionally — the bundle-size cost
/// `bundle_size_lock` measures on a page that places components which never listen for anything.
pub(crate) fn has_listener_site(nodes: &[ViewNode<'_>]) -> bool {
    nodes.iter().any(|node| {
        let here = match node {
            ViewNode::Element { attrs, .. } => attrs.iter().any(|a| matches!(a, Attr::Bind { .. })),
            ViewNode::Handler { .. } => true,
            _ => false,
        };
        here || view_subtrees(node).into_iter().any(has_listener_site)
    })
}

/// Every node list nested directly under `node` that is drawn in the SAME render function as
/// `node` itself — an element's children, each arm of an `{if}`/`{inspect}`, a loop's body and its
/// empty branch, a block's default, and a placement's own children (which `slot_argument` renders
/// inline, in the placing component's frame). Exhaustive over `ViewNode`, so a walker built on it
/// cannot miss a nesting construct the way an enumerated `_ => false` does.
pub(crate) fn view_subtrees<'n, 'a>(node: &'n ViewNode<'a>) -> Vec<&'n [ViewNode<'a>]> {
    match node {
        ViewNode::Element { children, .. } | ViewNode::Component { children, .. } => vec![children],
        ViewNode::Each { body, empty, .. } => vec![body, empty],
        ViewNode::If {
            arms, otherwise, ..
        } => {
            let mut out: Vec<&'n [ViewNode<'a>]> = arms.iter().map(|(_, b)| &b[..]).collect();
            if let Some(o) = otherwise {
                out.push(o);
            }
            out
        }
        ViewNode::Inspect { arms, .. } => arms.iter().map(|a| &a.body[..]).collect(),
        ViewNode::Block { default, .. } => default.iter().map(|d| &d[..]).collect(),
        ViewNode::Text { .. }
        | ViewNode::Handler { .. }
        | ViewNode::Raw { .. }
        | ViewNode::Let { .. }
        | ViewNode::Extends { .. }
        | ViewNode::Children { .. } => Vec::new(),
    }
}

/// The nodes `def`'s own render function draws: its view. A layout it `{extends}` is an instance
/// drawn by its own render function, as a placed child is.
pub(crate) fn drawn_view<'n, 'a>(def: &'n ComponentDef<'a>) -> &'n [ViewNode<'a>] {
    def.view.as_ref().map_or(&[][..], |v| &v.nodes[..])
}

/// The `{block <name>}` slots `def`'s view declares, in document order, once each — the block
/// parameters its render function takes. A view that `{extends}` a layout declares none: its
/// `{block name:}` bodies FILL the layout's slots.
pub(crate) fn slot_blocks(def: &ComponentDef<'_>) -> Vec<Symbol> {
    fn walk(nodes: &[ViewNode<'_>], out: &mut Vec<Symbol>) {
        for node in nodes {
            if let ViewNode::Block { name, .. } = node {
                if !out.contains(name) {
                    out.push(*name);
                }
            }
            for sub in view_subtrees(node) {
                walk(sub, out);
            }
        }
    }
    let Some(view) = &def.view else {
        return Vec::new();
    };
    if super::render::extended_layout(&view.nodes).is_some() {
        return Vec::new();
    }
    let mut out = Vec::new();
    walk(&view.nodes, &mut out);
    out
}

/// Whether this component registers ANY listener at all — `has_listener_site` over its view, OR a
/// component-level `When … :` handler (`def.events`: a keyed selector, a window event, a message
/// subscription, a key press). The latter registers through the exact same `ListenerSite::Action`
/// path but OUTSIDE the view tree entirely (there is no `ViewNode` for it), so a component whose
/// only listener is one of these was missed by `has_listener_site` alone — the ledger it feeds was
/// gated off while the registration it was built to retire still ran, an undeclared-local wasm trap
/// waiting to happen the day such a component also got no view-level listener.
pub(crate) fn has_any_listener_site(def: &ComponentDef<'_>) -> bool {
    has_listener_site(drawn_view(def)) || !def.events.is_empty()
}

/// Every OWN prop this component's view calls directly as an event handler
/// (`on-click={onGo}`, `docs/frontend.md`'s "Passing actions down" pattern) — the shape whose
/// TARGET varies by placement and needs its own per-instance identity record, since a closure
/// cannot survive in a map the way its identity's two scalar parts can
/// (`events.rs::ListenerSite::CallbackProp`'s own doc). Deduplicated: two elements calling the
/// same prop share one set of bindings (`events.rs::callback_prop_declarations` declares one).
pub(crate) fn callback_prop_targets(def: &ComponentDef<'_>) -> Vec<Symbol> {
    let mut out = Vec::new();
    if let Some(view) = &def.view {
        walk_callback_prop_targets(&view.nodes, def, &mut out);
    }
    out
}

fn walk_callback_prop_targets(
    nodes: &[ViewNode<'_>],
    def: &ComponentDef<'_>,
    out: &mut Vec<Symbol>,
) {
    // A callback can also arrive through a placement's own children, which `slot_argument` lowers
    // inline into THIS render — `view_subtrees` walks them with every other nesting.
    for node in nodes {
        if let ViewNode::Handler { body, .. } = node {
            if let Some(Stmt::Call { function, .. }) = body.first() {
                if def.props.iter().any(|(p, _)| p == function) && !out.contains(function) {
                    out.push(*function);
                }
            }
        }
        for sub in view_subtrees(node) {
            walk_callback_prop_targets(sub, def, out);
        }
    }
}

/// Whether a prop's declared type makes it a CALLBACK: `fn(…) -> …`, or the `Action` spelling the
/// resolver accepts for the same shape (`lgx_resolve`'s `named_shape`).
pub(crate) fn is_callback_type(ty: &TypeExpr<'_>, interner: &Interner) -> bool {
    match ty {
        TypeExpr::Function { .. } => true,
        TypeExpr::Named(s) | TypeExpr::Primitive(s) => interner.resolve(*s) == "Action",
        _ => false,
    }
}

/// Every callback-typed prop `def` declares, in declaration order.
pub(crate) fn callback_props<'a>(
    def: &ComponentDef<'a>,
    interner: &Interner,
) -> Vec<(Symbol, &'a TypeExpr<'a>)> {
    def.props
        .iter()
        .copied()
        .filter(|(_, t)| is_callback_type(t, interner))
        .collect()
}

/// Every `<Child p={v}/>` in `nodes`, however deeply nested — a placement's own children, a loop's
/// body and its `{else:}`, every `{if}` arm, every `{inspect}` arm, a block's default body — with
/// the name it places, the props it passes, and where each prop was written.
pub(crate) fn for_each_placement<'v, 'a>(
    nodes: &'v [ViewNode<'a>],
    f: &mut dyn FnMut(Symbol, &'v [(Symbol, &'a Expr<'a>)], &'v [Span], Span),
) {
    for node in nodes {
        match node {
            ViewNode::Component {
                name,
                props,
                prop_spans,
                children,
                span,
            } => {
                f(*name, props, prop_spans, *span);
                for_each_placement(children, f);
            }
            ViewNode::Element { children, .. } => for_each_placement(children, f),
            ViewNode::Each { body, empty, .. } => {
                for_each_placement(body, f);
                for_each_placement(empty, f);
            }
            ViewNode::If {
                arms, otherwise, ..
            } => {
                for (_, b) in arms {
                    for_each_placement(b, f);
                }
                if let Some(o) = otherwise {
                    for_each_placement(o, f);
                }
            }
            ViewNode::Inspect { arms, .. } => {
                for a in arms {
                    for_each_placement(&a.body, f);
                }
            }
            ViewNode::Block {
                default: Some(d), ..
            } => for_each_placement(d, f),
            _ => {}
        }
    }
}

/// What a callback prop value `value`, written in `placer`'s view, hands down: one of `placer`'s own
/// actions, or one of `placer`'s own callback props forwarded unchanged. Anything else carries no
/// identity the lowered tier can resolve.
pub(crate) enum CallbackSource<'a> {
    Action(&'a crate::ast::component::ActionDef<'a>),
    Forwarded(Symbol),
}

pub(crate) fn callback_source<'d, 'a>(
    value: &Expr<'a>,
    placer: &'d ComponentDef<'a>,
    interner: &Interner,
) -> Option<CallbackSource<'d>>
where
    'a: 'd,
{
    let Expr::Identifier(sym) = value else {
        return None;
    };
    if let Some(action) = placer.actions.iter().find(|a| a.name == *sym) {
        return Some(CallbackSource::Action(action));
    }
    if placer
        .props
        .iter()
        .any(|(p, t)| p == sym && is_callback_type(t, interner))
    {
        return Some(CallbackSource::Forwarded(*sym));
    }
    None
}

/// The parameter types `component`'s callback prop `prop` is called with.
///
/// A `fn(I…) -> O` prop answers its own `I…`. An `Action` prop names no parameters, so its
/// signature is read from what the program hands it: the parameters of the action bound at a
/// placement, or — for a value forwarded from the placer's own callback prop — that prop's
/// signature. `None` only for an `Action` prop no placement binds to anything.
pub(crate) fn callback_signature<'a>(
    all: &[ComponentDef<'a>],
    component: Symbol,
    prop: Symbol,
    interner: &Interner,
) -> Option<Vec<&'a TypeExpr<'a>>> {
    fn go<'a>(
        all: &[ComponentDef<'a>],
        component: Symbol,
        prop: Symbol,
        interner: &Interner,
        visiting: &mut Vec<(Symbol, Symbol)>,
    ) -> Option<Vec<&'a TypeExpr<'a>>> {
        let def = all.iter().find(|d| d.name == component)?;
        let (_, ty) = def.props.iter().find(|(p, _)| *p == prop)?;
        if let TypeExpr::Function { inputs, .. } = ty {
            return Some(inputs.iter().collect());
        }
        if visiting.contains(&(component, prop)) {
            return None;
        }
        visiting.push((component, prop));
        for placer in all {
            let Some(view) = &placer.view else { continue };
            let mut bound: Vec<CallbackSourceOwned> = Vec::new();
            for_each_placement(&view.nodes, &mut |name, props, _, _| {
                if name != component {
                    return;
                }
                let Some((_, value)) = props.iter().find(|(p, _)| *p == prop) else {
                    return;
                };
                match callback_source(value, placer, interner) {
                    Some(CallbackSource::Action(a)) => {
                        bound.push(CallbackSourceOwned::Action(a.name))
                    }
                    Some(CallbackSource::Forwarded(p)) => {
                        bound.push(CallbackSourceOwned::Forwarded(p))
                    }
                    None => {}
                }
            });
            for source in bound {
                let found = match source {
                    CallbackSourceOwned::Action(a) => placer
                        .actions
                        .iter()
                        .find(|x| x.name == a)
                        .map(|x| x.params.iter().map(|(_, t)| *t).collect()),
                    CallbackSourceOwned::Forwarded(p) => {
                        go(all, placer.name, p, interner, visiting)
                    }
                };
                if found.is_some() {
                    return found;
                }
            }
        }
        None
    }
    enum CallbackSourceOwned {
        Action(Symbol),
        Forwarded(Symbol),
    }
    go(all, component, prop, interner, &mut Vec::new())
}

/// Whether two parameter types are the same shape — the question the candidate table asks of every
/// action against a callback's signature.
pub(crate) fn param_shapes_match(a: &TypeExpr<'_>, b: &TypeExpr<'_>) -> bool {
    match (a, b) {
        (
            TypeExpr::Primitive(x) | TypeExpr::Named(x),
            TypeExpr::Primitive(y) | TypeExpr::Named(y),
        ) => x == y,
        (
            TypeExpr::Generic {
                base: b1,
                params: p1,
            },
            TypeExpr::Generic {
                base: b2,
                params: p2,
            },
        ) => {
            b1 == b2
                && p1.len() == p2.len()
                && p1
                    .iter()
                    .zip(p2.iter())
                    .all(|(x, y)| param_shapes_match(x, y))
        }
        (TypeExpr::Tuple(a1), TypeExpr::Tuple(a2)) => {
            a1.len() == a2.len()
                && a1
                    .iter()
                    .zip(a2.iter())
                    .all(|(x, y)| param_shapes_match(x, y))
        }
        (
            TypeExpr::Function {
                inputs: i1,
                output: o1,
            },
            TypeExpr::Function {
                inputs: i2,
                output: o2,
            },
        ) => {
            i1.len() == i2.len()
                && i1
                    .iter()
                    .zip(i2.iter())
                    .all(|(x, y)| param_shapes_match(x, y))
                && param_shapes_match(o1, o2)
        }
        (TypeExpr::Mutable { inner: x }, y) | (y, TypeExpr::Mutable { inner: x }) => {
            param_shapes_match(x, y)
        }
        _ => false,
    }
}

/// Why `def`'s view hands a callback prop something the lowered tier cannot resolve, or `None`.
///
/// A browser calls the closure a render attached; a driving verb, a repaint and a turn cannot, so
/// the lowering records which action a callback means — an identity — at the placement. That
/// identity exists for exactly two values: one of the placer's own actions, whose parameters match
/// the callback's signature, or one of its own callback props forwarded unchanged. Any other value
/// is refused here rather than lowered into a page whose handler answers nothing.
pub(crate) fn callback_binding_refusal(
    def: &ComponentDef<'_>,
    all: &[ComponentDef<'_>],
    interner: &Interner,
) -> Option<NotLowerable> {
    let view = def.view.as_ref()?;
    let mut refusal: Option<NotLowerable> = None;
    for_each_placement(&view.nodes, &mut |name, props, spans, span| {
        if refusal.is_some() {
            return;
        }
        let Some(child) = all.iter().find(|d| d.name == name) else {
            return;
        };
        for (i, (prop, value)) in props.iter().enumerate() {
            let Some((_, ty)) = child.props.iter().find(|(p, _)| p == prop) else {
                continue;
            };
            if !is_callback_type(ty, interner) {
                continue;
            }
            let at = spans.get(i).copied().unwrap_or(span);
            let tag = interner.resolve(name);
            let p = interner.resolve(*prop);
            match callback_source(value, def, interner) {
                None => {
                    refusal = Some(NotLowerable::at(
                        at,
                        format!(
                        "`{p}` of `<{tag}>` is a callback, and this value is neither one of this \
                         component's actions nor a callback prop it was handed — pass one by name: \
                         `{p}={{actionName}}`"
                    ),
                    ));
                    return;
                }
                Some(CallbackSource::Action(action)) => {
                    let Some(sig) = callback_signature(all, name, *prop, interner) else {
                        continue;
                    };
                    let fits = sig.len() == action.params.len()
                        && sig
                            .iter()
                            .zip(action.params.iter())
                            .all(|(s, (_, t))| param_shapes_match(s, t));
                    if !fits {
                        refusal = Some(NotLowerable::at(at, format!(
                            "`{p}` of `<{tag}>` is called with one parameter shape, and `{a}` takes \
                             another; every action handed to one callback must take the same \
                             parameters",
                            a = interner.resolve(action.name)
                        )));
                        return;
                    }
                }
                Some(CallbackSource::Forwarded(_)) => {}
            }
        }
    });
    refusal
}

/// Every action of `def` that `stmts` call by bare name (`Call bump.`), at any depth.
pub(crate) fn own_action_calls(stmts: &[Stmt<'_>], def: &ComponentDef<'_>, out: &mut Vec<Symbol>) {
    for stmt in stmts {
        match stmt {
            Stmt::Call { function, .. } => {
                if def.actions.iter().any(|a| a.name == *function) && !out.contains(function) {
                    out.push(*function);
                }
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                own_action_calls(then_block, def, out);
                if let Some(e) = else_block {
                    own_action_calls(e, def, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Splice { body } => {
                own_action_calls(body, def, out)
            }
            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    own_action_calls(arm.body, def, out);
                }
            }
            _ => {}
        }
    }
}

/// An action of `def` that reaches ITSELF through bare `Call <sibling>.` statements, or `None`.
///
/// The lowered tier runs a sibling call nested in the caller's turn by inlining the sibling's body
/// (`Parser::inline_own_action_call`), and a circle of siblings has no finite inlining, so the
/// component is refused by name.
pub(crate) fn own_action_cycle<'d>(
    def: &'d ComponentDef<'_>,
) -> Option<&'d crate::ast::component::ActionDef<'d>> {
    fn reaches(
        def: &ComponentDef<'_>,
        from: Symbol,
        target: Symbol,
        seen: &mut Vec<Symbol>,
    ) -> bool {
        let Some(action) = def.actions.iter().find(|a| a.name == from) else {
            return false;
        };
        let mut callees = Vec::new();
        own_action_calls(action.body, def, &mut callees);
        for callee in callees {
            if callee == target {
                return true;
            }
            if !seen.contains(&callee) {
                seen.push(callee);
                if reaches(def, callee, target, seen) {
                    return true;
                }
            }
        }
        false
    }
    def.actions
        .iter()
        .find(|a| reaches(def, a.name, a.name, &mut Vec::new()))
}

thread_local! {
    /// The components whose lowerability is being decided right now — see [`lowerable`].
    static CHECKING: std::cell::RefCell<Vec<Symbol>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Leaves the in-progress set however [`lowerable`] returns, `?` included.
pub(crate) struct CheckingGuard;

impl Drop for CheckingGuard {
    fn drop(&mut self) {
        CHECKING.with(|c| {
            c.borrow_mut().pop();
        });
    }
}

/// Whether this component's view is expressible as plain DOM calls.
///
/// `types` is the file's type registry: whether a persisted store's record field, or a typed
/// request's `as <T>`, goes through the derived JSON codec is the codec's own question
/// (`JsonShapes`), asked here of the registry the lowering emits the codec from.
pub fn lowerable<'a>(
    def: &ComponentDef<'a>,
    all: &[ComponentDef<'a>],
    types: Option<&crate::analysis::registry::TypeRegistry>,
    interner: &Interner,
) -> Result<(), NotLowerable> {
    // A component may place itself (`nodes_lowerable` meets `<Tree/>` and asks `lowerable`
    // again). "Can this component be lowered" has one answer per component, so a component
    // already on the path contributes nothing the second time; every other check runs before the
    // descent, so every other reason to refuse is still reached.
    if CHECKING.with(|c| c.borrow().contains(&def.name)) {
        return Ok(());
    }
    CHECKING.with(|c| c.borrow_mut().push(def.name));
    let _guard = CheckingGuard;
    if !matches!(
        def.kind,
        ComponentKind::Component | ComponentKind::Page { .. }
    ) {
        return Err(NotLowerable::at(
            def.span,
            "a store or error page is not a mountable view",
        ));
    }
    let shapes = JsonShapes::new(types, interner);
    for used in &def.uses {
        match all.iter().find(|d| d.name == *used) {
            Some(store) => {
                if let Some(why) = store_not_lowerable(store, &shapes) {
                    return Err(NotLowerable::at(store.span, why));
                }
            }
            None => {
                return Err(NotLowerable::at(
                    def.span,
                    format!(
                        "it uses `{0}`, and this file declares no `## Store {0}`",
                        interner.resolve(*used)
                    ),
                ))
            }
        }
    }
    // Invalid CSS is a refusal, pointed at the character that broke it: the render lowering
    // emits the `<style>` element only when the stylesheet scopes, so lowering past an invalid
    // one would render the page unstyled.
    if let Some(style) = &def.style {
        let class = scope_class(interner.resolve(def.name));
        let css = interner.resolve(style.css);
        if let Err(e) = crate::lgx::css::scope(css, &class) {
            let at = style.file_offset(css, e.at);
            return Err(NotLowerable::at(
                Span::new(at, at + 1),
                format!(
                "its `Style:` block is not valid CSS, so the stylesheet would be dropped and the \
                 page would render unstyled. {}",
                e.message()
            ),
            ));
        }
    }
    // A stateful component's triggers must each have a browser delivery. The allowlist below is
    // every trigger that does, with the host surface it uses; a trigger added to the language
    // without a delivery fails closed here, refused by its written form (`Trigger::form`).
    if stateful(def) {
        for handler in &def.events {
            if !matches!(
                handler.trigger,
                Trigger::Selector { .. }
                    | Trigger::PageLoads
                    | Trigger::Appears
                    // A turn knows what it wrote, so the watched field is compared before and
                    // after (`turn_body`).
                    | Trigger::FieldChanges { .. }
                    // `onRoute(handler)`, a popstate listener in `runtime.js`.
                    | Trigger::RouteChanges
                    // Registered on the page (`lower_component_render`) through `body`,
                    // `addEventListener` and `eventKey`.
                    | Trigger::Key { .. }
                    // A typed fetch's failure paths call the handler with the url and the reason.
                    | Trigger::FetchFails { .. }
                    // A paint stamps every instance it draws with the pass number; the sweep reads
                    // which ones it did not draw.
                    | Trigger::Disappears
                    // One import (`dom_on_window_event`) for all three; `Raise "offline".` causes
                    // one where there is no window.
                    | Trigger::Online
                    | Trigger::Offline
                    | Trigger::WindowResizes
                    // One geometry read (`dom_scroll_gap`); the threshold is compiled LOGOS.
                    | Trigger::ScrollsToBottom { .. }
                    // One import (`dom_set_interval`) for the page's clock; which timer is due is
                    // compiled LOGOS, so `Advance time by …` runs the same arithmetic.
                    | Trigger::Every { .. }
                    | Trigger::After { .. }
                    // A trap unwinds to the host; the module recorded whose turn it was.
                    | Trigger::ActionFails { .. }
                    // One read (`dom_event_file_name`); the handler receives `filedropped`.
                    | Trigger::FileDropped { .. }
                    // One import (`dom_on_message`) subscribing through `logicaffeine_system::relay`;
                    // the topic decision is compiled LOGOS.
                    | Trigger::Message { .. }
            ) {
                return Err(NotLowerable::at(
                    handler.span,
                    format!(
                        "its `{}` sentence waits for something this tier cannot deliver yet, so a \
                     browser would be missing that handler",
                        handler.trigger.form()
                    ),
                ));
            }
        }
    }
    // A typed request whose answer the codec cannot read is refused by its type, rather than
    // lowered into a page whose load never happens.
    let bodies = def
        .actions
        .iter()
        .map(|a| (a.body, a.span))
        .chain(def.events.iter().map(|h| (h.body, h.span)));
    for (body, span) in bodies {
        if let Some(why) = unreadable_fetch(body, &shapes) {
            return Err(NotLowerable::at(span, why));
        }
    }
    let Some(view) = &def.view else {
        return Err(NotLowerable::at(def.span, "it has no `View:` to render"));
    };
    if let Some(why) = callback_binding_refusal(def, all, interner) {
        return Err(why);
    }
    if let Some(action) = own_action_cycle(def) {
        return Err(NotLowerable::at(action.span, format!(
            "its action `{}` reaches itself through `Call <action>.` statements; a sibling call runs \
             nested in the caller's turn, and a circle of them has no end this tier can compile",
            interner.resolve(action.name)
        )));
    }
    nodes_lowerable(&view.nodes, all, types, interner)
}

/// The refusal for the first `Get … as <T>` in `stmts`, at any depth, whose answer the codec
/// cannot read ([`JsonShapes::fetch_read`]).
fn unreadable_fetch(stmts: &[Stmt<'_>], shapes: &JsonShapes<'_>) -> Option<String> {
    let mut refusal = None;
    super::json::typed_requests(stmts, &mut |ty| {
        if refusal.is_none() {
            refusal = shapes.fetch_read(ty).err();
        }
    });
    refusal
}

pub(crate) fn nodes_lowerable<'a>(
    nodes: &[ViewNode<'a>],
    all: &[ComponentDef<'a>],
    types: Option<&crate::analysis::registry::TypeRegistry>,
    interner: &Interner,
) -> Result<(), NotLowerable> {
    for node in nodes {
        let at = node.span();
        match node {
            ViewNode::Element {
                tag,
                attrs,
                children,
                ..
            } => {
                for attr in attrs {
                    match attr {
                        Attr::Plain { .. } => {}
                        // A `bind` writes `value` or `checked` depending on the element's TYPE,
                        // and a computed type leaves that undecidable — refused rather than
                        // guessed, since guessing `value` renders a checkbox the browser draws
                        // unchecked while the page believes it is checked.
                        Attr::Bind { .. } => {
                            // Asked through `BoundInput::refusal`, which is total over the kinds.
                            if let Some(why) =
                                crate::ast::view::bound_input_kind(*tag, attrs, interner).refusal()
                            {
                                return Err(NotLowerable::at(at, why));
                            }
                        }
                        // A directive runs as a function call against its element; a `When removed:`
                        // block runs from the sweep that follows the repaint no longer drawing it.
                        Attr::Class { .. } | Attr::Directive { .. } => {}
                    }
                }
                nodes_lowerable(children, all, types, interner)?;
            }
            ViewNode::Text { .. } => {}
            // `{raw h}` drops trusted markup into the containing element: one `setHtml`.
            ViewNode::Raw { .. } => {}
            ViewNode::If {
                arms, otherwise, ..
            } => {
                for (_, body) in arms {
                    nodes_lowerable(body, all, types, interner)?;
                }
                if let Some(o) = otherwise {
                    nodes_lowerable(o, all, types, interner)?;
                }
            }
            ViewNode::Each { body, empty, .. } => {
                nodes_lowerable(body, all, types, interner)?;
                nodes_lowerable(empty, all, types, interner)?;
            }
            ViewNode::Let { .. } => {}
            ViewNode::Inspect { arms, .. } => {
                for arm in arms {
                    nodes_lowerable(&arm.body, all, types, interner)?;
                }
            }
            // `{extends L}` draws the layout as an instance of its own — its state, actions and
            // registry, keyed like a placement — around the blocks this view draws in its own
            // scope, so the layout lowers exactly when a placement of it would. `{extends}` gives
            // it no props, so a layout that declares one has nothing to read.
            ViewNode::Extends { layout, .. } => match all.iter().find(|d| d.name == *layout) {
                Some(l) if !l.props.is_empty() => {
                    return Err(NotLowerable::at(
                        at,
                        format!(
                        "the layout it extends, `{}`, declares props, and `{{extends}}` has no way \
                         to give them",
                        interner.resolve(*layout)
                    ),
                    ))
                }
                Some(l) => lowerable(l, all, types, interner)?,
                None => {
                    return Err(NotLowerable::at(
                        at,
                        format!(
                            "it extends `{0}`, and this file declares no `## Component {0}`",
                            interner.resolve(*layout)
                        ),
                    ))
                }
            },
            ViewNode::Block { default, .. } => {
                if let Some(d) = default {
                    nodes_lowerable(d, all, types, interner)?;
                }
            }
            // Every qualifier arrives classified by the event-modifier catalogue, and every one
            // lowers: a key travels in the dispatch's value slot from a driving verb and through
            // `eventKey` from a browser; `.once` is compiled LOGOS (a spent record both the
            // dispatcher and the browser's listener consult); `.prevent`, `.stop`, `.capture` and
            // `.passive` are declared at the registration for the host to dispatch by. System keys
            // never reach here: the template parse refuses them.
            ViewNode::Handler { .. } => {}
            // A child is only renderable here if IT is: a parent cannot lower past a child that
            // needs the runtime, or the page would come out missing a piece with nothing said.
            ViewNode::Component { name, children, .. } => {
                let Some(child) = all.iter().find(|d| d.name == *name) else {
                    return Err(NotLowerable::at(
                        at,
                        format!(
                            "it places `<{0}/>`, and this file declares no `## Component {0}`",
                            interner.resolve(*name)
                        ),
                    ));
                };
                lowerable(child, all, types, interner)?;
                // A wrapper's children are checked here, in the caller: they are drawn here and
                // read the caller's scope. The wrapper itself is an instance with its own frame.
                nodes_lowerable(children, all, types, interner)?;
            }
            ViewNode::Children { .. } => {}
        }
    }
    Ok(())
}
