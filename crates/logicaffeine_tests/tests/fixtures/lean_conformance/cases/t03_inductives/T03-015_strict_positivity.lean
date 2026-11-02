/-!
@case      T03-015
@tier      3 · Inductives and structures
@title     Strict positivity violation — must be refused *before* `add_constructor`
@lean      inductive with a negative occurrence (`Bad := mk : (Bad → False) → Bad`),
           Lean's "(non-)positive occurrence" error
@target    (none — no sound translation)
@expect    reject
@diag      E_POSITIVITY_VIOLATION — must name the inductive, the constructor, and the argument
           position in which the inductive occurs negatively, and must state that no
           declaration was registered. The diagnostic must be produced by the *translator*, not
           delegated to the kernel: LOGOS has an unchecked registration path and a translator
           that reaches it has already lost.
@evidence  kernel/src/context.rs:413-428 (`add_constructor` — **UNCHECKED**. It is a
           `HashMap::insert` plus a push onto `constructor_order`; no positivity, no universes,
           no typecheck of `ty` at all);
           kernel/src/context.rs:538-560 (`add_constructor_checked` — the *only* path that
           calls `positivity::check_positivity` and `check_constructor_universes`);
           kernel/src/positivity.rs:29-47 (`check_positivity` / `check_positivity_mutual`, and
           the doc: "This prevents paradoxes like `Inductive Bad := Cons : (Bad -> False) -> Bad`",
           context.rs:538-544);
           kernel/src/positivity.rs:54-85 (the rule: in `Π(x:A). B`, if `A` is not a recursive
           argument and a block member occurs in `A`, it is a `PositivityViolation`);
           kernel/src/context.rs:562-594 (`add_mutual_inductives` checks positivity over the
           whole block up front and is transactional — the mutual path is safe by construction,
           the single path is not);
           kernel/src/type_checker.rs:321-343 (the termination guard protects `Fix`, not
           declarations — a negative inductive needs no `Fix` to produce `False`)
@guard     **The primary guard: never call `add_constructor`.** It is the ergonomic API — it is
           what `add_structure` itself uses internally (context.rs:306) — and it performs no
           check whatsoever. A translator that registers constructors with it has silently
           disabled the kernel's only defence against the Girard/Hurkens family of paradoxes.
           `Bad := mk : (Bad → False) → Bad` registered this way yields a closed inhabitant of
           `False`, and every certificate minted afterwards is worthless while still reporting
           `verified == true` (verify.rs:31-43).
@guard     Catches "Lean already checked it, so we can use the fast path". Lean checked the
           *Lean* declaration. What reaches LOGOS is the translator's rendering of it, and a
           rendering bug — a dropped arrow, a `Pi` whose domain and codomain were swapped, a
           parameter telescope reattached in the wrong place — can turn a positive declaration
           into a negative one. The check must be re-run on the emitted term, which is exactly
           what `add_constructor_checked` is for.
@guard     Catches routing a *mutual* block member through `add_constructor_checked`. That
           checks positivity against **one** inductive name (positivity.rs:29-35), so a sibling
           in a negative position — `Even`'s constructor storing `Odd n → False` — passes.
           Mutual blocks must go through `add_mutual_inductives`, which calls
           `check_positivity_mutual` over the whole block before mutating anything
           (context.rs:575-580).
@guard     Catches confusing "positive" with "appears only to the right of arrows in the final
           result". LOGOS's rule (positivity.rs:54-85) is per-`Π`: a member may appear as an
           *argument type* (`Π(x : I …). …` — a recursive argument), but must not appear
           *inside* a non-recursive argument type. `(Nat → Bad) → Bad` is fine;
           `((Bad → Nat) → Nat) → Bad` is not.
@guard     Catches degrading instead of rejecting. There is no loss record that makes an
           inconsistent context acceptable.
@note      Lean 4.14.0 refuses every declaration this case is about, so none of them can appear
           as compiling source. Each is reproduced verbatim in the comment block below,
           together with Lean's exact diagnostic. The compiling declarations that follow are the
           **nearest legal neighbours** — the strictly-positive variants that differ from the
           illegal ones by a single arrow — so the file still exercises exactly the
           discrimination `check_positivity` has to make, from the accepting side.
@oracle    Translator exits non-zero with E_POSITIVITY_VIOLATION and registers nothing. For
           every inductive it *does* register, the registration went through
           `add_constructor_checked` (single) or `add_mutual_inductives` (block) — never
           `add_constructor`. Independently: re-running `positivity::check_positivity` over the
           emitted constructor types must succeed for all of them.
