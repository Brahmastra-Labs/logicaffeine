//! R1 — an independent proof-term re-checker (the de Bruijn criterion taken seriously).
//!
//! The kernel's [`infer_type`](crate::infer_type) is the entire trust base. This module
//! is a SECOND, deliberately separate implementation of the CIC type checker: trust then
//! rests on two small independent kernels agreeing, not on one. For the independence to
//! be meaningful the two must not share their checking logic — so this re-checker uses a
//! DIFFERENT term representation (de Bruijn indices, no names) with its own reduction and
//! its own cumulative conversion. A variable-capture or substitution bug in the main
//! kernel's name-based machinery would surface here as a type error rather than slipping
//! through identically — Logos's analog of `lean4checker`/`nanoda`.
//!
//! Scope: the CIC logical core — `Sort`/`Var`/`Global`/`Pi`/`Lambda`/`App`/`Lit` (incl.
//! `BigInt`/`Nat` literal arithmetic and the Nat↔Peano bridge) with β/ι/ζ/δ-reduction and
//! cumulative conversion — PLUS the inductive eliminator `Match` (coverage, per-constructor
//! case typing via de Bruijn telescope instantiation, ι-reduction, large-elimination
//! restriction) AND both single (`Fix`) and MUTUAL (`MutualFix`) fixpoints, each with an
//! INDEPENDENT structural-termination guard. Transparent definitions ARE δ-unfolded during
//! whnf; only axioms/constructors/inductives stay opaque. The only fragment reported
//! HONESTLY as [`ReCheckError::Unsupported`] — rather than silently passed — is `Hole`
//! (an unelaborated metavariable) and a `match` on an inductive this checker cannot
//! resolve: such a proof is single-checked by the main kernel and [`double_check`] says so.
//! The re-checker never reports a term as independently verified unless it actually
//! verified every part of it.

use std::collections::{HashMap, HashSet};

use crate::term::{int_lit, lit_bigint, Literal, Term, Universe};
use crate::Context;

/// Why the re-checker could not produce a verdict, split by soundness meaning.
#[derive(Debug, Clone, PartialEq)]
pub enum ReCheckError {
    /// A construct outside the covered fragment — a `Hole` (unelaborated metavariable) or a
    /// `match` on an inductive whose head this checker does not recognize. NOT a soundness
    /// signal: the term is still fully checked by the main kernel, and [`double_check`]
    /// flags it as single-checked rather than claiming agreement.
    Unsupported(String),
    /// A genuine ill-typedness the re-checker is confident about (a non-function in
    /// application position, a sort/type mismatch, wrong case count, unbound variable).
    /// If the main kernel accepted the same term, the two kernels DISAGREE — an alarm.
    Ill(String),
}

impl ReCheckError {
    fn ill(msg: impl Into<String>) -> Self {
        ReCheckError::Ill(msg.into())
    }
    fn unsupported(msg: impl Into<String>) -> Self {
        ReCheckError::Unsupported(msg.into())
    }
}

type RResult<T> = Result<T, ReCheckError>;

/// A locally-nameless term: every bound variable is a de Bruijn INDEX (0 = innermost
/// binder), so α-equivalence is syntactic identity and there are no names to capture.
/// This is the independence-bearing representation — distinct from the main kernel's
/// named [`Term`].
#[derive(Debug, Clone, PartialEq)]
enum Db {
    Sort(Universe),
    /// de Bruijn index of a `λ`/`Π`-bound variable (0 = nearest enclosing binder).
    Var(usize),
    /// A reference into the global environment (inductive, constructor, definition,
    /// or declaration) — kept opaque.
    Global(String),
    /// A universe-polymorphic global at explicit levels — `name.{ℓ…}`.
    Const { name: String, levels: Vec<Universe> },
    /// `Π(_:dom). body` — `body` is one binder deeper than `dom`.
    Pi(Box<Db>, Box<Db>),
    /// `λ(_:dom). body`.
    Lam(Box<Db>, Box<Db>),
    App(Box<Db>, Box<Db>),
    /// `match disc return motive with cases` — the node binds nothing itself; each case
    /// and the motive are `λ`s that introduce their own binders.
    Match { disc: Box<Db>, motive: Box<Db>, cases: Vec<Db> },
    /// `fix rec. body` — binds the recursive self-reference (index 0 inside `body`).
    Fix(Box<Db>),
    /// `mutualfix { b₀, …, b_{n-1} }.index` — a block of `n` mutually-recursive bodies.
    /// ALL `n` names bind in EVERY body: entering a body pushes `n` levels, with def `j`
    /// at level `base + j` (`base` = enclosing depth). The node reduces to the
    /// `index`-th body once an argument is constructor-headed.
    MutualFix { defs: Vec<Db>, index: usize },
    /// `let _:ty := value in body` — `body` is one binder deeper than `ty`/`value`.
    Let(Box<Db>, Box<Db>, Box<Db>),
    Lit(Literal),
}

// ---------------------------------------------------------------------------
// Named `Term` ⟷ de Bruijn `Db`
// ---------------------------------------------------------------------------

/// Lower a named [`Term`] to a de Bruijn [`Db`], resolving each `Var` to the index of
/// its nearest enclosing binder. `scope` holds the binder names, innermost LAST. A
/// `Var` with no matching binder is a free local — illegal in a closed certificate
/// term, so it is rejected. `Fix`/`Hole` are outside the covered fragment.
fn to_db(term: &Term, scope: &mut Vec<String>) -> RResult<Db> {
    match term {
        Term::Sort(u) => Ok(Db::Sort(u.clone())),
        Term::Var(name) => {
            for (depth_from_inner, bound) in scope.iter().rev().enumerate() {
                if bound == name {
                    return Ok(Db::Var(depth_from_inner));
                }
            }
            Err(ReCheckError::ill(format!("unbound local variable '{}'", name)))
        }
        Term::Global(name) => Ok(Db::Global(name.clone())),
        Term::Const { name, levels } => {
            Ok(Db::Const { name: name.clone(), levels: levels.clone() })
        }
        Term::Pi { param, param_type, body_type } => {
            let dom = to_db(param_type, scope)?;
            scope.push(param.clone());
            let body = to_db(body_type, scope);
            scope.pop();
            Ok(Db::Pi(Box::new(dom), Box::new(body?)))
        }
        Term::Lambda { param, param_type, body } => {
            let dom = to_db(param_type, scope)?;
            scope.push(param.clone());
            let inner = to_db(body, scope);
            scope.pop();
            Ok(Db::Lam(Box::new(dom), Box::new(inner?)))
        }
        Term::App(f, a) => Ok(Db::App(Box::new(to_db(f, scope)?), Box::new(to_db(a, scope)?))),
        Term::Match { discriminant, motive, cases } => {
            let disc = to_db(discriminant, scope)?;
            let mot = to_db(motive, scope)?;
            let cs = cases.iter().map(|c| to_db(c, scope)).collect::<RResult<Vec<_>>>()?;
            Ok(Db::Match { disc: Box::new(disc), motive: Box::new(mot), cases: cs })
        }
        Term::Fix { name, body } => {
            scope.push(name.clone());
            let b = to_db(body, scope);
            scope.pop();
            Ok(Db::Fix(Box::new(b?)))
        }
        // Mutual fixpoint: all `n` names bind in every body, so push all `n` before
        // converting any body (innermost-last, so def `j` sits at level `base + j`).
        Term::MutualFix { defs, index } => {
            let names: Vec<String> = defs.iter().map(|(n, _)| n.clone()).collect();
            for n in &names {
                scope.push(n.clone());
            }
            let mut dbs = Vec::with_capacity(defs.len());
            let mut err = None;
            for (_, body) in defs {
                match to_db(body, scope) {
                    Ok(d) => dbs.push(d),
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                }
            }
            for _ in &names {
                scope.pop();
            }
            match err {
                Some(e) => Err(e),
                None => Ok(Db::MutualFix { defs: dbs, index: *index }),
            }
        }
        Term::Let { name, ty, value, body } => {
            let d_ty = to_db(ty, scope)?;
            let d_val = to_db(value, scope)?;
            scope.push(name.clone());
            let d_body = to_db(body, scope);
            scope.pop();
            Ok(Db::Let(Box::new(d_ty), Box::new(d_val), Box::new(d_body?)))
        }
        Term::Lit(l) => Ok(Db::Lit(l.clone())),
        Term::Hole => Err(ReCheckError::unsupported("Hole (unelaborated implicit)")),
    }
}

