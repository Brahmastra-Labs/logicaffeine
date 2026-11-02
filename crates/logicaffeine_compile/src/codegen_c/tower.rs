//! The exact extension in the AST → C backend. A binding, parameter or result the extension reaches
//! ([`crate::analysis::tower`]), and every value of a declared exact type, is an `LTower`: a collected cell
//! around a handle into the linked value runtime (`tower.c`, over `logicaffeine_data::abi`), so it computes,
//! prints, compares and fails as the VM's value does. An expression is typed from its operands' C types: an
//! operator or builtin with an `LTower` operand answers one, and a value of any other kind that meets one is
//! converted into the runtime's value model on the way in.
//!
//! A cell belongs to the one place that holds it. An operation answers a fresh cell; a value read from a
//! binding, a field or a call is copied into a cell of its own wherever it is stored, so `Push`, `Pop` and
//! `Set item` change only the list their binding owns. A program the extension never reaches has no reach
//! here and emits exactly what it did before.

use std::rc::Rc;

use logicaffeine_base::extension::Builtin;

use super::types::{infer_expr_type, CContext, CType, Ints};
use crate::analysis::tower::{TowerTypes, LIFTING_BUILTINS};
use crate::analysis::TypeRegistry;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};

/// The C runtime of the extension, emitted only for a program it reaches: the core every runtime on the collector
/// embeds, then what the C runtime's own values are in it.
pub(super) const RUNTIME: &str = concat!(include_str!("tower.c"), "\n", include_str!("tower_of_c.c"));

/// The core of the extension's C runtime ([`RUNTIME`] without the C backends' own conversions), which the RISC-V
/// shim embeds over the same collector.
pub(crate) const CORE: &str = include_str!("tower.c");

/// The declared type names that are exact values.
pub(super) const TYPE_NAMES: &[&str] = crate::analysis::tower::TOWER_TYPES;

/// The program's reach of the extension: the analysis's answer, or an empty one for a program whose only
/// exact values are fields of its records. `None` when the extension reaches nothing.
pub(super) fn reach_of(stmts: &[Stmt], registry: &TypeRegistry, interner: &Interner) -> Option<Rc<TowerTypes>> {
    if let Some(reach) = TowerTypes::of_program(stmts, interner) {
        return Some(Rc::new(reach));
    }
    let exact_field = registry.iter_types().any(|(_, def)| {
        let fields: Vec<&crate::analysis::FieldType> = match def {
            crate::analysis::TypeDef::Struct { fields, .. } => fields.iter().map(|f| &f.ty).collect(),
            crate::analysis::TypeDef::Enum { variants, .. } => {
                variants.iter().flat_map(|v| v.fields.iter().map(|f| &f.ty)).collect()
            }
            _ => Vec::new(),
        };
        fields.into_iter().any(|t| names_exact(t, interner))
    });
    exact_field.then(|| Rc::new(TowerTypes::default()))
}

fn names_exact(t: &crate::analysis::FieldType, interner: &Interner) -> bool {
    use crate::analysis::FieldType;
    match t {
        FieldType::Primitive(s) | FieldType::Named(s) => TYPE_NAMES.contains(&interner.resolve(*s)),
        FieldType::Generic { params, .. } | FieldType::Tuple(params) => params.iter().any(|p| names_exact(p, interner)),
        FieldType::Record(fields) => fields.iter().any(|(_, p)| names_exact(p, interner)),
        FieldType::TypeParam(_) => false,
    }
}

/// Whether the binding `sym`, named in the body being emitted, holds an exact value.
pub(super) fn binding(ctx: &CContext, sym: Symbol) -> bool {
    ctx.tower.as_ref().is_some_and(|t| t.binding(ctx.tower_scope, sym))
}

/// Whether `function`'s parameter `param` holds an exact value.
pub(super) fn param(ctx: &CContext, function: Symbol, param: Symbol) -> bool {
    ctx.tower.as_ref().is_some_and(|t| t.binding(Some(function), param))
}

/// Whether `function` returns an exact value.
pub(super) fn returns(ctx: &CContext, function: Symbol) -> bool {
    ctx.tower.as_ref().is_some_and(|t| t.returns(function))
}

/// The builtin of the extension a call names, unless the program defines a function of that name.
fn builtin(function: Symbol, ctx: &CContext) -> Option<Builtin> {
    if ctx.funcs.contains_key(&function) && !ctx.native_funcs.contains(&function) {
        return None;
    }
    Builtin::named(ctx.interner.resolve(function))
}

