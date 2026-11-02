//! Equation lemmas — Lean's `f.eq_def`, `f.eq_1 … f.eq_n` — the NAME a proof cites when it appeals to a
//! definition's behaviour, which is what `rw [f]` / `simp [f]` / `unfold f` all need.
//!
//! A LOGOS definition is δ-transparent and its `Fix`/`Match` ι-compute, so the facts about a
//! recursive function are all TRUE by conversion the moment it is declared: `dbl 3 ≡ 6`, and
//! `dbl (Succ n) ≡ Succ (Succ (dbl n))` for a free `n`. What was missing is a name. Every appeal
//! to a definition in a real proof — `rw [f]`, `simp [f]`, `unfold f` — cites a CONSTANT, and
//! without one every such appeal degrades to a full `normalize` whose fuel silently returns a
//! partially reduced term when it runs out.
//!
//! # What Lean does, and where
//!
//! * **The names.** `Lean/Meta/Eqns.lean:59-68` fixes them: `eqnThmSuffixBase := "eq"`,
//!   `eqn1ThmSuffix := "eq_1"`, `unfoldThmSuffix := "eq_def"`. In Lean's hierarchical namespace
//!   those are `f.eq_def` and `f.eq_1 … f.eq_n`; LOGOS's context is flat, so the same suffixes
//!   are spelled `f_eq_def` and `f_eq_1 … f_eq_n`.
//! * **The shape and the proof.** `Lean/Meta/Eqns.lean:149-162` (`mkSimpleEqThm`) telescopes the
//!   definition's VALUE, states `∀ xs, f xs = body`, and proves it with `Eq.refl (f xs)` under
//!   the same telescope. That is exactly the shape built here: the theorem is true by conversion,
//!   so `refl` is the whole proof, and the kernel's own registration gateway re-checks it.
//! * **When.** `Lean/Meta/Eqns.lean:142-144` — *"We generate the equations on demand"* — via
//!   `getEqnsFor?` and the reserved-name action at `Lean/Meta/Eqns.lean:281-289`. LOGOS generates
//!   EAGERLY instead, and the reason is a signature rather than a preference:
//!   `Context::get_global` returns `Option<&Term>`, a reference into the context, so a lookup
//!   cannot synthesise a term it does not already hold. Eager generation costs one `refl` per
//!   definition and makes the lemma visible to every reader, including `Check`.
//!
//! # The three hazards, and how each is closed
//!
//! * **Re-entrancy.** A lemma is itself a definition, so generating one would generate its own.
//!   [`is_equation_name`] is checked first and an equation name generates nothing.
//! * **Bootstrap.** `Eq` and `refl` are registered part-way through the prelude
//!   (`prelude.rs::register_eq`), so everything declared before them has no equality to state an
//!   equation IN. Generation is a no-op while they are absent, and
//!   [`generate_missing_for_all`] back-fills once they exist.
//! * **Displacement.** `Context::declare` displaces the name it registers. A lemma is skipped
//!   whenever its name is already taken, so generation can never remove something a user wrote.

use crate::type_checker::substitute;
use crate::{Context, Term};

/// Lean's `unfoldThmSuffix` (`Lean/Meta/Eqns.lean:68`), flattened.
pub const UNFOLD_SUFFIX: &str = "_eq_def";

/// Lean's `eqnThmSuffixBasePrefix` (`Lean/Meta/Eqns.lean:60`), flattened.
pub const EQUATION_INFIX: &str = "_eq_";

/// Whether a name is one this module MINTS, rather than one it generates lemmas for.
///
/// This is Lean's `isEqnReservedNameSuffix` (`Lean/Meta/Eqns.lean:63-66`) — the `eq_<digits>`
/// test — plus the unfold suffix. It is the re-entrancy guard: without it the first lemma
/// generated would ask for its own lemmas, for ever.
pub fn is_equation_name(name: &str) -> bool {
    if name.ends_with(UNFOLD_SUFFIX) {
        return true;
    }
    match name.rfind(EQUATION_INFIX) {
        Some(at) => {
            let (head, tail) = name.split_at(at);
            let idx = &tail[EQUATION_INFIX.len()..];
            !head.is_empty() && !idx.is_empty() && idx.bytes().all(|b| b.is_ascii_digit())
        }
        None => false,
    }
}

