//! Equality of C values as the VM compares them (`semantics::compare::values_equal`), and the order of an integer
//! and a real.
//!
//! Two numbers compare by their mathematical values: an `Int` and a `Real` are equal only when the real is that
//! integer, and one orders before the other as the numbers do, however far past 2^53 the integer is. Two texts are
//! equal when their characters are. Two sequences are equal item for item, two maps or sets entry for entry whatever
//! order their entries went in and whatever capacity either grew to, two records or tuples field for field. A
//! value-or-nothing is equal to another when both hold nothing or both hold equal values, and to a value when it holds
//! one equal to it; two values of one enum when they are the same variant with equal fields. `nothing` is equal to
//! `nothing`, to a value-or-nothing that holds nothing and to an `Option` that is `None`, and to no other value — which
//! is still computed: the VM computes both sides of a comparison, whatever it finds.
//!
//! A comparison of structured values is a function of its own, generated once for each pair of types beside the
//! program's other generated functions and declared before its body, so a record holding a list of itself compares
//! by recursion.

use super::types::{c_param_decl, escape_c_ident, opt_unpack, seq_element, CContext, CType};

/// The order of an integer `i` and a real `f` as numbers: -1, 0 or 1 as `i` is below, at or above `f`, and 2 when `f`
/// is not a number. Exact for every integer: one within 2^53 is a double exactly; past it, the real's whole part is
/// compared as an integer, then its fraction.
const INT_REAL_ORDER: &str = "__attribute__((unused)) static int __logos_int_real_order(int64_t i, double f) { \
     if (f != f) return 2; \
     if (i >= -9007199254740992LL && i <= 9007199254740992LL) { double d = (double)i; return d < f ? -1 : (d > f ? 1 : 0); } \
     if (f >= 0x1p63) return -1; if (f < -0x1p63) return 1; \
     double whole = trunc(f); int64_t w = (int64_t)whole; \
     if (i != w) return i < w ? -1 : 1; return f > whole ? -1 : (f < whole ? 1 : 0); }\n";

/// The C expression ordering the integer `int` and the real `real` (see [`INT_REAL_ORDER`]).
fn int_real_order(int: &str, real: &str, ctx: &CContext) -> String {
    if ctx.equal_fns.borrow_mut().insert("__logos_int_real_order".to_string()) {
        ctx.lifted.borrow_mut().push(INT_REAL_ORDER.to_string());
    }
    format!("__logos_int_real_order((int64_t)({int}), (double)({real}))")
}

/// The C expression of the order of `left`, of `lt`, against `right`, of `rt`, when one is an integer (a machine word
/// or an exact one) and the other a real: -1 below, 0 at, 1 above, 2 when the real is not a number. `None` for any
/// other pair.
pub(super) fn mixed_order(lt: &CType, left: &str, rt: &CType, right: &str, ctx: &CContext) -> Option<String> {
    let (order, flipped) = match (lt, rt) {
        (CType::Int64, CType::Float64) => (int_real_order(left, right, ctx), false),
        (CType::Float64, CType::Int64) => (int_real_order(right, left, ctx), true),
        (CType::BigInt, CType::Float64) => (format!("ldec_cmp_f64(ldec_from_lint({left}), (double)({right}))"), false),
        (CType::Float64, CType::BigInt) => (format!("ldec_cmp_f64(ldec_from_lint({right}), (double)({left}))"), true),
        _ => return None,
    };
    if !flipped {
        return Some(order);
    }
    let n = ctx.select_ctr.get();
    ctx.select_ctr.set(n + 1);
    Some(format!("({{ int __flip{n} = {order}; __flip{n} == 2 ? 2 : -__flip{n}; }})"))
}

/// A map's key and value types, or a set's key and no value: the hash tables the C tier holds.
pub(super) fn table_parts(ty: &CType) -> Option<(CType, Option<CType>)> {
    if let Some((key, value)) = super::types::map_entry_types(ty) {
        return Some((key, Some(value)));
    }
    match ty {
        CType::SetI64(ints) => Some((ints.ctype(), None)),
        CType::SetStr => Some((CType::String, None)),
        CType::SetTuple(elems) => Some((CType::Tuple(elems.clone()), None)),
        _ => None,
    }
}

