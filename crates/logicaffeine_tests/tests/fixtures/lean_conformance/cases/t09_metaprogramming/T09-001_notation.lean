/-!
@case      T09-001
@tier      9 · Metaprogramming and syntax
@title     `notation` — expands before the `Expr` exists, so there is nothing left to translate
@lean      notation, notation with precedence, notation binding several arguments
@expect    translate
@target    The translation of the EXPANDED term. A `notation` command itself produces NO LOGOS
           declaration; the application it abbreviates translates by the ordinary Tier 0 rules
           (`Term::App` spine, kernel/src/term.rs:459).
@evidence  kernel/src/term.rs:443-545 (`Term` — the COMPLETE kernel term language: there is no
           `Notation`, no `Syntax`, no parse-tree variant, and no place to record how a term was
           written); kernel/src/context.rs:20-80 (`Context` — flat `HashMap<String, _>` maps for
           inductives, constructors, declarations, definitions, universe polys, instances,
           coercions and binder kinds. There is no notation table, no parser extension, no
           precedence database); kernel/src/context.rs:201 (`set_binder_kinds` — the closest
           thing to surface information LOGOS records, and it stores only
           `ParamKind::{Explicit, Implicit, Instance}`, elaborate.rs:1018-1027);
           kernel/src/interface/literate_parser.rs:1223-1270 (the ONE fixed infix table LOGOS
           has — `<= ≤ >= ≥ < > + - *` and the word `equals` — which is hard-coded in the
           literate parser and not extensible);
           kernel/src/elaborate.rs (elaboration works on `Term`, never on a surface tree: there
           is no `Syntax` type in the kernel at all)
@guard     THE GOVERNING RULE OF TIER 9, stated here because this is the first case it applies
           to: **translate the post-elaboration `Expr`, never the surface `Syntax`.** A
           translator that reads Lean's `Syntax` would have to expand `notation` itself — which
           means reimplementing Lean's macro expander, including precedence resolution, binder
           notation, and the interaction with `open`/`local`/`scoped`. Every divergence from
           Lean's expander is a silent mistranslation, because the resulting term is
           well-formed and simply means something else. Working from the elaborated `Expr`
           makes this case vanish: by then `x ⊕ y` is already `myOp x y`.
@guard     Catches emitting a LOGOS declaration for the `notation` command. There is no target
           for it (term.rs:443-545, context.rs:20-80). A translator that tries — say by
           registering a definition named after the notation token — pollutes the flat global
           namespace with a name no term will ever reference, and risks colliding with a real
           declaration (see T02-020).
@guard     Catches losing the notation at the REPORTING boundary. The `Expr` carries no record
           of the surface spelling, so a diagnostic about a term the user wrote as `x ⊕ y` will
           name `myOp`. That is unavoidable (there is no LOGOS carrier for it), but a translator
           should keep the Lean-side source position so its own messages can point back. This is
           the same information loss as T09-010 and T00-009, and must not be confused with a
           translation failure.
@guard     Catches PRECEDENCE-SENSITIVE parses. `notation:65 a " ⊕ " b => myOp a b` and a
           different precedence produce DIFFERENT `Expr`s for the same character sequence when
           combined with other operators. A `Syntax`-reading translator must get the precedence
           right; an `Expr`-reading one cannot get it wrong. `precedenceMatters` below is the
           witness: the two spellings elaborate to different trees.
@guard     Catches notation that is NOT a simple abbreviation. `notation "⟦" x "⟧" => wrap x`
           introduces brackets that change how the parser reads what is inside them. A
           translator working from `Syntax` must handle the bracketing; one working from `Expr`
           sees only `wrap`.
@oracle    The `notation` commands below produce ZERO LOGOS declarations. Each theorem's
           translated term is `types_equal` to the translation of the same theorem written
           without the notation — i.e. `notationIsTransparent` and `writtenOut` below must
           translate to the same LOGOS `Term`, up to alpha-renaming.
-/

namespace T09_001

def myOp (a b : Nat) : Nat := a + b * 2

/-- A plain infix abbreviation. -/
notation:65 a " ⊕ " b => myOp a b

/-- The notation and the written-out form must translate to the SAME LOGOS term. -/
theorem notationIsTransparent (x y : Nat) : (x ⊕ y) = myOp x y := rfl

theorem writtenOut (x y : Nat) : myOp x y = myOp x y := rfl

/-- PRECEDENCE MATTERS: two notations at different precedences over the same operands. -/
def myOther (a b : Nat) : Nat := a * b

notation:70 a " ⊗ " b => myOther a b

theorem precedenceMatters (x y z : Nat) : (x ⊕ y ⊗ z) = myOp x (myOther y z) := rfl

/-- BRACKETING notation — the surface form changes how the parser reads its contents. -/
def wrap (n : Nat) : Nat := n + 1

notation:max "⟦" x "⟧" => wrap x

theorem bracketing (n : Nat) : ⟦n⟧ = wrap n := rfl

/-- Notation for a PROPOSITION, so the expansion happens inside a `Prop`-valued term. -/
def divides (a b : Nat) : Prop := ∃ k, b = a * k

notation:50 a " ∣∣ " b => divides a b

theorem propNotation (a b : Nat) : (a ∣∣ b) ↔ divides a b := Iff.rfl

/-- Notation applied to a COMPOUND argument, so the expansion is not a leaf. -/
theorem compoundArgument (x y z : Nat) : ((x ⊕ y) ⊕ z) = myOp (myOp x y) z := rfl

/-- A notation used inside a TACTIC proof — the expansion has already happened by the time
    the tactic block elaborates. -/
theorem inTacticProof (x : Nat) : (x ⊕ 0) = x := by
  show myOp x 0 = x
  simp [myOp]

end T09_001