/// `Eq A lhs rhs`.
fn eq_prop(carrier: &Term, lhs: &Term, rhs: &Term) -> Term {
    Term::App(
        Box::new(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("Eq".to_string())),
                Box::new(carrier.clone()),
            )),
            Box::new(lhs.clone()),
        )),
        Box::new(rhs.clone()),
    )
}

/// `refl A x` — the proof of every equation here, because every equation here is true by
/// conversion. Lean's `mkEqRefl` at `Lean/Meta/Eqns.lean:156`.
fn refl_proof(carrier: &Term, at: &Term) -> Term {
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("refl".to_string())),
            Box::new(carrier.clone()),
        )),
        Box::new(at.clone()),
    )
}

fn pi_over(binders: &[(String, Term)], body: Term) -> Term {
    binders.iter().rev().fold(body, |acc, (n, t)| Term::Pi {
        param: n.clone(),
        param_type: Box::new(t.clone()),
        body_type: Box::new(acc),
    })
}

fn lam_over(binders: &[(String, Term)], body: Term) -> Term {
    binders.iter().rev().fold(body, |acc, (n, t)| Term::Lambda {
        param: n.clone(),
        param_type: Box::new(t.clone()),
        body: Box::new(acc),
    })
}

fn apply_all(head: Term, args: &[Term]) -> Term {
    args.iter()
        .fold(head, |acc, a| Term::App(Box::new(acc), Box::new(a.clone())))
}

/// Apply a case's λ-telescope to the fields, β-reducing as it goes.
///
/// A `Term::Match` case IS a λ-telescope over its constructor's fields, so applying it to the
/// field binders is correct either way — but `dbl_eq_2 : dbl (Succ k) = (λk. Succ (Succ (dbl k))) k`
/// is a redex on the right-hand side, and Lean's `dbl.eq_2` reads `dbl n.succ = (dbl n).succ.succ`.
/// A lemma is cited by `rw`, which matches its shape, so the shape is part of the lemma.
fn apply_case(case: &Term, args: &[Term]) -> Term {
    let mut out = case.clone();
    let mut rest = args;
    while let (Term::Lambda { param, body, .. }, Some((arg, tail))) = (&out, rest.split_first()) {
        let next = substitute(body, param, arg);
        out = next;
        rest = tail;
    }
    apply_all(out, rest)
}

/// Every name a term spells, so a generated binder can be stepped clear of all of them.
fn spelled(term: &Term, out: &mut Vec<String>) {
    match term {
        Term::Var(n) | Term::Global(n) => out.push(n.clone()),
        Term::Const { name, .. } => out.push(name.clone()),
        Term::App(f, x) => {
            spelled(f, out);
            spelled(x, out);
        }
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            out.push(param.clone());
            spelled(param_type, out);
            spelled(body_type, out);
        }
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            out.push(param.clone());
            spelled(param_type, out);
            spelled(body, out);
        }
        Term::Fix { name, body } => {
            out.push(name.clone());
            spelled(body, out);
        }
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            out.push(name.clone());
            spelled(ty, out);
            spelled(value, out);
            spelled(body, out);
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            spelled(discriminant, out);
            spelled(motive, out);
            for c in cases {
                spelled(c, out);
            }
        }
        _ => {}
    }
}

/// The head inductive of a type application and its arguments: `List A` → `("List", [A])`.
fn split_type_app(ty: &Term) -> Option<(String, Vec<Term>)> {
    let mut args = Vec::new();
    let mut cur = ty;
    while let Term::App(f, a) = cur {
        args.push((**a).clone());
        cur = f;
    }
    args.reverse();
    match cur {
        Term::Global(n) => Some((n.clone(), args)),
        _ => None,
    }
}

