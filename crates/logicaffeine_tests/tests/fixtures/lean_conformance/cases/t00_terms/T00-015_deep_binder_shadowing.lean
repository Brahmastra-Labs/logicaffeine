/-!
@case      T00-015
@tier      0 · Kernel term layer
@title     Deep binder shadowing — alpha-equivalence when binders are NAMES, not indices
@lean      nested binders reusing one name, alpha-equivalent `Expr`s, hygienic shadowing
@expect    translate
@target    Nested `Term::Pi` / `Term::Lambda` / `Term::Fix` / `Term::Let` whose `param` / `name`
           fields are GLOBALLY FRESH within the emitted declaration — one freshening pass over
           the whole term, not a per-binder decision.
@evidence  kernel/src/term.rs:463-476 (`Pi.param`, `Lambda.param`, `Fix.name`, `Let.name` are all
           plain `String`s);
           kernel/src/context.rs:228-241 (`Context::add`/`extend` insert into a
           `HashMap<String, Arc<Term>>` — a second binder with the same name REPLACES the first,
           so the outer binding becomes unreachable rather than being shadowed in a stack);
           kernel/src/type_checker.rs:1600-1607 (`types_equal` compares `Var` names LITERALLY:
           `(Var(n1), Var(n2)) => n1 == n2` — there is no index, so alpha-equivalence is
           implemented by RENAMING, at :1611-1620 for `Pi` and :1507-1515 for `Lambda`, where the
           second term's binder is `substitute`d to the first's name);
           kernel/src/type_checker.rs:1088-1110 (`substitute_avoiding` freshens a binder when it
           would capture a free variable of the REPLACEMENT — so the rename used by the
           congruence rules is itself capture-safe);
           kernel/src/term.rs:577-590 (`Display` prints `Π({param}:{ty}). {body}` — two nested
           binders of the same name print IDENTICALLY, with nothing to distinguish them);
           kernel/src/recheck.rs (the second kernel converts to de Bruijn deliberately, so a
           naming error shows up as a DISAGREEMENT between kernels rather than as an error)
@guard     Catches reusing Lean's binder names verbatim. It is very nearly safe — the kernel's
           own congruence and substitution are capture-correct — and that is exactly what makes
           it dangerous, because the failure is not in the kernel. It is in every OTHER consumer:
           `Display` (term.rs:577-590) renders `Π(n:Nat). Π(n:Fin n). Nat` with two
           indistinguishable `n`s, so any round-trip through printed form (a certificate dump, a
           literate-surface re-parse, a diff between two translator versions) reads back a
           DIFFERENT term — the inner `n` in `Fin n` now refers to the inner binder. The term
           that was checked and the term that is stored are then not the same term.
@guard     Catches freshening PER BINDER instead of per term. A pass that renames a binder only
           when it collides with an enclosing binder still produces collisions with names
           introduced later by the kernel itself: `derive_recursor` (recursor.rs:79-81) generates
           binders literally named `A0`, `A1`, `Idx0`, `Idx1`, and `add_structure`
           (context.rs:295-320) generates projection bodies with its own binders. A Lean binder
           innocently named `A0` collides with a generated one inside a derived term, where the
           translator's freshening pass never ran.
@guard     Catches assuming `Context::extend` behaves like a stack. It does not — it is a map
           insert (context.rs:228-241). So after `ctx.extend("n", Nat).extend("n", Fin n)` the
           OUTER `n` is gone from the context entirely. Any code the translator writes that
           looks up a binder by name after extending (for instance, to compute a motive) reads
           the wrong type, and the resulting `Match` motive is well-typed against the wrong
           discriminant type.
@guard     Catches asymmetric freshening between a TERM and its TYPE. A declaration is emitted as
           a (type, body) pair (context.rs:442). If the freshening pass runs independently on
           each, the same Lean binder gets two different fresh names, and the body no longer
           matches the type's telescope. `check_type`'s `Lambda`-against-`Pi` rule
           (type_checker.rs:474-500) prefers the Pi's parameter type and will happily accept the
           mismatch, binding the body's variable to the wrong name and leaving the other
           dangling.
@oracle    Walking the emitted term, no binder name is ever introduced while a binder of the same
           name is already in scope; `infer_type` succeeds; the de Bruijn re-checker in
           `recheck.rs` agrees with `type_checker.rs` on the inferred type; and
           `parse(Display(term))` — where a surface parser exists — yields a term `def_eq` to the
           original.
-/

namespace T00_015

/-- Three lambdas all named `x`; the body means the INNERMOST. -/
def threeX : Nat → Nat → Nat → Nat := fun x => fun x => fun x => x

/-- Three lambdas named `x`; the body means the OUTERMOST. Under verbatim names this term is
    not expressible — every `Var("x")` resolves to the innermost binding. -/
def outerX : Nat → Nat → Nat → Nat := fun x => fun _ => fun _ => x

/-- A `Pi` whose inner binder's TYPE mentions the outer binder of the same name. The occurrence
    in `Fin n` is the OUTER `n`; the codomain's `n` would be the INNER one. -/
def shadowInPiType : Type := ∀ (n : Nat), ∀ (n : Fin n), Nat

/-- Five deep, alternating meaning: the freshening pass must keep four distinct names alive. -/
def fiveDeep : Type :=
  ∀ (n : Nat), ∀ (n : Fin (n + 1)), ∀ (n : Fin (n.val + 1)), ∀ (_n : Fin (n.val + 1)), Nat

/-- A `let` whose bound name shadows a lambda binder, so the collision crosses TERM FORMS —
    `Let.name` and `Lambda.param` are different fields of different variants but the same
    flat namespace at check time (type_checker.rs:133-142 substitutes by name). -/
def letShadowsLambda (n : Nat) : Nat :=
  let n : Nat := n + 1
  let n : Nat := n + 1
  n

/-- A recursive function whose `Fix` name collides with a binder name. `Term::Fix { name, body }`
    (term.rs:472) binds `name` in `body`, in the same namespace as the lambda binders. -/
def loop : Nat → Nat
  | 0 => 0
  | k + 1 => loop k

/-- Binders named exactly like the identifiers `derive_recursor` generates (recursor.rs:79-81).
    A per-binder freshening pass leaves these alone, and they then collide inside a derived
    recursor the translator never wrote. -/
def generatedNameCollision : Type 1 := ∀ (A0 : Type), ∀ (Idx0 : Nat), ∀ (_ : Fin Idx0), A0

/-- Alpha-equivalence as a positive obligation: these two are the SAME `Expr` in Lean and must
    be `def_eq` in LOGOS after freshening. -/
def alphaA : Type := ∀ (a : Nat), Fin (a + 1) → Nat
def alphaB : Type := ∀ (b : Nat), Fin (b + 1) → Nat

example : alphaA = alphaB := rfl

end T00_015
