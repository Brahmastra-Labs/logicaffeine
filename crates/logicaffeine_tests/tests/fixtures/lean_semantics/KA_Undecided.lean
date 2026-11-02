/-!
Lane KA — an original Lean 4.14.0 witness whose conversion is beyond LOGOS's reduction budget.

`step n b` flips `b` exactly `n` times, one `Nat` iota and one `Bool` iota per flip, with a
strict accumulator (no pending work). `sweep m b` runs `step i` on the accumulator for every
`i < m`, so `sweep 1500` is `Σ_{i<1500} i = 1124250` flips — even, hence the identity on `Bool`.
Every inner run has a distinct length, so no term is ever reduced twice, and every length stays a
compact literal (no `+`, so no unary addition inflates the term). The nesting keeps Lean's
recursion depth near 3000 frames; a flat `step 100000` already overflows Lean's stack.

`sweepOpen` states it OPEN, under a variable `b`: `true || b` is `true` by `Bool.or`'s first
equation, so Lean's kernel checks `rfl` by running every flip. LOGOS's closed-data evaluators
refuse an open term, so the conversion is left to reduction, whose budget is 2^20 whole-term steps:
fewer than the 2248500 iota steps the flips alone take. (`maxHeartbeats 0` lifts only the
elaborator's own `whnf` budget; the kernel check is unchanged.)
-/
namespace KA_UD
noncomputable def step (n : Nat) : Bool → Bool :=
  Nat.rec (motive := fun _ => Bool → Bool) (fun b => b)
    (fun _ ih b => Bool.rec (ih true) (ih false) b) n
noncomputable def sweep (m : Nat) : Bool → Bool :=
  Nat.rec (motive := fun _ => Bool → Bool) (fun b => b)
    (fun i ih b => Bool.rec (ih (step i false)) (ih (step i true)) b) m
set_option maxHeartbeats 0 in
theorem sweepOpen (b : Bool) : sweep 1500 (true || b) = true := rfl
end KA_UD
