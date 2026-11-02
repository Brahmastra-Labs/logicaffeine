//! A value's display as Text on wasm — byte-for-byte `RuntimeValue::to_display_string`.
//!
//! A concatenation, an interpolation, `format` and `Fail with` all turn a value into the Text it
//! shows as. The scalars, a Text and a list or set of Int/Bool render inline ([`emit_stringify`],
//! through the host formatters); every other shape — a list of Text or of records, a set of Text, a
//! map, a record, an enum, a tuple, nested to any depth — renders through ONE generated function per
//! shape, `(handle: i32) -> i32 Text`, whose elements render through their own shape's function in
//! turn. The shapes a module needs are collected from its display sites before any body is lowered,
//! closed under their parts, and numbered after the closure trampoline.

use super::*;

/// What a value displays as: its scalar, or the composite its parts display within.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Shown {
    Int,
    Float,
    Bool,
    Text,
    /// `[e0, e1, …]`.
    Seq(Box<Shown>),
    /// `{e0, e1, …}`, in insertion order.
    Set(Box<Shown>),
    /// `{k0: v0, k1: v1, …}`, in insertion order.
    Map(Box<Shown>, Box<Shown>),
    /// `Name { a: …, b: … }` — fields in name order; `Name` alone for a record with none.
    Record(String),
    /// `Ctor` or `Ctor(p0, p1, …)`.
    Variant(String),
    /// `(e0, e1, …)`.
    Tuple(Vec<Shown>),
}

impl Shown {
    /// The shape a DECLARED type displays as; `None` for a type this backend holds no display for.
    pub(super) fn of_type(bt: &BoundaryType) -> Option<Shown> {
        Some(match bt {
            BoundaryType::Int => Shown::Int,
            BoundaryType::Float => Shown::Float,
            BoundaryType::Bool => Shown::Bool,
            BoundaryType::Text => Shown::Text,
            BoundaryType::Seq(e) => Shown::Seq(Box::new(Shown::of_type(e)?)),
            BoundaryType::Set(e) => match Shown::of_type(e)? {
                e @ (Shown::Int | Shown::Text) => Shown::Set(Box::new(e)),
                _ => return None,
            },
            BoundaryType::Map(k, v) => match Shown::of_type(k)? {
                k @ (Shown::Int | Shown::Text) => {
                    Shown::Map(Box::new(k), Box::new(Shown::of_type(v)?))
                }
                _ => return None,
            },
            BoundaryType::Struct(name) => Shown::Record(name.clone()),
            BoundaryType::Enum(name) => Shown::Variant(name.clone()),
            BoundaryType::Tuple(es) => {
                Shown::Tuple(es.iter().map(Shown::of_type).collect::<Option<Vec<_>>>()?)
            }
            _ => return None,
        })
    }

    /// The shape of a scalar or Text kind.
    fn of_scalar(k: Kind) -> Option<Shown> {
        Some(match k {
            Kind::Int => Shown::Int,
            Kind::Float => Shown::Float,
            Kind::Bool => Shown::Bool,
            Kind::Text => Shown::Text,
            _ => return None,
        })
    }

    /// The kind a value of this shape is held at in a register or an 8-byte slot.
    fn kind(&self) -> Option<Kind> {
        Some(match self {
            Shown::Int => Kind::Int,
            Shown::Float => Kind::Float,
            Shown::Bool => Kind::Bool,
            Shown::Text => Kind::Text,
            Shown::Seq(e) => match e.kind()? {
                Kind::Int => Kind::SeqInt,
                Kind::Bool => Kind::SeqBool,
                Kind::Float => Kind::SeqFloat,
                Kind::Text => Kind::SeqText,
                Kind::Struct => Kind::SeqStruct,
                Kind::Enum => Kind::SeqEnum,
                Kind::SeqInt => Kind::SeqSeqInt,
                _ => Kind::SeqHandle,
            },
            Shown::Set(e) => match **e {
                Shown::Int => Kind::Set,
                Shown::Text => Kind::SetText,
                _ => return None,
            },
            Shown::Map(..) => Kind::Map,
            Shown::Record(_) => Kind::Struct,
            Shown::Variant(_) => Kind::Enum,
            Shown::Tuple(_) => Kind::Tuple,
        })
    }

