# formally.md — Formal Verification of the Entire LOGOS System

LogicAffeine runs every program through a fleet of engines: a tree-walking interpreter,
a resumable bytecode VM, a copy-patch JIT (EXODIA/Forge), an ahead-of-time compile-to-Rust
backend, a self-applicable partial evaluator, and the three Futamura projections built on top
of it. Today the claim "all of these compute the same thing" is held together by **differential
testing** — the tree-walker is the de-facto specification, and roughly a hundred tests assert the
other tiers match it on a finite corpus of inputs. That is sound *testing*. It is not a *proof*.

This document specifies the path from where we are (empirical agreement on finite inputs, with the
spec being a 238 KB Rust file nobody can reason about) to where we want to be: **a property proved
once about a program is machine-checked to hold on every tier that can ever run it.** Not "we tested
it and it matched." Proved.

It is a companion to `PROPER_FUTAMURA.md` (the partial evaluator and self-interpreter that this plan
leans on as its keystone), `FIX_SPECIALIZER_PLAN.md`, and `FINISH_INTERPRETER.md` (the concurrency
semantics). Where those documents build the *transformations*, this one builds the *proof that the
transformations preserve meaning* — and the surface to state what "meaning" we care about.

---

## 0. Thesis

There are two questions a user can ask, and we answer both with one mechanism.

- **Axis A — properties about a program.** "This sort returns a sorted permutation." "This index is
  never out of bounds." "This loop terminates." "This function honors its contract." Proving things
  *about* the source program.
- **Axis B — the pipeline preserves meaning.** "The VM computes what the tree-walker computes."
  "The optimizer did not change the answer." "The JIT's native fast-path agrees with the bytecode
  it replaced." "The partial-evaluator residual is equal to running the interpreter on the source."

Differential testing today touches only Axis B, and only empirically. The two axes are not
independent — a property on the source is worthless if a downstream tier silently computes something
else. So we unify them through **one reference semantics**:

> **The payoff theorem.** Let `⟦·⟧` be the reference semantics. If property `P` is proved against
> `⟦source⟧`, and every tier `T` is proved to refine the reference (`⟦T(source)⟧ ≈ ⟦source⟧`), then
> `P` holds of `T(source)` for *every* tier `T` — tree-walker, VM, JIT, AOT, and every
> partial-evaluator residual — with no per-tier reproof.

The strategy is **verified compiler, once-and-for-all**: the optimizer passes and each tier carry a
machine-checked semantics-preservation *theorem*, not a per-compile re-check. The north-star
obligations are

```
   ⊢  ∀ p.  optimize(p)   ≈  p          (the 40-pass optimizer preserves meaning)
   ⊢  ∀ p.  tier(p)       ≈  tw(p)       for tier ∈ {VM, JIT, AOT}
   ⊢  ∀ s a b.  ⟦pe(s, a)⟧(b)  =  ⟦s⟧(a, b)   (the partial evaluator preserves meaning — the mix equation)
```

The whole thing is anchored on the CIC kernel `logicaffeine_kernel` as the trusted core, under the
**de Bruijn criterion** that is already enforced across the proof stack: *the search is untrusted; the
certificate is kernel-checked.* We are not building this discipline from scratch. We are extending it
from theorems to whole-program behavior.

---

## 1. What we already have

The reason this is tractable and not a multi-year greenfield is that most of the hard machinery exists
— it is simply pointed at *theorems* today, not at *programs*. Honest inventory:

### The trusted core (TCB)

| Asset | Location | What it is |
|---|---|---|
| CIC kernel | `crates/logicaffeine_kernel/src/{lib.rs,term.rs,type_checker.rs,reduction.rs}` | Calculus of Inductive Constructions. `infer_type` + `is_subtype` are the final arbiters — a term that type-checks against a proposition-as-type **is** a proof of it. |
| Decision procedures | `kernel/src/{ring.rs,lia.rs,cc.rs,omega.rs,simp.rs}` | Proof-producing ring identities, linear integer arithmetic (Fourier–Motzkin), congruence closure, Omega, simplification. |
| Termination & positivity | `kernel/src/{termination.rs,positivity.rs}` | Guardedness/termination checking and strict-positivity of inductive definitions — exactly the side-conditions an operational semantics and a termination property need. |
| Certificate machinery | `kernel/src/certificate.rs` | Already-existing checkable-certificate surface. |
| Milner invariant | `kernel/src/lib.rs` | The kernel has **no path to the lexicon**. Adding English words never recompiles the type-checker. The trusted core is purely logical and language-agnostic. |

