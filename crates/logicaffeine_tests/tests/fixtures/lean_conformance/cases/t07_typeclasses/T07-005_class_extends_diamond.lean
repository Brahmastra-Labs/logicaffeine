/-!
@case      T07-005
@tier      7 · Typeclasses
@title     `class ... extends` and the diamond it creates — LOGOS has no coherence checking
@lean      class extends, parent projections, a Foo/Bar/Baz/Qux diamond, `toParent` coercions
@expect    degrade
@target    Per derived class, ONE `add_structure` whose field list is the FLATTENED union of the
           parent fields and the new ones — because `add_structure` gives one projection
           DEFINITION per field and nothing else, there is no parent-as-subobject notion and no
           `toParent` to project. Then, per parent, a synthesised "forget" instance:
           `add_definition("Bar_toFoo", Π(α). Bar α → Foo α, λα b. Foo_mk α (Bar_op α b))`
           plus `add_instance(Π(α). Bar α → Foo α, Global("Bar_toFoo"))` so that a `[Foo α]`
           goal can be discharged from a `Bar α` in scope. Plus a loss record.
@evidence  kernel/src/context.rs:271-369 (`add_structure` — a flat `fields` list; there is no
           parent parameter, no `extends`, no subobject field kind, and each field becomes an
           independent projection definition `{name}_{field}` whose body is a `Term::Match`);
           kernel/src/context.rs:300 (sort hard-coded to `Type 0`);
           kernel/src/context.rs:71,184-186 (`instances: Arc<Vec<(Term, Term)>>` and
           `add_instance` — no key, no priority, no coherence check, no diamond detection);
           kernel/src/elaborate.rs:1321-1341 (`resolve_instance_at` — a linear scan, FIRST
           MATCH WINS, and it stops at the first success without ever discovering that a second
           path exists); kernel/src/type_checker.rs:1391-1417 (`try_structure_eta`, gated by the
           `structures` registry at context.rs:159-167 and reachable only from
           `def_eq_normalized` at :1473-1478, never from `normalize`)
@loss      NO_PARENT_SUBOBJECT. Lean's `class Bar α extends Foo α` stores the parent as a single
           `toFoo` field (or flattens it, depending on `structureDiamondWarning` and on whether
           the parent is itself a flat structure), and `Bar.toFoo` is a real projection with a
           real coercion. The LOGOS encoding flattens, so the parent's *identity* is gone: after
           translation there is no term that IS the `Foo α` inside a `Bar α`, only a
           reconstruction `Foo_mk α (Bar_op α b)`. Two reconstructions from the same `b` are
           equal only by structure eta — which holds in `def_eq` (type_checker.rs:1391-1417) but
           NOT under `normalize`, so any proof that reaches the equality through `normalize`
           fails.
@loss      NO_COHERENCE. Lean warns (and Mathlib engineers around) the case where two paths
           through a diamond give different parent instances. LOGOS cannot even ask the
           question: `add_instance` appends to a vector (context.rs:184-186) and resolution
           returns at the first success (elaborate.rs:1334-1339). A diamond where
           `Qux → Bar → Foo` and `Qux → Baz → Foo` disagree is not detected, not warned about,
           and resolves to whichever path happens to be earlier in insertion order. The loss
           record must list every class reachable by more than one path so a human can check
           what LOGOS cannot. See T07-013 for the reject case.
@loss      ORDER_DEPENDENCE. Because resolution is insertion-ordered, the MEANING of a
           development depends on the order in which the translator emitted its instances.
           Translating the same Lean file with declarations visited in a different order can
           produce a different — still typechecking — LOGOS development. The loss record must
           pin the emission order so the result is reproducible.
@guard     Catches emitting a `toParent` PROJECTION that does not exist. There is no
           `Term::Proj` (term.rs:443-545) and `add_structure` names projections
           `format!("{name}_{field}")` (context.rs:310-362, looked up by `elaborate_dot` at
           elaborate.rs:1284). `Global("Bar_toFoo")` resolves to nothing unless the translator
           explicitly emits it as a definition; a translator that assumes `add_structure`
           created it gets `UnboundVariable`.