/// Raise a de Bruijn [`Db`] back to a named [`Term`], naming binders `v{depth}` by their
/// binding depth (used only to report an inferred type back; comparisons stay in
/// de Bruijn).
fn from_db(t: &Db, depth: usize) -> Term {
    match t {
        Db::Sort(u) => Term::Sort(u.clone()),
        Db::Var(k) => {
            let binder = depth.saturating_sub(1).saturating_sub(*k);
            Term::Var(format!("v{}", binder))
        }
        Db::Global(n) => Term::Global(n.clone()),
        Db::Const { name, levels } => Term::Const { name: name.clone(), levels: levels.clone() },
        Db::Pi(a, b) => Term::Pi {
            param: format!("v{}", depth),
            param_type: Box::new(from_db(a, depth)),
            body_type: Box::new(from_db(b, depth + 1)),
        },
        Db::Lam(a, b) => Term::Lambda {
            param: format!("v{}", depth),
            param_type: Box::new(from_db(a, depth)),
            body: Box::new(from_db(b, depth + 1)),
        },
        Db::App(f, a) => Term::App(Box::new(from_db(f, depth)), Box::new(from_db(a, depth))),
        Db::Match { disc, motive, cases } => Term::Match {
            discriminant: Box::new(from_db(disc, depth)),
            motive: Box::new(from_db(motive, depth)),
            cases: cases.iter().map(|c| from_db(c, depth)).collect(),
        },
        Db::Fix(body) => Term::Fix {
            name: format!("rec{}", depth),
            body: Box::new(from_db(body, depth + 1)),
        },
        Db::MutualFix { defs, index } => {
            let n = defs.len();
            Term::MutualFix {
                defs: defs
                    .iter()
                    .enumerate()
                    .map(|(j, b)| (format!("rec{depth}_{j}"), from_db(b, depth + n)))
                    .collect(),
                index: *index,
            }
        }
        Db::Let(ty, value, body) => Term::Let {
            name: format!("v{}", depth),
            ty: Box::new(from_db(ty, depth)),
            value: Box::new(from_db(value, depth)),
            body: Box::new(from_db(body, depth + 1)),
        },
        Db::Lit(l) => Term::Lit(l.clone()),
    }
}

// ---------------------------------------------------------------------------
// de Bruijn shifting and substitution (the capture-free core)
// ---------------------------------------------------------------------------

/// Shift every free index `≥ cutoff` by `d` (which may be negative). Binders raise the
/// cutoff so bound indices are untouched (the classic TAPL shift).
fn shift(t: &Db, d: isize, cutoff: usize) -> Db {
    match t {
        Db::Var(k) => {
            if *k >= cutoff {
                Db::Var((*k as isize + d) as usize)
            } else {
                Db::Var(*k)
            }
        }
        Db::Pi(a, b) => Db::Pi(Box::new(shift(a, d, cutoff)), Box::new(shift(b, d, cutoff + 1))),
        Db::Lam(a, b) => Db::Lam(Box::new(shift(a, d, cutoff)), Box::new(shift(b, d, cutoff + 1))),
        Db::App(f, a) => Db::App(Box::new(shift(f, d, cutoff)), Box::new(shift(a, d, cutoff))),
        Db::Match { disc, motive, cases } => Db::Match {
            disc: Box::new(shift(disc, d, cutoff)),
            motive: Box::new(shift(motive, d, cutoff)),
            cases: cases.iter().map(|c| shift(c, d, cutoff)).collect(),
        },
        Db::Fix(body) => Db::Fix(Box::new(shift(body, d, cutoff + 1))),
        Db::MutualFix { defs, index } => Db::MutualFix {
            // Every body lives beneath ALL `n` mutual binders, so the cutoff rises by `n`.
            defs: defs.iter().map(|b| shift(b, d, cutoff + defs.len())).collect(),
            index: *index,
        },
        Db::Let(ty, value, body) => Db::Let(
            Box::new(shift(ty, d, cutoff)),
            Box::new(shift(value, d, cutoff)),
            Box::new(shift(body, d, cutoff + 1)),
        ),
        Db::Sort(_) | Db::Global(_) | Db::Const { .. } | Db::Lit(_) => t.clone(),
    }
}

/// Substitute the variable with index `j` by `s` (capture-free: `s` is shifted as it
/// descends under binders, `j` rises in step).
fn subst(t: &Db, j: usize, s: &Db) -> Db {
    match t {
        Db::Var(k) => {
            if *k == j {
                s.clone()
            } else {
                Db::Var(*k)
            }
        }
        Db::Pi(a, b) => Db::Pi(
            Box::new(subst(a, j, s)),
            Box::new(subst(b, j + 1, &shift(s, 1, 0))),
        ),
        Db::Lam(a, b) => Db::Lam(
            Box::new(subst(a, j, s)),
            Box::new(subst(b, j + 1, &shift(s, 1, 0))),
        ),
        Db::App(f, a) => Db::App(Box::new(subst(f, j, s)), Box::new(subst(a, j, s))),
        Db::Match { disc, motive, cases } => Db::Match {
            disc: Box::new(subst(disc, j, s)),
            motive: Box::new(subst(motive, j, s)),
            cases: cases.iter().map(|c| subst(c, j, s)).collect(),
        },
        Db::Fix(body) => Db::Fix(Box::new(subst(body, j + 1, &shift(s, 1, 0)))),
        Db::MutualFix { defs, index } => {
            // Under `n` mutual binders: the substituted index rises by `n` and `s` is
            // shifted by `n` to keep its free variables aligned.
            let n = defs.len();
            Db::MutualFix {
                defs: defs.iter().map(|b| subst(b, j + n, &shift(s, n as isize, 0))).collect(),
                index: *index,
            }
        }
        Db::Let(ty, value, body) => Db::Let(
            Box::new(subst(ty, j, s)),
            Box::new(subst(value, j, s)),
            Box::new(subst(body, j + 1, &shift(s, 1, 0))),
        ),
        Db::Sort(_) | Db::Global(_) | Db::Const { .. } | Db::Lit(_) => t.clone(),
    }
}

/// β-substitution of the argument into a binder body: `(λ. body) arg ⤳ body[0 := arg]`.
fn beta_open(body: &Db, arg: &Db) -> Db {
    shift(&subst(body, 0, &shift(arg, 1, 0)), -1, 0)
}

/// Unfold the `index`-th body of a mutual block: replace each of the `n` mutual names by
/// its own (closed) `MutualFix` projection and drop the `n` binders. The mutual analog of
/// `beta_open` — the innermost binder (index 0) is name `n-1`, so it receives projection
/// `n-1`, down to name `0`. Each projection is closed, so iterated `beta_open` is exact.
fn open_mutual(defs: &[Db], index: usize) -> Db {
    let n = defs.len();
    let mut result = defs[index].clone();
    for k in 0..n {
        let proj = Db::MutualFix { defs: defs.to_vec(), index: n - 1 - k };
        result = beta_open(&result, &proj);
    }
    result
}

/// Decompose an application spine into its head and arguments (in source order).
fn spine(t: &Db) -> (Db, Vec<Db>) {
    let mut args = Vec::new();
    let mut cur = t.clone();
    while let Db::App(f, a) = cur {
        args.push(*a);
        cur = *f;
    }
    args.reverse();
    (cur, args)
}

/// Whether `t` reduces to an application headed by a constructor — the guard that lets a
/// fixpoint unfold (its decreasing argument is a value) without risking a loop.
fn ctor_headed(genv: &Context, t: &Db) -> bool {
    let (head, _) = spine(&whnf(genv, t));
    matches!(head, Db::Global(n) if genv.is_constructor(&n))
}

/// Reduce a primitive Int/Bool operation applied to two literal arguments — the main
/// kernel's hardware-ALU builtins, replicated here so the re-checker computes the same
/// arithmetic (`le 2 3 ⤳ true`, `add 2 4 ⤳ 6`). The arguments are themselves reduced
/// first (so `le (add 1 1) 3` works). `None` if `t` is not such a primitive application.
fn try_builtin(genv: &Context, t: &Db) -> Option<Db> {
    let (head, args) = spine(t);
    let op = match &head {
        Db::Global(n) if args.len() == 2 => n.as_str(),
        _ => return None,
    };
    if !matches!(op, "add" | "sub" | "mul" | "div" | "mod" | "le" | "lt" | "ge" | "gt") {
        return None;
    }
    let (xl, yl) = match (whnf(genv, &args[0]), whnf(genv, &args[1])) {
        (Db::Lit(xl), Db::Lit(yl)) => (xl, yl),
        _ => return None,
    };
    let bool_op = |b: bool| Some(Db::Global(if b { "true" } else { "false" }.to_string()));

    // Fast i64 path; an overflowing arithmetic op falls through to exact BigInt (K6).
    if let (Literal::Int(x), Literal::Int(y)) = (&xl, &yl) {
        let fast = match op {
            "add" => x.checked_add(*y),
            "sub" => x.checked_sub(*y),
            "mul" => x.checked_mul(*y),
            "div" => x.checked_div(*y),
            "mod" => x.checked_rem(*y),
            _ => None,
        };
        if let Some(r) = fast {
            return Some(Db::Lit(Literal::Int(r)));
        }
        match op {
            "le" => return bool_op(*x <= *y),
            "lt" => return bool_op(*x < *y),
            "ge" => return bool_op(*x >= *y),
            "gt" => return bool_op(*x > *y),
            _ => {}
        }
    }

    // Exact arbitrary-precision path — the independent kernel's copy of the main kernel's
    // BigInt arithmetic (`try_primitive_reduce`), canonicalised by `int_lit`.
    let (xb, yb) = (lit_bigint(&xl)?, lit_bigint(&yl)?);
    let big = match op {
        "add" => Some(xb.add(&yb)),
        "sub" => Some(xb.sub(&yb)),
        "mul" => Some(xb.mul(&yb)),
        "div" => xb.div_rem(&yb).map(|(q, _)| q),
        "mod" => xb.div_rem(&yb).map(|(_, r)| r),
        _ => None,
    };
    if let Some(r) = big {
        return Some(Db::Lit(int_lit(r)));
    }
    match op {
        "le" => bool_op(xb <= yb),
        "lt" => bool_op(xb < yb),
        "ge" => bool_op(xb >= yb),
        "gt" => bool_op(xb > yb),
        _ => None,
    }
}

