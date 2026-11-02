/-!
@case      T11-004
@tier      11 · Adversarial and soundness traps
@title     `Term::Hole` in emitted output — `types_equal` matches it against ANYTHING, so one
           `Hole` disables type checking wherever it sits
@lean      `_` placeholder, metavariable `?m`, implicit argument left unsolved, `sorry`-free
           elaboration that postpones, `@f _ _ x`
@expect    reject
@target    (none — a `Hole` must never reach the kernel; the translator must instantiate every
           placeholder before emitting)
@evidence  kernel/src/type_checker.rs:1592-1597 (`types_equal`: "Hole matches anything (it's a
           type wildcard)" — `if matches!(a, Term::Hole) || matches!(b, Term::Hole) { return
           true; }`, BEFORE any structural comparison);
           kernel/src/type_checker.rs:1310-1315 (`is_subtype`'s FAST PATH is
           `if types_equal(a, b) { return true; }` — so the `Hole` leniency leaks straight into
           subtyping, ahead of normalisation and ahead of `is_subtype_normalized`);
           kernel/src/type_checker.rs:505-513 (`check_type` ends in `is_subtype`, so every
           checking position inherits it); kernel/src/type_checker.rs:454-470 (`check_type`'s
           own two `Hole` short-circuits: a `Hole` TERM is accepted whenever the expected type
           is any `Sort`, and a `Hole` EXPECTED TYPE accepts any well-typed term);
           kernel/src/type_checker.rs:311-313 (`infer_type` on a bare `Hole` is
           `Err(KernelError::CannotInferHole)` — the ONLY place a `Hole` is refused, and it is
           reachable only when the `Hole` is in HEAD position, never when it is an argument or
           a binder type); kernel/src/type_checker.rs:1103-1104 (`substitute_avoiding`:
           "Holes are never substituted", so a `Hole` survives every reduction);
           kernel/src/elaborate.rs:138 (elaborator metavariables are `Term::Var` names starting
           with `?`, NOT `Term::Hole` — so the two placeholder mechanisms are different and a
           translator that conflates them emits the wrong one)
@diag      E_HOLE_IN_OUTPUT — must name the declaration and the position of the `Hole`, state
           that `types_equal` matches a `Hole` against anything so the kernel's acceptance of
           the term is meaningless, and emit nothing.
@guard     **TIER 11 THESIS, IN ITS PUREST FORM.** LOGOS's kernel accepting a term is NOT
           evidence that the translation is faithful — and a single `Term::Hole` is the proof,
           because it turns the kernel's verdict into an unconditional "yes" at that position.
           `types_equal` returns `true` before looking at either side (:1594-1597), and
           `is_subtype` calls it FIRST, before normalising (:1314). So `is_subtype(Hole, T)`
           and `is_subtype(T, Hole)` hold for every `T` whatsoever — including terms that are
           not types, including `False`, including a type from a different inductive family.
           A translator that emits a `Hole` for an implicit argument it could not resolve has
           not produced an under-specified term that the kernel will complete; it has produced
           a term whose typecheck is a no-op at that position.
@guard     Catches "the `Hole` is only in a TYPE argument, which is erased anyway". It is not
           erased — LOGOS has no erasure — and the type argument is exactly where it does the
           most damage: `Eq Hole a b` typechecks for ANY `a` and `b`, of any two different
           types, because `check_type(a, Hole)` succeeds for any well-typed `a`
           (type_checker.rs:466-470). That is heterogeneous equality by accident, in a kernel
           that deliberately has no `HEq` (T06-016).
@guard     Catches the `Hole` that arrives by "helpful" repair. The realistic path is: a term
           fails to typecheck because of a universe mismatch (T11-001) or a missing structural
           type (T10-013); the translator patches the offending position with a `Hole` to get
           past it; the build goes green. Two bugs compose into a passing build, and the second
           one hides the first.
@guard     Catches the `Hole` that SURVIVES normalisation. `substitute_avoiding` explicitly does
           not substitute into a `Hole` (type_checker.rs:1103-1104), and `normalize`'s base
           case returns it unchanged (reduction.rs:302). So a `Hole` emitted once stays in the
           term forever, and every later `def_eq` against that subterm is vacuously true —
           including inside a certificate that is then re-checked by `recheck.rs` and passes.
@guard     Catches conflating `Term::Hole` with the elaborator's metavariables. LOGOS's
           elaborator represents an unsolved metavariable as a `Term::Var` whose name begins
           with `?` (elaborate.rs:138), NOT as `Term::Hole`, and such a name must be fully
           instantiated before the kernel sees the term. A `?`-named `Var` that reaches the
           kernel is an UNBOUND VARIABLE and errors loudly; a `Hole` that reaches the kernel is
           accepted silently. The failure modes are opposite, so the choice of placeholder
           decides whether the bug is caught or hidden.
@oracle    Translator exits non-zero with E_HOLE_IN_OUTPUT for any declaration it cannot fully
           instantiate, and the decisive harness assertion is structural, not behavioural: the
           emitted term tree for EVERY case in the whole corpus must contain zero `Term::Hole`
           nodes. Because the kernel cannot detect a `Hole`, this check must be performed by
           the translator's own serialiser, on the term it is about to hand over.
-/

namespace T11_004

/-- An implicit argument that Lean resolves by unification. The translator must emit the
    RESOLVED value; a `Hole` in its place typechecks and means nothing. -/
def idImp {α : Type} (a : α) : α := a

example : idImp 3 = 3 := rfl

/-- `@`-form, showing the argument the translator must supply. -/
example : @idImp Nat 3 = 3 := rfl

/-- An underscore Lean solves from the expected type. The elaborated `Expr` has no hole left;
    a translator working from `Syntax` rather than the elaborated term would. -/
def explicitUnderscore : Nat := @idImp _ 5

/-- The dangerous shape: an equation whose TYPE argument is what pins the two sides together.
    `Eq Hole a b` typechecks for any `a` and `b` (type_checker.rs:466-470), so a `Hole` here
    turns `Eq` into heterogeneous equality — which LOGOS deliberately does not have. -/
example : @Eq Nat 3 3 := rfl

/-- Two terms of DIFFERENT types. In Lean this cannot be stated as an `Eq` at all; with a
    `Hole` as the type argument, LOGOS would accept it. -/
def natSide : Nat := 3
def boolSide : Bool := true

/-
  example : @Eq _ natSide boolSide := rfl

  error: application type mismatch
    natSide = boolSide
  argument
    boolSide
  has type
    Bool : Type
  but is expected to have type
    Nat : Type

  Lean refuses because `_` is a metavariable that must be SOLVED, and no solution exists.
  LOGOS's `Term::Hole` is not a metavariable — it is a wildcard that `types_equal` matches
  against anything (type_checker.rs:1594-1597) — so the analogous emitted term is accepted.
-/

/-- The compiling neighbour: the two sides must genuinely have the same type. -/
example : @Eq Nat natSide 3 := rfl

/-- Instance-implicit arguments, the other common source of an unresolved position. -/
def useDecEq {α : Type} [DecidableEq α] (a b : α) : Bool := decide (a = b)

example : useDecEq 3 3 = true := rfl

/-- A dependent implicit whose value is determined only by the RESULT type — the case a
    translator is most likely to give up on and fill with a `Hole`. -/
def replicate {α : Type} (n : Nat) (a : α) : List α :=
  match n with
  | 0 => []
  | m + 1 => a :: replicate m a

example : (replicate 2 true).length = 2 := rfl

/-- Nested implicits, several positions at once. -/
def compose {α β γ : Type} (g : β → γ) (f : α → β) : α → γ := fun a => g (f a)

example : compose Nat.succ Nat.succ 1 = 3 := rfl

/-- A universe-level implicit. LOGOS's `Term::Const { levels }` is the only universe-
    instantiating form, and it takes a `Vec<Universe>` — there is no `Hole` for a LEVEL, so a
    translator that reaches for one has to put it in the TERM instead, where it is silently
    accepted. -/
def idPoly {α : Sort u} (a : α) : α := a

example : idPoly True = True := rfl

end T11_004