/// Whether a call names a builtin that defines a real number by a function of the program, unless the program
/// defines a function of that name.
fn definer(function: Symbol, ctx: &CContext) -> bool {
    !(ctx.funcs.contains_key(&function) && !ctx.native_funcs.contains(&function))
        && crate::analysis::tower::DEFINERS.contains(&ctx.interner.resolve(function))
}

/// The builtins that round, measure or combine a number in its own kind, unless the program defines one.
fn lifter(function: Symbol, ctx: &CContext) -> Option<&'static str> {
    if ctx.funcs.contains_key(&function) && !ctx.native_funcs.contains(&function) {
        return None;
    }
    let name = ctx.interner.resolve(function);
    LIFTING_BUILTINS.iter().copied().find(|l| *l == name)
}

fn is_tower(expr: &Expr, ctx: &CContext) -> bool {
    infer_expr_type(expr, ctx) == CType::Tower
}

/// `Some(LTower)` for an expression whose value is exact: an operator other than `and`, `or` and `combined
/// with` over an exact operand, `not` of one, a builtin of the extension, a real number a function defines, a
/// lifting builtin over an exact argument, an item or a run of items of an exact list, a list literal holding an exact item.
pub(super) fn infer(expr: &Expr, ctx: &CContext) -> Option<CType> {
    ctx.tower.as_ref()?;
    let exact = match expr {
        Expr::BinaryOp { op, left, right } => {
            !matches!(op, BinaryOpKind::And | BinaryOpKind::Or | BinaryOpKind::Concat)
                && (is_tower(left, ctx) || is_tower(right, ctx))
        }
        Expr::Not { operand } => is_tower(operand, ctx),
        Expr::Call { function, args } => {
            builtin(*function, ctx).is_some()
                || definer(*function, ctx)
                || (lifter(*function, ctx).is_some() && args.iter().any(|a| is_tower(a, ctx)))
        }
        Expr::Index { collection, .. } | Expr::Slice { collection, .. } => is_tower(collection, ctx),
        Expr::List(items) => items.iter().any(|i| is_tower(i, ctx)),
        _ => false,
    };
    exact.then_some(CType::Tower)
}

/// The C of an exact expression [`infer`] types, built on the value runtime; `None` for any other node, which
/// the general emission writes (a bound name, a field, a call of the program's own function).
pub(super) fn emit(expr: &Expr, ctx: &CContext) -> Option<String> {
    infer(expr, ctx)?;
    let v = |e: &Expr| value(e, ctx);
    Some(match expr {
        Expr::BinaryOp { op, left, right } => {
            let f = match op {
                BinaryOpKind::Add => "add",
                BinaryOpKind::Subtract => "sub",
                BinaryOpKind::Multiply => "mul",
                BinaryOpKind::Divide => "div",
                BinaryOpKind::ExactDivide => "exact_div",
                BinaryOpKind::FloorDivide => "floor_div",
                BinaryOpKind::Modulo => "rem",
                BinaryOpKind::Pow => "pow",
                BinaryOpKind::Eq => "eq",
                BinaryOpKind::NotEq => "ne",
                BinaryOpKind::Lt => "lt",
                BinaryOpKind::Gt => "gt",
                BinaryOpKind::LtEq => "le",
                BinaryOpKind::GtEq => "ge",
                BinaryOpKind::ApproxEq => "approx_eq",
                BinaryOpKind::SeqConcat => "seq_concat",
                BinaryOpKind::BitAnd => "bit_and",
                BinaryOpKind::BitOr => "bit_or",
                BinaryOpKind::BitXor => "bit_xor",
                BinaryOpKind::Shl => "shl",
                BinaryOpKind::Shr => "shr",
                BinaryOpKind::And | BinaryOpKind::Or | BinaryOpKind::Concat => {
                    unreachable!("`and`, `or` and `combined with` are never exact")
                }
            };
            format!("ltower_{f}({}, {})", v(left), v(right))
        }
        Expr::Not { operand } => format!("ltower_not({})", v(operand)),
        Expr::Call { function, args } => {
            if let Some(b) = builtin(*function, ctx) {
                return Some(format!("ltower_call(\"{}\", {})", b.name(), run(args.iter().copied(), ctx)));
            }
            if definer(*function, ctx) {
                return Some(defined_real(args, ctx));
            }
            let lifted = lifter(*function, ctx).expect("an exact call is a builtin or a lifting builtin");
            match lifted {
                "floor" | "ceil" | "round" | "abs" | "sqrt" => format!("ltower_{lifted}({})", v(args[0])),
                "min" | "max" => format!("ltower_{lifted}({}, {})", v(args[0]), v(args[1])),
                "pow" => format!("ltower_power({}, {})", v(args[0]), v(args[1])),
                other => unreachable!("`{other}` is not a lifting builtin"),
            }
        }
        Expr::Index { collection, index } => format!("ltower_item({}, {})", v(collection), v(index)),
        Expr::Slice { collection, start, end } => {
            format!("ltower_slice({}, {}, {})", v(collection), v(start), v(end))
        }
        Expr::List(items) => format!("ltower_list({})", run(items.iter().copied(), ctx)),
        _ => return None,
    })
}

