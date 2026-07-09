//! R2 — auto-derived recursors (dependent eliminators) for inductive types.
//!
//! Today a user who declares an inductive must hand-write the `match`/`fix` to recurse
//! over it. Lean/Coq instead AUTO-GENERATE the recursor `I.rec` (the dependent
//! eliminator) from the declaration, so "declare `ℕ`, get induction for free." This
//! module is that derivation: from an inductive's registered constructors it synthesizes
//!
//! ```text
//! I_rec : Π(P : I → Type). minor₀ → … → minorₖ → Π(x : I). P x
//! I_rec := λP. λf₀ … λfₖ. fix rec. λx. match x return (λx. P x) with
//!            | Cᵢ a… => fᵢ a… (rec aⱼ)…        -- one rec-call per recursive argument
//! ```
//!
//! where each minor premise is `Π(args). Π(IH : P aⱼ for each recursive arg). P (Cᵢ args)`.
//! The synthesized term is an ordinary kernel `Term`, so it is re-checked by `infer_type`
//! for coverage + termination — and (the point of building it now) independently
//! re-derived by the `recheck` second kernel. A Prop motive may be
//! passed wherever the `Type` motive is expected, by cumulativity (`Prop ≤ Type 0`), so
//! the single derived recursor serves BOTH computation and induction.
//!
//! Scope: parametric AND indexed inductive families. Beyond the uniform PARAMETERS of a
//! type like `List A`, an indexed family has INDICES that vary per constructor — `Eq A x :
//! A → Prop` with `refl : Eq A x x`. Their eliminator's motive abstracts over the indices,
//! so `derive_recursor("Eq")` synthesizes FULL Paulin-Mohring J:
//! `Π(A). Π(x:A). Π(P : Π(y:A). Eq A x y → Sort). P x (refl A x) → Π(y). Π(h:Eq A x y). P y h`
//! — the identity eliminator as a kernel-checked term, not an axiom.

use crate::context::Context;
use crate::error::{KernelError, KernelResult};
use crate::infer_type;
use crate::term::{Term, Universe};

