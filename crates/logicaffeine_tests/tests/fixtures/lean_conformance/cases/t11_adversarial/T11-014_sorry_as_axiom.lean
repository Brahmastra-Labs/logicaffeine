/-!
@case      T11-014
@tier      11 · Adversarial and soundness traps
@title     `sorry` smuggled in as an `axiom` declaration — the one translation that is
           indistinguishable from a proof
@lean      sorry, sorryAx, `admit`, a `theorem` whose proof term mentions `sorryAx`,
           a declaration transitively depending on one
@expect    reject
@target    (none — and in particular NOT `add_declaration`)
@evidence  kernel/src/context.rs:434-438 (`add_declaration(name, ty)` — "Add a declaration
           (typed assumption/hypothesis)… Used for proof certification where hypotheses are
           assumed." It stores a TYPE with no body: this is the axiom form, and it is the
           natural target for `sorryAx : ∀ (α : Sort u), α`);
           kernel/src/context.rs:449-464 (`get_global` finds `declarations` LAST, so an
           axiom is looked up exactly like any other global and nothing marks it as an
           assumption); kernel/src/reduction.rs:305-314 (delta reduction reads
           `get_definition_body`, which returns `None` for a declaration — so an axiom is
           simply opaque, exactly like every legitimate prelude primitive);
           kernel/src/prelude.rs:78 (`dne` — the ONE classical axiom, added with
           `add_declaration`) and prelude.rs:352-430 (the arithmetic primitives, also
           `add_declaration`s) — so the trusted base's own axioms are indistinguishable, by
           API, from a smuggled `sorry`; kernel/src/certificate.rs:28 (`PRELUDE_VERSION =
           "logos-coc-1"`); kernel/src/certificate.rs:63-75 (`recheck` refuses a mismatched
           `prelude_version` and then rebuilds the trusted context ITSELF from
           `StandardLibrary::register`); LOGOS_FACTS: `VerifiedProof.verified == true` iff a
           kernel term was produced and typechecked (verify.rs:31-43)
@diag      E_INCOMPLETE_PROOF(sorry) — must name the declaration, name every declaration that
           transitively depends on it, and emit NOTHING for any of them. The diagnostic must
           be a hard failure, never a warning, and must not be suppressible.
@guard     **TIER 11 THESIS, AT ITS HIGHEST STAKES.** LOGOS's kernel accepting a term is not
           evidence that the translation is faithful. `sorry` is where that gap becomes a
           forged proof. The tempting move is one line:
           `ctx.add_declaration("sorryAx", Π(α : Sort u). α)`.
           It compiles. Every theorem whose Lean proof used `sorry` then typechecks in LOGOS,
           `VerifiedProof.verified` comes back `true`, and a certificate is minted. Lean at
           least paints the declaration with a yellow warning and lists `sorryAx` under
           `#print axioms`; LOGOS has no axiom-tracking mechanism at all — `get_global` treats
           `declarations` as just the fourth map to check (context.rs:463) — so the artefact
           that was supposed to record incompleteness says the theorem is proved.
@guard     Catches the axiom that is not called `sorry`. The realistic path is not a literal
           `sorry` in the source; it is a declaration whose ELABORATED proof term mentions
           `sorryAx` because some lemma three files away was left open. The translator must
           inspect the elaborated term, not the surface syntax, and must propagate: every
           declaration transitively depending on a `sorryAx`-bearing one is equally
           unemittable.
@guard     Catches "emit it but mark it". There is nowhere to put the mark. LOGOS's `Context`
           has no attributes, no visibility, no metadata of any kind (LOGOS_FACTS: flat
           `String` keys, four parallel maps), and `Term::Lit` has no annotation variant. A
           mark that lives only in the translator's own side-channel is lost the moment the
           context is serialised into a certificate, and `certificate::recheck` rebuilds only
           the trusted base (certificate.rs:63-75) — it will not re-derive which of the
           translator's declarations were placeholders.
