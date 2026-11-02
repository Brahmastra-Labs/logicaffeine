/-!
@case      T02-013
@tier      2 · Declarations
@title     `@[simp]`, `@[ext]` and attributes generally — LOGOS has no attribute system
@lean      @[simp], @[ext], @[instance], @[inline], attribute … , Lean environment extensions
@target    Context::add_definition(name, ty, body)  + a loss record; for the *effects* that
           matter, an explicit re-encoding — `Context::add_instance` for `@[instance]`,
           `Context::add_hint` for a lemma intended to drive `auto`
@expect    degrade
@evidence  kernel/src/context.rs:20-100 (the `Context` has no attribute map; the only
           attribute-shaped surfaces are the purpose-built `instances`, `coercions`,
           `binder_kinds`, `implicit_args` and `hints` fields, each written by its own
           dedicated API); kernel/src/context.rs:658-674 (`add_hint`/`get_hints`/`is_hint` —
           the *entire* extensible-lemma-database surface LOGOS has, a `Vec<String>` for the
           `auto` tactic); kernel/src/simp.rs (the simplifier is a fixed set of built-in rules,
           not an attribute-populated rewrite set — there is no `simp` lemma registry to add
           to); kernel/src/context.rs:184-186 (`add_instance` — the only way an `@[instance]`
           attribute's effect can be reproduced)
@loss      ATTRIBUTE_SYSTEM. Every attribute is erased. Nothing in the emitted LOGOS context
           records that a lemma was `@[simp]`, that a structure had `@[ext]`, or that a
           definition was `@[inline]`.
@loss      SIMP_SET. The single most consequential erasure. In Lean a large fraction of proofs
           are `by simp`, and they close *because* of the ambient simp set. LOGOS's script-level
           `simp` takes **no lemma-list argument at all** (it is arity-0) and rewrites with a
           fixed built-in rule set, so there is no target to translate the simp set into. Any
           Lean proof whose term was produced by `simp` must be translated as its *elaborated
           proof term*, never as the tactic.
@guard     Catches translating `@[simp] theorem foo` by emitting the theorem and assuming
           downstream `simp` will find it. It will not: there is no registry. A translator that
           also rewrites Lean `by simp` proofs into LOGOS `simp` produces scripts that fail with
           an unhelpful "no progress" rather than a missing-lemma error.
@guard     Catches silently dropping `@[instance]`. `instance` the *command* and `@[instance]`
           the *attribute applied later* have the same effect in Lean. A translator that only
           handles the command form emits the second as a plain definition, and every
           downstream instance-implicit argument fails to synthesise (see T02-010).
@guard     Catches translating `@[ext]` as if it produced logical content. `@[ext]` generates a
           *lemma* (`Foo.ext`) — that generated declaration is what must be translated, if
           anything; the attribute itself is inert. The same rule as `deriving` (T02-017):
           translate the handler's *output*, never the clause.
@guard     Catches using `add_hint` as a general attribute dumping ground. `add_hint` feeds
           exactly one consumer — the `auto` tactic's fallback (context.rs:654-657) — and
           entries are tried in insertion order. Registering a large Lean simp set as hints
           changes `auto`'s search space and its termination behaviour.
@oracle    Every attributed declaration is emitted; the loss report lists each attribute by
           name with ATTRIBUTE_SYSTEM; `@[instance]` additionally produces an `add_instance`
           call; and no emitted LOGOS tactic script relies on a lemma that was only reachable
           through a Lean attribute.
-/

namespace T02_013

def size (n : Nat) : Nat := n + 0

/-- A `@[simp]` lemma. In Lean this changes the behaviour of every later `simp`; after
    translation it is an ordinary `Prop`-typed definition and nothing consults it. -/
@[simp] theorem size_eq (n : Nat) : size n = n := Nat.add_zero n

/-- A proof that closes *only* because of the ambient simp set above. Its elaborated term is
    what must be translated — the tactic text has no LOGOS counterpart. -/
example (n : Nat) : size (size n) = n := by simp

structure Box where
  val : Nat

/-- `@[ext]` generates `Box.ext` and `Box.ext_iff`. Those generated declarations are the
    translatable artefacts. -/
@[ext] theorem Box.ext' : ∀ {a b : Box}, a.val = b.val → a = b := by
  intro a b h
  cases a; cases b; simp_all

class Marked (α : Type) where
  tag : Nat

def markedNatImpl : Marked Nat := ⟨1⟩

-- The attribute form of `instance`. A translator handling only the `instance` *command*
-- misses this one entirely.
attribute [instance] markedNatImpl

example : Marked.tag (α := Nat) = 1 := rfl

/-- A codegen-only attribute: logically inert, and there is not even a LOGOS surface where it
    could matter. -/
@[inline] def fastAdd (a b : Nat) : Nat := a + b

-- Attribute applied after the fact, to a pre-existing declaration. The translator must read
-- the *final* environment, not the declaration site.
theorem fastAdd_eq (a b : Nat) : fastAdd a b = a + b := rfl
attribute [simp] fastAdd_eq

end T02_013
