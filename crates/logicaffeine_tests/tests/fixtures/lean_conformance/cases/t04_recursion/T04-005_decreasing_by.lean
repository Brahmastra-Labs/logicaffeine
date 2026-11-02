/-!
@case      T04-005
@tier      4 · Recursion and pattern matching
@title     `decreasing_by` — the termination *proof* has no carrier and cannot be replayed
@lean      decreasing_by, decreasing_tactic, the `WF` elaborator's proof obligations
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/term.rs:493-509 (`Term::Fix { name, body }` has exactly two fields: there
           is nowhere to attach a proof term);
           kernel/src/type_checker.rs:328-339 (`Fix` inference runs `check_termination` and
           then a sanity `infer_type`; the termination argument is never a *term* the kernel
           checks, it is a decision procedure over syntax);
           kernel/src/termination.rs:378-400 (`verify_structural_arg_smaller` accepts exactly
           two shapes — a `smaller_than` variable, or an application headed by one — and takes
           no proof, no hypothesis, and no context of assumptions);
           kernel/src/termination.rs:19-43 (`GuardContext` holds only names and a
           `HashSet<String>` of smaller variables — there is no proof state at all)
@diag      E_NO_TERMINATION_PROOF_CARRIER — must name the function and state that LOGOS's
           termination check is a syntactic guard that accepts no proof obligation, so the
           `decreasing_by` script is unreplayable. Must NOT emit a `Fix`.
@guard     Catches the "just discharge it on the LOGOS side" move — translating the
           `decreasing_by` script into a LOGOS tactic script and attaching it somewhere. There
           is nowhere to attach it: the `Fix` rule never produces a goal. A translator that
           emits the `Fix` *plus* a separate LOGOS theorem `n % m < m` has produced a true
           theorem and an unrelated `Fix` that the guard will reject on its own terms — the
           theorem does not license the recursion.
@guard     Catches the subtler variant: emitting the `Fix` and a `Term::Lit`/axiom "termination
           certificate" as an extra argument. Because the guard only inspects the argument at
           the recorded structural POSITION (termination.rs:218-232), padding the call with an
           extra argument SHIFTS that position — a translator that appends a certificate
           argument can make the guard inspect the certificate instead of the real recursive
           argument. If the certificate happens to be a match-bound variable the guard passes
           and a non-terminating fixpoint is admitted. That inhabits every type.
@guard     Catches assuming `decreasing_by omega` means "LOGOS's `omega` can do it too". The
           LOGOS `omega` is a *proof script* tactic on the vernacular surfaces
           (LOGOS_FACTS, Surface A/C tactic lists); it is not part of the kernel's `Fix` rule
           and cannot be invoked from term elaboration.
@guard     Catches dropping a `decreasing_by` that used `simp_wf` or `Nat.sub_lt`: these make
           the *Lean* definition legal, and after they are dropped the surviving `Expr` still
           looks like an ordinary recursive definition. The translator must key the rejection
           off the presence of well-founded recursion in the Lean declaration
           (`WellFounded.fix`/`WF` in the compiled value), not off surface syntax.
@oracle    Translator exits non-zero with E_NO_TERMINATION_PROOF_CARRIER for each definition
           below and writes no declaration into the LOGOS context.
-/

namespace T04_005

/-- An explicit `decreasing_by` script. `n - 1 < n` needs the hypothesis `¬(n = 0)`, which
    only exists because the `if` is *dependent*. -/
def downFrom (n : Nat) : Nat :=
  if _h : n = 0 then 0 else downFrom (n - 1) + 1
termination_by n
decreasing_by omega

/-- A `decreasing_by` that appeals to a library lemma rather than arithmetic. -/
def gcdSteps (m n : Nat) : Nat :=
  if _h : m = 0 then 0 else gcdSteps (n % m) m + 1
termination_by m
decreasing_by exact Nat.mod_lt _ (Nat.pos_of_ne_zero _h)

/-- Two recursive calls, each with its OWN proof obligation — Lean generates one goal per
    call, so even the count of obligations is not recoverable from the LOGOS `Fix`. -/
def binCount (n : Nat) : Nat :=
  if _h : n ≤ 1 then n
  else binCount (n / 2) + binCount (n - 1)
termination_by n
decreasing_by
  · omega
  · omega

/-- The measure is on a *derived* quantity, so the obligation is not even about a binder. -/
def shrink (p : Nat × Nat) : Nat :=
  if _h : p.1 + p.2 = 0 then 0 else shrink (p.1 / 2, p.2 / 2) + 1
termination_by p.1 + p.2
decreasing_by omega

end T04_005
