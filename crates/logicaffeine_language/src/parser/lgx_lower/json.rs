use super::*;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use logicaffeine_base::{Interner, Symbol};

/// Which result type an `Inspect` in the decoder is matching. See `inspect_result`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResultShape {
    /// `Result of T and Text` — `Ok` / `Err`.
    Result,
    /// `JsonResult` from json.lg — `JOk (v)` / `JErr (msg)`.
    Json,
}

/// What kind of value a record field is read back as. See `lower_from_json`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DecodeKind {
    Int,
    Bool,
    Text,
    /// A nested record, decoded through its own `fromJson_`.
    Record(Symbol),
}

/// The derived JSON codec's shape questions — which records encode, which decode, what a field is
/// read back as — asked of a file's type registry and nothing else. The lowering asks them to emit
/// the codec, and whoever asks whether a page lowers (`lowerable`, on the parser and on every tier
/// that explains a missing `Mount`) asks the same questions of the same registry, so a store is
/// never lowered by one and refused by the other.
pub(crate) struct JsonShapes<'r> {
    registry: Option<&'r crate::analysis::registry::TypeRegistry>,
    interner: &'r Interner,
}

/// How a `Seq of <elem>` goes through the codec: a declared record's own derived pair, or the list
/// pair a scalar element gets written on first ask.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SeqCodec {
    Record,
    Scalar,
}

/// How a typed request (`Get … as <T>`) reads its answer back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FetchRead {
    /// `as Text`: the body is the value.
    Text,
    /// `as <Record>`: through `fromJson_<Record>`.
    Record(Symbol),
    /// `as Seq of <elem>`: through `fromJsonSeq_<elem>`.
    Seq(Symbol),
    /// `as Int`, `as Bool`: through `fromJson_<Scalar>`, written on first ask.
    Scalar(Symbol),
}

/// Every type a typed request (`Get … as <T>`) in `stmts` asks its answer as, at any depth of a
/// block and inside a component's actions and event sentences — each request once, in order.
pub(crate) fn typed_requests<'s>(stmts: &[Stmt<'s>], visit: &mut dyn FnMut(&'s TypeExpr<'s>)) {
    for stmt in stmts {
        match stmt {
            Stmt::Fetch { as_type, .. } => visit(*as_type),
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                typed_requests(then_block, visit);
                if let Some(e) = else_block {
                    typed_requests(e, visit);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Splice { body } => {
                typed_requests(body, visit)
            }
            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    typed_requests(arm.body, visit);
                }
            }
            Stmt::ComponentDef(def) => {
                for action in &def.actions {
                    typed_requests(action.body, visit);
                }
                for handler in &def.events {
                    typed_requests(handler.body, visit);
                }
            }
            _ => {}
        }
    }
}

impl<'r> JsonShapes<'r> {
    /// The questions over `registry`; with none, no record is known, so none encodes.
    pub(crate) fn new(
        registry: Option<&'r crate::analysis::registry::TypeRegistry>,
        interner: &'r Interner,
    ) -> Self {
        JsonShapes { registry, interner }
    }

