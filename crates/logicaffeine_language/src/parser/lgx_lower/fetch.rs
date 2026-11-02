use super::*;
use crate::ast::stmt::{BinaryOpKind, Expr, Stmt, TypeExpr};
use logicaffeine_base::Symbol;

impl<'a, 'ctx, 'int> super::super::Parser<'a, 'ctx, 'int> {
    /// The call a failed request makes to `When a fetch fails (url: Text) and (why: Text):`, or
    /// `None` when the component has no such handler.
    ///
    /// ★ AN OBSERVER, NOT A REPLACEMENT. The action that asked still receives its `Err` on both
    /// paths — a page that writes only the action is unchanged by this, and a page that writes only
    /// the handler still gets the `Err` it can ignore. Two deliveries of one fact.
    pub(crate) fn fetch_failed_call(
        &mut self,
        component: &str,
        url: &'a Expr<'a>,
        why: &'a Expr<'a>,
    ) -> Option<Stmt<'a>> {
        let (index, _, _) = self.lgx_fetch_fails?;
        let f = self
            .interner
            .intern(&action_fn_name(component, &format!("__e{index}")));
        let key = self.key_expr();
        Some(Stmt::Call {
            function: f,
            args: vec![key, url, why],
        })
    }

    /// `Get URL as T, then action.` → the correlated request plus a completion closure.
    ///
    /// The completion hands the action the value the request came to: a non-2xx is an ANSWER (`Err` carrying the status), a body that
    /// will not decode is an `Err` carrying the DECODER's message and the REAL status (the server
    /// did answer; we could not read it), and everything else is `Ok`.
    pub(crate) fn lower_typed_fetch(
        &mut self,
        component: &str,
        method: crate::ast::stmt::FetchMethod,
        url: &'a Expr<'a>,
        body: Option<&'a Expr<'a>>,
        as_type: &'a TypeExpr<'a>,
        then: Symbol,
    ) -> Option<Stmt<'a>> {
        use crate::ast::stmt::FetchMethod;
        let verb = match method {
            FetchMethod::Get => "GET",
            FetchMethod::Post => "POST",
            FetchMethod::Put => "PUT",
            FetchMethod::Patch => "PATCH",
            FetchMethod::Delete => "DELETE",
            FetchMethod::Upload => "POST",
        };
        // The read the answer takes — the ONE answer the gate refuses a component by
        // (`JsonShapes::fetch_read`), so a page the gate admits always has its decoder. `Text` is
        // the wire format and decodes to itself; a record reads through `fromJson_<T>`, a list
        // through `fromJsonSeq_<T>` and a bare scalar through its own `fromJson_<Scalar>`.
        let decode_fn = match self.json_shapes().fetch_read(as_type).ok()? {
            super::json::FetchRead::Text => None,
            super::json::FetchRead::Seq(elem) => Some(self.ensure_seq_codec(elem)?.1),
            super::json::FetchRead::Record(rec) => Some(self.ensure_record_codec(rec)?.1),
            super::json::FetchRead::Scalar(scalar) => Some(self.ensure_scalar_codec(scalar)?),
        };
        // Named the same as every other host-callback closure's own parameter
        // (`host_callback_closure`'s `ev_param`) — this fetch completion builds its own multi-
        // statement body instead of going through the helper (its `Inspect`/`If` chain is not a
        // single statement), but the closure it hands `dom_add_event_listener`-family imports must
        // still bind the SAME name the trampoline's one `Int` argument arrives under.
        let req = self.interner.intern("__lgx_event");
        let url_var = self.interner.intern("__lgx_fetch_url");
        let status = self.interner.intern("__lgx_fetch_status");
        let payload = self.interner.intern("__lgx_fetch_body");
        let act = self
            .interner
            .intern(&action_fn_name(component, self.interner.resolve(then)));

        let mut inner: Vec<Stmt<'a>> = Vec::new();
        let status_fn = self.interner.intern("responseStatus");
        let body_fn = self.interner.intern("responseBody");
        inner.push(Stmt::Let {
            var: status,
            ty: None,
            value: self.te_call(status_fn, vec![self.te_id(req)]),
            mutable: false,
        });
        inner.push(Stmt::Let {
            var: payload,
            ty: None,
            value: self.te_call(body_fn, vec![self.te_id(req)]),
            mutable: false,
        });

        // `<status>: <body>` — the message a non-2xx answer carries, word for word.
        let colon = self.interner.intern(": ");
        let blank = self.interner.intern("");
        let status_text = self.te_bin(BinaryOpKind::Add, self.te_text(blank), self.te_id(status));
        let with_colon = self.te_bin(BinaryOpKind::Add, status_text, self.te_text(colon));
        let failure_msg = self.te_bin(BinaryOpKind::Add, with_colon, self.te_id(payload));
        let http_failure = self.http_error_expr(self.te_id(status), failure_msg);
        let key = self.key_expr();
        let call_err = Stmt::Call {
            function: act,
            args: vec![key, self.result_err(http_failure)],
        };

        // 2xx or not: a non-2xx is an answer the page must be able to act on, never a crash.
        let two_hundred = self.te_int(200);
        let three_hundred = self.te_int(300);
        let too_low = self.te_bin(BinaryOpKind::Lt, self.te_id(status), two_hundred);
        let too_high = self.te_bin(BinaryOpKind::GtEq, self.te_id(status), three_hundred);
        let out_of_range = self.te_bin(BinaryOpKind::Or, too_low, too_high);

        // ★ `Text` ALWAYS DECODES — the payload IS the value, so the success path calls the
        // action directly with `Ok(payload)` rather than through an `Inspect` no decoder exists
        // to drive. Every other type reads through its derived `fromJson[Seq]_<T>`, which DOES
        // answer a `Result` (a body that will not parse is a real possibility for anything else).
        let success_block = if let Some(decode_fn) = decode_fn {
            let ok_var = self.interner.intern("__lgx_fetch_ok");
            let err_var = self.interner.intern("__lgx_fetch_err");
            let decoded = self.te_call(decode_fn, vec![self.te_id(payload)]);
            let ok_arm = crate::ast::stmt::MatchArm {
                enum_name: None,
                variant: Some(self.interner.intern("Ok")),
                bindings: vec![(self.interner.intern("value"), ok_var)],
                body: {
                    let ok_id = self.te_id(ok_var);
                    let wrapped = self.result_ok(ok_id);
                    let k = self.key_expr();
                    self.alloc_block(vec![Stmt::Call {
                        function: act,
                        args: vec![k, wrapped],
                    }])
                },
            };
            let decode_failure = self.http_error_expr(self.te_id(status), self.te_id(err_var));
            let err_arm = crate::ast::stmt::MatchArm {
                enum_name: None,
                variant: Some(self.interner.intern("Err")),
                bindings: vec![(self.interner.intern("error"), err_var)],
                body: {
                    let wrapped = self.result_err(decode_failure);
                    let k = self.key_expr();
                    let mut arm = vec![Stmt::Call {
                        function: act,
                        args: vec![k, wrapped],
                    }];
                    let (u, w) = (self.te_id(url_var), self.te_id(err_var));
                    if let Some(c) = self.fetch_failed_call(component, u, w) {
                        arm.push(c);
                    }
                    self.alloc_block(arm)
                },
            };
            let decode_stmt = Stmt::Inspect {
                target: decoded,
                arms: vec![ok_arm, err_arm],
                has_otherwise: false,
            };
            self.alloc_block(vec![decode_stmt])
        } else {
            let wrapped = self.result_ok(self.te_id(payload));
            let k = self.key_expr();
            self.alloc_block(vec![Stmt::Call {
                function: act,
                args: vec![k, wrapped],
            }])
        };
        inner.push(Stmt::If {
            cond: out_of_range,
            then_block: {
                let mut answered = vec![call_err];
                let u = self.te_id(url_var);
                if let Some(c) = self.fetch_failed_call(component, u, failure_msg) {
                    answered.push(c);
                }
                self.alloc_block(answered)
            },
            else_block: Some(success_block),
        });

        let closure = self.host_callback_closure(inner);

        let headers = self.interner.intern("__lgx_fetch_headers");
        let text_name = self.interner.intern("Text");
        let seq_name = self.interner.intern("Seq");
        let empty_headers = self.ctx.alloc_imperative_expr(Expr::New {
            type_name: seq_name,
            type_args: vec![TypeExpr::Primitive(text_name)],
            init_fields: Vec::new(),
        });
        let send = self.interner.intern("fetchSend");
        let verb_sym = self.interner.intern(verb);
        let body_expr = body.unwrap_or_else(|| self.te_text(blank));
        // ★ AN UPLOAD IS `multipart/form-data`, which is the only thing that distinguishes it
        // from a plain POST — without the framing a server receives a body with no idea it is a
        // FILE, and the sentence `Upload … to …` is a lie. The body and the header are built by
        // the SAME two stdlib functions, from the same content, so the boundary they declare and
        // the boundary they frame with cannot differ.
        //
        // A typed request that carries any OTHER body sends it as `application/json`, so a server
        // sees the same request shape regardless of which tier sent it.
        let ct_name = self.interner.intern("content-type");
        let (body_expr, header_setup) = if matches!(method, FetchMethod::Upload) {
            let mp_body = self.interner.intern("multipartBody");
            let mp_type = self.interner.intern("multipartContentType");
            let framed = self.te_call(mp_body, vec![body_expr]);
            let ctype = self.te_call(mp_type, vec![body_expr]);
            let setup = vec![
                Stmt::Push {
                    value: self.te_text(ct_name),
                    collection: self.te_id(headers),
                },
                Stmt::Push {
                    value: ctype,
                    collection: self.te_id(headers),
                },
            ];
            (framed, setup)
        } else if body.is_some() {
            let json = self.interner.intern("application/json");
            let setup = vec![
                Stmt::Push {
                    value: self.te_text(ct_name),
                    collection: self.te_id(headers),
                },
                Stmt::Push {
                    value: self.te_text(json),
                    collection: self.te_id(headers),
                },
            ];
            (body_expr, setup)
        } else {
            (body_expr, Vec::new())
        };
        let call = self.te_call(
            send,
            vec![
                self.te_text(verb_sym),
                self.te_id(url_var),
                body_expr,
                self.te_id(headers),
                closure,
            ],
        );
        let req_var = self.interner.intern("__lgx_fetch_req");
        // ★ THE URL IS BOUND BEFORE THE SEND, not read again at completion time. A failure handler
        // names the request that failed, and the sentence may compute that URL (`"/posts/" + id`);
        // evaluated a second time inside the completion closure it could name a DIFFERENT request
        // than the one that actually went out. Bound once, both halves say the same thing.
        let mut stmts = vec![
            Stmt::Let {
                var: url_var,
                ty: None,
                value: url,
                mutable: false,
            },
            Stmt::Let {
                var: headers,
                ty: None,
                value: empty_headers,
                mutable: true,
            },
        ];
        stmts.extend(header_setup);
        stmts.push(Stmt::Let {
            var: req_var,
            ty: None,
            value: call,
            mutable: false,
        });
        Some(Stmt::Splice {
            body: self.alloc_block(stmts),
        })
    }

    /// `a new HttpError with status … and message …`.
    pub(crate) fn http_error_expr(
        &mut self,
        status: &'a Expr<'a>,
        message: &'a Expr<'a>,
    ) -> &'a Expr<'a> {
        let ty = self.interner.intern("HttpError");
        let s = self.interner.intern("status");
        let m = self.interner.intern("message");
        self.ctx.alloc_imperative_expr(Expr::New {
            type_name: ty,
            type_args: Vec::new(),
            init_fields: vec![(s, status), (m, message)],
        })
    }
}
