/-!
@case      T06-013
@tier      6 · Propositions, equality, classical reasoning
@title     `Decidable` and `decide` — present in the prelude, with only two instances
@lean      Decidable, Decidable.isTrue/isFalse, decide, of_decide_eq_true, DecidableEq Nat/Bool
@expect    translate
@target    `Decidable p`   ⟼ `App(Global("Decidable"), <p>)` — an INDEXED inductive registered
                              with `num_params = 1` at sort `Π(p:Prop). Type 0`
           `isTrue h`      ⟼ `App(App(Global("isTrue"), <p>), <h>)`
           `isFalse h`     ⟼ `App(App(Global("isFalse"), <p>), <h>)`
           `decide p`      ⟼ `App(App(Global("decide"), <p>), <inst>)` — a DEFINITION whose body
                              is `Decidable_rec p (λ_. Bool) (λ_. true) (λ_. false) inst`
           `of_decide_eq_true` ⟼ `Global("of_decide_eq_true")` — a proven DEFINITION, not an axiom
           `instDecidableEqNat`  ⟼ `Global("decEqNat")`,  `instDecidableEqBool` ⟼ `Global("decEqBool")`
@evidence  kernel/src/prelude.rs:757-818 (`register_decidable`:
           `ctx.add_indexed_inductive("Decidable", Π(p:Prop). Type 0, 1)`, constructors `isTrue`
           and `isFalse`, then `derive_recursor(ctx, "Decidable")` installed as the DEFINITION
           `Decidable_rec`, then `decide` as a definition built from it);
           kernel/src/prelude.rs:820-922 (`of_decide_eq_true` is "PROVEN from `Decidable_rec`
           and Bool no-confusion (via `Eq_rec_dep`), NOT axiomatized");
           kernel/src/prelude.rs:923-1026 (`decEqBool`, registered as a definition AND as an
           `add_instance` so `decide` can discharge `Eq Bool _ _` goals);
           kernel/src/prelude.rs:1027-1194 (`decEqNat`, by structural recursion on the first
           argument); kernel/src/context.rs:184 (`add_instance` — a flat `Vec<(Term,Term)>`,
           first match wins)
@guard     Catches assuming a `DecidableEq` instance exists for anything else. The LOGOS prelude
           registers EXACTLY TWO decision procedures — `decEqBool` (prelude.rs:926) and
           `decEqNat` (prelude.rs:1027). There is no `Decidable` for `Int` comparisons (the
           `le`/`lt` primitives at prelude.rs:202-340 are `Int → Int → Bool` axioms with ORDER
           AXIOMS, not decision procedures returning `Decidable`), none for `List`, none derived
           from `deriving DecidableEq`. `decideCustom` below is the trap: Lean synthesises the
           instance for a user enum automatically, so the Lean source shows nothing unusual, and
           a translator that maps the instance by name silently emits `Global("instDecidableEqColour")`
           which resolves to nothing (context.rs:449).
@guard     Catches emitting `Decidable` with the wrong `num_params`. It is registered via
           `add_indexed_inductive(…, 1)` (prelude.rs:781): `p` is the single PARAMETER and there
           are no indices. A translator using `add_inductive` (context.rs:253, "full arity
           treated as parameters") happens to agree here — but only by accident, and the same
           reflex applied to `Eq` (2 params, 1 index) is wrong. Pin the parameter count
           explicitly; do not rely on the default.
@guard     Catches emitting `decide`/`of_decide_eq_true` as AXIOMS. Both are proven definitions
           in the prelude (prelude.rs:796-818, :824-922), deliberately kept out of the trusted
           base. Axiomatising them is the T06-009 trusted-base-expansion bug in miniature: the
           bridge from a computed `Bool` back to a `Prop` is exactly the place where an unsound
           shortcut is invisible, because the computed side always evaluates to `true` in the
           cases anyone tests.
@guard     Catches confusing `decide` with `reduceBool`. `decide` reduces by ordinary kernel
           normalization; `reduceBool` hands the term to the CBV evaluator
           (reduction.rs:104-115) and is reachable only through the `ofReduceBool` AXIOM. They
           have different trusted bases — see T06-014.
@guard     Catches `Decidable`'s sort. It is `Type 0`-valued (prelude.rs:781), deliberately, so
           it can be eliminated into `Type` to compute a `Bool`. A translator that places it in
           `Prop` (because Lean writes `Decidable p` next to propositions) makes `decide`
           unwritable: `Decidable` would then be a two-constructor `Prop`, hence not a
           subsingleton (type_checker.rs:1688-1716), hence its derived recursor's motive would
           be pinned to `Sort(Prop)` (recursor.rs:64-71) and could not produce a `Bool`.
@oracle    Emitted terms typecheck; `decide`/`of_decide_eq_true` are referenced, never
           re-declared and never emitted via `add_declaration`; the two prelude instances are
           reached by `add_instance` lookup rather than by name; and `decideCustom` is escalated
           or accompanied by an emitted decision procedure plus its `add_instance`.
-/

namespace T06_013

/-- Decidable equality on `Nat` — the prelude's `decEqNat`. -/
theorem natDecide : ((3 : Nat) = 3) := of_decide_eq_true rfl

/-- `decide` computing to a `Bool`. -/
example : decide ((2 : Nat) = 2) = true := rfl
example : decide ((2 : Nat) = 3) = false := rfl

/-- Decidable equality on `Bool` — the prelude's `decEqBool`. -/
example : decide (true = true) = true := rfl

/-- The two constructors, reached explicitly. -/
def yes : Decidable True := isTrue True.intro
def no  : Decidable False := isFalse (fun h => h)

/-- `Decidable` eliminated into `Type` — the reason it must be `Type 0`-valued. -/
def choose (p : Prop) [inst : Decidable p] (a b : Nat) : Nat :=
  match inst with
  | isTrue _  => a
  | isFalse _ => b

/-- The `Bool`/`Prop` bridge in the sound direction. -/
theorem bridge (p : Prop) [inst : Decidable p] (h : decide p = true) : p :=
  of_decide_eq_true h

/-- `if then else` driven by a `Decidable` instance — the common surface form. -/
def cmp (a b : Nat) : Nat := if a = b then 0 else 1

example : cmp 3 3 = 0 := rfl
example : cmp 3 4 = 1 := rfl

/-- BOUNDARY. Lean synthesises this instance from the `deriving` clause; LOGOS has no such
    instance and no `deriving` machinery. The translator must emit the decision procedure
    itself plus an `add_instance`, or escalate. -/
inductive Colour where
  | red | green | blue
  deriving DecidableEq

def decideCustom : Bool := decide (Colour.red = Colour.blue)

example : decideCustom = false := rfl

end T06_013
