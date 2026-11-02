/-!
@case      T04-020
@tier      4 · Recursion and pattern matching
@title     `dite` — `if h : c then … else …` binds the decision's proof in each branch
@lean      dite, `if h : c then a h else b h`, Decidable.byCases
@expect    translate
@target    Match { discriminant: <the resolved `Decidable p` instance>,
                   motive: <λ_ : Decidable p. C>,
                   cases: [ λhp  : p.     <then branch, with `h := hp`>,
                            λhnp : Not p. <else branch, with `h := hnp`> ] }
           — in `Decidable`'s registration order `isTrue`, `isFalse` (prelude.rs:783,786).
@evidence  kernel/src/prelude.rs:781-786 (`Decidable` is an indexed inductive with ONE
           parameter; `isTrue : Π(p:Prop). p → Decidable p` carries the PROOF of `p`, and
           `isFalse : Π(p:Prop). Not p → Decidable p` carries the refutation — these fields
           are precisely the `h` that `dite` binds);
           kernel/src/type_checker.rs:612-678 (`infer_indexed_match`) and :685-740
           (`compute_indexed_case_type`: each case's expected type is the constructor's
           remaining Π-telescope — here `Π(hp : p). motive …` and `Π(hnp : Not p). motive …` —
           so the branch bodies must be λ-abstractions, not bare terms);
           kernel/src/prelude.rs:820-910 (`of_decide_eq_true`, PROVEN from `Decidable_rec`,
           is the only bridge back from a `Bool` to a proof — the long way round that `dite`
           avoids by matching on the instance directly);
           kernel/src/type_checker.rs:275-288 (large elimination: `Decidable` is `Type 0`-valued
           (prelude.rs:781), NOT a `Prop`, so eliminating it into `Type` is unrestricted —
           which is exactly why `dite` can return data);
           kernel/src/context.rs:184 (`add_instance`: flat `Vec`, first-match-wins)
@guard     Catches erasing the proof binder. A translator that lowers `dite` the same way it
           lowers `ite` (T04-019) emits case bodies as bare terms instead of
           `λhp. …` / `λhnp. …`. `compute_indexed_case_type` then reports an arity mismatch —
           unless the branch body happens to already be a function, in which case the term
           typechecks and the binder `h` in the source body becomes a FREE `Term::Var`.
           Because LOGOS `Var`s are NAMED (term.rs:450), that free `h` silently resolves
           against any enclosing binder called `h`, and `h` is the most common hypothesis name
           in existence. The result is a well-typed declaration whose proof obligation was
           discharged by an unrelated hypothesis.
@guard     Catches routing through `decide`. `decide p inst : Bool` (prelude.rs:796-815)
           throws away both `hp` and `hnp`; a translator that matches on the `Bool` then has
           no proof to give the `then` branch and must recover one via `of_decide_eq_true`
           (prelude.rs:820-910) — which needs an `Eq Bool (decide p inst) true`, i.e. exactly
           the `match h : e with` equation hypothesis that LOGOS cannot express (T04-012). So
           the `Bool` route is a dead end, not merely a slower one.
@guard     Catches branch-order inversion. `Decidable` registers `isTrue` first
           (prelude.rs:783), matching Lean's then-first surface order — so a source-order copy
           is accidentally right here and wrong the moment the translator registers the
           constructors in a different order or meets `Decidable.byCases`, whose argument order
           is the opposite. The two cases have *different* types (`p → C` vs `¬p → C`), so a
           swap usually fails loudly — but when `p` is itself a negation (`¬q`) the two become
           confusable and the swap typechecks.
@guard     Catches dropping the `Not p` case's argument type. `Not p` must be emitted as
           `Π(_:p). False`, not as an opaque global: there is no `Not` primitive, and
           `isFalse`'s field type (prelude.rs:786) is what the case must be checked against.
@guard     Catches assuming the instance exists. The prelude has exactly two `Decidable`
           instances, `decEqBool` and `decEqNat` (prelude.rs:923-1021, :1023-1160).
           `safeDiv` below needs a decision for `m = 0` on `Nat` (available via `decEqNat`) but
           `firstBig` needs one for `<` (not available). Resolution is first-match-wins over a
           flat `Vec` (context.rs:184) with no coherence check, so registering a competing
           instance silently changes which proof term ends up in the branch — and since the
           proof is a `Prop` inhabitant, definitional proof irrelevance
           (type_checker.rs:1547-1567) will often hide the difference.
@oracle    The emitted term typechecks; both case bodies are `Term::Lambda`s whose binder types
           are the translations of `p` and `Not p`; the emitted term contains no free
           `Term::Var` for the source's `h`; `normalize` agrees with Lean on both truth values;
           and the output contains no `Global("dite")`.
-/

namespace T04_020

/-- The canonical `dite`: the refutation `h : ¬(m = 0)` bound by the `isFalse` branch is what
    discharges the returned subtype's obligation. -/
def safeDiv (n m : Nat) : { q : Nat // m = 0 → q = 0 } :=
  if h : m = 0 then ⟨0, fun _ => rfl⟩ else ⟨n / m, fun hc => absurd hc h⟩

/-- `h` is genuinely USED in the branch body — this is what distinguishes `dite` from `ite`. -/
def predOf (n : Nat) (h : n ≠ 0) : Nat :=
  if hz : n = 0 then absurd hz h else n - 1

/-- Both branches use their hypothesis, so both case bodies must be real λ-abstractions. -/
def describe (n : Nat) : { b : Bool // (b = true → n = 0) ∧ (b = false → n ≠ 0) } :=
  if h : n = 0 then ⟨true, fun _ => h, fun hc => absurd hc (by simp)⟩
  else ⟨false, fun hc => absurd hc (by simp), fun _ => h⟩

/-- `dite` producing DATA whose type depends on the branch — legal because LOGOS's `Decidable`
    is `Type 0`-valued, so there is no large-elimination restriction. -/
def Carrier (n : Nat) : Type := if n = 0 then Unit else Nat

/-- A `Decidable` for `<`, which the LOGOS prelude does not supply. -/
def firstBig : List Nat → Option { z : Nat // 10 < z }
  | [] => none
  | x :: t => if h : 10 < x then some ⟨x, h⟩ else firstBig t

/-- `dite` under recursion: the recursive call lives in the `isFalse` case, so the guard must
    see it through the λ that binds the refutation. -/
def dropZeros : List Nat → List Nat
  | [] => []
  | x :: t => if _h : x = 0 then dropZeros t else x :: dropZeros t

/-- `Decidable.byCases` — the same elimination with the branch arguments in the opposite
    order, which is what makes the registration order load-bearing. -/
def byCasesEx (n : Nat) : Nat :=
  Decidable.byCases (p := n = 0) (fun _ => 1) (fun _ => 0)

/-- The instance supplied explicitly, pinning that the `Match`'s discriminant is the
    INSTANCE and not a `Bool`. -/
def diteExplicit (p : Prop) [inst : Decidable p] (f : p → Nat) (g : ¬p → Nat) : Nat :=
  @dite Nat p inst f g

example : (safeDiv 10 2).1 = 5 := rfl
example : (safeDiv 10 0).1 = 0 := rfl
example : predOf 5 (by decide) = 4 := rfl
example : (describe 0).1 = true := rfl
example : dropZeros [0, 1, 0, 2] = [1, 2] := rfl
example : byCasesEx 0 = 1 := by decide

end T04_020
