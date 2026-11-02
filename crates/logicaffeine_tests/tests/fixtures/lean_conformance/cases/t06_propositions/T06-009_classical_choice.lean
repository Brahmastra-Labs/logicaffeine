/-!
@case      T06-009
@tier      6 · Propositions, equality, classical reasoning
@title     `Classical.choice` — ABSENT from the LOGOS trusted base; emitting it as an axiom
           silently enlarges the TCB
@lean      Classical.choice, Classical.indefiniteDescription, Classical.choose,
           Classical.choose_spec, Classical.propDecidable, Nonempty.elim into Type
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/prelude.rs:33-44 (`StandardLibrary::register` — the COMPLETE list of what
           the trusted base contains); kernel/src/prelude.rs:46-78 (`register_classical` is the
           whole classical fragment, and it registers EXACTLY ONE declaration: `dne`. Its own
           doc comment names `Classical.em`/`choice` as the Lean/Coq analogue and then adds only
           `dne`); a grep of `crates/logicaffeine_kernel/src` for `choice`, `Nonempty` or
           `indefiniteDescription` returns NOTHING outside doc comments; kernel/src/prelude.rs:
           1910-1983 (`Ex`/`witness` — the existential is a two-constructor-free `Prop` whose
           derived eliminator has a `Sort(Prop)` motive, see recursor.rs:64-71 and
           type_checker.rs:1688-1716, so there is no way to extract a `Type`-level witness);
           kernel/src/certificate.rs:28 (`PRELUDE_VERSION = "logos-coc-1"`);
           kernel/src/certificate.rs:63-75 (`recheck` refuses a mismatched
           `prelude_version` and then rebuilds the trusted context ITSELF from
           `StandardLibrary::register`, "never trust a context the certificate might carry")
@diag      E_AXIOM_NOT_IN_TRUSTED_BASE — must name `Classical.choice`, state that the LOGOS
           trusted base contains exactly one classical axiom (`dne`, prelude.rs:78), list the
           declaration that depends on it, and state that adding it would change the axiom set
           the certificate's `PRELUDE_VERSION` describes. Must NOT offer to emit it.
@guard     THIS IS THE HIGHEST-VALUE REJECTION IN TIER 6. The tempting wrong move is a single
           line: `ctx.add_declaration("Classical_choice", <translated type>)`. It compiles. The
           proof terms that use it typecheck in the translator's own `Context`. Nothing warns.
           And the trusted base has just grown by an axiom that is strictly stronger than
           everything LOGOS deliberately chose to trust — `choice` gives `propDecidable`, hence
           full `Decidable` for every `Prop`, hence `decide`-based "computation" on undecidable
           propositions; combined with the `Quot` primitives already in the prelude
           (prelude.rs:1230-1338) it gives Diaconescu's theorem, i.e. excluded middle *again*
           but now with data. The `dne` axiom alone gives none of that.
@guard     The failure is SILENT IN THE PLACE THAT MATTERS. `certificate::recheck` rebuilds the
           context from `StandardLibrary::register` alone (certificate.rs:73-75), so a
           certificate whose proof term mentions the invented axiom fails with a bare
           `KernelError::UnboundVariable("Classical_choice")` — a message that looks like a name
           resolution bug, not like "you added an axiom". And if the translator "fixes" that by
           ALSO patching the prelude, then `PRELUDE_VERSION` still reads `"logos-coc-1"`
           (certificate.rs:28), the version guard at certificate.rs:65 passes, and every
           certificate ever minted now claims to have been checked against an axiom set that is
           not the one it was checked against. The de Bruijn criterion is broken and the
           artefact that was supposed to record the breakage says everything is fine.
@guard     Catches the "it's just `Nonempty` elimination" rationalisation. `Nonempty α` is a
           `Prop` and `α` is data; eliminating the former into the latter is EXACTLY the
           content of choice. LOGOS has no `Nonempty` at all, and its `Prop`-elimination gate
           (`is_subsingleton_prop`, type_checker.rs:1688-1716) permits large elimination only
           for zero-constructor or single-propositional-field inductives — a translator that
           declares `Nonempty` as such an inductive and then large-eliminates it has written
           choice by hand.
@guard     Catches the scale error. This is not one missing lemma. `Classical.choice` is one of
           Lean 4's three core axioms and essentially the whole of Mathlib depends on it
           transitively; `#print axioms` on almost any Mathlib theorem lists it. A translator
           that rejects it correctly will reject most real Lean input — which is the honest
           answer, and is what this corpus exists to establish.
@oracle    Translator exits non-zero with E_AXIOM_NOT_IN_TRUSTED_BASE for every declaration
           below, writes NO declaration into the LOGOS context, and in particular performs zero
           `add_declaration` calls. A harness additionally asserts that the set of
           `add_declaration` names produced over the whole corpus is a subset of the names
           registered by `StandardLibrary::register`.
-/

namespace T06_009

/-- The axiom itself. Lean's `Classical.choice : Nonempty α → α` turns a proposition into
    data; nothing in LOGOS can do that. -/
noncomputable def pick {α : Sort u} (h : Nonempty α) : α := Classical.choice h

/-- `indefiniteDescription` — choice in its most recognisable form. -/
noncomputable def describe {α : Sort u} (p : α → Prop) (h : ∃ x, p x) :
    {x // p x} := Classical.indefiniteDescription p h

/-- `Classical.choose` / `choose_spec`: extracting a `Type`-level witness from a `Prop`-level
    existential. LOGOS's `Ex` eliminator has a `Sort(Prop)` motive and cannot do this. -/
noncomputable def witnessOf (p : Nat → Prop) (h : ∃ n, p n) : Nat := Classical.choose h

theorem witnessSpec (p : Nat → Prop) (h : ∃ n, p n) : p (Classical.choose h) :=
  Classical.choose_spec h

/-- `Classical.propDecidable` — the step from `em` to DATA. This is the consequence that makes
    choice strictly stronger than `dne`, and it is why T06-008 must not be allowed to reach
    here by "just one more derivation". -/
noncomputable def decideAnything (p : Prop) : Decidable p := Classical.propDecidable p

/-- A `Decidable`-instance-driven computation over an arbitrary proposition. Under LOGOS's
    prelude only `decEqBool` and `decEqNat` exist (prelude.rs:926, :1027); this term claims
    to decide an arbitrary `Prop`. -/
noncomputable def chooseBranch (p : Prop) (a b : Nat) : Nat :=
  @ite Nat p (Classical.propDecidable p) a b

/-- An innocuous-looking lemma whose elaborated proof term mentions `Classical.choice`.
    This is the realistic case: the Lean source never types the word "choice". -/
theorem innocuous (p : Nat → Prop) (h : ∃ n, p n) : ∃ n, p n :=
  ⟨Classical.choose h, Classical.choose_spec h⟩

end T06_009