    /// The interner the questions resolve names through.
    pub(crate) fn interner(&self) -> &'r Interner {
        self.interner
    }

    /// How a `Seq of <elem>` is coded, or `None` when this file cannot round-trip it: a declared
    /// record must encode AND decode field by field ([`Self::record_round_trips`]); anything else
    /// must be a scalar the list decoder reads back ([`Self::scalar_decode_kind`]).
    pub(crate) fn seq_codec(&self, elem: Symbol) -> Option<SeqCodec> {
        if self.declared_record(elem) {
            return self.record_round_trips(elem).then_some(SeqCodec::Record);
        }
        self.scalar_decode_kind(elem).map(|_| SeqCodec::Scalar)
    }

    /// How `Get … as <ty>` reads its answer, or — when the codec cannot read `ty` — the refusal
    /// naming it ([`crate::ast::component::typed_fetch_refusal`]). The fetch lowering emits the
    /// read this answers, and the gate refuses the component with the sentence it answers, so a
    /// page is never admitted by one and left without its mount by the other.
    pub(crate) fn fetch_read(&self, ty: &TypeExpr<'_>) -> Result<FetchRead, String> {
        let refused = || crate::ast::component::typed_fetch_refusal(ty, self.interner);
        match ty {
            TypeExpr::Named(t) | TypeExpr::Primitive(t) => {
                if self.interner.resolve(*t).eq_ignore_ascii_case("Text") {
                    Ok(FetchRead::Text)
                } else if self.declared_record(*t) {
                    self.record_round_trips(*t)
                        .then_some(FetchRead::Record(*t))
                        .ok_or_else(refused)
                } else if self.scalar_decode_kind(*t).is_some() {
                    Ok(FetchRead::Scalar(*t))
                } else {
                    Err(refused())
                }
            }
            TypeExpr::Generic { base, params }
                if self.interner.resolve(*base).eq_ignore_ascii_case("Seq")
                    && params.len() == 1 =>
            {
                match &params[0] {
                    TypeExpr::Named(t) | TypeExpr::Primitive(t) if self.seq_codec(*t).is_some() => {
                        Ok(FetchRead::Seq(*t))
                    }
                    _ => Err(refused()),
                }
            }
            _ => Err(refused()),
        }
    }

    /// A `Seq of T` field's element type — the one generic shape the JSON derive encodes, because
    /// a list is what a record most often holds and what it was silently getting wrong.
    pub(crate) fn seq_element(&self, ty: &crate::analysis::registry::FieldType) -> Option<Symbol> {
        use crate::analysis::registry::FieldType;
        let FieldType::Generic { base, params } = ty else {
            return None;
        };
        // Only `Seq`. An `Option` or a `Result` is not an array, and encoding one as an array
        // would be a WRONG answer rather than a missing one.
        if !self.interner.resolve(*base).eq_ignore_ascii_case("Seq") || params.len() != 1 {
            return None;
        }
        match params.first() {
            Some(FieldType::Primitive(a) | FieldType::Named(a) | FieldType::TypeParam(a)) => {
                Some(*a)
            }
            // A list OF lists, or of options, is not encoded — better no encoder at all than one
            // that is right about some elements.
            _ => None,
        }
    }

    /// The element type of an `Option of <T>` field, when the derive can encode one.
    ///
    /// Separate from [`Self::seq_element`] because an option is not an array: it is the VALUE or
    /// `null`, and encoding one as `[]`/`[v]` would be a wrong answer rather than a missing one.
    pub(crate) fn option_element(
        &self,
        ty: &crate::analysis::registry::FieldType,
    ) -> Option<Symbol> {
        use crate::analysis::registry::FieldType;
        let FieldType::Generic { base, params } = ty else {
            return None;
        };
        if !self.interner.resolve(*base).eq_ignore_ascii_case("Option") || params.len() != 1 {
            return None;
        }
        match params.first() {
            Some(FieldType::Primitive(a) | FieldType::Named(a) | FieldType::TypeParam(a)) => {
                Some(*a)
            }
            // An option OF a list or of another option is not encoded — better no encoder at all
            // than one that is right about some shapes.
            _ => None,
        }
    }

    /// The types that print as themselves and are VALID JSON doing so.
    ///
    /// An allowlist, deliberately, because the alternative is what this replaced: a fall-through
    /// that printed LOGOS's own display for ANY type it had no case for. That is correct for a
    /// number and a boolean and WRONG for everything else — an enum went out as `{"level":Red}`,
    /// invalid JSON, emitted silently by a derive reporting success. A list of what is safe can
    /// only fail by being too small, which is a missing encoder; a fall-through fails by being
    /// too large, which is a wrong body in somebody else's parser.
    pub(crate) fn json_bare_scalar(&self, s: Symbol) -> bool {
        json_bare_scalar_name(self.interner.resolve(s))
    }

    /// Can this ONE field be encoded, given the records that can be?
    pub(crate) fn json_field_encodable(
        &self,
        ty: &crate::analysis::registry::FieldType,
        ok_records: &[Symbol],
    ) -> bool {
        use crate::analysis::registry::FieldType;
        let scalar_ok = |s: Symbol| {
            self.json_bare_scalar(s)
                || self.interner.resolve(s).eq_ignore_ascii_case("Text")
                || ok_records.contains(&s)
        };
        match ty {
            FieldType::Primitive(s) | FieldType::Named(s) | FieldType::TypeParam(s) => {
                scalar_ok(*s)
            }
            FieldType::Generic { .. } => self
                .seq_element(ty)
                .or_else(|| self.option_element(ty))
                .is_some_and(scalar_ok),
            FieldType::Tuple(_) | FieldType::Record(_) => false,
        }
    }

    /// Every non-generic record this file declares, with its own field list.
    ///
    /// Records live in the TYPE REGISTRY, not the statement list — a `## A Todo has:` block is
    /// consumed by the discovery pass and emits no `Stmt`. A GENERIC record needs one encoder per
    /// instantiation, which this pass does not do yet; encoding it as if its parameters were
    /// concrete would be a wrong answer, so it is left out here rather than encoded wrong.
    ///
    /// The one place this file reads the registry's record shapes — [`Self::lower_derived_json`]'s
    /// own derive pass, and any OTHER caller asking "can `Seq of X`/a bare `X` be JSON-coded"
    /// before that pass has run (`Save`/`Load`, a store's persistence) — read the SAME list, so the
    /// two can never disagree about what a record IS.
    pub(crate) fn all_records(
        &self,
    ) -> Vec<(Symbol, Vec<(Symbol, crate::analysis::registry::FieldType)>)> {
        use crate::analysis::registry::TypeDef;
        let Some(registry) = self.registry else {
            return Vec::new();
        };
        registry
            .iter_types()
            .filter_map(|(name, def)| match def {
                TypeDef::Struct {
                    fields, generics, ..
                } if generics.is_empty() => Some((
                    *name,
                    fields
                        .iter()
                        .map(|f| (f.name, f.ty.clone()))
                        .collect::<Vec<_>>(),
                )),
                _ => None,
            })
            .collect()
    }

    /// The records every one of whose fields the derive can encode.
    ///
    /// A FIXPOINT rather than one pass, because a record is encodable only if the records it
    /// nests are: dropping `Inner` has to drop `Outer` too, or `toJson_Outer` is emitted calling
    /// a function that does not exist. Starts optimistic and shrinks, which also terminates on a
    /// record that refers to itself.
    pub(crate) fn json_encodable_records(
        &self,
        records: &[(Symbol, Vec<(Symbol, crate::analysis::registry::FieldType)>)],
    ) -> Vec<Symbol> {
        let mut ok: Vec<Symbol> = records.iter().map(|(n, _)| *n).collect();
        loop {
            let before = ok.len();
            let keep: Vec<Symbol> = ok
                .iter()
                .copied()
                .filter(|n| {
                    records
                        .iter()
                        .find(|(m, _)| m == n)
                        .is_some_and(|(_, fields)| {
                            fields
                                .iter()
                                .all(|(_, ty)| self.json_field_encodable(ty, &ok))
                        })
                })
                .collect();
            ok = keep;
            if ok.len() == before {
                return ok;
            }
        }
    }

    /// Whether `ty` names a record `## A Foo has:` declared in this file (a struct in the type
    /// registry) — as opposed to a primitive, a type param, or a name nothing declares.
    pub(crate) fn declared_record(&self, ty: Symbol) -> bool {
        use crate::analysis::registry::TypeDef;
        self.registry
            .is_some_and(|r| matches!(r.get(ty), Some(TypeDef::Struct { .. })))
    }

    /// Whether `ty` names a record this file can round-trip through the derived JSON codec BOTH
    /// ways — encode ([`Self::json_encodable_records`]) and decode ([`Self::decode_plan`]). The
    /// read-only twin of [`Self::ensure_seq_codec`]/[`Self::ensure_record_codec`]'s own
    /// verification, for a caller (`gate.rs`'s `store_lowerable`) that only needs the ANSWER, never
    /// the seeding a real ask performs.
    pub(crate) fn record_round_trips(&self, ty: Symbol) -> bool {
        if !self.declared_record(ty) {
            return false;
        }
        let all = self.all_records();
        if !self.json_encodable_records(&all).contains(&ty) {
            return false;
        }
        let record_names: Vec<Symbol> = all.iter().map(|(n, _)| *n).collect();
        let Some((_, fields)) = all.iter().find(|(n, _)| *n == ty) else {
            return false;
        };
        self.decode_plan(fields, &record_names).is_some()
    }

    /// The record this field IS — not one it merely contains.
    ///
    /// Deliberately separate from [`Self::field_encodes_through`], which follows a list or an
    /// option through to its ELEMENT because reachability has to: `toJson_Outer` calls
    /// `toJson_Inner` whether the inner sits in the field or in a list in the field. Asking the
    /// reachability question where the "is this field a record" question belongs would classify a
    /// `Seq of Inner` as a single `Inner` and decode a whole ARRAY as one record — a silent wrong
    /// answer, and the exact bug widening the walk introduced here before the two were split.
    pub(crate) fn direct_record(
        &self,
        ty: &crate::analysis::registry::FieldType,
        records: &[Symbol],
    ) -> Option<Symbol> {
        use crate::analysis::registry::FieldType;
        let named = match ty {
            FieldType::Primitive(s) | FieldType::Named(s) | FieldType::TypeParam(s) => *s,
            FieldType::Generic { .. } | FieldType::Tuple(_) | FieldType::Record(_) => return None,
        };
        if self.interner.resolve(named).eq_ignore_ascii_case("Text") {
            return None;
        }
        records.contains(&named).then_some(named)
    }

    /// Which record, if any, a field of type `ty` is encoded THROUGH — the single decision the
    /// derive's reachability closure and its emitter both read.
    ///
    /// They must agree exactly: a field the emitter recurses into but the closure misses emits a
    /// call to a function that was never derived, and a field the closure follows but the emitter
    /// ignores derives a function nobody calls — which is the regression this reachability rule
    /// exists to prevent. Widening the emitter (Seq/Option fields, say) widens this, once.
    ///
    /// Every record REACHED through this field, for the derive's closure — through a list or an
    /// option as well as directly. See [`Self::direct_record`] for the other question.
    pub(crate) fn field_encodes_through(
        &self,
        ty: &crate::analysis::registry::FieldType,
        records: &[Symbol],
    ) -> Option<Symbol> {
        use crate::analysis::registry::FieldType;
        let named = match ty {
            FieldType::Primitive(s) | FieldType::Named(s) | FieldType::TypeParam(s) => *s,
            // ★ A `Seq of Inner` and an `Option of Inner` REACH `Inner` too. This used to return
            // `None` for every generic — true when neither was encoded, and quietly wrong from the
            // moment lists became arrays: `Inner` was never reached, so it got no encoder, so the
            // record HOLDING the list was judged unencodable and got none either. The whole record
            // disappeared from the wire because of a stale line in a reachability walk.
            FieldType::Tuple(_) | FieldType::Record(_) => return None,
            FieldType::Generic { .. } => {
                match self.seq_element(ty).or_else(|| self.option_element(ty)) {
                    Some(elem) => elem,
                    None => return None,
                }
            }
        };
        if self.interner.resolve(named).eq_ignore_ascii_case("Text") {
            return None;
        }
        records.contains(&named).then_some(named)
    }

    /// How ONE field is read back off the wire.
    pub(crate) fn decode_plan(
        &self,
        fields: &[(Symbol, crate::analysis::registry::FieldType)],
        record_names: &[Symbol],
    ) -> Option<Vec<(Symbol, DecodeKind, bool)>> {
        use crate::analysis::registry::FieldType;
        let mut out = Vec::new();
        for (field, ty) in fields {
            if let Some(rec) = self.direct_record(ty, record_names) {
                out.push((*field, DecodeKind::Record(rec), false));
                continue;
            }
            // An OPTIONAL field is its inner type plus permission to be absent, so it decodes
            // through the same accessor with a `null` test in front. Written here rather than as
            // four more kinds because the only thing optionality changes is what happens when the
            // value is not there.
            let (named, optional) = match (ty, self.option_element(ty)) {
                (_, Some(inner)) => (inner, true),
                (FieldType::Primitive(s) | FieldType::Named(s) | FieldType::TypeParam(s), _) => {
                    (*s, false)
                }
                // A Seq, a Result or a tuple field is not decoded, so the record gets no decoder.
                (FieldType::Generic { .. } | FieldType::Tuple(_) | FieldType::Record(_), _) => {
                    return None
                }
            };
            if optional {
                if let Some(rec) = record_names.iter().find(|r| **r == named) {
                    out.push((*field, DecodeKind::Record(*rec), true));
                    continue;
                }
            }
            // Anything else — a Counter, a Money, a type this derive has no accessor for. The
            // record gets NO decoder rather than one that is right about some fields.
            let Some(kind) = self.scalar_decode_kind(named) else {
                return None;
            };
            out.push((*field, kind, optional));
        }
        Some(out)
    }

    /// The bare scalar accessor a type name reads back through — `Int`/`Nat`/`Bool`/`Text` and
    /// their synonyms, `None` for anything else (a record is [`DecodeKind::Record`], read by
    /// [`Self::direct_record`]/[`Self::decode_plan`] instead; nothing else decodes).
    ///
    /// ONE definition, so a record FIELD and a bare `Seq of <scalar>` element (`Save`/`Load`, a
    /// store's own field) read the identical set back rather than two lists drifting the first
    /// time a synonym is added.
    pub(crate) fn scalar_decode_kind(&self, name: Symbol) -> Option<DecodeKind> {
        match self.interner.resolve(name).to_ascii_lowercase().as_str() {
            "int" | "nat" | "integer" => Some(DecodeKind::Int),
            "bool" | "boolean" => Some(DecodeKind::Bool),
            "text" | "string" => Some(DecodeKind::Text),
            _ => None,
        }
    }
}

