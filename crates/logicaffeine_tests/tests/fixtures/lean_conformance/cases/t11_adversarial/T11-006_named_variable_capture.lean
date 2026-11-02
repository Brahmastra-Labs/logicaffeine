/-!
@case      T11-006
@tier      11 · Adversarial and soundness traps
@title     Variable capture under the NAMED representation — Lean's de Bruijn binders carry no
           identity, LOGOS's carry nothing else
@lean      nested binders sharing a name, `Expr.bvar` / `Expr.fvar`, shadowing, hygienic
           macro-generated names, `variable` + `section`
@expect    reject
@target    (none for a term the translator has not freshened — the obligation is to emit a
           globally freshened term, and to refuse rather than emit a term it cannot prove
           capture-free)
@evidence  kernel/src/term.rs:443-545 (`Term::Var(String)` — NAMED, not de Bruijn; there is no
           `bvar`/`fvar` distinction and no binder identity beyond the string);
           kernel/src/type_checker.rs:1088-1140 (`substitute_avoiding` — capture avoidance
           exists, and its doc comment says exactly when it fires: "When a binder (`Pi`/
           `Lambda`/`Fix`) would capture a free variable of `replacement`, the binder is
           alpha-renamed"); kernel/src/type_checker.rs:1113-1121 (the SHADOWING arm: "The
           parameter shadows `var`: do not substitute in the body" — so an inner binder with
           the SAME NAME silently stops the substitution, which is correct for a
           well-scoped term and catastrophic for a mis-scoped one);
           kernel/src/type_checker.rs:890-899 (`substitute`'s fast path: `if !occurs_free(body,
           var) { return body.clone(); }` — the capture-avoidance machinery is not even reached
           when the translator's name choice makes the occurrence check answer wrongly);
           kernel/src/type_checker.rs:1061-1071 (`freshen` avoids `replacement_fvs` ∪
           `all_var_names(body)` ∪ `{var}` — it is sound for the substitution it is performing,
           and knows nothing about names the TRANSLATOR intends to introduce later);
           kernel/src/recheck.rs (the second kernel deliberately uses de Bruijn, which is the
           repo's own acknowledgement that the named representation is the fragile one)
@diag      E_CAPTURE_RISK — must name the declaration, the binder, and the colliding name, and
           emit nothing. A translator that freshens correctly need never raise this; a
           translator that cannot prove its output capture-free must raise it rather than
           emit.
@guard     **TIER 11 THESIS.** LOGOS's kernel accepting a term is not evidence of faithfulness.
           A captured variable produces a term that is PERFECTLY WELL TYPED — it just denotes a
           different function. `fun x => fun x => x` and `fun x => fun y => x` are both
           well-typed; they are different functions; and in Lean they are `fun _ => fun _ =>
           bvar 0` and `fun _ => fun _ => bvar 1`, which a translator that drops the index and
           keeps the hint name collapses into one. The kernel cannot possibly object: both
           results are terms it was designed to accept.
@guard     Catches the naive `Expr.bvar i ↦ Term::Var(binderName i)` lowering. Lean's binder
           names are HINTS with no uniqueness guarantee: `∀ (n : Nat) (n : Fin n), …` is legal
           Lean, `variable`-inserted binders routinely repeat, and macro hygiene means two
           binders can share a display name while being distinct. The de Bruijn INDEX is the
           identity; the name is decoration. Lowering must allocate a fresh name per binder and
           record the mapping, not reuse the hint.
@guard     Catches capture introduced by the TRANSLATOR's own generated names. Emitting support
           code (a `List` for T10-012, a projection for T03-011, a recursor motive) introduces
           binders the source never had. `freshen` (type_checker.rs:1061-1071) protects the
           substitution it is currently performing against names present in the term at that
           moment; it cannot protect against a name the translator will emit into a different
           subterm later. The freshening obligation is GLOBAL to the emitted declaration, not
           local to a substitution.
@guard     Catches the SHADOWING arm being mistaken for capture avoidance. When the binder name
           equals the substituted variable, `substitute_avoiding` deliberately stops descending
           (type_checker.rs:1113-1121). That is correct — the inner binder shadows — but it
           means a translator bug that gives an inner binder the same name as an outer one does
           not produce a capture error; it produces a substitution that silently does nothing,
           so the outer variable's value never reaches the body that was supposed to use it.
@guard     Catches trusting the fast path. `substitute` returns `body.clone()` unchanged when
           `occurs_free(body, var)` is false (type_checker.rs:897-899). If the translator has
           already collapsed two distinct binders onto one name, `occurs_free` answers about
           the wrong binder, the fast path fires, and the capture-avoidance code at :1088 is
           never reached at all.
@oracle    The decisive check is structural and belongs to the translator: in the emitted term
           for every case in the corpus, no `Pi`/`Lambda`/`Fix`/`Let` binder name may shadow an
           enclosing binder of the same name, and every `Term::Var` must resolve to the nearest
           enclosing binder that the Lean de Bruijn index designated. A harness verifies this by
           round-tripping the emitted named term back to de Bruijn and comparing against Lean's
           original `Expr` up to alpha — not by asking LOGOS `infer_type`, which accepts both
           the captured and the uncaptured term.
@note      This file compiles with `unusedVariables` warnings on the shadowed binders. Those
           warnings ARE the case's subject: Lean is reporting that an outer binder is
           unreachable because an inner binder of the same name shadows it. Renaming the
           binders to silence them would delete the phenomenon under test, so they are left in
           deliberately and no `set_option` is used to suppress them.
-/

namespace T11_006

/-- Two nested binders with the SAME hint name. Legal Lean; the inner shadows the outer, and
    `bvar 0` in the body means the inner one. A named lowering that reuses the hint produces
    the same term for this and for `fun x => fun y => y` — which is fine — and ALSO for
    `fun x => fun y => x`, which is not. -/
def innerWins : Nat → Nat → Nat := fun x => fun x => x

example : innerWins 1 2 = 2 := rfl

/-- The term that must NOT collapse onto the one above. -/
def outerWins : Nat → Nat → Nat := fun x => fun _ => x

example : outerWins 1 2 = 1 := rfl

/-- The two are observably different functions. -/
example : innerWins 1 2 ≠ outerWins 1 2 := by decide

/-- Shadowing in a DEPENDENT binder, where the inner binder's TYPE mentions the outer binder of
    the same name. The name `n` denotes the outer binder in the type and the inner binder in
    the body — one string, two referents, in one telescope. -/
def dependentShadow : Type := ∀ (n : Nat), ∀ (n : Fin n), Nat

/-- Three levels, so a freshening scheme that only compares against the immediately enclosing
    binder is not enough. -/
def tripleShadow : Nat → Nat → Nat → Nat := fun x => fun x => fun x => x

example : tripleShadow 1 2 3 = 3 := rfl

/-- Capture through SUBSTITUTION rather than through nesting: `subst` replaces `y` by a term
    whose free variable is named `x`, and the binder it must pass under is also named `x`.
    This is the case `substitute_avoiding`'s alpha-renaming branch (type_checker.rs:1123-1131)
    exists for — and it fires only if the translator did not already conflate the two names. -/
def captureViaSubst (x : Nat) : Nat → Nat := fun y => (fun x => x + y) 0

example : captureViaSubst 5 3 = 3 := rfl

/-- A `let` whose bound name shadows a binder. LOGOS's `Let` is zeta-TRANSPARENT: `infer_type`
    substitutes the value into the body before typing it (type_checker.rs:135-143), so a
    shadowing bug here changes the TYPE, not just the value. -/
def letShadow (x : Nat) : Nat :=
  let x := x + 1
  x + 1

example : letShadow 1 = 3 := rfl

/-- Macro-generated binders. Lean's hygiene makes these distinct even when they display
    identically; after elaboration the `Expr` carries de Bruijn indices and inaccessible
    names, and a translator that renders those names as strings can produce collisions that
    were impossible in the source. -/
macro "twice" x:term : term => `(fun a => $x (fun a => a))

def hygienic : (Nat → Nat) → Nat → Nat := twice (fun f => f)

example : hygienic id 7 = 7 := rfl

/-- A `fun` whose binder name matches a GLOBAL. LOGOS's `Term::Var` and `Term::Global` are
    different constructors, so this is safe in the kernel — but a translator that emits a
    `Global` where a `Var` was meant (or vice versa) produces a well-typed term denoting a
    different thing, and `substitute_avoiding` explicitly never substitutes a `Global`
    (type_checker.rs:1110-1111). -/
def shadowsAGlobal : Nat → Nat := fun innerWins => innerWins + 1

example : shadowsAGlobal 1 = 2 := rfl

end T11_006
