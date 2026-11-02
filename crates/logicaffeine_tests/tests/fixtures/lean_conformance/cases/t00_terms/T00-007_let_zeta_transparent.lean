/-!
@case      T00-007
@tier      0 · Kernel term layer
@title     `Expr.letE` lowers to `Term::Let`, which is zeta-TRANSPARENT during type checking
@lean      Expr.letE, `let x : A := v; b`
@expect    translate
@target    Term::Let { name: String, ty: Box<Term>, value: Box<Term>, body: Box<Term> }
           — kernel/src/term.rs:475.
@evidence  kernel/src/term.rs:475 (`Term::Let` carries the ANNOTATED type `ty` as well as the
           value — both must be emitted; the type is not optional);
           kernel/src/type_checker.rs:133-142 (`Let` inference: check `ty` is a sort, check
           `value : ty`, then `substitute(body, name, value)` and infer the type of the RESULT —
           i.e. the let-bound name is zeta-expanded away before the body is typed, so the body
           sees `x ≡ v` definitionally, not an opaque hypothesis of type `A`);
           kernel/src/reduction.rs:298 (`reduce_step` on `Let` performs the same zeta
           substitution); kernel/src/type_checker.rs:1088-1110 (`substitute_avoiding` — the
           substitution used, capture-avoiding but O(size) per crossed binder)
@guard     Catches lowering `letE` to a beta-redex `App(Lambda { param, param_type: A, body }, v)`.
           This is the standard "lets are just applications" move and it is WRONG here: the
           `App` rule (type_checker.rs:144-177) types the lambda's body with `x : A` OPAQUE and
           only substitutes into the resulting *type* afterwards. So any body whose type or whose
           later definitional equalities depend on `x`'s VALUE stops typechecking. `Term::Let`
           is not sugar for that redex in LOGOS — it is strictly stronger. (T00-017 is the
           dedicated dependent-body case; this guard is its converse, and it matters even for
           non-dependent bodies because it silently changes which `rfl`s go through.)
@guard     Catches the opposite error: lowering Lean's `have` (`letFun`) to `Term::Let`. Lean's
           `have` is deliberately OPAQUE — the elaborator does not let the goal see the value —
           so translating it to a zeta-transparent `Let` makes LOGOS accept proofs that Lean
           rejects. That is the over-permissive direction, which the kernel will not catch. See
           T00-019.
@guard     Catches dropping the `ty` field or filling it with `Term::Hole`. `type_checker.rs:139`
           calls `infer_sort(ctx, ty)`, but `check_type` accepts anything against a `Hole`
           expected type (type_checker.rs:468-471), so a `Hole` here disables the check that the
           value actually has the annotated type — and the annotation is the only place the
           Lean-side type is recorded. See T00-016.
@guard     Catches assuming `Let` restores SHARING. It does not: `type_checker.rs:141`
           substitutes the value at EVERY occurrence before typing the body, so a value used `n`
           times is checked `n` times, and nested lets multiply. A translator that introduces
           `Let` nodes to compress a large term makes checking exponentially slower, not faster,
           and can push `normalize` past its 10000-step fuel (reduction.rs:52-69). See T00-022.
@oracle    The emitted term is a `Term::Let` (not an `App`/`Lambda` pair); `infer_type` succeeds
           and yields the translation of Lean's inferred type; and `def_eq(Let{x,A,v,b}, b[x:=v])`
           holds — the test that distinguishes `Let` from the beta-redex encoding.
-/

namespace T00_007

/-- The plain case: a let used once. -/
def simpleLet : Nat :=
  let n : Nat := 3
  n + 1

/-- The let-bound name used several times. Zeta-substitution duplicates the value at each
    occurrence during checking (type_checker.rs:141). -/
def repeatedUse : Nat :=
  let n : Nat := 2 + 2
  n + n + n + n

/-- The zeta-transparency test that the beta-redex encoding FAILS: the proof needs `n ≡ 3`,
    which only holds because `Let` substitutes the value before typing the body. -/
def zetaVisible : Fin 4 :=
  let n : Nat := 3
  -- `⟨n, _⟩ : Fin 4` only elaborates because the body sees `n ≡ 3`, and the `Fin` index
  -- proof below is discharged against the VALUE, not against the opaque binder.
  ⟨n, by omega⟩

example : (let n : Nat := 3; n) = 3 := rfl

/-- The same test stated as a propositional equality the body itself must prove by `rfl`. -/
def zetaProof : Nat :=
  let n : Nat := 3
  have _h : n = 3 := rfl
  n

/-- Nested lets where the inner value mentions the outer name — substitution must happen
    outside-in, and the duplication compounds. -/
def nestedLets : Nat :=
  let a : Nat := 2
  let b : Nat := a + a
  let c : Nat := b + b
  c + c

/-- A let whose bound name shadows an enclosing binder. In the named representation the `Let`'s
    `name` is a plain `String`, so the shadowing is real and the substitution at
    type_checker.rs:141 must not reach into the shadowed region. -/
def shadowingLet (n : Nat) : Nat :=
  let n : Nat := 10
  n + 1

/-- A let at the *type* level: the bound name is a type. The annotation `ty` is `Type`. -/
def letOfType : Type :=
  let α : Type := Nat
  α → α

/-- A let whose value is a function, then applied — checks that zeta happens before beta. -/
def letOfFunction : Nat :=
  let f : Nat → Nat := fun k => k * 2
  f (f 3)

end T00_007
