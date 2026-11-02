//! The LGX component model — `## Page` / `## Component` / `## Store` / `## Error page` and
//! their `State:` / `Actions:` / `View:` / `Style:` sections, plus `## Directive`,
//! `## Route`, `## Before each request` and `## Redirect`.
//!
//! Everything here builds FIRST-CLASS AST ([`crate::ast::component`]): no desugaring, so
//! every tier receives the definition as written. The `View:` section arrives as a raw
//! block (HTML is not LOGOS tokens), goes through [`crate::lgx::template`], and each
//! expression it holds — a hole, a condition, an iterable, a directive value — is
//! SUB-LEXED at its own byte offset by the real expression parser, so a mistake inside a
//! hole is reported at the hole rather than at the section header.

use super::{ParseResult, Parser, ParserMode, StmtList};
use crate::ast::component::{
    ActionDef, ComponentDef, ComponentKind, EventHandler, RouteBody, StateField, StoreMode,
    StyleBlock, Trigger,
};
use crate::ast::stmt::{FetchMethod, Pattern, StringPart, UiStmt};
use crate::ast::view::{Attr, ViewArm, ViewDef, ViewNode, ViewRoot};
use crate::ast::{Block, Expr, Literal, Stmt, TypeExpr};
use crate::error::{ParseError, ParseErrorKind};
use crate::lexer::Lexer;
use crate::lgx::html::{is_component_name, AttrPart};
use crate::lgx::raw::RawMap;
use crate::lgx::template::{self, TAttr, TExpr, TNode};
use crate::token::{RawKind, Span, TokenType};
use logicaffeine_base::Symbol;

/// A captured `View:` / `Style:` body: the dedented text, where its first character sits in
/// the file, and the indentation stripped from each line.
#[derive(Clone, Copy)]
pub(super) struct RawBody {
    pub(super) kind: RawKind,
    pub(super) body: Symbol,
    pub(super) content_start: usize,
    pub(super) indent: usize,
}

/// Hand every `TExpr` in the tree to `f`, wherever one can appear.
///
/// One walk over the template's expression positions — a hole, a filter argument, a condition, an
/// iterable, a key, a `{let}` value, an attribute's interpolation, a handler's call.
fn collect_template_exprs(nodes: &[TNode], f: &mut impl FnMut(&TExpr)) {
    fn attrs(list: &[TAttr], f: &mut impl FnMut(&TExpr)) {
        for a in list {
            match a {
                TAttr::On { expr, .. } | TAttr::Class { expr, .. } => f(expr),
                // `class="card {tone}"` — each brace is an expression at its own offset. The
                // `+ 1` steps past the `{`, exactly as `expr_from_attr_parts` does when it builds
                // the expression from the same part: one convention, two readers.
                TAttr::Dynamic { parts, .. } => {
                    for part in parts {
                        if let AttrPart::Brace { inner, span } = part {
                            f(&TExpr {
                                text: inner.clone(),
                                offset: span.start + 1,
                            });
                        }
                    }
                }
                // A `bind` names a field directly rather than holding an expression, and a static
                // attribute holds no names at all.
                TAttr::Bind { .. } | TAttr::Static { .. } => {}
            }
        }
    }
    for node in nodes {
        match node {
            TNode::Hole { expr, filters, .. } => {
                f(expr);
                for filter in filters {
                    for a in &filter.args {
                        f(a);
                    }
                }
            }
            TNode::Element {
                attrs: list,
                children,
                ..
            } => {
                attrs(list, f);
                collect_template_exprs(children, f);
            }
            TNode::If {
                arms, otherwise, ..
            } => {
                for (cond, body) in arms {
                    f(cond);
                    collect_template_exprs(body, f);
                }
                if let Some(o) = otherwise {
                    collect_template_exprs(o, f);
                }
            }
            TNode::For {
                iterable,
                key,
                body,
                empty,
                ..
            } => {
                f(iterable);
                if let Some(k) = key {
                    f(k);
                }
                collect_template_exprs(body, f);
                if let Some(e) = empty {
                    collect_template_exprs(e, f);
                }
            }
            TNode::Inspect {
                scrutinee, arms, ..
            } => {
                f(scrutinee);
                for (_, body) in arms {
                    collect_template_exprs(body, f);
                }
            }
            TNode::Let { expr, .. } => f(expr),
            TNode::Component {
                props, children, ..
            } => {
                attrs(props, f);
                collect_template_exprs(children, f);
            }
            _ => {}
        }
    }
}

/// Hand every `bind={field}`'s field and its offset in the template to `f`.
fn collect_bind_fields(nodes: &[TNode], f: &mut impl FnMut(&str, usize)) {
    for node in nodes {
        match node {
            TNode::Element {
                attrs, children, ..
            } => {
                for a in attrs {
                    if let TAttr::Bind { field, at, .. } = a {
                        f(field, *at);
                    }
                }
                collect_bind_fields(children, f);
            }
            TNode::Component { children, .. } => collect_bind_fields(children, f),
            TNode::If {
                arms, otherwise, ..
            } => {
                for (_, body) in arms {
                    collect_bind_fields(body, f);
                }
                if let Some(o) = otherwise {
                    collect_bind_fields(o, f);
                }
            }
            TNode::For { body, empty, .. } => {
                collect_bind_fields(body, f);
                if let Some(e) = empty {
                    collect_bind_fields(e, f);
                }
            }
            TNode::Inspect { arms, .. } => {
                for (_, body) in arms {
                    collect_bind_fields(body, f);
                }
            }
            TNode::Block {
                default: Some(body),
                ..
            } => collect_bind_fields(body, f),
            _ => {}
        }
    }
}

/// Hand every PLACED COMPONENT's name and the offset of its element to `f`.
fn collect_component_names(nodes: &[TNode], f: &mut impl FnMut(&str, usize)) {
    for node in nodes {
        match node {
            TNode::Component {
                name,
                children,
                span,
                ..
            } => {
                f(name, span.start);
                collect_component_names(children, f);
            }
            TNode::Element { children, .. } => collect_component_names(children, f),
            TNode::If {
                arms, otherwise, ..
            } => {
                for (_, body) in arms {
                    collect_component_names(body, f);
                }
                if let Some(o) = otherwise {
                    collect_component_names(o, f);
                }
            }
            TNode::For { body, empty, .. } => {
                collect_component_names(body, f);
                if let Some(e) = empty {
                    collect_component_names(e, f);
                }
            }
            TNode::Inspect { arms, .. } => {
                for (_, body) in arms {
                    collect_component_names(body, f);
                }
            }
            TNode::Block {
                default: Some(body),
                ..
            } => collect_component_names(body, f),
            _ => {}
        }
    }
}

/// The identifier-shaped runs in a snippet of expression text, each with its offset in it.
///
/// A LOGOS identifier starts with a letter or `_` and continues with letters, digits, `_`. A run
/// that follows a `.` is a FIELD, not a name in scope, and is skipped — otherwise `{todo.text}`
/// would offer to jump to any local called `text`. Text inside quotes is skipped for the same
/// reason: `{"count"}` mentions nothing.
fn identifiers_in(text: &str) -> Vec<(&str, usize)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if c == b'"' || c == b'\'' {
            quote = Some(c);
            i += 1;
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let after_dot = start > 0 && bytes[start - 1] == b'.';
            if !after_dot {
                out.push((&text[start..i], start));
            }
            continue;
        }
        i += 1;
    }
    out
}

/// A sentence that changes a named place — recorded where it is written, so a refusal about the
/// place can quote the sentence and point at the place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WriteVerb {
    Set,
    SetField,
    SetItem,
    Compound,
    Toggle,
    Push,
    Add,
    Remove,
    Pop,
    Increase,
    Decrease,
    Load,
    Swap,
}

impl WriteVerb {
    /// The sentence as the author wrote it, around the place `target`.
    pub(super) fn sentence(self, target: &str) -> String {
        match self {
            WriteVerb::Set => format!("Set {target} to …"),
            WriteVerb::SetField => format!("Set {target}'s … to …"),
            WriteVerb::SetItem => format!("Set item … of {target} to …"),
            WriteVerb::Compound => format!("{target} … = …"),
            WriteVerb::Toggle => format!("Toggle {target}."),
            WriteVerb::Push => format!("Push … to {target}"),
            WriteVerb::Add => format!("Add … to {target}"),
            WriteVerb::Remove => format!("Remove … from {target}"),
            WriteVerb::Pop => format!("Pop from {target}"),
            WriteVerb::Increase => format!("Increase {target} by …"),
            WriteVerb::Decrease => format!("Decrease {target} by …"),
            WriteVerb::Load => format!("Load … into {target}"),
            WriteVerb::Swap => format!("Swap {target} and …"),
        }
    }
}

/// One recorded write inside a component: the place's root name, where that name was written, and
/// the sentence that wrote it.
#[derive(Clone, Copy, Debug)]
pub(super) struct ComponentWrite {
    pub(super) target: Symbol,
    pub(super) span: Span,
    pub(super) verb: WriteVerb,
}

/// The name a place bottoms out in: `todos` in `todos`, `todo's done`, `item 1 of todos`.
fn place_root(expr: &Expr<'_>) -> Option<Symbol> {
    match expr {
        Expr::Identifier(s) => Some(*s),
        Expr::FieldAccess { object, .. } => place_root(object),
        Expr::Index { collection, .. } => place_root(collection),
        _ => None,
    }
}

/// The section words that only mean something inside a component definition.
pub(super) const SECTION_WORDS: &[&str] = &["State", "Actions", "View", "Style", "Head"];

/// The first word of every UI sentence a test may write OUTSIDE a component.
///
/// ★ ONE TABLE, BECAUSE THERE WERE TWO AND THEY DRIFTED. `parse_statement` gates on this set —
/// outside a component an ordinary English sentence must not be mistaken for a driving verb — and
/// `parse_ui_statement` reads the verb itself. Both were hand-maintained lists of the same thing,
/// so `Hover` sat in the gate with no branch behind it (accepted, then dropped as "I expected a
/// statement here") while `Focus` had a branch and was missing from the gate. Each half looked
/// complete on its own, which is exactly why neither was noticed.
///
/// `Press` and `Type` are not here: both are ordinary English at the start of a sentence, so they
/// carry an extra "not followed by a colon" guard at the gate and stay written out there.
pub(super) const UI_STATEMENT_OPENERS: &[&str] = &[
    "Click", "Navigate", "Submit", "Choose", "Hover", "Focus", "Blur", "Raise",
];

/// The DOM event an English predicate names — the ONE vocabulary the language understands.
///
/// One vocabulary for every sentence form that names a DOM event by an English predicate
/// (`When "#x" is <predicate>:` and its kin) — so the same word is a valid event everywhere it
/// can appear, never a parse error in one sentence and accepted in another.
///
/// A sentence FORM may differ. What an event is CALLED may not: it is one vocabulary, so it is one
/// table, and both readers ask it. `event_round_trip_lock` reads it back and requires every entry
/// to be causable by some sentence, which is what stops a word being added here that no test can
/// ever reach.
pub(super) const SELECTOR_EVENTS: &[(&str, &str)] = &[
    ("clicked", "click"),
    ("submitted", "submit"),
    ("focused", "focus"),
    ("blurred", "blur"),
    ("hovered", "mouseover"),
    ("changed", "change"),
    // From the `Whenever` form's copy. `typed` is the `input` event — the same one
    // `When the user types in "#x":` names, and the one `Type into` raises.
    ("typed", "input"),
    ("keyed", "keyup"),
];

impl<'a, 'ctx, 'int> Parser<'a, 'ctx, 'int> {
    // ── header blocks ──────────────────────────────────────────────────────────────────

