/-
Reference terms for validating Lean's primitive `Nat` operations, elaborated by the pinned Lean
toolchain and written in the `lean4export` NDJSON record format.

The kernel accelerates `Nat.div`, `Nat.mod`, `Nat.gcd` and `Nat.bitwise` on literals only after it
has proven that the environment's definitions satisfy their defining equations. Those proofs are
stated with the terms below (the scheme of lean4lean's `Primitive.lean`). Regenerate with
`scripts/lean/arena-crown/gen-primitive-refs.sh`.
-/
import Lean
open Lean Elab Term Meta

structure Out where
  names : Std.HashMap Name Nat := {}
  levels : Std.HashMap Level Nat := {}
  exprs : Std.HashMap Expr Nat := {}
  lines : Array String := #[]

abbrev W := StateT Out TermElabM

def jstr (s : String) : String := Json.compress (Json.str s)

partial def name (n : Name) : W Nat := do
  if n.isAnonymous then return 0
  if let some i := (← get).names[n]? then return i
  let body ← match n with
    | .str p s => do let pi ← name p; pure s!"\"str\":\{\"pre\":{pi},\"str\":{jstr s}}"
    | .num p k => do let pi ← name p; pure s!"\"num\":\{\"i\":{k},\"pre\":{pi}}"
    | .anonymous => unreachable!
  let i := (← get).names.size + 1
  modify fun o => { o with names := o.names.insert n i, lines := o.lines.push s!"\{{body},\"in\":{i}}" }
  return i

partial def level (l : Level) : W Nat := do
  if l.isZero then return 0
  if let some i := (← get).levels[l]? then return i
  let body ← match l with
    | .succ a => do pure s!"\"succ\":{← level a}"
    | .max a b => do pure s!"\"max\":[{← level a},{← level b}]"
    | .imax a b => do pure s!"\"imax\":[{← level a},{← level b}]"
    | .param n => do pure s!"\"param\":{← name n}"
    | _ => throwError "unexpected level"
  let i := (← get).levels.size + 1
  modify fun o => { o with levels := o.levels.insert l i, lines := o.lines.push s!"\{{body},\"il\":{i}}" }
  return i

def binfo : BinderInfo → String
  | .default => "default" | .implicit => "implicit"
  | .strictImplicit => "strictImplicit" | .instImplicit => "instImplicit"

partial def expr (e : Expr) : W Nat := do
  if let some i := (← get).exprs[e]? then return i
  let body ← match e with
    | .bvar k => pure s!"\"bvar\":{k}"
    | .sort l => do pure s!"\"sort\":{← level l}"
    | .const n us => do
        let us ← us.mapM level
        pure s!"\"const\":\{\"name\":{← name n},\"us\":{us}}"
    | .app f a => do pure s!"\"app\":\{\"arg\":{← expr a},\"fn\":{← expr f}}"
    | .lam n t b bi => do
        pure s!"\"lam\":\{\"binderInfo\":\"{binfo bi}\",\"body\":{← expr b},\"name\":{← name n},\"type\":{← expr t}}"
    | .forallE n t b bi => do
        pure s!"\"forallE\":\{\"binderInfo\":\"{binfo bi}\",\"body\":{← expr b},\"name\":{← name n},\"type\":{← expr t}}"
    | .letE n t v b nd => do
        pure s!"\"letE\":\{\"body\":{← expr b},\"name\":{← name n},\"nondep\":{nd},\"type\":{← expr t},\"value\":{← expr v}}"
    | .lit (.natVal k) => pure s!"\"natVal\":\"{k}\""
    | .lit (.strVal s) => pure s!"\"strVal\":{jstr s}"
    | .proj s k x => do pure s!"\"proj\":\{\"idx\":{k},\"struct\":{← expr x},\"typeName\":{← name s}}"
    | .mdata _ x => return ← expr x
    | _ => throwError "unexpected expression"
  let i := (← get).exprs.size
  modify fun o => { o with exprs := o.exprs.insert e i, lines := o.lines.push s!"\{{body},\"ie\":{i}}" }
  return i

def refs : List (String × String) := [
  ("refl1.type", "fun p b => ∀ {q : Prop}, ((b = true → p) → (¬b = true → ¬p) → q) → q"),
  ("refl1.ofTrue", "fun p (H : ∀ {q : Prop}, ((true = true → p) → (¬true = true → ¬p) → q) → q) => H fun h _ => h rfl"),
  ("refl1.ofFalse", "fun p (H : ∀ {q : Prop}, ((false = true → p) → (¬false = true → ¬p) → q) → q) => H fun _ h => h Bool.noConfusion"),
  ("refl1.toDec", "fun p b (H : ∀ {q : Prop}, ((b = true → p) → (¬b = true → ¬p) → q) → q) => if h : b = true then isTrue (H fun h' _ => h' h) else isFalse (H fun _ h' => h' h)"),
  ("refl2.type", "fun p b => ∀ {q : Prop}, ((b = true → p) → (b = false → ¬p) → q) → q"),
  ("refl2.ofTrue", "fun p (H : ∀ {q : Prop}, ((true = true → p) → (true = false → ¬p) → q) → q) => H fun h _ => h rfl"),
  ("refl2.ofFalse", "fun p (H : ∀ {q : Prop}, ((false = true → p) → (false = false → ¬p) → q) → q) => H fun _ h => h rfl"),
  ("refl2.toDec", "fun p b (H : ∀ {q : Prop}, ((b = true → p) → (b = false → ¬p) → q) → q) => b.casesOn (motive := fun b' => b = b' → Decidable p) (fun h => isFalse (H fun _ h' => h' h)) (fun h => isTrue (H fun h' _ => h' h)) rfl"),
  ("natLE.prop", "@LE.le Nat _"),
  ("natLE.dec", "Nat.decLe"),
  ("natLE.proof", "fun n m {q : Prop} (H : _ → _ → q) => H (@Nat.le_of_ble_eq_true n m) (@Nat.not_le_of_not_ble_eq_true n m)"),
  ("natEq.prop", "@Eq Nat"),
  ("natEq.dec", "Nat.decEq"),
  ("natEq.proof", "fun n m {q : Prop} (H : _ → _ → q) => H (@Nat.eq_of_beq_eq_true n m) (@Nat.ne_of_beq_eq_false n m)"),
  ("bool.prop", "fun x : Bool => x = true"),
  ("bool.dec", "fun x => Bool.decEq x true"),
  ("natLE.type", "Nat → Nat → Prop"),
  ("goType", "∀ n, Nat.succ Nat.zero ≤ n → ∀ fuel x : Nat, Nat.succ x ≤ fuel → Nat"),
  ("gcd.eq_def", "type_of% Nat.gcd.eq_def"),
  ("bitwise.eq_def", "type_of% Nat.bitwise.eq_def")
]

elab "#emit_primitive_refs" : command => Command.liftTermElabM do
  let mut o : Out := {}
  for (key, src) in refs do
    let stx ← match Parser.runParserCategory (← getEnv) `term src with
      | .ok s => pure s
      | .error e => throwError m!"{key}: {e}"
    let e ← elabTerm stx none
    synthesizeSyntheticMVarsNoPostponing
    let e ← instantiateMVars e
    if e.hasMVar then throwError m!"{key}: unassigned metavariables in {e}"
    let (i, o') ← (expr e).run o
    o := { o' with lines := o'.lines.push s!"\{\"ref\":\{\"key\":{jstr key},\"value\":{i}}}" }
  IO.println s!"\{\"meta\":\{\"exporter\":\{\"name\":\"logos-primitive-refs\",\"version\":\"1\"},\"format\":\{\"version\":\"3.1.0\"},\"lean\":\{\"version\":{jstr Lean.versionString}}}}"
  for l in o.lines do IO.println l

#emit_primitive_refs
