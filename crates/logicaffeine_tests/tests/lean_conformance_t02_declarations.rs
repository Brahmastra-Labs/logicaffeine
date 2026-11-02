//! Tier 2 · DECLARATIONS — the Lean-4 conformance corpus, re-read as a capability matrix.
//!
//! One fact drives almost this whole tier: LOGOS's `Context` is fifteen parallel `HashMap`s keyed
//! by a flat `String`. There is no `Name` type, no namespace component, no visibility, no
//! attribute table and no reducibility hint — so Lean's *declaration* layer (which is mostly
//! metadata around a term) has to land in four doors and nothing else: `add_inductive`,
//! `add_constructor`, `add_definition` (transparent, body-carrying) and `add_declaration`
//! (opaque axiom). Everything Lean records beside the term either maps onto one of the four
//! purpose-built side tables (`instances`, `coercions`, `binder_kinds`, `hints`) or is erased.
//!
//! What that buys us and what it costs is exactly what these twenty cases measure:
//!
//! - `def`/`theorem`/`axiom`/`opaque`/`example` (T02-001/002/003/005/006) all land cleanly, and
//!   the one thing that matters about `opaque` — that its witness must not leak into conversion —
//!   we get *for free*, because a declaration has no body at all.
//! - `mutual def` (T02-014), lifted `where` auxiliaries (T02-015), local `let rec` (T02-016) and
//!   the interleaved `{α} (xs) [Inst α]` telescope a `variable` block produces (T02-018) all work
//!   today.
//! - Reducibility (T02-004), namespaces (T02-011), visibility and scope lifetime (T02-012),
//!   attributes and the simp set (T02-013), derive handlers (T02-017) and `open` (T02-019) have
//!   no carrier at all. Those are the work queue.
//! - `partial def` (T02-007) and `unsafe def` (T02-008) are refused, loudly, and the tests show
//!   why every tempting fallback is worse than refusing.
//!
//! T02-020 (the flat-name collision itself) is confirmed LOGOS defect 3 and is reproduced in
//! `lean_conformance_defects.rs`; this file does not write a second reproduction.

use logicaffeine_kernel::interface::Repl;
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    defeq_for_test, infer_type, normalize, normalize_at, resolve_instance, surface_elaborate,
    Context, KernelError, MetaCtx, ParamKind, Reducibility, Term, Transparency, Universe,
};

