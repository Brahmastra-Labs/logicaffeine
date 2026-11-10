//! Exact numbers in generated Rust. A `LogosRational` is the language's exact number: every operator over one takes
//! the rule the interpreter takes for the value it holds, a whole value the integer's — so `/` of two whole values
//! truncates, `%` and the bitwise operators take two whole values and refuse a fraction in the interpreter's words,
//! and a real beside one meets the double it converts to (an order test, the real's exact value). An integer that
//! meets a rational, or is held where the program holds rationals — a binding some of whose values are fractions, an
//! item of a list that holds one, an argument or a result declared one — is the rational it equals; a rational, or a
//! value of the exact extension, read where the language takes a machine integer (an index, a range's or a slice's
//! bound) is the integer it equals, refused in the interpreter's words for the place when it is none.

use std::collections::{HashMap, HashSet};

use logicaffeine_language::ast_depth::{walk_nodes, Visited, Walk};

use super::expr::{codegen_expr_ctx, ExprCtx};
use crate::analysis::registry::TypeRegistry;
use crate::analysis::types::{FnSig, LogosType, TypeEnv};
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Pattern, Stmt};
use crate::intern::{Interner, Symbol};
use crate::semantics::collections::{integer_place_refusals, IntegerPlace};

/// The Rust type of a rational.
pub(super) const RATIONAL: &str = "LogosRational";

/// What typing an expression reads: the interner, the Rust types of the bindings in scope, and the program's records.
#[derive(Clone, Copy)]
pub(super) struct Typing<'a> {
    pub(super) interner: &'a Interner,
    pub(super) variable_types: &'a HashMap<Symbol, String>,
    pub(super) registry: &'a TypeRegistry,
}

impl<'a> Typing<'a> {
    /// The typing an expression's emission reads.
    pub(super) fn of(ecx: &ExprCtx<'a>) -> Typing<'a> {
        Typing {
            interner: ecx.interner,
            variable_types: ecx.variable_types,
            registry: ecx.registry,
        }
    }

    /// The same typing over the bindings `variable_types`.
    fn over<'b>(self, variable_types: &'b HashMap<Symbol, String>) -> Typing<'b>
    where
        'a: 'b,
    {
        Typing {
            variable_types,
            ..self
        }
    }

    /// The Rust type of `expr`, a read of a record's field.
    fn field(&self, expr: &Expr) -> Option<String> {
        super::expr::record_field_rust_type(expr, self.interner, self.variable_types, self.registry)
    }
}

/// The type a Rust type string names, a borrowed slice as the list it lends.
pub(super) fn named(rust: &str) -> LogosType {
    let held = rust.split("|__hl:").next().unwrap_or(rust);
    match held
        .strip_prefix("&mut [")
        .or_else(|| held.strip_prefix("&["))
        .and_then(|t| t.strip_suffix(']'))
    {
        Some(item) => LogosType::Seq(Box::new(named(item))),
        None => LogosType::from_rust_type_str(held),
    }
}

/// The static type of `expr` under `t`, a call typed by its function's declared result and a read of a record's field
/// by the field's declared type. The environment holds exactly the bindings and functions `expr` names — all its type
/// can read — so typing an expression costs its own size, not the program's.
pub(super) fn type_of(expr: &Expr, t: Typing) -> LogosType {
    let (bindings, calls) = names_in(expr);
    let mut env = TypeEnv::new();
    for sym in bindings {
        if let Some(rust) = t.variable_types.get(&sym) {
            env.register(sym, named(rust));
        }
    }
    super::expr::with_declared_returns(|returns| {
        for f in calls {
            if let Some(rust) = returns.get(&f) {
                env.register_fn(f, FnSig { params: Vec::new(), return_type: named(rust) });
            }
        }
    });
    env.infer_expr_with(expr, t.interner, &|e| t.field(e).map(|rust| named(&rust)))
}

/// The bindings `expr` reads and the functions it calls, anywhere in it.
fn names_in(expr: &Expr) -> (HashSet<Symbol>, HashSet<Symbol>) {
    let (mut bindings, mut calls) = (HashSet::new(), HashSet::new());
    let mut work = vec![expr];
    while let Some(e) = work.pop() {
        match e {
            Expr::Identifier(sym) => {
                bindings.insert(*sym);
            }
            Expr::Call { function, .. } => {
                calls.insert(*function);
            }
            _ => {}
        }
        work.extend(logicaffeine_language::ast_depth::expr_children(e));
    }
    (bindings, calls)
}

