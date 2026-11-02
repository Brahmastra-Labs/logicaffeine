import Lean

/-!
@case      T09-005
@tier      9 · Metaprogramming and syntax
@title     Syntax quotation and antiquotation — `Syntax` is a Lean-runtime data type with no
           LOGOS representation, and `?` cannot even be lexed by the script surface
@lean      `(…) quotation, $x antiquotation, `(tactic| …), Syntax.mkApp, TSyntax
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/term.rs:443-545 (`Term` — thirteen variants, none of which is a
           representation of syntax. `Term::Lit` (term.rs:13-44) holds `Int`, `Float`, `Text`,
           `Duration`, `Date`, `Moment`, `BigInt`, `Nat` — a quoted `Syntax` tree is none of
           these, and encoding one as `Literal::Text` would be a string, not a tree);
           kernel/src/prelude.rs (the registered inductives are `Entity`, `Int`, `Float`,
           `Text`, `Duration`, `Date`, `Moment`, `Nat`/`Zero`/`Succ`, `EList`/`ENil`/`ECons`,
           `Bool`, `TList`, `True`/`I`, `False`, `Decidable`, `Eq`, `Acc`, `And`, `Or`, `Ex`,
           and the hardware types — there is no `Syntax` inductive and no `Name` type);
           kernel/src/interface/literate_parser.rs:631-635, :2277-2316
           (the ONE place LOGOS has something called `Syntax`: `try_ring` has type
           `Syntax → Derivation` (prelude.rs:2984-2990) and the literate parser REIFIES a goal
           into it with `term_to_syntax`. That `Syntax` is a fixed reflection of a GOAL for the
           tactic reducers — `SName`, `SApp` — not a general parse tree, and nothing constructs
           one from user text);
           proof/src/tactic_script.rs:51-105 (the script lexer: `is_alphanumeric() || '_'` for
           words, plus `;.,()[]|` and `<;>`. The character `?` falls to the `other =>` arm at
           :101 and produces `ScriptError("unexpected character '?'")` — an antiquotation
           cannot be TOKENISED, let alone represented);
           kernel/src/elaborate.rs:137-140 (`is_meta` — LOGOS's own `?`-prefixed names are
           ELABORATOR metavariables in `Term::Var`, a completely different thing from an
           antiquotation, and they must be fully instantiated before the kernel runs)
@diag      E_NO_SYNTAX_REPRESENTATION — must name the quotation, state that LOGOS has no
           `Syntax` type in its term language or its prelude, and state that the `Syntax`-typed
           VALUE is what is unrepresentable (not the term the quotation eventually produces).
           Must emit no declaration for anything whose type mentions `Lean.Syntax` or `TSyntax`.
@guard     THE `?`-COLLISION TRAP. LOGOS uses a `?` prefix on `Term::Var` names for its own
           elaborator metavariables (elaborate.rs:137-140). A translator that sees Lean's
           antiquotation `$x` or a named hole `?h` and reaches for that convention produces a
           `Term::Var("?x")` — which the LOGOS elaborator will then try to SOLVE by unification,
           and which must be fully instantiated before the kernel sees it. An antiquotation is
           not a metavariable: it is a splice point in a parse tree. Encoding one as the other
           means the elaborator will either solve it to something arbitrary or leave it, at
           which point it reaches the kernel as an unbound named variable.
@guard     Catches encoding a quotation as `Literal::Text`. `Term::Lit(Literal::Text(..))`
           (term.rs:13-44) will hold the source text, and it will typecheck as a `Text`. It is
           not a syntax tree: nothing can traverse it, splice into it, or elaborate it, and
           LOGOS has NO string operations at all (see T10-016), so it cannot even be
           manipulated. A translator that does this has smuggled uninterpretable data into a
           proof term under a type that claims it is a string.
@guard     Catches confusing Lean's `Syntax` with LOGOS's. `try_ring : Syntax → Derivation`
           (prelude.rs:2984-2990) makes it look as though LOGOS has a syntax type to target.
           It does not have a general one: `term_to_syntax` (literate_parser.rs) reifies a
           kernel `Term` into a fixed `SName`/`SApp` encoding so the tactic REDUCERS can pattern
           match on a goal (reduction.rs:2282-2316). It represents propositions, not programs,
           and there is no constructor for a binder, a tactic block, or an antiquotation.
@guard     Catches the scope of the rejection, as in T09-004. A quotation's eventual OUTPUT —
           the term it builds once spliced and elaborated — is ordinary and translates. What is
           rejected is any declaration whose TYPE or BODY mentions `Syntax`/`TSyntax` as a
           value, because that value has no LOGOS counterpart at any stage.
@guard     Catches `` `(tactic| …) `` specifically. A quoted TACTIC is a value of type
           `TSyntax `tactic`, and the LOGOS script language is a STRING parsed at run time
           (tactic_script.rs:330-353) with no reified form. The nearest analogue,
           `TacticEnv::define(name, script_source)` (tactic_script.rs:123-125), stores text and
           re-expands it textually — it cannot be constructed programmatically from parts, and
           has no antiquotation.
@note      `import Lean` (core, not Mathlib) is required to write a quotation at all; see the
           @note on T09-004 about the metadata block's position.
@oracle    Translator exits non-zero with E_NO_SYNTAX_REPRESENTATION for every declaration below
           whose type mentions `Lean.Syntax` or `Lean.TSyntax`, and emits no `Term::Lit` whose
           `Literal::Text` payload is Lean source text. A harness additionally asserts that no
           emitted `Term::Var` name begins with `?` anywhere in the corpus.
-/

namespace T09_005

open Lean

/-- A quotation as a first-class VALUE — its type is `Syntax`, which has no LOGOS counterpart. -/
def quotedTerm : MetaM Syntax := do
  return (← `(1 + 1))

/-- A quotation with an ANTIQUOTATION splice. The `$x` is a hole in a parse tree, not a
    metavariable. -/
def spliced (x : Term) : MetaM Syntax := do
  return (← `($x + $x))

/-- A quoted TACTIC — a `TSyntax `tactic` value. -/
def quotedTactic : MetaM (TSyntax `tactic) := do
  return (← `(tactic| rfl))

/-- Building syntax programmatically rather than by quotation — same unrepresentable type. -/
def builtSyntax : Syntax :=
  Syntax.node SourceInfo.none `myNode #[Syntax.ident SourceInfo.none "f".toSubstring `f []]

/-- A function that CONSUMES syntax — so even its Pi type mentions the absent type. -/
def countArgs (s : Syntax) : Nat := s.getNumArgs

/-- A quotation used the ordinary way, inside a macro: the quotation itself is rejected, and
    the term it expands to is ordinary and must translate. -/
macro "twoPlusTwo" : term => `(2 + 2)

theorem expansionTranslates : twoPlusTwo = 4 := rfl

/-- The same for a tactic quotation used inside a macro. -/
macro "justRfl" : tactic => `(tactic| rfl)

theorem tacticExpansionTranslates (n : Nat) : n = n := by justRfl

end T09_005
