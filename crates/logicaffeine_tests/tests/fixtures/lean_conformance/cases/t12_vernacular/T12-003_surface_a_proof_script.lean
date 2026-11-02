/-!
@case      T12-003
@tier      12 · Vernacular surfaces
@title     Mapping a Lean tactic block onto a Surface A `Proof:` script
@lean      `by` tactic block: intro, cases (with `inl`/`inr`/`intro` alternatives),
           constructor / Or.inl / Or.inr, assumption
@expect    degrade
@target    A `## Theorem` block whose `Proof:` line is anything other than the bare word
           `Auto`, which parser/mod.rs:6157-6174 collects verbatim and wraps as
           `ProofStrategy::Script(<joined lexemes>)`; `compile_theorem` then runs it
           through `ProofState::run_script` and `qed()` (compile.rs:1136-1150).

           The three verbatim scripts this case targets, quoted from the suite:

           ```logos
           ## Theorem: Sad_Implies_Happy
           Given: Bob is happy.
           Prove: If Bob is sad then Bob is happy.
           Proof: Assume h. By assumption.
           ```
           ```logos
           ## Theorem: Or_Commutative
           Given: Bob is happy or Bob is sad.
           Prove: Bob is sad or Bob is happy.
           Proof: By cases on hp0. Right, by assumption. Left, by assumption.
           ```
           ```logos
           ## Theorem: And_Commutative
           Given: Bob is happy and Bob is tall.
           Prove: Bob is tall and Bob is happy.
           Proof: By cases on hp0. Split, by assumption, by assumption.
           ```
@evidence  tests/e2e_studio_examples.rs:292-305, :306-320, :321-335 — the three blocks
           above are quoted verbatim from those tests, each asserted `result.is_ok()`
           and `output.contains("Proved")`;
           proof/src/tactic_script.rs:275-292 (the primitive table:
           `intro`/`exact`/`cases`/`rewrite`/`exists` take one argument;
           `assumption`/`simp`/`decide`/`omega`/`crush`/`split`/`left`/`right`/`auto`/`induction`
           take none);
           proof/src/tactic_script.rs:308-327 (`canonical`: the alias table —
           `assume`/`suppose`/`let`/`fix` ⟶ `intro`, `destruct`/`consider` ⟶ `cases`,
           `both`/`constructor` ⟶ `split`, `automatically`/`trivially` ⟶ `auto`);
           proof/src/tactic_script.rs:298-303 (`is_filler`: `by the on that now with we
           it a an of to and` are skipped, which is what makes "By cases on hp0" mean
           `cases hp0`);
           proof/src/tactic_script.rs:61-64 (`.`, `,` and `;` all lex to the SAME
           separator token, so sentence punctuation is sequencing);
           language/src/parser/mod.rs:6079-6094 (unnamed premises are named positionally
           `hp0`, `hp1`, … — see also T12-004)
