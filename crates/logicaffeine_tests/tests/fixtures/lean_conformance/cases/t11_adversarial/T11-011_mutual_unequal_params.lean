/-!
@case      T11-011
@tier      11 · Adversarial and soundness traps
@title     Mutual inductives with unequal parameter counts — the block registers, and only the
           RECURSOR refuses, long after the damage is done
@lean      mutual inductive block, members with different numbers of parameters, mutual
           recursion over them
@expect    reject
@target    (none — `derive_recursor` cannot produce an eliminator for such a block, so the
           inductives are declarable but not usable)
@evidence  kernel/src/recursor.rs:193-207 (`derive_mutual_recursors`: it takes
           `num_params` from `block[0]` at :199, then for every member checks
           `if ctx.inductive_num_params(name).min(peel_pis(full).0.len()) != num_params {
           return Err(KernelError::CertificationError("auto-recursor: mutual block members
           disagree on parameter count ('{name}')")) }` at :200-207 — the check lives in the
           RECURSOR derivation, not in the declaration);
           kernel/src/context.rs:570-628 (`add_mutual_inductives` is transactional for
           POSITIVITY — "if any constructor violates positivity, nothing is added" — but its
           `MutualInductive` records carry a per-member `num_params` and nothing reconciles
           them); kernel/src/context.rs:253 (`add_inductive` is UNCHECKED and treats the full
           arity as parameters, so a translator that declares the members one at a time never
           reaches the mutual path at all); kernel/src/context.rs:374
           (`add_indexed_inductive(name, sort, num_params)`, the only way to set a different
           split); kernel/src/recursor.rs:36 (`derive_recursor` synthesises `(type, term)` and
           "the caller must `add_definition` it" — so a translator that never calls it never
           sees the error); kernel/src/type_checker.rs:254-260 and :612-678 (`Match` is the
           PRIMITIVE in LOGOS, so a translator can pattern-match on these inductives without
           ever deriving a recursor)
@diag      E_MUTUAL_PARAM_COUNT_MISMATCH — must name the block, list each member's parameter
           count, quote the constraint from `recursor.rs:200-207`, and emit NOTHING. Partial
           emission — declaring the inductives and omitting the recursors — is the failure this
           case is designed to catch.
@guard     **TIER 11 THESIS.** The kernel accepting a declaration is not evidence of
           faithfulness — and here the acceptance and the rejection are SEPARATED IN TIME. The
           declaration path takes the block happily. The failure surfaces only when
           `derive_recursor` is called, which is a SEPARATE, OPTIONAL step the caller must
           initiate (recursor.rs:36). A translator that emits the inductives, emits its
           functions as `Term::Match` (which is primitive in LOGOS, unlike Lean where `.rec` is
           primitive — T03-021), and never derives a recursor, produces a context that
           typechecks completely and in which the mutual block's induction principle DOES NOT
           EXIST. Every proof by mutual induction in the source then has no target, and the
           translator discovers this one declaration at a time, far from the block.
@guard     Catches the "just declare them separately" workaround. `add_inductive`
           (context.rs:253) will register each member on its own with no mutual bookkeeping at
           all, and `add_constructor` (context.rs:419) is UNCHECKED, so a constructor of `Even`
           mentioning `Odd` is accepted. The block then has no positivity check over the WHOLE
           block — which `add_mutual_inductives` performs specifically because "a sibling
           occurrence is a recursive occurrence, a sibling in a negative position is a
           cross-block paradox" (context.rs:571-578). Bypassing the mutual path to dodge the
           parameter-count check also bypasses the cross-block positivity check. See T11-013.
@guard     Catches EQUALISING the parameter counts by moving a parameter to an index, or an
           index to a parameter. Either changes the family: a parameter is uniform across all
           constructors and appears in the recursor's motive telescope before the indices
           (recursor.rs:208-212), an index does not. Silently re-splitting to satisfy
           `recursor.rs:200-207` produces an eliminator with the wrong motive shape, and the
           terms built with it typecheck.
@guard     Catches the `.min()` in the check. The comparison is
           `ctx.inductive_num_params(name).min(peel_pis(full).0.len())` (recursor.rs:200-202),
           i.e. the recorded parameter count CLAMPED to the member's actual arity. So a member
           whose recorded `num_params` exceeds its arity is silently clamped rather than
           diagnosed, and two members can pass the check while disagreeing about what their
           parameters mean. A translator must set `num_params` from Lean's own params/indices
           split, never let it default.