/// Count the leading `Π` binders of a de Bruijn type (an inductive's arity / a
/// constructor's parameter count).
fn pi_count(t: &Db) -> usize {
    match t {
        Db::Pi(_, b) => 1 + pi_count(b),
        _ => 0,
    }
}

/// The number of leading parameters (arity) of an inductive — how many of a
/// constructor's leading arguments are type parameters fixed by the discriminant.
fn inductive_arity(genv: &Context, ind: &str) -> usize {
    match genv.get_global(ind) {
        Some(ty) => to_db(&ty.clone(), &mut Vec::new()).map(|d| pi_count(&d)).unwrap_or(0),
        None => 0,
    }
}

/// Weak head normal form under β and ι (globals stay opaque — no δ). `ι`: a `match`
/// whose discriminant reduces to a constructor application selects and applies the
/// corresponding case to the constructor's value arguments. Terminates on the supported
/// (non-`Fix`) fragment.
/// The quotient computation rule in de Bruijn form: `Quot_lift A r B f h (Quot_mk A r a) ⤳
/// f a`. Returns the reduct when `t` is that spine, else `None`.
fn try_quot_lift_db(genv: &Context, t: &Db) -> Option<Db> {
    let (head, args) = spine(t);
    if !matches!(&head, Db::Global(n) if n == "Quot_lift") || args.len() != 6 {
        return None;
    }
    let q = whnf(genv, &args[5]);
    let (qh, qa) = spine(&q);
    if !matches!(&qh, Db::Global(n) if n == "Quot_mk") || qa.len() != 3 {
        return None;
    }
    Some(Db::App(Box::new(args[3].clone()), Box::new(qa[2].clone())))
}

fn whnf(genv: &Context, t: &Db) -> Db {
    let mut cur = t.clone();
    // Strong normalization guarantees termination on well-typed terms; the fuel is a
    // backstop so a (hypothetical) ill-typed input can never hang the re-checker.
    let mut fuel: usize = 1_000_000;
    loop {
        if fuel == 0 {
            return cur;
        }
        fuel -= 1;
        // Quotient computation: `Quot_lift A r B f h (Quot_mk A r a) ⤳ f a` (mirrors the
        // main kernel, so quotient proofs are two-kernel verified).
        if matches!(cur, Db::App(..)) {
            if let Some(reduced) = try_quot_lift_db(genv, &cur) {
                cur = reduced;
                continue;
            }
        }
        // Native-reduction hook: the main kernel reduces `reduceBool t` with its
        // trusted evaluator; the re-checker reduces `t` with its OWN machinery
        // (δ/ι/builtins) — an independent computation of the same value, so a
        // `native_decide` proof is genuinely two-kernel verified.
        if let Db::App(f, a) = &cur {
            if matches!(f.as_ref(), Db::Global(n) if n == "reduceBool") {
                let av = whnf(genv, a);
                if matches!(&av, Db::Global(n) if n == "true" || n == "false") {
                    cur = av;
                    continue;
                }
            }
        }
        match cur {
            // δ: unfold a transparent definition (a global with a body) — so the
            // re-checker COMPUTES `le 2 3 ⇝ true` etc., independently of the main kernel.
            // Axioms/declarations (no body) stay opaque.
            Db::Global(name) => {
                if let Some(body) = genv.get_definition_body(&name) {
                    if let Ok(db) = to_db(&body.clone(), &mut Vec::new()) {
                        cur = db;
                        continue;
                    }
                }
                return Db::Global(name);
            }
            Db::App(f, a) => {
                let fw = whnf(genv, &f);
                match fw {
                    Db::Lam(_, body) => {
                        cur = beta_open(&body, &a);
                    }
                    // Guarded fix-unfolding: `(fix rec. body) args ⤳ body[rec := fix] args`,
                    // but ONLY when some argument is constructor-headed, so the unfolded
                    // `match` makes progress and a stuck fixpoint cannot loop.
                    Db::Fix(fbody) => {
                        let (_, args) = spine(&Db::App(Box::new(Db::Fix(fbody.clone())), a.clone()));
                        if args.iter().any(|x| ctor_headed(genv, x)) {
                            let unfolded = beta_open(&fbody, &Db::Fix(fbody.clone()));
                            cur = Db::App(Box::new(unfolded), a);
                        } else {
                            return Db::App(Box::new(Db::Fix(fbody)), a);
                        }
                    }
                    // Mutual-fix unfolding: same guarded rule, but unfold the `index`-th
                    // body, substituting every sibling by its own projection.
                    Db::MutualFix { defs, index } => {
                        let applied =
                            Db::App(Box::new(Db::MutualFix { defs: defs.clone(), index }), a.clone());
                        let (_, args) = spine(&applied);
                        if args.iter().any(|x| ctor_headed(genv, x)) {
                            cur = Db::App(Box::new(open_mutual(&defs, index)), a);
                        } else {
                            return Db::App(Box::new(Db::MutualFix { defs, index }), a);
                        }
                    }
                    other => {
                        // A primitive Int/Bool operation on literal arguments
                        // (`le 2 3 ⤳ true`, `add 2 4 ⤳ 6`) — the re-checker independently
                        // replicates the main kernel's ALU builtins so it can verify
                        // arithmetic certificates.
                        let full = Db::App(Box::new(other), a);
                        if let Some(r) = try_builtin(genv, &full) {
                            cur = r;
                        } else {
                            return full;
                        }
                    }
                }
            }
            Db::Match { disc, motive, cases } => {
                let mut d = whnf(genv, &disc);
                // Peano bridge (K6): a `Nat(n)` literal is `Zero`/`Succ(Nat(n-1))` — expand
                // one step so the constructor-selection below fires (the recursor computes
                // on Nat literals, peeling one `Succ` per match).
                if matches!(&d, Db::Lit(Literal::Nat(_))) {
                    d = db_nat_peano_step(&d);
                }
                let (head, args) = spine(&d);
                if let Db::Global(cname) = &head {
                    if let Some(ind) = genv.constructor_inductive(cname) {
                        let ctor_names: Vec<String> =
                            genv.get_constructors(ind).iter().map(|(n, _)| n.to_string()).collect();
                        if let Some(idx) = ctor_names.iter().position(|n| n == cname) {
                            if idx < cases.len() {
                                let arity = inductive_arity(genv, ind);
                                let val_args =
                                    if args.len() >= arity { &args[arity..] } else { &args[..] };
                                let mut res = cases[idx].clone();
                                for a in val_args {
                                    res = Db::App(Box::new(res), Box::new(a.clone()));
                                }
                                cur = res;
                                continue;
                            }
                        }
                    }
                }
                return Db::Match { disc: Box::new(d), motive, cases };
            }
            // Zeta: `let _:_ := v in b` ⤳ `b[0 := v]`.
            Db::Let(_ty, value, body) => {
                cur = beta_open(&body, &value);
            }
            other => return other,
        }
    }
}

// ---------------------------------------------------------------------------
// Conversion (definitional equality + cumulativity)
// ---------------------------------------------------------------------------

/// One Peano-unfolding step of a de Bruijn `Nat` literal: `Nat(0) → Zero`,
/// `Nat(n) → Succ (Nat(n-1))` (K6). Non-Nat terms are returned unchanged.
fn db_nat_peano_step(t: &Db) -> Db {
    match t {
        // `n ≤ 0` collapses to `Zero` so the peel TERMINATES on any (incl. malformed
        // negative) literal — critical here, where the re-checker consumes untrusted input.
        Db::Lit(Literal::Nat(n)) if *n <= logicaffeine_base::BigInt::from_i64(0) => {
            Db::Global("Zero".to_string())
        }
        Db::Lit(Literal::Nat(n)) => Db::App(
            Box::new(Db::Global("Succ".to_string())),
            Box::new(Db::Lit(Literal::Nat(n.sub(&logicaffeine_base::BigInt::from_i64(1))))),
        ),
        other => other.clone(),
    }
}

/// True if `t` heads a `Nat` Peano value — `Zero` or `Succ _` — the shape a `Nat` literal
/// bridges against.
fn db_nat_peano_headed(t: &Db) -> bool {
    match t {
        Db::Global(n) => n == "Zero",
        Db::App(f, _) => matches!(f.as_ref(), Db::Global(n) if n == "Succ"),
        _ => false,
    }
}

