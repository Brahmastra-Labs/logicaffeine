/-!
Lane LF2 · nesting through a carried POSITIVE function field, as an original Lean witness.

`CFn`'s `cons` carries `Nat → α` beside the element. Nesting `TFn` through `CFn TFn` makes that
field `Nat → TFn` in Lean's auxiliary type: a strictly positive, reflexive occurrence, which
Lean 4.14.0 accepts. Its recursor recurses through the function field with a pointwise
hypothesis `(a : Nat) → motive_1 (f a)`, and structural recursion compiles through `brecOn`.
Every `#guard_msgs` below makes Lean itself check the printed form it pins — the recursors,
`brecOn`, the compiled `swap`, and the refusals of the neighbours — so this file compiles only
while Lean still decides each of them exactly so.
-/
set_option autoImplicit false
namespace LF_NF

inductive CFn (α : Type) where
  | nil : CFn α
  | cons : α → (Nat → α) → CFn α → CFn α

inductive TFn where
  | node : CFn TFn → TFn

def leaf : TFn := .node .nil
def fan : TFn := .node (.cons leaf (fun _ => leaf) .nil)
/-- The head, the function field and the tail each lead to a different tree, and the function
field is not constant: `f 0 = leaf`, `f 7 = fan`. -/
def mixed : TFn :=
  .node (.cons leaf (fun n => match n with | 0 => leaf | _ + 1 => fan) (.cons fan (fun _ => leaf) .nil))
/-- `mixed` with each head exchanged for the function field's value at 7, and back. -/
def swapped : TFn := .node (.cons fan (fun _ => leaf) (.cons leaf (fun _ => fan) .nil))

/-- The recursor itself. Its `cons` case moves the function field's hypothesis at 7 into the head
and the head's into the function field, so a wrong, dropped or swapped hypothesis in either rule,
or a pointwise hypothesis taken at the wrong point, changes `mirror mixed`. -/
noncomputable def mirror : TFn → TFn :=
  @TFn.rec (fun _ => TFn) (fun _ => CFn TFn) (fun _ ih => .node ih) .nil
    (fun _ _ _ ihHead ihFn ihRest => .cons (ihFn 7) (fun _ => ihHead) ihRest)

theorem mirror_mixed : mirror mixed = swapped := rfl

/-! The same by structural recursion, which Lean compiles through `TFn.brecOn`. -/

mutual
def TFn.swap : TFn → TFn
  | .node cs => .node (CFn.swap cs)
  termination_by structural t => t
def CFn.swap : CFn TFn → CFn TFn
  | .nil => .nil
  | .cons t f rest => .cons (TFn.swap (f 7)) (fun _ => TFn.swap t) (CFn.swap rest)
  termination_by structural cs => cs
end

theorem swap_mixed : TFn.swap mixed = swapped := rfl

/-! Computing a number through the same recursor and through `brecOn`. -/

noncomputable def code : TFn → Nat :=
  @TFn.rec (fun _ => Nat) (fun _ => Nat) (fun _ ih => 2 * ih + 1) 0
    (fun _ _ _ ihHead ihFn ihRest => ihHead + 3 * ihFn 7 + 5 * ihRest + 1)

theorem code_mixed : code mixed = 221 := rfl

mutual
def TFn.size : TFn → Nat
  | .node cs => CFn.size cs + 1
  termination_by structural t => t
def CFn.size : CFn TFn → Nat
  | .nil => 0
  | .cons t f rest => TFn.size t + TFn.size (f 7) + CFn.size rest
  termination_by structural cs => cs
end

theorem size_mixed : TFn.size mixed = 9 := rfl

/-! The same pointwise hypothesis without nesting, in a reflexive family: `flip` computes
through `W.brecOn` exactly as `TFn.swap` computes through `TFn.brecOn`. -/

inductive W where
  | leaf : W
  | sup : (Nat → W) → W

def W.flip : W → W
  | .leaf => .leaf
  | .sup f => .sup (fun _ => (f 7).flip)

def w2 : W := .sup (fun n => match n with | 0 => .leaf | _ + 1 => .sup (fun _ => .leaf))

theorem w_flip : W.flip w2 = .sup (fun _ => .sup (fun _ => .leaf)) := rfl

/-! Lean nests pointwise in every position a nesting compiler has to mirror: a carried field whose
codomain is itself a nesting, a curried carried field, and the inductive's own field. -/

