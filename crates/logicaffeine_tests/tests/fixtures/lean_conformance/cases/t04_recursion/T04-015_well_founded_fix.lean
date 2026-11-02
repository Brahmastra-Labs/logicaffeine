/-!
@case      T04-015
@tier      4 · Recursion and pattern matching
@title     `WellFounded.fix` / `Acc.rec` recursion — `Acc` is a type, recursion *through* it is
           not a LOGOS construct
@lean      WellFounded.fix, WellFounded.fixF, Acc.rec, Acc.ndrec, WellFoundedRecursion
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/prelude.rs:1354-1418 (`register_acc`: `Acc` is registered by
           `add_indexed_inductive("Acc", acc_ty, 2)` at :1379, `Acc_intro` at :1398, and
           `Acc_rec` is the AUTO-DERIVED eliminator `add_definition`ed at :1403-1405;
           `WellFounded` at :1407-1418 is a plain **abbreviation**
           `λA. λR. Π(x:A). Acc A R x`, not a recursion combinator);
           there is **no `WellFounded_fix`, no `WellFounded_fixF`, no `Acc_ndrec`** anywhere in
           the prelude — `Acc_rec` is the only elimination form;
           kernel/src/recursor.rs:60-72 (`derive_recursor` hard-codes the motive target;
           `Acc` is a one-constructor `Prop` whose only field is propositional, so
           `is_subsingleton_prop` (type_checker.rs:1688-1716) holds and the target is
           `Sort(Type 0)` — `Acc_rec` therefore **cannot eliminate into `Type 1+`**);
           kernel/src/type_checker.rs:328-339 (every `Fix` is gated on `check_termination`);
           kernel/src/termination.rs:378-400 (`verify_structural_arg_smaller` accepts a
           `smaller_than` variable or an APPLICATION HEADED BY ONE — the applied-smaller rule
           that makes recursion over an `Acc` proof terminate *when written as a `Fix` whose
           guarding `match` is on the `Acc` proof itself*)
@diag      E_NO_WELL_FOUNDED_RECURSION — must name the function, name the combinator it used
           (`WellFounded.fix` / `Acc.rec`), and state that LOGOS provides `Acc` as an inductive
           and `Acc_rec` as a derived eliminator but has no fixpoint combinator over a
           well-foundedness proof. Must NOT emit a `Fix` or an `Acc_rec` application.
@guard     The distinction this case exists to make crisp: **`Acc` as a TYPE is fine; recursion
           THROUGH it is not.** `Acc Nat Nat.lt n` translates — it is an ordinary indexed
           inductive in the prelude. A *proof* of `WellFounded Nat.lt` translates — it is a
           term of the abbreviation's type. What does not translate is
           `WellFounded.fix wf F x`, because there is no `WellFounded_fix` global to apply and
           no way to add one: its body would itself have to be a well-founded recursion.
@guard     Catches the "just call `Acc_rec`" move. `Acc_rec` exists, so
           `Global("Acc_rec")` resolves and an application of it typechecks. But Lean's
           `WellFounded.fix F x` is `Acc.rec` at the motive `fun x _ => C x` with a step
           function that recurses through the accessibility *hypothesis*, and Lean's `Acc.rec`
           is universe-polymorphic in that motive. The LOGOS-derived `Acc_rec`'s motive target
           is pinned to `Sort(Type 0)` (recursor.rs:64-71), so a Lean well-founded definition
           producing anything in `Type 1+` has no counterpart — while one producing `Type 0`
           data typechecks and is a *different, delta-transparent* term that unfolds during
           `normalize` and burns the 10000-step fuel (reduction.rs:52-69) on every use.
@guard     Catches the "reconstruct it as a `Fix` over the `Acc` proof" move. This is the one
           encoding that can be made to pass the guard — `termination.rs:378-400` explicitly
           supports `rec (h y hr)` where `h` is `Acc_intro`'s match-bound functional field —
           but it is a *hand-written LOGOS proof*, not a translation: the translator would
           have to synthesise the accessibility argument `hr : R y x` at every call site, and
           that argument is exactly the `decreasing_by` obligation LOGOS has no carrier for
           (T04-005). Emitting the `Fix` without it leaves a free variable or a `Hole` in the
           recursive call.
@guard     Catches silently dropping the well-foundedness proof and emitting a structural
           `Fix` on whatever binder happens to be inductive.
           `extract_structural_param` falls back to "the first inductive-typed binder"
           (termination.rs:145-149), so `wfDiv` below — whose first binder is a `Nat` — would be
           checked as a structural recursion on that `Nat` and rejected with a confusing
           `TerminationViolation` pointing at the wrong argument.
@guard     Catches treating `WellFounded` as an inductive. It is an `add_definition`
           (prelude.rs:1407-1418), so it is delta-TRANSPARENT: `Match`ing on a `WellFounded`
           value fails with `NotAnInductive` after normalisation exposes the `Π`.
@oracle    Translator exits non-zero with E_NO_WELL_FOUNDED_RECURSION for every definition
           below that *recurses*, and writes no declaration for them. It must NOT reject
           `accIsAType` or `ltWf`, which merely mention `Acc`/`WellFounded` as types.
-/

namespace T04_015

/-- `Acc` used purely as a TYPE — this must translate. -/
def accIsAType (n : Nat) : Prop := Acc Nat.lt n

/-- A `WellFounded` proof as a value — also fine: `WellFounded` is the LOGOS abbreviation
    `Π(x:A). Acc A R x`. -/
def ltWf : WellFounded Nat.lt := Nat.lt_wfRel.wf

/-- `WellFounded.fix` — no LOGOS counterpart. -/
noncomputable def wfSum (n : Nat) : Nat :=
  WellFounded.fix ltWf
    (fun x ih => match x, ih with
      | 0, _ => 0
      | m + 1, ih => (m + 1) + ih m (Nat.lt_succ_self m))
    n

/-- `Acc.rec` applied directly, with a `Type 0` motive. Tempting because LOGOS *has* an
    `Acc_rec`, but the LOGOS one is a derived definition with a pinned motive target. -/
noncomputable def accDepth {α : Type} {r : α → α → Prop} {x : α} (h : Acc r x) : Nat :=
  Acc.rec (motive := fun _ _ => Nat) (fun _ _ _ => 0) h

/-- `Acc.rec` whose motive genuinely recurses — the shape `WellFounded.fix` expands to. -/
noncomputable def accCount {x : Nat} (h : Acc Nat.lt x) : Nat :=
  Acc.rec (motive := fun _ _ => Nat)
    (fun y _ ih => match y with
      | 0 => 0
      | m + 1 => ih m (Nat.lt_succ_self m) + 1)
    h

/-- Well-founded division, the canonical motivating example. Lean compiles it through
    `WellFounded.fix`; LOGOS's structural guard has no measure to work with. -/
noncomputable def wfDiv (n m : Nat) : Nat :=
  if _h : 0 < m ∧ m ≤ n then wfDiv (n - m) m + 1 else 0
termination_by n
decreasing_by omega

end T04_015
