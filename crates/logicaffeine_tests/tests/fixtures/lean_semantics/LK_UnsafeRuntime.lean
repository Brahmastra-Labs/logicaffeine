/-!
@case      T02-008
@tier      2 · Declarations
@title     `unsafe def` — outside the logic; there is nothing to translate it *into*
@lean      unsafe def, unsafeCast, `implemented_by`, the unsafe/safe declaration split
@target    (none — no sound translation)
@expect    reject
@diag      E_UNSAFE_DECLARATION — must name the declaration and state that `unsafe` code is
           excluded from Lean's own logical environment (it is not checked for termination,
           not checked for type soundness across `unsafeCast`, and may not be referenced by
           safe declarations), so there is no property for LOGOS to preserve. Must produce no
           LOGOS output at all, including no axiom.
@evidence  kernel/src/context.rs:20-100 (the entire `Context`: `inductives`, `constructors`,
           `declarations`, `definitions`, … — there is no unsafe/compiler-only compartment and
           no `implemented_by` table, so an unsafe declaration has nowhere to live except the
           *trusted* maps); kernel/src/context.rs:434-436 and :442-444 (`add_declaration` and
           `add_definition` both feed `get_global`, context.rs:449-464, which the type checker
           consults with no notion of trust level); kernel/src/type_checker.rs:321-343 (every
           `Fix` is termination-checked — there is no unchecked recursion form to park an
           unsafe function in)
@guard     Catches the "it's just an axiom" fallback. Emitting `add_declaration` for an
           `unsafe def` promotes a declaration Lean *refused to admit to its own logic* into a
           LOGOS axiom the kernel will happily use. For `unsafeCast : α → β` that axiom is
           immediately inconsistent: it inhabits `Nat → False`.
@guard     Catches following `@[implemented_by]`. A safe declaration with an unsafe
           implementation has *two* bodies in Lean; only the safe one has logical content. A
           translator that reads the implementation (because it is the one that actually runs)
           emits the unchecked term. The rule is: translate the safe body, ignore the
           implementation, and record nothing about it.
@guard     Catches leaking an unsafe declaration through a *safe* one that mentions it. Lean
           enforces the safe/unsafe barrier at `addDecl`; LOGOS has no barrier, so if the
           translator emits the unsafe constant at all, any later safe declaration referencing
           it links up silently.
@guard     Catches degrading rather than rejecting. There is no loss record that makes this
           acceptable: the information lost is the *entire* soundness argument.
@oracle    Translator exits non-zero with E_UNSAFE_DECLARATION and the LOGOS context contains
           no entry (definition, declaration, inductive or constructor) for any `unsafe` name.
-/

namespace T02_008

/-- The archetype: a type-punning cast. As a LOGOS axiom this immediately proves `False`. -/
unsafe def coerceAnything {α β : Type} (a : α) : β := unsafeCast a

/-- Unsafe recursion with no termination argument and no `partial` marker — Lean does not
    even attempt a guard here. -/
unsafe def diverge (n : Nat) : Nat := diverge n

/-- An unsafe declaration that *calls* another unsafe one. Both must be refused; emitting
    either one makes the other linkable. -/
unsafe def useCoerce (n : Nat) : Bool := coerceAnything n

/-- The `implemented_by` shape: a perfectly safe declaration whose *runtime* body is unsafe.
    Only `safeId` has logical content, and it is the only thing that may be translated. -/
unsafe def fastIdImpl (n : Nat) : Nat := n

@[implemented_by fastIdImpl]
def safeId (n : Nat) : Nat := n

/-- A theorem about the safe declaration. It must survive translation; nothing about
    `fastIdImpl` may. -/
theorem safeId_eq (n : Nat) : safeId n = n := rfl

end T02_008
