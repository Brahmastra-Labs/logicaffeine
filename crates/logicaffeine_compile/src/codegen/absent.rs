//! The Rust storage of what a stored `nothing` reaches ([`crate::analysis::absence`]).
//!
//! A binding, parameter, result or field that can itself be `nothing` holds a `LogosOpt` (`logicaffeine_data::opt`):
//! present, the value it would hold anyway; absent, `nothing`. A list or a map whose items can be `nothing` holds
//! `LogosOpt`s, at whatever depth of its items they can be. A place that can never be `nothing` keeps the type it has
//! without this module, and a program with no such place emits exactly what it did before.
//!
//! A binding's Rust type names its storage, so every rule of the emitter that looks a binding up sees it. The language
//! has no type of its own for it: a `LogosOpt<T>` is a `T` to every inference ([`LogosType::from_rust_type_str`]).
//! Where an operation needs a value, it reads its operand as the value the operand holds ([`held`], [`held_mut`],
//! [`binary`]) and stops, when it holds `nothing`, in the words the VM stops with for that operation on `nothing`.
//! Everywhere else a value-or-nothing is read whole: shown, compared, copied, passed and returned as it is.
//!
//! [`LogosType::from_rust_type_str`]: crate::analysis::types::LogosType::from_rust_type_str

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::expr::ExprCtx;
use crate::analysis::absence::{Absence, Depths};
use crate::analysis::types::{LogosType, RustNames, TypeEnv};
use crate::analysis::TypeRegistry;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use crate::interpreter::RuntimeValue;
use crate::intern::{Interner, Symbol};

/// The head of the Rust type that holds a value or `nothing`.
pub(super) const OPT: &str = "LogosOpt";

/// The names the two operands of an operation on values-or-nothing are read into ([`binary`]).
const OPERANDS: [&str; 2] = ["__held_left", "__held_right"];

/// One program's absence, as this emission reads it.
struct Program {
    absence: Absence,
    types: TypeEnv,
    /// What each function returns when its result holds a value-or-nothing.
    returns: HashMap<Symbol, String>,
    /// Each function's parameters that hold a value-or-nothing, by position: the type, and whether it is lent.
    parameters: HashMap<Symbol, Vec<Option<(String, bool)>>>,
    operands: [Symbol; 2],
}

thread_local! {
    static ABSENCE: RefCell<Option<Rc<Program>>> = const { RefCell::new(None) };
    /// The result type of the function being emitted, when it holds a value-or-nothing.
    static RETURNING: RefCell<Option<String>> = const { RefCell::new(None) };
    /// Operands an enclosing operation has read already, each by the Rust it is read as.
    static READ: RefCell<HashMap<usize, String>> = RefCell::new(HashMap::new());
}

/// Where a stored `nothing` reaches in the program `stmts`, when anything can hold one. A program whose absence is not
/// empty interns the operand names in `interner`, which must then be the emission's own.
pub(super) fn of_program(stmts: &[Stmt], registry: &TypeRegistry, interner: &Interner) -> Option<(Absence, TypeEnv)> {
    let types = TypeEnv::infer_program(stmts, interner, registry);
    let absence = crate::analysis::absence::of_program(stmts, &types, interner);
    (!absence.is_empty()).then_some((absence, types))
}

/// Intern the names [`binary`] reads operands into.
pub(super) fn intern_operands(interner: &mut Interner) {
    for name in OPERANDS {
        interner.intern(name);
    }
}

/// Emit the program `stmts` with `found` ([`of_program`]) until the returned guard drops.
pub(super) fn begin_program(
    found: Option<(Absence, TypeEnv)>,
    stmts: &[Stmt],
    interner: &Interner,
) -> ProgramGuard {
    let program = found.map(|(absence, types)| {
        let operands = OPERANDS.map(|name| interner.lookup(name).expect("operand names are interned"));
        let mut program =
            Program { absence, types, returns: HashMap::new(), parameters: HashMap::new(), operands };
        for stmt in stmts {
            if let Stmt::FunctionDef { name, params, return_type, .. } = stmt {
                if let Some(depths) = program.absence.returned(*name) {
                    let declared = return_type.map(|ty| super::types::codegen_type_expr(ty, interner));
                    let inferred = || {
                        let sig = program.types.lookup_fn(*name)?;
                        super::types::logos_return_type_to_rust(&sig.return_type, interner)
                    };
                    if let Some(ty) = declared.or_else(inferred) {
                        program.returns.insert(*name, at_depth(&ty, depths, 0));
                    }
                }
                let held: Vec<Option<(String, bool)>> = params
                    .iter()
                    .map(|(param, ty)| {
                        let depths = program.absence.parameter(*name, *param)?;
                        let held = at_depth(&super::types::codegen_type_expr(ty, interner), depths, 0);
                        Some((held, matches!(ty, TypeExpr::Mutable { .. })))
                    })
                    .collect();
                if held.iter().any(Option::is_some) {
                    program.parameters.insert(*name, held);
                }
            }
        }
        Rc::new(program)
    });
    ProgramGuard(ABSENCE.with(|p| p.replace(program)))
}