/// A constructor's VALUE fields — its `Π` telescope past the inductive's parameters, with those
/// parameters instantiated by the discriminant type's own arguments. The same walk
/// `interface::match_compile::constructor_field_types` does, and for the same reason: it is what
/// `type_checker::compute_case_type` will check the case against.
fn constructor_fields(ctor_ty: &Term, type_args: &[Term]) -> Vec<(String, Term)> {
    let mut all: Vec<(String, Term)> = Vec::new();
    let mut cur = ctor_ty;
    while let Term::Pi {
        param,
        param_type,
        body_type,
    } = cur
    {
        all.push((param.clone(), (**param_type).clone()));
        cur = body_type;
    }
    let split = type_args.len().min(all.len());
    let params: Vec<String> = all[..split].iter().map(|(n, _)| n.clone()).collect();
    all[split..]
        .iter()
        .map(|(n, t)| {
            let mut ty = t.clone();
            for (p, a) in params.iter().zip(type_args.iter()) {
                if p != "_" {
                    ty = substitute(&ty, p, a);
                }
            }
            (n.clone(), ty)
        })
        .collect()
}

/// The equation lemmas a definition owes, as `(name, statement, proof)` in Lean's order:
/// the unfold theorem first, then one equation per constructor of the matched argument.
///
/// ★ TWO CANDIDATES MAY SHARE THE NAME `<f>_eq_def`, IN PREFERENCE ORDER, and [`generate`]
/// registers the first the kernel actually CHECKS. This is not indecision; it is the one place
/// where LOGOS's reduction differs from Lean's and the difference is visible.
///
/// Lean's unfold theorem is the APPLIED form — `dbl.eq_def : ∀ x, dbl x = match x with …`,
/// confirmed by `#check @dbl.eq_def` on this machine — and Lean proves it with a real derivation
/// (`Lean/Elab/PreDefinition/Eqns.lean:312` `mkUnfoldEq`), not with `rfl`. It cannot be `rfl` in
/// LOGOS either, and for a sharper reason: `reduction.rs`'s `try_fix_unroll` unrolls a fixpoint
/// only when its STRUCTURAL argument is already a constructor, so `dbl n` for a free `n` is a
/// normal form and is not convertible with the `match` it would step to. The applied form is
/// therefore offered first and, where the guard blocks it, the UNAPPLIED `dbl = fix dbl. …`
/// stands in — true by one δ step, always, and still a name `unfold dbl` can cite.
///
/// Returns an empty vector when there is nothing to state — an equation name, a context with no
/// `Eq`/`refl` yet, or a body whose shape carries no equations.
pub fn equation_lemmas(
    ctx: &Context,
    name: &str,
    ty: &Term,
    body: &Term,
) -> Vec<(String, Term, Term)> {
    if is_equation_name(name) {
        return Vec::new();
    }
    if ctx.get_global("Eq").is_none() || ctx.get_global("refl").is_none() {
        return Vec::new();
    }

    // ONE δι STEP. The stored body of a recursive definition is `fix f. e`; the equation Lean
    // states is about the function APPLIED, so the self-reference is folded back to the name it
    // now has. `f ≡ e[f := f]` by δ followed by ι, so `refl` still proves every statement below.
    let unfolded = match body {
        Term::Fix {
            name: fix_binder,
            body,
        } => substitute(body, fix_binder, &Term::Global(name.to_string())),
        other => other.clone(),
    };

    // Lean's `lambdaTelescope info.value` (`Lean/Meta/Eqns.lean:151`), walked against the
    // declared type so the result type is known at the leaf.
    let mut binders: Vec<(String, Term)> = Vec::new();
    let mut rest = unfolded;
    let mut result_ty = ty.clone();
    loop {
        let (param, param_type, next_body) = match &rest {
            Term::Lambda {
                param,
                param_type,
                body,
            } => (param.clone(), (**param_type).clone(), (**body).clone()),
            _ => break,
        };
        let next_ty = match &result_ty {
            Term::Pi {
                param: p,
                body_type,
                ..
            } => {
                if *p == param {
                    (**body_type).clone()
                } else {
                    substitute(body_type, p, &Term::Var(param.clone()))
                }
            }
            _ => break,
        };
        binders.push((param, param_type));
        rest = next_body;
        result_ty = next_ty;
    }

    let head = Term::Global(name.to_string());
    let args: Vec<Term> = binders.iter().map(|(n, _)| Term::Var(n.clone())).collect();
    let applied = apply_all(head.clone(), &args);

    let mut out = Vec::new();

    // THE UNFOLD THEOREM, Lean's shape first: `∀ xs, f xs = <body>`
    // (`Lean/Meta/Eqns.lean:152-157`, and `#check @dbl.eq_def`).
    out.push((
        format!("{name}{UNFOLD_SUFFIX}"),
        pi_over(&binders, eq_prop(&result_ty, &applied, &rest)),
        lam_over(&binders, refl_proof(&result_ty, &applied)),
    ));
    // …and the fallback the fixpoint guard forces: `f = <its stored body>`, one δ step.
    out.push((
        format!("{name}{UNFOLD_SUFFIX}"),
        eq_prop(ty, &head, body),
        refl_proof(ty, &head),
    ));

    // THE PER-CONSTRUCTOR EQUATIONS. These are what `rw [f]` at one constructor actually cites,
    // and they exist only when the definition's leaf is a `Match` on one of its own parameters —
    // which is every definition the `Consider` surface compiles.
    let Term::Match {
        discriminant,
        cases,
        ..
    } = &rest
    else {
        return out;
    };
    let Term::Var(scrutinee) = discriminant.as_ref() else {
        return out;
    };
    let Some(position) = binders.iter().position(|(n, _)| n == scrutinee) else {
        return out;
    };
    let Some((ind, type_args)) = split_type_app(&binders[position].1) else {
        return out;
    };
    let constructors: Vec<(String, Term)> = ctx
        .get_constructors(&ind)
        .into_iter()
        .map(|(n, t)| (n.to_string(), t.clone()))
        .collect();
    if constructors.is_empty() || constructors.len() != cases.len() {
        return out;
    }

    let mut taken: Vec<String> = binders.iter().map(|(n, _)| n.clone()).collect();
    spelled(&rest, &mut taken);
    spelled(&result_ty, &mut taken);

    for (i, (ctor_name, ctor_ty)) in constructors.iter().enumerate() {
        let fields = constructor_fields(ctor_ty, &type_args);

        // One fresh binder per value field, stepped clear of every name the definition spells —
        // the field names come from the constructor's own telescope and may collide with the
        // definition's parameters, and a collision would make the substitution below rewrite the
        // wrong occurrence.
        let mut field_binders: Vec<(String, Term)> = Vec::new();
        for (orig, fty) in &fields {
            // A constructor telescope often names its fields `_` (`Succ : Nat -> Nat`), which is
            // a hole rather than a name; Lean's generated equation calls that field `n`. Give an
            // anonymous field a readable base so the lemma reads as a statement.
            let base = if orig == "_" { "x" } else { orig.as_str() };
            let mut candidate = format!("{base}#eq");
            let mut step = 0usize;
            while taken.contains(&candidate) {
                candidate = format!("{base}#eq{step}");
                step += 1;
            }
            taken.push(candidate.clone());
            // A DEPENDENT constructor's later field types name its earlier fields; rewrite
            // each to the binder actually introduced for it.
            let mut closed = fty.clone();
            for (j, (prev_name, _)) in field_binders.iter().enumerate() {
                closed = substitute(&closed, &fields[j].0, &Term::Var(prev_name.clone()));
            }
            field_binders.push((candidate, closed));
        }

        // `C typeargs… fields…` — the value this equation's scrutinee is fixed to.
        let mut ctor_value = Term::Global(ctor_name.clone());
        for ta in &type_args {
            ctor_value = Term::App(Box::new(ctor_value), Box::new(ta.clone()));
        }
        for (n, _) in &field_binders {
            ctor_value = Term::App(Box::new(ctor_value), Box::new(Term::Var(n.clone())));
        }

        // The binders of the equation: everything before the scrutinee, the constructor's
        // fields in its place, and everything after it with the scrutinee already fixed.
        let mut lemma_binders: Vec<(String, Term)> = binders[..position].to_vec();
        lemma_binders.extend(field_binders.iter().cloned());
        for (n, t) in &binders[position + 1..] {
            lemma_binders.push((n.clone(), substitute(t, scrutinee, &ctor_value)));
        }

        let lemma_args: Vec<Term> = binders
            .iter()
            .map(|(n, _)| {
                if n == scrutinee {
                    ctor_value.clone()
                } else {
                    Term::Var(n.clone())
                }
            })
            .collect();
        let lhs = apply_all(head.clone(), &lemma_args);
        let rhs = apply_case(
            &cases[i],
            &field_binders
                .iter()
                .map(|(n, _)| Term::Var(n.clone()))
                .collect::<Vec<_>>(),
        );
        let carrier = substitute(&result_ty, scrutinee, &ctor_value);

        out.push((
            format!("{name}{EQUATION_INFIX}{}", i + 1),
            pi_over(&lemma_binders, eq_prop(&carrier, &lhs, &rhs)),
            lam_over(&lemma_binders, refl_proof(&carrier, &lhs)),
        ));
    }

    out
}

