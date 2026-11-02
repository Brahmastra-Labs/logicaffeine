/-!
@case      T12-005
@tier      12 · Vernacular surfaces
@title     `Proof: Induction on n.` — LOGOS's own documented spelling does not parse
@lean      `induction n with | zero => … | succ k ih => …`
@expect    reject
@target    (none — the documented spelling is not accepted, and the accepted spelling
           cannot carry a Lean `induction` proof; see @note for the correct target)
@evidence  language/src/ast/theorem.rs:47-50 — the doc comment on
           `ProofStrategy::Induction(String)` reads, verbatim:

               /// Induction on a variable (for inductive types like Nat, List).
               /// Example: `Proof: Induction on n.`

           This is the documentation an implementer will read, and it is wrong.
@evidence  language/src/parser/mod.rs:6151-6174 — the `Proof:` line has exactly TWO
           outcomes. `if self.check(&TokenType::Auto)` yields `ProofStrategy::Auto`;
           EVERYTHING else is collected lexeme-by-lexeme to the end of the block and
           joined into `ProofStrategy::Script(script)`. `ProofStrategy::Induction` is
           never constructed here or anywhere — the only four construction sites in the
           workspace are parser/mod.rs:6153, :6171, :6173 and :6178, and they build only
           `Auto` and `Script`.
@evidence  proof/src/tactic_script.rs:290 — `"induction" => Ok(c::induction())`. The
           primitive is ZERO-ARGUMENT: unlike `intro`/`exact`/`cases`/`rewrite`/`exists`
           (tactic_script.rs:276-280) it never calls the `arg()` closure, so it consumes
           no following word.
@evidence  proof/src/tactic_script.rs:298-303 — `is_filler` skips `on`, but ONLY at the
           start of an atom or before an argument (`skip_filler` is called from
           `parse_atom`, :212, and from the `arg` closure, :265). After a zero-argument
           primitive returns, control is back in `parse_chain`/`parse_seq`, where no
           filler skip happens.
@evidence  proof/src/tactic_script.rs:162-166 — `at_separator` accepts only `Semi`
           (lexed from `;`, `.`, `,` — see :61-64) or the literal word `then`. `on` is
           neither, so `parse_seq` breaks immediately.
@evidence  proof/src/tactic_script.rs:346-351 — the leftover tokens then trip
           `return Err(ScriptError(format!("trailing tokens after the script: {:?}", …)))`.
           For `"Induction on n ."` the reported residue is
           `[Word("on"), Word("n"), Semi]`.
@evidence  language/src/compile.rs:1145-1149 — `Err(e) => fail(e.to_string())`, and
           `Display for ScriptError` (tactic_script.rs:33-37) prefixes
           `"tactic script error: "`. With `derivation: None`, compile.rs:1165-1175
           returns `Err(ParseError::Custom("Theorem '…' failed. … Error: tactic script
           error: trailing tokens after the script: [Word(\"on\"), Word(\"n\"), Semi]"))`.
@diag      E_DEAD_PROOF_STRATEGY_SPELLING — must state that `Proof: Induction on n.` is
           parsed as `Script("Induction on n .")`, that `induction` is a zero-argument
           tactic, and that the variable name is unrepresentable. Must NOT silently
           rewrite the user's intent to `Proof: Auto.`
@guard     This is the single most likely thing to mislead an implementer in the whole
           tier, because the wrong spelling is in LOGOS's OWN source documentation and
           the right one is not documented anywhere. An implementer who reads
           `ast/theorem.rs:43-60`, sees four `ProofStrategy` variants with examples, and
           writes an emitter that maps Lean's `induction n` to `Proof: Induction on n.`
           has written code that is consistent with the type and consistent with the
           docs and cannot work.
