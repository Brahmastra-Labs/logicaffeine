//! A LOGOS context replayed into the core: every declaration, in dependency order, admitted by
//! [`Theory::Logos`](super::tc::Theory::Logos)'s checks, with the core's verdict for each.
//!
//! A [`Context`] keeps its declarations in maps with no order, so the order is the dependency
//! order of the globals each declaration names: the strongly connected components of that graph,
//! dependencies first. A component of several inductive types is one mutual block; any other
//! cycle is reported, never broken.
//!
//! Inductive types are admitted LOGOS's way, without generated recursors (`match` eliminates), and
//! with Lean's checks on each constructor: the arity is a type ending in a sort; a constructor's
//! type is a type whose parameter binders are the family's, whose fields live in sorts at most the
//! family's unless the family is a proposition, and whose result is the family applied to exactly
//! its parameters and then indices that do not mention the block; it takes the family's universe
//! parameters in the family's order; and strict positivity holds (the `Term` kernel's own check,
//! `positivity::check_positivity_mutual_resolved`). The block's recursive and reflexive shape and
//! its irrelevant universe parameters are recorded. Definitions, theorems and axioms go through
//! the core's ordinary declaration checks; the certified quotient is installed whole.

use super::check::{Decl, DefnDecl, Header};
use super::env::{Const, CtorInfo, Hint, InductiveInfo, Kind, QuotKind, Safety};
use super::error::{KResult, KernelFailure};
use super::expr::{ExprId, Node};
use super::level::LevelId;
use super::logos::{explicit_levels, lower};
use super::name::NameId;
use super::tc::Kernel;
use crate::context::{Context, Registration};
use crate::quotient::PreludeKind;
use crate::term::{Literal, Term};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

/// One declaration's verdict in the replay.
#[derive(Clone, Debug)]
pub struct Replayed {
    pub name: String,
    pub kind: &'static str,
    pub verdict: Result<(), String>,
}

/// A declaration as the core admits it, in the `Term` forms a [`Context`] registers. `params` are
/// its universe parameters, in the order its constants take levels.
#[derive(Clone, Debug)]
pub enum Declaration {
    /// One inductive family of a block: its arity (parameters included), how many of the arity's
    /// binders are parameters, and each constructor's full type.
    Inductive {
        name: String,
        params: Vec<String>,
        arity: Term,
        num_params: usize,
        ctors: Vec<(String, Term)>,
    },
    Definition {
        name: String,
        params: Vec<String>,
        ty: Term,
        body: Term,
    },
    Axiom {
        name: String,
        params: Vec<String>,
        ty: Term,
    },
    /// The quotient: its four primitives in the order the core admits them, installed together
    /// as Lean's `init_quot` installs them, so a declaration that uses any of them sees the whole
    /// quotient and its computation rule. Each is checked against its canonical signature.
    Quotient {
        primitives: Vec<(QuotKind, String, Vec<String>, Term)>,
        equality: String,
    },
    /// The certified observational equality's constants (`crate::observational`), axioms to the
    /// core and computing by `logos_obs`, admitted together after `after`: every name the
    /// certificate rests on, which its rules' answers are built from.
    Observational {
        constants: Vec<(String, Vec<String>, Term)>,
        after: Vec<String>,
    },
}

impl Declaration {
    fn name(&self) -> &str {
        match self {
            Declaration::Inductive { name, .. }
            | Declaration::Definition { name, .. }
            | Declaration::Axiom { name, .. } => name,
            Declaration::Quotient { primitives, .. } => &primitives[0].1,
            Declaration::Observational { constants, .. } => &constants[0].0,
        }
    }

    /// Every global the item declares, constructors and quotient primitives included.
    pub(crate) fn names(&self) -> Vec<&str> {
        match self {
            Declaration::Inductive { name, ctors, .. } => std::iter::once(name.as_str())
                .chain(ctors.iter().map(|(c, _)| c.as_str()))
                .collect(),
            Declaration::Quotient { primitives, .. } => {
                primitives.iter().map(|p| p.1.as_str()).collect()
            }
            Declaration::Observational { constants, .. } => {
                constants.iter().map(|c| c.0.as_str()).collect()
            }
            _ => vec![self.name()],
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Declaration::Inductive { .. } => "inductive",
            Declaration::Definition { .. } => "definition",
            Declaration::Axiom { .. } => "axiom",
            Declaration::Quotient { .. } => "quotient",
            Declaration::Observational { .. } => "observational",
        }
    }

