/-!
@case      T07-011
@tier      7 · Typeclasses
@title     Mathlib's algebraic hierarchy — the case that explains why Mathlib as a whole does
           not port
@lean      class Mul/One/Semigroup/Monoid/Group and their `extends` chain, universe-polymorphic
           algebraic structures, `Monoid.{u}` instantiated at more than one level
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/context.rs:271-369 (`add_structure` is the ONLY record-declaring API);
           kernel/src/context.rs:300 (`let ind_type = pis(params, Term::Sort(Universe::Type(0)))`
           — the structure's sort is HARD-CODED to `Type 0`; there is no sort argument and no
           universe-parameter list); kernel/src/context.rs:253,374
           (`add_inductive(name, sort)` / `add_indexed_inductive(name, sort, num_params)` take a
           single FIXED `sort: Term` — an inductive cannot carry universe parameters either);
           kernel/src/context.rs:218 (`add_universe_poly(name, params, ty, body)` — universe
           polymorphism exists for DEFINITIONS ONLY, and requires a `body`, which a type
           declaration does not have); kernel/src/term.rs:458 (`Term::Const { name, levels }` is
           the only universe-instantiating form, and kernel/src/type_checker.rs:70-85 plus
           kernel/src/reduction.rs:82-91 resolve it ONLY against `Context::universe_polys`);
           kernel/src/term.rs:167-191 (`Universe::Type(u32)` takes a CONCRETE `u32`, not a level
           expression); kernel/src/recursor.rs:64-71 (`derive_recursor` hard-codes the motive
           target to `Sort(Type 0)` for a non-`Prop` inductive, so a structure's eliminator
           cannot eliminate into `Type 1+` either); kernel/src/context.rs:71,184-186
           (the instance database is a flat `Vec<(Term,Term)>` with no priorities and no
           coherence checking — T07-005, T07-008, T07-013)
@diag      E_NO_UNIVERSE_POLYMORPHIC_STRUCTURE — must name the class, list its universe
           parameters, cite `add_structure`'s hard-coded `Type 0` (context.rs:300), and state
           that neither the structure nor a replacement inductive can carry a universe
           parameter. Must NOT suggest monomorphisation at `Type 0` as a fix.
@guard     THIS IS THE CASE THAT SETTLES THE MATHLIB QUESTION, and it is a CONJUNCTION of three
           independent walls. A translator that works around any one of them still hits the
           other two:
             1. UNIVERSE POLYMORPHISM. `Monoid.{u} (M : Type u)` needs a universe parameter on a
                TYPE. `add_structure` hard-codes `Type 0` (context.rs:300); `add_inductive` takes
                one fixed sort (context.rs:253); `add_universe_poly` handles DEFINITIONS only and
                needs a body (context.rs:218). There is no third option. See T01-007.
             2. THE `extends` CHAIN. Mathlib's hierarchy is ~15 levels of `extends` with
                deliberate diamonds (`CommRing` reaches `AddCommMonoid` by several paths) whose
                coherence is maintained by hand and checked by Lean. LOGOS's instance database
                cannot express a priority, cannot express a scoped instance, and resolves
                first-match-wins on insertion order (elaborate.rs:1331-1340) — so the diamond
                resolves to whichever path the translator happened to emit first, undetectably.
                See T07-005 and T07-013.
             3. `outParam`. `HMul`/`HAdd`/`SMul` — the classes that carry Mathlib's notation —
                have `outParam` positions, which LOGOS has no concept of. See T07-006.
@guard     THE TEMPTING WRONG MOVE: monomorphise every class at `Type 0` and carry on. It
           compiles. It typechecks. And it is wrong the moment the source instantiates a class
           at two different levels — which Mathlib does constantly, because a `Module R M` has
           `R : Type u` and `M : Type v` with `u ≠ v` in general, and because `CategoryTheory`
           quantifies over `Type (u+1)`. The two instantiations collapse to ONE flat LOGOS
           global (keys are flat `String`s, context.rs:449), so `Monoid` at level 0 and `Monoid`
           at level 1 share a declaration whose projections and derived eliminator can only
           land in `Type 0` (recursor.rs:64-71). Every downstream proof is then checked against
           a type that is not the one the Lean source meant. And because LOGOS is CUMULATIVE
           (`Prop ≤ Type 0 ≤ Type 1`, type_checker.rs:1330-1360) while Lean is not, the
           resulting terms very often STILL TYPECHECK — so the kernel will not catch this for
           you. See T01-007 and T11-019.
