/-!
@case      T06-017
@tier      6 · Propositions, equality, classical reasoning
@title     `sorry` / `sorryAx` — must NEVER reach the LOGOS context, in any form
@lean      sorry, sorryAx, `admit`, tactic-mode `sorry`, a `sorry`-tainted transitive dependency
@expect    reject
@target    (none — no sound translation, and no degraded one either)
@evidence  a grep of `crates/logicaffeine_kernel/src` for `sorry` returns NOTHING — LOGOS has
           no incomplete-proof marker of any kind; kernel/src/term.rs:443-545 (the complete
           `Term` enum: the only "unknown" variant is `Hole`, and `Hole` is not an admission of
           a proposition — it is a type wildcard); kernel/src/type_checker.rs:1594-1597
           (`types_equal` treats `Hole` as matching ANYTHING, so a `Hole` is strictly worse than
           an axiom, not better); kernel/src/context.rs:434 (`add_declaration(name, ty)` — the
           only way to assert a proposition without a proof, i.e. an axiom);
           kernel/src/certificate.rs:28 (`PRELUDE_VERSION = "logos-coc-1"`);
           kernel/src/certificate.rs:63-75 (`recheck` rebuilds the trusted context from
           `StandardLibrary::register` and "never trust[s] a context the certificate might
           carry"); crates/logicaffeine_kernel/src/verify.rs:31-43 via LOGOS_FACTS
           (`VerifiedProof.verified == true` iff a kernel term was produced AND typechecked —
           which a `sorryAx` application would do, since it is well-typed by construction)
@diag      E_INCOMPLETE_PROOF — must name the declaration, name `sorryAx`, and state that no
           output was produced. It must be a hard error, not a warning, and it must be
           non-suppressible. A translator that offers a `--allow-sorry` flag has built the bug.
@guard     THE TRAP: `ctx.add_declaration("sorryAx", Π(A : Type 0). A)`. That one line is a
           proof of every proposition in the context. Because LOGOS's `Prop` is impredicative
           and its `Pi` formation uses `imax` (type_checker.rs:108), `sorryAx False` inhabits
           `False`, and `False` has zero constructors so it large-eliminates into anything
           (type_checker.rs:1688-1716). EVERY downstream theorem in the library becomes
           vacuously "verified": `verified == true` really does hold, because a kernel term was
           produced and it really does typecheck. The trust door
           (`prove_certify_check` → `check_derivation`) cannot tell the difference — its whole
           contract is "a term was produced and typechecked", and that contract is satisfied.
@guard     Catches the "it's only one axiom, and `PRELUDE_VERSION` will catch it" defence. It
           will not. `PRELUDE_VERSION` is a single constant string (certificate.rs:28) compared
           for equality at certificate.rs:65; it describes the PRELUDE, and a translator-added
           declaration is not part of the prelude, so the string does not change. The only thing
           that catches an added axiom is `recheck` rebuilding the context
           (certificate.rs:73-75) and then failing with `UnboundVariable("sorryAx")` — a
           message that reads like a name-resolution bug and invites exactly the wrong fix
           (adding it to the prelude).
@guard     Catches emitting `Term::Hole` for the missing proof. This is STRICTLY WORSE than an
           axiom. `types_equal` returns `true` whenever either side is a `Hole`
           (type_checker.rs:1594-1597) and that leniency leaks into `is_subtype`'s fast path
           (:1314), so a `Hole` does not merely assert one proposition — it makes the type
           checker stop discriminating at every position it flows into. See T00-016 and T11-004.
@guard     Catches TRANSITIVE contamination. `taintedUser` below never mentions `sorry`; its
           proof term mentions `taintedLemma`, whose proof term mentions `sorryAx`. The
           translator must propagate the taint through the dependency graph of ELABORATED
           terms, exactly as Lean's `#print axioms` does, and reject every declaration in the
           closure — not just the one with the keyword in its source.
@guard     Catches the "translate it as an `add_declaration` and record a loss" compromise.
           There is no acceptable loss record for this: a loss record documents information that
           could not be carried, whereas this documents a proposition that was never proved.
           `reject` is the only correct verdict, which is why INDEX.md marks it `R` and why this
           case has no `@loss` field.
@oracle    Translator exits non-zero with E_INCOMPLETE_PROOF for EVERY declaration in this file
           — including `taintedUser`, which contains no `sorry` — and performs zero
           `add_declaration`, `add_definition` and `add_instance` calls. A harness additionally
           asserts that no emitted term anywhere in the corpus contains `Term::Hole`.
@note      This file deliberately emits Lean's `declaration uses 'sorry'` WARNING — on exactly
           THREE declarations: `taintedLemma`, `tacticSorry` and `dataSorry`. `lean` still exits
           0, so a harness keyed on the exit code sees a clean build. That is the first half of
           the lesson.
           The second half is that `taintedUser` gets NO warning at all: its proof term mentions
           the constant `taintedLemma`, not `sorryAx`, so the warning does not propagate. A
           translator that treats the warning as its taint signal will therefore translate
           `taintedUser` and emit a "verified" LOGOS theorem resting on an unproved lemma. The
           taint must be computed over the transitive closure of the elaborated environment —
           what `#print axioms` reports — not over warnings and not over source text.
-/

namespace T06_017

/-- The bare admission. Lean records this as `sorryAx`. -/
theorem taintedLemma : ∀ n : Nat, n + 0 = n := sorry

/-- Tactic-mode spelling. Same `sorryAx`, different surface syntax, so a keyword scan of the
    tactic block and a keyword scan of the term both have to fire. -/
theorem tacticSorry (p q : Prop) (hp : p) : q := by sorry

/-- `sorry` at a DATA type, not a proposition. `sorryAx` is `{α : Sort u} → α`, so it fabricates
    values as well as proofs; a translator that only guards propositional positions misses this. -/
def dataSorry : Nat := sorry

/-- TRANSITIVE CONTAMINATION. This declaration is clean; its proof term is not. -/
theorem taintedUser : (3 : Nat) + 0 = 3 := taintedLemma 3

/-- A clean declaration in the same file, to show the taint is per-declaration and that a
    correct translator rejects the four above while still being ABLE to translate this one.
    (Whether it does so, or refuses the whole file, is a policy choice the diagnostic must
    state.) -/
theorem clean : (3 : Nat) = 3 := rfl

end T06_017
