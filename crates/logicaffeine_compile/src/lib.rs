#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

// Re-export base types
pub use logicaffeine_base::{Arena, Interner, Symbol, SymbolEq};

// Re-export language types needed for compilation
pub use logicaffeine_language::{
    ast, drs, error, lexer, optimization, parser, token,
    analysis::{TypeRegistry, DiscoveryPass, PolicyRegistry, PolicyCondition},
    arena_ctx::AstContext,
    registry::SymbolRegistry,
    formatter,
    mwe,
    Lexer, Parser, ParseError,
};

// Re-export kernel for extraction
pub use logicaffeine_kernel as kernel;

// Module loading
pub mod loader;
pub use loader::{Loader, ModuleSource};

// Compile-time analysis
pub mod analysis;

// Concurrency determinacy model + classifier (Phase 0 of work/FINISH_INTERPRETER.md).
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

// SVA/PSL code generation for hardware verification
pub mod codegen_sva;

// Compilation pipeline
pub mod compile;
pub use compile::{CompileOutput, CrateDependency, classify_source, compile_program_full, compile_program_full_deterministic, compile_program_full_with_proven, compile_to_rust, compile_to_rust_deterministic, compile_to_rust_with_proven, first_parallel_block_independent, send_check_source};

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

// Type-directed division resolution: rewrites `Divide → ExactDivide` where a `/`'s
// result flows into a `Rational` context (the default stays floor — zero breakage).
pub(crate) mod resolve_division;

// Shared semantics kernel — ONE implementation of value semantics used by the
// tree-walker, the bytecode VM, and (later) the JIT slow paths.
pub mod semantics;

// Register bytecode VM (fast portable interpreter — WASM engine + JIT substrate).
pub mod vm;

// The Studio bytecode debugger (step / breakpoints / time-travel). Zero production
// cost — nothing here is on the execution path.
pub mod debug;

// UI Bridge - high-level compilation for web interface
pub mod ui_bridge;

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
    answer_question, compile_for_ui, compile_for_proof, compile_theorem_for_ui,
    grounded_grid_problem, solve_grid, SolvedGrid, GridColumn,
    prove_theorem_trace, theorem_proof_exprs, theorem_dependency_graph, verify_theorem, TheoremTrace,
    interpret_for_ui, interpret_for_ui_with_args, interpret_for_ui_sync,
    interpret_for_ui_sync_with_args, interpret_for_ui_baseline,
    interpret_for_ui_baseline_with_args, interpret_for_ui_baseline_sync_with_args,
    interpret_streaming, interpret_streaming_with_vfs, interpret_streaming_with_vfs_observer,
    ObserverCallback, run_vm_concurrent, run_vm_net_async,
    run_vm_concurrent_seeded, run_treewalker_concurrent_seeded,
    CompileResult, ProofCompileResult,
    TheoremCompileResult, AstNode, TokenInfo, TokenCategory,
    extract_math_rust, extract_math_rust_from_source, extract_logic_rust, parse_math_statements,
    extract_math_module, extract_math_module_from_source, extract_logic_module, partition_mixed,
};

// The work-stealing M:N driver is native-only (no OS threads on wasm32).
#[cfg(not(target_arch = "wasm32"))]
pub use ui_bridge::run_vm_workstealing_seeded;
#[cfg(feature = "codegen")]
pub use ui_bridge::{generate_rust_code, generate_rust_code_with_proven};
#[cfg(feature = "verification")]
pub use ui_bridge::{
    check_theorem_defeasible, check_theorem_defeasible_consistent,
    check_theorem_premises_consistent, check_theorem_smt,
};

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
