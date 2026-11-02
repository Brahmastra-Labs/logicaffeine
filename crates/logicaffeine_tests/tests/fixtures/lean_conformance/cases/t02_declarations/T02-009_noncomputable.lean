/-!
@case      T02-009
@tier      2 · Declarations
@title     `noncomputable` — an extraction marker with no LOGOS carrier
@lean      noncomputable def, noncomputable section, `Classical.choice`-dependent definitions
@target    Context::add_definition(name, ty, body)  + a loss record
@expect    degrade
@evidence  kernel/src/context.rs:442-444 (`add_definition` stores `(ty, body)` only — no
           computability flag, no compiler IR, no `@[extern]` table); kernel/src/context.rs:20-100
           (the `Context` has no code-generation compartment at all);
           kernel/src/reduction.rs:52-69 (`normalize` unfolds any definition body it finds —
           it has no notion of a body that "must not be run");
           kernel/src/prelude.rs:1195-1219 (`reduceBool`/`ofReduceBool` — the one place LOGOS
           wires an *evaluator* into kernel reduction, and the only place computability is
           load-bearing)
@loss      COMPUTABILITY_MARKER. In Lean `noncomputable` records that the definition has no
           executable compiler implementation (typically because it goes through
           `Classical.choice`). It has **no** effect on the logic: the definitional content is
           identical. LOGOS keeps the definitional content and drops the marker.
@loss      EXTRACTION_VIABILITY. A LOGOS context assembled from a Lean development can no
           longer report which globals are runnable. That matters for exactly one LOGOS
           feature: `reduceBool` (prelude.rs:1195-1219) reduces by invoking the CBV evaluator
           (reduction.rs:109-115). A `reduceBool` applied to a term whose head unfolds into a
           noncomputable definition will not reduce, and the failure looks like a `decide`
           failure rather than a computability error.
@guard     Catches the translator *rejecting* `noncomputable` declarations. They are ordinary
           definitions logically; refusing them would drop most of a classical development for
           no soundness reason.
@guard     Catches the translator translating `noncomputable` to `add_declaration` (an axiom)
           "because it cannot be evaluated". That throws away a body Lean fully checked, and
           turns a theorem provable by unfolding into an assumption.
@guard     Catches the *real* hazard hiding behind this case: a `noncomputable` Lean definition
           is usually noncomputable because it uses `Classical.choice`, and
           **`Classical.choice` is absent from the LOGOS trusted base** (prelude.rs:78 declares
           `dne` as the only classical axiom; there is no `choice`, no `propext`, no `funext`).
           So the *body* of such a definition frequently does not translate at all — that is a
           `reject` under T06-009, not a degrade. A translator that sees `noncomputable`,
           emits the loss record, and moves on without inspecting the body for `Classical.choice`
           will emit a term mentioning an unregistered global, which `get_global`
           (context.rs:449-464) resolves to `None` and the checker reports as an unbound name
           far from the cause.
@oracle    The emitted declaration is a definition with the same body; the loss report contains
           COMPUTABILITY_MARKER; and if the body mentions `Classical.choice` the translator
           instead reports the T06-009 rejection rather than the loss.
-/

namespace T02_009

/-- A `noncomputable` definition that is nonetheless *logically* ordinary: it is only
    noncomputable because `Classical.choice` has no executable content. -/
noncomputable def someNat : Nat := Classical.choice (⟨0⟩ : Nonempty Nat)

/-- The marker is load-bearing only for codegen; the type is fine and the definition is
    usable in proofs. -/
example : someNat = someNat := rfl

/-- `Classical.choice` reached indirectly, through `Classical.em`. This is the shape that must
    be *rejected* (T06-009), not merely degraded — the translator has to look at the body. -/
noncomputable def decideProp (P : Prop) : Bool :=
  @decide P (Classical.propDecidable P)

/-- A `noncomputable` definition whose body contains **no** classical content. Lean still
    demands the marker once it is inside a `noncomputable section`; here it is marked
    explicitly. This one degrades cleanly: body translates, marker is dropped. -/
noncomputable def plainlyFine : Nat := 3 + 4

example : plainlyFine = 7 := by decide

/-- The contrast case: the identical body without the marker. After translation the two are
    indistinguishable, which is exactly the information the loss record must carry. -/
def plainlyFineComputable : Nat := 3 + 4

end T02_009
