/-!
@case      T11-013
@tier      11 · Adversarial and soundness traps
@title     `add_inductive` / `add_constructor` are UNCHECKED — the declaration API will build an
           inconsistent environment on request
@lean      strict positivity violation, universe-inconsistent constructor, an inductive whose
           declaration Lean refuses
@expect    reject
@target    (none — and, decisively, the translator must use `add_constructor_checked` for every
           constructor it ever emits, including the ones it emits for types it merely needs as
           support)
@evidence  kernel/src/context.rs:246-256 (`add_inductive(name, sort)` — the whole body is
           `Arc::make_mut(&mut self.inductives).insert(name.to_string(), sort)`. No positivity
           check, no universe check, no check that the name is fresh, no check that `sort` is a
           sort); kernel/src/context.rs:413-431 (`add_constructor(name, inductive, ty)` —
           inserts into `constructors` and appends to `constructor_order`. No check that `ty`
           targets `inductive`, no positivity check, no universe check, no arity check);
           kernel/src/context.rs:539-561 (`add_constructor_checked` — the SAFE path: it calls
           `crate::positivity::check_positivity(inductive, name, &ty)?` and then
           `crate::type_checker::check_constructor_universes(self, inductive, name, &ty)?`
           before delegating to `add_constructor`. Both checks exist; both are OPT-IN);
           kernel/src/type_checker.rs:540-583 (`check_constructor_universes` — its own doc
           comment names what it prevents: "the universe inconsistency that opens
           Girard/Hurkens paradoxes"); kernel/src/context.rs:570-628
           (`add_mutual_inductives` — checks positivity over the whole block and is
           transactional, the third safe entry point);
           kernel/src/context.rs:442-444 (`add_definition(name, ty, body)` is ALSO unchecked:
           it inserts `(ty, body)` with NO verification that `body : ty`. There is no
           `add_definition_checked` anywhere in the crate)
@diag      E_UNCHECKED_DECLARATION_PATH — must name the declaration, state which check
           (positivity or constructor universes) the Lean source would have failed, and emit
           nothing. A translator that reaches this diagnostic because it was ABOUT to call
           `add_constructor` rather than `add_constructor_checked` must treat that as a bug in
           itself, not in the input.
@guard     **TIER 11 THESIS, STATED AS AN API FACT.** LOGOS's kernel accepting a term is not
           evidence that the translation is faithful — and the declaration API does not even
           pretend to check. `add_inductive` and `add_constructor` are `insert` calls. A
           translator that uses them builds whatever environment it is told to build, including
           one in which `False` is provable, and every subsequent `infer_type` call is then
           checking terms against an inconsistent theory. The kernel's "yes" is relative to an
           environment the translator chose.
@guard     Catches the negative-occurrence inductive. `inductive Bad where | mk : (Bad → False)
           → Bad` is refused by Lean and refused by `check_positivity`
           (context.rs:545-546) — but ONLY if the translator called
           `add_constructor_checked`. Via `add_constructor` it registers, and the standard
           Berardi/Hurkens derivation of `False` goes through with every step typechecking.
           The kernel is not broken; it was asked to trust a declaration.
@guard     Catches the universe-inconsistent constructor. `check_constructor_universes`
           (type_checker.rs:540-583) enforces that every VALUE argument's sort is `≤` the
           inductive's result sort, with the impredicative-`Prop` exemption at :556-557 — and
           it decides `≤` with `is_subtype_of`, i.e. the CUMULATIVE order (term.rs:243-249).
           So even the checked path is more permissive than Lean's non-cumulative constraint;
           see T11-019. A translator must therefore run BOTH: `add_constructor_checked` AND
           its own record of Lean's universe judgement.
@guard     Catches the unchecked path being reached by convenience rather than by intent. Tier
           10 requires the translator to emit `List`, `Option`, `Prod`, `Sum`, `Sigma`,
           `Subtype` and `Fin` from scratch (T10-012, T10-013), plus `Nat` arithmetic
           (T10-017) and possibly a `String`/`Char` layer (T10-005, T10-006). That is a lot of
           declaration-emitting code, written once, far from the input, and the two-argument
           `add_constructor` is the shorter call. Every one of those emissions must go through
           the checked path.
@guard     Catches the SAME hole on the definition side, which has no checked path at all.
           `add_definition` (context.rs:442-444) inserts a `(ty, body)` pair without verifying
           `body : ty`, and there is no `add_definition_checked` in the crate. So a translator
           that emits a definition whose body does not have the stated type installs a global
           whose `get_global` type (context.rs:459-461) and whose `get_definition_body` value
           (context.rs:474) describe different things — the same typing/reduction split as
           T11-003, arrived at without any name collision. The translator MUST call
           `infer_type` on the body and `is_subtype` against the type itself, before
           `add_definition`.
@guard     Catches "the re-checker will catch it". `recheck.rs` re-verifies TERMS against a
           context; it does not re-derive the environment's well-formedness, and
           `certificate::recheck` rebuilds only the TRUSTED BASE from
           `StandardLibrary::register` (certificate.rs:63-75) — every translator-added
           declaration is taken as given.
@oracle    Translator exits non-zero with E_UNCHECKED_DECLARATION_PATH for each declaration
           below and emits nothing. The decisive harness assertion is about the translator's
           own call graph, not about any term: over the whole corpus, the number of
           `add_constructor` calls must be ZERO — every constructor must go through
           `add_constructor_checked` or `add_mutual_inductives` — and every `add_definition`
           call must be immediately preceded by a successful `infer_type` of the body and an
           `is_subtype` check against the declared type.
-/

namespace T11_013

/-! ### Strict positivity. Lean refuses the declaration; `add_constructor` would accept it. -/

/-
  inductive Bad where
    | mk : (Bad → False) → Bad

  error: (kernel) arg #1 of 'T11_013.Bad.mk' has a non positive occurrence of the datatypes
  being declared

  Via `ctx.add_constructor("Bad_mk", "Bad", …)` this registers with no complaint
  (context.rs:413-431). `add_constructor_checked` (context.rs:539-561) refuses it by calling
  `positivity::check_positivity`.
-/

/-- The compiling neighbour: the same shape with the occurrence in a POSITIVE position. -/
inductive Good where
  | mk : (Nat → Good) → Good

/-- Why the negative version matters: with `Bad` registered, this is the standard route to
    `False`. Each step typechecks; the environment is what is wrong. Written here against
    `Good` so the file compiles, with the `Bad` version in the comment above. -/
def unfold : Good → (Nat → Good)
  | Good.mk f => f

/-! ### Universe inconsistency in a constructor. -/

/-
  inductive Univ : Type where
    | mk : Type → Univ

  error: invalid universe level in constructor 'T11_013.Univ.mk', parameter has type
    Type
  at universe level
    2
  it must be smaller than or equal to the inductive datatype universe level
    1

  `check_constructor_universes` (type_checker.rs:540-583) rejects this on the checked path —
  the stored field lives in `Type 1` and the inductive is `Type 0`. `add_constructor` does
  not look.
-/

/-- The compiling neighbour: declare the inductive one universe higher. -/
inductive Univ : Type 1 where
  | mk : Type → Univ

/-- And the `Prop` exemption, which the checked path implements at type_checker.rs:556-557:
    an impredicative `Prop` inductive may store an argument of any sort. -/
inductive AnyProp : Prop where
  | mk : Type → AnyProp

/-! ### The support types Tier 10 forces the translator to emit — every one of which must go
     through the CHECKED path even though none of them is suspicious. -/

inductive MyOption (α : Type) : Type where
  | none : MyOption α
  | some : α → MyOption α

inductive MyList (α : Type) : Type where
  | nil : MyList α
  | cons : α → MyList α → MyList α

structure MyProd (α β : Type) where
  fst : α
  snd : β

/-- A definition whose body's type must be checked against the declared type BEFORE
    `add_definition` installs it — there is no checked variant in the kernel to do it for
    you (context.rs:442-444). -/
def myLength {α : Type} : MyList α → Nat
  | MyList.nil => 0
  | MyList.cons _ xs => 1 + myLength xs

example : myLength (MyList.cons 1 MyList.nil) = 1 := by simp [myLength]

/-! ### A constructor whose type does not even target its own inductive. `add_constructor`
     records the association from its `inductive` ARGUMENT, not from the type
     (context.rs:420-431), so the two can disagree and nothing notices. -/

inductive A' where
  | mk : A'

inductive B' where
  | mk : B'

/-- In Lean these are two separate declarations and `A'.mk : A'` is forced by the syntax. A
    translator calling `ctx.add_constructor("B_mk", "A'", <type targeting B'>)` creates a
    constructor filed under `A'` whose type produces a `B'` — and `Match` on an `A'` will then
    hand that case a `B'`-typed binder. -/
example : A'.mk = A'.mk := rfl

end T11_013
