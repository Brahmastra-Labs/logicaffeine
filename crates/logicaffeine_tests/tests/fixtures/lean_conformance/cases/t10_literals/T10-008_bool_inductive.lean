/-!
@case      T10-008
@tier      10 · Literals and primitive types
@title     `Bool` — the one primitive type that maps across cleanly, and the two ways to break it
@lean      Bool, Bool.true / Bool.false, cond, Bool.and / Bool.or / Bool.not, decide
@expect    translate
@target    Global("Bool") with constructors Global("true") / Global("false") — the LOGOS prelude's
           own registrations, NOT freshly declared ones
@evidence  kernel/src/prelude.rs:593-604 (`register_bool`: `Bool : Type 0` via `add_inductive`,
           then `add_constructor("true", "Bool", Bool)` and
           `add_constructor("false", "Bool", Bool)` — IN THAT ORDER);
           kernel/src/type_checker.rs:254-260 (`Match` is POSITIONAL: `cases.len()` must equal
           the constructor count and the cases are in registration order);
           kernel/src/prelude.rs:373-381 (`le`/`lt`/`ge`/`gt : Int → Int → Bool` — the
           comparison primitives land in this exact `Bool`);
           kernel/src/reduction.rs:449-450 and :466-476 (`try_primitive_reduce` returns
           `Term::Global("true")`/`Global("false")` literally — the bare constructor names);
           kernel/src/prelude.rs:202-340 (the `Int` order axioms encode `a ≤ b` as
           `Eq Bool (le a b) true`, so this `Bool` and these two constructor names are load-
           bearing for the entire order theory); kernel/src/prelude.rs:761-823
           (`register_decidable`: `Decidable`/`isTrue`/`isFalse`);
           kernel/src/prelude.rs:1195-1219 (`reduceBool : Bool → Bool` and `ofReduceBool`)
@guard     Catches the translator that DECLARES ITS OWN `Bool`. `Bool` is one of the very few
           Lean core types the LOGOS prelude already has, and re-registering it under a
           namespace-flattened name such as `Bool_Bool`, or re-adding constructors
           `Bool_true`/`Bool_false`, produces a type that is not the prelude's. Every
           comparison primitive (`le`, `lt`, `ge`, `gt`) returns the PRELUDE's `true`/`false`
           (reduction.rs:449-450), and every order axiom is stated about them
           (prelude.rs:202-340), so the shadow `Bool` silently severs all arithmetic
           comparison from all arithmetic reasoning. Nothing errors; the proofs just stop
           going through, at sites far from the declaration.
@guard     Catches CONSTRUCTOR ORDER INVERSION. LOGOS `Match` has no patterns: it is a
           positional list of cases in the inductive's registration order
           (type_checker.rs:254-260), and the prelude registers `true` FIRST, `false` second
           (prelude.rs:599-603). Lean's `Bool` also declares `false` first
           (`inductive Bool | false | true`), so the registration orders are OPPOSITE. A
           translator that lowers a Lean `match b with | false => x | true => y` by keeping
           Lean's constructor order emits `Match { cases: [x, y] }`, which LOGOS reads as
           `true => x, false => y`. Both branches typecheck whenever they have the same type —
           which is the common case — so the term is accepted and computes the NEGATION of
           what the source said. This is the most mechanical silent-mistranslation in the
           corpus and it is invisible to the kernel.
@guard     Catches translating `Bool` as `Prop`. LOGOS has both, and `decide`/`Decidable`
           (prelude.rs:761-823) is the bridge, exactly as in Lean. Collapsing them would make
           `if` untranslatable and would route through cumulativity (`Prop ≤ Type 0`,
           type_checker.rs:1330-1334), which Lean does not have — see T11-001.
@guard     Catches losing the `true`/`false` names to flattening. The prelude's constructors are
           keyed by the bare strings `"true"` and `"false"` (prelude.rs:600, :603). A translator
           that flattens `Bool.true` to `"Bool_true"` produces an unbound global; one that
           flattens it to `"true"` is correct here but must then reckon with T11-003, where the
           same flattening collides two distinct Lean declarations.
@oracle    Emitted `Bool` terms use `Global("Bool")`, `Global("true")`, `Global("false")` with no
           new registration; and for every Lean `match` on `Bool` in the file, the emitted
           `Match`'s `cases[0]` is the Lean `true`-branch and `cases[1]` the `false`-branch.
           A harness asserts `normalize(le 3 4)` is `Global("true")` and that this is the same
           global the emitted `Match` dispatches on.
-/

namespace T10_008

-- Lean's declaration order is `false` then `true`, which is the OPPOSITE of the LOGOS
-- prelude's `true` then `false` (prelude.rs:599-603). `Bool.rec`'s minor-premise order makes
-- Lean's order visible: `motive false → motive true → …`.
#check @Bool.rec

/-- A match written in Lean's constructor order. Emitted positionally without reordering, this
    computes the negation of what it says. -/
def toNat (b : Bool) : Nat :=
  match b with
  | false => 0
  | true  => 1

example : toNat true = 1 := rfl
example : toNat false = 0 := rfl

/-- The same function written in the other order. A correct translator emits the SAME LOGOS
    `Match` for both — the case list is keyed on the constructor, not on source order. -/
def toNat' (b : Bool) : Nat :=
  match b with
  | true  => 1
  | false => 0

example : ∀ b, toNat b = toNat' b := by decide

/-- The branches have the same type, so an inverted `Match` still typechecks. This is the
    declaration that makes the order bug silent. -/
def swapIsWellTyped (b : Bool) : Nat := if b then 7 else 7

/-- Boolean operations. In LOGOS these must be emitted as `Match`-bodied definitions over the
    prelude's `Bool`; there is no `Bool` primitive hook. -/
def myAnd (a b : Bool) : Bool := if a then b else false

example : myAnd true false = false := rfl
example : myAnd true true = true := rfl

/-- The `Decidable` bridge from `Prop` to `Bool`, present in both systems. -/
example : decide (2 < 3) = true := rfl

/-- A comparison whose result must be the PRELUDE's `true`, because the `Int` order axioms are
    stated as `Eq Bool (le a b) true` (prelude.rs:202-340). -/
example : ((3 : Int) ≤ 4) := by decide

end T10_008
