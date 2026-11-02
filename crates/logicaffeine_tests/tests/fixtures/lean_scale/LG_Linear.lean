/-!
Lane LG — minimal originals for the W06 `Nat.Linear` timeout group. The roots are Lean's own
`Init` theorems, exported with their dependency closure and nothing else: every conformance-corpus root
whose proof normalizes linear arithmetic over `Nat` depends on them. The primary kernel stalled
more than 300 s in `Poly.of_denote_eq_cancelAux` (registration) and in `Expr.denote_toPoly`
(`body.infer`); `Poly.denote_sort_go` is the group's third named member. Lean's kernel decides each
of their conversions by lazy δ-reduction, never by normalizing both sides.
-/
