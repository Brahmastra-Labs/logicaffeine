use super::sites::split_keyed_listener_event;
use super::*;
use crate::ast::component::{ComponentDef, Trigger};
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use crate::ast::view::BoundInput;
use crate::lgx::html::EventModifier;
use logicaffeine_base::Symbol;

/// One attached listener, as the dispatcher needs to call it back.
///
/// ★ IT CARRIES WHAT TO CALL, NOT A CLOSURE. A module cannot hold a listener closure on the browser
/// tier — `Map of Text to fn(…)` is refused by that backend ("unsupported indirect call"), measured
/// rather than assumed. But the dispatcher never needed the closure: it needs to know WHICH handler
/// and FOR WHICH INSTANCE, and both of those are data. The closure stays where it belongs, on the
/// element, for the browser to call; this is the parallel record a driving verb resolves through.
#[derive(Clone)]
pub enum ListenerSite {
    /// `on-<event>={action}` and `When "<sel>" is <event>:` — run that action's turn.
    ///
    /// `key` is the KEY MODIFIER this listener waits for (`on-keydown.enter` → `enter`), or
    /// `None` for a handler that answers any press. It is compared case-insensitively against
    /// the key the dispatch carries, because a browser reports `KeyboardEvent.key` as `"Enter"`
    /// while the author writes `.enter` — the reason the two ends meet at all.
    Action {
        component: Symbol,
        action: Symbol,
        argc: usize,
        key: Option<Symbol>,
        /// Whether the handler's LAST parameter is what the event CARRIED rather than something
        /// captured where it was rendered. A dropped file's name is not state and not a prop — it
        /// arrives with the event — so the arm reads the dispatcher's value slot for it.
        carries_value: bool,
        /// For an `on-<event>.once` handler, the render's local holding the listener's identity
        /// (see [`once_spent_binding`]); the registration records it against the listener key and
        /// the arm spends it.
        once: Option<Symbol>,
    },
    /// `bind={field}` — store the delivered value, then repaint.
    Bind {
        component: Symbol,
        field: Symbol,
        checked: bool,
    },
    /// A click on a bound checkbox or radio. Not a listener the page wrote: it is the BROWSER's
    /// default action, which happens before any handler runs, and an in-process host performs it
    /// itself (`page_host`'s `Driver::toggle`). Registered so the node-keyed lookup finds it like
    /// any other.
    DefaultToggle {
        component: Symbol,
        field: Symbol,
        radio: bool,
    },
    /// `on-<event>={<callbackProp>}` — a component's own element calling a callback it was HANDED
    /// rather than one of its own actions (`docs/frontend.md`'s "Passing actions down"). The arm
    /// calls the prop's trampoline (`Parser::callback_trampoline`) for the registering instance,
    /// with the `argc` arguments the render recorded; `key` is the key modifier, as on
    /// [`Self::Action`].
    CallbackProp {
        component: Symbol,
        prop: Symbol,
        argc: usize,
        key: Option<Symbol>,
        once: Option<Symbol>,
    },
    /// A `scroll` the page heard where `When the user scrolls to the bottom:` waits: the arm asks
    /// the geometry how far the scrolled node is from its end, and runs the handler's turn only
    /// within 32px of it — the threshold is compiled LOGOS, the host only measures.
    ScrollNear { component: Symbol, action: Symbol },
}

impl ListenerSite {
    /// The render-local `.once` identity this site records at registration, if it is `.once`.
    pub(crate) fn once_local(&self) -> Option<Symbol> {
        match self {
            ListenerSite::Action { once, .. } | ListenerSite::CallbackProp { once, .. } => *once,
            ListenerSite::Bind { .. }
            | ListenerSite::DefaultToggle { .. }
            | ListenerSite::ScrollNear { .. } => None,
        }
    }

    /// The one key this site answers, when it waits for one.
    pub(crate) fn waits_for(&self) -> Option<Symbol> {
        match self {
            ListenerSite::Action { key, .. } | ListenerSite::CallbackProp { key, .. } => *key,
            ListenerSite::Bind { .. }
            | ListenerSite::DefaultToggle { .. }
            | ListenerSite::ScrollNear { .. } => None,
        }
    }
}

/// The drag events a `When a file is dropped on "<sel>":` target cancels at registration. A browser
/// makes an element a drop target only while `dragenter` and `dragover` are cancelled on it, and
/// performs its own default for an uncancelled `drop` (it opens the file in the tab).
pub const DROP_TARGET_CANCELS: &[&str] = &["dragenter", "dragover", "drop"];

