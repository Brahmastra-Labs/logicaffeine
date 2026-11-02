/-!
@case      T02-012
@tier      2 · Declarations
@title     `private` / `protected` / `scoped` / `local` — LOGOS has no visibility at all
@lean      private def, protected def, scoped instance, local instance, local notation
@target    Context::add_definition / add_declaration / add_instance with no visibility
           qualifier  + a loss record
@expect    degrade
@evidence  kernel/src/context.rs:20-100 (the `Context` carries no visibility, no module
           boundary, no scope stack — the global maps are flat and permanently open);
           kernel/src/context.rs:449-464 (`get_global` has one resolution path for everyone);
           kernel/src/context.rs:184-186 (`add_instance` pushes to a single global `Vec`;
           there is no scoped-instance compartment to pop);
           kernel/src/context.rs:442-444 (`add_definition` is a bare `HashMap::insert`, so a
           "private" name occupies the same key space as a public one)
@loss      VISIBILITY. `private`, `protected`, `scoped` and `local` all erase to
           "unconditionally visible, forever". Nothing downstream can reconstruct the intended
           interface of a translated module.
@loss      SCOPE_LIFETIME. `local instance` and `scoped instance` are *supposed* to leave the
           instance database when their section/namespace closes. `add_instance` has no
           removal API (context.rs:184-186 is push-only), so a local instance registered during
           translation is permanently in the flat `Vec` that resolution walks first-match-wins.
@guard     Catches the highest-consequence version of this: a `local instance` deliberately
           installed for one section — a common Lean idiom for temporarily overriding a
           default — becomes a **permanent, high-priority** LOGOS instance. Because resolution
           is first match wins over insertion order (context.rs:67-71, :210-213), that
           override now silently wins for the rest of the development, changing which method
           body every later `[Class α]` argument resolves to. Nothing errors; the proofs just
           talk about a different structure.
@guard     Catches `private` name mangling. Lean's private names carry a module-hash
           component (`_private.Module.0.Foo.bar`). A mangling that *drops* the private prefix
           makes two private declarations of the same short name in two modules collide
           (T02-020); a mangling that keeps it produces a LOGOS key containing `.` and digits
           that no human can match to a source declaration. Either way the choice must be
           documented in the loss record.
@guard     Catches assuming `protected` is inert. It is inert *logically*, but it changes
           which short name resolves at a use site in Lean; translation must read the resolved
           `Name` from the elaborated term, never re-resolve a short name.
@oracle    Every visibility-qualified declaration is emitted; the loss report lists each with
           VISIBILITY, and every `local`/`scoped` instance additionally with SCOPE_LIFETIME and
           its position in the emitted instance `Vec`.
-/

namespace T02_012

/-- Private: invisible outside this module in Lean, unconditionally visible after
    translation. -/
private def hiddenHelper (n : Nat) : Nat := n + 1

/-- A public declaration that uses it. The dependency is real, so the private name must be
    emitted — it cannot simply be dropped. -/
def publicUser (n : Nat) : Nat := hiddenHelper (hiddenHelper n)

example : publicUser 0 = 2 := rfl

/-- Protected: `T02_012.protectedThing` cannot be referred to as `protectedThing` after
    `open T02_012`, but it is otherwise ordinary. -/
protected def protectedThing : Nat := 9

class Weight (α : Type) where
  w : α → Nat

/-- The default instance, declared first. -/
instance defaultWeight : Weight Nat where
  w := fun _ => 1

section Override

/-- A `local instance`: Lean removes it at `end Override`. LOGOS cannot remove it, and
    because it is registered later it is *later* in the flat `Vec` — so whether it wins
    depends entirely on the translator's insertion order, not on Lean's scoping. -/
local instance overrideWeight : Weight Nat where
  w := fun n => n

/-- Inside the section, Lean resolves to `overrideWeight`. -/
example : Weight.w 5 = 5 := rfl

end Override

/-- Outside the section, Lean resolves back to `defaultWeight`. A translation that kept the
    local instance in the global list cannot reproduce both of these two lines. -/
example : Weight.w 5 = 1 := rfl

namespace Scoped

/-- A `scoped instance` is visible only where the namespace is open. Same erasure. -/
scoped instance scopedWeightBool : Weight Bool where
  w := fun _ => 7

example : Weight.w true = 7 := rfl

end Scoped

end T02_012
