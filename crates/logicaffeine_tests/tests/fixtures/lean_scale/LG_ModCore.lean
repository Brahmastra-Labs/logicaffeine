/-!
Lane LG — minimal originals for the W06 `Nat.modCore_eq_mod` timeout group. The roots are Lean's
own `Init` theorems, exported with their dependency closure and nothing else: `wfDiv` (T04-015)
stalled for 270 s in the registration of `Nat.modCore_eq_mod`, then in `Nat.gcd_zero_right`.
Lean's kernel decides each of their conversions by lazy δ-reduction, never by normalizing both
sides.
-/
