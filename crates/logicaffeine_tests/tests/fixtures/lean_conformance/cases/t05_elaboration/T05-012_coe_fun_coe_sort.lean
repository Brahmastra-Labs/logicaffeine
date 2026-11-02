/-!
@case      T05-012
@tier      5 · Elaboration surface
@title     `CoeFun` / `CoeSort` — no analogue: LOGOS coercions go between two `Term`s, not into
           a function or a sort position
@lean      CoeFun, CoeSort, a structure used as a function, a structure used as a type
@expect    reject
@target    (none — no sound translation of the mechanism)
@evidence  kernel/src/context.rs:190-196 (`add_coercion(from: Term, to: Term, coe: Term)` — a
           coercion is a triple of TERMS; there is no "coerce when used in head position" and
           no "coerce when used in type position" form);
           kernel/src/elaborate.rs:1301-1330 (`resolve_coercion` is called when an ARGUMENT's
           type does not match an expected PARAMETER type — that is the only insertion site;
           nothing consults the coercion table when a non-function is applied or when a
           non-sort appears as a type);
           kernel/src/type_checker.rs:144-170 (the `App` rule: the head's type is whnf'd and
           must be a `Term::Pi`, else `KernelError::NotAFunction` — no coercion hook);
           kernel/src/type_checker.rs:172-200 (the `Match` rule requires the discriminant's
           type to be headed by a registered inductive, else `NotAnInductive`);
           kernel/src/context.rs:271-369 and :300-303 (`add_structure` hard-codes the structure's
           sort to `Type 0`, so a `CoeSort` target cannot even be a general sort)
@diag      E_NO_COE_FUN_COE_SORT — must name the instance (`CoeFun`/`CoeSort`), the source
           type, and the use site, and state that LOGOS inserts coercions only at argument
           positions. Must NOT emit a `Term::App` whose head is the un-coerced structure, and
           must NOT register the instance with `add_coercion`.
@guard     **Translate the post-elaboration `Expr`, never the surface `Syntax`.** In the
           elaborated `Expr`, `f x` where `f : Fn` is already `Fn.f f x` — the `CoeFun`
           projection is applied. So a translator reading `Expr` gets a perfectly ordinary
           application and this case never fires for translated *call sites*. What is rejected
           is translating the `CoeFun`/`CoeSort` INSTANCE itself, which is what a `Syntax`-level
           translator (or one that tries to make LOGOS-side elaboration behave the same) would
           attempt. The diagnostic must fire on the instance declaration, not on the uses.
@guard     Catches registering a `CoeFun` instance via `add_coercion` (context.rs:190). The
           triple would be `(Fn, Nat → Nat, Fn_f)`, and `resolve_coercion` would then fire at
           ARGUMENT positions — so passing an `Fn` where a `Nat → Nat` is expected would
           silently be coerced, which Lean's `CoeFun` does NOT do. The translated program
           accepts terms the source rejects.
@guard     Catches emitting the un-coerced application `App(Global("f_value"), x)`.
           `infer_type`'s `App` rule (type_checker.rs:144-170) whnfs the head's type; a
           structure type is headed by a `Global`, not a `Pi`, so this is `NotAFunction` — a
           clean failure, except when the structure is a one-field wrapper around a function
           AND the translator registered the structure with `add_structure`, whose eta rule
           (`try_structure_eta`, type_checker.rs:1391-1417) can make the wrapper and its field
           definitionally equal inside `def_eq`. Then the error moves somewhere unrelated.
@guard     Catches `CoeSort` specifically. The Lean idiom `(t : Ty)` used as a TYPE requires
           coercing a VALUE into a sort position. LOGOS has no rule that consults the coercion
           table when a term appears as a type: `infer_sort` simply demands a `Term::Sort`. And
           `add_structure` pins the structure's own sort to `Type 0` (context.rs:300-303), so even
           a hand-rolled encoding cannot target `Type 1+` or `Prop` — see T03-012.
@guard     Catches "just unfold it". Replacing `Fn` by its field type everywhere is sound for
           `CoeFun` on a one-field wrapper and unsound in general: a multi-field structure with
           a `CoeFun` carries data the function type does not, and any Lean theorem quantifying
           over `Fn` is no longer expressible. A translator that unfolds must do so only where
           it can prove the structure is a single-field wrapper, and record it as a separate,
           explicitly-named transformation — not as a coercion.
@guard     Catches over-rejection of ordinary `Coe`. `Coe A B` between two ordinary types is
           `degrade`, not `reject` (T05-011); the diagnostic must distinguish the `CoeFun`/
           `CoeSort` classes from `Coe`/`CoeTail`/`CoeHead`.
@oracle    Translator exits non-zero with E_NO_COE_FUN_COE_SORT on the `CoeFun` and `CoeSort`
           instance declarations below; writes no declaration for them; adds no entry to
           `ctx.coercions()` for them; and does not reject the ordinary `Coe` instance in
           T05-011.
-/

namespace T05_012

/-- A one-field wrapper around a function, the canonical `CoeFun` shape. -/
structure Fn where
  f : Nat → Nat

instance : CoeFun Fn (fun _ => Nat → Nat) := ⟨Fn.f⟩

/-- `g 3` applies a STRUCTURE. In the elaborated `Expr` this is `Fn.f g 3`. -/
def useFn (g : Fn) : Nat := g 3

example : useFn ⟨fun n => n + 1⟩ = 4 := rfl

/-- A MULTI-FIELD structure with a `CoeFun` — unfolding it to its function field would lose
    the `name` field, and every theorem quantifying over `Named` with it. -/
structure Named where
  name : Nat
  run  : Nat → Nat

instance : CoeFun Named (fun _ => Nat → Nat) := ⟨Named.run⟩

def useNamed (g : Named) : Nat := g 2 + g.name

example : useNamed ⟨10, fun n => n * 3⟩ = 16 := rfl

/-- A `CoeFun` whose target type DEPENDS on the value — the strongest form, and the one for
    which "unfold to the field type" is not even well-defined. -/
structure Arity where
  n : Nat

def ArityFun : Arity → Type
  | ⟨0⟩ => Nat
  | ⟨_ + 1⟩ => Nat → Nat

instance : CoeFun Arity ArityFun := ⟨fun a => match a with
  | ⟨0⟩ => (0 : Nat)
  | ⟨_ + 1⟩ => fun n => n⟩

/-- `CoeSort`: a VALUE used in type position. -/
structure Ty where
  carrier : Type

instance : CoeSort Ty Type := ⟨Ty.carrier⟩

/-- `x : t` where `t : Ty` — `t` appears where a sort is expected. -/
def idOfTy (t : Ty) (x : t) : t := x

example : idOfTy ⟨Nat⟩ 5 = 5 := rfl

/-- `CoeSort` into `Prop`, which `add_structure`'s hard-coded `Type 0` cannot target at all. -/
structure PropLike where
  holds : Prop

instance : CoeSort PropLike Prop := ⟨PropLike.holds⟩

def assertIt (p : PropLike) : Prop := p

example : assertIt ⟨True⟩ := trivial

end T05_012
