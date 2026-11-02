use super::*;
use crate::ast::component::ComponentDef;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use crate::ast::view::{bound_input_kind, ViewNode};
use logicaffeine_base::Symbol;

/// The listener-table entry of a listener waiting for one key: its event and the key, lowercased
/// as the dispatcher compares it, in one spelling no event name can carry.
pub(crate) fn keyed_listener_event(event: &str, key: &str) -> String {
    format!("{event}\u{1}{}", key.to_lowercase())
}

/// A listener-table entry as its event and the key it waits for, when it waits for one.
pub(crate) fn split_keyed_listener_event(entry: &str) -> Option<(&str, &str)> {
    entry.split_once('\u{1}')
}

/// The view node a dotted site path names, following ordinary element children only.
///
/// Control-flow bodies are not followed: their sites are never patchable, so a path leading into
/// one never reaches here.
pub(crate) fn view_node_at<'v, 'a>(
    nodes: &'v [ViewNode<'a>],
    path: &str,
) -> Option<&'v ViewNode<'a>> {
    let mut here: &'v [ViewNode<'a>] = nodes;
    let mut found: Option<&'v ViewNode<'a>> = None;
    for segment in path.split('.') {
        let index: usize = segment.parse().ok()?;
        let node = here.get(index)?;
        found = Some(node);
        here = match node {
            ViewNode::Element { children, .. } => children,
            _ => &[],
        };
    }
    found
}

