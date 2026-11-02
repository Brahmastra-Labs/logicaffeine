/-!
Lane LG — minimal originals for the W06 Omega timeout group. The roots are Lean's own `Init`
theorems, exported with their dependency closure and nothing else: every conformance-corpus root whose
proof uses `omega` depends on them, and the primary kernel stalled more than 300 s inside them
(`dot_mod_gcd_left` in `body.infer`, `gcd_cons_div_left` in registration, `normalize_sat` in
`body.infer`). Lean's kernel decides each of their conversions by lazy δ-reduction, never by
normalizing both sides.
-/