/// Restores the absence that held before [`begin_program`].
pub(super) struct ProgramGuard(Option<Rc<Program>>);

impl Drop for ProgramGuard {
    fn drop(&mut self) {
        ABSENCE.with(|p| *p.borrow_mut() = self.0.take());
    }
}

fn program() -> Option<Rc<Program>> {
    ABSENCE.with(|p| p.borrow().clone())
}

/// Emit the body of `function` until the returned guard drops.
pub(super) fn enter_function(function: Symbol) -> FunctionGuard {
    let returning = program().and_then(|p| p.returns.get(&function).cloned());
    FunctionGuard(RETURNING.with(|r| r.replace(returning)))
}

/// Restores the function that was being emitted before [`enter_function`].
pub(super) struct FunctionGuard(Option<String>);

impl Drop for FunctionGuard {
    fn drop(&mut self) {
        RETURNING.with(|r| *r.borrow_mut() = self.0.take());
    }
}

/// The type the function being emitted returns, when its result holds a value-or-nothing.
pub(super) fn returning() -> Option<String> {
    RETURNING.with(|r| r.borrow().clone())
}

/// The head of the Rust type `ty` and its type arguments, split at its own commas: `LogosMap<String, LogosSeq<i64>>`
/// is `LogosMap` with `String` and `LogosSeq<i64>`. `None` for a type with no arguments.
pub(super) fn generic(ty: &str) -> Option<(&str, Vec<&str>)> {
    let ty = ty.trim();
    let open = ty.find('<')?;
    let inner = ty.strip_suffix('>')?.get(open + 1..)?;
    let mut args = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    for (at, c) in inner.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                args.push(inner[start..at].trim());
                start = at + 1;
            }
            _ => {}
        }
    }
    args.push(inner[start..].trim());
    Some((&ty[..open], args))
}

/// `ty`, the Rust type of a place, as the storage of a value that can be `nothing` at `depths` below `depth`: a
/// `LogosOpt` at depth 0, a list or a map of them at depth 1, and so on down its items. A language `Option` already
/// holds `nothing` as its `none`.
pub(super) fn at_depth(ty: &str, depths: &Depths, depth: usize) -> String {
    if depths.iter().all(|d| *d < depth) {
        return ty.to_string();
    }
    let held = match generic(ty) {
        Some((OPT | "Option", _)) => return ty.to_string(),
        Some((head @ ("LogosSeq" | "Seq" | "Vec"), args)) if args.len() == 1 => {
            format!("{head}<{}>", at_depth(args[0], depths, depth + 1))
        }
        Some((head @ ("LogosMap" | "Map"), args)) if args.len() == 2 => {
            format!("{head}<{}, {}>", args[0], at_depth(args[1], depths, depth + 1))
        }
        _ => ty.to_string(),
    };
    if depths.contains(&depth) {
        format!("{OPT}<{held}>")
    } else {
        held
    }
}

/// Whether a value of the Rust type `ty` is itself a value-or-nothing.
pub(super) fn holds(ty: &str) -> bool {
    payload(ty).is_some()
}

/// The type of the value a value-or-nothing of Rust type `ty` holds.
pub(super) fn payload(ty: &str) -> Option<&str> {
    ty.trim_start_matches('&').trim_start_matches("mut ").strip_prefix(OPT)?.strip_prefix('<')?.strip_suffix('>')
}

/// The type of an item of a list or a value of a map of Rust type `ty`.
fn item(ty: &str) -> Option<&str> {
    match generic(ty.trim_start_matches('&').trim_start_matches("mut "))? {
        ("LogosSeq" | "Seq" | "Vec", args) if args.len() == 1 => Some(args[0]),
        ("LogosMap" | "Map", args) if args.len() == 2 => Some(args[1]),
        _ => None,
    }
}

/// Whether the Rust type `ty` holds a value-or-nothing, itself or at any depth.
fn holds_anywhere(ty: &str) -> bool {
    ty.contains(OPT)
}

/// The Rust type of what the `Let` statement `stmt` binds to `value`, when it can hold `nothing`: its declared type
/// `declared`, or the type of the value that flows into it, as the storage its depths call for.
pub(super) fn binding(
    stmt: &Stmt,
    declared: Option<&TypeExpr>,
    value: &Expr,
    ecx: &ExprCtx,
) -> Option<String> {
    let program = program()?;
    let depths = program.absence.binding(stmt)?;
    let base = match declared {
        Some(ty) => super::types::codegen_type_expr(ty, ecx.interner),
        None => type_of(value, ecx).unwrap_or_else(|| rust_type(&program.types.infer_expr(value, ecx.interner), ecx)),
    };
    Some(at_depth(&base, depths, 0))
}

/// The Rust type of `function`'s parameter `param`, `ty` being the type it has otherwise.
pub(super) fn parameter(function: Symbol, param: Symbol, ty: &str) -> Option<String> {
    let program = program()?;
    program.absence.parameter(function, param).map(|depths| at_depth(ty, depths, 0))
}

