//! LOGOS's own terms in the core's store: [`lower`] takes a [`Term`] of LOGOS's Calculus of
//! Inductive Constructions to an [`ExprId`], and [`lift`] takes it back.
//!
//! Named variables become de Bruijn indices, a global becomes a constant (one name component, as
//! written), and LOGOS's universes become levels: `Prop` is `0`, `Type n` is `n + 1`, `SProp` is
//! [`super::level::LevelNode::SProp`]. `match`, `fix` and mutual `fix` are their
//! own nodes, and the literals keep their LOGOS meaning (`Nat` and `Text` as the core's `Nat` and
//! string literals, the rest as [`Lit`]). An annotation is transparent and is not kept; a hole is
//! not a kernel term and is refused.
//!
//! `lift` names a variable after the depth of its binder (`v0` outermost), so a lifted term is
//! α-equivalent to the term that was lowered, and lowering it again gives the same id. It names the
//! kernel's local `i` `fvar{i}`, which [`lower_open`] takes back to that local.

use super::expr::{ExprId, Lit, Node, Store};
use super::level::{LevelId, LevelNode, LevelsId};
use super::name::{NameId, NameNode};
use super::nat::Nat;
use crate::term::{Literal, Term, Universe};

/// Why a term has no image in the core.
#[derive(Clone, Debug, PartialEq)]
pub enum LowerError {
    /// A variable no enclosing binder binds.
    Unbound(String),
    /// An unelaborated metavariable.
    Hole,
}

/// The core expression for a closed LOGOS term.
pub fn lower(store: &mut Store, t: &Term) -> Result<ExprId, LowerError> {
    Lowering {
        store,
        scope: Vec::new(),
        open: false,
    }
    .term(t)
}

/// The core expression for a LOGOS term over the kernel's locals, named as [`lift`] names them.
pub fn lower_open(store: &mut Store, t: &Term) -> Result<ExprId, LowerError> {
    Lowering {
        store,
        scope: Vec::new(),
        open: true,
    }
    .term(t)
}

/// The local `lift` names `v`, if it names one.
fn local_named(v: &str) -> Option<u32> {
    let i: u32 = v.strip_prefix("fvar")?.parse().ok()?;
    (local_name(i) == v).then_some(i)
}

fn local_name(i: u32) -> String {
    format!("fvar{i}")
}

/// The core level for a LOGOS universe.
pub fn lower_universe(store: &mut Store, u: &Universe) -> LevelId {
    match u {
        Universe::SProp => store.levels.sprop(),
        Universe::Prop => LevelId::ZERO,
        Universe::Type(n) => store.levels.of_nat(*n as u64 + 1),
        Universe::Var(v) => {
            let n = store.names.str(NameId::ANON, v);
            store.levels.param(n)
        }
        Universe::Succ(a) => {
            let a = lower_universe(store, a);
            store.levels.succ(a)
        }
        Universe::Max(a, b) => {
            let (a, b) = (lower_universe(store, a), lower_universe(store, b));
            store.levels.max_raw(a, b)
        }
        Universe::IMax(a, b) => {
            let (a, b) = (lower_universe(store, a), lower_universe(store, b));
            store.levels.imax_raw(a, b)
        }
    }
}

struct Lowering<'s> {
    store: &'s mut Store,
    /// The binder names in scope, innermost last.
    scope: Vec<String>,
    /// Whether a free variable may name one of the kernel's locals.
    open: bool,
}