impl<'a, 'ctx, 'int> super::super::Parser<'a, 'ctx, 'int> {
    /// The statements that update every site a turn writing `fields` invalidates, or `None` when
    /// this turn must repaint.
    ///
    /// An empty plan is still `Some`: a turn whose fields no site reads (state the page holds but
    /// does not show) needs no DOM work at all, and painting for it is exactly the waste Stage B
    /// exists to remove.
    pub(crate) fn patch_statements(
        &mut self,
        def: &ComponentDef<'a>,
        fields: &[Symbol],
    ) -> Option<Vec<Stmt<'a>>> {
        use crate::lgx::deps::SiteKind;
        // A turn that writes nothing this component owns may still have changed a STORE, which
        // every reader of that store shows. Absence of evidence is not evidence, so it repaints.
        if fields.is_empty() {
            return None;
        }
        let plan = self.patch_plan(def, fields)?;
        let view = def.view.as_ref()?;
        let component = self.interner.resolve(def.name).to_string();
        let mut out: Vec<Stmt<'a>> = Vec::new();
        if plan.is_empty() {
            return Some(out);
        }
        // ★ DERIVED FIELDS ARE RECOMPUTED BEFORE THE PATCH READS THEM. A repaint got them for
        // free — the render function binds every state field, plain then derived, in declaration
        // order. A patch does not run the render, so a site showing `{remaining}` would resolve a
        // name nothing had bound. Recomputed here from the plain locals the write-back just
        // committed, in the same order, so a derivation reading another derivation still works.
        for field in def.state.iter().filter(|f| f.derived) {
            let bound = self.state_field_let(field, field.init, false);
            out.push(bound);
        }
        // ★ A PATCH WRITES THE RETAINED NODE IN PLACE: the node this instance's site names, its text
        // or live value set where it stands (`vdPatch*`), the change carried to the host by one
        // apply — so the retained tree and the document stay one tree, and the next paint's diff
        // compares against what is on screen.
        for (key, kind, _) in &plan {
            let held = self.site_read(&component, key);
            let site = self.vd_expr("vdSiteNode", vec![held]);
            let path = key.rsplit('@').next()?;
            let node = view_node_at(&view.nodes, path)?;
            match (kind, node) {
                (SiteKind::Text, ViewNode::Text { text, .. }) => {
                    let shown = self.as_text(text);
                    out.push(self.vd_call("vdPatchText", vec![site, shown]));
                }
                (SiteKind::Bind(field), ViewNode::Element { tag, attrs, .. }) => {
                    let shown = self.te_id(*field);
                    if bound_input_kind(*tag, attrs, self.interner).writes_checked() {
                        out.push(self.vd_call("vdPatchChecked", vec![site, shown]));
                    } else {
                        let text = self.as_text(shown);
                        out.push(self.vd_call("vdPatchValue", vec![site, text]));
                    }
                }
                // A site whose recorded shape does not match the view is a disagreement between
                // the walks, not something to guess at: repaint.
                _ => return None,
            }
        }
        out.push(self.vd_call("vdApply", Vec::new()));
        Some(out)
    }

    /// Every site of `def` a turn could write in place, in document order.
    pub(crate) fn patchable_site_keys(&self, def: &ComponentDef<'a>) -> Vec<String> {
        match crate::lgx::deps::component_deps(def, self.interner) {
            Ok(deps) => deps
                .sites
                .iter()
                .filter(|s| s.patchable() && !self.holds_view(def, s))
                .map(|s| s.key(self.interner))
                .collect(),
            // A derived cycle is refused by name in `lower_lgx_components` before anything reaches
            // here, so this arm is unreachable for that cause — it stays as the total answer for a
            // graph that will not build, and answering "no patchable sites" is the safe direction:
            // no site globals, no patchers, repaint.
            Err(_) => Vec::new(),
        }
    }

    /// The sites a turn that writes `fields` invalidates, or `None` when this turn cannot be
    /// patched and must repaint.
    ///
    /// `None` is the SAFE answer and the default for everything not yet handled: repainting when
    /// a patch would have done is waste, while patching when a rebuild was needed leaves a
    /// visitor looking at a page that no longer matches the state.
    pub(crate) fn patch_plan(
        &self,
        def: &ComponentDef<'a>,
        fields: &[Symbol],
    ) -> Option<Vec<(String, crate::lgx::deps::SiteKind, usize)>> {
        let deps = crate::lgx::deps::component_deps(def, self.interner).ok()?;
        let mut plan = Vec::new();
        for (index, site) in deps.sites.iter().enumerate() {
            if !fields.iter().any(|f| site.reads.contains(f)) {
                continue;
            }
            if !site.patchable() || self.holds_view(def, site) {
                return None;
            }
            plan.push((site.key(self.interner), site.kind.clone(), index));
        }
        Some(plan)
    }

    /// Whether `site` is a hole holding a View: its nodes are placed, not written as text, so a
    /// change to what it shows is a repaint, never a patch.
    fn holds_view(&self, def: &ComponentDef<'a>, site: &crate::lgx::deps::SiteDeps) -> bool {
        let Some(view) = def.view.as_ref() else {
            return false;
        };
        let key = site.key(self.interner);
        let Some(path) = key.strip_prefix("text@") else {
            return false;
        };
        match view_node_at(&view.nodes, path) {
            Some(ViewNode::Text { text, .. }) => {
                self.view_typed(super::views::sole_hole(text), def)
            }
            _ => false,
        }
    }

    /// `Let __lgx_key be "".` — the ROOT instance, bound as an ordinary local.
    ///
    /// Every synthesized function opens with this, so `state_read`/`state_write` always have a key
    /// in scope. A function that serves a REPEATED component takes the key as a parameter instead
    /// and omits this line; binding it as a local here is what lets a component placed once — every
    /// page that already worked — go through the identical instance-keyed path with a constant key,
    /// rather than keeping a second, scalar mechanism alive beside it.
    pub(crate) fn key_local(&mut self) -> Stmt<'a> {
        let var = self.interner.intern(instance_key_param());
        let root = self.interner.intern("");
        let value = self.te_text(root);
        Stmt::Let {
            var,
            ty: None,
            value,
            mutable: false,
        }
    }

    /// The `Text` type expression, built once rather than at each site that needs one.
    pub(crate) fn text_type(&mut self) -> &'a TypeExpr<'a> {
        let t = self.interner.intern("Text");
        self.ctx.alloc_type_expr(TypeExpr::Primitive(t))
    }

    /// The instance key this function is acting for, as an expression.
    pub(crate) fn key_expr(&mut self) -> &'a Expr<'a> {
        // ★ WHOSE INSTANCE, WHEN THE ANSWER IS NOT "THE ONE WHOSE CODE THIS IS".
        //
        // A wrapper's turn repaints the PAGE rather than itself, and every read a repaint makes —
        // where it painted, its props, its state — goes through this one door with `__lgx_key` in
        // it. Inside a wrapper's turn that identifier holds the WRAPPER's key, so the redirected
        // repaint would have looked the page up under the card's name: `item "/0" of __M_App`,
        // against a map holding only the root's `""`. A LOGOS map TRAPS on a missing key, so that
        // is a page that goes blank on a click rather than one that renders wrongly.
        //
        // Overriding here rather than at the three call sites is deliberate: they funnel through
        // this function precisely so they cannot disagree about who they are reading, and a fourth
        // reader added later inherits the answer instead of needing to be told.
        if let Some(k) = self.lgx_key_override {
            return self.te_text(k);
        }
        let k = self.interner.intern(instance_key_param());
        self.te_id(k)
    }

    /// `item <key> of __S_<C>_<field>` — reading THIS instance's value of a state field.
    ///
    /// ★ ONE DOOR. A state field is read by the render function, by every turn, and by the patch
    /// that follows a turn; a write happens in a turn and in a bind's write-back. If any one of
    /// them reached the global directly it would be reading a different instance's value from all
    /// the others, and the page would render one row's state into another's.
    pub(crate) fn state_read(&mut self, component: &str, field: Symbol) -> &'a Expr<'a> {
        let global = self
            .interner
            .intern(&state_binding(component, self.interner.resolve(field)));
        let key = self.key_expr();
        self.ctx.alloc_imperative_expr(Expr::Index {
            collection: self.te_id(global),
            index: key,
        })
    }

    /// `Let <field> be <value>` — a state field bound as a local of a turn, a render, a patch, a
    /// mount or a server walk, typed as the LOWERED program types it ([`Self::lowered_type`]): a
    /// `View` field is the program's `__View` enum, as a `View` prop or parameter is. The one place
    /// a state field becomes a binding, so no binding carries the surface type a tier would read as
    /// something else.
    pub(crate) fn state_field_let(
        &mut self,
        field: &crate::ast::component::StateField<'a>,
        value: &'a Expr<'a>,
        mutable: bool,
    ) -> Stmt<'a> {
        let ty = field.ty.map(|ty| self.lowered_type(ty));
        Stmt::Let {
            var: field.name,
            ty,
            value,
            mutable,
        }
    }

    /// `item <key> of __P_<C>_<prop>` — the prop value THIS instance was rendered with. A callback
    /// prop has no stored value: it reads as a closure over its trampoline, bound to this key.
    pub(crate) fn prop_read(&mut self, component: &str, prop: Symbol) -> &'a Expr<'a> {
        let callback = self
            .lgx_all_components
            .iter()
            .find(|d| self.interner.resolve(d.name) == component)
            .filter(|d| {
                d.props
                    .iter()
                    .any(|(p, t)| *p == prop && is_callback_type(t, self.interner))
            })
            .cloned();
        if let Some(def) = callback {
            return self.callback_invoker(&def, prop);
        }
        let global = self
            .interner
            .intern(&prop_binding(component, self.interner.resolve(prop)));
        let key = self.key_expr();
        self.ctx.alloc_imperative_expr(Expr::Index {
            collection: self.te_id(global),
            index: key,
        })
    }

    /// `Set item <key> of __P_<C>_<prop> to <value>` — recorded as the instance renders.
    pub(crate) fn prop_write(
        &mut self,
        component: &str,
        prop: Symbol,
        value: &'a Expr<'a>,
    ) -> Stmt<'a> {
        let global = self
            .interner
            .intern(&prop_binding(component, self.interner.resolve(prop)));
        let key = self.key_expr();
        Stmt::SetIndex {
            collection: self.te_id(global),
            index: key,
            value,
        }
    }

    /// A value of `ty` to seed a one-entry map with, so the map takes the right VALUE TYPE.
    ///
    /// ★ IT EXISTS ONLY TO TYPE THE MAP. The entry it creates is the ROOT instance's, and a root
    /// instance of a props-carrying component never renders (a component with props is placed, not
    /// mounted), so this value is never read — but the map's value type comes from it, and a
    /// mistyped map is a backend refusal rather than a wrong answer.
    pub(crate) fn zero_of_type(&mut self, ty: &'a TypeExpr<'a>) -> &'a Expr<'a> {
        if !self.lgx_views.functions.is_empty() && super::views::is_view_type(ty, self.interner) {
            return self.empty_view();
        }
        let name = match ty {
            TypeExpr::Primitive(n) => self.interner.resolve(*n).to_string(),
            _ => String::new(),
        };
        match name.as_str() {
            "Text" => {
                let empty = self.interner.intern("");
                self.te_text(empty)
            }
            "Bool" => self.te_bool(false),
            "Real" | "Float" => self
                .ctx
                .alloc_imperative_expr(Expr::Literal(Literal::Float(0.0))),
            // Int, and anything whose zero this walk does not know — a handle-shaped value rides an
            // integer, which is what an unrendered seed needs to be.
            _ => self.te_int(0),
        }
    }

    /// Record that `node` answers `event`, and emit the two writes that say so.
    ///
    /// ★ REGISTERED WHERE THE LISTENER IS ATTACHED, so the instance key in scope is the right one by
    /// construction rather than by a later lookup. That is the whole trick: a closure attached inside
    /// a render already closes over that render's key, and this records the same key beside it for
    /// the one caller that cannot invoke the closure — a driving verb on a tier that cannot call a
    /// closure it found at run time.
    pub(crate) fn register_listener(
        &mut self,
        node: &'a Expr<'a>,
        event_name: Symbol,
        site: ListenerSite,
    ) -> Vec<Stmt<'a>> {
        self.register_listener_with(node, event_name, site, &[])
    }

    pub(crate) fn register_listener_with(
        &mut self,
        node: &'a Expr<'a>,
        event_name: Symbol,
        site: ListenerSite,
        args: &[&'a Expr<'a>],
    ) -> Vec<Stmt<'a>> {
        let id = self.lgx_listeners.len() as i64;
        self.lgx_listeners.push(site);
        // ★ AN INTEGER KEY, NOT A STRING. This was `"{node}:{event}"`, which is a fresh Text
        // built on EVERY render for EVERY listener — a concat, a header and a copy, per paint. The
        // page's per-repaint cost rose about a fifth and `wasm_memory_bound_lock`'s growing-list
        // case, which has the least headroom, fell from 10,000+ turns to 4,063.
        //
        // The element is already a number and the event is known HERE at compile time, so the slot
        // is arithmetic: `node * __LISTENER_SPACE + code`. Nothing is allocated, and the dispatcher
        // recovers the code from the runtime event name once per dispatch rather than once per
        // listener per paint. The first listener on a slot is looked up BY the slot; each one after
        // it spills (`spill_when_taken`) under a lookup of its own.
        let entry = match self.lgx_listeners[id as usize].waits_for() {
            Some(want) => {
                let entry = keyed_listener_event(
                    self.interner.resolve(event_name),
                    self.interner.resolve(want),
                );
                self.interner.intern(&entry)
            }
            None => event_name,
        };
        let code = match self.lgx_listener_events.iter().position(|e| *e == entry) {
            Some(i) => i as i64,
            None => {
                self.lgx_listener_events.push(entry);
                (self.lgx_listener_events.len() - 1) as i64
            }
        };
        let space = self.interner.intern(listener_space_binding());
        let scaled = self.te_bin(BinaryOpKind::Multiply, node, self.te_id(space));
        let slot_value = self.te_bin(BinaryOpKind::Add, scaled, self.te_int(code));
        let slot = self.interner.intern(&format!("__lgx_slot{id}"));
        let lookup = self.interner.intern(&format!("__lgx_lk{id}"));
        let fns = self.interner.intern(listener_fn_binding());
        let keys = self.interner.intern(listener_key_binding());
        let which = self.key_expr();
        let paint_sites = self.interner.intern(paint_sites_local());
        let mut out = vec![
            Stmt::Let {
                var: slot,
                ty: None,
                value: slot_value,
                mutable: false,
            },
            Stmt::Let {
                var: lookup,
                ty: None,
                value: self.te_id(slot),
                mutable: true,
            },
        ];
        out.push(self.spill_when_taken(slot, lookup, id));
        let key = self.te_id(lookup);
        out.extend([
            Stmt::SetIndex {
                collection: self.te_id(fns),
                index: key,
                value: self.te_int(id),
            },
            Stmt::SetIndex {
                collection: self.te_id(keys),
                index: key,
                value: which,
            },
            // Recorded so THIS instance's next repaint can retire it (`lower_component_render`'s
            // own prune-then-write-back, at the top and tail of the function this call lives in).
            Stmt::Push {
                value: key,
                collection: self.te_id(paint_sites),
            },
        ]);
        if let Some(id) = self.lgx_listeners[id as usize].once_local() {
            let ids = self.interner.intern(listener_once_binding());
            out.push(Stmt::SetIndex {
                collection: self.te_id(ids),
                index: key,
                value: self.te_id(id),
            });
        }
        // Each argument's VALUE, evaluated here where the names it mentions are in scope.
        match &self.lgx_listeners[id as usize] {
            ListenerSite::Action {
                component, action, ..
            } => {
                let c = self.interner.resolve(*component).to_string();
                let a = self.interner.resolve(*action).to_string();
                for (i, arg) in args.iter().enumerate() {
                    let var = self.interner.intern(&listener_arg_binding(&c, &a, i));
                    out.push(Stmt::SetIndex {
                        collection: self.te_id(var),
                        index: key,
                        value: arg,
                    });
                }
            }
            // A callback prop's own arguments are typed by the PROP's declared shape, not by any
            // one target action's name — the target varies by placement, and every action ever
            // bound to this prop shares its parameter types by construction (the type checker
            // requires it). Keyed by `(component, prop)`, the twin of `listener_arg_binding`.
            ListenerSite::CallbackProp {
                component, prop, ..
            } => {
                let c = self.interner.resolve(*component).to_string();
                let p = self.interner.resolve(*prop).to_string();
                for (i, arg) in args.iter().enumerate() {
                    let var = self.interner.intern(&callback_arg_binding(&c, &p, i));
                    out.push(Stmt::SetIndex {
                        collection: self.te_id(var),
                        index: key,
                        value: arg,
                    });
                }
            }
            ListenerSite::Bind { .. }
            | ListenerSite::DefaultToggle { .. }
            | ListenerSite::ScrollNear { .. } => {}
        }
        out
    }

    /// `If the slot is taken: spill.` — a listener arriving at a slot another listener already holds
    /// (or whose earlier arrivals spilled) takes the next spilled lookup, `-1 - n`, and joins the
    /// end of the slot's list, so the dispatcher answers every listener on the slot in the order
    /// they registered.
    fn spill_when_taken(&mut self, slot: Symbol, lookup: Symbol, id: i64) -> Stmt<'a> {
        let fns = self.interner.intern(listener_fn_binding());
        let more = self.interner.intern(listener_spill_binding());
        let at = self.interner.intern(listener_spill_slot_binding());
        let spilled = self.interner.intern(listener_spill_count_binding());
        let held = self.ctx.alloc_imperative_expr(Expr::Contains {
            collection: self.te_id(fns),
            value: self.te_id(slot),
        });
        let queued = self.ctx.alloc_imperative_expr(Expr::Contains {
            collection: self.te_id(more),
            value: self.te_id(slot),
        });
        let taken = self.te_bin(BinaryOpKind::Or, held, queued);
        let list = self.interner.intern(&format!("__lgx_more{id}"));
        let int_name = self.interner.intern("Int");
        let count = self.te_bin(BinaryOpKind::Add, self.te_id(spilled), self.te_int(1));
        let below = self.te_bin(
            BinaryOpKind::Subtract,
            self.te_int(UNUSED_LOOKUP),
            self.te_id(spilled),
        );
        let earlier = self.ctx.alloc_imperative_expr(Expr::Index {
            collection: self.te_id(more),
            index: self.te_id(slot),
        });
        let queued_again = self.ctx.alloc_imperative_expr(Expr::Contains {
            collection: self.te_id(more),
            value: self.te_id(slot),
        });
        let spill = vec![
            Stmt::Set {
                target: spilled,
                value: count,
            },
            Stmt::Set {
                target: lookup,
                value: below,
            },
            Stmt::Let {
                var: list,
                ty: None,
                value: self.new_seq_expr(int_name),
                mutable: true,
            },
            Stmt::If {
                cond: queued_again,
                then_block: self.alloc_block(vec![Stmt::Set {
                    target: list,
                    value: earlier,
                }]),
                else_block: None,
            },
            Stmt::Push {
                value: self.te_id(lookup),
                collection: self.te_id(list),
            },
            Stmt::SetIndex {
                collection: self.te_id(more),
                index: self.te_id(slot),
                value: self.te_id(list),
            },
            Stmt::SetIndex {
                collection: self.te_id(at),
                index: self.te_id(lookup),
                value: self.te_id(slot),
            },
        ];
        Stmt::If {
            cond: taken,
            then_block: self.alloc_block(spill),
            else_block: None,
        }
    }

    /// `__lgx_forget_listener(lookup)` — retire the registration `lookup` names: its entry in every
    /// map keyed by lookup, and — a spilled one — its place in its slot's list, which leaves with
    /// the slot's last spilled listener. A lookup a delivery in flight holds is only marked, and is
    /// forgotten when the delivery lets go of it (`lower_lgx_dispatcher`). What one instance
    /// registered is retired by its next paint (`lower_component_render`) or, when it leaves the
    /// page, by the unmount sweep; both call this one function.
    pub(crate) fn forget_listener_fn(&mut self, all: &[ComponentDef<'a>]) -> Stmt<'a> {
        let lookup = self.interner.intern("__lgx_lk");
        let fns = self.interner.intern(listener_fn_binding());
        let keys = self.interner.intern(listener_key_binding());
        let mut retire: Vec<Stmt<'a>> = vec![
            Stmt::Remove {
                value: self.te_id(lookup),
                collection: self.te_id(fns),
            },
            Stmt::Remove {
                value: self.te_id(lookup),
                collection: self.te_id(keys),
            },
        ];
        // ★ EVERY MAP A LISTENER'S ARGUMENTS LIVE IN, UNCONDITIONALLY. A lookup belongs to whichever
        // action or callback registered it, but `Remove` on a Map is a no-op when the key is absent
        // — cheaper to ask every (few) argument map of every listening component than to carry, per
        // listener, which one it was for.
        let listening: Vec<ComponentDef<'a>> = all
            .iter()
            .filter(|d| self.lgx_components.contains(&d.name) && has_any_listener_site(d))
            .cloned()
            .collect();
        let mut once = false;
        for def in &listening {
            let component = self.interner.resolve(def.name).to_string();
            for action in &def.actions {
                let a = self.interner.resolve(action.name).to_string();
                for i in 0..action.params.len() {
                    let var = self
                        .interner
                        .intern(&listener_arg_binding(&component, &a, i));
                    retire.push(Stmt::Remove {
                        value: self.te_id(lookup),
                        collection: self.te_id(var),
                    });
                }
            }
            for (var, _) in self.callback_arg_maps(def, all) {
                retire.push(Stmt::Remove {
                    value: self.te_id(lookup),
                    collection: self.te_id(var),
                });
            }
            once |= has_once_handler(def);
        }
        // So is a `.once` listener's identity; its SPENT record is the instance's, not the site's.
        if once {
            let ids = self.interner.intern(listener_once_binding());
            retire.push(Stmt::Remove {
                value: self.te_id(lookup),
                collection: self.te_id(ids),
            });
        }
        let more = self.interner.intern(listener_spill_binding());
        let at = self.interner.intern(listener_spill_slot_binding());
        let slot = self.interner.intern("__lgx_rs");
        let kept = self.interner.intern("__lgx_rk");
        let other = self.interner.intern("__lgx_ro");
        let int_name = self.interner.intern("Int");
        let slot_of = self.ctx.alloc_imperative_expr(Expr::Index {
            collection: self.te_id(at),
            index: self.te_id(lookup),
        });
        let spilled_before = self.ctx.alloc_imperative_expr(Expr::Index {
            collection: self.te_id(more),
            index: self.te_id(slot),
        });
        let stays = self.te_bin(BinaryOpKind::NotEq, self.te_id(other), self.te_id(lookup));
        let keep = Stmt::If {
            cond: stays,
            then_block: self.alloc_block(vec![Stmt::Push {
                value: self.te_id(other),
                collection: self.te_id(kept),
            }]),
            else_block: None,
        };
        let remaining = self.ctx.alloc_imperative_expr(Expr::Length {
            collection: self.te_id(kept),
        });
        let emptied = self.te_bin(BinaryOpKind::Eq, remaining, self.te_int(0));
        let unspill = vec![
            Stmt::Let {
                var: slot,
                ty: None,
                value: slot_of,
                mutable: false,
            },
            Stmt::Remove {
                value: self.te_id(lookup),
                collection: self.te_id(at),
            },
            Stmt::Let {
                var: kept,
                ty: None,
                value: self.new_seq_expr(int_name),
                mutable: true,
            },
            Stmt::Repeat {
                pattern: crate::ast::stmt::Pattern::Identifier(other),
                iterable: spilled_before,
                body: self.alloc_block(vec![keep]),
            },
            Stmt::If {
                cond: emptied,
                then_block: self.alloc_block(vec![Stmt::Remove {
                    value: self.te_id(slot),
                    collection: self.te_id(more),
                }]),
                else_block: Some(self.alloc_block(vec![Stmt::SetIndex {
                    collection: self.te_id(more),
                    index: self.te_id(slot),
                    value: self.te_id(kept),
                }])),
            },
        ];
        let spilled = self.te_bin(
            BinaryOpKind::Lt,
            self.te_id(lookup),
            self.te_int(UNUSED_LOOKUP),
        );
        retire.push(Stmt::If {
            cond: spilled,
            then_block: self.alloc_block(unspill),
            else_block: None,
        });
        let held = self.interner.intern(listener_held_binding());
        let left = self.interner.intern(listener_left_binding());
        let is_held = self.ctx.alloc_imperative_expr(Expr::Contains {
            collection: self.te_id(held),
            value: self.te_id(lookup),
        });
        let mark = Stmt::SetIndex {
            collection: self.te_id(left),
            index: self.te_id(lookup),
            value: self.te_bool(true),
        };
        let body = vec![Stmt::If {
            cond: is_held,
            then_block: self.alloc_block(vec![mark]),
            else_block: Some(self.alloc_block(retire)),
        }];
        let int_ty: &'a TypeExpr<'a> = self.ctx.alloc_type_expr(TypeExpr::Primitive(int_name));
        Stmt::FunctionDef {
            name: self.interner.intern(forget_listener_fn_name()),
            generics: Vec::new(),
            params: vec![(lookup, int_ty)],
            body: self.alloc_block(body),
            return_type: None,
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        }
    }

    /// `name(args)` as a statement — a vdom.lg call the render writes the tape through.
    pub(crate) fn vd_call(&mut self, name: &str, args: Vec<&'a Expr<'a>>) -> Stmt<'a> {
        Stmt::Call {
            function: self.interner.intern(name),
            args,
        }
    }

    /// `name(args)` as an expression.
    pub(crate) fn vd_expr(&mut self, name: &str, args: Vec<&'a Expr<'a>>) -> &'a Expr<'a> {
        let function = self.interner.intern(name);
        self.te_call(function, args)
    }

    /// The site this instance finds one of its patchable nodes by — `__SITE_<C>_<site>[key]`, made
    /// the first time the instance draws that node and kept for the instance's life (the unmount
    /// sweep hands it back). Bound to a fresh local, returned.
    pub(crate) fn stable_site(
        &mut self,
        component: &str,
        site: &str,
        next: &mut usize,
        out: &mut Vec<Stmt<'a>>,
    ) -> Symbol {
        let local = self.lgx_temp(next);
        let held = self.site_read(component, site);
        let map = self.interner.intern(&site_binding(component, site));
        let this = self.key_expr();
        let known = self.ctx.alloc_imperative_expr(Expr::Contains {
            collection: self.te_id(map),
            value: this,
        });
        let read = Stmt::Set {
            target: local,
            value: held,
        };
        out.push(Stmt::Let {
            var: local,
            ty: None,
            value: self.te_int(0),
            mutable: true,
        });
        out.push(Stmt::If {
            cond: known,
            then_block: self.alloc_block(vec![read]),
            else_block: None,
        });
        let fresh = self.vd_expr("vdNewSite", Vec::new());
        let unset = self.te_bin(BinaryOpKind::Eq, self.te_id(local), self.te_int(0));
        let mint = vec![
            Stmt::Set {
                target: local,
                value: fresh,
            },
            self.site_write(component, site, self.te_id(local)),
        ];
        out.push(Stmt::If {
            cond: unset,
            then_block: self.alloc_block(mint),
            else_block: None,
        });
        local
    }

    /// A site for this paint only — an element a listener or directive must find after the commit
    /// — kept against the instance's fragment (`vdAnchor`), so its next paint, or its unmount,
    /// hands it back. Bound to a fresh local, returned.
    pub(crate) fn paint_anchor(&mut self, next: &mut usize, out: &mut Vec<Stmt<'a>>) -> Symbol {
        let local = self.lgx_temp(next);
        let frag = self.interner.intern(render_frag_param());
        let fresh = self.vd_expr("vdAnchor", vec![self.te_id(frag)]);
        out.push(Stmt::Let {
            var: local,
            ty: None,
            value: fresh,
            mutable: false,
        });
        local
    }

    /// The fragment instance `key` of `component` is drawn as — `__M_<C>[key]`, made the first time
    /// the instance is drawn and recorded against its owner so the commit that removes it names the
    /// instance (`__lgx_own_frag`). Bound to a fresh local, returned.
    pub(crate) fn instance_frag(
        &mut self,
        component: Symbol,
        key: &'a Expr<'a>,
        next: &mut usize,
        out: &mut Vec<Stmt<'a>>,
    ) -> Symbol {
        let name = self.interner.resolve(component).to_string();
        let local = self.lgx_temp(next);
        let map = self.interner.intern(&mount_binding(&name));
        let known = self.ctx.alloc_imperative_expr(Expr::Contains {
            collection: self.te_id(map),
            value: key,
        });
        let held = self.ctx.alloc_imperative_expr(Expr::Index {
            collection: self.te_id(map),
            index: key,
        });
        out.push(Stmt::Let {
            var: local,
            ty: None,
            value: self.te_int(0),
            mutable: true,
        });
        out.push(Stmt::If {
            cond: known,
            then_block: self.alloc_block(vec![Stmt::Set {
                target: local,
                value: held,
            }]),
            else_block: None,
        });
        let fresh = self.vd_expr("vdNewFrag", Vec::new());
        let unset = self.te_bin(BinaryOpKind::Eq, self.te_id(local), self.te_int(0));
        let owner_tag = self.component_tag(component);
        let own = self.interner.intern(own_frag_fn_name());
        let mut mint = vec![
            Stmt::Set {
                target: local,
                value: fresh,
            },
            Stmt::SetIndex {
                collection: self.te_id(map),
                index: key,
                value: self.te_id(local),
            },
        ];
        if self.instances_can_leave() {
            mint.push(Stmt::Call {
                function: own,
                args: vec![self.te_id(local), self.te_int(owner_tag), key],
            });
        }
        out.push(Stmt::If {
            cond: unset,
            then_block: self.alloc_block(mint),
            else_block: None,
        });
        local
    }

    /// The number the fragment registry records component `name` by: its position among every
    /// lowered component, from 1.
    pub(crate) fn component_tag(&self, name: Symbol) -> i64 {
        self.lgx_all_components
            .iter()
            .position(|d| d.name == name)
            .map_or(0, |i| i as i64 + 1)
    }

    /// `item <key> of __SITE_<C>_<site>` — the node THIS instance painted at that site.
    ///
    /// ★ A SITE IS PER-INSTANCE FOR THE SAME REASON STATE IS. A patch writes the new value into a
    /// remembered node, and two rows paint two different nodes at the same site — so a shared
    /// global would hold whichever rendered LAST, and a turn on one row would write into the other
    /// row's element. The page would update, visibly, in the wrong place.
    pub(crate) fn site_read(&mut self, component: &str, site: &str) -> &'a Expr<'a> {
        let global = self.interner.intern(&site_binding(component, site));
        let key = self.key_expr();
        self.ctx.alloc_imperative_expr(Expr::Index {
            collection: self.te_id(global),
            index: key,
        })
    }

    /// `Set item <key> of __SITE_<C>_<site> to <node>` — the write twin of [`site_read`].
    pub(crate) fn site_write(
        &mut self,
        component: &str,
        site: &str,
        node: &'a Expr<'a>,
    ) -> Stmt<'a> {
        let global = self.interner.intern(&site_binding(component, site));
        let key = self.key_expr();
        Stmt::SetIndex {
            collection: self.te_id(global),
            index: key,
            value: node,
        }
    }

    /// `Set item <key> of __S_<C>_<field> to <value>` — the write twin of [`state_read`].
    pub(crate) fn state_write(
        &mut self,
        component: &str,
        field: Symbol,
        value: &'a Expr<'a>,
    ) -> Stmt<'a> {
        let global = self
            .interner
            .intern(&state_binding(component, self.interner.resolve(field)));
        let key = self.key_expr();
        Stmt::SetIndex {
            collection: self.te_id(global),
            index: key,
            value,
        }
    }
}