@guard     Catches emitting the parent as a FIELD (`toFoo : Foo α`) instead of flattening.
           That is also a valid encoding, but it must be chosen CONSISTENTLY: mixing the two
           across a hierarchy means `Bar_op` exists on one class and must be reached as
           `Foo_op α (Bar_toFoo α b)` on another, and no call site can know which. Pick one and
           record it in the loss record.
@guard     Catches forgetting the forget-instances. Without `add_instance(Π(α). Bar α → Foo α,
           …)` a `[Foo α]` goal cannot be discharged from a `[Bar α]` binder at all, because
           resolution only ever looks in `ctx.instances()` (elaborate.rs:1331). Every
           Lean function that takes the PARENT class and is called with a CHILD instance breaks
           — which is most of a hierarchy's surface.
@guard     Catches forget-instances that shadow base instances. A forget-instance's conclusion
           is `Foo ?α` for a metavariable `?α`, which unifies with EVERY `Foo` goal. Inserted
           before the concrete `Foo Nat` instance, it captures all of them and then recursively
           demands a `Bar ?α` that may not exist — and the failure is reported as
           `no typeclass instance found for Foo Nat` (elaborate.rs:1147-1152), naming the wrong
           class. Forget-instances must be emitted AFTER every base instance.
@oracle    One `add_structure` per class with the agreed field convention; one forget-definition
           plus one `add_instance` per `extends` edge, emitted after all base instances; a loss
           record naming NO_PARENT_SUBOBJECT, NO_COHERENCE and the emission order; and the
           diamond in `useQux` below is listed in the loss record's multiple-path set.
-/

namespace T07_005

/-- The root of the diamond. -/
class Foo (α : Type) where
  base : α

/-- Left branch. -/
class Bar (α : Type) extends Foo α where
  left : α → α

/-- Right branch. -/
class Baz (α : Type) extends Foo α where
  right : α → α

/-- The join. Both parents carry a `Foo α`, so `Qux α` reaches `Foo α` by two paths. -/
class Qux (α : Type) extends Bar α, Baz α where
  join : α → α → α

instance : Foo Nat := ⟨0⟩
instance : Bar Nat := ⟨fun n => n + 1⟩
instance : Baz Nat := ⟨fun n => n * 2⟩
/-- Note Lean's OWN diamond resolution here: `Qux.mk` is
    `{α} → [toBar : Bar α] → (α → α) → (α → α → α) → Qux α`. Lean stored the `Bar` parent as a
    subobject and FLATTENED the `Baz` parent down to its own `right` field, because `Baz`'s
    `Foo` part is already reachable through `Bar`. That choice is invisible in the source and
    is exactly the structural decision LOGOS has no way to represent or record. -/
instance : Qux Nat := ⟨fun n => n * 2, fun a b => a + b⟩

/-- A function over the PARENT class, called with a CHILD instance in scope. This is the call
    that needs the synthesised forget-instance. -/
def useFoo (α : Type) [Foo α] : α := Foo.base

/-- Reaching `Foo` through `Bar`. -/
def viaBar (α : Type) [Bar α] : α := useFoo α

/-- Reaching `Foo` through `Baz`. -/
def viaBaz (α : Type) [Baz α] : α := useFoo α

/-- Reaching `Foo` through BOTH — the diamond. LOGOS resolves this by insertion order and
    never discovers that a second path exists. -/
def useQux (α : Type) [Qux α] : α := useFoo α

example : useQux Nat = 0 := rfl

/-- The parent projection Lean generates and LOGOS does not. -/
def parentOfBar (α : Type) [inst : Bar α] : Foo α := inst.toFoo

/-- Parent-field access through a child — the flattening case. -/
def baseViaQux (α : Type) [Qux α] : α := Foo.base

example : baseViaQux Nat = 0 := rfl

/-- A method from each level, so the flattened field list really does span all four classes. -/
def allFour (α : Type) [Qux α] (a b : α) : α :=
  Qux.join (Bar.left a) (Baz.right b)

example : allFour Nat 1 2 = 6 := rfl

end T07_005
