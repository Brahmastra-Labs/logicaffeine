//! A `-> View` template on the lowered tiers, as DATA.
//!
//! The reference tier's View value is a detached fragment, spent by its first placement. Lowered
//! code cannot hold that: a page repaints, so a view kept in a prop or a `State:` list is placed
//! again and again; and a closure stored in a map is an indirect call the wasm backend refuses. So
//! a View value is the template CALL itself, defunctionalized — one variant of the program's
//! `__View` enum per template, carrying that template's arguments — and placing one draws the
//! template's nodes afresh, as often as it is placed, in the scope of whoever places it.
//!
//! `## To badge (label: Text) -> View:` becomes:
//! - `badge(label) -> __View`, answering `__View_badge` with its label — the value every call,
//!   prop, list and binding holds;
//! - `__view_badge(depth, scope, label)`, which draws the template's nodes onto the tape, each
//!   element carrying the placing component's scope class `scope`, `depth` templates deep;
//! - one `__lgx_place_view(view, scope, depth)` over every template, for a view whose template
//!   is decided at run time.
//!
//! A program that renders on a server (a `## Route`, an `html of`) makes its views data the same
//! way, so a server draws them with the same functions and serializes what they drew.

use super::render::LowerTrail;
use super::*;
use crate::analysis::registry::TypeDef;
use crate::ast::component::{ComponentDef, ComponentKind};
use crate::ast::stmt::{BinaryOpKind, Expr, MatchArm, Stmt, StringPart, TypeExpr};
use crate::ast::view::{Attr, ViewDef, ViewNode};
use crate::token::Span;
use logicaffeine_base::{Interner, Symbol};

/// One `-> View` template, as the author wrote it.
#[derive(Clone)]
pub(crate) struct ViewTemplate<'a> {
    pub name: Symbol,
    pub params: Vec<(Symbol, &'a TypeExpr<'a>)>,
    pub view: ViewDef<'a>,
}

/// What the lowering knows about a page program's views while it walks the components.
#[derive(Default)]
pub(crate) struct ViewLowering<'a> {
    /// Every template, in declaration order — the order of `__View`'s variants.
    pub templates: Vec<ViewTemplate<'a>>,
    /// Every function whose value is a View: the templates, and any `-> View` function that
    /// computes one.
    pub functions: Vec<Symbol>,
    /// The names the walk in progress knows to hold a View: a loop variable over a list of views,
    /// a `{let}` bound to one.
    pub locals: Vec<Symbol>,
    /// While a template's nodes are drawn, the parameter carrying the placing component's scope
    /// class, which every element the template draws takes.
    pub scope_param: Option<Symbol>,
    /// While a template's nodes are drawn, the parameter holding how deep it is — a view it places
    /// is one deeper.
    pub depth_param: Option<Symbol>,
}

/// Whether `ty` is the `View` type.
pub(crate) fn is_view_type(ty: &TypeExpr<'_>, interner: &Interner) -> bool {
    matches!(ty, TypeExpr::Primitive(n) | TypeExpr::Named(n) if interner.resolve(*n) == "View")
}

/// Whether `ty` is a list of views (`Seq of View`, `List of View`).
pub(crate) fn is_view_list_type(ty: &TypeExpr<'_>, interner: &Interner) -> bool {
    matches!(ty, TypeExpr::Generic { base, params }
        if matches!(interner.resolve(*base), "Seq" | "List")
            && params.first().is_some_and(|p| is_view_type(p, interner)))
}

/// The one expression a hole holds, when the text is nothing but that hole.
pub(crate) fn sole_hole<'e, 'a>(text: &'e Expr<'a>) -> &'e Expr<'a> {
    match text {
        Expr::InterpolatedString(parts) => match parts.as_slice() {
            [StringPart::Expr {
                value,
                format_spec: None,
                debug: false,
            }] => value,
            _ => text,
        },
        _ => text,
    }
}