/// Build `I_rec`'s term and type for the inductive `ind`. Returns `(recursor_type,
/// recursor_term)` where `recursor_type` is exactly `infer_type(recursor_term)` — so the
/// type is, by construction, the one the kernel certifies the term against.
pub fn derive_recursor(ctx: &Context, ind: &str) -> KernelResult<(Term, Term)> {
    // A member of a mutual block gets the mutual recursor (one shared `MutualFix`).
    if let Some(block) = ctx.mutual_block_of(ind) {
        let block = block.to_vec();
        return derive_mutual_recursor(ctx, ind, &block);
    }

    let ind_full_ty = ctx
        .get_global(ind)
        .ok_or_else(|| KernelError::UnboundVariable(ind.to_string()))?
        .clone();
    if !ctx.is_inductive(ind) {
        return Err(KernelError::UnboundVariable(ind.to_string()));
    }

    // Split the inductive's arity telescope into uniform PARAMETERS (the first
    // `num_params`) and INDICES (the rest). `List : Type → Type` is 1 param / 0 index;
    // `Eq : Π(A). A → A → Prop` is 2 params (`A`, `x`) / 1 index (`y`). Parameters are
    // bound once at the top as `A0 … A_{p-1}`; indices are re-abstracted inside the
    // recursion (`Idx0 … Idx_{k-1}`) because each constructor targets its own index.
    let (arity_tele, ind_sort) = peel_pis(&ind_full_ty);
    let num_params = ctx.inductive_num_params(ind).min(arity_tele.len());

    // The sort the motive targets. A `Prop` inductive may be eliminated into a larger sort
    // (`Type`) only if it is a subsingleton (0 constructors, or 1 with propositional args) —
    // so `Eq`/`False` get the full `Type` eliminator (J, ex falso), but a multi-constructor
    // proposition like `le` gets its `Prop` induction principle. Everything else eliminates
    // into `Type 0` (a `Prop` motive still passes there by cumulativity).
    let motive_target = match ind_sort {
        Term::Sort(Universe::Prop)
            if !crate::type_checker::is_subsingleton_prop(ctx, ind).unwrap_or(false) =>
        {
            Term::Sort(Universe::Prop)
        }
        _ => Term::Sort(Universe::Type(0)),
    };
    let param_tele: Vec<(String, Term)> = arity_tele[..num_params].to_vec();
    let index_tele: Vec<(String, Term)> = arity_tele[num_params..].to_vec();
    let k = index_tele.len();

    let tp_names: Vec<String> = (0..num_params).map(|i| format!("A{i}")).collect();
    let idx_names: Vec<String> = (0..k).map(|i| format!("Idx{i}")).collect();

    // Parameter binder types, closed over the earlier parameters' recursor names.
    let param_types: Vec<Term> = (0..num_params)
        .map(|i| rename_tele_types(&param_tele[i].1, &param_tele[..i], &tp_names))
        .collect();
    // Index binder types, closed over ALL parameters and the earlier indices — with the
    // given index binder names (the fix and the match motive use different names).
    let index_types = |names: &[String]| -> Vec<Term> {
        (0..k)
            .map(|j| {
                let with_params = rename_tele_types(&index_tele[j].1, &param_tele, &tp_names);
                rename_index_refs(&with_params, &index_tele[..j], names)
            })
            .collect::<Vec<_>>()
    };

    // `I A0 … A_{p-1}` (applied to the parameters only).
    let mut ind_params = Term::Global(ind.to_string());
    for name in &tp_names {
        ind_params = app(ind_params, var(name));
    }

    // The motive `P : Π(Idx0:T0)…Π(Idx_{k-1}). (I A… Idx…) → Type 0`. `Prop` motives pass
    // by cumulativity, so this one recursor serves computation and induction alike.
    let idx_types_motive = index_types(&idx_names);
    let mut ind_at_idx = ind_params.clone();
    for name in &idx_names {
        ind_at_idx = app(ind_at_idx, var(name));
    }
    let mut motive_ty = arrow(ind_at_idx.clone(), motive_target);
    for j in (0..k).rev() {
        motive_ty = pi(&idx_names[j], idx_types_motive[j].clone(), motive_ty);
    }

    let ctors: Vec<(String, Term)> = ctx
        .get_constructors(ind)
        .iter()
        .map(|(n, t)| (n.to_string(), (*t).clone()))
        .collect();

    let mut minors: Vec<(String, Term)> = Vec::with_capacity(ctors.len());
    let mut cases: Vec<Term> = Vec::with_capacity(ctors.len());
    // A standalone inductive is a mutual block of one: its only motive is `P`, its only
    // recursive name `rec`, and every recursive occurrence is of member 0 (itself).
    let block_single = [ind];
    let motive_single = ["P".to_string()];
    let rec_single = ["rec".to_string()];
    for (i, (cname, ctype)) in ctors.iter().enumerate() {
        let an = constructor_analysis(ctype, ind, &block_single, num_params, &tp_names)?;
        minors.push((format!("f{i}"), minor_type("P", &motive_single, cname, &tp_names, &an)));
        cases.push(case_term(cname, &tp_names, &an, &format!("f{i}"), &rec_single));
    }

    // The match's return clause `λj0…λj_{k-1}. λx:(I A… j…). P j… x` — the motive
    // re-abstracted with FRESH index binders so applying it at the discriminant's own
    // indices captures nothing.
    let mj_names: Vec<String> = (0..k).map(|i| format!("mj{i}")).collect();
    let idx_types_match = index_types(&mj_names);
    let mut ind_at_mj = ind_params.clone();
    for name in &mj_names {
        ind_at_mj = app(ind_at_mj, var(name));
    }
    let mut motive_body = var("P");
    for name in &mj_names {
        motive_body = app(motive_body, var(name));
    }
    motive_body = app(motive_body, var("x"));
    let mut motive_fn = lam("x", ind_at_mj, motive_body);
    for j in (0..k).rev() {
        motive_fn = lam(&mj_names[j], idx_types_match[j].clone(), motive_fn);
    }

    // fix rec. λIdx0…λIdx_{k-1}. λx:(I A… Idx…). match x return motive_fn with { cases }
    let match_term = Term::Match {
        discriminant: Box::new(var("x")),
        motive: Box::new(motive_fn),
        cases,
    };
    let mut fix_body = lam("x", ind_at_idx.clone(), match_term);
    for j in (0..k).rev() {
        fix_body = lam(&idx_names[j], idx_types_motive[j].clone(), fix_body);
    }
    let mut term = Term::Fix { name: "rec".to_string(), body: Box::new(fix_body) };

    // Wrap `λfₙ … λf₀`, then `λP`, then the parameters `λA_{p-1} … λA₀`.
    for (fname, fty) in minors.iter().rev() {
        term = lam(fname, fty.clone(), term);
    }
    term = lam("P", motive_ty, term);
    for i in (0..num_params).rev() {
        term = lam(&tp_names[i], param_types[i].clone(), term);
    }

    // The recursor's type is, by construction, the type the kernel certifies for it —
    // coverage and termination included, in both kernels.
    let ty = infer_type(ctx, &term)?;
    Ok((ty, term))
}