/// Definitional equality: β/ι-convertible up to α (syntactic in de Bruijn). Globals are
/// compared by name (opaque — no δ).
fn def_eq(genv: &Context, lctx: &[Db], a: &Db, b: &Db) -> bool {
    let a = whnf(genv, a);
    let b = whnf(genv, b);

    // Peano bridge (K6): a `Nat(n)` literal is definitionally `Succ^n Zero`. Two Nat
    // literals are equal iff their counts are; a Nat literal against a `Zero`/`Succ`-headed
    // term is compared by peeling one `Succ` (terminating at `Nat(0) ≡ Zero`).
    match (&a, &b) {
        (Db::Lit(Literal::Nat(x)), Db::Lit(Literal::Nat(y))) => return x == y,
        (Db::Lit(Literal::Nat(_)), _) if db_nat_peano_headed(&b) => {
            return def_eq(genv, lctx, &db_nat_peano_step(&a), &b);
        }
        (_, Db::Lit(Literal::Nat(_))) if db_nat_peano_headed(&a) => {
            return def_eq(genv, lctx, &a, &db_nat_peano_step(&b));
        }
        _ => {}
    }

    // η-conversion: `λ. f (Var 0) ≡ g`. When exactly one side is a `λ`, compare its body
    // against the other side (shifted under the binder) applied to `Var 0`.
    if let Db::Lam(dom, body) = &a {
        if !matches!(b, Db::Lam(..)) {
            let bx = whnf(genv, &Db::App(Box::new(shift(&b, 1, 0)), Box::new(Db::Var(0))));
            let mut ext = lctx.to_vec();
            ext.push((**dom).clone());
            return def_eq(genv, &ext, body, &bx);
        }
    }
    if let Db::Lam(dom, body) = &b {
        if !matches!(a, Db::Lam(..)) {
            let ax = whnf(genv, &Db::App(Box::new(shift(&a, 1, 0)), Box::new(Db::Var(0))));
            let mut ext = lctx.to_vec();
            ext.push((**dom).clone());
            return def_eq(genv, &ext, &ax, body);
        }
    }

    // Structure η (the second kernel's copy of the main kernel's rule): a
    // fully-applied structure constructor is convertible with any term whose
    // projections match it field-wise.
    if let Some(eq) = try_struct_eta_db(genv, lctx, &a, &b) {
        return eq;
    }
    if let Some(eq) = try_struct_eta_db(genv, lctx, &b, &a) {
        return eq;
    }

    let congruent = match (&a, &b) {
        (Db::Sort(x), Db::Sort(y)) => x.equiv(y),
        (Db::Var(i), Db::Var(j)) => i == j,
        (Db::Global(m), Db::Global(n)) => m == n,
        (Db::Lit(x), Db::Lit(y)) => x == y,
        (Db::App(f1, a1), Db::App(f2, a2)) => {
            def_eq(genv, lctx, f1, f2) && def_eq(genv, lctx, a1, a2)
        }
        (Db::Pi(d1, b1), Db::Pi(d2, b2)) => {
            def_eq(genv, lctx, d1, d2) && {
                let mut ext = lctx.to_vec();
                ext.push((**d1).clone());
                def_eq(genv, &ext, b1, b2)
            }
        }
        (Db::Lam(d1, b1), Db::Lam(d2, b2)) => {
            def_eq(genv, lctx, d1, d2) && {
                let mut ext = lctx.to_vec();
                ext.push((**d1).clone());
                def_eq(genv, &ext, b1, b2)
            }
        }
        (
            Db::Match { disc: d1, motive: m1, cases: c1 },
            Db::Match { disc: d2, motive: m2, cases: c2 },
        ) => {
            def_eq(genv, lctx, d1, d2)
                && def_eq(genv, lctx, m1, m2)
                && c1.len() == c2.len()
                && c1.iter().zip(c2.iter()).all(|(x, y)| def_eq(genv, lctx, x, y))
        }
        (Db::Fix(b1), Db::Fix(b2)) => {
            let mut ext = lctx.to_vec();
            ext.push(Db::Sort(Universe::Prop)); // `rec` placeholder (its type is never consulted)
            def_eq(genv, &ext, b1, b2)
        }
        (Db::MutualFix { defs: d1, index: i1 }, Db::MutualFix { defs: d2, index: i2 }) => {
            i1 == i2
                && d1.len() == d2.len()
                && {
                    let mut ext = lctx.to_vec();
                    for _ in 0..d1.len() {
                        ext.push(Db::Sort(Universe::Prop)); // the `n` recursive-name placeholders
                    }
                    d1.iter().zip(d2.iter()).all(|(a, b)| def_eq(genv, &ext, a, b))
                }
        }
        (Db::Const { name: n1, levels: l1 }, Db::Const { name: n2, levels: l2 }) => {
            n1 == n2 && l1.len() == l2.len() && l1.iter().zip(l2.iter()).all(|(a, b)| a.equiv(b))
        }
        _ => false,
    };
    if congruent {
        return true;
    }

    // Proof irrelevance: two proofs of the same proposition are equal.
    proof_irrel(genv, lctx, &a, &b)
}

/// Structure η in the de Bruijn re-checker, ONE direction (see the main kernel's
/// `try_structure_eta`). `None` when `mk_term` is not a fully-applied registered
/// structure constructor, or `other` is the same constructor (ordinary congruence).
fn try_struct_eta_db(genv: &Context, lctx: &[Db], mk_term: &Db, other: &Db) -> Option<bool> {
    let (head, args) = spine(mk_term);
    let Db::Global(hname) = &head else { return None };
    let (_sname, info) = genv.struct_of_constructor(hname)?;
    let nfields = info.projections.len();
    if args.len() != info.num_params + nfields {
        return None;
    }
    let (ohead, _) = spine(other);
    if matches!(&ohead, Db::Global(n) if n == hname) {
        return None;
    }
    let params = &args[..info.num_params];
    let field_args = &args[info.num_params..];
    Some(info.projections.iter().enumerate().all(|(i, proj)| {
        // `proj params… other` as a Db application.
        let mut app = Db::Global(proj.clone());
        for p in params {
            app = Db::App(Box::new(app), Box::new(p.clone()));
        }
        app = Db::App(Box::new(app), Box::new(other.clone()));
        def_eq(genv, lctx, &field_args[i], &app)
    }))
}

/// Proof irrelevance for the re-checker: `a ≡ b` if `a : A`, `A : Prop`, and `b`'s type is
/// definitionally equal to `A`. Mirrors the main kernel's [`crate::type_checker::def_eq`].
fn proof_irrel(genv: &Context, lctx: &[Db], a: &Db, b: &Db) -> bool {
    let ta = match infer(genv, lctx, a) {
        Ok(t) => t,
        Err(_) => return false,
    };
    match infer(genv, lctx, &ta) {
        Ok(s) if matches!(
            whnf(genv, &s),
            Db::Sort(Universe::Prop) | Db::Sort(Universe::SProp)
        ) => {}
        _ => return false,
    }
    match infer(genv, lctx, b) {
        Ok(tb) => def_eq(genv, lctx, &ta, &tb),
        Err(_) => false,
    }
}

/// Cumulative subtyping `sub ≤ sup`: sorts follow `Prop ≤ Type i ≤ Type j`; a `Π` is
/// covariant in its codomain and invariant in its domain; everything else is `def_eq`.
fn is_sub(genv: &Context, lctx: &[Db], sub: &Db, sup: &Db) -> bool {
    let s = whnf(genv, sub);
    let t = whnf(genv, sup);
    match (&s, &t) {
        (Db::Sort(x), Db::Sort(y)) => x.is_subtype_of(y),
        (Db::Pi(d1, b1), Db::Pi(d2, b2)) => {
            def_eq(genv, lctx, d1, d2) && {
                let mut ext = lctx.to_vec();
                ext.push((**d1).clone());
                is_sub(genv, &ext, b1, b2)
            }
        }
        _ => def_eq(genv, lctx, &s, &t),
    }
}

// ---------------------------------------------------------------------------
// Inference
// ---------------------------------------------------------------------------

/// The type of de Bruijn variable `k`, read from `lctx` (binder types, innermost LAST,
/// each stored in the scope of ITS binding point) and lifted by `k+1` to current depth.
fn var_type(lctx: &[Db], k: usize) -> RResult<Db> {
    if k >= lctx.len() {
        return Err(ReCheckError::ill(format!("de Bruijn index {} out of range", k)));
    }
    let stored = &lctx[lctx.len() - 1 - k];
    Ok(shift(stored, (k + 1) as isize, 0))
}

