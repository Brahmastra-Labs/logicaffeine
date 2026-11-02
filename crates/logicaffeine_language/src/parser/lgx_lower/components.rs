use super::*;
use crate::ast::component::{ComponentDef, ComponentKind, RouteBody, Trigger};
use crate::ast::stmt::{Expr, Stmt};
use crate::ast::view::{bound_input_kind, Attr, BoundInput, ViewNode};
use logicaffeine_base::{Interner, Symbol};

/// Every field a view BINDS, in source order, WITH what each binding writes — one write-back
/// function each. The kind travels with the field because a checkbox's write-back stores a Bool
/// where a text input's stores Text, and getting that from the field name alone is impossible.
pub(crate) fn bound_fields(
    nodes: &[ViewNode<'_>],
    interner: &Interner,
) -> Vec<(Symbol, BoundInput)> {
    fn go(nodes: &[ViewNode<'_>], interner: &Interner, out: &mut Vec<(Symbol, BoundInput)>) {
        for node in nodes {
            if let ViewNode::Element { tag, attrs, .. } = node {
                for a in attrs {
                    if let Attr::Bind { field } = a {
                        if !out.iter().any(|(f, _)| f == field) {
                            out.push((*field, bound_input_kind(*tag, attrs, interner)));
                        }
                    }
                }
            }
            for sub in view_subtrees(node) {
                go(sub, interner, out);
            }
        }
    }
    let mut out: Vec<(Symbol, BoundInput)> = Vec::new();
    go(nodes, interner, &mut out);
    out
}

impl<'a, 'ctx, 'int> super::super::Parser<'a, 'ctx, 'int> {
    pub(crate) fn lower_lgx_components(
        &mut self,
        statements: &[Stmt<'a>],
    ) -> crate::parser::ParseResult<()> {
        let defs: Vec<ComponentDef<'a>> = statements
            .iter()
            .filter_map(|s| match s {
                Stmt::ComponentDef(d) => Some((**d).clone()),
                _ => None,
            })
            .collect();
        // A directive is behavior attached to an element: on this tier, a function.
        // ★ THE LISTENER TABLE, declared once for the whole program. Not per component, because a
        // driving verb names an element and has no idea which component rendered it — that is the
        // question the table answers, so it cannot be partitioned by the answer.
        //
        // ★★ AND ONLY WHEN THE FILE HAS COMPONENTS. `te_call` allocates an IMPERATIVE expression,
        // and a logic-mode parse (`## Axiom`, `## Theory`, `## Define`) never initialises those
        // arenas — so declaring this unconditionally panicked with "imperative arenas not
        // initialized" on files that contain no LGX at all. This exact mistake is recorded in this
        // campaign already: `lower_lgx_routes` once allocated its shared type exprs before checking
        // whether the file had any routes, and took out every logic-mode parser the same way.
        //
        // The rule it teaches, which is why the comment is here rather than in a commit message:
        // A LOWERING FOR A CONSTRUCT THE FILE DOES NOT USE MUST COST NOTHING — not "almost
        // nothing", nothing, because the arenas it would touch may not exist.
        if !defs.is_empty() {
            // Keyed by listener lookup, so the seed key is an Int — the map's KEY type comes from
            // this entry exactly as its value type does, and a Text seed here would type the map
            // against the thing that indexes it. The seed is `UNUSED_LOOKUP`, which no listener
            // has: a registration asks whether its slot is taken, and a seed on a real slot would
            // answer yes.
            let map_of = self.interner.intern("mapOf");
            let root = self.interner.intern("");
            let fns = self.interner.intern(listener_fn_binding());
            let seed_key = self.te_int(UNUSED_LOOKUP);
            let zero = self.te_int(0);
            let value = self.te_call(map_of, vec![seed_key, zero]);
            self.deferred_functions.push(Stmt::Let {
                var: fns,
                ty: None,
                value,
                mutable: true,
            });
            let keys = self.interner.intern(listener_key_binding());
            let seed_key = self.te_int(UNUSED_LOOKUP);
            let empty = self.te_text(root);
            let value = self.te_call(map_of, vec![seed_key, empty]);
            self.deferred_functions.push(Stmt::Let {
                var: keys,
                ty: None,
                value,
                mutable: true,
            });
            // The listeners past the first on one slot, and the holds deliveries in flight keep on
            // lookups — declared when something listens.
            if defs.iter().any(has_any_listener_site) {
                let int_name = self.interner.intern("Int");
                let more = self.interner.intern(listener_spill_binding());
                let none_spilled = self.new_seq_expr(int_name);
                let value = self.te_call(map_of, vec![self.te_int(UNUSED_LOOKUP), none_spilled]);
                self.deferred_functions.push(Stmt::Let {
                    var: more,
                    ty: None,
                    value,
                    mutable: true,
                });
                let at = self.interner.intern(listener_spill_slot_binding());
                let value = self.te_call(map_of, vec![self.te_int(UNUSED_LOOKUP), self.te_int(0)]);
                self.deferred_functions.push(Stmt::Let {
                    var: at,
                    ty: None,
                    value,
                    mutable: true,
                });
                let spilled = self.interner.intern(listener_spill_count_binding());
                self.deferred_functions.push(Stmt::Let {
                    var: spilled,
                    ty: None,
                    value: self.te_int(0),
                    mutable: true,
                });
                let held = self.interner.intern(listener_held_binding());
                let value = self.te_call(map_of, vec![self.te_int(UNUSED_LOOKUP), self.te_int(0)]);
                self.deferred_functions.push(Stmt::Let {
                    var: held,
                    ty: None,
                    value,
                    mutable: true,
                });
                let left = self.interner.intern(listener_left_binding());
                let value = self.te_call(
                    map_of,
                    vec![self.te_int(UNUSED_LOOKUP), self.te_bool(false)],
                );
                self.deferred_functions.push(Stmt::Let {
                    var: left,
                    ty: None,
                    value,
                    mutable: true,
                });
            }
            // The turns in progress, for the failure door to journal — declared whenever a turn can.
            if defs.iter().any(stateful) {
                let text_name = self.interner.intern("Text");
                for var in [open_turns_binding(), open_keys_binding()] {
                    let var = self.interner.intern(var);
                    let value = self.new_seq_expr(text_name);
                    self.deferred_functions.push(Stmt::Let {
                        var,
                        ty: None,
                        value,
                        mutable: true,
                    });
                }
            }
            // Which component's changed-field handlers are running, whose, and how deep the open
            // turns were — declared when some component has such handlers.
            if !watching_components(&defs).is_empty() {
                let blank = self.interner.intern("");
                let columns = [
                    (watching_binding(), self.te_int(0)),
                    (watch_key_binding(), self.te_text(blank)),
                    (watch_depth_binding(), self.te_int(0)),
                ];
                for (var, value) in columns {
                    let var = self.interner.intern(var);
                    self.deferred_functions.push(Stmt::Let {
                        var,
                        ty: None,
                        value,
                        mutable: true,
                    });
                }
                let var = self.interner.intern(watch_changes_binding());
                let text_name = self.interner.intern("Text");
                let value = self.new_seq_expr(text_name);
                self.deferred_functions.push(Stmt::Let {
                    var,
                    ty: None,
                    value,
                    mutable: true,
                });
            }
            // How deeply self-placing components are nested in the paint in flight — declared
            // when some lowered component can be drawn inside itself.
            if self.counts_render_depth(&defs) {
                let var = self.interner.intern(render_depth_binding());
                let value = self.te_int(0);
                self.deferred_functions.push(Stmt::Let {
                    var,
                    ty: None,
                    value,
                    mutable: true,
                });
            }
            // The `When this component appears:` handlers a paint owes — declared when some
            // lowered component has one.
            if !appearing_components(&defs, self.type_registry.as_ref(), self.interner).is_empty() {
                let int_name = self.interner.intern("Int");
                let text_name = self.interner.intern("Text");
                let columns = [
                    (appear_who_binding(), self.new_seq_expr(int_name)),
                    (appear_key_binding(), self.new_seq_expr(text_name)),
                    (appear_at_binding(), self.te_int(1)),
                ];
                for (var, value) in columns {
                    let var = self.interner.intern(var);
                    self.deferred_functions.push(Stmt::Let {
                        var,
                        ty: None,
                        value,
                        mutable: true,
                    });
                }
            }
            // Whether the page may still show a store as it was before a commit — declared when
            // a store can commit.
            if commits_stores(&defs) {
                let var = self.interner.intern(stores_dirty_binding());
                let value = self.te_bool(false);
                self.deferred_functions.push(Stmt::Let {
                    var,
                    ty: None,
                    value,
                    mutable: true,
                });
            }
            // ★ WHO IS WATCHING A STORE, known before any store lowers: a store action notes the
            // watched fields its commit moved, so the store has to know which of its fields anyone
            // watches — including watchers declared in the last component of the file.
            for def in &defs {
                if lowerable(def, &defs, self.type_registry.as_ref(), self.interner).is_err() {
                    continue;
                }
                for (index, handler) in def.events.iter().enumerate() {
                    if let Trigger::FieldChanges {
                        store: Some(store),
                        field,
                    } = handler.trigger
                    {
                        self.lgx_store_watchers
                            .push((def.name, store, field, index));
                    }
                }
            }
            // How many store-watcher rounds are running inside one another, and one flag per
            // watched store field — declared when some component watches a store.
            if self.wakes_store_watchers() {
                let var = self.interner.intern(cascade_binding());
                let value = self.te_int(0);
                self.deferred_functions.push(Stmt::Let {
                    var,
                    ty: None,
                    value,
                    mutable: true,
                });
                for (store, field) in self.watched_store_fields() {
                    let var = self.interner.intern(&store_moved_binding(
                        self.interner.resolve(store),
                        self.interner.resolve(field),
                    ));
                    let value = self.te_bool(false);
                    self.deferred_functions.push(Stmt::Let {
                        var,
                        ty: None,
                        value,
                        mutable: true,
                    });
                }
            }
            // The `.once` records — declared exactly when some handler is `.once` (`uses_once`, one
            // question). The spent map is keyed by listener identity, seeded with a key no identity
            // has; the identity map is keyed like the listener table.
            if uses_once(&defs) {
                let spent = self.interner.intern(once_spent_binding());
                let seed = self.interner.intern(super::naming::seed_key());
                let value = self.te_call(map_of, vec![self.te_text(seed), self.te_bool(false)]);
                self.deferred_functions.push(Stmt::Let {
                    var: spent,
                    ty: None,
                    value,
                    mutable: true,
                });
                let ids = self.interner.intern(listener_once_binding());
                let value =
                    self.te_call(map_of, vec![self.te_int(UNUSED_LOOKUP), self.te_text(root)]);
                self.deferred_functions.push(Stmt::Let {
                    var: ids,
                    ty: None,
                    value,
                    mutable: true,
                });
            }
        }
        // ★ A TRAP IN A DIRECTIVE NAMES THE DIRECTIVE. Each block opens `__LGX_CONTEXT` with the
        // sentence every tier puts in front of the trap and closes it on every way out; a host
        // that catches the trap takes the context (`__lgx_failure_context`) and names it.
        let has_directives = statements
            .iter()
            .any(|s| matches!(s, Stmt::DirectiveDef { .. }));
        if has_directives {
            let context = self.interner.intern(failure_context_binding());
            let blank = self.interner.intern("");
            let value = self.te_text(blank);
            self.deferred_functions.push(Stmt::Let {
                var: context,
                ty: None,
                value,
                mutable: true,
            });
            let taken = self.interner.intern("__lgx_ctx");
            let body = vec![
                Stmt::Let {
                    var: taken,
                    ty: None,
                    value: self.te_id(context),
                    mutable: false,
                },
                Stmt::Set {
                    target: context,
                    value: self.te_text(blank),
                },
                Stmt::Return {
                    value: Some(self.te_id(taken)),
                },
            ];
            let text_ty = self
                .ctx
                .alloc_type_expr(crate::ast::stmt::TypeExpr::Primitive(
                    self.interner.intern("Text"),
                ));
            self.deferred_functions.push(Stmt::FunctionDef {
                name: self.interner.intern(failure_context_fn_name()),
                generics: Vec::new(),
                params: Vec::new(),
                body: self.alloc_block(body),
                return_type: Some(text_ty),
                is_native: false,
                native_path: None,
                is_exported: false,
                export_target: None,
                opt_flags: crate::optimization::OptimizationConfig::all_on(),
            });
        }
        for stmt in statements {
            let Stmt::DirectiveDef {
                name,
                params,
                body,
                removed,
            } = stmt
            else {
                continue;
            };
            let directive = self.interner.resolve(*name).to_string();
            let blocks = std::iter::once((directive_fn_name(&directive), *body, false))
                .chain(removed.map(|r| (directive_removed_fn_name(&directive), r, true)));
            for (fname, block, is_removed) in blocks {
                let fname = self.interner.intern(&fname);
                let context = self.interner.intern(failure_context_binding());
                let sentence =
                    self.interner
                        .intern(&crate::ast::component::directive_failure_prefix(
                            &directive, is_removed,
                        ));
                let blank = self.interner.intern("");
                let mut body = vec![Stmt::Set {
                    target: context,
                    value: self.te_text(sentence),
                }];
                body.extend(self.protect_early_return(block.to_vec()));
                body.push(Stmt::Set {
                    target: context,
                    value: self.te_text(blank),
                });
                let block = self.alloc_block(body);
                self.deferred_functions.push(Stmt::FunctionDef {
                    name: fname,
                    generics: Vec::new(),
                    params: params.clone(),
                    body: block,
                    return_type: None,
                    is_native: false,
                    native_path: None,
                    is_exported: false,
                    export_target: None,
                    opt_flags: crate::optimization::OptimizationConfig::all_on(),
                });
            }
        }
        // ★ WHAT EACH `When removed:` DIRECTIVE HAS ATTACHED, per occurrence — the list the sweep
        // walks, the pass each was last drawn in, and the element and value it was drawn with,
        // which its removed block receives. Declared only for a directive that has the block.
        if !defs.is_empty() {
            for (directive, value_ty) in self.removable_directives() {
                let map_of = self.interner.intern("mapOf");
                let seed = self.interner.intern(super::naming::seed_key());
                let keys = self.interner.intern(&directive_keys_binding(&directive));
                let seeded = self
                    .ctx
                    .alloc_imperative_expr(Expr::List(vec![self.te_text(seed)]));
                self.deferred_functions.push(Stmt::Let {
                    var: keys,
                    ty: None,
                    value: seeded,
                    mutable: true,
                });
                let columns = [
                    (directive_stamp_binding(&directive), self.te_int(-1)),
                    (directive_node_binding(&directive), self.te_int(0)),
                ];
                for (var, init) in columns {
                    let var = self.interner.intern(&var);
                    let value = self.te_call(map_of, vec![self.te_text(seed), init]);
                    self.deferred_functions.push(Stmt::Let {
                        var,
                        ty: None,
                        value,
                        mutable: true,
                    });
                }
                if let Some(ty) = value_ty {
                    let var = self.interner.intern(&directive_value_binding(&directive));
                    let zero = self.zero_of_type(ty);
                    let value = self.te_call(map_of, vec![self.te_text(seed), zero]);
                    self.deferred_functions.push(Stmt::Let {
                        var,
                        ty: None,
                        value,
                        mutable: true,
                    });
                }
                if !self.lgx_declared_pass {
                    self.lgx_declared_pass = true;
                    let var = self.interner.intern(pass_binding());
                    let value = self.te_int(0);
                    self.deferred_functions.push(Stmt::Let {
                        var,
                        ty: None,
                        value,
                        mutable: true,
                    });
                }
            }
        }
        // ★ A CYCLE OF DERIVED STATE IS REFUSED HERE, WHERE IT IS WRITTEN.
        //
        // The dependency walk has always detected it and built a message naming every field on the
        // circle — and `component_deps` is called at two sites that both DISCARD the error, each
        // under the same comment: "a derived cycle is refused elsewhere". There was no elsewhere.
        // What the author got was `Undefined variable: b` about a field they had declared three
        // lines up, which sends them hunting a typo in a name that is spelled correctly.
        //
        // This is the third instance of one shape today: a reason computed in full at the place
        // that had it, then dropped, so the failure surfaces somewhere with nothing to say.
        for def in &defs {
            if def.state.iter().all(|f| !f.derived) {
                continue;
            }
            if let Err(crate::lgx::deps::DepError::DerivedCycle(names)) =
                crate::lgx::deps::component_deps(def, self.interner)
            {
                let chain = names.join("` → `");
                let component = self.interner.resolve(def.name).to_string();
                let at = names
                    .first()
                    .and_then(|first| {
                        def.state
                            .iter()
                            .find(|f| self.interner.resolve(f.name) == first)
                    })
                    .map_or(def.span, |f| f.span);
                return Err(crate::error::ParseError {
                    kind: crate::error::ParseErrorKind::Custom(format!(
                        "`{component}` has derived values that depend on each other in a circle: \
                         `{chain}`. A `Let … always be …` is recomputed whenever what it reads \
                         changes, so a circle has no answer to settle on — whichever one happened \
                         to be computed last would win, and the next turn could just as easily \
                         pick the other. Which of these is the one a person actually changes? \
                         Make that one plain state (`Let … be …`) that an action writes, and let \
                         the rest derive from it."
                    )),
                    span: at,
                });
            }
        }
        // A store lowers first: a component that uses one needs its record to exist.
        for def in &defs {
            if !store_lowerable(def, &self.json_shapes()) {
                continue;
            }
            if let Some(stmts) = self.lower_store(def) {
                self.deferred_functions.extend(stmts);
            }
        }
        self.lgx_all_components = defs.clone();
        // ★ THE PLACEMENT GRAPH, ONCE. `repaint_def` is emitted several times per component and
        // asked the same two questions each time — who is placed, and which component is the page.
        // Recomputing them per call also meant CLONING every component definition per call, which
        // is a lot of work to keep answering with the same answer.
        self.lgx_placed = defs
            .iter()
            .filter_map(|d| d.view.as_ref())
            .flat_map(|v| placed_components(&v.nodes))
            .collect();
        self.lgx_loop_placed = defs
            .iter()
            .filter_map(|d| d.view.as_ref())
            .flat_map(|v| loop_placed_components(&v.nodes))
            .collect();
        // A page is mounted, never handed to anything: the root is the component nobody places.
        //
        // ★ AND IT MUST BE SOMETHING THAT PAINTS. This used to take the first unplaced propless
        // definition of ANY kind — and a `## Store` is exactly that: nothing places it and it has
        // no props, so on any page with a store the STORE was elected the paint root. The
        // consequences were silent and two-fold: the root's stylesheets were emitted by a
        // definition that never renders (so a styled page served its CSS and painted none), and a
        // wrapper's repaint redirected to a store rather than to the page.
        //
        // Found by building an app that combined a store with a wrapper — neither feature alone
        // shows it, which is why every earlier test passed.
        self.lgx_paint_root = defs
            .iter()
            .find(|d| {
                matches!(
                    d.kind,
                    ComponentKind::Component | ComponentKind::Page { .. }
                ) && !self.lgx_placed.contains(&d.name)
                    && d.props.is_empty()
            })
            .map(|d| d.name);
        // Every component a `## Route … -> View:` template or a `-> View` function's template
        // places, directly: those are server roots exactly as `html of <Name>` is, and their
        // `__html_` must exist before the route lowering (which runs next) emits the call to it.
        let routed: Vec<Symbol> = statements
            .iter()
            .filter_map(|s| match s {
                Stmt::RouteDef { body, .. } => match &**body {
                    RouteBody::View(view) => Some(view),
                    _ => None,
                },
                other => self.view_template_of(other).map(|(_, _, view)| view),
            })
            .flat_map(|view| placed_components(&view.nodes))
            .collect();
        for def in &defs {
            self.lower_component_serve(def, &defs, &routed)?;
        }

        // ★ A PRE-PASS, BECAUSE A TURN MUST KNOW ABOUT EVERY COMPONENT, NOT JUST THE ONES ALREADY
        // LOWERED. These three registries — the unmount sweeps, the timers, and the failure
        // handlers — are read while lowering a component's TURNS, and a turn of the first
        // component repaints (and so sweeps) every other one. Populated inside the emission loop,
        // the first component's turns were built against an empty registry and silently omitted
        // the sweep, the tick and the failure id for every component declared after it. Order in a
        // single pass is invisible; two passes make it impossible.
        for def in &defs {
            if lowerable(def, &defs, self.type_registry.as_ref(), self.interner).is_err() {
                continue;
            }
            let component = self.interner.resolve(def.name).to_string();
            // ★ THE SAME SHAPE, FOUND WHILE FIXING THE OTHERS. `repaint_def` walks
            // `styled_components` to clear each stylesheet flag, so a turn of the FIRST component
            // was lowered against a list that did not yet know about any component declared after
            // it — and a repaint driven by that turn would not re-emit those stylesheets. It was
            // never reported because a page usually styles its first component. Registered here
            // with the rest, where the whole program is known.
            // ★ AND A FOURTH, FOR THE SAME REASON. `repaint_def` has to know whether the
            // component it is repainting takes CHILDREN, and a turn of the first component can
            // repaint the last one.
            if def
                .view
                .as_ref()
                .is_some_and(|v| view_takes_children(&v.nodes))
            {
                self.lgx_wrappers.push(def.name);
            }
            if def.style.is_some() {
                let var = self.interner.intern(&styled_binding(&component));
                self.styled_components.push(def.name);
                let value = self.te_bool(false);
                self.deferred_functions.push(Stmt::Let {
                    var,
                    ty: None,
                    value,
                    mutable: true,
                });
            }
            // The seen-map and the pass counter exist only for a component that asks to be told
            // when it leaves, so a page that never writes the sentence carries neither.
            if let Some(idx) = def
                .events
                .iter()
                .position(|h| matches!(h.trigger, Trigger::ActionFails { .. }))
            {
                if !self.lgx_fail_handlers.iter().any(|(c, _)| *c == def.name) {
                    self.lgx_fail_handlers.push((def.name, idx));
                    if self.lgx_fail_handlers.len() == 1 {
                        // Declared once for the program: which component is mid-turn, and which
                        // of its instances. `0` is "nobody with a handler", the state before the
                        // first turn and the state during any turn that cannot handle a failure.
                        let which = self.interner.intern(failing_component_binding());
                        let zero = self.te_int(0);
                        self.deferred_functions.push(Stmt::Let {
                            var: which,
                            ty: None,
                            value: zero,
                            mutable: true,
                        });
                        let whose = self.interner.intern(failing_key_binding());
                        let root = self.interner.intern("");
                        let empty = self.te_text(root);
                        self.deferred_functions.push(Stmt::Let {
                            var: whose,
                            ty: None,
                            value: empty,
                            mutable: true,
                        });
                    }
                }
            }
            // ★ THE TIMERS OF THIS COMPONENT. `Every 60 seconds:` and `After 5 seconds:` differ
            // only in whether they re-arm, so they share every piece: per instance, the page time
            // it is next due (`__DUE_`), its arming order (`__SEQ_`) and its period as its sentence
            // read when the instance was drawn (`__PERIOD_`); and one arm in `__lgx_tick`.
            let timers: Vec<(usize, bool, Option<Symbol>)> = def
                .events
                .iter()
                .enumerate()
                .filter_map(|(i, h)| match &h.trigger {
                    Trigger::Every { name, .. } => Some((i, true, *name)),
                    Trigger::After { name, .. } => Some((i, false, *name)),
                    _ => None,
                })
                .collect();
            for (i, repeating, name) in &timers {
                // `-1` in each is "not running": the seed entry types the map and never fires.
                for var in [
                    due_binding(&component, *i),
                    seq_binding(&component, *i),
                    period_binding(&component, *i),
                ] {
                    let var = self.interner.intern(&var);
                    let map_of = self.interner.intern("mapOf");
                    let seed = self.interner.intern(seed_key());
                    let seed_key_expr = self.te_text(seed);
                    let off = self.te_int(-1);
                    let value = self.te_call(map_of, vec![seed_key_expr, off]);
                    self.deferred_functions.push(Stmt::Let {
                        var,
                        ty: None,
                        value,
                        mutable: true,
                    });
                }
                self.lgx_timers.push((def.name, *i, *repeating, *name));
            }
            // ★ AND A STORE-WATCHER NEEDS ITS KEYS TOO. Telling a component that a store moved
            // means telling every instance of it, and the list of live instances is this list —
            // the one the unmount sweep and the timers already keep. One registry, three readers.
            // ★ AND A COMPONENT THAT OWNS STATE NEEDS ITS KEYS, because an instance that leaves
            // the page must take its state with it. Without a registry nothing notices the
            // instance vanished, so `__S_<C>_<field>` kept the entry and a REMOUNT under the same
            // key found the old value: a `{if on:}<Tile/>{end}` toggled off and on again would come
            // back showing its previous count where a fresh instance shows 0.
            let wants_keys = !timers.is_empty() || keeps_instance_registry(def);
            if wants_keys {
                let var = self.interner.intern(&keys_binding(&component));
                let seed = self.interner.intern(seed_key());
                let seed_key_expr = self.te_text(seed);
                let seeded = self
                    .ctx
                    .alloc_imperative_expr(Expr::List(vec![seed_key_expr]));
                self.deferred_functions.push(Stmt::Let {
                    var,
                    ty: None,
                    value: seeded,
                    mutable: true,
                });
            }
            // ★ THE PRESENCE MAP IS DECLARED WHENEVER THE KEY LIST IS, because the list cannot
            // guard itself: asking a SEQUENCE whether it contains a Text is not something the wasm
            // backend lowers, while asking a MAP whether it holds a key is. Every reader of
            // `__KEYS_<C>` — the sweep, the timers, and now a store watcher — needs the same
            // "have I seen this instance" test, so the map that answers it belongs with the list
            // rather than with any one of them.
            if wants_keys {
                let var = self.interner.intern(&seen_binding(&component));
                let map_of = self.interner.intern("mapOf");
                let seed = self.interner.intern(seed_key());
                let root_key = self.te_text(seed);
                let minus = self.te_int(-1);
                let value = self.te_call(map_of, vec![root_key, minus]);
                self.deferred_functions.push(Stmt::Let {
                    var,
                    ty: None,
                    value,
                    mutable: true,
                });
                // ★ THE PASS COUNTER BELONGS WITH THE REGISTRY, not with `Disappears`. It was
                // declared on the first component that asked to be told about unmounts, which was
                // fine while that was the only thing reading it — and the moment the sweep began
                // clearing state for every stateful component, a page with no `Disappears` handler
                // referenced a binding nothing had declared ("Undefined variable: __LGX_PASS").
                // Whoever keeps a registry stamps into it, and a stamp is meaningless without the
                // number it stamps.
                if !self.lgx_declared_pass {
                    self.lgx_declared_pass = true;
                    let var = self.interner.intern(pass_binding());
                    let value = self.te_int(0);
                    self.deferred_functions.push(Stmt::Let {
                        var,
                        ty: None,
                        value,
                        mutable: true,
                    });
                }
            }
            if let Some(idx) = def
                .events
                .iter()
                .position(|h| matches!(h.trigger, Trigger::Disappears))
            {
                // ★ SEEDED AS ALREADY-TOLD (`-1`) where it is declared above, NOT AS PASS 0. A
                // `0` reads as "rendered in pass 0 and not since", so the first paint reported the
                // seed as having disappeared before anything had appeared. `-1` says "not
                // currently on the page", which is what is true of a key no instance has.
                if !self.lgx_sweeps.iter().any(|(c, _)| *c == def.name) {
                    self.lgx_sweeps.push((def.name, idx));
                }
            }
        }
        // The page's own clock and its arming count, once `self.lgx_timers` is complete; and the
        // wake request every arming, stop and tick ends with.
        if !self.lgx_timers.is_empty() {
            for var in [now_binding(), timer_seq_binding()] {
                let var = self.interner.intern(var);
                let value = self.te_int(0);
                self.deferred_functions.push(Stmt::Let {
                    var,
                    ty: None,
                    value,
                    mutable: true,
                });
            }
            if let Some(rearm) = self.lower_lgx_rearm() {
                self.deferred_functions.push(rearm);
            }
        }
        // The store-watcher round every turn ends with — after the instance registries it walks.
        if let Some(wake) = self.lower_wake_stores() {
            self.deferred_functions.push(wake);
        }
        // The appears handlers every paint ends by running.
        let appearing = appearing_components(&defs, self.type_registry.as_ref(), self.interner);
        if let Some(appear) = self.lower_lgx_appear(&appearing) {
            self.deferred_functions.push(appear);
        }
        // The outbox the element events a turn asks for wait in until it commits — declared exactly
        // when the dispatcher that flushes it has its flush arm (`uses_outbox`, one question).
        if uses_outbox(&defs) {
            let count = self.interner.intern(outbox_count_binding());
            let zero = self.te_int(0);
            self.deferred_functions.push(Stmt::Let {
                var: count,
                ty: None,
                value: zero,
                mutable: true,
            });
            for var in [
                outbox_selector_binding(),
                outbox_event_binding(),
                outbox_value_binding(),
            ] {
                let var = self.interner.intern(var);
                let value = self.empty_outbox_list();
                self.deferred_functions.push(Stmt::Let {
                    var,
                    ty: None,
                    value,
                    mutable: true,
                });
            }
        }
        // The components whose actions are lowered to `__act_*` functions — the only ones a callback
        // trampoline may call.
        let lowered: Vec<Symbol> = defs
            .iter()
            .filter(|d| lowerable(d, &defs, self.type_registry.as_ref(), self.interner).is_ok())
            .map(|d| d.name)
            .collect();
        for def in &defs {
            if !lowered.contains(&def.name) {
                continue;
            }
            // State that CHANGES lives in module-level bindings, initialized once at program
            // start — which is exactly what a component with ONE instance means, and why a
            // repeated component with changing state is refused rather than lowered.
            //
            // ★ Only when it changes. A component with no actions, no event sentences and no
            // `bind` reads its initializers each render, so a module binding for it is written
            // once and read never — and its INITIALIZER is emitted twice. For a page holding a
            // list literal that doubled the module: 42KB of data became 85KB, all of the second
            // copy dead. Found by `bundle_size_lock`, which is what a size ratchet is for.
            let component = self.interner.resolve(def.name).to_string();
            {
                // ★ WHERE EACH INSTANCE PAINTED — declared for EVERY lowerable component, not only
                // the stateful ones, because a mount writes it before painting and a stateless
                // component is mounted like any other. Gating it on `stateful` left `__M_App`
                // undefined for a page whose own state never changes.
                let var = self.interner.intern(&mount_binding(&component));
                let root = self.interner.intern("");
                let map_of = self.interner.intern("mapOf");
                let root_key = self.te_text(root);
                let zero = self.te_int(0);
                let value = self.te_call(map_of, vec![root_key, zero]);
                self.deferred_functions.push(Stmt::Let {
                    var,
                    ty: None,
                    value,
                    mutable: true,
                });
            }
            // ★ THE LISTENER-SITE LEDGER, declared for every lowerable component that registers
            // ANY listener (`has_listener_site`) for the same reason `__M_<C>` is unconditional: a
            // component that only ever gets PLACED (never independently repainted) is still
            // called again every time its mount point's owner repaints, with fresh node ids each
            // time — it needs its own retirement record exactly as much as a stateful one does.
            // Gated on `has_any_listener_site`, unlike `__M_<C>`: a component with no `on-*`/`bind`/
            // `When … :` handler at all registers nothing this ledger could ever be asked to
            // retire, and paid for it anyway — `bundle_size_lock`'s own finding (2,626 B on a page
            // that places components which never listen for anything).
            if has_any_listener_site(def) {
                let var = self.interner.intern(&listener_sites_binding(&component));
                let root = self.interner.intern("");
                let map_of = self.interner.intern("mapOf");
                let root_key = self.te_text(root);
                let int_name = self.interner.intern("Int");
                let empty_sites = self.new_seq_expr(int_name);
                let value = self.te_call(map_of, vec![root_key, empty_sites]);
                self.deferred_functions.push(Stmt::Let {
                    var,
                    ty: None,
                    value,
                    mutable: true,
                });
            }
            // What each instance's `.once` listeners have spent, for the sweep to forget.
            if self.once_recorded(def.name) {
                let var = self.interner.intern(&once_record_binding(&component));
                let seed = self.interner.intern(seed_key());
                let map_of = self.interner.intern("mapOf");
                let text_name = self.interner.intern("Text");
                let empty = self.new_seq_expr(text_name);
                let value = self.te_call(map_of, vec![self.te_text(seed), empty]);
                self.deferred_functions.push(Stmt::Let {
                    var,
                    ty: None,
                    value,
                    mutable: true,
                });
            }
            // Each callback prop this component's own elements call: the identity its placer
            // records (tag + owning key) and one map per declared parameter.
            for stmt in self.callback_prop_declarations(def, &defs) {
                self.deferred_functions.push(stmt);
            }
            for (prop, _) in callback_props(def, self.interner) {
                let trampoline = self.callback_trampoline(def, prop, &defs, &lowered);
                self.deferred_functions.push(trampoline);
            }
            // ★ ONE MAP PER ACTION PARAMETER, typed by the parameter's own declared type. An
            // argument is evaluated where the listener is ATTACHED — `remove(todo's id)` names a
            // loop item the dispatcher has never heard of — so the value is recorded there and read
            // back by the element+event key.
            for action in &def.actions {
                let a = self.interner.resolve(action.name).to_string();
                for (i, (_, ty)) in action.params.iter().enumerate() {
                    let var = self
                        .interner
                        .intern(&listener_arg_binding(&component, &a, i));
                    let map_of = self.interner.intern("mapOf");
                    let root_key = self.te_int(0);
                    let seed = self.zero_of_type(ty);
                    let value = self.te_call(map_of, vec![root_key, seed]);
                    self.deferred_functions.push(Stmt::Let {
                        var,
                        ty: None,
                        value,
                        mutable: true,
                    });
                }
            }
            if stateful(def) {
                // What each instance was handed — read back by a repaint, which has no parent to
                // ask. Only a stateful component ever repaints, so only it needs them.
                let root = self.interner.intern("");
                let map_of = self.interner.intern("mapOf");
                for (prop, ty) in &def.props {
                    // A callback prop is never a value: its trampoline reads its identity instead.
                    if is_callback_type(ty, self.interner) {
                        continue;
                    }
                    let var = self
                        .interner
                        .intern(&prop_binding(&component, self.interner.resolve(*prop)));
                    let root_key = self.te_text(root);
                    let seed = self.zero_of_type(ty);
                    let value = self.te_call(map_of, vec![root_key, seed]);
                    self.deferred_functions.push(Stmt::Let {
                        var,
                        ty: None,
                        value,
                        mutable: true,
                    });
                }
            }
            if stateful(def) {
                for field in def.state.iter().filter(|f| !f.derived) {
                    let var = self.interner.intern(&state_binding(
                        &component,
                        self.interner.resolve(field.name),
                    ));
                    // ★ A STATE BINDING DECLARES ITS TYPE WHEN THE INITIALIZER NAMES ONE.
                    //
                    // The binding becomes a module GLOBAL, and the wasm backend resolves a field
                    // read out of a list by its element's DECLARED type — a function that only
                    // READS the global has no construction to trace. Without a type here, a page
                    // whose `State:` holds a `Seq of <record>` (which is every page showing a
                    // table of anything) could not read one field of one row.
                    //
                    // Inferred only where it is unambiguous: `a new Seq of Todo`, or a literal
                    // whose first element is a record. Anything else keeps the author's own
                    // annotation, or none.
                    // ★ A ONE-ENTRY MAP, NOT A SCALAR: `mapOf("", <init>)`. State is per INSTANCE,
                    // and an instance is named by its key — so the binding holds a map from key to
                    // value and the ROOT instance (key `""`) is seeded here with the initializer.
                    //
                    // The literal form is what makes this need no annotation: a map built from an
                    // entry takes its value type from that entry, so a field whose type nobody
                    // wrote is typed by its own initializer exactly as the scalar binding was.
                    let map_of = self.interner.intern("mapOf");
                    let root = self.interner.intern("");
                    let root_key = self.te_text(root);
                    let value = self.te_call(map_of, vec![root_key, field.init]);
                    self.deferred_functions.push(Stmt::Let {
                        var,
                        ty: None,
                        value,
                        mutable: true,
                    });
                }
            }
            // ★ ONE GLOBAL PER PATCHABLE SITE — the compiled half of Stage B. The render function
            // writes each site's node here; a per-turn patcher reads it back and writes the new
            // value in, so a turn that moves one field costs one `setText` rather than clearing
            // the mount and rebuilding everything under it.
            //
            // Seeded 0, which is no node: before the first paint there is nothing to patch, and a
            // patcher that ran then would be writing into the sentinel. It cannot happen — a turn
            // only runs after a mount — and the seed says so rather than leaving it to luck.
            if stateful(def) {
                for key in self.patchable_site_keys(def) {
                    let var = self.interner.intern(&site_binding(&component, &key));
                    // A map from instance key to the node that instance painted here — seeded
                    // with the root's 0, which is "no node": before the first paint there is
                    // nothing to patch, and the seed says so rather than leaving it to luck.
                    let map_of = self.interner.intern("mapOf");
                    let root = self.interner.intern("");
                    let root_key = self.te_text(root);
                    let zero = self.te_int(0);
                    let value = self.te_call(map_of, vec![root_key, zero]);
                    self.deferred_functions.push(Stmt::Let {
                        var,
                        ty: None,
                        value,
                        mutable: true,
                    });
                }
            }
            // (`__M_<C>` is declared above as a per-instance MAP. The scalar declaration that used
            // to sit here shadowed it, and a shadowed scalar is what "Cannot index into Int" was.)
            for action in &def.actions {
                if let Some(function) = self.lower_component_action(def, action) {
                    self.deferred_functions.push(function);
                }
            }
            // One write-back function per BOUND field. The listener a bound input registers can
            // only be a closure, and a closure captures what it touches — so the write has to
            // happen somewhere that resolves the module binding directly, or the keystroke lands
            // on a copy and the page never changes.
            if let Some(view) = &def.view {
                for (field, kind) in bound_fields(&view.nodes, self.interner) {
                    if let Some(function) = self.lower_bind_writeback(def, field, kind) {
                        self.deferred_functions.push(function);
                    }
                }
            }
            for (index, handler) in def.events.iter().enumerate() {
                if let Some(function) = self.lower_component_event(def, handler, index) {
                    self.deferred_functions.push(function);
                }
            }
            if let Some(function) = self.lower_component_render(def, &defs) {
                self.deferred_functions.push(function);
            }
            if let Some(function) = self.lower_component_mount(def) {
                self.deferred_functions.push(function);
            }
        }
        // ★ THESE TWO DO NOT DEPEND ON THERE BEING A DISPATCHER, and putting them inside the block
        // below said otherwise. The dispatcher exists only when something registered a DOM
        // LISTENER; a page whose only trigger is `Every 10 seconds:` registers none, so its tick
        // was never emitted and the interval closure called a function that was not there —
        // "Unknown function: `__lgx_tick`", at run time, in the browser. A clock and a failure
        // handler are answers to things that are not clicks.
        // The page's own route, as the one predicate an in-page move and a document load are told
        // apart by (`__lgx_owns`), and the flag an in-page move raises for its turn's epilogue.
        let page_route = defs.iter().find_map(|d| match &d.kind {
            ComponentKind::Page { path, .. } => Some(*path),
            _ => None,
        });
        if let Some(pattern) = page_route {
            let owns = self.lower_lgx_owns(pattern);
            self.deferred_functions.push(owns);
        }
        if any_navigates(&defs) {
            let var = self.interner.intern(moved_binding());
            let value = self.te_bool(false);
            self.deferred_functions.push(Stmt::Let {
                var,
                ty: None,
                value,
                mutable: true,
            });
        }
        // ONE route entry for the whole page, registered with the host at start: whatever moves
        // the location, every live listener hears it through the same fan-out.
        if let Some((changed, listens)) = self.lower_lgx_route_changed(&lowered) {
            self.deferred_functions.push(changed);
            if listens {
                let fan_out = self.interner.intern(route_changed_fn_name());
                let call = Stmt::Call {
                    function: fan_out,
                    args: Vec::new(),
                };
                let closure = self.listener_closure(call);
                let on_route = self.interner.intern("onRoute");
                self.deferred_functions.push(Stmt::Call {
                    function: on_route,
                    args: vec![closure],
                });
            }
        }
        // The named root repaint a shared store's arrival runs (`lgx_lower::shared`).
        let shares = defs.iter().any(|d| {
            matches!(
                d.kind,
                ComponentKind::Store {
                    mode: crate::ast::component::StoreMode::Shared { .. }
                }
            )
        });
        if shares {
            if let Some(repaint) = self.lower_lgx_repaint_root(&lowered) {
                self.deferred_functions.push(repaint);
            }
        }
        if self.uses_scoped_sweep() {
            let descends = self.lower_lgx_descends();
            self.deferred_functions.push(descends);
        }
        if let Some(tick) = self.lower_lgx_tick() {
            self.deferred_functions.push(tick);
        }
        if let Some(on_failure) = self.lower_lgx_on_failure() {
            self.deferred_functions.push(on_failure);
        }
        // ★ THE DISPATCHER IS EMITTED WHENEVER THERE IS A PAGE, EVEN WITH NOTHING LISTENING.
        //
        // It used to exist only when some element had a handler, which was fine while only the
        // DRIVING VERBS called it. Now compiled page code calls it too — `Focus "#draft".` performs
        // the DOM command and then delivers `focus` to whatever waits for it — and a call to a
        // function that was not emitted is `Unknown function` at run time, in the browser, on a
        // page whose only crime was listening for nothing.
        //
        // An empty dispatcher answers `false` to everything, which is the truth: a driving verb
        // on an element nothing listens to reports "nothing answers" in the words every tier uses.
        // `Raise` stays gated on a listener existing — a page that attaches none has no window
        // event to deliver.
        if let Some(dispatcher) = self.lower_lgx_dispatcher(&defs) {
            self.deferred_functions.push(dispatcher);
            if !self.lgx_listeners.is_empty() {
                let raise = self.lower_lgx_raise();
                self.deferred_functions.push(raise);
            }
        }
        Ok(())
    }
}
