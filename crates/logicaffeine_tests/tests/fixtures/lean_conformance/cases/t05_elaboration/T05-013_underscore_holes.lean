/-!
@case      T05-013
@tier      5 · Elaboration surface
@title     `_` holes resolved by unification — must be fully instantiated before the kernel
@lean      `_` placeholder, `(_ : T)`, holes solved by unification against the expected type
@expect    translate
@target    The SOLVED term, with every hole replaced. `idf _ 3` becomes
           `App(App(Global("idf"), Global("Nat")), Lit(Nat 3))`. The emitted term must contain
           **no `Term::Hole`** and **no `Term::Var` whose name begins with `?`**.
@evidence  kernel/src/term.rs:540-544 (`Term::Hole` — "implicit argument"; it is the only
           unknown in the term language and it is not a metavariable);
           kernel/src/type_checker.rs:313 (`Term::Hole => Err(KernelError::CannotInferHole)` —
           a hole reaching `infer_type` is a hard error);
           kernel/src/type_checker.rs:1593-1597 (`types_equal` returns **true** whenever either
           side is a `Hole` — a hole matches ANYTHING) and :1314 (that leniency leaks into
           `is_subtype`'s fast path);
           kernel/src/type_checker.rs:456-470 (`check_type` special-cases `Hole` on both the
           term and the expected side, which is what lets holes survive `check_type` while
           failing `infer_type`);
           kernel/src/elaborate.rs:137-139 (`is_meta`: elaborator metavariables are
           `Term::Var` names beginning with `?`, NOT a `Term` variant);
           kernel/src/elaborate.rs:143-186 (`instantiate` substitutes every solved
           metavariable, transitively — this is the call that must precede handing the term to
           the kernel)
@guard     **Translate the post-elaboration `Expr`, never the surface `Syntax`.** A `_` has no
           meaning in `Syntax`; Lean's unifier decides what it is. After elaboration the `Expr`
           holds the solution. A translator on `Syntax` must run its own unifier against
           LOGOS's — whose `unify_in` (elaborate.rs:216-292) is a *different* algorithm with
           Miller-pattern restrictions (T05-014) and an eta rule — and will diverge from Lean
           wherever the two disagree.
@guard     Catches emitting `Term::Hole` for an unsolved `_` "to be filled in later". This is
           the single most dangerous output in the corpus. `types_equal` treats a `Hole` as
           matching **anything** (type_checker.rs:1593-1597), and `is_subtype`'s fast path
           inherits that (:1314) — so a declaration whose TYPE contains a hole will typecheck
           against any type at all, and every theorem stated with it proves nothing. See
           T00-016 and T11-004.
@guard     Catches the asymmetry between `check_type` and `infer_type`. `check_type`
           accommodates holes (type_checker.rs:456-470) while `infer_type` rejects them
           (:313). A translator that validates its output with `check_type` against an expected
           type it also emitted will therefore *pass* on hole-carrying terms, and only fail
           later when something calls `infer_type` — typically inside a `Match` motive or a
           `Fix` sanity check, far from the source of the hole.
@guard     Catches leaving an elaborator METAVARIABLE in the output. These are not `Hole`s:
           they are `Term::Var("?m0")` (elaborate.rs:137-139). The kernel has no idea they are
           special — it treats `?m0` as an ordinary free variable, so it either fails lookup or,
           in LOGOS's NAMED representation (term.rs:450), resolves against a binder that
           happens to share the name. `instantiate` (elaborate.rs:143-186) must be called, and
           its result must be checked to contain no `?`-prefixed `Var`.
@guard     Catches a `_` in a POSITION Lean solved from a later constraint. In `viaLater`
           below the hole's solution comes from the expected type of the whole expression, not
           from any argument — so a translator that solves left-to-right and gives up when an
           argument does not pin the hole emits a hole where Lean had an answer.
@guard     Catches `_` standing for a PROOF. In `holeProof` the `_` is filled by a proof term;
           because proofs are `Prop` inhabitants, definitional proof irrelevance
           (type_checker.rs:1547-1567) will accept almost anything there — including a hole —
           so nothing downstream complains.
@oracle    The emitted term contains no `Term::Hole` and no `Term::Var` whose name starts with
           `?`; `infer_type` (not merely `check_type`) succeeds on it; and it is term-for-term
           identical to the translation of the same expression written with every `_` filled in
           by hand.
-/

namespace T05_013

def idf {α : Type} (a : α) : α := a

/-- A hole in an explicit type-argument position, solved by the value argument. -/
example : @idf _ 3 = 3 := rfl

/-- A hole solved from the EXPECTED type rather than from any argument. -/
def viaLater : List Nat := @List.nil _

/-- A hole in an ascription. -/
example : (3 : _) = (3 : Nat) := rfl

/-- A hole standing for a PROOF — proof irrelevance makes this the least-checked position. -/
theorem holeProof (n : Nat) : n = n := by
  exact (rfl : n = _)

/-- A hole in a DEPENDENT position: the solution of the first determines the second. -/
def applyTo {α : Type} {β : α → Type} (f : (x : α) → β x) (a : α) : β a := f a

example : @applyTo Nat _ (fun n => n + 1) 2 = 3 := rfl

/-- Several holes in one spine, solved by different constraints. -/
def compose {α β γ : Type} (g : β → γ) (f : α → β) (a : α) : γ := g (f a)

example : @compose _ _ _ (· + 1) (· * 2) 3 = 7 := rfl

/-- A hole in a structure literal's parameter position. -/
structure Pair (α β : Type) where
  fst : α
  snd : β

example : (@Pair.mk _ _ 1 true).fst = 1 := rfl

/-- A hole that Lean solves through a MATCH motive — the hardest position, and the one where
    LOGOS's own `fill_match_motives` (elaborate.rs:578-636) is Miller-restricted. -/
def boolCase (b : Bool) : Nat :=
  match b with
  | true => 1
  | false => 0

example : boolCase true = 1 := rfl

/-- A hole in a `Fin` index, solved by the numeric literal's `OfNat` elaboration. -/
example : (⟨0, by decide⟩ : Fin 3).val = 0 := rfl

end T05_013
