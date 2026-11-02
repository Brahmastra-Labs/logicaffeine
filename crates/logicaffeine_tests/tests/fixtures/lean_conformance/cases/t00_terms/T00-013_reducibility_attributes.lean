/-!
@case      T00-013
@tier      0 · Kernel term layer
@title     `@[reducible]` / `@[irreducible]` — LOGOS has no transparency control
@lean      @[reducible], @[irreducible], abbrev, TransparencyMode, `with_unfolding_all`
@expect    degrade
@target    Every Lean definition, whatever its reducibility, becomes
           `Context::add_definition(name, ty, body)` — which is unconditionally
           delta-TRANSPARENT. An `@[irreducible]` definition has only one faithful alternative,
           `Context::add_declaration(name, ty)` (an opaque axiom), which loses the body
           entirely. The translator must pick `add_definition` and record the loss.
@evidence  kernel/src/context.rs:442 (`add_definition(name, ty, body)` — the ONLY definition
           form; it takes no reducibility argument and there is no later API to set one);
           kernel/src/context.rs:434 (`add_declaration(name, ty)` — type only, no body, the only
           way to make something opaque, and it makes it opaque FOREVER);
           kernel/src/reduction.rs:36-91 (`reduce_step` unfolds a global whenever
           `ctx.get_global`/`get_universe_poly` yields a body — there is no transparency gate,
           no `TransparencyMode`, and no per-call setting);
           kernel/src/context.rs (the `Context` maps carry name → type/body only: no attribute
           map, no reducibility hint map — LOGOS_FACTS: "no visibility, no attributes, no
           reducibility hints")
@loss      TRANSPARENCY_HINT. Lean's four transparency levels (`reducible`, `instances`,
           `default`, `all`) collapse to one. Two consequences, in opposite directions:
           (a) `@[irreducible]` becomes transparent, so LOGOS proves by `refl` statements that
           Lean's default elaboration refuses — the OVER-PERMISSIVE direction;
           (b) `@[reducible]` becomes merely transparent rather than preferentially unfolded, so
           LOGOS's `normalize` unfolds it at the same priority as everything else and spends
           fuel it did not need to (reduction.rs:52-69, T00-012).
           Acceptable only because Lean's reducibility is an ELABORATION hint with no effect on
           the Lean kernel: the kernel unfolds everything. So the translated term is still a
           faithful translation of the Lean *kernel* term — what is lost is the ability to
           reproduce Lean's elaboration behaviour, i.e. whether a given `rfl` would have been
           accepted at the source level.
@guard     Catches translating `@[irreducible]` as `add_declaration` (an axiom). It preserves
           opacity, which looks faithful, and it is a catastrophe: the body is gone, so every
           equation the definition was supposed to satisfy becomes unprovable, and — worse — an
           `add_declaration` with a `Prop` type is INDISTINGUISHABLE from a genuine axiom in the
           emitted context. A proof library that acquires axioms this way has no way to report
           its own trusted base. This is the same failure mode as T11-014 (`sorry` smuggled in
           as an axiom), reached by a different route.
@guard     Catches translating `abbrev` (which is `@[reducible] def`) as anything other than a
           definition — in particular as a notation to be inlined at every use site, which
           changes term size and blocks the syntactic matches `try_structure_eta`
           (type_checker.rs:1391-1417) and the termination guard (termination.rs) perform.
@guard     Catches silently accepting the over-permissive direction. A translator that verifies
           its output by "does LOGOS accept it" will see the `@[irreducible]` case pass and
           conclude the translation is faithful. It is not: LOGOS accepted MORE than Lean would.
           The verification obligation runs the other way — every `rfl` the LOGOS side discharges
           must correspond to a `rfl` Lean's elaborator discharges at DEFAULT transparency, and
           that is exactly the fact this attribute destroys.
@guard     Catches dropping `@[reducible]` on a TYPE synonym used as a class instance head. In
           Lean, instance search unfolds reducible definitions only; in LOGOS, `add_instance`
           (context.rs:184) is a flat `Vec<(Term, Term)>` matched first-wins, with no
           unfolding policy at all — so which instance is found can differ. That divergence is
           T07-008's subject; here the guard is simply that the reducibility that DROVE Lean's
           choice is gone by the time LOGOS resolves.
@oracle    Every Lean definition, regardless of attribute, appears in the emitted context via
           `add_definition` with its body intact; no `add_declaration` is emitted for anything
           that had a body in Lean; and a loss record with category TRANSPARENCY_HINT lists each
           declaration whose Lean reducibility was not `default`.
-/

namespace T00_013

/-- The default: Lean's elaborator unfolds it at default transparency, and so does LOGOS. -/
def ordinary : Nat := 5

example : ordinary = 5 := rfl

/-- `@[reducible]`: unfolded even at `reducible` transparency, so instance search and
    unification see through it. In LOGOS this is just another `add_definition`. -/
@[reducible] def NatAlias : Type := Nat

example : NatAlias = Nat := rfl

def usesAlias (n : NatAlias) : Nat := n

/-- `abbrev` is `@[reducible] def`. -/
abbrev BoolAlias : Type := Bool

def usesAbbrev (b : BoolAlias) : Bool := b

/-- `@[irreducible]`: Lean's elaborator will NOT unfold this at default transparency, so
    `example : secret = 5 := rfl` is refused at the source level. The Lean KERNEL would accept
    it, and LOGOS's `add_definition` unconditionally will — that is the whole loss. -/
@[irreducible] def secret : Nat := 5

/-- A consumer of the irreducible definition. Its body mentions `secret` without unfolding it;
    the emitted LOGOS term will unfold it during any `normalize`. -/
def usesSecret : Nat := secret + 1

/-- An irreducible definition at a TYPE, so the opacity affects unification rather than just
    computation. -/
@[irreducible] def OpaqueType : Type := Nat

def intoOpaque (n : Nat) : OpaqueType := by unfold OpaqueType; exact n

/-- The genuinely opaque construct, for contrast: `opaque` has no body at all and is the only
    thing that should ever reach `add_declaration`. -/
opaque genuinelyOpaque : Nat

end T00_013