/// Argument `i` of a call to `function`, when the parameter it goes to holds a value-or-nothing: the caller's own
/// lent to a `mutable` one, else the value stored as the parameter holds it.
pub(super) fn argument(function: Symbol, i: usize, arg: &Expr, ecx: &ExprCtx) -> Option<String> {
    let program = program()?;
    let (ty, lent) = program.parameters.get(&function)?.get(i)?.as_ref()?;
    if *lent {
        // A place is lent where it is; any other value is a temporary the call borrows for its own.
        return Some(match place(arg, ecx) {
            Some(at) => format!("&mut {at}"),
            None => format!("&mut {}", stored(arg, ty, ecx).unwrap_or_else(|| whole(arg, ecx))),
        });
    }
    Some(stored(arg, ty, ecx).unwrap_or_else(|| whole(arg, ecx)))
}

/// The Rust type of what `function` returns, when its result can hold `nothing`.
pub(super) fn returned(function: Symbol) -> Option<String> {
    program()?.returns.get(&function).cloned()
}

/// The Rust type of field `field` of the record `owner`, `ty` being its declared type.
pub(super) fn field(owner: Symbol, field: Symbol, ty: String) -> String {
    match program().and_then(|p| p.absence.field(owner, field)) {
        Some(depths) => at_depth(&ty, &depths, 0),
        None => ty,
    }
}

/// The Rust type a language type is held as where it can hold `nothing`.
fn rust_type(ty: &LogosType, ecx: &ExprCtx) -> String {
    match ty {
        LogosType::UserDefined(name) => super::types::rust_type_ident(ecx.interner.resolve(*name)),
        LogosType::Seq(inner) => format!("LogosSeq<{}>", rust_type(inner, ecx)),
        LogosType::Map(key, value) => format!("LogosMap<{}, {}>", rust_type(key, ecx), rust_type(value, ecx)),
        other => other.to_rust_type(),
    }
}

/// The Rust type of `expr` when it holds a value-or-nothing, itself or at any depth.
pub(super) fn type_of(expr: &Expr, ecx: &ExprCtx) -> Option<String> {
    place_type(expr, ecx).filter(|ty| holds_anywhere(ty))
}

/// The Rust type of the value `expr` names: a binding's, a field's (a generic record's with its type arguments, a
/// value-or-nothing record's of the record it holds), an item's of the list or map it is in, a call's result.
fn place_type(expr: &Expr, ecx: &ExprCtx) -> Option<String> {
    if read(expr).is_some() {
        return None;
    }
    match expr {
        Expr::Identifier(sym) => ecx.variable_types.get(sym).map(|t| {
            t.split("|__hl:").next().unwrap_or(t).trim_start_matches('&').trim_start_matches("mut ").to_string()
        }),
        Expr::FieldAccess { object, field } => {
            super::expr::record_field_rust_type(expr, ecx.interner, ecx.variable_types, ecx.registry).or_else(|| {
                let record = place_type(object, ecx)?;
                field_type(payload(&record).unwrap_or(&record), *field, ecx)
            })
        }
        Expr::Index { collection, .. } => {
            let list = place_type(collection, ecx)?;
            item(payload(&list).unwrap_or(&list)).map(str::to_string)
        }
        Expr::Call { function, .. } => returned(*function),
        Expr::Copy { expr } | Expr::Give { value: expr } => place_type(expr, ecx),
        _ => None,
    }
}

/// Whether `expr` is a value-or-nothing itself.
pub(super) fn is_held(expr: &Expr, ecx: &ExprCtx) -> bool {
    type_of(expr, ecx).is_some_and(|t| holds(&t))
}

/// The Rust an enclosing operation reads `expr` as, once it has read the value-or-nothing `expr` names.
pub(super) fn read(expr: &Expr) -> Option<String> {
    READ.with(|r| r.borrow().get(&(expr as *const Expr as usize)).cloned())
}

/// `value` as the Rust value a place of type `target` stores, when `target` holds a value-or-nothing: `nothing`
/// itself, a value-or-nothing whole (a copy of its own), any other value present. `None` when `target` holds none.
pub(super) fn stored(value: &Expr, target: &str, ecx: &ExprCtx) -> Option<String> {
    if !holds_anywhere(target) {
        return None;
    }
    let target = target.trim_start_matches('&').trim_start_matches("mut ");
    if let Expr::Literal(Literal::Nothing) | Expr::OptionNone = value {
        if holds(target) {
            return Some(format!("logicaffeine_data::LogosOpt::<{}>::NOTHING", payload(target)?));
        }
    }
    if type_of(value, ecx).is_some() {
        return Some(whole(value, ecx));
    }
    if let Some(inner) = payload(target) {
        let present = stored(value, inner, ecx).unwrap_or_else(|| plain(value, inner, ecx));
        return Some(format!("logicaffeine_data::LogosOpt::from({present})"));
    }
    match (value, generic(target)) {
        (Expr::New { init_fields, .. }, Some(_)) if init_fields.is_empty() => Some(format!("<{target}>::default()")),
        (Expr::List(items), Some(("LogosSeq" | "Seq", args))) if args.len() == 1 => {
            let items: Vec<String> =
                items.iter().map(|i| stored(i, args[0], ecx).unwrap_or_else(|| plain(i, args[0], ecx))).collect();
            Some(format!("LogosSeq::from_vec(vec![{}])", items.join(", ")))
        }
        _ => None,
    }
}

