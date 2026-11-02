/-!
@case      T05-010
@tier      5 · Elaboration surface
@title     Dot notation and generalized field notation — `elaborate_dot` and the `H_field` convention
@lean      x.f dot notation, generalized field notation (`xs.map f`), `.ctor` anonymous
           constructor dot notation
@expect    translate
@target    `H_field <the receiver's type arguments…> <receiver>` — the LOGOS convention
           implemented by `elaborate::elaborate_dot`, e.g. `p.fst` with `p : Pair A B` becomes
           `Pair_fst A B p`.
@evidence  kernel/src/elaborate.rs:1262-1299 (`elaborate_dot(ctx, mctx, receiver, field)`:
           the receiver's type is normalised, its head must be a `Term::Global` `H`, the
           projection name is `format!("{hname}_{field}")`, and the result is
           `H_field params… receiver` — the parameters are read off the RECEIVER'S TYPE and
           applied BEFORE the receiver);
           kernel/src/elaborate.rs:1284-1289 (an unknown projection is a hard error:
           "dot notation: no projection `{proj}` for field `{field}` of `{hname}`");
           kernel/src/elaborate.rs:1297 (`crate::infer_type(ctx, &applied)?` — the result is
           certified, so an arity slip is caught here rather than later);
           kernel/src/context.rs:271-369 (`add_structure` is what creates `{name}_{field}` as
           an ordinary *definition* whose body is a `Term::Match`);
           kernel/src/term.rs:443-545 (there is **no `Term::Proj`** — dot notation has no
           term-level counterpart, only this encoding)
@guard     **Translate the post-elaboration `Expr`, never the surface `Syntax`.** Lean's dot
           notation is resolved against the receiver's *elaborated* type, and generalized field
           notation (`xs.map f`) resolves to a function in the receiver's type's namespace with
           the receiver inserted at the first argument of that type — a rule involving
           namespace search and argument-position search. After elaboration the `Expr` is a
           plain application of a fully-resolved constant. A translator on `Syntax` has to
           reimplement both searches; one on `Expr` only has to map the resolved constant's
           flat name.
@guard     Catches the PARAMETERS-FIRST convention. `elaborate_dot` applies the structure's
           parameters, taken from the receiver's type, before the receiver
           (elaborate.rs:1290-1296). A translator that emits `Pair_fst p` — natural, since
           Lean's `p.fst` looks unary — produces a term whose first argument is checked against
           the parameter `α : Type 0`. When the receiver's own type is a `Type 0` (a `Pair Type
           Type`, say) that can typecheck.
@guard     Catches the flat-name convention. LOGOS keys are flat `String`s with no namespaces
           (LOGOS_FACTS; context.rs:419,442), and `elaborate_dot` looks up exactly
           `format!("{hname}_{field}")`. Lean's `Pair.fst` must therefore be emitted as
           `Pair_fst`, and a Lean declaration literally named `Pair_fst` collides with it —
           see T02-020.
@guard     Catches generalized field notation on a NON-structure. `xs.length` with
           `xs : List Nat` resolves to `List.length xs`, an ordinary function, not a
           projection. `elaborate_dot` would look for `List_length` and, if the translator
           registered `List.length` under that name, find it — but the receiver is inserted at
           the END of the parameter list by `elaborate_dot`, whereas Lean inserts it at the
           first argument whose type is headed by `List`. For `List.append` (`xs.append ys`)
           those are different positions, so the arguments are silently swapped.
@guard     Catches `.ctor` dot notation (`.nil`, `.cons a t`, `.red`). That is anonymous
           CONSTRUCTOR notation resolved against the expected type, not field access; it must
           become `Global("List_nil")`-style constructor application (T05-009), not a
           projection lookup.
@guard     Catches dot notation on a receiver whose type is a REDEX. `elaborate_dot`
           normalises the receiver's type first (elaborate.rs:1274); a translator that reads
           the head off the un-normalised type gets a `Lambda` or an `App` and hits the
           "not headed by an inductive" error, or worse, picks the wrong `H`.
@oracle    The emitted term is `Global("{H}_{field}")` applied to the receiver's type arguments
           and then the receiver, in that order; it typechecks; and `normalize` of the emitted
           projection of a literal constructor value equals the corresponding field.
-/

namespace T05_010

structure Pair (α β : Type) where
  fst : α
  snd : β

/-- Parameterised projection: `Pair_fst α β p`, parameters first. -/
def getFst {α β : Type} (p : Pair α β) : α := p.fst

example : (Pair.mk 3 true).fst = 3 := rfl
example : (Pair.mk 3 true).snd = true := rfl

structure Point where
  x : Nat
  y : Nat

/-- Non-parameterised projection: `Point_x p`. -/
example : ({ x := 1, y := 2 } : Point).x = 1 := rfl

/-- A method in the structure's namespace — dot notation resolves to it, and the receiver is
    the first argument whose type is headed by `Point`. -/
def Point.norm1 (p : Point) : Nat := p.x + p.y

example : ({ x := 1, y := 2 } : Point).norm1 = 3 := rfl

/-- The receiver is NOT the first argument: `Point.shift` takes a `Nat` first, and Lean still
    inserts the receiver at the `Point`-typed position. -/
def Point.shift (d : Nat) (p : Point) : Point := { x := p.x + d, y := p.y + d }

example : ({ x := 1, y := 2 } : Point).shift 10 = { x := 11, y := 12 } := rfl

/-- Generalized field notation on a non-structure inductive. -/
example : [1, 2, 3].length = 3 := rfl
example : [1, 2].append [3] = [1, 2, 3] := rfl
example : (5 : Nat).succ = 6 := rfl

/-- `.ctor` dot notation — anonymous CONSTRUCTOR resolution against the expected type, a
    different mechanism from field access. -/
inductive Color where
  | red | green | blue

def c : Color := .red

def firstOrRed : List Color → Color
  | [] => .red
  | x :: _ => x

/-- Chained dot notation: each step re-normalises the receiver's type. -/
example : (({ x := 1, y := 2 } : Point).shift 1).norm1 = 5 := rfl

/-- Dot notation whose receiver's type is a definitional alias — `elaborate_dot` normalises
    before reading the head. -/
def MyPoint : Type := Point

def mp : MyPoint := ({ x := 4, y := 5 } : Point)

example : (mp : Point).x = 4 := rfl

example : getFst (Pair.mk 7 false) = 7 := rfl

end T05_010
