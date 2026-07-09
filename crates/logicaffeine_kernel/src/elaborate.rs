//! R4 — the elaborator: metavariables, unification, and implicit-argument inference.
//!
//! The layer between the surface language and the trusted kernel. A user writes `id 0`
//! and means `id Nat 0`; `length xs` and means `length Nat xs`. The elaborator fills the
//! gaps: it inserts METAVARIABLES (`?A`, …) for the omitted/implicit arguments and SOLVES
//! them by UNIFICATION against the types it infers, producing a fully-explicit kernel
//! `Term` that `infer_type` then re-checks. Nothing here is trusted — elaboration only
//! *constructs* a term; the kernel still certifies it.
//!
//! A metavariable is represented as a `Term::Var` whose name starts with `?` (a character
//! no real binder uses), so the whole machinery rides on the existing `Term` with no new
//! variant. Scope: first-order unification with the occurs-check, and implicit-argument
//! insertion driven by an explicit implicit/explicit mask — the core that makes the
//! surface usable. (Higher-order / pattern unification is the next layer.)

use std::collections::HashMap;

use crate::error::{KernelError, KernelResult};
use crate::term::Term;
use crate::type_checker::substitute;
use crate::{infer_type, normalize, Context};

/// Reserved head marking a surface anonymous constructor `⟨e₁, …, eₙ⟩` (E3). The parser
/// emits `⟨anon⟩ e₁ … eₙ`; the elaborator (this module) rewrites it to the sole constructor
/// of the expected inductive and then discards the marker — it NEVER reaches a kernel term.
/// The name contains `⟨`/`⟩`, characters the identifier lexer cannot produce, so it can
/// never collide with a user global. Like the `?`-prefixed metavariables above, this rides
/// on the existing `Term` with no new variant.
pub const ANON_CTOR_MARKER: &str = "⟨anon⟩";

/// Reserved head marking surface dot notation `receiver.field` (E4). The parser emits
/// `⟨proj⟩ receiver field` (the field name carried as a `Global`); the elaborator rewrites it
/// to the projection `H_field params… receiver` and discards the marker. Collision-proof for
/// the same reason as [`ANON_CTOR_MARKER`].
pub const DOT_MARKER: &str = "⟨proj⟩";

/// A surface-sugar application recognized by its reserved marker head.
enum Sugar<'a> {
    /// `⟨e₁, …, eₙ⟩` — an anonymous constructor with these component terms.
    AnonCtor(Vec<&'a Term>),
    /// `receiver.field` — a projection.
    Dot(&'a Term, &'a str),
}

/// Recognize a marker-headed surface-sugar application (anonymous constructor or dot
/// notation), returning its shape, or `None` for an ordinary term. Decomposing the spine
/// here means a bare nullary `⟨⟩` (just the marker global) and a nested one are both caught.
fn as_surface_sugar(term: &Term) -> Option<Sugar<'_>> {
    // Fast path (the common case): walk to the spine head WITHOUT allocating and bail unless
    // it is a marker — so an ordinary term costs only a pointer chase per `elaborate_in` call.
    let mut head = term;
    while let Term::App(f, _) = head {
        head = f;
    }
    let is_anon = matches!(head, Term::Global(n) if n == ANON_CTOR_MARKER);
    let is_dot = matches!(head, Term::Global(n) if n == DOT_MARKER);
    if !is_anon && !is_dot {
        return None;
    }
    // A marker head — now decompose the (rare) spine.
    let mut args: Vec<&Term> = Vec::new();
    let mut cur = term;
    while let Term::App(f, a) = cur {
        args.push(a);
        cur = f;
    }
    args.reverse();
    if is_anon {
        Some(Sugar::AnonCtor(args))
    } else if let [_, Term::Global(field)] = args.as_slice() {
        Some(Sugar::Dot(args[0], field))
    } else {
        None
    }
}

/// Elaborate a marker-headed surface-sugar term to a fully-explicit kernel term paired with
/// its inferred type. Shared by [`elaborate_in`] (so NESTED sugar inside a field or receiver
/// resolves with its expected type) and [`elab_surface`] (the top-level surface entry).
fn elaborate_sugar(
    ctx: &Context,
    mctx: &mut MetaCtx,
    sugar: Sugar<'_>,
    expected: Option<&Term>,
) -> KernelResult<(Term, Term)> {
    let result = match sugar {
        Sugar::AnonCtor(comps) => {
            let expected = expected.ok_or_else(|| {
                KernelError::CertificationError(
                    "anonymous constructor `⟨…⟩` needs a known expected type to choose its \
                     inductive — annotate it or use it where a type is expected"
                        .to_string(),
                )
            })?;
            let comps: Vec<Term> = comps.into_iter().cloned().collect();
            elaborate_anon_ctor(ctx, mctx, expected, &comps)?
        }
        Sugar::Dot(receiver, field) => elaborate_dot(ctx, mctx, receiver, field)?,
    };
    let ty = infer_type(ctx, &result)?;
    Ok((result, ty))
}

/// The metavariable context: fresh-name supply + the substitution being solved.
#[derive(Debug, Default, Clone)]
pub struct MetaCtx {
    solutions: HashMap<String, Term>,
    counter: usize,
}

impl MetaCtx {
    pub fn new() -> Self {
        MetaCtx::default()
    }

    /// A fresh, unsolved metavariable `?n`.
    pub fn fresh(&mut self) -> Term {
        let m = Term::Var(format!("?{}", self.counter));
        self.counter += 1;
        m
    }

    /// The solution recorded for metavariable `name`, if any.
    pub fn solution(&self, name: &str) -> Option<&Term> {
        self.solutions.get(name)
    }

    /// Directly bind metavariable `name := term`, WITHOUT normalizing `term`. Used when
    /// the term is already known well-typed and should be kept structured (e.g. a resolved
    /// typeclass instance `list_inst Nat (mk Nat Zero)`, which must not be δ-unfolded into
    /// its body). The kernel re-checks the assembled term regardless.
    pub fn solve(&mut self, name: &str, term: Term) {
        self.solutions.insert(name.to_string(), term);
    }
}

/// Whether `name` denotes a metavariable (the `?`-prefix convention).
pub fn is_meta(name: &str) -> bool {
    name.starts_with('?')
}

