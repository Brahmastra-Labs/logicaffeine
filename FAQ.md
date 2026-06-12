# LOGOS / Logicaffeine — FAQ

**[Try LOGOS Online →](https://logicaffeine.com/guide)**

---

## The Basics

### What is LOGOS?

LOGOS is a natural language compiler: plain English in, formal artifacts out. It has two modes:

- **Logic mode** — English sentences become first-order logic, and provable claims become *kernel-checked proof certificates*.
- **Imperative mode** — English programs compile to executable Rust.

The spec doesn't *describe* the program. The spec **is** the program.

### Isn't English too ambiguous to be a spec language?

English is ambiguous — and LOGOS treats that as a feature to surface, not a problem to paper over. When a sentence has one reading, you get that reading as explicit formal logic (Unicode FOL or LaTeX). When it genuinely has more than one, **LOGOS enumerates every defensible reading as a parse forest** rather than silently picking for you: lexical ambiguity (noun vs. verb), PP attachment, quantifier scope ("Every woman loves a man" → surface and inverse readings), collective vs. distributive plurals, and intensional readings. You see each candidate as inspectable logic and choose the one you meant.

Compare that to the status quo: a human translating an English spec to code *also* resolves these ambiguities — invisibly, in their head, with no record of the choice. LOGOS makes the ambiguity explicit and the resolution auditable.

The parser itself is a genuine computational-linguistics pipeline — garden-path sentences, polarity items, tense and aspect, wh-movement, reciprocals, event semantics — and the test suite is organized by linguistic phenomenon, so none of this is folklore; it's pinned down by thousands of tests.

### Is this an LLM?

No. LOGOS is a deterministic compiler: lexer → parser → AST → logic. Same input, same output, every time. No sampling, no hallucination, no temperature. An LLM can *write* LOGOS for you, but the meaning of what it wrote is fixed by the compiler — which makes LOGOS a natural target language for AI-generated specs: the AI drafts, the kernel checks.

---

## The Trust Story

### How is this different from other "English to Z3" tools?

Tools that end at "the SMT solver said unsat" put a multi-million-line solver inside the trusted computing base. LOGOS inverts that:

- **Z3 is an advisor, not an arbiter.** Solvers and search procedures are *proof-producing oracles*: they emit an actual proof term, and a small dependent-type kernel is the only verifier.
- A bug in any oracle gives you a **failed proof, never a false theorem**.
- Proofs serialize to **independently re-checkable certificates** — a standalone checker re-verifies them from scratch without trusting the toolchain that produced them.

### What exactly is in the trusted computing base?

The kernel type checker, plus **seven commutative-ring axioms** for primitive integer arithmetic (`add_comm`, `add_assoc`, `add_zero`, `mul_comm`, `mul_assoc`, `mul_one`, `mul_distrib_add`). That's the inventory — and a test locks it, so the TCB can't silently grow. Closed arithmetic (`2 + 3 = 5`) needs zero axioms; it's proven by computation.

Z3, the proof search engine, the arithmetic normalizer, and the English front-end are all **outside** the TCB.

### Why are the ring axioms axioms at all?

`Int` is an opaque primitive (machine integers) for performance — the same trade-off Coq makes with `Int63` and Lean makes with GMP-backed `Nat`. Because it has no constructors, ring laws can't be proven by induction, so they're axioms for now. The roadmap is the Coq `Int63` approach: anchor the primitive to an inductive model via a few specification axioms and *prove* the ring laws from the spec. Everything downstream depends only on the ring-law interface, so the swap is churn-free.

### Can I check a proof without trusting your code?

Yes. Certificates are JSON, and the kernel ships a standalone re-checker:

```bash
cargo run -p logicaffeine-kernel --example recheck --features serde -- cert.json
```

The checker **rebuilds its own prelude** rather than trusting any context carried in the certificate — so a certificate can't smuggle in extra axioms. The checker is small enough to audit by hand.

### What can the kernel do that Z3 can't?

Induction. There's a test where the Z3 oracle honestly declines a goal (returns "I don't know") and the kernel certifies it by structural induction (`Term::Fix`). Conversely, anything Z3 *does* find gets elaborated to a kernel-checked proof rather than taken on faith.

### What happens when my spec is inconsistent?

LOGOS doesn't just say "unsat" — **verified conflict detection** produces a kernel-checked proof of `False` and identifies *which premises clash*, with no false alarms on consistent sets. The classic demo: feed it the Barber paradox ("the barber shaves everyone who doesn't shave themselves") and get back a certified contradiction pointing at the offending premise.

---

## Hardware Verification

### What does LOGOS do for hardware?

English specs become **IEEE 1800-2023 compliant SystemVerilog Assertions** — property connectives, LTL operators, sequence composition, abort operators, local variables, bounded delays ("within 4 cycles" → `##[0:4]`), the works. The 1800-2023 upgrade (array `.map()`, `type(this)`, `rand real` checker variables, triple-quoted action blocks) shipped in v0.9.16.

### Is it just assertion translation?

No — there's a full model-checking stack behind it:

- **IC3/PDR**, **k-induction** with auxiliary invariant strengthening, and **Craig interpolation** for unbounded properties
- **CEGAR** abstraction refinement with automatic predicate discovery
- **Liveness** checking with fairness constraints and ranking functions
- **Multi-clock / CDC** analysis (synchronizer verification, metastability)
- **Compositional verification** via assume-guarantee reasoning

### Does it check the spec itself?

Yes — spec health is a first-class concern:

- **Vacuity analysis** (IEEE 16.14.8 compliant) — assertions that pass trivially
- **Contradiction, vacuity, and redundancy detection** on the English spec, before you ever run a simulator
- **Sufficiency analysis** — lonely signals, unconstrained outputs, missing handshake patterns
- **CEGAR synthesis refinement** — when a synthesized assertion diverges from intent, it's classified as too-strong or too-weak, with a transformation strategy

### Can it go from properties to circuits?

Yes — **Verilog extraction from kernel proof terms** via the Curry-Howard correspondence, plus Z3-guided reactive synthesis with realizability checking. Certificates embed proof witnesses, so synthesis results are independently checkable too.

### How do you measure SVA coverage?

Against public benchmarks: engineering specs target **FVEval NL2SVA** (300 cases), **VERT** (20,000 cases), and **AssertionBench** (101 designs), aiming at complete IEEE 1800 SVA coverage.

---

## The Language & System

### English programs compile to *Rust*?

Yes. Imperative mode compiles English to executable Rust with a runtime library. The same kernel that checks proofs also governs the language that runs — LOGOS both **proves and executes** in one system, which is rare: proof assistants usually don't run real programs, and languages usually don't carry proofs.

### What's the partial evaluator about?

LOGOS has a self-applicable partial evaluator (written in LOGOS itself) working toward the **Futamura projections** — specializing the interpreter with respect to a program yields a compiler, mechanically. An abstract-interpretation oracle (a product lattice over types, collection shapes, nullability, intervals, and aliasing) feeds it. It's the foundation for a verified-by-construction compilation tier.

### How big is the test suite?

**~8,700 tests across 13 crates**, built strictly test-first. House philosophy: *tests are the IP; code is ephemeral*. Tests are organized as phases by linguistic and logical complexity, so the suite doubles as a map of exactly which phenomena are supported.

### What's the architecture, briefly?

```
English → Lexer → Parser → AST → Semantics (DRS / events) → FOL
                                      ├→ Proof engine → Kernel certificate
                                      ├→ SVA synthesis → SystemVerilog
                                      └→ Imperative codegen → Rust
```

Thirteen crates with lockstep versioning, published to crates.io, plus an LSP server and a web playground.

---

## Practical

### Where can I try it?

The playground at **[logicaffeine.com/guide](https://logicaffeine.com/guide)** runs in the browser. The CLI (`largo`) and crates are on crates.io.

### What's the license?

**BSL 1.1.** Free for individuals, teams under 25, and education; commercial licensing is contact-based. Z3-backed static verification is the gated commercial feature.

### What's honestly not done yet?

- The **full natural-language Barber paradox** end-to-end (the clean self-referential form certifies today; the NL rendition needs nested-quantifier rule extraction and event-abstraction definitions, in progress).
- **Proving the ring axioms** from an inductive integer model (designed, not yet built — see above).
- The **Futamura/JIT tier** is a roadmap with its first phases landed, not a shipped compiler.
- Quantum backend (Cirq) is at the detailed-plan stage.

We'd rather tell you where the edge is than have you find it.

### Why "Logicaffeine"?

Logic, with enough caffeine to compile the universe's information. No, not collect — *compile*. Like code.
