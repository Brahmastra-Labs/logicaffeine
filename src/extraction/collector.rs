//! Dependency collection for program extraction.
//!
//! Collects all transitive dependencies needed to extract a definition.

use crate::kernel::{Context, Term};
use std::collections::HashSet;

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
fn collect_globals(term: &Term, deps: &mut Vec<String>) {
    match term {
        Term::Global(name) => deps.push(name.clone()),
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
        // Base cases: no dependencies
        Term::Sort(_) | Term::Var(_) | Term::Lit(_) => {}
    }
}
