# LOGOS / Logicaffeine — FAQ

**[Try LOGOS Online →](https://logicaffeine.com/studio)**

---

## The Basics

### What is LOGOS?

LOGOS is a natural language compiler: plain English in, formal artifacts out. It has two modes:

- **Logic mode** — English sentences become first-order logic, and provable claims become *kernel-checked proof certificates*.
- **Imperative mode** — English programs run on a tree-walking interpreter, a register bytecode VM and a copy-and-patch JIT, and compile ahead of time to Rust, C, WebAssembly, RISC-V and CUDA.

The spec doesn't *describe* the program. The spec **is** the program.

### Isn't English too ambiguous to be a spec language?

English is ambiguous, and LOGOS surfaces the ambiguity instead of papering over it. When a sentence has one reading, you get that reading as explicit formal logic (Unicode FOL or LaTeX). When it genuinely has more than one, **LOGOS enumerates every defensible reading as a parse forest** rather than silently picking for you: lexical ambiguity (noun vs. verb), PP attachment, quantifier scope ("Every woman loves a man" → surface and inverse readings), collective vs. distributive plurals, and intensional readings. You see each candidate as inspectable logic and choose the one you meant.

Compare that to the status quo: a human translating an English spec to code *also* resolves these ambiguities — invisibly, in their head, with no record of the choice. LOGOS makes the ambiguity explicit and the resolution auditable.

The parser is a computational-linguistics pipeline: garden-path sentences, polarity items, tense and aspect, wh-movement, reciprocals, event semantics. The test suite is organized by linguistic phenomenon, so each of these is pinned by tests.

### Is this an LLM?

No. LOGOS is a deterministic compiler: lexer → parser → AST → logic. Same input, same output, every time. No sampling, no hallucination, no temperature. An LLM can *write* LOGOS for you, but the meaning of what it wrote is fixed by the compiler — which makes LOGOS a natural target language for AI-generated specs: the AI drafts, the kernel checks.

---

## The Trust Story

### How is this different from other "English to Z3" tools?

Tools that end at "the SMT solver said unsat" put a multi-million-line solver inside the trusted computing base. LOGOS keeps it out:

- **Z3 is an advisor, not an arbiter.** The proof search (backward chaining, CDCL/SAT, the arithmetic procedures) emits proof terms and certificates that a dependent-type kernel checks. A Z3 verdict is never certified: the certifier refuses an oracle step, so a goal discharged only by Z3 is reported as unverified.
- A bug in any search procedure gives you a **failed proof, never a false theorem**.
- Proofs serialize to **independently re-checkable certificates** — a standalone checker re-verifies them from scratch without trusting the toolchain that produced them.

### What exactly is in the trusted computing base?

The kernel type checker, plus the assumptions the standard prelude registers. `crates/logicaffeine_kernel/TCB.md` inventories them assumption by assumption, classified against Lean 4's trusted base and argued in writing. `tcb_ledger_lock` holds that ledger to the live prelude: it fails when the prelude registers an assumption the ledger does not classify, when the ledger names one the prelude does not register, and when a constant Lean proves is a LOGOS axiom.

Z3, the proof search engine, the arithmetic normalizer, and the English front-end are all **outside** the TCB.

### Are the integer laws axioms?

No. `Int` is Lean's inductive, with constructors `ofNat | negSucc`; `add`, `sub`, `mul`, `div`, `mod` and `le` are checked definitions over `Nat` arithmetic, and the public ring and order laws (`add_comm`, `mul_assoc`, `mul_distrib_add`, `le_trans`, …) are kernel-checked proof terms with empty axiom closures. Integer literals are definitionally equal to their constructor forms and evaluate natively, so closed arithmetic (`2 + 3 = 5`) is proven by computation. `div` and `mod` are Lean's Euclidean `Int.ediv`/`Int.emod`: `div (-7) 3 = -3`, `mod (-7) 3 = 2`, and `div x 0 = 0`.

### Can I check a proof without trusting your code?

Without trusting the proof search, yes. Certificates are JSON, and the kernel ships a standalone re-checker:

```bash
cargo run -p logicaffeine-kernel --example recheck --features serde -- cert.json
```

The checker **rebuilds the prelude itself**. A bare certificate carries only the proof term and the proposition it claims; a theorem or environment certificate discloses every declaration it adds, each re-validated by the kernel on the way in, and an entry that collides with a prelude name is refused. A certificate minted against a retired prelude version is refused rather than migrated. `--package <dir>` replays a Lean → LOGOS proof package the same way. The trusted surface of a re-check is the `logicaffeine-kernel` crate and `serde_json` — no proof search, no SMT solver.

### What can the kernel do that Z3 can't?

Induction. `phase104_induction_vs_z3` poses `∀n:Nat. n + 0 = n`: the Z3 oracle declines it, the proof engine proves it by structural induction, and the certifier turns a structural-induction derivation into a recursive kernel term (`Term::Fix`) whose recursion the kernel's termination guard checks.

### Is the compiled code proved to match its source?

Not yet. `logicaffeine_tv` is translation validation, built to prove for each compile that the emitted Rust behaves observably like its LOGOS source, with Z3 discharging the equivalence. Today only the LOGOS side has a symbolic executor. Nothing executes the emitted Rust symbolically yet, so the source-to-Rust equivalence is not closed.

What runs is the source encoder and its soundness check. `check_encoder_sound` runs a program through the tree-walking interpreter and through the symbolic encoder, then proves with Z3 that they agree on every `Show` and on whether an error was raised. It covers straight-line `Int`/`Bool` programs and a determinate concurrency fragment; anything else is reported `Unsupported`, never as agreement. Translation validation is also not a machine-checked proof of the compiler: its trust rests on the encoders, Z3 and rustc. Between the execution tiers, the evidence today is the differential suites described under [What does an English program run on?](#what-does-an-english-program-run-on).

### What happens when my spec is inconsistent?

LOGOS doesn't stop at "unsat". **Verified conflict detection** produces a kernel-checked proof of `False` and names *which premises clash*, with no false alarms on consistent sets (`phase_conflict_detection`). The Barber paradox stated over predicates (the barber shaves exactly the men who do not shave themselves) certifies to a kernel term of type `False`.

---

## Hardware Verification

### What does LOGOS do for hardware?

English specs become **SystemVerilog Assertions** — property connectives, LTL operators, sequence composition, abort operators, local variables, and bounded delays ("within three cycles" → a `##[..]` delay window). The SVA model parses and round-trips the IEEE 1800-2023 additions (triple-quoted action-block strings, `$timeunit`/`$timeprecision`/`$stacktrace`, array `.map()`, `type(this)`, `rand real` checker variables); array `.map()` and `type(this)` have no bounded translation and fail closed, encoding to `False` in the verification path (`phase_hw_sva_2023`).

### Is it just assertion translation?

No — there's a model-checking stack behind it (`logicaffeine_verify`, the `verification` feature):

- **IC3/PDR**, **k-induction**, and **interpolation-based** model checking for unbounded properties, with candidate invariants discovered from the design's knowledge graph (mutex, handshake, pipeline-stability and reset patterns)
- **CEGAR** predicate abstraction with refinement
- **Liveness** checking by liveness-to-safety reduction, with fairness constraints
- **Multi-clock** domain modeling and **CDC** analysis (synchronizers, metastability)
- **Compositional verification** via assume-guarantee reasoning

### Does it check the spec itself?

Yes — spec health is a first-class concern:

- **Vacuity analysis** (IEEE 16.14.8 nonvacuous-evaluation rules) — assertions that pass trivially
- **Contradiction, vacuity, and redundancy detection** on the English spec, with a minimal unsatisfiable subset for a contradiction, before you ever run a simulator
- **Sufficiency analysis** — lonely signals, unconstrained outputs, missing handshake patterns
- **CEGAR synthesis refinement** — when a synthesized assertion diverges from intent, it's classified as too-strong or too-weak, with a transformation strategy

### Can it go from properties to circuits?

Yes. **Verilog extraction from kernel proof terms** follows the Curry-Howard correspondence: a proof inhabiting the spec's type is the circuit. Reactive synthesis from LTL builds a controller (a memoryless safety strategy, else a bounded-state controller over a Büchi automaton) and reports a contradictory spec as unrealizable.

### How do you measure SVA coverage?

`phase_hw_sva_coverage` and `phase_hw_sva_ieee1800` walk the IEEE 1800 SVA constructs through parsing, translation and kernel encoding. `phase_hw_benchmark` runs a table of hardware property sentences through the English → SVA synthesizer. When NVIDIA's FVEval NL2SVA data is checked out at the repository root, it also reports the pass rate on those specs as a non-failing discovery probe; specs confirmed to pass become individual regression tests.

---

## The Language & System

### What does an English program run on?

The same source runs on the tree-walking interpreter, the register bytecode VM, and EXODIA, the copy-and-patch JIT that the VM tiers hot functions and loops up to on x86-64 Linux and macOS. `largo build` compiles it ahead of time through Rust; `largo emit` produces Rust, C, RISC-V assembly, WebAssembly modules and CUDA, and writes device bundles for Tenstorrent, analog, Plasticine and SambaNova RDU targets. Differential suites (`tier_differential_default`, `jit_vm_differential`, `e2e_differential`, `wasm_aot_lock`, `riscv_aot_lock`, `cuda_aot_lock`, …) compare each execution tier's results with the tree-walker and the VM over their corpora. The kernel that checks proofs sits behind the same language: a program can `Assert that` a logical claim, and `largo prove` certifies its `## Theorem` blocks. LOGOS both **proves and executes** in one system.

### Can I build a web application in it?

Yes, with LGX, the web framework. A page is one file: `State:` and `Actions:` in English sentences, `View:` as HTML with `{…}` holes, `Style:` as scoped CSS, all compiled to WebAssembly for the browser. The backend is a set of typed `## Route`s with sessions and CSRF checks. The HTTP helpers, static-file server, JSON reader, session cookies and PostgreSQL wire protocol are LOGOS source in the standard library.

`largo new mysite --web` scaffolds a project, and `largo serve` runs pages and backend together with hot reload. `largo test` runs each page's `## Test` blocks on the tree-walker and the VM (and in headless Chrome with `--browser`). `largo build --release` packs pages, assets and backend into one native binary, and `largo build --static` exports a site for any static host. See [Frontend](../frontend.md), [Web](../web.md), and the example projects in [`examples/`](../../examples/README.md).

### What's the partial evaluator about?

LOGOS has a self-applicable partial evaluator written in LOGOS (`crates/logicaffeine_compile/src/optimize/pe_source.logos`) that runs all three **Futamura projections**. Specializing the interpreter to a program yields a compiled program; specializing the partial evaluator to the interpreter (`pe(pe, int)`) yields a compiler; specializing it to itself (`pe(pe, pe)`) yields a compiler generator. `futamura_tier_lock` requires a first-projection residual to run identically on the tree-walker and the VM and to match the original program's output. An abstract-interpretation oracle (a product lattice over types, collection shapes, nullability, intervals, and aliasing) feeds the optimizer and the code generators.

### What does the test suite cover?

It is built strictly test-first. House philosophy: *tests are the IP; code is ephemeral*. Every test lives in `crates/logicaffeine_tests`. The logic-mode suites are organized as phases by linguistic and logical complexity, so the suite doubles as a map of exactly which phenomena are supported. Beside them sit the execution-tier differentials, concurrency, the wire codec, cryptography, the proof engine and the kernel's trusted-base ledger. The documentation is under test too: every LOGOS example in the guides compiles, every shown output is what the program prints, and every English → FOL pair is re-derived by the compiler (`doc_examples`). Mutation testing is scoped to the correctness core; the [challenge rules](CHALLENGE.md) say where that measurement stands. `./scripts/run-all-tests-fast.sh` runs the full suite.

### What's the architecture, briefly?

```text
English → Lexer → Parser → AST ─┬→ Semantics (DRS / events) → FOL
                                 │                             ├→ Proof engine → Kernel certificate
                                 │                             └→ SVA synthesis → SystemVerilog
                                 └→ Imperative program ─┬→ Interpreter · VM → JIT
                                                        └→ AOT: Rust · C · WASM · RISC-V · CUDA
```

A workspace of Rust crates with lockstep versioning, published to crates.io, plus the `largo` build tool, an LSP server with a VS Code extension, and the browser Studio.

---

## Practical

### Where can I try it?

The Studio at **[logicaffeine.com/studio](https://logicaffeine.com/studio)** runs the parser, interpreter, proof engine and code generation in the browser. The CLI (`largo`) installs as a prebuilt binary (`curl -fsSL https://logicaffeine.com/install.sh | sh`) or with `cargo install logicaffeine-cli`, and the crates are on crates.io.

### What's the license?

**MIT** (`LICENSE.md`), every crate and the website. Use it, ship it, sell what you build on it. Z3-backed static verification runs for anyone with the full build (`largo verify`), no key involved. Questions for the team go through [logicaffeine.com/contact](https://logicaffeine.com/contact).

### What's honestly not done yet?

- The **English Barber paradox**: the proof engine finds a derivation, but the certifier does not yet turn it into a kernel term, so `compile_theorem` reports it as not proved (`phase105_string_door_honesty`); the predicate-level form certifies (above).
- **Source ↔ emitted-Rust equivalence**: translation validation checks the LOGOS side only; the emitted Rust has no symbolic executor yet (above).
- **Jones-optimal self-application**: the second and third Futamura projections run by genuine self-application, but their residuals still carry interpreter dispatch; `futamura_p2p3_ratchet` holds the surviving dispatch count to ceilings that may only fall.
- **A trusted base larger than Lean's three axioms**: `crates/logicaffeine_kernel/TCB.md` names every assumption Lean does not share and the argument for each.
- **No quantum backend**: a Cirq target is a planned milestone on the [roadmap](https://logicaffeine.com/roadmap), with no code behind it.

The edges are stated here so you don't find them the hard way.

### Why "Logicaffeine"?

Logic, with enough caffeine to compile the universe's information. No, not collect — *compile*. Like code.

---
[Docs index](../README.md) · [Root README](../../README.md) · [Changelog](../../CHANGELOG.md)
