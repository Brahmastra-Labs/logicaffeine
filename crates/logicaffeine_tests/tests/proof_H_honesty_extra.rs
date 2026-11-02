//! Lane H (honesty) — **the proof crate's `#print axioms`, held to the kernel's arm for arm.**
//!
//! There are three doors in this workspace that answer "what does this proof rest on?":
//!
//! | closure | door |
//! |---|---|
//! | `logicaffeine_kernel::axiom_dependencies` | certificates, `Print Axioms`, the REPL — held against Lean's `collect` in `logicaffeine_kernel/tests/H_honesty_extra.rs` |
//! | `logicaffeine_kernel::axiom_dependencies_of` | `Print Axioms <name>.` — delegates to the above |
//! | `logicaffeine_proof::verify::VerifiedProof::axiom_footprint` | **every proof verdict this crate returns** — reads the kernel's closure off the proof term |
//!
//! The third is the one whose doc comment calls it "the only thing that catches an admission":
//! `double_check` cannot see an added axiom (it asks both kernels the same question against the
//! same context), so an under-report here is the highest-consequence one in the workspace.
//!
//! Lean's `collect` (`src/lean/Lean/Util/CollectAxioms.lean` in the v4.14.0 toolchain)
//! spells out the arms the footprint must walk, and the context below reaches one smuggled
//! assumption through each of them:
//!
//!   * **`:29-30` `defnInfo`/`thmInfo` walk `v.type` AND `v.value`** — an assumption named only in
//!     a definition's TYPE.
//!   * **`:33` `ctorInfo` walks the constructor's type; `:35` `inductInfo` walks the arity and
//!     every constructor** — an assumption named only in a constructor's type, reachable from the
//!     constructor and from the bare inductive name.
//!
//! **This file is a lock over one closure seen through two doors.** It asserts the footprint and
//! the kernel's closure agree EXACTLY on every route, and that the assumption lands in the slot
//! an admission lands in. Let the two walks diverge in either direction and it goes red.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{axiom_dependencies, Context, Term, Universe};
use logicaffeine_proof::verify::{AxiomFootprint, VerifiedProof};
use std::collections::BTreeSet;

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}

fn arrow(a: Term, b: Term) -> Term {
    Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}

/// A context carrying a smuggled admission reachable by three different routes, so one fix cannot
/// satisfy the census by accident.
///
/// * `Admits : Prop` is the assumption. Nothing in a rebuilt `StandardLibrary` declares it, so
///   wherever it IS reported it lands in `AxiomFootprint::introduced` — the slot the doc comment
///   calls "the slot a `sorryAx` lands in".
/// * `byFiat : Admits` proves it out of nowhere. Reachable through a BODY.
/// * `viaType : Admits := byFiat` — a definition whose TYPE names the assumption and whose body
///   does not.
/// * `wrap : Admits → Wrap` — a constructor whose TYPE names the assumption.
fn smuggled_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx.add_declaration("Admits", Term::Sort(Universe::Prop));
    ctx.add_declaration("byFiat", g("Admits"));
    ctx.add_definition("viaType".to_string(), g("Admits"), g("byFiat"));
    ctx.add_inductive("Wrap", Term::Sort(Universe::Prop));
    ctx.add_constructor("wrap", "Wrap", arrow(g("Admits"), g("Wrap")));
    assert!(
        ctx.refused_declarations().is_empty(),
        "the fixture itself must register cleanly, or the census is measuring a broken context \
         rather than a broken walk: {:?}",
        ctx.refused_declarations()
    );
    ctx
}

fn footprint(ctx: &Context, term: &Term) -> AxiomFootprint {
    VerifiedProof {
        derivation: None,
        proof_term: Some(term.clone()),
        kernel_ctx: ctx.clone(),
        verified: true,
        verification_error: None,
    }
    .axiom_footprint()
}

fn footprint_names(ctx: &Context, term: &Term) -> BTreeSet<String> {
    let fp = footprint(ctx, term);
    fp.trusted.into_iter().chain(fp.introduced).collect()
}

fn kernel_names(ctx: &Context, term: &Term) -> BTreeSet<String> {
    let usage = axiom_dependencies(ctx, term);
    usage.axioms.into_iter().chain(usage.unresolved).collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| (*s).to_string()).collect()
}

/// THE POSITIVE CONTROL, first: the kernel's closure — the one this campaign made Lean-conformant
/// — finds the admission by every route. Without this, a census showing "the footprint misses it"
/// could just mean the fixture never put it anywhere findable.
#[test]
fn the_kernel_closure_finds_the_admission_by_every_route() {
    let ctx = smuggled_ctx();
    assert_eq!(
        kernel_names(&ctx, &g("byFiat")),
        set(&["byFiat"]),
        "an assumption named directly must be reported"
    );
    assert_eq!(
        kernel_names(&ctx, &g("viaType")),
        set(&["Admits", "byFiat"]),
        "a definition's TYPE is a dependency — Lean's `defnInfo` arm is \
         `collectExpr v.type *> collectExpr v.value` (CollectAxioms.lean:29)"
    );
    assert_eq!(
        kernel_names(&ctx, &g("wrap")),
        set(&["Admits"]),
        "a constructor's TYPE is a dependency — Lean's `ctorInfo` arm is `collectExpr v.type` \
         (CollectAxioms.lean:33)"
    );
}

