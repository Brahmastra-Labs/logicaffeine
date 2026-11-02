use super::*;
use crate::ast::component::{ComponentDef, ComponentKind};
use crate::ast::stmt::{BinaryOpKind, Expr, Stmt};
use logicaffeine_base::Symbol;

impl<'a, 'ctx, 'int> super::super::Parser<'a, 'ctx, 'int> {
    /// One named DOM operation on the element `selector` names, followed by the event a browser
    /// would have raised for it — a `Splice`, because it is two statements that are one sentence.
    /// The DOM command with no event after it — for the operations a browser raises nothing for.
    pub(crate) fn lower_element_command_only(
        &mut self,
        selector: &'a Expr<'a>,
        command: &'static str,
    ) -> Stmt<'a> {
        let query = self.interner.intern("querySelector");
        let cmd_fn = self.interner.intern("elementCommand");
        let cmd = self.interner.intern(command);
        let el = self.te_call(query, vec![selector]);
        Stmt::Call {
            function: cmd_fn,
            args: vec![el, self.te_text(cmd)],
        }
    }

    pub(crate) fn lower_element_command(
        &mut self,
        selector: &'a Expr<'a>,
        command: &'static str,
        event: &'static str,
    ) -> Stmt<'a> {
        let query = self.interner.intern("querySelector");
        let cmd_fn = self.interner.intern("elementCommand");
        let cmd = self.interner.intern(command);
        let el = self.te_call(query, vec![selector]);
        let run = Stmt::Call {
            function: cmd_fn,
            args: vec![el, self.te_text(cmd)],
        };
        let ev = self.interner.intern(event);
        let empty = self.interner.intern("");
        let (ev, empty) = (self.te_text(ev), self.te_text(empty));
        let queued = self.queue_element_event(selector, ev, empty);
        Stmt::Splice {
            body: self.alloc_block(vec![run, queued]),
        }
    }

    /// Have the turn being lowered flush the outbox after its write-back — once, however many
    /// statements in it queue.
    pub(crate) fn request_outbox_flush(&mut self) {
        if !self.lgx_pending_events.is_empty() {
            return;
        }
        let flush = self.interner.intern(outbox_flush_event());
        let empty = self.interner.intern("");
        let request = (
            self.te_text(empty),
            self.te_text(flush),
            self.te_text(empty),
        );
        self.lgx_pending_events.push(request);
    }

    /// Append `(selector, event, value)` to the page's outbox — HERE, where the statement asking for
    /// the event runs — and have the enclosing turn flush the outbox once it has committed.
    ///
    /// ★ THE EVENT WAITS FOR THE WRITE-BACK. Delivering here would run a nested turn whose changes
    /// this turn's own write-back then overwrites with values read before it ran, so it is queued
    /// and delivered after the turn.
    ///
    /// ★ AND IT IS QUEUED AT RUN TIME, BY THE STATEMENT ITSELF. What is recorded is whatever this
    /// statement evaluates to, only when it actually runs: a `Publish` under an `If` that is false
    /// queues nothing, one in a loop queues once per pass, and a value naming a loop variable is read
    /// while that variable exists. At compile time the turn only learns that it may have something
    /// to flush (`lgx_pending_events` holds the one flush request its epilogue dispatches — the
    /// reserved [`outbox_flush_event`], answered by the dispatcher's first arm).
    pub(crate) fn queue_element_event(
        &mut self,
        selector: &'a Expr<'a>,
        event: &'a Expr<'a>,
        value: &'a Expr<'a>,
    ) -> Stmt<'a> {
        self.request_outbox_flush();
        let count = self.interner.intern(outbox_count_binding());
        let next = self.te_bin(BinaryOpKind::Add, self.te_id(count), self.te_int(1));
        let mut appended = vec![Stmt::Set {
            target: count,
            value: next,
        }];
        let entry = [
            (outbox_selector_binding(), selector),
            (outbox_event_binding(), event),
            (outbox_value_binding(), self.as_text(value)),
        ];
        for (list, value) in entry {
            let list = self.interner.intern(list);
            appended.push(Stmt::SetIndex {
                collection: self.te_id(list),
                index: self.te_id(count),
                value,
            });
        }
        Stmt::Splice {
            body: self.alloc_block(appended),
        }
    }

    /// `mapOf(0, "")` — one outbox list with no entries: a map from position to `Text`, whose seed
    /// entry types it on every backend, read only at positions `1..=count`.
    pub(crate) fn empty_outbox_list(&mut self) -> &'a Expr<'a> {
        let map_of = self.interner.intern("mapOf");
        let empty = self.interner.intern("");
        self.te_call(map_of, vec![self.te_int(0), self.te_text(empty)])
    }

    /// `setAttribute(el, "class", <rule>(getAttribute(el, "class"), c))` — the shared shape of
    /// adding and removing a class, which differ only in which pure rule decides the new list.
    pub(crate) fn lower_class_change(
        &mut self,
        selector: &'a Expr<'a>,
        class: &'a Expr<'a>,
        rule: &str,
    ) -> Stmt<'a> {
        let query = self.interner.intern("querySelector");
        let get = self.interner.intern("getAttribute");
        let set = self.interner.intern("setAttribute");
        let rule = self.interner.intern(rule);
        let class_attr = self.interner.intern("class");
        let el = self.te_call(query, vec![selector]);
        let name = self.te_text(class_attr);
        let current = self.te_call(get, vec![el, name]);
        let next = self.te_call(rule, vec![current, class]);
        let el = self.te_call(query, vec![selector]);
        let name = self.te_text(class_attr);
        Stmt::Call {
            function: set,
            args: vec![el, name, next],
        }
    }

    /// `Save v as "k"` writes a SELF-DESCRIBING text: a one-letter tag, a colon, then the value.
    ///
    /// The tag is what lets `Load` give a field back what `Save` was handed. Browser storage holds
    /// only text, and a field holds a typed value — without the tag, what came out would not be
    /// what went in.
    ///
    /// A `Seq` or a declared record gets the `j:` tag, encoded through the derived JSON codec
    /// (`json.rs`) instead of the fixed one-letter scalar tags. `None` when the field NAMES one of
    /// those shapes but this file cannot actually round-trip it (a record with a field the codec
    /// has no case for) — refused at the `Save` statement rather than silently falling back to the
    /// scalar `t` tag, which would hand a Seq or a record to `as_text` and stringify neither.
    pub(crate) fn tagged_value(&mut self, value: &'a Expr<'a>) -> Option<&'a Expr<'a>> {
        // The tag names what went in. A `Save <field> as …` is the shape that carries a type worth
        // recording; anything else stores its display text and reads back as Text.
        let named = match value {
            Expr::Identifier(n) => self
                .lgx_state_types
                .iter()
                .find(|(f, _)| f == n)
                .and_then(|(_, t)| *t),
            _ => None,
        };
        if let Some(ty) = named {
            if self.needs_json_codec(ty) {
                let (enc, _) = self.json_codec_for(ty)?;
                return Some(self.json_tagged(enc, value));
            }
        }
        let tag = self.type_tag(named);
        let prefix = self.interner.intern(&format!("{tag}:"));
        let head = self.te_text(prefix);
        let body = self.as_text(value);
        Some(self.te_bin(BinaryOpKind::Add, head, body))
    }

    /// `"j:" + encoder(value)` — the JSON half of [`Self::tagged_value`], shared with the
    /// missing-key fallback [`Self::json_load_stmt`] builds (there, `value` is the field's own
    /// CURRENT value rather than the one being saved).
    fn json_tagged(&mut self, encoder: Symbol, value: &'a Expr<'a>) -> &'a Expr<'a> {
        let tag = self.interner.intern("j:");
        let head = self.te_text(tag);
        let encoded = self.te_call(encoder, vec![value]);
        self.te_bin(BinaryOpKind::Add, head, encoded)
    }

    /// The one-letter tag a type stores under. Text is the default, so a value this cannot name a
    /// type for still round-trips as its own display text rather than failing to lower.
    pub(crate) fn type_tag(&self, ty: Option<Symbol>) -> &'static str {
        match ty.map(|t| self.interner.resolve(t).to_string()).as_deref() {
            Some("Int") => "i",
            Some("Float") => "f",
            Some("Bool") => "b",
            _ => "t",
        }
    }

    /// `Load "k" into f` reads the tagged text back and converts it to the FIELD's declared type,
    /// as the whole `Stmt` the statement lowers to.
    ///
    /// `None` when the field cannot be typed, or names a `Seq`/record this file cannot actually
    /// round-trip — refused rather than guessed, the same rule `store_field_types` applies.
    ///
    /// A `Seq`/record field's own missing-key or malformed-JSON fallback is "leave the field at
    /// its current value": a `Stmt::Splice` reads the key with `json_load_stmt`'s own
    /// `"j:" + encoder(current)` fallback (so a missing key round-trips to itself) and only
    /// `Set`s the field on a successful decode, leaving it untouched on `Err`. A plain scalar
    /// keeps the exact fixed-zero fallback it always had, as an unconditional `Stmt::Set`.
    pub(crate) fn tagged_read(
        &mut self,
        key: &'a Expr<'a>,
        ty: Option<Symbol>,
        var: Symbol,
    ) -> Option<Stmt<'a>> {
        let ty = ty?;
        if self.needs_json_codec(ty) {
            let (enc, dec) = self.json_codec_for(ty)?;
            return Some(self.json_load_stmt(key, enc, dec, var));
        }
        // ★ THE FALLBACK IS TAGGED TOO, so a key that is not there takes the IDENTICAL path a key
        // that is there takes. An untagged `""` reached `parseInt` and the tiers answered
        // differently — the tree-walker raised `Cannot parse '' as Int` and the VM did not — which
        // is a `Load` of a forgotten key behaving one way in a test and another in a browser.
        // Written this way there is no absent-value branch to get wrong: a missing key reads back
        // as the type's empty value on every tier.
        let name = self.interner.resolve(ty).to_string();
        let zero = match name.as_str() {
            "Int" => "i:0",
            "Float" => "f:0.0",
            "Bool" => "b:false",
            "Text" => "t:",
            // A field this walk cannot type is REFUSED, never guessed at: decoding a value as the
            // wrong type is a wrong answer rather than a refusal, the worst shape a gap can take.
            _ => return None,
        };
        let get = self.interner.intern("storageGetOr");
        let area = self.storage_area("local");
        let empty = self.interner.intern(zero);
        let fallback = self.te_text(empty);
        let raw = self.te_call(get, vec![area, key, fallback]);
        // Drop the `x:` tag the write put on.
        let after = self.interner.intern("cutAfter");
        let colon = self.interner.intern(":");
        let colon_text = self.te_text(colon);
        let body = self.te_call(after, vec![raw, colon_text]);
        let parse = |me: &mut Self, f: &str| {
            let f = me.interner.intern(f);
            me.te_call(f, vec![body])
        };
        let value = match name.as_str() {
            "Int" => parse(self, "parseInt"),
            "Float" => parse(self, "parseFloat"),
            "Bool" => parse(self, "parseBool"),
            _ => body,
        };
        Some(Stmt::Set { target: var, value })
    }

    /// The `Seq`/record half of [`Self::tagged_read`]: `Let raw = storageGetOr(area, key, "j:" +
    /// encoder(<var's current value>)). Let body = cutAfter(raw, ":"). Inspect decoder(body): When
    /// Err (_): (nothing — `var` keeps its current value). When Ok (v): Set var to v.`
    ///
    /// The fallback text is the field's OWN current value, JSON-encoded — not a fixed literal,
    /// since a `Seq`/record has no universal "zero" the way a scalar's `i:0`/`t:` does. A missing
    /// key therefore decodes back to exactly the value that was already there, by construction,
    /// with no separate "was the key even present" branch to get wrong.
    fn json_load_stmt(
        &mut self,
        key: &'a Expr<'a>,
        enc: Symbol,
        dec: Symbol,
        var: Symbol,
    ) -> Stmt<'a> {
        let current = self.te_id(var);
        let fallback = self.json_tagged(enc, current);
        let get = self.interner.intern("storageGetOr");
        let area = self.storage_area("local");
        let raw_call = self.te_call(get, vec![area, key, fallback]);
        let raw_var = self.interner.intern("__lgx_load_raw");
        let after = self.interner.intern("cutAfter");
        let colon = self.interner.intern(":");
        let body_call = self.te_call(after, vec![self.te_id(raw_var), self.te_text(colon)]);
        let body_var = self.interner.intern("__lgx_load_body");
        let decoded = self.te_call(dec, vec![self.te_id(body_var)]);
        let err = self.interner.intern("__lgx_load_err");
        let no_op = Stmt::Splice {
            body: self.alloc_block(Vec::new()),
        };
        let ok_bind = self.interner.intern("__lgx_load_v");
        let set = Stmt::Set {
            target: var,
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
        let stmts = vec![
            Stmt::Let {
                var: raw_var,
                ty: None,
                value: raw_call,
                mutable: false,
            },
            Stmt::Let {
                var: body_var,
                ty: None,
                value: body_call,
                mutable: false,
            },
            inspect,
        ];
        Stmt::Splice {
            body: self.alloc_block(stmts),
        }
    }

    pub(crate) fn storage_area(&mut self, area: &'static str) -> &'a Expr<'a> {
        let sym = self.interner.intern(area);
        self.te_text(sym)
    }

    /// The store's own key: the author's for a persisted store, its NAME for a preserved one —
    /// a reload rebuilds the same program, so the name is stable identity and there is no key
    /// for the author to invent.
    pub(crate) fn storage_key(&mut self, def: &ComponentDef<'a>) -> &'a Expr<'a> {
        match &def.kind {
            ComponentKind::Store {
                mode: crate::ast::component::StoreMode::Persisted { key },
            } => key,
            ComponentKind::Store {
                mode: crate::ast::component::StoreMode::Shared { topic },
            } => topic,
            _ => {
                let sym = self
                    .interner
                    .intern(&format!("reload:{}", self.interner.resolve(def.name)));
                self.te_text(sym)
            }
        }
    }

    /// `"{key}.{field}"` — an interpolation, so a computed storage key works exactly as a
    /// literal one does and neither tier has to insist on a constant.
    pub(crate) fn storage_field_key(
        &mut self,
        def: &ComponentDef<'a>,
        field: Symbol,
    ) -> &'a Expr<'a> {
        use crate::ast::stmt::StringPart;
        let key = self.storage_key(def);
        let suffix = self
            .interner
            .intern(&format!(".{}", self.interner.resolve(field)));
        self.ctx
            .alloc_imperative_expr(Expr::InterpolatedString(vec![
                StringPart::Expr {
                    value: key,
                    format_spec: None,
                    debug: false,
                },
                StringPart::Literal(suffix),
            ]))
    }

    /// Stored text back to the field's type. The parse is the INVERSE of the interpolation the
    /// write-through used, so a value written by one tier reads back on the other.
    pub(crate) fn storage_parse(&mut self, raw: Symbol, ty: Symbol) -> &'a Expr<'a> {
        let value = self.te_id(raw);
        match self.interner.resolve(ty) {
            "Int" => {
                let f = self.interner.intern("parseInt");
                self.ctx.alloc_imperative_expr(Expr::Call {
                    function: f,
                    args: vec![value],
                })
            }
            "Float" | "Real" => {
                let f = self.interner.intern("parseFloat");
                self.ctx.alloc_imperative_expr(Expr::Call {
                    function: f,
                    args: vec![value],
                })
            }
            "Bool" => {
                let t = self.interner.intern("true");
                self.ctx.alloc_imperative_expr(Expr::BinaryOp {
                    op: crate::ast::stmt::BinaryOpKind::Eq,
                    left: value,
                    right: self.te_text(t),
                })
            }
            // Text, and nothing else reaches here: `store_lowerable` refuses a store whose field
            // has no faithful text form before this tier ever lowers it.
            _ => value,
        }
    }
}
