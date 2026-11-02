/-!
@case      T08-015
@tier      8 · Tactics
@title     `ring` exists only on the LITERATE surface, is `Int`/`Nat`-only, and reports failure
           by REDUCING to an error derivation rather than by failing
@lean      ring, ring_nf, ring over a general CommRing
@expect    translate
@target    Surface C (literate kernel theorem) proof tactic `ring.`, parsed at
           `kernel/src/interface/literate_parser.rs:626-636` into
           `Term::App(Global("try_ring"), <goal as Syntax>)`, declared at
           `kernel/src/prelude.rs:2966-2991` (`try_ring : Syntax → Derivation`, plus the
           `DRingSolve : Syntax → Derivation` constructor) and reduced by
           `kernel/src/reduction.rs:579-582` → `try_try_ring_reduce`
           (`kernel/src/reduction.rs:2282-2316`)
@evidence  proof/src/tactic_script.rs:308-327 (`canonical` — there is NO arm mapping any word to
           "ring"; `ring` in a Surface-A `Proof:` script is `unknown tactic \`ring\``. The
           tactic-script surface and the literate surface DO NOT INTEROPERATE);
           kernel/src/interface/literate_parser.rs:626-636 (the `ring` arm of
           `parse_proof_tactic`); kernel/src/prelude.rs:2984-2990 (`try_ring`'s type is
           `Syntax → Derivation` — it consumes a REIFIED goal, not a proposition);
           kernel/src/reduction.rs:2282-2316 (`try_try_ring_reduce`: extract `Eq T a b`, require
           `is_ring_type(T)`, reify both sides with ONE shared `VarInterner`, compare
           `canonical_eq`, and emit `DRingSolve goal` on success);
           kernel/src/reduction.rs:2352-2357 (`is_ring_type` — `name == "Int" || name == "Nat"`,
           and nothing else);
           kernel/src/reduction.rs:2290-2292, :2296-2315 (EVERY failure path returns
           `Some(make_error_derivation())` — the reduction SUCCEEDS, producing
           `DAxiom (SName "Error")`, rather than getting stuck or returning `None`);
           kernel/src/interface/literate_parser.rs:867-870 (literate `induction` is hardcoded to
           `Nat`); LOGOS_FACTS §Surface C (there is NO `forall` in the `Statement:` grammar —
           free variables are effectively universally closed)
@guard     THE ERROR-DERIVATION TRAP, and the reason this case is in the corpus. When `ring`
           cannot prove the goal, `try_try_ring_reduce` does not fail: it returns
           `make_error_derivation()` (reduction.rs:2290, :2294, :2299, :2315). A `Term` of type
           `Derivation` is produced, the reduction step reports success, and the difference
           between "proved" and "not proved" is a `DAxiom (SName "Error")` sitting inside an
           otherwise well-typed term. A translator that checks only "did the kernel accept the
           term" will record a proof where there is none. The check must be on the derivation's
           HEAD CONSTRUCTOR: `DRingSolve` means proved, `DAxiom (SName "Error")` means not.
@guard     Catches emitting `ring` into a Surface-A script. The three LOGOS vernacular surfaces
           do not interoperate (LOGOS_FACTS §"The three vernacular surfaces"), and `ring` is
           reachable only from Surface C. A Lean theorem proved `by ring` must therefore be
           emitted as a LITERATE theorem — which in turn means its statement must fit the
           literate `Statement:` grammar, which has no `forall` (literate_parser.rs:1223-1270).
           A `ring` proof of a UNIVERSALLY QUANTIFIED identity is expressible only because the
           literate surface universally closes free variables implicitly; a translator that
           emits an explicit quantifier will not parse.
@guard     Catches the CARRIER. `is_ring_type` admits exactly `Int` and `Nat`
           (reduction.rs:2352-2357). Lean's `ring` works in any `CommSemiring`/`CommRing`,
           which is most of Mathlib's algebra. Translating a `ring` call over an abstract
           `CommRing R` to LOGOS's `ring` silently changes the carrier to `Int`, and since
           LOGOS's `Int` is OPAQUE with eight axiomatised ring laws (prelude.rs:92-191) rather
           than a derived structure, the resulting statement is about a different object than
           the Lean source's.
@guard     Catches losing the SHARED INTERNER. `try_try_ring_reduce` reifies both sides with ONE
           `VarInterner` (reduction.rs:2295-2306) precisely so the two sides agree on which
           named global is which polynomial variable. A translator that reifies the sides
           separately — natural if it normalises subterms independently — produces polynomials
           whose variables are numbered differently and whose `canonical_eq` then fails on a
           true identity.
@note      Bare Lean 4.14 has NO `ring`: it is a Mathlib tactic and `by ring` here errors with
           "unknown tactic". Since this corpus forbids `import Mathlib`, every theorem below
           carries the Mathlib invocation in a comment and proves the same goal with a core
           tactic (`omega`, `simp`, or an explicit `Nat`/`Int` lemma). The CONSTRUCT under test
           is the commented `ring` call; the live code exists so the file compiles and so a
           harness has a Lean-side goal to compare the LOGOS `ring` result against.
@oracle    Each theorem is emitted as a Surface-C literate theorem whose `Proof:` is `ring.`;
           the reduced derivation's head is `DRingSolve`, NOT `DAxiom (SName "Error")`; the
           statement's `Eq` type argument is `Int` or `Nat`; and no Surface-A script anywhere in
           the corpus contains the token `ring`.
-/

namespace T08_015

/-- Mathlib: `by ring`. Commutativity of `+` as a polynomial identity over `Nat`. -/
theorem addComm (a b : Nat) : a + b = b + a := by omega

/-- Mathlib: `by ring`. Distribution — the shape `try_ring`'s polynomial normaliser exists for. -/
theorem distrib (a b c : Nat) : a * (b + c) = a * b + a * c := Nat.mul_add a b c

/-- Mathlib: `by ring`. Over `Int`, the other carrier `is_ring_type` admits. -/
theorem intIdentity (x y : Int) : (x + y) * (x + y) = x * x + (x * y + y * x) + y * y := by
  simp [Int.add_mul, Int.mul_add]
  omega

/-- Mathlib: `by ring`. A goal `ring` CANNOT prove — the two sides are not equal as
    polynomials. In LOGOS this reduces to `DAxiom (SName "Error")` rather than failing, which
    is the trap the first guard describes. Stated here as the negation so the file compiles. -/
theorem notARingIdentity : ¬ (∀ a b : Nat, a * b = a + b) := by
  intro h
  have := h 2 5
  omega

/-- Mathlib: `by ring`. The carrier is an ABSTRACT commutative ring, declared locally so no
    Mathlib import is needed. `is_ring_type` admits neither this type nor any other. -/
class CommRingLike (R : Type) where
  add : R → R → R
  mul : R → R → R
  add_comm : ∀ a b : R, add a b = add b a

theorem abstractCarrier (R : Type) [CommRingLike R] (a b : R) :
    CommRingLike.add a b = CommRingLike.add b a :=
  CommRingLike.add_comm a b

/-- Mathlib: `by ring_nf` — normalise without closing. LOGOS has no normal-form-only mode:
    `try_ring` either emits `DRingSolve` or the error derivation. -/
theorem needsNormalOnly (a b : Nat) : (a + b) * 1 = a + b := by simp

end T08_015
