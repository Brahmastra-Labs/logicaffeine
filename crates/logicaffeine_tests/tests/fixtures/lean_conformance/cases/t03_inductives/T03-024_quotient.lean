/-!
@case      T03-024
@tier      3 · Inductives and structures
@title     `Quot` / `Quotient` — LOGOS's quotients exist but are pinned to `Type 0`
@lean      Quot, Quot.mk, Quot.lift, Quot.sound, Quot.ind, Quotient, Setoid, Quotient.mk,
           Quotient.lift, Quot.liftOn
@target    the prelude's `Quot`, `Quot_mk`, `Quot_lift`, `Quot_sound`, `Quot_ind` — mapped
           onto, never re-declared — plus a loss record; `Quotient`/`Setoid` must be *unfolded*
           to their `Quot` forms before translation
@expect    degrade
@evidence  kernel/src/prelude.rs:1230-1338 (the whole quotient block);
           kernel/src/prelude.rs:1249 (`add_declaration("Quot", Π(A:Type 0). (A→A→Prop) →
           Type 0)` — an **axiom**, and `Type 0` on both sides, hard-coded);
           kernel/src/prelude.rs:1252 (`Quot_mk`), :1270 (`Quot_lift`), :1291 (`Quot_sound`),
           :1320 (`Quot_ind`, whose motive is restricted to `Prop`);
           kernel/src/reduction.rs:429-439 (the computation rule
           `Quot_lift … (Quot_mk … a) ⤳ f a` — the only reduction quotients have);
           kernel/src/term.rs:169-191 (`Universe::Type(u32)` is a concrete level, and an
           inductive cannot carry universe parameters at all — T01-007 — so there is nowhere to
           put `Quot`'s `u`);
           kernel/src/context.rs:20-100 (no `Setoid`, no `Quotient`, no instance-driven
           quotient formation — see LOGOS_FACTS "Absent entirely")
@loss      QUOTIENT_UNIVERSE_PINNED. Lean's `Quot : {α : Sort u} → (α → α → Prop) → Sort u` is
           universe-polymorphic. LOGOS's is `Π(A : Type 0). (A → A → Prop) → Type 0`. A
           quotient of a `Prop`, of a `Type 1`, or at a level variable has no counterpart;
           cumulativity (`Prop ≤ Type 0`, type_checker.rs:1330-1360) makes the *argument*
           direction go through and does nothing for the result.
@loss      NO_SETOID_LAYER. `Quotient`, `Setoid`, `Quotient.mk'`, `Quotient.lift₂`,
           `Quot.liftOn`, `Quotient.sound`, `Quotient.exact` and the `≈` notation are all
           absent. Every one must be unfolded to its `Quot` core *in Lean*, before translation,
           and the instance-driven resolution of the `Setoid` argument disappears with them.
@loss      LIFT_IS_THE_ONLY_ELIMINATOR. LOGOS has `Quot_lift` (into `Type 0`) and `Quot_ind`
           (motive restricted to `Prop`, prelude.rs:1320). There is no dependent
           `Quot.rec`/`Quot.hrecOn`, so a Lean definition by `Quot.rec` with a genuinely
           dependent motive cannot be transcribed — it must be rejected, not approximated.
@loss      QUOT_IS_AXIOMATIC_HERE. `Quot` is an `add_declaration` (prelude.rs:1249), i.e. an
           axiom, not an inductive: `is_inductive("Quot")` is false, it has no constructors in
           the `constructors` map, and **it cannot be matched on**. Its only computational
           content is the one reduction rule at reduction.rs:429-439.
@guard     Catches re-declaring `Quot`. The name is taken by a prelude axiom, and
           `add_declaration` is a silent `HashMap::insert` (context.rs:434-436). Overwriting it
           breaks the hard-coded reduction rule, which matches on the literal names `Quot_lift`
           and `Quot_mk` (reduction.rs:429-439).
@guard     Catches translating `Quot` as an **inductive** with `Quot.mk` as its constructor.
           That looks harmless and is not: an inductive `Quot` can be *matched on*, so a
           translated `Match` could recover the representative from an equivalence class,
           which is exactly what `Quot.sound` makes contradictory. The prelude's choice of
           `add_declaration` over `add_inductive` is deliberate.
@guard     Catches translating `Quotient.mk`/`Quotient.lift` directly rather than unfolding to
           `Quot`. There is no `Quotient` global; the emitted term would carry an unbound name
           that `get_global` resolves to `None` (context.rs:449-464), reported at first use.
@guard     Catches silently monomorphising a `Sort u`-valued quotient to `Type 0`. If the Lean
           source quotients a `Prop`, the result in Lean is a `Prop`; at `Type 0` it becomes
           data, proof irrelevance stops applying (type_checker.rs:1547-1567) and the
           large-elimination gate stops firing (T03-012, T01-007).
@guard     Catches `Quot.lift`'s **respect obligation**. `Quot_lift` takes a proof that the
           function respects the relation; a translator that drops it (because Lean inferred it
           by unification) produces an application with too few arguments, which
           `add_definition` accepts unchecked (context.rs:442-444).
@guard     Catches `Quot_ind`'s motive restriction. It is `Prop`-only (prelude.rs:1320), so a
           Lean `Quot.ind` used to build *data* has no LOGOS counterpart.
@oracle    Every `Quot`/`Quotient` occurrence resolves to the prelude's `Quot`, `Quot_mk`,
           `Quot_lift`, `Quot_sound` or `Quot_ind` with no re-registration; `Quotient`/`Setoid`
           appear nowhere in the emitted term; `Quot_lift f h (Quot_mk A r a)` reduces to
           `f a`; every quotiented carrier is `Type 0`; and the loss report contains
           QUOTIENT_UNIVERSE_PINNED and NO_SETOID_LAYER.
-/

namespace T03_024

/-- A relation on a `Type 0` carrier — the only shape LOGOS's `Quot` accepts. -/
def sameParity (a b : Nat) : Prop := a % 2 = b % 2

/-- The raw `Quot` layer, which maps one-for-one onto the prelude. -/
def Parity : Type := Quot sameParity

def mkParity (n : Nat) : Parity := Quot.mk sameParity n

/-- `Quot.lift` with its respect obligation written out — the argument a translator must not
    drop. -/
def parityBit : Parity → Nat :=
  Quot.lift (fun n => n % 2) (fun _ _ h => h)

/-- The computation rule: `lift f h (mk a) ⤳ f a`. This is the *only* reduction quotients
    have in LOGOS (reduction.rs:429-439). -/
example : parityBit (mkParity 4) = 0 := rfl
example : parityBit (mkParity 7) = 1 := rfl

/-- `Quot.sound` — related representatives give equal classes. This is what makes an
    inductive encoding of `Quot` unsound: a `Match` recovering the representative would
    contradict it. -/
example : mkParity 2 = mkParity 4 := Quot.sound rfl

/-- `Quot.ind`, whose motive LOGOS restricts to `Prop` (prelude.rs:1320). -/
example (P : Parity → Prop) (h : ∀ n, P (mkParity n)) (q : Parity) : P q :=
  Quot.ind h q

/-- The **`Quotient`/`Setoid` layer**, which has no LOGOS counterpart at all and must be
    unfolded to the `Quot` forms above before translation. -/
instance paritySetoid : Setoid Nat where
  r := sameParity
  iseqv := ⟨fun _ => rfl, fun h => h.symm, fun h1 h2 => h1.trans h2⟩

def ParityQ : Type := Quotient paritySetoid

def mkParityQ (n : Nat) : ParityQ := Quotient.mk paritySetoid n

def parityBitQ : ParityQ → Nat :=
  Quotient.lift (fun n => n % 2) (fun _ _ h => h)

example : parityBitQ (mkParityQ 6) = 0 := rfl

/-- `Quotient.sound` — the `Setoid`-flavoured version of the same axiom. -/
example : mkParityQ 1 = mkParityQ 3 := Quotient.sound rfl

/-- A **`Prop`-valued quotient**. Lean's `Quot` is `Sort u`-polymorphic so this is legal;
    LOGOS's `Quot` takes a `Type 0` carrier and returns `Type 0`, so there is no counterpart.
    Silently placing it at `Type 0` turns a proposition into data. -/
def PropQuot (P : Prop) : Prop := Quot (fun (_ _ : P) => True)

def mkPropQuot {P : Prop} (h : P) : PropQuot P := Quot.mk _ h

/-- A quotient of a **`Type 1`** carrier — above LOGOS's pinned level in the other direction,
    and not rescued by cumulativity. -/
def bigRel (a b : Type) : Prop := (a → b) → True

def BigQuot : Type 1 := Quot bigRel

def mkBig (α : Type) : BigQuot := Quot.mk bigRel α

/-
A **dependent** elimination via `Quot.rec` has no LOGOS counterpart. Lean's

    Quot.rec : {α : Sort u} → {r : α → α → Prop} → {motive : Quot r → Sort v} →
               ((a : α) → motive (Quot.mk r a)) →
               ((a b : α) → (h : r a b) → Eq.ndrec (f a) (Quot.sound h) = f b) →
               (q : Quot r) → motive q

takes a motive that varies over the quotient and a respect obligation stated *up to
transport*. LOGOS's prelude offers exactly two eliminators: `Quot_lift` (prelude.rs:1270),
non-dependent and into `Type 0`, and `Quot_ind` (prelude.rs:1320), whose motive is
restricted to `Prop`. Neither can express a dependent elimination into `Type`, and there is
no third. A Lean definition built by `Quot.rec` with a genuinely dependent motive is
therefore a rejection, not a degrade — the translator must not approximate it with
`Quot_lift` at the motive's instantiation, because that discards the transport and the term
it produces has a different type from the one the source declared.
-/

/-- The non-dependent eliminator, which *is* available — for contrast with the above. -/
def parityIsEven : Parity → Bool :=
  Quot.lift (fun n => n % 2 == 0) (fun _ _ h => by simp [sameParity] at h; simp [h])

example : parityIsEven (mkParity 4) = true := rfl
example : parityIsEven (mkParity 5) = false := rfl

/-- Two distinct representatives whose classes are equal — the property that any encoding
    permitting a `Match` on the quotient would break. -/
example : parityBit (mkParity 2) = parityBit (mkParity 8) := rfl

end T03_024