/// `value`, a value of no value-or-nothing, as one of Rust type `ty`.
fn plain(value: &Expr, ty: &str, ecx: &ExprCtx) -> String {
    let code = super::expr::codegen_expr_ctx(value, ecx);
    match (ty, value) {
        ("f64", Expr::Literal(Literal::Number(_))) => format!("({code} as f64)"),
        ("String", Expr::Literal(Literal::Text(_))) => format!("String::from({code})"),
        _ => super::stmt::stored_value(value, code, ecx.variable_types),
    }
}

/// The value-or-nothing `expr` names, read whole as a value of its own.
pub(super) fn whole(expr: &Expr, ecx: &ExprCtx) -> String {
    let code = super::expr::codegen_expr_ctx(expr, ecx);
    match expr {
        Expr::Identifier(_) | Expr::FieldAccess { .. } => format!("{code}.clone()"),
        Expr::Index { .. } => format!("({code}).clone()"),
        _ => code,
    }
}

/// The Rust place `collection` names, for an operation that reads or writes it where it is.
pub(super) fn place(collection: &Expr, ecx: &ExprCtx) -> Option<String> {
    let names = RustNames::new(ecx.interner);
    match collection {
        Expr::Identifier(sym) => Some(if super::expr::lent_in_place(*sym, ecx.variable_types) {
            format!("(*{})", names.ident(*sym))
        } else {
            names.ident(*sym)
        }),
        Expr::FieldAccess { object, field } => Some(format!(
            "{}.{}",
            place(object, ecx).unwrap_or_else(|| super::expr::codegen_expr_ctx(object, ecx)),
            crate::codegen::escape_rust_ident(ecx.interner.resolve(*field))
        )),
        _ => None,
    }
}

/// The value the value-or-nothing `expr` holds, read in place by an operation that stops with `failure` when it
/// holds `nothing`.
pub(super) fn held(expr: &Expr, failure: &str, ecx: &ExprCtx) -> String {
    match place(expr, ecx) {
        Some(at) => format!("(*{at}.held({failure:?}))"),
        None => format!("({}).into_held({failure:?})", whole(expr, ecx)),
    }
}

/// The words the VM stops with for an operation on `nothing`, as the shared semantics give them. `None` when the
/// operation on `nothing` does not stop.
pub(super) fn words(operation: impl FnOnce(RuntimeValue) -> Result<(), String>) -> String {
    operation(RuntimeValue::Nothing).err().unwrap_or_else(|| "an operation the VM performs on nothing".to_string())
}

/// A value of the language type of a Rust type, to ask the shared semantics what an operation on it does.
fn sample(ty: &str) -> RuntimeValue {
    match LogosType::from_rust_type_str(ty) {
        LogosType::Float => RuntimeValue::Float(1.0),
        LogosType::Bool => RuntimeValue::Bool(true),
        LogosType::String => RuntimeValue::Text(Rc::new("a".to_string())),
        _ => RuntimeValue::Int(1),
    }
}

/// `left op right` where either operand is a value-or-nothing: both evaluated, in order, then read as the values
/// they hold, the operation stopping as the VM's does on whichever is `nothing`; the operation itself emitted as for
/// two values. `None` when neither operand is a value-or-nothing, for the operations that take `nothing` as a value
/// (equality, joining text), and for `and` and `or`, which read their right operand only when they need it.
pub(super) fn binary(op: BinaryOpKind, left: &Expr, right: &Expr, ecx: &ExprCtx) -> Option<String> {
    if matches!(
        op,
        BinaryOpKind::Eq
            | BinaryOpKind::NotEq
            | BinaryOpKind::Concat
            | BinaryOpKind::SeqConcat
            | BinaryOpKind::And
            | BinaryOpKind::Or
    ) {
        return None;
    }
    let types = [left, right].map(|e| if is_held(e, ecx) { type_of(e, ecx) } else { None });
    if types.iter().all(Option::is_none) {
        return None;
    }
    let program = program()?;
    let payloads = payload_types([left, right], &types, ecx);
    if matches!(op, BinaryOpKind::Add) && payloads.iter().any(|t| t == "String") {
        return None;
    }
    let failure = |l: RuntimeValue, r: RuntimeValue| {
        crate::semantics::arith::binary_op(op, l, r)
            .err()
            .unwrap_or_else(|| "an operation the VM performs on nothing".to_string())
    };
    let failures = [
        failure(RuntimeValue::Nothing, sample(&payloads[1])),
        failure(sample(&payloads[0]), RuntimeValue::Nothing),
        failure(RuntimeValue::Nothing, RuntimeValue::Nothing),
    ];
    let operands: Vec<String> = [left, right]
        .iter()
        .zip(&types)
        .zip(&payloads)
        .map(|((e, t), p)| match t {
            Some(_) => whole(e, ecx),
            None => format!("logicaffeine_data::LogosOpt::<{p}>::from({})", copied(e, ecx)),
        })
        .collect();
    let mut variable_types = ecx.variable_types.clone();
    for (sym, ty) in program.operands.iter().zip(&payloads) {
        variable_types.insert(*sym, ty.clone());
    }
    let (l, r) = (Expr::Identifier(program.operands[0]), Expr::Identifier(program.operands[1]));
    let operation = Expr::BinaryOp { op, left: &l, right: &r };
    let inner = ExprCtx { variable_types: &variable_types, ..*ecx };
    Some(format!(
        "{{ let ({}, {}) = logicaffeine_data::LogosOpt::pair({}, {}, {:?}); {} }}",
        OPERANDS[0],
        OPERANDS[1],
        operands[0],
        operands[1],
        failures,
        super::expr::codegen_expr_ctx(&operation, &inner)
    ))
}

