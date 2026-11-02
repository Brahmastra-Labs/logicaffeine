/-!
@case      T05-011
@tier      5 · Elaboration surface
@title     `Coe` coercions and `↑` — LOGOS's coercion table is a flat, unordered list
@lean      Coe, CoeTail, CoeHead, CoeT, ↑x, automatic coercion insertion, transitive coercion
           chains
@expect    degrade
@target    The COERCION FUNCTION applied explicitly, taken from the elaborated `Expr`:
           `App(Global("boolToNat"), b)`. Plus one `Context::add_coercion(from, to, coe)` per
           Lean `Coe` instance, so LOGOS-side elaboration can insert it too.
@evidence  kernel/src/context.rs:190-196 (`add_coercion(from: Term, to: Term, coe: Term)`
           pushes onto a flat `Arc<Vec<(Term, Term, Term)>>`; `coercions()` returns the slice);
           kernel/src/elaborate.rs:1301-1330 (`resolve_coercion(ctx, mctx, from, to)` scans
           `ctx.coercions()` linearly and returns the FIRST triple that unifies — "a match
           commits the unification");
           kernel/src/term.rs:443-545 (there is no coercion term, no `mdata`, and no marker of
           any kind: an inserted coercion is indistinguishable from a function the user wrote);
           kernel/src/context.rs:184-188 (`add_instance` is a separate, equally flat list —
           LOGOS does not model `Coe` as a typeclass, so Lean's instance-resolution search does
           not carry over)
@loss      COERCION_TRANSITIVITY. Lean's coercion resolution is a typeclass search: `CoeT`
           chains `CoeHead`, `Coe` and `CoeTail`, so a path `A → B → C` is found automatically
           from two one-step instances. `resolve_coercion` (elaborate.rs:1301-1330) is a single
           linear scan over `(from, to, coe)` triples with NO transitive closure. Every
           composite path the Lean source relied on must be pre-composed and registered as its
           own triple, and the loss record must list which composites were synthesised — they
           are new definitions that did not exist in the source.
@loss      COERCION_INVISIBILITY. Lean marks an inserted coercion in the `Expr` (it is a
           recognisable application of a `Coe`-family constant, and Lean's delaborator can hide
           it again). LOGOS has no marker at all, so after translation an inserted coercion and
           a user-written function application are the same term. A round-trip cannot restore
           the `↑`, and a reader cannot tell where the source relied on coercion.
@guard     **Translate the post-elaboration `Expr`, never the surface `Syntax`.** Coercion
           insertion is decided by Lean's unifier: `(b : Nat)` where `b : Bool` inserts
           `boolToNat b` only because the expected type forced it. The `Syntax` contains no
           coercion at all. A translator on `Syntax` must reimplement `CoeT`'s search — head,
           body, tail, and the `Coe` output-parameter rules — and get the same answer Lean did,
           including where Lean chose NOT to coerce.
@guard     Catches the order sensitivity of the flat list. `resolve_coercion` takes the FIRST
           unifying triple (elaborate.rs:1305-1330). Two coercions `Bool → Nat` registered in
           either order both typecheck; only one of them is the one Lean chose. Because both
           produce a `Nat`, nothing downstream disagrees — the translated program simply
           computes different numbers. There is no priority mechanism and no coherence check.
@guard     Catches relying on transitivity. `Bit → Bool → Nat` below is two Lean instances and
           one automatic chain. Registering the two one-step triples and expecting
           `resolve_coercion` to compose them silently fails: the scan finds no
           `(Bit, Nat, _)` triple and the argument is left un-coerced, so the type mismatch
           surfaces at the kernel — or, if LOGOS's CUMULATIVITY (type_checker.rs:1330-1360)
           happens to bridge the gap at a sort, does not surface at all.
@guard     Catches dropping the coercion because "the types look close enough". LOGOS is
           cumulative where Lean is not, so `Prop ≤ Type 0 ≤ Type 1` is admitted
           (LOGOS_FACTS). A translator that decides a coercion is unnecessary on the basis
           that `is_subtype` accepts the term has just produced a term that means something
           different from the Lean source — this is the T11-001 trap reached through Tier 5.
@guard     Catches the `from`/`to` argument ORDER of `add_coercion`. The signature is
           `add_coercion(from, to, coe)` (context.rs:190); registering it reversed makes the
           scan look for the wrong direction. For a coercion between two types that both
           appear on both sides elsewhere in the table, the reversed triple can still unify
           against a different query and be applied backwards.
@guard     Catches a coercion into a FUNCTION or a SORT. `CoeFun`/`CoeSort` have no analogue at
           all (T05-012); they must not be registered as ordinary `add_coercion` triples, which
           would make `resolve_coercion` return a function used as if it were the coerced
           value.
@oracle    The emitted term contains the coercion function applied explicitly, matching the
           elaborated `Expr` application for application; `ctx.coercions()` contains one triple
           per Lean `Coe` instance plus any synthesised composite, in a recorded order; the
           emitted term typechecks; and the loss record contains COERCION_TRANSITIVITY listing
           every synthesised composite.
-/

namespace T05_011

/-- A one-step coercion. -/
def boolToNat (b : Bool) : Nat := if b then 1 else 0

instance : Coe Bool Nat := ⟨boolToNat⟩

/-- Explicit `↑`. -/
example : (↑true : Nat) = 1 := rfl

/-- Automatic insertion driven by the expected type — nothing in the source says `↑`. -/
def sumWith (b : Bool) (n : Nat) : Nat := n + b

example : sumWith true 5 = 6 := rfl

/-- A second, COMPETING `Bool → Nat` coercion path, registered later. Lean's instance
    resolution prefers the most recent; LOGOS's flat list prefers the first. -/
def boolToNat' (b : Bool) : Nat := if b then 100 else 200

/-- A TRANSITIVE chain: `Bit → Bool` and `Bool → Nat` compose automatically in Lean. -/
inductive Bit where
  | zero | one

def bitToBool : Bit → Bool
  | .zero => false
  | .one => true

instance : Coe Bit Bool := ⟨bitToBool⟩

/-- Lean finds `Bit → Bool → Nat` with no `Bit → Nat` instance declared. -/
def bitSum (b : Bit) (n : Nat) : Nat := n + (↑(↑b : Bool) : Nat)

example : bitSum .one 5 = 6 := rfl

/-- A coercion on a PARAMETERISED type: the triple's `from`/`to` contain metavariables that
    `resolve_coercion` commits by unification. -/
structure Wrapped (α : Type) where
  unwrap : α

instance {α : Type} : Coe (Wrapped α) α := ⟨Wrapped.unwrap⟩

def unwrapNat (w : Wrapped Nat) : Nat := w

example : unwrapNat ⟨7⟩ = 7 := rfl

/-- Coercion in ARGUMENT position, where the expected type comes from the callee's signature
    rather than from an ascription. -/
def takesNat (n : Nat) : Nat := n + 1

example : takesNat (↑false) = 1 := rfl

/-- Coercion inside a list literal — every element is coerced independently. -/
def bits : List Nat := [↑true, ↑false, ↑true]

example : bits = [1, 0, 1] := rfl

end T05_011
