/-!
@case      T09-002
@tier      9 · Metaprogramming and syntax
@title     `infixl` / `infixr` / `prefix` / `postfix` — associativity is resolved into the
           `Expr` spine before translation
@lean      infixl, infixr, infix, prefix, postfix, associativity and precedence
@expect    translate
@target    The translation of the EXPANDED application spine. The command itself produces NO
           LOGOS declaration; `a ⋄ b ⋄ c` under `infixl` is
           `App(App(Global("f"), App(App(Global("f"), a), b)), c)` and under `infixr` it is
           `App(App(Global("f"), a), App(App(Global("f"), b), c))`
           (kernel/src/term.rs:459, left-nested `App`).
@evidence  kernel/src/term.rs:459 (`App(Box<Term>, Box<Term>)` — binary and left-nested; a
           multi-argument application is a SPINE, and the spine shape is the only record of how
           the arguments were grouped);
           kernel/src/term.rs:443-545 (no `Syntax`, no fixity, no associativity anywhere in the
           term language); kernel/src/context.rs:20-80 (no fixity table in `Context`);
           kernel/src/interface/literate_parser.rs:1223-1270 (LOGOS's only infix support is a
           FIXED table in the literate parser — `<= ≤ >= ≥ < > + - *` plus the word `equals` —
           with hard-coded associativity and no extension point);
           proof/src/tactic_script.rs:51-105 (the tactic-script lexer has no operator tokens at
           all beyond `<;>`, so no user fixity is expressible there either);
           proof/src/formula.rs:427-432 (Surface B's `parse_term` accepts only a bare
           identifier — no operators, user-defined or otherwise)
@guard     THE ASSOCIATIVITY TRAP, and the reason this case is separate from T09-001. `infixl`
           and `infixr` produce DIFFERENT `Expr`s for the identical character sequence
           `a ⋄ b ⋄ c`. A translator that reads surface `Syntax` must carry the fixity
           declaration forward and apply it; if it guesses left-associativity (the common
           default), every `infixr` operator in the source is regrouped — and since most
           interesting operators are not associative, the resulting LOGOS term denotes a
           different value. It still typechecks. **Translate the post-elaboration `Expr`, never
           the surface `Syntax`** makes the trap unreachable: the spine already has the right
           shape.
@guard     Catches PRECEDENCE between two user operators. `assocDiffers` and `mixedPrecedence`
           below are the witnesses: the same token sequence groups differently depending on the
           declared precedences, and only the elaborated `Expr` records the outcome.
@guard     Catches `prefix` and `postfix` colliding in the flat namespace. Both expand to an
           application of a named function; the NAME is what survives into LOGOS's flat
           `HashMap<String, _>` keys (context.rs:20-80). Two notations for the same function are
           fine; two functions whose flattened names collide are not — see T02-020.
@guard     Catches assuming LOGOS can round-trip the notation. There is no fixity carrier, so a
           LOGOS term printed back out will use application syntax, not the operator. Any
           harness that compares printed forms rather than `types_equal` will report spurious
           failures on every case in this tier.
@guard     Catches the SECTION-LOCAL form. `local infixl` and `scoped infixl` are visible only
           inside a section or namespace; the expansion still happens before elaboration, so the
           `Expr` is unaffected. A translator that tracks notation scope is doing work it does
           not need to do, and doing it wrong is a way to mistranslate a term that would
           otherwise have been fine.
@oracle    The fixity commands below produce ZERO LOGOS declarations. `assocLeft` and
           `assocRight` must translate to STRUCTURALLY DIFFERENT LOGOS terms (the harness
           asserts they are not `types_equal` after erasing names), and each must be
           `types_equal` to the translation of its written-out counterpart.
-/

namespace T09_002

def f (a b : Nat) : Nat := a * 10 + b

/-- LEFT-associative. -/
infixl:65 " ⋄ " => f

/-- RIGHT-associative, same precedence, different function. -/
def g (a b : Nat) : Nat := a * 10 + b

infixr:65 " ▷ " => g

/-- The two groupings are different terms with different VALUES. -/
theorem assocLeft (a b c : Nat) : (a ⋄ b ⋄ c) = f (f a b) c := rfl

theorem assocRight (a b c : Nat) : (a ▷ b ▷ c) = g a (g b c) := rfl

/-- And they genuinely differ: the same three arguments give different results. -/
theorem assocDiffers : (1 ⋄ 2 ⋄ 3) ≠ (1 ▷ 2 ▷ 3) := by decide

/-- PREFIX notation. -/
def neg (n : Nat) : Nat := 100 - n

prefix:max "∽" => neg

theorem prefixExpands (n : Nat) : (∽n) = neg n := rfl

/-- POSTFIX notation. -/
def bang (n : Nat) : Nat := n * n

postfix:max "‼" => bang

theorem postfixExpands (n : Nat) : (n‼) = bang n := rfl

/-- Non-associative `infix`, which requires explicit parentheses on the Lean side. -/
def h (a b : Nat) : Nat := a + b * 3

infix:60 " ⊙ " => h

theorem infixNonAssoc (a b c : Nat) : ((a ⊙ b) ⊙ c) = h (h a b) c := rfl

/-- MIXED PRECEDENCE: `⋄` at 65 binds tighter than `⊙` at 60. -/
theorem mixedPrecedence (a b c : Nat) : (a ⊙ b ⋄ c) = h a (f b c) := rfl

-- A `local` fixity — scope affects the parser, not the `Expr`.
section Scoped

local infixl:70 " ⊛ " => f

theorem localFixity (a b : Nat) : (a ⊛ b) = f a b := rfl

end Scoped

end T09_002
