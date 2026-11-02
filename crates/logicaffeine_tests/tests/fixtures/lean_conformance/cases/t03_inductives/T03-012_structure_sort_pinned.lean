/-!
@case      T03-012
@tier      3 · Inductives and structures
@title     Structure in `Prop` or `Type u` (u ≥ 1) — `add_structure` hard-codes `Type 0`
@lean      structure in Prop, structure in Type 1, structure with a `Type`-valued field
@target    (none — no sound translation through `add_structure`)
@expect    reject
@diag      E_STRUCTURE_SORT_UNREPRESENTABLE — must name the structure and its Lean sort, and
           state that `Context::add_structure` registers every record at `Sort(Type 0)`. Must
           not silently re-sort the structure, and must not fall back to a hand-assembled
           inductive (that loses definitional eta, T03-010).
@evidence  kernel/src/context.rs:299-301 (`let ind_type = pis(params, Term::Sort(Universe::Type(0)));`
           followed by `add_indexed_inductive(name, ind_type, params.len())` — the sort is a
           **literal** `Type(0)`, not a parameter of `add_structure`);
           kernel/src/context.rs:268-270 (the doc says so: "The structure lives in `Type 0`
           (fields over `Type 0` carriers) — the common case for the algebraic hierarchy");
           kernel/src/context.rs:271-276 (the signature — `params` and `fields`, no sort
           argument, so there is nowhere to pass one);
           kernel/src/context.rs:88-92 and :365-368 (only `add_structure` writes the
           `structures` registry via `register_struct_info`, and only entries in that registry
           get definitional eta);
           kernel/src/type_checker.rs:1391-1417 (`try_structure_eta` is gated on
           `struct_of_constructor`, i.e. on that registry);
           kernel/src/type_checker.rs:545-580 (`check_constructor_universes`: a `Type k`
           inductive may not store a field whose sort exceeds `Type k` — so a `Type 0`
           structure cannot hold a `Type 0`-typed field, which lives in `Type 1`);
           kernel/src/term.rs:169-191 (`Universe::Type(u32)` — `Prop` is a *separate*
           constructor, not `Type 0`, so "Prop is close enough to Type 0" is false in LOGOS
           too)
@guard     Catches silently re-sorting a `Prop` structure to `Type 0`. In Lean a
           `structure … : Prop` is a proposition: it is subject to proof irrelevance and to the
           large-elimination restriction, and its inhabitants are not data. Registering it at
           `Type 0` makes it data. Because LOGOS is **cumulative** (`Prop ≤ Type 0 ≤ Type 1`,
           type_checker.rs:1330-1360) while Lean is not, almost every term that mentioned the
           structure still typechecks — so the kernel will not catch this. What changes is that
           `is_subsingleton_prop` (type_checker.rs:1688) no longer applies, the
           large-elimination gate (type_checker.rs:277-290) no longer fires, and a `Match` on
           the structure may now extract a `Type`-level value from what the source called a
           proof.
@guard     Catches silently re-sorting a `Type 1` structure **down** to `Type 0`. This is the
           direction cumulativity does *not* rescue: a structure with a `Type`-valued field
           genuinely lives in `Type 1`, and registering it at `Type 0` makes
           `check_constructor_universes` reject the constructor — if the translator went through
           `add_constructor_checked`. If it went through the unchecked `add_constructor`
           (context.rs:419-428), which is what `add_structure` itself calls (context.rs:306),
           **the universe violation is never checked at all** and a Girard/Hurkens-shaped
           declaration enters the context.
@guard     Catches the "just use a plain inductive at the right sort" repair. `add_inductive`
           does take a sort, so a `Prop` or `Type 1` record *can* be registered as a
           one-constructor inductive with hand-written projections — but nothing calls
           `register_struct_info`, so the type is absent from the `structures` registry and
           **definitional eta is gone**. Every Lean proof that closed by `rfl` on
           `⟨p.1, p.2⟩ = p` fails. That is a degrade, not a translation, and it must be
           reported as one rather than passed off as success.
@guard     Catches assuming `Prop` structures are rare. `And`, `Iff`, `Exists` and every
           bundled `Prop`-valued class are of this shape; LOGOS's own `And`/`Ex` are registered
           as plain inductives at `Prop` (prelude.rs:1815, :1925) precisely *because*
           `add_structure` could not do it.
@oracle    Translator exits non-zero with E_STRUCTURE_SORT_UNREPRESENTABLE for any structure
           whose Lean sort is not `Type 0`, and registers nothing for it. For every structure it
           does emit, `get_global(name)` normalizes to a sort ending in `Type 0` and
           `struct_info(name)` is `Some`.
-/

namespace T03_012

/-- The translatable baseline, for contrast: a `Type 0` structure. -/
structure Point where
  x : Nat
  y : Nat

example (p : Point) : Point.mk p.x p.y = p := rfl

/-- **Case 1 — a structure in `Prop`.** Lean accepts it; `add_structure` cannot express it.
    Its inhabitants are proofs, and Lean's proof irrelevance applies to them. -/
structure Conj (P : Prop) (Q : Prop) : Prop where
  left : P
  right : Q

def conjSwap {P Q : Prop} (h : Conj P Q) : Conj Q P := Conj.mk h.right h.left

/-- Proof irrelevance: in Lean any two inhabitants of a `Prop` are equal. Re-sorting `Conj`
    to `Type 0` destroys this. -/
example {P Q : Prop} (h1 h2 : Conj P Q) : h1 = h2 := rfl

/-- Large elimination is *forbidden* for a non-subsingleton `Prop` in Lean and in LOGOS
    (type_checker.rs:277-290). At `Type 0` it would be allowed — so re-sorting changes what
    can be extracted from a proof. `Conj` happens to be a subsingleton, so the commented line
    below is legal in Lean; the point is that the *gate* stops applying at all.
    `def extract {P Q : Prop} (h : Conj P Q) : Type := Nat` -/
example {P Q : Prop} (h : Conj P Q) : P := h.left

/-- **Case 2 — a structure in `Type 1`.** It stores a `Type`, whose sort is `Type 1`, so the
    structure itself cannot live in `Type 0`. Registering it at `Type 0` is a universe
    violation that only `add_constructor_checked` would catch — and `add_structure` does not
    use it. -/
structure Carrier : Type 1 where
  ty : Type
  point : ty

def natCarrier : Carrier := Carrier.mk Nat 0

example : natCarrier.ty = Nat := rfl

/-- The universe obligation made explicit: `Carrier`'s own sort is `Type 1`. -/
example : Carrier = Carrier := rfl

/-- **Case 3 — a parameterised structure in `Type 1`.** `add_structure`'s parameters are
    unconstrained in principle, but the codomain it builds is always `Type 0`
    (context.rs:300), so no parameterisation rescues this. -/
structure Bundle (ι : Type) : Type 1 where
  fibre : ι → Type
  base : ι

def boolBundle : Bundle Bool := Bundle.mk (fun _ => Nat) true

example : boolBundle.base = true := rfl

/-- **Case 4 — the non-subsingleton `Prop`.** Lean refuses to generate projections for a
    `Prop` structure carrying a data field ("field 'idx' is not a proof"), so this shape has
    to be an ordinary two-constructor inductive at `Prop`. That is exactly why LOGOS's own
    `Or` is registered with `add_inductive` at `Prop` (prelude.rs:1858) and not through
    `add_structure`: the record API cannot reach `Prop` at all, and the inductive API cannot
    grant eta. The large-elimination gate (type_checker.rs:277-290) bites here and would stop
    biting if the sort were silently moved to `Type 0`. -/
inductive Disj (P : Prop) (Q : Prop) : Prop where
  | inl : P → Disj P Q
  | inr : Q → Disj P Q

def disjSwap {P Q : Prop} : Disj P Q → Disj Q P
  | Disj.inl h => Disj.inr h
  | Disj.inr h => Disj.inl h

/-- **Case 5 — a structure whose *field* is a `Prop`, but which is itself `Type 0`.** This one
    *is* representable: the sort gate is on the structure, not on its fields. Included to pin
    the boundary from the legal side. -/
structure Bounded where
  value : Nat
  proof : value ≤ 10

def five : Bounded := Bounded.mk 5 (by omega)

example : five.value = 5 := rfl
example (b : Bounded) : Bounded.mk b.value b.proof = b := rfl

end T03_012