### Certified proving (the de Bruijn discipline, already in force)

| Asset | Location | Trust |
|---|---|---|
| Single trust door | `crates/logicaffeine_proof/src/verify.rs::prove_certify_check` | prove → certify → kernel type-check. Returns `verified: true` **iff** the kernel signed off. |
| Curry–Howard bridge | `proof/src/certifier.rs` | `DerivationTree → kernel Term`. Untrusted; its output is re-checked by `infer_type`. |
| SAT core | `proof/src/cdcl.rs` | Competition-grade CDCL(T): watched literals, 1UIP, VSIDS, Luby, DRAT/LRAT logging. **Untrusted** search. |
| RUP checker | `proof/src/rup.rs` | Independent, linear, **fail-closed** replay of the UNSAT refutation. A solver that lies about UNSAT is caught here. |
| Propositional discharge | `proof/src/sat.rs::{prove_equivalence, find_model}` | `F ≡ S` via "¬(F↔S) is UNSAT", RUP-certified; `find_model` returns witnesses. **This is the engine the optimizer-equivalence proofs reuse.** |
| Proof-producing arithmetic | `proof/src/arith.rs::prove_int_eq` | Integer/ring equalities with kernel-checkable proof terms. |
| Bounded model checking | `proof/src/bmc.rs` | k-induction / trace unrolling — the bridge for temporal and loop-bounded properties. |
| Three trust tiers, cross-checked | `crates/logicaffeine_compile/tests/trust_tiers_simon.rs` | CDCL (untrusted) → RUP (certified) → kernel (certified) must agree cell-for-cell. This is the template for every new prover output. |
| Z3 oracle (uncertified) | `crates/logicaffeine_verify/src/{solver.rs,ir.rs}` | Fast SMT for development. Never trusted in a proof — an oracle we must beat, not a backstop. |
| Hardware certified equivalence | `compile/src/codegen_sva/sva_to_proof.rs` → `prove_equivalence` | SVA ↔ FOL equivalence already reduced to the certified Boolean tier. |

### The tier equivalence we want to upgrade from "tested" to "proved"

| Asset | Location | Role today |
|---|---|---|
| Reference (de facto) | `compile/src/interpreter.rs` | The tree-walker. The *spec* — but it is Rust, not a formal object. |
| Tier-invariance gate | `tests/tier_invariance.rs` | VM at run-path tiers T0–T3 vs the tree-walker oracle on a 33-program corpus. |
| End-to-end differential | `tests/e2e_differential.rs` | AOT (codegen→rustc→run) vs interpreter. |
| Concurrency parity | `tests/concurrency_differential.rs` | Tree-walker vs VM under one shared seeded scheduler; includes `diff_cooperative_eq_workstealing` (M:N work-stealing byte-identical to cooperative). |
| Optimization toggles | `tests/opt_toggle_aot_differential.rs` | AOT output invariant under toggling each optimization. |
| Forge micro-op specs | `tests/phase_exodia_forge.rs` | Z3-backed bitvector laws; three independent evaluators must agree on each witness. |

### The keystone foundation — already landed (see `PROPER_FUTAMURA.md`)

| Asset | Location | Role |
|---|---|---|
| Partial evaluator | `compile/src/optimize/partial_eval.rs` | Polyvariant specializer with homeomorphic-embedding termination, effect-gated, mixed-arg specialization. |
| Binding-time analysis | `compile/src/optimize/bta.rs` | Static/dynamic classification driving the specializer. |
| Self-interpreter in LOGOS | `compile/src/optimize/{pe_source.logos,pe_mini_source.logos,pe_bti_source.logos,decompile_source.logos}` | LOGOS-in-LOGOS. The meta-circular definition that makes the Futamura collapse possible. |
| Projection driver | `compile/src/optimize/mod.rs::optimize_for_projection` | The P1 residual pipeline. |
| Jones-optimality gate | `tests/phase_pe_jones.rs` | The residual contains **zero** surviving interpreter dispatch — the interpreter has fully dissolved. |

