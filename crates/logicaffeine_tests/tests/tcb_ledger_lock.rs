//! THE TRUSTED BASE LEDGER — every axiom we assume, named and classified.
//!
//! Lean 4's trusted base is three axioms: `propext`, `Classical.choice`, `Quot.sound`. Ours is
//! larger, and in a head-to-head on mathematics that is the first thing a judge checks — an axiom
//! is a thing we *assume* rather than *prove*, and a bigger set is a weaker claim.
//!
//! The corpus gives us the comparison for free. `scripts/lean/conformance-lean-oracle.sh` ran the real
//! Lean 4.14.0 compiler over all 248 cases and recorded the transitive axiom set of every
//! declaration. Across 5,827 declarations Lean reached for exactly: `propext` (392),
//! `Quot.sound` (155), `Classical.choice` (41), `sorryAx` (17, the deliberate `sorry` cases),
//! `Lean.ofReduceBool` (8), `Lean.trustCompiler` (3), `lcProof` (2). That is the bar.
//!
//! This lock pins our side. It reads the axioms the standard prelude actually registers and
//! requires each to be classified — here, and argued in `TCB.md`, which
//! [`every_ledger_entry_is_argued_in_the_tcb_document`] reads so the document can never drift
//! from the kernel:
//!
//!   Primitive     — Lean has an equivalent primitive. Defensible.
//!   Definable     — Lean PROVES or DEFINES this; we assume it. **We owe a derivation.**
//!   Reflection    — an axiomatised operational semantics for the deep `Syntax`/`Derivation`
//!                   embedding. Its own category because Lean has no counterpart at all.
//!   DomainSpecific— no Lean counterpart; justified in writing.
//!
//! The `Definable` count is a **downward ratchet**. Its old root cause was an `Int` with no
//! constructors, so its ring and order laws could not be proved and had to be assumed; `Int` is
//! now Lean's `ofNat | negSucc`, its operations are checked definitions and its laws are
//! kernel-checked proofs, and those 22 left. The last entry, `Quot_rec`, which Lean DEFINES (over
//! `Quot.lift` and `Quot.ind`), is now Lean's own definition too, and the class is empty.
//!
//! The other direction is locked too, and it is the one that matters most:
//! [`the_constants_lean_proves_are_never_logos_axioms`] names the constants that Lean *derives*
//! and requires each of ours to be a DEFINITION. `funext` is the reason it exists — an earlier
//! wave assumed it, which bought at permanent cost to the trusted base something Lean proves from
//! `Quot.sound` for free. It is now a kernel-checked theorem, and this lock is the door shutting
//! behind it.
//!
//! Wave 4 walked the same road for the classical layer. `dne` was the prelude's one home-grown
//! foundational assumption, and Lean does not have it: `Classical.em` is DIACONESCU'S THEOREM
//! (`Init/Classical.lean:32`, from `choice` + `funext` + `propext`) and `Classical.byContradiction`
//! — our `dne` — is two steps downstream of it. Porting that derivation removed `dne` from the
//! ledger, so the FOUNDATIONAL fragment of the trusted base is now exactly Lean's three
//! (`propext`, `Classical_choice`, `Quot_sound`) and not one more.
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THIS LOCK TO MAKE IT PASS.  ⚠️
//!  Adding an axiom to close a capability gap makes the gap disappear from the matrix and reappear
//!  as an unsound proof. That is the single worst move available in this campaign.

use std::collections::{BTreeMap, BTreeSet};

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::Context;

/// How an axiom is justified. See the module header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    Primitive,
    Definable,
    Reflection,
    DomainSpecific,
}

use Class::*;