/// Generate and REGISTER the equation lemmas a definition owes.
///
/// Every candidate goes through inference and the checked declaration gateway.
/// Failed optional candidates are omitted; they cannot poison a valid source
/// declaration or enter the pending-obligation/refusal ledger. Existing names,
/// including pending user definitions, are reserved. Successful lemmas share the
/// existing auxiliary ownership bookkeeping so source replacement invalidates them.
pub fn generate(ctx: &mut Context, name: &str) {
    if ctx.equations_suspended()
        || is_equation_name(name)
        || ctx.get_global("Eq").is_none()
        || ctx.get_global("refl").is_none()
    {
        return;
    }
    let Some((ty, body)) = ctx
        .get_definition_type(name)
        .cloned()
        .zip(ctx.get_definition_body(name).cloned())
    else {
        return;
    };
    for (lemma, statement, proof) in equation_lemmas(ctx, name, &ty, &body) {
        // Never displace: a name already taken keeps whatever it holds, whether that is a user's
        // lemma or an earlier candidate for this same suffix.
        if ctx.constant_name_reserved(&lemma) {
            continue;
        }
        // Check exactly once, through the ordinary gateway with deferral
        // disabled. A failed optional candidate creates neither an assumption
        // nor a pending obligation, and leaves the refusal ledger unchanged.
        if ctx
            .declare_checked_auxiliary(&lemma, statement, proof)
            .is_ok()
        {
            ctx.record_derived_aux(name, &lemma);
        }
    }
}

/// Back-fill every definition that was registered before `Eq`/`refl` existed.
///
/// The prelude declares hundreds of definitions before `register_eq` runs, and none of them can
/// state an equation until there is an equality to state it in. Call this once at the end of
/// `StandardLibrary::register`.
pub fn generate_missing_for_all(ctx: &mut Context) {
    if ctx.equations_suspended()
        || ctx.get_global("Eq").is_none()
        || ctx.get_global("refl").is_none()
    {
        return;
    }
    let names: Vec<String> = ctx
        .iter_definitions()
        .map(|(n, _, _)| n.to_string())
        .filter(|n| !is_equation_name(n))
        .collect();
    for n in names {
        generate(ctx, &n);
    }
}
