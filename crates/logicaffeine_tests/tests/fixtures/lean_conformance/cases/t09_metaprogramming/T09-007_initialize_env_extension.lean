import Lean

/-!
@case      T09-007
@tier      9 · Metaprogramming and syntax
@title     `initialize` and environment extensions — LOGOS's `Context` has no extension slot,
           no attribute system, and no import-time effects
@lean      initialize, builtin_initialize, registerSimpAttr, EnvExtension, IO.Ref state,
           attribute registration
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/context.rs:20-80 (`Context` — the COMPLETE state: `bindings`,
           `inductives`, `constructors`, `inductive_params`, `constructor_order`,
           `declarations`, `definitions`, `hints`, `universe_polys`, `instances`, `coercions`,
           and per-binder kinds. Every one is a flat `HashMap`/`Vec` of TERMS. There is no
           extension registry, no attribute map, no user-extensible slot, and no IO);
           kernel/src/context.rs:434, :442 (the two ways a name enters the environment —
           `add_declaration` and `add_definition` — both take only a name and terms, so nothing
           can attach metadata to a declaration);
           kernel/src/context.rs:449 (`get_global`'s lookup precedence: inductives,
           constructors, definitions, declarations. Nothing consults an extension);
           kernel/src/certificate.rs:63-75 (`recheck` REBUILDS the trusted context itself from
           `StandardLibrary::register`, explicitly refusing to "trust a context the certificate
           might carry" — so even if a translator did smuggle extra state into a `Context`, the
           artefact that is supposed to be re-checkable would not carry it);
           kernel/src/certificate.rs:28 (`PRELUDE_VERSION = "logos-coc-1"` — the ONLY
           environment-level metadata in the system, a single version string);
           proof/src/tactic_script.rs:385-396, :421-437 (`ScriptedTheorem::simp` and the
           `simp_pool` — the nearest thing to an attribute, and it is CALL-LOCAL to one
           `prove_scripted_library` invocation, not persistent);
           kernel/src/prelude.rs (the trusted base is built by a fixed sequence of registration
           calls with no hook for user code to join in)
@diag      E_NO_ENVIRONMENT_EXTENSION — must name the `initialize` block and the extension or
           attribute it registers, state that LOGOS's `Context` has no extension slot and no
           attribute system, and state that the effect is a persistent, import-time mutation
           that has no artefact to live in. Must emit nothing for the block.
@guard     THE STATE-IS-NOT-A-DECLARATION TRAP. `initialize` does not declare anything: it RUNS
           `IO` at import time and mutates an environment extension or an `IO.Ref`. There is no
           term, no type, and no name for the translator to emit. A translator that treats the
           `initialize` block as an ordinary `def` (it does elaborate to a term of type
           `IO Unit` or similar) and emits it as a definition produces a LOGOS global of an
           uninhabited-in-LOGOS type that no one calls — harmless, but it hides the fact that
           the SIDE EFFECT was silently dropped, and the side effect is the whole point.
@guard     Catches dropping an attribute registration and continuing as if nothing happened.
           `initialize registerSimpAttr` and `attribute [simp] foo` change how LATER
           declarations elaborate and how tactics behave. LOGOS has no attribute system at all
           (T02-013), so the effect cannot be carried — and its absence is not local to the
           `initialize` block: every downstream `simp` now runs a different rule set. The
           rejection must therefore extend to a REPORT that the affected tactic calls are no
           longer faithful, not just to the block itself. See T08-012's DEFAULT_SET loss.
@guard     Catches the `simp_pool` false friend. `ScriptedTheorem { simp: true }`
           (tactic_script.rs:392-395) puts a proved conclusion into a pool visible to later
           theorems in the SAME `prove_scripted_library` call (tactic_script.rs:432-437). That
           looks like `@[simp]` and is not: it does not persist past the call, it is keyed on
           proof order within the call, and a `## Theorem` block never sees it. Mapping Lean's
           `@[simp]` onto it converts a global persistent fact into a call-local one, which
           changes which theorems are provable depending on how the translator batched them.
@guard     Catches `IO.Ref` state. `initialize ref : IO.Ref Nat ← IO.mkRef 0` creates mutable
           state that later metaprograms read and write. LOGOS's kernel has NO IO — the crate is
           pure by construction (`logicaffeine_data` carries the "Lamport invariant: NO IO", and
           `logicaffeine_kernel` the "Milner invariant: NO LEXICON") — so there is not merely no
           representation for the reference, there is no place for it to exist.
@guard     Catches the IMPORT-TIME timing. `initialize` runs when the module is IMPORTED, not
           where it is written — which is why T09-006's custom deriving handler cannot be used
           in the file that registers it. A translator processing one module at a time will
           never observe the effect at all, and one processing a whole build will observe it in
           an order determined by the import graph. Neither ordering is representable in a
           LOGOS `Context`, which is built by a linear sequence of `add_*` calls.
@note      `import Lean` (core, not Mathlib) is required for `initialize` and for the extension
           API; see the @note on T09-004 about the metadata block's position.
@oracle    Translator exits non-zero with E_NO_ENVIRONMENT_EXTENSION for every `initialize`
           block and every `attribute`/`@[…]` registration below, emits nothing for them, and —
           critically — emits a REPORT entry saying which later declarations' elaboration or
           tactic behaviour depended on the dropped state. The ordinary declarations
           (`tagged`, `plainDef`) must still translate.
-/

namespace T09_007

open Lean

/-- An `initialize` block creating mutable `IO` state. LOGOS's kernel has no IO at all. -/
initialize counterRef : IO.Ref Nat ← IO.mkRef 0

/-- An `initialize` block registering a persistent ENVIRONMENT EXTENSION — state attached to
    the environment itself, serialised into the `.olean` and restored on import. -/
initialize myExt : SimplePersistentEnvExtension Name (Array Name) ←
  registerSimplePersistentEnvExtension {
    addImportedFn := fun as => as.foldl Array.append #[]
    addEntryFn := fun s n => s.push n
  }

/-- An `initialize` block registering an ATTRIBUTE, which changes how later commands elaborate. -/
initialize registerTraceClass `t09007.trace

/-- An ordinary declaration in the same file — must still translate. -/
def plainDef (n : Nat) : Nat := n + 1

theorem plainDefTranslates : plainDef 1 = 2 := rfl

/-- A declaration carrying a core ATTRIBUTE. The attribute has no LOGOS carrier, and its effect
    (joining the global `simp` set) changes every downstream `simp`. -/
@[simp] theorem tagged (n : Nat) : n + 0 = n := Nat.add_zero n

/-- A `simp` call whose behaviour DEPENDS on the attribute above — the downstream effect the
    diagnostic must report, not just the attribute itself. -/
theorem dependsOnAttribute (k : Nat) : k + 0 = k := by simp

/-- The `attribute` command form, applied after the fact. -/
theorem laterTagged (n : Nat) : n * 1 = n := Nat.mul_one n

attribute [simp] laterTagged

theorem dependsOnLaterAttribute (k : Nat) : k * 1 = k := by simp

end T09_007
