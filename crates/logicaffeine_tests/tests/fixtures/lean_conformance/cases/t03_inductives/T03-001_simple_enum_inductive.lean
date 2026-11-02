/-!
@case      T03-001
@tier      3 · Inductives and structures
@title     Simple enum inductive — registration order *is* the match order
@lean      inductive with nullary constructors, `Lean.InductiveVal`, `casesOn`
@target    Context::add_inductive(name, Sort(Type 0)) followed by one
           Context::add_constructor_checked(ctor, name, Global(name)) per constructor, in
           Lean's declaration order
@expect    translate
@evidence  kernel/src/context.rs:253-255 (`add_inductive` records `name -> sort`; it is
           **unchecked** — a bare `HashMap::insert`);
           kernel/src/context.rs:419-428 (`add_constructor` appends to `constructor_order`,
           "HashMap doesn't preserve insertion order, so we track it explicitly");
           kernel/src/context.rs:498-515 (`get_constructors` returns them *in registration
           order*); kernel/src/type_checker.rs:253-266 (a `Match` must supply exactly
           `constructors.len()` cases, and case *i* is checked against constructor *i* of that
           order — matching is **positional**, there are no patterns and no tags);
           kernel/src/context.rs:545-560 (`add_constructor_checked` = positivity +
           `check_constructor_universes`, the only checked registration path)
@guard     Catches registering constructors in any order other than Lean's. LOGOS `Match` has
           **no patterns**: `cases[i]` is the branch for `get_constructors(ind)[i]`. Permuting
           two nullary constructors of the same inductive produces a `Match` that typechecks
           perfectly — all the branches have the same type — and computes the wrong answer for
           every input. This is the cheapest possible catastrophic mistranslation, and the
           kernel cannot detect it in principle.
@guard     Catches using `add_inductive` + `add_constructor` (both **unchecked**,
           context.rs:253,419) instead of `add_constructor_checked` (context.rs:545). For a
           simple enum the checks happen to pass, so the difference is invisible *here* — which
           is exactly why the habit must be established here rather than at T03-015, where the
           unchecked path admits a paradox.
@guard     Catches emitting the wrong sort. Lean's `inductive Colour where | red | green` lives
           in `Type 0`; a translator that writes `Sort(Prop)` because the type has no fields
           makes it a proposition, and LOGOS's large-elimination gate
           (type_checker.rs:277-290) then refuses to let any `Match` on it return a `Type`
           value — an error at the *use* site, far from the declaration.
@guard     Catches inventing `Colour.rec`. LOGOS does **not** create a recursor at declaration
           time; `recursor::derive_recursor` (recursor.rs:36) synthesises one and the caller
           must `add_definition` it. If the Lean source mentions `Colour.rec` (or `casesOn`),
           that global does not exist until the translator asks for it. See T03-021.
@oracle    `get_constructors(name)` returns exactly Lean's constructor names in Lean's order;
           the inductive's sort is `Type 0`; a `Match` whose `cases` are built in that order
           reduces on each constructor to the intended branch.
-/

namespace T03_001

/-- Three nullary constructors. Order is the whole content of the declaration. -/
inductive Colour where
  | red
  | green
  | blue

/-- A `match` whose branches are pairwise distinguishable, so a permuted registration order
    is observable rather than merely theoretical. -/
def code : Colour → Nat
  | Colour.red => 0
  | Colour.green => 1
  | Colour.blue => 2

example : code Colour.red = 0 := rfl
example : code Colour.green = 1 := rfl
example : code Colour.blue = 2 := rfl

/-- The permutation trap made explicit: a translator that registered `green` before `red`
    would satisfy every typing obligation of `code` and still fail this. -/
example : code Colour.red ≠ code Colour.green := by decide

/-- Source order and *alphabetical* order differ here, which is the accident a translator
    that iterates a `HashMap` of constructors will hit. -/
inductive Weekday where
  | wednesday
  | monday
  | friday

def dayIndex : Weekday → Nat
  | Weekday.wednesday => 0
  | Weekday.monday => 1
  | Weekday.friday => 2

example : dayIndex Weekday.monday = 1 := rfl

/-- A single-constructor enum. It is **not** a structure — `add_structure` would additionally
    register `StructInfo` and turn on definitional eta (context.rs:365-368), which Lean does
    *not* grant to a general one-constructor inductive that was not declared as a structure. -/
inductive OnlyOne where
  | sole

example : (OnlyOne.sole : OnlyOne) = OnlyOne.sole := rfl

/-- A two-constructor enum whose branches return different *types* would not compile; here
    the branches return the same type, which is why the order bug survives typechecking. -/
def isWarm : Colour → Bool
  | Colour.red => true
  | Colour.green => false
  | Colour.blue => false

example : isWarm Colour.red = true := rfl
example : isWarm Colour.blue = false := rfl

end T03_001
