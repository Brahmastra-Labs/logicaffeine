//! Name resolution and placement checks over every LGX view in a program.
//!
//! One pass, run once the whole file is parsed. Every view — a component's `View:` and `Head:`,
//! a `## Route … -> View:` template, a `## To … -> View:` function — is walked with the scope its
//! tiers give it: the component's `State:` (derived fields and page parameters included), its
//! props and actions, the stores it `Use`s, the names the template itself binds (`{for each}`
//! patterns and indexes, `{let}`, `{inspect}` arms, visible exactly where each tier makes them
//! visible), and the program's own definitions (top-level `Let`s, `## To` functions, enum
//! variants, `the session`).
//!
//! What it refuses, each with the construct's own span and a did-you-mean:
//! - a VALUE name that nothing in scope binds — in a hole of any shape, a condition, an iterable, a
//!   key, a `{let}`, an attribute, a class condition, a prop value, a handler argument;
//! - an `on-<event>={name}` whose name is neither an action of the component nor a callback prop;
//! - a `bind={field}` whose field is not the component's own state;
//! - a placed `<Name/>` or `{extends Name}` that names no declared component;
//! - a placement that leaves out a prop its component declares, passes one it does not declare,
//!   or passes a value whose type is known and is not the declared one.
//!
//! The NAME a call is made to (`{format(x)}`, a `| filter`) is not resolved here: a call may reach
//! a kernel builtin or a standard-library function the demand-driven prelude splices in, and
//! neither is visible to the parser. The compile seams resolve call names once the prelude is in.

use super::Parser;
use crate::ast::component::{ComponentDef, ComponentKind, RouteBody};
use crate::ast::stmt::{BinaryOpKind, ClosureBody, Expr, Literal, Pattern, StringPart, TypeExpr};
use crate::ast::view::{Attr, ViewNode};
use crate::ast::Stmt;
use crate::error::{ParseError, ParseErrorKind};
use crate::token::Span;
use logicaffeine_base::Symbol;

/// What a template can know about a value's type without running it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Shape {
    Int,
    Real,
    Text,
    Bool,
    /// A user type or any other named type (`Todo`, `Date`).
    Named(Symbol),
    Seq(Box<Shape>),
    /// A function value: an action passed by name, a `## To` function, a callback prop.
    Callback,
    /// Not known here. Never disagrees with anything.
    Unknown,
}

/// Where a name in a view's scope comes from. Decides which uses are legal: only an `Action` or a
/// callback `Prop` answers an event, only a `State` field takes a `bind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    State,
    Prop,
    Action,
    Store,
    Template,
    Program,
}

#[derive(Clone, Debug)]
struct Binding {
    name: Symbol,
    kind: Kind,
    shape: Shape,
}

/// The scope of one view: frames of bindings, innermost last. A control-flow body (`{for each}`,
/// an `{if}` arm, an `{inspect}` arm, a `{block}` default) opens a frame; element children share
/// their parent's, because both tiers emit a `{let}` inside an element into the enclosing body.
struct Scope {
    frames: Vec<Vec<Binding>>,
}

impl Scope {
    fn find(&self, name: Symbol) -> Option<&Binding> {
        self.frames
            .iter()
            .rev()
            .flat_map(|f| f.iter().rev())
            .find(|b| b.name == name)
    }

    fn bind(&mut self, name: Symbol, kind: Kind, shape: Shape) {
        if let Some(frame) = self.frames.last_mut() {
            frame.push(Binding { name, kind, shape });
        }
    }

    fn open(&mut self) {
        self.frames.push(Vec::new());
    }

    fn close(&mut self) {
        self.frames.pop();
    }

    fn names(&self) -> Vec<Symbol> {
        let mut out: Vec<Symbol> = Vec::new();
        for b in self.frames.iter().rev().flat_map(|f| f.iter().rev()) {
            if !out.contains(&b.name) {
                out.push(b.name);
            }
        }
        out
    }
}

/// The view being walked: whose it is, and the source positions of the names it mentions.
struct ViewCtx<'v, 'a> {
    owner: Option<&'v ComponentDef<'a>>,
    /// How a refusal names this view (`` `App` ``, `` `## Page /about` ``, `` `## Route GET /r` ``).
    label: String,
    mentions: &'v [(Symbol, Span)],
    /// Whether an `on-<event>` in this view can be resolved: a `-> View` function's handlers run
    /// against whichever component renders it, so they are not known here.
    handlers_known: bool,
}

