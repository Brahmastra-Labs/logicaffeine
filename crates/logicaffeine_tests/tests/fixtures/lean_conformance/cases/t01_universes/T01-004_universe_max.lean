/-!
@case      T01-004
@tier      1 · Universes
@title     `max u v` — `Universe::Max`, and why the derived `PartialEq` on `Universe` is a trap
@lean      Type (max u v), Sort (max u v), universe max in Pi formation
@expect    translate
@target    Term::Sort(Universe::Max(Box<Universe>, Box<Universe>)) — kernel/src/term.rs:183.
           Built through `Universe::max` (term.rs:205-216), never by hand, so that concrete
           operands collapse the way the rest of the kernel expects.
@evidence  kernel/src/term.rs:183 (`Max(Box<Universe>, Box<Universe>)` — the least upper bound,
           used in Π formation);
           kernel/src/term.rs:205-216 (`Universe::max`: `SProp` is absorbed, `Prop` is absorbed,
           two concrete `Type`s collapse to `Type(a.max(b))`, and ONLY otherwise is a symbolic
           `Max` formed);
           kernel/src/term.rs:164-167 (the doc: "The algebra (`succ`/`max`/`equiv`/
           `is_subtype_of`) is decided over a canonical normal form, NOT by the derived
           structural equality — `max(u,u) ≡ u`, `max(succ u, u) ≡ succ u`, etc.");
           kernel/src/term.rs:169 (`#[derive(Debug, Clone, PartialEq, Eq)]` on `Universe` — the
           derived `==` IS structural, and it is what `Term`'s own derived `PartialEq` uses);
           kernel/src/term.rs:249-252 (`equiv` = mutual `level_leq`, the CORRECT comparison) and
           term.rs:266-360 (`to_lnat` / `simp_lnat` / `level_leq`, the decision core);
           kernel/src/type_checker.rs:104-110 (Pi formation: `a_sort.imax(&b_sort)`, and `imax`
           delegates to `max` when the codomain is a concrete non-`Prop`, term.rs:227);
           kernel/src/type_checker.rs:1496 (`def_eq_normalized` compares two `Sort`s with
           `u1.equiv(u2)` — so the kernel itself never relies on the derived equality for sorts)
@guard     Catches comparing emitted levels with `==` (the derived `PartialEq`, term.rs:169).
           `Max(Var "u", Var "u")` and `Var "u"` are `equiv` (term.rs:249) and structurally
           UNEQUAL. A translator that canonicalises or de-duplicates levels with `==` — in a
           cache key, in a memo table, in a "did this declaration change" check, or in its own
           conformance assertion — will treat two equivalent levels as different and emit two
           different monomorphisations, or will fail its own round-trip test on a term the
           kernel is perfectly happy with. Every level comparison the translator makes must go
           through `Universe::equiv`.
@guard     Catches building `Universe::Max(a, b)` directly instead of calling `Universe::max`.
           The constructor does no collapsing; `max` (term.rs:205-216) absorbs `Prop`/`SProp` and
           folds two concrete `Type`s. A hand-built `Max(Type(0), Type(0))` therefore reaches the
           rest of the kernel in a shape nothing else produces. It is still *decided* correctly
           by `level_leq` (which normalises through `to_lnat`), so nothing errors — but
           `types_equal` (type_checker.rs:1600) compares `Sort`s with `equiv`, while a translator's
           own structural diff does not, and the two disagree about whether the emitted term
           changed.
@guard     Catches `max u v ⟼ Universe::Type(k)` for some guessed `k`. `Universe::Type` takes a
           concrete `u32` (term.rs:177); there is no `Type(<expr>)`. A translator that "evaluates"
           `max u v` by defaulting `u` and `v` to 0 monomorphises the declaration (T01-007,
           T01-008) and, because LOGOS is cumulative (type_checker.rs:1333), the monomorphised
           form still typechecks at every use site that wanted a LARGER level.
@guard     Catches the `Prop` absorption difference. `Universe::max` (term.rs:208-210) absorbs
           `Prop`: `max(Prop, u) = u`. In Lean's level algebra `Prop` is level 0 and `max 0 u = u`
           numerically, so the two agree — but only because LOGOS's `to_lnat` (term.rs:268-271)
           also puts `Prop` at 0. A translator that "fixes" the `Prop : Type 1` off-by-one
           (T01-010) by shifting `Prop` up one in its own level model breaks this agreement and
           makes `max Prop (Type 0)` come out as `Type 1`.
@guard     Catches forgetting that `max` is where the LEVEL ORDER of a definition's parameters
           becomes observable. `add_universe_poly`'s `params` is a `Vec<String>` zipped
           positionally against `Term::Const`'s `levels` (type_checker.rs:84-85). `max u v` is
           symmetric, so a swapped parameter order is undetectable in a declaration whose only
           level expression is a `max` — and detectable in the next declaration that uses `u`
           alone. The file below pairs one of each for exactly this reason.
@oracle    For each declaration, the emitted level is `Universe::equiv` to the translation of
           Lean's level (NOT `==`); levels are constructed via `Universe::max` so that concrete
           operands are already collapsed; and `infer_type` on the emitted `Pi` returns a `Sort`
           whose level is `equiv` to the declared one. Additionally the harness asserts
           `Universe::Max(Var "u", Var "u") != Var "u"` structurally while
           `Universe::Max(Var "u", Var "u").equiv(&Var "u")` is true — pinning that the two
           comparisons genuinely differ.
-/

namespace T01_004

/-- The canonical `max`: a function type between two independently-levelled types lives at
    `max u v`. A DEFINITION, so `add_universe_poly` can bind both parameters. -/
def arrow.{u, v} (α : Type u) (β : Type v) : Type (max u v) := α → β

/-- Instantiated so that `u ≠ v`, which is what makes the `max` observable. -/
def arrowAt01 : Type 1 := arrow Nat Type
def arrowAt10 : Type 1 := arrow Type Nat
def arrowAt00 : Type := arrow Nat Bool

/-- `max u u`. Lean's level normaliser collapses this to `u`; LOGOS's `Universe::max`
    (term.rs:211-213) collapses it only when both operands are CONCRETE, so the emitted level is
    a symbolic `Max(Var "u", Var "u")` — structurally different from `Var "u"`, `equiv` to it. -/
def arrowSameLevel.{u} (α : Type u) (β : Type u) : Type (max u u) := α → β

/-- `max u u` written as plain `u`, so the harness has both spellings of one level to compare. -/
def arrowSameLevelPlain.{u} (α : Type u) (β : Type u) : Type u := α → β

/-- A THREE-way max, which associates: `max u (max v w)` versus `max (max u v) w`. `level_leq`
    decides them equal; the derived `==` does not. -/
def arrow3.{u, v, w} (α : Type u) (β : Type v) (γ : Type w) : Type (max u (max v w)) :=
  α → β → γ

def arrow3Assoc.{u, v, w} (α : Type u) (β : Type v) (γ : Type w) : Type (max (max u v) w) :=
  α → β → γ

/-- `max u (u+1)`, which the algebra collapses to `u+1` (term.rs:165-166 names exactly this
    identity). Symbolic operands, so no eager collapse. -/
def maxWithSucc.{u} (α : Type u) (β : Type (u + 1)) : Type (max u (u + 1)) := α → β

/-- ASYMMETRIC use of the same two parameters, so a swapped `params` vector IS detectable —
    the control for `arrow`, where the `max` hides a swap. -/
def secondOnly.{u, v} (_α : Type u) (β : Type v) : Type v := β

/-- `max` arising from Π formation rather than from a written annotation: Lean computes the sort
    of this `∀` itself, and LOGOS computes it at type_checker.rs:104-110. The two must agree. -/
def piComputedMax.{u, v} : Type (max (u + 1) (v + 1)) := Type u → Type v

/-- `max` with a `Prop` operand, where `Universe::max`'s absorption rule (term.rs:208-210)
    applies. In Lean this is `max 0 (u+1)`. -/
def maxWithProp.{u} (α : Type u) : Type u := (Prop → α)

end T01_004
