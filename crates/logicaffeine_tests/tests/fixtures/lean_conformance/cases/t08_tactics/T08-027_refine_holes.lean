/-!
@case      T08-027
@tier      8 · Tactics
@title     `refine ⟨?_, ?_⟩` — no `refine` atom, and the obvious term-level encoding puts a
           `Term::Hole` in the output, which `types_equal` matches against ANYTHING
@lean      refine, refine ?_ , refine' , named holes ?h, exact? placeholders
@expect    reject
@target    (none — no sound translation)
@evidence  proof/src/tactic_script.rs:308-327 (`canonical` — no arm for `refine`; the script
           grammar's fifteen primitives (:275-292) contain nothing that takes a partial term);
           proof/src/tactic_script.rs:51-105 (the script lexer has no `?`: the character falls
           into the `other =>` arm at :101 and the script fails with
           `unexpected character '?'` — so a `?_` hole cannot even be tokenised);
           kernel/src/term.rs:443-545 (`Term::Hole` — the only placeholder variant; there is NO
           metavariable variant, no `mvar`, and no assignment map);
           kernel/src/type_checker.rs:1594-1597 (`types_equal` treats `Term::Hole` as MATCHING
           ANYTHING — the leniency that makes an emitted hole a soundness hole, not merely an
           incompleteness);
           kernel/src/type_checker.rs:1314 (the same leniency leaks into `is_subtype`'s fast
           path); kernel/src/elaborate.rs:138 (elaborator metavariables are `Term::Var` names
           beginning with `?`, NOT a `Term` variant — they exist only inside the elaborator and
           must be fully instantiated before the kernel sees the term);
           proof/src/tactic.rs:83-90, :943 (`ProofState`'s own open goals are `Node::Hole(Goal)`
           — a PROOF-STATE hole, not a term hole; `assemble` refuses to build a derivation while
           one remains, returning `TacticError::GoalsRemain`)
@diag      E_NO_REFINE / E_HOLE_IN_OUTPUT — must name the theorem, report the number and names
           of the `?` holes, state that the script grammar has no `refine` and its lexer rejects
           `?`, and state that emitting a `Term::Hole` is forbidden because `types_equal`
           matches it against anything. Must emit NO term and NO script.
@guard     THE HOLE-IS-NOT-A-METAVARIABLE TRAP, and the reason this is a rejection rather than a
           "not supported yet". A translator that cannot find a `refine` atom will reach for the
           term level and emit the partial term directly, filling the `?_`s with `Term::Hole`.
           That term TYPECHECKS — `types_equal` returns true for a `Hole` against any type
           (type_checker.rs:1594-1597), and the leniency leaks into `is_subtype`
           (type_checker.rs:1314). So the kernel accepts a "proof" with a gap in it, and the
           certificate that results is indistinguishable from a real one. This is the same
           failure as T00-016 and T11-004, reached from the tactic side.
@guard     Catches the `Term::Var("?h")` encoding. LOGOS's elaborator represents metavariables as
           `Term::Var` names beginning with `?` (elaborate.rs:138) and requires them to be
           instantiated before the kernel runs. A translator that emits such a `Var` and hands
           it to the kernel gets an unbound-variable error if it is lucky, and — if a binder
           anywhere in the term happens to be named `?h` — a CAPTURED variable if it is not,
           because LOGOS's representation is named (term.rs:449) and substitution alpha-renames
           only on collision (type_checker.rs:1088).
@guard     Catches mapping `refine` onto `split`/`constructor`. `refine ⟨?_, ?_⟩` on a
           conjunction IS `split` — and only there. `refine` with a partially-applied function,
           a nested anonymous constructor, or a hole in a non-head position has no
           correspondence to any of the fifteen primitives. A translator that special-cases the
           `⟨?_, ?_⟩`-on-`∧` shape must reject every other shape, and must not present the
           special case as general support.
@guard     Catches NAMED holes. `refine ⟨?left, ?right⟩` names the resulting goals, and the
           names are then used by `case left => …`. LOGOS's `Goal` has no name field at all
           (see T08-025), so even the goals the special case would produce cannot carry the
           names forward.
@guard     Catches the ORDER of `refine`'s goals. Lean's `refine` produces goals in the order
           the holes appear in the term, and `refine ⟨?_, ?_⟩` therefore agrees with
           `constructor`. A translator that assumes this in general will be wrong for
           `refine f ?a ?b` where `f`'s arguments are not in goal order.
@oracle    Translator exits non-zero with E_NO_REFINE (or E_HOLE_IN_OUTPUT) for every theorem
           below and writes NO declaration. A harness additionally asserts the global invariant
           this case is a member of: **no `Term::Hole` appears anywhere in the translator's
           output for the entire corpus**, checked by walking every emitted `Term`.
-/

namespace T08_027

/-- The shape a translator is most likely to special-case: `refine ⟨?_, ?_⟩` on a conjunction,
    which coincides with `constructor`. -/
theorem refineAnd (p q : Prop) (hp : p) (hq : q) : p ∧ q := by
  refine ⟨?_, ?_⟩
  · exact hp
  · exact hq

/-- A hole in a NON-HEAD position: the term is `hpq ?_`, and the hole is an argument. -/
theorem refineArgument (p q : Prop) (hpq : p → q) (hp : p) : q := by
  refine hpq ?_
  exact hp

/-- NAMED holes, whose names are then used as case tags. -/
theorem refineNamed (p q : Prop) (hp : p) (hq : q) : p ∧ q := by
  refine ⟨?left, ?right⟩
  case left => exact hp
  case right => exact hq

/-- Holes in an order that does NOT match the goal order a `constructor` would produce. -/
theorem refineOutOfOrder (p q : Prop) (hp : p) (hq : q) : p ∧ q := by
  refine ⟨?second, ?first⟩
  case first => exact hq
  case second => exact hp

/-- A hole under a BINDER — the lambda's parameter is in scope inside the hole's goal, which is
    exactly the context a `Term::Hole` cannot record. -/
theorem refineUnderBinder (p : Nat → Prop) (h : ∀ n, p n) : ∀ n, p n := by
  refine fun n => ?_
  exact h n

/-- A partially-applied constructor with a NESTED hole. -/
theorem refineNested (p q r : Prop) (hp : p) (hq : q) (hr : r) : p ∧ (q ∧ r) := by
  refine ⟨?_, ⟨?_, ?_⟩⟩
  · exact hp
  · exact hq
  · exact hr

/-- A single anonymous hole standing for the whole proof — the degenerate case, and the one
    that most obviously must not become a `Term::Hole`. -/
theorem refineWhole (p : Prop) (hp : p) : p := by
  refine ?_
  exact hp

end T08_027
