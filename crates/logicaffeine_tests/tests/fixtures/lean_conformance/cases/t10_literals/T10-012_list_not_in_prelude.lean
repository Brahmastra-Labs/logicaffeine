/-!
@case      T10-012
@tier      10 · Literals and primitive types
@title     `List` — Lean's `List` is not in the LOGOS prelude, and the nearest neighbour
           `TList` is pinned to `Type 0`
@lean      List, [], ::, [1,2,3] literal, List.append, List.map, List.length, List.rec
@expect    degrade
@target    A translator-emitted inductive. The nearest prelude neighbour is
           `TList : Type 0 → Type 0` with `TNil : Π(A:Type 0). TList A` and
           `TCons : Π(A:Type 0). A → TList A → TList A`, plus a `derive_recursor` result the
           caller must `add_definition`.
@evidence  kernel/src/prelude.rs:609-652 (`register_tlist` — the exact shape above, and note
           `add_inductive("TList", Π(A:Type 0). Type 0)` at :619, so the parameter is a
           `Type 0` parameter, not a level-polymorphic one);
           kernel/src/prelude.rs:564-592 (`register_monolist`'s doc comment: `EList` exists
           because `TList`'s `Π(A:Type)…` constructors do not fit the generic
           `InductionScheme`, and the `E`-prefixed names "deliberately avoid the user-space
           `List`/`Nil`/`Cons` a REPL program defines (typically a parametric `List A`)" —
           i.e. the prelude EXPECTS the user to declare their own `List`);
           kernel/src/recursor.rs:36 and :64-71 (`derive_recursor` synthesises `(type, term)`
           and the caller must `add_definition` it; the motive target is HARD-CODED to
           `Sort(Type 0)` for a non-`Prop` inductive — so the derived `List` recursor cannot
           eliminate into `Type 1`, and cannot be universe-polymorphic);
           kernel/src/type_checker.rs:254-260 (`Match` is positional over the constructor
           registration order); kernel/src/context.rs:253 (`add_inductive` is UNCHECKED)
@loss      LIST_IS_REDECLARED. The emitted `List` is a fresh LOGOS inductive that happens to
           have the same shape as Lean's. It carries none of Lean's ~900 `List` lemmas, none of
           its `@[simp]` set, and no `DecidableEq`; every fact the source relies on must be
           re-translated from its Lean proof or the declaration must be rejected. "The types
           line up" is not evidence that a `List` theorem survives.
@loss      LIST_IS_MONOMORPHISED_AT_TYPE_0. Lean's `List : Type u → Type u` is universe-
           polymorphic; LOGOS inductives carry a single fixed sort (context.rs:253, :374) and
           `derive_recursor` pins the motive to `Type 0` (recursor.rs:64-71). So the emitted
           list is `Type 0 → Type 0`. `List Prop` survives only via LOGOS's cumulativity
           (`Prop ≤ Type 0`, type_checker.rs:1330-1334), which Lean does not have; `List Type`
           and `List (List α)` at two different levels do not survive at all. This is the
           `reject` of T01-007 showing through a `degrade` case: the loss record must name it,
           and if the source instantiates the list at two distinct levels the case escalates
           to T01-007's rejection.
@guard     Catches the constructor-order flip. Lean declares `List` as `nil` then `cons`; a
           translator that registers `Cons` before `Nil` produces a `Match` whose positional
           cases (type_checker.rs:254-260) are swapped. Both branches of a `List` match
           frequently have the same type — `length`, `sum`, `any` all do — so the term
           typechecks and computes the wrong answer with no diagnostic. Compare T10-008.
@guard     Catches forgetting that `TNil`/`TCons` take the element TYPE as an explicit leading
           argument (prelude.rs:637-651). Lean's `[]` and `a :: as` have the type implicit, so
           the elaborated `Expr` has it as an implicit argument that must be turned into an
           EXPLICIT first argument, or the binder kind recorded with
           `set_binder_kinds(name, [ParamKind::Implicit, …])` (context.rs:201). Omitting it and
           leaving a `Term::Hole` in its place typechecks, because `types_equal` treats `Hole`
           as matching anything (type_checker.rs:1594-1597). See T11-004.
@guard     Catches reusing the prelude's `TList` as if it were Lean's `List`. It is a different
           global with different constructor names; any Lean lemma name-mapped onto it is a
           claim about a type the source never mentioned. And `EList` is worse: it is
           MONOMORPHIC in `Entity` (prelude.rs:564-592), so `EList` is `List Entity` and
           nothing else.
@guard     Catches emitting the inductive but not the recursor. `derive_recursor` does not
           register anything — "the caller must `add_definition` it" (recursor.rs:36). A
           translator that emits `List` and its constructors and then translates `List.rec`
           as an unbound `Global("List_rec")` produces a term whose `infer_type` fails with
           `UnboundVariable`, which reads like a name-resolution bug rather than a missing
           elimination principle.
@oracle    The emitted declaration set contains exactly one inductive with two constructors in
           `nil`-then-`cons` order plus an `add_definition`-registered recursor; `infer_type`
           accepts every term below; and the loss record contains LIST_IS_REDECLARED and
           LIST_IS_MONOMORPHISED_AT_TYPE_0. A harness asserts the emitted `Match` case order
           matches the registration order.
-/

namespace T10_012

/-- The literal, which is `List.cons 1 (List.cons 2 (List.cons 3 List.nil))`. -/
def small : List Nat := [1, 2, 3]

/-- Structural recursion — the shape that becomes `Fix` + a positional `Match`. Both branches
    have type `Nat`, which is what makes an inverted case order silent. -/
def length' : List Nat → Nat
  | [] => 0
  | _ :: rest => 1 + length' rest

example : length' small = 3 := rfl

/-- Append, whose LOGOS translation needs the element type as an explicit argument. -/
def append' {α : Type} : List α → List α → List α
  | [], ys => ys
  | x :: xs, ys => x :: append' xs ys

example : append' [1, 2] [3] = [1, 2, 3] := rfl

/-- `List.rec` used directly. `derive_recursor` must be called AND its result registered
    (recursor.rs:36); it is not created by declaring the inductive. -/
noncomputable def sumViaRec (xs : List Nat) : Nat :=
  List.rec 0 (fun x _ ih => x + ih) xs

example : sumViaRec small = 6 := rfl

/-- A list at `Type 1`. `TList : Type 0 → Type 0` (prelude.rs:613-619) cannot hold it, and a
    freshly emitted list carries a single fixed sort too. -/
def listOfTypes : List Type := [Nat, Bool]

/-- Two instantiations at two different universes in one file — the condition under which the
    `degrade` of this case must escalate to the `reject` of T01-007. -/
def nested : List (List Nat) := [[1], [2, 3]]
def nestedAtTypeOne : List (List Type) := [[Nat]]

/-- A list of propositions. Lean allows it because `List` is polymorphic over `Sort`'s `Type`
    levels; a `Type 0`-pinned LOGOS list admits it only by CUMULATIVITY (`Prop ≤ Type 0`,
    type_checker.rs:1330-1334), which Lean does not have. -/
def listOfProps : List Prop := [True, False]

end T10_012