/// A callback's TAG: the position of `(component, action)` among every action the program declares,
/// component by component in declaration order. One global numbering, so the placement that records
/// a tag and the trampoline that reads it back agree without either knowing the other's signature.
pub(crate) fn action_tag(
    all: &[ComponentDef<'_>],
    component: Symbol,
    action: Symbol,
) -> Option<i64> {
    all.iter()
        .flat_map(|c| c.actions.iter().map(move |a| (c.name, a.name)))
        .position(|(c, a)| c == component && a == action)
        .map(|i| i as i64)
}

/// Every `(tag, component, action)` among `owners` whose parameters match `signature` — the arms of
/// a callback trampoline. An action of another shape cannot be what the callback means: the
/// placement gate refuses binding one.
pub(crate) fn callback_candidates<'a>(
    all: &[ComponentDef<'a>],
    owners: &[Symbol],
    signature: &[&'a TypeExpr<'a>],
) -> Vec<(i64, Symbol, Symbol)> {
    let mut out = Vec::new();
    let mut tag = 0i64;
    for c in all {
        for a in &c.actions {
            let fits = owners.contains(&c.name)
                && a.params.len() == signature.len()
                && a.params
                    .iter()
                    .zip(signature.iter())
                    .all(|((_, p), s)| param_shapes_match(p, s));
            if fits {
                out.push((tag, c.name, a.name));
            }
            tag += 1;
        }
    }
    out
}

impl<'a, 'ctx, 'int> super::super::Parser<'a, 'ctx, 'int> {
    /// The identity a placement records for each callback prop it hands down, against the CHILD's
    /// instance key: which action the prop means (its [`action_tag`]) and whose instance that action
    /// runs against. A value naming one of `owner`'s actions records that action and `owner`'s own
    /// key; a value forwarding one of `owner`'s own callback props copies through the identity
    /// `owner` was itself handed. `callback_binding_refusal` has already refused every other value.
    pub(crate) fn callback_prop_registration(
        &mut self,
        value: &'a Expr<'a>,
        owner: &ComponentDef<'a>,
        child: Symbol,
        prop: Symbol,
        child_key: &'a Expr<'a>,
        all: &[ComponentDef<'a>],
    ) -> Vec<Stmt<'a>> {
        let Some(child_def) = all.iter().find(|d| d.name == child) else {
            return Vec::new();
        };
        let Some((_, ty)) = child_def.props.iter().find(|(p, _)| *p == prop).copied() else {
            return Vec::new();
        };
        if !is_callback_type(ty, self.interner) {
            return Vec::new();
        }
        let (tag_expr, key_expr) = match callback_source(value, owner, self.interner) {
            Some(CallbackSource::Action(action)) => {
                let Some(tag) = action_tag(all, owner.name, action.name) else {
                    return Vec::new();
                };
                (self.te_int(tag), self.key_expr())
            }
            Some(CallbackSource::Forwarded(own)) => {
                let oname = self.interner.resolve(owner.name).to_string();
                let pname = self.interner.resolve(own).to_string();
                (
                    self.callback_identity_read(&oname, &pname, true),
                    self.callback_identity_read(&oname, &pname, false),
                )
            }
            None => return Vec::new(),
        };
        let component = self.interner.resolve(child).to_string();
        let p = self.interner.resolve(prop).to_string();
        let tag_map = self.interner.intern(&callback_tag_binding(&component, &p));
        let key_map = self.interner.intern(&callback_key_binding(&component, &p));
        vec![
            Stmt::SetIndex {
                collection: self.te_id(tag_map),
                index: child_key,
                value: tag_expr,
            },
            Stmt::SetIndex {
                collection: self.te_id(key_map),
                index: child_key,
                value: key_expr,
            },
        ]
    }

    /// `item <this render's own key> of __P_<Component>_<prop>_TAG` (or `_KEY`) — this instance's
    /// own recorded identity for one of its callback props. `want_tag` selects which of the pair.
    fn callback_identity_read(
        &mut self,
        component: &str,
        prop: &str,
        want_tag: bool,
    ) -> &'a Expr<'a> {
        let name = if want_tag {
            callback_tag_binding(component, prop)
        } else {
            callback_key_binding(component, prop)
        };
        let global = self.interner.intern(&name);
        let key = self.key_expr();
        self.ctx.alloc_imperative_expr(Expr::Index {
            collection: self.te_id(global),
            index: key,
        })
    }

    /// Every module-level binding `def`'s callback props need: the identity pair for each one
    /// (`__P_<C>_<prop>_TAG`/`_KEY`, keyed by instance), and — for a prop an element of `def` calls
    /// as a handler — one map per parameter, keyed by listener site, where the render records the
    /// arguments it evaluated ([`callback_arg_binding`], the twin of [`listener_arg_binding`]).
    ///
    /// Declared whether or not `def` is `stateful`: a component with no `State:` and no `Actions:`
    /// still calls the callbacks it was handed (`docs/frontend.md`'s "Passing actions down").
    pub(crate) fn callback_prop_declarations(
        &mut self,
        def: &ComponentDef<'a>,
        all: &[ComponentDef<'a>],
    ) -> Vec<Stmt<'a>> {
        let mut out = Vec::new();
        let component = self.interner.resolve(def.name).to_string();
        let map_of = self.interner.intern("mapOf");
        let root = self.interner.intern("");
        for (prop, _) in callback_props(def, self.interner) {
            let p = self.interner.resolve(prop).to_string();
            let tag_var = self.interner.intern(&callback_tag_binding(&component, &p));
            let tag_value = self.te_call(map_of, vec![self.te_text(root), self.te_int(0)]);
            out.push(Stmt::Let {
                var: tag_var,
                ty: None,
                value: tag_value,
                mutable: true,
            });
            let key_var = self.interner.intern(&callback_key_binding(&component, &p));
            let key_value = self.te_call(map_of, vec![self.te_text(root), self.te_text(root)]);
            out.push(Stmt::Let {
                var: key_var,
                ty: None,
                value: key_value,
                mutable: true,
            });
        }
        for (var, ty) in self.callback_arg_maps(def, all) {
            let seed = self.zero_of_type(ty);
            let value = self.te_call(map_of, vec![self.te_int(0), seed]);
            out.push(Stmt::Let {
                var,
                ty: None,
                value,
                mutable: true,
            });
        }
        out
    }

    /// The `_TAG`/`_KEY` identity maps of every callback prop `def` declares — keyed by instance, for
    /// `sweep_gone` to retire a gone instance's entries from, beside its state and prop maps.
    pub(crate) fn callback_prop_identity_map_names(
        &mut self,
        def: &ComponentDef<'a>,
    ) -> Vec<Symbol> {
        let mut out = Vec::new();
        let component = self.interner.resolve(def.name).to_string();
        for (prop, _) in callback_props(def, self.interner) {
            let p = self.interner.resolve(prop).to_string();
            out.push(self.interner.intern(&callback_tag_binding(&component, &p)));
            out.push(self.interner.intern(&callback_key_binding(&component, &p)));
        }
        out
    }

    /// `__ARG_<C>_<prop>_cb<i>` for every parameter of every callback prop `def`'s own elements call
    /// as a handler, with the parameter's type. Keyed by LISTENER SITE (an `Int`), like
    /// [`listener_arg_binding`]'s maps — so `lower_component_render`'s retire loop removes a stale
    /// site's entries from these exactly as it does from an action's.
    pub(crate) fn callback_arg_maps(
        &mut self,
        def: &ComponentDef<'a>,
        all: &[ComponentDef<'a>],
    ) -> Vec<(Symbol, &'a TypeExpr<'a>)> {
        let mut out = Vec::new();
        let component = self.interner.resolve(def.name).to_string();
        for prop in callback_prop_targets(def) {
            let Some(signature) = callback_signature(all, def.name, prop, self.interner) else {
                continue;
            };
            let p = self.interner.resolve(prop).to_string();
            for (i, ty) in signature.into_iter().enumerate() {
                out.push((
                    self.interner
                        .intern(&callback_arg_binding(&component, &p, i)),
                    ty,
                ));
            }
        }
        out
    }

    /// `__cb_<C>_<prop>(key, a…)` — run whatever action instance `key`'s callback prop `prop` means.
    ///
    /// ★ ONE CALL PATH FOR EVERY CALLER. A browser's listener, a driving verb's dispatch and a turn
    /// that calls the prop all arrive here with the instance key: the identity the placement
    /// recorded names the action by its tag and the instance it runs against by key, and an arm per
    /// candidate calls that action's turn. No closure is ever handed to a render, stored in a map, or
    /// found at run time — which is what lets one child be placed with two different actions, repaint
    /// with its callbacks intact, and build for the browser. An instance with no recorded identity
    /// runs nothing.
    pub(crate) fn callback_trampoline(
        &mut self,
        def: &ComponentDef<'a>,
        prop: Symbol,
        all: &[ComponentDef<'a>],
        owners: &[Symbol],
    ) -> Stmt<'a> {
        let component = self.interner.resolve(def.name).to_string();
        let p = self.interner.resolve(prop).to_string();
        let signature = callback_signature(all, def.name, prop, self.interner).unwrap_or_default();
        let key = self.interner.intern(instance_key_param());
        let tag_map = self.interner.intern(&callback_tag_binding(&component, &p));
        let key_map = self.interner.intern(&callback_key_binding(&component, &p));
        let known = self.ctx.alloc_imperative_expr(Expr::Contains {
            collection: self.te_id(tag_map),
            value: self.te_id(key),
        });
        let unknown = self.ctx.alloc_imperative_expr(Expr::Not { operand: known });
        let mut body = vec![Stmt::If {
            cond: unknown,
            then_block: self.alloc_block(vec![Stmt::Return { value: None }]),
            else_block: None,
        }];
        let tag_local = self.interner.intern("__lgx_cbtag");
        let tag_read = self.ctx.alloc_imperative_expr(Expr::Index {
            collection: self.te_id(tag_map),
            index: self.te_id(key),
        });
        body.push(Stmt::Let {
            var: tag_local,
            ty: None,
            value: tag_read,
            mutable: false,
        });
        let owner_local = self.interner.intern("__lgx_cbkey");
        let owner_read = self.ctx.alloc_imperative_expr(Expr::Index {
            collection: self.te_id(key_map),
            index: self.te_id(key),
        });
        body.push(Stmt::Let {
            var: owner_local,
            ty: None,
            value: owner_read,
            mutable: false,
        });
        let mut params = vec![(key, self.text_type())];
        let mut call_args = vec![self.te_id(owner_local)];
        for (i, ty) in signature.iter().enumerate() {
            let name = self.interner.intern(&format!("__lgx_cb{i}"));
            params.push((name, *ty));
            call_args.push(self.te_id(name));
        }
        let mut chain: Vec<Stmt<'a>> = Vec::new();
        for (tag, owner_c, owner_a) in callback_candidates(all, owners, &signature)
            .into_iter()
            .rev()
        {
            let oc = self.interner.resolve(owner_c).to_string();
            let oa = self.interner.resolve(owner_a).to_string();
            let act = self.interner.intern(&action_fn_name(&oc, &oa));
            let cond = self.te_bin(BinaryOpKind::Eq, self.te_id(tag_local), self.te_int(tag));
            let run = self.alloc_block(vec![Stmt::Call {
                function: act,
                args: call_args.clone(),
            }]);
            let otherwise = if chain.is_empty() {
                None
            } else {
                Some(self.alloc_block(chain))
            };
            chain = vec![Stmt::If {
                cond,
                then_block: run,
                else_block: otherwise,
            }];
        }
        body.extend(chain);
        let name = self
            .interner
            .intern(&callback_trampoline_fn_name(&component, &p));
        Stmt::FunctionDef {
            name,
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

    /// The VALUE of `component`'s callback prop `prop` for the instance in scope — a closure over
    /// that prop's trampoline, bound to this instance's key. What a turn binds the prop's name to,
    /// so an action body reads a callback prop the way it reads any other.
    pub(crate) fn callback_invoker(
        &mut self,
        def: &ComponentDef<'a>,
        prop: Symbol,
    ) -> &'a Expr<'a> {
        let all = self.lgx_all_components.clone();
        let signature = callback_signature(&all, def.name, prop, self.interner).unwrap_or_default();
        let component = self.interner.resolve(def.name).to_string();
        let p = self.interner.resolve(prop).to_string();
        let trampoline = self
            .interner
            .intern(&callback_trampoline_fn_name(&component, &p));
        let mut params = Vec::new();
        let mut args = vec![self.key_expr()];
        for (i, ty) in signature.into_iter().enumerate() {
            let name = self.interner.intern(&format!("__lgx_cb{i}"));
            params.push((name, ty));
            args.push(self.te_id(name));
        }
        let body = self.alloc_block(vec![Stmt::Call {
            function: trampoline,
            args,
        }]);
        self.ctx.alloc_imperative_expr(Expr::Closure {
            params,
            body: crate::ast::stmt::ClosureBody::Block(body),
            return_type: None,
        })
    }

    /// `__lgx_dispatch(selector, event, value) -> Bool` — did anything answer that act? Emitted in
    /// two halves: this one finds the element, and `__lgx_dispatch_node(node, event, value)` —
    /// pushed beside it — raises the event on the element itself, the half a host holding the
    /// element calls directly.
    ///
    /// ★★ IT RESOLVES THE NODE AND LOOKS IT UP. It used to be a table of STATIC arms, one per
    /// `(selector text, event)` the program literally wrote, and the walk that built them emitted an
    /// arm only where the element carried a LITERAL `id`. So a driving verb could not reach an
    /// element identified by a class, or by a computed id — which is every row of every list — and a
    /// page the browser runs perfectly could not be driven by a test.
    ///
    /// ★ AND ONCE A COMPONENT COULD BE PLACED TWICE IT WAS WORSE THAN INCOMPLETE. A selector is a
    /// way to FIND an element, not an identity: two rows rendered from one component carry the same
    /// markup, so a selector cannot say which was clicked. The static table resolved to the
    /// COMPONENT and ran against the ROOT instance — finding the element, picking the wrong record,
    /// and painting. A loud gap had rotted into a silent wrong answer because a feature landed
    /// beneath it, with nothing edited and nothing turning red.
    ///
    /// Now: resolve the selector to a NODE at run time, look `"{node}:{event}"` up in the table each
    /// listener registered as it was attached, and run the handler it names against the instance key
    /// recorded beside it. Matching by the element itself answers both questions at once.
    ///
    /// ★ NO NEW BUILTIN AND NO HOST IMPORT. The old premise — this tier cannot call a closure it
    /// found at run time — is TRUE and stays true: a `Map of Text to fn(…)` is refused by the wasm
    /// backend, measured. But it never needed the closure. It needed to know WHICH handler and FOR
    /// WHICH INSTANCE, and both of those are data.
    pub(crate) fn lower_lgx_dispatcher(&mut self, all: &[ComponentDef<'a>]) -> Option<Stmt<'a>> {
        // An empty listener set still yields a dispatcher — one that answers `false` to
        // everything, which is the truth and is callable. See the note at the emission site.
        if self.lgx_components.is_empty() {
            return None;
        }
        if all
            .iter()
            .any(|d| self.lgx_components.contains(&d.name) && has_any_listener_site(d))
        {
            let forget = self.forget_listener_fn(all);
            self.deferred_functions.push(forget);
        }
        // ★ WHAT EVERY PAINT CALLS AFTER ITS COMMIT, emitted once: the commit itself, the fragment
        // registry and the sweep of the instances a commit removed, and the drain of the directives
        // a paint drew.
        let commit = self.lower_lgx_commit();
        self.deferred_functions.push(commit);
        if self.instances_can_leave() {
            self.declare_frag_registry();
            let own = self.lower_lgx_own_frag();
            self.deferred_functions.push(own);
            let gone = self.lower_lgx_gone();
            self.deferred_functions.push(gone);
        }
        if !self.drawn_directives().is_empty() {
            self.declare_directive_queues();
            let drain = self.lower_lgx_drain_directives();
            self.deferred_functions.push(drain);
        }
        let selector = self.interner.intern("__lgx_sel");
        let event = self.interner.intern("__lgx_ev");
        let node = self.interner.intern("__lgx_node");
        let lookup = self.interner.intern("__lgx_lk");
        let id = self.interner.intern("__lgx_id");
        let key = self.interner.intern(instance_key_param());
        let mut body: Vec<Stmt<'a>> = Vec::new();

        if uses_outbox(all) {
            body.push(self.outbox_flush_arm(event));
        }

        // ★ A MESSAGE IS ONE-TO-MANY AND THE TABLE BELOW IS ONE-TO-ONE. Every other event this
        // dispatcher routes resolves an ELEMENT, and an element has one handler for one event, so
        // the `(node, event) -> listener` table below holds exactly one id and one instance key.
        // A subscription is the opposite shape by definition: every live instance of every
        // component that named the topic is meant to hear it. Registering them all into a
        // one-slot table meant the LAST one registered answered and the rest heard nothing — two
        // `<Tile/>`s in a list, one `Publish`, and only the second reset. It needed two instances
        // to show, so every single-instance parity test passed while the feature was half-built.
        //
        // The fan-out belongs HERE rather than at `Publish`, because `Raise "message:<topic>"`
        // from a `## Test` arrives the same way and is owed the same delivery. The browser's two
        // halves already fan out on their own — `addEventListener` keeps a list, and the relay
        // keeps `subs` per topic — so this is the in-page half catching up with them, not a new
        // rule.
        //
        // It runs BEFORE the table lookup and returns, so a subscription never consults the slot
        // it shares; and it walks `__KEYS_<C>`, which the unmount sweep keeps to exactly the
        // instances on the page, so an instance that has left is not delivered to — skipping only
        // the seed key, whose stamp is `-1`.
        let val = self.lgx_value_param();
        let mut topics: Vec<(String, Vec<(Symbol, usize)>)> = Vec::new();
        for d in all.iter().filter(|d| keeps_instance_registry(d)) {
            for (index, handler) in d.events.iter().enumerate() {
                let Trigger::Message { topic, .. } = &handler.trigger else {
                    continue;
                };
                let Expr::Literal(Literal::Text(t)) = topic else {
                    continue;
                };
                let name = self.interner.resolve(*t).to_string();
                match topics.iter_mut().find(|(n, _)| *n == name) {
                    Some((_, subs)) => subs.push((d.name, index)),
                    None => topics.push((name, vec![(d.name, index)])),
                }
            }
        }
        for (topic, subs) in topics {
            let ev_name = self.interner.intern(&format!("message:{topic}"));
            let is_it = self.te_bin(BinaryOpKind::Eq, self.te_id(event), self.te_text(ev_name));
            let mut fan: Vec<Stmt<'a>> = Vec::new();
            for (component, index) in subs {
                let name = self.interner.resolve(component).to_string();
                let keys = self.interner.intern(&keys_binding(&name));
                let seen = self.interner.intern(&seen_binding(&name));
                let k = self.interner.intern("__lgx_mk");
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
                    args: vec![self.te_id(k), self.te_id(val)],
                };
                let then_block = self.alloc_block(vec![call]);
                let inner = self.alloc_block(vec![Stmt::If {
                    cond: live,
                    then_block,
                    else_block: None,
                }]);
                fan.push(Stmt::Repeat {
                    pattern: crate::ast::stmt::Pattern::Identifier(k),
                    iterable: self.te_id(keys),
                    body: inner,
                });
            }
            fan.push(Stmt::Return {
                value: Some(self.te_bool(true)),
            });
            body.push(Stmt::If {
                cond: is_it,
                then_block: self.alloc_block(fan),
                else_block: None,
            });
        }

        // ★ A SELECTOR FINDS THE ELEMENT; THE EVENT HAPPENS TO THE ELEMENT. Everything from here on
        // is the node half, `__lgx_dispatch_node(node, event, value)`, which a host that already
        // holds the element — the one a focus change blurred or focused — calls directly. This half
        // resolves the selector and hands the node over.
        let query = self.interner.intern("querySelector");
        let dispatch_node = self.interner.intern(dispatch_node_fn_name());
        let found = self.te_call(query, vec![self.te_id(selector)]);
        let carried = self.lgx_value_param();
        let handed = self.te_call(
            dispatch_node,
            vec![found, self.te_id(event), self.te_id(carried)],
        );
        body.push(Stmt::Return {
            value: Some(handed),
        });
        let selector_body = std::mem::take(&mut body);
        // ★ THE EVENT NAME BECOMES ITS CODE, ONCE. Registration built this key from a name known
        // at compile time; dispatch has only the runtime name, so it is turned back into a code by
        // a chain over exactly the events this program registers. Once per dispatch — against once
        // per listener per paint, which is what building a `"{node}:{event}"` string cost.
        //
        // ★ A LISTENER WAITING FOR ONE KEY HAS A CODE OF ITS OWN (`keyed_listener_event`), so one
        // element can wait for Enter and for Escape at once: each keyed listener is its own table
        // entry, found by the event AND the key the host delivered. A press answers the listener
        // waiting for its key, and then the one waiting for any key — both, as every listener the
        // press reaches runs.
        let code = self.interner.intern("__lgx_code");
        let keyed = self.interner.intern("__lgx_keyed");
        body.push(Stmt::Let {
            var: code,
            ty: None,
            value: self.te_int(-1),
            mutable: true,
        });
        body.push(Stmt::Let {
            var: keyed,
            ty: None,
            value: self.te_int(-1),
            mutable: true,
        });
        let value = self.lgx_value_param();
        let lower = self.interner.intern("lower");
        for (i, entry) in self.lgx_listener_events.clone().iter().enumerate() {
            let entry = self.interner.resolve(*entry).to_string();
            let (cond, target) = match split_keyed_listener_event(&entry) {
                Some((name, want)) => {
                    let name = self.interner.intern(name);
                    let want = self.interner.intern(want);
                    let is_event =
                        self.te_bin(BinaryOpKind::Eq, self.te_id(event), self.te_text(name));
                    let pressed = self.te_call(lower, vec![self.te_id(value)]);
                    let is_key = self.te_bin(BinaryOpKind::Eq, pressed, self.te_text(want));
                    (self.te_bin(BinaryOpKind::And, is_event, is_key), keyed)
                }
                None => {
                    let name = self.interner.intern(&entry);
                    (
                        self.te_bin(BinaryOpKind::Eq, self.te_id(event), self.te_text(name)),
                        code,
                    )
                }
            };
            let set = Stmt::Set {
                target,
                value: self.te_int(i as i64),
            };
            body.push(Stmt::If {
                cond,
                then_block: self.alloc_block(vec![set]),
                else_block: None,
            });
        }
        let answer = self.interner.intern(answer_fn_name());
        let answered = self.interner.intern("__lgx_answered");
        body.push(Stmt::Let {
            var: answered,
            ty: None,
            value: self.te_bool(false),
            mutable: true,
        });
        // ★ EVERY LISTENER THE EVENT REACHES, IN THE ORDER THEY REGISTERED: on the keyed slot and
        // then the plain one, the listener the slot names and then each that spilled after it. The
        // whole queue is read before any of them runs, as a browser fixes an event's listeners when
        // it starts delivering it: a listener whose turn redraws the element leaves the rest of
        // this delivery as it found it, and one that turn retired answers nothing.
        //
        // ★ THE SLOT SPACE IS THE PROGRAM'S OWN ENTRY COUNT, known here, where every entry is: a
        // slot `site * __LISTENER_SPACE + code` never reaches the next site's slots however many
        // events and keys the program listens for. Declared FIRST among the deferred statements, so
        // it precedes every function that reads it.
        //
        // ★ THE HOST NAMES THE NODE; THE LISTENERS ARE KEYED BY ITS SITE. The retained tree knows
        // the node by the id the host does (`vdNodeOfWire`) and the site it carries (`vdSiteOf`) —
        // the site every listener on it registered against — and a node with no site answers
        // nothing.
        if !self.lgx_listener_events.is_empty() {
            let space = self.interner.intern(listener_space_binding());
            let entries = self.te_int(self.lgx_listener_events.len() as i64);
            self.deferred_functions.insert(
                0,
                Stmt::Let {
                    var: space,
                    ty: None,
                    value: entries,
                    mutable: false,
                },
            );
            let more = self.interner.intern(listener_spill_binding());
            let queue = self.interner.intern("__lgx_queue");
            let int_name = self.interner.intern("Int");
            let mut queued: Vec<Stmt<'a>> = Vec::new();
            queued.push(Stmt::Let {
                var: queue,
                ty: None,
                value: self.new_seq_expr(int_name),
                mutable: true,
            });
            let anchor = self.interner.intern("__lgx_anchor");
            let vnode = self.vd_expr("vdNodeOfWire", vec![self.te_id(node)]);
            let site = self.vd_expr("vdSiteOf", vec![vnode]);
            body.push(Stmt::Let {
                var: anchor,
                ty: None,
                value: site,
                mutable: false,
            });
            let plain_slot = self.interner.intern("__lgx_cs");
            let scaled = self.te_bin(
                BinaryOpKind::Multiply,
                self.te_id(anchor),
                self.te_id(space),
            );
            let composed = self.te_bin(BinaryOpKind::Add, scaled, self.te_id(code));
            body.push(Stmt::Let {
                var: plain_slot,
                ty: None,
                value: composed,
                mutable: false,
            });
            for (which, slot, spilled) in [
                (keyed, "__lgx_ks", "__lgx_ksp"),
                (code, "__lgx_cs", "__lgx_csp"),
            ] {
                let slot = self.interner.intern(slot);
                let spilled = self.interner.intern(spilled);
                let scaled = self.te_bin(
                    BinaryOpKind::Multiply,
                    self.te_id(anchor),
                    self.te_id(space),
                );
                let composed = self.te_bin(BinaryOpKind::Add, scaled, self.te_id(which));
                let has_more = self.ctx.alloc_imperative_expr(Expr::Contains {
                    collection: self.te_id(more),
                    value: self.te_id(slot),
                });
                let after = self.ctx.alloc_imperative_expr(Expr::Index {
                    collection: self.te_id(more),
                    index: self.te_id(slot),
                });
                let each_after = Stmt::Repeat {
                    pattern: crate::ast::stmt::Pattern::Identifier(spilled),
                    iterable: after,
                    body: self.alloc_block(vec![Stmt::Push {
                        value: self.te_id(spilled),
                        collection: self.te_id(queue),
                    }]),
                };
                let mut enqueue = Vec::new();
                if slot != plain_slot {
                    enqueue.push(Stmt::Let {
                        var: slot,
                        ty: None,
                        value: composed,
                        mutable: false,
                    });
                }
                enqueue.push(Stmt::Push {
                    value: self.te_id(slot),
                    collection: self.te_id(queue),
                });
                enqueue.push(Stmt::If {
                    cond: has_more,
                    then_block: self.alloc_block(vec![each_after]),
                    else_block: None,
                });
                let registered = self.te_bin(BinaryOpKind::GtEq, self.te_id(which), self.te_int(0));
                queued.push(Stmt::If {
                    cond: registered,
                    then_block: self.alloc_block(enqueue),
                    else_block: None,
                });
            }
            let each = self.interner.intern("__lgx_each");
            let held = self.interner.intern(listener_held_binding());
            let left = self.interner.intern(listener_left_binding());
            let holds = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(held),
                index: self.te_id(each),
            });
            let already = self.ctx.alloc_imperative_expr(Expr::Contains {
                collection: self.te_id(held),
                value: self.te_id(each),
            });
            let one_more = self.te_bin(BinaryOpKind::Add, holds, self.te_int(1));
            let hold = Stmt::If {
                cond: already,
                then_block: self.alloc_block(vec![Stmt::SetIndex {
                    collection: self.te_id(held),
                    index: self.te_id(each),
                    value: one_more,
                }]),
                else_block: Some(self.alloc_block(vec![Stmt::SetIndex {
                    collection: self.te_id(held),
                    index: self.te_id(each),
                    value: self.te_int(1),
                }])),
            };
            queued.push(Stmt::Repeat {
                pattern: crate::ast::stmt::Pattern::Identifier(each),
                iterable: self.te_id(queue),
                body: self.alloc_block(vec![hold]),
            });
            let asked = self.te_call(
                answer,
                vec![self.te_id(each), self.te_id(value), self.te_id(node)],
            );
            let either = self.te_bin(BinaryOpKind::Or, asked, self.te_id(answered));
            queued.push(Stmt::Repeat {
                pattern: crate::ast::stmt::Pattern::Identifier(each),
                iterable: self.te_id(queue),
                body: self.alloc_block(vec![Stmt::Set {
                    target: answered,
                    value: either,
                }]),
            });
            // Let go of each lookup; the last holder of one retired meanwhile forgets it now.
            let forget = self.interner.intern(forget_listener_fn_name());
            let remaining = self.interner.intern("__lgx_holds");
            let holds = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(held),
                index: self.te_id(each),
            });
            let fewer = self.te_bin(BinaryOpKind::Subtract, holds, self.te_int(1));
            let was_left = self.ctx.alloc_imperative_expr(Expr::Contains {
                collection: self.te_id(left),
                value: self.te_id(each),
            });
            let last = self.te_bin(BinaryOpKind::Eq, self.te_id(remaining), self.te_int(0));
            let release = vec![
                Stmt::Let {
                    var: remaining,
                    ty: None,
                    value: fewer,
                    mutable: false,
                },
                Stmt::If {
                    cond: last,
                    then_block: self.alloc_block(vec![
                        Stmt::Remove {
                            value: self.te_id(each),
                            collection: self.te_id(held),
                        },
                        Stmt::If {
                            cond: was_left,
                            then_block: self.alloc_block(vec![
                                Stmt::Remove {
                                    value: self.te_id(each),
                                    collection: self.te_id(left),
                                },
                                Stmt::Call {
                                    function: forget,
                                    args: vec![self.te_id(each)],
                                },
                            ]),
                            else_block: None,
                        },
                    ]),
                    else_block: Some(self.alloc_block(vec![Stmt::SetIndex {
                        collection: self.te_id(held),
                        index: self.te_id(each),
                        value: self.te_id(remaining),
                    }])),
                },
            ];
            queued.push(Stmt::Repeat {
                pattern: crate::ast::stmt::Pattern::Identifier(each),
                iterable: self.te_id(queue),
                body: self.alloc_block(release),
            });
            // ★ ONE LISTENER NEEDS NO QUEUE. With no listener waiting for a key of this event and
            // nothing spilled on the plain slot, the slot's own listener is the whole delivery: it
            // is answered directly, and a click — nearly every event a page hears — allocates
            // nothing here.
            let unkeyed = self.te_bin(BinaryOpKind::Lt, self.te_id(keyed), self.te_int(0));
            let spilled_here = self.ctx.alloc_imperative_expr(Expr::Contains {
                collection: self.te_id(more),
                value: self.te_id(plain_slot),
            });
            let alone = self.ctx.alloc_imperative_expr(Expr::Not {
                operand: spilled_here,
            });
            let single = self.te_bin(BinaryOpKind::And, unkeyed, alone);
            let asked = self.te_call(
                answer,
                vec![self.te_id(plain_slot), self.te_id(value), self.te_id(node)],
            );
            let registered = self.te_bin(BinaryOpKind::GtEq, self.te_id(code), self.te_int(0));
            let direct = Stmt::If {
                cond: registered,
                then_block: self.alloc_block(vec![Stmt::Set {
                    target: answered,
                    value: asked,
                }]),
                else_block: None,
            };
            body.push(Stmt::If {
                cond: single,
                then_block: self.alloc_block(vec![direct]),
                else_block: Some(self.alloc_block(queued)),
            });
        }
        body.push(Stmt::Return {
            value: Some(self.te_id(answered)),
        });

        // `__lgx_answer(lookup, value) -> Bool` — the listener one table entry names, run for the
        // instance that registered it; `false` when the entry names nothing.
        let mut arms: Vec<Stmt<'a>> = Vec::new();
        // Nothing registered for this element and event: the page does not answer that act.
        // Reported as a VALUE rather than a failure, so the caller can try the second event a user
        // action raises before deciding the page answers neither.
        let fns = self.interner.intern(listener_fn_binding());
        let known = self.ctx.alloc_imperative_expr(Expr::Contains {
            collection: self.te_id(fns),
            value: self.te_id(lookup),
        });
        let unknown = self.ctx.alloc_imperative_expr(Expr::Not { operand: known });
        arms.push(Stmt::If {
            cond: unknown,
            then_block: self.alloc_block(vec![Stmt::Return {
                value: Some(self.te_bool(false)),
            }]),
            else_block: None,
        });
        let read_id = self.ctx.alloc_imperative_expr(Expr::Index {
            collection: self.te_id(fns),
            index: self.te_id(lookup),
        });
        arms.push(Stmt::Let {
            var: id,
            ty: None,
            value: read_id,
            mutable: false,
        });
        let keys = self.interner.intern(listener_key_binding());
        let read_key = self.ctx.alloc_imperative_expr(Expr::Index {
            collection: self.te_id(keys),
            index: self.te_id(lookup),
        });
        // ★ BOUND AS `__lgx_key`, so every helper below — `state_write`, `repaint_def`, the action
        // call — acts on the instance that registered the listener, with no special casing.
        arms.push(Stmt::Let {
            var: key,
            ty: None,
            value: read_key,
            mutable: false,
        });
        for (which, site) in self.lgx_listeners.clone().iter().enumerate() {
            let arm = self.dispatch_site_arm(which, site);
            arms.push(arm);
        }
        arms.push(Stmt::Return {
            value: Some(self.te_bool(false)),
        });

        let text_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Text")));
        let int_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Int")));
        let bool_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Bool")));
        let node_half = Stmt::FunctionDef {
            name: dispatch_node,
            generics: Vec::new(),
            params: vec![
                (node, int_ty),
                (event, text_ty),
                (self.lgx_value_param(), text_ty),
            ],
            body: self.alloc_block(body),
            return_type: Some(bool_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        };
        self.deferred_functions.push(node_half);
        let value = self.lgx_value_param();
        self.deferred_functions.push(Stmt::FunctionDef {
            name: answer,
            generics: Vec::new(),
            params: vec![(lookup, int_ty), (value, text_ty), (node, int_ty)],
            body: self.alloc_block(arms),
            return_type: Some(bool_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        });
        let name = self.interner.intern(dispatch_fn_name());
        Some(Stmt::FunctionDef {
            name,
            generics: Vec::new(),
            params: vec![
                (selector, text_ty),
                (event, text_ty),
                (self.lgx_value_param(), text_ty),
            ],
            body: self.alloc_block(selector_body),
            return_type: Some(bool_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        })
    }

    /// `If event is "__lgx_outbox": deliver everything queued; Return true.` — the flush a turn's
    /// epilogue asks for once it has committed (`queue_element_event`).
    ///
    /// The outbox is TAKEN before anything is delivered: each delivery runs another turn, which may
    /// queue events of its own and flush them at its own epilogue, so this walks a snapshot while
    /// the lists themselves start empty again for whatever the deliveries queue.
    fn outbox_flush_arm(&mut self, event: Symbol) -> Stmt<'a> {
        let flush = self.interner.intern(outbox_flush_event());
        let is_flush = self.te_bin(BinaryOpKind::Eq, self.te_id(event), self.te_text(flush));
        let mut arm: Vec<Stmt<'a>> = Vec::new();
        let count = self.interner.intern(outbox_count_binding());
        let taken_count = self.interner.intern("__lgx_on");
        arm.push(Stmt::Let {
            var: taken_count,
            ty: None,
            value: self.te_id(count),
            mutable: false,
        });
        arm.push(Stmt::Set {
            target: count,
            value: self.te_int(0),
        });
        let mut taken = Vec::new();
        for (list, local) in [
            (outbox_selector_binding(), "__lgx_os"),
            (outbox_event_binding(), "__lgx_oe"),
            (outbox_value_binding(), "__lgx_ov"),
        ] {
            let list = self.interner.intern(list);
            let local = self.interner.intern(local);
            arm.push(Stmt::Let {
                var: local,
                ty: None,
                value: self.te_id(list),
                mutable: false,
            });
            let empty = self.empty_outbox_list();
            arm.push(Stmt::Set {
                target: list,
                value: empty,
            });
            taken.push(local);
        }
        let at = self.interner.intern("__lgx_oi");
        arm.push(Stmt::Let {
            var: at,
            ty: None,
            value: self.te_int(1),
            mutable: true,
        });
        let entry: Vec<&'a Expr<'a>> = taken
            .iter()
            .map(|list| {
                self.ctx.alloc_imperative_expr(Expr::Index {
                    collection: self.te_id(*list),
                    index: self.te_id(at),
                })
            })
            .collect();
        let dispatch = self.interner.intern(dispatch_fn_name());
        let step = self.te_bin(BinaryOpKind::Add, self.te_id(at), self.te_int(1));
        let deliver = self.alloc_block(vec![
            Stmt::Call {
                function: dispatch,
                args: entry,
            },
            Stmt::Set {
                target: at,
                value: step,
            },
        ]);
        let more = self.te_bin(BinaryOpKind::LtEq, self.te_id(at), self.te_id(taken_count));
        arm.push(Stmt::While {
            cond: more,
            body: deliver,
            decreasing: None,
        });
        arm.push(Stmt::Return {
            value: Some(self.te_bool(true)),
        });
        Stmt::If {
            cond: is_flush,
            then_block: self.alloc_block(arm),
            else_block: None,
        }
    }

    /// The closure a host invokes for a registration the dispatcher cannot answer from the node
    /// alone — a window event, a relay topic, a route change, a timer, a drop whose file the host
    /// reads, a scroll whose geometry decides whether it counts. It takes the EVENT.
    ///
    /// ★ IT MUST TAKE ONE PARAMETER. A host calls it as `__logos_call_closure(handle, ev)`, so a
    /// zero-parameter closure is an arity mismatch that TRAPS the moment the host fires it.
    pub(crate) fn listener_closure(&mut self, call: Stmt<'a>) -> &'a Expr<'a> {
        self.host_callback_closure(vec![call])
    }

    /// [`Self::listener_closure`], for a body of more than one statement (a fetch completion's
    /// decode-then-dispatch, say) — the SAME single-`Int`-parameter shape either way. Every closure
    /// this lowering hands to a callback-registering host import (`onRoute`, a window event, a
    /// timer, a fetch completion) is built by one of these two, never `Expr::Closure` directly: the
    /// wasm trampoline (`__logos_call_closure`) only dispatches a closure taking exactly one `Int`,
    /// and a bare literal builder one line away from this one is exactly how the route-change
    /// closure went missing its param and trapped every module that fired it.
    pub(crate) fn host_callback_closure(&mut self, call: Vec<Stmt<'a>>) -> &'a Expr<'a> {
        let ev_param = self.interner.intern("__lgx_event");
        let int_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Int")));
        let body = self.alloc_block(call);
        self.ctx.alloc_imperative_expr(Expr::Closure {
            params: vec![(ev_param, int_ty)],
            body: crate::ast::stmt::ClosureBody::Block(body),
            return_type: None,
        })
    }

    /// `listenFlags(<node>, "<event>", <bits>)` — the host is told only that `node` answers `event`
    /// and how the event is dispatched there (`EventModifier::host_listen_bits`). What answers it is
    /// the entry this paint wrote into the listener table, which `__lgx_dispatch_node` reads when
    /// the host hands the event over, so a browser's event and a driving verb run the same arm.
    pub(crate) fn listen_flags_call(
        &mut self,
        node: &'a Expr<'a>,
        event: Symbol,
        bits: u8,
    ) -> Stmt<'a> {
        let flagged = self.interner.intern("listenFlags");
        let event_text = self.te_text(event);
        Stmt::Call {
            function: flagged,
            args: vec![node, event_text, self.te_int(i64::from(bits))],
        }
    }

    /// The key an `on-keydown.enter={…}` directive waits for, as a browser reports it
    /// (`KeyboardEvent.key`), or `None` for a handler that answers any key. Read from the
    /// event-modifier catalogue, the one every reader of a handler's qualifiers reads.
    pub(crate) fn handler_key(&mut self, modifiers: &[EventModifier]) -> Option<Symbol> {
        EventModifier::key_of(modifiers).map(|k| self.interner.intern(&k))
    }

    /// `If __lgx_id is <which>: <run it>; Return true.` — one registered listener.
    pub(crate) fn dispatch_site_arm(&mut self, which: usize, site: &ListenerSite) -> Stmt<'a> {
        let id = self.interner.intern("__lgx_id");
        let cond = self.te_bin(BinaryOpKind::Eq, self.te_id(id), self.te_int(which as i64));
        let mut run: Vec<Stmt<'a>> = Vec::new();
        match site {
            ListenerSite::Action {
                component,
                action,
                argc,
                carries_value,
                once,
                ..
            } => {
                if once.is_some() {
                    run.extend(self.dispatch_once_gate(*component));
                }
                let name = self.interner.resolve(*component).to_string();
                let a = self.interner.resolve(*action).to_string();
                let act = self.interner.intern(&action_fn_name(&name, &a));
                let lookup = self.interner.intern("__lgx_lk");
                let mut call_args = vec![self.key_expr()];
                for i in 0..*argc {
                    let var = self.interner.intern(&listener_arg_binding(&name, &a, i));
                    call_args.push(self.ctx.alloc_imperative_expr(Expr::Index {
                        collection: self.te_id(var),
                        index: self.te_id(lookup),
                    }));
                }
                if *carries_value {
                    let carried = self.lgx_value_param();
                    call_args.push(self.te_id(carried));
                }
                run.push(Stmt::Call {
                    function: act,
                    args: call_args,
                });
            }
            // The write-back function does the typing itself (`bound_value` recovers a Bool from
            // the Text the host delivers), so the arm hands over the raw value exactly as the
            // browser's own listener does. One conversion, one place.
            ListenerSite::Bind {
                component,
                field,
                checked: _,
            } => {
                let name = self.interner.resolve(*component).to_string();
                let write = self
                    .interner
                    .intern(&bind_fn_name(&name, self.interner.resolve(*field)));
                let value = self.lgx_value_param();
                let key = self.key_expr();
                run.push(Stmt::Call {
                    function: write,
                    args: vec![key, self.te_id(value)],
                });
            }
            // ★ THE BROWSER'S OWN ACT, THEN THE BIND TURN. A click moves the box — a checkbox flips,
            // a radio is selected — and the browser then raises `input`, which is what the bound
            // field's write-back answers, carrying `"true"`/`"false"`; so the arm hands the new
            // state to the same `__bind_<C>_<field>` turn a keystroke runs: written back, watched,
            // journaled `bind`.
            ListenerSite::DefaultToggle {
                component,
                field,
                radio,
            } => {
                let name = self.interner.resolve(*component).to_string();
                let write = self
                    .interner
                    .intern(&bind_fn_name(&name, self.interner.resolve(*field)));
                let yes = self.interner.intern("true");
                let now = self.interner.intern("__lgx_now");
                run.push(Stmt::Let {
                    var: now,
                    ty: None,
                    value: self.te_text(yes),
                    mutable: true,
                });
                if !*radio {
                    let no = self.interner.intern("false");
                    let current = self.state_read(&name, *field);
                    let flip = Stmt::Set {
                        target: now,
                        value: self.te_text(no),
                    };
                    run.push(Stmt::If {
                        cond: current,
                        then_block: self.alloc_block(vec![flip]),
                        else_block: None,
                    });
                }
                let key = self.key_expr();
                run.push(Stmt::Call {
                    function: write,
                    args: vec![key, self.te_id(now)],
                });
            }
            // The trampoline resolves the identity the placement recorded; an instance with none
            // recorded is a site nothing answers, reported as `false` like any unregistered one.
            ListenerSite::CallbackProp {
                component,
                prop,
                argc,
                once,
                ..
            } => {
                let cname = self.interner.resolve(*component).to_string();
                let pname = self.interner.resolve(*prop).to_string();
                let tag_map = self.interner.intern(&callback_tag_binding(&cname, &pname));
                let this_key = self.key_expr();
                let has_identity = self.ctx.alloc_imperative_expr(Expr::Contains {
                    collection: self.te_id(tag_map),
                    value: this_key,
                });
                let missing = self.ctx.alloc_imperative_expr(Expr::Not {
                    operand: has_identity,
                });
                run.push(Stmt::If {
                    cond: missing,
                    then_block: self.alloc_block(vec![Stmt::Return {
                        value: Some(self.te_bool(false)),
                    }]),
                    else_block: None,
                });
                if once.is_some() {
                    run.extend(self.dispatch_once_gate(*component));
                }
                let lookup = self.interner.intern("__lgx_lk");
                let mut call_args = vec![self.key_expr()];
                for i in 0..*argc {
                    let var = self
                        .interner
                        .intern(&callback_arg_binding(&cname, &pname, i));
                    call_args.push(self.ctx.alloc_imperative_expr(Expr::Index {
                        collection: self.te_id(var),
                        index: self.te_id(lookup),
                    }));
                }
                let trampoline = self
                    .interner
                    .intern(&callback_trampoline_fn_name(&cname, &pname));
                run.push(Stmt::Call {
                    function: trampoline,
                    args: call_args,
                });
            }
            ListenerSite::ScrollNear { component, action } => {
                let name = self.interner.resolve(*component).to_string();
                let a = self.interner.resolve(*action).to_string();
                let act = self.interner.intern(&action_fn_name(&name, &a));
                let node = self.interner.intern("__lgx_node");
                let gap_of = self.interner.intern("scrollGap");
                let gap = self.te_call(gap_of, vec![self.te_id(node)]);
                let far = self.te_bin(BinaryOpKind::Gt, gap, self.te_int(32));
                run.push(Stmt::If {
                    cond: far,
                    then_block: self.alloc_block(vec![Stmt::Return {
                        value: Some(self.te_bool(false)),
                    }]),
                    else_block: None,
                });
                run.push(Stmt::Call {
                    function: act,
                    args: vec![self.key_expr()],
                });
            }
        }
        run.push(Stmt::Return {
            value: Some(self.te_bool(true)),
        });
        Stmt::If {
            cond,
            then_block: self.alloc_block(run),
            else_block: None,
        }
    }

    /// `Let id be item __lgx_lk of __LISTENER_ONCE. If __LGX_ONCE contains id: Return false.` and
    /// then spend it — a `.once` listener answers the first event that passes its key gate, and
    /// every later one finds nothing to answer it, exactly as a spent browser listener does.
    fn dispatch_once_gate(&mut self, component: Symbol) -> Vec<Stmt<'a>> {
        let id = self.interner.intern("__lgx_oid");
        let lookup = self.interner.intern("__lgx_lk");
        let ids = self.interner.intern(listener_once_binding());
        let read = self.ctx.alloc_imperative_expr(Expr::Index {
            collection: self.te_id(ids),
            index: self.te_id(lookup),
        });
        let mut out = vec![Stmt::Let {
            var: id,
            ty: None,
            value: read,
            mutable: false,
        }];
        out.push(self.once_spent_guard(id));
        out.extend(self.spend_once(component, id));
        out
    }

    /// `If __LGX_ONCE contains <id>: Return false.` — a spent `.once` listener answers nothing.
    pub(crate) fn once_spent_guard(&mut self, id: Symbol) -> Stmt<'a> {
        let spent = self.interner.intern(once_spent_binding());
        let is_spent = self.ctx.alloc_imperative_expr(Expr::Contains {
            collection: self.te_id(spent),
            value: self.te_id(id),
        });
        Stmt::If {
            cond: is_spent,
            then_block: self.alloc_block(vec![Stmt::Return {
                value: Some(self.te_bool(false)),
            }]),
            else_block: None,
        }
    }

    /// Mark the `.once` listener `id` spent, and — for a component whose instances can leave the
    /// page — record it against the instance in scope, so the unmount sweep forgets it.
    pub(crate) fn spend_once(&mut self, component: Symbol, id: Symbol) -> Vec<Stmt<'a>> {
        let spent = self.interner.intern(once_spent_binding());
        let mut out = vec![Stmt::SetIndex {
            collection: self.te_id(spent),
            index: self.te_id(id),
            value: self.te_bool(true),
        }];
        if !self.once_recorded(component) {
            return out;
        }
        let name = self.interner.resolve(component).to_string();
        let record = self.interner.intern(&once_record_binding(&name));
        let list = self.interner.intern("__lgx_ol");
        let text = self.interner.intern("Text");
        let fresh = self.new_seq_expr(text);
        out.push(Stmt::Let {
            var: list,
            ty: None,
            value: fresh,
            mutable: true,
        });
        let key = self.key_expr();
        let known = self.ctx.alloc_imperative_expr(Expr::Contains {
            collection: self.te_id(record),
            value: key,
        });
        let key = self.key_expr();
        let held = self.ctx.alloc_imperative_expr(Expr::Index {
            collection: self.te_id(record),
            index: key,
        });
        out.push(Stmt::If {
            cond: known,
            then_block: self.alloc_block(vec![Stmt::Set {
                target: list,
                value: held,
            }]),
            else_block: None,
        });
        out.push(Stmt::Push {
            value: self.te_id(id),
            collection: self.te_id(list),
        });
        let key = self.key_expr();
        out.push(Stmt::SetIndex {
            collection: self.te_id(record),
            index: key,
            value: self.te_id(list),
        });
        out
    }

    /// Whether `component` keeps a per-instance record of the `.once` listeners it spent: it has
    /// one, and its instances can leave the page for the sweep to forget them.
    pub(crate) fn once_recorded(&self, component: Symbol) -> bool {
        self.lgx_all_components
            .iter()
            .any(|d| d.name == component && has_once_handler(d))
            && unmountable_components(&self.lgx_all_components).contains(&component)
    }

    /// The dispatcher's third parameter: what the user typed, for a `bind` to store.
    pub(crate) fn lgx_value_param(&mut self) -> Symbol {
        self.interner.intern("__lgx_val")
    }

    pub(crate) fn te_bool(&self, b: bool) -> &'a Expr<'a> {
        self.ctx
            .alloc_imperative_expr(Expr::Literal(Literal::Boolean(b)))
    }

    /// `__lgx_raise(event)` — a WINDOW event, delivered to the page.
    ///
    /// ★ IT ALWAYS REPORTS "ANSWERED", and that is the difference from every other driver. A click
    /// on an element nothing listens to is a test naming the wrong element; going offline when the
    /// page does not care about the network is an ordinary thing that happens, and failing there
    /// would make a test that sets the scene before mounting anything impossible to write.
    ///
    /// There is no missing-element check either: the page always exists.
    pub(crate) fn lower_lgx_raise(&mut self) -> Stmt<'a> {
        let selector = self.interner.intern("__lgx_sel");
        let event = self.interner.intern("__lgx_event");
        // The value slot the dispatcher already has — what a `Raise … with "photo.png".` carries,
        // and empty for the events that bring nothing.
        let carried = self.lgx_value_param();
        let dispatch = self.interner.intern(dispatch_fn_name());
        let two = self.te_int(2);
        let body = vec![
            Stmt::Call {
                function: dispatch,
                args: vec![self.te_id(selector), self.te_id(event), self.te_id(carried)],
            },
            Stmt::Return { value: Some(two) },
        ];
        self.lgx_driver_fn(raise_fn_name(), vec![selector, event, carried], body)
    }

    pub(crate) fn lgx_driver_fn(
        &mut self,
        name: &str,
        params: Vec<Symbol>,
        body: Vec<Stmt<'a>>,
    ) -> Stmt<'a> {
        let text_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Text")));
        let int_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Int")));
        let name = self.interner.intern(name);
        Stmt::FunctionDef {
            name,
            generics: Vec::new(),
            params: params.into_iter().map(|p| (p, text_ty)).collect(),
            body: self.alloc_block(body),
            return_type: Some(int_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        }
    }

    /// `__bind_<Component>_<field>(v)` — store the keystroke, then repaint.
    pub(crate) fn lower_bind_writeback(
        &mut self,
        def: &ComponentDef<'a>,
        field: Symbol,
        kind: BoundInput,
    ) -> Option<Stmt<'a>> {
        let component = self.interner.resolve(def.name).to_string();
        let param = self.interner.intern("__lgx_typed");
        // A checkbox arrives as the TEXT of its state, because the host has one event-value
        // channel; storing that text straight into a Bool field would make `"false"` truthy.
        let stored = self.bound_value(kind, param);
        // ★ A KEYSTROKE IS A TURN. Read state in, set the field, write state back, note "bind",
        // patch or repaint — the shape `turn_body_with` is the one home of, and the one place a `When <field> changes:`
        // watcher runs, so typing into a bound field wakes the sentence watching it.
        let set = Stmt::Set {
            target: field,
            value: stored,
        };
        let body = self.turn_body_with(def, self.alloc_block(vec![set]), true, Some("bind"))?;
        let text_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Text")));
        let name = self
            .interner
            .intern(&bind_fn_name(&component, self.interner.resolve(field)));
        Some(Stmt::FunctionDef {
            name,
            generics: Vec::new(),
            // WHICH INSTANCE first, then the delivered value — the same shape every turn takes, so
            // a keystroke routed through a driving verb and one from a real listener land in the
            // same record.
            params: vec![
                (self.interner.intern(instance_key_param()), text_ty),
                (param, text_ty),
            ],
            body,
            return_type: None,
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        })
    }

    /// What a bound control's reported state MEANS as a value. The host has one event-value
    /// channel and it carries Text, so a checkbox reports `"true"`/`"false"` and the Bool is
    /// recovered here, in the write-back every route into a bound field goes through.
    pub(crate) fn bound_value(&mut self, kind: BoundInput, param: Symbol) -> &'a Expr<'a> {
        match kind {
            k if k.writes_checked() => {
                let t = self.interner.intern("true");
                self.te_bin(BinaryOpKind::Eq, self.te_id(param), self.te_text(t))
            }
            _ => self.te_id(param),
        }
    }
}
