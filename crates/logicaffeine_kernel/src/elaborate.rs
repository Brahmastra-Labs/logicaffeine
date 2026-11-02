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
use crate::reduction::normalize_elab as normalize;
use crate::term::Term;
use crate::type_checker::substitute;
use crate::{infer_type, Context};

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
    holes: Vec<Hole>,
}

/// A hole (`_`) the elaborator met: its metavariable, the local context around it, and the type its
/// position asks for — everything a tactic needs to make it a goal.
#[derive(Debug, Clone)]
pub(crate) struct Hole {
    pub(crate) name: String,
    pub(crate) scope: Vec<(String, Term)>,
    pub(crate) ty: Term,
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

    /// The holes elaborated so far, in the order they were met.
    pub(crate) fn holes(&self) -> &[Hole] {
        &self.holes
    }

    fn declare_hole(&mut self, meta: &Term, scope: &[(String, Term)], ty: &Term) {
        if let Term::Var(name) = meta {
            self.holes.push(Hole {
                name: name.clone(),
                scope: scope.to_vec(),
                ty: ty.clone(),
            });
        }
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
        Term::Const { name, levels } => Term::Const {
            name: name.clone(),
            levels: levels.clone(),
        },
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Term::Pi {
            param: param.clone(),
            param_type: Box::new(instantiate(param_type, mctx)),
            body_type: Box::new(instantiate(body_type, mctx)),
        },
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(instantiate(param_type, mctx)),
            body: Box::new(instantiate(body, mctx)),
        },
        Term::App(f, a) => Term::App(
            Box::new(instantiate(f, mctx)),
            Box::new(instantiate(a, mctx)),
        ),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(instantiate(discriminant, mctx)),
            motive: Box::new(instantiate(motive, mctx)),
            cases: cases.iter().map(|c| instantiate(c, mctx)).collect(),
        },
        Term::Fix { name, body } => Term::Fix {
            name: name.clone(),
            body: Box::new(instantiate(body, mctx)),
        },
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs
                .iter()
                .map(|(n, b)| (n.clone(), instantiate(b, mctx)))
                .collect(),
            index: *index,
        },
        Term::Let {
            name,
            ty,
            value,
            body,
        } => Term::Let {
            name: name.clone(),
            ty: Box::new(instantiate(ty, mctx)),
            value: Box::new(instantiate(value, mctx)),
            body: Box::new(instantiate(body, mctx)),
        },
        Term::Annotated { annotation, term } => Term::Annotated {
            annotation: annotation.clone(),
            term: Box::new(instantiate(term, mctx)),
        },
    }
}

/// Whether `term` mentions any metavariable, solved or not.
pub(crate) fn mentions_meta(term: &Term) -> bool {
    match term {
        Term::Var(name) => is_meta(name),
        Term::Global(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole | Term::Const { .. } => false,
        Term::Pi {
            param_type,
            body_type,
            ..
        } => mentions_meta(param_type) || mentions_meta(body_type),
        Term::Lambda {
            param_type, body, ..
        } => mentions_meta(param_type) || mentions_meta(body),
        Term::App(f, a) => mentions_meta(f) || mentions_meta(a),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            mentions_meta(discriminant) || mentions_meta(motive) || cases.iter().any(mentions_meta)
        }
        Term::Fix { body, .. } => mentions_meta(body),
        Term::MutualFix { defs, .. } => defs.iter().any(|(_, body)| mentions_meta(body)),
        Term::Let {
            ty, value, body, ..
        } => mentions_meta(ty) || mentions_meta(value) || mentions_meta(body),
        Term::Annotated { term, .. } => mentions_meta(term),
    }
}

/// Whether the metavariable `m` occurs in `term` — the occurs-check that keeps the
/// solution acyclic (`?m := f ?m` would loop).
fn occurs(m: &str, term: &Term) -> bool {
    match term {
        Term::Var(name) => name == m,
        Term::Global(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole | Term::Const { .. } => false,
        Term::Pi {
            param_type,
            body_type,
            ..
        } => occurs(m, param_type) || occurs(m, body_type),
        Term::Lambda {
            param_type, body, ..
        } => occurs(m, param_type) || occurs(m, body),
        Term::App(f, a) => occurs(m, f) || occurs(m, a),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => occurs(m, discriminant) || occurs(m, motive) || cases.iter().any(|c| occurs(m, c)),
        Term::Fix { body, .. } => occurs(m, body),
        Term::MutualFix { defs, .. } => defs.iter().any(|(_, b)| occurs(m, b)),
        Term::Let {
            ty, value, body, ..
        } => occurs(m, ty) || occurs(m, value) || occurs(m, body),
        Term::Annotated { term, .. } => occurs(m, term),
    }
}

/// `instantiate` then weak-head-normalize — the form unification compares.
fn resolve(ctx: &Context, mctx: &MetaCtx, t: &Term) -> Term {
    normalize(ctx, &instantiate(t, mctx))
}

/// Expose only the Pi needed by the application telescope. An already-visible
/// Pi does not need its potentially enormous domain/codomain normalized merely
/// to peel a binder. Arguments and the assembled term still undergo all normal
/// elaborator and kernel checks. Active rewrites retain the old full path;
/// hidden heads retain elaborator (including irreducible) transparency.
fn resolve_application_pi(ctx: &Context, mctx: &MetaCtx, t: &Term) -> Term {
    let instantiated = instantiate(t, mctx);
    if ctx.simp_rules().is_empty() && matches!(instantiated, Term::Pi { .. }) {
        instantiated
    } else {
        normalize(ctx, &instantiated)
    }
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
    // Two terms that are already the same term, with no metavariable left to solve, unify with
    // nothing assigned: say so before normalizing either (a declaration whose inferred type IS its
    // declared type is the common case, and its normal form can be enormous).
    let (a, b) = (instantiate(a, mctx), instantiate(b, mctx));
    if !mentions_meta(&a) && crate::type_checker::alpha_equivalent(&a, &b) {
        return true;
    }
    if positive::equivalent_scoped(ctx, &a, &b) {
        return true;
    }
    #[cfg(test)]
    LAZY_UNIFY_FULL_NORMALIZATIONS.with(|n| n.set(n.get() + 1));
    let a = normalize(ctx, &a);
    let b = normalize(ctx, &b);
    // A normal form can be far deeper than the written terms the caller's stack was sized for (a
    // statement over literal arithmetic the positive check declines normalizes to a deep partial
    // term), so the structural walk runs on a stack sized for what normalization produced, and a
    // comparison past the kernel's depth door is a recorded resource stop, as in conversion.
    let depth = crate::term_store::term_depth(&a).max(crate::term_store::term_depth(&b));
    if depth > crate::type_checker::MAX_TERM_DEPTH {
        crate::reduction::record_resource_stop(crate::Unreduced::TooDeep);
        return false;
    }
    crate::term_store::grow_stack_for_depth(depth, move || unify_normal(ctx, mctx, lctx, a, b))
}

/// [`unify_in`] past normalization: `a` and `b` are normal forms, and the stack is sized for them.
fn unify_normal(
    ctx: &Context,
    mctx: &mut MetaCtx,
    lctx: &[(String, Term)],
    a: Term,
    b: Term,
) -> bool {
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
        (
            Term::Lambda {
                param,
                param_type,
                body,
            },
            other,
        )
        | (
            other,
            Term::Lambda {
                param,
                param_type,
                body,
            },
        ) if !matches!(other, Term::Lambda { .. }) => {
            let applied = Term::App(Box::new(other.clone()), Box::new(Term::Var(param.clone())));
            let mut lctx2 = lctx.to_vec();
            lctx2.push((param.clone(), (**param_type).clone()));
            return unify_in(ctx, mctx, &lctx2, body, &applied);
        }
        _ => {}
    }

    match (&a, &b) {
        // A level METAVARIABLE (a universe-polymorphic head's level, instantiated for this
        // application) is solved by the kernel's application rule when the finished term is
        // checked; the elaborator has no level assignment of its own and does not refuse one.
        (Term::Sort(u), Term::Sort(v)) => {
            u.equiv(v)
                || crate::term::mentions_level_meta(&a)
                || crate::term::mentions_level_meta(&b)
        }
        (Term::Global(x), Term::Global(y)) => x == y,
        (Term::Var(x), Term::Var(y)) => x == y,
        (Term::Lit(x), Term::Lit(y)) => x == y,
        (Term::Hole, Term::Hole) => true,
        (Term::App(f1, a1), Term::App(f2, a2)) => {
            unify_in(ctx, mctx, lctx, f1, f2) && unify_in(ctx, mctx, lctx, a1, a2)
        }
        (
            Term::Pi {
                param: p1,
                param_type: t1,
                body_type: b1,
            },
            Term::Pi {
                param: p2,
                param_type: t2,
                body_type: b2,
            },
        ) => unify_binder(ctx, mctx, lctx, t1, b1, p1, t2, b2, p2),
        (
            Term::Lambda {
                param: p1,
                param_type: t1,
                body: b1,
            },
            Term::Lambda {
                param: p2,
                param_type: t2,
                body: b2,
            },
        ) => unify_binder(ctx, mctx, lctx, t1, b1, p1, t2, b2, p2),
        (
            Term::Const {
                name: n1,
                levels: l1,
            },
            Term::Const {
                name: n2,
                levels: l2,
            },
        ) => n1 == n2 && l1.len() == l2.len() && l1.iter().zip(l2.iter()).all(|(x, y)| x.equiv(y)),
        _ => false,
    }
}

/// Try to solve a Miller pattern `a = ?M x̄ =?= b`. Returns `None` if `a` is not a pattern
/// (the caller falls through to structural unification), `Some(true)` if solved, and
/// `Some(false)` if it IS a pattern but unsolvable (occurs-check / an out-of-scope
/// variable on the right). The arguments `x̄` must be DISTINCT bound variables of `lctx`,
/// and `b` may mention only those variables (plus globals/metavariables).
fn try_pattern(mctx: &mut MetaCtx, lctx: &[(String, Term)], a: &Term, b: &Term) -> Option<bool> {
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
        sol = Term::Lambda {
            param: name.clone(),
            param_type: Box::new(ty),
            body: Box::new(sol),
        };
    }
    mctx.solve(&meta, sol);
    Some(true)
}

/// Whether every FREE variable of `t` (tracking binders in `bound`) is either a
/// metavariable, bound inside `t`, or one of the `allowed` pattern arguments — the
/// condition for `λallowed. t` to be well-scoped.
fn pattern_rhs_in_scope(t: &Term, allowed: &[String], bound: &mut Vec<String>) -> bool {
    match t {
        Term::Var(v) => {
            is_meta(v) || bound.iter().any(|b| b == v) || allowed.iter().any(|a| a == v)
        }
        Term::Sort(_) | Term::Global(_) | Term::Lit(_) | Term::Hole | Term::Const { .. } => true,
        Term::App(f, a) => {
            pattern_rhs_in_scope(f, allowed, bound) && pattern_rhs_in_scope(a, allowed, bound)
        }
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            if !pattern_rhs_in_scope(param_type, allowed, bound) {
                return false;
            }
            bound.push(param.clone());
            let ok = pattern_rhs_in_scope(body_type, allowed, bound);
            bound.pop();
            ok
        }
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
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
            let ok = defs
                .iter()
                .all(|(_, b)| pattern_rhs_in_scope(b, allowed, bound));
            for _ in defs {
                bound.pop();
            }
            ok
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            pattern_rhs_in_scope(discriminant, allowed, bound)
                && pattern_rhs_in_scope(motive, allowed, bound)
                && cases
                    .iter()
                    .all(|c| pattern_rhs_in_scope(c, allowed, bound))
        }
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
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
        Term::Annotated { term, .. } => pattern_rhs_in_scope(term, allowed, bound),
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

/// Resolve a nonnegative numeral through the declared OfNat dictionary before
/// using native literal conversion or LOGOS's record injection.
fn elaborate_ofnat(
    ctx: &Context,
    mctx: &mut MetaCtx,
    lit: &crate::term::Literal,
    expected: &Term,
) -> KernelResult<Option<Term>> {
    use crate::term::{Literal, Universe};
    let zero = logicaffeine_base::BigInt::from_i64(0);
    let n = match lit {
        Literal::Nat(n) | Literal::BigInt(n) if *n >= zero => n.clone(),
        Literal::Int(n) if *n >= 0 => logicaffeine_base::BigInt::from_i64(*n),
        _ => return Ok(None),
    };
    if ctx.poly_declaration("OfNat").is_none() {
        return Ok(None);
    }
    let expected = instantiate(expected, mctx);
    if has_unsolved_meta(&expected) {
        return Ok(None);
    }
    let level: Universe = match crate::type_checker::infer_sort(ctx, &expected) {
        Ok(level) => level,
        Err(_) => return Ok(None),
    };
    let cref = |name: &str| Term::Const {
        name: name.to_string(),
        levels: vec![level.clone()],
    };
    let app = |f: Term, a: Term| Term::App(Box::new(f), Box::new(a));
    let numeral = Term::Lit(Literal::Nat(n));
    let required = app(app(cref("OfNat"), expected.clone()), numeral.clone());
    let mut trial = mctx.clone();
    let Some(dictionary) = resolve_instance(ctx, &mut trial, &required) else {
        return Ok(None);
    };
    // Instance entries are search metadata, not typing evidence. Certify the selected
    // dictionary and its projection before committing any metavariable assignments.
    let value = app(
        app(app(cref("OfNat_ofNat"), expected.clone()), numeral),
        dictionary,
    );
    let actual = infer_type(ctx, &value)?;
    if !unify(ctx, &mut trial, &actual, &expected) {
        return Err(KernelError::TypeMismatch {
            expected: expected.to_string(),
            found: actual.to_string(),
        });
    }
    *mctx = trial;
    Ok(Some(normalize(ctx, &value)))
}

