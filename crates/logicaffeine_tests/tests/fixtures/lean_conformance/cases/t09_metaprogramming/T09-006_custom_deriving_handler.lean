import Lean

/-!
@case      T09-006
@tier      9 · Metaprogramming and syntax
@title     Custom `deriving` handlers — the handler is a `CommandElabM` program; the
           declarations it generates are ordinary and must be translated
@lean      deriving with a custom handler, registerDerivingHandler, deriving Repr/BEq/Inhabited
@expect    reject
@target    (none for the handler — no sound translation). The GENERATED declarations translate
           by the ordinary rules: an instance becomes `Context::add_instance`
           (kernel/src/context.rs:184), a generated function becomes
           `Context::add_definition` (kernel/src/context.rs:442).
@evidence  kernel/src/context.rs:20-80 (`Context` — flat maps only; there is no handler
           registry, no attribute system, and no hook that fires on an inductive's
           registration); kernel/src/context.rs:253, :374, :419, :545 (`add_inductive`,
           `add_indexed_inductive`, `add_constructor`, `add_constructor_checked` — declaring an
           inductive runs NO user code and generates NO companions);
           kernel/src/recursor.rs:36 (`derive_recursor(ctx, ind)` — even the RECURSOR, the one
           thing LOGOS derives, is not created by declaration: it synthesises `(type, term)` and
           the CALLER must `add_definition` it. That is the whole extent of automatic
           generation in LOGOS);
           kernel/src/context.rs:184 (`add_instance(ty, value)` — a flat `Vec<(Term, Term)>`,
           first match wins: the target for a generated instance);
           kernel/src/prelude.rs:926, :1027 (`decEqBool` and `decEqNat` — the ONLY two decidable
           equality procedures in the trusted base; there is no `DecidableEq` synthesis, see
           T03-023);
           kernel/src/term.rs:443-545 (no `Term` variant can hold a Lean program, so the handler
           cannot be carried as data either)
@diag      E_METAPROGRAM_NOT_REPLAYABLE — must name the handler and the classes it derives,
           state that LOGOS has no deriving-handler registry and no hook on inductive
           declaration, and state explicitly that the GENERATED declarations are translated
           normally. Must emit nothing for the handler itself.
@guard     THE SAME NUANCE AS T09-004, and it matters more here because `deriving` is common in
           ordinary Lean. Rejecting the handler must NOT mean rejecting the type that uses it.
           `deriving Repr, BEq` appears on a large fraction of real Lean inductives; a
           translator that refuses any inductive carrying a `deriving` clause will refuse most
           input. The clause is a REQUEST TO RUN A PROGRAM; Lean has already run it, and the
           resulting instances are ordinary definitions sitting in the environment. The
           translator reads those, not the clause. This is the index's own note on T02-017:
           "handler output must be translated, not the `deriving` clause".
@guard     Catches translating the generated instance WITHOUT checking it is in a supported
           fragment. Deriving `BEq` for a recursive inductive generates a structurally recursive
           function; LOGOS's termination check is a syntactic Giménez guard
           (kernel/src/termination.rs) with no well-founded recursion, so a generated function
           Lean accepts may be refused. Deriving `Repr` generates `String`-producing code, and
           LOGOS has `Literal::Text` but NO string operations at all (T10-016), so `Repr`
           instances are untranslatable in practice even though the `deriving` clause that asked
           for them is what gets the diagnostic. The translator must diagnose against the
           GENERATED declaration, naming it, not against the clause.
@guard     Catches deriving `DecidableEq`. The trusted base has exactly `decEqBool` and
           `decEqNat` (prelude.rs:926, :1027) and no synthesis mechanism, so a generated
           `DecidableEq` for a user inductive must be emitted as an ordinary definition and
           registered with `add_instance` (context.rs:184) — a flat `Vec` searched by
           unification, first match wins. There is no priority and no coherence check, so two
           derived instances for overlapping types silently resolve to whichever was added
           first. See T07-008 and T03-023.
@guard     Catches the ORDER of emission. `add_instance` is insertion-ordered and first-match
           wins (context.rs:184). Lean's instance resolution uses priorities and specificity. A
           translator that emits derived instances in declaration order will get a different
           resolution than Lean whenever two of them could apply — and derived instances are
           exactly the ones a user never looks at, so the divergence is invisible.
@guard     Catches assuming the handler is deterministic or pure. `registerDerivingHandler`
           takes a `Array Name → CommandElabM Bool`: it can read the environment, emit several
           declarations, log messages, and decline (returning `false`) so that another handler
           runs. None of that is recoverable from the `deriving` clause.
