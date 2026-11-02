use super::render::LowerTrail;
use super::*;
use crate::ast::component::{ComponentDef, Trigger};
use crate::ast::stmt::{BinaryOpKind, Expr, Stmt, TypeExpr};
use crate::ast::view::{Attr, ViewNode};
use logicaffeine_base::Symbol;

impl<'a, 'ctx, 'int> super::super::Parser<'a, 'ctx, 'int> {
    /// `__lgx_tick(ms)` — the page's clock (`__LGX_NOW`) moves forward by `ms`, and every timer the
    /// time that passed makes due fires, one at a time, earliest first.
    ///
    /// Each round finds the running timer — any timer, any instance — with the earliest due time
    /// not after the end of the step, the one armed first among equals (`__SEQ_`); moves the clock
    /// to it; re-arms it a period on (`__PERIOD_`) or, for a one-shot, stops it (`-1`) BEFORE its
    /// handler runs, so a `Stop the timer …` in the handler is what stands; and runs its turn. A
    /// timer armed in that turn counts from the moment it fired. When nothing more is due the clock
    /// stands at the end of the step, and the host is asked to wake the page for the next one.
    pub(crate) fn lower_lgx_tick(&mut self) -> Option<Stmt<'a>> {
        if self.lgx_timers.is_empty() {
            return None;
        }
        let ms = self.interner.intern("__lgx_ms");
        let until = self.interner.intern("__lgx_until");
        let best = self.interner.intern("__lgx_best");
        let best_seq = self.interner.intern("__lgx_best_seq");
        let which = self.interner.intern("__lgx_which");
        let best_k = self.interner.intern("__lgx_best_k");
        let k = self.interner.intern("__lgx_k");
        let d = self.interner.intern("__lgx_d");
        let seq_of = self.interner.intern("__lgx_s");
        let now = self.interner.intern(now_binding());
        let blank = self.interner.intern("");
        let timers = self.lgx_timers.clone();
        let mut round: Vec<Stmt<'a>> = vec![
            Stmt::Let {
                var: best,
                ty: None,
                value: self.te_int(-1),
                mutable: true,
            },
            Stmt::Let {
                var: best_seq,
                ty: None,
                value: self.te_int(0),
                mutable: true,
            },
            Stmt::Let {
                var: which,
                ty: None,
                value: self.te_int(0),
                mutable: true,
            },
            Stmt::Let {
                var: best_k,
                ty: None,
                value: self.te_text(blank),
                mutable: true,
            },
        ];
        for (j, (component, index, _, _)) in timers.iter().enumerate() {
            let name = self.interner.resolve(*component).to_string();
            let due = self.interner.intern(&due_binding(&name, *index));
            let seq = self.interner.intern(&seq_binding(&name, *index));
            let keys = self.interner.intern(&keys_binding(&name));
            let read_due = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(due),
                index: self.te_id(k),
            });
            let read_seq = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(seq),
                index: self.te_id(k),
            });
            let running = self.te_bin(BinaryOpKind::GtEq, self.te_id(d), self.te_int(0));
            let in_step = self.te_bin(BinaryOpKind::LtEq, self.te_id(d), self.te_id(until));
            let candidate = self.te_bin(BinaryOpKind::And, running, in_step);
            let none_yet = self.te_bin(BinaryOpKind::Lt, self.te_id(best), self.te_int(0));
            let earlier = self.te_bin(BinaryOpKind::Lt, self.te_id(d), self.te_id(best));
            let same_time = self.te_bin(BinaryOpKind::Eq, self.te_id(d), self.te_id(best));
            let armed_first =
                self.te_bin(BinaryOpKind::Lt, self.te_id(seq_of), self.te_id(best_seq));
            let tie_won = self.te_bin(BinaryOpKind::And, same_time, armed_first);
            let wins = self.te_bin(BinaryOpKind::Or, none_yet, earlier);
            let wins = self.te_bin(BinaryOpKind::Or, wins, tie_won);
            let take = self.alloc_block(vec![
                Stmt::Set {
                    target: best,
                    value: self.te_id(d),
                },
                Stmt::Set {
                    target: best_seq,
                    value: self.te_id(seq_of),
                },
                Stmt::Set {
                    target: which,
                    value: self.te_int(j as i64 + 1),
                },
                Stmt::Set {
                    target: best_k,
                    value: self.te_id(k),
                },
            ]);
            let consider = self.alloc_block(vec![
                Stmt::Let {
                    var: seq_of,
                    ty: None,
                    value: read_seq,
                    mutable: false,
                },
                Stmt::If {
                    cond: wins,
                    then_block: take,
                    else_block: None,
                },
            ]);
            let armed = self.ctx.alloc_imperative_expr(Expr::Contains {
                collection: self.te_id(due),
                value: self.te_id(k),
            });
            let per_key = self.alloc_block(vec![Stmt::If {
                cond: armed,
                then_block: self.alloc_block(vec![
                    Stmt::Let {
                        var: d,
                        ty: None,
                        value: read_due,
                        mutable: false,
                    },
                    Stmt::If {
                        cond: candidate,
                        then_block: consider,
                        else_block: None,
                    },
                ]),
                else_block: None,
            }]);
            round.push(Stmt::Repeat {
                pattern: crate::ast::stmt::Pattern::Identifier(k),
                iterable: self.te_id(keys),
                body: per_key,
            });
        }
        let nothing_due = self.te_bin(BinaryOpKind::Lt, self.te_id(best), self.te_int(0));
        round.push(Stmt::If {
            cond: nothing_due,
            then_block: self.alloc_block(vec![Stmt::Break]),
            else_block: None,
        });
        round.push(Stmt::Set {
            target: now,
            value: self.te_id(best),
        });
        for (j, (component, index, repeating, _)) in timers.iter().enumerate() {
            let name = self.interner.resolve(*component).to_string();
            let due = self.interner.intern(&due_binding(&name, *index));
            let period = self.interner.intern(&period_binding(&name, *index));
            let act = self
                .interner
                .intern(&action_fn_name(&name, &format!("__e{index}")));
            let next = if *repeating {
                let period_ms = self.ctx.alloc_imperative_expr(Expr::Index {
                    collection: self.te_id(period),
                    index: self.te_id(best_k),
                });
                self.te_bin(BinaryOpKind::Add, self.te_id(best), period_ms)
            } else {
                self.te_int(-1)
            };
            let fire = self.alloc_block(vec![
                Stmt::SetIndex {
                    collection: self.te_id(due),
                    index: self.te_id(best_k),
                    value: next,
                },
                Stmt::Call {
                    function: act,
                    args: vec![self.te_id(best_k)],
                },
            ]);
            let this_one = self.te_bin(
                BinaryOpKind::Eq,
                self.te_id(which),
                self.te_int(j as i64 + 1),
            );
            round.push(Stmt::If {
                cond: this_one,
                then_block: fire,
                else_block: None,
            });
        }
        let step_end = self.te_bin(BinaryOpKind::Add, self.te_id(now), self.te_id(ms));
        let rearm = self.interner.intern(rearm_fn_name());
        let mut out: Vec<Stmt<'a>> = vec![
            Stmt::Let {
                var: until,
                ty: None,
                value: step_end,
                mutable: false,
            },
            Stmt::While {
                cond: self.te_bool(true),
                body: self.alloc_block(round),
                decreasing: None,
            },
            Stmt::Set {
                target: now,
                value: self.te_id(until),
            },
            Stmt::Call {
                function: rearm,
                args: Vec::new(),
            },
        ];
        // ★ IT REPORTS "ANSWERED" LIKE A DRIVING VERB, because `Advance time by …` reaches it
        // through the same `compile_lgx_drive` the other drivers use — which reads a 0/1/2 code and
        // raises "nothing answers a timer on that selector" for anything else. A clock that moved
        // and found nothing due has not failed; it has done its job.
        let two = self.te_int(2);
        out.push(Stmt::Return { value: Some(two) });
        let int_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Int")));
        let name = self.interner.intern(tick_fn_name());
        Some(Stmt::FunctionDef {
            name,
            generics: Vec::new(),
            params: vec![(ms, int_ty)],
            body: self.alloc_block(out),
            return_type: Some(int_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        })
    }

    /// `__lgx_rearm()` — ask the host to wake the page when its earliest running timer comes due
    /// (`wakeAfter`), or, with nothing running, not at all (`wakeAfter(-1)`).
    pub(crate) fn lower_lgx_rearm(&mut self) -> Option<Stmt<'a>> {
        if self.lgx_timers.is_empty() {
            return None;
        }
        let earliest = self.interner.intern("__lgx_earliest");
        let k = self.interner.intern("__lgx_k");
        let d = self.interner.intern("__lgx_d");
        let now = self.interner.intern(now_binding());
        let mut body: Vec<Stmt<'a>> = vec![Stmt::Let {
            var: earliest,
            ty: None,
            value: self.te_int(-1),
            mutable: true,
        }];
        for (component, index, _, _) in self.lgx_timers.clone() {
            let name = self.interner.resolve(component).to_string();
            let due = self.interner.intern(&due_binding(&name, index));
            let keys = self.interner.intern(&keys_binding(&name));
            let read_due = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(due),
                index: self.te_id(k),
            });
            let running = self.te_bin(BinaryOpKind::GtEq, self.te_id(d), self.te_int(0));
            let none_yet = self.te_bin(BinaryOpKind::Lt, self.te_id(earliest), self.te_int(0));
            let earlier = self.te_bin(BinaryOpKind::Lt, self.te_id(d), self.te_id(earliest));
            let first = self.te_bin(BinaryOpKind::Or, none_yet, earlier);
            let wins = self.te_bin(BinaryOpKind::And, running, first);
            let armed = self.ctx.alloc_imperative_expr(Expr::Contains {
                collection: self.te_id(due),
                value: self.te_id(k),
            });
            let per_key = self.alloc_block(vec![Stmt::If {
                cond: armed,
                then_block: self.alloc_block(vec![
                    Stmt::Let {
                        var: d,
                        ty: None,
                        value: read_due,
                        mutable: false,
                    },
                    Stmt::If {
                        cond: wins,
                        then_block: self.alloc_block(vec![Stmt::Set {
                            target: earliest,
                            value: self.te_id(d),
                        }]),
                        else_block: None,
                    },
                ]),
                else_block: None,
            }]);
            body.push(Stmt::Repeat {
                pattern: crate::ast::stmt::Pattern::Identifier(k),
                iterable: self.te_id(keys),
                body: per_key,
            });
        }
        let wake = self.interner.intern("wakeAfter");
        let none = self.te_bin(BinaryOpKind::Lt, self.te_id(earliest), self.te_int(0));
        let delay = self.te_bin(
            BinaryOpKind::Subtract,
            self.te_id(earliest),
            self.te_id(now),
        );
        body.push(Stmt::If {
            cond: none,
            then_block: self.alloc_block(vec![Stmt::Call {
                function: wake,
                args: vec![self.te_int(-1)],
            }]),
            else_block: Some(self.alloc_block(vec![Stmt::Call {
                function: wake,
                args: vec![delay],
            }])),
        });
        Some(Stmt::FunctionDef {
            name: self.interner.intern(rearm_fn_name()),
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

    /// Open a paint: step the pass counter, so everything drawn in it stamps a new number.
    pub(crate) fn open_pass(&mut self) -> Vec<Stmt<'a>> {
        if self.lgx_sweeps.is_empty()
            && !self.lgx_all_components.iter().any(keeps_instance_registry)
            && self.removable_directives().is_empty()
        {
            return Vec::new();
        }
        let pass = self.interner.intern(pass_binding());
        let one = self.te_int(1);
        let next = self.te_bin(BinaryOpKind::Add, self.te_id(pass), one);
        vec![Stmt::Set {
            target: pass,
            value: next,
        }]
    }

    /// Whether any instance can leave the document: only a PLACED component's instance can — a
    /// mounted root stays for the life of the page — so a program that places none needs no
    /// fragment registry and no unmount sweep, and ships neither.
    pub(crate) fn instances_can_leave(&self) -> bool {
        !self.lgx_placed.is_empty()
    }

    /// ★ THE UNMOUNT SWEEP, as one function the paint calls after its commit: `__lgx_gone()`.
    ///
    /// The commit reports every fragment it removed (`vdGone`); each fragment was drawn for one
    /// instance (`__LGX_FRAGC`/`__LGX_FRAGK`), so the instances that left are exactly known — no
    /// stamps, no scope. Gone means GONE, not marked: every registry the instance kept loses its
    /// key, its listener lookups and sites go back, and a `When this component disappears:` handler
    /// is told.
    ///
    /// Three phases, strictly in order, because a handler can paint again — calling this very
    /// function, nested — before this one returns:
    ///
    ///   1. TAKE. The gone list is read into locals and cleared, and each fragment's owner record
    ///      emptied, so a nested sweep sees only what ITS commit removed.
    ///   2. WRITE, NO HANDLERS. Every `__KEYS_<C>` loses its gone keys — before any author code runs,
    ///      so a nested walk of a key list can never rediscover one.
    ///   3. HANDLERS, THEN RETIREMENT, per gone instance: marked told, its handler run with every
    ///      registry still intact (a `Disappears` action reads its own last state like any other),
    ///      then every registry entry it leaves behind removed.
    pub(crate) fn lower_lgx_gone(&mut self) -> Stmt<'a> {
        let int_name = self.interner.intern("Int");
        let text_name = self.interner.intern("Text");
        let (whos, keys) = (
            self.interner.intern("__lgx_gw"),
            self.interner.intern("__lgx_gk"),
        );
        let fragc = self.interner.intern(frag_owner_binding());
        let fragk = self.interner.intern(frag_key_binding());
        let mut body: Vec<Stmt<'a>> = Vec::new();
        let empty_whos = self.new_seq_expr(int_name);
        body.push(Stmt::Let {
            var: whos,
            ty: None,
            value: empty_whos,
            mutable: true,
        });
        let empty_keys = self.new_seq_expr(text_name);
        body.push(Stmt::Let {
            var: keys,
            ty: None,
            value: empty_keys,
            mutable: true,
        });
        // Phase 1 — take.
        let g = self.interner.intern("__lgx_g");
        let f = self.interner.intern("__lgx_gf");
        let w = self.interner.intern("__lgx_gwho");
        body.push(Stmt::Let {
            var: g,
            ty: None,
            value: self.te_int(1),
            mutable: true,
        });
        let at = self.te_bin(BinaryOpKind::Add, self.te_id(f), self.te_int(1));
        let owner = self.ctx.alloc_imperative_expr(Expr::Index {
            collection: self.te_id(fragc),
            index: at,
        });
        let at = self.te_bin(BinaryOpKind::Add, self.te_id(f), self.te_int(1));
        let key_of = self.ctx.alloc_imperative_expr(Expr::Index {
            collection: self.te_id(fragk),
            index: at,
        });
        let at = self.te_bin(BinaryOpKind::Add, self.te_id(f), self.te_int(1));
        let owned = vec![
            Stmt::Push {
                value: self.te_id(w),
                collection: self.te_id(whos),
            },
            Stmt::Push {
                value: key_of,
                collection: self.te_id(keys),
            },
            Stmt::SetIndex {
                collection: self.te_id(fragc),
                index: at,
                value: self.te_int(0),
            },
        ];
        let is_owned = self.te_bin(BinaryOpKind::Gt, self.te_id(w), self.te_int(0));
        let recorded_len = self.ctx.alloc_imperative_expr(Expr::Length {
            collection: self.te_id(fragc),
        });
        let within = self.te_bin(
            BinaryOpKind::LtEq,
            self.te_bin(BinaryOpKind::Add, self.te_id(f), self.te_int(1)),
            recorded_len,
        );
        let read_owner = vec![
            Stmt::Let {
                var: w,
                ty: None,
                value: owner,
                mutable: false,
            },
            Stmt::If {
                cond: is_owned,
                then_block: self.alloc_block(owned),
                else_block: None,
            },
        ];
        let take = vec![
            Stmt::Let {
                var: f,
                ty: None,
                value: self.vd_expr("vdGone", vec![self.te_id(g)]),
                mutable: false,
            },
            Stmt::If {
                cond: within,
                then_block: self.alloc_block(read_owner),
                else_block: None,
            },
            Stmt::Set {
                target: g,
                value: self.te_bin(BinaryOpKind::Add, self.te_id(g), self.te_int(1)),
            },
        ];
        let count = self.vd_expr("vdGoneCount", Vec::new());
        let more = self.te_bin(BinaryOpKind::LtEq, self.te_id(g), count);
        body.push(Stmt::While {
            cond: more,
            body: self.alloc_block(take),
            decreasing: None,
        });
        body.push(self.vd_call("vdClearGone", Vec::new()));
        let components: Vec<ComponentDef<'a>> = self
            .lgx_all_components
            .clone()
            .into_iter()
            .filter(|d| self.lgx_components.contains(&d.name))
            .collect();
        // Phase 2 — every key list loses its gone keys.
        let x = self.interner.intern("__lgx_gx");
        for def in components.iter().filter(|d| keeps_instance_registry(d)) {
            let name = self.interner.resolve(def.name).to_string();
            let tag = self.component_tag(def.name);
            let list = self.interner.intern(&keys_binding(&name));
            let kept = self.interner.intern("__lgx_kept");
            let k = self.interner.intern("__lgx_k");
            let leaving = self.interner.intern("__lgx_leaving");
            let empty = self.new_seq_expr(text_name);
            let who_at = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(whos),
                index: self.te_id(x),
            });
            let key_at = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(keys),
                index: self.te_id(x),
            });
            let same_who = self.te_bin(BinaryOpKind::Eq, who_at, self.te_int(tag));
            let same_key = self.te_bin(BinaryOpKind::Eq, key_at, self.te_id(k));
            let hit = self.te_bin(BinaryOpKind::And, same_who, same_key);
            let scan = vec![
                Stmt::If {
                    cond: hit,
                    then_block: self.alloc_block(vec![Stmt::Set {
                        target: leaving,
                        value: self.te_bool(true),
                    }]),
                    else_block: None,
                },
                Stmt::Set {
                    target: x,
                    value: self.te_bin(BinaryOpKind::Add, self.te_id(x), self.te_int(1)),
                },
            ];
            let gone_count = self.ctx.alloc_imperative_expr(Expr::Length {
                collection: self.te_id(whos),
            });
            let more = self.te_bin(BinaryOpKind::LtEq, self.te_id(x), gone_count);
            let stays = self.ctx.alloc_imperative_expr(Expr::Not {
                operand: self.te_id(leaving),
            });
            let per_key = vec![
                Stmt::Let {
                    var: leaving,
                    ty: None,
                    value: self.te_bool(false),
                    mutable: true,
                },
                Stmt::Let {
                    var: x,
                    ty: None,
                    value: self.te_int(1),
                    mutable: true,
                },
                Stmt::While {
                    cond: more,
                    body: self.alloc_block(scan),
                    decreasing: None,
                },
                Stmt::If {
                    cond: stays,
                    then_block: self.alloc_block(vec![Stmt::Push {
                        value: self.te_id(k),
                        collection: self.te_id(kept),
                    }]),
                    else_block: None,
                },
            ];
            let shrink = vec![
                Stmt::Let {
                    var: kept,
                    ty: None,
                    value: empty,
                    mutable: true,
                },
                Stmt::Repeat {
                    pattern: crate::ast::stmt::Pattern::Identifier(k),
                    iterable: self.te_id(list),
                    body: self.alloc_block(per_key),
                },
                Stmt::Set {
                    target: list,
                    value: self.te_id(kept),
                },
            ];
            let any_gone = self.ctx.alloc_imperative_expr(Expr::Length {
                collection: self.te_id(whos),
            });
            let some = self.te_bin(BinaryOpKind::Gt, any_gone, self.te_int(0));
            body.push(Stmt::If {
                cond: some,
                then_block: self.alloc_block(shrink),
                else_block: None,
            });
        }
        // Phase 3 — per gone instance: told, handler, retirement.
        let k = self.interner.intern("__lgx_k");
        let mut per_gone: Vec<Stmt<'a>> = Vec::new();
        let who_at = self.ctx.alloc_imperative_expr(Expr::Index {
            collection: self.te_id(whos),
            index: self.te_id(x),
        });
        per_gone.push(Stmt::Let {
            var: w,
            ty: None,
            value: who_at,
            mutable: false,
        });
        let key_at = self.ctx.alloc_imperative_expr(Expr::Index {
            collection: self.te_id(keys),
            index: self.te_id(x),
        });
        per_gone.push(Stmt::Let {
            var: k,
            ty: None,
            value: key_at,
            mutable: false,
        });
        for def in &components {
            let retire = self.retire_instance(def, k);
            if retire.is_empty() {
                continue;
            }
            let tag = self.component_tag(def.name);
            let is_it = self.te_bin(BinaryOpKind::Eq, self.te_id(w), self.te_int(tag));
            per_gone.push(Stmt::If {
                cond: is_it,
                then_block: self.alloc_block(retire),
                else_block: None,
            });
        }
        per_gone.push(Stmt::Set {
            target: x,
            value: self.te_bin(BinaryOpKind::Add, self.te_id(x), self.te_int(1)),
        });
        body.push(Stmt::Let {
            var: x,
            ty: None,
            value: self.te_int(1),
            mutable: true,
        });
        let gone_count = self.ctx.alloc_imperative_expr(Expr::Length {
            collection: self.te_id(whos),
        });
        let more = self.te_bin(BinaryOpKind::LtEq, self.te_id(x), gone_count);
        body.push(Stmt::While {
            cond: more,
            body: self.alloc_block(per_gone),
            decreasing: None,
        });
        // A commit that removed nothing — nearly every one — costs one comparison: the lists above
        // are built only when something left.
        let count = self.vd_expr("vdGoneCount", Vec::new());
        let left = self.te_bin(BinaryOpKind::Gt, count, self.te_int(0));
        let body = vec![Stmt::If {
            cond: left,
            then_block: self.alloc_block(body),
            else_block: None,
        }];
        Stmt::FunctionDef {
            name: self.interner.intern(gone_fn_name()),
            generics: Vec::new(),
            params: Vec::new(),
            body: self.alloc_block(body),
            return_type: None,
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        }
    }

    /// What one gone instance `k` of `def` leaves behind, retired in order: told (`__SEEN_` to
    /// `-1`), its `Disappears` handler run, then every registry entry it kept — its fragment, its
    /// listener lookups and sites, its spent `.once` records, its state, props, callbacks and timers.
    fn retire_instance(&mut self, def: &ComponentDef<'a>, k: Symbol) -> Vec<Stmt<'a>> {
        let name = self.interner.resolve(def.name).to_string();
        let mut out: Vec<Stmt<'a>> = Vec::new();
        let registry = keeps_instance_registry(def);
        let seen = self.interner.intern(&seen_binding(&name));
        if registry {
            out.push(Stmt::SetIndex {
                collection: self.te_id(seen),
                index: self.te_id(k),
                value: self.te_int(-1),
            });
        }
        if let Some(index) = def
            .events
            .iter()
            .position(|h| matches!(h.trigger, Trigger::Disappears))
        {
            let act = self
                .interner
                .intern(&action_fn_name(&name, &format!("__e{index}")));
            out.push(Stmt::Call {
                function: act,
                args: vec![self.te_id(k)],
            });
        }
        if registry {
            out.push(Stmt::Remove {
                value: self.te_id(k),
                collection: self.te_id(seen),
            });
        }
        let mount = self.interner.intern(&mount_binding(&name));
        // The sites its last paint handed out go back with its fragment, which the entry names.
        if super::render::needs_anchors(def) {
            let known = self.ctx.alloc_imperative_expr(Expr::Contains {
                collection: self.te_id(mount),
                value: self.te_id(k),
            });
            let frag = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(mount),
                index: self.te_id(k),
            });
            let release = vec![self.vd_call("vdReleaseAnchors", vec![frag])];
            out.push(Stmt::If {
                cond: known,
                then_block: self.alloc_block(release),
                else_block: None,
            });
        }
        out.push(Stmt::Remove {
            value: self.te_id(k),
            collection: self.te_id(mount),
        });
        if has_any_listener_site(def) {
            let sites = self.interner.intern(&listener_sites_binding(&name));
            let ls = self.interner.intern("__lgx_ls");
            let forget = self.interner.intern(forget_listener_fn_name());
            let retire = vec![Stmt::Call {
                function: forget,
                args: vec![self.te_id(ls)],
            }];
            let held = self.ctx.alloc_imperative_expr(Expr::Contains {
                collection: self.te_id(sites),
                value: self.te_id(k),
            });
            let registered = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(sites),
                index: self.te_id(k),
            });
            let forget = vec![
                Stmt::Repeat {
                    pattern: crate::ast::stmt::Pattern::Identifier(ls),
                    iterable: registered,
                    body: self.alloc_block(retire),
                },
                Stmt::Remove {
                    value: self.te_id(k),
                    collection: self.te_id(sites),
                },
            ];
            out.push(Stmt::If {
                cond: held,
                then_block: self.alloc_block(forget),
                else_block: None,
            });
        }
        if stateful(def) {
            for site_key in self.patchable_site_keys(def) {
                let map = self.interner.intern(&site_binding(&name, &site_key));
                let known = self.ctx.alloc_imperative_expr(Expr::Contains {
                    collection: self.te_id(map),
                    value: self.te_id(k),
                });
                let held = self.ctx.alloc_imperative_expr(Expr::Index {
                    collection: self.te_id(map),
                    index: self.te_id(k),
                });
                let release = vec![
                    self.vd_call("vdFreeSite", vec![held]),
                    Stmt::Remove {
                        value: self.te_id(k),
                        collection: self.te_id(map),
                    },
                ];
                out.push(Stmt::If {
                    cond: known,
                    then_block: self.alloc_block(release),
                    else_block: None,
                });
            }
        }
        // The `.once` listeners this instance spent are forgotten with it, so a remount under the
        // same key starts with every listener armed.
        if self.once_recorded(def.name) {
            let record = self.interner.intern(&once_record_binding(&name));
            let spent = self.interner.intern(once_spent_binding());
            let id = self.interner.intern("__lgx_oi");
            let known = self.ctx.alloc_imperative_expr(Expr::Contains {
                collection: self.te_id(record),
                value: self.te_id(k),
            });
            let held = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(record),
                index: self.te_id(k),
            });
            let forget = Stmt::Repeat {
                pattern: crate::ast::stmt::Pattern::Identifier(id),
                iterable: held,
                body: self.alloc_block(vec![Stmt::Remove {
                    value: self.te_id(id),
                    collection: self.te_id(spent),
                }]),
            };
            out.push(Stmt::If {
                cond: known,
                then_block: self.alloc_block(vec![forget]),
                else_block: None,
            });
            out.push(Stmt::Remove {
                value: self.te_id(k),
                collection: self.te_id(record),
            });
        }
        if registry && stateful(def) {
            let mut maps: Vec<Symbol> = def
                .state
                .iter()
                .filter(|f| !f.derived)
                .map(|f| {
                    self.interner
                        .intern(&state_binding(&name, self.interner.resolve(f.name)))
                })
                .collect();
            for (p, t) in &def.props {
                if !is_callback_type(t, self.interner) {
                    maps.push(
                        self.interner
                            .intern(&prop_binding(&name, self.interner.resolve(*p))),
                    );
                }
            }
            maps.extend(self.callback_prop_identity_map_names(def));
            let timers: Vec<String> = self
                .lgx_timers
                .clone()
                .iter()
                .filter(|(c, ..)| *c == def.name)
                .flat_map(|(_, i, ..)| {
                    [
                        due_binding(&name, *i),
                        seq_binding(&name, *i),
                        period_binding(&name, *i),
                    ]
                })
                .collect();
            maps.extend(timers.iter().map(|m| self.interner.intern(m)));
            for m in maps {
                out.push(Stmt::Remove {
                    value: self.te_id(k),
                    collection: self.te_id(m),
                });
            }
        }
        out
    }

    /// `__lgx_commit(node)`: commit the tape into `node`'s retained children and hand the stream to
    /// the host — or fail the turn with the commit's own refusal, the document untouched.
    pub(crate) fn lower_lgx_commit(&mut self) -> Stmt<'a> {
        let node = self.interner.intern("__lgx_into");
        let problem = self.interner.intern("__lgx_problem");
        let committed = self.vd_expr("vdCommit", vec![self.te_id(node)]);
        let blank = self.interner.intern("");
        let refused = self.te_bin(
            BinaryOpKind::NotEq,
            self.te_id(problem),
            self.te_text(blank),
        );
        let body = vec![
            Stmt::Let {
                var: problem,
                ty: None,
                value: committed,
                mutable: false,
            },
            Stmt::If {
                cond: refused,
                then_block: self.alloc_block(vec![Stmt::Fail {
                    reason: self.te_id(problem),
                }]),
                else_block: None,
            },
            self.vd_call("vdApply", Vec::new()),
        ];
        let int_ty = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Int")));
        Stmt::FunctionDef {
            name: self.interner.intern(commit_fn_name()),
            generics: Vec::new(),
            params: vec![(node, int_ty)],
            body: self.alloc_block(body),
            return_type: None,
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        }
    }

    /// `__lgx_own_frag(frag, who, key)`: records the instance a fresh fragment is drawn for, growing
    /// the registry as far as `frag` needs.
    pub(crate) fn lower_lgx_own_frag(&mut self) -> Stmt<'a> {
        let (frag, who, key) = (
            self.interner.intern("__lgx_of"),
            self.interner.intern("__lgx_ow"),
            self.interner.intern("__lgx_ok"),
        );
        let fragc = self.interner.intern(frag_owner_binding());
        let fragk = self.interner.intern(frag_key_binding());
        let blank = self.interner.intern("");
        let short = self.te_bin(
            BinaryOpKind::LtEq,
            self.ctx.alloc_imperative_expr(Expr::Length {
                collection: self.te_id(fragc),
            }),
            self.te_id(frag),
        );
        let grow = vec![
            Stmt::Push {
                value: self.te_int(0),
                collection: self.te_id(fragc),
            },
            Stmt::Push {
                value: self.te_text(blank),
                collection: self.te_id(fragk),
            },
        ];
        let at = self.te_bin(BinaryOpKind::Add, self.te_id(frag), self.te_int(1));
        let at2 = self.te_bin(BinaryOpKind::Add, self.te_id(frag), self.te_int(1));
        let body = vec![
            Stmt::While {
                cond: short,
                body: self.alloc_block(grow),
                decreasing: None,
            },
            Stmt::SetIndex {
                collection: self.te_id(fragc),
                index: at,
                value: self.te_id(who),
            },
            Stmt::SetIndex {
                collection: self.te_id(fragk),
                index: at2,
                value: self.te_id(key),
            },
        ];
        let int_ty = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Int")));
        let text_ty = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Text")));
        Stmt::FunctionDef {
            name: self.interner.intern(own_frag_fn_name()),
            generics: Vec::new(),
            params: vec![(frag, int_ty), (who, int_ty), (key, text_ty)],
            body: self.alloc_block(body),
            return_type: None,
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        }
    }

    /// The fragment registry `__lgx_own_frag` fills, declared once — seeded for fragment `0`, which
    /// no instance holds.
    pub(crate) fn declare_frag_registry(&mut self) {
        let fragc = self.interner.intern(frag_owner_binding());
        let value = self
            .ctx
            .alloc_imperative_expr(Expr::List(vec![self.te_int(0)]));
        self.deferred_functions.push(Stmt::Let {
            var: fragc,
            ty: None,
            value,
            mutable: true,
        });
        let fragk = self.interner.intern(frag_key_binding());
        let blank = self.interner.intern("");
        let value = self
            .ctx
            .alloc_imperative_expr(Expr::List(vec![self.te_text(blank)]));
        self.deferred_functions.push(Stmt::Let {
            var: fragk,
            ty: None,
            value,
            mutable: true,
        });
    }

    /// The components a repaint of `scope` can draw — every one its view places (a layout it
    /// `{extends}` included), to any depth.
    pub(crate) fn swept_under(&self, scope: &ComponentDef<'a>) -> Vec<Symbol> {
        let all = &self.lgx_all_components;
        let mut out: Vec<Symbol> = Vec::new();
        let mut todo: Vec<Symbol> = placed_components(drawn_view(scope));
        while let Some(name) = todo.pop() {
            if out.contains(&name) {
                continue;
            }
            out.push(name);
            if let Some(def) = all.iter().find(|d| d.name == name) {
                todo.extend(placed_components(drawn_view(def)));
            }
        }
        out
    }

    /// Whether a repaint of `scope` holds only part of the page: `scope` is placed somewhere, so its
    /// repaint draws the subtree under its own key rather than the page from its root.
    pub(crate) fn sweep_is_scoped(&self, scope: &ComponentDef<'a>) -> bool {
        self.lgx_placed.contains(&scope.name)
    }

    /// Whether any repaint the program can make is scoped and reaches a component that is swept —
    /// the one question the descent test's emission asks.
    pub(crate) fn uses_scoped_sweep(&self) -> bool {
        let can_leave = unmountable_components(&self.lgx_all_components);
        let tracks_directives = !self.removable_directives().is_empty();
        self.lgx_all_components.iter().any(|d| {
            self.sweep_is_scoped(d)
                && self.repaint_target(d).is_none()
                && (tracks_directives
                    || self.swept_under(d).iter().any(|c| {
                        self.lgx_sweeps.iter().any(|(s, _)| s == c)
                            || (can_leave.contains(c)
                                && self
                                    .lgx_all_components
                                    .iter()
                                    .any(|x| x.name == *c && keeps_instance_registry(x)))
                    }))
        })
    }

    /// `__lgx_descends(k, under) -> Bool` — whether instance key `k` begins with `under` (a key and
    /// its trailing `/`): the instance was placed, at some depth, inside the one keyed `under`.
    /// Compared byte by byte, so no key's content can be mistaken for the boundary.
    pub(crate) fn lower_lgx_descends(&mut self) -> Stmt<'a> {
        let k = self.interner.intern("__lgx_dk");
        let under = self.interner.intern("__lgx_du");
        let kb = self.interner.intern("__lgx_dkb");
        let ub = self.interner.intern("__lgx_dub");
        let i = self.interner.intern("__lgx_di");
        let bytes = self.interner.intern("text_bytes");
        let len = |me: &mut Self, v: Symbol| {
            me.ctx.alloc_imperative_expr(Expr::Length {
                collection: me.te_id(v),
            })
        };
        let mut body: Vec<Stmt<'a>> = vec![
            Stmt::Let {
                var: kb,
                ty: None,
                value: self.te_call(bytes, vec![self.te_id(k)]),
                mutable: false,
            },
            Stmt::Let {
                var: ub,
                ty: None,
                value: self.te_call(bytes, vec![self.te_id(under)]),
                mutable: false,
            },
        ];
        let too_long = {
            let (lu, lk) = (len(self, ub), len(self, kb));
            self.te_bin(BinaryOpKind::Gt, lu, lk)
        };
        body.push(Stmt::If {
            cond: too_long,
            then_block: self.alloc_block(vec![Stmt::Return {
                value: Some(self.te_bool(false)),
            }]),
            else_block: None,
        });
        body.push(Stmt::Let {
            var: i,
            ty: None,
            value: self.te_int(1),
            mutable: true,
        });
        let at_k = self.ctx.alloc_imperative_expr(Expr::Index {
            collection: self.te_id(kb),
            index: self.te_id(i),
        });
        let at_u = self.ctx.alloc_imperative_expr(Expr::Index {
            collection: self.te_id(ub),
            index: self.te_id(i),
        });
        let differs = self.te_bin(BinaryOpKind::NotEq, at_k, at_u);
        let step = self.te_bin(BinaryOpKind::Add, self.te_id(i), self.te_int(1));
        let each = self.alloc_block(vec![
            Stmt::If {
                cond: differs,
                then_block: self.alloc_block(vec![Stmt::Return {
                    value: Some(self.te_bool(false)),
                }]),
                else_block: None,
            },
            Stmt::Set {
                target: i,
                value: step,
            },
        ]);
        let lu = len(self, ub);
        let more = self.te_bin(BinaryOpKind::LtEq, self.te_id(i), lu);
        body.push(Stmt::While {
            cond: more,
            body: each,
            decreasing: None,
        });
        body.push(Stmt::Return {
            value: Some(self.te_bool(true)),
        });
        let text_ty = self.text_type();
        let bool_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Bool")));
        Stmt::FunctionDef {
            name: self.interner.intern(descends_fn_name()),
            generics: Vec::new(),
            params: vec![(k, text_ty), (under, text_ty)],
            body: self.alloc_block(body),
            return_type: Some(bool_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        }
    }

    /// ★ A WRAPPER OR A LAYOUT REPAINTS THROUGH THE PAGE, NOT ITSELF.
    ///
    /// Every other instance repaints alone — its fragment's contents drawn again and committed
    /// into that fragment, wherever it stands. A wrapper and a layout cannot: part of what they
    /// draw is a range their CALLER drew, in the caller's scope, and no caller runs when their own
    /// turn repaints. So their repaint goes through the page's root, which draws the caller, which
    /// draws them — the per-instance repaint is the optimization, and these two shapes are the ones
    /// that cannot take it.
    pub(crate) fn repaint_target(&self, def: &ComponentDef<'a>) -> Option<Symbol> {
        // ★ A WRAPPER NOBODY PLACES IS ALREADY A ROOT — it is mounted, so nothing hands it children.
        // Redirecting it would send the repaint to a DIFFERENT unplaced component, and two wrappers
        // redirecting to each other would never finish compiling.
        let draws_a_callers_range = (self.lgx_wrappers.contains(&def.name)
            && self.lgx_placed.contains(&def.name))
            || super::render::is_extended_layout(def, &self.lgx_all_components);
        if !draws_a_callers_range {
            return None;
        }
        self.lgx_paint_root.filter(|root| *root != def.name)
    }

    /// One paint of `def`'s instance: its render written to the tape and committed.
    ///
    /// `first`: `true` only from the component's own mount — the instance is drawn as a new
    /// fragment, after every reachable stylesheet, and committed into the mount's root (`__lgx_root`,
    /// bound by the mount). `false` from every other caller (a turn, a bound input's write-back, a
    /// store-moved repaint): the instance's contents are drawn again and committed into its own
    /// fragment, which the commit leaves standing wherever it is.
    pub(crate) fn repaint_def(&mut self, def: &ComponentDef<'a>, first: bool) -> Vec<Stmt<'a>> {
        if let Some(root) = self.repaint_target(def) {
            if let Some(rd) = self
                .lgx_all_components
                .iter()
                .find(|d| d.name == root)
                .cloned()
            {
                // The page is the ROOT instance, whose key is `""` — not whichever instance's turn
                // happens to be running.
                let outer = self.lgx_key_override.replace(self.interner.intern(""));
                let out = self.repaint_def(&rd, false);
                self.lgx_key_override = outer;
                return out;
            }
        }
        let component = self.interner.resolve(def.name).to_string();
        let render = self.interner.intern(&render_fn_name(&component));
        let mut out: Vec<Stmt<'a>> = Vec::new();
        for styled in self.styled_components.clone() {
            let flag = self
                .interner
                .intern(&styled_binding(self.interner.resolve(styled)));
            out.push(Stmt::Set {
                target: flag,
                value: self.te_bool(false),
            });
        }
        let key = self.key_expr();
        let mut args = vec![key];
        for (prop, ty) in &def.props {
            if !is_callback_type(ty, self.interner) {
                args.push(self.prop_read(&component, *prop));
            }
        }
        // A repaint has no caller running, so there are no children to hand down.
        args.push(self.te_int(0));
        args.extend(self.block_slot_zeros(def));
        out.extend(self.open_pass());
        // A paint starts from an empty tape, whatever a failed paint before it left.
        out.push(self.vd_call("vdAbort", Vec::new()));
        let commit = self.interner.intern(commit_fn_name());
        if first {
            // ★ EVERY STYLESHEET GOES IN AT THE MOUNT POINT, BEFORE THE MARKUP — matching the
            // server, which emits them all from the root; a child's repaint commits into its own
            // fragment and leaves them where they are.
            if self.lgx_paint_root == Some(def.name) || self.lgx_paint_root.is_none() {
                let sheets = self.styled_reachable(def.name, &self.lgx_all_components.clone());
                for styled in sheets {
                    let owner = self.interner.resolve(styled).to_string();
                    let Some(sheet) = self
                        .lgx_all_components
                        .iter()
                        .find(|d| d.name == styled)
                        .and_then(|d| d.style.clone())
                    else {
                        continue;
                    };
                    let class = scope_class(&owner);
                    let css = self.interner.resolve(sheet.css).to_string();
                    let Ok(scoped) = crate::lgx::css::scope(&css, &class) else {
                        continue;
                    };
                    let tag = self.interner.intern("style");
                    let css_sym = self.interner.intern(&scoped);
                    out.push(self.vd_call("vdOpen", vec![self.te_text(tag)]));
                    out.push(self.vd_call("vdText", vec![self.te_text(css_sym)]));
                    out.push(self.vd_call("vdClose", Vec::new()));
                }
            }
            let mut next = 0usize;
            let key = self.key_expr();
            let frag = self.instance_frag(def.name, key, &mut next, &mut out);
            args.insert(1, self.te_id(frag));
            out.push(self.vd_call("vdFragOpen", vec![self.te_id(frag)]));
            out.push(Stmt::Call {
                function: render,
                args,
            });
            out.push(self.vd_call("vdFragClose", Vec::new()));
            let root = self.interner.intern("__lgx_root");
            out.push(Stmt::Call {
                function: commit,
                args: vec![self.te_id(root)],
            });
        } else {
            let mount = self.interner.intern(&mount_binding(&component));
            let key = self.key_expr();
            let held = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(mount),
                index: key,
            });
            let frag = self.interner.intern("__lgx_repainted");
            out.push(Stmt::Let {
                var: frag,
                ty: None,
                value: held,
                mutable: false,
            });
            args.insert(1, self.te_id(frag));
            out.push(Stmt::Call {
                function: render,
                args,
            });
            let node = self.vd_expr("vdFragNode", vec![self.te_id(frag)]);
            out.push(Stmt::Call {
                function: commit,
                args: vec![node],
            });
        }
        // ★ WHAT THE COMMIT PUT IN THE DOCUMENT IS THERE NOW: the directives drawn on it run against
        // their elements, the instances it removed are swept, and the removed directive
        // occurrences are told.
        if !self.drawn_directives().is_empty() {
            let drain = self.interner.intern(drain_directives_fn_name());
            out.push(Stmt::Call {
                function: drain,
                args: Vec::new(),
            });
        }
        if self.instances_can_leave() {
            let gone = self.interner.intern(gone_fn_name());
            out.push(Stmt::Call {
                function: gone,
                args: Vec::new(),
            });
        }
        let removed = self.sweep_directives(def);
        out.extend(removed);
        out.extend(self.appear_call());
        // The element events a placed instance's `When the page loads:` asked for, delivered now
        // that the paint that loaded it is drawn — the flush the root's mount performs for its own.
        if self.placed_loads_queue_events() {
            let dispatch = self.interner.intern(dispatch_fn_name());
            let flush = self.interner.intern(outbox_flush_event());
            let empty = self.interner.intern("");
            out.push(Stmt::Call {
                function: dispatch,
                args: vec![
                    self.te_text(empty),
                    self.te_text(flush),
                    self.te_text(empty),
                ],
            });
        }
        out
    }

    /// Whether some component that another view places has a `When the page loads:` that queues an
    /// element event — which that load, run inside a paint, leaves in the outbox for the paint's end.
    fn placed_loads_queue_events(&self) -> bool {
        let placed: Vec<Symbol> = self
            .lgx_all_components
            .iter()
            .filter_map(|d| d.view.as_ref())
            .flat_map(|v| placed_components(&v.nodes))
            .collect();
        self.lgx_all_components
            .iter()
            .filter(|d| placed.contains(&d.name))
            .any(|d| {
                d.events.iter().any(|h| {
                    matches!(h.trigger, Trigger::PageLoads) && body_queues_element_event(h.body)
                })
            })
    }

    /// Queues one directive occurrence for the paint's commit: the element's site, the value it is
    /// drawn with (when the directive takes one) and — for a directive with a `When removed:`
    /// block — the occurrence's identity (instance key, static site, row), which the removal sweep
    /// tells occurrences apart by. `drain_directives` runs them once the element is in the document.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn queue_directive(
        &mut self,
        name: Symbol,
        site: Symbol,
        value: &'a Expr<'a>,
        _owner: &ComponentDef<'a>,
        trail: &LowerTrail,
        next: &mut usize,
        out: &mut Vec<Stmt<'a>>,
    ) {
        let directive = self.interner.resolve(name).to_string();
        let takes_value = self.directive_arity(name).is_some_and(|n| n > 1);
        let removable = self
            .removable_directives()
            .iter()
            .any(|(d, _)| *d == directive);
        let sites = self
            .interner
            .intern(&directive_queue_site_binding(&directive));
        out.push(Stmt::Push {
            value: self.te_id(site),
            collection: self.te_id(sites),
        });
        if takes_value {
            let values = self
                .interner
                .intern(&directive_queue_value_binding(&directive));
            out.push(Stmt::Push {
                value,
                collection: self.te_id(values),
            });
        }
        if removable {
            let occurrence = self.lgx_temp(next);
            let tag = format!("\u{1}{}", self.interner.resolve(occurrence));
            let tag = self.interner.intern(&tag);
            let own = self.key_expr();
            let base = self.te_bin(BinaryOpKind::Add, own, self.te_text(tag));
            let identity = self.with_row(base, trail);
            let keys = self
                .interner
                .intern(&directive_queue_key_binding(&directive));
            out.push(Stmt::Push {
                value: identity,
                collection: self.te_id(keys),
            });
        }
    }

    /// Every directive some lowered view draws, in declaration order, with the type of the value it
    /// takes when it takes one — the queues `queue_directive` fills and `drain_directives` empties.
    pub(crate) fn drawn_directives(&self) -> Vec<(Symbol, Option<&'a TypeExpr<'a>>)> {
        fn walk(nodes: &[ViewNode<'_>], found: &mut Vec<Symbol>) {
            for node in nodes {
                match node {
                    ViewNode::Element {
                        attrs, children, ..
                    } => {
                        for a in attrs {
                            if let Attr::Directive { name, .. } = a {
                                if !found.contains(name) {
                                    found.push(*name);
                                }
                            }
                        }
                        walk(children, found);
                    }
                    ViewNode::Each { body, empty, .. } => {
                        walk(body, found);
                        walk(empty, found);
                    }
                    ViewNode::If {
                        arms, otherwise, ..
                    } => {
                        for (_, b) in arms {
                            walk(b, found);
                        }
                        if let Some(o) = otherwise {
                            walk(o, found);
                        }
                    }
                    ViewNode::Inspect { arms, .. } => {
                        for arm in arms {
                            walk(&arm.body, found);
                        }
                    }
                    ViewNode::Component { children, .. } => walk(children, found),
                    ViewNode::Block {
                        default: Some(b), ..
                    } => walk(b, found),
                    _ => {}
                }
            }
        }
        let mut found = Vec::new();
        for def in &self.lgx_all_components {
            if let Some(view) = &def.view {
                walk(&view.nodes, &mut found);
            }
        }
        found
            .into_iter()
            .map(|name| {
                let want = directive_fn_name(self.interner.resolve(name));
                let ty = self.deferred_functions.iter().find_map(|s| match s {
                    Stmt::FunctionDef {
                        name: f, params, ..
                    } if self.interner.resolve(*f) == want => Some(params.get(1).map(|(_, t)| *t)),
                    _ => None,
                });
                (name, ty.flatten())
            })
            .collect()
    }

    /// The queues every drawn directive's occurrences wait in between a render and its commit.
    pub(crate) fn declare_directive_queues(&mut self) {
        let int_name = self.interner.intern("Int");
        let text_name = self.interner.intern("Text");
        for (name, value_ty) in self.drawn_directives() {
            let directive = self.interner.resolve(name).to_string();
            let sites = self
                .interner
                .intern(&directive_queue_site_binding(&directive));
            let value = self.new_seq_expr(int_name);
            self.deferred_functions.push(Stmt::Let {
                var: sites,
                ty: None,
                value,
                mutable: true,
            });
            if let Some(ty) = value_ty {
                let values = self
                    .interner
                    .intern(&directive_queue_value_binding(&directive));
                let seq = self.seq_of_type(ty);
                self.deferred_functions.push(Stmt::Let {
                    var: values,
                    ty: None,
                    value: seq,
                    mutable: true,
                });
            }
            if self
                .removable_directives()
                .iter()
                .any(|(d, _)| *d == directive)
            {
                let keys = self
                    .interner
                    .intern(&directive_queue_key_binding(&directive));
                let value = self.new_seq_expr(text_name);
                self.deferred_functions.push(Stmt::Let {
                    var: keys,
                    ty: None,
                    value,
                    mutable: true,
                });
            }
        }
    }

    /// `a new Seq of <ty>`, for a queue of values of a directive's declared type.
    fn seq_of_type(&mut self, ty: &'a TypeExpr<'a>) -> &'a Expr<'a> {
        let seq_name = self.interner.intern("Seq");
        self.ctx.alloc_imperative_expr(Expr::New {
            type_name: seq_name,
            type_args: vec![ty.clone()],
            init_fields: Vec::new(),
        })
    }

    /// Runs every directive occurrence the committed paint drew, against its element as it now
    /// stands in the document, and records a removable one's occurrence for the removal sweep
    /// (`sweep_directives`). The queues are taken first, so a directive that paints again queues
    /// into fresh ones.
    fn drain_directives(&mut self) -> Vec<Stmt<'a>> {
        let mut out = Vec::new();
        let int_name = self.interner.intern("Int");
        let text_name = self.interner.intern("Text");
        let removable = self.removable_directives();
        for (name, value_ty) in self.drawn_directives() {
            let directive = self.interner.resolve(name).to_string();
            let takes_value = self.directive_arity(name).is_some_and(|n| n > 1);
            let is_removable = removable.iter().any(|(d, _)| *d == directive);
            let sites = self
                .interner
                .intern(&directive_queue_site_binding(&directive));
            let values = self
                .interner
                .intern(&directive_queue_value_binding(&directive));
            let keys = self
                .interner
                .intern(&directive_queue_key_binding(&directive));
            let (taken_sites, taken_values, taken_keys) = (
                self.interner.intern(&format!("__lgx_dqs_{directive}")),
                self.interner.intern(&format!("__lgx_dqv_{directive}")),
                self.interner.intern(&format!("__lgx_dqk_{directive}")),
            );
            out.push(Stmt::Let {
                var: taken_sites,
                ty: None,
                value: self.te_id(sites),
                mutable: false,
            });
            let fresh = self.new_seq_expr(int_name);
            out.push(Stmt::Set {
                target: sites,
                value: fresh,
            });
            if takes_value {
                if let Some(ty) = value_ty {
                    out.push(Stmt::Let {
                        var: taken_values,
                        ty: None,
                        value: self.te_id(values),
                        mutable: false,
                    });
                    let fresh = self.seq_of_type(ty);
                    out.push(Stmt::Set {
                        target: values,
                        value: fresh,
                    });
                }
            }
            if is_removable {
                out.push(Stmt::Let {
                    var: taken_keys,
                    ty: None,
                    value: self.te_id(keys),
                    mutable: false,
                });
                let fresh = self.new_seq_expr(text_name);
                out.push(Stmt::Set {
                    target: keys,
                    value: fresh,
                });
            }
            let at = self.interner.intern(&format!("__lgx_dqi_{directive}"));
            let el = self.interner.intern(&format!("__lgx_dqe_{directive}"));
            let site_at = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(taken_sites),
                index: self.te_id(at),
            });
            let node = self.vd_expr("vdSiteNode", vec![site_at]);
            let wire = self.vd_expr("vdWire", vec![node]);
            let mut each = vec![Stmt::Let {
                var: el,
                ty: None,
                value: wire,
                mutable: false,
            }];
            let fname = self.interner.intern(&directive_fn_name(&directive));
            let mut args = vec![self.te_id(el)];
            let value_at = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(taken_values),
                index: self.te_id(at),
            });
            if takes_value {
                args.push(value_at);
            }
            each.push(Stmt::Call {
                function: fname,
                args,
            });
            if is_removable {
                let occurrence = self.ctx.alloc_imperative_expr(Expr::Index {
                    collection: self.te_id(taken_keys),
                    index: self.te_id(at),
                });
                let k = self.interner.intern(&format!("__lgx_dqo_{directive}"));
                each.push(Stmt::Let {
                    var: k,
                    ty: None,
                    value: occurrence,
                    mutable: false,
                });
                let known_keys = self.interner.intern(&directive_keys_binding(&directive));
                let stamps = self.interner.intern(&directive_stamp_binding(&directive));
                let nodes = self.interner.intern(&directive_node_binding(&directive));
                let known = self.ctx.alloc_imperative_expr(Expr::Contains {
                    collection: self.te_id(stamps),
                    value: self.te_id(k),
                });
                let unknown = self.ctx.alloc_imperative_expr(Expr::Not { operand: known });
                let push = Stmt::Push {
                    value: self.te_id(k),
                    collection: self.te_id(known_keys),
                };
                each.push(Stmt::If {
                    cond: unknown,
                    then_block: self.alloc_block(vec![push]),
                    else_block: None,
                });
                let pass = self.interner.intern(pass_binding());
                each.push(Stmt::SetIndex {
                    collection: self.te_id(stamps),
                    index: self.te_id(k),
                    value: self.te_id(pass),
                });
                each.push(Stmt::SetIndex {
                    collection: self.te_id(nodes),
                    index: self.te_id(k),
                    value: self.te_id(el),
                });
                if takes_value {
                    let recorded = self.interner.intern(&directive_value_binding(&directive));
                    let value_again = self.ctx.alloc_imperative_expr(Expr::Index {
                        collection: self.te_id(taken_values),
                        index: self.te_id(at),
                    });
                    each.push(Stmt::SetIndex {
                        collection: self.te_id(recorded),
                        index: self.te_id(k),
                        value: value_again,
                    });
                }
            }
            each.push(Stmt::Set {
                target: at,
                value: self.te_bin(BinaryOpKind::Add, self.te_id(at), self.te_int(1)),
            });
            out.push(Stmt::Let {
                var: at,
                ty: None,
                value: self.te_int(1),
                mutable: true,
            });
            let count = self.ctx.alloc_imperative_expr(Expr::Length {
                collection: self.te_id(taken_sites),
            });
            let more = self.te_bin(BinaryOpKind::LtEq, self.te_id(at), count);
            out.push(Stmt::While {
                cond: more,
                body: self.alloc_block(each),
                decreasing: None,
            });
        }
        out
    }

    /// `__lgx_drain_directives()` — the drain as one function every paint calls after its commit.
    pub(crate) fn lower_lgx_drain_directives(&mut self) -> Stmt<'a> {
        let body = self.drain_directives();
        Stmt::FunctionDef {
            name: self.interner.intern(drain_directives_fn_name()),
            generics: Vec::new(),
            params: Vec::new(),
            body: self.alloc_block(body),
            return_type: None,
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        }
    }

    /// Every directive with a `When removed:` block, as `(name, its value parameter's type when it
    /// takes one)` — read off the `__dir_<name>__removed` functions this lowering emitted, in
    /// declaration order, so the attach sites, the registries and the sweep read one list.
    pub(crate) fn removable_directives(&self) -> Vec<(String, Option<&'a TypeExpr<'a>>)> {
        self.deferred_functions
            .iter()
            .filter_map(|s| match s {
                Stmt::FunctionDef { name, params, .. } => {
                    let n = self.interner.resolve(*name);
                    let directive = n.strip_prefix("__dir_")?.strip_suffix("__removed")?;
                    Some((directive.to_string(), params.get(1).map(|(_, t)| *t)))
                }
                _ => None,
            })
            .collect()
    }

    /// How many parameters `directive`'s own `__dir_<name>` function takes — the element, and its
    /// value when it declares one — so every call is shaped by the function it calls.
    pub(crate) fn directive_arity(&self, directive: Symbol) -> Option<usize> {
        let want = directive_fn_name(self.interner.resolve(directive));
        self.deferred_functions.iter().find_map(|s| match s {
            Stmt::FunctionDef { name, params, .. } if self.interner.resolve(*name) == want => {
                Some(params.len())
            }
            _ => None,
        })
    }

    /// ★ THE DIRECTIVE SWEEP — `When removed:` for every attached occurrence the repaint of `scope`
    /// no longer drew: within the repainted instance (its own elements, keyed `<key>\u{1}…`) and
    /// every instance under it (`<key>/…`); the page root's repaint, every occurrence. Run after
    /// the instance sweep, with the element and value each occurrence
    /// was last drawn with. The list shrinks before any removed block runs, so one that repaints
    /// cannot rediscover an occurrence already taken.
    pub(crate) fn sweep_directives(&mut self, scope: &ComponentDef<'a>) -> Vec<Stmt<'a>> {
        let mut out: Vec<Stmt<'a>> = Vec::new();
        let directives = self.removable_directives();
        if directives.is_empty() {
            return out;
        }
        let scoped = self.sweep_is_scoped(scope);
        let (under, own) = (
            self.interner.intern("__lgx_dunder"),
            self.interner.intern("__lgx_downer"),
        );
        if scoped {
            for (var, sep) in [(under, "/"), (own, "\u{1}")] {
                let sep = self.interner.intern(sep);
                let key = self.key_expr();
                let prefix = self.te_bin(BinaryOpKind::Add, key, self.te_text(sep));
                out.push(Stmt::Let {
                    var,
                    ty: None,
                    value: prefix,
                    mutable: false,
                });
            }
        }
        let pass = self.interner.intern(pass_binding());
        let text_name = self.interner.intern("Text");
        let (k, v) = (
            self.interner.intern("__lgx_dk"),
            self.interner.intern("__lgx_dv"),
        );
        let (kept, gone) = (
            self.interner.intern("__lgx_dkept"),
            self.interner.intern("__lgx_dgone"),
        );
        for (directive, value_ty) in directives {
            let keys = self.interner.intern(&directive_keys_binding(&directive));
            let stamps = self.interner.intern(&directive_stamp_binding(&directive));
            let nodes = self.interner.intern(&directive_node_binding(&directive));
            let values = self.interner.intern(&directive_value_binding(&directive));
            let empty = self.new_seq_expr(text_name);
            out.push(Stmt::Let {
                var: kept,
                ty: None,
                value: empty,
                mutable: true,
            });
            let empty = self.new_seq_expr(text_name);
            out.push(Stmt::Let {
                var: gone,
                ty: None,
                value: empty,
                mutable: true,
            });
            let stamp = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(stamps),
                index: self.te_id(k),
            });
            let read = Stmt::Let {
                var: v,
                ty: None,
                value: stamp,
                mutable: false,
            };
            let drawn = self.te_bin(BinaryOpKind::Eq, self.te_id(v), self.te_id(pass));
            let is_seed = self.te_bin(BinaryOpKind::Eq, self.te_id(v), self.te_int(-1));
            let mut live = self.te_bin(BinaryOpKind::Or, drawn, is_seed);
            if scoped {
                let descends = self.interner.intern(descends_fn_name());
                let below = self.te_call(descends, vec![self.te_id(k), self.te_id(under)]);
                let mine = self.te_call(descends, vec![self.te_id(k), self.te_id(own)]);
                let inside = self.te_bin(BinaryOpKind::Or, below, mine);
                let outside = self
                    .ctx
                    .alloc_imperative_expr(Expr::Not { operand: inside });
                live = self.te_bin(BinaryOpKind::Or, live, outside);
            }
            let sort = Stmt::If {
                cond: live,
                then_block: self.alloc_block(vec![Stmt::Push {
                    value: self.te_id(k),
                    collection: self.te_id(kept),
                }]),
                else_block: Some(self.alloc_block(vec![Stmt::Push {
                    value: self.te_id(k),
                    collection: self.te_id(gone),
                }])),
            };
            out.push(Stmt::Repeat {
                pattern: crate::ast::stmt::Pattern::Identifier(k),
                iterable: self.te_id(keys),
                body: self.alloc_block(vec![read, sort]),
            });
            out.push(Stmt::Set {
                target: keys,
                value: self.te_id(kept),
            });
            let node = self.ctx.alloc_imperative_expr(Expr::Index {
                collection: self.te_id(nodes),
                index: self.te_id(k),
            });
            let mut args = vec![node];
            if value_ty.is_some() {
                args.push(self.ctx.alloc_imperative_expr(Expr::Index {
                    collection: self.te_id(values),
                    index: self.te_id(k),
                }));
            }
            let run = self.interner.intern(&directive_removed_fn_name(&directive));
            let mut per_gone = vec![Stmt::Call {
                function: run,
                args,
            }];
            let mut columns = vec![stamps, nodes];
            if value_ty.is_some() {
                columns.push(values);
            }
            for m in columns {
                per_gone.push(Stmt::Remove {
                    value: self.te_id(k),
                    collection: self.te_id(m),
                });
            }
            out.push(Stmt::Repeat {
                pattern: crate::ast::stmt::Pattern::Identifier(k),
                iterable: self.te_id(gone),
                body: self.alloc_block(per_gone),
            });
        }
        out
    }
}
