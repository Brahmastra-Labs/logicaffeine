/-!
@case      T11-015
@tier      11 · Adversarial and soundness traps
@title     Trusted-base drift — `PRELUDE_VERSION` is a single hard-coded string, and nothing
           makes it change when the prelude does
@lean      any theorem whose translation adds a declaration, `#print axioms`, an `axiom`
           declaration, a dependency on a non-prelude primitive
@expect    degrade
@target    The faithful term, PLUS a machine-readable trusted-base record: the
           `PRELUDE_VERSION` string in force, the complete list of names the translator added
           with `add_declaration`, and the list of prelude names the translation depends on.
@evidence  kernel/src/certificate.rs:28 (`pub const PRELUDE_VERSION: &str = "logos-coc-1";` —
           a hand-maintained literal, not a hash of the prelude);
           kernel/src/certificate.rs:50 (`prelude_version: PRELUDE_VERSION.to_string()` — the
           value is stamped into every certificate at mint time);
           kernel/src/certificate.rs:63-75 (`recheck` first refuses a certificate whose
           `prelude_version` differs at :65-69, and then REBUILDS the trusted context itself
           from `StandardLibrary::register` — the repo's own comment for this is "never trust a
           context the certificate might carry"); kernel/src/prelude.rs:18-51
           (`StandardLibrary::register` — the single function that defines what the string is
           supposed to describe); kernel/src/context.rs:434-438 (`add_declaration` — the way a
           translator enlarges the axiom set, with no record kept anywhere);
           kernel/src/context.rs:449-464 (`get_global` — a translator-added declaration is
           looked up exactly like a prelude one; nothing distinguishes them)
@loss      TRUSTED_BASE_IS_NOT_PINNED. `PRELUDE_VERSION` is a constant a human must remember to
           bump. It is not derived from `StandardLibrary::register`, so a change to the prelude
           that leaves the string alone produces certificates that claim to have been checked
           against an axiom set that is not the one they were checked against — and the version
           guard at certificate.rs:65 passes, because the string still matches. The de Bruijn
           criterion depends on knowing exactly what was trusted; a hand-maintained string is
           the weakest possible way to record that.
@loss      TRANSLATOR_ADDITIONS_ARE_UNRECORDED. Every `add_declaration` the translator makes
           enlarges the trusted base, and LOGOS keeps no list: `declarations` is just the
           fourth map `get_global` consults (context.rs:463), with no attribute system and no
           provenance. So the translator must keep the list itself, and must ship it with the
           output, or the resulting library's trusted base is simply unknown.
@guard     **TIER 11 THESIS, AT THE ARTEFACT LEVEL.** The kernel accepting a term is not
           evidence of faithfulness — and here the artefact that was supposed to record what
           the kernel accepted is itself unreliable. The failure mode is quiet and permanent:
           a certificate minted against a modified prelude carries `"logos-coc-1"`, passes
           `recheck`'s version guard (certificate.rs:65-69), and is then re-checked against the
           CURRENT `StandardLibrary::register` (certificate.rs:73-75) — which may be a
           different axiom set again. A certificate is only as good as the pinning of the base
           it names, and this one is pinned by a string literal.
@guard     Catches the translator patching the prelude to make something typecheck. This is the
           exact composition with T06-009 and T11-014: add `Classical_choice` or `sorryAx` to
           `StandardLibrary::register` so the emitted terms check, and every certificate
           thereafter lies. The rule is absolute: the translator may not modify
           `StandardLibrary::register`, and must fail rather than need to.
@guard     Catches the OPPOSITE failure — refusing to record a legitimate addition. Some Lean
           `axiom` declarations are genuinely intended as assumptions and translate correctly to
           `add_declaration` (see T11-014's contrast case). Those additions are sound and must
           still be RECORDED, because the resulting LOGOS library's trusted base is then
           `StandardLibrary::register` ∪ {those names}, which is not what `PRELUDE_VERSION`
           describes.
@guard     Catches assuming `recheck` will surface the difference. It will surface a MISSING
           name — a certificate mentioning `Classical_choice` fails with
           `KernelError::UnboundVariable("Classical_choice")` after the rebuild at
           certificate.rs:73-75 — but that error reads like a name-resolution bug, not like
           "you added an axiom", and it does not fire at all if the extra declaration was also
           added to the rebuilt context. The diagnostic quality is the problem, not just the
           presence of a check.
@guard     Catches recording the version once per run instead of once per declaration. A
           translation is typically incremental: declarations accumulate across invocations,
           and a prelude change between two runs leaves half the output checked against one
           base and half against another, with one `PRELUDE_VERSION` string covering both.
@oracle    Every emitted declaration carries a trusted-base record naming (a) the
           `PRELUDE_VERSION` in force, (b) the translator's own `add_declaration` additions,
           and (c) a CONTENT HASH of `StandardLibrary::register`'s output computed by the
           translator itself — not the version string, which is what this case exists to
           distrust. A harness asserts that the recorded hash is identical across the whole
           corpus, and that (b) is a subset of the axioms the corpus declares legitimate.
-/

namespace T11_015

/-- A theorem with no axiom dependencies at all — the baseline. Its trusted-base record should
    list the prelude names it uses and no additions. -/
theorem pureComputation : (2 : Nat) + 2 = 4 := rfl

-- Lean's own audit trail; LOGOS has no counterpart, so the translator must synthesise one.
-- Prints: 'T11_015.pureComputation' does not depend on any axioms
#print axioms pureComputation

/-- A theorem that depends on one of Lean's three core axioms. Its LOGOS translation must
    record that dependency — and, since `propext` is ABSENT from the LOGOS trusted base
    (T06-010), must in fact be rejected rather than recorded. -/
theorem usesPropext (p q : Prop) (h : p ↔ q) : p = q := propext h

#print axioms usesPropext

/-- A deliberate assumption. Translating this to `add_declaration` is sound and still enlarges
    the base relative to `StandardLibrary::register`. -/
axiom heightBound : ∀ n : Nat, n ≤ 1000

theorem usesAssumption : (5 : Nat) ≤ 1000 := heightBound 5

#print axioms usesAssumption

/-- A theorem whose axiom dependency is INVISIBLE in its own source — it arrives through a
    lemma. This is the realistic shape and is why the record must be transitive. -/
theorem indirect (p q : Prop) (h : p ↔ q) : (p = q) ∧ True :=
  ⟨usesPropext p q h, trivial⟩

#print axioms indirect

/-- A `Classical` dependency, which Lean introduces silently through `by_cases`, `Decidable`
    instances on arbitrary `Prop`s, or `em`. LOGOS's trusted base contains exactly one
    classical axiom, `dne` (prelude.rs:78), and NOT `Classical.choice` — see T06-009.
    Note what Lean reports below: `Classical.em` pulls in THREE axioms, `propext`,
    `Classical.choice` and `Quot.sound`. Of those the LOGOS trusted base has only the
    quotient primitives (prelude.rs:1230-1338); `propext` and `choice` are both absent. A
    trusted-base record that says "uses classical reasoning" is not good enough — it must
    name all three. -/
theorem usesClassical (p : Prop) : p ∨ ¬p := Classical.em p

#print axioms usesClassical

/-- `native_decide`, which adds `Lean.ofReduceBool` and `Lean.trustCompiler` to the axiom set.
    LOGOS has `reduceBool`/`ofReduceBool` (prelude.rs:1195-1219) but the TCB expansion is a
    different one — see T11-017. -/
theorem usesNative : (100 : Nat) * 100 = 10000 := by native_decide

#print axioms usesNative

end T11_015