/// Whether `expr` may compute with a rational: it names a binding, a field or a call whose type holds one, or makes
/// one — the cheap test before the program's types are asked.
fn mentions_rational(expr: &Expr, t: Typing) -> bool {
    let typed = |rust: Option<&String>| rust.is_some_and(|r| r.contains(RATIONAL));
    match expr {
        Expr::Identifier(sym) => typed(t.variable_types.get(sym)),
        Expr::FieldAccess { .. } => typed(t.field(expr).as_ref()),
        Expr::BinaryOp { op: BinaryOpKind::ExactDivide, .. } => true,
        Expr::BinaryOp { left, right, .. } => mentions_rational(left, t) || mentions_rational(right, t),
        Expr::Index { collection, index } => mentions_rational(collection, t) || mentions_rational(index, t),
        Expr::Slice { collection, .. } => mentions_rational(collection, t),
        Expr::Call { function, args } => {
            super::expr::with_declared_returns(|returns| typed(returns.get(function)))
                || args.iter().any(|a| mentions_rational(a, t))
        }
        Expr::List(items) => items.iter().any(|i| mentions_rational(i, t)),
        Expr::Copy { expr } | Expr::Give { value: expr } => mentions_rational(expr, t),
        _ => false,
    }
}

/// Whether `expr` reads a binding that holds an integer past a machine word (`LogosInt`).
fn reads_big_integer(expr: &Expr, variable_types: &HashMap<Symbol, String>) -> bool {
    let mut found = false;
    let mut visit = |e: &Expr| {
        if let Expr::Identifier(sym) = e {
            found |= variable_types.get(sym).is_some_and(|t| t.contains("__bigint"));
        }
    };
    each_operand(expr, &mut visit);
    found
}

fn each_operand(expr: &Expr, visit: &mut dyn FnMut(&Expr)) {
    visit(expr);
    match expr {
        Expr::BinaryOp { left, right, .. } => {
            each_operand(left, visit);
            each_operand(right, visit);
        }
        Expr::Copy { expr } | Expr::Give { value: expr } => each_operand(expr, visit),
        _ => {}
    }
}

/// `code`, the emission of `expr`, a value of type `from`, held as a value of type `to`, which holds rationals: an
/// integer as the rational it equals, a list item by item. `None` when nothing converts.
pub(super) fn converted(
    expr: &Expr,
    code: &str,
    from: &LogosType,
    to: &LogosType,
    variable_types: &HashMap<Symbol, String>,
) -> Option<String> {
    if from == to || !to.holds_rational() {
        return None;
    }
    match (from, to) {
        (LogosType::Int, LogosType::Rational) if reads_big_integer(expr, variable_types) => {
            Some(format!("LogosRational::from({code})"))
        }
        (LogosType::Int, LogosType::Rational) => Some(format!("LogosRational::from_i64(({code}) as i64)")),
        (LogosType::Seq(_), LogosType::Seq(_)) if LogosType::number_join(from, to).as_ref() == Some(to) => {
            Some(format!("ToRationals::to_rationals(&{code})"))
        }
        _ => None,
    }
}

/// A field of the record type `type_name` a construction that gives `init_fields` leaves unset where its declared
/// type is a rational: the language holds nothing there, which a `LogosRational` cannot hold.
pub(super) fn unset_rational_field(
    type_name: Symbol,
    init_fields: &[(Symbol, &Expr)],
    ecx: &ExprCtx,
) -> Option<Symbol> {
    let crate::analysis::registry::TypeDef::Struct { fields, .. } = ecx.registry.get(type_name)? else {
        return None;
    };
    fields
        .iter()
        .filter(|f| !init_fields.iter().any(|(given, _)| *given == f.name))
        .find(|f| super::types::codegen_field_type(&f.ty, ecx.interner) == RATIONAL)
        .map(|f| f.name)
}

/// The Rust type of `expr` where it holds rationals: a rational, or a list of them.
pub(super) fn rational_type_of(expr: &Expr, t: Typing) -> Option<String> {
    if !mentions_rational(expr, t) {
        return None;
    }
    let held = type_of(expr, t);
    held.holds_rational().then(|| held.to_rust_type())
}

