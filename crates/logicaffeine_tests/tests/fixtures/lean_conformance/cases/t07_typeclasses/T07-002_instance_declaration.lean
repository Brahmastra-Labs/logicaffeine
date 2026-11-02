/-!
@case      T07-002
@tier      7 · Typeclasses
@title     `instance` declaration — one `add_definition` plus one `add_instance` entry
@lean      instance, named instance, `instance : C α := ⟨…⟩`, anonymous instance naming
@expect    translate
@target    Two calls per instance, in this order:
           1. `Context::add_definition(name, <class type>, <dictionary value>)` — the instance's
              VALUE, an application of the structure constructor `{Class}_mk`.
           2. `Context::add_instance(<class type>, Global(name))` — the search entry. Note the
              second argument is the instance's *value term*; pushing the unfolded body here
              rather than the `Global` makes the resolved instance delta-transparent at every
              use site and defeats the elaborator's deliberate choice to "[b]ind directly (not
              via normalizing unification) so the instance stays structured rather than being
              δ-unfolded into its body".
@evidence  kernel/src/context.rs:180-186 (`add_instance(ty, value)`: "Register a typeclass
           instance: a `value` of type `ty` (e.g. `mk Nat Zero` of type `Inhabited Nat`). The
           elaborator resolves an instance-implicit argument by searching these for a `ty` that
           unifies with the required class type"); kernel/src/context.rs:71 (the backing store
           is `Arc<Vec<(Term, Term)>>` — insertion-ordered, unkeyed);
           kernel/src/context.rs:210-213 (`instances()` returns the raw slice);
           kernel/src/elaborate.rs:1136-1153 (deferred instance obligations are resolved after
           the explicit arguments, and the solution is bound "directly (not via normalizing
           unification) so the instance stays structured");
           kernel/src/context.rs:442 (`add_definition` — delta-transparent, and the only
           definition form: a `theorem` is just this with a `Prop` type);
           kernel/src/prelude.rs:923-1026 (`decEqBool` is registered exactly this way — a
           definition, then `add_instance` — the prelude's own worked example)
@guard     Catches emitting ONLY the `add_instance` and not the `add_definition`. The instance
           would then resolve, but its value is an anonymous term with no global name, so it
           cannot be referred to, cannot be overridden, and appears nowhere in the certificate's
           dependency set — the certificate's `proof_term` would inline the whole dictionary.
@guard     Catches emitting ONLY the `add_definition`. Nothing scans definitions during
           resolution: `resolve_instance_at` iterates `ctx.instances()` and nothing else
           (elaborate.rs:1331-1340). A defined-but-unregistered instance is invisible, and since
           `class_heads` is also computed from `ctx.instances()` (elaborate.rs:1178-1189), the
           class itself becomes invisible too — a `[C α]` binder stops being treated as an
           instance premise at all. The error surfaces as `no typeclass instance found`
           (elaborate.rs:1147-1152) at a call site far from the omission.
@guard     Catches pushing the instance's BODY as the `value`. `add_instance(ty, body)` makes
           every resolution substitute the unfolded dictionary, so a program with n uses of one
           instance carries n copies of it. LOGOS's `Term` is a tree with no sharing
           (term.rs:443-545) and substitution is O(size) per binder (type_checker.rs:1088), so
           this is a quadratic blow-up, not just an aesthetic one. Push `Global(name)`.
@guard     Catches translating the instance's TYPE by its Lean name rather than by its
           elaborated form. The `ty` in `add_instance` is matched by UNIFICATION against the
           goal (elaborate.rs:1372-1375), so it must be the fully applied class type
           (`App(Global("Pointed"), Global("Nat"))`), not a `Global("instPointedNat")` and not
           a partially applied head.
@guard     Catches losing the anonymous instance's generated name. Lean names anonymous
           instances `instPointedNat`-style; LOGOS keys are flat `String`s (context.rs:449), so
           two anonymous instances of the same class at the same type — legal in Lean, and
           exercised by `dup₁`/`dup₂` below — generate the SAME name and the second
           `add_definition` silently replaces the first, while BOTH `add_instance` entries
           remain. The instance vector then contains two entries whose `Global` resolves to one
           body. See T02-020 and T07-008.
@oracle    For each instance below: exactly one `add_definition` and exactly one `add_instance`,
           in that order; the `add_instance` value is a `Term::Global` naming that definition;
           the `add_instance` type is the fully-applied class type; and every emitted instance
           name is distinct.
-/

namespace T07_002

class Pointed (α : Type) where
  point : α

/-- Anonymous instance. Lean generates a name; the translator must too, and it must be unique. -/
instance : Pointed Nat := ⟨0⟩

/-- Named instance. The name is the `add_definition` key and the `add_instance` value. -/
instance pointedBool : Pointed Bool := ⟨false⟩

/-- Two instances at DIFFERENT types — two vector entries, both reachable. -/
def usesNat : Nat := Pointed.point
def usesBool : Bool := Pointed.point

example : usesNat = 0 := rfl
example : usesBool = false := rfl

/-- An instance whose value is a non-trivial term, so the `add_definition`/`add_instance`
    split actually matters for term size. -/
class Sized (α : Type) where
  size : α → Nat

instance sizedNat : Sized Nat := ⟨fun n => n + n + n⟩

def measure (α : Type) [Sized α] (a : α) : Nat := Sized.size a

example : measure Nat 2 = 6 := rfl

/-- TWO instances of the same class at the SAME type. Legal in Lean — the later one wins by
    default priority ordering. In LOGOS both land in the flat vector and the FIRST wins
    (elaborate.rs:1331-1340), which is the opposite order. -/
class Tagged (α : Type) where
  tag : Nat

instance dup₁ : Tagged Nat := ⟨1⟩
instance dup₂ : Tagged Nat := ⟨2⟩

def whichTag : Nat := Tagged.tag (α := Nat)

/-- Lean resolves this to the LAST-declared instance. -/
example : whichTag = 2 := rfl

end T07_002
