/-!
@case      T00-017
@tier      0 · Kernel term layer
@title     `let` whose BODY'S TYPE depends on the bound value — zeta in the type, not just the term
@lean      Expr.letE with a dependent body, `let n := 3; (v : Fin n)`
@expect    translate
@target    Term::Let { name, ty, value, body } — kernel/src/term.rs:475. The dependency is
           carried by LOGOS's `Let` typing rule, which zeta-substitutes BEFORE inferring the
           body's type; no extra machinery is needed and none may be substituted for it.
@evidence  kernel/src/type_checker.rs:133-142 — the whole rule:
           `let _ = infer_sort(ctx, ty)?;  check_type(ctx, value, ty)?;
            let unfolded = substitute(body, name, value);  infer_type(ctx, &unfolded)`.
           The body is never typed with `name` in the context at all; it is typed AFTER the
           value has been substituted in, so the inferred type mentions the VALUE.
           Contrast kernel/src/type_checker.rs:144-177 (`App`): the function's body type is
           computed with the parameter OPAQUE (`ctx.extend(param, param_type)` at :121 for the
           `Lambda` that produced it) and the argument is substituted into the RESULT type only,
           at :176 — which is strictly weaker.
           kernel/src/reduction.rs:298 (`reduce_step` on `Let` performs the same zeta step, so
           the term and its type agree);
           kernel/src/type_checker.rs:1088-1110 (`substitute_avoiding` — capture-avoiding, and
           O(size) per crossed binder, which is what makes this rule expensive)
@guard     Catches the "a `let` is just a beta-redex" lowering, `App(Lambda { param, param_type:
           A, body }, v)`. For a NON-dependent body the two are interchangeable, so a test suite
           of ordinary `let`s will not distinguish them. For a dependent body the redex fails:
           the `Lambda` rule (type_checker.rs:112-131) types `body` with `param : A` opaque, so
           `Fin n` cannot reduce to `Fin 3`, the `Fin` literal does not check, and the
           declaration is rejected. A translator that hits this and "fixes" it by weakening the
           body's type, inserting a cast, or axiomatising the index bound has converted a
           lowering bug into a permanent loss.
@guard     Catches the SILENT half of the same bug. When the body's type mentions `n` but the
           declaration's DECLARED type also mentions `n`-free spellings only (because Lean
           already zeta-reduced it away in the stated type), the beta-redex encoding produces a
           term whose inferred type is `Π(n : Nat). …` applied — which normalizes to the same
           thing, so it passes. It fails only where the value's identity is needed inside the
           body's own elaboration, which is a strictly smaller set of positions than a test
           suite covers. Both encodings must therefore be compared on the DEPENDENT instances
           below, not on the easy ones.
@guard     Catches zeta-substituting in the term but not in the declared TYPE. A declaration is
           `add_definition(name, ty, body)` (context.rs:442) and the kernel checks `body` against
           `ty`. If the translator pre-zeta-reduces the body (turning `let n := 3; Fin n` into
           `Fin 3`) but copies Lean's stated type verbatim (`let n := 3; Fin n`), the two are
           still `def_eq` — but the emitted type now contains a `Let` that the certificate
           records and that every downstream consumer must re-zeta. Pick one and be consistent.
@guard     Catches the COST. `type_checker.rs:141` substitutes the value at every occurrence,
           and `substitute_avoiding` walks the body computing `all_var_names` at each binder it
           might capture (type_checker.rs:1060-1071). Nested dependent lets — each of whose
           values mentions the previous — expand multiplicatively, and `normalize`'s 10000-step
           fuel (reduction.rs:40) is shared across the whole check. A translator that introduces
           `Let` nodes to make a term *smaller* makes the CHECK bigger. See T00-022.
@oracle    The emitted term is a `Term::Let`; `infer_type` succeeds; the inferred type is
           `def_eq` to the translation of Lean's inferred type; and the harness additionally
           checks that replacing the `Let` with `App(Lambda …)` makes `infer_type` FAIL — the
           test that the dependency is real and that the correct form was chosen for a reason.
-/

namespace T00_017

/-- The minimal dependent case: the `Fin` bound is the let-bound value. Only a zeta-transparent
    `let` types this. -/
def finAtLet : Nat :=
  let n : Nat := 3
  let v : Fin n := ⟨2, by omega⟩
  v.val

/-- The dependency in the RESULT type rather than in an intermediate binding. -/
def resultTypeDependsOnLet : (let n : Nat := 3; Fin n) := ⟨2, by omega⟩

example : (let n : Nat := 3; (⟨2, by omega⟩ : Fin n)).val = 2 := rfl

/-- Two dependent lets in sequence: the second value's TYPE mentions the first's VALUE, so the
    substitutions must compose in order. -/
def chainedDependency : Nat :=
  let n : Nat := 2
  let m : Nat := n + 1
  let v : Fin m := ⟨n, by omega⟩
  v.val

/-- The let-bound name is a TYPE, and the body's type is that type. Zeta must happen at the
    sort level as well as the value level. -/
def letOfTypeDependent : Nat :=
  let α : Type := Nat
  let x : α := 5
  x

/-- A let-bound PROOF used in a dependent position. The proof's identity does not matter
    (proof irrelevance, type_checker.rs:1547-1567), but its TYPE mentions the earlier let. -/
def letOfProof : Nat :=
  let n : Nat := 4
  let h : n = 4 := rfl
  let v : Fin (n + 1) := ⟨4, by omega⟩
  v.val + (match h with | rfl => 0)

/-- The NON-dependent control. Both the `Let` lowering and the beta-redex lowering type this
    one, which is why it must not be the only `let` in the suite. -/
def nonDependentControl : Nat :=
  let n : Nat := 3
  n + n

/-- A let under a binder, so the substitution crosses a `Lambda` and `substitute_avoiding`'s
    freshening logic (type_checker.rs:1060-1110) is actually exercised. -/
def letUnderBinder (k : Nat) : Nat :=
  let n : Nat := 3
  let v : Fin (n + k + 1) := ⟨n, by omega⟩
  v.val

end T00_017