/// `expr` where it reads a value-or-nothing, emitted here: an operation on one ([`binary`]), an equality with one
/// ([`equality`]), or a read of a list or a map that is one, which reads the collection it holds in place, stopping in
/// the VM's words for that read on `nothing`. `None` for every other expression.
pub(super) fn emit(expr: &Expr, ecx: &ExprCtx) -> Option<String> {
    program()?;
    match expr {
        Expr::BinaryOp { op, left, right } => {
            binary(*op, left, right, ecx).or_else(|| equality(*op, left, right, ecx))
        }
        Expr::FieldAccess { object, .. } if is_held(object, ecx) => {
            let failure = logicaffeine_data::failure::field_access_on(RuntimeValue::Nothing.type_name());
            let _read = reading(object, held(object, &failure, ecx));
            Some(super::expr::codegen_expr_ctx(expr, ecx))
        }
        Expr::Index { collection, index } if !is_held(collection, ecx) && read(collection).is_none() => {
            // A map whose values can be `nothing` is read by a borrow of the key, as every map is.
            let map = type_of(collection, ecx)?;
            let ("LogosMap", args) = generic(&map)? else { return None };
            if args.len() != 2 {
                return None;
            }
            let at = place(collection, ecx).unwrap_or_else(|| whole(collection, ecx));
            Some(format!(
                "{{ let __key = &({}); logicaffeine_data::failure::found({at}.get(__key), __key) }}",
                super::expr::codegen_expr_ctx(index, ecx)
            ))
        }
        Expr::Index { collection, .. } if is_held(collection, ecx) => {
            let failure = words(|n| {
                crate::semantics::collections::index_get(&n, &RuntimeValue::Int(1)).map(|_| ())
            });
            Some(reading_collection(expr, collection, &failure, ecx))
        }
        Expr::Length { collection } if is_held(collection, ecx) => {
            let failure = words(|n| crate::semantics::collections::length_of(&n).map(|_| ()));
            Some(reading_collection(expr, collection, &failure, ecx))
        }
        Expr::Contains { collection, value } if read(value).is_none() => {
            let held = is_held(collection, ecx);
            let items = type_of(collection, ecx)
                .and_then(|t| item(payload(&t).unwrap_or(&t)).map(str::to_string))
                .filter(|i| holds_anywhere(i));
            if !held && items.is_none() {
                return None;
            }
            // A list of values-or-nothing is searched for the value as one of its items.
            let _needle = items.map(|i| {
                let needle = stored(value, &i, ecx).unwrap_or_else(|| plain(value, &i, ecx));
                reading(value, needle)
            });
            if held {
                let failure =
                    words(|n| crate::semantics::collections::contains(&n, &RuntimeValue::Nothing).map(|_| ()));
                Some(reading_collection(expr, collection, &failure, ecx))
            } else {
                Some(super::expr::codegen_expr_ctx(expr, ecx))
            }
        }
        Expr::Slice { collection, .. } if is_held(collection, ecx) => {
            let one = RuntimeValue::Int(1);
            let failure = words(|n| crate::semantics::collections::slice(&n, &one, &one).map(|_| ()));
            Some(reading_collection(expr, collection, &failure, ecx))
        }
        _ => None,
    }
}

/// `expr`, a read of `collection`, a value-or-nothing: emitted over the collection it holds, read in place.
fn reading_collection(expr: &Expr, collection: &Expr, failure: &str, ecx: &ExprCtx) -> String {
    let ty = type_of(collection, ecx).expect("a value-or-nothing");
    let mut variable_types = ecx.variable_types.clone();
    if let Expr::Identifier(sym) = collection {
        variable_types.insert(*sym, payload(&ty).unwrap_or(&ty).to_string());
    }
    let _read = reading(collection, held(collection, failure, ecx));
    super::expr::codegen_expr_ctx(expr, &ExprCtx { variable_types: &variable_types, ..*ecx })
}

