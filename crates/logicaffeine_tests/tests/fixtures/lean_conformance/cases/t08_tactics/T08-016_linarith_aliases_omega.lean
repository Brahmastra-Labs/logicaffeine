/-!
@case      T08-016
@tier      8 · Tactics
@title     `linarith` is an ALIAS for `omega` — an ordered-field goal silently becomes
           integer-linear, and integer discreteness is unsound over ℚ
@lean      linarith, linarith over ℚ/ℝ, linarith with rational coefficients, nlinarith
@expect    degrade
@target    LOGOS script atom `omega` — reached through the alias `linarith`
           (proof/src/tactic_script.rs:318: `"omega" | "arithmetic" | "linarith" | "integers"
           => Some("omega")`) → `combinators::omega` (proof/src/tactic.rs:1008-1011) →
           `ProofState::omega` (proof/src/tactic.rs:852-878) →
           `omega_solve::omega_close` over `linarith_solve`'s Fourier-Motzkin core
@evidence  proof/src/tactic_script.rs:318 (the alias itself — `linarith` and `omega` are THE
           SAME canonical tactic; there is no separate rational solver reachable from a script);
           proof/src/tactic.rs:846-851 (the doc comment, which states the divergence outright:
           "linear INTEGER arithmetic. When the in-scope `≤`/`<` hypotheses are jointly
           unsatisfiable over ℤ — using DISCRETENESS (`a < b ⟹ a+1 ≤ b`), the fact rational
           solvers lack — refute them … Proves `x < y ∧ y < x+1 ⊢ Q`, WHICH `linarith` CANNOT
           (that system is rationally satisfiable)");
           proof/src/linarith_solve.rs:1-8, :13-20 (`LinExpr` — coefficients are `i64`, the
           constant is `i64`; there is no rational coefficient type anywhere in the solver);
           proof/src/lib.rs:199-238 (`ProofExpr` is UNTYPED: a `Predicate`'s arguments are
           `ProofTerm`s carrying no carrier information, so nothing in a translated goal records
           that the Lean source was about ℚ or ℝ);
           kernel/src/prelude.rs:202-340 (the eight order axioms, all stated for `Int`, where
           `a ≤ b ≔ Eq Bool (le a b) true` — there is no ordered-field structure in the trusted
           base at all, and no `Rat`/`Real` type: see LOGOS_FACTS §"Absent entirely")
@loss      FIELD_TO_INTEGER. A `linarith` call over ℚ, ℝ, or any `LinearOrderedField` becomes an
           integer-linear refutation. The coefficients are `i64` (linarith_solve.rs:13-20), so
           rational multipliers are unrepresentable, and the solver's use of discreteness makes
           it prove strictly MORE than `linarith` does. Acceptable ONLY when the translator has
           independently established that the carrier is `ℤ` or `ℕ`; otherwise it must
           escalate — see the second guard.
@loss      DIRECTION_OF_STRENGTH. Ordinary degrades lose power. This one GAINS it: LOGOS's
           `omega` refutes hypothesis sets that Lean's `linarith` correctly leaves alone. So the
           usual safety argument ("a weaker target just fails louder") does not apply, and a
           loss record is not optional.
@guard     THE SOUNDNESS TRAP, and the most dangerous alias in the table. `x < y` and
           `y < x + 1` are jointly UNSATISFIABLE over ℤ and perfectly SATISFIABLE over ℚ. LOGOS's
           `omega` refutes them (tactic.rs:846-851 says so explicitly) and then discharges ANY
           goal by `ExFalso` (tactic.rs:866-871). If the Lean source was about ℚ, the translated
           theorem is FALSE and LOGOS has certified it — and nothing in the LOGOS artefact
           records the carrier, because `ProofExpr` is untyped. The kernel cannot catch this: it
           is checking a correct ℤ proof of a statement that was never about ℤ. The translator
           is the only place this can be caught, and it must be caught before emission.
@guard     Catches the mitigation that looks sufficient. "LOGOS has no ℚ, so a ℚ goal would fail
           earlier" is false in exactly the case that matters: the ORDER PREDICATES translate
           fine (`le`/`lt` are prelude primitives, prelude.rs:352-515), the variables translate
           to bare `ProofTerm`s, and only the TYPE — which the FOL layer discards — was
           carrying the field-ness. A translator that erases types before choosing the tactic
           has already lost the information it needs to refuse.
@guard     Catches the alias reaching the script by accident. `arithmetic` and `integers` are
           also aliases (tactic_script.rs:318). A translator that renders a Lean `linarith` into
           prose ("by arithmetic") lands on `omega` too. There is no spelling of "linear
           arithmetic over an ordered field" anywhere in the grammar.
@guard     Catches `nlinarith` and `polyrith`. Neither has an alias; the words are unknown to
           `canonical` and the script dies at parse time. That is the correct outcome and must
           not be "fixed" by mapping them onto `omega` or `crush`.
@note      Bare Lean 4.14 has NO `linarith` (Mathlib). Every theorem below carries the Mathlib
           invocation in a comment and is proved with the core equivalent — `omega` for the
           integer-linear goals, explicit lemmas elsewhere. The ℚ/ℝ cases are written against a
           LOCALLY declared ordered-field-like class rather than `import Mathlib`, which is also
           the honest model of the situation: the field structure is a typeclass the LOGOS side
           has no counterpart for.
@note      A defensible reading makes the ℚ/ℝ cases `reject` rather than `degrade`, since no
           sound translation of them exists at all. The index records `degrade`, which this file
           keeps: the degrade is correct for the ℤ/ℕ fragment, and the ℚ/ℝ cases are the
           boundary the loss record must refuse to cross. A translator that emits `omega` for
           `ratField*` below without a carrier proof must be failed.
@oracle    Every emitted `omega` atom that originated from a Lean `linarith` carries a loss
           record naming FIELD_TO_INTEGER, and the translator has recorded a proof obligation
           that the goal's carrier is `Int` or `Nat`. For `ratFieldSatisfiable` the harness
           asserts the translator emitted NO tactic — a certified LOGOS proof of that goal is a
           corpus failure, not a pass.
-/

namespace T08_016

/-- Mathlib: `by linarith`. Integer-linear, and in the fragment where alias and target agree. -/
theorem intLinear (x y : Int) (h1 : x ≤ y) (h2 : y ≤ x) : x = y := by omega

/-- Mathlib: `by linarith`. Also fine: `Nat`, integer-linear. -/
theorem natLinear (a b c : Nat) (h1 : a ≤ b) (h2 : b ≤ c) : a ≤ c := by omega

/-- THE DIVERGENCE. Over ℤ these hypotheses are contradictory and `omega` discharges anything.
    Over ℚ they are satisfiable and `linarith` correctly refuses. Written over `Int` so it
    compiles; the danger is a translator applying this reasoning to a ℚ-typed source. -/
theorem discretenessRefutes (x y : Int) (q : Prop) (h1 : x < y) (h2 : y < x + 1) : q := by
  omega

/-- A locally-declared ordered field, so the carrier is abstract and NOT `Int`. -/
class OrderedFieldLike (F : Type) where
  le : F → F → Prop
  add : F → F → F
  one : F
  half : F → F
  le_trans : ∀ a b c : F, le a b → le b c → le a c
  half_between : ∀ a b : F, le a b → le a (half (add a b)) ∧ le (half (add a b)) b

/-- Mathlib: `by linarith`. The transitivity fragment — sound over any ordered structure, and
    the one `linarith` use that survives the alias intact. -/
theorem fieldTrans (F : Type) [OrderedFieldLike F] (a b c : F)
    (h1 : OrderedFieldLike.le a b) (h2 : OrderedFieldLike.le b c) :
    OrderedFieldLike.le a c :=
  OrderedFieldLike.le_trans a b c h1 h2

/-- THE CASE THAT MUST NOT BE TRANSLATED. Over a dense ordered field, `a < b` and
    `b < a + 1` are jointly SATISFIABLE — there is a midpoint. Lean's `linarith` cannot and
    must not derive anything from them; LOGOS's `omega` refutes them by discreteness. Stated
    here as the positive fact that a midpoint exists, which is what makes the refutation
    unsound over this carrier. -/
theorem ratFieldSatisfiable (F : Type) [OrderedFieldLike F] (a b : F)
    (h : OrderedFieldLike.le a b) :
    OrderedFieldLike.le a (OrderedFieldLike.half (OrderedFieldLike.add a b)) :=
  (OrderedFieldLike.half_between a b h).1

/-- Mathlib: `by nlinarith`. No alias at all — `canonical("nlinarith")` is `None`. -/
theorem nonlinear (x : Int) (h : 0 ≤ x) : 0 ≤ x * x := by
  exact Int.mul_nonneg h h

end T08_016
