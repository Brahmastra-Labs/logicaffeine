/-!
@case      T00-006
@tier      0 · Kernel term layer
@title     `Expr.app` — left-nested binary spines, partial application, argument order
@lean      Expr.app, `f a b c`, partial application, `Expr.getAppFn` / `getAppArgs`
@expect    translate
@target    Term::App(Box<Term>, Box<Term>) — kernel/src/term.rs:470. `f a b c` is
           `App(App(App(f, a), b), c)`: binary, LEFT-nested, head innermost.
@evidence  kernel/src/term.rs:470 (`App` is binary — there is no n-ary application node and no
           separate head/args representation); kernel/src/type_checker.rs:144-181 (`App`
           inference: infer the function's type, NORMALIZE it if it is not already a `Pi`, check
           the argument against `param_type`, then `substitute` the argument into `body_type`);
           kernel/src/type_checker.rs:176 (the result type is `substitute(body_type, param, arg)`
           — so argument order determines which binder each argument lands in);
           kernel/src/elaborate.rs:1267-1299 (`spine` decomposes a left-nested `App` chain —
           the canonical shape the rest of the elaborator assumes)
@guard     Catches right-nesting. `App(f, App(a, App(b, c)))` is a *well-formed* `Term` and will
           be reported as `NotAFunction` only if `a`'s type is not a `Pi` — but when the
           arguments are themselves functions (higher-order code, a motive, a fold), `a b c` is
           perfectly well-typed and the whole thing typechecks as `f (a b c)`. That is a
           completely different term with a plausible type, and no error is produced.
@guard     Catches reversing the argument order. `type_checker.rs:176` substitutes positionally;
           with two arguments of the same type (`f : Nat → Nat → Nat`) a swap is undetectable by
           the kernel and changes the value. The file below therefore includes a non-commutative
           function at two same-typed arguments.
@guard     Catches placing universe levels in the spine. For a universe-polymorphic head, the
           levels belong on the `Term::Const { name, levels }` NODE (term.rs:458), not as leading
           `App` arguments; a translator that models Lean's `Expr.const n [u]` as
           `App(Global(n), <level>)` produces a term whose head has the wrong arity and whose
           first "argument" is a sort. See T01-006.
@guard     Catches *eagerly beta-reducing* a partial application into a lambda in order to
           "normalise" it. `type_checker.rs:148-152` already normalizes the function's type when
           it is not a `Pi` head, so no such help is needed, and reducing changes the term the
           certificate records.
@guard     Catches dropping implicit arguments that Lean elaborated in. Lean's `Expr` spine is
           fully explicit; the `@`-form and the non-`@`-form produce the SAME `Expr`. A
           translator that re-derives the spine from surface syntax will emit too few arguments,
           and `infer_type` will then return a `Pi` where a saturated type was expected — a type
           that is still well-formed, so it propagates.
@oracle    `Expr.getAppFn` / `Expr.getAppNumArgs` on the Lean side and `elaborate::spine` on the
           LOGOS side agree on head and arity for every application in the file; `infer_type`
           of the emitted term equals the translation of Lean's inferred type; and a partially
           applied spine infers a `Pi` with exactly the remaining binders.
-/

namespace T00_006

/-- A NON-commutative function at two same-typed arguments: a swapped spine is silently wrong,
    not a type error. -/
def sub2 (a b : Nat) : Nat := a - b

def appliedInOrder : Nat := sub2 7 3   -- 4
def spineHead : Nat → Nat := sub2 7    -- partial: one `App`, infers `Nat → Nat`

/-- Five arguments: a five-deep left-nested `App` chain. -/
def f5 (a b c d e : Nat) : Nat := a + 2 * b + 3 * c + 4 * d + 5 * e

def saturated : Nat := f5 1 2 3 4 5
def partial3 : Nat → Nat → Nat := f5 1 2 3
def partial1 : Nat → Nat → Nat → Nat → Nat := f5 1

/-- Higher-order: the argument is itself applied. This is the shape where a right-nested spine
    typechecks and means something different — `applyTwice g 3` versus `applyTwice (g 3)`. -/
def applyTwice (g : Nat → Nat) (n : Nat) : Nat := g (g n)
def hoSpine : Nat := applyTwice (fun k => k + 1) 3

/-- Implicit arguments are present in the `Expr` spine even though the source omits them. Both
    of these elaborate to the *same* four-argument spine. -/
def constFn {α : Type} {β : Type} (a : α) (_b : β) : α := a
def implicitSpine : Nat := constFn 3 true
def explicitSpine : Nat := @constFn Nat Bool 3 true

/-- A dependent spine: the second argument's type mentions the first, so the positional
    `substitute` at type_checker.rs:176 is load-bearing. -/
def dependentApply (n : Nat) (v : Fin n) : Nat := v.val
def dependentSpine : Nat := dependentApply 5 ⟨2, by omega⟩

/-- A universe-polymorphic head applied to a type argument. The level rides on the head node,
    the type rides in the spine — two different channels that look alike in Lean's `Expr`. -/
def polyHeadSpine : List Nat := List.cons 1 List.nil

end T00_006