/// THE LEDGER. Every axiom the standard prelude registers, with its justification.
///
/// To add an axiom you must add it here, which means saying out loud which class it is in. There
/// is deliberately no default.
const LEDGER: &[(&str, Class)] = &[
    // ---- Primitive: Lean has the same primitive. Defensible. -------------------------------
    // LEAN'S OWN THREE, under LOGOS spellings, and they are now the WHOLE foundational fragment:
    // `dne` LEFT this list in wave 4. It was the prelude's one home-grown classical assumption,
    // standing where Lean has a proof, and it is now the last link of Lean's own derivation
    // (`Classical.choice` -> `indefiniteDescription` -> `choose`, plus `funext` and `propext` ->
    // Diaconescu's `em` -> `propDecidable` -> `byContradiction`), so the classical layer costs the
    // trusted base exactly `Classical_choice` and nothing else. Keeping them is not a weakness; it is
    // parity, and it is what makes `Set`, quotient extensionality, `simp` on `↔` and almost every
    // Mathlib `#print axioms` line STATEABLE. `funext` — the fourth constant a Lean user reaches
    // for — is deliberately NOT here: Lean proves it from `Quot.sound` and so do we.
    ("propext", Primitive),
    ("Classical_choice", Primitive),
    // Quotients. Lean's kernel quotient primitives are `Quot`/`Quot.mk`/`Quot.lift`/`Quot.ind`,
    // and `Quot.sound` is one of Lean's three axioms. The CARRIER is universe-polymorphic as
    // Lean's is (`Quot.{u} : Π(A : Sort u). (A → A → Prop) → Sort u`), and so is `Quot_lift`'s
    // target (`Sort v`).
    ("Quot", Primitive),
    ("Quot_mk", Primitive),
    ("Quot_lift", Primitive),
    ("Quot_ind", Primitive),
    ("Quot_sound", Primitive),
    // `native_decide`. Exactly Lean's `reduceBool`/`ofReduceBool` pair — same shape, and the
    // trusted evaluator behind it differs in content. Lean calls its own `Lean.ofReduceBool`.
    ("reduceBool", Primitive),
    ("ofReduceBool", Primitive),
    // `Float`. Lean's is opaque and its operations are `@[extern] opaque Float.add : Float →
    // Float → Float` — they exist, they have types, the kernel never unfolds them, the compiler
    // implements them natively. Ours are declarations of exactly those types and the posture is
    // identical: no theorem about floating-point arithmetic is provable from them.
    //
    // THE ONE HONEST DIFFERENCE, stated rather than glossed: Lean's are `opaque` DEFINITIONS, so
    // they never appear in Lean's `#print axioms`; ours are DECLARATIONS, so they appear here.
    // That is because LOGOS's `Float` has no constructors — there is no body to write, and a
    // definition with an invented body would COMPUTE a wrong answer, the one outcome worse than
    // not computing. Each is nevertheless CONSERVATIVE: every one is a function type into an
    // already-declared carrier, so `λ_ _. c` inhabits it and no new closed proof of `False` can
    // pass through. The downward path is named in TCB.md — give `Float` a representation and
    // these six become `Definable`, then theorems.
    ("Float_add", Primitive),
    ("Float_sub", Primitive),
    ("Float_mul", Primitive),
    ("Float_div", Primitive),
    ("Float_lt", Primitive),
    ("Float_le", Primitive),
    // ---- Definable: Lean PROVES these. We owe a derivation. --------------------------------
    // All twenty-two original entries — the nine `Int` ring laws, the seven remaining order laws
    // and the six ALU operations — left once `Int` gained Lean's `ofNat`/`negSucc` constructors:
    // the operations are checked definitions (`int_inductive.rs`) and the laws are kernel-checked
    // proofs over them (`integer_laws.rs`). `Quot_rec` left last: Lean 4.14.0's `#print Quot.rec`
    // is `@[reducible] protected def Quot.rec.{u, v} … := fun f h q => Eq.ndrecOn ⋯ (Quot.lift
    // (Quot.indep f) ⋯ q).snd`, a DEFINITION over `Quot.lift` into `PSigma` and `Quot.ind` with
    // `#print axioms Quot.rec` = `[Quot.sound]`, and ours is that definition. All of them are
    // locked in `LEAN_PROVES` below, so re-assuming any goes red there as well as here.

    // ---- Reflection: the deep Syntax/Derivation embedding's operational semantics. ----------
    // Lean has no counterpart — this is the substrate `phase87_reflection.rs` through
    // `phase95_incompleteness.rs` build quoting, the diagonal lemma and Gödel's First
    // Incompleteness Theorem on. Axiomatising an operational semantics is a real trust cost and
    // is tracked as its own class rather than hidden among the primitives. The algebra over
    // `Syntax` (`REFLECTION_DEFINED`) is defined; the evaluator, which reads the global
    // environment, and `concludes` remain assumed.
    ("syn_step", Reflection),
    ("syn_eval", Reflection),
    ("concludes", Reflection),
    // Tactics as proof-term builders. `phase98_verified_reflection.rs` states the cost honestly in
    // its own header: the decision procedures are native Rust hooked into `normalize`, and the
    // "recompute in `concludes`" check re-runs the SAME Rust — so ~3,500 lines of unverified Rust
    // sit in the TCB and reflective proofs stop at `Syntax`, never reaching `Prop`.
    ("try_refl", Reflection),
    ("try_compute", Reflection),
    ("try_cong", Reflection),
    ("try_ring", Reflection),
    ("try_lia", Reflection),
    ("try_cc", Reflection),
    ("try_simp", Reflection),
    ("try_omega", Reflection),
    ("try_auto", Reflection),
    ("try_decide", Reflection),
    ("try_script", Reflection),
    ("try_induction", Reflection),
    ("try_inversion", Reflection),
    ("try_rewrite", Reflection),
    ("try_rewrite_rev", Reflection),
    ("try_destruct", Reflection),
    ("try_apply", Reflection),
    ("try_bitblast", Reflection),
    ("try_tabulate", Reflection),
    ("try_hw_auto", Reflection),
    // Structural tactics. These four were missed by a source-grep census of the prelude (their
    // registration call form differs) — which is precisely why this lock enumerates the live
    // `Context` instead of parsing `prelude.rs`. The true axiom count is 90, not the 86 a grep
    // reports.
    ("try_assumption", Reflection),
    ("try_split", Reflection),
    ("try_left", Reflection),
    ("try_right", Reflection),
    ("induction_base_goal", Reflection),
    ("induction_step_goal", Reflection),
    ("induction_num_cases", Reflection),
    // Tactic combinators.
    ("tact_fail", Reflection),
    ("tact_orelse", Reflection),
    ("tact_try", Reflection),
    ("tact_repeat", Reflection),
    ("tact_then", Reflection),
    ("tact_first", Reflection),
    ("tact_solve", Reflection),
    // ---- DomainSpecific: no Lean counterpart; LOGOS is a programming language too. ----------
    // Temporal/duration arithmetic for the first-class `Duration`/`Date`/`Moment` value types.
    // These are opaque primitive operations in the same sense as the ALU ones, and could be
    // defined if those types were given constructors — tracked separately because unlike `Int`
    // they have no Lean equivalent to be measured against.
    ("add_duration", DomainSpecific),
    ("sub_duration", DomainSpecific),
    ("mul_duration", DomainSpecific),
    ("div_duration", DomainSpecific),
    ("date_add_days", DomainSpecific),
    ("date_sub_date", DomainSpecific),
    ("moment_add_duration", DomainSpecific),
    ("moment_sub_moment", DomainSpecific),
    ("date_lt", DomainSpecific),
    ("moment_lt", DomainSpecific),
    ("duration_lt", DomainSpecific),
];

