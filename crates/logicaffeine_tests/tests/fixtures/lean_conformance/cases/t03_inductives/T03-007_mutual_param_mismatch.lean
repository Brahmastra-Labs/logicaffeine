/-!
@case      T03-007
@tier      3 · Inductives and structures
@title     Mutual block with disagreeing parameter counts — the recursor cannot be derived
@lean      mutual inductive with per-member parameters; Lean's own
           "number of parameters mismatch in mutually inductive datatypes" check
@target    (none — no sound translation)
@expect    reject
@diag      E_MUTUAL_PARAM_MISMATCH — must name the block, list each member with the parameter
           count the translator assigned it, and state that a LOGOS mutual block shares **one**
           parameter telescope taken from its first member. Must register **nothing**:
           `add_mutual_inductives` does not itself check the agreement, so a translator that
           "tries it and sees" leaves a permanently unusable block in the context.
@evidence  kernel/src/recursor.rs:192-207 (`derive_mutual_recursor` takes `num_params` from
           `block[0]` and then, for every member, errors with "auto-recursor: mutual block
           members disagree on parameter count" if
           `inductive_num_params(name).min(arity(name)) != num_params`);
           kernel/src/recursor.rs:208-212 (the shared `param_tele`, `tp_names` and
           `param_types` are all read off `block[0]` — every member is *assumed* to take that
           exact telescope);
           kernel/src/context.rs:562-614 (`add_mutual_inductives` checks positivity and
           constructor universes but **never** compares `num_params` across members — the block
           registers happily and the failure surfaces only at recursor derivation);
           kernel/src/context.rs:246-255 and :399-411 (the default is "all of the arity is
           parameters", which is what *produces* the disagreement in practice)
@guard     The Lean-illegal direct form is commented out below, because Lean refuses it with
           "number of parameters mismatch in mutually inductive datatypes". The translator must
           never *synthesise* that shape either. The realistic way it gets synthesised is the
           T03-005 bug: registering each member with `add_inductive`, whose default parameter
           count is the member's **full arity**. In a Lean-legal block whose members have
           different numbers of *indices* — which Lean permits — that default assigns different
           parameter counts to different members, and `derive_recursor` then refuses the whole
           block.
@guard     Catches the deferred-failure trap. `add_mutual_inductives` returns `Ok` for a block
           with mismatched counts: positivity and universe checks pass, headers and constructors
           are inserted, and `mutual_blocks` is populated (context.rs:595-612). The context now
           contains a block for which no recursor can ever be derived, and the error appears
           later at an unrelated call site as `CertificationError("auto-recursor: …")`. The
           translator must verify the agreement *itself*, before emitting.
@guard     Catches "patching" the mismatch by padding the shorter member's parameter list. The
           recursor's shared telescope comes from `block[0]` (recursor.rs:208-212), so padding
           means the padded member's constructors are checked against parameters it does not
           actually take; `compute_indexed_case_type` (type_checker.rs:685-714) then
           instantiates those positions from the discriminant and silently discards whatever
           the constructor declared there.
@guard     Catches splitting the block to dodge the check. Registering the members separately
           removes cross-block positivity (`check_positivity_mutual`, positivity.rs:36-47) and
           leaves `mutual_block_of` empty, so `derive_recursor` builds standalone recursors that
           treat a sibling occurrence as an opaque field — no recursion through the pair. That
           is a different, quieter corruption, not a fix.
@note      Lean itself rejects the declaration this case is named for, so it cannot appear as
           compiling source. It is reproduced verbatim in the comment block below, together with
           Lean's exact diagnostic. The compiling declarations in this file are the **nearest
           legal boundary**: mutual blocks that Lean accepts because their members share
           parameters but differ in *index* count. Those are the blocks on which the
           "arity is parameters" default produces the illegal shape inside LOGOS, and they are
           therefore the actual input a translator will meet.
@oracle    Translator exits non-zero with E_MUTUAL_PARAM_MISMATCH and the LOGOS context contains
           no member, no constructor and no `mutual_blocks` entry for the block. Separately, for
           every mutual block the translator *does* emit, `derive_recursor` must succeed for
           every member.
-/

namespace T03_007

/-
THE OFFENDING DECLARATION — rejected by Lean 4.14.0 with:

  error: invalid inductive type, number of parameters mismatch in mutually inductive datatypes

    mutual
      inductive U (α : Type) where
        | u : α → V → U α
      inductive V where
        | v : V
        | vu : U Nat → V
    end

Same rejection for a mismatch in the other direction:

    mutual
      inductive W (α : Type) : Type where
        | w : α → X α β → W α
      inductive X (α : Type) (β : Type) : Type where
        | x : X α β
    end

LOGOS has no such check at registration time. `Context::add_mutual_inductives`
(context.rs:562-614) would accept the translated form of either block — positivity and
constructor universes are all it verifies — and the disagreement would only be discovered
by `recursor::derive_mutual_recursor` (recursor.rs:200-207), by which time the block is
already in the context.
-/

mutual
  /-- The *agreeing* case, for contrast: both members have 0 parameters and 1 index, so the
      arity default happens to assign both `num_params = 1` and the block still agrees — wrong
      (T03-005), but not caught here. Agreement is necessary, never sufficient. -/
  inductive EvenL : Nat → Prop where
    | zero : EvenL 0
    | succ : {n : Nat} → OddL n → EvenL (n + 1)

  inductive OddL : Nat → Prop where
    | succ : {n : Nat} → EvenL n → OddL (n + 1)
end

example : EvenL 2 := EvenL.succ (OddL.succ EvenL.zero)

mutual
  /-- **This is the boundary case.** Lean accepts it: both members have 0 parameters. But
      `Chain` has arity 1 (one `Nat` index) and `Tag` has arity 0. Under the `add_inductive`
      default — "all of the arity is parameters" (context.rs:246-255) — the translator assigns
      `Chain` 1 parameter and `Tag` 0, and `derive_recursor` then refuses the block
      (recursor.rs:200-207). The correct registration gives **both** members
      `num_params = 0`. -/
  inductive Chain : Nat → Type where
    | stop : Chain 0
    | more : {n : Nat} → Tag → Chain n → Chain (n + 1)

  inductive Tag : Type where
    | t1 : Tag
    | t2 : Tag
end

def chainLen : {n : Nat} → Chain n → Nat
  | _, Chain.stop => 0
  | _, Chain.more _ c => Nat.succ (chainLen c)

example : chainLen (Chain.more Tag.t1 (Chain.more Tag.t2 Chain.stop)) = 2 := rfl

mutual
  /-- The same hazard *with* a real shared parameter. Lean's `numParams` is 1 for both
      members; the arities are 2 and 1. The arity default would assign 2 and 1 — again a
      disagreement — where the truth is 1 and 1. -/
  inductive PA (α : Type) : Nat → Type where
    | leaf : α → PA α 0
    | branch : {n : Nat} → PB α → PA α n → PA α (n + 1)

  inductive PB (α : Type) : Type where
    | hold : PA α 0 → PB α
    | empty : PB α
end

def paDepth {α : Type} : {n : Nat} → PA α n → Nat
  | _, PA.leaf _ => 0
  | _, PA.branch _ p => Nat.succ (paDepth p)

example : paDepth (PA.branch (PB.empty) (PA.leaf (1 : Nat))) = 1 := rfl

end T03_007