/// Substitute every solved metavariable throughout `term` (transitively, since a solution
/// may itself mention other metavariables — the occurs-check keeps this terminating).
pub fn instantiate(term: &Term, mctx: &MetaCtx) -> Term {
    match term {
        Term::Var(name) if is_meta(name) => match mctx.solutions.get(name) {
            Some(sol) => instantiate(sol, mctx),
            None => term.clone(),
        },
        Term::Var(_) | Term::Global(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole => term.clone(),
        Term::Const { name, levels } => {
            Term::Const { name: name.clone(), levels: levels.clone() }
        }
        Term::Pi { param, param_type, body_type } => Term::Pi {
            param: param.clone(),
            param_type: Box::new(instantiate(param_type, mctx)),
            body_type: Box::new(instantiate(body_type, mctx)),
        },
        Term::Lambda { param, param_type, body } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(instantiate(param_type, mctx)),
            body: Box::new(instantiate(body, mctx)),
        },
        Term::App(f, a) => {
            Term::App(Box::new(instantiate(f, mctx)), Box::new(instantiate(a, mctx)))
        }
        Term::Match { discriminant, motive, cases } => Term::Match {
            discriminant: Box::new(instantiate(discriminant, mctx)),
            motive: Box::new(instantiate(motive, mctx)),
            cases: cases.iter().map(|c| instantiate(c, mctx)).collect(),
        },
        Term::Fix { name, body } => {
            Term::Fix { name: name.clone(), body: Box::new(instantiate(body, mctx)) }
        }
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs.iter().map(|(n, b)| (n.clone(), instantiate(b, mctx))).collect(),
            index: *index,
        },
        Term::Let { name, ty, value, body } => Term::Let {
            name: name.clone(),
            ty: Box::new(instantiate(ty, mctx)),
            value: Box::new(instantiate(value, mctx)),
            body: Box::new(instantiate(body, mctx)),
        },
    }
}

/// Whether the metavariable `m` occurs in `term` — the occurs-check that keeps the
/// solution acyclic (`?m := f ?m` would loop).
fn occurs(m: &str, term: &Term) -> bool {
    match term {
        Term::Var(name) => name == m,
        Term::Global(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole | Term::Const { .. } => false,
        Term::Pi { param_type, body_type, .. } => occurs(m, param_type) || occurs(m, body_type),
        Term::Lambda { param_type, body, .. } => occurs(m, param_type) || occurs(m, body),
        Term::App(f, a) => occurs(m, f) || occurs(m, a),
        Term::Match { discriminant, motive, cases } => {
            occurs(m, discriminant) || occurs(m, motive) || cases.iter().any(|c| occurs(m, c))
        }
        Term::Fix { body, .. } => occurs(m, body),
        Term::MutualFix { defs, .. } => defs.iter().any(|(_, b)| occurs(m, b)),
        Term::Let { ty, value, body, .. } => {
            occurs(m, ty) || occurs(m, value) || occurs(m, body)
        }
    }
}

/// `instantiate` then weak-head-normalize — the form unification compares.
fn resolve(ctx: &Context, mctx: &MetaCtx, t: &Term) -> Term {
    normalize(ctx, &instantiate(t, mctx))
}

/// First-order unification of `a` and `b`, solving metavariables into `mctx`. Returns
/// whether they were made equal. A metavariable unifies with anything (after the
/// occurs-check); everything else decomposes structurally.
pub fn unify(ctx: &Context, mctx: &mut MetaCtx, a: &Term, b: &Term) -> bool {
    unify_in(ctx, mctx, &[], a, b)
}

/// Unify `a` and `b` under a LOCAL CONTEXT `lctx` (`(name, type)` of the bound variables
/// in scope). The local context enables higher-order PATTERN (Miller) unification:
/// `?M x̄ =?= t`, where `?M` is a metavariable applied to distinct bound variables `x̄`,
/// is solved by `?M := λx̄. t`. The top-level [`unify`] runs with an empty context, so its
/// behavior — and every existing caller — is the first-order one.
pub fn unify_in(
    ctx: &Context,
    mctx: &mut MetaCtx,
    lctx: &[(String, Term)],
    a: &Term,
    b: &Term,
) -> bool {
    let a = resolve(ctx, mctx, a);
    let b = resolve(ctx, mctx, b);

    if let Term::Var(n) = &a {
        if is_meta(n) {
            return assign(ctx, mctx, n, &b);
        }
    }
    if let Term::Var(n) = &b {
        if is_meta(n) {
            return assign(ctx, mctx, n, &a);
        }
    }

    // Higher-order PATTERN unification: `?M x̄ =?= t` ⇒ `?M := λx̄. t`.
    if let Some(result) = try_pattern(mctx, lctx, &a, &b) {
        return result;
    }
    if let Some(result) = try_pattern(mctx, lctx, &b, &a) {
        return result;
    }

    // η-unification: `λx:A. body =?= t` (with `t` not a λ) ⇒ `body =?= t x` under the
    // binder, and symmetrically — so a function and its η-expansion unify.
    match (&a, &b) {
        (Term::Lambda { param, param_type, body }, other)
        | (other, Term::Lambda { param, param_type, body })
            if !matches!(other, Term::Lambda { .. }) =>
        {
            let applied = Term::App(Box::new(other.clone()), Box::new(Term::Var(param.clone())));
            let mut lctx2 = lctx.to_vec();
            lctx2.push((param.clone(), (**param_type).clone()));
            return unify_in(ctx, mctx, &lctx2, body, &applied);
        }
        _ => {}
    }

    match (&a, &b) {
        (Term::Sort(u), Term::Sort(v)) => u.equiv(v),
        (Term::Global(x), Term::Global(y)) => x == y,
        (Term::Var(x), Term::Var(y)) => x == y,
        (Term::Lit(x), Term::Lit(y)) => x == y,
        (Term::Hole, Term::Hole) => true,
        (Term::App(f1, a1), Term::App(f2, a2)) => {
            unify_in(ctx, mctx, lctx, f1, f2) && unify_in(ctx, mctx, lctx, a1, a2)
        }
        (
            Term::Pi { param: p1, param_type: t1, body_type: b1 },
            Term::Pi { param: p2, param_type: t2, body_type: b2 },
        ) => unify_binder(ctx, mctx, lctx, t1, b1, p1, t2, b2, p2),
        (
            Term::Lambda { param: p1, param_type: t1, body: b1 },
            Term::Lambda { param: p2, param_type: t2, body: b2 },
        ) => unify_binder(ctx, mctx, lctx, t1, b1, p1, t2, b2, p2),
        (
            Term::Const { name: n1, levels: l1 },
            Term::Const { name: n2, levels: l2 },
        ) => n1 == n2 && l1.len() == l2.len() && l1.iter().zip(l2.iter()).all(|(x, y)| x.equiv(y)),
        _ => false,
    }
}

/// Try to solve a Miller pattern `a = ?M x̄ =?= b`. Returns `None` if `a` is not a pattern
/// (the caller falls through to structural unification), `Some(true)` if solved, and
/// `Some(false)` if it IS a pattern but unsolvable (occurs-check / an out-of-scope
/// variable on the right). The arguments `x̄` must be DISTINCT bound variables of `lctx`,
/// and `b` may mention only those variables (plus globals/metavariables).
fn try_pattern(
    mctx: &mut MetaCtx,
    lctx: &[(String, Term)],
    a: &Term,
    b: &Term,
) -> Option<bool> {
    let (head, args) = spine(a);
    if args.is_empty() {
        return None;
    }
    let meta = match &head {
        Term::Var(m) if is_meta(m) && mctx.solution(m).is_none() => m.clone(),
        _ => return None,
    };
    // Every argument must be a distinct bound variable of the local context.
    let mut var_names: Vec<String> = Vec::new();
    for arg in &args {
        match arg {
            Term::Var(v) if !is_meta(v) && lctx.iter().any(|(n, _)| n == v) => {
                if var_names.iter().any(|u| u == v) {
                    return None; // a repeated argument — not a pattern
                }
                var_names.push(v.clone());
            }
            _ => return None, // a non-variable argument — not a pattern
        }
    }
    // It IS a pattern; now decide whether it is solvable.
    if occurs(&meta, b) {
        return Some(false); // `?M` occurs in `t` — cyclic
    }
    if !pattern_rhs_in_scope(b, &var_names, &mut Vec::new()) {
        return Some(false); // `t` mentions a variable we cannot abstract
    }
    // Solve `?M := λx̄. t`, taking each binder's type from the local context.
    let mut sol = b.clone();
    for name in var_names.iter().rev() {
        let ty = lctx
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t.clone())
            .unwrap_or(Term::Hole);
        sol = Term::Lambda { param: name.clone(), param_type: Box::new(ty), body: Box::new(sol) };
    }
    mctx.solve(&meta, sol);
    Some(true)
}

