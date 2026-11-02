namespace Ext

/-- Propositional extensionality: equivalent propositions are equal. -/
theorem iffEq (p q : Prop) (h : p ↔ q) : p = q := propext h

end Ext