/// `left == right` (or `!=`) where either side is a value-or-nothing, as the VM compares them: `nothing` equals only
/// `nothing`, and two values compare as values do. `None` when neither side is one.
pub(super) fn equality(op: BinaryOpKind, left: &Expr, right: &Expr, ecx: &ExprCtx) -> Option<String> {
    if !matches!(op, BinaryOpKind::Eq | BinaryOpKind::NotEq) {
        return None;
    }
    let types = [left, right].map(|e| if is_held(e, ecx) { type_of(e, ecx) } else { None });
    if types.iter().all(Option::is_none) {
        return None;
    }
    let negate = if op == BinaryOpKind::NotEq { "!" } else { "" };
    let absent = |e: &Expr| matches!(e, Expr::Literal(Literal::Nothing) | Expr::OptionNone);
    for (side, other) in [(left, right), (right, left)] {
        if absent(other) && is_held(side, ecx) {
            let at = place(side, ecx).unwrap_or_else(|| format!("({})", whole(side, ecx)));
            return Some(format!("{negate}{at}.is_nothing()"));
        }
    }
    let program = program()?;
    let payloads = payload_types([left, right], &types, ecx);
    let operands: Vec<String> = [left, right]
        .iter()
        .zip(&types)
        .zip(&payloads)
        .map(|((e, t), p)| match t {
            Some(_) => whole(e, ecx),
            None => format!("logicaffeine_data::LogosOpt::<{p}>::from({})", copied(e, ecx)),
        })
        .collect();
    let kinds = payloads.iter().map(|p| LogosType::from_rust_type_str(p)).collect::<Vec<_>>();
    let comparable = kinds[0] == kinds[1] || (kinds[0].is_numeric() && kinds[1].is_numeric());
    let same = if comparable {
        let mut variable_types = ecx.variable_types.clone();
        for (sym, ty) in program.operands.iter().zip(&payloads) {
            variable_types.insert(*sym, ty.clone());
        }
        let (l, r) = (Expr::Identifier(program.operands[0]), Expr::Identifier(program.operands[1]));
        let operation = Expr::BinaryOp { op: BinaryOpKind::Eq, left: &l, right: &r };
        super::expr::codegen_expr_ctx(&operation, &ExprCtx { variable_types: &variable_types, ..*ecx })
    } else {
        "false".to_string()
    };
    Some(format!(
        "{negate}(match (({}).0, ({}).0) {{ (Some({}), Some({})) => {{ {} }}, (None, None) => true, _ => false }})",
        operands[0], operands[1], OPERANDS[0], OPERANDS[1], same
    ))
}

/// A place a statement changes, reached for writing: the Rust that evaluates every key on the way to it, the Rust
/// that reaches it, the place itself, its Rust type, and whether this statement may copy it on write.
struct Writable {
    keys: String,
    reach: String,
    at: String,
    ty: String,
    owned: bool,
}

impl Writable {
    /// The place entered when it is a value-or-nothing: the value it holds, the statement stopping with `failure`
    /// when it holds `nothing`.
    fn entered(self, failure: &str) -> Writable {
        match payload(&self.ty).map(str::to_string) {
            Some(held) => Writable { at: format!("(*{}.held_mut({failure:?}))", self.at), ty: held, ..self },
            None => self,
        }
    }

    /// `{at}.cow(); ` when the place is a list or a map another value may share and the statement may copy it.
    fn copy_on_write(&self) -> String {
        if self.owned
            && crate::semantics::collections::value_semantics_enabled()
            && super::expr::lent_by_shared_reference(&self.ty)
        {
            format!("{}.cow(); ", self.at)
        } else {
            String::new()
        }
    }
}

/// `expr` as a place to change: a binding, a field or an item of one, at any depth. A level that is a value-or-nothing
/// is entered through the value it holds, stopping with `failure` when it holds `nothing`; a list or a map entered
/// to reach one of its items is copied first when another value shares it, so no copy of it ever changes. Every key
/// is evaluated, outermost first, before any list is borrowed. `None` for a place this tier does not write this way.
fn writable(expr: &Expr, failure: &str, depth: usize, ecx: &ExprCtx) -> Option<Writable> {
    match expr {
        Expr::Identifier(sym) => {
            let ty = ecx.variable_types.get(sym)?;
            let ty = ty.split("|__hl:").next().unwrap_or(ty).trim_start_matches('&').trim_start_matches("mut ");
            let names = RustNames::new(ecx.interner);
            let owned = holds(ty)
                || !super::expr::lent_by_shared_reference(ty)
                || super::expr::cow_handle(expr, ecx.variable_types, &names).is_some();
            Some(Writable { keys: String::new(), reach: String::new(), at: place(expr, ecx)?, ty: ty.to_string(), owned })
        }
        Expr::FieldAccess { object, field } => {
            let outer = writable(object, failure, depth + 1, ecx)?.entered(failure);
            let ty = field_type(&outer.ty, *field, ecx)?;
            let at = format!("{}.{}", outer.at, crate::codegen::escape_rust_ident(ecx.interner.resolve(*field)));
            Some(Writable { at, ty, owned: true, ..outer })
        }
        Expr::Index { collection, index } => {
            let outer = writable(collection, failure, depth + 1, ecx)?.entered(failure);
            let (head, args) = generic(&outer.ty)?;
            let cow = outer.copy_on_write();
            let (item, find) = match head {
                "LogosSeq" if args.len() == 1 => (args[0].to_string(), format!("logos_item_mut(__key{depth})")),
                "LogosMap" if args.len() == 2 => (
                    args[1].to_string(),
                    String::new(),
                ),
                _ => return None,
            };
            let reach = if find.is_empty() {
                format!(
                    "{}{cow}let mut __items{depth} = {}.borrow_mut(); let __item{depth} = \
                     logicaffeine_data::failure::found(__items{depth}.get_mut(&__key{depth}), &__key{depth}); ",
                    outer.reach, outer.at
                )
            } else {
                format!(
                    "{}{cow}let mut __items{depth} = {}.borrow_mut(); let __item{depth} = __items{depth}.{find}; ",
                    outer.reach, outer.at
                )
            };
            Some(Writable {
                keys: format!("{}let __key{depth} = {}; ", outer.keys, copied(index, ecx)),
                reach,
                at: format!("(*__item{depth})"),
                ty: item,
                owned: true,
            })
        }
        _ => None,
    }
}

