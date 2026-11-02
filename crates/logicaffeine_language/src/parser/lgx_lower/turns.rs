use super::*;
use crate::ast::component::{ComponentDef, Trigger};
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use logicaffeine_base::Symbol;

impl<'a, 'ctx, 'int> super::super::Parser<'a, 'ctx, 'int> {
    /// `__lgx_on_failure(message) -> Bool` — the one door a trap reaches: journal the turns it
    /// interrupted, then say whether the page absorbed it (`page_host::lowered::on_failure` reads
    /// exactly this: `true` only from a real absorption, never from a void return it could not tell
    /// apart from "nothing absorbed it").
    ///
    /// ★ A TRAP IN A WATCH PHASE IS ABSORBED. `__LGX_WATCHING` names the component whose changed-
    /// field handlers were running: its action had already committed, so the turns opened after
    /// the watching turn (store actions its handlers called) are journaled as interrupted, the
    /// watching turn as it committed, the page is redrawn from the state it left, and the door
    /// answers `true`: a watch phase's failure is dropped.
    ///
    /// ★ EVERY OTHER TRAP journals each open turn innermost-first (a store action interrupted
    /// inside a component's action before that action), then runs the failure handler of the
    /// component whose turn it was, when it declares `When an action fails:`. The state is already
    /// safe: a turn reads state into locals and writes back only at its END, so a trap never
    /// reached the write-back.
    pub(crate) fn lower_lgx_on_failure(&mut self) -> Option<Stmt<'a>> {
        let journals = self.lgx_all_components.iter().any(stateful);
        let watchers = watching_components(&self.lgx_all_components);
        let wakes = self.wakes_store_watchers();
        let stores = commits_stores(&self.lgx_all_components);
        let nests = self.declares_binding(render_depth_binding());
        let appears = self.declares_binding(appear_who_binding());
        if self.lgx_fail_handlers.is_empty() && !journals && !wakes && !stores && !nests && !appears
        {
            return None;
        }
        let msg = self.interner.intern("__lgx_msg");
        let stack = self.interner.intern(open_turns_binding());
        let mut body: Vec<Stmt<'a>> = Vec::new();
        // ★ A TRAP ABANDONS EVERY STORE-WATCHER ROUND IT INTERRUPTED, so the count starts over
        // here. What it does not abandon is a store the failing turn already called: a store
        // action commits when it returns, so its watchers still run — every way out of this door
        // ends with the round, as a failed turn's store changes are drained too.
        if wakes {
            let cascade = self.interner.intern(cascade_binding());
            body.push(Stmt::Set {
                target: cascade,
                value: self.te_int(0),
            });
        }
        // A trap mid-paint leaves every render it interrupted counted; the next paint starts at the
        // top.
        if nests {
            let depth = self.interner.intern(render_depth_binding());
            body.push(Stmt::Set {
                target: depth,
                value: self.te_int(0),
            });
        }
        if !watchers.is_empty() {
            let watching = self.interner.intern(watching_binding());
            let depth = self.interner.intern(watch_depth_binding());
            let watch_key = self.interner.intern(watch_key_binding());
            let mut absorb: Vec<Stmt<'a>> = Vec::new();
            let open = self.ctx.alloc_imperative_expr(Expr::Length {
                collection: self.te_id(stack),
            });
            let above = self.te_bin(BinaryOpKind::Gt, open, self.te_id(depth));
            let note = self.journal_top_open_turn(self.te_id(msg), None);
            absorb.push(Stmt::While {
                cond: above,
                body: self.alloc_block(note),
                decreasing: None,
            });
            let open = self.ctx.alloc_imperative_expr(Expr::Length {
                collection: self.te_id(stack),
            });
            let any = self.te_bin(BinaryOpKind::Gt, open, self.te_int(0));
            let blank = self.interner.intern("");
            let committed = self.te_text(blank);
            let watch_changes = self.interner.intern(watch_changes_binding());
            let note = self.journal_top_open_turn(committed, Some(self.te_id(watch_changes)));
            absorb.push(Stmt::If {
                cond: any,
                then_block: self.alloc_block(note),
                else_block: None,
            });
            let which = self.interner.intern("__lgx_w");
            absorb.push(Stmt::Let {
                var: which,
                ty: None,
                value: self.te_id(watching),
                mutable: false,
            });
            absorb.push(Stmt::Set {
                target: watching,
                value: self.te_int(0),
            });
            let root = self.lgx_paint_root.and_then(|r| {
                self.lgx_all_components
                    .iter()
                    .find(|d| d.name == r)
                    .cloned()
            });
            match root {
                Some(rd) => {
                    absorb.extend(self.stores_painted());
                    let outer = self.lgx_key_override.replace(self.interner.intern(""));
                    absorb.extend(self.repaint_def(&rd, false));
                    self.lgx_key_override = outer;
                }
                None => {
                    for (i, c) in watchers.iter().enumerate() {
                        let Some(cd) = self
                            .lgx_all_components
                            .iter()
                            .find(|d| d.name == *c)
                            .cloned()
                        else {
                            continue;
                        };
                        let outer = self.lgx_key_override.replace(watch_key);
                        let redraw = self.repaint_def(&cd, false);
                        self.lgx_key_override = outer;
                        let mine = self.te_bin(
                            BinaryOpKind::Eq,
                            self.te_id(which),
                            self.te_int(i as i64 + 1),
                        );
                        absorb.push(Stmt::If {
                            cond: mine,
                            then_block: self.alloc_block(redraw),
                            else_block: None,
                        });
                    }
                }
            }
            absorb.extend(self.door_exit());
            absorb.push(Stmt::Return {
                value: Some(self.te_bool(true)),
            });
            let in_watch = self.te_bin(BinaryOpKind::NotEq, self.te_id(watching), self.te_int(0));
            body.push(Stmt::If {
                cond: in_watch,
                then_block: self.alloc_block(absorb),
                else_block: None,
            });
        }
        // ★ A STORE THE FAILING TURN COMMITTED IS DRAWN HERE. A turn that moves a store repaints
        // from the page root once it has written back, and a trap never reaches that paint: every
        // `{<Store>'s <field>}` on the page would keep showing the store as it was before a commit
        // that did happen, until some later turn happened to repaint from the root.
        let root = self.lgx_paint_root.and_then(|r| {
            self.lgx_all_components
                .iter()
                .find(|d| d.name == r)
                .cloned()
        });
        if let (true, Some(rd)) = (commits_stores(&self.lgx_all_components), root) {
            let mut redraw = self.stores_painted();
            let outer = self.lgx_key_override.replace(self.interner.intern(""));
            redraw.extend(self.repaint_def(&rd, false));
            self.lgx_key_override = outer;
            let dirty = self.interner.intern(stores_dirty_binding());
            body.push(Stmt::If {
                cond: self.te_id(dirty),
                then_block: self.alloc_block(redraw),
                else_block: None,
            });
        }
        if journals {
            let open = self.ctx.alloc_imperative_expr(Expr::Length {
                collection: self.te_id(stack),
            });
            let any = self.te_bin(BinaryOpKind::Gt, open, self.te_int(0));
            let note = self.journal_top_open_turn(self.te_id(msg), None);
            body.push(Stmt::While {
                cond: any,
                body: self.alloc_block(note),
                decreasing: None,
            });
        }
        let which = self.interner.intern(failing_component_binding());
        let whose = self.interner.intern(failing_key_binding());
        for (i, (component, index)) in self.lgx_fail_handlers.clone().iter().enumerate() {
            let name = self.interner.resolve(*component).to_string();
            let act = self
                .interner
                .intern(&action_fn_name(&name, &format!("__e{index}")));
            let mine = self.te_bin(
                BinaryOpKind::Eq,
                self.te_id(which),
                self.te_int(i as i64 + 1),
            );
            let call = Stmt::Call {
                function: act,
                args: vec![self.te_id(whose), self.te_id(msg)],
            };
            let mut absorbed = vec![call];
            absorbed.extend(self.door_exit());
            absorbed.push(Stmt::Return {
                value: Some(self.te_bool(true)),
            });
            body.push(Stmt::If {
                cond: mine,
                then_block: self.alloc_block(absorbed),
                else_block: None,
            });
        }
        // No handler's index matched the failing component's: nothing here absorbed it.
        body.extend(self.door_exit());
        body.push(Stmt::Return {
            value: Some(self.te_bool(false)),
        });
        let text_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Text")));
        let bool_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Bool")));
        let name = self.interner.intern(on_failure_fn_name());
        Some(Stmt::FunctionDef {
            name,
            generics: Vec::new(),
            params: vec![(msg, text_ty)],
            body: self.alloc_block(body),
            return_type: Some(bool_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        })
    }