    /// The kind `emit_stringify` renders this shape at inline — the scalars, Text, and a list or set
    /// of Int/Bool the host formatters render — or `None` for a shape rendered by its own function.
    pub(super) fn inline_kind(&self) -> Option<Kind> {
        match self {
            Shown::Int | Shown::Float | Shown::Bool | Shown::Text => self.kind(),
            Shown::Seq(e) if matches!(**e, Shown::Int | Shown::Bool) => self.kind(),
            Shown::Set(e) if **e == Shown::Int => self.kind(),
            _ => None,
        }
    }

    /// The shapes this one's parts display as.
    fn parts(&self, struct_types: &[StructTypeDef], enum_types: &[EnumTypeDef]) -> Vec<Shown> {
        match self {
            Shown::Seq(e) | Shown::Set(e) => vec![(**e).clone()],
            Shown::Map(k, v) => vec![(**k).clone(), (**v).clone()],
            Shown::Tuple(es) => es.clone(),
            Shown::Record(name) => kind::struct_def(name, struct_types)
                .map(|def| {
                    def.fields
                        .iter()
                        .filter_map(|(_, t)| Shown::of_type(t))
                        .collect()
                })
                .unwrap_or_default(),
            Shown::Variant(name) => enum_types
                .iter()
                .find(|e| &e.name == name)
                .map(|def| {
                    def.variants
                        .iter()
                        .flat_map(|v| v.field_types.iter().filter_map(Shown::of_type))
                        .collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }
}

/// The shape the value in `reg` displays as: its declared type wherever one reached the register,
/// else what its kind and the names the layout pass traced say.
pub(super) fn shown_of_reg(plan: &Plan, reg: u16) -> Option<Shown> {
    // A value of the exact extension prints as the runtime prints it, whatever its binding declares.
    if plan.kinds.get(reg as usize) == Some(Kind::Tower) {
        return None;
    }
    if let Some(s) = plan.declared.get(&reg).and_then(Shown::of_type) {
        return Some(s);
    }
    let named = |m: &std::collections::HashMap<u16, String>| m.get(&reg).cloned();
    Some(match plan.kinds.get(reg as usize)? {
        k @ (Kind::Int | Kind::Float | Kind::Bool | Kind::Text) => Shown::of_scalar(k)?,
        Kind::SeqInt => Shown::Seq(Box::new(Shown::Int)),
        Kind::SeqBool => Shown::Seq(Box::new(Shown::Bool)),
        Kind::SeqFloat => Shown::Seq(Box::new(Shown::Float)),
        Kind::SeqText => Shown::Seq(Box::new(Shown::Text)),
        Kind::SeqSeqInt => Shown::Seq(Box::new(Shown::Seq(Box::new(Shown::Int)))),
        Kind::Set => Shown::Set(Box::new(Shown::Int)),
        Kind::SetText => Shown::Set(Box::new(Shown::Text)),
        Kind::Struct => Shown::Record(named(&plan.structs.struct_name_of)?),
        Kind::SeqStruct => Shown::Seq(Box::new(Shown::Record(named(
            &plan.structs.seq_elem_struct_name,
        )?))),
        Kind::Enum => Shown::Variant(named(&plan.structs.ind_type_of)?),
        Kind::SeqEnum => Shown::Seq(Box::new(Shown::Variant(named(
            &plan.structs.seq_elem_ind_type,
        )?))),
        Kind::Tuple => Shown::Tuple(
            plan.structs
                .tuple_layouts
                .get(&reg)?
                .iter()
                .map(|&e| shown_of_reg(plan, e))
                .collect::<Option<Vec<_>>>()?,
        ),
        Kind::Map => match map_show_kinds(plan, reg) {
            MapShowKinds::Entries(Some(k), Some(v)) => match Shown::of_scalar(k)? {
                k @ (Shown::Int | Shown::Text) => {
                    Shown::Map(Box::new(k), Box::new(Shown::of_scalar(v)?))
                }
                _ => return None,
            },
            _ => return None,
        },
        _ => return None,
    })
}

/// The registers an op renders as Text: the operands of a Text concatenation, an appended value, a
/// default-format interpolation piece, `format(x)` and `Fail with x`.
pub(super) fn displayed_registers(op: &Op, kinds: &KindTable) -> Vec<u16> {
    let text = |r: u16| kinds.get(r as usize) == Some(Kind::Text);
    match *op {
        Op::Concat { lhs, rhs, .. } => vec![lhs, rhs],
        Op::Add { dst, lhs, rhs } if text(dst) => vec![lhs, rhs],
        Op::AddAssign { dst, src } if text(dst) => vec![src],
        Op::Show { src }
            if matches!(
                kinds.get(src as usize),
                Some(Kind::Struct | Kind::SeqStruct)
            ) =>
        {
            vec![src]
        }
        Op::FormatValue { src, .. } => vec![src],
        Op::CallBuiltin {
            builtin: BuiltinId::Format | BuiltinId::Fail,
            args_start,
            arg_count,
            ..
        } if arg_count >= 1 => {
            vec![args_start]
        }
        _ => Vec::new(),
    }
}

/// Every shape a module renders through a generated function, numbered in a deterministic order
/// starting at `base` in the function index space.
#[derive(Default)]
pub(super) struct ShownTable {
    order: std::collections::BTreeMap<Shown, u32>,
    base: u32,
}

impl ShownTable {
    /// The composite shapes `plans` display, closed under their parts.
    pub(super) fn collect(
        plans: &[&Plan],
        struct_types: &[StructTypeDef],
        enum_types: &[EnumTypeDef],
    ) -> std::collections::BTreeSet<Shown> {
        let mut found = std::collections::BTreeSet::new();
        let mut pending: Vec<Shown> = Vec::new();
        for plan in plans {
            for op in &plan.ops {
                for reg in displayed_registers(op, &plan.kinds) {
                    if let Some(s) = shown_of_reg(plan, reg) {
                        pending.push(s);
                    }
                }
            }
        }
        while let Some(s) = pending.pop() {
            if s.inline_kind().is_some() || !found.insert(s.clone()) {
                continue;
            }
            pending.extend(s.parts(struct_types, enum_types));
        }
        found
    }

    pub(super) fn new(shapes: std::collections::BTreeSet<Shown>) -> ShownTable {
        ShownTable {
            order: shapes
                .into_iter()
                .enumerate()
                .map(|(i, s)| (s, i as u32))
                .collect(),
            base: 0,
        }
    }

    /// Number the display functions from `base` in the function index space.
    pub(super) fn place(&mut self, base: u32) {
        self.base = base;
    }

    pub(super) fn len(&self) -> u32 {
        self.order.len() as u32
    }

    pub(super) fn shapes(&self) -> impl Iterator<Item = &Shown> {
        self.order.keys()
    }

    /// The wasm function index of `shape`'s display function.
    fn function(&self, shape: &Shown) -> Option<u32> {
        self.order.get(shape).map(|i| self.base + i)
    }

    /// Every scalar kind the module's display functions render through a host formatter.
    pub(super) fn leaf_kinds(
        &self,
        struct_types: &[StructTypeDef],
        enum_types: &[EnumTypeDef],
    ) -> Vec<Kind> {
        let mut out = Vec::new();
        for s in self.order.keys() {
            for part in s.parts(struct_types, enum_types) {
                if let Some(k) = part.inline_kind() {
                    if !out.contains(&k) {
                        out.push(k);
                    }
                }
            }
        }
        out
    }
}

/// Render the value in local `src`, of shape `shown`, as Text into local `out`.
pub(super) fn emit_shown(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    src: u32,
    shown: &Shown,
    out: u32,
) -> R<()> {
    if let Some(k) = shown.inline_kind() {
        return emit_stringify(code, ctx, num_regs, src, Some(k), out);
    }
    let f = ctx
        .shown
        .function(shown)
        .ok_or(WasmLowerError::Unsupported(
            "display of a value shape with no display function",
        ))?;
    local_get(code, src);
    code.push(0x10); // call
    leb_u32(code, f);
    local_set(code, out);
    Ok(())
}

/// Render register `reg` of `plan` as Text into local `out` — through its shape's display function
/// when it has one, else inline exactly as a scalar always has.
pub(super) fn emit_display_of(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    reg: u16,
    out: u32,
) -> R<()> {
    if super::exact::display(code, plan, ctx, reg, out)? {
        return Ok(());
    }
    match shown_of_reg(plan, reg) {
        Some(s) if s.inline_kind().is_none() => {
            emit_shown(code, ctx, plan.num_regs, reg as u32, &s, out)
        }
        _ => emit_stringify(
            code,
            ctx,
            plan.num_regs,
            reg as u32,
            plan.kinds.get(reg as usize),
            out,
        ),
    }
}

/// The local declarations of a display function: its one `i32` handle parameter is register 0, and
/// the scratch locals follow in the layout [`encode_locals`] gives a one-register function, so the
/// helpers that address scratch as `num_regs + k` work unchanged here.
fn display_locals() -> Vec<u8> {
    let groups: [(u32, u8); 9] = [
        (1, I32), // the dispatch local every body has at `num_regs`
        (4, I64), // +1..+4
        (7, I32), // +5..+11
        (1, F64), // +12
        (2, I32), // +13, +14
        (2, I64), // +15, +16
        (6, I32), // +17..+22
        (1, I32), // +23
        (1, I32), // +24
    ];
    let mut out = Vec::new();
    leb_u32(&mut out, groups.len() as u32);
    for (n, vt) in groups {
        leb_u32(&mut out, n);
        out.push(vt);
    }
    out
}

/// The body of `shape`'s display function: `(handle: i32) -> i32 Text`.
pub(super) fn emit_display_fn(shape: &Shown, ctx: &Ctx) -> R<Vec<u8>> {
    const NR: u32 = 1;
    const HANDLE: u32 = 0;
    let (acc, i, part) = (NR + 17, NR + 18, NR + 19);
    // One local per valtype an element is loaded into; a map's key is rendered before its value is
    // loaded, so the two share them.
    let slot_local = |k: Kind| match k.wasm_valtype() {
        F64 => NR + 12,
        I64 => NR + 15,
        _ => NR + 20,
    };
    let mut code = display_locals();
    let lit = |code: &mut Vec<u8>, bytes: &[u8]| {
        lower_text_literal(code, ctx, NR, bytes);
        local_set(code, part);
        emit_text_append(code, ctx, NR, acc, part, acc);
    };
    let start = |code: &mut Vec<u8>, bytes: &[u8]| {
        lower_text_literal(code, ctx, NR, bytes);
        local_set(code, acc);
    };
    // `for i in 0..len(handle)`, with ", " between iterations; `each` emits one element.
    let each = |code: &mut Vec<u8>, each: &mut dyn FnMut(&mut Vec<u8>) -> R<()>| -> R<()> {
        i32_const(code, 0);
        local_set(code, i);
        code.push(0x02); // block
        code.push(0x40);
        code.push(0x03); // loop
        code.push(0x40);
        local_get(code, i);
        local_get(code, HANDLE);
        i32_load(code, 0); // len
        code.push(0x4E); // i32.ge_s
        code.push(0x0D); // br_if block
        leb_u32(code, 1);
        local_get(code, i);
        code.push(0x04); // if i != 0
        code.push(0x40);
        lit(code, b", ");
        code.push(0x0B); // end if
        each(code)?;
        local_get(code, i);
        i32_const(code, 1);
        code.push(0x6A);
        local_set(code, i);
        code.push(0x0C); // br loop
        leb_u32(code, 0);
        code.push(0x0B); // end loop
        code.push(0x0B); // end block
        Ok(())
    };
    // Load the value at `offset` from the address on the stack as `shown`'s kind, and render it.
    let render_at = |code: &mut Vec<u8>, shown: &Shown, offset: u32| -> R<()> {
        let k = shown.kind().ok_or(WasmLowerError::Unsupported(
            "display of a value shape with no register kind",
        ))?;
        emit_slot_load(code, Some(k), offset)?;
        let tmp = slot_local(k);
        local_set(code, tmp);
        emit_shown(code, ctx, NR, tmp, shown, part)?;
        emit_text_append(code, ctx, NR, acc, part, acc);
        Ok(())
    };
    // The address of element `i`'s 8-byte slot.
    let slot_addr = |code: &mut Vec<u8>| {
        local_get(code, HANDLE);
        i32_load(code, 8); // data_ptr
        local_get(code, i);
        i32_const(code, 8);
        code.push(0x6C); // i32.mul
        code.push(0x6A); // i32.add
    };
    match shape {
        Shown::Seq(e) | Shown::Set(e) => {
            let (open, close): (&[u8], &[u8]) = if matches!(shape, Shown::Seq(_)) {
                (b"[", b"]")
            } else {
                (b"{", b"}")
            };
            start(&mut code, open);
            each(&mut code, &mut |code| {
                slot_addr(code);
                render_at(code, e, 0)
            })?;
            lit(&mut code, close);
        }
        Shown::Map(k, v) => {
            start(&mut code, b"{");
            each(&mut code, &mut |code| {
                emit_map_entry_addr(code, HANDLE, i);
                if ctx.exact.map_ints && matches!(**k, Shown::Int) {
                    super::exact::map_integer_text(code, ctx, NR, HANDLE, 0, part)?;
                    emit_text_append(code, ctx, NR, acc, part, acc);
                } else {
                    render_at(code, k, 0)?;
                }
                lit(code, b": ");
                emit_map_entry_addr(code, HANDLE, i);
                if ctx.exact.map_ints && matches!(**v, Shown::Int) {
                    super::exact::map_integer_text(code, ctx, NR, HANDLE, 8, part)?;
                    emit_text_append(code, ctx, NR, acc, part, acc);
                    Ok(())
                } else {
                    render_at(code, v, 8)
                }
            })?;
            lit(&mut code, b"}");
        }
        Shown::Tuple(es) => {
            start(&mut code, b"(");
            for (j, e) in es.iter().enumerate() {
                if j > 0 {
                    lit(&mut code, b", ");
                }
                local_get(&mut code, HANDLE);
                i32_load(&mut code, 8); // data_ptr
                render_at(&mut code, e, (j as u32) * 8)?;
            }
            lit(&mut code, b")");
        }
        Shown::Record(name) => {
            let def = kind::struct_def(name, ctx.struct_types).ok_or(
                WasmLowerError::Unsupported("display of an unknown record type"),
            )?;
            let mut order: Vec<usize> = (0..def.fields.len()).collect();
            order.sort_by(|&a, &b| def.fields[a].0.cmp(&def.fields[b].0));
            if order.is_empty() {
                start(&mut code, def.name.as_bytes());
            } else {
                start(&mut code, format!("{} {{ ", def.name).as_bytes());
                for (j, &slot) in order.iter().enumerate() {
                    let (fname, bt) = &def.fields[slot];
                    if j > 0 {
                        lit(&mut code, b", ");
                    }
                    lit(&mut code, format!("{fname}: ").as_bytes());
                    let field = Shown::of_type(bt).ok_or(WasmLowerError::Unsupported(
                        "display of a record field of a type with no display",
                    ))?;
                    if ctx.exact.record_ints() && matches!(&field, Shown::Int) {
                        super::exact::record_integer_text(
                            &mut code,
                            ctx,
                            NR,
                            HANDLE,
                            slot as u16,
                            part,
                        )?;
                        emit_text_append(&mut code, ctx, NR, acc, part, acc);
                    } else {
                        local_get(&mut code, HANDLE);
                        i32_load(&mut code, 8); // data_ptr
                        render_at(&mut code, &field, (slot as u32) * 8)?;
                    }
                }
                lit(&mut code, b" }");
            }
        }
        Shown::Variant(name) => {
            let def = ctx.enum_types.iter().find(|e| &e.name == name).ok_or(
                WasmLowerError::Unsupported("display of an unknown enum type"),
            )?;
            start(&mut code, b"");
            for v in &def.variants {
                // The tag is the constant-pool index of the variant's name, the value `NewInductive`
                // stored; a variant never constructed has none and cannot be the value.
                let Some(tag) = ctx
                    .constants
                    .iter()
                    .position(|c| matches!(c, Constant::Text(n) if *n == v.name))
                else {
                    continue;
                };
                local_get(&mut code, HANDLE);
                i32_load(&mut code, 0);
                i32_const(&mut code, tag as i32);
                code.push(0x46); // i32.eq
                code.push(0x04); // if
                code.push(0x40);
                start(&mut code, v.name.as_bytes());
                if !v.field_types.is_empty() {
                    lit(&mut code, b"(");
                    for (j, ft) in v.field_types.iter().enumerate() {
                        if j > 0 {
                            lit(&mut code, b", ");
                        }
                        let field = Shown::of_type(ft).ok_or(WasmLowerError::Unsupported(
                            "display of a payload of a type with no display",
                        ))?;
                        local_get(&mut code, HANDLE);
                        emit_enum_payload_base(&mut code, ctx);
                        render_at(&mut code, &field, enum_payload_offset(ctx, j as u32))?;
                    }
                    lit(&mut code, b")");
                }
                code.push(0x0B); // end if
            }
        }
        Shown::Int | Shown::Float | Shown::Bool | Shown::Text => {
            return Err(WasmLowerError::Unsupported(
                "a scalar shape has no display function",
            ));
        }
    }
    local_get(&mut code, acc);
    code.push(0x0B); // end
    Ok(code)
}