/// Native literal conversion used when no declared numeral dictionary applies.
/// The expected carrier chooses the literal representation; typing still checks it.
///
/// The kernel types a literal by its kind and must keep doing so: `Literal::Nat(3) : Nat` is the
/// kernel's answer and an expected type is never allowed to change what a term IS. What an
/// expected type may do is decide which term to BUILD, which is elaboration's job — and `3`
/// written where a `Nat` is wanted and `3` written where an `Int` is wanted are two different
/// terms of the same surface token, exactly as `(3 : Nat)` and `(3 : Int)` are in Lean.
///
/// Only the two INTEGER kinds move, and only between each other. `Nat` is the non-negative half
/// of `Int`, so a negative numeral at `Nat` has no reading at all and is left exactly as it was,
/// to be refused where it always was; every other kind (`Text`, `Float`, `Date`, `Duration`,
/// `Moment`) has a single carrier and is never retyped. Returning `None` therefore means "this
/// numeral cannot mean that", never "I did not look".
///
/// The expected type is REDUCED before it is read, so an abbreviation (`Definition Count := Nat.`)
/// asks for the same numeral its right-hand side does. A type the reduction leaves as anything but
/// `Nat` or `Int` — a metavariable, a record, a Π — declines here and falls through to the
/// injection path, which is the mechanism for a user carrier.
fn retype_numeral(ctx: &Context, lit: &crate::term::Literal, expected: &Term) -> Option<Term> {
    use crate::term::Literal;
    let carrier = match normalize(ctx, expected) {
        Term::Global(n) => n,
        _ => return None,
    };
    let zero = logicaffeine_base::BigInt::from_i64(0);
    match (carrier.as_str(), lit) {
        // `(n : Nat)` for a numeral written as an `Int`. The `n >= 0` guard is the whole content
        // of the `Nat`/`Int` distinction: below zero there is no `Nat` to take.
        ("Nat", Literal::Int(n)) if *n >= 0 => Some(Term::Lit(Literal::Nat(
            logicaffeine_base::BigInt::from_i64(*n),
        ))),
        ("Nat", Literal::BigInt(n)) if *n >= zero => Some(Term::Lit(Literal::Nat(n.clone()))),
        // `(n : Int)` for a numeral written as a `Nat`. Always available — every `Nat` is an
        // `Int` — and routed through `int_lit` so the one-representation-per-value invariant
        // definitional equality of literals rests on is preserved above the `i64` boundary.
        ("Int", Literal::Nat(n)) => Some(Term::Lit(crate::term::int_lit(n.clone()))),
        _ => None,
    }
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
    // A reference to a UNIVERSE-POLYMORPHIC declaration by its bare name, with the levels left
    // unwritten. Solve them from the arguments and continue with the explicit `Const{name, ℓ…}`.
    if let Some(rewritten) = solve_unwritten_levels(ctx, term)? {
        return elaborate_in(ctx, mctx, lctx, &rewritten, expected);
    }
    if let (Term::Lit(lit), Some(exp)) = (term, expected) {
        if let Some(value) = elaborate_ofnat(ctx, mctx, lit, exp)? {
            let ty = infer_type(ctx, &value)?;
            return Ok((value, ty));
        }
    }
    match term {
        Term::Hole => {
            let m = mctx.fresh();
            let ty = expected.cloned().unwrap_or_else(|| mctx.fresh());
            mctx.declare_hole(&m, lctx, &ty);
            Ok((m, ty))
        }
        Term::App(f, a) => {
            let (f_elab, f_ty) = elaborate_in(ctx, mctx, lctx, f, None)?;
            let f_ty = resolve_application_pi(ctx, mctx, &f_ty);
            match f_ty {
                Term::Pi {
                    param,
                    param_type,
                    body_type,
                } => {
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
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            // Descend under the binder, extending BOTH the kernel context (for leaf type
            // inference) and the unification context (for pattern unification). The body's
            // expected type is the expected `Π`'s codomain, α-renamed to this binder.
            let body_expected = match expected.map(|e| resolve_application_pi(ctx, mctx, e)) {
                Some(Term::Pi {
                    param: ep,
                    body_type: ecod,
                    ..
                }) => Some(if ep == *param {
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
                if unify_in(ctx, mctx, lctx, &ty, exp) {
                    return Ok((term.clone(), ty));
                }
                // A NUMERAL that did not fit is retyped at the carrier the position asks for
                // (see [`retype_numeral`]). This is the ARGUMENT and FIELD door — `f (3 : Nat)`
                // and `⟨3⟩ : MyNum` reach it — and it has to be here rather than only on the
                // top-level surface arm, or the same token would mean different things at
                // depth 0 and depth 1.
                //
                // Attempted on a CLONE of the metavariable context and committed only when the
                // retyped literal really does fit: a failed attempt must leave no solution
                // behind, or a numeral that cannot be retyped would still have moved the
                // unifier.
                if let Term::Lit(l) = term {
                    let exp_i = instantiate(exp, mctx);
                    if let Some(retyped) = retype_numeral(ctx, l, &exp_i) {
                        let rty = infer_type(ctx, &retyped)?;
                        let mut trial = mctx.clone();
                        if unify_in(ctx, &mut trial, lctx, &rty, exp) {
                            *mctx = trial;
                            return Ok((retyped, rty));
                        }
                    }
                }
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
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            let disc = fill_match_motives(ctx, discriminant, None)?;
            if cases
                .iter()
                .any(|c| crate::interface::raw_match::as_case(c).is_some())
            {
                return crate::interface::raw_match::compile(ctx, disc, motive, cases, expected);
            }

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
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            let ext = ctx.extend(param, (**param_type).clone());
            // The body's expected type is the codomain of the expected `Π`, α-renamed. It is read
            // only by a `match` the λ-spine ends in (an application, a `Π` or a `fix` hands its
            // subterms no expected type), so it is computed only for one: a declared type is
            // otherwise never normalized just to be discarded.
            let expected = expected.filter(|_| ends_in_match(body));
            let body_expected = match expected.map(|e| normalize(ctx, e)) {
                Some(Term::Pi {
                    param: ep,
                    body_type,
                    ..
                }) => Some(if ep == *param {
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
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
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
        Term::Annotated {
            annotation,
            term: inner,
        } => Ok(Term::Annotated {
            annotation: annotation.clone(),
            term: Box::new(fill_match_motives(ctx, inner, expected)?),
        }),
        _ => Ok(term.clone()),
    }
}

/// Whether `term`'s λ-spine ends in a `match` — the only place [`fill_match_motives`] reads an
/// expected type.
fn ends_in_match(term: &Term) -> bool {
    let mut term = term;
    loop {
        match term {
            Term::Match { .. } => return true,
            Term::Lambda { body, .. } => term = body,
            _ => return false,
        }
    }
}

/// Build the motive `λx:I. T[disc := x]` for a `match` written without a `return` clause,
/// by ABSTRACTING the discriminant out of the expected type — the Miller-pattern solution
/// of `?P disc =?= T`. When `T` mentions the discriminant the motive is DEPENDENT (so a
/// match whose result type varies per branch, like an eliminator `Π(n). P n`, elaborates);
/// when it does not, this collapses to the constant motive `λ_:I. T`. A bare variable
/// discriminant just captures the free variable; any other discriminant has its
/// occurrences replaced.
pub(crate) fn infer_match_motive(
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
    Ok(Term::Lambda {
        param,
        param_type: Box::new(disc_ty),
        body: Box::new(body),
    })
}

/// Replace every subterm structurally equal to `target` by `repl`.
fn replace_subterm(t: &Term, target: &Term, repl: &Term) -> Term {
    if t == target {
        return repl.clone();
    }
    match t {
        Term::Var(_)
        | Term::Global(_)
        | Term::Sort(_)
        | Term::Lit(_)
        | Term::Hole
        | Term::Const { .. } => t.clone(),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Term::Pi {
            param: param.clone(),
            param_type: Box::new(replace_subterm(param_type, target, repl)),
            body_type: Box::new(replace_subterm(body_type, target, repl)),
        },
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(replace_subterm(param_type, target, repl)),
            body: Box::new(replace_subterm(body, target, repl)),
        },
        Term::App(f, a) => Term::App(
            Box::new(replace_subterm(f, target, repl)),
            Box::new(replace_subterm(a, target, repl)),
        ),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(replace_subterm(discriminant, target, repl)),
            motive: Box::new(replace_subterm(motive, target, repl)),
            cases: cases
                .iter()
                .map(|c| replace_subterm(c, target, repl))
                .collect(),
        },
        Term::Fix { name, body } => Term::Fix {
            name: name.clone(),
            body: Box::new(replace_subterm(body, target, repl)),
        },
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs
                .iter()
                .map(|(n, b)| (n.clone(), replace_subterm(b, target, repl)))
                .collect(),
            index: *index,
        },
        Term::Let {
            name,
            ty,
            value,
            body,
        } => Term::Let {
            name: name.clone(),
            ty: Box::new(replace_subterm(ty, target, repl)),
            value: Box::new(replace_subterm(value, target, repl)),
            body: Box::new(replace_subterm(body, target, repl)),
        },
        Term::Annotated { annotation, term } => Term::Annotated {
            annotation: annotation.clone(),
            term: Box::new(replace_subterm(term, target, repl)),
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
    // First candidate becomes the OUTERMOST binder, quantified at the sort its OWN USES force —
    // `Sort u` for a fresh universe parameter when nothing forces one, which is Lean's
    // `autoImplicit`, and not `Type 0`.
    //
    // Pinning an invented binder to `Type 0` is a silent MONOMORPHISATION: `id : A -> A` then
    // accepts every value but no TYPE, so `id Nat` — where `A` has to be `Type 0`, one level up —
    // fails with a universe mismatch the writer never wrote a universe to cause. The level is a
    // real parameter of the declaration, and the kernel already solves such a parameter at each
    // use site from the argument's own type (`implicit_levels_for_head`), so the honest binder
    // costs nothing and monomorphises nothing.
    //
    // Where a use DOES force a level it is taken, which is the same answer Lean's unifier
    // reaches: `nil : TList A` applies `A` to a `Type 0 → Type 0` family, so `A : Type 0` and the
    // declaration is not polymorphic after all. Guessing `Sort u` there would produce a body the
    // kernel then refuses, and the diagnostic would name a universe the writer never mentioned.
    for name in candidates.iter().rev() {
        let sort = autobind_sort(ctx, name, ty, body);
        new_ty = Term::Pi {
            param: name.clone(),
            param_type: Box::new(sort.clone()),
            body_type: Box::new(new_ty),
        };
        new_body = Term::Lambda {
            param: name.clone(),
            param_type: Box::new(sort),
            body: Box::new(new_body),
        };
    }
    (new_ty, new_body, existing_implicit + candidates.len())
}

/// The sort an auto-bound type variable is quantified at: the CONCRETE level its own uses force,
/// or a fresh universe parameter when nothing does.
///
/// A use forces a level when the variable is passed to something whose parameter at that position
/// is a written sort — `TList : Type 0 → Type 0` forces `A : Type 0` in `nil : TList A`. A
/// variable that appears only as a BINDER'S TYPE (`fun a : A => …`) constrains nothing, and that
/// is the polymorphic case.
fn autobind_sort(ctx: &Context, name: &str, ty: &Term, body: &Term) -> Term {
    forced_sort(ctx, name, ty)
        .or_else(|| forced_sort(ctx, name, body))
        .unwrap_or_else(|| Term::Sort(crate::term::Universe::Var(autobind_level(name))))
}

/// Walk `t` for an application that passes `Global(name)` to a global whose parameter at that
/// position is a concrete sort, and return that sort.
fn forced_sort(ctx: &Context, name: &str, t: &Term) -> Option<Term> {
    if let Term::App(..) = t {
        let (head, args) = spine(t);
        if let Term::Global(h) = &head {
            if let Some(head_ty) = ctx.get_global(h) {
                let mut cur = head_ty;
                for a in &args {
                    let Term::Pi {
                        param_type,
                        body_type,
                        ..
                    } = cur
                    else {
                        break;
                    };
                    if matches!(a, Term::Global(n) if n == name) {
                        if let Term::Sort(u) = param_type.as_ref() {
                            return Some(Term::Sort(u.clone()));
                        }
                    }
                    cur = body_type;
                }
            }
        }
        for a in &args {
            if let Some(s) = forced_sort(ctx, name, a) {
                return Some(s);
            }
        }
        return forced_sort(ctx, name, &head);
    }
    match t {
        Term::Pi {
            param_type,
            body_type,
            ..
        } => forced_sort(ctx, name, param_type).or_else(|| forced_sort(ctx, name, body_type)),
        Term::Lambda {
            param_type, body, ..
        } => forced_sort(ctx, name, param_type).or_else(|| forced_sort(ctx, name, body)),
        Term::Fix { body, .. } => forced_sort(ctx, name, body),
        Term::MutualFix { defs, .. } => defs.iter().find_map(|(_, b)| forced_sort(ctx, name, b)),
        Term::Let {
            ty, value, body, ..
        } => forced_sort(ctx, name, ty)
            .or_else(|| forced_sort(ctx, name, value))
            .or_else(|| forced_sort(ctx, name, body)),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => forced_sort(ctx, name, discriminant)
            .or_else(|| forced_sort(ctx, name, motive))
            .or_else(|| cases.iter().find_map(|c| forced_sort(ctx, name, c))),
        _ => None,
    }
}

/// The universe parameter an unconstrained auto-bound type variable is quantified at. Named after
/// the variable so two auto-bound binders in one declaration get INDEPENDENT levels — `constf :
/// A -> B -> A` must not force `A` and `B` to the same universe.
fn autobind_level(name: &str) -> String {
    format!("u_{name}")
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
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            collect_autobind(ctx, param_type, acc);
            collect_autobind(ctx, body_type, acc);
        }
        Term::Lambda {
            param_type, body, ..
        } => {
            collect_autobind(ctx, param_type, acc);
            collect_autobind(ctx, body, acc);
        }
        Term::App(f, a) => {
            collect_autobind(ctx, f, acc);
            collect_autobind(ctx, a, acc);
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            collect_autobind(ctx, discriminant, acc);
            collect_autobind(ctx, motive, acc);
            for c in cases {
                collect_autobind(ctx, c, acc);
            }
        }
        Term::Fix { body, .. } => collect_autobind(ctx, body, acc),
        Term::Annotated { term, .. } => collect_autobind(ctx, term, acc),
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
        Term::Fix {
            name: name.to_string(),
            body: Box::new(global_to_var(body, name)),
        }
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
        Term::Pi {
            param_type,
            body_type,
            ..
        } => references_global(param_type, name) || references_global(body_type, name),
        Term::Lambda {
            param_type, body, ..
        } => references_global(param_type, name) || references_global(body, name),
        Term::App(f, a) => references_global(f, name) || references_global(a, name),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            references_global(discriminant, name)
                || references_global(motive, name)
                || cases.iter().any(|c| references_global(c, name))
        }
        Term::Fix { body, .. } => references_global(body, name),
        Term::MutualFix { defs, .. } => defs.iter().any(|(_, b)| references_global(b, name)),
        Term::Let {
            ty, value, body, ..
        } => {
            references_global(ty, name)
                || references_global(value, name)
                || references_global(body, name)
        }
        Term::Annotated { term, .. } => references_global(term, name),
    }
}

fn global_to_var(term: &Term, name: &str) -> Term {
    match term {
        Term::Global(n) if n == name => Term::Var(n.clone()),
        Term::Global(_)
        | Term::Var(_)
        | Term::Sort(_)
        | Term::Lit(_)
        | Term::Hole
        | Term::Const { .. } => term.clone(),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Term::Pi {
            param: param.clone(),
            param_type: Box::new(global_to_var(param_type, name)),
            body_type: Box::new(global_to_var(body_type, name)),
        },
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(global_to_var(param_type, name)),
            body: Box::new(global_to_var(body, name)),
        },
        Term::App(f, a) => Term::App(
            Box::new(global_to_var(f, name)),
            Box::new(global_to_var(a, name)),
        ),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(global_to_var(discriminant, name)),
            motive: Box::new(global_to_var(motive, name)),
            cases: cases.iter().map(|c| global_to_var(c, name)).collect(),
        },
        Term::Fix { name: fname, body } => Term::Fix {
            name: fname.clone(),
            body: Box::new(global_to_var(body, name)),
        },
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs
                .iter()
                .map(|(fname, b)| (fname.clone(), global_to_var(b, name)))
                .collect(),
            index: *index,
        },
        Term::Let {
            name: lname,
            ty,
            value,
            body,
        } => Term::Let {
            name: lname.clone(),
            ty: Box::new(global_to_var(ty, name)),
            value: Box::new(global_to_var(value, name)),
            body: Box::new(global_to_var(body, name)),
        },
        Term::Annotated { annotation, term } => Term::Annotated {
            annotation: annotation.clone(),
            term: Box::new(global_to_var(term, name)),
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
    let elaborated = elab_surface_uncoerced(ctx, mctx, term, expected)?;
    Ok(coerce_to_expected(ctx, mctx, elaborated, expected))
}

/// An ASCRIBED type drives coercion insertion, the same way a parameter type does. Writing
/// `Definition coerced : Nat := true.` with `Bool ⇒ Nat` registered must produce
/// `boolToNat true`, not a `Bool` stamped with the type `Nat` — the declared type is the
/// constraint the term has to meet, and a coercion is how Lean meets it.
///
/// A term that already has the expected type is returned untouched, and a mismatch with no
/// registered path is left exactly as it was, so it fails where it would have failed before.
fn coerce_to_expected(
    ctx: &Context,
    mctx: &mut MetaCtx,
    elaborated: Term,
    expected: Option<&Term>,
) -> Term {
    let Some(exp) = expected else {
        return elaborated;
    };
    let Ok(actual) = infer_type(ctx, &elaborated) else {
        return elaborated;
    };
    let mut trial = mctx.clone();
    if unify(ctx, &mut trial, &actual, exp) {
        *mctx = trial;
        return elaborated;
    }
    let (actual, exp) = (instantiate(&actual, mctx), instantiate(exp, mctx));
    coerce_term(ctx, mctx, &elaborated, &actual, &exp).unwrap_or(elaborated)
}

fn elab_surface_uncoerced(
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
    if let Some(rewritten) = solve_unwritten_levels(ctx, term)? {
        return elab_surface_uncoerced(ctx, mctx, &rewritten, expected);
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

            // SURFACE ARGUMENT SUGAR, read off the RAW spine before anything is elaborated: a
            // named argument and the `..` ellipsis are reserved marker applications, and
            // elaborating one as an ordinary term would report the marker as an unbound global.
            let ellipsis = args.last().is_some_and(|a| is_ellipsis_marker(a));
            let has_named = args.iter().any(|a| as_named_argument(a).is_some());
            if ellipsis || has_named {
                let resolved = match cur {
                    Term::Global(name) => ctx.get_global(name).cloned().map(|t| (name, t)),
                    _ => None,
                };
                let Some((name, head_ty)) = resolved else {
                    // Neither form means anything without a DECLARATION to read parameter names
                    // and arity from. Say so: dropping the request would silently change `f ..`
                    // into `f`, and elaborating the marker as an ordinary global would hand the
                    // kernel a term built out of a piece of syntax.
                    return Err(KernelError::CertificationError(format!(
                        "`{}` cannot take named arguments or `..`: it is not a declared global",
                        cur
                    )));
                };
                if ellipsis {
                    args.pop();
                }
                return elaborate_by_name(
                    ctx, mctx, name, cur, &head_ty, &args, ellipsis, expected,
                );
            }

            let head = elab_surface(ctx, mctx, cur, None)?;
            let args: Vec<Term> = args
                .iter()
                .map(|a| elab_surface(ctx, mctx, a, None))
                .collect::<KernelResult<_>>()?;

            // Route EVERY global-headed application through the typed elaboration path — even one
            // with no implicits — so argument type-checking and COERCION insertion apply
            // uniformly. An ANONYMOUS head takes the same path when its own binders carry kinds
            // (`Term::Annotated`): `(fun {α} (a : α) => a) 3` inserts `α` exactly as a declared
            // `{α} → α → α` does, because there is no name to record the kinds under.
            let typed = match &head {
                Term::Global(name) => ctx.get_global(name).cloned().map(|head_ty| {
                    let kinds = application_kinds(
                        ctx.binder_kinds(name),
                        ctx.implicit_args(name),
                        args.len(),
                    );
                    (head_ty, kinds)
                }),
                _ => annotated_binder_kinds(&head).and_then(|recorded| {
                    let head_ty = infer_type(ctx, &head).ok()?;
                    let leading = recorded
                        .iter()
                        .take_while(|k| **k == ParamKind::Implicit)
                        .count();
                    Some((
                        head_ty,
                        application_kinds(Some(&recorded), leading, args.len()),
                    ))
                }),
            };
            if let Some((head_ty, kinds)) = typed {
                match elaborate_app_against(ctx, mctx, &head, &head_ty, &kinds, &args, expected) {
                    Ok((t, _)) => return Ok(t),
                    Err(typed_err) => {
                        // A function type hidden by elaborator transparency must stay
                        // hidden; asking kernel conversion here would unfold it again.
                        if matches!(typed_err, KernelError::NotAFunction(_)) {
                            return Err(typed_err);
                        }
                        // Fall through to a plain application if typed elaboration did not
                        // apply (e.g. the head is not a function of the expected arity) —
                        // preserving the permissive behaviour for non-standard shapes.
                        //
                        // But an elaboration that returns `Ok` must return something the
                        // kernel accepts. If the plain application is ill-typed for a
                        // reason the typed path already diagnosed, report THAT — the
                        // alternative is an `Ok` whose failure surfaces later in
                        // `infer_type`, naming neither the argument nor the coercion that
                        // was missing.
                        //
                        // An UNBOUND VARIABLE is the one failure that does not mean the
                        // term is wrong: this pass carries no local context, so a body
                        // mentioning its own binder cannot be typed here and must keep the
                        // permissive answer.
                        let plain = args
                            .iter()
                            .cloned()
                            .fold(head.clone(), |f, a| Term::App(Box::new(f), Box::new(a)));
                        return match infer_type(ctx, &plain) {
                            Ok(_) | Err(KernelError::UnboundVariable(_)) => Ok(plain),
                            Err(_) => Err(typed_err),
                        };
                    }
                }
            }
            Ok(args
                .into_iter()
                .fold(head, |f, a| Term::App(Box::new(f), Box::new(a))))
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
        // POSTPONEMENT — a λ whose parameter type the writer did not give.
        //
        // `fun x => Succ x` has NO type at all until something says what `x` is. Lean postpones
        // the whole elaboration problem onto a queue and re-runs it when the expected type
        // arrives; a single pass cannot postpone, but it does not have to: by the time this arm
        // runs, the expected type either is known or never will be. So the domain is read off it
        // here, and the body is elaborated against the codomain WITH the binder in scope — which
        // is the same answer the queue would have reached, one pass earlier.
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            let exp_pi = expected.map(|e| resolve_application_pi(ctx, mctx, e));
            let (exp_dom, exp_cod) = match &exp_pi {
                Some(Term::Pi {
                    param: ep,
                    param_type: d,
                    body_type: b,
                }) => (
                    Some((**d).clone()),
                    // The codomain may mention the expected type's own binder; α-rename it to
                    // this λ's binder so a DEPENDENT expected type stays correct.
                    Some(substitute(b, ep, &Term::Var(param.clone()))),
                ),
                _ => (None, None),
            };
            let dom = match (param_type.as_ref(), &exp_dom) {
                (Term::Hole, Some(d)) => d.clone(),
                _ => ensure_type(ctx, elab_surface(ctx, mctx, param_type, None)?),
            };
            // A domain that is still a hole cannot be put in scope — the binder would type-check
            // against anything. Elaborate the body without it, exactly as before.
            let body = if matches!(dom, Term::Hole) {
                elab_surface(ctx, mctx, body, exp_cod.as_ref())?
            } else {
                let scoped = ctx.extend(param, dom.clone());
                elab_surface(&scoped, mctx, body, exp_cod.as_ref())?
            };
            Ok(Term::Lambda {
                param: param.clone(),
                param_type: Box::new(dom),
                body: Box::new(body),
            })
        }
        // A Π has TWO type positions, its domain and its codomain, and each is an `ensureType`
        // site — which is why `Π(x : t). t` with a `CoeSort`-able `t` coerces on both sides.
        // The binder is in scope for the codomain, at its COERCED domain, so a codomain
        // mentioning it reads the type the elaborated binder actually has.
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            let dom = ensure_type(ctx, elab_surface(ctx, mctx, param_type, None)?);
            let scoped = ctx.extend(param, dom.clone());
            let cod = ensure_type(&scoped, elab_surface(&scoped, mctx, body_type, None)?);
            Ok(Term::Pi {
                param: param.clone(),
                param_type: Box::new(dom),
                body_type: Box::new(cod),
            })
        }
        Term::Fix { name, body } => Ok(Term::Fix {
            name: name.clone(),
            body: Box::new(elab_surface(ctx, mctx, body, None)?),
        }),
        Term::Match { cases, .. }
            if cases
                .iter()
                .any(|c| crate::interface::raw_match::as_case(c).is_some()) =>
        {
            let compiled = fill_match_motives(ctx, term, expected)?;
            elab_surface(ctx, mctx, &compiled, expected)
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Ok(Term::Match {
            discriminant: Box::new(elab_surface(ctx, mctx, discriminant, None)?),
            motive: Box::new(elab_surface(ctx, mctx, motive, None)?),
            cases: cases
                .iter()
                .map(|c| elab_surface(ctx, mctx, c, None))
                .collect::<KernelResult<_>>()?,
        }),
        // An annotation is elaborated THROUGH: the term under it meets the same expected type,
        // and the annotation stays on what it produced.
        Term::Annotated {
            annotation,
            term: inner,
        } => Ok(Term::Annotated {
            annotation: annotation.clone(),
            term: Box::new(elab_surface_uncoerced(ctx, mctx, inner, expected)?),
        }),
        // Resolve declared numeral dictionaries before applying LOGOS record injection.
        //
        // The kernel types a literal by its KIND and must keep doing so: `Literal::Nat(3) : Nat`
        // is the kernel's answer, and an expected type is not allowed to change what a term IS.
        // What an expected type may do is decide what term to BUILD, which is elaboration's job.
        //
        // If no dictionary applies, a single-field record can still use LOGOS's
        // constructor injection; declared instances always determine the value first.
        Term::Lit(_) => {
            let Some(exp) = expected else {
                return Ok(term.clone());
            };
            let exp = instantiate(exp, mctx);
            if let Term::Lit(lit) = term {
                if let Some(value) = elaborate_ofnat(ctx, mctx, lit, &exp)? {
                    return Ok(value);
                }
            }
            // The literal already has the expected type: nothing to do, and in particular no
            // wrapping of a `Nat` literal in a `Nat`-carrying record that happens to be expected.
            if matches!(infer_type(ctx, term), Ok(native) if crate::is_subtype(ctx, &native, &exp))
            {
                return Ok(term.clone());
            }
            // THE KIND CHANGE, before any injection is considered: `Yield 1` against a
            // declared `-> Nat` is the `Nat` numeral `1`, not an `Int` wrapped in whatever
            // single-field record happens to be expected. Injection is the door for a USER
            // carrier; the two built-in integer carriers are reached by retyping the token.
            if let Term::Lit(l) = term {
                if let Some(retyped) = retype_numeral(ctx, l, &exp) {
                    return Ok(retyped);
                }
            }
            match elaborate_anon_ctor(ctx, mctx, &exp, std::slice::from_ref(term)) {
                Ok(wrapped) => Ok(wrapped),
                // No injection available. Hand back the literal unchanged so the mismatch is
                // reported where it always was, by the kernel, against the real expected type.
                Err(_) => Ok(term.clone()),
            }
        }
        _ => Ok(term.clone()),
    }
}

/// The kinds an application of a head with `recorded` per-binder kinds and `implicit_args` leading
/// implicits supplies, given `explicit` written arguments: the recorded kinds (E2) when they exist
/// and name exactly that many explicit positions — so implicit and instance parameters interleave
/// with explicit ones — and otherwise the legacy model, the leading implicits then the explicit
/// arguments. One rule for a declared head and an anonymous one.
fn application_kinds(
    recorded: Option<&[ParamKind]>,
    implicit_args: usize,
    explicit: usize,
) -> Vec<ParamKind> {
    match recorded {
        Some(kinds) if kinds.iter().filter(|k| **k == ParamKind::Explicit).count() == explicit => {
            kinds.to_vec()
        }
        _ => {
            let mut kinds = vec![ParamKind::Implicit; implicit_args];
            kinds.extend(std::iter::repeat(ParamKind::Explicit).take(explicit));
            kinds
        }
    }
}

/// The per-binder kinds an anonymous `fun` telescope carries as [`crate::Annotation::Binder`]s,
/// outermost first — `None` when no binder of it carries one, so an unannotated head keeps the
/// plain application it always had.
fn annotated_binder_kinds(head: &Term) -> Option<Vec<ParamKind>> {
    let mut kinds = Vec::new();
    let mut cur = head;
    while let Term::Lambda { body, .. } = cur.peel_annotations() {
        kinds.push(cur.binder_kind().unwrap_or(ParamKind::Explicit));
        cur = body;
    }
    kinds
        .iter()
        .any(|k| *k != ParamKind::Explicit)
        .then_some(kinds)
}

/// How a function parameter is supplied during elaboration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    /// The caller provides the argument.
    Explicit,
    /// Inferred by unification (a fresh metavariable is inserted).
    Implicit,
    /// Lean's `⦃x : T⦄` — inferred like [`ParamKind::Implicit`], but inserted ONLY when a later
    /// EXPLICIT argument is actually supplied.
    ///
    /// The fourth inhabitant of Lean's `BinderInfo`, and the distinction is observable in one
    /// direction only: fully applied, a strict implicit and an eager one build the same term.
    /// Unapplied they do not. `nilOf ⦃α⦄ : List α` written on its own is still the polymorphic
    /// FUNCTION awaiting its type; inserting the metavariable there emits `nilOf ?0`, which
    /// nothing in the application can solve and which the kernel then reports as an unbound
    /// variable far from where it was made. Under-inserting leaves a term that is still correct;
    /// over-inserting manufactures one that is not.
    StrictImplicit,
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
    let mut fills: Vec<Fill> = Vec::with_capacity(kinds.len());
    let mut next_arg = 0usize;
    for (i, kind) in kinds.iter().enumerate() {
        // An implicit that NOTHING can determine is not inserted. If no later parameter takes
        // an explicit argument and no expected type was supplied, there is no constraint left
        // to solve it from, and filling it anyway emits `f ?0` — a term the kernel rejects as
        // an unbound variable, far from the application that created it. Leaving the function
        // polymorphic is both honest and useful: it is still a value awaiting its argument,
        // which is exactly Lean's treatment of a strict implicit `⦃α⦄` with nothing following.
        //
        // A MISSING EXPLICIT argument is a different thing and still a hard error: it becomes a
        // `Fill::Missing`, which the engine reports only once the position is known to exist.
        if *kind == ParamKind::Implicit
            && expected.is_none()
            && !kinds[i..].iter().any(|k| *k == ParamKind::Explicit)
        {
            break;
        }
        // A STRICT IMPLICIT waits for an argument that is actually WRITTEN — Lean's `⦃α⦄`, and
        // the one place the fourth binder kind is observable. The eager rule above stops when
        // nothing COULD determine the position; this one stops when nothing has been SUPPLIED,
        // which is stricter and is the point: an expected type is not a reason to instantiate
        // `nilOf`, because `nilOf` unapplied is still the function its type says it is.
        if *kind == ParamKind::StrictImplicit && next_arg >= explicit_args.len() {
            break;
        }
        fills.push(match kind {
            ParamKind::Implicit | ParamKind::StrictImplicit => Fill::Infer,
            ParamKind::Instance => Fill::Instance,
            ParamKind::Explicit => match explicit_args.get(next_arg) {
                Some(a) => {
                    next_arg += 1;
                    Fill::Given(a.clone())
                }
                // NOT WRITTEN — before this is an arity complaint, ask the DECLARATION whether it
                // filled the position itself. `f 1` against `f : Int -> (m : Int := 3) -> Int` is
                // `f 1 3`, at FULL arity: Lean inlines the default at the call site, so the term
                // the kernel sees has no hole in it and nothing downstream needs to know a default
                // was involved.
                None => match declared_default(ctx, head, i) {
                    Some(v) => Fill::Default(v),
                    None => Fill::Missing,
                },
            },
        });
    }

    if next_arg != explicit_args.len() {
        return Err(KernelError::CertificationError(format!(
            "too many explicit arguments: {} provided, {} consumed",
            explicit_args.len(),
            next_arg
        )));
    }

    // TRAILING OBLIGATIONS. `kinds` is sized by what the CALLER WROTE, so a declaration with more
    // parameters than arguments simply stops — which is right for a partial application and wrong
    // for a position the declaration expects to be discharged rather than written (Lean's
    // `(h : P := by tac)`). The remaining telescope positions are therefore walked as OPTIONAL:
    // each is discharged if it can be, and the first one that cannot ENDS the spine, leaving
    // exactly the partial application that was there before.
    if fills.len() == kinds.len() {
        for _ in kinds.len()..count_leading_pis(head_ty) {
            fills.push(Fill::Optional);
        }
    }

    elaborate_spine(ctx, mctx, head, head_ty, &fills, expected)
}

/// DISCHARGE A PROOF OBLIGATION the caller did not write — Lean's auto-param, by the route that
/// needs no new trusted constant.
///
/// `goal` must be a PROPOSITION. The proof is built the way the `decide` tactic builds one:
/// resolve `Decidable goal` from the instance database, COMPUTE `decide goal inst`, and — only if
/// it computes to `true` — hand back `of_decide_eq_true goal inst (refl Bool true)`.
/// `of_decide_eq_true` is a PROVEN prelude theorem (from `Decidable_rec` and Bool no-confusion),
/// so an obligation discharged here costs the trusted base nothing; axiomatising the obligation
/// instead is the one repair that must never be made, since the axiom's own proof is exactly what
/// is missing.
///
/// Three properties this has to have, and each is the reason for one line:
///
/// * it is ABOUT THIS CALL SITE. The goal arrives with the earlier arguments already substituted
///   in, so `safeDiv 10 2` and `safeDiv 9 3` discharge two different propositions and get two
///   different proof terms. Reusing one proof across call sites would prove the wrong thing.
/// * it never guesses. A goal whose `decide` does not compute to `true` — including one that is
///   simply FALSE — yields `None`, and the caller reports the missing argument exactly as before.
/// * the result is KERNEL-CHECKED here, not merely constructed: this is untrusted elaborator code
///   filling a position the writer never saw, which is the last place a term should be taken on
///   trust.
fn discharge_prop(ctx: &Context, mctx: &mut MetaCtx, goal: &Term) -> Option<Term> {
    if has_unsolved_meta(goal) {
        return None;
    }
    // A PROPOSITION, and nothing else. A missing `Nat` is a missing argument.
    if !matches!(
        normalize(ctx, &infer_type(ctx, goal).ok()?),
        Term::Sort(crate::term::Universe::Prop)
    ) {
        return None;
    }
    let dec_goal = Term::App(
        Box::new(Term::Global("Decidable".to_string())),
        Box::new(goal.clone()),
    );
    let mut trial = mctx.clone();
    let inst = resolve_instance(ctx, &mut trial, &dec_goal)?;
    let decided = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("decide".to_string())),
            Box::new(goal.clone()),
        )),
        Box::new(inst.clone()),
    );
    if normalize(ctx, &decided) != Term::Global("true".to_string()) {
        return None;
    }
    let refl_true = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("refl".to_string())),
            Box::new(Term::Global("Bool".to_string())),
        )),
        Box::new(Term::Global("true".to_string())),
    );
    let proof = Term::App(
        Box::new(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("of_decide_eq_true".to_string())),
                Box::new(goal.clone()),
            )),
            Box::new(inst),
        )),
        Box::new(refl_true),
    );
    let got = infer_type(ctx, &proof).ok()?;
    if !crate::is_subtype(ctx, &got, goal) {
        return None;
    }
    *mctx = trial;
    Some(proof)
}

