/-!
@case      T04-004
@tier      4 · Recursion and pattern matching
@title     `termination_by` — a measure-based termination argument has no LOGOS carrier
@lean      termination_by, well-founded recursion compiled by Lean's `WF` elaborator
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/term.rs:493-525 (`Term::Fix` and `Term::MutualFix` carry ONLY a name and
           a body: there is no measure, no relation, no well-foundedness proof field);
           kernel/src/type_checker.rs:336 and :363 (every `Fix`/`MutualFix` is gated on
           `termination::check_termination{,_mutual}` — there is no alternative admission path);
           kernel/src/termination.rs:1-14 and :378-400 (the guard is the purely SYNTACTIC
           Giménez structural condition: the recursive argument must be a variable bound by
           matching on the structural parameter, or an application headed by one);
           kernel/src/termination.rs:378-395 (`verify_structural_arg_smaller` — anything that
           is not headed by a `smaller_than` variable is a `TerminationViolation`)
@diag      E_NO_MEASURE_TERMINATION — must name the function, quote its `termination_by`
           measure, and state that LOGOS admits recursion only through the syntactic structural
           guard. Must NOT emit a `Fix`.
@guard     The tempting wrong move is to **drop the `termination_by` clause and emit the `Fix`
           anyway**, reasoning that "LOGOS will re-check termination, so if it is wrong the
           kernel catches it". It does catch it — with a `TerminationViolation` — which means
           the translator has converted a *perfectly good Lean theorem* into a hard error at
           some later, unrelated point in the build, with no pointer back to the source. Worse
           is the variant that emits the `Fix` into a context where the guard accidentally
           passes: `extract_structural_param` falls back to "the first inductive-typed binder"
           (termination.rs:145-149), so a function whose real decreasing measure is `n - m` but
           whose first binder happens to be matched on can be admitted with a *different*
           recursion than the source had.
@guard     Catches "translate the measure into a fuel parameter". Adding a `Nat` fuel argument
           and recursing on it produces a LOGOS `Fix` that passes the guard and is a DIFFERENT
           function: it returns a junk value when the fuel runs out. Every Lean theorem about
           the original is then false about the translation, and — because LOGOS is cumulative
           and the junk value has the right type — nothing in the kernel notices.
@guard     Catches "route it through `Acc`". `Acc`/`Acc_intro`/`Acc_rec`/`WellFounded` DO exist
           in the LOGOS prelude (prelude.rs:1354-1418), which makes this look available. But
           `WellFounded` is only the abbreviation `Π(x:A). Acc A R x` (prelude.rs:1407-1418) —
           there is no `WellFounded.fix`, and the recursion still has to be written as a `Fix`
           that the syntactic guard must accept. See T04-015.
@guard     Catches a translator that silently succeeds because the Lean function *happens* to
           also be structurally recursive and `termination_by` was only an optimisation hint.
           The verdict is still `reject`: the translator cannot know that in general, and
           emitting output for some `termination_by` functions and not others makes the
           conformance report unreliable. Recognising the structural subcase is allowed only
           as a separate, explicitly-recorded path.
@oracle    Translator exits non-zero with E_NO_MEASURE_TERMINATION for each of the four
           definitions below and writes no declaration into the LOGOS context.
-/

namespace T04_004

/-- Measure on a single argument. `n - 1` is not a variable bound by matching on `n`, so the
    Giménez guard has nothing to mark smaller. -/
def countdown (n : Nat) : Nat :=
  if n = 0 then 0 else countdown (n - 1) + 1
termination_by n

/-- The classic: Euclid's algorithm. The decreasing argument is `n % m`, which is not a
    structural subterm of anything in scope. -/
def gcd' (m n : Nat) : Nat :=
  if _h : m = 0 then n else gcd' (n % m) m
termination_by m
decreasing_by exact Nat.mod_lt _ (Nat.pos_of_ne_zero _h)

/-- A LEXICOGRAPHIC measure — two arguments, neither of which decreases alone. LOGOS's guard
    tracks exactly one `struct_param` per fixpoint (termination.rs:33-35), so there is no
    representation for this at all. -/
def ack : Nat → Nat → Nat
  | 0, n => n + 1
  | m + 1, 0 => ack m 1
  | m + 1, n + 1 => ack m (ack (m + 1) n)
termination_by m n => (m, n)

/-- A measure computed by a *function*, not read off a binder. -/
def collatzish (n : Nat) : Nat :=
  if _h : n ≤ 1 then 0
  else collatzish (n / 2) + 1
termination_by n
decreasing_by omega

end T04_004