/// Whether every FREE variable of `t` (tracking binders in `bound`) is either a
/// metavariable, bound inside `t`, or one of the `allowed` pattern arguments — the
/// condition for `λallowed. t` to be well-scoped.
fn pattern_rhs_in_scope(t: &Term, allowed: &[String], bound: &mut Vec<String>) -> bool {
    match t {
        Term::Var(v) => is_meta(v) || bound.iter().any(|b| b == v) || allowed.iter().any(|a| a == v),
        Term::Sort(_) | Term::Global(_) | Term::Lit(_) | Term::Hole | Term::Const { .. } => true,
        Term::App(f, a) => {
            pattern_rhs_in_scope(f, allowed, bound) && pattern_rhs_in_scope(a, allowed, bound)
        }
        Term::Pi { param, param_type, body_type } => {
            if !pattern_rhs_in_scope(param_type, allowed, bound) {
                return false;
            }
            bound.push(param.clone());
            let ok = pattern_rhs_in_scope(body_type, allowed, bound);
            bound.pop();
            ok
        }
        Term::Lambda { param, param_type, body } => {
            if !pattern_rhs_in_scope(param_type, allowed, bound) {
                return false;
            }
            bound.push(param.clone());
            let ok = pattern_rhs_in_scope(body, allowed, bound);
            bound.pop();
            ok
        }
        Term::Fix { name, body } => {
            bound.push(name.clone());
            let ok = pattern_rhs_in_scope(body, allowed, bound);
            bound.pop();
            ok
        }
        Term::MutualFix { defs, .. } => {
            for (n, _) in defs {
                bound.push(n.clone());
            }
            let ok = defs.iter().all(|(_, b)| pattern_rhs_in_scope(b, allowed, bound));
            for _ in defs {
                bound.pop();
            }
            ok
        }
        Term::Match { discriminant, motive, cases } => {
            pattern_rhs_in_scope(discriminant, allowed, bound)
                && pattern_rhs_in_scope(motive, allowed, bound)
                && cases.iter().all(|c| pattern_rhs_in_scope(c, allowed, bound))
        }
        Term::Let { name, ty, value, body } => {
            if !pattern_rhs_in_scope(ty, allowed, bound)
                || !pattern_rhs_in_scope(value, allowed, bound)
            {
                return false;
            }
            bound.push(name.clone());
            let ok = pattern_rhs_in_scope(body, allowed, bound);
            bound.pop();
            ok
        }
    }
}

/// Decompose an application spine `head a0 a1 …` into its head and arguments (in order).
fn spine(t: &Term) -> (Term, Vec<Term>) {
    let mut args = Vec::new();
    let mut cur = t;
    while let Term::App(f, a) = cur {
        args.push((**a).clone());
        cur = f;
    }
    args.reverse();
    (cur.clone(), args)
}

/// Unify two binders (`Π`/`λ`): their domains, then their bodies α-renamed to a common
/// binder name — under a local context EXTENDED with that binder, so pattern unification
/// can fire on metavariables applied to it.
#[allow(clippy::too_many_arguments)]
fn unify_binder(
    ctx: &Context,
    mctx: &mut MetaCtx,
    lctx: &[(String, Term)],
    dom1: &Term,
    body1: &Term,
    p1: &str,
    dom2: &Term,
    body2: &Term,
    p2: &str,
) -> bool {
    if !unify_in(ctx, mctx, lctx, dom1, dom2) {
        return false;
    }
    let body2 = if p1 == p2 {
        body2.clone()
    } else {
        substitute(body2, p2, &Term::Var(p1.to_string()))
    };
    let mut ext = lctx.to_vec();
    ext.push((p1.to_string(), dom1.clone()));
    unify_in(ctx, mctx, &ext, body1, &body2)
}

/// Solve metavariable `m := t` (or, if already solved, unify the existing solution with
/// `t`). The occurs-check rejects a cyclic solution.
fn assign(ctx: &Context, mctx: &mut MetaCtx, m: &str, t: &Term) -> bool {
    if let Some(sol) = mctx.solutions.get(m).cloned() {
        return unify(ctx, mctx, &sol, t);
    }
    let t = instantiate(t, mctx);
    // `?m := ?m` is trivially satisfied; a deeper occurrence is a cycle.
    if matches!(&t, Term::Var(n) if n == m) {
        return true;
    }
    if occurs(m, &t) {
        return false;
    }
    mctx.solutions.insert(m.to_string(), t);
    true
}

/// Elaborate `term` (which may contain `Hole`s) against an optional `expected` type,
/// solving the holes by unification. Returns the elaborated term and its type (both still
/// containing metavariables until `instantiate`d).
pub fn elaborate(
    ctx: &Context,
    mctx: &mut MetaCtx,
    term: &Term,
    expected: Option<&Term>,
) -> KernelResult<(Term, Term)> {
    elaborate_in(ctx, mctx, &[], term, expected)
}

