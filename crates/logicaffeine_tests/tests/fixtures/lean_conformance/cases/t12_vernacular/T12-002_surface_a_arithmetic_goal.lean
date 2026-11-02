/-!
@case      T12-002
@tier      12 · Vernacular surfaces
@title     Surface A cannot state an arithmetic goal — `∀ n : Nat, n + 0 = n` is not expressible
@lean      theorem whose statement contains a term-level operation on a bound variable
           (HAdd.hAdd / Nat.add) under a ∀ binder
@expect    reject
@target    (none — no sound rendering on Surface A)
@evidence  language/src/parser/mod.rs:6072 and :6136 — a `Given:`/`Prove:` line is parsed
           by `self.parse_sentence()`, the declarative ENGLISH sentence parser, which
           produces a `LogicExpr`. There is no term-expression entry point on this path.
@evidence  language/src/parser/mod.rs:8352-8363 — the English infix operator words
           `plus`/`minus` exist ONLY in the IMPERATIVE expression grammar: they build
           `Expr::BinaryOp { op: BinaryOpKind::Add, .. }` through
           `alloc_imperative_expr`, a different arena and a different AST from the
           `LogicExpr` a theorem premise is made of. `plus` is unreachable from
           `Prove:`.
@evidence  language/src/proof_convert.rs:840-860 (`term_to_proof_term`) — the ONLY route
           by which Surface A ever emits `ProofTerm::Function("Add", …)` is
           `canonical_arithmetic_fn` (proof_convert.rs:49-57) applied to a
           `Term::Function` the parser built in parser/verb.rs:1005-1032 — the
           comparative-with-differential construction ("… scored 5 points more than …").
           That construction produces `Score(x) = Add(Score(y), 5)`: arithmetic over a
           MEASURE FUNCTION OF AN ENTITY, keyed to the LIA oracle's `Add`/`Sub`
           vocabulary. It cannot be pointed at a quantified numeric variable.
@evidence  language/src/compile.rs:1152 — `Auto` reaches
           `logicaffeine_proof::verify::prove_certify_check`, whose search vocabulary is
           `ProofExpr` (proof/src/lib.rs:199-244): `Predicate`, `Identity`, `Atom`,
           connectives, `ForAll`/`Exists` over UNTYPED variables. `ForAll` carries a
           bare `variable: String` (lib.rs:235-238) — there is no place to put `: Nat`,
           so the binder cannot even be restricted to the naturals except by adding a
           guard predicate, which changes the proposition.
@diag      E_SURFACE_A_NO_TERM_ARITHMETIC — must name the offending Lean subterm
           (`n + 0`, head `HAdd.hAdd`/`Nat.add`), state that `Given:`/`Prove:` lines are
           parsed by the English sentence parser and carry no term grammar, and
           redirect: Surface C (T12-008) has infix `+ - *` and the word `equals`, and
           the raw kernel path (T12-011) has the real thing. Emitting NOTHING on
           Surface A is required.
@guard     THE trap of this tier. `∀ n : Nat, n + 0 = n` reads like English, so the
           tempting emission is

               Prove: For all n, n plus 0 equals n.

           `equals` IS a real logic-mode token (tests/phase22_equals.rs:4-15 asserts
           `TokenType::Equals` is produced for `"x equals 5"`), and `for all` IS
           ordinary English, so this line does not look obviously wrong. But `plus`
           never reaches the logic grammar (parser/mod.rs:8355 is imperative-only), so
           at best the sentence parser errors on an unexpected word, and at worst it
           reads `plus` as an unknown noun and builds an `Identity` between a variable
           and a *thing*. Neither is `n + 0 = n`.
@guard     Catches the second tempting wrong move: axiomatising the arithmetic away as
           a predicate, e.g.

               Given: Zero is an additive identity.
               Prove: Every number plus zero is itself.

           A translator that does this has silently replaced the Lean proposition with
           an English paraphrase whose LOGOS meaning is a relation between two
           *entities*. It will "prove" — `Auto` will happily discharge a paraphrase from
           a paraphrased premise — and the resulting certificate attests a proposition
           that is not the translation of the Lean type. Every `reject` in this corpus
           is dangerous; this one is dangerous because it SUCCEEDS.
@guard     Catches reaching for the comparative-differential path
           (parser/verb.rs:1005-1032) as an escape hatch. That path requires an English
           comparative over a measure of a named entity and always names the measure
           after a unit or the verb; there is no phrasing of it that makes the measured
           object a ∀-bound numeral, and its `Add` lands in the oracle's Int vocabulary,
           not LOGOS `Nat`.
@oracle    Translator exits non-zero with E_SURFACE_A_NO_TERM_ARITHMETIC, names the
           subterm, and writes no `## Theorem` block. If the translator instead routes
           this theorem to Surface C or the kernel path, that is a PASS provided it
           records the surface decision — the failure mode being caught is emitting
           Surface A text.
@note      INDEX.md records this case as "no term arithmetic exists" on Surface A. That
           is very nearly right but not literally right, and the difference matters to a
           translator author: parser/verb.rs:1005-1032 + proof_convert.rs:845-860 do
           construct `ProofTerm::Function("Add", [Measure(y), k])` for one specific
           English comparative construction. The accurate statement is: **Surface A has
           no term grammar a translator can address** — the only arithmetic it can build
           is a by-product of a fixed English idiom over entity measures, and it is
           unreachable for a ∀-bound numeric variable. The verdict `reject` is
           unaffected.
-/

namespace T12_002

/-- The canonical unexpressible statement. `n + 0` is `HAdd.hAdd n 0`, elaborating to
    `Nat.add n 0`; `rfl` closes it because `Nat.add` recurses on its second argument. -/
theorem addZeroRight : ∀ n : Nat, n + 0 = n :=
  fun _ => rfl

/-- The same obstruction with the operation in the goal's head rather than under the
    binder — still a term operation, still unexpressible. -/
theorem addZeroLeft (n : Nat) : 0 + n = n :=
  Nat.zero_add n

/-- Multiplication, to show the obstruction is not about `+`: it is about there being
    no term grammar at all. -/
theorem mulOneRight (n : Nat) : n * 1 = n :=
  Nat.mul_one n

/-- A statement that is arithmetic-free on its face but whose PROOF obligation is
    arithmetic: the `≤` is `Nat.le`, an inductive *relation*, so a naive translator may
    think this one is safe for Surface A. It is not — the successor term `n + 1` is
    still a term operation in the statement. -/
theorem leSucc (n : Nat) : n ≤ n + 1 :=
  Nat.le_succ n

end T12_002
