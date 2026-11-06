# Test suites

Every `.rs` file in this directory is one integration suite and one test binary:
`cargo nextest run -p logicaffeine-tests --test <file stem>`. Name the suites you run with `--test`; an `-E` filter
picks which tests run, but cargo still builds every binary the command points at.

A suite is named for what it pins. A name ending in `_lock` is an invariant of the product or of the tree, RED the
moment it breaks; a name ending in `_extra` holds the neighbours of an existing suite's cases. The families below
share a name prefix (`ls` the glob); every other suite stands alone under its own name. The directories hold shared
helpers, fixtures and goldens, not suites. A family that reaches eight suites gets its row here (`tests_index_lock`).

| Family | What it covers |
|---|---|
| `a_*` | One rule each, named as the sentence it pins: English-to-FOL semantics (conditionals, coordination, gapping, group subjects, event PPs, pronoun agreement, tense), the prover's premises and conclusions, the parser's rules for code (full stops, moves), and checker findings. |
| `aot_*` | The AOT-Rust tier: generated Rust keeps LOGOS value semantics, bindings and module globals. |
| `analog_*` | The analog compute lane: calibration, noise budgets, winner-take-all certificates, the benchmark classification. |
| `benchmark_*` | The benchmark harness: registry coverage, fair profiles, reference ports and every LOGOS execution tier. |
| `c_*` | The C backend: every binding, scope and stdlib call means on the C tier what it means on the interpreter. |
| `cascade_*` | The certified SAT cascade on combinatorial families (Ramsey, clique, colouring), each answer a checked proof. |
| `component_*` | LGX components: state, events, instances, and component tests run by `largo test`. |
| `correctness_*` | Language semantics: truthiness, bitwise and set operators, exact integer arithmetic, bindings. |
| `cuda_*` | The CUDA tier: GPU kernels byte-identical to the C oracle, tensor cores, attention, the Qwen forward pass. |
| `d5005_*` | The decode engine on one Intel PAC D5005: the serving roofline and the cycle twin, the arithmetic bit for bit, the model, its program and the machine's jobs over the card's memory, every unit in Verilog against its numerics, the DDR4 model and the weight image, the accelerator's OFS flow. |
| `dag_*` | The proof kernel's DAG core: memoized unfolding, the declaration meter, diagnostics, the Arena corpora. |
| `deploy_*` | The Samba book's DEPLOY tickets: each model family's twin brought up on the SN50 from its Hugging Face directory against transformers, the reference and the VM, and every published size's layer kinds lowered at real width and certified. |
| `derived_*` | Derived definitions: the kernel's derived auxiliaries (`noConfusion`, `below`/`brecOn`) and LGX derived state. |
| `e2e_*` | Whole programs compiled through the AOT-Rust tier and run: the class `--skip e2e` leaves out. |
| `exact_*` | Exact numbers on every tier: integers past 2^63, division exact by context, and the exact extension (algebraic numbers, computable reals, hyperreals) on the C, RISC-V, WebAssembly and device tiers, with the interpreter's and compiled code's value models held to one behaviour. |
| `experiment_*` | Research experiments with proofs attached: number theory, cryptanalysis, decidability. |
| `fabric_*` | The spatial fabric: programs lowered to a processing-element grid and run cycle-accurate. |
| `forge_*` | The copy-and-patch JIT's native lanes: word arithmetic, hashing, guards that deoptimize to the VM. |
| `hotswap_*` | Hot swap: background native compilation, native and bytecode replacement of a running function. |
| `http_*` | The HTTP server on every tier: concurrent workers, the cross-tier byte-identical differential, the engine bench. |
| `interp_*` | The tree-walking interpreter's IO: the virtual filesystem, concurrency, networking over the relay. |
| `jit_*` | EXODIA, the JIT: VM bytecode to native code, differential against the VM, codegen-quality gates. |
| `kernel_*` | The proof kernel end to end: honesty, registration, tactics, observational equality and differential oracles. |
| `lapack_*` | LOGOS-LAPACK: factorizations, eigensolvers and least squares against reference values. |
| `lean_*` | The Lean 4 interface: reading exports, replaying them in the kernel, the conformance corpus tier by tier with its confirmed kernel defects, the Arena cases. |
| `lgx_*` | LGX, the web framework: templates, checks, the page surface, compilation to WebAssembly. |
| `mlkem*` | ML-KEM-768 in LOGOS and in the native kernels, against the NIST vectors, with its speed bench. |
| `model_*` | A model from its checkpoint: the GGUF reader and quantized formats, the spec and tokenizer, the program the author writes, the f64 reference against llama.cpp's goldens, the bring-up scoreboard. |
| `multimodule_*` | Multi-file projects: namespaced types, directory modules, deterministic assembly. |
| `native_*` | Native decision procedures in the kernel: `native_decide` and the recursor-aware compiled deciders. |
| `nested_*` | Nested inductive types in the kernel, admitted all-or-nothing as Lean admits them. |
| `obs_*` | Observation statements: structured events on their own channel, zero cost when compiled out. |
| `one_*` | One of each thing, never two: an event's one path to a page and every listener it reaches, an LGX page's one implementation (its lowered LOGOS), the one IR below the front end and its fusion table with the artifacts its migration could move pinned by digest, a model's one logical representation, and the one Studio the front page and `/studio` draw. |
| `original_*` | Declarations from the original Lean corpus, bridged into the kernel unchanged. |
| `page_*` | LGX pages under test: traces after every statement, page coverage, isolation between tiers. |
| `parity_*` | Stdlib parity: each function against the other languages' standard library it matches. |
| `phase*` | The language by phase: English-to-FOL phenomena (garden paths, quantifiers, tense and aspect, movement) and imperative-mode features, one suite per phase. |
| `plasticine_*` | The Plasticine lane, the open RDU (ISCA 2017): the target and its mesh, placement and static routing of metapipelines, the control plane's tokens, credits and buffer depths, reader fusion, GEMM parity against the hand-written Spatial program, the public toolchain's numbers with our model held to them, the twin, the report and the PLDI'18 Table 4 gate. |
| `qa_*` | Quality locks on the network stack: codec error paths, the elastic controller, WebRTC edges. |
| `qwen*` | Qwen models written in LOGOS: the tiny model byte-exact on the VM, the Qwen3.8 bundle's stages and contracts, and Qwen3.8-Flash-Next's checkpoint read against the Samba book's section 7. |
| `rdu_*` | The SambaNova RDU lowering: whole-program lanes, fit reports, index datatypes. |
| `rust_*` | The AOT-Rust tier where values move: a place lent to a `mutable` parameter reads `nothing` while the call holds it (globals, items, map values, fields, nested places), copies keep their own values, lent lists and maps are written back level by level, a callee's change to a list never reaches an iteration over it, and the CRDT types. |
| `samba_*` | The RDU bring-up exam book (`benchmarks/PREPARE_FOR_SAMBA.md`): its lock, the papers it cites, the technique ledger drawn from them, the scorecard and climb ledgers, and the exam's rows, green and as handed REDs. |
| `sass_*` | NVIDIA SASS: the register dependency graph, control codes, the reuse optimizer, on real GEMM machine code. |
| `spatial_*` | The spatial IR: one dataflow representation lowered to fabrics, Tenstorrent and RDU, with its cost and work models. |
| `ssr_*` | Server-side rendering of LGX pages: inert markup, child components, scaling. |
| `stdlib_*` | The standard library as a whole: name hygiene, collisions, native-tier coverage. |
| `studio_*` | The Studio: the file opened decides its mode and its first frame is the state it ends in, its server render byte for byte, the debounced web preview, every example's contract and math document, the Heal button, and the browser scenarios the docs cite. |
| `tactic_*` | Proof tactics: combinators, elimination, the tactic surface. |
| `tensor_*` | Tensors as language: gather sugars, row blocks, compile-time evaluation of static tensor programs. |
| `tt_*` | Tenstorrent: the work meter, cycles that track work, dynamic meshes on the Tensix grid. |
| `vm_*` | The register bytecode VM: sessions, copy-on-write scaling, host callbacks. |
| `wasm_*` | The WebAssembly backend: AOT programs, arguments, text, coverage of the language. |
| `wpe_*` | The whole-program engine as silicon: board profiles, the Caravel project, a transformer on the engine. |
| `wprog_*` | The whole-program processor: one generic engine running any program, the benchmarks, completeness. |
