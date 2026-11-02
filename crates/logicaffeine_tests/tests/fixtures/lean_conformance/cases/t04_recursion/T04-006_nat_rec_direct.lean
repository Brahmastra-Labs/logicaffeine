/-!
@case      T04-006
@tier      4 · Recursion and pattern matching
@title     `Nat.rec` applied directly — the recursor is primitive in Lean, derived in LOGOS
@lean      Nat.rec, Nat.casesOn, @Nat.rec with an explicit motive
@expect    degrade
@target    Either (a) `Global("Nat_rec")` after the translator has itself called
           `recursor::derive_recursor(ctx, "Nat")` and `Context::add_definition`ed the
           `(type, term)` pair, or (b) the `Fix` + `Match` the recursor would unfold to.
@evidence  kernel/src/recursor.rs:36-48 (`derive_recursor` SYNTHESISES `(recursor_type,
           recursor_term)`; it does not register anything — "the caller must add_definition it");
           kernel/src/recursor.rs:60-72 (the motive target is HARD-CODED: `Sort(Prop)` for a
           non-subsingleton `Prop` inductive, `Sort(Type 0)` for everything else);
           kernel/src/term.rs:487-492 (`Term::Match` is the primitive — the inverse of Lean 4,
           where `.rec` is primitive and `match` is compiled away);
           kernel/src/context.rs:449 (`get_global` precedence: inductives → constructors →
           definitions → declarations; a derived recursor is an ordinary *definition*, hence
           delta-transparent)
@loss      RECURSOR_PRIMITIVITY. In Lean, `Nat.rec` is a kernel constant with an iota rule. In
           LOGOS it is an ordinary, delta-transparent definition whose body is a `Fix` over a
           `Match`. Consequences the translator must record: (a) `Nat_rec` unfolds during
           `normalize`, so terms that mention it do not stay in recursor form and proof
           scripts keyed on its shape break; (b) it participates in the 10000-step
           `normalize` fuel (reduction.rs:52-69) like any other definition; (c) it is not
           available unless the translator explicitly derived and registered it.
@loss      MOTIVE_UNIVERSE_PINNING. `derive_recursor` gives `Nat_rec`'s motive the target
           `Sort(Type 0)` (recursor.rs:64-71). Lean's `Nat.rec` is universe-polymorphic:
           `{motive : Nat → Sort u}`. Any Lean use of `Nat.rec` at `Sort u` for `u ≥ 2` —
           in particular building a `Type 1`-valued family by recursion — has no translation.
           A `Prop` motive still passes by LOGOS cumulativity (type_checker.rs:1330-1360), so
           only the *large* end of the range is lost.
@guard     Catches emitting `Global("Nat_rec")` without having registered it. `get_global`
           returns `None` and the term fails to typecheck — but only at the point of USE, which
           may be many declarations later, so the error blames the wrong source file.
@guard     Catches the opposite: registering `Nat_rec` as an `add_declaration` AXIOM
           (context.rs:434) to make the name resolve. That makes the recursor opaque, so
           `Nat_rec P z s (Succ k)` never reduces to `s k (Nat_rec P z s k)` — every `rfl`
           proof in the source that relied on the iota rule silently stops holding, and a
           translator that then "fixes" the proofs with more axioms has axiomatised
           arithmetic.
@guard     Catches translating `Nat.rec` at a `Sort u` motive by instantiating `u := 0`
           (i.e. `Prop`) to make it fit. The resulting term typechecks by cumulativity but
           computes nothing: a `Prop`-motive recursion cannot produce the `Type`-level data
           the Lean source built.
@guard     Catches assuming `Nat.casesOn`, `Nat.recAux`, `Nat.noConfusion`, `Nat.brecOn` are
           equally derivable. `derive_recursor` produces ONE eliminator; `casesOn`/`noConfusion`
           /`brecOn` have no LOGOS generator at all (LOGOS_FACTS: "no `casesOn`/`brecOn`/
           `noConfusion`/`injection`"). See T03-018 and T04-016.
@oracle    The emitted term typechecks, and the loss record contains RECURSOR_PRIMITIVITY; if
           any Lean occurrence instantiates `Nat.rec`'s motive above `Type 0`, the loss record
           must additionally contain MOTIVE_UNIVERSE_PINNING (or the case must be rejected).
-/

namespace T04_006

/-- `Nat.rec` used directly with a constant `Type 0` motive — the case that translates. -/
def addRec (m : Nat) (n : Nat) : Nat :=
  Nat.rec (motive := fun _ => Nat) m (fun _ ih => ih + 1) n

/-- A *dependent* motive, still landing in `Type 0`. -/
def repeatNat (n : Nat) : Nat :=
  Nat.rec (motive := fun _ => Nat) 0 (fun k ih => k + ih) n

/-- A `Prop` motive — the induction principle. In LOGOS the derived recursor's motive target
    is `Type 0`, and a `Prop` motive passes only because LOGOS is *cumulative*. Lean, which is
    not cumulative, needs the polymorphism instead. -/
theorem addRecZero (m : Nat) : addRec m 0 = m :=
  Nat.rec (motive := fun _ => addRec m 0 = m) rfl (fun _ _ => rfl) 0

/-- The motive lands in `Type 1`: a family of *types* built by recursion. `derive_recursor`
    hard-codes the motive target to `Sort (Type 0)`, so this instantiation is outside what a
    LOGOS-derived `Nat_rec` can express. -/
def tower : Nat → Type 1
  | 0 => Type
  | n + 1 => tower n

def towerRec (n : Nat) : Type 1 :=
  Nat.rec (motive := fun _ => Type 1) Type (fun _ ih => ih) n

/-- `casesOn` — a *different* auto-generated constant in Lean with no LOGOS generator. -/
def isZero (n : Nat) : Bool :=
  Nat.casesOn (motive := fun _ => Bool) n true (fun _ => false)

example : addRec 3 2 = 5 := rfl
example : isZero 0 = true := rfl

end T04_006