// ---- THE RATCHET. Only a human moves this, only DOWN. ---------------------------------------
/// Axioms Lean proves and we assume. 2026-09-14: seeded at 26 — the nine `Int` ring laws, the
/// eight order laws, and the nine ALU operations, all traceable to `Int` having zero constructors.
/// Making `Int` an inductive and proving its laws drives this to zero.
///
/// 2026-09-15 (wave 3): **26 → 22**, BANKED. `lt`, `ge` and `gt` are now DEFINITIONS over `le`
/// — Lean's own bodies, and none of them needs an `Int` constructor — and `lt_succ_le` became
/// the theorem `λa b h. h` along with its previously-missing converse `le_succ_lt`. The
/// remaining 22 all wait on the same one refactor: give `Int` its `ofNat`/`negSucc`
/// constructors and every one of them becomes provable.
///
/// 2026-09-23: **22 → 0**, BANKED. `Int` is Lean's two-constructor inductive; `add`, `sub`,
/// `mul`, `div`, `mod` and `le` are checked definitions and all sixteen laws are kernel-checked
/// proofs.
///
/// 2026-09-23, later: **0 → 1**, a CORRECTION and not a new assumption. `Quot_rec` had been filed
/// `Primitive` on the premise that Lean's `Quot.rec` is a kernel primitive; Lean 4.14.0 prints it
/// as a definition over `Quot.lift` and `Quot.ind` resting on `Quot.sound` alone. The count was
/// always one. Deriving `Quot_rec` takes it back to zero.
///
/// 2026-09-28: **1 → 0**, BANKED (`logos-coc-6`). `Quot_rec` is `recursor::derive_quot_rec`
/// registered as a checked definition over `PSigma`, at the statement the assumption had.
const DEFINABLE_CEILING: usize = 0;

