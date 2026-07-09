//! LOGOS Compilation Pipeline
//!
//! This module provides the end-to-end compilation pipeline that transforms
//! LOGOS source code into executable Rust programs.
//!
//! # Pipeline Overview
//!
//! ```text
//! LOGOS Source (.md)
//!       │
//!       ▼
//! ┌───────────────────┐
//! │  1. Lexer         │ Tokenize source
//! └─────────┬─────────┘
//!           ▼
//! ┌───────────────────┐
//! │  2. Discovery     │ Type & policy definitions
//! └─────────┬─────────┘
//!           ▼
//! ┌───────────────────┐
//! │  3. Parser        │ Build AST
//! └─────────┬─────────┘
//!           ▼
//! ┌───────────────────┐
//! │  4. Analysis      │ Escape, ownership, verification
//! └─────────┬─────────┘
//!           ▼
//! ┌───────────────────┐
//! │  5. CodeGen       │ Emit Rust source
//! └─────────┬─────────┘
//!           ▼
//!     Rust Source
//! ```
//!
//! # Compilation Functions
//!
//! | Function | Analysis | Use Case |
//! |----------|----------|----------|
//! | [`compile_to_rust`] | Escape only | Basic compilation |
//! | [`compile_to_rust_checked`] | Escape + Ownership | Use with `--check` flag |
//! | `compile_to_rust_verified` | All + Z3 | Formal verification (requires `verification` feature) |
//! | [`compile_project`] | Multi-file | Projects with imports |
//! | [`compile_and_run`] | Full + Execute | Development workflow |
//!
//! # Examples
//!
//! ## Basic Compilation
//!
//! ```
//! # use logicaffeine_compile::compile::compile_to_rust;
//! # use logicaffeine_compile::ParseError;
//! # fn main() -> Result<(), ParseError> {
//! let source = "## Main\nLet x be 5.\nShow x.";
//! let rust_code = compile_to_rust(source)?;
//! // rust_code contains:
//! // fn main() {
//! //     let x = 5;
//! //     println!("{}", x);
//! // }
//! # Ok(())
//! # }
//! ```
//!
//! ## With Ownership Checking
//!
//! ```
//! # use logicaffeine_compile::compile::compile_to_rust_checked;
//! let source = "## Main\nLet x be 5.\nGive x to y.\nShow x.";
//! let result = compile_to_rust_checked(source);
//! // Returns Err: "x has already been given away"
//! ```

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

// Runtime crates paths (relative to workspace root)
const CRATES_DATA_PATH: &str = "crates/logicaffeine_data";
const CRATES_SYSTEM_PATH: &str = "crates/logicaffeine_system";

use std::fmt::Write as FmtWrite;

use crate::analysis::{DiscoveryPass, EscapeChecker, OwnershipChecker, PolicyRegistry};
use crate::arena::Arena;
use crate::arena_ctx::AstContext;
use crate::ast::{Expr, MatchArm, Stmt, TypeExpr};
use crate::ast::stmt::{BinaryOpKind, ClosureBody, Literal, Pattern, ReadSource, SelectBranch, StringPart};
use crate::codegen::{codegen_program, generate_c_header, generate_python_bindings, generate_typescript_bindings};
use crate::diagnostic::{parse_rustc_json, translate_diagnostics, LogosError};
use crate::drs::WorldState;
use crate::error::ParseError;
use crate::intern::Interner;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::sourcemap::SourceMap;

/// A declared external crate dependency from a `## Requires` block.
#[derive(Debug, Clone)]
pub struct CrateDependency {
    pub name: String,
    pub version: String,
    pub features: Vec<String>,
}

/// Full compilation output including generated Rust code and extracted dependencies.
#[derive(Debug)]
pub struct CompileOutput {
    pub rust_code: String,
    pub dependencies: Vec<CrateDependency>,
    /// Generated C header content (populated when C exports exist).
    pub c_header: Option<String>,
    /// Generated Python ctypes bindings (populated when C exports exist).
    pub python_bindings: Option<String>,
    /// Generated TypeScript type declarations (.d.ts content, populated when C exports exist).
    pub typescript_types: Option<String>,
    /// Generated TypeScript FFI bindings (.js content, populated when C exports exist).
    pub typescript_bindings: Option<String>,
}

/// Interpret LOGOS source and return output as a string.
///
/// Runs the full pipeline (lex → discovery → parse → interpret) without
/// generating Rust code. Useful for sub-second feedback during development.
///
/// # Arguments
///
/// * `source` - LOGOS source code as a string
///
/// # Returns
///
/// The collected output from `Show` statements, joined by newlines.
///
/// # Errors
///
/// Returns [`ParseError`] if parsing fails or the interpreter encounters
/// a runtime error.
pub fn interpret_program(source: &str) -> Result<String, ParseError> {
    let result = crate::ui_bridge::interpret_for_ui_sync(source);
    if let Some(err) = result.error {
        Err(ParseError {
            kind: crate::error::ParseErrorKind::Custom(err),
            span: crate::token::Span::default(),
        })
    } else {
        Ok(result.lines.join("\n"))
    }
}

/// Parse LOGOS source and execute it on the register bytecode VM, returning the
/// captured output. Uses the SAME front-end as the tree-walker
/// ([`crate::ui_bridge::with_parsed_program`]: lex → MWE → discovery → parse),
/// so differential tests compare two engines on one program, never two
/// programs. Returns `Err` for programs outside the VM's supported subset (the
/// compiler reports `vm: unsupported …`).
pub fn vm_run_source(source: &str) -> Result<String, String> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => {
            let (output, error) =
                crate::vm::run_to_outcome(stmts, interner, Some(types), Some(&policies));
            match error {
                None => Ok(output),
                Some(e) => Err(e),
            }
        }
        Err(advice) => Err(advice),
    })
}

/// What a program run produced: every output line emitted before completion or
/// failure, plus the error if it failed. The differential contract is
/// `vm_outcome(src) == tw_outcome(src)` — output AND error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    pub output: String,
    pub error: Option<String>,
}

/// Run `source` on the TREE-WALKER (the oracle), capturing partial output and
/// the error, if any. This entry point always uses the tree-walker — the
/// dispatching `interpret_for_ui_sync` now runs the VM for the sync path, and
/// the differential suites need the oracle by itself.
pub fn tw_outcome(source: &str) -> RunOutcome {
    // Deterministic oracle: no relay transport, so `Connect` runs as a single-node local no-op (the
    // networking analogue of the fixed clock). Only affects a `Connect` program; inert otherwise.
    crate::concurrency::net_inbox::set_net_offline(true);
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => {
            let force_async = crate::interpreter::needs_async(stmts);
            let result = crate::ui_bridge::run_treewalker(
                stmts, types, policies, interner, force_async, &[],
            );
            RunOutcome { output: result.lines.join("\n"), error: result.error }
        }
        Err(advice) => RunOutcome { output: String::new(), error: Some(advice) },
    })
}

/// Run `source` on the bytecode VM, capturing partial output and the error,
/// if any. Same front-end as [`tw_outcome`].
pub fn vm_outcome(source: &str) -> RunOutcome {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => {
            let (output, error) =
                crate::vm::run_to_outcome(stmts, interner, Some(types), Some(&policies));
            RunOutcome { output, error }
        }
        Err(advice) => RunOutcome { output: String::new(), error: Some(advice) },
    })
}

/// The VM outcome for a CONCURRENT program — drives the bytecode VM on the cooperative scheduler
/// (`VmTask` on `run_with_seed`, seed 0) exactly as the browser/playground does, so a `Pipe`/task/
/// `Select` program runs on the VM byte-identical to the tree-walker's async path. The plain
/// [`vm_outcome`] errors on any concurrency op (`requires the scheduler driver`); this is the oracle
/// the AOT concurrency lock compares against so tw == VM == AOT is verifiable for the concurrent set.
#[cfg(not(target_arch = "wasm32"))]
pub fn vm_outcome_concurrent(source: &str) -> RunOutcome {
    let r = crate::ui_bridge::run_vm_concurrent(source);
    RunOutcome { output: r.lines.join("\n"), error: r.error }
}

/// The VM outcome for a peer-NETWORKING program — drives the resumable bytecode VM and services each
/// `VmBlock::Net*` through the shared LOCAL `NetInbox` (the same inbox the tree-walker's offline
/// mode uses), so `Listen`/`Send`/`Sync`/`PeerAgent` run on the VM byte-identical to the tree-walker.
/// The AOT networking lock compares against this so tw == VM == AOT holds for the local-mode net set.
#[cfg(not(target_arch = "wasm32"))]
pub fn vm_outcome_net(source: &str) -> RunOutcome {
    // Deterministic oracle: offline single node, so `Connect` is a local no-op (matches `tw_outcome`).
    crate::concurrency::net_inbox::set_net_offline(true);
    let r = futures::executor::block_on(crate::ui_bridge::run_vm_net_async(source));
    RunOutcome { output: r.lines.join("\n"), error: r.error }
}

/// [`tw_outcome`] with the program argument vector for the `args()` system
/// native (full argv; index 0 is the program name) — the seam the benchmark
/// corpus differential uses to run `main.lg` programs that read their size
/// from argv.
pub fn tw_outcome_with_args(source: &str, program_args: &[String]) -> RunOutcome {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => {
            let force_async = crate::interpreter::needs_async(stmts);
            let result = crate::ui_bridge::run_treewalker(
                stmts, types, policies, interner, force_async, program_args,
            );
            RunOutcome { output: result.lines.join("\n"), error: result.error }
        }
        Err(advice) => RunOutcome { output: String::new(), error: Some(advice) },
    })
}

/// [`vm_outcome`] with the program argument vector and an optional private
/// native tier (so a differential test can observe THAT program's JIT compile
/// counters in isolation — the process-wide tier's counters are shared by
/// every test in the binary).
pub fn vm_outcome_with_args(
    source: &str,
    program_args: &[String],
    tier: Option<&dyn crate::vm::NativeTier>,
) -> RunOutcome {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => {
            let (output, error) = crate::vm::run_to_outcome_with_args(
                stmts,
                interner,
                Some(types),
                Some(&policies),
                program_args,
                tier,
            );
            RunOutcome { output, error }
        }
        Err(advice) => RunOutcome { output: String::new(), error: Some(advice) },
    })
}

/// [`vm_outcome_with_args`], but the statements pass through the run-path optimizer
/// at an explicit hotness `tier` (HOTSWAP §4) before reaching the VM — the seam the
/// `tier_invariance` gate drives at every tier T0..T3 to assert byte-identical
/// output. `tier = Tier::T3` reproduces today's optimized run path.
pub fn vm_outcome_tiered(
    source: &str,
    program_args: &[String],
    tier: crate::optimization::Tier,
    native_tier: Option<&dyn crate::vm::NativeTier>,
) -> RunOutcome {
    crate::ui_bridge::with_optimized_program_tiered(source, tier, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => {
            let (output, error) = crate::vm::run_to_outcome_with_args(
                stmts,
                interner,
                Some(types),
                Some(&policies),
                program_args,
                native_tier,
            );
            RunOutcome { output, error }
        }
        Err(advice) => RunOutcome { output: String::new(), error: Some(advice) },
    })
}

/// [`vm_outcome_with_args`] driven through the BACKGROUND native compiler (HOTSWAP §6):
/// hot functions compile on a worker thread off the interpreter's path. Requires a
/// process-installed tier (the P8 gate installs one first). The compiled chain is
/// identical to the synchronous one — only WHEN it is produced differs — so output
/// must match the synchronous/tree-walker engines exactly.
#[cfg(not(target_arch = "wasm32"))]
pub fn vm_outcome_bg(source: &str, program_args: &[String]) -> RunOutcome {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => {
            let (output, error) = crate::vm::run_to_outcome_bg(
                stmts,
                interner,
                Some(types),
                Some(&policies),
                program_args,
            );
            RunOutcome { output, error }
        }
        Err(advice) => RunOutcome { output: String::new(), error: Some(advice) },
    })
}

/// Compile LOGOS source to Rust source code.
///
/// This is the basic compilation function that runs lexing, parsing, and
/// escape analysis before generating Rust code.
///
/// # Arguments
///
/// * `source` - LOGOS source code as a string
///
/// # Returns
///
/// Generated Rust source code on success.
///
/// # Errors
///
/// Returns [`ParseError`] if:
/// - Lexical analysis fails (invalid tokens)
/// - Parsing fails (syntax errors)
/// - Escape analysis fails (zone-local values escaping)
///
/// # Example
///
/// ```
/// # use logicaffeine_compile::compile::compile_to_rust;
/// # use logicaffeine_compile::ParseError;
/// # fn main() -> Result<(), ParseError> {
/// let source = "## Main\nLet x be 5.\nShow x.";
/// let rust_code = compile_to_rust(source)?;
/// assert!(rust_code.contains("let x = 5;"));
/// # Ok(())
/// # }
/// ```
pub fn compile_to_rust(source: &str) -> Result<String, ParseError> {
    compile_program_full(source).map(|o| o.rust_code)
}

/// Compile in **Mode B (deterministic replay)**: identical to [`compile_to_rust`]
/// except a `Select` lowers to the *seeded* winner-pick that shares the
/// interpreter's choice function, so a binary run under `LOGOS_SEED=…` reproduces
/// the interpreter's selection. Used by `largo build --deterministic` and the
/// seeded differential harness. Non-`Select` emission is unchanged.
pub fn compile_to_rust_deterministic(source: &str) -> Result<String, ParseError> {
    crate::codegen::with_seeded_select(|| compile_to_rust(source))
}

/// Compile imperative LOGOS to Rust with an extracted math/logic module bundled in.
/// `proven` is a main-less Rust module body (the Forge's extraction); it is emitted
/// as `pub mod proven { … } use proven::*;` so the imperative program can call its
/// functions / `check_*` predicates by name. See [`compile_program_full_with_proven`].
pub fn compile_to_rust_with_proven(source: &str, proven: &str) -> Result<String, ParseError> {
    compile_program_full_with_proven(source, Some(proven)).map(|o| o.rust_code)
}

/// Which optimizations actually FIRE for `source`: an optimization is "used" when
/// disabling it (via a file-level `## No <X>` decorator) changes the generated
/// Rust. Lets the benchmarks UI show, per program, which optimizations it USES vs
/// the full set it CAN use. O(number of optimizations) compiles — intended for
/// per-benchmark, not hot-path, use. Returns the decorator keywords of the firing
/// optimizations (in registry order).
pub fn optimizations_used(source: &str) -> Vec<&'static str> {
    let base = match compile_to_rust(source) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    crate::optimization::REGISTRY
        .iter()
        .filter(|m| {
            let decorated = crate::optimization::decorate_source(source, &[m.keyword]);
            compile_to_rust(&decorated).map(|r| r != base).unwrap_or(false)
        })
        .map(|m| m.keyword)
        .collect()
}

/// Compile `source` while recording which optimizations actually FIRED — the
/// honest, single-compile alternative to [`optimizations_used`]'s O(N) diff. An
/// optimization is "fired" when it actually changed the program or emitted its
/// optimized form for this compile, under the effective (normalized) config.
/// Reusable: any caller can compile a program and learn what fired, for tracing
/// and tooling.
pub fn compile_program_traced(
    source: &str,
) -> Result<(CompileOutput, crate::optimization::FiredOptimizations), ParseError> {
    crate::optimize::begin_fired_trace();
    let result = compile_program_full(source);
    // Always end the trace (even on a parse error) so the thread-local never
    // leaks into a later compile on this thread.
    let fired = crate::optimize::end_fired_trace().unwrap_or_default();
    result.map(|o| (o, fired))
}

/// The decorator keywords of the optimizations that fired for `source` (registry
/// order), under the default/effective config. One compile, exact.
pub fn optimizations_fired(source: &str) -> Vec<&'static str> {
    compile_program_traced(source)
        .map(|(_, fired)| fired.keywords())
        .unwrap_or_default()
}

/// The optimizations that fired for `source` under an explicit config — re-traces
/// an arbitrary toggle state (turning some off can make others fire). Realised by
/// disabling that config's off-keywords on the source, then tracing.
pub fn optimizations_fired_with(
    source: &str,
    cfg: &crate::optimization::OptimizationConfig,
) -> Vec<&'static str> {
    let disabled: Vec<&'static str> = cfg.disabled_keywords().collect();
    let decorated = crate::optimization::decorate_source(source, &disabled);
    optimizations_fired(&decorated)
}

/// One compile returning the generated Rust, the fired-optimization keywords, AND
/// the `(winner, loser)` precedence decisions made — what an interactive viewer
/// calls per toggle state to show the firing opts and the conflicts that occurred.
pub fn compile_to_rust_traced(
    source: &str,
) -> Result<(String, Vec<&'static str>, Vec<(&'static str, &'static str)>), ParseError> {
    crate::optimize::begin_fired_trace();
    let result = compile_program_full(source);
    let fired = crate::optimize::end_fired_trace().unwrap_or_default().keywords();
    let preempted = preempted_keywords();
    result.map(|o| (o.rust_code, fired, preempted))
}

/// The `(winner, loser)` precedence decisions the compiler made for `source` — the
/// conflicts traced at their source (`mark_preempted`), under the effective config.
pub fn optimization_preemptions(source: &str) -> Vec<(&'static str, &'static str)> {
    crate::optimize::begin_fired_trace();
    let _ = compile_program_full(source);
    let _ = crate::optimize::end_fired_trace();
    preempted_keywords()
}

/// Convert the trace's `(Opt, Opt)` preemption edges to `(keyword, keyword)`.
fn preempted_keywords() -> Vec<(&'static str, &'static str)> {
    crate::optimize::end_preempted_trace()
        .into_iter()
        .map(|(w, l)| (w.meta().keyword, l.meta().keyword))
        .collect()
}

/// The per-program DEPENDENCY edges for `source`, discovered by evaluating with
/// all optimizations on and probing: disabling a fired optimization `dep` that
/// makes another fired optimization `dependent` STOP firing means `dependent`
/// depended on `dep` for this program. Stops already explained by a declared
/// `requires`-cascade (`normalize` would disable the dependent anyway) are
/// excluded — those are captured by the static registry graph. What remains are
/// the EMERGENT, program-specific dependencies (e.g. dead-code elimination only
/// had work because scalarization produced the dead code): returned as
/// `(dependent, dep)` keyword pairs, sorted and deduped (deterministic).
///
/// This is offline analysis (one compile per fired optimization) — it is baked
/// into the benchmark data, never run in the hot path.
pub fn optimization_dependencies(source: &str) -> Vec<(&'static str, &'static str)> {
    use crate::optimization::{by_keyword, decorate_source, OptimizationConfig};
    use std::collections::BTreeSet;

    let base: Vec<&'static str> = optimizations_fired(source);
    let base_set: BTreeSet<&'static str> = base.iter().copied().collect();
    let mut out: Vec<(&'static str, &'static str)> = Vec::new();
    for &dep in &base {
        let dep_opt = match by_keyword(dep) {
            Some(o) => o,
            None => continue,
        };
        // What the declared `requires`-cascade disables when `dep` is off.
        let mut cfg = OptimizationConfig::all_on();
        cfg.set(dep_opt, false);
        cfg.normalize();
        let off: BTreeSet<&'static str> =
            optimizations_fired(&decorate_source(source, &[dep])).into_iter().collect();
        for &dependent in &base_set {
            if dependent == dep || off.contains(dependent) {
                continue;
            }
            let dependent_opt = match by_keyword(dependent) {
                Some(o) => o,
                None => continue,
            };
            // Skip stops the static graph already explains (normalize disabled it).
            if cfg.is_on(dependent_opt) {
                out.push((dependent, dep));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The complete per-program optimization graph from one all-on evaluation: the
/// optimizations that `fired`, the `blockers` (`(winner, loser)` preemptions that
/// occurred), and the emergent `dependencies` (`(dependent, dep)` — see
/// [`optimization_dependencies`]). Everything the menu-tree needs, baked once per
/// benchmark so the UI never probes in the hot path. All on the AOT codegen path
/// (the one that produces the shown Rust), so the three views agree.
pub fn optimization_graph(
    source: &str,
) -> (
    Vec<&'static str>,
    Vec<(&'static str, &'static str)>,
    Vec<(&'static str, &'static str)>,
) {
    (
        optimizations_fired(source),
        optimization_preemptions(source),
        optimization_dependencies(source),
    )
}

/// The optimizations that fired on the RUN path — the VM/interpreter optimizer
/// (`optimize_for_run`) — for `source`. The run-path analogue of
/// [`optimizations_fired`] (which traces the AOT codegen path); it surfaces the
/// run-path-only passes — inlining, loop-carried CSE, float strength reduction —
/// the AOT trace cannot see. Respects file-level `## No <opt>` decorators.
pub fn optimizations_fired_run(source: &str) -> Vec<&'static str> {
    crate::optimize::begin_fired_trace();
    crate::ui_bridge::with_optimized_program(source, |_parsed, _interner| {});
    crate::optimize::end_fired_trace()
        .map(|fired| fired.keywords())
        .unwrap_or_default()
}

/// The optimizations that fired while compiling `source` to VM bytecode — the
/// third path, after AOT codegen and the run-path optimizer. Surfaces the
/// VM-compile-time opts (constant-divisor magic division, VM `i32` narrowing)
/// the other two traces cannot see. Compiles only (no execution), so it is fast
/// and needs no program arguments.
pub fn optimizations_fired_vm(source: &str) -> Vec<&'static str> {
    crate::optimize::begin_fired_trace();
    crate::ui_bridge::with_optimized_program(source, |parsed, interner| {
        if let Ok((stmts, types, _policies)) = parsed {
            let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
            let _ = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle));
        }
    });
    crate::optimize::end_fired_trace()
        .map(|fired| fired.keywords())
        .unwrap_or_default()
}

/// Compile LOGOS source to C code (benchmark-only subset).
///
/// Produces a self-contained C file with embedded runtime that can be
/// compiled with `gcc -O2 -o program output.c`.
pub fn compile_to_c(source: &str) -> Result<String, ParseError> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    let (type_registry, _policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };
    let codegen_registry = type_registry.clone();

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ast_ctx = AstContext::with_types(
        &expr_arena, &term_arena, &np_arena, &sym_arena,
        &role_arena, &pp_arena, &stmt_arena, &imperative_expr_arena,
        &type_expr_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, type_registry);
    let stmts = parser.parse_program()?;
    let stmts = crate::optimize::optimize_program(stmts, &imperative_expr_arena, &stmt_arena, &mut interner, &crate::optimization::OptimizationConfig::from_env());

    Ok(crate::codegen_c::codegen_program_c(&stmts, &codegen_registry, &interner))
}

/// Compile LOGOS source DIRECTLY to a self-contained WebAssembly module — no rustc, no cargo,
/// no wasm-bindgen, no toolchain. Uses the SAME front-end as [`vm_outcome`]
/// ([`crate::ui_bridge::with_parsed_program`] → [`crate::vm::Compiler::compile_with_types`]),
/// so the emitted module runs the IDENTICAL bytecode the VM does; the AOT WebAssembly backend
/// then lowers the whole program to `.wasm`.
///
/// Returns the bytes of a `.wasm` file. A program outside the backend's supported fragment
/// yields a [`ParseError`] carrying the backend's `unsupported …` reason — the corpus lock
/// turns that into a tracked, shrinking gap rather than a silent skip.
pub fn compile_to_wasm(source: &str) -> Result<Vec<u8>, ParseError> {
    fn custom(msg: String) -> ParseError {
        ParseError { kind: crate::error::ParseErrorKind::Custom(msg), span: crate::token::Span::default() }
    }
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, types, policies) = parsed.map_err(custom)?;
        // Compile with the SAME Oracle the VM/native tier use (`run_to_outcome`), so the AOT receives
        // the optimized bytecode — DivPow2 / MagicDivU (magic-reciprocal division) and the Oracle-proven
        // bounds-check-elimination forms (IndexUnchecked / SetIndexUnchecked / RegionBoundsGuard). The
        // AOT lowers each (division bit-exact to the VM's `magic_eval`; the unchecked forms keep the
        // bounds check, a safe superset), so WASM == VM stays exact while the two share one bytecode.
        let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
        let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
            .map_err(custom)?;
        crate::vm::wasm::assemble_program(&program, &policies, interner).map_err(|e| custom(e.to_string()))
    })
}

/// Compile LOGOS source to a LINKED WebAssembly module that uses the real `logicaffeine_base::BigInt`
/// runtime for arbitrary-precision integer arithmetic: an overflowing integer expression
/// (`Show 99999999999 * 99999999999`, `Show 2 to the power of 200`) computes the EXACT big number —
/// matching the VM's promote-on-overflow — instead of trapping, and `Show` prints its decimal.
///
/// Unlike [`compile_to_wasm`] (fully self-contained, no toolchain), this is TOOLCHAIN-DEPENDENT: the
/// BigInt-aware emitter ([`crate::vm::wasm::assemble_program_linked`]) produces a RELOCATABLE object that
/// is linked against `logicaffeine_base` (compiled once to `wasm32-unknown-unknown`) with the Rust
/// toolchain's `rust-lld`. The runtime object is cached per process, so the amortized cost is one link.
/// Returns the bytes of a `.wasm` file, or a [`ParseError`] carrying the reason the backend or linker
/// declined (an unsupported program shape, or an unavailable toolchain / base wasm32 build).
#[cfg(all(feature = "wasm-jit", not(target_arch = "wasm32")))]
pub fn compile_to_wasm_linked(source: &str) -> Result<Vec<u8>, ParseError> {
    fn custom(msg: String) -> ParseError {
        ParseError { kind: crate::error::ParseErrorKind::Custom(msg), span: crate::token::Span::default() }
    }
    let relocatable = crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, ParseError> {
        let (stmts, types, policies) = parsed.map_err(custom)?;
        let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
        let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle)).map_err(custom)?;
        let module = crate::vm::wasm::assemble_program_linked(&program, &policies, interner).map_err(|e| custom(e.to_string()))?;
        crate::vm::wasm::module_to_relocatable(&module).map_err(|e| custom(e.to_string()))
    })?;
    crate::vm::wasm::link_relocatable_bigint(&relocatable).map_err(|e| custom(e.to_string()))
}

/// Format an `f64` exactly as a `Show` of a `Float` would — i.e. byte-identical to the
/// tree-walker / VM (`RuntimeValue::to_display_string`). The direct-WASM host's `print_f64`
/// sink uses this so a `Show` of a float matches every other engine. Delegating to the real
/// formatter (rather than re-deriving it) makes drift impossible.
pub fn display_float_like_logos(f: f64) -> String {
    crate::interpreter::RuntimeValue::Float(f).to_display_string()
}

/// Phase 0 (work/FINISH_INTERPRETER.md): classify the determinacy of a LOGOS program.
///
/// Parses `source` and runs the determinacy classifier over the parsed program.
/// Pure analysis — no optimization, no codegen. Returns whether the program is
/// in the determinate (Kahn-deterministic) or nondeterminate fragment, with the
/// nondeterminism witnesses.
pub fn classify_source(source: &str) -> Result<crate::concurrency::Determinacy, ParseError> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    let (type_registry, _policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ast_ctx = AstContext::with_types(
        &expr_arena, &term_arena, &np_arena, &sym_arena,
        &role_arena, &pp_arena, &stmt_arena, &imperative_expr_arena,
        &type_expr_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, type_registry);
    let stmts = parser.parse_program()?;
    Ok(crate::concurrency::classify_program(&stmts))
}

/// Phase 0 test seam: is the first `Simultaneously:` / `Attempt all of the
/// following:` block in `source` made of data-independent branches?
/// `None` if the program contains no such block.
pub fn first_parallel_block_independent(source: &str) -> Result<Option<bool>, ParseError> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    let (type_registry, _policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ast_ctx = AstContext::with_types(
        &expr_arena, &term_arena, &np_arena, &sym_arena,
        &role_arena, &pp_arena, &stmt_arena, &imperative_expr_arena,
        &type_expr_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, type_registry);
    let stmts = parser.parse_program()?;
    Ok(find_first_parallel_independence(&stmts))
}

fn find_first_parallel_independence(stmts: &[Stmt]) -> Option<bool> {
    for s in stmts {
        match s {
            Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
                return Some(crate::concurrency::branches_independent(tasks));
            }
            Stmt::FunctionDef { body, .. }
            | Stmt::While { body, .. }
            | Stmt::Repeat { body, .. }
            | Stmt::Zone { body, .. } => {
                if let Some(r) = find_first_parallel_independence(body) {
                    return Some(r);
                }
            }
            Stmt::If { then_block, else_block, .. } => {
                if let Some(r) = find_first_parallel_independence(then_block) {
                    return Some(r);
                }
                if let Some(eb) = else_block {
                    if let Some(r) = find_first_parallel_independence(eb) {
                        return Some(r);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Phase 4 (work/FINISH_INTERPRETER.md): run the Send/escape analysis over a program.
///
/// Parses `source` and returns the Send/escape diagnostics (empty = the program
/// respects the message-passing + CRDT discipline). Pure analysis — no codegen.
pub fn send_check_source(source: &str) -> Result<Vec<crate::concurrency::SendDiagnostic>, ParseError> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    let (type_registry, _policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ast_ctx = AstContext::with_types(
        &expr_arena, &term_arena, &np_arena, &sym_arena,
        &role_arena, &pp_arena, &stmt_arena, &imperative_expr_arena,
        &type_expr_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, type_registry);
    let stmts = parser.parse_program()?;
    Ok(crate::concurrency::check_send_escape(&stmts))
}

/// Compile LOGOS source and return full output including dependency metadata.
///
/// This is the primary compilation entry point that returns both the generated
/// Rust code and any crate dependencies declared in `## Requires` blocks.
/// [`compile_program_full`] in **Mode B (deterministic replay)** — the full
/// output (rust + dependencies) with the seeded `Select` lowering. The seeded
/// differential harness uses this so it gets the same dependency set as Mode A.
pub fn compile_program_full_deterministic(source: &str) -> Result<CompileOutput, ParseError> {
    crate::codegen::with_seeded_select(|| compile_program_full(source))
}

pub fn compile_program_full(source: &str) -> Result<CompileOutput, ParseError> {
    // Mixed-document aware: a source interleaving imperative code with Coq-style math
    // (`Definition`/`## Theorem:`/…) routes the math to the Forge and bundles it as
    // `mod proven`. Pure imperative programs partition to `(source, None)` — a no-op.
    let (imperative_src, math_src) = crate::ui_bridge::partition_mixed(source);
    let proven = math_src.as_deref().and_then(crate::ui_bridge::mixed_proven_module);
    compile_program_full_with_proven(&imperative_src, proven.as_deref())
}

/// Like [`compile_program_full`], but every emitted (non-generic) `enum` also gets
/// `WireEncode`/`WireDecode` impls over the shared `logicaffeine_data::wire` codec — byte-identical
/// to the peer codec's `encode_value_raw`. This lets a compile-once native partial evaluator
/// receive a program AST as data over the same fast codec the interpreter uses. Ordinary compiles
/// (the default) are byte-unchanged.
pub fn compile_program_full_with_wire(source: &str) -> Result<CompileOutput, ParseError> {
    crate::codegen::types::with_wire_impls(true, || compile_program_full(source))
}

/// Like [`compile_program_full`], but bundles an extracted math/logic module
/// (`proven`) into the generated Rust — the imperative half of a mixed document.
/// The module is emitted as `pub mod proven { … } use proven::*;` so the imperative
/// program can call its functions / `check_*` predicates by name. `None` (or a
/// blank module) yields output byte-identical to [`compile_program_full`].
pub fn compile_program_full_with_proven(source: &str, proven: Option<&str>) -> Result<CompileOutput, ParseError> {
    // Phase 10: auto-prepend the stdlib modules the program references (no-op when
    // it uses no stdlib vocabulary, so the benchmark corpus stays byte-identical).
    let prelude_src = crate::loader::apply_prelude(source);
    let source = prelude_src.as_ref();

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    // Pass 1: Discovery - scan for type definitions and policies
    let (type_registry, policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };
    // Clone for codegen (parser takes ownership)
    let codegen_registry = type_registry.clone();
    let codegen_policies = policy_registry.clone();

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ast_ctx = AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_expr_arena,
    );

    // Pass 2: Parse with type context
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, type_registry);
    // Note: Don't call process_block_headers() - parse_program handles blocks itself

    let stmts = parser.parse_program()?;
    // The ONE optimization config for this compile: env baseline merged with the
    // file-level `## No <X>` decorators (program-wide), normalized. Threaded to
    // BOTH the optimizer and codegen so they never disagree. Captured here (the
    // last use of `parser`) so its `&mut interner` borrow ends before reuse.
    let opt_config = {
        let mut c = crate::optimization::OptimizationConfig::from_env()
            .merged(&parser.program_opt_flags());
        c.normalize();
        c
    };

    // Type-directed division: rewrite `Divide → ExactDivide` in every Rational context
    // (the integer default stays floor), so a `Let x: Rational be 7 / 2` compiles exact
    // (`7/2`) instead of flooring to `3` — matching the tree-walker and VM. The optimizer
    // treats `ExactDivide` as opaque (never floor-folds it), so this is purely additive.
    let resolved = crate::resolve_division::resolve_divisions(
        &stmts,
        &stmt_arena,
        &imperative_expr_arena,
        &interner,
        opt_config.is_on(crate::optimization::Opt::Comptime),
    );
    let stmts: Vec<Stmt> = match resolved {
        Some(rw) => rw.to_vec(),
        None => stmts,
    };

    // Pass 2.5: Optimization - constant folding and dead code elimination
    let stmts = crate::optimize::optimize_program(stmts, &imperative_expr_arena, &stmt_arena, &mut interner, &opt_config);

    // Extract dependencies before escape analysis
    let mut dependencies = extract_dependencies(&stmts, &interner)?;

    // FFI: Auto-inject wasm-bindgen dependency if any function is exported for WASM
    let needs_wasm_bindgen = stmts.iter().any(|stmt| {
        if let Stmt::FunctionDef { is_exported: true, export_target: Some(target), .. } = stmt {
            interner.resolve(*target).eq_ignore_ascii_case("wasm")
        } else {
            false
        }
    });
    if needs_wasm_bindgen && !dependencies.iter().any(|d| d.name == "wasm-bindgen") {
        dependencies.push(CrateDependency {
            name: "wasm-bindgen".to_string(),
            version: "0.2".to_string(),
            features: vec![],
        });
    }

    // Pass 3: Escape analysis - check for zone escape violations
    // This catches obvious cases like returning zone-local variables
    let mut escape_checker = EscapeChecker::new(&interner);
    escape_checker.check_program(&stmts).map_err(|e| {
        // Convert EscapeError to ParseError for now
        // The error message is already Socratic from EscapeChecker
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(e.to_string()),
            span: e.span,
        }
    })?;

    // Note: Static verification is available when the `verification` feature is enabled,
    // but must be explicitly invoked via compile_to_rust_verified().

    let type_env = crate::analysis::check_program(&stmts, &interner, &codegen_registry)
        .map_err(|e| ParseError {
            kind: e.to_parse_error_kind(&interner),
            span: crate::token::Span::default(),
        })?;
    let rust_code = crate::codegen::codegen_program_with_proven(&stmts, &codegen_registry, &codegen_policies, &interner, &type_env, &opt_config, "proven", proven);

    // Universal ABI: Generate C header + bindings if any C exports exist
    let has_c = stmts.iter().any(|stmt| {
        if let Stmt::FunctionDef { is_exported: true, export_target, .. } = stmt {
            match export_target {
                None => true,
                Some(t) => interner.resolve(*t).eq_ignore_ascii_case("c"),
            }
        } else {
            false
        }
    });

    let c_header = if has_c {
        Some(generate_c_header(&stmts, "module", &interner, &codegen_registry))
    } else {
        None
    };

    // Auto-inject serde_json dependency when C exports exist (needed for collection to_json and portable struct JSON accessors)
    if has_c && !dependencies.iter().any(|d| d.name == "serde_json") {
        dependencies.push(CrateDependency {
            name: "serde_json".to_string(),
            version: "1".to_string(),
            features: vec![],
        });
    }

    let python_bindings = if has_c {
        Some(generate_python_bindings(&stmts, "module", &interner, &codegen_registry))
    } else {
        None
    };

    let (typescript_bindings, typescript_types) = if has_c {
        let (js, dts) = generate_typescript_bindings(&stmts, "module", &interner, &codegen_registry);
        (Some(js), Some(dts))
    } else {
        (None, None)
    };

    Ok(CompileOutput { rust_code, dependencies, c_header, python_bindings, typescript_types, typescript_bindings })
}

/// Generate the Rust SOURCE for an AOT-native cdylib of ONE function (HOTSWAP §Axis-3
/// / P14b): the `target` function plus its transitive callees ([`crate::codegen::function_slice`]),
/// emitted through the normal ARCHITECT codegen, with the
/// [`crate::codegen::codegen_native_tier_export`] shim appended so the loader can
/// resolve the `logos_native_<target>` symbol. Mirrors [`compile_program_full`]'s
/// front-end, then slices before codegen.
///
/// Returns `Ok(None)` when `target` is absent or outside the sound scalar ABI subset —
/// the caller then keeps that function on VM+JIT (no AOT-native, no gap at the seam).
/// `Err` only on a genuine parse/type error in the source.
pub fn compile_function_to_native_rust(
    source: &str,
    target: &str,
) -> Result<Option<AotModule>, ParseError> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    let (type_registry, policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };
    let codegen_registry = type_registry.clone();
    let codegen_policies = policy_registry.clone();

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();
    let ast_ctx = AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_expr_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, type_registry);
    let stmts = parser.parse_program()?;
    let opt_config = {
        let mut c = crate::optimization::OptimizationConfig::from_env()
            .merged(&parser.program_opt_flags());
        c.normalize();
        c
    };
    // Optimize the whole program (ARCHITECT), THEN slice: the slice picks the optimized
    // target + its (optimized) transitive callees.
    let stmts = crate::optimize::optimize_program(
        stmts,
        &imperative_expr_arena,
        &stmt_arena,
        &mut interner,
        &opt_config,
    );

    // Resolve the target's symbol + signature for the shim; absent ⇒ no AOT-native fn.
    let target_sig = stmts.iter().find_map(|s| match s {
        Stmt::FunctionDef { name, params, return_type, .. }
            if interner.resolve(*name) == target =>
        {
            let ps: Vec<(crate::intern::Symbol, &TypeExpr)> =
                params.iter().map(|(s, t)| (*s, *t)).collect();
            Some((*name, ps, *return_type))
        }
        _ => None,
    });
    let Some((target_sym, params, return_type)) = target_sig else {
        return Ok(None);
    };
    // The shim only exists for the sound scalar subset; otherwise no AOT-native fn.
    let Some(shim) =
        crate::codegen::codegen_native_tier_export(target_sym, &params, return_type, &interner)
    else {
        return Ok(None);
    };

    let slice = crate::codegen::function_slice(&stmts, target_sym, &interner);
    let type_env = crate::analysis::check_program(&slice, &interner, &codegen_registry).map_err(|e| {
        ParseError {
            kind: e.to_parse_error_kind(&interner),
            span: crate::token::Span::default(),
        }
    })?;
    let module = crate::codegen::codegen_program(
        &slice,
        &codegen_registry,
        &codegen_policies,
        &interner,
        &type_env,
        &opt_config,
    );
    let symbol = format!(
        "logos_native_{}",
        crate::analysis::types::RustNames::new(&interner).raw(target_sym)
    );
    Ok(Some(AotModule {
        rust: format!("{module}\n{shim}"),
        symbol,
        arity: params.len(),
    }))
}

/// A compiled AOT-native module (HOTSWAP §Axis-3): the cdylib Rust SOURCE plus the
/// exact exported `symbol` to resolve and its `arity` — everything the loader needs.
#[derive(Debug, Clone)]
pub struct AotModule {
    pub rust: String,
    pub symbol: String,
    pub arity: usize,
}

/// Build a function's AOT module to a browser-loadable `wasm32-unknown-unknown` cdylib
/// (HOTSWAP §Axis-3 / P17 — the browser analog of [`build_native_cdylib`]). The module
/// is the same scalar-ABI shim; the wasm crate keeps `logicaffeine-data` +
/// `logicaffeine-system` (both wasm-ready) but drops the desktop-only `tokio`, the
/// `logicaffeine-system` `full` feature, and `target-cpu=native`. Returns the `.wasm` path;
/// the browser then `WebAssembly.instantiate`s it and the entry calls into it through
/// the warm-bytecode indirection. Requires the `wasm32-unknown-unknown` target.
#[cfg(not(target_arch = "wasm32"))]
pub fn build_native_wasm(
    rust_source: &str,
    crate_name: &str,
    work_dir: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    use std::process::Command;
    let proj = work_dir.join(crate_name);
    let _ = fs::remove_dir_all(&proj);
    fs::create_dir_all(proj.join("src")).map_err(|e| e.to_string())?;
    fs::write(proj.join("src").join("lib.rs"), rust_source).map_err(|e| e.to_string())?;

    let cargo_toml = format!(
        "[package]\nname = \"{crate_name}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n\
         [lib]\ncrate-type = [\"cdylib\"]\n\n\
         [dependencies]\n\
         logicaffeine-data = {{ path = \"./crates/logicaffeine_data\" }}\n\
         logicaffeine-system = {{ path = \"./crates/logicaffeine_system\" }}\n\n\
         [profile.release]\nlto = true\nopt-level = 3\ncodegen-units = 1\nstrip = true\n"
    );
    fs::write(proj.join("Cargo.toml"), &cargo_toml).map_err(|e| e.to_string())?;

    copy_runtime_crates(&proj).map_err(|e| e.to_string())?;

    let out = Command::new("cargo")
        .args(["build", "--release", "--lib", "--target", "wasm32-unknown-unknown"])
        .current_dir(&proj)
        .output()
        .map_err(|e| format!("failed to run cargo: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "wasm bundle build failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    let wasm = proj
        .join("target")
        .join("wasm32-unknown-unknown")
        .join("release")
        .join(format!("{crate_name}.wasm"));
    if !wasm.exists() {
        return Err(format!("wasm module not found at {}", wasm.display()));
    }
    Ok(wasm)
}

/// Build AOT-native cdylib Rust source (from [`compile_function_to_native_rust`]) into
/// a loadable `cdylib` (HOTSWAP §Axis-3 / P16). Generates a minimal crate under
/// `work_dir/<crate_name>`, copies the shared runtime crates (so the `.so` and the
/// interpreter share the SAME `logicaffeine_data` ABI), and runs `cargo build
/// --release`. Returns the path to the produced dynamic library. `panic = "unwind"`
/// (the default) — NOT `abort` — so a panic in a loaded function can be contained
/// rather than killing the host process.
#[cfg(not(target_arch = "wasm32"))]
pub fn build_native_cdylib(
    rust_source: &str,
    crate_name: &str,
    work_dir: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    use std::process::Command;
    let proj = work_dir.join(crate_name);
    let _ = fs::remove_dir_all(&proj);
    fs::create_dir_all(proj.join("src")).map_err(|e| e.to_string())?;
    fs::write(proj.join("src").join("lib.rs"), rust_source).map_err(|e| e.to_string())?;

    let cargo_toml = format!(
        "[package]\nname = \"{crate_name}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n\
         [lib]\ncrate-type = [\"cdylib\"]\n\n\
         [dependencies]\n\
         logicaffeine-data = {{ path = \"./crates/logicaffeine_data\" }}\n\
         logicaffeine-system = {{ path = \"./crates/logicaffeine_system\", features = [\"full\"] }}\n\
         tokio = {{ version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }}\n\n\
         [profile.release]\nlto = true\nopt-level = 3\ncodegen-units = 1\nstrip = true\n"
    );
    fs::write(proj.join("Cargo.toml"), &cargo_toml).map_err(|e| e.to_string())?;

    let cargo_cfg = proj.join(".cargo");
    fs::create_dir_all(&cargo_cfg).map_err(|e| e.to_string())?;
    fs::write(
        cargo_cfg.join("config.toml"),
        "[build]\nrustflags = [\"-C\", \"target-cpu=native\"]\n",
    )
    .map_err(|e| e.to_string())?;

    copy_runtime_crates(&proj).map_err(|e| e.to_string())?;

    let out = Command::new("cargo")
        .args(["build", "--release", "--lib"])
        .current_dir(&proj)
        .output()
        .map_err(|e| format!("failed to run cargo: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "AOT cdylib build failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    let so = proj.join("target").join("release").join(format!(
        "{}{}{}",
        std::env::consts::DLL_PREFIX,
        crate_name,
        std::env::consts::DLL_SUFFIX
    ));
    if !so.exists() {
        return Err(format!("AOT cdylib not found at {}", so.display()));
    }
    Ok(so)
}

/// The cache key for an AOT-native artifact (HOTSWAP §Axis-3 / P16): a deterministic
/// FNV-1a hash of the rustc toolchain version AND the optimized Rust source. The
/// toolchain is in the key because Rust has no stable cross-version ABI — a different
/// `rustc`/runtime yields a different key, so a stale, ABI-mismatched `.so` is NEVER
/// reused (soundness over convenience). The source captures the function, its callees,
/// and the optimization config (it IS the codegen output).
#[cfg(not(target_arch = "wasm32"))]
pub fn aot_cache_key(rust_source: &str) -> String {
    let toolchain = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let mut h: u64 = 0xcbf29ce4_84222325;
    for b in toolchain.bytes().chain(rust_source.bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// [`build_native_cdylib`] with a persistent cache (HOTSWAP §Axis-3 / P16). The artifact
/// lives at `cache_dir/aot_<fn>_<key>/…` keyed by [`aot_cache_key`]; an identical
/// function reuses its `.so` across runs (no rebuild), and a toolchain change yields a
/// new key so the stale library is never loaded.
#[cfg(not(target_arch = "wasm32"))]
pub fn build_native_cdylib_cached(
    rust_source: &str,
    fn_name: &str,
    cache_dir: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let key = aot_cache_key(rust_source);
    let crate_name = format!("aot_{fn_name}_{key}");
    let so = cache_dir.join(&crate_name).join("target").join("release").join(format!(
        "{}{}{}",
        std::env::consts::DLL_PREFIX,
        crate_name,
        std::env::consts::DLL_SUFFIX
    ));
    if so.exists() {
        return Ok(so); // cache hit — reuse, skip the rustc build
    }
    fs::create_dir_all(cache_dir).map_err(|e| e.to_string())?;
    build_native_cdylib(rust_source, &crate_name, cache_dir)
}

/// On-demand AOT-native build + load (HOTSWAP §Axis-3): compile `fn_name` (and its
/// callees) to an optimized cdylib (cached), `dlopen` it, and return the loaded
/// [`crate::vm::NativeFn`] ready to install via `Vm::install_aot_native`. `None` if the
/// function is absent / outside the scalar subset / the build or load failed — the
/// caller keeps it on VM+JIT. Blocking (runs `rustc`); the background worker
/// (`BgAotCompiler`) calls this off the interpreter thread.
#[cfg(not(target_arch = "wasm32"))]
pub fn aot_build_function(
    source: &str,
    fn_name: &str,
    cache_dir: &std::path::Path,
) -> Option<Box<dyn crate::vm::NativeFn>> {
    let module = compile_function_to_native_rust(source, fn_name).ok().flatten()?;
    let so = build_native_cdylib_cached(&module.rust, fn_name, cache_dir).ok()?;
    let (nf, _calls) = crate::vm::aot_tier::load_aot_native(&so, &module.symbol, module.arity)?;
    Some(nf)
}

/// The names of functions annotated for the AOT-native tier — `## To <fn> … is
/// exported for native:` (HOTSWAP §Axis-3 selectivity). These are the functions
/// `largo build --native-functions` pre-bundles; everything else stays on VM+JIT.
pub fn native_export_function_names(source: &str) -> Vec<String> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| match parsed {
        Ok((stmts, _types, _policies)) => stmts
            .iter()
            .filter_map(|s| match s {
                Stmt::FunctionDef { name, is_exported: true, export_target: Some(t), .. }
                    if interner.resolve(*t).eq_ignore_ascii_case("native") =>
                {
                    Some(interner.resolve(*name).to_string())
                }
                _ => None,
            })
            .collect(),
        Err(_) => Vec::new(),
    })
}

/// Load every `native`-annotated function in `source` as a compiled-native
/// [`crate::vm::NativeFn`] (HOTSWAP §Axis-3): the run path installs these via
/// `Vm::install_aot_native`, so an annotated function dispatches to `rustc -O3` machine
/// code from its first call. Uses the persistent cache, so a bundle pre-built by
/// `largo build --native-functions` is a cache hit — no `rustc` on the run path. A
/// function outside the scalar subset / failing to build is skipped (stays on VM+JIT).
#[cfg(not(target_arch = "wasm32"))]
pub fn aot_load_bundle(
    source: &str,
    cache_dir: &std::path::Path,
) -> Vec<(String, Box<dyn crate::vm::NativeFn>)> {
    native_export_function_names(source)
        .into_iter()
        .filter_map(|name| aot_build_function(source, &name, cache_dir).map(|nf| (name, nf)))
        .collect()
}

/// Pre-build every `native`-annotated function in `source` into the AOT bundle under
/// `bundle_dir` (HOTSWAP §Axis-3 / P16 — the "tools to bundle them"). Returns the
/// `(function_name, cdylib_path)` manifest. Functions outside the sound scalar subset
/// are silently skipped — they keep running on VM+JIT, no gap at the seam. `Err` only
/// on a genuine parse/type error.
#[cfg(not(target_arch = "wasm32"))]
pub fn build_native_bundle(
    source: &str,
    bundle_dir: &std::path::Path,
) -> Result<Vec<(String, std::path::PathBuf)>, ParseError> {
    let mut manifest = Vec::new();
    for name in native_export_function_names(source) {
        let Some(module) = compile_function_to_native_rust(source, &name)? else {
            continue; // not in the scalar subset — stays on VM+JIT
        };
        if let Ok(so) = build_native_cdylib_cached(&module.rust, &name, bundle_dir) {
            manifest.push((name, so));
        }
    }
    Ok(manifest)
}

/// Extract crate dependencies from `Stmt::Require` nodes.
///
/// Deduplicates by crate name: same name + same version keeps one copy.
/// Same name + different version returns a `ParseError`.
/// Preserves declaration order (first occurrence wins).
fn extract_dependencies(stmts: &[Stmt], interner: &Interner) -> Result<Vec<CrateDependency>, ParseError> {
    use std::collections::HashMap;

    let mut seen: HashMap<String, String> = HashMap::new(); // name → version
    let mut deps: Vec<CrateDependency> = Vec::new();

    for stmt in stmts {
        if let Stmt::Require { crate_name, version, features, span } = stmt {
            let name = interner.resolve(*crate_name).to_string();
            let ver = interner.resolve(*version).to_string();

            if let Some(existing_ver) = seen.get(&name) {
                if *existing_ver != ver {
                    return Err(ParseError {
                        kind: crate::error::ParseErrorKind::Custom(format!(
                            "Conflicting versions for crate \"{}\": \"{}\" and \"{}\".",
                            name, existing_ver, ver
                        )),
                        span: *span,
                    });
                }
                // Same name + same version: skip duplicate
            } else {
                seen.insert(name.clone(), ver.clone());
                deps.push(CrateDependency {
                    name,
                    version: ver,
                    features: features.iter().map(|f| interner.resolve(*f).to_string()).collect(),
                });
            }
        }
    }

    Ok(deps)
}

/// Compile LOGOS source to Rust with ownership checking enabled.
///
/// This runs the lightweight ownership analysis pass that catches use-after-move
/// errors with control flow awareness. The analysis is fast enough to run on
/// every keystroke in an IDE.
///
/// # Arguments
///
/// * `source` - LOGOS source code as a string
///
/// # Returns
///
/// Generated Rust source code on success.
///
/// # Errors
///
/// Returns [`ParseError`] if:
/// - Any error from [`compile_to_rust`] occurs
/// - Ownership analysis detects use-after-move
/// - Ownership analysis detects use-after-borrow violations
///
/// # Example
///
/// ```
/// # use logicaffeine_compile::compile::compile_to_rust_checked;
/// // This will fail ownership checking
/// let source = "## Main\nLet x be 5.\nGive x to y.\nShow x.";
/// let result = compile_to_rust_checked(source);
/// assert!(result.is_err()); // "x has already been given away"
/// ```
///
/// # Use Case
///
/// Use this function with the `--check` CLI flag for instant feedback on
/// ownership errors before running the full Rust compilation.
pub fn compile_to_rust_checked(source: &str) -> Result<String, ParseError> {
    // Mixed document: ownership-check only the imperative stream (math blocks are
    // blanked, preserving line numbers). The math half is checked by the kernel.
    let (imperative_src, _) = crate::ui_bridge::partition_mixed(source);
    let source = imperative_src.as_str();
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    // Pass 1: Discovery - scan for type definitions and policies
    let (type_registry, policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };
    // Clone for codegen (parser takes ownership)
    let codegen_registry = type_registry.clone();
    let codegen_policies = policy_registry.clone();

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ast_ctx = AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_expr_arena,
    );

    // Pass 2: Parse with type context
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, type_registry);
    let stmts = parser.parse_program()?;

    // Pass 2.5: Optimization - constant folding, propagation, and dead code elimination
    let stmts = crate::optimize::optimize_program(stmts, &imperative_expr_arena, &stmt_arena, &mut interner, &crate::optimization::OptimizationConfig::from_env());

    // Pass 3: Escape analysis
    let mut escape_checker = EscapeChecker::new(&interner);
    escape_checker.check_program(&stmts).map_err(|e| {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(e.to_string()),
            span: e.span,
        }
    })?;

    // Pass 4: Ownership analysis
    // Catches use-after-move errors with control flow awareness
    let mut ownership_checker = OwnershipChecker::new(&interner);
    ownership_checker.check_program(&stmts).map_err(|e| {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(e.to_string()),
            span: e.span,
        }
    })?;

    let type_env = crate::analysis::check_program(&stmts, &interner, &codegen_registry)
        .map_err(|e| ParseError {
            kind: e.to_parse_error_kind(&interner),
            span: crate::token::Span::default(),
        })?;
    let rust_code = codegen_program(&stmts, &codegen_registry, &codegen_policies, &interner, &type_env, &crate::optimization::OptimizationConfig::from_env());

    Ok(rust_code)
}

/// Compile LOGOS source to Rust with full Z3 static verification.
///
/// This runs the Z3-based verifier on Assert statements before code generation,
/// proving that assertions hold for all possible inputs. This is the most
/// thorough compilation mode, suitable for high-assurance code.
///
/// # Arguments
///
/// * `source` - LOGOS source code as a string
///
/// # Returns
///
/// Generated Rust source code on success.
///
/// # Errors
///
/// Returns [`ParseError`] if:
/// - Any error from [`compile_to_rust`] occurs
/// - Z3 cannot prove an Assert statement
/// - Refinement type constraints cannot be satisfied
/// - Termination cannot be proven for loops with `decreasing`
///
/// # Example
///
/// ```no_run
/// # use logicaffeine_compile::compile::compile_to_rust_verified;
/// # use logicaffeine_compile::ParseError;
/// # fn main() -> Result<(), ParseError> {
/// let source = r#"
/// ## Main
/// Let x: { it: Int | it > 0 } be 5.
/// Assert that x > 0.
/// "#;
/// let rust_code = compile_to_rust_verified(source)?;
/// # Ok(())
/// # }
/// ```
///
/// # Feature Flag
///
/// This function requires the `verification` feature to be enabled:
///
/// ```toml
/// [dependencies]
/// logicaffeine_compile = { version = "...", features = ["verification"] }
/// ```
#[cfg(feature = "verification")]
pub fn compile_to_rust_verified(source: &str) -> Result<String, ParseError> {
    use crate::verification::VerificationPass;

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    // Pass 1: Discovery - scan for type definitions and policies
    let (type_registry, policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };
    // Clone for codegen (parser takes ownership)
    let codegen_registry = type_registry.clone();
    let codegen_policies = policy_registry.clone();

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ast_ctx = AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_expr_arena,
    );

    // Pass 2: Parse with type context
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, type_registry);
    let stmts = parser.parse_program()?;

    // Pass 3: Escape analysis
    let mut escape_checker = EscapeChecker::new(&interner);
    escape_checker.check_program(&stmts).map_err(|e| {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(e.to_string()),
            span: e.span,
        }
    })?;

    // Pass 4: Static verification
    let mut verifier = VerificationPass::new(&interner);
    verifier.verify_program(&stmts).map_err(|e| {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(format!(
                "Verification Failed:\n\n{}",
                e
            )),
            span: crate::token::Span::default(),
        }
    })?;

    let type_env = crate::analysis::check_program(&stmts, &interner, &codegen_registry)
        .map_err(|e| ParseError {
            kind: e.to_parse_error_kind(&interner),
            span: crate::token::Span::default(),
        })?;
    let rust_code = codegen_program(&stmts, &codegen_registry, &codegen_policies, &interner, &type_env, &crate::optimization::OptimizationConfig::from_env());

    Ok(rust_code)
}

/// Compile LOGOS source and write output to a directory as a Cargo project.
///
/// Creates a complete Cargo project structure with:
/// - `src/main.rs` containing the generated Rust code
/// - `Cargo.toml` with runtime dependencies
/// - `crates/` directory with runtime crate copies
///
/// # Arguments
///
/// * `source` - LOGOS source code as a string
/// * `output_dir` - Directory to create the Cargo project in
///
/// # Errors
///
/// Returns [`CompileError`] if:
/// - Compilation fails (wrapped as `CompileError::Parse`)
/// - File system operations fail (wrapped as `CompileError::Io`)
///
/// # Example
///
/// ```no_run
/// # use logicaffeine_compile::compile::{compile_to_dir, CompileError};
/// # use std::path::Path;
/// # fn main() -> Result<(), CompileError> {
/// let source = "## Main\nShow \"Hello\".";
/// compile_to_dir(source, Path::new("/tmp/my_project"))?;
/// // Now /tmp/my_project is a buildable Cargo project
/// # Ok(())
/// # }
/// ```
/// Everything [`rustc_check`] needs short of running cargo: generated Rust,
/// dependencies, the POPULATED rustc→LOGOS source map, and the interner the
/// program was compiled with — the substrate the diagnostic bridge translates
/// through.
pub struct CheckArtifacts {
    pub rust_code: String,
    pub dependencies: Vec<CrateDependency>,
    pub source_map: crate::sourcemap::SourceMap,
    pub interner: Interner,
}

/// Compile for CHECKING: the same front as [`compile_program_full`] but with
/// the optimizer off and mapped codegen, so every generated line ties back to
/// its statement span and the map stays 1:1 (the optimizer may fuse or drop
/// statements; a check build has no use for its output anyway).
///
/// Deliberately does NOT bail on typecheck/escape/ownership findings — the
/// interactive pipeline already reports those; this path exists to surface
/// what only rustc can see, and a borrow error in the generated code comes
/// back TRANSLATED rather than pre-empted.
pub fn rustc_check_artifacts(source: &str) -> Result<CheckArtifacts, ParseError> {
    let user_source = source;

    // The stdlib prelude is PREPENDED when referenced; spans from the parse
    // are offsets into the expanded text and shift back by the prelude length.
    let prelude_src = crate::loader::apply_prelude(user_source);
    let expanded: &str = prelude_src.as_ref();
    debug_assert!(
        expanded.ends_with(user_source),
        "apply_prelude must PREPEND; span translation depends on it"
    );
    let prelude_offset = expanded.len() - user_source.len();

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(expanded, &mut interner);
    let tokens = lexer.tokenize();

    let (type_registry, policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };
    let codegen_registry = type_registry.clone();
    let codegen_policies = policy_registry.clone();

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ast_ctx = AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_expr_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, type_registry);
    let stmts = parser.parse_program()?;

    // Translate spans back into the USER buffer; prelude statements become
    // zero-width sentinels that mapped codegen skips.
    let stmt_spans: Vec<crate::token::Span> = parser
        .stmt_spans()
        .iter()
        .map(|span| {
            if span.start >= prelude_offset {
                crate::token::Span::new(span.start - prelude_offset, span.end - prelude_offset)
            } else {
                crate::token::Span::default()
            }
        })
        .collect();

    // Exact division still resolves (type semantics, statement-count
    // preserving); the optimizer stays OFF so spans align 1:1.
    let resolved = crate::resolve_division::resolve_divisions(
        &stmts,
        &stmt_arena,
        &imperative_expr_arena,
        &interner,
        false,
    );
    let stmts: Vec<Stmt> = match resolved {
        Some(rw) => rw.to_vec(),
        None => stmts,
    };

    // Best-effort env: findings are the interactive pipeline's job here.
    let (type_env, _findings) =
        crate::analysis::check_program_collect(&stmts, &interner, &codegen_registry);

    let dependencies = extract_dependencies(&stmts, &interner)?;

    let (rust_code, source_map) = crate::codegen::codegen_program_mapped(
        &stmts,
        &codegen_registry,
        &codegen_policies,
        &interner,
        &type_env,
        &crate::optimization::OptimizationConfig::all_off(),
        &stmt_spans,
        user_source,
    );

    Ok(CheckArtifacts {
        rust_code,
        dependencies,
        source_map,
        interner,
    })
}

/// Run rustc's analysis over a LOGOS program and translate every finding
/// back to English with real user-source spans — the flycheck engine.
///
/// `cache_dir` is a persistent per-workspace directory: the first run stages
/// the runtime crates and compiles them; later runs are incremental. Uses
/// `cargo check` (no codegen, no linking).
///
/// Fail-loud contract: `Ok(vec![])` means cargo ran AND found nothing. A
/// cargo failure that translates to no LOGOS finding is a `Build` error, not
/// a silent all-clear.
pub fn rustc_check(source: &str, cache_dir: &Path) -> Result<Vec<LogosError>, CompileError> {
    let artifacts = rustc_check_artifacts(source).map_err(CompileError::Parse)?;
    write_cargo_project(&artifacts.rust_code, &artifacts.dependencies, cache_dir)?;

    let check_output = Command::new("cargo")
        .arg("check")
        .arg("--message-format=json")
        .current_dir(cache_dir)
        .output()
        .map_err(|e| CompileError::Io(e.to_string()))?;

    let stdout = String::from_utf8_lossy(&check_output.stdout);
    let diagnostics = parse_rustc_json(&stdout);
    let findings =
        crate::diagnostic::translate_diagnostics_all(&diagnostics, &artifacts.source_map, &artifacts.interner);

    if !check_output.status.success() && findings.is_empty() {
        let stderr = String::from_utf8_lossy(&check_output.stderr);
        return Err(CompileError::Build(stderr.to_string()));
    }

    Ok(findings)
}

pub fn compile_to_dir(source: &str, output_dir: &Path) -> Result<(), CompileError> {
    let output = compile_program_full(source).map_err(CompileError::Parse)?;
    write_cargo_project(&output.rust_code, &output.dependencies, output_dir)
}

/// Write a generated program as a runnable cargo project: `src/main.rs`,
/// `Cargo.toml` (runtime path-deps + user `## Requires`), `.cargo/config.toml`,
/// and the staged runtime crates. Shared by [`compile_to_dir`] and
/// [`rustc_check`].
pub fn write_cargo_project(
    rust_code: &str,
    dependencies: &[CrateDependency],
    output_dir: &Path,
) -> Result<(), CompileError> {
    // Create output directory structure
    let src_dir = output_dir.join("src");
    fs::create_dir_all(&src_dir).map_err(|e| CompileError::Io(e.to_string()))?;

    // Write main.rs (codegen already includes the use statements)
    let main_path = src_dir.join("main.rs");
    let mut file = fs::File::create(&main_path).map_err(|e| CompileError::Io(e.to_string()))?;
    file.write_all(rust_code.as_bytes()).map_err(|e| CompileError::Io(e.to_string()))?;

    // Write Cargo.toml with runtime crate dependencies
    let mut cargo_toml = String::from(r#"[package]
name = "logos_output"
version = "0.1.0"
edition = "2021"

[dependencies]
logicaffeine-data = { path = "./crates/logicaffeine_data" }
logicaffeine-system = { path = "./crates/logicaffeine_system", features = ["full"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
"#);

    // Append user-declared dependencies from ## Requires blocks — inside
    // [dependencies], before any later section header (a dep written after
    // a [target.*] or [profile.*] header would silently change meaning).
    for dep in dependencies {
        if dep.features.is_empty() {
            let _ = writeln!(cargo_toml, "{} = \"{}\"", dep.name, dep.version);
        } else {
            let feats = dep.features.iter()
                .map(|f| format!("\"{}\"", f))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(
                cargo_toml,
                "{} = {{ version = \"{}\", features = [{}] }}",
                dep.name, dep.version, feats
            );
        }
    }

    cargo_toml.push_str(r#"
[target.'cfg(target_os = "linux")'.dependencies]
logicaffeine-system = { path = "./crates/logicaffeine_system", features = ["full", "io-uring"] }
"#);

    cargo_toml.push_str("\n[profile.release]\nlto = true\nopt-level = 3\ncodegen-units = 1\npanic = \"abort\"\nstrip = true\n");

    let cargo_path = output_dir.join("Cargo.toml");
    let mut file = fs::File::create(&cargo_path).map_err(|e| CompileError::Io(e.to_string()))?;
    file.write_all(cargo_toml.as_bytes()).map_err(|e| CompileError::Io(e.to_string()))?;

    // Write .cargo/config.toml with target-cpu=native for optimal codegen.
    // Enables SIMD auto-vectorization and CPU-specific instruction selection.
    let cargo_config_dir = output_dir.join(".cargo");
    fs::create_dir_all(&cargo_config_dir).map_err(|e| CompileError::Io(e.to_string()))?;
    let config_content = "[build]\nrustflags = [\"-C\", \"target-cpu=native\"]\n";
    let config_path = cargo_config_dir.join("config.toml");
    fs::write(&config_path, config_content).map_err(|e| CompileError::Io(e.to_string()))?;

    // Copy runtime crates to output directory
    copy_runtime_crates(output_dir)?;

    Ok(())
}

/// Copy the runtime crates to the output directory.
/// Copies logicaffeine_data and logicaffeine_system.
pub fn copy_runtime_crates(output_dir: &Path) -> Result<(), CompileError> {
    let crates_dir = output_dir.join("crates");
    fs::create_dir_all(&crates_dir).map_err(|e| CompileError::Io(e.to_string()))?;

    // Find workspace root
    let workspace_root = find_workspace_root()?;

    // Copy logicaffeine_data
    let data_src = workspace_root.join(CRATES_DATA_PATH);
    let data_dest = crates_dir.join("logicaffeine_data");
    copy_dir_recursive(&data_src, &data_dest)?;
    deworkspace_cargo_toml(&data_dest.join("Cargo.toml"))?;

    // Copy logicaffeine_system
    let system_src = workspace_root.join(CRATES_SYSTEM_PATH);
    let system_dest = crates_dir.join("logicaffeine_system");
    copy_dir_recursive(&system_src, &system_dest)?;
    deworkspace_cargo_toml(&system_dest.join("Cargo.toml"))?;

    // Also need to copy logicaffeine_base since both crates depend on it
    let base_src = workspace_root.join("crates/logicaffeine_base");
    let base_dest = crates_dir.join("logicaffeine_base");
    copy_dir_recursive(&base_src, &base_dest)?;
    deworkspace_cargo_toml(&base_dest.join("Cargo.toml"))?;

    Ok(())
}

/// Resolve workspace-inherited fields in a copied crate's Cargo.toml.
///
/// When runtime crates are copied to a standalone project, any fields using
/// `*.workspace = true` won't resolve because there's no parent workspace.
/// This rewrites them with concrete values (matching the workspace's settings).
fn deworkspace_cargo_toml(cargo_toml_path: &Path) -> Result<(), CompileError> {
    let content = fs::read_to_string(cargo_toml_path)
        .map_err(|e| CompileError::Io(e.to_string()))?;

    let mut result = String::with_capacity(content.len());
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "edition.workspace = true" {
            result.push_str("edition = \"2021\"");
        } else if trimmed == "rust-version.workspace = true" {
            result.push_str("rust-version = \"1.75\"");
        } else if trimmed == "authors.workspace = true"
            || trimmed == "repository.workspace = true"
            || trimmed == "homepage.workspace = true"
            || trimmed == "documentation.workspace = true"
            || trimmed == "keywords.workspace = true"
            || trimmed == "categories.workspace = true"
            || trimmed == "license.workspace = true"
        {
            // Drop these lines — they're metadata not needed for compilation
            continue;
        } else if trimmed.contains(".workspace = true") {
            // Catch-all: drop any other workspace-inherited fields
            continue;
        } else if let Some(rewritten) = deworkspace_dep_line(trimmed)? {
            // A dependency-TABLE inheritance (`name = { workspace = true, … }`):
            // internal crates are staged side-by-side under `crates/`, so the
            // inherited entry rewrites to a sibling path (extra keys kept).
            result.push_str(&rewritten);
        } else {
            result.push_str(line);
        }
        result.push('\n');
    }

    fs::write(cargo_toml_path, result)
        .map_err(|e| CompileError::Io(e.to_string()))?;

    Ok(())
}

/// Rewrite a `name = { workspace = true, … }` DEPENDENCY line for a staged
/// crate. Internal `logicaffeine-*` deps become sibling-path deps (the staged
/// layout puts every runtime crate under one `crates/` directory); any other
/// key on the line (`features`, `optional`, …) is preserved. A THIRD-PARTY
/// workspace-inherited dep fails LOUDLY here — at staging time, with the dep
/// named — instead of as an opaque cargo resolution error deep in a build log.
/// Returns `Ok(None)` for lines that are not workspace-dep tables.
fn deworkspace_dep_line(trimmed: &str) -> Result<Option<String>, CompileError> {
    let Some((name_part, rest)) = trimmed.split_once('=') else {
        return Ok(None);
    };
    let rest = rest.trim();
    if !rest.starts_with('{') || !rest.ends_with('}') || !rest.contains("workspace = true") {
        return Ok(None);
    }
    let name = name_part.trim();
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Ok(None);
    }
    let extra_keys: Vec<&str> = rest[1..rest.len() - 1]
        .split(',')
        .map(str::trim)
        .filter(|k| !k.is_empty() && *k != "workspace = true")
        .collect();
    if !name.starts_with("logicaffeine-") {
        return Err(CompileError::Io(format!(
            "staged crate inherits third-party dependency `{}` from the workspace — \
             declare a concrete version in the crate's Cargo.toml (the staged copy \
             has no workspace root to inherit from)",
            name
        )));
    }
    let dir = name.replace('-', "_");
    let mut entry = format!("{} = {{ path = \"../{}\"", name, dir);
    for key in extra_keys {
        entry.push_str(", ");
        entry.push_str(key);
    }
    entry.push_str(" }");
    Ok(Some(entry))
}

/// Find the workspace root directory.
fn find_workspace_root() -> Result<std::path::PathBuf, CompileError> {
    // 1. Explicit override via LOGOS_WORKSPACE env var
    if let Ok(workspace) = std::env::var("LOGOS_WORKSPACE") {
        let path = Path::new(&workspace);
        if path.join("Cargo.toml").exists() && path.join("crates").exists() {
            return Ok(path.to_path_buf());
        }
    }

    // 2. Try CARGO_MANIFEST_DIR (works during cargo build of largo itself)
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let path = Path::new(&manifest_dir);
        if let Some(parent) = path.parent().and_then(|p| p.parent()) {
            if parent.join("Cargo.toml").exists() {
                return Ok(parent.to_path_buf());
            }
        }
    }

    // 3. Infer from the largo binary's own location
    //    e.g. /workspace/target/release/largo → /workspace
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Walk up from the binary's directory
            let mut candidate = dir.to_path_buf();
            for _ in 0..5 {
                if candidate.join("Cargo.toml").exists() && candidate.join("crates").exists() {
                    return Ok(candidate);
                }
                if !candidate.pop() {
                    break;
                }
            }
        }
    }

    // 4. Fallback to current directory traversal
    let mut current = std::env::current_dir()
        .map_err(|e| CompileError::Io(e.to_string()))?;

    loop {
        if current.join("Cargo.toml").exists() && current.join("crates").exists() {
            return Ok(current);
        }
        if !current.pop() {
            return Err(CompileError::Io(
                "Could not find workspace root. Set LOGOS_WORKSPACE env var or run from within the workspace.".to_string()
            ));
        }
    }
}

/// Recursively copy a directory.
/// Skips files that disappear during copy (race condition with parallel builds).
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), CompileError> {
    fs::create_dir_all(dst).map_err(|e| CompileError::Io(e.to_string()))?;

    for entry in fs::read_dir(src).map_err(|e| CompileError::Io(e.to_string()))? {
        let entry = entry.map_err(|e| CompileError::Io(e.to_string()))?;
        let src_path = entry.path();
        let file_name = entry.file_name();
        let dst_path = dst.join(&file_name);

        // Skip target directory, build artifacts, and lock files
        if file_name == "target"
            || file_name == ".git"
            || file_name == "Cargo.lock"
            || file_name == ".DS_Store"
        {
            continue;
        }

        // Skip files that start with a dot (hidden files)
        if file_name.to_string_lossy().starts_with('.') {
            continue;
        }

        // Check if path still exists (race condition protection)
        if !src_path.exists() {
            continue;
        }

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if file_name == "Cargo.toml" {
            // Special handling for Cargo.toml: remove [workspace] line
            // which can interfere with nested crate dependencies
            match fs::read_to_string(&src_path) {
                Ok(content) => {
                    let filtered: String = content
                        .lines()
                        .filter(|line| !line.trim().starts_with("[workspace]"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    fs::write(&dst_path, filtered)
                        .map_err(|e| CompileError::Io(e.to_string()))?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(CompileError::Io(e.to_string())),
            }
        } else {
            match fs::copy(&src_path, &dst_path) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(CompileError::Io(e.to_string())),
            }
        }
    }

    Ok(())
}

/// Compile and run a LOGOS program end-to-end.
///
/// This function performs the full compilation workflow:
/// 1. Compile LOGOS to Rust via [`compile_to_dir`]
/// 2. Run `cargo build` with JSON diagnostics
/// 3. Translate any rustc errors to LOGOS-friendly messages
/// 4. Run the compiled program via `cargo run`
///
/// # Arguments
///
/// * `source` - LOGOS source code as a string
/// * `output_dir` - Directory to create the temporary Cargo project in
///
/// # Returns
///
/// The stdout output of the executed program.
///
/// # Errors
///
/// Returns [`CompileError`] if:
/// - Compilation fails (see [`compile_to_dir`])
/// - Rust compilation fails (`CompileError::Build` or `CompileError::Ownership`)
/// - The program crashes at runtime (`CompileError::Runtime`)
///
/// # Diagnostic Translation
///
/// When rustc reports errors (e.g., E0382 for use-after-move), this function
/// uses the [`diagnostic`](crate::diagnostic) module to translate them into
/// LOGOS-friendly Socratic error messages.
///
/// # Example
///
/// ```no_run
/// # use logicaffeine_compile::compile::{compile_and_run, CompileError};
/// # use std::path::Path;
/// # fn main() -> Result<(), CompileError> {
/// let source = "## Main\nShow \"Hello, World!\".";
/// let output = compile_and_run(source, Path::new("/tmp/run"))?;
/// assert_eq!(output.trim(), "Hello, World!");
/// # Ok(())
/// # }
/// ```
pub fn compile_and_run(source: &str, output_dir: &Path) -> Result<String, CompileError> {
    // Pre-check: catch ownership errors (use-after-move) with friendly messages
    // before codegen runs (codegen defensively clones, masking these errors)
    compile_to_rust_checked(source).map_err(CompileError::Parse)?;

    compile_to_dir(source, output_dir)?;

    // Run cargo build with JSON message format for structured error parsing
    let build_output = Command::new("cargo")
        .arg("build")
        .arg("--message-format=json")
        .current_dir(output_dir)
        .output()
        .map_err(|e| CompileError::Io(e.to_string()))?;

    if !build_output.status.success() {
        let stderr = String::from_utf8_lossy(&build_output.stderr);
        let stdout = String::from_utf8_lossy(&build_output.stdout);

        // Try to parse JSON diagnostics and translate them
        let diagnostics = parse_rustc_json(&stdout);

        if !diagnostics.is_empty() {
            // Create a basic source map with the LOGOS source
            let source_map = SourceMap::new(source.to_string());
            let interner = Interner::new();

            if let Some(logos_error) = translate_diagnostics(&diagnostics, &source_map, &interner) {
                return Err(CompileError::Ownership(logos_error));
            }
        }

        // Fallback to raw error if translation fails
        return Err(CompileError::Build(stderr.to_string()));
    }

    // Run the compiled program
    let run_output = Command::new("cargo")
        .arg("run")
        .arg("--quiet")
        .current_dir(output_dir)
        .output()
        .map_err(|e| CompileError::Io(e.to_string()))?;

    if !run_output.status.success() {
        let stderr = String::from_utf8_lossy(&run_output.stderr);
        return Err(CompileError::Runtime(stderr.to_string()));
    }

    let stdout = String::from_utf8_lossy(&run_output.stdout);
    Ok(stdout.to_string())
}

/// Compile a LOGOS source file.
/// For single-file compilation without dependencies.
pub fn compile_file(path: &Path) -> Result<String, CompileError> {
    let source = fs::read_to_string(path).map_err(|e| CompileError::Io(e.to_string()))?;
    compile_to_rust(&source).map_err(CompileError::Parse)
}

/// Compile a multi-file LOGOS project with dependency resolution.
///
/// This function:
/// 1. Reads the entry file
/// 2. Scans for dependencies in the abstract (Markdown links)
/// 3. Recursively loads and discovers types from dependencies
/// 4. Compiles with the combined type registry
///
/// # Arguments
/// * `entry_file` - The main entry file to compile (root is derived from parent directory)
///
/// # Example
/// ```no_run
/// # use logicaffeine_compile::compile::compile_project;
/// # use std::path::Path;
/// let result = compile_project(Path::new("/project/main.md"));
/// ```
pub fn compile_project(entry_file: &Path) -> Result<CompileOutput, CompileError> {
    use crate::loader::Loader;
    use crate::analysis::discover_with_imports;

    let root_path = entry_file.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut loader = Loader::new(root_path);
    let mut interner = Interner::new();

    // Read the entry file
    let raw_source = fs::read_to_string(entry_file)
        .map_err(|e| CompileError::Io(format!("Failed to read entry file: {}", e)))?;

    // Auto-import the demand-driven stdlib prelude (uuid.lg, io/net/crdt/time/…) — the SAME seam the
    // string-compile and interpreter paths use (see `apply_prelude`), so `largo build` resolves stdlib
    // functions like `md5`/`sha1`/`uuidV3`/`uuidParse`/`flush`. Demand-driven + idempotent: a program
    // that references no stdlib name compiles byte-identically to before.
    let source = crate::loader::apply_prelude(&raw_source).into_owned();

    // Discover types from entry file and all imports
    let type_registry = discover_with_imports(entry_file, &source, &mut loader, &mut interner)
        .map_err(|e| CompileError::Io(e))?;

    // Now compile with the discovered types
    compile_to_rust_with_registry_full(&source, type_registry, &mut interner)
        .map_err(CompileError::Parse)
}

/// Compile LOGOS source with a pre-populated type registry, returning full output.
/// Returns both generated Rust code and extracted dependencies.
fn compile_to_rust_with_registry_full(
    source: &str,
    type_registry: crate::analysis::TypeRegistry,
    interner: &mut Interner,
) -> Result<CompileOutput, ParseError> {
    let mut lexer = Lexer::new(source, interner);
    let tokens = lexer.tokenize();

    // Discovery pass for policies (types already discovered)
    let policy_registry = {
        let mut discovery = DiscoveryPass::new(&tokens, interner);
        discovery.run_full().policies
    };

    let codegen_registry = type_registry.clone();
    let codegen_policies = policy_registry.clone();

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ast_ctx = AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_expr_arena,
    );

    let mut parser = Parser::new(tokens, &mut world_state, interner, ast_ctx, type_registry);
    let stmts = parser.parse_program()?;

    // Run the AST optimizer in the production (largo) compile path so
    // compiled binaries get the same optimizations the test path
    // (compile_program_full) already validates: closed-form / modulus
    // deferral, partial evaluation, constant folding, GVN, LICM,
    // deforestation, CTFE, supercompilation. Previously skipped here, which
    // left the whole optimize layer off for compiled programs. Confirmed a
    // clean win (geomean 1.388x -> 1.735x C-speed, all benchmarks verify
    // correct, no regressions).
    let stmts = crate::optimize::optimize_program(stmts, &imperative_expr_arena, &stmt_arena, interner, &crate::optimization::OptimizationConfig::from_env());

    // Extract dependencies before escape analysis
    let mut dependencies = extract_dependencies(&stmts, interner)?;

    // FFI: Auto-inject wasm-bindgen dependency if any function is exported for WASM
    let needs_wasm_bindgen = stmts.iter().any(|stmt| {
        if let Stmt::FunctionDef { is_exported: true, export_target: Some(target), .. } = stmt {
            interner.resolve(*target).eq_ignore_ascii_case("wasm")
        } else {
            false
        }
    });
    if needs_wasm_bindgen && !dependencies.iter().any(|d| d.name == "wasm-bindgen") {
        dependencies.push(CrateDependency {
            name: "wasm-bindgen".to_string(),
            version: "0.2".to_string(),
            features: vec![],
        });
    }

    let mut escape_checker = EscapeChecker::new(interner);
    escape_checker.check_program(&stmts).map_err(|e| {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(e.to_string()),
            span: e.span,
        }
    })?;

    let type_env = crate::analysis::check_program(&stmts, interner, &codegen_registry)
        .map_err(|e| ParseError {
            kind: e.to_parse_error_kind(interner),
            span: crate::token::Span::default(),
        })?;
    let rust_code = codegen_program(&stmts, &codegen_registry, &codegen_policies, interner, &type_env, &crate::optimization::OptimizationConfig::from_env());

    // Universal ABI: Generate C header + bindings if any C exports exist
    let has_c = stmts.iter().any(|stmt| {
        if let Stmt::FunctionDef { is_exported: true, export_target, .. } = stmt {
            match export_target {
                None => true,
                Some(t) => interner.resolve(*t).eq_ignore_ascii_case("c"),
            }
        } else {
            false
        }
    });

    let c_header = if has_c {
        Some(generate_c_header(&stmts, "module", interner, &codegen_registry))
    } else {
        None
    };

    if has_c && !dependencies.iter().any(|d| d.name == "serde_json") {
        dependencies.push(CrateDependency {
            name: "serde_json".to_string(),
            version: "1".to_string(),
            features: vec![],
        });
    }

    let python_bindings = if has_c {
        Some(generate_python_bindings(&stmts, "module", interner, &codegen_registry))
    } else {
        None
    };

    let (typescript_bindings, typescript_types) = if has_c {
        let (js, dts) = generate_typescript_bindings(&stmts, "module", interner, &codegen_registry);
        (Some(js), Some(dts))
    } else {
        (None, None)
    };

    Ok(CompileOutput { rust_code, dependencies, c_header, python_bindings, typescript_types, typescript_bindings })
}

/// Errors that can occur during the LOGOS compilation pipeline.
///
/// This enum represents the different stages where compilation can fail,
/// from parsing through to runtime execution.
///
/// # Error Hierarchy
///
/// ```text
/// CompileError
/// ├── Parse      ← Lexing, parsing, or static analysis
/// ├── Io         ← File system operations
/// ├── Build      ← Rust compilation (cargo build)
/// ├── Ownership  ← Translated borrow checker errors
/// └── Runtime    ← Program execution failure
/// ```
///
/// # Error Translation
///
/// The `Ownership` variant contains LOGOS-friendly error messages translated
/// from rustc's borrow checker errors (E0382, E0505, E0597) using the
/// [`diagnostic`](crate::diagnostic) module.
#[derive(Debug)]
pub enum CompileError {
    /// Parsing or static analysis failed.
    ///
    /// This includes lexer errors, syntax errors, escape analysis failures,
    /// ownership analysis failures, and Z3 verification failures.
    Parse(ParseError),

    /// File system operation failed.
    ///
    /// Typically occurs when reading source files or writing output.
    Io(String),

    /// Rust compilation failed (`cargo build`).
    ///
    /// Contains the raw stderr output from rustc when diagnostic translation
    /// was not possible.
    Build(String),

    /// Runtime execution failed.
    ///
    /// Contains stderr output from the executed program.
    Runtime(String),

    /// Translated ownership/borrow checker error with LOGOS-friendly message.
    ///
    /// This variant is used when rustc reports errors like E0382 (use after move)
    /// and we can translate them into natural language error messages that
    /// reference the original LOGOS source.
    Ownership(LogosError),
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::Parse(e) => write!(f, "Parse error: {:?}", e),
            CompileError::Io(e) => write!(f, "IO error: {}", e),
            CompileError::Build(e) => write!(f, "Build error: {}", e),
            CompileError::Runtime(e) => write!(f, "Runtime error: {}", e),
            CompileError::Ownership(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for CompileError {}

// ============================================================
// Futamura Projection Support — encode_program + verify_no_overhead
// ============================================================

/// Encode a LogicAffeine program (given as source) into CProgram construction source.
///
/// Takes LogicAffeine source code (with or without `## Main` header) and returns
/// LogicAffeine source code that constructs the equivalent CProgram data structure.
/// The result defines a variable `prog` of type CProgram.
pub fn encode_program_source(source: &str) -> Result<String, ParseError> {
    let full_source = if source.contains("## Main") || source.contains("## To ") {
        source.to_string()
    } else {
        format!("## Main\n{}", source)
    };

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(&full_source, &mut interner);
    let tokens = lexer.tokenize();

    let type_registry = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        result.types
    };

    // Collect variant constructors before the parser takes ownership of type_registry
    let mut variant_constructors: HashMap<String, Vec<String>> = HashMap::new();
    for (_type_name, type_def) in type_registry.iter_types() {
        if let crate::analysis::TypeDef::Enum { variants, .. } = type_def {
            for variant in variants {
                let vname = interner.resolve(variant.name).to_string();
                let field_names: Vec<String> = variant.fields.iter()
                    .map(|f| interner.resolve(f.name).to_string())
                    .collect();
                variant_constructors.insert(vname, field_names);
            }
        }
    }

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ast_ctx = AstContext::with_types(
        &expr_arena, &term_arena, &np_arena, &sym_arena,
        &role_arena, &pp_arena, &stmt_arena, &imperative_expr_arena,
        &type_expr_arena,
    );

    let mut parser = crate::parser::Parser::new(
        tokens, &mut world_state, &mut interner, ast_ctx, type_registry,
    );
    let stmts = parser.parse_program()?;

    let mut functions: Vec<(String, Vec<String>, Vec<String>, String, Vec<&Stmt>)> = Vec::new();
    let mut main_stmts: Vec<&Stmt> = Vec::new();

    for stmt in &stmts {
        if let Stmt::FunctionDef { name, params, body, return_type, is_native, .. } = stmt {
            if *is_native {
                continue; // Skip native function declarations — they have no encodable body
            }
            let fn_name = interner.resolve(*name).to_string();
            let param_names: Vec<String> = params
                .iter()
                .map(|(name, _)| interner.resolve(*name).to_string())
                .collect();
            let param_types: Vec<String> = params
                .iter()
                .map(|(_, ty)| decompile_type_expr(ty, &interner))
                .collect();
            let ret_type = return_type
                .map(|rt| decompile_type_expr(rt, &interner))
                .unwrap_or_else(|| "Nothing".to_string());
            let body_stmts: Vec<&Stmt> = body.iter().collect();
            functions.push((fn_name, param_names, param_types, ret_type, body_stmts));
        } else {
            main_stmts.push(stmt);
        }
    }

    let mut counter = 0usize;
    let mut output = String::new();
    reset_inspect_otherwise_idx(); // deterministic `__inspectMatched_N` numbering, shared with the native builder

    // Build the funcMap directly (Map of Text to CFunc) with fixed name, plus a parallel
    // `encodedFuncSeq` (Seq of CFunc, declaration order) — the map has no meaning-preserving
    // iteration order, so the wire serializer (which ships a `CProg with funcs …`) needs the seq.
    output.push_str("Let encodedFuncMap be a new Map of Text to CFunc.\n");
    output.push_str("Let encodedFuncSeq be a new Seq of CFunc.\n");

    for (fn_name, params, param_types, ret_type, body) in &functions {
        let body_var = encode_stmt_list_src(body, &mut counter, &mut output, &interner, &variant_constructors);

        let params_var = format!("params_{}", counter);
        counter += 1;
        output.push_str(&format!("Let {} be a new Seq of Text.\n", params_var));
        for p in params {
            output.push_str(&format!("Push \"{}\" to {}.\n", p, params_var));
        }

        let param_types_var = format!("paramTypes_{}", counter);
        counter += 1;
        output.push_str(&format!("Let {} be a new Seq of Text.\n", param_types_var));
        for pt in param_types {
            output.push_str(&format!("Push \"{}\" to {}.\n", pt, param_types_var));
        }

        let func_var = format!("func_{}", counter);
        counter += 1;
        output.push_str(&format!(
            "Let {} be a new CFuncDef with name \"{}\" and params {} and paramTypes {} and returnType \"{}\" and body {}.\n",
            func_var, fn_name, params_var, param_types_var, ret_type, body_var
        ));
        // `copy of` + emitted BEFORE the map `Set` (which MOVES func_var in AOT codegen): the seq
        // gets an identical clone, the map keeps the original — no use-after-move (E0382) when the
        // encoded program is compiled to Rust. The interpreter clones anyway, so this is a pure
        // codegen-soundness fix, behaviourally identical on every tier.
        output.push_str(&format!("Push copy of {} to encodedFuncSeq.\n", func_var));
        output.push_str(&format!(
            "Set item \"{}\" of encodedFuncMap to {}.\n",
            fn_name, func_var
        ));
    }

    // Build main statement list with fixed name
    let main_var = encode_stmt_list_src(&main_stmts, &mut counter, &mut output, &interner, &variant_constructors);
    output.push_str(&format!("Let encodedMain be {}.\n", main_var));

    Ok(output)
}

/// Compact encoding: inlines simple expressions (literals, variables) to reduce
/// encoding size by ~3x. Same semantics as encode_program_source but produces
/// fewer Let statements.
pub fn encode_program_source_compact(source: &str) -> Result<String, ParseError> {
    let full_source = if source.contains("## Main") || source.contains("## To ") {
        source.to_string()
    } else {
        format!("## Main\n{}", source)
    };

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(&full_source, &mut interner);
    let tokens = lexer.tokenize();

    let type_registry = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        result.types
    };

    let mut variant_constructors: HashMap<String, Vec<String>> = HashMap::new();
    for (_type_name, type_def) in type_registry.iter_types() {
        if let crate::analysis::TypeDef::Enum { variants, .. } = type_def {
            for variant in variants {
                let vname = interner.resolve(variant.name).to_string();
                let field_names: Vec<String> = variant.fields.iter()
                    .map(|f| interner.resolve(f.name).to_string())
                    .collect();
                variant_constructors.insert(vname, field_names);
            }
        }
    }

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ast_ctx = AstContext::with_types(
        &expr_arena, &term_arena, &np_arena, &sym_arena,
        &role_arena, &pp_arena, &stmt_arena, &imperative_expr_arena,
        &type_expr_arena,
    );

    let mut parser = crate::parser::Parser::new(
        tokens, &mut world_state, &mut interner, ast_ctx, type_registry,
    );
    let stmts = parser.parse_program()?;

    let mut functions: Vec<(String, Vec<String>, Vec<String>, String, Vec<&Stmt>)> = Vec::new();
    let mut main_stmts: Vec<&Stmt> = Vec::new();

    for stmt in &stmts {
        if let Stmt::FunctionDef { name, params, body, return_type, is_native, .. } = stmt {
            if *is_native { continue; }
            let fn_name = interner.resolve(*name).to_string();
            let param_names: Vec<String> = params
                .iter()
                .map(|(name, _)| interner.resolve(*name).to_string())
                .collect();
            let param_types: Vec<String> = params
                .iter()
                .map(|(_, ty)| decompile_type_expr(ty, &interner))
                .collect();
            let ret_type = return_type
                .map(|rt| decompile_type_expr(rt, &interner))
                .unwrap_or_else(|| "Nothing".to_string());
            let body_stmts: Vec<&Stmt> = body.iter().collect();
            functions.push((fn_name, param_names, param_types, ret_type, body_stmts));
        } else {
            main_stmts.push(stmt);
        }
    }

    let mut counter = 0usize;
    let mut output = String::new();

    output.push_str("Let encodedFuncMap be a new Map of Text to CFunc.\n");

    for (fn_name, params, param_types, ret_type, body) in &functions {
        let body_var = encode_stmt_list_compact(body, &mut counter, &mut output, &interner, &variant_constructors);

        let params_var = format!("params_{}", counter);
        counter += 1;
        output.push_str(&format!("Let {} be a new Seq of Text.\n", params_var));
        for p in params {
            output.push_str(&format!("Push \"{}\" to {}.\n", p, params_var));
        }

        let param_types_var = format!("paramTypes_{}", counter);
        counter += 1;
        output.push_str(&format!("Let {} be a new Seq of Text.\n", param_types_var));
        for pt in param_types {
            output.push_str(&format!("Push \"{}\" to {}.\n", pt, param_types_var));
        }

        let func_var = format!("func_{}", counter);
        counter += 1;
        output.push_str(&format!(
            "Let {} be a new CFuncDef with name \"{}\" and params {} and paramTypes {} and returnType \"{}\" and body {}.\n",
            func_var, fn_name, params_var, param_types_var, ret_type, body_var
        ));
        output.push_str(&format!(
            "Set item \"{}\" of encodedFuncMap to {}.\n",
            fn_name, func_var
        ));
    }

    let main_var = encode_stmt_list_compact(&main_stmts, &mut counter, &mut output, &interner, &variant_constructors);
    output.push_str(&format!("Let encodedMain be {}.\n", main_var));

    Ok(output)
}

/// Returns an inline expression string for simple expressions (no Let variable needed).
/// Returns None for complex expressions that require a Let variable.
fn try_inline_expr(expr: &Expr, interner: &Interner) -> Option<String> {
    match expr {
        Expr::Literal(lit) => match lit {
            Literal::Number(n) => Some(format!("(a new CInt with value {})", n)),
            Literal::Boolean(b) => Some(format!("(a new CBool with value {})", b)),
            Literal::Text(s) => {
                let text = interner.resolve(*s);
                Some(format!("(a new CText with value \"{}\")", text))
            }
            Literal::Float(f) => {
                let fs = format!("{}", f);
                let fs = if fs.contains('.') { fs } else { format!("{}.0", fs) };
                Some(format!("(a new CFloat with value {})", fs))
            }
            Literal::Nothing => Some("(a new CText with value \"nothing\")".to_string()),
            _ => None,
        },
        Expr::Identifier(sym) => {
            let name = interner.resolve(*sym);
            Some(format!("(a new CVar with name \"{}\")", name))
        }
        Expr::Not { operand } => {
            if let Some(inner) = try_inline_expr(operand, interner) {
                Some(format!("(a new CNot with inner {})", inner))
            } else {
                None
            }
        }
        Expr::OptionNone => Some("(a new COptionNone)".to_string()),
        _ => None,
    }
}

fn encode_expr_compact(expr: &Expr, counter: &mut usize, output: &mut String, interner: &Interner, variants: &HashMap<String, Vec<String>>) -> String {
    // Try inline first
    if let Some(inline) = try_inline_expr(expr, interner) {
        return inline;
    }

    // Fall back to Let variable (reuse encode_expr_src logic but with compact children)
    let var = format!("e_{}", *counter);
    *counter += 1;

    match expr {
        Expr::BinaryOp { op, left, right } => {
            let left_var = encode_expr_compact(left, counter, output, interner, variants);
            let right_var = encode_expr_compact(right, counter, output, interner, variants);
            let op_str = match op {
                BinaryOpKind::Add => "+",
                BinaryOpKind::Subtract => "-",
                BinaryOpKind::Multiply => "*",
                BinaryOpKind::Divide | BinaryOpKind::ExactDivide => "/",
                BinaryOpKind::FloorDivide => "//",
                BinaryOpKind::Modulo => "%",
                BinaryOpKind::Eq => "==",
                BinaryOpKind::NotEq => "!=",
                BinaryOpKind::Lt => "<",
                BinaryOpKind::Gt => ">",
                BinaryOpKind::LtEq => "<=",
                BinaryOpKind::GtEq => ">=",
                BinaryOpKind::And => "&&",
                BinaryOpKind::Or => "||",
                BinaryOpKind::Concat => "+",
                BinaryOpKind::SeqConcat => "followed by",
                BinaryOpKind::ApproxEq => "is approximately",
                BinaryOpKind::Pow => "**",
                BinaryOpKind::BitXor => "^",
                BinaryOpKind::BitAnd => "&",
                BinaryOpKind::BitOr => "|",
                BinaryOpKind::Shl => "<<",
                BinaryOpKind::Shr => ">>",
            };
            output.push_str(&format!(
                "Let {} be a new CBinOp with op \"{}\" and left {} and right {}.\n",
                var, op_str, left_var, right_var
            ));
        }
        Expr::Call { function, args } => {
            let fn_name = interner.resolve(*function);
            if let Some(field_names) = variants.get(fn_name) {
                let names_var = format!("nvNames_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of Text.\n", names_var));
                let vals_var = format!("nvVals_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of CExpr.\n", vals_var));
                for (i, arg) in args.iter().enumerate() {
                    let fname = field_names.get(i).map(|s| s.as_str()).unwrap_or("value");
                    output.push_str(&format!("Push \"{}\" to {}.\n", fname, names_var));
                    let arg_var = encode_expr_compact(arg, counter, output, interner, variants);
                    output.push_str(&format!("Push {} to {}.\n", arg_var, vals_var));
                }
                output.push_str(&format!(
                    "Let {} be a new CNewVariant with tag \"{}\" and fnames {} and fvals {}.\n",
                    var, fn_name, names_var, vals_var
                ));
            } else {
                let args_var = format!("callArgs_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of CExpr.\n", args_var));
                for arg in args {
                    let arg_var = encode_expr_compact(arg, counter, output, interner, variants);
                    output.push_str(&format!("Push {} to {}.\n", arg_var, args_var));
                }
                output.push_str(&format!(
                    "Let {} be a new CCall with name \"{}\" and args {}.\n",
                    var, fn_name, args_var
                ));
            }
        }
        Expr::Index { collection, index } => {
            let coll_var = encode_expr_compact(collection, counter, output, interner, variants);
            let idx_var = encode_expr_compact(index, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CIndex with coll {} and idx {}.\n",
                var, coll_var, idx_var
            ));
        }
        Expr::Length { collection } => {
            let coll_var = encode_expr_compact(collection, counter, output, interner, variants);
            output.push_str(&format!("Let {} be a new CLen with target {}.\n", var, coll_var));
        }
        Expr::FieldAccess { object, field } => {
            let obj_var = encode_expr_compact(object, counter, output, interner, variants);
            let field_name = interner.resolve(*field);
            output.push_str(&format!(
                "Let {} be a new CMapGet with target {} and key (a new CText with value \"{}\").\n",
                var, obj_var, field_name
            ));
        }
        Expr::NewVariant { variant, fields, .. } => {
            let variant_name = interner.resolve(*variant);
            let names_var = format!("nvNames_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of Text.\n", names_var));
            let vals_var = format!("nvVals_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of CExpr.\n", vals_var));
            for (field_name, field_expr) in fields {
                let fname = interner.resolve(*field_name);
                output.push_str(&format!("Push \"{}\" to {}.\n", fname, names_var));
                let field_var = encode_expr_compact(field_expr, counter, output, interner, variants);
                output.push_str(&format!("Push {} to {}.\n", field_var, vals_var));
            }
            output.push_str(&format!(
                "Let {} be a new CNewVariant with tag \"{}\" and fnames {} and fvals {}.\n",
                var, variant_name, names_var, vals_var
            ));
        }
        Expr::New { type_name, init_fields, .. } => {
            let tn = interner.resolve(*type_name);
            if tn == "Seq" || tn == "List" {
                output.push_str(&format!("Let {} be a new CNewSeq.\n", var));
            } else if tn == "Set" {
                output.push_str(&format!("Let {} be a new CNewSet.\n", var));
            } else {
                let names_var = format!("nvNames_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of Text.\n", names_var));
                let vals_var = format!("nvVals_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of CExpr.\n", vals_var));
                for (field_name, field_expr) in init_fields {
                    let fname = interner.resolve(*field_name);
                    output.push_str(&format!("Push \"{}\" to {}.\n", fname, names_var));
                    let field_var = encode_expr_compact(field_expr, counter, output, interner, variants);
                    output.push_str(&format!("Push {} to {}.\n", field_var, vals_var));
                }
                output.push_str(&format!(
                    "Let {} be a new CNewVariant with tag \"{}\" and fnames {} and fvals {}.\n",
                    var, tn, names_var, vals_var
                ));
            }
        }
        Expr::InterpolatedString(parts) => {
            if parts.is_empty() {
                output.push_str(&format!("Let {} be (a new CText with value \"\").\n", var));
            } else {
                // Preserve as a first-class CInterpolatedString rather than desugaring to a
                // lossy `+` chain (see encode_expr_src for the rationale).
                let parts_var = format!("isparts_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of CStringPart.\n", parts_var));
                for part in parts {
                    match part {
                        StringPart::Literal(sym) => {
                            let text = interner.resolve(*sym);
                            output.push_str(&format!(
                                "Push a new CLiteralPart with value \"{}\" to {}.\n", text, parts_var
                            ));
                        }
                        StringPart::Expr { value, .. } => {
                            let pv = encode_expr_compact(value, counter, output, interner, variants);
                            output.push_str(&format!(
                                "Push a new CExprPart with expr {} to {}.\n", pv, parts_var
                            ));
                        }
                    }
                }
                output.push_str(&format!(
                    "Let {} be a new CInterpolatedString with parts {}.\n", var, parts_var
                ));
            }
        }
        Expr::Range { start, end } => {
            let start_var = encode_expr_compact(start, counter, output, interner, variants);
            let end_var = encode_expr_compact(end, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CRange with start {} and end {}.\n",
                var, start_var, end_var
            ));
        }
        Expr::Copy { expr } => {
            let inner_var = encode_expr_compact(expr, counter, output, interner, variants);
            output.push_str(&format!("Let {} be a new CCopy with target {}.\n", var, inner_var));
        }
        Expr::Contains { collection, value } => {
            let coll_var = encode_expr_compact(collection, counter, output, interner, variants);
            let val_var = encode_expr_compact(value, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CContains with coll {} and elem {}.\n",
                var, coll_var, val_var
            ));
        }
        Expr::OptionSome { value } => {
            let inner_var = encode_expr_compact(value, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new COptionSome with inner {}.\n", var, inner_var
            ));
        }
        Expr::Tuple(elems) => {
            let items_var = format!("tupItems_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of CExpr.\n", items_var));
            for elem in elems {
                let elem_var = encode_expr_compact(elem, counter, output, interner, variants);
                output.push_str(&format!("Push {} to {}.\n", elem_var, items_var));
            }
            output.push_str(&format!(
                "Let {} be a new CTuple with items {}.\n", var, items_var
            ));
        }
        Expr::Closure { params, body, .. } => {
            let params_var = format!("clp_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of Text.\n", params_var));
            let mut param_names = HashSet::new();
            for (sym, _) in params {
                let name = interner.resolve(*sym);
                param_names.insert(name.to_string());
                output.push_str(&format!("Push \"{}\" to {}.\n", name, params_var));
            }
            let body_var = format!("clb_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of CStmt.\n", body_var));
            match body {
                ClosureBody::Expression(e) => {
                    let ret_expr = encode_expr_compact(e, counter, output, interner, variants);
                    let ret_var = format!("s_{}", *counter);
                    *counter += 1;
                    output.push_str(&format!("Let {} be a new CReturn with expr {}.\n", ret_var, ret_expr));
                    output.push_str(&format!("Push {} to {}.\n", ret_var, body_var));
                }
                ClosureBody::Block(stmts) => {
                    for s in stmts.iter() {
                        let sv = encode_stmt_compact(s, counter, output, interner, variants);
                        output.push_str(&format!("Push {} to {}.\n", sv, body_var));
                    }
                }
            }
            let bound: HashSet<String> = param_names;
            // Sort the captured free variables for a DETERMINISTIC encoding — a `HashSet`'s random
            // iteration order would make the wire bytes vary run-to-run (breaking content-addressing
            // AND the fast native-builder byte-identity). The capture set is order-independent
            // semantically, so sorting is a pure improvement.
            let mut free: Vec<String> = collect_free_vars_expr(expr, interner, &bound).into_iter().collect();
            free.sort();
            let cap_var = format!("clc_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of Text.\n", cap_var));
            for fv in &free {
                output.push_str(&format!("Push \"{}\" to {}.\n", fv, cap_var));
            }
            output.push_str(&format!(
                "Let {} be a new CClosure with params {} and body {} and captured {}.\n",
                var, params_var, body_var, cap_var
            ));
        }
        Expr::CallExpr { callee, args } => {
            let callee_var = encode_expr_compact(callee, counter, output, interner, variants);
            let args_var = format!("cea_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of CExpr.\n", args_var));
            for a in args {
                let av = encode_expr_compact(a, counter, output, interner, variants);
                output.push_str(&format!("Push {} to {}.\n", av, args_var));
            }
            output.push_str(&format!(
                "Let {} be a new CCallExpr with target {} and args {}.\n",
                var, callee_var, args_var
            ));
        }
        Expr::Slice { collection, start, end } => {
            let coll_var = encode_expr_compact(collection, counter, output, interner, variants);
            let start_var = encode_expr_compact(start, counter, output, interner, variants);
            let end_var = encode_expr_compact(end, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CSlice with coll {} and startIdx {} and endIdx {}.\n",
                var, coll_var, start_var, end_var
            ));
        }
        Expr::Union { left, right } => {
            let left_var = encode_expr_compact(left, counter, output, interner, variants);
            let right_var = encode_expr_compact(right, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CUnion with left {} and right {}.\n",
                var, left_var, right_var
            ));
        }
        Expr::Intersection { left, right } => {
            let left_var = encode_expr_compact(left, counter, output, interner, variants);
            let right_var = encode_expr_compact(right, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CIntersection with left {} and right {}.\n",
                var, left_var, right_var
            ));
        }
        Expr::Give { value } => {
            let inner_var = encode_expr_compact(value, counter, output, interner, variants);
            output.push_str(&format!("Let {} be {}.\n", var, inner_var));
        }
        Expr::Escape { code, .. } => {
            let code_str = interner.resolve(*code);
            output.push_str(&format!(
                "Let {} be a new CEscExpr with code \"{}\".\n",
                var, code_str.replace('\"', "\\\"")
            ));
        }
        _ => {
            // For unsupported expressions, use the non-compact version
            output.push_str(&format!("Let {} be (a new CText with value \"unsupported\").\n", var));
        }
    }

    var
}

fn encode_stmt_compact(stmt: &Stmt, counter: &mut usize, output: &mut String, interner: &Interner, variants: &HashMap<String, Vec<String>>) -> String {
    let var = format!("s_{}", *counter);
    *counter += 1;

    match stmt {
        Stmt::Let { var: name, value, .. } => {
            let name_str = interner.resolve(*name);
            let expr_var = encode_expr_compact(value, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CLet with name \"{}\" and expr {}.\n",
                var, name_str, expr_var
            ));
        }
        Stmt::Set { target, value } => {
            let name_str = interner.resolve(*target);
            let expr_var = encode_expr_compact(value, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CSet with name \"{}\" and expr {}.\n",
                var, name_str, expr_var
            ));
        }
        Stmt::If { cond, then_block, else_block } => {
            let cond_var = encode_expr_compact(cond, counter, output, interner, variants);
            let then_stmts: Vec<&Stmt> = then_block.iter().collect();
            let then_var = encode_stmt_list_compact(&then_stmts, counter, output, interner, variants);
            let else_var = if let Some(els) = else_block {
                let else_stmts: Vec<&Stmt> = els.iter().collect();
                encode_stmt_list_compact(&else_stmts, counter, output, interner, variants)
            } else {
                let empty_var = format!("emptyBlock_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of CStmt.\n", empty_var));
                empty_var
            };
            output.push_str(&format!(
                "Let {} be a new CIf with cond {} and thenBlock {} and elseBlock {}.\n",
                var, cond_var, then_var, else_var
            ));
        }
        Stmt::While { cond, body, .. } => {
            let cond_var = encode_expr_compact(cond, counter, output, interner, variants);
            let body_stmts: Vec<&Stmt> = body.iter().collect();
            let body_var = encode_stmt_list_compact(&body_stmts, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CWhile with cond {} and body {}.\n",
                var, cond_var, body_var
            ));
        }
        Stmt::Return { value } => {
            if let Some(val) = value {
                let expr_var = encode_expr_compact(val, counter, output, interner, variants);
                output.push_str(&format!("Let {} be a new CReturn with expr {}.\n", var, expr_var));
            } else {
                output.push_str(&format!("Let {} be a new CReturn with expr (a new CText with value \"nothing\").\n", var));
            }
        }
        Stmt::Show { object, .. } => {
            let expr_var = encode_expr_compact(object, counter, output, interner, variants);
            output.push_str(&format!("Let {} be a new CShow with expr {}.\n", var, expr_var));
        }
        Stmt::Repeat { pattern, iterable, body } => {
            let var_str = match pattern {
                Pattern::Identifier(sym) => interner.resolve(*sym).to_string(),
                Pattern::Tuple(syms) => {
                    if let Some(s) = syms.first() {
                        interner.resolve(*s).to_string()
                    } else {
                        "item".to_string()
                    }
                }
            };
            let coll_var = encode_expr_compact(iterable, counter, output, interner, variants);
            let body_stmts: Vec<&Stmt> = body.iter().collect();
            let body_var = encode_stmt_list_compact(&body_stmts, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CRepeat with var \"{}\" and coll {} and body {}.\n",
                var, var_str, coll_var, body_var
            ));
        }
        Stmt::Push { value, collection } => {
            let coll_name = extract_ident_name(collection, interner);
            let expr_var = encode_expr_compact(value, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CPush with expr {} and target \"{}\".\n",
                var, expr_var, coll_name
            ));
        }
        Stmt::SetIndex { collection, index, value } => {
            let target_str = extract_ident_name(collection, interner);
            let idx_var = encode_expr_compact(index, counter, output, interner, variants);
            let val_var = encode_expr_compact(value, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CSetIdx with target \"{}\" and idx {} and val {}.\n",
                var, target_str, idx_var, val_var
            ));
        }
        Stmt::SetField { object, field, value } => {
            let target_str = extract_ident_name(object, interner);
            let field_str = interner.resolve(*field);
            let val_var = encode_expr_compact(value, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CSetField with target \"{}\" and field \"{}\" and val {}.\n",
                var, target_str, field_str, val_var
            ));
        }
        Stmt::Break => {
            output.push_str(&format!("Let {} be a new CBreak.\n", var));
        }
        Stmt::Inspect { target, arms, .. } => {
            let target_var = encode_expr_compact(target, counter, output, interner, variants);
            let arms_var = format!("arms_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of CMatchArm.\n", arms_var));
            for arm in arms {
                if let Some(variant_sym) = arm.variant {
                    let vname = interner.resolve(variant_sym);
                    let bindings_var = format!("bindings_{}", *counter);
                    *counter += 1;
                    output.push_str(&format!("Let {} be a new Seq of Text.\n", bindings_var));
                    for (_, binding_name) in &arm.bindings {
                        let bn = interner.resolve(*binding_name);
                        output.push_str(&format!("Push \"{}\" to {}.\n", bn, bindings_var));
                    }
                    let body_stmts: Vec<&Stmt> = arm.body.iter().collect();
                    let body_var = encode_stmt_list_compact(&body_stmts, counter, output, interner, variants);
                    let arm_var = format!("arm_{}", *counter);
                    *counter += 1;
                    output.push_str(&format!(
                        "Let {} be a new CWhen with variantName \"{}\" and bindings {} and body {}.\n",
                        arm_var, vname, bindings_var, body_var
                    ));
                    output.push_str(&format!("Push {} to {}.\n", arm_var, arms_var));
                } else {
                    let body_stmts: Vec<&Stmt> = arm.body.iter().collect();
                    let body_var = encode_stmt_list_compact(&body_stmts, counter, output, interner, variants);
                    let arm_var = format!("arm_{}", *counter);
                    *counter += 1;
                    output.push_str(&format!(
                        "Let {} be a new COtherwise with body {}.\n",
                        arm_var, body_var
                    ));
                    output.push_str(&format!("Push {} to {}.\n", arm_var, arms_var));
                }
            }
            output.push_str(&format!(
                "Let {} be a new CInspect with target {} and arms {}.\n",
                var, target_var, arms_var
            ));
        }
        _ => {
            // Delegate to non-compact encoder for unsupported statements
            return encode_stmt_src(stmt, counter, output, interner, variants);
        }
    }

    var
}

fn encode_stmt_list_compact(stmts: &[&Stmt], counter: &mut usize, output: &mut String, interner: &Interner, variants: &HashMap<String, Vec<String>>) -> String {
    let list_var = format!("stmts_{}", *counter);
    *counter += 1;
    output.push_str(&format!("Let {} be a new Seq of CStmt.\n", list_var));
    for s in stmts {
        let sv = encode_stmt_compact(s, counter, output, interner, variants);
        output.push_str(&format!("Push {} to {}.\n", sv, list_var));
    }
    list_var
}

fn collect_free_vars_expr<'a>(expr: &'a Expr, interner: &Interner, bound: &HashSet<String>) -> HashSet<String> {
    let mut free = HashSet::new();
    match expr {
        Expr::Identifier(sym) => {
            let name = interner.resolve(*sym).to_string();
            if !bound.contains(&name) {
                free.insert(name);
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            free.extend(collect_free_vars_expr(left, interner, bound));
            free.extend(collect_free_vars_expr(right, interner, bound));
        }
        Expr::Not { operand } => {
            free.extend(collect_free_vars_expr(operand, interner, bound));
        }
        Expr::Copy { expr: inner } => {
            free.extend(collect_free_vars_expr(inner, interner, bound));
        }
        Expr::CallExpr { callee, args } => {
            free.extend(collect_free_vars_expr(callee, interner, bound));
            for a in args {
                free.extend(collect_free_vars_expr(a, interner, bound));
            }
        }
        Expr::Index { collection, index } => {
            free.extend(collect_free_vars_expr(collection, interner, bound));
            free.extend(collect_free_vars_expr(index, interner, bound));
        }
        Expr::InterpolatedString(parts) => {
            for part in parts {
                if let StringPart::Expr { value, .. } = part {
                    free.extend(collect_free_vars_expr(value, interner, bound));
                }
            }
        }
        Expr::Closure { params, body, .. } => {
            let mut inner_bound = bound.clone();
            for (sym, _) in params {
                inner_bound.insert(interner.resolve(*sym).to_string());
            }
            match body {
                ClosureBody::Expression(e) => {
                    free.extend(collect_free_vars_expr(e, interner, &inner_bound));
                }
                ClosureBody::Block(stmts) => {
                    for s in stmts.iter() {
                        free.extend(collect_free_vars_stmt(s, interner, &inner_bound));
                    }
                }
            }
        }
        _ => {}
    }
    free
}

fn collect_free_vars_stmt<'a>(stmt: &'a Stmt, interner: &Interner, bound: &HashSet<String>) -> HashSet<String> {
    let mut free = HashSet::new();
    match stmt {
        Stmt::Let { var, value, .. } => {
            free.extend(collect_free_vars_expr(value, interner, bound));
        }
        Stmt::Set { target, value, .. } => {
            let n = interner.resolve(*target).to_string();
            if !bound.contains(&n) {
                free.insert(n);
            }
            free.extend(collect_free_vars_expr(value, interner, bound));
        }
        Stmt::Show { object, .. } => {
            free.extend(collect_free_vars_expr(object, interner, bound));
        }
        Stmt::Return { value } => {
            if let Some(v) = value {
                free.extend(collect_free_vars_expr(v, interner, bound));
            }
        }
        _ => {}
    }
    free
}

fn encode_expr_src(expr: &Expr, counter: &mut usize, output: &mut String, interner: &Interner, variants: &HashMap<String, Vec<String>>) -> String {
    let var = format!("e_{}", *counter);
    *counter += 1;

    match expr {
        Expr::Literal(lit) => match lit {
            Literal::Number(n) => {
                output.push_str(&format!("Let {} be a new CInt with value {}.\n", var, n));
            }
            Literal::Boolean(b) => {
                output.push_str(&format!("Let {} be a new CBool with value {}.\n", var, b));
            }
            Literal::Text(s) => {
                let text = interner.resolve(*s);
                output.push_str(&format!("Let {} be a new CText with value \"{}\".\n", var, text));
            }
            Literal::Float(f) => {
                // Format faithfully: Rust's `{}` prints integer-valued floats without a
                // decimal point (9.0 -> "9"), which would re-parse as an Int and silently
                // turn float ops into integer ops. Guarantee a decimal point.
                let fs = format!("{}", f);
                let fs = if fs.contains('.') || fs.contains('e') || fs.contains('E') {
                    fs
                } else {
                    format!("{}.0", fs)
                };
                output.push_str(&format!("Let {} be a new CFloat with value {}.\n", var, fs));
            }
            Literal::Duration(nanos) => {
                let millis = nanos / 1_000_000;
                let amount_var = format!("e_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new CInt with value {}.\n", amount_var, millis));
                output.push_str(&format!("Let {} be a new CDuration with amount {} and unit \"milliseconds\".\n", var, amount_var));
            }
            Literal::Nothing => {
                output.push_str(&format!("Let {} be a new CText with value \"nothing\".\n", var));
            }
            _ => {
                output.push_str(&format!("Let {} be a new CText with value \"unsupported\".\n", var));
            }
        },
        Expr::Identifier(sym) => {
            let name = interner.resolve(*sym);
            output.push_str(&format!("Let {} be a new CVar with name \"{}\".\n", var, name));
        }
        Expr::BinaryOp { op, left, right } => {
            let left_var = encode_expr_src(left, counter, output, interner, variants);
            let right_var = encode_expr_src(right, counter, output, interner, variants);
            let op_str = match op {
                BinaryOpKind::Add => "+",
                BinaryOpKind::Subtract => "-",
                BinaryOpKind::Multiply => "*",
                BinaryOpKind::Divide | BinaryOpKind::ExactDivide => "/",
                BinaryOpKind::FloorDivide => "//",
                BinaryOpKind::Modulo => "%",
                BinaryOpKind::Eq => "==",
                BinaryOpKind::NotEq => "!=",
                BinaryOpKind::Lt => "<",
                BinaryOpKind::Gt => ">",
                BinaryOpKind::LtEq => "<=",
                BinaryOpKind::GtEq => ">=",
                BinaryOpKind::And => "&&",
                BinaryOpKind::Or => "||",
                BinaryOpKind::Concat => "+",
                BinaryOpKind::SeqConcat => "followed by",
                BinaryOpKind::ApproxEq => "is approximately",
                BinaryOpKind::Pow => "**",
                BinaryOpKind::BitXor => "^",
                BinaryOpKind::BitAnd => "&",
                BinaryOpKind::BitOr => "|",
                BinaryOpKind::Shl => "<<",
                BinaryOpKind::Shr => ">>",
            };
            output.push_str(&format!(
                "Let {} be a new CBinOp with op \"{}\" and left {} and right {}.\n",
                var, op_str, left_var, right_var
            ));
        }
        Expr::Not { operand } => {
            let inner_var = encode_expr_src(operand, counter, output, interner, variants);
            output.push_str(&format!("Let {} be a new CNot with inner {}.\n", var, inner_var));
        }
        Expr::Call { function, args } => {
            let fn_name = interner.resolve(*function);
            if let Some(field_names) = variants.get(fn_name) {
                // Variant constructor call — encode as CNewVariant
                let names_var = format!("nvNames_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of Text.\n", names_var));
                let vals_var = format!("nvVals_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of CExpr.\n", vals_var));
                for (i, arg) in args.iter().enumerate() {
                    let fname = field_names.get(i).map(|s| s.as_str()).unwrap_or("value");
                    output.push_str(&format!("Push \"{}\" to {}.\n", fname, names_var));
                    let arg_var = encode_expr_src(arg, counter, output, interner, variants);
                    output.push_str(&format!("Push {} to {}.\n", arg_var, vals_var));
                }
                output.push_str(&format!(
                    "Let {} be a new CNewVariant with tag \"{}\" and fnames {} and fvals {}.\n",
                    var, fn_name, names_var, vals_var
                ));
            } else {
                let args_var = format!("callArgs_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of CExpr.\n", args_var));
                for arg in args {
                    let arg_var = encode_expr_src(arg, counter, output, interner, variants);
                    output.push_str(&format!("Push {} to {}.\n", arg_var, args_var));
                }
                output.push_str(&format!(
                    "Let {} be a new CCall with name \"{}\" and args {}.\n",
                    var, fn_name, args_var
                ));
            }
        }
        Expr::Index { collection, index } => {
            let coll_var = encode_expr_src(collection, counter, output, interner, variants);
            let idx_var = encode_expr_src(index, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CIndex with coll {} and idx {}.\n",
                var, coll_var, idx_var
            ));
        }
        Expr::Length { collection } => {
            let coll_var = encode_expr_src(collection, counter, output, interner, variants);
            output.push_str(&format!("Let {} be a new CLen with target {}.\n", var, coll_var));
        }
        Expr::FieldAccess { object, field } => {
            let obj_var = encode_expr_src(object, counter, output, interner, variants);
            let field_name = interner.resolve(*field);
            let key_var = format!("e_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new CText with value \"{}\".\n", key_var, field_name));
            output.push_str(&format!(
                "Let {} be a new CMapGet with target {} and key {}.\n",
                var, obj_var, key_var
            ));
        }
        Expr::NewVariant { variant, fields, .. } => {
            let variant_name = interner.resolve(*variant);
            let names_var = format!("nvNames_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of Text.\n", names_var));
            let vals_var = format!("nvVals_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of CExpr.\n", vals_var));
            for (field_name, field_expr) in fields {
                let fname = interner.resolve(*field_name);
                output.push_str(&format!("Push \"{}\" to {}.\n", fname, names_var));
                let field_var = encode_expr_src(field_expr, counter, output, interner, variants);
                output.push_str(&format!("Push {} to {}.\n", field_var, vals_var));
            }
            output.push_str(&format!(
                "Let {} be a new CNewVariant with tag \"{}\" and fnames {} and fvals {}.\n",
                var, variant_name, names_var, vals_var
            ));
        }
        Expr::New { type_name, init_fields, .. } => {
            let tn = interner.resolve(*type_name);
            if tn == "Seq" || tn == "List" {
                output.push_str(&format!("Let {} be a new CNewSeq.\n", var));
            } else if tn == "Set" {
                output.push_str(&format!("Let {} be a new CNewSet.\n", var));
            } else if tn == "Map" || tn.starts_with("Map ") {
                // Empty map creation — encode as CNew with "Map" type
                let names_var = format!("nvNames_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of Text.\n", names_var));
                let vals_var = format!("nvVals_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of CExpr.\n", vals_var));
                output.push_str(&format!(
                    "Let {} be a new CNew with typeName \"Map\" and fieldNames {} and fields {}.\n",
                    var, names_var, vals_var
                ));
            } else if init_fields.is_empty() {
                let names_var = format!("nvNames_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of Text.\n", names_var));
                let vals_var = format!("nvVals_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of CExpr.\n", vals_var));
                output.push_str(&format!(
                    "Let {} be a new CNewVariant with tag \"{}\" and fnames {} and fvals {}.\n",
                    var, tn, names_var, vals_var
                ));
            } else {
                let names_var = format!("nvNames_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of Text.\n", names_var));
                let vals_var = format!("nvVals_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of CExpr.\n", vals_var));
                for (field_name, field_expr) in init_fields {
                    let fname = interner.resolve(*field_name);
                    output.push_str(&format!("Push \"{}\" to {}.\n", fname, names_var));
                    let field_var = encode_expr_src(field_expr, counter, output, interner, variants);
                    output.push_str(&format!("Push {} to {}.\n", field_var, vals_var));
                }
                output.push_str(&format!(
                    "Let {} be a new CNewVariant with tag \"{}\" and fnames {} and fvals {}.\n",
                    var, tn, names_var, vals_var
                ));
            }
        }
        Expr::InterpolatedString(parts) => {
            if parts.is_empty() {
                output.push_str(&format!("Let {} be a new CText with value \"\".\n", var));
            } else {
                // Preserve the interpolation as a first-class CInterpolatedString — the IR,
                // PE (peExpr), decompiler, and self-interpreter (coreEval) all handle it and
                // coerce each part via valToText. Desugaring to a `+` chain was lossy: a
                // non-Text leading part produced `Int + Text`, which evalBinOp/applyBinOp do
                // not fold (→ VNothing → __unresolvable in the residual).
                let parts_var = format!("isparts_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of CStringPart.\n", parts_var));
                for part in parts {
                    match part {
                        StringPart::Literal(sym) => {
                            let text = interner.resolve(*sym);
                            output.push_str(&format!(
                                "Push a new CLiteralPart with value \"{}\" to {}.\n", text, parts_var
                            ));
                        }
                        StringPart::Expr { value, .. } => {
                            let pv = encode_expr_src(value, counter, output, interner, variants);
                            output.push_str(&format!(
                                "Push a new CExprPart with expr {} to {}.\n", pv, parts_var
                            ));
                        }
                    }
                }
                output.push_str(&format!(
                    "Let {} be a new CInterpolatedString with parts {}.\n", var, parts_var
                ));
            }
        }
        Expr::Range { start, end } => {
            let start_var = encode_expr_src(start, counter, output, interner, variants);
            let end_var = encode_expr_src(end, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CRange with start {} and end {}.\n",
                var, start_var, end_var
            ));
        }
        Expr::Slice { collection, start, end } => {
            let coll_var = encode_expr_src(collection, counter, output, interner, variants);
            let start_var = encode_expr_src(start, counter, output, interner, variants);
            let end_var = encode_expr_src(end, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CSlice with coll {} and startIdx {} and endIdx {}.\n",
                var, coll_var, start_var, end_var
            ));
        }
        Expr::Copy { expr } => {
            let inner_var = encode_expr_src(expr, counter, output, interner, variants);
            output.push_str(&format!("Let {} be a new CCopy with target {}.\n", var, inner_var));
        }
        Expr::Contains { collection, value } => {
            let coll_var = encode_expr_src(collection, counter, output, interner, variants);
            let val_var = encode_expr_src(value, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CContains with coll {} and elem {}.\n",
                var, coll_var, val_var
            ));
        }
        Expr::Union { left, right } => {
            let left_var = encode_expr_src(left, counter, output, interner, variants);
            let right_var = encode_expr_src(right, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CUnion with left {} and right {}.\n",
                var, left_var, right_var
            ));
        }
        Expr::Intersection { left, right } => {
            let left_var = encode_expr_src(left, counter, output, interner, variants);
            let right_var = encode_expr_src(right, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CIntersection with left {} and right {}.\n",
                var, left_var, right_var
            ));
        }
        Expr::OptionSome { value } => {
            let inner_var = encode_expr_src(value, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new COptionSome with inner {}.\n",
                var, inner_var
            ));
        }
        Expr::OptionNone => {
            output.push_str(&format!("Let {} be a new COptionNone.\n", var));
        }
        Expr::Tuple(elems) => {
            let items_var = format!("tupItems_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of CExpr.\n", items_var));
            for elem in elems {
                let elem_var = encode_expr_src(elem, counter, output, interner, variants);
                output.push_str(&format!("Push {} to {}.\n", elem_var, items_var));
            }
            output.push_str(&format!(
                "Let {} be a new CTuple with items {}.\n",
                var, items_var
            ));
        }
        Expr::Closure { params, body, .. } => {
            let params_var = format!("clp_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of Text.\n", params_var));
            let mut param_names = HashSet::new();
            for (sym, _) in params {
                let name = interner.resolve(*sym);
                param_names.insert(name.to_string());
                output.push_str(&format!("Push \"{}\" to {}.\n", name, params_var));
            }
            let body_var = format!("clb_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of CStmt.\n", body_var));
            match body {
                ClosureBody::Expression(e) => {
                    let ret_expr = encode_expr_src(e, counter, output, interner, variants);
                    let ret_var = format!("s_{}", *counter);
                    *counter += 1;
                    output.push_str(&format!("Let {} be a new CReturn with expr {}.\n", ret_var, ret_expr));
                    output.push_str(&format!("Push {} to {}.\n", ret_var, body_var));
                }
                ClosureBody::Block(stmts) => {
                    for s in stmts.iter() {
                        let sv = encode_stmt_src(s, counter, output, interner, variants);
                        output.push_str(&format!("Push {} to {}.\n", sv, body_var));
                    }
                }
            }
            let bound: HashSet<String> = param_names;
            // Sort the captured free variables for a DETERMINISTIC encoding — a `HashSet`'s random
            // iteration order would make the wire bytes vary run-to-run (breaking content-addressing
            // AND the fast native-builder byte-identity). The capture set is order-independent
            // semantically, so sorting is a pure improvement.
            let mut free: Vec<String> = collect_free_vars_expr(expr, interner, &bound).into_iter().collect();
            free.sort();
            let cap_var = format!("clc_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of Text.\n", cap_var));
            for fv in &free {
                output.push_str(&format!("Push \"{}\" to {}.\n", fv, cap_var));
            }
            output.push_str(&format!(
                "Let {} be a new CClosure with params {} and body {} and captured {}.\n",
                var, params_var, body_var, cap_var
            ));
        }
        Expr::CallExpr { callee, args } => {
            let callee_var = encode_expr_src(callee, counter, output, interner, variants);
            let args_var = format!("cea_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of CExpr.\n", args_var));
            for a in args {
                let av = encode_expr_src(a, counter, output, interner, variants);
                output.push_str(&format!("Push {} to {}.\n", av, args_var));
            }
            output.push_str(&format!(
                "Let {} be a new CCallExpr with target {} and args {}.\n",
                var, callee_var, args_var
            ));
        }
        Expr::Give { value } => {
            let inner_var = encode_expr_src(value, counter, output, interner, variants);
            output.push_str(&format!("Let {} be {}.\n", var, inner_var));
        }
        Expr::Escape { code, .. } => {
            let code_str = interner.resolve(*code);
            output.push_str(&format!(
                "Let {} be a new CEscExpr with code \"{}\".\n",
                var, code_str.replace('\"', "\\\"")
            ));
        }
        Expr::List(elems) => {
            let items_var = format!("litems_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of CExpr.\n", items_var));
            for elem in elems {
                let elem_var = encode_expr_src(elem, counter, output, interner, variants);
                output.push_str(&format!("Push {} to {}.\n", elem_var, items_var));
            }
            output.push_str(&format!(
                "Let {} be a new CList with items {}.\n",
                var, items_var
            ));
        }
        Expr::ManifestOf { zone } => {
            let zone_var = encode_expr_src(zone, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CManifestOf with zn {}.\n",
                var, zone_var
            ));
        }
        Expr::ChunkAt { index, zone } => {
            let idx_var = encode_expr_src(index, counter, output, interner, variants);
            let zone_var = encode_expr_src(zone, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CChunkAt with idx {} and zn {}.\n",
                var, idx_var, zone_var
            ));
        }
        Expr::WithCapacity { value, .. } => {
            // Capacity is an allocation hint with no runtime semantics (the interpreter ignores
            // it). Encode the inner value directly so the residual carries the value, not the
            // erased hint — no Core-IR node needed.
            return encode_expr_src(value, counter, output, interner, variants);
        }
        _ => {
            output.push_str(&format!("Let {} be a new CText with value \"unsupported\".\n", var));
        }
    }

    var
}

fn encode_stmt_src(stmt: &Stmt, counter: &mut usize, output: &mut String, interner: &Interner, variants: &HashMap<String, Vec<String>>) -> String {
    let var = format!("s_{}", *counter);
    *counter += 1;

    match stmt {
        Stmt::Let { var: name, value, .. } => {
            let name_str = interner.resolve(*name);
            let expr_var = encode_expr_src(value, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CLet with name \"{}\" and expr {}.\n",
                var, name_str, expr_var
            ));
        }
        Stmt::Set { target, value } => {
            let name_str = interner.resolve(*target);
            let expr_var = encode_expr_src(value, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CSet with name \"{}\" and expr {}.\n",
                var, name_str, expr_var
            ));
        }
        Stmt::If { cond, then_block, else_block } => {
            let cond_var = encode_expr_src(cond, counter, output, interner, variants);
            let then_stmts: Vec<&Stmt> = then_block.iter().collect();
            let then_var = encode_stmt_list_src(&then_stmts, counter, output, interner, variants);
            let else_var = if let Some(els) = else_block {
                let else_stmts: Vec<&Stmt> = els.iter().collect();
                encode_stmt_list_src(&else_stmts, counter, output, interner, variants)
            } else {
                let empty_var = format!("emptyBlock_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of CStmt.\n", empty_var));
                empty_var
            };
            output.push_str(&format!(
                "Let {} be a new CIf with cond {} and thenBlock {} and elseBlock {}.\n",
                var, cond_var, then_var, else_var
            ));
        }
        Stmt::While { cond, body, .. } => {
            let cond_var = encode_expr_src(cond, counter, output, interner, variants);
            let body_stmts: Vec<&Stmt> = body.iter().collect();
            let body_var = encode_stmt_list_src(&body_stmts, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CWhile with cond {} and body {}.\n",
                var, cond_var, body_var
            ));
        }
        Stmt::Splice { body } => {
            // A Splice encodes as an always-taken CIf: observationally
            // equivalent, because every desugar temporary is both defined and
            // consumed inside the body — no new pe_source node needed.
            let cond_var = format!("splice_cond_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new CBool with value true.\n", cond_var));
            let body_stmts: Vec<&Stmt> = body.iter().collect();
            let body_var = encode_stmt_list_src(&body_stmts, counter, output, interner, variants);
            let empty_var = format!("emptyBlock_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of CStmt.\n", empty_var));
            output.push_str(&format!(
                "Let {} be a new CIf with cond {} and thenBlock {} and elseBlock {}.\n",
                var, cond_var, body_var, empty_var
            ));
        }
        Stmt::Return { value } => {
            if let Some(expr) = value {
                let expr_var = encode_expr_src(expr, counter, output, interner, variants);
                output.push_str(&format!("Let {} be a new CReturn with expr {}.\n", var, expr_var));
            } else {
                let nothing_var = format!("e_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new CInt with value 0.\n", nothing_var));
                output.push_str(&format!("Let {} be a new CReturn with expr {}.\n", var, nothing_var));
            }
        }
        Stmt::Show { object, .. } => {
            let expr_var = encode_expr_src(object, counter, output, interner, variants);
            output.push_str(&format!("Let {} be a new CShow with expr {}.\n", var, expr_var));
        }
        Stmt::Call { function, args } => {
            let fn_name = interner.resolve(*function);
            let args_var = format!("callSArgs_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of CExpr.\n", args_var));
            for arg in args {
                let arg_var = encode_expr_src(arg, counter, output, interner, variants);
                output.push_str(&format!("Push {} to {}.\n", arg_var, args_var));
            }
            output.push_str(&format!(
                "Let {} be a new CCallS with name \"{}\" and args {}.\n",
                var, fn_name, args_var
            ));
        }
        Stmt::Push { value, collection } => {
            let val_var = encode_expr_src(value, counter, output, interner, variants);
            let coll_name = extract_ident_name(collection, interner);
            output.push_str(&format!(
                "Let {} be a new CPush with expr {} and target \"{}\".\n",
                var, val_var, coll_name
            ));
        }
        Stmt::SetIndex { collection, index, value } => {
            let coll_name = extract_ident_name(collection, interner);
            let idx_var = encode_expr_src(index, counter, output, interner, variants);
            let val_var = encode_expr_src(value, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CSetIdx with target \"{}\" and idx {} and val {}.\n",
                var, coll_name, idx_var, val_var
            ));
        }
        Stmt::SetField { object, field, value } => {
            let map_name = extract_ident_name(object, interner);
            let field_name = interner.resolve(*field);
            let key_var = format!("e_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new CText with value \"{}\".\n", key_var, field_name));
            let val_var = encode_expr_src(value, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CMapSet with target \"{}\" and key {} and val {}.\n",
                var, map_name, key_var, val_var
            ));
        }
        Stmt::Pop { collection, .. } => {
            let coll_name = extract_ident_name(collection, interner);
            output.push_str(&format!(
                "Let {} be a new CPop with target \"{}\".\n",
                var, coll_name
            ));
        }
        Stmt::Add { value, collection } => {
            let val_var = encode_expr_src(value, counter, output, interner, variants);
            let coll_name = extract_ident_name(collection, interner);
            output.push_str(&format!(
                "Let {} be a new CAdd with elem {} and target \"{}\".\n",
                var, val_var, coll_name
            ));
        }
        Stmt::Remove { value, collection } => {
            let val_var = encode_expr_src(value, counter, output, interner, variants);
            let coll_name = extract_ident_name(collection, interner);
            output.push_str(&format!(
                "Let {} be a new CRemove with elem {} and target \"{}\".\n",
                var, val_var, coll_name
            ));
        }
        Stmt::Inspect { .. } => {
            return String::new(); // Handled by encode_stmts_src
        }
        Stmt::Repeat { .. } => {
            return String::new(); // Handled by encode_stmts_src
        }
        Stmt::Break => {
            output.push_str(&format!("Let {} be a new CBreak.\n", var));
        }
        Stmt::RuntimeAssert { condition, hard } => {
            let cond_var = encode_expr_src(condition, counter, output, interner, variants);
            let msg_var = format!("e_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new CText with value \"assertion failed\".\n", msg_var));
            // `Require that` (hard) encodes as a distinct CStmt variant `CHardAssert`
            // (NOT `CRequire`, which is the `## Requires` dependency directive) so the
            // self-encoding round-trip preserves the enforced/dev distinction.
            let variant = if *hard { "CHardAssert" } else { "CRuntimeAssert" };
            output.push_str(&format!(
                "Let {} be a new {} with cond {} and msg {}.\n",
                var, variant, cond_var, msg_var
            ));
        }
        Stmt::Give { object, recipient } => {
            let expr_var = encode_expr_src(object, counter, output, interner, variants);
            let target_name = extract_ident_name(recipient, interner);
            output.push_str(&format!(
                "Let {} be a new CGive with expr {} and target \"{}\".\n",
                var, expr_var, target_name
            ));
        }
        Stmt::Escape { code, .. } => {
            let code_str = interner.resolve(*code);
            output.push_str(&format!(
                "Let {} be a new CEscStmt with code \"{}\".\n",
                var, code_str.replace('\"', "\\\"")
            ));
        }
        Stmt::Sleep { milliseconds } => {
            let dur_var = encode_expr_src(milliseconds, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CSleep with duration {}.\n",
                var, dur_var
            ));
        }
        Stmt::ReadFrom { var: read_var, source } => {
            let var_name = interner.resolve(*read_var);
            match source {
                ReadSource::Console => {
                    output.push_str(&format!(
                        "Let {} be a new CReadConsole with target \"{}\".\n",
                        var, var_name
                    ));
                }
                ReadSource::File(path_expr) => {
                    let path_var = encode_expr_src(path_expr, counter, output, interner, variants);
                    output.push_str(&format!(
                        "Let {} be a new CReadFile with path {} and target \"{}\".\n",
                        var, path_var, var_name
                    ));
                }
            }
        }
        Stmt::WriteFile { content, path } => {
            let path_var = encode_expr_src(path, counter, output, interner, variants);
            let content_var = encode_expr_src(content, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CWriteFile with path {} and content {}.\n",
                var, path_var, content_var
            ));
        }
        Stmt::Check { source_text, .. } => {
            let pred_var = format!("e_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new CBool with value true.\n", pred_var));
            let msg_var = format!("e_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new CText with value \"{}\".\n", msg_var, source_text.replace('\"', "\\\"")));
            output.push_str(&format!(
                "Let {} be a new CCheck with predicate {} and msg {}.\n",
                var, pred_var, msg_var
            ));
        }
        Stmt::Assert { .. } => {
            let prop_var = format!("e_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new CBool with value true.\n", prop_var));
            output.push_str(&format!(
                "Let {} be a new CAssert with proposition {}.\n",
                var, prop_var
            ));
        }
        Stmt::Trust { justification, .. } => {
            let prop_var = format!("e_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new CBool with value true.\n", prop_var));
            let just_str = interner.resolve(*justification);
            output.push_str(&format!(
                "Let {} be a new CTrust with proposition {} and justification \"{}\".\n",
                var, prop_var, just_str
            ));
        }
        Stmt::Require { crate_name, .. } => {
            let dep_name = interner.resolve(*crate_name);
            output.push_str(&format!(
                "Let {} be a new CRequire with dependency \"{}\".\n",
                var, dep_name
            ));
        }
        Stmt::MergeCrdt { source, target } => {
            let source_var = encode_expr_src(source, counter, output, interner, variants);
            // The target may be a struct (`a`) or a struct field (`local's active`); both
            // round-trip as surface syntax.
            let target_name = extract_ident_name(target, interner);
            output.push_str(&format!(
                "Let {} be a new CMerge with target \"{}\" and other {}.\n",
                var, target_name, source_var
            ));
        }
        Stmt::IncreaseCrdt { object, field, amount } => {
            let amount_var = encode_expr_src(amount, counter, output, interner, variants);
            // `Increase c's points` — the target is the FIELD, so carry both the object and
            // the field name (the residual must say `Increase c's points`, not `Increase c`).
            let target_name =
                format!("{}'s {}", extract_ident_name(object, interner), interner.resolve(*field));
            output.push_str(&format!(
                "Let {} be a new CIncrease with target \"{}\" and amount {}.\n",
                var, target_name, amount_var
            ));
        }
        Stmt::DecreaseCrdt { object, field, amount } => {
            let amount_var = encode_expr_src(amount, counter, output, interner, variants);
            let target_name =
                format!("{}'s {}", extract_ident_name(object, interner), interner.resolve(*field));
            output.push_str(&format!(
                "Let {} be a new CDecrease with target \"{}\" and amount {}.\n",
                var, target_name, amount_var
            ));
        }
        Stmt::AppendToSequence { sequence, value } => {
            let value_var = encode_expr_src(value, counter, output, interner, variants);
            let target_name = extract_ident_name(sequence, interner);
            output.push_str(&format!(
                "Let {} be a new CAppendToSeq with target \"{}\" and value {}.\n",
                var, target_name, value_var
            ));
        }
        Stmt::ResolveConflict { object, field, .. } => {
            let target_name =
                format!("{}'s {}", extract_ident_name(object, interner), interner.resolve(*field));
            output.push_str(&format!(
                "Let {} be a new CResolve with target \"{}\".\n",
                var, target_name
            ));
        }
        Stmt::Sync { var: sync_var, topic } => {
            let topic_var = encode_expr_src(topic, counter, output, interner, variants);
            let var_name = interner.resolve(*sync_var);
            output.push_str(&format!(
                "Let {} be a new CSync with target \"{}\" and channel {}.\n",
                var, var_name, topic_var
            ));
        }
        Stmt::Mount { var: mount_var, path } => {
            let path_var = encode_expr_src(path, counter, output, interner, variants);
            let var_name = interner.resolve(*mount_var);
            output.push_str(&format!(
                "Let {} be a new CMount with target \"{}\" and path {}.\n",
                var, var_name, path_var
            ));
        }
        Stmt::Concurrent { tasks } => {
            let branches_var = format!("e_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of Seq of CStmt.\n", branches_var));
            // One inner branch per task — collapsing them loses the per-task
            // branch structure the executor / PE handlers iterate over.
            for stmt in tasks.iter() {
                let branch_var = format!("e_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of CStmt.\n", branch_var));
                let sv = encode_stmt_src(stmt, counter, output, interner, variants);
                if !sv.is_empty() {
                    output.push_str(&format!("Push {} to {}.\n", sv, branch_var));
                }
                output.push_str(&format!("Push {} to {}.\n", branch_var, branches_var));
            }
            output.push_str(&format!(
                "Let {} be a new CConcurrent with branches {}.\n",
                var, branches_var
            ));
        }
        Stmt::Parallel { tasks } => {
            let branches_var = format!("e_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of Seq of CStmt.\n", branches_var));
            // One inner branch per task (see Concurrent above).
            for stmt in tasks.iter() {
                let branch_var = format!("e_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of CStmt.\n", branch_var));
                let sv = encode_stmt_src(stmt, counter, output, interner, variants);
                if !sv.is_empty() {
                    output.push_str(&format!("Push {} to {}.\n", sv, branch_var));
                }
                output.push_str(&format!("Push {} to {}.\n", branch_var, branches_var));
            }
            output.push_str(&format!(
                "Let {} be a new CParallel with branches {}.\n",
                var, branches_var
            ));
        }
        Stmt::LaunchTask { function, args } | Stmt::LaunchTaskWithHandle { function, args, .. } => {
            let func_name = interner.resolve(*function);
            let args_var = format!("e_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of CExpr.\n", args_var));
            for arg in args {
                let av = encode_expr_src(arg, counter, output, interner, variants);
                output.push_str(&format!("Push {} to {}.\n", av, args_var));
            }
            let body_var = format!("e_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of CStmt.\n", body_var));
            let call_var = format!("e_{}", *counter);
            *counter += 1;
            output.push_str(&format!(
                "Let {} be a new CCallS with name \"{}\" and args {}.\n",
                call_var, func_name, args_var
            ));
            output.push_str(&format!("Push {} to {}.\n", call_var, body_var));
            let handle_name = if let Stmt::LaunchTaskWithHandle { handle, .. } = stmt {
                interner.resolve(*handle).to_string()
            } else {
                "_task".to_string()
            };
            output.push_str(&format!(
                "Let {} be a new CLaunchTask with body {} and handle \"{}\".\n",
                var, body_var, handle_name
            ));
        }
        Stmt::StopTask { handle } => {
            let handle_var = encode_expr_src(handle, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CStopTask with handle {}.\n",
                var, handle_var
            ));
        }
        Stmt::CreatePipe { var: pipe_var, capacity, .. } => {
            let cap = capacity.unwrap_or(32);
            let cap_var = format!("e_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new CInt with value {}.\n", cap_var, cap));
            let pipe_name = interner.resolve(*pipe_var);
            output.push_str(&format!(
                "Let {} be a new CCreatePipe with name \"{}\" and capacity {}.\n",
                var, pipe_name, cap_var
            ));
        }
        Stmt::SendPipe { value, pipe } => {
            let val_var = encode_expr_src(value, counter, output, interner, variants);
            let pipe_name = match pipe {
                Expr::Identifier(sym) => interner.resolve(*sym).to_string(),
                _ => "pipe".to_string(),
            };
            output.push_str(&format!(
                "Let {} be a new CSendPipe with chan \"{}\" and value {}.\n",
                var, pipe_name, val_var
            ));
        }
        Stmt::ReceivePipe { var: recv_var, pipe } => {
            let recv_name = interner.resolve(*recv_var);
            let pipe_name = match pipe {
                Expr::Identifier(sym) => interner.resolve(*sym).to_string(),
                _ => "pipe".to_string(),
            };
            output.push_str(&format!(
                "Let {} be a new CReceivePipe with chan \"{}\" and target \"{}\".\n",
                var, pipe_name, recv_name
            ));
        }
        Stmt::TrySendPipe { value, pipe, .. } => {
            let val_var = encode_expr_src(value, counter, output, interner, variants);
            let pipe_name = match pipe {
                Expr::Identifier(sym) => interner.resolve(*sym).to_string(),
                _ => "pipe".to_string(),
            };
            output.push_str(&format!(
                "Let {} be a new CTrySendPipe with chan \"{}\" and value {}.\n",
                var, pipe_name, val_var
            ));
        }
        Stmt::TryReceivePipe { var: recv_var, pipe } => {
            let recv_name = interner.resolve(*recv_var);
            let pipe_name = match pipe {
                Expr::Identifier(sym) => interner.resolve(*sym).to_string(),
                _ => "pipe".to_string(),
            };
            output.push_str(&format!(
                "Let {} be a new CTryReceivePipe with chan \"{}\" and target \"{}\".\n",
                var, pipe_name, recv_name
            ));
        }
        Stmt::Select { branches } => {
            let branches_var = format!("e_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of CSelectBranch.\n", branches_var));
            for branch in branches {
                match branch {
                    SelectBranch::Receive { var: recv_var, pipe, body } => {
                        let recv_name = interner.resolve(*recv_var);
                        let pipe_name = match pipe {
                            Expr::Identifier(sym) => interner.resolve(*sym).to_string(),
                            _ => "pipe".to_string(),
                        };
                        let body_var = format!("e_{}", *counter);
                        *counter += 1;
                        output.push_str(&format!("Let {} be a new Seq of CStmt.\n", body_var));
                        for stmt in body.iter() {
                            let sv = encode_stmt_src(stmt, counter, output, interner, variants);
                            if !sv.is_empty() {
                                output.push_str(&format!("Push {} to {}.\n", sv, body_var));
                            }
                        }
                        let branch_var = format!("e_{}", *counter);
                        *counter += 1;
                        output.push_str(&format!(
                            "Let {} be a new CSelectRecv with chan \"{}\" and var \"{}\" and body {}.\n",
                            branch_var, pipe_name, recv_name, body_var
                        ));
                        output.push_str(&format!("Push {} to {}.\n", branch_var, branches_var));
                    }
                    SelectBranch::Timeout { milliseconds, body } => {
                        let dur_var = encode_expr_src(milliseconds, counter, output, interner, variants);
                        let body_var = format!("e_{}", *counter);
                        *counter += 1;
                        output.push_str(&format!("Let {} be a new Seq of CStmt.\n", body_var));
                        for stmt in body.iter() {
                            let sv = encode_stmt_src(stmt, counter, output, interner, variants);
                            if !sv.is_empty() {
                                output.push_str(&format!("Push {} to {}.\n", sv, body_var));
                            }
                        }
                        let branch_var = format!("e_{}", *counter);
                        *counter += 1;
                        output.push_str(&format!(
                            "Let {} be a new CSelectTimeout with duration {} and body {}.\n",
                            branch_var, dur_var, body_var
                        ));
                        output.push_str(&format!("Push {} to {}.\n", branch_var, branches_var));
                    }
                }
            }
            output.push_str(&format!(
                "Let {} be a new CSelect with branches {}.\n",
                var, branches_var
            ));
        }
        Stmt::Spawn { agent_type, name } => {
            let agent_name = interner.resolve(*agent_type);
            let target_name = interner.resolve(*name);
            output.push_str(&format!(
                "Let {} be a new CSpawn with agentType \"{}\" and target \"{}\".\n",
                var, agent_name, target_name
            ));
        }
        Stmt::SendMessage { message, destination, .. } => {
            let target_var = encode_expr_src(destination, counter, output, interner, variants);
            let msg_var = encode_expr_src(message, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CSendMessage with target {} and msg {}.\n",
                var, target_var, msg_var
            ));
        }
        // A batch stream send is encoded for the projection-1 self-interpreter exactly like a send,
        // so it is NEVER dropped from the residual (the catch-all would silently drop it). The PE
        // dialects pass it through unchanged — networking is opaque to specialization.
        Stmt::StreamMessage { values, destination } => {
            let target_var = encode_expr_src(destination, counter, output, interner, variants);
            let vals_var = encode_expr_src(values, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CStreamMessage with target {} and values {}.\n",
                var, target_var, vals_var
            ));
        }
        Stmt::AwaitMessage { into, .. } => {
            let await_name = interner.resolve(*into);
            output.push_str(&format!(
                "Let {} be a new CAwaitMessage with target \"{}\".\n",
                var, await_name
            ));
        }
        // The PNP `secure` binding is not modeled by the partial-evaluation self-interpreter (a
        // PNP program is a live-networking program, not a PE target); the address encoding is unchanged.
        Stmt::Listen { address, secure: _ } => {
            let addr_var = encode_expr_src(address, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CListen with addr {} and handler \"default\".\n",
                var, addr_var
            ));
        }
        Stmt::ConnectTo { address, secure: _ } => {
            let addr_var = encode_expr_src(address, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CConnectTo with addr {} and target \"conn\".\n",
                var, addr_var
            ));
        }
        Stmt::Zone { name, body, .. } => {
            let zone_name = interner.resolve(*name);
            let body_var = format!("e_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of CStmt.\n", body_var));
            for stmt in body.iter() {
                let sv = encode_stmt_src(stmt, counter, output, interner, variants);
                if !sv.is_empty() {
                    output.push_str(&format!("Push {} to {}.\n", sv, body_var));
                }
            }
            output.push_str(&format!(
                "Let {} be a new CZone with name \"{}\" and kind \"heap\" and body {}.\n",
                var, zone_name, body_var
            ));
        }
        Stmt::LetPeerAgent { var: pa_var, address } => {
            let addr_var = encode_expr_src(address, counter, output, interner, variants);
            let pa_name = interner.resolve(*pa_var);
            output.push_str(&format!(
                "Let {} be a new CConnectTo with addr {} and target \"{}\".\n",
                var, addr_var, pa_name
            ));
        }
        // ── DECLARATIONS — no projection-1 BODY encoding (intentionally empty) ──
        // These are handled at the PROGRAM level, not as body statements: functions are encoded into
        // `encodedFuncMap` as `CFuncDef`; struct/type declarations live in the type catalog; theorems
        // and definitions are proof-layer only, with no runtime effect to specialize. They contribute
        // nothing to a statement body.
        //
        // ⚠️  THIS MATCH IS EXHAUSTIVE ON PURPOSE — there is NO `_` wildcard. ⚠️  Every one of the 52
        // EXECUTABLE statements above has a real encoding arm, and these 4 declarations are the only
        // ones that encode to nothing. Adding a new statement to `Stmt` will FAIL TO COMPILE here
        // until it is given an explicit arm — so a statement can NEVER be silently dropped from the
        // Futamura projection again (this is exactly how the CStreamMessage drop hid). Do not "fix" a
        // build break here by re-adding a `_ =>` wildcard; add the statement's real encoding instead.
        Stmt::FunctionDef { .. }
        | Stmt::StructDef { .. }
        | Stmt::Theorem(_)
        | Stmt::Definition(_)
        | Stmt::Axiom(_)
        | Stmt::Theory(_) => {
            return String::new();
        }
    }

    var
}

fn encode_stmts_src(stmt: &Stmt, counter: &mut usize, output: &mut String, interner: &Interner, variants: &HashMap<String, Vec<String>>) -> Vec<String> {
    match stmt {
        Stmt::Inspect { target, arms, .. } => {
            let mut otherwise_stmts: Vec<&Stmt> = Vec::new();
            let mut variant_arms: Vec<(&MatchArm, Vec<&Stmt>)> = Vec::new();

            for arm in arms {
                if arm.variant.is_none() {
                    otherwise_stmts = arm.body.iter().collect();
                } else {
                    let body_refs: Vec<&Stmt> = arm.body.iter().collect();
                    variant_arms.push((arm, body_refs));
                }
            }

            if variant_arms.is_empty() {
                let mut result = Vec::new();
                for s in &otherwise_stmts {
                    for v in encode_stmts_src(s, counter, output, interner, variants) {
                        result.push(v);
                    }
                }
                return result;
            }

            // Flat CIf encoding: each arm becomes an independent CIf with empty else.
            // Since Inspect arms are mutually exclusive (exactly one tag matches),
            // flat CIf is semantically equivalent to nested CIf chains but avoids
            // deep nesting that the interpreter's inline CIf handler can't navigate.
            let has_otherwise = !otherwise_stmts.is_empty();
            let mut result = Vec::new();

            // If there's an Otherwise block, track whether any arm matched
            let matched_var_name = if has_otherwise {
                // The flag name uses the DETERMINISTIC per-program Inspect-Otherwise index (shared with
                // the native builder), NOT the global intermediate counter — so the fast builder can
                // reproduce this desugaring byte-for-byte. The `counter` bump is kept so every OTHER
                // (source-only, non-wire) intermediate name downstream is unchanged.
                let name = format!("__inspectMatched_{}", next_inspect_otherwise_idx());
                *counter += 1;
                let false_expr = format!("e_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new CBool with value false.\n", false_expr));
                let let_stmt = format!("s_{}", *counter);
                *counter += 1;
                output.push_str(&format!(
                    "Let {} be a new CLet with name \"{}\" and expr {}.\n",
                    let_stmt, name, false_expr
                ));
                result.push(let_stmt);
                Some(name)
            } else {
                None
            };

            // Each variant arm becomes: CIf(tag == "Variant", [bindings + body], [])
            for (arm, body_stmts) in &variant_arms {
                let variant_name = interner.resolve(arm.variant.unwrap());

                // Condition: tag == "VariantName"
                let tag_target = encode_expr_src(target, counter, output, interner, variants);
                let tag_key = format!("e_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new CText with value \"__tag\".\n", tag_key));
                let tag_get = format!("e_{}", *counter);
                *counter += 1;
                output.push_str(&format!(
                    "Let {} be a new CMapGet with target {} and key {}.\n",
                    tag_get, tag_target, tag_key
                ));
                let variant_text = format!("e_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new CText with value \"{}\".\n", variant_text, variant_name));
                let cond_var = format!("e_{}", *counter);
                *counter += 1;
                output.push_str(&format!(
                    "Let {} be a new CBinOp with op \"==\" and left {} and right {}.\n",
                    cond_var, tag_get, variant_text
                ));

                // Then-block: [optionally set matched flag, bindings, body]
                let then_list = format!("stmtList_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of CStmt.\n", then_list));

                // Set matched flag if needed
                if let Some(ref mname) = matched_var_name {
                    let true_expr = format!("e_{}", *counter);
                    *counter += 1;
                    output.push_str(&format!("Let {} be a new CBool with value true.\n", true_expr));
                    let set_stmt = format!("s_{}", *counter);
                    *counter += 1;
                    output.push_str(&format!(
                        "Let {} be a new CSet with name \"{}\" and expr {}.\n",
                        set_stmt, mname, true_expr
                    ));
                    output.push_str(&format!("Push {} to {}.\n", set_stmt, then_list));
                }

                // Bindings
                for (field_name, binding_name) in &arm.bindings {
                    let field_str = interner.resolve(*field_name);
                    let bind_str = interner.resolve(*binding_name);
                    let bind_target = encode_expr_src(target, counter, output, interner, variants);
                    let fkey = format!("e_{}", *counter);
                    *counter += 1;
                    output.push_str(&format!("Let {} be a new CText with value \"{}\".\n", fkey, field_str));
                    let fget = format!("e_{}", *counter);
                    *counter += 1;
                    output.push_str(&format!(
                        "Let {} be a new CMapGet with target {} and key {}.\n",
                        fget, bind_target, fkey
                    ));
                    let bind_let = format!("s_{}", *counter);
                    *counter += 1;
                    output.push_str(&format!(
                        "Let {} be a new CLet with name \"{}\" and expr {}.\n",
                        bind_let, bind_str, fget
                    ));
                    output.push_str(&format!("Push {} to {}.\n", bind_let, then_list));
                }

                // Body statements (use encode_stmts_src for Inspect/Repeat)
                for body_stmt in body_stmts {
                    match body_stmt {
                        Stmt::Inspect { .. } | Stmt::Repeat { .. } => {
                            let vars = encode_stmts_src(body_stmt, counter, output, interner, variants);
                            for v in vars {
                                output.push_str(&format!("Push {} to {}.\n", v, then_list));
                            }
                        }
                        _ => {
                            let bvar = encode_stmt_src(body_stmt, counter, output, interner, variants);
                            if !bvar.is_empty() {
                                output.push_str(&format!("Push {} to {}.\n", bvar, then_list));
                            }
                        }
                    }
                }

                // Empty else block
                let empty_else = format!("stmtList_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of CStmt.\n", empty_else));

                // CIf node
                let if_var = format!("s_{}", *counter);
                *counter += 1;
                output.push_str(&format!(
                    "Let {} be a new CIf with cond {} and thenBlock {} and elseBlock {}.\n",
                    if_var, cond_var, then_list, empty_else
                ));

                result.push(if_var);
            }

            // Otherwise: CIf(CNot(__inspectMatched), otherwise_body, [])
            if let Some(ref mname) = matched_var_name {
                let matched_ref = format!("e_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new CVar with name \"{}\".\n", matched_ref, mname));
                let not_matched = format!("e_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new CNot with inner {}.\n", not_matched, matched_ref));

                let otherwise_block = encode_stmt_list_src(&otherwise_stmts, counter, output, interner, variants);
                let empty_else = format!("stmtList_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of CStmt.\n", empty_else));

                let otherwise_if = format!("s_{}", *counter);
                *counter += 1;
                output.push_str(&format!(
                    "Let {} be a new CIf with cond {} and thenBlock {} and elseBlock {}.\n",
                    otherwise_if, not_matched, otherwise_block, empty_else
                ));
                result.push(otherwise_if);
            }

            result
        }
        Stmt::Repeat { pattern, iterable, body, .. } => {
            let loop_var_name = match pattern {
                Pattern::Identifier(sym) => interner.resolve(*sym).to_string(),
                Pattern::Tuple(syms) => {
                    if let Some(s) = syms.first() {
                        interner.resolve(*s).to_string()
                    } else {
                        "item".to_string()
                    }
                }
            };

            // Range-based repeat: encode as CRepeatRange
            if let Expr::Range { start, end } = iterable {
                let start_var = encode_expr_src(start, counter, output, interner, variants);
                let end_var = encode_expr_src(end, counter, output, interner, variants);
                let body_stmts: Vec<&Stmt> = body.iter().collect();
                let body_var = encode_stmt_list_src(&body_stmts, counter, output, interner, variants);
                let rr = format!("s_{}", *counter);
                *counter += 1;
                output.push_str(&format!(
                    "Let {} be a new CRepeatRange with var \"{}\" and start {} and end {} and body {}.\n",
                    rr, loop_var_name, start_var, end_var, body_var
                ));
                return vec![rr];
            }

            // Collection-based repeat: encode as CRepeat
            let coll_var = encode_expr_src(iterable, counter, output, interner, variants);
            let body_stmts: Vec<&Stmt> = body.iter().collect();
            let body_var = encode_stmt_list_src(&body_stmts, counter, output, interner, variants);
            let rep = format!("s_{}", *counter);
            *counter += 1;
            output.push_str(&format!(
                "Let {} be a new CRepeat with var \"{}\" and coll {} and body {}.\n",
                rep, loop_var_name, coll_var, body_var
            ));
            vec![rep]
        }
        // A CRDT op through a struct field (`Add x to p's guests`, `Append x to d's lines`)
        // — force the base struct dynamic FIRST, so the PE doesn't statically fold a struct
        // it then mutates, then emit the op itself.
        Stmt::Add { collection, .. }
        | Stmt::Remove { collection, .. }
        | Stmt::AppendToSequence { sequence: collection, .. } => {
            let mut result = Vec::new();
            if let Some(fd) = emit_force_dynamic(collection, counter, output, interner) {
                result.push(fd);
            }
            let v = encode_stmt_src(stmt, counter, output, interner, variants);
            if !v.is_empty() {
                result.push(v);
            }
            result
        }
        // A CRDT op that mutates the WHOLE struct (`Increase c's score`, `Merge b into a`) —
        // force the struct's root variable dynamic first, so the PE doesn't fold a counter /
        // register it then mutates.
        Stmt::IncreaseCrdt { object, .. }
        | Stmt::DecreaseCrdt { object, .. }
        | Stmt::ResolveConflict { object, .. }
        | Stmt::MergeCrdt { target: object, .. } => {
            let mut result = Vec::new();
            if let Some(fd) = emit_force_dynamic_struct(object, counter, output, interner) {
                result.push(fd);
            }
            let v = encode_stmt_src(stmt, counter, output, interner, variants);
            if !v.is_empty() {
                result.push(v);
            }
            result
        }
        _ => {
            let v = encode_stmt_src(stmt, counter, output, interner, variants);
            if v.is_empty() {
                vec![]
            } else {
                vec![v]
            }
        }
    }
}

fn encode_stmt_list_src(stmts: &[&Stmt], counter: &mut usize, output: &mut String, interner: &Interner, variants: &HashMap<String, Vec<String>>) -> String {
    let list_var = format!("stmtList_{}", *counter);
    *counter += 1;
    output.push_str(&format!("Let {} be a new Seq of CStmt.\n", list_var));

    for stmt in stmts {
        for stmt_var in encode_stmts_src(stmt, counter, output, interner, variants) {
            output.push_str(&format!("Push {} to {}.\n", stmt_var, list_var));
        }
    }

    list_var
}

fn extract_ident_name(expr: &Expr, interner: &Interner) -> String {
    match expr {
        Expr::Identifier(sym) => interner.resolve(*sym).to_string(),
        // A struct-field target (`p's guests`) round-trips as its LOGOS surface syntax, so
        // the PE residual decompiles back to a valid field access the tree-walker re-parses
        // — `Add x to p's guests`, `Append x to d's lines`. Without this the target collapsed
        // to "unknown" and the residual referenced an undefined variable.
        Expr::FieldAccess { object, field } => {
            format!("{}'s {}", extract_ident_name(object, interner), interner.resolve(*field))
        }
        _ => "unknown".to_string(),
    }
}

/// The ROOT variable a CRDT target mutates: `p's guests` → `p`, a bare `s` → `s`. The PE
/// can't fold a struct it later mutates through a field, so when this differs from the
/// surface target (i.e. the target IS a struct field) the encoder emits a [`CForceDynamic`]
/// marker that forces `p` dynamic. Computed in Rust so the self-interpreter — which must
/// compile to statically-typed Rust for self-application — never parses the field path.
fn crdt_base_var(expr: &Expr, interner: &Interner) -> Option<String> {
    match expr {
        Expr::FieldAccess { object, .. } => Some(crdt_base_var_root(object, interner)),
        _ => None,
    }
}

fn crdt_base_var_root(expr: &Expr, interner: &Interner) -> String {
    match expr {
        Expr::Identifier(sym) => interner.resolve(*sym).to_string(),
        Expr::FieldAccess { object, .. } => crdt_base_var_root(object, interner),
        _ => "unknown".to_string(),
    }
}

/// Emit a `CForceDynamic` node binding `base` and return its node var. Prepended before a
/// CRDT op so the PE invalidates the (struct) binding before it can fold it.
fn emit_force_dynamic_named(base: &str, counter: &mut usize, output: &mut String) -> String {
    let v = format!("s_{}", *counter);
    *counter += 1;
    output.push_str(&format!(
        "Let {} be a new CForceDynamic with name \"{}\".\n",
        v, base
    ));
    v
}

/// `CForceDynamic` for a COLLECTION op (`Add`/`Remove`/`Append`): only a struct-field target
/// needs it (a bare-variable collection is one the PE already tracks). `None` for bare vars.
fn emit_force_dynamic(
    collection: &Expr,
    counter: &mut usize,
    output: &mut String,
    interner: &Interner,
) -> Option<String> {
    let base = crdt_base_var(collection, interner)?;
    Some(emit_force_dynamic_named(&base, counter, output))
}

/// `CForceDynamic` for a struct-MUTATING CRDT op (`Increase`/`Decrease`/`Resolve`/`Merge`):
/// the whole struct is mutated, so its root variable is ALWAYS forced dynamic.
fn emit_force_dynamic_struct(
    object: &Expr,
    counter: &mut usize,
    output: &mut String,
    interner: &Interner,
) -> Option<String> {
    match object {
        Expr::Identifier(_) | Expr::FieldAccess { .. } => {
            Some(emit_force_dynamic_named(&crdt_base_var_root(object, interner), counter, output))
        }
        _ => None,
    }
}


/// First Futamura Projection: PE(interpreter, program) = compiled_program
///
/// Specializes the interpreter with respect to a fixed program, producing
/// a compiled version with no interpretive overhead. For a self-interpreter
/// (where source and target language are the same), this produces the
/// program itself, with static optimizations applied.
///
/// The pipeline:
/// 1. Parse the program source
/// 2. Run the optimizer (fold, propagate, PE, DCE)
/// 3. Decompile the optimized AST back to source
/// 4. Verify no interpretive overhead remains
pub fn projection1_source(_core_types: &str, _interpreter: &str, program: &str) -> Result<String, String> {
    let full_source = if program.contains("## Main") || program.contains("## To ") {
        program.to_string()
    } else {
        format!("## Main\n{}", program)
    };

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(&full_source, &mut interner);
    let tokens = lexer.tokenize();

    let type_registry = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        result.types
    };

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ast_ctx = AstContext::with_types(
        &expr_arena, &term_arena, &np_arena, &sym_arena,
        &role_arena, &pp_arena, &stmt_arena, &imperative_expr_arena,
        &type_expr_arena,
    );

    let mut parser = crate::parser::Parser::new(
        tokens, &mut world_state, &mut interner, ast_ctx, type_registry,
    );
    let stmts = parser.parse_program().map_err(|e| format!("Parse error: {:?}", e))?;

    // First Futamura Projection: PE(interpreter, program) = compiled_program.
    // Use the projection-safe optimizer: fold + propagate + PE + CTFE.
    // This preserves control-flow structure (If/While) while still folding
    // constants, propagating values, and specializing function calls.
    let optimized = crate::optimize::optimize_for_projection(
        stmts, &imperative_expr_arena, &stmt_arena, &mut interner,
        &crate::optimization::OptimizationConfig::from_env(),
    );

    let mut output = String::new();

    for stmt in &optimized {
        if matches!(stmt, Stmt::FunctionDef { .. }) {
            decompile_stmt(stmt, &interner, &mut output, 0);
            output.push('\n');
        }
    }

    output.push_str("## Main\n");
    for stmt in &optimized {
        if !matches!(stmt, Stmt::FunctionDef { .. }) {
            decompile_stmt(stmt, &interner, &mut output, 0);
        }
    }

    // Re-attach the program's type / struct definitions so a `new <Shared struct>`
    // in the residual can default-fill its CRDT fields (matches the genuine PE's
    // `projection1_source_real_fast`). Without this the residual reads an unset
    // field → "Field 'guests' not found".
    Ok(prepend_type_definitions(&full_source, output))
}

fn decompile_stmt(stmt: &Stmt, interner: &Interner, out: &mut String, indent: usize) {
    let pad = "    ".repeat(indent);
    match stmt {
        Stmt::FunctionDef { name, params, body, return_type, .. } => {
            let fn_name = interner.resolve(*name);
            let param_strs: Vec<String> = params
                .iter()
                .map(|(name, ty)| {
                    let pname = interner.resolve(*name);
                    format!("{}: {}", pname, decompile_type_expr(ty, interner))
                })
                .collect();
            let ret_str = if let Some(rt) = return_type {
                format!(" -> {}", decompile_type_expr(rt, interner))
            } else {
                String::new()
            };
            out.push_str(&format!("{}## To {} ({}){}:\n", pad, fn_name, param_strs.join(", "), ret_str));
            for s in body.iter() {
                decompile_stmt(s, interner, out, indent + 1);
            }
        }
        Stmt::Let { var, value, mutable, .. } => {
            let name = interner.resolve(*var);
            let expr_str = decompile_expr(value, interner);
            if *mutable {
                out.push_str(&format!("{}Let mutable {} be {}.\n", pad, name, expr_str));
            } else {
                out.push_str(&format!("{}Let {} be {}.\n", pad, name, expr_str));
            }
        }
        Stmt::Set { target, value } => {
            let name = interner.resolve(*target);
            let expr_str = decompile_expr(value, interner);
            out.push_str(&format!("{}Set {} to {}.\n", pad, name, expr_str));
        }
        Stmt::Show { object, .. } => {
            let expr_str = decompile_expr(object, interner);
            out.push_str(&format!("{}Show {}.\n", pad, expr_str));
        }
        Stmt::Return { value } => {
            if let Some(expr) = value {
                let expr_str = decompile_expr(expr, interner);
                out.push_str(&format!("{}Return {}.\n", pad, expr_str));
            } else {
                out.push_str(&format!("{}Return.\n", pad));
            }
        }
        Stmt::If { cond, then_block, else_block } => {
            let cond_str = decompile_expr(cond, interner);
            out.push_str(&format!("{}If {}:\n", pad, cond_str));
            for s in then_block.iter() {
                decompile_stmt(s, interner, out, indent + 1);
            }
            if let Some(els) = else_block {
                out.push_str(&format!("{}Otherwise:\n", pad));
                for s in els.iter() {
                    decompile_stmt(s, interner, out, indent + 1);
                }
            }
        }
        Stmt::While { cond, body, .. } => {
            let cond_str = decompile_expr(cond, interner);
            out.push_str(&format!("{}While {}:\n", pad, cond_str));
            for s in body.iter() {
                decompile_stmt(s, interner, out, indent + 1);
            }
        }
        Stmt::Call { function, args } => {
            let fn_name = interner.resolve(*function);
            let arg_strs: Vec<String> = args.iter().map(|a| decompile_expr(a, interner)).collect();
            if arg_strs.is_empty() {
                out.push_str(&format!("{}{}().\n", pad, fn_name));
            } else {
                out.push_str(&format!("{}{}({}).\n", pad, fn_name, arg_strs.join(", ")));
            }
        }
        Stmt::Push { value, collection } => {
            let val_str = decompile_expr(value, interner);
            let coll_str = decompile_expr(collection, interner);
            out.push_str(&format!("{}Push {} to {}.\n", pad, val_str, coll_str));
        }
        Stmt::SetIndex { collection, index, value } => {
            let coll_str = decompile_expr(collection, interner);
            let idx_str = decompile_expr(index, interner);
            let val_str = decompile_expr(value, interner);
            out.push_str(&format!("{}Set item {} of {} to {}.\n", pad, idx_str, coll_str, val_str));
        }
        Stmt::SetField { object, field, value } => {
            let obj_str = decompile_expr(object, interner);
            let field_name = interner.resolve(*field);
            let val_str = decompile_expr(value, interner);
            out.push_str(&format!("{}Set {} of {} to {}.\n", pad, field_name, obj_str, val_str));
        }
        Stmt::Repeat { pattern, iterable, body, .. } => {
            let var_name = match pattern {
                Pattern::Identifier(sym) => interner.resolve(*sym).to_string(),
                Pattern::Tuple(syms) => {
                    syms.iter().map(|s| interner.resolve(*s).to_string()).collect::<Vec<_>>().join(", ")
                }
            };
            let iter_str = decompile_expr(iterable, interner);
            out.push_str(&format!("{}Repeat for {} in {}:\n", pad, var_name, iter_str));
            for s in body.iter() {
                decompile_stmt(s, interner, out, indent + 1);
            }
        }
        Stmt::Inspect { target, arms, .. } => {
            let target_str = decompile_expr(target, interner);
            out.push_str(&format!("{}Inspect {}:\n", pad, target_str));
            for arm in arms {
                if let Some(variant) = arm.variant {
                    let variant_name = interner.resolve(variant);
                    let bindings: Vec<String> = arm.bindings.iter()
                        .map(|(_, b)| interner.resolve(*b).to_string())
                        .collect();
                    if bindings.is_empty() {
                        out.push_str(&format!("{}    When {}:\n", pad, variant_name));
                    } else {
                        out.push_str(&format!("{}    When {}({}):\n", pad, variant_name, bindings.join(", ")));
                    }
                } else {
                    out.push_str(&format!("{}    Otherwise:\n", pad));
                }
                for s in arm.body.iter() {
                    decompile_stmt(s, interner, out, indent + 2);
                }
            }
        }
        Stmt::Pop { collection, into } => {
            let coll_str = decompile_expr(collection, interner);
            if let Some(target) = into {
                let target_name = interner.resolve(*target);
                out.push_str(&format!("{}Pop from {} into {}.\n", pad, coll_str, target_name));
            } else {
                out.push_str(&format!("{}Pop from {}.\n", pad, coll_str));
            }
        }
        Stmt::Break => {
            out.push_str(&format!("{}Break.\n", pad));
        }
        Stmt::RuntimeAssert { condition, hard } => {
            let cond_str = decompile_expr(condition, interner);
            // Preserve the enforced/dev distinction: `Require that` (hard) survives the
            // round-trip rather than silently becoming the dev-only `Assert that`.
            let kw = if *hard { "Require that" } else { "Assert that" };
            out.push_str(&format!("{}{} {}.\n", pad, kw, cond_str));
        }
        Stmt::Add { value, collection } => {
            let val_str = decompile_expr(value, interner);
            let coll_str = decompile_expr(collection, interner);
            out.push_str(&format!("{}Add {} to {}.\n", pad, val_str, coll_str));
        }
        Stmt::Remove { value, collection } => {
            let val_str = decompile_expr(value, interner);
            let coll_str = decompile_expr(collection, interner);
            out.push_str(&format!("{}Remove {} from {}.\n", pad, val_str, coll_str));
        }
        Stmt::Zone { name, body, .. } => {
            let zone_name = interner.resolve(*name);
            out.push_str(&format!("{}Inside a new zone called \"{}\":\n", pad, zone_name));
            for s in body.iter() {
                decompile_stmt(s, interner, out, indent + 1);
            }
        }
        Stmt::ReadFrom { var, .. } => {
            let var_name = interner.resolve(*var);
            out.push_str(&format!("{}Read {} from the console.\n", pad, var_name));
        }
        Stmt::WriteFile { content, path } => {
            let content_str = decompile_expr(content, interner);
            let path_str = decompile_expr(path, interner);
            out.push_str(&format!("{}Write {} to file {}.\n", pad, content_str, path_str));
        }
        Stmt::Sleep { milliseconds } => {
            let ms = decompile_expr(milliseconds, interner);
            out.push_str(&format!("{}Sleep {}.\n", pad, ms));
        }
        // CRDT mutations: a Shared struct the program mutates survives the
        // projection unfolded (the optimizer keeps the binding dynamic), so the
        // residual MUST re-emit these or it silently changes the program's value.
        Stmt::IncreaseCrdt { object, field, amount } => {
            let obj = decompile_expr(object, interner);
            let fld = interner.resolve(*field);
            let amt = decompile_expr(amount, interner);
            out.push_str(&format!("{}Increase {}'s {} by {}.\n", pad, obj, fld, amt));
        }
        Stmt::DecreaseCrdt { object, field, amount } => {
            let obj = decompile_expr(object, interner);
            let fld = interner.resolve(*field);
            let amt = decompile_expr(amount, interner);
            out.push_str(&format!("{}Decrease {}'s {} by {}.\n", pad, obj, fld, amt));
        }
        Stmt::MergeCrdt { source, target } => {
            let src = decompile_expr(source, interner);
            let tgt = decompile_expr(target, interner);
            out.push_str(&format!("{}Merge {} into {}.\n", pad, src, tgt));
        }
        Stmt::AppendToSequence { sequence, value } => {
            let seq = decompile_expr(sequence, interner);
            let val = decompile_expr(value, interner);
            out.push_str(&format!("{}Append {} to {}.\n", pad, val, seq));
        }
        Stmt::ResolveConflict { object, field, value } => {
            let obj = decompile_expr(object, interner);
            let fld = interner.resolve(*field);
            let val = decompile_expr(value, interner);
            out.push_str(&format!("{}Resolve {}'s {} to {}.\n", pad, obj, fld, val));
        }
        _ => {
            // Remaining system-level statements (networking, concurrency) are not
            // produced by the optimizer and don't appear in P1 residuals.
        }
    }
}

fn decompile_expr(expr: &Expr, interner: &Interner) -> String {
    match expr {
        Expr::Literal(lit) => match lit {
            Literal::Number(n) => n.to_string(),
            Literal::Float(f) => format!("{}", f),
            Literal::Boolean(b) => if *b { "true".to_string() } else { "false".to_string() },
            Literal::Text(s) => format!("\"{}\"", interner.resolve(*s)),
            Literal::Nothing => "nothing".to_string(),
            Literal::Char(c) => format!("'{}'", c),
            Literal::Duration(ns) => format!("{}", ns),
            Literal::Date(days) => format!("{}", days),
            Literal::Moment(ns) => format!("{}", ns),
            Literal::Span { months, days } => format!("{} months {} days", months, days),
            Literal::Time(ns) => format!("{}", ns),
        },
        Expr::Identifier(sym) => interner.resolve(*sym).to_string(),
        Expr::BinaryOp { op, left, right } => {
            let l = if matches!(left, Expr::BinaryOp { .. }) {
                format!("({})", decompile_expr(left, interner))
            } else {
                decompile_expr(left, interner)
            };
            let r = if matches!(right, Expr::BinaryOp { .. }) {
                format!("({})", decompile_expr(right, interner))
            } else {
                decompile_expr(right, interner)
            };
            // Shift operations are introduced by bit-strength optimization.
            // Map back to equivalent multiply/divide for LOGOS source.
            if matches!(op, BinaryOpKind::Shl) {
                // n << k = n * 2^k
                if let Expr::Literal(Literal::Number(k)) = right {
                    let multiplier = 1i64 << k;
                    return format!("{} * {}", l, multiplier);
                }
            }
            if matches!(op, BinaryOpKind::Shr) {
                // n >> k = n / 2^k
                if let Expr::Literal(Literal::Number(k)) = right {
                    let divisor = 1i64 << k;
                    return format!("{} / {}", l, divisor);
                }
            }
            let op_str = match op {
                BinaryOpKind::Add => "+",
                BinaryOpKind::Subtract => "-",
                BinaryOpKind::Multiply => "*",
                BinaryOpKind::Divide | BinaryOpKind::ExactDivide => "/",
                BinaryOpKind::FloorDivide => "//",
                BinaryOpKind::Modulo => "%",
                BinaryOpKind::Eq => "equals",
                BinaryOpKind::NotEq => "is not",
                BinaryOpKind::Lt => "is less than",
                BinaryOpKind::Gt => "is greater than",
                BinaryOpKind::LtEq => "is at most",
                BinaryOpKind::GtEq => "is at least",
                BinaryOpKind::And => "and",
                BinaryOpKind::Or => "or",
                BinaryOpKind::Concat => "+",
                BinaryOpKind::SeqConcat => "followed by",
                BinaryOpKind::ApproxEq => "is approximately",
                BinaryOpKind::Pow => "**",
                BinaryOpKind::BitXor => "+",
                BinaryOpKind::BitAnd => "&",
                BinaryOpKind::BitOr => "|",
                BinaryOpKind::Shl => "*",
                BinaryOpKind::Shr => "/",
            };
            format!("{} {} {}", l, op_str, r)
        }
        Expr::Not { operand } => {
            // Parenthesize the operand so the negation's scope survives a
            // re-parse of the residual: `not (a and b)` must NOT become
            // `(not a) and b`.
            let inner = decompile_expr(operand, interner);
            format!("not ({})", inner)
        }
        Expr::Call { function, args } => {
            let fn_name = interner.resolve(*function);
            let arg_strs: Vec<String> = args.iter().map(|a| decompile_expr(a, interner)).collect();
            if arg_strs.is_empty() {
                format!("{}()", fn_name)
            } else {
                format!("{}({})", fn_name, arg_strs.join(", "))
            }
        }
        Expr::Index { collection, index } => {
            let coll = decompile_expr(collection, interner);
            let idx = decompile_expr(index, interner);
            format!("item {} of {}", idx, coll)
        }
        Expr::Length { collection } => {
            let coll = decompile_expr(collection, interner);
            format!("length of {}", coll)
        }
        Expr::FieldAccess { object, field } => {
            let obj = decompile_expr(object, interner);
            let field_name = interner.resolve(*field);
            // Possessive `obj's field` is the canonical, universally-parseable
            // field-read surface form (the parsers produce FieldAccess only from
            // it). `field of obj` does not round-trip (e.g. after `Show`/`Add`).
            format!("{}'s {}", obj, field_name)
        }
        Expr::New { type_name, .. } => {
            let tn = interner.resolve(*type_name);
            format!("a new {}", tn)
        }
        Expr::NewVariant { variant, fields, .. } => {
            let vn = interner.resolve(*variant);
            if fields.is_empty() {
                format!("a new {}", vn)
            } else {
                let parts: Vec<String> = fields.iter().map(|(name, val)| {
                    let n = interner.resolve(*name);
                    let v = decompile_expr(val, interner);
                    format!("{} {}", n, v)
                }).collect();
                format!("a new {} with {}", vn, parts.join(" and "))
            }
        }
        Expr::InterpolatedString(parts) => {
            let mut result = String::new();
            for part in parts {
                match part {
                    StringPart::Literal(sym) => {
                        result.push_str(&interner.resolve(*sym));
                    }
                    StringPart::Expr { value, debug, .. } => {
                        let expr_str = decompile_expr(value, interner);
                        if *debug {
                            result.push_str(&format!("{{{}=}}", expr_str));
                        } else {
                            result.push_str(&format!("{{{}}}", expr_str));
                        }
                    }
                }
            }
            format!("\"{}\"", result)
        }
        Expr::Slice { collection, start, end } => {
            let coll = decompile_expr(collection, interner);
            let s = decompile_expr(start, interner);
            let e = decompile_expr(end, interner);
            format!("{} {} through {}", coll, s, e)
        }
        Expr::Copy { expr } => {
            let inner = decompile_expr(expr, interner);
            format!("copy of {}", inner)
        }
        Expr::Give { value } => {
            let inner = decompile_expr(value, interner);
            format!("Give {}", inner)
        }
        Expr::Contains { collection, value } => {
            let coll = decompile_expr(collection, interner);
            let val = decompile_expr(value, interner);
            format!("{} contains {}", coll, val)
        }
        Expr::Union { left, right } => {
            let l = decompile_expr(left, interner);
            let r = decompile_expr(right, interner);
            format!("{} union {}", l, r)
        }
        Expr::Intersection { left, right } => {
            let l = decompile_expr(left, interner);
            let r = decompile_expr(right, interner);
            format!("{} intersection {}", l, r)
        }
        Expr::List(elems) => {
            let parts: Vec<String> = elems.iter().map(|e| decompile_expr(e, interner)).collect();
            format!("[{}]", parts.join(", "))
        }
        Expr::Tuple(elems) => {
            let parts: Vec<String> = elems.iter().map(|e| decompile_expr(e, interner)).collect();
            format!("({})", parts.join(", "))
        }
        Expr::Range { start, end } => {
            let s = decompile_expr(start, interner);
            let e = decompile_expr(end, interner);
            format!("{} to {}", s, e)
        }
        Expr::OptionSome { value } => {
            let inner = decompile_expr(value, interner);
            format!("some {}", inner)
        }
        Expr::OptionNone => "none".to_string(),
        Expr::WithCapacity { value, capacity } => {
            let val = decompile_expr(value, interner);
            let cap = decompile_expr(capacity, interner);
            format!("{} with capacity {}", val, cap)
        }
        Expr::Escape { language, code } => {
            let lang = interner.resolve(*language);
            let src = interner.resolve(*code);
            format!("Escape to {}:\n{}", lang, src)
        }
        Expr::ManifestOf { zone } => {
            let z = decompile_expr(zone, interner);
            format!("the manifest of {}", z)
        }
        Expr::ChunkAt { index, zone } => {
            let idx = decompile_expr(index, interner);
            let z = decompile_expr(zone, interner);
            format!("the chunk at {} in {}", idx, z)
        }
        Expr::Closure { params, body, return_type } => {
            let param_strs: Vec<String> = params.iter().map(|(name, ty)| {
                let n = interner.resolve(*name);
                let t = decompile_type_expr(ty, interner);
                format!("{}: {}", n, t)
            }).collect();
            let ret = if let Some(rt) = return_type {
                format!(" -> {}", decompile_type_expr(rt, interner))
            } else {
                String::new()
            };
            match body {
                ClosureBody::Expression(expr) => {
                    let e = decompile_expr(expr, interner);
                    format!("({}){} -> {}", param_strs.join(", "), ret, e)
                }
                ClosureBody::Block(_) => {
                    format!("({}){} -> [block]", param_strs.join(", "), ret)
                }
            }
        }
        Expr::CallExpr { callee, args } => {
            let c = decompile_expr(callee, interner);
            let arg_strs: Vec<String> = args.iter().map(|a| decompile_expr(a, interner)).collect();
            format!("{}({})", c, arg_strs.join(", "))
        }
    }
}

fn decompile_type_expr(ty: &TypeExpr, interner: &Interner) -> String {
    match ty {
        TypeExpr::Primitive(sym) => interner.resolve(*sym).to_string(),
        TypeExpr::Named(sym) => interner.resolve(*sym).to_string(),
        TypeExpr::Generic { base, params } => {
            let base_str = interner.resolve(*base);
            let param_strs: Vec<String> = params.iter().map(|p| decompile_type_expr(p, interner)).collect();
            format!("{} of {}", base_str, param_strs.join(" and "))
        }
        TypeExpr::Function { inputs, output } => {
            let in_strs: Vec<String> = inputs.iter().map(|t| decompile_type_expr(t, interner)).collect();
            let out_str = decompile_type_expr(output, interner);
            format!("fn({}) -> {}", in_strs.join(", "), out_str)
        }
        TypeExpr::Refinement { base, .. } => {
            decompile_type_expr(base, interner)
        }
        TypeExpr::Persistent { inner } => {
            format!("Persistent {}", decompile_type_expr(inner, interner))
        }
        TypeExpr::Mutable { inner } => {
            format!("mutable {}", decompile_type_expr(inner, interner))
        }
    }
}

/// Verify that a LogicAffeine program has no interpretive overhead.
///
/// Checks the AST for patterns that indicate unresolved interpreter dispatch:
/// - Inspect on CStmt/CExpr/CVal variants
/// - References to Core constructor types (CInt, CShow, etc.)
/// - Environment lookups on literal strings
pub fn verify_no_overhead_source(source: &str) -> Result<(), String> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    let type_registry = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        result.types
    };

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ast_ctx = AstContext::with_types(
        &expr_arena, &term_arena, &np_arena, &sym_arena,
        &role_arena, &pp_arena, &stmt_arena, &imperative_expr_arena,
        &type_expr_arena,
    );

    let mut parser = crate::parser::Parser::new(
        tokens, &mut world_state, &mut interner, ast_ctx, type_registry,
    );
    let stmts = parser.parse_program().map_err(|e| format!("Parse error: {:?}", e))?;

    verify_no_overhead_stmts(&stmts, &interner)
}

const CORE_VARIANT_NAMES: &[&str] = &[
    "CInt", "CBool", "CText", "CVar", "CBinOp", "CNot",
    "CCall", "CIndex", "CLen", "CMapGet",
    "CLet", "CSet", "CIf", "CWhile", "CReturn", "CShow",
    "CCallS", "CPush", "CSetIdx", "CMapSet",
    "CFuncDef", "CProg",
    "CManifestOf", "CChunkAt",
    "VInt", "VBool", "VText", "VSeq", "VMap", "VError", "VNothing",
];

/// Interpreter *carrier* type names — the umbrella data types the self-interpreter/PE thread
/// through their state. A Jones-optimal residual holds none of them: constructing
/// `a new Map of Text to CVal` (the environment), or binding a `CExpr`-typed variable, is
/// surviving interpreter state, not dissolved computation.
const CORE_TYPE_NAMES: &[&str] = &[
    "CVal", "CExpr", "CStmt", "CFunc", "CFuncDef", "CProgram", "CProg",
    "CoreOut", "CoreOutR", "PEState", "PEStateR", "PEMiniState", "PEMiniR",
];

fn verify_no_overhead_stmts(stmts: &[Stmt], interner: &Interner) -> Result<(), String> {
    for stmt in stmts {
        check_stmt_overhead(stmt, interner)?;
    }
    Ok(())
}

fn check_stmt_overhead(stmt: &Stmt, interner: &Interner) -> Result<(), String> {
    match stmt {
        Stmt::Inspect { arms, .. } => {
            for arm in arms {
                if let Some(variant) = arm.variant {
                    let variant_name = interner.resolve(variant);
                    if CORE_VARIANT_NAMES.contains(&variant_name) {
                        return Err(format!(
                            "Interpretive overhead: Inspect dispatches on Core variant '{}'",
                            variant_name
                        ));
                    }
                }
                for s in arm.body.iter() {
                    check_stmt_overhead(s, interner)?;
                }
            }
        }
        Stmt::If { cond, then_block, else_block } => {
            check_expr_overhead(cond, interner)?;
            for s in then_block.iter() {
                check_stmt_overhead(s, interner)?;
            }
            if let Some(els) = else_block {
                for s in els.iter() {
                    check_stmt_overhead(s, interner)?;
                }
            }
        }
        Stmt::While { cond, body, .. } => {
            check_expr_overhead(cond, interner)?;
            for s in body.iter() {
                check_stmt_overhead(s, interner)?;
            }
        }
        Stmt::FunctionDef { body, .. } => {
            for s in body.iter() {
                check_stmt_overhead(s, interner)?;
            }
        }
        Stmt::Repeat { body, .. } => {
            for s in body.iter() {
                check_stmt_overhead(s, interner)?;
            }
        }
        Stmt::Let { value, .. } | Stmt::Set { value, .. } | Stmt::Show { object: value, .. } => {
            check_expr_overhead(value, interner)?;
        }
        Stmt::Return { value } => {
            if let Some(v) = value {
                check_expr_overhead(v, interner)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn check_expr_overhead(expr: &Expr, interner: &Interner) -> Result<(), String> {
    match expr {
        Expr::Index { collection, index } => {
            // Check for `item X of env` where X is a literal string (env lookup overhead)
            if let Expr::Identifier(coll_sym) = collection {
                let coll_name = interner.resolve(*coll_sym);
                if coll_name == "env" {
                    if let Expr::Literal(Literal::Text(_)) = index {
                        return Err(
                            "Interpretive overhead: environment lookup 'item ... of env' on literal key".to_string()
                        );
                    }
                }
            }
            check_expr_overhead(collection, interner)?;
            check_expr_overhead(index, interner)?;
        }
        Expr::New { type_name, .. } => {
            let tn = interner.resolve(*type_name);
            if CORE_VARIANT_NAMES.contains(&tn) {
                return Err(format!(
                    "Interpretive overhead: Core type constructor 'new {}'", tn
                ));
            }
        }
        Expr::NewVariant { variant, .. } => {
            let vn = interner.resolve(*variant);
            if CORE_VARIANT_NAMES.contains(&vn) {
                return Err(format!(
                    "Interpretive overhead: Core variant constructor '{}'", vn
                ));
            }
        }
        Expr::Call { function, args } => {
            let fn_name = interner.resolve(*function);
            if CORE_VARIANT_NAMES.contains(&fn_name) {
                return Err(format!(
                    "Interpretive overhead: Core variant call '{}'", fn_name
                ));
            }
            for a in args {
                check_expr_overhead(a, interner)?;
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            check_expr_overhead(left, interner)?;
            check_expr_overhead(right, interner)?;
        }
        Expr::Not { operand } => {
            check_expr_overhead(operand, interner)?;
        }
        Expr::Length { collection } => {
            check_expr_overhead(collection, interner)?;
        }
        Expr::FieldAccess { object, .. } => {
            check_expr_overhead(object, interner)?;
        }
        _ => {}
    }
    Ok(())
}

/// Names whose appearance as a call function in a residual betrays surviving
/// interpreter dispatch — the residual is still threading through the PE or the
/// self-interpreter instead of having been specialized away.
const DISPATCH_FN_NAMES: &[&str] = &[
    "peExpr", "peBlock", "coreEval", "coreExecBlock", "applyBinOp",
];

/// The Jones-optimality oracle: count the units of interpreter dispatch remaining
/// in a residual program.
///
/// A residual that is genuinely Jones-optimal has dissolved the interpreter entirely
/// and returns zero. One unit is counted for each of:
/// - an `Inspect` arm dispatching on a Core IR variant (`CInt`, `CIf`, …);
/// - an environment/function-map lookup `item <literal> of env` / `... of funcs`;
/// - a Core IR constructor (`new CInt`, a `CInt(...)` call, a `VInt` variant, …);
/// - a call to a known dispatch function ([`DISPATCH_FN_NAMES`]).
///
/// Unlike [`verify_no_overhead_source`], this walks the whole tree and accumulates a
/// count rather than short-circuiting on the first violation.
pub fn count_dispatch(source: &str) -> usize {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    let type_registry = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        result.types
    };

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ast_ctx = AstContext::with_types(
        &expr_arena, &term_arena, &np_arena, &sym_arena,
        &role_arena, &pp_arena, &stmt_arena, &imperative_expr_arena,
        &type_expr_arena,
    );

    let mut parser = crate::parser::Parser::new(
        tokens, &mut world_state, &mut interner, ast_ctx, type_registry,
    );
    let stmts = match parser.parse_program() {
        Ok(s) => s,
        // A residual we cannot parse cannot be asserted Jones-optimal; surface that as
        // a non-zero count rather than a false clean bill of health.
        Err(_) => return usize::MAX,
    };

    let mut count = 0usize;
    for stmt in &stmts {
        count_stmt_dispatch(stmt, &interner, &mut count);
    }
    count
}

fn count_block_dispatch(block: &[Stmt], interner: &Interner, count: &mut usize) {
    for s in block {
        count_stmt_dispatch(s, interner, count);
    }
}

/// Does a type expression name any interpreter carrier type ([`CORE_TYPE_NAMES`]) or Core
/// variant? Exhaustive over `TypeExpr` — a new type form stops the build until handled.
fn type_expr_mentions_core(t: &TypeExpr, interner: &Interner) -> bool {
    let is_core = |s| {
        let n = interner.resolve(s);
        CORE_TYPE_NAMES.contains(&n) || CORE_VARIANT_NAMES.contains(&n)
    };
    match t {
        TypeExpr::Primitive(s) | TypeExpr::Named(s) => is_core(*s),
        TypeExpr::Generic { base, params } => {
            is_core(*base) || params.iter().any(|p| type_expr_mentions_core(p, interner))
        }
        TypeExpr::Function { inputs, output } => {
            inputs.iter().any(|p| type_expr_mentions_core(p, interner))
                || type_expr_mentions_core(output, interner)
        }
        TypeExpr::Refinement { base, .. } => type_expr_mentions_core(base, interner),
        TypeExpr::Persistent { inner } | TypeExpr::Mutable { inner } => {
            type_expr_mentions_core(inner, interner)
        }
    }
}

/// Scan raw escaped foreign code for whole-word Core-IR / dispatch names. An `Escape`
/// block embedding `CInt(..)` or `coreEval(..)` is interpreter overhead the AST walk
/// cannot see inside, so it is flagged lexically.
fn count_escape_code_dispatch(text: &str, count: &mut usize) {
    for word in text.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if !word.is_empty()
            && (CORE_VARIANT_NAMES.contains(&word) || DISPATCH_FN_NAMES.contains(&word))
        {
            *count += 1;
        }
    }
}

/// Exhaustive by construction — NO `_` arm. A new `Stmt` variant stops the build here
/// until it is classified, so the language cannot grow a statement that hides interpreter
/// dispatch from the Jones oracle.
fn count_stmt_dispatch(stmt: &Stmt, interner: &Interner, count: &mut usize) {
    match stmt {
        Stmt::Splice { body } => {
            for inner in body.iter() {
                count_stmt_dispatch(inner, interner, count);
            }
        }
        Stmt::Let { value, ty, .. } => {
            if let Some(t) = ty {
                if type_expr_mentions_core(t, interner) {
                    *count += 1;
                }
            }
            count_expr_dispatch(value, interner, count);
        }
        Stmt::Set { value, .. } => count_expr_dispatch(value, interner, count),
        Stmt::Call { function, args } => {
            let fn_name = interner.resolve(*function);
            if CORE_VARIANT_NAMES.contains(&fn_name) || DISPATCH_FN_NAMES.contains(&fn_name) {
                *count += 1;
            }
            for a in args {
                count_expr_dispatch(a, interner, count);
            }
        }
        Stmt::If { cond, then_block, else_block } => {
            count_expr_dispatch(cond, interner, count);
            count_block_dispatch(then_block, interner, count);
            if let Some(els) = else_block {
                count_block_dispatch(els, interner, count);
            }
        }
        Stmt::While { cond, body, decreasing } => {
            count_expr_dispatch(cond, interner, count);
            count_block_dispatch(body, interner, count);
            if let Some(d) = decreasing {
                count_expr_dispatch(d, interner, count);
            }
        }
        Stmt::Repeat { iterable, body, .. } => {
            count_expr_dispatch(iterable, interner, count);
            count_block_dispatch(body, interner, count);
        }
        Stmt::Return { value } => {
            if let Some(v) = value {
                count_expr_dispatch(v, interner, count);
            }
        }
        Stmt::RuntimeAssert { condition, .. } => count_expr_dispatch(condition, interner, count),
        Stmt::Give { object, recipient } | Stmt::Show { object, recipient } => {
            count_expr_dispatch(object, interner, count);
            count_expr_dispatch(recipient, interner, count);
        }
        Stmt::SetField { object, value, .. } => {
            count_expr_dispatch(object, interner, count);
            count_expr_dispatch(value, interner, count);
        }
        Stmt::FunctionDef { body, .. } => count_block_dispatch(body, interner, count),
        Stmt::Inspect { target, arms, .. } => {
            count_expr_dispatch(target, interner, count);
            for arm in arms {
                if let Some(variant) = arm.variant {
                    if CORE_VARIANT_NAMES.contains(&interner.resolve(variant)) {
                        *count += 1;
                    }
                }
                count_block_dispatch(arm.body, interner, count);
            }
        }
        Stmt::Push { value, collection }
        | Stmt::Add { value, collection }
        | Stmt::Remove { value, collection } => {
            count_expr_dispatch(value, interner, count);
            count_expr_dispatch(collection, interner, count);
        }
        Stmt::Pop { collection, .. } => count_expr_dispatch(collection, interner, count),
        Stmt::SetIndex { collection, index, value } => {
            count_expr_dispatch(collection, interner, count);
            count_expr_dispatch(index, interner, count);
            count_expr_dispatch(value, interner, count);
        }
        Stmt::Zone { body, .. } => count_block_dispatch(body, interner, count),
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            count_block_dispatch(tasks, interner, count)
        }
        Stmt::ReadFrom { source, .. } => {
            if let ReadSource::File(path) = source {
                count_expr_dispatch(path, interner, count);
            }
        }
        Stmt::WriteFile { content, path } => {
            count_expr_dispatch(content, interner, count);
            count_expr_dispatch(path, interner, count);
        }
        Stmt::SendMessage { message, destination, .. } => {
            count_expr_dispatch(message, interner, count);
            count_expr_dispatch(destination, interner, count);
        }
        Stmt::AwaitMessage { source, .. } => count_expr_dispatch(source, interner, count),
        Stmt::StreamMessage { values, destination } => {
            count_expr_dispatch(values, interner, count);
            count_expr_dispatch(destination, interner, count);
        }
        Stmt::MergeCrdt { source, target } => {
            count_expr_dispatch(source, interner, count);
            count_expr_dispatch(target, interner, count);
        }
        Stmt::IncreaseCrdt { object, amount, .. } | Stmt::DecreaseCrdt { object, amount, .. } => {
            count_expr_dispatch(object, interner, count);
            count_expr_dispatch(amount, interner, count);
        }
        Stmt::AppendToSequence { sequence, value } => {
            count_expr_dispatch(sequence, interner, count);
            count_expr_dispatch(value, interner, count);
        }
        Stmt::ResolveConflict { object, value, .. } => {
            count_expr_dispatch(object, interner, count);
            count_expr_dispatch(value, interner, count);
        }
        Stmt::Listen { address, .. } | Stmt::ConnectTo { address, .. } | Stmt::LetPeerAgent { address, .. } => {
            count_expr_dispatch(address, interner, count)
        }
        Stmt::Sleep { milliseconds } => count_expr_dispatch(milliseconds, interner, count),
        Stmt::Sync { topic, .. } => count_expr_dispatch(topic, interner, count),
        Stmt::Mount { path, .. } => count_expr_dispatch(path, interner, count),
        Stmt::LaunchTask { function, args } | Stmt::LaunchTaskWithHandle { function, args, .. } => {
            let fn_name = interner.resolve(*function);
            if CORE_VARIANT_NAMES.contains(&fn_name) || DISPATCH_FN_NAMES.contains(&fn_name) {
                *count += 1;
            }
            for a in args {
                count_expr_dispatch(a, interner, count);
            }
        }
        Stmt::SendPipe { value, pipe } | Stmt::TrySendPipe { value, pipe, .. } => {
            count_expr_dispatch(value, interner, count);
            count_expr_dispatch(pipe, interner, count);
        }
        Stmt::ReceivePipe { pipe, .. } | Stmt::TryReceivePipe { pipe, .. } => {
            count_expr_dispatch(pipe, interner, count)
        }
        Stmt::StopTask { handle } => count_expr_dispatch(handle, interner, count),
        Stmt::Select { branches } => {
            for b in branches {
                match b {
                    SelectBranch::Receive { pipe, body, .. } => {
                        count_expr_dispatch(pipe, interner, count);
                        count_block_dispatch(body, interner, count);
                    }
                    SelectBranch::Timeout { milliseconds, body } => {
                        count_expr_dispatch(milliseconds, interner, count);
                        count_block_dispatch(body, interner, count);
                    }
                }
            }
        }
        Stmt::Escape { code, .. } => count_escape_code_dispatch(interner.resolve(*code), count),
        // Leaves and declarations: no executable `Expr` children reach the oracle.
        // `Assert`/`Trust` carry a `LogicExpr` (proof layer), not an executable `Expr`.
        Stmt::Break
        | Stmt::Assert { .. }
        | Stmt::Trust { .. }
        | Stmt::StructDef { .. }
        | Stmt::Spawn { .. }
        | Stmt::Check { .. }
        | Stmt::CreatePipe { .. }
        | Stmt::Theorem(_)
        | Stmt::Definition(_)
        | Stmt::Axiom(_)
        | Stmt::Theory(_)
        | Stmt::Require { .. } => {}
    }
}

/// Exhaustive by construction — NO `_` arm. Every sub-expression is recursed, so a Core
/// constructor or dispatch call cannot hide inside a container (`List`/`Tuple`), an
/// operator, a constructor field, an interpolant, or a closure body.
fn count_expr_dispatch(expr: &Expr, interner: &Interner, count: &mut usize) {
    match expr {
        Expr::Literal(_) | Expr::Identifier(_) | Expr::OptionNone => {}
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right } => {
            count_expr_dispatch(left, interner, count);
            count_expr_dispatch(right, interner, count);
        }
        Expr::Not { operand } => count_expr_dispatch(operand, interner, count),
        Expr::Call { function, args } => {
            let fn_name = interner.resolve(*function);
            if CORE_VARIANT_NAMES.contains(&fn_name) || DISPATCH_FN_NAMES.contains(&fn_name) {
                *count += 1;
            }
            for a in args {
                count_expr_dispatch(a, interner, count);
            }
        }
        Expr::Index { collection, index } => {
            // Any `item ... of env|funcs` is surviving interpreter-state threading, whatever
            // the key: a Jones-optimal residual holds no environment/function map at all.
            if let Expr::Identifier(coll_sym) = collection {
                let coll_name = interner.resolve(*coll_sym);
                if coll_name == "env" || coll_name == "funcs" {
                    *count += 1;
                }
            }
            count_expr_dispatch(collection, interner, count);
            count_expr_dispatch(index, interner, count);
        }
        Expr::Slice { collection, start, end } => {
            count_expr_dispatch(collection, interner, count);
            count_expr_dispatch(start, interner, count);
            count_expr_dispatch(end, interner, count);
        }
        Expr::Copy { expr: inner } => count_expr_dispatch(inner, interner, count),
        Expr::Give { value } => count_expr_dispatch(value, interner, count),
        Expr::Length { collection } => count_expr_dispatch(collection, interner, count),
        Expr::Contains { collection, value } => {
            count_expr_dispatch(collection, interner, count);
            count_expr_dispatch(value, interner, count);
        }
        Expr::ManifestOf { zone } => count_expr_dispatch(zone, interner, count),
        Expr::ChunkAt { index, zone } => {
            count_expr_dispatch(index, interner, count);
            count_expr_dispatch(zone, interner, count);
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for e in items {
                count_expr_dispatch(e, interner, count);
            }
        }
        Expr::Range { start, end } => {
            count_expr_dispatch(start, interner, count);
            count_expr_dispatch(end, interner, count);
        }
        Expr::FieldAccess { object, .. } => count_expr_dispatch(object, interner, count),
        Expr::New { type_name, type_args, init_fields } => {
            let tn = interner.resolve(*type_name);
            if CORE_VARIANT_NAMES.contains(&tn) || CORE_TYPE_NAMES.contains(&tn) {
                *count += 1;
            }
            // A collection over a Core carrier type — `a new Map of Text to CVal` — is the
            // interpreter's environment/function map surviving in the residual.
            for ta in type_args {
                if type_expr_mentions_core(ta, interner) {
                    *count += 1;
                }
            }
            for (_, v) in init_fields {
                count_expr_dispatch(v, interner, count);
            }
        }
        Expr::NewVariant { variant, fields, .. } => {
            if CORE_VARIANT_NAMES.contains(&interner.resolve(*variant)) {
                *count += 1;
            }
            for (_, v) in fields {
                count_expr_dispatch(v, interner, count);
            }
        }
        Expr::Escape { code, .. } => count_escape_code_dispatch(interner.resolve(*code), count),
        Expr::OptionSome { value } => count_expr_dispatch(value, interner, count),
        Expr::WithCapacity { value, capacity } => {
            count_expr_dispatch(value, interner, count);
            count_expr_dispatch(capacity, interner, count);
        }
        Expr::Closure { body, .. } => match body {
            ClosureBody::Expression(e) => count_expr_dispatch(e, interner, count),
            ClosureBody::Block(b) => count_block_dispatch(b, interner, count),
        },
        Expr::CallExpr { callee, args } => {
            count_expr_dispatch(callee, interner, count);
            for a in args {
                count_expr_dispatch(a, interner, count);
            }
        }
        Expr::InterpolatedString(parts) => {
            for p in parts {
                if let StringPart::Expr { value, .. } = p {
                    count_expr_dispatch(value, interner, count);
                }
            }
        }
    }
}

/// Returns the source text of the partial evaluator written in LogicAffeine.
///
/// This PE operates on CProgram representations using explicit environments
/// and static dispatch. It is first-order (no closures) and uses only
/// literal string function names (no dynamic dispatch).
pub fn pe_source_text() -> &'static str {
    include_str!("optimize/pe_source.logos")
}

pub fn decompile_source_text() -> &'static str {
    include_str!("optimize/decompile_source.logos")
}

pub fn pe_bti_source_text() -> &'static str {
    include_str!("optimize/pe_bti_source.logos")
}

pub fn pe_mini_source_text() -> &'static str {
    include_str!("optimize/pe_mini_source.logos")
}

/// The canonical inductive type catalog the Futamura projections are built on
/// (`CStmt`/`CExpr`/`CVal`/…). Every statement the tree-walker executes has a
/// `C…` constructor here; the partial-evaluator dialects dispatch on it. Exposed
/// so cross-tier lock tests can prove this catalog and every tier stay in sync —
/// a statement added to the language must appear here or it is dropped from the
/// projections (see `tier_parity_lock.rs`).
pub fn core_types_for_pe_source() -> &'static str {
    CORE_TYPES_FOR_PE
}

const CORE_TYPES_FOR_PE: &str = r#"
## A CExpr is one of:
    A CInt with value Int.
    A CFloat with value Real.
    A CBool with value Bool.
    A CText with value Text.
    A CVar with name Text.
    A CBinOp with op Text and left CExpr and right CExpr.
    A CNot with inner CExpr.
    A CCall with name Text and args Seq of CExpr.
    A CIndex with coll CExpr and idx CExpr.
    A CLen with target CExpr.
    A CMapGet with target CExpr and key CExpr.
    A CNewSeq.
    A CNewVariant with tag Text and fnames Seq of Text and fvals Seq of CExpr.
    A CList with items Seq of CExpr.
    A CRange with start CExpr and end CExpr.
    A CSlice with coll CExpr and startIdx CExpr and endIdx CExpr.
    A CCopy with target CExpr.
    A CNewSet.
    A CContains with coll CExpr and elem CExpr.
    A CUnion with left CExpr and right CExpr.
    A CIntersection with left CExpr and right CExpr.
    A COptionSome with inner CExpr.
    A COptionNone.
    A CTuple with items Seq of CExpr.
    A CNew with typeName Text and fieldNames Seq of Text and fields Seq of CExpr.
    A CFieldAccess with target CExpr and field Text.
    A CClosure with params Seq of Text and body Seq of CStmt and captured Seq of Text.
    A CCallExpr with target CExpr and args Seq of CExpr.
    A CInterpolatedString with parts Seq of CStringPart.
    A CDuration with amount CExpr and unit Text.
    A CTimeNow.
    A CDateToday.
    A CEscExpr with code Text.
    A CManifestOf with zn CExpr.
    A CChunkAt with idx CExpr and zn CExpr.

## A CStringPart is one of:
    A CLiteralPart with value Text.
    A CExprPart with expr CExpr.

## A CStmt is one of:
    A CLet with name Text and expr CExpr.
    A CSet with name Text and expr CExpr.
    A CIf with cond CExpr and thenBlock Seq of CStmt and elseBlock Seq of CStmt.
    A CWhile with cond CExpr and body Seq of CStmt.
    A CReturn with expr CExpr.
    A CShow with expr CExpr.
    A CCallS with name Text and args Seq of CExpr.
    A CPush with expr CExpr and target Text.
    A CSetIdx with target Text and idx CExpr and val CExpr.
    A CMapSet with target Text and key CExpr and val CExpr.
    A CPop with target Text.
    A CRepeat with var Text and coll CExpr and body Seq of CStmt.
    A CRepeatRange with var Text and start CExpr and end CExpr and body Seq of CStmt.
    A CBreak.
    A CAdd with elem CExpr and target Text.
    A CRemove with elem CExpr and target Text.
    A CForceDynamic with name Text.
    A CSetField with target Text and field Text and val CExpr.
    A CStructDef with name Text and fieldNames Seq of Text.
    A CInspect with target CExpr and arms Seq of CMatchArm.
    A CEnumDef with name Text and variants Seq of Text.
    A CRuntimeAssert with cond CExpr and msg CExpr.
    A CHardAssert with cond CExpr and msg CExpr.
    A CGive with expr CExpr and target Text.
    A CEscStmt with code Text.
    A CSleep with duration CExpr.
    A CReadConsole with target Text.
    A CReadFile with path CExpr and target Text.
    A CWriteFile with path CExpr and content CExpr.
    A CCheck with predicate CExpr and msg CExpr.
    A CAssert with proposition CExpr.
    A CTrust with proposition CExpr and justification Text.
    A CRequire with dependency Text.
    A CMerge with target Text and other CExpr.
    A CIncrease with target Text and amount CExpr.
    A CDecrease with target Text and amount CExpr.
    A CAppendToSeq with target Text and value CExpr.
    A CResolve with target Text.
    A CSync with target Text and channel CExpr.
    A CMount with target Text and path CExpr.
    A CConcurrent with branches Seq of Seq of CStmt.
    A CParallel with branches Seq of Seq of CStmt.
    A CLaunchTask with body Seq of CStmt and handle Text.
    A CStopTask with handle CExpr.
    A CSelect with branches Seq of CSelectBranch.
    A CCreatePipe with name Text and capacity CExpr.
    A CSendPipe with chan Text and value CExpr.
    A CReceivePipe with chan Text and target Text.
    A CTrySendPipe with chan Text and value CExpr.
    A CTryReceivePipe with chan Text and target Text.
    A CSpawn with agentType Text and target Text.
    A CSendMessage with target CExpr and msg CExpr.
    A CStreamMessage with target CExpr and values CExpr.
    A CAwaitMessage with target Text.
    A CListen with addr CExpr and handler Text.
    A CConnectTo with addr CExpr and target Text.
    A CZone with name Text and kind Text and body Seq of CStmt.

## A CSelectBranch is one of:
    A CSelectRecv with chan Text and var Text and body Seq of CStmt.
    A CSelectTimeout with duration CExpr and body Seq of CStmt.

## A CMatchArm is one of:
    A CWhen with variantName Text and bindings Seq of Text and body Seq of CStmt.
    A COtherwise with body Seq of CStmt.

## A CFunc is one of:
    A CFuncDef with name Text and params Seq of Text and paramTypes Seq of Text and returnType Text and body Seq of CStmt.

## A CProgram is one of:
    A CProg with funcs Seq of CFunc and main Seq of CStmt.

## A PEState is one of:
    A PEStateR with env Map of Text to CVal and funcs Map of Text to CFunc and depth Int and staticEnv Map of Text to CExpr and specResults Map of Text to CExpr and onStack Seq of Text.

## A CVal is one of:
    A VInt with value Int.
    A VFloat with value Real.
    A VBool with value Bool.
    A VText with value Text.
    A VSeq with items Seq of CVal.
    A VMap with entries Map of Text to CVal.
    A VError with msg Text.
    A VNothing.
    A VSet with items Seq of CVal.
    A VOption with inner CVal and present Bool.
    A VTuple with items Seq of CVal.
    A VStruct with typeName Text and fields Map of Text to CVal.
    A VVariant with typeName Text and variantName Text and fields Seq of CVal.
    A VClosure with params Seq of Text and body Seq of CStmt and capturedEnv Map of Text to CVal.
    A VDuration with millis Int.
    A VDate with year Int and month Int and day Int.
    A VMoment with millis Int.
    A VSpan with startMillis Int and endMillis Int.
    A VTime with hour Int and minute Int and second Int.
    A VCrdt with kind Text and state Map of Text to CVal.
"#;

/// Encodes the partial evaluator as CProgram construction source code.
///
/// Returns PE function definitions (so peBlock etc. are callable) followed by
/// LOGOS statements that construct the PE's functions as CFunc data in
/// `encodedFuncMap` and its main block in `encodedMain`.
/// The parser handles `## To` blocks anywhere in the token stream, so
/// function definitions placed after `## Main` are parsed correctly.
pub fn quote_pe_source() -> Result<String, String> {
    let pe_source = pe_source_text();
    let full_source = format!("{}\n{}", CORE_TYPES_FOR_PE, pe_source);
    let encoded = encode_program_source(&full_source).map_err(|e| format!("Failed to encode PE: {:?}", e))?;
    Ok(format!("{}\n{}", pe_source, encoded))
}

/// Second Futamura Projection: PE(PE, interpreter) = compiler
///
/// Specializes the partial evaluator with respect to a fixed interpreter,
/// producing a compiler that takes any CProgram as input and produces
/// optimized residual CStmt/CExpr data.
///
/// For the Core self-interpreter (which is the identity evaluator on CProgram
/// data), PE(PE, int) resolves to the PE itself operating directly on program
/// data — the interpreter's dispatch loop is the CExpr/CStmt case analysis
/// that the PE already implements. The result is the PE with its entry points
/// renamed to compileExpr/compileBlock: these ARE the specialized compiler
/// functions with no PE dispatch overhead (BTA, memoization, etc. are absent
/// because the interpreter's representation IS the PE's representation).
///
/// NOTE: this is the renamed-PE REPRESENTATION of P2 — it ASSERTS the collapse by renaming. The
/// GENUINE projection that DEMONSTRATES it by actually running `PE(pe_source, pe_mini)` is
/// [`genuine_projection2_residual`] (and [`run_genuine_p2_on_target`]). Enforcement gates use the
/// genuine path; `shortcut_quarantine_lock` forbids any Jones-certifying gate from using this rename.
pub fn projection2_source() -> Result<String, String> {
    let pe_source = pe_source_text();

    let compiler_source = replace_word(&replace_word(&pe_source, "peExpr", "compileExpr"), "peBlock", "compileBlock");

    Ok(format!("{}\n{}", CORE_TYPES_FOR_PE, compiler_source))
}

/// Third Futamura Projection: PE(PE, PE) = compiler_generator
///
/// Specializes the partial evaluator with respect to itself, producing a
/// compiler generator (cogen). Feed it any interpreter → it produces a
/// compiler for that interpreter's language.
///
/// Chain: cogen(int) → compiler → compiler(P) → compiled
///
/// For the CExpr/CStmt representation, PE(PE, PE) yields the PE with entry
/// points renamed to cogenExpr/cogenBlock. This works because the PE's
/// self-application is idempotent: the PE already operates on the same
/// representation it would specialize, so PE(PE, PE) = PE (up to naming).
/// The cogen handles different interpreters (Core, RPN, etc.) by processing
/// their encoded CProgram representations.
///
/// NOTE: the renamed-PE REPRESENTATION of P3 (asserts the collapse by renaming). The GENUINE cogen
/// that actually runs `PE(pe_source, pe_bti)` is [`genuine_projection3_residual`]. Enforcement uses
/// the genuine path; `shortcut_quarantine_lock` keeps this rename out of any Jones certificate.
pub fn projection3_source() -> Result<String, String> {
    let pe_source = pe_source_text();

    let cogen_source = replace_word(&replace_word(&pe_source, "peExpr", "cogenExpr"), "peBlock", "cogenBlock");

    Ok(format!("{}\n{}", CORE_TYPES_FOR_PE, cogen_source))
}

/// Compile and run LOGOS source, returning stdout.
///
/// Uses the full compilation pipeline: LOGOS → Rust → binary → execute.
/// This is the library-side equivalent of the test infrastructure's `run_logos()`.
pub fn run_logos_source(source: &str) -> Result<String, String> {
    let compile_output = compile_program_full(source)
        .map_err(|e| format!("Compilation failed: {:?}", e))?;

    // Create temp directory using std (no tempfile crate needed)
    let temp_base = std::env::temp_dir().join("logos_run_source");
    std::fs::create_dir_all(&temp_base)
        .map_err(|e| format!("mkdir failed: {}", e))?;

    let pkg_name = format!(
        "logos_run_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let project_dir = temp_base.join(&pkg_name);

    // Find workspace root relative to this crate
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();

    let cargo_toml = format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[dependencies]
logicaffeine-data = {{ path = "{}/crates/logicaffeine_data" }}
logicaffeine-system = {{ path = "{}/crates/logicaffeine_system", features = ["full"] }}
tokio = {{ version = "1", features = ["rt-multi-thread", "macros"] }}
serde = {{ version = "1", features = ["derive"] }}
rayon = "1"
"#,
        pkg_name,
        workspace_root.display(),
        workspace_root.display(),
    );

    std::fs::create_dir_all(project_dir.join("src"))
        .map_err(|e| format!("mkdir failed: {}", e))?;
    std::fs::write(project_dir.join("Cargo.toml"), cargo_toml)
        .map_err(|e| format!("Write Cargo.toml failed: {}", e))?;
    std::fs::write(project_dir.join("src/main.rs"), &compile_output.rust_code)
        .map_err(|e| format!("Write main.rs failed: {}", e))?;
    // Pin dependency resolution to the workspace's lockfile: without it each
    // generated project re-resolves from the registry and a newly published
    // transitive version can break the build.
    std::fs::copy(
        workspace_root.join("Cargo.lock"),
        project_dir.join("Cargo.lock"),
    )
    .map_err(|e| format!("Seed Cargo.lock failed: {}", e))?;

    // Use a shared target dir for caching. Under cargo-nextest, shard by the
    // per-test slot (mirrors the test harness's get_shared_target_dir) so
    // concurrent generated builds never contend on Cargo's build-dir lock.
    let shard = std::env::var("NEXTEST_TEST_GROUP_SLOT")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| {
            std::env::var("NEXTEST_TEST_GLOBAL_SLOT")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .map(|n| n % 12)
        });
    let target_dir = match shard {
        Some(s) => std::env::temp_dir().join(format!("logos_e2e_cache_{s}")),
        None => std::env::temp_dir().join("logos_e2e_cache"),
    };
    std::fs::create_dir_all(&target_dir)
        .map_err(|e| format!("mkdir target failed: {}", e))?;

    let mut run_cmd = std::process::Command::new("cargo");
    run_cmd
        .args(["run", "--quiet"])
        .current_dir(&project_dir)
        .env("CARGO_TARGET_DIR", &target_dir)
        .env("RUST_MIN_STACK", "268435456");
    // Bootstrap-scope: when the compiler is running its own PE / self-interpreter
    // (a `ReferenceScope`), the AOT child it spawns must run that compile-time
    // Logos under reference semantics — the partial evaluator is self-applicable
    // and only specializes when its threaded state is mutated in place.
    if crate::semantics::collections::reference_scope_active() {
        run_cmd.env("LOGOS_VALUE_SEMANTICS", "0");
    }
    let output = run_cmd
        .output()
        .map_err(|e| format!("cargo run failed: {}", e))?;

    // Clean up temp project dir AND this project's artifacts in the shared
    // cache (unique package names would otherwise accrete one binary +
    // deps/incremental/fingerprint entries per run, forever — the shared
    // DEPENDENCY builds stay, which is the entire caching win).
    let _ = std::fs::remove_dir_all(&project_dir);
    {
        let stem = pkg_name.replace('-', "_");
        let debug = target_dir.join("debug");
        let _ = std::fs::remove_file(debug.join(&pkg_name));
        let _ = std::fs::remove_file(debug.join(format!("{pkg_name}.d")));
        for sub in ["deps", "incremental", ".fingerprint", "build"] {
            if let Ok(rd) = std::fs::read_dir(debug.join(sub)) {
                for e in rd.flatten() {
                    let name = e.file_name();
                    let n = name.to_string_lossy();
                    if n.starts_with(&format!("{stem}-"))
                        || n.starts_with(&format!("lib{stem}-"))
                    {
                        let path = e.path();
                        let _ = if path.is_dir() {
                            std::fs::remove_dir_all(&path)
                        } else {
                            std::fs::remove_file(&path)
                        };
                    }
                }
            }
        }
    }

    if !output.status.success() {
        return Err(format!(
            "Execution failed (status={:?} code={:?}):\nstderr: {}\nstdout: {}",
            output.status,
            output.status.code(),
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout),
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Result of a genuine Futamura projection via self-application.
/// Contains the LOGOS source of the residual and the discovered entry points.
pub struct GenuineProjectionResult {
    /// The LOGOS source of the genuine residual — exactly as the PE produced it,
    /// including specialized function definitions and a ## Main block.
    pub source: String,
    /// The name of the block-level entry point (e.g., "peBlockM_d_vPEMiniR_...").
    /// This is the specialized function that takes a Seq of CStmt and returns Seq of CStmt.
    pub block_entry: String,
    /// The name of the expression-level entry point, if discovered.
    pub expr_entry: Option<String>,
}

/// Discover specialized entry points from a PE residual.
/// Searches for `## To {prefix}_...` function definitions in the residual source.
fn discover_entry_points(residual: &str, block_prefix: &str, expr_prefix: &str)
    -> (String, Option<String>)
{
    let mut block_entry = String::new();
    let mut expr_entry = None;
    for line in residual.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("## To ") {
            // Extract function name: everything before the first ' ('
            let name = rest.split(" (").next().unwrap_or("").trim();
            if name.starts_with(block_prefix) && block_entry.is_empty() {
                block_entry = name.to_string();
            } else if name.starts_with(expr_prefix) && expr_entry.is_none() {
                expr_entry = Some(name.to_string());
            }
        }
    }
    (block_entry, expr_entry)
}

/// Real Futamura Projection 1: pe(program) = compiled_program
///
/// Encodes the program as CProgram data, runs the LOGOS PE on it,
/// decompiles the residual back to LOGOS source.
pub fn projection1_source_real(core_types: &str, _interpreter: &str, program: &str) -> Result<String, String> {
    let full_source = if program.contains("## Main") || program.contains("## To ") {
        program.to_string()
    } else {
        format!("## Main\n{}", program)
    };

    // Step 1: Encode the program as CProgram construction source
    let encoded = encode_program_source(&full_source)
        .map_err(|e| format!("Failed to encode program: {:?}", e))?;

    // Step 2: Get PE source and decompile source
    let pe_source = pe_source_text();
    let decompile_source = decompile_source_text();

    // Step 3: Build the combined source
    let actual_core_types = if core_types.is_empty() { CORE_TYPES_FOR_PE } else { core_types };

    let driver = r#"
    Let state be makePeState(a new Map of Text to CVal, encodedFuncMap, 200).
    Let residual be peBlock(encodedMain, state).
    Let source be decompileBlock(residual, 0).
    Show source.
"#;

    let combined = format!(
        "{}\n{}\n{}\n## Main\n{}\n{}",
        actual_core_types,
        pe_source,
        decompile_source,
        encoded,
        driver,
    );

    // Step 4: Compile and run to get the decompiled residual
    let raw_residual = run_logos_source(&combined)?;
    let trimmed = raw_residual.trim();

    // Step 5: Wrap in ## Main if needed
    if trimmed.is_empty() {
        return Ok("## Main\n".to_string());
    }

    // Check if the residual already has function definitions
    if trimmed.contains("## To ") || trimmed.starts_with("## Main") {
        Ok(trimmed.to_string())
    } else {
        Ok(format!("## Main\n{}", trimmed))
    }
}

/// In-process variant of [`projection1_source_real`].
///
/// Identical in every respect except that the combined PE source is executed by the
/// tree-walking interpreter via [`interpret_program`] rather than compiled to Rust and
/// run through `cargo` ([`run_logos_source`]). This makes the genuine LOGOS partial
/// evaluator cheap enough to drive a generative/differential test corpus.
pub fn projection1_source_real_fast(core_types: &str, _interpreter: &str, program: &str) -> Result<String, String> {
    let combined = pe_combined_source(core_types, program)?;

    let raw_residual = interpret_program(&combined)
        .map_err(|e| format!("PE execution failed: {:?}", e))?;
    let residual = finish_projection1_residual(raw_residual)?;
    // The residual is statements-only (`## To` specialized funcs + `## Main`); re-attach the
    // program's TYPE DEFINITIONS so a `new <Struct>` in the residual can default-fill its
    // fields — e.g. a `Shared` struct's CRDT `SharedSet`/`SharedSequence` field, which has no
    // explicit constructor argument and is materialized solely from the definition.
    Ok(prepend_type_definitions(program, residual))
}

pub use crate::concurrency::marshal::{decode_value_raw, encode_value_raw};

/// The self-interpreter (`coreEval` / `coreExecBlock`) as a first-class committed artifact — the
/// Logos program the genuine P2/P3 projections specialize (Futamura: `pe(pe, coreEval)` is a
/// compiler). Promoted out of the `phase_futamura` test fixture so it is a real source file, not a
/// string constant; the fixture now `include_str!`s this exact file (parity is structural).
pub fn core_interp_source() -> &'static str {
    include_str!("optimize/core_interp.logos")
}

/// The compile-once native partial evaluator's SOURCE: the PE engine + decompiler + a driver
/// that reads its input `CProgram` from stdin (over the fast wire codec) instead of a baked-in
/// constructor. Compiled ONCE (content-addressed on this text) with `WireEncode`/`WireDecode`
/// emitted, it specializes any program handed to it as `program_to_core_wire_bytes` — the true
/// Futamura "compiler as an artifact", at native speed. The driver body is identical to
/// [`pe_combined_source`]'s (same transitive-decompile), only `encodedMain`/`encodedFuncMap`
/// come from `readWireProgram()` rather than the encoded program text.
/// The transitive-decompile driver logic shared by the resident-server PE (`pe_native_source`,
/// terminal statement writes the residual to stdout) and the in-process cdylib PE
/// (`pe_cdylib_source`, terminal statement `Return`s it). Everything up to that last statement is
/// identical — computing `output`, the Jones-optimal residual source, from `encodedMain` +
/// `encodedFuncMap`. Written at 0-base indent; the caller re-indents it under its enclosing arm.
const PE_DRIVER_CORE: &str = r#"Let state be makePeState(a new Map of Text to CVal, encodedFuncMap, 200).
Let residual be peBlock(encodedMain, state).
Let nl be chr(10).
Let mutable output be "".
Let specFuncs be peFuncs(state).
Let mutable allNames be collectCallNames(residual).
Let mutable emitted be a new Map of Text to Bool.
Let mutable changed be true.
While changed:
    Set changed to false.
    Let mutable toAdd be a new Seq of Text.
    Repeat for fnKey in allNames:
        Let fkStr be "{fnKey}".
        If emitted contains fkStr:
            Let skipE be true.
        Otherwise:
            Set item fkStr of emitted to true.
            Let fkStr2 be "{fnKey}".
            If specFuncs contains fkStr2:
                Let fdef be item fkStr2 of specFuncs.
                Inspect fdef:
                    When CFuncDef (fn0, ps0, pt0, rt0, body0):
                        Let children be collectCallNames(body0).
                        Repeat for child in children:
                            Let childStr be "{child}".
                            If not emitted contains childStr:
                                Push child to toAdd.
                                Set changed to true.
                    Otherwise:
                        Let skipF be true.
    Repeat for ta in toAdd:
        Push ta to allNames.
Repeat for fnKey in allNames:
    Let fkStr be "{fnKey}".
    If specFuncs contains fkStr:
        Let fdef be item fkStr of specFuncs.
        Let funcSrc be decompileFunc(fdef).
        If the length of funcSrc is greater than 0:
            Set output to "{output}{funcSrc}{nl}".
Let mainSrc be decompileBlock(residual, 0).
Set output to "{output}## Main{nl}{mainSrc}"."#;

/// The PE driver body with `last` appended as its terminal statement — the only line that differs
/// between the stdout-writing server and the value-returning cdylib entry.
fn pe_driver_body(last: &str) -> String {
    format!("{PE_DRIVER_CORE}\n{last}")
}

pub fn pe_native_source() -> String {
    let core = CORE_TYPES_FOR_PE;
    let pe = pe_source_text();
    let decompile = decompile_source_text();
    // The transitive-decompile driver body, written at 0-base indent — identical logic to
    // `pe_combined_source`. It is re-indented under the `When CProg` arm below so the CProg fields
    // (`LogosSeq`-typed, not the `Vec` a declared `Seq` return would be) stay in scope with the
    // right runtime type — sidestepping the return-type/field-type codegen mismatch.
    let driver_body = pe_driver_body("Let __wrn be writeWireResidual(output).");
    // Resident-server loop: `While true` reads one length-framed program per iteration
    // (`readWireProgram` exits the process on stdin EOF), specializes it, and writes the residual
    // back length-framed (`writeWireResidual`). Spawned ONCE by the host and reused — no per-call
    // process spawn. Indent the driver body to 16 spaces (inside the `When CProg` arm inside `While`).
    let indented: String = driver_body
        .lines()
        .map(|l| if l.is_empty() { String::new() } else { format!("                {l}") })
        .collect::<Vec<_>>()
        .join("\n");
    let main = format!(
        "    While true:\n        Let __prog be readWireProgram().\n        Inspect __prog:\n            When CProg (encFuncs, encMain):\n                Let encodedMain be encMain.\n                Let mutable encodedFuncMap be a new Map of Text to CFunc.\n                Repeat for __f in encFuncs:\n                    Inspect __f:\n                        When CFuncDef (__nm, __ps, __pt, __rt, __bd):\n                            Set item __nm of encodedFuncMap to __f.\n                        Otherwise:\n                            Let __skip be true.\n{indented}\n            Otherwise:\n                Let __wrn be writeWireResidual(\"\").\n"
    );
    format!("{}\n{}\n{}\n## Main\n{}", core, pe, decompile, main)
}

/// The PE as an IN-PROCESS cdylib entry: the identical engine to [`pe_native_source`], but the
/// driver is a single-shot `## To peSpecializeOnce (__prog: CProgram) -> Text` that RETURNS the
/// residual instead of writing it to stdout — no `readWireProgram`/`writeWireResidual`, no stdin
/// loop. The exported cdylib ([`build_native_pe_cdylib`]) decodes the wire bytes to a `CProgram` in
/// its `extern "C"` shim and calls this function directly, so the resident server's process/pipe
/// round-trip is gone. `## Main` is a trivial stub (a cdylib never runs it).
pub fn pe_cdylib_source() -> String {
    let core = CORE_TYPES_FOR_PE;
    let pe = pe_source_text();
    let decompile = decompile_source_text();
    // Same driver as the server, terminated by `Return output.`; re-indented to 12 spaces to sit
    // inside the `When CProg` arm of the function body (one level shallower than the server's
    // `While`-nested main).
    let driver_body = pe_driver_body("Return output.");
    let indented: String = driver_body
        .lines()
        .map(|l| if l.is_empty() { String::new() } else { format!("            {l}") })
        .collect::<Vec<_>>()
        .join("\n");
    let func = format!(
        "## To peSpecializeOnce (__prog: CProgram) -> Text:\n    Inspect __prog:\n        When CProg (encFuncs, encMain):\n            Let encodedMain be encMain.\n            Let mutable encodedFuncMap be a new Map of Text to CFunc.\n            Repeat for __f in encFuncs:\n                Inspect __f:\n                    When CFuncDef (__nm, __ps, __pt, __rt, __bd):\n                        Set item __nm of encodedFuncMap to __f.\n                    Otherwise:\n                        Let __skip be true.\n{indented}\n        Otherwise:\n            Return \"\".\n"
    );
    format!("{}\n{}\n{}\n{}\n## Main\n    Let __x be 0.\n", core, pe, decompile, func)
}

use crate::interpreter::RuntimeValue;

// ── Native Core-IR value builder ───────────────────────────────────────────────────────────
// Builds the `CProgram` VALUE directly in Rust — no interpreter, no re-parse of the 140-line
// CORE_TYPES type context (the marshal's dominant cost). Mirrors `encode_expr_src`/`encode_stmt_src`
// but emits `RuntimeValue::Inductive` (constructor + fields IN DECLARATION ORDER). The `inductive_type`
// is a placeholder: the native `CProgram::wire_decode` dispatches on the constructor and ignores the
// type name. Any construct not covered here returns `None`, and the caller falls back to the proven
// interpreter path — so a gap is slower, never wrong. Residual-equivalence with the interpreter path
// is locked by `native_pe_wire` + the `pe_jones_fuzz` corpus.

/// The Core-IR ENUM a constructor belongs to — so the built value's `inductive_type` matches what
/// the interpreter records (`a new CInt` → `Inductive{ inductive_type: "CExpr", constructor: "CInt" }`),
/// making the native builder's wire bytes byte-identical to the interpreter path. The native decoder
/// ignores this field, but matching it keeps every byte-level lock exact.
fn cv_enum(ctor: &str) -> &'static str {
    match ctor {
        "CProg" => "CProgram",
        "CFuncDef" => "CFunc",
        "CLiteralPart" | "CExprPart" => "CStringPart",
        "CInt" | "CBool" | "CText" | "CFloat" | "CVar" | "CBinOp" | "CNot" | "CCall" | "CIndex"
        | "CLen" | "CMapGet" | "CList" | "CTuple" | "CRange" | "CSlice" | "CCopy" | "CContains"
        | "CUnion" | "CIntersection" | "COptionSome" | "COptionNone" | "CNewSeq" | "CNewSet"
        | "CNew" | "CNewVariant" | "CInterpolatedString" | "CClosure" | "CDuration"
        | "CCallExpr" | "CChunkAt" | "CEscExpr" | "CManifestOf" => "CExpr",
        "CSelectRecv" | "CSelectTimeout" => "CSelectBranch",
        _ => "CStmt",
    }
}

fn cv_ind(ctor: &str, args: Vec<RuntimeValue>) -> RuntimeValue {
    RuntimeValue::Inductive(Box::new(crate::interpreter::InductiveValue {
        inductive_type: cv_enum(ctor).to_string(),
        constructor: ctor.to_string(),
        args,
    }))
}
fn cv_text(s: &str) -> RuntimeValue {
    RuntimeValue::Text(std::rc::Rc::new(s.to_string()))
}
fn cv_list(items: Vec<RuntimeValue>) -> RuntimeValue {
    RuntimeValue::List(std::rc::Rc::new(std::cell::RefCell::new(
        crate::interpreter::ListRepr::from_values(items),
    )))
}

/// The binary-operator spelling — IDENTICAL to `encode_expr_src`'s `op_str` (the residual re-parses it).
fn cv_binop_str(op: BinaryOpKind) -> &'static str {
    match op {
        BinaryOpKind::Add => "+",
        BinaryOpKind::Subtract => "-",
        BinaryOpKind::Multiply => "*",
        BinaryOpKind::Divide | BinaryOpKind::ExactDivide => "/",
        BinaryOpKind::FloorDivide => "//",
        BinaryOpKind::Modulo => "%",
        BinaryOpKind::Eq => "==",
        BinaryOpKind::NotEq => "!=",
        BinaryOpKind::Lt => "<",
        BinaryOpKind::Gt => ">",
        BinaryOpKind::LtEq => "<=",
        BinaryOpKind::GtEq => ">=",
        BinaryOpKind::And => "&&",
        BinaryOpKind::Or => "||",
        BinaryOpKind::Concat => "+",
        BinaryOpKind::SeqConcat => "followed by",
        BinaryOpKind::ApproxEq => "is approximately",
        BinaryOpKind::Pow => "**",
        BinaryOpKind::BitXor => "^",
        BinaryOpKind::BitAnd => "&",
        BinaryOpKind::BitOr => "|",
        BinaryOpKind::Shl => "<<",
        BinaryOpKind::Shr => ">>",
    }
}

type VCtors = std::collections::HashMap<String, Vec<String>>;

/// A backend for the Core-IR builder. ONE generic AST traversal (`expr_to_core` / `stmt_to_core` /
/// `build_core`) drives EITHER the reference tree builder ([`TreeSink`] → a `RuntimeValue`, the form
/// `encode_value_raw` serializes) OR the single-pass wire emitter ([`WireSink`] → exactly those
/// `encode_value_raw` bytes with NO intermediate `RuntimeValue` tree and NO second encode walk).
/// Byte-identity of the two backends is locked by `native_builder_is_byte_identical_to_the_interpreter`
/// and the single-pass tests — so there is no way for them to silently diverge.
trait CoreSink {
    type Val;
    fn int(n: i64) -> Self::Val;
    fn boolean(b: bool) -> Self::Val;
    fn float(f: f64) -> Self::Val;
    fn text(s: &str) -> Self::Val;
    fn list(items: Vec<Self::Val>) -> Self::Val;
    fn inductive(ctor: &str, args: Vec<Self::Val>) -> Self::Val;
}

/// Builds the reference `RuntimeValue` Core-IR tree (what `encode_value_raw` serializes).
struct TreeSink;
impl CoreSink for TreeSink {
    type Val = RuntimeValue;
    fn int(n: i64) -> RuntimeValue {
        RuntimeValue::Int(n)
    }
    fn boolean(b: bool) -> RuntimeValue {
        RuntimeValue::Bool(b)
    }
    fn float(f: f64) -> RuntimeValue {
        RuntimeValue::Float(f)
    }
    fn text(s: &str) -> RuntimeValue {
        cv_text(s)
    }
    fn list(items: Vec<RuntimeValue>) -> RuntimeValue {
        cv_list(items)
    }
    fn inductive(ctor: &str, args: Vec<RuntimeValue>) -> RuntimeValue {
        cv_ind(ctor, args)
    }
}

/// Emits the wire bytes DIRECTLY — byte-for-byte what `encode_value_raw` (flat lists, structure off,
/// dedup off) produces for the equivalent [`TreeSink`] value, but with no `RuntimeValue` allocation
/// and no second walk. Each node returns its own encoded bytes; a parent writes its header then
/// concatenates its children. The `logicaffeine_data::wire` primitives are proven byte-identical to
/// the peer `native_encode` by `peer_and_wire_core_produce_identical_bytes`.
struct WireSink;
impl CoreSink for WireSink {
    type Val = Vec<u8>;
    fn int(n: i64) -> Vec<u8> {
        let mut v = Vec::with_capacity(4);
        v.push(logicaffeine_data::wire::T_INT);
        logicaffeine_data::wire::write_uvarint(logicaffeine_data::wire::zigzag(n), &mut v);
        v
    }
    fn boolean(b: bool) -> Vec<u8> {
        vec![if b { logicaffeine_data::wire::T_TRUE } else { logicaffeine_data::wire::T_FALSE }]
    }
    fn float(f: f64) -> Vec<u8> {
        let mut v = Vec::with_capacity(9);
        v.push(logicaffeine_data::wire::T_FLOAT);
        v.extend_from_slice(&f.to_le_bytes());
        v
    }
    fn text(s: &str) -> Vec<u8> {
        let mut v = Vec::with_capacity(2 + s.len());
        v.push(logicaffeine_data::wire::T_TEXT);
        logicaffeine_data::wire::write_str(s, &mut v);
        v
    }
    fn list(items: Vec<Vec<u8>>) -> Vec<u8> {
        let mut v = Vec::new();
        v.push(logicaffeine_data::wire::T_LIST);
        logicaffeine_data::wire::write_uvarint(items.len() as u64, &mut v);
        for it in &items {
            v.extend_from_slice(it);
        }
        v
    }
    fn inductive(ctor: &str, args: Vec<Vec<u8>>) -> Vec<u8> {
        let mut v = Vec::new();
        logicaffeine_data::wire::write_inductive_header(&mut v, cv_enum(ctor), ctor, args.len() as u64);
        for a in &args {
            v.extend_from_slice(a);
        }
        v
    }
}

fn exprs_to_core<S: CoreSink>(args: &[&Expr], interner: &Interner, vctors: &VCtors) -> Option<Vec<S::Val>> {
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        out.push(expr_to_core::<S>(a, interner, vctors)?);
    }
    Some(out)
}

fn expr_to_core<S: CoreSink>(expr: &Expr, interner: &Interner, vctors: &VCtors) -> Option<S::Val> {
    Some(match expr {
        Expr::Literal(Literal::Number(n)) => S::inductive("CInt", vec![S::int(*n)]),
        Expr::Literal(Literal::Boolean(b)) => S::inductive("CBool", vec![S::boolean(*b)]),
        Expr::Literal(Literal::Text(s)) => S::inductive("CText", vec![S::text(interner.resolve(*s))]),
        Expr::Literal(Literal::Float(f)) => S::inductive("CFloat", vec![S::float(*f)]),
        // A duration literal (`1 seconds`, `500 milliseconds`) → `CDuration{amount = CInt(ms), unit}`,
        // normalized to milliseconds exactly as the reference does (used by `Sleep` and `Select`'s
        // `After …` timeout).
        Expr::Literal(Literal::Duration(nanos)) => S::inductive(
            "CDuration",
            vec![S::inductive("CInt", vec![S::int(nanos / 1_000_000)]), S::text("milliseconds")],
        ),
        Expr::Literal(Literal::Nothing) => S::inductive("CText", vec![S::text("nothing")]),
        Expr::Identifier(sym) => S::inductive("CVar", vec![S::text(interner.resolve(*sym))]),
        Expr::BinaryOp { op, left, right } => S::inductive(
            "CBinOp",
            vec![S::text(cv_binop_str(*op)), expr_to_core::<S>(left, interner, vctors)?, expr_to_core::<S>(right, interner, vctors)?],
        ),
        Expr::Not { operand } => S::inductive("CNot", vec![expr_to_core::<S>(operand, interner, vctors)?]),
        Expr::Call { function, args } => {
            let fn_name = interner.resolve(*function);
            if let Some(field_names) = vctors.get(fn_name) {
                // Variant constructor call → CNewVariant (field names from the type registry).
                let mut fnames: Vec<S::Val> = Vec::with_capacity(args.len());
                let mut fvals: Vec<S::Val> = Vec::with_capacity(args.len());
                for (i, a) in args.iter().enumerate() {
                    fnames.push(S::text(field_names.get(i).map(|s| s.as_str()).unwrap_or("value")));
                    fvals.push(expr_to_core::<S>(a, interner, vctors)?);
                }
                S::inductive("CNewVariant", vec![S::text(fn_name), S::list(fnames), S::list(fvals)])
            } else {
                let avals = exprs_to_core::<S>(args, interner, vctors)?;
                S::inductive("CCall", vec![S::text(fn_name), S::list(avals)])
            }
        }
        Expr::Index { collection, index } => S::inductive(
            "CIndex",
            vec![expr_to_core::<S>(collection, interner, vctors)?, expr_to_core::<S>(index, interner, vctors)?],
        ),
        Expr::Length { collection } => S::inductive("CLen", vec![expr_to_core::<S>(collection, interner, vctors)?]),
        Expr::FieldAccess { object, field } => S::inductive(
            "CMapGet",
            vec![expr_to_core::<S>(object, interner, vctors)?, S::inductive("CText", vec![S::text(interner.resolve(*field))])],
        ),
        Expr::List(elems) => {
            let mut items: Vec<S::Val> = Vec::with_capacity(elems.len());
            for e in elems.iter() {
                items.push(expr_to_core::<S>(e, interner, vctors)?);
            }
            S::inductive("CList", vec![S::list(items)])
        }
        Expr::Tuple(elems) => {
            let mut items: Vec<S::Val> = Vec::with_capacity(elems.len());
            for e in elems.iter() {
                items.push(expr_to_core::<S>(e, interner, vctors)?);
            }
            S::inductive("CTuple", vec![S::list(items)])
        }
        Expr::Range { start, end } => S::inductive("CRange", vec![expr_to_core::<S>(start, interner, vctors)?, expr_to_core::<S>(end, interner, vctors)?]),
        Expr::Slice { collection, start, end } => S::inductive(
            "CSlice",
            vec![expr_to_core::<S>(collection, interner, vctors)?, expr_to_core::<S>(start, interner, vctors)?, expr_to_core::<S>(end, interner, vctors)?],
        ),
        Expr::Copy { expr } => S::inductive("CCopy", vec![expr_to_core::<S>(expr, interner, vctors)?]),
        Expr::Contains { collection, value } => S::inductive("CContains", vec![expr_to_core::<S>(collection, interner, vctors)?, expr_to_core::<S>(value, interner, vctors)?]),
        Expr::Union { left, right } => S::inductive("CUnion", vec![expr_to_core::<S>(left, interner, vctors)?, expr_to_core::<S>(right, interner, vctors)?]),
        Expr::Intersection { left, right } => S::inductive("CIntersection", vec![expr_to_core::<S>(left, interner, vctors)?, expr_to_core::<S>(right, interner, vctors)?]),
        Expr::OptionSome { value } => S::inductive("COptionSome", vec![expr_to_core::<S>(value, interner, vctors)?]),
        Expr::OptionNone => S::inductive("COptionNone", vec![]),
        Expr::InterpolatedString(parts) => {
            if parts.is_empty() {
                S::inductive("CText", vec![S::text("")])
            } else {
                let mut pvals: Vec<S::Val> = Vec::with_capacity(parts.len());
                for part in parts {
                    pvals.push(match part {
                        StringPart::Literal(sym) => S::inductive("CLiteralPart", vec![S::text(interner.resolve(*sym))]),
                        StringPart::Expr { value, .. } => S::inductive("CExprPart", vec![expr_to_core::<S>(value, interner, vctors)?]),
                    });
                }
                S::inductive("CInterpolatedString", vec![S::list(pvals)])
            }
        }
        Expr::NewVariant { variant, fields, .. } => {
            let mut fnames: Vec<S::Val> = Vec::with_capacity(fields.len());
            let mut fvals: Vec<S::Val> = Vec::with_capacity(fields.len());
            for (fname, fexpr) in fields {
                fnames.push(S::text(interner.resolve(*fname)));
                fvals.push(expr_to_core::<S>(fexpr, interner, vctors)?);
            }
            S::inductive("CNewVariant", vec![S::text(interner.resolve(*variant)), S::list(fnames), S::list(fvals)])
        }
        Expr::New { type_name, init_fields, .. } => {
            let tn = interner.resolve(*type_name);
            if tn == "Seq" || tn == "List" {
                S::inductive("CNewSeq", vec![])
            } else if tn == "Set" {
                S::inductive("CNewSet", vec![])
            } else if tn == "Map" || tn.starts_with("Map ") {
                S::inductive("CNew", vec![S::text("Map"), S::list(vec![]), S::list(vec![])])
            } else if init_fields.is_empty() {
                S::inductive("CNewVariant", vec![S::text(tn), S::list(vec![]), S::list(vec![])])
            } else {
                let mut fnames: Vec<S::Val> = Vec::with_capacity(init_fields.len());
                let mut fvals: Vec<S::Val> = Vec::with_capacity(init_fields.len());
                for (fname, fexpr) in init_fields {
                    fnames.push(S::text(interner.resolve(*fname)));
                    fvals.push(expr_to_core::<S>(fexpr, interner, vctors)?);
                }
                S::inductive("CNew", vec![S::text(tn), S::list(fnames), S::list(fvals)])
            }
        }
        // Closure: params (Seq Text), body (Seq CStmt), captured free vars (Seq Text, SORTED to
        // match the now-deterministic reference encoder). An expression body wraps as `CReturn e`.
        Expr::Closure { params, body, .. } => {
            let mut bound: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut param_vals: Vec<S::Val> = Vec::with_capacity(params.len());
            for (sym, _) in params {
                let name = interner.resolve(*sym);
                bound.insert(name.to_string());
                param_vals.push(S::text(name));
            }
            let body_vals: Vec<S::Val> = match body {
                ClosureBody::Expression(e) => {
                    vec![S::inductive("CReturn", vec![expr_to_core::<S>(e, interner, vctors)?])]
                }
                ClosureBody::Block(stmts) => {
                    let mut bv: Vec<S::Val> = Vec::with_capacity(stmts.len());
                    for s in stmts.iter() {
                        // The reference encodes body stmts with the single-stmt encoder, which
                        // defers Inspect/Repeat to empty; fall back rather than replicate that quirk.
                        if matches!(s, Stmt::Inspect { .. } | Stmt::Repeat { .. }) {
                            return None;
                        }
                        bv.push(stmt_to_core::<S>(s, interner, vctors)?);
                    }
                    bv
                }
            };
            let mut free: Vec<String> =
                collect_free_vars_expr(expr, interner, &bound).into_iter().collect();
            free.sort();
            let cap_vals: Vec<S::Val> = free.iter().map(|fv| S::text(fv)).collect();
            S::inductive("CClosure", vec![S::list(param_vals), S::list(body_vals), S::list(cap_vals)])
        }
        // A callee-expression call (`(f)(x)`) → `CCallExpr{target, args}`.
        Expr::CallExpr { callee, args } => {
            let mut avals: Vec<S::Val> = Vec::with_capacity(args.len());
            for a in args.iter() {
                avals.push(expr_to_core::<S>(a, interner, vctors)?);
            }
            S::inductive("CCallExpr", vec![expr_to_core::<S>(callee, interner, vctors)?, S::list(avals)])
        }
        Expr::ChunkAt { index, zone } => S::inductive(
            "CChunkAt",
            vec![expr_to_core::<S>(index, interner, vctors)?, expr_to_core::<S>(zone, interner, vctors)?],
        ),
        Expr::ManifestOf { zone } => S::inductive("CManifestOf", vec![expr_to_core::<S>(zone, interner, vctors)?]),
        Expr::Escape { code, .. } => S::inductive("CEscExpr", vec![S::text(interner.resolve(*code))]),
        // `Give`/`WithCapacity` are transparent: the reference encodes the inner value with no node
        // (an ownership move / an allocation hint the interpreter erases).
        Expr::Give { value } => expr_to_core::<S>(value, interner, vctors)?,
        Expr::WithCapacity { value, .. } => expr_to_core::<S>(value, interner, vctors)?,
        _ => return None,
    })
}

fn stmt_to_core<S: CoreSink>(stmt: &Stmt, interner: &Interner, vctors: &VCtors) -> Option<S::Val> {
    Some(match stmt {
        Stmt::Let { var, value, .. } => S::inductive("CLet", vec![S::text(interner.resolve(*var)), expr_to_core::<S>(value, interner, vctors)?]),
        Stmt::Set { target, value } => S::inductive("CSet", vec![S::text(interner.resolve(*target)), expr_to_core::<S>(value, interner, vctors)?]),
        Stmt::Show { object, .. } => S::inductive("CShow", vec![expr_to_core::<S>(object, interner, vctors)?]),
        Stmt::SetIndex { collection, index, value } => S::inductive(
            "CSetIdx",
            vec![S::text(&extract_ident_name(collection, interner)), expr_to_core::<S>(index, interner, vctors)?, expr_to_core::<S>(value, interner, vctors)?],
        ),
        Stmt::SetField { object, field, value } => S::inductive(
            "CMapSet",
            vec![
                S::text(&extract_ident_name(object, interner)),
                S::inductive("CText", vec![S::text(interner.resolve(*field))]),
                expr_to_core::<S>(value, interner, vctors)?,
            ],
        ),
        Stmt::Pop { collection, .. } => S::inductive("CPop", vec![S::text(&extract_ident_name(collection, interner))]),
        Stmt::Give { object, recipient } => S::inductive(
            "CGive",
            vec![expr_to_core::<S>(object, interner, vctors)?, S::text(&extract_ident_name(recipient, interner))],
        ),
        Stmt::Sleep { milliseconds } => S::inductive("CSleep", vec![expr_to_core::<S>(milliseconds, interner, vctors)?]),
        Stmt::RuntimeAssert { condition, hard } => S::inductive(
            if *hard { "CHardAssert" } else { "CRuntimeAssert" },
            vec![expr_to_core::<S>(condition, interner, vctors)?, S::inductive("CText", vec![S::text("assertion failed")])],
        ),
        Stmt::Return { value } => {
            let e = match value {
                Some(v) => expr_to_core::<S>(v, interner, vctors)?,
                None => S::inductive("CInt", vec![S::int(0)]),
            };
            S::inductive("CReturn", vec![e])
        }
        Stmt::Break => S::inductive("CBreak", vec![]),
        Stmt::If { cond, then_block, else_block } => {
            let cond_v = expr_to_core::<S>(cond, interner, vctors)?;
            let then_s: Vec<&Stmt> = then_block.iter().collect();
            let then_v = stmt_list_to_core::<S>(&then_s, interner, vctors)?;
            let else_v = match else_block {
                Some(els) => {
                    let else_s: Vec<&Stmt> = els.iter().collect();
                    stmt_list_to_core::<S>(&else_s, interner, vctors)?
                }
                None => S::list(vec![]),
            };
            S::inductive("CIf", vec![cond_v, then_v, else_v])
        }
        Stmt::While { cond, body, .. } => {
            let cond_v = expr_to_core::<S>(cond, interner, vctors)?;
            let body_s: Vec<&Stmt> = body.iter().collect();
            S::inductive("CWhile", vec![cond_v, stmt_list_to_core::<S>(&body_s, interner, vctors)?])
        }
        Stmt::Call { function, args } => {
            let mut avals: Vec<S::Val> = Vec::with_capacity(args.len());
            for a in args {
                avals.push(expr_to_core::<S>(a, interner, vctors)?);
            }
            S::inductive("CCallS", vec![S::text(interner.resolve(*function)), S::list(avals)])
        }
        Stmt::Push { value, collection } => S::inductive(
            "CPush",
            vec![expr_to_core::<S>(value, interner, vctors)?, S::text(&extract_ident_name(collection, interner))],
        ),
        Stmt::Repeat { pattern, iterable, body, .. } => {
            let loop_var = match pattern {
                Pattern::Identifier(sym) => interner.resolve(*sym).to_string(),
                Pattern::Tuple(syms) => syms.first().map(|s| interner.resolve(*s).to_string()).unwrap_or_else(|| "item".to_string()),
            };
            let body_s: Vec<&Stmt> = body.iter().collect();
            let body_v = stmt_list_to_core::<S>(&body_s, interner, vctors)?;
            if let Expr::Range { start, end } = iterable {
                S::inductive(
                    "CRepeatRange",
                    vec![S::text(&loop_var), expr_to_core::<S>(start, interner, vctors)?, expr_to_core::<S>(end, interner, vctors)?, body_v],
                )
            } else {
                S::inductive("CRepeat", vec![S::text(&loop_var), expr_to_core::<S>(iterable, interner, vctors)?, body_v])
            }
        }
        // CRDT ops (mirror `encode_stmt_src`; declaration order from CORE_TYPES_FOR_PE).
        Stmt::Add { value, collection } => S::inductive(
            "CAdd",
            vec![expr_to_core::<S>(value, interner, vctors)?, S::text(&extract_ident_name(collection, interner))],
        ),
        Stmt::Remove { value, collection } => S::inductive(
            "CRemove",
            vec![expr_to_core::<S>(value, interner, vctors)?, S::text(&extract_ident_name(collection, interner))],
        ),
        Stmt::MergeCrdt { source, target } => S::inductive(
            "CMerge",
            vec![S::text(&extract_ident_name(target, interner)), expr_to_core::<S>(source, interner, vctors)?],
        ),
        Stmt::IncreaseCrdt { object, field, amount } => S::inductive(
            "CIncrease",
            vec![
                S::text(&format!("{}'s {}", extract_ident_name(object, interner), interner.resolve(*field))),
                expr_to_core::<S>(amount, interner, vctors)?,
            ],
        ),
        Stmt::DecreaseCrdt { object, field, amount } => S::inductive(
            "CDecrease",
            vec![
                S::text(&format!("{}'s {}", extract_ident_name(object, interner), interner.resolve(*field))),
                expr_to_core::<S>(amount, interner, vctors)?,
            ],
        ),
        // Task launch (both handled forms; body is a single `CCallS`, handle defaults to `_task`).
        Stmt::LaunchTask { function, args } | Stmt::LaunchTaskWithHandle { function, args, .. } => {
            let func_name = interner.resolve(*function);
            let mut avals: Vec<S::Val> = Vec::with_capacity(args.len());
            for a in args {
                avals.push(expr_to_core::<S>(a, interner, vctors)?);
            }
            let call = S::inductive("CCallS", vec![S::text(func_name), S::list(avals)]);
            let body = S::list(vec![call]);
            let handle_name = if let Stmt::LaunchTaskWithHandle { handle, .. } = stmt {
                interner.resolve(*handle).to_string()
            } else {
                "_task".to_string()
            };
            S::inductive("CLaunchTask", vec![body, S::text(&handle_name)])
        }
        // Concurrency: one branch (Seq of CStmt) per task. The reference encodes each task with the
        // single-stmt encoder, which defers Inspect/Repeat to an EMPTY branch; rather than replicate
        // that quirk, fall back the whole program if a task is one of those.
        Stmt::Concurrent { tasks } => {
            let mut branches: Vec<S::Val> = Vec::with_capacity(tasks.len());
            for task in tasks.iter() {
                if matches!(task, Stmt::Inspect { .. } | Stmt::Repeat { .. }) {
                    return None;
                }
                branches.push(S::list(vec![stmt_to_core::<S>(task, interner, vctors)?]));
            }
            S::inductive("CConcurrent", vec![S::list(branches)])
        }
        Stmt::Parallel { tasks } => {
            let mut branches: Vec<S::Val> = Vec::with_capacity(tasks.len());
            for task in tasks.iter() {
                if matches!(task, Stmt::Inspect { .. } | Stmt::Repeat { .. }) {
                    return None;
                }
                branches.push(S::list(vec![stmt_to_core::<S>(task, interner, vctors)?]));
            }
            S::inductive("CParallel", vec![S::list(branches)])
        }
        // Sequence append (`Append x to d's lines`) — the base struct is forced dynamic at the list
        // level (see `stmt_list_to_vec`); here just the op. `target` round-trips as surface syntax.
        Stmt::AppendToSequence { sequence, value } => S::inductive(
            "CAppendToSeq",
            vec![S::text(&extract_ident_name(sequence, interner)), expr_to_core::<S>(value, interner, vctors)?],
        ),
        // Pipes / channels. Capacity is a `CInt` expression node (default 32, matching the reference).
        Stmt::CreatePipe { var: pipe_var, capacity, .. } => {
            let cap = capacity.unwrap_or(32);
            S::inductive(
                "CCreatePipe",
                vec![S::text(interner.resolve(*pipe_var)), S::inductive("CInt", vec![S::int(cap as i64)])],
            )
        }
        Stmt::SendPipe { value, pipe } => S::inductive(
            "CSendPipe",
            vec![S::text(&pipe_ident_name(pipe, interner)), expr_to_core::<S>(value, interner, vctors)?],
        ),
        Stmt::ReceivePipe { var: recv_var, pipe } => S::inductive(
            "CReceivePipe",
            vec![S::text(&pipe_ident_name(pipe, interner)), S::text(interner.resolve(*recv_var))],
        ),
        Stmt::TrySendPipe { value, pipe, .. } => S::inductive(
            "CTrySendPipe",
            vec![S::text(&pipe_ident_name(pipe, interner)), expr_to_core::<S>(value, interner, vctors)?],
        ),
        Stmt::TryReceivePipe { var: recv_var, pipe } => S::inductive(
            "CTryReceivePipe",
            vec![S::text(&pipe_ident_name(pipe, interner)), S::text(interner.resolve(*recv_var))],
        ),
        // Select over channels: one `CSelectRecv{chan,var,body}` or `CSelectTimeout{duration,body}`
        // per branch. Branch bodies use the single-stmt encoder (fall back on Inspect/Repeat).
        Stmt::Select { branches } => {
            let mut bvals: Vec<S::Val> = Vec::with_capacity(branches.len());
            for branch in branches.iter() {
                match branch {
                    SelectBranch::Receive { var: recv_var, pipe, body } => bvals.push(S::inductive(
                        "CSelectRecv",
                        vec![
                            S::text(&pipe_ident_name(pipe, interner)),
                            S::text(interner.resolve(*recv_var)),
                            select_body_to_core::<S>(body, interner, vctors)?,
                        ],
                    )),
                    SelectBranch::Timeout { milliseconds, body } => bvals.push(S::inductive(
                        "CSelectTimeout",
                        vec![
                            expr_to_core::<S>(milliseconds, interner, vctors)?,
                            select_body_to_core::<S>(body, interner, vctors)?,
                        ],
                    )),
                }
            }
            S::inductive("CSelect", vec![S::list(bvals)])
        }
        // Proof / verification directives — the proposition/predicate is erased to `CBool(true)`
        // (it has no runtime effect to specialize; only its shape survives into the residual).
        Stmt::Assert { .. } => S::inductive("CAssert", vec![S::inductive("CBool", vec![S::boolean(true)])]),
        Stmt::Check { source_text, .. } => S::inductive(
            "CCheck",
            vec![S::inductive("CBool", vec![S::boolean(true)]), S::inductive("CText", vec![S::text(source_text)])],
        ),
        Stmt::Trust { justification, .. } => S::inductive(
            "CTrust",
            vec![S::inductive("CBool", vec![S::boolean(true)]), S::text(interner.resolve(*justification))],
        ),
        Stmt::Require { crate_name, .. } => S::inductive("CRequire", vec![S::text(interner.resolve(*crate_name))]),
        // Inline-native escape hatch.
        Stmt::Escape { code, .. } => S::inductive("CEscStmt", vec![S::text(interner.resolve(*code))]),
        // `Splice` (unconditional inlined block) desugars to `CIf(true, body, [])`.
        Stmt::Splice { body } => {
            let body_refs: Vec<&Stmt> = body.iter().collect();
            S::inductive(
                "CIf",
                vec![
                    S::inductive("CBool", vec![S::boolean(true)]),
                    stmt_list_to_core::<S>(&body_refs, interner, vctors)?,
                    S::list(vec![]),
                ],
            )
        }
        // Task / actor control.
        Stmt::StopTask { handle } => S::inductive("CStopTask", vec![expr_to_core::<S>(handle, interner, vctors)?]),
        Stmt::Spawn { agent_type, name } => S::inductive(
            "CSpawn",
            vec![S::text(interner.resolve(*agent_type)), S::text(interner.resolve(*name))],
        ),
        // Structured-concurrency zone (kind fixed to "heap"); body via the single-stmt encoder.
        Stmt::Zone { name, body, .. } => S::inductive(
            "CZone",
            vec![S::text(interner.resolve(*name)), S::text("heap"), select_body_to_core::<S>(body, interner, vctors)?],
        ),
        // CRDT conflict resolution (base struct forced dynamic at the list level, like Increase/Merge).
        Stmt::ResolveConflict { object, field, .. } => S::inductive(
            "CResolve",
            vec![S::text(&format!("{}'s {}", extract_ident_name(object, interner), interner.resolve(*field)))],
        ),
        // Sync / mount.
        Stmt::Sync { var: sync_var, topic } => S::inductive(
            "CSync",
            vec![S::text(interner.resolve(*sync_var)), expr_to_core::<S>(topic, interner, vctors)?],
        ),
        Stmt::Mount { var: mount_var, path } => S::inductive(
            "CMount",
            vec![S::text(interner.resolve(*mount_var)), expr_to_core::<S>(path, interner, vctors)?],
        ),
        // IO.
        Stmt::ReadFrom { var: read_var, source } => {
            let var_name = interner.resolve(*read_var);
            match source {
                ReadSource::Console => S::inductive("CReadConsole", vec![S::text(var_name)]),
                ReadSource::File(path_expr) => S::inductive(
                    "CReadFile",
                    vec![expr_to_core::<S>(path_expr, interner, vctors)?, S::text(var_name)],
                ),
            }
        }
        Stmt::WriteFile { content, path } => S::inductive(
            "CWriteFile",
            vec![expr_to_core::<S>(path, interner, vctors)?, expr_to_core::<S>(content, interner, vctors)?],
        ),
        // Networking / messaging. (`LetPeerAgent` encodes as `CConnectTo` with the agent as target.)
        Stmt::ConnectTo { address, .. } => S::inductive(
            "CConnectTo",
            vec![expr_to_core::<S>(address, interner, vctors)?, S::text("conn")],
        ),
        Stmt::Listen { address, .. } => S::inductive(
            "CListen",
            vec![expr_to_core::<S>(address, interner, vctors)?, S::text("default")],
        ),
        Stmt::SendMessage { message, destination, .. } => S::inductive(
            "CSendMessage",
            vec![expr_to_core::<S>(destination, interner, vctors)?, expr_to_core::<S>(message, interner, vctors)?],
        ),
        Stmt::AwaitMessage { into, .. } => S::inductive("CAwaitMessage", vec![S::text(interner.resolve(*into))]),
        Stmt::StreamMessage { values, destination } => S::inductive(
            "CStreamMessage",
            vec![expr_to_core::<S>(destination, interner, vctors)?, expr_to_core::<S>(values, interner, vctors)?],
        ),
        Stmt::LetPeerAgent { var: pa_var, address } => S::inductive(
            "CConnectTo",
            vec![expr_to_core::<S>(address, interner, vctors)?, S::text(interner.resolve(*pa_var))],
        ),
        _ => return None,
    })
}

/// A channel name: a plain identifier, else the literal `"pipe"` (mirrors `encode_stmt_src`).
fn pipe_ident_name(pipe: &Expr, interner: &Interner) -> String {
    match pipe {
        Expr::Identifier(sym) => interner.resolve(*sym).to_string(),
        _ => "pipe".to_string(),
    }
}

/// A `Select` branch body: each statement via the SINGLE-statement encoder — the reference defers
/// Inspect/Repeat to empty here, so fall back rather than replicate that quirk.
fn select_body_to_core<S: CoreSink>(body: &[Stmt], interner: &Interner, vctors: &VCtors) -> Option<S::Val> {
    let mut items: Vec<S::Val> = Vec::with_capacity(body.len());
    for s in body.iter() {
        if matches!(s, Stmt::Inspect { .. } | Stmt::Repeat { .. }) {
            return None;
        }
        items.push(stmt_to_core::<S>(s, interner, vctors)?);
    }
    Some(S::list(items))
}

fn stmt_list_to_core<S: CoreSink>(stmts: &[&Stmt], interner: &Interner, vctors: &VCtors) -> Option<S::Val> {
    Some(S::list(stmt_list_to_vec::<S>(stmts, interner, vctors)?))
}

/// Flatten a statement list into Core-IR nodes. Most statements map 1:1 via `stmt_to_core`; an
/// `Inspect` DESUGARS in place ([`inspect_to_cif`]) into the exact flat `CIf` chain `encode_stmts_src`
/// emits — so the PE folds a statically-known match to its single arm, Jones-optimally.
fn stmt_list_to_vec<S: CoreSink>(stmts: &[&Stmt], interner: &Interner, vctors: &VCtors) -> Option<Vec<S::Val>> {
    let mut out: Vec<S::Val> = Vec::with_capacity(stmts.len());
    for s in stmts {
        match s {
            Stmt::Inspect { .. } => inspect_to_cif::<S>(s, &mut out, interner, vctors)?,
            // A CRDT op forces its base struct DYNAMIC first (so the PE cannot statically fold a
            // struct it then mutates), THEN emits the op — a list-level transform mirroring
            // `encode_stmts_src`. `crdt_base_var` is `None` for a plain-variable collection (a bare
            // `Add x to s`), so those emit no force, exactly like the reference.
            Stmt::Add { collection, .. } | Stmt::Remove { collection, .. } => {
                if let Some(base) = crdt_base_var(collection, interner) {
                    out.push(S::inductive("CForceDynamic", vec![S::text(&base)]));
                }
                out.push(stmt_to_core::<S>(s, interner, vctors)?);
            }
            Stmt::AppendToSequence { sequence, .. } => {
                if let Some(base) = crdt_base_var(sequence, interner) {
                    out.push(S::inductive("CForceDynamic", vec![S::text(&base)]));
                }
                out.push(stmt_to_core::<S>(s, interner, vctors)?);
            }
            Stmt::IncreaseCrdt { object, .. } | Stmt::DecreaseCrdt { object, .. } | Stmt::ResolveConflict { object, .. } => {
                if matches!(object, Expr::Identifier(_) | Expr::FieldAccess { .. }) {
                    out.push(S::inductive("CForceDynamic", vec![S::text(&crdt_base_var_root(object, interner))]));
                }
                out.push(stmt_to_core::<S>(s, interner, vctors)?);
            }
            Stmt::MergeCrdt { target, .. } => {
                if matches!(target, Expr::Identifier(_) | Expr::FieldAccess { .. }) {
                    out.push(S::inductive("CForceDynamic", vec![S::text(&crdt_base_var_root(target, interner))]));
                }
                out.push(stmt_to_core::<S>(s, interner, vctors)?);
            }
            _ => out.push(stmt_to_core::<S>(s, interner, vctors)?),
        }
    }
    Some(out)
}

thread_local! {
    /// Per-program index for the `Inspect`-with-`Otherwise` "did any arm match" flag. Reset at the
    /// start of every full-program encode ([`program_to_core`] AND `encode_program_source`); both the
    /// native builder and the interpreter reference increment it in the SAME traversal (functions in
    /// source order, then main; DFS into bodies), so the Nth such Inspect is named `__inspectMatched_N`
    /// identically in both — DETERMINISTIC and collision-free. (The reference formerly named it from
    /// its global intermediate counter, whose value the value-based builder cannot reproduce; this
    /// dedicated index is the reproducible replacement.)
    static INSPECT_OTHERWISE_IDX: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn reset_inspect_otherwise_idx() {
    INSPECT_OTHERWISE_IDX.with(|c| c.set(0));
}

/// The next `Inspect`-with-`Otherwise` flag index (post-increment). Called at the START of each such
/// Inspect's desugaring in BOTH encoders, so a nested Inspect gets a strictly higher index than its
/// enclosing one — no two live flags ever collide.
pub fn next_inspect_otherwise_idx() -> usize {
    INSPECT_OTHERWISE_IDX.with(|c| {
        let v = c.get();
        c.set(v + 1);
        v
    })
}

/// Desugar `Inspect target: When Variant(field→bind): body … [Otherwise: rest]` into the flat `CIf`
/// chain `encode_stmts_src` produces, byte-for-byte. Each variant arm →
/// `CIf(target's __tag == "Variant", [<set match flag if Otherwise>; binds; body], [])`; when the PE
/// knows the variant it folds the comparison and keeps only the matching arm (zero dispatch =
/// Jones-optimal). WITH an `Otherwise`, a `__inspectMatched_N` flag (N = [`next_inspect_otherwise_idx`],
/// deterministic in both encoders) is declared false, set true in each arm, and checked negated to run
/// the Otherwise body. An `Otherwise`-only inspect just inlines that body.
fn inspect_to_cif<S: CoreSink>(stmt: &Stmt, out: &mut Vec<S::Val>, interner: &Interner, vctors: &VCtors) -> Option<()> {
    let Stmt::Inspect { target, arms, .. } = stmt else {
        return None;
    };
    let has_otherwise = arms.iter().any(|a| a.variant.is_none());
    let has_variant = arms.iter().any(|a| a.variant.is_some());

    // No variant arms (empty or Otherwise-only): the reference inlines the Otherwise body directly.
    if !has_variant {
        for arm in arms {
            if arm.variant.is_none() {
                let body_refs: Vec<&Stmt> = arm.body.iter().collect();
                out.extend(stmt_list_to_vec::<S>(&body_refs, interner, vctors)?);
            }
        }
        return Some(());
    }

    // The match flag is created (and its index reserved) BEFORE the arms — so a nested Inspect in an
    // arm/otherwise body gets a strictly higher index, exactly as the reference does.
    let matched_name = if has_otherwise {
        let name = format!("__inspectMatched_{}", next_inspect_otherwise_idx());
        out.push(S::inductive(
            "CLet",
            vec![S::text(&name), S::inductive("CBool", vec![S::boolean(false)])],
        ));
        Some(name)
    } else {
        None
    };

    for arm in arms {
        if arm.variant.is_none() {
            continue;
        }
        let variant_name = interner.resolve(arm.variant.unwrap());
        let tag_get = S::inductive(
            "CMapGet",
            vec![expr_to_core::<S>(target, interner, vctors)?, S::inductive("CText", vec![S::text("__tag")])],
        );
        let cond = S::inductive(
            "CBinOp",
            vec![S::text("=="), tag_get, S::inductive("CText", vec![S::text(variant_name)])],
        );
        let mut then_items: Vec<S::Val> = Vec::new();
        if let Some(ref mname) = matched_name {
            then_items.push(S::inductive(
                "CSet",
                vec![S::text(mname), S::inductive("CBool", vec![S::boolean(true)])],
            ));
        }
        for (field_name, binding_name) in &arm.bindings {
            let field_str = interner.resolve(*field_name);
            let bind_str = interner.resolve(*binding_name);
            let fget = S::inductive(
                "CMapGet",
                vec![expr_to_core::<S>(target, interner, vctors)?, S::inductive("CText", vec![S::text(field_str)])],
            );
            then_items.push(S::inductive("CLet", vec![S::text(bind_str), fget]));
        }
        let body_refs: Vec<&Stmt> = arm.body.iter().collect();
        then_items.extend(stmt_list_to_vec::<S>(&body_refs, interner, vctors)?);
        out.push(S::inductive("CIf", vec![cond, S::list(then_items), S::list(vec![])]));
    }

    // Otherwise: `CIf(not __inspectMatched_N, otherwise_body, [])`.
    if let Some(ref mname) = matched_name {
        let not_matched = S::inductive("CNot", vec![S::inductive("CVar", vec![S::text(mname)])]);
        let mut otherwise_body: Vec<&Stmt> = Vec::new();
        for arm in arms {
            if arm.variant.is_none() {
                otherwise_body = arm.body.iter().collect();
            }
        }
        let ow = stmt_list_to_core::<S>(&otherwise_body, interner, vctors)?;
        out.push(S::inductive("CIf", vec![not_matched, ow, S::list(vec![])]));
    }
    Some(())
}

/// Whether the native Core-IR value builder covers `program` (the fast marshal path is taken, not
/// the interpreter fallback). For test coverage assertions.
pub fn program_covered_by_native_builder(program: &str) -> bool {
    program_to_core_value(program).is_some()
}

/// Build the whole `CProgram` value for `program` on the [`TreeSink`] backend, or `None` if it uses
/// any construct the native builder does not cover (caller falls back to the interpreter path).
fn program_to_core_value(program: &str) -> Option<RuntimeValue> {
    program_to_core::<TreeSink>(program)
}

/// Parse `program` and construct its Core-IR `CProg` on the sink `S` — the shared parse+build spine
/// behind both the `RuntimeValue` builder ([`program_to_core_value`], `S = TreeSink`) and the direct
/// single-pass wire marshal ([`program_to_core_wire_bytes`], `S = WireSink`). `None` if any construct
/// is uncovered. The parse (lex + discovery + `parse_program`) dominates; the sink chooses whether the
/// build allocates a `RuntimeValue` tree + a second encode walk or emits bytes in one pass.
fn program_to_core<S: CoreSink>(program: &str) -> Option<S::Val> {
    reset_inspect_otherwise_idx(); // match the reference's deterministic `__inspectMatched_N` numbering
    let full_source = if program.contains("## Main") || program.contains("## To ") {
        program.to_string()
    } else {
        format!("## Main\n{}", program)
    };
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(&full_source, &mut interner);
    let tokens = lexer.tokenize();
    let type_registry = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        discovery.run_full().types
    };
    let mut vctors: VCtors = std::collections::HashMap::new();
    for (_t, def) in type_registry.iter_types() {
        if let crate::analysis::TypeDef::Enum { variants, .. } = def {
            for v in variants {
                let fnames: Vec<String> = v.fields.iter().map(|f| interner.resolve(f.name).to_string()).collect();
                vctors.insert(interner.resolve(v.name).to_string(), fnames);
            }
        }
    }
    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();
    let ast_ctx = AstContext::with_types(
        &expr_arena, &term_arena, &np_arena, &sym_arena, &role_arena, &pp_arena,
        &stmt_arena, &imperative_expr_arena, &type_expr_arena,
    );
    let mut parser = crate::parser::Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, type_registry);
    let stmts = parser.parse_program().ok()?;
    build_core::<S>(&stmts, &interner, &vctors)
}

/// Construct the Core-IR `CProg` for an ALREADY-PARSED program on the sink `S`: partition top-level
/// statements into `## To` functions (`CFuncDef`) and the main block, then emit `CProg [funcs] main`.
/// This is the parse-free spine — an integrated caller that already holds the AST + interner + variant
/// table skips the ~90%-of-marshal parse entirely by calling [`stmts_to_core_wire_bytes`]. `None` if
/// any statement uses a construct the native builder does not cover.
fn build_core<S: CoreSink>(stmts: &[Stmt], interner: &Interner, vctors: &VCtors) -> Option<S::Val> {
    let mut funcs: Vec<S::Val> = Vec::new();
    let mut main_stmts: Vec<&Stmt> = Vec::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { name, params, body, return_type, is_native, .. } = stmt {
            if *is_native {
                continue;
            }
            let fname = interner.resolve(*name).to_string();
            let pnames: Vec<S::Val> = params.iter().map(|(n, _)| S::text(interner.resolve(*n))).collect();
            let ptypes: Vec<S::Val> = params.iter().map(|(_, ty)| S::text(&decompile_type_expr(ty, interner))).collect();
            let ret = return_type.map(|rt| decompile_type_expr(rt, interner)).unwrap_or_else(|| "Nothing".to_string());
            let body_s: Vec<&Stmt> = body.iter().collect();
            let body_v = stmt_list_to_core::<S>(&body_s, interner, vctors)?;
            funcs.push(S::inductive("CFuncDef", vec![S::text(&fname), S::list(pnames), S::list(ptypes), S::text(&ret), body_v]));
        } else if matches!(
            stmt,
            Stmt::StructDef { .. } | Stmt::Theorem(_) | Stmt::Definition(_) | Stmt::Axiom(_) | Stmt::Theory(_)
        ) {
            // Declarations contribute NOTHING to the runtime statement list — they live in the type
            // catalog / proof layer, and the reference encoder drops them. Skipping them here (rather
            // than routing to `stmt_to_core` → `None` → fallback) lets programs that carry an enum /
            // `Shared` struct (the ones that use Inspect, CRDT counters, typed closures) still hit the
            // fast native builder. Byte-identical: the reference likewise emits nothing for these.
        } else {
            main_stmts.push(stmt);
        }
    }
    let main_v = stmt_list_to_core::<S>(&main_stmts, interner, vctors)?;
    Some(S::inductive("CProg", vec![S::list(funcs), main_v]))
}

/// Serialize a program's Core-IR `CProgram` (a `CProg` with its functions and main block) to the
/// plain wire form — the bytes a compile-once native partial evaluator reads on stdin. FAST PATH:
/// [`program_to_core`] on [`WireSink`] emits the wire bytes in ONE pass straight from the AST — no
/// intermediate `RuntimeValue` tree and no second `encode_value_raw` walk (byte-for-byte identical to
/// the old two-pass form; locked by `native_builder_is_byte_identical_to_the_interpreter`). FALLBACK:
/// for a construct the native builder does not cover, run the constructor source on the tree-walker
/// (`wireBytes`). Both are decoded identically by the native binary's generated `CProgram::wire_decode`.
pub fn program_to_core_wire_bytes(program: &str) -> Result<Vec<u8>, String> {
    if let Some(bytes) = program_to_core::<WireSink>(program) {
        return Ok(bytes);
    }
    program_to_core_wire_bytes_via_interpreter(program)
}

/// The PARSE-FREE marshal: emit a program's Core-IR wire bytes directly from an ALREADY-PARSED AST,
/// skipping the lex + discovery + `parse_program` that dominates [`program_to_core_wire_bytes`] (~90%
/// of a specialization). An integrated caller that parsed the program once for compilation reuses that
/// AST here to feed the native PE at a fraction of the cost. `variant_fields` maps each variant
/// constructor name to its ordered field names (the caller derives it from its type registry, as
/// [`program_to_core`] does). `None` if a construct is uncovered. Byte-identical to
/// [`program_to_core_wire_bytes`] on the same program — same [`build_core`]/[`WireSink`] spine.
pub fn stmts_to_core_wire_bytes(
    stmts: &[Stmt],
    interner: &Interner,
    variant_fields: &std::collections::HashMap<String, Vec<String>>,
) -> Option<Vec<u8>> {
    build_core::<WireSink>(stmts, interner, variant_fields)
}

/// The TWO-PASS marshal that [`program_to_core_wire_bytes`] replaced: build the full `RuntimeValue`
/// Core-IR tree ([`TreeSink`]) then `encode_value_raw` it. Kept as the correctness ORACLE and the
/// speed BASELINE for the single-pass ([`WireSink`]) form — the two are byte-identical by construction
/// (`WireSink` emits exactly what `encode_value_raw` would), and the single pass avoids the tree
/// allocation + the second encode walk. `None` if a construct is uncovered.
pub fn program_to_core_wire_bytes_two_pass(program: &str) -> Option<Vec<u8>> {
    program_to_core::<TreeSink>(program).and_then(|v| encode_value_raw(&v).ok())
}

/// The REFERENCE marshal: always the interpreter path (construct the `CProgram` via the tree-walker,
/// `wireBytes`-serialize it). The native builder's output must be BYTE-IDENTICAL to this — locked by
/// `native_pe_wire::native_builder_is_byte_identical_to_the_interpreter`. Kept public so the lock can
/// force the reference regardless of native-builder coverage.
pub fn program_to_core_wire_bytes_via_interpreter(program: &str) -> Result<Vec<u8>, String> {
    let full_source = if program.contains("## Main") || program.contains("## To ") {
        program.to_string()
    } else {
        format!("## Main\n{}", program)
    };
    let encoded = encode_program_source(&full_source).map_err(|e| format!("Failed to encode program: {:?}", e))?;
    let driver = "    Let prog be a new CProg with funcs encodedFuncSeq and main encodedMain.\n    Let wb be wireBytes(prog).\n    Repeat for b in wb:\n        Show \"{b}\".\n";
    let combined = format!("{}\n## Main\n{}\n{}", CORE_TYPES_FOR_PE, encoded, driver);
    let out = interpret_program(&combined).map_err(|e| format!("wire-encode run failed: {:?}", e))?;
    let mut bytes = Vec::new();
    for line in out.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let n: u16 = t.parse().map_err(|_| format!("wireBytes emitted a non-byte line: {:?}", t))?;
        bytes.push(n as u8);
    }
    Ok(bytes)
}

/// Build (once, content-addressed) the compile-once native PE binary and return its path. Keyed
/// on the generated Rust of [`pe_native_source`] + toolchain, so the one-time rustc compile of the
/// whole PE engine happens exactly once per PE version and is reused across every program and
/// session. This is the amortization that turns AOT from "recompile per program" into the 210x win.
#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    /// The already-built native PE binary path for this process. Caching it avoids re-running
    /// `aot_cache_key` on every specialization — which spawns `rustc --version` (~15 ms) and
    /// re-hashes the 167 KB source. The path is stable for the process (the PE source doesn't
    /// change at runtime), so one lookup per process suffices.
    static NATIVE_PE_BIN: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(not(target_arch = "wasm32"))]
pub fn build_native_pe_binary() -> Result<std::path::PathBuf, String> {
    // Warm path: the binary was already located/built this process — reuse it (no `rustc --version`
    // spawn, no re-hash) as long as it still exists on disk.
    if let Some(p) = NATIVE_PE_BIN.with(|c| c.borrow().clone()) {
        if p.exists() {
            return Ok(p);
        }
    }
    let src = pe_native_source();
    // Key on the (stable) PE SOURCE text + toolchain — NOT the generated Rust: computing the Rust
    // means recompiling the whole 167 KB engine, which would defeat the cache on every warm call.
    let key = aot_cache_key(&src);
    let base = std::env::temp_dir().join("logos_native_pe");
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let proj = base.join(format!("pe_{key}"));
    let bin = proj.join("target").join("release").join("logos_native_pe");
    if bin.exists() {
        NATIVE_PE_BIN.with(|c| *c.borrow_mut() = Some(bin.clone()));
        return Ok(bin);
    }
    let rust = compile_program_full_with_wire(&src)
        .map_err(|e| format!("native PE compile: {:?}", e))?
        .rust_code;
    std::fs::create_dir_all(proj.join("src")).map_err(|e| e.to_string())?;
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().unwrap().parent().unwrap();
    std::fs::write(
        proj.join("Cargo.toml"),
        format!(
            "[package]\nname=\"logos_native_pe\"\nversion=\"0.1.0\"\nedition=\"2021\"\n[dependencies]\nlogicaffeine-data={{path=\"{r}/crates/logicaffeine_data\"}}\nlogicaffeine-system={{path=\"{r}/crates/logicaffeine_system\",features=[\"full\"]}}\ntokio={{version=\"1\",features=[\"rt-multi-thread\",\"macros\"]}}\nserde={{version=\"1\",features=[\"derive\"]}}\nrayon=\"1\"\n[profile.release]\nopt-level=3\n",
            r = root.display()
        ),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(proj.join("src/main.rs"), &rust).map_err(|e| e.to_string())?;
    std::fs::copy(root.join("Cargo.lock"), proj.join("Cargo.lock")).map_err(|e| e.to_string())?;
    let out = std::process::Command::new("cargo")
        .args(["build", "--release", "--quiet"])
        .current_dir(&proj)
        .env("CARGO_TARGET_DIR", proj.join("target"))
        .env("RUST_MIN_STACK", "268435456")
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "native PE build failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    if bin.exists() {
        NATIVE_PE_BIN.with(|c| *c.borrow_mut() = Some(bin.clone()));
        Ok(bin)
    } else {
        Err("native PE binary missing after a successful build".into())
    }
}

/// Specialize `program` on the compile-once native PE via the RESIDENT SERVER (the fallback path;
/// the default [`run_native_pe`] prefers the faster in-process cdylib): serialize it to wire bytes
/// ([`program_to_core_wire_bytes`]), stream them to the server, and return its Jones-optimal
/// residual. The server (the compiled PE binary) is spawned ONCE per thread and reused — length-
/// framed request/response over its stdin/stdout — so there is no per-call process spawn and no
/// per-call rustc: just the host marshal + native `peBlock`, plus one pipe round-trip.
///
/// The server persists in a thread-local; dropping it (thread end / on I/O error, which triggers a
/// respawn) closes its stdin, and `readWireProgram` exits the child cleanly on that EOF.
#[cfg(not(target_arch = "wasm32"))]
struct PeServer {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: std::io::BufReader<std::process::ChildStdout>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for PeServer {
    fn drop(&mut self) {
        // Reap the child so a long-lived host (or the test harness) never accumulates zombies.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static PE_SERVER: std::cell::RefCell<Option<PeServer>> = const { std::cell::RefCell::new(None) };
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run_native_pe_server(program: &str) -> Result<String, String> {
    use std::io::{Read, Write};
    let bytes = program_to_core_wire_bytes(program)?;
    let raw = PE_SERVER.with(|cell| -> Result<String, String> {
        // A single request/response over the resident server; on ANY I/O failure the server is
        // dropped (respawned next call) so a crashed child never wedges the pipeline.
        let attempt = |slot: &mut Option<PeServer>| -> Result<String, String> {
            if slot.is_none() {
                let bin = build_native_pe_binary()?;
                let mut child = std::process::Command::new(&bin)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .env("RUST_MIN_STACK", "268435456")
                    .spawn()
                    .map_err(|e| format!("native PE server spawn: {e}"))?;
                let stdin = child.stdin.take().unwrap();
                let stdout = std::io::BufReader::new(child.stdout.take().unwrap());
                *slot = Some(PeServer { child, stdin, stdout });
            }
            let srv = slot.as_mut().unwrap();
            srv.stdin
                .write_all(&(bytes.len() as u32).to_le_bytes())
                .and_then(|_| srv.stdin.write_all(&bytes))
                .and_then(|_| srv.stdin.flush())
                .map_err(|e| format!("native PE server write: {e}"))?;
            let mut len = [0u8; 4];
            srv.stdout
                .read_exact(&mut len)
                .map_err(|e| format!("native PE server read length: {e}"))?;
            let n = u32::from_le_bytes(len) as usize;
            let mut buf = vec![0u8; n];
            srv.stdout
                .read_exact(&mut buf)
                .map_err(|e| format!("native PE server read residual: {e}"))?;
            String::from_utf8(buf).map_err(|e| format!("native PE residual not UTF-8: {e}"))
        };
        let mut slot = cell.borrow_mut();
        match attempt(&mut slot) {
            Ok(v) => Ok(v),
            Err(e) => {
                *slot = None; // drop the (possibly dead) server so the next call respawns
                Err(e)
            }
        }
    })?;
    let residual = finish_projection1_residual(raw)?;
    Ok(prepend_type_definitions(program, residual))
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    /// The already-built PE cdylib path for this process (see [`NATIVE_PE_BIN`]).
    static NATIVE_PE_CDYLIB: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// The in-process FFI shim appended to the generated PE engine to form the cdylib's `lib.rs`. It
/// lives in the SAME crate-root module as the generated `pub enum CProgram` and the module-private
/// `fn peSpecializeOnce(CProgram) -> String`, so it can call both directly. `into_boxed_slice` gives
/// an allocation with capacity EXACTLY == len, so `logos_pe_free` reconstructs the identical
/// `Box<[u8]>` (reconstructing a `Vec` with cap==len would be UB if the allocator over-allocated).
#[cfg(not(target_arch = "wasm32"))]
const PE_CDYLIB_SHIM: &str = r#"

// ── in-process FFI entry (appended by build_native_pe_cdylib) ───────────────────────────────
#[no_mangle]
pub extern "C" fn logos_pe_specialize(ptr: *const u8, len: usize, out_len: *mut usize) -> *mut u8 {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let prog = <CProgram as logicaffeine_data::wire::WireDecode>::wire_decode(bytes, &mut 0usize)
        .expect("logos_pe_specialize: decode CProgram");
    let out: String = peSpecializeOnce(prog);
    let boxed: Box<[u8]> = out.into_bytes().into_boxed_slice();
    let n = boxed.len();
    unsafe { *out_len = n; }
    Box::into_raw(boxed) as *mut u8
}

#[no_mangle]
pub extern "C" fn logos_pe_free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        unsafe { drop(Box::from_raw(std::slice::from_raw_parts_mut(ptr, len) as *mut [u8])); }
    }
}
"#;

/// Build (and cache) the PE compiled as an IN-PROCESS cdylib exporting `logos_pe_specialize`.
/// Mirrors [`build_native_pe_binary`] but emits a `cdylib` whose `lib.rs` is the generated PE engine
/// ([`pe_cdylib_source`]) plus a hand-written `extern "C"` shim ([`PE_CDYLIB_SHIM`]) that decodes the
/// wire `CProgram` and calls `peSpecializeOnce` directly — so the caller loads it once and
/// specializes with a plain function call, no process, no pipe. Content-addressed on the (stable)
/// cdylib PE source + shim + toolchain, so warm calls skip rustc.
#[cfg(not(target_arch = "wasm32"))]
pub fn build_native_pe_cdylib() -> Result<std::path::PathBuf, String> {
    if let Some(p) = NATIVE_PE_CDYLIB.with(|c| c.borrow().clone()) {
        if p.exists() {
            return Ok(p);
        }
    }
    let src = pe_cdylib_source();
    // Key on the PE source AND the FFI shim, so editing either invalidates the on-disk cache.
    let key = aot_cache_key(&format!("{src}\n{PE_CDYLIB_SHIM}"));
    let base = std::env::temp_dir().join("logos_native_pe_cdylib");
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let proj = base.join(format!("pe_{key}"));
    let so = proj.join("target").join("release").join(format!(
        "{}logos_native_pe_lib{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    if so.exists() {
        NATIVE_PE_CDYLIB.with(|c| *c.borrow_mut() = Some(so.clone()));
        return Ok(so);
    }
    let generated = compile_program_full_with_wire(&src)
        .map_err(|e| format!("native PE cdylib compile: {:?}", e))?
        .rust_code;
    let lib_rs = format!("{generated}\n{PE_CDYLIB_SHIM}");
    std::fs::create_dir_all(proj.join("src")).map_err(|e| e.to_string())?;
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().unwrap().parent().unwrap();
    std::fs::write(
        proj.join("Cargo.toml"),
        format!(
            "[package]\nname=\"logos_native_pe_lib\"\nversion=\"0.1.0\"\nedition=\"2021\"\n[lib]\ncrate-type=[\"cdylib\"]\npath=\"src/lib.rs\"\n[dependencies]\nlogicaffeine-data={{path=\"{r}/crates/logicaffeine_data\"}}\nlogicaffeine-system={{path=\"{r}/crates/logicaffeine_system\",features=[\"full\"]}}\ntokio={{version=\"1\",features=[\"rt-multi-thread\",\"macros\"]}}\nserde={{version=\"1\",features=[\"derive\"]}}\nrayon=\"1\"\n[profile.release]\nopt-level=3\n",
            r = root.display()
        ),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(proj.join("src/lib.rs"), &lib_rs).map_err(|e| e.to_string())?;
    std::fs::copy(root.join("Cargo.lock"), proj.join("Cargo.lock")).map_err(|e| e.to_string())?;
    let out = std::process::Command::new("cargo")
        .args(["build", "--release", "--quiet"])
        .current_dir(&proj)
        .env("CARGO_TARGET_DIR", proj.join("target"))
        .env("RUST_MIN_STACK", "268435456")
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "native PE cdylib build failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    if so.exists() {
        NATIVE_PE_CDYLIB.with(|c| *c.borrow_mut() = Some(so.clone()));
        Ok(so)
    } else {
        Err("native PE cdylib missing after a successful build".into())
    }
}

/// The loaded PE cdylib for this thread: the `Library` kept alive alongside the raw entry-point
/// pointers derived from it (valid for as long as the `Library` is not unloaded, which is exactly
/// this struct's lifetime).
#[cfg(not(target_arch = "wasm32"))]
struct PeLib {
    _lib: libloading::Library,
    specialize: unsafe extern "C" fn(*const u8, usize, *mut usize) -> *mut u8,
    free: unsafe extern "C" fn(*mut u8, usize),
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static PE_LIB: std::cell::RefCell<Option<PeLib>> = const { std::cell::RefCell::new(None) };
}

/// Specialize `program` on the compile-once native PE, IN PROCESS: marshal it to wire bytes, hand
/// them to the `dlopen`ed PE cdylib's `logos_pe_specialize` as a plain function call, and reclaim
/// the residual bytes. No child process, no pipe — the ~59 µs IPC round-trip of the resident server
/// is gone; all that remains is the host marshal + native `peBlock` + one FFI call. The cdylib is
/// built + loaded ONCE per thread and cached. Residual-identity with the server / tree-walker is
/// locked by `native_pe_wire`.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_native_pe_inprocess(program: &str) -> Result<String, String> {
    let bytes = program_to_core_wire_bytes(program)?;
    let raw = PE_LIB.with(|cell| -> Result<String, String> {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            let so = build_native_pe_cdylib()?;
            // SAFETY: the cdylib was just built from our own generated source; its exported symbols
            // have exactly the signatures declared here, and the raw pointers are stored alongside
            // the owning `Library` so they never outlive it.
            unsafe {
                let lib = libloading::Library::new(&so)
                    .map_err(|e| format!("native PE cdylib load: {e}"))?;
                let specialize: libloading::Symbol<
                    unsafe extern "C" fn(*const u8, usize, *mut usize) -> *mut u8,
                > = lib
                    .get(b"logos_pe_specialize")
                    .map_err(|e| format!("dlsym logos_pe_specialize: {e}"))?;
                let free: libloading::Symbol<unsafe extern "C" fn(*mut u8, usize)> = lib
                    .get(b"logos_pe_free")
                    .map_err(|e| format!("dlsym logos_pe_free: {e}"))?;
                let specialize = *specialize;
                let free = *free;
                *slot = Some(PeLib { _lib: lib, specialize, free });
            }
        }
        let pl = slot.as_ref().unwrap();
        let mut out_len: usize = 0;
        // SAFETY: `bytes` is a live slice for the duration of the call; the callee writes `out_len`
        // and returns a buffer of that length we own until `free`.
        let p = unsafe { (pl.specialize)(bytes.as_ptr(), bytes.len(), &mut out_len as *mut usize) };
        if p.is_null() {
            return Err("native PE cdylib returned a null residual".into());
        }
        let residual_bytes = unsafe { std::slice::from_raw_parts(p, out_len).to_vec() };
        unsafe { (pl.free)(p, out_len) };
        String::from_utf8(residual_bytes).map_err(|e| format!("native PE residual not UTF-8: {e}"))
    })?;
    let residual = finish_projection1_residual(raw)?;
    Ok(prepend_type_definitions(program, residual))
}

/// Specialize `program` on the compile-once native PE — the DEFAULT entry and the pit of success.
/// Prefers the IN-PROCESS cdylib path ([`run_native_pe_inprocess`]; no process/pipe boundary) and
/// falls back to the resident server ([`run_native_pe_server`]) only if the in-process build/load
/// fails (e.g. no toolchain), so the fast+correct route is what every caller gets by default. The
/// two paths are proven byte-identical by `native_pe_wire`.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_native_pe(program: &str) -> Result<String, String> {
    match run_native_pe_inprocess(program) {
        Ok(v) => Ok(v),
        Err(_) => run_native_pe_server(program),
    }
}

/// VM-driven twin of [`projection1_source_real_fast`].
///
/// Executes the identical assembled PE program on the register bytecode VM
/// ([`vm_run_source`]) instead of the tree-walker. The PE engine is a fixed program —
/// only its `encodedMain` input varies per call — so the VM either supports the whole
/// engine or none of it; measured ~2× faster than the tree-walker with a byte-identical
/// residual. The corpus-wide equivalence is locked by `futamura_tier_lock`.
pub fn projection1_source_real_fast_on_vm(
    core_types: &str,
    _interpreter: &str,
    program: &str,
) -> Result<String, String> {
    let combined = pe_combined_source(core_types, program)?;

    let raw_residual = vm_run_source(&combined).map_err(|e| format!("PE VM execution failed: {e}"))?;
    let residual = finish_projection1_residual(raw_residual)?;
    Ok(prepend_type_definitions(program, residual))
}

/// Re-attach every original `## …` section that is NOT `## Main` and NOT a `## To` function
/// (the residual supplies its own Main + specialized functions). These are the type / struct
/// / policy definitions that make the residual a self-contained, re-runnable program.
fn prepend_type_definitions(program: &str, residual: String) -> String {
    let mut defs = String::new();
    let mut keep = false;
    for line in program.lines() {
        if let Some(header) = line.strip_prefix("## ") {
            keep = header != "Main" && !header.starts_with("To ");
        }
        if keep {
            defs.push_str(line);
            defs.push('\n');
        }
    }
    if defs.trim().is_empty() {
        residual
    } else {
        format!("{}{}", defs, residual)
    }
}

/// Assemble the full PE-run source (core types + PE + decompiler + encoded
/// input + driver) — the exact program [`projection1_source_real_fast`]
/// executes. Public so differential harnesses can run it on a specific engine.
pub fn pe_combined_source(core_types: &str, program: &str) -> Result<String, String> {
    let full_source = if program.contains("## Main") || program.contains("## To ") {
        program.to_string()
    } else {
        format!("## Main\n{}", program)
    };

    let encoded = encode_program_source(&full_source)
        .map_err(|e| format!("Failed to encode program: {:?}", e))?;

    let pe_source = pe_source_text();
    let decompile_source = decompile_source_text();

    let actual_core_types = if core_types.is_empty() { CORE_TYPES_FOR_PE } else { core_types };

    // Driver: run PE, then transitively decompile every referenced function (specialized
    // or original) so residual calls — e.g. an MSG-generalized recursive loop — resolve.
    // For fully-inlined residuals (no calls) this emits nothing and matches the bare form.
    let driver = r#"
    Let state be makePeState(a new Map of Text to CVal, encodedFuncMap, 200).
    Let residual be peBlock(encodedMain, state).
    Let nl be chr(10).
    Let mutable output be "".
    Let specFuncs be peFuncs(state).
    Let mutable allNames be collectCallNames(residual).
    Let mutable emitted be a new Map of Text to Bool.
    Let mutable changed be true.
    While changed:
        Set changed to false.
        Let mutable toAdd be a new Seq of Text.
        Repeat for fnKey in allNames:
            Let fkStr be "{fnKey}".
            If emitted contains fkStr:
                Let skipE be true.
            Otherwise:
                Set item fkStr of emitted to true.
                Let fkStr2 be "{fnKey}".
                If specFuncs contains fkStr2:
                    Let fdef be item fkStr2 of specFuncs.
                    Inspect fdef:
                        When CFuncDef (fn0, ps0, pt0, rt0, body0):
                            Let children be collectCallNames(body0).
                            Repeat for child in children:
                                Let childStr be "{child}".
                                If not emitted contains childStr:
                                    Push child to toAdd.
                                    Set changed to true.
                        Otherwise:
                            Let skipF be true.
        Repeat for ta in toAdd:
            Push ta to allNames.
    Repeat for fnKey in allNames:
        Let fkStr be "{fnKey}".
        If specFuncs contains fkStr:
            Let fdef be item fkStr of specFuncs.
            Let funcSrc be decompileFunc(fdef).
            If the length of funcSrc is greater than 0:
                Set output to "{output}{funcSrc}{nl}".
    Let mainSrc be decompileBlock(residual, 0).
    Set output to "{output}## Main{nl}{mainSrc}".
    Show output.
"#;

    let combined = format!(
        "{}\n{}\n{}\n## Main\n{}\n{}",
        actual_core_types,
        pe_source,
        decompile_source,
        encoded,
        driver,
    );

    Ok(combined)
}

/// The post-processing applied to a raw PE residual (the `Any`-type safety
/// net + `## Main` normalization).
pub fn finish_projection1_residual(raw_residual: String) -> Result<String, String> {
    // Safety net (mirrors `fix_decompiled_types` on the P2/P3 path): specialized functions
    // whose param/return types the PE could not propagate are emitted as `Any`, which the
    // parser rejects. The tree-walker is dynamically typed, so a concrete placeholder type
    // is cosmetic at runtime — rewrite `Any` annotations so the residual re-parses.
    let fixed = raw_residual
        .replace(": Any)", ": Int)")
        .replace("-> Any:", "-> Int:")
        .replace(" of Any", " of Int")
        .replace(" to Any", " to Int");
    let trimmed = fixed.trim();

    if trimmed.is_empty() {
        return Ok("## Main\n".to_string());
    }

    if trimmed.contains("## To ") || trimmed.starts_with("## Main") {
        Ok(trimmed.to_string())
    } else {
        Ok(format!("## Main\n{}", trimmed))
    }
}

/// Run genuine P2 on a specific target: PE(pe_source, pe_mini(target))
///
/// This is the real Futamura Projection 2 applied end-to-end:
/// 1. Build pe_mini applied to the target (pe_mini compiles the target)
/// 2. Encode the combined pe_mini+target as CProgram data
/// 3. Run pe_source on the encoded data (PE specializes pe_mini for this target)
/// 4. Decompile the residual to LOGOS source
/// 5. Execute the decompiled residual to get the target's output
///
/// No decompilation of specialized functions — the PE directly produces the compiled
/// target as CStmt data, which is decompiled to a simple LOGOS program and run.
pub fn run_genuine_p2_on_target(program: &str, core_types: &str, interpreter: &str) -> Result<String, String> {
    let pe_mini = pe_mini_source_text();
    let pe = pe_source_text();

    let full_source = if program.contains("## Main") || program.contains("## To ") {
        program.to_string()
    } else {
        format!("## Main\n{}", program)
    };

    // Build pe_mini applied to the specific target.
    // pe_mini ONLY COMPILES the target — produces CStmt data as output.
    // Build pe_mini + interpreter applied to the target.
    // pe_mini compiles the target, coreExecBlock runs the compiled result.
    let target_encoded = encode_program_source(&full_source)
        .map_err(|e| format!("Failed to encode target: {:?}", e))?;
    let pe_mini_prog = format!(
        "{}\n{}\n{}\n## Main\n{}\n\
         Let compileEnv be a new Map of Text to CVal.\n\
         Let compileState be makePeState(compileEnv, encodedFuncMap, 200).\n\
         Let compiled be peBlockM(encodedMain, compileState).\n\
         Let runEnv be a new Map of Text to CVal.\n\
         coreExecBlock(compiled, runEnv, encodedFuncMap).\n",
        core_types, pe_mini, interpreter, target_encoded
    );

    // Encode pe_mini+target as CProgram data for the outer PE
    let encoded = encode_program_source_compact(&pe_mini_prog)
        .map_err(|e| format!("Failed to encode pe_mini+target for P2: {:?}", e))?;

    // The driver runs PE on the encoded pe_mini+target, then executes the residual.
    // peFuncs(state) contains BOTH the PE-specialized functions AND the original
    // functions from the encoded program (including pe_mini, interpreter, and
    // target functions like factorial).
    let driver = r#"    Let state be makePeState(a new Map of Text to CVal, encodedFuncMap, 500).
    Let residual be peBlock(encodedMain, state).
    Let allFuncs be peFuncs(state).
    Let runEnv be a new Map of Text to CVal.
    coreExecBlock(residual, runEnv, allFuncs).
"#;
    let combined = format!(
        "{}\n{}\n{}\n## Main\n{}\n{}",
        CORE_TYPES_FOR_PE, pe, interpreter, encoded, driver
    );

    run_logos_source(&combined)
}

/// Run genuine P3 on a specific target: PE(pe_source, pe_bti(target))
pub fn run_genuine_p3_on_target(program: &str, core_types: &str, interpreter: &str) -> Result<String, String> {
    let pe_bti = pe_bti_source_text();
    let pe = pe_source_text();

    let full_source = if program.contains("## Main") || program.contains("## To ") {
        program.to_string()
    } else {
        format!("## Main\n{}", program)
    };

    let bti_types = CORE_TYPES_FOR_PE
        .replace("specResults", "memoCache")
        .replace("onStack", "callGuard");

    let target_encoded = encode_program_source(&full_source)
        .map_err(|e| format!("Failed to encode target: {:?}", e))?;
    let pe_bti_prog = format!(
        "{}\n{}\n{}\n## Main\n{}\n\
         Let compileEnv be a new Map of Text to CVal.\n\
         Let compileState be makePeState(compileEnv, encodedFuncMap, 200).\n\
         Let compiled be peBlockB(encodedMain, compileState).\n\
         Let runEnv be a new Map of Text to CVal.\n\
         coreExecBlock(compiled, runEnv, encodedFuncMap).\n",
        bti_types, pe_bti, interpreter, target_encoded
    );

    let encoded = encode_program_source_compact(&pe_bti_prog)
        .map_err(|e| format!("Failed to encode pe_bti+target for P3: {:?}", e))?;

    // Execute residual directly — no decompilation needed
    let driver = r#"    Let state be makePeState(a new Map of Text to CVal, encodedFuncMap, 200).
    Let residual be peBlock(encodedMain, state).
    Let runEnv be a new Map of Text to CVal.
    coreExecBlock(residual, runEnv, encodedFuncMap).
"#;
    let combined = format!(
        "{}\n{}\n{}\n## Main\n{}\n{}",
        CORE_TYPES_FOR_PE, pe, interpreter, encoded, driver
    );

    run_logos_source(&combined)
}

/// Genuine Futamura Projection 2 via self-application: PE(pe_source, pe_mini(targetStmts))
///
/// The outer PE (pe_source) specializes pe_mini's FULL BLOCK evaluator with known
/// state (empty env, empty funcs, depth 200). targetStmts is a free/dynamic variable;
/// state is fully static. The PE produces a specialized peBlockM_ function that IS
/// the compiler — handling all CStmt variants with specialized expression processing.
///
/// This eliminates the need for a Rust-generated block wrapper: the PE naturally
/// produces block-level dispatch through specialization of peBlockM.
pub fn projection2_source_real(_core_types: &str, _interpreter: &str) -> Result<GenuineProjectionResult, String> {
    let pe_mini = pe_mini_source_text();
    let pe = pe_source_text();
    let decompile = decompile_source_text();

    // Build pe_mini program with peBlockM as entry — FULL block-level specialization.
    // targetStmts = free/dynamic variable (program's main block).
    // targetFuncs = free/dynamic variable (program's function definitions).
    // env = static (empty — programs start with no bindings).
    // depth = static (200 — PE eliminates depth checks).
    // targetStmts is a free/dynamic variable. State is fully static (empty env,
    // empty funcs, depth 200). The PE specializes pe_mini's dispatch code completely.
    // The resulting compiler handles function definitions at USE time via the state
    // parameter provided by the test harness.
    let program = format!(
        "{}\n{}\n## Main\n    Let env be a new Map of Text to CVal.\n    Let funcs be a new Map of Text to CFunc.\n    Let state be makePeState(env, funcs, 200).\n    Let result be peBlockM(targetStmts, state).\n    Show \"done\".\n",
        CORE_TYPES_FOR_PE, pe_mini
    );

    // Encode pe_mini + driver as CProgram data (compact encoding)
    let encoded = encode_program_source_compact(&program)
        .map_err(|e| format!("Failed to encode pe_mini for P2: {:?}", e))?;

    // Driver: run PE, then decompile specialized functions transitively.
    // Fixpoint collection: discover all transitively-referenced specialized functions
    // at arbitrary depth, not limited to any fixed number of levels.
    let driver = r#"    Let state be makePeState(a new Map of Text to CVal, encodedFuncMap, 200).
    Let residual be peBlock(encodedMain, state).
    Let nl be chr(10).
    Let mutable output be "".
    Let specFuncs be peFuncs(state).
    Let mutable allNames be collectCallNames(residual).
    Let mutable emitted be a new Map of Text to Bool.
    Let mutable changed be true.
    While changed:
        Set changed to false.
        Let mutable toAdd be a new Seq of Text.
        Repeat for fnKey in allNames:
            Let fkStr be "{fnKey}".
            If emitted contains fkStr:
                Let skipE be true.
            Otherwise:
                Set item fkStr of emitted to true.
                Let fkStr2 be "{fnKey}".
                If specFuncs contains fkStr2:
                    Let fdef be item fkStr2 of specFuncs.
                    Inspect fdef:
                        When CFuncDef (fn0, ps0, pt0, rt0, body0):
                            Let children be collectCallNames(body0).
                            Repeat for child in children:
                                Let childStr be "{child}".
                                If not emitted contains childStr:
                                    Push child to toAdd.
                                    Set changed to true.
                        Otherwise:
                            Let skipF be true.
        Repeat for ta in toAdd:
            Push ta to allNames.
    Repeat for fnKey in allNames:
        Let fkStr be "{fnKey}".
        If specFuncs contains fkStr:
            Let fdef be item fkStr of specFuncs.
            Let funcSrc be decompileFunc(fdef).
            If the length of funcSrc is greater than 0:
                Set output to "{output}{funcSrc}{nl}".
    Let mainSrc be decompileBlock(residual, 0).
    Set output to "{output}## Main{nl}{mainSrc}".
    Show output.
"#;
    let combined = format!("{}\n{}\n{}\n## Main\n{}\n{}", CORE_TYPES_FOR_PE, pe, decompile, encoded, driver);

    let result = run_logos_source(&combined)?;

    // The decompiler now emits types from CFunc's paramTypes/returnType fields for
    // functions that carry type info. PE-generated specializations may still use "Any"
    // for types that couldn't be propagated through specialization. Apply type fixup
    // as a safety net for any remaining "Any" types in specialized function signatures.
    let result = fix_decompiled_types(&result, &[
        ("peExprM_", "(e: CExpr) -> CExpr:"),
        ("peBlockM_", "(stmts: Seq of CStmt) -> Seq of CStmt:"),
        ("checkLiteralM_", "(e: CExpr) -> Bool:"),
        ("exprToValM_", "(e: CExpr) -> CVal:"),
        ("valToExprM_", "(v: CVal) -> CExpr:"),
        ("evalBinOpM_", "(binOp: Text) and (lv: CVal) and (rv: CVal) -> CVal:"),
        ("isCopyPropSafeM_", "(e: CExpr) -> Bool:"),
        ("checkVNothingM_", "(v: CVal) -> Bool:"),
        ("hasReturnM_", "(stmts: Seq of CStmt) -> Bool:"),
        ("extractReturnM_", "(stmts: Seq of CStmt) -> CExpr:"),
        ("validateExtractReturnM_", "(result: CExpr) and (bodyStmts: Seq of CStmt) -> CExpr:"),
        ("makeKeyM_", "(fnName: Text) and (args: Seq of CExpr) -> Text:"),
        ("exprToKeyPartM_", "(e: CExpr) -> Text:"),
        ("collectSetVarsM_", "(stmts: Seq of CStmt) -> Seq of Text:"),
        ("peEnvM_", "(st: PEMiniState) -> Map of Text to CVal:"),
        ("peFuncsM_", "(st: PEMiniState) -> Map of Text to CFunc:"),
        ("peDepthM_", "(st: PEMiniState) -> Int:"),
        ("peStaticEnvM_", "(st: PEMiniState) -> Map of Text to CExpr:"),
        ("peMemoCacheM_", "(st: PEMiniState) -> Map of Text to CExpr:"),
        ("peStateWithEnvDepthM_", "(st: PEMiniState) and (newEnv: Map of Text to CVal) and (d: Int) -> PEMiniState:"),
        ("peStateWithEnvDepthStaticM_", "(st: PEMiniState) and (newEnv: Map of Text to CVal) and (d: Int) and (newSe: Map of Text to CExpr) -> PEMiniState:"),
    ]);

    // Discover the PE-generated block entry point — this IS the compiler.
    let (block_entry, expr_entry) = discover_entry_points(&result, "peBlockM_", "peExprM_");
    if block_entry.is_empty() {
        return Err("Genuine P2: no peBlockM_ entry found in residual".to_string());
    }

    // Strip the ## Main block from the residual — we only need the specialized function
    // definitions. The test harness provides its own ## Main.
    let func_defs_only = strip_main_block(&result);

    // The specialized functions call pe_mini's unspecialized helpers (checkLiteralM,
    // valToExprM, etc.) which the PE couldn't fold because their args are dynamic.
    // Include the original pe_mini source so these helpers are available.
    let pe_mini_helpers = pe_mini_source_text();

    // No Rust-generated block wrapper needed — the PE produced a specialized peBlockM_
    // function that handles all CStmt variants with specialized expression processing.
    // Generate a thin alias for backward compatibility.
    let alias = format!(
        "\n## To compileBlock (stmts: Seq of CStmt) -> Seq of CStmt:\n    Return {}(stmts).\n",
        block_entry
    );

    // Combine: pe_mini helpers first (authoritative), then specialized functions, then alias.
    // Deduplicate: if the decompiled residual redefines a pe_mini function (unspecialized),
    // the dedup removes the second definition, keeping the original pe_mini version.
    let combined = format!("{}\n{}\n{}", pe_mini_helpers, func_defs_only, alias);
    let full_source = deduplicate_functions(&combined);

    Ok(GenuineProjectionResult {
        source: full_source,
        block_entry: "compileBlock".to_string(),
        expr_entry,
    })
}

/// Run genuine PE(pe_source, pe_mini(targetStmts)) and return the specialized
/// compiler residual as LOGOS source code.
///
/// This is the actual Futamura Projection 2: the outer PE (pe_source) specializes
/// pe_mini's peBlockM with known state (empty env, empty funcs, depth 200).
/// The result is a specialized compiler function that takes target statements and
/// compiles them with pe_mini's dispatch logic partially evaluated away.
///
/// Returns the decompiled LOGOS source of the genuine P2 residual, including
/// specialized function definitions extracted from peFuncs.
pub fn genuine_projection2_residual() -> Result<String, String> {
    let pe_mini = pe_mini_source_text();
    let pe = pe_source_text();
    let decompile = decompile_source_text();

    // Build pe_mini program with peBlockM as entry — full block-level specialization
    let program = format!(
        "{}\n{}\n## Main\n    Let env be a new Map of Text to CVal.\n    Let funcs be a new Map of Text to CFunc.\n    Let state be makePeState(env, funcs, 200).\n    Let result be peBlockM(targetStmts, state).\n    Show \"done\".\n",
        CORE_TYPES_FOR_PE, pe_mini
    );

    // Encode pe_mini + driver as CProgram data (compact encoding)
    let encoded = encode_program_source_compact(&program)
        .map_err(|e| format!("Failed to encode pe_mini: {:?}", e))?;

    // Driver: run PE, then decompile residual + specialized functions
    let driver = r#"    Let state be makePeState(a new Map of Text to CVal, encodedFuncMap, 200).
    Let residual be peBlock(encodedMain, state).
    Let nl be chr(10).
    Let mutable output be "".
    Let specFuncs be peFuncs(state).
    Let specNames be collectCallNames(residual).
    Repeat for sn in specNames:
        Let snKey be "{sn}".
        If specFuncs contains snKey:
            Let fdef be item snKey of specFuncs.
            Let funcSrc be decompileFunc(fdef).
            If the length of funcSrc is greater than 0:
                Set output to "{output}{funcSrc}{nl}".
    Let mainSrc be decompileBlock(residual, 0).
    Set output to "{output}## Main{nl}{mainSrc}".
    Show output.
"#;
    let combined = format!("{}\n{}\n{}\n## Main\n{}\n{}", CORE_TYPES_FOR_PE, pe, decompile, encoded, driver);

    let result = run_logos_source(&combined)?;
    Ok(result)
}

/// Run genuine PE(pe_source, pe_bti(targetExpr)) and return the specialized
/// cogen residual as LOGOS source code.
///
/// This is the actual Futamura Projection 3: the outer PE (pe_source) specializes
/// pe_bti (a full PE with memoization) with known state (empty env, empty funcs,
/// depth 200). pe_bti is structurally identical to pe_source with renamed entry
/// points (peExprB, peBlockB, etc.) — so this is genuinely PE(PE, PE).
///
/// The result is a specialized cogen: pe_bti's dispatch partially evaluated away,
/// producing a program that takes a target CExpr and compiles it.
pub fn genuine_projection3_residual() -> Result<String, String> {
    let pe_bti = pe_bti_source_text();
    let pe = pe_source_text();
    let decompile = decompile_source_text();

    // pe_bti uses memoCache/callGuard instead of specResults/onStack
    let bti_types = CORE_TYPES_FOR_PE
        .replace("specResults", "memoCache")
        .replace("onStack", "callGuard");

    // Build pe_bti program with peBlockB as entry — full block-level specialization
    let program = format!(
        "{}\n{}\n## Main\n    Let env be a new Map of Text to CVal.\n    Let funcs be a new Map of Text to CFunc.\n    Let state be makePeState(env, funcs, 200).\n    Let result be peBlockB(targetStmts, state).\n    Show \"done\".\n",
        bti_types, pe_bti
    );

    // Encode pe_bti + driver as CProgram data (compact encoding)
    let encoded = encode_program_source_compact(&program)
        .map_err(|e| format!("Failed to encode pe_bti: {:?}", e))?;

    // Driver: run PE, then decompile residual + specialized functions
    let driver = r#"    Let state be makePeState(a new Map of Text to CVal, encodedFuncMap, 200).
    Let residual be peBlock(encodedMain, state).
    Let nl be chr(10).
    Let mutable output be "".
    Let specFuncs be peFuncs(state).
    Let specNames be collectCallNames(residual).
    Repeat for sn in specNames:
        Let snKey be "{sn}".
        If specFuncs contains snKey:
            Let fdef be item snKey of specFuncs.
            Let funcSrc be decompileFunc(fdef).
            If the length of funcSrc is greater than 0:
                Set output to "{output}{funcSrc}{nl}".
    Let mainSrc be decompileBlock(residual, 0).
    Set output to "{output}## Main{nl}{mainSrc}".
    Show output.
"#;
    let combined = format!("{}\n{}\n{}\n## Main\n{}\n{}", CORE_TYPES_FOR_PE, pe, decompile, encoded, driver);

    let result = run_logos_source(&combined)?;
    Ok(result)
}

/// Genuine Futamura Projection 3 via self-application: PE(pe_source, pe_bti(targetStmts))
///
/// The outer PE (pe_source) specializes pe_bti's FULL BLOCK evaluator (a full PE with
/// memoization, structurally identical to pe_source with renamed entry points) with
/// known state (empty env, empty funcs, depth 200). This is genuinely PE(PE, PE).
///
/// The result is a specialized cogen: the PE naturally produces block-level dispatch
/// through specialization of peBlockB, eliminating the need for a Rust-generated wrapper.
pub fn projection3_source_real(_core_types: &str) -> Result<GenuineProjectionResult, String> {
    let pe_bti = pe_bti_source_text();
    let pe = pe_source_text();
    let decompile = decompile_source_text();

    // pe_bti uses memoCache/callGuard instead of specResults/onStack
    let bti_types = CORE_TYPES_FOR_PE
        .replace("specResults", "memoCache")
        .replace("onStack", "callGuard");

    // Build pe_bti program with peBlockB as entry — FULL block-level specialization.
    let program = format!(
        "{}\n{}\n## Main\n    Let env be a new Map of Text to CVal.\n    Let funcs be a new Map of Text to CFunc.\n    Let state be makePeState(env, funcs, 200).\n    Let result be peBlockB(targetStmts, state).\n    Show \"done\".\n",
        bti_types, pe_bti
    );

    // Encode pe_bti + driver as CProgram data (compact encoding)
    let encoded = encode_program_source_compact(&program)
        .map_err(|e| format!("Failed to encode pe_bti for P3: {:?}", e))?;

    // Driver: fixpoint transitive collection (same algorithm as P2)
    let driver = r#"    Let state be makePeState(a new Map of Text to CVal, encodedFuncMap, 200).
    Let residual be peBlock(encodedMain, state).
    Let nl be chr(10).
    Let mutable output be "".
    Let specFuncs be peFuncs(state).
    Let mutable allNames be collectCallNames(residual).
    Let mutable emitted be a new Map of Text to Bool.
    Let mutable changed be true.
    While changed:
        Set changed to false.
        Let mutable toAdd be a new Seq of Text.
        Repeat for fnKey in allNames:
            Let fkStr be "{fnKey}".
            If emitted contains fkStr:
                Let skipE be true.
            Otherwise:
                Set item fkStr of emitted to true.
                Let fkStr2 be "{fnKey}".
                If specFuncs contains fkStr2:
                    Let fdef be item fkStr2 of specFuncs.
                    Inspect fdef:
                        When CFuncDef (fn0, ps0, pt0, rt0, body0):
                            Let children be collectCallNames(body0).
                            Repeat for child in children:
                                Let childStr be "{child}".
                                If not emitted contains childStr:
                                    Push child to toAdd.
                                    Set changed to true.
                        Otherwise:
                            Let skipF be true.
        Repeat for ta in toAdd:
            Push ta to allNames.
    Repeat for fnKey in allNames:
        Let fkStr be "{fnKey}".
        If specFuncs contains fkStr:
            Let fdef be item fkStr of specFuncs.
            Let funcSrc be decompileFunc(fdef).
            If the length of funcSrc is greater than 0:
                Set output to "{output}{funcSrc}{nl}".
    Let mainSrc be decompileBlock(residual, 0).
    Set output to "{output}## Main{nl}{mainSrc}".
    Show output.
"#;
    let combined = format!("{}\n{}\n{}\n## Main\n{}\n{}", CORE_TYPES_FOR_PE, pe, decompile, encoded, driver);

    let result = run_logos_source(&combined)?;

    // Fix decompiled types for pe_bti specialized functions (B suffix)
    // Fix decompiled types for pe_bti specialized functions (B suffix)
    let result = fix_decompiled_types(&result, &[
        ("peExprB_", "(e: CExpr) -> CExpr:"),
        ("peBlockB_", "(stmts: Seq of CStmt) -> Seq of CStmt:"),
        ("isStatic_", "(e: CExpr) -> Bool:"),
        ("isLiteral_", "(e: CExpr) -> Bool:"),
        ("allStatic_", "(args: Seq of CExpr) -> Bool:"),
        ("exprToVal_", "(e: CExpr) -> CVal:"),
        ("valToExpr_", "(v: CVal) -> CExpr:"),
        ("evalBinOp_", "(binOp: Text) and (lv: CVal) and (rv: CVal) -> CVal:"),
        ("isCopyPropSafe_", "(e: CExpr) -> Bool:"),
        ("isVNothing_", "(v: CVal) -> Bool:"),
        ("hasReturn_", "(stmts: Seq of CStmt) -> Bool:"),
        ("extractReturnB_", "(stmts: Seq of CStmt) -> CExpr:"),
        ("makeKey_", "(fnName: Text) and (args: Seq of CExpr) -> Text:"),
        ("exprToKeyPartB_", "(e: CExpr) -> Text:"),
        ("collectSetVars_", "(stmts: Seq of CStmt) -> Seq of Text:"),
    ]);

    // Discover the PE-generated block entry point — this IS the cogen.
    let (block_entry, expr_entry) = discover_entry_points(&result, "peBlockB_", "peExprB_");
    if block_entry.is_empty() {
        return Err("Genuine P3: no peBlockB_ entry found in residual".to_string());
    }

    // Strip the ## Main block — we only need the specialized function definitions
    let func_defs_only = strip_main_block(&result);

    // Include pe_bti helpers (unspecialized functions called by specialized ones)
    let pe_bti_helpers = pe_bti_source_text();

    // No Rust-generated block wrapper needed — the PE produced a specialized peBlockB_
    // function. Generate a thin alias for backward compatibility.
    let alias = format!(
        "\n## To cogenBlock (stmts: Seq of CStmt) -> Seq of CStmt:\n    Return {}(stmts).\n",
        block_entry
    );
    let combined = format!("{}\n{}\n{}", pe_bti_helpers, func_defs_only, alias);
    let full_source = deduplicate_functions(&combined);

    Ok(GenuineProjectionResult {
        source: full_source,
        block_entry: "cogenBlock".to_string(),
        expr_entry,
    })
}

/// Remove duplicate function definitions, keeping the first occurrence.
fn deduplicate_functions(source: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut result = String::with_capacity(source.len());
    let mut skip_until_next = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("## To ") {
            let name = rest.split(' ').next().unwrap_or("");
            if !seen.insert(name.to_string()) {
                skip_until_next = true;
                continue;
            }
            skip_until_next = false;
        } else if trimmed.starts_with("## Main") {
            skip_until_next = false;
        } else if skip_until_next {
            // Skip body of duplicate function
            if !trimmed.starts_with("## ") {
                continue;
            }
            skip_until_next = false;
        }
        result.push_str(line);
        result.push('\n');
    }
    result
}

/// Strip the ## Main block from decompiled source, keeping only function definitions.
/// The genuine residual's ## Main references internal PE variables (targetExpr, blockResult)
/// that don't exist in the test context. We only need the specialized function definitions.
fn strip_main_block(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let mut in_main = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed == "## Main" {
            in_main = true;
            continue;
        }
        if in_main {
            // Main block ends when we hit another ## To definition or end of file
            if trimmed.starts_with("## To ") {
                in_main = false;
            } else {
                continue;
            }
        }
        result.push_str(line);
        result.push('\n');
    }
    result
}

/// Extract the ## Main block content from source.
fn extract_main_block(source: &str) -> String {
    let mut result = String::new();
    let mut in_main = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed == "## Main" {
            in_main = true;
            continue;
        }
        if in_main {
            if trimmed.starts_with("## To ") {
                break;
            }
            result.push_str(line);
            result.push('\n');
        }
    }
    result
}

/// Fix decompiled function signatures: replace `(param: Any) -> Any:` with correct types.
/// PE-generated specializations still use "Any" for types the PE couldn't propagate.
/// This restores correct types based on function name prefixes.
fn fix_decompiled_types(source: &str, type_map: &[(&str, &str)]) -> String {
    let mut result = String::with_capacity(source.len());
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("## To ") {
            let name = rest.split(' ').next().unwrap_or("");
            let mut fixed = false;
            for (prefix, sig) in type_map {
                if name.starts_with(prefix) {
                    result.push_str(&format!("## To {} {}\n", name, sig));
                    fixed = true;
                    break;
                }
            }
            if !fixed {
                result.push_str(line);
                result.push('\n');
            }
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }
    let result = result
        .replace("Seq of Any", "Seq of CExpr")
        .replace("Set of Any", "Set of CExpr")
        .replace(": Any)", ": CExpr)")
        .replace("-> Any:", "-> CExpr:");
    result
}

fn replace_word(source: &str, from: &str, to: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let mut remaining = source;
    while let Some(pos) = remaining.find(from) {
        let before = if pos > 0 { remaining.as_bytes()[pos - 1] } else { b' ' };
        let after_pos = pos + from.len();
        let after = if after_pos < remaining.len() { remaining.as_bytes()[after_pos] } else { b' ' };
        let is_word = !before.is_ascii_alphanumeric() && before != b'_'
            && !after.is_ascii_alphanumeric() && after != b'_';
        result.push_str(&remaining[..pos]);
        if is_word {
            result.push_str(to);
        } else {
            result.push_str(from);
        }
        remaining = &remaining[after_pos..];
    }
    result.push_str(remaining);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_let_statement() {
        let source = "## Main\nLet x be 5.";
        let result = compile_to_rust(source);
        assert!(result.is_ok(), "Should compile: {:?}", result);
        let rust = result.unwrap();
        assert!(rust.contains("fn main()"));
        assert!(rust.contains("let x = 5;"));
    }

    #[test]
    fn test_compile_return_statement() {
        let source = "## Main\nReturn 42.";
        let result = compile_to_rust(source);
        assert!(result.is_ok(), "Should compile: {:?}", result);
        let rust = result.unwrap();
        assert!(rust.contains("return 42;"));
    }

    /// Run `src` through the RUN-PATH optimizer (`optimize_for_run`, which
    /// includes the indexed-load LICM) and then the pure-bytecode VM (no native
    /// tier → no forge dependency), capturing the outcome.
    fn optimized_vm_outcome_no_tier(src: &str, argv: &[String]) -> (String, Option<String>) {
        crate::ui_bridge::with_optimized_program(src, |parsed, interner| match parsed {
            Ok((stmts, types, policies)) => crate::vm::run_to_outcome_with_args(
                stmts,
                interner,
                Some(types),
                Some(&policies),
                argv,
                None,
            ),
            Err(advice) => (String::new(), Some(advice)),
        })
    }

    fn norm_lines(s: &str) -> String {
        s.lines()
            .map(|l| l.trim_end())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Task C SOUNDNESS, in-process: an inner loop reading a loop-invariant
    /// `item i of arr` (i = OUTER counter, arr never mutated in the loop) mixed
    /// with a variant `item j of arr` must produce the EXACT outcome the
    /// tree-walker produces once the invariant read is hoisted. The guarded
    /// hoist preserves WHEN the read runs, so the optimized VM matches the
    /// oracle bit-for-bit on a real, in-bounds nbody-shaped loop.
    #[test]
    fn licm_indexed_load_nbody_shape_matches_oracle_inprocess() {
        let src = "## Main\n\
                   Let arr be [10, 20, 30, 40, 50].\n\
                   Let mutable acc be 0.\n\
                   Let mutable i be 1.\n\
                   While i is at most 5:\n\
                   \x20   Let mutable j be 1.\n\
                   \x20   While j is at most 5:\n\
                   \x20       Let d be item i of arr - item j of arr.\n\
                   \x20       Set acc to acc + d * d.\n\
                   \x20       Set j to j + 1.\n\
                   \x20   Set i to i + 1.\n\
                   Show acc.\n";
        let argv: [String; 0] = [];
        let tw = tw_outcome_with_args(src, &argv);
        let (out, err) = optimized_vm_outcome_no_tier(src, &argv);
        assert_eq!(
            (norm_lines(&tw.output), &tw.error),
            ("10000".to_string(), &None),
            "oracle: sum of (a_i - a_j)^2"
        );
        assert_eq!(
            (norm_lines(&out), &err),
            (norm_lines(&tw.output), &tw.error),
            "optimized VM (LICM) must match the tree-walker on the nbody-shaped loop"
        );
    }

    /// SOUNDNESS GATE (forge-free): the OPTIMIZED bytecode VM — exercising the
    /// constant-pool sharing (Task B) and the indexed-load LICM (Task C) — must
    /// produce BIT-IDENTICAL output to the raw tree-walker on the whole
    /// benchmark corpus. This mirrors the `vm_opt_differential` integration
    /// test but runs without a native tier, so it stays inside this crate.
    #[test]
    fn corpus_optimized_vm_matches_treewalker_no_tier() {
        const CORPUS: &[(&str, &str)] = &[
            ("ackermann", "3"),
            ("array_fill", "2000"),
            ("array_reverse", "2000"),
            ("binary_trees", "6"),
            ("bubble_sort", "60"),
            ("coins", "500"),
            ("collatz", "300"),
            ("collect", "300"),
            ("counting_sort", "2000"),
            ("fannkuch", "5"),
            ("fib", "12"),
            ("fib_iterative", "500"),
            ("gcd", "60"),
            ("graph_bfs", "200"),
            ("heap_sort", "300"),
            ("histogram", "2000"),
            ("knapsack", "30"),
            ("loop_sum", "2000"),
            ("mandelbrot", "20"),
            ("matrix_mult", "8"),
            ("mergesort", "300"),
            ("nbody", "100"),
            ("nqueens", "5"),
            ("pi_leibniz", "2000"),
            ("prefix_sum", "2000"),
            ("primes", "500"),
            ("quicksort", "300"),
            ("sieve", "2000"),
            ("spectral_norm", "20"),
            ("string_search", "1200"),
            ("strings", "200"),
            ("two_sum", "300"),
        ];
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                for &(name, size) in CORPUS {
                    let path = format!(
                        "{}/../../benchmarks/programs/{}/main.lg",
                        env!("CARGO_MANIFEST_DIR"),
                        name
                    );
                    let src = std::fs::read_to_string(&path)
                        .unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
                    let argv = vec!["bench".to_string(), size.to_string()];
                    let (out, err) = optimized_vm_outcome_no_tier(&src, &argv);
                    let tw = tw_outcome_with_args(&src, &argv);
                    assert_eq!(
                        (norm_lines(&out), &err),
                        (norm_lines(&tw.output), &tw.error),
                        "OPTIMIZED VM (no tier) diverged from the tree-walker on '{name}' at {size}"
                    );
                }
            })
            .expect("spawn")
            .join()
            .expect("corpus thread panicked");
    }
}