The partial evaluator and the three projections are *built*. That matters enormously: it means L3 below
does not have to prove five tiers separately. It has to prove **one** thing — that specialization
preserves meaning — and the projections carry the rest.

---

## 2. The gap

What stands between the inventory above and "formally verified entire programs":

1. **Differential testing is finite inputs, not a theorem.** `tier_invariance.rs` proves the tiers
   agree on 33 programs at fixed sizes. It says nothing about the 34th program, or input `n+1`.
   It is excellent regression scaffolding and a terrible substitute for `∀p`.

2. **The reference semantics is Rust, not a formal object.** `interpreter.rs` is 238 KB of hand-written
   imperative code. You cannot prove `VM(p) ≈ tw(p)` because `tw` is not a mathematical object you can
   state a theorem against. The first job is to *make the spec a thing the kernel can see*.

3. **The optimizer and tiers carry no semantics-preservation theorem.** The Oracle
   (`optimize/abstract_interp.rs`) produces conservative facts, and some of them are already
   kernel-LIA-certified — but the *passes that consume those facts* (unchecked indexing, narrowing,
   dense-map, scalarization) are unverified. A wrong pass is caught only if the differential corpus
   happens to exercise the divergence.

4. **Properties about programs are not first-class.** There is no surface to write a precondition, a
   loop invariant, a refinement type, or a `terminates` obligation on a LOGOS program and have it
   discharged. Axis A does not yet exist for imperative LOGOS (it exists only for `## Theorem:` blocks).

---

## 3. The architecture — five layers (L0–L4)

Each layer states its obligation, names its trusted/checked split, and reuses existing machinery.

### L0 — Reference semantics (the spec becomes a formal object)

**Obligation.** Define LOGOS's operational semantics as a formal object *inside the kernel*, and pin
the observational-equivalence relation `≈` that every other layer is stated against.

- **Deep-embed the AST.** Reflect `Stmt`/`Expr` (`crates/logicaffeine_language/src/ast/`) into kernel
  inductive types. `positivity.rs` gives the strict-positivity guarantee these inductive definitions
  need to be admissible.
- **Define the step relation.** A small-step relation `⟶ : Config → Config → Prop` with
  `Config = (stmts, env, store, output)`, as a kernel inductive definition. `output` is the observable
  (the `Show` line stream that every differential test already compares on).
- **Define `≈`.** Observational equivalence: two configurations are equivalent iff they emit the same
  `output` stream for equal input, modulo divergence. This is the *single definition* the payoff
  theorem quantifies over.
- **Make the runnable reference correct-by-construction.** Use the kernel→Rust extraction (the Studio
  "compile-to-Rust"/Forge path) to **extract** the reference interpreter from the `⟶` definition.
  The hand-written `interpreter.rs` then stops being the spec: it becomes a *checked implementation*,
  differential-tested against the extracted reference over the full corpus. The spec moves from
  "238 KB of trusted Rust" to "an inductive relation the kernel understands, plus an extracted
  interpreter trusted only through the extractor." (Reality check from the §5 audit: the extractor is
  TCB-grade for `Term → Rust` today — `compile/src/extraction/{mod.rs,codegen.rs,fol_model.rs}` — but
  it covers math/logic mode, not the full imperative `RuntimeValue` semantics; extending it to the
  imperative fragment is **BC3**, and until it lands the differential gate against `interpreter.rs`
  carries the runnable side.)

**Trusted:** the kernel type-checker, the extractor. **Checked:** everything downstream is stated
against `⟶`.

**This is the prerequisite.** Nothing else can be a theorem until the spec is a formal object.

### L1 — Front-end faithfulness

**Obligation.** The AST *is* the program, so the parser carries no semantic obligation — only
**determinism** and **round-trip** (`pretty ∘ parse = id` on canonical forms). This is a
property-based fixpoint, not a semantics proof.

The semantically rich front-end work is the *English → FOL* transpile, and it already has the
kernel-certified path: `ui_bridge.rs::{prove_theorem_trace, verify_theorem}` →
`proof/verify.rs::prove_certify_check`. L1 folds that in rather than reinventing it: a proved theorem
about the English already terminates at a kernel-checked term.

### L2 — Optimizer correctness: `⊢ ∀p. optimize(p) ≈ p`

