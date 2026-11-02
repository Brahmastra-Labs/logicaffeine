/-!
@case      T07-012
@tier      7 · Typeclasses
@title     Bundled vs unbundled structures — the same mathematics, two very different LOGOS
           encodings, and only one of them resolves
@lean      bundled structure (`structure Mon where carrier; mul; …`) vs unbundled class
           (`class Monoid (α : Type) where mul; …`), `CoeSort` on a bundled carrier
@expect    degrade
@target    UNBUNDLED (`class C (α : Type)`): `add_structure("C", &[("a", Sort(Type 0))],
           <fields>)` + `set_binder_kinds` with `ParamKind::Instance` at each `[C α]` + one
           `add_instance` per instance. Method calls resolve.
           BUNDLED (`structure Mon where carrier : Type; …`): `add_structure("Mon", &[],
           &[("carrier", Sort(Type 0)), …])` — a PARAMETERLESS structure whose first field is a
           type. Nothing about it is an instance; `Mon_carrier m` is an ordinary projection and
           every use site must pass `m` explicitly. Plus a loss record, because the bundled
           form loses the `CoeSort` that makes `(m : Mon)` usable as a type.
@evidence  kernel/src/context.rs:271-369 (`add_structure(name, params, fields)` — `params`
           become the inductive's arity and are bound BEFORE the receiver in every projection's
           type, :310-362; so the unbundled/bundled distinction is exactly "is the carrier a
           param or a field"); kernel/src/context.rs:300 (both forms land in `Type 0`, so a
           bundled structure whose `carrier : Type` field makes it morally `Type 1` is
           mis-sorted — see the guard below); kernel/src/context.rs:184-186 and
           kernel/src/elaborate.rs:1174-1189 (`class_heads` is derived from registered
           instances' conclusion heads, so a bundled structure — which is never registered as an
           instance — is never a "class" and its fields are never resolved automatically);
           kernel/src/elaborate.rs:1267-1299 (`elaborate_dot` looks up
           `format!("{hname}_{field}")` — the same mechanism for both forms, which is why they
           look interchangeable and are not); a grep for `CoeSort` in
           `crates/logicaffeine_kernel/src` returns NOTHING (see T05-012)
@loss      CARRIER_POSITION. In the unbundled form the carrier is a PARAMETER, so
           `C_mul α inst a b`; in the bundled form it is a FIELD, so
           `Mon_mul m a b` where `a b : Mon_carrier m`. These are different terms with different
           telescopes, and a development that mixes the two — as Mathlib does, with
           `Monoid M` unbundled and `MonCat` bundled — needs an explicit bridge in both
           directions. LOGOS has no `CoeSort`, so the bundled-to-unbundled direction cannot be
           implicit: every `(m : Mon)` used as a type must become `Mon_carrier m` by hand.
@loss      NO_RESOLUTION_FOR_BUNDLED. A bundled structure's "instance" is a VALUE the user
           passes. Nothing resolves it, because resolution only scans `ctx.instances()`
           (elaborate.rs:1331) and a bundled value is not registered there. So the translation
           of a bundled development is correct but every call site grows an explicit argument —
           which is exactly what the Lean source used the bundling to avoid, and which changes
           the arity of every function. Downstream, `set_binder_kinds` for those functions must
           mark that argument `Explicit`, not `Instance`.
@loss      BUNDLED_SORT. A bundled structure whose first field is `carrier : Type` morally lives
           in `Type 1`, because it quantifies over `Type 0`. `add_structure` declares it at
           `Type 0` regardless (context.rs:300). LOGOS's cumulativity
           (type_checker.rs:1330-1360) hides the error at most use sites — `Type 0 ≤ Type 1`
           holds, so terms typecheck — while Lean would have rejected the same declaration at
           that sort. This is the T11-019 shape: a universe constraint only Lean's
           non-cumulative check catches.
