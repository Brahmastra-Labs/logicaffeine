# MATH_GAPS.md — what Logos is missing to host the graph of mathematics

## 1. Frame: math is a graph, Logos is where the nodes get proved

`mathscrapes` is building a dependency graph of all mathematics: scrape textbooks/PDFs (Euclid, the calculi,
trig, stats, analysis), extract theorems / definitions / axioms, link them by *depends-on / unlocks* edges.
Today it maps edges *within* texts — the babies of the graph. The end-state: all of math as one connected
graph, with **Logos as the playground where each node is actually expressed and proved** — a node is not prose,
it is a checkable definition or theorem in the CoIC, and an edge is a dependency the prover enforces.

This document inventories the **missing primitives** that stand between today's Logos and that role. Not
missing proofs — missing primitives of the language / kernel / prover. The lens, applied to every gap:

> *Does this primitive block hosting or laddering a region of the graph?* A primitive earns a rung if its
> absence stops whole families of nodes from being minted, stated, or linked.

The ladder is ordered so each rung unlocks the next, and it **starts at node-minting machinery** (§3, Rung 0):
the operations that let Logos create a graph node and wire it to its dependencies at all. Nothing ladders
without that.

One correction up front, because it changes the whole shape of the problem: **Logos is already a genuine CoIC**
(Calculus of Inductive Constructions), not "FOL plus solvers." The gaps are therefore *upward* gaps — what
stops a real dependent type theory and its English surface from climbing into analysis and geometry — not
"go build a kernel." §2 establishes that baseline so we never re-litigate it.

---

## 2. Baseline: what Logos already is (a three-layer stack)

Logos is **three layers wired together**, and this document treats it as one system:

| Layer | What it is | Crate |
|-------|-----------|-------|
| **Vernacular logic** | English → FOL, `## Theorem` blocks | `logicaffeine_language` |
| **CoIC kernel** | dependent type theory, Curry–Howard proof checking | `logicaffeine_kernel` |
| **Executable PL** | imperative LOGOS + interpreter/VM/JIT/AOT, numeric tower, concurrency | `logicaffeine_compile`, `logicaffeine_forge`, `logicaffeine_system`, `logicaffeine_base` |

These are not siloed. The kernel carries a **deep embedding** that reflects terms and derivations as data
(`Univ` `prelude.rs:967`, `Syntax` `prelude.rs:998`, `Derivation` `prelude.rs:1335`); ι-reduction lets the
kernel *compute*; the numeric tower is shared substrate. The consequence for math: **a graph node is often not
a proposition but a computation** — an algorithm, a ruler-compass construction, a numeric method. The missing
primitives therefore include the *seams between the layers* (§4), not only the logical rungs (§3).

What exists today, with anchors:

- **Kernel is a real dependent type theory.** `Term` (`term.rs:155`) = `Sort(Universe)` (`:157`), dependent
  `Pi` (`:169`), `Lambda` (`:176`), `App` (`:183`), `Match` (`:192`), `Fix` (`:202`), `Lit` (`:213`),
  `Hole` (`:219`); universes `Prop`/`Type(n)` (`term.rs:103`). β/δ/ι reduction + Fix-unfolding in
  `reduction.rs`; definitional equality by normalize-then-compare in `is_subtype` (`type_checker.rs:843`);
  structural-termination guard in `termination.rs`.
- **Proofs are Curry–Howard terms, re-checked.** `Certificate{proof_term, claimed_type}`
  (`certificate.rs:35`); `recheck` (`certificate.rs:64`) rebuilds a fresh trusted `Context` and re-type-checks.
  A proof of `P` is *any* term of type `P`.
- **Inductives live in the `Context`, not as first-class `Term` nodes.** `add_inductive`/`add_constructor`/
  `add_definition` (`context.rs:92`/`:102`/`:125`). Built-ins in `prelude.rs`: `Int`/`Float`/`Text` (`:153`+),
  `Nat` (`:315`), `Bool` (`:339`), `TList` (`:361`), `True`/`False` (`:465`/`:472`), `Eq` (+ `refl`, `Eq_rec`,
  `Eq_sym`, `Eq_trans`, ~`:505`), `And`/`Or`/`Ex`.
