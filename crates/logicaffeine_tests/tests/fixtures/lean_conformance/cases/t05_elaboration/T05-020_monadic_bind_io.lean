/-!
@case      T05-020
@tier      5 · Elaboration surface
@title     Monadic bind and `IO` — no `Monad` class can be declared, and `IO` is an opaque
           world-passing primitive
@lean      Monad, Bind.bind (`>>=`), Pure.pure, Functor.map (`<$>`), Seq (`<*>`), IO,
           IO.println, EStateM, IO.RealWorld
@expect    reject
@target    (none — no sound translation)
@evidence  There is **no `Monad`, `Bind`, `Pure`, `Functor`, `Applicative`, `Seq` or `IO`**
           anywhere in `crates/logicaffeine_kernel/src` (LOGOS_FACTS prelude inventory;
           prelude.rs registers only `Entity`, `Int`, `Float`, `Text`, `Duration`, `Date`,
           `Moment`, `Nat`/`Zero`/`Succ`, `EList`/`ENil`/`ECons`, `Bool`, `TList`, `True`/`I`,
           `False`, `Decidable`, `Eq`, `Acc`, `And`, `Or`, `Ex`, and the hardware types);
           kernel/src/context.rs:271-369 and :300-303 (`add_structure(name, params, fields)`
           hard-codes the structure's sort to `Type 0`. A `Monad` class is parameterised by
           `m : Type u → Type v` and lives in `Type (max (u+1) (v+1))`, so it cannot be
           declared at all — this is T03-012 / T07-011);
           kernel/src/context.rs:253,374 (`add_inductive`/`add_indexed_inductive` take a single
           FIXED sort — there is no universe-polymorphic inductive, T01-007, and `IO`'s
           `EStateM IO.Error IO.RealWorld` encoding needs one);
           kernel/src/context.rs:434 (`add_declaration` — the only opaque form — is an AXIOM:
           an `IO` primitive declared this way asserts an inhabitant of its type, which for
           `IO Unit` is harmless and for `IO α` at an arbitrary `α` is `Π(α:Type 0). α`,
           i.e. inconsistency);
           kernel/src/context.rs:449 (`get_global` resolves against four maps; none contains
           any of these names)
@diag      E_NO_MONAD_NO_IO — must name the declaration, name the class or monad
           (`Bind`/`Pure`/`Functor`/`IO`), and state that LOGOS has no `Monad` class, no
           higher-kinded class declaration path, and no `IO`. Must NOT emit any `Global` for
           them and must NOT declare any as an axiom.
@guard     **Translate the post-elaboration `Expr`, never the surface `Syntax` — and be
           precise about WHY this rejects.** T05-019 rejects because the do-elaborator's
           OUTPUT refers to absent globals. This case rejects for a strictly deeper reason: the
           globals cannot be *supplied*. Even a translator willing to emit the missing
           declarations cannot, because (a) `Monad` is higher-kinded and `add_structure` pins
           structures to `Type 0` (context.rs:300-303), and (b) `IO` is `EStateM IO.Error
           IO.RealWorld` — a state monad over an opaque world token whose primitives are
           implemented by the Lean runtime, not by any term. The distinction matters for the
           diagnostic: `do` on `Option` is "LOGOS lacks this definition"; `Monad`/`IO` is
           "LOGOS cannot express this kind".
@guard     Catches declaring a `Monad` class via `add_structure`. The call would need a field
           `bind : Π{α β : Type 0}. m α → (α → m β) → m β` with `m : Type 0 → Type 0`, and
           `add_structure` gives the resulting structure the sort `Type 0` (context.rs:300-303) —
           which is WRONG: a structure with a `Type 0 → Type 0` field lives in `Type 1`. The
           registration is UNCHECKED (`add_inductive`, context.rs:253), so it succeeds, and the
           library now contains a type whose declared sort is a lie. Because LOGOS is
           CUMULATIVE (type_checker.rs:1330-1360), downstream terms often still typecheck —
           this is the T11-013 / T11-019 trap reached through Tier 5.
@guard     Catches declaring `IO` as an axiom. `add_declaration` (context.rs:434) is unchecked
           and opaque; `IO : Type 0 → Type 0` as an axiom is a type former with NO
           constructors, so nothing inhabits `IO α` and every `IO`-valued definition becomes
           unprovable — or, if the translator also axiomatises `IO.println : Text → IO Unit`
           and `pure : Π(α). α → IO α`, it has asserted `Π(α:Type 0). α → IO α` plus an
           elimination it cannot justify.
