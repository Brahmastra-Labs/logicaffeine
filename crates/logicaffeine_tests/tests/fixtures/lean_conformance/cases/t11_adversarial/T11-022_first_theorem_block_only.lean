/-!
@case      T11-022
@tier      11 · Adversarial and soundness traps
@title     Only the FIRST `## Theorem` block is proved — a multi-theorem Lean file silently
           becomes a one-theorem document
@lean      a .lean file containing several theorems, a namespace of lemmas, a `section` of
           results, a theorem that depends on an earlier lemma in the same file
@expect    degrade
@target    One vernacular document PER Lean theorem, each compiled separately — never one
           document with several `## Theorem` blocks.
@evidence  logicaffeine_language/src/compile.rs:1070 (`pub fn compile_theorem(input: &str)`);
           logicaffeine_language/src/compile.rs:1105-1119 (the decisive lines: after parsing
           the whole program into `statements`, it does
           `let theorem = statements.iter().find_map(|stmt| if let Stmt::Theorem(t) = stmt
           { Some(t) } else { None }).ok_or_else(|| … "No theorem block found in input")?;`
           — `find_map` returns the FIRST match. Every later `Stmt::Theorem` in the parsed
           program is silently discarded, with no warning and no count);
           logicaffeine_language/src/compile.rs:1161 (the only diagnostic the function ever
           emits about a theorem's status is "derivation found but NOT kernel-certified (this
           is not a proof)" — there is nothing about discarded blocks);
           logicaffeine_proof/src/verify.rs:25-43 (`VerifiedProof.verified == true` iff a
           kernel term was produced and typechecked — it describes ONE theorem, so a
           `verified: true` result says nothing whatever about the rest of the document);
           LOGOS_FACTS: the three vernacular surfaces do not interoperate, so "put the lemmas
           on another surface" is not an escape
@loss      ONE_THEOREM_PER_DOCUMENT. A Lean file is a sequence of declarations; a LOGOS
           vernacular document proves exactly one theorem. The translation is therefore
           one-to-many at the file level, and the loss is the FILE STRUCTURE: the grouping, the
           ordering, and — critically — the intra-file dependencies, since a later Lean theorem
           that cites an earlier lemma must have that lemma's statement re-supplied as a
           `Given:` premise or re-proved, because there is no cross-document lemma environment.
           The loss record must name the theorem count and the dependency edges that were
           broken.
@guard     **TIER 11 THESIS, AT THE DOCUMENT LEVEL.** A successful LOGOS result is not evidence
           that the translation is complete. This is the quietest failure in the whole corpus:
           the translator concatenates a file's theorems into one document, calls
           `compile_theorem`, gets back a `VerifiedProof` with `verified == true`, and reports
           success. The result is true — of the FIRST theorem. The other nine were parsed into
           `Stmt::Theorem` values, passed over by `find_map` (compile.rs:1105-1119), and
           dropped. Nothing in the return value carries a count, so there is no field to
           check; the only way to know is to have counted the inputs.
@guard     Catches the silence being mistaken for absence of input. `compile_theorem` DOES
           error when there is no theorem at all — `"No theorem block found in input"`
           (compile.rs:1116-1119) — which trains the reader to expect a diagnostic when
           something is wrong with the theorem blocks. There is no corresponding diagnostic for
           "found ten, proved one".
@guard     Catches the intra-file dependency. Lean files are written as lemma-then-theorem
           chains, and after splitting into one document per theorem the later document has no
           access to the earlier one's result: each `compile_theorem` call builds its own
           kernel context from `StandardLibrary::register` plus that document's own premises
           (verify.rs:338-345). So the lemma must be re-stated as a `Given:` — which changes
           the theorem's statement — or re-proved inside the second document. A translator that
           splits the file but does not fix up the dependencies produces documents whose
           theorems are strictly weaker than the Lean originals, all of which certify.
