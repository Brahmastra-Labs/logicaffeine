/-!
@case      T00-014
@tier      0 · Kernel term layer
@title     Function eta — present in LOGOS, but ONLY inside `def_eq`, never in `normalize`
@lean      definitional eta for functions, `(fun x => f x) = f` by `rfl`
@expect    translate
@target    No special term is emitted: eta is a property of the CONVERSION relation, not of the
           syntax. The obligation is on the translator's own equality oracle — it must decide
           LOGOS definitional equality with `type_checker::def_eq`, never with
           `normalize` + structural comparison.
@evidence  kernel/src/type_checker.rs:1451-1466 (η-conversion in `def_eq_normalized`: when
           exactly ONE side is a `Lambda`, the binder is pushed into the context and the other
           side is APPLIED to `Var(param)` and normalized, then compared under the binder);
           kernel/src/type_checker.rs:1366-1375 (`def_eq` normalizes both sides first, then calls
           `def_eq_normalized` — so eta fires only after normalisation, on the normal forms);
           kernel/src/reduction.rs:36-120 (`reduce_step` has beta, delta, zeta, iota, quotient,
           primitive and `reduceBool` arms — there is NO eta-expansion arm, so `normalize` never
           eta-expands and two eta-equal terms have DIFFERENT normal forms);
           kernel/src/type_checker.rs:1594-1597 (`types_equal`, the alpha-equivalence fast path,
           has no eta arm either — it is called FIRST from both `def_eq` at :1367 and
           `is_subtype` at :1314)
@guard     Catches the translator using `normalize(a) == normalize(b)` as its definitional-
           equality oracle. This is the obvious, cheap implementation and it is WRONG for exactly
           this case: `normalize` (reduction.rs) has no eta rule, so `λx. f x` and `f` normalize
           to distinct terms and the oracle reports them unequal — while the LOGOS KERNEL, via
           `def_eq_normalized` at type_checker.rs:1451-1466, reports them equal. The translator
           then "repairs" a term that needed no repair: it eta-expands or eta-contracts to force
           syntactic agreement, changing the emitted term, or it decides the Lean `rfl` does not
           translate and emits a propositional proof obligation that has no proof.
@guard     Catches the one-sided shape of the rule. `def_eq_normalized`'s eta arms fire only when
           EXACTLY one side is a `Lambda` (`if !matches!(b, Term::Lambda { .. })`,
           type_checker.rs:1453). When both sides are lambdas the congruence arm at :1507-1515
           handles them, and when NEITHER is (both are stuck applications), eta is never
           attempted. So `f` versus `g` where both are eta-expandable but neither is syntactically
           a lambda is NOT decided by eta — a translator that assumes "LOGOS has eta, so any two
           extensionally equal functions are convertible" is assuming extensionality. It does not
           have it: `funext` is ABSENT from the LOGOS trusted base (prelude.rs; LOGOS_FACTS:
           "There is no `Classical.choice`, no `propext`, no `funext`"). See T06-011.
@guard     Catches eta interacting with the fuel budget. The eta arm calls
           `normalize(ctx, App(b, Var(param)))` (type_checker.rs:1456) — a FRESH `normalize` with
           a fresh 10000-step budget (reduction.rs:40), inside a comparison that has already
           normalized both sides once. A translator that eta-expands large functions to "help"
           multiplies the normalisation work and can push an equation that was decidable past the
           budget (T00-012).
@guard     Catches assuming eta holds for the DEPENDENT case for free. The eta arm extends the
           context with `param : param_type` taken from the LAMBDA side only
           (type_checker.rs:1454) and never checks that the non-lambda side's type is a `Pi` with
           that domain. If the two domains differ (which cumulativity permits — `Type 0 ≤ Type 1`
           at type_checker.rs:1333), `App(b, Var(param))` is still formed and normalized, and
           comparison proceeds under a binder typed by the wrong side.
@oracle    For each pair below, LOGOS `def_eq` returns true; and the harness additionally asserts
           that `normalize(lhs) != normalize(rhs)` structurally for at least the first pair —
           the test that a `normalize`-based oracle would have failed. No emitted term is
           eta-modified relative to the Lean `Expr`.
-/

namespace T00_014

/-- The canonical eta pair. Lean proves it by definitional eta; LOGOS by
    `def_eq_normalized`'s η arm, NOT by normalisation. -/
example (f : Nat → Nat) : (fun x => f x) = f := rfl

/-- Eta at a DEPENDENT function type: the domain and the codomain both matter. -/
example (g : (n : Nat) → Fin (n + 1)) : (fun n => g n) = g := rfl

/-- Eta on a function with an IMPLICIT binder — the binder kind is not carried by
    `Term::Lambda` (T00-005), so both sides lower to the same shape. -/
example (h : {α : Type} → α → α) : (fun {α} (a : α) => h a) = @h := rfl

/-- Two-argument eta, which needs the rule to fire under a binder: after the first η step the
    comparison continues in the extended context. -/
example (k : Nat → Nat → Nat) : (fun a b => k a b) = k := rfl

/-- Eta where the non-lambda side is a stuck APPLICATION rather than a variable. The η arm
    applies it to the bound variable and normalizes the result. -/
example (k : Nat → Nat → Nat) (a : Nat) : (fun b => k a b) = k a := rfl

/-- Eta combined with beta: the lambda's body is a redex, so `normalize` must run before the η
    arm can see that exactly one side is a lambda. -/
example (f : Nat → Nat) : (fun x => (fun y => f y) x) = f := rfl

/-- Eta under a `Pi` in a TYPE position, where the comparison is driven by `is_subtype`
    (type_checker.rs:1330-1360) rather than `def_eq` — `is_subtype` delegates non-`Pi`,
    non-`Sort` positions to `def_eq_normalized`, so eta is reachable there too. -/
def etaInType (F : (Nat → Nat) → Type) (f : Nat → Nat) (x : F (fun n => f n)) : F f := x

/-- The NEGATIVE control: extensionally equal, not eta-equal, not convertible. Lean needs
    `funext` here, and LOGOS has no `funext` at all. A translator that believes eta gives it
    extensionality will try to discharge this by conversion and silently fail. -/
theorem needsFunext (f g : Nat → Nat) (h : ∀ n, f n = g n) : f = g := funext h

end T00_014
