/-!
@case      T01-002
@tier      1 · Universes
@title     `Type` ≡ `Type 0` ≡ `Sort 1`, and the `Type n` / `Sort n` index shift
@lean      Type, Type 0, Type 1, Type 2, Sort 1, Sort 2, Sort (n+1)
@expect    translate
@target    `Type n` ≡ `Sort (n+1)` ⟼ `Term::Sort(Universe::Type(n))` — kernel/src/term.rs:445 and
           term.rs:177. `Type` with no index is `Type 0`, i.e. `Universe::Type(0)`.
@evidence  kernel/src/term.rs:175-177 (`Universe::Prop` and `Universe::Type(u32)` — LOGOS's
           `Type` index agrees with LEAN's `Type` index: `Type 0` is `Universe::Type(0)`);
           kernel/src/term.rs:266-276 (`to_lnat`, the level DECISION encoding, uses a DIFFERENT
           numbering: `Prop ↦ 0`, `Type n ↦ n+1` — this is the internal ℕ-model, mirroring
           Lean's `Level`, and it is NOT the surface mapping);
           kernel/src/type_checker.rs:54 (`Sort(u) : Sort(u.succ())`);
           kernel/src/term.rs:196-201 (`succ`: `Type(n) ↦ Type(n+1)`, `Prop ↦ Type(1)` — the
           `Type` arm agrees with Lean, the `Prop` arm does not, see T01-010);
           kernel/src/term.rs:205-216 (`max`: concrete `Type` operands collapse immediately);
           kernel/src/type_checker.rs:1330-1336 (`is_subtype_normalized` on two `Sort`s is
           `u1.is_subtype_of(u2)` — CUMULATIVE, so `Type 0 ≤ Type 1` holds in LOGOS and does NOT
           hold in Lean, see T01-009)
@guard     Catches the OFF-BY-ONE from using LOGOS's internal `to_lnat` encoding (term.rs:268-271,
           where `Type n ↦ n+1`) as the surface mapping. Lean's `Expr.sort` carries a `Level`,
           and `Type n` is `Sort (n+1)` — so a translator reading the `Level` directly and
           emitting `Universe::Type(level)` produces `Type 1` where `Type 0` was meant, for
           EVERY type in the development, uniformly. Because LOGOS is cumulative
           (type_checker.rs:1333), a uniformly-too-high level accepts every term the correct one
           accepted: `Nat : Type 1` still checks against every position expecting `Type 1` or
           higher. The error surfaces only where a level is DECLARED and compared downward — for
           instance against a structure, which `add_structure` pins to exactly `Type 0`
           (context.rs:300), or against a derived recursor's motive, pinned to `Type 0`
           (recursor.rs:64-71). At that point the diagnostic points at the structure, not at the
           translation of `Type`.
@guard     Catches the off-by-one in the OTHER direction — emitting `Universe::Type(n-1)` for
           `Sort n` by subtracting without handling `n = 0`. `Sort 0` is `Prop`, a different
           enum variant entirely (T01-001); a translator that computes `Type(n.pred)` maps
           `Sort 0` to `Type(0)`, silently losing impredicativity, proof irrelevance and the
           large-elimination gate.
@guard     Catches assuming `Type` with no index is polymorphic. In Lean, a bare `Type` inside a
           definition's signature may be AUTO-BOUND to a fresh universe parameter (T01-011), or
           may elaborate to the literal `Type 0`, depending on context and on
           `autoImplicit`/`relaxedAutoImplicit`. The translator must read the ELABORATED level,
           never the token, because the two spellings produce genuinely different declarations:
           one goes to `add_definition` (context.rs:442), the other needs `add_universe_poly`
           (context.rs:218) and `Term::Const { levels }` at every use site (T01-006).
@guard     Catches verifying the mapping with `Universe::is_subtype_of` (term.rs:244-246). It is
           cumulative, so `Type(0).is_subtype_of(Type(1))` is TRUE — an off-by-one passes that
           test in one direction. The conformance check must be STRUCTURAL equality on the
           `Universe` enum.
@guard     Catches the harness concluding "levels do not matter because LOGOS is cumulative".
           Cumulativity in LOGOS is SHALLOW: `is_subtype_normalized` (type_checker.rs:1330-1360)
           applies it only to two `Sort`s at the top and covariantly under a `Pi` codomain.
           Every other position — a type constructor's argument, a `Match` motive, a `Lambda`
           subterm — is INVARIANT and delegates to `def_eq_normalized`, where `Sort(u1)` and
           `Sort(u2)` are compared by `equiv` (type_checker.rs:1496). So the same off-by-one is
           forgiven at the top level and fatal one constructor deep — producing a translator
           that works on simple types and fails on containers.
@oracle    For each declaration, the emitted `Universe` is STRUCTURALLY equal to the table in
           `@target` — `Type n` ⟼ `Universe::Type(n)`, `Sort (n+1)` ⟼ `Universe::Type(n)`,
           `Sort 0` ⟼ `Universe::Prop`; and `infer_type` of each emitted `Sort` returns
           `Sort(u.succ())` matching the level Lean reports for the same `Expr`, except for the
           documented `Prop` discrepancy of T01-010.
-/

namespace T01_002

/-- `Type` with no index, `Type 0`, and `Sort 1` are the same Lean `Expr`. -/
def bareType : Type 1 := Type
def typeZero : Type 1 := Type 0
def sortOne  : Type 1 := Sort 1

example : bareType = typeZero := rfl
example : typeZero = sortOne := rfl

/-- One level up, and the corresponding `Sort`. -/
def typeOne : Type 2 := Type 1
def sortTwo : Type 2 := Sort 2

example : typeOne = sortTwo := rfl

/-- Two levels up — an off-by-one here is a type error, because the declared and computed sorts
    are adjacent. -/
def typeTwo : Type 3 := Type 2

/-- An ordinary type at `Type 0`. Most of a development looks like this, which is why a uniform
    off-by-one is invisible in a simple test suite. -/
def concreteType : Type := Nat

/-- A type CONSTRUCTOR at `Type 0`: the argument position is INVARIANT under
    `is_subtype_normalized` (type_checker.rs:1356-1358), so an off-by-one one constructor deep
    is fatal where the same off-by-one at the top is forgiven. -/
def container (α : Type) : Type := α → α

def containerAtNat : Type := container Nat

/-- A type constructor at `Type 1`, whose argument is a `Type 0`-valued type constructor. Two
    levels interacting, so a uniform shift changes both consistently and a non-uniform one
    does not. -/
def higherContainer (F : Type → Type) : Type 1 := (α : Type) → F α

def higherAtContainer : Type 1 := higherContainer container

/-- A function BETWEEN universes: domain at `Type 1`, codomain at `Type 0`. The `Pi` sort is
    `imax(Type 2, Type 1) = Type 2` in Lean's numbering, i.e. `Type 1` here. -/
def acrossLevels : Type 2 := Type 1 → Type 0

/-- `Type 0` is an INHABITANT of `Type 1`, which is a different statement from `Type 0` being a
    SUBTYPE of `Type 1`. Lean has the first and not the second (T01-009); LOGOS has both, and
    the second is what makes an off-by-one quiet. -/
def typeZeroInhabitsTypeOne : Type 1 := Type 0

/-- A universe-indexed ladder, so the relative offsets are pinned even under a uniform shift. -/
def ladder0 : Type 1 := Type 0
def ladder1 : Type 2 := Type 1
def ladder2 : Type 3 := Type 2

end T01_002