/// Lean 4's own trusted base, for the comparison this lock exists to make, paired with the LOGOS
/// spelling of each. All three ARE registered here — that is parity, not debt, and
/// [`leans_own_three_axioms_are_all_present_under_logos_spellings`] pins it so a later wave cannot
/// shrink the count by dropping a capability.
const LEAN_AXIOMS: &[(&str, &str)] = &[
    ("propext", "propext"),
    ("Classical.choice", "Classical_choice"),
    ("Quot.sound", "Quot_sound"),
];

/// Constants Lean DERIVES rather than assumes — plus the ones LOGOS derives that Lean takes as
/// kernel recursors. Every one of them must be a DEFINITION here. A name on this list appearing in
/// `iter_declarations` means a wave bought a capability with an assumption instead of a proof.
///
/// `funext` heads the list because it is the one this campaign got wrong once: it is the fourth
/// constant a Lean user reaches for, it is NOT one of Lean's three axioms, and Lean proves it from
/// `Quot.sound`. The proof is in `prelude.rs::register_extensionality_and_choice` and its axiom
/// closure is exactly the four quotient primitives.
const LEAN_PROVES: &[(&str, &str)] = &[
    ("funext", "Lean proves it from `Quot.sound`; so do we — pointwise `Quot_lift` + η"),
    // THE WAVE-4 ENTRIES. `dne` heads them for the same reason `funext` heads the list: it was
    // ASSUMED, Lean proves it, and the derivation is Lean's own file read top to bottom
    // (`Init/Classical.lean`). Re-assuming any of the six puts a fourth foundational axiom back.
    ("dne", "Lean's `Classical.byContradiction` — `propDecidable`, then the `isFalse` branch"),
    ("Classical_em", "DIACONESCU — `Classical_choice` + `funext` + `propext` (Init/Classical.lean:32)"),
    ("Classical_propDecidable", "`Or_rec` on `em` into `Nonempty (Decidable p)`, then `choice`"),
    ("Classical_indefiniteDescription", "`choice` on `Nonempty (Subtype A p)` (Init/Classical.lean:15)"),
    ("Classical_choose", "`Subtype_val` of the description (Init/Classical.lean:25)"),
    ("Classical_choose_spec", "`Subtype_property` of the description (Init/Classical.lean:28)"),
    ("Classical_axiomOfChoice", "pointwise `Classical_choice` through `Subtype`"),
    // The `HEq` bridge. Lean proves `eq_of_heq` with a `rfl` its kernel accepts by K-like
    // iota-reduction; we prove the same `rfl` as a PROPOSITION (`cast_refl`) out of the proof
    // irrelevance the kernel already has. Assuming it instead would be Coq's `JMeq_eq`.
    ("cast", "Lean's `cast` — transport along an equation between carriers, written with J"),
    ("cast_refl", "UIP at the carrier, from proof irrelevance — `cast A A h a = a`"),
    ("eq_of_heq", "Init/Prelude.lean:464, with `cast_refl` where Lean's kernel uses K"),
    ("Eq_rec_dep", "auto-derived J — Lean takes `Eq.rec` as a kernel recursor"),
    ("Eq_rec", "derived from J by discarding the proof in the motive"),
    ("Eq_sym", "derived from J"),
    ("Eq_trans", "derived from J"),
    ("Or_rec", "auto-derived eliminator"),
    ("Ex_rec", "auto-derived eliminator"),
    ("HEq_rec", "derived type, `Match` body — Lean takes `HEq.rec` as a kernel recursor"),
    ("heq_of_eq", "proved by J"),
    ("Quotient", "`λa s. Quot a (Setoid_r a s)` — δ-transparent over the primitives"),
    ("Quotient_mk", "`λa s x. Quot_mk a (Setoid_r a s) x`"),
    // The rest of the API Mathlib actually calls. Lean's are one-liners over `Quot` and so are
    // ours (Init/Core.lean:1678, :1685, :1689, :1716).
    ("Quotient_sound", "`Quot_sound` at the setoid's relation"),
    ("Quotient_lift", "`Quot_lift` at the setoid's relation"),
    ("Quotient_ind", "`Quot_ind` at the setoid's relation"),
    ("Quotient_rec", "`Quot_rec` at the setoid's relation — the DEPENDENT eliminator"),
    ("Quot_rec", "Lean's `Quot.rec` (Init/Core.lean:1581) — `Quot_lift` into `PSigma`, `Quot_ind`, a transport"),
    ("nonemptyNat", "`Nonempty_intro Nat Zero`"),
    // The `Int` strict order. Lean writes `Int.lt a b := a + 1 ≤ b`, `GE.ge a b := LE.le b a` and
    // `GT.gt a b := LT.lt b a`, and once `lt` has that body the discreteness bridge is the
    // identity in both directions. Listed here so the door shuts behind them: re-opaquing any of
    // the five takes a proved fact back on trust, and it would show up as a capability (T08-021's
    // mixed `≤`/`<` `calc` chain) quietly going away rather than as a count going up.
    ("lt", "`Int.lt a b := a + 1 ≤ b` — integer discreteness, written as a body"),
    ("ge", "`GE.ge a b := LE.le b a` — Lean's definition, argument swap"),
    ("gt", "`GT.gt a b := LT.lt b a` — Lean's definition, argument swap"),
    ("lt_succ_le", "`λa b h. h` — `lt a b` IS `le (a+1) b`, so the bridge is a conversion"),
    ("le_succ_lt", "the converse, same proof; the only order law that CONCLUDES a strict `<`"),
    // THE `Int` ALU AND ITS LAWS — the whole former `Definable` class. Lean defines `Int` as
    // `ofNat | negSucc` (Init/Data/Int/Basic.lean:40) and every operation by matching on it, and
    // proves every law; so do we. Re-assuming one puts a derivation back on trust.
    ("add", "`Int_add` — Lean's `Int.add`, a match on `ofNat`/`negSucc` bottoming out in `Nat_add`"),
    ("sub", "`Int_sub` — Lean's `Int.sub := m + -n`"),
    ("mul", "`Int_mul` — Lean's `Int.mul`, a match on both constructors"),
    ("div", "checked integer division over the constructors (`int_inductive.rs`)"),
    ("mod", "checked integer remainder over the constructors (`int_inductive.rs`)"),
    ("le", "`Int_le` — a match on both constructors bottoming out in `natLe`"),
    ("add_comm", "proved over the constructors (`integer_laws.rs`)"),
    ("add_assoc", "proved over the constructors (`integer_laws.rs`)"),
    ("add_zero", "proved over the constructors (`integer_laws.rs`)"),
    ("mul_comm", "proved over the constructors (`integer_laws.rs`)"),
    ("mul_assoc", "proved over the constructors (`integer_laws.rs`)"),
    ("mul_one", "proved over the constructors (`integer_laws.rs`)"),
    ("mul_zero", "proved over the constructors (`integer_laws.rs`)"),
    ("mul_distrib_add", "proved over the constructors (`integer_laws.rs`)"),
    ("sub_elim", "proved over the constructors (`integer_laws.rs`)"),
    ("le_refl", "proved over the constructors (`integer_laws.rs`)"),
    ("le_trans", "proved over the constructors (`integer_laws.rs`)"),
    ("le_add_mono", "proved over the constructors (`integer_laws.rs`)"),
    ("le_mul_nonneg", "proved over the constructors (`integer_laws.rs`)"),
    ("le_sub", "proved over the constructors (`integer_laws.rs`)"),
    ("lt_add1_le", "proved over the constructors (`integer_laws.rs`)"),
    ("le_total", "proved over the constructors (`integer_laws.rs`)"),
];

