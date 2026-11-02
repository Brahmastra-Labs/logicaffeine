/-!
@case      T05-018
@tier      5 · Elaboration surface
@title     Type ascription and `show … from` — erased by elaboration, but they CHANGE the term
@lean      (e : T), show T from e, show T by tac, `(e : T)` driving coercion and `OfNat`
           resolution
@expect    translate
@target    Nothing for the ascription itself — it leaves no node. The target is whatever the
           ascription made Lean elaborate: a different `OfNat` instance, an inserted coercion,
           a differently-solved implicit. `(5 : Int)` is `Term::Lit(Literal::Int 5)`, not a
           `Nat` literal wrapped in anything.
@evidence  kernel/src/term.rs:443-545 (the `Term` enum has **no ascription variant** and no
           `mdata` — there is nowhere for an ascription to live);
           kernel/src/type_checker.rs:294-311 (a `Term::Lit`'s type is its KIND, so an
           ascription that changed a literal's type must have changed the literal itself);
           kernel/src/elaborate.rs:1046-1080 (`elaborate_app_against` — the expected type is a
           first-class input to elaboration, and it runs BEFORE instance resolution: this is
           the LOGOS-side analogue of what an ascription supplies);
           kernel/src/elaborate.rs:1301-1330 (`resolve_coercion` fires when an argument's type
           does not match the expected one — an ascription is one way to create that
           mismatch);
           kernel/src/type_checker.rs:456-470 (`check_type`, the "check against an expected
           type" direction, is a separate entry point from `infer_type`)
@guard     **Translate the post-elaboration `Expr`, never the surface `Syntax`.** An ascription
           is not a no-op that can be stripped: it is an INPUT to elaboration, and removing it
           from the surface text can change the resulting `Expr` (a different `OfNat`
           instance, an inserted or omitted coercion, a differently-defaulted numeral). A
           translator on `Syntax` that "erases ascriptions and translates the rest" therefore
           translates a different program. A translator on `Expr` sees only the result, which
           is exactly right.
@guard     Catches emitting an ascription as an identity application. Lean's `(e : T)` for a
           `Type`-valued `T` elaborates to just `e`; a translator that emits
           `App(Global("id"), e)` adds a delta step that counts against `normalize`'s 10000
           fuel (reduction.rs:52-69) and, more importantly, changes the term's SHAPE — so any
           later pattern-match on the emitted term (a proof script, a `simp` set) no longer
           fires.
@guard     Catches emitting `show T from e` as anything other than `e`. In Lean `show T from e`
           is `(e : T)` and `show T by tac` is a tactic block; the first leaves no trace, the
           second leaves the tactic's proof term. A translator that treats them uniformly
           either loses a proof or invents one.
@guard     Catches the interaction with LOGOS's CUMULATIVITY. `is_subtype_normalized` admits
           `Prop ≤ Type 0 ≤ Type 1` (type_checker.rs:1330-1360) where Lean admits none. An
           ascription that Lean used to force a *specific* universe — `(α : Type 1)` on
           something that would otherwise be `Type 0` — is erased, and the LOGOS term then
           typechecks at the smaller universe too. The ascription's constraint is simply gone,
           so a translator that relies on "it still typechecks" to validate its output has no
           signal here. See T01-009 and T11-001.
@guard     Catches the `Prop`/`Type 0` off-by-one. LOGOS says `Prop : Type 1`
           (`Sort(Prop).succ() == Type(1)`, term.rs:198) where Lean says `Prop : Type 0`. An
           ascription `(p : Prop)` is erased in both, but an ascription mentioning the SORT of
           `Prop` translates to a different universe. See T01-010.
@guard     Catches an ascription that drives COERCION insertion. `(b : Nat)` with `b : Bool`
           inserts `boolToNat b` (T05-011); the emitted term must contain that application.
           A translator that strips the ascription and emits the bare `b` produces a `Bool`
           where a `Nat` was meant.
@guard     Catches an ascription inside a `match` branch, which is how Lean is often told the
           motive. Erasing it is harmless in Lean (the motive is already in the `Expr`) but a
           `Syntax`-level translator that erases it then has to infer the motive itself, and
           LOGOS's `infer_match_motive` is Miller-restricted (T04-011).
@oracle    The emitted term contains no identity wrapper, no `mdata`-analogue, and no
           `Global("id")` introduced by an ascription; it is term-for-term identical to the
           translation of Lean's `Expr`; and for each example the emitted term's `infer_type`
           is the translation of the ascribed type.
-/

namespace T05_018

/-- The ascription is erased — but it CHOSE the literal's type. -/
def asNat : Nat := (5 : Nat)
def asInt : Int := (5 : Int)

example : asNat = 5 := rfl
example : asInt = 5 := rfl

/-- `show … from` — the same thing as `(e : T)`. -/
def showFrom : Nat := show Nat from 5

example : showFrom = 5 := rfl

/-- `show … by` — a tactic block, which leaves a real proof term behind. -/
theorem showBy (n : Nat) : n = n := show n = n by rfl

/-- Ascription driving COERCION insertion: the emitted term must contain the coercion. -/
def boolToNat (b : Bool) : Nat := if b then 1 else 0

instance : Coe Bool Nat := ⟨boolToNat⟩

def coerced : Nat := (↑true : Nat)

example : coerced = 1 := rfl

/-- Ascription pinning a UNIVERSE. Erased in the term, and LOGOS's cumulativity means the
    constraint is not recoverable from the emitted term either.

    Lean REFUSES the cumulative form, which is the whole point:

        def atType1 : Type 1 := (Nat : Type)
        -- error: type mismatch — Nat has type Type : Type 1 but is expected to have type
        --        Type 1 : Type 2

    The LOGOS translation of that same term WOULD typecheck, because
    `is_subtype_normalized` admits `Type 0 ≤ Type 1` (type_checker.rs:1330-1360). -/
def atType0 : Type := (Nat : Type)

/-- Ascription on a PROP. LOGOS says `Prop : Type 1`; Lean says `Prop : Type 0`. -/
def aProp : Prop := (True : Prop)

/-- Ascription that pins an IMPLICIT which nothing else determines. -/
def idf {α : Type} (a : α) : α := a

def pinned : List Nat := (idf [] : List Nat)

example : pinned = [] := rfl

/-- Ascription inside a match branch, used to tell Lean the motive. -/
def Carrier : Bool → Type
  | true => Nat
  | false => Bool

def mk (b : Bool) : Carrier b :=
  match b with
  | true => (0 : Nat)
  | false => (false : Bool)

example : mk true = (0 : Nat) := rfl

/-- Nested ascriptions, all of which vanish. -/
example : ((((5 : Nat) : Nat) : Nat)) = 5 := rfl

/-- Ascription on a function, which pins the domain of an otherwise-polymorphic lambda. -/
def pinnedFun : Nat → Nat := ((fun x => x) : Nat → Nat)

example : pinnedFun 3 = 3 := rfl

end T05_018
