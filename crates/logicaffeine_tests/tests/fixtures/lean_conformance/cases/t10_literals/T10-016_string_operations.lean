/-!
@case      T10-016
@tier      10 · Literals and primitive types
@title     `String` operations — `++`, `length`, `get`, `append`: LOGOS has literally none
@lean      String.append (++), String.length, String.get, String.push, String.isPrefixOf,
           String.toNat?, ToString
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/prelude.rs:356 (`ctx.add_inductive("Text", Sort(Type 0))` — the ONLY
           mention of `Text` as a type in the whole registration path, and it adds ZERO
           constructors, so `Text` is an opaque carrier with no eliminator);
           kernel/src/prelude.rs:352-430 (`register_primitives` — the complete list of declared
           operations: `add`/`sub`/`mul`/`div`/`mod : Int → Int → Int`,
           `le`/`lt`/`ge`/`gt : Int → Int → Bool`, and the `Duration`/`Date`/`Moment`
           operations. NOT ONE of them mentions `Text`);
           kernel/src/reduction.rs:444-506 (`try_primitive_reduce` dispatches on the nine
           operation names above and nothing else — no string operation can reduce);
           kernel/src/term.rs:19-20 (`Literal::Text(String)` — a literal injection with no
           accompanying elimination, unlike `Literal::Nat` which has the Peano bridge at
           reduction.rs:329-345); kernel/src/prelude.rs:2151-2152 (the only other use of `Text`
           in the kernel is as the payload of the reflection constructor
           `SName : Text → Syntax` — a name tag, not a string library)
@diag      E_OPERATION_ABSENT_FROM_TARGET(String) — must name the specific operation, state that
           LOGOS's `Text` is a zero-constructor opaque type with no declared operations and no
           reduction rules, and must NOT offer to declare the operation as an axiom. The
           diagnostic must distinguish itself from T10-005: the TYPE degrades, the OPERATIONS
           are rejected.
@guard     Catches declaring the operations as axioms. `ctx.add_declaration("Text_append",
           Text → Text → Text)` compiles and every term using it typechecks. But there is no
           reduction rule for it — `try_primitive_reduce` matches nine hard-coded names
           (reduction.rs:455-500) and a new global is not one of them — so `"a" ++ "b"` never
           becomes `"ab"`, `def_eq("a" ++ "b", "ab")` is FALSE, and every `rfl`-proof in the
           source fails at a site far from the axiom. Meanwhile the trusted base has grown:
           `certificate::recheck` rebuilds the context from `StandardLibrary::register` alone
           (certificate.rs:63-75), so the certificate fails with
           `UnboundVariable("Text_append")`, which reads like a name-resolution bug. Compare
           T06-009 and T10-007.
@guard     Catches declaring the operations as DEFINITIONS over an invented character list.
           That at least computes — but only up to `normalize`'s hard 10000-step fuel, after
           which the partially reduced term is returned SILENTLY (reduction.rs:52-69). Since
           the element type must itself be invented (`Char` is absent, T10-006, and so is
           `UInt32`, T10-009), the encoding is three declarations deep before the first
           character, and every one of them goes through the UNCHECKED `add_inductive` /
           `add_constructor` path (context.rs:253, :419). See T11-013.
@guard     Catches the asymmetry with `Nat`. `Literal::Nat` has an eliminator — the Peano
           bridge — so LOGOS can compute on numerals. `Literal::Text` has none. A translator
           that reasons "literals work for `Nat`, so they work for `Text`" will emit terms
           that typecheck and never reduce.
@guard     Catches reaching this case through `ToString`, `repr`, `IO.println`, string
           interpolation `s!"…"`, or `decide`'s error messages. The rejection must fire on the
           elaborated `Expr`; string interpolation in particular expands to a chain of
           `String.append` calls that no longer look like strings in the source.
@guard     Catches the claim that string operations are "not mathematical content". They are:
           a `Decidable` instance driven by `String.decEq`, or a `deriving Repr`, puts a string
           operation inside a proof term. If it is in the term, it must translate or be
           rejected.
@oracle    Translator exits non-zero with E_OPERATION_ABSENT_FROM_TARGET(String) for every
           declaration below, emits no LOGOS output for them, and performs zero
           `add_declaration` calls naming a `Text` operation. A harness asserts that the set of
           `Text`-typed applications in the whole corpus output is empty apart from `Eq`.
-/

namespace T10_016

/-- Concatenation, and the `rfl` that an axiomatised `Text_append` cannot prove. -/
example : "ab" ++ "cd" = "abcd" := rfl

/-- Length — needs the character list `Text` does not have. -/
example : "abc".length = 3 := rfl

/-- Indexing — needs `Char` (T10-006) as well. -/
example : "abc".get 0 = 'a' := rfl

/-- Append is associative and has a unit. These are the lemmas an encoding would have to
    re-prove; an axiomatisation proves neither. Note that in Lean they are inherited from
    `List.append` on `String.data` — the character-list layer LOGOS has no counterpart for. -/
example (s t u : String) : ((s ++ t) ++ u).data = (s ++ (t ++ u)).data := by
  show (s.data ++ t.data) ++ u.data = s.data ++ (t.data ++ u.data)
  exact List.append_assoc _ _ _

example (s : String) : (s ++ "").data = s.data := by
  show s.data ++ [] = s.data
  exact List.append_nil _

/-- Predicates over strings, which is how string operations sneak into proof terms. -/
example : "abc".isPrefixOf "abcdef" = true := by native_decide

/-- Decidable equality on strings — a `Decidable` instance carrying a string operation. -/
example : decide ("a" = "a") = true := by decide

/-- String interpolation, which expands to `String.append` chains that no longer look like
    string operations in the source text. -/
def describe (n : Nat) : String := s!"value is {n}"

/-- `ToString`, the most common accidental entry point. -/
example : toString (3 : Nat) = "3" := rfl

end T10_016
