/-!
@case      T01-003
@tier      1 · Universes
@title     `Sort u` for a level VARIABLE — `Universe::Var` exists but is bindable only on a definition
@lean      Sort u, Type u, universe parameters on a declaration, `universe u`
@expect    degrade
@target    Term::Sort(Universe::Var("u")) — kernel/src/term.rs:179 — together with a binding
           site, which exists in exactly one place: `Context::add_universe_poly(name, params,
           ty, body)` (context.rs:218), and is usable only for DEFINITIONS. References then use
           `Term::Const { name, levels }` (term.rs:458). A loss record naming
           LEVEL_BINDING_SCOPE must be emitted for every declaration whose universe parameters
           cannot be bound — i.e. every non-definition.
@evidence  kernel/src/term.rs:179 (`Universe::Var(String)` — a universe variable, carried as a
           flat `String`);
           kernel/src/context.rs:218-225 (`add_universe_poly` — the ONLY API that records a
           `Vec<String>` of universe parameters for a name, and its doc says "universe-
           polymorphic DEFINITION");
           kernel/src/type_checker.rs:69-85 (`Const` resolution: `get_universe_poly`, arity check
           `params.len() != levels.len()`, then `instantiate_universes`);
           kernel/src/context.rs:253, :374, :419, :434 (`add_inductive`, `add_indexed_inductive`,
           `add_constructor`, `add_declaration` — NONE of them takes a universe-parameter list,
           so a `Universe::Var` inside their types has no binder);
           kernel/src/term.rs:268-276 (`to_lnat`: `Var(v) ↦ LNat::Var(v)`, and the doc at
           term.rs:270-272 states every universe VARIABLE ranges over all of ℕ "so a variable may
           be `Prop`, which is what closes the `Sort u := Nat` unsoundness — `Type 0 ≤ u` is
           false");
           kernel/src/term.rs:244-246 and the `level_leq` decision core (term.rs:298-360)
           (`u ≤ u`, `Type 0 ≤ u` FALSE, `u ≤ v` FALSE — decided by case-splitting on whether the
           pivot variable is `Prop`)
@loss      LEVEL_BINDING_SCOPE. Lean attaches universe parameters to EVERY declaration kind —
           `def`, `theorem`, `axiom`, `inductive`, `structure`, `instance`. LOGOS attaches them
           to definitions only. So `Sort u` appearing in an axiom's type, an inductive's arity,
           or a constructor's type is emitted as a `Universe::Var` with NO BINDING SITE. The
           kernel does not reject this — `infer_sort` happily returns `Universe::Var("u")` and
           `level_leq` answers questions about it conservatively — so the declaration is
           accepted with a free level variable that can never be instantiated. Acceptable only
           for definitions, where `add_universe_poly` binds it; for anything else the loss
           escalates to a rejection (inductives: T01-007; axioms and level metavariables:
           T01-012).
@guard     Catches the FLAT NAMESPACE of level variables. `Universe::Var` holds a bare `String`
           and `Universe::equiv` (term.rs:249-252) compares levels through `to_lnat`, where
           `Var("u")` is `LNat::Var("u")` — matched by NAME. Two unrelated declarations that both
           call their parameter `u` therefore have levels that `equiv` reports EQUAL. Since Lean
           names auto-bound parameters `u_1`, `u_2`, … in every declaration independently, a
           translator that copies Lean's names silently identifies the first universe parameter
           of every polymorphic declaration in the corpus. Nothing errors; terms that should be
           incomparable become comparable, and the resulting library is checked against
           constraints weaker than the source's.
@guard     Catches `Universe::Var("u")` left UNBOUND in a non-definition. `add_declaration`
           (context.rs:434) and `add_inductive` (context.rs:253) accept any `Term` as a type with
           no universe-parameter list, so `axiom f : Sort u → Sort u` is registered and every
           later use of it carries a level variable that no `Const` can instantiate. The
           declaration is unusable in a way that only shows up as a conversion failure at the use
           site, attributed to the use site.
@guard     Catches `Sort u` ⟼ `Universe::Type(0)` ("u is unknown, default it"). `Sort u` in Lean
           RANGES over `Prop` as well, and LOGOS's own decision core encodes that
           (term.rs:270-272): `Type 0 ≤ u` is FALSE precisely because `u` may be `Prop`. So
           defaulting to `Type 0` is not a conservative approximation in either direction — it is
           both too strong (it assumes `u ≠ Prop`, which loses the `Prop` instance) and too weak
           (it monomorphises, so a second instantiation collapses onto the first, T01-007).
@guard     Catches `Sort u` ⟼ `Type u`. `Universe::Type` takes a concrete `u32` (term.rs:177);
           there is no `Type(<expr>)` form at all. `Sort u` must be `Sort(Var("u"))`, and a
           translator that needs `Type u` specifically (i.e. `u ≥ 1`) has no way to express the
           constraint — LOGOS has no level-constraint store.
@guard     Catches the arity trap at the use site. `infer_type` on `Const` errors when
           `params.len() != levels.len()` (type_checker.rs:75-82), but `reduce_step` on `Const`
           (reduction.rs:82-91) SILENTLY returns the term unreduced on the same mismatch. So a
           wrong-arity reference is a hard error in a checking position and a silent opacity in a
           reduction position — the same bug reported two incompatible ways.
@oracle    Every emitted `Universe::Var(n)` lies inside the type or body of a declaration
           registered with `add_universe_poly` whose `params` contains `n`; the names `n` are
           freshened per declaration so that no two declarations share a level-variable name
           unless the Lean source genuinely shares one; every reference is a
           `Term::Const { name, levels }` with `levels.len() == params.len()`; and a loss record
           with category LEVEL_BINDING_SCOPE lists every Lean declaration with universe
           parameters that is not a definition.
-/

namespace T01_003

/-- The canonical level-variable definition. `Sort u` ranges over `Prop` too, which is why it is
    `Sort` and not `Type`. This one CAN be bound: `add_universe_poly("polyId", ["u"], …)`. -/
def polyId.{u} {α : Sort u} (a : α) : α := a

/-- Instantiated at three genuinely different levels, including `Prop`. Each use site needs its
    own `Term::Const { name: "polyId", levels: [_] }`. -/
def atProp (h : True) : True := polyId h
def atTypeZero (n : Nat) : Nat := polyId n
def atTypeOne (α : Type) : Type := polyId α

/-- `Type u` rather than `Sort u`: the level is constrained to be at least 1 in Lean. LOGOS has
    no level-constraint store, so the constraint is simply not representable — the emitted level
    is `Universe::Var("u")`, which `level_leq` treats as ranging over `Prop` as well. -/
def polyTypeId.{u} (α : Type u) : Type u := α

/-- TWO level parameters on one definition. `add_universe_poly`'s `params` is a `Vec<String>`
    and `Term::Const`'s `levels` is positional (type_checker.rs:84-85 zips them) — so the ORDER
    is load-bearing and a swap is silent when both arguments are variables. -/
def polyPair.{u, v} (α : Type u) (β : Type v) : Type (max u v) := α → β

/-- The same two parameters used ASYMMETRICALLY, so a swap is detectable: `v` appears alone in
    the result. -/
def polySecond.{u, v} (_α : Type u) (β : Type v) : Type v := β

/-- A level parameter appearing ONLY in the body, not in the type — Lean still binds it on the
    declaration. -/
def polyInBodyOnly.{u} (n : Nat) : Nat :=
  let _f : Sort u → Sort u := fun x => x
  n

-- `universe` command: the parameter is declared once and shared by the declarations that
-- mention it. Lean binds it per-declaration regardless; the sharing is a source-level
-- convenience, and a translator that preserves the SHARING (one `Universe::Var("w")` across
-- several LOGOS declarations) creates exactly the cross-declaration identification this case
-- guards against.
universe w

def usesW1 (α : Sort w) : Sort w := α
def usesW2 (α : Sort w) : Sort w := α

/-- An AXIOM with a universe parameter — the case that cannot be bound at all, because
    `add_declaration` (context.rs:434) takes no parameter list. -/
axiom polyAxiom.{u} : Sort u → Prop

/-- A use of that axiom at a concrete level. The reference has nowhere to put the level, so
    either it is a `Global` (losing the instantiation) or a `Const` against a name that
    `get_universe_poly` does not know (type_checker.rs:70-72: `UnboundVariable`). -/
def usesPolyAxiom : Prop := polyAxiom Nat

/-- The same axiom at a `Prop` level, so the two instantiations genuinely differ. -/
def usesPolyAxiomAtProp : Prop := polyAxiom True

end T01_003
