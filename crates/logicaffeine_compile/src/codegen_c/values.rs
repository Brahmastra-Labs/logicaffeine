//! Value semantics on the C tier.
//!
//! A LOGOS list, map or set — and a record or tuple holding one — is a value: binding it again, handing it to a
//! function, storing it in another collection or iterating it gives a copy the program can change without the
//! original seeing the change, as on the VM. The C tier keeps each collection as a small struct over a heap
//! buffer, so a C copy of the struct shares the buffer. A binding that could be changed through one name while
//! another still reads it therefore takes a copy of its own ([`copy_value`]); which bindings a body changes at
//! all is [`mutated_roots`], and which names a value may share a buffer with is [`source_roots`].

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use logicaffeine_language::ast_depth::{walk_nodes, Visited, Walk};

use super::types::{
    c_param_decl, c_type_str_resolved, escape_c_ident, seq_element, CContext, CType,
};
use crate::analysis::TypeRegistry;
use crate::ast::stmt::{BinaryOpKind, Expr, Stmt};
use crate::intern::Symbol;

/// Whether a value of `ty` reaches a heap buffer the program can change in place: a list, a map or a set, or a
/// record or tuple holding one.
pub(super) fn holds_buffer(
    ty: &CType,
    registry: &TypeRegistry,
    interner: &crate::intern::Interner,
) -> bool {
    fn walk(
        ty: &CType,
        registry: &TypeRegistry,
        interner: &crate::intern::Interner,
        seen: &mut HashSet<Symbol>,
    ) -> bool {
        match ty {
            CType::SeqI64(_)
            | CType::SeqBool
            | CType::SeqStr
            | CType::SeqF64
            | CType::SeqStruct(_)
            | CType::SeqTuple(_)
            | CType::SeqSeq(_)
            | CType::SeqValue(_)
            | CType::MapI64I64(_, _)
            | CType::MapStrI64(_)
            | CType::MapStrStr
            | CType::MapI64Str(_)
            | CType::MapStrValue(_)
            | CType::MapI64Value(_)
            | CType::SetI64(_)
            | CType::SetStr
            | CType::Tower => true,
            CType::Tuple(elems) => elems.iter().any(|e| walk(e, registry, interner, seen)),
            CType::Opt(inner) => walk(inner, registry, interner, seen),
            CType::Record(fields) => fields
                .iter()
                .any(|(_, e)| walk(e, registry, interner, seen)),
            CType::Struct(sym) => {
                if !seen.insert(*sym) {
                    return false;
                }
                match registry.get(*sym) {
                    Some(crate::analysis::TypeDef::Struct { fields, .. }) => {
                        fields.iter().any(|f| {
                            walk(
                                &super::types::field_type_to_ctype(&f.ty, interner, registry),
                                registry,
                                interner,
                                seen,
                            )
                        })
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }
    walk(ty, registry, interner, &mut HashSet::new())
}

/// `value`, a C expression of type `ty`, as a copy sharing no buffer with it: a list's, map's or set's buffers
/// copied, each element copied in turn, a record's and a tuple's parts copied. A value of a type holding no
/// buffer is itself.
pub(super) fn copy_value(ty: &CType, value: &str, ctx: &CContext) -> String {
    match copy_fn(ty, ctx) {
        Some(f) => format!("{f}({value})"),
        None => value.to_string(),
    }
}

/// The generated function copying a value of `ty` (see [`copy_value`]), lifted beside the program's other
/// generated functions; `None` for a type holding no buffer.
fn copy_fn(ty: &CType, ctx: &CContext) -> Option<String> {
    if !holds_buffer(ty, ctx.registry, ctx.interner) {
        return None;
    }
    // An exact value is a cell the runtime copies.
    if *ty == CType::Tower {
        return Some("ltower_own".to_string());
    }
    let tn = c_type_str_resolved(ty, ctx.interner);
    // Every value-or-nothing is one C type, and so is a list or a map of them; its copy is named for what it holds.
    let held = if super::absent::holds_nothing(ty) {
        format!("{ty:?}")
    } else {
        tn.clone()
    };
    let name = format!(
        "__logos_copy_{}",
        held.replace(|c: char| !c.is_ascii_alphanumeric() && c != '_', "_")
    );
    let decl = c_param_decl(ty, "v", ctx.interner);
    // Declared before its body, and generated once: a record may hold a list of itself.
    if !ctx.copy_fns.borrow_mut().insert(name.clone()) {
        return Some(name);
    }
    ctx.lifted.borrow_mut().push(format!(
        "__attribute__((unused)) static {tn} {name}({decl});\n"
    ));
    let mut body = String::new();
    match ty {
        // A value-or-nothing holding a buffer holds it boxed: a present one is boxed again, around a copy.
        CType::Opt(inner) => {
            let inner_copy = copy_fn(inner, ctx)?;
            let it = c_type_str_resolved(inner, ctx.interner);
            writeln!(body, "    {tn} r = v;").unwrap();
            writeln!(
                body,
                "    if (v.present) {{ {it} *box = ({it} *)lc_gc_alloc(sizeof({it})); *box = {inner_copy}(*({it} *)(intptr_t)v.value); r.value = (long long)(intptr_t)box; }}"
            )
            .unwrap();
        }
        CType::Tuple(_) | CType::Record(_) => {
            let elems: Vec<CType> = match ty {
                CType::Record(fields) => fields.iter().map(|(_, t)| t.clone()).collect(),
                CType::Tuple(elems) => elems.clone(),
                _ => unreachable!("a tuple or a record"),
            };
            writeln!(body, "    {tn} r = v;").unwrap();
            for (i, e) in elems.iter().enumerate() {
                if let Some(f) = copy_fn(e, ctx) {
                    writeln!(body, "    r.f{i} = {f}(v.f{i});").unwrap();
                }
            }
        }
        CType::Struct(sym) => {
            writeln!(body, "    {tn} r = v;").unwrap();
            if let Some(crate::analysis::TypeDef::Struct { fields, .. }) = ctx.registry.get(*sym) {
                for field in fields {
                    let fty = super::exact::field_ctype(
                        *sym,
                        field,
                        ctx.interner,
                        ctx.registry,
                        &ctx.field_facts(),
                    );
                    if let Some(f) = copy_fn(&fty, ctx) {
                        let fname = escape_c_ident(ctx.interner.resolve(field.name));
                        writeln!(body, "    r.{fname} = {f}(v.{fname});").unwrap();
                    }
                }
            }
        }
        CType::MapI64I64(_, _)
        | CType::MapStrI64(_)
        | CType::MapStrStr
        | CType::MapI64Str(_)
        | CType::MapStrValue(_)
        | CType::MapI64Value(_) => {
            let value_copy = match ty {
                CType::MapStrValue(inner) | CType::MapI64Value(inner) => copy_fn(inner, ctx),
                _ => None,
            };
            writeln!(body, "    {tn} r = v;").unwrap();
            writeln!(body, "    r.keys = (__typeof__(r.keys))lc_gc_alloc(v.cap * sizeof(*v.keys)); memcpy(r.keys, v.keys, v.cap * sizeof(*v.keys));").unwrap();
            writeln!(body, "    r.vals = (__typeof__(r.vals))lc_gc_alloc(v.cap * sizeof(*v.vals)); memcpy(r.vals, v.vals, v.cap * sizeof(*v.vals));").unwrap();
            if let Some(f) = value_copy {
                writeln!(body, "    for (size_t i = 0; i < v.cap; i++) if (v.state[i]) r.vals[i] = {f}(v.vals[i]);").unwrap();
            }
            writeln!(body, "    r.state = (__typeof__(r.state))lc_gc_alloc(v.cap); memcpy(r.state, v.state, v.cap);").unwrap();
            writeln!(body, "    r.order = (__typeof__(r.order))lc_gc_alloc(v.cap * sizeof(size_t)); memcpy(r.order, v.order, v.cap * sizeof(size_t));").unwrap();
        }
        CType::SetI64(_) | CType::SetStr => {
            writeln!(body, "    {tn} r = v;").unwrap();
            writeln!(body, "    r.keys = (__typeof__(r.keys))lc_gc_alloc(v.cap * sizeof(*v.keys)); memcpy(r.keys, v.keys, v.cap * sizeof(*v.keys));").unwrap();
            writeln!(body, "    r.state = (__typeof__(r.state))lc_gc_alloc(v.cap); memcpy(r.state, v.state, v.cap);").unwrap();
            writeln!(body, "    r.order = (__typeof__(r.order))lc_gc_alloc(v.cap * sizeof(size_t)); memcpy(r.order, v.order, v.cap * sizeof(size_t));").unwrap();
        }
        seq => {
            let element = seq_element(seq, ctx.registry)?;
            let element_copy = copy_fn(&element, ctx);
            writeln!(body, "    {tn} r = v;").unwrap();
            writeln!(body, "    r.cap = v.len;").unwrap();
            writeln!(
                body,
                "    r.data = v.len ? (__typeof__(r.data))lc_gc_alloc(v.len * sizeof(*v.data)) : NULL;"
            )
            .unwrap();
            match element_copy {
                Some(f) => writeln!(
                    body,
                    "    for (size_t i = 0; i < v.len; i++) r.data[i] = {f}(v.data[i]);"
                )
                .unwrap(),
                None => writeln!(
                    body,
                    "    if (v.len) memcpy(r.data, v.data, v.len * sizeof(*v.data));"
                )
                .unwrap(),
            }
        }
    }
    ctx.lifted.borrow_mut().push(format!(
        "__attribute__((unused)) static {tn} {name}({decl}) {{\n{body}    return r;\n}}\n"
    ));
    Some(name)
}

/// Whether the value of `expr` is new — shared with no name — so binding or storing it needs no copy.
pub(super) fn is_fresh(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(_)
        | Expr::List(_)
        | Expr::New { .. }
        | Expr::NewVariant { .. }
        | Expr::Slice { .. }
        | Expr::Copy { .. }
        | Expr::Range { .. }
        | Expr::ManifestOf { .. }
        | Expr::InterpolatedString(_)
        | Expr::Union { .. }
        | Expr::Intersection { .. }
        | Expr::Closure { .. } => true,
        Expr::BinaryOp { .. } | Expr::Not { .. } | Expr::Length { .. } | Expr::Contains { .. } => {
            true
        }
        Expr::WithCapacity { value, .. } => is_fresh(value),
        Expr::Tuple(items) => items.iter().all(|i| is_fresh(i)),
        _ => false,
    }
}

/// The names whose buffers the value of `expr` may share: a name read whole or through its fields and items,
/// and, for a call, every name its arguments may share (a function may hand an argument back).
pub(super) fn source_roots(expr: &Expr, out: &mut HashSet<Symbol>) {
    source_roots_through(expr, &CallShapes::new(), out);
}

/// For each function, which of its arguments its result may share a buffer with: those whose parameter holds a
/// buffer, when its result does. A function missing here may hand back any argument.
pub(super) type CallShapes = std::collections::HashMap<Symbol, Vec<bool>>;

/// [`source_roots`], a call to a function in `shapes` sharing only the arguments its result can be.
pub(super) fn source_roots_through(expr: &Expr, shapes: &CallShapes, out: &mut HashSet<Symbol>) {
    match expr {
        Expr::Identifier(sym) => {
            out.insert(*sym);
        }
        Expr::FieldAccess { object, .. } => source_roots_through(object, shapes, out),
        Expr::Index { collection, .. } => source_roots_through(collection, shapes, out),
        Expr::Give { value } => source_roots_through(value, shapes, out),
        Expr::Call { function, args } => match shapes.get(function) {
            Some(shares) => {
                for (a, &shared) in args.iter().zip(shares) {
                    if shared {
                        source_roots_through(a, shapes, out);
                    }
                }
            }
            None => {
                for a in args {
                    source_roots_through(a, shapes, out);
                }
            }
        },
        Expr::CallExpr { args, .. } => {
            for a in args {
                source_roots_through(a, shapes, out);
            }
        }
        Expr::Tuple(items) => {
            for i in items {
                source_roots_through(i, shapes, out);
            }
        }
        _ => {}
    }
}

/// The name at the root of a place a statement changes (`xs`, `b's items`, `item 2 of b's rows`).
pub(super) fn place_root(place: &Expr) -> Option<Symbol> {
    match place {
        Expr::Identifier(sym) => Some(*sym),
        Expr::FieldAccess { object, .. } => place_root(object),
        Expr::Index { collection, .. } => place_root(collection),
        _ => None,
    }
}

/// The place `stmt` changes in place, when it changes one: the list pushed to, popped from or set an item of,
/// the set added to or removed from, the record whose field is set, the replicated value changed or merged into.
pub(super) fn changed_place<'s, 'a>(stmt: &'s Stmt<'a>) -> Option<&'s Expr<'a>> {
    match stmt {
        Stmt::Push { collection, .. }
        | Stmt::Pop { collection, .. }
        | Stmt::Add { collection, .. }
        | Stmt::Remove { collection, .. }
        | Stmt::SetIndex { collection, .. } => Some(*collection),
        Stmt::AppendToSequence { sequence, .. } => Some(*sequence),
        Stmt::SetField { object, .. }
        | Stmt::ResolveConflict { object, .. }
        | Stmt::IncreaseCrdt { object, .. }
        | Stmt::DecreaseCrdt { object, .. } => Some(*object),
        Stmt::MergeCrdt { target, .. } | Stmt::JoinCrdt { target, .. } => Some(*target),
        _ => None,
    }
}

/// Each function's `mutable` parameter positions (`CContext::fn_inout`): an argument there is the caller's variable,
/// which the callee may change.
pub(super) type LentParams = HashMap<Symbol, Vec<bool>>;

/// The roots of what statement `s`'s own calls lend to a `mutable` parameter: bindings and places the callee may
/// change. A closure's calls happen when it is called, and a nested statement's are its own.
pub(super) fn stmt_lent_roots(s: &Stmt, lends: &LentParams, out: &mut HashSet<Symbol>) {
    fn lent(function: Symbol, args: &[&Expr], lends: &LentParams, out: &mut HashSet<Symbol>) {
        for (arg, &by_ref) in args.iter().zip(lends.get(&function).into_iter().flatten()) {
            if by_ref {
                out.extend(place_root(arg));
            }
        }
    }
    fn in_expr(e: &Expr, lends: &LentParams, out: &mut HashSet<Symbol>) {
        if matches!(e, Expr::Closure { .. }) {
            return;
        }
        if let Expr::Call { function, args } = e {
            lent(*function, args, lends, out);
        }
        for child in logicaffeine_language::ast_depth::expr_children(e) {
            in_expr(child, lends, out);
        }
    }
    if let Stmt::Call { function, args } = s {
        lent(*function, args, lends, out);
    }
    for e in logicaffeine_language::ast_depth::stmt_expr_children(s) {
        in_expr(e, lends, out);
    }
}

/// Every name `body` changes in place — a list pushed to, popped from or set an item of, a set added to or
/// removed from, a record's field set, a text appended to, a binding or place lent to a `mutable` parameter
/// (`lends`) — wherever it sits in the body's blocks. A nested function's body and a closure's are bodies of their
/// own.
pub(super) fn mutated_roots(body: &[Stmt], lends: &LentParams) -> HashSet<Symbol> {
    let mut out = HashSet::new();
    walk_nodes(body, |node| match node {
        Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => Walk::Past,
        Visited::Stmt(s) => {
            if let Some(root) = changed_place(s).and_then(place_root) {
                out.insert(root);
            }
            stmt_lent_roots(s, lends, &mut out);
            // `Set t to t + a + …`: a joining whose first part is the target itself appends to it in place.
            if let Stmt::Set { target, value } = s {
                let mut first = *value;
                while let Expr::BinaryOp {
                    op: BinaryOpKind::Add | BinaryOpKind::Concat,
                    left,
                    ..
                } = first
                {
                    first = left;
                }
                if !std::ptr::eq(first, *value)
                    && matches!(first, Expr::Identifier(sym) if sym == target)
                {
                    out.insert(*target);
                }
            }
            Walk::Into
        }
        Visited::Expr(_) => Walk::Into,
    });
    out
}

/// Whether binding or storing `value` of type `ty` under a name in `changed` — or while a name it may share
/// is in `changed` — needs a copy of its own.
pub(super) fn binding_needs_copy(
    value: &Expr,
    ty: &CType,
    target: Option<Symbol>,
    changed: &HashSet<Symbol>,
    ctx: &CContext,
) -> bool {
    if is_fresh(value) || !holds_buffer(ty, ctx.registry, ctx.interner) {
        return false;
    }
    let mut roots = HashSet::new();
    source_roots(value, &mut roots);
    target.is_some_and(|t| changed.contains(&t)) || roots.iter().any(|r| changed.contains(r))
}

/// Whether binding `value` of type `ty` to `target` at `stmt` needs a copy of its own: the value may share a
/// buffer with another name that is read after `stmt`, and a change can reach that buffer while a name other
/// than the one changing it still reads it ([`changes_are_seen`]). Without the body's sharing and liveness, any
/// binding a change could reach is copied.
pub(super) fn binding_copies(
    value: &Expr,
    ty: &CType,
    target: Symbol,
    stmt: &Stmt,
    ctx: &CContext,
) -> bool {
    if is_fresh(value) || !holds_buffer(ty, ctx.registry, ctx.interner) {
        return false;
    }
    let decided = ctx.ownership.as_deref().and_then(|own| {
        let after = own.live.after(stmt)?;
        let class = own.sharing.class_of(target);
        let shared_after = class
            .iter()
            .any(|m| *m != target && after.contains(m) && may_hold_buffer(*m, ctx));
        Some(own.sharing.changed(target) && shared_after && changes_are_seen(own, &class, ctx))
    });
    decided.unwrap_or_else(|| binding_needs_copy(value, ty, Some(target), &ctx.mutated, ctx))
}

/// Whether a change to a buffer the names `class` may share can be seen through another of them: some statement
/// changes a member's buffer in place while another member is read after it, or the class holds a program
/// binding a function changes. A change no other name reads again is the changer's own, and needs no copy
/// beforehand.
fn changes_are_seen(own: &super::types::Ownership, class: &[Symbol], ctx: &CContext) -> bool {
    if class.iter().any(|m| ctx.changed_statics.contains(m)) {
        return true;
    }
    let Some(first) = class.first() else {
        return false;
    };
    own.sharing
        .change_sites(*first)
        .iter()
        .any(|(key, changed)| match own.live.after_key(*key) {
            Some(after) => class
                .iter()
                .any(|m| m != changed && after.contains(m) && may_hold_buffer(*m, ctx)),
            None => true,
        })
}

/// Whether the argument `arg`, handed to a parameter its function changes in place, goes as a copy: some name
/// that may share its buffer is read again — after the statement making the call (the name it binds anew
/// excepted, unless a function reads that name too), or elsewhere in the statement. Otherwise the function takes
/// the buffer itself.
pub(super) fn argument_copies(arg: &Expr, ctx: &CContext) -> bool {
    if is_fresh(arg) {
        return false;
    }
    let Some(own) = ctx.ownership.as_deref() else {
        return true;
    };
    let frames = ctx.stmt_frames.borrow();
    let Some(frame) = frames.last() else {
        return true;
    };
    let Some(after) = own.live.after_key(frame.key) else {
        return true;
    };
    let mut roots = HashSet::new();
    source_roots_through(arg, &ctx.call_shapes, &mut roots);
    let inside = super::sharing::expr_addresses(arg);
    own.sharing
        .closure(&roots)
        .iter()
        .filter(|m| may_hold_buffer(**m, ctx))
        .any(|m| {
            let read_after =
                after.contains(m) && (frame.binds != Some(*m) || ctx.program_statics.contains(m));
            read_after
                || frame
                    .reads
                    .iter()
                    .any(|(at, s)| s == m && !inside.contains(at))
        })
}

/// Whether the name `m` may hold a list, map, set or record: its type, where the lowering has bound it by now,
/// and yes otherwise.
fn may_hold_buffer(m: Symbol, ctx: &CContext) -> bool {
    match ctx.vars.get(&m).or_else(|| ctx.static_types.get(&m)) {
        Some(ty) => holds_buffer(ty, ctx.registry, ctx.interner),
        None => true,
    }
}