@guard     Catches the `PRELUDE_VERSION` deception. If the translator instead patches the
           prelude to include its placeholder axiom, `PRELUDE_VERSION` still reads
           `"logos-coc-1"` (certificate.rs:28), the version guard at certificate.rs:65 passes,
           and every certificate ever minted now claims to have been checked against an axiom
           set that is not the one it was checked against. This is the same failure as T06-009
           and T11-015, and `sorry` is the version of it with no upside at all.
@guard     Catches the "it's just a hypothesis" rationalisation. `add_declaration`'s own doc
           comment says "Used for proof certification where hypotheses are assumed"
           (context.rs:434-437) — which is true, and is why the API is there, and is exactly
           why it must not be used for `sorry`: a hypothesis is discharged by the theorem's
           own statement, whereas `sorryAx` is an unrestricted `Π(α : Sort u). α`, i.e. a proof
           of everything. A translator that cannot distinguish "assume the premises of this
           theorem" from "assume anything" has no faithfulness oracle at all.
@guard     Catches the `Type`-valued `sorry`. `sorryAx` returns any `Sort`, so it can produce
           DATA as well as proofs — a `sorry`-provided `Nat`, or a `sorry`-provided instance —
           and such a term reduces to nothing (it is opaque, reduction.rs:305-314), so it
           silently blocks every computation downstream of it while typechecking perfectly.
@oracle    Translator exits non-zero with E_INCOMPLETE_PROOF(sorry) for every declaration below
           and its transitive dependents, and emits nothing for any of them. The decisive
           harness assertion is global and is the same one T06-009 uses: the set of names
           passed to `add_declaration` over the WHOLE corpus must be a subset of the names
           registered by `StandardLibrary::register`. A translator that never adds an axiom
           cannot smuggle a `sorry` as one.
-/

namespace T11_014

/-- The literal case. Lean warns; it does not error, so this file compiles, which is itself
    the point: a `sorry` is not a compile failure to detect, it is a declaration to inspect. -/
theorem obviouslyUnproved : ∀ n : Nat, n = n + 1 := by
  sorry

/-- Term-mode `sorry`, producing a proof. -/
theorem alsoUnproved : (2 : Nat) = 3 := sorry

/-- `sorry` producing DATA rather than a proof. `sorryAx : ∀ (α : Sort u), α` is not restricted
    to propositions, so this is a `Nat` that typechecks and does not reduce. -/
def sorryNat : Nat := sorry

/-- The realistic shape: the source never types the word. This declaration's own proof is
    complete; it depends on one that is not. The translator must propagate the rejection. -/
theorem dependsOnIt : ∀ n : Nat, n = n + 1 := fun n => obviouslyUnproved n

/-- Two levels deep. -/
theorem dependsTransitively : (0 : Nat) = 1 := dependsOnIt 0

/-- A `sorry` inside an instance, so the incompleteness is carried by typeclass resolution and
    shows up in terms that never mention the instance by name. -/
class Trivial' (α : Type) where
  witness : α

instance : Trivial' Nat := ⟨sorry⟩

def usesInstance [Trivial' Nat] : Nat := Trivial'.witness

-- Lean's own audit trail, which LOGOS has no counterpart for: this prints
-- `'T11_014.obviouslyUnproved' depends on axioms: [sorryAx]`. `sorryAx : ∀ (α : Sort u), α`
-- is the forbidden `add_declaration` target — a proof of everything.
#print axioms obviouslyUnproved

/-- A legitimately axiomatised declaration, for contrast. This one is a deliberate assumption
    with a specific type, and translating it to `add_declaration` is correct in principle —
    though it still enlarges the trusted base relative to `StandardLibrary::register` and so
    must be reported (T11-015). The difference from `sorryAx` is that this axiom is not a proof
    of everything. -/
axiom deliberateAssumption : ∀ n : Nat, 0 ≤ n

/-- And the same statement proved, to mark what a translated theorem should look like. -/
theorem actuallyProved : ∀ n : Nat, 0 ≤ n := Nat.zero_le

end T11_014