    pub(crate) fn terms(&self) -> Vec<&Term> {
        match self {
            Declaration::Inductive { arity, ctors, .. } => std::iter::once(arity)
                .chain(ctors.iter().map(|(_, t)| t))
                .collect(),
            Declaration::Definition { ty, body, .. } => vec![ty, body],
            Declaration::Axiom { ty, .. } => vec![ty],
            Declaration::Quotient { primitives, .. } => primitives.iter().map(|p| &p.3).collect(),
            Declaration::Observational { constants, .. } => {
                constants.iter().map(|c| &c.2).collect()
            }
        }
    }
}

/// Replays every declaration of `ctx` into a [`Kernel::logos`] kernel.
pub fn replay(ctx: &Context) -> (Kernel, Vec<Replayed>) {
    let (k, _, verdicts) = replay_items(ctx);
    (k, verdicts)
}

/// [`replay`], with the declarations it read.
pub(crate) fn replay_items(ctx: &Context) -> (Kernel, Vec<Declaration>, Vec<Replayed>) {
    let items = collect(ctx);
    let mut k = Kernel::logos(ctx);
    let verdicts = admit_items(&mut k, ctx, &items);
    (k, items, verdicts)
}

/// Admits `items` into `k` in dependency order (each may depend on what `k` holds already): the
/// core's verdict on each declared name, a family by its own name, the quotient by each primitive.
pub(crate) fn admit_items(k: &mut Kernel, ctx: &Context, items: &[Declaration]) -> Vec<Replayed> {
    let mut out = Vec::new();
    for component in order(ctx, items) {
        let members: Vec<&Declaration> = component.iter().map(|i| &items[*i]).collect();
        let verdict = admit_block(k, ctx, &members).map_err(|e| e.to_string());
        for m in &members {
            let names = match m {
                Declaration::Quotient { .. } | Declaration::Observational { .. } => m.names(),
                _ => vec![m.name()],
            };
            for name in names {
                out.push(Replayed {
                    name: name.to_string(),
                    kind: m.kind(),
                    verdict: verdict.clone(),
                });
            }
        }
    }
    out
}

/// Admits one declaration, or one mutual block of inductive families, into a kernel built by
/// [`Kernel::logos`] over `ctx`: the core's verdict on it.
pub fn admit(k: &mut Kernel, ctx: &Context, block: &[Declaration]) -> Result<(), String> {
    let members: Vec<&Declaration> = block.iter().collect();
    admit_block(k, ctx, &members).map_err(|e| e.to_string())
}

fn admit_block(k: &mut Kernel, ctx: &Context, members: &[&Declaration]) -> KResult<()> {
    if members.is_empty() {
        return Ok(());
    }
    if members.len() > 1
        && !members
            .iter()
            .all(|m| matches!(m, Declaration::Inductive { .. }))
    {
        return Err(KernelFailure::reject(
            "part of a dependency cycle that is not a mutual inductive block",
        ));
    }
    match members[0] {
        Declaration::Inductive { .. } => admit_inductives(k, ctx, members),
        Declaration::Definition {
            name,
            params,
            ty,
            body,
        } => admit_definition(k, ctx, name, params, ty, body),
        Declaration::Axiom { name, params, ty } => admit_axiom(k, ctx, name, params, ty),
        Declaration::Quotient {
            primitives,
            equality,
        } => admit_quotient(k, ctx, primitives, equality),
        Declaration::Observational { constants, .. } => constants
            .iter()
            .try_for_each(|(name, params, ty)| admit_axiom(k, ctx, name, params, ty)),
    }
}

/// A declaration's universe parameters: its explicit level telescope, or else the level
/// variables its type mentions, which the `Term` kernel reads as implicitly bound by it
/// (`type_checker::lookup_const` returns such a type with its level variables free).
fn level_params(ctx: &Context, name: &str, ty: &Term) -> Vec<String> {
    match super::logos::level_telescope(ctx, name) {
        Some((ps, _)) => ps,
        None => crate::term::free_level_vars(ty),
    }
}

fn collect(ctx: &Context) -> Vec<Declaration> {
    let names: BTreeSet<&str> = ctx
        .iter_inductives()
        .map(|(n, _)| n)
        .chain(ctx.iter_definitions().map(|(n, _, _)| n))
        .chain(ctx.iter_universe_polys().map(|(n, ..)| n))
        .chain(ctx.iter_declarations().map(|(n, _)| n))
        .collect();
    let mut items: BTreeMap<String, Declaration> = names
        .into_iter()
        .filter_map(|n| Some((n.to_string(), item(ctx, n)?)))
        .collect();
    group_quotient(ctx, &mut items);
    group_observational(ctx, &mut items);
    items.into_values().collect()
}