/// `definedReal(kind, base, name, f)`, the phrase `the real number whose digits in base 10 are f`: the runtime's
/// real number, reading `f` through a reader lifted beside the program's closures, which calls the closure at the
/// place the runtime asks for and hands its answer over as the runtime's value.
fn defined_real(args: &[&Expr], ctx: &CContext) -> String {
    let [Expr::Literal(Literal::Text(kind)), Expr::Literal(Literal::Number(base)), Expr::Literal(Literal::Text(name)), f] =
        args
    else {
        unreachable!("the checker admits definedReal only as its phrase writes it")
    };
    let CType::FnPtr { ret, params } = infer_expr_type(f, ctx) else {
        unreachable!("the phrase defines a real number by a closure")
    };
    let id = ctx.lambda_ctr.get();
    ctx.lambda_ctr.set(id + 1);
    let closure = super::emit::codegen_expr(f, ctx);
    let reader = format!("__logos_defines_{id}");
    let place = if params.first() == Some(&CType::BigInt) { "lint_of(n)" } else { "n" };
    let answer = convert(
        &ret,
        format!("(({})code)(env, {place})", super::types::closure_code_type(&ret, &params, ctx.interner)),
    );
    ctx.lifted.borrow_mut().push(format!(
        "static LogosRtValue *{reader}(void *code, void *env, int64_t n) {{\n    return ltower_hand_out({answer});\n}}\n"
    ));
    let text = |s: &Symbol| super::emit::c_string_literal(ctx.interner.resolve(*s));
    format!(
        "({{ LClosure __defines{id} = {closure}; ltower_define({}, {base}LL, {}, {reader}, __defines{id}.call, \
         __defines{id}.env); }})",
        text(kind),
        text(name)
    )
}

/// What an expression reads off an exact value as a machine value: `length of` an exact list (an `Int`), and
/// whether one `contains` a value (a `Bool`). `None` for any other node.
pub(super) fn emit_reading(expr: &Expr, ctx: &CContext) -> Option<String> {
    ctx.tower.as_ref()?;
    match expr {
        Expr::Length { collection } if is_tower(collection, ctx) => {
            Some(format!("ltower_length({})", super::emit::codegen_expr(collection, ctx)))
        }
        Expr::Contains { collection, value: item } if is_tower(collection, ctx) => Some(format!(
            "ltower_contains({}, {})",
            super::emit::codegen_expr(collection, ctx),
            value(item, ctx)
        )),
        _ => None,
    }
}

/// A count and a C array of the values of `items`, as `ltower_list` and `ltower_call` take them.
fn run<'e, 'a: 'e>(items: impl Iterator<Item = &'e Expr<'a>>, ctx: &CContext) -> String {
    let values: Vec<String> = items.map(|i| value(i, ctx)).collect();
    if values.is_empty() {
        "0, NULL".to_string()
    } else {
        format!("{}, (LTower[]){{{}}}", values.len(), values.join(", "))
    }
}