/// What `logicaffeine_proof`'s own report says about the same terms: the admission by every
/// route, and always in `introduced` — the slot the footprint's doc comment calls "the slot a
/// `sorryAx` lands in". Nothing smuggled may ever be reported as part of the trusted base.
#[test]
fn the_proof_crates_axiom_footprint_walks_every_one_of_leans_arms() {
    let ctx = smuggled_ctx();

    // A name cited directly by the proof term.
    assert_eq!(
        footprint_names(&ctx, &g("byFiat")),
        set(&["byFiat"]),
        "an assumption the term names outright must reach the footprint, or this walk is not \
         doing its job at all"
    );

    // A definition's TYPE — Lean's `defnInfo` arm is `collectExpr v.type *> collectExpr v.value`
    // (CollectAxioms.lean:29-30).
    assert_eq!(
        footprint_names(&ctx, &g("viaType")),
        set(&["Admits", "byFiat"]),
        "`viaType : Admits := byFiat` rests on the assumption `Admits`, named only in its TYPE"
    );

    // A constructor's TYPE, reachable from the constructor (:33) AND from the bare inductive
    // name (:35). Every term of type `Wrap` mentions `Wrap`, so this is the route an ORDINARY
    // proof about an ordinary inductive takes.
    assert_eq!(
        footprint_names(&ctx, &g("wrap")),
        set(&["Admits"]),
        "`wrap : Admits → Wrap` rests on `Admits`, named only in the constructor's type"
    );
    assert_eq!(
        footprint_names(&ctx, &g("Wrap")),
        set(&["Admits"]),
        "the inductive `Wrap` rests on what its constructors assume — Lean's `inductInfo` walks \
         the arity and then every constructor by name"
    );

    // The slot. `Admits` is nowhere in a rebuilt prelude, so wherever it is reported it is
    // INTRODUCED, never trusted.
    for name in ["byFiat", "viaType", "wrap", "Wrap"] {
        let fp = footprint(&ctx, &g(name));
        assert!(
            fp.trusted.is_empty(),
            "`{name}`: a smuggled assumption must never be reported as trusted base: {fp:?}"
        );
    }
}

/// The two closures answer the same question, so their agreement is stated as a DIFFERENCE in
/// both directions rather than as two independent snapshots — a snapshot pair can agree by both
/// being wrong.
///
/// `logicaffeine_kernel::axiom_dependencies` is the reference because it is the one held against
/// Lean's `collect` arm for arm (`logicaffeine_kernel/tests/H_honesty_extra.rs`), and because it is
/// the closure the certificate checker and the REPL already use. Two doors onto one graph giving
/// two answers is the failure that makes an audit trail worthless.
#[test]
fn the_proof_crates_report_is_the_kernels_closure_with_no_shortfall() {
    let ctx = smuggled_ctx();
    let mut shortfall = Vec::new();
    for name in ["byFiat", "viaType", "wrap", "Admits", "Wrap"] {
        let term = g(name);
        let kernel = kernel_names(&ctx, &term);
        let footprint = footprint_names(&ctx, &term);
        let extra: BTreeSet<&String> = footprint.difference(&kernel).collect();
        assert!(
            extra.is_empty(),
            "`{name}`: the proof crate reports {extra:?}, which the kernel's Lean-conformant \
             closure does not. An over-report here would inflate every trusted-base claim this \
             crate makes"
        );
        let missed: Vec<String> = kernel.difference(&footprint).cloned().collect();
        if !missed.is_empty() {
            shortfall.push(format!("{name} -> {missed:?}"));
        }
    }
    assert!(
        shortfall.is_empty(),
        "the proof crate's footprint misses what the kernel's Lean-conformant closure reports — \
         an admission just became harder to see at the surface that the doc comment calls the \
         only thing that catches one: {shortfall:?}"
    );
}

/// …and every route above is reachable by a term that TYPE-CHECKS.
///
/// A lock over terms the kernel would reject proves nothing — the report would never be
/// consulted. `viaType`, `wrap` and `Wrap` all infer, so a real verified proof can cite any of
/// them, and the footprint of that proof is the one the locks above pin.
#[test]
fn the_terms_the_footprint_is_held_on_are_terms_the_kernel_actually_accepts() {
    let ctx = smuggled_ctx();
    for name in ["viaType", "wrap", "Wrap"] {
        let inferred = logicaffeine_kernel::infer_type(&ctx, &g(name));
        assert!(
            inferred.is_ok(),
            "`{name}` must type-check, or a proof could never cite it and the locks above are \
             about an unreachable state: {inferred:?}"
        );
    }
}
