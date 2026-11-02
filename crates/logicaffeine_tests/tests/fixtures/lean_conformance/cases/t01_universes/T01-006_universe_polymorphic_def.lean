/-!
@case      T01-006
@tier      1 · Universes
@title     Universe-polymorphic DEFINITION — the one construct LOGOS genuinely supports
@lean      def with universe parameters, `def f.{u}`, Expr.const with a non-empty level list
@expect    translate
@target    `Context::add_universe_poly(name, params: Vec<String>, ty: Term, body: Term)`
           (kernel/src/context.rs:218) for the declaration, and
           `Term::Const { name, levels: Vec<Universe> }` (kernel/src/term.rs:458) at EVERY
           reference — one `levels` entry per parameter, in `params` order.
@evidence  kernel/src/context.rs:218-225 (`add_universe_poly` — the only universe-parameter
           binder in the whole API, and its doc says "universe-polymorphic DEFINITION … A
           `Term::Const { name, levels }` reference later instantiates `params` with `levels`");
           kernel/src/context.rs:227-229 (`get_universe_poly` returns `(params, ty, body)`);
           kernel/src/type_checker.rs:69-85 (`Const` inference: `get_universe_poly` or
           `UnboundVariable`; then `params.len() != levels.len()` is a `CertificationError`; then
           `instantiate_universes(ty, subst)`);
           kernel/src/term.rs:256-265 (`Universe::substitute` — the substitution
           `instantiate_universes` performs, which maps `Var(v)` through the table and recurses
           through `Succ`/`Max`/`IMax` using `succ`/`max`/`imax` so the result is collapsed);
           kernel/src/reduction.rs:82-91 (delta for `Const`: unfold the body with the levels
           substituted — but ONLY `if params.len() == levels.len()`; otherwise `term.clone()`,
           SILENTLY unreduced);
           kernel/src/context.rs:449 (`get_global` — inductives → constructors → definitions →
           declarations — does NOT consult `universe_polys`, so a `Global` naming a
           universe-polymorphic definition is `UnboundVariable`)
@guard     Catches referencing a universe-polymorphic definition with `Term::Global`. The two
           maps are disjoint (context.rs:449 versus :227), so this is `UnboundVariable` at
           `infer_type` — loud, and therefore the easy half.
@guard     Catches the QUIET half: a `Term::Const` whose `levels.len()` does not match
           `params.len()`. `infer_type` reports `CertificationError` (type_checker.rs:75-82), but
           `reduce_step` (reduction.rs:83-90) hits the same mismatch and returns the term
           UNCHANGED with no diagnostic. So a wrong-arity reference behaves as a hard error in a
           checking position and as a SILENT OPAQUE CONSTANT in a reduction position. The visible
           symptom is that a `rfl` which should hold by delta-unfolding does not — attributed to
           the theorem, not to the reference. A translator that drops a level it believes is
           "determined by the context" produces exactly this.
@guard     Catches getting the `levels` ORDER wrong. `type_checker.rs:84-85` zips `params` with
           `levels` positionally. For `polyPair.{u, v}` a swap is undetectable whenever the level
           expression is symmetric (`max u v`, T01-004) and produces a silently wrong
           instantiation where it is not. The order must come from Lean's
           `ConstantInfo.levelParams`, which is itself the auto-bound order (T01-011), not from
           order of appearance in the type.
@guard     Catches instantiating `levels` with the level VARIABLE of the ENCLOSING declaration
           without renaming. Level-variable names are flat `String`s (term.rs:179) compared by
           name through `to_lnat` (term.rs:271), so a caller whose own parameter is also called
           `u` will pass `Var("u")` meaning ITS `u`, and `instantiate_universes` will substitute
           it for the callee's `u` — which happens to be correct here and is correct by accident.
           Where the caller's `u` should map to the callee's `v`, the same accident is a silent
           mis-instantiation. See T01-003.
@guard     Catches emitting the polymorphic definition once per instantiation instead of once
           with parameters. That is the monomorphisation path (T01-008); it is the required
           fallback for INDUCTIVES (T01-007), and it is a needless, lossy choice for a
           definition, which LOGOS supports natively. A translator that monomorphises
           definitions because its inductive path had to will produce a development in which
           `polyId@0` and `polyId@1` are unrelated globals and no universe-polymorphic lemma
           about `polyId` can be stated.
@guard     Catches forgetting that `add_universe_poly` registers a DEFINITION — delta-transparent
           (reduction.rs:82-91 unfolds it on sight). There is no universe-polymorphic AXIOM
           form: `add_declaration` (context.rs:434) takes no parameter list. A polymorphic
           `axiom` or `opaque` in Lean therefore has no target at all (T01-003, T01-012).
@oracle    Every Lean declaration with non-empty `levelParams` that is a `def`/`theorem` appears
           via `add_universe_poly` with `params` equal to `levelParams` in order; every reference
           to it is a `Term::Const` with `levels.len() == params.len()`; `infer_type` on each
           reference returns `instantiate_universes(ty, subst)` matching the translation of
           Lean's instantiated type; and `normalize` on each reference actually UNFOLDS — the
           test that distinguishes a correct `Const` from a silently-opaque wrong-arity one.
-/

namespace T01_006

/-- One parameter, `Sort u` so it ranges over `Prop` too. -/
def polyId.{u} {α : Sort u} (a : α) : α := a

/-- Three instantiations, at `Prop`, `Type 0` and `Type 1`. Each needs its own
    `Term::Const { levels: [_] }`; a single `Global` cannot serve all three. -/
theorem polyIdAtProp (h : True) : True := polyId h
def polyIdAtType0 (n : Nat) : Nat := polyId n
def polyIdAtType1 (α : Type) : Type := polyId α

/-- Delta-transparency: `polyId` must UNFOLD under `normalize` for this to hold. If the
    reference were a wrong-arity `Const`, `reduce_step` (reduction.rs:83-90) would return it
    unreduced and this `rfl` would silently fail. -/
example : polyId (3 : Nat) = 3 := rfl
example : polyIdAtType0 3 = 3 := rfl

/-- TWO parameters, used SYMMETRICALLY — a swapped `levels` vector is undetectable here. -/
def polyArrow.{u, v} (α : Type u) (β : Type v) : Type (max u v) := α → β

/-- The same two parameters used ASYMMETRICALLY, so a swap IS detectable. Pair these two in any
    conformance run. -/
def polySecond.{u, v} (_α : Type u) (β : Type v) : Type v := β

def secondAt01 : Type 1 := polySecond Nat Type
def secondAt10 : Type := polySecond Type Nat

/-- A polymorphic definition whose body MENTIONS another polymorphic definition, so the levels
    must be threaded through: the inner `Const`'s `levels` are the outer's parameters, not
    fresh variables. -/
def polyCompose.{u} {α : Sort u} (a : α) : α := polyId (polyId a)

/-- A polymorphic THEOREM. Lean treats `theorem` and `def` identically for universe parameters;
    LOGOS has no separate theorem form (context.rs:442, LOGOS_FACTS) so this is another
    `add_universe_poly`. -/
theorem polyEqRefl.{u} {α : Sort u} (a : α) : a = a := rfl

/-- A polymorphic definition at a level expression that is NOT a bare variable: the parameter
    appears under a `succ`. `instantiate_universes` (term.rs:256-265) must recurse through it
    with `succ`, not substitute structurally. -/
def polyAtSucc.{u} (α : Type u) : Type (u + 1) := Type u → α

/-- Zero universe parameters — the control. This one is an ordinary `add_definition`
    (context.rs:442) and a `Term::Global`, and a translator must not promote it. -/
def monoId (n : Nat) : Nat := n

/-- A polymorphic definition used INSIDE a monomorphic one, so the boundary between
    `Const`-with-levels and `Global` is crossed in a single declaration. -/
def mixedReference : Nat := monoId (polyId 5)

end T01_006