@loss      TACTIC_GRANULARITY. Every one of these is a real, silent change of meaning
           relative to the Lean tactic block, and each must be recorded:
           · `exact` takes a HYPOTHESIS NAME, never a term (tactic_script.rs:277,
             `c::exact(&arg()?)` where `arg` returns a `String`). Lean's `exact
             Or.inr hp` has no rendering; it must be re-expressed as `right; assumption`.
           · `cases` has no pattern language and no case names. Binder names are
             assigned by LOGOS: `A ∧ B` ⟶ `h_1`,`h_2`; `A ∨ B` ⟶ `h_l`,`h_r`;
             `∃` ⟶ `h_w` plus a fresh constant. Lean's `with | inl hp => …` naming is
             dropped, and the two branches are sequenced by punctuation rather than
             addressed by name.
           · `rewrite` rewrites EVERY occurrence; Lean's `rw [h]` rewrites the first
             match and `conv`/positional targeting does not exist.
           · `simp` takes no lemma list; `simp only [foo]` cannot be expressed.
           · `exists` takes a bare constant (`c::exists(ProofTerm::Constant(arg()?))`,
             tactic_script.rs:280) — no witness terms.
           · `apply`, `induction_over` and `all_goals` are Rust-API-only and have NO
             script spelling (tactic_script.rs:275-292 has no arm for them).
           Because branch structure is carried by *sequencing*, the mapping is only
           sound when the Lean block's goals are discharged in the order LOGOS spawns
           them. That ordering is an implementation detail of the tactic engine, not a
           contract — so the loss record must also carry GOAL_ORDER_DEPENDENCE.
@guard     Catches a translator that transcribes the Lean tactic block token for token.
           `intro hp` is fine; `exact (Or.inr hp)` is a `ScriptError` at best — but
           `exact Or` (if the emitter strips the application) is NOT an error, it is a
           lookup for a hypothesis named `Or`, which fails at run time with a message
           about hypotheses, not about terms. The translator must lower Lean's
           term-valued `exact` into the LOGOS constructor tactics (`left`/`right`/`split`)
           plus `assumption`, or refuse.
@guard     Catches assuming `Proof:` content is validated at parse time. It is not:
           parser/mod.rs:6157-6174 collects every lexeme up to the next `##` header, a
           `Dedent`, or EOF and joins them with spaces. Any English at all becomes a
           `Script(String)`. The failure surfaces only when `run_script` is called
           inside `compile_theorem` (compile.rs:1145-1149), as a `ParseError::Custom`
           carrying a `ScriptError` — so a translator that "parses" its own output by
           calling the LOGOS *parser* will never see the problem.
@guard     Catches capitalisation assumptions. `canonical` is case-insensitive
           (tactic_script.rs:309, `word.to_ascii_lowercase()`), so `Assume` and `assume`
           both work — but `first`, `try` and `repeat` are matched as EXACT lowercase
           words in `parse_atom` (tactic_script.rs:221, :232, :236). `Try assumption.`
           is not `try assumption`; it is an unknown tactic named `Try`.
@oracle    `compile_theorem(<emitted text>)` returns `Ok(s)` with
           `s.contains("[kernel-verified]")`, AND the loss record names
           TACTIC_GRANULARITY. A translator that emits `Proof: Auto.` for a theorem
           whose Lean proof was a tactic block has not failed this case — but it must
           then record STRATEGY_ERASURE, because `Auto` is a different proof object
           (T12-013, T12-014).
-/

namespace T12_003

variable (P Q : Prop)

/-- `Proof: Assume h. By assumption.` — `intro` then `assumption`.
    Note the Lean hypothesis `hp0` is deliberately named to match the positional name
    LOGOS assigns an unnamed `Given:` (parser/mod.rs:6079-6094). -/
theorem sadImpliesHappy (hp0 : Q) : P → Q := by
  intro _h
  assumption

/-- `Proof: By cases on hp0. Right, by assumption. Left, by assumption.`
    The Lean block names the branch binders; LOGOS names them `h_l` / `h_r` and the
    two branches are ordered by punctuation. -/
theorem orCommutative (hp0 : P ∨ Q) : Q ∨ P := by
  cases hp0 with
  | inl hp => exact Or.inr hp
  | inr hq => exact Or.inl hq

/-- `Proof: By cases on hp0. Split, by assumption, by assumption.`
    `cases` on a conjunction is ∧-elimination; `split` is `constructor`. Lean's
    anonymous-constructor `exact ⟨hq, hp⟩` has no script spelling — it must become
    `split` followed by two `assumption`s. -/
theorem andCommutative (hp0 : P ∧ Q) : Q ∧ P := by
  cases hp0 with
  | intro hp hq => exact ⟨hq, hp⟩

/-- A Lean block that has NO faithful script: `exact` here needs a term, and `apply`
    is unreachable from script text. This one must be re-derived, not transcribed. -/
theorem needsTermExact (h : P → Q) (hp : P) : Q := by
  exact h hp

end T12_003