/// The declaration `name` is registered as in `ctx` ([`Context::registration`]).
pub(crate) fn item(ctx: &Context, name: &str) -> Option<Declaration> {
    Some(match ctx.registration(name)? {
        Registration::Inductive(arity) => Declaration::Inductive {
            name: name.to_string(),
            params: level_params(ctx, name, arity),
            arity: arity.clone(),
            num_params: ctx.inductive_num_params(name),
            ctors: ctx
                .get_constructors(name)
                .into_iter()
                .map(|(c, t)| (c.to_string(), t.clone()))
                .collect(),
        },
        Registration::Definition(ty, body) => Declaration::Definition {
            name: name.to_string(),
            params: level_params(ctx, name, ty),
            ty: ty.clone(),
            body: body.clone(),
        },
        Registration::UniversePoly(params, ty, body) => Declaration::Definition {
            name: name.to_string(),
            params: params.to_vec(),
            ty: ty.clone(),
            body: body.clone(),
        },
        Registration::Axiom(ty) => Declaration::Axiom {
            name: name.to_string(),
            params: level_params(ctx, name, ty),
            ty: ty.clone(),
        },
    })
}

/// The certified quotient's primitives, by name, with the kind each is certified as.
const QUOTIENT: [(&str, PreludeKind, QuotKind); 4] = [
    ("Quot", PreludeKind::Type, QuotKind::Type),
    ("Quot_mk", PreludeKind::Constructor, QuotKind::Ctor),
    ("Quot_lift", PreludeKind::Lift, QuotKind::Lift),
    ("Quot_ind", PreludeKind::Induction, QuotKind::Ind),
];

/// The quotient a kernel installs for content imported from Lean, by the names `quotient::Kind`
/// certifies.
const INSTALLED_QUOTIENT: [(crate::quotient::Kind, QuotKind); 4] = [
    (crate::quotient::Kind::Type, QuotKind::Type),
    (crate::quotient::Kind::Constructor, QuotKind::Ctor),
    (crate::quotient::Kind::Lift, QuotKind::Lift),
    (crate::quotient::Kind::Induction, QuotKind::Ind),
];

/// Each certified quotient's four primitives, when all of them are among `items` as axioms,
/// become one [`Declaration::Quotient`]: the standard library's, and the one a kernel installs.
fn group_quotient(ctx: &Context, items: &mut BTreeMap<String, Declaration>) {
    let prelude: Vec<(&str, QuotKind)> = QUOTIENT
        .iter()
        .filter(|(name, kind, _)| crate::quotient::prelude_kind(ctx, name) == Some(*kind))
        .map(|(name, _, kind)| (*name, *kind))
        .collect();
    let installed_equality = ctx
        .quotient_primitives
        .get(crate::quotient::Kind::Lift.name())
        .and_then(|c| c.equality.clone());
    let installed: Vec<(&str, QuotKind)> = INSTALLED_QUOTIENT
        .iter()
        .filter(|(kind, _)| {
            ctx.quotient_primitives
                .get(kind.name())
                .is_some_and(|c| c.kind == *kind)
        })
        .map(|(kind, quot)| (kind.name(), *quot))
        .collect();
    for (set, equality) in [
        (prelude, Some("Eq".to_string())),
        (installed, installed_equality),
    ] {
        let whole = set.len() == 4
            && set
                .iter()
                .all(|(name, _)| matches!(items.get(*name), Some(Declaration::Axiom { .. })));
        let Some(equality) = equality.filter(|_| whole) else {
            continue;
        };
        let primitives = set
            .iter()
            .map(|(name, kind)| match items.remove(*name) {
                Some(Declaration::Axiom { name, params, ty }) => (*kind, name, params, ty),
                _ => unreachable!("certified above"),
            })
            .collect();
        items.insert(
            set[0].0.to_string(),
            Declaration::Quotient {
                primitives,
                equality,
            },
        );
    }
}

/// The certified observational constants, when all of them are among `items` as axioms, become
/// one [`Declaration::Observational`].
fn group_observational(ctx: &Context, items: &mut BTreeMap<String, Declaration>) {
    let Some(certificate) = ctx.observational.as_ref() else {
        return;
    };
    if !certificate
        .names()
        .all(|n| matches!(items.get(n), Some(Declaration::Axiom { .. })))
    {
        return;
    }
    let constants: Vec<(String, Vec<String>, Term)> = certificate
        .names()
        .map(|n| match items.remove(n) {
            Some(Declaration::Axiom { name, params, ty }) => (name, params, ty),
            _ => unreachable!("an axiom above"),
        })
        .collect();
    let after = certificate
        .dependencies
        .iter()
        .filter(|d| !constants.iter().any(|c| &c.0 == *d))
        .cloned()
        .collect();
    items.insert(
        constants[0].0.clone(),
        Declaration::Observational { constants, after },
    );
}

