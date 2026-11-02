import Lean

/-!
@case      T09-004
@tier      9 · Metaprogramming and syntax
@title     `syntax` + `elab` — an arbitrary `MetaM` program; the metaprogram is rejected, its
           OUTPUT is ordinary
@lean      syntax + elab, elab_rules, custom term elaborators, custom tactic elaborators
@expect    reject
@target    (none for the metaprogram — no sound translation)
@evidence  kernel/src/term.rs:443-545 (`Term` — the complete target language: `Sort`, `Var`,
           `Global`, `Const`, `Pi`, `Lambda`, `App`, `Match`, `Fix`, `MutualFix`, `Let`, `Lit`,
           `Hole`. There is no computation-at-elaboration-time construct, no `MetaM`, no
           tactic-as-term, and no reflection of the `Context` back into the term language);
           kernel/src/context.rs:20-80 (`Context` holds only inductives, constructors,
           declarations, definitions, universe polys, instances, coercions and binder kinds —
           all flat `HashMap<String, _>`. There is no elaborator-extension table and no way to
           register a Rust closure against a syntactic form);
           kernel/src/elaborate.rs:1016-1027, :1030-1050 (`ParamKind` and `elaborate_app` —
           LOGOS's elaborator is a FIXED algorithm: insert metavariables for implicits, defer
           instance resolution until the explicit arguments are unified, then solve. It exposes
           no registration API);
           proof/src/tactic_script.rs:113-133 (`TacticEnv` — the ONLY user extension in the
           system: a `HashMap<String, String>` from a name to SCRIPT SOURCE TEXT, expanded
           textually and depth-bounded at `MAX_TACTIC_DEPTH = 128`. It cannot run code, inspect
           a goal, or construct a term);
           proof/src/tactic.rs:978 (`pub type Tactic = Box<dyn Fn(&mut ProofState) ->
           Result<(), TacticError>>` — Rust closures ARE first-class tactics, but only inside
           the Rust process; nothing serialises one, so no LOGOS artefact can carry it)
@diag      E_METAPROGRAM_NOT_REPLAYABLE — must name the syntax category and the elaborator,
           state that the elaborator is an arbitrary `MetaM` program with no LOGOS counterpart,
           and — importantly — state that the DECLARATIONS THE ELABORATOR PRODUCES are not
           themselves rejected. Must emit nothing for the `syntax`/`elab` pair.
@guard     THE SCOPE-OF-REJECTION GUARD, and the nuance this case exists to fix. It is easy to
           over-read this rejection as "any file using a custom elaborator is untranslatable".
           That is wrong, and would make the translator useless on real Lean. What cannot be
           translated is the METAPROGRAM — the `elab` body, which is Lean code running in
           `MetaM` with access to the environment, the local context, unification and IO. What
           the metaprogram EMITS is an ordinary `Expr`, and it translates by the ordinary rules.
           A translator must therefore reject the `syntax`+`elab` commands and CONTINUE,
           translating every declaration whose elaborated term happens to have been produced by
           them. `usesCustomElab` below is the test: its `Expr` is a plain `Nat` literal and
           must translate.
@guard     Catches the over-generalisation running the other way. Inlining the elaborator's
           output works only because Lean already ran the elaborator. A translator asked to
           process a Lean FILE rather than an elaborated environment cannot run it — `MetaM`
           needs the whole Lean runtime — so it must refuse the file rather than guess.
           **Translate the post-elaboration `Expr`, never the surface `Syntax`**: an
           `Expr`-driven translator never faces this choice.
@guard     Catches mapping `elab` onto `TacticEnv::define` (tactic_script.rs:123-125). That
           registry stores SCRIPT TEXT and expands it textually with a depth bound; an `elab`
           body is a program. Registering the elaborator's NAME there with some plausible script
           produces a LOGOS tactic that runs something the Lean source never specified, under a
           name that suggests it does.
@guard     Catches elaborators that are NOT deterministic functions of their syntax. An `elab`
           body may read the environment (`getEnv`), the local context, the message log, or
           `IO` — so the same syntax elaborates differently in different positions.
           `usesEnvReadingElab` below does exactly this. Nothing about it is recoverable from
           the syntax alone, which is why the rejection is of the metaprogram rather than a
           deferral.
@guard     Catches `elab_rules : tactic`. A custom TACTIC elaborator produces no term at all: it
           manipulates goals. LOGOS's script surface has a closed set of fifteen primitives
           (tactic_script.rs:275-292, see T08-003 and T08-020), so even a tactic elaborator
           whose effect is simple has no spelling — and its EFFECT, the proof term, still
           translates.
@note      This file opens with `import Lean`, which is part of the Lean 4 CORE distribution
           (not Mathlib) and is required to write any elaborator at all. Lean requires `import`
           to be the first command in a file, so the metadata block sits immediately after it
           rather than on line 1. The same applies to T09-005, T09-006, T09-007 and T09-012.
@oracle    Translator exits non-zero with E_METAPROGRAM_NOT_REPLAYABLE for each `syntax`/`elab`
           pair below, emits nothing for them, and DOES emit a correct LOGOS declaration for
           `usesCustomElab`, `usesEnvReadingElab`, `usesCustomTactic`, `usesBranchingElab` and
           `usesBranchingElabOdd` — whose elaborated `Expr`s contain no trace of the
           metaprogram. A translator that refuses those has over-rejected and fails this case.
-/

namespace T09_004

open Lean Elab Term Tactic

/-- A custom syntax-category member and a term elaborator for it. The elaborator is an
    arbitrary `MetaM` program; here it merely builds a literal, but nothing in the language
    restricts it to that. -/
elab "theAnswer" : term => return mkNatLit 42

/-- The elaborator's OUTPUT is an ordinary `Nat` literal and must translate. -/
theorem usesCustomElab : theAnswer = 42 := rfl

/-- An elaborator that READS THE ENVIRONMENT, so its result is not a function of its syntax. -/
elab "envIsNonEmpty" : term => do
  let env ← getEnv
  return mkNatLit (if env.contains ``Nat then 1 else 0)

theorem usesEnvReadingElab : envIsNonEmpty = 1 := rfl

/-- A custom TACTIC elaborator: it produces no term, it manipulates the goal. -/
elab "closeWithRfl" : tactic => do
  evalTactic (← `(tactic| rfl))

/-- The tactic's EFFECT is an ordinary proof term, which translates. -/
theorem usesCustomTactic (n : Nat) : n = n := by closeWithRfl

/-- An `elab` whose body BRANCHES on the argument's value — a program, not a rewrite. -/
elab "doubleIfEven " n:num : term => do
  let v := n.getNat
  return mkNatLit (if v % 2 == 0 then v * 2 else v)

theorem usesBranchingElab : doubleIfEven 4 = 8 := rfl

theorem usesBranchingElabOdd : doubleIfEven 5 = 5 := rfl

/-- A DECLARATION whose body mixes elaborator output with ordinary terms — the realistic case,
    where the translator must keep going rather than refuse the whole file. -/
def mixed (k : Nat) : Nat := k + theAnswer

theorem mixedTranslates : mixed 8 = 50 := rfl

end T09_004
