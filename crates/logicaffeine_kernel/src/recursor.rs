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
    derive_recursor_at(ctx, ind, None)
}

/// Build `I_rec.{u}` — the UNIVERSE-POLYMORPHIC eliminator, `Π(P : I → Sort u). … → Π(x). P x`
/// with `u` an explicit level PARAMETER. Returns `(level_params, recursor_type, recursor_term)`.
///
/// This is the other half of the motive-sort rule (see `motive_target`): monomorphic and
/// polymorphic are two DECLARATIONS, not two spellings of one. `derive_recursor` gives an
/// inductive an eliminator at its own concrete sort — the one a `Type 1` inductive needs to
/// recover the `Type 0` its constructor stores — and this gives Lean's `Nat.rec.{u}`, the one a
/// `Type 1`-valued family built by recursion over `Nat` needs. A single artefact cannot be both:
/// a level variable in the motive's target makes `Type 0 ⋠ u` (a variable ranges over `Prop`),
/// so the concrete instance is lost the moment the polymorphic one is claimed.
///
/// Register it either way and both reference forms work:
/// * [`Context::add_universe_poly`] + `Term::Const { name, levels }` — Lean's explicit
///   `Nat.rec.{1}`; the level telescope is checked positionally by `term::resolve_const_levels`.
/// * [`Context::add_definition`] + a bare `Term::Global` — the application rule turns the free
///   level variable into a metavariable and solves it from the motive actually supplied
///   (`type_checker.rs::implicit_levels_for_head`).
///
/// The soundness gates are NOT parameterised: a non-subsingleton `Prop` still targets `Prop`, a
/// subsingleton `Prop` still targets `Type 0`, `SProp` still targets `SProp`, and an inductive
/// whose own sort is already a level variable keeps that variable rather than gaining a second,
/// independently-instantiable one. A sort provably above Prop (including symbolic successors/maxima) gets an
/// independent motive level.
///
/// The returned telescope COVERS the eliminator's type, in Lean's order (`Eq.rec.{u, u_1}`): the
/// motive's own level first, then the inductive's levels — its declared universe parameters, in
/// order and including any unused one, or else the levels its arity auto-binds, in first-occurrence
/// order — then any level the type still mentions (a mutual sibling's). The explicit door
/// refuses a level variable no parameter binds, so only a covering telescope keeps the promise
/// that both reference forms above work.
pub fn derive_recursor_universe_poly(
    ctx: &Context,
    ind: &str,
) -> KernelResult<(Vec<String>, Term, Term)> {
    let count = ctx.mutual_block_of(ind).map_or(1, |b| b.len());
    let levels = fresh_motive_levels(ctx, ind, count);
    let (ty, term) = derive_recursor_at(ctx, ind, Some(&levels))?;
    // Only the motive levels the motive-sort rule actually USED are parameters of this
    // declaration: a `Prop` member's motive keeps `Prop`, so promising a level telescope entry
    // for it would let a caller instantiate a parameter that appears nowhere — a level arity
    // mismatch the `Const` door would then have to accept or refuse for reasons the type does not
    // show.
    let used = crate::term::free_level_vars(&ty);
    let mut levels: Vec<String> = levels.into_iter().filter(|l| used.contains(l)).collect();
    let own = match ctx.poly_declaration(ind) {
        Some((declared, _)) => declared.to_vec(),
        None => ctx
            .get_global(ind)
            .map(crate::term::free_level_vars)
            .unwrap_or_default(),
    };
    for level in own.into_iter().chain(used) {
        if !levels.contains(&level) {
            levels.push(level);
        }
    }
    Ok((levels, ty, term))
}