    /// `## Page /path (params)` — a page component mounted at `path`; each `:name` segment
    /// of the path binds a typed parameter the page's state can read.
    pub(super) fn parse_page_block(&mut self, path: Symbol) -> ParseResult<Stmt<'a>> {
        let span = self.current_span();
        self.advance(); // consume the header
        let params = self.parse_param_groups()?;
        let name = self.interner.intern(&format!(
            "__page_{}",
            self.interner.resolve(path).replace(['/', ':'], "_")
        ));
        // A page is an LGX component like any other — registered so it composes, renders and
        // MOUNTS through the same runtime. Leaving it out is what made a page a header that
        // decided a URL and rendered nothing: its lowered name is not in the source, so no
        // `Mount` sentence could ever reach it.
        self.components.insert(name);
        self.lgx_components.insert(name);
        let bound = self.page_param_state(path, &params, span)?;
        let mut def = self.parse_component_sections(
            ComponentKind::Page { path, params },
            name,
            Vec::new(),
            span,
        )?;
        // ★ THE PARAMETERS GO IN FRONT OF THE AUTHOR'S OWN STATE, because a `Let` in
        // `State:` may read one (`Let post always be findPost(id).`) and a binding cannot
        // be read before it exists.
        bound.into_iter().rev().for_each(|f| def.state.insert(0, f));
        Ok(Stmt::ComponentDef(Box::new(def)))
    }

    /// Is this the primitive `Int`? A page parameter declared `Int` is parsed from its path
    /// segment rather than handed over as text, so `{id + 1}` is arithmetic, not concatenation.
    fn is_int_type(&self, ty: &TypeExpr<'a>) -> bool {
        match ty {
            TypeExpr::Primitive(s) | TypeExpr::Named(s) => self.interner.resolve(*s) == "Int",
            _ => false,
        }
    }

    /// A page's `:name` path segments, as ordinary `State:` fields.
    ///
    /// ★ THE HEADER WAS PARSED AND NEVER READ. `ComponentKind::Page { path, params }`
    /// carried the list and every consumer spelled it `..`, so `## Page /posts/:id (id: Int)`
    /// accepted the sentence and then failed with `Undefined variable: id`. That is the same
    /// bug `path` itself had — recorded in `web_bundle.rs` as "carried and never read" — sitting
    /// one field away in the same variant, which is exactly how this shape keeps recurring.
    ///
    /// ★ NOTHING NEW MATCHES ANYTHING. `web.lg` already ships `routeParam(pattern, name, path)`,
    /// documented with `routeParam("/todos/:id", "id", "/todos/42")` is `"42"`, and `routePath()`
    /// already reads the location on every tier. Lowering a parameter to ORDINARY STATE means the
    /// interpreter, the VM and wasm each render it with the code they already have — no tier
    /// learns what a path parameter is.
    ///
    /// They are DERIVED, so a client-side navigation that changes the URL changes them; a plain
    /// binding would freeze the first id the page was ever mounted at.
    fn page_param_state(
        &mut self,
        path: Symbol,
        params: &[(Symbol, &'a TypeExpr<'a>)],
        span: Span,
    ) -> ParseResult<Vec<StateField<'a>>> {
        if params.is_empty() {
            return Ok(Vec::new());
        }
        let pattern = self.interner.resolve(path).to_string();
        let slots: Vec<String> = pattern
            .split('/')
            .filter(|s| s.starts_with(':'))
            .map(|s| s[1..].to_string())
            .collect();
        let names: Vec<String> = params
            .iter()
            .map(|(n, _)| self.interner.resolve(*n).to_string())
            .collect();
        // ★ A PARAMETER THE PATH HAS NO SLOT FOR CAN NEVER BE FILLED. Binding it to "" would
        // hand the page a value that looks like an answer, so it is a refusal — by name, with
        // the slots the path actually offers, because the mistake is nearly always a typo.
        for word in &names {
            if !slots.contains(word) {
                let offered = if slots.is_empty() {
                    "the path declares no `:name` segments at all".to_string()
                } else {
                    format!(
                        "the path offers {}",
                        slots
                            .iter()
                            .map(|s| format!("`:{s}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                return Err(ParseError {
                    kind: ParseErrorKind::Custom(format!(
                        "`## Page {pattern}` declares a parameter `{word}`, but {offered}. A page \
                         parameter is filled from the `:{word}` segment of its own path, so one \
                         the path never names could only ever be empty — add `:{word}` to the \
                         path, or drop the parameter."
                    )),
                    span,
                });
            }
        }
        let route_param = self.interner.intern("routeParam");
        let route_path = self.interner.intern("routePath");
        let parse_int = self.interner.intern("parseInt");
        let mut fields = Vec::with_capacity(params.len());
        for ((name, ty), _word) in params.iter().zip(names.iter()) {
            let pat = self
                .ctx
                .alloc_imperative_expr(Expr::Literal(Literal::Text(path)));
            let nm = self
                .ctx
                .alloc_imperative_expr(Expr::Literal(Literal::Text(*name)));
            let here = self.te_call(route_path, Vec::new());
            let raw = self.te_call(route_param, vec![pat, nm, here]);
            let init = if self.is_int_type(ty) {
                self.te_call(parse_int, vec![raw])
            } else {
                raw
            };
            fields.push(StateField {
                name: *name,
                ty: Some(*ty),
                init,
                derived: true,
                span,
            });
        }
        Ok(fields)
    }

    /// The name a header DECLARES, exactly as it is spelled. Every reference to a component,
    /// store or directive (`<Toasts/>`, `Call Messages's post.`, `highlights={v}`) reads its
    /// spelling, so the declaration must record that spelling too — not the dictionary form the
    /// lexicon gives a word it knows as a plural noun (`Toasts` is the noun `toast`).
    fn expect_declared_name(&mut self) -> ParseResult<Symbol> {
        match self.peek().kind {
            TokenType::Noun(_) | TokenType::ProperName(_) | TokenType::Adjective(_) => {
                let spelled = self.peek().lexeme;
                self.advance();
                Ok(spelled)
            }
            _ => self.expect_identifier(),
        }
    }

    /// `## Component Name (props)` — the LGX component (a trailing `:` after the name is
    /// silently accepted too, uniformly with every other header that takes no colon of its own).
    pub(super) fn parse_lgx_component_block(&mut self) -> ParseResult<Stmt<'a>> {
        let span = self.current_span();
        let name = self.expect_declared_name()?;
        self.components.insert(name);
        self.lgx_components.insert(name);
        let props = self.parse_param_groups()?;
        let def = self.parse_component_sections(ComponentKind::Component, name, props, span)?;
        Ok(Stmt::ComponentDef(Box::new(def)))
    }

    /// `## Store Name [persisted as "key" | shared on "topic"]` — shared state whose only
    /// mutators are its own actions.
    pub(super) fn parse_store_block(&mut self) -> ParseResult<Stmt<'a>> {
        let span = self.current_span();
        self.advance(); // consume the header
        let name = self.expect_declared_name()?;
        let mode = if self.eat_words(&["persisted", "as"]) {
            StoreMode::Persisted {
                key: self.parse_imperative_expr()?,
            }
        } else if self.eat_words(&["shared", "on"]) {
            StoreMode::Shared {
                topic: self.parse_imperative_expr()?,
            }
        } else if self.eat_words(&["preserved", "across", "reloads"]) {
            StoreMode::Preserved
        } else {
            StoreMode::Local
        };
        let def =
            self.parse_component_sections(ComponentKind::Store { mode }, name, Vec::new(), span)?;
        self.lgx_stores
            .insert(name, def.actions.iter().map(|a| a.name).collect());
        if def.view.is_some() || def.style.is_some() {
            return Err(ParseError {
                kind: ParseErrorKind::Custom(
                    "A store holds state and the actions that change it — it has no `View:` or `Style:`. \
                     Read it from a component (`Use <Store>.`) and render there."
                        .to_string(),
                ),
                span,
            });
        }
        Ok(Stmt::ComponentDef(Box::new(def)))
    }

    /// `## Error page <status>` — the view served for an HTTP error.
    pub(super) fn parse_error_page_block(&mut self, status: u16) -> ParseResult<Stmt<'a>> {
        let span = self.current_span();
        self.advance(); // consume the header
        let name = self.interner.intern(&format!("__error_{status}"));
        let def = self.parse_component_sections(
            ComponentKind::ErrorPage { status },
            name,
            Vec::new(),
            span,
        )?;
        Ok(Stmt::ComponentDef(Box::new(def)))
    }

    /// `## Directive name (el: Element) and (value: T):` — a custom attribute behavior,
    /// with an optional `When removed:` cleanup block.
    pub(super) fn parse_directive_block(&mut self) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume the header
        let name = self.expect_declared_name()?;
        self.directives.insert(name);
        let params = self.parse_param_groups()?;
        self.expect_colon()?;
        self.expect_indent()?;
        let mut body = StmtList::default();
        let mut removed: Option<Block<'a>> = None;
        while !self.check(&TokenType::Dedent) && !self.is_at_end() {
            if self.check(&TokenType::Newline) {
                self.advance();
                continue;
            }
            if self.lexeme_is(0, "When") && self.lexeme_is(1, "removed") {
                self.advance();
                self.advance();
                self.expect_colon()?;
                removed = Some(self.parse_indented_block()?);
                continue;
            }
            self.read_block_statement(&mut body)?;
        }
        if self.check(&TokenType::Dedent) {
            self.advance();
        }
        let body = self.alloc_read_block(body);
        Ok(Stmt::DirectiveDef {
            name,
            params,
            body,
            removed,
        })
    }

    /// `## Route METHOD /path (params) -> T:` — a typed HTTP route. A `-> View` route's
    /// body is a template; anything else is ordinary statements.
    pub(super) fn parse_route_block(
        &mut self,
        method: Symbol,
        path: Symbol,
    ) -> ParseResult<Stmt<'a>> {
        let span = self.current_span();
        self.advance(); // consume the header
        let params = self.parse_param_groups()?;
        let return_type = if self.check(&TokenType::Arrow) {
            self.advance();
            let t = self.parse_type_expression()?;
            Some(self.ctx.alloc_type_expr(t))
        } else {
            None
        };
        self.expect_colon()?;
        // A `-> View` route renders a template: the lexer captured the body raw.
        if let Some(raw) = self.peek_raw_block_after_indent() {
            if raw.kind == RawKind::View {
                self.consume_raw_block();
                let view = self.view_from_raw(raw, None, span)?;
                return Ok(Stmt::RouteDef {
                    method,
                    path,
                    params,
                    return_type,
                    body: Box::new(RouteBody::View(view)),
                    span,
                });
            }
        }
        let body = self.parse_indented_block()?;
        Ok(Stmt::RouteDef {
            method,
            path,
            params,
            return_type,
            body: Box::new(RouteBody::Statements(body)),
            span,
        })
    }

    /// `## Before each request (request: Request) -> Option of Response:` — middleware.
    pub(super) fn parse_middleware_block(&mut self) -> ParseResult<Stmt<'a>> {
        let span = self.current_span();
        self.advance(); // consume the header
        let params = self.parse_param_groups()?;
        let param = params.into_iter().next().ok_or(ParseError {
            kind: ParseErrorKind::Custom(
                "`## Before each request` takes the request it may answer early: \
                 `## Before each request (request: Request) -> Option of Response:`."
                    .to_string(),
            ),
            span,
        })?;
        if self.check(&TokenType::Arrow) {
            self.advance();
            let _ = self.parse_type_expression()?;
        }
        self.expect_colon()?;
        let body = self.parse_indented_block()?;
        Ok(Stmt::MiddlewareDef { param, body })
    }

    /// `## Redirect /from -> /to.`
    pub(super) fn parse_redirect_block(
        &mut self,
        from: Symbol,
        to: Symbol,
    ) -> ParseResult<Stmt<'a>> {
        self.advance(); // consume the header
        if self.check(&TokenType::Period) {
            self.advance();
        }
        Ok(Stmt::RedirectDef { from, to })
    }

    // ── sections ───────────────────────────────────────────────────────────────────────

    /// The `Use` / `State:` / `Actions:` / `View:` / `Style:` sections of one definition,
    /// in any order, until the next `##` header or the end of the program.
    fn parse_component_sections(
        &mut self,
        kind: ComponentKind<'a>,
        name: Symbol,
        props: Vec<(Symbol, &'a TypeExpr<'a>)>,
        span: Span,
    ) -> ParseResult<ComponentDef<'a>> {
        let mut def = ComponentDef {
            kind,
            name,
            props,
            uses: Vec::new(),
            state: Vec::new(),
            events: Vec::new(),
            actions: Vec::new(),
            view: None,
            head: None,
            style: None,
            span,
        };
        let saved_in_component = std::mem::replace(&mut self.in_component, true);
        let saved_stops = std::mem::take(&mut self.timer_stops);
        let saved_writes = std::mem::take(&mut self.component_writes);
        let result = self.parse_component_sections_inner(&mut def, name);
        self.in_component = saved_in_component;
        let stops = std::mem::replace(&mut self.timer_stops, saved_stops);
        let writes = std::mem::replace(&mut self.component_writes, saved_writes);
        result?;
        self.check_timer_stops(&def, &stops)?;
        self.check_prop_writes(&def, &writes)?;
        Ok(def)
    }

    /// A PROP is what the parent said, not a cell this component owns, so no sentence in the
    /// component may change one. What a body writes is decided by the one write walk
    /// (`lgx::deps::block_prop_writes`); the refusal points at the place as the author wrote it,
    /// from the write recorded where the sentence was parsed, and quotes that sentence.
    fn check_prop_writes(
        &self,
        def: &ComponentDef<'a>,
        writes: &[ComponentWrite],
    ) -> ParseResult<()> {
        let mut bodies: Vec<(crate::ast::stmt::Block<'a>, Vec<Symbol>, Span)> = Vec::new();
        for a in &def.actions {
            bodies.push((a.body, a.params.iter().map(|(p, _)| *p).collect(), a.span));
        }
        for e in &def.events {
            bodies.push((e.body, trigger_bindings(&e.trigger), e.span));
        }
        let label = self.component_label(def);
        for (body, bound, fallback) in bodies {
            let Some(prop) = crate::lgx::deps::block_prop_writes(def, body, &bound)
                .into_iter()
                .next()
            else {
                continue;
            };
            let word = self.interner.resolve(prop);
            let site = writes.iter().find(|w| w.target == prop);
            let sentence = site
                .map(|w| format!("`{}`: ", w.verb.sentence(word)))
                .unwrap_or_default();
            return Err(ParseError {
                kind: ParseErrorKind::Custom(format!(
                    "{sentence}`{word}` is a prop of {label}, so this component cannot change it — its \
                     value is whatever the parent passed. Keep the changing value in the PARENT's \
                     `State:` and hand this component a callback prop to call, or declare a \
                     separate `Let` in this component's own `State:`."
                )),
                span: site.map_or(fallback, |w| w.span),
            });
        }
        Ok(())
    }

    /// Every `Stop the timer <name>` and `Start the timer <name>` names a timer this component
    /// declares. Checked once its sections close, so an action may name a timer declared below it.
    fn check_timer_stops(
        &self,
        def: &ComponentDef<'a>,
        stops: &[(Symbol, Span)],
    ) -> ParseResult<()> {
        let declared: Vec<Symbol> = def
            .events
            .iter()
            .filter_map(|h| match h.trigger {
                Trigger::Every { name, .. } | Trigger::After { name, .. } => name,
                _ => None,
            })
            .collect();
        for (name, span) in stops {
            if declared.contains(name) {
                continue;
            }
            let word = self.interner.resolve(*name);
            let have: Vec<&str> = declared.iter().map(|n| self.interner.resolve(*n)).collect();
            let known = if have.is_empty() {
                "this component declares no named timers".to_string()
            } else {
                format!("its timers are {}", have.join(", "))
            };
            return Err(ParseError {
                kind: ParseErrorKind::Custom(format!(
                    "`{word}` is not a timer of this component — {known}. A timer is named where \
                     it is declared: `Every 60 seconds as {word}:`."
                )),
                span: *span,
            });
        }
        Ok(())
    }

    fn parse_component_sections_inner(
        &mut self,
        def: &mut ComponentDef<'a>,
        name: Symbol,
    ) -> ParseResult<()> {
        loop {
            while self.check(&TokenType::Newline) || self.check(&TokenType::Period) {
                self.advance();
            }
            if self.is_at_end() || matches!(self.peek().kind, TokenType::BlockHeader { .. }) {
                break;
            }
            if self.lexeme_is(0, "Use") {
                self.advance();
                let use_span = self.current_span();
                let store = self.expect_declared_name()?;
                def.uses.push(store);
                self.use_sites.push((store, use_span));
                if self.check(&TokenType::Period) {
                    self.advance();
                }
                continue;
            }
            if self.lexeme_is(0, "State") && self.next_is_colon() {
                self.advance();
                self.expect_colon()?;
                self.parse_state_section(def)?;
                continue;
            }
            if self.lexeme_is(0, "Actions") && self.next_is_colon() {
                self.advance();
                self.expect_colon()?;
                self.parse_actions_section(def)?;
                continue;
            }
            if self.lexeme_is(0, "View") && self.next_is_colon() {
                let view_span = self.current_span();
                self.advance();
                self.expect_colon()?;
                let raw = match self.consume_raw_block() {
                    Some(raw) => raw,
                    None => return Err(self.view_body_is_not_markup(view_span)),
                };
                if raw.kind != RawKind::View {
                    return Err(ParseError {
                        kind: ParseErrorKind::Custom("expected a View body".to_string()),
                        span: view_span,
                    });
                }
                def.view = Some(self.view_from_raw(raw, Some(name), view_span)?);
                continue;
            }
            if self.lexeme_is(0, "Head") && self.next_is_colon() {
                let head_span = self.current_span();
                self.advance();
                self.expect_colon()?;
                let raw = self.consume_raw_block().ok_or(ParseError {
                    kind: ParseErrorKind::Custom(
                        "A `Head:` section holds head HTML: `<title>`, `<meta>`, `<link>` or \
                         `<base>`, one per line."
                            .to_string(),
                    ),
                    span: head_span,
                })?;
                if raw.kind != RawKind::Head {
                    return Err(ParseError {
                        kind: ParseErrorKind::Custom("expected a Head body".to_string()),
                        span: head_span,
                    });
                }
                // A page owns its document's head, and a layout carries the head every page
                // extending it shares (merged under the page's own, tag by tag). A component
                // PLACED inside a page owns nothing of the document; `lgx_resolve` refuses such a
                // placement by name, since which components are layouts is known only there.
                def.head = Some(self.head_from_raw(raw, head_span)?);
                continue;
            }
            if self.lexeme_is(0, "Style") && self.next_is_colon() {
                let style_span = self.current_span();
                self.advance();
                self.expect_colon()?;
                let raw = self.consume_raw_block().ok_or(ParseError {
                    kind: ParseErrorKind::Custom("A `Style:` section holds CSS.".to_string()),
                    span: style_span,
                })?;
                // An invalid stylesheet is refused here, at the character that broke it: the
                // scoper's offset is into the dedented body, mapped into the file the way a
                // template's are. The scope class only prefixes selectors, so a placeholder
                // decides validity.
                let style = StyleBlock {
                    css: raw.body,
                    content_start: raw.content_start,
                    indent: raw.indent,
                };
                let css_text = self.interner.resolve(raw.body).to_string();
                if let Err(e) = crate::lgx::css::scope(&css_text, "x") {
                    let at = style.file_offset(&css_text, e.at);
                    return Err(ParseError {
                        kind: ParseErrorKind::Custom(format!(
                            "The `Style:` of {} is not valid CSS. {}",
                            self.component_label(def),
                            e.message()
                        )),
                        span: Span::new(at, at + 1),
                    });
                }
                def.style = Some(style);
                continue;
            }
            // ★ A STRAY `Indent` HERE IS ALWAYS THE LEGACY HEADER COLON, not an unrecognized word.
            // The lexer is indentation-sensitive and opens a nested block after ANY line ending
            // in `:` regardless of what the parser expects there — `## Component App:` (the old
            // dialect's own header, one indented body underneath) opens exactly the block this
            // loop is not reading a header-colon for, since a component's OWN sections open their
            // indent from their own `State:`/`View:`/etc keyword instead. The token this loop then
            // sees has no word to quote (`Indent` carries no author-written lexeme), which is what
            // made the old message read as a bare pair of empty backticks — naming the real mistake
            // instead of the token that stands for it is what actually teaches the LGX layout.
            if matches!(self.peek().kind, TokenType::Indent) {
                return Err(ParseError {
                    kind: ParseErrorKind::Custom(
                        "A component's sections open their OWN indent, from their OWN keyword — a \
                         `## Component <Name>:` header does not open one itself. Remove the colon \
                         after the name, declare state under its own `State:` section, and the view \
                         under its own `View:` section, written as HTML."
                            .to_string(),
                    ),
                    span: self.current_span(),
                });
            }
            return Err(ParseError {
                kind: ParseErrorKind::Custom(format!(
                    "`{}` is not part of a component. Its sections are `State:` (the data), `Actions:` \
                     (the only things that change it), `View:` (the HTML), `Style:` (its CSS) and, on a \
                     `## Page`, `Head:` (the document's title and metadata), plus `Use <Store>.` to \
                     read a store.",
                    self.interner.resolve(self.peek().lexeme)
                )),
                span: self.current_span(),
            });
        }
        Ok(())
    }

    /// `State:` — `Let x [: T] be <init>.` and `Let x always be <expr>.` (derived).
    fn parse_state_section(&mut self, def: &mut ComponentDef<'a>) -> ParseResult<()> {
        self.expect_indent()?;
        while !self.check(&TokenType::Dedent) && !self.is_at_end() {
            if self.check(&TokenType::Newline) {
                self.advance();
                continue;
            }
            let span = self.current_span();
            if !self.eat_words(&["Let"]) {
                return Err(ParseError {
                    kind: ParseErrorKind::Custom(
                        "A `State:` section declares data: `Let draft be \"\".`, `Let count: Int be 0.`, \
                         or `Let remaining always be length of todos.` for a value derived from the rest."
                            .to_string(),
                    ),
                    span,
                });
            }
            // `mutable`/`mut` is optional intent, not enforced: every `State:` field is already
            // writable from `Actions:`, so the word is a no-op here — accepted for the same reason
            // a top-level `Let mutable x be …` is.
            let mutable_span = self.current_span();
            let mutable = self.check_mutable_keyword();
            if mutable {
                self.advance();
            }
            let field = self.expect_identifier()?;
            let ty = if self.check(&TokenType::Colon) {
                self.advance();
                let t = self.parse_type_expression()?;
                Some(self.ctx.alloc_type_expr(t))
            } else {
                None
            };
            let derived = self.eat_words(&["always"]);
            if mutable && derived {
                return Err(ParseError {
                    kind: ParseErrorKind::Custom(format!(
                        "`{name}` is declared both `mutable` and `always`: `always` computes a value \
                         from the rest of `State:`, so nothing ever writes it, and `mutable` is \
                         meaningless there. Drop one.",
                        name = self.interner.resolve(field),
                    )),
                    span: mutable_span,
                });
            }
            // From `Let` through `always` (or the type, or the name): the field's own header, so a
            // refusal about the FIELD — a derived cycle, a `mutable`/`always` conflict — reads at
            // the declaration, not merely at the leading `Let`.
            let field_span = Span::new(span.start, self.current_span().start);
            if !self.eat_words(&["be"]) {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword {
                        keyword: "be".to_string(),
                    },
                    span: self.current_span(),
                });
            }
            let init_tok = self.current;
            let init = self.parse_imperative_expr()?;
            // The same agreement a top-level `Let` is held to. Without this a `State:` field's
            // annotation was accepted and then ignored.
            //
            // A CONVERGENT type (`Tally`, `SharedSet of T`, …) means something only where the state
            // is shared: in a `## Store … shared on "topic"` the field is the plain value its CRDT
            // converges to across every page on the topic (`lgx_lower::shared`). Anywhere else
            // nothing shares it, and accepting the annotation would be ignoring it — so it is
            // refused there, by name. A counter starts from the whole number beside it (its
            // starting total); a set or sequence starts empty and fills from its topic.
            let init = match ty {
                Some(declared) => {
                    match super::lgx_lower::shared_field_kind(declared, self.interner) {
                        Some(kind) => {
                            use super::lgx_lower::SharedFieldKind;
                            let name = self.interner.resolve(field).to_string();
                            let type_name = match declared {
                                TypeExpr::Primitive(t)
                                | TypeExpr::Named(t)
                                | TypeExpr::Generic { base: t, .. } => {
                                    self.interner.resolve(*t).to_string()
                                }
                                _ => String::new(),
                            };
                            let refuse = |message: String| ParseError {
                                kind: ParseErrorKind::Custom(message),
                                span: field_span,
                            };
                            let kind = kind.map_err(|why| refuse(format!("`{name}`: {why}.")))?;
                            let shared = matches!(
                                def.kind,
                                ComponentKind::Store {
                                    mode: StoreMode::Shared { .. }
                                }
                            );
                            let (carrier, start) = match kind {
                                SharedFieldKind::Counter { .. } => {
                                    ("Int".to_string(), "0".to_string())
                                }
                                SharedFieldKind::Set { elem } => {
                                    let elem = self.interner.resolve(elem);
                                    (format!("Set of {elem}"), format!("a new Set of {elem}"))
                                }
                                SharedFieldKind::Sequence { elem } => {
                                    let elem = self.interner.resolve(elem);
                                    (format!("Seq of {elem}"), format!("a new Seq of {elem}"))
                                }
                                SharedFieldKind::Register => (String::new(), String::new()),
                            };
                            if !shared {
                                return Err(refuse(format!(
                                "`{name}` is declared a {type_name}, which converges across every page \
                                 sharing a topic — it belongs in a `## Store … shared on \"topic\"`. Here \
                                 nothing shares it, so it would only be a plain {carrier}: declare it as \
                                 one, `Let {name}: {carrier} be ….`"
                            )));
                            }
                            let starts_right = match kind {
                                SharedFieldKind::Counter { .. } => {
                                    matches!(init, Expr::Literal(Literal::Number(_)))
                                }
                                SharedFieldKind::Set { elem }
                                | SharedFieldKind::Sequence { elem } => {
                                    let collection = if matches!(kind, SharedFieldKind::Set { .. })
                                    {
                                        "Set"
                                    } else {
                                        "Seq"
                                    };
                                    matches!(
                                        init,
                                        Expr::New { type_name, type_args, init_fields }
                                            if self.interner.resolve(*type_name) == collection
                                                && init_fields.is_empty()
                                                && matches!(
                                                    type_args.as_slice(),
                                                    [TypeExpr::Primitive(e) | TypeExpr::Named(e)] if *e == elem
                                                )
                                    )
                                }
                                SharedFieldKind::Register => true,
                            };
                            if !starts_right {
                                let (how, spelled) = match kind {
                                    SharedFieldKind::Set { elem }
                                    | SharedFieldKind::Sequence { elem } => (
                                        "starts empty and fills from its topic",
                                        format!("{type_name} of {}", self.interner.resolve(elem)),
                                    ),
                                    _ => (
                                        "starts from a whole number, its starting total",
                                        type_name.clone(),
                                    ),
                                };
                                return Err(refuse(format!(
                                "`{name}` is a {type_name}: it {how} — `Let {name}: {spelled} be {start}.`"
                            )));
                            }
                            init
                        }
                        None => {
                            self.check_declared_type(declared, init, self.span_since(init_tok))?;
                            self.widen_literal_to_declared(declared, init)
                        }
                    }
                }
                None => init,
            };
            if self.check(&TokenType::Period) {
                self.advance();
            }
            def.state.push(StateField {
                name: field,
                ty,
                init,
                derived,
                span: field_span,
            });
        }
        if self.check(&TokenType::Dedent) {
            self.advance();
        }
        Ok(())
    }

    /// `Actions:` — English event sentences (`When …:` / `Every N seconds:`) and the
    /// `To name (params):` procedures they call.
    fn parse_actions_section(&mut self, def: &mut ComponentDef<'a>) -> ParseResult<()> {
        self.expect_indent()?;
        while !self.check(&TokenType::Dedent) && !self.is_at_end() {
            if self.check(&TokenType::Newline) {
                self.advance();
                continue;
            }
            let span = self.current_span();
            if self.lexeme_is(0, "To") {
                self.advance();
                let name = self.expect_identifier()?;
                let params = self.parse_param_groups()?;
                self.expect_colon()?;
                // ★ SITE TRACKING (Wave 3): the sentences of THIS action's own body get a
                // `Stmt::Site` marker each — `def.name`/`name` are exactly the `(component,
                // action)` pair `lgx_lower.rs::action_fn_name` compiles this action's body under,
                // so a `FailureSite` and the function a trap actually happened in always agree.
                let outer_tracking = self.site_tracking.replace((def.name, name));
                let body = self.parse_indented_block();
                self.site_tracking = outer_tracking;
                let body = body?;
                def.actions.push(ActionDef {
                    name,
                    params,
                    body,
                    span,
                });
                continue;
            }
            let trigger = self.parse_trigger()?;
            self.expect_colon()?;
            // ★ THE HANDLER'S SPAN COVERS THE HANDLER'S SENTENCE, not its first word. `span` above
            // is `current_span()` — one token — so a refusal that points at this handler used to
            // put the caret on `Every` and stop, which reads in an editor as a squiggle under a
            // word rather than under the sentence that cannot be lowered. The browser tier refuses
            // BY TRIGGER (`NotLowerable::at(handler.span, …)`), so this is the span an author is
            // actually shown, and it should be the thing the message is talking about.
            let span = Span::new(span.start, self.tokens[self.current - 1].span.end);
            // ★ THE SAME `__e{index}` NAME `lgx_lower.rs::action_fn_name` COMPILES THIS HANDLER
            // UNDER — computed from this handler's OWN position in `def.events`, before it is
            // pushed, so a `FailureSite` built here names the exact function a trap happens in.
            let handler_action = self.interner.intern(&format!("__e{}", def.events.len()));
            let outer_tracking = self.site_tracking.replace((def.name, handler_action));
            // `When "#add" is submitted: add.` — an inline single action call is allowed.
            let body = if self.check(&TokenType::Indent) {
                self.parse_indented_block()
            } else {
                (|| {
                    let mut one = StmtList::default();
                    self.read_block_statement(&mut one)?;
                    Ok(self.alloc_read_block(one))
                })()
            };
            self.site_tracking = outer_tracking;
            let body = body?;
            def.events.push(EventHandler {
                trigger,
                body,
                span,
            });
        }
        if self.check(&TokenType::Dedent) {
            self.advance();
        }
        Ok(())
    }

    /// One English event sentence, up to (not including) its `:`.
    fn parse_trigger(&mut self) -> ParseResult<Trigger<'a>> {
        let span = self.current_span();
        // `Every <n> seconds[ as <name>]:` and `After <n> seconds[ as <name>]:` — the page's
        // two timers, repeating and one-shot. Both run on the sealed clock, so `Advance time by`
        // drives them exactly.
        for (word, repeating) in [("Every", true), ("After", false)] {
            if !self.lexeme_is(0, word) {
                continue;
            }
            self.advance();
            let seconds = self.parse_imperative_expr()?;
            let _ = self.eat_words(&["seconds"]) || self.eat_words(&["second"]);
            let name = if self.eat_words(&["as"]) {
                Some(self.expect_identifier()?)
            } else {
                None
            };
            return Ok(if repeating {
                Trigger::Every { seconds, name }
            } else {
                Trigger::After { seconds, name }
            });
        }
        if !self.eat_words(&["When"]) {
            return Err(self.trigger_error(span));
        }
        // Fixed lifecycle and environment sentences.
        for (words, trigger) in [
            (&["the", "page", "loads"][..], Trigger::PageLoads),
            (&["this", "component", "appears"][..], Trigger::Appears),
            (
                &["this", "component", "disappears"][..],
                Trigger::Disappears,
            ),
            (&["the", "route", "changes"][..], Trigger::RouteChanges),
            (
                &["the", "app", "comes", "back", "online"][..],
                Trigger::Online,
            ),
            (&["the", "app", "goes", "offline"][..], Trigger::Offline),
            (&["the", "window", "resizes"][..], Trigger::WindowResizes),
        ] {
            if self.eat_words(words) {
                return Ok(trigger);
            }
        }
        // `the user …` sentences.
        if self.eat_words(&["the", "user", "presses"]) {
            let key = self.peek().lexeme;
            self.advance();
            if self.eat_words(&["in"]) {
                let selector = self.parse_imperative_expr()?;
                let event = self.interner.intern("keydown");
                return Ok(Trigger::Selector {
                    selector,
                    event,
                    key: Some(key),
                });
            }
            return Ok(Trigger::Key { key });
        }
        if self.eat_words(&["the", "user", "types", "in"]) {
            let selector = self.parse_imperative_expr()?;
            let event = self.interner.intern("input");
            return Ok(Trigger::Selector {
                selector,
                event,
                key: None,
            });
        }
        if self.eat_words(&["the", "user", "scrolls", "to", "the", "bottom"]) {
            let selector = if self.eat_words(&["of"]) {
                Some(self.parse_imperative_expr()?)
            } else {
                None
            };
            return Ok(Trigger::ScrollsToBottom { selector });
        }
        // Failure and message sentences.
        if self.eat_words(&["an", "action", "fails"]) {
            let params = self.parse_param_groups()?;
            let (param, ty) = params
                .first()
                .copied()
                .ok_or_else(|| self.trigger_error(span))?;
            self.require_trigger_param_type(
                ty,
                "Text",
                "`When an action fails (…):` — the failure's own message",
                span,
            )?;
            return Ok(Trigger::ActionFails { param });
        }
        if self.eat_words(&["a", "fetch", "fails"]) {
            let params = self.parse_param_groups()?;
            if params.len() < 2 {
                return Err(self.trigger_error(span));
            }
            self.require_trigger_param_type(
                params[0].1,
                "Text",
                "`When a fetch fails (url: …) …:` — the request's own URL",
                span,
            )?;
            self.require_trigger_param_type(
                params[1].1,
                "Text",
                "`When a fetch fails … (error: …):` — the failure's own message",
                span,
            )?;
            return Ok(Trigger::FetchFails {
                url: params[0].0,
                error: params[1].0,
            });
        }
        if self.eat_words(&["a", "message", "arrives", "on"]) {
            let topic = self.parse_imperative_expr()?;
            let params = self.parse_param_groups()?;
            let param = params
                .into_iter()
                .next()
                .ok_or_else(|| self.trigger_error(span))?;
            return Ok(Trigger::Message { topic, param });
        }
        if self.eat_words(&["a", "file", "is", "dropped", "on"]) {
            let selector = self.parse_imperative_expr()?;
            let params = self.parse_param_groups()?;
            let param = params
                .into_iter()
                .next()
                .ok_or_else(|| self.trigger_error(span))?;
            let name = self
                .trigger_param_type_name(param.1)
                .map(|s| self.interner.resolve(s));
            if !matches!(name, Some("File") | Some("Text")) {
                return Err(ParseError {
                    kind: ParseErrorKind::Custom(
                        "a dropped file binds as `File` (its name and size) or as `Text` (its \
                         name)."
                            .to_string(),
                    ),
                    span,
                });
            }
            return Ok(Trigger::FileDropped { selector, param });
        }
        // A selector sentence (`When "#add" is submitted:`) or a data sentence
        // (`When todos change:`, `When Session's name changes:`).
        let is_selector = matches!(
            self.peek().kind,
            TokenType::StringLiteral(_) | TokenType::InterpolatedString(_)
        );
        if is_selector {
            // ★ THE SUBJECT IS A STRING, SO ONLY A STRING IS READ. Parsing a full expression here
            // let the trigger collide with a page QUESTION: `"#b" is focused` is also
            // `DomQuery::Focused`, so the expression parser swallowed the whole clause and
            // `When "#b" is focused:` was refused — an entry in `SELECTOR_EVENTS` that could never
            // be declared, while `blurred` and `hovered` (which name no page question) worked.
            // A trigger's selector is a literal by construction; reading one is both correct and
            // what keeps the two grammars from overlapping.
            let selector = self.parse_primary_expr()?;
            let _ = self.eat_words(&["is"]);
            let word = self
                .interner
                .resolve(self.peek().lexeme)
                .to_ascii_lowercase();
            let event = SELECTOR_EVENTS
                .iter()
                .find(|(english, _)| *english == word)
                .map(|(_, dom)| *dom)
                .or_else(|| (word == "changes" || word == "change").then_some("change"))
                .ok_or_else(|| self.trigger_error(span))?;
            self.advance();
            let event = self.interner.intern(event);
            return Ok(Trigger::Selector {
                selector,
                event,
                key: None,
            });
        }
        let first = if self
            .tokens
            .get(self.current + 1)
            .is_some_and(|t| matches!(t.kind, TokenType::Possessive))
        {
            self.expect_declared_name()?
        } else {
            self.expect_identifier()?
        };
        let (store, field) = if self.check(&TokenType::Possessive) {
            self.advance();
            (Some(first), self.expect_identifier()?)
        } else {
            (None, first)
        };
        if !self.eat_words(&["changes"]) && !self.eat_words(&["change"]) {
            return Err(self.trigger_error(span));
        }
        Ok(Trigger::FieldChanges { store, field })
    }

    fn trigger_error(&self, span: Span) -> ParseError {
        ParseError {
            kind: ParseErrorKind::Custom(
                "An `Actions:` sentence names what it waits for: `When the page loads:`, \
                 `When \"#add\" is submitted:`, `When the user presses Escape in \"#draft\":`, \
                 `When todos change:`, `When an action fails (e: Text):`, `Every 60 seconds:` — \
                 or `To <name> (params):` for an action other sentences call."
                    .to_string(),
            ),
            span,
        }
    }

    /// The plain scalar/named type a trigger parameter's annotation resolves to — `Text`, `File`,
    /// whatever the author wrote — or `None` for a shape with no single name (`Seq of X`, a tuple,
    /// a record): never a valid trigger payload type either way.
    fn trigger_param_type_name(&self, ty: &TypeExpr) -> Option<Symbol> {
        match ty {
            TypeExpr::Primitive(s) | TypeExpr::Named(s) => Some(*s),
            _ => None,
        }
    }

    /// Every trigger parameter's own annotation is either KEPT in the AST (`Trigger::Message`,
    /// `Trigger::FileDropped` — the trigger delivers more than one possible shape, so the parser
    /// cannot decide FOR the author) or VALIDATED against the ONE shape the trigger actually
    /// delivers — this is that second half, for a trigger whose payload the AST keeps as a bare
    /// `Symbol`. Refuses rather than silently discarding a mismatched annotation, naming what the
    /// trigger delivers instead. `trigger_param_type_census_lock` checks every trigger-parsing site
    /// does one or the other.
    fn require_trigger_param_type(
        &self,
        ty: &TypeExpr,
        want: &str,
        sentence: &str,
        span: Span,
    ) -> ParseResult<()> {
        match self
            .trigger_param_type_name(ty)
            .map(|s| self.interner.resolve(s))
        {
            Some(found) if found == want => Ok(()),
            Some(found) => Err(ParseError {
                kind: ParseErrorKind::Custom(format!(
                    "{sentence} delivers `{want}`, never `{found}`."
                )),
                span,
            }),
            None => Err(ParseError {
                kind: ParseErrorKind::Custom(format!("{sentence} delivers `{want}`.")),
                span,
            }),
        }
    }

    // ── the action vocabulary ──────────────────────────────────────────────────────────

    /// One `Actions:` sentence that DOES something — an element verb, navigation,
    /// persistence, feedback, a store call, a timer, or a typed request. Returns `None` when
    /// the cursor is not on one, so an action body still parses ordinary statements.
    ///
    /// These are COMPONENT vocabulary: [`Parser::parse_statement_inner`] only offers a
    /// statement here while `in_component` holds, so an ordinary program that happens to
    /// start a sentence with `Save` or `Focus` keeps whatever meaning it had.
    pub(super) fn parse_ui_statement(&mut self) -> ParseResult<Option<Stmt<'a>>> {
        // Typed requests: `Get URL as T, then action.` / `Post BODY to URL as T, then action.`
        for (word, method, has_body) in [
            ("Get", FetchMethod::Get, false),
            ("Delete", FetchMethod::Delete, false),
            ("Post", FetchMethod::Post, true),
            ("Put", FetchMethod::Put, true),
            ("Patch", FetchMethod::Patch, true),
            ("Upload", FetchMethod::Upload, true),
        ] {
            if self.lexeme_is(0, word) {
                self.advance();
                let (body, url) = if has_body {
                    let b = self.parse_imperative_expr()?;
                    if !self.eat_words(&["to"]) {
                        return Err(self.fetch_error(word));
                    }
                    (Some(b), self.parse_imperative_expr()?)
                } else {
                    (None, self.parse_imperative_expr()?)
                };
                if !self.eat_words(&["as"]) {
                    return Err(self.fetch_error(word));
                }
                let ty = self.parse_type_expression()?;
                let as_type = self.ctx.alloc_type_expr(ty);
                if self.check(&TokenType::Comma) {
                    self.advance();
                }
                if !self.eat_words(&["then"]) {
                    return Err(self.fetch_error(word));
                }
                let then = self.expect_identifier()?;
                return Ok(Some(Stmt::Fetch {
                    method,
                    url,
                    body,
                    as_type,
                    then,
                }));
            }
        }
        // Element, navigation, persistence, feedback, store and timer verbs.
        let ui = if self.eat_words(&["Click"]) {
            UiStmt::Click {
                selector: self.parse_imperative_expr()?,
            }
        } else if self.eat_words(&["Submit"]) {
            UiStmt::Submit {
                selector: self.parse_imperative_expr()?,
            }
        } else if self.lexeme_is(0, "Choose") {
            self.advance();
            // TIGHTLY, for the same reason `Press` is: a full expression swallows the
            // `in "#s"` as the postfix zoned-time conversion, turning the selector into a
            // time zone. The second trap of this exact shape in the verb vocabulary.
            let value = self.parse_primary_expr()?;
            if !self.eat_words(&["in"]) {
                return Err(self.verb_error("Choose VALUE in SEL"));
            }
            UiStmt::Choose {
                value,
                selector: self.parse_primary_expr()?,
            }
        } else if self.lexeme_is(0, "Type") {
            self.advance();
            let text = self.parse_imperative_expr()?;
            if !self.eat_words(&["into"]) {
                return Err(self.verb_error("Type TEXT into SEL"));
            }
            UiStmt::TypeInto {
                text,
                selector: self.parse_imperative_expr()?,
            }
        } else if self.eat_words(&["Raise"]) {
            // `Raise "offline".` is the page; `Raise "scrolledtobottom" on "#list".` is one
            // element. One sentence for both, because what differs is only WHERE it happened.
            let event = self.parse_imperative_expr()?;
            // The PAGE is the default target, and it is spelled the same way every page-level
            // listener is resolved — `"body"` — so the driver and the registration name one node.
            let target = if self.eat_words(&["on"]) {
                self.parse_imperative_expr()?
            } else {
                let body = self.interner.intern("body");
                self.ctx
                    .alloc_imperative_expr(Expr::Literal(Literal::Text(body)))
            };
            // ★ AND WHAT THE EVENT CARRIED. Some events bring a value with them — the name of the
            // file that was dropped — and a trigger that BINDS one cannot be caused by a sentence
            // with no way to say it. Empty when the event carries nothing, which is most of them.
            let value = if self.eat_words(&["with"]) {
                self.parse_imperative_expr()?
            } else {
                let empty = self.interner.intern("");
                self.ctx
                    .alloc_imperative_expr(Expr::Literal(Literal::Text(empty)))
            };
            UiStmt::RaiseWindowEvent {
                event,
                target,
                value,
            }
        } else if self.eat_words(&["Focus"]) {
            UiStmt::Focus {
                selector: self.parse_imperative_expr()?,
            }
        } else if self.eat_words(&["Blur"]) {
            UiStmt::Blur {
                selector: self.parse_imperative_expr()?,
            }
        } else if self.eat_words(&["Hover"]) {
            UiStmt::Hover {
                selector: self.parse_imperative_expr()?,
            }
        } else if self.eat_words(&["Scroll", "to"]) {
            UiStmt::ScrollTo {
                selector: self.parse_imperative_expr()?,
            }
        } else if self.lexeme_is(0, "Add") && self.lexeme_is(1, "class") {
            self.advance();
            self.advance();
            let class = self.parse_imperative_expr()?;
            if !self.eat_words(&["to"]) {
                return Err(self.verb_error("Add class C to SEL"));
            }
            UiStmt::AddClass {
                class,
                selector: self.parse_imperative_expr()?,
            }
        } else if self.lexeme_is(0, "Remove") && self.lexeme_is(1, "class") {
            self.advance();
            self.advance();
            let class = self.parse_imperative_expr()?;
            if !self.eat_words(&["from"]) {
                return Err(self.verb_error("Remove class C from SEL"));
            }
            UiStmt::RemoveClass {
                class,
                selector: self.parse_imperative_expr()?,
            }
        } else if self.eat_words(&["Select", "the", "text", "in"]) {
            UiStmt::SelectText {
                selector: self.parse_imperative_expr()?,
            }
        } else if self.lexeme_is(0, "Copy") {
            self.advance();
            let text = self.parse_imperative_expr()?;
            if !self.eat_words(&["to", "the", "clipboard"]) {
                return Err(self.verb_error("Copy TEXT to the clipboard"));
            }
            UiStmt::CopyToClipboard { text }
        } else if self.lexeme_is(0, "Download") {
            self.advance();
            let bytes = self.parse_imperative_expr()?;
            if !self.eat_words(&["as"]) {
                return Err(self.verb_error("Download BYTES as \"name.ext\""));
            }
            UiStmt::Download {
                bytes,
                name: self.parse_imperative_expr()?,
            }
        } else if self.eat_words(&["Navigate", "to"]) {
            UiStmt::Navigate {
                url: self.parse_imperative_expr()?,
            }
        } else if self.eat_words(&["Replace", "the", "URL", "with"]) {
            UiStmt::ReplaceUrl {
                url: self.parse_imperative_expr()?,
            }
        } else if self.eat_words(&["Go", "back"]) {
            UiStmt::GoBack
        } else if self.eat_words(&["Set", "the", "title", "to"]) {
            UiStmt::SetTitle {
                title: self.parse_imperative_expr()?,
            }
        } else if self.lexeme_is(0, "Save") {
            self.advance();
            let value = self.parse_imperative_expr()?;
            if !self.eat_words(&["as"]) {
                return Err(self.verb_error("Save VALUE as \"key\""));
            }
            UiStmt::Save {
                value,
                key: self.parse_imperative_expr()?,
            }
        } else if self.lexeme_is(0, "Load") {
            self.advance();
            let key = self.parse_imperative_expr()?;
            if !self.eat_words(&["into"]) {
                return Err(self.verb_error("Load \"key\" into FIELD"));
            }
            let at = self.current_span();
            let var = self.expect_identifier()?;
            self.note_write_at(var, at, WriteVerb::Load);
            UiStmt::Load { key, var }
        } else if self.lexeme_is(0, "Press") {
            self.advance();
            // The key is parsed TIGHTLY: a full expression would swallow the `in "#draft"` as
            // the postfix zoned-time conversion (`<moment> in "<zone>"`), turning the selector
            // into a time zone.
            let key = self.parse_primary_expr()?;
            // `Press "Escape".` with no element means the PAGE, which is what a page-level
            // `When the user presses Escape:` listens on. Filling that in here means every tier
            // receives the same shape rather than each inventing the default.
            let selector = if self.eat_words(&["in"]) {
                self.parse_primary_expr()?
            } else {
                let body = self.interner.intern("body");
                self.ctx
                    .alloc_imperative_expr(Expr::Literal(Literal::Text(body)))
            };
            UiStmt::Press {
                key,
                selector: Some(selector),
            }
        } else if self.eat_words(&["Reload", "the", "page"]) {
            UiStmt::Reload
        } else if self.eat_words(&["Forget"]) {
            UiStmt::Forget {
                key: self.parse_imperative_expr()?,
            }
        } else if self.lexeme_is(0, "Notify") {
            self.advance();
            let message = self.parse_imperative_expr()?;
            let tone = if self.eat_words(&["as"]) {
                Some(self.expect_identifier()?)
            } else {
                None
            };
            UiStmt::Notify { message, tone }
        } else if self.lexeme_is(0, "Publish") {
            // ★ COMPONENT-ONLY, DELIBERATELY. `Publish` is not in `UI_STATEMENT_OPENERS`, so it
            // keeps whatever meaning it had in an ordinary program — "Publish the results." is a
            // sentence somebody writes. Inside a component `in_component` already opens the gate,
            // which is where a page publishes from anyway.
            self.advance();
            let message = self.parse_imperative_expr()?;
            if !self.eat_words(&["on"]) {
                return Err(self.verb_error("Publish MESSAGE on \"topic\""));
            }
            UiStmt::Publish {
                message,
                topic: self.parse_imperative_expr()?,
            }
        } else if self.eat_words(&["Toggle"]) {
            let at = self.current_span();
            let field = self.expect_identifier()?;
            self.note_write_at(field, at, WriteVerb::Toggle);
            UiStmt::Toggle { field }
        } else if (self.lexeme_is(0, "Stop") || self.lexeme_is(0, "Start"))
            && self.lexeme_is(1, "the")
            && self.lexeme_is(2, "timer")
        {
            let span = self.current_span();
            let start = self.lexeme_is(0, "Start");
            self.advance();
            self.advance();
            self.advance();
            let name = self.expect_identifier()?;
            self.timer_stops.push((name, span));
            if start {
                UiStmt::StartTimer { name }
            } else {
                UiStmt::StopTimer { name }
            }
        } else if self.lexeme_is(0, "Call")
            && self
                .tokens
                .get(self.current + 2)
                .is_some_and(|t| matches!(t.kind, TokenType::Possessive))
        {
            // `Call <Store>'s <action> [with a and b].` — the ONLY way to change a store.
            let span = self.current_span();
            self.advance();
            let store = self.expect_declared_name()?;
            self.advance(); // the possessive
            let action = self.expect_identifier()?;
            let mut args = Vec::new();
            if self.eat_words(&["with"]) {
                loop {
                    // `parse_comparison`, not the full expression grammar: `and` is this loop's
                    // OWN argument separator, exactly the ambiguity `parse_constructor_fields_body`
                    // already resolves the same way. The full grammar reads `and` as the boolean
                    // operator first, so `with "tea" and 300` parsed as ONE argument
                    // (`"tea" and 300`, a `BinaryOp` that evaluates to `true`) — the second
                    // parameter was never bound, and the call silently ran with one argument short.
                    args.push(self.parse_comparison()?);
                    if self.eat_words(&["and"]) || self.check(&TokenType::Comma) {
                        if self.check(&TokenType::Comma) {
                            self.advance();
                        }
                        continue;
                    }
                    break;
                }
            }
            self.store_calls.push((store, action, span));
            UiStmt::CallStore {
                store,
                action,
                args,
            }
        } else {
            return Ok(None);
        };
        Ok(Some(Stmt::Ui(ui)))
    }

    /// Record a write to the place `place`, parsed from tokens `from..to`. The span is the
    /// token spelling the place's root name, so a refusal points at the name, not the sentence.
    pub(super) fn note_write(&mut self, place: &Expr<'a>, from: usize, to: usize, verb: WriteVerb) {
        let Some(root) = place_root(place) else {
            return;
        };
        let span = self.tokens[from.min(self.tokens.len())..to.min(self.tokens.len())]
            .iter()
            .find(|t| t.lexeme == root)
            .or_else(|| self.tokens.get(from))
            .map(|t| t.span)
            .unwrap_or_default();
        self.note_write_at(root, span, verb);
    }

    /// Record a write to `target`, written at `span`. A component's own writes are ALSO checked
    /// against its props; every write, everywhere, is checked at end of parse against the file's
    /// stores (`resolve_store_writes`).
    pub(super) fn note_write_at(&mut self, target: Symbol, span: Span, verb: WriteVerb) {
        if self.in_component {
            self.component_writes
                .push(ComponentWrite { target, span, verb });
        }
        self.all_writes.push((target, span, verb));
    }

    fn fetch_error(&self, word: &str) -> ParseError {
        ParseError {
            kind: ParseErrorKind::Custom(format!(
                "A typed request reads `{word} <url> as <Type>, then <action>.` — with a body for \
                 Post/Put/Patch/Upload: `Post draft to \"/api/todos\" as Seq of Todo, then loaded.` \
                 The action receives `Result of <Type> and HttpError`."
            )),
            span: self.current_span(),
        }
    }

    fn verb_error(&self, shape: &str) -> ParseError {
        ParseError {
            kind: ParseErrorKind::Custom(format!("This action sentence reads `{shape}.`")),
            span: self.current_span(),
        }
    }

    // ── the view: raw block → template → view IR ───────────────────────────────────────

    /// Parse a raw `View:` body into the view IR, sub-lexing every expression it holds at
    /// its own byte offset in the file.
    pub(super) fn view_from_raw(
        &mut self,
        raw: RawBody,
        component: Option<Symbol>,
        span: Span,
    ) -> ParseResult<ViewDef<'a>> {
        let text = self.interner.resolve(raw.body).to_string();
        let map = RawMap {
            text,
            indent: raw.indent,
            content_start: raw.content_start,
        };
        let nodes = template::parse(&map.text).map_err(|e| ParseError {
            kind: ParseErrorKind::Custom(e.message()),
            span: map.span(e.span),
        })?;
        // The template checker: an Error stops the parse where the mistake is; a Warning is
        // collected for the LSP and `largo check`. A finding's span is an offset into the RAW
        // BLOCK, so it is mapped into the file — errors and stored warnings alike — the same way
        // a template parse error is.
        let findings = crate::lgx::check::check(&nodes);
        if let Some(bad) = findings
            .iter()
            .find(|f| f.severity == crate::lgx::check::Severity::Error)
        {
            return Err(ParseError {
                kind: ParseErrorKind::Custom(bad.message.clone()),
                span: map.span(bad.span),
            });
        }
        self.lgx_findings.extend(findings.into_iter().map(|mut f| {
            f.span = map.span(f.span);
            f
        }));
        let root = if component.is_some() {
            ViewRoot::ComponentParent
        } else {
            ViewRoot::Body
        };
        let mentions = self.template_mentions(&nodes, &map);
        let nodes = self.view_nodes_from_template(&nodes, &map)?;
        Ok(ViewDef {
            component,
            root,
            nodes,
            mentions,
            span,
        })
    }

    /// A `Head:` body — the same template machinery a `View:` gets, then the one rule
    /// that is different: what may appear there.
    ///
    /// ★ WHY THE CHECK EXISTS. A browser SILENTLY RELOCATES a body element out of the
    /// head and into the body, where it renders. So `<div>oops</div>` in a `Head:`
    /// produces a page that looks wrong with nothing anywhere saying why — the exact
    /// shape of failure this framework refuses by name everywhere else.
    pub(super) fn head_from_raw(&mut self, raw: RawBody, span: Span) -> ParseResult<ViewDef<'a>> {
        let text = self.interner.resolve(raw.body).to_string();
        let map = RawMap {
            text,
            indent: raw.indent,
            content_start: raw.content_start,
        };
        let nodes = template::parse(&map.text).map_err(|e| ParseError {
            kind: ParseErrorKind::Custom(e.message()),
            span: map.span(e.span),
        })?;
        if let Some((tag, bad_span)) = crate::lgx::check::first_non_head_element(&nodes) {
            return Err(ParseError {
                kind: ParseErrorKind::Custom(format!(
                    "`<{tag}>` cannot go in a `Head:` section. A document's head holds `<title>`, \
                     `<meta>`, `<link>` and `<base>`; a browser moves anything else out of it and \
                     renders it in the body, so this would show up somewhere you did not put it. \
                     Move `<{tag}>` to the `View:`."
                )),
                span: map.span(bad_span),
            });
        }
        let mentions = self.template_mentions(&nodes, &map);
        let nodes = self.view_nodes_from_template(&nodes, &map)?;
        Ok(ViewDef {
            component: None,
            root: ViewRoot::Head,
            nodes,
            mentions,
            span,
        })
    }

    /// Every identifier the template mentions, at its span in the FILE.
    ///
    /// ★ SCANNED OUT OF THE EXPRESSION TEXT, not out of the parsed expression, because the parsed
    /// one has thrown the positions away: `Expr::Identifier(Symbol)` carries a name and nothing
    /// else. `TExpr` is the last place where a name still knows WHERE it was written — its `text`
    /// plus its `offset` in the template body — and `RawMap` turns that into a file offset.
    ///
    /// ★ AND A LOOSE SCAN IS THE RIGHT INSTRUMENT HERE. It will also pick up a filter name, a
    /// field after a dot, a `true`; none of them resolve to a definition, and an unresolved
    /// mention is simply not a reference — the index already drops those. Over-collecting costs a
    /// lookup that fails; under-collecting costs the author a jump that silently does nothing.
    fn template_mentions(&mut self, nodes: &[TNode], map: &RawMap) -> Vec<(Symbol, Span)> {
        let mut out = Vec::new();
        collect_template_exprs(nodes, &mut |e: &crate::lgx::template::TExpr| {
            for (name, at) in identifiers_in(&e.text) {
                let start = map.offset(e.offset + at);
                let end = map.offset(e.offset + at + name.len());
                out.push((self.interner.intern(name), Span::new(start, end)));
            }
        });
        // A `bind={field}` names a state field directly rather than through an expression.
        collect_bind_fields(nodes, &mut |field: &str, at: usize| {
            out.push((
                self.interner.intern(field),
                Span::new(map.offset(at), map.offset(at + field.len())),
            ));
        });
        // ★ A PLACED COMPONENT'S NAME IS A MENTION TOO, and it is not in any expression — `<Card/>`
        // holds the name in the NODE. Jumping from a placement to the `## Component` it names is
        // how a reader follows a page apart, so leaving it out would have covered every hole and
        // still not answered the question most worth asking of a template.
        collect_component_names(nodes, &mut |name: &str, at: usize| {
            // `at` is the element's `<`; the name begins one byte later.
            let start = map.offset(at + 1);
            let end = map.offset(at + 1 + name.len());
            out.push((self.interner.intern(name), Span::new(start, end)));
        });
        out
    }

    fn view_nodes_from_template(
        &mut self,
        nodes: &[TNode],
        map: &RawMap,
    ) -> ParseResult<Vec<ViewNode<'a>>> {
        let mut out = Vec::with_capacity(nodes.len());
        for n in nodes {
            out.push(self.view_node_from_template(n, map)?);
        }
        Ok(out)
    }

    fn view_node_from_template(&mut self, node: &TNode, map: &RawMap) -> ParseResult<ViewNode<'a>> {
        let sp = |s: crate::token::Span| map.span(s);
        Ok(match node {
            TNode::Text { text, span } => {
                let sym = self.interner.intern(text);
                ViewNode::Text {
                    text: self
                        .ctx
                        .alloc_imperative_expr(Expr::Literal(Literal::Text(sym))),
                    span: sp(*span),
                }
            }
            TNode::Hole {
                expr,
                filters,
                raw,
                span,
            } => {
                let mut value = self.expr_from_texpr(expr, map)?;
                for f in filters {
                    let name = self.interner.intern(&f.name);
                    let mut args = vec![value];
                    for a in &f.args {
                        args.push(self.expr_from_texpr(a, map)?);
                    }
                    value = self.ctx.alloc_imperative_expr(Expr::Call {
                        function: name,
                        args,
                    });
                }
                if *raw {
                    ViewNode::Raw {
                        html: value,
                        span: sp(*span),
                    }
                } else {
                    ViewNode::Text {
                        text: value,
                        span: sp(*span),
                    }
                }
            }
            TNode::Element {
                tag,
                attrs,
                children,
                span,
            } => {
                let tag_sym = self.interner.intern(tag);
                let (view_attrs, mut handlers) = self.attrs_from_template(attrs, map)?;
                let mut kids = self.view_nodes_from_template(children, map)?;
                handlers.append(&mut kids);
                ViewNode::Element {
                    tag: tag_sym,
                    attrs: view_attrs,
                    children: handlers,
                    span: sp(*span),
                }
            }
            TNode::Component {
                name,
                props,
                children,
                span,
            } => {
                let name_sym = self.interner.intern(name);
                let mut prop_pairs = Vec::with_capacity(props.len());
                let mut prop_spans = Vec::with_capacity(props.len());
                for prop in props {
                    let (prop_name, value, at) = match prop {
                        TAttr::Static { name, value, span } => {
                            let text = self.interner.intern(value.as_deref().unwrap_or(""));
                            (
                                name,
                                self.ctx
                                    .alloc_imperative_expr(Expr::Literal(Literal::Text(text))),
                                *span,
                            )
                        }
                        TAttr::Dynamic { name, parts, span } => {
                            (name, self.expr_from_attr_parts(parts, map)?, *span)
                        }
                        TAttr::Bind { span, .. }
                        | TAttr::Class { span, .. }
                        | TAttr::On { span, .. } => {
                            let what = match prop {
                                TAttr::Bind { .. } => "`bind`".to_string(),
                                TAttr::Class { name: class, .. } => format!("`class:{class}`"),
                                TAttr::On { event, .. } => format!("`on-{event}`"),
                                _ => unreachable!("matched above"),
                            };
                            return Err(ParseError {
                                kind: ParseErrorKind::Custom(format!(
                                    "`<{name}>` is a component, and {what} belongs on an HTML element: a \
                                     component takes props. Pass the value as a prop (a callback prop \
                                     for an event, `fn(…) -> Unit`) and use it inside `{name}`'s own view."
                                )),
                                span: sp(*span),
                            });
                        }
                    };
                    prop_pairs.push((self.interner.intern(prop_name), value));
                    prop_spans.push(sp(at));
                }
                ViewNode::Component {
                    name: name_sym,
                    props: prop_pairs,
                    prop_spans,
                    children: self.view_nodes_from_template(children, map)?,
                    span: sp(*span),
                }
            }
            TNode::If {
                arms,
                otherwise,
                span,
            } => {
                let mut view_arms = Vec::with_capacity(arms.len());
                for (cond, body) in arms {
                    view_arms.push((
                        self.expr_from_texpr(cond, map)?,
                        self.view_nodes_from_template(body, map)?,
                    ));
                }
                let otherwise = match otherwise {
                    Some(o) => Some(self.view_nodes_from_template(o, map)?),
                    None => None,
                };
                ViewNode::If {
                    arms: view_arms,
                    otherwise,
                    span: sp(*span),
                }
            }
            TNode::For {
                pattern,
                iterable,
                key,
                index,
                body,
                empty,
                span,
            } => {
                let pat = self.pattern_from_text(pattern, sp(*span))?;
                let iterable = self.expr_from_texpr(iterable, map)?;
                let key = match key {
                    Some(k) => Some(self.expr_from_texpr(k, map)?),
                    None => None,
                };
                let index = index.as_ref().map(|i| self.interner.intern(i));
                let body = self.view_nodes_from_template(body, map)?;
                let empty = match empty {
                    Some(e) => self.view_nodes_from_template(e, map)?,
                    None => Vec::new(),
                };
                ViewNode::Each {
                    pattern: pat,
                    iterable,
                    key,
                    index,
                    body,
                    empty,
                    span: sp(*span),
                }
            }
            TNode::Inspect {
                scrutinee,
                arms,
                span,
            } => {
                let scrutinee = self.expr_from_texpr(scrutinee, map)?;
                let mut view_arms = Vec::with_capacity(arms.len());
                for (pat, body) in arms {
                    let (variant, bindings) = self.match_pattern_from_text(pat);
                    view_arms.push(ViewArm {
                        variant,
                        bindings,
                        body: self.view_nodes_from_template(body, map)?,
                    });
                }
                ViewNode::Inspect {
                    scrutinee,
                    arms: view_arms,
                    span: sp(*span),
                }
            }
            TNode::Let { name, expr, span } => ViewNode::Let {
                var: self.interner.intern(name),
                value: self.expr_from_texpr(expr, map)?,
                span: sp(*span),
            },
            TNode::Extends { name, span } => ViewNode::Extends {
                layout: self.interner.intern(name),
                span: sp(*span),
            },
            TNode::Block {
                name,
                default,
                span,
            } => {
                let default = match default {
                    Some(d) => Some(self.view_nodes_from_template(d, map)?),
                    None => None,
                };
                ViewNode::Block {
                    name: self.interner.intern(name),
                    default,
                    span: sp(*span),
                }
            }
            TNode::Children { span } => ViewNode::Children { span: sp(*span) },
        })
    }

    /// Template attributes → view-IR attributes, with `on-…` directives split out as the
    /// handler nodes the lowering attaches to the element.
    fn attrs_from_template(
        &mut self,
        attrs: &[TAttr],
        map: &RawMap,
    ) -> ParseResult<(Vec<Attr<'a>>, Vec<ViewNode<'a>>)> {
        let mut out = Vec::with_capacity(attrs.len());
        let mut handlers = Vec::new();
        for a in attrs {
            match a {
                TAttr::Static { name, value, .. } => {
                    // Left as `Plain`; `resolve_directive_attributes` reclassifies the declared
                    // directive names once the whole file is parsed, so order does not matter.
                    let name_sym = self.interner.intern(name);
                    let text = self.interner.intern(value.as_deref().unwrap_or(""));
                    out.push(Attr::Plain {
                        name: name_sym,
                        value: self
                            .ctx
                            .alloc_imperative_expr(Expr::Literal(Literal::Text(text))),
                    });
                }
                TAttr::Dynamic { name, parts, span } => {
                    let name_sym = self.interner.intern(name);
                    let value = self.expr_from_attr_parts(parts, map)?;
                    if self.directives.contains(&name_sym) {
                        out.push(Attr::Directive {
                            name: name_sym,
                            value,
                        });
                    } else {
                        let _ = span;
                        out.push(Attr::Plain {
                            name: name_sym,
                            value,
                        });
                    }
                }
                TAttr::Bind { field, .. } => out.push(Attr::Bind {
                    field: self.interner.intern(field),
                }),
                TAttr::Class { name, expr, .. } => out.push(Attr::Class {
                    name: self.interner.intern(name),
                    cond: self.expr_from_texpr(expr, map)?,
                }),
                TAttr::On {
                    event,
                    modifiers,
                    expr,
                    span,
                } => {
                    let event_sym = self.interner.intern(event);
                    let mods = modifiers.clone();
                    let call = self.expr_from_texpr(expr, map)?;
                    let stmt = match call {
                        Expr::Identifier(name) => Stmt::Call { function: *name, args: Vec::new() },
                        Expr::Call { function, args } => Stmt::Call { function: *function, args: args.clone() },
                        _ => {
                            return Err(ParseError {
                                kind: ParseErrorKind::Custom(format!(
                                    "`on-{event}={{…}}` names an action to run: `on-{event}={{add}}` or \
                                     `on-{event}={{remove(todo's id)}}`."
                                )),
                                span: map.span(*span),
                            })
                        }
                    };
                    let body = self.alloc_block(vec![stmt]);
                    handlers.push(ViewNode::Handler {
                        event: event_sym,
                        modifiers: mods,
                        body,
                        span: map.span(*span),
                    });
                }
            }
        }
        Ok((out, handlers))
    }

    /// A dynamic attribute value: one `{expr}` is that expression; text and holes mixed
    /// become an interpolated string.
    fn expr_from_attr_parts(
        &mut self,
        parts: &[AttrPart],
        map: &RawMap,
    ) -> ParseResult<&'a Expr<'a>> {
        if let [AttrPart::Brace { inner, span }] = parts {
            return self.expr_from_texpr(
                &TExpr {
                    text: inner.clone(),
                    offset: span.start + 1,
                },
                map,
            );
        }
        let mut string_parts = Vec::with_capacity(parts.len());
        for p in parts {
            match p {
                AttrPart::Text(t) => {
                    string_parts.push(StringPart::Literal(self.interner.intern(t)))
                }
                AttrPart::Brace { inner, span } => {
                    let value = self.expr_from_texpr(
                        &TExpr {
                            text: inner.clone(),
                            offset: span.start + 1,
                        },
                        map,
                    )?;
                    string_parts.push(StringPart::Expr {
                        value,
                        format_spec: None,
                        debug: false,
                    });
                }
            }
        }
        Ok(self
            .ctx
            .alloc_imperative_expr(Expr::InterpolatedString(string_parts)))
    }

    /// A loop pattern from its template text (`todo`, or `(k, v)`).
    fn pattern_from_text(&mut self, text: &str, span: Span) -> ParseResult<Pattern> {
        let t = text.trim();
        if let Some(inner) = t.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
            let names: Vec<Symbol> = inner
                .split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(|p| self.interner.intern(p))
                .collect();
            if names.is_empty() {
                return Err(ParseError {
                    kind: ParseErrorKind::Custom("a loop needs an item name".to_string()),
                    span,
                });
            }
            return Ok(Pattern::Tuple(names));
        }
        if t.is_empty() || t.contains(char::is_whitespace) {
            return Err(ParseError {
                kind: ParseErrorKind::Custom(format!(
                    "`{t}` is not an item name: write `{{for each todo in todos:}}`, or `{{for each (key, value) in map:}}` to take both."
                )),
                span,
            });
        }
        Ok(Pattern::Identifier(self.interner.intern(t)))
    }

    /// A `{when …:}` arm's pattern: `Ok (v)` / `JObj (entries)` / `Otherwise`.
    fn match_pattern_from_text(&mut self, text: &str) -> (Option<Symbol>, Vec<(Symbol, Symbol)>) {
        let t = text.trim();
        if t.eq_ignore_ascii_case("otherwise") {
            return (None, Vec::new());
        }
        let (head, rest) = match t.find('(') {
            Some(i) => (t[..i].trim(), Some(t[i + 1..].trim_end_matches(')'))),
            None => (t, None),
        };
        let variant = self.interner.intern(head);
        let bindings = rest
            .map(|r| {
                r.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        let sym = self.interner.intern(s);
                        (sym, sym)
                    })
                    .collect()
            })
            .unwrap_or_default();
        (Some(variant), bindings)
    }

    /// Parse one template expression with the REAL expression parser, its spans shifted to
    /// where the text sits in the file.
    fn expr_from_texpr(&mut self, e: &TExpr, map: &RawMap) -> ParseResult<&'a Expr<'a>> {
        self.parse_expr_snippet(&e.text, map.offset(e.offset))
    }

    /// Sub-lex `text` as CODE and parse it as one expression; every span is shifted by
    /// `offset` so an error inside a hole points at the hole.
    fn parse_expr_snippet(&mut self, text: &str, offset: usize) -> ParseResult<&'a Expr<'a>> {
        let mut tokens = {
            let mut lexer = Lexer::new_imperative(text, &mut *self.interner);
            lexer.tokenize()
        };
        for t in &mut tokens {
            t.span = Span::new(t.span.start + offset, t.span.end + offset);
        }
        let saved_tokens = std::mem::replace(&mut self.tokens, tokens);
        let saved_current = std::mem::replace(&mut self.current, 0);
        let saved_mode = std::mem::replace(&mut self.mode, ParserMode::Imperative);
        let parsed = self.parse_imperative_expr();
        // Anything left over means the text is not ONE expression.
        let leftover = !self.is_at_end()
            && !self.check(&TokenType::Newline)
            && !self.check(&TokenType::Period);
        let leftover_span = self.current_span();
        self.tokens = saved_tokens;
        self.current = saved_current;
        self.mode = saved_mode;
        // A hole that is exactly ONE word is a variable reference. Taking it as such rescues the
        // names that also open a statement form — `{item}` would otherwise start `item N of xs`
        // and demand an `of` — without changing any snippet that already parses.
        let single_word = {
            let t = text.trim();
            !t.is_empty()
                && t.chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !matches!(t, "true" | "false" | "nothing" | "none" | "some")
        };
        if single_word && (parsed.is_err() || leftover) {
            let sym = self.interner.intern(text.trim());
            return Ok(self.ctx.alloc_imperative_expr(Expr::Identifier(sym)));
        }
        let expr = parsed?;
        if leftover {
            return Err(ParseError {
                kind: ParseErrorKind::Custom(format!(
                    "`{}` is not one LOGOS expression — a hole holds a single value, like `{{name}}`, \
                     `{{length of todos}}` or `{{todo's text}}`.",
                    text.trim()
                )),
                span: leftover_span,
            });
        }
        Ok(expr)
    }

    // ── small shared helpers ───────────────────────────────────────────────────────────

    /// `(a: T) and (b: U)` parameter groups, or nothing.
    fn parse_param_groups(&mut self) -> ParseResult<Vec<(Symbol, &'a TypeExpr<'a>)>> {
        let mut params = Vec::new();
        while self.check(&TokenType::LParen) {
            self.advance();
            if self.check(&TokenType::RParen) {
                self.advance();
                break;
            }
            loop {
                let name = self.expect_identifier()?;
                if !self.check(&TokenType::Colon) {
                    return Err(ParseError {
                        kind: ParseErrorKind::ExpectedKeyword {
                            keyword: ":".to_string(),
                        },
                        span: self.current_span(),
                    });
                }
                self.advance();
                let ty = self.parse_type_expression()?;
                params.push((name, self.ctx.alloc_type_expr(ty)));
                if self.check(&TokenType::Comma) {
                    self.advance();
                    continue;
                }
                break;
            }
            if !self.check(&TokenType::RParen) {
                return Err(ParseError {
                    kind: ParseErrorKind::ExpectedKeyword {
                        keyword: ")".to_string(),
                    },
                    span: self.current_span(),
                });
            }
            self.advance();
            if self.check_word("and") || self.check(&TokenType::And) {
                self.advance();
            }
        }
        Ok(params)
    }

    fn expect_colon(&mut self) -> ParseResult<()> {
        if !self.check(&TokenType::Colon) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedKeyword {
                    keyword: ":".to_string(),
                },
                span: self.current_span(),
            });
        }
        self.advance();
        Ok(())
    }

    fn expect_indent(&mut self) -> ParseResult<()> {
        if !self.check(&TokenType::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedStatement,
                span: self.current_span(),
            });
        }
        self.advance();
        Ok(())
    }

    /// Whether the token `offset` ahead spells `word` (case-insensitively), whatever its
    /// token type — trigger sentences are prose, not keywords.
    pub(super) fn lexeme_is(&self, offset: usize, word: &str) -> bool {
        self.tokens
            .get(self.current + offset)
            .is_some_and(|t| self.interner.resolve(t.lexeme).eq_ignore_ascii_case(word))
    }

    /// Consume `words` if they all appear, in order, from the cursor.
    fn eat_words(&mut self, words: &[&str]) -> bool {
        if words.iter().enumerate().all(|(i, w)| self.lexeme_is(i, w)) {
            for _ in words {
                self.advance();
            }
            return true;
        }
        false
    }

    fn next_is_colon(&self) -> bool {
        self.tokens
            .get(self.current + 1)
            .is_some_and(|t| matches!(t.kind, TokenType::Colon))
    }

    /// Peek an `Indent RawBlock Dedent` run without consuming it.
    fn peek_raw_block_after_indent(&self) -> Option<RawBody> {
        if !matches!(self.tokens.get(self.current)?.kind, TokenType::Indent) {
            return None;
        }
        match self.tokens.get(self.current + 1)?.kind {
            TokenType::RawBlock {
                kind,
                body,
                content_start,
                indent,
            } => Some(RawBody {
                kind,
                body,
                content_start,
                indent,
            }),
            _ => None,
        }
    }

    /// Consume an `Indent RawBlock Dedent` run.
    /// Whether a function's declared return type is `View` — the mark of a template macro.
    pub(super) fn returns_a_view(&self, return_type: Option<&'a TypeExpr<'a>>) -> bool {
        matches!(
            return_type,
            Some(TypeExpr::Named(name) | TypeExpr::Primitive(name))
                if self.interner.resolve(*name) == "View"
        )
    }

    /// The raw template body of a `-> View` function, when one follows.
    pub(super) fn consume_view_body(&mut self) -> Option<RawBody> {
        if self.peek_raw_block_after_indent()?.kind != RawKind::View {
            return None;
        }
        self.consume_raw_block()
    }

    /// The refusal of a `View:` whose body the lexer did not capture as markup, pointing at the
    /// first word of the body — the line that decided it — or at the `View:` word when the section
    /// has no indented body at all. The lexer captures a view body only when its first line opens
    /// with a tag or a template tag, so what arrives here is words; a first line in the retired
    /// `Render:` dialect is refused by name, in the words the template uses for the same line
    /// anywhere else.
    fn view_body_is_not_markup(&self, view_span: Span) -> ParseError {
        let is_break = |kind: &TokenType| {
            matches!(
                kind,
                TokenType::Newline | TokenType::Indent | TokenType::Dedent
            )
        };
        let first = self.tokens[self.current..]
            .iter()
            .position(|t| !is_break(&t.kind))
            .map(|i| self.current + i)
            .filter(|&i| {
                self.tokens[self.current..i]
                    .iter()
                    .any(|t| matches!(t.kind, TokenType::Indent))
                    && !matches!(
                        self.tokens[i].kind,
                        TokenType::EOF | TokenType::BlockHeader { .. }
                    )
            });
        let words = first.map_or(&[][..], |i| &self.tokens[i..]);
        let span = words.first().map_or(view_span, |t| t.span);
        if let Some(form) = self.retired_first_line(words) {
            let e = template::TemplateError {
                kind: template::TemplateErrorKind::RetiredViewForm(form),
                span,
            };
            return ParseError {
                kind: ParseErrorKind::Custom(e.message()),
                span: e.span,
            };
        }
        ParseError {
            kind: ParseErrorKind::Custom(
                "A `View:` section holds HTML: its first line must start with a tag (`<div>`) \
                 or a template tag (`{if …:}`, `{extends …}`)."
                    .to_string(),
            ),
            span,
        }
    }

    /// The retired `Render:` form a view's body opens with, read from its first words: an HTML
    /// element name, optionally a bracketed attribute list, then `:`; or `css` then a string.
    fn retired_first_line(&self, words: &[crate::token::Token]) -> Option<template::RetiredForm> {
        let head = self.interner.resolve(words.first()?.lexeme);
        let next = words.get(1).map(|t| &t.kind);
        if head == "css" {
            return matches!(
                next,
                Some(TokenType::StringLiteral(_) | TokenType::InterpolatedString(_))
            )
            .then_some(template::RetiredForm::Stylesheet);
        }
        if !crate::lgx::html::is_html_element(head) || head.chars().any(|c| c.is_ascii_uppercase())
        {
            return None;
        }
        let colon = match next {
            Some(TokenType::LBracket) => {
                words
                    .iter()
                    .take(32)
                    .position(|t| matches!(t.kind, TokenType::RBracket))?
                    + 1
            }
            _ => 1,
        };
        matches!(words.get(colon).map(|t| &t.kind), Some(TokenType::Colon))
            .then(|| template::RetiredForm::Element(head.to_string()))
    }

    fn consume_raw_block(&mut self) -> Option<RawBody> {
        let found = self.peek_raw_block_after_indent()?;
        self.advance(); // Indent
        self.advance(); // RawBlock
        if self.check(&TokenType::Dedent) {
            self.advance();
        }
        Some(found)
    }
}