@note      `import Lean` (core, not Mathlib) is required to register a handler; see the @note on
           T09-004 about the metadata block's position. The handler below deliberately does
           something trivial and visible — it emits a `def` named after the type plus an
           instance — so a harness can check that the OUTPUT was translated while the handler
           was rejected.
@note      Lean's `initialize registerDerivingHandler` takes effect only when the module is
           IMPORTED; a `deriving MyTag` clause in the SAME file fails with "default handlers
           have not been implemented yet". `Flag` and `Pair` below therefore carry no
           `deriving MyTag` clause and the handler's output is written out by hand. Nothing is
           lost for this case — the point is precisely that the translator reads the generated
           DECLARATIONS and never the clause — and the limitation is direct evidence for
           T09-007, since it exists because `initialize` mutates a persistent environment
           extension, which is the thing LOGOS has no counterpart for.
@oracle    Translator exits non-zero with E_METAPROGRAM_NOT_REPLAYABLE for the
           `initialize registerDerivingHandler` command and for each `deriving` clause below,
           emits nothing for them, and DOES emit LOGOS declarations for `Flag`, `Pair`,
           `Colour`, `Chain`, their constructors, the `Flag_tag` / `Pair_tag` definitions, the
           `MyTag` instances, and every instance Lean's built-in handlers generated for
           `Colour` and `Chain`. A translator that refuses any of those types has over-rejected
           and fails this case.
-/

namespace T09_006

open Lean Elab Command

/-- The class a custom handler will derive. -/
class MyTag (α : Type) where
  tag : α → Nat

/-- A CUSTOM DERIVING HANDLER: an arbitrary `CommandElabM` program. It reads the environment,
    chooses a name, and elaborates a fresh command. None of this is replayable in LOGOS. -/
def myTagHandler (declNames : Array Name) : CommandElabM Bool := do
  for declName in declNames do
    let base := declName.getString!
    let defName := Lean.mkIdent (Name.mkSimple (base ++ "_tag"))
    let tyName := Lean.mkIdent declName
    elabCommand (← `(
      def $defName : $tyName → Nat := fun _ => 0
      instance : MyTag $tyName := ⟨$defName⟩
    ))
  return true

initialize registerDerivingHandler ``MyTag myTagHandler

/-- An inductive that the handler is MEANT to serve. Lean's `initialize` registration takes
    effect only when this module is IMPORTED, never within the module that performs it (a
    `deriving MyTag` clause here reports "default handlers have not been implemented yet"), so
    the declarations the handler would generate are written out by hand below. That limitation
    is itself the subject of T09-007: `initialize` mutates a persistent environment extension,
    and LOGOS has no environment extensions at all. -/
inductive Flag where
  | up | down

/-- What `myTagHandler` emits for `Flag`. This is what the translator must produce — an
    ordinary definition plus an ordinary `add_instance` entry (context.rs:184). -/
def Flag_tag : Flag → Nat := fun _ => 0

instance : MyTag Flag := ⟨Flag_tag⟩

theorem generatedDefTranslates : Flag_tag Flag.up = 0 := rfl

theorem generatedInstanceTranslates : MyTag.tag Flag.up = 0 := rfl

/-- The same for a structure. -/
structure Pair where
  a : Nat
  b : Nat

def Pair_tag : Pair → Nat := fun _ => 0

instance : MyTag Pair := ⟨Pair_tag⟩

theorem pairGenerated : Pair_tag ⟨1, 2⟩ = 0 := rfl

/-- BUILT-IN deriving handlers, which are equally metaprograms. `BEq` generates a structurally
    recursive comparison; `Repr` generates `String`-producing code that LOGOS cannot express at
    all, because it has `Literal::Text` but no string operations. -/
inductive Colour where
  | red | green | blue
  deriving BEq, DecidableEq, Repr, Inhabited

theorem derivedBEq : (Colour.red == Colour.red) = true := rfl

theorem derivedDecEq : Colour.red ≠ Colour.green := by decide

theorem derivedInhabited : (default : Colour) = Colour.red := rfl

/-- A RECURSIVE inductive whose derived `BEq` is a structurally recursive function — the case
    where the generated declaration may be refused even though the type is fine. -/
inductive Chain where
  | stop : Chain
  | link : Nat → Chain → Chain
  deriving BEq, DecidableEq

theorem derivedRecursiveBEq : (Chain.link 1 Chain.stop == Chain.link 1 Chain.stop) = true := rfl

end T09_006