- **Proof engine** (`logicaffeine_proof`): backward chaining `prove` (`engine.rs:1079`); certified saturation
  `cert_saturate` (`engine.rs:631`, 5 rules); DPLL case-split; **genuine unbounded structural induction over
  Nat/List** with Miller-pattern motive inference (`try_nat_induction_with_motive engine.rs:4810`,
  `try_list_induction engine.rs:4949`); CDCL+RUP SAT `prove_unsat` (`sat.rs:108`); BMC/k-induction
  (`bmc.rs`); ring/LIA arithmetic oracle `prove_int_eq` (`arith.rs:69`); equational rewrite + congruence
  (`engine.rs:4328`); finite-domain grid solver (`grid_solver.rs`). Theory modules in the kernel:
  `ring.rs`, `lia.rs`, `cc.rs`, `omega.rs`, `simp.rs`, `bitvector.rs`. Everything certifies down to a kernel
  term via `prove_certify_check` (`verify.rs:50`).
- **Surface language**: `LogicExpr` (`ast/logic.rs:512`) has ∀/∃ plus generalized `Most`/`Few`/`Cardinal`/
  `AtLeast`/`AtMost` (`QuantifierKind ast/logic.rs:132`, `Cardinal:144`, `AtLeast:146`), ∧∨¬→↔, `Lambda`/`App`,
  `Predicate`, `Identity`, `Comparative`; nested alternating quantifiers work. Theorem syntax
  `## Theorem / Given: / Prove: / Proof:` with strategy `Auto | Induction(var) | ByRule` (`ast/theorem.rs:22`,
  `:38`). Numbers: `NumberKind::{Real(f64), Integer(i64), Symbolic}` (`ast/logic.rs:67`). Entry points
  `verify_theorem` (`ui_bridge.rs:2071`), `prove_theorem_trace` (`ui_bridge.rs:2117`).