/// The Rust type of an item of `collection`, a list of rationals; `None` for any other collection.
pub(super) fn held_item(collection: &Expr, t: Typing) -> Option<String> {
    if !mentions_rational(collection, t) {
        return None;
    }
    match type_of(collection, t) {
        LogosType::Seq(item) if item.holds_rational() => Some(item.to_rust_type()),
        _ => None,
    }
}

/// The emission of `expr` as an owned value: a binding of a type that is not `Copy` read as a copy.
fn owned(expr: &Expr, t: &LogosType, ecx: &ExprCtx) -> String {
    let code = codegen_expr_ctx(expr, ecx);
    match expr {
        Expr::Identifier(_) if !t.is_copy() => format!("{code}.clone()"),
        _ => code,
    }
}

/// The emission of `expr` as a value of type `to`, which holds rationals, where it is not one already: an integer as
/// the rational it equals, a list literal item by item, any other list of integers converted whole. `None` where
/// `expr` needs no conversion, or none applies.
pub(super) fn value_as(expr: &Expr, to: &LogosType, ecx: &ExprCtx) -> Option<String> {
    if !to.holds_rational() {
        return None;
    }
    if let (Expr::List(items), LogosType::Seq(item)) = (expr, to) {
        let types: Vec<LogosType> = items.iter().map(|i| type_of(i, Typing::of(ecx))).collect();
        if types.iter().all(|t| t == &**item) {
            return None;
        }
        let codes: Vec<String> = items
            .iter()
            .zip(&types)
            .map(|(i, t)| value_as(i, item, ecx).unwrap_or_else(|| owned(i, t, ecx)))
            .collect();
        return Some(format!("LogosSeq::from_vec(vec![{}])", codes.join(", ")));
    }
    let from = type_of(expr, Typing::of(ecx));
    if from == *to {
        return None;
    }
    match (expr, to) {
        (Expr::Literal(Literal::Number(n)), LogosType::Rational) => Some(format!("LogosRational::from_i64({n})")),
        _ => {
            let code = codegen_expr_ctx(expr, &ExprCtx { int_exact_tolerant: false, ..*ecx });
            converted(expr, &code, &from, to, ecx.variable_types)
        }
    }
}

/// The emission of `expr`, of type `t`, as a rational operand.
fn rational_operand(expr: &Expr, t: &LogosType, ecx: &ExprCtx) -> String {
    match t {
        LogosType::Rational => codegen_expr_ctx(expr, ecx),
        _ => {
            let code = codegen_expr_ctx(expr, &ExprCtx { int_exact_tolerant: false, ..*ecx });
            match expr {
                Expr::Literal(Literal::Number(n)) => format!("LogosRational::from_i64({n})"),
                _ => converted(expr, &code, t, &LogosType::Rational, ecx.variable_types).unwrap_or(code),
            }
        }
    }
}

/// The emission of `expr`, of type `t`, as a real operand: a rational the double the language converts it to.
fn real_operand(expr: &Expr, t: &LogosType, ecx: &ExprCtx) -> String {
    let code = codegen_expr_ctx(expr, ecx);
    match t {
        LogosType::Rational => format!("{code}.to_f64()"),
        _ => code,
    }
}

/// The order test `op` of the rational `rational` against the real `real`, `real_left` when the real is on the left:
/// decided on the real's exact value, false against NaN but for `!=`.
fn order_test(op: BinaryOpKind, rational: &str, real: &str, real_left: bool) -> String {
    use BinaryOpKind::*;
    let pattern = match (op, real_left) {
        (Lt, false) | (Gt, true) => "Some(core::cmp::Ordering::Less)",
        (Gt, false) | (Lt, true) => "Some(core::cmp::Ordering::Greater)",
        (LtEq, false) | (GtEq, true) => "Some(core::cmp::Ordering::Less | core::cmp::Ordering::Equal)",
        (GtEq, false) | (LtEq, true) => "Some(core::cmp::Ordering::Greater | core::cmp::Ordering::Equal)",
        (Eq, _) => "Some(core::cmp::Ordering::Equal)",
        (NotEq, _) => return format!("(!matches!({rational}.order_real({real}), Some(core::cmp::Ordering::Equal)))"),
        _ => unreachable!("an order test"),
    };
    format!("matches!({rational}.order_real({real}), {pattern})")
}

