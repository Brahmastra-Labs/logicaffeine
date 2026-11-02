//! The exact extension in generated Rust. A binding, parameter or result the extension reaches
//! ([`crate::analysis::tower`]) is a `LogosValue`, the compiled value model that keeps every value
//! in its canonical form, so it computes and prints what the interpreter does; an expression over
//! one is emitted on that model, each operation answering through
//! `logicaffeine_data::failure::answered`, which stops with the interpreter's own sentence. A
//! program the extension never reaches emits exactly what it did before.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use logicaffeine_base::extension::Builtin;

use super::expr::ExprCtx;
use crate::analysis::tower::{Scope, TowerTypes};
use crate::analysis::types::RustNames;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};

/// The Rust type of a reached binding, parameter or result.
pub(crate) const VALUE: &str = "LogosValue";

thread_local! {
    static TOWER: RefCell<Option<Rc<TowerTypes>>> = const { RefCell::new(None) };
    static SCOPE: Cell<Scope> = const { Cell::new(None) };
}

/// Read the program's reach before a line of it is emitted; it holds until the returned guard
/// drops at the end of that program's emission, so no other compile can read it.
pub(crate) fn begin_program(stmts: &[Stmt], interner: &Interner) -> ProgramGuard {
    let reach = TowerTypes::of_program(stmts, interner).map(Rc::new);
    ProgramGuard {
        tower: TOWER.with(|t| t.replace(reach)),
        scope: SCOPE.with(|s| s.replace(None)),
    }
}

/// Restores the reach and scope that held before [`begin_program`].
pub(crate) struct ProgramGuard {
    tower: Option<Rc<TowerTypes>>,
    scope: Scope,
}

impl Drop for ProgramGuard {
    fn drop(&mut self) {
        TOWER.with(|t| *t.borrow_mut() = self.tower.take());
        SCOPE.with(|s| s.set(self.scope));
    }
}

/// Emit inside `scope` (a function, or the program's root) until the returned guard drops.
pub(crate) fn enter_scope(scope: Scope) -> ScopeGuard {
    ScopeGuard(SCOPE.with(|s| s.replace(scope)))
}

/// Restores the scope that was current when [`enter_scope`] was called.
pub(crate) struct ScopeGuard(Scope);

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        SCOPE.with(|s| s.set(self.0));
    }
}

fn with<R>(f: impl FnOnce(&TowerTypes) -> R) -> Option<R> {
    TOWER.with(|t| t.borrow().as_deref().map(f))
}

/// Whether the extension reaches the program at all.
pub(crate) fn active() -> bool {
    with(|_| ()).is_some()
}

/// Whether `sym`, named in the scope being emitted, holds a `LogosValue`.
pub(crate) fn binding(sym: Symbol) -> bool {
    let scope = SCOPE.with(Cell::get);
    with(|t| t.binding(scope, sym)).unwrap_or(false)
}

/// Whether parameter `param` of function `f` is a `LogosValue`.
pub(crate) fn param(f: Symbol, param: Symbol) -> bool {
    with(|t| t.binding(Some(f), param)).unwrap_or(false)
}

/// Whether the `i`th parameter of function `f` is a `LogosValue`.
pub(crate) fn param_at(f: Symbol, i: usize) -> bool {
    with(|t| t.param_at(f, i)).unwrap_or(false)
}

/// Whether the function being emitted returns a `LogosValue`.
pub(crate) fn emitting_a_reached_function() -> bool {
    SCOPE.with(Cell::get).is_some_and(returns)
}

/// Whether function `f` returns a `LogosValue`.
pub(crate) fn returns(f: Symbol) -> bool {
    with(|t| t.returns(f)).unwrap_or(false)
}

/// Whether any of `stmts` names a binding the extension reaches in the scope being emitted: the
/// statements a pattern lowering written for machine integers must leave to the general emission.
pub(crate) fn mentions_reached(stmts: &[&Stmt]) -> bool {
    use logicaffeine_language::ast_depth::{walk_nodes, Visited, Walk};
    if !active() {
        return false;
    }
    let mut found = false;
    for s in stmts {
        walk_nodes(std::slice::from_ref(*s), |node| {
            if found {
                return Walk::Past;
            }
            match node {
                Visited::Expr(Expr::Identifier(sym)) => found = binding(*sym),
                Visited::Stmt(Stmt::Let { var, .. }) => found = binding(*var),
                Visited::Stmt(Stmt::Set { target, .. }) => found = binding(*target),
                Visited::Stmt(Stmt::FunctionDef { .. }) => return Walk::Past,
                _ => {}
            }
            Walk::Into
        });
        if found {
            return true;
        }
    }
    false
}

