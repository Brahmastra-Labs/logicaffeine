universe u

mutual
  inductive ArenaTree (α : Type u) : Type u where
    | leaf : α → ArenaTree α
    | branch : ArenaForest α → ArenaTree α
  inductive ArenaForest (α : Type u) : Type u where
    | nil : ArenaForest α
    | cons : ArenaTree α → ArenaForest α → ArenaForest α
end

def arenaSingleton {α : Type u} (a : α) : ArenaForest α :=
  ArenaForest.cons (ArenaTree.leaf a) ArenaForest.nil
