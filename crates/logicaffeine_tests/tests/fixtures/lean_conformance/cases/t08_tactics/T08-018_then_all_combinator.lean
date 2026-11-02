/-!
@case      T08-018
@tier      8 · Tactics
@title     `<;>` — same spelling, but LOGOS applies the right-hand tactic to EVERY open goal,
           not only to the ones the left-hand tactic produced
@lean      t1 <;> t2, chained <;>, <;> after a tactic that leaves several goals
@expect    translate
@target    LOGOS script `chain := atom ('<;>' atom)*`, left-associative
           (proof/src/tactic_script.rs:202-210) → `combinators::then_all`
           (proof/src/tactic.rs:1138-1146)
@evidence  proof/src/tactic_script.rs:85-88 (the lexer's only multi-character token:
           `'<' if src[i..].starts_with("<;>")` ⟹ `Tok::ThenAll`);
           proof/src/tactic_script.rs:202-210 (`parse_chain` — left-associative
           `c::then_all(acc, rhs)`, binding TIGHTER than the `;`/`.`/`,`/`then` separators
           handled by `parse_seq` at :177-199, exactly as in Lean);
           proof/src/tactic.rs:1138-1146 (`then_all(t1, t2)` = `t1(st)?;
           all_goals(t2)(st)` — note that `all_goals` is applied to the state AFTER `t1`, with
           no record of which goals `t1` created);
           proof/src/tactic.rs:1124-1136 (`all_goals` iterates `st.open` — the COMPLETE open-goal
           deque — focusing each in turn and propagating failure with `t(st)?`);
           proof/src/tactic.rs:280-298 (`refine` pushes new subgoals to the FRONT of `open`, so
           after `split` the deque is [left, right, …any goals that were already there]);
           proof/src/tactic.rs:1091-1099, :1104-1119 (`try_` and `repeat` speculate on a CLONE
           and commit only on success — `then_all` does NOT, so a failure inside `t2` leaves
           whatever `t1` committed)
@guard     THE OPEN-SET LEAK, and the only real semantic difference in this case. Lean's `<;>`
           applies `t2` to exactly the goals `t1` produced. LOGOS's `then_all` applies it to
           `st.open` in full (tactic.rs:1126-1133), which also contains any goal that was
           already waiting from an EARLIER branching step. So in a script like
           `split . left <;> assumption`, Lean's `<;>` touches only the single goal `left`
           produced, while LOGOS's also touches the right conjunct that `split` left behind.
           When `assumption` happens to discharge it, the script succeeds and is now one goal
           further along than the translator believes, so every subsequent atom targets the
           wrong goal. When it does not, `all_goals` propagates the failure (`t(st)?`,
           tactic.rs:1130) and the whole chain fails — for a goal the Lean source never asked
           `t2` to look at.
@guard     Catches the "it usually works" corollary. The leak is invisible whenever `<;>` is the
           FIRST branching step of a proof, because `st.open` then contains only `t1`'s output.
           Almost every textbook use of `<;>` has that shape, so a translator can pass a large
           test suite and still be wrong in exactly the nested proofs where it matters.
@guard     Catches the ASSOCIATIVITY assumption. `parse_chain` folds left
           (tactic_script.rs:204-208), matching Lean, so `a <;> b <;> c` is `(a <;> b) <;> c`.
           But because `then_all`'s second stage is `all_goals`, the middle `b` has already
           spread across the whole open set before `c` runs, and `c` then spreads across the
           result of that. The fan-out compounds. A translator that models `<;>` as a simple
           pairing of goals will mispredict the goal count after two chained `<;>`s.
@guard     Catches PRECEDENCE against the separators. `<;>` binds tighter than `;`/`.`/`,`/the
           word `then` (tactic_script.rs:177-199 vs :202-210) — the same as Lean. But the LOGOS
           separators include the PERIOD, so a translator that emits prose punctuation inside a
           chain (`split . left <;> assumption`) gets a different tree from one that emits
           `split ; left <;> assumption` only by accident of where it put the period. The
           period is a separator, not decoration.
@guard     Catches assuming `<;>` backtracks. `try` and `repeat` clone the state before
           speculating (tactic.rs:1091-1119); `then_all` does not (tactic.rs:1141-1145). A
           `t1 <;> t2` whose `t2` fails on the third goal leaves `t1`'s effects and `t2`'s
           effects on the first two goals committed. Lean's `<;>` is atomic. A translator
           relying on all-or-nothing semantics to recover from a failed chain will read a
           half-applied state.
@oracle    Every Lean `<;>` maps to a `<;>` token in the emitted script with the same
           associativity. The harness runs `run_script` and asserts that the open-goal count
           after the chain equals the open-goal count Lean reports after the corresponding
           `<;>`; for `leakAfterSplit` below it asserts the translator did NOT emit a bare
           `<;>` but either restructured or recorded the divergence.
-/

namespace T08_018

/-- The clean shape: `<;>` is the first branching step, so `st.open` holds only `t1`'s goals. -/
theorem cleanChain (p : Prop) (hp : p) : p ∧ p := by
  constructor <;> exact hp

/-- Two goals from `constructor`, both discharged by the same tactic. -/
theorem bothByAssumption (p q : Prop) (hp : p) (hq : q) : p ∧ q := by
  constructor <;> assumption

/-- THE LEAK. `constructor` leaves two goals; the `left` acts on the first, and Lean's `<;>`
    then applies `assumption` only to what `left` produced. LOGOS's would also apply it to the
    second conjunct, which is still open. -/
theorem leakAfterSplit (p q : Prop) (hp : p) (hq : q) : (p ∨ q) ∧ q := by
  constructor
  · left <;> assumption
  · assumption

/-- CHAINED `<;>`: left-associative in both, but the LOGOS fan-out compounds across the whole
    open set at each stage. -/
theorem chained (p q r s : Prop) (hp : p) (hq : q) (hr : r) (hs : s) :
    (p ∧ q) ∧ (r ∧ s) := by
  constructor <;> constructor <;> assumption

/-- `<;>` after a CASE SPLIT: two branches, one closing tactic. -/
theorem afterCases (p q r : Prop) (h : p ∨ q) (hpr : p → r) (hqr : q → r) : r := by
  cases h <;> simp_all

/-- Precedence against the separator: `;` binds looser than `<;>` in both languages, so this is
    `(constructor) ; (exact hp <;> skip)` — not a chain over the first step. -/
theorem precedence (p : Prop) (hp : p) : p ∧ p := by
  constructor
  exact hp
  exact hp

/-- A chain whose right-hand tactic fails on one goal: Lean's `<;>` is atomic and the whole
    tactic fails, leaving nothing committed. Written here as the version that succeeds. -/
theorem partialFailureShape (p q : Prop) (hp : p) (hq : q) : p ∧ q := by
  constructor
  · exact hp
  · exact hq

end T08_018
