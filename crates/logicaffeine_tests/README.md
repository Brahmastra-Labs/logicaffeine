# logicaffeine-tests

Integration test suite and E2E harness for the Logicaffeine ecosystem. 314 test files covering natural language parsing, formal verification, type theory, distributed systems, hardware synthesis, and three Futamura projections.

## Overview

This crate contains no library code -- all tests run from the `tests/` directory. It provides:

- **Phase tests** for linguistic phenomena, type theory, proofs, hardware verification, and Futamura projections
- **E2E tests** for full compilation pipeline (LOGOS -> Rust -> binary, LOGOS -> C -> binary)
- **Test harness** with parsing, compilation, interpretation, C codegen, C ABI linkage, and assertion utilities

## Running Tests

Default workflow (skips slow E2E tests):

```bash
cargo test --workspace -- --skip e2e
```

Run all tests including E2E:

```bash
cargo test --workspace
```

Run with verification (requires Z3):

```bash
cargo test --workspace --features verification -- --skip e2e
```

Persistent logging for long test runs:

```bash
cargo test --workspace -- --skip e2e 2>&1 | tee test_output.log
```

## Test Organization

| Category | Files | Description |
|----------|-------|-------------|
| Phase tests (numbered) | 127 | Linguistics, language features, kernel, proofs, metamathematics |
| Phase tests (named) | 104 | Hardware verification, CRDTs, tactics, Futamura, optimization |
| E2E tests | 58 | End-to-end compilation and execution |
| Debug tests | 7 | Diagnostic and targeting helpers |
| Other tests | 18 | Specialized areas (interpreter, parser, codegen audit, etc.) |
| **Total** | **314** | |

---

## Numbered Phases

### Phases 1-9: Core Linguistics

Parse and resolve structurally ambiguous English sentences into first-order logic.

| Phase | File | What it tests |
|-------|------|---------------|
| 1 | `phase1_garden_path` | Garden-path sentences: "The horse raced past the barn fell" -- reduced relative clause resolution |
| 2 | `phase2_polarity` | Negative polarity items: "I did not see any dogs" -- NPI licensing in downward-entailing contexts |
| 3 | `phase3_aspect`, `phase3_time` | Vendler aspect classes (states, activities, achievements, accomplishments) and temporal reference |
| 4 | `phase4_movement`, `phase4_reciprocals` | Syntactic movement (raising, control, passivization) and reciprocal constructions ("each other") |
| 5 | `phase5_wh_movement` | Wh-extraction: "What did John think that Mary bought?" -- long-distance dependencies |
| 6 | `phase6_complex_tense` | Stacked tense/aspect: "will have been being signed" -- future perfect progressive passive |
| 7 | `phase7_semantics` | Neo-Davidsonian event semantics with thematic roles (agent, theme, instrument, manner) |
| 8 | `phase8_degrees` | Gradable adjectives, comparatives, superlatives, degree modifiers |
| 9 | `phase9_conversion`, `phase9_structured_concurrency` | Type conversion and structured concurrency primitives |

### Phases 10-19: Advanced Linguistics

| Phase | File | What it tests |
|-------|------|---------------|
| 10 | `phase10_ellipsis`, `phase10_io`, `phase10b_sluicing` | VP ellipsis ("John ran and Mary did too"), I/O operations, sluicing ("Someone left but I don't know who") |
| 11 | `phase11_metaphor`, `phase11_sorts` | Metaphorical predication, ontological sort hierarchies |
| 12 | `phase12_ambiguity` | Lexical and structural ambiguity resolution |
| 13 | `phase13_mwe` | Multi-word expressions ("kick the bucket", "by and large") |
| 14 | `phase14_ontology` | Ontological categories and type-theoretic sorts |
| 15 | `phase15_negation` | Sentential negation, constituent negation, double negation |
| 16 | `phase16_aspect` | Fine-grained aspectual composition |
| 17 | `phase17_degrees` | Degree semantics, measure phrases, differential comparatives |
| 18 | `phase18_plurality` | Plural quantification, distributive vs. collective readings |
| 19 | `phase19_group_plurals` | Group-forming plurals and committee-type nouns |

### Phases 20-29: LOGOS Language Core

The imperative programming language built on top of the linguistic parser.