/// The items registered under `names` in `ctx`, the certified quotient and observational
/// equality each grouped as one.
pub(crate) fn items_named(ctx: &Context, names: &BTreeSet<String>) -> Vec<Declaration> {
    let mut items: BTreeMap<String, Declaration> = names
        .iter()
        .filter_map(|n| Some((n.clone(), item(ctx, n)?)))
        .collect();
    group_quotient(ctx, &mut items);
    group_observational(ctx, &mut items);
    items.into_values().collect()
}

fn globals(t: &Term, out: &mut BTreeSet<String>) {
    match t {
        Term::Global(n) | Term::Const { name: n, .. } => {
            out.insert(n.clone());
        }
        Term::Pi {
            param_type,
            body_type: body,
            ..
        }
        | Term::Lambda {
            param_type, body, ..
        } => {
            globals(param_type, out);
            globals(body, out);
        }
        Term::App(f, a) => {
            globals(f, out);
            globals(a, out);
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            globals(discriminant, out);
            globals(motive, out);
            cases.iter().for_each(|c| globals(c, out));
        }
        Term::Fix { body, .. } => globals(body, out),
        Term::MutualFix { defs, .. } => defs.iter().for_each(|(_, b)| globals(b, out)),
        Term::Let {
            ty, value, body, ..
        } => {
            globals(ty, out);
            globals(value, out);
            globals(body, out);
        }
        Term::Annotated { term, .. } => globals(term, out),
        Term::Lit(lit) => {
            out.insert(carrier(lit).to_string());
        }
        Term::Sort(_) | Term::Var(_) | Term::Hole => {}
    }
}

/// The family a literal's type is (`logos_tc`'s typing of literals): a declaration holding the
/// literal depends on it.
fn carrier(lit: &Literal) -> &'static str {
    match lit {
        Literal::Int(_) | Literal::BigInt(_) => "Int",
        Literal::Nat(_) => "Nat",
        Literal::Float(_) => "Float",
        Literal::Text(_) => "Text",
        Literal::Duration(_) => "Duration",
        Literal::Date(_) => "Date",
        Literal::Moment(_) => "Moment",
    }
}

/// Every global `item` names, a literal naming its carrier, and for the observational constants
/// every name their rules' answers are built from.
pub(crate) fn item_globals(item: &Declaration) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    item.terms()
        .into_iter()
        .for_each(|t| globals(t, &mut names));
    if let Declaration::Observational { after, .. } = item {
        names.extend(after.iter().cloned());
    }
    names
}

/// The items' strongly connected components, each listed after every component it depends on.
fn order(ctx: &Context, items: &[Declaration]) -> Vec<Vec<usize>> {
    let mut owner: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, item) in items.iter().enumerate() {
        for name in item.names() {
            owner.insert(name, i);
        }
    }
    let globals: Vec<BTreeSet<String>> = items.iter().map(item_globals).collect();
    let mut edges: Vec<Vec<usize>> = globals
        .iter()
        .enumerate()
        .map(|(i, names)| {
            names
                .iter()
                .filter_map(|n| owner.get(n.as_str()).copied())
                .filter(|j| *j != i)
                .collect()
        })
        .collect();
    // An item that names a registered structure's constructor also waits for the structure's
    // projections, which structure η (`Kernel::logos_eta_struct`) reads to compare a constructor
    // application with a value of the structure. A projection that already depends on the item
    // gets no edge back.
    for (i, names) in globals.iter().enumerate() {
        let mut projections: Vec<usize> = names
            .iter()
            .filter_map(|n| ctx.struct_of_constructor(n))
            .flat_map(|(_, info)| info.projections.iter())
            .filter_map(|p| owner.get(p.as_str()).copied())
            .filter(|j| *j != i)
            .collect();
        projections.sort_unstable();
        projections.dedup();
        for j in projections {
            if !edges[i].contains(&j) && !reaches(&edges, j, i) {
                edges[i].push(j);
            }
        }
    }
    tarjan(&edges)
}

/// Whether `to` is reachable from `from` along `edges`.
fn reaches(edges: &[Vec<usize>], from: usize, to: usize) -> bool {
    let mut seen = vec![false; edges.len()];
    let mut work = vec![from];
    while let Some(i) = work.pop() {
        if i == to {
            return true;
        }
        if !std::mem::replace(&mut seen[i], true) {
            work.extend(edges[i].iter().copied());
        }
    }
    false
}

/// Tarjan's algorithm, iterative: components in reverse topological order of the condensation,
/// which with edges pointing at dependencies is dependencies first.
fn tarjan(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let n = edges.len();
    let (mut index, mut low, mut on) = (vec![usize::MAX; n], vec![0; n], vec![false; n]);
    let (mut stack, mut out, mut next) = (Vec::new(), Vec::new(), 0);
    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];
        while let Some(&mut (v, ref mut e)) = work.last_mut() {
            if *e == 0 && index[v] == usize::MAX {
                index[v] = next;
                low[v] = next;
                next += 1;
                stack.push(v);
                on[v] = true;
            }
            if *e < edges[v].len() {
                let w = edges[v][*e];
                *e += 1;
                if index[w] == usize::MAX {
                    work.push((w, 0));
                } else if on[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[v]);
            }
            if low[v] == index[v] {
                let mut component = Vec::new();
                while let Some(w) = stack.pop() {
                    on[w] = false;
                    component.push(w);
                    if w == v {
                        break;
                    }
                }
                component.sort_unstable();
                out.push(component);
            }
        }
    }
    out
}

