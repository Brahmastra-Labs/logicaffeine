//! Lane H (honesty) — **our `#print axioms` against Lean's, transcribed arm for arm.**
//!
//! `#print axioms` is the single claim a Lean person tests first, because it is the only thing
//! that can tell a proof from a `sorry` wearing a proof's clothes. Ours is
//! `prelude::axiom_dependencies`; Lean's is `Lean/Util/CollectAxioms.lean`. Wave 1 and wave 2 both
//! reported it from reading OUR code alone, and both got the finding half-right — so this file
//! does the other thing: it transcribes Lean's `collect` onto our `Context`, arm for arm, with
//! every arm carrying its `file:line`, and DIFFS the two over the whole prelude.
//!
//! ```text
//! Lean/Util/CollectAxioms.lean:27-36     collect c = match env.find? c with
//!   :28  axiomInfo   -> record the name; DO NOT walk its type
//!   :29  defnInfo    -> walk type AND value
//!   :30  thmInfo     -> walk type AND value          (a theorem's STATEMENT is a dependency)
//!   :31  opaqueInfo  -> walk type AND value          (no LOGOS analogue)
//!   :32  quotInfo    -> pure ()                      (Quot, Quot.mk, Quot.lift, Quot.ind: NOTHING)
//!   :33  ctorInfo    -> walk the constructor's type
//!   :34  recInfo     -> walk the recursor's type
//!   :35  inductInfo  -> walk the arity, then every constructor BY NAME
//!   :36  none        -> pure ()                      (an unresolvable name is SILENTLY IGNORED)
//! ```
//!
//! Lean's `collectExpr` is `e.getUsedConstants.forM collect` (`:22`), and `getUsedConstants`
//! (`Lean/Util/FoldConsts.lean:50`) folds `.const` occurrences over the expression tree
//! (`:20-38`), visiting every sub-expression that can hold one.
//!
//! **This file is a lock, not a wish.** It asserts two things and nothing else:
//!
//!   1. **We never report FEWER axioms than Lean would.** Under-reporting is the only direction
//!      that can turn an admitted proof into a finished one, so it is zero, and it stays zero.
//!   2. **Where we report MORE, the excess is EXACTLY the census below** — each entry named, with
//!      the Lean arm that explains it. Change the closure and this goes red with the delta in the
//!      message, which is how a deliberate improvement gets told apart from a silent regression.
//!
//! The grid is not a handful of hand-picked terms: it is **every name the standard library
//! registers**, which is the only grid guaranteed to contain the adversarial one.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{axiom_dependencies, axiom_dependencies_of, Context, Term};
use std::collections::{BTreeMap, BTreeSet};

fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

/// Every constant name occurring anywhere in a term — our reading of Lean's
/// `Expr.getUsedConstants` (`Lean/Util/FoldConsts.lean:50`, folding `.const` over `:20-38`).
///
/// Written here rather than reused from `prelude.rs` on purpose: a differential in which both
/// sides share the traversal cannot see a traversal bug, which is the same reason `double_check`
/// exists. The `match` is exhaustive with no `_` arm, so a new `Term` variant is a COMPILE error
/// here — a global hiding in a field nobody walks is invisible to `#print axioms`, and that is a
/// path to reporting a `sorry`-tainted proof as clean.
fn used_constants(t: &Term, out: &mut Vec<String>) {
    match t {
        Term::Global(n) => out.push(n.clone()),
        Term::Const { name, .. } => out.push(name.clone()),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            used_constants(param_type, out);
            used_constants(body_type, out);
        }
        Term::Lambda {
            param_type, body, ..
        } => {
            used_constants(param_type, out);
            used_constants(body, out);
        }
        Term::App(f, a) => {
            used_constants(f, out);
            used_constants(a, out);
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            used_constants(discriminant, out);
            used_constants(motive, out);
            for c in cases {
                used_constants(c, out);
            }
        }
        Term::Fix { body, .. } => used_constants(body, out),
        Term::MutualFix { defs, .. } => {
            for (_, d) in defs {
                used_constants(d, out);
            }
        }
        Term::Let {
            ty, value, body, ..
        } => {
            used_constants(ty, out);
            used_constants(value, out);
            used_constants(body, out);
        }
        Term::Annotated { term, .. } => used_constants(term, out),
        Term::Sort(_) | Term::Var(_) | Term::Lit(_) | Term::Hole => {}
    }
}

