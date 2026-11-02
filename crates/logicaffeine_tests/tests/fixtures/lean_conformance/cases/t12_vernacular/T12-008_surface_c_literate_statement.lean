/-!
@case      T12-008
@tier      12 · Vernacular surfaces
@title     Surface C (literate kernel) — `Statement:` with s-expression application and infix arithmetic
@lean      arithmetic (in)equalities over free numeric variables: `x ≤ x`, `x < x + 1`,
           `2 * x ≤ 2 * x + 1`, `add a b = add b a`
@expect    translate
@target    A literate theorem block executed by `logicaffeine_kernel::interface::Repl`,
           parsed by `literate_parser::parse_theorem`:

           ```logos
           ## Theorem: LeRefl
               Statement: (le x x).
               Proof: lia.
           ```
           ```logos
           ## Theorem: LtSuccInfix
               Statement: x < x + 1.
               Proof: lia.
           ```
           ```logos
           ## Theorem: LinearCoeffInfix
               Statement: 2 * x <= 2 * x + 1.
               Proof: lia.
           ```
           ```logos
           ## Theorem: Commutativity_Add
               Statement: (add x y) equals (add y x).
               Proof: ring.
           ```

           which registers `Command::Definition { name, ty: Some(Global("Derivation")),
           body: try_<tactic> ⌜statement⌝ }` (literate_parser.rs:564-585) and is applied
           by `Repl::execute` through `infer_type` + `Context::add_definition`
           (interface/repl.rs:36-72).
@evidence  tests/phase_literate_lia.rs:136-145 (`Statement: x <= x.`), :150-159
           (`Statement: 2 < 5.`), :165-174 (`Statement: x < x + 1.`), :210-219
           (`Statement: 2 * x <= 2 * x + 1.`), :225-234 (`Statement: x - 1 < x.`),
           :240-249 (`Statement: x ≤ x.`), :255-264 (`Statement: 5 ≥ 3.`) — all quoted
           verbatim, all asserted to execute;
           tests/phase_literate_ring.rs:34-43 and :49-58 (the s-expression `equals`
           forms, verbatim);
           tests/phase_literate_omega.rs:15,:22,:30,:73,:101 (the `(Lt 2 5)`,
           `(implies (Gt x 0) (Ge x 1))`, `(implies (Le (mul 3 x) 10) (Le x 3))` forms);
           tests/phase_literate_simp.rs:38,:46,:64 (`(Eq (add 2 3) 5)`,
           `(Eq (mul (add 1 1) 3) 6)`, `(Eq (add (mul 2 3) (sub 4 1)) 9)`);
           kernel/src/interface/literate_parser.rs:519-560 (`parse_literate_theorem`:
           `## Theorem:` header, name, `Statement` `:` `parse_term` `.`);
           kernel/src/interface/literate_parser.rs:1217-1252 (`parse_term`: the word
           `equals` ⟶ `Eq Hole lhs rhs`, the word `implies` ⟶ a non-dependent `Pi`);
           kernel/src/interface/literate_parser.rs:1261-1290 (`parse_comparison`:
           `<=`/`≤` ⟶ `le`, `>=`/`≥` ⟶ `ge`, `<` ⟶ `lt`, `>` ⟶ `gt`, NON-ASSOCIATIVE);
           kernel/src/interface/literate_parser.rs:625-690 (`parse_proof_tactic`:
           `ring`, `refl`, `lia`, `cc`, `simp`, `omega` each wrap the statement's
           reflected `Syntax` in `try_<tactic>`)
@guard     THE guard for this surface: `Repl::execute(…)` returning `Ok` IS NOT A PROOF.
           `try_ring`/`try_lia`/`try_cc`/`try_simp`/`try_omega`/`try_refl` are all
           declared `Syntax -> Derivation` via `Context::add_declaration`
           (kernel/src/prelude.rs:2984-2990 for `try_ring`, :3020-3026 for `try_lia`,
           :2685-2691 for `try_refl`). So the emitted definition body
           `try_lia ⌜goal⌝` is well-typed for ANY `Syntax` argument whatsoever, true or
           false, and `infer_type` at repl.rs:52 always succeeds. A failed tactic
           returns a `Derivation` whose `concludes` is `SName "Error"`
           (tests/phase96_tactics.rs:180-197 asserts exactly this), not a type error.
           The suite itself concedes the point: `phase_literate_lia.rs:105-108` and
           :121-124 assert `result.is_err() || result.is_ok()` — a tautology — for the
           FALSE statements `(lt 5 2)` and `(lt x x)`. A harness that reads `Ok` as
           "proved" will report `5 < 2` as a theorem.
