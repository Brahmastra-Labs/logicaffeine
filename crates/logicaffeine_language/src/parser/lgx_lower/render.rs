use super::*;
use crate::ast::component::{ComponentDef, Trigger};
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use crate::ast::view::{bound_input_kind, Attr, BoundInput, ViewNode};
use crate::lgx::html::{Behaviour, EventModifier};
use logicaffeine_base::Symbol;

/// The `Style:` class an element a render draws carries: its component's own, or — while a
/// `-> View` template is drawn — the one the placing component handed the drawing.
#[derive(Clone)]
pub(crate) enum ElementScope {
    Class(String),
    Param(Symbol),
}

/// Where the lowering walk is in the view tree — the bytecode tier's twin of the interpreter's
/// `Trail`, and it must number children the same way or the two tiers address different nodes.
#[derive(Clone)]
pub(crate) struct LowerTrail {
    prefix: String,
    opaque: bool,
    /// Inside a `{for each}`, the local holding this iteration's ROW — the row's `keyed by` value
    /// (or, unkeyed, its position) joined after every enclosing loop's row, so a row is told apart
    /// from every other row of every loop around it.
    ///
    /// ★ A STATIC PATH IS NOT ENOUGH INSIDE A LOOP. Every iteration walks the same nodes, so every
    /// iteration computes the same `npath`; two rows would share one instance key and therefore one
    /// state record, which is the aliasing the component model exists to prevent. The row is what
    /// makes them different, so it travels with the walk — and it carries the OUTER rows too, since
    /// the first item of a second list and the first item of a first list are different rows.
    row: Option<Symbol>,
    /// Drawn to be serialized, not painted (`LowerTrail::serving`): the walk writes the records that
    /// make the markup and nothing a live document needs — no site, instance, fragment, listener,
    /// directive or anchor — and places a child by its server render (`__serve_<C>`).
    pub(crate) serving: bool,
    /// While a served component draws its view, the parameter holding how deep it is — a child it
    /// places is one deeper.
    serve_depth: Option<Symbol>,
}

impl LowerTrail {
    pub(crate) fn root() -> Self {
        LowerTrail {
            prefix: String::new(),
            opaque: false,
            row: None,
            serving: false,
            serve_depth: None,
        }
    }
    pub(crate) fn opaque() -> Self {
        LowerTrail {
            prefix: String::new(),
            opaque: true,
            row: None,
            serving: false,
            serve_depth: None,
        }
    }
    /// The walk of a view drawn only to be serialized — a server render, a `Head:`.
    pub(crate) fn serving() -> Self {
        LowerTrail {
            prefix: String::new(),
            opaque: true,
            row: None,
            serving: true,
            serve_depth: None,
        }
    }
    /// The walk of a served component's view, `depth` (a parameter of its `__serve_`) deep.
    pub(crate) fn serving_at(depth: Symbol) -> Self {
        LowerTrail {
            prefix: String::new(),
            opaque: true,
            row: None,
            serving: true,
            serve_depth: Some(depth),
        }
    }
    pub(crate) fn child(&self, npath: &str) -> Self {
        LowerTrail {
            prefix: format!("{npath}."),
            opaque: self.opaque,
            row: self.row,
            serving: self.serving,
            serve_depth: self.serve_depth,
        }
    }
    pub(crate) fn branch(&self, prefix: &str) -> Self {
        LowerTrail {
            prefix: prefix.to_string(),
            opaque: true,
            row: self.row,
            serving: self.serving,
            serve_depth: self.serve_depth,
        }
    }
    /// The body of a `{for each}`, whose rows are told apart by the local `row`.
    pub(crate) fn in_loop(&self, prefix: &str, row: Symbol) -> Self {
        LowerTrail {
            prefix: prefix.to_string(),
            opaque: true,
            row: Some(row),
            serving: self.serving,
            serve_depth: self.serve_depth,
        }
    }
    pub(crate) fn npath(&self, index: usize) -> String {
        format!("{}{index}", self.prefix)
    }
}