/// Infer the type of `t` under global env `genv` and local context `lctx`.
fn infer(genv: &Context, lctx: &[Db], t: &Db) -> RResult<Db> {
    match t {
        Db::Sort(u) => Ok(Db::Sort(u.succ())),
        Db::Var(k) => var_type(lctx, *k),
        Db::Global(name) => {
            let ty = genv
                .get_global(name)
                .ok_or_else(|| ReCheckError::ill(format!("unknown global '{}'", name)))?;
            to_db(&ty.clone(), &mut Vec::new())
        }
        Db::Const { name, levels } => {
            let (params, ty, _body) = genv
                .get_universe_poly(name)
                .ok_or_else(|| ReCheckError::ill(format!("unknown universe-poly global '{}'", name)))?;
            if params.len() != levels.len() {
                return Err(ReCheckError::ill(format!(
                    "universe-poly '{}' expects {} levels, got {}",
                    name,
                    params.len(),
                    levels.len()
                )));
            }
            let subst: std::collections::HashMap<String, Universe> =
                params.iter().cloned().zip(levels.iter().cloned()).collect();
            let instantiated = crate::term::instantiate_universes(&ty.clone(), &subst);
            to_db(&instantiated, &mut Vec::new())
        }
        Db::Lit(Literal::Nat(n)) if *n < logicaffeine_base::BigInt::from_i64(0) => {
            Err(ReCheckError::ill("a `Nat` literal must be non-negative".to_string()))
        }
        Db::Lit(l) => Ok(Db::Global(lit_type_name(l).to_string())),
        Db::Pi(dom, body) => {
            let dom_sort = infer_sort(genv, lctx, dom)?;
            let mut ext = lctx.to_vec();
            ext.push((**dom).clone());
            let body_sort = infer_sort(genv, &ext, body)?;
            // Impredicative product formation via `imax` (see the main kernel's
            // `Pi` arm) — the second kernel decides the same level algebra.
            let pi_sort = dom_sort.imax(&body_sort);
            Ok(Db::Sort(pi_sort))
        }
        Db::Lam(dom, body) => {
            let _ = infer_sort(genv, lctx, dom)?;
            let mut ext = lctx.to_vec();
            ext.push((**dom).clone());
            let body_ty = infer(genv, &ext, body)?;
            Ok(Db::Pi(dom.clone(), Box::new(body_ty)))
        }
        Db::App(f, a) => {
            let f_ty = whnf(genv, &infer(genv, lctx, f)?);
            match f_ty {
                Db::Pi(dom, body) => {
                    check(genv, lctx, a, &dom)?;
                    Ok(beta_open(&body, a))
                }
                other => Err(ReCheckError::ill(format!(
                    "application of a non-function (type {})",
                    from_db(&other, lctx.len())
                ))),
            }
        }
        Db::Match { disc, motive, cases } => infer_match(genv, lctx, disc, motive, cases),
        Db::Fix(body) => infer_fix(genv, lctx, body),
        Db::MutualFix { defs, index } => infer_mutual_fix(genv, lctx, defs, *index),
        // Let: check `ty` is a sort and `value : ty`, then type the body with the
        // value substituted in (zeta) — exactly the main kernel's by-substitution
        // rule, so both kernels agree.
        Db::Let(ty, value, body) => {
            let _ = infer_sort(genv, lctx, ty)?;
            check(genv, lctx, value, ty)?;
            infer(genv, lctx, &beta_open(body, value))
        }
    }
}

/// The effective motive of a `match`: a function motive `λx:I.T` (checked: its domain
/// matches the discriminant type, its codomain is a type) is used as-is; a raw type `T`
/// (a `Sort`-typed motive) is wrapped as `λ_:I. T` (shifting `T` under the new binder).
fn effective_motive(genv: &Context, lctx: &[Db], motive: &Db, disc_ty: &Db) -> RResult<Db> {
    let motive_ty = whnf(genv, &infer(genv, lctx, motive)?);
    match &motive_ty {
        Db::Pi(dom, cod) => {
            if !def_eq(genv, lctx, dom, disc_ty) {
                return Err(ReCheckError::ill(format!(
                    "motive domain {} does not match discriminant type {}",
                    from_db(dom, lctx.len()),
                    from_db(disc_ty, lctx.len())
                )));
            }
            let mut ext = lctx.to_vec();
            ext.push(disc_ty.clone());
            infer_sort(genv, &ext, cod)?;
            Ok(motive.clone())
        }
        Db::Sort(_) => Ok(Db::Lam(Box::new(disc_ty.clone()), Box::new(shift(motive, 1, 0)))),
        other => Err(ReCheckError::ill(format!(
            "motive is neither a function nor a type (type {})",
            from_db(other, lctx.len())
        ))),
    }
}

/// Type a `fix rec. body`: compute its (structural) type, INDEPENDENTLY verify the
/// termination guard, sanity-check the body under `rec : T`, and return `T`. The guard
/// is the load-bearing soundness check — without it `fix f. f` inhabits every type.
fn infer_fix(genv: &Context, lctx: &[Db], body: &Db) -> RResult<Db> {
    let fix_level = lctx.len();

    // 1. The fixpoint's type, read from the body's λ-telescope and its `match` codomain
    //    (which does not depend on `rec`). The body's indices are relative to a context
    //    that INCLUDES the `rec` binder, so `fix_type` must run with that slot present
    //    (a placeholder — its type is never consulted), or every index is off by one.
    let mut inner = lctx.to_vec();
    inner.push(Db::Sort(Universe::Prop));
    let fix_ty_inner = fix_type(genv, &inner, body)?;
    // The type does not reference `rec` (level `fix_level`, index 0 at this base), so drop
    // that slot to express the type in the enclosing context.
    let fix_ty = shift(&fix_ty_inner, -1, 0);

    // 2. THE GUARDIAN — an independent structural-decrease check. The body lives one
    //    binder (`rec`, at level `fix_level`) beneath the current depth.
    check_terminates(genv, body, fix_level)?;

    // 3. Sanity: the body type-checks with `rec : T` in scope.
    let mut ext = lctx.to_vec();
    ext.push(fix_ty.clone());
    let _ = infer(genv, &ext, body)?;

    Ok(fix_ty)
}

/// Type a `mutualfix { b₀ … b_{n-1} }.index` — the mutual analog of [`infer_fix`]. Each
/// body's structural type is read from its λ-telescope (independent of the recursive
/// names), the MUTUAL termination guard is verified over the whole block, the bodies are
/// sanity-checked with all `n` names bound, and the `index`-th type is returned.
fn infer_mutual_fix(genv: &Context, lctx: &[Db], defs: &[Db], index: usize) -> RResult<Db> {
    let n = defs.len();
    if n == 0 || index >= n {
        return Err(ReCheckError::ill("malformed mutual fixpoint".to_string()));
    }
    let base = lctx.len();

    // 1. Structural type of each body, computed with `n` placeholder binders in scope
    //    (the bodies' indices assume all `n` mutual names are present). The types do not
    //    mention those names, so drop the `n` slots (shift down by `n`) into the
    //    enclosing context — each `types[j]` is then expressed relative to depth `base`.
    let mut inner = lctx.to_vec();
    for _ in 0..n {
        inner.push(Db::Sort(Universe::Prop));
    }
    let mut types: Vec<Db> = Vec::with_capacity(n);
    for body in defs {
        let ty_inner = fix_type(genv, &inner, body)?;
        types.push(shift(&ty_inner, -(n as isize), 0));
    }

    // 2. THE GUARDIAN — the mutual structural-decrease check.
    check_terminates_mutual(genv, defs, base)?;

    // 3. Sanity: every body type-checks with all `n` names bound. Binder `j` sits at level
    //    `base + j`, so its type must be expressed relative to depth `base + j` (shift up
    //    by `j` from the `base`-relative `types[j]`).
    let mut ext = lctx.to_vec();
    for (j, ty) in types.iter().enumerate() {
        ext.push(shift(ty, j as isize, 0));
    }
    for body in defs {
        let _ = infer(genv, &ext, body)?;
    }

    Ok(types[index].clone())
}

/// The structural type of a fixpoint body: each leading `λ(_:A). …` contributes a
/// `Π(_:A). …`, and the innermost `match` contributes its return type `motive(disc)`
/// (independent of `rec`). A non-`match` tail is typed directly.
fn fix_type(genv: &Context, lctx: &[Db], body: &Db) -> RResult<Db> {
    match body {
        Db::Lam(dom, inner) => {
            let _ = infer_sort(genv, lctx, dom)?;
            let mut ext = lctx.to_vec();
            ext.push((**dom).clone());
            let inner_ty = fix_type(genv, &ext, inner)?;
            Ok(Db::Pi(dom.clone(), Box::new(inner_ty)))
        }
        Db::Match { disc, motive, .. } => {
            let disc_ty = whnf(genv, &infer(genv, lctx, disc)?);
            match_return_type(genv, lctx, motive, disc, &disc_ty)
        }
        other => infer(genv, lctx, other),
    }
}

// ---------------------------------------------------------------------------
// The termination guard (Giménez 1995 / the Coq guard) — independently re-derived.
//
// A fixpoint is sound only if every recursive call decreases a STRUCTURAL argument.
// Without this, `fix f. f` (and the higher-order escape `(λg. g Zero) f`) inhabit every
// type, including `False`. This is a SEPARATE implementation of the main kernel's
// `termination` module. Crucially, the main kernel tracks shadowing with explicit
// `struct_param_live`/`fix_name_live` flags because it works with NAMES; here, working
// in de Bruijn LEVELS, shadowing is automatic — an inner binder gets a fresh level, so a
// reference can never be mistaken for the recursive name or the structural parameter.
// That structural simplicity is the point: the two guards cannot share a shadowing bug.
// ---------------------------------------------------------------------------