/// The DEFAULT declared for parameter `index` of the global `head` names, if there is one.
///
/// Only a `Term::Global` head can have one: a default is recorded against a DECLARATION, and an
/// anonymous function has no declaration to have written it in.
fn declared_default(ctx: &Context, head: &Term, index: usize) -> Option<Term> {
    let name = head_global(head)?;
    ctx.param_defaults(name)?.get(index)?.1.clone()
}

/// The name the DECLARATION gave parameter `index` of the global `head` names, if it recorded one.
fn declared_param_name(ctx: &Context, head: &Term, index: usize) -> Option<String> {
    let name = head_global(head)?;
    Some(ctx.param_defaults(name)?.get(index)?.0.clone())
}

/// How ONE parameter position of an application is supplied — the instruction set the spine
/// engine runs, and the single place where "what goes here" is decided.
///
/// Positional application, named arguments, the `..` ellipsis and η-expansion are all the same
/// walk down a Π-telescope; they differ only in what each position is told to do. Keeping that
/// difference in a per-position instruction is what stops a second call shape from becoming a
/// second copy of the engine — and a second copy is how two surface forms come to disagree about
/// coercion insertion, or about when instance resolution runs relative to the expected type.
#[derive(Clone, Debug)]
enum Fill {
    /// The caller supplied this argument (positionally or by name).
    Given(Term),
    /// Insert a fresh metavariable — an implicit, or a position the `..` ellipsis says to infer.
    Infer,
    /// Insert a placeholder resolved from the instance database AFTER the explicit arguments and
    /// the expected type have run.
    Instance,
    /// η-EXPAND this position under the given binder name: a LATER parameter was supplied by
    /// name, and a partial application cannot skip a parameter. Lean's answer to
    /// `sub3 (y := 2) (z := 9)` is `fun x => sub3 x 2 9`, a λ — not a shorter spine.
    Abstract(String),
    /// An explicit parameter the caller did not supply, filled from the DECLARATION'S OWN DEFAULT
    /// — Lean's `(m : Int := 3)`. Carried raw and closed over the arguments the earlier positions
    /// actually received, at the point in the walk where those are known.
    Default(Term),
    /// An explicit parameter the caller did not supply. Reported by the engine, so the position
    /// is known to exist before the arity complaint is made.
    Missing,
    /// A trailing parameter BEYOND the ones the caller addressed: discharged if it can be, and
    /// otherwise the end of the spine. Never an error — a partial application is a value.
    Optional,
}

