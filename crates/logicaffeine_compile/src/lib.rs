#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

// Re-export base types
pub use logicaffeine_base::{Arena, Interner, Symbol, SymbolEq};

// Re-export language types needed for compilation
pub use logicaffeine_language::{
    analysis::{DiscoveryPass, PolicyCondition, PolicyRegistry, TypeRegistry},
    arena_ctx::AstContext,
    ast, drs, error, formatter, lexer, mwe, optimization, parser,
    registry::SymbolRegistry,
    token, Lexer, ParseError, Parser,
};

// Re-export kernel for extraction
pub use logicaffeine_kernel as kernel;

// Module loading
pub mod loader;
// The stdlib's text and the index the build script derives from it
pub(crate) mod stdlib_text;
pub use loader::{Loader, ModuleSource};

// Compile-time analysis
pub mod analysis;

// Concurrency determinacy model + classifier.
// Pure AST analysis — independent of the codegen feature.
pub mod concurrency;

// Code generation
#[cfg(feature = "codegen")]
pub mod codegen;

// Counted-loop recognition shared by the for-range peephole and the unroller.
// Gated with codegen because it reuses the peephole's pure-AST guard helpers;
// the only consumer is the AOT (Rust-emitting) pipeline.
#[cfg(feature = "codegen")]
pub(crate) mod loop_shape;

// C code generation (benchmark-only subset)
#[cfg(feature = "codegen")]
pub mod codegen_c;

// LOGOS → RISC-V (RV64GC) AOT codegen tier
pub mod codegen_riscv;

// The ops no single-device AOT backend can lower (concurrency/networking),
// classified once so the Tenstorrent and CUDA tiers cannot drift.
#[cfg(feature = "codegen")]
pub mod aot_unsupported;

// LOGOS → Tenstorrent (TT-Metalium) AOT codegen tier — single Tensix baby-core
// kernel + host, reusing the C compute body (needs the `codegen` C backend).
#[cfg(feature = "codegen")]
pub mod codegen_tt;

// LOGOS → CUDA AOT codegen tier — one GPU kernel running a whole LOGOS program
// (integer/bool scalar fragment), lifting the C tier's program body onto the device.
#[cfg(feature = "codegen")]
pub mod codegen_cuda;

// The CPU tier for the one IR: a shim answering the shared optimizer for each CPU instruction set, and its link.
#[cfg(feature = "codegen")]
pub mod codegen_cpu;

// The SPATIAL IR — one stage/buffer/handshake/placement program that EVERY spatial backend
// (our fabric, the RDU, Tensix, a persistent GPU kernel) lowers FROM. Passes are target-parametric;
// a lowering may only read the IR + its target descriptor, never invent choreography.
#[cfg(feature = "codegen")]
pub mod spatial;
#[cfg(feature = "codegen")]
pub mod model;
// Model bring-up: a checkpoint's one program lowered by every chip that runs it (CUDA, the D5005, the SN50), each
// artifact naming the cut of the program it was lowered from.
#[cfg(feature = "codegen")]
pub mod bringup;
// Tensor programs as graphs of ops, and the transforms over them: reverse-mode differentiation.
#[cfg(feature = "codegen")]
pub mod tensor_graph;
/// Backend-neutral tensor-op recognition (LOGOS AST → typed specs), shared by every codegen backend.
pub mod tensor_recognize;

// Our own sm_86 SASS assembler foundation (read/decode/patch machine code — the MaxAs lever)
#[cfg(feature = "codegen")]
pub mod codegen_sass;

// LOGOS → analog (float dataflow) codegen tier — fail-closed refusal gate + artifact.
#[cfg(feature = "codegen")]
pub mod codegen_ana;

// LOGOS → DRAM (ComputeDRAM bulk bitwise + RowClone) compute-in-memory co-processor tier —
// op-subset descriptor + fail-closed refusal gate, reusing the codegen_sva::dram primitives.
#[cfg(feature = "codegen")]
pub mod codegen_dram;

// SVA/PSL code generation for hardware verification
pub mod codegen_sva;

// The mixed-signal XPU composition: the digital wprog engine + the analog plane, with
// certificate-gated region dispatch and digital fallback always compiled.
#[cfg(feature = "codegen")]
pub mod codegen_mx;

// The accelerator backends' shared host/region framework: region discovery, the device lanes at their real
// widths (a multi-word integer core and an exact binary64 softfloat), and the executable device models.
#[cfg(feature = "codegen")]
pub mod accel;

// Native crypto acceleration for the interpreted tiers (byte-identical to the
// pure-LOGOS reference in assets/std/crypto.lg).
pub mod crypto_accel;

// Compilation pipeline
pub mod compile;
pub use compile::{
    classify_source, compile_program_full, compile_program_full_deterministic,
    compile_program_full_with_proven, compile_to_rust, compile_to_rust_deterministic,
    compile_to_rust_with_proven, first_parallel_block_independent, run_strict_tw,
    run_strict_tw_with_args, run_strict_vm, run_strict_vm_with_args, send_check_source,
    CompileOutput, CrateDependency, EngineId, EngineOutcome,
};

// Diagnostics
pub mod diagnostic;

// Source mapping
pub mod sourcemap;

// Extraction (proof term extraction)
pub mod extraction;

// Optimization passes
pub mod optimize;

// Interpreter
pub mod interpreter;

// Tail-call recognition shared by all three execution tiers (tree-walker, VM,
// AOT) so "a self-tail-call runs in constant stack" is one definition, not three.
pub(crate) mod tail_call;

