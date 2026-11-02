/-!
@case      T04-012
@tier      4 · Recursion and pattern matching
@title     `match h : e with` — the equation hypothesis has no carrier in LOGOS `Match`
@lean      match h : e with, `Expr` matchers whose motive is generalised over an `Eq` proof
@expect    reject
@target    (none — no sound translation)
@evidence  kernel/src/term.rs:487-492 (`Term::Match { discriminant, motive, cases }` — three
           fields; there is no hypothesis binder, no per-case proof argument, and no place to
           record that a case body is under an assumption);
           kernel/src/type_checker.rs:262-265 and :778-830 (`compute_case_type`: a case's
           expected type is exactly the constructor's remaining Π-telescope ending in
           `motive (ctor args…)` — the case binds the CONSTRUCTOR'S FIELDS and nothing else, so
           an extra `h : Eq I e (ctor …)` binder makes the case's arity wrong);
           kernel/src/type_checker.rs:612-678 (`infer_indexed_match` — same one-case-per-
           constructor discipline, and there is **no index unifier**: `:632-638` demands a case
           for every constructor even when its indices cannot match);
           kernel/src/prelude.rs:1440-1460 (`Eq` is an indexed inductive with two parameters
           and one index — it exists, but nothing in the `Match` rule ever builds an `Eq` proof
           and hands it to a case)
@diag      E_MATCH_EQUATION_HYPOTHESIS — must name the function, name the hypothesis binder,
           and state that LOGOS's `Match` gives each case only the matched constructor's
           fields. Must NOT emit a `Match`.
@guard     The tempting wrong move is to **drop the hypothesis and emit the plain match**,
           because `h` is often unused. That is exactly backwards: when `h` IS used — as in
           `findSub` below, where the returned subtype value is `⟨y, h⟩` — dropping it leaves a
           free `Term::Var("h")` in the case body. LOGOS's representation is NAMED
           (term.rs:450), so that free variable is not a dangling de Bruijn index the kernel
           will notice; it resolves against whatever `h` is in scope. If an enclosing binder is
           also called `h` (very common — `h` is the universal hypothesis name), the term
           typechecks with a completely unrelated proof substituted in. That is a proof-library
           corruption, not a crash.
@guard     Catches "generalise the motive and pass `refl`". The sound-looking encoding is to
           use the motive `λx. Eq I e x → C` and apply the resulting match to `refl I e`. It
           does not work in LOGOS: the motive's domain must be the discriminant's type
           (type_checker.rs:213-219), which it still is, but the *cases* now have to produce
           functions `Eq I e (ctor …) → C`, and the discriminant `e` is generalised in the
           motive while the `Eq`'s left-hand side is not. There is no index unifier
           (type_checker.rs:612-678) to relate the two, so what the translator gets back is
           `Eq I e disc → C` applied to `refl I e` — which only typechecks when
           `disc` is definitionally `e`, i.e. only in the cases where the hypothesis is
           useless. Emitting it anyway produces a term that typechecks on the trivial
           instances and fails, opaquely, on exactly the ones that motivated the feature.
@guard     Catches "replace `h` with a `Term::Hole`". `types_equal` treats `Hole` as matching
           **anything** (type_checker.rs:1594-1597) and that leniency leaks into `is_subtype`
           (`:1314`), so the hole is accepted and the emitted declaration is a hole-carrying
           term in the library.
@guard     Catches "replace `h` with an axiom `Eq I e (ctor …)` via `add_declaration`"
           (context.rs:434). That axiom is FALSE for every other constructor of the same
           inductive, so the library becomes inconsistent — and `add_declaration` performs no
           check at all.
@guard     Catches over-rejection: a `match h : e with` whose `h` is provably unused in every
           case body is still `reject` under this case's verdict, because the translator cannot
           decide "unused" reliably after the equation compiler has run. A translator may
           degrade such a match only on a separate, explicitly-recorded path that proves `h`
           does not occur in the elaborated `Expr`.
@oracle    Translator exits non-zero with E_MATCH_EQUATION_HYPOTHESIS for each definition
           below and writes no declaration into the LOGOS context.
-/

namespace T04_012

/-- The canonical load-bearing use: `h : p y = true` is exactly the proof the subtype needs. -/
def findSub (p : Nat → Bool) : List Nat → Option { x : Nat // p x = true }
  | [] => none
  | y :: t => match h : p y with
    | true  => some ⟨y, h⟩
    | false => findSub p t

/-- The scrutinee is an *expression*, not a binder, and `h` is the only thing that proves the
    returned subtype's property. -/
def firstEven : List Nat → Option { z : Nat // z % 2 = 0 }
  | [] => none
  | y :: t => match h : y % 2 with
    | 0 => some ⟨y, h⟩
    | _ + 1 => firstEven t

/-- The hypothesis carries a *decision procedure's* result into a proof — the standard
    `Bool`-to-`Prop` bridge, which is precisely what has no LOGOS carrier. -/
def firstPos : List Nat → Option { z : Nat // 0 < z }
  | [] => none
  | y :: t => match h : Nat.ble 1 y with
    | true  => some ⟨y, Nat.le_of_ble_eq_true h⟩
    | false => firstPos t

/-- A `match h : e with` whose `h` is genuinely unused — still `reject`, because "unused" is
    not decidable from the translator's side after compilation. -/
def parity (n : Nat) : Bool :=
  match _h : n % 2 with
  | 0 => true
  | _ + 1 => false

end T04_012