/// Like [`elaborate`] but under a LOCAL CONTEXT `lctx` of bound variables. `ctx` is the
/// kernel context already EXTENDED with those same variables (so `infer_type` sees them
/// at the leaves), while `lctx` is the parallel `(name, type)` list the unifier uses for
/// higher-order PATTERN unification. Descending under a `λ` extends both — which is how a
/// metavariable applied to a bound variable (a motive `?P n`) gets solved in a body.
pub fn elaborate_in(
    ctx: &Context,
    mctx: &mut MetaCtx,
    lctx: &[(String, Term)],
    term: &Term,
    expected: Option<&Term>,
) -> KernelResult<(Term, Term)> {
    // Surface sugar `⟨…⟩` / `receiver.field` is dispatched FIRST, so a nested occurrence in a
    // field or receiver resolves here too — with the expected type flowing in from the
    // enclosing constructor domain (`⟨⟨a, b⟩, c⟩`) or projection.
    if let Some(sugar) = as_surface_sugar(term) {
        return elaborate_sugar(ctx, mctx, sugar, expected);
    }
    match term {
        Term::Hole => {
            let m = mctx.fresh();
            let ty = expected.cloned().unwrap_or_else(|| mctx.fresh());
            Ok((m, ty))
        }
        Term::App(f, a) => {
            let (f_elab, f_ty) = elaborate_in(ctx, mctx, lctx, f, None)?;
            let f_ty = resolve(ctx, mctx, &f_ty);
            match f_ty {
                Term::Pi { param, param_type, body_type } => {
                    let (a_elab, a_ty) = elaborate_in(ctx, mctx, lctx, a, Some(&param_type))?;
                    if !unify_in(ctx, mctx, lctx, &a_ty, &param_type) {
                        return Err(KernelError::TypeMismatch {
                            expected: format!("{}", instantiate(&param_type, mctx)),
                            found: format!("{}", instantiate(&a_ty, mctx)),
                        });
                    }
                    let result_ty = substitute(&body_type, &param, &a_elab);
                    // Reconcile the result with the expected type — this is where a motive
                    // `?P n` is solved against the application's actual type `Vec n`.
                    if let Some(exp) = expected {
                        unify_in(ctx, mctx, lctx, &result_ty, exp);
                    }
                    Ok((Term::App(Box::new(f_elab), Box::new(a_elab)), result_ty))
                }
                other => Err(KernelError::NotAFunction(format!("{}", other))),
            }
        }
        Term::Lambda { param, param_type, body } => {
            // Descend under the binder, extending BOTH the kernel context (for leaf type
            // inference) and the unification context (for pattern unification). The body's
            // expected type is the expected `Π`'s codomain, α-renamed to this binder.
            let body_expected = match expected.map(|e| resolve(ctx, mctx, e)) {
                Some(Term::Pi { param: ep, body_type: ecod, .. }) => Some(if ep == *param {
                    (*ecod).clone()
                } else {
                    substitute(&ecod, &ep, &Term::Var(param.clone()))
                }),
                _ => None,
            };
            let ext_ctx = ctx.extend(param, (**param_type).clone());
            let mut ext_lctx = lctx.to_vec();
            ext_lctx.push((param.clone(), (**param_type).clone()));
            let (body_elab, body_ty) =
                elaborate_in(&ext_ctx, mctx, &ext_lctx, body, body_expected.as_ref())?;
            Ok((
                Term::Lambda {
                    param: param.clone(),
                    param_type: param_type.clone(),
                    body: Box::new(body_elab),
                },
                Term::Pi {
                    param: param.clone(),
                    param_type: param_type.clone(),
                    body_type: Box::new(body_ty),
                },
            ))
        }
        _ => {
            // A leaf (hole-free): defer to the kernel for its type (in the extended
            // context, so bound variables resolve), then reconcile with the expected type
            // via unification (which may solve metavariables on either side, including
            // higher-order patterns over the local context).
            let ty = infer_type(ctx, term)?;
            if let Some(exp) = expected {
                unify_in(ctx, mctx, lctx, &ty, exp);
            }
            Ok((term.clone(), ty))
        }
    }
}

/// Fill in inferred motives for `match` expressions written WITHOUT a `return` clause
/// (the `Hole` motive the parser leaves). The motive is a CONSTANT `λ_:I. T` — covering
/// non-dependent matches — where `T` is the EXPECTED type (a definition's declared
/// result type, propagated through binders) or, lacking one, the type of a nullary first
/// branch. The pass threads the kernel context through binders so the discriminant's type
/// resolves; a `match` it cannot give a motive (a dependent case, no expected type) is
/// reported so the user can add an explicit `return`.
pub fn fill_match_motives(
    ctx: &Context,
    term: &Term,
    expected: Option<&Term>,
) -> KernelResult<Term> {
    match term {
        Term::Match { discriminant, motive, cases } => {
            let disc = fill_match_motives(ctx, discriminant, None)?;
            let cases = cases
                .iter()
                .map(|c| fill_match_motives(ctx, c, None))
                .collect::<KernelResult<Vec<_>>>()?;
            let motive = if matches!(motive.as_ref(), Term::Hole) {
                infer_match_motive(ctx, &disc, &cases, expected)?
            } else {
                fill_match_motives(ctx, motive, None)?
            };
            Ok(Term::Match {
                discriminant: Box::new(disc),
                motive: Box::new(motive),
                cases,
            })
        }
        Term::Lambda { param, param_type, body } => {
            let ext = ctx.extend(param, (**param_type).clone());
            // The body's expected type is the codomain of the expected `Π`, α-renamed.
            let body_expected = match expected.map(|e| normalize(ctx, e)) {
                Some(Term::Pi { param: ep, body_type, .. }) => Some(if ep == *param {
                    *body_type
                } else {
                    substitute(&body_type, &ep, &Term::Var(param.clone()))
                }),
                _ => None,
            };
            Ok(Term::Lambda {
                param: param.clone(),
                param_type: param_type.clone(),
                body: Box::new(fill_match_motives(&ext, body, body_expected.as_ref())?),
            })
        }
        Term::App(f, a) => Ok(Term::App(
            Box::new(fill_match_motives(ctx, f, None)?),
            Box::new(fill_match_motives(ctx, a, None)?),
        )),
        Term::Pi { param, param_type, body_type } => {
            let ext = ctx.extend(param, (**param_type).clone());
            Ok(Term::Pi {
                param: param.clone(),
                param_type: Box::new(fill_match_motives(ctx, param_type, None)?),
                body_type: Box::new(fill_match_motives(&ext, body_type, None)?),
            })
        }
        Term::Fix { name, body } => Ok(Term::Fix {
            name: name.clone(),
            body: Box::new(fill_match_motives(ctx, body, None)?),
        }),
        _ => Ok(term.clone()),
    }
}

/// Build the motive `λx:I. T[disc := x]` for a `match` written without a `return` clause,
/// by ABSTRACTING the discriminant out of the expected type — the Miller-pattern solution
/// of `?P disc =?= T`. When `T` mentions the discriminant the motive is DEPENDENT (so a
/// match whose result type varies per branch, like an eliminator `Π(n). P n`, elaborates);
/// when it does not, this collapses to the constant motive `λ_:I. T`. A bare variable
/// discriminant just captures the free variable; any other discriminant has its
/// occurrences replaced.
fn infer_match_motive(
    ctx: &Context,
    disc: &Term,
    cases: &[Term],
    expected: Option<&Term>,
) -> KernelResult<Term> {
    let disc_ty = normalize(ctx, &infer_type(ctx, disc)?);
    let result_ty = match expected {
        Some(t) => t.clone(),
        None => {
            // No expected type: infer it from a nullary first branch (a bare term, not a
            // case lambda whose binder types are placeholders we cannot infer through).
            match cases.first() {
                Some(c) if !matches!(c, Term::Lambda { .. }) => infer_type(ctx, c)?,
                _ => {
                    return Err(KernelError::CertificationError(
                        "cannot infer the motive of this `match`; add a `return` clause or a \
                         type annotation"
                            .to_string(),
                    ))
                }
            }
        }
    };
    let (param, body) = match disc {
        // A variable discriminant: bind its name so its free occurrences in the result
        // type are captured (`Π(n). P n` ⇒ motive `λn:I. P n`).
        Term::Var(v) => (v.clone(), result_ty),
        // Otherwise replace occurrences of the (compound) discriminant by a fresh binder.
        other => {
            let p = "__motive".to_string();
            (p.clone(), replace_subterm(&result_ty, other, &Term::Var(p)))
        }
    };
    Ok(Term::Lambda { param, param_type: Box::new(disc_ty), body: Box::new(body) })
}

