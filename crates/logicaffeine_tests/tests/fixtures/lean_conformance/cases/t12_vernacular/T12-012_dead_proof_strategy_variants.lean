/-!
@case      T12-012
@tier      12 · Vernacular surfaces
@title     `ProofStrategy::Induction` and `ProofStrategy::ByRule` are dead variants
@lean      any `theorem` whose Lean proof a translator would classify as "by induction"
           or "by a named rule" and encode as a proof-strategy annotation
@expect    reject
@target    (none — neither variant is constructible from surface text, and neither is
           consumed by any driver)
@evidence  language/src/ast/theorem.rs:41-61 — `ProofStrategy` has four variants:
           `Auto`, `Induction(String)`, `ByRule(String)`, `Script(String)`.
@evidence  The complete set of construction sites in the workspace, by grep for
           `ProofStrategy::`:
             · language/src/parser/mod.rs:6153  ⟶ `Auto`   (the literal word `Auto`)
             · language/src/parser/mod.rs:6171  ⟶ `Auto`   (an empty `Proof:` line)
             · language/src/parser/mod.rs:6173  ⟶ `Script` (everything else)
             · language/src/parser/mod.rs:6178  ⟶ `Auto`   (no `Proof:` line at all)
             · language/src/ast/theorem.rs:64   ⟶ `Auto`   (the `Default` impl)
           `Induction` and `ByRule` are constructed NOWHERE.
@evidence  The complete set of consumption sites:
             · language/src/compile.rs:1136 — `if let ProofStrategy::Script(src) = …`,
               with everything else falling to the `Auto` arm at :1152.
             · compile/src/ui_bridge.rs:635 — `ProofStrategy::Script(s) => Some(s.clone())`
             · compile/src/ui_bridge.rs:644 — `is_auto: matches!(…, ProofStrategy::Auto)`
           No driver has an arm for `Induction` or `ByRule`. Even if one were
           constructed, `compile.rs:1136` would route it to `prove_certify_check`, i.e.
           it would behave as `Auto` while carrying a variable name nothing reads.
@evidence  language/src/ast/theorem.rs:47-54 — both dead variants carry doc comments with
           worked examples (`Proof: Induction on n.`, `Proof: ModusPonens.`). Those are
           the two spellings an implementer will copy.
@evidence  language/src/parser/mod.rs:6157-6174 — the fall-through that turns both
           spellings into `Script(<joined lexemes>)`.
@evidence  proof/src/tactic_script.rs:308-327 — `canonical` has no arm mapping
           `modusponens` (or any inference-rule name) to a tactic, so a `ByRule`-style
           spelling dies at tactic_script.rs:261-262 with ``unknown tactic `ModusPonens` ``.
@evidence  proof/src/tactic_script.rs:290 + :346-351 — an `Induction`-style spelling dies
           with `"trailing tokens after the script"` instead, because `induction` is
           zero-argument. The two dead variants therefore fail DIFFERENTLY, and neither
           failure mentions strategies.
@diag      E_DEAD_PROOF_STRATEGY — must state that the `ProofStrategy` enum's
           `Induction`/`ByRule` variants are unreachable from surface text, name the
           spelling the translator was about to emit, and give the live alternative
           (`Auto`, or a `Script` built from the tactic vocabulary at
           tactic_script.rs:275-292). Must emit no `Proof:` line naming an inference
           rule or an induction variable.
@guard     Catches an implementer who designs the emitter around the TYPE. Reading
           `ast/theorem.rs:41-61` gives a four-way classification of Lean proofs — auto,
           induction, single-rule, script — which is a perfectly reasonable design and
           maps neatly onto Lean's `by decide` / `by induction` / `by exact` / general
           tactic blocks. Two of those four branches can never be exercised. A
           translator built on that classification will emit text for half its cases that
           is not merely unsupported but *misparsed into the other branch*.
@guard     Catches the belief that an unrecognised `Proof:` line is a parse error. It is
           not (parser/mod.rs:6157-6174 accepts any lexeme sequence), so a translator
           that validates its output by re-parsing will see no problem. The failure
           appears only at `compile_theorem`, and both dead spellings surface as theorem
           failures whose `Error:` field talks about scripts — a harness matching on
           "proof search failed" will classify them as "LOGOS is not strong enough".
@guard     Catches the near-miss `Proof: Induction.` — which is LIVE, because `Induction`
           canonicalises to the zero-argument `induction` tactic and the trailing `.` is a
           separator. It parses, it runs, and it inducts on whatever the focused goal's
           outermost `∀` binds, which is not necessarily the Lean variable. A translator
           that "fixes" the dead spelling by deleting the variable has converted a loud
           failure into a silent one. See T12-005.
@guard     Catches encoding an inference rule name in a different position — e.g. the
           theorem NAME or a `cites` list — on the theory that `ByRule` was meant for it.
           Surface A has no citation mechanism at all; `cites` belongs to Surface B's
           `Theorem <name> cites <n1>, <n2>: …` (proof/src/development.rs:20-22), and
           its argument is a THEOREM name, never an inference-rule name. Cited names that
           do not match a theorem in the development are silently treated as external
           givens (proof/src/verify.rs:110-113) — no diagnostic.
@oracle    The translator's emitted text contains no `Proof:` line whose content is
           an inference-rule name or `Induction <var>`. Negative oracle for a harness:
           `compile_theorem` on a block with `Proof: Induction on n.` returns `Err`
           containing `"trailing tokens after the script"`, and on `Proof: ModusPonens.`
           returns `Err` containing `"unknown tactic"`. If either returns `Ok`, the
           LOGOS build under test is not the one this corpus is written against.
@note      INDEX.md classes this `reject`, and that is right for the emitter. Note the
           asymmetry it creates for a CONSUMER, though: a translator that reads a LOGOS
           document (rather than writing one) must still handle all four variants in its
           `match`, because the enum is public and a future LOGOS release could start
           constructing them. Treating the dead pair as `unreachable!()` is a latent
           panic.
-/

namespace T12_012

/-- The proof a translator would classify as `Induction(n)`. -/
theorem byInduction (P : Nat → Prop) (base : P 0) (step : ∀ k, P k → P (k + 1)) :
    ∀ n, P n := by
  intro n
  induction n with
  | zero => exact base
  | succ k ih => exact step k ih

/-- The proof a translator would classify as `ByRule("ModusPonens")`: one application of
    one rule, exactly the shape the `ByRule` doc comment invites. -/
theorem byModusPonens (P Q : Prop) (hpq : P → Q) (hp : P) : Q :=
  hpq hp

/-- `ByRule("UniversalInst")`: one ∀-elimination. Also nothing to encode. -/
theorem byUniversalInst (α : Type) (P : α → Prop) (h : ∀ x, P x) (a : α) : P a :=
  h a

/-- The two LIVE classifications, for contrast: this one is `Auto` (nothing to say about
    the proof) … -/
theorem liveAuto (P : Prop) (hp : P) : P := hp

/-- … and this one is `Script`, because the tactic steps map onto the vocabulary at
    tactic_script.rs:275-292 (`cases`, `right`/`left`, `assumption`). -/
theorem liveScript (P Q : Prop) (hp0 : P ∨ Q) : Q ∨ P := by
  cases hp0 with
  | inl hp => exact Or.inr hp
  | inr hq => exact Or.inl hq

end T12_012
