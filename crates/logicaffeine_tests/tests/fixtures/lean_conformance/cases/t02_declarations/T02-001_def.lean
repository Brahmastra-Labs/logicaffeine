/-!
@case      T02-001
@tier      2 · Declarations
@title     `def` — a delta-transparent global definition
@lean      def, Lean.DefinitionVal, ReducibilityStatus.regular
@target    Context::add_definition(name: String, ty: Term, body: Term)
@expect    translate
@evidence  kernel/src/context.rs:442-444 (`add_definition` stores `name -> (ty, body)` in the
           `definitions` map); kernel/src/context.rs:54-57 (definitions are documented as
           transparent — "they unfold during normalization (delta reduction)");
           kernel/src/context.rs:474-476 (`get_definition_body` returns `None` for axioms,
           constructors and inductives — only definitions carry a body);
           kernel/src/context.rs:449-464 (`get_global` precedence: inductives → constructors →
           definitions → declarations)
@guard     Catches emitting a `def` as `add_declaration` (an axiom). That compiles, and every
           *statement* about the function still typechecks, but the body is gone: `double 2 = 4`
           becomes unprovable by `rfl`/`decide` because there is nothing to delta-unfold. The
           failure surfaces only in downstream proofs, arbitrarily far from the mistranslation.
@guard     Catches emitting the *body* where the *type* goes. `add_definition` takes
           `(name, ty, body)` positionally with both `Term`s adjacent and no validation at the
           call site — `add_definition` performs **no typecheck at all**, unlike Lean's
           `addDecl`. A swapped pair is accepted silently and only explodes later inside
           `infer_type` of some unrelated term.
@guard     Catches dropping the declaration's own typecheck. Lean's kernel checks
           `body : ty` at `addDecl` time. LOGOS's `add_definition` is a bare `HashMap::insert`.
           A conforming translator must call `infer_type`/`is_subtype` on the pair itself and
           refuse to register when it fails, or an ill-typed definition sits in the context
           poisoning everything that unfolds it.
@guard     Catches name shadowing: `add_definition` is `HashMap::insert`, so registering a name
           that already exists **silently overwrites** it with no error. See T02-020.
@oracle    After translation the LOGOS context has a *definition* (not a declaration) under the
           flattened name; `get_definition_body` returns `Some`; `infer_type(body)` is a subtype
           of the emitted type; and `normalize(App(Global(name), args))` reduces past the global.
-/

namespace T02_001

/-- The plain case: a closed definition with an explicit type ascription. -/
def double (n : Nat) : Nat := n + n

/-- No explicit type — Lean infers it, so the translator must read the *elaborated*
    `DefinitionVal.type`, never the surface syntax (there is none to read). -/
def quadruple (n : Nat) := double (double n)

/-- Delta-transparency is observable: this `rfl` only closes because `double` unfolds. -/
example : double 2 = 4 := rfl

/-- Transparency again, one level deeper — a definition whose body mentions another
    definition. Both must be registered as definitions for this to reduce. -/
example : quadruple 1 = 4 := rfl

/-- A definition whose *type* is itself a definition. `Nat` here arrives through a delta
    step, so the emitted type must be a `Global` that unfolds, not an inlined copy. -/
def Count : Type := Nat
def zeroCount : Count := (0 : Nat)

/-- A definition at the type level: the body is a `Sort`-valued term. LOGOS makes no
    distinction between a type-valued and a value-valued definition; both are
    `add_definition`. -/
def Endo (α : Type) : Type := α → α
def idNat : Endo Nat := fun n => n

/-- A dependent definition — the return type mentions an earlier parameter, so the emitted
    `ty` is a real `Term::Pi` telescope, not an arrow chain. -/
def replicateType (n : Nat) (α : Type) : Type := Fin n → α

end T02_001