/// Whether two values of C type `ty` compare by a generated function rather than by C's `==`: a sequence of any
/// element, a map or a set, a record, a tuple, or a value of an enum.
pub(super) fn is_structured(ty: &CType, ctx: &CContext) -> bool {
    seq_element(ty, ctx.registry).is_some()
        || table_parts(ty).is_some()
        || matches!(ty, CType::Struct(_) | CType::Tuple(_) | CType::Record(_) | CType::Enum(_))
}

/// Whether the C expression `operand` is a literal — a number, a truth value or a text — which a comparison has nothing
/// left to compute of. A name is read, as the VM reads it.
fn inert(operand: &str) -> bool {
    let literal_text = operand.len() >= 2
        && operand.starts_with('"')
        && operand.ends_with('"')
        && !operand[1..operand.len() - 1].contains('"');
    let number = operand.starts_with(|c: char| c.is_ascii_digit())
        && operand.chars().all(|c| c.is_ascii_alphanumeric() || c == '.');
    literal_text || number || operand == "true" || operand == "false"
}

/// The C expression of `is not equal to` from `eq`, the one of `is equal to`: a decided answer is the other one,
/// anything else its negation.
pub(super) fn unequal(eq: String) -> String {
    match eq.as_str() {
        "true" => "false".to_string(),
        "false" => "true".to_string(),
        _ => format!("(!{eq})"),
    }
}

/// When `ty` is an `Option` (the built-in one as the program monomorphized it, or the program's own): its `None`
/// constant.
fn option_none(ty: &CType, ctx: &CContext) -> Option<String> {
    let CType::Enum(sym) = ty else { return None };
    let shown = ctx.display_names.get(sym).copied().unwrap_or(*sym);
    let has_none = matches!(
        ctx.registry.get(*sym),
        Some(crate::analysis::TypeDef::Enum { variants, .. })
            if variants.iter().any(|v| ctx.interner.resolve(v.name) == "None")
    );
    (ctx.interner.resolve(shown) == "Option" && has_none)
        .then(|| format!("{}_None", escape_c_ident(ctx.interner.resolve(*sym))))
}

/// Whether `value`, of C type `ty`, which is not a value-or-nothing, is `nothing`: an `Option` is when it is `None`;
/// a value of any other type never is, though it is still computed.
fn is_nothing(ty: &CType, value: &str, ctx: &CContext) -> String {
    match option_none(ty, ctx) {
        Some(none) => format!("(({value}).tag == {none})"),
        None if inert(value) => "false".to_string(),
        None => format!("((void)({value}), false)"),
    }
}

/// `test`, with `first` computed before it when it computes anything: the left operand of a comparison whose `test`
/// reads only the right.
fn first_then(first: &str, test: String) -> String {
    if first.is_empty() || inert(first) {
        test
    } else {
        format!("((void)({first}), {test})")
    }
}

/// `test`, with `then` computed after it when it computes anything: the right operand of a comparison whose `test`
/// reads only the left.
fn then_computed(test: String, then: &str, n: usize) -> String {
    if then.is_empty() || inert(then) {
        test
    } else {
        format!("({{ bool __tested{n} = {test}; (void)({then}); __tested{n}; }})")
    }
}