fn prelude_axioms() -> BTreeSet<String> {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx.iter_declarations()
        .map(|(n, _)| n.to_string())
        .collect()
}

fn ledger() -> BTreeMap<&'static str, Class> {
    LEDGER.iter().copied().collect()
}

#[test]
fn the_ledger_has_no_duplicate_entries() {
    let mut seen = BTreeSet::new();
    let dupes: Vec<&str> = LEDGER
        .iter()
        .filter(|(n, _)| !seen.insert(*n))
        .map(|(n, _)| *n)
        .collect();
    assert!(dupes.is_empty(), "an axiom is classified twice: {dupes:?}");
}

#[test]
fn every_prelude_axiom_is_classified() {
    let actual = prelude_axioms();
    let known = ledger();
    let unclassified: Vec<&str> = actual
        .iter()
        .map(|s| s.as_str())
        .filter(|n| !known.contains_key(n))
        .collect();
    assert!(
        unclassified.is_empty(),
        "these axioms are in the prelude but not in the ledger. An axiom is a thing we ASSUME \
         rather than prove — adding one is a deliberate act and must be justified here, not \
         slipped in:\n  {unclassified:?}"
    );
}

#[test]
fn the_ledger_names_no_axiom_the_prelude_does_not_register() {
    let actual = prelude_axioms();
    let stale: Vec<&str> = LEDGER
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| !actual.contains(*n))
        .collect();
    assert!(
        stale.is_empty(),
        "the ledger lists axioms the prelude no longer registers — if one was PROVED rather than \
         assumed, that is a win and the ledger should record it by removing the entry and lowering \
         DEFINABLE_CEILING:\n  {stale:?}"
    );
}

