use super::render::LowerTrail;
use super::*;
use crate::ast::component::{ComponentDef, ComponentKind, RouteBody};
use crate::ast::stmt::{BinaryOpKind, Expr, Stmt, TypeExpr};
use crate::ast::view::ViewNode;
use logicaffeine_base::Symbol;

/// The function one of a component's actions lowers to.
/// The `Request -> Response` the route table synthesizes, and what a bare `Serve on port N.`
/// serves with. Named here so the parser and the lowering cannot spell it differently.
pub const ROUTE_HANDLER: &str = "__lgx_route_handler";

/// The three walks over the route table. Each takes the WHOLE `Request` — see the wrappers at the
/// end of `lower_lgx_routes` for why the three-argument conveniences are built on top of these
/// rather than the other way around.
pub const ROUTE_BODY_WALK: &str = "__lgx_route_for";

pub const ROUTE_STATUS_WALK: &str = "__lgx_route_status_for";

pub const ROUTE_CTYPE_WALK: &str = "__lgx_route_ctype_for";

/// The FOURTH walk: the headers of the response the matched route answered.
///
/// ★ ONE WALK, TWO CONSUMERS, and that is the whole reason it exists rather than a `routeHeader`
/// that reaches in on its own. `__lgx_route_handler` puts these on the response it hands the
/// socket — which is how a `Set-Cookie` a route set actually reaches a browser — and
/// `routeHeader(method, url, name)` reads one out by name so a test can see it without a server.
/// Built arm-for-arm beside the other three, so it cannot disagree with them about which route
/// matched.
pub const ROUTE_HEADERS_WALK: &str = "__lgx_route_headers_for";

/// ★ THE SERVING WALK — the ONE `__lgx_route_handler` runs, answering a whole `Response`.
///
/// The other four answer one question each and are what `route(…)`, `routeStatus(…)`,
/// `routeContentType(…)` and `routeHeader(…)` are built on. The handler must not compose them:
/// `routeStatusWith` and `routeContentTypeWith` answer per-route CONSTANTS and never call a route's
/// function, which is exactly what made "the handler runs once per request" true — and a `Response`
/// route breaks it, because its status and content type are FIELDS OF THE ANSWER. Reading them
/// through those walks would call the route once per walk, so a login would execute four times for
/// one request: harmless for setting a cookie, a bug for writing a session row.
///
/// So this walk exists, built in the SAME loop from the SAME conditions as the other four, and it
/// calls each matched route exactly once. `## Before each request` runs once in it too, which the
/// old body-and-status pair never managed either.
pub const ROUTE_RESPONSE_WALK: &str = "__lgx_route_response_for";

/// The module global a `## Session` lives in. Named `session` on purpose: it makes
/// `the session's user` an ordinary possessive rather than a sentence form of its own.
pub const SESSION_VAR: &str = "session";

pub const SESSION_SECRET_VAR: &str = "__lgx_secret";

pub const SESSION_LOADED_VAR: &str = "__lgx_session_loaded";

pub const SESSION_PAYLOAD_FN: &str = "__lgx_session_payload";

pub const SESSION_LOAD_FN: &str = "__lgx_session_load";

pub const SESSION_SAVE_FN: &str = "__lgx_session_save";

/// The one cookie. See `lower_session` for why there is exactly one.
pub const SESSION_COOKIE: &str = "lgxsession";

/// The hidden field every session carries, whether the author declared it or not: the value a
/// visitor's CSRF token is derived from.
///
/// ★ A NONCE RATHER THAN THE SESSION'S CONTENTS, and the difference is the whole protection.
/// Deriving the token from the fields would hand every ANONYMOUS visitor the same token — they all
/// carry the same empty session — and a token every visitor shares is one an attacker reads off
/// their own page. It is minted the first time `csrf()` is called on a visit, which is the moment
/// a page needs one and not before, so a visitor who never sees a form is never given a cookie.
pub const SESSION_NONCE_FIELD: &str = "__csrfnonce";

/// The form field and the header a token may arrive in. `_csrf` is the one `csrf_lock` already
/// requires of every state-changing `<form>`; the header is how a scripted `fetch` sends one.
pub const CSRF_FIELD: &str = "_csrf";

pub const CSRF_HEADER: &str = "x-csrf-token";

/// What a page calls to get its own token: `value={csrf()}`.
pub const CSRF_FN: &str = "csrf";

/// Where a server with a `## Session` answers the visitor's own CSRF token — the value a page
/// composed around it (`<meta name="csrf-token">`) hands its scripted requests. The path is the
/// router's own, never a route an author declares.
pub const CSRF_TOKEN_PATH: &str = "/__logos/csrf";

/// Whether a lowered program declares a `## Session` — read off the program itself, because the
/// session's loader is defined exactly when one is. A serving tier that composes page documents
/// asks the router for the visitor's token ([`CSRF_TOKEN_PATH`]) only then: without a session there
/// is no token to ask for, and asking would run the handler for every page served.
pub fn declares_session(stmts: &[Stmt<'_>], interner: &logicaffeine_base::Interner) -> bool {
    stmts.iter().any(|s| {
        matches!(s, Stmt::FunctionDef { name, .. } if interner.resolve(*name) == SESSION_LOAD_FN)
    })
}

/// ★ `components` IS NOT OPTIONAL CONTEXT — IT DECIDES THE ANSWER.
///
/// A `-> View` route's template is checked by the server-side walk, and that walk refuses a
/// component it CANNOT SEE (rightly: whether an unseen child renders on a server is unknown). This
/// function used to call the no-list form, so **every `## Route … -> View:` that placed a component
/// was quietly dropped from the router** — the route never reached the dispatcher, and
/// `route("GET", "/report")` came back as an unknown function, or as a 404 for a route the author
/// can see in their file. Which is the whole shape a page is BUILT out of: a row, a card, a field.
///
/// So every caller hands over the components it can see. A caller that genuinely has none (a route
/// checked on its own) passes an empty slice and gets the honest, conservative refusal.
/// The kinds of field a `## Session` may carry.
///
/// Deliberately three. A session rides in a COOKIE — a few hundred bytes a visitor holds and a
/// browser sends on every request — so it is the wrong place for a list, a record or a map, and
/// accepting one would be inviting a page to put its database in a header. Anything else is
/// refused BY NAME at compile time rather than encoded as something plausible.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SessionFieldKind {
    Text,
    Int,
    Bool,
}

/// Where a route parameter's value comes from at dispatch.
///
/// ★ THREE-STATE ON PURPOSE, replacing a `from_path: bool`. A boolean could only ever ask "path or
/// not", so the arrival of a third source had nowhere to go but a second boolean beside the first —
/// and a pair of booleans has four states of which one is nonsense. Making the question total means
/// a fourth source cannot be added without every `match` on it failing to compile, which is the
/// same discipline `is_void` and `action_verb_tier_lock` already hold elsewhere.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParamFrom {
    /// A `:name` segment of the route's own path pattern.
    Path,
    /// The query string, or the request body for a method that carries one.
    Extra,
    /// The whole request, bound as-is.
    Request,
}