/// Derive the recursor for `ind`, a member of the MUTUAL block `block`. Every member
/// shares one `MutualFix`; this returns `(type, term)` for `ind`'s recursor, whose body
/// selects `ind`'s component. The eliminator takes ONE motive per member and ALL
/// members' minor premises; a recursive occurrence of member `k` in any constructor
/// uses motive `P_k` for its induction hypothesis and calls `rec_k`.
///
/// Scope: mutual blocks with shared uniform PARAMETERS (`Tree A`/`Forest A`) and per-member
/// indices are both supported; every member must agree on the parameter count.
fn derive_mutual_recursor(ctx: &Context, ind: &str, block: &[String]) -> KernelResult<(Term, Term)> {
    let m = block.len();
    let bi = block.iter().position(|n| n == ind).ok_or_else(|| {
        KernelError::CertificationError(format!("'{ind}' is not in its own mutual block"))
    })?;
    let block_refs: Vec<&str> = block.iter().map(|s| s.as_str()).collect();

    // Shared PARAMETERS — a mutual block shares its uniform parameters (`Tree A`/`Forest A`).
    // Taken from the first member; every member must agree on the count.
    let block0_ty = ctx
        .get_global(&block[0])
        .ok_or_else(|| KernelError::UnboundVariable(block[0].clone()))?
        .clone();
    let (arity0, _) = peel_pis(&block0_ty);
    let num_params = ctx.inductive_num_params(&block[0]).min(arity0.len());
    for name in block {
        let full = ctx.get_global(name).ok_or_else(|| KernelError::UnboundVariable(name.clone()))?;
        if ctx.inductive_num_params(name).min(peel_pis(full).0.len()) != num_params {
            return Err(KernelError::CertificationError(format!(
                "auto-recursor: mutual block members disagree on parameter count ('{name}')"
            )));
        }
    }
    let param_tele: Vec<(String, Term)> = arity0[..num_params].to_vec();
    let tp_names: Vec<String> = (0..num_params).map(|i| format!("A{i}")).collect();
    let param_types: Vec<Term> = (0..num_params)
        .map(|i| rename_tele_types(&param_tele[i].1, &param_tele[..i], &tp_names))
        .collect();

    let motive_names: Vec<String> = (0..m).map(|k| format!("P{k}")).collect();
    let rec_names: Vec<String> = (0..m).map(|k| format!("rec{k}")).collect();

    // `I_k` applied to the shared parameters `A0 … A_{p-1}`.
    let ind_at_params = |name: &str| {
        tp_names.iter().fold(Term::Global(name.to_string()), |acc, n| app(acc, var(n)))
    };

    // Per-member skeleton: the index telescope, the motive type, and the applied form.
    struct MemberInfo {
        idx_names: Vec<String>,
        idx_types: Vec<Term>,
        ind_at_idx: Term,
        motive_ty: Term,
    }
    let mut infos: Vec<MemberInfo> = Vec::with_capacity(m);
    for (k, name) in block.iter().enumerate() {
        let full_ty = ctx
            .get_global(name)
            .ok_or_else(|| KernelError::UnboundVariable(name.clone()))?
            .clone();
        let (arity_tele, ind_sort) = peel_pis(&full_ty);
        let index_tele: Vec<(String, Term)> = arity_tele[num_params..].to_vec();
        let target = match ind_sort {
            Term::Sort(Universe::Prop)
                if !crate::type_checker::is_subsingleton_prop(ctx, name).unwrap_or(false) =>
            {
                Term::Sort(Universe::Prop)
            }
            _ => Term::Sort(Universe::Type(0)),
        };
        let idx_names: Vec<String> = (0..index_tele.len()).map(|j| format!("Idx{k}_{j}")).collect();
        // Index binder types: rename the shared params → `A_i`, and earlier index refs → `Idx…`.
        let idx_types: Vec<Term> = (0..index_tele.len())
            .map(|j| {
                let with_params = rename_tele_types(&index_tele[j].1, &param_tele, &tp_names);
                rename_index_refs(&with_params, &index_tele[..j], &idx_names)
            })
            .collect();
        // I_k A0 … A_{p-1} Idx0 … Idx_{k-1}
        let mut ind_at_idx = ind_at_params(name);
        for n in &idx_names {
            ind_at_idx = app(ind_at_idx, var(n));
        }
        // motive_k : Π(idx…). (I_k params idx…) → target  (params are bound by the outer λ)
        let mut motive_ty = arrow(ind_at_idx.clone(), target);
        for j in (0..idx_names.len()).rev() {
            motive_ty = pi(&idx_names[j], idx_types[j].clone(), motive_ty);
        }
        infos.push(MemberInfo { idx_names, idx_types, ind_at_idx, motive_ty });
    }

    // All minor premises (across every member, in block order) and, per member, the
    // `match` cases of its own fixpoint body.
    let mut minors: Vec<(String, Term)> = Vec::new();
    let mut member_cases: Vec<Vec<Term>> = Vec::with_capacity(m);
    for (k, name) in block.iter().enumerate() {
        let ctors: Vec<(String, Term)> = ctx
            .get_constructors(name)
            .iter()
            .map(|(n, t)| (n.to_string(), (*t).clone()))
            .collect();
        let mut cases = Vec::with_capacity(ctors.len());
        for (cname, ctype) in &ctors {
            let an = constructor_analysis(ctype, name, &block_refs, num_params, &tp_names)?;
            let fi = format!("f{}", minors.len());
            minors.push((
                fi.clone(),
                minor_type(&motive_names[k], &motive_names, cname, &tp_names, &an),
            ));
            cases.push(case_term(cname, &tp_names, &an, &fi, &rec_names));
        }
        member_cases.push(cases);
    }

    // Each member's fixpoint body: `λidx…. λx:(I_k params idx…). match x return motive_fn`.
    let mut defs: Vec<(String, Term)> = Vec::with_capacity(m);
    for (k, name) in block.iter().enumerate() {
        let info = &infos[k];
        // The match return clause `λmj…. λx. P_k mj… x`, fresh index binders.
        let mj_names: Vec<String> = (0..info.idx_names.len()).map(|j| format!("mj{k}_{j}")).collect();
        let mut motive_body = var(&motive_names[k]);
        for n in &mj_names {
            motive_body = app(motive_body, var(n));
        }
        motive_body = app(motive_body, var("x"));
        let mut ind_at_mj = ind_at_params(name);
        for n in &mj_names {
            ind_at_mj = app(ind_at_mj, var(n));
        }
        let mut motive_fn = lam("x", ind_at_mj, motive_body);
        for j in (0..mj_names.len()).rev() {
            // Re-express the index type with the FRESH match binders: `Idx_i → mj_i`.
            let mut mj_ty = info.idx_types[j].clone();
            for i in 0..j {
                mj_ty = subst_name(&mj_ty, &info.idx_names[i], &var(&mj_names[i]));
            }
            motive_fn = lam(&mj_names[j], mj_ty, motive_fn);
        }
        let match_term = Term::Match {
            discriminant: Box::new(var("x")),
            motive: Box::new(motive_fn),
            cases: member_cases[k].clone(),
        };
        let mut fix_body = lam("x", info.ind_at_idx.clone(), match_term);
        for j in (0..info.idx_names.len()).rev() {
            fix_body = lam(&info.idx_names[j], info.idx_types[j].clone(), fix_body);
        }
        defs.push((rec_names[k].clone(), fix_body));
    }

    // The whole recursor: `λA0…A_{p-1}. λP0…P_{m-1}. λf0…f_{last}. (mutualfix { rec_k := … }).bi`.
    let mut term = Term::MutualFix { defs, index: bi };
    for (fname, fty) in minors.iter().rev() {
        term = lam(fname, fty.clone(), term);
    }
    for k in (0..m).rev() {
        term = lam(&motive_names[k], infos[k].motive_ty.clone(), term);
    }
    for i in (0..num_params).rev() {
        term = lam(&tp_names[i], param_types[i].clone(), term);
    }

    let ty = infer_type(ctx, &term)?;
    Ok((ty, term))
}

