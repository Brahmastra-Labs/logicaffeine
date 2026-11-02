/-!
@case      T05-006
@tier      5 · Elaboration surface
@title     Default arguments `(n : Nat := 3)` — the default value must be inlined at every call
@lean      optional parameters with default values, `optParam`, structure field defaults
@expect    degrade
@target    At each call site, the default's translated VALUE inlined as an ordinary positional
           argument. For the declaration itself, an ordinary `Term::Pi` — `optParam` is erased,
           because LOGOS has no optional-parameter form and no `optParam` wrapper.
@evidence  kernel/src/term.rs:464-469 (`Term::Pi { param, param_type, body_type }` — three
           fields, no default slot);
           kernel/src/elaborate.rs:1016-1027 (`ParamKind` has exactly `Explicit`, `Implicit`,
           `Instance` — there is no `Optional` kind, so a parameter with a default cannot be
           marked as fillable);
           kernel/src/context.rs:199-206 (`set_binder_kinds` is the only per-parameter metadata
           `Context` holds; nothing stores a default term);
           kernel/src/context.rs:271-369 (`add_structure(name, params, fields)` takes fields as
           name/type pairs — a structure field's default has nowhere to go either, and
           `add_structure` generates a single `{name}_mk` constructor of full arity);
           kernel/src/elaborate.rs:1029-1045 (`elaborate_app` consumes `explicit_args`
           positionally and never fills a missing one)
@loss      DEFAULT_VALUE_INDIRECTION. Lean's `optParam` keeps the default as part of the
           function's TYPE, so it is one shared definition that every call site refers to.
           After translation it is a value copied into each call site. Consequences: (a) term
           size grows with call-site count, and since substitution is O(size) per binder
           comparison (type_checker.rs:1088) that is a real cost on a large library; (b) the
           defaults are frozen at translation time, so changing the Lean default no longer
           changes existing translated call sites; (c) a call site that a human later reads in
           LOGOS cannot tell which arguments the Lean author actually wrote.
@loss      STRUCTURE_FIELD_DEFAULTS. `add_structure` (context.rs:271-369) emits one
           `{name}_mk` constructor taking every field. A Lean structure-instance literal that
           omits a defaulted field must be emitted with that field's default value supplied
           explicitly. The knowledge that it *was* a default is lost. Lean's `..` is a
           SEPARATE mechanism — it supplies `_`, not the default (T05-016) — so the two must
           not share a translation path.
@guard     **Translate the post-elaboration `Expr`, never the surface `Syntax`.** Lean has
           already inserted every default by the time the `Expr` exists — including defaults
           that are themselves elaborated terms mentioning earlier parameters. A translator on
           `Syntax` must re-elaborate the default expression *in the call site's context*,
           which means reimplementing the elaborator. This is the same failure the auto-param
           case (T05-007) makes fatal.
@guard     Catches emitting the declaration's `Pi` with `optParam` still wrapped. Lean's type
           for `withDefault` is `Nat → optParam Nat 3 → Nat`, and `optParam` is a real constant
           in Lean's environment. Emitting `Global("optParam")` gives an unresolvable name; and
           declaring it as an axiom (context.rs:434, unchecked) gives an opaque type former
           that nothing inhabits, so every call site stops typechecking.
@guard     Catches dropping the default argument from the call site's spine rather than
           inlining it. The spine is then one argument short, and `infer_type`'s `Pi` rule
           (type_checker.rs:156-167) substitutes the *next* argument into the defaulted
           binder. For `withDefault` the following argument is also a `Nat`, so this
           typechecks and silently shifts every later argument by one.
@guard     Catches defaults that MENTION earlier parameters. `sized n (m := n * 2)` has a
           default whose value depends on `n`; the inlined term must be the default with `n`
           substituted by the call site's actual argument, not the default's open term with a
           free `Term::Var("n")` — which, in LOGOS's NAMED representation (term.rs:450), will
           silently capture any enclosing binder called `n` instead of failing.
@guard     Catches structure-field defaults. `{ val := 1 }` for a structure with a defaulted
           `tag` must emit `Wrap_mk 1 0`, not `Wrap_mk 1`. `add_structure`'s constructor has
           full arity (context.rs:271-369), so the short form fails — unless the omitted field
           type happens to make the partial application well-typed.
@oracle    The emitted spine for a call that omitted a default has the same arity as the
           fully-explicit call and is term-for-term identical to it; the output contains no
           `Global("optParam")`; and the loss record contains DEFAULT_VALUE_INDIRECTION with
           the defaulted parameter named.
-/

namespace T05_006

/-- The simplest default. -/
def withDefault (n : Nat) (m : Nat := 3) : Nat := n + m

example : withDefault 1 = 4 := rfl
example : withDefault 1 = withDefault 1 3 := rfl

/-- A default that MENTIONS an earlier parameter — the inlined value must be substituted. -/
def sized (n : Nat) (m : Nat := n * 2) : Nat := n + m

example : sized 5 = 15 := rfl
example : sized 5 = sized 5 10 := rfl

/-- Two defaults, the second depending on the first. -/
def twoDefaults (a : Nat := 2) (b : Nat := a + 1) : Nat := a * 10 + b

example : twoDefaults = 23 := rfl
example : twoDefaults 5 = 56 := rfl

/-- A default in a NON-final position, so omitting it does not simply shorten the spine. -/
def midDefault (a : Nat) (b : Nat := 7) (c : Nat) : Nat := a * 100 + b * 10 + c

example : midDefault 1 (c := 9) = 179 := rfl

/-- `optParam` written out by hand — the same thing Lean's `:=` desugars to. -/
def rawOpt (n : Nat) (m : optParam Nat 4) : Nat := n + m

example : rawOpt 1 = 5 := rfl

/-- Structure field defaults: `add_structure`'s `{name}_mk` takes every field. -/
structure Wrap where
  val : Nat
  tag : Nat := 0

def w1 : Wrap := { val := 1 }

example : w1 = { val := 1, tag := 0 } := rfl
example : w1.tag = 0 := rfl

/-- A default whose value is a non-trivial term, so inlining it at many call sites is what
    costs term size. -/
def bigDefault (n : Nat) (m : Nat := 1 + 2 + 3 + 4 + 5 + 6 + 7 + 8 + 9 + 10) : Nat := n + m

example : bigDefault 0 = 55 := rfl

end T05_006