#[test]
fn the_definable_axiom_count_only_falls() {
    let known = ledger();
    let actual = prelude_axioms();
    let definable: Vec<&str> = actual
        .iter()
        .map(|s| s.as_str())
        .filter(|n| known.get(n) == Some(&Definable))
        .collect();
    assert!(
        definable.len() <= DEFINABLE_CEILING,
        "Definable axioms rose to {} (ceiling {DEFINABLE_CEILING}). These are facts LEAN PROVES and \
         we merely assume — the count may only fall. Never raise the ceiling to go green:\n  {:?}",
        definable.len(),
        definable
    );
    if definable.len() < DEFINABLE_CEILING {
        eprintln!(
            "\n  ✓ Definable is {}, ceiling {DEFINABLE_CEILING} — lower DEFINABLE_CEILING to bank it.\n",
            definable.len()
        );
    }
}

/// The reason the `Int` laws LEFT the `Definable` class: `Int` has Lean's two constructors, so its
/// operations have bodies and its laws are proved rather than assumed.
#[test]
fn int_has_leans_two_constructors_which_is_why_its_laws_are_theorems() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    assert!(ctx.is_inductive("Int"), "Int is registered as an inductive");
    let ctors: Vec<&str> = ctx
        .get_constructors("Int")
        .iter()
        .map(|(n, _)| *n)
        .collect();
    assert_eq!(
        ctors,
        vec!["Int_ofNat", "Int_negSucc"],
        "Int must be Lean's `ofNat : Nat → Int | negSucc : Nat → Int`, in that order"
    );
    let axioms = prelude_axioms();
    for law in [
        "add",
        "sub",
        "mul",
        "div",
        "mod",
        "le",
        "add_comm",
        "add_assoc",
        "add_zero",
        "mul_comm",
        "mul_assoc",
        "mul_one",
        "mul_zero",
        "mul_distrib_add",
        "sub_elim",
        "le_refl",
        "le_trans",
        "le_add_mono",
        "le_mul_nonneg",
        "le_sub",
        "lt_add1_le",
        "le_total",
    ] {
        assert!(
            ctx.is_definition(law) && !axioms.contains(law),
            "`{law}` must be a kernel-checked definition over Int's constructors, not an axiom"
        );
    }
}

/// `Nat`, by contrast, IS an inductive with constructors — which is exactly why it needs no
/// axioms. The contrast is the argument for fixing `Int`.
#[test]
fn nat_has_constructors_and_therefore_needs_no_axioms() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    let ctors: BTreeSet<&str> = ctx
        .get_constructors("Nat")
        .iter()
        .map(|(n, _)| *n)
        .collect();
    assert!(
        ctors.contains("Zero") && ctors.contains("Succ"),
        "Nat must be a genuine inductive; found {ctors:?}"
    );
    let known = ledger();
    let nat_axioms: Vec<&str> = prelude_axioms()
        .iter()
        .map(|s| s.as_str())
        .filter(|n| n.starts_with("nat_") && known.get(n) == Some(&Definable))
        .map(|n| Box::leak(n.to_string().into_boxed_str()) as &str)
        .collect();
    assert!(
        nat_axioms.is_empty(),
        "Nat is inductive, so nothing about it should need assuming: {nat_axioms:?}"
    );
}