/// A constructor's shape in the recursor's scope: the value-argument types, whether each is
/// recursive (and if so, at which INDEX arguments), and the index arguments of the
/// constructor's own result. Names are already the recursor's (`A0…`, `a0…`).
struct CtorAnalysis {
    value_types: Vec<Term>,
    /// `Some(occ)` if value argument `j` is a recursive occurrence — either the DIRECT
    /// form `I A… e…` (`occ.tele` empty) or the strictly-positive FUNCTIONAL form
    /// `Π(z:B…). I A… e…` (`occ.tele` = the `(z, B)` binders), as in `Acc_intro`'s
    /// `Π(y). R y x → Acc A R y`. `None` for a non-recursive argument.
    recursive: Vec<Option<RecOcc>>,
    /// The index arguments of the constructor's result `I A… e…` (empty when non-indexed).
    result_indices: Vec<Term>,
}

/// A recursive occurrence in a constructor argument: WHICH block member it is an
/// occurrence of (`0` for a standalone inductive — the whole block is itself), the
/// domain telescope of a functional argument (empty for a direct argument), and the
/// occurrence's index arguments (in scope of the telescope).
struct RecOcc {
    member: usize,
    tele: Vec<(String, Term)>,
    indices: Vec<Term>,
}

/// Analyse a constructor in the recursor's scope. Its leading `num_params` parameters are
/// the inductive's parameters (renamed to `A₀ … A_{p-1}`); the rest are value arguments
/// (renamed `a₀ … aₘ`). A value argument is recursive iff its (rewritten) type's head is
/// the inductive, and we record that occurrence's INDEX arguments. We also record the
/// constructor result's own index arguments — for `refl : Π(A). Π(x). Eq A x x` the value
/// arguments are empty and the result index is `[x]`, so the refl case demands `P x (refl
/// A x)`.
fn constructor_analysis(
    ctor_type: &Term,
    owner: &str,
    block: &[&str],
    num_params: usize,
    tp_names: &[String],
) -> KernelResult<CtorAnalysis> {
    let (params, residual) = peel_pis(ctor_type);
    if params.len() < num_params {
        return Err(KernelError::CertificationError(format!(
            "auto-recursor: constructor of '{}' has fewer parameters than the inductive",
            owner
        )));
    }
    if head_global(residual) != Some(owner) {
        return Err(KernelError::CertificationError(format!(
            "auto-recursor: constructor result {} is not the inductive '{}'",
            residual, owner
        )));
    }
    let (type_params, value_params) = params.split_at(num_params);

    // Rewrite a term from the constructor's scope to the recursor's: parameter names →
    // `A_i`, and the first `upto` value names → `a_k`.
    let rewrite = |t: &Term, upto: usize| -> Term {
        let mut out = t.clone();
        for (i, (tp_orig, _)) in type_params.iter().enumerate() {
            if tp_orig != "_" {
                out = subst_name(&out, tp_orig, &var(&tp_names[i]));
            }
        }
        for (kk, (vk_orig, _)) in value_params.iter().enumerate().take(upto) {
            if vk_orig != "_" {
                out = subst_name(&out, vk_orig, &var(&format!("a{kk}")));
            }
        }
        out
    };

    let mut value_types = Vec::with_capacity(value_params.len());
    let mut recursive = Vec::with_capacity(value_params.len());
    for (j, (_, ty)) in value_params.iter().enumerate() {
        let t = rewrite(ty, j);
        // Peel any functional telescope: a strictly-positive argument may be
        // `Π(z:B…). I A… e…` (Acc's `Π(y). R y x → Acc A R y`) as well as the
        // direct `I A… e…`. The occurrence is recursive iff, after peeling, the
        // head is the inductive.
        let (tele, occ) = peel_pis(&t);
        // Recursive iff the occurrence's head is ANY block member; record which one so
        // the minor premise's IH and the recursive call route to that member's motive/fix.
        if let Some(member) = head_global(occ).and_then(|h| block.iter().position(|m| *m == h)) {
            let indices: Vec<Term> = app_args(occ).into_iter().skip(num_params).collect();
            recursive.push(Some(RecOcc { member, tele, indices }));
        } else {
            recursive.push(None);
        }
        value_types.push(t);
    }

    // The constructor result's own index arguments, closed over all value arguments.
    let result_indices: Vec<Term> = app_args(residual)
        .into_iter()
        .skip(num_params)
        .map(|e| rewrite(&e, value_params.len()))
        .collect();

    Ok(CtorAnalysis { value_types, recursive, result_indices })
}