-/

namespace T03_015

/-
THE OFFENDING DECLARATIONS — each rejected by Lean 4.14.0.

1. The classical paradox. Lean:
     error: (kernel) arg #1 of 'Bad.mk' has a non positive occurrence of the datatypes being declared

     inductive Bad where
       | mk : (Bad → False) → Bad

   With it, `False` is inhabited:

     def unBad : Bad → (Bad → False) := fun b => match b with | Bad.mk f => f
     def selfApp (b : Bad) : False := unBad b b
     def boom : False := selfApp (Bad.mk (fun b => selfApp b))

2. Negative occurrence one level deeper — still refused:

     inductive Bad2 where
       | mk : ((Bad2 → Nat) → Nat) → Bad2

3. Negative occurrence inside a container argument:

     inductive Bad3 where
       | mk : (Bad3 → Bool) → Nat → Bad3

4. A CROSS-BLOCK negative occurrence. Lean rejects the block; LOGOS catches it only via
   `check_positivity_mutual` (positivity.rs:36-47), i.e. only if the block is registered
   through `add_mutual_inductives`:

     mutual
       inductive Ping where
         | p : (Pong → False) → Ping
       inductive Pong where
         | q : Ping → Pong
     end

LOGOS's `Context::add_constructor` (context.rs:413-428) would accept the translated form of
every one of these without complaint. Only `add_constructor_checked` (context.rs:538-560)
and `add_mutual_inductives` (context.rs:562-594) run `positivity::check_positivity`.
-/

/-- **Legal neighbour of (1).** The inductive occurs to the *right* of the arrow, so it is a
    strictly-positive recursive occurrence. One arrow apart from the paradox. -/
inductive GoodFn where
  | stop : GoodFn
  | mk : (Nat → GoodFn) → GoodFn

def depthG : GoodFn → Nat
  | GoodFn.stop => 0
  | GoodFn.mk f => Nat.succ (depthG (f 0))

example : depthG GoodFn.stop = 0 := rfl
example : depthG (GoodFn.mk (fun _ => GoodFn.stop)) = 1 := rfl

/-- **Legal neighbour of (3).** A *recursive argument* — the inductive appears as an argument
    type directly, which `is_recursive_arg` (positivity.rs:60-70) accepts. -/
inductive GoodRec where
  | leaf : Nat → GoodRec
  | node : GoodRec → GoodRec → GoodRec

def sumG : GoodRec → Nat
  | GoodRec.leaf n => n
  | GoodRec.node l r => sumG l + sumG r

example : sumG (GoodRec.node (GoodRec.leaf 1) (GoodRec.leaf 2)) = 3 := rfl

/-- **Legal neighbour of (2).** Two nested arrows, but every occurrence of the inductive is
    still strictly positive. This is the case a *too eager* positivity check would wrongly
    reject — the discrimination has to be exact in both directions. -/
inductive GoodNested where
  | stop : GoodNested
  | mk : ((Nat → Nat) → GoodNested) → GoodNested

def probeG : GoodNested → Nat
  | GoodNested.stop => 0
  | GoodNested.mk f => Nat.succ (probeG (f (fun n => n)))

example : probeG GoodNested.stop = 0 := rfl
example : probeG (GoodNested.mk (fun _ => GoodNested.stop)) = 1 := rfl

mutual
  /-- **Legal neighbour of (4).** The sibling occurs positively in both directions, so
      `check_positivity_mutual` accepts the block. Flip either occurrence into a domain and
      the block becomes the paradox above. -/
  inductive PingOk where
    | p : PongOk → PingOk
    | pstop : PingOk
  inductive PongOk where
    | q : PingOk → PongOk
    | qstop : PongOk
end

mutual
  def pingSize : PingOk → Nat
    | PingOk.pstop => 0
    | PingOk.p q => Nat.succ (pongSize q)
  def pongSize : PongOk → Nat
    | PongOk.qstop => 0
    | PongOk.q p => Nat.succ (pingSize p)
end

example : pingSize (PingOk.p (PongOk.q PingOk.pstop)) = 2 := rfl

/-- The occurrence that *is* legal inside a domain: a **different**, already-declared
    inductive. `check_positivity` only ever objects to members of the block being declared
    (positivity.rs:54-85). -/
inductive UsesOther where
  | mk : (GoodRec → Nat) → UsesOther

def applyOther : UsesOther → Nat
  | UsesOther.mk f => f (GoodRec.leaf 7)

example : applyOther (UsesOther.mk sumG) = 7 := rfl

end T03_015
