/-!
@case      T02-006
@tier      2 · Declarations
@title     `opaque` — an axiom plus a discharged inhabitation obligation
@lean      opaque, Lean.OpaqueVal, the `Inhabited`/`Nonempty` side condition
@target    Context::add_declaration(name, ty)   (the witness is *not* emitted)
@expect    translate
@evidence  kernel/src/context.rs:434-436 (`add_declaration` — type only, opaque, no body);
           kernel/src/context.rs:474-476 (`get_definition_body` is `None`, so `opaque` never
           delta-unfolds, matching Lean); kernel/src/context.rs:20-100 (no `OpaqueVal`
           analogue and no place to store the unused witness value)
@guard     Catches translating `opaque` as `add_definition` with the *inhabitation witness* as
           the body. Lean's `opaque x : T := v` deliberately hides `v`: the value exists only
           to prove `T` non-empty, and unfolding to it is unsound with respect to the author's
           intent (this is precisely how Lean models `IO`, `unsafeCast` targets, and FFI
           constants). Emitting `add_definition(name, T, v)` makes `x` *definitionally equal*
           to the placeholder — so `x = v` becomes provable by `rfl` in LOGOS while it is
           unprovable in Lean. That is a term Lean rejects being accepted by LOGOS: a
           soundness-shaped divergence, not a cosmetic one.
@guard     Catches dropping the inhabitation obligation. `opaque` is safe in Lean *only*
           because Lean checked the witness. `add_declaration` checks nothing
           (context.rs:434-436 is a bare `HashMap::insert`), so an `opaque` whose type turns
           out to be empty becomes a plain inconsistent axiom. The translator must verify the
           witness in Lean before emitting, and must record the resulting assumption exactly as
           it records a real `axiom` (see T02-003).
@guard     Catches conflating `opaque` with `@[irreducible] def`. `@[irreducible]` still has a
           body Lean can be told to unfold; `opaque` has none. Both degrade to *different*
           LOGOS forms (definition vs declaration), so mapping them to the same thing loses a
           real distinction in one direction and is unsound in the other.
@oracle    The emitted name is a *declaration* (not a definition); `get_definition_body` is
           `None`; no witness term appears anywhere in the emitted context; and the trust
           report lists the name among assumed constants.
-/

namespace T02_006

/-- The canonical `opaque`: inhabited by `0`, but `0` must not be visible. -/
opaque secretNat : Nat := 0

/-- Observable in Lean: this does **not** typecheck by `rfl`, because `secretNat` does not
    unfold. The commented line is the term a translator that emitted the witness as a body
    would make provable in LOGOS.
    `example : secretNat = 0 := rfl`  -- rejected by Lean -/
example : secretNat = secretNat := rfl

/-- A function-typed `opaque`. The witness is a total function; the declaration exposes only
    the arrow type. -/
opaque hash : Nat → Nat := fun n => n

/-- Again, `hash 3 = 3` is NOT provable in Lean. Only reflexivity on the opaque head is. -/
example : hash 3 = hash 3 := rfl

/-- A *parameterised* opaque over a type that needs an `Inhabited` instance to be known
    non-empty. This is the shape where the obligation is non-trivial. -/
opaque defaultOf (α : Type) [Inhabited α] : α := default

/-- For contrast: `@[irreducible] def` keeps a body in Lean's environment, so it translates to
    `add_definition`, not `add_declaration`. Mapping both to one form is the bug the guard
    names. -/
@[irreducible] def sealed : Nat := 5

end T02_006