/// The core's verdict on `block`, a candidate declaration, in `k`, admitting nothing: a definition or
/// an axiom gets every check its admission makes; an inductive block is admitted into a clone of
/// `k`, discarded after.
#[cfg(feature = "shadow")]
pub(crate) fn judge(k: &mut Kernel, ctx: &Context, block: &[Declaration]) -> Result<(), String> {
    let verdict = match block {
        [Declaration::Definition {
            name,
            params,
            ty,
            body,
        }] => judge_definition(k, ctx, name, params, ty, body),
        [Declaration::Axiom { name, params, ty }] => judge_axiom(k, ctx, name, params, ty),
        _ => admit_block(&mut k.clone(), ctx, &block.iter().collect::<Vec<_>>()),
    };
    verdict.map_err(|e| e.to_string())
}

#[cfg(feature = "shadow")]
fn judge_definition(
    k: &mut Kernel,
    ctx: &Context,
    name: &str,
    params: &[String],
    ty: &Term,
    body: &Term,
) -> KResult<()> {
    let header = Header {
        name: name_id(k, name),
        lparams: lparams(k, params),
        ty: lowered(k, ctx, ty)?,
    };
    let value = lowered(k, ctx, body)?;
    let height = definition_height(k, value);
    k.check_without_admitting(&Decl::Defn(DefnDecl {
        header,
        value,
        hint: Hint::Regular(height),
        safety: Safety::Safe,
    }))
}

#[cfg(feature = "shadow")]
fn judge_axiom(
    k: &mut Kernel,
    ctx: &Context,
    name: &str,
    params: &[String],
    ty: &Term,
) -> KResult<()> {
    let header = Header {
        name: name_id(k, name),
        lparams: lparams(k, params),
        ty: lowered(k, ctx, ty)?,
    };
    k.check_without_admitting(&Decl::Axiom {
        header,
        is_unsafe: false,
    })
}

/// The definition a door of `ctx` puts to the core: its terms as `ctx` stores them
/// ([`Context::stored_term`]) and the telescope the door gives it, the stated one or the level
/// variables of the stored type and then of the body (`term`'s rules 1 and 4).
pub fn candidate_definition(
    ctx: &Context,
    name: &str,
    levels: Option<&[String]>,
    ty: &Term,
    body: &Term,
) -> Declaration {
    let (ty, body) = (ctx.stored_term(ty.clone()), ctx.stored_term(body.clone()));
    let params = match levels {
        Some(levels) => levels.to_vec(),
        None => crate::term::definition_telescope(&ty, &body)
            .unwrap_or_else(|| crate::term::free_level_vars(&ty)),
    };
    Declaration::Definition {
        name: name.to_string(),
        params,
        ty,
        body,
    }
}

/// The axiom a door of `ctx` puts to the core, its type stored and its levels read as
/// [`candidate_definition`] reads them.
pub fn candidate_axiom(
    ctx: &Context,
    name: &str,
    levels: Option<&[String]>,
    ty: &Term,
) -> Declaration {
    let ty = ctx.stored_term(ty.clone());
    Declaration::Axiom {
        name: name.to_string(),
        params: levels.map_or_else(|| crate::term::free_level_vars(&ty), <[String]>::to_vec),
        ty,
    }
}

/// The block of inductive families a door of `ctx` puts to the core. Each arity is stored as
/// `ctx` stores it, and each member takes the stated telescope or its stored arity's level
/// variables. A constructor's bare reference to a member stands at that member's telescope, and
/// one to anything else at the level `ctx` gives it.
pub fn candidate_block(
    ctx: &Context,
    levels: Option<&[String]>,
    block: &[crate::context::MutualInductive],
) -> Vec<Declaration> {
    let arities: Vec<Term> = block
        .iter()
        .map(|m| ctx.stored_term(m.sort.clone()))
        .collect();
    let telescopes: Vec<Vec<String>> = arities
        .iter()
        .map(|arity| levels.map_or_else(|| crate::term::free_level_vars(arity), <[String]>::to_vec))
        .collect();
    let own_levels = |name: &str| match block.iter().position(|m| m.name == name) {
        Some(i) => (!telescopes[i].is_empty()).then(|| telescopes[i].clone()),
        None => ctx.own_levels(name),
    };
    block
        .iter()
        .zip(arities)
        .zip(&telescopes)
        .map(|((m, arity), params)| Declaration::Inductive {
            name: m.name.clone(),
            params: params.clone(),
            arity,
            num_params: m.num_params,
            ctors: m
                .constructors
                .iter()
                .map(|(c, t)| {
                    let mut t = crate::annotation::stored(t.clone());
                    crate::term::bare_references_at_own_levels(&mut t, &own_levels);
                    (c.clone(), t)
                })
                .collect(),
        })
        .collect()
}