/// Replace every subterm structurally equal to `target` by `repl`.
fn replace_subterm(t: &Term, target: &Term, repl: &Term) -> Term {
    if t == target {
        return repl.clone();
    }
    match t {
        Term::Var(_) | Term::Global(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole
        | Term::Const { .. } => t.clone(),
        Term::Pi { param, param_type, body_type } => Term::Pi {
            param: param.clone(),
            param_type: Box::new(replace_subterm(param_type, target, repl)),
            body_type: Box::new(replace_subterm(body_type, target, repl)),
        },
        Term::Lambda { param, param_type, body } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(replace_subterm(param_type, target, repl)),
            body: Box::new(replace_subterm(body, target, repl)),
        },
        Term::App(f, a) => Term::App(
            Box::new(replace_subterm(f, target, repl)),
            Box::new(replace_subterm(a, target, repl)),
        ),
        Term::Match { discriminant, motive, cases } => Term::Match {
            discriminant: Box::new(replace_subterm(discriminant, target, repl)),
            motive: Box::new(replace_subterm(motive, target, repl)),
            cases: cases.iter().map(|c| replace_subterm(c, target, repl)).collect(),
        },
        Term::Fix { name, body } => {
            Term::Fix { name: name.clone(), body: Box::new(replace_subterm(body, target, repl)) }
        }
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs.iter().map(|(n, b)| (n.clone(), replace_subterm(b, target, repl))).collect(),
            index: *index,
        },
        Term::Let { name, ty, value, body } => Term::Let {
            name: name.clone(),
            ty: Box::new(replace_subterm(ty, target, repl)),
            value: Box::new(replace_subterm(value, target, repl)),
            body: Box::new(replace_subterm(body, target, repl)),
        },
    }
}

/// Auto-bind free type variables as leading implicit parameters. A definition written
/// `id : A -> A := fun a : A => a` mentions `A` as a FREE, unregistered, single-uppercase
/// global — the type-variable convention. This pass generalizes each such variable: it
/// prepends `Π(A:Type)` to the type and `λ(A:Type)` to the body (converting `Global(A)`
/// to the bound `Var(A)`), and returns the new implicit count, so `A` becomes an inferred
/// implicit argument. Definitions that already bind their parameters reference them as
/// `Var`s, not free `Global`s, so they are untouched — this only rescues what was
/// previously an unbound-variable error.
pub fn auto_bind_implicits(
    ctx: &Context,
    ty: &Term,
    body: &Term,
    existing_implicit: usize,
) -> (Term, Term, usize) {
    let mut candidates: Vec<String> = Vec::new();
    collect_autobind(ctx, ty, &mut candidates);
    collect_autobind(ctx, body, &mut candidates);
    if candidates.is_empty() {
        return (ty.clone(), body.clone(), existing_implicit);
    }

    let mut new_ty = ty.clone();
    let mut new_body = body.clone();
    for name in &candidates {
        new_ty = global_to_var(&new_ty, name);
        new_body = global_to_var(&new_body, name);
    }
    // First candidate becomes the OUTERMOST binder.
    for name in candidates.iter().rev() {
        new_ty = Term::Pi {
            param: name.clone(),
            param_type: Box::new(Term::Sort(crate::term::Universe::Type(0))),
            body_type: Box::new(new_ty),
        };
        new_body = Term::Lambda {
            param: name.clone(),
            param_type: Box::new(Term::Sort(crate::term::Universe::Type(0))),
            body: Box::new(new_body),
        };
    }
    (new_ty, new_body, existing_implicit + candidates.len())
}

