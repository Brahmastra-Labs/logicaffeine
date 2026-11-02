/-!
@case      T10-015
@tier      10 · Literals and primitive types
@title     `Literal::Nat` is DEFINITIONALLY `Succ^n Zero` — the accelerated numeral bridge
@lean      Nat literal, Nat.zero, Nat.succ, Nat.rec, match on a numeral, OfNat at Nat
@expect    translate
@target    Term::Lit(Literal::Nat(n)), which the kernel treats as constructor-form: in
           `extract_constructor` it peels one `Succ` per step, and in `def_eq_normalized` it is
           interchangeable with the spelled-out Peano term.
@evidence  kernel/src/term.rs:38-44 (`Literal::Nat` is "a compact, accelerated form of the
           unary Peano numeral `Succ^n Zero`. `Nat(n)` is DEFINITIONALLY EQUAL to `Succ`
           applied `n` times to `Zero`… in BOTH kernels");
           kernel/src/reduction.rs:329-345 (`extract_constructor`'s bridge: `Nat(0)` yields
           `Zero`'s registration index with no arguments, `Nat(n)` yields `Succ`'s index with
           the single argument `Lit(Nat(n-1))` — so a `Match` or a derived recursor computes on
           a literal directly); kernel/src/type_checker.rs:1480-1493 (the `def_eq` side:
           two `Nat` literals are equal iff their `BigInt`s are, and a literal versus a
           `Zero`/`Succ`-headed term is compared by peeling one `Succ` at a time via
           `nat_peano_step`, :1434-1440); kernel/src/type_checker.rs:302
           (`Literal::Nat(_) => Ok(Global("Nat"))`); kernel/src/prelude.rs:529-552
           (`register_nat` — the `Zero`/`Succ` registration order the bridge's indices refer to);
           kernel/src/recheck.rs (the second, de Bruijn kernel implements the same bridge, which
           is what "in BOTH kernels" refers to)
@guard     Catches the translator that "helpfully" expands literals to Peano form to be safe.
           It is definitionally correct but destroys the acceleration the bridge exists for:
           `Succ^n Zero` is `n` heap nodes, `substitute` is O(size) per binder comparison
           (type_checker.rs:1088), and `normalize`'s fuel is a hard 10000 steps that returns
           the PARTIALLY reduced term on exhaustion with no error (reduction.rs:52-69). A
           literal in the low thousands therefore makes a `def_eq` check silently answer
           "not equal" when the two sides were equal. See T11-005 and T11-007.
@guard     Catches the mirror error: assuming the bridge means a `Nat` literal is a `Term`
           constructor application that can be *pattern-matched on syntactically*. It is not —
           it is a `Term::Lit`, and any translator-side analysis that walks the term looking
           for `App(Global("Succ"), _)` will not find it. The bridge lives in the KERNEL's
           `extract_constructor` and `def_eq`; a translator that re-implements either must
           implement the bridge too or its own defeq check will disagree with LOGOS's.
@guard     Catches the constructor-INDEX assumption. `extract_constructor` looks the indices up
           by NAME at reduction time — `idx_of("Zero")` and `idx_of("Succ")`
           (reduction.rs:333-344) — against whatever is registered for `"Nat"`. A translator
           that emits its own `Nat` (say because it flattened `Nat` from a namespace, or
           because it declared `Nat` fresh rather than reusing the prelude's) breaks the
           bridge: the names no longer resolve, `idx_of` returns `None`, and every literal
           silently stops being constructor-form. Matches on literals then get stuck and
           `normalize` reports the stuck term as normal.
@guard     Catches the boundary at zero. The bridge clamps: `n ≤ 0` maps to `Zero`
           (reduction.rs:337-341), so a *negative* `Literal::Nat` — which `infer_type` rejects
           (type_checker.rs:297-301) but which can be produced by a translator bug — computes as
           `0` rather than erroring. The translator must never emit one. See T10-002.
@oracle    For each example below, LOGOS `def_eq` holds between the emitted literal term and the
           emitted Peano term, AND the emitted term is a `Term::Lit(Literal::Nat)` — not an
           application spine. A harness additionally asserts that `normalize` of a `Match` on a
           literal takes O(1) `reduce_step` calls per peeled `Succ`, not O(n) up front.
-/

namespace T10_015

/-- The bridge, stated directly. -/
example : (0 : Nat) = Nat.zero := rfl
example : (1 : Nat) = Nat.succ Nat.zero := rfl
example : (3 : Nat) = Nat.succ (Nat.succ (Nat.succ Nat.zero)) := rfl

/-- The bridge in the other direction: a Peano-built term equals a literal. -/
example : Nat.succ (Nat.succ Nat.zero) = 2 := rfl

/-- A literal as a `Match` scrutinee. This works only because `extract_constructor` makes the
    literal constructor-form (reduction.rs:329-345). -/
def pred : Nat → Nat
  | 0 => 0
  | n + 1 => n

example : pred 5 = 4 := rfl
example : pred 0 = 0 := rfl

/-- Peeling several `Succ`s off one literal — the incremental behaviour the bridge gives. -/
def minusThree : Nat → Nat
  | 0 => 0
  | 1 => 0
  | 2 => 0
  | n + 3 => n

example : minusThree 10 = 7 := rfl

/-- A literal driving a recursor. In LOGOS the recursor is a DERIVED term
    (recursor.rs:36) and computes through the same bridge. -/
noncomputable def addThreeViaRec (n : Nat) : Nat :=
  Nat.rec 3 (fun _ ih => Nat.succ ih) n

example : addThreeViaRec 2 = 5 := rfl

/-- A literal large enough that the Peano spelling would exhaust `normalize`'s 10000-step fuel
    (reduction.rs:52-69). The accelerated form must be used. -/
def bigLit : Nat := 50000

example : pred bigLit = 49999 := rfl

/-- Literal-versus-literal equality, decided by `BigInt` comparison
    (type_checker.rs:1485), not by peeling. -/
example : (50000 : Nat) = 50000 := rfl
example : (50000 : Nat) ≠ 50001 := by decide

end T10_015
