/-!
Lane LF — a proof both Lean's kernel and LOGOS's primary kernel check, whose independent re-check
stops for want of a resource.

`parity (k+1)` is `Bool.rec true false (parity k)`: the step eliminates on the recursive result, so
deciding `parity n` demands `n` nested major premises. Each theorem passes `rfl` through `@id` at the
statement, so the application's argument is converted against `parity n = true` inside the proof
term, where each kernel checks it. LOGOS's independent kernel bounds the nesting of its demands
(512 frames): at `n = 800` its check is cut short, which is a resource limit, never a refutation.
Lean's kernel accepts every theorem here.
-/
namespace LF_RB

/-- `true` at even arguments, by the recursor applied directly. -/
noncomputable def parity (n : Nat) : Bool :=
  Nat.rec (motive := fun _ => Bool) true (fun _ ih => Bool.rec true false ih) n

/-- Within the independent kernel's nesting bound. -/
theorem parityShallow : parity 100 = true := @id (parity 100 = true) rfl

-- `maxRecDepth` lifts only the elaborator's own bound; the kernel check is unchanged.
set_option maxRecDepth 100000 in
/-- Beyond it. -/
theorem parityDeep : parity 800 = true := @id (parity 800 = true) rfl

end LF_RB