/// A free auto-bind candidate: a single uppercase letter that is not a registered global.
fn is_autobind_name(n: &str) -> bool {
    n.len() == 1 && n.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

/// Collect free auto-bind candidates from `term` in first-appearance order (deduped).
fn collect_autobind(ctx: &Context, term: &Term, acc: &mut Vec<String>) {
    match term {
        Term::Global(n) => {
            if is_autobind_name(n) && ctx.get_global(n).is_none() && !acc.contains(n) {
                acc.push(n.clone());
            }
        }
        Term::Pi { param_type, body_type, .. } => {
            collect_autobind(ctx, param_type, acc);
            collect_autobind(ctx, body_type, acc);
        }
        Term::Lambda { param_type, body, .. } => {
            collect_autobind(ctx, param_type, acc);
            collect_autobind(ctx, body, acc);
        }
        Term::App(f, a) => {
            collect_autobind(ctx, f, acc);
            collect_autobind(ctx, a, acc);
        }
        Term::Match { discriminant, motive, cases } => {
            collect_autobind(ctx, discriminant, acc);
            collect_autobind(ctx, motive, acc);
            for c in cases {
                collect_autobind(ctx, c, acc);
            }
        }
        Term::Fix { body, .. } => collect_autobind(ctx, body, acc),
        _ => {}
    }
}

/// Replace every `Global(name)` by `Var(name)` (turning a free type-variable reference
/// into a reference to the binder this pass prepends).
/// Recursive-definition sugar: if `body` refers to the definition's own `name` as a free
/// `Global`, bind that self-reference with a `fix` so the definition can call itself —
/// `Definition f : T := … f … .` becomes `f := fix f. …`. The kernel's termination guard
/// (run by `infer_type`) then certifies the recursion decreases structurally; a body with
/// no self-reference (or one that already wrote an explicit `fix`, whose self-references are
/// already bound `Var`s) is returned unchanged. A self-reference SHADOWS any same-named
/// global, so a recursive `Definition add` over `Nat` overrides a built-in `add`.
pub fn bind_self_recursion(name: &str, body: &Term) -> Term {
    if references_global(body, name) {
        Term::Fix { name: name.to_string(), body: Box::new(global_to_var(body, name)) }
    } else {
        body.clone()
    }
}

/// Whether `term` mentions `Global(name)` anywhere — the self-reference test for
/// [`bind_self_recursion`]. Bound occurrences (already `Var`) do not count.
fn references_global(term: &Term, name: &str) -> bool {
    match term {
        Term::Global(n) => n == name,
        Term::Var(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole | Term::Const { .. } => false,
        Term::Pi { param_type, body_type, .. } => {
            references_global(param_type, name) || references_global(body_type, name)
        }
        Term::Lambda { param_type, body, .. } => {
            references_global(param_type, name) || references_global(body, name)
        }
        Term::App(f, a) => references_global(f, name) || references_global(a, name),
        Term::Match { discriminant, motive, cases } => {
            references_global(discriminant, name)
                || references_global(motive, name)
                || cases.iter().any(|c| references_global(c, name))
        }
        Term::Fix { body, .. } => references_global(body, name),
        Term::MutualFix { defs, .. } => defs.iter().any(|(_, b)| references_global(b, name)),
        Term::Let { ty, value, body, .. } => {
            references_global(ty, name)
                || references_global(value, name)
                || references_global(body, name)
        }
    }
}

fn global_to_var(term: &Term, name: &str) -> Term {
    match term {
        Term::Global(n) if n == name => Term::Var(n.clone()),
        Term::Global(_) | Term::Var(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole
        | Term::Const { .. } => term.clone(),
        Term::Pi { param, param_type, body_type } => Term::Pi {
            param: param.clone(),
            param_type: Box::new(global_to_var(param_type, name)),
            body_type: Box::new(global_to_var(body_type, name)),
        },
        Term::Lambda { param, param_type, body } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(global_to_var(param_type, name)),
            body: Box::new(global_to_var(body, name)),
        },
        Term::App(f, a) => {
            Term::App(Box::new(global_to_var(f, name)), Box::new(global_to_var(a, name)))
        }
        Term::Match { discriminant, motive, cases } => Term::Match {
            discriminant: Box::new(global_to_var(discriminant, name)),
            motive: Box::new(global_to_var(motive, name)),
            cases: cases.iter().map(|c| global_to_var(c, name)).collect(),
        },
        Term::Fix { name: fname, body } => {
            Term::Fix { name: fname.clone(), body: Box::new(global_to_var(body, name)) }
        }
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs.iter().map(|(fname, b)| (fname.clone(), global_to_var(b, name))).collect(),
            index: *index,
        },
        Term::Let { name: lname, ty, value, body } => Term::Let {
            name: lname.clone(),
            ty: Box::new(global_to_var(ty, name)),
            value: Box::new(global_to_var(value, name)),
            body: Box::new(global_to_var(body, name)),
        },
    }
}

/// Elaborate a whole surface term: walk it, and at every application of a global with
/// declared implicit parameters (`Context::implicit_args`), insert and infer those
/// arguments — so `id 0` becomes `id Int 0`. Terms with no implicits are returned
/// unchanged. The result is fully explicit and metavariable-free; the kernel certifies
/// it as usual. This is the seam that wires the elaborator into the REPL.
pub fn surface_elaborate(ctx: &Context, term: &Term) -> KernelResult<Term> {
    surface_elaborate_against(ctx, term, None)
}

/// Like [`surface_elaborate`] but with an EXPECTED type (e.g. a definition's declared
/// type). The expected type is propagated to the top-level application/global so an
/// implicit with no value argument — `nil : {A} → List A` used where a `List Int` is
/// wanted — is inferred from context.
pub fn surface_elaborate_against(
    ctx: &Context,
    term: &Term,
    expected: Option<&Term>,
) -> KernelResult<Term> {
    let mut mctx = MetaCtx::new();
    let elaborated = elab_surface(ctx, &mut mctx, term, expected)?;
    Ok(instantiate(&elaborated, &mctx))
}

fn elab_surface(
    ctx: &Context,
    mctx: &mut MetaCtx,
    term: &Term,
    expected: Option<&Term>,
) -> KernelResult<Term> {
    // A top-level surface-sugar term (`⟨…⟩` / `receiver.field`) is resolved through the typed
    // core so its expected type propagates; its subterms are elaborated there.
    if let Some(sugar) = as_surface_sugar(term) {
        return elaborate_sugar(ctx, mctx, sugar, expected).map(|(t, _)| t);
    }
    match term {
        Term::App(..) => {
            // Decompose the application spine `head a0 a1 …`.
            let mut args: Vec<&Term> = Vec::new();
            let mut cur = term;
            while let Term::App(f, a) = cur {
                args.push(a);
                cur = f;
            }
            args.reverse();
            let head = elab_surface(ctx, mctx, cur, None)?;
            let args: Vec<Term> = args
                .iter()
                .map(|a| elab_surface(ctx, mctx, a, None))
                .collect::<KernelResult<_>>()?;

            if let Term::Global(name) = &head {
                if let Some(head_ty) = ctx.get_global(name).cloned() {
                    // Route EVERY global-headed application through the typed elaboration
                    // path — even one with no implicits — so argument type-checking and
                    // COERCION insertion apply uniformly.
                    //
                    // The parameter kinds come from the recorded PER-BINDER info (E2) when it
                    // exists and matches the number of explicit arguments — so implicit and
                    // instance parameters may interleave with explicit ones. Otherwise the
                    // legacy model applies: `implicit_args` leading implicits, the rest
                    // explicit.
                    let kinds = match ctx.binder_kinds(name) {
                        Some(bk)
                            if bk.iter().filter(|k| **k == ParamKind::Explicit).count()
                                == args.len() =>
                        {
                            bk.to_vec()
                        }
                        _ => {
                            let k = ctx.implicit_args(name);
                            let mut kinds = vec![ParamKind::Implicit; k];
                            kinds.extend(std::iter::repeat(ParamKind::Explicit).take(args.len()));
                            kinds
                        }
                    };
                    if let Ok((t, _)) =
                        elaborate_app_against(ctx, mctx, &head, &head_ty, &kinds, &args, expected)
                    {
                        return Ok(t);
                    }
                    // Fall through to a plain application if typed elaboration did not apply
                    // (e.g. the head is not a function of the expected arity) — preserving
                    // the previous permissive behaviour for non-standard shapes.
                }
            }
            Ok(args.into_iter().fold(head, |f, a| Term::App(Box::new(f), Box::new(a))))
        }
        // A bare implicit global (no value arguments) is elaborated only when an expected
        // type is available to pin its implicits — otherwise it stays the polymorphic
        // function value, not an unsolvable metavariable application.
        Term::Global(name) if expected.is_some() && ctx.implicit_args(name) > 0 => {
            let k = ctx.implicit_args(name);
            if let Some(head_ty) = ctx.get_global(name).cloned() {
                let kinds = vec![ParamKind::Implicit; k];
                let (t, _) =
                    elaborate_app_against(ctx, mctx, term, &head_ty, &kinds, &[], expected)?;
                Ok(t)
            } else {
                Ok(term.clone())
            }
        }
        Term::Lambda { param, param_type, body } => Ok(Term::Lambda {
            param: param.clone(),
            param_type: Box::new(elab_surface(ctx, mctx, param_type, None)?),
            body: Box::new(elab_surface(ctx, mctx, body, None)?),
        }),
        Term::Pi { param, param_type, body_type } => Ok(Term::Pi {
            param: param.clone(),
            param_type: Box::new(elab_surface(ctx, mctx, param_type, None)?),
            body_type: Box::new(elab_surface(ctx, mctx, body_type, None)?),
        }),
        Term::Fix { name, body } => Ok(Term::Fix {
            name: name.clone(),
            body: Box::new(elab_surface(ctx, mctx, body, None)?),
        }),
        Term::Match { discriminant, motive, cases } => Ok(Term::Match {
            discriminant: Box::new(elab_surface(ctx, mctx, discriminant, None)?),
            motive: Box::new(elab_surface(ctx, mctx, motive, None)?),
            cases: cases
                .iter()
                .map(|c| elab_surface(ctx, mctx, c, None))
                .collect::<KernelResult<_>>()?,
        }),
        _ => Ok(term.clone()),
    }
}

/// How a function parameter is supplied during elaboration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    /// The caller provides the argument.
    Explicit,
    /// Inferred by unification (a fresh metavariable is inserted).
    Implicit,
    /// A typeclass instance, resolved from the [`Context`]'s instance database — but only
    /// AFTER the explicit arguments have been processed, so any metavariable in the class
    /// type (e.g. the `A` in `Inhabited A`) is solved first.
    Instance,
}

