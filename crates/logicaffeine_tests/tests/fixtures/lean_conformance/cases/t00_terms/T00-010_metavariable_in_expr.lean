/-!
@case      T00-010
@tier      0 · Kernel term layer
@title     `Expr.mvar` reaching the translator — an uninstantiated metavariable must be refused
@lean      Expr.mvar, `?a`, `_` placeholders, delayed-assignment metavariables from tactic blocks
@expect    reject
@target    (none — no sound translation)
@diag      E_UNINSTANTIATED_METAVARIABLE — must name the metavariable, the declaration it was
           found in, and the position in the term. Must NOT emit a `Term::Hole`, a
           `Term::Var("?…")`, or a guessed instantiation.
@evidence  kernel/src/term.rs:443-545 (the `Term` enum has NO metavariable variant — LOGOS_FACTS
           states this explicitly: "There is no `Proj`, no metavariable variant, no `mdata`, no
           `fvar`/`bvar` distinction");
           kernel/src/elaborate.rs:137-140 (`is_meta`: LOGOS models elaborator metavariables as
           `Term::Var` names beginning with `?` — a NAMING CONVENTION inside the elaborator, not
           a kernel form) and elaborate.rs:142-150 (`instantiate` must have replaced every one of
           them before the kernel sees the term);
           kernel/src/type_checker.rs:313 (`Term::Hole => Err(KernelError::CannotInferHole)`);
           kernel/src/type_checker.rs:455-471 (but `check_type` ACCEPTS a `Hole` term against any
           `Sort`, and accepts any term against a `Hole` expected type) and
           kernel/src/type_checker.rs:1594-1597 (`types_equal` returns `true` whenever EITHER
           side is a `Hole`), which is why the kernel is not a backstop here
@guard     Catches `Expr.mvar ⟼ Term::Hole`, the natural-looking mapping. It is the worst
           available option. `CannotInferHole` fires only if the `Hole` lands in a position that
           `infer_type` actually visits (type_checker.rs:313); a `Hole` sitting in an argument's
           TYPE, in a `Pi`'s `param_type`, or in a `Let`'s annotation is never inferred — it is
           only ever *compared*, and `types_equal` (type_checker.rs:1594-1597) says a `Hole`
           equals anything. The leniency also leaks into `is_subtype`'s fast path
           (type_checker.rs:1314), so a single surviving `Hole` in a theorem's statement makes
           that statement satisfiable by a proof of something else entirely. See T00-016.
@guard     Catches `Expr.mvar ⟼ Term::Var("?m_1")`, i.e. reusing LOGOS's own metavariable
           convention. `elaborate.rs:137-140` will classify that `Var` as a metavariable and
           `instantiate` (elaborate.rs:142-150) will substitute whatever the ambient `MetaCtx`
           happens to have solved for the name `?m_1` — a name the translator invented, whose
           collision with a genuine elaborator metavariable is entirely accidental. The result is
           a term silently rewritten to something unrelated, with no error at any stage.
@guard     Catches "the metavariable is obviously `Nat`, so instantiate it". Guessing is a
           soundness decision disguised as a convenience: LOGOS is cumulative
           (type_checker.rs:1330-1360), so a guessed type that is merely a SUPERTYPE of the right
           one still typechecks everywhere, and the error surfaces only where the two differ.
@guard     Catches reading the `Expr` before `instantiateMVars`. Every term in this file
           elaborates to an mvar-free `Expr` — but only AFTER instantiation. A translator hooked
           in at `Term.elabTerm` rather than at the finished `Declaration` sees live `mvar`s for
           each `_` below, and `⟨_, rfl⟩`'s witness is assigned only when `rfl` is unified.
           Delayed-assignment metavariables (produced by every `by` block) are not resolved even
           by `instantiateMVars` until the tactic block closes.
@oracle    The translator runs `instantiateMVars` and then asserts the `Expr` contains no
           `.mvar`; if any remains it exits non-zero with E_UNINSTANTIATED_METAVARIABLE and
           writes NO declaration. Independently: the emitted `Term` for every declaration in the
           corpus contains no `Term::Hole` and no `Term::Var` whose name starts with `?`.
-/

namespace T00_010

/-- A `_` that unification solves from the expected type. Live `?m : Type` until then. -/
def underscoreFromExpectedType : Nat := (id (α := _) 3)

/-- A `_` solved from a LATER argument — the metavariable outlives its own position in the
    spine, so a left-to-right translator walks past an unsolved node. -/
def constFn {α : Type} {β : Type} (a : α) (_b : β) : α := a
def underscoreFromLaterArg : Nat := @constFn _ Bool 3 true

/-- The anonymous-constructor witness: `?w` is solved only when `rfl` is unified against
    `?w = 1`. Between those two points the `Expr` genuinely contains an `mvar`. -/
theorem existsViaHole : ∃ n : Nat, n = 1 := ⟨_, rfl⟩

/-- A `by` block produces a DELAYED-ASSIGNMENT metavariable: `instantiateMVars` will not resolve
    it until the tactic block has run to completion. -/
theorem viaTactic : 2 + 2 = 4 := by decide

/-- Nested: a tactic block inside a structure instance. The `by omega` proof is a delayed-
    assignment metavariable whose type mentions the (already elaborated) first component. -/
def nestedMetas : Fin 5 := ⟨3, by omega⟩

/-- An implicit argument that is never written and never printed, but is an `mvar` throughout
    elaboration and must be fully instantiated before the level is read (cf. T01-012 for the
    universe-level analogue). -/
def implicitTypeArg : List Nat := List.cons 1 []

/-- A metavariable in a POSITION `infer_type` never visits: the domain of a function type that
    is only ever compared, never inferred. This is where a `Hole` survives undetected. -/
def hiddenPosition : (Nat → Nat) → Nat := fun f => f 0

end T00_010
