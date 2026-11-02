//! Explicit type arguments where rustc cannot infer them.
//!
//! The AOT-Rust tier emits LOGOS generics as Rust generics, and rustc infers each call's type
//! arguments from its arguments — except a PHANTOM one, which no argument carries:
//! `okay(a new Ok with value "x")` never says what the `Result`'s error type is, and nothing in the
//! program ever does. The checker solved every site ([`crate::analysis::infer_instantiations`]);
//! a type argument it left open that is not one of a generic function's own parameters is exactly
//! such a phantom. It carries no value, so any type represents it, and it becomes `i64` in a
//! turbofish — `okay::<_, i64>(…)`, `Result::<_, i64>::Ok { … }` — every other position left `_`
//! for rustc. A site with no phantom is emitted exactly as before.

use std::collections::HashMap;

use crate::analysis::unify::{InferType, TyVar};
use crate::analysis::{infer_instantiations, Site, TypeRegistry};
use crate::ast::stmt::{Expr, Stmt};
use crate::intern::{Interner, Symbol};

thread_local! {
    /// Each call or construction site's turbofish (`::<_, i64>`), for the program being emitted.
    /// Set once per program at codegen entry by [`install`].
    static TURBOFISH: std::cell::RefCell<HashMap<Site, String>> = std::cell::RefCell::new(HashMap::new());
}

/// Solve `stmts` — the exact statements about to be emitted, so each site's address is the one
/// the emitters see — and record the turbofish every phantom-carrying site needs.
pub(crate) fn install(stmts: &[Stmt], registry: &TypeRegistry, interner: &Interner) {
    let inst = infer_instantiations(stmts, interner, registry);
    let params: HashMap<TyVar, Symbol> = inst
        .type_params
        .values()
        .flatten()
        .filter_map(|(name, t)| match t {
            InferType::Var(tv) => Some((*tv, *name)),
            _ => None,
        })
        .collect();
    let mut fish = HashMap::new();
    for (site, call) in &inst.calls {
        if let Some(t) = turbofish(&call.type_args, &params, interner) {
            fish.insert(*site, t);
        }
    }
    for (site, built) in &inst.constructed {
        if let InferType::UserDefined(_, args) = built {
            if let Some(t) = turbofish(args, &params, interner) {
                fish.insert(*site, t);
            }
        }
    }
    TURBOFISH.with(|f| *f.borrow_mut() = fish);
}

/// The turbofish a call expression needs, or `""`.
pub(crate) fn for_expr(e: &Expr) -> String {
    TURBOFISH.with(|f| {
        f.borrow()
            .get(&Site::of_expr(e))
            .cloned()
            .unwrap_or_default()
    })
}

/// The turbofish a statement-level call needs, or `""`.
pub(crate) fn for_stmt(s: &Stmt) -> String {
    TURBOFISH.with(|f| {
        f.borrow()
            .get(&Site::of_stmt(s))
            .cloned()
            .unwrap_or_default()
    })
}

fn turbofish(
    args: &[InferType],
    params: &HashMap<TyVar, Symbol>,
    interner: &Interner,
) -> Option<String> {
    let phantom = |t: &InferType| mentions_phantom(t, params);
    if !args.iter().any(phantom) {
        return None;
    }
    let parts: Vec<String> = args
        .iter()
        .map(|a| {
            if phantom(a) {
                rust_type(a, params, interner).unwrap_or_else(|| "_".into())
            } else {
                "_".into()
            }
        })
        .collect();
    Some(format!("::<{}>", parts.join(", ")))
}

fn mentions_phantom(t: &InferType, params: &HashMap<TyVar, Symbol>) -> bool {
    let rec = |x: &InferType| mentions_phantom(x, params);
    match t {
        InferType::Var(tv) => !params.contains_key(tv),
        InferType::Seq(i) | InferType::Set(i) | InferType::Option(i) => rec(i),
        InferType::Map(k, v) => rec(k) || rec(v),
        InferType::UserDefined(_, args) => args.iter().any(rec),
        InferType::Function(ps, r) => ps.iter().any(rec) || rec(r),
        _ => false,
    }
}

/// `t` spelled as the Rust type this tier gives it, a phantom as `i64` and a generic function's
/// own parameter by its name; `None` for a type with no spelling here (a closure).
fn rust_type(
    t: &InferType,
    params: &HashMap<TyVar, Symbol>,
    interner: &Interner,
) -> Option<String> {
    let rec = |x: &InferType| rust_type(x, params, interner);
    Some(match t {
        InferType::Var(tv) => match params.get(tv) {
            Some(name) => interner.resolve(*name).to_string(),
            None => "i64".to_string(),
        },
        InferType::Int => "i64".to_string(),
        InferType::Float => "f64".to_string(),
        InferType::Bool => "bool".to_string(),
        InferType::String => "String".to_string(),
        InferType::Char => "char".to_string(),
        InferType::Unit => "()".to_string(),
        InferType::Seq(i) => format!("LogosSeq<{}>", rec(i)?),
        InferType::Set(i) => format!("Set<{}>", rec(i)?),
        InferType::Option(i) => format!("Option<{}>", rec(i)?),
        InferType::Map(k, v) => format!("LogosMap<{}, {}>", rec(k)?, rec(v)?),
        InferType::UserDefined(sym, args) if args.is_empty() => {
            super::types::rust_type_ident(interner.resolve(*sym))
        }
        InferType::UserDefined(sym, args) => {
            let args: Option<Vec<String>> = args.iter().map(rec).collect();
            format!(
                "{}<{}>",
                super::types::rust_type_ident(interner.resolve(*sym)),
                args?.join(", ")
            )
        }
        _ => return None,
    })
}