    /// Journal the innermost open turn and close it, with `failure` (`""`: it committed) and
    /// `changes` (none, for a turn a trap interrupted: its state was never written back).
    fn journal_top_open_turn(
        &mut self,
        failure: &'a Expr<'a>,
        changes: Option<&'a Expr<'a>>,
    ) -> Vec<Stmt<'a>> {
        let names = self.interner.intern(open_turns_binding());
        let keys = self.interner.intern(open_keys_binding());
        let (name, key) = (
            self.interner.intern("__lgx_ft"),
            self.interner.intern("__lgx_fk"),
        );
        let mut out = Vec::new();
        for (var, stack) in [(name, names), (key, keys)] {
            let last = self.ctx.alloc_imperative_expr(Expr::Length {
                collection: self.te_id(stack),
            });
            let read = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(stack),
                index: last,
            });
            out.push(Stmt::Let {
                var,
                ty: None,
                value: read,
                mutable: false,
            });
        }
        out.extend(self.close_turn());
        let text_name = self.interner.intern("Text");
        let changes = changes.unwrap_or_else(|| self.new_seq_expr(text_name));
        let note = self.interner.intern("noteTurn");
        out.push(Stmt::Call {
            function: note,
            args: vec![self.te_id(name), self.te_id(key), changes, failure],
        });
        out
    }

    /// One action, lowered to a function.
    ///
    /// ★ ROLLBACK BY CONSTRUCTION. The state is read out of its module bindings into LOCALS at
    /// the top, the body runs against those locals, and they are written back only at the very
    /// END. A trap partway through never reaches the write-back, so the committed state is
    /// exactly what the turn started from — the guarantee an explicit snapshot would give,
    /// without needing to catch anything. It also means the body needs no
    /// rewriting: it says `Set n to n + 1` and `n` is the local.
    pub(crate) fn lower_component_action(
        &mut self,
        def: &ComponentDef<'a>,
        action: &crate::ast::component::ActionDef<'a>,
    ) -> Option<Stmt<'a>> {
        let component = self.interner.resolve(def.name).to_string();
        let name = self.interner.intern(&action_fn_name(
            &component,
            self.interner.resolve(action.name),
        ));
        let action_name = self.interner.resolve(action.name).to_string();
        let body = self.turn_body(def, action.body, &action_name)?;
        // ★ WHICH INSTANCE THIS TURN RUNS AGAINST, first — before the action's own parameters. The
        // listener that fires was built inside a render and captured that render's key, so the
        // right instance arrives without anything having to look it up.
        let mut params = vec![(self.interner.intern(instance_key_param()), self.text_type())];
        params.extend(action.params.iter().copied());
        Some(Stmt::FunctionDef {
            name,
            generics: Vec::new(),
            params,
            body,
            return_type: None,
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        })
    }

    /// A `When …:` sentence, lowered the same way an action is — it is a turn like any other.
    pub(crate) fn lower_component_event(
        &mut self,
        def: &ComponentDef<'a>,
        handler: &crate::ast::component::EventHandler<'a>,
        index: usize,
    ) -> Option<Stmt<'a>> {
        let component = self.interner.resolve(def.name).to_string();
        let name = self
            .interner
            .intern(&action_fn_name(&component, &format!("__e{index}")));
        let paints = !matches!(handler.trigger, Trigger::Disappears);
        // ★ WHAT JOURNALS, PER TRIGGER — an EXHAUSTIVE match, not a guess from `paints`: the two
        // questions are independent (`PageLoads` paints nothing and is not a turn either,
        // `FieldChanges` inlines into ITS OWN caller's turn and is not a turn of its own, and both
        // facts happen to coincide only for `Disappears`). The journal holds exactly three shapes —
        // an action, a bind write, and a REAL listener delivery ("event") — and every other handler
        // here (`PageLoads`, `Disappears`, both `FieldChanges`, `ActionFails`, `FetchFails`) runs
        // with no journal entry at all: a lifecycle hook or a reactive observer, not something a
        // visitor did. `Appears` IS a listener delivery — it fires as a real `When …:` sentence
        // does, once its own markup has committed — so it journals as `"event"` too. A new
        // `Trigger` variant must say which it is before this compiles.
        let journal_name: Option<&str> = match handler.trigger {
            Trigger::PageLoads
            | Trigger::Disappears
            | Trigger::FieldChanges { .. }
            | Trigger::ActionFails { .. }
            | Trigger::FetchFails { .. } => None,
            Trigger::Appears
            | Trigger::RouteChanges
            | Trigger::Online
            | Trigger::Offline
            | Trigger::Selector { .. }
            | Trigger::Key { .. }
            | Trigger::Every { .. }
            | Trigger::After { .. }
            | Trigger::Message { .. }
            | Trigger::WindowResizes
            | Trigger::ScrollsToBottom { .. }
            | Trigger::FileDropped { .. } => Some("event"),
        };
        let mut body = self.turn_body_with(def, handler.body, paints, journal_name)?;
        // A trigger that BINDS NAMES takes them as parameters — the caller is the only thing that
        // knows the values, exactly as an `on-click={act(x)}` handler's arguments are.
        let mut params = vec![(self.interner.intern(instance_key_param()), self.text_type())];
        if let Trigger::FetchFails { url, error } = handler.trigger {
            let text = self.text_type();
            params.push((url, text));
            params.push((error, text));
        }
        if let Trigger::ActionFails { param } = handler.trigger {
            let text = self.text_type();
            params.push((param, text));
        }
        // G25: the DECLARED type decides what binds — `(file: File)` a real record (`name: Text`,
        // `size: Int`), `(name: Text)` the file's bare carried name unwrapped. The
        // parser refuses any other spelling at parse time (`parser/component.rs`), so only these
        // two ever reach here.
        if let Trigger::FileDropped {
            param: (name, declared_ty),
            ..
        } = handler.trigger
        {
            params.push((name, declared_ty));
            if matches!(
                declared_ty,
                TypeExpr::Named(n) | TypeExpr::Primitive(n) if self.interner.resolve(*n) == "File"
            ) {
                let file_ty = self.file_record_type();
                body = self.wrap_carried_file(name, file_ty, body);
            }
        }
        // ★ THE PUBLISHED PAYLOAD IS A PARAMETER TOO. Without this the lowered handler read a name
        // nothing declared and the page failed at run time with "Undefined variable: m" — a name
        // the AUTHOR wrote, reported as if they had misspelled it. The payload is in scope before
        // the body runs; a handler is a FUNCTION, so "in scope" means a parameter, and the
        // dispatcher passes the value it carried.
        if let Trigger::Message { param, .. } = handler.trigger {
            let text = self.text_type();
            params.push((param.0, text));
        }
        Some(Stmt::FunctionDef {
            name,
            generics: Vec::new(),
            params,
            body,
            return_type: None,
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        })
    }

    /// The `File` type expression — `name: Text`, `size: Int`, the built-in shape `TypeRegistry::
    /// with_primitives` already registers for a file-typed trigger's own bound parameter.
    fn file_record_type(&mut self) -> &'a TypeExpr<'a> {
        let file = self.interner.intern("File");
        self.ctx.alloc_type_expr(TypeExpr::Named(file))
    }

    /// The ONE place a file-typed trigger's bound parameter gets its real value — `param` arrives
    /// as whatever bare payload the dispatcher's own generic event-delivery mechanism carries (a
    /// name, on every trigger this compiles today), but the parameter's OWN declared type
    /// ([`Self::file_record_type`], already pushed onto `params` by the caller) is `File`, so a
    /// body that reads `param's name`/`param's size` compiles against a real record. Reconciles
    /// the two by prepending one synthesized statement — `Let param be a new File with name
    /// param and size eventFileSize(0) and type eventFileType(0) and bytes eventFileBytes(0).` —
    /// that shadows the raw parameter with the wrapped record before the author's own body ever
    /// runs. S17: NONE of `size`/`type`/`bytes` is ever a literal baked in here —
    /// `eventFileSize`/`eventFileType`/`eventFileBytes` each answer the REAL event on a browser
    /// (`dom_event_file_size`/`dom_event_file_type`/`dom_event_file_bytes`) and whatever the
    /// driving test statement declared on an in-process tier
    /// (`semantics::dom::set_dropped_file`/`dropped_file`).
    ///
    /// Every file-typed trigger this compiles calls through here — today that is only
    /// `Trigger::FileDropped`, but a future one (a file INPUT's own "chosen" event has no compiled
    /// trigger of its own yet; `ast/view.rs`'s `BoundInput::File` refusal only ever POINTS an author
    /// at a `When chosen:` handler as advice, not at an existing keyword this file's parser can
    /// lower) shares this same wrap rather than growing its own copy.
    fn wrap_carried_file(
        &mut self,
        param: Symbol,
        file_ty: &'a TypeExpr<'a>,
        body: crate::ast::stmt::Block<'a>,
    ) -> crate::ast::stmt::Block<'a> {
        let file = self.interner.intern("File");
        let name_field = self.interner.intern("name");
        let size_field = self.interner.intern("size");
        let type_field = self.interner.intern("type");
        let bytes_field = self.interner.intern("bytes");
        let carried = self.ctx.alloc_imperative_expr(Expr::Identifier(param));
        let event_size = self.interner.intern("eventFileSize");
        let event_type = self.interner.intern("eventFileType");
        let placeholder_event = self
            .ctx
            .alloc_imperative_expr(Expr::Literal(Literal::Number(0)));
        let size = self.ctx.alloc_imperative_expr(Expr::Call {
            function: event_size,
            args: vec![placeholder_event],
        });
        let placeholder_event = self
            .ctx
            .alloc_imperative_expr(Expr::Literal(Literal::Number(0)));
        let file_type = self.ctx.alloc_imperative_expr(Expr::Call {
            function: event_type,
            args: vec![placeholder_event],
        });
        let event_bytes = self.interner.intern("eventFileBytes");
        let placeholder_event = self
            .ctx
            .alloc_imperative_expr(Expr::Literal(Literal::Number(0)));
        let bytes = self.ctx.alloc_imperative_expr(Expr::Call {
            function: event_bytes,
            args: vec![placeholder_event],
        });
        let wrapped = self.ctx.alloc_imperative_expr(Expr::New {
            type_name: file,
            type_args: Vec::new(),
            init_fields: vec![
                (name_field, carried),
                (size_field, size),
                (type_field, file_type),
                (bytes_field, bytes),
            ],
        });
        let rebind = Stmt::Let {
            var: param,
            ty: Some(file_ty),
            value: wrapped,
            mutable: false,
        };
        let mut wrapped_body = Vec::with_capacity(body.len() + 1);
        wrapped_body.push(rebind);
        wrapped_body.extend_from_slice(body);
        self.alloc_block(wrapped_body)
    }

    /// How a turn of `def` redraws what it committed: from the page root when it moved a store (a
    /// store's fields belong to the store, so `{Counter's hits}` is no site of `def`'s and any
    /// component may show it); otherwise a patch of the sites `writes` reaches, or a repaint when
    /// the patch plan
    /// cannot say. The patch runs after the write-back, so its site expressions read the values
    /// the turn just committed.
    fn turn_paint(
        &mut self,
        def: &ComponentDef<'a>,
        moved_a_store: bool,
        writes: &[Symbol],
    ) -> Vec<Stmt<'a>> {
        if moved_a_store {
            let mut out = self.stores_painted();
            let root = self.lgx_paint_root.filter(|r| *r != def.name);
            match root.and_then(|r| {
                self.lgx_all_components
                    .iter()
                    .find(|d| d.name == r)
                    .cloned()
            }) {
                Some(rd) => {
                    let outer = self.lgx_key_override.replace(self.interner.intern(""));
                    out.extend(self.repaint_def(&rd, false));
                    self.lgx_key_override = outer;
                }
                None => out.extend(self.repaint_def(def, false)),
            }
            return out;
        }
        match self.patch_statements(def, writes) {
            Some(patch) => patch,
            None => self.repaint_def(def, false),
        }
    }

    /// `__lgx_appear()` — every owed `When this component appears:`, first drawn first; `None` when
    /// no component in `appearing` has one.
    ///
    /// ★ THE CURSOR IS THE PAGE'S, NOT THE CALL'S. An appears turn repaints, and its paint ends by
    /// calling this again: that inner call carries on through the same queue — including what its
    /// own paint just owed — and the outer one finds nothing left. A trap leaves the cursor past
    /// the entry that trapped, so the failure door's own call runs the rest.
    pub(crate) fn lower_lgx_appear(&mut self, appearing: &[Symbol]) -> Option<Stmt<'a>> {
        if appearing.is_empty() {
            return None;
        }
        let who = self.interner.intern(appear_who_binding());
        let keys = self.interner.intern(appear_key_binding());
        let at = self.interner.intern(appear_at_binding());
        let (w, k) = (
            self.interner.intern("__lgx_aw"),
            self.interner.intern("__lgx_ak"),
        );
        let mut step: Vec<Stmt<'a>> = Vec::new();
        for (var, column) in [(w, who), (k, keys)] {
            let read = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(column),
                index: self.te_id(at),
            });
            step.push(Stmt::Let {
                var,
                ty: None,
                value: read,
                mutable: false,
            });
        }
        let next = self.te_bin(BinaryOpKind::Add, self.te_id(at), self.te_int(1));
        step.push(Stmt::Set {
            target: at,
            value: next,
        });
        for (i, component) in appearing.iter().enumerate() {
            let Some(def) = self
                .lgx_all_components
                .iter()
                .find(|d| d.name == *component)
                .cloned()
            else {
                continue;
            };
            let name = self.interner.resolve(*component).to_string();
            let calls: Vec<Stmt<'a>> = def
                .events
                .iter()
                .enumerate()
                .filter(|(_, h)| matches!(h.trigger, Trigger::Appears))
                .map(|(index, _)| Stmt::Call {
                    function: self
                        .interner
                        .intern(&action_fn_name(&name, &format!("__e{index}"))),
                    args: vec![self.te_id(k)],
                })
                .collect();
            let mine = self.te_bin(BinaryOpKind::Eq, self.te_id(w), self.te_int(i as i64 + 1));
            step.push(Stmt::If {
                cond: mine,
                then_block: self.alloc_block(calls),
                else_block: None,
            });
        }
        let owed = self.ctx.alloc_imperative_expr(Expr::Length {
            collection: self.te_id(who),
        });
        let more = self.te_bin(BinaryOpKind::LtEq, self.te_id(at), owed);
        let int_name = self.interner.intern("Int");
        let text_name = self.interner.intern("Text");
        let body = vec![
            Stmt::While {
                cond: more,
                body: self.alloc_block(step),
                decreasing: None,
            },
            Stmt::Set {
                target: who,
                value: self.new_seq_expr(int_name),
            },
            Stmt::Set {
                target: keys,
                value: self.new_seq_expr(text_name),
            },
            Stmt::Set {
                target: at,
                value: self.te_int(1),
            },
        ];
        Some(Stmt::FunctionDef {
            name: self.interner.intern(appear_fn_name()),
            generics: Vec::new(),
            params: Vec::new(),
            body: self.alloc_block(body),
            return_type: None,
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        })
    }

    /// The call that ends every paint: the appears handlers it owes, run now that it is drawn.
    pub(crate) fn appear_call(&mut self) -> Vec<Stmt<'a>> {
        if !self.declares_binding(appear_who_binding()) {
            return Vec::new();
        }
        vec![Stmt::Call {
            function: self.interner.intern(appear_fn_name()),
            args: Vec::new(),
        }]
    }

    /// How every way out of the failure door ends: what the failing turn's paint still owed to the
    /// instances it drew, then the watchers of the stores it moved.
    fn door_exit(&mut self) -> Vec<Stmt<'a>> {
        let mut out = self.appear_call();
        out.extend(self.wake_stores_call());
        out
    }

    /// What a repaint from the page root says before it draws: every store it draws is current.
    fn stores_painted(&mut self) -> Vec<Stmt<'a>> {
        if !commits_stores(&self.lgx_all_components) {
            return Vec::new();
        }
        let dirty = self.interner.intern(stores_dirty_binding());
        vec![Stmt::Set {
            target: dirty,
            value: self.te_bool(false),
        }]
    }

    /// `__lgx_owns(path) -> Bool` — whether the page route `pattern` serves `path`, its query and
    /// fragment set aside. A route with no `:name` segment is compared byte by byte: `path` begins
    /// with the route and ends there or at a `?` or `#`. One with a `:name` segment cuts the path at
    /// its fragment and query and asks `matchRoute`, which the loader already provides (with
    /// `split`) for such a page.
    pub(crate) fn lower_lgx_owns(&mut self, pattern: Symbol) -> Stmt<'a> {
        let path = self.interner.intern("__lgx_op");
        let body = if self.interner.resolve(pattern).contains("/:") {
            let split = self.interner.intern("split");
            let cut_at = |me: &mut Self, text: &'a Expr<'a>, sep: &str| -> &'a Expr<'a> {
                let sep = me.interner.intern(sep);
                let parts = me.te_call(split, vec![text, me.te_text(sep)]);
                me.ctx.alloc_imperative_expr(Expr::Index {
                    collection: parts,
                    index: me.te_int(1),
                })
            };
            let without_fragment = cut_at(self, self.te_id(path), "#");
            let bare = cut_at(self, without_fragment, "?");
            let match_route = self.interner.intern("matchRoute");
            vec![Stmt::Return {
                value: Some(self.te_call(match_route, vec![self.te_text(pattern), bare])),
            }]
        } else {
            let (pb, tb, n, i) = (
                self.interner.intern("__lgx_opb"),
                self.interner.intern("__lgx_otb"),
                self.interner.intern("__lgx_on"),
                self.interner.intern("__lgx_oi"),
            );
            let text_bytes = self.interner.intern("text_bytes");
            let len = |me: &mut Self, v: Symbol| {
                me.ctx.alloc_imperative_expr(Expr::Length {
                    collection: me.te_id(v),
                })
            };
            let no = |me: &mut Self| {
                me.alloc_block(vec![Stmt::Return {
                    value: Some(me.te_bool(false)),
                }])
            };
            let mut body = vec![
                Stmt::Let {
                    var: pb,
                    ty: None,
                    value: self.te_call(text_bytes, vec![self.te_id(path)]),
                    mutable: false,
                },
                Stmt::Let {
                    var: tb,
                    ty: None,
                    value: self.te_call(text_bytes, vec![self.te_text(pattern)]),
                    mutable: false,
                },
            ];
            let tn = len(self, tb);
            body.push(Stmt::Let {
                var: n,
                ty: None,
                value: tn,
                mutable: false,
            });
            let pn = len(self, pb);
            let short = self.te_bin(BinaryOpKind::Lt, pn, self.te_id(n));
            let refuse = no(self);
            body.push(Stmt::If {
                cond: short,
                then_block: refuse,
                else_block: None,
            });
            body.push(Stmt::Let {
                var: i,
                ty: None,
                value: self.te_int(1),
                mutable: true,
            });
            let at_p = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(pb),
                index: self.te_id(i),
            });
            let at_t = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(tb),
                index: self.te_id(i),
            });
            let differs = self.te_bin(BinaryOpKind::NotEq, at_p, at_t);
            let refuse = no(self);
            let step = self.te_bin(BinaryOpKind::Add, self.te_id(i), self.te_int(1));
            let more = self.te_bin(BinaryOpKind::LtEq, self.te_id(i), self.te_id(n));
            body.push(Stmt::While {
                cond: more,
                body: self.alloc_block(vec![
                    Stmt::If {
                        cond: differs,
                        then_block: refuse,
                        else_block: None,
                    },
                    Stmt::Set {
                        target: i,
                        value: step,
                    },
                ]),
                decreasing: None,
            });
            let pn = len(self, pb);
            let exact = self.te_bin(BinaryOpKind::Eq, pn, self.te_id(n));
            body.push(Stmt::If {
                cond: exact,
                then_block: self.alloc_block(vec![Stmt::Return {
                    value: Some(self.te_bool(true)),
                }]),
                else_block: None,
            });
            let next = self.te_bin(BinaryOpKind::Add, self.te_id(n), self.te_int(1));
            let after = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(pb),
                index: next,
            });
            let query = self.te_bin(BinaryOpKind::Eq, after, self.te_int(i64::from(b'?')));
            let after = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(pb),
                index: next,
            });
            let fragment = self.te_bin(BinaryOpKind::Eq, after, self.te_int(i64::from(b'#')));
            body.push(Stmt::Return {
                value: Some(self.te_bin(BinaryOpKind::Or, query, fragment)),
            });
            body
        };
        let text_ty = self.text_type();
        let bool_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Bool")));
        Stmt::FunctionDef {
            name: self.interner.intern(owns_fn_name()),
            generics: Vec::new(),
            params: vec![(path, text_ty)],
            body: self.alloc_block(body),
            return_type: Some(bool_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        }
    }

    /// `__lgx_repaint_root()` — the page repainted from its root, keyed `""`; `None` when the root
    /// is not among the `lowered` components.
    pub(crate) fn lower_lgx_repaint_root(&mut self, lowered: &[Symbol]) -> Option<Stmt<'a>> {
        let root = self
            .lgx_paint_root
            .filter(|r| lowered.contains(r))
            .and_then(|r| {
                self.lgx_all_components
                    .iter()
                    .find(|d| d.name == r)
                    .cloned()
            })?;
        let outer = self.lgx_key_override.replace(self.interner.intern(""));
        let body = self.repaint_def(&root, false);
        self.lgx_key_override = outer;
        Some(Stmt::FunctionDef {
            name: self.interner.intern(repaint_root_fn_name()),
            generics: Vec::new(),
            params: Vec::new(),
            body: self.alloc_block(body),
            return_type: None,
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        })
    }

    /// `__lgx_route_changed()` — every live instance of every component in `lowered` with a
    /// `When the route changes:` handler, run in declaration order: the one fan-out both a host's
    /// move of the location and a turn's own `Navigate to` reach. Emitted when something listens or
    /// something navigates (a page that navigates with no listener gets an empty one, which its
    /// turns' epilogues still call); `None` otherwise. The flag says whether anything listens.
    ///
    /// LIVE is "on the page": `__KEYS_<C>` holds exactly the instances no sweep has taken (a sweep
    /// removes every instance its repaint no longer draws), so the walk skips only the seed key —
    /// never an instance that simply was not in the most recent paint.
    pub(crate) fn lower_lgx_route_changed(
        &mut self,
        lowered: &[Symbol],
    ) -> Option<(Stmt<'a>, bool)> {
        let mut body: Vec<Stmt<'a>> = Vec::new();
        for d in self.lgx_all_components.clone() {
            if !lowered.contains(&d.name) || !keeps_instance_registry(&d) {
                continue;
            }
            for (index, handler) in d.events.iter().enumerate() {
                if !matches!(handler.trigger, Trigger::RouteChanges) {
                    continue;
                }
                let name = self.interner.resolve(d.name).to_string();
                let keys = self.interner.intern(&keys_binding(&name));
                let seen = self.interner.intern(&seen_binding(&name));
                let k = self.interner.intern("__lgx_rk");
                let stamp = self.ctx.alloc_imperative_expr(Expr::Index {
                    collection: self.te_id(seen),
                    index: self.te_id(k),
                });
                let live = self.te_bin(BinaryOpKind::NotEq, stamp, self.te_int(-1));
                let act = self
                    .interner
                    .intern(&action_fn_name(&name, &format!("__e{index}")));
                let call = Stmt::Call {
                    function: act,
                    args: vec![self.te_id(k)],
                };
                let then_block = self.alloc_block(vec![call]);
                let inner = self.alloc_block(vec![Stmt::If {
                    cond: live,
                    then_block,
                    else_block: None,
                }]);
                body.push(Stmt::Repeat {
                    pattern: crate::ast::stmt::Pattern::Identifier(k),
                    iterable: self.te_id(keys),
                    body: inner,
                });
            }
        }
        let listens = !body.is_empty();
        if !listens && !any_navigates(&self.lgx_all_components) {
            return None;
        }
        Some((
            Stmt::FunctionDef {
                name: self.interner.intern(route_changed_fn_name()),
                generics: Vec::new(),
                params: Vec::new(),
                body: self.alloc_block(body),
                return_type: None,
                is_native: false,
                native_path: None,
                is_exported: false,
                export_target: None,
                opt_flags: crate::optimization::OptimizationConfig::all_on(),
            },
            listens,
        ))
    }

    /// A timer's period in milliseconds, as the page reads it when the instance is drawn: a literal
    /// (`60 seconds`, `500 milliseconds`) folded to its number, anything else a count of seconds
    /// computed from what is in scope there — state, derived fields and props.
    pub(crate) fn timer_period_ms(&self, seconds: &'a Expr<'a>) -> &'a Expr<'a> {
        match seconds {
            Expr::Literal(Literal::Number(n)) => self.te_int(n.saturating_mul(1000)),
            Expr::Literal(Literal::Duration(ns)) => self.te_int(ns / 1_000_000),
            _ => self.te_bin(BinaryOpKind::Multiply, seconds, self.te_int(1000)),
        }
    }

    /// The handler index of `component`'s timer named `name` — `None` for a name it declares no
    /// timer under, which the parse has already refused by name.
    pub(crate) fn own_timer(&self, component: &str, name: Symbol) -> Option<usize> {
        self.lgx_timers
            .iter()
            .find(|(c, _, _, n)| self.interner.resolve(*c) == component && *n == Some(name))
            .map(|(_, i, ..)| *i)
    }

    /// The first render of an instance arms each of its timers ([`Self::arm_timer`]); a later
    /// render finds the instance armed and arms nothing.
    pub(crate) fn arm_timers(&mut self, def: &ComponentDef<'a>) -> Vec<Stmt<'a>> {
        let component = self.interner.resolve(def.name).to_string();
        let mut out = Vec::new();
        for (_, i, _, _) in self
            .lgx_timers
            .clone()
            .iter()
            .filter(|(c, ..)| *c == def.name)
        {
            let due = self.interner.intern(&due_binding(&component, *i));
            let key = self.key_expr();
            let known = self.ctx.alloc_imperative_expr(Expr::Contains {
                collection: self.te_id(due),
                value: key,
            });
            let unknown = self.ctx.alloc_imperative_expr(Expr::Not { operand: known });
            let arm = self.arm_timer(def, *i);
            out.push(Stmt::If {
                cond: unknown,
                then_block: self.alloc_block(arm),
                else_block: None,
            });
        }
        out
    }

    /// Arm handler `index` of `def` on this instance, from now: its period as its sentence reads it
    /// here (`__PERIOD_`, refused when not positive, in every tier's words), its due time a
    /// period on from the page's clock (`__DUE_`), its place in arming order (`__SEQ_`); then the
    /// host is asked to wake the page for the earliest timer (`__lgx_rearm`). What an instance's
    /// first render does, and what `Start the timer …` does to a running or stopped timer.
    pub(crate) fn arm_timer(&mut self, def: &ComponentDef<'a>, index: usize) -> Vec<Stmt<'a>> {
        let seconds = match &def.events[index].trigger {
            Trigger::Every { seconds, .. } | Trigger::After { seconds, .. } => *seconds,
            _ => return Vec::new(),
        };
        let component = self.interner.resolve(def.name).to_string();
        let now = self.interner.intern(now_binding());
        let count = self.interner.intern(timer_seq_binding());
        let rearm = self.interner.intern(rearm_fn_name());
        let due = self.interner.intern(&due_binding(&component, index));
        let seq = self.interner.intern(&seq_binding(&component, index));
        let period = self.interner.intern(&period_binding(&component, index));
        let ms = self.interner.intern(&format!("__lgx_period{index}"));
        let not_positive = self.te_bin(BinaryOpKind::LtEq, self.te_id(ms), self.te_int(0));
        let refusal = self
            .interner
            .intern(crate::ast::component::timer_period_not_positive());
        let due_at = self.te_bin(BinaryOpKind::Add, self.te_id(now), self.te_id(ms));
        let next = self.te_bin(BinaryOpKind::Add, self.te_id(count), self.te_int(1));
        vec![
            Stmt::Let {
                var: ms,
                ty: None,
                value: self.timer_period_ms(seconds),
                mutable: false,
            },
            Stmt::If {
                cond: not_positive,
                then_block: self.alloc_block(vec![Stmt::Fail {
                    reason: self.te_text(refusal),
                }]),
                else_block: None,
            },
            Stmt::SetIndex {
                collection: self.te_id(period),
                index: self.key_expr(),
                value: self.te_id(ms),
            },
            Stmt::SetIndex {
                collection: self.te_id(due),
                index: self.key_expr(),
                value: due_at,
            },
            Stmt::Set {
                target: count,
                value: next,
            },
            Stmt::SetIndex {
                collection: self.te_id(seq),
                index: self.key_expr(),
                value: self.te_id(count),
            },
            Stmt::Call {
                function: rearm,
                args: Vec::new(),
            },
        ]
    }

    /// `Push "<name>" to __LGX_OPEN_TURNS. Push <instance> to __LGX_OPEN_KEYS.` — a turn is open
    /// from its first statement.
    pub(crate) fn open_turn(&mut self, name: &str, instance: &'a Expr<'a>) -> Vec<Stmt<'a>> {
        let names = self.interner.intern(open_turns_binding());
        let keys = self.interner.intern(open_keys_binding());
        let name = self.interner.intern(name);
        vec![
            Stmt::Push {
                value: self.te_text(name),
                collection: self.te_id(names),
            },
            Stmt::Push {
                value: instance,
                collection: self.te_id(keys),
            },
        ]
    }

    /// Pop both open-turn stacks — the turn is about to journal itself, so it is no longer open.
    pub(crate) fn close_turn(&mut self) -> Vec<Stmt<'a>> {
        let names = self.interner.intern(open_turns_binding());
        let keys = self.interner.intern(open_keys_binding());
        vec![
            Stmt::Pop {
                collection: self.te_id(names),
                into: None,
            },
            Stmt::Pop {
                collection: self.te_id(keys),
                into: None,
            },
        ]
    }

    /// The journal's name for the instance a turn of `def` runs against: a store's `/store/<Name>`,
    /// a component's `<key>/<Name>` — `/<Name>` for a mounted root, the placement's key before the
    /// name for a placed instance — an instance's path ends in its component.
    pub(crate) fn journal_instance(&mut self, def: &ComponentDef<'a>) -> &'a Expr<'a> {
        let name = self.interner.resolve(def.name).to_string();
        if matches!(def.kind, crate::ast::component::ComponentKind::Store { .. }) {
            let path = self.interner.intern(&format!("/store/{name}"));
            return self.te_text(path);
        }
        let tail = self.interner.intern(&format!("/{name}"));
        let key = self.key_expr();
        self.te_bin(BinaryOpKind::Add, key, self.te_text(tail))
    }

    /// `journalDetail()` — whether the host keeps the journal's detail. Every display the journal
    /// captures is built under it, so a host that keeps none (a shipped page) builds none.
    fn journal_detail(&mut self) -> &'a Expr<'a> {
        let f = self.interner.intern("journalDetail");
        self.ctx.alloc_imperative_expr(Expr::Call {
            function: f,
            args: Vec::new(),
        })
    }

    /// `Let mutable <seq> be a new Seq of Text.`, then, when the host keeps the journal's detail,
    /// `Push "{field}" to <seq>.` for every field of `def`, in declaration order — the
    /// display each showed before the statements that follow. `<seq>` is also the `changes` the
    /// turn hands `noteTurn`: [`Self::journal_changes`] replaces the befores with the triples, and a
    /// host that keeps no detail is handed it empty.
    pub(crate) fn journal_before(
        &mut self,
        def: &ComponentDef<'a>,
        seq: &str,
    ) -> (Symbol, Vec<Stmt<'a>>) {
        let seq = self.interner.intern(seq);
        let text_name = self.interner.intern("Text");
        let mut out = vec![Stmt::Let {
            var: seq,
            ty: None,
            value: self.new_seq_expr(text_name),
            mutable: true,
        }];
        let fields = def.state.clone();
        if !fields.is_empty() {
            let captures: Vec<Stmt<'a>> = fields
                .iter()
                .map(|f| Stmt::Push {
                    value: self.as_text(self.te_id(f.name)),
                    collection: self.te_id(seq),
                })
                .collect();
            out.push(Stmt::If {
                cond: self.journal_detail(),
                then_block: self.alloc_block(captures),
                else_block: None,
            });
        }
        (seq, out)
    }

    /// Under `journalDetail()`: the turn's changes as `noteTurn` takes them — `[field, before, after,
    /// …]` for every field whose display moved since [`Self::journal_before`] filled
    /// `before`, in declaration order — built into a fresh local named `into`, then `Set` into
    /// `target`. A derived field's after is recomputed from the plain fields the turn holds now.
    pub(crate) fn journal_changes(
        &mut self,
        def: &ComponentDef<'a>,
        before: Symbol,
        into: &str,
        target: Symbol,
    ) -> Vec<Stmt<'a>> {
        let fields = def.state.clone();
        if fields.is_empty() {
            return Vec::new();
        }
        let list = self.interner.intern(into);
        let text_name = self.interner.intern("Text");
        let mut built = vec![Stmt::Let {
            var: list,
            ty: None,
            value: self.new_seq_expr(text_name),
            mutable: true,
        }];
        for (i, f) in fields.into_iter().enumerate() {
            let field = self.interner.resolve(f.name).to_string();
            let was = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(before),
                index: self.te_int(i as i64 + 1),
            });
            let now = if f.derived {
                f.init
            } else {
                self.te_id(f.name)
            };
            let after = self.interner.intern(&format!("{into}_{field}"));
            let shown = self.as_text(now);
            built.push(Stmt::Let {
                var: after,
                ty: None,
                value: shown,
                mutable: false,
            });
            let moved = self.te_bin(BinaryOpKind::NotEq, self.te_id(after), was);
            let name = self.interner.intern(&field);
            let push = |me: &mut Self, value: &'a Expr<'a>| Stmt::Push {
                value,
                collection: me.te_id(list),
            };
            let triple = vec![
                push(self, self.te_text(name)),
                push(self, was),
                push(self, self.te_id(after)),
            ];
            built.push(Stmt::If {
                cond: moved,
                then_block: self.alloc_block(triple),
                else_block: None,
            });
        }
        built.push(Stmt::Set {
            target,
            value: self.te_id(list),
        });
        vec![Stmt::If {
            cond: self.journal_detail(),
            then_block: self.alloc_block(built),
            else_block: None,
        }]
    }

    /// The watch phase of a turn of `def`: its changed-field handlers, each under the flag its
    /// field's change set, bracketed by `__LGX_WATCHING` so a trap among them is absorbed
    /// (`lower_lgx_on_failure`).
    fn watch_phase(
        &mut self,
        def: &ComponentDef<'a>,
        handlers: Vec<(Symbol, crate::ast::stmt::Block<'a>)>,
    ) -> Vec<Stmt<'a>> {
        let id = watching_components(&self.lgx_all_components)
            .iter()
            .position(|c| *c == def.name)
            .map_or(0, |i| i as i64 + 1);
        let watching = self.interner.intern(watching_binding());
        let watch_key = self.interner.intern(watch_key_binding());
        let watch_depth = self.interner.intern(watch_depth_binding());
        let stack = self.interner.intern(open_turns_binding());
        let depth = self.ctx.alloc_imperative_expr(Expr::Length {
            collection: self.te_id(stack),
        });
        let mut out = vec![
            Stmt::Set {
                target: watching,
                value: self.te_int(id),
            },
            Stmt::Set {
                target: watch_key,
                value: self.key_expr(),
            },
            Stmt::Set {
                target: watch_depth,
                value: depth,
            },
        ];
        for (flag, block) in handlers {
            out.push(Stmt::If {
                cond: self.te_id(flag),
                then_block: block,
                else_block: None,
            });
        }
        out.push(Stmt::Set {
            target: watching,
            value: self.te_int(0),
        });
        out
    }

    /// `noteTurn(name, instance, changes, failure)` — one whole journal entry: the turn's name, the
    /// instance it ran against, its `[field, before, after, …]` changes, and the failure that
    /// interrupted it (`""` for a turn that committed). A statement: nothing reads its answer.
    pub(crate) fn note_turn(
        &mut self,
        name: &str,
        instance: &'a Expr<'a>,
        changes: &'a Expr<'a>,
        failure: &'a Expr<'a>,
    ) -> Stmt<'a> {
        let f = self.interner.intern("noteTurn");
        let text = self.interner.intern(name);
        Stmt::Call {
            function: f,
            args: vec![self.te_text(text), instance, changes, failure],
        }
    }

    /// Whether `stmts` contains a `Return` an author could actually write and reach — walking into
    /// every construct a real action/event body can nest one inside (`If`, `While`, `Repeat`,
    /// `Inspect`). Gates [`Self::protect_early_return`]: a turn with no `Return` anywhere pays
    /// nothing for a wrap it does not need — `bundle_size_lock` polices exactly that kind of
    /// per-component overhead a rare feature should not tax every page for.
    pub(crate) fn body_contains_return(stmts: &[Stmt<'a>]) -> bool {
        stmts.iter().any(|s| match s {
            Stmt::Return { .. } => true,
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                Self::body_contains_return(then_block)
                    || else_block.is_some_and(|b| Self::body_contains_return(b))
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                Self::body_contains_return(body)
            }
            Stmt::Inspect { arms, .. } => arms.iter().any(|a| Self::body_contains_return(a.body)),
            _ => false,
        })
    }

    /// ★ AN EARLY `Return` MUST STILL REACH THE TURN'S OWN EPILOGUE. `noteTurn` and the state
    /// write-back both sit AFTER a turn's own body, in the SAME compiled function
    /// (`turn_body_with`, the store-action loop) — a `Return` lowered as a native function return
    /// leaves that function immediately, skipping BOTH: `Set n to n + 1. If cond: Return.` left `n`
    /// at its turn-START value and never journaled the turn at all, where `Return` means "the
    /// turn's body ends here", not "skip the turn's own book-keeping" — a real, silent data loss,
    /// not merely a wording gap.
    ///
    /// Wraps `stmts` in a single-pass `While true: { …; Break. }` and rewrites every `Return`
    /// inside into "note we're done, then `Break`" — landing control immediately AFTER this
    /// wrapper, which is exactly where the epilogue already sits. A `Return` nested inside the
    /// AUTHOR's own `While`/`Repeat` only breaks that loop (`Break`'s own rule: the innermost one),
    /// so the flag is re-checked right after each such loop returns and re-broken if it was set —
    /// cascading out through every nesting level between the `Return` and this wrapper, however
    /// deep. `Inspect` needs no such re-check: it is not a loop, so a `Break` inside one of its arms
    /// already passes straight through it looking for the nearest enclosing `While`, the same way
    /// it already does through a plain `If`.
    pub(crate) fn protect_early_return(&mut self, stmts: Vec<Stmt<'a>>) -> Vec<Stmt<'a>> {
        if !Self::body_contains_return(&stmts) {
            return stmts;
        }
        let done = self.interner.intern("__lgx_returned");
        let mut rewritten = self.rewrite_returns_to_break(stmts, done);
        rewritten.push(Stmt::Break);
        let init = Stmt::Let {
            var: done,
            ty: None,
            value: self.te_bool(false),
            mutable: true,
        };
        let loop_stmt = Stmt::While {
            cond: self.te_bool(true),
            body: self.alloc_block(rewritten),
            decreasing: None,
        };
        vec![init, loop_stmt]
    }

    pub(crate) fn rewrite_returns_to_break(
        &mut self,
        stmts: Vec<Stmt<'a>>,
        done: Symbol,
    ) -> Vec<Stmt<'a>> {
        stmts
            .into_iter()
            .flat_map(|s| self.rewrite_return_stmt(s, done))
            .collect()
    }

    /// One statement of [`Self::rewrite_returns_to_break`]'s own walk — a `Vec` result because a
    /// bare `Return` becomes TWO statements (set the flag, then `Break`).
    pub(crate) fn rewrite_return_stmt(&mut self, stmt: Stmt<'a>, done: Symbol) -> Vec<Stmt<'a>> {
        match stmt {
            Stmt::Return { .. } => vec![
                Stmt::Set {
                    target: done,
                    value: self.te_bool(true),
                },
                Stmt::Break,
            ],
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                let then_rewritten = self.rewrite_returns_to_break(then_block.to_vec(), done);
                let then_block = self.alloc_block(then_rewritten);
                let else_block = else_block.map(|b| {
                    let rewritten = self.rewrite_returns_to_break(b.to_vec(), done);
                    self.alloc_block(rewritten) as crate::ast::stmt::Block<'a>
                });
                vec![Stmt::If {
                    cond,
                    then_block,
                    else_block,
                }]
            }
            Stmt::While {
                cond,
                body,
                decreasing,
            } => {
                let body_rewritten = self.rewrite_returns_to_break(body.to_vec(), done);
                let body = self.alloc_block(body_rewritten);
                vec![
                    Stmt::While {
                        cond,
                        body,
                        decreasing,
                    },
                    Stmt::If {
                        cond: self.te_id(done),
                        then_block: self.alloc_block(vec![Stmt::Break]),
                        else_block: None,
                    },
                ]
            }
            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => {
                let body_rewritten = self.rewrite_returns_to_break(body.to_vec(), done);
                let body = self.alloc_block(body_rewritten);
                vec![
                    Stmt::Repeat {
                        pattern,
                        iterable,
                        body,
                    },
                    Stmt::If {
                        cond: self.te_id(done),
                        then_block: self.alloc_block(vec![Stmt::Break]),
                        else_block: None,
                    },
                ]
            }
            Stmt::Inspect {
                target,
                arms,
                has_otherwise,
            } => {
                let arms = arms
                    .into_iter()
                    .map(|mut arm| {
                        let rewritten = self.rewrite_returns_to_break(arm.body.to_vec(), done);
                        arm.body = self.alloc_block(rewritten);
                        arm
                    })
                    .collect();
                vec![Stmt::Inspect {
                    target,
                    arms,
                    has_otherwise,
                }]
            }
            other => vec![other],
        }
    }

    /// The shape every turn shares: read state in, run, write state back, repaint once.
    pub(crate) fn turn_body(
        &mut self,
        def: &ComponentDef<'a>,
        inner: crate::ast::stmt::Block<'a>,
        journal_name: &str,
    ) -> Option<crate::ast::stmt::Block<'a>> {
        self.turn_body_with(def, inner, true, Some(journal_name))
    }

    /// [`Self::turn_body`], with the closing paint made optional.
    ///
    /// ★ A HANDLER FOR A COMPONENT THAT HAS LEFT MUST NOT PAINT. `When this component disappears:`
    /// runs for an instance that is no longer on the page, and its repaint would `clearChildren`
    /// the element the instance used to occupy — an element some other component may now own. So
    /// it runs the state half of a turn, without the half that draws.
    ///
    /// `journal_name` is `Some(name)` exactly where this turn is a journal entry — see
    /// [`Self::lower_component_event`]'s own exhaustive trigger match for which handlers that is.
    /// `None` for a handler that is no turn of its own (no turn to note).
    pub(crate) fn turn_body_with(
        &mut self,
        def: &ComponentDef<'a>,
        inner: crate::ast::stmt::Block<'a>,
        paint: bool,
        journal_name: Option<&str>,
    ) -> Option<crate::ast::stmt::Block<'a>> {
        let component = self.interner.resolve(def.name).to_string();
        self.lgx_pending_events.clear();
        self.lgx_pending_reload = false;
        let mut body: Vec<Stmt<'a>> = Vec::new();
        // An OPEN turn, first thing: what a trap interrupts, the failure door journals.
        if let Some(name) = journal_name {
            let instance = self.journal_instance(def);
            body.extend(self.open_turn(name, instance));
        }
        // ★ WHOSE TURN THIS IS, written before anything can fail. EVERY turn writes it, including
        // one whose component has no failure handler — that case writes the id `0`, which matches
        // no arm. Writing only where there IS a handler would leave a stale id behind and
        // misattribute the next component's trap to the last one that could handle it.
        if !self.lgx_fail_handlers.is_empty() {
            let which = self.interner.intern(failing_component_binding());
            let whose = self.interner.intern(failing_key_binding());
            let id = self
                .lgx_fail_handlers
                .iter()
                .position(|(c, _)| *c == def.name)
                .map_or(0, |i| i as i64 + 1);
            body.push(Stmt::Set {
                target: which,
                value: self.te_int(id),
            });
            body.push(Stmt::Set {
                target: whose,
                value: self.key_expr(),
            });
        }
        // ★ A PROP IS IN SCOPE IN A TURN TOO, THE SAME WAY STATE IS. The render function receives a
        // prop as a plain parameter, but a turn is a SEPARATE function taking only the instance key
        // — so a handler that reads a prop by its bare name (`When this component appears: Show
        // "in {label}".` on a component whose only field is a prop) found no local bound for it at
        // all: "Undefined variable", on every tier, since both read the identical lowered body.
        // Read from `__P_<Component>_<prop>` by this turn's own key, the same door the render
        // function itself writes through — see `prop_write`/`prop_read`.
        for (prop, ty) in &def.props {
            let value = self.prop_read(&component, *prop);
            body.push(Stmt::Let {
                var: *prop,
                ty: Some(*ty),
                value,
                mutable: false,
            });
        }
        for field in def.state.iter().filter(|f| !f.derived) {
            let value = self.state_read(&component, field.name);
            body.push(self.state_field_let(field, value, true));
        }
        // ★ A DERIVED FIELD IS IN SCOPE IN A TURN TOO. Only the non-derived ones were bound, so an
        // action that so much as READ one — `If remaining is 0:`, the most ordinary sentence a page
        // has — would fail at run time with "Undefined variable". The filter above is right for
        // what a turn WRITES BACK; it is not what a turn can SEE.
        //
        // Bound from the locals just read, in DECLARATION ORDER, which is what makes a derived that
        // reads another derived work. And bound ONCE, at the top: a derived read mid-turn holds its
        // turn-start value, because a turn reads state in, runs, and writes back at the end:
        // `Set n to 10. Set seen to dbl.` leaves `seen` at the OLD `dbl`, and the field itself
        // finishes at the new one.
        for field in def.state.iter().filter(|f| f.derived) {
            body.push(self.state_field_let(field, field.init, true));
        }
        // What every field showed when the turn began — the `before` of its journal entry.
        let journal_seq = match journal_name {
            Some(_) => {
                let (seq, capture) = self.journal_before(def, "__lgx_jb");
                body.extend(capture);
                Some(seq)
            }
            None => None,
        };
        // ★ `When <field> changes:` — the effect that needed no host import, and the one a page
        // reaches for first. Each watched field's VALUE is compared before and after the turn, the
        // matching handlers run ONCE in declaration order, and then the page settles; a handler
        // that writes the field it watches does not retrigger, which is what keeps a turn
        // bounded.
        //
        // ★ THE HANDLERS ARE INLINED RATHER THAN CALLED. Each is also lowered as its own turn
        // function, and calling one here would run a SECOND complete turn — its own write-back and
        // its own patch — so one visitor action would paint twice. Inlined, they mutate the same
        // locals this turn already holds, the write-back below commits everything at once, and the
        // page settles once.
        //
        // ★ ONLY A TURN RUNS THEM — an action, a bind write, a listener delivery, exactly the
        // turns that journal. A lifecycle
        // hook or a failure handler (`journal_name` is `None`) runs its body and nothing after it.
        let watched: Vec<(Symbol, Symbol, crate::ast::stmt::Block<'a>)> = def
            .events
            .iter()
            .filter(|_| journal_name.is_some())
            .filter_map(|h| match h.trigger {
                Trigger::FieldChanges { store: None, field } => Some((field, h.body)),
                _ => None,
            })
            .filter(|(field, _)| def.state.iter().any(|f| f.name == *field && !f.derived))
            .map(|(field, block)| {
                let was = self
                    .interner
                    .intern(&format!("__lgx_was_{}", self.interner.resolve(field)));
                (field, was, block)
            })
            .collect();
        for (field, was, _) in &watched {
            let value = self.te_id(*field);
            body.push(Stmt::Let {
                var: *was,
                ty: None,
                value,
                mutable: false,
            });
        }
        // `Load "k" into f` needs f's type — see `lgx_state_types`.
        self.note_state_types(def);
        // ★ ASKED BEFORE THE REWRITE. `rewrite_store_calls` SHADOWS `inner` on the next line, and
        // what it returns no longer contains a `CallStore` — it has become the call plus its
        // watcher bookkeeping. Asked THROUGH every `Call <sibling>.` too: a sibling runs inlined in
        // this turn, so a store it moves is a store this turn moved.
        let moved_a_store = touches_store_through(def, inner);
        let inner = self.rewrite_store_calls(inner, &component)?;
        let inner = self.protect_early_return(inner.to_vec());
        let inner: crate::ast::stmt::Block<'a> = self.alloc_block(inner);
        body.extend(inner.iter().cloned());
        // Every field this turn could write, INCLUDING what a change handler writes — the patch
        // plan reads this to decide which sites it may update in place, and a handler's writes
        // are as real as the action's. `block_writes` already recurses into a `While`'s own body
        // (`writes_of_stmt`), so a write past a `protect_early_return` wrap is still seen.
        let mut writes = crate::lgx::deps::block_writes(def, inner, self.interner);
        // ★ THE CHANGED SET IS THE ACTION'S, judged once, before any handler runs: a handler's own
        // write wakes no other handler (a turn is one round), and every handler for a moved field
        // runs once, in declaration order.
        let mut handlers: Vec<(Symbol, crate::ast::stmt::Block<'a>)> = Vec::new();
        for (i, (field, was, block)) in watched.iter().enumerate() {
            let block = self.rewrite_store_calls(*block, &component)?;
            let block = self.protect_early_return(block.to_vec());
            let block: crate::ast::stmt::Block<'a> = self.alloc_block(block);
            for w in crate::lgx::deps::block_writes(def, block, self.interner) {
                if !writes.contains(&w) {
                    writes.push(w);
                }
            }
            let flag = self.interner.intern(&format!("__lgx_chg{i}"));
            let changed = self.te_bin(BinaryOpKind::NotEq, self.te_id(*field), self.te_id(*was));
            body.push(Stmt::Let {
                var: flag,
                ty: None,
                value: changed,
                mutable: false,
            });
            handlers.push((flag, block));
        }
        // ★ THE ACTION COMMITS BEFORE ITS HANDLERS RUN, so a handler cannot undo it: one that traps
        // is absorbed at the failure door (`__LGX_WATCHING`), which journals and redraws this turn
        // as its action left it.
        if !handlers.is_empty() {
            for field in def.state.iter().filter(|f| !f.derived) {
                let value = self.te_id(field.name);
                let write = self.state_write(&component, field.name, value);
                body.push(write);
            }
            if let Some(seq) = journal_seq {
                let watch_changes = self.interner.intern(watch_changes_binding());
                body.extend(self.journal_changes(def, seq, "__lgx_wc", watch_changes));
            }
            body.extend(self.watch_phase(def, handlers));
        }
        for field in def.state.iter().filter(|f| !f.derived) {
            let value = self.te_id(field.name);
            let write = self.state_write(&component, field.name, value);
            body.push(write);
        }
        // ★ NOTED RIGHT WHERE THE TURN COMMITS — after the write-back, before anything this turn
        // wakes gets a chance to run. So a component action that calls a store's own action
        // journals the STORE'S entry first (that call already ran, inside `inner`, above) and its
        // own turn second — the nesting order, for free, with no bookkeeping. Placed before
        // `woken`/pending events/the paint below, so whatever THOSE wake journals after this turn's
        // own entry, as a store change fanned out to a watcher does.
        if let (Some(name), Some(seq)) = (journal_name, journal_seq) {
            body.extend(self.journal_changes(def, seq, "__lgx_jc", seq));
            body.extend(self.close_turn());
            let instance = self.journal_instance(def);
            let blank = self.interner.intern("");
            let committed = self.te_text(blank);
            body.push(self.note_turn(name, instance, self.te_id(seq), committed));
        }
        // ★ THIS TURN PAINTS WHAT IT COMMITTED, THEN RUNS THE TURNS IT OWES — in that order. A
        // store watcher, a queued element event and a route change each run a turn of their own,
        // which writes back and paints for itself, and any of them may be a turn of THIS instance
        // (its own `When the route changes:` answering its own `Navigate to`). Painting after them
        // would redraw this instance from the locals this turn still holds, over what that nested
        // turn committed. A turn that will reload the page paints nothing: the page is replaced.
        let reloads = std::mem::take(&mut self.lgx_pending_reload);
        if paint && !reloads {
            body.extend(self.turn_paint(def, moved_a_store, &writes));
        }
        // ★ WHETHER THIS TURN MOVED THE LOCATION WITHIN THE PAGE, taken now — before the turns it
        // owes run — so the move is delivered once, by this turn, after them: its own `Navigate to`
        // (or a store action's, or an inlined sibling's) set it, and the host does not report a
        // pushState back to the page that made it.
        let navigates = any_navigates(&self.lgx_all_components);
        let moved_now = self.interner.intern("__lgx_moved_now");
        if navigates {
            let moved = self.interner.intern(moved_binding());
            body.push(Stmt::Let {
                var: moved_now,
                ty: None,
                value: self.te_id(moved),
                mutable: false,
            });
            body.push(Stmt::Set {
                target: moved,
                value: self.te_bool(false),
            });
        }
        // ★ THE STORE WATCHERS THIS TURN WOKE, after the write-back above: a watcher writes its own
        // component's state, and running it before the write-back would have that write undone by
        // the copy this turn is holding.
        body.extend(self.wake_stores_call());
        // The events this turn asked for, now that its state has committed and its paint is drawn.
        for (selector, event, value) in std::mem::take(&mut self.lgx_pending_events) {
            let dispatch = self.interner.intern(dispatch_fn_name());
            body.push(Stmt::Call {
                function: dispatch,
                args: vec![selector, event, value],
            });
        }
        if navigates {
            let changed = self.interner.intern(route_changed_fn_name());
            let deliver = self.alloc_block(vec![Stmt::Call {
                function: changed,
                args: Vec::new(),
            }]);
            body.push(Stmt::If {
                cond: self.te_id(moved_now),
                then_block: deliver,
                else_block: None,
            });
        }
        if reloads {
            let f = self.interner.intern("reloadPage");
            body.push(Stmt::Call {
                function: f,
                args: Vec::new(),
            });
        }
        Some(self.alloc_block(body))
    }
}