/// The minor-premise type for one constructor:
/// `Π(a₀:T₀)…Π(aₘ:Tₘ). Π(IHⱼ : P eⱼ… aⱼ per recursive arg). P r… (C A… a₀ … aₘ)`,
/// where `r…` are the constructor's result indices and `eⱼ…` the recursive occurrence's.
fn minor_type(
    owner_motive: &str,
    motive_names: &[String],
    cname: &str,
    tp_names: &[String],
    an: &CtorAnalysis,
) -> Term {
    let mut ctor_applied = Term::Global(cname.to_string());
    for name in tp_names {
        ctor_applied = app(ctor_applied, var(name));
    }
    for j in 0..an.value_types.len() {
        ctor_applied = app(ctor_applied, var(&format!("a{j}")));
    }
    // The OWNER's motive applied to the constructor's result indices, then the value.
    let mut body = var(owner_motive);
    for e in &an.result_indices {
        body = app(body, e.clone());
    }
    body = app(body, ctor_applied);
    // One induction hypothesis per recursive argument, using the motive of the block
    // MEMBER the occurrence is of (self or sibling). For a DIRECT occurrence it is
    // `P_member (its indices) aⱼ`; for a FUNCTIONAL occurrence `aⱼ : Π(z:B…). I e…`
    // it is `Π(z:B…). P_member (its indices) (aⱼ z…)`.
    for j in (0..an.value_types.len()).rev() {
        if let Some(occ) = &an.recursive[j] {
            let mut ih = var(&motive_names[occ.member]);
            for e in &occ.indices {
                ih = app(ih, e.clone());
            }
            let mut aj = var(&format!("a{j}"));
            for (tn, _) in &occ.tele {
                aj = app(aj, var(tn));
            }
            ih = app(ih, aj);
            for (tn, tt) in occ.tele.iter().rev() {
                ih = pi(tn, tt.clone(), ih);
            }
            body = pi(&format!("ih{j}"), ih, body);
        }
    }
    for j in (0..an.value_types.len()).rev() {
        body = pi(&format!("a{j}"), an.value_types[j].clone(), body);
    }
    body
}

