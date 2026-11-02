/-!
@case      T06-014
@tier      6 · Propositions, equality, classical reasoning
@title     `native_decide` — LOGOS has the same two axioms, but a DIFFERENT evaluator in the TCB
@lean      native_decide tactic, Lean.reduceBool, Lean.ofReduceBool, Decidable.decide
@expect    degrade
@target    `Lean.reduceBool`   ⟼ `Global("reduceBool")`   — `add_declaration`, an AXIOM
           `Lean.ofReduceBool` ⟼ `Global("ofReduceBool")` — `add_declaration`, an AXIOM
           A `native_decide` proof of `p` becomes
           `ofReduceBool (decide p inst) true <proof that reduceBool (decide p inst) = true>`,
           where the inner proof is `refl Bool true` — because the KERNEL closes the gap by
           running the CBV evaluator on `reduceBool`'s argument during `reduce_step`.
           Plus a loss record: the trusted evaluator is not the same program.
@evidence  kernel/src/prelude.rs:1189-1194 (doc: "The `native_decide` trust boundary (TCB
           additions, exactly as Lean's `native_decide` adds `ofReduceBool` + its compiler)");
           kernel/src/prelude.rs:1195-1220 (`register_native_decide` — both names registered
           with `ctx.add_declaration`, i.e. as opaque AXIOMS: `reduceBool : Bool → Bool` and
           `ofReduceBool : Π(a b:Bool). Eq Bool (reduceBool a) b → Eq Bool a b`);
           kernel/src/reduction.rs:104-115 ("Native reduction: `reduceBool t` evaluates `t` with
           the fast CBV evaluator … This is the ONLY place the trusted evaluator enters kernel
           reduction — gated behind the `reduceBool` primitive"); kernel/src/certificate.rs:28
           (`PRELUDE_VERSION = "logos-coc-1"` — the version string that is supposed to record
           what was trusted); kernel/src/certificate.rs:63-75 (`recheck` rebuilds the context
           from `StandardLibrary::register`, so it re-runs the SAME evaluator — an independent
           re-check does not escape this TCB entry)
@loss      EVALUATOR_IDENTITY. Lean's `native_decide` trusts the Lean compiler, its C code
           generator, the resulting native object code, and the `Decidable` instance's compiled
           form. LOGOS's trusts `crate::eval::eval_bool`, a CBV interpreter inside the kernel
           (reduction.rs:109-115). The two axioms have the same TYPES, so the translation is
           type-correct and the certificate re-checks — but the proposition "this `Bool`
           computation evaluates to `true`" is now being asserted by a DIFFERENT program. A
           `native_decide` proof that is sound in Lean is not thereby sound in LOGOS, and vice
           versa. The loss record must name both evaluators so a reviewer can see what changed.
@loss      TCB_EXPANSION_IS_INHERITED. `reduceBool`/`ofReduceBool` are genuine `add_declaration`
           axioms (prelude.rs:1206,1208) — unlike `decide`, `of_decide_eq_true`, `Eq_rec_dep`,
           `Eq_sym` and `Eq_trans`, which the prelude deliberately PROVES. Every theorem whose
           proof term mentions them has a strictly larger trusted base than one that does not,
           and nothing in the certificate distinguishes them: `PRELUDE_VERSION` is a single
           constant string (certificate.rs:28) covering the whole prelude. The translator must
           therefore emit a per-declaration loss record; a corpus-level `#print axioms`
           equivalent cannot be recovered afterwards.
@guard     Catches translating `native_decide` to `decide`. They are not interchangeable:
           `decide` reduces by ordinary kernel normalization under the 10000-step fuel
           (reduction.rs:52-69) and is in NO ONE'S trusted base, whereas `reduceBool` short-
           circuits to the evaluator. Downgrading `native_decide` to `decide` will usually fail
           (fuel exhaustion returns a PARTIALLY reduced term, silently — reduction.rs:57-60 —
           so the `rfl` does not close and the failure looks like a mysterious type mismatch),
           and upgrading `decide` to `native_decide` silently enlarges the TCB.
@guard     Catches emitting `reduceBool` as a DEFINITION with body `λb. b`. Its doc comment says
           it "is the identity" (prelude.rs:1190), which invites exactly this. But making it a
           definition destroys the mechanism: `try_reflection_reduce`/the `reduceBool` arm in
           `reduce_step` matches on `Term::Global("reduceBool")` applied to an argument
           (reduction.rs:111-115), and a delta-transparent definition would be unfolded to the
           identity BEFORE that arm is reached, so the evaluator never runs and the `rfl` fails.
@guard     Catches emitting a `Term::Hole` for the `b` argument of `ofReduceBool`. Because
           `types_equal` matches `Hole` against anything (type_checker.rs:1594-1597), a
           `native_decide` certificate with a hole in the result position would be accepted for
           ANY `Bool` — turning a trusted-evaluator appeal into an unconditional one.
@oracle    Emitted terms typecheck and reduce; both globals are emitted via `add_declaration`
           with the prelude's exact spellings; every declaration whose proof term mentions them
           carries a loss record naming EVALUATOR_IDENTITY; and no `Term::Hole` appears in the
           emitted `ofReduceBool` application.
@note      INDEX.md marks this `degrade`, which this file follows. A translator whose policy is
           "never expand the trusted base" (the T06-009/010/011 stance) may legitimately
           escalate this to `reject` instead — the two axioms ARE in the LOGOS prelude, so it is
           not a base expansion, but the evaluator behind them is not the one the Lean proof
           appealed to. Either verdict is defensible; a SILENT translation is not.
-/

namespace T06_014

/-- The two axioms, in Lean's spelling, named so a harness can see they are referenced. -/
def theEvaluatorAxiom : Bool → Bool := Lean.reduceBool

/-- `ofReduceBool : Π(a b:Bool). reduceBool a = b → a = b` — identical type in both systems. -/
def theBridgeAxiom : ∀ (a b : Bool), Lean.reduceBool a = b → a = b := Lean.ofReduceBool

/-- The bridge term a `native_decide` proof elaborates to, written out by hand so the shape is
    explicit: `ofReduceBool a b h : a = b`. -/
theorem bridgeShape (a b : Bool) (h : Lean.reduceBool a = b) : a = b :=
  Lean.ofReduceBool a b h

/-- A goal that `decide` closes by ordinary kernel reduction — NO evaluator, no extra axioms.
    The translator must keep this on the `decide` path. -/
theorem smallByDecide : (7 : Nat) % 2 = 1 := by decide

/-- The same shape at a size where kernel normalization is the expensive option. LOGOS's
    `normalize` has a hard fuel of 10000 and returns a PARTIALLY reduced term when it runs out
    (reduction.rs:52-69), so a translator that downgrades a `native_decide` here gets a silent
    non-reduction rather than an error. -/
def bigCheck : Bool := (List.range 2000).all (fun n => n < 2000)

/-- `Decidable.decide` in the position `native_decide` would occupy. Kept as `decide` here so
    the file compiles without a compiled `Decidable` instance; the translator's job is to
    notice that the Lean SOURCE of a real `native_decide` proof is indistinguishable from this
    one except for the tactic name. -/
theorem bigCheckHolds : bigCheck = true := by native_decide

end T06_014