@guard     Catches skipping the `Proof:` line. A literate theorem with no `Proof:` is
           registered as `Definition <name> : Prop := <statement>`
           (literate_parser.rs:582-585): the proposition becomes the BODY of a
           delta-transparent definition typed `Prop`. That is not a theorem at all, it
           is an abbreviation for the proposition — and `Repl::execute` returns `Ok` for
           it (tests/phase_literate_ring.rs:19-28 asserts precisely that
           `## Theorem: SimpleTheorem / Statement: True implies True.` parses with no
           proof). Emitting a Lean `theorem` this way records the statement and proves
           nothing.
@guard     Catches non-associativity of comparison. `parse_comparison`
           (literate_parser.rs:1261-1290) parses `additive (op additive)?` — at most ONE
           comparison. `a < b < c` does not parse as a chain and does not error into a
           useful message; a translator must decompose a Lean chain into a conjunction
           of two comparisons, which the `Statement:` grammar has no `and` for, so a
           chained comparison is out of scope for this surface.
@guard     Catches the `Hole`. `X equals Y` builds `Eq Hole X Y` with `Term::Hole` as the
           implicit type argument (literate_parser.rs:1227-1236). `types_equal` treats
           `Hole` as matching ANYTHING (type_checker.rs:1594-1597), so an `equals`
           statement carries a hole into the kernel by construction. That is inside
           LOGOS's own surface and unavoidable here — but a translator must never
           EMIT a `Hole` on the raw kernel path (T00-016, T11-004), and must not
           conclude from this surface's behaviour that holes are acceptable.
@guard     Catches `Nat` vs `Int` confusion in the operator vocabulary. `le`, `lt`, `ge`,
           `gt`, `add`, `sub`, `mul` are the prelude's opaque `Int` primitives
           (prelude.rs:352-515), axiomatised, not the `Nat` inductive. A Lean theorem
           about `Nat` subtraction translated to literate `x - 1 < x` is a statement
           about LOGOS `Int` and is NOT the Lean proposition (Lean's `Nat` subtraction
           truncates; see T10-017).
@oracle    Two steps, both required. (1) `Repl::execute(<emitted block>)` returns `Ok`.
           (2) The derivation actually concludes the goal: register the statement
           separately and check that `Eval (concludes <name>)` equals `Eval <goalSyntax>`
           — the pattern `tests/phase96_tactics.rs:276-301` uses. Step (1) alone is not
           an oracle for this case; see @guard.
-/

namespace T12_008

/-- `Statement: (le x x).` / `Statement: x <= x.` — the s-expression and infix spellings
    of the same proposition. -/
theorem leRefl (x : Nat) : x ≤ x := Nat.le_refl x

/-- `Statement: 2 < 5.` — a closed numeric comparison. -/
theorem twoLtFive : (2 : Nat) < 5 := by decide

/-- `Statement: x < x + 1.` — infix `+` in the right operand. -/
theorem ltSucc (x : Nat) : x < x + 1 := Nat.lt_succ_self x

/-- `Statement: 5 >= 3.` / `Statement: 5 ≥ 3.` — the ASCII and Unicode spellings both
    map to `ge` (literate_parser.rs:1269-1270). -/
theorem fiveGeThree : (5 : Nat) ≥ 3 := by decide

/-- `Statement: 2 * x <= 2 * x + 1.` — multiplicative and additive infix together. -/
theorem linearCoeff (x : Nat) : 2 * x ≤ 2 * x + 1 := Nat.le_succ _

/-- `Statement: (add x y) equals (add y x).` — the s-expression application form with
    the `equals` infix, which is what `Proof: ring.` is for. -/
theorem addComm' (x y : Nat) : x + y = y + x := Nat.add_comm x y

/-- `Statement: (Eq (add (mul 2 3) (sub 4 1)) 9).` — fully parenthesised prefix form,
    nested three deep. -/
theorem mixedArith : 2 * 3 + (4 - 1) = 9 := by decide

/-- `Statement: (implies (Gt x 0) (Ge x 1)).` — the `implies` infix/prefix form builds a
    non-dependent `Pi` (literate_parser.rs:1239-1250), so a Lean implication between two
    comparisons does have a rendering. -/
theorem gtToGe (x : Nat) (h : x > 0) : x ≥ 1 := h

end T12_008