/// The de Bruijn LEVEL referred to by index `k` at the given `depth` (`None` if the
/// index is out of range). A level is absolute, so it identifies a binder independently
/// of how deep the reference is — which is what makes shadowing a non-issue.
fn level_of(depth: usize, k: usize) -> Option<usize> {
    depth.checked_sub(1)?.checked_sub(k)
}

/// Count the leading `Π` parameters of a (named) constructor type — its arity.
fn count_pi_named(t: &Term) -> usize {
    match t {
        Term::Pi { body_type, .. } => 1 + count_pi_named(body_type),
        _ => 0,
    }
}

/// Locate a fixpoint body's structural parameter: peel the λ-telescope from `base_depth`,
/// pick the scrutinee of the innermost `match` if inductive-typed (else the first
/// inductive-typed binder). Returns the parameter's LEVEL, its inductive name, the body
/// just past it, and its argument POSITION in the telescope.
fn locate_struct<'a>(
    genv: &Context,
    body: &'a Db,
    base_depth: usize,
) -> RResult<(usize, String, &'a Db, usize)> {
    let mut chain: Vec<(usize, Option<String>, &Db)> = Vec::new();
    let mut cur = body;
    let mut depth = base_depth;
    while let Db::Lam(dom, inner) = cur {
        let dom_h = whnf(genv, dom);
        chain.push((depth, extract_inductive(genv, &dom_h).map(|(n, _)| n), inner));
        cur = inner;
        depth += 1;
    }
    let scrutinee = match cur {
        Db::Match { disc, .. } => match &**disc {
            Db::Var(k) => {
                level_of(depth, *k).and_then(|lvl| chain.iter().position(|(l, ..)| *l == lvl))
            }
            _ => None,
        },
        _ => None,
    };
    let idx = scrutinee
        .filter(|&i| chain[i].1.is_some())
        .or_else(|| chain.iter().position(|(_, ind, _)| ind.is_some()))
        .ok_or_else(|| {
            ReCheckError::ill(
                "fixpoint has no inductive parameter for structural recursion".to_string(),
            )
        })?;
    Ok((chain[idx].0, chain[idx].1.clone().unwrap(), chain[idx].2, idx))
}

/// Verify the fixpoint `body` (with `rec` bound at level `fix_level`) terminates: locate
/// the structural parameter and check that every recursive call decreases it. A single
/// fixpoint is the one-entry case of the block guard.
fn check_terminates(genv: &Context, body: &Db, fix_level: usize) -> RResult<()> {
    let (struct_level, ind, guard_body, struct_pos) =
        locate_struct(genv, body, fix_level + 1)?;
    let mut fix_positions = HashMap::new();
    fix_positions.insert(fix_level, struct_pos);
    guard(genv, guard_body, &fix_positions, struct_level, &ind, &HashSet::new(), struct_level + 1)
}

/// Verify a MUTUAL block of `n` fixpoint bodies terminates. The `n` names occupy levels
/// `base .. base+n-1`, so each body lives beneath all of them. Every member's structural
/// position is found first (assembling `level → position`); then each body is guarded so
/// that a call to ANY member decreases the CURRENT body's structural parameter — the
/// mutual Giménez guard, independently re-derived alongside the main kernel's.
fn check_terminates_mutual(genv: &Context, defs: &[Db], base: usize) -> RResult<()> {
    let n = defs.len();
    let base_depth = base + n;
    let mut fix_positions = HashMap::new();
    let mut located: Vec<(usize, String, &Db)> = Vec::with_capacity(n);
    for (j, body) in defs.iter().enumerate() {
        let (struct_level, ind, guard_body, struct_pos) = locate_struct(genv, body, base_depth)?;
        fix_positions.insert(base + j, struct_pos);
        located.push((struct_level, ind, guard_body));
    }
    for (struct_level, ind, guard_body) in &located {
        guard(
            genv,
            guard_body,
            &fix_positions,
            *struct_level,
            ind,
            &HashSet::new(),
            struct_level + 1,
        )?;
    }
    Ok(())
}

/// Walk `term` (at `depth`) checking every recursive call — to ANY block member whose
/// level → structural-position mapping is in `fix_positions` — applies a structurally-
/// smaller argument (a variable whose level is in `smaller`, the set of constructor-bound
/// variables from matching on the structural parameter `struct_level` of `struct_type`).
fn guard(
    genv: &Context,
    term: &Db,
    fix_positions: &HashMap<usize, usize>,
    struct_level: usize,
    struct_type: &str,
    smaller: &HashSet<usize>,
    depth: usize,
) -> RResult<()> {
    match term {
        Db::App(..) => {
            let (head, args) = spine(term);
            if let Db::Var(k) = &head {
                if let Some(&pos) = level_of(depth, *k).and_then(|lvl| fix_positions.get(&lvl)) {
                    // A recursive call to some member: the argument at THAT member's
                    // structural position must be structurally smaller — a constructor-
                    // bound variable, or an APPLICATION `h a…` whose head `h` is one (the
                    // applied-smaller / Giménez rule, sound by strict positivity — see the
                    // main kernel's `verify_structural_arg_smaller`).
                    match args.get(pos) {
                        Some(arg) => {
                            let mut h: &Db = arg;
                            while let Db::App(f, _) = h {
                                h = f;
                            }
                            match h {
                                Db::Var(j)
                                    if level_of(depth, *j)
                                        .is_some_and(|l| smaller.contains(&l)) => {}
                                Db::Var(_) => {
                                    return Err(ReCheckError::ill(
                                        "recursive call on an argument not headed by a \
                                         structurally-smaller variable"
                                            .to_string(),
                                    ))
                                }
                                _ => {
                                    return Err(ReCheckError::ill(
                                        "recursive call whose structural argument is not a \
                                         variable or an application of one — cannot certify it \
                                         decreases"
                                            .to_string(),
                                    ))
                                }
                            }
                        }
                        None => {
                            return Err(ReCheckError::ill(
                                "recursive call is missing its structural argument".to_string(),
                            ))
                        }
                    }
                    for a in &args {
                        guard(genv, a, fix_positions, struct_level, struct_type, smaller, depth)?;
                    }
                    return Ok(());
                }
            }
            guard(genv, &head, fix_positions, struct_level, struct_type, smaller, depth)?;
            for a in &args {
                guard(genv, a, fix_positions, struct_level, struct_type, smaller, depth)?;
            }
            Ok(())
        }
        Db::Match { disc, motive, cases } => {
            // The return motive is an ordinary subterm and MUST be guarded too (a recursive
            // occurrence in the return predicate would otherwise evade the check).
            guard(genv, motive, fix_positions, struct_level, struct_type, smaller, depth)?;
            // A match on the (un-shadowed) structural parameter guards the recursive
            // calls in its cases: each constructor argument is a structural subterm.
            if let Db::Var(k) = &**disc {
                if level_of(depth, *k) == Some(struct_level) {
                    return guard_match_cases(
                        genv, cases, struct_type, fix_positions, struct_level, smaller, depth,
                    );
                }
            }
            guard(genv, disc, fix_positions, struct_level, struct_type, smaller, depth)?;
            for c in cases {
                guard(genv, c, fix_positions, struct_level, struct_type, smaller, depth)?;
            }
            Ok(())
        }
        // Guard a binder's DOMAIN annotation (current depth) as well as its body (one
        // deeper). `Fix` has no domain.
        Db::Lam(dom, inner) | Db::Pi(dom, inner) => {
            guard(genv, dom, fix_positions, struct_level, struct_type, smaller, depth)?;
            guard(genv, inner, fix_positions, struct_level, struct_type, smaller, depth + 1)
        }
        Db::Fix(inner) => {
            guard(genv, inner, fix_positions, struct_level, struct_type, smaller, depth + 1)
        }
        // A nested mutual block introduces `n` fresh levels; its own termination is checked
        // when it is type-checked. Descend into each body, past those `n` binders — an
        // outer recursive call from within it must still decrease.
        Db::MutualFix { defs, .. } => {
            for b in defs {
                guard(genv, b, fix_positions, struct_level, struct_type, smaller, depth + defs.len())?;
            }
            Ok(())
        }
        Db::Let(ty, value, body) => {
            // `ty`/`value` at the current depth; `body` one binder deeper. The
            // bound value is NOT marked smaller (conservative guard).
            guard(genv, ty, fix_positions, struct_level, struct_type, smaller, depth)?;
            guard(genv, value, fix_positions, struct_level, struct_type, smaller, depth)?;
            guard(genv, body, fix_positions, struct_level, struct_type, smaller, depth + 1)
        }
        Db::Var(k) => {
            // A bare member name (not the head of a guarded call) is the higher-order
            // escape — a recursive name used as a first-class value. Reject it.
            if level_of(depth, *k).is_some_and(|lvl| fix_positions.contains_key(&lvl)) {
                Err(ReCheckError::ill(
                    "recursive name occurs as a first-class value, not applied to a \
                     structurally-smaller argument"
                        .to_string(),
                ))
            } else {
                Ok(())
            }
        }
        Db::Sort(_) | Db::Global(_) | Db::Const { .. } | Db::Lit(_) => Ok(()),
    }
}