/// The names Lean's kernel classifies `ConstantInfo.quotInfo` — `QuotKind` is exactly four
/// (`Lean/Declaration.lean:380-384`: `type`, `ctor`, `lift`, `ind`), introduced together by
/// `init_quot` (`Init/Prelude.lean:393`). `collect` reports NOTHING for any of them
/// (`Lean/Util/CollectAxioms.lean:32`).
///
/// `Quot.sound` is deliberately NOT here: it is an ordinary `axiom` (`Init/Core.lean:1543`) and
/// one of the three Lean trusts. Our `Quot_rec` is deliberately not here either — Lean's
/// `Quot.rec` is a definition (`:29`, `Init/Core.lean:1581-1611`), and so is ours.
const LEAN_QUOT_PRIMITIVES: &[&str] = &["Quot", "Quot_mk", "Quot_lift", "Quot_ind"];

/// `Lean/Util/CollectAxioms.lean:21-36`, transcribed onto our `Context`.
fn lean_collect(ctx: &Context, term: &Term) -> BTreeSet<String> {
    let mut axioms = BTreeSet::new();
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut queue: Vec<Term> = vec![term.clone()];
    while let Some(t) = queue.pop() {
        let mut names = Vec::new();
        used_constants(&t, &mut names);
        for name in names {
            if !visited.insert(name.clone()) {
                continue;
            }
            if ctx.is_definition(&name) {
                // :29 defnInfo / :30 thmInfo — `collectExpr v.type *> collectExpr v.value`.
                if let Some(ty) = ctx.get_definition_type(&name) {
                    queue.push(ty.clone());
                }
                if let Some(body) = ctx.get_definition_body(&name) {
                    queue.push(body.clone());
                }
            } else if LEAN_QUOT_PRIMITIVES.contains(&name.as_str()) {
                // :32 quotInfo — `pure ()`. Not recorded, and its type is not even walked.
            } else if ctx.is_inductive(&name) {
                // :35 inductInfo — `collectExpr v.type *> v.ctors.forM collect`: the arity, then
                // each constructor BY NAME (which lands on :33 and walks that ctor's type).
                if let Some(arity) = ctx.get_global(&name) {
                    queue.push(arity.clone());
                }
                for (ctor, _) in ctx.get_constructors(&name) {
                    queue.push(Term::Global(ctor.to_string()));
                }
            } else if ctx.is_constructor(&name) {
                // :33 ctorInfo — `collectExpr v.type`.
                if let Some(ty) = ctx.get_global(&name) {
                    queue.push(ty.clone());
                }
            } else if ctx.get_global(&name).is_some() {
                // :28 axiomInfo — push the NAME. Lean does not walk an axiom's type.
                axioms.insert(name);
            } else {
                // :36 none — an unresolvable name is silently ignored.
            }
        }
    }
    axioms
}

/// Every name the standard library registers, in one list. The grid.
fn every_registered_name(ctx: &Context) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    for (n, _) in ctx.iter_declarations() {
        out.insert(n.to_string());
    }
    for (n, _, _) in ctx.iter_definitions() {
        out.insert(n.to_string());
    }
    for (n, _) in ctx.iter_inductives() {
        out.insert(n.to_string());
        for (c, _) in ctx.get_constructors(n) {
            out.insert(c.to_string());
        }
    }
    out.into_iter().collect()
}

/// The ONLY names our closure may report that Lean's would not, and the reason for each: NONE.
///
/// The census held the four `ConstantInfo.quotInfo` formers — `Quot`, `Quot_mk`, `Quot_lift`,
/// `Quot_ind`, reported by Lean's `#print axioms` as nothing at all
/// (`Lean/Util/CollectAxioms.lean:32`) — while LOGOS counted them as assumptions. They carry
/// registered primitive identities now (`logos-coc-5`) and are not assumption leaves, and
/// `Quot_rec` is a checked definition (`logos-coc-6`), so the census is empty. A name appearing
/// here again is a constant that is not an assumption being counted as one.
const OVER_REPORTED_VS_LEAN: &[&str] = &[];