/// The Rust type of field `field` of a record of Rust type `ty`: its declared type, the record's type arguments put
/// for its type parameters, as the storage a stored `nothing` calls for.
fn field_type(ty: &str, field: Symbol, ecx: &ExprCtx) -> Option<String> {
    let (head, args) = match generic(ty) {
        Some((head, args)) => (head, args),
        None => (ty.trim(), Vec::new()),
    };
    ecx.registry.iter_types().find_map(|(name, def)| match def {
        crate::analysis::registry::TypeDef::Struct { fields, generics, .. }
            if super::types::rust_type_ident(ecx.interner.resolve(*name)) == head =>
        {
            let declared = fields.iter().find(|f| f.name == field)?;
            let mut held = if crate::codegen::expr::stores_logos_int(*name, field) {
                "LogosInt".to_string()
            } else {
                super::types::codegen_field_type(&declared.ty, ecx.interner)
            };
            for (param, arg) in generics.iter().zip(&args) {
                held = super::context::replace_word(&held, ecx.interner.resolve(*param), arg);
            }
            Some(self::field(*name, field, held))
        }
        _ => None,
    })
}

/// The statement `stmt` where it changes a value-or-nothing or a collection holding them, emitted here: the place
/// reached for writing ([`writable`]), each stored value as the storage it goes into, and the change itself. `None`
/// for every other statement.
pub(super) fn statement(stmt: &Stmt, ecx: &ExprCtx, indent: &str) -> Option<(String, Option<(Symbol, String)>)> {
    program()?;
    let involves = |e: &Expr| type_of(e, ecx).is_some();
    let semantics = |change: fn(RuntimeValue) -> Result<(), String>| words(change);
    let block = |value: String, place: &Writable, change: String| {
        format!("{indent}{{ {value}{}{}{}{change}; }}\n", place.keys, place.reach, place.copy_on_write())
    };
    let code = match stmt {
        Stmt::Push { value, collection } if involves(collection) => {
            let failure = semantics(|n| crate::semantics::collections::list_push(&n, RuntimeValue::Nothing));
            let place = writable(collection, &failure, 0, ecx)?.entered(&failure);
            let item = item(&place.ty)?.to_string();
            let pushed = stored(value, &item, ecx).unwrap_or_else(|| plain(value, &item, ecx));
            let change = format!("{}.push(__pushed)", place.at);
            block(format!("let __pushed = {pushed}; "), &place, change)
        }
        Stmt::SetIndex { collection, index, value } if involves(collection) => {
            let failure = semantics(|n| {
                crate::semantics::collections::index_set(&n, &RuntimeValue::Int(1), RuntimeValue::Nothing)
            });
            let place = writable(collection, &failure, 0, ecx)?.entered(&failure);
            let item = item(&place.ty)?.to_string();
            let key = copied(index, ecx);
            let set = stored(value, &item, ecx).unwrap_or_else(|| plain(value, &item, ecx));
            let change = format!("LogosIndexMut::logos_set(&mut {}, __at, __set)", place.at);
            block(format!("let __at = {key}; let __set = {set}; "), &place, change)
        }
        Stmt::Pop { collection, into } if involves(collection) => {
            let failure = semantics(|n| crate::semantics::collections::list_pop(&n).map(|_| ()));
            let place = writable(collection, &failure, 0, ecx)?.entered(&failure);
            let item = item(&place.ty)?.to_string();
            let popped = format!(
                "{{ {}{}{}{}.pop().expect(\"Pop from empty collection\") }}",
                place.keys,
                place.reach,
                place.copy_on_write(),
                place.at
            );
            return Some(match into {
                Some(var) => (
                    format!("{indent}let {}: {item} = {popped};\n", RustNames::new(ecx.interner).ident(*var)),
                    Some((*var, item)),
                ),
                None => (format!("{indent}{popped};\n"), None),
            });
        }
        Stmt::Add { value, collection } | Stmt::Remove { value, collection } if involves(collection) => {
            let adding = matches!(stmt, Stmt::Add { .. });
            let failure = if adding {
                semantics(|n| crate::semantics::collections::set_add(&n, RuntimeValue::Nothing))
            } else {
                semantics(|n| crate::semantics::collections::remove_from(&n, &RuntimeValue::Nothing))
            };
            let place = writable(collection, &failure, 0, ecx)?.entered(&failure);
            let element = generic(&place.ty).map(|(_, args)| args[0].to_string()).unwrap_or_else(|| "i64".to_string());
            let v = plain(value, &element, ecx);
            let change = match (adding, generic(&place.ty)) {
                (true, _) => format!("{}.insert(__changed)", place.at),
                (false, Some(("LogosMap" | "Map", _))) => format!("{}.remove(&__changed)", place.at),
                (false, _) => format!("{}.shift_remove(&__changed)", place.at),
            };
            block(format!("let __changed = {v}; "), &place, change)
        }
        Stmt::SetField { object, field, value } => {
            let target = Expr::FieldAccess { object: *object, field: *field };
            if !involves(object) && !involves(&target) {
                return None;
            }
            let failure = logicaffeine_data::failure::SET_FIELD_ON_NON_RECORD;
            let place = writable(&target, failure, 0, ecx)?;
            let set = stored(value, &place.ty, ecx).unwrap_or_else(|| plain(value, &place.ty, ecx));
            let change = format!("{} = __set", place.at);
            format!("{indent}{{ let __set = {set}; {}{}{change}; }}\n", place.keys, place.reach)
        }
        _ => return None,
    };
    Some((code, None))
}

