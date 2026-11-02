/-!
@case      T02-005
@tier      2 · Declarations
@title     `example` — an anonymous declaration needs a generated, collision-free LOGOS name
@lean      example, Lean.Elab.Command.elabExample, `_example` internal naming
@target    Context::add_definition(<generated name>, ty, body)
@expect    translate
@evidence  kernel/src/context.rs:442-444 (`add_definition` takes a `String` name — there is no
           anonymous registration path); kernel/src/context.rs:20-100 (every global map in
           `Context` is keyed by `String`; there is no `Name` type, no anonymous/hygienic
           component, and no `Nat`-indexed macro scope); kernel/src/context.rs:449-464
           (`get_global` is a plain map lookup, so two `example`s that generate the same name
           are indistinguishable)
@guard     Catches the translator *dropping* `example`s. They are the cheapest conformance
           evidence a Lean file carries: each one is a closed, fully elaborated term that the
           LOGOS kernel can be asked to check. Silently skipping them means the translator's
           own regression surface disappears, and an `example` that Lean accepted but LOGOS
           rejects goes unnoticed.
@guard     Catches naive name generation. Lean's internal name for an example is `_example`,
           and *every* example in a module gets it — they do not collide in Lean because the
           declaration is never added to the environment under that name. In LOGOS
           `add_definition` is `HashMap::insert`, so emitting `_example` for each one means
           example N silently overwrites example N-1 and only the last is ever checked. The
           generated name must be unique per source position.
@guard     Catches a generated name that collides with a *user* name. LOGOS names are flat
           `String`s with no reserved namespace, so a scheme like `example_1` can be produced
           by a user declaration `example_1` or by flattening `example.1`. The scheme must use
           a character that cannot appear in a flattened Lean name, or must be checked against
           the context before insertion. See T02-020.
@guard     Catches emitting an `example` as an *axiom*: it has a body, and its whole value is
           that the body typechecks.
@oracle    Every `example` in the source produces exactly one LOGOS registration; the emitted
           names are pairwise distinct and distinct from every user name in the module; and
           each body `infer_type`s to its stated type.
-/

namespace T02_005

/-- Three examples with the *same* statement. Lean names all three `_example`; a translator
    that reuses one name keeps only the last. -/
example : 1 + 1 = 2 := rfl
example : 1 + 1 = 2 := by decide
example : 1 + 1 = 2 := Nat.add_comm 1 1 ▸ rfl

/-- An example with binders — the emitted term is a lambda, its type a `Pi`. -/
example (n : Nat) : n + 0 = n := Nat.add_zero n

/-- A *data*-valued example: `example` is not restricted to `Prop`. -/
example : Nat := 42

/-- An example whose type mentions a local `def`, so it is not closed over the prelude alone
    and the generated declaration must be ordered after `Threshold`. -/
def Threshold : Nat := 7
example : Threshold = 7 := rfl

/-- A user declaration named exactly like a plausible generated name. If the translator's
    scheme produces `example_1`, this collides. -/
def example_1 : Nat := 0

/-- And one named like the other plausible scheme. -/
def _example : Nat := 1

end T02_005