inductive CFnList (α : Type) where
  | nil : CFnList α
  | cons : α → (Nat → List α) → CFnList α → CFnList α

inductive TFnList where
  | node : CFnList TFnList → TFnList

inductive CCurry (α : Type) where
  | nil : CCurry α
  | cons : (Nat → Bool → α) → CCurry α → CCurry α

inductive TCurry where
  | node : CCurry TCurry → TCurry

inductive GTree where
  | node : (Nat → List GTree) → GTree

/--
info: @TFn.rec : {motive_1 : TFn → Sort u_1} →
  {motive_2 : CFn TFn → Sort u_1} →
    ((a : CFn TFn) → motive_2 a → motive_1 (TFn.node a)) →
      motive_2 CFn.nil →
        ((a : TFn) →
            (a_1 : Nat → TFn) →
              (a_2 : CFn TFn) →
                motive_1 a → ((a : Nat) → motive_1 (a_1 a)) → motive_2 a_2 → motive_2 (CFn.cons a a_1 a_2)) →
          (t : TFn) → motive_1 t
-/
#guard_msgs in
#check @TFn.rec

/--
info: @TFn.rec_1 : {motive_1 : TFn → Sort u_1} →
  {motive_2 : CFn TFn → Sort u_1} →
    ((a : CFn TFn) → motive_2 a → motive_1 (TFn.node a)) →
      motive_2 CFn.nil →
        ((a : TFn) →
            (a_1 : Nat → TFn) →
              (a_2 : CFn TFn) →
                motive_1 a → ((a : Nat) → motive_1 (a_1 a)) → motive_2 a_2 → motive_2 (CFn.cons a a_1 a_2)) →
          (t : CFn TFn) → motive_2 t
-/
#guard_msgs in
#check @TFn.rec_1

/--
info: @TFn.brecOn : {motive_1 : TFn → Type u_1} →
  {motive_2 : CFn TFn → Type u_1} →
    (t : TFn) → ((t : TFn) → t.below → motive_1 t) → ((t : CFn TFn) → TFn.below_1 t → motive_2 t) → motive_1 t
-/
#guard_msgs in
#check @TFn.brecOn

/--
info: def LF_NF.TFn.swap : TFn → TFn :=
fun x =>
  x.brecOn
    (fun x f =>
      (match (motive := (x : TFn) → x.below → TFn) x with
        | TFn.node cs => fun x => TFn.node x.1)
        f)
    fun x f =>
    (match (motive := (x : CFn TFn) → TFn.below_1 x → CFn TFn) x with
      | CFn.nil => fun x => CFn.nil
      | CFn.cons t f rest => fun x => CFn.cons (x.2.1.1 7) (fun x_1 => x.1.1) x.2.2.1)
      f
-/
#guard_msgs in
#print TFn.swap

/-! The neighbours. A carried field with the element in a DOMAIN is negative. -/

inductive CNeg (α : Type) where
  | nil : CNeg α
  | cons : α → (α → Nat) → CNeg α → CNeg α

/--
error: (kernel) arg #2 of '_nested.LF_NF.CNeg_1.cons' has a non positive occurrence of the datatypes being declared
-/
#guard_msgs in
inductive TNeg where
  | node : CNeg TNeg → TNeg

/-! With the element in the domain of a domain, it is positive but not STRICTLY positive. -/

inductive CDbl (α : Type) where
  | nil : CDbl α
  | cons : α → ((α → Nat) → α) → CDbl α → CDbl α

/--
error: (kernel) arg #2 of '_nested.LF_NF.CDbl_1.cons' has a non positive occurrence of the datatypes being declared
-/
#guard_msgs in
inductive TDbl where
  | node : CDbl TDbl → TDbl

/-! The inductive's own field with the nesting in a domain is negative. -/

/--
error: (kernel) arg #1 of 'LF_NF.GNeg.node' has a non positive occurrence of the datatypes being declared
-/
#guard_msgs in
inductive GNeg where
  | node : (List GNeg → Nat) → GNeg

/-! A carried field naming a type that is not declared never elaborates. -/

/-- error: unknown identifier 'NotYetDeclared' -/
#guard_msgs in
inductive CPend (α : Type) where
  | nil : CPend α
  | cons : α → NotYetDeclared → CPend α → CPend α

end LF_NF