@guard     Catches the belief that this fails at PARSE time. It does not: parser/mod.rs
           accepts the line unconditionally (it accepts any English at all) and builds a
           `Script`. The failure happens later, inside `compile_theorem`, and is reported
           as a *theorem failure* whose `Error:` field mentions trailing tokens — a
           message a harness scanning for "proof search failed" will misclassify as
           "LOGOS could not prove it" rather than "we emitted invalid syntax".
@guard     Catches "fix it by dropping the variable": `Proof: Induction.` DOES parse
           (`Induction` ⟶ `induction`, `.` ⟶ separator, nothing left over) — and that is
           worse, because the variable the Lean proof inducted on is now gone. LOGOS's
           `ProofState::induction` (proof/src/tactic.rs:561-570) inducts on whatever
           variable the FOCUSED GOAL's outermost `∀` binds. If the Lean theorem inducted
           on a variable that is not the goal's first binder, the two proofs are about
           different things, and the LOGOS one may still certify.
@guard     Catches "fix it with the sibling variant": `Proof: ModusPonens.` (the
           documented example for `ProofStrategy::ByRule`, theorem.rs:52-54) fails
           differently and more quietly — `canonical("modusponens")` returns `None`
           (tactic_script.rs:308-327 has no such arm), so the error is
           ``unknown tactic `ModusPonens` `` from tactic_script.rs:262. Both dead
           variants are covered by T12-012.
@oracle    Translator exits non-zero with E_DEAD_PROOF_STRATEGY_SPELLING and emits no
           `Proof: Induction …` line anywhere in its output. Negative oracle for a
           harness: feeding the documented spelling to `compile_theorem` must return
           `Err`, and the error string must contain `"trailing tokens after the script"`.
@note      The accepted spellings, quoted verbatim from the suite, are
           `"induction; auto; auto"` (proof/tests/tactic_script.rs:150-155,
           `script_induction_over_nat`, asserted kernel-verified) and its English form
           `"By induction. Automatically. Automatically."`
           (proof/tests/tactic_script.rs:195-200, `english_induction_proof`, likewise
           asserted kernel-verified). Both are run through `ProofState::run_script`
           directly; no test in the suite drives either of them through a Surface A
           `Proof:` line, so a translator that emits one into a `## Theorem` block is
           relying on the lexeme-collection round trip (parser/mod.rs:6157-6174) being
           transparent, which is untested.
@note      Even with the correct spelling, Surface A cannot carry a Lean `induction n`
           proof. `ProofState::induction` (proof/src/tactic.rs:561-600) substitutes the
           bound variable with `ProofTerm::Constant("Zero")` and
           `ProofTerm::Function("Succ", [K<n>])` — so the premises must be English
           sentences about `Zero` and `Succ`, which is precisely the term arithmetic
           Surface A does not have (T12-002). The correct target for a Lean `induction`
           proof over `Nat` is Surface C (T12-010) or the raw kernel path (T12-011).
-/

namespace T12_005

/-- The Lean proof whose translation is at issue: structural induction naming the
    variable, with named cases and a named induction hypothesis. Every one of those
    names is unrepresentable in a LOGOS `Proof:` script. -/
theorem allP (P : Nat → Prop) (base : P 0) (step : ∀ k, P k → P (k + 1)) :
    ∀ n, P n := by
  intro n
  induction n with
  | zero => exact base
  | succ k ih => exact step k ih

/-- Induction on a variable that is NOT the goal's outermost binder. `Proof: Induction.`
    would induct on `m`, not `n` — a different proof of a different shape, which may
    still certify. -/
theorem addComm' (n m : Nat) : n + m = m + n := by
  induction n with
  | zero => simp
  | succ k ih => omega

/-- Induction on a list: script-level `induction` is `Nat`-only
    (tactic.rs:583-586 hard-codes `Zero`/`Succ`), so this has no script spelling at all.
    See T08-010 and T12-010. -/
theorem lengthAppendNil (α : Type) (xs : List α) : (xs ++ []).length = xs.length := by
  induction xs with
  | nil => rfl
  | cons _ t ih => simp

end T12_005
