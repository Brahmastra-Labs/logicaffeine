/-!
@case      T12-006
@tier      12 · Vernacular surfaces
@title     Surface B (`## Theory`) — an axiomatic development over relations on bare identifiers
@lean      axiomatised relation on an opaque sort; theorems proved from the axioms by
           ∀-elimination and chaining; hypotheses passed explicitly
@expect    translate
@target    A `## Theory` block whose body is a sequence of `Axiom` / `Theorem`
           declarations parsed by `logicaffeine_proof::development::parse_development`
           and discharged in citation order by `prove_library_with_axioms`:

           ```logos
           ## Theory Tarski

           Axiom pseudo_reflexivity: for all a b, Cong(a, b, b, a).
           Axiom inner_transitivity: for all a b c d e f, if Cong(a, b, c, d) and Cong(a, b, e, f) then Cong(c, d, e, f).

           Theorem reflexivity: prove for all a b, Cong(a, b, a, b).
           Theorem symmetry cites reflexivity: prove for all a b c d, if Cong(a, b, c, d) then Cong(c, d, a, b).
           ```

           and, for a theorem with a premise:

           ```logos
           Axiom identity: for all a b c, if Cong(a, b, c, c) then a = b.
           Theorem null_segment: given Cong(P, Q, R, R); prove P = Q.
           ```
@evidence  tests(language)/axiom_theory_blocks.rs:107-133 — the `## Theory Tarski` block
           above is quoted verbatim from `theory_block_collects_a_multiline_development_body`,
           which asserts the collected body parses to 2 axioms and 2 theorems and that
           the second theorem's `cites` is `["reflexivity"]`;
           tests(proof)/development_parser.rs:49-76 — the same development with a third
           theorem, run end to end through `prove_development` and asserted
           `result.verified` for all three;
           tests(proof)/development_parser.rs:79-93 — the `null_segment` theorem with a
           `given` premise, verbatim;
           tests(language)/axiom_theory_blocks.rs:47-66 — a standalone
           `## Axiom flip: for all a b, Cong(a, b, b, a).` block round-tripping through
           `parse_formula` to the expected `ProofExpr`;
           proof/src/development.rs:9-23 (the grammar: `Axiom <name> : <formula>.` and
           `Theorem <name> [cites n1, n2] : [given <f>;]* prove <f>.`, statements
           terminated by `.`, clauses by `;`, citations by `,`);
           tests(proof)/formula_parser.rs:43-93 (the formula grammar: `for all a b`,
           `if … then …`, `and`, `there exists x`, `a = b`);
           tests(proof)/formula_parser.rs:99-107 (`symbolic_and_english_spellings_agree`:
           `∀ a b c d, … ∧ … → …` parses identically to the English spelling);
           proof/src/formula.rs:439-450 (`ident_to_term`: uppercase- or digit-leading
           ⟶ `ProofTerm::Constant`, lowercase-leading ⟶ `ProofTerm::Variable`)
@guard     Catches the case-convention bug, which is invisible and total. `Cong(a, b, b, a)`
           under `for all a b` is a universally quantified statement about VARIABLES.
           `Cong(A, B, B, A)` is a statement about four CONSTANTS — a completely
           different proposition that the quantifier prefix does not bind. Lean binder
           names are frequently uppercase (`P`, `Q`, `R`, `α`), so a translator that
           copies Lean binder names into Surface B formulas will emit constants where it
           meant variables, and `prove_library_with_axioms` will simply fail to prove
           the theorem — reported as a proof failure, never as a naming error. Bound
           variables must be lowercased on emission; free constants must be uppercased.
@guard     Catches the citation-order assumption. `citation_order` (proof/src/verify.rs:98-130)
           topologically sorts by `cites`, treating an UNKNOWN name as an external given
           rather than an error (verify.rs:110-113: `!known.contains(c) || placed_names.contains(c)`).
           A translator that misspells a cited lemma name gets no diagnostic at all —
           the citation is silently dropped and the theorem is attempted without it.
           Cited names must be emitted from the same mangling function that names the
           theorems.
