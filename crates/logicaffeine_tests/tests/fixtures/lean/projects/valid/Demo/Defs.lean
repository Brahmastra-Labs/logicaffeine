namespace Demo

/-- Doubling, by structural recursion. -/
def double : Nat → Nat
  | 0 => 0
  | n + 1 => double n + 2

end Demo