/// THE APPLICATION ENGINE: walk `head_ty`'s Π-telescope one position per [`Fill`], building the
/// spine, then run expected-type propagation and instance resolution in that order.
///
/// η-abstracted positions are wrapped back into λ-binders at the end, so a call that skipped an
/// explicit parameter is a FUNCTION of exactly the parameters it skipped.
fn elaborate_spine(
    ctx: &Context,
    mctx: &mut MetaCtx,
    head: &Term,
    head_ty: &Term,
    fills: &[Fill],
    expected: Option<&Term>,
) -> KernelResult<(Term, Term)> {
    let mut cur_term = head.clone();
    // A universe-polymorphic head's level variables become metavariables the arguments solve —
    // the kernel's own application rule (`type_checker::implicit_levels_for_head`). Left as
    // variables they pin the head to one level: `Type 0 ≤ u` is false for a variable `u` (it may
    // be `Prop`), so `Eq Nat 2 2` and `refl Nat 2` were refused here while the kernel accepts them.
    let mut cur_ty = crate::type_checker::implicit_levels_for_head(head, head_ty.clone());
    let mut levels: std::collections::HashMap<String, crate::Universe> =
        std::collections::HashMap::new();
    // (placeholder metavariable name, the class type to resolve it against).
    let mut obligations: Vec<(String, Term)> = Vec::new();
    // (binder name, domain) for every η-abstracted position, in order.
    let mut abstracted: Vec<(String, Term)> = Vec::new();
    // Every name that occurs in a SUPPLIED argument. An η-binder must avoid all of them or it
    // CAPTURES: `sub3 (y := x) (z := 9)` binding its own `x` would turn the caller's `x` into the
    // λ's, and the result still typechecks — it just computes with the wrong value.
    let mut occupied: Vec<String> = Vec::new();
    for f in fills {
        if let Fill::Given(t) = f {
            collect_var_names(t, &mut occupied);
        }
    }

    // (parameter name, the argument that position received) for every position walked so far —
    // what a DEFAULT at a later position is closed over.
    let mut supplied: Vec<(String, Term)> = Vec::new();

    for fill in fills {
        // A trailing OPTIONAL position stops the walk the moment it cannot be discharged, so a
        // partial application stays the function it is. Decided before the Π is peeled, because
        // running out of Π is also a reason to stop rather than to complain.
        if matches!(fill, Fill::Optional) {
            let Term::Pi { param_type, .. } = resolve_application_pi(ctx, mctx, &cur_ty) else {
                break;
            };
            if discharge_prop(ctx, mctx, &instantiate(&param_type, mctx)).is_none() {
                break;
            }
        }
        // COEFUN — a value used where a function is. Lean routes `g 0` for `g : Fn` through a
        // `CoeFun Fn (fun _ => Nat → Nat)` instance; a structure with exactly ONE function-typed
        // field IS that instance, and the coercion is the projection.
        //
        // It is a real mechanism and not a rewrite: `Named` keeps its other fields and every
        // theorem quantifying over it, because what is inserted is `Named_run n`, not `n`'s
        // function field standing in for `n`.
        let mut current = resolve_application_pi(ctx, mctx, &cur_ty);
        if !matches!(current, Term::Pi { .. }) {
            if let Some(coerced) = coe_fun(ctx, &current, &cur_term) {
                cur_ty = crate::infer_type(ctx, &coerced)?;
                cur_term = coerced;
                current = resolve_application_pi(ctx, mctx, &cur_ty);
            }
        }
        let (param, dom, body) = match current {
            Term::Pi {
                param,
                param_type,
                body_type,
            } => (param, *param_type, *body_type),
            other => {
                return Err(KernelError::NotAFunction(format!(
                    "expected a Π to apply an argument, got {}",
                    other
                )))
            }
        };

        let arg = match fill {
            // AN OBLIGATION THE CALLER DID NOT WRITE. Lean's `(h : P := by tac)` runs a tactic at
            // every call site, about THAT call site's proposition; the position is not an
            // argument the caller forgot, it is one the declaration expects to be discharged.
            // So before the arity complaint, try to PROVE it (see [`discharge_prop`]) — and when
            // there is no proof, report the arity exactly as before.
            Fill::Missing => match discharge_prop(ctx, mctx, &instantiate(&dom, mctx)) {
                Some(proof) => proof,
                None => {
                    return Err(KernelError::CertificationError(
                        "too few explicit arguments".to_string(),
                    ))
                }
            },
            // Already decided above; the discharge is repeated rather than cached because the
            // metavariable context may have moved since, and a proof is about the goal as it
            // stands at the moment it is inserted.
            Fill::Optional => match discharge_prop(ctx, mctx, &instantiate(&dom, mctx)) {
                Some(proof) => proof,
                None => break,
            },
            // A DEFAULT is closed over the arguments the earlier parameters actually received —
            // `sized 5` against `Int -> (m : Int := mul n 2) -> Int` inlines `mul 5 2`, never an
            // open term with a free `n` in it. The substitution primitive is the declaration
            // door's own (`TermParser::substitute_param_ref`), so a default reaching the
            // elaborator through the recorded telescope and one inlined inside the parser cannot
            // come to mean two different things; only the source of the ARGUMENTS differs, and
            // only here are the elaborated ones known.
            //
            // The closed term is then elaborated against the position's declared domain exactly
            // as a written argument is, so a default is checked and coerced by the same rules and
            // cannot smuggle in a term the position would have refused.
            Fill::Default(value) => {
                let mut closed = value.clone();
                for (n, a) in &supplied {
                    if n == "_" {
                        continue;
                    }
                    closed = crate::interface::TermParser::substitute_param_ref(&closed, n, a);
                }
                let (d_elab, d_ty) = elaborate(ctx, mctx, &closed, Some(&dom))?;
                if unify(ctx, mctx, &d_ty, &dom) {
                    d_elab
                } else {
                    let (d_ty_i, dom_i) = (instantiate(&d_ty, mctx), instantiate(&dom, mctx));
                    match coerce_term(ctx, mctx, &d_elab, &d_ty_i, &dom_i) {
                        Some(coerced) => coerced,
                        None => {
                            return Err(KernelError::TypeMismatch {
                                expected: format!("{}", dom_i),
                                found: format!("{}", d_ty_i),
                            })
                        }
                    }
                }
            }
            Fill::Infer => mctx.fresh(),
            Fill::Instance => {
                let m = mctx.fresh();
                if let Term::Var(name) = &m {
                    obligations.push((name.clone(), dom.clone()));
                }
                m
            }
            Fill::Abstract(binder) => {
                let name = fresh_binder(binder, &abstracted, &occupied);
                abstracted.push((name.clone(), dom.clone()));
                Term::Var(name)
            }
            Fill::Given(provided) => {
                let (a_elab, a_ty) = elaborate(ctx, mctx, provided, Some(&dom))?;
                // The level metavariables in this position are solved from the argument, as the
                // kernel's `solve_domain_levels` solves them, and the solution is carried into
                // every later position.
                let dom = if crate::term::mentions_level_meta(&dom) {
                    let actual = match instantiate(&a_ty, mctx) {
                        sort @ Term::Sort(_) => sort,
                        other => normalize(ctx, &other),
                    };
                    crate::term::unify_levels(&dom, &actual, &mut levels);
                    crate::term::instantiate_universes(&dom, &levels)
                } else {
                    dom.clone()
                };
                if unify(ctx, mctx, &a_ty, &dom) {
                    a_elab
                } else {
                    // The argument type does not match — try to bridge it with a registered
                    // COERCION (`↑`). On success wrap the argument in the coercion function;
                    // otherwise it is a genuine type mismatch.
                    let a_ty_i = instantiate(&a_ty, mctx);
                    let dom_i = instantiate(&dom, mctx);
                    match coerce_term(ctx, mctx, &a_elab, &a_ty_i, &dom_i) {
                        Some(coerced) => coerced,
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

        // The name a LATER default is closed over is the one the DECLARATION used, not the `Pi`
        // binder's: `Int -> (m : Int := mul n 2) -> Int` has an anonymous first binder, and the
        // default was written against `n`. The telescope's own name is the fallback for a head
        // with nothing recorded.
        let slot = declared_param_name(ctx, head, supplied.len()).unwrap_or_else(|| param.clone());
        supplied.push((slot, arg.clone()));
        cur_term = Term::App(Box::new(cur_term), Box::new(arg.clone()));
        cur_ty = substitute(&body, &param, &arg);
        if !levels.is_empty() {
            cur_ty = crate::term::instantiate_universes(&cur_ty, &levels);
        }
    }

    // Expected-type propagation: unify the result type with the context's expected type,
    // solving any implicit not already pinned by the explicit arguments.
    //
    // An η-abstracted call is compared as the FUNCTION it will become, not as its body: the
    // expected type of `sub3 (y := 2)` is an `Int → Int`, and unifying that against the body's
    // `Int` would reject the very shape η-expansion exists to produce.
    if let Some(exp) = expected {
        let effective = abstracted
            .iter()
            .rev()
            .fold(cur_ty.clone(), |acc, (n, d)| Term::Pi {
                param: n.clone(),
                param_type: Box::new(d.clone()),
                body_type: Box::new(acc),
            });
        if !unify(ctx, mctx, &effective, exp) && !converts(ctx, mctx, &effective, exp) {
            return Err(KernelError::TypeMismatch {
                expected: format!("{}", instantiate(exp, mctx)),
                found: format!("{}", instantiate(&effective, mctx)),
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

    let mut term = instantiate(&cur_term, mctx);
    let mut ty = instantiate(&cur_ty, mctx);
    for (name, dom) in abstracted.iter().rev() {
        let dom = instantiate(dom, mctx);
        ty = Term::Pi {
            param: name.clone(),
            param_type: Box::new(dom.clone()),
            body_type: Box::new(ty),
        };
        term = Term::Lambda {
            param: name.clone(),
            param_type: Box::new(dom),
            body: Box::new(term),
        };
    }
    Ok((term, ty))
}

/// The kernel's conversion, for an expected-type check the first-order unifier could not decide.
///
/// Lean's `ensureHasType` asks full definitional equality here, and the unifier's
/// normalise-then-compare does not see what conversion sees — a numeral beside the constructor
/// term it denotes, a definition that reduces only under conversion's δ-strategy:
/// `refl Nat 2 : Eq Nat 2 2` against a declared `Eq Nat (myAdd 1 1) 2`. Only a pair with nothing
/// left to solve is asked: a metavariable, a level metavariable or a hole is the unifier's to
/// settle, and conversion cannot settle one. The elaborated term is checked by the kernel
/// afterwards whatever this answers.
fn converts(ctx: &Context, mctx: &MetaCtx, found: &Term, expected: &Term) -> bool {
    let (found, expected) = (instantiate(found, mctx), instantiate(expected, mctx));
    let settled = |t: &Term| {
        !mentions_meta(t)
            && !crate::term::mentions_level_meta(t)
            && !crate::type_checker::mentions_hole(t)
    };
    settled(&found) && settled(&expected) && crate::is_subtype(ctx, &found, &expected)
}

/// Lean's `CoeFun`, read off the declaration instead of off an instance: a structure with exactly
/// ONE function-typed field is applicable, and applying it means projecting that field.
///
/// Returns the PROJECTION APPLIED to the value (parameters first), or `None` when the type is not
/// a structure, has no function-typed field, or has more than one — in which case there is no
/// canonical choice and guessing would silently pick a meaning.
///
/// Why this is not spelled as an ordinary `Context::add_coercion` triple: a `Coe Fn (Nat → Nat)`
/// would make an `Fn` acceptable at every position a `Nat → Nat` is expected, which Lean's
/// `CoeFun` deliberately does not do. This fires in HEAD position only.
fn coe_fun(ctx: &Context, ty: &Term, value: &Term) -> Option<Term> {
    let (head, args) = spine(ty);
    let Term::Global(hname) = &head else {
        return None;
    };
    let info = ctx.struct_info(hname)?;
    let mut functional = info
        .fields
        .iter()
        .enumerate()
        .filter(|(_, (_, t))| matches!(t, Term::Pi { .. }));
    let (index, _) = functional.next()?;
    if functional.next().is_some() {
        return None;
    }
    let proj = info.projections.get(index)?;
    let applied = args.iter().fold(Term::Global(proj.clone()), |f, a| {
        Term::App(Box::new(f), Box::new(a.clone()))
    });
    Some(Term::App(Box::new(applied), Box::new(value.clone())))
}

/// Lean's `CoeSort`, read off the declaration instead of off an instance: a structure with exactly
/// ONE sort-typed field may stand where a TYPE is expected, and standing there means projecting
/// that field.
///
/// This is `coe_fun`'s twin and is written as its twin on purpose — Lean's `CoeFun`
/// (`Init/Coe.lean:262`) and `CoeSort` (`Init/Coe.lean:277`) are one design with two consumers,
/// `coerceToFunction?` and `coerceToSort?` (`Lean/Meta/Coe.lean:56` and `:69`), and giving them
/// two different shapes here is how the second one came to be missing.
///
/// Returns the PROJECTION APPLIED to the value (structure parameters first), or `None` when the
/// value's type is not a structure, has no sort-typed field, or has more than one — in which case
/// there is no canonical choice and guessing would silently pick a meaning.
///
/// ★ WHERE THE LINE IS, and Lean puts it in the same place: this fires during ELABORATION, and
/// the KERNEL never learns to coerce. `ensureType` (`Lean/Elab/Term.lean:1859-1868`) calls
/// `coerceToSort?` before the term reaches the kernel, so a binder whose domain is not a sort is
/// still a type error — it is just not a term the elaborator ever emits.
fn coe_sort(ctx: &Context, value: &Term) -> Option<Term> {
    let ty = infer_type(ctx, value).ok()?;
    // `value` is ALREADY a type when its own type is a sort — nothing to coerce, and in
    // particular no wrapping of a genuine sort in a record that happens to carry one.
    if matches!(ty, Term::Sort(_)) {
        return None;
    }
    let (head, args) = spine(&crate::normalize(ctx, &ty));
    let Term::Global(hname) = &head else {
        return None;
    };
    let info = ctx.struct_info(hname)?;
    let mut carriers = info
        .fields
        .iter()
        .enumerate()
        .filter(|(_, (_, t))| matches!(t, Term::Sort(_)));
    let (index, _) = carriers.next()?;
    if carriers.next().is_some() {
        return None;
    }
    let proj = info.projections.get(index)?;
    let applied = args.iter().fold(Term::Global(proj.clone()), |f, a| {
        Term::App(Box::new(f), Box::new(a.clone()))
    });
    Some(Term::App(Box::new(applied), Box::new(value.clone())))
}

/// `ensureType` — Lean's (`Lean/Elab/Term.lean:1859`). A TYPE POSITION holds a term whose own type
/// is a sort; one that is not gets a `CoeSort` inserted, and one that cannot be coerced is left
/// exactly as written so it fails where it would have failed before.
fn ensure_type(ctx: &Context, t: Term) -> Term {
    if matches!(infer_type(ctx, &t), Ok(Term::Sort(_)) | Err(_)) {
        return t;
    }
    coe_sort(ctx, &t).unwrap_or(t)
}

/// A binder name for an η-abstracted position that collides with neither another binder of the
/// same expansion nor any name occurring in a supplied argument. `_` is not a name a body can
/// refer to, so it is replaced outright.
fn fresh_binder(wanted: &str, taken: &[(String, Term)], occupied: &[String]) -> String {
    let base = if wanted == "_" || wanted.is_empty() {
        "x"
    } else {
        wanted
    };
    let free = |c: &String| !taken.iter().any(|(n, _)| n == c) && !occupied.iter().any(|n| n == c);
    if free(&base.to_string()) {
        return base.to_string();
    }
    (1..).map(|i| format!("{base}{i}")).find(free).unwrap()
}

/// Every variable name occurring in `t` — bound or free. η-expansion avoids all of them, because
/// distinguishing the two here would only buy a prettier binder name at the cost of a capture bug.
fn collect_var_names(t: &Term, acc: &mut Vec<String>) {
    let note = |n: &String, acc: &mut Vec<String>| {
        if !acc.contains(n) {
            acc.push(n.clone());
        }
    };
    match t {
        Term::Var(n) => note(n, acc),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            note(param, acc);
            collect_var_names(param_type, acc);
            collect_var_names(body_type, acc);
        }
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            note(param, acc);
            collect_var_names(param_type, acc);
            collect_var_names(body, acc);
        }
        Term::App(f, a) => {
            collect_var_names(f, acc);
            collect_var_names(a, acc);
        }
        Term::Fix { name, body } => {
            note(name, acc);
            collect_var_names(body, acc);
        }
        Term::MutualFix { defs, .. } => {
            for (n, b) in defs {
                note(n, acc);
                collect_var_names(b, acc);
            }
        }
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            note(name, acc);
            collect_var_names(ty, acc);
            collect_var_names(value, acc);
            collect_var_names(body, acc);
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            collect_var_names(discriminant, acc);
            collect_var_names(motive, acc);
            for c in cases {
                collect_var_names(c, acc);
            }
        }
        Term::Annotated { term, .. } => collect_var_names(term, acc),
        _ => {}
    }
}

// ===============================================================================================
// SURFACE ARGUMENT SUGAR — named arguments (`f (x := 1)`) and the ellipsis (`f ..`, `⟨a, ..⟩`)
// ===============================================================================================
//
// The parser records a REQUEST it cannot act on — only the elaborator knows a declaration's
// parameter names and arity. Both forms land on the same engine as ordinary application: they
// only decide what each parameter position is told to do (see [`Fill`]).

/// Is `t` the parser's ELLIPSIS marker `..`?
fn is_ellipsis_marker(t: &Term) -> bool {
    matches!(t, Term::Global(n) if n == crate::interface::TermParser::ELLIPSIS_MARKER)
}

/// Recognize a NAMED ARGUMENT `(x := v)`, returning the parameter name and the supplied value.
fn as_named_argument(t: &Term) -> Option<(&str, &Term)> {
    crate::interface::TermParser::as_named_arg(t)
}

/// The PARAMETER NAMES of the global `name`, position by position.
///
/// Lean carries parameter names in the declaration's type, because `def f (x : Nat)` binds `x`
/// there. A LOGOS arrow type does not: `Definition sub3 : Int -> Int -> Int -> Int` has three
/// ANONYMOUS binders, and the names the writer used live in the body's λ-telescope instead. So
/// the type is asked first and the body answers for every position the type left as `_` — which
/// is what makes `sub3 (y := 2)` mean anything at all.
fn param_names(ctx: &Context, name: &str, head_ty: &Term) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut cur = head_ty;
    while let Term::Pi {
        param, body_type, ..
    } = cur
    {
        names.push(param.clone());
        cur = body_type;
    }
    if !names.iter().any(|n| n == "_") {
        return names;
    }
    // Fill the anonymous positions from the definition's own λ-telescope, where the writer's
    // names are. A `fix` wrapper (a recursive definition) is transparent for this purpose.
    let Some(mut body) = ctx.get_definition_body(name) else {
        return names;
    };
    while let Term::Fix { body: inner, .. } = body {
        body = inner;
    }
    for slot in names.iter_mut() {
        let Term::Lambda {
            param, body: inner, ..
        } = body
        else {
            break;
        };
        if slot == "_" && param != "_" {
            *slot = param.clone();
        }
        body = inner;
    }
    names
}

/// The parameter KINDS of the global `name`, position by position, over its FULL arity.
///
/// Named arguments and the ellipsis both address positions the caller did not write, so unlike
/// the positional path this cannot stop at "as many explicits as there are arguments".
fn full_param_kinds(ctx: &Context, name: &str, arity: usize) -> Vec<ParamKind> {
    let mut kinds = match ctx.binder_kinds(name) {
        Some(bk) => bk.to_vec(),
        None => {
            let k = ctx.implicit_args(name).min(arity);
            let mut v = vec![ParamKind::Implicit; k];
            v.extend(std::iter::repeat(ParamKind::Explicit).take(arity - k));
            v
        }
    };
    kinds.truncate(arity);
    while kinds.len() < arity {
        kinds.push(ParamKind::Explicit);
    }
    kinds
}

/// Elaborate an application written with NAMED arguments and/or a trailing ELLIPSIS.
///
/// Lean's two rules, and both are observable:
///
/// * a named argument goes to the parameter it NAMES, wherever the writer put it — resolving it
///   positionally is undetectable by the kernel when the parameters share a type, and computes a
///   different number;
/// * a named argument that SKIPS an earlier explicit parameter η-EXPANDS. `sub3 (y := 2) (z := 9)`
///   is `fun x => sub3 x 2 9`, a function of the skipped parameter — not a shorter spine, which
///   would apply `2` where `x` belongs.
///
/// The ellipsis means "infer every position I did not write", so it turns the remaining explicit
/// parameters into inferred ones rather than η-abstracting them.
#[allow(clippy::too_many_arguments)]
fn elaborate_by_name(
    ctx: &Context,
    mctx: &mut MetaCtx,
    name: &str,
    head: &Term,
    head_ty: &Term,
    args: &[&Term],
    ellipsis: bool,
    expected: Option<&Term>,
) -> KernelResult<Term> {
    let names = param_names(ctx, name, head_ty);
    let arity = names.len();
    let kinds = full_param_kinds(ctx, name, arity);
    let mut assigned: Vec<Option<Term>> = vec![None; arity];

    let mut positional: Vec<&Term> = Vec::new();
    for a in args {
        match as_named_argument(a) {
            Some((pname, value)) => {
                let slot = names.iter().position(|n| n == pname).ok_or_else(|| {
                    KernelError::CertificationError(format!(
                        "`{name}` has no parameter named `{pname}` (its parameters are {})",
                        names.join(", ")
                    ))
                })?;
                if assigned[slot].is_some() {
                    return Err(KernelError::CertificationError(format!(
                        "`{name}`'s parameter `{pname}` is supplied twice"
                    )));
                }
                assigned[slot] = Some((*value).clone());
            }
            None => positional.push(a),
        }
    }
    // Positional arguments fill the remaining EXPLICIT positions, in order.
    let mut next = 0usize;
    for a in positional {
        let slot = (next..arity)
            .find(|i| kinds[*i] == ParamKind::Explicit && assigned[*i].is_none())
            .ok_or_else(|| {
                KernelError::CertificationError(format!(
                    "too many arguments for `{name}`: it has {arity} parameter(s)"
                ))
            })?;
        assigned[slot] = Some((*a).clone());
        next = slot + 1;
    }

    // How far down the telescope to go. `..` says "every position". Otherwise stop just past the
    // last position the caller actually addressed — a partially applied function must stay a
    // function, not η-expand into one — and then keep going over trailing IMPLICIT and INSTANCE
    // parameters, which the caller never writes and which the expected type may still pin.
    let last = assigned.iter().rposition(|a| a.is_some());
    let mut limit = if ellipsis {
        arity
    } else {
        last.map_or(0, |i| i + 1)
    };
    while limit < arity && kinds[limit] != ParamKind::Explicit {
        limit += 1;
    }

    // Supplied values go through the SAME preparation a positional argument does, so `f (x := a)`
    // and `f a` cannot come to disagree about how `a` elaborates.
    let mut fills: Vec<Fill> = Vec::with_capacity(limit);
    for i in 0..limit {
        fills.push(match (&assigned[i], kinds[i]) {
            (Some(v), _) => Fill::Given(elab_surface(ctx, mctx, v, None)?),
            (None, ParamKind::Implicit | ParamKind::StrictImplicit) => Fill::Infer,
            (None, ParamKind::Instance) => Fill::Instance,
            // An explicit position the caller skipped: `..` says infer it, a DECLARED DEFAULT
            // fills it, and only with neither does a named argument further right η-abstract it.
            // Lean's order, and it is the only one that makes sense: a parameter the declaration
            // already answered for is not a parameter the caller was asked to skip.
            (None, ParamKind::Explicit) => {
                if ellipsis {
                    Fill::Infer
                } else {
                    match declared_default(ctx, head, i) {
                        Some(v) => Fill::Default(v),
                        None => Fill::Abstract(names[i].clone()),
                    }
                }
            }
        });
    }

    let (t, _) = elaborate_spine(ctx, mctx, head, head_ty, &fills, expected)?;
    // `..` and `_` are the same request, and a hole that survived it is a hole the kernel would
    // match against ANY type. Say so here, naming the parameter, rather than emitting a term
    // whose ill-typedness surfaces somewhere else entirely.
    let t = instantiate(&t, mctx);
    if let Some(unsolved) = first_unsolved_meta(&t) {
        let how = if ellipsis {
            "`..`"
        } else {
            "the arguments given"
        };
        return Err(KernelError::CertificationError(format!(
            "a parameter of `{name}` is left undetermined by {how} ({unsolved}): nothing pins \
             it, so write it"
        )));
    }
    Ok(t)
}

/// How many leading `Π`s a type has — a constructor's arity.
fn count_leading_pis(t: &Term) -> usize {
    let mut n = 0;
    let mut cur = t;
    while let Term::Pi { body_type, .. } = cur {
        n += 1;
        cur = body_type;
    }
    n
}

/// The first still-unsolved metavariable in `t`, if any.
fn first_unsolved_meta(t: &Term) -> Option<String> {
    match t {
        Term::Var(n) if is_meta(n) => Some(n.clone()),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => first_unsolved_meta(param_type).or_else(|| first_unsolved_meta(body_type)),
        Term::Lambda {
            param_type, body, ..
        } => first_unsolved_meta(param_type).or_else(|| first_unsolved_meta(body)),
        Term::App(f, a) => first_unsolved_meta(f).or_else(|| first_unsolved_meta(a)),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => first_unsolved_meta(discriminant)
            .or_else(|| first_unsolved_meta(motive))
            .or_else(|| cases.iter().find_map(first_unsolved_meta)),
        Term::Fix { body, .. } => first_unsolved_meta(body),
        Term::MutualFix { defs, .. } => defs.iter().find_map(|(_, b)| first_unsolved_meta(b)),
        Term::Let {
            ty, value, body, ..
        } => first_unsolved_meta(ty)
            .or_else(|| first_unsolved_meta(value))
            .or_else(|| first_unsolved_meta(body)),
        Term::Annotated { term, .. } => first_unsolved_meta(term),
        _ => None,
    }
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
///
/// A DECLARED class with no registered instance contributes nothing here, which is why the
/// test is `class_heads ∪ registered structures` at every use site: in LOGOS a class IS a
/// structure, so a `[Lonely α]` premise is a premise from the moment `Lonely` is declared —
/// not from the moment someone instantiates it. Reading classhood off the instance vector
/// alone made an uninstantiated class invisible, and its premise was silently demoted to an
/// ordinary type parameter that unifying the conclusion could never determine.
fn class_heads(ctx: &Context) -> std::collections::HashSet<String> {
    ctx.instances()
        .iter()
        .chain(ctx.fallback_instances())
        .filter_map(|(ty, _)| {
            let mut cur = ty;
            while let Term::Pi { body_type, .. } = cur {
                cur = body_type;
            }
            head_global(cur).map(|s| s.to_string())
        })
        .collect()
}

/// Whether the type `t` is an instance PREMISE — its head names a class, either because some
/// instance concludes at that head or because it is a registered structure (a declared class
/// with no instances yet).
fn is_premise(ctx: &Context, heads: &std::collections::HashSet<String>, t: &Term) -> bool {
    head_global(t).is_some_and(|h| heads.contains(h) || ctx.struct_info(h).is_some())
}

/// Whether `t` still carries an unsolved metavariable. The elaborator's contract is that none
/// may reach the kernel, and an instance whose freshened telescope still holds one has not
/// been PROVEN to apply — it has only failed to be contradicted.
fn has_unsolved_meta(t: &Term) -> bool {
    match t {
        Term::Var(n) => is_meta(n),
        Term::App(f, a) => has_unsolved_meta(f) || has_unsolved_meta(a),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => has_unsolved_meta(param_type) || has_unsolved_meta(body_type),
        Term::Lambda {
            param_type, body, ..
        } => has_unsolved_meta(param_type) || has_unsolved_meta(body),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            has_unsolved_meta(discriminant)
                || has_unsolved_meta(motive)
                || cases.iter().any(has_unsolved_meta)
        }
        Term::Fix { body, .. } => has_unsolved_meta(body),
        Term::MutualFix { defs, .. } => defs.iter().any(|(_, b)| has_unsolved_meta(b)),
        Term::Let {
            ty, value, body, ..
        } => has_unsolved_meta(ty) || has_unsolved_meta(value) || has_unsolved_meta(body),
        Term::Annotated { term, .. } => has_unsolved_meta(term),
        Term::Global(_) | Term::Sort(_) | Term::Const { .. } | Term::Lit(_) | Term::Hole => false,
    }
}

/// Unify an instance's CONCLUSION with the goal, leaving the class's OUTPUT positions out of
/// the match (Lean's `outParam`).
///
/// An output is determined BY the instance, so it must not filter candidates: `Decode Bool β`
/// selects on `Bool` alone, and the `β` the instance yields is then checked against whatever
/// the context expected — an ordinary type error at the use site. Letting the output take part
/// turns that error into a DIFFERENT instance, or into "no instance found" for a class that
/// plainly has one.
///
/// The output positions are still unified opportunistically, on a trial context committed only
/// on success, so a goal whose output is an open metavariable (`Decode Bool ?β`) gets it solved
/// from the instance — which is the entire point of an output parameter.
fn unify_conclusion(ctx: &Context, mctx: &mut MetaCtx, conclusion: &Term, goal: &Term) -> bool {
    let (c_head, c_args) = spine(conclusion);
    let (g_head, g_args) = spine(goal);
    let outs = match (&c_head, &g_head) {
        (Term::Global(c), Term::Global(g)) if c == g => ctx.out_params(c),
        _ => &[][..],
    };
    if outs.is_empty() || c_args.len() != g_args.len() {
        return unify(ctx, mctx, conclusion, goal);
    }
    for (i, (c, g)) in c_args.iter().zip(g_args.iter()).enumerate() {
        if outs.contains(&i) {
            continue;
        }
        if !unify(ctx, mctx, c, g) {
            return false;
        }
    }
    for i in outs {
        if let (Some(c), Some(g)) = (c_args.get(*i), g_args.get(*i)) {
            let mut trial = mctx.clone();
            if unify(ctx, &mut trial, c, g) {
                *mctx = trial;
            }
        }
    }
    true
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

/// Which TIER an instance was found in, and where inside it.
///
/// Lean tells you *that* an instance was synthesised; `set_option trace.Meta.synthInstance` tells
/// you a search log you then have to read backwards. This names the winner's coordinates directly,
/// which is what a translator actually needs when two sources disagree about which instance a goal
/// picked up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceTier {
    /// An explicitly declared instance — `Context::instances()[index]`.
    Declared { index: usize },
    /// A library/derived FALLBACK instance — `Context::fallback_instances()[index]`.
    Fallback { index: usize },
}

/// A resolved instance together with WHY it won — the audit trail Lean's instance resolution
/// does not expose.
///
/// `considered` lists, in the exact order they were tried, every candidate the search touched
/// before settling: `(tier, priority, whether it applied)`. A candidate that applied but lost
/// (an ambiguous hierarchy diamond) is therefore distinguishable from one that never matched,
/// which is the distinction that makes an "unexpected instance" bug findable.
#[derive(Debug, Clone)]
pub struct InstanceChoice {
    /// The instance term, with its metavariables instantiated — exactly what
    /// [`resolve_instance`] returns.
    pub value: Term,
    /// Where the winner came from.
    pub tier: InstanceTier,
    /// Its priority (Lean's default, 1000, unless declared otherwise). Fallback entries carry
    /// no priority of their own — the tier itself is the ranking — and report the default.
    pub priority: i64,
    /// Every candidate the search touched, in the order it touched them.
    pub considered: Vec<InstanceProbe>,
    /// How many declared instances the DISCRIMINATION INDEX excluded before any of them was
    /// tried — see [`Context::instance_search_order_for`].
    ///
    /// Lean's `getUnify` hands back the surviving entries and throws this number away, so
    /// "why was my instance never tried?" has no answer there short of reading a trace log
    /// backwards. Here it is a field: `considered.len() + pruned` is the whole database.
    pub pruned: usize,
}

/// One candidate examined during instance resolution.
#[derive(Debug, Clone)]
pub struct InstanceProbe {
    /// Where the candidate lives.
    pub tier: InstanceTier,
    /// Its priority.
    pub priority: i64,
    /// Whether it unified with the goal and discharged all of its premises.
    pub applied: bool,
}

/// [`resolve_instance`] with the search recorded — the same answer, plus which entry won, at
/// what priority and declaration index, and everything that was tried on the way.
///
/// The winner is guaranteed identical to `resolve_instance`'s: both call one search.
pub fn resolve_instance_audited(
    ctx: &Context,
    mctx: &mut MetaCtx,
    required: &Term,
) -> Option<InstanceChoice> {
    let mut audit = Vec::new();
    let mut pruned = 0;
    let found = search_instance(ctx, mctx, required, 0, Some(&mut audit), Some(&mut pruned));
    found.map(|(value, tier, priority)| InstanceChoice {
        value,
        tier,
        priority,
        considered: audit,
        pruned,
    })
}

/// Turn `polyId Nat Zero` — a UNIVERSE-POLYMORPHIC declaration referenced by its bare name, with
/// its levels unwritten — into `polyId.{0} Nat Zero`, or `None` if `term` is not such a reference.
///
/// A Lean file writes a level explicitly almost never; `Context::get_global` deliberately does not
/// see `universe_polys` (only `Term::Const { name, levels }` reaches them), so before this every
/// reference to a polymorphic declaration had to spell its levels out and one that did not was
/// simply an unbound name.
///
/// THE RULE, which is first-order and stated rather than guessed: a binder whose domain is
/// literally `Sort ℓ` for a level PARAMETER `ℓ` is inhabited by a TYPE, and `ℓ` is the least level
/// at or above that type's own sort. So `polyId : Π(A : Sort u). A → A` applied to `Nat : Type 0`
/// solves `u := Type 0`, and applied to a type in `SProp`, which is a sort and never a level, solves
/// `u := Prop`.
///
/// Its limits are deliberate and loud rather than defaulted:
///
/// * a level parameter no argument determines is REFUSED with its name, not defaulted to `Type 0`
///   — a silently defaulted level is a monomorphisation nobody asked for; and
/// * a domain that merely MENTIONS a level parameter (`Sort (u+1)`, `Sort (max u v)`) is not
///   solved from here. That needs level metavariables in the unifier, which is a separate change
///   in the level algebra; this pass never guesses in its place.
fn solve_unwritten_levels(ctx: &Context, term: &Term) -> KernelResult<Option<Term>> {
    let (head, args) = spine(term);
    let Term::Global(name) = &head else {
        return Ok(None);
    };
    // A name that IS an ordinary global keeps its ordinary meaning — one name, one declaration.
    if ctx.get_global(name).is_some() {
        return Ok(None);
    }
    let Some((params, poly_ty, _)) = ctx.get_universe_poly(name) else {
        return Ok(None);
    };
    let (params, poly_ty) = (params.clone(), poly_ty.clone());

    let mut subst: std::collections::HashMap<String, crate::term::Universe> =
        std::collections::HashMap::new();
    let mut cur = &poly_ty;
    for a in &args {
        let Term::Pi {
            param_type,
            body_type,
            ..
        } = cur
        else {
            break;
        };
        if let Term::Sort(crate::term::Universe::Var(p)) = param_type.as_ref() {
            if params.iter().any(|q| q == p) && !subst.contains_key(p) {
                if let Ok(Term::Sort(u)) = infer_type(ctx, a) {
                    subst.insert(p.clone(), u.least_level());
                }
            }
        }
        cur = body_type;
    }

    let mut levels = Vec::with_capacity(params.len());
    for p in &params {
        match subst.get(p) {
            Some(u) => levels.push(u.clone()),
            None => {
                return Err(KernelError::CertificationError(format!(
                    "the level `{p}` of `{name}` is not determined by any argument — write it \
                     explicitly as `{name}.{{…}}`; defaulting it would silently monomorphise the \
                     declaration"
                )))
            }
        }
    }
    let rewritten = args.iter().fold(
        Term::Const {
            name: name.clone(),
            levels,
        },
        |f, a| Term::App(Box::new(f), Box::new(a.clone())),
    );
    Ok(Some(rewritten))
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
    // A trailing `..` says "fill the REMAINING fields", so it is stripped here and honoured
    // after the written fields have been applied.
    let ellipsis = fields.last().is_some_and(is_ellipsis_marker);
    let fields = if ellipsis {
        &fields[..fields.len() - 1]
    } else {
        fields
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
    let mut applied = Term::Global(ctor.clone());
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
                let (fty_i, d_i) = (instantiate(&fty, mctx), instantiate(d, mctx));
                coerce_term(ctx, mctx, &fe, &fty_i, &d_i).unwrap_or(fe)
            }
        } else {
            fe
        };
        applied = Term::App(Box::new(applied), Box::new(arg));
    }
    // `⟨a, ..⟩` — every field the writer did not name becomes a hole to SOLVE, exactly as `_`
    // would. With nothing left to fill this is a no-op, which is what makes a trailing `..` on a
    // complete literal the same term as the literal.
    if ellipsis {
        // Counted off the CONSTRUCTOR'S declared arity rather than by re-inferring the type after
        // each fill: a spine carrying an unsolved metavariable has no inferrable type, so asking
        // would report an unbound `?0` instead of the missing field.
        let total = ctx.get_global(&ctor).map(count_leading_pis).unwrap_or(0);
        for _ in (args.len() + fields.len())..total {
            applied = Term::App(Box::new(applied), Box::new(mctx.fresh()));
        }
    }
    crate::infer_type(ctx, &applied)?;
    let applied = instantiate(&applied, mctx);
    if let Some(unsolved) = first_unsolved_meta(&applied) {
        return Err(KernelError::CertificationError(format!(
            "`⟨…, ..⟩` left a field of `{hname}` undetermined ({unsolved}): nothing in the \
             expected type pins it, so write it"
        )));
    }
    Ok(applied)
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

/// How many `Coe` steps a chain may compose. Lean's `CoeT` head/body/tail decomposition is
/// likewise bounded; the bound is what makes a CYCLIC coercion set (`A ⇒ B`, `B ⇒ A`) report
/// "no path" instead of searching forever.
const MAX_COERCION_STEPS: usize = 8;

/// Find a registered coercion carrying `from` to `to` (up to unification), returning the
/// coercion FUNCTION to wrap the argument in — Lean's `↑`. The elaborator calls this when
/// an argument's type does not match the expected parameter type; a match commits the
/// unification (a polymorphic coercion's type variables get solved).
///
/// A single registered step is returned as ITSELF. A composed chain is returned as the
/// λ-abstraction of the composite, so the function is always something that can be applied to
/// one argument whatever its length; elaboration itself uses `coerce_term`, which applies
/// the steps directly and so leaves no β-redex behind.
pub fn resolve_coercion(ctx: &Context, mctx: &mut MetaCtx, from: &Term, to: &Term) -> Option<Term> {
    let path = coercion_path(ctx, mctx, from, to)?;
    match path.as_slice() {
        [single] => Some(single.clone()),
        steps => {
            let arg = Term::Var("↑x".to_string());
            let body = steps
                .iter()
                .fold(arg, |acc, f| Term::App(Box::new(f.clone()), Box::new(acc)));
            Some(Term::Lambda {
                param: "↑x".to_string(),
                param_type: Box::new(instantiate(from, mctx)),
                body: Box::new(body),
            })
        }
    }
}

/// Wrap `term` (of type `from`) in the coercion path that carries it to `to`, or return `None`
/// if there is none. The steps are applied OUTWARD from the term, so a two-step bridge reads
/// `g (f x)` — the shape Lean's elaborated `↑↑x` has, and one the kernel reduces without a
/// β-step.
fn coerce_term(
    ctx: &Context,
    mctx: &mut MetaCtx,
    term: &Term,
    from: &Term,
    to: &Term,
) -> Option<Term> {
    let path = coercion_path(ctx, mctx, from, to)?;
    Some(
        path.into_iter()
            .fold(term.clone(), |acc, f| Term::App(Box::new(f), Box::new(acc))),
    )
}

/// The shortest chain of registered coercions carrying `from` to `to`, as the functions to
/// apply in order (innermost first).
///
/// Lean composes `Coe` instances transitively — `CoeT` chains a head, a body and a tail — so
/// `Bit → Bool` and `Bool → Nat` bridge a `Bit` where a `Nat` is wanted with no `Bit → Nat`
/// instance declared. This is that closure, as a breadth-first search so the path found is the
/// SHORTEST (hence deterministic where several exist), bounded by [`MAX_COERCION_STEPS`] and
/// guarded by a visited set so a cyclic coercion set terminates.
///
/// Unification is done on a trial context per edge and the winning path's solutions are
/// committed to `mctx` only once the target is reached — a failed branch never pollutes the
/// caller's metavariables.
fn coercion_path(ctx: &Context, mctx: &mut MetaCtx, from: &Term, to: &Term) -> Option<Vec<Term>> {
    if ctx.coercions().is_empty() {
        return None;
    }
    // (type reached, the metavariable state that reached it, the functions applied so far).
    let mut frontier: Vec<(Term, MetaCtx, Vec<Term>)> =
        vec![(instantiate(from, mctx), mctx.clone(), Vec::new())];
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    seen.insert(format!("{}", instantiate(from, mctx)));

    for _ in 0..MAX_COERCION_STEPS {
        let mut next: Vec<(Term, MetaCtx, Vec<Term>)> = Vec::new();
        for (node, state, path) in frontier {
            for (c_from, c_to, c_fn) in ctx.coercions() {
                let mut trial = state.clone();
                if !unify(ctx, &mut trial, c_from, &node) {
                    continue;
                }
                let reached = instantiate(c_to, &trial);
                let mut path = path.clone();
                path.push(instantiate(c_fn, &trial));
                // Arrived? Only the final step has to meet the TARGET type.
                let mut arrived = trial.clone();
                if unify(ctx, &mut arrived, &reached, to) {
                    *mctx = arrived;
                    return Some(path);
                }
                if seen.insert(format!("{reached}")) {
                    next.push((reached, trial, path));
                }
            }
        }
        if next.is_empty() {
            return None;
        }
        frontier = next;
    }
    None
}

/// If `inst_val` is a derived parent-forget edge, the parent class it reaches. A HIERARCHY
/// ROUTE, unlike an ordinary instance, is not something anyone declared for this goal — it is
/// generated from an `extends` relation, so two routes to one parent are a DIAMOND rather than
/// a deliberate overlap, and their dictionaries have to agree.
fn parent_route(ctx: &Context, inst_val: &Term) -> Option<String> {
    ctx.parent_edge_target(head_global(inst_val)?)
        .map(|s| s.to_string())
}

fn resolve_instance_at(
    ctx: &Context,
    mctx: &mut MetaCtx,
    required: &Term,
    depth: usize,
) -> Option<Term> {
    search_instance(ctx, mctx, required, depth, None, None).map(|(value, _, _)| value)
}

/// The ONE instance search. `resolve_instance`, `resolve_instance_at` and
/// `resolve_instance_audited` are all views of it, so no caller can be served by a different
/// ordering rule than another.
///
/// ORDER — Lean's, stated by [`Context::instance_search_order`]: higher priority first, then the
/// LATER declaration. The same order for every goal, whether or not the goal still carries an
/// unsolved metavariable. The previous two-tier scheme (forward for a pinned goal, backward for
/// an open one) had to exist because the forward direction is wrong for an open goal — the
/// earliest entry that unifies INVENTS the class variable from its own carrier — but it is
/// equally wrong for a pinned one, where it inverts every tie-break a Lean source relies on.
/// Once the rule is later-first, the two cases are the same case.
///
/// TIERS — `instances()` first, and only if the whole of it failed, `fallback_instances()`.
/// A fallback is a library or declaration-time entry (`decEq{I}`); it must be findable and must
/// never outrank something a user declared.
fn search_instance(
    ctx: &Context,
    mctx: &mut MetaCtx,
    required: &Term,
    depth: usize,
    mut audit: Option<&mut Vec<InstanceProbe>>,
    pruned_out: Option<&mut usize>,
) -> Option<(Term, InstanceTier, i64)> {
    if depth > MAX_INSTANCE_DEPTH {
        return None;
    }
    let heads = class_heads(ctx);

    // THE INDEX, not a scan. `Context::instance_search_order_for` is a SUBSET of
    // `instance_search_order()` in the same relative order, so this changes what the search pays
    // and never what it answers — and the goal is instantiated first, because a goal still
    // carrying an unsolved metavariable at its head has no key and must see everything.
    let indexed_goal = instantiate(required, mctx);
    let (candidates, pruned) = ctx.instance_search_order_for(&indexed_goal);
    if let Some(slot) = pruned_out {
        *slot = pruned;
    }
    let declared = candidates.into_iter().map(|i| {
        (
            InstanceTier::Declared { index: i },
            ctx.instance_priority(i),
            &ctx.instances()[i],
        )
    });
    // The lower tier keeps the same tie-break — later declaration first — so one rule governs
    // both, and a fallback registered later (a re-derivation) supersedes its own predecessor.
    // Apply the same conservative head discrimination before cloning trial metas or
    // unifying a fallback. This lower tier keeps its original reverse order.
    let fallback = std::iter::once_with(|| ctx.fallback_instance_search_order_for(&indexed_goal))
        .flatten()
        .map(|i| {
            (
                InstanceTier::Fallback { index: i },
                crate::context::DEFAULT_INSTANCE_PRIORITY,
                &ctx.fallback_instances()[i],
            )
        });

    let mut chosen: Option<(MetaCtx, Term, InstanceTier, i64)> = None;
    let mut exhausted_declared = false;
    for (tier, priority, (inst_ty, inst_val)) in declared.chain(fallback) {
        // The fallback tier is a strictly LOWER tier, not a continuation of the scan: an answer
        // from `instances()` is returned before a fallback is even considered.
        if matches!(tier, InstanceTier::Fallback { .. }) && !exhausted_declared {
            exhausted_declared = true;
            if let Some((trial, result, t, p)) = chosen.take() {
                *mctx = trial;
                return Some((result, t, p));
            }
        }
        let mut trial = mctx.clone();
        let result = try_instance(ctx, &mut trial, inst_ty, inst_val, required, &heads, depth);
        if let Some(a) = audit.as_deref_mut() {
            a.push(InstanceProbe {
                tier: tier.clone(),
                priority,
                applied: result.is_some(),
            });
        }
        let Some(result) = result else { continue };
        match &chosen {
            // The first candidate wins unless it is a hierarchy route, in which case the scan
            // continues to see whether a SECOND route reaches the same parent.
            None if parent_route(ctx, inst_val).is_none() => {
                *mctx = trial;
                return Some((result, tier, priority));
            }
            None => chosen = Some((trial, result, tier, priority)),
            Some((_, first, _, _)) => {
                if parent_route(ctx, inst_val).is_some()
                    && !crate::type_checker::def_eq(ctx, first, &result)
                {
                    // An INCOHERENT diamond. Both routes type-check and carry different data,
                    // so picking one silently mistranslates the source: the disagreement
                    // surfaces later as an unrelated proof failing `def_eq` arbitrarily far
                    // from the diamond. No answer is better than an arbitrary one.
                    return None;
                }
            }
        }
    }
    if let Some((trial, result, tier, priority)) = chosen {
        *mctx = trial;
        return Some((result, tier, priority));
    }

    // THE DERIVED TIER — last, and only for a goal nothing declared answers.
    //
    // `Decidable (Eq I a b)` for an inductive `I` is Lean's `deriving DecidableEq`: the decision
    // procedure is DERIVABLE from the constructor list, so requiring a human to have written and
    // registered one is requiring a transcription, not a proof. Deriving it here — at the USE site,
    // from the same `recursor::derive_dec_eq` a declaration would have used — means the instance
    // exists for every inductive without the standard library paying to synthesise constants for
    // types nobody asks about.
    let derived = derived_dec_eq_candidate(ctx, &instantiate(required, mctx))?;
    let mut trial = mctx.clone();
    let result = try_instance(
        ctx, &mut trial, &derived.0, &derived.1, required, &heads, depth,
    )?;
    if let Some(a) = audit.as_deref_mut() {
        a.push(InstanceProbe {
            tier: InstanceTier::Fallback { index: usize::MAX },
            priority: crate::context::DEFAULT_INSTANCE_PRIORITY,
            applied: true,
        });
    }
    *mctx = trial;
    Some((
        result,
        InstanceTier::Fallback { index: usize::MAX },
        crate::context::DEFAULT_INSTANCE_PRIORITY,
    ))
}

/// The `(type, value)` of a DERIVED decidable-equality instance for the goal `required`, if the
/// goal has the shape `Decidable (Eq I a b)` with `I` an inductive.
///
/// Two sources, in this order:
///
/// 1. a `decEq{I}` already in scope — hand-written (the prelude's `decEqNat`/`decEqBool`) or
///    synthesised by a declaration. Preferred, because a human-written procedure is the one whose
///    behaviour other proofs were written against;
/// 2. otherwise DERIVED on the spot, with every auxiliary it depends on INLINED into it so the
///    result is a closed term that needs nothing registered, and then KERNEL-CHECKED before it is
///    handed back. An untrusted derivation that does not check is declined, never returned.
///
/// The returned value is a candidate for `try_instance`, so the parameter telescope is freshened
/// and a `decEq` premise for a type parameter is resolved RECURSIVELY through this same tier —
/// which is how `Decidable (Eq (List Colour) …)` is answered without either being registered.
fn derived_dec_eq_candidate(ctx: &Context, goal: &Term) -> Option<(Term, Term)> {
    // `Decidable p`
    let (head, args) = spine(goal);
    if !matches!(&head, Term::Global(n) if n == "Decidable") || args.len() != 1 {
        return None;
    }
    // `Eq A a b`
    let (eq_head, eq_args) = spine(&args[0]);
    if !matches!(&eq_head, Term::Global(n) if n == "Eq") || eq_args.len() != 3 {
        return None;
    }
    let ind = head_global(&eq_args[0])?.to_string();
    if !ctx.is_inductive(&ind) {
        return None;
    }
    let name = format!("decEq{ind}");
    if let Some(ty) = ctx.get_global(&name) {
        return Some((ty.clone(), Term::Global(name)));
    }
    // Derive the whole group (it comes back in dependency order) and inline each earlier member
    // into the later ones, so `decEq{I}`'s term no longer references constants that were never
    // registered.
    let group = crate::recursor::derive_auxiliaries(ctx, &ind);
    let mut resolved: Vec<(String, Term)> = Vec::with_capacity(group.len());
    let mut answer: Option<(Term, Term)> = None;
    for aux in group {
        let mut term = aux.term;
        let mut ty = aux.ty;
        for (dep_name, dep_term) in &resolved {
            term = inline_global(&term, dep_name, dep_term);
            ty = inline_global(&ty, dep_name, dep_term);
        }
        if aux.name == name {
            answer = Some((ty, term));
            break;
        }
        resolved.push((aux.name, term));
    }
    let (ty, term) = answer?;
    // KERNEL-CHECKED before it is trusted: `derive_*` is untrusted front-end code, and an
    // instance is a term the surrounding proof will be built on.
    let inferred = crate::infer_type(ctx, &term).ok()?;
    if !crate::is_subtype(ctx, &inferred, &ty) {
        return None;
    }
    Some((ty, term))
}

/// Replace every `Global(name)` in `t` by `value`. Used to close a derived term over auxiliaries
/// that were never registered; both sides are closed terms, so no binder can capture.
fn inline_global(t: &Term, name: &str, value: &Term) -> Term {
    match t {
        Term::Global(n) if n == name => value.clone(),
        Term::App(f, a) => Term::App(
            Box::new(inline_global(f, name, value)),
            Box::new(inline_global(a, name, value)),
        ),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Term::Pi {
            param: param.clone(),
            param_type: Box::new(inline_global(param_type, name, value)),
            body_type: Box::new(inline_global(body_type, name, value)),
        },
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(inline_global(param_type, name, value)),
            body: Box::new(inline_global(body, name, value)),
        },
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(inline_global(discriminant, name, value)),
            motive: Box::new(inline_global(motive, name, value)),
            cases: cases
                .iter()
                .map(|c| inline_global(c, name, value))
                .collect(),
        },
        Term::Fix { name: f, body } => Term::Fix {
            name: f.clone(),
            body: Box::new(inline_global(body, name, value)),
        },
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs
                .iter()
                .map(|(n, b)| (n.clone(), inline_global(b, name, value)))
                .collect(),
            index: *index,
        },
        Term::Let {
            name: l,
            ty,
            value: v,
            body,
        } => Term::Let {
            name: l.clone(),
            ty: Box::new(inline_global(ty, name, value)),
            value: Box::new(inline_global(v, name, value)),
            body: Box::new(inline_global(body, name, value)),
        },
        Term::Annotated { annotation, term } => Term::Annotated {
            annotation: annotation.clone(),
            term: Box::new(inline_global(term, name, value)),
        },
        Term::Var(_)
        | Term::Global(_)
        | Term::Sort(_)
        | Term::Lit(_)
        | Term::Hole
        | Term::Const { .. } => t.clone(),
    }
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
            Term::Pi {
                param,
                param_type,
                body_type,
            } => {
                let mv = mctx.fresh();
                if is_premise(ctx, heads, &param_type) {
                    premises.push((mv.clone(), (*param_type).clone()));
                }
                applied = Term::App(Box::new(applied), Box::new(mv.clone()));
                cur = substitute(&body_type, &param, &mv);
            }
            conclusion => {
                // The conclusion must match the goal (this solves the type parameters), with
                // the class's OUTPUT positions left out of the match.
                if !unify_conclusion(ctx, mctx, &conclusion, required) {
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
                // DETERMINACY. Every position the telescope was freshened into must now be
                // solved. One that is not means the instance was never shown to apply — a
                // premise nothing discharged, or a parameter the conclusion never mentions —
                // and committing it would emit a `?`-variable that the kernel later reports as
                // an unbound name, arbitrarily far from the instance that produced it.
                let result = instantiate(&applied, mctx);
                if has_unsolved_meta(&result) {
                    return None;
                }
                return Some(result);
            }
        }
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/elaborate/pi_exposure_controls.rs"]
mod pi_exposure_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/elaborate/fallback_prefilter_controls.rs"]
mod fallback_prefilter_controls;

#[cfg(test)]
thread_local! { static LAZY_UNIFY_FULL_NORMALIZATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/elaborate/lazy_positive_controls.rs"]
mod lazy_positive_controls;

#[path = "elaborate_positive.rs"]
mod positive;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/elaborate/core_pi_exposure_controls.rs"]
mod core_pi_exposure_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/elaborate/compact_data_controls.rs"]
mod compact_data_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/elaborate/scoped_positive_controls.rs"]
mod scoped_positive_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/elaborate/natural_observation_controls.rs"]
mod natural_observation_controls;