fn lowered(k: &mut Kernel, ctx: &Context, t: &Term) -> KResult<ExprId> {
    lower(&mut k.s, &explicit_levels(ctx, t))
        .map_err(|e| KernelFailure::reject(format!("no core term: {e:?}")))
}

fn lparams(k: &mut Kernel, params: &[String]) -> Rc<[NameId]> {
    params
        .iter()
        .map(|p| k.s.names.str(NameId::ANON, p))
        .collect::<Vec<_>>()
        .into()
}

fn name_id(k: &mut Kernel, name: &str) -> NameId {
    k.s.names.str(NameId::ANON, name)
}

fn admit_axiom(
    k: &mut Kernel,
    ctx: &Context,
    name: &str,
    params: &[String],
    ty: &Term,
) -> KResult<()> {
    let header = Header {
        name: name_id(k, name),
        lparams: lparams(k, params),
        ty: lowered(k, ctx, ty)?,
    };
    k.add_decl(Decl::Axiom {
        header,
        is_unsafe: false,
    })
}

/// The certified quotient, admitted as the core's own: each primitive checked against its
/// canonical signature, and the four installed before anything that uses one of them.
fn admit_quotient(
    k: &mut Kernel,
    ctx: &Context,
    primitives: &[(QuotKind, String, Vec<String>, Term)],
    equality: &str,
) -> KResult<()> {
    let eq = name_id(k, equality);
    for (kind, name, params, ty) in primitives {
        let header = Header {
            name: name_id(k, name),
            lparams: lparams(k, params),
            ty: lowered(k, ctx, ty)?,
        };
        k.add_quotient(header, *kind, eq)?;
    }
    Ok(())
}

fn admit_definition(
    k: &mut Kernel,
    ctx: &Context,
    name: &str,
    params: &[String],
    ty: &Term,
    body: &Term,
) -> KResult<()> {
    let header = Header {
        name: name_id(k, name),
        lparams: lparams(k, params),
        ty: lowered(k, ctx, ty)?,
    };
    let value = lowered(k, ctx, body)?;
    let height = definition_height(k, value);
    k.add_decl(Decl::Defn(DefnDecl {
        header,
        value,
        hint: Hint::Regular(height),
        safety: Safety::Safe,
    }))
}

/// One more than the tallest definition `value` names (Lean's definitional height).
fn definition_height(k: &Kernel, value: ExprId) -> u32 {
    let mut h = 0;
    k.s.any_node(value, &mut |s, x| {
        if let Node::Const(n, _) = s.node(x) {
            if let Some(c) = k.env.get(n) {
                if let Hint::Regular(d) = c.hint() {
                    h = h.max(d);
                }
            }
        }
        false
    });
    h + 1
}

/// A type's Π-telescope opened onto fresh locals, and the type it ends in.
fn telescope(k: &mut Kernel, mut ty: ExprId) -> KResult<(Vec<ExprId>, ExprId)> {
    let mut locals = Vec::new();
    loop {
        let w = k.whnf(ty)?;
        match k.s.node(w) {
            Node::Pi(a, b) => {
                let x = k.push_local(a, None);
                locals.push(x);
                ty = k.s.instantiate1(b, x);
            }
            _ => return Ok((locals, w)),
        }
    }
}