| Phase | Files | What it tests |
|-------|-------|---------------|
| 20 | `phase20_axioms` | Axiom declarations and logical foundations |
| 21 | `phase21_block_headers`, `phase21_imperative_verbs`, `phase21_ownership` | Block structure, imperative verb parsing ("Set x to 5", "Add item to list"), ownership tracking |
| 22 | `phase22_equals`, `phase22_index`, `phase22_is_rejection`, `phase22_resolution`, `phase22_scope` | Equality semantics, collection indexing, is-rejection rules, name resolution, lexical scoping |
| 23 | `phase23_blocks`, `phase23_parsing`, `phase23_stmt`, `phase23_tokens`, `phase23_types` | Block parsing, statement forms, token classification, type annotations |
| 24 | `phase24_codegen`, `phase24_wired_types` | Rust code generation, built-in type wiring |
| 25 | `phase25_assertions`, `phase25_smoke_tests`, `phase25_type_expr` | Runtime assertions, smoke tests, type expression parsing |
| 26 | `phase26_e2e` | End-to-end language pipeline validation |
| 27 | `phase27_guards` | Guard clauses and conditional dispatch |
| 28 | `phase28_precedence` | Operator precedence and associativity |
| 29 | `phase29_runtime` | Runtime behavior and error handling |

### Phases 30-38: Language Features

| Phase | File | What it tests |
|-------|------|---------------|
| 30 | `phase30_iteration` | For-in loops, while loops, repeat-until, break/continue |
| 31 | `phase31_structs` | Struct definition, field access, mutation, nested structs |
| 32 | `phase32_functions` | Function definition, recursion, closures, higher-order functions |
| 33 | `phase33_enums` | Enum/variant types, pattern matching via Inspect |
| 34 | `phase34_generics` | Generic type parameters, monomorphization |
| 35 | `phase35_proofs`, `phase35_respectively` | Proof-carrying code, "respectively" distributive constructions |
| 36 | `phase36_modules` | Module system and namespacing |
| 37 | `phase37_cli` | CLI tool (`largo`) commands and flags |
| 38 | `phase38_stdlib` | Standard library functions and builtins |

### Phases 41-57: Semantics and Distributed Systems

| Phase | Files | What it tests |
|-------|-------|---------------|
| 41 | `phase41_event_adjectives` | Event-modifying adjectives in formal semantics |
| 42 | `phase42_drs` | Discourse Representation Structures (DRS) -- formal discourse semantics |
| 43 | `phase43_collections`, `phase43_discourse_scope`, `phase43_refinement`, `phase43_type_check` | Collection types, discourse scope, refinement types, type checking |
| 44 | `phase44_distributive`, `phase44_modal_subordination` | Distributive readings, modal subordination ("A wolf might come in. It would eat you.") |
| 45 | `phase45_intension`, `phase45_session` | Intensional semantics (de re/de dicto), session types |
| 46 | `phase46_agents`, `phase46_ellipsis` | Agent-based concurrency, ellipsis resolution |
| 48 | `phase48_network` | Network protocol primitives |
| 49 | `phase49_crdt` | Conflict-free Replicated Data Types (GCounter, PNCounter, ORSet) |
| 50 | `phase50_security` | Security policies and capability checking |
| 51 | `phase51_mesh` | Mesh networking topology |
| 52 | `phase52_sync` | Synchronization primitives |
| 53 | `phase53_persistence` | Durable state and recovery |
| 54 | `phase54_concurrency` | Concurrent execution patterns |
| 57 | `phase57_maps` | Map/dictionary operations |

### Phases 60-69: Proof Theory

| Phase | File | What it tests |
|-------|------|---------------|
| 60 | `phase60_proof_engine` | Core proof engine: derivation construction, rule application, proof validation |
| 61 | `phase61_induction` | Mathematical induction over natural numbers and structural induction |
| 62 | `phase62_oracle` | Oracle-guided proof search and decision procedures |
| 63 | `phase63_theorem_parser` | Parsing theorem statements and proof scripts from natural language |
| 65 | `phase65_event_semantics` | Formal event semantics in the proof system |
| 66 | `phase66_higher_order` | Higher-order logic: quantification over predicates and functions |
| 67 | `phase67_pattern_unification` | Higher-order pattern unification (Miller patterns) |
| 68 | `phase68_auto_induction` | Automated induction principle generation |
| 69 | `phase69_kernel_coc` | **Calculus of Constructions kernel** -- the foundational type theory. Universe hierarchy (Type 0 : Type 1 : Type 2 ...), dependent function types (Pi), identity function typing (forall A:Type. A -> A), type mismatch rejection |