@guard     Catches assuming Lean's mutual blocks always have equal parameter counts. Lean
           requires the members of a mutual inductive block to share their PARAMETERS, but not
           their INDICES — so a block of `Even : Nat → Prop` and `Odd : Nat → Prop` with an
           extra parameter on one member is expressible in Lean via separate parameterisation
           in ways that flatten differently, and any block whose members' arities differ will
           trip `recursor.rs:200-207` if the translator records arity as parameters (which
           `add_inductive` does by default).
@oracle    Translator exits non-zero with E_MUTUAL_PARAM_COUNT_MISMATCH and emits NO
           declaration for the block — in particular zero `add_inductive`,
           `add_indexed_inductive`, `add_constructor` and `add_mutual_inductives` calls. A
           harness additionally asserts, for every mutual block anywhere in the corpus output,
           that `derive_mutual_recursors` was actually CALLED and its result registered with
           `add_definition`; a block whose recursor was never derived counts as an unfinished
           translation even if every term using it typechecks.
-/

namespace T11_011

/-! ### A mutual block whose members have DIFFERENT arities. Lean accepts this; the recorded
    parameter counts after a default `add_inductive` lowering are 1 and 2, and
    `recursor.rs:200-207` refuses the block. -/

mutual
  inductive Tree (α : Type) : Type where
    | node : α → Forest α → Tree α

  inductive Forest (α : Type) : Type where
    | nil : Forest α
    | cons : Tree α → Forest α → Forest α
end

-- Both members have one parameter here, so this block is the CONTROL: it must translate.
mutual
  def treeSize {α : Type} : Tree α → Nat
    | Tree.node _ f => 1 + forestSize f

  def forestSize {α : Type} : Forest α → Nat
    | Forest.nil => 0
    | Forest.cons t f => treeSize t + forestSize f
end

example : treeSize (Tree.node 1 Forest.nil) = 1 := by rfl

/-! ### Now the failing shape: one member indexed, one not. After a default `add_inductive`
    lowering the whole arity counts as parameters, so `EvenT` records 1 and `OddPair` records
    2, and `derive_mutual_recursors` refuses. -/

mutual
  inductive EvenT : Nat → Prop where
    | zero : EvenT 0
    | succ : ∀ n, OddT n → EvenT (n + 1)

  inductive OddT : Nat → Prop where
    | succ : ∀ n, EvenT n → OddT (n + 1)
end

example : EvenT 2 := EvenT.succ 1 (OddT.succ 0 EvenT.zero)

-- A mutual block mixing a parameterised member with an indexed one. This is the shape where
-- the arities genuinely differ, and where equalising the counts by re-splitting changes the
-- family.
mutual
  inductive Expr' (α : Type) : Type where
    | lit : α → Expr' α
    | app : Stmt' α 0 → Expr' α

  inductive Stmt' (α : Type) : Nat → Type where
    | skip : Stmt' α 0
    | seq : Expr' α → Stmt' α 0 → Stmt' α 0
end

mutual
  def exprDepth {α : Type} : Expr' α → Nat
    | Expr'.lit _ => 1
    | Expr'.app s => 1 + stmtDepth s

  def stmtDepth {α : Type} : Stmt' α 0 → Nat
    | Stmt'.skip => 0
    | Stmt'.seq e s => exprDepth e + stmtDepth s
end

/-- Note that this does NOT close by `rfl`: the mutual block's equation compiler output is not
    definitionally transparent here, so the proof goes through the generated equation lemmas —
    which LOGOS does not generate at all (T04-017). -/
example : exprDepth (Expr'.lit (3 : Nat)) = 1 := by simp [exprDepth]

/-- A proof that needs the block's ELIMINATOR. Note that Lean's own `induction` tactic refuses
    to use `EvenT.rec` directly here ("the eliminator has multiple motives"), so the proof goes
    by induction on `Nat` and `cases` on the block — which is precisely the shape whose LOGOS
    translation needs a positional `Match` on each member, and, for the proof to be by
    induction at all, the derived mutual recursor that `recursor.rs:200-207` refuses to build
    for a block with unequal parameter counts. -/
theorem evenNotOdd : ∀ n, EvenT n → OddT n → False := by
  intro n
  induction n with
  | zero => intro _ ho; cases ho
  | succ k ih => intro he ho; cases he with
    | succ m hm => cases ho with
      | succ m' hm' => exact ih hm' hm

end T11_011