fn admit_inductives(k: &mut Kernel, ctx: &Context, block: &[&Declaration]) -> KResult<()> {
    let names: Vec<&str> = block.iter().map(|i| i.name()).collect();
    let mut types = Vec::new();
    for item in block {
        let Declaration::Inductive {
            name,
            params,
            arity,
            num_params,
            ctors,
        } = item
        else {
            unreachable!()
        };
        let n = name_id(k, name);
        let lps = lparams(k, params);
        k.check_name(n, true)?;
        let ty = lowered(k, ctx, arity)?;
        let level = k.with_decl(lps.clone(), Safety::Safe, |k| {
            let s = k.check(ty)?;
            k.ensure_sort(s)?;
            let (locals, end) = telescope(k, ty)?;
            match k.s.node(end) {
                Node::Sort(l) => Ok((l, locals.len() as u32)),
                _ => Err(KernelFailure::reject(format!(
                    "the arity of '{name}' does not end in a sort"
                ))),
            }
        })?;
        types.push((
            n,
            lps,
            ty,
            level,
            *num_params as u32,
            ctors.clone(),
            params.clone(),
        ));
    }
    for (n, lps, ty, (_, arity_len), num_params, ..) in &types {
        let info = InductiveInfo {
            num_params: *num_params,
            num_indices: arity_len - num_params,
            all: types.iter().map(|t| t.0).collect::<Vec<_>>().into(),
            ctors: Vec::new().into(),
            num_nested: 0,
            is_rec: false,
            is_reflexive: false,
            is_unsafe: false,
            rec: k.rec_name(*n),
        };
        k.env.add_provisional(Const {
            name: *n,
            lparams: lps.clone(),
            ty: *ty,
            kind: Kind::Induct(info),
        });
    }
    let members: BTreeSet<NameId> = types.iter().map(|t| t.0).collect();
    let mut finals = Vec::new();
    let result = (|| -> KResult<()> {
        for (n, lps, ind_ty, (level, arity_len), num_params, ctors, family_levels) in &types {
            let mut ctor_ids = Vec::new();
            let (mut is_rec, mut is_reflexive) = (false, false);
            for (cidx, (cname, cty)) in ctors.iter().enumerate() {
                let c = name_id(k, cname);
                k.check_name(c, true)?;
                // A constructor the context registers takes the telescope it was registered with; a
                // candidate's constructor, not in the context yet, takes its block's.
                let own_levels = super::logos::level_telescope(ctx, cname)
                    .map_or_else(|| family_levels.clone(), |(ps, _)| ps);
                if own_levels != *family_levels {
                    return Err(KernelFailure::reject(format!(
                        "constructor '{cname}' takes the universe parameters {own_levels:?}, its family {family_levels:?}"
                    )));
                }
                crate::positivity::check_positivity_mutual_resolved(ctx, &names, cname, cty)
                    .map_err(|e| KernelFailure::reject(e.to_string()))?;
                let ty = lowered(k, ctx, cty)?;
                let shape = k.with_decl(lps.clone(), Safety::Safe, |k| {
                    check_constructor(
                        k,
                        cname,
                        *n,
                        lps,
                        *ind_ty,
                        *level,
                        *num_params,
                        *arity_len,
                        ty,
                        &members,
                    )
                })?;
                is_rec |= shape.recursive;
                is_reflexive |= shape.reflexive;
                ctor_ids.push(c);
                finals.push(Const {
                    name: c,
                    lparams: lps.clone(),
                    ty,
                    kind: Kind::Ctor(CtorInfo {
                        induct: *n,
                        cidx: cidx as u32,
                        num_params: *num_params,
                        num_fields: shape.fields,
                        is_unsafe: false,
                    }),
                });
            }
            let provisional = k.env.get(*n).expect("provisional").clone();
            let Kind::Induct(mut info) = provisional.kind.clone() else {
                unreachable!()
            };
            info.ctors = ctor_ids.into();
            info.is_rec = is_rec;
            info.is_reflexive = is_reflexive;
            finals.push(Const {
                name: *n,
                lparams: lps.clone(),
                ty: provisional.ty,
                kind: Kind::Induct(info),
            });
        }
        Ok(())
    })();
    for (n, ..) in &types {
        k.env.remove(*n);
    }
    result?;
    let irrelevant = irrelevant_levels(k, &finals);
    for c in finals {
        if let Some(mask) = irrelevant.get(&c.name) {
            k.record_irrelevant_levels(c.name, *mask);
        }
        k.env.add(c);
    }
    Ok(())
}

/// What a constructor's check found: its field count, and whether a field mentions the block
/// (recursive) or does so as a function's result (reflexive).
struct CtorShape {
    fields: u32,
    recursive: bool,
    reflexive: bool,
}

