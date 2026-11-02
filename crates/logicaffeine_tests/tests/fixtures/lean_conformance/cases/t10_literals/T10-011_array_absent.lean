/-!
@case      T10-011
@tier      10 · Literals and primitive types
@title     `Array` — absent, and the `List` encoding changes the complexity *and* the proofs
@lean      Array, #[…] literal, Array.get / getElem, Array.push, Array.size, Array.data,
           Array.foldl, USize indexing
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/prelude.rs:18-51 (`StandardLibrary::register` — no `Array`);
           kernel/src/prelude.rs:609-652 (`register_tlist`: the only parametric sequence in the
           prelude is `TList : Type 0 → Type 0` with `TNil`/`TCons`, a CONS-LIST);
           kernel/src/prelude.rs:564-592 (`register_monolist`: `EList` is a monomorphic list of
           `Entity`, and its own doc comment explains it exists because "the kernel's parametric
           `TList A` carries a type argument, so its constructors are `Π(A:Type)…`" — i.e. even
           the prelude works around its own list); kernel/src/term.rs:13-44 (the `Literal` enum
           has no array/vector variant, so `#[1,2,3]` cannot be injected as a literal);
           kernel/src/reduction.rs:444-506 (`try_primitive_reduce` — no indexing, no
           `get`/`push`/`size` primitive exists); kernel/src/prelude.rs:3468-3477 (`BVec` is the
           only length-indexed container, and it is a `Bit` list — see T10-010)
@diag      E_TYPE_ABSENT_FROM_TARGET(Array) — must name `Array`, state that LOGOS has no array
           type, no array literal, and no indexing primitive, and must NOT silently substitute
           `TList`. If the translator offers a `TList` encoding it must do so as an explicit,
           user-approved rewrite with a loss record, never as a translation.
@guard     Catches the silent `Array ↦ TList` substitution. The two have the same INTERFACE for
           `foldl`/`map`/`size`, so most Lean sources translate without a type error. What
           changes is (a) `Array.get a i` is O(1) and total given a bound proof, while a list
           lookup is O(i) recursion that must be written and must handle the out-of-range case,
           (b) `Array.set` is destructive-with-sharing while a list update rebuilds, and most
           importantly (c) EVERY LEMMA. `Array.size_push`, `Array.get_push_eq`,
           `Array.getElem_set_eq` and friends do not hold of a cons-list by construction; the
           translator must re-prove each one, and if it instead re-declares them with
           `add_declaration` it has enlarged the trusted base with statements about a type the
           prelude does not have (compare T06-009).
@guard     Catches the `GetElem` bound proof being dropped. `a[i]` in Lean is
           `@getElem _ _ _ _ a i h` where `h : i < a.size`. In a `TList` encoding the bound
           proof has no target — LOGOS has no `Nat` order relation at all (the order axioms at
           prelude.rs:202-340 are every one of them about `Int`) — so the tempting move is to
           translate `a[i]` as a total lookup with a default. That is a DIFFERENT FUNCTION, and
           it makes `a[i] = a[j] → i = j`-style reasoning unsound at out-of-range indices while
           still typechecking.
@guard     Catches translating the array literal `#[1,2,3]` as a `Literal`. There is no array
           literal variant (term.rs:13-44); it must become nested `TCons` applications, each
           carrying the `Type 0` parameter explicitly (prelude.rs:637-651 — `TCons`'s first
           argument is the element type). An emitter that omits the type argument, or that
           leaves it as `Term::Hole`, produces a term that `types_equal` accepts because `Hole`
           matches anything (type_checker.rs:1594-1597) — see T11-004.
@guard     Catches `TList`'s `Type 0` pinning. `TList : Type 0 → Type 0` (prelude.rs:613-619),
           so an array of `Prop`s, of types, or of anything in `Type 1` is not encodable — and
           because LOGOS is cumulative (`Prop ≤ Type 0`, type_checker.rs:1330-1334) an
           `Array Prop` may well typecheck as a `TList Prop` anyway, which Lean would reject.
           See T11-001.
@oracle    Translator exits non-zero with E_TYPE_ABSENT_FROM_TARGET(Array) for every
           declaration below and emits no LOGOS declaration. A harness asserts that no
           `Global("TList")` appears in output translated from a Lean `Array`.
-/

namespace T10_011

/-- The array literal. No LOGOS `Literal` variant can carry it. -/
def small : Array Nat := #[1, 2, 3]

/-- `size` and the underlying list — the representation Lean itself exposes. -/
example : small.size = 3 := rfl
example : small.toList = [1, 2, 3] := rfl

/-- Indexing with a bound proof. The proof obligation is the part that has nowhere to go. -/
example : small[0] = 1 := rfl
example : small[2]'(by decide) = 3 := rfl

/-- `push` and the lemma that a cons-list encoding does not satisfy on the nose. -/
example : (small.push 4).size = 4 := rfl
example : (small.push 4)[3] = 4 := rfl

/-- The out-of-range case, where a "total lookup with a default" encoding silently differs. -/
example : small[7]? = none := rfl
example : small.getD 7 99 = 99 := rfl

/-- Folds — the interface that makes the substitution look safe. -/
example : small.foldl (· + ·) 0 = 6 := rfl

/-- An array whose element type is NOT in `Type 0`, which `TList : Type 0 → Type 0`
    (prelude.rs:613-619) cannot hold. -/
def ofTypes : Array Type := #[Nat, Bool]

/-- Realistic entry point: arrays arrive through `Array.mapM`, `for … in`, or a `Std` data
    structure, not by name. -/
def sumAll (xs : Array Nat) : Nat := xs.foldl (· + ·) 0

example : sumAll #[10, 20] = 30 := rfl

end T10_011
