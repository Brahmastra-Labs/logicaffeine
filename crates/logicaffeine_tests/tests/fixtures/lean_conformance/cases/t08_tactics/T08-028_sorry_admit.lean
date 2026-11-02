/-!
@case      T08-028
@tier      8 · Tactics
@title     `sorry` / `admit` — must never become an `add_declaration`, a `Term::Hole`, or a
           `verified == false` derivation that is reported as a proof
@lean      sorry, admit, sorryAx, a proof term containing sorry, a definition using sorry
@expect    reject
@target    (none — no sound translation)
@evidence  a grep of `crates/logicaffeine_kernel/src` and `crates/logicaffeine_proof/src` for
           `sorry` / `admit` returns NOTHING outside unrelated prose — there is no
           `sorryAx`, no admitted-proof marker, and no "incomplete" flag on a declaration
           anywhere in LOGOS;
           proof/src/tactic_script.rs:308-327 (`canonical` — no arm for `sorry` or `admit`;
           the script dies with "unknown tactic `sorry`" at :262);
           kernel/src/context.rs:434 (`add_declaration(name, ty)` — the AXIOM form: a type with
           no body, opaque, indistinguishable from a prelude axiom once registered);
           kernel/src/context.rs:449 (`get_global`'s lookup precedence — inductives,
           constructors, definitions, declarations: a declaration added this way is found and
           used like any other global);
           kernel/src/type_checker.rs:1594-1597 (`types_equal` treats `Term::Hole` as matching
           ANYTHING — so the other obvious encoding is worse than useless);
           proof/src/verify.rs:31-43 (`VerifiedProof.verified == true` IFF a kernel term was
           produced AND typechecked — a derivation may exist with `verified == false`);
           logicaffeine_language/src/compile.rs:1163 ("derivation found but NOT kernel-certified
           (this is not a proof)");
           kernel/src/certificate.rs:28 (`PRELUDE_VERSION = "logos-coc-1"`);
           kernel/src/certificate.rs:63-75 (`recheck` refuses a mismatched `prelude_version` and
           then REBUILDS the trusted context itself from `StandardLibrary::register` — "never
           trust a context the certificate might carry")
@diag      E_INCOMPLETE_PROOF — must name the declaration, state that its Lean proof is
           incomplete (`sorry`/`admit`/`sorryAx` in the elaborated term), and produce NO LOGOS
           output for it or for anything that depends on it. Must NOT offer an axiom, a hole,
           or an unverified derivation as a substitute.
@guard     THE AXIOM SMUGGLING TRAP, and the highest-value rejection in Tier 8.
           `ctx.add_declaration("sorryAx", <translated type>)` is one line. It compiles. Every
           term that uses it typechecks. `get_global` (context.rs:449) resolves it exactly like
           a prelude axiom, so nothing downstream can tell it apart from `dne` or `le_trans`.
           And because `Term::Sort(Prop)` is inhabited by fiat once such a declaration exists,
           the translator has just made LOGOS inconsistent — with no marker, no attribute
           system to tag it with (LOGOS has none, T02-013), and no `#print axioms` analogue to
           discover it. The certificate that results still reads `PRELUDE_VERSION =
           "logos-coc-1"` (certificate.rs:28).
@guard     Catches the "it fails at recheck anyway" rationalisation. `certificate::recheck`
           rebuilds the trusted context from `StandardLibrary::register` alone
           (certificate.rs:73-75), so a certificate mentioning the invented axiom fails with a
           bare `KernelError::UnboundVariable("sorryAx")` — a message that looks like a
           name-resolution bug, not like "this proof was never finished". And that is the GOOD
           case: a translator that "fixes" it by also patching the prelude gets a passing
           recheck against an axiom set the version string does not describe. See T11-014.
@guard     Catches the `Term::Hole` encoding. Leaving a hole where the proof should be is worse
           than the axiom: `types_equal` matches `Hole` against anything
           (type_checker.rs:1594-1597), so the gap does not even localise to one type. See
           T08-027, T00-016, T11-004.
@guard     Catches the `verified == false` encoding. Running the script, letting `qed()` produce
           a `VerifiedProof` with `verified == false`, and emitting it as the theorem's artefact
           looks honest — the flag is right there. It is not enough: `compile_theorem` reports
           such a result as "derivation found but NOT kernel-certified (this is not a proof)"
           (compile.rs:1163), and a translator that passes the artefact along leaves the "this
           is not a proof" determination to whoever reads the report. The verdict is `reject`:
           no output at all.
@guard     Catches TRANSITIVITY, and note how quiet Lean itself is about it. A `sorry` in one
           declaration poisons every declaration that uses it, but the compiler warns ONLY on
           the declaration whose own elaborated term contains `sorryAx`: `dependsOnSorry` below
           produces NO warning, and the poisoning is visible only through
           `#print axioms dependsOnSorry`, which reports `'dependsOnSorry' depends on axioms:
           [sorryAx]` (verified). A translator that keys its rejection on the compiler WARNING
           will therefore let the whole downstream cone through. It must key on the axiom
           closure of the elaborated `Expr` instead, and refuse the cone, not just the
           `sorry`-bearing declaration. LOGOS offers no second chance here: it has no
           dependency tracking of this kind at all.
@guard     Catches `sorry` in DATA, not just in proofs. `stubbedFunction` is a `Nat`-valued
           definition whose body is `sorry`. Translating it as `add_declaration` gives an opaque
           constant of type `Nat` — which is sound as an axiom but is NOT the function the
           source names, and any later `decide`/`simp` that tries to reduce it will silently get
           stuck instead of computing.
@note      THIS FILE WARNS, DELIBERATELY. `lean` exits 0 but prints "declaration uses 'sorry'"
           for SIX of the seven declarations below — that is the construct under test, and a
           corpus case about `sorry` that did not warn would not contain a `sorry`. No
           `set_option` is used to suppress it. The seventh, `dependsOnSorry`, is silent, which
           is the point of the transitivity guard: the warning is a necessary but NOT sufficient
           signal, and a translator must key its rejection on the axiom closure instead.
@oracle    Translator exits non-zero with E_INCOMPLETE_PROOF for every declaration below AND for
           `dependsOnSorry`, performs ZERO `add_declaration` calls, emits no `Term::Hole`, and
           emits no artefact with `verified == false`. A harness additionally asserts the
           corpus-wide invariant that the set of `add_declaration` names the translator produces
           is a subset of the names `StandardLibrary::register` already registers.
-/

namespace T08_028

/-- The bare tactic. -/
theorem bareSorry (p : Prop) : p := by sorry

/-- `admit` — the synonym. Same elaborated `sorryAx`. -/
theorem bareAdmit (p : Prop) : p := by admit

/-- `sorry` as a TERM rather than a tactic; the elaborated `Expr` is the same. -/
theorem termSorry (p : Prop) : p := sorry

/-- A PARTIAL proof: the first branch is real, the second is `sorry`. The declaration is still
    poisoned, and a translator that emits "the parts that worked" emits an unsound theorem. -/
theorem partialSorry (p q : Prop) (hp : p) : p ∧ q := by
  constructor
  · exact hp
  · sorry

/-- `sorry` in DATA: an opaque `Nat` with no computational content. -/
def stubbedFunction (n : Nat) : Nat := sorry

/-- TRANSITIVITY: this declaration contains no `sorry` of its own, and is poisoned anyway.
    Lean's `#print axioms dependsOnSorry` lists `sorryAx`. -/
theorem dependsOnSorry (p q : Prop) (hq : q) : p ∧ q := ⟨bareSorry p, hq⟩

/-- A `sorry` under a binder, so the hole's type mentions a local. -/
theorem sorryUnderBinder (p : Nat → Prop) : ∀ n, p n := by
  intro n
  sorry

end T08_028