/// The `match` case term for one constructor (binds only the VALUE arguments — the
/// parameters are fixed by the discriminant):
/// `λa₀ … λaₘ. fᵢ a₀ … aₘ (rec eⱼ… aⱼ)…`, where the recursive call carries the recursive
/// occurrence's index arguments so it lands at the right motive instance.
fn case_term(
    cname: &str,
    _tp_names: &[String],
    an: &CtorAnalysis,
    fi: &str,
    rec_names: &[String],
) -> Term {
    let _ = cname;
    let mut body = var(fi);
    for j in 0..an.value_types.len() {
        body = app(body, var(&format!("a{j}")));
    }
    for j in 0..an.value_types.len() {
        if let Some(occ) = &an.recursive[j] {
            // DIRECT: `rec (its indices) aⱼ`. FUNCTIONAL: `λ(z:B…). rec (its
            // indices) (aⱼ z…)` — recurse on each sub-structure the accessibility
            // function yields. The recursive call's structural argument `aⱼ z…`
            // is an APPLICATION of the smaller field `aⱼ`, which the guard admits
            // (an application headed by a smaller variable is smaller). The call routes
            // to the fixpoint of the block MEMBER the occurrence is of.
            let mut call = var(&rec_names[occ.member]);
            for e in &occ.indices {
                call = app(call, e.clone());
            }
            let mut aj = var(&format!("a{j}"));
            for (tn, _) in &occ.tele {
                aj = app(aj, var(tn));
            }
            call = app(call, aj);
            for (tn, tt) in occ.tele.iter().rev() {
                call = lam(tn, tt.clone(), call);
            }
            body = app(body, call);
        }
    }
    for j in (0..an.value_types.len()).rev() {
        body = lam(&format!("a{j}"), an.value_types[j].clone(), body);
    }
    body
}

