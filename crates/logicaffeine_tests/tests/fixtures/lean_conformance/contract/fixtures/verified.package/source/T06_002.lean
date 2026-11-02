/-!
@case      T06-002
@tier      6 · Propositions, equality, classical reasoning
@title     `Eq.rec` / J — LOGOS DERIVES the equality eliminator rather than axiomatising it
@lean      Eq.rec, Eq.ndrec, Eq.mpr, transport / subst along an equation
@expect    translate
@target    `Eq_rec_dep` — the auto-derived dependent eliminator (Paulin-Mohring J) registered
           as an ordinary `Context::add_definition`, NOT an `add_declaration`. Its type is
           `Π(A0). Π(A1:A0). Π(motive: Π(Idx0:A0). Eq A0 A1 Idx0 → Type 0).
            motive A1 (refl A0 A1) → Π(Idx0:A0). Π(h: Eq A0 A1 Idx0). motive Idx0 h`.
           The non-dependent `Eq_rec : Π(A). Π(x:A). Π(P:A→Prop). P x → Π(y:A). Eq A x y → P y`
           is itself DERIVED from `Eq_rec_dep`, also as a definition.
@evidence  kernel/src/prelude.rs:1481-1487 (`register_eq_recursor` calls
           `recursor::derive_recursor(ctx, "Eq")` and installs the result with `add_definition`
           — "a kernel-CHECKED definition … not an axiom — so the equality eliminator and its
           lemmas leave the trusted base"); kernel/src/recursor.rs:36 (`derive_recursor`
           returns `(type, term)`; the CALLER must add it); kernel/src/recursor.rs:60-71 (the
           motive target is hard-coded: `Sort(Prop)` for a non-subsingleton `Prop` inductive,
           `Sort(Type 0)` otherwise — `Eq` is a subsingleton `Prop` by
           type_checker.rs:1688-1716, so its motive lands in `Type 0`);
           kernel/src/term.rs:443-545 (`Match` is a `Term` variant; there is no recursor variant
           — recursors are *derived terms* in LOGOS, the inverse of Lean 4)
@guard     Catches the translator emitting `Eq.rec` as an AXIOM (`add_declaration("Eq_rec", …)`).
           That is a silent trusted-base expansion of exactly the kind T06-009/010/011 are
           about: the prelude deliberately proves this eliminator so it is NOT trusted
           (prelude.rs:1481-1484), and `certificate::recheck` rebuilds the context from
           `StandardLibrary::register` alone (certificate.rs:73-75), so a certificate whose
           proof term mentions a translator-invented `Eq_rec` axiom either fails to re-check
           with an unhelpful `UnboundVariable`, or — if the translator also patches the prelude
           — grows the trusted base while `PRELUDE_VERSION` still reads `"logos-coc-1"`
           (certificate.rs:28).
@guard     Catches assuming LOGOS's J eliminates into an arbitrary universe. Lean's `Eq.rec` has
           `{motive : (a' : α) → a = a' → Sort u}`. The derived `Eq_rec_dep`'s motive target is
           literally `Term::Sort(Universe::Type(0))` (recursor.rs:64-71) — there is no level to
           instantiate. A transport whose motive lands in `Type 1` (e.g. transporting a *type
           family*, `motive : Nat → Type 1`) has no translation via `Eq_rec_dep`. LOGOS's
           cumulativity (`Prop ≤ Type 0`, type_checker.rs:1330-1360) hides the `Prop`-motive
           case, so a translator tested only on `subst`-shaped goals will never notice.
@guard     Catches emitting `Eq.mpr`/`Eq.ndrec`/`Eq.subst` as four separate axioms because each
           has a distinct Lean name. They are all `Eq_rec_dep` instances; the prelude already
           derives `Eq_rec` (prelude.rs:1489), `Eq_sym` (:1597) and `Eq_trans` (:1684) as
           definitions and the translator must map onto those, not mint new globals.
@oracle    Every emitted transport term typechecks; the translator adds no `add_declaration`
           whose name matches `Eq_rec*`/`Eq_sym`/`Eq_trans`; and `bigMotive` below is escalated
           because its motive target exceeds `Type 0`.
-/

namespace T06_002

/-- Non-dependent transport — the `Eq_rec` shape. -/
theorem transport {α : Type} (P : α → Prop) {a b : α} (h : a = b) (pa : P a) : P b :=
  h ▸ pa

/-- The raw recursor application, which is what the translator sees after elaboration. -/
theorem transportRaw {α : Type} (P : α → Prop) {a b : α} (h : a = b) (pa : P a) : P b :=
  @Eq.rec α a (fun x _ => P x) pa b h

/-- Fully dependent J: the motive mentions the equation proof itself. This is the shape that
    only `Eq_rec_dep` can carry — a non-dependent `Eq_rec` is not enough. -/
theorem jDependent {α : Type} {a : α}
    (motive : (b : α) → a = b → Prop)
    (base : motive a rfl) : ∀ (b : α) (h : a = b), motive b h :=
  fun b h => @Eq.rec α a motive base b h

/-- `Eq.symm` and `Eq.trans` are themselves J instances; LOGOS derives both as definitions
    (`Eq_sym`, `Eq_trans`) rather than axiomatising them. -/
theorem symmViaRec {α : Type} {a b : α} (h : a = b) : b = a := h ▸ rfl

/-- BOUNDARY. The motive lands in `Type 1`, which the derived `Eq_rec_dep` cannot express:
    its motive target is hard-coded to `Type 0`. -/
def bigMotive {a b : Nat} (h : a = b) (F : Nat → Type) : F a → F b :=
  fun x => @Eq.rec Nat a (fun n _ => F n) x b h

/-- `Eq.mpr` — a distinct Lean name for the same eliminator. Must NOT become a new global. -/
theorem viaMpr (a b : Nat) (h : a = b) (p : b = b) : a = a :=
  Eq.mpr (by rw [h]) p

end T06_002
