/-!
@case      T06-015
@tier      6 · Propositions, equality, classical reasoning
@title     `Subsingleton` / `Squash` — the CLASS is absent; the KERNEL's subsingleton rule is a
           different, syntactic thing
@lean      Subsingleton, Subsingleton.elim, Subsingleton.intro, Squash, Squash.lift, Nonempty
@expect    degrade
@target    `Subsingleton α` ⟼ a NEW structure-or-inductive emitted by the translator plus an
           `add_instance` per instance — there is nothing to map onto. The natural encoding is
           `add_structure("Subsingleton", &[("a", Type 0)], &[("elim", Π(x y:a). Eq a x y)])`,
           which yields the inductive, the constructor `Subsingleton_mk`, and the projection
           definition `Subsingleton_elim`.
           `Squash α` ⟼ `App(App(Global("Quot"), <α>), λ_ _. True)` — the quotient by the total
           relation, using the prelude's `Quot` family (all of it pinned to `Type 0`).
           Plus a loss record: LOGOS's OWN subsingleton notion is not this class.
@evidence  a grep of `crates/logicaffeine_kernel/src` for `Subsingleton`, `Squash` or `Nonempty`
           returns NOTHING — none of the three is in the prelude;
           kernel/src/type_checker.rs:1684-1716 (`is_subsingleton_prop` — the kernel's own
           notion: a `Prop` inductive may be large-eliminated iff it has ZERO constructors, or
           exactly one whose non-arity arguments all live in `Prop`. This is a SYNTACTIC
           property of a declaration, computed from its constructor list, with no relation to
           an instance of a class); kernel/src/recursor.rs:60-71 (`derive_recursor` consults it
           to pick the motive target: `Sort(Prop)` for a non-subsingleton `Prop` inductive,
           `Sort(Type 0)` otherwise); kernel/src/prelude.rs:1230-1338 (`Quot` family, all
           binders `Type 0`, `Quot_ind`'s motive restricted to `Prop`);
           kernel/src/context.rs:271-369 (`add_structure`, sort hard-coded to `Type 0` at :300)
@loss      SUBSINGLETON_IS_NOT_A_CLASS. Lean's `Subsingleton α` is a `Prop`-valued CLASS with
           instances, inferred by typeclass resolution, and `Subsingleton.elim` is its method.
           LOGOS's `is_subsingleton_prop` is a compile-time predicate on an INDUCTIVE
           DECLARATION, evaluated by the kernel, and it is not addressable from a term at all:
           there is no way to write "this type is a subsingleton" as a hypothesis. The two agree
           on which things they call subsingletons only for the special case of single- or
           zero-constructor `Prop` inductives; they disagree on `Unit`, on `Fin 1`, on any
           `Subsingleton` instance a user declares, and on every `Subsingleton` instance that
           Mathlib derives.
@loss      LARGE_ELIMINATION_GATE. Lean's large-elimination criterion and LOGOS's are close but
           not identical, and where they differ LOGOS is the MORE permissive one for
           `Type 0` targets because of cumulativity (`Prop ≤ Type 0`,
           type_checker.rs:1330-1360) — a `Prop` motive "still passes there by cumulativity",
           recursor.rs:60-63. A translator must record which eliminations it relied on; it
           cannot assume Lean's gate transfers. See T01-014 and T11-020.
@loss      SQUASH_UNIVERSE. Lean's `Squash : Type u → Type u`. The LOGOS encoding via `Quot`
           pins the carrier to `Type 0` (prelude.rs:1235,1248), so `Squash` of anything larger
           has no translation.
@guard     Catches the translator "implementing" `Subsingleton.elim` by appealing to the
           kernel's proof irrelevance. That is only valid when `α` is a `Prop`. For
           `Subsingleton Unit` — a `Type` — irrelevance does not apply, and
           `proof_irrelevant` (type_checker.rs:1547-1567) returns `false` because
           `infer_type` of `Unit`'s type is `Type 0`, not `Sort(Prop)`. The translator must emit
           a real proof, not lean on the kernel rule.
@guard     Catches encoding `Squash α` as `Nonempty α`. `Nonempty` is a `Prop` and `Squash` is
           a `Type`; the difference is exactly whether you may eliminate into `Type` — and
           doing so from the `Prop` version is choice (T06-009). Neither exists in LOGOS, so the
           translator must not substitute one for the other on the grounds that both are
           "truncation".
@guard     Catches emitting `Subsingleton` as a `Prop`-valued structure. `add_structure`
           hard-codes the sort to `Term::Sort(Universe::Type(0))` (context.rs:300), so the
           emitted class is a `Type 0` inductive whatever the Lean source said. That is a
           universe change, not just a bookkeeping one: a `Type 0`-valued `Subsingleton α`
           appearing in a `Prop`'s statement changes that statement's sort through `imax`
           (type_checker.rs:108), silently promoting a `Prop` to a `Type 0`.
@oracle    The emitted `Subsingleton` structure and its instances typecheck; `Squash` is emitted
           over the prelude's `Quot`; a loss record naming SUBSINGLETON_IS_NOT_A_CLASS is
           produced for every declaration below; and `bigSquash` is escalated.
-/

namespace T06_015

/-- Lean's class, re-stated locally so the file compiles against core only and so the shape the
    translator must emit is explicit. -/
class MySubsingleton (α : Type) : Prop where
  elim : ∀ (a b : α), a = b

/-- The canonical `Type`-level instance. Proof irrelevance does NOT prove this in LOGOS. -/
instance : MySubsingleton Unit := ⟨fun a b => by cases a; cases b; rfl⟩

/-- A second instance, so first-match-wins ordering in the LOGOS instance `Vec` matters. -/
instance : MySubsingleton (Fin 1) := ⟨fun a b => by
  cases a; cases b; simp_all
  omega⟩

/-- Using the class method — this becomes the projection definition `MySubsingleton_elim`. -/
theorem useElim (a b : Unit) : a = b := MySubsingleton.elim a b

/-- Lean's real `Subsingleton`, for comparison, over a `Prop` — the one case where LOGOS's
    kernel rule and the class agree. -/
theorem propSubsingleton (p : Prop) (h₁ h₂ : p) : h₁ = h₂ := Subsingleton.elim h₁ h₂

/-- `Squash` — propositional truncation that still lives in `Type`. LOGOS must encode it as a
    quotient by the total relation. -/
def squashed : Squash Nat := Squash.mk 3

/-- Squash is a subsingleton: any two elements are equal. -/
theorem squashEq (a b : Squash Nat) : a = b := Subsingleton.elim a b

/-- Lifting out of a `Squash` — only into a subsingleton target. -/
def squashLift (q : Squash Nat) : Squash Bool := Quot.lift (fun _ => Squash.mk true) (fun _ _ _ => Subsingleton.elim _ _) q

/-- BOUNDARY. `Squash` at a `Type 1` carrier. LOGOS's `Quot` binds `A : Type 0`. -/
def bigSquash : Type 1 := Squash Type

end T06_015
