use std::fmt::Write;

use crate::ast::stmt::*;
use crate::intern::{Interner, Symbol};

use super::types::{
    c_param_decl, c_type_str, c_type_str_resolved, escape_c_ident, infer_expr_type,
    int_exceeds_i64, map_ctype, map_new, resolve_type_expr, resolve_type_expr_with_registry,
    seq_fn, seq_of, seq_struct_fn, seq_struct_type, tuple_c_name, CContext, CType, Ints,
};

/// Resolve a quantity unit NAME via the base `units::by_name` at CODEGEN time, rendering the C
/// fragments for its exact scale/offset (`rat_ratio(n, d)`), its 10-axis dimension vector
/// (`(const int[]){…}` numerators & denominators), and its symbol string. The unit catalog stays
/// single-sourced in Rust; the C runtime only carries the dimensional algebra. `None` on unknown name.
fn resolve_quantity_unit(name: &str) -> Option<(String, String, String, String, String)> {
    let unit = logicaffeine_base::quantity::units::by_name(name)?;
    let ratio = |s: String| -> (i64, i64) {
        match s.split_once('/') {
            Some((n, d)) => (n.parse().unwrap(), d.parse().unwrap()),
            None => (s.parse().unwrap(), 1),
        }
    };
    let (sn, sd) = ratio(unit.scale.to_string());
    let (on, od) = ratio(unit.offset.to_string());
    let (mut nums, mut dens) = (Vec::new(), Vec::new());
    for d in logicaffeine_base::dimension::BaseDim::ALL {
        let es = unit.dimension.exponent(d).to_string();
        let (n, dd): (i32, i32) = match es.split_once('/') {
            Some((n, dd)) => (n.parse().unwrap(), dd.parse().unwrap()),
            None => (es.parse().unwrap(), 1),
        };
        nums.push(n.to_string());
        dens.push(dd.to_string());
    }
    let sym = format!(
        "\"{}\"",
        unit.symbol.replace('\\', "\\\\").replace('"', "\\\"")
    );
    Some((
        format!("rat_ratio({}, {})", sn, sd),
        format!("rat_ratio({}, {})", on, od),
        format!("(const int[]){{{}}}", nums.join(", ")),
        format!("(const int[]){{{}}}", dens.join(", ")),
        sym,
    ))
}

/// The unit name of a `quantity`/`convert` second argument, if it is a compile-time string literal.
fn quantity_unit_literal(name_expr: &Expr, ctx: &CContext) -> Option<String> {
    match name_expr {
        Expr::Literal(Literal::Text(sym)) => Some(ctx.interner.resolve(*sym).to_string()),
        _ => None,
    }
}

/// The variable `expr` names when it holds a value-or-nothing (`CType::Opt` — what a receive, a select
/// arm or an `Await … into` binds), with that type. Read as an expression it is its element
/// (`logos_opt_int` unpacked, which fails the task on `nothing`, as arithmetic on `nothing` fails on the
/// VM); the operations that are total over `nothing` — showing it, rendering it into text, copying it,
/// comparing it with `is` — ask this first and take the whole value.
fn opt_var(expr: &Expr, ctx: &CContext) -> Option<(String, CType)> {
    if ctx
        .precomputed
        .borrow()
        .contains_key(&(expr as *const Expr as usize))
    {
        return None;
    }
    match expr {
        Expr::Identifier(sym) if !matches!(ctx.interner.resolve(*sym), "now" | "today") => {
            match ctx.vars.get(sym) {
                Some(ty @ CType::Opt(_)) => Some((ctx.resolve(*sym), ty.clone())),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The element a value-or-nothing type carries.
pub(super) fn opt_element(ty: &CType) -> CType {
    match ty {
        CType::Opt(inner) => (**inner).clone(),
        _ => CType::Int64,
    }
}

/// A collection an operation reads or changes, as the operation sees it.
struct CollectionOperand {
    /// Its C lvalue: the variable, or the collection a value-or-nothing variable holds.
    place: String,
    /// Its C type: for a value-or-nothing variable, that of the collection it holds.
    ty: Option<CType>,
    /// For a value-or-nothing variable, the statement that fails the run when it holds nothing, in the words the VM
    /// gives the operation on Nothing.
    absent: Option<String>,
}

impl CollectionOperand {
    /// The C expression `value`, which reads this operand, behind its check for nothing.
    fn read(&self, value: String) -> String {
        match &self.absent {
            None => value,
            Some(check) => format!("({{ {check} {value}; }})"),
        }
    }

    /// The C statement `act`, which reads or changes this operand, behind its check for nothing.
    fn act(&self, act: String) -> String {
        match &self.absent {
            None => act,
            Some(check) => format!("{check} {act}"),
        }
    }
}

/// Whether `collection` is a place an operation reads through [`collection_operand`]: a variable, or a field (of a
/// field) that holds a value-or-nothing.
fn operand_place(collection: &Expr, ctx: &CContext) -> bool {
    match collection {
        Expr::Identifier(sym) => ctx.vars.contains_key(sym) || ctx.program_statics.contains(sym),
        Expr::FieldAccess { .. } => matches!(infer_expr_type(collection, ctx), CType::Opt(_)),
        _ => false,
    }
}

/// The C lvalue of `item`, an item of a list (of an item of a list, …) of a variable, with its C type: the item in the
/// list's own buffer, its position checked as every read of an item is.
fn seq_item_place(item: &Expr, ctx: &CContext) -> Option<(String, CType)> {
    let Expr::Index { collection, index } = item else {
        return None;
    };
    let (parent, parent_ty) = match collection {
        Expr::Identifier(sym) if operand_place(collection, ctx) => {
            (ctx.resolve(*sym).to_string(), ctx.vars.get(sym).cloned()?)
        }
        Expr::Index { .. } => seq_item_place(collection, ctx)?,
        _ => return None,
    };
    if !super::types::is_seq_ctype(&parent_ty) {
        return None;
    }
    let item_ty = super::types::seq_element(&parent_ty, ctx.registry)?;
    let k = narrow_index(
        index,
        codegen_expr(index, ctx),
        ctx,
        crate::semantics::failcode::FailCode::ListIndexTooWide,
    );
    Some((format!("({parent}).data[logos_idx0({k}, ({parent}).len)]"), item_ty))
}

/// Whether `collection` is a variable or a field that holds a value-or-nothing.
fn opt_place(collection: &Expr, ctx: &CContext) -> bool {
    match collection {
        Expr::Identifier(sym) => matches!(ctx.vars.get(sym), Some(CType::Opt(_))),
        _ => operand_place(collection, ctx),
    }
}

/// `collection`, a variable or a field ([`operand_place`]), as the collection an operation reads or changes. One that
/// holds a value-or-nothing is the collection it holds, the operation failing first, when it holds none, in the words
/// `on_nothing` gives: the shared semantics' for the operation on Nothing.
fn collection_operand(
    collection: &Expr,
    on_nothing: impl FnOnce() -> Result<(), String>,
    ctx: &CContext,
) -> CollectionOperand {
    let (at, ty) = match collection {
        Expr::Identifier(sym) => (ctx.resolve(*sym).to_string(), ctx.vars.get(sym).cloned()),
        _ => (codegen_expr(collection, ctx), Some(infer_expr_type(collection, ctx))),
    };
    match ty {
        Some(CType::Opt(inner)) => {
            let absent = match on_nothing() {
                Err(words) => format!(
                    "if (!({at}).present) {{ {} }}",
                    ctx.fail_stmt(&c_string_literal(&words))
                ),
                Ok(()) => c_refuse_expr("an operation that the VM performs on nothing"),
            };
            CollectionOperand {
                place: super::types::opt_unpack(&inner, &format!("({at}).value"), ctx.interner),
                ty: Some(*inner),
                absent: Some(absent),
            }
        }
        ty => CollectionOperand {
            place: at,
            ty,
            absent: None,
        },
    }
}

/// `expr` as a value-or-nothing (`logos_opt`): an optional variable whole, `nothing` absent, any other
/// value present, in its scalar packing.
fn codegen_opt(expr: &Expr, ctx: &CContext) -> String {
    if let Some((var, _)) = opt_var(expr, ctx) {
        return var;
    }
    match expr {
        Expr::Literal(Literal::Nothing) => "((logos_opt){0, 0})".to_string(),
        _ if matches!(infer_expr_type(expr, ctx), CType::Opt(_)) => codegen_expr(expr, ctx),
        _ => {
            let value = codegen_expr(expr, ctx);
            format!(
                "((logos_opt){{1, {}}})",
                opt_pack(&infer_expr_type(expr, ctx), &value, ctx)
            )
        }
    }
}

/// `value` as the value-or-nothing a `Let` or a `Set` stores into `target` (of type `ty`): whole when it is a
/// value-or-nothing itself, else present; and a copy of its own wherever the binding must not share a buffer with it.
fn opt_stored(value: &Expr, ty: &CType, target: Symbol, stmt: &Stmt, ctx: &CContext) -> String {
    let copies = |of: &CType| super::values::binding_copies(value, of, target, stmt, ctx);
    if matches!(value, Expr::Literal(Literal::Nothing))
        || opt_var(value, ctx).is_some()
        || matches!(infer_expr_type(value, ctx), CType::Opt(_))
    {
        let whole = codegen_opt(value, ctx);
        return if copies(ty) {
            super::values::copy_value(ty, &whole, ctx)
        } else {
            whole
        };
    }
    // A present value is the target's element: an integer an exact one where the element is, a number a fraction
    // where the element is one, as a binding of the element's type takes it.
    let element = opt_element(ty);
    let value_ty = infer_expr_type(value, ctx);
    let plain = match (&element, &value_ty) {
        (CType::BigInt, from) if *from != CType::BigInt => {
            format!("lint_of((int64_t)({}))", codegen_expr(value, ctx))
        }
        (CType::Rational | CType::Decimal, from) if !matches!(from, CType::Rational | CType::Decimal) => {
            codegen_expr_as_rational(value, ctx)
        }
        _ => codegen_expr(value, ctx),
    };
    let plain = if copies(&element) {
        super::values::copy_value(&element, &plain, ctx)
    } else {
        plain
    };
    format!("((logos_opt){{1, {}}})", opt_pack(&element, &plain, ctx))
}

/// `value` made as a value of `ty`, a type holding values-or-nothing: a value-or-nothing as [`opt_item`] makes it, a
/// list written out item by item, each made as the list's item, and a new empty collection as a collection of `ty`.
fn value_as(value: &Expr, ty: &CType, ctx: &CContext) -> String {
    match (value, ty) {
        (_, CType::Opt(_)) => opt_item(value, ty, ctx),
        (Expr::List(items), seq) if !items.is_empty() && super::absent::holds_nothing(seq) => {
            match super::types::seq_element(seq, ctx.registry) {
                Some(item) => literal_as(items, seq, &item, ctx),
                None => codegen_expr(value, ctx),
            }
        }
        _ => fresh_collection(value, ty, ctx).unwrap_or_else(|| stored(value, codegen_expr(value, ctx), ctx)),
    }
}

/// `value`, a new empty collection (`a new Seq of …`, `a new Map of …`), made as a collection of `ty`, when `ty`
/// holds values-or-nothing where the collection's written type holds plain ones.
fn fresh_collection(value: &Expr, ty: &CType, ctx: &CContext) -> Option<String> {
    let Expr::New { type_name, init_fields, .. } = value else {
        return None;
    };
    if !init_fields.is_empty()
        || !matches!(ctx.interner.resolve(*type_name), "Seq" | "List" | "Map")
        || !super::absent::holds_nothing(ty)
    {
        return None;
    }
    let made = match ty {
        CType::MapStrValue(_) | CType::MapI64Value(_) => map_new(ty, ctx.interner)?,
        seq => format!("{}()", seq_fn(seq, "new", ctx.interner)?),
    };
    Some(made)
}

/// `value` as an item of a list or a map whose items are values-or-nothing of type `ty`: absent for `nothing`, whole
/// when it is a value-or-nothing itself, else present as the item's payload takes it; a copy of its own, as every
/// stored item is.
fn opt_item(value: &Expr, ty: &CType, ctx: &CContext) -> String {
    if matches!(value, Expr::Literal(Literal::Nothing)) {
        return "((logos_opt){0, 0})".to_string();
    }
    if opt_var(value, ctx).is_some() || matches!(infer_expr_type(value, ctx), CType::Opt(_)) {
        let whole = codegen_opt(value, ctx);
        return if super::values::is_fresh(value) {
            whole
        } else {
            super::values::copy_value(ty, &whole, ctx)
        };
    }
    let element = opt_element(ty);
    let plain = match (&element, infer_expr_type(value, ctx)) {
        (CType::BigInt, from) if from != CType::BigInt => {
            format!("lint_of((int64_t)({}))", codegen_expr(value, ctx))
        }
        (CType::Rational | CType::Decimal, from) if !matches!(from, CType::Rational | CType::Decimal) => {
            codegen_expr_as_rational(value, ctx)
        }
        _ => stored(value, codegen_expr(value, ctx), ctx),
    };
    format!("((logos_opt){{1, {}}})", opt_pack(&element, &plain, ctx))
}

/// `value`, of element type `elem`, in a value-or-nothing's scalar cell: itself when the cell holds it, else
/// the address of a copy of its own.
fn opt_pack(elem: &CType, value: &str, ctx: &CContext) -> String {
    if super::types::opt_holds_in_cell(elem) {
        task_arg_pack(elem, value).unwrap_or_else(|| format!("(long long)({value})"))
    } else {
        let tn = c_type_str_resolved(elem, ctx.interner);
        format!("((long long)(intptr_t)({{ {tn} *__box = ({tn} *)lc_gc_alloc(sizeof({tn})); *__box = {value}; __box; }}))")
    }
}

/// The element a pipe expression carries — `Int` for a handle whose element the C tier does not know.
pub(super) fn pipe_element(pipe: &Expr, ctx: &CContext) -> CType {
    match infer_expr_type(pipe, ctx) {
        CType::Pipe(element) => *element,
        _ => CType::Int64,
    }
}

/// Emit `expr` in RATIONAL context: every `+ − × ÷` node becomes `LDec` arithmetic, computing what the VM
/// computes on exact numbers — an exact quotient (`ExactDivide`, `1 / 4` where a rational is expected) is the
/// fraction, and a plain `/` truncates two whole values and divides anything else exactly (`ldec_quotient`), a
/// decimal dividing exactly. An already-`Rational`/`Decimal` operand is used as-is; an integer promotes exactly, a
/// machine word via `ldec_from_i64` and an `LInt` via `ldec_from_lint`. Used for a `: Rational`-annotated binding,
/// for the operands of any Rational-typed arithmetic/comparison, and for an exact quotient of integers.
pub(super) fn codegen_expr_as_rational(expr: &Expr, ctx: &CContext) -> String {
    if let Expr::BinaryOp { op, left, right } = expr {
        let decimal = [left, right]
            .iter()
            .any(|e| infer_expr_type(e, ctx) == CType::Decimal);
        if let Some(f) = match op {
            BinaryOpKind::Add => Some("ldec_add"),
            BinaryOpKind::Subtract => Some("ldec_sub"),
            BinaryOpKind::Multiply => Some("ldec_mul"),
            BinaryOpKind::Divide if !decimal => Some("ldec_quotient"),
            BinaryOpKind::Divide | BinaryOpKind::ExactDivide => Some("ldec_div"),
            _ => None,
        } {
            return format!(
                "{}({}, {})",
                f,
                codegen_expr_as_rational(left, ctx),
                codegen_expr_as_rational(right, ctx)
            );
        }
    }
    match infer_expr_type(expr, ctx) {
        CType::Rational | CType::Decimal => codegen_expr(expr, ctx),
        CType::BigInt => format!("ldec_from_lint({})", codegen_expr(expr, ctx)),
        _ => format!("ldec_from_i64((int64_t)({}))", codegen_expr(expr, ctx)),
    }
}

/// `left op right` where an operand is an exact `LDec` (a rational or a decimal), as the VM computes it: the
/// rational rules of the runtime's `ldec_*` over two exact operands, and the real one's beside a real. `None` for
/// an operation it does not take, which the general lowering then takes as it always has.
fn exact_operation(
    op: BinaryOpKind,
    left: &Expr,
    right: &Expr,
    lt: &CType,
    rt: &CType,
    ctx: &CContext,
) -> Option<String> {
    let ldec = |t: &CType| matches!(t, CType::Rational | CType::Decimal);
    if !ldec(lt) && !ldec(rt) {
        return None;
    }
    let decimal = *lt == CType::Decimal || *rt == CType::Decimal;
    let relation = match op {
        BinaryOpKind::Eq => Some("=="),
        BinaryOpKind::NotEq => Some("!="),
        BinaryOpKind::Lt => Some("<"),
        BinaryOpKind::LtEq => Some("<="),
        BinaryOpKind::Gt => Some(">"),
        BinaryOpKind::GtEq => Some(">="),
        _ => None,
    };
    if *lt == CType::Float64 || *rt == CType::Float64 {
        let double = |e: &Expr, t: &CType| match t {
            CType::Rational | CType::Decimal => format!("ldec_to_f64({})", codegen_expr(e, ctx)),
            CType::BigInt => format!("lint_to_f64({})", codegen_expr(e, ctx)),
            _ => format!("(double)({})", codegen_expr(e, ctx)),
        };
        let (l, r) = (double(left, lt), double(right, rt));
        if let Some(rel) = relation {
            // An exact number orders against a real by the real's exact value; a NaN orders against nothing.
            let (x, f, sign) = if ldec(lt) {
                (codegen_expr(left, ctx), r, "")
            } else {
                (codegen_expr(right, ctx), l, "-")
            };
            return Some(match op {
                BinaryOpKind::Eq => format!("(ldec_cmp_f64({x}, {f}) == 0)"),
                BinaryOpKind::NotEq => format!("(ldec_cmp_f64({x}, {f}) != 0)"),
                _ => format!("({{ int __c = ldec_cmp_f64({x}, {f}); __c != 2 && ({sign}__c) {rel} 0; }})"),
            });
        }
        return Some(match op {
            BinaryOpKind::Add => format!("({l} + {r})"),
            BinaryOpKind::Subtract => format!("({l} - {r})"),
            BinaryOpKind::Multiply => format!("({l} * {r})"),
            BinaryOpKind::Divide | BinaryOpKind::ExactDivide => format!("logos_rdiv({l}, {r})"),
            BinaryOpKind::FloorDivide => format!("floor(logos_rdiv({l}, {r}))"),
            BinaryOpKind::Pow => format!("pow({l}, {r})"),
            BinaryOpKind::Modulo => {
                // No remainder of a real: refused in the VM's words for the kinds the two values are, a whole
                // rational being the integer it equals.
                let refusal = |whole: bool| -> Option<String> {
                    let side = |t: &CType| -> Option<crate::interpreter::RuntimeValue> {
                        match t {
                            CType::Rational if !whole => Some(crate::interpreter::RuntimeValue::from_rational(
                                logicaffeine_base::Rational::from_ratio_i64(1, 2)?,
                            )),
                            CType::Rational => sample_of(&CType::Int64, 3),
                            CType::Decimal => Some(crate::interpreter::RuntimeValue::Decimal(std::rc::Rc::new(
                                logicaffeine_base::Decimal::from_i64(3),
                            ))),
                            other => sample_of(other, 3),
                        }
                    };
                    crate::semantics::arith::modulo(side(lt)?, side(rt)?).err().map(|w| c_string_literal(&w))
                };
                let (whole, fraction) = (refusal(true)?, refusal(false)?);
                let held = if ldec(lt) { left } else { right };
                format!(
                    "({{ LDec __x = {}; {} 0.0; }})",
                    codegen_expr(held, ctx),
                    ctx.fail_stmt(&format!("(ldec_whole(&__x) ? {whole} : {fraction})"))
                )
            }
            _ => return None,
        });
    }
    let (lc, rc) = (codegen_expr_as_rational(left, ctx), codegen_expr_as_rational(right, ctx));
    if let Some(rel) = relation {
        return Some(format!("(ldec_cmp({lc}, {rc}) {rel} 0)"));
    }
    let f = match op {
        BinaryOpKind::Add => "ldec_add",
        BinaryOpKind::Subtract => "ldec_sub",
        BinaryOpKind::Multiply => "ldec_mul",
        BinaryOpKind::Divide if !decimal => "ldec_quotient",
        BinaryOpKind::Divide | BinaryOpKind::ExactDivide => "ldec_div",
        BinaryOpKind::FloorDivide => "ldec_floor_quotient",
        BinaryOpKind::Modulo if !decimal => "ldec_remainder",
        BinaryOpKind::Pow if !decimal => "ldec_power",
        BinaryOpKind::BitAnd | BinaryOpKind::BitOr | BinaryOpKind::BitXor | BinaryOpKind::Shl | BinaryOpKind::Shr
            if !decimal =>
        {
            let symbol = match op {
                BinaryOpKind::BitAnd => '&',
                BinaryOpKind::BitOr => '|',
                BinaryOpKind::BitXor => '^',
                BinaryOpKind::Shl => '<',
                _ => '>',
            };
            return Some(format!("ldec_bitwise({lc}, {rc}, '{symbol}')"));
        }
        _ => return None,
    };
    Some(format!("{f}({lc}, {rc})"))
}

/// Whether `ty` is a sequence the C tier holds as a `Seq_*` family.
fn is_sequence(ty: &CType) -> bool {
    matches!(
        ty,
        CType::SeqI64(_) | CType::SeqBool | CType::SeqStr | CType::SeqF64 | CType::SeqValue(_) | CType::SeqSeq(_)
    )
}

/// Narrow an exact index to the machine word it fits: an `LInt`, failing past a word with the reference tier's refusal
/// of such an index (`why`'s sentence, `semantics::failcode::message`), or a rational, whole and within a word, else
/// refused in the VM's words for the place. A no-op for any other index.
pub(super) fn narrow_index(
    expr: &Expr,
    rendered: String,
    ctx: &CContext,
    why: crate::semantics::failcode::FailCode,
) -> String {
    let place = {
        use super::words::IntegerPlace;
        use crate::semantics::failcode::FailCode;
        match why {
            FailCode::TextIndexTooWide => IntegerPlace::Text,
            FailCode::ListStoreIndexTooWide => IntegerPlace::ListStore,
            _ => IntegerPlace::ListRead,
        }
    };
    if infer_expr_type(expr, ctx) == CType::Tower {
        return super::tower::as_int_at(rendered, place);
    }
    if infer_expr_type(expr, ctx) == CType::Rational {
        return format!(
            "ldec_as_i64({rendered}, {}, {})",
            super::words::integer_past_a_word(place),
            super::words::fraction_for_an_integer(place)
        );
    }
    if infer_expr_type(expr, ctx) == CType::BigInt {
        let refusal = crate::semantics::failcode::message(why.code(), 0);
        format!(
            "lint_index_i64({}, {})",
            rendered,
            c_string_literal(&refusal)
        )
    } else {
        rendered
    }
}

/// A range bound as the machine word the range counts in ([`integer_at_place`]).
fn range_bound(bound: &Expr, ctx: &CContext) -> String {
    integer_at_place(bound, super::words::IntegerPlace::Range, ctx)
}

/// The machine integer `expr` is where the VM takes one at `place` (a bound): a whole rational or a value of the
/// extension the integer it equals, a fraction or one past a word refused in the VM's words for the place; an exact
/// integer narrowed into a word.
fn integer_at_place(expr: &Expr, place: super::words::IntegerPlace, ctx: &CContext) -> String {
    match infer_expr_type(expr, ctx) {
        CType::Tower => super::tower::as_int_at(codegen_expr(expr, ctx), place),
        CType::Rational => format!(
            "ldec_as_i64({}, {}, {})",
            codegen_expr(expr, ctx),
            super::words::integer_past_a_word(place),
            super::words::fraction_for_an_integer(place)
        ),
        _ => narrow_i64_sink(expr, codegen_expr(expr, ctx), ctx),
    }
}

/// Narrow a bignum-promoted integer to `int64_t` for an int64 sink (a `Seq`/`Map` element, index, or key):
/// the sink's element type is i64, so the value provably fits, and `lint_expect_i64` extracts it. A no-op
/// unless the value is `LInt` (`CType::BigInt`) — so it never touches a genuine i64 / string / float operand.
/// This closes the class of `incompatible type for argument … of seq_i64_{push,get,set}` C errors that arise
/// when the bignum pass promotes an operand (`a / b % c`) whose result flows into an i64-element collection.
pub(super) fn narrow_i64_sink(expr: &Expr, rendered: String, ctx: &CContext) -> String {
    if infer_expr_type(expr, ctx) == CType::Tower {
        return super::tower::as_int(rendered);
    }
    if infer_expr_type(expr, ctx) == CType::BigInt {
        format!("lint_expect_i64({})", rendered)
    } else {
        rendered
    }
}

/// `rendered`, the C value of `value`, as field `field` of `owner` (a record, or an enum variant) holds it: a
/// machine-word integer stored into a field the program made exact (`LInt`, see `exact`) is promoted, and an
/// exact value stored into a proven machine-word field is narrowed at that typed boundary.
fn as_field_value(
    rendered: String,
    value: &Expr,
    owner: Symbol,
    field: Option<&crate::analysis::FieldDef>,
    ctx: &CContext,
) -> String {
    let held = field.map(|fd| {
        super::exact::field_ctype(owner, fd, ctx.interner, ctx.registry, &ctx.field_facts())
    });
    match (held, infer_expr_type(value, ctx)) {
        // A field a stored `nothing` reaches holds a value-or-nothing, or a list of them.
        (Some(held @ CType::Opt(_)), _) => opt_item(value, &held, ctx),
        (Some(seq @ CType::SeqValue(_)), _) if matches!(value, Expr::List(items) if !items.is_empty()) => {
            let (Expr::List(items), CType::SeqValue(item)) = (value, &seq) else {
                unreachable!("matched above")
            };
            literal_as(items, &seq, item, ctx)
        }
        (Some(CType::Tower), _) => super::tower::stored(value, ctx),
        // A rational field holds an integer as the rational it equals.
        (Some(CType::Rational), CType::Int64) => format!("ldec_from_i64((int64_t)({}))", rendered),
        (Some(CType::Rational), CType::BigInt) => format!("ldec_from_lint({})", rendered),
        (Some(CType::BigInt), from) if from != CType::BigInt => {
            format!("lint_of((int64_t)({}))", rendered)
        }
        (Some(CType::Int64), CType::BigInt) => format!("lint_expect_i64({})", rendered),
        _ => rendered,
    }
}

/// `x & c` (a non-negative literal `c`) or `x % c` (a nonzero literal `c`) over integers, as the VM computes it:
/// an exact `x` brought back within a word by the operator, or refused in the VM's words; a sum, difference or
/// product of machine words not proven to stay within one (see `word_ranges`) computed with every step checked,
/// and exactly from the first step that leaves the word. `None` for any other operation, and for a word operand
/// the plain word operator computes exactly.
fn bounded_operation(
    expr: &Expr,
    op: BinaryOpKind,
    left: &Expr,
    right: &Expr,
    lt: &CType,
    rt: &CType,
    ctx: &CContext,
) -> Option<String> {
    super::types::bounded_int(expr)?;
    let literal = |e: &Expr| match e {
        Expr::Literal(Literal::Number(c)) => Some(*c),
        _ => None,
    };
    // The operand the operator bounds, and the literal bounding it: a mask on either side of `&`, the divisor of `%`.
    let (x, xt, c) = match (
        op,
        literal(right).filter(|c| *c >= 0 || op == BinaryOpKind::Modulo),
    ) {
        (_, Some(c)) => (left, lt, c),
        (_, None) => (right, rt, literal(left)?),
    };
    let exact = |value: &str| match op {
        BinaryOpKind::BitAnd => {
            // A negative big integer is one no mask takes: the VM's words for it are the words for them all.
            let negative_big = logicaffeine_base::BigInt::from_i64(i64::MIN)
                .mul(&logicaffeine_base::BigInt::from_i64(2));
            let refusal = crate::semantics::arith::binary_op(
                BinaryOpKind::BitAnd,
                crate::interpreter::RuntimeValue::from_bigint(negative_big),
                crate::interpreter::RuntimeValue::Int(c),
            )
            .err()
            .unwrap_or_default();
            format!(
                "logos_exact_band({value}, {c}LL, {})",
                c_string_literal(&refusal)
            )
        }
        _ => format!("lint_expect_i64(lint_mod({value}, lint_of((int64_t)({c}LL))))"),
    };
    let word = |value: &str| match op {
        BinaryOpKind::BitAnd => format!("(({value}) & {c}LL)"),
        _ => format!("logos_imod((int64_t)({value}), (int64_t)({c}LL))"),
    };
    // A chain of machine words is computed checked (or proven), whatever exactness its own type would take.
    let xt = if is_word_chain(x, ctx) {
        &CType::Int64
    } else {
        xt
    };
    match xt {
        CType::BigInt => Some(exact(&codegen_expr(x, ctx))),
        CType::Int64 => {
            if !is_word_chain(x, ctx) {
                return None;
            }
            if proven_in_word(x, ctx) {
                return Some(word(&raw_word_chain(x, ctx)));
            }
            let n = ctx.select_ctr.get();
            ctx.select_ctr.set(n + 1);
            let mut cells = String::new();
            let mut leaves = 0usize;
            let checked = word_chain(x, n, &mut cells, &mut leaves, ctx, true);
            let mut ignored = String::new();
            let mut again = 0usize;
            let exact_chain = word_chain(x, n, &mut ignored, &mut again, ctx, false);
            Some(format!(
                "({{ {cells}int __over{n} = 0; int64_t __word{n} = {checked}; __builtin_expect(__over{n}, 0) ? {} : {}; }})",
                exact(&exact_chain),
                word(&format!("__word{n}"))
            ))
        }
        _ => None,
    }
}

/// Whether the body's word ranges (or, outside a body's ownership plan, the range oracle) prove `x` stays within a
/// machine word.
fn proven_in_word(x: &Expr, ctx: &CContext) -> bool {
    super::word_ranges::fits_word(int_range(x, ctx))
}

/// The range of `x`'s value: what the body's word ranges (where its ownership plan holds them), the range oracle and
/// the body's non-negativity facts each prove, together. Each is a sound bound on its own, so their intersection is.
fn int_range(x: &Expr, ctx: &CContext) -> super::word_ranges::Range {
    let words = ctx
        .ownership
        .as_deref()
        .and_then(|own| own.words.of_expr(x, ctx.int_oracle.as_deref()));
    let oracle = ctx
        .int_oracle
        .as_deref()
        .and_then(|o| o.expr_int_range(x))
        .map(|(lo, hi)| (i128::from(lo), i128::from(hi)));
    let (lo, hi) = match (words, oracle) {
        (Some((a, b)), Some((c, d))) if a.max(c) <= b.min(d) => (a.max(c), b.min(d)),
        (Some(r), _) | (None, Some(r)) => r,
        (None, None) => return None,
    };
    let lo = if lo < 0 && crate::codegen::expr::expr_is_nonnegative(x, &ctx.nonnegative_vars) {
        0
    } else {
        lo
    };
    Some((lo, hi.max(lo)))
}

/// What the C emitter proves about the integers it emits, for [`crate::codegen::lowbits`].
struct CWords<'c, 'a> {
    ctx: &'c CContext<'a>,
}

impl crate::codegen::lowbits::WordProof for CWords<'_, '_> {
    fn stays_in_word(&self, node: &Expr) -> bool {
        proven_in_word(node, self.ctx)
    }
    fn is_word(&self, operand: &Expr) -> bool {
        infer_expr_type(operand, self.ctx) == CType::Int64
    }
    fn is_nonnegative(&self, value: &Expr) -> bool {
        crate::codegen::expr::expr_is_nonnegative(value, &self.ctx.nonnegative_vars)
    }
}

/// Opt::MaskNarrow: `expr` computed in `uint32_t` arithmetic under its low mask, when it is an integer chain under
/// one (see [`crate::codegen::lowbits`]); unsigned 32-bit arithmetic wraps, so the narrowed form is defined C.
fn mask_narrow(expr: &Expr, ctx: &CContext) -> Option<String> {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::MaskNarrow) {
        return None;
    }
    let masked = crate::codegen::lowbits::masked_chain(expr, &CWords { ctx })?;
    crate::optimize::mark_fired(crate::optimization::Opt::MaskNarrow);
    Some(format!(
        "((int64_t)({} & {}u))",
        c_low_bits(&masked.chain, ctx),
        masked.mask
    ))
}

fn c_low_bits(chain: &crate::codegen::lowbits::LowBits, ctx: &CContext) -> String {
    use crate::codegen::lowbits::{LowBits, LowOp};
    match chain {
        LowBits::Word(e) => format!("((uint32_t)({}))", codegen_expr(e, ctx)),
        LowBits::Const(c) => format!("{c}u"),
        LowBits::Shl(c, s) => format!("({} << {s})", c_low_bits(c, ctx)),
        LowBits::Op(op, l, r) => {
            let symbol = match op {
                LowOp::Add => "+",
                LowOp::Sub => "-",
                LowOp::Mul => "*",
                LowOp::And => "&",
                LowOp::Or => "|",
                LowOp::Xor => "^",
            };
            format!("({} {symbol} {})", c_low_bits(l, ctx), c_low_bits(r, ctx))
        }
    }
}

/// Whether `expr` is a sum, difference or product whose leaves (see [`word_chain`]) are machine words.
fn is_word_chain(expr: &Expr, ctx: &CContext) -> bool {
    fn leaves_are_words(e: &Expr, ctx: &CContext) -> bool {
        match e {
            Expr::BinaryOp {
                op: BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply,
                left,
                right,
            } => leaves_are_words(left, ctx) && leaves_are_words(right, ctx),
            leaf => infer_expr_type(leaf, ctx) == CType::Int64,
        }
    }
    matches!(
        expr,
        Expr::BinaryOp {
            op: BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply,
            ..
        }
    ) && leaves_are_words(expr, ctx)
}

/// The C of the machine-word chain `expr` in plain machine-word operators, for a chain proven to stay within a word.
fn raw_word_chain(expr: &Expr, ctx: &CContext) -> String {
    match expr {
        Expr::BinaryOp {
            op: op @ (BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply),
            left,
            right,
        } => {
            let symbol = match op {
                BinaryOpKind::Add => "+",
                BinaryOpKind::Subtract => "-",
                _ => "*",
            };
            format!(
                "({} {symbol} {})",
                raw_word_chain(left, ctx),
                raw_word_chain(right, ctx)
            )
        }
        leaf => codegen_expr(leaf, ctx),
    }
}

/// The C of the machine-word chain `expr` over cells holding its leaves (the operands that are not themselves
/// sums, differences or products), each computed once, left to right, into `cells`: with every step checked into
/// `__over{n}` when `checked`, and as an exact `LInt` otherwise.
fn word_chain(
    expr: &Expr,
    n: usize,
    cells: &mut String,
    leaves: &mut usize,
    ctx: &CContext,
    checked: bool,
) -> String {
    match expr {
        Expr::BinaryOp {
            op: op @ (BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply),
            left,
            right,
        } => {
            let l = word_chain(left, n, cells, leaves, ctx, checked);
            let r = word_chain(right, n, cells, leaves, ctx, checked);
            let (ck, ex) = match op {
                BinaryOpKind::Add => ("logos_ck_add", "lint_add"),
                BinaryOpKind::Subtract => ("logos_ck_sub", "lint_sub"),
                _ => ("logos_ck_mul", "lint_mul"),
            };
            if checked {
                format!("{ck}({l}, {r}, &__over{n})")
            } else {
                format!("{ex}({l}, {r})")
            }
        }
        leaf => {
            let i = *leaves;
            *leaves += 1;
            let cell = format!("__leaf{n}_{i}");
            if checked {
                write!(cells, "int64_t {cell} = {}; ", codegen_expr(leaf, ctx)).unwrap();
                cell
            } else {
                format!("lint_of({cell})")
            }
        }
    }
}

/// Every list the counted loop `stmt` fills — a push on each pass, the passes fixed by its bounds (the length
/// analysis the Rust tier sizes its vectors by, `counted_push_loop`) — sized before the loop for all it will
/// push, so it grows in one allocation instead of by doubling. A loop that may leave early, or whose bound may
/// act when computed again, sizes nothing.
fn reserve_counted_fills(
    stmt: &Stmt,
    cond: &Expr,
    body: &[Stmt],
    ctx: &CContext,
    output: &mut String,
    pad: &str,
    native_counter: Option<(Symbol, &str)>,
) {
    if super::sharing::may_leave(stmt) {
        return;
    }
    let mut pushed: Vec<Symbol> = Vec::new();
    logicaffeine_language::ast_depth::walk_nodes(body, |node| {
        use logicaffeine_language::ast_depth::{Visited, Walk};
        match node {
            Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => {
                Walk::Past
            }
            Visited::Stmt(Stmt::Push {
                collection: Expr::Identifier(sym),
                ..
            }) => {
                if !pushed.contains(sym) {
                    pushed.push(*sym);
                }
                Walk::Into
            }
            _ => Walk::Into,
        }
    });
    for sym in pushed {
        let is_list = matches!(
            ctx.vars.get(&sym),
            Some(
                CType::SeqI64(_)
                    | CType::SeqBool
                    | CType::SeqStr
                    | CType::SeqF64
                    | CType::SeqStruct(_)
                    | CType::SeqTuple(_)
                    | CType::SeqSeq(_)
                    | CType::SeqValue(_)
            )
        );
        let Some(fill) =
            crate::codegen::peephole::counted_push_loop(cond, body, sym).filter(|_| is_list)
        else {
            continue;
        };
        let words = |e: &Expr| infer_expr_type(e, ctx) == CType::Int64 && !may_act(e, ctx);
        let counter = Expr::Identifier(fill.counter);
        let counter_is_word =
            words(&counter) || native_counter.is_some_and(|(sym, _)| sym == fill.counter);
        if !counter_is_word || !words(fill.limit) {
            continue;
        }
        let counter_value = native_counter
            .filter(|(counter, _)| *counter == fill.counter)
            .map_or_else(|| ctx.resolve(fill.counter), |(_, value)| value.to_string());
        writeln!(
            output,
            "{pad}LOGOS_SEQ_RESERVE(&{}, logos_counted_fill({}, {}, {}, {}));",
            ctx.resolve(sym),
            counter_value,
            codegen_expr(fill.limit, ctx),
            u8::from(fill.exclusive),
            fill.pushes.max(1)
        )
        .unwrap();
    }

    // The Rust emitter uses this same counted-loop proof to construct maps with
    // capacity up front. Carry the proof into C as well: reserving the loop's
    // maximum insertion count is sound even when a conditional skips some
    // writes, and prevents repeated resize/rehash cycles in map-building loops.
    let Some((counter, limit, exclusive, pass)) =
        crate::codegen::peephole::counted_loop(cond, body)
    else {
        return;
    };
    let mut written_maps: Vec<Symbol> = Vec::new();
    logicaffeine_language::ast_depth::walk_nodes(pass, |node| {
        use logicaffeine_language::ast_depth::{Visited, Walk};
        match node {
            Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => {
                Walk::Past
            }
            Visited::Stmt(Stmt::SetIndex {
                collection: Expr::Identifier(sym),
                ..
            }) => {
                if !written_maps.contains(sym) {
                    written_maps.push(*sym);
                }
                Walk::Into
            }
            _ => Walk::Into,
        }
    });
    for sym in written_maps {
        if !matches!(ctx.vars.get(&sym), Some(CType::MapI64I64(..))) {
            continue;
        }
        let counter_expr = Expr::Identifier(counter);
        let words = |e: &Expr| infer_expr_type(e, ctx) == CType::Int64 && !may_act(e, ctx);
        let counter_is_word =
            words(&counter_expr) || native_counter.is_some_and(|(sym, _)| sym == counter);
        if !counter_is_word || !words(limit) {
            continue;
        }
        let counter_value = native_counter
            .filter(|(candidate, _)| *candidate == counter)
            .map_or_else(|| ctx.resolve(counter), |(_, value)| value.to_string());
        crate::optimize::mark_fired(crate::optimization::Opt::Peephole);
        writeln!(
            output,
            "{pad}map_i64_i64_reserve(&{}, logos_counted_fill({}, {}, {}, 1));",
            ctx.resolve(sym),
            counter_value,
            codegen_expr(limit, ctx),
            u8::from(exclusive),
        )
        .unwrap();
    }
}

/// The C lvalue of a place a statement changes in place, with its type: a variable, or a field of a record.
fn changed_place_of(collection: &Expr, ctx: &CContext) -> Option<(String, CType)> {
    match collection {
        Expr::Identifier(sym) => Some((
            ctx.resolve(*sym).to_string(),
            collection_type(collection, ctx),
        )),
        Expr::FieldAccess { .. } => Some((
            codegen_expr(collection, ctx),
            infer_expr_type(collection, ctx),
        )),
        _ => None,
    }
}

/// `rendered`, the C of the value `val` a `Return` hands back, as a value of the function's return type: a raw
/// int64 promoted where the function returns `LInt`, and a tuple whose components the function widened to
/// `LInt` built with each so.
pub(super) fn as_returned(val: &Expr, rendered: String, ctx: &CContext) -> String {
    let returns = &ctx.current_fn_returns;
    // A result a stored `nothing` reaches hands back a value-or-nothing whole, and any other value present.
    if matches!(returns, CType::Opt(_)) {
        return opt_item(val, returns, ctx);
    }
    if let Some(fresh) = fresh_collection(val, returns, ctx) {
        return fresh;
    }
    let from = infer_expr_type(val, ctx);
    if *returns == CType::Tower && from != CType::Tower {
        return super::tower::value(val, ctx);
    }
    if let Some(widened) = rational_widened(val, returns, ctx) {
        return widened;
    }
    if *returns == CType::BigInt && from != CType::BigInt {
        return format!("lint_of((int64_t)({rendered}))");
    }
    let components = |t: &CType| match t {
        CType::Tuple(elems) => Some(elems.clone()),
        CType::Record(fields) => Some(fields.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>()),
        _ => None,
    };
    let (Some(to), Some(have)) = (components(returns), components(&from)) else {
        return rendered;
    };
    if to == have || to.len() != have.len() {
        return rendered;
    }
    let promote = |part: String, have: &CType, to: &CType| {
        if *to == CType::BigInt && *have != CType::BigInt {
            format!("lint_of((int64_t)({part}))")
        } else {
            part
        }
    };
    let name = tuple_c_name(&to, ctx.interner);
    if let Expr::Tuple(items) = val {
        let parts: Vec<String> = items
            .iter()
            .zip(have.iter().zip(&to))
            .map(|(item, (h, t))| promote(stored(item, codegen_expr(item, ctx), ctx), h, t))
            .collect();
        return format!("({name}){{{}}}", parts.join(", "));
    }
    let n = ctx.select_ctr.get();
    ctx.select_ctr.set(n + 1);
    let from_name = tuple_c_name(&have, ctx.interner);
    let parts: Vec<String> = have
        .iter()
        .zip(&to)
        .enumerate()
        .map(|(i, (h, t))| promote(format!("__ret{n}.f{i}"), h, t))
        .collect();
    format!(
        "({{ {from_name} __ret{n} = {rendered}; ({name}){{{}}}; }})",
        parts.join(", ")
    )
}

/// `expr` handed to a place declared `to` as the exact number it equals: an integer where a rational is declared, a
/// list of integers where a list of rationals is; `None` where `expr` is already of the place's form.
fn rational_widened(expr: &Expr, to: &CType, ctx: &CContext) -> Option<String> {
    let from = infer_expr_type(expr, ctx);
    match (to, &from) {
        (CType::Rational, CType::Int64 | CType::BigInt) => Some(codegen_expr_as_rational(expr, ctx)),
        (CType::SeqValue(elem), CType::SeqI64(Ints::Int)) if **elem == CType::Rational => {
            let n = ctx.select_ctr.get();
            ctx.select_ctr.set(n + 1);
            let cty = c_type_str_resolved(to, ctx.interner);
            let key = cty.strip_prefix("Seq_").unwrap_or(&cty).to_string();
            Some(format!(
                "({{ Seq_i64 __ints{n} = {}; {cty} __rats{n} = seq_{key}_new(); for (size_t __walk{n} = 0; __walk{n} < __ints{n}.len; __walk{n}++) seq_{key}_push(&__rats{n}, ldec_from_i64(__ints{n}.data[__walk{n}])); __rats{n}; }})",
                codegen_expr(expr, ctx)
            ))
        }
        _ => None,
    }
}

/// `rendered`, the C value of argument `i` of a call to the user function `function`: a copy of its own for a
/// parameter the function changes in place while the caller still reads the argument (see `sharing`), and a
/// machine-word integer handed to a parameter the program made exact (`LInt`, see `exact`) promoted.
fn as_param_value(
    rendered: String,
    arg: &Expr,
    function: Symbol,
    i: usize,
    ctx: &CContext,
) -> String {
    let declared = ctx.fn_params.get(&function).and_then(|ps| ps.get(i));
    // A `mutable` parameter takes the caller's variable by reference; any other argument there is a temporary of
    // the call's own (a compound literal), whose changes no one reads, as on the VM.
    if ctx
        .fn_inout
        .get(&function)
        .and_then(|ps| ps.get(i))
        .copied()
        .unwrap_or(false)
    {
        return match (arg, declared) {
            (Expr::Identifier(sym), _) if ctx.vars.contains_key(sym) || ctx.program_statics.contains(sym) => {
                format!("&{}", ctx.resolve(*sym))
            }
            // A lent element or field reaches here only where `optimize::sequence` could not lower it.
            (Expr::Index { .. } | Expr::FieldAccess { .. }, _) => {
                c_refuse_expr("an element or a field lent to a `mutable` parameter where it cannot be stored back (a `While` condition, or the value of an item or field store)")
            }
            (_, Some(ty)) => format!("&({}){{{rendered}}}", c_type_str_resolved(ty, ctx.interner)),
            (_, None) => format!("&(int64_t){{{rendered}}}"),
        };
    }
    // A parameter a stored `nothing` reaches takes a value-or-nothing whole, and any other value present.
    if let Some(ty @ CType::Opt(_)) = declared {
        return opt_item(arg, ty, ctx);
    }
    // A name nothing binds fails before the call: its stand-in only has to be of the parameter's type. A clock read
    // is no name a binding holds.
    if let (Expr::Identifier(sym), Some(ty)) = (arg, declared) {
        if !ctx.vars.contains_key(sym)
            && !ctx.program_statics.contains(sym)
            && crate::semantics::temporal::ClockRead::named(ctx.interner.resolve(*sym)).is_none()
        {
            let failure = format!("Undefined variable: {}", ctx.interner.resolve(*sym));
            return format!(
                "({{ {} {}; }})",
                ctx.fail_stmt(&c_string_literal(&failure)),
                c_zero_of(ty, ctx)
            );
        }
    }
    if declared == Some(&CType::Tower) {
        return super::tower::value(arg, ctx);
    }
    if let Some(widened) = declared.and_then(|to| rational_widened(arg, to, ctx)) {
        return widened;
    }
    if infer_expr_type(arg, ctx) == CType::String && shares_builder(arg, ctx) {
        return format!("lc_gc_strdup({rendered})");
    }
    // A parameter the function changes in place is its own: the argument goes as a copy while the caller still
    // reads it, and as itself otherwise.
    let changes = ctx
        .mutating
        .get(&function)
        .and_then(|flags| flags.get(i))
        .copied()
        .unwrap_or(false);
    if let Some(ty) = declared.filter(|ty| {
        changes
            && **ty != CType::String
            && super::values::holds_buffer(ty, ctx.registry, ctx.interner)
    }) {
        if super::values::argument_copies(arg, ctx) {
            return super::values::copy_value(ty, &rendered, ctx);
        }
        return rendered;
    }
    let exact = declared == Some(&CType::BigInt);
    let arg_exact = infer_expr_type(arg, ctx) == CType::BigInt;
    if exact && !arg_exact {
        format!("lint_of((int64_t)({}))", rendered)
    } else if arg_exact && declared == Some(&CType::Float64) {
        // An exact integer handed to a Real widens to the nearest double, as the reference tier widens an Int.
        format!("lint_to_f64({})", rendered)
    } else if arg_exact && declared == Some(&CType::Int64) {
        format!("lint_expect_i64({})", rendered)
    } else {
        rendered
    }
}

/// Render a non-`Text` operand as a string for concatenation: an `LInt` uses `lint_to_string` (the exact
/// arbitrary-precision value), any other integer uses `i64_to_str`.
fn stringify_num(expr: &Expr, ctx: &CContext) -> String {
    // `nothing` joins as the word it shows as.
    if matches!(expr, Expr::Literal(Literal::Nothing) | Expr::OptionNone) {
        return "lc_gc_strdup(\"nothing\")".to_string();
    }
    if let Some((var, ty)) = opt_var(expr, ctx) {
        return c_text_of(&ty, &var, ctx);
    }
    let s = codegen_expr(expr, ctx);
    match infer_expr_type(expr, ctx) {
        // Match the VM's rendering per type: an `LInt` its exact value, a `Bool` `true`/`false` (not 0/1),
        // a `Float` via the same `logos_fmt_f64` `show` uses (so `3.5` stays `3.5`, not truncated to `3`).
        CType::BigInt => format!("lint_to_string({})", s),
        CType::Bool => format!("lc_gc_strdup({} ? \"true\" : \"false\")", s),
        CType::Float64 => format!("logos_fmt_f64({})", s),
        CType::Int64 => format!("i64_to_str({})", s),
        other => c_text_of(&other, &s, ctx),
    }
}

/// A binary operation on a machine word, in the VM's ring (`semantics::arith`'s word path): `+ − ×` wrap
/// at the width, `/` and `%` divide unsigned and fail on a zero divisor in the VM's words, `&`/`|`/`xor`
/// are bitwise, a shift moves the bits by its count modulo the width, and two words order unsigned. A word
/// equals only a word of its own width and value. Operand pairs the VM refuses (a word with an `Int`, two
/// widths) never reach here: [`static_binary_failure`] answers them first. `None` when neither operand is
/// a word.
fn word_binary(
    op: BinaryOpKind,
    lt: &CType,
    rt: &CType,
    left: &Expr,
    right: &Expr,
    ctx: &CContext,
) -> Option<String> {
    use BinaryOpKind::*;
    let bits = match (lt, rt) {
        // Text and a word join as text, the word shown unsigned ([`c_text_join`]).
        (CType::String, _) | (_, CType::String) if op == Add => return None,
        (CType::Word(bits), _) | (_, CType::Word(bits)) => *bits,
        _ => return None,
    };
    if matches!(op, And | Or) {
        return None;
    }
    let ring = |value: String| match bits {
        64 => format!("((int64_t)({value}))"),
        _ => format!("((int64_t)(uint{bits}_t)({value}))"),
    };
    let l = codegen_expr(left, ctx);
    let r = codegen_expr(right, ctx);
    Some(match op {
        Eq | NotEq if lt != rt => format!(
            "({{ (void)({l}); (void)({r}); {}; }})",
            if op == Eq { "false" } else { "true" }
        ),
        Eq => format!("(({l}) == ({r}))"),
        NotEq => format!("(({l}) != ({r}))"),
        Lt | Gt | LtEq | GtEq => {
            let symbol = match op {
                Lt => "<",
                Gt => ">",
                LtEq => "<=",
                _ => ">=",
            };
            format!("((uint64_t)({l}) {symbol} (uint64_t)({r}))")
        }
        Add | Subtract | Multiply => {
            let symbol = match op {
                Add => "+",
                Subtract => "-",
                _ => "*",
            };
            ring(format!("(uint64_t)({l}) {symbol} (uint64_t)({r})"))
        }
        BitAnd | BitOr | BitXor => {
            let symbol = match op {
                BitAnd => "&",
                BitOr => "|",
                _ => "^",
            };
            format!("(({l}) {symbol} ({r}))")
        }
        Shl => ring(format!("(uint64_t)({l}) << ((uint32_t)({r}) % {bits}u)")),
        Shr => format!("((int64_t)((uint64_t)({l}) >> ((uint32_t)({r}) % {bits}u)))"),
        Divide | Modulo => {
            let zero = sample_of(rt, 0)?;
            let failure = crate::semantics::arith::binary_op(op, sample_of(lt, 1)?, zero).err()?;
            let n = ctx.select_ctr.get();
            ctx.select_ctr.set(n + 1);
            let symbol = if op == Divide { "/" } else { "%" };
            format!(
                "({{ uint64_t __wl{n} = (uint64_t)({l}); uint64_t __wr{n} = (uint64_t)({r}); if (__wr{n} == 0) {{ {} }} (int64_t)(__wl{n} {symbol} __wr{n}); }})",
                ctx.fail_stmt(&c_string_literal(&failure))
            )
        }
        _ => return None,
    })
}

fn is_temporal(t: &CType) -> bool {
    matches!(
        t,
        CType::Date | CType::Moment | CType::Time | CType::Duration | CType::Span
    )
}

/// Temporal arithmetic and comparison, exactly as the interpreter defines them (`semantics::arith`,
/// `semantics::compare`): a Span moves a Date or a Moment month-first, a Duration shifts a Moment,
/// Durations add; values of one temporal kind order and equal as numbers, a Moment orders against a
/// Time by its time of day, Spans are equal when both parts are, and values of different kinds are
/// never equal. An ordering the interpreter rejects is refused at generation. `None` when neither
/// operand is temporal. Operands are bound in source order, so their effects run left to right.
fn temporal_binary(
    op: BinaryOpKind,
    lt: &CType,
    rt: &CType,
    left: &Expr,
    right: &Expr,
    ctx: &CContext,
) -> Option<String> {
    use BinaryOpKind::{Add, Eq, Gt, GtEq, Lt, LtEq, NotEq, Subtract};
    if !is_temporal(lt) && !is_temporal(rt) {
        return None;
    }
    let bind = |ty: &CType, name: &str, expr: &Expr| {
        format!(
            "{} = {};",
            c_param_decl(ty, name, ctx.interner),
            codegen_expr(expr, ctx)
        )
    };
    let both = |body: String| {
        format!(
            "({{ {} {} {}; }})",
            bind(lt, "__l", left),
            bind(rt, "__r", right),
            body
        )
    };
    let c_rel = |op: BinaryOpKind| match op {
        Lt => "<",
        LtEq => "<=",
        Gt => ">",
        GtEq => ">=",
        Eq => "==",
        _ => "!=",
    };
    Some(match (op, lt, rt) {
        (Add, CType::Date, CType::Span) => {
            both("logos_date_add_span(__l, __r.months, __r.days)".into())
        }
        (Subtract, CType::Date, CType::Span) => both(
            "logos_date_add_span(__l, logos_wrap_neg32(__r.months), logos_wrap_neg32(__r.days))"
                .into(),
        ),
        (Add, CType::Moment, CType::Span) => {
            both("logos_moment_add_span(__l, __r.months, __r.days)".into())
        }
        (Add, CType::Span, CType::Moment) => {
            both("logos_moment_add_span(__r, __l.months, __l.days)".into())
        }
        (Subtract, CType::Moment, CType::Span) => both(
            "logos_moment_add_span(__l, logos_wrap_neg32(__r.months), logos_wrap_neg32(__r.days))"
                .into(),
        ),
        (Add, CType::Moment, CType::Duration)
        | (Add, CType::Duration, CType::Moment)
        | (Add, CType::Duration, CType::Duration) => both("logos_wrap_add64(__l, __r)".into()),
        (Subtract, CType::Moment, CType::Duration)
        | (Subtract, CType::Duration, CType::Duration) => both("logos_wrap_sub64(__l, __r)".into()),
        (Lt | LtEq | Gt | GtEq, CType::Moment, CType::Time) => both(format!(
            "logos_rem_euclid(__l, LOGOS_NANOS_PER_DAY) {} __r",
            c_rel(op)
        )),
        (Lt | LtEq | Gt | GtEq, CType::Time, CType::Moment) => both(format!(
            "__l {} logos_rem_euclid(__r, LOGOS_NANOS_PER_DAY)",
            c_rel(op)
        )),
        (Eq, CType::Span, CType::Span) => both("logos_span_equals(__l, __r)".into()),
        (NotEq, CType::Span, CType::Span) => both("!logos_span_equals(__l, __r)".into()),
        (Lt | LtEq | Gt | GtEq | Eq | NotEq, l, r) if l == r => {
            both(format!("__l {} __r", c_rel(op)))
        }
        (Eq, _, _) => both("false".into()),
        (NotEq, _, _) => both("true".into()),
        (other, l, r) => format!(
            "\n#error \"the operator {:?} is not defined between a {:?} and a {:?}\"\n",
            other, l, r
        ),
    })
}

pub(super) fn codegen_expr(expr: &Expr, ctx: &CContext) -> String {
    if let Some(cell) = ctx
        .precomputed
        .borrow()
        .get(&(expr as *const Expr as usize))
    {
        return cell.clone();
    }
    match expr {
        // `and`/`or` sequence their operands themselves; a closure's body runs later.
        Expr::BinaryOp {
            op: BinaryOpKind::And | BinaryOpKind::Or,
            ..
        }
        | Expr::Closure { .. } => codegen_expr_node(expr, ctx),
        _ => sequenced(
            &logicaffeine_language::ast_depth::expr_children(expr),
            ctx,
            || codegen_expr_node(expr, ctx),
        ),
    }
}

/// Whether computing `expr` may fail or act — call a function, index a collection, divide, read a value that
/// may be nothing, meet an operator its operand kinds do not take — so that its place in the order of
/// evaluation can be observed. A literal, a bound name and a total combination of those cannot.
fn may_act(expr: &Expr, ctx: &CContext) -> bool {
    acts(expr, ctx, false)
}

/// Whether `expr` may fail or act other than by a call to a quiet function (see `quiet_functions`): when no
/// operand of an expression acts so, its operands need no order at all — quiet calls may run in any order among
/// themselves, and beside pure operands.
fn acts_loudly(expr: &Expr, ctx: &CContext) -> bool {
    acts(expr, ctx, true)
}

fn acts(expr: &Expr, ctx: &CContext, quiet_is_silent: bool) -> bool {
    if ctx
        .precomputed
        .borrow()
        .contains_key(&(expr as *const Expr as usize))
    {
        return false;
    }
    match expr {
        Expr::Literal(_) => false,
        Expr::Identifier(sym) => {
            (!ctx.vars.contains_key(sym) && !ctx.program_statics.contains(sym))
                || opt_var(expr, ctx).is_some()
        }
        Expr::Not { operand } => acts(operand, ctx, quiet_is_silent),
        Expr::FieldAccess { object, .. } => acts(object, ctx, quiet_is_silent),
        Expr::Length { collection } => acts(collection, ctx, quiet_is_silent),
        Expr::Tuple(items) | Expr::List(items) => {
            items.iter().any(|i| acts(i, ctx, quiet_is_silent))
        }
        Expr::Call { function, args } if quiet_is_silent && ctx.quiet_fns.contains(function) => {
            args.iter().any(|a| acts(a, ctx, quiet_is_silent))
        }
        Expr::BinaryOp { op, left, right } => {
            matches!(
                op,
                BinaryOpKind::Divide
                    | BinaryOpKind::ExactDivide
                    | BinaryOpKind::FloorDivide
                    | BinaryOpKind::Modulo
                    | BinaryOpKind::Pow
            ) || static_binary_failure(
                *op,
                &infer_expr_type(left, ctx),
                &infer_expr_type(right, ctx),
            )
            .is_some()
                || infer_expr_type(expr, ctx) == CType::Tower
                || acts(left, ctx, quiet_is_silent)
                || acts(right, ctx, quiet_is_silent)
        }
        _ => true,
    }
}

/// `build`, the C of an expression over `operands`, with the operands computed left to right as the VM computes
/// them: when two or more may fail or act, every one of those but the last is computed first into a cell the
/// expression then reads, so C's unspecified order among operands cannot reorder them.
fn sequenced(operands: &[&Expr], ctx: &CContext, build: impl FnOnce() -> String) -> String {
    let acting: Vec<&Expr> = operands
        .iter()
        .copied()
        .filter(|o| may_act(o, ctx))
        .collect();
    if acting.len() < 2 || !operands.iter().any(|o| acts_loudly(o, ctx)) {
        return build();
    }
    let n = ctx.select_ctr.get();
    ctx.select_ctr.set(n + 1);
    let mut prelude = String::new();
    let mut keys = Vec::new();
    for (i, operand) in acting[..acting.len() - 1].iter().enumerate() {
        let ty = infer_expr_type(operand, ctx);
        if ty == CType::Void {
            continue;
        }
        let cell = format!("__seq{n}_{i}");
        write!(
            prelude,
            "{} = {}; ",
            c_param_decl(&ty, &cell, ctx.interner),
            codegen_expr(operand, ctx)
        )
        .unwrap();
        let key = *operand as *const Expr as usize;
        ctx.precomputed.borrow_mut().insert(key, cell);
        keys.push(key);
    }
    let body = build();
    for key in keys {
        ctx.precomputed.borrow_mut().remove(&key);
    }
    format!("({{ {prelude}{body}; }})")
}

/// The statement-level form of [`sequenced`]: each of `operands`, in the order given, but the last that may fail
/// or act computed into a cell ahead of the statement. Answers the cells, for the caller to forget once the
/// statement is written.
fn sequence_operands(
    operands: &[&Expr],
    ctx: &mut CContext,
    output: &mut String,
    pad: &str,
) -> Vec<usize> {
    let acting: Vec<&Expr> = operands
        .iter()
        .copied()
        .filter(|o| may_act(o, ctx))
        .collect();
    let mut keys = Vec::new();
    if acting.len() < 2 || !operands.iter().any(|o| acts_loudly(o, ctx)) {
        return keys;
    }
    for operand in &acting[..acting.len() - 1] {
        let ty = infer_expr_type(operand, ctx);
        if ty == CType::Void {
            continue;
        }
        let value = codegen_expr(operand, ctx);
        let cell = ctx.temp("__operand", ty.clone());
        writeln!(output, "{pad}{} = {value};", ctx.decl_head(&ty, &cell)).unwrap();
        let key = *operand as *const Expr as usize;
        ctx.precomputed.borrow_mut().insert(key, cell);
        keys.push(key);
    }
    keys
}

fn forget_operands(keys: Vec<usize>, ctx: &CContext) {
    for key in keys {
        ctx.precomputed.borrow_mut().remove(&key);
    }
}

fn codegen_expr_node(expr: &Expr, ctx: &CContext) -> String {
    if let Some(exact) = super::tower::emit(expr, ctx) {
        return exact;
    }
    if let Some(read) = super::tower::emit_reading(expr, ctx) {
        return read;
    }
    match expr {
        Expr::Literal(lit) => codegen_literal(lit, ctx),
        // The temporal builtins `now` / `today` resolve to the injected fixed clock (the runtime shim
        // reads `LOGOS_FIXED_NANOS` / `LOGOS_FIXED_DAYS`) — byte-for-byte the VM/tree-walker, where the
        // NAME wins even when shadowed by a `Let`. So this must fire BEFORE the identifier→variable path.
        Expr::Identifier(sym) if matches!(ctx.interner.resolve(*sym), "now") => {
            "logos_now()".to_string()
        }
        Expr::Identifier(sym) if matches!(ctx.interner.resolve(*sym), "today") => {
            "logos_today()".to_string()
        }
        Expr::Identifier(_) if opt_var(expr, ctx).is_some() => {
            let (var, ty) = opt_var(expr, ctx).expect("a value-or-nothing variable");
            super::types::opt_unpack(
                &opt_element(&ty),
                &format!("logos_opt_int({var})"),
                ctx.interner,
            )
        }
        // A name nothing binds fails when it is read, as on the VM — a function's name included, since a
        // function is called, never held as a value.
        Expr::Identifier(sym)
            if !ctx.vars.contains_key(sym) && !ctx.program_statics.contains(sym) =>
        {
            let failure = format!("Undefined variable: {}", ctx.interner.resolve(*sym));
            format!(
                "({{ {} (int64_t)0; }})",
                ctx.fail_stmt(&c_string_literal(&failure))
            )
        }
        Expr::Identifier(sym) => ctx.resolve(*sym).to_string(),
        Expr::BinaryOp { op, left, right } => {
            // `a is b` where either side may be `nothing`: equal when both are nothing or both hold equal values,
            // as the VM compares them (`equality`) — never a read of `nothing` as an Int.
            let optional = |e: &Expr| {
                opt_var(e, ctx).is_some() || matches!(infer_expr_type(e, ctx), CType::Opt(_))
            };
            if matches!(op, BinaryOpKind::Eq | BinaryOpKind::NotEq) && (optional(left) || optional(right)) {
                let side = |e: &Expr| match opt_var(e, ctx) {
                    Some((var, ty)) => (ty, var),
                    None if matches!(e, Expr::Literal(Literal::Nothing)) => (CType::Void, String::new()),
                    None => (infer_expr_type(e, ctx), codegen_expr(e, ctx)),
                };
                let ((lt, l), (rt, r)) = (side(left), side(right));
                let eq = super::equality::value_equal(&lt, &l, &rt, &r, ctx).unwrap_or_else(|| {
                    c_refuse_expr(&format!("`is equal to` between a {lt:?} and a {rt:?}"))
                });
                return if *op == BinaryOpKind::NotEq {
                    format!("(!{})", eq)
                } else {
                    eq
                };
            }
            // `and`/`or` read each side's truth, the right only when the left does not decide.
            if matches!(op, BinaryOpKind::And | BinaryOpKind::Or) {
                let joint = if *op == BinaryOpKind::And { "&&" } else { "||" };
                return format!("({} {joint} {})", c_truthy(left, ctx), c_truthy(right, ctx));
            }
            if let Some(code) = opt_operand_binary(expr, *op, left, right, ctx) {
                return code;
            }
            let lt = infer_expr_type(left, ctx);
            let rt = infer_expr_type(right, ctx);
            // Operands of kinds the operator does not combine: the VM computes both, then fails in the words
            // its arithmetic gives.
            if let Some(failure) = static_binary_failure(*op, &lt, &rt) {
                let zero = c_zero_of(&infer_expr_type(expr, ctx), ctx);
                return format!(
                    "({{ (void)({}); (void)({}); {} {zero}; }})",
                    codegen_expr(left, ctx),
                    codegen_expr(right, ctx),
                    ctx.fail_stmt(&c_string_literal(&failure))
                );
            }
            if let Some(code) = temporal_binary(*op, &lt, &rt, left, right, ctx) {
                return code;
            }
            if let Some(code) = word_binary(*op, &lt, &rt, left, right, ctx) {
                return code;
            }
            // A constructing optimizer can version an exact-integer expression behind a
            // guard and attach RawIntOp to the fresh node. Rust already consumes this
            // proof; direct C must do the same or it reconstructs LInt arithmetic inside
            // the proven-word branch. Narrow exact operands at that guarded boundary and
            // emit the operation itself raw. The cold sibling retains the original exact
            // nodes and therefore the arbitrary-precision fallback.
            if crate::optimize::facts::holds_expr(crate::optimize::facts::Fact::RawIntOp, expr) {
                let symbol = match op {
                    BinaryOpKind::Add => Some("+"),
                    BinaryOpKind::Subtract => Some("-"),
                    BinaryOpKind::Multiply => Some("*"),
                    _ => None,
                };
                if let Some(symbol) = symbol {
                    let l = codegen_expr(left, ctx);
                    let r = codegen_expr(right, ctx);
                    let l = if lt == CType::BigInt {
                        format!("lint_expect_i64({l})")
                    } else {
                        l
                    };
                    let r = if rt == CType::BigInt {
                        format!("lint_expect_i64({r})")
                    } else {
                        r
                    };
                    return format!("(({l}) {symbol} ({r}))");
                }
            }
            if let Some(code) = mask_narrow(expr, ctx) {
                return code;
            }
            // Consume the exact same whole-body non-negativity proof as the
            // Rust emitter: x % 2^k is a mask and x / 2^k is a shift. Exact
            // LInt values keep a guarded small fast path and fall back to the
            // original arbitrary-precision operation for a big (or defensive
            // negative) value.
            if matches!(op, BinaryOpKind::Modulo | BinaryOpKind::Divide)
                && matches!(lt, CType::Int64 | CType::BigInt)
                && crate::optimize::active_config()
                    .is_on(crate::optimization::Opt::StrengthReducePow2)
            {
                if let Expr::Literal(Literal::Number(divisor)) = right {
                    if *divisor > 0
                        && (*divisor as u64).is_power_of_two()
                        && crate::codegen::expr::expr_is_nonnegative(left, &ctx.nonnegative_vars)
                    {
                        crate::optimize::mark_fired(crate::optimization::Opt::StrengthReducePow2);
                        let value = codegen_expr(left, ctx);
                        return match (*op, &lt) {
                            (BinaryOpKind::Modulo, CType::BigInt) => {
                                format!("logos_lint_mod_pow2_nonnegative({value}, {divisor}LL)")
                            }
                            (BinaryOpKind::Modulo, _) => {
                                format!("(({value}) & {}LL)", divisor - 1)
                            }
                            (BinaryOpKind::Divide, CType::BigInt) => format!(
                                "logos_lint_div_pow2_nonnegative({value}, {divisor}LL, {})",
                                divisor.trailing_zeros()
                            ),
                            (BinaryOpKind::Divide, _) => {
                                format!("(({value}) >> {})", divisor.trailing_zeros())
                            }
                            _ => unreachable!(),
                        };
                    }
                }
            }
            // Opt::ModWrap: a remainder by a constant of a word bounded below twice it is one conditional
            // subtraction, or the word itself below the constant (see `crate::optimize::modwrap`).
            if *op == BinaryOpKind::Modulo
                && lt == CType::Int64
                && crate::optimize::active_config().is_on(crate::optimization::Opt::ModWrap)
            {
                if let Expr::Literal(Literal::Number(m)) = right {
                    if let Some(wrap) = crate::optimize::modwrap::mod_wrap(*m, int_range(left, ctx)) {
                        crate::optimize::mark_fired(crate::optimization::Opt::ModWrap);
                        let value = codegen_expr(left, ctx);
                        return match wrap {
                            crate::optimize::modwrap::ModWrap::Identity => format!("({value})"),
                            crate::optimize::modwrap::ModWrap::Once => format!(
                                "({{ int64_t __wrap = ({value}); __wrap >= {m}LL ? __wrap - {m}LL : __wrap; }})"
                            ),
                        };
                    }
                }
            }
            if let Some(code) = bounded_operation(expr, *op, left, right, &lt, &rt, ctx) {
                return code;
            }

            // A text joined by `+` or `combined with`, however many parts: one allocation.
            if is_text_join(expr, ctx) {
                return c_text_join(expr, ctx);
            }

            if *op == BinaryOpKind::SeqConcat {
                return c_seq_concat(expr, left, right, ctx);
            }

            // Two sequences are equal when their items are, in order, and two maps or sets when their entries are,
            // as the VM compares them.
            if (*op == BinaryOpKind::Eq || *op == BinaryOpKind::NotEq)
                && (is_sequence(&lt) || super::equality::table_parts(&lt).is_some())
            {
                let (l, r) = (codegen_expr(left, ctx), codegen_expr(right, ctx));
                if let Some(same) = super::equality::value_equal(&lt, &l, &rt, &r, ctx) {
                    return if *op == BinaryOpKind::Eq { same } else { format!("(!{same})") };
                }
            }
            // String comparison
            if (*op == BinaryOpKind::Eq || *op == BinaryOpKind::NotEq)
                && (lt == CType::String || rt == CType::String)
            {
                let l = codegen_expr(left, ctx);
                let r = codegen_expr(right, ctx);
                if *op == BinaryOpKind::Eq {
                    return format!("str_equals({}, {})", l, r);
                } else {
                    return format!("(!str_equals({}, {}))", l, r);
                }
            }

            // An exact quotient of integers is the fraction it denotes.
            if *op == BinaryOpKind::ExactDivide
                && !matches!(lt, CType::Rational | CType::Decimal)
                && !matches!(rt, CType::Rational | CType::Decimal)
                && infer_expr_type(expr, ctx) == CType::Rational
            {
                return codegen_expr_as_rational(expr, ctx);
            }
            let mut l = codegen_expr(left, ctx);
            let mut r = codegen_expr(right, ctx);
            // An integer and a real compare as the numbers do, however far past 2^53 the integer is: the left's
            // order against the right is -1 below, 0 at, 1 above, 2 for a real that is not a number, which nothing
            // is below, above or equal to.
            let holds = match op {
                BinaryOpKind::Eq => Some("__order{n} == 0"),
                BinaryOpKind::NotEq => Some("__order{n} != 0"),
                BinaryOpKind::Lt => Some("__order{n} == -1"),
                BinaryOpKind::LtEq => Some("(__order{n} == -1 || __order{n} == 0)"),
                BinaryOpKind::Gt => Some("__order{n} == 1"),
                BinaryOpKind::GtEq => Some("(__order{n} == 1 || __order{n} == 0)"),
                _ => None,
            };
            if let Some(holds) = holds {
                if let Some(order) = super::equality::mixed_order(&lt, &l, &rt, &r, ctx) {
                    let n = ctx.select_ctr.get();
                    ctx.select_ctr.set(n + 1);
                    let holds = holds.replace("{n}", &n.to_string());
                    return format!("({{ int __order{n} = {order}; {holds}; }})");
                }
            }
            // A BigInt (`LInt`) operand mixed with a Float64 operand is FLOATING-POINT arithmetic,
            // not bignum: convert the LInt side to `double` (`lint_to_f64`) so it flows through the C
            // float operators / `pow` below. `infer_expr_type` already types such a mix as `Float64`
            // (its `Float64` arm precedes the `BigInt` arm), so the sink is a `double`; without this
            // the bignum branch would coerce the FLOAT operand to an `LInt` (`lint_of((int64_t)f)`, a
            // silent truncation) and yield an `LInt` value — a type error at the `double` sink (a
            // loop-carried `state * 1.0`) and a wrong result for `bigval * 0.5`.
            let bigint_float_mix = (lt == CType::BigInt && rt == CType::Float64)
                || (lt == CType::Float64 && rt == CType::BigInt);
            if bigint_float_mix {
                if lt == CType::BigInt {
                    l = format!("lint_to_f64({})", l);
                }
                if rt == CType::BigInt {
                    r = format!("lint_to_f64({})", r);
                }
            }
            // Exact modular arithmetic — `+ − × ÷` in the ring/field ℤ/nℤ (`LMod`), and ONLY between
            // two same-ring `Modular` values: the interpreter deliberately rejects `Modular + Int` and
            // the `Modular ** Int` operator ("needs two ℤ/nℤ values of the same modulus"), so a mixed
            // or `**` form is left to fall through and fail loudly at gcc rather than silently compute
            // a value the spec refuses. `÷` multiplies by the divisor's modular inverse (`mod_div`).
            if lt == CType::Modular
                && rt == CType::Modular
                && matches!(
                    op,
                    BinaryOpKind::Add
                        | BinaryOpKind::Subtract
                        | BinaryOpKind::Multiply
                        | BinaryOpKind::Divide
                        | BinaryOpKind::ExactDivide
                )
            {
                let f = match op {
                    BinaryOpKind::Add => "mod_add",
                    BinaryOpKind::Subtract => "mod_sub",
                    BinaryOpKind::Multiply => "mod_mul",
                    _ => "mod_div",
                };
                return format!("{}({}, {})", f, l, r);
            }
            // Money arithmetic — same-currency `+ −`, a dimensionless `÷` ratio, and `× k` scaling.
            if lt == CType::Money
                && rt == CType::Money
                && matches!(op, BinaryOpKind::Add | BinaryOpKind::Subtract)
            {
                let f = if matches!(op, BinaryOpKind::Add) {
                    "money_add"
                } else {
                    "money_sub"
                };
                return format!("{}({}, {})", f, l, r);
            }
            if lt == CType::Money
                && rt == CType::Money
                && matches!(op, BinaryOpKind::Divide | BinaryOpKind::ExactDivide)
            {
                return format!("money_ratio({}, {})", l, r);
            }
            if matches!(op, BinaryOpKind::Multiply) && (lt == CType::Money) != (rt == CType::Money)
            {
                let (money, k) = if lt == CType::Money {
                    (&l, &r)
                } else {
                    (&r, &l)
                };
                return format!("money_scale({}, (int64_t)({}))", money, k);
            }
            // Quantity arithmetic — same-dimension `+ −`, dimension-combining `× ÷` (synthetic unit),
            // and scalar scaling (`q × k`, `k × q`, `q ÷ k`) which preserves the quantity's unit.
            if lt == CType::Quantity && rt == CType::Quantity {
                let f = match op {
                    BinaryOpKind::Add => Some("quant_add"),
                    BinaryOpKind::Subtract => Some("quant_sub"),
                    BinaryOpKind::Multiply => Some("quant_mul"),
                    BinaryOpKind::Divide | BinaryOpKind::ExactDivide => Some("quant_div"),
                    _ => None,
                };
                if let Some(f) = f {
                    return format!("{}({}, {})", f, l, r);
                }
            }
            if (lt == CType::Quantity) != (rt == CType::Quantity)
                && matches!(
                    op,
                    BinaryOpKind::Multiply | BinaryOpKind::Divide | BinaryOpKind::ExactDivide
                )
            {
                let q_left = lt == CType::Quantity;
                let (q, k) = if q_left { (&l, &r) } else { (&r, &l) };
                if matches!(op, BinaryOpKind::Multiply) {
                    // `q × k` or `k × q` — scale, preserving the unit.
                    return format!("quant_scale_mul({}, rat_from_i64((int64_t)({})))", q, k);
                } else if q_left {
                    // `q ÷ k` — scalar division (scalar-on-the-left `k ÷ q` falls through, failing loud).
                    return format!("quant_scale_div({}, rat_from_i64((int64_t)({})))", q, k);
                }
            }
            // `uuid == uuid` / `uuid != uuid` — a 16-byte compare (`uuid_eq`) yielding a C bool.
            if lt == CType::Uuid
                && rt == CType::Uuid
                && matches!(op, BinaryOpKind::Eq | BinaryOpKind::NotEq)
            {
                let call = format!("uuid_eq({}, {})", l, r);
                return if matches!(op, BinaryOpKind::NotEq) {
                    format!("(!{})", call)
                } else {
                    call
                };
            }
            // Exact complex arithmetic — `+ − × ÷` over `LComplex`; a bare integer operand promotes to
            // the real `re + 0i` (`cplx_ints`), mirroring the interpreter's `LogosComplex::from_i64`.
            if (lt == CType::Complex || rt == CType::Complex)
                && matches!(
                    op,
                    BinaryOpKind::Add
                        | BinaryOpKind::Subtract
                        | BinaryOpKind::Multiply
                        | BinaryOpKind::Divide
                        | BinaryOpKind::ExactDivide
                )
            {
                let lc = if lt == CType::Complex {
                    l.clone()
                } else {
                    format!("cplx_ints((int64_t)({}), 0)", l)
                };
                let rc = if rt == CType::Complex {
                    r.clone()
                } else {
                    format!("cplx_ints((int64_t)({}), 0)", r)
                };
                let f = match op {
                    BinaryOpKind::Add => "cplx_add",
                    BinaryOpKind::Subtract => "cplx_sub",
                    BinaryOpKind::Multiply => "cplx_mul",
                    _ => "cplx_div",
                };
                return format!("{}({}, {})", f, lc, rc);
            }
            // Exact arithmetic on an `LDec`, both operands re-emitted in rational context. A rational is the VM's
            // exact number: `/` truncates two whole values and divides anything else exactly, `//` floors, `%`
            // takes two whole values' integer remainder, `**` gives a whole base an integer's power, and the bitwise
            // operators take two whole values as integers (the runtime's `ldec_*` rules). A decimal keeps its digits
            // through `+ − ×` and divides exactly. A real operand makes the operation a real one, as the VM's.
            if let Some(exact) = exact_operation(*op, left, right, &lt, &rt, ctx) {
                return exact;
            }
            // Bitwise operations are machine-word operations in every runtime tier. Exact `LInt`
            // operands therefore narrow at this boundary before the raw operator is emitted. The
            // enclosing exact sink promotes the word result again when it must remain an `LInt`.
            // Without this boundary, exactness inference could emit illegal C such as `LInt | LInt`
            // or `8LL << LInt` (the N-Queens and Ackermann benchmark specializations).
            if matches!(
                op,
                BinaryOpKind::BitAnd
                    | BinaryOpKind::BitOr
                    | BinaryOpKind::BitXor
                    | BinaryOpKind::Shl
                    | BinaryOpKind::Shr
            ) && (lt == CType::BigInt || rt == CType::BigInt)
            {
                let lc = if lt == CType::BigInt {
                    format!("lint_expect_i64({})", l)
                } else {
                    l
                };
                let rc = if rt == CType::BigInt {
                    format!("lint_expect_i64({})", r)
                } else {
                    r
                };
                let symbol = match op {
                    BinaryOpKind::BitAnd => "&",
                    BinaryOpKind::BitOr => "|",
                    BinaryOpKind::BitXor => "^",
                    BinaryOpKind::Shl => "<<",
                    BinaryOpKind::Shr => ">>",
                    _ => unreachable!("matches! guards the five bitwise operators"),
                };
                return format!("({} {} {})", lc, symbol, rc);
            }
            // Exact arbitrary-precision integer (`LInt`) — arithmetic promotes any raw int64 operand
            // via `lint_of`, comparisons go through `lint_cmp`. Triggered by a BigInt operand OR a
            // provably-overflowing all-literal expression (`10000000000 * 10000000000`).
            if (lt == CType::BigInt || rt == CType::BigInt || int_exceeds_i64(expr))
                && !bigint_float_mix
            {
                let lc = if lt == CType::BigInt {
                    l.clone()
                } else {
                    format!("lint_of((int64_t)({}))", l)
                };
                let rc = if rt == CType::BigInt {
                    r.clone()
                } else {
                    format!("lint_of((int64_t)({}))", r)
                };
                if let Some(f) = match op {
                    BinaryOpKind::Add => Some("logos_lint_add"),
                    BinaryOpKind::Subtract => Some("logos_lint_sub"),
                    BinaryOpKind::Multiply => Some("logos_lint_mul"),
                    _ => None,
                } {
                    return format!("{}({}, {})", f, lc, rc);
                }
                // A zero divisor fails in the words the VM's arithmetic gives it.
                if let Some((f, refusal)) = match op {
                    BinaryOpKind::Divide | BinaryOpKind::ExactDivide => Some((
                        "logos_lint_div",
                        crate::semantics::arith::divide(
                            crate::interpreter::RuntimeValue::Int(1),
                            crate::interpreter::RuntimeValue::Int(0),
                        ),
                    )),
                    BinaryOpKind::Modulo => Some((
                        "logos_lint_mod",
                        crate::semantics::arith::modulo(
                            crate::interpreter::RuntimeValue::Int(1),
                            crate::interpreter::RuntimeValue::Int(0),
                        ),
                    )),
                    BinaryOpKind::FloorDivide => Some((
                        "logos_lint_floordiv",
                        crate::semantics::arith::floor_divide(
                            crate::interpreter::RuntimeValue::Int(1),
                            crate::interpreter::RuntimeValue::Int(0),
                        ),
                    )),
                    _ => None,
                } {
                    let refusal = refusal.err().unwrap_or_default();
                    let exact = format!("{}({}, {}, {})", f, lc, rc, c_string_literal(&refusal));
                    // A remainder by a machine-word divisor is a machine word (see `infer_expr_type`).
                    return if *op == BinaryOpKind::Modulo && rt == CType::Int64 {
                        format!("lint_expect_i64({exact})")
                    } else {
                        exact
                    };
                }
                if let Some(cmp) = match op {
                    BinaryOpKind::Eq => Some("=="),
                    BinaryOpKind::NotEq => Some("!="),
                    BinaryOpKind::Lt => Some("<"),
                    BinaryOpKind::LtEq => Some("<="),
                    BinaryOpKind::Gt => Some(">"),
                    BinaryOpKind::GtEq => Some(">="),
                    _ => None,
                } {
                    return format!("(logos_lint_cmp({}, {}) {} 0)", lc, rc, cmp);
                }
            }
            // Tolerant comparison — the shared isclose semantics
            // (`logos_approx_eq`), self-contained C (fabs/fmax from math.h).
            if matches!(op, BinaryOpKind::ApproxEq) {
                return format!(
                    "(({l}) == ({r}) || fabs(({l}) - ({r})) <= fmax(1e-9 * fmax(fabs({l}), fabs({r})), 1e-12))",
                    l = l, r = r
                );
            }
            // `**` has no C operator. A Float operand uses libm `pow`. Integer exponentiation is
            // EXACT and unbounded — it raises the base through the arbitrary-precision runtime
            // (`lint_pow` → `lbig_pow` + `lint_norm`), which downsizes a fitting result to a fast
            // small (`2 ** 10 → 1024`) and promotes an overflow to the exact BigInt (`2 ** 100`),
            // byte-for-byte the tree-walker/VM. A negative exponent aborts loudly inside `lint_pow`.
            if matches!(op, BinaryOpKind::Pow) {
                if lt == CType::Float64 || rt == CType::Float64 {
                    return format!("pow((double)({}), (double)({}))", l, r);
                }
                if matches!(lt, CType::Int64 | CType::BigInt)
                    && matches!(rt, CType::Int64 | CType::BigInt)
                {
                    let base = if lt == CType::BigInt {
                        l.clone()
                    } else {
                        format!("lint_of((int64_t)({}))", l)
                    };
                    let exp = if rt == CType::BigInt {
                        format!("lint_expect_i64({})", r)
                    } else {
                        format!("(int64_t)({})", r)
                    };
                    return format!("lint_pow({}, {})", base, exp);
                }
                return format!("((int64_t)llround(pow((double)({}), (double)({}))))", l, r);
            }
            // `//` floors toward negative infinity; C's `/` truncates toward zero, so the
            // integer case corrects by one when the operands differ in sign and the
            // remainder is nonzero. A Float operand floors the double quotient.
            if matches!(op, BinaryOpKind::FloorDivide) {
                if lt == CType::Float64 || rt == CType::Float64 {
                    return format!("floor(logos_rdiv((double)({}), (double)({})))", l, r);
                }
                return format!(
                    "(logos_idiv({l}, {r}) - (((logos_imod({l}, {r}) != 0) && ((logos_imod({l}, {r}) < 0) != (({r}) < 0))) ? 1 : 0))",
                    l = l, r = r
                );
            }
            // `/` and `%` fail on a zero divisor with the interpreter's words, where C's would trap or give `inf`.
            if matches!(op, BinaryOpKind::Divide | BinaryOpKind::Modulo) {
                let real = lt == CType::Float64 || rt == CType::Float64;
                let int = matches!(lt, CType::Int64 | CType::Bool)
                    && matches!(rt, CType::Int64 | CType::Bool);
                let checked = match (op, real, int) {
                    (BinaryOpKind::Divide, true, _) => Some(("logos_rdiv", "double")),
                    (BinaryOpKind::Divide, false, true) => Some(("logos_idiv", "int64_t")),
                    (BinaryOpKind::Modulo, false, true) => Some(("logos_imod", "int64_t")),
                    _ => None,
                };
                if let Some((f, ty)) = checked {
                    return format!("{f}(({ty})({l}), ({ty})({r}))");
                }
            }
            let op_str = match op {
                BinaryOpKind::Add => "+",
                BinaryOpKind::Subtract => "-",
                BinaryOpKind::Multiply => "*",
                BinaryOpKind::Divide | BinaryOpKind::ExactDivide => "/",
                BinaryOpKind::Modulo => "%",
                BinaryOpKind::Eq => "==",
                BinaryOpKind::NotEq => "!=",
                BinaryOpKind::Lt => "<",
                BinaryOpKind::LtEq => "<=",
                BinaryOpKind::Gt => ">",
                BinaryOpKind::GtEq => ">=",
                BinaryOpKind::And | BinaryOpKind::Or => unreachable!("handled above"),
                BinaryOpKind::Concat => "+",
                BinaryOpKind::SeqConcat => unreachable!("handled above"),
                BinaryOpKind::ApproxEq => unreachable!("handled above"),
                BinaryOpKind::Pow => unreachable!("handled above"),
                BinaryOpKind::FloorDivide => unreachable!("handled above"),
                BinaryOpKind::BitAnd => "&",
                BinaryOpKind::BitOr => "|",
                BinaryOpKind::BitXor => "^",
                BinaryOpKind::Shl => "<<",
                BinaryOpKind::Shr => ">>",
            };
            format!("({} {} {})", l, op_str, r)
        }
        Expr::Call { function, args } => {
            let raw_name = ctx.interner.resolve(*function);
            // "Declarer wins": a function the program defines (`funcs` holds every `## To` with a
            // body) is called as written — no builtin arm below may take its name.
            let defined_here =
                ctx.funcs.contains_key(function) && !ctx.native_funcs.contains(function);
            let builtin = if defined_here { "" } else { raw_name };
            match builtin {
                "args" => "logos_args()".to_string(),
                "randomInt" if args.len() == 2 => {
                    let lo = codegen_expr(args[0], ctx);
                    let hi = codegen_expr(args[1], ctx);
                    let lo = narrow_i64_sink(args[0], lo, ctx);
                    let hi = narrow_i64_sink(args[1], hi, ctx);
                    format!("lc_random_int({lo}, {hi})")
                }
                // env.lg's `get(name)`: the variable in the process environment, `some` of its text or
                // `none` when it is unset, built as the program's own `Option of Text`.
                "get" if args.len() == 1 => {
                    let name = codegen_expr(args[0], ctx);
                    let opt = c_type_str_resolved(&infer_expr_type(expr, ctx), ctx.interner);
                    format!(
                        "({{ const char *__env = getenv({name}); __env ? ({opt}){{.tag = {opt}_Some, \
                         .data.Some = {{.value = lc_gc_strdup(__env)}}}} : ({opt}){{.tag = {opt}_None}}; }})"
                    )
                }
                // time.lg's `now()`: the Unix time in milliseconds, not the moment the bare `now` names.
                "now" if args.is_empty() => "logos_time_now()".to_string(),

                // `format(x)` — the value's text exactly as `Show` prints it.
                "format" if args.len() == 1 => c_format_builtin(args[0], ctx),
                // The call spellings of `length of x`, `copy of x` and `x contains y`: each emits exactly what
                // its dedicated node emits.
                "length" if args.len() == 1 => codegen_expr(
                    &Expr::Length {
                        collection: args[0],
                    },
                    ctx,
                ),
                "contains" if args.len() == 2 => c_contains(args[0], args[1], ctx),
                "copy" if args.len() == 1 => codegen_expr(&Expr::Copy { expr: args[0] }, ctx),
                "text_bytes" => {
                    let a = args
                        .first()
                        .map(|a| codegen_expr(a, ctx))
                        .unwrap_or_else(|| "\"\"".to_string());
                    format!("logos_text_bytes({})", a)
                }
                "text_from_bytes" => {
                    let a = args
                        .first()
                        .map(|a| codegen_expr(a, ctx))
                        .unwrap_or_else(|| "seq_i64_new()".to_string());
                    format!("logos_text_from_bytes({})", a)
                }
                "chr" => {
                    let a = args
                        .first()
                        .map(|a| codegen_expr(a, ctx))
                        .unwrap_or_else(|| "0".to_string());
                    format!("logos_chr({})", a)
                }
                "parseInt" => {
                    let arg = if let Some(a) = args.first() {
                        codegen_expr(a, ctx)
                    } else {
                        "\"0\"".to_string()
                    };
                    format!("logos_parseInt({})", arg)
                }
                "parseFloat" => {
                    let arg = if let Some(a) = args.first() {
                        codegen_expr(a, ctx)
                    } else {
                        "\"0\"".to_string()
                    };
                    format!("atof({})", arg)
                }
                "readReals" => {
                    let arg = args.first().map(|a| codegen_expr(a, ctx)).unwrap_or_else(|| "\"\"".to_string());
                    format!("logos_read_reals({arg})")
                }
                "sqrt" => {
                    let arg = if let Some(a) = args.first() {
                        codegen_expr(a, ctx)
                    } else {
                        "0.0".to_string()
                    };
                    format!("sqrt({})", arg)
                }
                // Fixed-width UNSIGNED wrap (word8/16/32/64): mask the argument to the width as an
                // unsigned value held in an i64 — the VM/RISC-V `arg_wrapping` semantics. Without these
                // arms the C backend emitted an undeclared `word32(...)` call (e.g. sha256).
                "word8" | "word16" | "word32" | "word64" => {
                    let arg = args
                        .first()
                        .map(|a| {
                            let value = codegen_expr(a, ctx);
                            if infer_expr_type(a, ctx) == CType::BigInt {
                                format!("lint_word_bits({value})")
                            } else {
                                narrow_i64_sink(a, value, ctx)
                            }
                        })
                        .unwrap_or_else(|| "0".to_string());
                    let cast = match raw_name {
                        "word8" => "uint8_t",
                        "word16" => "uint16_t",
                        "word32" => "uint32_t",
                        _ => "uint64_t",
                    };
                    format!("((int64_t)({cast})({arg}))")
                }
                // 32-bit unsigned shifts + rotates (the sha256 word32 bit-op family). Standard 32-bit
                // semantics with the shift amount masked to 5 bits (matches the VM/RISC-V builtins).
                "word32Shr" | "word32Shl" | "rotr" | "rotl" if args.len() == 2 => {
                    let x = codegen_expr(&args[0], ctx);
                    let n = codegen_expr(&args[1], ctx);
                    match raw_name {
                        "word32Shr" => format!("((int64_t)((uint32_t)({x}) >> (({n}) & 31)))"),
                        "word32Shl" => format!("((int64_t)(uint32_t)((uint32_t)({x}) << (({n}) & 31)))"),
                        "rotr" => format!(
                            "((int64_t)(uint32_t)(((uint32_t)({x}) >> (({n}) & 31)) | ((uint32_t)({x}) << ((32 - (({n}) & 31)) & 31))))"
                        ),
                        _ => format!(
                            "((int64_t)(uint32_t)(((uint32_t)({x}) << (({n}) & 31)) | ((uint32_t)({x}) >> ((32 - (({n}) & 31)) & 31))))"
                        ),
                    }
                }
                // 32-bit unsigned bitwise logic (word_and/or/xor) — two operands, unsigned-32 result.
                "word_and" | "word_or" | "word_xor" if args.len() == 2 => {
                    let a = codegen_expr(&args[0], ctx);
                    let b = codegen_expr(&args[1], ctx);
                    let op = match raw_name {
                        "word_and" => "&",
                        "word_or" => "|",
                        _ => "^",
                    };
                    format!("((int64_t)((uint32_t)({a}) {op} (uint32_t)({b})))")
                }
                // word_not = 32-bit bitwise complement; intOfWord32 reinterprets a word32 as a
                // non-negative int (both mask to 32 bits, held in an i64).
                "word_not" => {
                    let a = args
                        .first()
                        .map(|x| codegen_expr(x, ctx))
                        .unwrap_or_else(|| "0".to_string());
                    format!("((int64_t)(uint32_t)(~(uint32_t)({a})))")
                }
                // A word's value as an Int, as the VM reads it: the narrow widths as their unsigned value,
                // a `Word64` as the signed integer its 64 bits spell.
                "intOfWord8" | "intOfWord16" | "intOfWord32" | "intOfWord64" => {
                    let a = args
                        .first()
                        .map(|x| codegen_expr(x, ctx))
                        .unwrap_or_else(|| "0".to_string());
                    match raw_name {
                        "intOfWord8" => format!("((int64_t)(uint8_t)({a}))"),
                        "intOfWord16" => format!("((int64_t)(uint16_t)({a}))"),
                        "intOfWord32" => format!("((int64_t)(uint32_t)({a}))"),
                        _ => format!("((int64_t)({a}))"),
                    }
                }
                "complex"
                    if args.len() == 2
                        && infer_expr_type(&args[0], ctx) != CType::Float64
                        && infer_expr_type(&args[1], ctx) != CType::Float64 =>
                {
                    // exact int-part complex → LComplex (the C mirror of LogosComplex).
                    format!(
                        "cplx_ints({}, {})",
                        codegen_expr(&args[0], ctx),
                        codegen_expr(&args[1], ctx)
                    )
                }
                // float-part complex → LComplex over exact dyadic rationals (mirrors LogosComplexF).
                "complex" if args.len() == 2 => {
                    format!(
                        "cplx_reals((double)({}), (double)({}))",
                        codegen_expr(&args[0], ctx),
                        codegen_expr(&args[1], ctx)
                    )
                }
                // Real / imaginary part of a complex, extracted by value as an f64.
                "re" if args.len() == 1 && infer_expr_type(&args[0], ctx) == CType::Complex => {
                    format!("cplx_re_f64({})", codegen_expr(&args[0], ctx))
                }
                "re" if args.len() == 1 => format!("((double)({}))", codegen_expr(&args[0], ctx)),
                "im" if args.len() == 1 && infer_expr_type(&args[0], ctx) == CType::Complex => {
                    format!("cplx_im_f64({})", codegen_expr(&args[0], ctx))
                }
                "im" if args.len() == 1 => {
                    let _ = codegen_expr(&args[0], ctx);
                    "0.0".to_string()
                }
                "abs"
                    if args
                        .first()
                        .map_or(false, |a| infer_expr_type(a, ctx) == CType::Complex) =>
                {
                    format!("cplx_abs_f64({})", codegen_expr(&args[0], ctx))
                }
                // |x| of an exact Rational/Decimal stays exact (`LDec`, sign cleared).
                "abs"
                    if args
                        .first()
                        .is_some_and(|a| matches!(infer_expr_type(a, ctx), CType::Rational | CType::Decimal)) =>
                {
                    format!("ldec_abs({})", codegen_expr(&args[0], ctx))
                }
                // `decimal("1.50")` — scale-aware Decimal parsed from its digit string (`LDec`).
                "decimal" if args.len() == 1 && infer_expr_type(&args[0], ctx) == CType::String => {
                    format!("ldec_from_text({})", codegen_expr(&args[0], ctx))
                }
                "decimal" if args.len() == 1 => {
                    // scale-0 decimal (`LDec`) — the C mirror of LOGOS `decimal(n)`.
                    format!("ldec_from_i64((int64_t)({}))", codegen_expr(&args[0], ctx))
                }
                "modular"
                    if args.len() == 2
                        && infer_expr_type(&args[0], ctx) != CType::Float64
                        && infer_expr_type(&args[1], ctx) != CType::Float64 =>
                {
                    // exact ℤ/nℤ → LMod (the C mirror of logicaffeine_base::Modular).
                    format!(
                        "mod_ints((int64_t)({}), (int64_t)({}))",
                        codegen_expr(&args[0], ctx),
                        codegen_expr(&args[1], ctx)
                    )
                }
                // UUID (the C mirror of logicaffeine_base::Uuid): parse/constants/version/bytes.
                "uuid" if args.len() == 1 => format!("uuid_parse({})", codegen_expr(&args[0], ctx)),
                "uuid_nil" if args.is_empty() => "uuid_nil()".to_string(),
                "uuid_max" if args.is_empty() => "uuid_max()".to_string(),
                "uuid_dns" if args.is_empty() => "uuid_dns()".to_string(),
                "uuid_url" if args.is_empty() => "uuid_url()".to_string(),
                "uuid_oid" if args.is_empty() => "uuid_oid()".to_string(),
                "uuid_x500" if args.is_empty() => "uuid_x500()".to_string(),
                "uuid_version" if args.len() == 1 => {
                    format!("uuid_version({})", codegen_expr(&args[0], ctx))
                }
                "uuid_bytes" if args.len() == 1 => {
                    format!("uuid_bytes({})", codegen_expr(&args[0], ctx))
                }
                "uuid_from_bytes" if args.len() == 1 => {
                    format!("uuid_from_bytes({})", codegen_expr(&args[0], ctx))
                }
                // `quantity(v, "unit")` → `LQuant`; the unit resolves to its exact scale/offset/dimension
                // at codegen time. An unresolvable (non-literal or unknown) unit fails loud at gcc.
                "quantity" if args.len() == 2 => {
                    match quantity_unit_literal(&args[1], ctx)
                        .and_then(|n| resolve_quantity_unit(&n))
                    {
                        Some((scale, offset, dnums, ddens, sym)) => format!(
                            "quant_of(rat_from_i64((int64_t)({})), {}, {}, {}, {}, {})",
                            codegen_expr(&args[0], ctx),
                            scale,
                            offset,
                            dnums,
                            ddens,
                            sym
                        ),
                        None => {
                            format!("quantity_unresolved_unit({})", codegen_expr(&args[0], ctx))
                        }
                    }
                }
                // `convert(q, "unit")` (and the `X in unit` surface) → re-express in a same-dimension unit.
                "convert" if args.len() == 2 => {
                    match quantity_unit_literal(&args[1], ctx)
                        .and_then(|n| resolve_quantity_unit(&n))
                    {
                        Some((scale, offset, dnums, ddens, sym)) => format!(
                            "quant_convert({}, {}, {}, {}, {}, {})",
                            codegen_expr(&args[0], ctx),
                            scale,
                            offset,
                            dnums,
                            ddens,
                            sym
                        ),
                        None => {
                            format!("quantity_unresolved_unit({})", codegen_expr(&args[0], ctx))
                        }
                    }
                }
                // `money(amount, "USD")` → `LMoney`; a bare integer amount promotes to a scale-0 decimal.
                "money" if args.len() == 2 => {
                    let amt = if matches!(infer_expr_type(&args[0], ctx), CType::Rational | CType::Decimal) {
                        codegen_expr(&args[0], ctx)
                    } else {
                        format!("ldec_from_i64((int64_t)({}))", codegen_expr(&args[0], ctx))
                    };
                    format!("money_of({}, {})", amt, codegen_expr(&args[1], ctx))
                }
                "abs" => {
                    let arg = if let Some(a) = args.first() {
                        codegen_expr(a, ctx)
                    } else {
                        "0".to_string()
                    };
                    format!("fabs({})", arg)
                }
                "count_ones" => {
                    // Population count over the value's u64 bit pattern — matches the
                    // AOT-Rust `(x as u64).count_ones()` and the interpreter/VM.
                    let arg = args.first().map_or_else(
                        || "0".to_string(),
                        |a| narrow_i64_sink(a, codegen_expr(a, ctx), ctx),
                    );
                    format!(
                        "(int64_t)__builtin_popcountll((unsigned long long)({}))",
                        arg
                    )
                }
                "leading_zeros" => {
                    // High zero bits; `__builtin_clzll(0)` is UB, so match Rust's
                    // `(0u64).leading_zeros() == 64` explicitly.
                    let arg = args.first().map_or_else(
                        || "0".to_string(),
                        |a| narrow_i64_sink(a, codegen_expr(a, ctx), ctx),
                    );
                    format!("((unsigned long long)({0})==0ULL?(int64_t)64:(int64_t)__builtin_clzll((unsigned long long)({0})))", arg)
                }
                "trailing_zeros" => {
                    // Low zero bits; `__builtin_ctzll(0)` is UB → Rust's
                    // `(0u64).trailing_zeros() == 64`.
                    let arg = args.first().map_or_else(
                        || "0".to_string(),
                        |a| narrow_i64_sink(a, codegen_expr(a, ctx), ctx),
                    );
                    format!("((unsigned long long)({0})==0ULL?(int64_t)64:(int64_t)__builtin_ctzll((unsigned long long)({0})))", arg)
                }
                // `floor`/`ceil`/`round` of an exact Rational/Decimal yield the exact integer (`lint_norm`
                // downsizes it to a fast small when it fits). `round` is HALF-AWAY-FROM-ZERO, matching the
                // interpreter's `Rational::round`. A float argument keeps the libm form below.
                "floor"
                    if args
                        .first()
                        .is_some_and(|a| matches!(infer_expr_type(a, ctx), CType::Rational | CType::Decimal)) =>
                {
                    format!("lint_norm(ldec_floor({}))", codegen_expr(&args[0], ctx))
                }
                "ceil"
                    if args
                        .first()
                        .is_some_and(|a| matches!(infer_expr_type(a, ctx), CType::Rational | CType::Decimal)) =>
                {
                    format!("lint_norm(ldec_ceil({}))", codegen_expr(&args[0], ctx))
                }
                "round"
                    if args
                        .first()
                        .is_some_and(|a| matches!(infer_expr_type(a, ctx), CType::Rational | CType::Decimal)) =>
                {
                    format!("lint_norm(ldec_round({}))", codegen_expr(&args[0], ctx))
                }
                "floor" => {
                    let arg = if let Some(a) = args.first() {
                        codegen_expr(a, ctx)
                    } else {
                        "0.0".to_string()
                    };
                    format!("(int64_t)floor({})", arg)
                }
                "ceil" => {
                    let arg = if let Some(a) = args.first() {
                        codegen_expr(a, ctx)
                    } else {
                        "0.0".to_string()
                    };
                    format!("(int64_t)ceil({})", arg)
                }
                "round" => {
                    let arg = if let Some(a) = args.first() {
                        codegen_expr(a, ctx)
                    } else {
                        "0.0".to_string()
                    };
                    format!("(int64_t)round({})", arg)
                }
                // Modular exponentiation: `pow(m, e)` where the base is a ℤ/nℤ element — the C mirror
                // of the interpreter's `Modular::pow` (square-and-multiply in the ring, exponent ≥ 0).
                "pow" if args.len() == 2 && infer_expr_type(&args[0], ctx) == CType::Modular => {
                    format!(
                        "mod_pow({}, (int64_t)({}))",
                        codegen_expr(&args[0], ctx),
                        codegen_expr(&args[1], ctx)
                    )
                }
                "pow" => {
                    let base_arg = args.first();
                    let exp_arg = args.get(1);
                    let base = base_arg
                        .map(|a| codegen_expr(a, ctx))
                        .unwrap_or_else(|| "0.0".to_string());
                    let exp = exp_arg
                        .map(|a| codegen_expr(a, ctx))
                        .unwrap_or_else(|| "1.0".to_string());
                    // Integer pow is EXACT and promotes on overflow (matching the interpreter), not a
                    // lossy float `pow` nor a wrapping `uint64_t`. When both operands are integer-typed,
                    // raise the base through the arbitrary-precision runtime (`lint_pow`), which
                    // downsizes a fitting result and promotes an overflow to the exact BigInt.
                    let is_int_pow = matches!(
                        (base_arg, exp_arg),
                        (Some(b), Some(e))
                            if !matches!(infer_expr_type(b, ctx), CType::Float64)
                                && !matches!(infer_expr_type(e, ctx), CType::Float64)
                    );
                    if is_int_pow {
                        let bt = base_arg
                            .map(|a| infer_expr_type(a, ctx))
                            .unwrap_or(CType::Int64);
                        let et = exp_arg
                            .map(|a| infer_expr_type(a, ctx))
                            .unwrap_or(CType::Int64);
                        let base_l = if bt == CType::BigInt {
                            base
                        } else {
                            format!("lint_of((int64_t)({}))", base)
                        };
                        let exp_i = if et == CType::BigInt {
                            format!("lint_expect_i64({})", exp)
                        } else {
                            format!("(int64_t)({})", exp)
                        };
                        format!("lint_pow({}, {})", base_l, exp_i)
                    } else {
                        format!("pow((double)({}), (double)({}))", base, exp)
                    }
                }
                "min" | "max" => {
                    let operand = |k: usize| {
                        args.get(k).map_or_else(
                            || ("0".to_string(), CType::Int64),
                            |a| (codegen_expr(a, ctx), infer_expr_type(a, ctx)),
                        )
                    };
                    let ((a, at), (b, bt)) = (operand(0), operand(1));
                    let order = if raw_name == "min" { "<" } else { ">" };
                    let exact = (at == CType::BigInt || bt == CType::BigInt)
                        && at != CType::Float64
                        && bt != CType::Float64;
                    if exact {
                        // The VM's `min`/`max` choose between two machine-word Ints and refuse anything else,
                        // a big one included, in its own words.
                        let big = crate::interpreter::RuntimeValue::from_bigint(
                            logicaffeine_base::BigInt::from_i64(i64::MIN)
                                .mul(&logicaffeine_base::BigInt::from_i64(2)),
                        );
                        let builtin = if raw_name == "min" {
                            crate::semantics::builtins::BuiltinId::Min
                        } else {
                            crate::semantics::builtins::BuiltinId::Max
                        };
                        let refusal = crate::semantics::builtins::call_builtin(
                            builtin,
                            vec![big, crate::interpreter::RuntimeValue::Int(0)],
                        )
                        .err()
                        .unwrap_or_default();
                        let lint = |v: &str, t: &CType| {
                            if *t == CType::BigInt {
                                v.to_string()
                            } else {
                                format!("lint_of((int64_t)({v}))")
                            }
                        };
                        format!(
                            "({{ LInt __lhs = {}; LInt __rhs = {}; if (__lhs.big || __rhs.big) {{ {} }} lint_of(__lhs.small {order} __rhs.small ? __lhs.small : __rhs.small); }})",
                            lint(&a, &at),
                            lint(&b, &bt),
                            ctx.fail_stmt(&c_string_literal(&refusal))
                        )
                    } else if at == CType::Float64 || bt == CType::Float64 {
                        // A Real on either side: the language's own `min`/`max` of reals, which decides the
                        // sign of equal zeros and a NaN as every other tier does.
                        format!("logos_{raw_name}_f64((double)({a}), (double)({b}))")
                    } else {
                        // Evaluate each argument EXACTLY once (a plain ternary double-evaluates the selected
                        // operand, duplicating any side effects). A GCC statement expression with __typeof__
                        // binds each operand to a temp first.
                        format!(
                            "({{ __typeof__({a}) __lhs = ({a}); __typeof__({b}) __rhs = ({b}); __lhs {order} __rhs ? __lhs : __rhs; }})"
                        )
                    }
                }
                // Temporal tower → the C runtime (codegen_c/runtime.rs). One-argument builtins:
                "parse_timestamp" | "format_timestamp" | "year_of" | "month_of" | "day_of"
                | "hour_of" | "minute_of" | "second_of" | "weekday_of" | "week_of"
                | "quarter_of" | "date_of" | "time_of" => {
                    let cfn = match raw_name {
                        "parse_timestamp" => "logos_parse_timestamp",
                        "format_timestamp" => "logos_format_rfc3339",
                        "year_of" => "logos_year_of",
                        "month_of" => "logos_month_of",
                        "day_of" => "logos_day_of",
                        "hour_of" => "logos_hour_of",
                        "minute_of" => "logos_minute_of",
                        "second_of" => "logos_second_of",
                        "weekday_of" => "logos_weekday_of",
                        "week_of" => "logos_week_of",
                        "quarter_of" => "logos_quarter_of",
                        "date_of" => "logos_date_of",
                        "time_of" => "logos_time_of",
                        _ => unreachable!(),
                    };
                    let a = args
                        .first()
                        .map(|a| codegen_expr(a, ctx))
                        .unwrap_or_else(|| "0".to_string());
                    // A `Date` is days-since-epoch; the Moment-based extractors expect nanos, so feed a
                    // Date through a midnight-UTC conversion (`date_of` stays identity via the round-trip:
                    // `(days*NANOS_PER_DAY) / NANOS_PER_DAY == days`). A `Moment` is already nanos, and
                    // `parse_timestamp`/`format_timestamp` take a Text/Moment (never a Date) — unchanged.
                    let arg_is_date = args
                        .first()
                        .map(|e| infer_expr_type(e, ctx) == CType::Date)
                        .unwrap_or(false);
                    if arg_is_date {
                        format!("{}(({}) * LOGOS_NANOS_PER_DAY)", cfn, a)
                    } else {
                        format!("{}({})", cfn, a)
                    }
                }
                // Civil-component constructors (3-arg Date, 6-arg Moment).
                "makeDate" if args.len() == 3 => {
                    let a: Vec<String> = args.iter().map(|e| codegen_expr(e, ctx)).collect();
                    format!("logos_make_date({}, {}, {})", a[0], a[1], a[2])
                }
                "makeMoment" if args.len() == 6 => {
                    let a: Vec<String> = args.iter().map(|e| codegen_expr(e, ctx)).collect();
                    format!(
                        "logos_make_moment({}, {}, {}, {}, {}, {})",
                        a[0], a[1], a[2], a[3], a[4], a[5]
                    )
                }
                // Two-argument temporal builtins.
                "seconds_between" | "add_seconds" | "months_between" | "years_between" => {
                    let cfn = match raw_name {
                        "seconds_between" => "logos_seconds_between",
                        "months_between" => "logos_months_between",
                        "years_between" => "logos_years_between",
                        _ => "logos_add_seconds",
                    };
                    let a = args
                        .first()
                        .map(|a| codegen_expr(a, ctx))
                        .unwrap_or_else(|| "0".to_string());
                    let b = args
                        .get(1)
                        .map(|a| codegen_expr(a, ctx))
                        .unwrap_or_else(|| "0".to_string());
                    format!("{}({}, {})", cfn, a, b)
                }
                _ => {
                    let fname = escape_c_ident(raw_name);
                    // Two exact-integer promotions: (a) calling a scalar EXACT-HOF — re-codegen its closure arg
                    // as exact + promote an i64 scalar arg to `LInt`; (b) calling a fn-typed PARAM `f` whose
                    // params are exact — promote an i64 arg (`f(i64::MAX)` inside a HOF body).
                    let exact_hof = ctx.exact_hofs.contains(function);
                    let fnptr_params: Vec<CType> = match ctx.vars.get(function) {
                        Some(CType::FnPtr { params, .. }) => params.clone(),
                        _ => Vec::new(),
                    };
                    let args_str: Vec<String> = args
                        .iter()
                        .enumerate()
                        .map(|(i, a)| {
                            if exact_hof {
                                if let Expr::Closure {
                                    params,
                                    body,
                                    return_type,
                                } = a
                                {
                                    return codegen_closure(params, body, *return_type, ctx);
                                }
                                let ac = codegen_expr(a, ctx);
                                if matches!(infer_expr_type(a, ctx), CType::Int64) {
                                    return format!("lint_of((int64_t)({}))", ac);
                                }
                                return ac;
                            }
                            let ac = codegen_expr(a, ctx);
                            if fnptr_params.get(i) == Some(&CType::BigInt)
                                && infer_expr_type(a, ctx) != CType::BigInt
                            {
                                format!("lint_of((int64_t)({}))", ac)
                            } else {
                                as_param_value(ac, a, *function, i, ctx)
                            }
                        })
                        .collect();
                    // A variable holding a function value is called through it.
                    if let Some(CType::FnPtr { ret, params }) = ctx.vars.get(function) {
                        return closure_call(
                            &ctx.resolve(*function),
                            true,
                            ret,
                            params,
                            &args_str,
                            ctx,
                        );
                    }
                    format!("{}({})", fname, args_str.join(", "))
                }
            }
        }
        // A tuple / record literal → a compound-literal `(LTuple_…){a, b}`; the typedef was emitted from the
        // enclosing function's signature. Element C types are inferred to name the struct.
        Expr::Tuple(items) => {
            let elems: Vec<CType> = items.iter().map(|e| infer_expr_type(e, ctx)).collect();
            let name = tuple_c_name(&elems, ctx.interner);
            let vals: Vec<String> = items
                .iter()
                .map(|e| stored(e, codegen_expr(e, ctx), ctx))
                .collect();
            format!("({}){{{}}}", name, vals.join(", "))
        }
        Expr::Index { collection, index } => {
            // One rule for every collection: its type, then its accessor. Only a named variable is
            // addressable (`seq_*_get(&xs, i)`); any other collection — a field, a call's result —
            // is bound to a statement-expression temporary first, since `&rvalue` is illegal C.
            // A value-or-nothing variable is the collection it holds; the index is computed before the VM finds
            // that it holds nothing.
            let (coll, ty, addressable, absent) = match collection {
                _ if operand_place(collection, ctx) => {
                    let operand = collection_operand(
                        collection,
                        || {
                            use crate::interpreter::RuntimeValue;
                            crate::semantics::collections::index_get(&RuntimeValue::Nothing, &RuntimeValue::Int(1))
                                .map(|_| ())
                        },
                        ctx,
                    );
                    let ty = operand.ty.clone().unwrap_or(CType::SeqI64(Ints::Int));
                    (operand.place, ty, true, operand.absent)
                }
                _ => (
                    codegen_expr(collection, ctx),
                    infer_expr_type(collection, ctx),
                    false,
                    None,
                ),
            };
            let rendered = codegen_expr(index, ctx);
            let idx = match &ty {
                CType::String => narrow_index(
                    index,
                    rendered,
                    ctx,
                    crate::semantics::failcode::FailCode::TextIndexTooWide,
                ),
                CType::MapI64I64(_, _)
                | CType::MapStrI64(_)
                | CType::MapStrStr
                | CType::MapI64Str(_)
                | CType::MapStrValue(_)
                | CType::MapI64Value(_) => narrow_i64_sink(index, rendered, ctx),
                _ => narrow_index(
                    index,
                    rendered,
                    ctx,
                    crate::semantics::failcode::FailCode::ListIndexTooWide,
                ),
            };
            let (idx, computed) = match &absent {
                None => (idx, None),
                Some(_) => ("__index".to_string(), Some(idx)),
            };
            let read = (|| {
            if let Expr::Identifier(sym) = collection {
                if ctx.text_bytes_views.contains_key(sym) {
                    return format!(
                        "(int64_t)((const unsigned char *){coll})[logos_idx0({idx}, strlen((const char *){coll}))]"
                    );
                }
            }

            match &ty {
                // `item N of t` on a tuple (1-based) → its field `f{N-1}` (the index is a constant).
                CType::Tuple(elems) => tuple_item(&coll, elems, index, ctx),
                CType::Record(fields) => {
                    let elems: Vec<CType> = fields.iter().map(|(_, t)| t.clone()).collect();
                    tuple_item(&coll, &elems, index, ctx)
                }
                // `item k of <Text>` → its k-th character, counted from the end for a negative k.
                CType::String => match collection {
                    Expr::Identifier(sym) if is_text_builder(*sym, ctx) => {
                        format!("logos_text_builder_char_at({coll}, {idx})")
                    }
                    _ => format!("logos_text_char_at({coll}, {idx})"),
                },
                _ => {
                    // The versioning pass records `InBounds` only on the fast clone whose
                    // hoisted guard proves this exact 1-based access names an element.  Rust
                    // consumes that fact with unchecked indexing; consume it here too instead
                    // of silently putting `logos_idx0` back into every C hot-loop load.
                    let proven_direct = crate::optimize::active_config()
                        .is_on(crate::optimization::Opt::Unchecked)
                        && crate::optimize::facts::holds_expr(
                            crate::optimize::facts::Fact::InBounds,
                            index,
                        )
                        && addressable
                        && matches!(
                            &ty,
                            CType::SeqI64(_)
                                | CType::SeqBool
                                | CType::SeqStr
                                | CType::SeqF64
                                | CType::SeqStruct(_)
                                | CType::SeqTuple(_)
                                | CType::SeqSeq(_)
                                | CType::SeqValue(_)
                        );
                    if proven_direct {
                        crate::optimize::mark_fired(crate::optimization::Opt::Unchecked);
                        return format!("({coll}).data[(size_t)(({idx}) - 1LL)]");
                    }
                    let getfn = match &ty {
                        CType::MapI64I64(_, _) => "map_i64_i64_get".to_string(),
                        CType::MapStrI64(_) => "map_str_i64_get".to_string(),
                        CType::MapStrStr => "map_str_str_get".to_string(),
                        CType::MapI64Str(_) => "map_i64_str_get".to_string(),
                        CType::MapStrValue(inner) => {
                            format!("map_str_{}_get", c_type_str_resolved(inner, ctx.interner))
                        }
                        CType::MapI64Value(inner) => {
                            format!("map_i64_{}_get", c_type_str_resolved(inner, ctx.interner))
                        }
                        CType::SeqBool
                        | CType::SeqStr
                        | CType::SeqF64
                        | CType::SeqStruct(_)
                        | CType::SeqTuple(_)
                        | CType::SeqSeq(_)
                        | CType::SeqValue(_) => {
                            let seq = c_type_str_resolved(&ty, ctx.interner);
                            format!("seq_{}_get", seq.strip_prefix("Seq_").unwrap_or(&seq))
                        }
                        _ => "seq_i64_get".to_string(),
                    };
                    if addressable {
                        format!("{}(&{}, {})", getfn, coll, idx)
                    } else {
                        let cty = match &ty {
                            CType::MapI64I64(_, _)
                            | CType::MapStrI64(_)
                            | CType::MapStrStr
                            | CType::MapI64Str(_)
                            | CType::MapStrValue(_)
                            | CType::MapI64Value(_)
                            | CType::SeqBool
                            | CType::SeqStr
                            | CType::SeqF64
                            | CType::SeqStruct(_)
                            | CType::SeqTuple(_)
                            | CType::SeqSeq(_)
                            | CType::SeqValue(_) => c_type_str_resolved(&ty, ctx.interner),
                            _ => "Seq_i64".to_string(),
                        };
                        format!(
                            "({{ {} __idx_tmp = {}; {}(&__idx_tmp, {}); }})",
                            cty, coll, getfn, idx
                        )
                    }
                }
            }
            })();
            match (absent, computed) {
                (Some(check), Some(index)) => format!("({{ __auto_type __index = {index}; {check} {read}; }})"),
                _ => read,
            }
        }
        Expr::Length { collection } => {
            if let Expr::Identifier(sym) = collection {
                if ctx.text_bytes_views.contains_key(sym) {
                    let coll = ctx.resolve(*sym);
                    return format!("(int64_t)strlen((const char *){coll})");
                }
            }
            if operand_place(collection, ctx) {
                let operand = collection_operand(
                    collection,
                    || crate::semantics::collections::length_of(&crate::interpreter::RuntimeValue::Nothing).map(|_| ()),
                    ctx,
                );
                let coll = operand.place.clone();
                operand.read(match operand.ty.as_ref() {
                    Some(CType::SeqI64(_)) => format!("seq_i64_len(&{})", coll),
                    Some(CType::SeqBool) => format!("seq_bool_len(&{})", coll),
                    Some(CType::SeqStr) => format!("seq_str_len(&{})", coll),
                    Some(CType::SeqF64) => format!("seq_f64_len(&{})", coll),
                    Some(CType::String) if matches!(collection, Expr::Identifier(sym) if is_text_builder(*sym, ctx)) => {
                        format!("(int64_t)logos_text_builder_len({coll})")
                    }
                    Some(CType::String) => format!("str_len({})", coll),
                    Some(CType::SetI64(_)) => format!("set_i64_len(&{})", coll),
                    Some(CType::SetStr) => format!("set_str_len(&{})", coll),
                    // `length of <map>` is the distinct-entry count (the struct's `len` field, bumped once per
                    // new key). Without this a Map fell to the `seq_i64_len` default and read a garbage field.
                    Some(CType::MapI64I64(_, _))
                    | Some(CType::MapStrI64(_))
                    | Some(CType::MapStrStr)
                    | Some(CType::MapI64Str(_))
                    | Some(CType::MapStrValue(_))
                    | Some(CType::MapI64Value(_)) => format!("(int64_t)({}).len", coll),
                    Some(CType::SeqStruct(s)) => {
                        format!("{}(&{})", seq_struct_fn(*s, "len", ctx.interner), coll)
                    }
                    Some(CType::SeqValue(inner)) => {
                        format!("seq_{}_len(&{})", c_type_str(inner), coll)
                    }
                    // A nested `Seq of Seq of …` uses the `seq_Seq_<inner>_len` family (matching the
                    // SeqSeq get at the top of this match), not the flat `seq_i64_len` default.
                    Some(CType::SeqSeq(inner)) => format!(
                        "seq_{}_len(&{})",
                        c_type_str_resolved(inner, ctx.interner),
                        coll
                    ),
                    _ => format!("seq_i64_len(&{})", coll),
                })
            } else {
                let coll = codegen_expr(collection, ctx);
                let ty = infer_expr_type(collection, ctx);
                if ty == CType::String {
                    return format!("str_len({})", coll);
                }
                if let CType::SeqStruct(s) = ty {
                    return format!(
                        "({{ {} __len_tmp = {}; {}(&__len_tmp); }})",
                        seq_struct_type(s, ctx.interner),
                        coll,
                        seq_struct_fn(s, "len", ctx.interner)
                    );
                }
                if let CType::SeqValue(inner) = &ty {
                    let e = c_type_str(inner);
                    return format!(
                        "({{ Seq_{e} __len_tmp = {}; seq_{e}_len(&__len_tmp); }})",
                        coll
                    );
                }
                // The `*_len` helpers take a pointer, but the collection here may be an rvalue (a
                // call result like `length of split(…)`), and `&rvalue` is illegal C. Bind it in a
                // GCC statement-expression first (the backend already uses this form elsewhere).
                let (cty, lenfn) = match ty {
                    CType::SeqBool => ("Seq_bool", "seq_bool_len"),
                    CType::SeqStr => ("Seq_str", "seq_str_len"),
                    CType::SeqF64 => ("Seq_f64", "seq_f64_len"),
                    CType::SetI64(_) => ("Set_i64", "set_i64_len"),
                    CType::SetStr => ("Set_str", "set_str_len"),
                    _ => ("Seq_i64", "seq_i64_len"),
                };
                format!(
                    "({{ {} __len_tmp = {}; {}(&__len_tmp); }})",
                    cty, coll, lenfn
                )
            }
        }
        Expr::Contains { collection, value } => c_contains(collection, value, ctx),
        Expr::New {
            type_name,
            type_args,
            init_fields,
            ..
        } => {
            let name = ctx.interner.resolve(*type_name);
            match name {
                // The sequence's own family, named after its type exactly as `infer_expr_type` types it
                // (`seq_of` the element): a user record or enum, a tuple, a nested sequence, a value type.
                "Seq" | "List" => {
                    let elem = type_args
                        .first()
                        .map(|arg| {
                            resolve_type_expr_with_registry(arg, ctx.interner, Some(ctx.registry))
                        })
                        .unwrap_or(CType::Int64);
                    let seq = seq_of(elem);
                    if seq == CType::Tower {
                        return "ltower_list(0, NULL)".to_string();
                    }
                    format!(
                        "{}()",
                        seq_fn(&seq, "new", ctx.interner)
                            .unwrap_or_else(|| "seq_i64_new".to_string())
                    )
                }
                "Map" => {
                    let key_type = type_args
                        .first()
                        .map(|p| resolve_type_expr(p, ctx.interner))
                        .unwrap_or(CType::Int64);
                    let val_type = type_args
                        .get(1)
                        .map(|p| resolve_type_expr(p, ctx.interner))
                        .unwrap_or(CType::Int64);
                    map_new(&map_ctype(&key_type, &val_type), ctx.interner)
                        .expect("a map type has a constructor")
                }
                "Set" => {
                    if let Some(arg) = type_args.first() {
                        match resolve_type_expr(arg, ctx.interner) {
                            CType::String => "set_str_new()".to_string(),
                            _ => "set_i64_new()".to_string(),
                        }
                    } else {
                        "set_i64_new()".to_string()
                    }
                }
                _ => {
                    // Check if this is a user-defined struct
                    if let Some(crate::analysis::TypeDef::Struct { .. }) =
                        ctx.registry.get(*type_name)
                    {
                        let escaped = escape_c_ident(name);
                        // Every counter-CRDT field of this struct is seeded with a fresh per-replica map
                        // handle (`logos_crdt_new()`), stored in its `int64_t` cell as a cast pointer. A
                        // plain `New` (no init fields) still needs these — a zero cell would be a null
                        // handle. Explicit inits for non-counter fields are honored as before.
                        let counter_inits: Vec<String> = ctx
                            .counter_fields
                            .iter()
                            .filter(|(s, _)| *s == *type_name)
                            .map(|(_, fsym)| {
                                let fn_str = escape_c_ident(ctx.interner.resolve(*fsym));
                                format!(".{} = (int64_t)(intptr_t)logos_crdt_new()", fn_str)
                            })
                            .collect();
                        let declared = match ctx.registry.get(*type_name) {
                            Some(crate::analysis::TypeDef::Struct { fields, .. }) => {
                                fields.as_slice()
                            }
                            _ => &[],
                        };
                        let mut fields_str: Vec<String> = init_fields
                            .iter()
                            .map(|(fname, fexpr)| {
                                let fn_str = escape_c_ident(ctx.interner.resolve(*fname));
                                let fd = declared.iter().find(|f| f.name == *fname);
                                let val = as_field_value(
                                    stored(fexpr, codegen_expr(fexpr, ctx), ctx),
                                    fexpr,
                                    *type_name,
                                    fd,
                                    ctx,
                                );
                                format!(".{} = {}", fn_str, val)
                            })
                            .collect();
                        fields_str.extend(counter_inits);
                        // A field the construction leaves out holds its type's empty value, as the VM fills it: an
                        // empty text, set or map (a zeroed list is already an empty one, and a zero a number).
                        for fd in declared
                            .iter()
                            .filter(|fd| !init_fields.iter().any(|(n, _)| *n == fd.name))
                        {
                            let fty = super::exact::field_ctype(
                                *type_name,
                                fd,
                                ctx.interner,
                                ctx.registry,
                                &ctx.field_facts(),
                            );
                            let empty = match &fty {
                                // The VM holds nothing in a rational field a record is built without, which a C
                                // rational cannot hold.
                                CType::Rational => {
                                    return c_refuse_expr(&format!(
                                        "a `{name}` built without its field `{}`, a rational: a C rational holds no \
                                         nothing",
                                        ctx.interner.resolve(fd.name)
                                    ))
                                }
                                CType::String => Some("(char *)\"\"".to_string()),
                                // An exact field starts at nothing, as the VM's does.
                                CType::Tower => Some("ltower_nothing()".to_string()),
                                // A field a stored `nothing` reaches starts present with its type's empty value, or
                                // absent where the VM leaves it nothing (a rational).
                                CType::Opt(inner) => Some(match &**inner {
                                    CType::Rational | CType::Decimal => "((logos_opt){0, 0})".to_string(),
                                    held => {
                                        let empty = match held {
                                            CType::String => "(char *)\"\"".to_string(),
                                            CType::SetI64(_) => "set_i64_new()".to_string(),
                                            CType::SetStr => "set_str_new()".to_string(),
                                            held => map_new(held, ctx.interner).unwrap_or_else(|| {
                                                format!("({}){{0}}", c_type_str_resolved(held, ctx.interner))
                                            }),
                                        };
                                        format!("((logos_opt){{1, {}}})", opt_pack(held, &empty, ctx))
                                    }
                                }),
                                CType::SetI64(_) => Some("set_i64_new()".to_string()),
                                CType::SetStr => Some("set_str_new()".to_string()),
                                map => map_new(map, ctx.interner),
                            };
                            if let Some(empty) = empty {
                                fields_str.push(format!(
                                    ".{} = {empty}",
                                    escape_c_ident(ctx.interner.resolve(fd.name))
                                ));
                            }
                        }
                        if fields_str.is_empty() {
                            format!("({}){{0}}", escaped)
                        } else {
                            format!("({}){{{}}}", escaped, fields_str.join(", "))
                        }
                    } else {
                        format!("/* unsupported new {} */0", name)
                    }
                }
            }
        }
        Expr::WithCapacity { value, capacity } => {
            let cap = codegen_expr(capacity, ctx);
            let ty = infer_expr_type(value, ctx);
            match ty {
                CType::MapI64I64(_, _) => format!("map_i64_i64_with_capacity({})", cap),
                CType::MapStrI64(_) => format!("map_str_i64_with_capacity({})", cap),
                CType::MapStrStr => format!("map_str_str_with_capacity({})", cap),
                CType::MapI64Str(_) => format!("map_i64_str_with_capacity({})", cap),
                CType::SeqI64(_) => format!("seq_i64_with_capacity({})", cap),
                CType::SeqBool => format!("seq_bool_with_capacity({})", cap),
                CType::SeqStr => format!("seq_str_with_capacity({})", cap),
                CType::SeqF64 => format!("seq_f64_with_capacity({})", cap),
                // A text's capacity is a hint to a text built in place (see `text_builder_of`); here it is read
                // for its effects and the text is itself.
                CType::String => format!("({{ (void)({}); {}; }})", cap, codegen_expr(value, ctx)),
                _ => codegen_expr(value, ctx),
            }
        }
        Expr::List(elems) if mixed_kinds(elems, ctx) => c_refuse_expr(MIXED_LIST),
        Expr::List(elems) => {
            let elem_type = super::types::list_item_type(elems, ctx);
            if elem_type == CType::Rational {
                let seq = crate::codegen_c::types::seq_of(CType::Rational);
                return literal_as(elems, &seq, &CType::Rational, ctx);
            }
            // The literal's sequence type is `seq_of` its first element's — the one mapping every
            // `Seq of X` is typed through — and every sequence family is spelled from its type's
            // name: `Seq_<key>` with `seq_<key>_new` / `seq_<key>_push` (a scalar's runtime family,
            // a record's or enum's, a tuple's, a value type's, a nested sequence's alike).
            let cty =
                c_type_str_resolved(&crate::codegen_c::types::seq_of(elem_type), ctx.interner);
            let key = cty.strip_prefix("Seq_").unwrap_or(&cty);
            let (new_fn, push_fn) = (format!("seq_{key}_new"), format!("seq_{key}_push"));
            if elems.is_empty() {
                return format!("{}()", new_fn);
            }
            // Materialize the literal in a GCC statement-expression so it works in ANY position —
            // an argument (`sumSeq([1,2,3])`), a return, etc. The old form emitted an empty seq
            // with the elements only in a comment, so every literal passed by value was empty.
            let mut s = format!("({{ {} __lit = {}(); ", cty, new_fn);
            for e in elems {
                // An exact (`LInt`) element of a `Seq of Int` narrows into its `Seq_i64` slot, as `Push` does.
                let value = if key == "i64" {
                    narrow_i64_sink(e, codegen_expr(e, ctx), ctx)
                } else {
                    stored(e, codegen_expr(e, ctx), ctx)
                };
                s.push_str(&format!("{}(&__lit, {}); ", push_fn, value));
            }
            s.push_str("__lit; })");
            s
        }
        // `copy of x`: a value sharing nothing with `x`, as the VM's deep clone.
        Expr::Copy { expr: inner } => {
            let ty = infer_expr_type(inner, ctx);
            super::values::copy_value(&ty, &codegen_expr(inner, ctx), ctx)
        }
        Expr::Give { value } => codegen_expr(value, ctx),
        // Boolean/truthiness negation `not x`. A Text negates its TRUTHINESS (empty = falsy, so `not ""` is
        // true) — matching the VM, NOT C pointer-truthiness (`!ptr` on a non-null `""` would wrongly be 0).
        Expr::Not { operand } => format!("(!{})", c_truthy(operand, ctx)),
        // Set union / intersection — `a union b` / `a intersection b`. Operands are `Let`-bound sets (lvalues),
        // so `&a`/`&b` are taken directly; the result is a fresh `Set_*` bound by the enclosing `Let`.
        Expr::Union { left, right } => {
            let (l, r) = (codegen_expr(left, ctx), codegen_expr(right, ctx));
            match infer_expr_type(left, ctx) {
                CType::SetStr => format!("set_str_union(&{}, &{})", l, r),
                _ => format!("set_i64_union(&{}, &{})", l, r),
            }
        }
        Expr::Intersection { left, right } => {
            let (l, r) = (codegen_expr(left, ctx), codegen_expr(right, ctx));
            match infer_expr_type(left, ctx) {
                CType::SetStr => format!("set_str_intersect(&{}, &{})", l, r),
                _ => format!("set_i64_intersect(&{}, &{})", l, r),
            }
        }
        Expr::FieldAccess { object, field } => {
            let obj = codegen_expr(object, ctx);
            let obj_ty = infer_expr_type(object, ctx);
            // A named-record field `r's quotient` → the positional `.fN` of the shared tuple struct.
            if let CType::Record(fields) = &obj_ty {
                if let Some(idx) = fields.iter().position(|(n, _)| n == field) {
                    return format!("{}.f{}", obj, idx);
                }
            }
            let fname = escape_c_ident(ctx.interner.resolve(*field));
            // A counter-CRDT field (`c's points`) holds a `logos_crdt*` handle in its `int64_t` cell; its
            // OBSERVABLE is the SUM of the per-replica map — project it exactly as the VM/tree-walker does.
            if let CType::Struct(sym) = &obj_ty {
                if ctx.is_counter_field(*sym, *field) {
                    return format!(
                        "logos_crdt_sum((logos_crdt *)(intptr_t)({}.{}))",
                        obj, fname
                    );
                }
            }
            format!("{}.{}", obj, fname)
        }
        Expr::NewVariant {
            enum_name,
            variant,
            fields,
        } => {
            let ename = escape_c_ident(ctx.interner.resolve(*enum_name));
            let vname = escape_c_ident(ctx.interner.resolve(*variant));
            if fields.is_empty() {
                format!("({}){{{}.tag = {}_{}}}", ename, "", ename, vname)
            } else {
                // Check which fields are recursive (self-referencing pointer)
                let variant_def_fields = ctx.registry.get(*enum_name).and_then(|td| {
                    if let crate::analysis::TypeDef::Enum { variants, .. } = td {
                        variants
                            .iter()
                            .find(|v| v.name == *variant)
                            .map(|v| &v.fields)
                    } else {
                        None
                    }
                });

                let recursive_fields: Vec<bool> = fields
                    .iter()
                    .map(|(fname, _)| {
                        variant_def_fields
                            .and_then(|vfields| vfields.iter().find(|f| f.name == *fname))
                            .is_some_and(|f| {
                                matches!(&f.ty, crate::analysis::FieldType::Named(fsym) if *fsym == *enum_name)
                            })
                    })
                    .collect();
                let recursive_count = recursive_fields
                    .iter()
                    .filter(|&&recursive| recursive)
                    .count();
                // Recursive payload pointers are never freed independently: they are immutable
                // value children owned by the containing enum. Put sibling children in one block
                // so a binary node pays for one allocator round trip instead of two.
                let batch_recursive = recursive_count > 1
                    && crate::optimize::active_config().is_on(crate::optimization::Opt::Scalarize);
                let recursive_storage =
                    batch_recursive.then(|| ctx.fresh_temporary("recursive_fields"));
                let mut recursive_index = 0usize;
                let mut recursive_assignments = Vec::with_capacity(recursive_count);
                let field_inits: Vec<String> = fields
                    .iter()
                    .zip(&recursive_fields)
                    .map(|((fname, fexpr), &is_recursive)| {
                        let fn_str = escape_c_ident(ctx.interner.resolve(*fname));
                        let fd = variant_def_fields
                            .and_then(|vfields| vfields.iter().find(|f| f.name == *fname));
                        let val = as_field_value(
                            stored(fexpr, codegen_expr(fexpr, ctx), ctx),
                            fexpr,
                            *variant,
                            fd,
                            ctx,
                        );

                        if is_recursive && batch_recursive {
                            let index = recursive_index;
                            recursive_index += 1;
                            let storage = recursive_storage
                                .as_deref()
                                .expect("batched recursive storage");
                            recursive_assignments.push(format!("{storage}[{index}] = {val};"));
                            format!(".{fn_str} = &{storage}[{index}]")
                        } else if is_recursive {
                            // Heap-allocate recursive field: ({Type *__p = lc_gc_alloc(sizeof(Type)); *__p = val; __p;})
                            format!(
                                ".{} = ({{{}* __p = ({}*)lc_gc_alloc(sizeof({})); *__p = {}; __p;}})",
                                fn_str, ename, ename, ename, val
                            )
                        } else {
                            format!(".{} = {}", fn_str, val)
                        }
                    })
                    .collect();
                let value = format!(
                    "({}){{{}.tag = {}_{}, .data.{} = {{{}}}}}",
                    ename,
                    "",
                    ename,
                    vname,
                    vname,
                    field_inits.join(", ")
                );
                if batch_recursive {
                    crate::optimize::mark_fired(crate::optimization::Opt::Scalarize);
                    let storage = recursive_storage
                        .as_deref()
                        .expect("batched recursive storage");
                    format!(
                        "({{ {ename} *{storage} = ({ename} *)lc_gc_alloc({recursive_count} * sizeof({ename})); {} {value}; }})",
                        recursive_assignments.join(" ")
                    )
                } else {
                    value
                }
            }
        }
        Expr::Slice {
            collection,
            start,
            end,
        } => c_slice(collection, start, end, ctx),
        Expr::InterpolatedString(parts) => codegen_interpolated_string_c(parts, ctx),
        // An `Escape` holds Rust source, which runs on the Rust backends only: here it fails as it does on the VM.
        Expr::Escape { .. } => {
            let failure = "Escape expressions contain raw Rust code and cannot be interpreted. \
                           Use `largo build` or `largo run` to compile and run this program.";
            format!(
                "({{ {} (int64_t)0; }})",
                ctx.fail_stmt(&c_string_literal(failure))
            )
        }
        // Lambda-lifting: a closure argument becomes a top-level `static` C function; the value
        // here is that function's name (a function pointer the higher-order callee invokes).
        Expr::Closure {
            params,
            body,
            return_type,
        } => {
            // Generic closure position (not an exact-HOF arg) — plain i64 integers; the exact-HOF call site
            // re-codegens its closure arg with `exact = true`.
            codegen_closure(params, body, *return_type, ctx)
        }
        // Calling a value (a function-pointer variable / closure param): `f(x)`.
        Expr::CallExpr { callee, args } => {
            let f = codegen_expr(callee, ctx);
            // The callee's function type says which params are exact `LInt` — promote an i64 argument there, so
            // `f(i64::MAX)` (a function-typed param inside an exact-HOF body) doesn't truncate.
            let (ret, param_tys): (CType, Vec<CType>) = match infer_expr_type(callee, ctx) {
                CType::FnPtr { ret, params } => (*ret, params),
                _ => (CType::Int64, Vec::new()),
            };
            let a: Vec<String> = args
                .iter()
                .enumerate()
                .map(|(i, x)| {
                    let xc = codegen_expr(x, ctx);
                    if param_tys.get(i) == Some(&CType::BigInt)
                        && infer_expr_type(x, ctx) != CType::BigInt
                    {
                        format!("lint_of((int64_t)({}))", xc)
                    } else {
                        xc
                    }
                })
                .collect();
            let is_variable = matches!(callee, Expr::Identifier(sym) if ctx.vars.contains_key(sym));
            closure_call(&f, is_variable, &ret, &param_tys, &a, ctx)
        }
        // Kinds this backend does not lower refuse at generation, naming the construct; a value it
        // cannot compute is never a stand-in `0`. (`some`/`none` reach C as core's `Option` enum
        // after monomorphization, so the two arms below fire only on a path that skipped it.)
        // A range used as a value is the list of the Ints from its start to its end. The walk stops AT the
        // end, never past it, so a range ending at the largest Int ends.
        Expr::Range { start, end } => {
            let n = ctx.select_ctr.get();
            ctx.select_ctr.set(n + 1);
            format!(
                "({{ int64_t __lo{n} = {}; int64_t __hi{n} = {}; Seq_i64 __r{n} = seq_i64_new(); \
                 if (__lo{n} <= __hi{n}) for (int64_t __logos_k{n} = __lo{n}; ; __logos_k{n}++) {{ seq_i64_push(&__r{n}, __logos_k{n}); \
                 if (__logos_k{n} == __hi{n}) break; }} __r{n}; }})",
                range_bound(start, ctx),
                range_bound(end, ctx),
            )
        }
        // A wire chunk is nothing on the VM, its operands never read.
        Expr::ChunkAt { .. } => "((logos_opt){0, 0})".to_string(),
        // A zone's manifest is the empty list on the VM, its zone never read.
        Expr::ManifestOf { .. } => "seq_i64_new()".to_string(),
        Expr::Dom(_) => c_refuse_expr("a page query"),
        Expr::View(_) => c_refuse_expr("a View"),
        Expr::OptionSome { .. } => c_refuse_expr("`some` outside a monomorphized program"),
        Expr::OptionNone => c_refuse_expr("`none` outside a monomorphized program"),
    }
}

/// `left op right` where an operand is a value-or-nothing variable (outside `is`, `and`, `or` and a joining of
/// text, which take nothing whole): both operands are computed in order, then a nothing fails as the VM's
/// arithmetic fails between Nothing and the other operand, and a value there is read as itself. `None` when no
/// operand may be nothing, or the operator takes nothing without failing.
fn opt_operand_binary(
    expr: &Expr,
    op: BinaryOpKind,
    left: &Expr,
    right: &Expr,
    ctx: &CContext,
) -> Option<String> {
    use crate::interpreter::RuntimeValue as V;
    // An operand that may be nothing: a value-or-nothing variable, or an expression of that type, computed once.
    let optional = |e: &Expr| opt_var(e, ctx).is_some() || matches!(infer_expr_type(e, ctx), CType::Opt(_));
    if !optional(left) && !optional(right) {
        return None;
    }
    let payload = |e: &Expr| match opt_var(e, ctx) {
        Some((_, ty)) => opt_element(&ty),
        None => match infer_expr_type(e, ctx) {
            CType::Opt(inner) => *inner,
            other => other,
        },
    };
    let (lt, rt) = (payload(left), payload(right));
    if matches!(
        op,
        BinaryOpKind::Eq | BinaryOpKind::NotEq | BinaryOpKind::Concat | BinaryOpKind::SeqConcat
    ) || (op == BinaryOpKind::Add && (lt == CType::String || rt == CType::String))
    {
        return None;
    }
    let failure = |l_absent: bool, r_absent: bool| -> Option<String> {
        let l = if l_absent {
            V::Nothing
        } else {
            sample_of(&lt, 1)?
        };
        let r = if r_absent {
            V::Nothing
        } else {
            sample_of(&rt, 1)?
        };
        crate::semantics::arith::binary_op(op, l, r).err()
    };
    let n = ctx.select_ctr.get();
    ctx.select_ctr.set(n + 1);
    let mut code = String::from("({ ");
    let mut cells = Vec::new();
    let mut unpacked = Vec::new();
    // Each operand's presence test, for one that may be nothing.
    let mut presence: [Option<String>; 2] = [None, None];
    for (i, operand) in [left, right].into_iter().enumerate() {
        let key = operand as *const Expr as usize;
        let element = if i == 0 { &lt } else { &rt };
        if let Some((var, _)) = opt_var(operand, ctx) {
            cells.push((key, super::types::opt_unpack(element, &format!("({var}).value"), ctx.interner)));
            presence[i] = Some(format!("({var}).present"));
        } else if optional(operand) {
            let cell = format!("__operand{n}_{i}");
            code.push_str(&format!("logos_opt {cell} = {}; ", codegen_expr(operand, ctx)));
            cells.push((key, super::types::opt_unpack(element, &format!("{cell}.value"), ctx.interner)));
            unpacked.push((key, element.clone()));
            presence[i] = Some(format!("{cell}.present"));
        } else if !matches!(operand, Expr::Literal(_) | Expr::Identifier(_)) {
            let cell = format!("__operand{n}_{i}");
            let ty = infer_expr_type(operand, ctx);
            code.push_str(&format!(
                "{} = {}; ",
                c_param_decl(&ty, &cell, ctx.interner),
                codegen_expr(operand, ctx)
            ));
            cells.push((key, cell));
        }
    }
    let mut checks = Vec::new();
    match (&presence[0], &presence[1]) {
        (Some(l), Some(r)) => {
            checks.push((format!("!{l} && !{r}"), failure(true, true)?));
            checks.push((format!("!{l}"), failure(true, false)?));
            checks.push((format!("!{r}"), failure(false, true)?));
        }
        (Some(l), None) => checks.push((format!("!{l}"), failure(true, false)?)),
        (None, Some(r)) => checks.push((format!("!{r}"), failure(false, true)?)),
        (None, None) => unreachable!("an operand may be nothing"),
    }
    let mut first = true;
    for (absent, failure) in &checks {
        let kw = if first { "if" } else { "else if" };
        first = false;
        code.push_str(&format!(
            "{kw} ({absent}) {{ {} }} ",
            ctx.fail_stmt(&c_string_literal(failure))
        ));
    }
    for (key, cell) in &cells {
        ctx.precomputed.borrow_mut().insert(*key, cell.clone());
    }
    for (key, ty) in &unpacked {
        ctx.unpacked.borrow_mut().insert(*key, ty.clone());
    }
    let value = codegen_expr(expr, ctx);
    for (key, _) in &cells {
        ctx.precomputed.borrow_mut().remove(key);
    }
    for (key, _) in &unpacked {
        ctx.unpacked.borrow_mut().remove(key);
    }
    code.push_str(&value);
    code.push_str("; })");
    Some(code)
}

/// `left followed by right`: a fresh sequence holding every element of the left, then every element of the
/// right, each operand read once in order.
fn c_seq_concat(expr: &Expr, left: &Expr, right: &Expr, ctx: &CContext) -> String {
    let ty = infer_expr_type(expr, ctx);
    let (Some(new_fn), Some(push_fn)) = (
        seq_fn(&ty, "new", ctx.interner),
        seq_fn(&ty, "push", ctx.interner),
    ) else {
        return c_refuse_expr(&format!("`followed by` over a {ty:?}"));
    };
    for side in [left, right] {
        let side_ty = infer_expr_type(side, ctx);
        if side_ty != ty && !matches!(side, Expr::List(items) if items.is_empty()) {
            return c_refuse_expr(&format!("`followed by` joining a {ty:?} and a {side_ty:?}"));
        }
    }
    let n = ctx.select_ctr.get();
    ctx.select_ctr.set(n + 1);
    let tn = c_type_str_resolved(&ty, ctx.interner);
    let (l, r) = (codegen_expr(left, ctx), codegen_expr(right, ctx));
    let l = if matches!(left, Expr::List(items) if items.is_empty()) {
        format!("{new_fn}()")
    } else {
        l
    };
    let r = if matches!(right, Expr::List(items) if items.is_empty()) {
        format!("{new_fn}()")
    } else {
        r
    };
    format!(
        "({{ {tn} __a{n} = {l}; {tn} __b{n} = {r}; {tn} __r{n} = {new_fn}(); \
         for (size_t __logos_k{n} = 0; __logos_k{n} < __a{n}.len; __logos_k{n}++) {push_fn}(&__r{n}, __a{n}.data[__logos_k{n}]); \
         for (size_t __logos_k{n} = 0; __logos_k{n} < __b{n}.len; __logos_k{n}++) {push_fn}(&__r{n}, __b{n}.data[__logos_k{n}]); __r{n}; }})"
    )
}

/// `value`, of type `ty`, as a binding the body changes in place holds it when `changed`: a text as a builder
/// of its own, a list, map, set or record as a copy of its own.
fn own_binding(ty: &CType, value: String, changed: bool, ctx: &CContext) -> String {
    match (changed, ty) {
        (false, _) => value,
        (true, CType::String) => format!("logos_text_builder({value}, 0)"),
        (true, other) => super::values::copy_value(other, &value, ctx),
    }
}

/// Whether the text `expr` computes may be a builder's own buffer, which the builder's next append can move:
/// held anywhere else, it is copied first.
pub(super) fn shares_builder(expr: &Expr, ctx: &CContext) -> bool {
    if super::values::is_fresh(expr) {
        return false;
    }
    let mut roots = std::collections::HashSet::new();
    super::values::source_roots(expr, &mut roots);
    roots.into_iter().any(|r| is_text_builder(r, ctx))
}

/// Whether `sym` is a builder: a text the body being lowered appends to in place.
pub(super) fn is_text_builder(sym: Symbol, ctx: &CContext) -> bool {
    builds_text(sym, ctx) && ctx.vars.get(&sym) == Some(&CType::String)
}

/// Whether a text bound to `sym` is built in place: the body changes it, and no `mutable` parameter lends it (a
/// lent text is one string the caller and the callee share, a plain one on both sides).
pub(super) fn builds_text(sym: Symbol, ctx: &CContext) -> bool {
    ctx.mutated.contains(&sym) && !ctx.plain_texts.contains(ctx.interner.resolve(sym))
}

/// Whether `expr` joins texts: `combined with`, or a `+` with a text on either side.
fn is_text_join(expr: &Expr, ctx: &CContext) -> bool {
    match expr {
        Expr::BinaryOp {
            op: BinaryOpKind::Concat,
            ..
        } => true,
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } => {
            infer_expr_type(left, ctx) == CType::String
                || infer_expr_type(right, ctx) == CType::String
        }
        _ => false,
    }
}

/// The parts a text joining puts together, left to right, the joinings among them flattened.
fn text_parts<'e, 'a>(expr: &'e Expr<'a>, ctx: &CContext, out: &mut Vec<&'e Expr<'a>>) {
    match expr {
        Expr::BinaryOp { left, right, .. } if is_text_join(expr, ctx) => {
            text_parts(left, ctx, out);
            text_parts(right, ctx, out);
        }
        _ => out.push(expr),
    }
}

/// A part of a joined text as C text, and whether that text is a fresh rendering the join may free: a text
/// is itself, any other value its rendering as `Show` gives it.
fn text_part(part: &Expr, ctx: &CContext) -> (String, bool) {
    if opt_var(part, ctx).is_none() && infer_expr_type(part, ctx) == CType::String {
        let code = codegen_expr(part, ctx);
        // A builder read into another text is copied by the join, so it is read in place.
        (code, false)
    } else {
        (stringify_num(part, ctx), true)
    }
}

/// A text joining as one allocation sized from all its parts, each part computed in order.
fn c_text_join(expr: &Expr, ctx: &CContext) -> String {
    let mut parts = Vec::new();
    text_parts(expr, ctx, &mut parts);
    let n = ctx.select_ctr.get();
    ctx.select_ctr.set(n + 1);
    let mut code = String::from("({ ");
    let mut names = Vec::new();
    let mut owned = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        let (text, fresh) = text_part(part, ctx);
        write!(code, "const char *__part{n}_{i} = {text}; ").unwrap();
        names.push(format!("__part{n}_{i}"));
        owned.push(if fresh { "1" } else { "0" });
    }
    write!(
        code,
        "logos_text_join({}, (const char *[]){{{}}}, (const unsigned char[]){{{}}}); }})",
        parts.len(),
        names.join(", "),
        owned.join(", ")
    )
    .unwrap();
    code
}

/// The parts `Set target to value` appends to `target` in place: `value` is a joining whose first part is
/// `target` itself and whose later parts never read it — so appending them one by one, in order, gives the
/// text the VM computes.
fn self_appended_parts<'e, 'a>(
    target: Symbol,
    value: &'e Expr<'a>,
    ctx: &CContext,
) -> Option<Vec<&'e Expr<'a>>> {
    if !is_text_join(value, ctx) {
        return None;
    }
    let mut parts = Vec::new();
    text_parts(value, ctx, &mut parts);
    let (first, rest) = parts.split_first()?;
    fn reads(e: &Expr, target: Symbol, through_calls: bool) -> bool {
        matches!(e, Expr::Identifier(s) if *s == target)
            || matches!(e, Expr::Closure { .. })
            || (through_calls && matches!(e, Expr::Call { .. } | Expr::CallExpr { .. }))
            || logicaffeine_language::ast_depth::expr_children(e)
                .into_iter()
                .any(|c| reads(c, target, through_calls))
    }
    // A function reads a program static, never a local.
    let through_calls = ctx.program_statics.contains(&target);
    let reads_target = |e: &Expr| reads(e, target, through_calls);
    (matches!(first, Expr::Identifier(s) if *s == target) && !rest.iter().any(|p| reads_target(p)))
        .then(|| rest.to_vec())
}

/// `value` as a builder's text: its text in a builder of its own, reserving the capacity a `with capacity`
/// asks for.
fn text_builder_of(value: &Expr, ctx: &CContext) -> String {
    match value {
        Expr::WithCapacity { value, capacity } => format!(
            "logos_text_builder({}, {})",
            codegen_expr(value, ctx),
            narrow_i64_sink(capacity, codegen_expr(capacity, ctx), ctx)
        ),
        other => format!("logos_text_builder({}, 0)", codegen_expr(other, ctx)),
    }
}

/// `expr` as the VM reads it for a condition, `not`, `and` and `or` (`RuntimeValue::is_truthy`): a Bool as itself,
/// a number when not zero, a Text, a sequence, a set or a map when not empty, nothing as false, and any other
/// value as true once computed.
pub(super) fn c_truthy(expr: &Expr, ctx: &CContext) -> String {
    if let Some((var, ty)) = opt_var(expr, ctx) {
        let element = opt_element(&ty);
        let value = super::types::opt_unpack(&element, &format!("({var}).value"), ctx.interner);
        return format!("(({var}).present && {})", truthy_of(&element, &value, ctx));
    }
    truthy_of(&infer_expr_type(expr, ctx), &codegen_expr(expr, ctx), ctx)
}

/// `value`, a C expression of type `ty`, read as true or false (see [`c_truthy`]).
fn truthy_of(ty: &CType, value: &str, ctx: &CContext) -> String {
    match ty {
        CType::Bool | CType::Int64 | CType::Float64 => format!("({value})"),
        CType::String => format!("(str_len({value}) != 0)"),
        CType::BigInt => format!("(lint_cmp({value}, lint_of(0)) != 0)"),
        CType::Rational | CType::Decimal => format!("(ldec_cmp({value}, ldec_from_i64(0)) != 0)"),
        CType::Tower => format!("ltower_truth({value})"),
        CType::SeqI64(_)
        | CType::SeqBool
        | CType::SeqStr
        | CType::SeqF64
        | CType::SeqStruct(_)
        | CType::SeqTuple(_)
        | CType::SeqSeq(_)
        | CType::SeqValue(_)
        | CType::SetI64(_)
        | CType::SetStr
        | CType::MapI64I64(_, _)
        | CType::MapStrI64(_)
        | CType::MapStrStr
        | CType::MapI64Str(_)
        | CType::MapStrValue(_)
        | CType::MapI64Value(_) => {
            let n = ctx.select_ctr.get();
            ctx.select_ctr.set(n + 1);
            let decl = c_param_decl(ty, &format!("__c{n}"), ctx.interner);
            format!("({{ {decl} = {value}; __c{n}.len != 0; }})")
        }
        CType::Opt(inner) => {
            let n = ctx.select_ctr.get();
            ctx.select_ctr.set(n + 1);
            let unpacked = super::types::opt_unpack(inner, &format!("__o{n}.value"), ctx.interner);
            format!(
                "({{ logos_opt __o{n} = {value}; __o{n}.present && {}; }})",
                truthy_of(inner, &unpacked, ctx)
            )
        }
        _ => format!("((void)({value}), 1)"),
    }
}

/// A value the VM holds of the kind `ty` lowers from, to ask the VM's own semantics what it does with one:
/// `None` for a kind no single value stands for.
pub(super) fn sample_of(ty: &CType, which: i64) -> Option<crate::interpreter::RuntimeValue> {
    use crate::interpreter::RuntimeValue as V;
    Some(match ty {
        CType::Int64 | CType::BigInt => V::Int(which),
        CType::Word(bits) => V::Word(logicaffeine_base::WordVal::from_u64(u32::from(*bits), which as u64)?),
        CType::Float64 => V::Float(which as f64 + 0.5),
        CType::Bool => V::Bool(which % 2 == 1),
        CType::String => V::Text(std::rc::Rc::new(format!("t{which}"))),
        CType::Date => V::Date(which as i32),
        CType::Moment => V::Moment(which),
        CType::Time => V::Time(which),
        CType::Duration => V::Duration(which),
        CType::Span => V::Span {
            months: which as i32,
            days: which as i32,
        },
        CType::Tuple(elems) => V::Tuple(std::rc::Rc::new(
            elems
                .iter()
                .map(|e| sample_of(e, which))
                .collect::<Option<Vec<_>>>()?,
        )),
        _ => return None,
    })
}

/// The failure the VM's arithmetic reports for `op` between values of the kinds `lt` and `rt`, when every such
/// pair fails alike: two different samples of each kind fail with the same words. `None` when the operator
/// combines them, or the kinds have no sample.
fn static_binary_failure(op: BinaryOpKind, lt: &CType, rt: &CType) -> Option<String> {
    if matches!(
        op,
        BinaryOpKind::Eq | BinaryOpKind::NotEq | BinaryOpKind::And | BinaryOpKind::Or
    ) {
        return None;
    }
    let attempt = |which: i64| {
        crate::semantics::arith::binary_op(op, sample_of(lt, which)?, sample_of(rt, which)?).err()
    };
    match (attempt(1), attempt(2)) {
        (Some(a), Some(b)) if a == b => Some(a),
        _ => None,
    }
}

/// A value of the C type `ty` for an expression whose evaluation has already failed: never read.
fn c_zero_of(ty: &CType, ctx: &CContext) -> String {
    match ty {
        CType::Void => "(void)0".to_string(),
        CType::Opt(_) => "((logos_opt){0, 0})".to_string(),
        other => format!("(({}){{0}})", c_type_str_resolved(other, ctx.interner)),
    }
}

/// `item index of` a tuple whose elements are `elems`: a position the program writes resolves at generation,
/// through the VM's own indexing (1-based, negative from the end, any other a failure in its words); one it
/// computes resolves when it runs, over elements of one kind.
fn tuple_item(tuple: &str, elems: &[CType], index: &Expr, ctx: &CContext) -> String {
    use crate::interpreter::RuntimeValue as V;
    if let Expr::Literal(Literal::Number(n)) = index {
        let positions = V::Tuple(std::rc::Rc::new(
            (0..elems.len() as i64).map(V::Int).collect(),
        ));
        return match crate::semantics::collections::index_get(&positions, &V::Int(*n)) {
            Ok(V::Int(k)) => format!("({tuple}).f{k}"),
            Ok(_) => unreachable!("a tuple of positions holds positions"),
            Err(failure) => {
                let ty = elems.first().cloned().unwrap_or(CType::Int64);
                format!(
                    "({{ (void)({tuple}); {} {}; }})",
                    ctx.fail_stmt(&c_string_literal(&failure)),
                    c_zero_of(&ty, ctx)
                )
            }
        };
    }
    let Some(first) = elems.first() else {
        return c_refuse_expr("an item of the empty tuple");
    };
    if elems.iter().any(|e| e != first) {
        return c_refuse_expr("an item, at a position computed when the program runs, of a tuple whose elements differ in kind");
    }
    let n = ctx.select_ctr.get();
    ctx.select_ctr.set(n + 1);
    let tn = tuple_c_name(elems, ctx.interner);
    let (fmt, _) = failcode_format(crate::semantics::failcode::FailCode::IndexOutOfBounds);
    let arms: String = (0..elems.len())
        .map(|k| format!("case {k}: __v{n} = __t{n}.f{k}; break; "))
        .collect();
    format!(
        "({{ {tn} __t{n} = {tuple}; int64_t __logos_i{n} = {}; int64_t __logos_k{n} = __logos_i{n} < 0 ? {len} + __logos_i{n} : __logos_i{n} - 1; \
         if (__logos_i{n} == 0 || __logos_k{n} < 0 || __logos_k{n} >= {len}) {} {}; switch (__logos_k{n}) {{ {arms}default: break; }} __v{n}; }})",
        narrow_i64_sink(index, codegen_expr(index, ctx), ctx),
        ctx.fail_stmt(&format!("logos_dyn_sprintf({fmt}, (long long)__logos_i{n})")),
        c_param_decl(first, &format!("__v{n}"), ctx.interner),
        len = elems.len(),
    )
}

/// The C `printf` format of the failure `code` reports, taken from the VM's own wording (`failcode::message`)
/// with the value it names as its one `%lld`; and whether the wording names the value at all.
pub(super) fn failcode_format(code: crate::semantics::failcode::FailCode) -> (String, bool) {
    const SENTINEL: i64 = 7_318_294_056_183_527;
    let text = crate::semantics::failcode::message(code.code(), SENTINEL);
    let names_value = text.contains(&SENTINEL.to_string());
    let escaped = escape_c_string(&text)
        .replace('%', "%%")
        .replace(&SENTINEL.to_string(), "%lld");
    (format!("\"{escaped}\""), names_value)
}

/// A pass of a loop whose pattern cannot take its element apart: it fails with `failure`, each name the pattern
/// binds declared so the body still types.
fn bind_pattern_failing(
    syms: &[Symbol],
    failure: &str,
    ctx: &mut CContext,
    output: &mut String,
    pad: &str,
) {
    writeln!(output, "{pad}{}", ctx.fail_stmt(&c_string_literal(failure))).unwrap();
    for sym in syms {
        emit_binding(*sym, CType::Int64, "0", ctx, output, pad);
    }
}

/// The name the VM gives the kind of a value of C type `ty` (`RuntimeValue::type_name`).
fn vm_type_name(ty: &CType, ctx: &CContext) -> String {
    use crate::interpreter::RuntimeValue as V;
    match ty {
        CType::SeqI64(_)
        | CType::SeqBool
        | CType::SeqStr
        | CType::SeqF64
        | CType::SeqStruct(_)
        | CType::SeqTuple(_)
        | CType::SeqSeq(_)
        | CType::SeqValue(_) => "List".to_string(),
        CType::SetI64(_) | CType::SetStr => "Set".to_string(),
        CType::MapI64I64(_, _)
        | CType::MapStrI64(_)
        | CType::MapStrStr
        | CType::MapI64Str(_)
        | CType::MapStrValue(_)
        | CType::MapI64Value(_) => "Map".to_string(),
        CType::Record(_) => "Tuple".to_string(),
        CType::Struct(sym) | CType::Enum(sym) => ctx.display_name(*sym),
        CType::Opt(_) => "Nothing".to_string(),
        other => sample_of(other, 1)
            .map_or_else(|| format!("{other:?}"), |v: V| v.type_name().to_string()),
    }
}

/// `rendered`, the C of `value`, as a collection, record or tuple stores it: a value some name still holds is
/// stored as a copy of its own, so changing the one leaves the other as it was.
/// A list literal built as the sequence `seq` its binding declares, each item made the element type
/// `elem` — an integer item of a `Seq of Rational` is the fraction it is — where the literal's first
/// item alone would type it otherwise.
fn literal_as(elems: &[&Expr], seq: &CType, elem: &CType, ctx: &CContext) -> String {
    let cty = c_type_str_resolved(seq, ctx.interner);
    let key = cty.strip_prefix("Seq_").unwrap_or(&cty).to_string();
    let mut s = format!("({{ {cty} __lit = seq_{key}_new(); ");
    for e in elems {
        let from = infer_expr_type(e, ctx);
        let value = match elem {
            held if super::absent::holds_nothing(held) => value_as(e, held, ctx),
            CType::Rational | CType::Decimal if !matches!(from, CType::Rational | CType::Decimal) => {
                codegen_expr_as_rational(e, ctx)
            }
            CType::BigInt if from != CType::BigInt => format!("lint_of((int64_t)({}))", codegen_expr(e, ctx)),
            _ => stored(e, codegen_expr(e, ctx), ctx),
        };
        s.push_str(&format!("seq_{key}_push(&__lit, {value}); "));
    }
    s.push_str("__lit; })");
    s
}

fn stored(value: &Expr, rendered: String, ctx: &CContext) -> String {
    let ty = infer_expr_type(value, ctx);
    if ty == CType::String && shares_builder(value, ctx) {
        return format!("lc_gc_strdup({rendered})");
    }
    if super::values::is_fresh(value)
        || !super::values::holds_buffer(&ty, ctx.registry, ctx.interner)
    {
        rendered
    } else {
        super::values::copy_value(&ty, &rendered, ctx)
    }
}

/// The C type of `collection` as a value: a variable's own, anything else inferred.
fn collection_type(collection: &Expr, ctx: &CContext) -> CType {
    match collection {
        Expr::Identifier(sym) => match ctx.vars.get(sym) {
            Some(CType::Opt(held)) => (**held).clone(),
            Some(ty) => ty.clone(),
            None => infer_expr_type(collection, ctx),
        },
        _ => infer_expr_type(collection, ctx),
    }
}

/// `apply(address)` over the collection `collection` of C type `ty`: a variable by its address (a value-or-nothing
/// one by the collection it holds, failing first in `on_nothing`'s words when it holds none), any other collection
/// bound once to a temporary of a statement expression first, since `&rvalue` is illegal C.
fn with_collection(
    collection: &Expr,
    ty: &CType,
    on_nothing: impl FnOnce() -> Result<(), String>,
    ctx: &CContext,
    apply: impl FnOnce(&str) -> String,
) -> String {
    match collection {
        _ if operand_place(collection, ctx) => {
            let operand = collection_operand(collection, on_nothing, ctx);
            operand.read(apply(&format!("&{}", operand.place)))
        }
        _ => {
            let n = ctx.select_ctr.get();
            ctx.select_ctr.set(n + 1);
            let tmp = format!("__coll{n}");
            let decl = c_param_decl(ty, &tmp, ctx.interner);
            format!(
                "({{ {decl} = {}; {}; }})",
                codegen_expr(collection, ctx),
                apply(&format!("&{tmp}"))
            )
        }
    }
}

/// `collection contains value`, as the VM answers it: a Text holds a Text as a substring, a sequence or a
/// set holds an equal element, and a map holds a key. The collection is read before the value.
fn c_contains(collection: &Expr, value: &Expr, ctx: &CContext) -> String {
    let on_nothing = || {
        use crate::interpreter::RuntimeValue;
        crate::semantics::collections::contains(&RuntimeValue::Nothing, &RuntimeValue::Nothing).map(|_| ())
    };
    let ty = collection_type(collection, ctx);
    // A rational is found by its value, an integer among rationals by the rational it equals.
    if matches!(&ty, CType::SeqValue(e) if matches!(**e, CType::Rational | CType::Decimal))
        || (matches!(ty, CType::SeqI64(Ints::Int))
            && matches!(infer_expr_type(value, ctx), CType::Rational | CType::Decimal))
    {
        let n = ctx.select_ctr.get();
        ctx.select_ctr.set(n + 1);
        let item = if matches!(ty, CType::SeqI64(_)) {
            format!("ldec_from_i64((*__seq{n}).data[__walk{n}])")
        } else {
            format!("(*__seq{n}).data[__walk{n}]")
        };
        let needle = codegen_expr_as_rational(value, ctx);
        let decl = c_param_decl(&ty, &format!("*__seq{n}"), ctx.interner);
        return with_collection(collection, &ty, on_nothing, ctx, |at| {
            format!(
                "({{ {decl} = {at}; LDec __v{n} = {needle}; bool __f{n} = false; for (size_t __walk{n} = 0; __walk{n} < (*__seq{n}).len && !__f{n}; __walk{n}++) __f{n} = ldec_cmp({item}, __v{n}) == 0; __f{n}; }})"
            )
        });
    }
    // A list whose items may be nothing holds the value when an item equals it as the VM compares them.
    if let CType::SeqValue(item) = &ty {
        if matches!(**item, CType::Opt(_)) {
            let n = ctx.select_ctr.get();
            ctx.select_ctr.set(n + 1);
            let vt = infer_expr_type(value, ctx);
            let (needle, cell) = match vt {
                CType::Void => (String::new(), String::new()),
                _ => (
                    format!("{} = {}; ", c_param_decl(&vt, &format!("__v{n}"), ctx.interner), codegen_expr(value, ctx)),
                    format!("__v{n}"),
                ),
            };
            let Some(same) = super::equality::value_equal(item, &format!("(*__seq{n}).data[__walk{n}]"), &vt, &cell, ctx)
            else {
                return c_refuse_expr(&format!("`contains` of a {vt:?} among items that may be nothing"));
            };
            return with_collection(collection, &ty, on_nothing, ctx, |at| {
                format!(
                    "({{ Seq_logos_opt *__seq{n} = {at}; {needle}bool __f{n} = false; \
                     for (size_t __walk{n} = 0; __walk{n} < (*__seq{n}).len && !__f{n}; __walk{n}++) __f{n} = {same}; __f{n}; }})"
                )
            });
        }
    }
    let membership = match &ty {
        CType::String => {
            let n = ctx.select_ctr.get();
            ctx.select_ctr.set(n + 1);
            let found = |hay: &str| {
                format!(
                    "({{ const char *__hay{n} = {hay}; strstr(__hay{n}, {}) != NULL; }})",
                    codegen_expr(value, ctx)
                )
            };
            return match collection {
                _ if operand_place(collection, ctx) => {
                    let operand = collection_operand(collection, on_nothing, ctx);
                    operand.read(found(&operand.place))
                }
                _ => found(&codegen_expr(collection, ctx)),
            };
        }
        CType::SeqI64(_) => "seq_i64_contains".to_string(),
        CType::SeqBool => "seq_bool_contains".to_string(),
        CType::SeqStr => "seq_str_contains".to_string(),
        CType::SeqF64 => "seq_f64_contains".to_string(),
        CType::MapI64I64(_, _) => "map_i64_i64_contains".to_string(),
        CType::MapStrI64(_) => "map_str_i64_contains".to_string(),
        CType::MapStrStr => "map_str_str_contains".to_string(),
        CType::MapI64Str(_) => "map_i64_str_contains".to_string(),
        CType::MapStrValue(inner) => format!(
            "map_str_{}_contains",
            c_type_str_resolved(inner, ctx.interner)
        ),
        CType::MapI64Value(inner) => format!(
            "map_i64_{}_contains",
            c_type_str_resolved(inner, ctx.interner)
        ),
        CType::SetI64(_) => "set_i64_contains".to_string(),
        CType::SetStr => "set_str_contains".to_string(),
        other => {
            return c_refuse_expr(&format!(
                "`contains` over a {other:?}, whose elements it cannot compare yet"
            ))
        }
    };
    let integer_key = matches!(
        ty,
        CType::SeqI64(_) | CType::MapI64I64(_, _) | CType::MapI64Str(_) | CType::SetI64(_)
    );
    with_collection(collection, &ty, on_nothing, ctx, |at| {
        let v = codegen_expr(value, ctx);
        let v = if integer_key {
            narrow_i64_sink(value, v, ctx)
        } else {
            v
        };
        format!("{membership}({at}, {v})")
    })
}

/// `items start through end of collection`, as the VM slices: the elements from position `start` to `end`
/// inclusive when `1 <= start <= end <= length`, and an empty sequence for any other pair — `start` 0 counting
/// as 1, as the VM's unsigned arithmetic takes it. Read in order: the collection, then `start`, then `end`.
fn c_slice(collection: &Expr, start: &Expr, end: &Expr, ctx: &CContext) -> String {
    let ty = collection_type(collection, ctx);
    let (Some(new_fn), Some(push_fn)) = (
        seq_fn(&ty, "new", ctx.interner),
        seq_fn(&ty, "push", ctx.interner),
    ) else {
        return c_refuse_expr(&format!("a slice of a {ty:?}, which is not a sequence"));
    };
    let n = ctx.select_ctr.get();
    ctx.select_ctr.set(n + 1);
    let tn = c_type_str_resolved(&ty, ctx.interner);
    let (from, to) = (
        integer_at_place(start, super::words::IntegerPlace::Slice, ctx),
        integer_at_place(end, super::words::IntegerPlace::Slice, ctx),
    );
    format!(
        "({{ {tn} __s{n} = {coll}; uint64_t __a{n} = (uint64_t)(int64_t)({from}); uint64_t __b{n} = (uint64_t)(int64_t)({to}); \
         uint64_t __lo{n} = __a{n} == 0 ? 0 : __a{n} - 1; {tn} __r{n} = {new_fn}(); \
         if (__lo{n} < __b{n} && __b{n} <= (uint64_t)__s{n}.len) for (uint64_t __logos_k{n} = __lo{n}; __logos_k{n} < __b{n}; __logos_k{n}++) \
         {push_fn}(&__r{n}, __s{n}.data[__logos_k{n}]); __r{n}; }})",
        coll = match collection {
            // A value-or-nothing variable is the sequence it holds, failing in the VM's words when it holds none.
            _ if operand_place(collection, ctx) => {
                let operand = collection_operand(
                    collection,
                    || {
                        use crate::interpreter::RuntimeValue;
                        let one = RuntimeValue::Int(1);
                        crate::semantics::collections::slice(&RuntimeValue::Nothing, &one, &one).map(|_| ())
                    },
                    ctx,
                );
                operand.read(operand.place.clone())
            }
            _ => codegen_expr(collection, ctx),
        },
    )
}

/// What a refusal of a list literal whose items differ in kind names.
pub(super) const MIXED_LIST: &str = "a list whose items differ in kind";

/// Whether the items of a list literal differ in kind beyond the forms of one integer: the VM keeps each item the
/// value it is, and a C sequence holds one element type, taken from the first item.
fn mixed_kinds(elems: &[&Expr], ctx: &CContext) -> bool {
    let integer = |t: &CType| matches!(t, CType::Int64 | CType::BigInt);
    if super::types::list_item_type(elems, ctx) == CType::Rational {
        return false;
    }
    let mut kinds = elems.iter().map(|e| infer_expr_type(e, ctx));
    let Some(first) = kinds.next() else {
        return false;
    };
    kinds.any(|k| k != first && !(integer(&k) && integer(&first)))
}

/// The C text that refuses `what` at generation: an `#error` naming the construct.
pub(super) fn c_refuse_expr(what: &str) -> String {
    format!("\n#error \"the C backend does not lower {what}\"\n")
}

/// A closure `(params) -> body` as a function value: its body lifted to a top-level `static` function that takes
/// the closure's environment first, and the environment holding, as they are when the closure is made, the
/// bindings its body reads from around it — the VM's snapshot captures.
fn codegen_closure(
    params: &[(Symbol, &TypeExpr)],
    body: &ClosureBody,
    return_type: Option<&TypeExpr>,
    ctx: &CContext,
) -> String {
    let id = ctx.lambda_ctr.get();
    ctx.lambda_ctr.set(id + 1);
    let name = format!("__logos_lambda_{}", id);
    let captured: Vec<(Symbol, CType)> =
        crate::interpreter::Interpreter::free_vars_in_closure(params, body)
            .into_iter()
            .filter_map(|sym| ctx.vars.get(&sym).map(|ty| (sym, ty.clone())))
            .collect();

    let mut child = CContext::new(ctx.interner, ctx.registry);
    child.funcs = ctx.funcs.clone();
    child.exact_hofs = ctx.exact_hofs.clone();
    child.exact = ctx.exact.clone();
    child.fn_params = ctx.fn_params.clone();
    child.fn_inout = ctx.fn_inout.clone();
    child.plain_texts = ctx.plain_texts.clone();
    child.native_funcs = ctx.native_funcs.clone();
    child.uses_concurrency = ctx.uses_concurrency;
    child.nothing_names = ctx.nothing_names.clone();
    child.absence = ctx.absence.clone();
    *child.copy_fns.borrow_mut() = ctx.copy_fns.borrow().clone();
    *child.equal_fns.borrow_mut() = ctx.equal_fns.borrow().clone();
    child.program_statics = ctx.program_statics.clone();
    child.static_types = ctx.static_types.clone();
    child.counter_fields = ctx.counter_fields.clone();
    child.display_names = ctx.display_names.clone();
    child.policies = ctx.policies;
    child.changed_statics = ctx.changed_statics.clone();
    child.mutating = ctx.mutating.clone();
    child.call_shapes = ctx.call_shapes.clone();
    child.int_oracle = ctx.int_oracle.clone();
    child.quiet_fns = ctx.quiet_fns.clone();
    child.mutated = match body {
        ClosureBody::Block(stmts) => super::values::mutated_roots(stmts, &ctx.fn_inout),
        ClosureBody::Expression(_) => std::collections::HashSet::new(),
    };
    child.lambda_ctr.set(ctx.lambda_ctr.get());
    for g in &ctx.program_statics {
        if let Some(ty) = ctx.static_types.get(g) {
            child.vars.insert(*g, ty.clone());
        }
    }
    child.enter_scope();
    let mut decls = Vec::new();
    for (pn, pt) in params {
        // A function value's integers are exact (`LInt`), so an overflowing body promotes, as on the VM.
        let mut cty = resolve_type_expr_with_registry(pt, ctx.interner, Some(ctx.registry));
        super::exactify_int_ctype(&mut cty);
        let cname = child.bind_in_place(*pn, cty.clone());
        decls.push(c_param_decl(&cty, &cname, ctx.interner));
    }
    let mut prologue = String::new();
    for (pn, _) in params {
        if child.mutated.contains(pn) && child.vars.get(pn) == Some(&CType::String) {
            let cname = child.resolve(*pn);
            writeln!(prologue, "    {cname} = logos_text_builder({cname}, 0);").unwrap();
        }
    }
    if !captured.is_empty() {
        writeln!(
            prologue,
            "    struct __logos_env_{id} *__e = (struct __logos_env_{id} *)__env;"
        )
        .unwrap();
        for (i, (sym, ty)) in captured.iter().enumerate() {
            let arrived = if *ty == CType::String && child.mutated.contains(sym) {
                format!("logos_text_builder(__e->c{i}, 0)")
            } else {
                format!("__e->c{i}")
            };
            let local = child.declare(*sym, ty.clone());
            writeln!(prologue, "    {} = {arrived};", child.decl_head(ty, &local)).unwrap();
        }
    }
    let mut ret_cty = match return_type {
        Some(rt) => resolve_type_expr_with_registry(rt, ctx.interner, Some(ctx.registry)),
        None => match body {
            ClosureBody::Expression(e) => infer_expr_type(e, &child),
            ClosureBody::Block(_) => CType::Int64,
        },
    };
    super::exactify_int_ctype(&mut ret_cty);
    child.current_fn_returns = ret_cty.clone();
    let body_c = match body {
        ClosureBody::Expression(e) => format!(
            "    return {};\n",
            as_returned(e, codegen_expr(e, &child), &child)
        ),
        ClosureBody::Block(stmts) => {
            let mut b = String::new();
            for s in *stmts {
                codegen_stmt(s, &mut child, &mut b, 1);
            }
            b
        }
    };
    let mut def = String::new();
    if !captured.is_empty() {
        let fields: String = captured
            .iter()
            .enumerate()
            .map(|(i, (_, ty))| format!(" {};", c_param_decl(ty, &format!("c{i}"), ctx.interner)))
            .collect();
        writeln!(def, "struct __logos_env_{id} {{{fields} }};").unwrap();
    }
    let ps: String = decls.iter().map(|d| format!(", {d}")).collect();
    write!(
        def,
        "static {} {}(void *__env{ps}) {{\n{prologue}{body_c}}}\n",
        c_type_str_resolved(&ret_cty, ctx.interner),
        name
    )
    .unwrap();
    // Nested lambdas from the child first, then this one, into the parent's collection; the functions the child
    // generated are the parent's now, never generated twice.
    {
        let mut pl = ctx.lifted.borrow_mut();
        pl.extend(child.lifted.borrow_mut().drain(..));
        pl.push(def);
    }
    ctx.copy_fns.borrow_mut().extend(child.copy_fns.borrow_mut().drain());
    ctx.equal_fns.borrow_mut().extend(child.equal_fns.borrow_mut().drain());
    ctx.lambda_ctr.set(child.lambda_ctr.get());
    if captured.is_empty() {
        return format!("((LClosure){{(void *){name}, NULL}})");
    }
    // A capture either side may change in place is held as a copy of its own.
    let stores: String = captured
        .iter()
        .enumerate()
        .map(|(i, (sym, ty))| {
            let value = ctx.resolve(*sym);
            let value = if *ty == CType::String && is_text_builder(*sym, ctx) {
                format!("lc_gc_strdup({value})")
            } else if ctx.mutated.contains(sym) || child.mutated.contains(sym) {
                super::values::copy_value(ty, &value, ctx)
            } else {
                value
            };
            format!("__env{id}->c{i} = {value}; ")
        })
        .collect();
    format!(
        "({{ struct __logos_env_{id} *__env{id} = (struct __logos_env_{id} *)lc_gc_alloc(sizeof(struct __logos_env_{id})); {stores}(LClosure){{(void *){name}, __env{id}}}; }})"
    )
}

/// A call of the function value `callee` (a C expression of type `fn(params) -> ret`) with `args`: its code,
/// given its environment first. A callee that is not a variable is computed once, before the arguments.
fn closure_call(
    callee: &str,
    is_variable: bool,
    ret: &CType,
    params: &[CType],
    args: &[String],
    ctx: &CContext,
) -> String {
    let code = super::types::closure_code_type(ret, params, ctx.interner);
    let rest: String = args.iter().map(|a| format!(", {a}")).collect();
    if is_variable {
        return format!("((({code})({callee}).call)(({callee}).env{rest}))");
    }
    let n = ctx.select_ctr.get();
    ctx.select_ctr.set(n + 1);
    format!("({{ LClosure __fn{n} = {callee}; (({code})__fn{n}.call)(__fn{n}.env{rest}); }})")
}

pub(super) fn codegen_literal(lit: &Literal, ctx: &CContext) -> String {
    match lit {
        Literal::Number(n) => format!("{}LL", n),
        // Rust's `{}` renders a non-finite f64 as `NaN`/`inf` — invalid C tokens. Emit the
        // `<math.h>` macros so a folded or literal NaN/infinity constant compiles (the prelude
        // always includes math.h). Finite values format identically to a valid C double literal.
        Literal::Float(f) if f.is_nan() => "NAN".to_string(),
        Literal::Float(f) if f.is_infinite() => {
            if *f < 0.0 {
                "(-INFINITY)".to_string()
            } else {
                "INFINITY".to_string()
            }
        }
        // A finite float literal MUST keep its decimal point: Rust's `{}` renders an integral value
        // like `2.0` as `2`, which C then treats as an INT — so `2.0 * x / n` (x, n ints) becomes
        // integer division and truncates (broke mandelbrot/spectral_norm/nbody). Append `.0` when the
        // shortest-round-trip form has no `.`/exponent, so the C literal stays a `double`.
        Literal::Float(f) => {
            let s = format!("{}", f);
            if s.contains('.') || s.contains('e') || s.contains('E') {
                s
            } else {
                format!("{s}.0")
            }
        }
        Literal::Boolean(b) => {
            if *b {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        Literal::Text(sym) => {
            let raw = ctx.interner.resolve(*sym);
            let escaped: String = raw
                .chars()
                .map(|c| match c {
                    '\n' => "\\n".to_string(),
                    '\r' => "\\r".to_string(),
                    '\t' => "\\t".to_string(),
                    '\\' => "\\\\".to_string(),
                    '"' => "\\\"".to_string(),
                    other => other.to_string(),
                })
                .collect();
            format!("\"{}\"", escaped)
        }
        Literal::Nothing => "0".to_string(),
        Literal::Date(days) => format!("{}LL", days),
        Literal::Moment(nanos) | Literal::Time(nanos) | Literal::Duration(nanos) => {
            format!("{}LL", nanos)
        }
        Literal::Span { months, days } => format!("logos_span({}, {})", months, days),
        Literal::Char(c) => format!(
            "\n#error \"a Char value ({:?}) has no C representation yet\"\n",
            c
        ),
    }
}

/// `value`, a C expression of type `ty`, as a fresh `char*` — the one text every C path renders a
/// value with (`Show`, an interpolation hole, `format()`, a concatenation operand), so no kind can
/// print one way in one place and another way elsewhere. The value is bound once, since a renderer
/// may read it more than once or take its address.
pub(super) fn c_text_of(ty: &CType, value: &str, ctx: &CContext) -> String {
    let render = crate::codegen_c::types::c_field_to_string_expr(ty, "__t", ctx.interner);
    match ty {
        CType::Void => render,
        _ => {
            let decl = c_param_decl(ty, "__t", ctx.interner);
            format!("({{ {decl} = {value}; {render}; }})")
        }
    }
}

/// The C expression for a `Select` TIMEOUT arm's tick count, matching the VM's `eval_select_timeout_ticks`
/// byte-for-byte: an `Int` (`After 5:`) is that many ticks, a `Duration` (`After 5 seconds` = 5·10⁹ nanos)
/// is its nanosecond count as ticks, a calendar `Span` collapses to `(months·30 + days)·86400`, each
/// clamped at zero. A non-literal expression (a variable holding an Int/Duration) already yields its raw
/// int64 tick value, so `codegen_expr` serves it directly.
pub(super) fn select_timeout_ticks_c(expr: &Expr, ctx: &CContext) -> String {
    match expr {
        Expr::Literal(Literal::Number(n)) => format!("{}LL", (*n).max(0)),
        Expr::Literal(Literal::Duration(d)) => format!("{}LL", (*d).max(0)),
        Expr::Literal(Literal::Span { months, days }) => {
            let ticks = (((*months as i64) * 30 + *days as i64) * 86_400).max(0);
            format!("{}LL", ticks)
        }
        _ => codegen_expr(expr, ctx),
    }
}

pub(super) fn c_format_spec_for_hole(
    expr: &Expr,
    spec: Option<&str>,
    ctx: &CContext,
) -> (String, String) {
    if let (Some((var, ty)), None) = (opt_var(expr, ctx), spec) {
        return ("%s".to_string(), c_text_of(&ty, &var, ctx));
    }
    // `nothing` in a hole is the word it shows as.
    if let (Expr::Literal(Literal::Nothing) | Expr::OptionNone, None) = (expr, spec) {
        return ("%s".to_string(), "\"nothing\"".to_string());
    }
    let ty = infer_expr_type(expr, ctx);
    let val_str = codegen_expr(expr, ctx);

    if let Some(spec) = spec {
        // The interpreter's `semantics::format::apply_format_spec`, kind by kind: `$` gives an Int or
        // a Real two decimals and a BigInt its exact digits with `.00`; `.N` rounds an Int, a Real or
        // a BigInt to N places; any other kind keeps its own text (a decimal's digits, a money's
        // currency), after the `$`.
        if spec == "$" {
            return match ty {
                CType::Int64 | CType::Float64 => {
                    ("$%.2f".to_string(), format!("(double)({})", val_str))
                }
                CType::BigInt => ("$%s.00".to_string(), format!("lint_to_string({})", val_str)),
                other => ("$%s".to_string(), c_text_of(&other, &val_str, ctx)),
            };
        }
        if spec.starts_with('.') {
            if let Ok(prec) = spec[1..].parse::<usize>() {
                return match ty {
                    CType::Int64 | CType::Float64 => {
                        (format!("%.{}f", prec), format!("(double)({})", val_str))
                    }
                    CType::BigInt => (format!("%.{}f", prec), format!("lint_to_f64({})", val_str)),
                    other => ("%s".to_string(), c_text_of(&other, &val_str, ctx)),
                };
            }
        }
        if spec.starts_with('>') || spec.starts_with('<') || spec.starts_with('^') {
            let align = &spec[..1];
            if let Ok(width) = spec[1..].parse::<usize>() {
                if align == "^" {
                    match ty {
                        CType::Int64 => {
                            return (
                                "%s".to_string(),
                                format!("logos_center_i64({}, {})", val_str, width),
                            )
                        }
                        CType::Word(_) => {
                            return (
                                "%s".to_string(),
                                format!(
                                    "logos_center_str({}, {})",
                                    c_text_of(&ty, &val_str, ctx),
                                    width
                                ),
                            )
                        }
                        CType::String => {
                            return (
                                "%s".to_string(),
                                format!("logos_center_str({}, {})", val_str, width),
                            )
                        }
                        _ => {
                            return (
                                "%s".to_string(),
                                format!(
                                    "logos_center_str({}, {})",
                                    c_text_of(&ty, &val_str, ctx),
                                    width
                                ),
                            )
                        }
                    }
                }
                let c_flag = match align {
                    ">" => "",
                    "<" => "-",
                    _ => unreachable!(),
                };
                match ty {
                    CType::String => return (format!("%{}{}s", c_flag, width), val_str),
                    CType::Int64 => return (format!("%{}{}\" PRId64 \"", c_flag, width), val_str),
                    CType::Word(_) => {
                        return (
                            format!("%{}{}\" PRIu64 \"", c_flag, width),
                            format!("(uint64_t)({val_str})"),
                        )
                    }
                    _ => {
                        return (
                            format!("%{}{}s", c_flag, width),
                            c_text_of(&ty, &val_str, ctx),
                        )
                    }
                }
            }
        }
        // A BARE width `{n:5}` (no alignment prefix) pads to that width, RIGHT-aligned (the LOGOS default,
        // like `>`) — e.g. `[{n:5}]` with n=42 → `[   42]`.
        if let Ok(width) = spec.parse::<usize>() {
            match ty {
                CType::String => return (format!("%{}s", width), val_str),
                CType::Int64 => return (format!("%{}\" PRId64 \"", width), val_str),
                CType::Word(_) => {
                    return (
                        format!("%{}\" PRIu64 \"", width),
                        format!("(uint64_t)({val_str})"),
                    )
                }
                _ => return (format!("%{}s", width), c_text_of(&ty, &val_str, ctx)),
            }
        }
    }

    match ty {
        CType::Int64 => ("%\" PRId64 \"".to_string(), val_str),
        CType::Word(_) => (
            "%\" PRIu64 \"".to_string(),
            format!("(uint64_t)({val_str})"),
        ),
        CType::Float64 => ("%s".to_string(), format!("logos_fmt_f64({val_str})")),
        CType::Bool => (
            "%s".to_string(),
            format!("{} ? \"true\" : \"false\"", val_str),
        ),
        CType::String => ("%s".to_string(), val_str),
        // Value types interpolate via their `*_to_string` renderer (`"Balance: {m}"` → `10.00 USD`),
        // not the `%lld` that would print the struct as a garbage integer. `LDec` takes a pointer, so
        // bind it first (the hole may be an rvalue). The returned buffer leaks — acceptable in the
        // short-lived generated program, matching the rest of the value-type by-value convention.
        CType::Money => ("%s".to_string(), format!("money_to_string({})", val_str)),
        CType::Quantity => ("%s".to_string(), format!("quant_to_string({})", val_str)),
        CType::Modular => ("%s".to_string(), format!("mod_to_string({})", val_str)),
        CType::Uuid => ("%s".to_string(), format!("uuid_to_string({})", val_str)),
        CType::Complex => ("%s".to_string(), format!("cplx_to_string({})", val_str)),
        CType::BigInt => ("%s".to_string(), format!("lint_to_string({})", val_str)),
        CType::Rational | CType::Decimal => (
            "%s".to_string(),
            format!("({{ LDec __d = {}; ldec_to_string(&__d); }})", val_str),
        ),
        // Collections interpolate via their `*_to_string` (insertion-ordered for Map/Set, `[…]` for Seq),
        // byte-identical to `Show`. The renderer takes a pointer, so bind the (possibly rvalue) hole to a
        // temp first via a GCC statement-expression.
        CType::SeqI64(_)
        | CType::SeqBool
        | CType::SeqStr
        | CType::SeqF64
        | CType::SeqValue(_)
        | CType::SeqSeq(_)
        | CType::Tuple(_)
        | CType::Record(_)
        | CType::SeqTuple(_)
        | CType::MapI64I64(_, _)
        | CType::MapStrI64(_)
        | CType::MapStrStr
        | CType::MapI64Str(_)
        | CType::MapStrValue(_)
        | CType::MapI64Value(_)
        | CType::SetI64(_)
        | CType::SetStr => {
            let tn = c_type_str_resolved(&ty, ctx.interner);
            let render = crate::codegen_c::types::c_field_to_string_expr(&ty, "__t", ctx.interner);
            (
                "%s".to_string(),
                format!("({{ {tn} __t = {}; {render}; }})", val_str),
            )
        }
        CType::SeqStruct(sym)
            if crate::codegen_c::types::c_user_type_showable(sym, ctx.registry, ctx.interner) =>
        {
            let tn = c_type_str_resolved(&ty, ctx.interner);
            let render = crate::codegen_c::types::c_field_to_string_expr(&ty, "__t", ctx.interner);
            (
                "%s".to_string(),
                format!("({{ {tn} __t = {}; {render}; }})", val_str),
            )
        }
        CType::Struct(sym)
            if crate::codegen_c::types::c_struct_showable(sym, ctx.registry, ctx.interner) =>
        {
            let ts = crate::codegen_c::types::struct_to_string_fn(sym, ctx.interner);
            ("%s".to_string(), format!("{}({})", ts, val_str))
        }
        CType::Enum(sym)
            if crate::codegen_c::types::c_enum_showable(sym, ctx.registry, ctx.interner) =>
        {
            let ts = crate::codegen_c::types::struct_to_string_fn(sym, ctx.interner);
            ("%s".to_string(), format!("{}({})", ts, val_str))
        }
        other => ("%s".to_string(), c_text_of(&other, &val_str, ctx)),
    }
}

pub(super) fn codegen_interpolated_string_c(
    parts: &[crate::ast::stmt::StringPart],
    ctx: &CContext,
) -> String {
    let mut fmt = String::new();
    let mut args = Vec::new();

    for part in parts {
        match part {
            crate::ast::stmt::StringPart::Literal(sym) => {
                let text = ctx.interner.resolve(*sym);
                for c in text.chars() {
                    match c {
                        '\n' => fmt.push_str("\\n"),
                        '\r' => fmt.push_str("\\r"),
                        '\t' => fmt.push_str("\\t"),
                        '\\' => fmt.push_str("\\\\"),
                        '"' => fmt.push_str("\\\""),
                        '%' => fmt.push_str("%%"),
                        other => fmt.push(other),
                    }
                }
            }
            crate::ast::stmt::StringPart::Expr {
                value,
                format_spec,
                debug,
            } => {
                if *debug {
                    let name = match value {
                        Expr::Identifier(sym) => ctx.interner.resolve(*sym).to_string(),
                        _ => "?".to_string(),
                    };
                    fmt.push_str(&name);
                    fmt.push('=');
                }
                let spec_str = format_spec.map(|s| ctx.interner.resolve(s).to_string());
                let (hole_fmt, hole_arg) = c_format_spec_for_hole(value, spec_str.as_deref(), ctx);
                fmt.push_str(&hole_fmt);
                args.push(hole_arg);
            }
        }
    }

    if args.is_empty() {
        format!("lc_gc_strdup(\"{}\")", fmt)
    } else {
        format!("logos_dyn_sprintf(\"{}\", {})", fmt, args.join(", "))
    }
}

pub(super) fn codegen_interpolated_show_c(
    parts: &[crate::ast::stmt::StringPart],
    ctx: &CContext,
    pad: &str,
    output: &mut String,
) {
    let mut fmt = String::new();
    let mut args = Vec::new();

    for part in parts {
        match part {
            crate::ast::stmt::StringPart::Literal(sym) => {
                let text = ctx.interner.resolve(*sym);
                for c in text.chars() {
                    match c {
                        '\n' => fmt.push_str("\\n"),
                        '\r' => fmt.push_str("\\r"),
                        '\t' => fmt.push_str("\\t"),
                        '\\' => fmt.push_str("\\\\"),
                        '"' => fmt.push_str("\\\""),
                        '%' => fmt.push_str("%%"),
                        other => fmt.push(other),
                    }
                }
            }
            crate::ast::stmt::StringPart::Expr {
                value,
                format_spec,
                debug,
            } => {
                if *debug {
                    let name = match value {
                        Expr::Identifier(sym) => ctx.interner.resolve(*sym).to_string(),
                        _ => "?".to_string(),
                    };
                    fmt.push_str(&name);
                    fmt.push('=');
                }
                let spec_str = format_spec.map(|s| ctx.interner.resolve(s).to_string());
                let (hole_fmt, hole_arg) = c_format_spec_for_hole(value, spec_str.as_deref(), ctx);
                fmt.push_str(&hole_fmt);
                args.push(hole_arg);
            }
        }
    }

    if args.is_empty() {
        writeln!(output, "{}puts(\"{}\");", pad, fmt).unwrap();
    } else {
        writeln!(
            output,
            "{}printf(\"{}\\n\", {});",
            pad,
            fmt,
            args.join(", ")
        )
        .unwrap();
    }
}

// =============================================================================
// Statement Codegen
// =============================================================================

/// A loop-local sequence which can reuse the storage of the sequence it
/// replaces at the end of the loop.  The Rust emitter recognizes the same
/// ping-pong shape and lowers it to `Vec::clear` + `mem::swap`; keeping the
/// recognition here prevents the C tier from silently paying one allocation
/// per outer iteration.
struct CBufferReuse {
    inner: Symbol,
    outer: Symbol,
    inner_let: usize,
    move_set: usize,
    ty: CType,
}

fn unit_increment(stmt: &Stmt) -> bool {
    let Stmt::Set { target, value } = stmt else {
        return false;
    };
    matches!(
        value,
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } if (matches!(left, Expr::Identifier(sym) if sym == target)
            && matches!(right, Expr::Literal(Literal::Number(1))))
            || (matches!(left, Expr::Literal(Literal::Number(1)))
                && matches!(right, Expr::Identifier(sym) if sym == target))
    )
}

/// Recognize:
///
/// ```text
/// While ...:
///     Let mutable curr be a new Seq of T.
///     ... fully refill curr ...
///     Set prev to curr.
///     [increment the loop counter]
/// ```
///
/// The final transfer already has move semantics in the C ownership analysis.
/// Exchanging the two sequence structs is therefore equivalent, while retaining
/// `prev`'s old allocation for `curr` to refill on the next iteration.
fn detect_c_buffer_reuse(body: &[Stmt], ctx: &CContext) -> Option<CBufferReuse> {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::Peephole) || body.len() < 2
    {
        return None;
    }

    let mut inner = None;
    for (idx, stmt) in body.iter().take(2).enumerate() {
        let Stmt::Let {
            var,
            value,
            ty,
            mutable: true,
        } = stmt
        else {
            continue;
        };
        let fresh = match value {
            Expr::New {
                type_name,
                init_fields,
                ..
            } => {
                matches!(ctx.interner.resolve(*type_name), "Seq" | "List" | "Vec")
                    && init_fields.is_empty()
            }
            Expr::List(items) => items.is_empty(),
            _ => false,
        };
        if !fresh {
            continue;
        }
        let c_type = match ty {
            Some(ty) => resolve_type_expr_with_registry(ty, ctx.interner, Some(ctx.registry)),
            None => infer_expr_type(value, ctx),
        };
        if seq_fn(&c_type, "new", ctx.interner).is_some() {
            inner = Some((*var, idx, c_type));
            break;
        }
    }
    let (inner, inner_let, ty) = inner?;

    let (outer, move_set) = body.iter().enumerate().rev().find_map(|(idx, stmt)| {
        let Stmt::Set {
            target,
            value: Expr::Identifier(source),
        } = stmt
        else {
            return None;
        };
        (*source == inner && *target != inner).then_some((*target, idx))
    })?;

    if move_set + 1 == body.len() {
        // The move is the final statement.
    } else if move_set + 2 == body.len()
        && unit_increment(&body[move_set + 1])
        && !matches!(&body[move_set + 1], Stmt::Set { target, .. } if *target == inner)
    {
        // The only following statement advances the outer-loop counter.
    } else {
        return None;
    }

    let Stmt::Set { value, .. } = &body[move_set] else {
        unreachable!()
    };
    if ctx.vars.get(&outer) != Some(&ty)
        || super::values::binding_copies(value, &ty, outer, &body[move_set], ctx)
    {
        return None;
    }

    Some(CBufferReuse {
        inner,
        outer,
        inner_let,
        move_set,
        ty,
    })
}

/// Emit one iteration of a loop whose fresh inner sequence has been hoisted.
/// The matcher proves the final assignment moves `inner` into `outer`; swapping
/// their sequence structs preserves that move while retaining `outer`'s old
/// allocation for the next iteration.
fn emit_c_buffer_reuse_body(
    body: &[Stmt],
    reuse: &CBufferReuse,
    inner: &str,
    outer: &str,
    ctx: &mut CContext,
    output: &mut String,
    indent: usize,
) {
    let pad = "    ".repeat(indent);
    for (idx, child) in body.iter().enumerate() {
        if idx == reuse.inner_let {
            writeln!(output, "{pad}{inner}.len = 0;").unwrap();
        } else if idx == reuse.move_set {
            let ty = c_type_str_resolved(&reuse.ty, ctx.interner);
            writeln!(
                output,
                "{pad}{{ {ty} __buffer_swap = {outer}; {outer} = {inner}; {inner} = __buffer_swap; }}"
            )
            .unwrap();
        } else {
            codegen_stmt(child, ctx, output, indent);
        }
    }
}

/// Emit the C spelling of the same fixed-window byte comparison recognized by
/// the Rust emitter.  The structural proof lives in one shared matcher; only
/// the final target syntax differs.  Constant windows are deliberately
/// unrolled so gcc sees bare byte loads instead of per-character allocation,
/// UTF-8 walking, and `strcmp` in the authored scalar loop.
fn try_emit_c_byte_compare_window(
    stmt: &Stmt,
    ctx: &CContext,
    output: &mut String,
    indent: usize,
) -> bool {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::Simd) {
        return false;
    }
    let Some(m) = crate::codegen::peephole::match_byte_compare_window(
        stmt,
        ctx.interner,
        ctx.int_oracle.as_deref(),
    ) else {
        return false;
    };
    let Some(len) = m.const_len else { return false };
    if ctx.vars.get(&m.text) != Some(&CType::String)
        || ctx.vars.get(&m.needle) != Some(&CType::String)
    {
        return false;
    }

    let pad = "    ".repeat(indent);
    let inner = "    ".repeat(indent + 1);
    let text = ctx.resolve(m.text);
    let needle = ctx.resolve(m.needle);
    let flag = ctx.resolve(m.flag);
    // The shared matcher may hand us an induction variable promoted to `LInt` by exactness analysis.
    // This fast path performs raw byte addressing, so keep the same explicit Text-index boundary as
    // ordinary indexing instead of emitting illegal pointer arithmetic over an `LInt` aggregate.
    let text_base = narrow_index(
        m.text_base,
        codegen_expr(m.text_base, ctx),
        ctx,
        crate::semantics::failcode::FailCode::TextIndexTooWide,
    );
    let needle_base = narrow_index(
        m.needle_base,
        codegen_expr(m.needle_base, ctx),
        ctx,
        crate::semantics::failcode::FailCode::TextIndexTooWide,
    );
    let flag_value = codegen_expr(m.flag_value, ctx);
    let terms: Vec<String> = (0..len)
        .map(|offset| {
            format!(
                "((const unsigned char *)({text}))[((size_t)(({text_base}) - 1LL)) + {offset}] == \
                 ((const unsigned char *)({needle}))[((size_t)(({needle_base}) - 1LL)) + {offset}]"
            )
        })
        .collect();
    writeln!(output, "{pad}/* shared fixed-window byte compare */").unwrap();
    writeln!(output, "{pad}if (!({})) {{", terms.join(" && ")).unwrap();
    writeln!(output, "{inner}{flag} = {flag_value};").unwrap();
    writeln!(output, "{pad}}}").unwrap();
    crate::optimize::mark_fired(crate::optimization::Opt::Simd);
    true
}

/// Runtime lower-bound guards needed to keep every integer expression over a
/// counted-loop `counter` in a machine word. `None` means one expression still
/// requires the exact binding in the hot body.
///
/// The loop header already proves that the native counter and limit fit i64,
/// and leaves one increment of headroom for an inclusive loop. That proves
/// `counter + 1`. A common split-loop index, `counter - floor (+ 1)`, is also
/// a word when `floor` is a stable, non-negative i64 and the loop starts at or
/// above it; return `floor` so the emitter can put those facts in the fast-path
/// guard. The exact loop remains the fallback when they do not hold.
fn counted_body_counter_word_guards(
    body: &[Stmt],
    counter: Symbol,
    ctx: &CContext,
) -> Option<Vec<Symbol>> {
    let Some(ownership) = ctx.ownership.as_ref() else {
        return None;
    };

    let is_counter = |expr: &Expr| matches!(expr, Expr::Identifier(sym) if *sym == counter);
    let is_one = |expr: &Expr| matches!(expr, Expr::Literal(Literal::Number(1)));
    let counter_minus_name = |expr: &Expr| match expr {
        Expr::BinaryOp {
            op: BinaryOpKind::Subtract,
            left,
            right: Expr::Identifier(floor),
        } if is_counter(left) => Some(*floor),
        _ => None,
    };
    let guarded_shape = |expr: &Expr| -> Option<Option<Symbol>> {
        match expr {
            Expr::BinaryOp {
                op: BinaryOpKind::Add,
                left,
                right,
            } if (is_counter(left) && is_one(right)) || (is_one(left) && is_counter(right)) => {
                Some(None)
            }
            Expr::BinaryOp {
                op: BinaryOpKind::Subtract,
                ..
            } => counter_minus_name(expr).map(Some),
            Expr::BinaryOp {
                op: BinaryOpKind::Add,
                left,
                right,
            } if is_one(right) => counter_minus_name(left).map(Some),
            Expr::BinaryOp {
                op: BinaryOpKind::Add,
                left,
                right,
            } if is_one(left) => counter_minus_name(right).map(Some),
            _ => None,
        }
    };

    let mut safe = true;
    let mut lower_bounds = Vec::new();
    logicaffeine_language::ast_depth::walk_nodes(body, |node| {
        use logicaffeine_language::ast_depth::{Visited, Walk};
        match node {
            Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => {
                safe = false;
                Walk::Past
            }
            Visited::Expr(expr @ Expr::BinaryOp { .. }) => {
                let mut names = Vec::new();
                crate::codegen::peephole::collect_expr_symbols(expr, &mut names);
                if names.contains(&counter)
                    && matches!(infer_expr_type(expr, ctx), CType::Int64 | CType::BigInt)
                    && !crate::optimize::facts::holds_expr(
                        crate::optimize::facts::Fact::RawIntOp,
                        expr,
                    )
                    && !super::word_ranges::fits_word(
                        ownership.words.of_expr(expr, ctx.int_oracle.as_deref()),
                    )
                {
                    match guarded_shape(expr) {
                        Some(None) => {}
                        Some(Some(floor))
                            if floor != counter
                                && ctx.vars.get(&floor) == Some(&CType::Int64)
                                && !ctx.program_statics.contains(&floor)
                                && !crate::codegen::peephole::body_modifies_var(body, floor) =>
                        {
                            if !lower_bounds.contains(&floor) {
                                lower_bounds.push(floor);
                            }
                        }
                        _ => safe = false,
                    }
                }
                Walk::Into
            }
            _ => Walk::Into,
        }
    });
    safe.then_some(lower_bounds)
}

/// Tiled-loop lowering has its own compound guard and currently accepts only
/// bodies whose word proof needs no additional per-counter lower bound.
fn counted_body_counter_is_word_safe(body: &[Stmt], counter: Symbol, ctx: &CContext) -> bool {
    counted_body_counter_word_guards(body, counter, ctx).is_some_and(|guards| guards.is_empty())
}

/// Emit the C spelling of the same three-deep tiled counted loop as the Rust
/// backend.  The optimizer's `TileLegal` fact is the sole reorder proof: this
/// matcher only verifies that the C emitter is looking at the exact structural
/// shape the proof describes.
///
/// The outer counter is already an exact `LInt` binding when a `While` reaches
/// this function.  A runtime representation/start guard therefore keeps the
/// original loop as a cold fallback, while the proven zero-based case shadows
/// all three counters with native words.  That makes the ordinary expression
/// emitter consume the optimizer's `RawIntOp`/range proofs rather than putting
/// exact compare, add, multiply, and index conversions back into the hot nest.
fn try_emit_c_tiled_while(
    stmt: &Stmt,
    ctx: &mut CContext,
    output: &mut String,
    indent: usize,
) -> bool {
    const TILE: i64 = 32;

    if ctx.stepped.is_some()
        || !crate::optimize::active_config().is_on(crate::optimization::Opt::Tile)
    {
        return false;
    }
    let Stmt::While { cond, body, .. } = stmt else {
        return false;
    };
    // `stmt` is borrowed, so destructuring its `&Expr` field binds `cond` as
    // `&&Expr`.  Facts are keyed by the expression node, not by the address of
    // the reference slot inside the statement.
    if !crate::optimize::facts::holds_expr(crate::optimize::facts::Fact::TileLegal, *cond) {
        return false;
    }

    macro_rules! reject_proven_tile {
        ($why:expr) => {{
            if std::env::var_os("LOGOS_DEBUG_C_TILE").is_some() {
                eprintln!("C TileLegal decline: {}", $why);
            }
            return false;
        }};
    }

    let Some((outer_sym, bound, outer_exclusive, outer_pass)) =
        crate::codegen::peephole::counted_loop(cond, body)
    else {
        reject_proven_tile!("outer loop no longer has counted-loop shape");
    };
    let bound_type = infer_expr_type(bound, ctx);
    if !outer_exclusive
        || outer_pass.len() != 2
        || ctx.vars.get(&outer_sym) != Some(&CType::BigInt)
        || ctx.program_statics.contains(&outer_sym)
        || !matches!(bound_type, CType::Int64 | CType::BigInt)
        || !crate::codegen::peephole::is_simple_expr(bound)
    {
        reject_proven_tile!(format!(
            "outer gate: exclusive={outer_exclusive}, pass={}, counter={:?}, static={}, bound={bound_type:?}, simple={}",
            outer_pass.len(),
            ctx.vars.get(&outer_sym),
            ctx.program_statics.contains(&outer_sym),
            crate::codegen::peephole::is_simple_expr(bound)
        ));
    }

    let mid_sym = match &outer_pass[0] {
        Stmt::Let {
            var,
            value: Expr::Literal(Literal::Number(0)),
            ..
        } => *var,
        _ => reject_proven_tile!("middle counter is not initialized to literal zero"),
    };
    let (mid_cond, mid_body) = match &outer_pass[1] {
        Stmt::While { cond, body, .. } => (*cond, *body),
        _ => reject_proven_tile!("middle statement is not a while loop"),
    };
    let Some((matched_mid, mid_bound, mid_exclusive, mid_pass)) =
        crate::codegen::peephole::counted_loop(mid_cond, mid_body)
    else {
        reject_proven_tile!("middle loop no longer has counted-loop shape");
    };
    if matched_mid != mid_sym
        || !mid_exclusive
        || !crate::codegen::peephole::exprs_equal(mid_bound, bound)
        || mid_pass.len() != 2
    {
        reject_proven_tile!(format!(
            "middle gate: same_counter={}, exclusive={mid_exclusive}, same_bound={}, pass={}",
            matched_mid == mid_sym,
            crate::codegen::peephole::exprs_equal(mid_bound, bound),
            mid_pass.len()
        ));
    }

    let inner_sym = match &mid_pass[0] {
        Stmt::Let {
            var,
            value: Expr::Literal(Literal::Number(0)),
            ..
        } => *var,
        _ => reject_proven_tile!("inner counter is not initialized to literal zero"),
    };
    let (inner_cond, inner_body) = match &mid_pass[1] {
        Stmt::While { cond, body, .. } => (*cond, *body),
        _ => reject_proven_tile!("inner statement is not a while loop"),
    };
    let Some((matched_inner, inner_bound, inner_exclusive, inner_pass)) =
        crate::codegen::peephole::counted_loop(inner_cond, inner_body)
    else {
        reject_proven_tile!("inner loop no longer has counted-loop shape");
    };
    if matched_inner != inner_sym
        || !inner_exclusive
        || !crate::codegen::peephole::exprs_equal(inner_bound, bound)
        || outer_sym == mid_sym
        || outer_sym == inner_sym
        || mid_sym == inner_sym
    {
        reject_proven_tile!(format!(
            "inner gate: same_counter={}, exclusive={inner_exclusive}, same_bound={}, counters_distinct={}",
            matched_inner == inner_sym,
            crate::codegen::peephole::exprs_equal(inner_bound, bound),
            outer_sym != mid_sym && outer_sym != inner_sym && mid_sym != inner_sym
        ));
    }

    // Reordering a loop containing early exits would change which loop they
    // leave.  Dependence analysis already rejects them; keep this local guard
    // so a future proof expansion cannot silently widen C's lowering.
    let mut nonlocal_flow = false;
    logicaffeine_language::ast_depth::walk_nodes(inner_pass, |node| {
        use logicaffeine_language::ast_depth::{Visited, Walk};
        match node {
            Visited::Stmt(Stmt::Break | Stmt::Return { .. }) => {
                nonlocal_flow = true;
                Walk::Past
            }
            Visited::Stmt(Stmt::FunctionDef { .. }) => Walk::Past,
            _ => Walk::Into,
        }
    });
    if nonlocal_flow
        || !counted_body_counter_is_word_safe(inner_pass, outer_sym, ctx)
        || !counted_body_counter_is_word_safe(inner_pass, mid_sym, ctx)
        || !counted_body_counter_is_word_safe(inner_pass, inner_sym, ctx)
    {
        reject_proven_tile!(format!(
            "body gate: nonlocal={nonlocal_flow}, outer_word_safe={}, middle_word_safe={}, inner_word_safe={}",
            counted_body_counter_is_word_safe(inner_pass, outer_sym, ctx),
            counted_body_counter_is_word_safe(inner_pass, mid_sym, ctx),
            counted_body_counter_is_word_safe(inner_pass, inner_sym, ctx)
        ));
    }

    let mut bound_names = Vec::new();
    crate::codegen::peephole::collect_expr_symbols(bound, &mut bound_names);
    if bound_names.contains(&outer_sym)
        || bound_names.contains(&mid_sym)
        || bound_names.contains(&inner_sym)
        || bound_names.iter().any(|name| {
            crate::codegen::peephole::body_modifies_var(body, *name)
                || crate::codegen::peephole::body_mutates_collection(body, *name)
        })
    {
        reject_proven_tile!("the shared bound changes inside the proven loop band");
    }

    let pad = "    ".repeat(indent);
    let one = "    ".repeat(indent + 1);
    let two = "    ".repeat(indent + 2);
    let three = "    ".repeat(indent + 3);
    let four = "    ".repeat(indent + 4);
    let five = "    ".repeat(indent + 5);
    let six = "    ".repeat(indent + 6);
    let seven = "    ".repeat(indent + 7);
    let outer_exact = ctx.resolve(outer_sym);
    let hi = ctx.temp("__tile_hi", CType::Int64);
    let outer_tile = ctx.temp("__tile_outer", CType::Int64);
    let mid_tile = ctx.temp("__tile_mid", CType::Int64);
    let inner_tile = ctx.temp("__tile_inner", CType::Int64);
    let outer_end = ctx.temp("__tile_outer_end", CType::Int64);
    let mid_end = ctx.temp("__tile_mid_end", CType::Int64);
    let inner_end = ctx.temp("__tile_inner_end", CType::Int64);
    // The versioning guard that owns `TileLegal` bounds every name used by the
    // nest before this fast clone runs. An exact bound therefore fits a word
    // here; extract it once instead of rejecting the shared proof merely
    // because C chose `LInt` for the source binding.
    let bound_value = narrow_i64_sink(bound, codegen_expr(bound, ctx), ctx);

    writeln!(
        output,
        "{pad}/* shared TileLegal: 32x32x32 native C tile */"
    )
    .unwrap();
    writeln!(output, "{pad}{{").unwrap();
    writeln!(
        output,
        "{one}{} = {bound_value};",
        ctx.decl_head(&CType::Int64, &hi)
    )
    .unwrap();
    writeln!(
        output,
        "{one}if (__builtin_expect(!{outer_exact}.big && {outer_exact}.small == 0 && {hi} >= 0, 1)) {{"
    )
    .unwrap();

    ctx.enter_scope();
    let outer = ctx.declare(outer_sym, CType::Int64);
    let mid = ctx.declare(mid_sym, CType::Int64);
    let inner = ctx.declare(inner_sym, CType::Int64);
    writeln!(output, "{two}{};", ctx.decl_head(&CType::Int64, &outer)).unwrap();
    writeln!(output, "{two}{};", ctx.decl_head(&CType::Int64, &mid)).unwrap();
    writeln!(output, "{two}{};", ctx.decl_head(&CType::Int64, &inner)).unwrap();
    writeln!(
        output,
        "{two}for (int64_t {outer_tile} = 0; {outer_tile} < {hi};) {{"
    )
    .unwrap();
    writeln!(
        output,
        "{three}int64_t {outer_end} = ({hi} - {outer_tile} > {TILE}) ? {outer_tile} + {TILE} : {hi};"
    )
    .unwrap();
    writeln!(
        output,
        "{three}for (int64_t {mid_tile} = 0; {mid_tile} < {hi};) {{"
    )
    .unwrap();
    writeln!(
        output,
        "{four}int64_t {mid_end} = ({hi} - {mid_tile} > {TILE}) ? {mid_tile} + {TILE} : {hi};"
    )
    .unwrap();
    writeln!(
        output,
        "{four}for (int64_t {inner_tile} = 0; {inner_tile} < {hi};) {{"
    )
    .unwrap();
    writeln!(
        output,
        "{five}int64_t {inner_end} = ({hi} - {inner_tile} > {TILE}) ? {inner_tile} + {TILE} : {hi};"
    )
    .unwrap();
    writeln!(
        output,
        "{five}for ({outer} = {outer_tile}; {outer} < {outer_end}; {outer}++) {{"
    )
    .unwrap();
    writeln!(
        output,
        "{six}for ({mid} = {mid_tile}; {mid} < {mid_end}; {mid}++) {{"
    )
    .unwrap();
    writeln!(
        output,
        "{seven}for ({inner} = {inner_tile}; {inner} < {inner_end}; {inner}++) {{"
    )
    .unwrap();
    ctx.in_loop(|ctx| {
        for child in inner_pass {
            codegen_stmt(child, ctx, output, indent + 8);
        }
    });
    writeln!(output, "{seven}}}").unwrap();
    writeln!(output, "{six}}}").unwrap();
    writeln!(output, "{five}}}").unwrap();
    writeln!(output, "{five}{inner_tile} = {inner_end};").unwrap();
    writeln!(output, "{four}}}").unwrap();
    writeln!(output, "{four}{mid_tile} = {mid_end};").unwrap();
    writeln!(output, "{three}}}").unwrap();
    writeln!(output, "{three}{outer_tile} = {outer_end};").unwrap();
    writeln!(output, "{two}}}").unwrap();
    writeln!(output, "{two}{outer_exact} = lint_of({hi});").unwrap();
    ctx.exit_scope();

    writeln!(output, "{one}}} else {{").unwrap();
    let exact_cond = c_truthy(cond, ctx);
    writeln!(output, "{two}while ({exact_cond}) {{").unwrap();
    ctx.enter_scope();
    ctx.in_loop(|ctx| {
        for child in *body {
            codegen_stmt(child, ctx, output, indent + 3);
        }
    });
    ctx.exit_scope();
    writeln!(output, "{two}}}").unwrap();
    writeln!(output, "{one}}}").unwrap();
    writeln!(output, "{pad}}}").unwrap();
    crate::optimize::mark_fired(crate::optimization::Opt::Tile);
    true
}

#[derive(Clone, Copy)]
struct NativeIntStrengthAccumulator {
    symbol: Symbol,
    stride: i64,
}

/// Exact affine accumulators synthesized by `IntStrength` at the tail of this
/// counted loop. The optimizer fact, rather than a generated-name convention,
/// is the contract between the shared transform and backend lowering.
fn native_int_strength_accumulators(
    pass: &[Stmt],
    ctx: &CContext,
) -> Vec<NativeIntStrengthAccumulator> {
    let mut out = Vec::new();
    for stmt in pass {
        let Stmt::Set { target, value } = stmt else {
            continue;
        };
        if !crate::optimize::facts::holds_expr(
            crate::optimize::facts::Fact::IntStrengthAdvance,
            value,
        ) || ctx.vars.get(target) != Some(&CType::BigInt)
            || ctx.program_statics.contains(target)
        {
            continue;
        }
        let Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } = value
        else {
            continue;
        };
        let stride = match (&**left, &**right) {
            (Expr::Identifier(sym), Expr::Literal(Literal::Number(stride))) if sym == target => {
                *stride
            }
            (Expr::Literal(Literal::Number(stride)), Expr::Identifier(sym)) if sym == target => {
                *stride
            }
            _ => continue,
        };
        if !out
            .iter()
            .any(|acc: &NativeIntStrengthAccumulator| acc.symbol == *target)
        {
            out.push(NativeIntStrengthAccumulator {
                symbol: *target,
                stride,
            });
        }
    }
    out
}

fn c_i128_literal(value: i64) -> String {
    if value == i64::MIN {
        "(-((__int128)INT64_MAX) - 1)".to_string()
    } else {
        format!("((__int128){value}LL)")
    }
}

/// A descending counted `While` — `counter >= limit` or `counter > limit` — whose
/// body steps the counter down by one in exactly one top-level
/// `Set counter to counter - 1`, anywhere in the body: heap sort's construction
/// loop steps last, its extraction loop steps before the sift that reads the
/// stepped value.
struct DescendingLoop<'e, 'a> {
    counter: Symbol,
    limit: &'e Expr<'a>,
    /// `counter > limit`: the step never takes the counter below the limit.
    exclusive: bool,
    /// The statements before the step, which read the counter's value.
    before: &'e [Stmt<'a>],
    /// The statements after the step, which read the value one below it.
    after: &'e [Stmt<'a>],
}

fn descending_counted_loop<'e, 'a>(
    cond: &'e Expr<'a>,
    body: &'e [Stmt<'a>],
) -> Option<DescendingLoop<'e, 'a>> {
    let (counter, limit, exclusive) = match cond {
        Expr::BinaryOp {
            op: op @ (BinaryOpKind::GtEq | BinaryOpKind::Gt),
            left: Expr::Identifier(counter),
            right,
        } => (*counter, *right, matches!(op, BinaryOpKind::Gt)),
        _ => return None,
    };
    let is_step = |stmt: &Stmt| {
        matches!(
            stmt,
            Stmt::Set {
                target,
                value: Expr::BinaryOp {
                    op: BinaryOpKind::Subtract,
                    left: Expr::Identifier(source),
                    right: Expr::Literal(Literal::Number(1)),
                },
            } if *target == counter && *source == counter
        )
    };
    let mut steps = body
        .iter()
        .enumerate()
        .filter(|(_, stmt)| is_step(stmt));
    let (at, _) = steps.next()?;
    if steps.next().is_some() {
        return None;
    }
    Some(DescendingLoop {
        counter,
        limit,
        exclusive,
        before: &body[..at],
        after: &body[at + 1..],
    })
}

/// The smallest and largest constant offsets applied to `counter` by integer
/// arithmetic in `body`, when every otherwise-unbounded use is exactly
/// `counter +/- small_literal`. A descending fast path guards both endpoints
/// with these offsets before shadowing the exact counter as an `int64_t`.
fn descending_counter_offsets(
    body: &[Stmt],
    counter: Symbol,
    ctx: &CContext,
) -> Option<(i64, i64)> {
    let Some(ownership) = ctx.ownership.as_ref() else {
        return None;
    };
    let mut safe = true;
    let mut min_offset = 0i64;
    let mut max_offset = 0i64;
    logicaffeine_language::ast_depth::walk_nodes(body, |node| {
        use logicaffeine_language::ast_depth::{Visited, Walk};
        match node {
            Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => {
                safe = false;
                Walk::Past
            }
            Visited::Expr(expr @ Expr::BinaryOp { op, left, right }) => {
                let mut names = Vec::new();
                crate::codegen::peephole::collect_expr_symbols(expr, &mut names);
                let needs_guard = names.contains(&counter)
                    && matches!(infer_expr_type(expr, ctx), CType::Int64 | CType::BigInt)
                    && !super::word_ranges::fits_word(
                        ownership.words.of_expr(expr, ctx.int_oracle.as_deref()),
                    );
                if needs_guard {
                    let offset = match (op, left, right) {
                        (
                            BinaryOpKind::Add,
                            Expr::Identifier(sym),
                            Expr::Literal(Literal::Number(n)),
                        ) if *sym == counter && n.unsigned_abs() <= (1u64 << 40) => Some(*n),
                        (
                            BinaryOpKind::Add,
                            Expr::Literal(Literal::Number(n)),
                            Expr::Identifier(sym),
                        ) if *sym == counter && n.unsigned_abs() <= (1u64 << 40) => Some(*n),
                        (
                            BinaryOpKind::Subtract,
                            Expr::Identifier(sym),
                            Expr::Literal(Literal::Number(n)),
                        ) if *sym == counter && n.unsigned_abs() <= (1u64 << 40) => n.checked_neg(),
                        _ => None,
                    };
                    if let Some(offset) = offset {
                        min_offset = min_offset.min(offset);
                        max_offset = max_offset.max(offset);
                    } else {
                        safe = false;
                    }
                }
                Walk::Into
            }
            _ => Walk::Into,
        }
    });
    safe.then_some((min_offset, max_offset))
}

fn c_i64_bound(value: i64) -> String {
    match value {
        i64::MIN => "INT64_MIN".to_string(),
        i64::MAX => "INT64_MAX".to_string(),
        other => format!("{other}LL"),
    }
}

/// Descending sibling of [`try_emit_c_native_counted_while`]. Rust already
/// lowers heap-sort's `start >= 0` and `end > 0` loops as native ranges. The C
/// path keeps an exact fallback, but executes the ordinary bounded case with a
/// word counter after guarding every constant offset used by the loop body:
///
/// * the counter must be a local exact integer (never a program static), and
///   one that is already big stays on the exact path;
/// * the invariant limit must be an integer with a machine-word representation;
/// * the body may change the counter only by its one unit step;
/// * every name the limit reads stays invariant through the body; and
/// * every offset the body adds to the counter — before the step to its value,
///   after it to the value one below — keeps the word in range, which the guard
///   checks at both ends of the counter's run before the loop starts.
///
/// Every case outside that proof executes the byte-for-byte exact lowering, so
/// this is a representation fast path, never a change to LOGOS `Int` semantics.
fn try_emit_c_native_descending_while(
    stmt: &Stmt,
    ctx: &mut CContext,
    output: &mut String,
    indent: usize,
) -> bool {
    if ctx.stepped.is_some()
        || !crate::optimize::active_config().is_on(crate::optimization::Opt::Peephole)
    {
        return false;
    }
    let Stmt::While { cond, body, .. } = stmt else {
        return false;
    };
    let Some(DescendingLoop {
        counter,
        limit,
        exclusive,
        before,
        after,
    }) = descending_counted_loop(cond, body)
    else {
        return false;
    };
    let parts = [before, after];
    if ctx.vars.get(&counter) != Some(&CType::BigInt)
        || ctx.program_statics.contains(&counter)
        || infer_expr_type(limit, ctx) != CType::Int64
        || parts
            .iter()
            .any(|part| crate::codegen::peephole::body_modifies_var(part, counter))
        || !crate::codegen::peephole::is_simple_expr(limit)
    {
        return false;
    }
    let mut limit_names = Vec::new();
    crate::codegen::peephole::collect_expr_symbols(limit, &mut limit_names);
    if limit_names.contains(&counter)
        || limit_names.iter().any(|name| {
            parts.iter().any(|part| {
                crate::codegen::peephole::body_modifies_var(part, *name)
                    || crate::codegen::peephole::body_mutates_collection(part, *name)
            })
        })
    {
        return false;
    }
    let Some((mut min_offset, mut max_offset)) = descending_counter_offsets(before, counter, ctx)
    else {
        return false;
    };
    if !after.is_empty() {
        // After the step the body reads the value one below: its own offsets, and the stepped
        // value itself, sit one lower than the counter the guard bounds.
        let Some((after_min, after_max)) = descending_counter_offsets(after, counter, ctx) else {
            return false;
        };
        min_offset = min_offset.min(after_min - 1);
        max_offset = max_offset.max(after_max - 1);
    }
    if !exclusive {
        // An inclusive loop executes its final `counter -= 1` at `counter == limit`.
        min_offset = min_offset.min(-1);
    }
    let low = (i64::MIN as i128 - min_offset as i128) as i64;
    let high = (i64::MAX as i128 - max_offset as i128) as i64;

    let pad = "    ".repeat(indent);
    let inner = "    ".repeat(indent + 1);
    let body_pad = "    ".repeat(indent + 2);
    let outer = ctx.resolve(counter);
    let lo = ctx.temp("__count_lo", CType::Int64);
    let limit_value = codegen_expr(limit, ctx);
    let compare = if exclusive { ">" } else { ">=" };
    writeln!(
        output,
        "{pad}/* exact descending loop: guarded native-word fast path */"
    )
    .unwrap();
    writeln!(output, "{pad}{{").unwrap();
    writeln!(
        output,
        "{inner}{} = {limit_value};",
        ctx.decl_head(&CType::Int64, &lo)
    )
    .unwrap();
    writeln!(
        output,
        "{inner}if (__builtin_expect(!{outer}.big && {outer}.small >= {} && {outer}.small <= {} && {lo} >= {} && {lo} <= {}, 1)) {{",
        c_i64_bound(low),
        c_i64_bound(high),
        c_i64_bound(low),
        c_i64_bound(high),
    )
    .unwrap();

    ctx.enter_scope();
    let native = ctx.declare(counter, CType::Int64);
    writeln!(output, "{body_pad}int64_t {native} = {outer}.small;").unwrap();
    if after.is_empty() {
        writeln!(
            output,
            "{body_pad}for (; {native} {compare} {lo}; {native}--) {{"
        )
        .unwrap();
    } else {
        writeln!(output, "{body_pad}for (; {native} {compare} {lo};) {{").unwrap();
    }
    ctx.enter_scope();
    ctx.in_loop(|ctx| {
        for child in before {
            codegen_stmt(child, ctx, output, indent + 3);
        }
        if !after.is_empty() {
            writeln!(output, "{}{native}--;", "    ".repeat(indent + 3)).unwrap();
            for child in after {
                codegen_stmt(child, ctx, output, indent + 3);
            }
        }
    });
    ctx.exit_scope();
    writeln!(output, "{body_pad}}}").unwrap();
    writeln!(output, "{body_pad}{outer} = lint_of({native});").unwrap();
    ctx.exit_scope();

    writeln!(output, "{inner}}} else {{").unwrap();
    let exact_cond = c_truthy(cond, ctx);
    writeln!(output, "{body_pad}while ({exact_cond}) {{").unwrap();
    ctx.enter_scope();
    ctx.in_loop(|ctx| {
        for child in *body {
            codegen_stmt(child, ctx, output, indent + 3);
        }
    });
    ctx.exit_scope();
    writeln!(output, "{body_pad}}}").unwrap();
    writeln!(output, "{inner}}}").unwrap();
    writeln!(output, "{pad}}}").unwrap();
    crate::optimize::mark_fired(crate::optimization::Opt::Peephole);
    true
}

fn try_emit_c_native_counted_while(
    stmt: &Stmt,
    ctx: &mut CContext,
    output: &mut String,
    indent: usize,
) -> bool {
    if ctx.stepped.is_some()
        || !crate::optimize::active_config().is_on(crate::optimization::Opt::Peephole)
    {
        return false;
    }
    let Stmt::While { cond, body, .. } = stmt else {
        return false;
    };
    let Some((counter, limit, exclusive, pass)) =
        crate::codegen::peephole::counted_loop(cond, body)
    else {
        return false;
    };

    // `counted_loop` identifies the syntax shared by sizing peepholes.  Recheck
    // the stronger semantic facts the Rust range emitter requires before this
    // lowering changes the representation of the induction variable.
    let limit_ty = infer_expr_type(limit, ctx);
    if ctx.vars.get(&counter) != Some(&CType::BigInt)
        || ctx.program_statics.contains(&counter)
        || !matches!(limit_ty, CType::Int64 | CType::BigInt)
        || crate::codegen::peephole::body_modifies_var(pass, counter)
        || !crate::codegen::peephole::is_simple_expr(limit)
    {
        return false;
    }
    let mut limit_names = Vec::new();
    crate::codegen::peephole::collect_expr_symbols(limit, &mut limit_names);
    if limit_names.contains(&counter)
        || limit_names.iter().any(|name| {
            crate::codegen::peephole::body_modifies_var(pass, *name)
                || crate::codegen::peephole::body_mutates_collection(pass, *name)
        })
    {
        return false;
    }
    let reuse = detect_c_buffer_reuse(body, ctx);

    let pad = "    ".repeat(indent);
    let inner = "    ".repeat(indent + 1);
    let body_pad = "    ".repeat(indent + 2);
    let outer = ctx.resolve(counter);
    let hi = ctx.temp("__count_hi", CType::Int64);
    let exact_hi = (limit_ty == CType::BigInt).then(|| ctx.temp("__count_exact_hi", CType::BigInt));
    let word_guards = counted_body_counter_word_guards(pass, counter, ctx);
    let word_body = word_guards.is_some();
    let strength_accumulators = native_int_strength_accumulators(pass, ctx);
    let word_entry_guard = word_guards
        .as_ref()
        .into_iter()
        .flatten()
        .map(|floor| {
            let floor = ctx.resolve(*floor);
            format!(" && {floor} >= 0 && {outer}.small >= {floor}")
        })
        .collect::<String>();
    let limit_value = codegen_expr(limit, ctx);
    let compare = if exclusive { "<" } else { "<=" };
    let final_increment_fits = if exclusive {
        "1".to_string()
    } else {
        format!("{hi} != INT64_MAX")
    };
    let trip_count = if exclusive {
        format!("({outer}.small < {hi} ? ((__int128){hi} - (__int128){outer}.small) : (__int128)0)")
    } else {
        format!("({outer}.small <= {hi} ? ((__int128){hi} - (__int128){outer}.small + 1) : (__int128)0)")
    };
    let strength_outer = strength_accumulators
        .iter()
        .map(|acc| (acc.symbol, acc.stride, ctx.resolve(acc.symbol).to_string()))
        .collect::<Vec<_>>();
    let strength_entry_guard = strength_outer
        .iter()
        .map(|(_, stride, exact)| {
            let end = format!(
                "((__int128){exact}.small + {} * {trip_count})",
                c_i128_literal(*stride)
            );
            format!(
                " && !{exact}.big && {end} >= (__int128)INT64_MIN && {end} <= (__int128)INT64_MAX"
            )
        })
        .collect::<String>();

    writeln!(
        output,
        "{pad}/* exact counted loop: native-word fast path, exact fallback */"
    )
    .unwrap();
    writeln!(output, "{pad}{{").unwrap();
    let reuse_names = reuse.as_ref().map(|reuse| {
        ctx.enter_scope();
        let inner_name = ctx.declare(reuse.inner, reuse.ty.clone());
        let outer_name = ctx.resolve(reuse.outer).to_string();
        let head = ctx.decl_head(&reuse.ty, &inner_name);
        let new_fn = seq_fn(&reuse.ty, "new", ctx.interner)
            .expect("buffer reuse is only detected for a sequence type");
        writeln!(output, "{inner}{head} = {new_fn}();").unwrap();
        (inner_name, outer_name)
    });
    let native_scope = word_body || !strength_outer.is_empty();
    if native_scope {
        ctx.enter_scope();
    }
    let native = if word_body {
        ctx.declare(counter, CType::Int64)
    } else {
        ctx.temp("__count_i", CType::Int64)
    };
    let native_strength = strength_outer
        .iter()
        .map(|(symbol, stride, exact)| {
            let native = ctx.declare(*symbol, CType::Int64);
            (*symbol, *stride, exact.clone(), native)
        })
        .collect::<Vec<_>>();
    let limit_is_word = if let Some(exact_hi) = &exact_hi {
        writeln!(
            output,
            "{inner}{} = {limit_value};",
            ctx.decl_head(&CType::BigInt, exact_hi)
        )
        .unwrap();
        writeln!(
            output,
            "{inner}{} = {exact_hi}.big ? 0LL : {exact_hi}.small;",
            ctx.decl_head(&CType::Int64, &hi)
        )
        .unwrap();
        format!("!{exact_hi}.big && ")
    } else {
        writeln!(
            output,
            "{inner}{} = {limit_value};",
            ctx.decl_head(&CType::Int64, &hi)
        )
        .unwrap();
        String::new()
    };
    writeln!(
        output,
        "{inner}if (__builtin_expect(!{outer}.big && {limit_is_word}({final_increment_fits}){word_entry_guard}{strength_entry_guard}, 1)) {{"
    )
    .unwrap();

    writeln!(
        output,
        "{body_pad}{} = {outer}.small;",
        ctx.decl_head(&CType::Int64, &native)
    )
    .unwrap();
    if !native_strength.is_empty() {
        writeln!(
            output,
            "{body_pad}/* shared IntStrength: guarded native accumulators */"
        )
        .unwrap();
        for (_, _, exact, native_acc) in &native_strength {
            writeln!(
                output,
                "{body_pad}{} = {exact}.small;",
                ctx.decl_head(&CType::Int64, native_acc)
            )
            .unwrap();
        }
    }
    // The fast-path counter is now a proven word, so the shared counted-fill
    // proof can preallocate from it without narrowing an arbitrary-precision
    // value.  The exact fallback deliberately performs no potentially-huge
    // reservation.
    reserve_counted_fills(
        stmt,
        cond,
        body,
        ctx,
        output,
        &body_pad,
        Some((counter, &native)),
    );
    writeln!(
        output,
        "{body_pad}for (; {native} {compare} {hi}; {native}++) {{"
    )
    .unwrap();
    ctx.enter_scope();
    // Keep the source binding exact inside the body.  This lets an expression
    // such as `counter * a_large_exact_integer` retain arbitrary-precision
    // semantics; the C compiler can still fold the small representation and
    // its guards because it sees this assignment on every iteration.
    if !word_body {
        writeln!(output, "{body_pad}    {outer} = lint_of({native});").unwrap();
    }
    ctx.in_loop(|ctx| {
        if let (Some(reuse), Some((inner_name, outer_name))) = (&reuse, &reuse_names) {
            emit_c_buffer_reuse_body(pass, reuse, inner_name, outer_name, ctx, output, indent + 3);
        } else {
            for child in pass {
                codegen_stmt(child, ctx, output, indent + 3);
            }
        }
    });
    ctx.exit_scope();
    writeln!(output, "{body_pad}}}").unwrap();
    writeln!(output, "{body_pad}{outer} = lint_of({native});").unwrap();
    for (_, _, exact, native_acc) in &native_strength {
        writeln!(output, "{body_pad}{exact} = lint_of({native_acc});").unwrap();
    }
    if native_scope {
        ctx.exit_scope();
    }

    writeln!(output, "{inner}}} else {{").unwrap();
    let exact_cond = c_truthy(cond, ctx);
    writeln!(output, "{body_pad}while ({exact_cond}) {{").unwrap();
    ctx.enter_scope();
    ctx.in_loop(|ctx| {
        if let (Some(reuse), Some((inner_name, outer_name))) = (&reuse, &reuse_names) {
            emit_c_buffer_reuse_body(body, reuse, inner_name, outer_name, ctx, output, indent + 3);
        } else {
            for child in *body {
                codegen_stmt(child, ctx, output, indent + 3);
            }
        }
    });
    ctx.exit_scope();
    writeln!(output, "{body_pad}}}").unwrap();
    writeln!(output, "{inner}}}").unwrap();
    if reuse_names.is_some() {
        ctx.exit_scope();
    }
    writeln!(output, "{pad}}}").unwrap();
    crate::optimize::mark_fired(crate::optimization::Opt::Peephole);
    if !native_strength.is_empty() {
        crate::optimize::mark_fired(crate::optimization::Opt::IntStrength);
    }
    true
}

pub(super) fn codegen_stmt(stmt: &Stmt, ctx: &mut CContext, output: &mut String, indent: usize) {
    let framed =
        ctx.ownership.is_some() && ctx.mutating.values().any(|flags| flags.contains(&true));
    if framed {
        let binds = match stmt {
            Stmt::Let { var, .. } => Some(*var),
            Stmt::Set { target, .. } => Some(*target),
            _ => None,
        };
        let frame = super::types::StmtFrame {
            key: super::sharing::stmt_key(stmt),
            binds,
            reads: super::sharing::name_reads(stmt),
        };
        ctx.stmt_frames.borrow_mut().push(frame);
    }
    codegen_stmt_node(stmt, ctx, output, indent);
    if framed {
        ctx.stmt_frames.borrow_mut().pop();
    }
}

fn codegen_stmt_node(stmt: &Stmt, ctx: &mut CContext, output: &mut String, indent: usize) {
    if ctx.stepped.is_some() && super::stepped::scheduling_point(stmt, ctx, output, indent) {
        return;
    }
    let pad = "    ".repeat(indent);
    match stmt {
        Stmt::Let {
            var,
            value,
            ty,
            mutable,
        } => {
            let var_name = ctx.resolve(*var).to_string();
            // At Main's root, a program static's `Let` assigns the file-scope static every function reaches.
            let program_static = ctx.main_root_depth == Some(ctx.decl_scopes.len())
                && ctx.program_statics.contains(var);

            // Determine C type. An undeclared copy of a value-or-nothing variable is value-or-nothing too.
            if ty.is_none() && ctx.number_locals.refused.contains(var) {
                output.push_str(&c_refuse_expr(&format!(
                    "`{}`, which holds an exact number and a real",
                    ctx.interner.resolve(*var)
                )));
                return;
            }
            let c_type = if super::tower::binding(ctx, *var) {
                CType::Tower
            } else if let Some(ty_expr) = ty {
                resolve_type_expr_with_registry(ty_expr, ctx.interner, Some(ctx.registry))
            } else if let Some(widened) = ctx.number_locals.types.get(var) {
                widened.clone()
            } else if let Some((_, ty)) = opt_var(value, ctx) {
                ty
            } else {
                infer_expr_type(value, ctx)
            };
            // A BigInt-flagged local is declared `LInt` even when seeded with a small literal.
            let c_type = if ctx.bigint_vars.contains(var) && c_type == CType::Int64 {
                CType::BigInt
            } else {
                c_type
            };
            // A binding a stored `nothing` reaches holds value-or-nothing.
            let c_type = super::absent::binding(stmt, c_type, ctx);
            if program_static {
                match ctx.static_types.get(var) {
                    Some(bound) if *bound != c_type => {
                        output.push_str(&c_refuse_expr(&format!(
                            "the program's `{}` bound once as {:?} and again as {:?}",
                            var_name, bound, c_type
                        )));
                        return;
                    }
                    _ => {
                        ctx.static_types.insert(*var, c_type.clone());
                    }
                }
            }

            if c_type == CType::Tower {
                let val_str = super::tower::stored(value, ctx);
                let reuse = program_static || ctx.declared_value_here(*var, &c_type);
                let var_name = if program_static {
                    ctx.bind_static(*var, c_type.clone())
                } else {
                    ctx.declare(*var, c_type.clone())
                };
                let head = if reuse {
                    var_name
                } else {
                    ctx.decl_head(&c_type, &var_name)
                };
                writeln!(output, "{pad}{head} = {val_str};").unwrap();
                return;
            }

            if let Some(source) = ctx
                .text_bytes_candidates
                .get(&(stmt as *const Stmt as usize))
                .copied()
            {
                // A stack-local pointer borrows an ordinary text value. Frame and program-static
                // bindings retain owned sequence storage across their longer lifetimes.
                if !program_static
                    && ctx.stepped.is_none()
                    && ctx.vars.get(&source) == Some(&CType::String)
                {
                    let var_name = ctx.declare_text_bytes(*var, source);
                    writeln!(
                        output,
                        "{pad}const unsigned char *{var_name} = (const unsigned char *)({});",
                        ctx.resolve(source)
                    )
                    .unwrap();
                    crate::optimize::mark_fired(crate::optimization::Opt::TextBytesView);
                    return;
                }
            }

            // Consume the same fixed-buffer proof as the Rust AOT emitter,
            // while retaining C's ordinary Seq representation and operations.
            // Because the analysis proves the buffer has exactly `len` pushes,
            // is non-escaping, and is only lent at read-only call positions,
            // `seq_i64_push` never reaches its realloc path.  This closes the
            // backend drift where Rust emitted `[T; N]` but C heap-allocated the
            // same SHA/message/work buffers on every loop iteration.
            if let (Some(&len), CType::SeqI64(_), true) = (
                ctx.fixed_stack_seqs.get(var),
                &c_type,
                matches!(
                    value,
                    Expr::New { type_name, init_fields, .. }
                        if matches!(ctx.interner.resolve(*type_name), "Seq" | "List" | "Vec")
                            && init_fields.is_empty()
                ) || matches!(value, Expr::List(items) if items.is_empty()),
            ) {
                let reuse = program_static || (ctx.declared_value_here(*var, &c_type));
                let var_name = if program_static {
                    ctx.bind_static(*var, c_type.clone())
                } else {
                    ctx.declare(*var, c_type.clone())
                };
                if reuse {
                    writeln!(output, "{pad}{var_name}.len = 0;").unwrap();
                } else {
                    let backing = format!(
                        "__logos_fixed_{}",
                        var_name.replace(|c: char| !c.is_ascii_alphanumeric() && c != '_', "_")
                    );
                    writeln!(output, "{pad}int64_t {backing}[{len}] = {{0}};").unwrap();
                    writeln!(output, "{pad}Seq_i64 {var_name} = (Seq_i64){{{backing}, 0, {len}}}; /* shared fixed Seq */").unwrap();
                }
                crate::optimize::mark_fired(crate::optimization::Opt::Scalarize);
                return;
            }

            // Handle list literals specially — build inline. The DYNAMIC-NAMED Seq elements — a VALUE-TYPE
            // Seq (`Seq_LMoney` …), a user-STRUCT Seq (`Seq_<Name>`), a nested Seq (`Seq_<inner>`), and a
            // Seq-of-TUPLE (`Seq_LTuple_…`) — are the exceptions: they fall through to the general path
            // below, whose `codegen_expr(Expr::List)` materializes the literal in a GCC statement-expression
            // with the right dynamic `Seq_<E>` type + push fn (`c_type_str` can only render them as a
            // `/* … */` placeholder, so a static-name declaration here would be `undeclared` under gcc). A
            // binding that can hold nothing takes the literal as a value-or-nothing on the general path too.
            // Everything else keeps its exact prior behavior here.
            if let Expr::List(elems) = value {
                if mixed_kinds(elems, ctx) {
                    output.push_str(&c_refuse_expr(MIXED_LIST));
                    return;
                }
            }
            if let (Expr::List(elems), false) = (
                value,
                matches!(
                    c_type,
                    CType::SeqValue(_)
                        | CType::SeqStruct(_)
                        | CType::SeqSeq(_)
                        | CType::SeqTuple(_)
                        | CType::Opt(_)
                ),
            ) {
                if !elems.is_empty() {
                    let (new_fn, push_fn) = match &c_type {
                        CType::SeqBool => ("seq_bool_new", "seq_bool_push"),
                        CType::SeqStr => ("seq_str_new", "seq_str_push"),
                        CType::SeqF64 => ("seq_f64_new", "seq_f64_push"),
                        _ => ("seq_i64_new", "seq_i64_push"),
                    };
                    let values: Vec<String> = elems
                        .iter()
                        .map(|e| stored(e, codegen_expr(e, ctx), ctx))
                        .collect();
                    let reuse = program_static || (ctx.declared_value_here(*var, &c_type));
                    let var_name = if program_static {
                        ctx.bind_static(*var, c_type.clone())
                    } else {
                        ctx.declare(*var, c_type.clone())
                    };
                    let head = if reuse {
                        var_name.clone()
                    } else {
                        ctx.decl_head(&c_type, &var_name)
                    };
                    writeln!(output, "{}{} = {}();", pad, head, new_fn).unwrap();
                    for v in values {
                        writeln!(output, "{}{}(&{}, {});", pad, push_fn, var_name, v).unwrap();
                    }
                    return;
                }
            }

            // A text the body appends to is a builder of its own; any other mutable text a copy of its own.
            if c_type == CType::String && (*mutable || builds_text(*var, ctx)) {
                let val_str = if builds_text(*var, ctx) {
                    text_builder_of(value, ctx)
                } else if matches!(value, Expr::Literal(Literal::Text(_)))
                    && crate::optimize::active_config().is_on(crate::optimization::Opt::Borrow)
                {
                    // Rebinding a text pointer never mutates an immutable
                    // program literal. Avoid a strdup on every loop iteration.
                    crate::optimize::mark_fired(crate::optimization::Opt::Borrow);
                    codegen_expr(value, ctx)
                } else {
                    format!("lc_gc_strdup({})", codegen_expr(value, ctx))
                };
                let reuse = program_static || (ctx.declared_value_here(*var, &c_type));
                let var_name = if program_static {
                    ctx.bind_static(*var, c_type)
                } else {
                    ctx.declare(*var, c_type)
                };
                let declarator = if reuse {
                    var_name
                } else {
                    ctx.decl_head(&CType::String, &var_name)
                };
                writeln!(output, "{}{} = {};", pad, declarator, val_str).unwrap();
                return;
            }

            let val_str = match (value, &c_type) {
                (Expr::List(elems), CType::SeqValue(elem)) if !elems.is_empty() => {
                    literal_as(elems, &c_type, elem, ctx)
                }
                (Expr::List(_), seq @ CType::SeqSeq(_)) if super::absent::holds_nothing(seq) => {
                    value_as(value, seq, ctx)
                }
                _ => fresh_collection(value, &c_type, ctx).unwrap_or_else(|| codegen_expr(value, ctx)),
            };
            // Seed an `LInt` local from a raw int64 value (`Let result be 1` → `LInt result = lint_of(1)`);
            // seed a `: Rational`-annotated local by re-emitting its value in rational context so
            // `Let x: Rational be 7 / 2` is the exact `7/2` (`ldec_div`), not the truncated int `3`.
            let val_str = if c_type == CType::BigInt && infer_expr_type(value, ctx) != CType::BigInt
            {
                format!("lint_of((int64_t)({}))", val_str)
            } else if matches!(c_type, CType::Rational | CType::Decimal)
                && !matches!(infer_expr_type(value, ctx), CType::Rational | CType::Decimal)
            {
                codegen_expr_as_rational(value, ctx)
            } else if matches!(c_type, CType::Opt(_)) {
                opt_stored(value, &c_type, *var, stmt, ctx)
            } else if super::values::binding_copies(value, &c_type, *var, stmt, ctx) {
                super::values::copy_value(&c_type, &val_str, ctx)
            } else if c_type == CType::String && shares_builder(value, ctx) {
                format!("lc_gc_strdup({val_str})")
            } else {
                val_str
            };
            // Re-binding a name ALREADY declared in this exact block with the same type (the optimizer's
            // loop-unroll duplicates `Let x …`) assigns that variable — gcc rejects a redefinition (nbody).
            // With another type it declares a variable of its own (`CContext::declare`).
            let reuse = program_static || (ctx.declared_value_here(*var, &c_type));
            let var_name = if program_static {
                ctx.bind_static(*var, c_type.clone())
            } else {
                ctx.declare(*var, c_type.clone())
            };
            // A function-pointer variable (a lambda value) declares with the NAME INSIDE the declarator
            // (`RET (*g)(ARGS)`), which `decl_head`'s `c_param_decl` spells.
            let head = if reuse {
                var_name
            } else {
                ctx.decl_head(&c_type, &var_name)
            };
            writeln!(output, "{}{} = {};", pad, head, val_str).unwrap();
        }
        // A name nothing binds: the value is computed, then the assignment fails, as on the VM.
        Stmt::Set { target, value }
            if !ctx.vars.contains_key(target) && !ctx.program_statics.contains(target) =>
        {
            let failure = format!("Undefined variable: {}", ctx.interner.resolve(*target));
            writeln!(output, "{pad}(void)({});", codegen_expr(value, ctx)).unwrap();
            writeln!(
                output,
                "{pad}{}",
                ctx.fail_stmt(&c_string_literal(&failure))
            )
            .unwrap();
        }
        Stmt::Set { target, value } => {
            // A text the program appends to: `Set t to t + a + b` appends each part in place, in order, when no
            // later part reads `t`; any other value becomes the builder's new text.
            if is_text_builder(*target, ctx) {
                let builder = ctx.resolve(*target);
                if let Some(appended) = self_appended_parts(*target, value, ctx) {
                    for part in appended {
                        let fast = crate::optimize::active_config()
                            .is_on(crate::optimization::Opt::StringAccumReuse);
                        if fast && infer_expr_type(part, ctx) == CType::Int64 {
                            crate::optimize::mark_fired(crate::optimization::Opt::StringAccumReuse);
                            writeln!(
                                output,
                                "{pad}logos_text_append_i64(&{builder}, {});",
                                codegen_expr(part, ctx)
                            )
                            .unwrap();
                            continue;
                        }
                        if fast {
                            if let Expr::Literal(Literal::Text(sym)) = part {
                                let bytes = ctx.interner.resolve(*sym).as_bytes();
                                if bytes.len() == 1 && bytes[0] != 0 && bytes[0].is_ascii() {
                                    crate::optimize::mark_fired(
                                        crate::optimization::Opt::StringAccumReuse,
                                    );
                                    writeln!(
                                        output,
                                        "{pad}logos_text_push_ascii(&{builder}, {});",
                                        bytes[0]
                                    )
                                    .unwrap();
                                    continue;
                                }
                            }
                        }
                        let (text, owned) = text_part(part, ctx);
                        let release = if owned {
                            " lc_gc_free((void *)__appended);"
                        } else {
                            ""
                        };
                        writeln!(output, "{pad}{{ const char *__appended = {text}; logos_text_append(&{builder}, __appended);{release} }}").unwrap();
                    }
                } else {
                    writeln!(output, "{pad}{builder} = {};", text_builder_of(value, ctx)).unwrap();
                }
                return;
            }
            if ctx.vars.get(target).or_else(|| ctx.static_types.get(target)) == Some(&CType::Tower) {
                let val_str = super::tower::stored(value, ctx);
                writeln!(output, "{pad}{} = {val_str};", ctx.resolve(*target)).unwrap();
                return;
            }
            let val_str = match (value, ctx.vars.get(target).cloned()) {
                (Expr::List(elems), Some(seq @ CType::SeqValue(_))) if !elems.is_empty() => {
                    let CType::SeqValue(elem) = &seq else { unreachable!("matched above") };
                    literal_as(elems, &seq, elem, ctx)
                }
                (_, Some(ty)) => fresh_collection(value, &ty, ctx).unwrap_or_else(|| codegen_expr(value, ctx)),
                _ => codegen_expr(value, ctx),
            };
            // Reassigning an `LInt` local a raw int64 value (`Set result to 1`) promotes via `lint_of`;
            // reassigning a Rational local an int-arithmetic value re-emits it in rational context.
            let val_str = if ctx.vars.get(target) == Some(&CType::BigInt)
                && infer_expr_type(value, ctx) != CType::BigInt
            {
                format!("lint_of((int64_t)({}))", val_str)
            } else if matches!(ctx.vars.get(target), Some(CType::Rational | CType::Decimal))
                && !matches!(infer_expr_type(value, ctx), CType::Rational | CType::Decimal)
            {
                codegen_expr_as_rational(value, ctx)
            } else if let Some(ty @ CType::Opt(_)) = ctx.vars.get(target).cloned() {
                opt_stored(value, &ty, *target, stmt, ctx)
            } else if ctx.vars.get(target) == Some(&CType::String)
                && builds_text(*target, ctx)
                && !super::values::is_fresh(value)
            {
                // A text appended to in place holds a buffer of its own.
                format!("lc_gc_strdup({val_str})")
            } else if let Some(ty) = ctx
                .vars
                .get(target)
                .cloned()
                .filter(|ty| super::values::binding_copies(value, ty, *target, stmt, ctx))
            {
                super::values::copy_value(&ty, &val_str, ctx)
            } else if ctx.vars.get(target) == Some(&CType::String) && shares_builder(value, ctx) {
                format!("lc_gc_strdup({val_str})")
            } else {
                val_str
            };
            let var_name = ctx.resolve(*target);
            writeln!(output, "{}{} = {};", pad, var_name, val_str).unwrap();
        }
        // `Give x to f` hands `x` to its recipient exactly as `Show x to f` does.
        Stmt::Give { object, recipient } => {
            codegen_stmt_node(
                &Stmt::Show {
                    object: *object,
                    recipient: *recipient,
                },
                ctx,
                output,
                indent,
            );
        }
        Stmt::Show { object, recipient } => {
            // Only `show` itself prints: any other recipient is called with the value, and a recipient that is
            // not a name only has the value computed.
            match recipient {
                Expr::Identifier(f) if ctx.interner.resolve(*f) == "show" => {}
                Expr::Identifier(f) => {
                    emit_recipient_call(object, *f, ctx, output, &pad);
                    return;
                }
                _ => {
                    writeln!(output, "{pad}(void)({});", codegen_expr(object, ctx)).unwrap();
                    return;
                }
            }
            if let Expr::InterpolatedString(parts) = object {
                let holes: Vec<&Expr> = parts
                    .iter()
                    .filter_map(|p| match p {
                        crate::ast::stmt::StringPart::Expr { value, .. } => Some(*value),
                        crate::ast::stmt::StringPart::Literal(_) => None,
                    })
                    .collect();
                let cells = sequence_operands(&holes, ctx, output, &pad);
                codegen_interpolated_show_c(parts, ctx, &pad, output);
                forget_operands(cells, ctx);
                return;
            }
            if let Expr::Literal(Literal::Nothing) = object {
                writeln!(output, "{}printf(\"nothing\\n\");", pad).unwrap();
                return;
            }
            // A value-or-nothing variable: the value if present, else `nothing` (the VM's Nothing). A scalar
            // prints straight from its cell; any other value through its rendering.
            if let Some((var, ty)) = opt_var(object, ctx) {
                let element = opt_element(&ty);
                let show = match element {
                    CType::Int64 => Some("show_i64"),
                    CType::Float64 => Some("show_f64"),
                    CType::Bool => Some("show_bool"),
                    CType::String => Some("show_str"),
                    _ => None,
                };
                match show {
                    Some(show) => {
                        let value =
                            task_arg_unpack(&element, &format!("({var}).value"), ctx.interner);
                        writeln!(output, "{pad}if (({var}).present) {{ {show}({value}); }} else {{ printf(\"nothing\\n\"); }}").unwrap();
                    }
                    None => writeln!(
                        output,
                        "{pad}logos_show_text({});",
                        c_text_of(&ty, &var, ctx)
                    )
                    .unwrap(),
                }
                return;
            }
            let ty = infer_expr_type(object, ctx);
            let val_str = codegen_expr(object, ctx);
            match ty {
                // A value-or-nothing, or a value holding them, renders as their payloads do, `nothing` where one is
                // absent (a value-or-nothing variable is shown above).
                ty if super::absent::holds_nothing(&ty) => {
                    let render = crate::codegen_c::types::c_field_to_string_expr(&ty, "__c", ctx.interner);
                    writeln!(
                        output,
                        "{pad}{{ {} __c = {val_str}; char *__s = {render}; printf(\"%s\\n\", __s); lc_gc_free(__s); }}",
                        c_type_str_resolved(&ty, ctx.interner)
                    )
                    .unwrap();
                }
                CType::Int64 => writeln!(output, "{}show_i64({});", pad, val_str).unwrap(),
                CType::Word(_) => writeln!(
                    output,
                    "{pad}printf(\"%\" PRIu64 \"\\n\", (uint64_t)({val_str}));"
                )
                .unwrap(),
                CType::Float64 => writeln!(output, "{}show_f64({});", pad, val_str).unwrap(),
                CType::Complex => writeln!(output, "{}show_complex({});", pad, val_str).unwrap(),
                CType::Rational | CType::Decimal => writeln!(output, "{}show_ldec({});", pad, val_str).unwrap(),
                CType::Tower => writeln!(output, "{pad}show_ltower({val_str});").unwrap(),
                CType::BigInt => writeln!(output, "{}show_lint({});", pad, val_str).unwrap(),
                CType::Modular => writeln!(output, "{}show_mod({});", pad, val_str).unwrap(),
                CType::Uuid => writeln!(output, "{}show_uuid({});", pad, val_str).unwrap(),
                CType::Money => writeln!(output, "{}show_money({});", pad, val_str).unwrap(),
                CType::Quantity => writeln!(output, "{}show_quant({});", pad, val_str).unwrap(),
                CType::Bool => writeln!(output, "{}show_bool({});", pad, val_str).unwrap(),
                CType::String => writeln!(output, "{}show_str({});", pad, val_str).unwrap(),
                CType::Moment => writeln!(output, "{}show_moment({});", pad, val_str).unwrap(),
                CType::Date => writeln!(output, "{}show_date({});", pad, val_str).unwrap(),
                CType::Time => writeln!(output, "{}show_time({});", pad, val_str).unwrap(),
                CType::Duration => writeln!(output, "{}show_duration({});", pad, val_str).unwrap(),
                CType::Span => writeln!(output, "{}show_span({});", pad, val_str).unwrap(),
                // A collection of machine words shows each by its unsigned value, of temporal values each as itself.
                _ if crate::codegen_c::types::holds_ints_read_apart(&ty) => {
                    let render = crate::codegen_c::types::c_field_to_string_expr(
                        &ty,
                        "__c",
                        ctx.interner,
                    );
                    writeln!(
                        output,
                        "{pad}{{ {} __c = {val_str}; char *__s = {render}; printf(\"%s\\n\", __s); lc_gc_free(__s); }}",
                        c_type_str(&ty)
                    )
                    .unwrap();
                }
                CType::SeqI64(_) => {
                    if let Expr::Identifier(_) = object {
                        writeln!(output, "{}show_seq_i64(&{});", pad, val_str).unwrap();
                    } else {
                        writeln!(
                            output,
                            "{}{{ Seq_i64 __tmp = {}; show_seq_i64(&__tmp); }}",
                            pad, val_str
                        )
                        .unwrap();
                    }
                }
                CType::SeqBool => {
                    if let Expr::Identifier(_) = object {
                        writeln!(output, "{}show_seq_bool(&{});", pad, val_str).unwrap();
                    } else {
                        writeln!(
                            output,
                            "{}{{ Seq_bool __tmp = {}; show_seq_bool(&__tmp); }}",
                            pad, val_str
                        )
                        .unwrap();
                    }
                }
                CType::SeqStr => {
                    if let Expr::Identifier(_) = object {
                        writeln!(output, "{}show_seq_str(&{});", pad, val_str).unwrap();
                    } else {
                        writeln!(
                            output,
                            "{}{{ Seq_str __tmp = {}; show_seq_str(&__tmp); }}",
                            pad, val_str
                        )
                        .unwrap();
                    }
                }
                CType::SeqF64 => {
                    if let Expr::Identifier(_) = object {
                        writeln!(output, "{}show_seq_f64(&{});", pad, val_str).unwrap();
                    } else {
                        writeln!(
                            output,
                            "{}{{ Seq_f64 __tmp = {}; show_seq_f64(&__tmp); }}",
                            pad, val_str
                        )
                        .unwrap();
                    }
                }
                // A Seq of a value type (`Show [money(…), …]`) → its `show_seq_<E>` (`[a, b]` format).
                CType::SeqValue(inner) => {
                    let e = c_type_str(&inner);
                    if let Expr::Identifier(_) = object {
                        writeln!(output, "{}show_seq_{e}(&{});", pad, val_str).unwrap();
                    } else {
                        writeln!(
                            output,
                            "{}{{ Seq_{e} __tmp = {}; show_seq_{e}(&__tmp); }}",
                            pad, val_str
                        )
                        .unwrap();
                    }
                }
                // A nested Seq (`Show [[1,2],[3,4]]`) → `show_seq_<inner>` (`[[..], ..]`), where the inner
                // is itself a `Seq_…`. Its family + `to_string` are synthesized in `codegen_c::mod`.
                CType::SeqSeq(inner) => {
                    let elem = c_type_str_resolved(&inner, ctx.interner); // e.g. "Seq_i64"
                    let tn = format!("Seq_{elem}"); // "Seq_Seq_i64"
                    if let Expr::Identifier(_) = object {
                        writeln!(output, "{}show_seq_{elem}(&{});", pad, val_str).unwrap();
                    } else {
                        writeln!(
                            output,
                            "{}{{ {tn} __tmp = {}; show_seq_{elem}(&__tmp); }}",
                            pad, val_str
                        )
                        .unwrap();
                    }
                }
                // A tuple / named record (`Show (1, 2)`) → `<LTuple>_to_string` (PARENS). Records share the
                // positional `LTuple` struct, so both use the same renderer.
                CType::Tuple(elems) => {
                    let tn = crate::codegen_c::types::tuple_c_name(&elems, ctx.interner);
                    writeln!(
                        output,
                        "{}{{ char *__s = {}_to_string({}); printf(\"%s\\n\", __s); lc_gc_free(__s); }}",
                        pad, tn, val_str
                    )
                    .unwrap();
                }
                CType::Record(fields) => {
                    let elems: Vec<CType> = fields.iter().map(|(_, t)| t.clone()).collect();
                    let tn = crate::codegen_c::types::tuple_c_name(&elems, ctx.interner);
                    writeln!(
                        output,
                        "{}{{ char *__s = {}_to_string({}); printf(\"%s\\n\", __s); lc_gc_free(__s); }}",
                        pad, tn, val_str
                    )
                    .unwrap();
                }
                // A Seq of tuples (`Show [(1,2),(3,4)]`) → `show_seq_<LTuple>` (`[(1, 2), (3, 4)]`).
                CType::SeqTuple(elems) => {
                    let tn = crate::codegen_c::types::tuple_c_name(&elems, ctx.interner);
                    let sq = format!("Seq_{tn}");
                    if let Expr::Identifier(_) = object {
                        writeln!(output, "{}show_seq_{tn}(&{});", pad, val_str).unwrap();
                    } else {
                        writeln!(
                            output,
                            "{}{{ {sq} __tmp = {}; show_seq_{tn}(&__tmp); }}",
                            pad, val_str
                        )
                        .unwrap();
                    }
                }
                // A Seq of a showable user struct (`Show [p1, p2]`) → its `show_seq_<Name>` (each element
                // via `<Name>_to_string`). Only when the element struct is C-renderable (see the Struct arm).
                CType::SeqStruct(sym)
                    if crate::codegen_c::types::c_user_type_showable(
                        sym,
                        ctx.registry,
                        ctx.interner,
                    ) =>
                {
                    let n = escape_c_ident(ctx.interner.resolve(sym));
                    let sq = crate::codegen_c::types::seq_struct_type(sym, ctx.interner);
                    if let Expr::Identifier(_) = object {
                        writeln!(output, "{}show_seq_{n}(&{});", pad, val_str).unwrap();
                    } else {
                        writeln!(
                            output,
                            "{}{{ {sq} __tmp = {}; show_seq_{n}(&__tmp); }}",
                            pad, val_str
                        )
                        .unwrap();
                    }
                }
                // Maps render `{k: v, …}` and Sets `{a, …}` in INSERTION order (the C runtime carries an
                // `order` array), byte-identical to the interpreter oracle. `show_<T>` takes a pointer.
                CType::MapI64I64(_, _)
                | CType::MapStrI64(_)
                | CType::MapStrStr
                | CType::MapI64Str(_)
                | CType::MapStrValue(_)
                | CType::MapI64Value(_)
                | CType::SetI64(_)
                | CType::SetStr => {
                    let (tn, fun): (String, String) = match &ty {
                        CType::MapI64I64(_, _) => ("Map_i64_i64".into(), "show_map_i64_i64".into()),
                        CType::MapStrI64(_) => ("Map_str_i64".into(), "show_map_str_i64".into()),
                        CType::MapStrStr => ("Map_str_str".into(), "show_map_str_str".into()),
                        CType::MapI64Str(_) => ("Map_i64_str".into(), "show_map_i64_str".into()),
                        CType::MapStrValue(inner) => {
                            let e = c_type_str_resolved(inner, ctx.interner);
                            (format!("Map_str_{e}"), format!("show_map_str_{e}"))
                        }
                        CType::MapI64Value(inner) => {
                            let e = c_type_str_resolved(inner, ctx.interner);
                            (format!("Map_i64_{e}"), format!("show_map_i64_{e}"))
                        }
                        CType::SetI64(_) => ("Set_i64".into(), "show_set_i64".into()),
                        CType::SetStr => ("Set_str".into(), "show_set_str".into()),
                        _ => unreachable!(),
                    };
                    if let Expr::Identifier(_) = object {
                        writeln!(output, "{}{}(&{});", pad, fun, val_str).unwrap();
                    } else {
                        writeln!(
                            output,
                            "{}{{ {tn} __tmp = {}; {fun}(&__tmp); }}",
                            pad, val_str
                        )
                        .unwrap();
                    }
                }
                // A whole enum value (`Show m`) → `<Name>_to_string` (`Just(42)`, `Leaf`, an Option's payload).
                CType::Enum(sym)
                    if crate::codegen_c::types::c_enum_showable(
                        sym,
                        ctx.registry,
                        ctx.interner,
                    ) =>
                {
                    let ts = crate::codegen_c::types::struct_to_string_fn(sym, ctx.interner);
                    writeln!(
                        output,
                        "{}{{ char *__s = {}({}); printf(\"%s\\n\", __s); lc_gc_free(__s); }}",
                        pad, ts, val_str
                    )
                    .unwrap();
                }
                CType::Struct(sym)
                    if crate::codegen_c::types::c_struct_showable(
                        sym,
                        ctx.registry,
                        ctx.interner,
                    ) =>
                {
                    // Whole-value Show via the generated `<Name>_to_string` (braces, sorted fields,
                    // nested structs recursive) — the one renderer shared with nested fields + Seq-of-struct.
                    let ts = crate::codegen_c::types::struct_to_string_fn(sym, ctx.interner);
                    writeln!(
                        output,
                        "{}{{ char *__s = {}({}); printf(\"%s\\n\", __s); lc_gc_free(__s); }}",
                        pad, ts, val_str
                    )
                    .unwrap();
                }
                CType::Struct(sym) => {
                    if let Some(crate::analysis::TypeDef::Struct { fields, .. }) =
                        ctx.registry.get(sym)
                    {
                        let struct_name = ctx.display_name(sym);
                        // Print struct as `StructName { field1: val1, field2: val2 }` — byte-identical to the
                        // interpreter oracle: brace-delimited, fields SORTED by name (a struct's field storage
                        // is unordered, so a stable display order is what keeps every tier in lockstep), and an
                        // empty struct shows just its name. Display uses the raw LOGOS field name (what the
                        // oracle prints); field ACCESS uses the escaped C member name.
                        let mut sorted: Vec<&crate::analysis::FieldDef> = fields.iter().collect();
                        sorted.sort_by(|a, b| {
                            ctx.interner
                                .resolve(a.name)
                                .cmp(ctx.interner.resolve(b.name))
                        });
                        if sorted.is_empty() {
                            writeln!(output, "{}printf(\"{}\\n\");", pad, struct_name).unwrap();
                        } else {
                            let mut fmt_parts = Vec::new();
                            let mut arg_parts = Vec::new();
                            fmt_parts.push(format!("{} {{ ", struct_name));
                            for (i, f) in sorted.iter().enumerate() {
                                let disp_name = ctx.interner.resolve(f.name);
                                let acc_name = escape_c_ident(ctx.interner.resolve(f.name));
                                let ctype = super::exact::field_ctype(
                                    sym,
                                    f,
                                    ctx.interner,
                                    ctx.registry,
                                    &ctx.field_facts(),
                                );
                                if i > 0 {
                                    fmt_parts.push(", ".to_string());
                                }
                                // A value-type field renders via its `*_to_string` (%s), not the `%lld`
                                // that would print the struct as a garbage `0`. The buffer leaks — fine in
                                // the short-lived program (same convention as elsewhere).
                                let (fmt_spec, arg): (&str, String) = match ctype {
                                    CType::Int64 => {
                                        ("%\" PRId64 \"", format!("{}.{}", val_str, acc_name))
                                    }
                                    CType::Float64 => {
                                        ("%s", format!("logos_fmt_f64({}.{})", val_str, acc_name))
                                    }
                                    CType::Bool => (
                                        "%s",
                                        format!("{}.{} ? \"true\" : \"false\"", val_str, acc_name),
                                    ),
                                    CType::String => ("%s", format!("{}.{}", val_str, acc_name)),
                                    CType::Money => {
                                        ("%s", format!("money_to_string({}.{})", val_str, acc_name))
                                    }
                                    CType::Quantity => {
                                        ("%s", format!("quant_to_string({}.{})", val_str, acc_name))
                                    }
                                    CType::Modular => {
                                        ("%s", format!("mod_to_string({}.{})", val_str, acc_name))
                                    }
                                    CType::Uuid => {
                                        ("%s", format!("uuid_to_string({}.{})", val_str, acc_name))
                                    }
                                    CType::Complex => {
                                        ("%s", format!("cplx_to_string({}.{})", val_str, acc_name))
                                    }
                                    CType::BigInt => {
                                        ("%s", format!("lint_to_string({}.{})", val_str, acc_name))
                                    }
                                    CType::Rational | CType::Decimal => (
                                        "%s",
                                        format!(
                                            "({{ LDec __d = {}.{}; ldec_to_string(&__d); }})",
                                            val_str, acc_name
                                        ),
                                    ),
                                    other => (
                                        "%s",
                                        c_text_of(
                                            &other,
                                            &format!("{}.{}", val_str, acc_name),
                                            ctx,
                                        ),
                                    ),
                                };
                                fmt_parts.push(format!("{}: {}", disp_name, fmt_spec));
                                arg_parts.push(arg);
                            }
                            fmt_parts.push(" }".to_string());
                            let fmt_string = fmt_parts.join("");
                            writeln!(
                                output,
                                "{}printf(\"{}\\n\", {});",
                                pad,
                                fmt_string,
                                arg_parts.join(", ")
                            )
                            .unwrap();
                        }
                    } else {
                        writeln!(output, "{}show_i64({});", pad, val_str).unwrap();
                    }
                }
                other => writeln!(
                    output,
                    "{}logos_show_text({});",
                    pad,
                    c_text_of(&other, &val_str, ctx)
                )
                .unwrap(),
            }
        }
        // A scope-transparent desugar (e.g. `Let (a, b) be e`) — emit its statements FLAT, same indent.
        // A `Reversibly:` / `Uncompute:` block is likewise output-TRANSPARENT: the C tier has no
        // history-tape notion, so it emits the body FLAT (the forward semantics of the in-place abelian
        // updates are ordinary arithmetic), producing byte-identical output to the same statements
        // outside the block — matching the tree-walker / VM / AOT-Rust tiers.
        Stmt::Splice { body } | Stmt::Reversibly { body } => {
            for inner in body.iter() {
                codegen_stmt(inner, ctx, output, indent);
            }
        }
        // `During <gerund>:` is a lexical TRACE span — no runtime start/stop; its body is ordinary code that
        // must run transparently (the VM/tree-walker emit the body's output, the span name is trace-only).
        Stmt::During { body, .. } => {
            for inner in body.iter() {
                codegen_stmt(inner, ctx, output, indent);
            }
        }
        // `Swap a and b` — the reversible register exchange (`Op::RevSwap`, its own inverse) → a typed
        // temp-swap of the two same-typed locals, byte-identical to the VM/tree-walker's swap.
        // `Swap a and b` exchanges the two values through a temporary of their one C type.
        Stmt::Swap { a, b } => {
            if a == b {
                return;
            }
            let (na, nb) = (ctx.resolve(*a).to_string(), ctx.resolve(*b).to_string());
            let ty = ctx
                .vars
                .get(a)
                .or_else(|| ctx.vars.get(b))
                .cloned()
                .unwrap_or(CType::Int64);
            let tmp = ctx.temp("__swap", ty.clone());
            let head = ctx.decl_head(&ty, &tmp);
            // A builder taking a text that is not one gets a builder of its own; a builder's text passes as is.
            let taking = |to: Symbol, from: Symbol, value: &str| {
                if is_text_builder(to, ctx) && !is_text_builder(from, ctx) {
                    format!("logos_text_builder({value}, 0)")
                } else {
                    value.to_string()
                }
            };
            writeln!(
                output,
                "{pad}{{ {head} = {na}; {na} = {}; {nb} = {}; }}",
                taking(*a, *b, &nb),
                taking(*b, *a, &tmp)
            )
            .unwrap();
        }
        Stmt::Return { value } => {
            // A zone swallows a `Return` made inside it, through any loops: the value is computed and the zone
            // left. In `## Main` a `Return` ends the run, as finished.
            let leaves = ctx
                .enclosing_zone_exit()
                .map(|exit| format!("goto {exit};"))
                .or_else(|| ctx.in_main.then(|| "return 0;".to_string()));
            if let Some(leave) = leaves {
                if let Some(val) = value {
                    writeln!(output, "{pad}(void)({});", codegen_expr(val, ctx)).unwrap();
                }
                writeln!(output, "{pad}{leave}").unwrap();
                return;
            }
            // Rust drops unique Vec temporaries here automatically. Direct C must do the same or a
            // recursive divide-and-conquer function retains its whole allocation tree. The plan is
            // deliberately limited to fresh singleton sharing classes; this final gate admits only
            // scalar sequences with no nested ownership, currently in scope, and never stack-backed
            // or borrowed byte views.
            if crate::optimize::active_config().is_on(crate::optimization::Opt::Unbox) {
                let mut drops: Vec<_> = ctx
                    .ownership
                    .as_ref()
                    .into_iter()
                    .flat_map(|ownership| ownership.owned_drops.iter().copied())
                    .filter(|sym| {
                        !ctx.fixed_stack_seqs.contains_key(sym)
                            && !ctx.text_bytes_views.contains_key(sym)
                            && matches!(
                                ctx.vars.get(sym),
                                Some(CType::SeqI64(_) | CType::SeqBool | CType::SeqF64)
                            )
                    })
                    .collect();
                drops.sort_by_key(|sym| ctx.resolve(*sym));
                for sym in drops {
                    writeln!(
                        output,
                        "{pad}lc_gc_free((void *){}.data); /* unique local drop */",
                        ctx.resolve(sym)
                    )
                    .unwrap();
                    crate::optimize::mark_fired(crate::optimization::Opt::Unbox);
                }
            }
            if let Some(val) = value {
                let val_str = codegen_expr(val, ctx);
                // A builder's text leaves as a copy of its own.
                let val_str =
                    if infer_expr_type(val, ctx) == CType::String && shares_builder(val, ctx) {
                        format!("lc_gc_strdup({val_str})")
                    } else {
                        val_str
                    };
                let val_str = as_returned(val, val_str, ctx);
                writeln!(output, "{}return {};", pad, val_str).unwrap();
            } else {
                writeln!(output, "{}return;", pad).unwrap();
            }
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            // BranchlessAccumulate is profitable only when its add can be plain. The shared body
            // proof admits exactly a zero-based `+1` accumulator changed in one i64-bounded loop;
            // every other shape keeps the ordinary checked conditional add.
            if else_block.is_none()
                && then_block.len() == 1
                && crate::optimize::active_config()
                    .is_on(crate::optimization::Opt::BranchlessAccumulate)
            {
                if let Some(Stmt::Set {
                    target,
                    value:
                        Expr::BinaryOp {
                            op: BinaryOpKind::Add,
                            left,
                            right,
                        },
                }) = then_block.first()
                {
                    let unit_self_add = matches!(left, Expr::Identifier(sym) if *sym == *target)
                        && matches!(right, Expr::Literal(Literal::Number(1)));
                    let proven = ctx
                        .ownership
                        .as_ref()
                        .is_some_and(|own| own.bounded_count_accumulators.contains(target));
                    if unit_self_add && proven && ctx.vars.get(target) == Some(&CType::Int64) {
                        crate::optimize::mark_fired(crate::optimization::Opt::BranchlessAccumulate);
                        let name = ctx.resolve(*target);
                        let truth = c_truthy(cond, ctx);
                        writeln!(output, "{pad}{name} += (int64_t)!!({truth});").unwrap();
                        return;
                    }
                }
            }
            let cond_str = c_truthy(cond, ctx);
            writeln!(output, "{}if ({}) {{", pad, cond_str).unwrap();
            ctx.enter_scope();
            for s in *then_block {
                codegen_stmt(s, ctx, output, indent + 1);
            }
            ctx.exit_scope();
            if let Some(eb) = else_block {
                writeln!(output, "{}}} else {{", pad).unwrap();
                ctx.enter_scope();
                for s in *eb {
                    codegen_stmt(s, ctx, output, indent + 1);
                }
                ctx.exit_scope();
            }
            writeln!(output, "{}}}", pad).unwrap();
        }
        Stmt::While { cond, body, .. } => {
            if try_emit_c_byte_compare_window(stmt, ctx, output, indent) {
                return;
            }
            if try_emit_c_tiled_while(stmt, ctx, output, indent) {
                return;
            }
            if try_emit_c_native_descending_while(stmt, ctx, output, indent) {
                return;
            }
            if try_emit_c_native_counted_while(stmt, ctx, output, indent) {
                return;
            }
            if let Some(reuse) = detect_c_buffer_reuse(body, ctx) {
                crate::optimize::mark_fired(crate::optimization::Opt::Peephole);
                let inner_pad = format!("{pad}    ");
                writeln!(output, "{pad}{{").unwrap();
                ctx.enter_scope();
                let inner = ctx.declare(reuse.inner, reuse.ty.clone());
                let outer = ctx.resolve(reuse.outer).to_string();
                let head = ctx.decl_head(&reuse.ty, &inner);
                let new_fn = seq_fn(&reuse.ty, "new", ctx.interner)
                    .expect("buffer reuse is only detected for a sequence type");
                writeln!(output, "{inner_pad}{head} = {new_fn}();").unwrap();

                reserve_counted_fills(stmt, cond, body, ctx, output, &inner_pad, None);
                let cond_str = c_truthy(cond, ctx);
                writeln!(output, "{inner_pad}while ({cond_str}) {{").unwrap();
                ctx.enter_scope();
                ctx.in_loop(|ctx| {
                    for (idx, child) in body.iter().enumerate() {
                        if idx == reuse.inner_let {
                            writeln!(output, "{inner_pad}    {inner}.len = 0;").unwrap();
                        } else if idx == reuse.move_set {
                            let ty = c_type_str_resolved(&reuse.ty, ctx.interner);
                            writeln!(
                                output,
                                "{inner_pad}    {{ {ty} __buffer_swap = {outer}; {outer} = {inner}; {inner} = __buffer_swap; }}"
                            )
                            .unwrap();
                        } else {
                            codegen_stmt(child, ctx, output, indent + 2);
                        }
                    }
                });
                ctx.exit_scope();
                writeln!(output, "{inner_pad}}}").unwrap();
                ctx.exit_scope();
                writeln!(output, "{pad}}}").unwrap();
                return;
            }
            reserve_counted_fills(stmt, cond, body, ctx, output, &pad, None);
            let cond_str = c_truthy(cond, ctx);
            writeln!(output, "{}while ({}) {{", pad, cond_str).unwrap();
            ctx.enter_scope();
            ctx.in_loop(|ctx| {
                for s in *body {
                    codegen_stmt(s, ctx, output, indent + 1);
                }
            });
            ctx.exit_scope();
            writeln!(output, "{}}}", pad).unwrap();
        }
        // `Break` leaves the innermost loop or zone; outside every one it is a value-less `Return`.
        Stmt::Break => match ctx.flow.last() {
            Some(super::types::Flow::Loop) => writeln!(output, "{pad}break;").unwrap(),
            Some(super::types::Flow::Zone { exit, taken }) => {
                taken.set(true);
                writeln!(output, "{pad}goto {exit};").unwrap();
            }
            None => codegen_stmt(&Stmt::Return { value: None }, ctx, output, indent),
        },
        // A zone runs its body in a block of its own, its name bound to nothing; a `Break` directly in it or a
        // `Return` anywhere in it leaves it.
        Stmt::Zone { name, body, .. } => {
            let inner = format!("{pad}    ");
            ctx.in_zone(output, &pad, |ctx, output| {
                writeln!(output, "{pad}{{").unwrap();
                ctx.enter_scope();
                bind_nothing(*name, ctx, output, &inner);
                for s in *body {
                    codegen_stmt(s, ctx, output, indent + 1);
                }
                ctx.exit_scope();
                writeln!(output, "{pad}}}").unwrap();
            });
        }
        // Each task runs in order, its bindings kept, a zone of its own: its `Break` or `Return` ends it alone.
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            for task in tasks.iter() {
                ctx.in_zone(output, &pad, |ctx, output| {
                    codegen_stmt(task, ctx, output, indent)
                });
            }
        }
        Stmt::Push { collection, value } if infer_expr_type(collection, ctx) == CType::Tower => {
            writeln!(
                output,
                "{pad}ltower_push({}, {});",
                codegen_expr(collection, ctx),
                super::tower::value(value, ctx)
            )
            .unwrap();
        }
        Stmt::Push { collection, value } => {
            let val_str = stored(value, codegen_expr(value, ctx), ctx);
            if operand_place(collection, ctx) {
                let operand = collection_operand(
                    collection,
                    || {
                        use crate::interpreter::RuntimeValue;
                        crate::semantics::collections::list_push(&RuntimeValue::Nothing, RuntimeValue::Nothing)
                    },
                    ctx,
                );
                // A bignum-promoted value flowing into an int64-element `Seq` must be narrowed back to i64
                // (see `narrow_i64_sink`) — otherwise `Push (a / b % c)` emits `seq_i64_push(&s, <LInt>)`.
                let i64_val = narrow_i64_sink(value, val_str.clone(), ctx);
                let (push, pushed) = match operand.ty.as_ref() {
                    Some(CType::SeqI64(_)) => ("seq_i64_push".to_string(), i64_val),
                    Some(CType::SeqBool) => ("seq_bool_push".to_string(), val_str),
                    Some(CType::SeqStr) => ("seq_str_push".to_string(), val_str),
                    Some(CType::SeqF64) => ("seq_f64_push".to_string(), val_str),
                    Some(CType::SeqStruct(s)) => (seq_struct_fn(*s, "push", ctx.interner), val_str),
                    Some(CType::SeqValue(inner)) if matches!(**inner, CType::Opt(_)) => {
                        ("seq_logos_opt_push".to_string(), opt_item(value, inner, ctx))
                    }
                    Some(CType::SeqValue(inner)) => (format!("seq_{}_push", c_type_str(inner)), val_str),
                    // Pushing an inner sequence into a `Seq of Seq of …` → the `seq_Seq_<inner>_push`
                    // family (the value is itself a Seq, so it rides `val_str`, not the i64-narrowed form).
                    Some(CType::SeqSeq(inner)) => (
                        format!("seq_{}_push", c_type_str_resolved(inner, ctx.interner)),
                        val_str,
                    ),
                    _ => ("seq_i64_push".to_string(), i64_val),
                };
                let coll = &operand.place;
                // The VM computes the value before it finds the list holds nothing.
                let act = match operand.absent {
                    None => format!("{push}(&{coll}, {pushed});"),
                    Some(_) => format!(
                        "{{ __auto_type __pushed = {pushed}; {} }}",
                        operand.act(format!("{push}(&{coll}, __pushed);"))
                    ),
                };
                writeln!(output, "{pad}{act}").unwrap();
            } else if matches!(collection, Expr::FieldAccess { .. }) {
                // Push to a struct field that is itself a Seq (`Push v to b's items`): the field lvalue is
                // `b.items`, and its element type comes from the field's inferred `CType`.
                let coll = codegen_expr(collection, ctx);
                let i64_val = narrow_i64_sink(value, val_str.clone(), ctx);
                match infer_expr_type(collection, ctx) {
                    CType::SeqStr => {
                        writeln!(output, "{}seq_str_push(&{}, {});", pad, coll, val_str).unwrap()
                    }
                    CType::SeqF64 => {
                        writeln!(output, "{}seq_f64_push(&{}, {});", pad, coll, val_str).unwrap()
                    }
                    CType::SeqBool => {
                        writeln!(output, "{}seq_bool_push(&{}, {});", pad, coll, val_str).unwrap()
                    }
                    CType::SeqStruct(s) => writeln!(
                        output,
                        "{}{}(&{}, {});",
                        pad,
                        seq_struct_fn(s, "push", ctx.interner),
                        coll,
                        val_str
                    )
                    .unwrap(),
                    CType::SeqValue(inner) => writeln!(
                        output,
                        "{}seq_{}_push(&{}, {});",
                        pad,
                        c_type_str(&inner),
                        coll,
                        val_str
                    )
                    .unwrap(),
                    _ => writeln!(output, "{}seq_i64_push(&{}, {});", pad, coll, i64_val).unwrap(),
                }
            }
        }
        // `Pop from xs into v` takes the last element, or binds nothing when there is none; without `into` it only
        // drops the last element, if any.
        Stmt::Pop { collection, into } if infer_expr_type(collection, ctx) == CType::Tower => {
            let popped = format!("ltower_pop({})", codegen_expr(collection, ctx));
            match into {
                Some(var) => emit_binding(*var, CType::Tower, &popped, ctx, output, &pad),
                None => writeln!(output, "{pad}(void){popped};").unwrap(),
            }
        }
        Stmt::Pop { collection, into } => {
            let (place, ty) = match collection {
                _ if operand_place(collection, ctx) => {
                    let operand = collection_operand(
                        collection,
                        || {
                            crate::semantics::collections::list_pop(&crate::interpreter::RuntimeValue::Nothing)
                                .map(|_| ())
                        },
                        ctx,
                    );
                    if let Some(check) = &operand.absent {
                        writeln!(output, "{pad}{check}").unwrap();
                    }
                    let ty = operand.ty.unwrap_or_else(|| collection_type(collection, ctx));
                    (operand.place, ty)
                }
                Expr::FieldAccess { .. } => (
                    codegen_expr(collection, ctx),
                    infer_expr_type(collection, ctx),
                ),
                _ => {
                    output.push_str(&c_refuse_expr(
                        "`Pop from` a list that is neither a variable nor a field",
                    ));
                    return;
                }
            };
            let Some(element) = super::types::seq_element(&ty, ctx.registry) else {
                output.push_str(&c_refuse_expr(&format!(
                    "`Pop from` a {ty:?}, which is not a list"
                )));
                return;
            };
            match into {
                // A popped item that may be nothing is the value-or-nothing it is, with no second layer.
                Some(var) if matches!(element, CType::Opt(_)) => {
                    let popped = format!(
                        "(({place}).len > 0 ? ({place}).data[--({place}).len] : (logos_opt){{0, 0}})"
                    );
                    emit_binding(*var, element, &popped, ctx, output, &pad);
                }
                Some(var) => {
                    let n = ctx.select_ctr.get();
                    ctx.select_ctr.set(n + 1);
                    let taken =
                        opt_pack(&element, &format!("({place}).data[--({place}).len]"), ctx);
                    let popped = format!(
                        "({{ logos_opt __popped{n} = {{0, 0}}; if (({place}).len > 0) {{ __popped{n}.present = 1; __popped{n}.value = {taken}; }} __popped{n}; }})"
                    );
                    emit_binding(
                        *var,
                        CType::Opt(Box::new(element)),
                        &popped,
                        ctx,
                        output,
                        &pad,
                    );
                }
                None => writeln!(output, "{pad}if (({place}).len > 0) ({place}).len--;").unwrap(),
            }
        }
        // A call made for its effect is the call expression, its value dropped.
        Stmt::Call { function, args } => {
            let call = Expr::Call {
                function: *function,
                args: args.clone(),
            };
            writeln!(output, "{pad}{};", codegen_expr(&call, ctx)).unwrap();
        }
        Stmt::SetIndex {
            collection,
            index,
            value,
        } if infer_expr_type(collection, ctx) == CType::Tower => {
            writeln!(
                output,
                "{pad}ltower_set_item({}, {}, {});",
                codegen_expr(collection, ctx),
                super::tower::value(index, ctx),
                super::tower::value(value, ctx)
            )
            .unwrap();
        }
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            // A named collection, or a place reached through fields (`b's items`): either is an lvalue. A
            // value-or-nothing variable is the collection it holds, found to hold nothing after the position and
            // the value are computed, as the VM finds it.
            let mut absent = None;
            let (coll_name, coll_ty) = match collection {
                _ if operand_place(collection, ctx) => {
                    let operand = collection_operand(
                        collection,
                        || {
                            use crate::interpreter::RuntimeValue;
                            crate::semantics::collections::index_set(
                                &RuntimeValue::Nothing,
                                &RuntimeValue::Int(1),
                                RuntimeValue::Nothing,
                            )
                        },
                        ctx,
                    );
                    absent = operand.absent;
                    (operand.place, operand.ty)
                }
                Expr::FieldAccess { .. } => (
                    codegen_expr(collection, ctx),
                    Some(infer_expr_type(collection, ctx)),
                ),
                // An item of a list is a list held in the outer list's buffer.
                Expr::Index { .. } if seq_item_place(collection, ctx).is_some() => {
                    let (place, ty) = seq_item_place(collection, ctx).expect("checked above");
                    (place, Some(ty))
                }
                _ => {
                    output.push_str(&c_refuse_expr(
                        "`Set item … of` a collection that is neither a variable nor a field",
                    ));
                    return;
                }
            };
            // The position, then the value, as the VM computes them.
            let cells = sequence_operands(&[*index, *value], ctx, output, &pad);
            let idx_str = match &coll_ty {
                Some(
                    CType::MapI64I64(_, _)
                    | CType::MapStrI64(_)
                    | CType::MapStrStr
                    | CType::MapI64Str(_)
                    | CType::MapStrValue(_)
                    | CType::MapI64Value(_),
                ) => narrow_i64_sink(index, codegen_expr(index, ctx), ctx),
                _ => narrow_index(
                    index,
                    codegen_expr(index, ctx),
                    ctx,
                    crate::semantics::failcode::FailCode::ListStoreIndexTooWide,
                ),
            };
            let val_str = match &coll_ty {
                // A map of the extension's values holds a cell of its own for each.
                Some(CType::MapStrValue(inner) | CType::MapI64Value(inner)) if **inner == CType::Tower => {
                    super::tower::stored(value, ctx)
                }
                Some(CType::SeqValue(inner) | CType::MapStrValue(inner) | CType::MapI64Value(inner))
                    if matches!(**inner, CType::Opt(_)) =>
                {
                    opt_item(value, inner, ctx)
                }
                _ => stored(
                    value,
                    narrow_i64_sink(value, codegen_expr(value, ctx), ctx),
                    ctx,
                ),
            };
            let proven_direct = crate::optimize::active_config()
                .is_on(crate::optimization::Opt::Unchecked)
                && crate::optimize::facts::holds_expr(
                    crate::optimize::facts::Fact::InBounds,
                    index,
                )
                && matches!(
                    &coll_ty,
                    Some(
                        CType::SeqI64(_)
                            | CType::SeqBool
                            | CType::SeqStr
                            | CType::SeqF64
                            | CType::SeqStruct(_)
                            | CType::SeqTuple(_)
                            | CType::SeqSeq(_)
                            | CType::SeqValue(_)
                    )
                );
            if let Some(check) = absent {
                writeln!(output, "{pad}{check}").unwrap();
            }
            if proven_direct {
                crate::optimize::mark_fired(crate::optimization::Opt::Unchecked);
                writeln!(
                    output,
                    "{pad}({coll_name}).data[(size_t)(({idx_str}) - 1LL)] = {val_str};"
                )
                .unwrap();
                forget_operands(cells, ctx);
                return;
            }
            let setter = match &coll_ty {
                Some(CType::MapI64I64(_, _)) => "map_i64_i64_set".to_string(),
                Some(CType::MapStrI64(_)) => "map_str_i64_set".to_string(),
                Some(CType::MapStrStr) => "map_str_str_set".to_string(),
                Some(CType::MapI64Str(_)) => "map_i64_str_set".to_string(),
                Some(CType::MapStrValue(inner)) => {
                    format!("map_str_{}_set", c_type_str_resolved(inner, ctx.interner))
                }
                Some(CType::MapI64Value(inner)) => {
                    format!("map_i64_{}_set", c_type_str_resolved(inner, ctx.interner))
                }
                Some(seq) => {
                    seq_fn(seq, "set", ctx.interner).unwrap_or_else(|| "seq_i64_set".to_string())
                }
                None => "seq_i64_set".to_string(),
            };
            writeln!(
                output,
                "{}{}(&{}, {}, {});",
                pad, setter, coll_name, idx_str, val_str
            )
            .unwrap();
            forget_operands(cells, ctx);
        }
        Stmt::Repeat {
            pattern,
            iterable,
            body,
        } if infer_expr_type(iterable, ctx) == CType::Tower => {
            // The items are taken once, before the first pass, as the VM takes them; each pass binds the
            // pattern afresh in a block of its own.
            let items = ctx.temp("__items", CType::Tower);
            let count = ctx.temp("__count", CType::Int64);
            let at = ctx.temp("__at", CType::Int64);
            writeln!(output, "{pad}{{").unwrap();
            writeln!(
                output,
                "{pad}    {} = ltower_items({});",
                ctx.decl_head(&CType::Tower, &items),
                codegen_expr(iterable, ctx)
            )
            .unwrap();
            writeln!(
                output,
                "{pad}    {} = ltower_length({items});",
                ctx.decl_head(&CType::Int64, &count)
            )
            .unwrap();
            writeln!(
                output,
                "{pad}    for ({} = 1; {at} <= {count}; {at}++) {{",
                ctx.decl_head(&CType::Int64, &at)
            )
            .unwrap();
            ctx.enter_scope();
            let inner = format!("{pad}        ");
            let item = format!("ltower_item_at({items}, {at})");
            match pattern {
                Pattern::Identifier(sym) => emit_binding(*sym, CType::Tower, &item, ctx, output, &inner),
                Pattern::Tuple(syms) => {
                    let whole = ctx.temp("__item", CType::Tower);
                    writeln!(output, "{inner}{} = {item};", ctx.decl_head(&CType::Tower, &whole)).unwrap();
                    for (k, sym) in syms.iter().enumerate() {
                        let part = format!("ltower_item_at({whole}, {})", k + 1);
                        emit_binding(*sym, CType::Tower, &part, ctx, output, &inner);
                    }
                }
            }
            writeln!(output, "{inner}{{").unwrap();
            ctx.enter_scope();
            ctx.in_loop(|ctx| {
                for s in *body {
                    codegen_stmt(s, ctx, output, indent + 3);
                }
            });
            ctx.exit_scope();
            writeln!(output, "{inner}}}").unwrap();
            ctx.exit_scope();
            writeln!(output, "{pad}    }}").unwrap();
            writeln!(output, "{pad}}}").unwrap();
        }
        Stmt::Repeat {
            pattern,
            iterable,
            body,
        } => {
            if let Expr::Identifier(view_sym) = iterable {
                if ctx.text_bytes_views.contains_key(view_sym) {
                    let var_sym = match pattern {
                        Pattern::Identifier(sym) => *sym,
                        Pattern::Tuple(syms) => {
                            if let Some(sym) = syms.first() {
                                *sym
                            } else {
                                return;
                            }
                        }
                    };
                    let view = ctx.temp("__tbv", CType::String);
                    let at = ctx.temp("__tbi", CType::Int64);
                    let len = ctx.temp("__tbn", CType::Int64);
                    writeln!(
                        output,
                        "{pad}const unsigned char *{view} = (const unsigned char *){};",
                        ctx.resolve(*view_sym)
                    )
                    .unwrap();
                    writeln!(
                        output,
                        "{pad}for (size_t {at} = 0, {len} = strlen((const char *){view}); {at} < {len}; {at}++) {{"
                    )
                    .unwrap();
                    ctx.enter_scope();
                    match pattern {
                        Pattern::Tuple(syms) => bind_pattern_failing(
                            syms,
                            "Expected tuple for pattern, got Int",
                            ctx,
                            output,
                            &format!("{pad}    "),
                        ),
                        Pattern::Identifier(_) => {
                            let var_name = ctx.declare(var_sym, CType::Int64);
                            writeln!(
                                output,
                                "{pad}    {} = (int64_t){view}[{at}];",
                                ctx.decl_head(&CType::Int64, &var_name)
                            )
                            .unwrap();
                        }
                    }
                    ctx.in_loop(|ctx| {
                        for stmt in *body {
                            codegen_stmt(stmt, ctx, output, indent + 1);
                        }
                    });
                    ctx.exit_scope();
                    writeln!(output, "{pad}}}").unwrap();
                    return;
                }
            }

            // Check for Range-based iteration (optimized)
            if let Expr::Range { start, end } = iterable {
                let var_sym = match pattern {
                    Pattern::Identifier(sym) => *sym,
                    Pattern::Tuple(syms) => {
                        if let Some(s) = syms.first() {
                            *s
                        } else {
                            return;
                        }
                    }
                };
                // The range is read once, before the variable is bound, into temporaries of the loop's own
                // block. A hidden count walks it and stops AT the end, never past it, so a range ending at the
                // largest Int ends. Each pass binds the variable to the count afresh, and the body is a block of
                // its own: a body that sets the variable, or binds its name anew, changes that pass only.
                // The counter is a machine word: an exact bound narrows into it (`lint_expect_i64` aborts on a
                // bound past `i64`, a range no program can walk).
                let start_str = range_bound(start, ctx);
                let end_str = range_bound(end, ctx);
                let lo = ctx.temp("__lo", CType::Int64);
                let hi = ctx.temp("__hi", CType::Int64);
                writeln!(output, "{pad}{{").unwrap();
                writeln!(
                    output,
                    "{pad}    {} = {start_str};",
                    ctx.decl_head(&CType::Int64, &lo)
                )
                .unwrap();
                writeln!(
                    output,
                    "{pad}    {} = {end_str};",
                    ctx.decl_head(&CType::Int64, &hi)
                )
                .unwrap();
                let count = ctx.temp("__logos_count", CType::Int64);
                writeln!(
                    output,
                    "{pad}    if ({lo} <= {hi}) for ({} = {lo}; ; {count}++) {{",
                    ctx.decl_head(&CType::Int64, &count)
                )
                .unwrap();
                ctx.enter_scope();
                let var_name = ctx.declare(var_sym, CType::Int64);
                writeln!(
                    output,
                    "{pad}        {} = {count};",
                    ctx.decl_head(&CType::Int64, &var_name)
                )
                .unwrap();
                writeln!(output, "{pad}        {{").unwrap();
                ctx.enter_scope();
                ctx.in_loop(|ctx| {
                    for s in *body {
                        codegen_stmt(s, ctx, output, indent + 3);
                    }
                });
                ctx.exit_scope();
                ctx.exit_scope();
                writeln!(output, "{pad}        }}").unwrap();
                writeln!(output, "{pad}        if ({count} == {hi}) break;").unwrap();
                writeln!(output, "{pad}    }}").unwrap();
                writeln!(output, "{pad}}}").unwrap();
                return;
            }

            // A value-or-nothing variable is the collection it holds, failing in the VM's words when it holds none.
            let operand = match iterable {
                _ if opt_place(iterable, ctx) => {
                    Some(collection_operand(
                        iterable,
                        || {
                            crate::semantics::collections::iteration_snapshot(
                                &crate::interpreter::RuntimeValue::Nothing,
                            )
                            .map(|_| ())
                        },
                        ctx,
                    ))
                }
                _ => None,
            };
            let coll_type = match &operand {
                Some(CollectionOperand { ty: Some(ty), .. }) => ty.clone(),
                _ => infer_expr_type(iterable, ctx),
            };
            // The collection is read once, before the first pass — as a copy of its own when the body changes it,
            // since the passes walk the collection as it was when the loop began.
            let changed_in_body = {
                let changed = super::sharing::changed_in(
                    body,
                    &ctx.changed_statics,
                    &ctx.mutating,
                    &ctx.call_shapes,
                    &ctx.fn_inout,
                );
                match ctx.ownership.as_deref() {
                    Some(own) => own.sharing.closure(&changed),
                    None => changed,
                }
            };
            let iterated = match &operand {
                Some(operand) => operand.read(operand.place.clone()),
                None => codegen_expr(iterable, ctx),
            };
            let iterated = if super::values::binding_needs_copy(
                iterable,
                &coll_type,
                None,
                &changed_in_body,
                ctx,
            ) {
                super::values::copy_value(&coll_type, &iterated, ctx)
            } else {
                iterated
            };
            let iter_str = ctx.temp("__it", coll_type.clone());
            writeln!(
                output,
                "{pad}{} = {iterated};",
                ctx.decl_head(&coll_type, &iter_str)
            )
            .unwrap();

            // Map iteration — direct bucket scan
            let map_types = super::types::map_entry_types(&coll_type);

            if let Some((key_type, val_type)) = map_types {
                // In insertion order, as the interpreter's map iterates: the `order` array `Show` walks too.
                let oi = ctx.temp("__oi", CType::Int64);
                let mi = ctx.temp("__mi", CType::Int64);
                let oi_head = ctx.decl_head(&CType::Int64, &oi);
                let mi_head = ctx.decl_head(&CType::Int64, &mi);
                writeln!(
                    output,
                    "{pad}for ({oi_head} = 0; {oi} < (int64_t){iter_str}.ord_len; {oi}++) {{"
                )
                .unwrap();
                writeln!(
                    output,
                    "{pad}    {mi_head} = (int64_t){iter_str}.order[{oi}];"
                )
                .unwrap();
                writeln!(output, "{pad}    if (!{iter_str}.state[{mi}]) continue;").unwrap();

                ctx.enter_scope();
                match pattern {
                    Pattern::Tuple(syms) if syms.len() != 2 => {
                        let failure = format!("Cannot bind a 2-tuple to {} names", syms.len());
                        bind_pattern_failing(syms, &failure, ctx, output, &format!("{pad}    "));
                    }
                    Pattern::Tuple(syms) => {
                        let value = format!("{iter_str}.vals[{mi}]");
                        let value =
                            own_binding(&val_type, value, changed_in_body.contains(&syms[1]), ctx);
                        let k_name = ctx.declare(syms[0], key_type.clone());
                        let v_name = ctx.declare(syms[1], val_type.clone());
                        writeln!(
                            output,
                            "{}    {} = {}.keys[{}];",
                            pad,
                            ctx.decl_head(&key_type, &k_name),
                            iter_str,
                            mi
                        )
                        .unwrap();
                        writeln!(
                            output,
                            "{}    {} = {value};",
                            pad,
                            ctx.decl_head(&val_type, &v_name)
                        )
                        .unwrap();
                    }
                    // One name binds the whole entry, `(key, value)`, as on the interpreter.
                    _ => {
                        let var_sym = match pattern {
                            Pattern::Identifier(sym) => *sym,
                            Pattern::Tuple(syms) => {
                                if let Some(s) = syms.first() {
                                    *s
                                } else {
                                    return;
                                }
                            }
                        };
                        let entry = CType::Tuple(vec![key_type.clone(), val_type.clone()]);
                        let entry_c = c_type_str_resolved(&entry, ctx.interner);
                        let var_name = ctx.declare(var_sym, entry.clone());
                        let head = ctx.decl_head(&entry, &var_name);
                        writeln!(
                            output,
                            "{pad}    {head} = ({entry_c}){{ {iter_str}.keys[{mi}], {iter_str}.vals[{mi}] }};"
                        )
                        .unwrap();
                    }
                }

                ctx.in_loop(|ctx| {
                    for s in *body {
                        codegen_stmt(s, ctx, output, indent + 1);
                    }
                });
                ctx.exit_scope();
                writeln!(output, "{}}}", pad).unwrap();
            } else if let Some(elem_type) = super::types::set_element(&coll_type) {
                // A set iterates its elements in insertion order, as the interpreter's does: the `order` array
                // its `Show` walks.
                let var_sym = match pattern {
                    Pattern::Identifier(sym) => *sym,
                    Pattern::Tuple(syms) => match syms.first() {
                        Some(sym) => *sym,
                        None => return,
                    },
                };
                let oi = ctx.temp("__oi", CType::Int64);
                let si = ctx.temp("__si", CType::Int64);
                let oi_head = ctx.decl_head(&CType::Int64, &oi);
                let si_head = ctx.decl_head(&CType::Int64, &si);
                writeln!(
                    output,
                    "{pad}for ({oi_head} = 0; {oi} < (int64_t){iter_str}.ord_len; {oi}++) {{"
                )
                .unwrap();
                writeln!(output, "{pad}    {si_head} = (int64_t){iter_str}.order[{oi}];").unwrap();
                writeln!(output, "{pad}    if (!{iter_str}.state[{si}]) continue;").unwrap();
                ctx.enter_scope();
                let var_name = ctx.declare(var_sym, elem_type.clone());
                writeln!(
                    output,
                    "{pad}    {} = {iter_str}.keys[{si}];",
                    ctx.decl_head(&elem_type, &var_name)
                )
                .unwrap();
                ctx.in_loop(|ctx| {
                    for s in *body {
                        codegen_stmt(s, ctx, output, indent + 1);
                    }
                });
                ctx.exit_scope();
                writeln!(output, "{}}}", pad).unwrap();
            } else if let (CType::SeqTuple(elems), Pattern::Tuple(syms)) = (&coll_type, pattern) {
                // A Seq of tuples destructured positionally (`Repeat for (a, b) in pairs`): iterate the
                // `seq_<LTuple>` family and bind each pattern variable to the element's positional field
                // (`f0..fN`), at that field's own C type (so a `(Int, Text)` element binds `a`/`b` correctly).
                let tn = tuple_c_name(elems, ctx.interner);
                let idx = ctx.temp("__idx", CType::Int64);
                let tup = ctx.temp("__tup", CType::Tuple(elems.clone()));
                writeln!(
                    output,
                    "{pad}for ({} = 1; {idx} <= seq_{tn}_len(&{iter_str}); {idx}++) {{",
                    ctx.decl_head(&CType::Int64, &idx)
                )
                .unwrap();
                writeln!(
                    output,
                    "{pad}    {} = seq_{tn}_get(&{iter_str}, {idx});",
                    ctx.decl_head(&CType::Tuple(elems.clone()), &tup)
                )
                .unwrap();
                ctx.enter_scope();
                if syms.len() == elems.len() {
                    for (i, sym) in syms.iter().enumerate() {
                        let field_ty = elems[i].clone();
                        let part = format!("{tup}.f{i}");
                        let part = own_binding(&field_ty, part, changed_in_body.contains(sym), ctx);
                        let name = ctx.declare(*sym, field_ty.clone());
                        writeln!(
                            output,
                            "{pad}    {} = {part};",
                            ctx.decl_head(&field_ty, &name)
                        )
                        .unwrap();
                    }
                } else {
                    let failure = format!(
                        "Cannot bind a {}-tuple to {} names",
                        elems.len(),
                        syms.len()
                    );
                    bind_pattern_failing(syms, &failure, ctx, output, &format!("{pad}    "));
                }
                ctx.in_loop(|ctx| {
                    for s in *body {
                        codegen_stmt(s, ctx, output, indent + 1);
                    }
                });
                ctx.exit_scope();
                writeln!(output, "{}}}", pad).unwrap();
            } else if coll_type == CType::String {
                // A Text is iterated character by character, each a Text of its own.
                let var_sym = match pattern {
                    Pattern::Identifier(sym) => *sym,
                    Pattern::Tuple(syms) => {
                        if let Some(s) = syms.first() {
                            *s
                        } else {
                            return;
                        }
                    }
                };
                let at = ctx.temp("__at", CType::String);
                let width = ctx.temp("__width", CType::Int64);
                writeln!(output, "{pad}{} = 0;", ctx.decl_head(&CType::Int64, &width)).unwrap();
                writeln!(
                    output,
                    "{pad}for ({} = {iter_str}; *{at}; {at} += {width}) {{",
                    ctx.decl_head(&CType::String, &at)
                )
                .unwrap();
                writeln!(
                    output,
                    "{pad}    {width} = ((unsigned char)*{at} < 0x80) ? 1 : (((unsigned char)*{at} >> 5) == 6) ? 2 : (((unsigned char)*{at} >> 4) == 14) ? 3 : 4;"
                )
                .unwrap();
                ctx.enter_scope();
                match pattern {
                    Pattern::Tuple(syms) => bind_pattern_failing(
                        syms,
                        "Expected tuple for pattern, got Text",
                        ctx,
                        output,
                        &format!("{pad}    "),
                    ),
                    Pattern::Identifier(_) => {
                        let var_name = ctx.declare(var_sym, CType::String);
                        writeln!(
                            output,
                            "{pad}    {} = lc_gc_strndup({at}, (size_t){width});",
                            ctx.decl_head(&CType::String, &var_name)
                        )
                        .unwrap();
                    }
                }
                ctx.in_loop(|ctx| {
                    for s in *body {
                        codegen_stmt(s, ctx, output, indent + 1);
                    }
                });
                ctx.exit_scope();
                writeln!(output, "{}}}", pad).unwrap();
            } else {
                // Seq iteration
                let var_sym = match pattern {
                    Pattern::Identifier(sym) => *sym,
                    Pattern::Tuple(syms) => {
                        if let Some(s) = syms.first() {
                            *s
                        } else {
                            return;
                        }
                    }
                };
                let (len_fn, get_fn, elem_type): (String, String, CType) = match &coll_type {
                    CType::SeqBool => ("seq_bool_len".into(), "seq_bool_get".into(), CType::Bool),
                    CType::SeqStr => ("seq_str_len".into(), "seq_str_get".into(), CType::String),
                    CType::SeqF64 => ("seq_f64_len".into(), "seq_f64_get".into(), CType::Float64),
                    // Iterate a Seq of a value type — the loop var binds the element (Money/Quantity/…).
                    CType::SeqValue(inner) => {
                        let e = c_type_str(inner);
                        (
                            format!("seq_{e}_len"),
                            format!("seq_{e}_get"),
                            (**inner).clone(),
                        )
                    }
                    // Iterate a nested `Seq of Seq of …` — the loop var binds an INNER sequence
                    // (`Seq_i64`/`Seq_str`) via the `seq_Seq_<inner>_*` family, not the flat default.
                    CType::SeqSeq(inner) => {
                        let e = c_type_str_resolved(inner, ctx.interner);
                        (
                            format!("seq_{e}_len"),
                            format!("seq_{e}_get"),
                            (**inner).clone(),
                        )
                    }
                    // Iterate a `Seq of <Struct>` / `Seq of <Enum>` — the loop var binds each value.
                    CType::SeqStruct(sym) => (
                        seq_struct_fn(*sym, "len", ctx.interner),
                        seq_struct_fn(*sym, "get", ctx.interner),
                        crate::codegen_c::types::user_type_ctype(*sym, ctx.registry),
                    ),
                    // Iterate a `Seq of (…)` into ONE name — the loop var binds the whole tuple.
                    CType::SeqTuple(elems) => {
                        let tn = tuple_c_name(elems, ctx.interner);
                        (
                            format!("seq_{tn}_len"),
                            format!("seq_{tn}_get"),
                            CType::Tuple(elems.clone()),
                        )
                    }
                    // Iterate a `Seq of Int` or of a machine word — the loop var binds each as it reads.
                    CType::SeqI64(ints) => ("seq_i64_len".into(), "seq_i64_get".into(), ints.ctype()),
                    _ => ("seq_i64_len".into(), "seq_i64_get".into(), CType::Int64),
                };
                let idx = ctx.temp("__idx", CType::Int64);
                writeln!(
                    output,
                    "{pad}for ({} = 1; {idx} <= {len_fn}(&{iter_str}); {idx}++) {{",
                    ctx.decl_head(&CType::Int64, &idx)
                )
                .unwrap();
                ctx.enter_scope();
                match pattern {
                    // An element that is not a tuple cannot be taken apart: the pass fails, as on the VM.
                    Pattern::Tuple(syms) => {
                        let failure = format!(
                            "Expected tuple for pattern, got {}",
                            vm_type_name(&elem_type, ctx)
                        );
                        bind_pattern_failing(syms, &failure, ctx, output, &format!("{pad}    "));
                    }
                    Pattern::Identifier(_) => {
                        let element = format!("{get_fn}(&{iter_str}, {idx})");
                        let element = own_binding(
                            &elem_type,
                            element,
                            changed_in_body.contains(&var_sym),
                            ctx,
                        );
                        let var_name = ctx.declare(var_sym, elem_type.clone());
                        writeln!(
                            output,
                            "{pad}    {} = {element};",
                            ctx.decl_head(&elem_type, &var_name)
                        )
                        .unwrap();
                    }
                }
                ctx.in_loop(|ctx| {
                    for s in *body {
                        codegen_stmt(s, ctx, output, indent + 1);
                    }
                });
                ctx.exit_scope();
                writeln!(output, "{}}}", pad).unwrap();
            }
        }
        // `Add v to s` and `Remove v from c` change the set or map a place holds: a variable or a record's field.
        // An integer the program made exact goes in as the word it is; any other collection fails in the words
        // the VM's own `Add` and `Remove` give it.
        Stmt::Add { value, collection } | Stmt::Remove { value, collection } => {
            let adding = matches!(stmt, Stmt::Add { .. });
            // A value-or-nothing variable is the collection it holds, found to hold nothing once the member is
            // computed, as the VM finds it.
            let operand = match collection {
                _ if operand_place(collection, ctx) => Some(collection_operand(
                    collection,
                    || {
                        use crate::interpreter::RuntimeValue as V;
                        if adding {
                            crate::semantics::collections::set_add(&V::Nothing, V::Nothing)
                        } else {
                            crate::semantics::collections::remove_from(&V::Nothing, &V::Nothing)
                        }
                    },
                    ctx,
                )),
                _ => None,
            };
            let absent = operand.as_ref().and_then(|o| o.absent.clone());
            let v = match &absent {
                None => codegen_expr(value, ctx),
                Some(_) => "__member".to_string(),
            };
            let found = match operand {
                Some(CollectionOperand { place, ty: Some(ty), .. }) => Some((place, ty)),
                _ => changed_place_of(collection, ctx),
            };
            let Some((place, ty)) = found else {
                output.push_str(&c_refuse_expr(
                    "`Add`/`Remove` on a collection that is neither a variable nor a field",
                ));
                return;
            };
            let word = narrow_i64_sink(value, v.clone(), ctx);
            let call = match (adding, &ty) {
                (true, CType::SetI64(_)) => Some(format!("set_i64_add(&{place}, {word})")),
                (true, CType::SetStr) => Some(format!("set_str_add(&{place}, {v})")),
                (false, CType::SetI64(_)) => Some(format!("set_i64_remove(&{place}, {word})")),
                (false, CType::SetStr) => Some(format!("set_str_remove(&{place}, {v})")),
                (false, CType::MapI64I64(_, _)) => Some(format!("map_i64_i64_remove(&{place}, {word})")),
                (false, CType::MapI64Str(_)) => Some(format!("map_i64_str_remove(&{place}, {word})")),
                (false, CType::MapStrI64(_)) => Some(format!("map_str_i64_remove(&{place}, {v})")),
                (false, CType::MapStrStr) => Some(format!("map_str_str_remove(&{place}, {v})")),
                (false, CType::MapStrValue(inner)) => Some(format!(
                    "map_str_{}_remove(&{place}, {v})",
                    c_type_str_resolved(inner, ctx.interner)
                )),
                (false, CType::MapI64Value(inner)) => Some(format!(
                    "map_i64_{}_remove(&{place}, {word})",
                    c_type_str_resolved(inner, ctx.interner)
                )),
                _ => None,
            };
            match (call, absent) {
                (Some(call), None) => writeln!(output, "{pad}{call};").unwrap(),
                (Some(call), Some(check)) => writeln!(
                    output,
                    "{pad}{{ __auto_type __member = {}; {check} {call}; }}",
                    codegen_expr(value, ctx)
                )
                .unwrap(),
                (None, _) => {
                    use crate::interpreter::RuntimeValue as V;
                    let sample = V::Int(0);
                    let refusal = if adding {
                        crate::semantics::collections::set_add(&V::Int(0), sample)
                    } else {
                        crate::semantics::collections::remove_from(&V::Int(0), &sample)
                    }
                    .err()
                    .unwrap_or_default();
                    writeln!(output, "{pad}(void)({v});").unwrap();
                    writeln!(
                        output,
                        "{pad}{}",
                        ctx.fail_stmt(&c_string_literal(&refusal))
                    )
                    .unwrap();
                }
            }
        }
        Stmt::SetField {
            object,
            field,
            value,
        } => {
            let obj = codegen_expr(object, ctx);
            let fname = escape_c_ident(ctx.interner.resolve(*field));
            let val = codegen_expr(value, ctx);
            let val = match infer_expr_type(object, ctx) {
                CType::Struct(sym) => match ctx.registry.get(sym) {
                    Some(crate::analysis::TypeDef::Struct { fields, .. }) => as_field_value(
                        val,
                        value,
                        sym,
                        fields.iter().find(|f| f.name == *field),
                        ctx,
                    ),
                    _ => val,
                },
                _ => val,
            };
            writeln!(output, "{}{}.{} = {};", pad, obj, fname, val).unwrap();
        }
        // ── Single-replica CRDT counters (`ConvergentCount`/`Tally`) — the field cell holds a
        //    `logos_crdt*` handle; `Increase`/`Decrease` bump THIS replica's own slot in place, so the
        //    mutation is visible through the struct just like `Set … field`. Matches the VM/tree-walker
        //    (`CrdtValue::bump`) + the RISC-V tier (`logos_crdt_bump`). ──
        Stmt::IncreaseCrdt {
            object,
            field,
            amount,
        } => {
            emit_crdt_bump_c(object, *field, amount, false, ctx, output, &pad);
        }
        Stmt::DecreaseCrdt {
            object,
            field,
            amount,
        } => {
            emit_crdt_bump_c(object, *field, amount, true, ctx, output, &pad);
        }
        // `Merge source into target` — the field-wise lattice join. A counter field takes the per-replica
        // MAX (idempotent — a redelivered state never double-counts); other fields keep the VM's plain-
        // field ADD. Both operands share the struct type, so iterate its counter fields.
        Stmt::MergeCrdt { source, target } => {
            let src = codegen_expr(source, ctx);
            let tgt = codegen_expr(target, ctx);
            if let CType::Struct(sym) = infer_expr_type(target, ctx) {
                if let Some(crate::analysis::TypeDef::Struct { fields, .. }) = ctx.registry.get(sym)
                {
                    for f in fields {
                        let fname = escape_c_ident(ctx.interner.resolve(f.name));
                        if ctx.is_counter_field(sym, f.name) {
                            writeln!(
                                output,
                                "{}logos_crdt_merge((logos_crdt *)(intptr_t)({}.{}), (logos_crdt *)(intptr_t)({}.{}));",
                                pad, tgt, fname, src, fname
                            )
                            .unwrap();
                        } else {
                            // Plain field: the VM's `crdt_merge_field` ADDs.
                            writeln!(
                                output,
                                "{}{}.{} = {}.{} + {}.{};",
                                pad, tgt, fname, tgt, fname, src, fname
                            )
                            .unwrap();
                        }
                    }
                }
            }
        }
        // `Converge source into target` — the IDEMPOTENT lattice join, the sibling of `Merge`. A counter
        // field takes the same per-replica MAX (`logos_crdt_merge`); a plain field takes the per-field MAX
        // (not ADD), so applying a redelivered state twice equals applying it once. Matches `crdt_join_field`.
        Stmt::JoinCrdt { source, target } => {
            let src = codegen_expr(source, ctx);
            let tgt = codegen_expr(target, ctx);
            if let CType::Struct(sym) = infer_expr_type(target, ctx) {
                if let Some(crate::analysis::TypeDef::Struct { fields, .. }) = ctx.registry.get(sym)
                {
                    for f in fields {
                        let fname = escape_c_ident(ctx.interner.resolve(f.name));
                        if ctx.is_counter_field(sym, f.name) {
                            writeln!(
                                output,
                                "{}logos_crdt_merge((logos_crdt *)(intptr_t)({}.{}), (logos_crdt *)(intptr_t)({}.{}));",
                                pad, tgt, fname, src, fname
                            )
                            .unwrap();
                        } else {
                            // Plain field: the VM's `crdt_join_field` takes the per-field MAX.
                            writeln!(
                                output,
                                "{}{}.{} = {}.{} > {}.{} ? {}.{} : {}.{};",
                                pad, tgt, fname, tgt, fname, src, fname, tgt, fname, src, fname
                            )
                            .unwrap();
                        }
                    }
                }
            }
        }
        Stmt::Inspect {
            target,
            arms,
            has_otherwise,
        } => {
            let target_type = infer_expr_type(target, ctx);
            // The value inspected is computed once, before any arm is tried.
            let target_str = match target {
                Expr::Identifier(_) => codegen_expr(target, ctx),
                _ => {
                    let value = codegen_expr(target, ctx);
                    let cell = ctx.temp("__inspected", target_type.clone());
                    writeln!(
                        output,
                        "{pad}{} = {value};",
                        ctx.decl_head(&target_type, &cell)
                    )
                    .unwrap();
                    cell
                }
            };
            let enum_sym = if let CType::Enum(sym) = target_type {
                Some(sym)
            } else {
                None
            };

            let mut first = true;
            for arm in arms {
                if let Some(variant_sym) = arm.variant {
                    let ename = arm
                        .enum_name
                        .map(|s| escape_c_ident(ctx.interner.resolve(s)))
                        .or_else(|| enum_sym.map(|s| escape_c_ident(ctx.interner.resolve(s))))
                        .unwrap_or_else(|| "Unknown".to_string());
                    let vname = escape_c_ident(ctx.interner.resolve(variant_sym));

                    if first {
                        writeln!(
                            output,
                            "{}if ({}.tag == {}_{}) {{",
                            pad, target_str, ename, vname
                        )
                        .unwrap();
                        first = false;
                    } else {
                        writeln!(
                            output,
                            "{}}} else if ({}.tag == {}_{}) {{",
                            pad, target_str, ename, vname
                        )
                        .unwrap();
                    }

                    // Extract bindings from variant fields, each the arm's own.
                    ctx.enter_scope();
                    for (field_name, binding_name) in &arm.bindings {
                        let fname = escape_c_ident(ctx.interner.resolve(*field_name));
                        // Infer the field type from the registry, detecting recursive fields
                        let (field_ctype, is_recursive_field) = if let Some(esym) = enum_sym {
                            if let Some(crate::analysis::TypeDef::Enum { variants, .. }) =
                                ctx.registry.get(esym)
                            {
                                variants.iter()
                                    .find(|v| v.name == variant_sym)
                                    .and_then(|v| v.fields.iter().find(|f| f.name == *field_name))
                                    .map(|f| {
                                        let is_self = matches!(&f.ty, crate::analysis::FieldType::Named(fsym) if *fsym == esym);
                                        let ctype = super::exact::field_ctype(variant_sym, f, ctx.interner, ctx.registry, &ctx.field_facts());
                                        (ctype, is_self)
                                    })
                                    .unwrap_or((CType::Int64, false))
                            } else {
                                (CType::Int64, false)
                            }
                        } else {
                            (CType::Int64, false)
                        };
                        // A recursive field is a pointer — dereference to get a value copy.
                        let deref = if is_recursive_field { "*" } else { "" };
                        let taken = format!("{deref}{target_str}.data.{vname}.{fname}");
                        // A binding the arm changes in place is the arm's own copy.
                        let changed_in_arm = super::values::mutated_roots(arm.body, &ctx.fn_inout)
                            .contains(binding_name);
                        let taken = own_binding(&field_ctype, taken, changed_in_arm, ctx);
                        let bname = ctx.declare(*binding_name, field_ctype.clone());
                        let head = ctx.decl_head(&field_ctype, &bname);
                        writeln!(output, "{pad}    {head} = {taken};").unwrap();
                    }
                } else {
                    // Otherwise arm
                    if first {
                        writeln!(output, "{}{{", pad).unwrap();
                        first = false;
                    } else {
                        writeln!(output, "{}}} else {{", pad).unwrap();
                    }
                    ctx.enter_scope();
                }

                for s in arm.body {
                    codegen_stmt(s, ctx, output, indent + 1);
                }
                ctx.exit_scope();
            }
            if !first {
                if *has_otherwise {
                    writeln!(output, "{}}}", pad).unwrap();
                } else {
                    // A variant no arm names, with no `Otherwise`, fails the run in the words the VM's `Inspect`
                    // gives it. The last such variant takes the plain `else`: every tag is then accounted for.
                    let uncovered: Vec<Symbol> = match enum_sym.and_then(|e| ctx.registry.get(e)) {
                        Some(crate::analysis::TypeDef::Enum { variants, .. }) => variants
                            .iter()
                            .map(|v| v.name)
                            .filter(|v| !arms.iter().any(|arm| arm.variant == Some(*v)))
                            .collect(),
                        _ => Vec::new(),
                    };
                    let enum_shown = enum_sym.map(|e| ctx.display_name(e)).unwrap_or_default();
                    let ename = arms
                        .iter()
                        .find_map(|arm| arm.enum_name)
                        .or(enum_sym)
                        .map(|e| escape_c_ident(ctx.interner.resolve(e)))
                        .unwrap_or_default();
                    for (i, v) in uncovered.iter().enumerate() {
                        let words = crate::semantics::builtins::inspect_unhandled_message(
                            &enum_shown,
                            ctx.interner.resolve(*v),
                        );
                        let fail = ctx.fail_stmt(&c_string_literal(&words));
                        if i + 1 == uncovered.len() {
                            writeln!(output, "{pad}}} else {{ {fail} }}").unwrap();
                        } else {
                            let vname = escape_c_ident(ctx.interner.resolve(*v));
                            writeln!(
                                output,
                                "{pad}}} else if ({target_str}.tag == {ename}_{vname}) {{ {fail}"
                            )
                            .unwrap();
                        }
                    }
                    if uncovered.is_empty() {
                        writeln!(output, "{pad}}}").unwrap();
                    }
                }
            }
        }
        // ── Deterministic concurrency (the cooperative `ucontext` scheduler; gated on the whole program
        //    using concurrency, so a non-concurrency program never reaches these arms). A pipe lives as an
        //    `int64_t` handle (`CType::Pipe` of its element), cast to `logos_chan*` at each use; a value
        //    crosses it in its element's scalar packing (a Real by its bits, a Text by its pointer). ──
        Stmt::CreatePipe {
            var,
            capacity,
            element_type,
        } => {
            let element = resolve_type_expr_with_registry(
                &TypeExpr::Named(*element_type),
                ctx.interner,
                Some(ctx.registry),
            );
            if task_arg_pack(&element, "0").is_none() {
                output.push_str(&c_refuse_expr(&format!(
                    "a pipe of {}, which a pipe's scalar cell cannot carry",
                    ctx.interner.resolve(*element_type)
                )));
                return;
            }
            // `Pipe of Int` with no explicit capacity is UNBOUNDED (`cap = -1`), matching the VM
            // (`capacity.map(...).unwrap_or(-1)`); an explicit capacity is bounded.
            let cap: i64 = capacity.map(|c| c as i64).unwrap_or(-1);
            let handle = format!("(int64_t)(intptr_t)logos_chan_new({})", cap);
            emit_binding(
                *var,
                CType::Pipe(Box::new(element)),
                &handle,
                ctx,
                output,
                &pad,
            );
        }
        Stmt::SendPipe { value, pipe } => {
            let packed = pipe_packed_value(value, pipe, ctx);
            let chan = codegen_expr(pipe, ctx);
            writeln!(
                output,
                "{}logos_chan_send((logos_chan *)(intptr_t)({}), {});",
                pad, chan, packed
            )
            .unwrap();
        }
        // `Receive x from ch` — park until a value arrives; binds `x` to what it read, value-or-Nothing of
        // the pipe's element: a closed pipe, once drained, reads `nothing`, as it does on the VM.
        Stmt::ReceivePipe { var, pipe } => {
            let chan = codegen_expr(pipe, ctx);
            let read = CType::Opt(Box::new(pipe_element(pipe, ctx)));
            emit_binding(
                *var,
                read,
                &format!("logos_chan_recv((logos_chan *)(intptr_t)({}))", chan),
                ctx,
                output,
                &pad,
            );
        }
        // `Try to send v to ch into ok` — non-blocking; `logos_chan_trysend` returns the success flag
        // (matching the VM's `dst = true/false`).
        Stmt::TrySendPipe {
            value,
            pipe,
            result,
        } => {
            let packed = pipe_packed_value(value, pipe, ctx);
            let chan = codegen_expr(pipe, ctx);
            let call = format!(
                "logos_chan_trysend((logos_chan *)(intptr_t)({}), {})",
                chan, packed
            );
            match result {
                Some(res) => emit_binding(
                    *res,
                    CType::Bool,
                    &format!("{} != 0", call),
                    ctx,
                    output,
                    &pad,
                ),
                None => writeln!(output, "{}(void){};", pad, call).unwrap(),
            }
        }
        // `Try to receive x from ch` — non-blocking; binds `x` to a value-OR-Nothing of the pipe's element
        // (`CType::Opt`, the runtime's `logos_opt`), `nothing` when none is buffered.
        Stmt::TryReceivePipe { var, pipe } => {
            let chan = codegen_expr(pipe, ctx);
            let read = CType::Opt(Box::new(pipe_element(pipe, ctx)));
            emit_binding(
                *var,
                read,
                &format!("logos_chan_tryrecv((logos_chan *)(intptr_t)({}))", chan),
                ctx,
                output,
                &pad,
            );
        }
        Stmt::LaunchTask { function, args } => {
            emit_spawn_c(*function, args, None, ctx, output, &pad);
        }
        Stmt::LaunchTaskWithHandle {
            handle,
            function,
            args,
        } => {
            emit_spawn_c(*function, args, Some(*handle), ctx, output, &pad);
        }
        // `Stop h` — abort a spawned task (Op::TaskAbort). The handle var holds the `logos_task*` cast to
        // int64 (see `emit_spawn_c`); `logos_task_abort` marks it aborted (unless already done) and yields so
        // the scheduler drops it — its post-`Receive` code never runs and a later `Send` to its channel does
        // NOT wake it. Mirrors the RISC-V lowering (`logos_task_abort`); byte-for-byte the VM's
        // `VmBlock::Abort` lazy-teardown. Without this the statement no-op'd → a `Stop`-then-`Send` wrongly
        // resumed the "aborted" task in C (a real C≠VM divergence this fixes).
        Stmt::StopTask { handle } => {
            let h = codegen_expr(handle, ctx);
            writeln!(
                output,
                "{}logos_task_abort((logos_task *)(intptr_t)({}));",
                pad, h
            )
            .unwrap();
        }
        // `Close ch` — mark the channel closed (Op::ChanClose). Wakes any parked receivers with the
        // closed signal and yields (`logos_chan_close`); buffered values stay receivable, a later send is
        // the error. The deterministic-scheduler channel sibling of `Stop`. Byte-for-byte the VM's
        // `VmBlock::Close`.
        Stmt::CloseChannel { channel } => {
            let ch = codegen_expr(channel, ctx);
            writeln!(
                output,
                "{}logos_chan_close((logos_chan *)(intptr_t)({}));",
                pad, ch
            )
            .unwrap();
        }
        // `Await h` / `Wait for h` — park until the task has finished (Op::TaskAwait). The handle var holds
        // the `logos_task*` cast to int64 (see `emit_spawn_c`). A task answers `nothing`, as it does on the
        // interpreter and the VM, so `… into r` binds `r` to the absent `logos_opt`.
        Stmt::AwaitTask { handle, result } => {
            let h = codegen_expr(handle, ctx);
            writeln!(
                output,
                "{}logos_task_await((logos_task *)(intptr_t)({}));",
                pad, h
            )
            .unwrap();
            if let Some(r) = result {
                emit_binding(
                    *r,
                    CType::Opt(Box::new(CType::Int64)),
                    "(logos_opt){0, 0}",
                    ctx,
                    output,
                    &pad,
                );
            }
        }
        // `Sleep n` — park on the scheduler's logical clock: an Int counts ticks, a Duration nanoseconds,
        // as the tick a `After n:` select arm waits on.
        Stmt::Sleep { milliseconds } => {
            let ticks = select_timeout_ticks_c(milliseconds, ctx);
            writeln!(output, "{}logos_sleep((long long)({}));", pad, ticks).unwrap();
        }
        Stmt::Select { branches } => {
            // Register each arm in declaration order (arm index 0,1,2… — the convention the runtime's
            // `logos_select_wait` returns and the VM/tree-walker oracle dispatch on). A recv arm mints a
            // unique `logos_opt` slot the runtime writes what it read into — a value, or `nothing` from a
            // closed pipe; a timeout arm passes the tick count `select_timeout_ticks_c` derives EXACTLY as
            // the VM's `eval_select_timeout_ticks` (`After N seconds` is a `Duration` of `N·10⁹` nanos →
            // that many virtual ticks). The winning arm's body then runs; a recv body first binds its
            // variable from the slot, value-or-nothing, as `Receive … from` binds.
            writeln!(output, "{}logos_select_reset();", pad).unwrap();
            let mut slots: Vec<Option<String>> = Vec::with_capacity(branches.len());
            for branch in branches.iter() {
                match branch {
                    SelectBranch::Receive { pipe, .. } => {
                        let chan = codegen_expr(pipe, ctx);
                        let n = ctx.select_ctr.get();
                        ctx.select_ctr.set(n + 1);
                        let slot = format!("__selslot_{}", n);
                        writeln!(output, "{}logos_opt {};", pad, slot).unwrap();
                        writeln!(
                            output,
                            "{}logos_select_arm_recv((logos_chan *)(intptr_t)({}), &{});",
                            pad, chan, slot
                        )
                        .unwrap();
                        slots.push(Some(slot));
                    }
                    SelectBranch::Timeout { milliseconds, .. } => {
                        let ticks = select_timeout_ticks_c(milliseconds, ctx);
                        writeln!(
                            output,
                            "{}logos_select_arm_timeout((long long)({}));",
                            pad, ticks
                        )
                        .unwrap();
                        slots.push(None);
                    }
                }
            }
            writeln!(output, "{}long long __arm = logos_select_wait();", pad).unwrap();
            for (i, branch) in branches.iter().enumerate() {
                let kw = if i == 0 { "if" } else { "} else if" };
                writeln!(output, "{}{} (__arm == {}) {{", pad, kw, i).unwrap();
                ctx.enter_scope();
                let body = match branch {
                    SelectBranch::Receive { var, pipe, body } => {
                        let slot = slots[i].as_ref().expect("recv arm has a slot");
                        let read = CType::Opt(Box::new(pipe_element(pipe, ctx)));
                        emit_binding(*var, read, slot, ctx, output, &format!("{}    ", pad));
                        *body
                    }
                    SelectBranch::Timeout { body, .. } => *body,
                };
                for s in body {
                    codegen_stmt(s, ctx, output, indent + 1);
                }
                ctx.exit_scope();
            }
            if !branches.is_empty() {
                writeln!(output, "{}}}", pad).unwrap();
            }
        }
        Stmt::FunctionDef { .. } => {}
        // `Append v to <obj>'s <field>` (SharedSequence RGA). On the single replica this is an in-place
        // push to the struct's `Seq` field — the element's push fn is chosen from the field's Seq type.
        Stmt::AppendToSequence { sequence, value } => {
            let seq_str = codegen_expr(sequence, ctx);
            let val_str = codegen_expr(value, ctx);
            let push_fn = match infer_expr_type(sequence, ctx) {
                CType::SeqBool => "seq_bool_push".to_string(),
                CType::SeqStr => "seq_str_push".to_string(),
                CType::SeqF64 => "seq_f64_push".to_string(),
                CType::SeqStruct(s) => seq_struct_fn(s, "push", ctx.interner),
                CType::SeqValue(inner) => format!("seq_{}_push", c_type_str(&inner)),
                CType::SeqTuple(elems) => {
                    format!("seq_{}_push", tuple_c_name(&elems, ctx.interner))
                }
                _ => "seq_i64_push".to_string(),
            };
            writeln!(output, "{}{}(&{}, {});", pad, push_fn, seq_str, val_str).unwrap();
        }
        // `Resolve <obj>'s <field> to v` (MVRegister LWW). On the single replica the last write wins, so it
        // is a plain field assignment — exactly like `SetField`.
        Stmt::ResolveConflict {
            object,
            field,
            value,
        } => {
            let obj = codegen_expr(object, ctx);
            let fname = escape_c_ident(ctx.interner.resolve(*field));
            let val = codegen_expr(value, ctx);
            writeln!(output, "{}{}.{} = {};", pad, obj, fname, val).unwrap();
        }
        // `Check that <subject> is <predicate>` / `... can <action> the <object>` lowers to an inline
        // guard: `if (!(<condition>)) { <fail> }`. The condition is the policy's predicate/capability
        // condition emitted over the subject (and object) C structs — byte-identical semantics to
        // `semantics::policy::check_policy`, the tree-walker/VM oracle. An unlowerable form emits a LOUD
        // `#error` (never a silent no-op, which would be a C≠VM miscompile).
        Stmt::Check {
            subject,
            predicate,
            is_capability,
            object,
            source_text,
            ..
        } => {
            emit_policy_check(
                ctx,
                output,
                &pad,
                *subject,
                *predicate,
                *is_capability,
                *object,
                source_text,
            );
        }
        // ── Offline networking (the loopback transport, see `C_NET_RUNTIME`) ──────────────────────────────
        // `Connect`/`Listen` dial nothing (no relay); the offline single node just registers the address.
        // `Let peer be a PeerAgent at "addr"` binds the address string as the peer handle (MakePeer = identity).
        // `Sync` is offline-identity. `Send`/`Stream` enqueue a 64-bit message (a scalar, or a `Seq` pointer for
        // a stream) into the destination's FIFO; `Await` dequeues it. Byte-for-byte the VM's offline model. The
        // one-time-pad (`secure`) is ignored: on a single node encrypt-then-decrypt is identity, so the observed
        // message is unchanged (≡VM for the loopback).
        Stmt::ConnectTo { address, .. } => {
            let a = codegen_expr(address, ctx);
            writeln!(
                output,
                "{}logos_net_connect((const char *)(intptr_t)({}));",
                pad, a
            )
            .unwrap();
        }
        Stmt::Listen { address, .. } => {
            let a = codegen_expr(address, ctx);
            writeln!(
                output,
                "{}logos_net_listen((const char *)(intptr_t)({}));",
                pad, a
            )
            .unwrap();
        }
        Stmt::LetPeerAgent { var, address } => {
            let a = codegen_expr(address, ctx);
            let peer = format!(
                "(char *)(intptr_t)logos_net_makepeer((long long)(intptr_t)({}))",
                a
            );
            emit_binding(*var, CType::String, &peer, ctx, output, &pad);
        }
        Stmt::Sync { .. } => {
            // Offline-identity: no peer state to merge.
            writeln!(output, "{}/* offline Sync — no-op */", pad).unwrap();
        }
        Stmt::SendMessage {
            message,
            destination,
            ..
        } => {
            let m = codegen_expr(message, ctx);
            let d = codegen_expr(destination, ctx);
            writeln!(
                output,
                "{}logos_net_send((const char *)(intptr_t)({}), (long long)({}));",
                pad, d, m
            )
            .unwrap();
        }
        Stmt::StreamMessage {
            values,
            destination,
        } => {
            // A stream is a `Seq` — send its ADDRESS as the 64-bit message; the loopback round-trips the same
            // object (an `Await stream` reads it back). `Seq of Int` on this path (a device-clean `Seq_i64`).
            let vals = codegen_expr(values, ctx);
            let d = codegen_expr(destination, ctx);
            writeln!(
                output,
                "{}logos_net_send((const char *)(intptr_t)({}), (long long)(intptr_t)(&({})));",
                pad, d, vals
            )
            .unwrap();
        }
        Stmt::AwaitMessage {
            source,
            into,
            stream,
            ..
        } => {
            let s = codegen_expr(source, ctx);
            if *stream {
                // Stream receive: the message is the streamed `Seq`'s pointer; deref to a local copy of the batch.
                let batch = format!(
                    "*(Seq_i64 *)(intptr_t)logos_net_await((const char *)(intptr_t)({}))",
                    s
                );
                emit_binding(*into, CType::SeqI64(Ints::Int), &batch, ctx, output, &pad);
            } else {
                // Scalar receive: the dequeued 64-bit message is the sent Int.
                let message = format!("logos_net_await((const char *)(intptr_t)({}))", s);
                emit_binding(*into, CType::Int64, &message, ctx, output, &pad);
            }
        }
        Stmt::RuntimeAssert {
            condition,
            hard,
            dim: _,
            reason,
        } => {
            let cond_str = c_truthy(condition, ctx);
            let report = match reason {
                None => ctx.fail_stmt("\"Assertion failed\""),
                Some(r) => c_failure_report(r, ctx, true),
            };
            // The soft form mirrors AOT-Rust's `debug_assert!`: enforced unless NDEBUG.
            if !*hard {
                writeln!(output, "{}#ifndef NDEBUG", pad).unwrap();
            }
            writeln!(output, "{}if (!({})) {{ {} }}", pad, cond_str, report).unwrap();
            if !*hard {
                writeln!(output, "{}#endif", pad).unwrap();
            }
        }
        Stmt::Fail { reason } => {
            writeln!(
                output,
                "{}{{ {} }}",
                pad,
                c_failure_report(reason, ctx, false)
            )
            .unwrap();
        }
        // `Read x from console` reads empty Text, as on the interpreter; a file's whole text otherwise.
        Stmt::ReadFrom { var, source } => {
            let read = match source {
                ReadSource::Console => "lc_gc_strdup(\"\")".to_string(),
                ReadSource::File(path) => {
                    format!("logos_file_read({})", c_format_builtin(path, ctx))
                }
            };
            emit_binding(*var, CType::String, &read, ctx, output, &pad);
        }
        // The content is rendered, then the path, in the order the VM evaluates them.
        Stmt::WriteFile { content, path } => {
            let text = ctx.temp("__content", CType::String);
            writeln!(
                output,
                "{pad}{} = {};",
                ctx.decl_head(&CType::String, &text),
                c_format_builtin(content, ctx)
            )
            .unwrap();
            writeln!(
                output,
                "{pad}logos_file_write({}, {text});",
                c_format_builtin(path, ctx)
            )
            .unwrap();
        }
        Stmt::Mount { var, path } => {
            let mounted = format!("logos_file_mount({})", c_format_builtin(path, ctx));
            emit_binding(*var, CType::String, &mounted, ctx, output, &pad);
        }
        // Agents do not run: the spawned agent's name is bound to nothing.
        Stmt::Spawn { name, .. } => bind_nothing(*name, ctx, output, &pad),
        // An `Escape` holds Rust source, which runs on the Rust backends only: here it fails as it does on the VM.
        Stmt::Escape { .. } => {
            let failure = "Escape blocks contain raw Rust code and cannot be interpreted. \
                           Use `largo build` or `largo run` to compile and run this program.";
            writeln!(output, "{pad}{}", ctx.fail_stmt(&c_string_literal(failure))).unwrap();
        }
        // Declarations, proofs, dependencies and observation: nothing happens when the program runs. A `## Test`'s
        // statements and a page's never reach a program's body.
        Stmt::Assert { .. }
        | Stmt::Trust { .. }
        | Stmt::Require { .. }
        | Stmt::Theorem(_)
        | Stmt::Definition(_)
        | Stmt::Axiom(_)
        | Stmt::Theory(_)
        | Stmt::StructDef { .. }
        | Stmt::TestDef { .. }
        | Stmt::ComponentDef(_)
        | Stmt::DirectiveDef { .. }
        | Stmt::RouteDef { .. }
        | Stmt::MiddlewareDef { .. }
        | Stmt::RedirectDef { .. }
        | Stmt::Site(_)
        | Stmt::Observe { .. }
        | Stmt::Expect { .. }
        | Stmt::ExpectFail { .. }
        | Stmt::ExpectOutput { .. }
        | Stmt::AdvanceTime { .. }
        | Stmt::GivenFile { .. }
        | Stmt::ForAll { .. }
        | Stmt::Fetch { .. }
        | Stmt::Ui(_)
        | Stmt::GivenFetch { .. }
        | Stmt::ServeFetches
        | Stmt::Replay { .. } => {}
        Stmt::Serve { .. } => output.push_str(&c_refuse_expr("`Serve on port`")),
    }
}

/// Escape a Rust string for embedding in a C string literal — the same rules the `Literal::Text` emitter
/// uses (newlines/tabs/backslash/quote), so a policy message or literal renders identically.
fn escape_c_string(raw: &str) -> String {
    raw.chars()
        .map(|c| match c {
            '\n' => "\\n".to_string(),
            '\r' => "\\r".to_string(),
            '\t' => "\\t".to_string(),
            '\\' => "\\\\".to_string(),
            '"' => "\\\"".to_string(),
            other if (other as u32) < 0x20 || other == '\u{7f}' => {
                format!("\\{:03o}", other as u32)
            }
            other => other.to_string(),
        })
        .collect()
}

/// The C type of field `field` of the record `struct_sym`, as its storage lays it out.
fn c_struct_field_ctype(ctx: &CContext, struct_sym: Symbol, field: Symbol) -> Option<CType> {
    if let Some(crate::analysis::TypeDef::Struct { fields, .. }) = ctx.registry.get(struct_sym) {
        fields.iter().find(|f| f.name == field).map(|f| {
            super::exact::field_ctype(struct_sym, f, ctx.interner, ctx.registry, &ctx.field_facts())
        })
    } else {
        None
    }
}

/// A value the VM holds of the C type `ty` for a `Check`: a record of its type holding no field, or a
/// sample of any other kind.
fn policy_sample(ty: &CType, ctx: &CContext) -> Option<crate::interpreter::RuntimeValue> {
    match ty {
        CType::Struct(sym) => Some(crate::interpreter::RuntimeValue::Struct(Box::new(
            crate::interpreter::StructValue {
                type_name: ctx.display_name(*sym),
                fields: std::collections::HashMap::new(),
            },
        ))),
        other => sample_of(other, 1),
    }
}

/// Lower `Check that <subject> is <predicate>` / `Check that <subject> can <action> <object>` as the VM runs it
/// (`semantics::policy::check_policy`): the subject is read, then the object; a subject that is not a record,
/// or a type with no such predicate or capability, fails outright; otherwise the condition is evaluated over
/// the two records and a false one fails. Every failure's words are the ones `check_policy` gives.
fn emit_policy_check(
    ctx: &CContext,
    output: &mut String,
    pad: &str,
    subject: Symbol,
    predicate: Symbol,
    is_capability: bool,
    object: Option<Symbol>,
    source_text: &str,
) {
    let object = if is_capability { object } else { None };
    for name in std::iter::once(subject).chain(object) {
        if !ctx.vars.contains_key(&name) && !ctx.program_statics.contains(&name) {
            let failure = format!("Undefined variable: {}", ctx.interner.resolve(name));
            writeln!(
                output,
                "{pad}{}",
                ctx.fail_stmt(&c_string_literal(&failure))
            )
            .unwrap();
            return;
        }
    }
    let subject_ty = ctx.vars.get(&subject).cloned().unwrap_or(CType::Int64);
    let object_ty = object.map(|o| ctx.vars.get(&o).cloned().unwrap_or(CType::Int64));
    let (Some(subject_sample), object_sample) = (
        policy_sample(&subject_ty, ctx),
        object_ty.as_ref().map(|t| policy_sample(t, ctx)),
    ) else {
        output.push_str(&c_refuse_expr(&format!("a `Check` of a {subject_ty:?}")));
        return;
    };
    let Some(object_sample) = object_sample.map_or(Some(None), |s| s.map(Some)) else {
        output.push_str(&c_refuse_expr(&format!(
            "a `Check` against a {object_ty:?}"
        )));
        return;
    };
    let verdict = crate::semantics::policy::check_policy(
        ctx.policies,
        ctx.interner,
        &subject_sample,
        predicate,
        is_capability,
        object_sample.as_ref(),
        source_text,
    );
    let Err(failure) = verdict else {
        output.push_str(&c_refuse_expr(
            "a `Check` its policy passes for a record holding no field",
        ));
        return;
    };
    let fail = ctx.fail_stmt(&c_string_literal(&failure));
    let CType::Struct(subject_type) = subject_ty else {
        writeln!(output, "{pad}{fail}").unwrap();
        return;
    };
    let condition = if is_capability {
        ctx.policies
            .get_capabilities(subject_type)
            .and_then(|caps| caps.iter().find(|c| c.action == predicate))
            .map(|c| &c.condition)
    } else {
        ctx.policies
            .get_predicates(subject_type)
            .and_then(|preds| preds.iter().find(|p| p.predicate_name == predicate))
            .map(|p| &p.condition)
    };
    let Some(condition) = condition else {
        writeln!(output, "{pad}{fail}").unwrap();
        return;
    };
    let object = object.zip(object_ty).and_then(|(o, ty)| match ty {
        CType::Struct(sym) => Some((ctx.resolve(o), sym)),
        _ => None,
    });
    let holds = emit_policy_condition(
        ctx,
        condition,
        &ctx.resolve(subject),
        subject_type,
        object.as_ref(),
        0,
    );
    writeln!(output, "{pad}if (!({holds})) {{ {fail} }}").unwrap();
}

/// A policy condition as a C truth value over the subject record `subject` of type `subject_type` and, for a
/// capability, the object record: `semantics::policy::evaluate_policy_condition`, form by form. A field the
/// record lacks, or of a kind the form does not compare, makes the form false, as there.
fn emit_policy_condition(
    ctx: &CContext,
    cond: &crate::analysis::policy::PolicyCondition,
    subject: &str,
    subject_type: Symbol,
    object: Option<&(String, Symbol)>,
    depth: usize,
) -> String {
    use crate::analysis::policy::PolicyCondition as PC;
    match cond {
        PC::FieldEquals {
            field,
            value,
            is_string_literal,
        } => {
            let fname = escape_c_ident(ctx.interner.resolve(*field));
            let expected = ctx.interner.resolve(*value);
            match c_struct_field_ctype(ctx, subject_type, *field) {
                Some(CType::String) => format!(
                    "(strcmp(({subject}).{fname}, {}) == 0)",
                    c_string_literal(expected)
                ),
                Some(CType::Int64) if !*is_string_literal => match expected.parse::<i64>() {
                    Ok(e) => format!("(({subject}).{fname} == {e}LL)"),
                    Err(_) => "0".to_string(),
                },
                Some(CType::BigInt) if !*is_string_literal => match expected.parse::<i64>() {
                    Ok(e) => format!("(lint_cmp(({subject}).{fname}, lint_of({e}LL)) == 0)"),
                    Err(_) => "0".to_string(),
                },
                Some(CType::Bool) if !*is_string_literal => match expected.parse::<bool>() {
                    Ok(e) => format!("(({subject}).{fname} == {e})"),
                    Err(_) => "0".to_string(),
                },
                _ => "0".to_string(),
            }
        }
        PC::FieldBool { field, value } => match c_struct_field_ctype(ctx, subject_type, *field) {
            Some(CType::Bool) => {
                format!(
                    "(({subject}).{} == {value})",
                    escape_c_ident(ctx.interner.resolve(*field))
                )
            }
            _ => "0".to_string(),
        },
        PC::Predicate { predicate, .. } => {
            let inner = ctx
                .policies
                .get_predicates(subject_type)
                .and_then(|preds| preds.iter().find(|p| p.predicate_name == *predicate))
                .map(|p| &p.condition);
            match inner {
                Some(inner) if depth < 64 => {
                    emit_policy_condition(ctx, inner, subject, subject_type, object, depth + 1)
                }
                Some(_) => c_refuse_expr("a policy predicate that names itself"),
                None => "0".to_string(),
            }
        }
        PC::ObjectFieldEquals {
            subject: subject_field,
            field: object_field,
            ..
        }
        | PC::SubjectFieldEqualsObjectField {
            subject_field,
            object_field,
            ..
        } => {
            let Some((object, object_type)) = object else {
                return "0".to_string();
            };
            let (a, b) = (
                c_struct_field_ctype(ctx, subject_type, *subject_field),
                c_struct_field_ctype(ctx, *object_type, *object_field),
            );
            let (Some(a), Some(b)) = (a, b) else {
                return "0".to_string();
            };
            let left = format!(
                "({subject}).{}",
                escape_c_ident(ctx.interner.resolve(*subject_field))
            );
            let right = format!(
                "({object}).{}",
                escape_c_ident(ctx.interner.resolve(*object_field))
            );
            match (&a, &b) {
                (CType::String, CType::String) => format!("(strcmp({left}, {right}) == 0)"),
                (CType::BigInt, CType::BigInt) => format!("(lint_cmp({left}, {right}) == 0)"),
                (x, y)
                    if x == y
                        && matches!(
                            x,
                            CType::Int64
                                | CType::Bool
                                | CType::Float64
                                | CType::Date
                                | CType::Moment
                                | CType::Time
                                | CType::Duration
                        ) =>
                {
                    format!("({left} == {right})")
                }
                (x, y) if x == y => c_refuse_expr(&format!("a policy comparing two {x:?} fields")),
                _ => "0".to_string(),
            }
        }
        PC::Or(l, r) => format!(
            "({} || {})",
            emit_policy_condition(ctx, l, subject, subject_type, object, depth),
            emit_policy_condition(ctx, r, subject, subject_type, object, depth)
        ),
        PC::And(l, r) => format!(
            "({} && {})",
            emit_policy_condition(ctx, l, subject, subject_type, object, depth),
            emit_policy_condition(ctx, r, subject, subject_type, object, depth)
        ),
    }
}

/// Lower `Increase/Decrease s's counter by n` to a `logos_crdt_bump` on the field's per-replica map (in
/// place, so it is visible through the struct). `negate` distinguishes `Decrease` (a `Tally`); the map's
/// `int64_t` cell holds the `logos_crdt*` handle. Non-counter fields never reach here (the parser only
/// emits `IncreaseCrdt`/`DecreaseCrdt` for a `Shared` counter field), but if the type is unknown the bump
/// is a no-op comment rather than a miscompile.
fn emit_crdt_bump_c(
    object: &Expr,
    field: Symbol,
    amount: &Expr,
    negate: bool,
    ctx: &mut CContext,
    output: &mut String,
    pad: &str,
) {
    let obj = codegen_expr(object, ctx);
    let fname = escape_c_ident(ctx.interner.resolve(field));
    let amt = codegen_expr(amount, ctx);
    let struct_sym = match infer_expr_type(object, ctx) {
        CType::Struct(s) => Some(s),
        _ => None,
    };
    let is_counter = matches!(struct_sym, Some(sym) if ctx.is_counter_field(sym, field));
    if is_counter {
        let cell = format!("(logos_crdt *)(intptr_t)({}.{})", obj, fname);
        let neg = if negate { 1 } else { 0 };
        if struct_sym
            .map(|s| ctx.field_is_grow_only_counter(s, field))
            .unwrap_or(false)
        {
            // A grow-only counter (ConvergentCount/GCounter) TRAPS on a negative net delta (a negative
            // `Increase` or any `Decrease`) — matching the VM/tree-walker.
            writeln!(
                output,
                "{pad}{{ long long __amt = (long long)({amt}); long long __d = {neg} ? -__amt : __amt; \
                 if (__d < 0) {fail} \
                 logos_crdt_bump({cell}, __amt, {neg}); }}",
                fail = ctx.fail_stmt("\"grow-only counter cannot decrease\"")
            )
            .unwrap();
        } else {
            writeln!(
                output,
                "{pad}logos_crdt_bump({cell}, (long long)({amt}), {neg});"
            )
            .unwrap();
        }
    } else {
        writeln!(output, "{}/* crdt bump of non-counter field */", pad).unwrap();
    }
}

/// Lower `Launch a task to fn [with a…]` / `Let h be Launch …` to `logos_spawn`. The task body is the
/// spawned function's `logos_taskentry_<fn>` wrapper, which the scheduler calls by pointer with the task's
/// arguments as one array of scalars, however many there are, each packed by the callee's parameter type
/// (`task_arg_pack`). A captured handle (`Let h be Launch …`) binds the returned `logos_task*` as an
/// `int64_t`.
/// `value` in the scalar packing of the element `pipe` carries — refused by name for a value no scalar holds.
pub(super) fn pipe_packed_value(value: &Expr, pipe: &Expr, ctx: &CContext) -> String {
    let element = pipe_element(pipe, ctx);
    match task_arg_pack(&element, &codegen_expr(value, ctx)) {
        Some(packed) => packed,
        None => c_refuse_expr(&format!(
            "a {:?} sent into a pipe, which a pipe's scalar cell cannot carry",
            element
        )),
    }
}

/// Bind `sym` to `value`, a C expression of type `ty`: a declaration, or an assignment to the variable the
/// current block already declared for `sym` with that type (see `CContext::declare`).
pub(super) fn emit_binding(
    sym: Symbol,
    ty: CType,
    value: &str,
    ctx: &mut CContext,
    output: &mut String,
    pad: &str,
) {
    // A text the body appends to is bound as a builder of its own.
    let built;
    let value = if ty == CType::String && builds_text(sym, ctx) {
        built = format!("logos_text_builder({value}, 0)");
        built.as_str()
    } else {
        value
    };
    let reuse = ctx.declared_value_here(sym, &ty);
    let name = ctx.declare(sym, ty.clone());
    let head = if reuse {
        name
    } else {
        ctx.decl_head(&ty, &name)
    };
    writeln!(output, "{pad}{head} = {value};").unwrap();
}

/// Bind `sym` to nothing, as a zone binds its name and a `Spawn` its agent's: a value-or-nothing that is
/// absent. A name the program never reads or sets needs no variable.
fn bind_nothing(sym: Symbol, ctx: &mut CContext, output: &mut String, pad: &str) {
    if ctx.nothing_names.contains(&sym) {
        emit_binding(
            sym,
            CType::Opt(Box::new(CType::Int64)),
            "((logos_opt){0, 0})",
            ctx,
            output,
            pad,
        );
    }
}

/// `object` handed to the recipient `f` of a `Show … to f` or a `Give … to f`, as the VM's recipient call
/// does: a function of one parameter is called with it, a variable holding a closure is called with it, and
/// any other recipient fails once the value is computed.
fn emit_recipient_call(
    object: &Expr,
    f: Symbol,
    ctx: &mut CContext,
    output: &mut String,
    pad: &str,
) {
    let name = ctx.interner.resolve(f);
    let arity = ctx.fn_params.get(&f).map(Vec::len);
    match (arity, ctx.vars.get(&f)) {
        (Some(1), _) => {
            let call = Expr::Call {
                function: f,
                args: vec![object],
            };
            writeln!(output, "{pad}{};", codegen_expr(&call, ctx)).unwrap();
        }
        (Some(n), _) => {
            let failure = format!("Function {name} expects {n} arguments, got 1");
            writeln!(output, "{pad}(void)({});", codegen_expr(object, ctx)).unwrap();
            writeln!(
                output,
                "{pad}{}",
                ctx.fail_stmt(&c_string_literal(&failure))
            )
            .unwrap();
        }
        (None, Some(CType::FnPtr { .. })) => {
            let callee = Expr::Identifier(f);
            let call = Expr::CallExpr {
                callee: &callee,
                args: vec![object],
            };
            writeln!(output, "{pad}{};", codegen_expr(&call, ctx)).unwrap();
        }
        (None, _) => {
            let failure = format!("Unknown function: {name}");
            writeln!(output, "{pad}(void)({});", codegen_expr(object, ctx)).unwrap();
            writeln!(
                output,
                "{pad}{}",
                ctx.fail_stmt(&c_string_literal(&failure))
            )
            .unwrap();
        }
    }
}

/// `text` as a C string literal.
pub(super) fn c_string_literal(text: &str) -> String {
    format!("\"{}\"", escape_c_string(text))
}

fn emit_spawn_c(
    function: Symbol,
    args: &[&Expr],
    handle: Option<Symbol>,
    ctx: &mut CContext,
    output: &mut String,
    pad: &str,
) {
    let fname = escape_c_ident(ctx.interner.resolve(function));
    let params = ctx.task_params.get(&function).cloned().unwrap_or_default();
    let mut packed = Vec::with_capacity(args.len());
    for (i, e) in args.iter().enumerate() {
        let ty = params.get(i).cloned().unwrap_or(CType::Int64);
        match task_arg_pack(&ty, &codegen_expr(e, ctx)) {
            Some(p) => packed.push(p),
            None => {
                let refusal = format!(
                    "a task launched with a {:?} argument, which a task's scalars cannot carry",
                    ty
                );
                output.push_str(&c_refuse_expr(&refusal));
                return;
            }
        }
    }
    let argv = if packed.is_empty() {
        "0".to_string()
    } else {
        format!("(const long long[]){{{}}}", packed.join(", "))
    };
    let call = format!(
        "logos_spawn((void *)logos_taskentry_{}, {}, {})",
        fname,
        args.len(),
        argv
    );
    if let Some(h) = handle {
        emit_binding(
            h,
            CType::Int64,
            &format!("(int64_t){}", call),
            ctx,
            output,
            pad,
        );
    } else {
        writeln!(output, "{}{};", pad, call).unwrap();
    }
}

/// A launch argument, bound for a parameter of type `ty`, as one of the task's scalars: a Real crosses as
/// its bits (so it arrives whole), an exact integer as the cell `lint_to_cell` makes of it, and an integer,
/// machine word, truth value, instant, text or handle as itself. `None` for a value no scalar holds (a
/// sequence, a map, a struct), which a launch refuses by name.
pub(super) fn task_arg_pack(ty: &CType, value: &str) -> Option<String> {
    match ty {
        CType::Float64 => Some(format!("logos_real_bits((double)({}))", value)),
        held if super::types::opt_holds_in_cell(held) => Some(format!("(long long)({})", value)),
        CType::BigInt => Some(format!("lint_to_cell({})", value)),
        // A function value crosses as the address of a copy of its own.
        CType::FnPtr { .. } => Some(format!(
            "((long long)(intptr_t)({{ LClosure *__boxed = (LClosure *)lc_gc_alloc(sizeof(LClosure)); *__boxed = {value}; __boxed; }}))"
        )),
        _ => None,
    }
}

/// The inverse of [`task_arg_pack`]: the task-entry expression that reads scalar `slot` back as a `ty`.
pub(super) fn task_arg_unpack(ty: &CType, slot: &str, interner: &Interner) -> String {
    match ty {
        CType::Float64 => format!("logos_bits_real({})", slot),
        CType::BigInt => format!("lint_of_cell({slot})"),
        CType::FnPtr { .. } => format!("(*(LClosure *)(intptr_t)({slot}))"),
        _ => format!("({}){}", c_type_str_resolved(ty, interner), slot),
    }
}

/// The C statement that stops the program with `reason` on stderr: its text for a Text reason,
/// its `Show` rendering for an Int or Bool raised by `Fail with`. A contract reason must be Text
/// (`must_be_text`), and a raised value this backend cannot render is refused at generation with
/// an `#error` naming it — never a program that stops with the wrong words.
fn c_failure_report(reason: &Expr, ctx: &CContext, must_be_text: bool) -> String {
    let ty = infer_expr_type(reason, ctx);
    if ty == CType::String {
        return ctx.fail_stmt(&codegen_expr(reason, ctx));
    }
    if must_be_text {
        return match ty {
            CType::Int64 | CType::BigInt => c_refuse_reason("Int", ctx),
            CType::Float64 => c_refuse_reason("Real", ctx),
            CType::Bool => c_refuse_reason("Bool", ctx),
            other => format!(
                "\n#error \"a contract reason of kind {:?} is refused by the C backend\"\n",
                other
            ),
        };
    }
    match ty {
        CType::Int64 | CType::BigInt | CType::Bool | CType::Float64 => {
            ctx.fail_stmt(&stringify_num(reason, ctx))
        }
        other => format!(
            "\n#error \"Fail with: the C backend cannot yet render a {:?} reason\"\n",
            other
        ),
    }
}

/// `format(x)` on the C tier: the value's text exactly as `Show` renders it. A kind this backend
/// cannot render yet is refused at generation, never rendered wrong.
fn c_format_builtin(arg: &Expr, ctx: &CContext) -> String {
    if let Some((var, ty)) = opt_var(arg, ctx) {
        return c_text_of(&ty, &var, ctx);
    }
    match infer_expr_type(arg, ctx) {
        CType::String => format!("lc_gc_strdup({})", codegen_expr(arg, ctx)),
        CType::Int64 | CType::BigInt | CType::Bool | CType::Float64 => stringify_num(arg, ctx),
        other => c_text_of(&other, &codegen_expr(arg, ctx), ctx),
    }
}

/// A non-Text contract reason stops with the same refusal every engine reports.
fn c_refuse_reason(type_name: &str, ctx: &CContext) -> String {
    ctx.fail_stmt(&format!(
        "\"The reason after `otherwise` must be Text, but it was {type_name}.\""
    ))
}