impl<'a, 'ctx, 'int> Parser<'a, 'ctx, 'int> {
    /// Resolve every view in the program. See the module documentation.
    pub(super) fn resolve_views(&self, statements: &[Stmt<'a>]) -> Result<(), ParseError> {
        let defs: Vec<&ComponentDef<'a>> = statements
            .iter()
            .filter_map(|s| match s {
                Stmt::ComponentDef(d) => Some(&**d),
                _ => None,
            })
            .collect();
        let program = self.program_scope(statements);
        for def in &defs {
            let mut scope = Scope {
                frames: vec![program.clone()],
            };
            scope.open();
            self.bind_component(def, &mut scope);
            let label = self.component_label(def);
            // The `View:` and the `Head:` are two templates over the same component scope; what one
            // binds with `{let}` or a loop is not visible in the other.
            for view in def.view.iter().chain(def.head.iter()) {
                let cx = ViewCtx {
                    owner: Some(def),
                    label: label.clone(),
                    mentions: &view.mentions,
                    handlers_known: true,
                };
                scope.open();
                let r = self.resolve_nodes(&view.nodes, &mut scope, &cx, &defs);
                scope.close();
                r?;
            }
        }
        for s in statements {
            match s {
                Stmt::RouteDef {
                    method,
                    path,
                    params,
                    body,
                    ..
                } => {
                    let RouteBody::View(view) = &**body else {
                        continue;
                    };
                    let mut scope = Scope {
                        frames: vec![program.clone()],
                    };
                    scope.open();
                    for (p, ty) in params {
                        scope.bind(*p, Kind::Program, self.shape_of_type(ty));
                    }
                    let label = format!(
                        "`## Route {} {}`",
                        self.interner.resolve(*method).to_uppercase(),
                        self.interner.resolve(*path)
                    );
                    let cx = ViewCtx {
                        owner: None,
                        label,
                        mentions: &view.mentions,
                        handlers_known: false,
                    };
                    self.resolve_nodes(&view.nodes, &mut scope, &cx, &defs)?;
                }
                Stmt::FunctionDef {
                    name, params, body, ..
                } => {
                    let Some(Stmt::Return {
                        value: Some(Expr::View(view)),
                    }) = body.first()
                    else {
                        continue;
                    };
                    let mut scope = Scope {
                        frames: vec![program.clone()],
                    };
                    scope.open();
                    for (p, ty) in params {
                        scope.bind(*p, Kind::Program, self.shape_of_type(ty));
                    }
                    let label = format!("`## To {}`", self.interner.resolve(*name));
                    let cx = ViewCtx {
                        owner: None,
                        label,
                        mentions: &view.mentions,
                        handlers_known: false,
                    };
                    self.resolve_nodes(&view.nodes, &mut scope, &cx, &defs)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// The program-level names every view may read.
    fn program_scope(&self, statements: &[Stmt<'a>]) -> Vec<Binding> {
        let mut out = Vec::new();
        let mut push = |name: Symbol, shape: Shape| {
            out.push(Binding {
                name,
                kind: Kind::Program,
                shape,
            })
        };
        for s in statements {
            match s {
                Stmt::Let { var, ty, value, .. } => {
                    let shape = match ty {
                        Some(t) => self.shape_of_type(t),
                        None => self.shape_of_literalish(value),
                    };
                    push(*var, shape);
                }
                Stmt::Mount { var, .. } => push(*var, Shape::Unknown),
                Stmt::FunctionDef { name, .. } => push(*name, Shape::Callback),
                _ => {}
            }
        }
        // The temporal names every tier reads as values (`today`, `now`), and the session record
        // a `## Session` declares.
        let session = if self.session_declared {
            Some("session")
        } else {
            None
        };
        for special in ["today", "now"].into_iter().chain(session) {
            if let Some(sym) = self.interner.lookup(special) {
                push(sym, Shape::Unknown);
            }
        }
        out
    }

    /// A component's own names: state (page parameters and derived fields included), props,
    /// actions, and the stores it uses.
    fn bind_component(&self, def: &ComponentDef<'a>, scope: &mut Scope) {
        for f in &def.state {
            let shape = match f.ty {
                Some(t) => self.shape_of_type(t),
                None => self.shape_of_literalish(f.init),
            };
            scope.bind(f.name, Kind::State, shape);
        }
        for (p, ty) in &def.props {
            scope.bind(*p, Kind::Prop, self.shape_of_type(ty));
        }
        for a in &def.actions {
            scope.bind(a.name, Kind::Action, Shape::Callback);
        }
        for u in &def.uses {
            scope.bind(*u, Kind::Store, Shape::Named(*u));
        }
    }

    /// How a refusal names a component: its own name, or the header of a page or error page,
    /// whose internal names (`__page__about`) appear nowhere in the source.
    pub(super) fn component_label(&self, def: &ComponentDef<'a>) -> String {
        def.label(self.interner)
    }

    fn resolve_nodes(
        &self,
        nodes: &[ViewNode<'a>],
        scope: &mut Scope,
        cx: &ViewCtx<'_, 'a>,
        defs: &[&ComponentDef<'a>],
    ) -> Result<(), ParseError> {
        for node in nodes {
            self.resolve_node(node, scope, cx, defs)?;
        }
        Ok(())
    }

    fn resolve_node(
        &self,
        node: &ViewNode<'a>,
        scope: &mut Scope,
        cx: &ViewCtx<'_, 'a>,
        defs: &[&ComponentDef<'a>],
    ) -> Result<(), ParseError> {
        let at = node.span();
        match node {
            ViewNode::Element {
                attrs, children, ..
            } => {
                for a in attrs {
                    match a {
                        Attr::Plain { value, .. } | Attr::Directive { value, .. } => {
                            self.resolve_expr(value, scope, cx, at)?
                        }
                        Attr::Class { cond, .. } => self.resolve_expr(cond, scope, cx, at)?,
                        Attr::Bind { field } => self.resolve_bind(*field, scope, cx, at)?,
                    }
                }
                self.resolve_nodes(children, scope, cx, defs)
            }
            ViewNode::Text { text, .. } => self.resolve_expr(text, scope, cx, at),
            ViewNode::Raw { html, .. } => self.resolve_expr(html, scope, cx, at),
            ViewNode::Children { .. } => Ok(()),
            ViewNode::Handler { body, span, .. } => self.resolve_handler(body, scope, cx, *span),
            ViewNode::If {
                arms, otherwise, ..
            } => {
                for (cond, body) in arms {
                    self.resolve_expr(cond, scope, cx, at)?;
                    scope.open();
                    let r = self.resolve_nodes(body, scope, cx, defs);
                    scope.close();
                    r?;
                }
                if let Some(o) = otherwise {
                    scope.open();
                    let r = self.resolve_nodes(o, scope, cx, defs);
                    scope.close();
                    r?;
                }
                Ok(())
            }
            ViewNode::Each {
                pattern,
                iterable,
                key,
                index,
                body,
                empty,
                ..
            } => {
                self.resolve_expr(iterable, scope, cx, at)?;
                let item = match self.shape_of(iterable, scope) {
                    Shape::Seq(inner) => *inner,
                    _ => Shape::Unknown,
                };
                scope.open();
                match pattern {
                    Pattern::Identifier(v) => scope.bind(*v, Kind::Template, item),
                    Pattern::Tuple(vs) => {
                        for v in vs {
                            scope.bind(*v, Kind::Template, Shape::Unknown);
                        }
                    }
                }
                if let Some(i) = index {
                    scope.bind(*i, Kind::Template, Shape::Int);
                }
                let r = match key {
                    Some(k) => self
                        .resolve_expr(k, scope, cx, at)
                        .and_then(|_| self.resolve_nodes(body, scope, cx, defs)),
                    None => self.resolve_nodes(body, scope, cx, defs),
                };
                scope.close();
                r?;
                scope.open();
                let r = self.resolve_nodes(empty, scope, cx, defs);
                scope.close();
                r
            }
            ViewNode::Inspect {
                scrutinee, arms, ..
            } => {
                self.resolve_expr(scrutinee, scope, cx, at)?;
                for arm in arms {
                    scope.open();
                    for (_, b) in &arm.bindings {
                        scope.bind(*b, Kind::Template, Shape::Unknown);
                    }
                    let r = self.resolve_nodes(&arm.body, scope, cx, defs);
                    scope.close();
                    r?;
                }
                Ok(())
            }
            ViewNode::Let { var, value, .. } => {
                self.resolve_expr(value, scope, cx, at)?;
                let shape = self.shape_of(value, scope);
                scope.bind(*var, Kind::Template, shape);
                Ok(())
            }
            ViewNode::Extends { layout, span } => {
                if defs.iter().any(|d| d.name == *layout) {
                    return Ok(());
                }
                Err(self.unknown_component(*layout, *span, defs, false))
            }
            ViewNode::Block { default, .. } => {
                if let Some(d) = default {
                    scope.open();
                    let r = self.resolve_nodes(d, scope, cx, defs);
                    scope.close();
                    r?;
                }
                Ok(())
            }
            ViewNode::Component {
                name,
                props,
                prop_spans,
                children,
                span,
            } => {
                self.resolve_placement(*name, props, prop_spans, *span, scope, cx, defs)?;
                self.resolve_nodes(children, scope, cx, defs)
            }
        }
    }

    /// `on-<event>={name(args)}`: the name is an action of this component or a callback prop it
    /// was handed; the arguments are values in scope.
    fn resolve_handler(
        &self,
        body: &crate::ast::stmt::Block<'a>,
        scope: &mut Scope,
        cx: &ViewCtx<'_, 'a>,
        span: Span,
    ) -> Result<(), ParseError> {
        let Some(Stmt::Call { function, args }) = body.first() else {
            return Ok(());
        };
        for a in args {
            self.resolve_expr(a, scope, cx, span)?;
        }
        if !cx.handlers_known {
            return Ok(());
        }
        let Some(owner) = cx.owner else { return Ok(()) };
        let found = scope.find(*function).map(|b| (b.kind, b.shape.clone()));
        match found {
            Some((Kind::Action, _)) => Ok(()),
            Some((Kind::Prop, Shape::Callback | Shape::Unknown)) => Ok(()),
            Some((Kind::Prop, other)) => Err(ParseError {
                kind: ParseErrorKind::Custom(format!(
                    "`{name}` is a prop of {label} holding a `{shape}`, so an event has nothing to \
                     run. An `on-<event>={{…}}` names an action of {label}, or a callback prop \
                     declared `fn(…) -> Unit`.",
                    name = self.interner.resolve(*function),
                    shape = self.shape_text(&other),
                    label = cx.label,
                )),
                span: self.mention_in(*function, span, cx),
            }),
            _ => {
                let actions: Vec<&str> = owner
                    .actions
                    .iter()
                    .map(|a| self.interner.resolve(a.name))
                    .collect();
                let callbacks: Vec<&str> = owner
                    .props
                    .iter()
                    .filter(|(_, t)| matches!(t, TypeExpr::Function { .. }))
                    .map(|(p, _)| self.interner.resolve(*p))
                    .collect();
                let written = self.interner.resolve(*function);
                let guess = crate::suggest::did_you_mean(
                    written,
                    actions.iter().chain(callbacks.iter()).copied(),
                )
                .map(|g| format!(" — did you mean `{g}`?"))
                .unwrap_or_default();
                let have = if actions.is_empty() {
                    format!("{} declares no actions", cx.label)
                } else {
                    format!("its actions are {}", backticked(&actions))
                };
                Err(ParseError {
                    kind: ParseErrorKind::Custom(format!(
                        "`{written}` is not an action of {label}{guess}{stop} An event runs one of the \
                         component's own actions, or a callback prop it was handed — {have}.",
                        label = cx.label,
                        stop = if guess.is_empty() { "." } else { "" },
                    )),
                    span: self.mention_in(*function, span, cx),
                })
            }
        }
    }

    /// A bare `Call <name>.` (no possessive) inside a component's own `Actions:`/event body,
    /// misspelling one of the SAME component's own OTHER actions — `Call bum.` where the
    /// component declares `To bump:`.
    ///
    /// ★ IT NEVER REFUSES A NAME IT MERELY DOES NOT RECOGNISE. This pass cannot see every name a
    /// `Call` might legally reach — a kernel builtin, or a standard-library function the
    /// demand-driven prelude splices into the TEXT before this parse ever runs (see the module
    /// doc) — so treating "not found here" as "refuse" would put a false error on a legitimate
    /// stdlib call. It refuses only a name that is ALSO a close typo of a sibling action, which a
    /// real function name essentially never is at the same time.
    ///
    /// An EXACT match against a sibling action, or against a top-level `## To` function this same
    /// file declares, needs no refusal at all: the exact match is the decided shape for "the
    /// component's own action runs by its bare name", and
    /// running it is the tree-walker's and the lowering's job, not this check's.
    pub(super) fn resolve_bare_action_typos(
        &self,
        statements: &[Stmt<'a>],
    ) -> Result<(), ParseError> {
        let top_level: Vec<Symbol> = statements
            .iter()
            .filter_map(|s| match s {
                Stmt::FunctionDef { name, .. } => Some(*name),
                _ => None,
            })
            .collect();
        for s in statements {
            let Stmt::ComponentDef(def) = s else { continue };
            for action in &def.actions {
                self.check_bare_action_typos_in(action.body, def, &top_level, action.span)?;
            }
            for handler in &def.events {
                self.check_bare_action_typos_in(handler.body, def, &top_level, handler.span)?;
            }
        }
        Ok(())
    }

    fn check_bare_action_typos_in(
        &self,
        block: crate::ast::stmt::Block<'a>,
        owner: &ComponentDef<'a>,
        top_level: &[Symbol],
        at: Span,
    ) -> Result<(), ParseError> {
        for st in block {
            match st {
                Stmt::Call { function, .. } => {
                    if owner.actions.iter().any(|a| a.name == *function) {
                        continue;
                    }
                    if top_level.contains(function) {
                        continue;
                    }
                    let written = self.interner.resolve(*function);
                    let names: Vec<&str> = owner
                        .actions
                        .iter()
                        .map(|a| self.interner.resolve(a.name))
                        .collect();
                    let Some(guess) = crate::suggest::did_you_mean(written, names.iter().copied())
                    else {
                        continue;
                    };
                    let heading = crate::parser::lgx_lower::construct_heading(owner, self.interner);
                    return Err(ParseError {
                        kind: ParseErrorKind::Custom(format!(
                            "`{written}` is not an action of {heading} — did you mean `{guess}`? A \
                             bare `Call <name>.` inside {heading}'s own `Actions:` runs one of its \
                             OWN actions, nested in the caller's turn, exactly as `Call <Store>'s \
                             <action>.` runs a store's."
                        )),
                        span: at,
                    });
                }
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    self.check_bare_action_typos_in(then_block, owner, top_level, at)?;
                    if let Some(e) = else_block {
                        self.check_bare_action_typos_in(e, owner, top_level, at)?;
                    }
                }
                Stmt::While { body, .. } => {
                    self.check_bare_action_typos_in(body, owner, top_level, at)?;
                }
                Stmt::Repeat { body, .. } => {
                    self.check_bare_action_typos_in(body, owner, top_level, at)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// `bind={field}`: a field of this component's own `State:`.
    fn resolve_bind(
        &self,
        field: Symbol,
        scope: &Scope,
        cx: &ViewCtx<'_, 'a>,
        at: Span,
    ) -> Result<(), ParseError> {
        let Some(owner) = cx.owner else { return Ok(()) };
        let written = self.interner.resolve(field);
        let span = self.mention_in(field, at, cx);
        match scope.find(field).map(|b| b.kind) {
            Some(Kind::State) => Ok(()),
            Some(Kind::Prop) => Err(ParseError {
                kind: ParseErrorKind::Custom(format!(
                    "`bind={{{written}}}` would write `{written}`, and `{written}` is a prop of {label} — \
                     its value is whatever the parent passed, so this component cannot change it. \
                     Keep the value in `State:` (here, or in the parent with a callback prop).",
                    label = cx.label,
                )),
                span,
            }),
            _ => {
                let fields: Vec<&str> = owner.state.iter().filter(|f| !f.derived).map(|f| self.interner.resolve(f.name)).collect();
                let guess = crate::suggest::did_you_mean(written, fields.iter().copied())
                    .map(|g| format!(" — did you mean `{g}`?"))
                    .unwrap_or_default();
                let have = if fields.is_empty() {
                    "it declares no state".to_string()
                } else {
                    format!("its state is {}", backticked(&fields))
                };
                Err(ParseError {
                    kind: ParseErrorKind::Custom(format!(
                        "`bind={{{written}}}` names no field of {label}'s `State:`{guess}. A binding \
                         writes what the visitor types into a state field — {have}.",
                        label = cx.label,
                    )),
                    span,
                })
            }
        }
    }

    /// One placement: the component exists, every declared prop is passed, nothing undeclared
    /// is, and each value's known type is the declared one.
    #[allow(clippy::too_many_arguments)]
    fn resolve_placement(
        &self,
        name: Symbol,
        props: &[(Symbol, &'a Expr<'a>)],
        prop_spans: &[Span],
        span: Span,
        scope: &mut Scope,
        cx: &ViewCtx<'_, 'a>,
        defs: &[&ComponentDef<'a>],
    ) -> Result<(), ParseError> {
        let prop_at = |i: usize| prop_spans.get(i).copied().unwrap_or(span);
        for (i, (_, value)) in props.iter().enumerate() {
            self.resolve_expr(value, scope, cx, prop_at(i))?;
        }
        let Some(child) = defs.iter().find(|d| d.name == name) else {
            return Err(self.unknown_component(name, span, defs, true));
        };
        let tag = self.interner.resolve(name);
        if !matches!(child.kind, ComponentKind::Component) {
            return Err(ParseError {
                kind: ParseErrorKind::Custom(format!(
                    "`<{tag}>` places {label}, which is not a component a view can place — only a \
                     `## Component` is.",
                    label = self.component_label(child),
                )),
                span: self.tag_name_span(span, tag),
            });
        }
        // A placed component draws inside a page's document and owns none of it: a `Head:` belongs
        // to the page, or to the layout the page extends.
        if child.head.is_some() {
            return Err(ParseError {
                kind: ParseErrorKind::Custom(format!(
                    "`<{tag}/>` places `{tag}`, which carries a `Head:`. A placed component draws \
                     inside a page's document and owns none of its head — two of them naming a title \
                     would leave the document's title to render order. Move these tags to the page \
                     that places `{tag}`, or to a layout the page extends (`{{extends Layout}}`), whose \
                     `Head:` every page extending it shares."
                )),
                span: self.tag_name_span(span, tag),
            });
        }
        let declared: Vec<String> = child
            .props
            .iter()
            .map(|(p, t)| format!("{}: {}", self.interner.resolve(*p), self.type_text(t)))
            .collect();
        for (i, (given, _)) in props.iter().enumerate() {
            if child.props.iter().any(|(p, _)| p == given) {
                continue;
            }
            let written = self.interner.resolve(*given);
            let names: Vec<&str> = child
                .props
                .iter()
                .map(|(p, _)| self.interner.resolve(*p))
                .collect();
            let guess = crate::suggest::did_you_mean(written, names.iter().copied())
                .map(|g| format!(" — did you mean `{g}`?"))
                .unwrap_or_default();
            let has = if declared.is_empty() {
                format!("`{tag}` declares no props")
            } else {
                format!(
                    "`{tag}` declares {}",
                    backticked(&declared.iter().map(String::as_str).collect::<Vec<_>>())
                )
            };
            return Err(ParseError {
                kind: ParseErrorKind::Custom(format!(
                    "`<{tag}>` has no prop `{written}`{guess}. {has}; an attribute on a component is \
                     a prop, and one it does not declare would be ignored."
                )),
                span: prop_at(i),
            });
        }
        for (p, ty) in &child.props {
            let Some(i) = props.iter().position(|(given, _)| given == p) else {
                return Err(ParseError {
                    kind: ParseErrorKind::Custom(format!(
                        "`<{tag}>` does not pass `{prop}`, which `{header}` declares. A placement \
                         passes every prop its component declares: add `{prop}={{…}}`.",
                        prop = self.interner.resolve(*p),
                        header = self.component_header(child),
                    )),
                    span: self.tag_name_span(span, tag),
                });
            };
            let want = self.shape_of_type(ty);
            let got = self.shape_of(props[i].1, scope);
            if shapes_agree(&want, &got) {
                continue;
            }
            let prop = self.interner.resolve(*p);
            let message = if want == Shape::Callback {
                format!(
                    "`{prop}` of `<{tag}>` is a callback (`{ty}`), and this value is a `{got}`. Pass \
                     one of this component's actions by name: `{prop}={{actionName}}`.",
                    ty = self.type_text(ty),
                    got = self.shape_text(&got),
                )
            } else {
                format!(
                    "`{prop}` of `<{tag}>` is a `{want}`, and this value is a `{got}`.",
                    want = self.type_text(ty),
                    got = self.shape_text(&got),
                )
            };
            return Err(ParseError {
                kind: ParseErrorKind::Custom(message),
                span: prop_at(i),
            });
        }
        Ok(())
    }

    /// A placed (`<Nav/>`) or extended (`{extends Nav}`) name no component answers to.
    fn unknown_component(
        &self,
        name: Symbol,
        span: Span,
        defs: &[&ComponentDef<'a>],
        placed: bool,
    ) -> ParseError {
        let written = self.interner.resolve(name);
        let mut known: Vec<&str> = defs
            .iter()
            .filter(|d| matches!(d.kind, ComponentKind::Component))
            .map(|d| self.interner.resolve(d.name))
            .collect();
        known.sort_unstable();
        let guess = crate::suggest::did_you_mean(written, known.iter().copied())
            .map(|g| format!(" — did you mean `{g}`?"))
            .unwrap_or_default();
        let declared = if known.is_empty() {
            "This file declares no components.".to_string()
        } else {
            format!("This file declares {}.", backticked(&known))
        };
        let (construct, declare) = if placed {
            (format!("`<{written}/>`"), format!("Declare it as `## Component {written}` with a `View:`, or place one this file has."))
        } else {
            (
                format!("`{{extends {written}}}`"),
                format!("Write the layout as `## Component {written}` with the `{{block …}}` slots this view fills."),
            )
        };
        ParseError {
            kind: ParseErrorKind::Custom(format!(
                "{construct} names no component{guess}{stop} {declared} {declare}",
                stop = if guess.is_empty() { "." } else { "" },
            )),
            span: if placed {
                self.tag_name_span(span, written)
            } else {
                span
            },
        }
    }

    /// Every value name in `expr` resolves in `scope`.
    fn resolve_expr(
        &self,
        expr: &Expr<'a>,
        scope: &mut Scope,
        cx: &ViewCtx<'_, 'a>,
        at: Span,
    ) -> Result<(), ParseError> {
        match expr {
            Expr::Identifier(sym) => {
                if scope.find(*sym).is_some() || self.is_variant(*sym) {
                    return Ok(());
                }
                Err(self.unresolved(*sym, scope, cx, at))
            }
            Expr::Literal(_) | Expr::OptionNone | Expr::Escape { .. } => Ok(()),
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
                self.resolve_expr(left, scope, cx, at)?;
                self.resolve_expr(right, scope, cx, at)
            }
            Expr::Not { operand }
            | Expr::Copy { expr: operand }
            | Expr::Give { value: operand }
            | Expr::Length {
                collection: operand,
            }
            | Expr::ManifestOf { zone: operand }
            | Expr::OptionSome { value: operand } => self.resolve_expr(operand, scope, cx, at),
            Expr::Slice {
                collection,
                start,
                end,
            } => {
                self.resolve_expr(collection, scope, cx, at)?;
                self.resolve_expr(start, scope, cx, at)?;
                self.resolve_expr(end, scope, cx, at)
            }
            Expr::Call { args, .. } => args
                .iter()
                .try_for_each(|a| self.resolve_expr(a, scope, cx, at)),
            Expr::CallExpr { callee, args } => {
                self.resolve_expr(callee, scope, cx, at)?;
                args.iter()
                    .try_for_each(|a| self.resolve_expr(a, scope, cx, at))
            }
            Expr::List(items) | Expr::Tuple(items) => items
                .iter()
                .try_for_each(|i| self.resolve_expr(i, scope, cx, at)),
            Expr::FieldAccess { object, .. } => self.resolve_expr(object, scope, cx, at),
            Expr::New { init_fields, .. } => init_fields
                .iter()
                .try_for_each(|(_, e)| self.resolve_expr(e, scope, cx, at)),
            Expr::NewVariant { fields, .. } => fields
                .iter()
                .try_for_each(|(_, e)| self.resolve_expr(e, scope, cx, at)),
            // A closure's parameters are in scope for its expression body. A statement body
            // brings its own `Let`s and loops, which this pass does not follow; it is left to the
            // tiers rather than guessed at.
            Expr::Closure { params, body, .. } => match body {
                ClosureBody::Expression(e) => {
                    scope.open();
                    for (p, ty) in params {
                        scope.bind(*p, Kind::Template, self.shape_of_type(ty));
                    }
                    let r = self.resolve_expr(e, scope, cx, at);
                    scope.close();
                    r
                }
                ClosureBody::Block(_) => Ok(()),
            },
            Expr::InterpolatedString(parts) => parts.iter().try_for_each(|p| match p {
                StringPart::Expr { value, .. } => self.resolve_expr(value, scope, cx, at),
                StringPart::Literal(_) => Ok(()),
            }),
            Expr::View(_) => Ok(()),
            Expr::Dom(q) => match q.selector() {
                Some(sel) => self.resolve_expr(sel, scope, cx, at),
                None => Ok(()),
            },
        }
    }

    fn is_variant(&self, sym: Symbol) -> bool {
        self.type_registry
            .as_ref()
            .is_some_and(|r| r.is_variant(sym))
    }

    /// The refusal for a value name nothing binds.
    fn unresolved(&self, sym: Symbol, scope: &Scope, cx: &ViewCtx<'_, 'a>, at: Span) -> ParseError {
        let written = self.interner.resolve(sym);
        let in_scope: Vec<&str> = scope
            .names()
            .into_iter()
            .map(|n| self.interner.resolve(n))
            .collect();
        let guess = crate::suggest::did_you_mean(written, in_scope.iter().copied())
            .map(|g| format!(" — did you mean `{g}`?"))
            .unwrap_or_default();
        let store_hint = if self.lgx_stores.contains_key(&sym) {
            format!(" `{written}` is a store: read it after `Use {written}.` in the component.")
        } else {
            String::new()
        };
        ParseError {
            kind: ParseErrorKind::Custom(format!(
                "`{written}` is not defined in {label}'s view{guess}{stop}{store_hint} A view reads its \
                 component's `State:` and props, the names its template binds (`{{for each}}`, \
                 `{{let}}`, `{{inspect}}`), the stores it uses, and the program's own definitions.",
                label = cx.label,
                stop = if guess.is_empty() { "." } else { "" },
            )),
            span: self.mention_in(sym, at, cx),
        }
    }

    /// Where `sym` was written inside the node spanning `at`: the first mention of it there, or the
    /// node itself.
    fn mention_in(&self, sym: Symbol, at: Span, cx: &ViewCtx<'_, 'a>) -> Span {
        cx.mentions
            .iter()
            .find(|(s, sp)| *s == sym && sp.start >= at.start && sp.end <= at.end)
            .map(|(_, sp)| *sp)
            .unwrap_or(at)
    }

    /// The span of a placement's tag name: `Row` in `<Row …/>`.
    fn tag_name_span(&self, element: Span, tag: &str) -> Span {
        Span::new(element.start + 1, element.start + 1 + tag.len())
    }

    /// `## Component Row (label: Text) and (done: Bool)`, as the header was written.
    fn component_header(&self, def: &ComponentDef<'a>) -> String {
        let name = self.interner.resolve(def.name);
        if def.props.is_empty() {
            return format!("## Component {name}");
        }
        let groups: Vec<String> = def
            .props
            .iter()
            .map(|(p, t)| format!("({}: {})", self.interner.resolve(*p), self.type_text(t)))
            .collect();
        format!("## Component {name} {}", groups.join(" and "))
    }

    fn type_text(&self, ty: &TypeExpr<'a>) -> String {
        match ty {
            TypeExpr::Primitive(s) | TypeExpr::Named(s) => self.interner.resolve(*s).to_string(),
            TypeExpr::Generic { base, params } => {
                let ps: Vec<String> = params.iter().map(|p| self.type_text(p)).collect();
                format!("{} of {}", self.interner.resolve(*base), ps.join(" and "))
            }
            TypeExpr::Function { inputs, output } => {
                let is: Vec<String> = inputs.iter().map(|p| self.type_text(p)).collect();
                format!("fn({}) -> {}", is.join(", "), self.type_text(output))
            }
            TypeExpr::Mutable { inner } | TypeExpr::Persistent { inner } => self.type_text(inner),
            TypeExpr::Refinement { base, .. } => self.type_text(base),
            TypeExpr::Tuple(items) => {
                let is: Vec<String> = items.iter().map(|p| self.type_text(p)).collect();
                format!("({})", is.join(", "))
            }
            TypeExpr::Record {
                field_names,
                field_types,
            } => {
                let fs: Vec<String> = field_names
                    .iter()
                    .zip(field_types.iter())
                    .map(|(n, t)| format!("{}: {}", self.interner.resolve(*n), self.type_text(t)))
                    .collect();
                format!("({})", fs.join(", "))
            }
        }
    }

    fn shape_text(&self, shape: &Shape) -> String {
        match shape {
            Shape::Int => "Int".into(),
            Shape::Real => "Real".into(),
            Shape::Text => "Text".into(),
            Shape::Bool => "Bool".into(),
            Shape::Named(s) => self.interner.resolve(*s).to_string(),
            Shape::Seq(inner) => format!("Seq of {}", self.shape_text(inner)),
            Shape::Callback => "callback".into(),
            Shape::Unknown => "value".into(),
        }
    }

    /// The shape a declared type promises.
    pub(crate) fn shape_of_type(&self, ty: &TypeExpr<'a>) -> Shape {
        match ty {
            TypeExpr::Primitive(s) | TypeExpr::Named(s) => self.named_shape(*s),
            TypeExpr::Generic { base, params } => match self.interner.resolve(*base) {
                "Seq" | "List" if params.len() == 1 => {
                    Shape::Seq(Box::new(self.shape_of_type(&params[0])))
                }
                _ => Shape::Unknown,
            },
            TypeExpr::Function { .. } => Shape::Callback,
            TypeExpr::Mutable { inner } => self.shape_of_type(inner),
            TypeExpr::Refinement { base, .. } => self.shape_of_type(base),
            _ => Shape::Unknown,
        }
    }

    fn named_shape(&self, s: Symbol) -> Shape {
        match self.interner.resolve(s) {
            "Int" | "Nat" | "Byte" => Shape::Int,
            "Real" | "Float" => Shape::Real,
            "Text" | "String" => Shape::Text,
            "Bool" | "Boolean" => Shape::Bool,
            // `Action` is the type a component's action has when it is handed to a child.
            "Action" => Shape::Callback,
            _ => Shape::Named(s),
        }
    }

    /// The shape of a `State:` or program initializer, read without a scope.
    fn shape_of_literalish(&self, expr: &Expr<'a>) -> Shape {
        self.shape_of(expr, &Scope { frames: Vec::new() })
    }

    /// What `expr` is known to be, given `scope`.
    fn shape_of(&self, expr: &Expr<'a>, scope: &Scope) -> Shape {
        match expr {
            Expr::Literal(Literal::Number(_)) => Shape::Int,
            Expr::Literal(Literal::Float(_)) => Shape::Real,
            Expr::Literal(Literal::Text(_)) => Shape::Text,
            Expr::Literal(Literal::Boolean(_)) => Shape::Bool,
            Expr::InterpolatedString(_) => Shape::Text,
            Expr::Identifier(sym) => match scope.find(*sym) {
                Some(b) => b.shape.clone(),
                None => Shape::Unknown,
            },
            Expr::Not { .. } | Expr::Contains { .. } => Shape::Bool,
            Expr::Length { .. } => Shape::Int,
            Expr::BinaryOp { op, left, right } => match op {
                BinaryOpKind::Eq
                | BinaryOpKind::NotEq
                | BinaryOpKind::Lt
                | BinaryOpKind::Gt
                | BinaryOpKind::LtEq
                | BinaryOpKind::GtEq
                | BinaryOpKind::ApproxEq => Shape::Bool,
                BinaryOpKind::And | BinaryOpKind::Or => {
                    match (self.shape_of(left, scope), self.shape_of(right, scope)) {
                        (Shape::Bool, Shape::Bool) => Shape::Bool,
                        _ => Shape::Unknown,
                    }
                }
                BinaryOpKind::Concat => Shape::Text,
                BinaryOpKind::Add
                | BinaryOpKind::Subtract
                | BinaryOpKind::Multiply
                | BinaryOpKind::Modulo => {
                    match (self.shape_of(left, scope), self.shape_of(right, scope)) {
                        (Shape::Int, Shape::Int) => Shape::Int,
                        (Shape::Real, Shape::Real | Shape::Int) | (Shape::Int, Shape::Real) => {
                            Shape::Real
                        }
                        (Shape::Text, Shape::Text) if matches!(op, BinaryOpKind::Add) => {
                            Shape::Text
                        }
                        _ => Shape::Unknown,
                    }
                }
                _ => Shape::Unknown,
            },
            Expr::List(items) => {
                let mut shapes = items.iter().map(|i| self.shape_of(i, scope));
                match shapes.next() {
                    None => Shape::Seq(Box::new(Shape::Unknown)),
                    Some(first) => {
                        let all_same = shapes.all(|s| s == first);
                        Shape::Seq(Box::new(if all_same { first } else { Shape::Unknown }))
                    }
                }
            }
            Expr::New {
                type_name,
                type_args,
                ..
            } => match self.interner.resolve(*type_name) {
                "Seq" | "List" if type_args.len() == 1 => {
                    Shape::Seq(Box::new(self.shape_of_type(&type_args[0])))
                }
                "Seq" | "List" | "Map" | "Set" => Shape::Unknown,
                _ => self.named_shape(*type_name),
            },
            Expr::FieldAccess { object, field } => {
                let Shape::Named(record) = self.shape_of(object, scope) else {
                    return Shape::Unknown;
                };
                self.field_shape(record, *field)
            }
            _ => Shape::Unknown,
        }
    }

    /// The declared shape of `record`'s `field`, from the type registry.
    fn field_shape(&self, record: Symbol, field: Symbol) -> Shape {
        use crate::analysis::registry::{FieldType, TypeDef};
        let Some(TypeDef::Struct {
            fields, generics, ..
        }) = self.type_registry.as_ref().and_then(|r| r.get(record))
        else {
            return Shape::Unknown;
        };
        if !generics.is_empty() {
            return Shape::Unknown;
        }
        fn of(p: &Parser<'_, '_, '_>, ty: &FieldType) -> Shape {
            match ty {
                FieldType::Primitive(s) | FieldType::Named(s) => p.named_shape(*s),
                FieldType::Generic { base, params } => match p.interner.resolve(*base) {
                    "Seq" | "List" if params.len() == 1 => Shape::Seq(Box::new(of(p, &params[0]))),
                    _ => Shape::Unknown,
                },
                FieldType::TypeParam(_) | FieldType::Tuple(_) | FieldType::Record(_) => {
                    Shape::Unknown
                }
            }
        }
        fields
            .iter()
            .find(|f| f.name == field)
            .map_or(Shape::Unknown, |f| of(self, &f.ty))
    }
}

/// Whether a value of shape `got` may fill a slot declared as `want`. Anything not known agrees;
/// a whole number fills a Real slot, as it does everywhere else in the language.
fn shapes_agree(want: &Shape, got: &Shape) -> bool {
    match (want, got) {
        (Shape::Unknown, _) | (_, Shape::Unknown) => true,
        (Shape::Real, Shape::Int) => true,
        (Shape::Seq(a), Shape::Seq(b)) => shapes_agree(a, b),
        (a, b) => a == b,
    }
}

fn backticked(items: &[&str]) -> String {
    items
        .iter()
        .map(|i| format!("`{i}`"))
        .collect::<Vec<_>>()
        .join(", ")
}