/// Rewrite an inductive-parameter binder type from the declaration's scope into the
/// recursor's: each of the earlier parameter binders `tele[i]` becomes `names[i]`.
fn rename_tele_types(ty: &Term, tele: &[(String, Term)], names: &[String]) -> Term {
    let mut out = ty.clone();
    for (i, (orig, _)) in tele.iter().enumerate() {
        if orig != "_" {
            out = subst_name(&out, orig, &var(&names[i]));
        }
    }
    out
}

/// Rewrite an index binder type's references to the EARLIER index binders `tele[i]` to the
/// recursor-scope `names[i]` (parameters must already be renamed).
fn rename_index_refs(ty: &Term, tele: &[(String, Term)], names: &[String]) -> Term {
    rename_tele_types(ty, tele, names)
}

/// The argument spine of an application `f a b c` → `[a, b, c]` (`[]` for a non-application).
fn app_args(t: &Term) -> Vec<Term> {
    let mut args = Vec::new();
    let mut cur = t;
    while let Term::App(f, a) = cur {
        args.push((**a).clone());
        cur = f;
    }
    args.reverse();
    args
}

/// Peel a term's leading `Π`s into `(name, type)` pairs plus the residual.
fn peel_pis(t: &Term) -> (Vec<(String, Term)>, &Term) {
    let mut params = Vec::new();
    let mut cur = t;
    while let Term::Pi { param, param_type, body_type } = cur {
        params.push((param.clone(), (**param_type).clone()));
        cur = body_type;
    }
    (params, cur)
}

/// The head `Global` of an application spine, if any (`List A B` → `List`).
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

/// Substitute the free variable `old` by `repl` (binder-respecting).
fn subst_name(t: &Term, old: &str, repl: &Term) -> Term {
    match t {
        Term::Var(n) if n == old => repl.clone(),
        Term::Var(_) | Term::Global(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole
        | Term::Const { .. } => t.clone(),
        Term::Pi { param, param_type, body_type } => Term::Pi {
            param: param.clone(),
            param_type: Box::new(subst_name(param_type, old, repl)),
            body_type: if param == old {
                body_type.clone()
            } else {
                Box::new(subst_name(body_type, old, repl))
            },
        },
        Term::Lambda { param, param_type, body } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(subst_name(param_type, old, repl)),
            body: if param == old { body.clone() } else { Box::new(subst_name(body, old, repl)) },
        },
        Term::App(f, a) => {
            Term::App(Box::new(subst_name(f, old, repl)), Box::new(subst_name(a, old, repl)))
        }
        Term::Match { discriminant, motive, cases } => Term::Match {
            discriminant: Box::new(subst_name(discriminant, old, repl)),
            motive: Box::new(subst_name(motive, old, repl)),
            cases: cases.iter().map(|c| subst_name(c, old, repl)).collect(),
        },
        Term::Fix { name, body } => Term::Fix {
            name: name.clone(),
            body: if name == old { body.clone() } else { Box::new(subst_name(body, old, repl)) },
        },
        Term::MutualFix { defs, index } => {
            // Every def name binds in every body; `old` is shadowed iff it matches any.
            if defs.iter().any(|(n, _)| n == old) {
                t.clone()
            } else {
                Term::MutualFix {
                    defs: defs.iter().map(|(n, b)| (n.clone(), subst_name(b, old, repl))).collect(),
                    index: *index,
                }
            }
        }
        Term::Let { name, ty, value, body } => Term::Let {
            name: name.clone(),
            ty: Box::new(subst_name(ty, old, repl)),
            value: Box::new(subst_name(value, old, repl)),
            body: if name == old { body.clone() } else { Box::new(subst_name(body, old, repl)) },
        },
    }
}

// --- small term builders ---
fn var(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn lam(p: &str, ty: Term, body: Term) -> Term {
    Term::Lambda { param: p.to_string(), param_type: Box::new(ty), body: Box::new(body) }
}
fn pi(p: &str, ty: Term, body: Term) -> Term {
    Term::Pi { param: p.to_string(), param_type: Box::new(ty), body_type: Box::new(body) }
}
fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}