/// Not an assertion about size — a standing, visible statement of where we are against Lean.
/// Run with `-- --nocapture`.
#[test]
fn the_comparison_against_lean() {
    let actual = prelude_axioms();
    let known = ledger();
    let mut by_class: BTreeMap<Class, Vec<&str>> = BTreeMap::new();
    for name in actual.iter().map(|s| s.as_str()) {
        if let Some(c) = known.get(name) {
            by_class.entry(*c).or_default().push(name);
        }
    }

    eprintln!("\n  ══ TRUSTED BASE ══");
    eprintln!("  LOGOS: {} axioms", actual.len());
    eprintln!(
        "  Lean 4: {} axioms — {}",
        LEAN_AXIOMS.len(),
        LEAN_AXIOMS
            .iter()
            .map(|(l, _)| *l)
            .collect::<Vec<_>>()
            .join(", ")
    );
    eprintln!();
    for (class, names) in &by_class {
        eprintln!("    {:>3}  {:?}", names.len(), class);
    }
    eprintln!();
    if let Some(d) = by_class.get(&Definable) {
        eprintln!(
            "  the {} Definable ones are the debt — Lean proves each of these:",
            d.len()
        );
        for chunk in d.chunks(9) {
            eprintln!("    {}", chunk.join(" "));
        }
    }
    eprintln!();
}

/// THE DOOR SHUTTING BEHIND `funext`. Every constant Lean derives must be a DEFINITION here — a
/// kernel-checked body, not an assumption. This is the ratchet that actually protects the trusted
/// base, because the failure mode it catches is not "the count went up by one", it is "a
/// capability was bought with an assumption and the count going up by one was the receipt".
///
/// The test asserts BOTH halves on purpose. A later wave must not be able to satisfy it by
/// DELETING the constant: the capability has to still be there, and it has to still be proved.
#[test]
fn the_constants_lean_proves_are_never_logos_axioms() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    let assumed: Vec<String> = LEAN_PROVES
        .iter()
        .filter(|(n, _)| ctx.get_global(n).is_some() && !ctx.is_definition(n))
        .map(|(n, why)| format!("{n} ({why})"))
        .collect();
    assert!(
        assumed.is_empty(),
        "these are ASSUMED and Lean PROVES them. An axiom is a permanent cost; a derivation is \
         free. Write the proof, or say in the report why it cannot be written:\n  {assumed:?}"
    );
    let missing: Vec<&str> = LEAN_PROVES
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| ctx.get_global(n).is_none())
        .collect();
    assert!(
        missing.is_empty(),
        "the capability itself is gone. Deleting a constant is not a way to get this lock green — \
         Lean has every one of these:\n  {missing:?}"
    );
}

/// The reflection algebra over `Syntax`, each a checked structural recursion (`reflection.rs`): the
/// native rules accelerate these definitions and never stand in for an assumption again.
const REFLECTION_DEFINED: [&str; 7] = [
    "syn_size",
    "syn_max_var",
    "syn_lift",
    "syn_subst",
    "syn_beta",
    "syn_quote",
    "syn_diag",
];

#[test]
fn the_reflection_algebra_is_defined_never_assumed() {
    let ctx = StandardLibrary::context();
    for name in REFLECTION_DEFINED {
        assert!(
            ctx.get_global(name).is_some(),
            "`{name}` is still there: deleting it is not defining it"
        );
        assert!(
            ctx.is_definition(name),
            "`{name}` is assumed again; it is a checked definition"
        );
    }
}

