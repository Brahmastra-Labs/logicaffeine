use super::*;
use crate::ast::component::{ComponentDef, Trigger};
use crate::ast::stmt::{BinaryOpKind, Expr, Stmt, TypeExpr};

impl<'a, 'ctx, 'int> super::super::Parser<'a, 'ctx, 'int> {
    /// `__mount_<Name>(parent)` — what a `Mount` actually calls.
    ///
    /// It records the mount point, runs `When the page loads:` and `When this component
    /// appears:` ONCE, and then paints. The load handlers cannot live in the render function,
    /// because that runs again on every repaint; and they cannot end with a repaint of their
    /// own, because the paint that follows them is the first one.
    pub(crate) fn lower_component_mount(&mut self, def: &ComponentDef<'a>) -> Option<Stmt<'a>> {
        // ★ A COMPONENT WITH PROPS IS NEVER MOUNTED, so it gets no mount function.
        //
        // `Mount X` carries no props (`UiStmt::Mount { component, selector }`) — a props-carrying
        // component is PLACED by a parent, which is the only thing that knows what to pass it. So a
        // mount function for one could only ever call its render with the props missing, which is
        // exactly the call that cannot type-check: `__render_Row` takes (parent, key, label) and the
        // mount had (parent, key).
        //
        // It was wrong before the instance key existed too — (parent) against (parent, label) — and
        // stayed invisible because nothing type-checked a component's lowering until `## Component`
        // became a declaration on AOT-Rust. A latent arity error, surfaced rather than introduced.
        if !def.props.is_empty() {
            return None;
        }
        let component = self.interner.resolve(def.name).to_string();
        let parent = self.interner.intern("__lgx_parent");
        let mut body: Vec<Stmt<'a>> = Vec::new();
        // The key is bound whether or not this component holds state: the paint below names it,
        // and a stateless component is still an instance — the root one.
        body.push(self.key_local());
        // (Where this instance paints is recorded just before the paint below, per instance —
        // the scalar write that used to live here assigned a node handle to what is now a map.)
        for handler in &def.events {
            // `Appears` is NOT here any more — it is per-instance and fires on an instance's
            // first render (see `lower_component_render`). Leaving a copy here would run the root's
            // twice; `When the page loads:` stays because it is about the PAGE, which is mounted
            // exactly once.
            if !matches!(handler.trigger, Trigger::PageLoads) {
                continue;
            }
            // The same read-in / write-back a turn uses, minus the repaint: what these set is
            // what the first paint shows. Through the SAME two helpers `turn_body` uses — this
            // block used to be a second hand-written copy of that shape, which is exactly how it
            // came to be missed when state moved from a scalar binding to a per-instance map.
            for field in def.state.iter().filter(|f| !f.derived) {
                let value = self.state_read(&component, field.name);
                body.push(self.state_field_let(field, value, true));
            }
            // `Load "k" into f` needs f's type — see `lgx_state_types`.
            self.note_state_types(def);
            self.lgx_pending_events.clear();
            let inner = self.rewrite_store_calls(handler.body, &component)?;
            let inner = self.protect_early_return(inner.to_vec());
            body.extend(inner.iter().cloned());
            for field in def.state.iter().filter(|f| !f.derived) {
                let value = self.te_id(field.name);
                let write = self.state_write(&component, field.name, value);
                body.push(write);
            }
            // A store the handler moved is taken here, before the first paint: the instances that
            // paint draws did not exist when it moved, so they first see it moved, not moving.
            body.extend(self.wake_stores_call());
            // The element events the handler queued, delivered once its state has committed —
            // the flush a turn's epilogue performs.
            for (selector, event, value) in std::mem::take(&mut self.lgx_pending_events) {
                let dispatch = self.interner.intern(dispatch_fn_name());
                body.push(Stmt::Call {
                    function: dispatch,
                    args: vec![selector, event, value],
                });
            }
        }
        // ★ `Head:` RENDERS ONCE, RIGHT HERE — BEFORE the first paint, from the state the load
        // handlers above left (a hole like `<title>{count} left</title>` reads it; the paint
        // changes no state, so it reads what the painted page shows), and never again: a repaint
        // never touches it, because the head is part of the DOCUMENT a browser already has by the
        // time the module loads, and `Set the title to …` is the sentence for a title that must
        // move at run time. Before the paint so a body that cannot draw — a trap in the view —
        // still leaves the page its title, description and social card.
        // ★ A LAYOUT'S `Head:` COMES FIRST, the page's after it, in ONE head string: the host
        // applies a head tag by tag, a later tag replacing an earlier one of the same identity
        // (`dom::HeadTag::identity`), so the page's own tags win and the layout's stand wherever
        // the page says nothing.
        let layout_head = def
            .view
            .as_ref()
            .and_then(|v| super::render::extended_layout(&v.nodes))
            .and_then(|l| self.lgx_all_components.iter().find(|d| d.name == l))
            .and_then(|l| l.head.clone());
        let heads: Vec<crate::ast::view::ViewDef<'a>> =
            layout_head.into_iter().chain(def.head.clone()).collect();
        if !heads.is_empty() {
            // Only a STATEFUL component's fields live in a global `__S_<C>_<field>` map — one
            // whose fields no action ever writes reads its own initializers directly, the same
            // split `lower_component_render` makes for the same reason (`is_stateful` there).
            let is_stateful = stateful(def);
            for field in def.state.iter().filter(|f| !f.derived) {
                let value = if is_stateful {
                    self.state_read(&component, field.name)
                } else {
                    field.init
                };
                body.push(self.state_field_let(field, value, true));
            }
            for field in def.state.iter().filter(|f| f.derived) {
                body.push(self.state_field_let(field, field.init, true));
            }
            // The head is drawn as a server draws a page — by the serving walk into a root no host
            // holds, serialized — because it is markup handed to the host whole, not a subtree
            // the page keeps and patches.
            let out = self.interner.intern("__lgx_head_html");
            let all = self.lgx_all_components.clone();
            let mut draw = Vec::new();
            for head in &heads {
                let owner = self.template_owner(def.name, Vec::new(), head);
                draw.extend(self.serving_draw(&head.nodes, &owner, &all));
            }
            self.serve_markup(draw, out, &mut body);
            let set_head_html = self.interner.intern("setHeadHtml");
            body.push(Stmt::Call {
                function: set_head_html,
                args: vec![self.te_id(out)],
            });
        }
        // ★ THE MOUNT POINT BECOMES A RETAINED ROOT the first paint commits into, and what it
        // already holds is either that paint or not the page's. A pre-rendered document marks its
        // markup as the page's own first paint (`PRERENDERED_ATTR`): the first commit adopts it,
        // node for node, so what a visitor focused or typed before the program ran stays theirs.
        // Anything else is emptied first — a paint that only appended would depend on what was
        // already there. Every later paint is a diff against what the first one drew.
        let root = self.interner.intern("__lgx_root");
        let made = self.vd_expr("vdNewRoot", vec![self.te_id(parent)]);
        body.push(Stmt::Let {
            var: root,
            ty: None,
            value: made,
            mutable: false,
        });
        let get_attribute = self.interner.intern("getAttribute");
        let mark = self.interner.intern(PRERENDERED_ATTR);
        let marked = self.ctx.alloc_imperative_expr(Expr::Call {
            function: get_attribute,
            args: vec![self.te_id(parent), self.te_text(mark)],
        });
        let blank = self.interner.intern("");
        let prerendered = self.te_bin(BinaryOpKind::NotEq, marked, self.te_text(blank));
        let clear = self.interner.intern("clearChildren");
        let adopt = vec![self.vd_call("vdHydrate", vec![self.te_id(root)])];
        let empty = vec![Stmt::Call {
            function: clear,
            args: vec![self.te_id(parent)],
        }];
        body.push(Stmt::If {
            cond: prerendered,
            then_block: self.alloc_block(adopt),
            else_block: Some(self.alloc_block(empty)),
        });
        body.extend(self.repaint_def(def, true));
        let int_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Int")));
        let name = self.interner.intern(&mount_fn_name(&component));
        Some(Stmt::FunctionDef {
            name,
            generics: Vec::new(),
            params: vec![(parent, int_ty)],
            body: self.alloc_block(body),
            return_type: None,
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        })
    }
}