/// The operator `op` over `left` and `right` where one is a rational and the other an exact number or a real.
fn operator(op: BinaryOpKind, left: &Expr, right: &Expr, ecx: &ExprCtx) -> Option<String> {
    use BinaryOpKind::*;
    let t = Typing::of(ecx);
    if !mentions_rational(left, t) && !mentions_rational(right, t) {
        return None;
    }
    let (lt, rt) = (type_of(left, t), type_of(right, t));
    if lt != LogosType::Rational && rt != LogosType::Rational {
        return None;
    }
    let exact = |t: &LogosType| matches!(t, LogosType::Int | LogosType::Rational);
    if exact(&lt) && exact(&rt) {
        let method = match op {
            Add => "add",
            Subtract => "sub",
            Multiply => "mul",
            ExactDivide => "div_exact",
            Divide => "quotient",
            Modulo => "remainder",
            FloorDivide => "floor_quotient",
            Pow => "power",
            BitAnd => "bit_and",
            BitOr => "bit_or",
            BitXor => "bit_xor",
            Shl => "shl",
            Shr => "shr",
            Lt | Gt | LtEq | GtEq | Eq | NotEq => {
                let (l, r) = (rational_operand(left, &lt, ecx), rational_operand(right, &rt, ecx));
                let sym = match op {
                    Lt => "<",
                    Gt => ">",
                    LtEq => "<=",
                    GtEq => ">=",
                    Eq => "==",
                    _ => "!=",
                };
                return Some(format!("({l} {sym} {r})"));
            }
            ApproxEq => {
                let (l, r) = (rational_operand(left, &lt, ecx), rational_operand(right, &rt, ecx));
                return Some(format!("logos_approx_eq({l}.to_f64(), {r}.to_f64())"));
            }
            _ => return None,
        };
        let (l, r) = (rational_operand(left, &lt, ecx), rational_operand(right, &rt, ecx));
        return Some(format!("{l}.{method}(&{r})"));
    }
    if !matches!(
        (&lt, &rt),
        (LogosType::Rational, LogosType::Float) | (LogosType::Float, LogosType::Rational)
    ) {
        return None;
    }
    let (l, r) = (real_operand(left, &lt, ecx), real_operand(right, &rt, ecx));
    Some(match op {
        Add | Subtract | Multiply | Divide | ExactDivide => {
            let sym = match op {
                Add => "+",
                Subtract => "-",
                Multiply => "*",
                _ => "/",
            };
            format!("({l} {sym} {r})")
        }
        Lt | Gt | LtEq | GtEq | Eq | NotEq => {
            let real_left = lt == LogosType::Float;
            let (rational, real) = if real_left {
                (codegen_expr_ctx(right, ecx), codegen_expr_ctx(left, ecx))
            } else {
                (codegen_expr_ctx(left, ecx), codegen_expr_ctx(right, ecx))
            };
            order_test(op, &rational, &real, real_left)
        }
        ApproxEq => format!("logos_approx_eq({l}, {r})"),
        Modulo | FloorDivide | Pow => {
            let method = match op {
                Modulo => "rem",
                FloorDivide => "floor_div",
                _ => "pow",
            };
            let value = |e: &Expr| format!("logicaffeine_data::LogosValue::from({})", codegen_expr_ctx(e, ecx));
            format!(
                "logicaffeine_data::failure::answered({}.{method}(&{})).into_real()",
                value(left),
                value(right)
            )
        }
        _ => return None,
    })
}

/// The machine integer `code`, a rational, is where the language takes one at `place`, refused in the
/// interpreter's words when it is none.
pub(super) fn integer_at(code: &str, place: IntegerPlace) -> String {
    let (past_word, fraction) = integer_place_refusals(place);
    format!("{code}.integer({past_word:?}, {fraction:?})")
}

/// Whether `expr` holds a value of the exact extension (a `LogosValue`).
fn exact_value(expr: &Expr, ecx: &ExprCtx) -> bool {
    super::tower::active() && super::tower::is_tower_expr(expr, ecx.variable_types, ecx.interner)
}