**Obligation.** One companion theorem per pass in the optimizer registry
(`crates/logicaffeine_language/src/optimization.rs::REGISTRY`), proved as a simulation/refinement
against L0's `⟶`. The registry's existing metadata (`requires`, `conflicts`, `paths`, `emits_unsafe`,
`mem_class`) is the scaffold: each entry gains a `preserves_semantics` obligation, and the dependency
edges tell us the legal proof order.

- **The classic cluster first** — fold, propagate, DCE, GVN/CSE, inline, TCO. These have textbook
  simulation proofs; `optimize/src/{fold.rs,propagate.rs,dce.rs,gvn.rs,inline_*.rs}` plus the TCO path.
- **The aggressive cluster** — unchecked indexing, narrowing (`Narrow`/`NarrowVm`/`NarrowMap`),
  `DenseMap`, `Affine`, `Scalarize`, `Simd`, `Supercompile` — are sound only under a side-condition
  (an index is in bounds, a value fits i32, keys are bounded). Those side-conditions are exactly what
  the Oracle computes. L2 consumes them as **kernel-checked certificates** produced by L3-of-analysis
  (Phase 3 below), so the pass proof is "preserves meaning *given* this kernel-checked fact." Two
  caveats the primitives audit forces (§5): those facts are *modular/division/bitvector* bounds the
  kernel cannot yet discharge (`arith.rs` treats `/` and `%` as opaque atoms) — they need **BC4**; and
  per **D4** we certify each *consumed* fact as a point-of-use subgoal (the `affine.rs` pattern,
  generalized), leaving the interval/type/alias domain itself untrusted — no Galois-connection proof
  required.

The existing per-pass differential tests (`vm_opt_differential.rs`, `opt_toggle_aot_differential.rs`)
are not deleted — they are **downgraded to fuzzers** that hammer the proved theorem with random inputs
and toggles. Tests are the IP; they stay, repurposed.

### L3 — Tier refinement: `⊢ ∀p. tier(p) ≈ tw(p)`

Two routes that compose.

**Route 1 — the Futamura collapse (the keystone).** Prove the partial evaluator semantics-preserving,
**once**:

```
   ⊢  ∀ s a b.  ⟦pe(s, a)⟧(b)  =  ⟦s⟧(a, b)
```

Then *any* tier expressible as a projection of the reference interpreter is correct **by
construction** — Projection 1 (`pe(int, program)`) is, by this theorem, semantically equal to running
the interpreter on the program. The self-interpreter and `pe` already exist (`PROPER_FUTAMURA.md`),
and Jones optimality is already gated (`phase_pe_jones.rs`). This is the "lift and shift" move: one
specializer-correctness theorem subsumes a per-tier proof for everything on the projected path.

**Route 2 — the refinement ladder** for the hand-written tiers that are *not* produced by `pe`:

- **Bytecode VM** (`compile/src/vm/{compiler.rs,machine.rs,instruction.rs}`): a forward simulation,
  per opcode — each VM step corresponds to reference steps with equal observable output. The existing
  `baseline_vm_differential.rs` / `tier_invariance.rs` become the proof's fuzz backbone.