impl Lowering<'_> {
    fn under<T>(&mut self, names: &[&String], f: impl FnOnce(&mut Self) -> T) -> T {
        self.scope.extend(names.iter().map(|n| (*n).clone()));
        let r = f(self);
        self.scope.truncate(self.scope.len() - names.len());
        r
    }

    fn global(&mut self, name: &str, levels: &[Universe]) -> ExprId {
        let n = self.store.names.str(NameId::ANON, name);
        let ls: Vec<LevelId> = levels
            .iter()
            .map(|u| lower_universe(self.store, u))
            .collect();
        self.store.cnst_levels(n, &ls)
    }

    fn term(&mut self, t: &Term) -> Result<ExprId, LowerError> {
        Ok(match t {
            Term::Sort(u) => {
                let l = lower_universe(self.store, u);
                self.store.sort(l)
            }
            Term::Var(v) => match self.scope.iter().rposition(|n| n == v) {
                Some(i) => self.store.bvar((self.scope.len() - 1 - i) as u32),
                None => match local_named(v).filter(|_| self.open) {
                    Some(i) => self.store.fvar(i),
                    None => return Err(LowerError::Unbound(v.clone())),
                },
            },
            Term::Global(n) => self.global(n, &[]),
            Term::Const { name, levels } => self.global(name, levels),
            Term::Pi {
                param,
                param_type,
                body_type,
            } => {
                let a = self.term(param_type)?;
                let b = self.under(&[param], |l| l.term(body_type))?;
                self.store.pi(a, b)
            }
            Term::Lambda {
                param,
                param_type,
                body,
            } => {
                let a = self.term(param_type)?;
                let b = self.under(&[param], |l| l.term(body))?;
                self.store.lam(a, b)
            }
            Term::App(f, a) => {
                let (f, a) = (self.term(f)?, self.term(a)?);
                self.store.app(f, a)
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                let d = self.term(discriminant)?;
                let m = self.term(motive)?;
                let cs = cases
                    .iter()
                    .map(|c| self.term(c))
                    .collect::<Result<Vec<_>, _>>()?;
                self.store.match_(d, m, &cs)
            }
            Term::Fix { name, body } => {
                let b = self.under(&[name], |l| l.term(body))?;
                self.store.fix(b)
            }
            Term::MutualFix { defs, index } => {
                let names: Vec<&String> = defs.iter().map(|(n, _)| n).collect();
                let bodies = self.under(&names, |l| {
                    defs.iter()
                        .map(|(_, b)| l.term(b))
                        .collect::<Result<Vec<_>, _>>()
                })?;
                self.store.mutual_fix(&bodies, *index as u32)
            }
            Term::Let {
                name,
                ty,
                value,
                body,
            } => {
                let (ty, value) = (self.term(ty)?, self.term(value)?);
                let body = self.under(&[name], |l| l.term(body))?;
                self.store.let_(ty, value, body)
            }
            Term::Lit(lit) => match lit {
                Literal::Int(i) => self.store.lit(Lit::Int(*i)),
                Literal::BigInt(b) => self.store.lit(Lit::BigInt(b.clone())),
                Literal::Nat(n) => self.store.nat_lit(Nat::from_bigint(n)),
                Literal::Float(f) => self.store.lit(Lit::Float(f.to_bits())),
                Literal::Text(s) => self.store.str_lit(s),
                Literal::Duration(d) => self.store.lit(Lit::Duration(*d)),
                Literal::Date(d) => self.store.lit(Lit::Date(*d)),
                Literal::Moment(m) => self.store.lit(Lit::Moment(*m)),
            },
            Term::Annotated { term, .. } => self.term(term)?,
            Term::Hole => return Err(LowerError::Hole),
        })
    }
}

/// The LOGOS term for a closed core expression.
pub fn lift(store: &Store, e: ExprId) -> Term {
    lift_at(store, e, 0)
}

/// The LOGOS universe for a core level.
pub fn lift_level(store: &Store, l: LevelId) -> Universe {
    let ls = &store.levels;
    if let Some(n) = concrete(store, l) {
        return if n == 0 {
            Universe::Prop
        } else {
            Universe::Type((n - 1) as u32)
        };
    }
    match ls.node(l) {
        LevelNode::SProp => Universe::SProp,
        LevelNode::Zero => Universe::Prop,
        LevelNode::Succ(a) => Universe::Succ(Box::new(lift_level(store, a))),
        LevelNode::Max(a, b) => Universe::Max(
            Box::new(lift_level(store, a)),
            Box::new(lift_level(store, b)),
        ),
        LevelNode::IMax(a, b) => Universe::IMax(
            Box::new(lift_level(store, a)),
            Box::new(lift_level(store, b)),
        ),
        LevelNode::Param(n) => Universe::Var(component(store, n)),
    }
}

/// `succ` applied `n` times to `0`.
fn concrete(store: &Store, mut l: LevelId) -> Option<u64> {
    let mut n = 0u64;
    loop {
        match store.levels.node(l) {
            LevelNode::Zero => return Some(n),
            LevelNode::Succ(a) => {
                n += 1;
                l = a;
            }
            _ => return None,
        }
    }
}

/// A core name as the `Term` kernel spells it.
pub(super) fn component(store: &Store, n: NameId) -> String {
    match store.names.node(n) {
        NameNode::Str(p, s) if *p == NameId::ANON => store.names.component(*s).to_string(),
        _ => store.names.display(n),
    }
}