/// The comparison has two directions and this is the second one. All three of Lean's own axioms
/// are registered here, so the 98-vs-3 gap is a gap in the count and NOT in what can be stated:
/// without `propext` there is no `Set` and no `simp` on `↔`, without `Classical_choice` almost
/// every Mathlib `#print axioms` line loses its head, and without `Quot_sound` there are no
/// quotients — nor, now, `funext`.
#[test]
fn leans_own_three_axioms_are_all_present_under_logos_spellings() {
    let actual = prelude_axioms();
    let known = ledger();
    for (lean, logos) in LEAN_AXIOMS {
        assert!(
            actual.contains(*logos),
            "Lean's `{lean}` has no LOGOS counterpart. Parity on the trusted base is not a smaller \
             set, it is the SAME set — dropping `{logos}` drops what it makes stateable"
        );
        assert_eq!(
            known.get(logos),
            Some(&Primitive),
            "`{logos}` is Lean's own `{lean}` and must be classified `Primitive`: it is parity, \
             not debt, and filing it as `Definable` would put a derivation we do not owe on the \
             ratchet"
        );
    }
}

/// The ledger says WHICH class; `TCB.md` says WHY. This reads the document and requires every
/// classified axiom to be named in it, so the argument cannot rot into a list of names.
///
/// Matching is on the backticked spelling — `` `add` `` and not the substring `add` — because a
/// three-letter axiom is a substring of half the prose in the file and a check that always passes
/// is worse than no check.
#[test]
fn every_ledger_entry_is_argued_in_the_tcb_document() {
    const TCB: &str = include_str!("../../logicaffeine_kernel/TCB.md");
    let undocumented: Vec<&str> = LEDGER
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| !TCB.contains(&format!("`{n}`")))
        .collect();
    assert!(
        undocumented.is_empty(),
        "these axioms are classified in the ledger but never argued in TCB.md. The class is the \
         verdict; the document is the reasoning, and a trusted base whose reasoning is missing is \
         a list of names:\n  {undocumented:?}"
    );
    for class in ["Primitive", "Definable", "Reflection", "DomainSpecific"] {
        assert!(
            TCB.contains(class),
            "TCB.md must explain what the `{class}` class means — the classes are the whole point \
             of the comparison against Lean"
        );
    }
}

/// `TCB.md`'s headline numbers are read out of the live `Context`, not typed in — a document that
/// says 98 while the kernel registers 99 is worse than one that says nothing, because a reader
/// stops checking after the first number that looks authoritative.
#[test]
fn the_tcb_document_reports_the_live_axiom_count() {
    const TCB: &str = include_str!("../../logicaffeine_kernel/TCB.md");
    let actual = prelude_axioms();
    assert!(
        TCB.contains(&format!("is **{}**", actual.len())),
        "TCB.md must state the live axiom count, which is {}. Run this test's sibling \
         `the_comparison_against_lean` with `--nocapture` for the per-class breakdown",
        actual.len()
    );
    let known = ledger();
    let mut counts: BTreeMap<Class, usize> = BTreeMap::new();
    for name in actual.iter().map(|s| s.as_str()) {
        if let Some(c) = known.get(name) {
            *counts.entry(*c).or_default() += 1;
        }
    }
    for (class, n) in &counts {
        assert!(
            TCB.contains(&format!("| `{class:?}` | {n} |")),
            "TCB.md's class table must say `{class:?}` is {n}; the kernel is the source of truth \
             and the table is read from it"
        );
    }
}

/// The kernel's README (its crates.io and docs.rs page) states the same inventory `TCB.md` does, read from the
/// same live registration, so the number a reader meets first is the kernel's own.
#[test]
fn the_kernel_readme_reports_the_live_axiom_count() {
    const README: &str = include_str!("../../logicaffeine_kernel/README.md");
    let actual = prelude_axioms();
    assert!(
        README.contains(&format!("{} named assumptions", actual.len()))
            && README.contains(&format!("The {} assumptions", actual.len())),
        "the kernel README must state the live axiom count, which is {}",
        actual.len()
    );
    let known = ledger();
    let mut counts: BTreeMap<Class, usize> = BTreeMap::new();
    for name in actual.iter().map(|s| s.as_str()) {
        if let Some(c) = known.get(name) {
            *counts.entry(*c).or_default() += 1;
        }
    }
    for (class, n) in &counts {
        assert!(
            README.contains(&format!("{n} `{class:?}`")),
            "the kernel README must say {n} `{class:?}`; the kernel is the source of truth"
        );
    }
}