- **JIT / Forge** (`crates/logicaffeine_forge/src/{jit.rs,regalloc.rs,x64asm.rs,patch.rs}`,
  `compile/src/vm/native_tier.rs`): the native fast-path must equal the bytecode it replaced **under
  its guards**, and **deopt must be faithful** — the guard implies fast-path = bytecode-path, and a
  region deopt restores state exactly (the region-deopt array rollback and precise-deopt machinery
  already exist for this reason). The integer micro-ops graduate from the Z3-backed specs in
  `phase_exodia_forge.rs` to **kernel bitvector lemmas** in `kernel/src/bitvector.rs` (which today holds only gate ops and
  reflection identities — the wrapping-arithmetic / two's-complement / shift laws are **BC4**).
- **Concurrency** (`FINISH_INTERPRETER.md`): the byte-identical property
  `diff_cooperative_eq_workstealing` becomes a theorem — M:N work-stealing **refines** the shared
  seeded cooperative schedule. The determinism is already engineered (one seeded scheduler); the proof
  is that scheduling choice does not affect the observable for determinate programs.

**Trusted:** nothing new. Every route terminates at a kernel-checked term or a kernel bitvector lemma.

### L4 — Program property verification (Axis A) + the unification meta-theorem

**Obligation.** A surface to *state* properties on LOGOS programs — preconditions/postconditions,
loop invariants, refinement types, termination, no-panic (no out-of-bounds, no division-by-zero, the
very facts the Oracle already reasons about) — discharged against L0's `⟶`.

- **Discharge** via the existing three-tier prover: `prove_certify_check` → CDCL → RUP → kernel, with
  `bmc.rs` for bounded/temporal obligations and Z3 (`logicaffeine_verify`) as a dev-time oracle only.
  `termination.rs` supplies the termination obligations directly. Caveat from the §5 audit: the
  three-tier prover proves *FOL theorems*, not Hoare triples over imperative state — `ProofExpr` has no
  `Seq`/`While`/`Assign`. Axis-A therefore needs a weakest-precondition / VCGen program logic built over
  L0 (**BC6**); it is a build, not a reuse.
- **The meta-theorem (the entire point of "both axes, one story"):**

  > If `P` is proved against `⟦source⟧` (L4), and `optimize` (L2) and every `tier` (L3) are proved to
  > refine the reference, then `P` holds of `tier(optimize(source))` for every tier — **no reproof per
  > engine.**

This is what makes the breadth — interpreter, VM, treewalker, compiled, Futamura, partial evaluator —
a feature instead of a liability: you prove the property once, against one semantics, and the L2/L3
theorems carry it to all six.

---

## 4. The Trusted Computing Base

State it honestly and minimally. **Trusted:**

1. The CIC kernel type-checker (`kernel/src/type_checker.rs`).
2. The kernel→Rust extractor (used to produce the L0 reference interpreter).
3. `rustc` and the host hardware.

**Everything else is checked, not trusted** — every prover (`engine.rs`, `cdcl.rs`, `grid_solver.rs`),
the certifier, the optimizer, every tier, the parser, the Oracle. This is the de Bruijn criterion,
already the law of this codebase (`trust_tiers_simon.rs` is its monument). The policy for new work:

> Any new prover or pass must either be proved semantics-preserving **once-and-for-all** (the
> committed strategy) *or* emit a kernel/RUP-checkable certificate. Nothing is trusted on the search
> side.

**Interim soundness, not a corner cut:** while the once-and-for-all proofs are being built, a
not-yet-proved pass may run behind a *per-compile* checked equivalence certificate, or be disabled. The
system stays sound at every commit; the end state is still the full verified compiler. The certificate
is the scaffolding, not the building. (Caveat from the §5 audit: `sat.rs::prove_equivalence` is
*Boolean-only* — it cannot yet compare two programs with state — so a real per-compile equivalence
certificate itself depends on **BC3** + **BC5**; until those land, the honest interim gate is "disable,
or differential-fuzz," not a free certificate.)

---

## 5. Base Camp — the primitives to forge before the climb

A source-level audit of `logicaffeine_kernel` and `logicaffeine_proof` answers the only question that
matters before committing: **do we have the primitives to even state these theorems?** Honest verdict:
**no — the climb needs six primitives we lack, but the three scariest can be designed around rather than
built.** The mountain is reachable; we stock Base Camp first. Every verdict below carries file evidence.

### What the kernel and proof layers already give us (HAVE)

- **Indexed inductive families work.** `BVec` is length-indexed (`kernel/prelude.rs:2405`), so a
  `Prop`-valued `Step : Config → Config → Prop` whose constructors *are* the inference rules is
  admissible. The single most important "can we even express an operational semantics" question is yes.
- **Strict positivity** enforced at constructor registration (`kernel/positivity.rs:32`, `context.rs:228`).
- **Leibniz equality + rewriting:** `Eq`, `refl`, `Eq_rec`, `Eq_sym`, `Eq_trans` (`prelude.rs:499–738`).
- **Universe hierarchy** with cumulativity and impredicative `Prop` (`term.rs:95–143`) — enough to host
  the semantics in `Prop`.
- **De Bruijn certificates** that recheck against a freshly rebuilt prelude (`certificate.rs:34–89`);
  **structural recursion** on inductive arguments (`termination.rs:40–97`); a rich prelude (Nat, Bool,
  List, And/Or/Ex, Derivation, reflection).
- **A TCB-grade kernel→Rust extractor** — deterministic, structurally honest (`compile/src/extraction/`).
- **BMC / k-induction, RUP-certified** (`proof/bmc.rs`); the prove→certify→type-check trust door
  (`proof/verify.rs:230`).

### The gap (PARTIAL / MISSING) — and which pieces are load-bearing

| Primitive | Verdict | Evidence | Load-bearing for |
|---|---|---|---|
| Finite-map / store / heap theory | **MISSING** | no map/assoc/heap in `prelude.rs` | L0 (`Config = env+store`) — *the* blocker |
| Mutual induction | **MISSING** (encodable) | single `Inductive` only (`interface/command.rs:33`) | L0 (Stmt ↔ Expr are mutually recursive) |
| Coinduction / streams / infinite traces | **MISSING** | no `cofix`/`stream` | divergence & obs-equivalence "modulo ⊥" |
| Well-founded recursion / fuel / measures | **PARTIAL** | structural guard only (`termination.rs:64`) | defining `While`; Axis-A termination |
| Division / modulo / nonlinear arithmetic | **PARTIAL→MISSING** | `/`,`%` opaque in `arith.rs:172` | L2 aggressive passes (FastDiv, `%k` bounds, DenseMap) |
| Bitvector arithmetic lemmas | **PARTIAL** | gates + reflection only (`bitvector.rs`) | L3 JIT/Forge; modular optimizer facts |
| Program equivalence **with state** | **MISSING** | `prove_equivalence` Boolean-only (`sat.rs:87`, `cnf.rs:249`) | L2/L3 (the equivalence we actually need) |
| Simulation / bisimulation / refinement combinator | **MISSING** | grep: zero in proof crate | every L2 pass + every L3 tier |
| Hoare logic / WP / VCGen | **MISSING** | no `Seq/While/Assign` in `ProofExpr` | L4 (Axis-A) |
| Abstract-domain soundness | **PARTIAL** | only affine subgoals reach kernel (`affine.rs:92`); interval/type/alias trusted | L2 Oracle-gated passes |
| Σ / subset / refinement types | **MISSING** | none in `prelude.rs` | Axis-A refinement types (deferrable) |

The state we must model is large: `RuntimeValue` has 25 variants incl. the BigInt/Rational numeric
tower, mutable `Rc<RefCell<…>>` collections (aliasing!), and first-class concurrency (`Chan`/`Task` +
the seeded scheduler in `logicaffeine_runtime`); the AST is ~100 variants (Stmt 54, Expr 28) over arena
lifetimes. A naive L0 that models Rc-aliasing directly would drag in separation logic.

### The design moves that shrink the mountain (forge less — not cut corners)

LIFT-AND-SHIFT: don't build the scary primitives — pick semantics that don't need them.

- **D1 — Fuel-indexed big-step semantics instead of coinduction.** Index evaluation by `fuel : Nat`
  (HAVE) with structural recursion on fuel (HAVE). Divergence = "no fuel suffices"; observational
  equivalence = agreement on every fuel prefix. This converts a *research-grade missing primitive*
  (coinduction) into a `Nat` parameter — and simultaneously gives `While` a structural decreasing
  argument, retiring the well-founded-recursion gap too.
- **D2 — Define L0 over a functional store, downstream of ownership analysis.** The de-Rc / `Unbox`
  pass already establishes unique ownership; semantics over the *post-ownership* functional store avoids
  modeling `Rc<RefCell>` aliasing. Aliasing becomes a single front-end obligation ("de-Rc is sound")
  rather than a pervasive heap theory. The only genuinely-new data structure left is a finite map (BC1).
- **D3 — Encode mutual induction as a tagged single inductive** (admissible today) to start; promote to
  a real kernel mutual-block only if the recursor ergonomics hurt.
- **D4 — Certify each *consumed* fact as a kernel subgoal (the `affine.rs` pattern, generalized), not
  the whole abstract domain.** No Galois-connection proof for intervals/types/aliases; each pass emits
  the exact proposition it relies on and the kernel discharges it (LIA today; div/mod + bitvector after
  BC4). The domain stays untrusted; its outputs are checked at the point of use.
- **D5 — Build the simulation/refinement combinator once** (`Simulation R A B ⇒ obs_eq A B`) as the
  shared meta-primitive every L2/L3 obligation instantiates — so program-equivalence-with-state is
  *derived*, not re-proved per pass.
- **D6 — Program logic (WP/VCGen) over L0 for Axis-A**; defer Σ/refinement types (contracts subsume
  most of the need).

### Base Camp build list (Phase −1), in dependency order

1. **BC1 — Finite-map/store theory**: a kernel map type with `lookup`/`update` and the lemmas
   (`lookup-after-update`, frame/disjointness). *The* prerequisite; all of L0 is stated over it.
   *Difficulty: medium.*
2. **BC2 — AST inductive embedding** (Stmt/Expr/Literal), mutual recursion via D3. *Difficulty: medium,
   largely mechanical (~3k LOC).*
3. **BC3 — Fuel-indexed eval relation** `Eval : Fuel → Config → Result → Prop` over BC1+BC2 (D1). Plus
   the runnable side: extend the extractor (HAVE for `Term → Rust`) toward the imperative fragment, kept
   honest by a differential gate against `interpreter.rs`. *Difficulty: research-grade — the first wall,
   but D1/D2 make it buildable on HAVE primitives.*
4. **BC4 — Division/modulo + bitvector arithmetic lemmas** in the kernel (wrap add/mul, two's-complement,
   shifts, magic-number division), lifting the Forge Z3 specs (`phase_exodia_forge.rs`) to
   `kernel/bitvector.rs`. *Difficulty: medium-high; bounded, lemma-by-lemma.*
5. **BC5 — Simulation/refinement combinator** (D5), the meta-primitive for L2/L3. *Difficulty: medium.*
6. **BC6 — WP/VCGen program logic** over BC3 for Axis-A (D6). *Difficulty: medium-high; gated on BC3.*

**Deferred / designed-around (not built):** coinduction (D1), separation-logic heap (D2),
Σ/refinement types (D6).

> **Bottom line.** We do need more primitives — six to forge — but they are enumerable,
> dependency-ordered, and free of open *research* risk once D1 (fuel) and D2 (functional store) retire
> coinduction and separation logic. BC1 (finite map) is the literal first stone; BC3 (fuel-indexed
> semantics) is the first wall and gates the whole climb. Base Camp is Phase −1 in §6.

---

## 6. Phased TDD plan

Each phase: goal, the RED test that defines it, the crates/files it touches, honest difficulty. Order
is dictated by Base Camp (§5) gating L0, and L0 being the universal prerequisite for everything above it.

**Phase −1 — Base Camp.** Forge BC1–BC6 (§5) in dependency order; BC1 (finite-map/store) and BC3
(fuel-indexed semantics) gate Phase 1. RED: `finite_map_lookup_after_update`, then
`eval_relation_deterministic_on_corpus`. *Difficulty: BC1/BC4/BC5 medium; BC3/BC6 the hard
prerequisites.*

**Phase 0 — Foundations & TCB.** Pin `≈` (observational equivalence) and the trust model in prose +
an executable definition. RED: a test that fixes the `obs_eq` definition and checks the tree-walker
output stream against it on the corpus. *Difficulty: low.*

**Phase 1 — L0 reference semantics + extraction.** Deep-embed `Stmt`/`Expr` into kernel inductives
(`positivity.rs` admissibility), define `⟶`, extract the reference interpreter, differential-gate
extracted-ref vs `interpreter.rs` over the full corpus. RED: `extracted_reference_eq_treewalker`.
*Difficulty: research-grade. This is the first wall and unblocks everything.*

**Phase 2 — L2 classic passes.** `pass_<T>_preserves_semantics` kernel theorems for
fold/propagate/DCE/inline/TCO/GVN. RED per pass. *Difficulty: medium; textbook simulation proofs.*

**Phase 3 — Oracle facts as certificates.** `abstract_interp.rs` emits kernel-LIA-checked bound
certificates (the LIA bridge to `kernel/lia.rs` already exists). RED: a deliberately-wrong bound must
**fail** the kernel check (fail-closed). *Difficulty: medium.*

**Phase 4 — L2 aggressive passes.** Unchecked / Narrow* / DenseMap / Affine / Scalarize / Simd /
Supercompile, each proved *given* a Phase-3 certificate. *Difficulty: medium-high.*

**Phase 5 — L3 VM refinement.** Per-opcode forward simulation `tw ⊑ VM`;
`baseline_vm_differential.rs` / `tier_invariance.rs` become the fuzz backbone. *Difficulty: high.*

**Phase 6 — L3 JIT + deopt soundness.** Guard ⇒ fast-path = bytecode-path; faithful region rollback;
Forge bitvector lemmas lifted into `kernel/bitvector.rs`. *Difficulty: high.*

**Phase 7 — L3 Futamura collapse.** Prove `pe` correct (the mix equation); tie the projected
(AOT/P1) path to it so it is correct by construction. *Difficulty: research-grade. The second wall and
the biggest single payoff.*

**Phase 8 — L4 property layer + unification meta-theorem.** Surface syntax for contracts/invariants/
refinement/termination; discharge via the three-tier prover; prove the property-carries-to-all-tiers
meta-theorem. *Difficulty: medium once L0–L3 land.*

**Phase 9 — Concurrency refinement.** Work-stealing ⊑ cooperative as a theorem over the seeded
scheduler. *Difficulty: high.*

---

## 7. Honest difficulty & sequencing

- **Base Camp first (§5):** BC1 (finite-map) is the literal first stone; BC3 (fuel-indexed semantics)
  is the first wall and gates the whole climb. BC1/BC4/BC5 are medium; BC3/BC6 are the hard ones.
- **Tractable once Base Camp lands (weeks each):** Phases 0, 2, 3, and the surface of 8.
- **Hard but bounded (months each):** Phases 4, 5, 6, 9.
- **Research-grade prerequisites (the two walls):** Phase 1 (L0 reference semantics, gated on BC1+BC3)
  and Phase 7 (the Futamura collapse). Everything hangs off L0; sequence Base Camp → L0 first. It is the
  conversion of "the tree-walker is the spec" into a formal object, and what lets every later phase even
  be *stated* as a theorem.

This is CompCert/CakeML-class work, and we do not pretend otherwise. What makes it *our* version and not
a from-scratch CompCert is that the trusted kernel, the certified-proving discipline, the kernel→Rust
extractor, the affine-bounds bridge already reaching the kernel, and — critically — the partial evaluator
and its three projections are **already built**. We are not inventing the machine; we are forging six
bounded primitives (§5), turning the spec into a formal object, and discharging the obligations on the
proving stack that already beats Z3 on certified grids. The honest delta versus "we already have it":
program-equivalence-with-state, a simulation combinator, and a program logic do **not** exist yet — they
are Base Camp, not reuse.

---

## 8. How we verify the verification

- **Tier agreement** across CDCL (untrusted) → RUP (certified) → kernel (certified), the pattern in
  `trust_tiers_simon.rs`, applied to every new prover output.
- **RUP self-check, fail-closed**, with `kernel::infer_type` as the final gate. A search bug that
  reports a false proof is caught by the checker, never trusted.
- **Proof fuzzing:** toggle every `REGISTRY` optimization (`opt_toggle_aot_differential.rs`) and random
  inputs against the proved theorem; a counterexample is a proof bug, surfaced immediately.
- **No test is ever deleted.** The existing differential suites (`tier_invariance.rs`,
  `e2e_differential.rs`, `concurrency_differential.rs`, `phase_exodia_forge.rs`, …) become permanent
  regression fuzzers behind the proofs. Tests are the IP; they are ported to the new mechanism, never
  stripped.

---

## 9. The one-paragraph summary

Forge six bounded primitives at Base Camp (§5) — a finite-map store, an AST embedding, a fuel-indexed
eval relation, div/mod + bitvector lemmas, a simulation combinator, and a WP program logic — using
fuel-indexing and a post-ownership functional store to sidestep coinduction and separation logic.
Make the tree-walker's semantics a formal object in the kernel (L0). Prove every optimizer pass and
every hand-written tier refines it, and prove the partial evaluator preserves meaning once so the
whole projected/compiled path is correct by construction (L2, L3). Let users state properties on
programs and discharge them against that one semantics (L4). Then the breadth we have — interpreter,
VM, treewalker, compiled language, Futamura projections, partial evaluator — stops being a verification
liability and becomes the proof's reach: a property proved once is machine-checked to hold on all of
them. The trusted base stays tiny (the kernel and the extractor); everything else is checked, not
trusted; and we never delete a test along the way.