fn var(level: u32) -> String {
    format!("v{level}")
}

fn lift_at(store: &Store, e: ExprId, depth: u32) -> Term {
    let bx = |t: Term| Box::new(t);
    match store.node(e) {
        Node::BVar(i) => Term::Var(var(depth - 1 - i)),
        Node::FVar(i) => Term::Var(local_name(i)),
        Node::Sort(l) => Term::Sort(lift_level(store, l)),
        Node::Const(n, ls) => {
            let levels: Vec<Universe> = store
                .levels
                .get_list(ls)
                .iter()
                .map(|l| lift_level(store, *l))
                .collect();
            if ls == LevelsId::EMPTY {
                Term::Global(component(store, n))
            } else {
                Term::Const {
                    name: component(store, n),
                    levels,
                }
            }
        }
        Node::App(f, a) => Term::App(bx(lift_at(store, f, depth)), bx(lift_at(store, a, depth))),
        Node::Lam(t, b) => Term::Lambda {
            param: var(depth),
            param_type: bx(lift_at(store, t, depth)),
            body: bx(lift_at(store, b, depth + 1)),
        },
        Node::Pi(t, b) => Term::Pi {
            param: var(depth),
            param_type: bx(lift_at(store, t, depth)),
            body_type: bx(lift_at(store, b, depth + 1)),
        },
        Node::Let(t, v, b) => Term::Let {
            name: var(depth),
            ty: bx(lift_at(store, t, depth)),
            value: bx(lift_at(store, v, depth)),
            body: bx(lift_at(store, b, depth + 1)),
        },
        Node::NatLit(n) => Term::Lit(Literal::Nat(store.nat(n).to_bigint())),
        Node::StrLit(s) => Term::Lit(Literal::Text(store.string(s).to_string())),
        Node::Lit(l) => Term::Lit(match store.lit_value(l) {
            Lit::Int(i) => Literal::Int(*i),
            Lit::BigInt(b) => Literal::BigInt(b.clone()),
            Lit::Float(bits) => Literal::Float(f64::from_bits(*bits)),
            Lit::Duration(d) => Literal::Duration(*d),
            Lit::Date(d) => Literal::Date(*d),
            Lit::Moment(m) => Literal::Moment(*m),
        }),
        Node::Match(d, m, cs) => Term::Match {
            discriminant: bx(lift_at(store, d, depth)),
            motive: bx(lift_at(store, m, depth)),
            cases: store
                .seq_items(cs)
                .iter()
                .map(|c| lift_at(store, *c, depth))
                .collect(),
        },
        Node::Fix(b) => Term::Fix {
            name: var(depth),
            body: bx(lift_at(store, b, depth + 1)),
        },
        Node::MutualFix(ds, index) => {
            let bodies = store.seq_items(ds);
            let n = bodies.len() as u32;
            Term::MutualFix {
                defs: bodies
                    .iter()
                    .enumerate()
                    .map(|(i, b)| (var(depth + i as u32), lift_at(store, *b, depth + n)))
                    .collect(),
                index: index as usize,
            }
        }
        Node::Proj(..) => {
            unreachable!("LOGOS has no projection node: a Lean projection is not a LOGOS term")
        }
        Node::Seq(..) | Node::Nil => unreachable!("a sequence is lifted by the node that holds it"),
    }
}

