/-!
@case      T07-004
@tier      7 · Typeclasses
@title     Recursive / polymorphic instances — supported, and depth-bounded at 64
@lean      `instance {α} [C α] : C (F α)`, instance chains, recursion through a type constructor
@expect    translate
@target    One `add_definition` + one `add_instance` whose `ty` is the instance's FULL type
           including its premises: `Π{α}. Sized α → Sized (Box α)`. `try_instance` peels that
           telescope, replaces each parameter with a fresh metavariable, classifies a parameter
           as a PREMISE iff its head `Global` is in `class_heads`, unifies the conclusion with
           the goal, and then resolves each premise RECURSIVELY at `depth + 1`.
@evidence  kernel/src/elaborate.rs:1190-1198 (doc: "Handles POLYMORPHIC / RECURSIVE instances
           (`instance {A} [Inhabited A] : Inhabited (List A)`): the instance's parameter
           telescope is freshened to metavariables, its conclusion is unified against
           `required` (solving the type parameters), and each PREMISE parameter is then resolved
           RECURSIVELY. The first instance that fully resolves wins; failed trials never pollute
           `mctx` (each runs on a clone, committed only on success)");
           kernel/src/elaborate.rs:1346-1400 (`try_instance` — the telescope peel at :1360-1371,
           the premise test `head_global(&param_type).is_some_and(|h| heads.contains(h))` at
           :1365, the conclusion unification at :1373, the recursive resolve at :1380);
           kernel/src/elaborate.rs:1158-1160 (`MAX_INSTANCE_DEPTH = 64` — "a backstop against a
           pathological instance set (`Inhabited A` from `Inhabited A`) looping forever");
           kernel/src/elaborate.rs:1327-1329 (the depth check `if depth > MAX_INSTANCE_DEPTH {
           return None }` — note it returns `None`, i.e. "no instance", NOT an error);
           kernel/src/elaborate.rs:1174-1189 (`class_heads`, folded over registered instances'
           CONCLUSIONS)
@guard     Catches registering the instance's CONCLUSION as its `ty` and dropping the premises.
           `add_instance(Sized (Box α), …)` with the `[Sized α]` premise elided would make
           `try_instance` see a conclusion with a free `Var("α")` and no telescope to freshen,
           so unification binds nothing and the instance either never matches or matches
           EVERYTHING. The whole premise mechanism lives in the `ty`'s `Pi` binders; there is
           nowhere else to put it.
@guard     Catches the ORDER dependency in `class_heads`. A parameter is classified as a premise
           only if its head is already in `class_heads` (elaborate.rs:1365), and `class_heads`
           is computed from the instances registered SO FAR (elaborate.rs:1178-1189). If a
           recursive instance is registered before any BASE instance of the same class, its
           `[C α]` parameter's head is not yet a known class head at the moment resolution runs
           — in practice `class_heads` is recomputed per call (elaborate.rs:1330) so the order
           of REGISTRATION is what matters, not of use. A translator must emit all of a class's
           instances before any consumer is elaborated, and must not interleave classes.
@guard     Catches the silent depth cutoff. At depth > 64 `resolve_instance_at` returns `None`
           (elaborate.rs:1327-1329), which is indistinguishable from "no such instance". The
           caller then reports `no typeclass instance found for …`
           (elaborate.rs:1147-1152) — a misleading message for a chain that DOES exist but is
           65 links long. `deepChain` below is a 6-deep chain, well inside the bound; a
           translator that monomorphises a class hierarchy (T07-011's tempting escape) can
           easily exceed 64 and will then see this message with no indication that depth was the
           cause.
@guard     Catches assuming the premise's metavariable is solved by unification. It is not:
           `try_instance` binds it with `mctx.solve(name, resolved)` from the RECURSIVE call's
           result (elaborate.rs:1382-1387), deliberately keeping the nested instance
           structured. A translator that instead unifies the premise against the goal type
           loses the nesting and emits a flat dictionary.
@guard     Catches a premise whose head is NOT a class. `try_instance` treats any parameter
           whose head is not in `class_heads` as a type parameter to be solved by the conclusion
           unification (elaborate.rs:1364-1370). So a genuine value premise — an instance taking
           a `Nat` argument — is silently turned into a metavariable that the conclusion must
           pin, and if it cannot, the instance is skipped with no diagnostic.
@oracle    Each recursive instance is registered with its full `Π` type including premises;
           `boxedBoxedNat` resolves to a two-level nesting whose inner instance is the
           `Global` of the base instance, not its unfolded body; `deepChain` resolves; and
           the emitted terms are `?`-free and typecheck.
-/

namespace T07_004

class Sized (α : Type) where
  size : α → Nat

/-- BASE instance. Must be registered before the recursive one so `class_heads` contains
    `Sized` when the recursive instance's premise is classified. -/
instance sizedNat : Sized Nat := ⟨fun n => n⟩

/-- A unary type constructor to recurse through. -/
inductive Box (α : Type) where
  | mk : α → Box α

/-- RECURSIVE instance. Its LOGOS `ty` must be the whole
    `Π(α : Type 0). Sized α → Sized (Box α)`, premise included. -/
instance sizedBox {α : Type} [Sized α] : Sized (Box α) :=
  ⟨fun b => match b with | Box.mk a => Sized.size a + 1⟩

def sizeOf' (α : Type) [Sized α] (a : α) : Nat := Sized.size a

example : sizeOf' (Box Nat) (Box.mk 5) = 6 := rfl

/-- TWO levels of nesting — the recursive instance resolves its own premise recursively. -/
def boxedBoxedNat : Nat := sizeOf' (Box (Box Nat)) (Box.mk (Box.mk 5))

example : boxedBoxedNat = 7 := rfl

/-- A SECOND type constructor, so resolution must pick between two recursive instances by
    unifying the conclusion, not by class head alone. -/
inductive Pair2 (α : Type) where
  | mk : α → α → Pair2 α

instance sizedPair2 {α : Type} [Sized α] : Sized (Pair2 α) :=
  ⟨fun p => match p with | Pair2.mk a b => Sized.size a + Sized.size b⟩

example : sizeOf' (Pair2 Nat) (Pair2.mk 2 3) = 5 := rfl

/-- Mixed nesting: `Box (Pair2 (Box Nat))` exercises three instances in one resolution. -/
example : sizeOf' (Box (Pair2 (Box Nat))) (Box.mk (Pair2.mk (Box.mk 1) (Box.mk 2))) = 6 := rfl

/-- A 6-deep chain — well inside `MAX_INSTANCE_DEPTH = 64`, but the shape that exceeds it once
    a translator monomorphises a hierarchy. -/
def deepChain : Nat :=
  sizeOf' (Box (Box (Box (Box (Box (Box Nat))))))
    (Box.mk (Box.mk (Box.mk (Box.mk (Box.mk (Box.mk 0))))))

example : deepChain = 6 := rfl

/-- A class with TWO premises, resolved independently at `depth + 1`. -/
class Pair2Show (α : Type) where
  render : α → Nat

instance : Pair2Show Nat := ⟨fun n => n⟩
instance {α : Type} [Sized α] [Pair2Show α] : Pair2Show (Box α) :=
  ⟨fun b => match b with | Box.mk a => Sized.size a + Pair2Show.render a⟩

example : Pair2Show.render (Box.mk (3 : Nat)) = 6 := rfl

end T07_004
