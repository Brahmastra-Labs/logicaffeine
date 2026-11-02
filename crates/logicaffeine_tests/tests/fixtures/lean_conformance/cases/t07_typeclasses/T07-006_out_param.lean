/-!
@case      T07-006
@tier      7 · Typeclasses
@title     `outParam` / `semiOutParam` — no output-parameter concept; resolution unifies the
           WHOLE conclusion
@lean      outParam, semiOutParam, HAdd/HMul-style heterogeneous classes, Membership, GetElem
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/elaborate.rs:1016-1027 (`ParamKind` has exactly three variants —
           `Explicit`, `Implicit`, `Instance` — with no per-ARGUMENT annotation of any kind on
           the class's own parameters); kernel/src/context.rs:71,184-186 (an instance is an
           unadorned `(Term, Term)` pair: there is no mask, no arity metadata, no output
           positions); kernel/src/elaborate.rs:1346-1375 (`try_instance` unifies the instance's
           WHOLE conclusion against the goal — `if !unify(ctx, mctx, &conclusion, required)
           { return None }` at :1373 — so every argument of the class, output or not, must
           already be determined on the goal side or be solvable by that single unification);
           kernel/src/elaborate.rs:1016-1027 again (`ParamKind::Instance`'s doc: resolution runs
           "only AFTER the explicit arguments have been processed, so any metavariable in the
           class type … is solved first" — the design assumes the class type is GROUND by then,
           which is precisely what `outParam` denies); kernel/src/elaborate.rs:138 (metavariables
           are `Term::Var` names beginning with `?` and must be fully instantiated before the
           kernel sees the term)
@diag      E_NO_OUT_PARAM — must name the class, list the `outParam`/`semiOutParam` positions,
           and state that LOGOS resolution requires the whole conclusion to unify. Must NOT
           silently demote the `outParam` to an ordinary parameter.
@guard     THE TRAP: dropping the `outParam` wrapper and emitting the class as if all its
           arguments were ordinary. `outParam` is definitionally the identity
           (`outParam : Sort u → Sort u`), so erasing it produces a class that TYPECHECKS
           perfectly. What changes is resolution. In Lean, `HAdd α β γ` with `γ` an `outParam`
           means "resolve on `α` and `β`, and let the instance DETERMINE `γ`". With the wrapper
           erased, LOGOS's `try_instance` unifies the whole conclusion `HAdd Nat Nat ?γ` against
           each candidate — and `?γ` is a metavariable on the GOAL side, so unification will
           bind it to whatever the FIRST vector entry says (elaborate.rs:1331-1340,
           first-match-wins). The elaboration succeeds, silently, with the wrong output type;
           every later use of the result is then checked against a type the Lean source never
           mentioned. Because LOGOS is cumulative (type_checker.rs:1330-1360) the resulting
           terms very often still typecheck.
@guard     Catches the opposite failure — emitting `outParam` as a real global. It would then be
           an opaque `add_declaration` or a transparent `add_definition` wrapping the argument.
           Opaque: `HAdd α β (outParam γ)` never unifies with `HAdd Nat Nat Nat`, so no instance
           ever resolves and every arithmetic expression in the development fails.
           Transparent: `unify` normalizes it away and you are back to the first trap, having
           spent a global.
@guard     Catches "just require the output to be ascribed at every call site". That is not the
           same program. `a + b` in Lean elaborates with `γ` unknown; forcing an ascription
           changes elaboration ORDER, and LOGOS's expected-type pass runs BEFORE instance
           resolution (elaborate.rs:1044-1049), so the ascription would pin `γ` from the
           CONTEXT rather than from the instance — the reverse of what `outParam` specifies.
           Where the two disagree (a heterogeneous instance whose output is not the expected
           type), the translated program means something different.
@guard     Catches `semiOutParam`, which is subtler: it permits resolution to proceed with the
           position unknown but ALSO allows it to be used for matching. LOGOS has neither
           behaviour, and a translator that maps `semiOutParam` onto its `outParam` handling
           has conflated two different resolution orders.
@guard     Catches the scale error. `outParam` is not exotic: `HAdd`, `HSub`, `HMul`, `HDiv`,
           `HMod`, `HPow`, `HAppend`, `Membership`, `GetElem`, `Coe`, `MonadLift` and every
           `Bind`/`Functor` in Lean core use it. Rejecting it correctly rejects almost all
           notation-bearing Lean input — including the `+` in `n + 1`, unless the translator
           special-cases the homogeneous defaults, which is T07-007's problem.
@oracle    Translator exits non-zero with E_NO_OUT_PARAM for every class below that has an
           `outParam` position, emits no `add_instance` for those classes, and in particular
           never emits an instance whose conclusion contains a `?`-prefixed `Term::Var`.
-/

namespace T07_006

/-- A minimal heterogeneous class with the output position marked. This is `HAdd`'s shape. -/
class HPlus (α : Type) (β : Type) (γ : outParam Type) where
  hPlus : α → β → γ

/-- Two instances that differ ONLY in the output position. With `outParam` erased, both
    conclusions unify against `HPlus Nat Nat ?γ` and the first vector entry wins. -/
instance : HPlus Nat Nat Nat := ⟨fun a b => a + b⟩
instance : HPlus Nat Bool Nat := ⟨fun a b => if b then a + 1 else a⟩

/-- The output type is determined by the INSTANCE, not by the context. -/
def sum₁ := HPlus.hPlus (3 : Nat) (4 : Nat)
def sum₂ := HPlus.hPlus (3 : Nat) true

example : sum₁ = 7 := rfl
example : sum₂ = 4 := rfl

/-- A class where two instances share the input positions and differ in the output — the
    discriminating case for the first-match-wins trap. -/
class Decode (α : Type) (β : outParam Type) where
  decode : α → β

instance decodeToNat : Decode Bool Nat := ⟨fun b => if b then 1 else 0⟩

/-- Resolution must pin `β := Nat` FROM the instance. -/
def decoded := Decode.decode true

example : decoded = 1 := rfl

/-- `semiOutParam` — a third resolution order that LOGOS also has no notion of. -/
class Rel (α : Type) (β : semiOutParam Type) where
  related : α → β → Prop

instance : Rel Nat Nat := ⟨fun a b => a = b⟩

/-- `Membership`, the Lean-core class whose second parameter is an `outParam`. Re-declared
    locally so the file compiles against core only. -/
class Mem (γ : Type) (coll : outParam Type) where
  mem : coll → γ → Prop

instance : Mem Nat (List Nat) := ⟨fun l a => a ∈ l⟩

def isIn (a : Nat) (l : List Nat) : Prop := Mem.mem l a

end T07_006