/// The C expression of whether `left`, of C type `lt`, equals `right`, of `rt`, as the VM compares them; `None` for a
/// pair it does not compare, which the caller lowers its own way. Each operand is evaluated once, left first.
pub(super) fn value_equal(lt: &CType, left: &str, rt: &CType, right: &str, ctx: &CContext) -> Option<String> {
    if let Some(same) = scalar_equal(lt, left, rt, right, ctx) {
        return Some(same);
    }
    let n = ctx.select_ctr.get();
    ctx.select_ctr.set(n + 1);
    match (lt, rt) {
        (CType::Void, CType::Void) => Some(first_then(left, then_computed("true".to_string(), right, n))),
        (CType::Opt(_), CType::Void) => Some(then_computed(format!("(!({left}).present)"), right, n)),
        (CType::Void, CType::Opt(_)) => Some(first_then(left, format!("(!({right}).present)"))),
        (other, CType::Void) => Some(then_computed(is_nothing(other, left, ctx), right, n)),
        (CType::Void, other) => Some(first_then(left, is_nothing(other, right, ctx))),
        (CType::Opt(l), CType::Opt(r)) => {
            let same = value_equal(
                l,
                &opt_unpack(l, &format!("__l{n}.value"), ctx.interner),
                r,
                &opt_unpack(r, &format!("__r{n}.value"), ctx.interner),
                ctx,
            )?;
            Some(format!(
                "({{ logos_opt __l{n} = {left}, __r{n} = {right}; \
                 __l{n}.present == __r{n}.present && (!__l{n}.present || {same}); }})"
            ))
        }
        (CType::Opt(l), r) => {
            let same = value_equal(l, &opt_unpack(l, &format!("__l{n}.value"), ctx.interner), r, &format!("__r{n}"), ctx)?;
            let decl = c_param_decl(r, &format!("__r{n}"), ctx.interner);
            Some(format!("({{ logos_opt __l{n} = {left}; {decl} = {right}; __l{n}.present && {same}; }})"))
        }
        (l, CType::Opt(r)) => {
            let same = value_equal(l, &format!("__l{n}"), r, &opt_unpack(r, &format!("__r{n}.value"), ctx.interner), ctx)?;
            let decl = c_param_decl(l, &format!("__l{n}"), ctx.interner);
            Some(format!("({{ {decl} = {left}; logos_opt __r{n} = {right}; __r{n}.present && {same}; }})"))
        }
        _ => structured_equal(lt, left, rt, right, ctx),
    }
}

/// Equality of two scalars, inline; `None` for a pair that is not two scalars it knows.
fn scalar_equal(lt: &CType, left: &str, rt: &CType, right: &str, ctx: &CContext) -> Option<String> {
    use CType::*;
    Some(match (lt, rt) {
        (Int64, Int64) | (Bool, Bool) | (Float64, Float64) | (Moment, Moment) | (Date, Date) | (Time, Time)
        | (Duration, Duration) => format!("(({left}) == ({right}))"),
        (Word(a), Word(b)) if a == b => format!("(({left}) == ({right}))"),
        (String, String) => format!("str_equals({left}, {right})"),
        (Rational, Rational) | (Decimal, Decimal) => format!("(ldec_cmp({left}, {right}) == 0)"),
        (BigInt, BigInt) => format!("(logos_lint_cmp({left}, {right}) == 0)"),
        (Int64, BigInt) => format!("(logos_lint_cmp(lint_of({left}), {right}) == 0)"),
        (BigInt, Int64) => format!("(logos_lint_cmp({left}, lint_of({right})) == 0)"),
        (Int64 | BigInt, Float64) | (Float64, Int64 | BigInt) => {
            format!("({} == 0)", mixed_order(lt, left, rt, right, ctx)?)
        }
        (Span, Span) => format!("logos_span_equals({left}, {right})"),
        (Uuid, Uuid) => format!("uuid_eq({left}, {right})"),
        (Bool, Int64 | Float64 | String) | (Int64 | Float64 | String, Bool) => "false".to_string(),
        (String, Int64 | Float64) | (Int64 | Float64, String) => "false".to_string(),
        _ => return None,
    })
}

/// The fields a record, a tuple or a struct of C type `ty` compares by: each one's C member and type.
fn fields(ty: &CType, ctx: &CContext) -> Option<Vec<(String, CType)>> {
    Some(match ty {
        CType::Struct(sym) => {
            let crate::analysis::TypeDef::Struct { fields, .. } = ctx.registry.get(*sym)? else {
                return None;
            };
            fields
                .iter()
                .map(|field| {
                    (
                        escape_c_ident(ctx.interner.resolve(field.name)),
                        super::exact::field_ctype(*sym, field, ctx.interner, ctx.registry, &ctx.field_facts()),
                    )
                })
                .collect()
        }
        CType::Tuple(items) => items.iter().enumerate().map(|(i, ty)| (format!("f{i}"), ty.clone())).collect(),
        CType::Record(items) => items.iter().enumerate().map(|(i, (_, ty))| (format!("f{i}"), ty.clone())).collect(),
        _ => return None,
    })
}

