//! The taint path of every assumption a refused result rests on: the declarations through which the
//! target reaches it.
//!
//! Lean's `#print axioms` (`CollectAxioms`) names the axioms a declaration rests on and says nothing
//! of how they are reached, so a refusal that names only `sorryAx` leaves the reader to find which
//! dependency admitted a step. This walks the checked environment along exactly the edges
//! `CollectAxioms` follows — a definition, theorem or opaque through its statement and its body or
//! proof, an inductive through its statement and its constructors, a constructor through its
//! statement, an axiom never (an axiom is a leaf: the axioms its own type mentions are not reached) —
//! breadth first, so each path is a shortest one, and it takes a declaration's references in
//! ascending Lean-name order, so the path is a function of the environment alone. A declaration the
//! importer introduced (a sharing helper, whose name encodes no Lean name) is walked through and
//! never printed: the path names Lean declarations only.
//!
//! Only the environment is read. A path is an explanation of a refusal the assumption policy already
//! made; it never decides one.

use super::intake;
use super::package::ALLOWED_AXIOMS;
use super::pipeline::Diagnostic;
use super::{DiagnosticCode, ImportedContext};
use crate::{Context, Term};
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

/// One assumption and the Lean declarations through which the target reaches it, target first and
/// the assumption last, in Lean's spelling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaintPath {
    pub assumption: String,
    pub path: Vec<String>,
}

impl TaintPath {
    /// `T.user → T.lemma → sorryAx`.
    pub fn render(&self) -> String {
        self.path.join(" → ")
    }
}

/// The receipt stage whose refusal a taint path explains.
pub const STAGE: &str = "assumption_policy";

/// The taint path of each of `assumptions` (Lean names) from `target` (a kernel name), in the order
/// `assumptions` lists them. An assumption the target does not reach through the environment has no
/// path and is left out.
pub fn taint_paths(
    imported: &ImportedContext,
    target: &str,
    assumptions: &[String],
) -> Vec<TaintPath> {
    let wanted: BTreeMap<String, &String> = assumptions
        .iter()
        .filter_map(|lean| intake::kernel_name(lean).ok().map(|kernel| (kernel, lean)))
        .collect();
    let parents = breadth_first(imported, target, &wanted.keys().cloned().collect());
    assumptions
        .iter()
        .filter_map(|lean| {
            let kernel = intake::kernel_name(lean).ok()?;
            let mut path = vec![kernel.clone()];
            let mut at = kernel;
            if at != target {
                loop {
                    at = parents.get(&at)?.clone();
                    path.push(at.clone());
                    if at == target {
                        break;
                    }
                }
            }
            path.reverse();
            Some(TaintPath {
                assumption: lean.clone(),
                path: path.iter().map(|k| intake::lean_name(k)).collect(),
            })
        })
        .collect()
}

/// One `disallowed_assumption` diagnostic of the assumption-policy stage per assumption outside
/// Lean's three standard axioms that `target` (a kernel name, printed as `target_lean`) rests on,
/// naming its taint path. `assumptions` are the result's (Lean names).
pub fn diagnostics(
    imported: &ImportedContext,
    target: &str,
    target_lean: &str,
    assumptions: &[String],
) -> Vec<Diagnostic> {
    let refused: Vec<String> = assumptions
        .iter()
        .filter(|a| !ALLOWED_AXIOMS.iter().any(|(allowed, _)| allowed == a))
        .cloned()
        .collect();
    let paths = taint_paths(imported, target, &refused);
    refused
        .iter()
        .map(|assumption| {
            let message = match paths.iter().find(|p| &p.assumption == assumption) {
                Some(path) => format!("taint path to {assumption}: {}", path.render()),
                None => format!(
                    "taint path to {assumption}: none found through the checked environment, although the target's \
                     transitive assumptions include it"
                ),
            };
            Diagnostic::new(STAGE, DiagnosticCode::DisallowedAssumption, message).with_declaration(target_lean)
        })
        .collect()
}