/// The items a `Repeat` over `iterable` binds, when it is a value-or-nothing list or a list of them: a copy of the
/// list taken on entry (for one that is a value-or-nothing, of the list it holds, stopping in the VM's words for
/// iterating `nothing`), and the Rust type of each item.
pub(super) fn iteration(iterable: &Expr, ecx: &ExprCtx) -> Option<(String, String)> {
    let ty = type_of(iterable, ecx)?;
    let (list, items) = match payload(&ty) {
        Some(list) => {
            let failure = words(|n| crate::semantics::collections::iteration_snapshot(&n).map(|_| ()));
            (list, held(iterable, &failure, ecx))
        }
        None => (ty.as_str(), place(iterable, ecx).unwrap_or_else(|| whole(iterable, ecx))),
    };
    match generic(list)? {
        ("LogosSeq" | "Seq" | "Vec", args) if args.len() == 1 => Some((format!("{items}.to_vec()"), args[0].to_string())),
        _ => None,
    }
}

/// `expr`, a value of no value-or-nothing, as a value of its own: a binding that is not `Copy` copied, not moved.
fn copied(expr: &Expr, ecx: &ExprCtx) -> String {
    super::stmt::stored_value(expr, super::expr::codegen_expr_ctx(expr, ecx), ecx.variable_types)
}

/// Whether `expr` is a value-or-nothing that holds text.
pub(super) fn holds_text(expr: &Expr, ecx: &ExprCtx) -> bool {
    is_held(expr, ecx) && type_of(expr, ecx).is_some_and(|t| payload(&t) == Some("String"))
}

/// The Rust type an operand of no value-or-nothing is read as: the record a construction builds, what the bindings in
/// scope say, else what the program says. `None` when nothing says.
fn operand_type(expr: &Expr, ecx: &ExprCtx) -> Option<String> {
    if let Expr::New { type_name, .. } = expr {
        if let Some(crate::analysis::registry::TypeDef::Struct { .. }) = ecx.registry.get(*type_name) {
            return Some(super::types::rust_type_ident(ecx.interner.resolve(*type_name)));
        }
    }
    let local = super::types::infer_logos_type(expr, ecx.interner, ecx.variable_types);
    let ty = match (local, program()) {
        (LogosType::Unknown, Some(program)) => program.types.infer_expr(expr, ecx.interner),
        (known, _) => known,
    };
    match ty {
        LogosType::Unknown => None,
        other => Some(rust_type(&other, ecx)),
    }
}

/// The payload types of two operands of which `types` gives each value-or-nothing's own: an operand of no
/// value-or-nothing is read as the type it has, or, when nothing says, as the other operand's payload.
fn payload_types(operands: [&Expr; 2], types: &[Option<String>; 2], ecx: &ExprCtx) -> [String; 2] {
    let own = |i: usize| match &types[i] {
        Some(t) => Some(payload(t).unwrap_or(t).to_string()),
        None => operand_type(operands[i], ecx),
    };
    let (left, right) = (own(0), own(1));
    let fallback = |mine: Option<String>, other: &Option<String>| {
        mine.or_else(|| other.clone()).unwrap_or_else(|| "i64".to_string())
    };
    [fallback(left.clone(), &right), fallback(right, &left)]
}

/// Read `expr` as `code` while the returned guard lives: an operation that has read the value-or-nothing `expr`
/// names emits the rest of itself over what it read.
pub(super) fn reading(expr: &Expr, code: String) -> ReadGuard {
    let key = expr as *const Expr as usize;
    ReadGuard(key, READ.with(|r| r.borrow_mut().insert(key, code)))
}

/// Ends a [`reading`].
pub(super) struct ReadGuard(usize, Option<String>);

impl Drop for ReadGuard {
    fn drop(&mut self) {
        READ.with(|r| {
            let mut r = r.borrow_mut();
            match self.1.take() {
                Some(code) => r.insert(self.0, code),
                None => r.remove(&self.0),
            };
        });
    }
}