@guard     Catches translating a bundled structure AS a class — i.e. calling `add_instance` on
           a bundled value. Its type is `Mon`, whose head `Global("Mon")` then enters
           `class_heads` (elaborate.rs:1178-1189), and from that moment EVERY parameter of type
           `Mon` anywhere in the development is classified as an instance PREMISE by
           `try_instance` (elaborate.rs:1365) and resolved automatically — silently substituting
           the first registered `Mon` for whichever one the caller meant to pass. This is the
           worst bug in this case, because the resulting term typechecks and computes a
           plausible answer.
@guard     Catches the reverse: translating an unbundled class as a bundled structure by moving
           the carrier into the field list. The projections' telescopes change
           (context.rs:310-362 binds `params` before the receiver), so `C_mul α inst a b`
           becomes `C_mul inst a b`, and every emitted call site must agree. A translator that
           chooses per-declaration rather than per-CLASS will emit both spellings for one class.
@guard     Catches dropping the `CoeSort`. `(m : Mon)` used in type position is `Mon_carrier m`;
           there is no `CoeSort` in LOGOS (T05-012) and `add_coercion` (context.rs:190) is a
           flat `(from, to, coe)` list over TERMS, not sorts, so it cannot express this. The
           translator must insert `Mon_carrier` explicitly at every type-position occurrence,
           and must record that it did.
@oracle    Each form is emitted with a consistent carrier position across the whole class; no
           `add_instance` is emitted for a bundled structure; every type-position use of a
           bundled value is rewritten to an explicit `_carrier` projection; and the loss record
           names CARRIER_POSITION and NO_RESOLUTION_FOR_BUNDLED.
-/

namespace T07_012

/-- UNBUNDLED: the carrier is a class PARAMETER, so it becomes an `add_structure` param and
    every projection takes it before the receiver. -/
class UMonoid (α : Type) where
  mul : α → α → α
  one : α
  one_mul : ∀ a, mul one a = a

instance : UMonoid Nat := ⟨Nat.mul, 1, Nat.one_mul⟩

/-- Resolution works: the instance argument is synthesised. -/
def ucombine (α : Type) [UMonoid α] (a b : α) : α := UMonoid.mul a b

example : ucombine Nat 3 4 = 12 := rfl

/-- BUNDLED: the carrier is a FIELD, so the whole structure is parameterless and nothing about
    it is an instance. -/
structure BMonoid where
  carrier : Type
  mul : carrier → carrier → carrier
  one : carrier
  one_mul : ∀ a, mul one a = a

def natBMonoid : BMonoid := ⟨Nat, Nat.mul, 1, Nat.one_mul⟩

/-- No resolution: the "instance" is an explicit argument, and the carrier must be projected
    out by hand wherever a type is needed. This is the `CoeSort` LOGOS does not have. -/
def bcombine (m : BMonoid) (a b : m.carrier) : m.carrier := m.mul a b

example : bcombine natBMonoid (3 : Nat) (4 : Nat) = (12 : Nat) := rfl

/-- The BRIDGE from bundled to unbundled, which a mixed development needs in both directions.
    Note it constructs an unbundled instance from a bundled value — it cannot be an `instance`
    itself without making every `BMonoid` argument resolvable (the guard above). -/
def toUnbundled (m : BMonoid) : UMonoid m.carrier :=
  ⟨m.mul, m.one, m.one_mul⟩

/-- The bridge the other way. -/
def toBundled (α : Type) [inst : UMonoid α] : BMonoid :=
  ⟨α, UMonoid.mul, UMonoid.one, UMonoid.one_mul⟩

example : (toBundled Nat).carrier = Nat := rfl

/-- A function over the bundled form used at a value produced by the bridge — exercises the
    round trip, which is where a carrier-position mismatch shows up. -/
example : bcombine (toBundled Nat) (3 : Nat) (4 : Nat) = (12 : Nat) := rfl

/-- A morphism between bundled objects — the shape that makes bundling worth it in Lean and
    that costs an extra explicit argument at every call site in LOGOS. -/
structure BHom (m n : BMonoid) where
  toFun : m.carrier → n.carrier
  map_one : toFun m.one = n.one

def idHom (m : BMonoid) : BHom m m := ⟨id, rfl⟩

end T07_012