// An exhaustive rebuilding traversal of the imperative AST: every statement and expression variant
// is named, so a rewriting pass built on it cannot silently skip one.
pub(crate) use logicaffeine_language::ast_map;

// Monomorphization for the tiers with no generics of their own: one concrete function per
// (generic function, type arguments) a program uses, one registered type per applied generic.
pub mod monomorphize;

// Program bindings on the ahead-of-time tiers: the root bindings functions reach become the fields
// of one record every such function takes as a `mutable` parameter.
pub(crate) mod core_ir_scope;
pub mod program_bindings;

// Type-directed division resolution: rewrites `Divide → ExactDivide` where a `/`'s
// result flows into a `Rational` context (the default stays floor — zero breakage).
pub(crate) mod resolve_division;
pub(crate) mod resolve_pop;
pub(crate) mod resolve_program;
pub(crate) mod supertask;

// Observation elision for the compiled tiers: strips Note/Warn/Record and unwraps
// During spans BEFORE the AOT optimizer/codegen, so observation can never perturb
// a compiled artifact (§0.7 — locked by obs_zero_cost_codegen_lock).
pub(crate) mod elide_observation;

// Shared semantics kernel — ONE implementation of value semantics used by the
// tree-walker, the bytecode VM, and (later) the JIT slow paths.
pub mod semantics;

// Register bytecode VM (fast portable interpreter — WASM engine + JIT substrate).
pub mod vm;

// The shared browser host for LOGOS→WASM DOM apps (CLI `emit`/`serve` + the web
// Studio's live preview): the DOM/signal runtime, the default theme + favicon, the
// build-error overlay, and the in-browser preview assembler.
pub mod web_host;

// The Studio bytecode debugger (step / breakpoints / time-travel). Zero production
// cost — nothing here is on the execution path.
pub mod debug;

// UI Bridge - high-level compilation for web interface
pub mod ui_bridge;

// The `.replay` failure artifact — every failing run is a replayable execution.
pub mod replay;

// The `largo test` runner core — discovery, fresh-world execution, deterministic
// reports. `MemVfs` (the sealed test
// world) carries both `Vfs` trait shapes, so the runner compiles for the browser too
// — the in-page test runner behind the guide's exercise grader.
pub mod testrun;

// One driver, several page tiers: the statements a `## Test` uses to work a page, carried out
// on whichever tier executes the page (the tree-walker's own runtime, the lowered page on the
// bytecode VM, a browser), and the per-statement traces that hold the tiers to one another.
pub mod page_host;

// The observation channel's emission buffer — Note/Warn/Record events flow to
// sinks here, separate from program output.
pub mod observe;

// FrameEval — evaluate a LOGOS expression against a paused frame's bindings,
// the one evaluator behind every debug surface (D4.2, Wave 10).
pub mod frame_eval;

// The DebugSession facade — the single DAP-vocabulary seam the `largo dap`
// server and the Studio drawer both sit on, so they can never diverge.
pub mod debug_session;

// The replay-based interactive session behind `largo repl`.
pub mod repl;
pub use repl::{ReplOutcome, ReplSession};

#[cfg(feature = "verification")]
pub mod defeasible;

// Verification pass (Z3-based, requires verification feature)
#[cfg(feature = "verification")]
pub mod verification;
#[cfg(feature = "verification")]
pub use verification::VerificationPass;

// Re-export UI types at crate root for convenience
pub use ui_bridge::{
    answer_question, compile_for_proof, compile_for_ui, compile_theorem_for_ui,
    extract_logic_module, extract_logic_rust, extract_math_module, extract_math_module_from_source,
    extract_math_rust, extract_math_rust_from_source, grounded_grid_problem, interpret_for_ui,
    interpret_for_ui_baseline, interpret_for_ui_baseline_sync_with_args,
    interpret_for_ui_baseline_with_args, interpret_for_ui_sync, interpret_for_ui_sync_with_args,
    interpret_for_ui_with_args, interpret_streaming, interpret_streaming_with_vfs,
    interpret_streaming_with_vfs_observer, logic_compile_note, parse_math_statements,
    partition_mixed, prove_theorem_trace, run_treewalker_concurrent_seeded, run_vm_concurrent,
    run_vm_concurrent_seeded, run_vm_concurrent_with_config, run_vm_io_async, run_vm_net_async,
    solve_grid, theorem_certificate, theorem_dependency_graph, theorem_proof_exprs, verify_theorem,
    AstNode, CompileResult, GridColumn, ObserverCallback, ProofCompileResult, SolvedGrid,
    TheoremCompileResult, TheoremTrace, TokenCategory, TokenInfo,
};

// The work-stealing M:N driver is native-only (no OS threads on wasm32).
#[cfg(not(target_arch = "wasm32"))]
pub use ui_bridge::run_vm_workstealing_seeded;
#[cfg(feature = "verification")]
pub use ui_bridge::{
    check_theorem_defeasible, check_theorem_defeasible_consistent,
    check_theorem_premises_consistent, check_theorem_smt,
};
#[cfg(feature = "codegen")]
pub use ui_bridge::{generate_rust_code, generate_rust_code_with_proven};

// Provide module aliases for internal code
pub mod intern {
    pub use logicaffeine_base::{Interner, Symbol, SymbolEq};
}

pub mod arena {
    pub use logicaffeine_base::Arena;
}

pub mod arena_ctx {
    pub use logicaffeine_language::arena_ctx::*;
}

pub mod registry {
    pub use logicaffeine_language::registry::*;
}

pub mod style {
    pub use logicaffeine_language::style::*;
}
