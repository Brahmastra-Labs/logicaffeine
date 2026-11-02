/-!
Lane LF · W03 — the uniform counterpart of the Lean Kernel Arena's `nested-nonuniform-param` input.
`E` takes a VALUE parameter `w : W` and nests itself, at that same `w`, through the container `L`.
Lean 4.14.0 accepts it and generates `E.rec` / `E.rec_1` with one parameter and two motives.
-/
namespace LF_W03V

inductive W where
  | mk : Bool → W

inductive L (α : Type) where
  | mk : L α

inductive E (w : W) where
  | mk : L (E w) → E w

def eFalse : E (.mk false) := .mk .mk

end LF_W03V
