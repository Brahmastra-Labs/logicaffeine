/-!
@case      T10-013
@tier      10 · Literals and primitive types
@title     `Option` / `Prod` / `Sum` / `Sigma` / `Subtype` / `Fin` — none is registered; each
           must be emitted, and each emission is a fresh, lemma-free type
@lean      Option, Prod (α × β), Sum (α ⊕ β), Sigma (Σ x, β x), Subtype ({x // p x}),
           Fin n, and their anonymous-constructor / projection notation
@expect    degrade
@target    One translator-emitted declaration per type. `Prod`, `Sigma` and `Subtype` are
           records and should go through `Context::add_structure` (which also gives eta);
           `Option` and `Sum` are plain inductives via `add_inductive` + `add_constructor`;
           `Fin` needs `Subtype` first, so it is emitted last.
@evidence  kernel/src/prelude.rs:18-51 (`StandardLibrary::register` — the complete registration
           list; none of these six appears); kernel/src/context.rs:271-369 (`add_structure`
           desugars a record into an inductive + `{name}_mk` + one DEFINITION per field whose
           body is a `Term::Match`, and registers the `StructInfo` that gates eta);
           kernel/src/context.rs:300 (`add_structure` HARD-CODES the sort to `Type 0`);
           kernel/src/type_checker.rs:1391-1417 (`try_structure_eta` — eta for registered
           structures) and :1473-1478 (it fires only inside `def_eq_normalized`, never in
           `normalize`); kernel/src/recursor.rs:64-71 (`derive_recursor` pins a non-`Prop`
           inductive's motive to `Sort(Type 0)`); kernel/src/context.rs:253, :419
           (`add_inductive` / `add_constructor` are UNCHECKED);
           kernel/src/prelude.rs:202-340 (every order axiom is about `Int`, so there is no
           `Nat` `<` for `Fin`'s bound to use)
@loss      NO_STRUCTURAL_TYPES. Six of the most-used types in Lean core arrive with nothing.
           Each emitted counterpart is a new LOGOS global with no lemmas, no `@[simp]` set, no
           `DecidableEq`, and no relationship to any other emitted type. The loss is
           per-declaration and must be recorded per-declaration, not once for the file.
@loss      SORT_PINNING_AT_TYPE_0. `add_structure` fixes the sort at `Type 0`
           (context.rs:300) and `derive_recursor` fixes the motive at `Type 0`
           (recursor.rs:64-71). So `Prod`, `Sigma` and `Subtype` land monomorphically at
           `Type 0`. `Subtype {x : α // p x}` with `p : α → Prop` is the acute case: Lean's
           `Subtype` is `Sort u`-polymorphic and is routinely used at `Prop`; the emitted one
           is not, and a `Subtype` over a `Prop` survives only by LOGOS's cumulativity
           (type_checker.rs:1330-1334), which Lean does not have. See T11-001 and T01-007.
@loss      SIGMA_IS_NOT_DEPENDENT_ENOUGH. `add_structure`'s `fields` may reference earlier
           fields by name (context.rs:275-282), so a two-field dependent record is expressible
           — but the field type must be a `Term`, and the second component's type in
           `Sigma β` is `β a`, an APPLICATION of a bound type family. That is representable,
           yet the derived projection is a `Term::Match` whose motive must be filled by
           `fill_match_motives`, which handles only Miller patterns. Dependent-pair
           projections outside that fragment must be rejected, not emitted.
@guard     Catches emitting `Option α` as `Sum Unit α` or as `α` "with a sentinel". The first
           is a different type with different constructor names, so every `Option.some`/`none`
           in the source has to be rewritten and no Lean lemma transfers; the second is
           unsound the moment `α` is inhabited by the sentinel.
@guard     Catches `Prod` emitted as an inductive rather than a STRUCTURE. Only structures in
           the `structures` registry get eta (type_checker.rs:1391-1417), so an inductive
           `Prod` makes `(p.1, p.2) = p` unprovable — a fact Lean proves by `rfl`. And even as
           a structure, eta lives only in `def_eq_normalized`, never in `normalize`
           (:1473-1478), so a proof that routes through `normalize` still fails. See T03-011.
@guard     Catches the `Subtype` projection losing the proof component. LOGOS has definitional
           proof irrelevance (`proof_irrelevant`, type_checker.rs:1547-1567), but it is a
           LAST-RESORT rule that calls `infer_type` on both sides, so it only fires when both
           proofs' types actually infer. A `Subtype` emitted with the proof field's type left
           as `Term::Hole` typechecks — `types_equal` matches `Hole` against anything
           (:1594-1597) — and then `proof_irrelevant` cannot fire because `infer_type` on a
           `Hole` is `CannotInferHole` (:313). Two distinct subtype elements become
           non-equal where Lean says they are equal. See T11-004 and T11-009.
@guard     Catches emitting `Fin n` as `Nat`. That drops the bound entirely, so
           `Fin 0` becomes inhabited and every `Fin`-indexed total function becomes a partial
           one. And the honest encoding — `{i : Nat // i < n}` — needs a `Nat` `<`, which
           LOGOS does not have as a primitive: its order axioms are all `Int` order axioms
           (prelude.rs:202-340), so the relation itself must be emitted as an inductive or a
           `Bool`-valued recursion, and the bound proofs re-derived.
@guard     Catches emitting six types with six different naming conventions. LOGOS keys are
           flat `String`s with no namespaces (context.rs:442 and the `get_global` chain at
           :449-464), so `Prod.fst` and a user's `Prod_fst` collide; see T11-003.
@oracle    Each of the six types is emitted exactly once; `Prod`, `Sigma` and `Subtype` appear
           in the `structures` registry (so `try_structure_eta` can fire); `infer_type` accepts
           every term below; and the loss record contains NO_STRUCTURAL_TYPES and
           SORT_PINNING_AT_TYPE_0 with a per-type breakdown.
-/

namespace T10_013

/-! ### Option -/

def maybeFive : Option Nat := some 5
def nothing : Option Nat := none

def orZero : Option Nat → Nat
  | none => 0
  | some n => n

example : orZero maybeFive = 5 := rfl

/-! ### Prod — a structure, so eta must hold -/

def pair : Nat × Bool := (3, true)

example : pair.1 = 3 := rfl
example : (pair.1, pair.2) = pair := rfl

/-! ### Sum -/

def leftVal : Nat ⊕ Bool := Sum.inl 3

def sumToNat : Nat ⊕ Bool → Nat
  | Sum.inl n => n
  | Sum.inr b => if b then 1 else 0

example : sumToNat leftVal = 3 := rfl

/-! ### Sigma — a DEPENDENT pair; the second field's type is an application -/

def dependentPair : Σ n : Nat, Fin (n + 1) := ⟨2, 0⟩

example : dependentPair.1 = 2 := rfl

/-! ### Subtype — the proof field is where irrelevance and `Hole` leniency collide -/

def positive : {n : Nat // 0 < n} := ⟨1, Nat.zero_lt_one⟩

example : positive.1 = 1 := rfl

/-- Two subtype elements with the SAME value and DIFFERENT proof terms. Lean identifies them
    by proof irrelevance; LOGOS does too, but only when `infer_type` succeeds on both proofs
    (type_checker.rs:1547-1567). -/
example : (⟨1, Nat.zero_lt_one⟩ : {n : Nat // 0 < n}) = ⟨1, Nat.one_pos⟩ := rfl

/-! ### Fin — needs Subtype, needs a `Nat` `<`, and `Fin 0` must stay empty -/

def index : Fin 3 := 1

example : index.val = 1 := rfl
example : (index : Nat) < 3 := index.isLt

/-- `Fin 0` is uninhabited. A `Nat` encoding that drops the bound makes this false. -/
example (i : Fin 0) : False := i.elim0

/-! ### The `Type 0` pinning, made observable -/

/-- A `Subtype` over a `Prop` — Lean's `Subtype` is `Sort u`-polymorphic. `add_structure`
    hard-codes `Type 0` (context.rs:300). -/
def propSubtype : {p : Prop // p} := ⟨True, trivial⟩

/-- A product at `Type 1`. -/
def typePair : Type × Type := (Nat, Bool)

end T10_013