/// The builtin of the extension a call names, unless the program declares a function of that name.
fn builtin(function: Symbol, interner: &Interner) -> Option<Builtin> {
    if with(|t| t.declares(function)).unwrap_or(true) {
        return None;
    }
    Builtin::named(interner.resolve(function))
}

/// `definedReal`, the builtin a real number a program's function defines is made by, where the
/// program does not define that name itself.
fn definer(function: Symbol, interner: &Interner) -> bool {
    !with(|t| t.declares(function)).unwrap_or(true)
        && crate::analysis::tower::DEFINERS.contains(&interner.resolve(function))
}

/// The builtins that round, measure or combine a number in its own kind.
fn lifter(function: Symbol, interner: &Interner) -> Option<&'static str> {
    if with(|t| t.declares(function)).unwrap_or(true) {
        return None;
    }
    let name = interner.resolve(function);
    crate::analysis::tower::LIFTING_BUILTINS.iter().copied().find(|l| *l == name)
}

/// Whether `expr` is a `LogosValue` in generated code: the analysis's reach, read against the types
/// the emitter has given the bindings in view.
pub(crate) fn is_tower_expr(
    expr: &Expr,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> bool {
    if !active() {
        return false;
    }
    let rec = |e: &Expr| is_tower_expr(e, variable_types, interner);
    match expr {
        Expr::Identifier(s) => variable_types.get(s).is_some_and(|t| t == VALUE),
        Expr::BinaryOp { op, left, right } => {
            !matches!(op, BinaryOpKind::And | BinaryOpKind::Or | BinaryOpKind::Concat)
                && (rec(left) || rec(right))
        }
        Expr::Not { operand } => rec(operand),
        Expr::Call { function, args } => {
            builtin(*function, interner).is_some()
                || definer(*function, interner)
                || returns(*function)
                || (lifter(*function, interner).is_some() && args.iter().any(|a| rec(a)))
        }
        Expr::Index { collection, .. } => rec(collection),
        Expr::List(items) => items.iter().any(|i| rec(i)),
        Expr::FieldAccess { object, field } => {
            let object = match object {
                Expr::Copy { expr } => *expr,
                other => other,
            };
            let Expr::Identifier(record) = object else {
                return false;
            };
            variable_types
                .get(record)
                .and_then(|ty| {
                    let base = ty.split("|__hl:").next().unwrap_or(ty).trim_start_matches('&');
                    super::types::program_record_field_type(base, *field)
                })
                .is_some_and(|ty| ty == VALUE)
        }
        _ => false,
    }
}

/// `a and b` / `a or b` with an operand of the extension: each side's decided truth, the right
/// one read only when the left does not settle the answer, as the interpreter reads them.
pub(crate) fn emit_logic(expr: &Expr, ecx: &ExprCtx) -> Option<String> {
    let Expr::BinaryOp { op, left, right } = expr else {
        return None;
    };
    let joiner = match op {
        BinaryOpKind::And => "&&",
        BinaryOpKind::Or => "||",
        _ => return None,
    };
    let reached = |e: &Expr| is_tower_expr(e, ecx.variable_types, ecx.interner);
    if !reached(left) && !reached(right) {
        return None;
    }
    let side = |e: &Expr| format!("logos_truthy(&({}))", super::expr::codegen_expr_ctx(e, ecx));
    Some(format!("({} {joiner} {})", side(left), side(right)))
}

/// `expr` as a `LogosValue`: emitted on the model when it is one, converted from the Rust value the
/// emitter gives it otherwise.
pub(crate) fn value(expr: &Expr, ecx: &ExprCtx) -> String {
    if is_tower_expr(expr, ecx.variable_types, ecx.interner) {
        return super::expr::codegen_expr_ctx(expr, ecx);
    }
    match expr {
        Expr::Literal(Literal::Number(n)) => format!("LogosValue::Int({n}_i64)"),
        Expr::List(items) => {
            let items: Vec<String> = items.iter().map(|i| value(i, ecx)).collect();
            format!("LogosValue::list(vec![{}])", items.join(", "))
        }
        Expr::Identifier(sym) => format!(
            "LogosValue::from({}.clone())",
            RustNames::new(ecx.interner).ident(*sym)
        ),
        _ => format!("LogosValue::from({})", super::expr::codegen_expr_ctx(expr, ecx)),
    }
}

/// `length of c` and `c contains v` where `c` is a `LogosValue`: an `Int` and a `Bool` as the
/// emitter types them, read on the model.
pub(crate) fn emit_collection(expr: &Expr, ecx: &ExprCtx) -> Option<String> {
    let reached = |e: &Expr| is_tower_expr(e, ecx.variable_types, ecx.interner);
    match expr {
        Expr::Length { collection } if reached(collection) => Some(answered(format!(
            "({}).length_of()",
            super::expr::codegen_expr_ctx(collection, ecx)
        ))),
        Expr::Contains { collection, value: v } if reached(collection) => Some(answered(format!(
            "({}).contains_value(&({}))",
            super::expr::codegen_expr_ctx(collection, ecx),
            value(v, ecx)
        ))),
        Expr::Slice { collection, start, end } if reached(collection) => Some(answered(format!(
            "({}).slice(&({}), &({}))",
            super::expr::codegen_expr_ctx(collection, ecx),
            value(start, ecx),
            value(end, ecx)
        ))),
        _ => None,
    }
}

/// A builtin of the extension called as a statement (`At the limit, take …`).
pub(crate) fn extension_call(function: Symbol, args: &[&Expr], ecx: &ExprCtx) -> Option<String> {
    builtin(function, ecx.interner)?;
    emit(
        &Expr::Call {
            function,
            args: args.to_vec(),
        },
        ecx,
    )
}

/// [`value`] in the context a statement emits its values in.
pub(crate) fn statement_value(
    expr: &Expr,
    interner: &Interner,
    synced_vars: &std::collections::HashSet<Symbol>,
    async_functions: &std::collections::HashSet<Symbol>,
    boxed_fields: &std::collections::HashSet<(String, String, String)>,
    registry: &crate::analysis::registry::TypeRegistry,
    variable_types: &HashMap<Symbol, String>,
) -> String {
    value(
        expr,
        &ExprCtx {
            async_functions,
            boxed_fields,
            registry,
            variable_types,
            ..ExprCtx::bare(interner, synced_vars)
        },
    )
}

/// [`value`] where only the binding types and async functions are in hand (the tail-call emitters).
pub(crate) fn bare_value(
    expr: &Expr,
    interner: &Interner,
    synced_vars: &std::collections::HashSet<Symbol>,
    async_functions: &std::collections::HashSet<Symbol>,
    variable_types: &HashMap<Symbol, String>,
) -> String {
    value(
        expr,
        &ExprCtx {
            async_functions,
            variable_types,
            ..ExprCtx::bare(interner, synced_vars)
        },
    )
}

/// `acc op v` on the model, for a function's accumulator: `+` or `×`.
pub(crate) fn accumulate(acc: &str, op: BinaryOpKind, v: &str) -> String {
    let method = match op {
        BinaryOpKind::Add => "add",
        BinaryOpKind::Multiply => "mul",
        other => unreachable!("an accumulator folds by `+` or `*`, not {other:?}"),
    };
    answered(format!("({acc}).{method}(&({v}))"))
}

fn answered(code: String) -> String {
    format!("logicaffeine_data::failure::answered({code})")
}

/// `definedReal(kind, base, name, f)` as the compiled value it makes: the closure `f` compiles as
/// any closure does, and each answer it gives is read as a value of the model.
fn defined_real(args: &[&Expr], ecx: &ExprCtx) -> String {
    let [Expr::Literal(Literal::Text(kind)), Expr::Literal(Literal::Number(base)), Expr::Literal(Literal::Text(name)), f] =
        args
    else {
        unreachable!("the checker admits definedReal only as its phrase writes it")
    };
    let text = |s: &Symbol| format!("{:?}", ecx.interner.resolve(*s));
    let f = super::expr::codegen_expr_ctx(f, ecx);
    format!(
        "logicaffeine_data::exact::LogosValue::defined_real({}, {base}_i64, {}, {{ let f = {f}; move |n: i64| \
         logicaffeine_data::exact::LogosValue::from(f(n)) }})",
        text(kind),
        text(name)
    )
}

/// A launched task's argument `arg`, the emission of `expr`: a value of the model crosses to the task only
/// when it may, the program stopping with the interpreter's sentence when it holds a real number a
/// function of the program defines.
pub(crate) fn crossing_to_a_task(
    expr: &Expr,
    arg: String,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> String {
    if !is_tower_expr(expr, variable_types, interner) {
        return arg;
    }
    format!("{{ let crossing = {arg}; logicaffeine_data::failure::answered(crossing.sendable()); crossing }}")
}

/// The emission of a reached expression, or `None` for a call of the program's own function, which
/// the general call emission writes with its arguments converted to the parameters' types.
pub(crate) fn emit(expr: &Expr, ecx: &ExprCtx) -> Option<String> {
    let v = |e: &Expr| value(e, ecx);
    Some(match expr {
        Expr::Identifier(sym) => format!("{}.clone()", RustNames::new(ecx.interner).ident(*sym)),
        Expr::BinaryOp { op, left, right } => {
            let (l, r) = (v(left), v(right));
            let method = |m: &str| answered(format!("({l}).{m}(&({r}))"));
            let relation = |rel: &str| {
                answered(format!("({l}).relation(logicaffeine_data::exact::Rel::{rel}, &({r}))"))
            };
            match op {
                BinaryOpKind::Add => method("add"),
                BinaryOpKind::Subtract => method("sub"),
                BinaryOpKind::Multiply => method("mul"),
                BinaryOpKind::Divide => method("div"),
                BinaryOpKind::ExactDivide => method("exact_div"),
                BinaryOpKind::FloorDivide => method("floor_div"),
                BinaryOpKind::Modulo => method("rem"),
                BinaryOpKind::Pow => method("pow"),
                BinaryOpKind::Eq => relation("Eq"),
                BinaryOpKind::NotEq => relation("NotEq"),
                BinaryOpKind::Lt => relation("Lt"),
                BinaryOpKind::Gt => relation("Gt"),
                BinaryOpKind::LtEq => relation("LtEq"),
                BinaryOpKind::GtEq => relation("GtEq"),
                BinaryOpKind::ApproxEq => method("approx_eq"),
                BinaryOpKind::SeqConcat => method("seq_concat"),
                BinaryOpKind::BitAnd => method("bit_and"),
                BinaryOpKind::BitOr => method("bit_or"),
                BinaryOpKind::BitXor => method("bit_xor"),
                BinaryOpKind::Shl => method("shl"),
                BinaryOpKind::Shr => method("shr"),
                BinaryOpKind::And | BinaryOpKind::Or | BinaryOpKind::Concat => {
                    unreachable!("`and`, `or` and `combined with` reach nothing")
                }
            }
        }
        Expr::Not { operand } => answered(format!("({}).not()", v(operand))),
        Expr::Call { function, args } => {
            if definer(*function, ecx.interner) {
                return Some(defined_real(args, ecx));
            }
            if let Some(b) = builtin(*function, ecx.interner) {
                let args: Vec<String> = args.iter().map(|a| v(a)).collect();
                return Some(format!(
                    "logicaffeine_data::exact::logos_extension(logicaffeine_data::exact::ExtensionBuiltin::{:?}, vec![{}])",
                    b,
                    args.join(", ")
                ));
            }
            let lifted = lifter(*function, ecx.interner)?;
            let first = v(args[0]);
            let method = match lifted {
                "floor" | "ceil" | "round" | "abs" | "sqrt" => format!("({first}).{lifted}()"),
                "min" | "max" => format!("({first}).{lifted}(&({}))", v(args[1])),
                "pow" => format!("({first}).power(&({}))", v(args[1])),
                other => unreachable!("`{other}` is not a lifting builtin"),
            };
            answered(method)
        }
        Expr::Index { collection, index } => {
            answered(format!("({}).item(&({}))", v(collection), v(index)))
        }
        Expr::List(items) => {
            let items: Vec<String> = items.iter().map(|i| v(i)).collect();
            format!("LogosValue::list(vec![{}])", items.join(", "))
        }
        _ => return None,
    })
}