/// Elaborate an application of `head` (of type `head_ty`) where leading parameters may be
/// implicit or instance-implicit (`kinds[i]`). A fresh metavariable is inserted for each
/// implicit position; instance positions get a placeholder metavariable whose resolution
/// is DEFERRED until all explicit arguments (which may pin the class's type variables)
/// have been unified. Returns the fully explicit, metavariable-instantiated term and type.
pub fn elaborate_app(
    ctx: &Context,
    mctx: &mut MetaCtx,
    head: &Term,
    head_ty: &Term,
    kinds: &[ParamKind],
    explicit_args: &[Term],
) -> KernelResult<(Term, Term)> {
    elaborate_app_against(ctx, mctx, head, head_ty, kinds, explicit_args, None)
}

/// Like [`elaborate_app`] but with an EXPECTED result type. After the explicit arguments
/// are processed, the result type is unified with `expected` — so an implicit that the
/// arguments alone do not pin (e.g. the `A` of `nil : {A} → List A`, which has no value
/// argument) is solved from the surrounding context. The expected-type pass runs BEFORE
/// instance resolution, so a class's type variable can be fixed by the context too.
#[allow(clippy::too_many_arguments)]
pub fn elaborate_app_against(
    ctx: &Context,
    mctx: &mut MetaCtx,
    head: &Term,
    head_ty: &Term,
    kinds: &[ParamKind],
    explicit_args: &[Term],
    expected: Option<&Term>,
) -> KernelResult<(Term, Term)> {
    let mut cur_term = head.clone();
    let mut cur_ty = head_ty.clone();
    let mut next_arg = 0usize;
    // (placeholder metavariable name, the class type to resolve it against).
    let mut obligations: Vec<(String, Term)> = Vec::new();

    for kind in kinds {
        let (param, dom, body) = match resolve(ctx, mctx, &cur_ty) {
            Term::Pi { param, param_type, body_type } => (param, *param_type, *body_type),
            other => {
                return Err(KernelError::NotAFunction(format!(
                    "expected a Π to apply an argument, got {}",
                    other
                )))
            }
        };

        let arg = match kind {
            ParamKind::Implicit => mctx.fresh(),
            ParamKind::Instance => {
                let m = mctx.fresh();
                if let Term::Var(name) = &m {
                    obligations.push((name.clone(), dom.clone()));
                }
                m
            }
            ParamKind::Explicit => {
                let provided = explicit_args.get(next_arg).ok_or_else(|| {
                    KernelError::CertificationError("too few explicit arguments".to_string())
                })?;
                next_arg += 1;
                let (a_elab, a_ty) = elaborate(ctx, mctx, provided, Some(&dom))?;
                if unify(ctx, mctx, &a_ty, &dom) {
                    a_elab
                } else {
                    // The argument type does not match — try to bridge it with a registered
                    // COERCION (`↑`). On success wrap the argument in the coercion function;
                    // otherwise it is a genuine type mismatch.
                    let a_ty_i = instantiate(&a_ty, mctx);
                    let dom_i = instantiate(&dom, mctx);
                    match resolve_coercion(ctx, mctx, &a_ty_i, &dom_i) {
                        Some(coe) => Term::App(Box::new(coe), Box::new(a_elab)),
                        None => {
                            return Err(KernelError::TypeMismatch {
                                expected: format!("{}", dom_i),
                                found: format!("{}", a_ty_i),
                            })
                        }
                    }
                }
            }
        };

        cur_term = Term::App(Box::new(cur_term), Box::new(arg.clone()));
        cur_ty = substitute(&body, &param, &arg);
    }

    if next_arg != explicit_args.len() {
        return Err(KernelError::CertificationError(format!(
            "too many explicit arguments: {} provided, {} consumed",
            explicit_args.len(),
            next_arg
        )));
    }

    // Expected-type propagation: unify the result type with the context's expected type,
    // solving any implicit not already pinned by the explicit arguments.
    if let Some(exp) = expected {
        if !unify(ctx, mctx, &cur_ty, exp) {
            return Err(KernelError::TypeMismatch {
                expected: format!("{}", instantiate(exp, mctx)),
                found: format!("{}", instantiate(&cur_ty, mctx)),
            });
        }
    }

    // Resolve the deferred instance obligations now that the metavariables their class
    // types mention have been solved by the explicit arguments (and the expected type).
    for (meta_name, class_ty) in &obligations {
        let required = instantiate(class_ty, mctx);
        match resolve_instance(ctx, mctx, &required) {
            Some(inst) => {
                // Bind directly (not via normalizing unification) so the instance stays
                // structured rather than being δ-unfolded into its body.
                mctx.solve(meta_name, inst);
            }
            None => {
                return Err(KernelError::CertificationError(format!(
                    "no typeclass instance found for {}",
                    required
                )))
            }
        }
    }

    Ok((instantiate(&cur_term, mctx), instantiate(&cur_ty, mctx)))
}

/// Depth bound on recursive instance resolution — a backstop against a pathological
/// instance set (`Inhabited A` from `Inhabited A`) looping forever.
const MAX_INSTANCE_DEPTH: usize = 64;

/// The head `Global` of an application spine (`Inhabited (List A)` → `Inhabited`).
fn head_global(t: &Term) -> Option<&str> {
    let mut cur = t;
    while let Term::App(f, _) = cur {
        cur = f;
    }
    match cur {
        Term::Global(n) => Some(n),
        _ => None,
    }
}

/// The set of typeclass "heads" — the head `Global` of every registered instance's
/// CONCLUSION (`Inhabited (List A)` and `Inhabited Nat` both contribute `Inhabited`).
/// A parameter whose type has one of these heads is an instance PREMISE, to be resolved
/// recursively, rather than a type parameter solved by unifying the conclusion.
fn class_heads(ctx: &Context) -> std::collections::HashSet<String> {
    ctx.instances()
        .iter()
        .filter_map(|(ty, _)| {
            let mut cur = ty;
            while let Term::Pi { body_type, .. } = cur {
                cur = body_type;
            }
            head_global(cur).map(|s| s.to_string())
        })
        .collect()
}

/// Search the [`Context`]'s instance database for an instance proving `required`,
/// returning the (metavariable-instantiated) instance term. Handles POLYMORPHIC /
/// RECURSIVE instances (`instance {A} [Inhabited A] : Inhabited (List A)`): the instance's
/// parameter telescope is freshened to metavariables, its conclusion is unified against
/// `required` (solving the type parameters), and each PREMISE parameter is then resolved
/// RECURSIVELY. The first instance that fully resolves wins; failed trials never pollute
/// `mctx` (each runs on a clone, committed only on success).
pub fn resolve_instance(ctx: &Context, mctx: &mut MetaCtx, required: &Term) -> Option<Term> {
    resolve_instance_at(ctx, mctx, required, 0)
}