#[test]
fn our_print_axioms_never_reports_fewer_axioms_than_leans_would() {
    let ctx = std_ctx();
    let mut under: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for name in every_registered_name(&ctx) {
        let term = Term::Global(name.clone());
        let ours = axiom_dependencies(&ctx, &term).axioms;
        let theirs = lean_collect(&ctx, &term);
        let missed: BTreeSet<String> = theirs.difference(&ours).cloned().collect();
        if !missed.is_empty() {
            under.insert(name, missed);
        }
    }
    assert!(
        under.is_empty(),
        "our `#print axioms` reports FEWER axioms than Lean's `collect` \
         (Lean/Util/CollectAxioms.lean:21-36) does for the same constant. Under-reporting is the \
         one direction that can present an ADMITTED proof as a finished one — a `sorry` reachable \
         only through the arm we skipped is invisible at every surface we have. Each entry is \
         `constant -> the axioms Lean would name and we do not`:\n  {:#?}",
        under
    );
}

#[test]
fn where_we_report_more_than_lean_the_excess_is_exactly_the_census() {
    let ctx = std_ctx();
    let mut over: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for name in every_registered_name(&ctx) {
        let term = Term::Global(name.clone());
        let ours = axiom_dependencies(&ctx, &term).axioms;
        let theirs = lean_collect(&ctx, &term);
        let extra: BTreeSet<String> = ours.difference(&theirs).cloned().collect();
        if !extra.is_empty() {
            over.insert(name, extra);
        }
    }
    let names: BTreeSet<String> = over.values().flatten().cloned().collect();
    let expected: BTreeSet<String> = OVER_REPORTED_VS_LEAN
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    assert_eq!(
        names, expected,
        "the set of names we report and Lean does not has changed. If the difference SHRANK, a \
         lane closed the quotient over-report and this census must shrink with it; if it GREW, a \
         constant that is not an assumption is now being counted as one, and every trusted-base \
         number we publish just went up for no reason. Per constant:\n  {:#?}",
        over
    );

    // …and the census is measured on `funext` — the exact constant a Lean person types
    // `#print axioms` at, because `funext` being PROVED rather than assumed is the headline claim
    // of a small trusted base. Lean proves it at `Init/Core.lean:1890` (the proof is
    // `exact congrArg extfunApp (Quot.sound h)`, `:1898`) and `#print axioms funext` answers
    // `[Quot.sound]` — ONE name. Ours must answer exactly that name.
    let for_funext = over.get("funext").cloned().unwrap_or_default();
    assert!(
        for_funext.is_empty(),
        "`funext` is the constant this difference is measured on in public. Lean reports \
         `[Quot.sound]` and we must report nothing beyond it; we report {for_funext:?} more"
    );
    assert_eq!(
        axiom_dependencies(&ctx, &Term::Global("funext".to_string())).axioms,
        ["Quot_sound"]
            .iter()
            .map(|s| (*s).to_string())
            .collect::<BTreeSet<_>>(),
        "`#print axioms funext` is Lean's `[Quot.sound]`, exactly"
    );

    // The BOUNDARY, one rung either side of `funext`: an axiom's own closure must be the name and
    // nothing else (Lean's `axiomInfo` arm records and does not walk the type,
    // CollectAxioms.lean:28), so it can never be a source of this difference.
    for axiom in ["Quot_sound", "propext"] {
        let closure = axiom_dependencies(&ctx, &Term::Global(axiom.to_string())).axioms;
        assert_eq!(
            closure,
            [axiom.to_string()].into_iter().collect::<BTreeSet<_>>(),
            "`{axiom}` is an axiom; its closure is itself. Walking an axiom's TYPE would make \
             every quotient axiom drag the primitives in through a second door and make the \
             census above unfixable"
        );
        assert!(
            !over.contains_key(axiom),
            "…and therefore it cannot over-report: {over:#?}"
        );
    }
}

