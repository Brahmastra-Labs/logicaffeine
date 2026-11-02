use super::*;
use crate::ast::component::{ComponentDef, ComponentKind, StateField, Trigger};
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use logicaffeine_base::Symbol;

impl<'a, 'ctx, 'int> super::super::Parser<'a, 'ctx, 'int> {
    /// Rewrite `Call <Store>'s <action>` into the call it lowers to, everywhere inside a body.
    ///
    /// Returns `None` when the body nests one inside a form this walk does not descend into —
    /// a concurrency block, a zone, a span. A `Call` this missed would not happen at all, so the
    /// component is refused instead.
    pub(crate) fn rewrite_store_calls(
        &mut self,
        body: crate::ast::stmt::Block<'a>,
        component: &str,
    ) -> Option<crate::ast::stmt::Block<'a>> {
        let mut out = Vec::with_capacity(body.len());
        for stmt in body.iter() {
            out.extend(self.rewrite_store_call_stmt(stmt, component)?);
        }
        Some(self.alloc_block(out))
    }

    /// Whether the program's prelude declares the module binding `name` — the one record of which
    /// optional page bookkeeping this program carries, read wherever a later part of the lowering
    /// must agree with it.
    pub(crate) fn declares_binding(&self, name: &str) -> bool {
        let Some(var) = self.interner.lookup(name) else {
            return false;
        };
        self.deferred_functions
            .iter()
            .any(|s| matches!(s, Stmt::Let { var: v, .. } if *v == var))
    }

    /// Whether some lowerable component in `defs` can be drawn inside itself, so the page counts
    /// render depth (`__LGX_RENDER_DEPTH`).
    pub(crate) fn counts_render_depth(&self, defs: &[ComponentDef<'a>]) -> bool {
        defs.iter().any(|d| {
            lowerable(d, defs, self.type_registry.as_ref(), self.interner).is_ok()
                && on_placement_cycle(d, defs)
        })
    }

    /// Whether some lowered component watches a store's field — the one question behind the
    /// round counter, the moved-field flags, `__lgx_wake_stores` and every turn's call to it.
    pub(crate) fn wakes_store_watchers(&self) -> bool {
        !self.lgx_store_watchers.is_empty()
    }

    /// Every `(store, field)` some component watches, once each, in the order the watchers were
    /// declared — the order a round runs them in.
    pub(crate) fn watched_store_fields(&self) -> Vec<(Symbol, Symbol)> {
        let mut out: Vec<(Symbol, Symbol)> = Vec::new();
        for (_, store, field, _) in &self.lgx_store_watchers {
            if !out.contains(&(*store, *field)) {
                out.push((*store, *field));
            }
        }
        out
    }

    /// What a store action does just before its write-back: flag every watched field whose value
    /// the action is about to change.
    ///
    /// ★ THE STORE NOTES THE CHANGE, THE TURN RUNS THE WATCHERS — the store action notes the
    /// changed set, the outermost turn drains it. Noted here, the change is seen wherever the call
    /// is: in a branch, a loop, a sibling's inlined body, another
    /// store's action, `When the page loads:`. A comparison kept at the CALL SITE sees only calls
    /// at the top of a turn, and one kept per call site runs a watcher once per call rather than
    /// once per move. ★ ONLY THE FIELDS THAT ACTUALLY MOVED: a handler runs only for a field in
    /// the changed set.
    pub(crate) fn store_moved_notes(&mut self, def: &ComponentDef<'a>) -> Vec<Stmt<'a>> {
        let mut out = Vec::new();
        for (store, field) in self.watched_store_fields() {
            if store != def.name {
                continue;
            }
            let committed = self.ctx.alloc_imperative_expr(Expr::FieldAccess {
                object: self.te_id(store),
                field,
            });
            let moved = self.te_bin(BinaryOpKind::NotEq, self.te_id(field), committed);
            let flag = self.interner.intern(&store_moved_binding(
                self.interner.resolve(store),
                self.interner.resolve(field),
            ));
            let note = self.alloc_block(vec![Stmt::Set {
                target: flag,
                value: self.te_bool(true),
            }]);
            out.push(Stmt::If {
                cond: moved,
                then_block: note,
                else_block: None,
            });
        }
        out
    }

    /// The call every turn ends with once its state is written back and its paint drawn: the
    /// watchers of whatever store the turn moved run now, each as a turn of its own.
    pub(crate) fn wake_stores_call(&mut self) -> Vec<Stmt<'a>> {
        if !self.wakes_store_watchers() {
            return Vec::new();
        }
        let function = self.interner.intern(wake_stores_fn_name());
        vec![Stmt::Call {
            function,
            args: Vec::new(),
        }]
    }

    /// `__lgx_wake_stores()` — one round of store watchers.
    ///
    /// ★ THE FLAGS ARE TAKEN BEFORE ANY WATCHER RUNS. A watcher is a turn, and its own round
    /// (the call it ends with) must see only what IT moved; a flag still set would run this
    /// round's watchers again from inside it.
    ///
    /// ★ A ROUND IS COUNTED WHILE IT RUNS, and only a round with watchers to run is one. A watcher
    /// that moves a store someone watches starts the next round from INSIDE this one, so the
    /// rounds running inside one another are the cascade's depth; past `STORE_CASCADE_LIMIT` the
    /// page fails in `store_cascade_message`'s words before the call stack gives out. A turn that moved
    /// nothing enters no round, so a cascade that settles on its last watcher is as deep as the
    /// watchers that ran. The failure door starts the count over, so a refused cascade leaves
    /// nothing behind for the next turn.
    pub(crate) fn lower_wake_stores(&mut self) -> Option<Stmt<'a>> {
        if !self.wakes_store_watchers() {
            return None;
        }
        let watchers = self.lgx_store_watchers.clone();
        let mut body: Vec<Stmt<'a>> = Vec::new();
        let any = self.interner.intern("__lgx_any");
        body.push(Stmt::Let {
            var: any,
            ty: None,
            value: self.te_bool(false),
            mutable: true,
        });
        let mut taken: Vec<(Symbol, Symbol, Symbol)> = Vec::new();
        for (i, (store, field)) in self.watched_store_fields().into_iter().enumerate() {
            let flag = self.interner.intern(&store_moved_binding(
                self.interner.resolve(store),
                self.interner.resolve(field),
            ));
            let moved = self.interner.intern(&format!("__lgx_moved{i}"));
            body.push(Stmt::Let {
                var: moved,
                ty: None,
                value: self.te_id(flag),
                mutable: false,
            });
            body.push(Stmt::Set {
                target: flag,
                value: self.te_bool(false),
            });
            let either = self.te_bin(BinaryOpKind::Or, self.te_id(any), self.te_id(moved));
            body.push(Stmt::Set {
                target: any,
                value: either,
            });
            taken.push((store, field, moved));
        }
        let mut round: Vec<Stmt<'a>> = Vec::new();
        let cascade = self.interner.intern(cascade_binding());
        let deeper = self.te_bin(BinaryOpKind::Add, self.te_id(cascade), self.te_int(1));
        round.push(Stmt::Set {
            target: cascade,
            value: deeper,
        });
        let limit = crate::ast::component::STORE_CASCADE_LIMIT as i64;
        let past = self.te_bin(BinaryOpKind::Gt, self.te_id(cascade), self.te_int(limit));
        let message = self
            .interner
            .intern(&crate::ast::component::store_cascade_message());
        let refuse = self.alloc_block(vec![Stmt::Fail {
            reason: self.te_text(message),
        }]);
        round.push(Stmt::If {
            cond: past,
            then_block: refuse,
            else_block: None,
        });
        let k = self.interner.intern("__lgx_wk");
        let seed = self.interner.intern(seed_key());
        for (store, field, moved) in taken {
            let mut fired: Vec<Stmt<'a>> = Vec::new();
            for (component, _, _, index) in watchers
                .iter()
                .filter(|(_, s, f, _)| *s == store && *f == field)
            {
                let name = self.interner.resolve(*component).to_string();
                let keys = self.interner.intern(&keys_binding(&name));
                let act = self
                    .interner
                    .intern(&action_fn_name(&name, &format!("__e{index}")));
                // ★ SKIP THE SEED. `__KEYS_<C>` is seeded with a key no instance can have, so the
                // list has a type before anything renders; the sweep skips it by its `__SEEN_`
                // stamp and the timers by their due entry, and this walk reads neither, so it says
                // so itself — or the handler runs once for an instance that does not exist.
                let real = self.te_bin(BinaryOpKind::NotEq, self.te_id(k), self.te_text(seed));
                let fire = self.alloc_block(vec![Stmt::Call {
                    function: act,
                    args: vec![self.te_id(k)],
                }]);
                let per_key = self.alloc_block(vec![Stmt::If {
                    cond: real,
                    then_block: fire,
                    else_block: None,
                }]);
                fired.push(Stmt::Repeat {
                    pattern: crate::ast::stmt::Pattern::Identifier(k),
                    iterable: self.te_id(keys),
                    body: per_key,
                });
            }
            let then_block = self.alloc_block(fired);
            round.push(Stmt::If {
                cond: self.te_id(moved),
                then_block,
                else_block: None,
            });
        }
        let shallower = self.te_bin(BinaryOpKind::Subtract, self.te_id(cascade), self.te_int(1));
        round.push(Stmt::Set {
            target: cascade,
            value: shallower,
        });
        let round = self.alloc_block(round);
        body.push(Stmt::If {
            cond: self.te_id(any),
            then_block: round,
            else_block: None,
        });
        Some(Stmt::FunctionDef {
            name: self.interner.intern(wake_stores_fn_name()),
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

    /// ★ IT RETURNS A LIST, because one sentence can lower to more than one statement: a
    /// `Call <sibling>.` becomes the sibling's whole action, inlined into this turn. There is no
    /// block statement in this AST to fold those into one, and a degenerate `if true:` to fake one
    /// would be a lie in the tree that every later pass has to see through.
    pub(crate) fn rewrite_store_call_stmt(
        &mut self,
        stmt: &Stmt<'a>,
        component: &str,
    ) -> Option<Vec<Stmt<'a>>> {
        use crate::ast::stmt::UiStmt;
        if let Stmt::Call { function, args } = stmt {
            if let Some(inlined) = self.inline_own_action_call(*function, args, component) {
                return inlined;
            }
            if let Some(call) = self.callback_prop_call(*function, args, component) {
                return Some(vec![call]);
            }
        }
        if let Stmt::Ui(UiStmt::CallStore {
            store,
            action,
            args,
        }) = stmt
        {
            let fname = self.interner.intern(&action_fn_name(
                self.interner.resolve(*store),
                self.interner.resolve(*action),
            ));
            // ★ NO INSTANCE KEY. A store is not a component instance: `lower_store` gives it a
            // RECORD and one function per action, and its state lives in that record's fields
            // rather than in the per-instance map a component's `State:` uses. Passing a key here
            // was an arity error — caught by `lgx_vm_parity_lock`, which is what a cross-tier lock
            // over a construct you did not think you touched is for.
            let call = Stmt::Call {
                function: fname,
                args: args.clone(),
            };
            // A store action runs inside the caller's turn, so what it queues is delivered after
            // the CALLER commits — one drain, at the outermost turn.
            if store_action_queues(&self.lgx_all_components, *store, *action) {
                self.request_outbox_flush();
            }
            return Some(vec![call]);
        }
        Some(vec![match stmt {
            // ★ A TYPED REQUEST RUNS ON THIS TIER. It used to be refused by name — "its answer
            // arrives in a closure, and this tier cannot call a closure it found at run time" —
            // which was true of the DRIVING VERBS and not of this: an `on-click={action}` already
            // lowers to a closure the host calls, and a fetch completion is the same shape.
            //
            // So it lowers to the correlated request every other fetch uses, with the completion
            // as a closure that reads the answer, builds the `Result of T and HttpError` the
            // action's parameter declares, and calls the action. WITHOUT this a page could not
            // load its own data in a browser: the one thing a frontend framework is for.
            Stmt::Fetch {
                method,
                url,
                body,
                as_type,
                then,
            } => self.lower_typed_fetch(component, *method, url, *body, as_type, *then)?,
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => Stmt::If {
                cond,
                then_block: self.rewrite_store_calls(then_block, component)?,
                else_block: match else_block {
                    Some(b) => Some(self.rewrite_store_calls(b, component)?),
                    None => None,
                },
            },
            Stmt::While {
                cond,
                body,
                decreasing,
            } => Stmt::While {
                cond,
                body: self.rewrite_store_calls(body, component)?,
                decreasing: *decreasing,
            },
            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => Stmt::Repeat {
                pattern: pattern.clone(),
                iterable,
                body: self.rewrite_store_calls(body, component)?,
            },
            Stmt::Splice { body } => Stmt::Splice {
                body: self.rewrite_store_calls(body, component)?,
            },
            Stmt::Inspect {
                target,
                arms,
                has_otherwise,
            } => {
                let mut rewritten = Vec::with_capacity(arms.len());
                for arm in arms {
                    rewritten.push(crate::ast::stmt::MatchArm {
                        enum_name: arm.enum_name,
                        variant: arm.variant,
                        bindings: arm.bindings.clone(),
                        body: self.rewrite_store_calls(arm.body, component)?,
                    });
                }
                Stmt::Inspect {
                    target,
                    arms: rewritten,
                    has_otherwise: *has_otherwise,
                }
            }
            // ★ THE `Actions:` VERBS THAT REACH A BROWSER, lowered into what already exists.
            //
            // Seventeen of them used to fail `compile_to_wasm` behind one catch-all that named
            // neither the verb nor the page, while every other tier said yes — they parse, they run
            // on the tree-walker, they pass `largo test`. So a page written from `docs/web.md`
            // (which teaches `Set the title to` by name) rendered in a test and would not build.
            //
            // These three need no new host import and no opcode, which is this file's own rule:
            // lower into what exists. The rest still refuse, and now refuse BY NAME.

            // `Toggle done.` touches no document at all — it flips a yes/no.
            //
            // ★ AGAINST THE LOCAL, NEVER THE STATE BINDING. A component's state is a map from
            // instance key to value, and `turn_body` reads every field into a local at the top and
            // writes them all back at the end — which is what makes a turn a transaction, since a
            // trap never reaches the write-back. Assigning the binding here would put a scalar
            // where a map belongs and surface as "Cannot index into Int" somewhere unrelated.
            Stmt::Ui(UiStmt::Toggle { field }) => Stmt::Set {
                target: *field,
                value: self.te_not(self.te_id(*field)),
            },
            // `Navigate to "/todos".` and `Replace the URL with "/todos/1".` are pushState and
            // replaceState — `dom_navigate` and `dom_replace_route`, both already in the frozen
            // host surface, reached through their ordinary builtin names.
            // ★ MOVING THE ROUTE OWES `When the route changes:` A DELIVERY. `navigate` records the
            // new path and nothing else — the handler registered with `onRoute` is what a real
            // browser's popstate fires, and a pushState fires nothing. So the route change is noted
            // here and delivered when the turn unwinds; `navigate` alone would leave a page that
            // navigated itself with its own route handler never run.
            //
            // Deferred for the same reason every other delivery is: a handler is another turn, and
            // running it inline would have this turn's write-back overwrite whatever it set.
            //
            // ★ ANOTHER PAGE FILE IS ANOTHER DOCUMENT. A module with a `## Page` asks its own
            // `__lgx_owns` whether it serves the target: it does → pushState within the page; it
            // does not → `assignLocation`, which loads the document that serves it (1.9). Only an
            // in-page move sets `__LGX_MOVED`, the flag the turn's epilogue takes to deliver it.
            Stmt::Ui(UiStmt::Navigate { url }) => {
                let navigate = self.interner.intern("navigate");
                let moved = self.interner.intern(moved_binding());
                let set_moved = Stmt::Set {
                    target: moved,
                    value: self.te_bool(true),
                };
                let has_page = self
                    .lgx_all_components
                    .iter()
                    .any(|d| matches!(d.kind, ComponentKind::Page { .. }));
                if !has_page {
                    let push = Stmt::Call {
                        function: navigate,
                        args: vec![url],
                    };
                    return Some(vec![push, set_moved]);
                }
                let to = self.interner.intern("__lgx_to");
                let owns = self.interner.intern(owns_fn_name());
                let assign = self.interner.intern("assignLocation");
                let within = self.te_call(owns, vec![self.te_id(to)]);
                let body = vec![
                    Stmt::Let {
                        var: to,
                        ty: None,
                        value: url,
                        mutable: false,
                    },
                    Stmt::If {
                        cond: within,
                        then_block: self.alloc_block(vec![
                            Stmt::Call {
                                function: navigate,
                                args: vec![self.te_id(to)],
                            },
                            set_moved,
                        ]),
                        else_block: Some(self.alloc_block(vec![Stmt::Call {
                            function: assign,
                            args: vec![self.te_id(to)],
                        }])),
                    },
                ];
                Stmt::Splice {
                    body: self.alloc_block(body),
                }
            }
            Stmt::Ui(UiStmt::ReplaceUrl { url }) => {
                let f = self.interner.intern("replaceRoute");
                Stmt::Call {
                    function: f,
                    args: vec![url],
                }
            }
            // ★ `Save … as`, `Load … into` and `Forget` REACH THE BROWSER, over the storage
            // imports that were already frozen (`storageSet`/`storageGetOr`/`storageRemove`).
            //
            // A browser's storage holds text, and `Load` must give back the value that was saved —
            // the Int you saved is an Int again — so mapping `Save n as "k"` straight onto Text
            // would hand the page a Text it then computes with as if it were the Int.
            //
            // So the stored text is SELF-DESCRIBING. `Save` writes a one-letter
            // tag and the value (`i:42`, `t:hi`, `b:true`); `Load` reads the tag off and gives the
            // field back what was put in. Both halves are compiled LOGOS over the existing
            // imports, so the tiers cannot drift and no host was involved.
            Stmt::Ui(UiStmt::Save { value, key }) => {
                let set = self.interner.intern("storageSet");
                let tagged = self.tagged_value(value)?;
                let area = self.storage_area("local");
                Stmt::Call {
                    function: set,
                    args: vec![area, key, tagged],
                }
            }
            // ★ `Set the title to "…".` — taught by name in `docs/web.md` and refused by the
            // browser tier until now, which is the worst combination: a sentence the docs hand
            // someone, that fails to build the moment they use it.
            Stmt::Ui(UiStmt::SetTitle { title }) => {
                let f = self.interner.intern("setTitle");
                Stmt::Call {
                    function: f,
                    args: vec![title],
                }
            }
            Stmt::Ui(UiStmt::GoBack) => {
                let f = self.interner.intern("goBack");
                Stmt::Call {
                    function: f,
                    args: Vec::new(),
                }
            }
            // ★ `Add class "x" to "#s".` / `Remove class "x" from "#s".` — the class RULE is
            // compiled LOGOS (`classListWith`/`classListWithout` in web.lg), not a host function.
            //
            // A class list is space-separated WORDS, so which words survive is a decision, and a
            // decision belongs where every tier runs it and every ordinary lock tests it. That is
            // why the import added here is the generic `getAttribute` READ rather than
            // `dom_add_class`/`dom_remove_class`: with those two, the rule would live in a .js
            // file only a browser executes.
            //
            // `querySelector` is called twice rather than bound to a local: it is a pure lookup,
            // and a synthesized `Let` here would collide with itself the moment a body changed two
            // classes.
            // ★ `Focus "#draft".` / `Blur "#draft".` — the DOM command, then the event.
            //
            // A real browser fires `focus`/`blur` itself when `el.focus()` is called, so the page's
            // own listener is answered by the platform. The in-process tiers have no platform, so
            // the delivery is spelled out — and it is the SAME dispatcher a click goes through,
            // which is why a `When "#draft" is focused:` sentence answers on every tier.
            //
            // Neither requires an answer: putting the cursor in a field is a thing a page does,
            // and a form that focuses its own first input answers nothing.
            // ★ NO EVENT FOLLOWS THESE TWO. `focus` and `blur` are events a page can wait for;
            // scrolling an element into view and selecting its text raise nothing the language
            // offers a sentence for, so the command is the whole of it.
            // ★ `Reload the page.` RUNS AFTER THE WRITE-BACK, AND ENDS THE TURN.
            //
            // It was refused here for a long time, and the reason was real: a turn writes its state
            // back at the END, so clearing every instance mid-body meant the write-back re-created
            // the one it just cleared and the page came back carrying what the reload was meant to
            // forget. What closed it was not new machinery but the ORDER — the same deferral focus
            // and blur needed, one step further: the reload is emitted after the write-back AND the
            // paint is skipped, because a page that has been torn down has nothing to draw.
            Stmt::Ui(UiStmt::Reload) => {
                self.lgx_pending_reload = true;
                // Nothing here: a `Splice` with an empty body keeps the statement list shaped
                // without emitting an act that would run at the wrong moment.
                Stmt::Splice {
                    body: self.alloc_block(Vec::new()),
                }
            }
            Stmt::Ui(UiStmt::CopyToClipboard { text }) => {
                let f = self.interner.intern("clipboardWrite");
                Stmt::Call {
                    function: f,
                    args: vec![text],
                }
            }
            Stmt::Ui(UiStmt::Download { bytes, name }) => {
                let f = self.interner.intern("downloadFile");
                Stmt::Call {
                    function: f,
                    args: vec![bytes, name],
                }
            }
            Stmt::Ui(UiStmt::Notify { message, tone }) => {
                let f = self.interner.intern("notify");
                // A tone the author did not write is the empty text, not a default word — every
                // host must agree about what "no tone" IS, and inventing one here would make "no
                // tone" and this guess two different things.
                let t = match tone {
                    Some(t) => self.te_text(*t),
                    None => {
                        let empty = self.interner.intern("");
                        self.te_text(empty)
                    }
                };
                Stmt::Call {
                    function: f,
                    args: vec![message, t],
                }
            }
            // ★ TWO LEGS, AND ONLY ONE OF THEM IS THE HOST'S. The frame goes out through
            // `publish`; the page's OWN subscribers are reached by dispatching `message:<topic>`
            // through the same table `Raise` drives, in compiled LOGOS. So a page's local
            // delivery is identical on every tier with no transport present at all, and the
            // import carries only the leg that genuinely leaves the page — the shim rule honoured
            // rather than worked around.
            //
            // The local leg is DEFERRED to after the write-back, like every other event a turn
            // asks for: a subscriber is another turn, and delivering it inline would have this
            // turn's commit overwrite whatever it set.
            //
            // The topic is a VALUE here, unlike at a subscription, where it must be a literal
            // because the host is told what to listen for before the page runs. Sending needs no
            // registration, so a computed topic is answerable — hence `"message:" + topic` rather
            // than a name folded at compile time.
            Stmt::Ui(UiStmt::Publish { message, topic }) => {
                let f = self.interner.intern("publish");
                let send = Stmt::Call {
                    function: f,
                    args: vec![topic, message],
                };
                let prefix = self.interner.intern("message:");
                let event = self.te_bin(BinaryOpKind::Add, self.te_text(prefix), topic);
                let body_sym = self.interner.intern("body");
                let body = self.te_text(body_sym);
                let queued = self.queue_element_event(body, event, message);
                Stmt::Splice {
                    body: self.alloc_block(vec![send, queued]),
                }
            }
            Stmt::Ui(UiStmt::ScrollTo { selector }) => {
                self.lower_element_command_only(selector, "scrollto")
            }
            Stmt::Ui(UiStmt::SelectText { selector }) => {
                self.lower_element_command_only(selector, "selecttext")
            }
            // ★ THE PLATFORM RAISES `focus` AND `blur`, not the page. `element.focus()` and
            // `element.blur()` move focus only when it actually moves, and the host raises the
            // events of that move on the elements that lost and gained it once the turn has
            // committed — a browser for real, the in-process hosts from `dom::take_raised_events`.
            // Queueing them here as well would deliver each one twice.
            Stmt::Ui(UiStmt::Focus { selector }) => {
                self.lower_element_command_only(selector, "focus")
            }
            Stmt::Ui(UiStmt::Blur { selector }) => {
                self.lower_element_command_only(selector, "blur")
            }
            // ★ THE COMMAND AND THE EVENT ARE DIFFERENT WORDS HERE. The page asks its host to
            // hover; what a browser raises for that is `mouseover`, which is what a
            // `When "#card" is hovered:` sentence waits for.
            Stmt::Ui(UiStmt::Hover { selector }) => {
                self.lower_element_command(selector, "hover", "mouseover")
            }
            Stmt::Ui(UiStmt::AddClass { class, selector }) => {
                self.lower_class_change(selector, class, "classListWith")
            }
            Stmt::Ui(UiStmt::RemoveClass { class, selector }) => {
                self.lower_class_change(selector, class, "classListWithout")
            }
            // ★ `Stop the timer refresh.` NEEDED NO IMPORT. A timer is a due time held in a map;
            // stopping it is writing the value that means "not running", then asking the host to
            // wake the page for whatever is still running.
            // ★ THE TIMER IS THIS COMPONENT'S. Two components may each name a timer `clock`; the
            // sentence reaches the one declared by the component whose turn is running.
            Stmt::Ui(UiStmt::StopTimer { name }) => {
                let index = self.own_timer(component, *name)?;
                let due = self.interner.intern(&due_binding(component, index));
                let key = self.key_expr();
                let rearm = self.interner.intern(rearm_fn_name());
                let stop = vec![
                    Stmt::SetIndex {
                        collection: self.te_id(due),
                        index: key,
                        value: self.te_int(-1),
                    },
                    Stmt::Call {
                        function: rearm,
                        args: Vec::new(),
                    },
                ];
                Stmt::Splice {
                    body: self.alloc_block(stop),
                }
            }
            // `Start the timer clock.` arms it again from now — the same arming a first render does,
            // its period read from its sentence as the turn holds the state now.
            Stmt::Ui(UiStmt::StartTimer { name }) => {
                let index = self.own_timer(component, *name)?;
                let def = self
                    .lgx_all_components
                    .iter()
                    .find(|d| self.interner.resolve(d.name) == component)
                    .cloned()?;
                let arm = self.arm_timer(&def, index);
                Stmt::Splice {
                    body: self.alloc_block(arm),
                }
            }
            Stmt::Ui(UiStmt::Forget { key }) => {
                let remove = self.interner.intern("storageRemove");
                let area = self.storage_area("local");
                Stmt::Call {
                    function: remove,
                    args: vec![area, key],
                }
            }
            Stmt::Ui(UiStmt::Load { key, var }) => {
                // The field being written names its own type; a `Load` whose target this walk
                // cannot type is refused rather than guessed at, which is the same rule
                // `store_field_types` applies one function down.
                let ty = self
                    .lgx_state_types
                    .iter()
                    .find(|(n, _)| n == var)
                    .and_then(|(_, t)| *t);
                self.tagged_read(key, ty, *var)?
            }

            // A form this walk cannot see inside: refuse rather than risk missing a call.
            Stmt::Zone { .. }
            | Stmt::Concurrent { .. }
            | Stmt::Parallel { .. }
            | Stmt::Select { .. }
            | Stmt::During { .. }
            | Stmt::ForAll { .. }
            | Stmt::Reversibly { .. } => return None,
            other => other.clone(),
        }])
    }

    /// `Call <prop> [with a and b].` naming one of `component`'s CALLBACK props — a call of the
    /// prop's trampoline for this instance (`__cb_<C>_<prop>(key, a…)`), the one path every call of
    /// a callback prop takes, so the action the placement handed down runs as its own turn against
    /// its owner's instance. `None` for anything else; a `## To` function of the same name wins.
    fn callback_prop_call(
        &mut self,
        function: Symbol,
        args: &[&'a Expr<'a>],
        component: &str,
    ) -> Option<Stmt<'a>> {
        if self.declared_function_names.contains(&function) {
            return None;
        }
        let def = self.lgx_all_components.iter().find(|d| {
            self.interner.resolve(d.name) == component
                && matches!(
                    d.kind,
                    crate::ast::component::ComponentKind::Component
                        | crate::ast::component::ComponentKind::Page { .. }
                )
        })?;
        if !def
            .props
            .iter()
            .any(|(p, t)| *p == function && is_callback_type(t, self.interner))
        {
            return None;
        }
        let trampoline = self.interner.intern(&callback_trampoline_fn_name(
            component,
            self.interner.resolve(function),
        ));
        let mut call_args = vec![self.key_expr()];
        call_args.extend(args.iter().copied());
        Some(Stmt::Call {
            function: trampoline,
            args: call_args,
        })
    }

    /// `Call <action> [with a and b].` naming one of `component`'s OWN actions by its bare name —
    /// `None` when it names anything else (a `## To` function, which resolves first, or a callback
    /// prop), otherwise the inlined call, or `Some(None)` when the action's body
    /// holds a form this walk refuses.
    ///
    /// ★ INLINED, NOT CALLED. The sibling runs NESTED in the caller's turn: its writes land on the
    /// caller's own bindings and commit at the caller's write-back, and a trap anywhere rolls back
    /// both. Calling `__act_<C>_<a>`
    /// would run a second, complete turn — reading state the caller has not written back yet,
    /// writing its own copy back, and having the caller's write-back overwrite it. So the body runs
    /// right here, against the caller's locals:
    ///
    /// * each argument is evaluated first, in the caller's scope, into a fresh local — so a
    ///   parameter named like another argument's variable reads the caller's value;
    /// * the body runs inside a single-pass `While true: … Break.`, which gives the parameters and
    ///   the body's own `Let`s a scope of their own and turns the
    ///   body's `Return` into leaving THAT loop — the end of the nested action, not of the turn;
    /// * `noteTurn("<action>")` follows, journaling the nested call before the caller's own entry,
    ///   which is written at its epilogue.
    ///
    /// The body goes through this same rewrite, so a sibling that calls a store, publishes or calls
    /// a third sibling lowers exactly as it would at the top of a turn. `own_action_cycle` refuses a
    /// component whose actions call each other in a circle, which is what keeps this finite.
    fn inline_own_action_call(
        &mut self,
        function: Symbol,
        args: &[&'a Expr<'a>],
        component: &str,
    ) -> Option<Option<Vec<Stmt<'a>>>> {
        if self.declared_function_names.contains(&function) {
            return None;
        }
        let def = self
            .lgx_all_components
            .iter()
            .find(|d| {
                self.interner.resolve(d.name) == component
                    && matches!(
                        d.kind,
                        crate::ast::component::ComponentKind::Component
                            | crate::ast::component::ComponentKind::Page { .. }
                    )
            })?
            .clone();
        if def.props.iter().any(|(p, _)| *p == function) {
            return None;
        }
        let action = def.actions.iter().find(|a| a.name == function)?.clone();
        Some(self.inline_action_body(&def, &action, args, component))
    }

    /// The sibling's body inlined into the caller's turn, journaled as a nested call: its own name,
    /// the caller's instance, and the changes the sibling made to the caller's fields.
    fn inline_action_body(
        &mut self,
        def: &ComponentDef<'a>,
        action: &crate::ast::component::ActionDef<'a>,
        args: &[&'a Expr<'a>],
        component: &str,
    ) -> Option<Vec<Stmt<'a>>> {
        self.lgx_html_slots += 1;
        let site = self.lgx_html_slots;
        let mut out: Vec<Stmt<'a>> = Vec::new();
        let mut scoped: Vec<Stmt<'a>> = Vec::new();
        for (i, ((param, ty), arg)) in action.params.iter().zip(args.iter()).enumerate() {
            let carried = self.interner.intern(&format!("__lgx_nest{site}_{i}"));
            out.push(Stmt::Let {
                var: carried,
                ty: Some(*ty),
                value: arg,
                mutable: false,
            });
            scoped.push(Stmt::Let {
                var: *param,
                ty: Some(*ty),
                value: self.te_id(carried),
                mutable: true,
            });
        }
        // A recursive step of this same walk, not a turn entry: the sibling's `Return`s are turned
        // into leaving the single-pass loop below by `nested_returns`, under a flag of their own —
        // the caller's turn keeps its own `protect_early_return` for the caller's `Return`s.
        let (flag, body) = match self.rewrite_store_calls(action.body, component) {
            Some(rewritten) => self.nested_returns(rewritten, site),
            None => return None,
        };
        out.extend(flag);
        scoped.extend(body);
        scoped.push(Stmt::Break);
        let name = self.interner.resolve(action.name).to_string();
        let instance = self.journal_instance(def);
        out.extend(self.open_turn(&name, instance));
        let (seq, capture) = self.journal_before(def, &format!("__lgx_nb{site}"));
        out.extend(capture);
        out.push(Stmt::While {
            cond: self.te_bool(true),
            body: self.alloc_block(scoped),
            decreasing: None,
        });
        out.extend(self.journal_changes(def, seq, &format!("__lgx_nc{site}"), seq));
        out.extend(self.close_turn());
        let instance = self.journal_instance(def);
        let blank = self.interner.intern("");
        let committed = self.te_text(blank);
        out.push(self.note_turn(&name, instance, self.te_id(seq), committed));
        Some(out)
    }

    /// An inlined sibling's body with each `Return` rewritten into `Break` out of the single-pass
    /// loop it is wrapped in, through a flag named for this call site (`__lgx_nest<site>_returned`),
    /// so a `Return` inside one of the sibling's own loops cascades out to that wrapper and no
    /// further. The flag's declaration comes back separately: it belongs outside the wrapper.
    fn nested_returns(
        &mut self,
        body: crate::ast::stmt::Block<'a>,
        site: usize,
    ) -> (Option<Stmt<'a>>, Vec<Stmt<'a>>) {
        if !Self::body_contains_return(body) {
            return (None, body.to_vec());
        }
        let returned = self.interner.intern(&format!("__lgx_nest{site}_returned"));
        let declared = Stmt::Let {
            var: returned,
            ty: None,
            value: self.te_bool(false),
            mutable: true,
        };
        (
            Some(declared),
            self.rewrite_returns_to_break(body.to_vec(), returned),
        )
    }

    /// A store's field types, or `None` when one cannot be named.
    ///
    /// A record needs a type per field, and `Let n be 0.` carries none — so the type comes from
    /// the annotation when there is one and from the initializer's literal otherwise. A field
    /// whose type this cannot name means the store is not lowered, which is the honest outcome:
    /// the alternative is guessing at a type and miscompiling the page that depends on it.
    pub(crate) fn store_field_types(
        &mut self,
        def: &ComponentDef<'a>,
    ) -> Option<Vec<(Symbol, Symbol, bool)>> {
        let mut fields = Vec::new();
        for field in def.state.iter().filter(|f| !f.derived) {
            fields.push((field.name, self.state_field_type_name(field)?, true));
        }
        Some(fields)
    }

    /// A state field's TYPE NAME — its annotation when it carries one, its initializer's literal
    /// otherwise, because `Let n be 0.` carries no annotation and that is how a field is ordinarily
    /// written.
    ///
    /// ★ ONE RULE, TWO ASKERS. A `## Store` needs a type per field to build its record, and
    /// `Load "k" into f` needs one to decode what `Save` wrote. Asked separately they answered
    /// differently — the `Load` half read only `field.ty` and so typed every unannotated field as
    /// Text, which made a saved `7` come back as the TEXT "7" and `seen + 1` concatenate to "71"
    /// instead of adding to 8. A field is now the same type wherever the compiler asks.
    pub(crate) fn state_field_type_name(&mut self, field: &StateField<'a>) -> Option<Symbol> {
        // A convergent field of a `shared on` store is, to everything that reads or writes it, the
        // plain value its CRDT converges to (`lgx_lower::shared`).
        if let Some(Ok(kind)) = field
            .ty
            .and_then(|ty| super::shared::shared_field_kind(ty, self.interner))
        {
            let carrier = super::shared::shared_carrier_name(kind, self.interner)?;
            return Some(self.interner.intern(&carrier));
        }
        if let Some(elem) = seq_field_element(field.ty, field.init, self.interner) {
            return Some(self.seq_state_type(elem));
        }
        match field.ty {
            Some(TypeExpr::Primitive(t) | TypeExpr::Named(t)) => Some(*t),
            Some(_) => None,
            None => match field.init {
                Expr::Literal(Literal::Number(_)) => Some(self.interner.intern("Int")),
                Expr::Literal(Literal::Text(_)) => Some(self.interner.intern("Text")),
                Expr::Literal(Literal::Boolean(_)) => Some(self.interner.intern("Bool")),
                Expr::Literal(Literal::Float(_)) => Some(self.interner.intern("Float")),
                _ => None,
            },
        }
    }

    /// `Seq of <element>`, named as ONE Symbol: the composite spelling `"Seq of <element>"`, which
    /// no LOGOS identifier can ever carry (an identifier holds no space), so it can never collide
    /// with a declared type's own name. This is how a field's FULL shape threads through
    /// `lgx_state_types` (`Vec<(Symbol, Option<Symbol>)>`, declared in `parser/mod.rs`) without
    /// widening that field's own type — every reader of it lives in this lane's own files
    /// (`stores.rs`, `elements.rs`), so the convention stays self-contained. See
    /// [`Self::seq_state_element`] for the inverse.
    pub(crate) fn seq_state_type(&mut self, elem: Symbol) -> Symbol {
        let name = format!("Seq of {}", self.interner.resolve(elem));
        self.interner.intern(&name)
    }

    /// The declared type a store action's local copy of a `Seq` field carries: `Seq of <element>`.
    /// A record keeps a `Seq` field's element type out of its layout, so the local is where an
    /// engine learns that walking the list yields `<element>`s — how `n's number` finds its slot
    /// in `Repeat for n in notes:`. `None` for a field that is not a `Seq`.
    pub(crate) fn seq_field_type_expr(&mut self, ty: Symbol) -> Option<&'a TypeExpr<'a>> {
        let elem = self.seq_state_element(ty)?;
        let scalar = matches!(
            self.interner.resolve(elem),
            "Int" | "Nat" | "Text" | "Bool" | "Real" | "Float" | "Unit"
        );
        let elem_ty = if scalar {
            TypeExpr::Primitive(elem)
        } else {
            TypeExpr::Named(elem)
        };
        Some(self.ctx.alloc_type_expr(TypeExpr::Generic {
            base: self.interner.intern("Seq"),
            params: self.ctx.alloc_type_exprs(vec![elem_ty]),
        }))
    }

    /// The inverse of [`Self::seq_state_type`]: the element's own Symbol, when `ty` names a `Seq`.
    pub(crate) fn seq_state_element(&mut self, ty: Symbol) -> Option<Symbol> {
        let full = self.interner.resolve(ty).to_string();
        let elem = full.strip_prefix("Seq of ")?;
        Some(self.interner.intern(elem))
    }

    /// Whether a field of type `ty` NEEDS the derived JSON codec at all — a `Seq` or a declared
    /// record — as opposed to `ty` simply being a plain scalar this file's fixed tags already
    /// carry. Kept SEPARATE from [`Self::json_codec_for`]'s own `None`, which instead means "needs
    /// the codec, and this file cannot actually build one" — a caller that conflated the two would
    /// fall back to the scalar tag for a Seq/record it cannot verify, handing `as_text` a value it
    /// cannot stringify: the exact silent-miscompile shape this codec exists to close.
    pub(crate) fn needs_json_codec(&mut self, ty: Symbol) -> bool {
        self.seq_state_element(ty).is_some()
            || (!storable_type(self.interner, ty) && self.json_shapes().declared_record(ty))
    }

    /// The `(encoder, decoder)` pair a field of type `ty` needs the derived JSON codec for, once
    /// [`Self::needs_json_codec`] has already said one is needed — `None` when this file cannot
    /// actually round-trip it (a record with a field the codec has no case for), which the caller
    /// must treat as a REFUSAL, never a fallback. Shared by `Save`/`Load` (`elements.rs`'s
    /// `tagged_value`/`tagged_read`) and a store's own persistence
    /// ([`Self::store_read_back`]/[`Self::store_write_through`]), so a field's shape resolves to
    /// the same codec wherever it is asked.
    pub(crate) fn json_codec_for(&mut self, ty: Symbol) -> Option<(Symbol, Symbol)> {
        if let Some(elem) = self.seq_state_element(ty) {
            return self.ensure_seq_codec(elem);
        }
        if !storable_type(self.interner, ty) && self.json_shapes().declared_record(ty) {
            return self.ensure_record_codec(ty);
        }
        None
    }

    /// The registry's own rich shape for a field's type-name Symbol — the composite `"Seq of X"`
    /// becomes the `FieldType::Generic` the VM's own boundary inference already reads for every
    /// ordinary declared record's `Seq` field; anything else keeps the flat mapping this file
    /// always gave the registry.
    pub(crate) fn registry_field_type(
        &mut self,
        ty: Symbol,
    ) -> crate::analysis::registry::FieldType {
        use crate::analysis::registry::FieldType;
        let (collection, elem) = match self.seq_state_element(ty) {
            Some(elem) => ("Seq", elem),
            None => match self
                .interner
                .resolve(ty)
                .strip_prefix("Set of ")
                .map(str::to_string)
            {
                Some(elem) => ("Set", self.interner.intern(&elem)),
                None => return FieldType::Primitive(ty),
            },
        };
        let base = self.interner.intern(collection);
        FieldType::Generic {
            base,
            params: vec![FieldType::Named(elem)],
        }
    }

    /// The flat type-name a `Stmt::StructDef` field carries when its real type is a `Seq` — its
    /// own BASE symbol (`"Seq"`), mirroring the exact degeneration the VM compiler's own
    /// registry-driven pass applies to the IDENTICAL field (`FieldType::Generic{base,..} =>
    /// *base`), so the two passes over one store's synthesized type can never disagree.
    pub(crate) fn flat_symbol_for_struct_def(&mut self, ty: Symbol) -> Symbol {
        if self.seq_state_element(ty).is_some() {
            return self.interner.intern("Seq");
        }
        if self.interner.resolve(ty).starts_with("Set of ") {
            return self.interner.intern("Set");
        }
        ty
    }

    /// A `## Store` as a RECORD TYPE, one instance of it, and a function per action.
    ///
    /// The instance is a module-level binding named after the store, so `Session's name` reads
    /// as the field access it already is — no expression anywhere needs rewriting, on either
    /// tier. Each action reads the fields into locals, runs, and writes them back at the END,
    /// which is the same rollback-by-construction a component's turn gets.
    pub(crate) fn lower_store(&mut self, def: &ComponentDef<'a>) -> Option<Vec<Stmt<'a>>> {
        if def.state.iter().any(|f| f.derived) {
            return None;
        }
        let fields = self.store_field_types(def)?;
        let store = self.interner.resolve(def.name).to_string();
        let type_name = self.interner.intern(&store_type_name(&store));
        // The record reaches the TYPE REGISTRY, not only the statement list, and it reaches it
        // from here alone: a `Stmt::StructDef` names a field's type by one flat word, and the
        // registry is where a `Seq of Text` field keeps its element. The engines read the
        // registry as the parse leaves it (`Parser::into_type_registry`), so the record they lay
        // a store out from is this one — every field typed by the rule that builds it, under
        // the name the author spelled, which no dictionary reads.
        let field_defs: Vec<crate::analysis::registry::FieldDef> = fields
            .iter()
            .map(
                |(name, ty, is_public)| crate::analysis::registry::FieldDef {
                    name: *name,
                    ty: self.registry_field_type(*ty),
                    is_public: *is_public,
                },
            )
            .collect();
        if let Some(registry) = self.type_registry.as_mut() {
            registry.register(
                type_name,
                crate::analysis::registry::TypeDef::Struct {
                    fields: field_defs,
                    generics: Vec::new(),
                    is_portable: false,
                    is_shared: false,
                },
            );
        }
        let struct_fields: Vec<(Symbol, Symbol, bool)> = fields
            .iter()
            .map(|(name, ty, is_public)| (*name, self.flat_symbol_for_struct_def(*ty), *is_public))
            .collect();
        let mut out = vec![Stmt::StructDef {
            name: type_name,
            fields: struct_fields,
            is_portable: false,
        }];
        let init_fields: Vec<(Symbol, &'a Expr<'a>)> = def
            .state
            .iter()
            .filter(|f| !f.derived)
            .map(|f| (f.name, f.init))
            .collect();
        let value = self.ctx.alloc_imperative_expr(Expr::New {
            type_name,
            type_args: Vec::new(),
            init_fields,
        });
        out.push(Stmt::Let {
            var: def.name,
            ty: None,
            value,
            mutable: true,
        });
        // ★ A STORE THAT SAYS IT PERSISTS, PERSISTS. Its fields are read back out of browser
        // storage over the initializers that just ran — that is what "persisted" means, and it
        // is what this tier did not do: the state lived in the module's own memory and died with
        // the page, while the sentence claimed otherwise.
        // A SHARED store's fields come from its topic instead: stamped frames, joined as they
        // arrive (`lgx_lower::shared`).
        match store_area(def) {
            Some(SHARED_AREA) => out.extend(self.shared_store_prelude(def, &fields)?),
            Some(area) => out.extend(self.store_read_back(def, area, &fields)?),
            None => {}
        }
        for action in &def.actions {
            let action_name = self.interner.resolve(action.name).to_string();
            let instance = self.journal_instance(def);
            let mut body: Vec<Stmt<'a>> = self.open_turn(&action_name, instance);
            let base = self.te_id(def.name);
            for (name, ty, _) in &fields {
                let read = self.ctx.alloc_imperative_expr(Expr::FieldAccess {
                    object: base,
                    field: *name,
                });
                let declared = self.seq_field_type_expr(*ty);
                body.push(Stmt::Let {
                    var: *name,
                    ty: declared,
                    value: read,
                    mutable: true,
                });
            }
            let (seq, capture) = self.journal_before(def, "__lgx_jb");
            body.extend(capture);
            let store_name = self.interner.resolve(def.name).to_string();
            let inner = self.rewrite_store_calls(action.body, &store_name)?;
            let inner = self.protect_early_return(inner.to_vec());
            body.extend(inner.iter().cloned());
            body.extend(self.store_moved_notes(def));
            for (name, _, _) in &fields {
                body.push(Stmt::SetField {
                    object: base,
                    field: *name,
                    value: self.te_id(*name),
                });
            }
            let dirty = self.interner.intern(stores_dirty_binding());
            body.push(Stmt::Set {
                target: dirty,
                value: self.te_bool(true),
            });
            // Noted here — right after the write-back, by the action's own name (never the
            // store's), before persisting or waking anything this action's fields moved.
            body.extend(self.journal_changes(def, seq, "__lgx_jc", seq));
            body.extend(self.close_turn());
            let instance = self.journal_instance(def);
            let blank = self.interner.intern("");
            let committed = self.te_text(blank);
            body.push(self.note_turn(&action_name, instance, self.te_id(seq), committed));
            // The write-through goes AFTER the write-back, so a trap partway through the action
            // never reaches either: what is stored is what the turn committed, and rollback by
            // construction covers storage too.
            match store_area(def) {
                Some(SHARED_AREA) => body.extend(self.shared_store_publish(def, &fields)?),
                Some(area) => body.extend(self.store_write_through(def, area, &fields)?),
                None => {}
            }
            let fname = self
                .interner
                .intern(&action_fn_name(&store, self.interner.resolve(action.name)));
            out.push(Stmt::FunctionDef {
                name: fname,
                generics: Vec::new(),
                params: action.params.clone(),
                body: self.alloc_block(body),
                return_type: None,
                is_native: false,
                native_path: None,
                is_exported: false,
                export_target: None,
                opt_flags: crate::optimization::OptimizationConfig::all_on(),
            });
        }
        Some(out)
    }

    /// `storageGet(area, "<key>.<field>")` over each field, written back over the initializer
    /// when the key is there.
    ///
    /// ★ ONE KEY PER FIELD, and it is not a micro-optimization. A record encoded into one string
    /// needs a separator, a separator needs escaping, and an escaping bug corrupts a visitor's
    /// saved draft silently. A field stored on its own is delimited by the storage API itself, so
    /// no byte is special. It also gives the RELOAD semantics for free: a field the edited page
    /// no longer declares is never read, and one it has just added is simply absent, so it starts
    /// from its initializer. Neither needs a case of its own.
    ///
    /// A `Seq`/record field round-trips through the derived JSON codec instead: the fallback
    /// text is the field's own CURRENT (initializer) value, JSON-encoded, so a missing key decodes
    /// back to exactly what was already there; a malformed stored value leaves the field
    /// untouched rather than trapping the whole read-back. `None` only if `json_codec_for`
    /// disagrees with the gate that already verified this field lowers — never reached in
    /// practice, but propagated as a refusal rather than risking a call to a codec that does not
    /// exist.
    pub(crate) fn store_read_back(
        &mut self,
        def: &ComponentDef<'a>,
        area: &'static str,
        fields: &[(Symbol, Symbol, bool)],
    ) -> Option<Vec<Stmt<'a>>> {
        let get = self.interner.intern("storageGetOr");
        let base = self.te_id(def.name);
        let mut out = Vec::new();
        for (name, ty, _) in fields {
            let current = self.ctx.alloc_imperative_expr(Expr::FieldAccess {
                object: base,
                field: *name,
            });
            if self.needs_json_codec(*ty) {
                let (enc, dec) = self.json_codec_for(*ty)?;
                let fallback = self.te_call(enc, vec![current]);
                let key = self.storage_field_key(def, *name);
                let area_arg = self.storage_area(area);
                let call = self.te_call(get, vec![area_arg, key, fallback]);
                let raw = self
                    .interner
                    .intern(&format!("__stored_{}", self.interner.resolve(*name)));
                out.push(Stmt::Let {
                    var: raw,
                    ty: None,
                    value: call,
                    mutable: false,
                });
                let decoded = self.te_call(dec, vec![self.te_id(raw)]);
                let err = self.interner.intern("__lgx_store_read_err");
                let no_op = Stmt::Splice {
                    body: self.alloc_block(Vec::new()),
                };
                let ok_bind = self.interner.intern("__lgx_store_read_v");
                let set = Stmt::SetField {
                    object: base,
                    field: *name,
                    value: self.te_id(ok_bind),
                };
                let inspect = self.inspect_result(
                    decoded,
                    super::json::ResultShape::Result,
                    err,
                    no_op,
                    ok_bind,
                    vec![set],
                );
                out.push(inspect);
                continue;
            }
            // ★ `storageGetOr`, NOT `storageGet` — no Option crosses this boundary. On the
            // WebAssembly tier an `Option of Text` is a boxed handle whose presence can be
            // tested but whose value cannot yet be taken out, and a read-back that could not
            // USE what it read would be a store that persists and never restores. Handing the
            // absence its meaning here — the field's own initializer, already computed — needs
            // no Option at all, and says what a read-back means in one place.
            let raw = self
                .interner
                .intern(&format!("__stored_{}", self.interner.resolve(*name)));
            let fallback = self.as_text(current);
            let args = vec![
                self.storage_area(area),
                self.storage_field_key(def, *name),
                fallback,
            ];
            let call = self.ctx.alloc_imperative_expr(Expr::Call {
                function: get,
                args,
            });
            out.push(Stmt::Let {
                var: raw,
                ty: None,
                value: call,
                mutable: false,
            });
            let value = self.storage_parse(raw, *ty);
            out.push(Stmt::SetField {
                object: base,
                field: *name,
                value,
            });
        }
        Some(out)
    }

    /// `storageSet(area, "<key>.<field>", "{field}")` over each field, plus the INDEX under the
    /// store's own key naming the fields that were written. A `Seq`/record field is encoded
    /// through the derived JSON codec instead of `as_text` — see [`Self::store_read_back`] for
    /// the read half and why `None` here is a refusal, never a silent fallback.
    pub(crate) fn store_write_through(
        &mut self,
        def: &ComponentDef<'a>,
        area: &'static str,
        fields: &[(Symbol, Symbol, bool)],
    ) -> Option<Vec<Stmt<'a>>> {
        let set = self.interner.intern("storageSet");
        let mut out = Vec::new();
        let mut names = Vec::new();
        for (name, ty, _) in fields {
            let value = if self.needs_json_codec(*ty) {
                let (enc, _) = self.json_codec_for(*ty)?;
                self.te_call(enc, vec![self.te_id(*name)])
            } else {
                self.as_text(self.te_id(*name))
            };
            let key = self.storage_field_key(def, *name);
            let args = vec![self.storage_area(area), key, value];
            out.push(Stmt::Call {
                function: set,
                args,
            });
            names.push(self.interner.resolve(*name).to_string());
        }
        // A SPACE joins the index, because a field name is an identifier and cannot contain one
        // — so the index needs no escaping either, and both tiers spell it the same way.
        let index = self.interner.intern(&names.join(" "));
        let index_text = self.te_text(index);
        let key = self.storage_key(def);
        let args = vec![self.storage_area(area), key, index_text];
        out.push(Stmt::Call {
            function: set,
            args,
        });
        Some(out)
    }

    /// Record this component's state fields and their types, for `Load … into <field>`.
    ///
    /// Types come from [`Self::state_field_type_name`], the same rule `store_field_types` applies,
    /// so a field cannot be an Int to one half of the compiler and a Text to the other.
    pub(crate) fn note_state_types(&mut self, def: &ComponentDef<'a>) {
        self.lgx_fetch_fails = def
            .events
            .iter()
            .enumerate()
            .find_map(|(i, h)| match h.trigger {
                Trigger::FetchFails { url, error } => Some((i, url, error)),
                _ => None,
            });
        let mut out = Vec::with_capacity(def.state.len());
        for field in def.state.iter() {
            let ty = self.state_field_type_name(field);
            out.push((field.name, ty));
        }
        self.lgx_state_types = out;
    }
}