/// The layout a view extends, when its first node says so. `{extends L}` must come first:
/// everything after it is the blocks that fill the layout.
pub(crate) fn extended_layout(nodes: &[ViewNode<'_>]) -> Option<Symbol> {
    match nodes.first() {
        Some(ViewNode::Extends { layout, .. }) => Some(*layout),
        _ => None,
    }
}

/// Whether an instance of `def` gives any element a site a listener or directive finds it by after
/// the commit.
pub(crate) fn needs_anchors(def: &ComponentDef<'_>) -> bool {
    fn directed(nodes: &[ViewNode<'_>]) -> bool {
        nodes.iter().any(|n| match n {
            ViewNode::Element {
                attrs, children, ..
            } => attrs.iter().any(|a| matches!(a, Attr::Directive { .. })) || directed(children),
            ViewNode::Each { body, empty, .. } => directed(body) || directed(empty),
            ViewNode::If {
                arms, otherwise, ..
            } => {
                arms.iter().any(|(_, b)| directed(b))
                    || otherwise.as_ref().is_some_and(|o| directed(o))
            }
            ViewNode::Inspect { arms, .. } => arms.iter().any(|a| directed(&a.body)),
            ViewNode::Component { children, .. } => directed(children),
            ViewNode::Block {
                default: Some(b), ..
            } => directed(b),
            _ => false,
        })
    }
    has_any_listener_site(def) || directed(drawn_view(def))
}

/// Whether some component's view `{extends}` `def` — a layout, drawn by its extender's render
/// around the blocks that render drew, so a repaint of its own goes through that extender.
pub(crate) fn is_extended_layout(def: &ComponentDef<'_>, all: &[ComponentDef<'_>]) -> bool {
    all.iter()
        .filter_map(|d| d.view.as_ref())
        .any(|v| extended_layout(&v.nodes) == Some(def.name))
}

impl<'a, 'ctx, 'int> super::super::Parser<'a, 'ctx, 'int> {
    /// ★ DUPLICATE KEYS — Tristen's rule: a `keyed by` loop whose siblings share a key fails
    /// LOUDLY, on every tier, naming the loop, the key and both positions; no aliasing, no
    /// positional fallback. ★
    ///
    /// A pre-pass `Repeat` over the WHOLE collection, run before any row renders (never a
    /// partial render followed by a trap mid-way): tracks every key it has seen, IN ORDER, in a
    /// fresh `Seq of Text`, and `Fail`s the moment a second item's key is already in it. Built
    /// from `Seq`/`Contains`/`Add` rather than a Map on purpose — a freshly-declared LOCAL Map's
    /// value kind is a wasm backend gap this lane does not own (`vm/wasm/module.rs`'s own
    /// `map_value_load`/`map_value_store`), and it broke EVERY keyed page's wasm build the one
    /// time this used one; a `Seq` is the SAME construct the keyed reconciler's own `__KEYS_<C>`
    /// tables already lower everywhere, so this stays inside ground already proven safe. `Stmt::
    /// Fail` is a real LOGOS statement every backend already compiles for an author's own
    /// `Fail "…".` — lowered here rather than reinvented per tier, so the VM, wasm (which lowers
    /// the VM's own bytecode, not a second AST-to-wasm pass) and AOT-Rust/C all fail identically.
    ///
    /// The key is coerced to `Text` via `"" + key` — the SAME coercion `lgx_pending_key`'s own
    /// consumer uses to stamp a row's `key` attribute — so two keys collide as the exact text a
    /// `keyed by` attribute would show, never on their original (possibly different) types.
    ///
    /// A duplicate's FIRST position is found by a small nested scan over the same `Seq` (its own
    /// index, 1-based, IS that key's first-seen position, since a key is only ever appended once,
    /// in iteration order) — one linear pass, only run the one time a collision is confirmed, so
    /// an ordinary unique-keyed loop never pays for it.
    ///
    /// The message is built from [`crate::ast::view::duplicate_keyed_loop_message_parts`] — the
    /// SAME literal segments `duplicate_keyed_loop_message` itself formats with, so the tree-
    /// walker's Rust-built text and this runtime string concatenation cannot drift apart.
    pub(crate) fn duplicate_key_check(
        &mut self,
        pattern: crate::ast::stmt::Pattern,
        iterable: &'a Expr<'a>,
        key: &'a Expr<'a>,
        next: &mut usize,
    ) -> Stmt<'a> {
        let n = *next;
        *next += 1;
        let text_sym = self.interner.intern("Text");
        let seen = self.interner.intern(&format!("__lgx_dupkeys{n}"));
        let pos = self.interner.intern(&format!("__lgx_duppos{n}"));
        let key_var = self.interner.intern(&format!("__lgx_dupkeytext{n}"));

        let empty_seq = self.new_seq_expr(text_sym);
        let mut pre: Vec<Stmt<'a>> = vec![
            Stmt::Let {
                var: seen,
                ty: None,
                value: empty_seq,
                mutable: true,
            },
            Stmt::Let {
                var: pos,
                ty: None,
                value: self.te_int(crate::ast::view::LOOP_INDEX_BASE),
                mutable: true,
            },
        ];

        let blank = self.interner.intern("");
        let key_text = self.te_bin(BinaryOpKind::Add, self.te_text(blank), key);

        // The scan doubles as the membership test: `Contains` on a `Seq of Text` is its own
        // separate wasm gap ("needs byte-equality") — this walks `seen` with a plain `Eq` between
        // two `Text` values instead, the SAME comparison an event-modifier match already lowers
        // everywhere (`listener_closure`'s own `want` filter), so it stays on ground already
        // proven safe. `first` (0 = "not found yet") doubles as both "was this key seen" and,
        // when it was, exactly where — one pass answers both questions.
        let first = self.interner.intern(&format!("__lgx_dupfirst{n}"));
        let scan_pos = self.interner.intern(&format!("__lgx_dupscan{n}"));
        let scan_item = self.interner.intern(&format!("__lgx_dupitem{n}"));
        let zero = self.te_int(0);
        let first_unset = self.te_bin(BinaryOpKind::Eq, self.te_id(first), self.te_int(0));
        let item_matches =
            self.te_bin(BinaryOpKind::Eq, self.te_id(scan_item), self.te_id(key_var));
        let should_capture = self.te_bin(BinaryOpKind::And, first_unset, item_matches);
        let scan_body = vec![
            Stmt::If {
                cond: should_capture,
                then_block: self.alloc_block(vec![Stmt::Set {
                    target: first,
                    value: self.te_id(scan_pos),
                }]),
                else_block: None,
            },
            Stmt::Set {
                target: scan_pos,
                value: self.te_bin(BinaryOpKind::Add, self.te_id(scan_pos), self.te_int(1)),
            },
        ];
        let scan = vec![
            Stmt::Let {
                var: first,
                ty: None,
                value: zero,
                mutable: true,
            },
            Stmt::Let {
                var: scan_pos,
                ty: None,
                value: self.te_int(crate::ast::view::LOOP_INDEX_BASE),
                mutable: true,
            },
            Stmt::Repeat {
                pattern: crate::ast::stmt::Pattern::Identifier(scan_item),
                iterable: self.te_id(seen),
                body: self.alloc_block(scan_body),
            },
        ];

        use crate::ast::view::duplicate_keyed_loop_message_parts as msg;
        let loop_desc = crate::ast::view::expr_shape(iterable, self.interner);
        let head = self.interner.intern(&format!(
            "{}{loop_desc}{}",
            msg::PREFIX,
            msg::AFTER_LOOP_DESC
        ));
        let mid1 = self.interner.intern(msg::AFTER_FIRST_POSITION);
        let mid2 = self.interner.intern(msg::AFTER_SECOND_POSITION);
        let tail = self.interner.intern(msg::SUFFIX);
        let mut reason = self.te_text(head);
        reason = self.te_bin(BinaryOpKind::Add, reason, self.te_id(first));
        reason = self.te_bin(BinaryOpKind::Add, reason, self.te_text(mid1));
        reason = self.te_bin(BinaryOpKind::Add, reason, self.te_id(pos));
        reason = self.te_bin(BinaryOpKind::Add, reason, self.te_text(mid2));
        reason = self.te_bin(BinaryOpKind::Add, reason, self.te_id(key_var));
        reason = self.te_bin(BinaryOpKind::Add, reason, self.te_text(tail));

        let found = self.te_bin(BinaryOpKind::NotEq, self.te_id(first), self.te_int(0));
        let record = Stmt::Push {
            value: self.te_id(key_var),
            collection: self.te_id(seen),
        };
        let bump = Stmt::Set {
            target: pos,
            value: self.te_bin(BinaryOpKind::Add, self.te_id(pos), self.te_int(1)),
        };
        let mut check_body = vec![Stmt::Let {
            var: key_var,
            ty: None,
            value: key_text,
            mutable: false,
        }];
        check_body.extend(scan);
        check_body.push(Stmt::If {
            cond: found,
            then_block: self.alloc_block(vec![Stmt::Fail { reason }]),
            else_block: Some(self.alloc_block(vec![record])),
        });
        check_body.push(bump);
        pre.push(Stmt::Repeat {
            pattern,
            iterable,
            body: self.alloc_block(check_body),
        });
        Stmt::Splice {
            body: self.alloc_block(pre),
        }
    }

    /// Build the `## To __render_<Name> (key, props…, slot, blocks…):` function for a lowerable
    /// component: the instance's contents written to the tape — its placement draws the fragment
    /// around them, and the paint commits them.
    pub(crate) fn lower_component_render(
        &mut self,
        def: &ComponentDef<'a>,
        all: &[ComponentDef<'a>],
    ) -> Option<Stmt<'a>> {
        if lowerable(def, all, self.type_registry.as_ref(), self.interner).is_err() {
            return None;
        }
        let view = def.view.as_ref()?;
        let mut body: Vec<Stmt<'a>> = Vec::new();
        let mut params: Vec<(Symbol, &'a TypeExpr<'a>)> = Vec::new();
        let mut next = 0usize;
        // The state fields become ordinary locals, in declaration order, so a derived field may
        // read the plain ones.
        let component = self.interner.resolve(def.name).to_string();
        let is_stateful = stateful(def);
        // ★ ONLY A COMPONENT THAT REGISTERS SOMETHING PAYS FOR THE LEDGER. `__LISTENER_SITES_<C>`
        // (declared above, gated the same way) has nothing to retire and nothing to write back for
        // a component with no `on-*`/`bind`/`When … :` handler anywhere — `has_any_listener_site`
        // decides once, here, whether any of the three blocks below (retire, the paint-time
        // accumulator, and its write-back) has any reason to exist in this render at all.
        let has_listeners = has_any_listener_site(def);
        // ★ AN INSTANCE THAT DRAWS SOMETHING A LISTENER OR DIRECTIVE MUST FIND AFTER THE COMMIT
        // GIVES THOSE ELEMENTS A SITE FOR THIS PAINT, kept against its fragment (`vdAnchor`), and
        // hands them back when it paints again or leaves (`vdReleaseAnchors`, the unmount sweep).
        let anchors = needs_anchors(def);
        // Where this instance's own records start on the tape, for a sentence that finds its
        // element by selector once the view is drawn.
        let from = self.interner.intern(tape_from_local());
        let written = self.vd_expr("vdTapeLength", Vec::new());
        let first_record = self.te_bin(BinaryOpKind::Add, written, self.te_int(1));
        body.push(Stmt::Let {
            var: from,
            ty: None,
            value: first_record,
            mutable: false,
        });
        // ★ A COMPONENT THAT CAN BE DRAWN INSIDE ITSELF COUNTS HOW DEEP IT IS, and one level past
        // `RENDER_DEPTH_LIMIT` fails the paint in `render_depth_message`'s words — the budget lands
        // before the stack does, on every tier that runs this function. Recursion is not the
        // fault; a nesting nothing stops is, and depth is what tells them apart.
        let counted = on_placement_cycle(def, all);
        if counted {
            let depth = self.interner.intern(render_depth_binding());
            let deeper = self.te_bin(BinaryOpKind::Add, self.te_id(depth), self.te_int(1));
            body.push(Stmt::Set {
                target: depth,
                value: deeper,
            });
            let limit = crate::ast::component::RENDER_DEPTH_LIMIT as i64;
            let past = self.te_bin(BinaryOpKind::Gt, self.te_id(depth), self.te_int(limit));
            let message = self
                .interner
                .intern(&crate::ast::component::render_depth_message(&component));
            let refuse = self.alloc_block(vec![Stmt::Fail {
                reason: self.te_text(message),
            }]);
            body.push(Stmt::If {
                cond: past,
                then_block: refuse,
                else_block: None,
            });
        }
        // ★ RETIRE WHAT THIS EXACT INSTANCE REGISTERED LAST PAINT, BEFORE THIS PAINT REGISTERS
        // ANYTHING NEW. `__LISTENER_FN`/`__LISTENER_KEY`/every `__ARG_<C>_<action>_<i>` are keyed
        // by a lookup derived from the node, and a repaint hands out FRESH node ids — so the
        // entries this same instance registered last time are dead the moment its old nodes are
        // cleared, and nothing else will ever remove them. Guarded on `Contains`, not on
        // `first`/`repaint_def`: a
        // component that only ever gets PLACED (never independently repainted) still runs this
        // function again every time its mount point's owner repaints, and its own key is the only
        // reliable "have I painted before" question — the render that reuses `repaint_def`'s own
        // notion of first/not-first pays no extra guard, and a component `repaint_def` never wraps
        // (a placed loop row, redirected through the root) gets the right answer anyway.
        if has_listeners {
            let key = self.key_expr();
            let sites = self.interner.intern(&listener_sites_binding(&component));
            let has_sites = self.ctx.alloc_imperative_expr(Expr::Contains {
                collection: self.te_id(sites),
                value: key,
            });
            let key = self.key_expr();
            let old_list = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(sites),
                index: key,
            });
            let ls = self.interner.intern("__lgx_ls");
            let forget = self.interner.intern(forget_listener_fn_name());
            let retire = vec![Stmt::Call {
                function: forget,
                args: vec![self.te_id(ls)],
            }];
            let prune = Stmt::If {
                cond: has_sites,
                then_block: self.alloc_block(vec![Stmt::Repeat {
                    pattern: crate::ast::stmt::Pattern::Identifier(ls),
                    iterable: old_list,
                    body: self.alloc_block(retire),
                }]),
                else_block: None,
            };
            body.push(prune);
        }
        // The accumulator THIS paint's own registrations append to (`register_listener_with`),
        // written back once, at the tail of this function, to `__LISTENER_SITES_<C>[key]` — so the
        // NEXT paint of this exact instance finds exactly what THIS one registered. Gated with the
        // retire block above: `register_listener_with` only ever runs, for this component, when
        // `has_listeners` is true, so an undeclared `paint_sites` is never reached.
        if has_listeners {
            let paint_sites = self.interner.intern(paint_sites_local());
            let int_name = self.interner.intern("Int");
            let fresh_sites = self.new_seq_expr(int_name);
            body.push(Stmt::Let {
                var: paint_sites,
                ty: None,
                value: fresh_sites,
                mutable: true,
            });
        }
        if anchors {
            let frag = self.interner.intern(render_frag_param());
            body.push(self.vd_call("vdReleaseAnchors", vec![self.te_id(frag)]));
        }
        // A `Style:` block is real CSS, scoped by stamping the component's class on each
        // selector's subject — performed once here instead of on every render.
        // ★ THE SHEET IS NOT EMITTED HERE ANY MORE. It used to be appended to whatever element this
        // component happened to render into, guarded by a once-per-paint flag — so a styled row in
        // a `<ul>` put its `<style>` between the list and its first item. `repaint_def` now emits
        // every reachable sheet at the MOUNT POINT before any markup, which is both valid HTML and
        // what the server does, so the two tiers agree about where a stylesheet lives.
        // ★ AN INSTANCE RECORDS WHAT IT WAS GIVEN. A turn repaints by calling this function
        // again, and a turn has no parent to hand the props down — so they are written here, per
        // instance, and read back by `repaint_def`.
        if is_stateful {
            for (prop, ty) in &def.props {
                if is_callback_type(ty, self.interner) {
                    continue;
                }
                let value = self.te_id(*prop);
                let w = self.prop_write(&component, *prop, value);
                body.push(w);
            }
        }
        // ★ AN INSTANCE IS CREATED THE FIRST TIME IT RENDERS — created, empty, unmounted, if this
        // is its first render. The root instance is seeded where the binding is declared; every other one arrives here, and
        // without this a second placement asks the map for a key nothing ever put in it.
        //
        // Guarded rather than written unconditionally, because a render runs again on every
        // repaint and an unguarded write would reset the instance's state on each one — the page
        // would look correct until the first turn and then snap back.
        if is_stateful {
            for field in def.state.iter().filter(|f| !f.derived) {
                let global = self.interner.intern(&state_binding(
                    &component,
                    self.interner.resolve(field.name),
                ));
                let key = self.key_expr();
                let known = self.ctx.alloc_imperative_expr(Expr::Contains {
                    collection: self.te_id(global),
                    value: key,
                });
                let unknown = self.ctx.alloc_imperative_expr(Expr::Not { operand: known });
                // ★ AN INSTANCE THE SWEEP RETIRED IS SEEDED AGAIN, because it is a NEW instance —
                // retiring an instance destroys its record. `sweep_gone`
                // now REMOVES a gone key's entry from this exact map rather than marking it, so
                // `unknown` alone already answers "is this a fresh instance" correctly: the map
                // holds no trace of a retired key, exactly as it holds none of a key that never
                // existed.
                //
                // The sweep does not write the initializer itself: this branch already holds the
                // only copy of it, and writing it twice — once here, once in the sweep — was
                // measured at 85 bytes of module per item for a `{for each}` over a list literal.
                let fresh = unknown;
                let seed = self.state_write(&component, field.name, field.init);
                body.push(Stmt::If {
                    cond: fresh,
                    then_block: self.alloc_block(vec![seed]),
                    else_block: None,
                });
            }
        }
        // ★ THE KEY LIST IS WRITTEN BEFORE THE TIMERS ARE ARMED, and the order is the whole
        // point. Both are guarded on "is this instance already in the due map"; arming first put
        // it there, so the guard below then read TRUE and the instance was never added to the list
        // the tick walks. The page armed its clock and then never looked at it — no trap, no
        // message, just a timer that silently never fires.
        //
        // The list is shared with the unmount sweep, which adds its own entry when the component
        // also asks to be told it left.
        // ★ ONE PUSH FOR EVERY READER OF THE KEY LIST. A timer walks it, the unmount sweep walks
        // it, and a store watcher walks it — but only the timer's own condition used to fill it,
        // so a component that watched a store declared an empty list and its handler ran for
        // nobody. It compiled, the catalogue said "reaches the browser", and the page silently did
        // not react: lowering is not correctness, and only the cross-tier render caught it.
        //
        // Guarded on the presence MAP rather than the list, because the wasm backend lowers
        // "does this map hold this key" and not "does this sequence contain this text".
        // ★ `When this component appears:` WAITS FOR THE WHOLE PAINT. Whether this is the
        // instance's first render is decided HERE, from the presence map, because the two blocks
        // a little further down both write into that same map and would leave nothing to ask
        // afterward — so the ANSWER is captured into a local now, and the instance is owed its
        // handlers once the view walk below has drawn it.
        let appears: Vec<usize> = def
            .events
            .iter()
            .enumerate()
            .filter(|(_, h)| matches!(h.trigger, Trigger::Appears))
            .map(|(i, _)| i)
            .collect();
        // ★ `When the page loads:` RUNS ONCE PER INSTANCE, BEFORE ITS FIRST PAINT — every instance
        // created is mounted, so a child placed under an `{if}` that leaves and comes back is a new
        // instance and loads again. The ROOT (key `""`)
        // is loaded by its mount (`lower_component_mount`) before that first paint; every placed
        // instance loads here, the first time it is drawn, from the same presence map `Appears`
        // asks.
        let loads: Vec<&'a [Stmt<'a>]> = def
            .events
            .iter()
            .filter(|h| matches!(h.trigger, Trigger::PageLoads))
            .map(|h| h.body)
            .collect();
        let load_flag = if keeps_instance_registry(def) && !loads.is_empty() {
            let seen = self.interner.intern(&seen_binding(&component));
            let key = self.key_expr();
            let known = self.ctx.alloc_imperative_expr(Expr::Contains {
                collection: self.te_id(seen),
                value: key,
            });
            let unknown = self.ctx.alloc_imperative_expr(Expr::Not { operand: known });
            let root = self.interner.intern("");
            let this = self.key_expr();
            let placed = self.te_bin(BinaryOpKind::NotEq, this, self.te_text(root));
            let flag = self.interner.intern("__lgx_first_load");
            let first = self.te_bin(BinaryOpKind::And, unknown, placed);
            body.push(Stmt::Let {
                var: flag,
                ty: None,
                value: first,
                mutable: false,
            });
            Some(flag)
        } else {
            None
        };
        let appears_flag = if keeps_instance_registry(def) && !appears.is_empty() {
            let seen = self.interner.intern(&seen_binding(&component));
            let key = self.key_expr();
            let known = self.ctx.alloc_imperative_expr(Expr::Contains {
                collection: self.te_id(seen),
                value: key,
            });
            let unknown = self.ctx.alloc_imperative_expr(Expr::Not { operand: known });
            let flag = self.interner.intern("__lgx_first_appear");
            body.push(Stmt::Let {
                var: flag,
                ty: None,
                value: unknown,
                mutable: false,
            });
            Some(flag)
        } else {
            None
        };
        let wants_keys = self.lgx_timers.iter().any(|(c, ..)| *c == def.name)
            || def.events.iter().any(|h| {
                matches!(
                    h.trigger,
                    Trigger::Disappears | Trigger::FieldChanges { store: Some(_), .. }
                )
            });
        if wants_keys {
            let keys = self.interner.intern(&keys_binding(&component));
            let seen = self.interner.intern(&seen_binding(&component));
            let key = self.key_expr();
            let known = self.ctx.alloc_imperative_expr(Expr::Contains {
                collection: self.te_id(seen),
                value: key,
            });
            let unknown = self.ctx.alloc_imperative_expr(Expr::Not { operand: known });
            let key = self.key_expr();
            let push = Stmt::Push {
                value: key,
                collection: self.te_id(keys),
            };
            let key2 = self.key_expr();
            // Recorded as present the moment it is listed, so the guard is true next time. A
            // component that also asks about unmounts overwrites this with the pass number below.
            let mark = Stmt::SetIndex {
                collection: self.te_id(seen),
                index: key2,
                value: self.te_int(-1),
            };
            body.push(Stmt::If {
                cond: unknown,
                then_block: self.alloc_block(vec![push, mark]),
                else_block: None,
            });
        }
        // ★ STAMP THIS INSTANCE WITH THE PAINT IT IS BEING DRAWN IN. Everything `When this
        // component disappears:` needs follows from this one line: after the paint, an instance
        // still carrying an older pass was not drawn, so it is gone.
        if keeps_instance_registry(def) {
            let seen = self.interner.intern(&seen_binding(&component));
            let keys = self.interner.intern(&keys_binding(&component));
            let pass = self.interner.intern(pass_binding());
            // First render of this instance: remember its key, so the sweep can walk it later.
            //
            // ★ GUARDED ON THE SEEN-MAP, NOT ON THE KEY LIST. Asking a SEQUENCE whether it
            // contains a Text is not something the wasm backend lowers ("needs byte-equality"),
            // while asking a MAP whether it holds a key is — it is the same test the state
            // bindings seed themselves with. The two stay in step because both are seeded with the
            // root key, so "already in the map" and "already in the list" answer alike.
            let key = self.key_expr();
            let known = self.ctx.alloc_imperative_expr(Expr::Contains {
                collection: self.te_id(seen),
                value: key,
            });
            let unknown = self.ctx.alloc_imperative_expr(Expr::Not { operand: known });
            let key = self.key_expr();
            let push = Stmt::Push {
                value: key,
                collection: self.te_id(keys),
            };
            body.push(Stmt::If {
                cond: unknown,
                then_block: self.alloc_block(vec![push]),
                else_block: None,
            });
            let key = self.key_expr();
            body.push(Stmt::SetIndex {
                collection: self.te_id(seen),
                index: key,
                value: self.te_id(pass),
            });
        }
        // A component whose state never changes reads its initializers directly; one with
        // actions reads the module bindings the turns write back to.
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
        // ★ ARM THIS INSTANCE'S TIMERS THE FIRST TIME IT RENDERS, and never again — a render runs
        // on EVERY repaint, and a re-arming render would push a timer on a page that repaints past
        // every due time it ever had. Armed here, below the state and derived bindings, so a period
        // computed from state or a prop reads this instance's values as it is drawn.
        // The load runs as the root's does at its mount: the body over this instance's fields, the
        // fields written back, the derived ones recomputed from them, the stores it moved woken —
        // and what it shows is what the paint below draws.
        if let Some(flag) = load_flag {
            self.note_state_types(def);
            let outer_events = std::mem::take(&mut self.lgx_pending_events);
            let mut run: Vec<Stmt<'a>> = Vec::new();
            for body_of in &loads {
                let inner = self.rewrite_store_calls(body_of, &component)?;
                let inner = self.protect_early_return(inner.to_vec());
                run.extend(inner.iter().cloned());
            }
            self.lgx_pending_events = outer_events;
            for field in def.state.iter().filter(|f| !f.derived) {
                let value = self.te_id(field.name);
                run.push(self.state_write(&component, field.name, value));
            }
            for field in def.state.iter().filter(|f| f.derived) {
                run.push(Stmt::Set {
                    target: field.name,
                    value: field.init,
                });
            }
            run.extend(self.wake_stores_call());
            body.push(Stmt::If {
                cond: self.te_id(flag),
                then_block: self.alloc_block(run),
                else_block: None,
            });
        }
        body.extend(self.arm_timers(def));
        // The instance's own view is drawn with no row key pending: a placement's instance is
        // matched by its fragment's identity, and a `{for each}` inside the view keys its own rows.
        let outer_key = self.lgx_pending_key.take();
        match extended_layout(&view.nodes).and_then(|l| all.iter().find(|d| d.name == l)) {
            // ★ `{extends L}` DRAWS TWO INSTANCES. Each `{block X:}` this view fills is drawn HERE,
            // in this component's scope, as a range (`slot_range`); then the layout draws itself
            // around them as an instance of its own — its fragment, state and registry keyed like a
            // placement at the `{extends}` node — placing each range at its `{block X}` (`0`: the
            // layout's default) and this component's own `{children}` at its own.
            Some(layout) => {
                let mut blocks = Vec::new();
                for block in slot_blocks(layout) {
                    let fill = view.nodes.iter().enumerate().find_map(|(at, n)| match n {
                        ViewNode::Block {
                            name,
                            default: Some(b),
                            ..
                        } if *name == block => Some((at, &b[..])),
                        _ => None,
                    });
                    blocks.push(match fill {
                        Some((at, b)) => {
                            let trail = LowerTrail::root()
                                .branch(&format!("{}.", LowerTrail::root().npath(at)));
                            self.slot_range(b, &mut next, &mut body, all, def, &trail)
                        }
                        None => self.te_int(0),
                    });
                }
                let here = self
                    .interner
                    .intern(&format!("/{}", LowerTrail::root().npath(0)));
                let own_key = self.key_expr();
                let layout_key = self.te_bin(BinaryOpKind::Add, own_key, self.te_text(here));
                let key_local = self.lgx_temp(&mut next);
                body.push(Stmt::Let {
                    var: key_local,
                    ty: None,
                    value: layout_key,
                    mutable: false,
                });
                let slot = self.interner.intern(render_slot_param());
                let own_children = self.te_id(slot);
                let frag =
                    self.instance_frag(layout.name, self.te_id(key_local), &mut next, &mut body);
                let mut args = vec![self.te_id(key_local), self.te_id(frag), own_children];
                args.extend(blocks);
                let render = self
                    .interner
                    .intern(&render_fn_name(self.interner.resolve(layout.name)));
                body.push(self.vd_call("vdFragOpen", vec![self.te_id(frag)]));
                body.push(Stmt::Call {
                    function: render,
                    args,
                });
                body.push(self.vd_call("vdFragClose", Vec::new()));
            }
            None => self.lower_lgx_nodes(
                &view.nodes,
                &mut next,
                &mut body,
                all,
                def,
                &LowerTrail::root(),
            ),
        }
        self.lgx_pending_key = outer_key;
        // ★ OWED, NOT RUN. Each `When this component appears:` handler is the SAME compiled turn
        // `lower_component_event` gives every other one (its own read-in, write-back, `noteTurn`
        // and repaint), and a repaint from inside the paint still drawing this instance would draw
        // a second page beside it. So the instance goes on the appears queue, and the end of the
        // paint (`__lgx_appear`) runs its handlers with the whole page drawn — a `Focus`/`Add
        // class`/etc. they carry finds a real element.
        if let Some(flag) = appears_flag {
            let who = appearing_components(all, self.type_registry.as_ref(), self.interner)
                .iter()
                .position(|c| *c == def.name)
                .map_or(0, |i| i as i64 + 1);
            let (whos, keys) = (
                self.interner.intern(appear_who_binding()),
                self.interner.intern(appear_key_binding()),
            );
            let owe = vec![
                Stmt::Push {
                    value: self.te_int(who),
                    collection: self.te_id(whos),
                },
                Stmt::Push {
                    value: self.key_expr(),
                    collection: self.te_id(keys),
                },
            ];
            body.push(Stmt::If {
                cond: self.te_id(flag),
                then_block: self.alloc_block(owe),
                else_block: None,
            });
        }
        // ★ A SENTENCE FINDS ITS ELEMENT ON THE TAPE, now that the view is drawn: the selector is
        // asked of the records this instance wrote (`vdQueryText`), which is what the committed
        // document will hold — so a placed component's sentence finds its own element on every
        // tier, drawn or not yet in the document. The element's listener rides beside the tape
        // (`vdListenAt`), keyed by the site the element carries. A sentence about the page (a key
        // anywhere, the window, the page's scroll) listens on the page's own node.
        for (index, handler) in def.events.iter().enumerate() {
            let action = self.interner.intern(&format!("__e{index}"));
            let plain = |key: Option<Symbol>, carries_value: bool| ListenerSite::Action {
                component: def.name,
                action,
                argc: 0,
                carries_value,
                key,
                once: None,
            };
            match &handler.trigger {
                Trigger::Selector {
                    selector,
                    event,
                    key,
                } => {
                    let listens = [(*event, 0)];
                    self.sentence_on_element(
                        selector,
                        &listens,
                        &[(*event, plain(*key, false))],
                        &mut next,
                        &mut body,
                    );
                }
                // ★ `filedropped` IS WHAT THE HANDLER WAITS FOR, on every tier; the element takes
                // drops only while its drag events are cancelled, and a browser's `drop` is heard
                // as the file dropped (`runtime.js`'s page listener names it).
                Trigger::FileDropped { selector, .. } => {
                    let cancel = Behaviour::Prevent.listen_bit() as i64;
                    let listens: Vec<(Symbol, i64)> = DROP_TARGET_CANCELS
                        .iter()
                        .map(|e| (self.interner.intern(e), cancel))
                        .collect();
                    let dropped = self.interner.intern("filedropped");
                    self.sentence_on_element(
                        selector,
                        &listens,
                        &[(dropped, plain(None, true))],
                        &mut next,
                        &mut body,
                    );
                }
                // ★ THE SEMANTIC EVENT ANSWERS A TEST; THE RAW ONE DRIVES IT. `scrolledtobottom` is
                // what a tier with no viewport raises; a browser raises `scroll`, whose arm asks the
                // geometry whether this scroll counts (`ListenerSite::ScrollNear`).
                Trigger::ScrollsToBottom { selector } => {
                    let scroll = self.interner.intern("scroll");
                    let semantic = self.interner.intern("scrolledtobottom");
                    let near = ListenerSite::ScrollNear {
                        component: def.name,
                        action,
                    };
                    let answers = [(semantic, plain(None, false)), (scroll, near)];
                    match selector {
                        Some(sel) => self.sentence_on_element(
                            sel,
                            &[(scroll, 0)],
                            &answers,
                            &mut next,
                            &mut body,
                        ),
                        None => {
                            self.sentence_on_page(&[(scroll, 0)], &answers, &mut next, &mut body)
                        }
                    }
                }
                Trigger::Key { key } => {
                    let keydown = self.interner.intern("keydown");
                    self.sentence_on_page(
                        &[(keydown, 0)],
                        &[(keydown, plain(Some(*key), false))],
                        &mut next,
                        &mut body,
                    );
                }
                // ★ THE WINDOW EVENTS ARE HEARD ON THE WINDOW, NOT THE PAGE: `offline`, `online` and
                // `resize` fire on `window` and do not bubble. The page answers them on its own
                // node — where a test raises them — and the window's one listener, attached once
                // per page, hands each to that node.
                Trigger::Online | Trigger::Offline | Trigger::WindowResizes => {
                    let name = match &handler.trigger {
                        Trigger::Online => "online",
                        Trigger::Offline => "offline",
                        _ => "resize",
                    };
                    let event = self.interner.intern(name);
                    self.sentence_on_page(
                        &[],
                        &[(event, plain(None, false))],
                        &mut next,
                        &mut body,
                    );
                    body.push(self.window_forwarder(def.name, index, event));
                }
                // ★ A SUBSCRIPTION IS NOT A DOM LISTENER. Each instance waiting on the topic answers
                // `message:<topic>` on the page node, where a test raises it; the relay is told once
                // per page which topic the page wants, and hands each frame, payload and all, to the
                // page node.
                Trigger::Message { topic, .. } => {
                    let Expr::Literal(Literal::Text(t)) = topic else {
                        continue;
                    };
                    let event = self
                        .interner
                        .intern(&format!("message:{}", self.interner.resolve(*t)));
                    self.sentence_on_page(&[], &[(event, plain(None, true))], &mut next, &mut body);
                    body.push(self.topic_forwarder(def.name, index, *topic, event));
                }
                _ => {}
            }
        }
        // `When the page loads:` / `When this component appears:` run once, before the first
        // paint — but the lowered render runs on EVERY repaint, so they are driven from the
        // program's mount rather than from here.
        // ★ WRITTEN BACK LAST, so `__LISTENER_SITES_<C>[key]` always holds exactly what THIS paint
        // registered — nothing this render walked above can still append to the accumulator once
        // this runs. Gated with the accumulator's own declaration above: with no listeners, there
        // is no `paint_sites` local and no `__LISTENER_SITES_<C>` module binding to write it into.
        if has_listeners {
            let key = self.key_expr();
            let sites = self.interner.intern(&listener_sites_binding(&component));
            let paint_sites = self.interner.intern(paint_sites_local());
            body.push(Stmt::SetIndex {
                collection: self.te_id(sites),
                index: key,
                value: self.te_id(paint_sites),
            });
        }
        if counted {
            let depth = self.interner.intern(render_depth_binding());
            let shallower = self.te_bin(BinaryOpKind::Subtract, self.te_id(depth), self.te_int(1));
            body.push(Stmt::Set {
                target: depth,
                value: shallower,
            });
        }
        let block = self.alloc_block(body);
        let name = self
            .interner
            .intern(&render_fn_name(self.interner.resolve(def.name)));
        let int_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Int")));
        // WHICH INSTANCE this render is for comes first, then the fragment it is drawn as, then the
        // props in DECLARATION order — the order the call site reads them back out of, so a
        // template may name them in any order it likes.
        //
        // ★ THE KEY IS A PARAMETER RATHER THAN A LOCAL because the caller is the only one who
        // knows it: a placement decides which instance it is drawing. Everywhere else the key is
        // already in scope (a turn's, a mount's), and the ROOT is `""`.
        let key = self.interner.intern(instance_key_param());
        let text_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Text")));
        params.push((key, text_ty));
        // ★ THE FRAGMENT IT IS DRAWN AS, beside the key: the caller made or found it, and what the
        // paint hands out for its elements to be found by after the commit is kept against it.
        params.push((self.interner.intern(render_frag_param()), int_ty));
        let props: Vec<(Symbol, &'a TypeExpr<'a>)> = def
            .props
            .iter()
            .copied()
            .filter(|(_, t)| !is_callback_type(t, self.interner))
            .collect();
        for (prop, ty) in props {
            let ty = self.lowered_type(ty);
            params.push((prop, ty));
        }
        // ★ THE SLOT COMES AFTER THEM AND IS ALWAYS THERE, for the same reason the server's is: a
        // component that grows a `{children}` hole must not change its own signature and break
        // every call site that already places it. A placement with no children passes `0`.
        params.push((self.interner.intern(render_slot_param()), int_ty));
        // A layout's `{block X}` slots last, one each, in the order its view declares them — what
        // an extending view drew for the block, or `0` for the layout's default
        // ([`Self::block_slot_zeros`] is what every other caller passes).
        for block in slot_blocks(def) {
            let param = self
                .interner
                .intern(&block_slot_param(self.interner.resolve(block)));
            params.push((param, int_ty));
        }
        Some(Stmt::FunctionDef {
            name,
            generics: Vec::new(),
            params,
            body: block,
            return_type: None,
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        })
    }

    /// ★ THE VIEW WALK WRITES THE TAPE. Each node becomes the vdom.lg records that draw it — an
    /// element's open, its modifiers (attributes, class list, bound value, listeners, raw content,
    /// the site something finds it by), its children, its close — in document order, and
    /// `vdCommit` turns the tape into the fewest DOM operations that make the retained tree match.
    /// Nothing here touches the host: a render is data until its paint commits.
    pub(crate) fn lower_lgx_nodes(
        &mut self,
        nodes: &[ViewNode<'a>],
        next: &mut usize,
        out: &mut Vec<Stmt<'a>>,
        all: &[ComponentDef<'a>],
        owner: &ComponentDef<'a>,
        trail: &LowerTrail,
    ) {
        let owner_name = self.interner.resolve(owner.name).to_string();
        // A `{let}` binds for the siblings that follow it, and no further.
        let views_in_scope = self.lgx_views.locals.len();
        // Patch sites exist only for a component that can CHANGE — the same rule its state
        // bindings follow: a component that never runs a turn is never patched.
        let records_sites = stateful(owner);
        for (sibling, node) in nodes.iter().enumerate() {
            // The same numbering `lgx::deps` and the interpreter use — three walks over one tree
            // that must agree on an address, so they agree on ONE string.
            let npath = trail.npath(sibling);
            let opaque = trail.opaque;
            match node {
                ViewNode::Element {
                    tag,
                    attrs,
                    children,
                    ..
                } => {
                    // A `keyed by` row is identified among its siblings by its key, carried on its
                    // first element's open — TAKEN, so it lands on the row and not its children,
                    // and shown through `"" + k` because a key is whatever the author keyed by.
                    // Any other element that names itself with a literal `id` is identified by
                    // that name, so a repaint that shifts its siblings keeps it (and its focus)
                    // where the id is; an id computed from state is data, and a change to it must
                    // patch the element rather than replace it.
                    let tag_text = self.te_text(*tag);
                    let open = match self.lgx_pending_key.take() {
                        Some(k) => {
                            let blank = self.interner.intern("");
                            let shown = self.te_bin(BinaryOpKind::Add, self.te_text(blank), k);
                            self.vd_call("vdOpenKeyed", vec![tag_text, shown])
                        }
                        None => match self.literal_id(attrs) {
                            Some(id) => {
                                let named = self.interner.intern(&format!("#{id}"));
                                let named = self.te_text(named);
                                self.vd_call("vdOpenSoft", vec![tag_text, named])
                            }
                            None => self.vd_call("vdOpen", vec![tag_text]),
                        },
                    };
                    out.push(open);
                    let scope = match self.lgx_views.scope_param {
                        Some(param) => Some(ElementScope::Param(param)),
                        None => owner
                            .style
                            .is_some()
                            .then(|| ElementScope::Class(scope_class(&owner_name))),
                    };
                    let sites_at = (!opaque && records_sites).then_some(npath.as_str());
                    self.emit_element_attrs(
                        *tag, attrs, children, owner, scope, sites_at, trail, next, out,
                    );
                    self.lower_lgx_nodes(children, next, out, all, owner, &trail.child(&npath));
                    out.push(self.vd_call("vdClose", Vec::new()));
                }
                // A hole holding a View places its nodes; anything else renders as text — a text
                // a turn can rewrite carries this instance's site for it, so the patch finds it.
                ViewNode::Text { text, .. } => {
                    if let Some(place) = self.place_view_hole(text, owner) {
                        out.push(place);
                        continue;
                    }
                    let shown = self.as_text(text);
                    if !opaque && records_sites {
                        let site =
                            self.stable_site(&owner_name, &format!("text@{npath}"), next, out);
                        out.push(self.vd_call("vdTextSite", vec![shown, self.te_id(site)]));
                    } else {
                        out.push(self.vd_call("vdText", vec![shown]));
                    }
                }
                // `{raw h}` and `on-<event>` belong to the element they sit in, whose open draws
                // them (`emit_element_attrs`).
                ViewNode::Raw { .. } | ViewNode::Handler { .. } => {}
                // `{if c:} … {else if c:} … {else:} … {end}` is ordinary control flow: the arms
                // become nested `If`s, innermost-last, so exactly one runs.
                ViewNode::If {
                    arms, otherwise, ..
                } => {
                    let mut fallback: Option<crate::ast::stmt::Block<'a>> = match otherwise {
                        Some(body) => {
                            let mut stmts = Vec::new();
                            self.lower_lgx_nodes(
                                body,
                                next,
                                &mut stmts,
                                all,
                                owner,
                                &trail.branch(&format!("{npath}.")),
                            );
                            Some(self.alloc_block(stmts))
                        }
                        None => None,
                    };
                    for (cond, body) in arms.iter().rev() {
                        let mut stmts = Vec::new();
                        self.lower_lgx_nodes(
                            body,
                            next,
                            &mut stmts,
                            all,
                            owner,
                            &trail.branch(&format!("{npath}.")),
                        );
                        let then_block = self.alloc_block(stmts);
                        let arm = Stmt::If {
                            cond,
                            then_block,
                            else_block: fallback,
                        };
                        fallback = Some(self.alloc_block(vec![arm]));
                    }
                    if let Some(block) = fallback {
                        out.extend(block.iter().cloned());
                    }
                }
                // `{for each x in xs[, at index i]:} … {else:} … {end}`. The `{else:}` arm is a
                // separate emptiness test rather than a flag on the loop, because that is what it
                // means: the body runs per item, the else runs when there were none.
                ViewNode::Each {
                    pattern,
                    iterable,
                    index,
                    key,
                    body,
                    empty,
                    ..
                } => {
                    if let Some(key_expr) = key {
                        out.push(self.duplicate_key_check(
                            pattern.clone(),
                            iterable,
                            *key_expr,
                            next,
                        ));
                    }
                    if !empty.is_empty() {
                        let mut stmts = Vec::new();
                        self.lower_lgx_nodes(
                            empty,
                            next,
                            &mut stmts,
                            all,
                            owner,
                            &trail.branch(&format!("{npath}.e")),
                        );
                        let then_block = self.alloc_block(stmts);
                        let is_empty = self.te_bin(
                            BinaryOpKind::Eq,
                            self.ctx.alloc_imperative_expr(Expr::Length {
                                collection: iterable,
                            }),
                            self.te_int(0),
                        );
                        out.push(Stmt::If {
                            cond: is_empty,
                            then_block,
                            else_block: None,
                        });
                    }
                    let mut stmts = Vec::new();
                    // ONE position counter, starting at `LOOP_INDEX_BASE`. `at index i` names it;
                    // without one it is kept under a fresh name anyway, because a child component
                    // placed in the loop keys its instance by the position.
                    let pos = match index {
                        Some(i) => *i,
                        None => {
                            let n = *next;
                            *next += 1;
                            self.interner.intern(&format!("__lgx_pos{n}"))
                        }
                    };
                    out.push(Stmt::Let {
                        var: pos,
                        ty: None,
                        value: self.te_int(crate::ast::view::LOOP_INDEX_BASE),
                        mutable: true,
                    });
                    // ★ THE ROW, first thing in each iteration: the item's `keyed by` value or,
                    // unkeyed, its position — after the enclosing loop's row, when there is one.
                    let row = self.interner.intern(&format!("__lgx_row{}", *next));
                    *next += 1;
                    let blank = self.interner.intern("");
                    let own_row = match key {
                        Some(k) => self.te_bin(BinaryOpKind::Add, self.te_text(blank), k),
                        None => {
                            self.te_bin(BinaryOpKind::Add, self.te_text(blank), self.te_id(pos))
                        }
                    };
                    let row_value = match trail.row {
                        Some(outer) => {
                            let hash = self.interner.intern("#");
                            let joined = self.te_bin(
                                BinaryOpKind::Add,
                                self.te_id(outer),
                                self.te_text(hash),
                            );
                            self.te_bin(BinaryOpKind::Add, joined, own_row)
                        }
                        None => own_row,
                    };
                    stmts.push(Stmt::Let {
                        var: row,
                        ty: None,
                        value: row_value,
                        mutable: false,
                    });
                    // The row's key is taken by the first element inside — saved and restored
                    // around the walk so a nested loop cannot leak its key outward.
                    let outer_key = self.lgx_pending_key.take();
                    self.lgx_pending_key = *key;
                    // A loop over a list of views binds each item as a View.
                    let views_in_loop = self.lgx_views.locals.len();
                    if let crate::ast::stmt::Pattern::Identifier(item) = pattern {
                        if self.view_list_typed(iterable, owner) {
                            self.lgx_views.locals.push(*item);
                        }
                    }
                    self.lower_lgx_nodes(
                        body,
                        next,
                        &mut stmts,
                        all,
                        owner,
                        &trail.in_loop(&format!("{npath}."), row),
                    );
                    self.lgx_views.locals.truncate(views_in_loop);
                    self.lgx_pending_key = outer_key;
                    stmts.push(Stmt::Set {
                        target: pos,
                        value: self.te_bin(BinaryOpKind::Add, self.te_id(pos), self.te_int(1)),
                    });
                    let loop_body = self.alloc_block(stmts);
                    out.push(Stmt::Repeat {
                        pattern: pattern.clone(),
                        iterable,
                        body: loop_body,
                    });
                }
                // `{let x be e}` is a plain binding, in scope for the siblings that follow.
                ViewNode::Let { var, value, .. } => {
                    if self.view_typed(value, owner) {
                        self.lgx_views.locals.push(*var);
                    }
                    out.push(Stmt::Let {
                        var: *var,
                        ty: None,
                        value,
                        mutable: false,
                    });
                }
                // ★ `<Child prop={x}/>` DRAWS AN INSTANCE AS A FRAGMENT. The child's render writes
                // its nodes between `vdFragOpen`/`vdFragClose` of the instance's own fragment, so
                // the commit matches the instance by identity wherever it moves, and an instance
                // that stops being drawn is reported gone. A wrapper's children are drawn HERE,
                // in the scope they were written in, as a range the child places at its
                // `{children}`.
                ViewNode::Component {
                    name,
                    props,
                    children,
                    ..
                } if trail.serving => {
                    let Some(child) = all.iter().find(|d| d.name == *name) else {
                        continue;
                    };
                    let serve = self
                        .interner
                        .intern(&serve_fn_name(self.interner.resolve(*name)));
                    let depth = match trail.serve_depth {
                        Some(d) => self.te_bin(BinaryOpKind::Add, self.te_id(d), self.te_int(1)),
                        None => self.te_int(1),
                    };
                    let mut args = vec![depth];
                    for (prop, ty) in &child.props {
                        if is_callback_type(ty, self.interner) {
                            continue;
                        }
                        let (_, value) = props.iter().find(|(k, _)| k == prop).expect(
                            "resolve_views refuses a placement that leaves out a declared prop",
                        );
                        args.push(*value);
                    }
                    args.push(self.slot_range(
                        children,
                        next,
                        out,
                        all,
                        owner,
                        &trail.branch(&format!("{npath}.")),
                    ));
                    args.extend(self.block_slot_zeros(child));
                    out.push(Stmt::Call {
                        function: serve,
                        args,
                    });
                }
                ViewNode::Component {
                    name,
                    props,
                    children,
                    ..
                } => {
                    let Some(child) = all.iter().find(|d| d.name == *name) else {
                        continue;
                    };
                    let fname = self
                        .interner
                        .intern(&render_fn_name(self.interner.resolve(*name)));
                    // ★ WHICH INSTANCE THIS PLACEMENT DRAWS: this component's own key, this
                    // placement's static position in the view, and — inside a `{for each}` — the
                    // row. Each part answers a way two instances could collide: two rows each
                    // placing a child, two placements side by side, every iteration of one loop.
                    let child_key = {
                        let own = self.key_expr();
                        let here = self.interner.intern(&format!("/{npath}"));
                        let base = self.te_bin(BinaryOpKind::Add, own, self.te_text(here));
                        self.with_row(base, trail)
                    };
                    let key_local = self.lgx_temp(next);
                    out.push(Stmt::Let {
                        var: key_local,
                        ty: None,
                        value: child_key,
                        mutable: false,
                    });
                    let child_key = self.te_id(key_local);
                    // Props in the CHILD's declared order; a CALLBACK prop is not an argument —
                    // the placement records which action it means against the child's key.
                    let frag = self.instance_frag(child.name, child_key, next, out);
                    let mut args = vec![child_key, self.te_id(frag)];
                    for (prop, ty) in &child.props {
                        let (_, value) = props.iter().find(|(k, _)| k == prop).expect(
                            "resolve_views refuses a placement that leaves out a declared prop",
                        );
                        if is_callback_type(ty, self.interner) {
                            let identity = self.callback_prop_registration(
                                value, owner, *name, *prop, child_key, all,
                            );
                            out.extend(identity);
                        } else {
                            args.push(value);
                        }
                    }
                    args.push(self.slot_range(
                        children,
                        next,
                        out,
                        all,
                        owner,
                        &trail.branch(&format!("{npath}.")),
                    ));
                    args.extend(self.block_slot_zeros(child));
                    out.push(self.vd_call("vdFragOpen", vec![self.te_id(frag)]));
                    out.push(Stmt::Call {
                        function: fname,
                        args,
                    });
                    out.push(self.vd_call("vdFragClose", Vec::new()));
                }
                // ★ `{children}` — the caller's nodes, drawn by the caller as a range, placed here.
                // A placement that gave none passes `0`, and a range is never position 0.
                ViewNode::Children { .. } => {
                    let slot = self.interner.intern(render_slot_param());
                    let given = self.te_bin(BinaryOpKind::Gt, self.te_id(slot), self.te_int(0));
                    let place = self.vd_call("vdSlot", vec![self.te_id(slot)]);
                    out.push(Stmt::If {
                        cond: given,
                        then_block: self.alloc_block(vec![place]),
                        else_block: None,
                    });
                }
                // `{inspect v:} {when Pat:} … {end}` is the statement of the same name, with each
                // arm's subtree as its body.
                ViewNode::Inspect {
                    scrutinee, arms, ..
                } => {
                    let mut match_arms = Vec::with_capacity(arms.len());
                    let mut has_otherwise = false;
                    for arm in arms {
                        let mut stmts = Vec::new();
                        self.lower_lgx_nodes(
                            &arm.body,
                            next,
                            &mut stmts,
                            all,
                            owner,
                            &trail.branch(&format!("{npath}.")),
                        );
                        has_otherwise |= arm.variant.is_none();
                        match_arms.push(crate::ast::stmt::MatchArm {
                            enum_name: None,
                            variant: arm.variant,
                            bindings: arm.bindings.clone(),
                            body: self.alloc_block(stmts),
                        });
                    }
                    out.push(Stmt::Inspect {
                        target: scrutinee,
                        arms: match_arms,
                        has_otherwise,
                    });
                }
                // ★ `{block X}` IN A LAYOUT IS A SLOT: what the extending view drew for it arrives
                // as the range `__lgx_block_X` and is placed whole; with nothing given (`0`) the
                // layout's own default is drawn here, on a branch trail like any conditional body.
                ViewNode::Block { name, default, .. } => {
                    let param = self
                        .interner
                        .intern(&block_slot_param(self.interner.resolve(*name)));
                    let filled = self.te_bin(BinaryOpKind::Gt, self.te_id(param), self.te_int(0));
                    let take = vec![self.vd_call("vdSlot", vec![self.te_id(param)])];
                    let mut fallback = Vec::new();
                    if let Some(body) = default {
                        self.lower_lgx_nodes(
                            body,
                            next,
                            &mut fallback,
                            all,
                            owner,
                            &trail.branch(&format!("{npath}.")),
                        );
                    }
                    out.push(Stmt::If {
                        cond: filled,
                        then_block: self.alloc_block(take),
                        else_block: Some(self.alloc_block(fallback)),
                    });
                }
                _ => {}
            }
        }
        self.lgx_views.locals.truncate(views_in_scope);
    }

    /// The statements that append one class to the list being built: a space between pieces,
    /// and no leading space before the first.
    pub(crate) fn class_append(&mut self, list: Symbol, piece: &'a Expr<'a>) -> Vec<Stmt<'a>> {
        let empty = self.interner.intern("");
        let space = self.interner.intern(" ");
        let first = Stmt::Set {
            target: list,
            value: piece,
        };
        let joined = self.te_bin(
            BinaryOpKind::Concat,
            self.te_bin(BinaryOpKind::Concat, self.te_id(list), self.te_text(space)),
            piece,
        );
        let rest = Stmt::Set {
            target: list,
            value: joined,
        };
        vec![Stmt::If {
            cond: self.te_bin(BinaryOpKind::Eq, self.te_id(list), self.te_text(empty)),
            then_block: self.alloc_block(vec![first]),
            else_block: Some(self.alloc_block(vec![rest])),
        }]
    }

    /// ★ THE ONE ELEMENT-MODIFIER EMITTER. Everything an element's open carries — the site
    /// something finds it by, its attributes, directives, `bind`, composed class list, raw
    /// content and listeners — is drawn here, in source order, straight after the open and
    /// before any child, so no path draws a subset of them. `owner` is the component a `bind`,
    /// handler or directive belongs to; `scope` is the `Style:` class the element carries;
    /// `sites_at` is the view path a `bind` records its patch site under, `None` where the walk
    /// records none; `trail` the enclosing loop row a directive or `.once` is identified by.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn emit_element_attrs(
        &mut self,
        tag: Symbol,
        attrs: &[Attr<'a>],
        children: &[ViewNode<'a>],
        owner: &ComponentDef<'a>,
        scope: Option<ElementScope>,
        sites_at: Option<&str>,
        trail: &LowerTrail,
        next: &mut usize,
        out: &mut Vec<Stmt<'a>>,
    ) {
        let owner_name = self.interner.resolve(owner.name).to_string();
        let class_word = self.interner.intern("class");
        // ★ THE SITE THIS ELEMENT IS FOUND BY. A listener's slot is keyed by it, a directive
        // finds its element through it after the commit, and a `bind` patches its value through
        // it — so an element any of those touch carries one: a bound element its instance's
        // stable patch site, any other this paint's own (`paint_anchor`), handed back when the
        // instance paints again.
        let bound = attrs.iter().find_map(|a| match a {
            Attr::Bind { field } => Some(*field),
            _ => None,
        });
        let handled = children
            .iter()
            .any(|c| matches!(c, ViewNode::Handler { .. }));
        let directed = attrs.iter().any(|a| matches!(a, Attr::Directive { .. }));
        let anchor = match (bound, sites_at) {
            _ if trail.serving => None,
            (Some(field), Some(npath)) => {
                let key = format!("bind:{}@{npath}", self.interner.resolve(field));
                Some(self.stable_site(&owner_name, &key, next, out))
            }
            _ if bound.is_some() || handled || directed => Some(self.paint_anchor(next, out)),
            _ => None,
        };
        if let Some(site) = anchor {
            out.push(self.vd_call("vdSite", vec![self.te_id(site)]));
        }
        for attr in attrs {
            match attr {
                // A declared `## Directive` runs against the element it sits on, once the paint
                // that draws it is committed — queued here with this element's site and its value
                // (`drain_directives`).
                Attr::Directive { .. } if trail.serving => {}
                Attr::Directive { name, value } => {
                    let site = anchor.expect("a directed element carries a site");
                    self.queue_directive(*name, site, value, owner, trail, next, out);
                }
                // `bind={field}` shows the field's value and listens for `input`: a keystroke and
                // a driving verb reach the write-back through the same dispatcher arm.
                Attr::Bind { field } => {
                    let shown = self.te_id(*field);
                    let kind = bound_input_kind(tag, attrs, self.interner);
                    let writer = if kind.writes_checked() {
                        self.vd_call("vdSetChecked", vec![shown])
                    } else {
                        let text = self.as_text(shown);
                        self.vd_call("vdSetValue", vec![text])
                    };
                    out.push(writer);
                    if trail.serving {
                        continue;
                    }
                    let site = anchor.expect("a bound element carries a site");
                    let input = self.interner.intern("input");
                    let listener = ListenerSite::Bind {
                        component: owner.name,
                        field: *field,
                        checked: kind.writes_checked(),
                    };
                    let regs = self.register_listener(self.te_id(site), input, listener);
                    out.extend(regs);
                    // ★ A CLICK ON A BOX IS THE BROWSER'S OWN ACT, not a listener the page wrote:
                    // registered so a driving verb's click finds it like a real listener.
                    if kind.writes_checked() {
                        let click = self.interner.intern("click");
                        let toggle = ListenerSite::DefaultToggle {
                            component: owner.name,
                            field: *field,
                            radio: matches!(kind, BoundInput::Radio),
                        };
                        let regs = self.register_listener(self.te_id(site), click, toggle);
                        out.extend(regs);
                    }
                    let input_text = self.te_text(input);
                    out.push(self.vd_call("vdListen", vec![input_text, self.te_int(0)]));
                }
                Attr::Plain { name, value } if *name != class_word => {
                    let spelled = self.interner.resolve(*name).to_string();
                    // `key` is a row's identity, never an attribute: a keyed row carries it on its
                    // open.
                    if spelled == "key" {
                        continue;
                    }
                    // ★ `checked` IS THE LIVE PROPERTY, whatever was written. The attribute is
                    // `defaultChecked`, which a patched box ignores; a Bool decides the property,
                    // and any written text — `checked`, `checked="no"` — checks the box.
                    if spelled.eq_ignore_ascii_case("checked") {
                        let on = match value {
                            Expr::Literal(Literal::Text(_)) | Expr::InterpolatedString(_) => {
                                self.te_bool(true)
                            }
                            other => other,
                        };
                        out.push(self.vd_call("vdSetChecked", vec![on]));
                        continue;
                    }
                    let name_text = self.te_text(*name);
                    // ★ AN HTML BOOLEAN ATTRIBUTE IS A PRESENCE: drawn (empty) while its value
                    // holds and absent otherwise — the commit removes it when a paint stops
                    // drawing it. A text value is the attribute's own value, present.
                    if crate::lgx::html::is_boolean_attribute(&spelled) {
                        match value {
                            Expr::Literal(Literal::Text(_)) | Expr::InterpolatedString(_) => {
                                out.push(self.vd_call("vdSetAttribute", vec![name_text, value]));
                            }
                            _ => {
                                let blank = self.interner.intern("");
                                let present = self.vd_call(
                                    "vdSetAttribute",
                                    vec![name_text, self.te_text(blank)],
                                );
                                out.push(Stmt::If {
                                    cond: value,
                                    then_block: self.alloc_block(vec![present]),
                                    else_block: None,
                                });
                            }
                        }
                    } else if crate::lgx::html::is_url_attribute(&spelled) {
                        // ★ A URL-VALUED ATTRIBUTE IS A NAVIGATION: `javascript:alert(1)` carries
                        // no character escaping touches and runs when the link is followed, so
                        // the scheme is checked — a written URL here, a computed one by `safeUrl`
                        // when the paint runs.
                        let guarded = match value {
                            Expr::Literal(Literal::Text(t))
                                if crate::lgx::html::is_executable_url(
                                    self.interner.resolve(*t),
                                ) =>
                            {
                                let blocked = self.interner.intern("about:blank#blocked");
                                self.te_text(blocked)
                            }
                            Expr::Literal(Literal::Text(_)) => value,
                            other => {
                                let safe = self.interner.intern("safeUrl");
                                let shown = self.as_text(other);
                                self.te_call(safe, vec![shown])
                            }
                        };
                        out.push(self.vd_call("vdSetAttribute", vec![name_text, guarded]));
                    } else {
                        let text = self.as_text(value);
                        out.push(self.vd_call("vdSetAttribute", vec![name_text, text]));
                    }
                }
                // `class="…"` and every `class:name={cond}` compose into ONE list, below.
                Attr::Plain { .. } | Attr::Class { .. } => {}
            }
        }
        // ★ `class="card"` AND EVERY `class:name={cond}` COMPOSE INTO ONE CLASS LIST, in source
        // order, so two toggles cannot each clobber the other — with the component's scope class
        // stamped on the elements it renders. A list decided at compile time is one constant; one
        // decided at run time is built in a local; an empty list draws no attribute at all.
        let has_classes = scope.is_some()
            || attrs.iter().any(|a| {
                matches!(a, Attr::Class { .. })
                    || matches!(a, Attr::Plain { name, .. } if *name == class_word)
            });
        let toggles = attrs.iter().any(|a| matches!(a, Attr::Class { .. }))
            || matches!(scope, Some(ElementScope::Param(_)));
        let base = attrs.iter().find_map(|a| match a {
            Attr::Plain { name, value } if *name == class_word => Some(*value),
            _ => None,
        });
        let folded = match (toggles, base) {
            (true, _) => None,
            (false, None) => Some(String::new()),
            (false, Some(Expr::Literal(Literal::Text(t)))) => {
                Some(self.interner.resolve(*t).to_string())
            }
            (false, Some(_)) => None,
        };
        match folded {
            _ if !has_classes => {}
            Some(mut text) => {
                if let Some(ElementScope::Class(class)) = scope.clone() {
                    if text.is_empty() {
                        text = class;
                    } else {
                        text.push(' ');
                        text.push_str(&class);
                    }
                }
                if !text.is_empty() {
                    let class_text = self.te_text(class_word);
                    let sym = self.interner.intern(&text);
                    let value = self.te_text(sym);
                    out.push(self.vd_call("vdSetAttribute", vec![class_text, value]));
                }
            }
            None => {
                let list = self.lgx_temp(next);
                let empty = self.interner.intern("");
                out.push(Stmt::Let {
                    var: list,
                    ty: None,
                    value: self.te_text(empty),
                    mutable: true,
                });
                for attr in attrs {
                    match attr {
                        Attr::Plain { name, value } if *name == class_word => {
                            let add = self.class_append(list, value);
                            let then_block = self.alloc_block(add);
                            let non_empty =
                                self.te_bin(BinaryOpKind::NotEq, value, self.te_text(empty));
                            out.push(Stmt::If {
                                cond: non_empty,
                                then_block,
                                else_block: None,
                            });
                        }
                        Attr::Class { name, cond } => {
                            let piece = self.te_text(*name);
                            let add = self.class_append(list, piece);
                            let then_block = self.alloc_block(add);
                            out.push(Stmt::If {
                                cond,
                                then_block,
                                else_block: None,
                            });
                        }
                        Attr::Plain { .. } | Attr::Directive { .. } | Attr::Bind { .. } => {}
                    }
                }
                match &scope {
                    Some(ElementScope::Class(class)) => {
                        let class = self.interner.intern(class);
                        let piece = self.te_text(class);
                        let add = self.class_append(list, piece);
                        out.extend(add);
                    }
                    // A template's scope is its placer's: empty when the placing component has no
                    // `Style:`, so it contributes to the list only when it names a class.
                    Some(ElementScope::Param(param)) => {
                        let add = self.class_append(list, self.te_id(*param));
                        let then_block = self.alloc_block(add);
                        let named = self.te_bin(
                            BinaryOpKind::NotEq,
                            self.te_id(*param),
                            self.te_text(empty),
                        );
                        out.push(Stmt::If {
                            cond: named,
                            then_block,
                            else_block: None,
                        });
                    }
                    None => {}
                }
                let class_text = self.te_text(class_word);
                let set = self.vd_call("vdSetAttribute", vec![class_text, self.te_id(list)]);
                let non_empty =
                    self.te_bin(BinaryOpKind::NotEq, self.te_id(list), self.te_text(empty));
                out.push(Stmt::If {
                    cond: non_empty,
                    then_block: self.alloc_block(vec![set]),
                    else_block: None,
                });
            }
        }
        // `{raw h}` is the element's whole content, verbatim — the HTML-interchange escape hatch.
        for child in children {
            if let ViewNode::Raw { html, .. } = child {
                let shown = self.as_text(html);
                out.push(self.vd_call("vdSetHtml", vec![shown]));
            }
        }
        // `on-<event>={action}` registers this element as answering the event, as DATA: the
        // dispatcher's arm for this site runs the action's turn. The element listens, with its
        // modifiers' behaviour bits, for as long as a paint draws the handler.
        for child in children {
            let ViewNode::Handler {
                event,
                body,
                modifiers,
                ..
            } = child
            else {
                continue;
            };
            if trail.serving {
                continue;
            }
            let Some(Stmt::Call { function, args }) = body.first() else {
                continue;
            };
            let site = anchor.expect("a handled element carries a site");
            // A handler names one of this component's actions or a CALLBACK prop it was handed.
            let is_prop = owner.props.iter().any(|(p, _)| p == function);
            let key = self.handler_key(modifiers);
            // ★ `.once` IS COMPILED LOGOS: the listener's identity — this instance, this static
            // site, this row — answers only while unspent, and a paint that finds it spent draws
            // no listener, so a spent listener is gone however its element is redrawn.
            let once = if modifiers.contains(&EventModifier::Behaviour(Behaviour::Once)) {
                let id = self.lgx_temp(next);
                let site_index = self
                    .interner
                    .intern(&format!("\u{1}{}", self.lgx_listeners.len()));
                let own = self.key_expr();
                let base = self.te_bin(BinaryOpKind::Add, own, self.te_text(site_index));
                let identity = self.with_row(base, trail);
                out.push(Stmt::Let {
                    var: id,
                    ty: None,
                    value: identity,
                    mutable: false,
                });
                Some(id)
            } else {
                None
            };
            let listener = if is_prop {
                ListenerSite::CallbackProp {
                    component: owner.name,
                    prop: *function,
                    argc: args.len(),
                    key,
                    once,
                }
            } else {
                ListenerSite::Action {
                    component: owner.name,
                    action: *function,
                    argc: args.len(),
                    carries_value: false,
                    key,
                    once,
                }
            };
            let mut registration =
                self.register_listener_with(self.te_id(site), *event, listener, args);
            let bits = EventModifier::host_listen_bits(modifiers);
            let event_text = self.te_text(*event);
            registration.push(self.vd_call("vdListen", vec![event_text, self.te_int(bits as i64)]));
            match once {
                Some(id) => {
                    let spent = self.interner.intern(once_spent_binding());
                    let is_spent = self.ctx.alloc_imperative_expr(Expr::Contains {
                        collection: self.te_id(spent),
                        value: self.te_id(id),
                    });
                    let unspent = self
                        .ctx
                        .alloc_imperative_expr(Expr::Not { operand: is_spent });
                    let then_block = self.alloc_block(registration);
                    out.push(Stmt::If {
                        cond: unspent,
                        then_block,
                        else_block: None,
                    });
                }
                None => out.extend(registration),
            }
        }
    }

    /// The `0` a caller hands each of `def`'s `{block X}` slots ([`slot_blocks`]) when it draws
    /// `def` other than by `{extends}` — a placement, a mount, a repaint — so the layout draws its
    /// own defaults. Empty for a component that declares no slot.
    pub(crate) fn block_slot_zeros(&self, def: &ComponentDef<'a>) -> Vec<&'a Expr<'a>> {
        slot_blocks(def).iter().map(|_| self.te_int(0)).collect()
    }

    /// The range a placement's children are drawn into, as the slot argument its child receives:
    /// the tape position of a range (`vdRangeOpen`) the child places at its `{children}`, or `0`
    /// when the placement gives none.
    ///
    /// ★ DRAWN HERE, IN THE CALLER'S SCOPE, the names in them meaning what they mean here — and
    /// set apart from the tape around them until the child's slot places them, so they land
    /// exactly where the child says and nowhere else.
    pub(crate) fn slot_range(
        &mut self,
        children: &[ViewNode<'a>],
        next: &mut usize,
        out: &mut Vec<Stmt<'a>>,
        all: &[ComponentDef<'a>],
        owner: &ComponentDef<'a>,
        trail: &LowerTrail,
    ) -> &'a Expr<'a> {
        if children.is_empty() {
            return self.te_int(0);
        }
        let range = self.lgx_temp(next);
        let open = self.vd_expr("vdRangeOpen", Vec::new());
        out.push(Stmt::Let {
            var: range,
            ty: None,
            value: open,
            mutable: false,
        });
        // ★ THE CALLER'S TRAIL, OPAQUE. The children's addresses are the CALLER's — they were
        // written there, under the placement at `trail`, so an instance placed among them is keyed
        // apart from the placement around it — but they are drawn under the child, so they record
        // no patch site a turn of the caller could write into the wrong node.
        self.lower_lgx_nodes(children, next, out, all, owner, trail);
        out.push(self.vd_call("vdRangeClose", Vec::new()));
        self.te_id(range)
    }

    /// A sentence waiting on the element `selector` names among this instance's drawn records:
    /// found on the tape (`vdQueryText` from `__lgx_from`), given a site when it carries none, each
    /// `answers` event registered against that site, and each `listens` event (with its behaviour
    /// flags) heard on the element beside the tape. A selector naming nothing drawn registers
    /// nothing, as a query that finds nothing does.
    pub(crate) fn sentence_on_element(
        &mut self,
        selector: &'a Expr<'a>,
        listens: &[(Symbol, i64)],
        answers: &[(Symbol, ListenerSite)],
        next: &mut usize,
        out: &mut Vec<Stmt<'a>>,
    ) {
        let at = self.lgx_temp(next);
        let from = self.interner.intern(tape_from_local());
        let text = self.as_text(selector);
        let found = self.vd_expr("vdQueryText", vec![self.te_id(from), text]);
        out.push(Stmt::Let {
            var: at,
            ty: None,
            value: found,
            mutable: false,
        });
        let site = self.lgx_temp(next);
        let carried = self.vd_expr("vdTapeSite", vec![self.te_id(at)]);
        let mut inner = vec![Stmt::Let {
            var: site,
            ty: None,
            value: carried,
            mutable: true,
        }];
        let frag = self.interner.intern(render_frag_param());
        let fresh = self.vd_expr("vdAnchor", vec![self.te_id(frag)]);
        let mint = vec![
            Stmt::Set {
                target: site,
                value: fresh,
            },
            self.vd_call("vdSiteAt", vec![self.te_id(at), self.te_id(site)]),
        ];
        let unmarked = self.te_bin(BinaryOpKind::Eq, self.te_id(site), self.te_int(0));
        inner.push(Stmt::If {
            cond: unmarked,
            then_block: self.alloc_block(mint),
            else_block: None,
        });
        for (event, listener) in answers {
            inner.extend(self.register_listener(self.te_id(site), *event, listener.clone()));
        }
        for (event, flags) in listens {
            let event_text = self.te_text(*event);
            inner.push(self.vd_call(
                "vdListenAt",
                vec![self.te_id(at), event_text, self.te_int(*flags)],
            ));
        }
        let drawn = self.te_bin(BinaryOpKind::Gt, self.te_id(at), self.te_int(0));
        out.push(Stmt::If {
            cond: drawn,
            then_block: self.alloc_block(inner),
            else_block: None,
        });
    }

    /// A sentence waiting on the page itself — a key anywhere, the page's scroll, a window event a
    /// test raises there: registered against the page node's own site (the node the retained tree
    /// knows the page by, made when the page is mounted elsewhere), and each `listens` event heard
    /// on the page. Neither is part of any paint's diff, so each paint says them again.
    pub(crate) fn sentence_on_page(
        &mut self,
        listens: &[(Symbol, i64)],
        answers: &[(Symbol, ListenerSite)],
        next: &mut usize,
        out: &mut Vec<Stmt<'a>>,
    ) {
        let wire = self.lgx_temp(next);
        let query = self.interner.intern("querySelector");
        let body_sym = self.interner.intern("body");
        let page = self.te_call(query, vec![self.te_text(body_sym)]);
        out.push(Stmt::Let {
            var: wire,
            ty: None,
            value: page,
            mutable: false,
        });
        let node = self.lgx_temp(next);
        let known = self.vd_expr("vdNodeOfWire", vec![self.te_id(wire)]);
        out.push(Stmt::Let {
            var: node,
            ty: None,
            value: known,
            mutable: true,
        });
        let unknown = self.te_bin(BinaryOpKind::Eq, self.te_id(node), self.te_int(-1));
        let root = self.vd_expr("vdNewRoot", vec![self.te_id(wire)]);
        out.push(Stmt::If {
            cond: unknown,
            then_block: self.alloc_block(vec![Stmt::Set {
                target: node,
                value: root,
            }]),
            else_block: None,
        });
        let site = self.lgx_temp(next);
        let ensured = self.vd_expr("vdEnsureSite", vec![self.te_id(node)]);
        out.push(Stmt::Let {
            var: site,
            ty: None,
            value: ensured,
            mutable: false,
        });
        for (event, listener) in answers {
            out.extend(self.register_listener(self.te_id(site), *event, listener.clone()));
        }
        for (event, flags) in listens {
            out.push(self.listen_flags_call(self.te_id(wire), *event, *flags as u8));
        }
    }

    /// The window's one listener for `event`, attached the first time any instance of `component`
    /// renders handler `index`: it hands the event to the page node, where every instance waiting
    /// for it has registered (`sentence_on_page`).
    pub(crate) fn window_forwarder(
        &mut self,
        component: Symbol,
        index: usize,
        event: Symbol,
    ) -> Stmt<'a> {
        let flag = self.forwarded_flag(component, index);
        let dispatch = self.interner.intern(dispatch_node_fn_name());
        let query = self.interner.intern("querySelector");
        let body_sym = self.interner.intern("body");
        let page = self.te_call(query, vec![self.te_text(body_sym)]);
        let blank = self.interner.intern("");
        let hand = Stmt::Call {
            function: dispatch,
            args: vec![page, self.te_text(event), self.te_text(blank)],
        };
        let closure = self.listener_closure(hand);
        let on_window = self.interner.intern("onWindowEvent");
        let attach = vec![
            Stmt::Set {
                target: flag,
                value: self.te_bool(true),
            },
            Stmt::Call {
                function: on_window,
                args: vec![self.te_text(event), closure],
            },
        ];
        let unheard = self.ctx.alloc_imperative_expr(Expr::Not {
            operand: self.te_id(flag),
        });
        Stmt::If {
            cond: unheard,
            then_block: self.alloc_block(attach),
            else_block: None,
        }
    }

    /// The relay's one subscription to `topic`, made the first time any instance of `component`
    /// renders handler `index`: each frame goes to the page node as `event`, where every instance
    /// waiting on the topic has registered.
    pub(crate) fn topic_forwarder(
        &mut self,
        component: Symbol,
        index: usize,
        topic: &'a Expr<'a>,
        event: Symbol,
    ) -> Stmt<'a> {
        let flag = self.forwarded_flag(component, index);
        let dispatch = self.interner.intern(dispatch_node_fn_name());
        let query = self.interner.intern("querySelector");
        let body_sym = self.interner.intern("body");
        let page = self.te_call(query, vec![self.te_text(body_sym)]);
        let value_of = self.interner.intern("eventValue");
        let ev = self.interner.intern("__lgx_event");
        let carried = self.te_call(value_of, vec![self.te_id(ev)]);
        let hand = Stmt::Call {
            function: dispatch,
            args: vec![page, self.te_text(event), carried],
        };
        let closure = self.listener_closure(hand);
        let on_message = self.interner.intern("onMessage");
        let attach = vec![
            Stmt::Set {
                target: flag,
                value: self.te_bool(true),
            },
            Stmt::Call {
                function: on_message,
                args: vec![topic, closure],
            },
        ];
        let unheard = self.ctx.alloc_imperative_expr(Expr::Not {
            operand: self.te_id(flag),
        });
        Stmt::If {
            cond: unheard,
            then_block: self.alloc_block(attach),
            else_block: None,
        }
    }

    /// `__LGX_HEARD_<C>_<i>`, declared `false` the first time it is asked for.
    fn forwarded_flag(&mut self, component: Symbol, index: usize) -> Symbol {
        let name = forwarded_binding(self.interner.resolve(component), index);
        let flag = self.interner.intern(&name);
        let declared = self
            .deferred_functions
            .iter()
            .any(|s| matches!(s, Stmt::Let { var, .. } if *var == flag));
        if !declared {
            let value = self.te_bool(false);
            self.deferred_functions.push(Stmt::Let {
                var: flag,
                ty: None,
                value,
                mutable: true,
            });
        }
        flag
    }

    /// `base + "#" + <row>` inside a `{for each}` (see [`LowerTrail::row`]), `base` outside one.
    pub(crate) fn with_row(&mut self, base: &'a Expr<'a>, trail: &LowerTrail) -> &'a Expr<'a> {
        let Some(row) = trail.row else { return base };
        let hash = self.interner.intern("#");
        let with_hash = self.te_bin(BinaryOpKind::Add, base, self.te_text(hash));
        self.te_bin(BinaryOpKind::Add, with_hash, self.te_id(row))
    }

    /// A fresh name for a node the lowering builds. Prefixed so it cannot collide with anything
    /// the author wrote.
    pub(crate) fn lgx_temp(&mut self, next: &mut usize) -> Symbol {
        let name = self.interner.intern(&format!("__lgx_n{next}"));
        *next += 1;
        name
    }

    /// Coerce a hole's value to TEXT before it reaches a DOM call.
    ///
    /// The tree-walker's DOM builtins accept any value and call `to_display_string` themselves.
    /// The WASM imports do not: they take a Text HANDLE, an `i32`. Handing them an `Int` produces
    /// a module that compiles and then fails to VALIDATE — "expected i32, found i64" — which is
    /// invisible until something actually instantiates it. Wrapping in a one-part interpolation
    /// is the same conversion the surrounding text would have done anyway, and it is a no-op on
    /// a value that is already Text.
    pub(crate) fn as_text(&self, value: &'a Expr<'a>) -> &'a Expr<'a> {
        if matches!(
            value,
            Expr::Literal(Literal::Text(_)) | Expr::InterpolatedString(_)
        ) {
            return value;
        }
        self.ctx
            .alloc_imperative_expr(Expr::InterpolatedString(vec![
                crate::ast::stmt::StringPart::Expr {
                    value,
                    format_spec: None,
                    debug: false,
                },
            ]))
    }

    /// The `id` an element names itself by when it is written as a literal — the name a repaint
    /// finds it by among its siblings.
    fn literal_id(&mut self, attrs: &[Attr<'a>]) -> Option<String> {
        let id_word = self.interner.intern("id");
        attrs.iter().find_map(|a| match a {
            Attr::Plain {
                name,
                value: Expr::Literal(Literal::Text(t)),
            } if *name == id_word => Some(self.interner.resolve(*t).to_string()),
            _ => None,
        })
    }

    /// A `Text` literal expression from an already-interned word.
    pub(crate) fn te_text(&self, sym: Symbol) -> &'a Expr<'a> {
        self.ctx
            .alloc_imperative_expr(Expr::Literal(Literal::Text(sym)))
    }

    /// A typed empty `Seq of <elem>` — `New`, not a literal: an empty literal has nothing to infer
    /// its element kind from, and every reader of the collection needs a real one.
    pub(crate) fn new_seq_expr(&mut self, elem: Symbol) -> &'a Expr<'a> {
        let seq_name = self.interner.intern("Seq");
        self.ctx.alloc_imperative_expr(Expr::New {
            type_name: seq_name,
            type_args: vec![TypeExpr::Primitive(elem)],
            init_fields: Vec::new(),
        })
    }
}
