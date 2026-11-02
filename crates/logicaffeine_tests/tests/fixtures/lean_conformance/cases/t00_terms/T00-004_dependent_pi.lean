/-!
@case      T00-004
@tier      0 · Kernel term layer
@title     Dependent function type whose binder is used in the codomain
@lean      Expr.forallE (dependent), ∀ / Π binder
@expect    translate
@target    Term::Pi { param: "n", param_type: Global("Nat"), body_type: <uses Var "n"> }
@evidence  kernel/src/term.rs:463 (Term::Pi has a named `param`); kernel/src/type_checker.rs:108
           (Pi formation computes the result sort with `imax`, matching Lean)
@guard     Catches binder-name erasure. LOGOS's Display renders a Pi whose `param == "_"` as a
           non-dependent arrow `A -> B` (term.rs:585-590). A translator that lowers every
           `forallE` with `param := "_"` — tempting, since Lean's binder names are only hints —
           produces a term that *prints* as an arrow while its body contains `Var("_")`, which
           then either captures a later `_` binder or fails lookup. Lean's de Bruijn body must
           be inspected: emit a fresh name iff `bvar 0` occurs, `"_"` iff it does not.
@guard     Also catches name capture: two nested binders both named `n` are legal in Lean
           (de Bruijn) but in LOGOS's *named* representation the inner shadows the outer, and
           `substitute` alpha-renames only when the replacement's free variables collide
           (type_checker.rs:1088). Nested same-name binders must be freshened at emit time.
@oracle    LOGOS `infer_type` on the emitted term returns a `Sort`; and `is_subtype` against the
           translation of Lean's inferred type succeeds. Additionally the emitted `param` must
           not be `"_"` for this case.
-/

namespace T00_004

/-- Dependent: the codomain mentions the binder, so `param` must be a real name. -/
def dependentArrow : Type := ∀ (n : Nat), Fin n → Fin n

/-- Non-dependent: the codomain does not mention the binder, so `param` may be `"_"`. -/
def plainArrow : Type := Nat → Nat

/-- Nested binders sharing a name. Legal in Lean; requires freshening in a named target. -/
def shadowedBinder : Type := ∀ (n : Nat), ∀ (n : Fin n), Nat

/-- The binder is used only in a *later* binder's type, not in the final codomain. -/
def usedOnlyInLaterBinderType : Type := ∀ (n : Nat) (_v : Fin n), Nat

end T00_004
