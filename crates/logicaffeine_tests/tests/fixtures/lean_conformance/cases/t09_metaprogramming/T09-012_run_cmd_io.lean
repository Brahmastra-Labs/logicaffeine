import Lean

/-!
@case      T09-012
@tier      9 · Metaprogramming and syntax
@title     `run_cmd` and `#eval` of an `IO` action — elaboration-time effects that the LOGOS
           kernel, which has no IO at all, cannot have
@lean      run_cmd, #eval with IO, IO.println, IO.FS.readFile, elaboration-time effects
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/term.rs:443-545 (`Term` — thirteen variants, none of which sequences an
           effect. There is no monadic bind, no `IO`, no world token, and `Term::Let`
           (term.rs:471) is a pure zeta-transparent binding, not a `do`-step);
           kernel/src/prelude.rs (the registered inductives and axioms — `Entity`, `Int`,
           `Float`, `Text`, `Duration`, `Date`, `Moment`, `Nat`, `EList`, `Bool`, `TList`,
           `True`, `False`, `Decidable`, `Eq`, `Acc`, `And`, `Or`, `Ex`, the quotient
           primitives, the `Int` ring and order axioms, and `reduceBool`/`ofReduceBool`. There
           is no `IO`, no `Monad`, no `Bind`, and no `PUnit`/`RealWorld`);
           kernel/src/prelude.rs:1195-1219 (`reduceBool : Bool → Bool` with `ofReduceBool` —
           the ONE place LOGOS deliberately wires an evaluator into kernel reduction
           (reduction.rs:109-115). It is Lean's `native_decide` pair, it is gated to `Bool`, and
           it is pure: it is NOT an escape hatch for effects. See T06-014);
           the crate invariants recorded in the repo's own guide — `logicaffeine_kernel` is
           "Pure CoC type theory — NO LEXICON (Milner invariant)" and `logicaffeine_data` is
           "WASM-safe data structures + CRDTs — NO IO (Lamport invariant)"; IO lives only in
           `logicaffeine_system`, which the kernel does not depend on;
           kernel/src/context.rs:20-80 (`Context` — no effect log, no output buffer; see
           T09-007 on the absence of any mutable environment state)
@diag      E_ELABORATION_TIME_EFFECT — must name the command, state that it performs `IO` during
           elaboration, and state that the LOGOS kernel has no IO and no monadic structure, so
           the effect has no representation at any stage. Must emit no declaration. If the
           effect PRODUCED declarations (a `run_cmd` that elaborates commands), those
           declarations are ordinary and translate — the same nuance as T09-004 and T09-006.
@guard     THE EFFECT-IS-NOT-A-VALUE TRAP. `#eval (IO.println "hi")` elaborates to a term of
           type `IO Unit` and then RUNS it. A translator that stops at "it is a term, terms
           translate" will emit something for the `IO Unit` value — at best an opaque
           `add_declaration` of a type LOGOS has no inductive for, at worst a definition whose
           body is a partially-translated monadic chain. Neither records that the source file
           printed something, read a file, or changed the world. The verdict is `reject` because
           a translation that silently drops an effect is a translation that claims the source
           did less than it did.
@guard     Catches `run_cmd` that GENERATES DECLARATIONS. This is the important sub-case and the
           exact mirror of T09-004: `run_cmd elabCommand (← `(def generated : Nat := 5))` is an
           effect whose visible consequence is an ordinary `def`. The effect is rejected; the
           `def` it produced is in the environment and must be translated. A translator that
           refuses the whole file because it saw a `run_cmd` will drop declarations that are
           perfectly translatable — including, in real Lean, whole families of generated lemmas.
@guard     Catches `reduceBool` being read as an IO escape hatch. `reduceBool` runs the CBV
           evaluator inside kernel reduction (prelude.rs:1195-1219, reduction.rs:109-115) and
           is the closest LOGOS comes to "running code during checking". It is pure, it is
           `Bool`-only, and its TCB story is already a degrade of its own (T06-014). Routing an
           `IO` action through it — encoding a print as a `Bool` computation, say — puts
           arbitrary evaluation into the kernel's trusted reduction path under a type that
           claims it is a decision procedure.
@guard     Catches NON-DETERMINISM. `IO.FS.readFile` makes the elaborated environment depend on
           the filesystem, so the same Lean source produces different declarations on different
           machines. A translator's output would then not be a function of its input, which
           breaks the one property a conformance corpus needs. The rejection must fire even when
           the effect looks harmless.
@guard     Catches `do` notation generally. Lean's `do` block desugars to `bind`/`pure`
           applications, and LOGOS has no `Monad`, no `Bind` and no `IO` (see T05-019 and
           T05-020). A translator that handles `IO` specially but lets other monads through will
           emit `bind` applications against globals that do not exist.
@note      `import Lean` (core, not Mathlib) is required for `run_cmd`; see the @note on T09-004
           about the metadata block's position. The `run_cmd` and `#eval`s below print during
           compilation — that output is expected and is not a warning.
