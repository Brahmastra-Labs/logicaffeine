/-!
@case      T00-011
@tier      0 · Kernel term layer
@title     `Expr.proj` on library structures — `Prod`, `Sigma`, `Subtype` are not in LOGOS at all
@lean      Expr.proj, Prod.fst/.snd, Sigma.fst/.snd, Subtype.val, `.1` / `.2`
@expect    degrade
@target    For each Lean structure the source projects out of, the translator must FIRST emit a
           `Context::add_structure(name, params, fields)` declaration, and then lower each
           `Expr.proj S i e` to the generated field DEFINITION applied to the structure's
           parameters and then the receiver:
           `p.1` with `p : Prod α β` ⟼ `App(App(App(Global("Prod_fst"), α), β), p)`.
@evidence  kernel/src/term.rs:443-545 (`Term` has no `Proj` variant — `Expr.proj` has no direct
           counterpart); kernel/src/context.rs:271-369 (`add_structure` desugars a record into an
           indexed inductive, a `{name}_mk` constructor, and one ordinary DEFINITION per field
           whose body is a `Term::Match`); kernel/src/context.rs:295-301 (the generated inductive
           is `pis(params, Term::Sort(Universe::Type(0)))` — the sort is HARD-CODED to `Type 0`);
           kernel/src/elaborate.rs:1262-1299 (`elaborate_dot` builds `{hname}_{field}` applied to
           the receiver's type arguments FIRST and the receiver LAST);
           kernel/src/prelude.rs and LOGOS_FACTS ("Absent entirely: `Subtype`, `Sigma`, `PSigma`,
           `Option`, `List`, `Prod`, `Sum`, `Fin`, `Char`, `Array`, `String` operations, `UInt*`,
           `HEq`, `Setoid`, `Quotient`");
           kernel/src/type_checker.rs:1391-1417 (`try_structure_eta`, reachable only from
           `def_eq_normalized` at :1473-1478, and only for names in the `structures` registry)
@loss      PROJECTION_PRIMITIVITY. Lean's `.proj` is a kernel primitive reducing in ONE iota
           step; the LOGOS encoding is a delta-unfold to a `Match`, so reduction costs two steps
           and counts against `normalize`'s 10000-step fuel (reduction.rs:52-69), and eta holds
           only inside `def_eq`, never under `normalize`.
@loss      LIBRARY_STRUCTURE_SYNTHESIS. `Prod`, `Sigma` and `Subtype` are not declared anywhere
           in LOGOS, so the translator must MANUFACTURE them. Their Lean definitions are
           universe-polymorphic (`Prod : Type u → Type v → Type (max u v)`), and `add_structure`
           pins the result to `Type 0` (context.rs:300) — so the synthesised `Prod` is only the
           `Type 0` instance. Every use at a higher level is outside the fragment (T01-007) and
           must be rejected, not silently reused.
@loss      PROOF_IRRELEVANT_FIELD. `Subtype`'s second field is a PROOF. `add_structure` builds
           the inductive at `Type 0`, so `{n : Nat // n > 0}` becomes a `Type 0` record with a
           `Prop`-typed field — legal, but its `Subtype_property` projection is a large
           elimination out of a `Prop`-sorted field, and the derived `Match` motive target is
           pinned to `Type 0` by recursor.rs:64-71. Subtype equality by proof irrelevance
           (`Subtype.ext`) has no counterpart and must be re-derived.
@guard     Catches assuming `Prod`/`Sigma`/`Subtype` EXIST in the LOGOS prelude and emitting
           `Global("Prod_fst")` without declaring anything. `get_global` (context.rs:449) then
           fails with `UnboundVariable` — the loud case — but a translator that pre-seeds a
           partial prelude gets the far worse case: `Prod` declared with `add_inductive`
           (UNCHECKED, context.rs:253) and no constructors, so every `Match` on it has
           `cases.len() == 0 == ctor count` and vacuously passes the positional check at
           type_checker.rs:254-260. A projection out of it then returns whatever the motive says.
@guard     Catches emitting the projection with the receiver FIRST. `elaborate_dot`
           (elaborate.rs:1290-1296) applies the structure's parameters before the receiver, so
           `Prod_fst p` is an arity error at best and, when `α` and the receiver happen to have
           compatible types (`Prod Type Type`), a silently wrong term.
@guard     Catches translating `Expr.proj` as an opaque AXIOM `Prod_fst : Prod α β → α` via
           `add_declaration` (context.rs:434). It typechecks and it is consistent — and it makes
           `(a, b).1 = a` UNPROVABLE, so every downstream `rfl` about a pair fails for a reason
           the translator cannot attribute.
@guard     Catches translating `Sigma` as `Prod`. `Sigma`'s second field's TYPE mentions the
           first field's VALUE; `add_structure` supports that (context.rs:307-320 rewrites
           earlier field references), but a translator that erases the dependency emits a
           non-dependent pair whose projections have the wrong types and whose `Match` motive is
           constant — which still typechecks for the non-dependent instances in a test suite.
@oracle    Each projected Lean structure has a corresponding `add_structure` in the emitted
           context; each `Expr.proj S i e` becomes `Global("S_field_i")` applied to the
           receiver's type arguments then the receiver; `def_eq(⟨a,b⟩.1, a)` holds; a loss
           record carries PROJECTION_PRIMITIVITY and LIBRARY_STRUCTURE_SYNTHESIS; and any
           projection at a universe level above `Type 0` is REJECTED rather than emitted.
-/

namespace T00_011

/-- `Prod` projections. `.1`/`.2` elaborate to `Expr.proj ``Prod 0/1`. -/
def pfst (p : Nat × Bool) : Nat := p.1
def psnd (p : Nat × Bool) : Bool := p.2

example : (Prod.mk 3 true).1 = 3 := rfl
example : (Prod.mk 3 true).2 = true := rfl

/-- Structure eta for `Prod`: Lean proves this definitionally. In LOGOS it holds only through
    `try_structure_eta` inside `def_eq`, never under `normalize`. -/
example (p : Nat × Bool) : Prod.mk p.1 p.2 = p := rfl

/-- A NON-dependent `Sigma`, which a translator can get away with treating as a `Prod`. -/
def snonDep (s : Sigma (fun _ : Nat => Bool)) : Nat := s.1

/-- A genuinely DEPENDENT `Sigma`: the second component's type mentions the first's value. This
    is the instance that breaks a `Prod`-shaped translation. -/
def sigmaDep : Sigma (fun n : Nat => Fin (n + 1)) := ⟨3, ⟨2, by omega⟩⟩
def sigmaFst (s : Sigma (fun n : Nat => Fin (n + 1))) : Nat := s.1
def sigmaSnd (s : Sigma (fun n : Nat => Fin (n + 1))) : Fin (s.1 + 1) := s.2

/-- `Subtype`: the second field is a PROOF, so the projection out of it is a large elimination
    out of a `Prop`-typed field. -/
def positives := {n : Nat // n > 0}
def subVal (s : positives) : Nat := s.1
def subProp (s : positives) : s.1 > 0 := s.2
def three : positives := ⟨3, by omega⟩

example : three.1 = 3 := rfl

/-- Nested projection, so the encoded form is a `Match` inside a `Match` and the fuel cost
    compounds. -/
def nestedPair : (Nat × Bool) × Nat := ⟨⟨1, true⟩, 2⟩
def deepProj : Nat := nestedPair.1.1

example : deepProj = 1 := rfl

end T00_011
