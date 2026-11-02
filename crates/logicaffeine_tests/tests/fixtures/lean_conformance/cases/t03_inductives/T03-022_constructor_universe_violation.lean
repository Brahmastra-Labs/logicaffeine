/-!
@case      T03-022
@tier      3 · Inductives and structures
@title     Constructor universe constraint violation — the Girard/Hurkens door
@lean      inductive at `Type k` storing a field whose sort exceeds `Type k`; Lean's
           "invalid universe level in constructor … it must be smaller than or equal to the
           inductive datatype universe level"
@target    (none — no sound translation)
@expect    reject
@diag      E_CONSTRUCTOR_UNIVERSE_VIOLATION — must name the inductive, the constructor, the
           offending field, the field's sort and the inductive's sort. Must register nothing:
           `add_inductive` and `add_constructor` are both unchecked, so a translator that
           "registers and lets the kernel complain" has already put an inconsistent declaration
           in the context.
@evidence  kernel/src/type_checker.rs:539-580 (`check_constructor_universes`: "every VALUE
           argument's sort must be `≤` the inductive's result sort — so a `Type 0` inductive
           cannot store a `Type 0`-typed field (which lives in `Type 1`), the universe
           inconsistency that opens Girard/Hurkens paradoxes"; the leading `num_params`
           arguments are exempted, and a `Prop` inductive is exempted entirely);
           kernel/src/context.rs:545-560 (`add_constructor_checked` — the **only** caller of
           that check);
           kernel/src/context.rs:413-428 (`add_constructor` — **unchecked**: no positivity, no
           universes, no typecheck);
           kernel/src/context.rs:271-306 (`add_structure` registers its constructor through the
           unchecked `add_constructor`, so **no structure's fields are universe-checked at
           all**);
           kernel/src/context.rs:581-594 (`add_mutual_inductives` *does* run the check, in a
           temp environment carrying the block's headers, before mutating — the mutual path is
           safe, the single and structure paths are not);
           kernel/src/term.rs:169-191 (`Universe::Type(u32)`, and `succ`: `Prop → Type 1`,
           `Type n → Type (n+1)` — so a field of type `Type 0` has sort `Type 1`)
@guard     **The primary guard: never call `add_constructor` directly.** It is the path
           `add_structure` itself uses, and it performs no universe check. `Bad : Type 0` with
           `mk : Type 0 → Bad` registered this way is a `Type 0` type that quantifies over
           `Type 0` — the impredicativity that Girard's paradox exploits. Nothing downstream
           notices; the context is simply inconsistent.
@guard     Catches trusting Lean's check to have covered it. Lean checked the Lean declaration.
           What LOGOS receives is the translator's rendering, and the check is sensitive to the
           **parameter count**: `check_constructor_universes` exempts the leading
           `inductive_num_params(ind)` binders (type_checker.rs:561-564). Register the
           inductive with too *many* parameters — the T03-005 `add_inductive` default — and a
           genuinely offending field is exempted as if it were a parameter, so even
           `add_constructor_checked` lets it through. The two bugs compose.
@guard     Catches the `Prop` exemption being applied to the wrong thing. A `Prop` inductive is
           exempt (impredicative `Prop`, type_checker.rs:556-561) — but only if it is
           *registered* at `Prop`. A translator that re-sorts a `Type 0` inductive to `Prop`
           to get past the check has not fixed anything; it has made a data type into a
           proposition (T03-012) and turned off the gate that was protecting it.
@guard     Catches the opposite over-correction: bumping the inductive's sort to `Type 1` to
           satisfy the check. That is often the *right* declaration — it is what Lean itself
           would infer — but it is a different type from the one the source declared, and every
           downstream use site that expected `Type 0` must be re-checked. Cumulativity
           (`Type 0 ≤ Type 1`, type_checker.rs:1330-1360) makes the *uses* typecheck in the
           permissive direction, hiding the change. If the bump was not in the Lean source, it
           must be reported, not performed silently.
@guard     Catches structures specifically. `add_structure` never universe-checks its fields
           (context.rs:306 calls the unchecked `add_constructor`) **and** hard-codes the
           structure's sort to `Type 0` (context.rs:300). So a Lean structure with a
           `Type`-valued field is doubly unrepresentable: the sort is wrong and the check that
           would have caught it does not run. See T03-012.
@note      Lean 4.14.0 refuses every offending declaration, so none can appear as compiling
           source. Each is reproduced verbatim in the comment block below with Lean's exact
           diagnostic. The compiling declarations are the **nearest legal neighbours** — the
           same records, one universe higher, which Lean accepts — so the file exercises the
           exact discrimination `check_constructor_universes` has to make, from the accepting
           side.