/// `expr` as an `LTower`: its own C when it is one, converted from the C value it has otherwise. A list or a
/// tuple written in place is built of its items' values, whatever C type its items alone would take.
pub(super) fn value(expr: &Expr, ctx: &CContext) -> String {
    match expr {
        Expr::List(items) if !is_tower(expr, ctx) => format!("ltower_list({})", run(items.iter().copied(), ctx)),
        // An exact division answers a ratio, which only the runtime holds as the VM does.
        Expr::BinaryOp { op: BinaryOpKind::ExactDivide, left, right } => {
            format!("ltower_exact_div({}, {})", value(left, ctx), value(right, ctx))
        }
        Expr::Tuple(items) => format!("ltower_tuple({})", run(items.iter().copied(), ctx)),
        Expr::Literal(Literal::Nothing) => "ltower_nothing()".to_string(),
        _ => convert(&infer_expr_type(expr, ctx), super::emit::codegen_expr(expr, ctx)),
    }
}

/// [`value`], in a cell of its own when it is read from a place that keeps its cell (a binding, a field, a
/// call's result): what a binding, a field, a list or a parameter stores.
pub(super) fn stored(expr: &Expr, ctx: &CContext) -> String {
    let rendered = value(expr, ctx);
    if is_tower(expr, ctx) && keeps_its_cell(expr, ctx) {
        format!("ltower_own({rendered})")
    } else {
        rendered
    }
}

/// Whether the cell `expr` answers is one some place still holds.
fn keeps_its_cell(expr: &Expr, ctx: &CContext) -> bool {
    match expr {
        Expr::Identifier(_) | Expr::FieldAccess { .. } | Expr::Copy { .. } | Expr::Give { .. } => true,
        Expr::Call { function, .. } => builtin(*function, ctx).is_none() && lifter(*function, ctx).is_none(),
        _ => infer(expr, ctx).is_none(),
    }
}

/// An `LTower` from `rendered`, a C value of type `ty`.
pub(super) fn convert(ty: &CType, rendered: String) -> String {
    match ty {
        CType::Tower => rendered,
        CType::Int64 => format!("ltower_int({rendered})"),
        CType::Float64 => format!("ltower_float({rendered})"),
        CType::Bool => format!("ltower_bool({rendered})"),
        CType::String => format!("ltower_text({rendered})"),
        CType::Rational | CType::Decimal => format!("ltower_of_ldec({rendered})"),
        CType::BigInt => format!("ltower_of_lint({rendered})"),
        CType::Void => format!("({{ (void)({rendered}); ltower_nothing(); }})"),
        CType::Date => format!("ltower_date({rendered})"),
        CType::Moment => format!("ltower_moment({rendered})"),
        CType::Time => format!("ltower_time({rendered})"),
        CType::Span => format!("({{ LogosSpan __span = {rendered}; ltower_span(__span.months, __span.days); }})"),
        CType::SeqI64(Ints::Int) => format!("({{ Seq_i64 __items = {rendered}; ltower_of_seq_i64(&__items); }})"),
        CType::SeqF64 => format!("({{ Seq_f64 __items = {rendered}; ltower_of_seq_f64(&__items); }})"),
        CType::SeqBool => format!("({{ Seq_bool __items = {rendered}; ltower_of_seq_bool(&__items); }})"),
        CType::SeqStr => format!("({{ Seq_str __items = {rendered}; ltower_of_seq_str(&__items); }})"),
        other => super::emit::c_refuse_expr(&format!("{} held where an exact value is", kind_name(other))),
    }
}

/// How a refusal names a C type a value of the extension does not mix with.
fn kind_name(ty: &CType) -> &'static str {
    match ty {
        CType::Word(_) => "a machine word",
        CType::Complex => "a complex number",
        CType::Modular => "a modular integer",
        CType::Money => "money",
        CType::Quantity => "a quantity",
        CType::Uuid => "a UUID",
        CType::Duration => "a duration",
        CType::Struct(_) | CType::Enum(_) => "a record",
        CType::FnPtr { .. } => "a function value",
        _ => "a collection of this kind",
    }
}

/// The `Int` a place holding a machine integer reads from `expr`, an exact value.
pub(super) fn as_int(rendered: String) -> String {
    format!("ltower_int_of({rendered})")
}

/// The `Int` the machine-integer place `place` reads from `expr`, an exact value: a fraction, or an integer past a
/// word, refused in the VM's words for the place.
pub(super) fn as_int_at(rendered: String, place: super::words::IntegerPlace) -> String {
    format!(
        "ltower_int_at({rendered}, {}, {})",
        super::words::integer_past_a_word(place),
        super::words::fraction_for_an_integer(place)
    )
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_c/tower/tests.rs"]
mod tests;