/// Lean's constructor check, over LOGOS's universes: the constructor's parameter binders are the
/// family's, each field lives in a sort at most the family's unless the family is a proposition,
/// and the result is the family applied to exactly its parameters and then its indices.
#[allow(clippy::too_many_arguments)]
fn check_constructor(
    k: &mut Kernel,
    cname: &str,
    family: NameId,
    lps: &Rc<[NameId]>,
    family_ty: ExprId,
    level: LevelId,
    num_params: u32,
    arity_len: u32,
    ty: ExprId,
    members: &BTreeSet<NameId>,
) -> KResult<CtorShape> {
    let reject = |what: String| KernelFailure::reject(format!("constructor '{cname}' {what}"));
    let s = k.check(ty)?;
    k.ensure_sort(s)?;
    let mut arity = family_ty;
    let mut params = Vec::with_capacity(num_params as usize);
    let mut cur = ty;
    for i in 0..num_params {
        let aw = k.whnf(arity)?;
        let Node::Pi(param_ty, arity_body) = k.s.node(aw) else {
            return Err(reject("outruns its family's parameters".into()));
        };
        let cw = k.whnf(cur)?;
        let Node::Pi(dom, body) = k.s.node(cw) else {
            return Err(reject(format!(
                "has fewer than its family's {num_params} parameters"
            )));
        };
        if !k.is_def_eq(dom, param_ty)? {
            return Err(reject(format!(
                "takes its parameter {i} at a type other than its family's"
            )));
        }
        let p = k.push_local(param_ty, None);
        params.push(p);
        arity = k.s.instantiate1(arity_body, p);
        cur = k.s.instantiate1(body, p);
    }
    let mentions_block = |k: &Kernel, e: ExprId| {
        k.s.any_node(
            e,
            &mut |s, x| matches!(s.node(x), Node::Const(m, _) if members.contains(&m)),
        )
    };
    let mut shape = CtorShape {
        fields: 0,
        recursive: false,
        reflexive: false,
    };
    loop {
        let cw = k.whnf(cur)?;
        let Node::Pi(dom, body) = k.s.node(cw) else {
            cur = cw;
            break;
        };
        shape.fields += 1;
        if mentions_block(k, dom) {
            shape.recursive = true;
            let dw = k.whnf(dom)?;
            shape.reflexive |= matches!(k.s.node(dw), Node::Pi(..));
        }
        if level != LevelId::ZERO && !k.s.levels.is_sprop(level) {
            let fs = k.infer(dom, true)?;
            let fs = k.ensure_sort(fs)?;
            let fl = k.sort_level(fs);
            if !k.s.levels.leq(fl, level) {
                return Err(KernelFailure::reject(format!(
                    "universe inconsistency: constructor '{cname}' stores an argument in a sort above its inductive's"
                )));
            }
        }
        let x = k.push_local(dom, None);
        cur = k.s.instantiate1(body, x);
    }
    let (head, args) = k.s.app_args(cur);
    let identity = k.const_levels_of(lps);
    let returns_family =
        matches!(k.s.node(head), Node::Const(m, ls) if m == family && ls == identity);
    if !returns_family || args.len() != arity_len as usize || args[..params.len()] != params[..] {
        return Err(reject(format!(
            "must return its family applied to its parameters and then {} indices, not {}",
            arity_len - num_params,
            k.show(cur)
        )));
    }
    for index in &args[params.len()..] {
        if mentions_block(k, *index) {
            return Err(reject(
                "mentions its family in an index of its result".into(),
            ));
        }
    }
    Ok(shape)
}

/// The universe parameters of a block's inductives that occur nowhere past their parameters —
/// neither in an arity's indices or sort nor in a constructor's fields or result indices — as a
/// mask over each constant's level list. The block's own recursive references at their declared
/// instance do not count. Such a parameter is irrelevant in the sense of cumulative inductive
/// types (Timany–Sozeau): with the parameters applied, the instances it ranges over denote one
/// family.
fn irrelevant_levels(k: &mut Kernel, block: &[Const]) -> BTreeMap<NameId, u64> {
    let members: BTreeSet<NameId> = block
        .iter()
        .filter(|c| matches!(c.kind, Kind::Induct(_)))
        .map(|c| c.name)
        .collect();
    let mut declared = BTreeMap::new();
    for c in block.iter().filter(|c| members.contains(&c.name)) {
        let ls = k.const_levels_of(&c.lparams);
        declared.insert(c.name, ls);
    }
    let mut mentioned: BTreeSet<NameId> = BTreeSet::new();
    for c in block {
        let num_params = match &c.kind {
            Kind::Induct(info) => info.num_params,
            Kind::Ctor(ci) => ci.num_params,
            _ => continue,
        };
        let mut rest = c.ty;
        for _ in 0..num_params {
            match k.s.node(rest) {
                Node::Pi(_, b) => rest = b,
                _ => break,
            }
        }
        for p in c.lparams.iter() {
            let s = &k.s;
            if s.any_node(rest, &mut |s, x| match s.node(x) {
                Node::Sort(l) => s.levels.mentions_param(l, *p),
                Node::Const(n, ls) if declared.get(&n) != Some(&ls) => s
                    .levels
                    .get_list(ls)
                    .iter()
                    .any(|l| s.levels.mentions_param(*l, *p)),
                _ => false,
            }) {
                mentioned.insert(*p);
            }
        }
    }
    block
        .iter()
        .map(|c| {
            let mask = c
                .lparams
                .iter()
                .enumerate()
                .filter(|(i, p)| *i < 64 && !mentioned.contains(p))
                .fold(0u64, |m, (i, _)| m | 1 << i);
            (c.name, mask)
        })
        .collect()
}