/// One variant of an enum, as a comparison reads it: its tag constant, and each field's member, whether it is held
/// through a pointer (a field naming its own enum), and its C type.
struct Variant {
    tag: String,
    fields: Vec<(String, bool, CType)>,
}

/// The variants a value of the enum of C type `ty` compares by, in declaration order; `None` for any other type.
fn variants(ty: &CType, ctx: &CContext) -> Option<Vec<Variant>> {
    let CType::Enum(sym) = ty else { return None };
    let crate::analysis::TypeDef::Enum { variants, .. } = ctx.registry.get(*sym)? else {
        return None;
    };
    let name = escape_c_ident(ctx.interner.resolve(*sym));
    Some(
        variants
            .iter()
            .map(|v| {
                let vname = escape_c_ident(ctx.interner.resolve(v.name));
                let fields = v
                    .fields
                    .iter()
                    .map(|f| {
                        let member = format!("data.{vname}.{}", escape_c_ident(ctx.interner.resolve(f.name)));
                        let boxed = matches!(&f.ty, crate::analysis::FieldType::Named(fsym) if fsym == sym);
                        let ctype = if boxed {
                            ty.clone()
                        } else {
                            super::exact::field_ctype(v.name, f, ctx.interner, ctx.registry, &ctx.field_facts())
                        };
                        (member, boxed, ctype)
                    })
                    .collect();
                Variant { tag: format!("{name}_{vname}"), fields }
            })
            .collect(),
    )
}

/// Whether every comparison a structured comparison of `lt` and `rt` makes is one this module makes.
fn compares(lt: &CType, rt: &CType, ctx: &CContext, seen: &mut Vec<(CType, CType)>) -> bool {
    if seen.iter().any(|(l, r)| l == lt && r == rt) {
        return true;
    }
    let scalar = |t: &CType| {
        matches!(
            t,
            CType::Int64
                | CType::Bool
                | CType::Float64
                | CType::String
                | CType::Rational
                | CType::Decimal
                | CType::BigInt
                | CType::Moment
                | CType::Date
                | CType::Time
                | CType::Duration
                | CType::Span
                | CType::Uuid
                | CType::Word(_)
        )
    };
    if scalar(lt) && scalar(rt) {
        return scalar_equal(lt, "a", rt, "b", ctx).is_some();
    }
    seen.push((lt.clone(), rt.clone()));
    let answer = match (lt, rt) {
        (CType::Opt(l), CType::Opt(r)) => compares(l, r, ctx, seen),
        (CType::Opt(l), r) | (r, CType::Opt(l)) => compares(l, r, ctx, seen),
        _ => match (seq_element(lt, ctx.registry), seq_element(rt, ctx.registry)) {
            (Some(l), Some(r)) => compares(&l, &r, ctx, seen),
            _ if lt != rt => false,
            _ => match table_parts(lt) {
                Some((key, value)) => {
                    compares(&key, &key, ctx, seen) && value.as_ref().is_none_or(|v| compares(v, v, ctx, seen))
                }
                None => match variants(lt, ctx) {
                    Some(vs) => vs.iter().flat_map(|v| &v.fields).all(|(_, _, t)| compares(t, t, ctx, seen)),
                    None => fields(lt, ctx).is_some_and(|fs| fs.iter().all(|(_, t)| compares(t, t, ctx, seen))),
                },
            },
        },
    };
    seen.pop();
    answer
}

