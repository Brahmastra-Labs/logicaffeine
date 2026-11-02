/-!
@case      T07-014
@tier      7 · Typeclasses
@title     Typeclass method projection — a method call is a structure projection applied
           parameters-first, then the dictionary
@lean      class method call, `C.method`, dot notation on a dictionary, generalized field
           notation, method with its own explicit arguments
@expect    translate
@target    `C.method (α := α) (self := inst) a b` ⟼
           `App(App(App(App(Global("C_method"), <α>), <inst>), <a>), <b>)`.
           The projection is the ordinary DEFINITION that `add_structure` emitted — body a
           `Term::Match` on the dictionary — and it takes the structure's PARAMETERS first, then
           the receiver, then the method's own arguments. There is no `Term::Proj`.
@evidence  kernel/src/context.rs:271-369 (`add_structure` — "a projection
           `{name}_{fieldᵢ} : Π(params). Π(s:{name} params). Tᵢ` for each field (its body a
           `match` on `s`)", :265-267; the projection type is built by
           `pis(params, pi("s", s_applied, …))` at :330-362, so params come BEFORE the receiver);
           kernel/src/term.rs:443-545 (the complete `Term` enum — there is NO `Proj` variant);
           kernel/src/elaborate.rs:1267-1299 (`elaborate_dot` resolves `x.field` by looking up
           `format!("{hname}_{field}")` at :1284, where `hname` is the head `Global` of `x`'s
           type); kernel/src/type_checker.rs:1391-1417 (`try_structure_eta`, gated by the
           `structures` registry at context.rs:153-167 and reachable only from
           `def_eq_normalized` at :1473-1478 — never from `normalize`);
           kernel/src/elaborate.rs:1136-1153 (the dictionary argument itself is a deferred
           instance obligation, solved after the explicit arguments)
@guard     Catches emitting the projection with the receiver FIRST. `add_structure` binds the
           structure's parameters before the receiver (context.rs:330-362), so a two-parameter
           class's method is `C_method α β inst a b`, not `C_method inst α β a b` and not
           `C_method inst a b`. `twoParams` below is the discriminating case. Getting the order
           wrong produces a term whose `infer_type` fails — which is the GOOD outcome; the bad
           outcome is a class whose parameters happen to have the same type as its first method
           argument, where the mis-ordered application typechecks and computes nonsense.
@guard     Catches emitting the method as an opaque axiom. If `C_method` were an
           `add_declaration` (context.rs:434) rather than the `Match`-bodied definition
           `add_structure` produces, then `C_method α (C_mk α f) a ≡ f a` would NOT hold, and
           every `rfl` that computes through an instance — which is most of them — fails.
           `computesThroughInstance` below is the case.
@guard     Catches inlining the `Match` at each use site instead of calling the projection
           definition. The term typechecks, but (a) it blows up term size, and LOGOS's `Term` is
           a tree with no sharing (term.rs:443-545) while substitution is O(size) per binder
           (type_checker.rs:1088), so this is quadratic; and (b) `try_structure_eta` is gated on
           the `structures` registry entry (context.rs:159-167), which records the PROJECTION
           NAMES — an inlined `Match` is not recognised as a projection, so eta silently stops
           firing and `C_mk α (C_method α c) ≡ c` becomes unprovable.
@guard     Catches losing the flat-name collision. Projections are named
           `format!("{name}_{field}")` (context.rs:310-362) into a flat `String` keyspace
           (context.rs:449), so Lean's `Pointed.point` and a user's `def Pointed_point` both key
           to `"Pointed_point"` and the second silently replaces the first. `Pointed_point`
           below is declared deliberately to exercise this. See T02-020.
@guard     Catches the method whose own arguments include ANOTHER instance premise. The
           dictionary is resolved as a deferred obligation AFTER the explicit arguments
           (elaborate.rs:1016-1027,1136-1140), so a method taking `[D β]` has two obligations
           resolved in one pass, and their order matters if one pins the other's type variable.
           `nestedPremise` below exercises it.