impl<'a, 'ctx, 'int> super::super::Parser<'a, 'ctx, 'int> {
    /// Where a route parameter's raw text comes from: a `:name` slot in the PATH, or the
    /// QUERY when the pattern has no slot for it.
    ///
    /// The path wins when a name appears in both — a path segment is part of the route's
    /// IDENTITY, while a query is decoration any client may append to any URL, so letting
    /// `?id=99` override `/todos/7` would let a caller aim a request at one record and have it
    /// served another.
    /// ★ `## Session` — a visitor's state, carried between requests in ONE signed cookie.
    ///
    /// It parsed and did NOTHING before this: `## Session\n    a user, which is Text.` compiled
    /// clean, ran, and the declaration was discarded as prose — the exact failure the four server
    /// headers were fixed for, where an author is told their intent was understood and it was not.
    ///
    /// What gets built, and why each piece is the shape it is:
    ///
    /// * A module global named `session`, so `the session's user` is ORDINARY LOGOS — a possessive
    ///   on a record, the same form `todo's id` uses everywhere else. No new sentence form, no
    ///   special case in the parser, and every tier that can read a record global reads this one.
    /// * `__lgx_session_load` and `__lgx_session_save`, called by the router: the load runs in the
    ///   walks' shared prelude, so it happens once per request before anything dispatches, and the
    ///   save runs in the handler, where the response is finally in hand.
    /// * ONE cookie. `HttpResponse.headers` is a `BTreeMap`, so a response carries exactly one
    ///   `Set-Cookie` — a second silently overwrites the first. One is not a simplification here,
    ///   it is the only correct number, and it is why the CSRF token is DERIVED from the session
    ///   rather than being a cookie of its own.
    /// * A payload of hex fields joined by `.`. Hex because a cookie value may not contain `;`,
    ///   `,`, `"`, a space or a control character and a session holds text a visitor typed; the
    ///   separator is safe precisely because no hex digit is a `.`, so there is nothing to escape
    ///   and no escaping bug to have.
    /// * The cookie is re-issued only when the payload CHANGED. Setting it on every reply would
    ///   put a `Set-Cookie` on every 404 and every asset, and would silently overwrite a cookie a
    ///   route set for itself — there is only the one slot.
    pub(crate) fn lower_session(&mut self) -> crate::parser::ParseResult<()> {
        use crate::analysis::registry::{FieldType, TypeDef};
        if !self.session_declared {
            return Ok(());
        }
        let session_ty = self.interner.intern("Session");
        let declared: Vec<(Symbol, FieldType)> = match self.type_registry.as_ref().and_then(|r| {
            r.iter_types()
                .find(|(n, _)| **n == session_ty)
                .map(|(_, d)| d.clone())
        }) {
            Some(TypeDef::Struct {
                fields, generics, ..
            }) if generics.is_empty() => fields.iter().map(|f| (f.name, f.ty.clone())).collect(),
            // A `## Session` whose fields the record scanner could not read at all. It has already
            // reported that; synthesizing a session with no fields on top would bury it.
            _ => return Ok(()),
        };
        let mut fields: Vec<(Symbol, SessionFieldKind)> = Vec::new();
        for (name, ty) in &declared {
            let named = match ty {
                FieldType::Primitive(s) | FieldType::Named(s) => self.interner.resolve(*s),
                _ => "",
            };
            let kind = match named.to_ascii_lowercase().as_str() {
                "text" => SessionFieldKind::Text,
                "int" | "nat" => SessionFieldKind::Int,
                "bool" => SessionFieldKind::Bool,
                _ => {
                    let field = self.interner.resolve(*name).to_string();
                    let span = self
                        .type_registry
                        .as_ref()
                        .and_then(|r| r.field_span(session_ty, *name))
                        .unwrap_or_default();
                    return Err(crate::error::ParseError {
                        kind: crate::error::ParseErrorKind::Custom(format!(
                            "`## Session` carries `{field}`, which a cookie cannot hold. A session \
                             travels in a few hundred bytes the visitor keeps and the browser \
                             sends with every single request, so it holds Text, Int and Bool and \
                             nothing larger. What does the NEXT request actually need to know? \
                             Usually that is an id — keep the id in the session and look the rest \
                             up from it."
                        )),
                        span,
                    });
                }
            };
            fields.push((*name, kind));
        }
        if fields.is_empty() {
            return Ok(());
        }

        let text_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Text")));
        let session_named: &'a TypeExpr<'a> = self.ctx.alloc_type_expr(TypeExpr::Named(session_ty));
        let response_named: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Named(self.interner.intern("Response")));
        let request_named: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Named(self.interner.intern("Request")));
        let blank = self.interner.intern("");
        let session_var = self.interner.intern(SESSION_VAR);
        let secret_var = self.interner.intern(SESSION_SECRET_VAR);
        let loaded_var = self.interner.intern(SESSION_LOADED_VAR);

        // The globals: the secret (read ONCE, at startup), the session itself, and the payload as
        // it arrived — which is what "did this request change anything?" is answered against.
        let secret_call = {
            let f = self.interner.intern("sessionSecret");
            self.te_call(f, Vec::new())
        };
        self.deferred_functions.push(Stmt::Let {
            var: secret_var,
            ty: Some(text_ty),
            value: secret_call,
            mutable: true,
        });
        let fresh = self.fresh_session(session_ty, &fields);
        self.deferred_functions.push(Stmt::Let {
            var: session_var,
            ty: Some(session_named),
            value: fresh,
            mutable: true,
        });
        self.deferred_functions.push(Stmt::Let {
            var: loaded_var,
            ty: Some(text_ty),
            value: self.te_text(blank),
            mutable: true,
        });

        // `__lgx_session_payload()` — every field as hex, joined by `.`.
        let dot = self.interner.intern(".");
        let hex = self.interner.intern("hexText");
        let mut payload: Option<&'a Expr<'a>> = None;
        for (name, _) in &fields {
            let read = self.ctx.alloc_imperative_expr(Expr::FieldAccess {
                object: self.te_id(session_var),
                field: *name,
            });
            // `"" + value` is how a non-Text is shown, the same way a bare-scalar route answers.
            let shown = self.te_bin(BinaryOpKind::Add, self.te_text(blank), read);
            let piece = self.te_call(hex, vec![shown]);
            payload = Some(match payload {
                None => piece,
                Some(acc) => {
                    let with_dot = self.te_bin(BinaryOpKind::Add, acc, self.te_text(dot));
                    self.te_bin(BinaryOpKind::Add, with_dot, piece)
                }
            });
        }
        let payload = payload.expect("at least one field");
        let payload_fn = self.interner.intern(SESSION_PAYLOAD_FN);
        self.deferred_functions.push(Stmt::FunctionDef {
            name: payload_fn,
            generics: Vec::new(),
            params: Vec::new(),
            body: self.alloc_block(vec![Stmt::Return {
                value: Some(payload),
            }]),
            return_type: Some(text_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        });

        // `__lgx_session_load(request)` — verify the cookie, fill the fields, and record what
        // arrived. A cookie is attacker-controlled input, so a signature that does not check is
        // not an error and not a crash: it is simply a visitor with no session.
        let req_p = self.interner.intern("__lgx_sreq");
        let mut load: Vec<Stmt<'a>> = Vec::new();
        let fresh = self.fresh_session(session_ty, &fields);
        load.push(Stmt::Set {
            target: session_var,
            value: fresh,
        });
        let cookie_name = self.interner.intern(SESSION_COOKIE);
        let cookie_of = self.interner.intern("cookieOf");
        let raw = self.te_call(
            cookie_of,
            vec![self.te_id(req_p), self.te_text(cookie_name)],
        );
        let verify = self.interner.intern("verifySession");
        let carried = self.interner.intern("__lgx_scarried");
        let mut filled: Vec<Stmt<'a>> = Vec::new();
        for (i, (name, kind)) in fields.iter().enumerate() {
            let reader = match kind {
                SessionFieldKind::Text => "sessionField",
                SessionFieldKind::Int => "sessionInt",
                SessionFieldKind::Bool => "sessionBool",
            };
            let f = self.interner.intern(reader);
            // LOGOS indices count from 1, and the payload's fields are read with `item i of`.
            let idx = self.te_int(i as i64 + 1);
            let value = self.te_call(f, vec![self.te_id(carried), idx]);
            filled.push(Stmt::SetField {
                object: self.te_id(session_var),
                field: *name,
                value,
            });
        }
        let some_arm = crate::ast::stmt::MatchArm {
            enum_name: None,
            variant: Some(self.interner.intern("Some")),
            bindings: vec![(carried, carried)],
            body: self.alloc_block(filled),
        };
        let none_arm = crate::ast::stmt::MatchArm {
            enum_name: None,
            variant: Some(self.interner.intern("None")),
            bindings: Vec::new(),
            body: self.alloc_block(Vec::new()),
        };
        load.push(Stmt::Inspect {
            target: self.te_call(verify, vec![self.te_id(secret_var), raw]),
            arms: vec![some_arm, none_arm],
            has_otherwise: false,
        });
        // ★ RECORDED AFTER LOADING, WHETHER OR NOT A COOKIE ARRIVED. The question the save asks is
        // "did this REQUEST change the session", so the baseline has to be the session as it was
        // when the request started — which for a visitor with no cookie is the empty one, not the
        // empty STRING. Recording `""` instead would make every anonymous visit look like a
        // change and put a `Set-Cookie` on every reply.
        let now = self.te_call(payload_fn, Vec::new());
        load.push(Stmt::Set {
            target: loaded_var,
            value: now,
        });
        let load_fn = self.interner.intern(SESSION_LOAD_FN);
        self.deferred_functions.push(Stmt::FunctionDef {
            name: load_fn,
            generics: Vec::new(),
            params: vec![(req_p, request_named)],
            body: self.alloc_block(load),
            return_type: None,
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        });

        // `csrf()` — the token a form puts in its `_csrf` field, minted on first use.
        //
        // Minting HERE, rather than when the session loads, is what keeps the promise that a
        // visitor who never sees a form is never given a cookie: asking for a token is the moment
        // one is needed. And because minting writes to the session, the save below issues the
        // cookie on that very response — no separate "start a session" step to forget.
        let nonce_field = self.interner.intern(SESSION_NONCE_FIELD);
        let read_nonce = self.ctx.alloc_imperative_expr(Expr::FieldAccess {
            object: self.te_id(session_var),
            field: nonce_field,
        });
        let unminted = self.te_bin(BinaryOpKind::Eq, read_nonce, self.te_text(blank));
        let fresh_nonce = {
            let f = self.interner.intern("randomSecret");
            self.te_call(f, Vec::new())
        };
        let mint = self.alloc_block(vec![Stmt::SetField {
            object: self.te_id(session_var),
            field: nonce_field,
            value: fresh_nonce,
        }]);
        let read_nonce = self.ctx.alloc_imperative_expr(Expr::FieldAccess {
            object: self.te_id(session_var),
            field: nonce_field,
        });
        let token = {
            let f = self.interner.intern("csrfToken");
            self.te_call(f, vec![self.te_id(secret_var), read_nonce])
        };
        let csrf_fn = self.interner.intern(CSRF_FN);
        self.deferred_functions.push(Stmt::FunctionDef {
            name: csrf_fn,
            generics: Vec::new(),
            params: Vec::new(),
            body: self.alloc_block(vec![
                Stmt::If {
                    cond: unminted,
                    then_block: mint,
                    else_block: None,
                },
                Stmt::Return { value: Some(token) },
            ]),
            return_type: Some(text_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        });

        // `__lgx_session_save(response)` — re-issue the cookie only if the payload changed.
        let resp_p = self.interner.intern("__lgx_sresp");
        let now_var = self.interner.intern("__lgx_snow");
        let mut save: Vec<Stmt<'a>> = Vec::new();
        let now = self.te_call(payload_fn, Vec::new());
        save.push(Stmt::Let {
            var: now_var,
            ty: Some(text_ty),
            value: now,
            mutable: false,
        });
        let same = self.te_bin(
            BinaryOpKind::Eq,
            self.te_id(now_var),
            self.te_id(loaded_var),
        );
        let unchanged = self.alloc_block(vec![Stmt::Return {
            value: Some(self.te_id(resp_p)),
        }]);
        save.push(Stmt::If {
            cond: same,
            then_block: unchanged,
            else_block: None,
        });
        let sign = self.interner.intern("signSession");
        let signed = self.te_call(sign, vec![self.te_id(secret_var), self.te_id(now_var)]);
        let with_cookie = self.interner.intern("withCookie");
        let set = self.te_call(
            with_cookie,
            vec![self.te_id(resp_p), self.te_text(cookie_name), signed],
        );
        save.push(Stmt::Return { value: Some(set) });
        let save_fn = self.interner.intern(SESSION_SAVE_FN);
        self.deferred_functions.push(Stmt::FunctionDef {
            name: save_fn,
            generics: Vec::new(),
            params: vec![(resp_p, response_named)],
            body: self.alloc_block(save),
            return_type: Some(response_named),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        });
        Ok(())
    }

    /// `a new Session with <every field at its empty value>` — a visitor who has never been seen.
    pub(crate) fn fresh_session(
        &mut self,
        session_ty: Symbol,
        fields: &[(Symbol, SessionFieldKind)],
    ) -> &'a Expr<'a> {
        let init: Vec<(Symbol, &'a Expr<'a>)> = fields
            .iter()
            .map(|(name, kind)| {
                let zero = match kind {
                    SessionFieldKind::Text => {
                        let blank = self.interner.intern("");
                        self.te_text(blank)
                    }
                    SessionFieldKind::Int => self.te_int(0),
                    SessionFieldKind::Bool => self.te_bool(false),
                };
                (*name, zero)
            })
            .collect();
        self.ctx.alloc_imperative_expr(Expr::New {
            type_name: session_ty,
            type_args: Vec::new(),
            init_fields: init,
        })
    }

    /// `a new Map of Text to Text` — the headers of an answer that set none.
    pub(crate) fn empty_header_map(&mut self) -> &'a Expr<'a> {
        let map_ty = self.interner.intern("Map");
        let text_arg = TypeExpr::Primitive(self.interner.intern("Text"));
        self.ctx.alloc_imperative_expr(Expr::New {
            type_name: map_ty,
            type_args: vec![text_arg.clone(), text_arg],
            init_fields: Vec::new(),
        })
    }

    /// `a new Response with status <status> and contentType <ctype> and body <body>`, no headers.
    pub(crate) fn response_value(
        &mut self,
        status: &'a Expr<'a>,
        ctype: &'a Expr<'a>,
        body: &'a Expr<'a>,
    ) -> &'a Expr<'a> {
        let headers = self.empty_header_map();
        self.ctx.alloc_imperative_expr(Expr::New {
            type_name: self.interner.intern("Response"),
            type_args: Vec::new(),
            init_fields: vec![
                (self.interner.intern("status"), status),
                (self.interner.intern("contentType"), ctype),
                (self.interner.intern("body"), body),
                (self.interner.intern("headers"), headers),
            ],
        })
    }

    /// The router's OWN answer — a 404, a 405, a 400, a moved page. Always plain text, because
    /// that is what those answers are.
    pub(crate) fn plain_response(&mut self, code: i64, body: &'a Expr<'a>) -> &'a Expr<'a> {
        let plain = self.interner.intern("text/plain; charset=utf-8");
        let ctype = self.te_text(plain);
        let status = self.te_int(code);
        self.response_value(status, ctype, body)
    }

    /// The type of a header table, `Map of Text to Text`.
    pub(crate) fn header_map_type(&mut self) -> &'a TypeExpr<'a> {
        let text = TypeExpr::Primitive(self.interner.intern("Text"));
        let params = self.ctx.alloc_type_exprs(vec![text.clone(), text]);
        let base = self.interner.intern("Map");
        self.ctx.alloc_type_expr(TypeExpr::Generic { base, params })
    }

    pub(crate) fn route_param_value(
        &mut self,
        from: ParamFrom,
        pattern: Symbol,
        name: Symbol,
        path_p: Symbol,
        query_p: Symbol,
        body_p: Option<Symbol>,
        req_p: Symbol,
    ) -> &'a Expr<'a> {
        // The request is the dispatcher's own parameter, so a route asking for it is handed the
        // very value the walk was called with — headers, query and all.
        if from == ParamFrom::Request {
            return self.te_id(req_p);
        }
        let name_text = self.te_text(name);
        if from == ParamFrom::Path {
            let get = self.interner.intern("routeParam");
            let pat = self.te_text(pattern);
            return self.te_call(get, vec![pat, name_text, self.te_id(path_p)]);
        }
        // ★ A METHOD THAT CARRIES A BODY READS ITS EXTRAS FROM THE BODY. That is where a
        // `<form method="post">` puts them and where a scripted JSON post puts them; a query
        // string on a POST is neither, and reading one would leave every ordinary form silently
        // handing the route empty values.
        match body_p {
            Some(body) => {
                let get = self.interner.intern("bodyParam");
                self.te_call(get, vec![self.te_id(body), name_text])
            }
            None => {
                let get = self.interner.intern("queryParam");
                self.te_call(get, vec![self.te_id(query_p), name_text])
            }
        }
    }

    /// ★ ON A SERVER A VIEW IS ITS HTML — every `-> View` function gains a twin answering its markup.
    ///
    /// `## To answer (who: Text) -> View:` returns a view — DATA on every lowered tier, one
    /// `__View` variant per template (`views.rs`). A route's answer, `Show`, a test want the markup,
    /// so each such function gains a TWIN, `__lgx_viewhtml_<f>`, taking the same parameters and
    /// answering the view it returns drawn by the serving walk and serialized (`serve_markup`). The
    /// router calls the twin; the original stays the view, which a template's hole or a browser
    /// view places as nodes.
    ///
    /// A function is twinned when every template it can answer is one the server can draw. Twins
    /// exist where a server renders — a program with a route or an `html of` — and where no page
    /// can place a view at all, so every reader wants its markup; a page's own bundle carries none.
    pub(crate) fn lower_view_function_twins(&mut self, statements: &[Stmt<'a>]) {
        use crate::ast_depth::{for_each_node, Visited};
        if self.lgx_views.functions.is_empty()
            || !(self.renders_on_a_server(statements) || no_page_places_a_view(statements))
        {
            return;
        }
        let fns: Vec<(Symbol, Vec<(Symbol, &'a TypeExpr<'a>)>, &'a [Stmt<'a>])> = statements
            .iter()
            .filter_map(|s| match s {
                Stmt::FunctionDef {
                    name, params, body, ..
                } if self.lgx_views.functions.contains(name) => {
                    Some((*name, params.clone(), *body))
                }
                _ => None,
            })
            .collect();
        let names: Vec<Symbol> = fns.iter().map(|(n, ..)| *n).collect();
        let defs: Vec<ComponentDef<'a>> = statements
            .iter()
            .filter_map(|s| match s {
                Stmt::ComponentDef(d) => Some((**d).clone()),
                _ => None,
            })
            .collect();
        // Every view function `stmts` reads: called, or named as a value.
        let reads = |stmts: &[Stmt<'a>]| -> Vec<Symbol> {
            let mut out: Vec<Symbol> = Vec::new();
            for_each_node(stmts, |node| {
                let named = match node {
                    Visited::Expr(Expr::Call { function, .. })
                    | Visited::Stmt(Stmt::Call { function, .. }) => *function,
                    Visited::Expr(Expr::Identifier(sym)) => *sym,
                    _ => return,
                };
                if names.contains(&named) && !out.contains(&named) {
                    out.push(named);
                }
            });
            out
        };
        let templates = self.lgx_views.templates.clone();
        let calls: Vec<Vec<Symbol>> = fns
            .iter()
            .map(
                |(name, _, body)| match templates.iter().find(|t| t.name == *name) {
                    Some(t) => {
                        let mut held = Vec::new();
                        super::views::view_holes(&t.view.nodes, &mut |hole| {
                            if let Expr::Call { function, .. } = hole {
                                if names.contains(function) && !held.contains(function) {
                                    held.push(*function);
                                }
                            }
                        });
                        held
                    }
                    None => reads(body),
                },
            )
            .collect();
        let mut twinned: Vec<Symbol> = fns
            .iter()
            .filter(
                |(name, ..)| match templates.iter().find(|t| t.name == *name) {
                    Some(t) => {
                        view_html_lowerable_with(&t.view.nodes, &defs, self.interner).is_ok()
                    }
                    None => true,
                },
            )
            .map(|(name, ..)| *name)
            .collect();
        loop {
            let before = twinned.len();
            let keep: Vec<Symbol> = fns
                .iter()
                .zip(&calls)
                .filter(|((name, ..), called)| {
                    twinned.contains(name) && called.iter().all(|c| twinned.contains(c))
                })
                .map(|((name, ..), _)| *name)
                .collect();
            twinned = keep;
            if twinned.len() == before {
                break;
            }
        }
        let text_ty = self.text_type();
        for (name, params, _) in fns.iter().filter(|(name, ..)| twinned.contains(name)) {
            let twin = self
                .interner
                .intern(&view_html_fn_name(self.interner.resolve(*name)));
            let view = self.interner.intern("__lgx_answered");
            let args: Vec<&'a Expr<'a>> = params.iter().map(|(p, _)| self.te_id(*p)).collect();
            let answered = self.te_call(*name, args);
            let mut body = vec![Stmt::Let {
                var: view,
                ty: None,
                value: answered,
                mutable: false,
            }];
            let blank = self.interner.intern("");
            let place = Stmt::Call {
                function: self.interner.intern(place_view_fn_name()),
                args: vec![self.te_id(view), self.te_text(blank), self.te_int(1)],
            };
            let out = self.interner.intern("__lgx_html");
            self.serve_markup(vec![place], out, &mut body);
            body.push(Stmt::Return {
                value: Some(self.te_id(out)),
            });
            self.deferred_functions.push(Stmt::FunctionDef {
                name: twin,
                generics: Vec::new(),
                params: params.clone(),
                body: self.alloc_block(body),
                return_type: Some(text_ty),
                is_native: false,
                native_path: None,
                is_exported: false,
                export_target: None,
                opt_flags: crate::optimization::OptimizationConfig::all_on(),
            });
        }
    }

    /// In a program no page places a view in, every reader of one outside the view functions —
    /// `## Main`, an ordinary function — reads its markup: each such read goes through the twin.
    pub(crate) fn read_views_as_markup(&mut self, statements: &mut [Stmt<'a>]) {
        if !no_page_places_a_view(statements) {
            return;
        }
        let twins = self.served_view_twins(statements);
        if twins.is_empty() {
            return;
        }
        for stmt in statements.iter_mut() {
            match stmt {
                Stmt::FunctionDef { name, body, .. } => {
                    if !self.lgx_views.functions.contains(name) {
                        *body = self.serve_views_through_twins(body, &twins);
                    }
                }
                other => {
                    let alone = self.alloc_block(vec![other.clone()]);
                    let mapped = self.serve_views_through_twins(alone, &twins);
                    if let Some(first) = mapped.first() {
                        *other = first.clone();
                    }
                }
            }
        }
    }

    /// The twin a `-> View` function is served through, when it has one.
    pub(crate) fn view_html_twin(&self, function: Symbol) -> Option<Symbol> {
        let twin = self
            .interner
            .lookup(&view_html_fn_name(self.interner.resolve(function)))?;
        self.deferred_functions
            .iter()
            .any(|s| matches!(s, Stmt::FunctionDef { name, .. } if *name == twin))
            .then_some(twin)
    }

    /// `body` with every read of a twinned view function taken through its twin.
    pub(crate) fn serve_views_through_twins(
        &mut self,
        body: &'a [Stmt<'a>],
        twins: &[(Symbol, Symbol)],
    ) -> &'a [Stmt<'a>] {
        if twins.is_empty() {
            return body;
        }
        let arenas = crate::ast_map::Arenas {
            exprs: self.ctx.imperative_exprs.expect("imperative arenas"),
            stmts: self.ctx.stmts.expect("imperative arenas"),
            types: self.ctx.type_exprs.expect("imperative arenas"),
        };
        crate::ast_map::AstMap::map_block(&mut TwinCalls { arenas, twins }, body)
    }

    /// Every `-> View` function in `statements` that is served through a twin, with its twin.
    pub(crate) fn served_view_twins(&self, statements: &[Stmt<'a>]) -> Vec<(Symbol, Symbol)> {
        statements
            .iter()
            .filter_map(|s| match s {
                Stmt::FunctionDef {
                    name, return_type, ..
                } if self.is_view_function(*name, *return_type) => {
                    self.view_html_twin(*name).map(|twin| (*name, twin))
                }
                _ => None,
            })
            .collect()
    }

    pub(crate) fn lower_lgx_routes(&mut self, statements: &[Stmt<'a>]) {
        struct Route {
            method: String,
            path: Symbol,
            fname: Symbol,
            /// (name, is_int, from_path) — `from_path` is false for a parameter the path
            /// pattern has no `:name` slot for, which is read from the QUERY instead. Decided
            /// here, at compile time, because the pattern is known: a runtime fallback would
            /// have to guess which source an empty value came from.
            params: Vec<(Symbol, bool, ParamFrom)>,
            literal: bool,
            /// The record this route answers, if it answers one — the dispatcher wraps the call
            /// in that record's derived encoder so the wire gets JSON.
            /// The record a route's answer is encoded through, and whether it is a LIST of them
            /// (`toJsonSeq_<T>` rather than `toJson_<T>`) — a collection endpoint's shape.
            encode: Option<(Symbol, bool)>,
            /// Does this method carry a request BODY? `POST`/`PUT`/`PATCH` do, and a declared
            /// parameter the path has no slot for is read from it rather than from the query —
            /// which is what a `<form method="post">` and a scripted JSON post both send.
            has_body: bool,
            /// The `Content-Type` this route's answer goes out with — decided from its RETURN
            /// TYPE, where the author already said what they are sending. A server that types
            /// every answer `text/plain` makes a browser show JSON as text and refuse to render
            /// HTML, so the type has to travel with the body.
            content_type: &'static str,
            /// Answers a bare JSON scalar (a number, a boolean): no encoder to call, but the
            /// value still has to become Text, and it goes out as JSON rather than as prose.
            bare_scalar: bool,
            /// Answers a whole `Response` — a status, a content type, a body and headers the
            /// route decided for itself.
            ///
            /// ★ A `Response` IS THE ANSWER, NOT A VALUE TO ENCODE, and conflating those was the
            /// whole bug. `route_lowerable` asked "is this a record?" and said yes; the emitter
            /// asked "is this record JSON-encodable?" and said no, because `Response` holds a
            /// `Map` and a `Seq of Int`. So `toJson_Response` was demanded, never derived, and
            /// every such route died at `Unknown function`. Two questions that should be one.
            ///
            /// Everything downstream reads a FIELD of it rather than encoding it, which is what
            /// lets a route carry a status, a `Location`, or a `Set-Cookie` at all.
            is_response: bool,
        }
        // Nothing to do, and — importantly — nothing to ALLOCATE. Allocating the shared type
        // exprs before this check made every program touch the type-expr arena, including the
        // logic-mode parsers that build an `AstContext` without one ("type_exprs arena not
        // initialized"). A lowering for a construct the file does not use must cost nothing.
        if !statements.iter().any(|s| {
            matches!(
                s,
                Stmt::RouteDef { .. } | Stmt::MiddlewareDef { .. } | Stmt::RedirectDef { .. }
            )
        }) {
            return;
        }
        let text_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Text")));
        let int_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Int")));
        let view_twins = self.served_view_twins(statements);
        let record_names: Vec<Symbol> = self
            .type_registry
            .as_ref()
            .map(|r| {
                r.iter_types()
                    .filter(|(_, d)| {
                        matches!(d, crate::analysis::registry::TypeDef::Struct { generics, .. }
                            if generics.is_empty())
                    })
                    .map(|(n, _)| *n)
                    .collect()
            })
            .unwrap_or_default();
        let is_record = |s: Symbol| record_names.contains(&s);
        // The components this file declares — what the server-side walk needs in order to say
        // whether a `-> View` route's template can be drawn. `lower_lgx_components` has already
        // run, so a child's own renderability is known before its route asks.
        let defs: Vec<ComponentDef<'a>> = statements
            .iter()
            .filter_map(|s| match s {
                Stmt::ComponentDef(d) => Some((**d).clone()),
                _ => None,
            })
            .collect();
        let mut routes: Vec<Route> = Vec::new();
        for stmt in statements {
            let Stmt::RouteDef {
                method,
                path,
                params,
                return_type,
                body,
                span,
            } = stmt
            else {
                continue;
            };
            if route_lowerable(
                params,
                *return_type,
                body,
                self.interner,
                &is_record,
                &defs,
                *span,
            )
            .is_err()
            {
                continue;
            }
            let m = self.interner.resolve(*method).to_ascii_uppercase();
            let p = self.interner.resolve(*path).to_string();
            let fname = self.interner.intern(&route_fn_name(&m, &p));
            let segments: Vec<&str> = p.split('/').collect();
            let bound: Vec<(Symbol, bool, ParamFrom)> = params
                .iter()
                .map(|(n, ty)| {
                    let named = |k: &str| {
                        matches!(ty, TypeExpr::Primitive(s) | TypeExpr::Named(s)
                        if self.interner.resolve(*s).eq_ignore_ascii_case(k))
                    };
                    let slot = format!(":{}", self.interner.resolve(*n));
                    let from = if named("Request") {
                        ParamFrom::Request
                    } else if segments.iter().any(|seg| *seg == slot) {
                        ParamFrom::Path
                    } else {
                        ParamFrom::Extra
                    };
                    (*n, named("Int"), from)
                })
                .collect();
            // A route declared `-> View` serves the markup, whether a template draws it or its
            // statements return one.
            let serves_view =
                matches!(**body, RouteBody::View(_)) || self.returns_a_view(*return_type);
            // A statements body is the function's body as written, reading a view through its
            // twin. A TEMPLATE body becomes a Text accumulator the HTML walk appends to — the same
            // tree, a different destination, because a server has no document to build.
            let (fn_body, fn_ret) = match &**body {
                // ★ THE HANDLER CARRIES THE TYPE ITS ROUTE DECLARES. This used to be hard-coded to
                // `Text`, so `-> Seq of Todo` produced a function declared `-> Text` whose body
                // answers a list — a contradiction the tree-walker and the VM infer their way past
                // and the AOT-Rust type checker, the tier a server actually SHIPS as, refuses.
                // Encoding was never the handler's job: the DISPATCHER wraps this call in the
                // derived encoder (`encode`, just below), which is what puts Text on the wire.
                RouteBody::Statements(block) => {
                    let block = self.serve_views_through_twins(block, &view_twins);
                    (
                        block,
                        if serves_view {
                            Some(text_ty)
                        } else {
                            return_type.or(Some(text_ty))
                        },
                    )
                }
                RouteBody::View(view) => {
                    let out = self.interner.intern("__lgx_html");
                    let owner = self.template_owner(fname, params.to_vec(), view);
                    let mut draw = Vec::new();
                    let mut sheets_of: Vec<Symbol> = Vec::new();
                    for child in placed_components(&view.nodes) {
                        for styled in self.styled_reachable(child, &defs) {
                            if !sheets_of.contains(&styled) {
                                sheets_of.push(styled);
                            }
                        }
                    }
                    for styled in sheets_of {
                        for sheet in self.served_sheets(styled, &defs) {
                            draw.push(sheet);
                        }
                    }
                    draw.extend(self.serving_draw(&view.nodes, &owner, &defs));
                    let mut html = Vec::new();
                    self.serve_markup(draw, out, &mut html);
                    html.push(Stmt::Return {
                        value: Some(self.te_id(out)),
                    });
                    (self.alloc_block(html), Some(text_ty))
                }
            };
            self.deferred_functions.push(Stmt::FunctionDef {
                name: fname,
                generics: Vec::new(),
                params: params.iter().map(|(n, ty)| (*n, *ty)).collect(),
                body: fn_body,
                return_type: fn_ret,
                is_native: false,
                native_path: None,
                is_exported: false,
                export_target: None,
                opt_flags: crate::optimization::OptimizationConfig::all_on(),
            });
            let encode = match (&**body, return_type) {
                // A route serving a view already answers HTML; wrapping it in a JSON encoder would
                // serve a quoted document.
                _ if serves_view => None,
                // ★ BEFORE the record arm, because `Response` IS one and would otherwise be sent
                // to an encoder that is never derived. It is the answer itself; its fields are
                // read, not serialised.
                (_, Some(TypeExpr::Primitive(sym) | TypeExpr::Named(sym)))
                    if self.interner.resolve(*sym).eq_ignore_ascii_case("Response") =>
                {
                    None
                }
                (_, Some(TypeExpr::Primitive(sym) | TypeExpr::Named(sym))) if is_record(*sym) => {
                    Some((*sym, false))
                }
                // A list of records answers as a JSON ARRAY, through the list encoder emitted
                // beside that record's own.
                (_, Some(TypeExpr::Generic { base, params }))
                    if self.interner.resolve(*base).eq_ignore_ascii_case("Seq")
                        && params.len() == 1 =>
                {
                    match &params[0] {
                        TypeExpr::Primitive(s) | TypeExpr::Named(s) if is_record(*s) => {
                            Some((*s, true))
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
            // The router synthesizes this encoder's CALL, so it never appears in the source the
            // author wrote — the derive's reachability scan cannot see it and has to be told.
            if let Some((rec, _)) = encode {
                if !self.json_encoders_needed.contains(&rec) {
                    self.json_encoders_needed.push(rec);
                }
            }
            let has_body = matches!(m.as_str(), "POST" | "PUT" | "PATCH");
            let is_response = !serves_view
                && matches!(return_type, Some(TypeExpr::Primitive(sym) | TypeExpr::Named(sym))
                    if self.interner.resolve(*sym).eq_ignore_ascii_case("Response"));
            let bare_scalar = !serves_view
                && matches!(return_type, Some(TypeExpr::Primitive(sym) | TypeExpr::Named(sym))
                    if json_bare_scalar_name(self.interner.resolve(*sym)));
            // A `Response` names its OWN content type, so the per-route constant is only a
            // fallback for the walks that never see the value; the arms below read the field.
            let content_type = if serves_view {
                "text/html; charset=utf-8"
            } else if encode.is_some() || bare_scalar {
                "application/json"
            } else {
                "text/plain; charset=utf-8"
            };
            routes.push(Route {
                method: m,
                path: *path,
                fname,
                params: bound,
                literal: !p.contains(':'),
                encode,
                has_body,
                content_type,
                bare_scalar,
                is_response,
            });
        }
        // A file may declare redirects (or middleware) and no routes at all; it still needs a
        // dispatcher, or `route(…)` would be an unknown function.
        if routes.is_empty()
            && !statements
                .iter()
                .any(|s| matches!(s, Stmt::MiddlewareDef { .. } | Stmt::RedirectDef { .. }))
        {
            return;
        }
        let method_p = self.interner.intern("__lgx_method");
        // The dispatcher is handed a URL, and a URL is a path AND a query. Splitting it here —
        // once, before anything matches — is what lets `/search?q=hi` reach the route at
        // `/search`: comparing the whole URL 404s every link a `get` form, a share and an
        // analytics tag produce, which is most of the links on a real site.
        let url_p = self.interner.intern("__lgx_url");
        let path_p = self.interner.intern("__lgx_path");
        let query_p = self.interner.intern("__lgx_query");
        // ★ THE BODY IS A THIRD INPUT, and it is ADDITIVE. `route(method, url)` stays exactly
        // what it was — every test, every `Serve fetches with routes`, every page that asks a
        // question of the table keeps working — and `routeWith(method, url, body)` is the same
        // walk with the body in hand. Changing the arity of `route` instead would have churned
        // every call site in the suite to say `""` in a place nobody was asking about.
        let body_p = self.interner.intern("__lgx_body");
        // ★ THE WALKS TAKE THE REQUEST AND READ THEIR OWN INPUTS OFF IT. Method, url and body used
        // to be three parameters, which is every field of a request EXCEPT the two a session needs
        // — so no route could read a cookie and no guard could see a header. Binding them here, in
        // the prelude all three walks share, means the request is in scope for the whole table.
        let req_p = self.interner.intern("__lgx_req");
        // ★ THE SESSION LOADS IN THE PRELUDE, so it is in scope for the whole table and happens
        // ONCE per request — the handler runs only the serving walk, and that walk's prelude is
        // this one. Doing it inside each route instead would mean a route that forgot the call
        // silently saw an empty session, which is the class of bug this campaign keeps closing.
        let has_session = self.session_declared;
        let split_url = |me: &mut Self| -> Vec<Stmt<'a>> {
            let path_of = me.interner.intern("pathOf");
            let query_string = me.interner.intern("queryString");
            let field = |me: &mut Self, f: &str| -> &'a Expr<'a> {
                let field = me.interner.intern(f);
                let object = me.te_id(req_p);
                me.ctx
                    .alloc_imperative_expr(Expr::FieldAccess { object, field })
            };
            let m = field(me, "method");
            let u = field(me, "path");
            let b = field(me, "body");
            let mut out = vec![
                Stmt::Let {
                    var: method_p,
                    ty: Some(text_ty),
                    value: m,
                    mutable: false,
                },
                Stmt::Let {
                    var: url_p,
                    ty: Some(text_ty),
                    value: u,
                    mutable: false,
                },
                Stmt::Let {
                    var: body_p,
                    ty: Some(text_ty),
                    value: b,
                    mutable: false,
                },
                Stmt::Let {
                    var: path_p,
                    ty: Some(text_ty),
                    value: me.te_call(path_of, vec![me.te_id(url_p)]),
                    mutable: false,
                },
                Stmt::Let {
                    var: query_p,
                    ty: Some(text_ty),
                    value: me.te_call(query_string, vec![me.te_id(url_p)]),
                    mutable: false,
                },
            ];
            if has_session {
                let load = me.interner.intern(SESSION_LOAD_FN);
                let req = me.te_id(req_p);
                out.push(Stmt::Call {
                    function: load,
                    args: vec![req],
                });
            }
            out
        };
        let mut arms: Vec<Stmt<'a>> = split_url(self);
        // The status companion, built arm for arm beside the body — see `routeStatus`.
        let mut status_arms: Vec<Stmt<'a>> = split_url(self);
        // And the CONTENT TYPE companion, built in the same loop for the same reason: three
        // answers about one request that must agree about which route matched.
        let mut ctype_arms: Vec<Stmt<'a>> = split_url(self);
        // And the HEADERS companion — see `ROUTE_HEADERS_WALK`.
        let mut header_arms: Vec<Stmt<'a>> = split_url(self);
        // And the SERVING walk — see `ROUTE_RESPONSE_WALK`. This is the one a real request runs.
        let mut response_arms: Vec<Stmt<'a>> = split_url(self);
        let mut next = 0usize;

        // ★ `routeStatus` MUST ANSWER THE STATUS A REAL REQUEST GETS, CSRF INCLUDED. It used to
        // skip straight to matching routes, so a state-changing route guarded by `## Session`
        // reported its ordinary success code to `routeStatus` regardless of whether the request
        // carried a valid token — the one framework promise this whole file exists to keep
        // ("the compiler checks the wire") broken for the one query whose entire job is "what
        // status code does this request get". A test written against `routeStatus` to prove a
        // route is CSRF-protected passed whether or not it was. The guard is built once and
        // pushed onto both walks, so the two can no longer read differently.
        // `as_status`: `response_arms` returns a `Response` on refusal, `status_arms` returns the
        // bare `Int` its function is declared to answer — the same gate, typed for its own walk.
        let csrf_guard = |me: &mut Self, as_status: bool| -> Vec<Stmt<'a>> {
            let refusal = |me: &mut Self| -> &'a Expr<'a> {
                if as_status {
                    me.te_int(403)
                } else {
                    let code = me.interner.intern("403");
                    let body_text = me.te_text(code);
                    me.plain_response(403, body_text)
                }
            };
            let m_upper = me.interner.intern("__lgx_method_upper");
            let upper = {
                let f = me.interner.intern("upper");
                me.te_call(f, vec![me.te_id(method_p)])
            };
            let mut out = vec![Stmt::Let {
                var: m_upper,
                ty: Some(text_ty),
                value: upper,
                mutable: false,
            }];
            let blank = me.interner.intern("");
            let secret_var = me.interner.intern(SESSION_SECRET_VAR);
            let session_var = me.interner.intern(SESSION_VAR);
            let nonce_field = me.interner.intern(SESSION_NONCE_FIELD);
            let sent = me.interner.intern("__lgx_csrf_sent");
            let mut guard: Vec<Stmt<'a>> = Vec::new();
            let from_body = {
                let f = me.interner.intern("bodyParam");
                let name = me.interner.intern(CSRF_FIELD);
                me.te_call(f, vec![me.te_id(body_p), me.te_text(name)])
            };
            guard.push(Stmt::Let {
                var: sent,
                ty: Some(text_ty),
                value: from_body,
                mutable: true,
            });
            // A scripted `fetch` has no form to put a hidden field in, so it sends a header.
            let from_header = {
                let f = me.interner.intern("headerOf");
                let name = me.interner.intern(CSRF_HEADER);
                me.te_call(f, vec![me.te_id(req_p), me.te_text(name)])
            };
            let none_sent = me.te_bin(BinaryOpKind::Eq, me.te_id(sent), me.te_text(blank));
            guard.push(Stmt::If {
                cond: none_sent,
                then_block: me.alloc_block(vec![Stmt::Set {
                    target: sent,
                    value: from_header,
                }]),
                else_block: None,
            });
            // ★ NO NONCE IS A REFUSAL, not a pass. A visitor with no session has a nonce of `""`,
            // and `csrfToken(secret, "")` is a value anybody can compute — so accepting it would
            // leave exactly the hole this is here to close.
            let read_nonce = me.ctx.alloc_imperative_expr(Expr::FieldAccess {
                object: me.te_id(session_var),
                field: nonce_field,
            });
            let unknown = me.te_bin(BinaryOpKind::Eq, read_nonce, me.te_text(blank));
            let refused = refusal(me);
            guard.push(Stmt::If {
                cond: unknown,
                then_block: me.alloc_block(vec![Stmt::Return {
                    value: Some(refused),
                }]),
                else_block: None,
            });
            let read_nonce = me.ctx.alloc_imperative_expr(Expr::FieldAccess {
                object: me.te_id(session_var),
                field: nonce_field,
            });
            let valid = {
                let f = me.interner.intern("csrfValid");
                me.te_call(f, vec![me.te_id(secret_var), read_nonce, me.te_id(sent)])
            };
            let bad = me.te_bin(BinaryOpKind::Eq, valid, me.te_bool(false));
            let refused = refusal(me);
            guard.push(Stmt::If {
                cond: bad,
                then_block: me.alloc_block(vec![Stmt::Return {
                    value: Some(refused),
                }]),
                else_block: None,
            });
            let mut changes: Option<&'a Expr<'a>> = None;
            for m in ["POST", "PUT", "PATCH", "DELETE"] {
                let sym = me.interner.intern(m);
                let is = me.te_bin(BinaryOpKind::Eq, me.te_id(m_upper), me.te_text(sym));
                changes = Some(match changes {
                    None => is,
                    Some(acc) => me.te_bin(BinaryOpKind::Or, acc, is),
                });
            }
            let changes = changes.expect("four methods");
            out.push(Stmt::If {
                cond: changes,
                then_block: me.alloc_block(guard),
                else_block: None,
            });
            out
        };

        // `## Before each request` runs FIRST, in file order, and `some(response)` short-circuits.
        //
        // Before MATCHING, not before each route: a guard that only ran on paths some route
        // claimed could not redirect an unauthenticated visitor away from a URL that does not
        // exist — and would leak which URLs DO exist by answering them differently.
        let middleware: Vec<&Stmt<'a>> = statements
            .iter()
            .filter(|s| matches!(s, Stmt::MiddlewareDef { .. }))
            .collect();
        if !middleware.is_empty() {
            // ★ A GUARD IS HANDED THE REAL REQUEST. It used to be handed one FABRICATED here from
            // the method and the path, with empty `headers` and empty `query` — so the campaign's
            // own headline guard, `currentUser(request) is none`, answered `none` for every request
            // that would ever be made, and a site "protected" by it was open. Worse, the fabricated
            // binding was pushed into `arms` ALONE while the status walk referred to it, so any
            // program with a guard emitted a `routeStatusWith` naming an undeclared variable —
            // invisible because every middleware test asks `route(…)` and none asks `routeStatus(…)`.
            // Both faults are gone: the headers and the query below are the REAL ones, read off
            // the walks' own parameter.
            //
            // ★ THE PATH IS STILL THE SPLIT PATH, and that is a deliberate carry-over rather than
            // an oversight — `a_middleware_guard_sees_the_path_without_the_query` pins it, and it
            // is right: a guard comparing `request's path` to `"/admin"` must not be defeated by
            // appending `?x=1`. So the guard is handed the request it should have had all along —
            // every field real, with the one field the router has already split kept split.
            let req = self.interner.intern("__lgx_guard_request");
            let derived = {
                let read = |me: &mut Self, f: &str| -> &'a Expr<'a> {
                    let field = me.interner.intern(f);
                    let object = me.te_id(req_p);
                    me.ctx
                        .alloc_imperative_expr(Expr::FieldAccess { object, field })
                };
                let headers = read(self, "headers");
                let query = read(self, "query");
                self.ctx.alloc_imperative_expr(Expr::New {
                    type_name: self.interner.intern("Request"),
                    type_args: Vec::new(),
                    init_fields: vec![
                        (self.interner.intern("method"), self.te_id(method_p)),
                        (self.interner.intern("path"), self.te_id(path_p)),
                        (self.interner.intern("body"), self.te_id(body_p)),
                        (self.interner.intern("headers"), headers),
                        (self.interner.intern("query"), query),
                    ],
                })
            };
            // ★ BOUND INTO ALL THREE WALKS. The old binding went into `arms` alone while the
            // status walk referred to it, which is exactly the bug described above; adding a
            // fourth walk without adding it here would reopen it in the headers walk.
            for dest in [
                &mut arms,
                &mut status_arms,
                &mut header_arms,
                &mut response_arms,
            ] {
                dest.push(Stmt::Let {
                    var: req,
                    ty: None,
                    value: derived,
                    mutable: false,
                });
            }
            for (i, m) in middleware.iter().enumerate() {
                let Stmt::MiddlewareDef { param, body } = m else {
                    continue;
                };
                let fname = self.interner.intern(&format!("__middleware_{i}"));
                let body = self.serve_views_through_twins(body, &view_twins);
                // A guard answers `Option of Response` whatever its header spells: the early answer, or none.
                let answers = {
                    let response = TypeExpr::Named(self.interner.intern("Response"));
                    let params = self.ctx.alloc_type_exprs(vec![response]);
                    self.ctx.alloc_type_expr(TypeExpr::Generic {
                        base: self.interner.intern("Option"),
                        params,
                    })
                };
                self.deferred_functions.push(Stmt::FunctionDef {
                    name: fname,
                    generics: Vec::new(),
                    params: vec![*param],
                    body,
                    return_type: Some(answers),
                    is_native: false,
                    native_path: None,
                    is_exported: false,
                    export_target: None,
                    opt_flags: crate::optimization::OptimizationConfig::all_on(),
                });
                let answered = self.interner.intern("__lgx_answer");
                let body_field = self.interner.intern("body");
                let taken = self.ctx.alloc_imperative_expr(Expr::FieldAccess {
                    object: self.te_id(answered),
                    field: body_field,
                });
                let some_arm = crate::ast::stmt::MatchArm {
                    enum_name: None,
                    variant: Some(self.interner.intern("Some")),
                    bindings: vec![(answered, answered)],
                    body: self.alloc_block(vec![Stmt::Return { value: Some(taken) }]),
                };
                // A `None` arm is REQUIRED even though it does nothing: an `Inspect` whose value
                // matches no arm is a runtime error, so leaving it out would turn "this guard
                // declined" into a crash.
                let none_arm = crate::ast::stmt::MatchArm {
                    enum_name: None,
                    variant: Some(self.interner.intern("None")),
                    bindings: Vec::new(),
                    body: self.alloc_block(Vec::new()),
                };
                arms.push(Stmt::Inspect {
                    target: self.te_call(fname, vec![self.te_id(req)]),
                    arms: vec![some_arm, none_arm],
                    has_otherwise: false,
                });
                // A guard that ANSWERS carries its own status — `unauthorized()` is a 401, and
                // reporting 200 for it would tell the page the request succeeded.
                let status_field = self.interner.intern("status");
                let its_status = self.ctx.alloc_imperative_expr(Expr::FieldAccess {
                    object: self.te_id(answered),
                    field: status_field,
                });
                let some_status = crate::ast::stmt::MatchArm {
                    enum_name: None,
                    variant: Some(self.interner.intern("Some")),
                    bindings: vec![(answered, answered)],
                    body: self.alloc_block(vec![Stmt::Return {
                        value: Some(its_status),
                    }]),
                };
                let none_status = crate::ast::stmt::MatchArm {
                    enum_name: None,
                    variant: Some(self.interner.intern("None")),
                    bindings: Vec::new(),
                    body: self.alloc_block(Vec::new()),
                };
                status_arms.push(Stmt::Inspect {
                    target: self.te_call(fname, vec![self.te_id(req)]),
                    arms: vec![some_status, none_status],
                    has_otherwise: false,
                });
                // ★ AND ITS HEADERS. A guard that answers `redirectTo("/login")` carries the
                // `Location` in a header, so dropping them here would send every unauthenticated
                // visitor a 302 to nowhere — the browser stays exactly where it was and the site
                // looks broken rather than protected.
                let headers_field = self.interner.intern("headers");
                let its_headers = self.ctx.alloc_imperative_expr(Expr::FieldAccess {
                    object: self.te_id(answered),
                    field: headers_field,
                });
                let some_headers = crate::ast::stmt::MatchArm {
                    enum_name: None,
                    variant: Some(self.interner.intern("Some")),
                    bindings: vec![(answered, answered)],
                    body: self.alloc_block(vec![Stmt::Return {
                        value: Some(its_headers),
                    }]),
                };
                let none_headers = crate::ast::stmt::MatchArm {
                    enum_name: None,
                    variant: Some(self.interner.intern("None")),
                    bindings: Vec::new(),
                    body: self.alloc_block(Vec::new()),
                };
                header_arms.push(Stmt::Inspect {
                    target: self.te_call(fname, vec![self.te_id(req)]),
                    arms: vec![some_headers, none_headers],
                    has_otherwise: false,
                });
                // ★ AND THE SERVING WALK TAKES THE ANSWER WHOLE — one call, every field. The three
                // walks above each ask this guard the same question again to read one field off the
                // same answer, which is fine for a test asking `routeStatus(…)` and wrong for a
                // real request: a guard that counts attempts or writes a log line would run four
                // times for one visit. A real request runs THIS arm and only this one.
                let some_whole = crate::ast::stmt::MatchArm {
                    enum_name: None,
                    variant: Some(self.interner.intern("Some")),
                    bindings: vec![(answered, answered)],
                    body: self.alloc_block(vec![Stmt::Return {
                        value: Some(self.te_id(answered)),
                    }]),
                };
                let none_whole = crate::ast::stmt::MatchArm {
                    enum_name: None,
                    variant: Some(self.interner.intern("None")),
                    bindings: Vec::new(),
                    body: self.alloc_block(Vec::new()),
                };
                response_arms.push(Stmt::Inspect {
                    target: self.te_call(fname, vec![self.te_id(req)]),
                    arms: vec![some_whole, none_whole],
                    has_otherwise: false,
                });
            }
        }

        // ★ CSRF IS CHECKED BEFORE ANY ROUTE MATCHES, in the SERVING walk AND in `routeStatus`.
        //
        // `csrf_lock` already makes forgetting the token a COMPILE ERROR — a state-changing
        // `<form>` must carry a `_csrf` field or the page does not build. But its own refusal ends
        // *"and check it with `csrfValid` in the route that answers"*, so the framework guaranteed
        // the token was SENT and left checking it to the author, in every route, forever. That is
        // the same checklist item one layer down, and the one route that forgets is the one that
        // gets exploited.
        //
        // ★ `routeStatus` GETS IT TOO, because it is the one question-walk whose entire job is
        // "what status code does this request get" — and a request denied for a missing or
        // invalid token gets 403 whether the answer comes from a browser or from `routeStatus` in
        // a `## Test`. `route`, `routeContentType` and `routeHeader` stay as they were: each asks
        // a structural question about the URL a route answers, not about one incoming request.
        if has_session {
            status_arms.extend(csrf_guard(self, true));
            response_arms.extend(csrf_guard(self, false));
        }

        // ★ THE VISITOR'S TOKEN, AT THE ROUTER'S OWN PATH. A page's scripted request carries the
        // token in `x-csrf-token`, read from its document's `<meta name="csrf-token">`; whatever
        // composes that document asks here. Answering is `csrf()` — the visit's nonce is minted and
        // the cookie set on this very response, as for a form — and the answer is never stored by
        // a cache, which would hand one visitor's token to the next. Pushed into every walk, so
        // `route`, `routeStatus`, `routeContentType` and `routeHeaders` agree with what is served.
        if has_session {
            let get = self.interner.intern("GET");
            let reserved = self.interner.intern(CSRF_TOKEN_PATH);
            let plain = self.interner.intern("text/plain; charset=utf-8");
            let is_get = self.te_bin(BinaryOpKind::Eq, self.te_id(method_p), self.te_text(get));
            let at_path = self.te_bin(BinaryOpKind::Eq, self.te_id(path_p), self.te_text(reserved));
            let cond = self.te_bin(BinaryOpKind::And, is_get, at_path);
            let csrf_fn = self.interner.intern(CSRF_FN);
            let no_store = |me: &mut Self, headers: Symbol| -> Stmt<'a> {
                let name = me.interner.intern("cache-control");
                let value = me.interner.intern("no-store");
                Stmt::SetIndex {
                    collection: me.te_id(headers),
                    index: me.te_text(name),
                    value: me.te_text(value),
                }
            };
            let token = self.te_call(csrf_fn, Vec::new());
            let body_then = self.alloc_block(vec![Stmt::Return { value: Some(token) }]);
            arms.push(Stmt::If {
                cond,
                then_block: body_then,
                else_block: None,
            });
            let status_then = self.alloc_block(vec![Stmt::Return {
                value: Some(self.te_int(200)),
            }]);
            status_arms.push(Stmt::If {
                cond,
                then_block: status_then,
                else_block: None,
            });
            let ct_then = self.alloc_block(vec![Stmt::Return {
                value: Some(self.te_text(plain)),
            }]);
            ctype_arms.push(Stmt::If {
                cond,
                then_block: ct_then,
                else_block: None,
            });
            let hmap = self.interner.intern("__lgx_csrf_headers");
            let fresh = self.empty_header_map();
            let mark = no_store(self, hmap);
            let back = Stmt::Return {
                value: Some(self.te_id(hmap)),
            };
            let header_then = self.alloc_block(vec![
                Stmt::Let {
                    var: hmap,
                    ty: None,
                    value: fresh,
                    mutable: true,
                },
                mark,
                back,
            ]);
            header_arms.push(Stmt::If {
                cond,
                then_block: header_then,
                else_block: None,
            });
            let answer = self.interner.intern("__lgx_csrf_answer");
            let token = self.te_call(csrf_fn, Vec::new());
            let served = self.response_value(self.te_int(200), self.te_text(plain), token);
            let served_headers = self.ctx.alloc_imperative_expr(Expr::FieldAccess {
                object: self.te_id(answer),
                field: self.interner.intern("headers"),
            });
            let cache = self.interner.intern("cache-control");
            let store = self.interner.intern("no-store");
            let mark = Stmt::SetIndex {
                collection: served_headers,
                index: self.te_text(cache),
                value: self.te_text(store),
            };
            let back = Stmt::Return {
                value: Some(self.te_id(answer)),
            };
            let response_then = self.alloc_block(vec![
                Stmt::Let {
                    var: answer,
                    ty: None,
                    value: served,
                    mutable: true,
                },
                mark,
                back,
            ]);
            response_arms.push(Stmt::If {
                cond,
                then_block: response_then,
                else_block: None,
            });
        }

        // (1) and (2): literals before patterns, so specificity beats file order.
        for pass_literal in [true, false] {
            for r in routes.iter().filter(|r| r.literal == pass_literal) {
                let m_sym = self.interner.intern(&r.method);
                let same_method =
                    self.te_bin(BinaryOpKind::Eq, self.te_id(method_p), self.te_text(m_sym));
                let matches_path = if r.literal {
                    self.te_bin(BinaryOpKind::Eq, self.te_id(path_p), self.te_text(r.path))
                } else {
                    let f = self.interner.intern("matchRoute");
                    self.te_call(f, vec![self.te_text(r.path), self.te_id(path_p)])
                };
                let cond = self.te_bin(BinaryOpKind::And, same_method, matches_path);

                // ★ THE SERVING ARM — see `ROUTE_RESPONSE_WALK`. It rebuilds its own parameter
                // bindings because the body arm's parsed locals are block-scoped, and it calls the
                // route's function EXACTLY ONCE: a `Response` route hands its answer straight back,
                // and every other route has its answer wrapped in the status and content type the
                // companion walks report as constants. Built here, from `cond`, so it can never
                // disagree with them about which route matched.
                {
                    let mut rbody: Vec<Stmt<'a>> = Vec::new();
                    let mut rargs: Vec<&'a Expr<'a>> = Vec::new();
                    for (name, is_int, from) in &r.params {
                        let body_src = r.has_body.then_some(body_p);
                        let raw = self.route_param_value(
                            *from, r.path, *name, path_p, query_p, body_src, req_p,
                        );
                        if *is_int {
                            let is_int_fn = self.interner.intern("isIntText");
                            let ok = self.te_call(is_int_fn, vec![raw]);
                            let bad = self.te_bin(BinaryOpKind::Eq, ok, self.te_bool(false));
                            let four = self.interner.intern("400");
                            let text = self.te_text(four);
                            let refused = self.plain_response(400, text);
                            let then = self.alloc_block(vec![Stmt::Return {
                                value: Some(refused),
                            }]);
                            rbody.push(Stmt::If {
                                cond: bad,
                                then_block: then,
                                else_block: None,
                            });
                            let parse = self.interner.intern("parseInt");
                            rargs.push(self.te_call(parse, vec![raw]));
                        } else {
                            rargs.push(raw);
                        }
                    }
                    let rcall = self.te_call(r.fname, rargs);
                    let served = if r.is_response {
                        rcall
                    } else {
                        let ranswer = match r.encode {
                            Some((record, as_list)) => {
                                let prefix = if as_list { "toJsonSeq_" } else { "toJson_" };
                                let enc = self
                                    .interner
                                    .intern(&format!("{prefix}{}", self.interner.resolve(record)));
                                self.te_call(enc, vec![rcall])
                            }
                            None if r.bare_scalar => {
                                let empty = self.interner.intern("");
                                self.te_bin(BinaryOpKind::Add, self.te_text(empty), rcall)
                            }
                            None => rcall,
                        };
                        let ct = self.interner.intern(r.content_type);
                        let ctype = self.te_text(ct);
                        let ok = self.te_int(200);
                        self.response_value(ok, ctype, ranswer)
                    };
                    rbody.push(Stmt::Return {
                        value: Some(served),
                    });
                    let rthen = self.alloc_block(rbody);
                    response_arms.push(Stmt::If {
                        cond,
                        then_block: rthen,
                        else_block: None,
                    });
                }

                let mut body: Vec<Stmt<'a>> = Vec::new();
                let mut args: Vec<&'a Expr<'a>> = Vec::new();
                for (name, is_int, from) in &r.params {
                    let raw = self.lgx_temp(&mut next);
                    let body_src = r.has_body.then_some(body_p);
                    let value = self
                        .route_param_value(*from, r.path, *name, path_p, query_p, body_src, req_p);
                    // The binding's type must match what `route_param_value` actually hands back:
                    // the raw TEXT of a path/query/body segment for every other origin, but the
                    // whole `Request` itself, unconverted, when the parameter asks for it.
                    let param_ty = if *from == ParamFrom::Request {
                        self.ctx
                            .alloc_type_expr(TypeExpr::Named(self.interner.intern("Request")))
                    } else {
                        text_ty
                    };
                    body.push(Stmt::Let {
                        var: raw,
                        ty: Some(param_ty),
                        value,
                        mutable: false,
                    });
                    if *is_int {
                        // A path segment that is not the declared type is a BAD REQUEST, not a
                        // 404 and emphatically not a silent zero: `parseInt("abc")` is 0, and
                        // serving todo 0 to someone who asked for "abc" is a wrong answer dressed
                        // as a right one. The round trip is the check — a value parses iff
                        // printing it back gives the same text.
                        // `parseInt` is strict and TRAPS on text that is not a number — right
                        // for a program reading its own data, wrong for a server reading a
                        // stranger's URL, where the answer must be 400 rather than a dead
                        // process. `isIntText` (web.lg) asks first.
                        let parse = self.interner.intern("parseInt");
                        let is_int = self.interner.intern("isIntText");
                        let ok = self.te_call(is_int, vec![self.te_id(raw)]);
                        let bad = self.te_bin(BinaryOpKind::Eq, ok, self.te_bool(false));
                        let four = self.interner.intern("400");
                        let then_block = self.alloc_block(vec![Stmt::Return {
                            value: Some(self.te_text(four)),
                        }]);
                        body.push(Stmt::If {
                            cond: bad,
                            then_block,
                            else_block: None,
                        });
                        let conv = self.lgx_temp(&mut next);
                        body.push(Stmt::Let {
                            var: conv,
                            ty: Some(int_ty),
                            value: self.te_call(parse, vec![self.te_id(raw)]),
                            mutable: false,
                        });
                        args.push(self.te_id(conv));
                    } else {
                        args.push(self.te_id(raw));
                    }
                }
                let call = self.te_call(r.fname, args);
                // A `Response` route answers its own body; the status, content type and headers
                // are read from the same value by the companion walks below.
                if r.is_response {
                    // Each walk reads ONE FIELD of the same answer. They are built here, in the
                    // one loop that decides which route matched, so a `Response` route cannot be
                    // added to the body and forgotten in the status — the discipline the three
                    // walks already follow.
                    //
                    // ★ AND `__lgx_route_handler` CALLS NONE OF THESE FOR SUCH A ROUTE — it runs
                    // the serving arm above, once. These three exist for the conveniences a test
                    // calls directly (`routeStatus(…)`, `routeContentType(…)`, `routeHeader(…)`),
                    // where calling the route again to read one more field of its answer is the
                    // point rather than a cost. See `ROUTE_RESPONSE_WALK` for why the handler must
                    // not be built out of them.
                    let field_of = |me: &mut Self, call: &'a Expr<'a>, f: &str| {
                        let field = me.interner.intern(f);
                        me.ctx.alloc_imperative_expr(Expr::FieldAccess {
                            object: call,
                            field,
                        })
                    };
                    let taken = field_of(self, call, "body");
                    body.push(Stmt::Return { value: Some(taken) });
                    let then_block = self.alloc_block(body);
                    arms.push(Stmt::If {
                        cond,
                        then_block,
                        else_block: None,
                    });

                    // The companions rebuild their own arguments: the body arm's parsed `Int`
                    // locals are block-scoped and cannot be reached from here.
                    for (field, dest) in [("status", 0u8), ("contentType", 1u8), ("headers", 2u8)] {
                        let mut cbody: Vec<Stmt<'a>> = Vec::new();
                        let mut cargs: Vec<&'a Expr<'a>> = Vec::new();
                        for (name, is_int, from) in &r.params {
                            let body_src = r.has_body.then_some(body_p);
                            let raw = self.route_param_value(
                                *from, r.path, *name, path_p, query_p, body_src, req_p,
                            );
                            if *is_int {
                                let is_int_fn = self.interner.intern("isIntText");
                                let ok = self.te_call(is_int_fn, vec![raw]);
                                let bad = self.te_bin(BinaryOpKind::Eq, ok, self.te_bool(false));
                                let four: &'a Expr<'a> = match dest {
                                    0 => self.te_int(400),
                                    1 => {
                                        let plain =
                                            self.interner.intern("text/plain; charset=utf-8");
                                        self.te_text(plain)
                                    }
                                    _ => self.empty_header_map(),
                                };
                                let then =
                                    self.alloc_block(vec![Stmt::Return { value: Some(four) }]);
                                cbody.push(Stmt::If {
                                    cond: bad,
                                    then_block: then,
                                    else_block: None,
                                });
                                let parse = self.interner.intern("parseInt");
                                cargs.push(self.te_call(parse, vec![raw]));
                            } else {
                                cargs.push(raw);
                            }
                        }
                        let ccall = self.te_call(r.fname, cargs);
                        let read = field_of(self, ccall, field);
                        cbody.push(Stmt::Return { value: Some(read) });
                        let cthen = self.alloc_block(cbody);
                        let arm = Stmt::If {
                            cond,
                            then_block: cthen,
                            else_block: None,
                        };
                        match dest {
                            0 => status_arms.push(arm),
                            1 => ctype_arms.push(arm),
                            _ => header_arms.push(arm),
                        }
                    }
                    continue;
                }
                let answer = match r.encode {
                    Some((record, as_list)) => {
                        let prefix = if as_list { "toJsonSeq_" } else { "toJson_" };
                        let enc = self
                            .interner
                            .intern(&format!("{prefix}{}", self.interner.resolve(record)));
                        self.te_call(enc, vec![call])
                    }
                    // A bare scalar has no encoder — its LOGOS display already IS its JSON — but
                    // the dispatcher answers Text, so it still has to be shown.
                    None if r.bare_scalar => {
                        let empty = self.interner.intern("");
                        self.te_bin(BinaryOpKind::Add, self.te_text(empty), call)
                    }
                    None => call,
                };
                body.push(Stmt::Return {
                    value: Some(answer),
                });
                let then_block = self.alloc_block(body);
                arms.push(Stmt::If {
                    cond,
                    then_block,
                    else_block: None,
                });
                // The SAME arm, answering the code. Built here, in the one walk that decides
                // which route matches, so a route can never be added to the body and forgotten
                // in the status. The 400 check is repeated rather than shared because the status
                // needs no parsed value — only the question of whether the segment IS one.
                let mut status_body: Vec<Stmt<'a>> = Vec::new();
                for (name, is_int, from) in &r.params {
                    if !*is_int {
                        continue;
                    }
                    let body_src = r.has_body.then_some(body_p);
                    let raw = self
                        .route_param_value(*from, r.path, *name, path_p, query_p, body_src, req_p);
                    let is_int_fn = self.interner.intern("isIntText");
                    let ok = self.te_call(is_int_fn, vec![raw]);
                    let bad = self.te_bin(BinaryOpKind::Eq, ok, self.te_bool(false));
                    let four = self.te_int(400);
                    let then = self.alloc_block(vec![Stmt::Return { value: Some(four) }]);
                    status_body.push(Stmt::If {
                        cond: bad,
                        then_block: then,
                        else_block: None,
                    });
                }
                status_body.push(Stmt::Return {
                    value: Some(self.te_int(200)),
                });
                let status_then = self.alloc_block(status_body);
                status_arms.push(Stmt::If {
                    cond,
                    then_block: status_then,
                    else_block: None,
                });
                let ct = self.interner.intern(r.content_type);
                let ct_then = self.alloc_block(vec![Stmt::Return {
                    value: Some(self.te_text(ct)),
                }]);
                ctype_arms.push(Stmt::If {
                    cond,
                    then_block: ct_then,
                    else_block: None,
                });
            }
        }

        // A `## Redirect /old -> /new.` answers ANY method: a moved URL has moved for everyone,
        // and answering 405 would tell the caller the path exists under some other method, which
        // is not what happened. It sits AFTER the real routes, so adding a redirect can never
        // silently take away a page some route already serves.
        for stmt in statements {
            let Stmt::RedirectDef { from, to } = stmt else {
                continue;
            };
            let same = self.te_bin(BinaryOpKind::Eq, self.te_id(path_p), self.te_text(*from));
            let answer = self
                .interner
                .intern(&format!("302 {}", self.interner.resolve(*to)));
            let then_block = self.alloc_block(vec![Stmt::Return {
                value: Some(self.te_text(answer)),
            }]);
            arms.push(Stmt::If {
                cond: same,
                then_block,
                else_block: None,
            });
            let moved = self.alloc_block(vec![Stmt::Return {
                value: Some(self.te_int(302)),
            }]);
            status_arms.push(Stmt::If {
                cond: same,
                then_block: moved,
                else_block: None,
            });
            // The BODY stays `"302 /new"` — `route("GET", "/old")` has always answered that string
            // and tests read it — and the handler turns it into a real `Location` header, which it
            // can tell apart from a route that redirected itself because that one arrives with the
            // header already set.
            let moved_body = self.te_text(answer);
            let moved_res = self.plain_response(302, moved_body);
            let rthen = self.alloc_block(vec![Stmt::Return {
                value: Some(moved_res),
            }]);
            response_arms.push(Stmt::If {
                cond: same,
                then_block: rthen,
                else_block: None,
            });
        }

        // A `## Error page STATUS` renders in place of the bare status. Collected by STATUS, so
        // declaring only one leaves the other bare rather than silently borrowing the page that
        // was declared: "this URL does not exist" and "this URL does not do that" are different
        // mistakes and deserve different pages.
        let mut error_pages: Vec<(u16, Symbol)> = Vec::new();
        for stmt in statements {
            let Stmt::ComponentDef(def) = stmt else {
                continue;
            };
            let ComponentKind::ErrorPage { status } = def.kind else {
                continue;
            };
            let Some(view) = &def.view else { continue };
            if view_html_lowerable(&view.nodes, self.interner).is_err() {
                continue;
            }
            let fname = self.interner.intern(&format!("__error_page_{status}"));
            let out = self.interner.intern("__lgx_html");
            let mut html: Vec<Stmt<'a>> = Vec::new();
            // An error page's `State:` fields are ordinary locals, exactly as a component's are
            // when it renders.
            for field in def.state.iter().filter(|f| !f.derived) {
                let bound = self.state_field_let(field, field.init, true);
                html.push(bound);
            }
            let owner = self.template_owner(fname, Vec::new(), view);
            let draw = self.serving_draw(&view.nodes, &owner, &defs);
            self.serve_markup(draw, out, &mut html);
            html.push(Stmt::Return {
                value: Some(self.te_id(out)),
            });
            self.deferred_functions.push(Stmt::FunctionDef {
                name: fname,
                generics: Vec::new(),
                params: Vec::new(),
                body: self.alloc_block(html),
                return_type: Some(text_ty),
                is_native: false,
                native_path: None,
                is_exported: false,
                export_target: None,
                opt_flags: crate::optimization::OptimizationConfig::all_on(),
            });
            error_pages.push((status, fname));
        }
        // ★ AN ERROR PAGE IS A PAGE, AND ITS CONTENT TYPE HAS TO SAY SO. The router's own answers
        // — a bare status, a redirect — are plain text because that is what they are; a declared
        // `## Error page` renders MARKUP, and served as `text/plain` a browser shows a visitor the
        // literal characters `<h1>Gone</h1>`. Whether a page was declared for a status is a fact
        // this lowering already has, collected just above, and never asked.
        let has_page = |code: u16| error_pages.iter().any(|(s, _)| *s == code);
        let answer_ctype = |me: &mut Self, code: u16| -> &'a Expr<'a> {
            let ct = if has_page(code) {
                me.interner.intern("text/html; charset=utf-8")
            } else {
                me.interner.intern("text/plain; charset=utf-8")
            };
            me.te_text(ct)
        };
        let status_answer = |me: &mut Self, code: u16| -> &'a Expr<'a> {
            match error_pages.iter().find(|(s, _)| *s == code) {
                Some((_, f)) => me.te_call(*f, Vec::new()),
                None => {
                    let bare = me.interner.intern(&code.to_string());
                    me.te_text(bare)
                }
            }
        };

        // (3) A path that EXISTS but not under this method is 405.
        let mut seen: Vec<Symbol> = Vec::new();
        for r in &routes {
            if seen.contains(&r.path) {
                continue;
            }
            seen.push(r.path);
            let cond = if r.literal {
                self.te_bin(BinaryOpKind::Eq, self.te_id(path_p), self.te_text(r.path))
            } else {
                let f = self.interner.intern("matchRoute");
                self.te_call(f, vec![self.te_text(r.path), self.te_id(path_p)])
            };
            let answer = status_answer(self, 405);
            let then_block = self.alloc_block(vec![Stmt::Return {
                value: Some(answer),
            }]);
            arms.push(Stmt::If {
                cond,
                then_block,
                else_block: None,
            });
            let wrong = self.alloc_block(vec![Stmt::Return {
                value: Some(self.te_int(405)),
            }]);
            status_arms.push(Stmt::If {
                cond,
                then_block: wrong,
                else_block: None,
            });
            let served = status_answer(self, 405);
            let ct = answer_ctype(self, 405);
            let code = self.te_int(405);
            let refused = self.response_value(code, ct, served);
            let rthen = self.alloc_block(vec![Stmt::Return {
                value: Some(refused),
            }]);
            response_arms.push(Stmt::If {
                cond,
                then_block: rthen,
                else_block: None,
            });
            // ★ ALWAYS, NOT ONLY WHEN A 405 PAGE EXISTS. The content-type walk's fall-through is
            // the 404's answer, so a 405 with no arm would inherit whatever the 404 says — declare
            // a `## Error page 404` and every 405 on the site starts claiming to be HTML. The two
            // statuses are separate declarations, so each answers for itself.
            let ct = answer_ctype(self, 405);
            let ct_then = self.alloc_block(vec![Stmt::Return { value: Some(ct) }]);
            ctype_arms.push(Stmt::If {
                cond,
                then_block: ct_then,
                else_block: None,
            });
        }

        // (4) Nothing matched.
        let not_found = status_answer(self, 404);
        arms.push(Stmt::Return {
            value: Some(not_found),
        });
        status_arms.push(Stmt::Return {
            value: Some(self.te_int(404)),
        });
        let missing = status_answer(self, 404);
        let ct = answer_ctype(self, 404);
        let code = self.te_int(404);
        let missing = self.response_value(code, ct, missing);
        response_arms.push(Stmt::Return {
            value: Some(missing),
        });
        // `routeStatus(method, path)` — the CODE the router answers with, beside the body.
        //
        // `route(…)` answers Text, so its status and its body were one string: a page fetching a
        // URL nothing serves heard `"404"` as a BODY and could only find out by failing to decode
        // it. Both are produced by the ONE walk above, arm for arm, so they cannot disagree about
        // which route matched.
        // Everything the router answers ITSELF — 404, 405, a redirect, a middleware refusal —
        // is plain text, because that is what those answers are.
        // The fall-through is the 404, so it answers what a 404 answers.
        let tail_ct = answer_ctype(self, 404);
        ctype_arms.push(Stmt::Return {
            value: Some(tail_ct),
        });
        let no_headers = self.empty_header_map();
        header_arms.push(Stmt::Return {
            value: Some(no_headers),
        });
        let request_walk_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Named(self.interner.intern("Request")));
        let ctype_name = self.interner.intern(ROUTE_CTYPE_WALK);
        self.deferred_functions.push(Stmt::FunctionDef {
            name: ctype_name,
            generics: Vec::new(),
            params: vec![(req_p, request_walk_ty)],
            body: self.alloc_block(ctype_arms),
            return_type: Some(text_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        });
        let response_named: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Named(self.interner.intern("Response")));
        let response_walk = self.interner.intern(ROUTE_RESPONSE_WALK);
        self.deferred_functions.push(Stmt::FunctionDef {
            name: response_walk,
            generics: Vec::new(),
            params: vec![(req_p, request_walk_ty)],
            body: self.alloc_block(response_arms),
            return_type: Some(response_named),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        });
        let headers_ty = self.header_map_type();
        let headers_name = self.interner.intern(ROUTE_HEADERS_WALK);
        self.deferred_functions.push(Stmt::FunctionDef {
            name: headers_name,
            generics: Vec::new(),
            params: vec![(req_p, request_walk_ty)],
            body: self.alloc_block(header_arms),
            return_type: Some(headers_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        });
        let status_name = self.interner.intern(ROUTE_STATUS_WALK);
        self.deferred_functions.push(Stmt::FunctionDef {
            name: status_name,
            generics: Vec::new(),
            params: vec![(req_p, request_walk_ty)],
            body: self.alloc_block(status_arms),
            return_type: Some(int_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        });
        // The two-argument conveniences: the same questions, asked without a body. `route(m, u)`
        // is what a test, a page's `Serve fetches with routes`, and anything else that just wants
        // to know what a URL answers has always called, and it keeps meaning exactly that.
        for (short, long) in [
            ("route", "routeWith"),
            ("routeStatus", "routeStatusWith"),
            ("routeContentType", "routeContentTypeWith"),
        ] {
            let short_sym = self.interner.intern(short);
            let long_sym = self.interner.intern(long);
            let blank = self.interner.intern("");
            let ret: &'a TypeExpr<'a> = if short == "routeStatus" {
                int_ty
            } else {
                text_ty
            };
            let call = self.te_call(
                long_sym,
                vec![self.te_id(method_p), self.te_id(url_p), self.te_text(blank)],
            );
            self.deferred_functions.push(Stmt::FunctionDef {
                name: short_sym,
                generics: Vec::new(),
                params: vec![(method_p, text_ty), (url_p, text_ty)],
                body: self.alloc_block(vec![Stmt::Return { value: Some(call) }]),
                return_type: Some(ret),
                is_native: false,
                native_path: None,
                is_exported: false,
                export_target: None,
                opt_flags: crate::optimization::OptimizationConfig::all_on(),
            });
        }

        // ★ `Serve on port 8080.` — the whole server, with no handler to write.
        //
        // A `## Route` table already says everything a request needs: which function answers,
        // what status it answers with, and what the answer IS. Making the author then hand-write
        // a `Request -> Response` that calls `route(…)` is ceremony that can only be got wrong —
        // and got wrong QUIETLY, by forgetting the content type or the redirect's `Location`, so
        // a browser shows JSON as text or follows a redirect to nowhere.
        //
        // Built from the same three walks, so it cannot disagree with them about anything.
        let handler_name = self.interner.intern(ROUTE_HANDLER);
        let request_p = self.interner.intern("__lgx_request_in");
        let s_var = self.interner.intern("__lgx_hs");
        let b_var = self.interner.intern("__lgx_hb");
        let c_var = self.interner.intern("__lgx_hc");
        let mut hbody: Vec<Stmt<'a>> = Vec::new();
        // ★ THE HANDLER PASSES THE REQUEST STRAIGHT THROUGH. It used to pull the method, the path
        // and the body out and ask the three-argument form — which is every field of a request
        // except the two that carry a cookie and a query, so the handler was the exact place the
        // headers were being dropped on a real server.
        // ★ ONE CALL. The four question-walks are NOT composed here — see `ROUTE_RESPONSE_WALK`:
        // doing that would run a `Response` route four times and its guards four times for a
        // single visit. The handler asks the serving walk once and reads the answer it got.
        let h_var = self.interner.intern("__lgx_hh");
        let served_var = self.interner.intern("__lgx_hres");
        let response_named: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Named(self.interner.intern("Response")));
        let serve_call = {
            let f = self.interner.intern(ROUTE_RESPONSE_WALK);
            self.te_call(f, vec![self.te_id(request_p)])
        };
        hbody.push(Stmt::Let {
            var: served_var,
            ty: Some(response_named),
            value: serve_call,
            mutable: false,
        });
        let field_of = |me: &mut Self, f: &str| -> &'a Expr<'a> {
            let field = me.interner.intern(f);
            let object = me.te_id(served_var);
            me.ctx
                .alloc_imperative_expr(Expr::FieldAccess { object, field })
        };
        let read_status = field_of(self, "status");
        let read_body = field_of(self, "body");
        let read_ctype = field_of(self, "contentType");
        let read_headers = field_of(self, "headers");
        let headers_ty = self.header_map_type();
        hbody.push(Stmt::Let {
            var: s_var,
            ty: Some(int_ty),
            value: read_status,
            mutable: false,
        });
        hbody.push(Stmt::Let {
            var: b_var,
            ty: Some(text_ty),
            value: read_body,
            mutable: false,
        });
        hbody.push(Stmt::Let {
            var: c_var,
            ty: Some(text_ty),
            value: read_ctype,
            mutable: false,
        });
        hbody.push(Stmt::Let {
            var: h_var,
            ty: Some(headers_ty),
            value: read_headers,
            mutable: false,
        });
        let loc_var = self.interner.intern("__lgx_hloc");
        let header_in = self.interner.intern("headerIn");
        let location = self.interner.intern("location");
        let read_loc = self.te_call(header_in, vec![self.te_id(h_var), self.te_text(location)]);
        hbody.push(Stmt::Let {
            var: loc_var,
            ty: Some(text_ty),
            value: read_loc,
            mutable: false,
        });
        // A redirect answers `"<status> <where>"`, and a browser told to move without being told
        // WHERE simply stays put. So the moved answers are rebuilt through `redirectTo`, which
        // sets the `Location` header the status is meaningless without.
        // ★ THE SESSION IS SAVED ON THE WAY OUT, at every exit the handler has. Wrapping only the
        // ordinary one would mean a login that answers `redirectTo("/")` — which is exactly what a
        // login answers, POST/redirect/GET being the whole point — set no cookie and logged nobody
        // in. The save itself is a no-op when nothing changed, so wrapping every exit costs a
        // string comparison and cannot be got wrong by forgetting one.
        let saved = |me: &mut Self, value: &'a Expr<'a>| -> &'a Expr<'a> {
            if !has_session {
                return value;
            }
            let f = me.interner.intern(SESSION_SAVE_FN);
            me.te_call(f, vec![value])
        };
        let redirect_to = self.interner.intern("redirectTo");
        let permanent_to = self.interner.intern("permanentRedirectTo");
        let cut_after = self.interner.intern("cutAfter");
        let space = self.interner.intern(" ");
        let where_to = self.te_call(cut_after, vec![self.te_id(b_var), self.te_text(space)]);
        // ★ ONLY WHEN THE ANSWER CARRIES NO `Location` OF ITS OWN. The router's own moved answer
        // is the string `"302 /new"` and has no headers, so it must be rebuilt to get one. A ROUTE
        // that answered `redirectTo("/")` already has the header and an empty body — rebuilding
        // that one would take `cutAfter("", " ")`, hand `redirectTo` an empty target, and send the
        // browser a redirect to nowhere. Reading which case it is off the headers rather than off
        // the status is what tells the two apart.
        let blank_sym = self.interner.intern("");
        let no_location = self.te_bin(
            BinaryOpKind::Eq,
            self.te_id(loc_var),
            self.te_text(blank_sym),
        );
        let moved_302 = self.te_bin(BinaryOpKind::Eq, self.te_id(s_var), self.te_int(302));
        let moved_301 = self.te_bin(BinaryOpKind::Eq, self.te_id(s_var), self.te_int(301));
        let is_302 = self.te_bin(BinaryOpKind::And, moved_302, no_location);
        let is_301 = self.te_bin(BinaryOpKind::And, moved_301, no_location);
        hbody.push(Stmt::If {
            cond: is_302,
            then_block: {
                let moved = self.te_call(redirect_to, vec![where_to]);
                let moved = saved(self, moved);
                self.alloc_block(vec![Stmt::Return { value: Some(moved) }])
            },
            else_block: None,
        });
        hbody.push(Stmt::If {
            cond: is_301,
            then_block: {
                let moved = self.te_call(permanent_to, vec![where_to]);
                let moved = saved(self, moved);
                self.alloc_block(vec![Stmt::Return { value: Some(moved) }])
            },
            else_block: None,
        });
        let response_ty = self.interner.intern("Response");
        let answer = self.ctx.alloc_imperative_expr(Expr::New {
            type_name: response_ty,
            type_args: Vec::new(),
            init_fields: vec![
                (self.interner.intern("status"), self.te_id(s_var)),
                (self.interner.intern("contentType"), self.te_id(c_var)),
                (self.interner.intern("body"), self.te_id(b_var)),
                (self.interner.intern("headers"), self.te_id(h_var)),
            ],
        });
        let answer = saved(self, answer);
        hbody.push(Stmt::Return {
            value: Some(answer),
        });
        let request_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Named(self.interner.intern("Request")));
        let response_ty_expr: &'a TypeExpr<'a> =
            self.ctx.alloc_type_expr(TypeExpr::Named(response_ty));
        self.deferred_functions.push(Stmt::FunctionDef {
            name: handler_name,
            generics: Vec::new(),
            params: vec![(request_p, request_ty)],
            body: self.alloc_block(hbody),
            return_type: Some(response_ty_expr),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        });
        let name = self.interner.intern(ROUTE_BODY_WALK);
        self.deferred_functions.push(Stmt::FunctionDef {
            name,
            generics: Vec::new(),
            params: vec![(req_p, request_walk_ty)],
            body: self.alloc_block(arms),
            return_type: Some(text_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        });

        // ★ `routeWith(method, url, body)` AND ITS TWO SIBLINGS ARE WRAPPERS NOW, not the walks.
        //
        // They mean exactly what they always meant, and every caller — every test, every
        // `Serve fetches with routes`, `fullstack_todo_lock`'s dispatcher probe — keeps working
        // unchanged. The direction is what matters: a wrapper can always BUILD a request out of
        // three fields, while a three-field walk can never RECOVER the two fields it was not given.
        // Putting the whole request at the bottom is what lets a route bind `Request`, read a
        // cookie and carry a session; the conveniences sit on top with empty headers, which is
        // honest, because a caller who passed only a method and a url genuinely sent no headers.
        for (wrapper, walk, ret) in [
            ("routeWith", ROUTE_BODY_WALK, text_ty),
            ("routeStatusWith", ROUTE_STATUS_WALK, int_ty),
            ("routeContentTypeWith", ROUTE_CTYPE_WALK, text_ty),
        ] {
            let map_ty = self.interner.intern("Map");
            let text_arg = TypeExpr::Primitive(self.interner.intern("Text"));
            let empty_map = self.ctx.alloc_imperative_expr(Expr::New {
                type_name: map_ty,
                type_args: vec![text_arg.clone(), text_arg],
                init_fields: Vec::new(),
            });
            let built = self.ctx.alloc_imperative_expr(Expr::New {
                type_name: self.interner.intern("Request"),
                type_args: Vec::new(),
                init_fields: vec![
                    (self.interner.intern("method"), self.te_id(method_p)),
                    (self.interner.intern("path"), self.te_id(url_p)),
                    (self.interner.intern("body"), self.te_id(body_p)),
                    (self.interner.intern("headers"), empty_map),
                    (self.interner.intern("query"), empty_map),
                ],
            });
            let walk_sym = self.interner.intern(walk);
            let call = self.te_call(walk_sym, vec![built]);
            let wrapper_sym = self.interner.intern(wrapper);
            self.deferred_functions.push(Stmt::FunctionDef {
                name: wrapper_sym,
                generics: Vec::new(),
                params: vec![(method_p, text_ty), (url_p, text_ty), (body_p, text_ty)],
                body: self.alloc_block(vec![Stmt::Return { value: Some(call) }]),
                return_type: Some(ret),
                is_native: false,
                native_path: None,
                is_exported: false,
                export_target: None,
                opt_flags: crate::optimization::OptimizationConfig::all_on(),
            });
        }

        // ★ `routeHeader(method, url, name)` — what a route SET, readable without a server.
        //
        // The handler already puts these on the wire; this is the same table asked the same
        // question from a test, which is what makes "the login set a cookie" an assertion rather
        // than something you have to start a process and dial a socket to find out.
        let name_p = self.interner.intern("__lgx_hname");
        for (wrapper, with_body) in [("routeHeaderWith", true), ("routeHeader", false)] {
            let body_expr = if with_body {
                let text_arg = TypeExpr::Primitive(self.interner.intern("Text"));
                let empty_map = self.ctx.alloc_imperative_expr(Expr::New {
                    type_name: self.interner.intern("Map"),
                    type_args: vec![text_arg.clone(), text_arg],
                    init_fields: Vec::new(),
                });
                let built = self.ctx.alloc_imperative_expr(Expr::New {
                    type_name: self.interner.intern("Request"),
                    type_args: Vec::new(),
                    init_fields: vec![
                        (self.interner.intern("method"), self.te_id(method_p)),
                        (self.interner.intern("path"), self.te_id(url_p)),
                        (self.interner.intern("body"), self.te_id(body_p)),
                        (self.interner.intern("headers"), empty_map),
                        (self.interner.intern("query"), empty_map),
                    ],
                });
                let walk = self.interner.intern(ROUTE_HEADERS_WALK);
                let table = self.te_call(walk, vec![built]);
                let read = self.interner.intern("headerIn");
                self.te_call(read, vec![table, self.te_id(name_p)])
            } else {
                let blank = self.interner.intern("");
                let long = self.interner.intern("routeHeaderWith");
                self.te_call(
                    long,
                    vec![
                        self.te_id(method_p),
                        self.te_id(url_p),
                        self.te_text(blank),
                        self.te_id(name_p),
                    ],
                )
            };
            let params = if with_body {
                vec![
                    (method_p, text_ty),
                    (url_p, text_ty),
                    (body_p, text_ty),
                    (name_p, text_ty),
                ]
            } else {
                vec![(method_p, text_ty), (url_p, text_ty), (name_p, text_ty)]
            };
            let wrapper_sym = self.interner.intern(wrapper);
            self.deferred_functions.push(Stmt::FunctionDef {
                name: wrapper_sym,
                generics: Vec::new(),
                params,
                body: self.alloc_block(vec![Stmt::Return {
                    value: Some(body_expr),
                }]),
                return_type: Some(text_ty),
                is_native: false,
                native_path: None,
                is_exported: false,
                export_target: None,
                opt_flags: crate::optimization::OptimizationConfig::all_on(),
            });
        }
    }

    /// Every STYLED component this root can reach, itself first, in placement order.
    ///
    /// The same closure `html_reachable` walks, kept separate because that one answers a yes/no
    /// about one name and this one needs the whole set in a stable order — a page's stylesheets
    /// must come out the same on every build, or two identical sources produce two different files.
    pub(crate) fn styled_reachable(&self, root: Symbol, all: &[ComponentDef<'a>]) -> Vec<Symbol> {
        let mut seen = vec![root];
        let mut queue = vec![root];
        while let Some(cur) = queue.pop() {
            let Some(def) = all.iter().find(|d| d.name == cur) else {
                continue;
            };
            let Some(view) = def.view.as_ref() else {
                continue;
            };
            for child in placed_components(&view.nodes) {
                if !seen.contains(&child) {
                    seen.push(child);
                    queue.push(child);
                }
            }
        }
        seen.retain(|n| all.iter().any(|d| d.name == *n && d.style.is_some()));
        seen
    }

    /// Is this component REACHED by an `html of` — itself named by one, or placed (transitively)
    /// in the view of something that is?
    ///
    /// ★ THE DEMAND HAS TO FOLLOW THE TREE. `html of App` is the only thing an author writes, and
    /// a page is built out of rows and cards nobody names — so a rule that fired only on the
    /// spelled name emitted the page's own renderer and none of the children it calls, which is a
    /// call to a function that does not exist. Same shape as the JSON derive closing over
    /// record-typed fields: the seed is what the source says, and the set is its CLOSURE.
    pub(crate) fn html_reachable(
        &self,
        name: Symbol,
        all: &[ComponentDef<'a>],
        routed: &[Symbol],
    ) -> bool {
        if self.names_html_of(name) || routed.contains(&name) {
            return true;
        }
        // ★ A `## Route … -> View:` IS A SECOND ROOT. Its template lowers through the same server
        // walk and calls `__html_<Child>` for every component it places, so a page reached only
        // from a route used to compile into a call to a function nothing emitted — the route built
        // and the server answered with a missing name. `routed` carries those roots.
        let mut seeds: Vec<Symbol> = all
            .iter()
            .filter(|d| self.names_html_of(d.name))
            .map(|d| d.name)
            .collect();
        seeds.extend(routed.iter().copied());
        let mut seen: Vec<Symbol> = seeds.clone();
        while let Some(cur) = seeds.pop() {
            let Some(def) = all.iter().find(|d| d.name == cur) else {
                continue;
            };
            let Some(view) = def.view.as_ref() else {
                continue;
            };
            for child in placed_components(&view.nodes) {
                if child == name {
                    return true;
                }
                if !seen.contains(&child) {
                    seen.push(child);
                    seeds.push(child);
                }
            }
        }
        false
    }

    /// Does the program say `html of <name>` anywhere?
    ///
    /// Scanned from the TOKEN STREAM, so it is whole-word by construction — the same way the
    /// JSON derive finds the encoders a program reaches.
    pub(crate) fn names_html_of(&self, name: Symbol) -> bool {
        self.tokens.windows(3).any(|w| {
            self.interner
                .resolve(w[0].lexeme)
                .eq_ignore_ascii_case("html")
                && self
                    .interner
                    .resolve(w[1].lexeme)
                    .eq_ignore_ascii_case("of")
                && w[2].lexeme == name
        })
    }

    /// Whether the program renders markup on a server: a `## Route`, or an `html of` anywhere.
    pub(crate) fn renders_on_a_server(&self, statements: &[Stmt<'a>]) -> bool {
        statements
            .iter()
            .any(|s| matches!(s, Stmt::RouteDef { .. }))
            || self.tokens.windows(2).any(|w| {
                self.interner
                    .resolve(w[0].lexeme)
                    .eq_ignore_ascii_case("html")
                    && self
                        .interner
                        .resolve(w[1].lexeme)
                        .eq_ignore_ascii_case("of")
            })
    }

    /// The component a template is drawn as: its parameters its props, and nothing of its own —
    /// no state, no actions, no style. A route's template, a `Head:`, a `-> View` function's.
    pub(crate) fn template_owner(
        &self,
        name: Symbol,
        params: Vec<(Symbol, &'a TypeExpr<'a>)>,
        view: &crate::ast::view::ViewDef<'a>,
    ) -> ComponentDef<'a> {
        ComponentDef {
            kind: crate::ast::component::ComponentKind::Component,
            name,
            props: params,
            uses: Vec::new(),
            state: Vec::new(),
            events: Vec::new(),
            actions: Vec::new(),
            view: Some(view.clone()),
            head: None,
            style: None,
            span: view.span,
        }
    }

    /// ★ ONE RENDERER, TWO DESTINATIONS. `draw` — records written onto the tape by the serving walk
    /// (`LowerTrail::serving`) — is drawn into a root no host holds on a tape of its own, committed,
    /// serialized by `vdSerializeChildren`, and the root discarded, leaving the markup in `out`. The markup is the
    /// retained tree the browser's paint of the same records would hold, serialized as a host
    /// serializes a document — escaping, void elements, attribute order and all — so a served page
    /// and a painted one are one document by construction, and nothing of the drawing (tree, tape,
    /// patch stream) outlives it.
    pub(crate) fn serve_markup(
        &mut self,
        draw: Vec<Stmt<'a>>,
        out: Symbol,
        body: &mut Vec<Stmt<'a>>,
    ) {
        let root = self.interner.intern("__lgx_served");
        let problem = self.interner.intern("__lgx_problem");
        let stash = self.interner.intern("__lgx_outer_tape");
        // ★ RE-ENTRANT: a root may be drawn while another draw is in flight — `html of` in a
        // served component's state, `{raw html of C}` in a painted view — so the tape in flight is
        // set aside, and put back once this root is serialized, exactly as it was.
        let stashed = self.vd_expr("vdTapeStash", Vec::new());
        body.push(Stmt::Let {
            var: stash,
            ty: None,
            value: stashed,
            mutable: false,
        });
        let detached = self.vd_expr("vdNewRoot", vec![self.te_int(-1)]);
        body.push(Stmt::Let {
            var: root,
            ty: None,
            value: detached,
            mutable: false,
        });
        body.extend(draw);
        let committed = self.vd_expr("vdCommit", vec![self.te_id(root)]);
        body.push(Stmt::Let {
            var: problem,
            ty: None,
            value: committed,
            mutable: false,
        });
        let blank = self.interner.intern("");
        let refused = self.te_bin(
            BinaryOpKind::NotEq,
            self.te_id(problem),
            self.te_text(blank),
        );
        let give_up = vec![
            self.vd_call("vdDiscardRoot", vec![self.te_id(root)]),
            self.vd_call("vdTapeUnstash", vec![self.te_id(stash)]),
            Stmt::Fail {
                reason: self.te_id(problem),
            },
        ];
        body.push(Stmt::If {
            cond: refused,
            then_block: self.alloc_block(give_up),
            else_block: None,
        });
        let markup = self.vd_expr("vdSerializeChildren", vec![self.te_id(root)]);
        let text_ty = self.text_type();
        body.push(Stmt::Let {
            var: out,
            ty: Some(text_ty),
            value: markup,
            mutable: false,
        });
        body.push(self.vd_call("vdDiscardRoot", vec![self.te_id(root)]));
        body.push(self.vd_call("vdTapeUnstash", vec![self.te_id(stash)]));
    }

    /// The records of every stylesheet the root `root` reaches, scoped as the browser scopes it —
    /// written once, ahead of the markup, as the mount writes them (`repaint_def`).
    pub(crate) fn served_sheets(
        &mut self,
        root: Symbol,
        all: &[ComponentDef<'a>],
    ) -> Vec<Stmt<'a>> {
        let mut out = Vec::new();
        for styled in self.styled_reachable(root, all) {
            let Some(sheet) = all
                .iter()
                .find(|d| d.name == styled)
                .and_then(|d| d.style.clone())
            else {
                continue;
            };
            let class = scope_class(self.interner.resolve(styled));
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
        out
    }

    /// `nodes` as `owner` draws them, by the serving walk — the records only; the caller decides
    /// where they are drawn.
    pub(crate) fn serving_draw(
        &mut self,
        nodes: &[ViewNode<'a>],
        owner: &ComponentDef<'a>,
        all: &[ComponentDef<'a>],
    ) -> Vec<Stmt<'a>> {
        let mut draw = Vec::new();
        let mut next = 0usize;
        let outer_locals = std::mem::take(&mut self.lgx_views.locals);
        self.lower_lgx_nodes(
            nodes,
            &mut next,
            &mut draw,
            all,
            owner,
            &LowerTrail::serving(),
        );
        self.lgx_views.locals = outer_locals;
        draw
    }

    /// Why `html of <def>`, or a server root that places `def`, cannot be served — the constructs
    /// the server has no rendering for, named in the author's words.
    fn refuse_serving(
        &self,
        def: &ComponentDef<'a>,
        all: &[ComponentDef<'a>],
    ) -> crate::parser::ParseResult<()> {
        let Some(view) = &def.view else { return Ok(()) };
        if let Err(why) = view_html_lowerable_with(&view.nodes, all, self.interner) {
            let name = self.interner.resolve(def.name).to_string();
            return Err(crate::error::ParseError {
                kind: crate::error::ParseErrorKind::Custom(format!(
                    "`html of {name}` asks for this component as server-rendered markup, and {}. \
                     A server has no document, so the server-side walk covers a smaller set of the \
                     view tree than a browser does — and rendering the page SHORT would be worse \
                     than saying so, because the reader would see a plausible document with a \
                     piece missing and nothing anywhere would say which. In a browser this view is \
                     ordinary: `Mount {name}.` renders it in full. So either render it there, or \
                     replace the construct in `{name}`'s view that the server cannot draw.",
                    why.message
                )),
                span: why.span,
            });
        }
        Ok(())
    }

    /// `__serve_<C>(props…, slot, blocks…)` for every component a server root reaches, and
    /// `__html_<C>()` for every component `html of` names.
    ///
    /// `__serve_<C>` is `C`'s view drawn by the serving walk over its INITIAL state — its fields
    /// bound as locals from their initializers, the derived ones after them — writing the records
    /// its markup is made of and nothing a live document needs. Nothing is kept between two
    /// calls: a server answering two requests draws two independent instances, because an instance
    /// never outlives the call that draws it. `__html_<C>()` draws it into a root of its own and
    /// answers the serialized markup (`serve_markup`), stylesheets first.
    ///
    /// ★ EMITTED ON USE, NOT ON DECLARATION. A page that only mounts a component carries no server
    /// render of it; only a root the server draws — `html of`, a `## Route … -> View:`, a `-> View`
    /// function's template — demands one, with the closure of what it places.
    pub(crate) fn lower_component_serve(
        &mut self,
        def: &ComponentDef<'a>,
        all: &[ComponentDef<'a>],
        routed: &[Symbol],
    ) -> crate::parser::ParseResult<()> {
        let Some(view) = &def.view else { return Ok(()) };
        if !self.html_reachable(def.name, all, routed) {
            return Ok(());
        }
        self.refuse_serving(def, all)?;
        let component = self.interner.resolve(def.name).to_string();
        let depth = self.interner.intern(draw_depth_param());
        let mut body: Vec<Stmt<'a>> = Vec::new();
        if on_placement_cycle(def, all) {
            body.push(self.refuse_past_render_depth(depth, def.name));
        }
        for field in def.state.iter().filter(|f| !f.derived) {
            let bound = self.state_field_let(field, field.init, true);
            body.push(bound);
        }
        for field in def.state.iter().filter(|f| f.derived) {
            let bound = self.state_field_let(field, field.init, false);
            body.push(bound);
        }
        let mut next = 0usize;
        let outer_locals = std::mem::take(&mut self.lgx_views.locals);
        let trail = LowerTrail::serving_at(depth);
        match super::render::extended_layout(&view.nodes)
            .and_then(|l| all.iter().find(|d| d.name == l))
        {
            // `{extends L}`: the blocks this view fills are drawn here, in its own scope, as ranges;
            // the layout draws itself around them, one deeper, with this component's `{children}`
            // as its own.
            Some(layout) => {
                let mut args =
                    vec![self.te_bin(BinaryOpKind::Add, self.te_id(depth), self.te_int(1))];
                let slot = self.interner.intern(render_slot_param());
                args.push(self.te_id(slot));
                for block in slot_blocks(layout) {
                    let fill = view.nodes.iter().enumerate().find_map(|(at, n)| match n {
                        ViewNode::Block {
                            name,
                            default: Some(b),
                            ..
                        } if *name == block => Some((at, &b[..])),
                        _ => None,
                    });
                    args.push(match fill {
                        Some((at, b)) => {
                            let branch = trail.branch(&format!("{at}."));
                            self.slot_range(b, &mut next, &mut body, all, def, &branch)
                        }
                        None => self.te_int(0),
                    });
                }
                let serve = self
                    .interner
                    .intern(&serve_fn_name(self.interner.resolve(layout.name)));
                body.push(Stmt::Call {
                    function: serve,
                    args,
                });
            }
            None => self.lower_lgx_nodes(&view.nodes, &mut next, &mut body, all, def, &trail),
        }
        self.lgx_views.locals = outer_locals;
        let int_ty = self.int_type();
        let mut params = vec![(depth, int_ty)];
        for (prop, ty) in &def.props {
            if is_callback_type(ty, self.interner) {
                continue;
            }
            let ty = self.lowered_type(ty);
            params.push((*prop, ty));
        }
        params.push((self.interner.intern(render_slot_param()), int_ty));
        for block in slot_blocks(def) {
            params.push((
                self.interner
                    .intern(&block_slot_param(self.interner.resolve(block))),
                int_ty,
            ));
        }
        self.deferred_functions.push(Stmt::FunctionDef {
            name: self.interner.intern(&serve_fn_name(&component)),
            generics: Vec::new(),
            params,
            body: self.alloc_block(body),
            return_type: None,
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        });
        if !self.names_html_of(def.name) {
            return Ok(());
        }
        // `html of <C>`: the root has no caller, so no props, no children, and every block its own.
        if let Some((prop, _)) = def.props.first() {
            let prop = self.interner.resolve(*prop).to_string();
            return Err(crate::error::ParseError {
                kind: crate::error::ParseErrorKind::Custom(format!(
                    "`html of {component}` draws `{component}` as a page nobody placed, and `{component}` \
                     takes `{prop}`, which only a placement hands it. Place `<{component} {prop}={{…}}/>` in \
                     a component `html of` draws, or in a `## Route … -> View:` template."
                )),
                span: def.span,
            });
        }
        let text_ty = self.text_type();
        let out = self.interner.intern("__lgx_html");
        let mut draw = self.served_sheets(def.name, all);
        let mut args = vec![self.te_int(1)];
        args.push(self.te_int(0));
        args.extend(self.block_slot_zeros(def));
        draw.push(Stmt::Call {
            function: self.interner.intern(&serve_fn_name(&component)),
            args,
        });
        let mut body = Vec::new();
        self.serve_markup(draw, out, &mut body);
        body.push(Stmt::Return {
            value: Some(self.te_id(out)),
        });
        self.deferred_functions.push(Stmt::FunctionDef {
            name: self.interner.intern(&html_fn_name(&component)),
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
        Ok(())
    }
}

/// Whether no definition in `statements` draws a `View:` — a program with no page to place a view
/// in, where every reader of a view reads its markup.
pub(crate) fn no_page_places_a_view(statements: &[Stmt<'_>]) -> bool {
    statements
        .iter()
        .all(|s| !matches!(s, Stmt::ComponentDef(d) if d.view.is_some()))
}

/// The function a `-> View` function's served half lowers to: the same view as HTML text.
///
/// Named apart from a component's `__html_<Name>`, so a function and a component of one name never
/// collide.
pub fn view_html_fn_name(function: &str) -> String {
    format!("__lgx_viewhtml_{function}")
}

/// Route every call of a view function that has a twin to its twin — on a server, reading a view
/// means reading its markup (`lower_view_function_twins`).
struct TwinCalls<'a, 't> {
    arenas: crate::ast_map::Arenas<'a>,
    twins: &'t [(Symbol, Symbol)],
}

impl<'a, 't> TwinCalls<'a, 't> {
    fn twin_of(&self, function: Symbol) -> Option<Symbol> {
        self.twins
            .iter()
            .find(|(f, _)| *f == function)
            .map(|(_, twin)| *twin)
    }
}

impl<'a, 't> crate::ast_map::AstMap<'a> for TwinCalls<'a, 't> {
    fn arenas(&self) -> crate::ast_map::Arenas<'a> {
        self.arenas
    }

    fn map_expr(&mut self, e: &'a Expr<'a>) -> &'a Expr<'a> {
        if let Expr::Call { function, args } = e {
            if let Some(twin) = self.twin_of(*function) {
                let args = args.iter().map(|a| self.map_expr(a)).collect();
                return self.arenas.exprs.alloc(Expr::Call {
                    function: twin,
                    args,
                });
            }
        }
        crate::ast_map::walk_expr(self, e)
    }

    fn map_stmt(&mut self, s: &Stmt<'a>) -> Stmt<'a> {
        if let Stmt::Call { function, args } = s {
            if let Some(twin) = self.twin_of(*function) {
                let args = args.iter().map(|a| self.map_expr(a)).collect();
                return Stmt::Call {
                    function: twin,
                    args,
                };
            }
        }
        crate::ast_map::walk_stmt(self, s)
    }
}