/// Whether a place that takes a machine integer reads `expr` as an exact number converted to one: a rational, or a
/// value of the exact extension ([`integer_operand`]).
pub(super) fn read_as_exact(expr: &Expr, t: Typing) -> bool {
    (super::tower::active() && super::tower::is_tower_expr(expr, t.variable_types, t.interner))
        || (mentions_rational(expr, t) && type_of(expr, t) == LogosType::Rational)
}

/// The emission of `expr`, read where the language takes a machine integer at `place`: a rational, or a value of
/// the exact extension, as the integer it equals, refused in the interpreter's words for the place when it is none.
/// `None` for an expression that is neither.
pub(super) fn integer_operand(expr: &Expr, place: IntegerPlace, ecx: &ExprCtx) -> Option<String> {
    if exact_value(expr, ecx) {
        let (past_word, fraction) = integer_place_refusals(place);
        return Some(format!(
            "logicaffeine_data::failure::answered(({}).integer_at({past_word:?}, {fraction:?}))",
            super::tower::value(expr, ecx)
        ));
    }
    if !mentions_rational(expr, Typing::of(ecx)) || type_of(expr, Typing::of(ecx)) != LogosType::Rational {
        return None;
    }
    Some(integer_at(&codegen_expr_ctx(expr, ecx), place))
}

/// The machine integer the rational `index` names an item of the list `collection` by, at `place` (a read or a
/// store); `None` for an index that is no rational, and for a collection that is no list, whose key it is.
pub(super) fn item_index(collection: &Expr, index: &Expr, place: IntegerPlace, ecx: &ExprCtx) -> Option<String> {
    if !mentions_rational(index, Typing::of(ecx)) && !exact_value(index, ecx) {
        return None;
    }
    match type_of(collection, Typing::of(ecx)) {
        LogosType::Seq(_) | LogosType::String => integer_operand(index, place, ecx),
        _ => None,
    }
}

/// Emit `expr` where it computes with a rational: an operator over one, an item read at one, a membership test of a
/// list of rationals or of one, a list literal holding one beside integers. `None` for every other expression.
pub(super) fn emit(expr: &Expr, ecx: &ExprCtx) -> Option<String> {
    let vt = ecx.variable_types;
    let t = Typing::of(ecx);
    match expr {
        Expr::BinaryOp { op, left, right } => operator(*op, left, right, ecx),
        Expr::Index { collection, index } if mentions_rational(index, t) || exact_value(index, ecx) => {
            let place = match type_of(collection, t) {
                LogosType::String => IntegerPlace::Text,
                _ => IntegerPlace::ListRead,
            };
            let at = item_index(collection, index, place, ecx)?;
            Some(format!("LogosIndex::logos_get(&{}, {at})", codegen_expr_ctx(collection, ecx)))
        }
        Expr::Contains { collection, value } => {
            if !mentions_rational(collection, t) && !mentions_rational(value, t) {
                return None;
            }
            let (ct, item) = (type_of(collection, t), type_of(value, t));
            let LogosType::Seq(held) = &ct else { return None };
            let list = LogosType::Seq(Box::new(LogosType::number_join(held, &item)?));
            if !list.holds_rational() || (**held == item) {
                return None;
            }
            let coll = codegen_expr_ctx(collection, ecx);
            let needle = codegen_expr_ctx(value, ecx);
            let coll = converted(collection, &coll, &ct, &list, vt).unwrap_or(coll);
            let LogosType::Seq(joined) = &list else { unreachable!("a list") };
            let needle = converted(value, &needle, &item, joined, vt).unwrap_or(needle);
            Some(format!("{coll}.logos_contains(&{needle})"))
        }
        Expr::List(items) if items.iter().any(|i| mentions_rational(i, t)) => {
            let list = type_of(expr, t);
            value_as(expr, &list, ecx)
        }
        _ => None,
    }
}

/// What a scope binds a name to: a value it is set to, or an item pushed onto it or stored into it.
enum Given<'e, 'a> {
    Value(&'e Expr<'a>),
    Item(&'e Expr<'a>),
}

/// The rounds a scope's bindings take to hold still: each round only widens a binding's type.
const SETTLE_ROUNDS: usize = 64;