/// The shared builder. `motive_levels` is `None` for the monomorphic eliminator and the block's
/// level-parameter names for the polymorphic one.
fn derive_recursor_at(
    ctx: &Context,
    ind: &str,
    motive_levels: Option<&[String]>,
) -> KernelResult<(Term, Term)> {
    // A member of a mutual block gets the mutual recursor (one shared `MutualFix`).
    if let Some(block) = ctx.mutual_block_of(ind) {
        let block = block.to_vec();
        return derive_mutual_recursor(ctx, ind, &block, motive_levels);
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

    let motive_target = motive_target_at(ctx, ind, ind_sort, motive_levels.and_then(|l| l.first()));
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
    let mut ind_params = declaration_head(ctx, ind);
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
    let mut recursive = false;
    let block_single = [ind];
    let motive_single = ["P".to_string()];
    let rec_single = ["rec".to_string()];
    for (i, (cname, ctype)) in ctors.iter().enumerate() {
        let an = constructor_analysis(ctx, ctype, ind, &block_single, num_params, &tp_names)?;
        recursive |= an.recursive.iter().any(Option::is_some);
        minors.push((
            format!("f{i}"),
            minor_type(ctx, "P", &motive_single, cname, &tp_names, &an),
        ));
        cases.push(case_term(
            cname,
            &tp_names,
            &an,
            &format!("f{i}"),
            &rec_single,
        ));
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
    let mut term = if recursive {
        Term::Fix {
            name: "rec".to_string(),
            body: Box::new(fix_body),
        }
    } else {
        fix_body
    };

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
fn derive_mutual_recursor(
    ctx: &Context,
    ind: &str,
    block: &[String],
    motive_levels: Option<&[String]>,
) -> KernelResult<(Term, Term)> {
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
        let full = ctx
            .get_global(name)
            .ok_or_else(|| KernelError::UnboundVariable(name.clone()))?;
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
        tp_names
            .iter()
            .fold(declaration_head(ctx, name), |acc, n| app(acc, var(n)))
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
        // The SAME rule as the standalone derivation — a block member must not get a weaker
        // eliminator for being declared in company. This arm used to pin `Type 0`, which
        // silently capped a `Type 1` member's motive one level below its own fields.
        let target = motive_target_at(ctx, name, ind_sort, motive_levels.and_then(|l| l.get(k)));
        let idx_names: Vec<String> = (0..index_tele.len())
            .map(|j| format!("Idx{k}_{j}"))
            .collect();
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
        infos.push(MemberInfo {
            idx_names,
            idx_types,
            ind_at_idx,
            motive_ty,
        });
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
            let an = constructor_analysis(ctx, ctype, name, &block_refs, num_params, &tp_names)?;
            let fi = format!("f{}", minors.len());
            minors.push((
                fi.clone(),
                minor_type(ctx, &motive_names[k], &motive_names, cname, &tp_names, &an),
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
        let mj_names: Vec<String> = (0..info.idx_names.len())
            .map(|j| format!("mj{k}_{j}"))
            .collect();
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

// =====================================================================================
// R3 — the DERIVED AUXILIARIES
//
// Lean synthesises, at declaration time and for EVERY inductive, a fixed family of
// auxiliaries besides the eliminator: `noConfusionType`/`noConfusion` (constructor
// disjointness and injectivity — what every `injection`, `cases h` on an equation and
// `simp [injEq]` proof actually cites), `below`/`brecOn` (course-of-values recursion — what
// every `| n+2 => …` definition compiles through), and `DecidableEq` instances. None of them
// may be an AXIOM: axiomatising `noConfusion` turns every `injection` proof in a development
// into an assumption. Everything below is therefore a TERM, checked by `infer_type` (and so
// re-derivable by the `recheck` second kernel) exactly as `derive_recursor` is.
//
// Like `derive_recursor`, these functions NEVER touch the caller's context: each returns
// `(type, term)` and the caller registers. Where one auxiliary is stated in terms of another
// (`noConfusion` cites `noConfusionType`, `brecOn` cites `below`), the derivation checks
// against a private extension of the context and the returned type mentions the cited name —
// so the caller must register the whole group, in the order `derive_auxiliaries` returns.
// =====================================================================================

/// One derived auxiliary: the name Lean gives it, the type the kernel certified for it, and
/// the term it certified.
#[derive(Debug, Clone, PartialEq)]
pub struct DerivedAux {
    /// The constant's name, in this kernel's flat spelling (`Chain_noConfusion`, `clink_inj`).
    pub name: String,
    /// `infer_type(term)` — the type the kernel certified, not a separately written claim.
    pub ty: Term,
    /// The definition's body.
    pub term: Term,
}

/// Everything the declaration of `ind` should bring with it, in DEPENDENCY ORDER, for a
/// caller to register verbatim:
///
/// ```text
/// for aux in derive_auxiliaries(&ctx, name) {
///     ctx.add_definition(aux.name, aux.ty, aux.term);
/// }
/// ```
///
/// The list is BEST-EFFORT: an auxiliary that this kernel genuinely cannot state for this
/// inductive (an indexed family with no index unifier, a `Prop` where disjointness is a
/// category error, a carrier above `Type 0` where `Eq` cannot even be applied) is OMITTED
/// rather than faked. Registration is therefore never blocked by a shape we do not reach, and
/// what IS returned was type-checked. Call the individual `derive_*` functions when you want
/// the REASON one is missing — each returns a `CertificationError` naming it.
///
/// It also never DISPLACES: a name already in `ctx` is left alone. Registration overwrites, so
/// without this the umbrella run over a prelude type would silently replace the hand-written,
/// separately-tested `decEqNat` with a freshly derived one — same intent, different term, and
/// nothing would have reported the swap.
pub fn derive_auxiliaries(ctx: &Context, ind: &str) -> Vec<DerivedAux> {
    let mut env = ctx.clone();
    let mut out: Vec<DerivedAux> = Vec::new();
    let mut generic = generic_family(ctx, ind);

    // The eliminator first: everything else is stated over the same constructor telescope, and
    // every existing caller already registers it under exactly this name.
    // The eliminator, derived ONCE and then RESHAPED twice. `recOn` and `casesOn` are the two
    // argument orders every Lean development actually writes — `cases h` compiles to `casesOn`,
    // `induction … using` and `match` elaborate through one of them, and
    // `Lean/Elab/Inductive.lean:729-730` generates both at the declaration, immediately after the
    // kernel has added `rec`. Sharing the derivation is not only thrift: three independent calls
    // would be three chances for the motive-sort rule to be asked differently.
    let elim = derive_recursor(ctx, ind);
    if let Ok(rec_pair) = &elim {
        push_aux(
            ctx,
            &mut env,
            &mut out,
            &mut generic,
            format!("{ind}_rec"),
            &|_e: &Context| Ok(rec_pair.clone()),
        );
        push_aux(
            ctx,
            &mut env,
            &mut out,
            &mut generic,
            format!("{ind}_recOn"),
            &|e| rec_on_from(e, ind, rec_pair.clone()),
        );
        push_aux(
            ctx,
            &mut env,
            &mut out,
            &mut generic,
            format!("{ind}_casesOn"),
            &|e| cases_on_from(e, ind, rec_pair.clone()),
        );
    }
    push_aux(
        ctx,
        &mut env,
        &mut out,
        &mut generic,
        format!("{ind}_noConfusionType"),
        &|e| derive_no_confusion_type(e, ind),
    );
    push_aux(
        ctx,
        &mut env,
        &mut out,
        &mut generic,
        format!("{ind}_noConfusion"),
        &|e| derive_no_confusion(e, ind),
    );

    let ctor_names: Vec<String> = ctx
        .get_constructors(ind)
        .iter()
        .map(|(n, _)| n.to_string())
        .collect();
    for c in &ctor_names {
        push_aux(
            ctx,
            &mut env,
            &mut out,
            &mut generic,
            format!("{c}_inj"),
            &|e| derive_ctor_inj(e, ind, c),
        );
    }

    push_aux(
        ctx,
        &mut env,
        &mut out,
        &mut generic,
        format!("{ind}_below"),
        &|e| derive_below(e, ind),
    );
    push_aux(
        ctx,
        &mut env,
        &mut out,
        &mut generic,
        format!("{ind}_brecOn"),
        &|e| derive_brec_on(e, ind),
    );
    push_aux(
        ctx,
        &mut env,
        &mut out,
        &mut generic,
        format!("decEq{ind}"),
        &|e| derive_dec_eq(e, ind),
    );

    out
}

/// `term` with every bare reference to one of `family`'s names written at the generic instance
/// `name.{levels…}`.
///
/// The derivations state a family's auxiliaries at the family's own instance, naming it bare. For
/// a universe-polymorphic family that instance is the one its telescope's variables denote, so the
/// auxiliaries are registered over that telescope with the references written out: an auxiliary is
/// then polymorphic as Lean's (`Big.below.{u}`) and serves every instance of the family, where a
/// bare reference would leave its level a variable nothing instantiates.
pub(crate) fn at_generic_levels(
    term: &Term,
    family: &std::collections::HashSet<String>,
    levels: &[String],
) -> Term {
    let go = |t: &Term| Box::new(at_generic_levels(t, family, levels));
    match term {
        Term::Global(n) if family.contains(n) => Term::Const {
            name: n.clone(),
            levels: levels.iter().map(|l| Universe::Var(l.clone())).collect(),
        },
        Term::Global(_)
        | Term::Const { .. }
        | Term::Var(_)
        | Term::Sort(_)
        | Term::Lit(_)
        | Term::Hole => term.clone(),
        Term::App(f, a) => Term::App(go(f), go(a)),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Term::Pi {
            param: param.clone(),
            param_type: go(param_type),
            body_type: go(body_type),
        },
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: param.clone(),
            param_type: go(param_type),
            body: go(body),
        },
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: go(discriminant),
            motive: go(motive),
            cases: cases
                .iter()
                .map(|c| at_generic_levels(c, family, levels))
                .collect(),
        },
        Term::Fix { name, body } => Term::Fix {
            name: name.clone(),
            body: go(body),
        },
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs
                .iter()
                .map(|(n, b)| (n.clone(), at_generic_levels(b, family, levels)))
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
            ty: go(ty),
            value: go(value),
            body: go(body),
        },
        Term::Annotated { annotation, term } => Term::Annotated {
            annotation: annotation.clone(),
            term: go(term),
        },
    }
}

/// A universe-polymorphic family's telescope, and the names its auxiliaries write at the generic
/// instance: the block's members and their constructors, joined by each auxiliary as it is derived.
/// `None` for a family without levels.
pub(crate) fn generic_family(
    ctx: &Context,
    ind: &str,
) -> Option<(Vec<String>, std::collections::HashSet<String>)> {
    let telescope = ctx
        .poly_declaration(ind)
        .map(|(ps, _)| ps.to_vec())
        .filter(|ps| !ps.is_empty())?;
    let members = ctx
        .mutual_block_of(ind)
        .map(<[String]>::to_vec)
        .unwrap_or_else(|| vec![ind.to_string()]);
    let constructors: Vec<String> = members
        .iter()
        .flat_map(|m| {
            ctx.get_constructors(m)
                .into_iter()
                .map(|(c, _)| c.to_string())
        })
        .collect();
    Some((telescope, members.into_iter().chain(constructors).collect()))
}

/// Derive one auxiliary unless its name is taken by a constant that is NOT this umbrella's own
/// prior generation, and make a success visible to the NEXT one (`noConfusion` is stated over
/// `noConfusionType`) — all in `env`, leaving the caller's context untouched.
///
/// The provenance test is what lets declaration-time synthesis work at all. "Never displace a
/// name already in scope" is the rule that protects the prelude's hand-written `decEqNat` from
/// being swapped for a same-intent, different term nothing would have reported — but once
/// auxiliaries are registered AT the declaration they are always already in scope, and a blanket
/// version of that rule turns the umbrella into a no-op on its second run. An inductive's
/// auxiliaries have to be re-derived after every constructor, because the ones derived after the
/// first describe an inductive that is not finished yet, so the umbrella must be able to replace
/// its OWN previous answer. [`Context::is_derived_aux`] is the record of which constants those
/// are; everything else is somebody's, and is left alone.
fn push_aux(
    ctx: &Context,
    env: &mut Context,
    out: &mut Vec<DerivedAux>,
    generic: &mut Option<(Vec<String>, std::collections::HashSet<String>)>,
    name: String,
    derive: &dyn Fn(&Context) -> KernelResult<(Term, Term)>,
) {
    if ctx.get_global(&name).is_some() && !ctx.is_derived_aux(&name) {
        return;
    }
    if let Ok((ty, term)) = derive(env) {
        // An auxiliary is stated as it will be registered, so the next one is typed against that:
        // a polymorphic family's own names at its generic instance (`at_generic_levels`), and every
        // other bare reference that is not a head at its own levels (`Context::stored_term`).
        let (ty, term) = match generic.as_mut() {
            Some((levels, family)) => {
                family.insert(name.clone());
                (
                    at_generic_levels(&ty, family, levels),
                    at_generic_levels(&term, family, levels),
                )
            }
            None => (ty, term),
        };
        let (ty, term) = (env.stored_term(ty), env.stored_term(term));
        // This environment exists only to type the next auxiliary. Its equation
        // products would be discarded, then checked again in the real Context.
        // The final registration still generates every public equation.
        env.with_equations_suspended(|env| match generic.as_ref() {
            Some((levels, _)) => {
                env.add_universe_poly(&name, levels.clone(), ty.clone(), term.clone())
            }
            None => env.add_definition(name.clone(), ty.clone(), term.clone()),
        });
        out.push(DerivedAux { name, ty, term });
    }
}

/// `I_noConfusionType : Π(A…). Π(P : Type 0). I A… → I A… → Type 0`
///
/// The type-level half of constructor disjointness, computed by cases on BOTH arguments:
///
/// ```text
/// noConfusionType P (C x⃗) (C y⃗) ⤳ (x⃗ = y⃗ → P) → P     -- the same constructor: injectivity
/// noConfusionType P (C x⃗) (D y⃗) ⤳ P                    -- different ones: anything follows
/// ```
///
/// Lean's is `{α} → Sort v → I α → I α → Sort v`; ours pins `P` to `Type 0` because the
/// prelude's `Eq` quantifies over a `Type 0` carrier, which is the same bound its companion
/// `noConfusion` runs into. A `Prop` goal still passes by cumulativity, which is the case
/// `cases h`/`injection` actually needs.
///
/// `Match` here is positional with no patterns and no wildcard, so an n-constructor inductive
/// genuinely spells out all n² arms — that is not a shortcut, it is the shape of the term.
pub fn derive_no_confusion_type(ctx: &Context, ind: &str) -> KernelResult<(Term, Term)> {
    let sc = AuxScope::new(ctx, ind)?;
    sc.require_equatable_carrier()?;
    require_global(
        ctx,
        "Eq",
        "the equation each same-constructor arm is built from",
    )?;

    let p_sort = Term::Sort(Universe::Type(0));
    let ret = lam("_", sc.carrier.clone(), p_sort.clone());

    let mut outer = Vec::with_capacity(sc.ctors.len());
    for (i, (ci, cti)) in sc.ctors.iter().enumerate() {
        let xs = ctor_value_fields(cti, ind, sc.num_params, &sc.tp_names, "x")?;
        let mut inner = Vec::with_capacity(sc.ctors.len());
        for (j, (_cj, ctj)) in sc.ctors.iter().enumerate() {
            let ys = ctor_value_fields(ctj, ind, sc.num_params, &sc.tp_names, "y")?;
            let arm = if i == j {
                homogeneous_fields(&xs, &ys, ci)?;
                // `(x₀ = y₀ → … → x_{m-1} = y_{m-1} → P) → P`. With m = 0 this degenerates to
                // `P → P`, which is the right answer: a nullary constructor equals itself and
                // the equation carries no information.
                let mut hyp = var("P");
                for k in (0..xs.len()).rev() {
                    hyp = arrow(eq3(xs[k].1.clone(), var(&xs[k].0), var(&ys[k].0)), hyp);
                }
                arrow(hyp, var("P"))
            } else {
                var("P")
            };
            inner.push(lams(&ys, arm));
        }
        outer.push(lams(
            &xs,
            Term::Match {
                discriminant: Box::new(var("b")),
                motive: Box::new(ret.clone()),
                cases: inner,
            },
        ));
    }

    let body = Term::Match {
        discriminant: Box::new(var("a")),
        motive: Box::new(ret),
        cases: outer,
    };
    let term = sc.close_lams(lam(
        "P",
        p_sort,
        lam("a", sc.carrier.clone(), lam("b", sc.carrier.clone(), body)),
    ));
    let ty = infer_type(ctx, &term)?;
    Ok((ty, term))
}

/// `I_noConfusion : Π(A…). Π(P : Type 0). Π(a b : I A…). Eq (I A…) a b → I_noConfusionType A… P a b`
///
/// The proof half. It is ONE transport: `noConfusionType P a a` is inhabited on the DIAGONAL
/// (for each constructor, `λk. k (refl x₀) … (refl x_{m-1})`), and J carries that along the
/// equation to `noConfusionType P a b`. Instantiating `P` gives both halves of what Lean's
/// `injection` and `cases h` use — `P := False` is constructor disjointness, `P := (xₖ = yₖ)`
/// is field-`k` injectivity (see [`derive_ctor_inj`]).
///
/// A definition, kernel-checked: nothing here is assumed.
pub fn derive_no_confusion(ctx: &Context, ind: &str) -> KernelResult<(Term, Term)> {
    let sc = AuxScope::new(ctx, ind)?;
    sc.require_equatable_carrier()?;
    require_global(ctx, "Eq", "the equation `noConfusion` consumes")?;
    require_global(ctx, "refl", "the diagonal arm's witness")?;
    require_global(
        ctx,
        "Eq_rec_dep",
        "the transport (J) that carries the diagonal along it",
    )?;

    let nct_name = format!("{ind}_noConfusionType");
    let mut env = ctx.clone();
    if env.get_global(&nct_name).is_none() {
        let (t, b) = derive_no_confusion_type(ctx, ind)?;
        env.add_definition(nct_name.clone(), t, b);
    }

    let p_sort = Term::Sort(Universe::Type(0));
    let nct_at = |u: Term, w: Term| {
        let mut t = gl(&nct_name);
        for n in &sc.tp_names {
            t = app(t, var(n));
        }
        app(app(app(t, var("P")), u), w)
    };

    // The DIAGONAL, by cases on `a`: at `C x⃗` the goal reduces to `(x⃗ = x⃗ → P) → P`, and the
    // hypothesis is discharged by one `refl` per field.
    let mut diag_cases = Vec::with_capacity(sc.ctors.len());
    for (cn, cty) in &sc.ctors {
        let xs = ctor_value_fields(cty, ind, sc.num_params, &sc.tp_names, "x")?;
        let ys = ctor_value_fields(cty, ind, sc.num_params, &sc.tp_names, "y")?;
        homogeneous_fields(&xs, &ys, cn)?;
        let mut hyp = var("P");
        for k in (0..xs.len()).rev() {
            hyp = arrow(eq3(xs[k].1.clone(), var(&xs[k].0), var(&xs[k].0)), hyp);
        }
        let mut applied = var("k");
        for (n, t) in &xs {
            applied = app(applied, app(app(gl("refl"), t.clone()), var(n)));
        }
        diag_cases.push(lams(&xs, lam("k", hyp, applied)));
    }
    let diagonal = Term::Match {
        discriminant: Box::new(var("a")),
        motive: Box::new(lam("z", sc.carrier.clone(), nct_at(var("z"), var("z")))),
        cases: diag_cases,
    };

    // `Eq_rec_dep (I A…) a (λy. λ_ : Eq (I A…) a y. noConfusionType A… P a y) diagonal b h`.
    let j_motive = lam(
        "y",
        sc.carrier.clone(),
        lam(
            "_",
            eq3(sc.carrier.clone(), var("a"), var("y")),
            nct_at(var("a"), var("y")),
        ),
    );
    let transported = apps(
        gl("Eq_rec_dep"),
        &[
            sc.carrier.clone(),
            var("a"),
            j_motive,
            diagonal,
            var("b"),
            var("h"),
        ],
    );
    let term = sc.close_lams(lam(
        "P",
        p_sort,
        lam(
            "a",
            sc.carrier.clone(),
            lam(
                "b",
                sc.carrier.clone(),
                lam(
                    "h",
                    eq3(sc.carrier.clone(), var("a"), var("b")),
                    transported,
                ),
            ),
        ),
    ));
    let ty = infer_type(&env, &term)?;
    Ok((ty, term))
}

/// `C_inj : Π(A…). Π(x⃗). Π(y⃗). Eq (I A…) (C A… x⃗) (C A… y⃗) → (x₀ = y₀ ∧ … ∧ x_{m-1} = y_{m-1})`
///
/// Lean's `C.inj` — the constant `injection h` produces. It is [`derive_no_confusion`] at
/// `P := the conjunction`, applied to the function that just repackages the equations, so it
/// needs no defaulted projection and therefore no inhabitant of the other constructors'
/// fields (the reason a hand-written `pred`-style projection cannot do this in general).
///
/// NOT derived here: Lean's `C.injEq`, which states the same fact as an EQUALITY of
/// propositions (`(C x⃗ = C y⃗) = (x⃗ = y⃗)`) and so needs `propext`. The implication above is
/// the propext-free content.
pub fn derive_ctor_inj(ctx: &Context, ind: &str, ctor: &str) -> KernelResult<(Term, Term)> {
    let sc = AuxScope::new(ctx, ind)?;
    sc.require_equatable_carrier()?;
    let cty = sc
        .ctors
        .iter()
        .find(|(n, _)| n == ctor)
        .map(|(_, t)| t.clone())
        .ok_or_else(|| unsupported(format!("'{ctor}' is not a constructor of '{ind}'")))?;
    let xs = ctor_value_fields(&cty, ind, sc.num_params, &sc.tp_names, "x")?;
    let ys = ctor_value_fields(&cty, ind, sc.num_params, &sc.tp_names, "y")?;
    homogeneous_fields(&xs, &ys, ctor)?;
    if xs.is_empty() {
        return Err(unsupported(format!(
            "'{ctor}' takes no arguments, so it has no injectivity principle to state"
        )));
    }
    if xs.len() > 1 {
        require_global(
            ctx,
            "And",
            "the conjunction of a multi-field constructor's equations",
        )?;
        require_global(ctx, "conj", "the conjunction's introduction rule")?;
    }
    let env = with_no_confusion(ctx, ind)?;

    let eqs: Vec<Term> = (0..xs.len())
        .map(|k| eq3(xs[k].1.clone(), var(&xs[k].0), var(&ys[k].0)))
        .collect();
    // `goals[k]` is the conjunction of `eqs[k..]`, right-nested as `And e_k goals[k+1]`.
    let mut goals: Vec<Term> = Vec::with_capacity(eqs.len());
    for k in (0..eqs.len()).rev() {
        let g_k = match goals.last() {
            None => eqs[k].clone(),
            Some(rest) => app(app(gl("And"), eqs[k].clone()), rest.clone()),
        };
        goals.push(g_k);
    }
    goals.reverse();

    // `λe₀ … λe_{m-1}. conj _ _ e₀ (conj _ _ e₁ (… e_{m-1}))`
    let mut payload = var(&format!("e{}", eqs.len() - 1));
    for k in (0..eqs.len() - 1).rev() {
        payload = apps(
            gl("conj"),
            &[
                eqs[k].clone(),
                goals[k + 1].clone(),
                var(&format!("e{k}")),
                payload,
            ],
        );
    }
    for k in (0..eqs.len()).rev() {
        payload = lam(&format!("e{k}"), eqs[k].clone(), payload);
    }

    let cx = sc.ctor_at(ctor, &xs);
    let cy = sc.ctor_at(ctor, &ys);
    let mut nc = gl(&format!("{ind}_noConfusion"));
    for n in &sc.tp_names {
        nc = app(nc, var(n));
    }
    let call = app(
        apps(nc, &[goals[0].clone(), cx.clone(), cy.clone(), var("h")]),
        payload,
    );

    let mut term = lam("h", eq3(sc.carrier.clone(), cx, cy), call);
    for (n, t) in ys.iter().rev() {
        term = lam(n, t.clone(), term);
    }
    for (n, t) in xs.iter().rev() {
        term = lam(n, t.clone(), term);
    }
    let term = sc.close_lams(term);
    let ty = infer_type(&env, &term)?;
    Ok((ty, term))
}

/// `I_below : Π(A…). Π(motive : I A… → Type 0). I A… → Type 0`
///
/// The course-of-values PACKAGE: `below motive t` is the type of "a `motive` value for every
/// subterm reachable below `t`", right-nested one entry per recursive field and closed with
/// `Unit`:
///
/// ```text
/// below motive (C x⃗) ⤳ Prod (Prod (motive xⱼ) (below motive xⱼ)) (… Unit)   -- recursive fields only
/// ```
///
/// Each entry is EXACTLY the type of `brecOn`'s internal `aux xⱼ`, which is what makes
/// [`derive_brec_on`]'s payload the recursive results themselves rather than a repacking. A
/// strictly-positive FUNCTIONAL field `xⱼ : Π(z:B…). I A…` contributes
/// `Π(z:B…). Prod (motive (xⱼ z…)) (below motive (xⱼ z…))`, as in `Acc`.
///
/// Lean's is `Sort (max 1 u)`-valued over `PProd`/`PUnit`; ours uses the prelude's `Prod`/`Unit`
/// and is therefore pinned to `Type 0`, the sort those live at.
pub fn derive_below(ctx: &Context, ind: &str) -> KernelResult<(Term, Term)> {
    let sc = AuxScope::new(ctx, ind)?;
    sc.require_no_mutual_block(ctx, "below")?;
    require_global(ctx, "Prod", "the pairing the below-package is built from")?;
    require_global(ctx, "Unit", "the empty tail of the below-package")?;

    let ty0 = Term::Sort(Universe::Type(0));
    let motive_ty = arrow(sc.carrier.clone(), ty0.clone());
    let below_at = |u: Term| app(var("bel"), u);

    let mut cases = Vec::with_capacity(sc.ctors.len());
    for (cn, cty) in &sc.ctors {
        let xs = ctor_value_fields(cty, ind, sc.num_params, &sc.tp_names, "x")?;
        let rfs = recursive_fields(&sc, &xs, cn)?;
        let entries: Vec<Term> = rfs.iter().map(|rf| entry_type(rf, &below_at)).collect();
        cases.push(lams(&xs, below_package_type(&entries)));
    }

    let body = Term::Fix {
        name: "bel".to_string(),
        body: Box::new(lam(
            "t",
            sc.carrier.clone(),
            Term::Match {
                discriminant: Box::new(var("t")),
                motive: Box::new(lam("_", sc.carrier.clone(), ty0)),
                cases,
            },
        )),
    };
    let term = sc.close_lams(lam("motive", motive_ty, body));
    let ty = infer_type(ctx, &term)?;
    Ok((ty, term))
}

/// `I_brecOn : Π(A…). Π(motive : I A… → Type 0). Π(t : I A…). Π(F : Π(s). I_below A… motive s → motive s). motive t`
///
/// Course-of-values RECURSION — the eliminator that hands the step function every subterm at
/// once, which is how `| n+2 => …` (and `halve`/`fib`/`pairSum`) is a structural definition
/// rather than a re-expressed mutual block. Built the way Lean builds it: one structural
/// recursion producing the PAIR `(motive s, below motive s)`, then the first projection.
pub fn derive_brec_on(ctx: &Context, ind: &str) -> KernelResult<(Term, Term)> {
    let sc = AuxScope::new(ctx, ind)?;
    sc.require_no_mutual_block(ctx, "brecOn")?;
    require_global(ctx, "Prod", "the pairing the recursion carries")?;
    require_global(ctx, "Prod_mk", "the pairing's introduction rule")?;
    require_global(
        ctx,
        "Prod_fst",
        "the projection that reads the answer back out",
    )?;
    require_global(ctx, "Unit", "the empty tail of the below-package")?;
    require_global(ctx, "Tt", "the empty below-package's inhabitant")?;

    let below_name = format!("{ind}_below");
    let mut env = ctx.clone();
    if env.get_global(&below_name).is_none() {
        let (t, b) = derive_below(ctx, ind)?;
        env.add_definition(below_name.clone(), t, b);
    }

    let ty0 = Term::Sort(Universe::Type(0));
    let motive_ty = arrow(sc.carrier.clone(), ty0);
    let below_at = |u: Term| {
        let mut t = gl(&below_name);
        for n in &sc.tp_names {
            t = app(t, var(n));
        }
        app(app(t, var("motive")), u)
    };
    // `F : Π(s : I A…). I_below A… motive s → motive s`
    let f_ty = pi(
        "s",
        sc.carrier.clone(),
        arrow(below_at(var("s")), app(var("motive"), var("s"))),
    );

    let mut cases = Vec::with_capacity(sc.ctors.len());
    for (cn, cty) in &sc.ctors {
        let xs = ctor_value_fields(cty, ind, sc.num_params, &sc.tp_names, "x")?;
        let rfs = recursive_fields(&sc, &xs, cn)?;
        let entries: Vec<(Term, Term)> = rfs
            .iter()
            .map(|rf| (entry_type(rf, &below_at), entry_value(rf, "aux")))
            .collect();
        let payload = below_package_value(&entries);
        let cx = sc.ctor_at(cn, &xs);
        // `Prod_mk (motive (C x⃗)) (below … (C x⃗)) (F (C x⃗) payload) payload`
        cases.push(lams(
            &xs,
            apps(
                gl("Prod_mk"),
                &[
                    app(var("motive"), cx.clone()),
                    below_at(cx.clone()),
                    app(app(var("F"), cx), payload.clone()),
                    payload,
                ],
            ),
        ));
    }

    let aux = Term::Fix {
        name: "aux".to_string(),
        body: Box::new(lam(
            "s",
            sc.carrier.clone(),
            Term::Match {
                discriminant: Box::new(var("s")),
                motive: Box::new(lam(
                    "z",
                    sc.carrier.clone(),
                    prod(app(var("motive"), var("z")), below_at(var("z"))),
                )),
                cases,
            },
        )),
    };
    let body = app(
        apps(
            gl("Prod_fst"),
            &[app(var("motive"), var("t")), below_at(var("t"))],
        ),
        app(aux, var("t")),
    );
    let term = sc.close_lams(lam(
        "motive",
        motive_ty,
        lam("t", sc.carrier.clone(), lam("F", f_ty, body)),
    ));
    let ty = infer_type(&env, &term)?;
    Ok((ty, term))
}

/// `decEqI : Π(A…). Π(dA_i : Π(u v : A_i). Decidable (Eq A_i u v))…. Π(a b : I A…). Decidable (Eq (I A…) a b)`
///
/// Lean's `deriving DecidableEq`, as a term. Structural recursion on `a`, then a case on `b`:
/// DIFFERENT constructors are refuted by [`derive_no_confusion`] at `P := False`; the SAME
/// constructor decides its fields left to right, assembling the positive proof by iterated
/// transport (one `Eq_rec_dep` per field, rewriting one argument of the constructor at a time)
/// and the negative one from field-`k` injectivity, again via `noConfusion`.
///
/// A parametric inductive gets one decidable-equality PREMISE per type parameter, exactly as
/// Lean's `instDecidableEqList [DecidableEq α]` does.
///
/// A field's own decision procedure is resolved as: the inductive itself (the recursion), a
/// type parameter (its premise), or a monomorphic `T` with `decEqT` already in scope (the
/// prelude's `decEqNat`/`decEqBool`, or an earlier derivation). Anything else is REFUSED with
/// the field named — a decision procedure we cannot actually exhibit must not be claimed.
pub fn derive_dec_eq(ctx: &Context, ind: &str) -> KernelResult<(Term, Term)> {
    let sc = AuxScope::new(ctx, ind)?;
    sc.require_equatable_carrier()?;
    require_global(ctx, "Eq", "the equation being decided")?;
    require_global(
        ctx,
        "refl",
        "the reflexivity the positive branch starts from",
    )?;
    require_global(
        ctx,
        "Eq_rec_dep",
        "the transport that assembles a constructor congruence",
    )?;
    require_global(ctx, "Decidable", "the decision procedure's carrier")?;
    require_global(ctx, "isTrue", "the decision's positive constructor")?;
    require_global(ctx, "isFalse", "the decision's negative constructor")?;
    require_global(
        ctx,
        "False",
        "the absurdity a mismatched constructor pair yields",
    )?;
    let env = with_no_confusion(ctx, ind)?;

    // One decidable-equality premise per parameter. A parameter that is not a `Type 0` (an
    // index-like value parameter, or a `Prop`) has no such premise to state, so refuse.
    let mut premises: Vec<(String, Term)> = Vec::with_capacity(sc.num_params);
    for i in 0..sc.num_params {
        if sc.param_types[i] != Term::Sort(Universe::Type(0)) {
            return Err(unsupported(format!(
                "parameter '{}' of '{ind}' is a {}, not a `Type 0`: a decidable-equality premise \
                 can only be stated for a type parameter",
                sc.tp_names[i], sc.param_types[i]
            )));
        }
        let a_i = var(&sc.tp_names[i]);
        premises.push((
            format!("dA{i}"),
            pi(
                "u",
                a_i.clone(),
                pi("v", a_i.clone(), dec(eq3(a_i, var("u"), var("v")))),
            ),
        ));
    }

    let mut nc = gl(&format!("{ind}_noConfusion"));
    for n in &sc.tp_names {
        nc = app(nc, var(n));
    }

    let mut outer = Vec::with_capacity(sc.ctors.len());
    for (i, (ci, cti)) in sc.ctors.iter().enumerate() {
        let xs = ctor_value_fields(cti, ind, sc.num_params, &sc.tp_names, "x")?;
        let cx = sc.ctor_at(ci, &xs);
        let mut inner = Vec::with_capacity(sc.ctors.len());
        for (j, (cj, ctj)) in sc.ctors.iter().enumerate() {
            let ys = ctor_value_fields(ctj, ind, sc.num_params, &sc.tp_names, "y")?;
            let cy = sc.ctor_at(cj, &ys);
            let goal = eq3(sc.carrier.clone(), cx.clone(), cy.clone());
            let arm = if i == j {
                homogeneous_fields(&xs, &ys, ci)?;
                dec_eq_same_ctor(&sc, ci, &xs, &ys, &nc, &premises)?
            } else {
                // `noConfusionType False (C x⃗) (D y⃗)` IS `False`, so `noConfusion` at `False` is
                // the refutation outright — no discriminating family to hand-write per pair.
                let refute = lam(
                    "h",
                    goal.clone(),
                    apps(nc.clone(), &[gl("False"), cx.clone(), cy.clone(), var("h")]),
                );
                app(app(gl("isFalse"), goal), refute)
            };
            inner.push(lams(&ys, arm));
        }
        outer.push(lams(
            &xs,
            Term::Match {
                discriminant: Box::new(var("b")),
                motive: Box::new(lam(
                    "b✧",
                    sc.carrier.clone(),
                    dec(eq3(sc.carrier.clone(), cx.clone(), var("b✧"))),
                )),
                cases: inner,
            },
        ));
    }

    let a_match = Term::Match {
        discriminant: Box::new(var("a")),
        motive: Box::new(lam(
            "a✧",
            sc.carrier.clone(),
            dec(eq3(sc.carrier.clone(), var("a✧"), var("b"))),
        )),
        cases: outer,
    };
    let mut term = Term::Fix {
        name: "rec".to_string(),
        body: Box::new(lam(
            "a",
            sc.carrier.clone(),
            lam("b", sc.carrier.clone(), a_match),
        )),
    };
    for (n, t) in premises.iter().rev() {
        term = lam(n, t.clone(), term);
    }
    let term = sc.close_lams(term);
    let ty = infer_type(&env, &term)?;
    Ok((ty, term))
}

/// The SAME-constructor arm of a derived decidable equality: decide field `0`, then `1`, …,
/// and only once every field agrees assemble `Eq I (C x⃗) (C y⃗)` from the field equations.
///
/// The positive proof is a CONGRUENCE built by transport: starting from `refl (C x⃗)`, step `k`
/// rewrites the `k`-th argument of the right-hand constructor from `xₖ` to `yₖ` along `eₖ`.
/// The negative one is field-`k` injectivity via `noConfusion` — which is why no field needs a
/// defaulted projection.
fn dec_eq_same_ctor(
    sc: &AuxScope,
    cname: &str,
    xs: &[(String, Term)],
    ys: &[(String, Term)],
    nc: &Term,
    premises: &[(String, Term)],
) -> KernelResult<Term> {
    let cx = sc.ctor_at(cname, xs);
    let cy = sc.ctor_at(cname, ys);
    let goal = eq3(sc.carrier.clone(), cx.clone(), cy.clone());
    let eqs: Vec<Term> = (0..xs.len())
        .map(|k| eq3(xs[k].1.clone(), var(&xs[k].0), var(&ys[k].0)))
        .collect();

    // Every field's decision procedure must EXIST before any of the term is built.
    let decs: Vec<Term> = (0..xs.len())
        .map(|k| field_decider(sc, &xs[k], cname, premises))
        .collect::<KernelResult<Vec<_>>>()?;

    // The congruence: `C x⃗ = C y₀…y_{k-1} xₖ…x_{m-1}` after `k` transports.
    let mut congruence = app(app(gl("refl"), sc.carrier.clone()), cx.clone());
    for k in 0..xs.len() {
        let rewritten = |at: Term| {
            let mut t = gl(cname);
            for n in &sc.tp_names {
                t = app(t, var(n));
            }
            for (q, (xn, _)) in xs.iter().enumerate() {
                t = app(
                    t,
                    match q.cmp(&k) {
                        std::cmp::Ordering::Less => var(&ys[q].0),
                        std::cmp::Ordering::Equal => at.clone(),
                        std::cmp::Ordering::Greater => var(xn),
                    },
                );
            }
            t
        };
        let motive = lam(
            "z",
            xs[k].1.clone(),
            lam(
                "_",
                eq3(xs[k].1.clone(), var(&xs[k].0), var("z")),
                eq3(sc.carrier.clone(), cx.clone(), rewritten(var("z"))),
            ),
        );
        congruence = apps(
            gl("Eq_rec_dep"),
            &[
                xs[k].1.clone(),
                var(&xs[k].0),
                motive,
                congruence,
                var(&ys[k].0),
                var(&format!("e{k}")),
            ],
        );
    }

    // Innermost: every field agreed.
    let mut arm = app(app(gl("isTrue"), goal.clone()), congruence);
    // …wrapped outwards, one decision per field.
    for k in (0..xs.len()).rev() {
        // `eₖ` is bound by the `isTrue` case, so the accumulated congruence sees it.
        let positive = lam(&format!("e{k}"), eqs[k].clone(), arm);
        // `noConfusion (xₖ = yₖ) (C x⃗) (C y⃗) h (λe₀ … e_{m-1}. eₖ)` — field-`k` injectivity.
        let mut project = var(&format!("p{k}"));
        for q in (0..xs.len()).rev() {
            project = lam(&format!("p{q}"), eqs[q].clone(), project);
        }
        let inj_k = app(
            apps(
                nc.clone(),
                &[eqs[k].clone(), cx.clone(), cy.clone(), var("hEq")],
            ),
            project,
        );
        let negative = lam(
            &format!("n{k}"),
            not_(eqs[k].clone()),
            app(
                app(gl("isFalse"), goal.clone()),
                lam("hEq", goal.clone(), app(var(&format!("n{k}")), inj_k)),
            ),
        );
        arm = Term::Match {
            discriminant: Box::new(app(app(decs[k].clone(), var(&xs[k].0)), var(&ys[k].0))),
            motive: Box::new(lam("_", dec(eqs[k].clone()), dec(goal.clone()))),
            cases: vec![positive, negative],
        };
    }
    Ok(arm)
}

/// The decision procedure for ONE constructor field: the recursion itself for a field of the
/// inductive, the matching premise for a type parameter, an already-registered `decEqT` for a
/// monomorphic `T`. Anything else is refused by name — a `Decidable` we cannot exhibit must
/// not be claimed, and `decEqNat` silently reused for another carrier would compare one field
/// and ignore the constructor tag.
fn field_decider(
    sc: &AuxScope,
    field: &(String, Term),
    cname: &str,
    premises: &[(String, Term)],
) -> KernelResult<Term> {
    let (fname, fty) = field;
    if *fty == sc.carrier {
        return Ok(var("rec"));
    }
    if let Term::Var(n) = fty {
        if let Some(i) = sc.tp_names.iter().position(|a| a == n) {
            return Ok(var(&premises[i].0));
        }
    }
    if let Term::Global(t) = fty {
        let name = format!("decEq{t}");
        if sc.known_dec_eq.contains(&name) {
            return Ok(gl(&name));
        }
        return Err(unsupported(format!(
            "field '{fname}' of '{cname}' has type '{t}', and '{name}' is not in scope: derive or \
             register a decidable equality for '{t}' first"
        )));
    }
    Err(unsupported(format!(
        "field '{fname}' of '{cname}' has type {fty}, which is neither the inductive itself, a \
         type parameter, nor a monomorphic carrier with a registered decidable equality"
    )))
}

// =====================================================================================
// THE DEPENDENT QUOTIENT ELIMINATOR — DERIVED, NOT ASSUMED
// =====================================================================================

/// `Quot_rec` — the DEPENDENT quotient eliminator, built from the four quotient primitives
/// rather than taken as a fifth one. The prelude registers exactly this body as `Quot_rec`.
///
/// **Lean does not assume it either.** `init_quot`
/// (`Init/Core.lean:1515`–`1527`) introduces exactly `Quot`, `Quot.mk`, `Quot.lift` and
/// `Quot.ind`, alongside the `Quot.sound` axiom — four constants and one axiom, not five.
/// `Quot.rec` is a `protected abbrev` assembled from them at `Init/Core.lean:1581`–`1611`, and
/// `#print axioms Quot.rec` answers `[Quot.sound]`: it costs the trusted base nothing beyond
/// the axiom already paid for. The recipe below is Lean's, transcribed constant for constant:
///
/// ```text
/// indep f a             := ⟨Quot_mk a, f a⟩              : PSigma (Quot A r) motive
/// indepCoherent f h a b := PSigma.eta (Quot_sound p) (h a b p)
/// L                     := Quot_lift A r (PSigma Q motive) indep indepCoherent
/// liftIndepPr1 q        : PSigma_fst (L q) = q           -- Quot_ind, then refl
/// Quot_rec … q          := liftIndepPr1 q ▸ PSigma_snd (L q)
/// ```
///
/// The two computation steps that make it work are already in the kernel: `Quot_lift … (Quot_mk
/// a) ⤳ indep a` is the one ι-rule quotients have, and `PSigma_fst (PSigma_mk x y) ⤳ x` is an
/// ordinary structure projection. So `PSigma_fst (L (Quot_mk a))` reduces to `Quot_mk a` and the
/// `liftIndepPr1` obligation is discharged by `refl` in every branch `Quot_ind` produces — which
/// is exactly why Lean's proof is `induction q using Quot.ind; exact rfl`.
///
/// `PSigma.eta` is inlined rather than registered: it is used once, and Lean's own
/// (`Init/Core.lean:1272`–`1276`) is two `subst`s, which is two `Eq_rec_dep` transports here.
///
/// The pair is `PSigma`, not `Sigma`, for the reason Lean's is: the carrier is `Sort u`, so the
/// pair lives in `Sort (max 1 u)` and `Quot_lift` reaches it through its own target level. Every
/// reference is a bare global whose levels the application rule solves at each use, so the only
/// level the body names is its carrier binder's, which β removes wherever the eliminator is
/// applied. The motive is `Quot A r → Type 0`, the prelude statement's own; Lean's is `Sort v`.
pub fn derive_quot_rec(ctx: &Context) -> KernelResult<(Term, Term)> {
    let body = quot_rec_body(ctx)?;
    let ty = infer_type(ctx, &body)?;
    Ok((ty, body))
}

/// [`derive_quot_rec`]'s term alone, for a caller whose own gateway checks it against a stated
/// type — the prelude registers it this way, so the body is checked once.
pub(crate) fn quot_rec_body(ctx: &Context) -> KernelResult<Term> {
    for need in [
        "Quot",
        "Quot_mk",
        "Quot_lift",
        "Quot_ind",
        "Quot_sound",
        "PSigma",
        "PSigma_mk",
        "PSigma_fst",
        "PSigma_snd",
        "Eq",
        "refl",
        "Eq_rec_dep",
    ] {
        require_global(ctx, need, "the derived dependent quotient eliminator")?;
    }

    let ty0 = Term::Sort(Universe::Type(0));
    let carrier = Term::Sort(Universe::Var("u".to_string()));
    let prop = Term::Sort(Universe::Prop);
    let a = || var("A");
    let r = || var("r");
    let motive = || var("motive");
    // `Quot A r`, the quotient being eliminated, and its class former.
    let q_ty = || apps(gl("Quot"), &[a(), r()]);
    let mk = |x: Term| apps(gl("Quot_mk"), &[a(), r(), x]);
    // `PSigma (Quot A r) motive` — the non-dependent carrier `Quot_lift` can actually target.
    let sig = || apps(gl("PSigma"), &[q_ty(), motive()]);
    let smk = |x: Term, y: Term| apps(gl("PSigma_mk"), &[q_ty(), motive(), x, y]);
    let sfst = |s: Term| apps(gl("PSigma_fst"), &[q_ty(), motive(), s]);
    let ssnd = |s: Term| apps(gl("PSigma_snd"), &[q_ty(), motive(), s]);
    let refl_at = |t: Term, x: Term| apps(gl("refl"), &[t, x]);
    // `e ▸ b` at the family `fam` over `carrier`, spelled with the prelude's derived J.
    let transport =
        |carrier: Term, x: Term, fam: &dyn Fn(Term) -> Term, b: Term, y: Term, e: Term| {
            apps(
                gl("Eq_rec_dep"),
                &[
                    carrier.clone(),
                    x.clone(),
                    lam(
                        "tgt✧",
                        carrier.clone(),
                        lam("_", eq3(carrier, x, var("tgt✧")), fam(var("tgt✧"))),
                    ),
                    b,
                    y,
                    e,
                ],
            )
        };
    let motive_at = |x: Term| app(motive(), x);

    // The respect obligation, stated up to transport exactly as Lean's `h` is: `f a` and `f b`
    // inhabit DIFFERENT types, so there is no carrier to equate them at until `f a` has been
    // moved along `Quot_sound`.
    let sound_p = || {
        apps(
            gl("Quot_sound"),
            &[a(), r(), var("a✧"), var("b✧"), var("p✧")],
        )
    };
    let respect = pi(
        "a✧",
        a(),
        pi(
            "b✧",
            a(),
            pi(
                "p✧",
                apps(r(), &[var("a✧"), var("b✧")]),
                eq3(
                    motive_at(mk(var("b✧"))),
                    transport(
                        q_ty(),
                        mk(var("a✧")),
                        &motive_at,
                        app(var("f"), var("a✧")),
                        mk(var("b✧")),
                        sound_p(),
                    ),
                    app(var("f"), var("b✧")),
                ),
            ),
        ),
    );

    // `PSigma.eta` at this quotient, inlined at the one instantiation that uses it:
    // `h₁ : a₁ = a₂` and `h₂ : h₁ ▸ b₁ = b₂` give `⟨a₁,b₁⟩ = ⟨a₂,b₂⟩`, by J on `h₁` and then
    // on `h₂` — Lean's `subst; subst; rfl`.
    let sigma_eta = |a1: Term, b1: Term, a2: Term, h1: Term, b2: Term, h2: Term| {
        // λb2'. λh2'. (h2' : b₁ = b2') ▸ refl ⟨a₁,b₁⟩ — the base case, where the outer
        // transport has already ι-reduced away on `refl`.
        let base = lam(
            "b2✧",
            motive_at(a1.clone()),
            lam(
                "h2✧",
                eq3(
                    motive_at(a1.clone()),
                    transport(
                        q_ty(),
                        a1.clone(),
                        &motive_at,
                        b1.clone(),
                        a1.clone(),
                        refl_at(q_ty(), a1.clone()),
                    ),
                    var("b2✧"),
                ),
                transport(
                    motive_at(a1.clone()),
                    b1.clone(),
                    &|z: Term| eq3(sig(), smk(a1.clone(), b1.clone()), smk(a1.clone(), z)),
                    refl_at(sig(), smk(a1.clone(), b1.clone())),
                    var("b2✧"),
                    var("h2✧"),
                ),
            ),
        );
        let fam = |y: Term| {
            pi(
                "b2✧",
                motive_at(y.clone()),
                arrow(
                    eq3(
                        motive_at(y.clone()),
                        transport(
                            q_ty(),
                            a1.clone(),
                            &motive_at,
                            b1.clone(),
                            y.clone(),
                            var("e✧"),
                        ),
                        var("b2✧"),
                    ),
                    eq3(sig(), smk(a1.clone(), b1.clone()), smk(y, var("b2✧"))),
                ),
            )
        };
        // The J motive mentions the equation itself (`e✧`), so it is written out rather than
        // routed through `transport`, whose family is equation-independent.
        let j_motive = lam(
            "tgt✧",
            q_ty(),
            lam("e✧", eq3(q_ty(), a1.clone(), var("tgt✧")), fam(var("tgt✧"))),
        );
        apps(
            apps(gl("Eq_rec_dep"), &[q_ty(), a1, j_motive, base, a2, h1]),
            &[b2, h2],
        )
    };

    // `indep : A → PSigma (Quot A r) motive`, and its coherence.
    let indep = lam("a✧", a(), smk(mk(var("a✧")), app(var("f"), var("a✧"))));
    let indep_coherent = lam(
        "a✧",
        a(),
        lam(
            "b✧",
            a(),
            lam(
                "p✧",
                apps(r(), &[var("a✧"), var("b✧")]),
                sigma_eta(
                    mk(var("a✧")),
                    app(var("f"), var("a✧")),
                    mk(var("b✧")),
                    sound_p(),
                    app(var("f"), var("b✧")),
                    apps(var("h"), &[var("a✧"), var("b✧"), var("p✧")]),
                ),
            ),
        ),
    );
    let lifted = || {
        apps(
            gl("Quot_lift"),
            &[a(), r(), sig(), indep.clone(), indep_coherent.clone()],
        )
    };

    // `liftIndepPr1 : Π(q). PSigma_fst (L q) = q` — `Quot_ind` reduces the goal to the `Quot_mk`
    // branch, where `L (mk a) ⤳ ⟨mk a, f a⟩` and the projection ⤳ `mk a`, so `refl` closes it.
    let pr1 = || {
        apps(
            gl("Quot_ind"),
            &[
                a(),
                r(),
                lam(
                    "q✧",
                    q_ty(),
                    eq3(q_ty(), sfst(app(lifted(), var("q✧"))), var("q✧")),
                ),
                lam("a✧", a(), refl_at(q_ty(), mk(var("a✧")))),
            ],
        )
    };

    let body = lam(
        "A",
        carrier,
        lam(
            "r",
            arrow(a(), arrow(a(), prop)),
            lam(
                "motive",
                arrow(q_ty(), ty0),
                lam(
                    "f",
                    pi("a✧", a(), motive_at(mk(var("a✧")))),
                    lam(
                        "h",
                        respect,
                        lam(
                            "q",
                            q_ty(),
                            transport(
                                q_ty(),
                                sfst(app(lifted(), var("q"))),
                                &motive_at,
                                ssnd(app(lifted(), var("q"))),
                                var("q"),
                                app(pr1(), var("q")),
                            ),
                        ),
                    ),
                ),
            ),
        ),
    );

    Ok(body)
}

// =====================================================================================
// `casesOn` AND `recOn` — THE TWO CONSTANTS LEAN GENERATES THAT WE DID NOT
// =====================================================================================

/// `Π(tele). body`.
fn pis(tele: &[(String, Term)], body: Term) -> Term {
    tele.iter()
        .rev()
        .fold(body, |acc, (n, t)| pi(n, t.clone(), acc))
}

/// How many MINOR premises and how many INDICES `ind`'s eliminator carries.
///
/// The minor count is the whole block's constructor count, not this member's: a mutual
/// eliminator takes one minor per constructor of every member, because the recursion crosses
/// between them. The index count is this member's own.
fn eliminator_shape(ctx: &Context, ind: &str) -> (usize, usize) {
    let block: Vec<String> = match ctx.mutual_block_of(ind) {
        Some(b) => b.to_vec(),
        None => vec![ind.to_string()],
    };
    let minors = block.iter().map(|m| ctx.get_constructors(m).len()).sum();
    (minors, ctx.inductive_num_indices(ind))
}

/// The value arity of each constructor, in the order the eliminator's minor premises take them.
fn minor_arities(ctx: &Context, ind: &str) -> Vec<usize> {
    let block: Vec<String> = match ctx.mutual_block_of(ind) {
        Some(b) => b.to_vec(),
        None => vec![ind.to_string()],
    };
    let mut out = Vec::new();
    for member in &block {
        let np = ctx.inductive_num_params(member);
        for (_, cty) in ctx.get_constructors(member) {
            let (binders, _) = peel_pis(cty);
            out.push(binders.len().saturating_sub(np));
        }
    }
    out
}

/// Split `I_rec`'s telescope into `(params_and_motives, minors, indices, major)` — the four
/// blocks Lean's `mkRecOn` permutes (`Lean/Meta/Constructions/RecOn.lean:20-26`, whose comment
/// spells the layout out: "before: As Cs minor_premises indices major-premise").
fn split_eliminator_telescope(
    ctx: &Context,
    ind: &str,
    rec_ty: &Term,
) -> KernelResult<(
    Vec<(String, Term)>,
    Vec<(String, Term)>,
    Vec<(String, Term)>,
    (String, Term),
    Term,
)> {
    let (xs, residual) = peel_pis(rec_ty);
    let (n_minors, n_indices) = eliminator_shape(ctx, ind);
    if xs.len() < n_minors + n_indices + 1 {
        return Err(unsupported(format!(
            "'{ind}'s eliminator binds {} arguments, too few for {n_minors} minor premise(s), \
             {n_indices} index/indices and a major premise",
            xs.len()
        )));
    }
    let ac = xs.len() - n_minors - n_indices - 1;
    // A permutation is only well-typed if the binder NAMES stay distinguishable, and the
    // eliminator's own names are the only thing standing between a moved binder and the wrong
    // reference. Refuse rather than emit a term whose meaning depends on shadowing.
    let mut seen: Vec<&str> = Vec::new();
    for (n, _) in &xs {
        if n == "_" || seen.contains(&n.as_str()) {
            return Err(unsupported(format!(
                "'{ind}'s eliminator binds '{n}' anonymously or twice, so its telescope cannot be \
                 reordered without capture"
            )));
        }
        seen.push(n);
    }
    let head = xs[..ac].to_vec();
    let minors = xs[ac..ac + n_minors].to_vec();
    let indices = xs[ac + n_minors..ac + n_minors + n_indices].to_vec();
    let major = xs[ac + n_minors + n_indices].clone();
    Ok((head, minors, indices, major, residual.clone()))
}

/// `I_recOn` — the eliminator with the MAJOR PREMISE moved in front of the minors, which is the
/// argument order `induction`/`match` elaboration wants and the one every Lean development
/// writes by hand.
///
/// Lean generates it at every declaration (`Lean/Elab/Inductive.lean:729`) by exactly this
/// permutation (`Lean/Meta/Constructions/RecOn.lean:20-31`):
///
/// ```text
/// before: As Cs minor_premises indices major-premise
/// after:  As Cs indices major-premise minor_premises
/// ```
///
/// It is a RESHAPING of `derive_recursor`'s answer and nothing else — every soundness gate
/// (the motive-sort rule, strict positivity, the termination guard that admitted the `Fix`) is
/// the one the eliminator already passed, and no new trusted code is added by moving binders.
pub fn derive_rec_on(ctx: &Context, ind: &str) -> KernelResult<(Term, Term)> {
    rec_on_from(ctx, ind, derive_recursor(ctx, ind)?)
}

/// [`derive_rec_on`] over an eliminator that has already been derived — the shape
/// [`derive_auxiliaries`] uses so one declaration pays for one derivation.
fn rec_on_from(ctx: &Context, ind: &str, elim: (Term, Term)) -> KernelResult<(Term, Term)> {
    let (rec_ty, rec_tm) = elim;
    let (head, minors, indices, major, residual) = split_eliminator_telescope(ctx, ind, &rec_ty)?;

    let mut reordered = head.clone();
    reordered.extend(indices.iter().cloned());
    reordered.push(major.clone());
    reordered.extend(minors.iter().cloned());

    // The body applies the ORIGINAL eliminator in the ORIGINAL order.
    let mut original: Vec<Term> = Vec::new();
    for (n, _) in head.iter().chain(minors.iter()).chain(indices.iter()) {
        original.push(var(n));
    }
    original.push(var(&major.0));
    let body = lams(&reordered, apps(rec_tm, &original));
    certified_reshaping(ctx, ind, "recOn", body, pis(&reordered, residual))
}

/// `I_casesOn` — `recOn` with the INDUCTION HYPOTHESES dropped from every minor premise.
///
/// Lean's is `@Nat.casesOn : {motive : Nat → Sort u} → (t : Nat) → motive Nat.zero →
/// ((n : Nat) → motive n.succ) → motive t`: the case analysis `cases h` compiles to, and the
/// constant every `match` elaborates through. It is generated at every declaration
/// (`Lean/Elab/Inductive.lean:730`).
///
/// Derived here rather than built: each minor premise keeps its FIELD binders and loses its
/// `ih` binders, and the body hands the original eliminator a minor that ignores the hypotheses
/// it was given. So `casesOn` is `rec` with the recursion thrown away, which is exactly what it
/// means — and it inherits every gate `rec` passed rather than opening a second door.
pub fn derive_cases_on(ctx: &Context, ind: &str) -> KernelResult<(Term, Term)> {
    cases_on_from(ctx, ind, derive_recursor(ctx, ind)?)
}

/// [`derive_cases_on`] over an eliminator that has already been derived.
fn cases_on_from(ctx: &Context, ind: &str, elim: (Term, Term)) -> KernelResult<(Term, Term)> {
    let (rec_ty, rec_tm) = elim;
    let (head, minors, indices, major, residual) = split_eliminator_telescope(ctx, ind, &rec_ty)?;
    let arities = minor_arities(ctx, ind);
    if arities.len() != minors.len() {
        return Err(unsupported(format!(
            "'{ind}'s eliminator has {} minor premise(s) but the block declares {} constructor(s)",
            minors.len(),
            arities.len()
        )));
    }

    // Each minor loses its `ih` binders; what is left is the constructor's own fields and the
    // motive instance they land at.
    let mut stripped: Vec<(String, Term)> = Vec::new();
    let mut supplied: Vec<Term> = Vec::new();
    for (i, (mname, mty)) in minors.iter().enumerate() {
        let (binders, res) = peel_pis(mty);
        if binders.len() < arities[i] {
            return Err(unsupported(format!(
                "'{ind}'s minor premise '{mname}' binds {} arguments, fewer than its \
                 constructor's {} field(s)",
                binders.len(),
                arities[i]
            )));
        }
        let fields = &binders[..arities[i]];
        let hyps = &binders[arities[i]..];
        stripped.push((mname.clone(), pis(fields, res.clone())));
        // `λfields. λihs. mname fields…` — the hypotheses are bound and dropped.
        let call = apps(
            var(mname),
            &fields.iter().map(|(n, _)| var(n)).collect::<Vec<_>>(),
        );
        supplied.push(lams(fields, lams(hyps, call)));
    }

    let mut reordered = head.clone();
    reordered.extend(indices.iter().cloned());
    reordered.push(major.clone());
    reordered.extend(stripped.iter().cloned());

    let mut original: Vec<Term> = head.iter().map(|(n, _)| var(n)).collect();
    original.extend(supplied);
    original.extend(indices.iter().map(|(n, _)| var(n)));
    original.push(var(&major.0));
    let body = lams(&reordered, apps(rec_tm, &original));
    certified_reshaping(ctx, ind, "casesOn", body, pis(&reordered, residual))
}

/// Return a reshaped eliminator ONLY if the kernel certifies it, and only at the type the
/// reshaping intended.
///
/// The type here is CONSTRUCTED — a permutation of the eliminator's telescope — where every
/// other `derive_*` in this module reports `infer_type` of the term it built. A constructed type
/// that the term does not have is the one failure mode a reshaping can hide: `derive_auxiliaries`
/// registers what a `derive_*` returns WITHOUT re-checking it, so an off-by-one in the
/// permutation would land a constant whose declared type is not its real one, and the first
/// proof to cite it would be the one that discovers it. Asking the kernel makes the reshaping
/// fail-closed, and comparing the two makes a SILENTLY different answer impossible.
fn certified_reshaping(
    ctx: &Context,
    ind: &str,
    what: &str,
    body: Term,
    intended: Term,
) -> KernelResult<(Term, Term)> {
    let certified = infer_type(ctx, &body)?;
    if !crate::type_checker::is_subtype(ctx, &certified, &intended)
        || !crate::type_checker::is_subtype(ctx, &intended, &certified)
    {
        return Err(unsupported(format!(
            "'{ind}'s {what} was reshaped to {intended} but the kernel certifies it at              {certified} — the telescope permutation is wrong"
        )));
    }
    Ok((certified, body))
}

/// The shared scope every derived auxiliary of a NON-INDEXED inductive is built in: the
/// inductive's uniform parameters renamed `A0 … A_{p-1}`, the applied carrier `I A0 …
/// A_{p-1}`, the sort its arity ends in, and its constructors in registration order.
struct AuxScope {
    ind: String,
    num_params: usize,
    tp_names: Vec<String>,
    param_types: Vec<Term>,
    ind_sort: Term,
    carrier: Term,
    ctors: Vec<(String, Term)>,
    /// The `decEq…` names already in scope, so a field's decision procedure can be resolved
    /// without carrying the whole context around.
    known_dec_eq: Vec<String>,
}

impl AuxScope {
    fn new(ctx: &Context, ind: &str) -> KernelResult<AuxScope> {
        if !ctx.is_inductive(ind) {
            return Err(KernelError::UnboundVariable(ind.to_string()));
        }
        let full = ctx
            .get_global(ind)
            .ok_or_else(|| KernelError::UnboundVariable(ind.to_string()))?
            .clone();
        let (arity, sort) = peel_pis(&full);
        let num_params = ctx.inductive_num_params(ind).min(arity.len());
        if arity.len() > num_params {
            return Err(unsupported(format!(
                "'{ind}' is an INDEXED family with {} index binder(s). Every auxiliary here gives \
                 one branch per CONSTRUCTOR PAIR, and the pairs whose declared result indices \
                 cannot unify have no inhabitant to offer — discharging them needs an index \
                 unifier, which this kernel's positional `Match` (no patterns, no wildcard, \
                 coverage by count) does not have",
                arity.len() - num_params
            )));
        }
        let tp_names: Vec<String> = (0..num_params).map(|i| format!("A{i}")).collect();
        let param_types: Vec<Term> = (0..num_params)
            .map(|i| rename_tele_types(&arity[i].1, &arity[..i], &tp_names))
            .collect();
        let carrier = tp_names
            .iter()
            .fold(Term::Global(ind.to_string()), |acc, n| app(acc, var(n)));
        let ctors = ctx
            .get_constructors(ind)
            .iter()
            .map(|(n, t)| (n.to_string(), (*t).clone()))
            .collect();
        let known_dec_eq = ctx
            .iter_definitions()
            .map(|(n, _, _)| n.to_string())
            .filter(|n| n.starts_with("decEq"))
            .collect();
        Ok(AuxScope {
            ind: ind.to_string(),
            num_params,
            tp_names,
            param_types,
            ind_sort: sort.clone(),
            carrier,
            ctors,
            known_dec_eq,
        })
    }

    /// Refuse an auxiliary that has to state an EQUATION over the inductive unless the carrier
    /// is one `Eq` can be applied to.
    ///
    /// On a `Prop` this is not a missing derivation but a category error: proof irrelevance
    /// makes any two inhabitants of a proposition definitionally equal, so constructor
    /// disjointness is FALSE there. Above `Type 0` it is the prelude's `Eq` that stops, not us.
    fn require_equatable_carrier(&self) -> KernelResult<()> {
        match &self.ind_sort {
            Term::Sort(Universe::Type(0)) => Ok(()),
            Term::Sort(level) if *level == Universe::SProp || level.equiv(&Universe::Prop) => {
                Err(unsupported(format!(
                "'{}' is a PROPOSITION: proof irrelevance identifies any two of its inhabitants, \
                 so constructor disjointness is FALSE there. That is a category error, not a \
                 derivation we are missing",
                self.ind
            )))
            }
            other => Err(unsupported(format!(
                "'{}' lives in {other}, and the prelude's `Eq` quantifies over a `Type 0` carrier \
                 only — `Eq {} …` cannot be stated at all. Closing this needs a \
                 universe-polymorphic `Eq`, not a change to the derivation",
                self.ind, self.ind
            ))),
        }
    }

    /// The course-of-values auxiliaries package the subterms of ONE type. A mutual block's
    /// below would have to package every member's, keyed by member — refuse rather than emit
    /// one that silently drops the siblings.
    fn require_no_mutual_block(&self, ctx: &Context, what: &str) -> KernelResult<()> {
        match ctx.mutual_block_of(&self.ind) {
            None => Ok(()),
            Some(block) => Err(unsupported(format!(
                "'{}' is a member of the mutual block {block:?}: its `{what}` would have to carry \
                 one package per member, keyed by which member each subterm belongs to. The plain \
                 mutual recursor already handles the block; this does not",
                self.ind
            ))),
        }
    }

    /// Wrap `body` in the parameter λ-telescope `λA0 … λA_{p-1}`.
    fn close_lams(&self, mut body: Term) -> Term {
        for i in (0..self.num_params).rev() {
            body = lam(&self.tp_names[i], self.param_types[i].clone(), body);
        }
        body
    }

    /// `C A0 … A_{p-1} f0 … f_{m-1}` over the given field binders.
    fn ctor_at(&self, cname: &str, fields: &[(String, Term)]) -> Term {
        let mut t = Term::Global(cname.to_string());
        for n in &self.tp_names {
            t = app(t, var(n));
        }
        for (n, _) in fields {
            t = app(t, var(n));
        }
        t
    }
}

/// Extend `ctx` with `ind`'s no-confusion pair, deriving whichever half is absent, so a term
/// that CITES them can be checked. The caller of the outer derivation still has to register
/// both — hence [`derive_auxiliaries`]'s order.
fn with_no_confusion(ctx: &Context, ind: &str) -> KernelResult<Context> {
    let mut env = ctx.clone();
    let nct = format!("{ind}_noConfusionType");
    if env.get_global(&nct).is_none() {
        let (t, b) = derive_no_confusion_type(ctx, ind)?;
        env.add_definition(nct, t, b);
    }
    let nc = format!("{ind}_noConfusion");
    if env.get_global(&nc).is_none() {
        let (t, b) = derive_no_confusion(&env, ind)?;
        env.add_definition(nc, t, b);
    }
    Ok(env)
}

/// One constructor's VALUE fields, expressed in the auxiliary's scope: the inductive's
/// parameters renamed to `A0 …`, and the k-th field named `{prefix}{k}` — so two copies of the
/// same telescope, one per side of an equation, never capture each other.
fn ctor_value_fields(
    ctor_type: &Term,
    owner: &str,
    num_params: usize,
    tp_names: &[String],
    prefix: &str,
) -> KernelResult<Vec<(String, Term)>> {
    let (params, residual) = peel_pis(ctor_type);
    if params.len() < num_params {
        return Err(unsupported(format!(
            "a constructor of '{owner}' has fewer binders than the inductive has parameters"
        )));
    }
    if head_global(residual) != Some(owner) {
        return Err(unsupported(format!(
            "constructor result {residual} is not the inductive '{owner}'"
        )));
    }
    let (type_params, value_params) = params.split_at(num_params);
    let mut out: Vec<(String, Term)> = Vec::with_capacity(value_params.len());
    for (k, (_, ty)) in value_params.iter().enumerate() {
        let mut t = ty.clone();
        for (i, (tp, _)) in type_params.iter().enumerate() {
            if tp != "_" {
                t = subst_name(&t, tp, &var(&tp_names[i]));
            }
        }
        for (j, (vj, _)) in value_params.iter().enumerate().take(k) {
            if vj != "_" {
                t = subst_name(&t, vj, &var(&format!("{prefix}{j}")));
            }
        }
        out.push((format!("{prefix}{k}"), t));
    }
    Ok(out)
}

/// Refuse a DEPENDENT field, whose two copies do not even inhabit the same type.
///
/// The check is exactly "the two renamings agree": the only difference between the `x` and `y`
/// telescopes is which earlier field names appear, so the k-th types differ IFF the k-th field
/// mentions an earlier one. `xₖ = yₖ` would then be heterogeneous.
fn homogeneous_fields(
    xs: &[(String, Term)],
    ys: &[(String, Term)],
    cname: &str,
) -> KernelResult<()> {
    for (k, ((_, tx), (_, ty))) in xs.iter().zip(ys.iter()).enumerate() {
        if tx != ty {
            return Err(unsupported(format!(
                "field {k} of '{cname}' has type {tx} on one side of the equation and {ty} on the \
                 other — a DEPENDENT field, whose two instances do not inhabit the same type. Its \
                 equation needs heterogeneous equality (`HEq`), which the prelude does not carry"
            )));
        }
    }
    Ok(())
}

/// A recursive field of one constructor, as `below`/`brecOn` see it: the field's binder name
/// and, for a strictly-positive FUNCTIONAL field `Π(z:B…). I A…`, that domain telescope.
struct RecField {
    field: String,
    tele: Vec<(String, Term)>,
}

/// The recursive fields of one constructor, in order.
///
/// A recursive occurrence must be the carrier at the inductive's OWN parameters: a NESTED
/// occurrence (`I` under another container) is refused, because the package would have to
/// recurse through that container's own below. A non-recursive field that merely mentions the
/// inductive is refused for the same reason — silently skipping it would produce a `below`
/// that is missing a subterm the step function is entitled to.
fn recursive_fields(
    sc: &AuxScope,
    xs: &[(String, Term)],
    cname: &str,
) -> KernelResult<Vec<RecField>> {
    let mut out = Vec::new();
    for (fname, fty) in xs {
        let (tele, occ) = peel_pis(fty);
        if *occ == sc.carrier {
            out.push(RecField {
                field: fname.clone(),
                tele,
            });
            continue;
        }
        if mentions_global(fty, &sc.ind) {
            return Err(unsupported(format!(
                "field '{fname}' of '{cname}' mentions '{}' somewhere other than as a direct \
                 recursive occurrence ({fty}): a course-of-values package would have to recurse \
                 through the container it is nested under",
                sc.ind
            )));
        }
    }
    Ok(out)
}

/// One below-package ENTRY's type: `Π(z:B…). Prod (motive (xⱼ z…)) (<below> (xⱼ z…))`.
fn entry_type(rf: &RecField, below_at: &dyn Fn(Term) -> Term) -> Term {
    let applied = rf
        .tele
        .iter()
        .fold(var(&rf.field), |acc, (n, _)| app(acc, var(n)));
    let mut t = prod(app(var("motive"), applied.clone()), below_at(applied));
    for (n, ty) in rf.tele.iter().rev() {
        t = pi(n, ty.clone(), t);
    }
    t
}

/// One below-package ENTRY's value: `λz…. aux (xⱼ z…)` — literally the recursive result, which
/// is why the entry's type is `Prod (motive …) (below …)` and not a repacking of it.
fn entry_value(rf: &RecField, aux: &str) -> Term {
    let applied = rf
        .tele
        .iter()
        .fold(var(&rf.field), |acc, (n, _)| app(acc, var(n)));
    let mut v = app(var(aux), applied);
    for (n, ty) in rf.tele.iter().rev() {
        v = lam(n, ty.clone(), v);
    }
    v
}

/// The right-nested `Prod` of the entry types, closed with `Unit` (so a constructor with no
/// recursive fields has the package `Unit` rather than no package).
fn below_package_type(entries: &[Term]) -> Term {
    entries
        .iter()
        .rev()
        .fold(gl("Unit"), |acc, e| prod(e.clone(), acc))
}

/// The matching value: `Prod_mk eₜ restₜ e rest`, closed with `Tt`.
fn below_package_value(entries: &[(Term, Term)]) -> Term {
    let mut ty = gl("Unit");
    let mut val = gl("Tt");
    for (e_ty, e_val) in entries.iter().rev() {
        val = apps(
            gl("Prod_mk"),
            &[e_ty.clone(), ty.clone(), e_val.clone(), val],
        );
        ty = prod(e_ty.clone(), ty);
    }
    val
}

/// Does `t` mention the global `name` anywhere?
fn mentions_global(t: &Term, name: &str) -> bool {
    match t {
        Term::Global(n) | Term::Const { name: n, .. } => n == name,
        Term::Var(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole => false,
        Term::Pi {
            param_type,
            body_type,
            ..
        } => mentions_global(param_type, name) || mentions_global(body_type, name),
        Term::Lambda {
            param_type, body, ..
        } => mentions_global(param_type, name) || mentions_global(body, name),
        Term::App(f, a) => mentions_global(f, name) || mentions_global(a, name),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            mentions_global(discriminant, name)
                || mentions_global(motive, name)
                || cases.iter().any(|c| mentions_global(c, name))
        }
        Term::Fix { body, .. } => mentions_global(body, name),
        Term::MutualFix { defs, .. } => defs.iter().any(|(_, b)| mentions_global(b, name)),
        Term::Let {
            ty, value, body, ..
        } => {
            mentions_global(ty, name) || mentions_global(value, name) || mentions_global(body, name)
        }
        Term::Annotated { term, .. } => mentions_global(term, name),
    }
}

/// A derivation this kernel declines to fake, with the reason. Never an axiom: a missing
/// auxiliary must stay visible as a gap, not reappear as an assumption.
fn unsupported(why: String) -> KernelError {
    KernelError::CertificationError(format!("auto-derive: {why}"))
}

/// Refuse early, naming the role, when a derivation's prerequisite is not in scope.
fn require_global(ctx: &Context, name: &str, role: &str) -> KernelResult<()> {
    if ctx.get_global(name).is_some() {
        Ok(())
    } else {
        Err(unsupported(format!(
            "'{name}' is not in scope, and it is {role}"
        )))
    }
}

/// The motive-sort rule with an optional level PARAMETER for the one arm that may carry one.
///
/// An inductive whose sort is provably above Prop may eliminate into the
/// independent motive level; a subsingleton Prop uses its existing large-elimination
/// permission. Other sorts keep the restrictions of the monomorphic rule.
fn motive_target_at(ctx: &Context, ind: &str, ind_sort: &Term, level: Option<&String>) -> Term {
    let Some(u) = level else {
        return motive_target(ctx, ind, ind_sort);
    };
    match ind_sort {
        // The universe solver proves the carrier cannot become Prop, including
        // symbolic successor and maximum levels.
        Term::Sort(sort) if Universe::Type(0).is_subtype_of(sort) => {
            Term::Sort(Universe::Var(u.clone()))
        }
        // A SUBSINGLETON `Prop` is already licensed for large elimination — that licence is what
        // the monomorphic rule spends on `Type 0`, and Lean spends it on a parameter instead
        // (`Eq.rec.{u}`, `False.rec.{u}`, `And.rec.{u}`). Parameterising it grants no elimination
        // the `Type 0` arm did not already grant; it grants the LEVELS of it. The subsingleton
        // test is the gate, and it is asked here exactly as the monomorphic rule asks it.
        Term::Sort(sort)
            if !matches!(sort, Universe::SProp)
                && crate::type_checker::is_subsingleton_prop(ctx, ind).unwrap_or(false) =>
        {
            Term::Sort(Universe::Var(u.clone()))
        }
        // Everything else is a gate, not a ceiling: a multi-constructor `Prop` keeps `Prop`,
        // `SProp` keeps `SProp`, and an inductive whose own sort is already a level variable keeps
        // THAT one rather than gaining a second, independently-instantiable level.
        _ => motive_target(ctx, ind, ind_sort),
    }
}

/// **THE MOTIVE-SORT RULE — stated once, and the same rule for a standalone inductive and for a
/// member of a mutual block.**
///
/// > A derived eliminator's motive targets the inductive's OWN sort.
///
/// Concretely its own sort, not a level the recursor invents: `Nat : Type 0` gets
/// `P : Nat → Type 0`, an inductive declared at `Type 1` gets `Type 1` — the level its own
/// constructor fields live at, which is the whole reason it was declared one level up. A motive
/// pinned BELOW the inductive's sort accepts the declaration and leaves the type unusable; a
/// motive at a free level variable is a DIFFERENT declaration — a universe-polymorphic one — and
/// saying so in a monomorphic eliminator's type loses the concrete instance without gaining the
/// polymorphic one, because `Type 0 ⋠ u` (a variable ranges over `Prop` too).
///
/// The polymorphic eliminator is a real and separate artefact:
/// [`derive_recursor_universe_poly`] builds `I_rec.{u} : Π(motive : I → Sort u). …` with `u` an
/// explicit level PARAMETER, to be registered through [`Context::add_universe_poly`] and
/// instantiated at each use. Monomorphic and polymorphic are two declarations, not two spellings
/// of one, and this function is the monomorphic one.
///
/// The four arms below are each a SOUNDNESS gate rather than a limitation:
/// - A **multi-constructor `Prop`** keeps its `Prop`-restricted induction principle:
///   eliminating it into `Type` would extract computational content from a proof.
/// - A **subsingleton `Prop`** gets `Type 0` — `Eq`'s J and `False`'s ex falso are the whole
///   reason the subsingleton rule exists, and folding this into the general arm would return
///   `Prop` and destroy large elimination.
/// - **`SProp`** eliminates ONLY into `SProp`: definitional irrelevance equates every
///   inhabitant, so a match out of one into a relevant sort equates its branches. Matches the
///   gate in `type_checker.rs`/`recheck.rs`.
/// - An inductive whose own sort is a level VARIABLE (`Box.{u} : Sort u → Sort u`) keeps THAT
///   variable — it is the inductive's own parameter, instantiated together with the inductive,
///   never a free level of the eliminator's own. A free one could be instantiated at `Prop`
///   independently, which is large elimination out of a proposition: the gate above, defeated by
///   an instantiation.
fn motive_target(ctx: &Context, ind: &str, ind_sort: &Term) -> Term {
    match ind_sort {
        Term::Sort(level)
            if level.equiv(&Universe::Prop)
                && !crate::type_checker::is_subsingleton_prop(ctx, ind).unwrap_or(false) =>
        {
            Term::Sort(Universe::Prop)
        }
        Term::Sort(level) if level.equiv(&Universe::Prop) => Term::Sort(Universe::Type(0)),
        Term::Sort(Universe::SProp) => Term::Sort(Universe::SProp),
        Term::Sort(u) => Term::Sort(u.clone()),
        _ => Term::Sort(Universe::Type(0)),
    }
}

/// `count` fresh universe-PARAMETER names for the motives of `ind`'s (block's) polymorphic
/// eliminator — one per motive, so a mutual member is never forced to eliminate at its sibling's
/// level just for being declared in company.
///
/// These are ordinary declaration parameters (`u`, `u1`, …), not metavariables: the artefact
/// being built is a universe-polymorphic DECLARATION, whose levels are supplied by the reference
/// (`Const { levels }`) or introduced as metavariables by the application rule at a bare
/// `Global`. Spelling them as metavariables here would claim they are already solvable, which at
/// a declaration is exactly backwards.
///
/// The names avoid every level variable the block's arities and constructors already mention:
/// reusing one would CAPTURE it, so instantiating the motive's level would silently instantiate
/// the inductive's and `Box.{u}`'s eliminator would be pinned to one level again.
fn fresh_motive_levels(ctx: &Context, ind: &str, count: usize) -> Vec<String> {
    let block: Vec<String> = match ctx.mutual_block_of(ind) {
        Some(b) => b.to_vec(),
        None => vec![ind.to_string()],
    };
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    for name in &block {
        if let Some((parameters, _)) = ctx.poly_declaration(name) {
            used.extend(parameters.iter().cloned());
        }
        if let Some(t) = ctx.get_global(name) {
            used.extend(crate::term::free_level_vars(t));
        }
        for (_, t) in ctx.get_constructors(name) {
            used.extend(crate::term::free_level_vars(t));
        }
    }
    let mut out = Vec::with_capacity(count);
    let mut next = 0usize;
    while out.len() < count {
        let cand = if next == 0 {
            "u".to_string()
        } else {
            format!("u{next}")
        };
        next += 1;
        if !used.contains(&cand) {
            used.insert(cand.clone());
            out.push(cand);
        }
    }
    out
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
    ctx: &Context,
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
        // Recursive fields may be hidden behind reducible aliases. Normalize only
        // the field used for occurrence analysis: the original constructor result
        // indices must retain their syntax for singleton/large-elimination checks.
        if !crate::positivity::mentions_inductive_block(ctx, block, &t) {
            recursive.push(None);
            value_types.push(t);
            continue;
        }
        let reduced = crate::normalize_checked(ctx, &t).map_err(|(_, reason)| {
            KernelError::CertificationError(format!(
                "auto-recursor: recursive-field normalization unfinished: {reason:?}"
            ))
        })?;
        let (tele, occ) = peel_pis(&reduced);
        // Recursive iff the occurrence's head is ANY block member; record which one so
        // the minor premise's IH and the recursive call route to that member's motive/fix.
        if let Some(member) = head_global(occ).and_then(|h| block.iter().position(|m| *m == h)) {
            let indices: Vec<Term> = app_args(occ).into_iter().skip(num_params).collect();
            recursive.push(Some(RecOcc {
                member,
                tele,
                indices,
            }));
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

    Ok(CtorAnalysis {
        value_types,
        recursive,
        result_indices,
    })
}

/// The minor-premise type for one constructor:
/// `Π(a₀:T₀)…Π(aₘ:Tₘ). Π(IHⱼ : P eⱼ… aⱼ per recursive arg). P r… (C A… a₀ … aₘ)`,
/// where `r…` are the constructor's result indices and `eⱼ…` the recursive occurrence's.
fn minor_type(
    ctx: &Context,
    owner_motive: &str,
    motive_names: &[String],
    cname: &str,
    tp_names: &[String],
    an: &CtorAnalysis,
) -> Term {
    let mut ctor_applied = declaration_head(ctx, cname);
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
    while let Term::Pi {
        param,
        param_type,
        body_type,
    } = cur
    {
        params.push((param.clone(), (**param_type).clone()));
        cur = body_type;
    }
    (params, cur)
}

/// Preserve a registered declaration's universe telescope in generated references.
fn declaration_head(ctx: &Context, name: &str) -> Term {
    match ctx.poly_declaration(name) {
        Some((params, _)) if !params.is_empty() => Term::Const {
            name: name.to_string(),
            levels: params.iter().cloned().map(Universe::Var).collect(),
        },
        _ => Term::Global(name.to_string()),
    }
}

/// The global or universe-applied constant at the head of an application spine, if any (`List A B` → `List`).
fn head_global(t: &Term) -> Option<&str> {
    let mut cur = t;
    while let Term::App(f, _) = cur {
        cur = f;
    }
    match cur {
        Term::Global(n) | Term::Const { name: n, .. } => Some(n),
        _ => None,
    }
}

/// Substitute the free variable `old` by `repl` (binder-respecting).
fn subst_name(t: &Term, old: &str, repl: &Term) -> Term {
    match t {
        Term::Var(n) if n == old => repl.clone(),
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
            param_type: Box::new(subst_name(param_type, old, repl)),
            body_type: if param == old {
                body_type.clone()
            } else {
                Box::new(subst_name(body_type, old, repl))
            },
        },
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(subst_name(param_type, old, repl)),
            body: if param == old {
                body.clone()
            } else {
                Box::new(subst_name(body, old, repl))
            },
        },
        Term::App(f, a) => Term::App(
            Box::new(subst_name(f, old, repl)),
            Box::new(subst_name(a, old, repl)),
        ),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(subst_name(discriminant, old, repl)),
            motive: Box::new(subst_name(motive, old, repl)),
            cases: cases.iter().map(|c| subst_name(c, old, repl)).collect(),
        },
        Term::Fix { name, body } => Term::Fix {
            name: name.clone(),
            body: if name == old {
                body.clone()
            } else {
                Box::new(subst_name(body, old, repl))
            },
        },
        Term::MutualFix { defs, index } => {
            // Every def name binds in every body; `old` is shadowed iff it matches any.
            if defs.iter().any(|(n, _)| n == old) {
                t.clone()
            } else {
                Term::MutualFix {
                    defs: defs
                        .iter()
                        .map(|(n, b)| (n.clone(), subst_name(b, old, repl)))
                        .collect(),
                    index: *index,
                }
            }
        }
        Term::Let {
            name,
            ty,
            value,
            body,
        } => Term::Let {
            name: name.clone(),
            ty: Box::new(subst_name(ty, old, repl)),
            value: Box::new(subst_name(value, old, repl)),
            body: if name == old {
                body.clone()
            } else {
                Box::new(subst_name(body, old, repl))
            },
        },
        Term::Annotated { annotation, term } => Term::Annotated {
            annotation: annotation.clone(),
            term: Box::new(subst_name(term, old, repl)),
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
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn pi(p: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}
fn gl(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn apps(f: Term, args: &[Term]) -> Term {
    args.iter().fold(f, |acc, a| app(acc, a.clone()))
}
fn lams(tele: &[(String, Term)], body: Term) -> Term {
    tele.iter()
        .rev()
        .fold(body, |acc, (n, t)| lam(n, t.clone(), acc))
}
/// `Eq T a b` — the prelude's homogeneous equation at carrier `T`.
fn eq3(t: Term, a: Term, b: Term) -> Term {
    apps(gl("Eq"), &[t, a, b])
}
/// `Prod A B`.
fn prod(a: Term, b: Term) -> Term {
    apps(gl("Prod"), &[a, b])
}
/// `Decidable p`.
fn dec(p: Term) -> Term {
    app(gl("Decidable"), p)
}
/// `Not p`.
fn not_(p: Term) -> Term {
    app(gl("Not"), p)
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recursor/helper_tests.rs"]
mod helper_tests;

#[path = "recursor_bounded.rs"]
pub(crate) mod bounded;
