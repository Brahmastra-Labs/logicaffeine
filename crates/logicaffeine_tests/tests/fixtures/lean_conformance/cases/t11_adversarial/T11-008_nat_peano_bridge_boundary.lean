/-!
@case      T11-008
@tier      11 · Adversarial and soundness traps
@title     The `Nat` literal / Peano bridge boundary — where the acceleration stops being an
           acceleration
@lean      Nat literal as a match scrutinee, Nat.rec on a literal, arithmetic on large
           numerals, `decide` over a `Nat` proposition
@expect    degrade
@target    `Term::Lit(Literal::Nat(n))` throughout, never the expanded `Succ^n Zero`, PLUS a
           loss record naming the step-cost boundary the bridge does not remove.
@evidence  kernel/src/term.rs:38-44 (`Literal::Nat(BigInt)` is "DEFINITIONALLY EQUAL to `Succ`
           applied `n` times to `Zero`" and "stores the count as one `BigInt` instead of `n`
           heap nodes"); kernel/src/reduction.rs:329-345 (`extract_constructor`'s bridge —
           note what it actually does: `Nat(n)` yields `Succ`'s index with the single argument
           `Lit(Nat(n-1))`. It peels ONE `Succ` PER CALL. The acceleration is in the
           REPRESENTATION, not in the reduction: a match that recurses down to `Zero` still
           takes n steps); kernel/src/type_checker.rs:1480-1493 (the `def_eq` side: two
           literals compare by `BigInt` equality in O(1) at :1485, but a literal against a
           `Zero`/`Succ`-headed TERM is compared by `nat_peano_step` one `Succ` at a time at
           :1486-1491); kernel/src/type_checker.rs:1430-1440 (`nat_peano_step` builds
           `App(Global("Succ"), Lit(Nat(n-1)))` — one step, one allocation);
           kernel/src/reduction.rs:52-69 (`normalize`'s hard 10000-step fuel, silently
           partial); kernel/src/reduction.rs:337-341 (the negative clamp: `n ≤ 0` yields
           `Zero`, "so the peel TERMINATES rather than descending toward −∞");
           kernel/src/type_checker.rs:297-301 (`infer_type` REJECTS a negative `Literal::Nat`)
@loss      BRIDGE_IS_REPRESENTATIONAL_NOT_COMPUTATIONAL. The bridge makes `Nat(1000000)` cost
           one node instead of a million, and makes literal-versus-literal equality O(1). It
           does NOT make `Nat` recursion cheap: `extract_constructor` peels exactly one `Succ`
           per call (reduction.rs:337-344), so any function defined by recursion on a `Nat` —
           which, in LOGOS, is EVERY `Nat` function, because the prelude ships no `Nat`
           arithmetic at all (prelude.rs:529-552 registers only `Nat`/`Zero`/`Succ`) — costs
           one `reduce_step` per unit of its argument. Against a 10000-step budget that is a
           hard ceiling around n ≈ 10^4, and Lean's GMP-accelerated `Nat` operations sail past
           it. The loss record must state this ceiling per declaration.
@loss      ASYMMETRIC_DEFEQ_COST. `def_eq(Lit(Nat(n)), Lit(Nat(m)))` is one `BigInt` comparison
           (type_checker.rs:1485). `def_eq(Lit(Nat(n)), Succ^n Zero)` is n peels
           (:1486-1491). So the SAME mathematical fact costs O(1) or O(n) depending on which
           side the translator chose to emit — and the translator's encoding choice, not the
           source, decides whether a proof fits in the budget.
@guard     **TIER 11 THESIS.** The kernel's acceptance is not evidence of faithfulness, and
           here the specific trap is that the bridge LOOKS like it makes the two
           representations interchangeable, so a translator stops distinguishing them. They are
           definitionally equal and operationally very different. Emitting `Succ^n Zero`
           wherever a literal would do is definitionally correct and blows the budget (T11-005);
           emitting a literal where the source had an explicit `Nat.succ` application is also
           definitionally correct and changes which `def_eq` path runs. Neither is caught.
