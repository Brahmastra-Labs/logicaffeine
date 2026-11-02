/-!
@case      T01-012
@tier      1 · Universes
@title     Level metavariables — `Universe` has no metavariable form; levels must be EXPLICIT
@lean      Lean.Level.mvar, `Type _`, `Sort _`, universe unification, instantiateLevelMVars
@expect    reject
@target    (none — an unresolved universe level has no LOGOS representation)
@diag      E_UNRESOLVED_LEVEL — must name the declaration, the position, and the Lean
           `Level.mvar` identifier. Must NOT default the level, must NOT turn it into a
           `Universe::Var`, and must write NO declaration.
@evidence  kernel/src/term.rs:167-191 (the complete `Universe` enum: `SProp`, `Prop`,
           `Type(u32)`, `Var(String)`, `Succ`, `Max`, `IMax` — there is NO metavariable variant,
           and no `Hole`-like wildcard at the level layer);
           kernel/src/term.rs:458 (`Term::Const { name, levels: Vec<Universe> }` — the level list
           is a concrete `Vec`, so every entry must be a fully-formed `Universe`);
           kernel/src/type_checker.rs:75-85 (`Const` inference requires `params.len() ==
           levels.len()` and then substitutes — there is no unification step and no constraint
           store anywhere in the kernel);
           kernel/src/elaborate.rs:137-140 (LOGOS's own metavariables are `Term::Var` names
           beginning with `?` — a TERM-level convention that has no level-level analogue, so
           there is not even a hack available);
           kernel/src/term.rs:266-276 (`to_lnat`, with the doc at :270-272: "every universe
           VARIABLE ranges over all of ℕ (so a variable may be `Prop`, which is what closes the
           `Sort u := Nat` unsoundness — `Type 0 ≤ u` is false, since `u` could be `Prop`)");
           kernel/src/term.rs:256-265 (`Universe::substitute`: an unmatched `Var` is returned
           UNCHANGED at :262, so a level variable that no binder covers is silently retained)
@guard     Catches `Level.mvar ⟼ Universe::Var(name)`. These are not the same thing and the
           difference is a quantifier. A metavariable is an EXISTENTIAL — "there is some level
           making this well-typed, not yet determined". A `Universe::Var` bound by
           `add_universe_poly` (context.rs:218) is a UNIVERSAL — "for every level". Turning the
           first into the second strengthens the declaration: it claims the definition works at
           EVERY level, including `Prop` (term.rs:270-272 is explicit that a variable ranges over
           `Prop`). LOGOS will not catch the strengthening, because a universally quantified
           declaration is checked only at its parameters, and `level_leq` is sound for variables
           by case-splitting — so the declaration typechecks and asserts more than Lean ever
           proved.
@guard     Catches `Level.mvar` left as a FREE `Universe::Var` with no binder. `Universe::substitute`
           (term.rs:260-263) returns an unmatched `Var` unchanged rather than erroring, and
           `infer_sort` (type_checker.rs:520-525) will return it happily. So the declaration is
           registered carrying a level that no `Term::Const` can ever instantiate, and every
           `level_leq` question about it is answered by case analysis over a variable that means
           nothing. Nothing errors at any point.
@guard     Catches DEFAULTING to `Universe::Type(0)`. This is not a conservative approximation in
           either direction. Too strong: it fixes `u := Type 0`, so any use at a higher level is
           rejected — and, because LOGOS is cumulative (type_checker.rs:1330-1336), a great many
           of those uses are NOT rejected, they are silently absorbed upward, collapsing distinct
           instantiations onto one declaration (T01-007, T01-008). Too weak: it excludes the
           `Prop` instantiation that `Sort _` may have been solved to, so a proof-level use has no
           image.
@guard     Catches reading the level BEFORE `instantiateLevelMVars`. Every `Type _`/`Sort _`
           below carries a live `Level.mvar` during elaboration; Lean solves them by unification
           against a later argument or a declared type, or auto-binds them (T01-011). A
           translator hooked in before that point sees metavariables in declarations whose
           finished `ConstantInfo` has none. This is the level-layer twin of T00-010: read the
           FINISHED declaration, run `instantiateLevelMVars`, and assert the result is
           mvar-free — `Expr.hasLevelMVar` is the check, and it must be applied to the type AND
           the value.
@guard     Catches "the level is obviously determined, so infer it myself". LOGOS has no
           constraint store and no level unifier — `type_checker.rs:83-85` performs a
           SUBSTITUTION, not a unification — so any level the translator infers is the
           translator's own claim, checked by nothing downstream. Lean's level solver has
           already run and either succeeded (in which case the answer is in the `ConstantInfo`)
           or failed (in which case the Lean declaration does not exist). There is no third case
           in which the translator should be solving levels.
@guard     Catches a metavariable surviving in a `Term::Const`'s `levels`. The arity check at
           type_checker.rs:75-82 counts entries; it does not inspect them. A `levels` vector of
           the right LENGTH containing a defaulted or invented level passes, and
           `instantiate_universes` (term.rs:256-265) substitutes it into the type without
           complaint. The emitted reference then has a concrete, wrong type.
@oracle    After `instantiateLevelMVars`, every translated declaration's type and value satisfy
           `!hasLevelMVar`; any that does not exits non-zero with E_UNRESOLVED_LEVEL and writes
           nothing. Every emitted `Universe::Var(n)` is a member of its declaration's
           `add_universe_poly` `params` (no free level variables anywhere in the emitted
           context); and no emitted level was chosen by the translator rather than read from
           Lean's elaborated declaration.
-/

namespace T01_012

/-- `Type _` whose level Lean SOLVES from the declared result type. During elaboration this is a
    `Level.mvar`; in the finished declaration it is `Type 0`. -/
def solvedFromResult (α : Type _) : Type := α
def usesSolvedFromResult : Type := solvedFromResult Nat

/-- `Sort _` whose level is solved from a LATER argument, so the metavariable outlives its own
    position and a left-to-right translator walks past it unsolved. -/
def solvedFromLaterArg (α : Sort _) (a : α) : α := a
def usesAtType : Nat := solvedFromLaterArg Nat 3
def usesAtProp : True := solvedFromLaterArg True True.intro

/-- `Type _` that Lean AUTO-BINDS rather than solving (T01-011). Source-identical to the solved
    cases; only the finished `ConstantInfo` distinguishes them. -/
def autoBoundNotSolved (α : Type _) : Type _ := α

/-- A level determined by UNIFICATION against a polymorphic callee: the caller writes nothing,
    and the level comes out of matching `polyId`'s type against the expected one. -/
def polyId.{u} {α : Sort u} (a : α) : α := a

def levelByUnificationAtZero : Nat := polyId 3
def levelByUnificationAtOne : Type := polyId Nat
theorem levelByUnificationAtProp : True := polyId True.intro

/-- Nested: the outer call's level is determined by the inner call's, which is determined by the
    literal. Two metavariables chained. -/
def nestedLevelSolving : Nat := polyId (polyId (polyId 3))

/-- A level solved through an IMPLICIT type argument that is itself solved by unification — the
    level and the type metavariable are interdependent, so neither can be read before both are
    instantiated. -/
def constFn.{u, v} {α : Sort u} {β : Sort v} (a : α) (_b : β) : α := a
def interdependent : Nat := constFn 3 true

/-- A level appearing in a TYPE ASCRIPTION, which is erased (T00-020) — so the ascription that
    pinned the level leaves no trace in the emitted term, and the level must have been read
    before erasure. -/
def levelPinnedByAscription : Type 1 := (ULift.{1, 0} Nat : Type 1)

/-- `Sort _` in a position where BOTH `Prop` and `Type 0` would typecheck in isolation; Lean's
    solver picks one. A translator that defaults would have picked the other. -/
def ambiguousInIsolation (α : Sort _) : Sort _ := α
def ambiguousAtProp : Prop := ambiguousInIsolation True
def ambiguousAtType : Type := ambiguousInIsolation Nat

end T01_012
