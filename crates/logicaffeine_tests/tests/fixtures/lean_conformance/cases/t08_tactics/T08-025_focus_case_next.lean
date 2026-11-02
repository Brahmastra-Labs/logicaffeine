/-!
@case      T08-025
@tier      8 · Tactics
@title     `·` focusing, `case`, `next` — the script's separators are FLAT and goals have no
           names or brackets
@lean      · focus dot, case <tag> => …, next => …, case' , focus
@expect    reject
@target    (none — no sound translation)
@evidence  proof/src/tactic_script.rs:177-199 (`parse_seq` — a script is a FLAT list of chains
           separated by `;`/`.`/`,`/the word `then`; there is no block, no indentation
           sensitivity, and no goal selector in the grammar at :9-17);
           proof/src/tactic_script.rs:215-220 (parentheses group a SUB-SCRIPT — `'(' script ')'`
           — but grouping is not focusing: the inner script still runs against `st.open` as it
           finds it, and leaves whatever it leaves);
           proof/src/tactic.rs:280-298 (`refine` — new subgoals are pushed to the FRONT of the
           `open` deque, in order, so execution is depth-first on the first subgoal and the
           following atoms simply continue into the next one when it closes);
           proof/src/tactic.rs:104-111, :83-90 (`ProofState`'s goals are `VecDeque<usize>`
           indices into a `Vec<Node>` — a `Goal` is `{ hyps, target }` (:around 30-40) with NO
           name, tag or label field, so there is nothing for `case <tag>` to select on);
           proof/src/tactic.rs:528-532, :401-405, :604 (`cases`, `split` and `induction` all
           produce their subgoals positionally, with no constructor tag attached);
           proof/src/tactic.rs:1124-1136 (`all_goals` — the only goal-selection mechanism in the
           system, and it selects ALL of them; it is not reachable from script text, see
           T08-020)
@diag      E_NO_GOAL_FOCUSING — must name the construct (`·`, `case`, `next`, `focus`), report
           how many goals the preceding step produced, and state that the script grammar is a
           flat sequence with no focusing bracket and that `Goal` carries no tag. Must NOT emit
           a flattened script.
@guard     THE FLATTENING TRAP, and the reason this is a rejection rather than a degrade. A
           translator that strips the `·` bullets and concatenates the blocks with `;` produces
           a script that WORKS — as long as every block closes exactly the one goal it was
           written for. Because `refine` pushes subgoals to the front (tactic.rs:296-297),
           execution is depth-first in exactly the order Lean's bullets appear, so the happy
           path is indistinguishable. The moment a block leaves a goal open, or closes two, the
           blocks silently misalign: the second bullet's tactics run against the first bullet's
           residue. The proof either fails somewhere far from the cause, or — when the tactics
           are generic enough (`assumption`, `simp`, `auto`) — SUCCEEDS, having proved the goals
           in an order and a pairing the source never specified. Lean's `·` exists precisely to
           make that impossible: it is an error for a focused block to leave its goal open.
@guard     Catches `case <tag>`. Lean's case tags come from the inductive's constructor names
           (`inl`, `inr`, `zero`, `succ`, `intro`). LOGOS's `Goal` has no tag field at all, and
           `cases`/`split`/`induction` attach none (tactic.rs:528-532, :401-405, :604). So `case`
           cannot be translated even by reordering: there is nothing to reorder BY. A translator
           that maps `case inr => …` to "the second goal" is assuming the LOGOS ordering matches
           Lean's constructor order — true today for `∨` (left then right) and `Nat` (base then
           step), and an undocumented coincidence rather than a guarantee.
@guard     Catches `next`. `next => …` focuses the FIRST goal without naming it, which is the
           closest thing to a flat sequence — and is therefore the case a translator is most
           likely to flatten silently. It still carries the "must close this goal" obligation
           that flattening discards.
@guard     Catches using PARENTHESES as focusing. `parse_atom` accepts `( script )`
           (tactic_script.rs:215-220), which looks like a block. It is not: it is precedence
           grouping. The inner script sees the same `st.open` and can close, open or leave any
           number of goals. Emitting `( … ) . ( … )` for two bullets changes nothing about
           which goals the second group runs on.
@guard     Catches the "just use `<;>`" alternative. `t1 <;> t2` applies the SAME tactic to
           every goal (and, per T08-018, to goals from earlier steps as well). Bullets exist
           because the branches need DIFFERENT tactics. The two are not interchangeable in
           either direction.
@oracle    Translator exits non-zero with E_NO_GOAL_FOCUSING for every theorem below and emits
           no script. A harness additionally asserts, for every multi-goal step anywhere in the
           corpus, that the translator recorded how many goals each emitted atom consumed — so
           that a flattened emission cannot be mistaken for a focused one.
-/

namespace T08_025

/-- Two bullets, each closing exactly one goal. The flattening happens to work — which is what
    makes it dangerous. -/
theorem bulletsHappyPath (p q : Prop) (hp : p) (hq : q) : p ∧ q := by
  constructor
  · exact hp
  · exact hq

/-- The first bullet's block needs TWO steps. Flattened, the boundary between the blocks is
    invisible and the second bullet's tactics could run against the first's residue. -/
theorem bulletsMultiStep (p q r : Prop) (hp : p) (hq : q) (hr : r) : (p ∧ q) ∧ r := by
  constructor
  · constructor
    · exact hp
    · exact hq
  · exact hr

/-- `case` with CONSTRUCTOR TAGS, and in the OPPOSITE order from the goals' natural order —
    the reordering a flat script cannot express at all. -/
theorem caseTagsReordered (p q : Prop) (hp : p) (hq : q) : p ∧ q := by
  constructor
  case right => exact hq
  case left => exact hp

/-- `case` tags from a disjunctive elimination. -/
theorem caseFromCases (p q r : Prop) (h : p ∨ q) (hpr : p → r) (hqr : q → r) : r := by
  cases h
  case inl hp => exact hpr hp
  case inr hq => exact hqr hq

/-- `next` — focusing without naming. The closest shape to a flat sequence, and the one a
    translator will flatten without noticing. -/
theorem nextBlocks (p q : Prop) (hp : p) (hq : q) : p ∧ q := by
  constructor
  next => exact hp
  next => exact hq

/-- `case` tags from an INDUCTION, where LOGOS's positional order (base then step) happens to
    match — an undocumented coincidence a translator must not rely on. -/
theorem caseFromInduction : ∀ n : Nat, n + 0 = n := by
  intro n
  induction n
  case succ k ih => simp
  case zero => rfl

/-- A block that deliberately leaves work for the NEXT block would be an error in Lean; the
    bullet enforces closure. Written here as the correct version. -/
theorem bulletEnforcesClosure (p q : Prop) (hp : p) (hq : q) : p ∧ q := by
  refine ⟨?_, ?_⟩
  · exact hp
  · exact hq

end T08_025
