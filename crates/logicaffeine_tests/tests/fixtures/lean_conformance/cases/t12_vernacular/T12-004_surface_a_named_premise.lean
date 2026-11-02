/-!
@case      T12-004
@tier      12 · Vernacular surfaces
@title     Surface A named premise — `Given (h):` binds the hypothesis name a script uses
@lean      named hypothesis binder in a theorem signature, referenced by name from the
           tactic proof
@expect    translate
@target    A `Given (<name>):` premise line, whose name lands in
           `TheoremBlock::premise_names` and reaches
           `ProofState::start_with_names` (compile.rs:1138-1139):

           ```logos
           ## Theorem: Or_Comm_Named
           Given (h): Bob is happy or Bob is sad.
           Prove: Bob is sad or Bob is happy.
           Proof: By cases on h. Right, by assumption. Left, by assumption.
           ```
           ```logos
           ## Theorem: Some_Mortal_Is_A_Man
           Given (h): Some man is mortal.
           Prove: Some mortal is a man.
           Proof: By cases on h. By automation.
           ```
@evidence  tests/e2e_studio_examples.rs:336-350 and :365-379 — both blocks above are
           quoted verbatim from those tests, each asserted `Ok` and `Proved`;
           language/src/parser/mod.rs:6079-6100 — the `Given (h):` form: an `LParen`
           immediately after `Given`, one token taken as the name, `RParen`, then the
           colon. `premise_names.push(this_name)` runs on EVERY `Given`, pushing `None`
           for an unnamed one, so the vector is positionally parallel to `premises`;
           language/src/ast/theorem.rs:29-33 (`premise_names: Vec<Option<String>>`,
           documented as "so a `Proof:` script can refer to it as `cases h` rather than
           the positional `hp0`");
           language/src/compile.rs:1138-1139 (`ProofState::start_with_names(premises,
           &theorem.premise_names, goal)`)
@guard     Catches a translator that names premises but proves by `Auto`. The names are
           threaded ONLY through the `Script` branch: compile.rs:1136 tests
           `if let ProofStrategy::Script(src) = &theorem.strategy`, and the `else` arm
           (compile.rs:1152) calls `prove_certify_check(&premises, &goal)` — which takes
           no names at all. Naming a premise costs nothing there, but a translator that
           believes the name is part of the proposition is wrong: it is proof-script
           scaffolding only.
@guard     Catches the positional/named mismatch. Premise names are POSITIONAL: the
           unnamed `Given:` lines in a block are still numbered `hp0`, `hp1`, … by
           position among ALL premises, not among the unnamed ones. A translator that
           emits `Given (h): …` first and then an unnamed `Given: …` must refer to the
           second as `hp1`, not `hp0`. Getting this wrong yields a `run_script` failure
           that reports a missing hypothesis, which reads like a proof-search failure.
@guard     Catches emitting a name that is also a tactic word. The name is consumed as a
           single token by parser/mod.rs:6087-6090 with no keyword check, and it is then
           spliced into script text where `canonical` (tactic_script.rs:308-327) matches
           case-insensitively. `Given (left): …` followed by `Proof: By cases on left.`
           parses as `cases` applied to … the filler-skipped next word, because `left`
           is itself a zero-argument tactic. Hypothesis names must be emitted from a
           reserved-word-avoiding namespace (`h0`, `h1`, …), never from the Lean binder
           name verbatim.
@guard     Catches assuming the parenthesised name may be multi-token. Only ONE token is
           taken (parser/mod.rs:6087-6090 advances exactly once inside the parens), so
           `Given (h_or_case):` is fine but `Given (my premise):` silently names the
           premise `my` and leaves `premise` to be consumed as part of the colon scan.
@oracle    `compile_theorem(<emitted text>)` returns `Ok(s)` with
           `s.contains("[kernel-verified]")`, and the emitted script refers to the
           premise by the name given in parentheses. A differential check strengthens
           it: replacing `Given (h):` with `Given:` and `cases h` with `cases hp0` must
           produce the same verdict.
-/

namespace T12_004

variable (P Q : Prop)

/-- `Given (h): Bob is happy or Bob is sad.` — the hypothesis is named in the Lean
    signature and referenced by that name in the tactic proof, exactly as the LOGOS
    block does. -/
theorem orCommNamed (h : P ∨ Q) : Q ∨ P := by
  cases h with
  | inl hp => exact Or.inr hp
  | inr hq => exact Or.inl hq

axiom Entity : Type
axiom Man : Entity → Prop
axiom Mortal : Entity → Prop

/-- `Given (h): Some man is mortal.` / `Prove: Some mortal is a man.`
    ∃-elimination: LOGOS's `cases h` on an existential binds `h_w` plus a fresh witness
    constant, then `By automation.` re-assembles the conjunct-swapped goal. -/
theorem someMortalIsAMan (h : ∃ x, Man x ∧ Mortal x) : ∃ x, Mortal x ∧ Man x := by
  cases h with
  | intro w hw => exact ⟨w, hw.right, hw.left⟩

/-- Two premises, one named and one not. The unnamed one keeps its POSITIONAL name
    `hp1` — position among all premises, not among the unnamed ones. -/
theorem mixedNaming (h : P ∨ Q) (hp1 : Q → P) : P := by
  cases h with
  | inl hp => exact hp
  | inr hq => exact hp1 hq

end T12_004
