/-!
@case      T07-001
@tier      7 · Typeclasses
@title     `class` declaration — LOGOS has no class concept; a class is a structure plus
           entries in a flat instance list
@lean      class, class methods, instance-implicit binders `[C α]`, `#print` of a class's
           structure representation
@expect    degrade
@target    Three separate LOGOS actions, none of which is "declare a class":
           1. `Context::add_structure(name, params, fields)` — the carrier. This yields the
              inductive, the constructor `{name}_mk`, one DEFINITION per field whose body is a
              `Term::Match`, and the `StructInfo` that gates eta. The sort is hard-coded `Type 0`.
           2. `Context::set_binder_kinds(user, kinds)` with `ParamKind::Instance` at each
              `[C α]` position, so the elaborator knows to resolve rather than unify there.
           3. `Context::add_instance(ty, value)` per instance — see T07-002.
           A class with NO instances is not a class at all in LOGOS: `class_heads` is computed
           from the registered INSTANCES, so a head with no instance is never recognised as an
           instance premise.
@evidence  kernel/src/context.rs:271-369 (`add_structure` — the only record-declaring API);
           kernel/src/context.rs:300 (`let ind_type = pis(params, Term::Sort(Universe::Type(0)))`
           — the sort is hard-coded); kernel/src/context.rs:71 (`instances: Arc<Vec<(Term,
           Term)>>` — the ENTIRE class system is one flat vector of pairs);
           kernel/src/context.rs:184-186 (`add_instance` pushes onto it; there is no key, no
           priority, no class name, no coherence check);
           kernel/src/elaborate.rs:1016-1027 (`ParamKind::{Explicit, Implicit, Instance}` — the
           only place "instance" appears as a concept, and it is a per-BINDER annotation, not a
           declaration form); kernel/src/context.rs:201-207 (`set_binder_kinds`);
           kernel/src/elaborate.rs:1174-1189 (`class_heads` — the set of "typeclass heads" is
           derived by scanning every registered INSTANCE's conclusion for its head `Global`);
           kernel/src/elaborate.rs:1321-1341 (`resolve_instance_at` — a linear scan of
           `ctx.instances()` in insertion order, first success wins, depth-bounded by
           `MAX_INSTANCE_DEPTH = 64` at :1160)
@loss      NO_CLASS_IDENTITY. Lean's `class C` creates a distinguished environment entry with
           an attribute, an instance table keyed on it, priorities, an `outParam` mask, default
           instances, and a `Std.TagAttribute` that `#print axioms`-style tooling can query.
           LOGOS has none of that. After translation, a class is indistinguishable from any
           other structure, and its instance-hood is inferred *a posteriori* from the shape of
           whatever was pushed into `instances`. Nothing can be reported back: there is no way
           to ask "what are the instances of `C`", only "what unifies with this goal".
@loss      EMPTY_CLASS_IS_INVISIBLE. Because `class_heads` (elaborate.rs:1178-1189) is built by
           folding over `ctx.instances()`, a class with zero registered instances contributes no
           head. A `[C α]` binder for such a class is therefore NOT treated as an instance
           premise during resolution — `try_instance` (elaborate.rs:1346-1400) classifies a
           parameter as a premise only when its head is in `class_heads`. So a class declared
           before any of its instances, or a class whose instances all failed to translate,
           degrades into an ordinary implicit argument that unification must solve. It usually
           cannot, and the error is `no typeclass instance found` (elaborate.rs:1147-1152) or a
           stuck metavariable — never "this class has no instances".
@loss      SORT_PINNING. `add_structure` hard-codes `Type 0` (context.rs:300). A Lean `class` in
           `Prop` (a *mixin* like `Std.Associative`) or in `Type u` for `u ≥ 1` cannot be
           declared this way at all. `PropClass` and `BigClass` below are the instances; this is
           the same wall as T03-012 and it is what makes T07-011 a rejection.
@guard     Catches emitting a class as an `add_inductive` + hand-written projections. The
           projections MUST come from `add_structure`, because only `add_structure` registers
           the `StructInfo` that `try_structure_eta` consults (type_checker.rs:1391-1417,
           gated at :1473-1478). Without it, eta silently stops firing for that class and
           `⟨c.f, c.g⟩ ≡ c` becomes unprovable — which breaks every `extends`-flattening
           argument in T07-005.
@guard     Catches forgetting `set_binder_kinds`. A `[C α]` binder translated as an ordinary
           `Pi` is an EXPLICIT parameter in LOGOS: `elab_surface`'s application path
           (elaborate.rs:948-962) falls back to `implicit_args` leading-implicits when the
           recorded kinds do not match the argument count, so every call site must then pass
           the dictionary by hand. Worse, the fallback at :958-962 will happily accept a
           DIFFERENT arity than intended and silently reinterpret which arguments are implicit.
@guard     Catches assuming method names survive. `add_structure` emits projections named
           `format!("{name}_{field}")` (context.rs:310-362, looked up by `elaborate_dot` at
           elaborate.rs:1284), so `Semigroup.op` becomes the flat `String` `"Semigroup_op"`.
           Lean's `Semigroup.op` and a user's `def Semigroup_op` collide — see T02-020.
@oracle    For each class below: exactly one `add_structure` call with the class's name; one
           `set_binder_kinds` call per consumer carrying `ParamKind::Instance` at the right
           positions; and a loss record naming NO_CLASS_IDENTITY. `PropClass` and `BigClass`
           must be escalated (E_STRUCTURE_SORT_NOT_TYPE0). A class declared with no instance
           must additionally carry EMPTY_CLASS_IS_INVISIBLE.
-/

namespace T07_001

/-- The plain case: a one-method class over a `Type 0` carrier. -/
class Pointed (α : Type) where
  point : α

instance : Pointed Nat := ⟨0⟩

/-- A consumer with an instance-implicit binder — this is what needs `ParamKind::Instance`. -/
def getPoint (α : Type) [Pointed α] : α := Pointed.point

example : getPoint Nat = 0 := rfl

/-- A two-method class, so the emitted structure has two projection definitions
    (`Magma_op` and `Magma_unit`) that must be applied to the PARAMETERS first. -/
class Magma (α : Type) where
  op : α → α → α
  unit : α

instance : Magma Nat := ⟨Nat.add, 0⟩

def combine (α : Type) [Magma α] (a b : α) : α := Magma.op a b

example : combine Nat 2 3 = 5 := rfl

/-- A class with NO instance anywhere. In LOGOS this contributes nothing to `class_heads`, so a
    `[Lonely α]` binder is not even recognised as an instance premise. -/
class Lonely (α : Type) where
  witness : α → α

def useLonely (α : Type) [Lonely α] (a : α) : α := Lonely.witness a

/-- BOUNDARY. A `Prop`-valued class (a mixin). `add_structure` hard-codes `Type 0`. -/
class PropClass (α : Type) : Prop where
  holds : ∀ a : α, a = a

instance : PropClass Nat := ⟨fun _ => rfl⟩

/-- BOUNDARY. A class whose carrier is `Type 1`. Same wall, one level up. -/
class BigClass (α : Type 1) where
  pick : α

end T07_001
