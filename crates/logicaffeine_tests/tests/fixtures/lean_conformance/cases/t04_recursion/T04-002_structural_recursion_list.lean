/-!
@case      T04-002
@tier      4 · Recursion and pattern matching
@title     Structural recursion over a list — a *parameterised* inductive as the guard's
           structural type
@lean      def by structural recursion on List, `| [] => .. | x :: xs => ..`
@expect    translate
@target    A locally-declared list inductive (Lean's `List` is NOT in the LOGOS prelude) plus
           Term::Fix { name: "len", body: Lambda{"l", App(Global("TList"), Global("Nat")),
             Match { discriminant: Var("l"), motive: .., cases: [<TNil>, <TCons: λh.λt. Succ (len t)>] }}}
@evidence  kernel/src/prelude.rs:606-613 (the prelude's only list types are the monomorphic
           `EList`/`ENil`/`ECons` over `Entity` and the parametric `TList`/`TNil`/`TCons`;
           Lean's `List` is absent — see LOGOS_FACTS "Absent entirely");
           kernel/src/termination.rs:166-174 (`extract_inductive_name` peels an application
           spine, so `TList A` is recognised as the inductive `TList`);
           kernel/src/termination.rs:403-440 (`check_match_cases_guarded` counts the
           constructor's Π-telescope with `count_pi_params` and marks *every* λ-binder of the
           case as smaller — including the PARAMETER binders);
           kernel/src/type_checker.rs:255-259 (positional case coverage)
@guard     Catches the translator marking the wrong λ-binders as the constructor's fields.
           `TCons : Π(A:Type 0). A → TList A → TList A` has THREE Π binders, so
           `count_pi_params` is 3 and `extract_lambda_params` (termination.rs:410-415) peels
           three λs from the case. A translator that emits the `TCons` case as `λh.λt. …` (two
           binders, parameter dropped because Lean's `List.cons` shows the parameter as an
           implicit) has its case arity disagree with the constructor's, and the third peel
           silently reaches into the case *body* — so a variable that is not a constructor
           field gets marked structurally smaller, and a non-decreasing recursive call on it
           is accepted. That is a soundness hole, not a type error.
@guard     Catches emitting Lean's `List` as `Global("List")` and assuming the prelude
           provides it. `Context::get_global` (context.rs:449) returns `None`, so
           `extract_inductive_name` fails and the `Match` is rejected with `NotAnInductive` —
           or, worse, a translator that "helpfully" calls the UNCHECKED `add_inductive` +
           `add_constructor` (context.rs:253,419) registers a list type with no positivity or
           universe check at all.
@guard     Catches confusing `List α` with the prelude's `EList`. `EList` is monomorphic over
           `Entity` (prelude.rs:561-577); silently retargeting `List Nat` at it makes every
           element `Entity`-typed, and because LOGOS is cumulative many downstream terms still
           typecheck.
@oracle    The emitted `Fix` typechecks against the translated Lean type, and
           `normalize(len (TCons Nat 1 (TCons Nat 2 (TNil Nat))))` is `Lit(Nat 2)`.
-/

namespace T04_002

/-- A local re-declaration standing in for the LOGOS prelude's parametric `TList`. Lean's own
    `List` has no LOGOS counterpart, so the corpus never assumes it translates for free. -/
inductive TList (α : Type) where
  | nil  : TList α
  | cons : α → TList α → TList α

/-- Structural recursion on the parameterised inductive. -/
def len {α : Type} : TList α → Nat
  | .nil => 0
  | .cons _ t => len t + 1

/-- Two recursive calls in one branch — both must be guarded independently. -/
def app {α : Type} : TList α → TList α → TList α
  | .nil, ys => ys
  | .cons x xs, ys => .cons x (app xs ys)

/-- The recursive call appears under another application, so the guard must peel the spine
    (`termination.rs:206-214`) to find the head. -/
def sumList : TList Nat → Nat
  | .nil => 0
  | .cons x t => x + sumList t

/-- Recursion on Lean's own `List`, to pin what the translator must *not* silently retarget at
    the prelude's `EList`. -/
def leanLen : List Nat → Nat
  | [] => 0
  | _ :: t => leanLen t + 1

example : len (TList.cons 1 (TList.cons 2 TList.nil)) = 2 := rfl
example : sumList (TList.cons 1 (TList.cons 2 TList.nil)) = 3 := rfl
example : leanLen [1, 2, 3] = 3 := rfl

end T04_002