/// The levels the `Term` kernel's application rule infers for a bare reference to a
/// universe-polymorphic declaration (`type_checker::implicit_levels_for_head`), written out. The
/// declaration's level parameters become metavariables; left to right, each argument whose domain
/// mentions one solves it from the argument's type; a head whose every level is solved becomes
/// `Const` at those levels. A head left with an unsolved level stays a bare reference, which the
/// core refuses rather than guesses at.
///
/// This is elaboration, outside the trusted core: the core checks the explicit term it produces,
/// so a level chosen wrongly here can only make a declaration fail, never let one through.
pub fn explicit_levels(ctx: &crate::Context, t: &Term) -> Term {
    let bx = |t: Term| Box::new(t);
    match t {
        Term::App(..) => {
            let mut args = Vec::new();
            let mut head = t;
            while let Term::App(f, a) = head {
                args.push(a.as_ref());
                head = f;
            }
            args.reverse();
            let args: Vec<Term> = args.iter().map(|a| explicit_levels(ctx, a)).collect();
            let head = match head {
                Term::Global(n) => solve_levels(ctx, n, &args).unwrap_or_else(|| head.clone()),
                other => explicit_levels(ctx, other),
            };
            args.into_iter().fold(head, |f, a| Term::App(bx(f), bx(a)))
        }
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            let a = explicit_levels(ctx, param_type);
            let inner = ctx.extend(param, a.clone());
            Term::Pi {
                param: param.clone(),
                param_type: bx(a),
                body_type: bx(explicit_levels(&inner, body_type)),
            }
        }
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            let a = explicit_levels(ctx, param_type);
            let inner = ctx.extend(param, a.clone());
            Term::Lambda {
                param: param.clone(),
                param_type: bx(a),
                body: bx(explicit_levels(&inner, body)),
            }
        }
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            let ty2 = explicit_levels(ctx, ty);
            let value2 = explicit_levels(ctx, value);
            let inner = ctx.extend(name, ty2.clone());
            Term::Let {
                name: name.clone(),
                ty: bx(ty2),
                value: bx(value2),
                body: bx(explicit_levels(&inner, body)),
            }
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: bx(explicit_levels(ctx, discriminant)),
            motive: bx(explicit_levels(ctx, motive)),
            cases: cases.iter().map(|c| explicit_levels(ctx, c)).collect(),
        },
        Term::Fix { name, body } => {
            let inner = match crate::type_checker::infer_type(ctx, t) {
                Ok(ty) => ctx.extend(name, ty),
                Err(_) => return t.clone(),
            };
            Term::Fix {
                name: name.clone(),
                body: bx(explicit_levels(&inner, body)),
            }
        }
        Term::MutualFix { defs, index } => {
            let mut inner = ctx.clone();
            for (i, (name, _)) in defs.iter().enumerate() {
                let member = Term::MutualFix {
                    defs: defs.clone(),
                    index: i,
                };
                match crate::type_checker::infer_type(ctx, &member) {
                    Ok(ty) => inner = inner.extend(name, ty),
                    Err(_) => return t.clone(),
                }
            }
            Term::MutualFix {
                defs: defs
                    .iter()
                    .map(|(n, b)| (n.clone(), explicit_levels(&inner, b)))
                    .collect(),
                index: *index,
            }
        }
        Term::Annotated { term, .. } => explicit_levels(ctx, term),
        other => other.clone(),
    }
}

/// The explicit reference `name.{levels}` the application rule solves from `args`, if it solves
/// every level.
fn solve_levels(ctx: &crate::Context, name: &str, args: &[Term]) -> Option<Term> {
    let (params, ty) = level_telescope(ctx, name)?;
    if params.is_empty() {
        return None;
    }
    let metas = crate::term::fresh_level_metas(name, &params);
    let mut cur = crate::term::instantiate_universes(&ty, &metas);
    let mut solved = std::collections::HashMap::new();
    for a in args {
        if !matches!(cur, Term::Pi { .. }) {
            cur = crate::reduction::normalize(ctx, &cur);
        }
        let Term::Pi {
            param_type,
            body_type,
            ..
        } = &cur
        else {
            break;
        };
        if crate::term::mentions_level_meta(param_type) {
            if let Ok(at) = crate::type_checker::infer_type(ctx, a) {
                let at = if matches!(at, Term::Sort(_)) {
                    at
                } else {
                    crate::reduction::normalize(ctx, &at)
                };
                let mut step = std::collections::HashMap::new();
                crate::term::unify_levels(param_type, &at, &mut step);
                for (k, v) in step {
                    solved.entry(k).or_insert(v);
                }
            }
        }
        cur = crate::term::instantiate_universes(body_type, &solved);
    }
    let levels: Vec<Universe> = params
        .iter()
        .map(|p| metas[p].substitute(&solved))
        .collect();
    let unsolved = levels
        .iter()
        .any(|u| crate::term::universe_mentions(u, &crate::term::is_level_meta));
    (!unsolved).then(|| Term::Const {
        name: name.to_string(),
        levels,
    })
}

/// A global's universe parameters and type: its explicit level telescope, or else the level
/// variables its type mentions, which the `Term` kernel reads as bound by the declaration.
pub fn level_telescope(ctx: &crate::Context, name: &str) -> Option<(Vec<String>, Term)> {
    if let Some((ps, ty)) = ctx.poly_declaration(name) {
        return Some((ps.to_vec(), ty.clone()));
    }
    let ty = ctx.get_global(name)?;
    Some((crate::term::free_level_vars(ty), ty.clone()))
}
