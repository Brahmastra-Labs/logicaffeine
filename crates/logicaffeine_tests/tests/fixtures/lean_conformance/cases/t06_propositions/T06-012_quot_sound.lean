/-!
@case      T06-012
@tier      6 · Propositions, equality, classical reasoning
@title     `Quot.sound` — present in the LOGOS prelude, but the whole `Quot` family is pinned
           to `Type 0` and there is no `Quotient`/`Setoid` wrapper
@lean      Quot, Quot.mk, Quot.lift, Quot.sound, Quot.ind, Quot.liftOn
@expect    translate
@target    `Quot r`          ⟼ `App(App(Global("Quot"), <α>), <r>)`
           `Quot.mk r a`     ⟼ `App(App(App(Global("Quot_mk"), <α>), <r>), <a>)`
           `Quot.lift f h q` ⟼ `Quot_lift <α> <r> <β> <f> <h> <q>`
           `Quot.sound h`    ⟼ `Quot_sound <α> <r> <a> <b> <h>`
           `Quot.ind`        ⟼ `Quot_ind <α> <r> <P> <mk-case> <q>`
           All five are `add_declaration`s — i.e. AXIOMS, with `Quot A r` OPAQUE, not an
           inductive. The only computation is the `Quot_lift … (Quot_mk … a) ⤳ f a` rule wired
           into `reduce_step`.
@evidence  kernel/src/prelude.rs:1230-1338 (`register_quot`: `Quot`, `Quot_mk`, `Quot_lift`,
           `Quot_sound`, `Quot_ind` all via `ctx.add_declaration`, and every carrier binder is
           `Term::Sort(Universe::Type(0))` — `type0()` at :1235); kernel/src/prelude.rs:1222-1229
           (doc: "`Quot A r` is OPAQUE (not an inductive), so it cannot be pattern-matched —
           only `Quot_lift`/`Quot_ind` eliminate it"); kernel/src/reduction.rs:117-120
           (`try_quot_lift_reduce` implements `Quot_lift A r B f h (Quot_mk A r a) ⤳ f a` inside
           `reduce_step`); kernel/src/prelude.rs:1320-1338 (`Quot_ind`'s motive is
           `Quot A r → Prop` — restricted to `Prop`, so there is no large elimination);
           a grep of `crates/logicaffeine_kernel/src` for `Setoid` or `Quotient` returns NOTHING
@guard     Catches emitting the Lean names. LOGOS's globals are `Quot_mk`, `Quot_lift`,
           `Quot_sound`, `Quot_ind` — flat underscored `String`s (context.rs:449), not
           `Quot.mk`. Emitting `Global("Quot.mk")` resolves to nothing.
@guard     Catches routing `Quotient`/`Setoid` through `Quot`. Lean's `Quotient s` is
           `Quot s.r` with the relation taken from a `Setoid` INSTANCE; LOGOS has neither the
           wrapper nor the class, and its instance database is a flat `Vec<(Term,Term)>` with
           first-match-wins (context.rs:71,184, elaborate.rs:1331-1340). A translator that
           silently unfolds `Quotient s` to `Quot s.r` must ALSO emit the `Setoid` structure and
           its instance, and must record that `Quotient.mk`'s instance argument was resolved at
           translation time and is no longer resolvable at check time.
@guard     Catches attempting to `Match` on a quotient. `Quot` is an `add_declaration`, not an
           inductive, so `ctx.is_inductive("Quot")` is false and `infer_match` (type_checker.rs:
           254-260) cannot find a constructor list. A translator that lowers Lean's
           `Quot.lift`-based recursors into `Term::Match` produces a term the kernel rejects —
           which is the good outcome; the BAD outcome is a translator that first declares `Quot`
           as an inductive with a single `Quot_mk` constructor to make the `Match` work, because
           that makes `Quot_sound` inconsistent: `Quot_mk a` and `Quot_mk b` would then be
           distinguishable by `Match` while `Quot_sound` asserts they are equal.
@guard     Catches the universe pinning. Every binder in `register_quot` is `Type 0`
           (prelude.rs:1235,1248-1338) while Lean's `Quot` is `{α : Sort u}` with
           `Quot.lift` landing in `Sort v`. A quotient of a `Type 1` carrier, or a lift into
           `Prop` via the `Type 0` slot, has no translation — `bigQuot` below must be escalated.
@guard     Catches forgetting that `Quot_lift`'s computation rule is the ONLY reduction. There
           is no eta for quotients and no `Quot.liftOn`; `Quot_ind`'s motive is `Prop`-only, so
           a Lean `Quot.rec` into `Type` (which Lean permits) has no LOGOS counterpart.
@oracle    Emitted terms typecheck; the five globals are spelled with underscores; the
           `Quot_lift ∘ Quot_mk` computation below reduces under `normalize`; and `bigQuot` and
           `recIntoType` are escalated.
-/

namespace T06_012

/-- A concrete relation on `Nat` to quotient by. -/
def rel (a b : Nat) : Prop := a % 2 = b % 2

/-- The quotient type itself. -/
def Parity : Type := Quot rel

/-- Class formation. -/
def mk (n : Nat) : Parity := Quot.mk rel n

/-- `Quot.sound` — the axiom that makes this a genuine quotient. Present in LOGOS. -/
theorem soundness (a b : Nat) (h : rel a b) : mk a = mk b := Quot.sound h

/-- A concrete instance of soundness: 0 and 2 have the same parity. -/
theorem zeroTwo : mk 0 = mk 2 := Quot.sound (show rel 0 2 from rfl)

/-- Lifting a relation-respecting function out of the quotient. -/
def parityBit (q : Parity) : Nat :=
  Quot.lift (fun n => n % 2) (fun _ _ h => h) q

/-- The COMPUTATION rule: `Quot_lift … (Quot_mk … a) ⤳ f a`, wired into `reduce_step`. -/
theorem liftComputes (n : Nat) : parityBit (mk n) = n % 2 := rfl

/-- `Quot.ind` at a `Prop` motive — the only motive LOGOS's `Quot_ind` allows. -/
theorem indProp (P : Parity → Prop) (h : ∀ n, P (mk n)) : ∀ q, P q :=
  fun q => Quot.ind h q

/-- BOUNDARY. Lean's `Quot.rec` eliminates into `Sort v`, including `Type`. LOGOS's `Quot_ind`
    motive is `Quot A r → Prop` (prelude.rs:1320-1338), so this has no translation. -/
def recIntoType (q : Parity) : Unit :=
  Quot.rec (motive := fun _ => Unit) (fun _ => ()) (fun _ _ _ => rfl) q

/-- BOUNDARY. The carrier is `Type`, whose own type is `Type 1`; LOGOS's `Quot` binds
    `A : Type 0`. No translation. -/
def bigQuot : Type 1 := Quot (fun a b : Type => a = b)

end T06_012