@note      A declaration created by `run_cmd elabCommand` is NOT visible to later commands in the
           same Lean 4.14 file (its elaboration is asynchronous, and a reference to the new name
           reports "unknown identifier"). The declaration-generating `run_cmd` below therefore
           emits a name nothing references, and the declaration it stands for is written out by
           hand as `generatedByRunCmd`. The case is unaffected: the point is that the EFFECT is
           rejected while the declaration it produces is ordinary — and the visibility rule is
           itself a consequence of the same import-time/elaboration-time staging that makes
           T09-007's `initialize` unrepresentable.
@oracle    Translator exits non-zero with E_ELABORATION_TIME_EFFECT for every `run_cmd` and for
           every `#eval` of an `IO`-typed term below, emits nothing for them, and DOES emit a
           correct LOGOS declaration for `generatedByRunCmd` and for the pure declarations. A
           harness additionally asserts that no emitted `Term::Global` names `IO`, `bind`,
           `pure` or any other monadic operation, and that no `add_declaration` introduces a
           type not registered by `StandardLibrary::register`.
-/

namespace T09_012

open Lean Elab Command

-- A `run_cmd` performing pure IO at elaboration time.
run_cmd Lean.logInfo "elaborating T09_012"

-- A `run_cmd` that GENERATES A DECLARATION. The effect is rejected; the `def` it produces is
-- ordinary and must be translated. Lean 4.14 elaborates the generated command
-- asynchronously, so the new name is not visible to LATER commands in the same file
-- ("unknown identifier"); the declaration it produces is therefore written out by hand
-- below, exactly as in T09-006.
run_cmd elabCommand (← `(def generatedElsewhere : Nat := 5))

-- What the `run_cmd` above emits. This is the declaration the translator must produce.
def generatedByRunCmd : Nat := 5

theorem generatedTranslates : generatedByRunCmd = 5 := rfl

-- `#eval` of an `IO` action — an effect, not a query. Contrast T09-009, where `#eval` of a
-- PURE term is a no-op the translator correctly ignores.
#eval (IO.println "side effect during elaboration" : IO Unit)

-- `#eval` of an IO action that READS THE WORLD, so the elaborated environment is not a
-- function of the source alone.
#eval show IO Unit from do
  let t ← IO.monoMsNow
  if t > 0 then pure () else IO.println "clock went backwards"

-- A pure declaration in the same file — must still translate.
def pureDecl (n : Nat) : Nat := n + generatedByRunCmd

theorem pureDeclTranslates : pureDecl 1 = 6 := rfl

/-- A `do` block in an ordinary definition — no `IO`, but still monadic, and LOGOS has no
    `Monad`/`Bind` at all. See T05-019 and T05-020. -/
def optionDo (a b : Option Nat) : Option Nat := do
  let x ← a
  let y ← b
  pure (x + y)

theorem optionDoWorks : optionDo (some 1) (some 2) = some 3 := rfl

/-- An `IO`-typed DEFINITION, so the effect is a value sitting in the environment rather than
    something that ran. Its type mentions `IO`, which LOGOS has no inductive for. -/
def ioValue : IO Unit := IO.println "never run at elaboration time"

end T09_012