@guard     Catches the "just use a different surface" move. Surface B (`## Axiom` / `## Theory`)
           accepts a sequence of `Axiom`/`Theorem` lines and so looks like the fix — but its
           `parse_term` accepts ONLY a bare identifier (formula.rs:427-432), so no arithmetic
           and no nested function terms survive the move; and Surface C (literate) has no
           `forall` in its `Statement:` grammar at all. The surfaces do not interoperate, so
           the choice is per-theorem and is itself a translation decision (T12-011).
@guard     Catches counting theorems in the SOURCE rather than in the parse. What
           `compile_theorem` iterates is `statements`, the parsed program
           (compile.rs:1103-1105), so the count that matters is how many `Stmt::Theorem`
           values the LOGOS parser produced from the emitted document — which may differ from
           the number of Lean theorems if the emitter merged or split any. The check must be
           on the emitted document.
@oracle    The translator emits exactly one vernacular document per Lean theorem, and each
           document contains exactly ONE `## Theorem` block; and a loss record with category
           ONE_THEOREM_PER_DOCUMENT names the source file's theorem count and every intra-file
           dependency that had to be re-supplied as a premise or re-proved. A harness asserts,
           for every emitted document, that the count of `## Theorem` blocks is 1, and that the
           number of documents emitted for this file is exactly TEN.
-/

namespace T11_022

/-! ### A file with several theorems — the ordinary case, and the one that silently loses
     everything after the first. There are TEN provable statements below — eight numbered
     theorems, one theorem with a premise, and one anonymous `example`. A translator that
     emits one document has translated ONE of them; a harness must see ten documents. -/

/-- 1. The first theorem. In a concatenated document this is the only one proved. -/
theorem firstOne (n : Nat) : n + 0 = n := Nat.add_zero n

/-- 2. Independent of the first. Silently dropped by `find_map`. -/
theorem secondOne (n : Nat) : 0 + n = n := Nat.zero_add n

/-- 3. Also independent. -/
theorem thirdOne (n m : Nat) : n + m = m + n := Nat.add_comm n m

/-! ### The dependency chain, which is what makes splitting non-trivial: each of these cites
     the previous one, and after splitting there is no shared lemma environment. -/

/-- 4. A lemma. -/
theorem doubleIsAdd (n : Nat) : 2 * n = n + n := by
  rw [Nat.two_mul]

/-- 5. Cites lemma 4. After splitting, `doubleIsAdd` is not in this document's context; its
    statement must be re-supplied as a `Given:` or the proof must be redone. -/
theorem doubleZero : 2 * 0 = (0 : Nat) := by
  rw [doubleIsAdd]

/-- 6. Cites lemma 5, two links down the chain. -/
theorem doubleZeroAgain : 2 * 0 + 1 = (1 : Nat) := by
  rw [doubleZero]

/-! ### A theorem stated with a premise, to mark what a `Given:` line can and cannot carry.
     A `Given:` goes through the full English parser, so a premise that is not expressible in
     English cannot be re-supplied this way — which is the point at which the dependency
     break becomes a rejection rather than a degrade (T12-002). -/

theorem withPremise (n : Nat) (h : 0 < n) : 0 < n + 1 := Nat.lt_of_lt_of_le h (Nat.le_succ n)

/-! ### An `example`, which is anonymous. It still parses as a provable statement and still
     needs a generated name (T02-005), and it still counts toward the document split. -/

example : (2 : Nat) + 2 = 4 := rfl

/-! ### A `def` between the theorems, to check that the splitter does not simply slice the
     file at `theorem` keywords: the definition must be carried into EVERY document that
     mentions it, because there is no cross-document definition environment either. -/

def triple (n : Nat) : Nat := 3 * n

/-- 7. Depends on a `def`, not on a lemma. -/
theorem tripleZero : triple 0 = 0 := rfl

/-- 8. Depends on the same `def`. Both documents must carry `triple`'s definition. -/
theorem tripleOne : triple 1 = 3 := rfl

end T11_022
