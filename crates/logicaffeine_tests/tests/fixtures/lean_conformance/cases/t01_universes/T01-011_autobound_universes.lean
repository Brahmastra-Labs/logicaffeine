/-!
@case      T01-011
@tier      1 · Universes
@title     Auto-bound universe parameters — generated names, and a POSITIONAL level list
@lean      autoImplicit/relaxedAutoImplicit universe auto-binding, `def f (α : Type u)` with no
           `.{u}`, `Type _`, Mathlib's `Type*` (which expands to auto-bound `Type _`)
@expect    degrade
@target    `Context::add_universe_poly(name, params, ty, body)` (kernel/src/context.rs:218) with
           `params` equal to Lean's `ConstantInfo.levelParams` IN ORDER, plus a loss record
           naming AUTOBOUND_LEVEL_PROVENANCE. References are `Term::Const { name, levels }`
           (term.rs:458) with `levels` in the same positional order.
@evidence  kernel/src/context.rs:218-225 (`add_universe_poly` stores `params: Vec<String>`);
           kernel/src/type_checker.rs:83-85 (`let subst = params.iter().cloned()
           .zip(levels.iter().cloned()).collect();` — the parameter/level association is
           POSITIONAL, by zip, with no name matching on the caller's side);
           kernel/src/type_checker.rs:75-82 (arity mismatch is a `CertificationError`) versus
           kernel/src/reduction.rs:83-90 (the SAME mismatch in a reduction position returns the
           term unreduced, silently);
           kernel/src/term.rs:179 and :271 (`Universe::Var(String)`, compared by NAME through
           `to_lnat` — so level-variable names are a flat global namespace);
           kernel/src/context.rs:253, :374, :419, :434 (no universe-parameter list on inductives,
           constructors or declarations — so an auto-bound level on any of those has no binder;
           T01-003, T01-007);
           kernel/src/term.rs:256-265 (`Universe::substitute`, which `instantiate_universes`
           drives, replaces `Var(v)` by `subst[v]` and leaves an unmatched `Var` UNCHANGED —
           `.unwrap_or_else(|| self.clone())` at term.rs:262 — so a missing parameter is not an
           error, it is a silently uninstantiated level)
@loss      AUTOBOUND_LEVEL_PROVENANCE. Lean's auto-bound parameters are named by the elaborator
           (`u_1`, `u_2`, …), ordered by first occurrence, and their EXISTENCE is a consequence
           of elaboration settings (`autoImplicit`, `relaxedAutoImplicit`) rather than of the
           source text. The generated names carry no meaning and are not stable across Lean
           versions or across edits to the signature. Emitting them verbatim into
           `add_universe_poly`'s `params` is correct but records nothing about where they came
           from, so a later re-translation that produces a different generated name yields a
           different LOGOS declaration for an unchanged source. Acceptable because the names are
           internal to `add_universe_poly`'s substitution; the loss must be recorded so that
           declaration-level diffs can be attributed.
@guard     Catches deriving the parameter ORDER from the TYPE rather than from
           `ConstantInfo.levelParams`. `type_checker.rs:83-85` zips positionally, so the order is
           the contract. Lean orders auto-bound parameters by first occurrence in the elaborated
           signature, which is NOT the order they appear in the surface syntax once implicit
           binders and instance arguments have been inserted and reordered. A swapped pair is
           undetectable whenever the level expression is symmetric (`max u v`, T01-004) and
           silently wrong where it is not.
@guard     Catches the FLAT NAMESPACE of generated names. Every auto-bound declaration in a Lean
           file gets its own `u_1`; `Universe::Var` compares by name (term.rs:271), so a
           translator that copies the names verbatim makes the first universe parameter of EVERY
           auto-bound declaration in the corpus the same level variable. Inside one declaration
           this is harmless (the substitution is per-`Const`); across declarations it makes
           `Universe::equiv` report two unrelated levels equal, weakening every constraint the
           source imposed. Names must be freshened per declaration.
@guard     Catches a MISSING parameter being silent. `Universe::substitute` (term.rs:260-263)
           leaves an unmatched `Var` unchanged rather than erroring, so if `params` omits a
           variable that occurs in `ty`, `instantiate_universes` returns a type still containing
           the free `Var` — and `infer_sort` will accept it, and `level_leq` will answer
           questions about it. The declaration is registered with a level that no instantiation
           can ever fix. The arity check at type_checker.rs:75-82 does NOT catch this: it
           compares `params.len()` to `levels.len()`, not `params` to the variables actually
           occurring.
@guard     Catches auto-binding on a NON-definition. Lean will auto-bind a universe on an
           `inductive`, a `structure`, an `axiom` or an `instance` just as readily as on a `def`,
           and LOGOS has a binder for none of them (context.rs:253, :374, :419, :434). An
           auto-bound `inductive` is T01-007's rejection reached by accident — the source never
           wrote a universe parameter, so a translator that keys its polymorphic-inductive check
           on "did the source write `.{u}`" will miss it entirely. The check must be on
           `levelParams.isEmpty`, not on the surface syntax.
@guard     Catches `Type _` being treated as `Type 0`. Lean elaborates the `_` to a level
           metavariable and then either solves it from the context or AUTO-BINDS it. Which of
           those happened is only visible in the finished `ConstantInfo`; a translator that reads
           the pre-elaboration `Syntax`, or that defaults an unsolved level, monomorphises a
           declaration Lean made polymorphic. An unsolved level that is NOT auto-bound is
           T01-012's rejection.
@guard     Catches assuming `set_option autoImplicit false` removes the issue. It removes
           auto-binding of TERM-level implicits; universe auto-binding is governed separately and
           `def f (α : Type u)` still binds `u` under the default settings. The translator must
           read the elaborated declaration, never infer the setting.
@oracle    For every declaration, the emitted `params` equals Lean's `ConstantInfo.levelParams`
           as an ORDERED list, modulo a per-declaration freshening that is injective across the
           corpus; every `Universe::Var` occurring in the emitted type or body is a member of
           that declaration's `params`; every reference is a `Term::Const` whose `levels.len()`
           matches; and a loss record with category AUTOBOUND_LEVEL_PROVENANCE lists each
           declaration whose universe parameters were auto-bound rather than written.
-/

namespace T01_011

/-- Auto-bound: the source never declares `u`, and Lean binds it. `levelParams = [u]`. -/
def autoId (α : Type u) : Type u := α

/-- Auto-bound from `Sort u`, so the parameter ranges over `Prop` as well. -/
def autoSortId {α : Sort u} (a : α) : α := a

theorem autoAtProp (h : True) : True := autoSortId h
def autoAtType (n : Nat) : Nat := autoSortId n

/-- TWO auto-bound parameters. Their order is Lean's first-occurrence order in the ELABORATED
    signature, which is the order `add_universe_poly`'s `params` must use. Symmetric use, so a
    swap is invisible. -/
def autoPair (α : Type u) (β : Type v) : Type (max u v) := α → β

/-- The same two, used ASYMMETRICALLY — the control that detects a swap. -/
def autoSecond (_α : Type u) (β : Type v) : Type v := β

def autoSecondAt01 : Type 1 := autoSecond Nat Type
def autoSecondAt10 : Type := autoSecond Type Nat

/-- Parameters whose FIRST OCCURRENCE order differs from their order in the result type: `v`
    occurs first in the binders, `u` first in the codomain. Lean's order comes from the binders. -/
def occurrenceOrder (_β : Type v) (_α : Type u) : Type (max (u + 1) (v + 1)) := Type u → Type v

/-- `Type _`: the level is a metavariable that Lean solves or auto-binds. Which one happened is
    visible only in the elaborated declaration. -/
def underscoreLevel (α : Type _) : Type _ := α

/-- `Sort _` in a position where the level IS determined by a later argument, so Lean solves
    rather than auto-binds. The two look identical in source. -/
def solvedLevel (α : Sort _) (a : α) : α := a
def usesSolvedLevel : Nat := solvedLevel Nat 3

/-- An auto-bound universe on an INDUCTIVE. The source writes no `.{u}` at all, so a translator
    that gates its polymorphic-inductive rejection on the surface syntax will emit this one.
    `levelParams` is non-empty; the check must be on that. (T01-007.) -/
inductive AutoBox (α : Type u) : Type u where
  | mk : α → AutoBox α

def autoBoxNat : AutoBox Nat := AutoBox.mk 3
def autoBoxType : AutoBox Type := AutoBox.mk Nat

/-- An auto-bound universe on a STRUCTURE, which `add_structure` (context.rs:271) cannot express
    at all: it pins the sort to `Type 0` (context.rs:300) and takes no parameter list. -/
structure AutoCell (α : Type u) : Type u where
  val : α

def autoCellNat : AutoCell Nat := ⟨3⟩

/-- A universe parameter on an AXIOM. Lean does NOT auto-bind here — the parameter must be
    written — which is itself a fact the translator must know, because it means a bare `Type u`
    in an `axiom` is an error in Lean and never reaches translation. Once written, there is
    still no LOGOS binder: `add_declaration` (context.rs:434) takes no parameter list. -/
axiom autoAxiom.{u} {α : Type u} : α → Prop

def usesAutoAxiom : Prop := autoAxiom (3 : Nat)

end T01_011
