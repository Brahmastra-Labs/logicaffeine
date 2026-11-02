/-!
@case      T03-014
@tier      3 · Inductives and structures
@title     `class` as a structure — LOGOS has no class concept, only a record plus a list
@lean      class … where, `Lean.ClassExtension`, method projections, instance-implicit binders
@target    Context::add_structure(class_name, params, fields) for the carrier, plus one
           Context::add_instance(class_type, instance_value) per instance, plus
           Context::set_binder_kinds(f, [.., ParamKind::Instance, ..]) for every declaration
           with an instance-implicit binder; and a loss record
@expect    degrade
@evidence  kernel/src/context.rs:20-100 (there is no `classes` map — a class is *only* a
           structure in the `structures` registry plus entries in the `instances` `Vec`);
           kernel/src/context.rs:257-276 and :299-301 (`add_structure`, sort hard-coded to
           `Type 0`); kernel/src/context.rs:181-186 (`add_instance` — flat
           `Vec<(Term, Term)>`, "Searched by unifying `type` against the required class type");
           kernel/src/context.rs:210-213 (`instances()` returns the raw slice: resolution is
           first-match-wins in insertion order);
           kernel/src/elaborate.rs:1017-1027 (`ParamKind::Instance`: resolution is DEFERRED
           until all explicit arguments have been unified, so the class's type variables are
           pinned first); kernel/src/context.rs:199-208 (`set_binder_kinds` — the only carrier
           for *which* binder is instance-implicit);
           kernel/src/elaborate.rs:1267-1299 (a method call is a projection:
           `Class_method <params> inst`)
@loss      CLASS_IDENTITY. Nothing distinguishes a class from an ordinary structure after
           translation. `Context` has a `structures` registry and an `instances` list; a
           structure with no instances and a class with no instances are the same object.
@loss      INSTANCE_SEARCH_SEMANTICS. Lean's resolution is a tabled, backtracking search over a
           discrimination tree with priorities, `outParam` modes and default instances. LOGOS's
           is a linear scan of a `Vec` taking the first unifying entry. Priorities, backtracking
           and default instances have no carrier at all (T07-006, T07-007, T07-008).
@loss      METHOD_IS_A_PROJECTION. A class method becomes `Class_method params inst`, an
           ordinary definition whose body is a `Term::Match` (context.rs:341-362). It is not
           special, it is not reducible-by-attribute, and it is not resolved by class
           membership — only by the flat name `{Class}_{method}` (elaborate.rs:1285).
@guard     Catches registering the class carrier with `add_inductive` + hand-written
           projections instead of `add_structure`. The carrier is then absent from the
           `structures` registry, so definitional eta never fires for it
           (type_checker.rs:1391-1400) — and eta is exactly what makes
           `⟨Class.method inst⟩ = inst` hold, which every "instance is determined by its
           methods" argument relies on.
@guard     Catches omitting `set_binder_kinds`. A declaration `f (α : Type) [C α] (n : Nat)`
           has kinds `[Explicit, Instance, Explicit]`. Without them LOGOS falls back to the
           `implicit_args` count model (context.rs:82-86), which can only express *leading*
           implicits — so an interleaved instance binder is mis-modelled and elaboration inserts
           arguments at the wrong positions.
@guard     Catches recording an instance binder as `Implicit`. `ParamKind::Instance` is
           deliberately resolved **after** the explicit arguments (elaborate.rs:1017-1027); an
           `Implicit` binder is solved by unification immediately. Marking `[C α]` implicit
           therefore triggers instance search while `α` is still a metavariable, and the flat
           first-match scan (context.rs:181-186) returns whichever entry unifies with an
           unsolved variable — in practice the first instance in the list, regardless of type.
@guard     Catches a class whose carrier is not `Type 0`. A `class` in `Prop` (a bundled
           proposition) or one storing a `Type`-valued field is outside `add_structure`'s
           reach entirely — that is the T03-012 rejection, and a class does not get an
           exemption.
@guard     Catches translating a class *without* its instances. The carrier alone is inert:
           every `[C α]` obligation downstream fails to synthesise, and the error names the use
           site rather than the missing `add_instance`.
@oracle    The class carrier is in the `structures` registry with its methods as `projections`;
           each Lean instance produces one `add_instance` whose class type unifies with the
           carrier applied to that instance's arguments, in Lean's resolution order; every
           declaration with an instance-implicit binder has matching `binder_kinds`; and the
           loss report contains CLASS_IDENTITY and INSTANCE_SEARCH_SEMANTICS.
-/

namespace T03_014

/-- A single-method class. Structurally it is a one-field record. -/
class Pointed (α : Type) where
  point : α

instance : Pointed Nat := ⟨0⟩
instance : Pointed Bool := ⟨true⟩

/-- The method call is a projection applied to the resolved instance. -/
example : (Pointed.point : Nat) = 0 := rfl
example : (Pointed.point : Bool) = true := rfl

/-- An **interleaved** instance binder: kinds are `[Explicit, Instance, Explicit]`. The
    `implicit_args` "N leading implicits" model cannot express this. -/
def repeatPoint (α : Type) [Pointed α] (n : Nat) : Nat := n

example : repeatPoint Nat 3 = 3 := rfl

/-- An instance-implicit binder that comes **after** the explicit argument it depends on —
    the exact case `ParamKind::Instance`'s deferral exists for. If it were marked `Implicit`,
    resolution would fire while `α` is still unknown. -/
def pointOfSame {α : Type} (_witness : α) [Pointed α] : α := Pointed.point

example : pointOfSame (5 : Nat) = 0 := rfl
example : pointOfSame true = true := rfl

/-- A two-method class, so the `projections` order is observable. Both methods return `Nat`,
    which makes a permutation type-invisible. -/
class Measured (α : Type) where
  weight : α → Nat
  height : α → Nat

instance : Measured Bool where
  weight := fun b => if b then 1 else 0
  height := fun _ => 100

example : Measured.weight true = 1 := rfl
example : Measured.height true = 100 := rfl
example : Measured.weight true ≠ Measured.height true := by decide

/-- A **recursive** instance: its own type carries an instance-implicit binder, so resolution
    re-enters the flat scan for the inner obligation. -/
class Sizeable (α : Type) where
  size : α → Nat

instance : Sizeable Nat where
  size := fun n => n

instance instSizeableList (α : Type) [Sizeable α] : Sizeable (List α) where
  size := fun xs => xs.foldl (fun acc a => acc + Sizeable.size a) 0

example : Sizeable.size [1, 2, 3] = 6 := rfl

/-- Eta on the class carrier: an instance is determined by its methods. This `rfl` holds
    *only* because `Pointed` is in the `structures` registry. -/
example (inst : Pointed Nat) : Pointed.mk (@Pointed.point Nat inst) = inst := rfl

/-- A class with a **parameter that is not the carrier** — the shape where first-match-wins
    resolution diverges from Lean, because two entries can both unify. -/
class Convert (α : Type) (β : Type) where
  convert : α → β

instance : Convert Nat Nat := ⟨fun n => n⟩
instance : Convert Bool Nat := ⟨fun b => if b then 1 else 0⟩

example : Convert.convert (3 : Nat) = (3 : Nat) := rfl
example : Convert.convert true = (1 : Nat) := rfl

end T03_014