/// THE BOUNDARY, one rung either side: the quotient arm must be the ONLY thing separating the two
/// closures. Teach the Lean model that our four names are ordinary axioms, and the two answers
/// must become identical — every name, every constant, no residue.
///
/// Without this, the census above is satisfiable by a closure that is wrong in some *other* way
/// whose error happens to land on the same four names.
#[test]
fn with_the_quotient_arm_removed_the_two_closures_agree_exactly() {
    let ctx = std_ctx();
    for name in every_registered_name(&ctx) {
        let term = Term::Global(name.clone());
        let ours = axiom_dependencies(&ctx, &term).axioms;
        let mut theirs = lean_collect(&ctx, &term);
        // Re-derive what the Lean model would have said with `quotInfo` treated as `axiomInfo`:
        // any primitive we reported, Lean's walk would also have reached.
        for q in OVER_REPORTED_VS_LEAN {
            if ours.contains(*q) {
                theirs.insert((*q).to_string());
            }
        }
        assert_eq!(
            ours, theirs,
            "with the `quotInfo` arm (Lean/Util/CollectAxioms.lean:32) neutralised, our closure \
             and Lean's must be the SAME function. They differ on `{name}`, so there is a second \
             divergence hiding behind the quotient one"
        );
    }
}

/// An unresolvable name: Lean's `none` arm is `pure ()` — it is **silently dropped**
/// (`Lean/Util/CollectAxioms.lean:36`). Ours puts it in `AxiomUsage::unresolved`.
///
/// This is a DELIBERATE improvement and the argument is short: a proof minted against a larger
/// base than the one re-checking it cites names this environment does not have, and that is
/// trusted-base DRIFT — the most dangerous state a certificate can be in, because the checker has
/// no idea what it was asked to assume. Lean never faces it (one environment, one session);
/// a re-checkable certificate does, every time it crosses a machine. Silently dropping the name
/// would report such a proof as resting on nothing at all.
#[test]
fn a_name_the_trusted_base_does_not_have_is_drift_where_lean_drops_it() {
    let ctx = std_ctx();
    let ghost = Term::Global("no_such_constant_anywhere".to_string());

    let theirs = lean_collect(&ctx, &ghost);
    assert!(
        theirs.is_empty(),
        "the transcription is wrong: Lean's `none` arm is `pure ()` and must yield nothing, got \
         {theirs:?}"
    );

    let ours = axiom_dependencies(&ctx, &ghost);
    assert!(
        ours.axioms.is_empty(),
        "an unresolvable name is not an AXIOM — calling it one would let a typo silently enlarge \
         the trusted base: {:?}",
        ours.axioms
    );
    assert!(
        ours.unresolved.contains("no_such_constant_anywhere"),
        "an unresolvable name must be reported as trusted-base DRIFT. Lean drops it \
         (CollectAxioms.lean:36) because Lean has one environment; a certificate re-checked \
         elsewhere does not, and a dropped name is a proof that reports resting on nothing while \
         citing something this checker never declared: {:?}",
        ours
    );

    // The NEGATIVE: a name that DOES resolve must never land in `unresolved`, or the improvement
    // degenerates into "everything is drift" and stops carrying information.
    let real = axiom_dependencies(&ctx, &Term::Global("Quot_sound".to_string()));
    assert!(
        real.unresolved.is_empty(),
        "a declared axiom is not drift: {:?}",
        real.unresolved
    );
    assert!(
        real.axioms.contains("Quot_sound"),
        "…and it must be reported as the axiom it is: {:?}",
        real.axioms
    );
}

/// The two DOORS onto one graph must agree. `axiom_dependencies` walks a TERM;
/// `axiom_dependencies_of` answers `Print Axioms <name>.` — and for anything that is not a
/// definition it walks the constant's TYPE and then adds the name, where Lean's `axiomInfo` arm
/// records the name and walks nothing (`Lean/Util/CollectAxioms.lean:28`).
///
/// That difference is real and it is recorded rather than asserted away: an audit trail whose two
/// entrances give two answers is the failure that makes the trail worthless, because a reader
/// cannot tell which door they came in by. What is pinned here is the invariant that survives
/// either choice — the named door must never report LESS than the term door.
#[test]
fn the_named_door_and_the_term_door_never_disagree_downward() {
    let ctx = std_ctx();
    let mut worse = Vec::new();
    for name in every_registered_name(&ctx) {
        let by_term = axiom_dependencies(&ctx, &Term::Global(name.clone())).axioms;
        let by_name = axiom_dependencies_of(&ctx, &name)
            .unwrap_or_else(|| panic!("`{name}` is registered, so it has a closure"))
            .axioms;
        let missed: BTreeSet<String> = by_term.difference(&by_name).cloned().collect();
        if !missed.is_empty() {
            worse.push(format!("{name}: `Print Axioms` omits {missed:?}"));
        }
    }
    assert!(
        worse.is_empty(),
        "`Print Axioms <name>.` reports fewer axioms than walking `Global(name)` does. The REPL \
         and the certificate checker would then disagree about what a proof rests on, and the \
         reader has no way to tell which door they came in by:\n  {}",
        worse.join("\n  ")
    );
}

