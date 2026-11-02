/-!
@case      T00-020
@tier      0 · Kernel term layer
@title     `show T from e` and `(e : T)` — ascription is erased by Lean before translation
@lean      (e : T), show T from e, show T by tac, Lean.Meta.mkExpectedTypeHint
@expect    translate
@target    The ascription leaves NO trace: `(e : T)` ⟼ `translate e`. No wrapper, no coercion
           function, no `Term::Let` with `ty := T`.
@evidence  kernel/src/term.rs:443-545 (there is no ascription, cast, or type-annotation variant
           in `Term`); kernel/src/type_checker.rs:50-52 (`infer_type` is purely synthesising —
           there is no bidirectional entry point a translator could hand an expected type to
           except `check_type`, which returns `()` and produces no term);
           kernel/src/prelude.rs (the registered globals are enumerated in LOGOS_FACTS and there
           is NO `id` — so there is no identity function to wrap with, either);
           kernel/src/type_checker.rs:452-471 and :1309-1323 (`check_type` uses `is_subtype`,
           i.e. CUMULATIVE subsumption — a strictly more permissive test than Lean's
           definitional equality, which is why the ascription cannot be re-derived downstream)
@guard     Catches emitting an ascription as an application of an identity function. `Global("id")`
           does not resolve (context.rs:449) and a locally defined `id` adds a delta-redex at
           every ascription: term size grows, `normalize`'s 10000-step fuel (reduction.rs:40) is
           consumed, and the syntactic head tests in `try_structure_eta`
           (type_checker.rs:1391-1417), the termination guard (termination.rs:378-400) and
           `elaborate::spine` (elaborate.rs:1276) all see `id` where they expected a constructor
           or a recursive call.
@guard     Catches emitting the ascription as `Term::Let { name: "_", ty: T, value: e, body:
           Var("_") }` in order to "keep" the type. It typechecks — `Let` checks `value` against
           `ty` (type_checker.rs:140) — and it introduces a binder literally named `_`, which
           `Display` renders indistinguishably from a non-dependent `Pi`'s binder
           (term.rs:583-588) and which collides with every other `_` binder the translator emits
           (T00-004, T00-015).
@guard     Catches the reverse error: erasing the ascription at the SYNTAX level. `(3 : Int)` and
           `(3 : Nat)` are the same surface tokens and different `Expr`s — the ascription is what
           selected the `OfNat` instance, and `Literal::Nat` versus `Literal::Int` are different
           LOGOS worlds with different types (`Global("Nat")` vs `Global("Int")`,
           type_checker.rs:294-302) and completely different theories (`Nat` inductive with
           `Zero`/`Succ`; `Int` opaque with axiomatised ring laws at prelude.rs:92-191). The
           erasure is sound ONLY post-elaboration. A `Syntax`-level translator must not do it.
@guard     Catches treating the erased ascription as still ENFORCED. In Lean, `(e : T)` requires
           `typeof(e)` and `T` to be definitionally equal. Once erased, the only remaining check
           is whatever the enclosing position performs — and enclosing positions in LOGOS use
           `check_type`/`is_subtype`, which admits `Prop ≤ Type 0 ≤ Type 1` and `Pi` variance
           (type_checker.rs:1330-1360). So a translator that relies on "the ascription will be
           re-checked anyway" is relying on a WEAKER check than the one it deleted. If the
           ascription was the only thing pinning a universe level, the level is now free to
           float up. See T01-009.
@guard     Catches `show T by tac`, where the ascription is attached to a TACTIC block. The
           tactic ran in Lean and produced a term; the ascription is erased along with the
           tactic, and what must be translated is the resulting `Expr`. A translator that tries
           to map the tactic to a LOGOS script surface (Tier 8/12) instead of translating the
           produced term has changed the trust story entirely — see T12-013.
@oracle    The emitted term for `(e : T)` is structurally identical to the emitted term for `e`
           written without the ascription; no `id`-like wrapper and no zero-use `Let` appears;
           and for each ascribed/unascribed pair below the two emitted terms are literally equal,
           not merely `def_eq`.
-/

namespace T00_020

/-- `(e : T)` where the ascription is redundant: same `Expr` with or without it. -/
def ascribedNat : Nat := (3 : Nat)
def plainNat : Nat := 3

example : ascribedNat = plainNat := rfl

/-- `show T from e`, the other spelling of the same thing. -/
def viaShowFrom : Nat := show Nat from 3

/-- The ascription that CHANGES the elaborated term: same tokens, different `OfNat` instance,
    different LOGOS literal and different LOGOS type. Erasure is only safe after this has
    already happened. -/
def numeralAsNat : Nat := (42 : Nat)
def numeralAsInt : Int := (42 : Int)

/-- An ascription that drives elaboration of a POLYMORPHIC term: without it the implicit is
    undetermined. The ascription is erased, but the instantiation it forced survives. -/
def emptyListAscribed : List Nat := ([] : List Nat)

/-- An ascription on a FUNCTION, so the erased type was a `Pi`. -/
def ascribedFn : Nat → Nat := ((fun n => n + 1) : Nat → Nat)

/-- An ascription that pins a UNIVERSE LEVEL. In Lean this is the only thing making the term
    live at `Type 1` rather than `Type 0`; under LOGOS's cumulativity the erased constraint is
    not recoverable from the enclosing check. -/
def pinnedLevel : Type 1 := (ULift.{1, 0} Nat : Type 1)

/-- `show T by tac`: the ascription and the tactic both vanish; the produced term is what
    translates. -/
theorem viaShowBy : 2 + 2 = 4 := show 2 + 2 = 4 by decide

/-- A nested chain of ascriptions — all of them erased, leaving one term. -/
def tripleAscribed : Nat := (((3 : Nat) : Nat) : Nat)

example : tripleAscribed = 3 := rfl

end T00_020
