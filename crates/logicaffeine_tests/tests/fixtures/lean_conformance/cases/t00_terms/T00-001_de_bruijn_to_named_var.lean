/-!
@case      T00-001
@tier      0 · Kernel term layer
@title     de Bruijn `Expr.bvar` / `Expr.fvar` lowered to a NAMED `Term::Var`
@lean      Expr.bvar, Expr.fvar, binder names as hints, hygienic/inaccessible names
@expect    translate
@target    Term::Var(String) — kernel/src/term.rs:447. Every `bvar i` becomes the *name* of the
           binder `i` levels out, after a freshening pass that makes every binder name in the
           emitted term globally distinct from every other binder it could shadow.
@evidence  kernel/src/term.rs:443-545 (`Term` has exactly one variable form, `Var(String)`;
           there is NO `bvar`/`fvar` distinction and no de Bruijn index);
           kernel/src/type_checker.rs:57-61 (`Var` is resolved by NAME against `Context`, which
           context.rs:228-236 implements as a `HashMap<String, Arc<Term>>` — so two binders with
           the same name are literally the same key and the inner one *erases* the outer);
           kernel/src/type_checker.rs:1088-1110 (`substitute_avoiding` alpha-renames a binder
           only when the REPLACEMENT's free variables would be captured — it does not repair a
           term that was already emitted with a wrong name);
           kernel/src/elaborate.rs:137-140 (`is_meta`: any `Var` whose name starts with `?` is
           treated as an elaborator metavariable, not as a bound variable);
           kernel/src/recheck.rs (the second kernel converts back to de Bruijn — a naming bug
           shows up as a disagreement between the two kernels, not as a clean error)
@guard     Catches the "binder names are only hints, so reuse them verbatim" translator. Lean
           binder names are non-unique *by design*: `fun x => fun x => x` is a legal `Expr` whose
           two `x` binders are distinguished only by de Bruijn index. Emitting both as `Var("x")`
           does not fail — `Context` is a name-keyed map, so the inner binding simply shadows the
           outer, and the term still typechecks with the WRONG variable resolved at every
           occurrence that meant the outer one. This is the canonical silent mistranslation:
           `infer_type` succeeds, the inferred type is plausible, and the meaning is different.
@guard     Catches emitting Lean's *inaccessible* / macro-scoped names (`x✝`, `n✝¹`,
           `_uniq.1234`) verbatim. `Universe`/`Term` names are flat `String`s with no interning
           discipline, so `x✝` and `x✝¹` are distinct keys and happen to work — but a sanitiser
           that strips the dagger collapses them onto one key, reintroducing the capture above.
           Any sanitiser must be injective over the binders live at each point.
@guard     Catches a sanitiser that emits a `?`-prefixed name. `elaborate.rs:137-140` classifies
           `Var("?x")` as a metavariable and `instantiate` will silently replace it with whatever
           the metavariable context has solved for `?x` — i.e. a bound variable is rewritten into
           an unrelated term. `?` must never appear in a translated binder name.
@guard     Catches confusing `fvar` with `Term::Global`. A Lean free variable is a *local*
           hypothesis and must become `Term::Var`; lowering it to `Term::Global` sends it through
           `get_global` (context.rs:449: inductives → constructors → definitions → declarations),
           where it either fails with `UnboundVariable` or — far worse — resolves to a
           same-named global from the prelude.
@oracle    For every binder in the emitted term, the set of binder names in scope at that point
           has no duplicates; no emitted `Var` name starts with `?`; `infer_type` succeeds; and
           re-checking the same term through `recheck.rs`'s de Bruijn kernel yields the same
           type. A term that passes `infer_type` but not the de Bruijn re-check is a naming bug.
-/

namespace T00_001

/-- Trivial case: one binder, one use. `bvar 0` ⟼ `Var("a")`. -/
def identityNat (a : Nat) : Nat := a

/-- Two binders, the OUTER one used. A translator that computes the de Bruijn index from the
    wrong end emits the inner binder's name here and still typechecks. -/
def pickOuter (a : Nat) (b : Nat) : Nat := a

/-- The adversarial case: three binders that all share the name `x`. Lean distinguishes them by
    index; a named target must freshen two of them. The body means the INNERMOST `x`. -/
def shadowedInnermost : Nat → Nat → Nat → Nat :=
  fun x => fun x => fun x => x

/-- Same shape, but the body means the OUTERMOST `x` — reachable in Lean only because the
    indices differ. Verbatim name reuse cannot express this term at all: every occurrence of
    `Var("x")` resolves to the innermost binding. -/
def shadowedOutermost : Nat → Nat → Nat → Nat :=
  fun x => fun _y => fun _z => x

-- A free variable (`fvar`) captured from a `variable` binder: Lean inserts it as a leading
-- binder before the translator ever sees the `Expr`, so it must be a `Var`, not a `Global`.
section
variable (k : Nat)

def usesSectionVar : Nat := k + 1

end

/-- A binder whose name collides with a prelude *global* (`Nat` is a registered inductive in
    LOGOS, kernel/src/prelude.rs). As a `Var` this is fine; as a `Global` it silently becomes
    the inductive. -/
def binderNamedLikeAGlobal (Nat : Bool) : Bool := Nat

/-- Nested binders where the inner binder's TYPE mentions the outer binder. The freshening pass
    must rename the binder but not the occurrence inside the type it scopes over. -/
def innerTypeMentionsOuter : Type := ∀ (n : Nat), ∀ (n : Fin n), Nat

end T00_001