impl<'a, 'ctx, 'int> super::super::Parser<'a, 'ctx, 'int> {
    /// The codec's shape questions over this file's type registry.
    pub(crate) fn json_shapes(&self) -> JsonShapes<'_> {
        JsonShapes::new(self.type_registry.as_ref(), self.interner)
    }

    /// The `toJsonSeq_<Elem>`/`fromJsonSeq_<Elem>` pair for a `Seq of <Elem>` value — the ONE
    /// entry point `Save`/`Load` and a store's own persistence ask through, `None` when `Elem` is
    /// neither a scalar this codec already knows nor a record this file can fully encode AND
    /// decode.
    ///
    /// A RECORD element is only PROMISED once its whole field-by-field closure is verified
    /// encodable ([`Self::json_encodable_records`]) and decodable ([`Self::decode_plan`]) — asking
    /// for a codec [`Self::lower_derived_json`] would silently decline to emit (its own
    /// all-or-nothing rule) would reproduce exactly the "compiles, then calls nothing" gap this
    /// codec exists to close. Verified, it is SEEDED into `json_encoders_needed`/
    /// `json_decoders_needed` (the same `Parser` fields `fetch.rs`/`server.rs` already seed) so
    /// [`Self::lower_derived_json`] — which runs AFTER every caller of this method, per
    /// `component.rs::resolve_lgx`'s own call order — emits the pair normally.
    ///
    /// A SCALAR element is emitted here directly and idempotently: a second ask for the same
    /// element reuses the function an earlier ask already wrote.
    pub(crate) fn ensure_seq_codec(&mut self, elem: Symbol) -> Option<(Symbol, Symbol)> {
        let enc_name = self
            .interner
            .intern(&format!("toJsonSeq_{}", self.interner.resolve(elem)));
        let dec_name = self
            .interner
            .intern(&format!("fromJsonSeq_{}", self.interner.resolve(elem)));
        if self.json_shapes().seq_codec(elem)? == SeqCodec::Record {
            if !self.json_encoders_needed.contains(&elem) {
                self.json_encoders_needed.push(elem);
            }
            if !self.json_decoders_needed.contains(&elem) {
                self.json_decoders_needed.push(elem);
            }
            return Some((enc_name, dec_name));
        }
        let already = self
            .deferred_functions
            .iter()
            .any(|s| matches!(s, Stmt::FunctionDef { name, .. } if *name == enc_name));
        if !already {
            let elem_ty: &'a TypeExpr<'a> = self.ctx.alloc_type_expr(TypeExpr::Named(elem));
            let text_ty: &'a TypeExpr<'a> = self
                .ctx
                .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Text")));
            self.lower_to_json_seq(elem, elem_ty, text_ty, &[]);
            self.lower_from_json_seq(elem, &[]);
        }
        Some((enc_name, dec_name))
    }

    /// The `toJson_<Rec>`/`fromJson_<Rec>` pair for a BARE declared record — `Save`/`Load` and a
    /// store's own persistence asking for a whole record's own codec, never through a `Seq`. See
    /// [`Self::ensure_seq_codec`] for the identical verify-then-seed shape; there is no scalar case
    /// here, since a bare scalar field already has its own fixed one-letter tag.
    pub(crate) fn ensure_record_codec(&mut self, rec: Symbol) -> Option<(Symbol, Symbol)> {
        if !self.json_shapes().record_round_trips(rec) {
            return None;
        }
        if !self.json_encoders_needed.contains(&rec) {
            self.json_encoders_needed.push(rec);
        }
        if !self.json_decoders_needed.contains(&rec) {
            self.json_decoders_needed.push(rec);
        }
        let enc = self
            .interner
            .intern(&format!("toJson_{}", self.interner.resolve(rec)));
        let dec = self
            .interner
            .intern(&format!("fromJson_{}", self.interner.resolve(rec)));
        Some((enc, dec))
    }

    /// `fromJson_<Scalar>` for a bare scalar answer (`Get … as Int`), `None` for a type the codec
    /// reads no scalar of. Written on first ask and reused by every later one, as a scalar list's
    /// pair is ([`Self::ensure_seq_codec`]).
    pub(crate) fn ensure_scalar_codec(&mut self, scalar: Symbol) -> Option<Symbol> {
        let kind = self.json_shapes().scalar_decode_kind(scalar)?;
        let dec = self
            .interner
            .intern(&format!("fromJson_{}", self.interner.resolve(scalar)));
        let already = self
            .deferred_functions
            .iter()
            .any(|s| matches!(s, Stmt::FunctionDef { name, .. } if *name == dec));
        if !already {
            self.lower_from_json_scalar(scalar, kind, dec);
        }
        Some(dec)
    }

    /// `## To fromJson_<Scalar> (text: Text) -> Result of <Scalar> and Text`: the text parsed as
    /// JSON, then read as the scalar — through the accessor a record field of that scalar is read
    /// with, so a value decodes the same whether it arrives alone, in a list, or in a record.
    pub(crate) fn lower_from_json_scalar(
        &mut self,
        scalar: Symbol,
        kind: DecodeKind,
        fn_name: Symbol,
    ) {
        let text_p = self.interner.intern("__lgx_text");
        let doc = self.interner.intern("__lgx_doc");
        let one = self.interner.intern("__lgx_one");
        let accessor = match kind {
            DecodeKind::Int => "jsonInt",
            DecodeKind::Bool => "jsonBool",
            DecodeKind::Text | DecodeKind::Record(_) => "jsonString",
        };
        let accessor = self.interner.intern(accessor);
        let try_parse = self.interner.intern("jsonTryParse");
        let read = self.te_call(accessor, vec![self.te_id(doc)]);
        let decoded = self.te_id(one);
        let answer = Stmt::Return {
            value: Some(self.result_ok(decoded)),
        };
        let scalar_gate =
            self.decode_arm_binding(read, ResultShape::Result, one, scalar, vec![answer]);
        let parsed = self.te_call(try_parse, vec![self.te_id(text_p)]);
        let json_word = self.interner.intern("JSON");
        let parse_gate =
            self.decode_arm_binding(parsed, ResultShape::Json, doc, json_word, vec![scalar_gate]);
        let text = self.interner.intern("Text");
        let text_ty: &'a TypeExpr<'a> = self.ctx.alloc_type_expr(TypeExpr::Primitive(text));
        let result_ty: &'a TypeExpr<'a> = self.ctx.alloc_type_expr(TypeExpr::Generic {
            base: self.interner.intern("Result"),
            params: self
                .ctx
                .alloc_type_exprs(vec![TypeExpr::Primitive(scalar), TypeExpr::Primitive(text)]),
        });
        self.deferred_functions.push(Stmt::FunctionDef {
            name: fn_name,
            generics: Vec::new(),
            params: vec![(text_p, text_ty)],
            body: self.alloc_block(vec![parse_gate]),
            return_type: Some(result_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        });
    }

    /// Encode ONE value of type `ty_sym` as JSON: quoted-and-escaped Text, a nested record
    /// through its own encoder, or an allowlisted scalar printed as itself.
    ///
    /// Shared by all three positions a value appears in — a plain field, an element of a list,
    /// and the inside of an option — so the escaping cannot reach one and miss another. That is
    /// exactly how the list bug happened: a quote inside an array element ends the string it
    /// lives in and everything after becomes structure nobody wrote.
    pub(crate) fn encode_scalar(
        &mut self,
        ty_sym: Symbol,
        read: &'a Expr<'a>,
        records: &[Symbol],
    ) -> &'a Expr<'a> {
        if self.interner.resolve(ty_sym).eq_ignore_ascii_case("Text") {
            let quote = self.interner.intern("jsonQuote");
            return self.te_call(quote, vec![read]);
        }
        if records.contains(&ty_sym) {
            let nested = self
                .interner
                .intern(&format!("toJson_{}", self.interner.resolve(ty_sym)));
            return self.te_call(nested, vec![read]);
        }
        let blank = self.interner.intern("");
        self.te_bin(BinaryOpKind::Add, self.te_text(blank), read)
    }

    pub(crate) fn te_not(&self, inner: &'a Expr<'a>) -> &'a Expr<'a> {
        self.ctx.alloc_imperative_expr(Expr::Not { operand: inner })
    }

    /// Emit `toJson_<T>` for every record the file declares.
    ///
    /// ONE derive, emitting ordinary LOGOS, because the alternative is a serializer per backend
    /// and the moment two tiers spell a record differently the wire stops being a contract. It is
    /// also what makes the result testable: `toJson_Todo` is a function every tier runs and every
    /// ordinary lock can call, rather than something only a running server reveals.
    ///
    /// A user who writes their own `toJson_<T>` keeps it — declarer wins, the same rule the
    /// stdlib prelude follows, so an escape hatch never needs a flag.
    pub(crate) fn lower_derived_json(&mut self, statements: &[Stmt<'a>]) {
        use crate::analysis::registry::FieldType;
        self.seed_fetch_codecs(statements);
        let mut records = self.json_shapes().all_records();
        if records.is_empty() {
            return;
        }
        // ★ REACHABILITY. An encoder is derived for a record that is ENCODED, never for one that
        // is merely DECLARED — the difference is not cosmetic. A function nobody calls has no
        // call site to infer its parameter's type from, and the C bytecode Op-walker types a
        // function's parameters from its callers: it saw `GetField` on a value of unknown kind
        // and refused THE WHOLE PROGRAM, taking out every C/CUDA/RISC-V lock whose fixture
        // happens to declare a record. A derive that fires on declaration taxes every program
        // that merely names the shape.
        //
        // Seeds are every `toJson_<T>` the program NAMES (scanned from the token stream, so it
        // is whole-word by construction) plus every record a `## Route` answers — the router
        // synthesizes those calls, so they never appear in the source the author wrote. The set
        // is then CLOSED over record-typed fields, because `toJson_Outer` calls `toJson_Inner`.
        // A typed fetch (`Get … as Todo, then loaded.`) DECODES that record, and its `as` clause
        // is the reference — the decoder's name never appears in the source the author wrote,
        // exactly as a route's encoder does not. Third construct with this shape; each one has
        // to declare its demand or the derive cannot see it — `seed_fetch_codecs`, above.
        let all_records: Vec<Symbol> = records.iter().map(|(n, _)| *n).collect();
        let mut reached: Vec<Symbol> = Vec::new();
        {
            let decoder_named: std::collections::HashSet<String> = self
                .tokens
                .iter()
                .map(|t| self.interner.resolve(t.lexeme).to_string())
                .filter(|w| w.starts_with("fromJson"))
                .collect();
            for (name, _) in &records {
                let d = format!("fromJson_{}", self.interner.resolve(*name));
                let ds = format!("fromJsonSeq_{}", self.interner.resolve(*name));
                if decoder_named.contains(&d) || decoder_named.contains(&ds) {
                    if !self.json_decoders_needed.contains(name) {
                        self.json_decoders_needed.push(*name);
                    }
                }
            }
            let named: std::collections::HashSet<&str> = self
                .tokens
                .iter()
                .map(|t| self.interner.resolve(t.lexeme))
                .filter(|w| {
                    w.starts_with("toJson_")
                        || w.starts_with("fromJson_")
                        || w.starts_with("fromJsonSeq_")
                })
                .collect();
            let seed = |n: Symbol, reached: &mut Vec<Symbol>| {
                if !reached.contains(&n) {
                    reached.push(n);
                }
            };
            for (name, _) in &records {
                let encoder = format!("toJson_{}", self.interner.resolve(*name));
                let decoder = format!("fromJson_{}", self.interner.resolve(*name));
                let seq_decoder = format!("fromJsonSeq_{}", self.interner.resolve(*name));
                if named.contains(encoder.as_str())
                    || named.contains(decoder.as_str())
                    || named.contains(seq_decoder.as_str())
                    || self.json_encoders_needed.contains(name)
                {
                    seed(*name, &mut reached);
                }
            }
            let mut i = 0;
            while i < reached.len() {
                let here = reached[i];
                i += 1;
                let Some((_, fields)) = records.iter().find(|(n, _)| *n == here) else {
                    continue;
                };
                for (_, ty) in fields.clone() {
                    if let Some(n) = self.json_shapes().field_encodes_through(&ty, &all_records) {
                        seed(n, &mut reached);
                    }
                }
            }
        }
        records.retain(|(n, _)| reached.contains(n));
        // ★ ALL-OR-NOTHING PER RECORD, the rule `fromJson_<T>` already followed. A record holding
        // one field the derive has no case for gets NO encoder rather than one that is right
        // about the others: a missing function is a loud failure at the call site, while a body
        // that is invalid JSON is a bug in somebody else's parser, found long after it shipped.
        let encodable = self.json_shapes().json_encodable_records(&records);
        records.retain(|(n, _)| encodable.contains(n));
        if records.is_empty() {
            return;
        }
        // The registry is a HashMap, so its iteration order varies run to run. Emitting functions
        // in that order would make the generated program differ between builds of the SAME source
        // — the kind of non-determinism that turns a byte-comparison lock into a coin flip.
        records.sort_by_key(|(n, _)| self.interner.resolve(*n).to_string());
        let declared: Vec<Symbol> = statements
            .iter()
            .filter_map(|s| match s {
                Stmt::FunctionDef { name, .. } => Some(*name),
                _ => None,
            })
            .collect();
        let text_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Text")));
        let record_names: Vec<Symbol> = records.iter().map(|(n, _)| *n).collect();
        for (name, fields) in &records {
            let fn_name = self
                .interner
                .intern(&format!("toJson_{}", self.interner.resolve(*name)));
            // Declarer wins: a hand-written encoder is never shadowed or duplicated, the same
            // rule the stdlib prelude follows — so the escape hatch needs no flag.
            if declared.contains(&fn_name) {
                continue;
            }
            let value_p = self.interner.intern("__lgx_value");
            let out = self.interner.intern("__lgx_json");
            let open = self.interner.intern("{");
            let mut body: Vec<Stmt<'a>> = vec![Stmt::Let {
                var: out,
                ty: Some(text_ty),
                value: self.te_text(open),
                mutable: true,
            }];
            for (i, (field, ty)) in fields.iter().enumerate() {
                let lead = if i == 0 {
                    String::new()
                } else {
                    ",".to_string()
                };
                let key = self
                    .interner
                    .intern(&format!("{lead}\"{}\":", self.interner.resolve(*field)));
                let grown = self.te_bin(BinaryOpKind::Add, self.te_id(out), self.te_text(key));
                body.push(Stmt::Set {
                    target: out,
                    value: grown,
                });
                let read = self.ctx.alloc_imperative_expr(Expr::FieldAccess {
                    object: self.te_id(value_p),
                    field: *field,
                });
                let ty_sym = match ty {
                    FieldType::Primitive(s) | FieldType::Named(s) | FieldType::TypeParam(s) => {
                        Some(*s)
                    }
                    FieldType::Generic { .. } | FieldType::Tuple(_) | FieldType::Record(_) => None,
                };
                let is_text =
                    ty_sym.is_some_and(|s| self.interner.resolve(s).eq_ignore_ascii_case("Text"));
                // Text is QUOTED and escaped — a value carrying a quote or a backslash must not
                // be able to end the string it lives in. A nested record recurses through its own
                // encoder, chosen by the SAME rule the reachability closure followed to derive it.
                // Everything else prints as itself.
                let nested_record = self.json_shapes().direct_record(ty, &record_names);
                // ★ A LIST FIELD IS A JSON ARRAY, BUILT ELEMENT BY ELEMENT.
                //
                // It used to fall through to the `"" + value` arm, which prints LOGOS's own list
                // display — so a `Seq of Text` went out as `[a, b]`, UNQUOTED, which every JSON
                // parser rejects. A route answering that record sent a body no client could read
                // while the derive claimed to have encoded it: silent, and visible only to
                // whoever was receiving it.
                if let Some(elem) = self.json_shapes().seq_element(ty) {
                    let open = self.interner.intern("[");
                    let opened =
                        self.te_bin(BinaryOpKind::Add, self.te_id(out), self.te_text(open));
                    body.push(Stmt::Set {
                        target: out,
                        value: opened,
                    });
                    // A `first` flag rather than a trailing comma trimmed off the end: the
                    // separator goes BETWEEN elements, and building it that way cannot leave a
                    // dangling comma before `]` on an empty or one-element list.
                    let first = self
                        .interner
                        .intern(&format!("__first_{}", self.interner.resolve(*field)));
                    let yes = self
                        .ctx
                        .alloc_imperative_expr(Expr::Literal(Literal::Boolean(true)));
                    body.push(Stmt::Let {
                        var: first,
                        ty: None,
                        value: yes,
                        mutable: true,
                    });
                    let item = self
                        .interner
                        .intern(&format!("__item_{}", self.interner.resolve(*field)));
                    let mut loop_body: Vec<Stmt<'a>> = Vec::new();
                    let comma = self.interner.intern(",");
                    let with_comma =
                        self.te_bin(BinaryOpKind::Add, self.te_id(out), self.te_text(comma));
                    loop_body.push(Stmt::If {
                        cond: self.te_not(self.te_id(first)),
                        then_block: self.alloc_block(vec![Stmt::Set {
                            target: out,
                            value: with_comma,
                        }]),
                        else_block: None,
                    });
                    let no = self
                        .ctx
                        .alloc_imperative_expr(Expr::Literal(Literal::Boolean(false)));
                    loop_body.push(Stmt::Set {
                        target: first,
                        value: no,
                    });
                    // The ELEMENT is encoded by its own type — the escaping has to reach INSIDE
                    // the array, where it is easy to forget: a quote in an element ends the string
                    // it lives in and everything after becomes structure nobody wrote.
                    let item_read = self.te_id(item);
                    let element = self.encode_scalar(elem, item_read, &record_names);
                    let appended = self.te_bin(BinaryOpKind::Add, self.te_id(out), element);
                    loop_body.push(Stmt::Set {
                        target: out,
                        value: appended,
                    });
                    body.push(Stmt::Repeat {
                        pattern: crate::ast::stmt::Pattern::Identifier(item),
                        iterable: read,
                        body: self.alloc_block(loop_body),
                    });
                    let close = self.interner.intern("]");
                    let closed =
                        self.te_bin(BinaryOpKind::Add, self.te_id(out), self.te_text(close));
                    body.push(Stmt::Set {
                        target: out,
                        value: closed,
                    });
                    continue;
                }
                // ★ AN OPTION IS THE VALUE OR `null`, and `null` is a VALUE — a reader can tell
                // "the sender has nothing here" apart from "the sender did not know this field",
                // which are different facts about different bugs. It used to fall through to the
                // display arm, so an `Option of Text` went out as `{"tag":x}`: unquoted, invalid,
                // and silent. LOGOS encodes options transparently (`some x` IS x), so the inner
                // value is read straight from the field once presence is settled.
                if let Some(inner) = self.json_shapes().option_element(ty) {
                    // ★ MATCH THE TAG, do not read through the wrapper. This used to test
                    // `read is none` and then hand `read` itself to the scalar encoder, because
                    // `some x` WAS `x`. An Option now carries its own tag, so the payload has to
                    // be BOUND — otherwise the encoder is handed the Option and reports
                    // `text_bytes() requires text, got Option`.
                    let null = self.interner.intern("null");
                    let as_null =
                        self.te_bin(BinaryOpKind::Add, self.te_id(out), self.te_text(null));
                    let bound = self.interner.intern("__json_opt_value");
                    let present = self.encode_scalar(inner, self.te_id(bound), &record_names);
                    let as_value = self.te_bin(BinaryOpKind::Add, self.te_id(out), present);
                    let some_v = self.interner.intern("Some");
                    let none_v = self.interner.intern("None");
                    let arms = vec![
                        crate::ast::stmt::MatchArm {
                            enum_name: None,
                            variant: Some(some_v),
                            bindings: vec![(bound, bound)],
                            body: self.alloc_block(vec![Stmt::Set {
                                target: out,
                                value: as_value,
                            }]),
                        },
                        crate::ast::stmt::MatchArm {
                            enum_name: None,
                            variant: Some(none_v),
                            bindings: Vec::new(),
                            body: self.alloc_block(vec![Stmt::Set {
                                target: out,
                                value: as_null,
                            }]),
                        },
                    ];
                    body.push(Stmt::Inspect {
                        target: read,
                        arms,
                        has_otherwise: false,
                    });
                    continue;
                }
                let rendered = if is_text {
                    let quote = self.interner.intern("jsonQuote");
                    self.te_call(quote, vec![read])
                } else if let Some(rec) = nested_record {
                    let nested = self
                        .interner
                        .intern(&format!("toJson_{}", self.interner.resolve(rec)));
                    self.te_call(nested, vec![read])
                } else {
                    // Reachable only for an allowlisted bare scalar: `json_encodable_records` has
                    // already dropped every record holding anything else.
                    let blank = self.interner.intern("");
                    self.te_bin(BinaryOpKind::Add, self.te_text(blank), read)
                };
                let grown = self.te_bin(BinaryOpKind::Add, self.te_id(out), rendered);
                body.push(Stmt::Set {
                    target: out,
                    value: grown,
                });
            }
            let close = self.interner.intern("}");
            let closed = self.te_bin(BinaryOpKind::Add, self.te_id(out), self.te_text(close));
            body.push(Stmt::Set {
                target: out,
                value: closed,
            });
            body.push(Stmt::Return {
                value: Some(self.te_id(out)),
            });
            let record_ty: &'a TypeExpr<'a> = self.ctx.alloc_type_expr(TypeExpr::Named(*name));
            // ★ AND THE LIST ENCODER BESIDE IT, so a route can ANSWER a list — which is the plan's
            // own headline example (`## Route GET /api/todos -> Seq of Todo:`) and the shape a
            // collection endpoint always has. A record that can be encoded can be encoded in a
            // list, exactly as `fromJsonSeq_<T>` is emitted beside every decoder.
            if !declared.contains(
                &self
                    .interner
                    .intern(&format!("toJsonSeq_{}", self.interner.resolve(*name))),
            ) {
                self.lower_to_json_seq(*name, record_ty, text_ty, &record_names);
            }
            self.deferred_functions.push(Stmt::FunctionDef {
                name: fn_name,
                generics: Vec::new(),
                params: vec![(value_p, record_ty)],
                body: self.alloc_block(body),
                return_type: Some(text_ty),
                is_native: false,
                native_path: None,
                is_exported: false,
                export_target: None,
                opt_flags: crate::optimization::OptimizationConfig::all_on(),
            });
        }
        // ★ A ROUTE THAT ANSWERS JSON DOES NOT READ IT. The encoder and the decoder were seeded
        // from ONE list, so a `## Route … -> Seq of Todo` emitted a decoder nothing ever calls —
        // and a decoder drags in json.lg's parser and its arbitrary-precision number accessors,
        // which is a large amount of code, and on the AOT tier a whole type-promotion problem,
        // for a server that only ever WRITES JSON.
        //
        // A decoder is emitted when the program NAMES one (its own source says `fromJson_T`) or a
        // typed fetch asks for one. Emit on USE — the same rule the encoder learned in T6.5a.
        // Closed over record-typed fields, exactly as the encoder list is: `fromJson_Outer` calls
        // `fromJson_Inner`, so wanting the outer one wants the inner one — and a decoder emitted
        // calling a function that was never emitted is a program that does not run.
        let mut want: Vec<Symbol> = self.json_decoders_needed.clone();
        let mut i = 0;
        while i < want.len() {
            let here = want[i];
            i += 1;
            let Some((_, fields)) = records.iter().find(|(n, _)| *n == here) else {
                continue;
            };
            for (_, ty) in fields.clone() {
                if let Some(n) = self.json_shapes().field_encodes_through(&ty, &record_names) {
                    if !want.contains(&n) {
                        want.push(n);
                    }
                }
            }
        }
        let wanted: Vec<(Symbol, Vec<(Symbol, crate::analysis::registry::FieldType)>)> = records
            .iter()
            .filter(|(n, _)| want.contains(n))
            .cloned()
            .collect();
        self.lower_from_json(&wanted, &declared, &record_names);
    }

    /// `a new Ok with value <v>` — the ONE place this lowering builds a success.
    ///
    /// ★ IT MUST BE `Expr::NewVariant`, NEVER `Expr::New`. A `New` compiles to `NewStruct` (a
    /// record whose type NAME is "Ok"); a `NewVariant` compiles to `NewInductive` (a tagged
    /// constructor of `Result`). The tree-walker cannot tell them apart — its `TestArm` matches a
    /// struct's type name OR an inductive's constructor with one comparison — so the record form
    /// runs correctly there and TRAPS in a browser, where an object carries no type name and the
    /// tag word holds a field instead. A second copy of this builder once got that wrong, and the
    /// only symptom was a page that painted and then died on its own first fetch.
    pub(crate) fn result_ok(&mut self, value: &'a Expr<'a>) -> &'a Expr<'a> {
        let result = self.interner.intern("Result");
        let ok = self.interner.intern("Ok");
        let field = self.interner.intern("value");
        self.ctx.alloc_imperative_expr(Expr::NewVariant {
            enum_name: result,
            variant: ok,
            fields: vec![(field, value)],
        })
    }

    /// `a new Err with error <message>` — the ONE place this lowering builds a failure.
    /// See [`Self::result_ok`] for why this is a `NewVariant` and not a `New`.
    pub(crate) fn result_err(&mut self, message: &'a Expr<'a>) -> &'a Expr<'a> {
        let result = self.interner.intern("Result");
        let err = self.interner.intern("Err");
        let field = self.interner.intern("error");
        self.ctx.alloc_imperative_expr(Expr::NewVariant {
            enum_name: result,
            variant: err,
            fields: vec![(field, message)],
        })
    }

    /// The decoder every typed request in `statements` reads its answer through
    /// ([`JsonShapes::fetch_read`]), asked for whether or not its page lowers: the reference tier
    /// decodes a request through the same derived function the lowered page calls, and the
    /// decoder's name never appears in the source the author wrote.
    pub(crate) fn seed_fetch_codecs(&mut self, statements: &[Stmt<'a>]) {
        let mut asked: Vec<&'a TypeExpr<'a>> = Vec::new();
        typed_requests(statements, &mut |ty| asked.push(ty));
        for ty in asked {
            match self.json_shapes().fetch_read(ty) {
                Ok(FetchRead::Record(rec)) => {
                    self.ensure_record_codec(rec);
                }
                Ok(FetchRead::Seq(elem)) => {
                    self.ensure_seq_codec(elem);
                }
                Ok(FetchRead::Scalar(scalar)) => {
                    self.ensure_scalar_codec(scalar);
                }
                Ok(FetchRead::Text) | Err(_) => {}
            }
        }
    }

    /// `toJsonSeq_<Elem>(items) -> Text` — the LIST encoder, beside every record encoder.
    ///
    /// `elem` is a scalar this codec already knows (`Int`/`Nat`/`Bool`/`Text`) as readily as a
    /// declared record — [`Self::encode_scalar`] already dispatches on exactly that distinction
    /// for a record FIELD's own element, so a bare `Seq of Int` (a `Save`/`Load` statement, or a
    /// store's own field — neither one a record field) reuses the identical per-item encoding
    /// rather than a second copy of the same three-way choice.
    ///
    /// Built with a `first` flag rather than a trailing comma trimmed off the end: the separator
    /// goes BETWEEN elements, and building it that way cannot leave a dangling comma before `]` on
    /// an empty or one-element list. An empty list is `[]` — a value the program HAS, not a
    /// missing answer.
    pub(crate) fn lower_to_json_seq(
        &mut self,
        elem: Symbol,
        elem_ty: &'a TypeExpr<'a>,
        text_ty: &'a TypeExpr<'a>,
        records: &[Symbol],
    ) {
        let fn_name = self
            .interner
            .intern(&format!("toJsonSeq_{}", self.interner.resolve(elem)));
        let items = self.interner.intern("items");
        let out = self.interner.intern("__out");
        let first = self.interner.intern("__first");
        let item = self.interner.intern("__item");
        let open = self.interner.intern("[");
        let mut body: Vec<Stmt<'a>> = vec![
            Stmt::Let {
                var: out,
                ty: None,
                value: self.te_text(open),
                mutable: true,
            },
            Stmt::Let {
                var: first,
                ty: None,
                value: self
                    .ctx
                    .alloc_imperative_expr(Expr::Literal(Literal::Boolean(true))),
                mutable: true,
            },
        ];
        let comma = self.interner.intern(",");
        let with_comma = self.te_bin(BinaryOpKind::Add, self.te_id(out), self.te_text(comma));
        let encoded = self.encode_scalar(elem, self.te_id(item), records);
        let appended = self.te_bin(BinaryOpKind::Add, self.te_id(out), encoded);
        let loop_body = vec![
            Stmt::If {
                cond: self.te_not(self.te_id(first)),
                then_block: self.alloc_block(vec![Stmt::Set {
                    target: out,
                    value: with_comma,
                }]),
                else_block: None,
            },
            Stmt::Set {
                target: first,
                value: self
                    .ctx
                    .alloc_imperative_expr(Expr::Literal(Literal::Boolean(false))),
            },
            Stmt::Set {
                target: out,
                value: appended,
            },
        ];
        body.push(Stmt::Repeat {
            pattern: crate::ast::stmt::Pattern::Identifier(item),
            iterable: self.te_id(items),
            body: self.alloc_block(loop_body),
        });
        let close = self.interner.intern("]");
        let closed = self.te_bin(BinaryOpKind::Add, self.te_id(out), self.te_text(close));
        body.push(Stmt::Set {
            target: out,
            value: closed,
        });
        body.push(Stmt::Return {
            value: Some(self.te_id(out)),
        });
        let seq_ty: &'a TypeExpr<'a> = self.ctx.alloc_type_expr(TypeExpr::Generic {
            base: self.interner.intern("Seq"),
            params: self.ctx.alloc_type_exprs(vec![elem_ty.clone()]),
        });
        self.deferred_functions.push(Stmt::FunctionDef {
            name: fn_name,
            generics: Vec::new(),
            params: vec![(items, seq_ty)],
            body: self.alloc_block(body),
            return_type: Some(text_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        });
    }

    /// `fromJsonSeq_<T>(text) -> Result of Seq of T and Text` — the LIST decoder.
    ///
    /// `Seq of Todo` is what a page showing a list asks a server for, so a decoder that only
    /// handled single records would leave the most ordinary request in the framework unserved.
    /// Emitted beside every record decoder, since a record that can be decoded can be decoded in
    /// a list.
    ///
    /// ALL-OR-NOTHING: one element that will not decode fails the whole answer. Skipping it would
    /// hand the page a shorter list than the server sent, with nothing anywhere to say so.
    ///
    /// `name` is a scalar this codec already knows (`Int`/`Nat`/`Bool`/`Text`) as readily as a
    /// record in `records`: a scalar element decodes straight off the `Json` value `jsonArray`
    /// handed back (the identical accessor [`Self::decode_plan`] uses for a scalar FIELD), never
    /// through the serialize round-trip a nested record's own decoder needs.
    pub(crate) fn lower_from_json_seq(&mut self, name: Symbol, records: &[Symbol]) {
        let type_name = self.interner.resolve(name).to_string();
        let fn_name = self.interner.intern(&format!("fromJsonSeq_{type_name}"));
        let text_p = self.interner.intern("__lgx_text");
        let out = self.interner.intern("__lgx_out");
        let doc = self.interner.intern("__lgx_doc");
        let elems = self.interner.intern("__lgx_elems");
        let item = self.interner.intern("__lgx_item");
        let one = self.interner.intern("__lgx_one");
        let seq_of = self.interner.intern("Seq");
        let elem_ty: &'a TypeExpr<'a> = self.ctx.alloc_type_expr(TypeExpr::Named(name));
        let seq_ty: &'a TypeExpr<'a> = self.ctx.alloc_type_expr(TypeExpr::Generic {
            base: seq_of,
            params: self.ctx.alloc_type_exprs(vec![TypeExpr::Named(name)]),
        });
        let _ = elem_ty;
        // `Push <decoded> to out` for one element, failing the whole answer if it will not read.
        let decode_one = if records.contains(&name) {
            let ser = self.interner.intern("jsonSerialize");
            let inner = self.interner.intern(&format!("fromJson_{type_name}"));
            self.te_call(inner, vec![self.te_call(ser, vec![self.te_id(item)])])
        } else {
            let accessor = match self.json_shapes().scalar_decode_kind(name) {
                Some(DecodeKind::Int) => "jsonInt",
                Some(DecodeKind::Bool) => "jsonBool",
                _ => "jsonString",
            };
            let acc = self.interner.intern(accessor);
            self.te_call(acc, vec![self.te_id(item)])
        };
        let push = Stmt::Push {
            value: self.te_id(one),
            collection: self.te_id(out),
        };
        let per_item =
            self.decode_arm_binding(decode_one, ResultShape::Result, one, name, vec![push]);
        let loop_body = self.alloc_block(vec![per_item]);
        let each = Stmt::Repeat {
            pattern: crate::ast::stmt::Pattern::Identifier(item),
            iterable: self.te_id(elems),
            body: loop_body,
        };
        let answer = Stmt::Return {
            value: Some(self.result_ok(self.te_id(out))),
        };
        // The array itself: an object where a list was expected is an ERROR, not an empty list.
        let arr = self.interner.intern("jsonArray");
        let as_array = self.te_call(arr, vec![self.te_id(doc)]);
        let array_gate = self.decode_arm_binding(
            as_array,
            ResultShape::Result,
            elems,
            name,
            vec![each, answer],
        );
        let try_parse = self.interner.intern("jsonTryParse");
        let parsed = self.te_call(try_parse, vec![self.te_id(text_p)]);
        // Capitalized so this label never collides with the stdlib function literally named
        // `json` (`assets/std/http.lg`) under `emitted_call_demand_lock`'s name-intersection scan
        // — this is a message prefix, never a call, but the census reads interned literals by
        // spelling alone.
        let json_word = self.interner.intern("JSON");
        let parse_gate =
            self.decode_arm_binding(parsed, ResultShape::Json, doc, json_word, vec![array_gate]);
        let empty = Stmt::Let {
            var: out,
            ty: Some(seq_ty),
            value: self.ctx.alloc_imperative_expr(Expr::New {
                type_name: seq_of,
                type_args: vec![TypeExpr::Named(name)],
                init_fields: Vec::new(),
            }),
            mutable: true,
        };
        let text_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Text")));
        let result_ty: &'a TypeExpr<'a> = self.ctx.alloc_type_expr(TypeExpr::Generic {
            base: self.interner.intern("Result"),
            params: self.ctx.alloc_type_exprs(vec![
                TypeExpr::Generic {
                    base: seq_of,
                    params: self.ctx.alloc_type_exprs(vec![TypeExpr::Named(name)]),
                },
                TypeExpr::Primitive(self.interner.intern("Text")),
            ]),
        });
        self.deferred_functions.push(Stmt::FunctionDef {
            name: fn_name,
            generics: Vec::new(),
            params: vec![(text_p, text_ty)],
            body: self.alloc_block(vec![empty, parse_gate]),
            return_type: Some(result_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        });
    }

    /// `Inspect <result>: When Err (e): Return err(<field>: e). When Ok (x): Set <local> to x.`
    ///
    /// The flat shape — the arm writes into a mutable that already exists, so a record with
    /// twenty scalar fields emits twenty siblings rather than a tree twenty deep.
    pub(crate) fn decode_arm(
        &mut self,
        target: &'a Expr<'a>,
        shape: ResultShape,
        local: Option<Symbol>,
        field: Symbol,
    ) -> Stmt<'a> {
        let e = self.interner.intern("__lgx_e");
        let x = self.interner.intern("__lgx_x");
        // The error NAMES THE FIELD. "expected Int" on its own sends the reader to look at the
        // whole document; `id: expected Int` sends them to the one thing that is wrong.
        let prefix = self
            .interner
            .intern(&format!("{}: ", self.interner.resolve(field)));
        let msg = self.te_bin(BinaryOpKind::Add, self.te_text(prefix), self.te_id(e));
        let fail = Stmt::Return {
            value: Some(self.result_err(msg)),
        };
        let bind = match local {
            Some(local) => Stmt::Set {
                target: local,
                value: self.te_id(x),
            },
            None => Stmt::Return {
                value: Some(self.te_id(x)),
            },
        };
        self.inspect_result(target, shape, e, fail, x, vec![bind])
    }

    /// [`Self::decode_arm`] for an OPTIONAL field: the Ok arm stores `some(x)` rather than `x`,
    /// so the local keeps the `Option` its declaration asked for. An error is STILL an error —
    /// optional means "may be absent", never "may be anything", and silently accepting a value of
    /// the wrong type would put it into a typed field where nothing downstream can question it.
    pub(crate) fn decode_arm_some(
        &mut self,
        target: &'a Expr<'a>,
        local: Symbol,
        field: Symbol,
    ) -> Stmt<'a> {
        let e = self.interner.intern("__lgx_e");
        let x = self.interner.intern("__lgx_x");
        let prefix = self
            .interner
            .intern(&format!("{}: ", self.interner.resolve(field)));
        let msg = self.te_bin(BinaryOpKind::Add, self.te_text(prefix), self.te_id(e));
        let fail = Stmt::Return {
            value: Some(self.result_err(msg)),
        };
        let wrapped = self.ctx.alloc_imperative_expr(Expr::OptionSome {
            value: self.te_id(x),
        });
        let bind = Stmt::Set {
            target: local,
            value: wrapped,
        };
        self.inspect_result(target, ResultShape::Result, e, fail, x, vec![bind])
    }

    /// The same, but the Ok arm BINDS a fresh name and everything after it lives inside — the
    /// shape a nested record needs, having no zero value to seed a mutable with.
    pub(crate) fn decode_arm_binding(
        &mut self,
        target: &'a Expr<'a>,
        shape: ResultShape,
        bind_as: Symbol,
        field: Symbol,
        rest: Vec<Stmt<'a>>,
    ) -> Stmt<'a> {
        let e = self.interner.intern("__lgx_e");
        let prefix = self
            .interner
            .intern(&format!("{}: ", self.interner.resolve(field)));
        let msg = self.te_bin(BinaryOpKind::Add, self.te_text(prefix), self.te_id(e));
        let fail = Stmt::Return {
            value: Some(self.result_err(msg)),
        };
        self.inspect_result(target, shape, e, fail, bind_as, rest)
    }

    /// `Inspect target: When <Err> (<e>): <fail>. When <Ok> (<ok_bind>): <ok_body>.`
    ///
    /// The variant names are a PARAMETER because the two result types this derive threads
    /// through spell them differently: `jsonTryParse` answers a `JsonResult` (`JOk (v)` /
    /// `JErr (msg)`, payload fields named), while the field accessors answer an ordinary
    /// `Result of T and Text` (`Ok` / `Err`). Hard-coding either set silently matched nothing
    /// and the Inspect failed as non-exhaustive at RUN time.
    pub(crate) fn inspect_result(
        &mut self,
        target: &'a Expr<'a>,
        shape: ResultShape,
        e: Symbol,
        fail: Stmt<'a>,
        ok_bind: Symbol,
        ok_body: Vec<Stmt<'a>>,
    ) -> Stmt<'a> {
        let (ok_v, ok_field, err_v, err_field) = match shape {
            // `Result` is `a new Ok with value …` / `a new Err with error …` — its payload fields
            // are NAMED, like every other variant's, and binding by the real field name is what
            // the VM's arm resolution reads.
            ResultShape::Result => ("Ok", Some("value"), "Err", Some("error")),
            ResultShape::Json => ("JOk", Some("v"), "JErr", Some("msg")),
        };
        let ok_v = self.interner.intern(ok_v);
        let err_v = self.interner.intern(err_v);
        let ok_field = ok_field.map_or(ok_bind, |f| self.interner.intern(f));
        let err_field = err_field.map_or(e, |f| self.interner.intern(f));
        let arms = vec![
            crate::ast::stmt::MatchArm {
                enum_name: None,
                variant: Some(err_v),
                bindings: vec![(err_field, e)],
                body: self.alloc_block(vec![fail]),
            },
            crate::ast::stmt::MatchArm {
                enum_name: None,
                variant: Some(ok_v),
                bindings: vec![(ok_field, ok_bind)],
                body: self.alloc_block(ok_body),
            },
        ];
        Stmt::Inspect {
            target,
            arms,
            has_otherwise: false,
        }
    }

    /// `fromJson_<T>(text) -> Result of T and Text` for every record `toJson_<T>` was written for.
    ///
    /// The decoder half. It answers a RESULT rather than a record, because the text came from
    /// somewhere else: a missing field, a field of the wrong kind, or a body that is not JSON at
    /// all are normal things for a stranger's input to be, and a decoder that trapped on them
    /// would put every server one malformed body away from going down. A missing field is an
    /// ERROR and never a zero — defaulting it would hand the program a record nobody wrote, with
    /// nothing anywhere to say so.
    pub(crate) fn lower_from_json(
        &mut self,
        records: &[(Symbol, Vec<(Symbol, crate::analysis::registry::FieldType)>)],
        declared: &[Symbol],
        record_names: &[Symbol],
    ) {
        let text_ty: &'a TypeExpr<'a> = self
            .ctx
            .alloc_type_expr(TypeExpr::Primitive(self.interner.intern("Text")));
        for (name, fields) in records {
            let fn_name = self
                .interner
                .intern(&format!("fromJson_{}", self.interner.resolve(*name)));
            // Declarer wins, exactly as for the encoder.
            if declared.contains(&fn_name) {
                continue;
            }
            // Every field has to be decodable, or the record has none. A partial decoder is the
            // silent-wrong-answer this whole file is written against.
            let Some(plan) = self.json_shapes().decode_plan(fields, record_names) else {
                continue;
            };
            let text_p = self.interner.intern("__lgx_text");
            let json = self.interner.intern("__lgx_j");
            let mut body: Vec<Stmt<'a>> = Vec::new();
            // `jsonTryParse` ANSWERS on malformed input where `jsonParse` traps — which is the
            // whole difference between a decoder and a crash.
            let try_parse = self.interner.intern("jsonTryParse");
            let parsed = self.interner.intern("__lgx_parsed");
            body.push(Stmt::Let {
                var: parsed,
                ty: None,
                value: self.te_call(try_parse, vec![self.te_id(text_p)]),
                mutable: false,
            });
            // The scalar fields bind FLAT — one `Inspect` each at the same level, writing into a
            // mutable seeded with its type's zero. Nesting one inside the next would make the
            // emitted tree as deep as the record is wide.
            let mut scalars: Vec<Stmt<'a>> = Vec::new();
            let mut inits: Vec<(Symbol, &'a Expr<'a>)> = Vec::new();
            let mut nested: Vec<(Symbol, Symbol, Symbol)> = Vec::new();
            for (field, kind, optional) in &plan {
                let local = self
                    .interner
                    .intern(&format!("__lgx_f_{}", self.interner.resolve(*field)));
                let key = *field;
                let got = self.interner.intern("jsonGet");
                let raw = self.te_call(got, vec![self.te_id(json), self.te_text(key)]);
                // ★ AN OPTIONAL FIELD SEEDS WITH `none` AND ONLY LOOKS FURTHER IF SOMETHING IS
                // THERE. `jsonGet` answers `JNull` both for an explicit `null` and for a key the
                // object does not carry, and an optional field treats those alike — which is the
                // whole point: a field added to a record must not break every sender that
                // predates it, and "the sender has nothing here" is what optional MEANS.
                //
                // It also gives the flat shape for free, INCLUDING for a nested record: the
                // non-optional record case has to wrap what follows because it has no zero to
                // seed a mutable with, and `none` is exactly that zero.
                if *optional {
                    let none_lit = self.ctx.alloc_imperative_expr(Expr::OptionNone);
                    scalars.push(Stmt::Let {
                        var: local,
                        ty: None,
                        value: none_lit,
                        mutable: true,
                    });
                    let holder = self
                        .interner
                        .intern(&format!("__lgx_j_{}", self.interner.resolve(*field)));
                    scalars.push(Stmt::Let {
                        var: holder,
                        ty: None,
                        value: raw,
                        mutable: false,
                    });
                    let decoded = match kind {
                        DecodeKind::Record(rec) => {
                            let ser = self.interner.intern("jsonSerialize");
                            let inner = self
                                .interner
                                .intern(&format!("fromJson_{}", self.interner.resolve(*rec)));
                            let text = self.te_call(ser, vec![self.te_id(holder)]);
                            self.te_call(inner, vec![text])
                        }
                        other => {
                            let accessor = match other {
                                DecodeKind::Int => "jsonInt",
                                DecodeKind::Bool => "jsonBool",
                                _ => "jsonString",
                            };
                            let acc = self.interner.intern(accessor);
                            self.te_call(acc, vec![self.te_id(holder)])
                        }
                    };
                    let arm = self.decode_arm_some(decoded, local, *field);
                    let is_null = self.interner.intern("jsonIsNull");
                    let null_test = self.te_call(is_null, vec![self.te_id(holder)]);
                    scalars.push(Stmt::If {
                        cond: self.te_not(null_test),
                        then_block: self.alloc_block(vec![arm]),
                        else_block: None,
                    });
                    inits.push((*field, self.te_id(local)));
                    continue;
                }
                match kind {
                    DecodeKind::Record(rec) => {
                        // A nested record decodes through its OWN decoder, and it is the one
                        // shape with no zero to seed a mutable with — so it binds by wrapping
                        // what follows, which makes the emitted tree exactly as deep as the
                        // record has record-typed fields (rarely more than one).
                        let ser = self.interner.intern("jsonSerialize");
                        let inner = self
                            .interner
                            .intern(&format!("fromJson_{}", self.interner.resolve(*rec)));
                        let call = self.te_call(inner, vec![self.te_call(ser, vec![raw])]);
                        let holder = self
                            .interner
                            .intern(&format!("__lgx_r_{}", self.interner.resolve(*field)));
                        scalars.push(Stmt::Let {
                            var: holder,
                            ty: None,
                            value: call,
                            mutable: false,
                        });
                        nested.push((*field, holder, local));
                        inits.push((*field, self.te_id(local)));
                    }
                    // ★ AN INT BINDS BY WRAPPING, NOT BY SEEDING. The flat shape seeds a mutable
                    // with its type's zero and writes the decoded value in — which is right for a
                    // Bool and a Text, and wrong for an Int: a JSON document may carry a number
                    // too large for a machine word, so the decoded value is arbitrary-precision
                    // while the literal `0` seeding the local is not. The generated Rust then
                    // refuses to compile, in a program whose author wrote nothing numeric.
                    //
                    // Wrapping costs one level of nesting per Int field and needs no zero at all,
                    // because the value arrives already bound. Deliberately NOT applied to Bool
                    // and Text, whose zeros are exact: keeping them flat keeps a wide record's
                    // decoder wide rather than deep, which is what T6.5c's shape is for.
                    DecodeKind::Int => {
                        let acc = self.interner.intern("jsonInt");
                        let call = self.te_call(acc, vec![raw]);
                        let holder = self
                            .interner
                            .intern(&format!("__lgx_n_{}", self.interner.resolve(*field)));
                        scalars.push(Stmt::Let {
                            var: holder,
                            ty: None,
                            value: call,
                            mutable: false,
                        });
                        nested.push((*field, holder, local));
                        inits.push((*field, self.te_id(local)));
                    }
                    _ => {
                        let (accessor, zero) = match kind {
                            DecodeKind::Int => ("jsonInt", self.te_int(0)),
                            DecodeKind::Bool => ("jsonBool", self.te_bool(false)),
                            _ => {
                                let blank = self.interner.intern("");
                                ("jsonString", self.te_text(blank))
                            }
                        };
                        let acc = self.interner.intern(accessor);
                        scalars.push(Stmt::Let {
                            var: local,
                            ty: None,
                            value: zero,
                            mutable: true,
                        });
                        let call = self.te_call(acc, vec![raw]);
                        scalars.push(self.decode_arm(
                            call,
                            ResultShape::Result,
                            Some(local),
                            *field,
                        ));
                        inits.push((*field, self.te_id(local)));
                    }
                }
            }
            // The answer, built once every field is bound.
            let built = self.ctx.alloc_imperative_expr(Expr::New {
                type_name: *name,
                type_args: Vec::new(),
                init_fields: inits,
            });
            let mut tail: Vec<Stmt<'a>> = vec![Stmt::Return {
                value: Some(self.result_ok(built)),
            }];
            // Wrap the tail in one `Inspect` per record-typed field, innermost last.
            for (field, holder, local) in nested.into_iter().rev() {
                let holder_id = self.te_id(holder);
                let arm =
                    self.decode_arm_binding(holder_id, ResultShape::Result, local, field, tail);
                tail = vec![arm];
            }
            let mut all = scalars;
            all.extend(tail);
            // The parse itself is the outermost gate: nothing is worth decoding until the text
            // is known to be JSON at all. Capitalized so this message label never collides with
            // the stdlib function literally named `json` (`assets/std/http.lg`) under
            // `emitted_call_demand_lock`'s name-intersection scan — it is never a call.
            let doc = self.interner.intern("JSON");
            let parsed_id = self.te_id(parsed);
            body.push(self.decode_arm_binding(parsed_id, ResultShape::Json, json, doc, all));
            self.lower_from_json_seq(*name, record_names);
            let result_ty: &'a TypeExpr<'a> = self.ctx.alloc_type_expr(TypeExpr::Generic {
                base: self.interner.intern("Result"),
                params: self.ctx.alloc_type_exprs(vec![
                    TypeExpr::Named(*name),
                    TypeExpr::Primitive(self.interner.intern("Text")),
                ]),
            });
            self.deferred_functions.push(Stmt::FunctionDef {
                name: fn_name,
                generics: Vec::new(),
                params: vec![(text_p, text_ty)],
                body: self.alloc_block(body),
                return_type: Some(result_ty),
                is_native: false,
                native_path: None,
                is_exported: false,
                export_target: None,
                opt_flags: crate::optimization::OptimizationConfig::all_on(),
            });
        }
    }
}