/// Elaborate an ANONYMOUS CONSTRUCTOR `⟨f₀, …, fₙ⟩` (E3) against an EXPECTED inductive/
/// structure type `H a…`. It applies `H`'s (unique) constructor to the type parameters
/// `a…`, read off the expected type, then the field values — so `⟨Zero, true⟩` expected at
/// `Prod Nat Bool` becomes `Prod_mk Nat Bool Zero true`. Each field is elaborated against
/// its declared type (so coercions/implicits fire), and the whole is kernel-certified.
pub fn elaborate_anon_ctor(
    ctx: &Context,
    mctx: &mut MetaCtx,
    expected: &Term,
    fields: &[Term],
) -> KernelResult<Term> {
    let exp = crate::normalize(ctx, &instantiate(expected, mctx));
    let (head, args) = spine(&exp);
    let hname = match &head {
        Term::Global(n) => n.clone(),
        _ => {
            return Err(KernelError::CertificationError(format!(
                "anonymous constructor: expected type {exp} is not an inductive"
            )))
        }
    };
    let ctors = ctx.get_constructors(&hname);
    let ctor = match ctors.as_slice() {
        [(c, _)] => c.to_string(),
        _ => {
            return Err(KernelError::CertificationError(format!(
                "anonymous constructor: `{hname}` does not have exactly one constructor"
            )))
        }
    };
    // Apply the constructor to the type parameters, then each field elaborated against its
    // declared domain.
    let mut applied = Term::Global(ctor);
    for a in &args {
        applied = Term::App(Box::new(applied), Box::new(a.clone()));
    }
    for fv in fields {
        let dom = match resolve(ctx, mctx, &crate::infer_type(ctx, &applied)?) {
            Term::Pi { param_type, .. } => Some(*param_type),
            _ => None,
        };
        let (fe, fty) = elaborate(ctx, mctx, fv, dom.as_ref())?;
        let arg = if let Some(d) = &dom {
            if unify(ctx, mctx, &fty, d) {
                fe
            } else {
                match resolve_coercion(ctx, mctx, &instantiate(&fty, mctx), &instantiate(d, mctx)) {
                    Some(coe) => Term::App(Box::new(coe), Box::new(fe)),
                    None => fe,
                }
            }
        } else {
            fe
        };
        applied = Term::App(Box::new(applied), Box::new(arg));
    }
    crate::infer_type(ctx, &applied)?;
    Ok(instantiate(&applied, mctx))
}

/// Elaborate DOT notation `receiver.field` (E4). The receiver's type head names an
/// inductive/structure `H`; the projection is `H_field` (K4's convention), applied to `H`'s
/// parameters — read off the receiver's type — and then the receiver itself. So
/// `p.fst` with `p : Prod A B` becomes `Prod_fst A B p`. Returns an error if no such
/// projection exists or the result does not type-check.
pub fn elaborate_dot(
    ctx: &Context,
    mctx: &mut MetaCtx,
    receiver: &Term,
    field: &str,
) -> KernelResult<Term> {
    let (r_elab, r_ty) = elaborate(ctx, mctx, receiver, None)?;
    let r_ty = crate::normalize(ctx, &instantiate(&r_ty, mctx));
    let (head, args) = spine(&r_ty);
    let hname = match &head {
        Term::Global(n) => n.clone(),
        _ => {
            return Err(KernelError::CertificationError(format!(
                "dot notation `.{field}`: the receiver's type {r_ty} is not headed by an inductive"
            )))
        }
    };
    let proj = format!("{hname}_{field}");
    if ctx.get_global(&proj).is_none() {
        return Err(KernelError::CertificationError(format!(
            "dot notation: no projection `{proj}` for field `{field}` of `{hname}`"
        )));
    }
    // `H_field params… receiver`.
    let mut applied = Term::Global(proj);
    for a in &args {
        applied = Term::App(Box::new(applied), Box::new(a.clone()));
    }
    applied = Term::App(Box::new(applied), Box::new(r_elab));
    // Certify it type-checks (the projection's arity/positions line up).
    crate::infer_type(ctx, &applied)?;
    Ok(applied)
}

/// Find a registered coercion carrying `from` to `to` (up to unification), returning the
/// coercion FUNCTION to wrap the argument in — Lean's `↑`. The elaborator calls this when
/// an argument's type does not match the expected parameter type; a match commits the
/// unification (a polymorphic coercion's type variables get solved).
pub fn resolve_coercion(
    ctx: &Context,
    mctx: &mut MetaCtx,
    from: &Term,
    to: &Term,
) -> Option<Term> {
    for (c_from, c_to, c_fn) in ctx.coercions() {
        let mut trial = mctx.clone();
        if unify(ctx, &mut trial, c_from, from) && unify(ctx, &mut trial, c_to, to) {
            *mctx = trial;
            return Some(instantiate(c_fn, mctx));
        }
    }
    None
}

fn resolve_instance_at(
    ctx: &Context,
    mctx: &mut MetaCtx,
    required: &Term,
    depth: usize,
) -> Option<Term> {
    if depth > MAX_INSTANCE_DEPTH {
        return None;
    }
    let heads = class_heads(ctx);
    for (inst_ty, inst_val) in ctx.instances() {
        let mut trial = mctx.clone();
        if let Some(result) =
            try_instance(ctx, &mut trial, inst_ty, inst_val, required, &heads, depth)
        {
            *mctx = trial;
            return Some(result);
        }
    }
    None
}

/// Attempt one instance against `required`: freshen its parameters to metavariables,
/// unify its conclusion with `required`, and recursively resolve every premise parameter.
#[allow(clippy::too_many_arguments)]
fn try_instance(
    ctx: &Context,
    mctx: &mut MetaCtx,
    inst_ty: &Term,
    inst_val: &Term,
    required: &Term,
    heads: &std::collections::HashSet<String>,
    depth: usize,
) -> Option<Term> {
    // Freshen: peel the parameter telescope, replacing each parameter by a fresh
    // metavariable and recording which are instance premises. `applied` accumulates the
    // instance value applied to those metavariables.
    let mut applied = inst_val.clone();
    let mut premises: Vec<(Term, Term)> = Vec::new(); // (metavariable, premise type)
    let mut cur = inst_ty.clone();
    loop {
        match cur {
            Term::Pi { param, param_type, body_type } => {
                let mv = mctx.fresh();
                if head_global(&param_type).is_some_and(|h| heads.contains(h)) {
                    premises.push((mv.clone(), (*param_type).clone()));
                }
                applied = Term::App(Box::new(applied), Box::new(mv.clone()));
                cur = substitute(&body_type, &param, &mv);
            }
            conclusion => {
                // The conclusion must match the goal (this solves the type parameters).
                if !unify(ctx, mctx, &conclusion, required) {
                    return None;
                }
                // Each premise is now (after that unification) a ground class goal; resolve
                // it recursively and bind its metavariable to the result.
                for (pm, pty) in &premises {
                    let sub_goal = instantiate(pty, mctx);
                    let resolved = resolve_instance_at(ctx, mctx, &sub_goal, depth + 1)?;
                    // Bind the premise's metavariable directly, keeping the (possibly
                    // nested) resolved instance structured.
                    match pm {
                        Term::Var(name) => mctx.solve(name, resolved),
                        _ => return None,
                    }
                }
                return Some(instantiate(&applied, mctx));
            }
        }
    }
}