@guard     Catches assuming eta holds under `normalize`. It does not: `try_structure_eta` is
           reached only from `def_eq_normalized` (type_checker.rs:1473-1478). A tactic or a
           translator step that compares two dictionaries by normalizing both and testing
           syntactic equality will report a mismatch that `def_eq` would have accepted.
           See T03-011.
@oracle    Every emitted method call is `Global("{Class}_{field}")` applied parameters-first,
           then the dictionary, then the method's arguments; `computesThroughInstance` reduces
           under `normalize`; the reconstruction in `etaRoundTrip` is accepted by `def_eq`; and
           no emitted term contains an inlined projection `Match`.
-/

namespace T07_014

/-- One parameter, one method. The projection is `Pointed_point α inst`. -/
class Pointed (α : Type) where
  point : α

instance : Pointed Nat := ⟨7⟩

/-- Method call in its plainest form. -/
def usePoint (α : Type) [Pointed α] : α := Pointed.point

/-- THE COMPUTATION the projection must support: unfolding the definition to its `Match` and
    iota-reducing against the constructor. An opaque axiom would break this. -/
theorem computesThroughInstance : usePoint Nat = 7 := rfl

/-- Directly against a literal dictionary, so the `Match` reduces with no resolution involved. -/
theorem computesOnLiteral : (Pointed.point : Nat) = 7 := rfl

/-- TWO class parameters, so the projection takes BOTH before the receiver. -/
class Convert (α : Type) (β : Type) where
  convert : α → β

instance : Convert Nat Bool := ⟨fun n => n > 0⟩

def twoParams (α β : Type) [Convert α β] (a : α) : β := Convert.convert a

example : twoParams Nat Bool 3 = true := rfl

/-- A method with its OWN explicit arguments, applied after the receiver. -/
class Binop (α : Type) where
  op : α → α → α

instance : Binop Nat := ⟨Nat.add⟩

def useBinop (α : Type) [Binop α] (a b : α) : α := Binop.op a b

example : useBinop Nat 2 3 = 5 := rfl

/-- Dot notation on an explicit dictionary — `elaborate_dot`'s path, which looks up
    `format!("{hname}_{field}")` from the head of the receiver's type. -/
def dotOnDictionary (inst : Binop Nat) (a b : Nat) : Nat := inst.op a b

example : dotOnDictionary ⟨Nat.mul⟩ 3 4 = 12 := rfl

/-- ETA ROUND TRIP: rebuilding a dictionary from its projections must be `def_eq` to the
    original. This holds only via `try_structure_eta`, and only inside `def_eq` — never under
    `normalize`. -/
theorem etaRoundTrip (inst : Pointed Nat) : (⟨Pointed.point⟩ : Pointed Nat) = inst := rfl

/-- A method whose own argument carries a SECOND instance premise: two obligations resolved in
    one elaboration pass. -/
class Show0 (α : Type) where
  render : α → Nat

instance : Show0 Nat := ⟨fun n => n⟩
instance : Show0 Bool := ⟨fun b => if b then 1 else 0⟩

class Wrap (α : Type) where
  wrap : α → α

instance : Wrap Nat := ⟨fun n => n + 1⟩

def nestedPremise (α β : Type) [Wrap α] [Show0 β] (a : α) (b : β) : α × Nat :=
  (Wrap.wrap a, Show0.render (Show0.render b))

example : nestedPremise Nat Bool 1 true = (2, 1) := rfl

/-- THE FLAT-NAME COLLISION. In LOGOS both `Pointed.point`'s projection and this definition key
    to the `String` `"Pointed_point"`, and the second registration silently wins. -/
def Pointed_point : Nat := 999

example : Pointed_point = 999 := rfl
example : (Pointed.point : Nat) = 7 := rfl

end T07_014
