/-!
@case      T12-007
@tier      12 · Vernacular surfaces
@title     Surface B rejects every non-atomic term — `parse_term` accepts a bare identifier only
@lean      a first-order statement containing a FUNCTION SYMBOL in term position:
           `Add n Zero = n`, `Loves x (father x)`
@expect    reject
@target    (none — no sound rendering on Surface B)
@evidence  proof/src/formula.rs:429-435 — the entire term grammar:

               fn parse_term(&mut self) -> Result<ProofTerm, FormulaError> {
                   match self.advance() {
                       Some(Tok::Ident(s)) => Ok(ident_to_term(s)),
                       other => Err(FormulaError::new(format!("expected a term, found {other:?}"))),
                   }
               }

           One token. No application, no parentheses, no nesting, no literals beyond a
           digit-leading identifier, no arithmetic.
@evidence  proof/src/formula.rs:398-416 — a `Name(` opening is consumed as a PREDICATE
           (`ProofExpr::Predicate { name, args, world: None }`), and its arguments are
           parsed with `parse_term`. There is no context in which `Name(...)` is read as
           a term, so `Add(n, Zero)` is a *proposition*, not a number.
@evidence  proof/src/formula.rs:418-423 — the `=` arm is reached only after `parse_atom`
           has read a BARE identifier and found no `(`. Once `Add(n, Zero)` has been
           consumed as a predicate at :398-416, the `=` is never examined here.
@evidence  proof/src/formula.rs:57-68 — the leftover `=` then reaches the top level,
           where `if p.pos != p.tokens.len()` produces
           `FormulaError("unexpected trailing input near token Some(Eq)")`. So
           `Add(n, Zero) = n` is a *parse error*, and the error names a token, not the
           construct.
@evidence  proof/src/lib.rs:140-141 — `ProofTerm::Function(String, Vec<ProofTerm>)`
           EXISTS in the proof IR, and the engine reasons with it (proof/src/tactic.rs:584-586
           builds `Succ(K1)` during induction). It is only the Surface B *parser* that
           cannot write one. A translator that reads the IR and concludes the surface can
           express function terms has read the wrong layer.
@diag      E_SURFACE_B_TERM_NOT_ATOMIC — must name the offending Lean subterm and its
           head symbol, state that `formula.rs:429-435` accepts a single identifier, and
           redirect to Surface C (T12-008, which has s-expression application) or the
           raw kernel path (T12-011). Must emit no `Axiom`/`Theorem` declaration.
@guard     Catches the near-miss that PARSES. `Add(n, Zero)` alone is accepted — as the
           0-argument-free proposition "n and Zero stand in the Add relation". So a
           translator that renders Lean's `Nat.add n 0 = n` by relationalising the
           function, e.g.

               Axiom add_zero: for all n, Add(n, Zero, n).

           gets a clean parse and a provable development. That is a *different theory*:
           it has replaced a function by a ternary relation without the totality and
           single-valuedness axioms that make the two equivalent. Every downstream
           theorem is then proved in a theory the Lean source did not state. If a
           translator does this, it is no longer a translator — and the kernel will not
           catch it, because the relational theory is perfectly consistent.
@guard     Catches the same near-miss in its equality form. `Add(n, Zero) = n` errors,
           but `AddNZero = n` does not — a translator that flattens the application into
           a single identifier to get past the parser has introduced a fresh CONSTANT
           (uppercase-leading, formula.rs:439-450) unrelated to `n` and `Zero`. It
           parses, it is provable from an axiom stating it, and it means nothing.
@guard     Catches assuming the symbolic spellings help. `formula.rs` accepts `∀`, `∃`,
           `∧`, `∨`, `¬`, `→`, `↔` as synonyms (tests/formula_parser.rs:99-107 asserts
           the two spellings agree) — but they are all CONNECTIVES. There is no symbolic
           spelling of application, and `+`/`*` are not tokens in this lexer at all.
@guard     Catches nesting a predicate inside a term. `Cong(a, father(b), c, d)` fails at
           `parse_term` on the `(` — error `"expected a term, found Some(LParen)"` — but
           `Cong(a, b, c, d) and Father(b, e)` parses, and a translator that Skolemises
           this way must record that it has changed the signature.
@oracle    Translator exits non-zero with E_SURFACE_B_TERM_NOT_ATOMIC and writes no
           `## Theory`/`## Axiom` text. Negative oracle for a harness:
           `parse_formula("for all n, Add(n, Zero) = n")` must return `Err` whose message
           contains `"unexpected trailing input"`.
@note      The prohibition is on the SURFACE, not the engine. Anything a translator needs
           a function symbol for must go to Surface C or the kernel; there is no partial
           credit on Surface B, because the only available workaround (relationalisation)
           changes the theory rather than losing information, which makes it a `reject`
           and not a `degrade`.
-/

namespace T12_007

/-- Function symbols in term position — the shape Surface B cannot write. `father` is a
    unary function on an opaque sort, so `Loves x (father x)` has a nested term. -/
axiom Person : Type
axiom father : Person → Person
axiom Loves : Person → Person → Prop

theorem lovesOwnFather (h : ∀ x, Loves x (father x)) (a : Person) : Loves a (father a) :=
  h a

/-- Two-level nesting: `father (father a)`. -/
theorem lovesGrandfather
    (h : ∀ x, Loves x (father x))
    (step : ∀ x y, Loves x y → Loves x (father y))
    (a : Person) : Loves a (father (father a)) :=
  step a (father a) (h a)

/-- Arithmetic is the same obstruction wearing different clothes: `Add n 0` is a term
    built by applying a function symbol, which `parse_term` cannot read. -/
def Add (n m : Nat) : Nat := n + m

theorem addZeroRight (n : Nat) : Add n 0 = n := rfl

/-- The equality whose LHS is a nested application — `Add (Add n 0) 0 = n`. -/
theorem addZeroTwice (n : Nat) : Add (Add n 0) 0 = n := rfl

/-- The relationalised version a translator is tempted to emit instead. It is a
    DIFFERENT proposition: nothing here says `AddRel` is total or single-valued, so
    `AddRel n 0 n` does not entail `Add n 0 = n`. Included so a conformance harness can
    diff the emitted development against it and detect the substitution. -/
axiom AddRel : Nat → Nat → Nat → Prop

theorem addRelZeroRight (h : ∀ n, AddRel n 0 n) (n : Nat) : AddRel n 0 n := h n

end T12_007
