/-!
@case      T08-010
@tier      8 · Tactics
@title     `induction xs` on a list — the script `induction` is hard-wired to `Nat` and will
           silently substitute `Zero` / `Succ K` into a list-typed goal
@lean      induction on List, induction on a user inductive, induction using a custom eliminator
@expect    reject
@target    (none — no sound translation)
@evidence  proof/src/tactic.rs:599-603 (`InferenceRule::StructuralInduction { variable,
           ind_type: "Nat".to_string(), step_var }` — the inductive type is a HARD-CODED
           string literal, not derived from the goal);
           proof/src/tactic.rs:583-586 (the base target is `body[variable := Constant("Zero")]`
           and the step target is `body[variable := Function("Succ", [Constant(step_var)])]` —
           the two Peano constructors, by name, unconditionally);
           proof/src/tactic.rs:556-560 (the method's own doc comment: "Nat only for now —
           `List`/general inductives need the dependent `InductionScheme`");
           proof/src/tactic.rs:623-695 (`induction_over` — the generic scheme, one subgoal per
           `CtorSpec`, IHs for the recursive arguments — which is exactly what a list induction
           needs);
           proof/src/tactic_script.rs:275-292 (`parse_primitive`'s dispatch — `induction_over`
           has NO arm, and `canonical` (:308-327) has no word mapping to it, so it is
           unreachable from script text; cf. T08-020);
           proof/src/lib.rs:199-238 (`ProofExpr` is UNTYPED first-order logic: a `ProofTerm`
           carries no type, so substituting `Zero` for a list-valued variable is well-formed at
           this layer and raises nothing);
           kernel/src/prelude.rs (the LOGOS prelude registers `EList`/`ENil`/`ECons` and
           `TList`, but Lean's `List` is ABSENT — see T10-012 — so even the generic scheme needs
           the inductive to be declared first)
@diag      E_INDUCTION_NOT_NAT — must name the variable, name its Lean inductive type, state
           that the script-level `induction` is `Nat`-only (tactic.rs:601) and that the generic
           `induction_over` is not reachable from script text (tactic_script.rs:275-292), and
           must NOT emit an `induction` atom. If the translator has Rust-level access it may
           instead emit `induction_over` with an explicit `Vec<CtorSpec>` and downgrade this
           case to `degrade` — see the @note.
@guard     THE HIGHEST-VALUE TIER-8 REJECTION. The tempting wrong move is to emit the bare
           `induction` atom and let the goal shape sort itself out. It will not fail. LOGOS's
           `induction` requires only that the goal be a `ProofExpr::ForAll`
           (tactic.rs:563-570) — it never looks at the variable's type, because there are no
           types at that layer. Against `∀ xs, length (append xs nil) = length xs` it produces
           the base goal `length (append Zero nil) = length Zero` and the step goal
           `length (append (Succ K1) nil) = length (Succ K1)`. Those are syntactically
           well-formed `ProofExpr`s. `auto`, `simp` and `crush` will all happily try to prove
           them, and for a statement whose predicate is insensitive to the substituted term —
           a reflexivity, a tautology, anything the backward chainer closes structurally —
           they will SUCCEED. The resulting `StructuralInduction` node claims a `Nat` induction
           for a `List` statement, and the failure, if it comes at all, surfaces in
           `check_derivation` as a kernel type error about `Nat` in a theorem that never
           mentioned `Nat`.
@guard     Catches "map `List` onto `Nat` by length". Inducting on `xs.length` instead of `xs`
           is a real proof technique, but it is a DIFFERENT proof: the induction hypothesis is
           `∀ ys, length ys = k → P ys`, not `P tail`. A translator that silently performs this
           reindexing has rewritten the user's proof, and the generalisation it must insert
           (over all lists of that length) is not expressible with the script atoms available.
@guard     Catches "use `induction_over`". It is the right scheme — but it is a Rust API
           (tactic.rs:623-695) with no script spelling, and it requires the caller to supply a
           `Vec<CtorSpec>` listing each constructor and which of its arguments are recursive
           (proof/src/tactic.rs:40-58). A translator that produces `ProofStrategy::Script` text
           cannot reach it, and one that drives `ProofState` from Rust must ALSO have declared
           the inductive in the kernel context first, since Lean's `List` is not in the LOGOS
           prelude. Claiming `induction_over` as the target without doing both is a phantom
           translation.
@guard     Catches `induction … using <custom eliminator>` and strong/two-step induction. Even
           the Rust `induction_over` is plain structural induction, one IH per recursive
           argument (tactic.rs:668-675); `Nat.strong_induction_on`, `List.reverseRecOn` and
           friends have no analogue, and LOGOS has no well-founded recursion at all
           (kernel/src/termination.rs — syntactic Giménez guard only).
@note      The index records this as `reject`, which is correct for a translator that emits
           script TEXT — the only surface a Lean tactic block can be compiled to. A translator
           that drives `ProofState` through the Rust API may legitimately emit `induction_over`
           with an explicit constructor list; for that translator this case is `degrade` with
           loss SCHEME_SELECTION (case names, IH names and IH placement are all machine-chosen,
           as in T08-009). The rejection stands for the script surface.
@oracle    Translator exits non-zero with E_INDUCTION_NOT_NAT for every theorem below and emits
           no script containing the atom `induction`. A harness additionally asserts that no
           emitted `StructuralInduction` rule anywhere in the corpus carries `ind_type` other
           than `"Nat"`, and that every goal an `induction` atom runs against has a `Nat`-typed
           outermost binder in the Lean source.
-/

namespace T08_010

/-- A local `List` — Lean's `List` is not in the LOGOS prelude either, so the inductive itself
    must be declared before any induction over it is even nameable. -/
inductive LList where
  | nil : LList
  | cons : Nat → LList → LList

def len : LList → Nat
  | .nil => 0
  | .cons _ t => len t + 1

def app : LList → LList → LList
  | .nil, ys => ys
  | .cons h t, ys => .cons h (app t ys)

/-- The canonical list induction. Two constructors, one recursive argument on the second. -/
theorem lenApp (xs ys : LList) : len (app xs ys) = len xs + len ys := by
  induction xs with
  | nil => simp [app, len]
  | cons _h t ih => simp [app, len, ih]; omega

/-- A statement whose predicate is insensitive to the scrutinee — the case where substituting
    `Zero` / `Succ K` produces a goal that automation CLOSES, so the mistranslation is silent. -/
theorem insensitive (xs : LList) : len xs = len xs := by
  induction xs with
  | nil => rfl
  | cons _h _t _ih => rfl

/-- Induction on a user inductive with THREE constructors and TWO recursive arguments — outside
    the nullary-base/unary-step shape `ProofState::induction` bakes in. -/
inductive Tree where
  | leaf : Tree
  | tip : Nat → Tree
  | node : Tree → Tree → Tree

def size : Tree → Nat
  | .leaf => 0
  | .tip _ => 1
  | .node l r => size l + size r

theorem sizeNonneg (t : Tree) : 0 ≤ size t := by
  induction t with
  | leaf => simp [size]
  | tip _n => simp [size]
  | node _l _r _ihl _ihr => simp [size]

/-- Induction using a CUSTOM eliminator: no analogue even in `induction_over`. -/
theorem strongOnNat (p : Nat → Prop) (h : ∀ n, (∀ m, m < n → p m) → p n) : ∀ n, p n := by
  intro n
  induction n using Nat.strongRecOn with
  | ind k ih => exact h k ih

end T08_010
