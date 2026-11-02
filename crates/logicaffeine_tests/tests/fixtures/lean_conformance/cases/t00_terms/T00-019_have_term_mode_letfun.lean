/-!
@case      T00-019
@tier      0 · Kernel term layer
@title     `have` in term mode (`letFun`) — Lean's opaque binding versus LOGOS's transparent `Let`
@lean      `have h : T := v; b` in term mode, Lean.Expr `letFun` annotation, mdata key `letFun`
@expect    degrade
@target    Term::Let { name, ty, value, body } — kernel/src/term.rs:475 — together with a loss
           record naming OPACITY. There is no opaque local binding in LOGOS: `Term::Let` is
           zeta-transparent by construction, and the only other candidates are
           `App(Lambda …)` (which is opaque but breaks dependent bodies, T00-017) and a
           top-level `add_declaration` (which is a global axiom, not a local binding).
@evidence  kernel/src/term.rs:443-545 (the `Term` enum has exactly one local-binding form with a
           value, `Let`; there is no opaque variant);
           kernel/src/type_checker.rs:133-142 (`Let` inference zeta-substitutes the value into
           the body BEFORE inferring — `substitute(body, name, value)` — so the body always sees
           `name ≡ value`);
           kernel/src/reduction.rs:298 (`reduce_step` on `Let` performs the same zeta step, so
           the transparency is not merely a checking artefact: it is in the reduction relation);
           kernel/src/context.rs:434 vs :442 (`add_declaration` = opaque, `add_definition` =
           transparent — both GLOBAL, neither local);
           Lean side: `have` elaborates to `letFun v (fun h => b)` under an mdata annotation, so
           the translator's `Expr.mdata` arm (T00-009) is where the two are distinguished
@loss      OPACITY. Lean's `have` deliberately hides the value from the goal: during elaboration
           the hypothesis `h : T` is available but `h`'s DEFINITION is not, so a proof that needs
           `h ≡ v` fails at the source level. Translating to `Term::Let` makes that definition
           visible, so LOGOS proves strictly MORE than Lean did. Acceptable only in the narrow
           sense that the resulting term is still a well-typed inhabitant of the translated
           type — the Lean proof, re-elaborated, is still a proof. It is not acceptable to
           SUPPRESS the record, because the translated development is no longer a conservative
           image of the Lean one: a later Lean-side change that breaks the source still
           typechecks in LOGOS.
@loss      PROOF_TERM_SIZE. `have` exists largely to SHARE a proof: `have big : P := <large>;
           <uses big five times>` is one copy in Lean. `Term::Let`'s zeta rule
           (type_checker.rs:141) substitutes the value at every occurrence before checking, so
           the checked term is five copies. Nested `have`s multiply. This interacts directly
           with `normalize`'s 10000-step fuel (reduction.rs:40) — see T00-012 and T00-022.
@guard     Catches the blanket mdata strip. `have` and `let` differ in the elaborated `Expr` ONLY
           by the `letFun` marker and the `letFun` constant; a translator whose `Expr.mdata` arm
           is `| .mdata _ e => translate e` (T00-009's correct default for pretty-printer keys)
           erases exactly the information that distinguishes them. The mdata arm must INSPECT
           the key.
@guard     Catches translating `have` as `App(Lambda …)` "because it is opaque". That preserves
           opacity faithfully and breaks every `have` whose body's TYPE mentions the bound name
           — and `have` is routinely used that way in dependent code. Worse, the two encodings
           are not interchangeable per-declaration: a translator that picks between them by
           trying one and falling back silently produces a development where some `have`s are
           transparent and some are not, with no record of which.
@guard     Catches the over-permissive direction being mistaken for success. LOGOS accepting a
           `have`-derived term is NOT evidence of faithfulness — it accepts strictly more here
           by construction. The conformance obligation is the reverse inclusion: anything the
           translated development proves must be provable in Lean from the same source. A
           harness that only checks "LOGOS accepted it" cannot see this class of error at all.
@guard     Catches treating `letFun` as an ordinary constant to be looked up. `letFun` is not in
           the LOGOS prelude; emitting `Global("letFun")` fails `get_global` (context.rs:449),
           and defining one via `add_definition` produces an extra delta-redex at every `have`,
           inflating every proof term and blocking the syntactic shape tests in
           `try_structure_eta` (type_checker.rs:1391-1417) and `termination.rs`.
@oracle    Every Lean `have` in term mode emits a `Term::Let` and a loss record with category
           OPACITY; `infer_type` succeeds. Additionally, for each such declaration the harness
           re-elaborates the Lean source with `have` replaced by `let` and asserts the Lean side
           still compiles — a `have` that would NOT survive that replacement is a case where the
           opacity was load-bearing and the degradation must be escalated.
-/

namespace T00_019

/-- The basic shape. In the elaborated `Expr` this is `letFun 3 (fun k => k + 1)` under an
    mdata annotation; the `let` form below is an `Expr.letE`. They are different `Expr`s. -/
def viaHave : Nat :=
  have k : Nat := 3
  k + 1

def viaLet : Nat :=
  let k : Nat := 3
  k + 1

/-- Both are 4, and both reduce to 4 for the Lean KERNEL (`letFun` is reducible). The difference
    is an ELABORATION-time one, which is exactly the difference LOGOS cannot represent. -/
example : viaHave = 4 := rfl
example : viaLet = 4 := rfl

/-- The distinguishing case, stated as the thing that works: with `let`, the body can prove
    `k = 3` by `rfl` because the value is visible. With `have` the same body is refused at the
    source level, because the goal does not see the definition. -/
def letValueVisible : Nat :=
  let k : Nat := 3
  have _h : k = 3 := rfl
  k

/-- `have` used for its real purpose — naming a PROOF so it is written once. Under `Term::Let`'s
    zeta rule this proof is substituted at each of its three uses before checking. -/
theorem sharedProof (n : Nat) : n + 0 = n ∧ n + 0 = n ∧ n + 0 = n :=
  have h : n + 0 = n := Nat.add_zero n
  ⟨h, h, h⟩

/-- A `have` whose bound name is used in a DEPENDENT position. This is the shape that the
    `App(Lambda …)` encoding cannot type, and it is ordinary Lean. -/
def haveInDependentPosition : Nat :=
  have n : Nat := 3
  have v : Fin (n + 1) := ⟨0, by omega⟩
  v.val

/-- Nested `have`s, so the zeta duplication compounds: the inner value is substituted into a
    body that is itself substituted into the outer one. -/
theorem nestedHaves (n : Nat) : n + 0 + 0 = n :=
  have h1 : n + 0 = n := Nat.add_zero n
  have h2 : n + 0 + 0 = n + 0 := Nat.add_zero (n + 0)
  h2.trans h1

/-- `have` in a TYPE position. The annotation carries into the type, where the zeta
    transparency changes what the declared type reduces to. -/
def haveInType : (have n : Nat := 3; Fin (n + 1)) := ⟨2, by omega⟩

end T00_019
