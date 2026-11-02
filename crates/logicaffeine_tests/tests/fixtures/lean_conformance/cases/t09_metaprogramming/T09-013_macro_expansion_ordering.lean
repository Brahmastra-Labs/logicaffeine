/-!
@case      T09-013
@tier      9 · Metaprogramming and syntax
@title     Macro expansion ORDERING — the integration case for the whole tier: read the
           post-elaboration `Expr` and the order stops existing
@lean      interleaved notation/macro_rules/infix, redefinition, scoped and local macros,
           expansion order vs. elaboration order
@expect    translate
@target    The translation of the final elaborated `Expr` for each declaration. Every command
           that only extends the PARSER — `notation`, `infixl`/`infixr`/`prefix`/`postfix`,
           `macro`, `macro_rules`, `syntax` — produces NO LOGOS declaration.
@evidence  kernel/src/term.rs:443-545 (`Term` — no `Syntax`, no expansion record, no ordering
           information: two declarations whose surface forms expanded differently are simply two
           different terms, with nothing marking why);
           kernel/src/context.rs:20-80 (`Context` — flat maps, populated by a LINEAR sequence of
           `add_*` calls. There is no notion of "this declaration was elaborated under that
           notation scope", and no re-elaboration);
           kernel/src/context.rs:449 (`get_global` — lookup precedence is inductives,
           constructors, definitions, declarations: a fixed order over a flat namespace,
           nothing to do with source order);
           kernel/src/elaborate.rs:1030-1050 (`elaborate_app` — LOGOS elaboration consumes and
           produces `Term`; there is no surface stage for an ordering to matter in);
           proof/src/tactic_script.rs:250-260 (the ONE place LOGOS has expansion ordering at
           all: a user-defined tactic takes PRECEDENCE over a built-in alias and is expanded in
           place, depth-bounded at 128 (:133). That is a textual tactic macro, and its ordering
           rule — "an explicit `define` is authoritative, so a user tactic may even shadow a
           built-in alias" — is LOGOS's own, unrelated to Lean's)
@guard     THE TIER'S GOVERNING RULE, stated as its own case: **translate the post-elaboration
           `Expr`, never the surface `Syntax`.** Every expansion-ordering question in this file
           — which `macro_rules` alternative wins, whether an `infixr` regroups, whether a
           `local` notation is in scope, whether a redefinition shadows — is a question about
           Lean's parser and macro expander. A translator that reads `Expr` never asks any of
           them. A translator that reads `Syntax` must answer all of them correctly, and every
           wrong answer produces a well-typed LOGOS term that means something else. This case
           exists so a conformance report can state which kind of translator is under test.
@guard     Catches LATER-BLOCK-WINS. A second `macro_rules` for the same syntax PREPENDS its
           alternatives, so the later block is tried FIRST. A translator that expands
           alternatives in source order gets the opposite answer. `laterWins` below is the
           witness, and it is a plain `Nat` equation, so the wrong answer is not a type error.
@guard     Catches REBINDING of a token. `firstMeaning` and `redefinedMeaning` below use the
           SAME token `⊞` and elaborate to `op` and to `alt` respectively, because each sits in
           a section with its own `local notation`. In the `Expr` this is simply two different
           globals with nothing recording the token; in `Syntax` it is a scoping question whose
           answer depends on where in the file each use sits. (Two notations for one token at
           the SAME scope level do not shadow in Lean 4 — they overload, and a use is then
           rejected as "ambiguous, possible interpretations"; the sectioned form above is how a
           real file rebinds a token, and it is what the translator will actually meet.)
@guard     Catches SCOPE. `local notation` inside a `section` is out of scope after `end`, and
           `scoped notation` inside a `namespace` requires `open`. Neither affects the `Expr` of
           anything already elaborated. A translator that models notation scope will get it
           subtly wrong at the `open` boundary; one that reads `Expr` has nothing to model.
@guard     Catches ELABORATION order being confused with EXPANSION order. Macros expand during
           parsing; elaboration happens afterwards and may postpone (instance resolution,
           `OfNat` literals, coercions — see T05-015, T05-017, T05-011). Two declarations can
           expand identically and elaborate differently because of a `variable` or an `open`
           between them. The `Expr` is the only artefact that records the outcome of BOTH
           stages, which is why it and not the expansion is the translation input.
@guard     Catches emitting anything for the parser-extension commands. Each `notation`,
           `infixl`, `macro` and `syntax` below must produce ZERO LOGOS declarations — see
           T09-001, T09-002, T09-003. A translator that emits a placeholder per command will
           produce as many phantom globals as there are notations in the file, in a flat
           namespace where any of them can collide (T02-020).
@oracle    The translator emits declarations for exactly `op`, `alt`, `usesVariable` and the
           eight theorems below — and for nothing else. For each theorem, the emitted LOGOS term
           is `types_equal` to the
           translation of the hand-expanded form given in the same theorem's right-hand side.
           A harness additionally asserts that the number of emitted declarations is unchanged
           when every `notation`/`macro`/`infix` command in the file is replaced by its
           expansion at each use site.
-/

namespace T09_013

def op (a b : Nat) : Nat := a + b * 2
def alt (a b : Nat) : Nat := a * 10 + b

-- THE SAME TOKEN, TWO MEANINGS, in two sections. The parser sees a scoping question; the
-- `Expr` sees `op` in one theorem and `alt` in the other, with nothing recording why.

section First

local notation:65 a " ⊞ " b => op a b

/-- Uses the FIRST meaning of `⊞`. -/
theorem firstMeaning (x y : Nat) : (x ⊞ y) = op x y := rfl

end First

section Second

local notation:65 a " ⊞ " b => alt a b

/-- Uses the SECOND meaning. One token, two globals, two different terms. -/
theorem redefinedMeaning (x y : Nat) : (x ⊞ y) = alt x y := rfl

end Second

-- Two `macro_rules` blocks for one syntax: the LATER block is tried FIRST.
syntax:max "bump " term:max : term

macro_rules
  | `(bump $x) => `($x + 1)

macro_rules
  | `(bump $x) => `($x + 1000)

/-- The later block wins. A source-order expander would produce `6`. -/
theorem laterWins : bump 5 = 1005 := rfl

-- A SCOPED notation: in scope only after `open`.
namespace Inner

scoped notation:70 a " ⊠ " b => alt a b

theorem insideNamespace (x y : Nat) : (x ⊠ y) = alt x y := rfl

end Inner

open Inner in
/-- Outside the namespace the notation needs an explicit `open`. -/
theorem scopedUse (x y : Nat) : (x ⊠ y) = alt x y := rfl

-- A LOCAL fixity, confined to its section, and RIGHT-associative (see T09-002).
section Local

local infixr:75 " ▷ " => op

theorem insideSection (x y z : Nat) : (x ▷ y ▷ z) = op x (op y z) := rfl

end Local

-- INTERLEAVED: a macro whose expansion uses a notation defined earlier, applied to an argument
-- that is itself a notation use. Expansion is nested; the `Expr` is flat.
notation:65 a " ⊡ " b => alt a b

macro:max "twice " e:term:max : term => `(($e) ⊡ ($e))

/-- Expands to `alt (alt x y) (alt x y)`. -/
theorem interleaved (x y : Nat) : (twice (x ⊡ y)) = alt (alt x y) (alt x y) := rfl

-- A `variable` between two uses changes ELABORATION, not expansion — the second stage the
-- `Expr` also records.
section Variables

variable (k : Nat)

def usesVariable : Nat := k ⊡ k

end Variables

theorem variableInserted (k : Nat) : usesVariable k = alt k k := rfl

end T09_013
