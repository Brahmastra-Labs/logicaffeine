/-!
@case      T05-016
@tier      5 · Elaboration surface
@title     `..` ellipsis — sugar for "fill the rest with `_`", expanded before translation
@lean      `{ x := 1, .. }` structure-instance ellipsis, `| .mk .. =>` and `| { x := v, .. } =>`
           pattern ellipsis, `f ..` trailing application ellipsis
@expect    translate
@target    The fully-expanded, fully-SOLVED form. `..` is Lean's sugar for supplying `_` in
           every remaining position, so the emitted term is whatever those holes were solved
           to: a full-arity constructor application, a case λ-abstracting every field, or a
           full application spine. The emitted term must contain **no `Term::Hole`**.
@evidence  kernel/src/context.rs:271-369 (`add_structure` generates ONE `{name}_mk` constructor
           of full field arity — there is no partial-constructor form);
           kernel/src/type_checker.rs:255-259 (`cases.len()` must equal the constructor count)
           and :778-830 (`compute_case_type`: a case's expected type is the constructor's
           REMAINING Π-telescope, so a case must bind every field);
           kernel/src/term.rs:487-492 (`Term::Match` has no pattern language at all, hence no
           ellipsis) and :478 (`App` is positional and binary);
           kernel/src/term.rs:540-544 and kernel/src/type_checker.rs:313 (`Term::Hole` is the
           only "unknown" and it is `CannotInferHole` at `infer_type`) and :1593-1597
           (`types_equal` treats `Hole` as matching **anything**);
           kernel/src/termination.rs:403-415 (`count_pi_params` / `extract_lambda_params`: the
           guard peels exactly as many λs from a case as the constructor has Π binders)
@note      INDEX.md lists this case as `translate`, which is right, but the reason is narrower
           than "`..` fills in defaults". In Lean 4.14 `..` does **not** apply a field's
           `:=` default — it substitutes `_`, and the elaboration then succeeds only if
           unification solves it. `{ val := 1, .. }` on a structure whose `tag` merely has a
           default is an ERROR ("don't know how to synthesize placeholder"), whereas
           `{ val := 1 }` succeeds by defaulting. Default-value inlining is therefore T05-006,
           not this case; `..` belongs with the holes of T05-013.
@guard     **Translate the post-elaboration `Expr`, never the surface `Syntax`.** All three
           ellipses are pure surface: Lean expands each to `_`s and then solves them. The
           `Expr` has no trace of them. A translator on `Syntax` must reimplement the expansion
           AND the unification that fills it — and since the fill is by unification, not by
           defaults, the answer depends on the whole surrounding elaboration problem.
@guard     Catches emitting `Term::Hole` for an ellipsis-supplied position. `types_equal`
           matches a `Hole` against **anything** (type_checker.rs:1593-1597) and the leniency
           leaks into `is_subtype`'s fast path (:1314), so an ellipsis that the translator did
           not resolve produces a declaration that typechecks against any type. Since `..` is
           exactly "I did not write this out", it is the most likely place for a translator to
           leave one.
@guard     Catches emitting a SHORT constructor application for a structure-instance ellipsis.
           `add_structure`'s `{name}_mk` takes every field (context.rs:271-369), so a
           two-argument spine against a three-field constructor is a partial application of
           type `Nat → Three`, not a `Three`. Where the omitted field's type makes that partial
           application well-typed in context, it typechecks and yields a function where a value
           was meant.
@guard     Catches emitting an under-bound CASE for `| .mk .. =>`. The case must λ-abstract
           every field. `compute_case_type` (type_checker.rs:778-830) catches the arity slip
           when the body's type differs — but `termination.rs:410-415` peels
           `count_pi_params` λs regardless, so under a `Fix` an under-bound case makes the
           guard peel into the case BODY and mark a non-field variable as structurally
           smaller. That admits a non-terminating fixpoint: a soundness hole, not a type error.
@guard     Catches `..` inside a NESTED pattern, where the ellipsis belongs to the inner
           constructor. Each nesting level of the emitted match tree needs its own full binder
           list (see T04-008), and an ellipsis at the outer level says nothing about the inner.
@guard     Catches confusing `..` with structure UPDATE (`{ s with x := 1 }`). Update reads the
           remaining fields off `s` via its projections, so the emitted term mentions `s` three
           times; `..` does not mention any receiver at all. Emitting one for the other changes
           which term the fields come from.
@guard     Catches a trailing-only parse assumption. `..` is legal only in trailing position
           (`@idf .. 5` is a parse error), so a translator that normalises an `Expr` back to
           surface text cannot re-introduce `..` wherever a `_` appears.
@oracle    The emitted term contains no `Term::Hole`; the emitted constructor application has
           the structure's full field arity; every emitted match case λ-abstracts exactly
           `count_pi_params(ctor_type)` binders; `infer_type` succeeds; and the emitted term is
           term-for-term identical to the translation of the same expression written without
           any `..`.
-/

namespace T05_016

structure Three where
  x : Nat
  y : Nat
  z : Nat

/-- A trailing `..` with nothing left to fill — a no-op that must produce the same term as the
    ellipsis-free literal. -/
def t0 : Three := { x := 1, y := 2, z := 3, .. }

example : t0 = Three.mk 1 2 3 := rfl

/-- Pattern ellipsis: the case must still bind all three fields. -/
def anyThree : Three → Nat
  | .mk .. => 0

/-- Partial pattern ellipsis: `x` is named, the rest are elided. -/
def getX : Three → Nat
  | { x := v, .. } => v

example : getX ⟨1, 2, 3⟩ = 1 := rfl
example : anyThree ⟨1, 2, 3⟩ = 0 := rfl

/-- Ellipsis in a NESTED pattern — the inner constructor's binder list must be complete too. -/
structure Outer where
  hd : Nat
  inner : Three

def innerX : Outer → Nat
  | { inner := { x := v, .. }, .. } => v

example : innerX ⟨9, ⟨1, 2, 3⟩⟩ = 1 := rfl

/-- Trailing application ellipsis whose holes ARE solvable — here from the expected type. -/
def emptyNats : List Nat := @List.nil ..

example : emptyNats = [] := rfl

/-- Pattern ellipsis under RECURSION: every replicated case body must bind the full field
    list, or the Giménez guard peels into the body. -/
def sumOuters : List Outer → Nat
  | [] => 0
  | { inner := { x := v, .. }, .. } :: t => v + sumOuters t

example : sumOuters [⟨0, ⟨1, 2, 3⟩⟩, ⟨0, ⟨4, 5, 6⟩⟩] = 5 := rfl

/-- Structure UPDATE, which is a different mechanism: the remaining fields come from `t`'s
    projections, so the emitted term mentions `t`. -/
def bumpX (t : Three) : Three := { t with x := t.x + 1 }

example : (bumpX ⟨1, 2, 3⟩).x = 2 := rfl
example : (bumpX ⟨1, 2, 3⟩).z = 3 := rfl

/-
  THE FORM LEAN REFUSES, given as a comment because the file must compile — and the reason
  this case's `@note` exists:

      structure Wrap where
        val : Nat
        tag : Nat := 0

      def w : Wrap := { val := 1, .. }
      -- error: don't know how to synthesize placeholder  ⊢ Nat

  `..` substitutes `_`, it does not apply `tag`'s default. `{ val := 1 }` — with no `..` —
  is the form that uses the default, and that is T05-006.
-/

end T05_016