- **Numeric tower**: `BigInt` (`numeric.rs:28`) + `Rational` (`numeric.rs:448`), both real and exported.
  `Real`/`Complex`/`Decimal` are header-comment futures only (`numeric.rs:6`: "Rational/Decimal/Complex build
  on it") — not types.
- **Z3** is an *untrusted oracle fallback*, feature-gated (`oracle.rs`); its verdicts are `OracleVerification`
  justifications, never trusted as kernel proofs.

The takeaway: the foundation is strong and constructive. The gaps are about **expressing more** (sets,
higher-order, ℝ, binders), **minting and linking more** (definitions, a library, structures), and **moving
content between the three layers** (reflection, extraction, coherence).

---

## 3. The ladder of missing primitives

Each rung: **(a)** what the corpus needs it for · **(b)** what EXISTS today · **(c)** what's MISSING ·
**(d)** where it lives in Logos · **(e)** what it unlocks.

### Rung 0 — Node-minting machinery *(the start — load-bearing for the whole graph)*

Four sub-parts. Without these, Logos cannot create a graph node, link it, or reuse it — every later rung
presupposes them.

**0a. Definitional extension in the logic layer.**
- (a) Every node of the math graph *is* a definition built from prior ones: "a group is a set with…",
  "`continuous(f)` means…", "`prime(n)` iff…". Laddering = defining new vocabulary on old.
- (b) The kernel *can already hold a definition* (`add_definition context.rs:125`, unfolded by δ-reduction).
  The imperative layer has `StructDef`/`FunctionDef` (`ast/stmt.rs:272`/`:279`). The lexicon is **compiled
  fixed** from `assets/lexicon.json` (`build.rs:290`).
- (c) There is **no way to introduce a new predicate / relation / abbreviation in the vernacular logic** —
  no `Define`/`Definition` construct in the logic surface (`token.rs` has no such keyword). The gap is the
  *surface→kernel definitional path*, not kernel capability.
- (d) Lexer/parser (`logicaffeine_language`) + a lowering that calls `Context::add_definition`.
- (e) Mints every node. Precondition for Rungs 2, 5, 6 and the library (0b).

**0b. Named, dependency-tracked lemma library.**
- (a) The graph itself: a proved theorem becomes a citable named lemma that later proofs invoke; the
  depends-on DAG is recorded. This is the literal embodiment of mathscrapes inside the prover.
- (b) `verify_theorem` (`ui_bridge.rs:2071`) proves **one** `## Theorem` string standalone; no persistence,
  no naming across theorems, no citation.
- (c) **No library/environment** of named definitions+theorems with edges; nothing tracks "theorem T used
  lemma L."
- (d) A new environment crate/module wrapping `kernel::Context` with named entries + a dependency index;
  surface citation syntax (ties to Rung 7).
- (e) Turns standalone proofs into a graph; the `uses` edge of §6; provenance for Rung 8.

**0c. Structures / typeclasses with instances + inheritance.**
- (a) "X is a field ⇒ inherit the entire field theory." This is how *unlocks* edges fire in bulk: prove the
  axioms once, get the whole downstream theory.
- (b) `ring`/`lia` exist as **tactics** (`kernel/ring.rs`, `kernel/lia.rs`), and `Pi`+inductives can *encode*
  records, but there is no first-class structure/instance notion.
- (c) **No surface structure / typeclass / instance / inheritance** (group ⊂ ring ⊂ field as a resolvable
  hierarchy).
- (d) Surface structure syntax + a kernel encoding (records as Σ-types / `Pi`-bundles) + instance resolution.
- (e) Algebra (Rung 5 analog), reuse across the graph, the "inherit a theory" unlock.

**0d. Computational (executable) definitions — the three-layer hook.**
- (a) `gcd`/`factorial`/`is_prime` should be **one** `Define` that is simultaneously a δ-unfoldable kernel
  definition *and* a VM/AOT program, proven to agree — so a node is both *checkable* and *runnable*.
- (b) Pieces exist unjoined: imperative `FunctionDef` + recursion/TCO run on the PL layer; kernel `Fix`/`Match`
  (`term.rs:202`/`:192`) express the same shape.
- (c) **Nothing ties one `Define` across both layers** with a coherence obligation.
- (d) Surface `Define` lowering to (kernel def ⊕ PL function) + a generated equality obligation.
- (e) Sets up proof-by-reflection and extraction (§4); makes constructive nodes execute.

### Rung 1 — Higher-order quantification
- (a) Analysis is saturated with it: "for every continuous **f**", "for every open **set** U", induction
  *schema* over predicates.
- (b) Kernel `Pi` already quantifies over any type, including `A → Prop`. The **surface is first-order only**:
  `QuantifierKind` (`ast/logic.rs:132`) binds individuals.
- (c) **No surface syntax / lowering for second-order quantifiers** (over functions, sets, predicates).
- (d) `logicaffeine_language` parser + lowering to kernel `Pi`/`Lambda`.
- (e) Rung 2 (sets), Rung 5 (analysis phrasing).

### Rung 2 — Sets as first-class
- (a) Almost every statement quantifies over or constructs sets: `{x ∈ S | φ(x)}`, ∪/∩/∖/⊆, powerset 𝒫,
  function spaces as sets.
- (b) FOL has membership-as-predicate only; ∪/∩/`Contains` exist in the **imperative** `Expr`, not the logic
  layer.
- (c) **No set type and no comprehension binder in the logic layer.**
- (d) Logic AST (`ast/logic.rs`) + kernel encoding (sets as `A → Prop` or as a `Set` inductive).
- (e) Topology, measure, geometry-of-point-sets; depends on Rung 1.

### Rung 3 — Numbers: complete the tower to ℝ (and ℂ)
- (a) Calculus, trig, stats, coordinate geometry all live over ℝ.
- (b) `BigInt` (`numeric.rs:28`), `Rational` (`numeric.rs:448`).
- (c) **No `Real`** — and the keystone is the **completeness / least-upper-bound axiom**, the single property
  that separates ℝ from ℚ and unlocks all of analysis. Also missing: `Complex`, `Decimal` (`numeric.rs:6`
  stubs).
- (d) **Shared substrate**: the `logicaffeine_base` tower (so programs *compute* with it) co-designed with a
  kernel `Real` sort + ordered-complete-field axioms (so the prover *reasons* about it) + lexicon. The PL
  tower and the kernel reals must be the *same* numbers, not two parallel notions (this is also a §4 seam).
- (e) Rung 4 (binders over ℝ), Rung 5 (analysis), Rung 6 (coordinate geometry).

### Rung 4 — Term-level binders
- (a) Σ, ∏, ∫, lim, sup, inf, ⋃, ⋂, `{·|·}` — the shape of nearly every analysis/algebra/measure statement.
- (b) Quantifier binders produce **Prop** only; there is **no value-producing binder** in the AST.
- (c) **No general variable-binding term-former** that yields a value.
- (d) Logic/term AST + kernel lowering (binders as applied higher-order operators over `Lambda`).
- (e) Rung 5; the elegant framing is that this is *one* structural gap, not N operators. Depends on Rungs 1–3.

### Rung 5 — Analysis content as definitions
- (a) limit, continuity (real ε–δ), derivative, integral, series convergence — the bulk of the calculus/trig/
  stats corpus.
- (b) Nothing yet (nested ∀∃ can be *written* but the notions don't exist).
- (c) **All of it MISSING** — but note each is almost pure composition: a *definition* (Rung 0) over ℝ
  (Rung 3) over functions (Rung 1) using binders (Rung 4). Once the lower rungs land, this is mostly library.
- (d) The lemma library (0b) as content, not new engine.
- (e) The calculi, statistics, applied math.

### Rung 6 — Geometry: an axiomatic theory
- (a) Euclid needs points/lines/circles + betweenness/congruence/incidence as a **declared theory**
  (Hilbert/Tarski) you open and reason within, plus ruler-compass **construction** reasoning.
- (b) Nothing geometric today.
- (c) **No mechanism to declare an axiomatic theory and reason inside it**, and **no construction reasoning.**
  This is Rung 0 (definitional/axiomatic extension) + Rung 5-style content applied to geometry.
- (d) Rung 0c (a theory = a structure of sorts + axioms) + a geometry library + §4 extraction for
  constructions.
- (e) The wishlist keystone. Euclid's `[I.47] → [I.4] → … → postulates` citation chain **is** a mathscrapes
  dependency graph — the place where graph-meets-prover is most literal. PL tie-in: a **construction is an
  algorithm** (ruler-compass = a program producing the witness point/line), so geometry exercises §4's
  extraction seam.

### Rung 7 — Proof-process vernacular
- (a) Real proofs are multi-step: "Suppose. Let. Consider. We have…. By lemma X. Therefore." Non-trivial
  theorems (MVT, etc.) must be *decomposed*; the prover will not one-shot them.
- (b) Push-button `Auto`/`Induction`/`ByRule` (`ast/theorem.rs:38`) + pedagogical hints
  (`suggest_hint hints.rs:64`).
- (c) **No forward, decomposable proof syntax** (`Suppose`/`Let`/`have`/`Therefore`) and **no
  lemma-invocation-by-name** (needs the library, 0b).
- (d) Theorem AST + parser + an interpreter that threads intermediate goals through the existing engine.
- (e) The interface between scraped proof *text* and the kernel — how an edge gets *traversed*, not just
  recorded.

### Rung 8 — Classical axioms, opt-in + provenance
- (a) Classical analysis needs LEM, choice, function extensionality (trichotomy, Bolzano–Weierstrass, …).
- (b) The kernel is intensional/constructive; `Eq` is Leibniz with `refl` only.
- (c) **No managed opt-in to classical axioms** and **no tracking of which theorems depend on which axiom.**
- (d) Declarable axiom sets in `Context` + dependency tagging in the library (0b).
- (e) Classical analysis; provenance is itself a graph edge ("uses AC") — ties back to 0b.

---

## 4. Layer seams (the programming-language tie-in)

The dimension the three-layer stack adds *on top of* the rungs. Graph nodes are heterogeneous —
**propositions** (proved), **definitions** (minted), **constructions/algorithms** (executed) — so some missing
primitives are about *moving content between layers*. Same (a–e) template.

**4a. Proof by reflection / proof-by-computation.**
- (a) Big *computational* facts — bounded number theory, finite case analysis, numeric bounds — that no proof
  search will ever close, but a *verified checker run on the PL layer* settles in one shot.
- (b) Seed form already present: `ring`/`lia`/`cdcl`/`grid_solver` *are* this pattern (run a procedure, certify
  the result); ι-reduction computes in-kernel; the deep embedding (`Syntax prelude.rs:998`,
  `Derivation prelude.rs:1335`) reflects terms/derivations as data.
- (c) **No general reflection primitive**: "Prop P holds because this verified checker, executed on the PL
  layer, returns true (and the checker is proven sound)."
- (d) A reflection rule bridging PL evaluation ↔ kernel `Match`/`Fix` over the `Syntax`/`Derivation` embedding.
- (e) The single biggest lever for the computational regions of the graph.

**4b. Proof extraction / programs-from-proofs.**
- (a) A constructive ∃-proof should yield a runnable witness program on the PL layer — Curry–Howard realized
  end-to-end. Euclid's constructions and algorithmic existence theorems are the motivating corpus.
- (b) Proof terms exist (`certificate.rs:35`); the PL layer can run programs; nothing connects them.
- (c) **No extraction path** from a kernel proof term to a `logicaffeine_compile` program.
- (d) An extractor: kernel `Term` (proof) → LOGOS function → VM/AOT.
- (e) Makes constructive nodes *do something*; powers Rung 6 constructions.

**4c. Executable-definition coherence.**
- (a) Rung 0d viewed as a seam: one `Define` ⇒ (kernel def ⊕ VM program) + a discharged proof they agree.
- (b)/(c)/(d)/(e) as Rung 0d. Stated here as an explicit obligation the system must generate and check, not a
  convention.

**4d. Shared numeric substrate.**
- (a) Rung 3 viewed as a seam: the tower must serve running programs *and* kernel reasoning as **one** notion.
- (b) `BigInt`/`Rational` exist in `logicaffeine_base`; the kernel reasons via `lia`/`ring`/`omega`.
- (c) **No single ℝ (and ℂ/Decimal)** shared across PL execution and kernel proof.
- (d) Co-design `logicaffeine_base` types with kernel sorts/axioms (one source of truth).
- (e) Prevents the classic split where the prover's numbers and the program's numbers drift apart.

---

## 5. The meta-ladder (dependency ordering of the rungs)

```
  PL layer ────────┐   §4 seams: 4a reflection · 4b extraction · 4c exec-coherence · 4d shared ℝ
 (compile/forge/   │        │  (each crosses PL ↔ kernel)
  system/base)     │        ▼
                   └──►  Rung 0  ◄── load-bearing for everything
  kernel ─────────────  (0a defs · 0b library · 0c structures · 0d exec-defs)
                          │        │            │            │           │
                          ▼        ▼            ▼            ▼           ▼
  logic ──────────       R1 ─► R2     R3 ─► R4          R6          R7         R8
                          └──────┬──────┴──────┘       (geometry)  (proof    (classical
                                 ▼                                  vernacular) axioms)
                                R5  =  analysis  (R1 + R3 + R4, defined via R0)
```

Punchline: **Rung 0 is load-bearing for the entire graph** — it is the first thing to build and the literal
mathscrapes bridge — while the §4 seams are what make nodes *runnable*, not merely *checkable*. The
recommended drive order, consistent with "node-minting machinery," is **Rung 0 first** (0a definitional
extension → 0b library → 0c structures → 0d exec-defs), because every other rung consumes it.

---

## 6. The graph-bridge (cross-cutting)

The seam between the two projects, made precise:

- **mathscrapes node → Logos artifact.** A node compiles to a Logos **definition**, **theorem**, *or*
  **executable construction** (heterogeneous, per §4) — not prose.
- **mathscrapes edge → tracked `uses` dependency.** An edge becomes a recorded dependency in the lemma
  library (Rung 0b). "Theorem T uses lemma L" is an edge the prover knows about.
- **Hosted vs. linked.** A scraped theorem is *hosted* once it is a checked kernel term; it is *linked* once
  its citations resolve to library entries.
- **The invariant we want.** *You cannot prove `[I.47]` until `[I.4]` is in the library.* The prover enforces
  the graph — citation is resolution, not decoration — and the PL layer makes the constructive nodes actually
  compute.

The whole document reduces to one sentence: **build Rung 0 and the §4 seams, and Logos stops being a theorem
checker and becomes a substrate the math graph can be poured into.**