@guard     Catches conflating the two block spellings. `## Axiom <name>: <formula>.`
           (one axiom, `Stmt::Axiom`) and `## Theory <name>` (a whole development body,
           `Stmt::Theory`) are different statements with different parse targets; the
           `Theory` body is NOT itself parsed by the language parser but collected as
           text and handed to `parse_development` (axiom_theory_blocks.rs:129). A
           translator that puts `Axiom`/`Theorem` declarations under a `## Axiom` header
           loses everything after the first formula.
@guard     Catches emitting a `Theorem` with no `prove` clause. `parse_theorem_decl`
           requires it; `given` clauses are optional and `;`-separated, and the goal is
           the clause introduced by `prove`.
@oracle    `parse_development(<emitted body>)` returns `Ok(dev)` with the expected axiom
           and theorem counts, and `prove_development(<emitted body>)` returns a result
           for every theorem with `verified == true`. A weaker oracle that only checks
           `parse_development` is not sufficient: the case-convention bug parses fine.
@note      Surface B is the right target for exactly the Lean theorems this case shows:
           relational, first-order, over an opaque sort, with no function symbols in the
           terms. Add one function symbol and the answer becomes `reject` (T12-007).
-/

namespace T12_006

/-- An opaque sort and a 4-ary relation on it — the exact shape Surface B accepts:
    `Point` never appears in a formula (Surface B is untyped FOL), only `Cong` does. -/
axiom Point : Type
axiom Cong : Point → Point → Point → Point → Prop

/-- The shared axiom base, as explicit hypotheses so each Lean theorem's signature is
    exactly its LOGOS `Axiom`/`Theorem` rendering.

    `Axiom pseudo_reflexivity: for all a b, Cong(a, b, b, a).`
    `Axiom inner_transitivity: for all a b c d e f,
        if Cong(a, b, c, d) and Cong(a, b, e, f) then Cong(c, d, e, f).` -/
abbrev PseudoReflexivity : Prop := ∀ a b, Cong a b b a

abbrev InnerTransitivity : Prop :=
  ∀ a b c d e f, Cong a b c d → Cong a b e f → Cong c d e f

/-- `Theorem reflexivity: prove for all a b, Cong(a, b, a, b).` -/
theorem reflexivity (pr : PseudoReflexivity) (it : InnerTransitivity) (a b : Point) :
    Cong a b a b :=
  it b a a b a b (pr b a) (pr b a)

/-- `Theorem symmetry cites reflexivity:
        prove for all a b c d, if Cong(a, b, c, d) then Cong(c, d, a, b).` -/
theorem symmetry (pr : PseudoReflexivity) (it : InnerTransitivity)
    (a b c d : Point) (h : Cong a b c d) : Cong c d a b :=
  it a b c d a b h (reflexivity pr it a b)

/-- `Theorem transitivity cites symmetry:
        prove for all a b c d e f,
        if Cong(a, b, c, d) and Cong(c, d, e, f) then Cong(a, b, e, f).` -/
theorem transitivity (pr : PseudoReflexivity) (it : InnerTransitivity)
    (a b c d e f : Point) (h1 : Cong a b c d) (h2 : Cong c d e f) : Cong a b e f :=
  it c d a b e f (symmetry pr it a b c d h1) h2

/-- `Axiom identity: for all a b c, if Cong(a, b, c, c) then a = b.`
    `Theorem null_segment: given Cong(P, Q, R, R); prove P = Q.`
    Note the CASE CONVENTION: `P`, `Q`, `R` are uppercase here because they really are
    constants in the LOGOS rendering — the theorem's `given` clause is not under a
    quantifier. In Lean they are ordinary bound variables, so the translator must decide
    the case from the LOGOS role, not from the Lean name. -/
theorem nullSegment
    (identity : ∀ a b c, Cong a b c c → a = b)
    (P Q R : Point) (h : Cong P Q R R) : P = Q :=
  identity P Q R h

/-- `there exists x` is in the formula grammar too:
    `Axiom segment_construction: for all b c d, there exists x, Cong(b, x, c, d).` -/
axiom segmentConstruction : ∀ b c d : Point, ∃ x, Cong b x c d

end T12_006
