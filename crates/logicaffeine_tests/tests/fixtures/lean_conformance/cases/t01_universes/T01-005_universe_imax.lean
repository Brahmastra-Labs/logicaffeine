/-!
@case      T01-005
@tier      1 · Universes
@title     `imax u v` — impredicative Π formation; LOGOS matches Lean, and must not "simplify"
@lean      Sort (imax u v), ∀ into Prop, Pi type formation rule
@expect    translate
@target    Term::Sort(Universe::IMax(Box<Universe>, Box<Universe>)) — kernel/src/term.rs:190.
           Built through `Universe::imax` (term.rs:219-237), never by hand. For a Π, the level is
           computed by the kernel itself (type_checker.rs:104-110) and need not be emitted; where
           Lean's source WRITES `Sort (imax u v)` as an annotation, the annotation must be
           emitted as an `IMax`, not collapsed.
@evidence  kernel/src/term.rs:184-190 (`IMax(Box, Box)` with the doc: "`imax(a, b)` = `b` if `b`
           is `Prop`, else `max(a, b)` … When `b` is a variable this stays SYMBOLIC — the level
           may be `Prop` or not depending on the instantiation");
           kernel/src/term.rs:219-237 (`Universe::imax`: `SProp ↦ SProp`, `Prop ↦ Prop`,
           `Type(_) | Succ(_) ↦ self.max(other)`, `imax(a,a) = a`, otherwise a symbolic `IMax`);
           kernel/src/type_checker.rs:100-110 (Π formation is `a_sort.imax(&b_sort)` — exactly
           Lean's rule, with the in-source comment naming impredicativity as the reason);
           kernel/src/term.rs:298-320 (`simp_lnat` resolves `imax` whose right operand's
           Prop-ness is DETERMINED and pushes it down over `max`/`imax` until the right operand
           is a bare variable — `LNat::Var(_) => LNat::IMax(...)`, "stays symbolic (v may be 0 or
           ≥ 1)");
           kernel/src/term.rs:322-360 (`level_leq` then case-splits on those pivot variables —
           the decision procedure for an unresolved `imax`);
           kernel/src/type_checker.rs:1547-1567 and :277-288 (proof irrelevance and the
           large-elimination gate both test for `Sort(Universe::Prop)` after `normalize`, so
           whether a Π lands in `Prop` decides whether either fires)
@guard     Catches "simplify `imax u v` to `max u v` because `v` is a type level". Whether `v` is
           `Prop` is NOT decidable before instantiation — LOGOS's own `simp_lnat`
           (term.rs:315-318) keeps `imax(a, Var v)` symbolic for exactly this reason, and
           `level_leq` case-splits on it (term.rs:322-360). Collapsing to `max` makes an
           impredicative `∀` predicative: `∀ (α : Sort u), P α` moves from `Prop` to
           `max u (level of P)`. Because LOGOS is cumulative (type_checker.rs:1333), the larger
           level is accepted wherever the smaller was, so nothing fails at the declaration. It
           fails LATER and elsewhere: `And`/`Or`/`Ex` from the prelude require `Prop` arguments,
           and `proof_irrelevant` (type_checker.rs:1553-1560) stops firing because the statement's
           sort is no longer syntactically `Universe::Prop`. The user sees "this `rfl` no longer
           works", three declarations away from the cause.
@guard     Catches the reverse "simplification", `max ⟼ imax`. `imax(a, Type n) = max(a, Type n)`
           (term.rs:227) so the two agree when the codomain is a concrete non-`Prop` — which is
           most of a test suite. They diverge exactly when the codomain level is a VARIABLE, and
           there `imax(a, Var v)` is strictly smaller than `max(a, Var v)` at the `v = Prop`
           instantiation. An `IMax` emitted where Lean computed a `Max` therefore under-states
           the level, and under-stating is the direction cumulativity does NOT forgive.
@guard     Catches building `Universe::IMax(a, b)` directly rather than calling `Universe::imax`.
           The constructor does no collapsing; `imax` resolves the `SProp`/`Prop`/concrete cases
           eagerly and also applies `imax(a,a) = a` (term.rs:230-232). A hand-built
           `IMax(Type(0), Prop)` is `equiv` to `Prop` but structurally different, so the
           translator's own structural comparisons disagree with the kernel's `equiv`-based ones
           (term.rs:249-252, type_checker.rs:1496). Same failure shape as T01-004.
@guard     Catches the `SProp` arm. `Universe::imax` checks `SProp` FIRST (term.rs:222-224), so
           `imax(a, SProp) = SProp`. Lean 4 has no `SProp` at all, so no Lean level can produce
           this arm — any translator output that reaches it has invented a universe. See T01-013.
@guard     Catches assuming the Π's level must be EMITTED. `infer_type` computes it
           (type_checker.rs:109) from the two operand sorts; a translator that also writes it
           into a declared type and gets it wrong creates a mismatch between the declared and
           computed sorts of the very same term — which `is_subtype` will forgive upward and
           refuse downward, so the bug is direction-dependent.
@oracle    For each declaration, `infer_type` on the emitted `Pi` returns a `Sort` whose level is
           `Universe::equiv` to the translation of Lean's level; every emitted `IMax` was
           produced by `Universe::imax`; and for each `∀` into a proposition below, the computed
           level is syntactically `Universe::Prop` — the test that impredicativity survived and
           that proof irrelevance and the large-elimination gate will still fire.
-/

namespace T01_005

/-- The written annotation: `Sort (imax u v)` is the sort of `α → β`. -/
def piSort.{u, v} (α : Sort u) (β : Sort v) : Sort (imax u v) := α → β

/-- Instantiated with `v = 0`, i.e. the codomain is a `Prop`: the whole Π is a `Prop`. -/
def piIntoProp : Prop := piSort Nat True

/-- Instantiated with `v ≥ 1`: the Π is predicative and `imax` behaves as `max`. -/
def piIntoType : Type := piSort Nat Bool

/-- IMPREDICATIVITY, unannotated: Lean computes this Π's sort with `imax` and gets `Prop`,
    even though the domain quantifies over all of `Type`. -/
def forallOverTypesIsProp : Prop := ∀ (α : Type), ∀ (_a : α), True

/-- One level higher, to pin that the collapse is `imax`'s and not an accident of `Type 0`. -/
def forallOverTypeOneIsProp : Prop := ∀ (α : Type 1), ∀ (_a : α), True

/-- The PREDICATIVE control: same domain, `Type`-valued codomain. `imax` does not collapse, and
    the Π lives at `max (u+1) 1`. If a translator turns `imax` into `max`, this and
    `forallOverTypesIsProp` become the same level. -/
def forallOverTypesIsType : Type 1 := ∀ (_α : Type), Nat

/-- `imax u u`, which `Universe::imax` collapses by its `imax(a,a) = a` rule (term.rs:230-232). -/
def piSameLevel.{u} (α : Sort u) (β : Sort u) : Sort (imax u u) := α → β

/-- The level that stays SYMBOLIC: the codomain level is a bare variable, so neither Lean nor
    LOGOS can decide whether the result is a `Prop`. This is the case a "simplifying"
    translator destroys. -/
def piSymbolic.{u, v} (α : Sort u) (β : Sort v) : Sort (imax u v) := α → β

/-- The same declaration instantiated on BOTH sides of the split, in one file — so a translator
    that resolved the `imax` one way is wrong for the other use. -/
def symbolicAtProp : Prop := piSymbolic Nat True
def symbolicAtType : Type := piSymbolic Nat Bool

/-- A NESTED `imax`: the codomain is itself a Π. `simp_lnat` (term.rs:316-318) rewrites
    `imax(a, imax(b1,b2))` to `imax(max(a,b1), b2)`. -/
def piNested.{u, v, w} (α : Sort u) (β : Sort v) (γ : Sort w) : Sort (imax u (imax v w)) :=
  α → β → γ

/-- An `imax` over a `max`: `simp_lnat` (term.rs:313-315) distributes,
    `imax(a, max(b1,b2)) = max(imax(a,b1), imax(a,b2))`. -/
def piOverMax.{u, v, w} (α : Sort u) (β : Type v) (γ : Type w) : Sort (imax u (max (v+1) (w+1))) :=
  α → (β → γ)

/-- A dependent `∀` whose codomain's level depends on the bound variable's VALUE — the codomain
    is a `Prop` for every instantiation, so `imax` collapses and proof irrelevance applies. -/
theorem dependentProp (P : Nat → Prop) (h : ∀ n, P n) : P 0 := h 0

end T01_005
