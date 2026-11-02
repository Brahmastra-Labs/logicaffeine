/-!
@case      T06-010
@tier      6 · Propositions, equality, classical reasoning
@title     `propext` — ABSENT from the LOGOS trusted base, and not derivable from `dne`
@lean      propext, Iff.to_eq, propositional extensionality, `Eq` between two `Prop`s
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/prelude.rs:33-44 (`StandardLibrary::register` — the complete trusted
           base); kernel/src/prelude.rs:46-78 (`register_classical` adds exactly one
           declaration, `dne`); a grep of `crates/logicaffeine_kernel/src` for `propext` returns
           NOTHING; kernel/src/prelude.rs:1422-1439 (`Eq : Π(A : Type 0). A → A → Prop` — the
           carrier is pinned to `Type 0`, and `Prop : Type 1` by term.rs:198, so the very
           STATEMENT of `propext` is not expressible: `Eq Prop p q` does not typecheck, because
           `Type 1 ≤ Type 0` is false even under LOGOS cumulativity,
           type_checker.rs:1330-1360); a grep for `Iff` returns NOTHING either (see T06-005);
           kernel/src/certificate.rs:28,63-75 (`PRELUDE_VERSION`; `recheck` rebuilds the trusted
           context from `StandardLibrary::register` and never trusts a carried one)
@diag      E_AXIOM_NOT_IN_TRUSTED_BASE (primary) — must name `propext` and state that the LOGOS
           trusted base contains exactly one classical axiom, `dne` (prelude.rs:78).
@diag      E_EQ_CARRIER_NOT_TYPE0 (secondary, and it fires FIRST) — `propext`'s conclusion is an
           equation between two `Prop`s, and LOGOS's `Eq` cannot even be APPLIED at that
           carrier. The translator must report this rather than silently widening `Eq`.
@guard     This case has a second, nastier failure mode than T06-009, because `propext` cannot
           be papered over with one `add_declaration`. To state it at all, a translator must
           ALSO widen `Eq` — either by re-registering `Eq` with a `Sort(Universe::Type(1))` or
           `Sort(Universe::Var _)` carrier, or by passing `Term::Hole` for the carrier. Both are
           catastrophic:
             • Re-registering `Eq` replaces the prelude's entry in the inductives map
               (context.rs:253/374, lookup order at :449) while the ALREADY-DERIVED `Eq_rec_dep`,
               `Eq_rec`, `Eq_sym` and `Eq_trans` definitions (prelude.rs:1481-1783) still carry
               bodies elaborated against the original registration. The environment is then
               internally inconsistent and the inconsistency is invisible to `infer_type`.
             • Passing `Term::Hole` is worse still: `types_equal` treats `Hole` as matching
               ANYTHING (type_checker.rs:1594-1597) and that leniency leaks into `is_subtype`'s
               fast path (:1314), so `Eq Hole p q` will unify with an equation at a completely
               unrelated carrier. That is a direct route to a proof of `False`.
@guard     Catches "`propext` is harmless, it is just extensionality". It is not harmless in
           LOGOS: `Eq` is the carrier of the prelude's entire rewriting apparatus, and `propext`
           makes every pair of equivalent propositions definitionally interchangeable under
           `Eq_rec_dep`. Combined with `Quot` (prelude.rs:1230-1338), `propext` plus choice is
           the standard route to Diaconescu; `propext` alone still collapses distinctions that
           `proof_irrelevant` (type_checker.rs:1547-1567) deliberately keeps at the level of
           PROOFS rather than PROPOSITIONS.
@guard     Catches the substitution `Iff` for `Eq`. There is no `Iff` in LOGOS either
           (T06-005); a translator that emits `p ↔ q` in place of `p = q` has changed the
           theorem, and every downstream `rw`/`Eq_rec_dep` step that expected an equation will
           fail to apply — or, if the translator also emits a cast, will apply a term whose type
           was never checked against the original.
@guard     Catches the scale error, as in T06-009. `propext` is one of Lean 4's three core
           axioms. `Set`, `Setoid`, quotient extensionality, and `simp`'s treatment of `Iff` all
           rest on it; rejecting it correctly rejects most of Mathlib, which is the honest
           result.
@oracle    Translator exits non-zero for every declaration below, emits
           E_EQ_CARRIER_NOT_TYPE0 and/or E_AXIOM_NOT_IN_TRUSTED_BASE, performs zero
           `add_declaration` calls, and in particular never re-registers `Eq` and never emits a
           `Term::Hole` in a carrier position.
-/

namespace T06_010

/-- The axiom itself. Note the conclusion `a = b` is an equation between two `Prop`s. -/
theorem ext {a b : Prop} (h : a ↔ b) : a = b := propext h

/-- The realistic shape: a rewrite that only works because equivalent propositions are equal. -/
theorem rewriteByIff (p q : Prop) (h : p ↔ q) (P : Prop → Prop) (hp : P p) : P q :=
  (propext h) ▸ hp

/-- `iff_iff_eq` — the same axiom behind a different Lean name, so a name-keyed allowlist
    misses it; and `rw [h]` reaches it with no name at all. -/
theorem toEq {a b : Prop} (h : a ↔ b) : a = b := (iff_iff_eq).mp h
theorem toEqByRw {a b : Prop} (h : a ↔ b) : a = b := by rw [h]

/-- Extensionality collapsing two syntactically unrelated true propositions to one. This is
    the instance that shows `propext` is not a bookkeeping lemma: after it, `True` and
    `0 = 0` are the SAME object, not merely equivalent ones. -/
theorem collapse : True = ((0 : Nat) = 0) :=
  propext (Iff.intro (fun _ => rfl) (fun _ => True.intro))

/-- Predicate extensionality, which is `propext` composed with `funext` (T06-011) — both
    absent, and the elaborated term mentions both. -/
theorem predExt (p q : Nat → Prop) (h : ∀ n, p n ↔ q n) : p = q :=
  funext (fun n => propext (h n))

/-- An innocuous-looking `simp`-style lemma whose proof term mentions `propext`. This is the
    realistic case: the Lean source never types the word `propext`. -/
theorem innocuous (p : Prop) : (p ∧ True) = p :=
  propext (Iff.intro (fun h => h.left) (fun h => ⟨h, True.intro⟩))

end T06_010
