/-!
@case      T00-018
@tier      0 · Kernel term layer
@title     `fun | pat => ..` — pattern lambdas must be re-expressed as a POSITIONAL `Match`
@lean      fun | pat => .., match-expressions, the equation compiler, auxiliary `match_1` defs
@expect    translate
@target    Term::Match { discriminant, motive, cases } — kernel/src/term.rs:469. `cases` is a
           `Vec<Term>` with EXACTLY one entry per constructor, in the inductive's REGISTRATION
           order, each entry a `Term::Lambda` telescope over that constructor's non-parameter
           arguments.
@evidence  kernel/src/term.rs:469 (`Match { discriminant, motive, cases: Vec<Term> }` — there is
           no pattern field, no wildcard, no guard, no fall-through);
           kernel/src/type_checker.rs:254-260 (`cases.len()` must equal the constructor count;
           a mismatch is an error, and the association of case to constructor is by POSITION
           only — there is no tag on a case);
           kernel/src/type_checker.rs:733-760 and :827-844 (each case's expected type is the
           constructor's argument telescope, beta-reduced against the motive — so a case must be
           a `Lambda` chain over exactly those arguments);
           kernel/src/type_checker.rs:271 (the match's result type is
           `beta_reduce(App(motive, discriminant))`);
           LOGOS_FACTS: "`Match` is the PRIMITIVE; recursors are derived terms. This is the
           inverse of Lean 4. There are no patterns, no wildcards, no nested patterns, no
           `casesOn`/`brecOn`/`noConfusion`/`injection`."
@guard     Catches preserving SOURCE ORDER of the alternatives. `cases` is positional
           (type_checker.rs:254-260): case `i` is the case for constructor `i` as registered by
           `add_constructor` (context.rs:419). Lean lets the author write the alternatives in any
           order, and `Col.blue` first is perfectly idiomatic. Emitting the source order
           produces a `Match` with the right ARITY and the wrong SEMANTICS — every branch runs
           for the wrong constructor. When the branches have the same type (all `Nat` below)
           nothing type-checks differently; the function simply computes something else.
           This is the highest-frequency silent mistranslation in the whole `Match` area.
@guard     Catches translating the AUXILIARY definition instead of the match. Lean's equation
           compiler emits `T00_018.pc.match_1` (and `brecOn`/`below` for recursive functions) and
           the function body is an APPLICATION of that auxiliary. A translator working on the
           final `Expr` sees `Expr.const ``pc.match_1` and, finding no LOGOS counterpart, is
           tempted to emit it via `add_declaration` — an AXIOM with the matcher's type, which is
           inhabited and therefore consistent, and which makes every equation of the function
           unprovable. The matcher must be UNFOLDED (`Meta.withTransparency .all` /
           `Split.simpMatch` / `matchMatcherApp?`) back to `casesOn`, and the `casesOn` mapped to
           a `Term::Match`.
@guard     Catches emitting a case as a BARE BODY rather than a `Lambda` telescope. For a nullary
           constructor the two coincide, so an enum-only test suite passes. For `Nat.succ k` the
           case must be `λ(k:Nat). …`; a bare body leaves `k` free and `infer_type` reports
           `UnboundVariable` — or, if a same-named binder happens to be in scope, resolves it to
           the WRONG variable (T00-001).
@guard     Catches including PARAMETERS in the case telescope. `type_checker.rs:756` and :832-844
           bind only the constructor arguments beyond the inductive's parameters; a translator
           that lowers Lean's `casesOn` minor premise verbatim includes the parameters and the
           arity check fails — or worse, for a parameterless inductive it silently does not.
@guard     Catches `n + 1` as a pattern. Lean's `n+1` is sugar for `Nat.succ n` via `OfNat` and
           the equation compiler; it is NOT a numeric guard. A translator that reads the surface
           pattern rather than the elaborated matcher will try to emit an arithmetic test, for
           which LOGOS has no form at all.
@oracle    For each `Match` emitted, `cases.len()` equals `ctx.get_constructors(ind).len()`, and
           case `i` is `def_eq` to the Lean alternative for constructor `i` *as registered*, not
           as written; `infer_type` succeeds; and for each constructor `C` the harness checks
           `def_eq(f (C args…), <the Lean alternative for C> args…)` — the test that catches a
           permuted `cases` vector, which no type check will.
-/

namespace T00_018

/-- A three-constructor enum, matched with the alternatives DELIBERATELY OUT OF ORDER.
    Registration order is `red`, `green`, `blue`; source order below is `blue`, `red`, `green`.
    A `cases` vector built from source order computes `pc red = 2`. -/
inductive Col where
  | red
  | green
  | blue

def pc : Col → Nat := fun
  | .blue => 2
  | .red => 0
  | .green => 1

example : pc Col.red = 0 := rfl
example : pc Col.green = 1 := rfl
example : pc Col.blue = 2 := rfl

/-- A constructor with an ARGUMENT, so the case must be a `Lambda` over it. `n + 1` here is
    `Nat.succ n`, not arithmetic. -/
def predOrOne : Nat → Nat := fun
  | 0 => 1
  | n + 1 => n

example : predOrOne 0 = 1 := rfl
example : predOrOne 7 = 6 := rfl

/-- A PARAMETERISED inductive: the case telescope must bind the constructor's arguments but NOT
    the parameter `α`. -/
inductive Opt (α : Type) where
  | none
  | some : α → Opt α

def unwrapOr : Opt Nat → Nat → Nat := fun
  | .none, d => d
  | .some a, _ => a

example : unwrapOr Opt.none 9 = 9 := rfl
example : unwrapOr (Opt.some 3) 9 = 3 := rfl

/-- A constructor with SEVERAL arguments of different types — the telescope order is part of the
    translation. -/
inductive Tagged where
  | tag : Nat → Bool → Tagged

def readTag : Tagged → Nat := fun
  | .tag n b => if b then n else 0

example : readTag (Tagged.tag 5 true) = 5 := rfl
example : readTag (Tagged.tag 5 false) = 0 := rfl

/-- A pattern lambda in a DEPENDENT position: the motive is not constant. The motive is a real
    `Term` and must be emitted; see T00-016 for what happens if it is holed out. -/
def depMatch : (b : Bool) → (if b then Nat else Bool) := fun
  | true => (3 : Nat)
  | false => false

/-- A `match` expression rather than a pattern lambda — the same elaborated `Expr` shape, so the
    translator must not branch on the surface form. -/
def viaMatchExpr (c : Col) : Nat :=
  match c with
  | Col.red => 0
  | Col.green => 1
  | Col.blue => 2

example : ∀ c, viaMatchExpr c = pc c := fun c => by cases c <;> rfl

end T00_018
