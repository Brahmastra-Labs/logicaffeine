/-!
@case      T01-008
@tier      1 · Universes
@title     Per-instantiation monomorphisation — the only partial escape from T01-007, with its own losses
@lean      universe-polymorphic inductive instantiated at finitely many levels; universe-polymorphic
           lemmas ABOUT it
@expect    degrade
@target    One COMPLETE set of LOGOS declarations per distinct universe instantiation appearing
           in the source: `add_indexed_inductive("Box@0", …)` / `add_indexed_inductive("Box@1", …)`
           (kernel/src/context.rs:374), their own `add_constructor_checked` calls
           (context.rs:545), and their own `derive_recursor` + `add_definition`
           (kernel/src/recursor.rs:36). Plus a loss record naming UNIVERSE_MONOMORPHISATION that
           enumerates the instantiations emitted.
@evidence  kernel/src/context.rs:253, :374, :419, :545 (`add_inductive`, `add_indexed_inductive`,
           `add_constructor`, `add_constructor_checked` — none takes a universe-parameter list,
           which is why T01-007 is a rejection and this is the only fallback);
           kernel/src/context.rs:218 (`add_universe_poly` binds parameters for DEFINITIONS only);
           kernel/src/recursor.rs:36 and :64-71 (`derive_recursor` synthesises `(type, term)`
           with the motive target HARD-CODED: `Sort(Prop)` for a non-subsingleton `Prop`
           inductive, `Sort(Type 0)` for everything else — so even `Box@1`'s recursor eliminates
           only into `Type 0`);
           kernel/src/context.rs:449 (`get_global` is keyed on a flat `String`, so `Box@0` and
           `Box@1` are unrelated globals with no relation recorded anywhere);
           kernel/src/type_checker.rs:1330-1360 (`is_subtype_normalized`: cumulativity applies to
           two `Sort`s and covariantly under a `Pi` codomain; every OTHER position, including a
           type constructor's argument, is INVARIANT and delegates to `def_eq_normalized`)
@loss      UNIVERSE_MONOMORPHISATION. The single Lean declaration becomes N unrelated LOGOS
           declarations. Anything the Lean source states ONCE about all levels must be restated
           N times, and anything it states for a level variable cannot be restated at all.
           Acceptable only when (a) the set of instantiations is finite and fully determined
           statically, and (b) no polymorphic lemma quantifies over the level. Both conditions
           must be CHECKED, not assumed; when either fails the verdict escalates to T01-007's
           rejection.
@loss      RECURSOR_LEVEL_CEILING. `derive_recursor` pins the motive target to `Sort(Type 0)`
           (recursor.rs:64-71). Monomorphising `Box` at `Type 1` gives you a `Box@1` whose
           ELEMENTS live at `Type 1` and whose recursor still eliminates only into `Type 0`. So
           monomorphisation does not actually recover large elimination at the higher level — it
           recovers the type and loses the eliminator. Any Lean proof that recurses over a
           `Type 1`-valued container into a `Type 1` motive has no image.
@loss      CROSS_LEVEL_IDENTITY. In Lean, `Box.mk` at level 0 and at level 1 are the SAME
           constant at two instantiations, so a lemma proved about `Box.mk` applies to both.
           After monomorphisation `Box@0_mk` and `Box@1_mk` are different globals; nothing in the
           LOGOS context records that they were one declaration, and no lemma transfers.
@guard     Catches monomorphising at `Type 0` ONLY and reusing that declaration everywhere. This
           is T01-007's headline trap and it is worth restating here because monomorphisation is
           the very path that invites it: having decided to monomorphise, it is tempting to emit
           one instance and let cumulativity absorb the rest. It will — `Type 0 ≤ Type 1` holds
           (type_checker.rs:1333) — right up to the point where the collapsed instances are
           compared in an INVARIANT position (a type constructor's argument,
           type_checker.rs:1356-1358), where `def_eq_normalized` compares the two `Box@0`s and
           finds them equal, because they are. Two Lean types that were distinct have become one.
@guard     Catches discovering the instantiation set from the declaration rather than from the
           WHOLE PROGRAM. A translator that emits `Box@0` when it first sees `Box Nat` and then
           meets `Box Type` must go back and emit `Box@1` — and every already-emitted term that
           mentioned `Box` must be re-checked against which instance it meant. A single-pass
           translator cannot do this and will silently attach the second use to the first
           instance.
@guard     Catches monomorphising the TYPE and forgetting the CONSTRUCTORS, the RECURSOR, or the
           `Match` cases. `add_constructor` (context.rs:419) is UNCHECKED — it will happily
           attach `Box@1_mk : Π(α : Type 1). α → Box@0 α` to the wrong inductive, and
           `Match`'s positional check (type_checker.rs:254-260) only counts cases, so a `Match`
           on a `Box@1` value with `Box@0`'s cases passes the count test.
@guard     Catches a polymorphic LEMMA. `polyBoxId` below quantifies over `u`; there is no level
           variable to bind on a non-definition and the lemma is about an inductive, so it can
           only be emitted once per instantiation — which is fine ONLY because this file
           instantiates it finitely. A source that proves a lemma for all `u` and then applies it
           at a level determined by a later module has no finite instantiation set, and the
           translator must REJECT rather than guess a set.
@guard     Catches the naming scheme colliding. LOGOS keys are flat `String`s with one namespace
           (context.rs:449); `Box@0` is a name the translator invented, and a Lean declaration
           literally named `Box@0` — or a different mangling that produces the same string — key
           to the same slot. The mangling must be injective and must be checked against
           `get_global` before registration (T02-020).
@oracle    For each universe-polymorphic inductive, the emitted context contains one complete
           declaration group per distinct instantiation reachable in the source (type,
           constructors via `add_constructor_checked`, derived recursor); no two Lean
           instantiations share a LOGOS global; a loss record with category
           UNIVERSE_MONOMORPHISATION enumerates them; and the translator EXITS NON-ZERO if any
           use site's level is not statically determined, rather than emitting a default.
-/

namespace T01_008

/-- The polymorphic inductive. Under T01-007 this alone is a rejection; monomorphisation is the
    escape, and it is only available because this file pins the instantiation set. -/
inductive Box (α : Type u) : Type u where
  | mk : α → Box α

/-- Instantiation at `Type 0` — `Box@0`. -/
def boxNat : Box Nat := Box.mk 3
def boxBool : Box Bool := Box.mk true

/-- Instantiation at `Type 1` — `Box@1`. A DIFFERENT LOGOS declaration, with its own
    constructor and its own recursor. -/
def boxType : Box Type := Box.mk Nat
def boxTypeAgain : Box Type := Box.mk Bool

/-- NESTED instantiation: `Box (Box Nat)` is still level 0, so it reuses `Box@0` — the
    translator must not emit a third instance for it. -/
def boxBoxNat : Box (Box Nat) := Box.mk (Box.mk 3)

/-- Nested ACROSS levels: the outer `Box` is at level 1 because its argument `Box Nat` is a
    `Type 0`… which is itself level 0. `Box (Box Nat) : Type 0`. The genuinely level-1 nesting
    is below. -/
def boxOfBoxType : Box (Box Type) := Box.mk (Box.mk Nat)

/-- Eliminating a `Box@0`. The derived recursor's motive is pinned to `Type 0`
    (recursor.rs:64-71), which happens to be enough here. -/
def unbox (b : Box Nat) : Nat :=
  match b with
  | Box.mk a => a

example : unbox boxNat = 3 := rfl

/-- Eliminating a `Box@1` into a `Type 1` motive — legal in Lean, and NOT expressible through a
    LOGOS derived recursor, whose motive target is hard-coded to `Sort(Type 0)`. This is the
    RECURSOR_LEVEL_CEILING loss made concrete. -/
def unboxType (b : Box Type) : Type :=
  match b with
  | Box.mk a => a

/-- A universe-polymorphic LEMMA about the inductive. In Lean this is one declaration covering
    every level; after monomorphisation it must be emitted once per instantiation, and there is
    no level variable to bind because it mentions an inductive. -/
theorem polyBoxId.{u} {α : Type u} (b : Box α) : b = b := rfl

/-- The lemma used at both instantiations, which is what makes the set finite and the
    degradation acceptable here. -/
theorem boxNatRefl : boxNat = boxNat := polyBoxId boxNat
theorem boxTypeRefl : boxType = boxType := polyBoxId boxType

/-- A polymorphic FUNCTION over the inductive. Its universe parameter CAN be bound
    (`add_universe_poly`, context.rs:218) because it is a definition — but its TYPE mentions
    `Box`, which is monomorphic, so the binding is useless: the body can only refer to one
    instance. This is the exact seam where the two mechanisms fail to compose. -/
def mapBox.{u} {α : Type u} {β : Type u} (f : α → β) (b : Box α) : Box β :=
  match b with
  | Box.mk a => Box.mk (f a)

def mapAtZero : Box Nat := mapBox (fun n => n + 1) boxNat
def mapAtOne : Box Type := mapBox (fun _ : Type => Bool) boxType

end T01_008
