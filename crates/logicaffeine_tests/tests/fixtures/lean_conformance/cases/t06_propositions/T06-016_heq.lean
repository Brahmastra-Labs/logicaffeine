/-!
@case      T06-016
@tier      6 · Propositions, equality, classical reasoning
@title     `HEq` — no heterogeneous equality exists in LOGOS, and `Eq` is not a substitute
@lean      HEq, HEq.refl, heq_of_eq, eq_of_heq, HEq.trans, `h : a ≍ b` across distinct types,
           Eq.rec's generated congruence obligations on indexed families
@expect    reject
@target    (none — no sound translation)
@evidence  a grep of `crates/logicaffeine_kernel/src` for `HEq` or `heq` returns NOTHING — there
           is no heterogeneous equality anywhere in the kernel, the prelude, or the term
           language; kernel/src/term.rs:443-545 (the complete `Term` enum — no variant that
           could carry two carriers); kernel/src/prelude.rs:1422-1439 (`Eq : Π(A : Type 0). A →
           A → Prop` — ONE carrier, shared by both sides, and pinned to `Type 0`);
           kernel/src/prelude.rs:33-44 (`StandardLibrary::register` — the complete list of what
           is registered; no heterogeneous relation appears);
           kernel/src/type_checker.rs:612-678 (`infer_indexed_match` — LOGOS has NO index
           unifier and no impossible-case elimination, which is the machinery Lean's `HEq`
           obligations come from in the first place)
@diag      E_NO_HETEROGENEOUS_EQUALITY — must name `HEq`, report the two DISTINCT carrier types
           of the occurrence, and state that LOGOS's `Eq` takes a single carrier. Must NOT
           suggest `Eq` with a cast, and must NOT suggest declaring `HEq` as a new inductive.
@guard     THE TRAP: emitting `HEq a b` as `Eq _ a b`. This is wrong twice over.
           (1) It is a DIFFERENT PROPOSITION even when the two types coincide: `HEq` carries
               both carriers in its statement, and `eq_of_heq` — the bridge from one to the
               other — is a THEOREM in Lean, not a definitional unfolding.
           (2) It is not even well-typed in general: for `a : α` and `b : β` with `α ≠ β`, there
               is no `A` to instantiate `Eq`'s carrier with. The translator that reaches for
               `Term::Hole` here is in the worst possible position, because `types_equal` treats
               `Hole` as matching ANYTHING (type_checker.rs:1594-1597) and that leniency leaks
               into `is_subtype`'s fast path (:1314). `Eq Hole a b` will therefore be accepted
               against an `Eq` at any carrier at all — a direct route to a proof of `False` by
               transporting along an equation between values of unrelated types.
@guard     Catches picking ONE of the two carriers. `Eq α a (cast h b)` is a real proposition,
           but it is not the one the Lean source stated, and it requires the very `h : α = β`
           that the `HEq` was standing in for. Silently inserting a cast changes the theorem and
           hides the missing hypothesis.
@guard     Catches declaring `HEq` as a fresh inductive to make the shape typecheck. LOGOS's
           `add_inductive` and `add_constructor` are UNCHECKED (context.rs:253,419), so
           `HEq : Π(A:Type 0)(B:Type 0). A → B → Prop` with constructor
           `HEq_refl : Π(A)(a:A). HEq A A a a` WILL register and WILL typecheck. It is then a
           new axiom in everything but name — an inductive the prelude does not know about,
           whose derived recursor (recursor.rs:36-71) has a `Type 0`-pinned motive and NO index
           unifier behind it (type_checker.rs:612-678,632-638), so `eq_of_heq` is not provable
           from it. The result is a trusted-base expansion that also fails to do the job.
@guard     Catches missing the IMPLICIT occurrences. Most real `HEq`s are never typed by the
           author: Lean's equation compiler and `Eq.rec` on indexed families generate them, and
           `cases`/`injection` on a dependent hypothesis produces them. The translator must scan
           the elaborated `Expr`, not the surface syntax — `implicitHEq` below is a `cases`
           whose motive mentions `HEq` even though the source does not.
@guard     Catches treating `HEq α α a b` (both carriers syntactically equal) as safe to
           downgrade. Even there, `eq_of_heq` is a theorem depending on `Eq.rec` at a motive
           LOGOS's derived `Eq_rec_dep` cannot express (its motive target is hard-coded to
           `Type 0`, recursor.rs:64-71). The downgrade is unproven, not merely unidiomatic.
@oracle    Translator exits non-zero with E_NO_HETEROGENEOUS_EQUALITY for every declaration
           below, writes no declaration into the LOGOS context, and in particular emits neither
           an `Eq` application with a `Term::Hole` carrier nor an inductive named `HEq`.
-/

namespace T06_016

/-- The relation itself. Note the TWO carriers in the type. -/
theorem heqRefl {α : Sort u} (a : α) : HEq a a := HEq.refl a

/-- Homogeneous → heterogeneous. A theorem in Lean, not a coercion. -/
theorem toHEq {α : Sort u} {a b : α} (h : a = b) : HEq a b := heq_of_eq h

/-- Heterogeneous → homogeneous, at a KNOWN common carrier. Also a theorem. -/
theorem fromHEq {α : Sort u} {a b : α} (h : HEq a b) : a = b := eq_of_heq h

/-- A genuinely heterogeneous statement: the two sides have syntactically different types that
    are only equal because of a hypothesis. There is no `Eq` carrier to choose here. -/
theorem acrossTypes (n m : Nat) (h : n = m) (v : Fin n) :
    HEq v (Eq.rec (motive := fun k _ => Fin k) v h) := by
  subst h
  rfl

/-- An indexed family where the index differs on the two sides — the canonical `HEq` shape. -/
inductive Vect (α : Type) : Nat → Type where
  | nil : Vect α 0
  | cons : α → {n : Nat} → Vect α n → Vect α (n + 1)

theorem vectHEq (α : Type) (n m : Nat) (h : n = m) (v : Vect α n) :
    HEq v (Eq.rec (motive := fun k _ => Vect α k) v h) := by
  subst h
  rfl

/-- IMPLICIT `HEq`. The source never types the word; `Vect.casesOn`'s motive does. This is the
    occurrence a surface-syntax scan misses. -/
theorem implicitHEq (α : Type) (n : Nat) (v : Vect α (n + 1)) :
    ∃ (a : α) (w : Vect α n), v = Vect.cons a w := by
  cases v with
  | cons a w => exact ⟨a, w, rfl⟩

/-- `HEq` transitivity across three distinct carriers. -/
theorem heqTrans {α β γ : Sort u} {a : α} {b : β} {c : γ}
    (h₁ : HEq a b) (h₂ : HEq b c) : HEq a c := HEq.trans h₁ h₂

end T06_016