/// Guard each case of a match on the structural parameter: mark every constructor
/// argument it binds as structurally smaller, then check the case body.
fn guard_match_cases(
    genv: &Context,
    cases: &[Db],
    struct_type: &str,
    fix_positions: &HashMap<usize, usize>,
    struct_level: usize,
    smaller: &HashSet<usize>,
    depth: usize,
) -> RResult<()> {
    let arities: Vec<usize> =
        genv.get_constructors(struct_type).iter().map(|(_, t)| count_pi_named(t)).collect();
    for (i, case) in cases.iter().enumerate() {
        let arity = arities.get(i).copied().unwrap_or(0);
        let mut smaller2 = smaller.clone();
        let mut cur = case;
        let mut d = depth;
        // Peel up to `arity` λs (a parametric inductive's case binds fewer than its
        // constructor's total parameter count — type parameters are fixed), marking each
        // bound variable's level as structurally smaller.
        for _ in 0..arity {
            if let Db::Lam(_, inner) = cur {
                smaller2.insert(d);
                cur = inner;
                d += 1;
            } else {
                break;
            }
        }
        guard(genv, cur, fix_positions, struct_level, struct_type, &smaller2, d)?;
    }
    Ok(())
}

/// Type a `match`: discriminant must be inductive; the motive (function `λx:I.T` or a
/// raw type) gives the return type; case count must equal constructor count; each case
/// is checked against the type derived from its constructor's signature; and a `Prop`
/// discriminant may only large-eliminate into `Type` if it is a subsingleton.
fn infer_match(
    genv: &Context,
    lctx: &[Db],
    disc: &Db,
    motive: &Db,
    cases: &[Db],
) -> RResult<Db> {
    // 1. Discriminant's inductive type and its type arguments.
    let disc_ty = whnf(genv, &infer(genv, lctx, disc)?);
    let (ind_name, type_args) = extract_inductive(genv, &disc_ty).ok_or_else(|| {
        ReCheckError::unsupported(format!(
            "match discriminant of unrecognized type {}",
            from_db(&disc_ty, lctx.len())
        ))
    })?;

    // 2. Parameter/index split: the first `p` type arguments are uniform parameters; any
    // remaining ones are INDICES the motive abstracts over (an indexed family like `Eq`).
    // `indexed == false` is the ordinary eliminator, handled exactly as before.
    let p = genv.inductive_num_params(&ind_name).min(type_args.len());
    let indexed = type_args.len() > p;
    let eff_motive = if indexed {
        let _ = infer(genv, lctx, motive)?; // the motive must at least type-check
        None
    } else {
        Some(effective_motive(genv, lctx, motive, &disc_ty)?)
    };

    // 3. Coverage: exactly one case per constructor, in registration order.
    let ctor_names: Vec<String> =
        genv.get_constructors(&ind_name).iter().map(|(n, _)| n.to_string()).collect();
    if cases.len() != ctor_names.len() {
        return Err(ReCheckError::ill(format!(
            "match on {} has {} cases but {} constructors",
            ind_name,
            cases.len(),
            ctor_names.len()
        )));
    }

    // 4. Each case against its constructor-derived type.
    for (case, cname) in cases.iter().zip(ctor_names.iter()) {
        let case_ty = match &eff_motive {
            Some(eff) => case_type(genv, eff, cname, &type_args)?,
            None => case_type_indexed(genv, motive, cname, &type_args[..p])?,
        };
        check_case(genv, lctx, case, &case_ty)?;
    }

    // 5. Return type: the motive applied to the discriminant's indices, then the scrutinee.
    let ret = match_return_type(genv, lctx, motive, disc, &disc_ty)?;

    // 6. Large-elimination restriction: a `Prop` may be eliminated into `Type` only if it
    // is a subsingleton (empty, or one constructor with purely propositional arguments).
    let disc_sort = whnf(genv, &infer(genv, lctx, &disc_ty)?);
    if matches!(disc_sort, Db::Sort(Universe::Prop)) {
        let ret_sort = whnf(genv, &infer(genv, lctx, &ret)?);
        let large = !matches!(ret_sort, Db::Sort(Universe::Prop));
        if large && !is_subsingleton(genv, &ind_name)? {
            return Err(ReCheckError::ill(format!(
                "large elimination of non-subsingleton proposition '{}' into a larger sort",
                ind_name
            )));
        }
    }

    Ok(ret)
}

/// Extract the inductive name and its type arguments from a (head-normal) type: peel the
/// application spine; the head must be a `Global` registered as an inductive.
fn extract_inductive(genv: &Context, ty: &Db) -> Option<(String, Vec<Db>)> {
    let (head, args) = spine(ty);
    match head {
        Db::Global(name) if genv.is_inductive(&name) => Some((name, args)),
        _ => None,
    }
}

/// The expected type of the case for `ctor_name`, given the `motive` and the
/// discriminant's `type_args`. The constructor's type `Π(p₀:U₀)…Π(pₘ₋₁:Uₘ₋₁). Ind …`
/// has its leading `|type_args|` parameters instantiated by the type arguments
/// (β-style), and its final codomain replaced by `motive (Ctor type_args value_args)`,
/// leaving `Π(value params). motive (Ctor …)`. Pure de Bruijn — no names to capture.
fn case_type(genv: &Context, motive: &Db, ctor_name: &str, type_args: &[Db]) -> RResult<Db> {
    let ctor_named = genv
        .get_global(ctor_name)
        .ok_or_else(|| ReCheckError::ill(format!("unknown constructor '{}'", ctor_name)))?
        .clone();
    let ctor_db = to_db(&ctor_named, &mut Vec::new())?;

    // Instantiate the type parameters (each `type_args[i]` is closed over the telescope).
    let mut body = ctor_db;
    for ta in type_args {
        match body {
            Db::Pi(_dom, rest) => body = beta_open(&rest, ta),
            _ => {
                return Err(ReCheckError::ill(format!(
                    "constructor '{}' has fewer parameters than the discriminant's type arguments",
                    ctor_name
                )))
            }
        }
    }

    // Peel the remaining value-parameter telescope.
    let mut value_doms = Vec::new();
    let mut cur = body;
    while let Db::Pi(dom, rest) = cur {
        value_doms.push(*dom);
        cur = *rest;
    }
    let num_val = value_doms.len();
    let lift = num_val as isize;

    // Build `Ctor type_args value_vars` at the bottom of the value telescope.
    let mut applied = Db::Global(ctor_name.to_string());
    for ta in type_args {
        applied = Db::App(Box::new(applied), Box::new(shift(ta, lift, 0)));
    }
    for p in 0..num_val {
        applied = Db::App(Box::new(applied), Box::new(Db::Var(num_val - 1 - p)));
    }

    let motive_bot = shift(motive, lift, 0);
    let cod = whnf(genv, &Db::App(Box::new(motive_bot), Box::new(applied)));

    // Re-wrap the value parameters.
    let mut case_ty = cod;
    for dom in value_doms.into_iter().rev() {
        case_ty = Db::Pi(Box::new(dom), Box::new(case_ty));
    }
    Ok(case_ty)
}

/// A match's return type, indexed-aware. For an ordinary (non-indexed) inductive it is the
/// motive applied to the discriminant. For an INDEXED family the motive abstracts over the
/// indices too, so it is applied to the discriminant's own index arguments and THEN the
/// discriminant — `Eq.rec`'s motive `P` yields `P y h` for a scrutinee `h : Eq A x y`.
fn match_return_type(genv: &Context, lctx: &[Db], motive: &Db, disc: &Db, disc_ty: &Db) -> RResult<Db> {
    let (ind_name, type_args) = extract_inductive(genv, disc_ty).ok_or_else(|| {
        ReCheckError::unsupported(format!(
            "match discriminant of unrecognized type {}",
            from_db(disc_ty, lctx.len())
        ))
    })?;
    let p = genv.inductive_num_params(&ind_name).min(type_args.len());
    if type_args.len() > p {
        let mut ret = motive.clone();
        for idx in &type_args[p..] {
            ret = Db::App(Box::new(ret), Box::new(idx.clone()));
        }
        ret = Db::App(Box::new(ret), Box::new(disc.clone()));
        Ok(whnf(genv, &ret))
    } else {
        let eff = effective_motive(genv, lctx, motive, disc_ty)?;
        Ok(whnf(genv, &Db::App(Box::new(eff), Box::new(disc.clone()))))
    }
}