### Phases 70-79: Kernel Type Theory

Implements CIC (Calculus of Inductive Constructions) -- the theory underlying Coq and Lean.

| Phase | File | What it tests |
|-------|------|---------------|
| 70 | `phase70_inductive_types`, `phase70b_elimination`, `phase70c_computation` | **Inductive type formation** (Nat, Bool, List), elimination principles (recursors), computation rules (iota reduction) |
| 71 | `phase71_cumulativity` | Universe cumulativity: Type i is a subtype of Type (i+1) |
| 72 | `phase72_kernel_prelude` | Built-in types and standard definitions |
| 73 | `phase73_certifier` | Proof certification: kernel verifies proof terms independently |
| 74 | `phase74_certify_quantifiers` | Universal and existential quantifier certification |
| 75 | `phase75_certify_intro` | Introduction rule certification (conjunction, disjunction) |
| 76 | `phase76_certify_induction` | Induction principle certification -- verified structural induction |
| 77 | `phase77_certify_exists` | Existential witness certification |
| 78 | `phase78_e2e_verification` | End-to-end: theorem statement -> proof -> kernel verification -> pass/reject |
| 79 | `phase79_termination` | Termination checking for recursive definitions (structural decrease) |

### Phases 80-90: Advanced Kernel

| Phase | File | What it tests |
|-------|------|---------------|
| 80 | `phase80_equality_rewriting` | Propositional equality, rewriting, transport along paths |
| 81 | `phase81_computation` | Beta, delta, iota, zeta reduction strategies |
| 82 | `phase82_delta_reduction` | Definition unfolding and delta reduction |
| 83 | `phase83_vernacular` | Command language: Definition, Theorem, Lemma, Inductive |
| 84 | `phase84_extraction`, `phase84_extraction_e2e` | **Program extraction**: verified kernel proofs -> executable Rust code. E2E: 2+3=5 through kernel proof -> extraction -> Rust compilation -> execution |
| 85 | `phase85_zones` | Zone-based memory management |
| 86 | `phase86_kernel_primitives` | Primitive operations in the kernel |
| 87 | `phase87_reflection` | Computational reflection: kernel terms as data |
| 88 | `phase88_substitution` | Capture-avoiding substitution with de Bruijn indices |
| 89 | `phase89_computation` | Advanced computation rules and normalization |
| 90 | `phase90_bounded_eval` | Bounded evaluation with fuel/step limits |

### Phases 91-99: Metamathematics and Automation