/// Whether `tag` names a component use rather than an HTML element.
pub(super) fn tag_is_component(tag: &str) -> bool {
    is_component_name(tag)
}

impl<'a> super::Parser<'a, '_, '_> {
    /// Every UI `Mount <Name>` names a component this file declares.
    ///
    /// Checked at END of parse, so a `## Main` may sit above the `## Component` it mounts. The
    /// refusal names the component that was written, the nearest declared one, and every
    /// component the file has; a capitalised name mounted at a file path also shows the
    /// CRDT-journal sentence it resembles, whose variable is lowercase.
    pub(super) fn check_mounts_name_something(&self) -> ParseResult<()> {
        for site in &self.mount_sites {
            if self.components.contains(&site.name) {
                continue;
            }
            let written = self.interner.resolve(site.name);
            let mut known: Vec<&str> = self
                .components
                .iter()
                .map(|c| self.interner.resolve(*c))
                .filter(|n| !n.starts_with("__page_"))
                .collect();
            known.sort_unstable();
            let guess = crate::suggest::did_you_mean(written, known.iter().copied())
                .map(|g| format!(" — did you mean `{g}`?"))
                .unwrap_or_default();
            // A page is real but has no name to mount, so "there is nothing to mount" would be a
            // lie: say that it mounts itself, which makes the sentence one to delete.
            let declared = if self
                .lgx_components
                .iter()
                .any(|c| self.interner.resolve(*c).starts_with("__page_"))
            {
                " This file declares a `## Page`, which mounts itself — delete the `Mount`."
                    .to_string()
            } else if known.is_empty() {
                " This file declares no components.".to_string()
            } else {
                format!(
                    " This file declares {}.",
                    known
                        .iter()
                        .map(|k| format!("`{k}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            let journal = match &site.target {
                Some(path) if !crate::lgx::html::is_selector_shaped(path) => format!(
                    " (A CRDT journal is mounted by a lowercase variable: `Mount {} at \"{path}\".`)",
                    written.to_lowercase()
                ),
                _ => String::new(),
            };
            return Err(ParseError {
                kind: ParseErrorKind::Custom(format!(
                    "`Mount {written}`: no component named `{written}`{guess}{declared}{journal}"
                )),
                span: site.span,
            });
        }
        Ok(())
    }

    /// A root mount cannot supply props, so mounting a component that declares any is refused.
    ///
    /// A root mount renders into the document; there is no caller and no placement, so there is
    /// nowhere for a prop to come from — `__mount_<C>` takes a parent and nothing else. A
    /// component with props is a piece a page places (`<Row note={n}/>`), and the refusal says so.
    pub(super) fn check_mounted_components_need_no_props(
        &self,
        statements: &[Stmt<'a>],
    ) -> ParseResult<()> {
        for site in &self.mount_sites {
            let (name, span) = (&site.name, &site.span);
            let Some(def) = statements.iter().find_map(|s| match s {
                Stmt::ComponentDef(d) if d.name == *name => Some(d),
                _ => None,
            }) else {
                continue;
            };
            if def.props.is_empty() {
                continue;
            }
            let props: Vec<&str> = def
                .props
                .iter()
                .map(|(p, _)| self.interner.resolve(*p))
                .collect();
            return Err(ParseError {
                kind: ParseErrorKind::Custom(format!(
                    "`Mount {0}` cannot run: `{0}` takes {1} ({2}), and a mount has no way to pass \
                     one — it renders into the document, so there is no caller to supply them. A \
                     component with props is a piece a page PLACES: put `<{0} {3}={{…}}/>` in \
                     another component's `View:` and mount that one.",
                    self.interner.resolve(*name),
                    if props.len() == 1 { "a prop" } else { "props" },
                    props.join(", "),
                    props[0]
                )),
                span: *span,
            });
        }
        Ok(())
    }

    /// A page mounts itself: it is the root of the document its URL serves, and its lowered name
    /// (`__page__about`) appears nowhere in the source for a `Mount` to name.
    ///
    /// An explicit root mount still wins — it says where the file begins. A file serves one URL,
    /// so a second page is refused.
    pub(super) fn mount_the_declared_page(
        &mut self,
        statements: &mut Vec<Stmt<'a>>,
    ) -> ParseResult<()> {
        let pages: Vec<(Symbol, Symbol, Span)> = statements
            .iter()
            .filter_map(|s| match s {
                Stmt::ComponentDef(def) => match &def.kind {
                    ComponentKind::Page { path, .. } => Some((def.name, *path, def.span)),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        if let [first, second, ..] = pages.as_slice() {
            return Err(ParseError {
                kind: ParseErrorKind::Custom(format!(
                    "this file declares two pages, `## Page {}` and `## Page {}`, but a file serves \
                     ONE URL — only the first would ever be reached. Give each page its own file.",
                    self.interner.resolve(first.1),
                    self.interner.resolve(second.1)
                )),
                span: second.2,
            });
        }
        let [(name, _, span)] = pages.as_slice() else {
            return Ok(());
        };
        // A `## Test:` mounts to drive a scenario; only a mount in the PROGRAM says where it
        // begins, so a page file with tests in it still mounts its page.
        if self.mount_sites.iter().any(|site| site.is_root) {
            return Ok(());
        }
        // The mount goes immediately before the first statement that acts on the page (a driving
        // verb needs a rendered document), and at the end when there is none — after the file's
        // top-level bindings, which a page's `State:` may read.
        let at = statements
            .iter()
            .position(|s| matches!(s, Stmt::Ui(_)))
            .unwrap_or(statements.len());
        statements.insert(
            at,
            Stmt::Ui(crate::ast::stmt::UiStmt::Mount {
                component: *name,
                selector: None,
            }),
        );
        self.stmt_spans.insert(at, *span);
        Ok(())
    }

    pub(super) fn resolve_lgx(&mut self, statements: &mut [Stmt<'a>]) -> ParseResult<()> {
        self.infer_state_field_types(statements);
        self.resolve_store_calls()?;
        self.resolve_uses()?;
        self.resolve_store_writes()?;
        self.resolve_journal_actions(statements)?;
        self.resolve_bare_action_typos(statements)?;
        self.resolve_directive_attributes(statements);
        self.resolve_views(statements)?;
        self.check_selectors_exist(statements)?;
        self.check_wire_contract(statements)?;
        // Lowering runs LAST and over the whole file, so a component may render a child declared
        // below it and a child's own lowerability is known before its parent asks. A `-> View`
        // function is data before anything draws it, and its served twin draws that data.
        self.lower_view_functions(statements)?;
        self.lower_view_function_twins(statements);
        self.read_views_as_markup(statements);
        self.lower_lgx_components(statements)?;
        // The session's global and its two movers are built BEFORE the router, because the walks'
        // shared prelude calls the load and the handler calls the save.
        self.lower_session()?;
        self.lower_lgx_routes(statements);
        self.lower_derived_json(statements);
        Ok(())
    }

    /// ★ THE COMPILER CHECKS THE WIRE — a page's `Get … as T` must agree with the `## Route`
    /// that answers it.
    ///
    /// Both declarations are in ONE program, in one language, in one compilation unit:
    ///
    /// ```text
    /// ## Route GET /api/todos -> Seq of Todo:          (the server's promise)
    /// Get "/api/todos" as Seq of Todo, then loaded.    (the page's expectation)
    /// ```
    ///
    /// Nothing compared them, so a page fetching `Seq of User` from a route answering
    /// `Seq of Todo` compiled clean on every tier and failed at RUN time, in a browser, in front
    /// of the person using it, as a decode error naming neither side. This is what tRPC and
    /// GraphQL codegen exist to provide for TypeScript; here it is a table lookup, because the
    /// client and the server were never separate programs.
    ///
    /// ★ ABSENCE IS NOT DISAGREEMENT. Most pages talk to something this program does not
    /// compile — a third-party API, another service — so a URL no route answers is fine.
    /// Only a route that EXISTS and answers something ELSE is an error. Refusing the unknown
    /// case would make the check unusable and teach people to switch it off.
    ///
    /// ★ AND IT ONLY SPEAKS WHEN IT IS SURE: a route whose path carries a `:param` needs the
    /// runtime matcher to say which URL it answers, and a type shape this cannot render is
    /// skipped rather than guessed at. An under-approximation misses a mistake; an
    /// over-approximation accuses correct code, and only one of those gets the check turned off.
    fn check_wire_contract(&self, statements: &[Stmt<'a>]) -> ParseResult<()> {
        let mut routes: Vec<(String, String, String)> = Vec::new();
        for s in statements {
            let Stmt::RouteDef {
                method,
                path,
                return_type,
                ..
            } = s
            else {
                continue;
            };
            let path_s = self.interner.resolve(*path).to_string();
            if path_s.contains(':') {
                continue;
            }
            let Some(rt) = return_type else { continue };
            let name = wire_type_name(rt, self.interner);
            if name.is_empty() {
                continue;
            }
            routes.push((self.interner.resolve(*method).to_uppercase(), path_s, name));
        }
        if routes.is_empty() {
            return Ok(());
        }
        for s in statements {
            let Stmt::ComponentDef(def) = s else { continue };
            for a in &def.actions {
                self.check_fetches_in(a.body, &routes, a.span)?;
            }
            for e in &def.events {
                self.check_fetches_in(e.body, &routes, e.span)?;
            }
        }
        Ok(())
    }

    /// Walk a block for `Stmt::Fetch`, descending the containers a fetch is realistically written
    /// inside. A fetch this walk does not reach is simply unchecked — the safe direction. `at` is
    /// the enclosing action or event handler, reported since the fetch statement carries no span
    /// of its own.
    fn check_fetches_in(
        &self,
        block: crate::ast::stmt::Block<'a>,
        routes: &[(String, String, String)],
        at: Span,
    ) -> ParseResult<()> {
        for st in block {
            match st {
                Stmt::Fetch {
                    method,
                    url,
                    as_type,
                    ..
                } => {
                    let Expr::Literal(Literal::Text(u)) = url else {
                        continue;
                    };
                    let full = self.interner.resolve(*u).to_string();
                    // A URL is a path AND a query; a route matches on the path.
                    let path = full.split('?').next().unwrap_or(&full).to_string();
                    let m = match method {
                        crate::ast::stmt::FetchMethod::Get => "GET",
                        crate::ast::stmt::FetchMethod::Post
                        | crate::ast::stmt::FetchMethod::Upload => "POST",
                        crate::ast::stmt::FetchMethod::Put => "PUT",
                        crate::ast::stmt::FetchMethod::Patch => "PATCH",
                        crate::ast::stmt::FetchMethod::Delete => "DELETE",
                    };
                    let Some((_, _, answers)) =
                        routes.iter().find(|(rm, rp, _)| rm == m && *rp == path)
                    else {
                        continue; // no local route answers this — a third party may
                    };
                    let asks = wire_type_name(as_type, self.interner);
                    if asks.is_empty() || &asks == answers {
                        continue;
                    }
                    return Err(ParseError {
                        kind: ParseErrorKind::Custom(format!(
                            "this page fetches `{path}` as `{asks}`, but the `## Route {m} {path}` \
                             in this same program answers `{answers}`. One of the two is wrong, and \
                             they cannot both ship: change the `as` clause to `{answers}`, or change \
                             what the route returns."
                        )),
                        span: at,
                    });
                }
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    self.check_fetches_in(then_block, routes, at)?;
                    if let Some(e) = else_block {
                        self.check_fetches_in(e, routes, at)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Every `When "<selector>" is <event>:` names an element some view actually renders.
    ///
    /// Checked over the WHOLE reachable tree — the component's own view plus the views of the
    /// components it renders and the layout it extends — because a page legitimately listens on
    /// an element a child draws. The check runs only when that tree is fully KNOWN: a `{raw}`
    /// block, a `{children}` slot, or a computed `id=`/`class=` means markup this parse cannot
    /// see, and a check that cannot see everything must not accuse anyone.
    fn check_selectors_exist(&self, statements: &[Stmt<'a>]) -> ParseResult<()> {
        use crate::lgx::selector;
        let defs: Vec<&ComponentDef<'a>> = statements
            .iter()
            .filter_map(|s| match s {
                Stmt::ComponentDef(d) => Some(&**d),
                _ => None,
            })
            .collect();
        for def in &defs {
            let mut surface = Surface::default();
            let mut seen = Vec::new();
            self.collect_surface(def, &defs, &mut seen, &mut surface);
            if surface.unknown {
                continue;
            }
            for handler in &def.events {
                let Trigger::Selector { selector, .. } = &handler.trigger else {
                    continue;
                };
                let Expr::Literal(Literal::Text(text)) = selector else {
                    continue;
                };
                let text = self.interner.resolve(*text).to_string();
                // ★ THE OTHER READER OF THE SAME PARSE, and it used to `continue` past exactly the
                // selectors it could not understand — so a handler on `ul > li` bound to nothing
                // and the one component that could have said so skipped it.
                if let Some(why) = selector::unsupported(&text) {
                    return Err(ParseError {
                        kind: ParseErrorKind::Custom(why),
                        span: handler.span,
                    });
                }
                let Some(parsed) = selector::parse(&text) else {
                    continue;
                };
                let Some(missing) = surface.first_unmatched(&parsed) else {
                    continue;
                };
                return Err(ParseError {
                    kind: ParseErrorKind::Custom(format!(
                        "`{text}` names no element this page renders — nothing here has {missing}. \
                         Check it against the ids, classes and tags in the `View:` section, or \
                         attach the handler with an `on-<event>={{action}}` directive on the \
                         element itself.",
                    )),
                    span: handler.span,
                });
            }
        }
        Ok(())
    }

    /// Everything the component's own view — and every view it reaches — is known to render.
    fn collect_surface(
        &self,
        def: &ComponentDef<'a>,
        defs: &[&ComponentDef<'a>],
        seen: &mut Vec<Symbol>,
        out: &mut Surface,
    ) {
        if seen.contains(&def.name) {
            return;
        }
        seen.push(def.name);
        let Some(view) = &def.view else { return };
        self.walk_surface(&view.nodes, defs, seen, out);
    }

    fn walk_surface(
        &self,
        nodes: &[ViewNode<'a>],
        defs: &[&ComponentDef<'a>],
        seen: &mut Vec<Symbol>,
        out: &mut Surface,
    ) {
        for node in nodes {
            match node {
                ViewNode::Element {
                    tag,
                    attrs,
                    children,
                    ..
                } => {
                    out.tags.push(self.interner.resolve(*tag).to_string());
                    for attr in attrs {
                        match attr {
                            Attr::Plain { name, value } => {
                                let name = self.interner.resolve(*name);
                                if !matches!(name, "id" | "class") {
                                    continue;
                                }
                                // A COMPUTED id or class is markup this parse cannot see.
                                let Expr::Literal(Literal::Text(text)) = value else {
                                    out.unknown = true;
                                    continue;
                                };
                                let text = self.interner.resolve(*text);
                                match name {
                                    "id" => out.ids.push(text.to_string()),
                                    _ => out
                                        .classes
                                        .extend(text.split_whitespace().map(str::to_string)),
                                }
                            }
                            Attr::Class { name, .. } => {
                                out.classes.push(self.interner.resolve(*name).to_string())
                            }
                            Attr::Bind { .. } | Attr::Directive { .. } => {}
                        }
                    }
                    self.walk_surface(children, defs, seen, out);
                }
                ViewNode::Each { body, empty, .. } => {
                    self.walk_surface(body, defs, seen, out);
                    self.walk_surface(empty, defs, seen, out);
                }
                ViewNode::If {
                    arms, otherwise, ..
                } => {
                    for (_, body) in arms {
                        self.walk_surface(body, defs, seen, out);
                    }
                    if let Some(o) = otherwise {
                        self.walk_surface(o, defs, seen, out);
                    }
                }
                ViewNode::Inspect { arms, .. } => {
                    for arm in arms {
                        self.walk_surface(&arm.body, defs, seen, out);
                    }
                }
                ViewNode::Block { default, .. } => {
                    if let Some(d) = default {
                        self.walk_surface(d, defs, seen, out);
                    }
                }
                // A child component, and the layout a view extends, both draw real elements.
                ViewNode::Component { name, children, .. } => {
                    self.walk_surface(children, defs, seen, out);
                    match defs.iter().find(|d| d.name == *name) {
                        Some(child) => self.collect_surface(child, defs, seen, out),
                        None => out.unknown = true,
                    }
                }
                ViewNode::Extends { layout, .. } => match defs.iter().find(|d| d.name == *layout) {
                    Some(l) => self.collect_surface(l, defs, seen, out),
                    None => out.unknown = true,
                },
                // Markup this parse cannot see through.
                ViewNode::Raw { .. } | ViewNode::Children { .. } => out.unknown = true,
                ViewNode::Text { .. } | ViewNode::Handler { .. } | ViewNode::Let { .. } => {}
            }
        }
    }

    /// Reclassify every attribute whose name is a declared `## Directive` as one. The template
    /// parser cannot know: a directive may be declared BELOW the component that uses it, and a
    /// directive taking only an element is written as a plain `focused=""`. Doing it here means
    /// there is one rule — "a declared name is a directive" — with no ordering caveat.
    fn resolve_directive_attributes(&mut self, statements: &mut [Stmt<'a>]) {
        if self.directives.is_empty() {
            return;
        }
        fn walk(nodes: &mut [ViewNode<'_>], declared: &std::collections::HashSet<Symbol>) {
            for node in nodes {
                match node {
                    ViewNode::Element {
                        attrs, children, ..
                    } => {
                        for a in attrs.iter_mut() {
                            if let Attr::Plain { name, value } = a {
                                if declared.contains(name) {
                                    *a = Attr::Directive { name: *name, value };
                                }
                            }
                        }
                        walk(children, declared);
                    }
                    ViewNode::Each { body, empty, .. } => {
                        walk(body, declared);
                        walk(empty, declared);
                    }
                    ViewNode::If {
                        arms, otherwise, ..
                    } => {
                        for (_, body) in arms.iter_mut() {
                            walk(body, declared);
                        }
                        if let Some(o) = otherwise {
                            walk(o, declared);
                        }
                    }
                    ViewNode::Inspect { arms, .. } => {
                        for arm in arms.iter_mut() {
                            walk(&mut arm.body, declared);
                        }
                    }
                    ViewNode::Component { children, .. } => walk(children, declared),
                    ViewNode::Block { default, .. } => {
                        if let Some(d) = default {
                            walk(d, declared);
                        }
                    }
                    _ => {}
                }
            }
        }
        let declared = self.directives.clone();
        for stmt in statements {
            if let Stmt::ComponentDef(def) = stmt {
                if let Some(view) = def.view.as_mut() {
                    walk(&mut view.nodes, &declared);
                }
            }
        }
    }

    /// Check every recorded `Call <Store>'s <action>` against the stores the file declares.
    ///
    /// This runs once the whole program is parsed, so declaration order does not matter, and it
    /// is the one place the rule lives: a misnamed store or action is a program error found
    /// before anything runs, on every tier, in every tool.
    fn resolve_store_calls(&mut self) -> ParseResult<()> {
        let calls = std::mem::take(&mut self.store_calls);
        for (store, action, span) in calls {
            let Some(actions) = self.lgx_stores.get(&store) else {
                let known = self.known_store_names();
                return Err(ParseError {
                    kind: ParseErrorKind::Custom(format!(
                        "`{}` is not a store. Declare it as `## Store {}` with an `Actions:` \
                         section, or call one of {}.",
                        self.interner.resolve(store),
                        self.interner.resolve(store),
                        known
                    )),
                    span,
                });
            };
            if !actions.contains(&action) {
                let declared: Vec<&str> =
                    actions.iter().map(|a| self.interner.resolve(*a)).collect();
                let have = if declared.is_empty() {
                    "it declares none yet".to_string()
                } else {
                    format!("its actions are {}", declared.join(", "))
                };
                return Err(ParseError {
                    kind: ParseErrorKind::Custom(format!(
                        "`{}` is not an action of the store `{}` — {}. Declare it in that store's \
                         `Actions:` section as `To {} …:`.",
                        self.interner.resolve(action),
                        self.interner.resolve(store),
                        have,
                        self.interner.resolve(action)
                    )),
                    span,
                });
            }
        }
        Ok(())
    }

    /// Every name in `the journal shows A then B` or `Replay these actions: …` is an action some
    /// component in the file declares — checked once every component is known, so an action
    /// declared below the test that names it still counts.
    fn resolve_journal_actions(&self, statements: &[Stmt<'a>]) -> ParseResult<()> {
        if self.journal_action_sites.is_empty() {
            return Ok(());
        }
        let mut known: Vec<Symbol> = Vec::new();
        for s in statements {
            let Stmt::ComponentDef(def) = s else { continue };
            known.extend(def.actions.iter().map(|a| a.name));
        }
        for (name, span) in &self.journal_action_sites {
            if known.contains(name) {
                continue;
            }
            let written = self.interner.resolve(*name);
            let mut names: Vec<&str> = known.iter().map(|a| self.interner.resolve(*a)).collect();
            names.sort_unstable();
            names.dedup();
            let guess = crate::suggest::did_you_mean(written, names.iter().copied())
                .map(|g| format!(" — did you mean `{g}`?"))
                .unwrap_or_default();
            let have = if names.is_empty() {
                "this file declares no actions".to_string()
            } else {
                format!("its actions are {}", names.join(", "))
            };
            return Err(ParseError {
                kind: ParseErrorKind::Custom(format!(
                    "`{written}` is not an action of any component here{guess}{stop} {have}.",
                    stop = if guess.is_empty() { "." } else { "" },
                )),
                span: *span,
            });
        }
        Ok(())
    }

    /// A `State:` field left untyped and initialised by a plain call to a `## To` function this
    /// program declares WITH its own return type — `Let errors be noErrors().` — is typed from
    /// that return type, run once the whole file is parsed so a function declared below its
    /// caller still counts. Every tier already lowers an explicitly typed field; the only thing
    /// missing was the annotation an author who wrote it out by hand would have supplied.
    fn infer_state_field_types(&self, statements: &mut [Stmt<'a>]) {
        let mut returns: std::collections::HashMap<Symbol, &'a TypeExpr<'a>> =
            std::collections::HashMap::new();
        for s in statements.iter() {
            if let Stmt::FunctionDef {
                name,
                return_type: Some(rt),
                ..
            } = s
            {
                returns.insert(*name, *rt);
            }
        }
        if returns.is_empty() {
            return;
        }
        for s in statements.iter_mut() {
            let Stmt::ComponentDef(def) = s else { continue };
            for field in &mut def.state {
                if field.ty.is_some() {
                    continue;
                }
                if let Expr::Call { function, .. } = field.init {
                    if let Some(rt) = returns.get(function) {
                        field.ty = Some(*rt);
                    }
                }
            }
        }
    }

    /// Every `Use <Store>.` names a store this file declares — checked once the whole file is
    /// parsed, so a store used above its own `## Store` declaration is still fine, at the
    /// sentence rather than at the component that wrote it.
    fn resolve_uses(&self) -> ParseResult<()> {
        for (store, span) in &self.use_sites {
            if self.lgx_stores.contains_key(store) {
                continue;
            }
            let written = self.interner.resolve(*store);
            let mut known: Vec<&str> = self
                .lgx_stores
                .keys()
                .map(|s| self.interner.resolve(*s))
                .collect();
            known.sort_unstable();
            let guess = crate::suggest::did_you_mean(written, known.iter().copied())
                .map(|g| format!(" — did you mean `{g}`?"))
                .unwrap_or_default();
            return Err(ParseError {
                kind: ParseErrorKind::Custom(format!(
                    "`Use {written}.` names no store{guess}{stop} Declare it as `## Store {written}` \
                     with an `Actions:` section, or use one of {names}.",
                    stop = if guess.is_empty() { "." } else { "" },
                    names = self.known_store_names(),
                )),
                span: *span,
            });
        }
        Ok(())
    }

    /// A sentence that writes a place whose ROOT names a declared `## Store` — `Set Cart's count
    /// to 5.` reads as ordinary field mutation and compiles clean on every tier, and every tier
    /// then either writes a copy nothing reads or a global nothing repaints: a store's own
    /// `Actions:` are the only place a turn recomputes its derived fields, saves it, or notifies
    /// what shares it, so a write that bypasses them is silently lost, not merely irregular.
    /// `Call <Store>'s <action>.` is the one door in; this closes every side entrance.
    fn resolve_store_writes(&self) -> ParseResult<()> {
        for (target, span, verb) in &self.all_writes {
            if !self.lgx_stores.contains_key(target) {
                continue;
            }
            let store = self.interner.resolve(*target);
            let Some(actions) = self.lgx_stores.get(target) else {
                continue;
            };
            let have = if actions.is_empty() {
                format!("`{store}` declares no actions yet")
            } else {
                let names: Vec<&str> = actions.iter().map(|a| self.interner.resolve(*a)).collect();
                format!("its actions are {}", names.join(", "))
            };
            return Err(ParseError {
                kind: ParseErrorKind::Custom(format!(
                    "`{}` reaches into the store `{store}` directly, and a store changes only \
                     through its own `Actions:` — {have}. A field written any other way is not \
                     saved, not shared, and does not recompute what derives from it: the write \
                     compiles and is silently lost. Call `{store}`'s action instead: `Call \
                     {store}'s <action>.`",
                    verb.sentence(store),
                )),
                span: *span,
            });
        }
        Ok(())
    }

    /// The declared store names, for the "you meant one of these" half of the message.
    fn known_store_names(&self) -> String {
        let mut names: Vec<&str> = self
            .lgx_stores
            .keys()
            .map(|s| self.interner.resolve(*s))
            .collect();
        names.sort_unstable();
        match names.len() {
            0 => "a store this file declares (there are none yet)".to_string(),
            _ => names.join(", "),
        }
    }
}

/// What a reachable view tree is known to render — the ids, classes and tags a `When "<sel>"`
/// sentence can name. `unknown` says the tree contains markup this parse cannot see through,
/// in which case nothing is checked: a check that cannot see everything must not accuse anyone.
#[derive(Default)]
struct Surface {
    ids: Vec<String>,
    classes: Vec<String>,
    tags: Vec<String>,
    unknown: bool,
}

impl Surface {
    /// The first requirement of `selector` nothing here satisfies, phrased for the message.
    fn first_unmatched(&self, selector: &[crate::lgx::selector::Compound]) -> Option<String> {
        for compound in selector {
            if let Some(id) = &compound.id {
                if !self.ids.iter().any(|k| k == id) {
                    return Some(format!("the id `{id}`"));
                }
            }
            for class in &compound.classes {
                if !self.classes.iter().any(|k| k == class) {
                    return Some(format!("the class `{class}`"));
                }
            }
            if let Some(tag) = &compound.tag {
                if !self.tags.iter().any(|k| k == tag) {
                    return Some(format!("a `<{tag}>`"));
                }
            }
        }
        None
    }
}

/// The type a declaration names, canonically enough to compare two of them.
///
/// Returns an empty string for a shape this comparison cannot render — a tuple, a function type —
/// so the caller SKIPS rather than compares two blanks and calls them equal.
fn wire_type_name(ty: &TypeExpr<'_>, interner: &crate::intern::Interner) -> String {
    match ty {
        TypeExpr::Primitive(s) | TypeExpr::Named(s) => interner.resolve(*s).to_string(),
        TypeExpr::Generic { base, params } => {
            let inner: Vec<String> = params.iter().map(|p| wire_type_name(p, interner)).collect();
            if inner.iter().any(|i| i.is_empty()) {
                return String::new();
            }
            format!("{} of {}", interner.resolve(*base), inner.join(" and "))
        }
        _ => String::new(),
    }
}

/// The names an `Actions:` sentence binds for its own body — the values its trigger carries.
fn trigger_bindings(trigger: &Trigger<'_>) -> Vec<Symbol> {
    match trigger {
        Trigger::Message { param, .. } | Trigger::FileDropped { param, .. } => vec![param.0],
        Trigger::ActionFails { param } => vec![*param],
        Trigger::FetchFails { url, error } => vec![*url, *error],
        Trigger::PageLoads
        | Trigger::Appears
        | Trigger::Disappears
        | Trigger::RouteChanges
        | Trigger::Online
        | Trigger::Offline
        | Trigger::Selector { .. }
        | Trigger::Key { .. }
        | Trigger::FieldChanges { .. }
        | Trigger::Every { .. }
        | Trigger::After { .. }
        | Trigger::WindowResizes
        | Trigger::ScrollsToBottom { .. } => Vec::new(),
    }
}
