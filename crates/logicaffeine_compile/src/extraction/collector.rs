//! Dependency collection for program extraction.
//!
//! Collects all transitive dependencies needed to extract a definition.

use crate::kernel::{Context, Term};
use std::collections::{HashMap, HashSet};

/// Whether `name` extracts to self-contained Rust: a constructor of an emittable
/// inductive, an emittable inductive (has constructors) or mapped primitive, or a
/// definition whose body only references extractable things. Axioms / declarations
/// / tactics (no extractable body, e.g. `syn_diag`, `try_auto`) are NOT extractable
/// — extracting them would emit Rust referencing undefined symbols.
pub fn is_extractable(ctx: &Context, name: &str) -> bool {
    extractable(ctx, name, &mut HashMap::new())
}

fn extractable(ctx: &Context, name: &str, memo: &mut HashMap<String, bool>) -> bool {
    // `_` is a placeholder (e.g. a match-motive binder) that surfaces as a Global
    // but is never emitted — treat it as benign.
    if name == "_" {
        return true;
    }
    if let Some(&b) = memo.get(name) {
        return b;
    }
    // Optimistic for recursive/cyclic references (a fixpoint that can't lower).
    memo.insert(name.to_string(), true);
    let result = if ctx.is_constructor(name) {
        ctx.constructor_inductive(name)
            .map(|ind| !ctx.get_constructors(ind).is_empty())
            .unwrap_or(false)
    } else if ctx.is_inductive(name) {
        (!ctx.get_constructors(name).is_empty() && !is_logical_type(name)) || is_mapped_primitive(name)
    } else if let Some(body) = ctx.get_definition_body(name) {
        let mut refs = Vec::new();
        collect_globals(body, &mut refs);
        refs.iter().all(|g| extractable(ctx, g, memo))
            && matches_inferable(ctx, body)
            // Arithmetic builtins must be fully (binary) applied — a bare/partial
            // `add` has no Rust function to call.
            && super::codegen::arith_uses_well_formed(body)
    } else {
        // A declaration / axiom / primitive op with no body. The mapped primitives
        // and the arithmetic builtins (`add`/`sub`/… → Rust operators) are the only
        // bodyless globals that extract to real Rust; everything else (tactics,
        // reflection axioms) would emit references to undefined symbols.
        is_mapped_primitive(name) || super::codegen::arith_operator(name).is_some()
    };
    memo.insert(name.to_string(), result);
    result
}

/// Whether every `match` in a term has an inductive the extractor can recognize
/// (from the motive or the discriminant). Literate-generated matches sometimes
/// lack a readable motive, which would extract to an empty (non-exhaustive) match;
/// such definitions are not cleanly extractable.
fn matches_inferable(ctx: &Context, term: &Term) -> bool {
    match term {
        Term::Match { discriminant, motive, cases } => {
            if motive_inductive(ctx, motive)
                .or_else(|| disc_inductive(ctx, discriminant))
                .is_none()
            {
                return false;
            }
            matches_inferable(ctx, discriminant)
                && matches_inferable(ctx, motive)
                && cases.iter().all(|c| matches_inferable(ctx, c))
        }
        Term::Lambda { param_type, body, .. } => {
            matches_inferable(ctx, param_type) && matches_inferable(ctx, body)
        }
        Term::Pi { param_type, body_type, .. } => {
            matches_inferable(ctx, param_type) && matches_inferable(ctx, body_type)
        }
        Term::App(f, a) => matches_inferable(ctx, f) && matches_inferable(ctx, a),
        Term::Fix { body, .. } => matches_inferable(ctx, body),
        _ => true,
    }
}

fn motive_inductive(ctx: &Context, motive: &Term) -> Option<String> {
    if let Term::Lambda { param_type, .. } = motive {
        if let Term::Global(name) = param_type.as_ref() {
            if ctx.is_inductive(name) {
                return Some(name.clone());
            }
        }
    }
    None
}

fn disc_inductive(ctx: &Context, term: &Term) -> Option<String> {
    match term {
        Term::Global(name) => {
            if ctx.is_constructor(name) {
                ctx.constructor_inductive(name).map(|s| s.to_string())
            } else if ctx.is_inductive(name) {
                Some(name.clone())
            } else {
                None
            }
        }
        Term::App(f, _) => disc_inductive(ctx, f),
        _ => None,
    }
}

fn is_mapped_primitive(name: &str) -> bool {
    matches!(
        name,
        "Int" | "Float" | "Text" | "Bool" | "Duration" | "Date" | "Moment"
    )
}

/// StandardLibrary types that encode logic/proof/reflection rather than user data
/// (`Syntax` ASTs, `Eq`/`And`/`Ex` propositions, `Derivation` proofs, …). Values of
/// these are not the "compile my data to Rust" use case and don't extract cleanly,
/// so definitions over them are left as a note instead of emitting broken Rust.
pub fn is_logical_type(name: &str) -> bool {
    matches!(
        name,
        "Syntax" | "Derivation" | "Eq" | "And" | "Or" | "Iff" | "Ex" | "Not"
            | "True" | "False" | "Entity" | "Univ" | "Prop"
    )
}

/// Collect all dependencies of a definition.
///
/// Starting from an entry point, recursively collects all global names
/// that are referenced, including inductives, constructors, and definitions.
pub fn collect_dependencies(ctx: &Context, entry: &str) -> HashSet<String> {
    let mut visited = HashSet::new();
    let mut to_visit = vec![entry.to_string()];

    while let Some(name) = to_visit.pop() {
        if visited.contains(&name) {
            continue;
        }
        visited.insert(name.clone());

        // Get the term to analyze based on what kind of global it is
        if let Some(body) = ctx.get_definition_body(&name) {
            collect_globals(body, &mut to_visit);
            // Also collect from the type
            if let Some(ty) = ctx.get_definition_type(&name) {
                collect_globals(ty, &mut to_visit);
            }
        }

        if ctx.is_inductive(&name) {
            // Add constructors and their types
            for (ctor_name, ctor_ty) in ctx.get_constructors(&name) {
                collect_globals(ctor_ty, &mut to_visit);
                to_visit.push(ctor_name.to_string());
            }
        }

        if ctx.is_constructor(&name) {
            // Add the inductive type
            if let Some(ind) = ctx.constructor_inductive(&name) {
                to_visit.push(ind.to_string());
            }
        }
    }

    visited
}

/// Recursively collect all Global references from a term.
pub(crate) fn collect_globals(term: &Term, deps: &mut Vec<String>) {
    match term {
        Term::Global(name) => deps.push(name.clone()),
        Term::Const { name, .. } => deps.push(name.clone()),
        Term::App(f, a) => {
            collect_globals(f, deps);
            collect_globals(a, deps);
        }
        Term::Lambda {
            param_type, body, ..
        } => {
            collect_globals(param_type, deps);
            collect_globals(body, deps);
        }
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            collect_globals(param_type, deps);
            collect_globals(body_type, deps);
        }
        Term::Fix { body, .. } => collect_globals(body, deps),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            collect_globals(discriminant, deps);
            collect_globals(motive, deps);
            for case in cases {
                collect_globals(case, deps);
            }
        }
        Term::Let {
            ty, value, body, ..
        } => {
            collect_globals(ty, deps);
            collect_globals(value, deps);
            collect_globals(body, deps);
        }
        Term::MutualFix { defs, .. } => {
            for (_, body) in defs {
                collect_globals(body, deps);
            }
        }
        // Base cases: no dependencies
        Term::Sort(_) | Term::Var(_) | Term::Lit(_) | Term::Hole => {}
    }
}
