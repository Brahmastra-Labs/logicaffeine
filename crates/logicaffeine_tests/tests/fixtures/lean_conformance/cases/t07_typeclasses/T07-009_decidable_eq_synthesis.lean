/-!
@case      T07-009
@tier      7 · Typeclasses
@title     `DecidableEq` synthesis — only two decision procedures exist, and `deriving` is not a
           translatable construct
@lean      DecidableEq, deriving DecidableEq, instDecidableEqNat/Bool, structural
           DecidableEq for inductives and structures
@expect    degrade
@target    `DecidableEq α` ⟼ `Π(a:α). Π(b:α). App(Global("Decidable"), Eq α a b)` — there is no
           `DecidableEq` global; it is an abbreviation and must be expanded.
           `instDecidableEqNat`  ⟼ `Global("decEqNat")`  (prelude DEFINITION + `add_instance`)
           `instDecidableEqBool` ⟼ `Global("decEqBool")` (prelude DEFINITION + `add_instance`)
           For every OTHER type, the translator must translate the decision procedure that
           Lean's `deriving DecidableEq` handler GENERATED — an ordinary recursive function
           returning `isTrue`/`isFalse` — and then `add_definition` it and `add_instance` it.
           Plus a loss record: the generated procedure is not the Lean source.
@evidence  kernel/src/prelude.rs:923-1026 (`register_dec_eq_bool` — "Decidable equality of
           `Bool` (`decEqBool : Π(a b:Bool). Decidable (Eq Bool a b)`) … registered as a
           `Decidable` INSTANCE, so `decide` can actually discharge `Eq Bool _ _` goals");
           kernel/src/prelude.rs:1027-1194 (`register_dec_eq_nat` — `decEqNat`, by structural
           recursion on the first argument, with a nested `b`-match in the `Succ a'` branch at
           :1157); a grep of prelude.rs for `add_instance` shows these are the ONLY two decision
           procedures registered; kernel/src/prelude.rs:757-818 (`Decidable` itself, `Type 0`-
           valued with `isTrue`/`isFalse`); kernel/src/prelude.rs:820-922
           (`of_decide_eq_true`, PROVEN not axiomatised — the bridge back to a `Prop`);
           kernel/src/termination.rs (the termination guard is a syntactic structural
           (Giménez) check; there is no well-founded recursion, no `termination_by`, no equation
           compiler — so a generated procedure must be structurally recursive to be accepted);
           kernel/src/type_checker.rs:254-260 (`Match` is POSITIONAL: `cases.len()` must equal
           the constructor count, in registration order; there are no patterns and no wildcards)
@loss      DERIVED_CODE_IS_NOT_SOURCE. `deriving DecidableEq` is a metaprogram
           (`Lean.Elab.Deriving.DecEq`) that runs in `MetaM` and cannot be replayed — see
           T09-006. What the translator can carry across is only its OUTPUT: the elaborated
           `Expr` of the generated instance. So the LOGOS development contains a decision
           procedure that appears in no Lean source file, whose correspondence to the Lean
           declaration rests on the derive handler having been correct. That must be recorded;
           it is not the same trust story as a hand-written instance.
@loss      NO_STRUCTURAL_SYNTHESIS_IN_LOGOS. LOGOS cannot synthesise a `DecidableEq` for
           anything. If the Lean source relied on Lean's synthesis and the generated term does
           not survive translation — because it uses nested patterns (T04-008), a wildcard
           (T04-010), `noConfusion`/`injection` (T03-018) or non-structural recursion
           (T04-018), all of which Lean's handler emits routinely — there is no fallback. The
           loss record must distinguish "translated the generated procedure" from "could not".
@loss      NO_DECIDABLE_FOR_INT. LOGOS's `Int` is OPAQUE with axiomatised order laws
           (prelude.rs:92-340); `le`/`lt` are `Int → Int → Bool` axioms (prelude.rs:352-515),
           not decision procedures, and no `Decidable (Eq Int a b)` instance is registered.
           A Lean `DecidableEq Int` therefore has NO target at all. `intEq` below is the case.
@guard     Catches emitting `DecidableEq` as a class. It is an ABBREVIATION in Lean
           (`DecidableEq α := (a b : α) → Decidable (a = b)`), so there is no class to declare,
           no `DecidableEq_mk`, and no projection. A translator that emits it via
           `add_structure` creates a genuinely different type, and the resulting instances no
           longer unify with the `Decidable (Eq α a b)` goals that `decide` actually raises
           (prelude.rs:796-818).
@guard     Catches assuming an instance for `List`, `Option`, `Prod` or `String`. None of those
           types is in the LOGOS prelude at all (see T10-011/012/013/016), let alone a decision
           procedure for them. A translator must emit the type AND its procedure AND the
           instance, in that order, or escalate.
@guard     Catches translating the derived procedure's `match` without expanding it. Lean's
           generated code uses nested and wildcard patterns; LOGOS's `Match` is positional with
           exactly one case per constructor in REGISTRATION order (type_checker.rs:254-260).
           Getting the order wrong produces a procedure that typechecks and returns the wrong
           answer — `isTrue` where `isFalse` was meant — which then feeds `of_decide_eq_true`
           (prelude.rs:824-922) and yields a "proof" of a false equation. This is the soundness
           bug in this case and it is why the loss record is not optional.
@guard     Catches registering the emitted procedure as a DEFINITION only. Nothing scans
           definitions during resolution (elaborate.rs:1331); without the `add_instance` the
           procedure is unreachable and every `decide` on that type fails.
@oracle    `natEq`/`boolEq` resolve to the prelude's `decEqNat`/`decEqBool` and are NOT
           re-emitted; `colourEq` and `pairEq` are accompanied by an emitted structurally-
           recursive procedure plus an `add_instance`, and a loss record naming
           DERIVED_CODE_IS_NOT_SOURCE; `intEq` and `listEq` are escalated. A harness checks
           every emitted `Match`'s case count against the inductive's constructor count.
-/

namespace T07_009

/-- The two types LOGOS already decides. -/
def natEq (a b : Nat) : Bool := decide (a = b)
def boolEq (a b : Bool) : Bool := decide (a = b)

example : natEq 3 3 = true := rfl
example : boolEq true false = false := rfl

/-- A user enum with a DERIVED instance. Lean generates the procedure; the translator must
    carry the generated term, not the `deriving` clause. -/
inductive Colour where
  | red | green | blue
  deriving DecidableEq

def colourEq (a b : Colour) : Bool := decide (a = b)

example : colourEq Colour.red Colour.red = true := rfl
example : colourEq Colour.red Colour.blue = false := rfl

/-- A RECURSIVE inductive. The generated procedure is structurally recursive — just barely
    within what LOGOS's Giménez guard accepts — and its `match` is nested. -/
inductive Tree where
  | leaf
  | node : Tree → Tree → Tree
  deriving DecidableEq

def treeEq (a b : Tree) : Bool := decide (a = b)

example : treeEq Tree.leaf Tree.leaf = true := rfl
example : treeEq (Tree.node Tree.leaf Tree.leaf) Tree.leaf = false := rfl

/-- A structure with two fields, so the generated procedure conjoins two sub-decisions. -/
structure Pt where
  x : Nat
  y : Nat
  deriving DecidableEq

def pairEq (a b : Pt) : Bool := decide (a = b)

example : pairEq ⟨1, 2⟩ ⟨1, 2⟩ = true := rfl
example : pairEq ⟨1, 2⟩ ⟨1, 3⟩ = false := rfl

/-- A HAND-WRITTEN instance, for contrast: this one the translator can carry faithfully. -/
inductive Flag where
  | on | off

instance : DecidableEq Flag := fun a b =>
  match a, b with
  | Flag.on, Flag.on => isTrue rfl
  | Flag.off, Flag.off => isTrue rfl
  | Flag.on, Flag.off => isFalse (fun h => Flag.noConfusion h)
  | Flag.off, Flag.on => isFalse (fun h => Flag.noConfusion h)

def flagEq (a b : Flag) : Bool := decide (a = b)

example : flagEq Flag.on Flag.off = false := rfl

/-- BOUNDARY. `Int` is OPAQUE in LOGOS with axiomatised order laws; no decision procedure and
    no `Decidable (Eq Int a b)` instance exists. -/
def intEq (a b : Int) : Bool := decide (a = b)

/-- BOUNDARY. `List` is not in the LOGOS prelude at all. -/
def listEq (a b : List Nat) : Bool := decide (a = b)

end T07_009