@guard     Catches the bridge being severed by a re-declared `Nat`. `extract_constructor` looks
           the constructor indices up BY NAME at reduction time — `idx_of("Zero")` and
           `idx_of("Succ")` against `ctx.get_constructors("Nat")` (reduction.rs:332-344). A
           translator that emits its own `Nat` (because it flattened a namespace, or because it
           declared `Nat` fresh rather than reusing the prelude's) makes `idx_of` return `None`,
           every literal silently stops being constructor-form, and every `Match` on a literal
           becomes stuck — which `normalize` reports as a normal form. See T11-003.
@guard     Catches the constructor ORDER assumption. The bridge uses the registration positions
           of `"Zero"` and `"Succ"` (reduction.rs:333-344), and `Match` is positional
           (type_checker.rs:254-260). The prelude registers `Zero` then `Succ`
           (prelude.rs:536-551), which agrees with Lean's `Nat.zero` then `Nat.succ` — so this
           one happens to line up, unlike `Bool` (T10-008). A translator must verify the
           agreement rather than assume it.
@guard     Catches the NEGATIVE boundary, which behaves differently in the two halves of the
           bridge. A negative `Literal::Nat` is REFUSED by `infer_type`
           (type_checker.rs:297-301) but CLAMPED TO `Zero` by `extract_constructor`
           (reduction.rs:337-341). So a negative literal that reaches the kernel errors when
           typed and computes as zero when matched on. If a translator emits one — e.g. by
           translating a `Nat` subtraction with the `Int` primitive (T10-017) and re-typing the
           result — the same term is simultaneously ill-typed and well-behaved, depending on
           which kernel entry point is called first.
@guard     Catches assuming the bridge exists in only one kernel. `Literal::Nat`'s own
           documentation says the bridge is implemented "in BOTH kernels" (term.rs:41-43), so
           `recheck.rs` will not catch a bridge-dependent error either — the independent
           re-check shares the acceleration.
@oracle    Every `Nat` numeral in the output is a `Term::Lit(Literal::Nat)`, never an
           application spine of `Succ`; the emitted `Nat` uses the PRELUDE's `Nat`/`Zero`/`Succ`
           globals so `extract_constructor`'s name lookup resolves; no emitted `Literal::Nat`
           carries a negative value; and a loss record with categories
           BRIDGE_IS_REPRESENTATIONAL_NOT_COMPUTATIONAL and ASYMMETRIC_DEFEQ_COST names the
           per-declaration step ceiling.
-/

namespace T11_008

/-! ### The bridge holds, in both directions and at both ends. -/

example : (0 : Nat) = Nat.zero := rfl
example : (4 : Nat) = Nat.succ (Nat.succ (Nat.succ (Nat.succ Nat.zero))) := rfl
example : Nat.succ (Nat.succ Nat.zero) = 2 := rfl

/-! ### Where it is O(1): literal against literal. -/

example : (123456789 : Nat) = 123456789 := rfl
example : (123456789 : Nat) ≠ 123456790 := by decide

/-! ### Where it is O(n): literal against a `Succ`-headed term. Each `Succ` costs one peel
     (type_checker.rs:1486-1491 via `nat_peano_step` at :1430-1440). -/

example : (3 : Nat) = Nat.succ 2 := rfl
example : (300 : Nat) = Nat.succ 299 := rfl

/-! ### Where the acceleration does NOT help: recursion. One `reduce_step` per unit. -/

def isZero : Nat → Bool
  | 0 => true
  | _ + 1 => false

/-- One peel. The bridge is doing its job. -/
example : isZero 1000000 = false := rfl

def down : Nat → Nat
  | 0 => 0
  | n + 1 => down n

/-- n peels. The bridge does not help at all; this is n `reduce_step` calls against a
    10000-step budget. -/
example : down 100 = 0 := rfl

/-- Past the budget. Lean needs its compiled evaluator here too, which is itself the signal
    that the reduction is long. -/
example : down 30000 = 0 := by native_decide

/-! ### The boundary between the two sides of the bridge, in one pair of terms. -/

/-- O(1): both sides are literals. -/
example : (5000 : Nat) = 5000 := rfl

/-- O(n): one side is `Succ`-headed, so `def_eq` peels 5000 times. Same mathematical fact,
    different cost, and the translator's encoding choice is what decides which one the kernel
    runs. -/
example : (5000 : Nat) = Nat.succ 4999 := rfl

/-! ### The negative boundary. Lean's `Nat` cannot be negative at all, which is exactly why a
     translator that routes `Nat` arithmetic through LOGOS's `Int` primitives (T10-017) can
     manufacture a value the bridge was never meant to see. -/

example : (3 : Nat) - 8 = 0 := rfl

/-- The corresponding `Int` computation, whose result is `-5`. Re-typing that as a
    `Literal::Nat` produces a term `infer_type` refuses (type_checker.rs:297-301) and
    `extract_constructor` silently clamps to `Zero` (reduction.rs:337-341). -/
example : (3 : Int) - 8 = -5 := by decide

end T11_008
