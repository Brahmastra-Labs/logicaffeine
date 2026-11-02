/-!
@case      T05-007
@tier      5 · Elaboration surface
@title     Auto params `(h : P := by tac)` — the tactic must be RUN by Lean before translation
@lean      autoParam, `(h : P := by decide)`, `(h : P := by simp)`, auto-bound proof obligations
@expect    reject
@target    (none — no sound translation from the surface form)
@evidence  kernel/src/term.rs:443-545 (the `Term` enum has no tactic, no `Syntax`, no delayed
           elaboration, and no metavariable variant — `Hole` is the only unknown and it is not
           a suspension);
           kernel/src/elaborate.rs:137-139 (`is_meta`: metavariables are `Term::Var` names
           beginning with `?`, and LOGOS_FACTS records that they "must be fully instantiated
           before the kernel sees the term");
           kernel/src/elaborate.rs:1016-1027 (`ParamKind` has no `AutoParam` variant, so the
           parameter cannot even be marked as tactic-filled);
           kernel/src/type_checker.rs:313 (`Term::Hole` reaching `infer_type` is
           `CannotInferHole`) and :1593-1597 (`types_equal` treats `Hole` as matching
           **anything**);
           LOGOS_FACTS, Surface A tactic grammar (`crates/logicaffeine_proof/src/
           tactic_script.rs`) — the LOGOS tactic language is a proof-script surface with a
           fixed primitive list; it is not reachable from term elaboration, and `exact` takes a
           **named hypothesis only, not a term**
@diag      E_AUTO_PARAM_UNELABORATED — must name the declaration, the auto-param binder, and
           the tactic block, and state that the tactic must be executed by Lean and its
           resulting proof term translated. Must NOT emit a `Hole`, an axiom, or a LOGOS tactic
           script.
@guard     **This is the case that makes the Tier-5 rule load-bearing: translate the
           post-elaboration `Expr`, never the surface `Syntax`.** An auto param is a `Syntax`
           object stored in the function's TYPE (`autoParam P (Syntax.ident ``decide)`) and run
           by Lean at each call site. Reading `Syntax` therefore gives the translator a tactic
           block and no proof; reading the elaborated `Expr` of the CALL SITE gives it the
           finished proof term, which translates like any other. The rejection is against
           translating the auto param *as such*; a translator that elaborates each call site
           and translates the resulting proof term is doing the right thing and this case's
           declarations will then never reach the rejection path — the diagnostic must fire
           only when the unelaborated form is met.
@guard     Catches emitting `Term::Hole` for the auto-param argument. `types_equal` matches a
           `Hole` against **anything** (type_checker.rs:1593-1597) and that leniency leaks into
           `is_subtype`'s fast path (:1314), so a declaration carrying the hole can typecheck
           against a type it does not have. For an auto param the hole stands where a PROOF
           should be, so this is the difference between a proof library and a pile of
           assertions.
@guard     Catches emitting an axiom of the auto param's type via `add_declaration`
           (context.rs:434), which is unchecked. `predOf`'s auto param is `n ≠ 0`; axiomatising
           it asserts `Π(n:Nat). Not (Eq Nat n Zero)`, from which `0 ≠ 0` and then `False`
           follow. One convenience axiom, whole library inconsistent.
@guard     Catches translating the tactic BLOCK into a LOGOS proof script. The LOGOS script
           surface is a separate vernacular (LOGOS_FACTS, Surface A) that does not interoperate
           with kernel terms; there is no way to attach a script to an argument position, the
           primitive list is fixed and small, and `exact` accepts only a named hypothesis. Even
           where a LOGOS script could prove the same proposition, the resulting `VerifiedProof`
           is a separate object — not a term that can sit in an application spine.
@guard     Catches assuming the tactic is deterministic across call sites. `by decide` on
           `n ≠ 0` produces a DIFFERENT proof term for each `n`, because it reduces a
           `Decidable` instance at that `n`. A translator that elaborates the auto param once,
           at the declaration, and reuses the term at every call site emits a proof of the
           wrong proposition — and since proofs are `Prop` inhabitants, definitional proof
           irrelevance (type_checker.rs:1547-1567) may hide the mismatch until the type itself
           disagrees.
@guard     Catches the `autoParam` wrapper leaking into the emitted type. Lean's type for
           `safeDiv` literally mentions the constant `autoParam`; emitting
           `Global("autoParam")` is an unresolvable name, and declaring it as an axiom gives a
           type former nothing inhabits.
@oracle    Translator exits non-zero with E_AUTO_PARAM_UNELABORATED for each declaration below
           when it meets the unelaborated form; writes no declaration; emits no `Term::Hole`,
           no new axiom, and no `Global("autoParam")`. If the translator instead elaborates
           call sites, the emitted proof term for each call must be the translation of the
           term Lean's tactic produced AT THAT CALL SITE.
-/

namespace T05_007

/-- The canonical auto param: `by decide` runs at each call site, against that site's `b`. -/
def safeDiv (a b : Nat) (_h : b ≠ 0 := by decide) : Nat := a / b

example : safeDiv 10 2 = 5 := rfl
example : safeDiv 9 3 = 3 := rfl

/-- The proof is genuinely USED in the body, so it cannot be erased. -/
def predOf (n : Nat) (h : n ≠ 0 := by decide) : Nat :=
  if hz : n = 0 then absurd hz h else n - 1

example : predOf 5 = 4 := rfl

/-- A `by simp` auto param — the tactic is a simplifier run, not a decision procedure, so the
    proof term it produces is not predictable from the proposition alone. -/
def atLeast (n : Nat) (_h : 0 ≤ n := by simp) : Nat := n

example : atLeast 7 = 7 := rfl

/-
  `autoParam` written out by hand is not even expressible in source text: its second argument
  has type `Lean.Syntax`, not `Lean.Name`, so the tactic block really is a piece of PARSED
  SYNTAX embedded in the function's type —

      def rawAuto (n : Nat) (_h : autoParam (0 ≤ n) <a Lean.Syntax value>) : Nat := n

  which is exactly why there is nothing for a `Term`-level target to receive, and why the
  tactic must be run by Lean before anything is translated.
-/

/-- An auto param whose proposition MENTIONS an earlier parameter, so each call site's proof is
    about a different proposition. -/
def boundedPick (n : Nat) (i : Nat) (_h : i < n + 1 := by omega) : Nat := i

example : boundedPick 5 3 = 3 := rfl
example : boundedPick 2 0 = 0 := rfl

/-- A structure field with an auto param — the same mechanism inside `add_structure`'s
    field list, which has no slot for it (context.rs:271-369). -/
structure Bounded where
  n : Nat
  i : Nat
  h : i ≤ n := by omega

def b1 : Bounded := { n := 5, i := 3 }

example : b1.i = 3 := rfl

end T05_007