// ==== HELPERS ====

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn var(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().fold(f, |a, x| app(a, x.clone()))
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
fn match_(d: Term, motive: Term, cases: Vec<Term>) -> Term {
    Term::Match {
        discriminant: Box::new(d),
        motive: Box::new(motive),
        cases,
    }
}
fn fix(name: &str, body: Term) -> Term {
    Term::Fix {
        name: name.to_string(),
        body: Box::new(body),
    }
}
fn mutfix(defs: &[(&str, Term)], index: usize) -> Term {
    Term::MutualFix {
        defs: defs
            .iter()
            .map(|(n, b)| (n.to_string(), b.clone()))
            .collect(),
        index,
    }
}
fn nat() -> Term {
    g("Nat")
}
fn bool_t() -> Term {
    g("Bool")
}
fn ty0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn num(n: u32) -> Term {
    (0..n).fold(g("Zero"), |t, _| app(g("Succ"), t))
}
fn eq(ty: Term, a: Term, b: Term) -> Term {
    apps(g("Eq"), &[ty, a, b])
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

/// `plus : Nat → Nat → Nat`, structurally recursive on the first argument. The prelude's `add`
/// is an `Int` builtin declaration, so Peano arithmetic on `Nat` is built here.
fn plus_body() -> Term {
    fix(
        "plus",
        lam(
            "n",
            nat(),
            lam(
                "m",
                nat(),
                match_(
                    var("n"),
                    lam("_", nat(), nat()),
                    vec![
                        var("m"),
                        lam(
                            "k",
                            nat(),
                            app(g("Succ"), apps(var("plus"), &[var("k"), var("m")])),
                        ),
                    ],
                ),
            ),
        ),
    )
}

/// The standard library plus `plus`.
fn arith_ctx() -> Context {
    let mut ctx = std_ctx();
    ctx.add_definition(
        "plus".to_string(),
        arrow(nat(), arrow(nat(), nat())),
        plus_body(),
    );
    ctx
}

/// `Pair A B` as a registered structure — a stand-in for Lean's `Prod`, used where a case needs
/// a type-former to abbreviate.
fn with_pair(ctx: &mut Context) {
    ctx.add_structure(
        "Pair",
        &[("A", ty0()), ("B", ty0())],
        &[("fst", var("A")), ("snd", var("B"))],
    );
}

// ==== T02-001 · `def` — a delta-transparent global definition ====

/// `def double (n : Nat) : Nat := n + n`, `def quadruple := double (double n)`,
/// `def Count : Type := Nat`, `def Endo α := α → α`, `def replicateType (n) (α) := Fin n → α`.
/// Transparency is the whole content of the case: every one of these only works because
/// `add_definition` stores a body that `normalize` delta-unfolds.
#[test]
fn a_def_is_a_transparent_global_that_unfolds_through_a_chain() {
    let mut ctx = arith_ctx();

    ctx.add_definition(
        "double".to_string(),
        arrow(nat(), nat()),
        lam("n", nat(), apps(g("plus"), &[var("n"), var("n")])),
    );
    ctx.add_definition(
        "quadruple".to_string(),
        arrow(nat(), nat()),
        lam("n", nat(), app(g("double"), app(g("double"), var("n")))),
    );

    assert!(
        ctx.is_definition("double") && ctx.get_definition_body("double").is_some(),
        "a `def` must register as a DEFINITION with a body, not as an axiom: is_definition={}, body={:?}",
        ctx.is_definition("double"),
        ctx.get_definition_body("double")
    );

    // `example : double 2 = 4 := rfl` — only closes because `double` delta-unfolds.
    assert!(
        defeq_for_test(&ctx, &app(g("double"), num(2)), &num(4)),
        "`double 2 = 4` must close by rfl, i.e. the definition must unfold: double 2 ⇝ {}",
        normalize(&ctx, &app(g("double"), num(2)))
    );
    // `example : quadruple 1 = 4 := rfl` — two definitions deep.
    assert!(
        defeq_for_test(&ctx, &app(g("quadruple"), num(1)), &num(4)),
        "a definition whose body mentions another definition must unfold through both: quadruple 1 ⇝ {}",
        normalize(&ctx, &app(g("quadruple"), num(1)))
    );

    // `def Count : Type := Nat` then `def zeroCount : Count := (0 : Nat)` — the ASCRIBED type
    // arrives through a delta step, so `Zero : Count` may only check if `Count` unfolds.
    ctx.add_definition("Count".to_string(), ty0(), nat());
    ctx.add_definition("zeroCount".to_string(), g("Count"), g("Zero"));
    let zero_ty = infer_type(&ctx, ctx.get_definition_body("zeroCount").unwrap());
    assert!(
        matches!(&zero_ty, Ok(t) if defeq_for_test(&ctx, t, &g("Count"))),
        "`zeroCount : Count := 0` must typecheck through the delta step Count ⇝ Nat: {zero_ty:?}"
    );

    // `def Endo (α : Type) : Type := α → α` then `def idNat : Endo Nat := fun n => n`.
    ctx.add_definition(
        "Endo".to_string(),
        arrow(ty0(), ty0()),
        lam("A", ty0(), arrow(var("A"), var("A"))),
    );
    let id_nat = lam("n", nat(), var("n"));
    ctx.add_definition("idNat".to_string(), app(g("Endo"), nat()), id_nat.clone());
    assert!(
        defeq_for_test(&ctx, &app(g("Endo"), nat()), &arrow(nat(), nat())),
        "a type-level definition must unfold like any other: Endo Nat ⇝ {}",
        normalize(&ctx, &app(g("Endo"), nat()))
    );

    // A DEPENDENT definition: the return type mentions an earlier parameter, so the stored type
    // is a real Π telescope rather than an arrow chain.
    let mut dep = ctx.clone();
    dep.add_indexed_inductive("Fin", arrow(nat(), ty0()), 0);
    dep.add_definition(
        "replicateType".to_string(),
        pi("n", nat(), arrow(ty0(), ty0())),
        lam(
            "n",
            nat(),
            lam("A", ty0(), arrow(app(g("Fin"), var("n")), var("A"))),
        ),
    );
    let inst = infer_type(&dep, &apps(g("replicateType"), &[num(3), nat()]));
    assert!(
        matches!(&inst, Ok(t) if defeq_for_test(&dep, t, &ty0())),
        "a dependent definition's Π telescope must instantiate: replicateType 3 Nat : {inst:?}"
    );
}

// ==== T02-002 · `theorem` — there is no theorem form; it is a `Prop`-typed definition ====

/// Lean's `TheoremVal` is a separate constructor; LOGOS has exactly one body-carrying door, so a
/// theorem *is* `add_definition` with a `Prop`-typed type. The case's danger is the opposite
/// mistranslation — emitting `add_declaration` "because proofs are irrelevant anyway", which turns
/// a proof library into a pile of axioms. `is_definition` is the only witness that survives.
#[test]
fn a_theorem_is_exactly_a_prop_typed_definition_and_keeps_its_body() {
    let mut ctx = arith_ctx();

    // `theorem two_refl : 2 = 2 := rfl` — a Prop-typed definition carrying the proof TERM.
    let stmt = eq(nat(), num(2), num(2));
    let proof = apps(g("refl"), &[nat(), num(2)]);
    let sort = infer_type(&ctx, &stmt);
    assert!(
        matches!(&sort, Ok(Term::Sort(Universe::Prop))),
        "a theorem's statement must live in Prop — that is the ONLY thing distinguishing it from a def: {sort:?}"
    );
    let proved = infer_type(&ctx, &proof);
    assert!(
        matches!(&proved, Ok(t) if defeq_for_test(&ctx, t, &stmt)),
        "the emitted body must be the PROOF TERM, whose type is the statement: {proved:?}"
    );
    ctx.add_definition("two_refl".to_string(), stmt.clone(), proof);

    // `theorem add_zero_zero := zero_add_eq 0` — a theorem used by a later theorem. If the first
    // had been emitted as an axiom this still checks, which is exactly why `is_definition` is the
    // assertion that matters.
    assert!(
        ctx.is_definition("two_refl") && ctx.get_definition_body("two_refl").is_some(),
        "a theorem emitted as an AXIOM is a silent `sorry`: is_definition={}, body={:?}",
        ctx.is_definition("two_refl"),
        ctx.get_definition_body("two_refl")
    );

    // The axiom mistranslation, made visible: the same name via `add_declaration` is
    // indistinguishable at every use site and differs only here.
    let mut as_axiom = arith_ctx();
    as_axiom.add_declaration("two_refl", stmt.clone());
    assert!(
        as_axiom.get_global("two_refl").is_some() && !as_axiom.is_definition("two_refl"),
        "an axiom-shaped theorem still RESOLVES — `is_definition` is the whole provenance record: {:?}",
        as_axiom.is_definition("two_refl")
    );

    // `def isZero (n) : Bool := Nat.beq n 0` then `theorem isZero_zero : isZero 0 = true := rfl`
    // — a theorem whose proof only checks because the `def` it mentions is TRANSPARENT.
    ctx.add_definition(
        "isZero".to_string(),
        arrow(nat(), bool_t()),
        lam(
            "n",
            nat(),
            match_(
                var("n"),
                lam("_", nat(), bool_t()),
                vec![g("true"), lam("_", nat(), g("false"))],
            ),
        ),
    );
    let is_zero_stmt = eq(bool_t(), app(g("isZero"), g("Zero")), g("true"));
    let is_zero_proof = apps(g("refl"), &[bool_t(), g("true")]);
    let checked = infer_type(&ctx, &is_zero_proof);
    assert!(
        matches!(&checked, Ok(t) if defeq_for_test(&ctx, t, &is_zero_stmt)),
        "`isZero 0 = true` by rfl needs the def to unfold: {checked:?}"
    );

    // The asymmetry with Lean, pinned: Lean will not delta-unfold a theorem body during defeq;
    // LOGOS unconditionally will, because `add_definition` has no opacity axis (see T02-004).
    assert!(
        ctx.get_definition_body("two_refl").is_some()
            && defeq_for_test(&ctx, &g("two_refl"), &apps(g("refl"), &[nat(), num(2)])),
        "a LOGOS theorem body is delta-transparent where Lean's is not: two_refl ⇝ {}",
        normalize(&ctx, &g("two_refl"))
    );
}

// ==== T02-003 · `axiom` — an opaque, body-less global ====

/// The whole trust surface of a LOGOS development is `iter_declarations`. There is no
/// `#print axioms`, so a translated Lean axiom joins the prelude's assumed set and the
/// consistency obligation becomes joint. The case's sharpest guard is the `get_global`
/// precedence trap: declarations are looked up LAST, so a name registered as both is invisible
/// as an axiom.
#[test]
fn an_axiom_is_opaque_and_the_declaration_map_is_the_whole_trust_report() {
    let mut ctx = arith_ctx();
    let baseline: Vec<String> = ctx
        .iter_declarations()
        .map(|(n, _)| n.to_string())
        .collect();

    // `axiom mysteryNat : Nat` — a DATA-valued axiom, which Lean also allows.
    ctx.add_declaration("mysteryNat", nat());
    assert!(
        !ctx.is_definition("mysteryNat") && ctx.get_definition_body("mysteryNat").is_none(),
        "an axiom must have NO body — nothing may delta-unfold: is_definition={}, body={:?}",
        ctx.is_definition("mysteryNat"),
        ctx.get_definition_body("mysteryNat")
    );
    assert_eq!(
        normalize(&ctx, &g("mysteryNat")),
        g("mysteryNat"),
        "an axiom is stuck under normalization; if it reduces, a witness leaked into the context"
    );

    // `theorem usesAxiom : mysteryNat = mysteryNat := rfl` — the axiom is usable, so the
    // dependence is real and must show up in the trust report.
    let used = infer_type(&ctx, &apps(g("refl"), &[nat(), g("mysteryNat")]));
    assert!(
        matches!(&used, Ok(t) if defeq_for_test(&ctx, t, &eq(nat(), g("mysteryNat"), g("mysteryNat")))),
        "an axiom must be usable in a proof: {used:?}"
    );

    // `axiom transportLike : ∀ (α) (P) (a b), a = b → P a → P b` — a dependent telescope, stored
    // verbatim with no well-formedness check of its own.
    let transport = pi(
        "A",
        ty0(),
        pi(
            "P",
            arrow(var("A"), prop()),
            pi(
                "a",
                var("A"),
                pi(
                    "b",
                    var("A"),
                    arrow(
                        eq(var("A"), var("a"), var("b")),
                        arrow(app(var("P"), var("a")), app(var("P"), var("b"))),
                    ),
                ),
            ),
        ),
    );
    ctx.add_declaration("transportLike", transport.clone());
    assert_eq!(
        ctx.get_global("transportLike"),
        Some(&transport),
        "a dependent axiom's Π telescope must be stored verbatim"
    );

    // The trust report: `iter_declarations` is the complete audit surface, and the two new
    // axioms sit in it ALONGSIDE the prelude's — the consistency obligation is joint.
    let now: Vec<String> = ctx
        .iter_declarations()
        .map(|(n, _)| n.to_string())
        .collect();
    assert_eq!(
        now.len(),
        baseline.len() + 2,
        "every assumed constant must be visible in the ONE audit surface we have: {} → {}",
        baseline.len(),
        now.len()
    );
    // …and they join the PRELUDE's assumed set, not an empty one: Lean's own three axioms are
    // already in it, while `dne` — derived from them — is a checked theorem and correctly absent.
    for lean_axiom in ["propext", "Classical_choice", "Quot_sound"] {
        assert!(
            now.iter().any(|n| n == lean_axiom),
            "a translated axiom joins the prelude's assumed set, which holds Lean's `{lean_axiom}`: {now:?}"
        );
    }
    assert!(
        !now.iter().any(|n| n == "dne") && ctx.is_definition("dne"),
        "`dne` is derived from those axioms, so it is a definition and never an assumption: {now:?}"
    );

    // The precedence trap: definitions are checked before declarations, so an axiom registered
    // under a name that is also a definition is completely invisible.
    let mut shadowed = arith_ctx();
    shadowed.add_declaration("separateName", arrow(nat(), nat()));
    shadowed.add_definition(
        "separateName".to_string(),
        arrow(nat(), bool_t()),
        lam("_", nat(), g("true")),
    );
    assert!(
        matches!(shadowed.get_global("separateName"), Some(t) if defeq_for_test(&shadowed, t, &arrow(nat(), bool_t()))),
        "get_global checks definitions BEFORE declarations, so the axiom is masked: {:?}",
        shadowed.get_global("separateName").map(|t| t.to_string())
    );
}

// ==== T02-004 · `abbrev` — the `@[reducible]` hint and its carrier ====

/// `abbrev`, plain `def` and `@[irreducible] def` differ in Lean only for the ELABORATOR: Lean's
/// kernel unfolds all three. LOGOS records the attribute on the declaration (`set_reducibility`)
/// and honours it at the elaborator's transparency (`Transparency::Excluding`), while the kernel's
/// conversion — like Lean's — unfolds every definition. The chain `A0 … A3` is here because the
/// *tempting repair* for a missing hint — inlining the abbreviation at every use — is the one move
/// `normalize`'s fuel punishes.
#[test]
fn an_irreducible_definition_still_unfolds_because_there_is_no_reducibility_hint() {
    let mut ctx = arith_ctx();
    with_pair(&mut ctx);

    // `abbrev NatPair := Nat × Nat` vs `def NatPairOpaque := Nat × Nat`.
    let pair_nat_nat = apps(g("Pair"), &[nat(), nat()]);
    ctx.add_definition("NatPair".to_string(), ty0(), pair_nat_nat.clone());
    ctx.add_definition("NatPairOpaque".to_string(), ty0(), pair_nat_nat.clone());
    ctx.set_reducibility("NatPair", Reducibility::Reducible);
    assert!(
        defeq_for_test(&ctx, &g("NatPair"), &g("NatPairOpaque")),
        "the KERNEL unfolds an abbrev and a def alike, as Lean's kernel does"
    );

    // The chain `abbrev A0 := Nat; A1 := A0 → A0; A2 := A1 → A1; A3 := A2 → A2`. The FOLDED form
    // converts fine — which is the argument against a translator that "preserves reducibility"
    // by inlining, since the inlined term grows multiplicatively against a hard fuel.
    ctx.add_definition("A0".to_string(), ty0(), nat());
    ctx.add_definition("A1".to_string(), ty0(), arrow(g("A0"), g("A0")));
    ctx.add_definition("A2".to_string(), ty0(), arrow(g("A1"), g("A1")));
    ctx.add_definition("A3".to_string(), ty0(), arrow(g("A2"), g("A2")));
    let a1 = arrow(nat(), nat());
    let a2 = arrow(a1.clone(), a1.clone());
    let a3 = arrow(a2.clone(), a2);
    assert!(
        defeq_for_test(&ctx, &g("A3"), &a3),
        "a chain of abbreviations must converge under delta without inlining: A3 ⇝ {}",
        normalize(&ctx, &g("A3"))
    );

    // `@[irreducible] def sealedZero : Nat := 0` — the other end of the axis. Lean's ELABORATOR
    // will not unfold it (`sealedZero = 0 := rfl` is refused at default transparency); Lean's
    // KERNEL does.
    ctx.add_definition("sealedZero".to_string(), nat(), g("Zero"));
    ctx.set_reducibility("sealedZero", Reducibility::Irreducible);
    let irreducible = |name: &str| ctx.is_irreducible(name);
    let elaborator = |t: &Term| normalize_at(&ctx, t, Transparency::Excluding(&irreducible));
    assert_eq!(
        elaborator(&g("sealedZero")),
        g("sealedZero"),
        "at the elaborator's transparency an @[irreducible] definition does NOT unfold, so the \
         elaborator cannot see `sealedZero ≡ Zero`"
    );
    assert_ne!(
        elaborator(&g("sealedZero")),
        elaborator(&g("Zero")),
        "…the two stay apart there"
    );
    assert_eq!(
        elaborator(&g("NatPair")),
        normalize(&ctx, &pair_nat_nat),
        "…while the `@[reducible]` end of the axis unfolds at that same transparency"
    );
    assert_eq!(
        ctx.get_definition_body("sealedZero"),
        Some(&g("Zero")),
        "the irreducible definition keeps its body — it is not an axiom"
    );
    assert!(
        defeq_for_test(&ctx, &g("sealedZero"), &g("Zero")),
        "the kernel's conversion unfolds it, exactly as Lean's kernel does"
    );
}

// ==== T02-005 · `example` — an anonymous declaration needs a generated, collision-free name ====

/// Lean names every `example` `_example` and never adds it to the environment; LOGOS's
/// `add_definition` is `HashMap::insert`, so reusing one key keeps only the last. The case is
/// satisfiable — keys are arbitrary `String`s and `get_global` is the freshness oracle — but only
/// if the scheme is *checked* rather than assumed, because a user declaration can be named
/// anything, `_example` and `example_1` included.
#[test]
fn every_anonymous_example_needs_its_own_generated_collision_free_key() {
    let mut ctx = arith_ctx();

    // The user declarations the naive schemes collide with: `def example_1 := 0`, `def _example := 1`.
    ctx.add_definition("example_1".to_string(), nat(), num(0));
    ctx.add_definition("_example".to_string(), nat(), num(1));
    // `def Threshold : Nat := 7` — an example below mentions it, so ordering is real.
    ctx.add_definition("Threshold".to_string(), nat(), num(7));

    // The five `example`s of the case: three identical `1 + 1 = 2`, one with binders, one
    // data-valued, one over `Threshold`.
    let one_plus_one = eq(nat(), apps(g("plus"), &[num(1), num(1)]), num(2));
    let examples: Vec<(Term, Term)> = vec![
        (one_plus_one.clone(), apps(g("refl"), &[nat(), num(2)])),
        (one_plus_one.clone(), apps(g("refl"), &[nat(), num(2)])),
        (one_plus_one, apps(g("refl"), &[nat(), num(2)])),
        (
            pi(
                "n",
                nat(),
                eq(nat(), apps(g("plus"), &[g("Zero"), var("n")]), var("n")),
            ),
            lam("n", nat(), apps(g("refl"), &[nat(), var("n")])),
        ),
        (nat(), num(42)),
        (
            eq(nat(), g("Threshold"), num(7)),
            apps(g("refl"), &[nat(), num(7)]),
        ),
    ];

    // The scheme: generate, then CHECK against the context — the only collision-free rule
    // available when keys are arbitrary strings with no reserved prefix.
    let mut emitted: Vec<String> = Vec::new();
    for (i, (ty, body)) in examples.iter().enumerate() {
        let mut key = format!("example_{i}");
        while ctx.get_global(&key).is_some() {
            key.push('\'');
        }
        let inferred = infer_type(&ctx, body);
        assert!(
            matches!(&inferred, Ok(t) if defeq_for_test(&ctx, t, ty)),
            "example {i} must typecheck against its stated type — that IS its whole value: {inferred:?}"
        );
        ctx.add_definition(key.clone(), ty.clone(), body.clone());
        emitted.push(key);
    }

    assert_eq!(
        emitted.len(),
        6,
        "every `example` in the source produces exactly one registration: {emitted:?}"
    );
    let distinct: std::collections::BTreeSet<&String> = emitted.iter().collect();
    assert_eq!(
        distinct.len(),
        emitted.len(),
        "the generated names must be pairwise distinct, or example N overwrites example N-1: {emitted:?}"
    );
    for key in &emitted {
        assert!(
            ctx.is_definition(key),
            "every emitted example must survive registration: {key} missing"
        );
    }

    // The user names must be untouched — the freshness check is what makes that true.
    assert!(
        defeq_for_test(&ctx, &g("example_1"), &num(0)) && defeq_for_test(&ctx, &g("_example"), &num(1)),
        "a generated name that collides with a user declaration silently replaces it: example_1 ⇝ {}, _example ⇝ {}",
        normalize(&ctx, &g("example_1")),
        normalize(&ctx, &g("_example"))
    );
}

// ==== T02-006 · `opaque` — an axiom plus a discharged inhabitation obligation ====

/// `opaque x : T := v` hides `v` deliberately: `x = v` is unprovable in Lean. LOGOS gets that
/// exactly right for free, because `add_declaration` has no body at all — this is one of the
/// places where having *fewer* declaration forms is the correct behaviour rather than a loss.
/// The test also pins the mistranslation it forbids: emitting the witness as a body makes
/// `x = v` provable by rfl, a term Lean rejects being accepted by us.
#[test]
fn an_opaque_constant_hides_its_witness_from_conversion() {
    let mut ctx = arith_ctx();

    // `opaque secretNat : Nat := 0`
    ctx.add_declaration("secretNat", nat());
    // `example : secretNat = secretNat := rfl` — reflexivity on the opaque head is all Lean gives.
    let refl_ok = infer_type(&ctx, &apps(g("refl"), &[nat(), g("secretNat")]));
    assert!(
        matches!(&refl_ok, Ok(t) if defeq_for_test(&ctx, t, &eq(nat(), g("secretNat"), g("secretNat")))),
        "reflexivity on an opaque head must still check: {refl_ok:?}"
    );
    // `example : secretNat = 0 := rfl` — REJECTED by Lean, and must be rejected here.
    assert!(
        !defeq_for_test(&ctx, &g("secretNat"), &g("Zero")),
        "the witness must not be visible to conversion: secretNat ⇝ {}",
        normalize(&ctx, &g("secretNat"))
    );

    // `opaque hash : Nat → Nat := fun n => n` — only `hash 3 = hash 3` holds, never `hash 3 = 3`.
    ctx.add_declaration("hash", arrow(nat(), nat()));
    assert!(
        defeq_for_test(&ctx, &app(g("hash"), num(3)), &app(g("hash"), num(3)))
            && !defeq_for_test(&ctx, &app(g("hash"), num(3)), &num(3)),
        "a function-typed opaque exposes only its arrow type: hash 3 ⇝ {}",
        normalize(&ctx, &app(g("hash"), num(3)))
    );

    // The forbidden translation, made concrete: the witness as a body.
    let mut leaked = arith_ctx();
    leaked.add_definition("secretNat".to_string(), nat(), g("Zero"));
    assert!(
        defeq_for_test(&leaked, &g("secretNat"), &g("Zero")),
        "emitting the witness as a body would make `secretNat = 0` provable by rfl — the divergence this case forbids"
    );

    // `@[irreducible] def sealed : Nat := 5` is NOT the same thing: it keeps a body, so it lands
    // in `definitions` and unfolds (T02-004). Mapping both onto one form loses a real distinction.
    leaked.add_definition("sealed".to_string(), nat(), num(5));
    assert!(
        leaked.is_definition("sealed") && !ctx.is_definition("secretNat"),
        "`opaque` and `@[irreducible] def` must land in DIFFERENT maps: sealed is_definition={}, secretNat is_definition={}",
        leaked.is_definition("sealed"),
        ctx.is_definition("secretNat")
    );
}

// ==== T02-007 · `partial def` — no termination proof, and no escape hatch ====

/// REFUSED, deliberately. Every recursion in the kernel goes through `Term::Fix`/`MutualFix` and
/// the syntactic Giménez guard; there is no measure, no well-founded escape, no `partial`. Lean
/// admits `partial def` because it is a *conservative* extension — the declaration is opaque and
/// the `Inhabited` instance discharges its logical content. LOGOS's nearest form, `add_declaration`,
/// checks nothing, so translating `partial def bad : Nat → False` would hand us an inconsistency
/// where Lean simply refuses. Refusing is the only sound answer, and the guard does it loudly.
#[test]
fn a_partial_def_is_refused_by_the_structural_guard_and_has_no_safe_fallback() {
    let ctx = arith_ctx();

    // `partial def spin (n : Nat) : Nat := spin (n + 1)` — written so the structural TYPE is
    // still inferrable, leaving termination as the only thing that can reject it.
    let spin = fix(
        "spin",
        lam(
            "n",
            nat(),
            match_(
                var("n"),
                lam("_", nat(), nat()),
                vec![
                    g("Zero"),
                    lam("k", nat(), app(var("spin"), app(g("Succ"), var("k")))),
                ],
            ),
        ),
    );
    let spun = infer_type(&ctx, &spin);
    assert!(
        matches!(&spun, Err(KernelError::TerminationViolation { .. })),
        "a non-descending recursion must be refused by the structural guard, by name: {spun:?}"
    );

    // `partial def collatzSteps` — terminating for every input a human supplies, but not for any
    // reason a syntactic guard can see. This is the case that *looks* translatable.
    let collatz = fix(
        "collatz",
        lam(
            "n",
            nat(),
            match_(
                var("n"),
                lam("_", nat(), nat()),
                vec![
                    g("Zero"),
                    lam(
                        "k",
                        nat(),
                        app(var("collatz"), apps(g("plus"), &[var("k"), var("k")])),
                    ),
                ],
            ),
        ),
    );
    assert!(
        matches!(infer_type(&ctx, &collatz), Err(KernelError::TerminationViolation { .. })),
        "recursion on a COMPUTED argument is refused too — the guard is structural, not semantic: {:?}",
        infer_type(&ctx, &collatz)
    );

    // The fallback the case forbids, and why. Lean will not accept `partial def bad : Nat → False`
    // at all (no `Inhabited False`); LOGOS's `add_declaration` accepts any type, so the "just make
    // it an axiom" repair does not degrade — it detonates.
    let mut axiomatised = arith_ctx();
    axiomatised.add_declaration("bad", arrow(nat(), g("False")));
    let false_proof = infer_type(&axiomatised, &app(g("bad"), g("Zero")));
    assert!(
        matches!(&false_proof, Ok(t) if defeq_for_test(&axiomatised, t, &g("False"))),
        "the axiom fallback for a partial def is not lossy, it is INCONSISTENT — which is why refusal is the only answer: {false_proof:?}"
    );

    // For contrast: the SAME shape written structurally — `spinStruct (succ k) = spinStruct k` —
    // is accepted, so the refusal is about the descent, not about recursion or the mathematics.
    let spin_struct = fix(
        "spinStruct",
        lam(
            "n",
            nat(),
            match_(
                var("n"),
                lam("_", nat(), nat()),
                vec![g("Zero"), lam("k", nat(), app(var("spinStruct"), var("k")))],
            ),
        ),
    );
    assert!(
        infer_type(&ctx, &spin_struct).is_ok(),
        "the structurally descending version of the same function must be accepted: {:?}",
        infer_type(&ctx, &spin_struct)
    );
}

// ==== T02-008 · `unsafe def` — outside the logic; nothing to translate it into ====

/// OUT of scope, and the test says why in the only terms that matter: Lean refuses `unsafe` code
/// admission to its OWN logic, so there is no property for LOGOS to preserve, and LOGOS has no
/// untrusted compartment to hold it. All four registration doors feed `get_global`, which the type
/// checker consults with no notion of trust level — so the moment an unsafe constant is emitted it
/// is a trusted axiom, and for the archetype `unsafeCast : α → β` that axiom proves `False`
/// immediately. The safe half of an `@[implemented_by]` pair is the only translatable content.
#[test]
fn an_unsafe_declaration_has_no_untrusted_compartment_to_live_in() {
    let mut ctx = arith_ctx();

    // Every door LOGOS has, and the fact that all of them are TRUSTED — visible to `get_global`
    // and therefore to the type checker.
    ctx.add_inductive("Widget", ty0());
    ctx.add_constructor("mkWidget", "Widget", g("Widget"));
    ctx.add_definition("widgetCount".to_string(), nat(), num(1));
    ctx.add_declaration("widgetAxiom", nat());
    for door in ["Widget", "mkWidget", "widgetCount", "widgetAxiom"] {
        assert!(
            ctx.get_global(door).is_some(),
            "every registration door feeds the ONE trusted lookup — there is no unsafe compartment: {door} unresolved"
        );
    }

    // `unsafe def coerceAnything {α β} (a : α) : β := unsafeCast a` — the archetype. The only
    // shape it could take is an axiom, and that axiom inhabits `False`.
    let cast_ty = pi("A", ty0(), pi("B", ty0(), arrow(var("A"), var("B"))));
    ctx.add_declaration("unsafeCast", cast_ty);
    let boom = apps(g("unsafeCast"), &[nat(), g("False"), g("Zero")]);
    let boom_ty = infer_type(&ctx, &boom);
    assert!(
        matches!(&boom_ty, Ok(t) if defeq_for_test(&ctx, t, &g("False"))),
        "an unsafe cast admitted as an axiom proves False outright — so NO unsafe name may ever be emitted: {boom_ty:?}"
    );

    // `@[implemented_by fastIdImpl] def safeId (n) : Nat := n` plus
    // `theorem safeId_eq : safeId n = n := rfl`. Only the SAFE body has logical content, and it
    // must survive translation with the unsafe implementation emitted nowhere.
    let mut safe = arith_ctx();
    safe.add_definition(
        "safeId".to_string(),
        arrow(nat(), nat()),
        lam("n", nat(), var("n")),
    );
    let safe_thm = infer_type(&safe, &apps(g("refl"), &[nat(), num(3)]));
    assert!(
        matches!(&safe_thm, Ok(t) if defeq_for_test(&safe, t, &eq(nat(), app(g("safeId"), num(3)), num(3)))),
        "the safe half of an @[implemented_by] pair must translate normally: {safe_thm:?}"
    );
    assert!(
        safe.get_global("fastIdImpl").is_none() && safe.get_global("coerceAnything").is_none(),
        "the unsafe implementation must leave NO trace — not a definition, not an axiom: fastIdImpl={:?}, coerceAnything={:?}",
        safe.get_global("fastIdImpl").is_some(),
        safe.get_global("coerceAnything").is_some()
    );
}

// ==== T02-009 · `noncomputable` — an extraction marker, DERIVED rather than declared ====

/// We DIVERGE and are better off. Lean's `noncomputable` is a flag with no logical content whose
/// only job is to gate code generation, and Lean makes the author declare what Lean already
/// inferred. LOGOS stores no flag — and does not need one: computability is a property of the
/// body, and the extractor (`logicaffeine_compile::extraction::is_extractable`) reads it off the
/// term, so it can never be stale or wrong the way a declared marker can. The kernel-visible form
/// of the same fact is here: a definition through an axiom is registered exactly like any other,
/// keeps its full logical content, and is STUCK under normalization, while its pure sibling
/// computes. That distinction is recoverable from the context alone, with nothing stored.
#[test]
fn a_noncomputable_definition_keeps_its_logic_and_its_computability_is_read_off_the_body() {
    let mut ctx = arith_ctx();

    // `noncomputable def someNat : Nat := Classical.choice ⟨0⟩` — the classical content becomes an
    // axiom reference; the definition itself is entirely ordinary.
    ctx.add_declaration("choiceNat", arrow(g("True"), nat()));
    ctx.add_definition("someNat".to_string(), nat(), app(g("choiceNat"), g("I")));
    // `noncomputable def plainlyFine : Nat := 3 + 4` — marked in Lean only because it sits in a
    // `noncomputable section`; nothing about it is actually noncomputable.
    ctx.add_definition(
        "plainlyFine".to_string(),
        nat(),
        apps(g("plus"), &[num(3), num(4)]),
    );
    // The contrast case: the identical body without the marker.
    ctx.add_definition(
        "plainlyFineComputable".to_string(),
        nat(),
        apps(g("plus"), &[num(3), num(4)]),
    );

    // The logical content survives in full — this is what a translator that REJECTS or
    // axiomatises noncomputable declarations throws away.
    assert!(
        ctx.is_definition("someNat") && ctx.get_definition_body("someNat").is_some(),
        "a noncomputable declaration is logically ordinary and must keep its body: is_definition={}",
        ctx.is_definition("someNat")
    );
    // `example : someNat = someNat := rfl`
    let refl_ok = infer_type(&ctx, &apps(g("refl"), &[nat(), g("someNat")]));
    assert!(
        matches!(&refl_ok, Ok(t) if defeq_for_test(&ctx, t, &eq(nat(), g("someNat"), g("someNat")))),
        "a noncomputable definition is usable in proofs: {refl_ok:?}"
    );
    // `example : plainlyFine = 7 := by decide`
    assert!(
        defeq_for_test(&ctx, &g("plainlyFine"), &num(7)),
        "a `noncomputable` body with no classical content still computes: plainlyFine ⇝ {}",
        normalize(&ctx, &g("plainlyFine"))
    );

    // Computability is DERIVED, not stored: the axiom-backed body is stuck at the axiom head
    // while the pure one reduces to a numeral. Nothing in the context records a marker.
    assert_eq!(
        normalize(&ctx, &g("someNat")),
        app(g("choiceNat"), g("I")),
        "an axiom-backed body must be stuck at the axiom — that IS the computability signal"
    );
    assert!(
        defeq_for_test(&ctx, &g("plainlyFine"), &g("plainlyFineComputable")),
        "the marked and unmarked bodies are the same definition; a stored flag would be the only difference, and we store none"
    );
}

// ==== T02-010 · `instance` — a definition plus a flat, first-match-wins registration ====

/// A class is a structure, an instance is `add_definition` + `add_instance`, and resolution
/// unifies the required class type against the instance database. The order is LEAN'S:
/// higher priority first, then the LATER declaration, so emitting in source order reproduces
/// Lean without anyone reordering by hand. The database is push-only, so this test measures the
/// count it causes RELATIVE to whatever the context already carried — an absolute count would
/// forbid the prelude and declaration-time synthesis from ever registering an instance, which is
/// a constraint on the language dressed up as an assertion about a `Vec`.
#[test]
fn an_instance_resolves_from_a_flat_first_match_wins_database() {
    let mut ctx = arith_ctx();

    // `class Pointed (α : Type) where point : α`
    ctx.add_structure("Pointed", &[("A", ty0())], &[("point", var("A"))]);
    // `instance instPointedNat : Pointed Nat where point := 0`
    let inst_nat = apps(g("Pointed_mk"), &[nat(), g("Zero")]);
    ctx.add_definition(
        "instPointedNat".to_string(),
        app(g("Pointed"), nat()),
        inst_nat.clone(),
    );
    ctx.add_instance(app(g("Pointed"), nat()), g("instPointedNat"));
    // `instance instPointedBool : Pointed Bool where point := true`
    let inst_bool = apps(g("Pointed_mk"), &[bool_t(), g("true")]);
    ctx.add_definition(
        "instPointedBool".to_string(),
        app(g("Pointed"), bool_t()),
        inst_bool,
    );
    ctx.add_instance(app(g("Pointed"), bool_t()), g("instPointedBool"));

    // `def getPoint (α) [Pointed α] : α := Pointed.point`
    ctx.add_definition(
        "getPoint".to_string(),
        pi("A", ty0(), arrow(app(g("Pointed"), var("A")), var("A"))),
        lam(
            "A",
            ty0(),
            lam(
                "i",
                app(g("Pointed"), var("A")),
                apps(g("Pointed_point"), &[var("A"), var("i")]),
            ),
        ),
    );
    ctx.set_binder_kinds("getPoint", vec![ParamKind::Explicit, ParamKind::Instance]);

    // `example : getPoint Nat = 0 := rfl` and `example : getPoint Bool = true := rfl` — the
    // instance-implicit argument must be SYNTHESISED, not written.
    for (carrier, expected) in [(nat(), g("Zero")), (bool_t(), g("true"))] {
        let elaborated = surface_elaborate(&ctx, &app(g("getPoint"), carrier.clone()));
        assert!(
            matches!(&elaborated, Ok(t) if normalize(&ctx, t) == expected),
            "an instance-implicit argument must resolve from the database: getPoint {carrier} ⇝ {elaborated:?}"
        );
    }

    // Registering the DEFINITION without `add_instance` leaves an ordinary global nothing will
    // ever find — there is no attribute system to scan, so the registration is the only channel.
    let mut unregistered = ctx.clone();
    unregistered.add_structure("Tag", &[("A", ty0())], &[("name", nat())]);
    unregistered.add_definition(
        "tagNat".to_string(),
        app(g("Tag"), nat()),
        apps(g("Tag_mk"), &[nat(), num(1)]),
    );
    let mut mctx = MetaCtx::new();
    assert!(
        resolve_instance(&unregistered, &mut mctx, &app(g("Tag"), nat())).is_none(),
        "a definition that was never `add_instance`d must not resolve — forgetting the call is silent"
    );

    // LATER DECLARATION WINS, exactly as Lean tie-breaks two instances of equal priority.
    let mut overlapping = unregistered.clone();
    let before = overlapping.instances().len();
    overlapping.add_instance(app(g("Tag"), nat()), apps(g("Tag_mk"), &[nat(), num(1)]));
    overlapping.add_instance(app(g("Tag"), nat()), apps(g("Tag_mk"), &[nat(), num(2)]));
    let mut m2 = MetaCtx::new();
    let picked = resolve_instance(&overlapping, &mut m2, &app(g("Tag"), nat()));
    assert!(
        matches!(&picked, Some(t) if normalize(&overlapping, &app(app(g("Tag_name"), nat()), t.clone())) == num(2)),
        "two instances at one type, no priorities: the LATER declaration wins, as Lean does. \
         Earliest-first inverts every tie-break a Lean source relies on, silently — both shapes \
         type-check and compute different numbers: {picked:?}"
    );
    assert_eq!(
        overlapping.instances().len(),
        before + 2,
        "the instance database is push-only: both registrations stay in it, and the count is \
         measured RELATIVE to the baseline so a prelude instance cannot break this assertion"
    );
}

// ==== T02-011 · namespaces and hierarchical `Name` ====

/// Lean's `Name` is a tree and resolution is namespace-relative. LOGOS's keys are flat `String`s.
/// The good news, which surprised the audit: the key space is wide enough to STORE a Lean name
/// injectively — `Outer.value`, `Outer.Inner.value` and `Outer_value` are three distinct keys and
/// stay distinct, so the collision of T02-020 comes from the `.` → `_` mangling, not from the map.
/// What does not exist is any RESOLUTION. A dotted key cannot even be *referred to*: the term
/// parser reads `.` as projection, so a stored `Outer.value` is unreachable from the surface. And
/// nothing turns the short name `value` written inside `namespace Outer.Inner` into
/// `Outer.Inner.value` — there is no namespace stack, no `open`, no `export`, no `_root_`.
#[test]
fn a_short_name_does_not_resolve_against_its_enclosing_namespace() {
    let mut repl = Repl::new();

    repl.execute("Definition Outer.value : Nat := (Succ Zero).")
        .expect("dotted key registers");
    repl.execute("Definition Outer.Inner.value : Nat := (Succ (Succ Zero)).")
        .expect("nested dotted key registers");
    repl.execute("Definition Outer_value : Nat := (Succ (Succ (Succ Zero))).")
        .expect("the flattened twin registers");

    // The half that works, and is worth pinning: a flat `String` key is injective over
    // hierarchical names as long as the separator is not rewritten.
    let stored: Vec<Option<Term>> = ["Outer.value", "Outer.Inner.value", "Outer_value"]
        .iter()
        .map(|k| repl.context().get_definition_body(k).cloned())
        .collect();
    assert_eq!(
        stored,
        vec![Some(num(1)), Some(num(2)), Some(num(3))],
        "three distinct Lean names must be three distinct keys — the map itself is injective"
    );

    // But a dotted key is unreachable by reference: `Outer.value` in term position is parsed as
    // the projection `.value` of a receiver `Outer`, so the name that WAS stored cannot be used.
    let by_name = repl.execute("Eval Outer.value.");
    assert!(
        by_name.is_ok(),
        "a hierarchical name must be referable, not merely storable: {by_name:?}"
    );

    // `namespace Outer.Inner … def usesInner : Nat := value` — resolution is namespace-relative in
    // Lean. There is no relative resolution in LOGOS at all.
    let relative = repl.execute("Definition Outer.Inner.usesInner : Nat := value.");
    assert!(
        relative.is_ok(),
        "a declaration inside `namespace Outer.Inner` must resolve the short name `value` to Outer.Inner.value: {relative:?}"
    );
}

// ==== T02-012 · `private` / `protected` / `scoped` / `local` — visibility and scope lifetime ====

/// A `local instance` lives exactly as long as its section. The case makes that observable with
/// two lines Lean answers differently — `Weight.w 5` is `5` inside the section and `1` outside —
/// and a context that could only ever ADD an instance could produce at most one of them.
/// `push_scope` opens the section, `add_local_instance` registers into it, and `pop_scope` takes
/// the instance out again while keeping every declaration made inside. (`Private` and
/// `Protected` are vernacular qualifiers, exercised through the command surface.)
#[test]
fn a_local_instance_can_never_leave_the_instance_database() {
    let mut ctx = arith_ctx();

    // `class Weight (α : Type) where w : α → Nat`
    ctx.add_structure("Weight", &[("A", ty0())], &[("w", arrow(var("A"), nat()))]);
    let weight_nat = app(g("Weight"), nat());
    let weigh_five = |ctx: &Context| {
        let mut mctx = MetaCtx::new();
        resolve_instance(ctx, &mut mctx, &weight_nat)
            .map(|i| normalize(ctx, &apps(g("Weight_w"), &[nat(), i, num(5)])))
    };
    // `instance defaultWeight : Weight Nat where w := fun _ => 1` — declared first.
    let default_w = apps(g("Weight_mk"), &[nat(), lam("_", nat(), num(1))]);
    ctx.add_instance(weight_nat.clone(), default_w);
    let outside_count = ctx.instances().len();
    assert_eq!(
        weigh_five(&ctx),
        Some(num(1)),
        "outside any section `Weight.w 5 = 1`"
    );

    // `section Override / local instance overrideWeight : Weight Nat where w := fun n => n`
    ctx.push_scope();
    let override_w = apps(g("Weight_mk"), &[nat(), lam("n", nat(), var("n"))]);
    ctx.add_local_instance(weight_nat.clone(), override_w.clone());
    // A permanent declaration made INSIDE the section — it must survive the section's close, which
    // is why wholesale snapshot/restore of the context is not a scoping mechanism.
    ctx.add_definition("insideSection".to_string(), nat(), num(5));

    // Inside the section Lean resolves to `overrideWeight`, so `Weight.w 5 = 5`.
    assert_eq!(
        weigh_five(&ctx),
        Some(num(5)),
        "inside its section a `local instance` must WIN — the later instance at equal priority"
    );

    // `end Override` — and Lean removes it again, so `Weight.w 5 = 1`.
    ctx.pop_scope();
    assert_eq!(
        ctx.instances().len(),
        outside_count,
        "a `local`/`scoped` instance must leave the database when its scope closes"
    );
    assert_eq!(
        weigh_five(&ctx),
        Some(num(1)),
        "…so outside it `Weight.w 5 = 1` again"
    );
    assert!(
        ctx.is_definition("insideSection"),
        "closing a section must keep the declarations made inside it — so restoring a whole snapshot is not the answer"
    );

    // Only `local` reverts: a plain `instance` declared inside a section is permanent.
    ctx.push_scope();
    ctx.add_instance(weight_nat.clone(), override_w);
    ctx.pop_scope();
    assert_eq!(
        ctx.instances().len(),
        outside_count + 1,
        "a non-local instance outlives the section it was declared in"
    );
}

#[test]
fn a_simp_lemma_has_no_registry_to_be_registered_in() {
    let mut ctx = arith_ctx();

    // `def size (n : Nat) : Nat := n + 0` — stuck on a variable, because `plus` recurses on its
    // FIRST argument. Exactly the shape a rewrite lemma exists to close.
    ctx.add_definition(
        "size".to_string(),
        arrow(nat(), nat()),
        lam("n", nat(), apps(g("plus"), &[var("n"), g("Zero")])),
    );
    // `@[simp] theorem size_eq (n : Nat) : size n = n := Nat.add_zero n`
    let size_eq_stmt = pi("n", nat(), eq(nat(), app(g("size"), var("n")), var("n")));
    ctx.add_declaration("size_eq", size_eq_stmt);

    let mut scoped = ctx.extend("n", nat());
    // `example (n : Nat) : size (size n) = n := by simp` — closes in Lean ONLY because of the
    // ambient simp set.
    let nested = app(g("size"), app(g("size"), var("n")));

    // The only extensible lemma database the kernel has is `hints`, a `Vec<String>` whose sole
    // consumer is the `auto` tactic's fallback. It records the NAME and nothing else — no
    // attribute identity, no rewrite direction, and nothing in the simplifier reads it.
    // (`simp::check_goal(goal: &Term) -> bool` does not even take a `Context`, so there is no
    // parameter through which a registered lemma could reach it.)
    scoped.add_hint("size_eq");
    assert!(
        scoped.is_hint("size_eq"),
        "`hints` is the whole extensible-lemma surface, and it did not even record the name"
    );
    assert_eq!(
        normalize(&scoped, &nested),
        var("n"),
        "a registered @[simp] lemma must let the simplifier close `size (size n) = n`; the work \
         this names is a simp-lemma registry on `Context` that the simplifier consults, which is \
         why `size (size n)` is stuck at {} whether or not `size_eq` is registered",
        normalize(&scoped, &nested)
    );

    // `attribute [instance] markedNatImpl` — the attribute FORM of `instance`. This one does
    // re-encode, and the test pins that so the gap is scoped to the simp set rather than to
    // attributes wholesale.
    let mut marked = arith_ctx();
    marked.add_structure("Marked", &[("A", ty0())], &[("tag", nat())]);
    marked.add_definition(
        "markedNatImpl".to_string(),
        app(g("Marked"), nat()),
        apps(g("Marked_mk"), &[nat(), num(1)]),
    );
    marked.add_instance(app(g("Marked"), nat()), g("markedNatImpl"));
    let mut mctx = MetaCtx::new();
    assert!(
        resolve_instance(&marked, &mut mctx, &app(g("Marked"), nat())).is_some(),
        "`@[instance]` applied after the fact must have the same effect as the `instance` command"
    );
}

// ==== T02-014 · `mutual def` — one `MutualFix`, projected once per member ====

/// A mutual block is ONE term with an index, not N terms. Both the block-wide Giménez guard and
/// the sibling scoping depend on that: this test shows the block working for two and three
/// members, and then shows the failure the case exists to catch — splitting the pair into
/// separate fixpoints with the sibling reached as a global lets a non-descending recursion
/// through the guard entirely.
#[test]
fn a_mutual_block_is_one_term_and_splitting_it_escapes_the_descent_check() {
    let mut ctx = arith_ctx();
    let bmotive = lam("_", nat(), bool_t());

    // `mutual def isEven | 0 => true | succ n => isOdd n  /  def isOdd | 0 => false | succ n => isEven n end`
    let is_even = lam(
        "n",
        nat(),
        match_(
            var("n"),
            bmotive.clone(),
            vec![g("true"), lam("m", nat(), app(var("isOdd"), var("m")))],
        ),
    );
    let is_odd = lam(
        "n",
        nat(),
        match_(
            var("n"),
            bmotive.clone(),
            vec![g("false"), lam("m", nat(), app(var("isEven"), var("m")))],
        ),
    );
    let block = [("isEven", is_even), ("isOdd", is_odd)];
    assert!(
        infer_type(&ctx, &mutfix(&block, 0)).is_ok(),
        "a structurally mutual block must pass the MUTUAL guard: {:?}",
        infer_type(&ctx, &mutfix(&block, 0))
    );
    ctx.add_definition(
        "isEven".to_string(),
        arrow(nat(), bool_t()),
        mutfix(&block, 0),
    );
    ctx.add_definition(
        "isOdd".to_string(),
        arrow(nat(), bool_t()),
        mutfix(&block, 1),
    );

    for (name, arg, expected) in [
        ("isEven", 4u32, "true"),
        ("isOdd", 4, "false"),
        ("isEven", 7, "false"),
        ("isOdd", 7, "true"),
    ] {
        assert_eq!(
            normalize(&ctx, &app(g(name), num(arg))),
            g(expected),
            "{name} {arg} must compute to {expected} — a wrong `index` inverts the pair silently"
        );
    }
    // `example : isEven 3 ≠ isOdd 3 := by decide` — the observable that an off-by-one `index`
    // flips, since both members typecheck at either index.
    assert_ne!(
        normalize(&ctx, &app(g("isEven"), num(3))),
        normalize(&ctx, &app(g("isOdd"), num(3))),
        "the two members must disagree on an odd input"
    );

    // A three-member block, so `index` has more than two values to be wrong about.
    let step = |next: &str| {
        lam(
            "n",
            nat(),
            match_(
                var("n"),
                bmotive.clone(),
                vec![g("false"), lam("m", nat(), app(var(next), var("m")))],
            ),
        )
    };
    let three = [
        (
            "modThree0",
            lam(
                "n",
                nat(),
                match_(
                    var("n"),
                    bmotive.clone(),
                    vec![g("true"), lam("m", nat(), app(var("modThree2"), var("m")))],
                ),
            ),
        ),
        ("modThree1", step("modThree0")),
        ("modThree2", step("modThree1")),
    ];
    let mut c3 = arith_ctx();
    for (i, (name, _)) in three.iter().enumerate() {
        c3.add_definition(name.to_string(), arrow(nat(), bool_t()), mutfix(&three, i));
    }
    for (name, arg) in [("modThree0", 6u32), ("modThree1", 4), ("modThree2", 5)] {
        assert_eq!(
            normalize(&c3, &app(g(name), num(arg))),
            g("true"),
            "{name} {arg} must be true — the block must be shared, with each member at its own index"
        );
    }
    assert!(
        infer_type(&c3, &mutfix(&three, 9)).is_err(),
        "an out-of-range index must be refused: {:?}",
        infer_type(&c3, &mutfix(&three, 9))
    );

    // Why the block MUST be emitted as one term. Written as one `MutualFix`, a member that calls
    // its sibling with the UN-decremented argument is refused by the mutual guard…
    let bad_even = lam(
        "n",
        nat(),
        match_(
            var("n"),
            bmotive.clone(),
            vec![g("true"), lam("_", nat(), app(var("isOdd"), var("n")))],
        ),
    );
    let bad_odd = lam(
        "n",
        nat(),
        match_(
            var("n"),
            bmotive.clone(),
            vec![g("false"), lam("m", nat(), app(var("isEven"), var("m")))],
        ),
    );
    let fused = [("isEven", bad_even.clone()), ("isOdd", bad_odd)];
    assert!(
        matches!(
            infer_type(&arith_ctx(), &mutfix(&fused, 0)),
            Err(KernelError::TerminationViolation { .. })
        ),
        "a non-descending sibling call must be refused when the block is one term: {:?}",
        infer_type(&arith_ctx(), &mutfix(&fused, 0))
    );

    // …and is ACCEPTED once the same bodies are split into separate fixpoints reaching the
    // sibling as a global, because the single-block guard sees an unrelated call and imposes no
    // descent requirement. Pinned as the HAZARD it is — the reason a translator must emit one
    // `MutualFix` — not as behaviour we endorse.
    let mut split = arith_ctx();
    split.add_declaration("isOdd", arrow(nat(), bool_t()));
    let split_even = fix(
        "isEven",
        lam(
            "n",
            nat(),
            match_(
                var("n"),
                bmotive,
                vec![g("true"), lam("_", nat(), app(g("isOdd"), var("n")))],
            ),
        ),
    );
    assert!(
        infer_type(&split, &split_even).is_ok(),
        "splitting the block sheds the descent requirement the fused form enforces — that asymmetry is exactly why the block must be ONE MutualFix: {:?}",
        infer_type(&split, &split_even)
    );
}

// ==== T02-015 · `where` clauses — auxiliaries lifted, with the parent's binders captured ====

/// Lean lifts a `where` body to a top-level auxiliary with the parent's binders as leading
/// parameters. LOGOS hosts that exactly: the auxiliary is an ordinary definition and the parent
/// mentions it as a `Global`. The two things that have to be right are the capture (a lifted body
/// that still mentions the parent's binder leaves a dangling `Var` that `add_definition` accepts
/// without complaint) and the mutual case, where two sibling auxiliaries lift to one `MutualFix`
/// rather than two fixpoints.
#[test]
fn a_where_auxiliary_lifts_to_its_own_definition_with_the_parents_binder_captured() {
    let mut ctx = arith_ctx();

    // `def sumOfSquares (a b) : Nat := sq a + sq b where sq n := n * n` — a NON-recursive
    // auxiliary used twice. Lifting keeps the body once; inlining would duplicate it.
    ctx.add_definition(
        "sumOfSquares_sq".to_string(),
        arrow(nat(), nat()),
        lam("n", nat(), apps(g("plus"), &[var("n"), var("n")])),
    );
    ctx.add_definition(
        "sumOfSquares".to_string(),
        arrow(nat(), arrow(nat(), nat())),
        lam(
            "a",
            nat(),
            lam(
                "b",
                nat(),
                apps(
                    g("plus"),
                    &[
                        app(g("sumOfSquares_sq"), var("a")),
                        app(g("sumOfSquares_sq"), var("b")),
                    ],
                ),
            ),
        ),
    );
    assert_eq!(
        normalize(&ctx, &apps(g("sumOfSquares"), &[num(3), num(4)])),
        num(14),
        "a non-recursive `where` auxiliary must be reachable as its own global"
    );

    // `def countUp (step n) : Nat := go n 0 where go | 0, acc => acc | succ k, acc => go k (acc + step)`
    // — `go` CAPTURES `step`, so the lifted auxiliary takes it as a leading parameter.
    let go = fix(
        "go",
        lam(
            "step",
            nat(),
            lam(
                "n",
                nat(),
                lam(
                    "acc",
                    nat(),
                    match_(
                        var("n"),
                        lam("_", nat(), nat()),
                        vec![
                            var("acc"),
                            lam(
                                "k",
                                nat(),
                                apps(
                                    var("go"),
                                    &[
                                        var("step"),
                                        var("k"),
                                        apps(g("plus"), &[var("acc"), var("step")]),
                                    ],
                                ),
                            ),
                        ],
                    ),
                ),
            ),
        ),
    );
    let go_ty = infer_type(&ctx, &go);
    assert!(
        matches!(&go_ty, Ok(t) if defeq_for_test(&ctx, t, &arrow(nat(), arrow(nat(), arrow(nat(), nat()))))),
        "the lifted auxiliary's type is the captured telescope followed by its own: {go_ty:?}"
    );
    ctx.add_definition(
        "countUp_go".to_string(),
        arrow(nat(), arrow(nat(), arrow(nat(), nat()))),
        go,
    );
    ctx.add_definition(
        "countUp".to_string(),
        arrow(nat(), arrow(nat(), nat())),
        lam(
            "step",
            nat(),
            lam(
                "n",
                nat(),
                apps(g("countUp_go"), &[var("step"), var("n"), g("Zero")]),
            ),
        ),
    );
    assert_eq!(
        normalize(&ctx, &apps(g("countUp"), &[num(2), num(3)])),
        num(6),
        "the captured binder must be threaded as a leading parameter, not left dangling"
    );
    assert_eq!(
        normalize(&ctx, &apps(g("countUp"), &[num(5), num(0)])),
        num(0),
        "countUp 5 0 = 0"
    );

    // The dangling-`Var` hazard: a lifted body that still mentions the parent's binder is
    // accepted by `add_definition` without a murmur and fails only when something normalizes
    // through it.
    let dangling = lam("n", nat(), apps(g("plus"), &[var("n"), var("step")]));
    assert!(
        infer_type(&ctx, &dangling).is_err(),
        "a lifted body that did not capture the parent's binder must not typecheck: {:?}",
        infer_type(&ctx, &dangling).map(|t| t.to_string())
    );

    // `def parity (n) := ev n where ev | 0 => true | succ k => od k; od | 0 => false | succ k => ev k`
    // — two sibling auxiliaries lift to ONE `MutualFix`, not two `Fix`es.
    let bmotive = lam("_", nat(), bool_t());
    let ev = lam(
        "n",
        nat(),
        match_(
            var("n"),
            bmotive.clone(),
            vec![g("true"), lam("k", nat(), app(var("od"), var("k")))],
        ),
    );
    let od = lam(
        "n",
        nat(),
        match_(
            var("n"),
            bmotive,
            vec![g("false"), lam("k", nat(), app(var("ev"), var("k")))],
        ),
    );
    let pair = [("ev", ev), ("od", od)];
    ctx.add_definition(
        "parity_ev".to_string(),
        arrow(nat(), bool_t()),
        mutfix(&pair, 0),
    );
    ctx.add_definition(
        "parity".to_string(),
        arrow(nat(), bool_t()),
        lam("n", nat(), app(g("parity_ev"), var("n"))),
    );
    assert_eq!(
        (
            normalize(&ctx, &app(g("parity"), num(4))),
            normalize(&ctx, &app(g("parity"), num(5)))
        ),
        (g("true"), g("false")),
        "mutually referring `where` auxiliaries lift to one MutualFix"
    );
}

// ==== T02-016 · `let rec` — a local fixpoint; `Let` alone cannot carry the recursion ====

/// `Term::Let` is zeta-transparent and NOT recursive — the bound name is not in scope in its own
/// value — so a `let rec` is a `Term::Fix` that a `Let` may bind. Both halves are asserted here:
/// the honest encoding computes, and the naive one (`Let` with a self-referential value) leaves a
/// free variable. Nested `let rec`s stay two separate fixpoints; fusing them into a `MutualFix`
/// would change what the guard demands of each.
#[test]
fn a_let_rec_is_a_fix_bound_by_a_let_and_a_plain_let_cannot_carry_it() {
    let ctx = arith_ctx();

    // `def sumTo (n) := go n 0 where let rec go | 0, acc => acc | succ k, acc => go k (acc + succ k)`
    let go = fix(
        "go",
        lam(
            "n",
            nat(),
            lam(
                "acc",
                nat(),
                match_(
                    var("n"),
                    lam("_", nat(), nat()),
                    vec![
                        var("acc"),
                        lam(
                            "k",
                            nat(),
                            apps(
                                var("go"),
                                &[
                                    var("k"),
                                    apps(g("plus"), &[var("acc"), app(g("Succ"), var("k"))]),
                                ],
                            ),
                        ),
                    ],
                ),
            ),
        ),
    );
    let sum_to = lam(
        "n",
        nat(),
        Term::Let {
            name: "go".to_string(),
            ty: Box::new(arrow(nat(), arrow(nat(), nat()))),
            value: Box::new(go.clone()),
            body: Box::new(apps(var("go"), &[var("n"), g("Zero")])),
        },
    );
    let ty = infer_type(&ctx, &sum_to);
    assert!(
        matches!(&ty, Ok(t) if defeq_for_test(&ctx, t, &arrow(nat(), nat()))),
        "a `let rec` is a Fix bound by a Let, and must typecheck as one: {ty:?}"
    );
    assert_eq!(
        normalize(&ctx, &app(sum_to.clone(), num(4))),
        num(10),
        "sumTo 4 = 4+3+2+1 = 10"
    );
    assert_eq!(normalize(&ctx, &app(sum_to, num(0))), num(0), "sumTo 0 = 0");

    // The mistranslation: `Let { name: "go", value: <body mentioning go> }`. `Let` gives no
    // recursive scope, so the occurrence inside the value is a FREE variable.
    let naive = Term::Let {
        name: "go".to_string(),
        ty: Box::new(arrow(nat(), nat())),
        value: Box::new(lam("n", nat(), app(var("go"), var("n")))),
        body: Box::new(app(var("go"), g("Zero"))),
    };
    assert!(
        matches!(
            infer_type(&ctx, &naive),
            Err(KernelError::UnboundVariable(_))
        ),
        "a plain `Let` must NOT bind its own value recursively: {:?}",
        infer_type(&ctx, &naive).map(|t| t.to_string())
    );

    // `def nestedRec (n) := outer n` with an inner `let rec` inside the outer one's body — two
    // independent fixpoints, one inside the other, not one fused block.
    let inner = fix(
        "inner",
        lam(
            "j",
            nat(),
            match_(
                var("j"),
                lam("_", nat(), nat()),
                vec![
                    num(1),
                    lam("i", nat(), app(g("Succ"), app(var("inner"), var("i")))),
                ],
            ),
        ),
    );
    let outer = fix(
        "outer",
        lam(
            "n",
            nat(),
            match_(
                var("n"),
                lam("_", nat(), nat()),
                vec![
                    g("Zero"),
                    lam(
                        "k",
                        nat(),
                        Term::Let {
                            name: "inner".to_string(),
                            ty: Box::new(arrow(nat(), nat())),
                            value: Box::new(inner),
                            body: Box::new(apps(
                                g("plus"),
                                &[app(var("inner"), var("k")), app(var("outer"), var("k"))],
                            )),
                        },
                    ),
                ],
            ),
        ),
    );
    assert!(
        infer_type(&ctx, &outer).is_ok(),
        "nested `let rec`s stay two separate fixpoints, each guarded on its own: {:?}",
        infer_type(&ctx, &outer)
    );
    assert_eq!(
        normalize(&ctx, &app(outer.clone(), num(0))),
        num(0),
        "nestedRec 0 = 0"
    );
    assert_eq!(
        normalize(&ctx, &app(outer, num(2))),
        num(3),
        "nestedRec 2 = 3"
    );

    // For contrast, a genuinely non-recursive `let` — zeta-transparent, as Lean's is.
    let plain = lam(
        "n",
        nat(),
        Term::Let {
            name: "twice".to_string(),
            ty: Box::new(nat()),
            value: Box::new(apps(g("plus"), &[var("n"), var("n")])),
            body: Box::new(apps(g("plus"), &[var("twice"), var("twice")])),
        },
    );
    assert_eq!(
        normalize(&ctx, &app(plain, num(2))),
        num(8),
        "plainLet 2 = 8"
    );
}

// ==== T02-017 · `deriving` — translate the handler's OUTPUT, and we have no handler ====

/// The `deriving` clause itself is not translatable — what Lean's handlers put in the environment
/// is. LOGOS has no derive mechanism at all: the only decidable-equality artefacts in the whole
/// system are two hand-written prelude definitions, `decEqBool` and `decEqNat`. So a Lean file
/// carrying `deriving DecidableEq` on a user inductive has no counterpart, and the tempting repair
/// — reaching for `decEqNat` because the fields happen to be `Nat` — decides a different equality.
#[test]
fn a_derived_decidable_equality_has_no_handler_to_generate_it() {
    let mut ctx = arith_ctx();

    // `inductive Colour | red | green | blue deriving Repr, BEq, DecidableEq, Inhabited`
    ctx.add_inductive("Colour", ty0());
    for c in ["red", "green", "blue"] {
        ctx.add_constructor(c, "Colour", g("Colour"));
    }

    // `example : decide (Colour.green = Colour.green) = true := rfl` — needs the DERIVED
    // `DecidableEq Colour`, which nothing generated.
    let mut mctx = MetaCtx::new();
    let want = app(g("Decidable"), eq(g("Colour"), g("red"), g("red")));
    let found = resolve_instance(&ctx, &mut mctx, &want);
    assert!(
        found.is_some(),
        "`deriving DecidableEq` must put a decision procedure for `Colour` in reach; LOGOS has only the hand-written decEqBool/decEqNat: {found:?}"
    );

    // And the trap the guard names: `decEqNat` decides equality of `Nat`, so mapping a derived
    // `DecidableEq Pixel` onto it because the fields are `Nat` compares one field and ignores the
    // constructor tag. Pinned so a later "repair" cannot take that route quietly.
    let dec_eq_nat = ctx.get_global("decEqNat").cloned();
    assert!(
        matches!(&dec_eq_nat, Some(t) if !defeq_for_test(&ctx, t, &pi("a", g("Colour"), pi("b", g("Colour"), app(g("Decidable"), eq(g("Colour"), var("a"), var("b"))))))),
        "decEqNat decides Nat equality and nothing else — it must never stand in for a derived instance: {dec_eq_nat:?}"
    );
}

// ==== T02-018 · `section` + `variable` — the binders Lean inserts before we see anything ====

/// For a translator this command pair is a non-event: Lean has already closed every declaration
/// by the time we look. What has to be true on our side is that the telescope it produces is
/// representable, and the hard shape is the interleaved one — `{α : Type} (xs : List α)
/// [Inhabited α]` — where the legacy "N leading implicits" model cannot express the ordering and
/// the instance argument must be resolved only AFTER the explicit argument pins `α`. `binder_kinds`
/// carries it exactly, position for position.
#[test]
fn a_section_variable_telescope_survives_as_interleaved_binder_kinds() {
    let mut ctx = arith_ctx();
    ctx.add_structure("Inhab", &[("A", ty0())], &[("dflt", var("A"))]);
    ctx.add_instance(
        app(g("Inhab"), nat()),
        apps(g("Inhab_mk"), &[nat(), g("Zero")]),
    );
    ctx.add_instance(
        app(g("Inhab"), bool_t()),
        apps(g("Inhab_mk"), &[bool_t(), g("false")]),
    );

    // `variable {α : Type} (xs : List α) [Inhabited α]` then `def headOrDefault : α := …`
    // elaborates to `{α} (xs) [Inhabited α] : α`. The closed type is what we store.
    ctx.add_definition(
        "headOrDefault".to_string(),
        pi(
            "A",
            ty0(),
            pi("x", var("A"), arrow(app(g("Inhab"), var("A")), var("A"))),
        ),
        lam(
            "A",
            ty0(),
            lam("x", var("A"), lam("i", app(g("Inhab"), var("A")), var("x"))),
        ),
    );
    ctx.set_binder_kinds(
        "headOrDefault",
        vec![
            ParamKind::Implicit,
            ParamKind::Explicit,
            ParamKind::Instance,
        ],
    );
    assert_eq!(
        ctx.binder_kinds("headOrDefault"),
        Some([ParamKind::Implicit, ParamKind::Explicit, ParamKind::Instance].as_slice()),
        "the inserted binders' kinds must be recorded position for position, not as a leading-implicit count"
    );

    // The user writes one argument; `α` is inferred from it and the instance is resolved after.
    for (arg, expected) in [(num(7), num(7)), (g("true"), g("true"))] {
        let elaborated = surface_elaborate(&ctx, &app(g("headOrDefault"), arg.clone()));
        assert!(
            matches!(&elaborated, Ok(t) if normalize(&ctx, t) == expected),
            "an interleaved {{α}} (x) [Inhab α] telescope must elaborate from one explicit argument: {arg} ⇝ {elaborated:?}"
        );
    }

    // `def noN : Nat := 0` does not mention the section variable, so Lean inserts NOTHING — a
    // translator that inserts every section variable changes the arity. The closed form we store
    // has to be the elaborated one.
    ctx.add_definition("noN".to_string(), nat(), g("Zero"));
    assert_eq!(
        ctx.get_definition_type("noN"),
        Some(&nat()),
        "a declaration that never used the section variable keeps its own type"
    );

    // `variable (a : Nat) (b : Nat)` with `def onlyB : Nat := b * 2` — only `b` is inserted, so
    // the arity is `Nat → Nat`, not `Nat → Nat → Nat`.
    ctx.add_definition(
        "onlyB".to_string(),
        arrow(nat(), nat()),
        lam("b", nat(), apps(g("plus"), &[var("b"), var("b")])),
    );
    assert_eq!(
        normalize(&ctx, &app(g("onlyB"), num(5))),
        num(10),
        "only the section variables a declaration actually used are inserted"
    );

    // `variable {β} (f : β → β) (z : β)` with `def applyTwice : β := f (f z)` — `β` appears only
    // in the OTHER binders' types and must still be inserted, first.
    ctx.add_definition(
        "applyTwice".to_string(),
        pi(
            "B",
            ty0(),
            pi("f", arrow(var("B"), var("B")), pi("z", var("B"), var("B"))),
        ),
        lam(
            "B",
            ty0(),
            lam(
                "f",
                arrow(var("B"), var("B")),
                lam("z", var("B"), app(var("f"), app(var("f"), var("z")))),
            ),
        ),
    );
    ctx.set_binder_kinds(
        "applyTwice",
        vec![
            ParamKind::Implicit,
            ParamKind::Explicit,
            ParamKind::Explicit,
        ],
    );
    let applied = surface_elaborate(
        &ctx,
        &apps(
            g("applyTwice"),
            &[lam("n", nat(), app(g("Succ"), var("n"))), g("Zero")],
        ),
    );
    assert!(
        matches!(&applied, Ok(t) if normalize(&ctx, t) == num(2)),
        "a section variable used only inside other binders' types must still be inserted, and first: {applied:?}"
    );
}

// ==== T02-019 · `open … in` — name resolution completes before translation begins ====

/// `Term::Global` carries one flat ABSOLUTE name, and the vernacular resolves a short name before
/// the kernel sees it: `Open Alpha in <command>` makes `token` denote `Alpha.token` for exactly
/// that command, and the identical text under `Open Beta in` denotes `Beta.token`. The hazard the
/// case names is the silent one — the prelude owns `Nat`, `Zero`, `Succ`, `Eq` and hundreds more,
/// so a short name written under an `open` could quietly resolve to the PRELUDE's constant. It
/// does not: as in Lean 4.14, a name both the opened namespace and the root answer to is refused
/// as ambiguous, and the author writes the one they mean.
#[test]
fn the_same_short_name_under_two_opens_cannot_denote_two_constants() {
    let mut repl = Repl::new();
    repl.execute("Definition Alpha.token : Nat := (Succ Zero).")
        .expect("Alpha.token");
    repl.execute("Definition Beta.token : Nat := (Succ (Succ Zero)).")
        .expect("Beta.token");

    // Without an `open`, the short name denotes nothing.
    assert!(
        matches!(
            repl.execute("Definition unopened : Nat := token."),
            Err(e) if format!("{e:?}").contains("UnboundVariable")
        ),
        "`token` resolves to nothing until a namespace is opened"
    );

    // `open Alpha in def fromAlpha : Nat := token`  — `token` is `Alpha.token`.
    let from_alpha = repl.execute("Open Alpha in Definition fromAlpha : Nat := token.");
    assert!(
        from_alpha.is_ok(),
        "`open Alpha in …` must make the short name `token` denote Alpha.token: {from_alpha:?}"
    );
    assert_eq!(
        repl.execute("Eval fromAlpha.").unwrap().trim(),
        "(Succ Zero)",
        "the same source text must denote Alpha.token here"
    );

    // `open Beta in def fromBeta : Nat := token` — the identical text, a different constant.
    repl.execute("Open Beta in Definition fromBeta : Nat := token.")
        .expect("Beta's token");
    assert_eq!(
        repl.execute("Eval fromBeta.").unwrap().trim(),
        "(Succ (Succ Zero))",
        "…and Beta.token there — the scoped `open` covers exactly one command"
    );

    // The silent version, which is the dangerous one. Inside `Alpha.Zero`'s own body the short
    // `Zero` would mean `Alpha.Zero` itself (Lean refuses that self-reference too), so the body
    // names the root constant absolutely.
    let mut shadow = Repl::new();
    shadow
        .execute("Definition Alpha.Zero : Nat := (Succ _root_.Zero).")
        .expect("Alpha.Zero");
    assert_eq!(
        shadow.execute("Eval Alpha.Zero.").unwrap().trim(),
        "(Succ Zero)"
    );
    // `open Alpha in def shadowed := Zero` is AMBIGUOUS in Lean 4.14 (`_root_.Zero` and
    // `Alpha.Zero`); binding it to the prelude's `Zero` without a word would change what the proof
    // is about.
    let shadowed = shadow.execute("Open Alpha in Definition shadowed : Nat := Zero.");
    assert!(
        matches!(&shadowed, Err(e) if {
            let why = format!("{e:?}");
            why.contains("AmbiguousName") && why.contains("Alpha.Zero")
        }),
        "a short name the opened namespace and the root both answer to must be refused as \
         ambiguous, naming both candidates — never bound silently to the prelude's: {shadowed:?}"
    );
    assert!(
        shadow.execute("Eval shadowed.").is_err(),
        "the refused definition registered nothing"
    );
    // Written in full, each reading is available.
    shadow
        .execute("Open Alpha in Definition viaAlpha : Nat := Alpha.Zero.")
        .expect("Alpha.Zero in full");
    assert_eq!(
        shadow.execute("Eval viaAlpha.").unwrap().trim(),
        "(Succ Zero)"
    );
    shadow
        .execute("Open Alpha in Definition viaRoot : Nat := _root_.Zero.")
        .expect("the root in full");
    assert_eq!(shadow.execute("Eval viaRoot.").unwrap().trim(), "Zero");
}