/// The expected case type for `ctor_name` in an INDEXED match. Only the inductive's
/// `params` are instantiated (not the trailing indices); the constructor's remaining value
/// arguments become the case's `Π` binders; and the codomain is the `motive` applied to
/// the constructor's RESULT indices (its declared return-type arguments past the
/// parameters) and then the constructor value. Pure de Bruijn — mirrors [`case_type`].
fn case_type_indexed(genv: &Context, motive: &Db, ctor_name: &str, params: &[Db]) -> RResult<Db> {
    let ctor_named = genv
        .get_global(ctor_name)
        .ok_or_else(|| ReCheckError::ill(format!("unknown constructor '{}'", ctor_name)))?
        .clone();
    let ctor_db = to_db(&ctor_named, &mut Vec::new())?;

    // Instantiate the inductive's PARAMETERS only (the leading `params.len()` binders).
    let mut body = ctor_db;
    for pa in params {
        match body {
            Db::Pi(_dom, rest) => body = beta_open(&rest, pa),
            _ => {
                return Err(ReCheckError::ill(format!(
                    "constructor '{}' has fewer parameters than the inductive",
                    ctor_name
                )))
            }
        }
    }

    // Peel the value-parameter telescope; the residual is the constructor's result type.
    let mut value_doms = Vec::new();
    let mut cur = body;
    while let Db::Pi(dom, rest) = cur {
        value_doms.push(*dom);
        cur = *rest;
    }
    let num_val = value_doms.len();
    let lift = num_val as isize;

    // The constructor's RESULT indices: its result-type spine arguments past the params
    // (already expressed under the value binders, so no shift).
    let (_head, res_args) = spine(&cur);
    let result_indices: Vec<Db> = res_args.into_iter().skip(params.len()).collect();

    // Build `Ctor params value_vars` at the bottom of the value telescope.
    let mut applied = Db::Global(ctor_name.to_string());
    for pa in params {
        applied = Db::App(Box::new(applied), Box::new(shift(pa, lift, 0)));
    }
    for i in 0..num_val {
        applied = Db::App(Box::new(applied), Box::new(Db::Var(num_val - 1 - i)));
    }

    // `motive result_indices… applied`, all under the value binders.
    let mut cod = shift(motive, lift, 0);
    for ri in &result_indices {
        cod = Db::App(Box::new(cod), Box::new(ri.clone()));
    }
    cod = Db::App(Box::new(cod), Box::new(applied));
    let cod = whnf(genv, &cod);

    // Re-wrap the value parameters.
    let mut case_ty = cod;
    for dom in value_doms.into_iter().rev() {
        case_ty = Db::Pi(Box::new(dom), Box::new(case_ty));
    }
    Ok(case_ty)
}

/// Whether an inductive is a subsingleton `Prop`: zero constructors (e.g. `False`), or
/// exactly one whose arguments beyond the inductive's parameters are all propositional
/// (e.g. `And`, `eq`). Only these may be large-eliminated into `Type`.
fn is_subsingleton(genv: &Context, ind: &str) -> RResult<bool> {
    let ctors: Vec<(String, Term)> = genv
        .get_constructors(ind)
        .iter()
        .map(|(n, t)| (n.to_string(), (*t).clone()))
        .collect();
    match ctors.len() {
        0 => Ok(true),
        1 => {
            let arity = inductive_arity(genv, ind);
            let ctor_db = to_db(&ctors[0].1, &mut Vec::new())?;
            let mut lctx: Vec<Db> = Vec::new();
            let mut cur = ctor_db;
            let mut i = 0;
            while let Db::Pi(dom, rest) = cur {
                if i >= arity && infer_sort(genv, &lctx, &dom)? != Universe::Prop {
                    return Ok(false);
                }
                lctx.push(*dom);
                cur = *rest;
                i += 1;
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Infer `t`'s type and require it to be a sort; return that universe.
fn infer_sort(genv: &Context, lctx: &[Db], t: &Db) -> RResult<Universe> {
    let ty = whnf(genv, &infer(genv, lctx, t)?);
    match ty {
        Db::Sort(u) => Ok(u),
        other => Err(ReCheckError::ill(format!(
            "expected a type, got something of type {}",
            from_db(&other, lctx.len())
        ))),
    }
}

/// Check `t` against `expected` under cumulativity.
/// Check a `match` case against its constructor-derived type. A case's pattern binders
/// carry only PLACEHOLDER types from the surface parser (`λx:_. …`), so — exactly as the
/// main kernel does — we ignore them and take each binder's type from the constructor
/// telescope in `case_ty` (`Π(a:Aₖ). … motive (Ctor …)`): peel the case `λ` and the `Π`
/// in lockstep, pushing the constructor-derived domains into the context, then check the
/// case body against the residual codomain. On a real (non-placeholder) case `λ` this is
/// identical to `check`, since the binder type equals the telescope domain.
fn check_case(genv: &Context, lctx: &[Db], case: &Db, case_ty: &Db) -> RResult<()> {
    match (case, case_ty) {
        (Db::Lam(_, body), Db::Pi(dom, cod)) => {
            let mut ext = lctx.to_vec();
            ext.push((**dom).clone());
            check_case(genv, &ext, body, cod)
        }
        _ => check(genv, lctx, case, case_ty),
    }
}

fn check(genv: &Context, lctx: &[Db], t: &Db, expected: &Db) -> RResult<()> {
    let inferred = infer(genv, lctx, t)?;
    if is_sub(genv, lctx, &inferred, expected) {
        Ok(())
    } else {
        Err(ReCheckError::ill(format!(
            "type mismatch: have {}, expected {}",
            from_db(&whnf(genv, &inferred), lctx.len()),
            from_db(&whnf(genv, expected), lctx.len())
        )))
    }
}

fn lit_type_name(l: &Literal) -> &'static str {
    match l {
        Literal::Int(_) | Literal::BigInt(_) => "Int",
        Literal::Nat(_) => "Nat",
        Literal::Float(_) => "Float",
        Literal::Text(_) => "Text",
        Literal::Duration(_) => "Duration",
        Literal::Date(_) => "Date",
        Literal::Moment(_) => "Moment",
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Independently re-check `term` in `ctx`, returning its inferred type on success.
///
/// A SEPARATE implementation from [`infer_type`](crate::infer_type): a de Bruijn core
/// (now including the `Match` eliminator) with its own reduction and cumulative
/// conversion, so a bug in one checker is unlikely to be shared by the other. Returns
/// [`ReCheckError::Unsupported`] for the fragment it does not yet cover (`Fix`/`Hole`,
/// or an unrecognized inductive) — never a false pass.
pub fn recheck(ctx: &Context, term: &Term) -> RResult<Term> {
    let db = to_db(term, &mut Vec::new())?;
    let ty = infer(ctx, &[], &db)?;
    Ok(from_db(&ty, 0))
}

/// The verdict of cross-checking a term with BOTH kernels.
#[derive(Debug, Clone, PartialEq)]
pub enum DoubleCheck {
    /// Both kernels accepted and inferred definitionally-equal types — the strongest
    /// guarantee: two independent checkers concur.
    Agreed,
    /// The main kernel accepted, but the re-checker hit a construct outside its current
    /// fragment (a `Fix`/`Hole`, or an inductive it does not recognize). The term is
    /// single-checked, honestly flagged — not a soundness failure, just incomplete
    /// redundancy.
    MainOnlyReCheckerIncomplete(String),
    /// The two kernels DISAGREE: one accepted and the other rejected, or they inferred
    /// non-equal types. A soundness alarm that must never fire on a valid proof.
    Disagree(String),
}

/// Cross-check `term` against both [`infer_type`](crate::infer_type) and `recheck`.
///
/// The de Bruijn criterion in action: a proof term is most trustworthy when two
/// independently-written kernels, on different representations, agree on its type.
/// Disagreement is surfaced loudly; an incomplete re-check (the `Fix` fragment) is
/// surfaced honestly rather than dressed up as agreement.
pub fn double_check(ctx: &Context, term: &Term) -> DoubleCheck {
    let main = crate::infer_type(ctx, term);
    let recheck_result = recheck(ctx, term);
    match (main, recheck_result) {
        (Ok(main_ty), Ok(re_ty)) => {
            match (to_db(&main_ty, &mut Vec::new()), to_db(&re_ty, &mut Vec::new())) {
                (Ok(m), Ok(r)) if def_eq(ctx, &[], &m, &r) => DoubleCheck::Agreed,
                (Ok(_), Ok(_)) => DoubleCheck::Disagree(format!(
                    "kernels inferred different types: main={}, recheck={}",
                    main_ty, re_ty
                )),
                _ => DoubleCheck::MainOnlyReCheckerIncomplete(
                    "inferred type outside re-checker fragment".to_string(),
                ),
            }
        }
        (Ok(_), Err(ReCheckError::Unsupported(why))) => {
            DoubleCheck::MainOnlyReCheckerIncomplete(why)
        }
        (Ok(_), Err(ReCheckError::Ill(why))) => DoubleCheck::Disagree(format!(
            "main kernel accepted but re-checker rejected: {}",
            why
        )),
        (Err(e), Ok(_)) => DoubleCheck::Disagree(format!(
            "re-checker accepted but main kernel rejected: {:?}",
            e
        )),
        (Err(_), Err(_)) => DoubleCheck::Agreed,
    }
}