/// Breadth-first search from `target` until every name of `wanted` is reached (or the environment is
/// exhausted). Returns each reached Lean declaration's predecessor.
fn breadth_first(
    ctx: &ImportedContext,
    target: &str,
    wanted: &BTreeSet<String>,
) -> BTreeMap<String, String> {
    let mut parents = BTreeMap::new();
    let mut seen: HashSet<String> = HashSet::from([target.to_string()]);
    let mut queue = VecDeque::from([target.to_string()]);
    let mut remaining: BTreeSet<&String> = wanted.iter().filter(|w| w.as_str() != target).collect();
    while let Some(name) = queue.pop_front() {
        if remaining.is_empty() {
            break;
        }
        for next in lean_references(ctx, &name) {
            if seen.insert(next.clone()) {
                parents.insert(next.clone(), name.clone());
                remaining.remove(&next);
                queue.push_back(next);
            }
        }
    }
    parents
}

/// The Lean declarations `name` refers to under `CollectAxioms`' edges, walking through importer
/// declarations that encode no Lean name, deduplicated and in ascending Lean-name order.
fn lean_references(imported: &ImportedContext, name: &str) -> Vec<String> {
    let mut found: BTreeMap<String, String> = BTreeMap::new();
    let mut internal_seen: HashSet<String> = HashSet::new();
    let mut pending = direct_references(imported, name);
    while let Some(next) = pending.pop() {
        if intake::components(&next).is_some() {
            found.entry(intake::lean_name(&next)).or_insert(next);
        } else if next != name && internal_seen.insert(next.clone()) {
            pending.extend(direct_references(imported, &next));
        }
    }
    found.into_values().collect()
}

/// The constants `name`'s own declaration mentions under `CollectAxioms`' rule.
fn direct_references(imported: &ImportedContext, name: &str) -> Vec<String> {
    let ctx: &Context = imported;
    let mut out = Vec::new();
    if let Some((ty, body)) = imported.retained(name) {
        constants(ty, &mut out);
        constants(body, &mut out);
    } else if let Some((_, ty, body)) = ctx.get_universe_poly(name) {
        constants(ty, &mut out);
        constants(body, &mut out);
    } else if let Some(body) = ctx.get_definition_body(name) {
        constants(body, &mut out);
        if let Some(ty) = ctx.get_definition_type(name) {
            constants(ty, &mut out);
        }
    } else if ctx.is_inductive(name) {
        if let Some(ty) = ctx.get_global(name) {
            constants(ty, &mut out);
        }
        out.extend(
            ctx.get_constructors(name)
                .into_iter()
                .map(|(constructor, _)| constructor.to_string()),
        );
    } else if ctx.is_constructor(name) {
        if let Some(ty) = ctx.get_global(name) {
            constants(ty, &mut out);
        }
    }
    out
}

/// Every constant a term mentions, `Global` and `Const` alike.
fn constants(term: &Term, out: &mut Vec<String>) {
    let mut stack = vec![term];
    while let Some(term) = stack.pop() {
        match term {
            Term::Global(name) | Term::Const { name, .. } => out.push(name.clone()),
            Term::Sort(_) | Term::Var(_) | Term::Lit(_) | Term::Hole => {}
            Term::Pi {
                param_type,
                body_type,
                ..
            } => stack.extend([&**param_type, &**body_type]),
            Term::Lambda {
                param_type, body, ..
            } => stack.extend([&**param_type, &**body]),
            Term::App(function, argument) => stack.extend([&**function, &**argument]),
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                stack.extend([&**discriminant, &**motive]);
                stack.extend(cases.iter());
            }
            Term::Fix { body, .. } => stack.push(body),
            Term::MutualFix { defs, .. } => stack.extend(defs.iter().map(|(_, body)| body)),
            Term::Let {
                ty, value, body, ..
            } => stack.extend([&**ty, &**value, &**body]),
            Term::Annotated { term, .. } => stack.push(term),
        }
    }
}
