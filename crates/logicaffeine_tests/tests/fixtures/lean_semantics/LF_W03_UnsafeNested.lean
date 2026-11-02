/-!
Lane LF · W03 — a Lean-positive nested family outside the safe kernel fragment.
Lean 4.14.0 accepts this `unsafe` nested family and a safe-looking user of it; LOGOS admits only
safe declarations, so the family must be declined at the family, never approximated.
-/
namespace LF_W03S

unsafe inductive Wild where
  | node : List Wild → Wild

unsafe def wildLeaf : Wild := .node []

end LF_W03S
