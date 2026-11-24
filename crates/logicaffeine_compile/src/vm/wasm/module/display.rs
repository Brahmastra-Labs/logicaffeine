//! A value's display as Text on wasm — byte-for-byte `RuntimeValue::to_display_string`.
//!
//! A concatenation, an interpolation, `format` and `Fail with` all turn a value into the Text it
//! shows as. The scalars, a Text and a list or set of Int/Bool render inline ([`emit_stringify`],
//! through the host formatters); every other shape — a list of Text or of records, a set of Text, a
//! map, a record, an enum, a tuple, nested to any depth — renders through ONE generated function per
//! shape, `(handle: i32) -> i32 Text`, whose elements render through their own shape's function in
//! turn. The shapes a module needs are collected from its display sites before any body is lowered,
//! closed under their parts, and numbered after the closure trampoline.
//!
//! A set or a list of tuples or lists compares its members BY VALUE, as the VM's `==` does, through the
//! same table: ONE generated equality function per member shape, `(a: i32, b: i32) -> i32`, whose
//! composite parts compare through their own shape's function in turn — collected from the module's set
//! sites and list `contains`, closed under their parts, and numbered after the display functions.

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
            BoundaryType::Set(e) => Shown::Set(Box::new(Shown::of_type(e)?)),
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
            Shown::Set(e) => Kind::set_of(e.kind()?)?,
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

    /// Whether two values of this shape compare inline — an `i64.eq`, an `f64.eq`, or a Text's bytes —
    /// rather than through an equality function of their own.
    fn compared_inline(&self) -> bool {
        matches!(self, Shown::Int | Shown::Float | Shown::Bool | Shown::Text)
    }

    /// The shapes this one's parts compare as, when its members compare by value through a generated
    /// function: a list's element, a tuple's positions, a record's fields, a variant's payloads (the
    /// SAME parts its display function walks — equality needs every field regardless of the NAME
    /// order display sorts them in). `None` for a shape no equality function is generated for — a
    /// map or a set as a member.
    fn compared_parts(
        &self,
        struct_types: &[StructTypeDef],
        enum_types: &[EnumTypeDef],
    ) -> Option<Vec<Shown>> {
        match self {
            Shown::Seq(e) => Some(vec![(**e).clone()]),
            Shown::Tuple(es) => Some(es.clone()),
            Shown::Record(_) | Shown::Variant(_) => Some(self.parts(struct_types, enum_types)),
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

/// The shape a member of the set or list in `collection` has, when its members are compared by value:
/// what the collection's declaration names, else the shape of `member`, a value an op adds, removes or
/// looks for.
pub(super) fn member_of(plan: &Plan, collection: u16, member: Option<u16>) -> Option<Shown> {
    match plan.declared.get(&collection).and_then(Shown::of_type) {
        Some(Shown::Set(e) | Shown::Seq(e)) => Some(*e),
        _ => member.and_then(|r| shown_of_reg(plan, r)),
    }
}

/// The set or list register an op compares members of, and the member it hands over, when its members
/// are handles: a set's add, `contains` and removal, a list's `contains`, a union or an intersection.
fn compared_members(op: &Op, kinds: &KindTable) -> Option<(u16, Option<u16>)> {
    let handles = |r: u16| kinds.get(r as usize) == Some(Kind::SetHandle);
    let list_of_handles = |r: u16| {
        matches!(
            kinds.get(r as usize),
            Some(Kind::SeqSeqInt | Kind::SeqHandle | Kind::SeqStruct | Kind::SeqEnum)
        )
    };
    match *op {
        Op::SetAdd { set, value } if handles(set) => Some((set, Some(value))),
        Op::Contains { collection, value, .. } | Op::RemoveFrom { collection, value }
            if handles(collection) =>
        {
            Some((collection, Some(value)))
        }
        Op::Contains { collection, value, .. } if list_of_handles(collection) => Some((collection, Some(value))),
        Op::UnionOp { lhs, rhs, .. } | Op::IntersectOp { lhs, rhs, .. } => {
            [lhs, rhs].into_iter().find(|&r| handles(r)).map(|r| (r, None))
        }
        _ => None,
    }
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
                Some(Kind::Struct | Kind::SeqStruct | Kind::SetHandle)
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

/// Every shape a module renders through a generated function, and every member shape a set compares
/// by value through one, numbered in a deterministic order starting at `base` in the function index
/// space: the display functions first, then the equality functions.
#[derive(Default)]
pub(super) struct ShownTable {
    order: std::collections::BTreeMap<Shown, u32>,
    equal: std::collections::BTreeMap<Shown, u32>,
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

    /// The member shapes `plans`' sets and lists compare by value, closed under the composite parts their
    /// equality functions call. A member shape no equality function is generated for is left out, and the
    /// op that needs it is refused by name where it is lowered.
    pub(super) fn collect_equal(
        plans: &[&Plan],
        struct_types: &[StructTypeDef],
        enum_types: &[EnumTypeDef],
    ) -> std::collections::BTreeSet<Shown> {
        let mut found = std::collections::BTreeSet::new();
        let mut pending: Vec<Shown> = Vec::new();
        for plan in plans {
            for op in &plan.ops {
                if let Some((collection, member)) = compared_members(op, &plan.kinds) {
                    pending.extend(member_of(plan, collection, member));
                }
            }
        }
        while let Some(s) = pending.pop() {
            if s.compared_inline() {
                continue;
            }
            let Some(parts) = s.compared_parts(struct_types, enum_types) else {
                continue;
            };
            if found.insert(s) {
                pending.extend(parts);
            }
        }
        found
    }

    pub(super) fn new(
        shapes: std::collections::BTreeSet<Shown>,
        equal: std::collections::BTreeSet<Shown>,
    ) -> ShownTable {
        let number = |set: std::collections::BTreeSet<Shown>| {
            set.into_iter()
                .enumerate()
                .map(|(i, s)| (s, i as u32))
                .collect()
        };
        ShownTable {
            order: number(shapes),
            equal: number(equal),
            base: 0,
        }
    }

    /// Number the display functions from `base` in the function index space.
    pub(super) fn place(&mut self, base: u32) {
        self.base = base;
    }

    /// How many functions the table numbers: the display functions and the equality functions.
    pub(super) fn len(&self) -> u32 {
        self.displays() + self.equal.len() as u32
    }

    /// How many display functions the table numbers; the equality functions follow them.
    pub(super) fn displays(&self) -> u32 {
        self.order.len() as u32
    }

    pub(super) fn shapes(&self) -> impl Iterator<Item = &Shown> {
        self.order.keys()
    }

    /// The member shapes the equality functions compare, in their numbering order.
    pub(super) fn equal_shapes(&self) -> impl Iterator<Item = &Shown> {
        self.equal.keys()
    }

    /// The wasm function index of `shape`'s display function.
    fn function(&self, shape: &Shown) -> Option<u32> {
        self.order.get(shape).map(|i| self.base + i)
    }

    /// The wasm function index of the function comparing two values of `shape` by value.
    pub(super) fn equality(&self, shape: &Shown) -> Option<u32> {
        self.equal.get(shape).map(|i| self.base + self.displays() + i)
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

/// The body of the function comparing two values of `shape` by value: `(a: i32, b: i32) -> i32`, `1`
/// when they are equal as the VM's `==` says. One handle is one value; a list is equal to another of the
/// same length whose elements are equal in order; a tuple, position for position. An Int, a Bool and a
/// Real compare as their words do (`f64.eq`, so `-0.0` equals `0.0` and a NaN equals nothing), a Text by
/// its bytes, and a composite part through its own shape's equality function.
pub(super) fn emit_equal_fn(shape: &Shown, ctx: &Ctx) -> R<Vec<u8>> {
    const NR: u32 = 2;
    const A: u32 = 0;
    const B: u32 = 1;
    let (i, ha, hb) = (NR + 1, NR + 2, NR + 3);
    let (wa, wb) = (NR + 13, NR + 14);
    let (fa, fb) = (NR + 15, NR + 16);
    let mut code = Vec::new();
    // Locals after the two parameters: `NR..NR+12` i32 (the `+9`/`+10` an inline Text comparison takes
    // among them), two i64 and two f64 slot words.
    leb_u32(&mut code, 3);
    leb_u32(&mut code, 13);
    code.push(I32);
    leb_u32(&mut code, 2);
    code.push(I64);
    leb_u32(&mut code, 2);
    code.push(F64);
    // The same handle is one value.
    local_get(&mut code, A);
    local_get(&mut code, B);
    code.push(0x46); // i32.eq
    code.push(0x04); // if
    code.push(0x40);
    i32_const(&mut code, 1);
    code.push(0x0F); // return
    code.push(0x0B); // end if
    // Compare the slots at `offset` from the addresses `addr(a)` / `addr(b)` leave as `part`, leaving
    // 1/0; then return 0 at once when they differ.
    let compare = |code: &mut Vec<u8>, part: &Shown, addr: &dyn Fn(&mut Vec<u8>, u32), offset: u32| -> R<()> {
        match part {
            Shown::Int | Shown::Bool => {
                addr(code, A);
                i64_load(code, offset);
                local_set(code, wa);
                addr(code, B);
                i64_load(code, offset);
                local_set(code, wb);
                local_get(code, wa);
                local_get(code, wb);
                code.push(0x51); // i64.eq
            }
            Shown::Float => {
                addr(code, A);
                f64_load(code, offset);
                local_set(code, fa);
                addr(code, B);
                f64_load(code, offset);
                local_set(code, fb);
                local_get(code, fa);
                local_get(code, fb);
                code.push(0x61); // f64.eq
            }
            composite => {
                addr(code, A);
                i32_load(code, offset);
                local_set(code, ha);
                addr(code, B);
                i32_load(code, offset);
                local_set(code, hb);
                if *composite == Shown::Text {
                    emit_text_eq(code, ctx, NR, ha, hb);
                } else {
                    let f = ctx.shown.equality(composite).ok_or(WasmLowerError::Unsupported(
                        "a member part with no equality function",
                    ))?;
                    local_get(code, ha);
                    local_get(code, hb);
                    code.push(0x10); // call
                    leb_u32(code, f);
                }
            }
        }
        code.push(0x45); // i32.eqz
        code.push(0x04); // if (unequal)
        code.push(0x40);
        i32_const(code, 0);
        code.push(0x0F); // return
        code.push(0x0B); // end if
        Ok(())
    };
    match shape {
        Shown::Seq(e) => {
            local_get(&mut code, A);
            i32_load(&mut code, 0);
            local_get(&mut code, B);
            i32_load(&mut code, 0);
            code.push(0x47); // i32.ne
            code.push(0x04); // if (lengths differ)
            code.push(0x40);
            i32_const(&mut code, 0);
            code.push(0x0F); // return
            code.push(0x0B); // end if
            i32_const(&mut code, 0);
            local_set(&mut code, i);
            code.push(0x02); // block
            code.push(0x40);
            code.push(0x03); // loop
            code.push(0x40);
            local_get(&mut code, i);
            local_get(&mut code, A);
            i32_load(&mut code, 0);
            code.push(0x4E); // i32.ge_s
            code.push(0x0D); // br_if block
            leb_u32(&mut code, 1);
            // element i's slot: data_ptr + i*8
            let element = |code: &mut Vec<u8>, handle: u32| {
                local_get(code, handle);
                i32_load(code, 8);
                local_get(code, i);
                i32_const(code, 8);
                code.push(0x6C); // i32.mul
                code.push(0x6A); // i32.add
            };
            compare(&mut code, e, &element, 0)?;
            local_get(&mut code, i);
            i32_const(&mut code, 1);
            code.push(0x6A);
            local_set(&mut code, i);
            code.push(0x0C); // br loop
            leb_u32(&mut code, 0);
            code.push(0x0B); // end loop
            code.push(0x0B); // end block
        }
        Shown::Tuple(es) => {
            let data = |code: &mut Vec<u8>, handle: u32| {
                local_get(code, handle);
                i32_load(code, 8);
            };
            for (k, e) in es.iter().enumerate() {
                compare(&mut code, e, &data, (k as u32) * 8)?;
            }
        }
        // A record compares field for field, in the SAME declaration-order slots its storage uses
        // (unlike display, which re-sorts by name for the printed text — equality needs every field
        // regardless of the order it visits them in). An Int field may ride the record's own counted
        // cell under the exact extension (`ctx.exact.record_ints()`), never a plain slot word, so it
        // compares through the cell-aware `record_int_field_eq` instead of this function's generic
        // `compare` (which would read the cell's HANDLE, not the integer it names).
        Shown::Record(name) => {
            let def = kind::struct_def(name, ctx.struct_types).ok_or(WasmLowerError::Unsupported(
                "equality of an unknown record type",
            ))?;
            let data = |code: &mut Vec<u8>, handle: u32| {
                local_get(code, handle);
                i32_load(code, 8);
            };
            for slot in 0..def.fields.len() {
                let (_, bt) = &def.fields[slot];
                let field = Shown::of_type(bt).ok_or(WasmLowerError::Unsupported(
                    "equality of a record field of a type with no equality",
                ))?;
                if ctx.exact.record_ints() && field == Shown::Int {
                    super::exact::record_int_field_eq(&mut code, ctx, A, B, slot as u16, wa, wb)?;
                    code.push(0x45); // i32.eqz
                    code.push(0x04); // if (unequal)
                    code.push(0x40);
                    i32_const(&mut code, 0);
                    code.push(0x0F); // return
                    code.push(0x0B); // end if
                } else {
                    compare(&mut code, &field, &data, (slot as u32) * 8)?;
                }
            }
        }
        // A variant compares by constructor first (two different ones are never equal, whatever their
        // payloads), then — only when both share a tag — that one variant's payload positions. Different
        // variants can carry different payload shapes, so which positions to compare is a runtime
        // decision, guarded per-tag exactly as the display function's dispatch is.
        Shown::Variant(name) => {
            let def = ctx
                .enum_types
                .iter()
                .find(|e| &e.name == name)
                .ok_or(WasmLowerError::Unsupported("equality of an unknown enum type"))?;
            local_get(&mut code, A);
            i32_load(&mut code, 0);
            local_get(&mut code, B);
            i32_load(&mut code, 0);
            code.push(0x47); // i32.ne
            code.push(0x04); // if (different constructors)
            code.push(0x40);
            i32_const(&mut code, 0);
            code.push(0x0F); // return
            code.push(0x0B); // end if
            let payload = |code: &mut Vec<u8>, handle: u32| {
                local_get(code, handle);
                emit_enum_payload_base(code, ctx);
            };
            for v in &def.variants {
                if v.field_types.is_empty() {
                    continue;
                }
                let Some(tag) = ctx
                    .constants
                    .iter()
                    .position(|c| matches!(c, Constant::Text(n) if *n == v.name))
                else {
                    continue;
                };
                local_get(&mut code, A);
                i32_load(&mut code, 0);
                i32_const(&mut code, tag as i32);
                code.push(0x46); // i32.eq
                code.push(0x04); // if (this is the shared constructor)
                code.push(0x40);
                for (j, ft) in v.field_types.iter().enumerate() {
                    let field = Shown::of_type(ft).ok_or(WasmLowerError::Unsupported(
                        "equality of a payload of a type with no equality",
                    ))?;
                    compare(&mut code, &field, &payload, enum_payload_offset(ctx, j as u32))?;
                }
                code.push(0x0B); // end if
            }
        }
        _ => {
            return Err(WasmLowerError::Unsupported(
                "an equality function for a shape that is not a list, a tuple, a record or a variant",
            ))
        }
    }
    i32_const(&mut code, 1);
    code.push(0x0B); // end
    Ok(code)
}
