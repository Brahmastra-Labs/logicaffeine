/-!
@case      T08-005
@tier      8 · Tactics
@title     `rfl` has no script spelling — it must be routed through `simp` or `decide`
@lean      rfl, rfl up to delta/iota, Eq.refl, rfl on a non-Eq reflexive relation
@expect    translate
@target    LOGOS script atom `simp` (canonical `simp`, arity 0, aliases `simplify`,
           `normalize`, `simplification` — proof/src/tactic_script.rs:282, :316) whose
           closing branch emits `InferenceRule::Reflexivity`
           (proof/src/tactic.rs:806-816), or `decide` (canonical `decide`, aliases `compute`,
           `evaluate`, `calculate` — tactic_script.rs:283, :317) for a ground goal
@evidence  proof/src/tactic_script.rs:308-327 (`canonical` — there is NO arm mapping any word
           to "rfl"; the word `rfl` in a script is `unknown tactic \`rfl\``);
           proof/src/tactic.rs:806-816 (`simp`'s closing attempt: if the normalised target is
           `ProofExpr::Identity(l, r)` with `l == r` it closes with a
           `InferenceRule::Reflexivity` leaf — this is the reflexivity route);
           proof/src/tactic.rs:820-826 (`simp` FAILS with `DoesNotApply` when it neither
           rewrote nor closed — so `simp` is not a no-op and cannot stand in for a `rfl` that
           had nothing to do);
           proof/src/decide.rs:1-12, :64 (`decide_expr` — a router over ground `Int` identities,
           ground comparisons and `Bool` equalities, with propositional structure recursing
           through the intro rules; it declines on anything open);
           kernel/src/type_checker.rs:1473-1493 (`def_eq_normalized` — the kernel's own
           definitional equality, which is what Lean's `rfl` corresponds to, and which the
           SCRIPT layer never calls: the script layer compares `ProofExpr`s, not kernel `Term`s)
@guard     Catches emitting the word `rfl`. It is not in the alias table (tactic_script.rs:
           308-327), so the script dies at parse time — before any step runs — with
           "unknown tactic `rfl`". Because `parse_script` is called from `run_script`
           (tactic_script.rs:358-362) and the state is left unchanged on a parse error, the
           failure is total: a single stray `rfl` anywhere in a long script discards the
           whole proof.
@guard     Catches assuming `simp` reproduces Lean's `rfl`. Lean's `rfl` succeeds up to FULL
           definitional equality in the kernel — delta-unfolding every transparent definition,
           iota-reducing every recursor application, beta, eta, and structure eta. LOGOS's
           reflexivity route is a SYNTACTIC `l == r` check on `ProofExpr` after simp-set
           rewriting (tactic.rs:808-816), and the simp set is only the in-scope hypotheses
           that orient as rules (tactic.rs:264-272). So `rfl` proving `2 + 2 = 4` by iota
           becomes `decide`; `rfl` proving `f x = g x` by unfolding `f` becomes NOTHING,
           because there is no delta step at the script layer at all.
@guard     Catches routing a NON-`Eq` reflexivity through this path. Lean's `rfl` closes any
           goal whose head is a relation tagged `@[refl]`, which in Mathlib includes `LE.le`.
           LOGOS's reflexivity branch matches only `ProofExpr::Identity` (tactic.rs:808), and
           `n ≤ n` translates to a `le` shape, not an `Identity`. The correct target there is
           the `le_refl` axiom (kernel/src/prelude.rs:202-340) reached through `auto`, not
           `simp` — and `simp` on such a goal fails outright (tactic.rs:820-826) rather than
           leaving it open, so the mis-route is at least loud.
@note      `LE.le` is NOT `@[refl]` in bare Lean 4.14 — `by rfl` on `n ≤ n` errors with "no
           @[refl] lemma registered for relation LE.le". Since this corpus forbids
           `import Mathlib`, `leRefl` below is written `Nat.le_refl n`; the Mathlib spelling
           `by rfl` is the construct the guard is about.
@guard     Catches the silent-success inversion. `decide` DECLINES a false goal rather than
           erroring (decide.rs:8-11: "A false, open, or unsupported goal yields `None` — decide
           declines, it never guesses"), and `simp` fails when it has nothing to do. So a
           translator cannot distinguish "the goal was already closed" from "the tactic did not
           apply" by exit status alone; it must inspect `open_goals()`.
@oracle    No emitted script contains the token `rfl`. For each `Eq`-shaped theorem the emitted
           atom is `simp` or `decide` and `run_script` leaves `open_goals() == 0`; the assembled
           derivation's closing rule is `Reflexivity` (simp route) or `ArithDecision`/
           `NativeDecide` (decide route, decide.rs:1-12). For `leRefl` the emitted atom must NOT
           be `simp`.
-/

namespace T08_005

/-- Syntactic reflexivity — the one case LOGOS's `simp` closing branch handles directly. -/
theorem syntactic (n : Nat) : n = n := by rfl

/-- Reflexivity up to IOTA: `2 + 2` must reduce. Routes to `decide`, not `simp`. -/
theorem iota : (2 : Nat) + 2 = 4 := by rfl

/-- Reflexivity up to DELTA: `double` must be unfolded. No script-layer delta exists. -/
def double (n : Nat) : Nat := n + n

theorem delta (n : Nat) : double n = n + n := by rfl

/-- Reflexivity up to BETA. -/
theorem beta (n : Nat) : (fun k => k + 1) n = n + 1 := by rfl

/-- A non-`Eq` reflexive relation. In Mathlib `LE.le` carries `@[refl]` and this is written
    `by rfl`; bare Lean 4.14 has no such attribute on it (`tactic 'rfl' failed, no @[refl]
    lemma registered for relation LE.le`), so the live code is the core equivalent. The LOGOS
    target is the `le_refl` AXIOM, not the reflexivity branch of `simp`. -/
theorem leRefl (n : Nat) : n ≤ n := Nat.le_refl n

/-- `Eq.refl` written as a term rather than a tactic; the elaborated `Expr` is the same. -/
theorem asTerm (n : Nat) : n = n := Eq.refl n

/-- A `rfl` that Lean discharges through a structure's eta rule. -/
structure Wrap where
  val : Nat

theorem structEta (w : Wrap) : Wrap.mk w.val = w := by rfl

end T08_005
