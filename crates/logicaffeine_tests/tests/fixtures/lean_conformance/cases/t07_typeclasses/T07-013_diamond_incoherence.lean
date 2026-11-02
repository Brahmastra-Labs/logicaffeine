/-!
@case      T07-013
@tier      7 · Typeclasses
@title     Instance diamond INCOHERENCE — two resolution paths that disagree, which LOGOS
           cannot detect
@lean      two `extends` paths to one parent carrying DIFFERENT parent data, non-defeq
           diamond, `set_option structureDiamondWarning true`
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/context.rs:71 (`instances: Arc<Vec<(Term, Term)>>` — no key, no priority,
           no coherence metadata, and no room for any); kernel/src/context.rs:184-186
           (`add_instance` is an unconditional `push`: it performs no lookup, so it cannot
           notice that an entry with a unifiable conclusion is already present);
           kernel/src/elaborate.rs:1331-1340 (`resolve_instance_at` returns at the FIRST
           candidate that fully resolves — `return Some(result)` — and never enumerates the
           rest, so the SECOND path is never even constructed, let alone compared);
           kernel/src/elaborate.rs:1190-1197 (doc: "The first instance that fully resolves
           wins"); kernel/src/type_checker.rs:1391-1417 (`try_structure_eta` — the only rule
           that could relate two differently-built dictionaries of the same structure, and it is
           reachable only from `def_eq_normalized` at :1473-1478, never from `normalize`, and
           only for structures in the `structures` registry); kernel/src/type_checker.rs:
           1536-1541,1547-1567 (proof irrelevance fires only for `Prop`s — a dictionary carrying
           DATA is not a `Prop`, so two incoherent dictionaries are genuinely unequal)
@diag      E_INSTANCE_DIAMOND_INCOHERENT — must name the class reached by more than one path,
           list the paths, exhibit the two resulting parent dictionaries, and state that LOGOS
           resolution is first-match-wins with no coherence check. Must NOT emit either path.
@guard     THE TRAP, and it is the subtlest one in Tier 7: the translator emits both paths'
           instances, LOGOS resolves the first, and EVERYTHING TYPECHECKS. There is no error, no
           warning, and no artefact recording that a choice was made. The Lean source may have
           been relying on the two paths being definitionally equal — Mathlib maintains exactly
           this invariant by hand across its whole hierarchy — and if the translation breaks it,
           the breakage shows up as a later proof mysteriously failing `def_eq`, arbitrarily far
           from the diamond.
@guard     Catches "LOGOS will catch it because the two dictionaries are not `def_eq`". It will
           not, because the two dictionaries are never COMPARED. Resolution returns at the first
           success (elaborate.rs:1334-1339); the alternative is never built. The only way a
           comparison happens is if the Lean source explicitly writes both paths' parent
           projections into one statement — which is exactly what `bothPaths` below does, and
           which real code does not do.
@guard     Catches relying on structure eta. `try_structure_eta` (type_checker.rs:1391-1417)
           relates `Foo_mk α x` to a variable `f` when `f`'s fields reconstruct to the same
           thing; it does NOT relate `Foo_mk α 0` to `Foo_mk α 1`. Eta makes coherent diamonds
           work; it does nothing for incoherent ones. And it is unavailable under `normalize`
           anyway (gated at :1473-1478), so a proof that reaches the comparison through
           normalization fails even in the COHERENT case.
@guard     Catches translating a diamond that is coherent IN LEAN but whose coherence depends on
           Lean's structure-flattening choice. Lean decides per-declaration which parent becomes
           a subobject and which is flattened (see T07-005: `Qux.mk` there takes `[toBar]` and a
           bare `right`, having flattened `Baz`). The resulting definitional equalities are a
           consequence of that choice. A LOGOS translation that flattens differently — and
           `add_structure` gives it no choice, since there is no subobject notion at all
           (context.rs:271-369) — can turn a coherent Lean diamond into an incoherent LOGOS one
           WITHOUT the Lean source containing any incoherence. So the translator must check
           coherence itself, on the emitted encoding, not merely trust Lean's check.
@guard     Catches emitting only ONE path "to avoid the ambiguity". That silently drops an
           instance the source declared, so a goal that Lean discharged via the dropped path
           fails with `no typeclass instance found` (elaborate.rs:1147-1152) somewhere else
           entirely.
@oracle    Translator exits non-zero with E_INSTANCE_DIAMOND_INCOHERENT, names both paths, and
           emits NO `add_instance` for the incoherent class. A harness additionally asserts
           that for every class the translator DOES emit, no two entries in the emitted instance
           vector have unifiable conclusions with non-`def_eq` values.
-/

namespace T07_013

/-- The shared parent. It carries DATA (a `Nat`), so two different parents are genuinely
    unequal — proof irrelevance does not apply. -/
class Base (α : Type) where
  stamp : Nat

/-- Left path. Its `Base` instance stamps 1. -/
class Left (α : Type) extends Base α where
  leftMark : Nat

/-- Right path. Its `Base` instance stamps 2. -/
class Right (α : Type) extends Base α where
  rightMark : Nat

/-- The join. `Both α` reaches `Base α` through `Left` and through `Right`. -/
class Both (α : Type) extends Left α, Right α where
  joinMark : Nat

/-- THE INCOHERENCE. The two paths carry different `stamp`s. -/
instance leftNat : Left Nat := { stamp := 1, leftMark := 10 }
instance rightNat : Right Nat := { stamp := 2, rightMark := 20 }

/-- The join instance must pick one, and in Lean that choice is explicit and visible in the
    constructor's telescope: `Both.mk : {α} → [toLeft : Left α] → Nat → Nat → Both α`. Lean kept
    `Left` as a subobject and FLATTENED `Right` to its own `rightMark`, so the `Base` reached
    from a `Both` is the LEFT one — by a structural decision the source never states. In LOGOS
    there is no subobject notion (context.rs:271-369) and therefore no telescope to make the
    choice visible in. -/
instance bothNat : Both Nat := { toLeft := leftNat, rightMark := 20, joinMark := 30 }

/-- A function over the PARENT. Which `stamp` does it see? -/
def readStamp (α : Type) [Base α] : Nat := Base.stamp α

/-- Through the left path. -/
def viaLeft : Nat := @readStamp Nat (@Left.toBase Nat leftNat)

/-- Through the right path. -/
def viaRight : Nat := @readStamp Nat (@Right.toBase Nat rightNat)

/-- THE DISCRIMINATING STATEMENT: the two paths disagree, and the disagreement is only visible
    because both projections are written out explicitly. Real code never does this, which is
    why LOGOS's inability to detect the diamond is dangerous rather than merely incomplete. -/
theorem bothPaths : viaLeft ≠ viaRight := by decide

/-- Resolution from the JOIN. Lean picks the `Left` path, because `Both.mk` stores `toLeft`.
    LOGOS picks whichever forget-instance the translator emitted first. -/
def viaBoth : Nat := @readStamp Nat (@Left.toBase Nat (@Both.toLeft Nat bothNat))

example : viaBoth = 1 := rfl

/-- The same goal reached without naming a path — this is what real code writes, and it is
    where LOGOS's first-match-wins silently decides. -/
def implicitPath (α : Type) [Both α] : Nat := readStamp α

example : implicitPath Nat = 1 := rfl

end T07_013
