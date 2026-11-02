namespace Runtime

/-- An `unsafe` definition is a runtime artifact: Lean's kernel does not check it as a term of its
type, so no faithful LOGOS statement of it exists and the import must refuse it as unsupported. -/
unsafe def identity (n : Nat) : Nat := n

end Runtime