| Phase | File | What it tests |
|-------|------|---------------|
| 91 | `phase91_quote` | Quoting and syntax reflection |
| 92 | `phase92_inference` | Type inference and elaboration |
| 93 | `phase93_diagonal_lemma` | **Diagonal lemma**: `syn_diag : Syntax -> Syntax` -- the self-reference mechanism. Constructs fixed points: syn_diag(x) = syn_subst(syn_quote(x), 0, x). Foundation for Godel's theorems |
| 94 | `phase94_godel_sentence` | **Godel sentence construction**: G = "I am not provable". Defines Provable via existential over Derivations. Verifies G contains genuine self-reference (G's AST size > template size) |
| 95 | `phase95_incompleteness` | **Godel's incompleteness theorems**: (I) Consistent -> not(Provable G), (II) Consistent -> not(Provable ConsistentSyn) -- "if LOGOS is consistent, it cannot prove its own consistency". Both theorem statements verified as well-typed Prop |
| 96 | `phase96_tactics` | Proof tactics: reflexivity (a = a via DRefl), congruence (Leibniz's law: a=b implies C[a]=C[b]), tactic success/failure classification |
| 97 | `phase97_deep_induction` | Deep structural induction over nested types |
| 98 | `phase98_strategist` | Proof strategy selection and backtracking search |
| 99 | `phase99_solver` | Automated theorem solving combining multiple tactics |

### Phases 100+: Summit Challenges

| Phase | File | What it tests |
|-------|------|---------------|
| 100 | `phase100_the_summit` | Congruence tactic proofs: DCong (Leibniz's law), chaining congruence applications, full induction proofs with congruence in the step case (forall n. add(n, Zero) = n) |
| 101a | `phase101a_poly_inductive` | Polymorphic inductive types |
| 101b | `phase101b_generic_elim` | Generic elimination principles |
| 101c | `phase101c_list_ops` | Verified list operations |
| 101d | `phase101d_theorems` | Theorem library proofs |
| 102 | `phase102_bridge` | Bridge between proof system and executable code |
| 103 | `phase103_generics` | Advanced generic programming |

---

## Named Phase Categories

### Futamura Projections (691 tests across 4 files)

The crown jewel of the optimization pipeline. Implements a self-interpreter in LOGOS, then applies partial evaluation to achieve all three Futamura projections.

| File | Tests | What it tests |
|------|-------|---------------|
| `phase_bta` | 37 | **Binding-time analysis**: classifies every expression as Static (compile-time) or Dynamic (runtime). Tests SCC-based convergence for mutual recursion, polyvariant analysis (different static args create different specialization variants), loop unrolling when bounds are static |
| `phase_partial_eval` | 44 | **Partial evaluation**: specializes functions by substituting static arguments. factorial(10) -> 3628800 fully at compile time. Cascading specialization through call chains. Effect tracking distinguishes Check (security, specializable) from Show (IO, blocked). Variant limit of 8 prevents code bloat. Structured keys prevent name collisions |
| `phase_supercompile` | 66 | **Supercompilation**: unified optimization engine subsuming constant folding, DCE, deforestation, and PE. Homeomorphic embedding detects divergence ((1+2) embeds in ((1+1)+(2+1)) via coupling). Most Specific Generalization: MSG(a+b, a+c) = a+?1 with fresh variable. Identity property: all-dynamic code = original (zero overhead). Loop widening via MSG preserves known variables |
| `phase_futamura` | 544 | **Self-interpreter and projections**: complete LOGOS interpreter written in LOGOS itself (eval_expr, eval_stmt, eval_main). Tests recursive factorial, fibonacci, mutual recursion through the self-interpreter. P1: PE(interpreter, program) = compiled_program. Encoding roundtrip verification. 544 tests proving the self-interpreter is correct |

### Hardware Verification (35 files)

Complete pipeline: **English specification -> FOL -> Knowledge Graph -> SVA -> Bounded IR -> Z3 equivalence**.

| File | What it tests |
|------|---------------|
| `phase_hw_e2e_pipeline` | Full pipeline from English specs to verified SVA. Real protocol patterns: AXI4 write handshake, SPI chip select, 3-way arbiter fairness, I2C, FIFO overflow protection. Labeled "THE PIPELINE THAT NOBODY ELSE HAS" |
| `phase_hw_futamura` | Futamura projections applied to hardware verification -- specializing the verification pipeline itself |
| `phase_hw_cegar_z3` | **CEGAR refinement loop**: starts with wrong SVA (e.g. overlapping implication |=>), automatically strengthens/weakens to match spec (non-overlapping |->). Correctly classifies divergence as TooStrong vs TooWeak. Post-refinement Z3 validation |
| `phase_hw_synthesis_z3` | **Synthesis from specs with Z3 proof**: English spec -> synthesize_sva_from_spec() -> check_z3_equivalence() -> HARD-ASSERT Equivalent. Patterns: G(P), F(P), G(P->Q), G(P->F(Q)) (handshake), G(not(P and Q)) (mutex), G(P->X(Q)) (next). Mutation tests catch altered bodies, swapped antecedent/consequent, missing eventually, removed negation. Protocol templates: AXI write address, APB setup |
| `phase_hw_codegen_sva` | SVA code generation from internal IR |
| `phase_hw_fol_to_sva`, `phase_hw_fol_translate` | FOL-to-SVA translation and lowering |
| `phase_hw_sva_ieee1800` | IEEE 1800 SystemVerilog Assertions compliance |
| `phase_hw_sva_roundtrip` | Parse -> render -> parse equivalence |
| `phase_hw_sva_surface`, `phase_hw_sva_translate` | SVA surface syntax, translation layers |
| `phase_hw_knowledge_graph`, `phase_hw_kg_extract` | Knowledge graph construction and extraction from specs |
| `phase_hw_rtl_extract`, `phase_hw_rtl_kg` | RTL signal extraction and RTL knowledge graphs |
| `phase_hw_equivalence`, `phase_hw_z3_equiv` | Structural and Z3 semantic equivalence checking |
| `phase_hw_e2e_z3`, `phase_hw_advanced_z3` | End-to-end Z3 pipeline, advanced Z3 features. Safety guardrails: bitvectors, array selects, and transitions MUST NOT silently become `true` |
| `phase_hw_invariants` | Hardware invariant verification |
| `phase_hw_temporal` | Temporal property verification |
| `phase_hw_consistency` | Cross-representation consistency |
| `phase_hw_coverage` | Specification coverage analysis |
| `phase_hw_decompose` | Property decomposition |
| `phase_hw_filter` | Property filtering and selection |
| `phase_hw_integration` | Cross-module integration |
| `phase_hw_lexicon` | Hardware domain vocabulary |
| `phase_hw_ontology` | Hardware ontology and type hierarchy |
| `phase_hw_protocols` | Protocol pattern library |
| `phase_hw_signal_bridge` | Signal-level bridging between representations |
| `phase_hw_spec_health` | Specification quality metrics |
| `phase_hw_sufficiency` | Specification completeness checking |
| `phase_hw_synthesis_refine` | Iterative synthesis refinement |
| `phase_hw_verify` | Core verification infrastructure |
| `phase_hw_waveform` | Waveform-level analysis |

### CRDT Suite (13 files, 48+ tests)

Production-grade Conflict-free Replicated Data Types with formal properties.

| File | What it tests |
|------|---------------|
| `phase_crdt_causal` | **Causal infrastructure** (34 tests): VClock merge commutativity/associativity/idempotence, dot-based event tracking, DotContext with clock compaction and out-of-order delivery, DeltaBuffer ring buffers for streaming deltas |
| `phase_crdt_concurrent` | **Concurrent stress tests** (14 tests): 10 threads x 1000 increments on GCounter (final merge = 10,000), PNCounter mixed inc/dec, ORSet concurrent add/remove with add-wins semantics, MVRegister conflict detection, LWWRegister timestamp ordering, VClock 10-clock x 100-event contention |
| `phase_crdt_delta` | Delta-state CRDT propagation |
| `phase_crdt_edge_cases` | Boundary conditions and degenerate inputs |
| `phase_crdt_language` | CRDT operations expressed in LOGOS syntax |
| `phase_crdt_mvregister` | Multi-value register with conflict tracking |
| `phase_crdt_ormap` | Observed-remove map operations |
| `phase_crdt_orset` | Observed-remove set with causal context |
| `phase_crdt_pncounter` | Positive-negative counter decomposition |
| `phase_crdt_sequence` | Ordered sequence CRDTs |
| `phase_crdt_serialization` | Binary serialization roundtrips (bincode) |
| `phase_crdt_stress` | High-contention stress testing |
| `phase_crdt_variants` | CRDT variant type interactions |

### Proof Tactics (8 files)

Automated reasoning tactics for the proof engine.

| File | What it tests |
|------|---------------|
| `phase_auto` | Automated proof search with backtracking |
| `phase_simp` | Simplification tactic (rewriting rules) |
| `phase_omega` | Linear arithmetic decision procedure |
| `phase_ring` | Ring algebra tactic (polynomial normalization) |
| `phase_lia` | Linear integer arithmetic solver |
| `phase_cc` | Congruence closure (equality reasoning) |
| `phase_induction` | Structural and well-founded induction |
| `phase_inversion` | Inversion on inductive hypotheses |

### Literate Tactics (7 files)

Each tactic also has a literate-mode proof script variant, testing natural-language-style proof input.

`phase_literate_{auto,cc,induction,lia,omega,ring,simp}`

### Optimization (5 files)

| File | What it tests |
|------|---------------|
| `phase_optimize` | Constant folding, dead code elimination, constant propagation |
| `phase_optimize_v2` | Next-generation optimizer with boolean algebra (x or true -> true, x and false -> false), self-comparison (x - x -> 0, x / x -> 1) |
| `phase_deforestation` | Intermediate data structure elimination: map -> filter -> fold without allocating intermediate lists |
| `phase_polyhedral` | Polyhedral loop optimization (affine transformations) |
| `phase_autoparallel` | Automatic parallelization of independent loop iterations |

### Other Named Phases (25 files)

| File | What it tests |
|------|---------------|
| `phase_abstract_interp` | Abstract interpretation framework |
| `phase_analysis` | Static analysis passes |
| `phase_barber_updated` | Russell's barber paradox formalization |
| `phase_benchmark_interp` | Interpreter performance benchmarks |
| `phase_bitwise` | Bitwise operations (AND, OR, XOR, shifts) |
| `phase_break` | Break/continue control flow |
| `phase_codegen_c` | **C code generation** (216 tests): hello world through Ackermann function, native function binding, keyword escaping, string operations, benchmarks (fib, sieve, bubble sort) |
| `phase_effects` | Algebraic effect tracking |
| `phase_escape_hatch` | Raw code escape hatches |
| `phase_ffi_requires` | FFI dependency declarations |
| `phase_hints` | Proof/optimization hint annotations |
| `phase_interpret_mode` | Interactive interpreter mode |
| `phase_interpreter_crdt` | CRDT operations through interpreter |
| `phase_interpreter_features` | Interpreter feature matrix |
| `phase_interpreter_map_keys` | Map key handling in interpreter |
| `phase_interpreter_policy` | Security policy enforcement in interpreter |
| `phase_interpreter_string_concat` | String concatenation in interpreter |
| `phase_kripke` | Kripke semantics for modal logic |
| `phase_lexer_refactor` | Lexer regression tests |
| `phase_mountain_climb` | **Optimization validation** (27 tests): constant folding, DCE, propagation, boolean algebra, IEEE 754 NaN handling |
| `phase_operator` | Custom operator definitions |
| `phase_ownership` | Ownership and borrowing semantics |
| `phase_primitives_extended` | Extended primitive operations |
| `phase_privation_modal` | Privation adjectives and modal logic |
| `phase_sets` | Set operations (union, intersection, difference) |
| `phase_totality` | Totality checking for functions |
| `phase_temporal_lexer` | Temporal expression lexing |
| `phase_temporal_operations` | Temporal logic operations |
| `phase_temporal_primitives` | Temporal primitive types |
| `phase_temporal_spans` | Time span arithmetic |
| `phase_verification` | Z3 verification integration |
| `phase_verification_refinement` | Refinement type verification |

---

## E2E Tests (58 files)

Full pipeline: LOGOS source -> Rust code generation -> cargo build -> execute -> check output.

### Codegen Verification (17 files)

Validates generated Rust code for each language construct:
`e2e_codegen_{collections,comparisons,control_flow,edge_cases,enums,expressions,functions,gaps,iteration,logical,maps,optimization,primitives,sets,structs,tuples,types,variables}`

### Language Feature E2E (19 files)

End-to-end validation of high-level features:
`e2e_{closures,collections,comparisons,control_flow,edge_cases,enums,expressions,feature_matrix,functions,iteration,logical,maps,primitives,ref_semantics,sets,string_interpolation,structs,tuples,types,variables}`

### Distributed Systems E2E (8 files)

Concurrent and distributed program execution:
`e2e_{async_cross_cutting,causal_consistency,crdt,gossip,gossip_edge_cases,mesh,multi_node,network_partition}`

### Other E2E (14 files)

`e2e_{concurrency,integration,interpreter_gaps,interpreter_optimization,language_gaps,math_builtins,policy,refinement,studio_examples,temporal,temporal_show,zones}`

---

## Other Tests (18 files)

| File | What it tests |
|------|-------------|
| `aktionsart_tests` | Vendler verb classification (states, activities, achievements, accomplishments) |
| `audit_codegen` | Code generation correctness auditing |
| `complex_combinations` | Complex multi-phenomena linguistic combinations |
| `diagnostic_bridge` | Diagnostic message bridge integration |
| `gq_test` | Generalized quantifier theory (every, some, most, few) |
| `grand_challenge_mergesort` | **Capstone E2E** (13 tests): parse merge sort in LOGOS -> generate Rust with LogosSeq -> cargo build -> execute -> verify [3,1,4,1,5,9,2,6] -> [1,1,2,3,4,5,6,9]. Validates 5+ layers: parsing, type checking, code generation, Rust compilation, runtime execution |
| `integration_tests` | Cross-module integration |
| `intensionality_tests` | De re / de dicto readings, belief contexts |
| `interpreter_tests` | Core interpreter correctness |
| `learn_state_tests` | Learning state machine |
| `modal_scope_tests` | Modal operator scope interactions |
| `parser_fixes_test` | Parser regression fixes |
| `struggle_tests` | Known-difficult edge cases |
| `symbol_dict_tests` | Symbol dictionary operations |
| `test_concurrency` | Low-level concurrency primitives |
| `torture_tests` | **Linguistic stress tests** (20 tests): maximal aspect chains ("will have been being signed"), triple-nested control ("seems to want to try to leave"), double center-embedding ("the dog that the cat that the mouse scared chased ran"), triple-quantifier scope, donkey anaphora, conditional donkeys, focus particles |
| `unlock_logic_tests` | Logic feature unlocking |
| `user_concurrency` | User-facing concurrency patterns |

---

## Feature Flags

| Feature | Description |
|---------|-------------|
| `verification` | Enables Z3-based verification tests (requires Z3 installed) |
| `ffi-link-tests` | Enables FFI C-linkage tests (requires gcc/cc) |
| `web-tests` | Enables web crate integration tests |

## Test Harness API

Located in `tests/common/mod.rs`.

### Result Types

```rust
pub struct E2EResult {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    pub rust_code: String,
}

pub struct CompileResult {
    pub binary_path: PathBuf,
    pub stderr: String,
    pub success: bool,
    pub rust_code: String,
}

pub struct InterpreterTestResult {
    pub output: String,
    pub error: String,
    pub success: bool,
}

pub struct CLinkResult {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    pub rust_code: String,
    pub c_code: String,
}
```

### Parsing

```rust
let view = parse_to_view("Every cat sleeps.");
let view = parse!("Every cat sleeps.");
```

### Compilation (no execution)

```rust
let result = compile_logos("print 42.");
assert!(result.success);
```

### Compilation + Execution

```rust
let result = run_logos("print 42.");
assert!(result.success);
assert!(result.stdout.contains("42"));
```

### Interpreter (no Rust compilation)

```rust
let result = run_interpreter("print 42.");
assert!(result.success);
```

### E2E Assertions

```rust
assert_output("print 42.", "42");                          // contains substring
assert_exact_output("print 42.", "42");                    // exact trimmed match
assert_output_lines("print 1.\nprint 2.", &["1", "2"]);   // line-by-line
assert_output_contains_all(source, &["hello", "world"]);   // all substrings (order-independent)
assert_runs("let x = 1.");                                 // runs without error
assert_panics("assert false.", "assertion");                // panics with message
assert_compile_fails("gibberish", "parse error");           // compilation fails
```

### Interpreter Assertions

```rust
assert_interpreter_output("print 42.", "42");              // exact trimmed match
assert_interpreter_output_lines(source, &["1", "2"]);     // line-by-line
assert_interpreter_output_contains("print 42.", "42");     // contains substring
assert_interpreter_runs("let x = 1.");                     // runs without error
assert_interpreter_fails("bad code", "error substring");   // fails with error
```

### C Codegen Assertions

```rust
assert_c_output("print 42.", "42");                        // LOGOS -> C -> gcc -> run
let result = compile_and_link_c(logos_source, c_code);     // staticlib + C ABI linkage
assert!(result.success);
```

### Macros

```rust
let view = parse!("Every cat sleeps.");
assert_snapshot!("my_test", actual_output);
```

## Extraction Harness

Located in `tests/extraction_common/mod.rs`. Compiles extracted Rust code from kernel proofs and runs it.

```rust
let result = run_extracted(rust_code, main_code);
assert!(result.success);

assert_extracted_output(rust_code, main_code, "expected");
```

## Snapshot Testing

12 snapshot files in `tests/snapshots/` covering Davidsonian semantics, donkey sentences, FFI exports, LaTeX output, passive constructions, and relative clauses.

Update all snapshots:

```bash
UPDATE_SNAPSHOTS=1 cargo test
```

## Dependencies

| Crate | Purpose |
|-------|---------|
| `logicaffeine-base` | Arena, interner, span types |
| `logicaffeine-kernel` | Type checker, CIC kernel |
| `logicaffeine-language` | Parser, lexer, AST |
| `logicaffeine-compile` | LOGOS -> Rust/C compilation |
| `logicaffeine-proof` | Proof engine and tactics |
| `logicaffeine-data` | Runtime data structures (LogosSeq, LogosMap) |
| `logicaffeine-system` | Distributed system primitives (CRDTs, gossip, mesh) |
| `logicaffeine-lexicon` | Vocabulary database |
| `logicaffeine-verify` | Z3-based formal verification |
| `tempfile` | Temporary directories for E2E |
| `futures` | Async interpreter execution |
| `tokio` | Async runtime for tests |
| `bincode` | Binary serialization (CRDT tests) |
| `serde` | Serialization framework |
| `serde_json` | JSON serialization |

## License

BUSL-1.1