@oracle    Translator exits non-zero with E_CONSTRUCTOR_UNIVERSE_VIOLATION and registers
           nothing. For every inductive it does emit, the registration went through
           `add_constructor_checked` or `add_mutual_inductives`, and re-running
           `check_constructor_universes` against the emitted `num_params` succeeds.
-/

namespace T03_022

/-
THE OFFENDING DECLARATIONS — each rejected by Lean 4.14.0.

1. A `Type 0` inductive storing a `Type 0`. The field's sort is `Type 1`.

     inductive Bad : Type where
       | mk : Type → Bad

   error: invalid universe level in constructor 'Bad.mk', parameter has type
     Type
   at universe level
     2
   it must be smaller than or equal to the inductive datatype universe level

   (Lean counts `Sort 1 = Type 0`, so its "universe level 2" is this file's `Type 1`.)

2. The same violation through a bound type variable — the shape that actually appears in
   practice, because it looks like ordinary parametricity:

     inductive Bad2 : Type where
       | mk : (α : Type) → α → Bad2

   error: invalid universe level in constructor 'Bad2.mk', parameter 'α' has type
     Type
   at universe level
     2
   it must be smaller than or equal to the inductive datatype universe level

3. The structure form. Lean rejects it for the same reason; LOGOS's `add_structure` would
   accept it, because it registers its constructor through the **unchecked**
   `add_constructor` (context.rs:306) and pins the sort to `Type 0` (context.rs:300):

     structure BadCarrier : Type where
       ty : Type
       point : ty

LOGOS's `Context::add_constructor` (context.rs:413-428) accepts the translated form of all
three without complaint. Only `add_constructor_checked` (context.rs:545-560) and
`add_mutual_inductives` (context.rs:581-594) run `check_constructor_universes`.
-/

/-- **Legal neighbour of (1).** One universe higher, and Lean accepts it. This is the
    declaration a translator is tempted to emit in place of the illegal one — correct as a
    type, but *different* from what the source said. -/
inductive Good : Type 1 where
  | mk : Type → Good

def goodNat : Good := Good.mk Nat

/-- **Legal neighbour of (2).** The existential-package shape, at `Type 1`. -/
inductive Package : Type 1 where
  | mk : (α : Type) → α → Package

def packNat : Package := Package.mk Nat 0

def packCarrier : Package → Type
  | Package.mk α _ => α

example : packCarrier packNat = Nat := rfl

/-- **Legal neighbour of (3).** The structure at `Type 1` — which `add_structure` cannot
    express at all, because its sort is hard-coded (T03-012). -/
structure Carrier : Type 1 where
  ty : Type
  point : ty

def natCarrier : Carrier := Carrier.mk Nat 0

example : natCarrier.ty = Nat := rfl

/-- **The `Prop` exemption, exercised.** Impredicative `Prop` really does admit a field of any
    sort, in both systems (type_checker.rs:556-561). This is legal and must stay legal — a
    translator that applies the universe check to `Prop` inductives breaks it. -/
inductive PropPackage : Prop where
  | mk : (α : Type) → α → PropPackage

def ppack : PropPackage := PropPackage.mk Nat 0

/-- …and the gate that keeps that from being dangerous: a non-subsingleton-free `Prop` cannot
    be large-eliminated, so the `α` cannot be extracted. `PropPackage` has one constructor but
    stores non-`Prop` data, so `is_subsingleton_prop` is false and the following is refused by
    both systems:
    `def unpack (p : PropPackage) : Type := match p with | PropPackage.mk α _ => α`
    ⇒ `recursor 'PropPackage.rec' can only eliminate into Prop` -/
example : PropPackage := ppack

/-- **A parameter is exempt.** `check_constructor_universes` skips the leading
    `inductive_num_params` binders (type_checker.rs:561-564), which is why an ordinary
    parameterised container at `Type 0` is fine: `α` is a parameter, not a stored field. This
    is the boundary the "too many parameters" guard is about — get `num_params` wrong and a
    genuine field gets this exemption. -/
inductive Box (α : Type) : Type where
  | mk : α → Box α

def boxed : Box Nat := Box.mk 3

def unbox : Box Nat → Nat
  | Box.mk n => n

example : unbox boxed = 3 := rfl

-- The **mutual** path, which *is* universe-checked (context.rs:581-594). Both members at
-- `Type 1`, both storing `Type`-valued fields — legal, and checked.
mutual
  inductive MA : Type 1 where
    | ma : Type → MB → MA
  inductive MB : Type 1 where
    | mb : Type → MB
    | mstop : MB
end

def mbNat : MB := MB.mb Nat
def maNat : MA := MA.ma Bool mbNat

end T03_022