/// **A half-registered prelude is a silent one.**
///
/// `Context::add_definition` is the `()`-returning face of the checked gateway: when the body does
/// not have the declared type it does not raise — it appends to `Context::refused_declarations()`
/// and carries on. So a constant the standard library believes it registered can simply not be
/// there, with nothing red anywhere and every report about it quietly wrong: the name resolves to
/// nothing, so `axiom_dependencies` files it under `unresolved` (trusted-base DRIFT) rather than
/// naming the missing definition, and every proof that cites it fails to infer for a reason that
/// points at the caller instead of at the prelude.
///
/// This is not hypothetical. Measured 2026-09-15 during wave 4, in a window while another lane was
/// mid-edit, `StandardLibrary::register` produced exactly this:
///
/// ```text
/// refused: [("HEq_rec", "Type mismatch: expected Π(A0:Type0). Π(P:Π(Idx0:Type0). … ,
///                        found Π(A:Type0). Π(P:Π(B:Type0). …")]
/// ```
///
/// — the heterogeneous-equality eliminator, dropped, with the suite green. It re-registered when
/// the edit landed, so the finding is not that `HEq_rec` is broken; the finding is that **nothing
/// in the workspace was watching the channel**, and a prelude can lose a constant without a single
/// test going red. `refused_declarations`'s own doc says "Empty is the healthy state and the thing
/// to lock." This locks it.
#[test]
fn the_standard_library_registers_with_nothing_silently_refused() {
    let ctx = std_ctx();
    let refused: Vec<String> = ctx
        .refused_declarations()
        .iter()
        .map(|(name, why)| format!("{name}: {why}"))
        .collect();
    assert!(
        refused.is_empty(),
        "`StandardLibrary::register` REFUSED {} declaration(s). The checked gateway records a \
         refusal instead of raising, so the prelude is now missing a constant it believes it has — \
         and every report about that name is wrong in a way no other test can see: it resolves to \
         nothing, so `#print axioms` files it as trusted-base drift rather than as a missing \
         definition.\n  {}",
        refused.len(),
        refused.join("\n  ")
    );
}

/// The guard's guard. A lock on "no refusals" is worthless if the channel it reads is dead — an
/// empty list because nothing is ever recorded looks exactly like an empty list because everything
/// registered. So plant one: a definition whose body definitely does not have its declared type
/// must appear in `refused_declarations()`, and must NOT be silently present in the context.
#[test]
fn a_mistyped_definition_reaches_the_refusal_channel_the_lock_above_reads() {
    let mut ctx = std_ctx();
    assert!(
        ctx.refused_declarations().is_empty(),
        "the prelude starts clean"
    );
    ctx.add_definition(
        "definitely_not_a_prop".to_string(),
        Term::Sort(logicaffeine_kernel::Universe::Prop),
        Term::Global("Nat".to_string()),
    );
    let names: Vec<&str> = ctx
        .refused_declarations()
        .iter()
        .map(|(n, _)| n.as_str())
        .collect();
    assert_eq!(
        names,
        ["definitely_not_a_prop"],
        "a body that does not have its declared type must be RECORDED as refused. If this list is \
         empty the channel is dead and `the_standard_library_registers_with_nothing_silently_\
         refused` is green for the wrong reason — it would read clean through a prelude that \
         dropped every definition it has"
    );
    assert!(
        !ctx.is_definition("definitely_not_a_prop"),
        "a refused definition must not also be REGISTERED — a context that both refuses and keeps \
         it is worse than either"
    );
}