/// Every hole in `nodes`, however deep: what each `{…}` of text holds, one expression per hole.
pub(crate) fn view_holes<'a>(nodes: &[ViewNode<'a>], visit: &mut dyn FnMut(&'a Expr<'a>)) {
    for node in nodes {
        if let ViewNode::Text { text, .. } = node {
            match text {
                Expr::InterpolatedString(parts) => {
                    for part in parts.iter() {
                        if let StringPart::Expr { value, .. } = part {
                            visit(value);
                        }
                    }
                }
                other => visit(other),
            }
        }
        for sub in view_subtrees(node) {
            view_holes(sub, visit);
        }
    }
}

/// What in a template's markup can only be answered by a component — which a template drawn
/// wherever it is placed does not have — if anything, and where it is written.
fn template_refusal(nodes: &[ViewNode<'_>]) -> Option<(&'static str, Span)> {
    nodes.iter().find_map(|node| match node {
        ViewNode::Element {
            attrs,
            children,
            span,
            ..
        } => attrs
            .iter()
            .find_map(|a| match a {
                Attr::Bind { .. } => Some(("a `bind`", *span)),
                Attr::Directive { .. } => Some(("a directive", *span)),
                Attr::Plain { .. } | Attr::Class { .. } => None,
            })
            .or_else(|| template_refusal(children)),
        ViewNode::Handler { span, .. } => Some(("an event handler", *span)),
        ViewNode::Component { span, .. } => Some(("a component", *span)),
        ViewNode::Children { span } => Some(("`{children}`", *span)),
        ViewNode::Block { span, .. } | ViewNode::Extends { span, .. } => {
            Some(("a layout block", *span))
        }
        ViewNode::Each { body, empty, .. } => {
            template_refusal(body).or_else(|| template_refusal(empty))
        }
        ViewNode::If {
            arms, otherwise, ..
        } => arms
            .iter()
            .find_map(|(_, body)| template_refusal(body))
            .or_else(|| otherwise.as_deref().and_then(template_refusal)),
        ViewNode::Inspect { arms, .. } => arms.iter().find_map(|arm| template_refusal(&arm.body)),
        ViewNode::Text { .. } | ViewNode::Raw { .. } | ViewNode::Let { .. } => None,
    })
}

impl<'a, 'ctx, 'int> super::super::Parser<'a, 'ctx, 'int> {
    /// Turn a program's `-> View` templates into DATA (see the module docs): register the `__View`
    /// enum, make each template answer its variant, and give every View-typed parameter and result
    /// the enum's type — on every tier, whoever draws the view: a page places it, a server serializes
    /// it, `Show` reads its markup.
    pub(crate) fn lower_view_functions(
        &mut self,
        statements: &mut [Stmt<'a>],
    ) -> crate::parser::ParseResult<()> {
        let templates: Vec<ViewTemplate<'a>> = statements
            .iter()
            .filter_map(|s| {
                view_template(s).map(|(name, params, view)| ViewTemplate {
                    name,
                    params: params.to_vec(),
                    view: view.clone(),
                })
            })
            .collect();
        let functions: Vec<Symbol> = statements
            .iter()
            .filter_map(|s| match s {
                Stmt::FunctionDef {
                    name,
                    return_type: Some(ty),
                    ..
                } if is_view_type(ty, self.interner) => Some(*name),
                _ => None,
            })
            .collect();
        if functions.is_empty() {
            return Ok(());
        }
        for template in &templates {
            if let Some((what, span)) = template_refusal(&template.view.nodes) {
                let name = self.interner.resolve(template.name).to_string();
                return Err(crate::error::ParseError {
                    kind: crate::error::ParseErrorKind::Custom(format!(
                        "`## To {name}`'s template holds {what}. A `-> View` template is drawn by \
                         whichever component places it, so it has no state or actions of its own \
                         to answer one with. Put {what} in the component that places `{name}`, or \
                         make `{name}` a `## Component` that owns it."
                    )),
                    span,
                });
            }
        }
        for template in &templates {
            if let Some(why) = self.view_variant_disagreement(template) {
                return Err(crate::error::ParseError {
                    kind: crate::error::ParseErrorKind::Custom(why),
                    span: template.view.span,
                });
            }
        }
        let view_ty = self.view_type_expr();
        for stmt in statements.iter_mut() {
            let Stmt::FunctionDef {
                name,
                params,
                body,
                return_type,
                ..
            } = stmt
            else {
                continue;
            };
            for (_, ty) in params.iter_mut() {
                if is_view_type(ty, self.interner) {
                    *ty = view_ty;
                }
            }
            if return_type.is_some_and(|t| is_view_type(t, self.interner)) {
                *return_type = Some(view_ty);
            }
            if let Some(template) = templates.iter().find(|t| t.name == *name) {
                let fields = template
                    .params
                    .iter()
                    .map(|(p, _)| (*p, self.te_id(*p)))
                    .collect();
                let value = self.ctx.alloc_imperative_expr(Expr::NewVariant {
                    enum_name: self.interner.intern(view_type_name()),
                    variant: self
                        .interner
                        .intern(&view_variant_name(self.interner.resolve(template.name))),
                    fields,
                });
                *body = self.alloc_block(vec![Stmt::Return { value: Some(value) }]);
            }
        }
        self.lgx_views.templates = templates;
        self.lgx_views.functions = functions;
        let defs: Vec<ComponentDef<'a>> = statements
            .iter()
            .filter_map(|s| match s {
                Stmt::ComponentDef(d) => Some((**d).clone()),
                _ => None,
            })
            .collect();
        self.lower_view_draws(&defs);
        Ok(())
    }

    /// The template the discovery pass read as `__View`'s variant for `template`, field for
    /// field — or why not. Discovery registers the enum from the file's tokens (every engine is
    /// handed that registry); this parse is what the variant's construction and drawing are built
    /// from, so the two readings of one header must agree, and a disagreement is refused rather
    /// than guessed across.
    fn view_variant_disagreement(&self, template: &ViewTemplate<'a>) -> Option<String> {
        let enum_name = self.interner.lookup(view_type_name())?;
        let variant = self
            .interner
            .lookup(&view_variant_name(self.interner.resolve(template.name)));
        let declared = self
            .type_registry
            .as_ref()
            .and_then(|r| match r.get(enum_name) {
                Some(TypeDef::Enum { variants, .. }) => {
                    variants.iter().find(|v| Some(v.name) == variant)
                }
                _ => None,
            });
        let name = self.interner.resolve(template.name);
        let Some(declared) = declared else {
            return Some(format!(
                "`## To {name}`'s header could not be read as a template before the program was \
                 parsed, so no tier knows the shape of the view it returns. Write its header as \
                 `## To {name} (<name>: <Type>) … -> View:` with the markup on the lines below."
            ));
        };
        let parsed: Vec<Symbol> = template.params.iter().map(|(p, _)| *p).collect();
        let discovered: Vec<Symbol> = declared.fields.iter().map(|f| f.name).collect();
        (parsed != discovered).then(|| {
            format!(
                "`## To {name}`'s parameters were read two ways — as {} before parsing and as {} \
                 after — so the view it returns has no single shape. Name each parameter once, \
                 as `(<name>: <Type>)`.",
                discovered.len(),
                parsed.len()
            )
        })
    }

    /// Whether a function's value is a View: its declared result, or — once the template has been
    /// made data — the lowering's own record of it. Every reader asking "is this a view function"
    /// asks here, before and after the rewrite alike.
    pub(crate) fn is_view_function(
        &self,
        name: Symbol,
        return_type: Option<&'a TypeExpr<'a>>,
    ) -> bool {
        self.returns_a_view(return_type) || self.lgx_views.functions.contains(&name)
    }

    /// `stmt` as a `-> View` template (`view_template`), or — once the template has been made data —
    /// the template it was.
    pub(crate) fn view_template_of<'s>(
        &'s self,
        stmt: &'s Stmt<'a>,
    ) -> Option<(Symbol, &'s [(Symbol, &'a TypeExpr<'a>)], &'s ViewDef<'a>)> {
        if let Some(found) = view_template(stmt) {
            return Some(found);
        }
        let Stmt::FunctionDef { name, .. } = stmt else {
            return None;
        };
        self.lgx_views
            .templates
            .iter()
            .find(|t| t.name == *name)
            .map(|t| (t.name, t.params.as_slice(), &t.view))
    }

    /// The `Int` type expression.
    pub(crate) fn int_type(&mut self) -> &'a TypeExpr<'a> {
        let name = self.interner.intern("Int");
        self.ctx.alloc_type_expr(TypeExpr::Primitive(name))
    }

    /// The `__View` type expression.
    pub(crate) fn view_type_expr(&mut self) -> &'a TypeExpr<'a> {
        let name = self.interner.intern(view_type_name());
        self.ctx.alloc_type_expr(TypeExpr::Named(name))
    }

    /// `ty`, with `View` read as the program's `__View` enum.
    pub(crate) fn lowered_type(&mut self, ty: &'a TypeExpr<'a>) -> &'a TypeExpr<'a> {
        if !self.lgx_views.functions.is_empty() && is_view_type(ty, self.interner) {
            return self.view_type_expr();
        }
        ty
    }

    /// The empty view: what a View prop holds before anything is handed to it.
    pub(crate) fn empty_view(&mut self) -> &'a Expr<'a> {
        self.ctx.alloc_imperative_expr(Expr::NewVariant {
            enum_name: self.interner.intern(view_type_name()),
            variant: self.interner.intern(empty_view_variant_name()),
            fields: Vec::new(),
        })
    }

    /// Every template's drawing function and the one placement dispatcher, for the program's
    /// deferred functions. Nothing when the program has no template.
    fn lower_view_draws(&mut self, all: &[ComponentDef<'a>]) {
        if self.lgx_views.functions.is_empty() {
            return;
        }
        let templates = self.lgx_views.templates.clone();
        for template in &templates {
            let draw = self.view_draw_fn(template, all);
            self.deferred_functions.push(draw);
        }
        let place = self.place_view_fn(&templates);
        self.deferred_functions.push(place);
    }

    /// `__view_<f>(__lgx_depth, __lgx_scope, params…)`: the template's nodes drawn onto the tape,
    /// each element carrying the scope class `__lgx_scope` names. A template that can place itself
    /// fails past `RENDER_DEPTH_LIMIT` deep, in the render-depth words, before the stack does.
    fn view_draw_fn(&mut self, template: &ViewTemplate<'a>, all: &[ComponentDef<'a>]) -> Stmt<'a> {
        let scope = self.interner.intern(view_scope_param());
        let depth = self.interner.intern(draw_depth_param());
        let owner = ComponentDef {
            kind: ComponentKind::Component,
            name: template.name,
            props: template.params.clone(),
            uses: Vec::new(),
            state: Vec::new(),
            events: Vec::new(),
            actions: Vec::new(),
            view: Some(template.view.clone()),
            head: None,
            style: None,
            span: template.view.span,
        };
        let mut body: Vec<Stmt<'a>> = Vec::new();
        if self.template_on_a_view_cycle(template.name) {
            body.push(self.refuse_past_render_depth(depth, template.name));
        }
        let mut next = 0usize;
        let outer_scope = self.lgx_views.scope_param.replace(scope);
        let outer_depth = self.lgx_views.depth_param.replace(depth);
        let outer_locals = std::mem::take(&mut self.lgx_views.locals);
        self.lower_lgx_nodes(
            &template.view.nodes,
            &mut next,
            &mut body,
            all,
            &owner,
            &LowerTrail::opaque(),
        );
        self.lgx_views.locals = outer_locals;
        self.lgx_views.depth_param = outer_depth;
        self.lgx_views.scope_param = outer_scope;
        let text_ty = self.text_type();
        let int_ty = self.int_type();
        let mut params = vec![(depth, int_ty), (scope, text_ty)];
        for (p, ty) in &template.params {
            let ty = self.lowered_type(ty);
            params.push((*p, ty));
        }
        Stmt::FunctionDef {
            name: self
                .interner
                .intern(&view_draw_fn_name(self.interner.resolve(template.name))),
            generics: Vec::new(),
            params,
            body: self.alloc_block(body),
            return_type: None,
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        }
    }

    /// `__lgx_place_view(view, scope, depth)`: draw whichever template `view` is a call of.
    fn place_view_fn(&mut self, templates: &[ViewTemplate<'a>]) -> Stmt<'a> {
        let view = self.interner.intern(view_value_param());
        let scope = self.interner.intern(view_scope_param());
        let depth = self.interner.intern(draw_depth_param());
        let enum_name = self.interner.intern(view_type_name());
        let mut arms: Vec<MatchArm<'a>> = templates
            .iter()
            .map(|template| {
                let f = self.interner.resolve(template.name).to_string();
                let mut args = vec![self.te_id(depth), self.te_id(scope)];
                args.extend(template.params.iter().map(|(p, _)| self.te_id(*p)));
                let draw = Stmt::Call {
                    function: self.interner.intern(&view_draw_fn_name(&f)),
                    args,
                };
                MatchArm {
                    enum_name: Some(enum_name),
                    variant: Some(self.interner.intern(&view_variant_name(&f))),
                    bindings: template.params.iter().map(|(p, _)| (*p, *p)).collect(),
                    body: self.alloc_block(vec![draw]),
                }
            })
            .collect();
        arms.push(MatchArm {
            enum_name: Some(enum_name),
            variant: Some(self.interner.intern(empty_view_variant_name())),
            bindings: Vec::new(),
            body: self.alloc_block(Vec::new()),
        });
        let view_ty = self.view_type_expr();
        let text_ty = self.text_type();
        let int_ty = self.int_type();
        Stmt::FunctionDef {
            name: self.interner.intern(place_view_fn_name()),
            generics: Vec::new(),
            params: vec![(view, view_ty), (scope, text_ty), (depth, int_ty)],
            body: self.alloc_block(vec![Stmt::Inspect {
                target: self.te_id(view),
                arms,
                has_otherwise: false,
            }]),
            return_type: None,
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        }
    }

    /// The statement drawing the View a hole holds where the hole is, when the hole holds one: a
    /// call of a template draws it directly; any other View value goes through the dispatcher. The
    /// scope is the placing component's own class, or the one a template being drawn was handed.
    pub(crate) fn place_view_hole(
        &mut self,
        text: &'a Expr<'a>,
        owner: &ComponentDef<'a>,
    ) -> Option<Stmt<'a>> {
        let hole = sole_hole(text);
        if !self.view_typed(hole, owner) {
            return None;
        }
        let scope = match self.lgx_views.scope_param {
            Some(param) => self.te_id(param),
            None => {
                let class = match owner.style.is_some() {
                    true => scope_class(self.interner.resolve(owner.name)),
                    false => String::new(),
                };
                let class = self.interner.intern(&class);
                self.te_text(class)
            }
        };
        // A view a template places is one deeper than the template; one a component places is the
        // first.
        let depth = match self.lgx_views.depth_param {
            Some(d) => self.te_bin(BinaryOpKind::Add, self.te_id(d), self.te_int(1)),
            None => self.te_int(1),
        };
        if let Expr::Call { function, args } = hole {
            if self.lgx_views.templates.iter().any(|t| t.name == *function) {
                let draw = self
                    .interner
                    .intern(&view_draw_fn_name(self.interner.resolve(*function)));
                let mut call_args = vec![depth, scope];
                call_args.extend(args.iter().copied());
                return Some(Stmt::Call {
                    function: draw,
                    args: call_args,
                });
            }
        }
        let place = self.interner.intern(place_view_fn_name());
        Some(Stmt::Call {
            function: place,
            args: vec![hole, scope, depth],
        })
    }

    /// Whether template `name` can place itself — through a hole of its own, or of a template it
    /// places — so its drawing counts how deep it is. A hole holding a View value the lowering
    /// cannot name a template for could be any of them, so it is an edge to every template.
    pub(crate) fn template_on_a_view_cycle(&self, name: Symbol) -> bool {
        let edges = |t: &ViewTemplate<'a>| -> Vec<Symbol> {
            let mut out = Vec::new();
            view_holes(&t.view.nodes, &mut |hole| match hole {
                Expr::Call { function, .. }
                    if self.lgx_views.templates.iter().any(|t| t.name == *function) =>
                {
                    out.push(*function)
                }
                other if self.hole_holds_a_view(other, &t.params) => {
                    out.extend(self.lgx_views.templates.iter().map(|t| t.name))
                }
                _ => {}
            });
            out
        };
        let mut seen: Vec<Symbol> = Vec::new();
        let mut frontier: Vec<Symbol> = self
            .lgx_views
            .templates
            .iter()
            .find(|t| t.name == name)
            .map(edges)
            .unwrap_or_default();
        while let Some(next) = frontier.pop() {
            if next == name {
                return true;
            }
            if seen.contains(&next) {
                continue;
            }
            seen.push(next);
            if let Some(t) = self.lgx_views.templates.iter().find(|t| t.name == next) {
                frontier.extend(edges(t));
            }
        }
        false
    }

    /// Whether a template's hole holds a View it names by value — a View parameter, or a call of a
    /// function whose value is a View but that is not itself a template.
    fn hole_holds_a_view(&self, hole: &Expr<'a>, params: &[(Symbol, &'a TypeExpr<'a>)]) -> bool {
        match hole {
            Expr::Identifier(name) => params
                .iter()
                .any(|(p, ty)| p == name && is_view_type(ty, self.interner)),
            Expr::Call { function, .. } => self.lgx_views.functions.contains(function),
            _ => false,
        }
    }

    /// `If depth > RENDER_DEPTH_LIMIT: Fail <render_depth_message(name)>.` — a drawing nested past
    /// the limit fails in the words every tier uses, before the stack gives out.
    pub(crate) fn refuse_past_render_depth(&mut self, depth: Symbol, name: Symbol) -> Stmt<'a> {
        let limit = crate::ast::component::RENDER_DEPTH_LIMIT as i64;
        let past = self.te_bin(BinaryOpKind::Gt, self.te_id(depth), self.te_int(limit));
        let words = crate::ast::component::render_depth_message(self.interner.resolve(name));
        let message = self.interner.intern(&words);
        let refuse = self.alloc_block(vec![Stmt::Fail {
            reason: self.te_text(message),
        }]);
        Stmt::If {
            cond: past,
            then_block: refuse,
            else_block: None,
        }
    }

    /// Whether `expr` is a View, by what the lowering can see of its type: a call of a function
    /// whose value is a View, a name bound to one (a View prop or parameter, a state field holding
    /// one, a loop variable over a list of them, a `{let}`), or an item of a list of views.
    pub(crate) fn view_typed(&self, expr: &Expr<'a>, owner: &ComponentDef<'a>) -> bool {
        if self.lgx_views.functions.is_empty() {
            return false;
        }
        match expr {
            Expr::Call { function, .. } => self.lgx_views.functions.contains(function),
            Expr::Identifier(name) => {
                self.lgx_views.locals.contains(name)
                    || owner
                        .props
                        .iter()
                        .any(|(p, ty)| p == name && is_view_type(ty, self.interner))
                    || owner.state.iter().any(|f| {
                        f.name == *name
                            && match f.ty {
                                Some(ty) => is_view_type(ty, self.interner),
                                None => self.view_typed(f.init, owner),
                            }
                    })
            }
            Expr::Index { collection, .. } => self.view_list_typed(collection, owner),
            _ => false,
        }
    }

    /// Whether `expr` is a list of views.
    pub(crate) fn view_list_typed(&self, expr: &Expr<'a>, owner: &ComponentDef<'a>) -> bool {
        if self.lgx_views.functions.is_empty() {
            return false;
        }
        match expr {
            Expr::List(items) => items.first().is_some_and(|e| self.view_typed(e, owner)),
            Expr::Identifier(name) => {
                owner
                    .props
                    .iter()
                    .any(|(p, ty)| p == name && is_view_list_type(ty, self.interner))
                    || owner.state.iter().any(|f| {
                        f.name == *name
                            && match f.ty {
                                Some(ty) => is_view_list_type(ty, self.interner),
                                None => self.view_list_typed(f.init, owner),
                            }
                    })
            }
            _ => false,
        }
    }
}
