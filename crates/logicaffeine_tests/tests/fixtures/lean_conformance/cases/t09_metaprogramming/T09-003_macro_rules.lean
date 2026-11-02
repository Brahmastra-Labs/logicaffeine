/-!
@case      T09-003
@tier      9 · Metaprogramming and syntax
@title     `macro_rules` — a RECURSIVE, multi-pattern rewriter on `Syntax` that must be run,
           not reimplemented; its output is ordinary
@lean      macro_rules, macro, recursive macro_rules, several alternatives for one syntax
@expect    translate
@target    The translation of the EXPANDED term. The `macro_rules` command produces NO LOGOS
           declaration; whatever the expansion elaborates to translates by the ordinary Tier 0
           rules.
@evidence  kernel/src/term.rs:443-545 (`Term` — no `Syntax`, no quotation, no macro variant, no
           way to record that a term arose from an expansion);
           kernel/src/context.rs:20-80 (`Context` — no macro table, no syntax-category registry,
           no expansion history);
           kernel/src/elaborate.rs (elaboration consumes `Term` and produces `Term`; there is no
           surface-tree stage in the kernel at all, so nothing downstream of the translator can
           expand anything);
           proof/src/tactic_script.rs:113-133, :250-260 (`TacticEnv` — the ONLY user-extension
           mechanism anywhere in LOGOS: a name-to-script-source map, expanded in place at parse
           time, DEPTH-BOUNDED at `MAX_TACTIC_DEPTH = 128` (:133) so a recursive definition is
           refused rather than looped. It is a textual macro over TACTIC SCRIPTS, not over
           terms, and it cannot pattern-match on anything);
           kernel/src/interface/literate_parser.rs:1223-1270 (the literate surface's fixed infix
           table — likewise not extensible)
@guard     THE REIMPLEMENTATION TRAP, and the sharpest instance of the governing rule.
           `macro_rules` is a pattern-matching, potentially RECURSIVE rewriter with several
           alternatives tried in order, hygiene, and `Syntax`-level antiquotation. A translator
           that reads surface `Syntax` must therefore implement Lean's macro expander —
           alternative ordering, recursion, hygiene and all — and any divergence produces a
           well-formed term with a different meaning. **Translate the post-elaboration `Expr`,
           never the surface `Syntax`.** After elaboration `nested![a, b, c]` is already
           `cons a (cons b (cons c nil))` and the macro has left no trace to get wrong.
@guard     Catches ALTERNATIVE ORDER. `macro_rules` alternatives are tried in declaration order,
           and a later `macro_rules` block for the same syntax PREPENDS its alternatives. A
           translator that collects the alternatives into a set, or that applies them in source
           order across blocks, expands differently from Lean. `overlappingRules` below has two
           blocks whose alternatives overlap, so the result depends on this ordering rule.
@guard     Catches RECURSION. A recursive `macro_rules` expands until a base alternative
           matches. A `Syntax`-reading translator needs a termination story of its own; LOGOS
           offers no help, because its one macro-like facility (`TacticEnv`, tactic_script.rs:
           113-133) is depth-bounded at 128 and refuses recursion rather than supporting it.
@guard     Catches emitting a LOGOS declaration for the macro. There is no target. In
           particular, mapping a term-level `macro_rules` onto `TacticEnv::define`
           (tactic_script.rs:123-125) is a category error: that registry holds TACTIC SCRIPT
           text, keyed case-insensitively, expanded textually with no pattern matching — it
           cannot represent a term macro, and registering one there would make a tactic name
           that expands to nonsense.
@guard     Catches HYGIENE. A macro that introduces a binder must not capture a name from the
           call site. Lean's expander handles this with macro scopes; the resulting `Expr` has
           already-freshened binder names. A `Syntax`-reading translator that expands naively
           captures, and since LOGOS's representation is NAMED (`Term::Var(String)`,
           term.rs:449) and substitution alpha-renames only on collision
           (type_checker.rs:1088), the capture survives into the kernel as a silently different
           term. `hygienic` below is the witness: the macro introduces `x` and the call site
           also has an `x`.
@oracle    The `macro_rules` commands below produce ZERO LOGOS declarations. Each theorem's
           translated term is `types_equal` to the translation of the same theorem written with
           the expansion inlined by hand — in particular `hygienic` must translate to a term
           whose inner binder is DISTINCT from the call site's `x`.
-/

namespace T09_003

inductive MyList where
  | nil : MyList
  | cons : Nat → MyList → MyList

/-- A recursive `macro_rules` over a list-literal syntax. -/
syntax "nested![" term,* "]" : term

macro_rules
  | `(nested![]) => `(MyList.nil)
  | `(nested![$x]) => `(MyList.cons $x MyList.nil)
  | `(nested![$x, $xs,*]) => `(MyList.cons $x (nested![$xs,*]))

theorem expandsFully :
    nested![1, 2, 3] = MyList.cons 1 (MyList.cons 2 (MyList.cons 3 MyList.nil)) := rfl

theorem expandsEmpty : nested![] = MyList.nil := rfl

/-- Two `macro_rules` blocks for the SAME syntax, with overlapping patterns. Lean tries the
    later block's alternatives FIRST. -/
syntax "pick![" term "]" : term

macro_rules
  | `(pick![$x]) => `($x + 1)

macro_rules
  | `(pick![$x]) => `($x + 100)

theorem overlappingRules : pick![5] = 105 := rfl

/-- A `macro` (the single-alternative shorthand) that introduces a BINDER. The macro's `x` must
    not capture the call site's `x`. -/
macro "twiceIt " e:term : term => `(fun x => $e + $e + x)

theorem hygienic (x : Nat) : (twiceIt x) 0 = x + x := rfl

/-- A macro whose expansion is a PROOF, not data. -/
macro "reflAll" : tactic => `(tactic| rfl)

theorem macroTactic (n : Nat) : n = n := by reflAll

/-- A macro applied to a COMPOUND argument, so the expansion is not a leaf substitution. -/
theorem compound (a b : Nat) : (twiceIt (a + b)) 0 = (a + b) + (a + b) := rfl

end T09_003
