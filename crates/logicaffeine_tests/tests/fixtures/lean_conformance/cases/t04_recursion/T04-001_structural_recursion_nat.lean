/-!
@case      T04-001
@tier      4 · Recursion and pattern matching
@title     Structural recursion on `Nat` — Lean's equation compiler vs. LOGOS `Fix` + `Match`
@lean      def by structural recursion on Nat, `| 0 => .. | n+1 => ..` equations
@expect    translate
@target    Term::Fix { name: "double", body: Lambda{param:"n", param_type: Global("Nat"),
             body: Match { discriminant: Var("n"), motive: <λ_:Nat. Nat>,
                           cases: [<Zero case>, <Succ case: λk. Succ (Succ (double k))>] } } }
@evidence  kernel/src/term.rs:497-509 (`Term::Fix { name, body }` — the only recursion form);
           kernel/src/term.rs:487-492 (`Term::Match { discriminant, motive, cases }` — `cases`
           is a positional `Vec<Term>`, no patterns);
           kernel/src/type_checker.rs:255-259 (`cases.len()` must equal the constructor count);
           kernel/src/type_checker.rs:336 (`Term::Fix` inference calls
           `termination::check_termination` before anything else);
           kernel/src/termination.rs:120-162 (`extract_structural_param` picks the binder the
           body's `match` discriminates on, else the FIRST inductive-typed binder);
           kernel/src/termination.rs:403-440 (`check_match_cases_guarded` marks a case's
           constructor-bound λ-binders as structurally smaller);
           kernel/src/recursor.rs:36 (`derive_recursor` is a *derived* term — `Match` is the
           primitive, the inverse of Lean 4)
@guard     Catches emitting the cases in source order rather than CONSTRUCTOR REGISTRATION
           order. `Term::Match` carries no constructor tags at all (term.rs:487-492) and
           `type_checker.rs:262-264` zips `cases` against `ctx.get_constructors(..)` by index.
           Lean lets the user write `| n+1 => .. | 0 => ..`; a translator that preserves source
           order emits `cases = [succCase, zeroCase]`. For `Nat` both cases often have the same
           *type* after the motive is applied (`Nat` here), so the kernel accepts it and
           `double 2` silently computes garbage. There is no error to catch this — only the
           obligation to re-sort by the LOGOS registration order of `Zero`/`Succ`.
@guard     Catches translating the recursion through a *recursor* because Lean's `Expr` shows
           `Nat.brecOn`/`Nat.rec`. LOGOS has no recursor primitive: `Nat_rec` only exists if
           somebody calls `derive_recursor` (recursor.rs:36) and `add_definition`s the result,
           and that derived recursor's motive target is hard-coded (recursor.rs:64-71), so it
           cannot eliminate into `Type 1+`. The faithful target is `Fix` + `Match`.
@guard     Catches binder reordering. `extract_structural_param` (termination.rs:120-162)
           records the ARGUMENT POSITION of the decreasing argument and
           `termination.rs:218-232` then checks the argument at *that* position of every
           recursive call. A translator that hoists Lean's implicit binders, or that emits the
           λ-telescope in a different order than the call spine, makes the guard inspect an
           unrelated argument — which can both reject an honest function and accept a call that
           does not decrease.
@guard     Catches scrutinising a `Nat` *literal*. `reduction.rs:329-346` bridges
           `Literal::Nat(n)` to `Succ^n Zero` by the Zero/Succ *registration positions*, so a
           mis-ordered `cases` vector is wrong for literal scrutinees too, not just for
           constructor-form ones.
@oracle    The emitted `Fix` typechecks (so `check_termination` passed), and LOGOS `normalize`
           of the emitted term applied to `Lit(Nat 3)` equals the translation of Lean's
           `double 3`, i.e. `Lit(Nat 6)` up to the Peano bridge.
-/

namespace T04_001

/-- The canonical structural recursion. Lean compiles this to `Nat.brecOn`; LOGOS must
    receive `Fix` + `Match`. -/
def double : Nat → Nat
  | 0 => 0
  | n + 1 => double n + 2

/-- Equations written in the *reverse* of constructor order. Lean does not care; a positional
    `Term::Match` does. -/
def half2 : Nat → Nat
  | n + 1 => half2 n + 1
  | 0 => 0

/-- The decreasing argument is NOT the first binder: `m` is a plain accumulator and the
    recursion runs on `n`, which is at argument position 1. -/
def addAcc (m : Nat) : Nat → Nat
  | 0 => m
  | n + 1 => (addAcc m n) + 1

/-- Recursion whose motive is not constant: the result type mentions nothing, but the
    accumulator threads through, so the emitted `Fix`'s λ-telescope has two binders and the
    structural position must be recorded as 1, not 0. -/
def sumTo : Nat → Nat → Nat
  | acc, 0 => acc
  | acc, n + 1 => sumTo (acc + n + 1) n

example : double 3 = 6 := rfl
example : half2 4 = 4 := rfl
example : addAcc 5 3 = 8 := rfl
example : sumTo 0 4 = 10 := rfl

end T04_001
