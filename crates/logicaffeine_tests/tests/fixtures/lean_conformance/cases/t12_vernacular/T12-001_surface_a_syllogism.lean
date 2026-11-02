/-!
@case      T12-001
@tier      12 · Vernacular surfaces
@title     Surface A (`## Theorem` / `Given:` / `Prove:`) — a pure first-order syllogism
@lean      theorem over unary Prop-valued predicates on an opaque domain; ∀-elimination
           and modus ponens only (no arithmetic, no equality, no data)
@expect    translate
@target    An English theorem block, parsed by
           `logicaffeine_language::parser::parse_theorem_block` and proved by
           `logicaffeine_language::compile_theorem`:

           ```logos
           ## Theorem: Socrates_Mortality
           Given: All men are mortal.
           Given: Socrates is a man.
           Prove: Socrates is mortal.
           Proof: Auto.
           ```

           `Auto` routes to `logicaffeine_proof::verify::prove_certify_check`
           (compile.rs:1152), whose derivation must carry `UniversalInst` then
           `ModusPonens` and must come back with `verified == true`.
@evidence  language/src/parser/mod.rs:5977-6194 (`parse_theorem_block`: optional `Given`
           lines, REQUIRED `Prove` line, optional `Proof` line defaulting to `Auto`);
           language/src/compile.rs:1070-1176 (`compile_theorem`);
           language/src/compile.rs:1152 (`Auto` ⟶ `prove_certify_check`);
           tests/phase63_theorem_parser.rs:31-52 (the VERBATIM block above, asserted to
           compile and to produce `Proved`/`ModusPonens`/`PremiseMatch`);
           tests/phase63_theorem_parser.rs:136-154 (three-premise chain);
           tests/phase_studio_grid_proof.rs:51-64 (the same syllogism through
           `compile_theorem_for_ui`, asserted to keep its `UniversalInst` trace rather
           than being grounded — see T12-014)
@guard     Catches a translator that writes FOL into `Given:`/`Prove:`. Those lines go
           through the FULL ENGLISH SENTENCE PARSER — `self.parse_sentence()` at
           parser/mod.rs:6072 and :6136 — not through
           `logicaffeine_proof::formula::parse_formula`. `Given: forall x, Man(x) -> Mortal(x).`
           is not a parse error you will notice: the lexer will tokenize `forall`, `x`,
           `Man` … as ordinary English words and the sentence parser will either error
           with a syntax complaint that names an English category, or (worse) succeed
           and build a `LogicExpr` about a *thing called "forall"*. The formal-FOL
           spelling belongs to Surface B (T12-006), which this surface does not accept.
@guard     Catches emitting the Lean predicate names verbatim. The premise/goal
           predicates must MATCH after the parser's noun canonicalisation — `men` is
           canonicalised to `Man` (phase63_theorem_parser.rs:29 documents this as the
           reason the syllogism closes). A translator that renders `Man` as the plural
           noun in one line and the singular in another is relying on that pass, and a
           translator that invents a predicate spelling the English lexicon does not
           canonicalise the same way produces two distinct predicates and an
           unprovable goal — which surfaces as a *proof failure*, not a parse error.
@guard     Catches forgetting that `Prove:` is mandatory. A block with only `Given:`
           lines returns `ParseErrorKind::ExpectedKeyword { keyword: "Prove" }`
           (parser/mod.rs:6125-6128); there is no "state the goal last" fallback.
@guard     Catches emitting TWO theorem blocks in one document and assuming both are
           checked. `compile_theorem` takes the FIRST `Stmt::Theorem` it finds
           (compile.rs:1106-1119, `find_map`) and silently ignores every later one.
           One Lean theorem per LOGOS document. (T11-022 is the adversarial twin.)
@oracle    `compile_theorem(<emitted text>)` returns `Ok(s)` AND `s` contains
           `"[kernel-verified]"` — the string only `compile.rs:1155-1159` emits. The
           weaker `s.contains("Proved")` is NOT sufficient, because the unverified
           branch (compile.rs:1160-1164) also returns `Ok`. See T12-013.
@note      This is the ONLY shape of Lean theorem for which Surface A is the right
           target: an unquantified-over-data, predicate-only entailment. The moment the
           statement mentions a term operation the answer changes to `reject` (T12-002)
           or to the raw kernel path (T12-011).
-/

namespace T12_001

/-- The domain of discourse. Opaque: Surface A has no data types, so the Lean source
    must not depend on any structure of `Entity`. -/
axiom Entity : Type

axiom Man : Entity → Prop
axiom Mortal : Entity → Prop
axiom Doomed : Entity → Prop
axiom socrates : Entity

/-- The canonical syllogism. Premises are hypotheses, not axioms, so the Lean statement
    is exactly the LOGOS block: two `Given:` lines and one `Prove:` line. -/
theorem socratesMortality
    (hAll : ∀ x, Man x → Mortal x)
    (hSoc : Man socrates) :
    Mortal socrates :=
  hAll socrates hSoc

/-- Multi-step chain: the `Auto` backward chainer must instantiate twice. -/
theorem socratesDoom
    (hMortal : ∀ x, Man x → Mortal x)
    (hDoomed : ∀ x, Mortal x → Doomed x)
    (hSoc : Man socrates) :
    Doomed socrates :=
  hDoomed socrates (hMortal socrates hSoc)

/-- Degenerate case: the goal IS a premise. LOGOS closes this with `PremiseMatch`
    (phase63_theorem_parser.rs:160-185) rather than `ModusPonens`; a translator must not
    special-case it away. -/
theorem trivialRestatement (hSoc : Mortal socrates) : Mortal socrates :=
  hSoc

end T12_001