@guard     Catches per-instantiation monomorphisation (`Monoid@0`, `Monoid@1`). It is sound-ish
           for the types but breaks the moment a universe-polymorphic LEMMA quantifies over `u`
           — `mul_assoc` in Mathlib is `∀ {M : Type u} [Monoid M]`, and there is no level
           variable to bind on the emitted structure. The translator must then reject any
           declaration whose universe parameters are not fully determined at every use site,
           which is most of the library. See T01-008.
@guard     Catches "declare the classes as `Prop`-valued mixins instead". Mathlib's `Monoid`
           carries DATA (`mul`, `one`), not just laws, so it cannot be a `Prop`; and
           `add_structure` would put it in `Type 0` regardless (context.rs:300).
@guard     Catches the partial-success failure mode, which is the realistic one. A translator
           will happily emit `Mul`, `One` and `Semigroup` at `Type 0` — they LOOK fine — and
           then fail on `Module` or on a `Type (u+1)` category. At that point the context
           already contains a dozen monomorphised classes and their instances, and the
           certificate minted from it claims `PRELUDE_VERSION = "logos-coc-1"`
           (certificate.rs:28) while describing a hierarchy that does not mean what its source
           meant. The rejection must happen at the FIRST universe-polymorphic class, not at the
           first one that fails to typecheck.
@oracle    Translator exits non-zero with E_NO_UNIVERSE_POLYMORPHIC_STRUCTURE at `Mul`
           (the first universe-polymorphic class in the file) and writes NO declaration into the
           LOGOS context — in particular, zero `add_structure` calls. A harness additionally
           asserts that the translator does not emit a `Type 0` monomorphisation of any class
           declared with a universe parameter.
-/

namespace T07_011

/-- A minimal local re-declaration of Mathlib's hierarchy. Every class here is
    UNIVERSE-POLYMORPHIC, exactly as Mathlib's are. -/
class Mul (α : Type u) where
  mul : α → α → α

class One (α : Type u) where
  one : α

/-- Level 1 of the `extends` chain. -/
class Semigroup (α : Type u) extends Mul α where
  mul_assoc : ∀ a b c : α, Mul.mul (Mul.mul a b) c = Mul.mul a (Mul.mul b c)

/-- Level 2 — a genuine diamond already: `Monoid` reaches `Mul` through `Semigroup` and would
    reach it again through `One` if `One` extended it. -/
class Monoid (α : Type u) extends Semigroup α, One α where
  one_mul : ∀ a : α, Mul.mul One.one a = a
  mul_one : ∀ a : α, Mul.mul a One.one = a

/-- Level 3. -/
class Group (α : Type u) extends Monoid α where
  inv : α → α
  inv_mul : ∀ a : α, Mul.mul (inv a) a = One.one

/-- An instance at `Type 0`. -/
instance : Mul Nat := ⟨Nat.mul⟩
instance : One Nat := ⟨1⟩
instance : Semigroup Nat := ⟨Nat.mul_assoc⟩
instance : Monoid Nat := ⟨Nat.one_mul, Nat.mul_one⟩

/-- A universe-polymorphic LEMMA over the hierarchy. There is nowhere in LOGOS to bind `u`. -/
theorem assoc4 {α : Type u} [Semigroup α] (a b c d : α) :
    Mul.mul (Mul.mul (Mul.mul a b) c) d = Mul.mul (Mul.mul a (Mul.mul b c)) d := by
  rw [Semigroup.mul_assoc a b c]

/-- THE INSTANTIATION THAT MAKES MONOMORPHISATION UNSOUND, not merely lossy: the same class at
    two different universe levels in one file. A `Type 0` monomorphisation collapses these to
    one declaration. -/
def endoMul (α : Type) : Mul (α → α) := ⟨fun f g => fun a => f (g a)⟩

instance : Mul (Nat → Nat) := endoMul Nat

/-- Level 1: a `Mul` on `Type 1`. Mathlib does this constantly (`Module R M` with `R` and `M`
    at different levels; `CategoryTheory` at `Type (u+1)`). -/
instance mulType : Mul (Type) := ⟨fun α _ => α⟩

/-- A `Module`-shaped class with TWO independent universe parameters — the shape that defeats
    even per-instantiation monomorphisation, because the two levels vary independently. -/
class Action (R : Type u) (M : Type v) where
  smul : R → M → M

instance : Action Nat Nat := ⟨Nat.mul⟩
instance : Action Nat (Nat → Nat) := ⟨fun n f => fun a => n * f a⟩

end T07_011