/// Equality of two structured values, as a generated function of the pair of types.
fn structured_equal(lt: &CType, left: &str, rt: &CType, right: &str, ctx: &CContext) -> Option<String> {
    if !compares(lt, rt, ctx, &mut Vec::new()) {
        return None;
    }
    let name = format!(
        "__logos_equal_{}_{}",
        escape_c_ident(&format!("{lt:?}")),
        escape_c_ident(&format!("{rt:?}"))
    )
    .replace(|c: char| !c.is_ascii_alphanumeric() && c != '_', "_");
    if ctx.equal_fns.borrow_mut().insert(name.clone()) {
        let params = format!(
            "{}, {}",
            c_param_decl(lt, "a", ctx.interner),
            c_param_decl(rt, "b", ctx.interner)
        );
        ctx.lifted
            .borrow_mut()
            .push(format!("__attribute__((unused)) static bool {name}({params});\n"));
        let body = match (seq_element(lt, ctx.registry), seq_element(rt, ctx.registry)) {
            (Some(l), Some(r)) => {
                let same = value_equal(&l, "a.data[i]", &r, "b.data[i]", ctx)?;
                format!(
                    "if (a.len != b.len) return false; \
                     for (size_t i = 0; i < a.len; i++) if (!({same})) return false; return true;"
                )
            }
            _ => match table_parts(lt) {
                Some((key, value)) => {
                    let same_key = value_equal(&key, "a.keys[i]", &key, "b.keys[j]", ctx)?;
                    let same_value = match &value {
                        Some(value) => value_equal(value, "a.vals[i]", value, "b.vals[j]", ctx)?,
                        None => "true".to_string(),
                    };
                    let slot = match &key {
                        CType::String => "_map_hash_str(a.keys[i], b.cap)".to_string(),
                        CType::Tuple(_) => format!(
                            "__logos_hash_slot({}_hash(a.keys[i]), b.cap)",
                            super::types::c_type_str_resolved(&key, ctx.interner)
                        ),
                        _ => "_map_hash(a.keys[i], b.cap)".to_string(),
                    };
                    // Each key of `a` is probed for in `b`, the probe bounded by `b`'s capacity: neither slot
                    // positions nor insertion order are values.
                    format!(
                        "if (a.len != b.len) return false; if (a.len == 0) return true; if (b.cap == 0) return false; \
                         for (size_t i = 0; i < a.cap; i++) {{ if (a.state[i] != 1) continue; \
                         size_t j = {slot}; bool found = false; \
                         for (size_t p = 0; p < b.cap && b.state[j]; p++) {{ \
                         if (b.state[j] == 1 && {same_key}) {{ if (!({same_value})) return false; found = true; break; }} \
                         j = (j + 1) & (b.cap - 1); }} \
                         if (!found) return false; }} return true;"
                    )
                }
                None if variants(lt, ctx).is_some() => {
                    let vs = variants(lt, ctx)?;
                    let mut cases = String::new();
                    for v in &vs {
                        if v.fields.is_empty() {
                            continue;
                        }
                        let mut checks = Vec::new();
                        for (member, boxed, ty) in &v.fields {
                            let at = |x: &str| if *boxed { format!("(*{x}.{member})") } else { format!("{x}.{member}") };
                            checks.push(value_equal(ty, &at("a"), ty, &at("b"), ctx)?);
                        }
                        cases.push_str(&format!("case {}: return {}; ", v.tag, checks.join(" && ")));
                    }
                    if cases.is_empty() {
                        "return a.tag == b.tag;".to_string()
                    } else {
                        format!("if (a.tag != b.tag) return false; switch (a.tag) {{ {cases}default: return true; }}")
                    }
                }
                None => {
                    let mut checks = Vec::new();
                    for (member, ty) in fields(lt, ctx)? {
                        checks.push(value_equal(&ty, &format!("a.{member}"), &ty, &format!("b.{member}"), ctx)?);
                    }
                    if checks.is_empty() {
                        "return true;".to_string()
                    } else {
                        format!("return {};", checks.join(" && "))
                    }
                }
            },
        };
        ctx.lifted
            .borrow_mut()
            .push(format!("__attribute__((unused)) static bool {name}({params}) {{ {body} }}\n"));
    }
    Some(format!("{name}({left}, {right})"))
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_c/equality/tests.rs"]
mod tests;