@guard     Catches `IO.RealWorld`/`EStateM` specifically. Lean's `IO` is a state monad over an
           opaque world token, and its primitives are `@[extern]` runtime calls with no
           defining equations. There is no term to translate — not "a term LOGOS cannot hold",
           but no term at all. A translator that reaches an `@[extern]` constant must reject
           rather than emit an axiom.
@guard     Catches `>>=`/`<$>`/`<*>` used OUTSIDE a `do` block. A translator that keys its
           rejection on the `do` keyword (or on `Syntax` node kinds) misses `bindEx` and
           `mapEx` below entirely and emits dangling `Global`s. The rejection must key on the
           elaborated `Expr`'s head constants.
@guard     Catches a USER-DEFINED monad being treated as easier. `MyOpt` below is an ordinary
           `Type 0 → Type 0` inductive whose `bind` is definable — but the `Monad MyOpt`
           INSTANCE still requires the higher-kinded class, so the class is the obstruction,
           not the monad. A translator may inline the user's `bind`/`pure` as plain functions
           (a re-expression, recorded as such) while still rejecting the class.
@guard     Catches over-rejection of `Decidable`. `Decidable` IS in the LOGOS prelude
           (prelude.rs:781-794) and is `Type 0`-valued with one parameter; it is not
           higher-kinded and must not be swept up by a rule that rejects "classes over type
           constructors".
@oracle    Translator exits non-zero with E_NO_MONAD_NO_IO for every definition below; writes
           no declaration for them; emits no `Global` flattening of `Monad`, `Bind`, `Pure`,
           `Functor`, `Seq`, `IO`, `EStateM` or `IO.RealWorld`; and adds no axiom and no
           `add_structure` call for a higher-kinded class.
-/

namespace T05_020

/-- `>>=` used directly, with no `do` block — a translator keyed on the `do` keyword misses
    this. -/
def bindEx (x : Option Nat) : Option Nat := x >>= fun a => some (a + 1)

example : bindEx (some 1) = some 2 := rfl

/-- `<$>` (`Functor.map`) and `<*>` (`Seq.seq`). -/
def mapEx (x : Option Nat) : Option Nat := (· + 1) <$> x

def seqEx (f : Option (Nat → Nat)) (x : Option Nat) : Option Nat := f <*> x

example : mapEx (some 1) = some 2 := rfl
example : seqEx (some (· + 1)) (some 1) = some 2 := rfl

/-- `pure` at an inferred monad. -/
def pureEx : Option Nat := pure 5

example : pureEx = some 5 := rfl

/-- A function POLYMORPHIC in the monad — the `Monad m` instance parameter is the thing that
    cannot be declared. -/
def twice {m : Type → Type} [Monad m] (x : m Nat) : m Nat := do
  let a ← x
  let b ← x
  return a + b

example : twice (some 3) = some 6 := rfl

/-- A USER-DEFINED monad: the type and its `bind` are ordinary definable things; the `Monad`
    INSTANCE is what LOGOS cannot express. -/
inductive MyOpt (α : Type) where
  | none : MyOpt α
  | some : α → MyOpt α

def MyOpt.bind {α β : Type} : MyOpt α → (α → MyOpt β) → MyOpt β
  | .none, _ => .none
  | .some a, f => f a

instance : Monad MyOpt where
  pure a := .some a
  bind := MyOpt.bind

def myTwice (x : MyOpt Nat) : MyOpt Nat := do
  let a ← x
  return a + a

example : myTwice (.some 2) = .some 4 := rfl

/-- `IO` — an opaque state monad over `IO.RealWorld` whose primitives are `@[extern]` runtime
    calls with no defining equations. -/
def prog : IO Unit := do
  IO.println "hello"

def progBind : IO Unit := IO.println "a" >>= fun _ => IO.println "b"

/-- `IO` at a non-`Unit` result type, where an axiomatisation would assert an inhabitant of an
    arbitrary type. -/
def readNothing : IO Nat := pure 0

/-- `Decidable` — present in the LOGOS prelude, `Type 0`-valued, and NOT higher-kinded. It
    must not be swept up by a rule that rejects classes over type constructors. -/
def decideEx (n : Nat) : Bool := decide (n = 0)

example : decideEx 0 = true := rfl

end T05_020