/// The bindings of a scope's `body` an exact join holds as rationals, by name, with the Rust type each is declared:
/// a name bound by one untyped `Let` whose values — what it is set to, what is pushed onto it or stored into it — are
/// integers and rationals, or lists of them, holds the rational type. `given` names the scope's parameters, typed in
/// `variable_types`. Each round types every binding of the scope (its declared type, the join of its values, a loop
/// variable's item) under the types the last round settled, so a value read before its binding's join is known gives
/// way to it.
pub(super) fn joins(body: &[Stmt], given: &HashSet<Symbol>, t: Typing) -> HashMap<Symbol, String> {
    let interner = t.interner;
    let mut values: HashMap<Symbol, Vec<Given>> = HashMap::new();
    let mut lets: HashMap<Symbol, usize> = HashMap::new();
    let mut declared: HashMap<Symbol, LogosType> = HashMap::new();
    let mut other: HashSet<Symbol> = given.clone();
    let mut loops: Vec<(Symbol, &Expr)> = Vec::new();
    walk_nodes(body, |node| {
        match node {
            Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => return Walk::Past,
            Visited::Stmt(Stmt::Let { var, value, ty: None, .. }) => {
                *lets.entry(*var).or_default() += 1;
                values.entry(*var).or_default().push(Given::Value(value));
            }
            Visited::Stmt(Stmt::Let { var, ty: Some(ty), .. }) => {
                let t = LogosType::from_type_expr(ty, interner);
                if t != LogosType::Unknown {
                    declared.insert(*var, t);
                }
                other.insert(*var);
            }
            Visited::Stmt(Stmt::Set { target, value }) => values.entry(*target).or_default().push(Given::Value(value)),
            Visited::Stmt(Stmt::Push { value, collection: Expr::Identifier(list) }) => {
                values.entry(*list).or_default().push(Given::Item(value));
            }
            Visited::Stmt(Stmt::SetIndex { collection: Expr::Identifier(list), value, .. }) => {
                values.entry(*list).or_default().push(Given::Item(value));
            }
            Visited::Stmt(Stmt::Repeat { pattern, iterable, .. }) => match pattern {
                Pattern::Identifier(x) => {
                    loops.push((*x, iterable));
                    other.insert(*x);
                }
                Pattern::Tuple(xs) => other.extend(xs.iter().copied()),
            },
            Visited::Stmt(Stmt::Pop { into: Some(v), .. }) => {
                other.insert(*v);
            }
            _ => {}
        }
        Walk::Into
    });
    values.retain(|var, _| lets.get(var) == Some(&1) && !other.contains(var));
    if values.is_empty() {
        return HashMap::new();
    }

    let mut typed: HashMap<Symbol, LogosType> = HashMap::new();
    let mut joined: HashMap<Symbol, LogosType> = HashMap::new();
    for _ in 0..SETTLE_ROUNDS {
        let mut types = t.variable_types.clone();
        types.extend(declared.iter().chain(&typed).map(|(k, t)| (*k, t.to_rust_type())));
        for (x, iterable) in &loops {
            let walked = type_of(iterable, t.over(&types));
            if let LogosType::Seq(item) = walked {
                if *item != LogosType::Unknown {
                    types.insert(*x, item.to_rust_type());
                }
            }
        }
        let mut next_typed: HashMap<Symbol, LogosType> = HashMap::new();
        let mut next_joined: HashMap<Symbol, LogosType> = HashMap::new();
        for (var, given) in &values {
            let each: Vec<LogosType> = given
                .iter()
                .map(|g| match g {
                    Given::Value(e) => type_of(e, t.over(&types)),
                    Given::Item(e) => LogosType::Seq(Box::new(type_of(e, t.over(&types)))),
                })
                .collect();
            let Some(first) = each.first() else { continue };
            let Some(join) = each.iter().try_fold(first.clone(), |a, b| LogosType::number_join(&a, b)) else {
                continue;
            };
            if join == LogosType::Unknown {
                continue;
            }
            if join.holds_rational() && each.iter().any(|t| *t != join) {
                next_joined.insert(*var, join.clone());
            }
            next_typed.insert(*var, join);
        }
        let settled = next_typed == typed && next_joined == joined;
        typed = next_typed;
        joined = next_joined;
        if settled {
            break;
        }
    }
    joined.into_iter().map(|(var, t)| (var, t.to_rust_type())).collect()
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen/rational/tests.rs"]
mod tests;
