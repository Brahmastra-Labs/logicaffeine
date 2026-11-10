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
//! Escape and ownership analysis judge the program as written — the freshly-parsed statements,
//! before observation elision, division resolution or the optimizer rewrite them — so a verdict
//! never depends on what the optimizer folded, and matches the editor's.
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

use std::collections::HashMap;
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
use crate::ast::stmt::{
    BinaryOpKind, ClosureBody, Literal, Pattern, ReadSource, SelectBranch, StringPart,
};
use crate::ast::{Expr, Stmt, TypeExpr};
use crate::codegen::{
    codegen_program, generate_c_header, generate_python_bindings, generate_typescript_bindings,
};
use crate::diagnostic::{parse_rustc_json, translate_diagnostics, LogosError};
use crate::drs::WorldState;
use crate::error::ParseError;
use crate::intern::Interner;
use crate::lexer::{
    char_literal, date_literal, duration_literal, moment_literal, span_literal, text_literal, text_literal_body,
    time_literal, Lexer,
};
use crate::parser::Parser;
use crate::semantics::collections::with_reference_semantics;
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

/// [`interpret_program`] for one of the compiler's own LOGOS programs — the partial evaluator, its
/// dialects, the self-interpreter, a compiler a projection produced — inside the bootstrap scope
/// ([`with_reference_semantics`]): those programs thread their state through `mutable` parameters and
/// are written for the in-place writes of reference semantics. A user program never runs here.
///
/// It runs on the tree-walker, on a thread of its own ([`on_bootstrap_stack`]). A bootstrap program runs once per
/// projection, and compiling the specializer's few thousand lines to bytecode costs several times the run itself.
pub fn interpret_bootstrap(source: &str) -> Result<String, ParseError> {
    let source = source.to_string();
    on_bootstrap_stack(move || with_reference_semantics(|| run_on_the_tree_walker(&source)))
}

/// A bootstrap program run as an ordinary program: [`interpret_bootstrap`]'s run outside the bootstrap scope, under
/// the language's value semantics — the host the host-agnostic proofs hold a bootstrap program to mean the same on.
pub fn interpret_bootstrap_on_the_value_host(source: &str) -> Result<String, ParseError> {
    let source = source.to_string();
    on_bootstrap_stack(move || run_on_the_tree_walker(&source))
}

/// `source` run on the tree-walker under the semantics in force, its output lines joined.
fn run_on_the_tree_walker(source: &str) -> Result<String, ParseError> {
    let result = crate::ui_bridge::with_parsed_program(source, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => crate::ui_bridge::run_treewalker(
            stmts,
            types,
            policies,
            interner,
            crate::interpreter::needs_async(stmts, interner),
            &[],
        ),
        Err(advice) => crate::interpreter::InterpreterResult {
            lines: Vec::new(),
            error: Some(advice),
        },
    });
    match result.error {
        Some(err) => Err(ParseError {
            kind: crate::error::ParseErrorKind::Custom(err),
            span: crate::token::Span::default(),
        }),
        None => Ok(result.lines.join("\n")),
    }
}

/// The stack a bootstrap program runs on: an interpreter interpreting an interpreter recurses as deep as both.
const BOOTSTRAP_STACK: usize = 1 << 30;

/// `f` on a thread with [`BOOTSTRAP_STACK`] — the depth a bootstrap program reaches is its own, never its caller's —
/// a panic in it resuming in the caller. In the browser, where a thread cannot be spawned, `f` runs here.
fn on_bootstrap_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    #[cfg(target_arch = "wasm32")]
    {
        f()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::Builder::new()
            .name("bootstrap".to_string())
            .stack_size(BOOTSTRAP_STACK)
            .spawn(f)
            .expect("a bootstrap program runs on a thread of its own")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
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

/// The CANONICAL set of value kinds that must be FIRST-CLASS as a function parameter AND return value on
/// EVERY execution tier — the single source of truth SHARED across crates so the coverage locks cannot
/// drift apart. The tests crate's `param_return_tier_coverage_lock` asserts its PROBES name EXACTLY this
/// set (executed on TW/VM/AOT-C/AOT-Rust, lowered on WASM), and this crate's
/// `wasm_aot_lock::first_class_param_return_round_trips_on_wasm` asserts it has an EXECUTABLE WASM program
/// for every kind here. Adding a kind here makes BOTH locks RED until each covers it — anti-drift by
/// construction, compiler-enforced (both crates reference this one `const`). Keep alphabetically grouped
/// scalars → sequences → collections → aggregates for readability; order is not significant (compared as
/// a set).
pub const FIRST_CLASS_VALUE_KINDS: &[&str] = &[
    "int",
    "real",
    "bool",
    "text",
    "seq_int",
    "seq_text",
    "seq_seq",
    "set_int",
    "struct",
    "tuple_anon",
    "multi_return",
    "record_named",
    "destructure",
    "seq_tuple",
];

/// Run `source` on the TREE-WALKER (the oracle), capturing partial output and
/// the error, if any. This entry point always uses the tree-walker — the
/// dispatching `interpret_for_ui_sync` now runs the VM for the sync path, and
/// the differential suites need the oracle by itself.
pub fn tw_outcome(source: &str) -> RunOutcome {
    let result = tw_lines(source);
    RunOutcome {
        output: result.lines.join("\n"),
        error: result.error,
    }
}

/// [`tw_outcome`] with the shown lines kept apart. The exact bytes a run writes to standard output are
/// every line followed by a newline, which is what a compiled binary writes; the joined `output` of a
/// [`RunOutcome`] cannot tell no output from one empty line, so a byte-exact comparison starts here.
pub fn tw_lines(source: &str) -> crate::interpreter::InterpreterResult {
    tw_lines_with_args(source, &[])
}

/// [`tw_lines`] with the program argument vector for the `args()` system native (full argv; index 0 is the
/// program name).
pub fn tw_lines_with_args(
    source: &str,
    program_args: &[String],
) -> crate::interpreter::InterpreterResult {
    // Deterministic oracle: no relay transport, so `Connect` runs as a single-node local no-op (the
    // networking analogue of the fixed clock). Only affects a `Connect` program; inert otherwise.
    crate::concurrency::net_inbox::set_net_offline(true);
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => {
            let force_async = crate::interpreter::needs_async(stmts, interner);
            crate::ui_bridge::run_treewalker(
                stmts,
                types,
                policies,
                interner,
                force_async,
                program_args,
            )
        }
        Err(advice) => crate::interpreter::InterpreterResult {
            lines: Vec::new(),
            error: Some(advice),
        },
    })
}

/// Whether `stmt` merely DEFINES something — a component, a type, a route, a theorem — rather
/// than doing anything when the program runs. The allowlist behind [`main_drives_a_page`]: an
/// unlisted statement kind at the boundary means the gate declines rather than guesses, so a
/// shape this function has not considered can only narrow the gate, never widen it onto ground it
/// has not been checked against.
///
/// Everything BEFORE the split compiles and runs exactly as the straight-through path always has
/// — an ordinary program prefix, whatever it does; only the TRAILING run has to be pure driving
/// statements, since that is the part `page_host::drive_main` carries out itself instead of
/// letting the compiler inline it into `main`. The split is the START of the longest such trailing
/// run, so a real program's own imports and top-level setup (which `page_host`'s driving
/// vocabulary was never meant to classify) never have to look like anything in particular.
fn main_drives_a_page(stmts: &[Stmt], interner: &Interner) -> Option<usize> {
    let is_drive = |s: &Stmt| {
        matches!(
            crate::page_host::classify(s, interner),
            crate::page_host::Classified::Drive(_)
        )
    };
    let mut split = stmts.len();
    while split > 0 && is_drive(&stmts[split - 1]) {
        split -= 1;
    }
    if split == stmts.len() {
        // No trailing driving statement at all — provably inert for this program.
        return None;
    }
    let mounts = stmts[split..].iter().any(|s| {
        matches!(
            crate::page_host::classify(s, interner),
            crate::page_host::Classified::Drive(crate::page_host::Drive::Mount { .. })
        )
    });
    mounts.then_some(split)
}

/// Run `source` on the bytecode VM, capturing partial output and the error,
/// if any. Same front-end as [`tw_outcome`].
///
/// A `## Main` that ENDS by driving a page (its trailing statements are nothing but driving
/// verbs mounting at least one component — `main_drives_a_page`) runs that trailing script
/// through `page_host::drive_main` instead of letting the compiler inline it into `main`: the same
/// guarded, settling driver a `## Test` candidate tier uses, so a trapping turn recovers and a
/// typed fetch completes here exactly as it does there. Every other program — anything with no
/// trailing driving statement at all — is untouched; the gate never fires for one.
///
/// A run is a fresh process, as on every tier: it starts with nothing stored
/// ([`crate::semantics::storage::reset`]).
pub fn vm_outcome(source: &str) -> RunOutcome {
    crate::semantics::storage::reset();
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => {
            if let Some(split) = main_drives_a_page(stmts, interner) {
                return match crate::page_host::drive_main(
                    &stmts[..split],
                    &stmts[split..],
                    interner,
                    types,
                    &policies,
                ) {
                    Ok(output) => RunOutcome {
                        output,
                        error: None,
                    },
                    Err(e) => RunOutcome {
                        output: String::new(),
                        error: Some(e),
                    },
                };
            }
            let (output, error) =
                crate::vm::run_to_outcome(stmts, interner, Some(types), Some(&policies));
            RunOutcome { output, error }
        }
        Err(advice) => RunOutcome {
            output: String::new(),
            error: Some(advice),
        },
    })
}

/// A runtime failure's message, prefixed with WHERE it happened when a `Stmt::Site` marker was in
/// force (`site`, from [`crate::interpreter::Interpreter::current_site`]/[`crate::vm::Vm::
/// failing_site`]) — Wave 3: "runtime failures carry a location". `None` — a failure in `##
/// Main`/`## Test`'s own top-level statements, which carry no marker — returns `message` unchanged.
///
/// `source` is re-parsed once for [`crate::ui_bridge::failure_sites_of`]'s table — a SECOND,
/// deterministic parse of the identical text through the identical front-end as whichever parse
/// produced `interner`/`site`. Their `Symbol`s align because [`Interner::intern`] assigns ids in
/// first-encounter order and both parses see the identical token stream (same source, same lexer,
/// same prelude splice) — `interner` here is the CALLER's own interner from that SAME source, not
/// the (discarded) one `failure_sites_of` used internally; it is what [`FailureSite::render`]
/// (`logicaffeine_language::ast::stmt::FailureSite`) needs to resolve the component/action symbols
/// this table's spans came with. The catalogue lock (`runtime_failure_location_lock`) is what holds
/// this alignment to account, on every failure kind, on both tiers.
fn locate_runtime_failure(
    source: &str,
    interner: &Interner,
    site: Option<u32>,
    message: &str,
) -> String {
    let Some(id) = site else {
        return message.to_string();
    };
    let sites = crate::ui_bridge::failure_sites_of(source);
    let Some(site) = sites.get(id as usize) else {
        return message.to_string();
    };
    let (line, col) = logicaffeine_language::diagnostic::line_col(source, site.span.start);
    format!("{line}:{col} — {}", site.render(interner, message))
}

/// [`tw_outcome`], with a failed run's error LOCATED (see `locate_runtime_failure`) — same front-
/// end, same interpreter, same error TEXT; this only decides what goes in FRONT of it. A concurrent
/// program (many `Interpreter` instances, one per task) has no single "the current site" to show, so
/// it falls back to [`tw_outcome`]'s own unlocated shape rather than guessing which task's site to
/// name.
pub fn tw_outcome_located(source: &str) -> RunOutcome {
    crate::concurrency::net_inbox::set_net_offline(true);
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => {
            if crate::concurrency::uses_scheduler(stmts) {
                let force_async = crate::interpreter::needs_async(stmts, interner);
                let result = crate::ui_bridge::run_treewalker(
                    stmts,
                    types,
                    policies,
                    interner,
                    force_async,
                    &[],
                );
                return RunOutcome {
                    output: result.lines.join("\n"),
                    error: result.error,
                };
            }
            let force_async = crate::interpreter::needs_async(stmts, interner);
            let mut interp = crate::interpreter::Interpreter::new(interner)
                .with_type_registry(types)
                .with_policies(policies)
                .with_vfs(std::sync::Arc::new(logicaffeine_system::fs::MemVfs::new()));
            let run_result = if force_async {
                futures::executor::block_on(interp.run(stmts))
            } else {
                interp.run_sync(stmts)
            };
            let site = interp.page_failure_site().or(interp.current_site);
            let unhandled = interp.take_page_failure();
            let (error, site) = match run_result {
                Err(e) => (Some(e), interp.current_site),
                Ok(()) => (unhandled, site),
            };
            let error = error.map(|msg| locate_runtime_failure(source, interner, site, &msg));
            RunOutcome {
                output: interp.output.join("\n"),
                error,
            }
        }
        Err(advice) => RunOutcome {
            output: String::new(),
            error: Some(advice),
        },
    })
}

/// [`vm_outcome`], with a failed run's error LOCATED (see `locate_runtime_failure`). Uses the
/// plain `Compiler::compile_with_oracle` → `Vm::run` path directly (not `page_host::drive_main`'s
/// page-driving split) — every failure-site fixture this lane's tests drive is a single-turn action
/// failure, which that plain path already covers exactly as `vm_failure_site_lock.rs` proves. A
/// run is a fresh process: it starts with nothing stored.
pub fn vm_outcome_located(source: &str) -> RunOutcome {
    crate::semantics::storage::reset();
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => {
            let (output, error, site) =
                crate::vm::run_to_outcome_located(stmts, interner, Some(types), Some(&policies));
            let error = error.map(|msg| locate_runtime_failure(source, interner, site, &msg));
            RunOutcome { output, error }
        }
        Err(advice) => RunOutcome {
            output: String::new(),
            error: Some(advice),
        },
    })
}

/// Which engine actually executed a strict run — the identity a caller can
/// trust, because the strict entries never substitute one engine for another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineId {
    /// The tree-walking interpreter (the reference oracle).
    TreeWalker,
    /// The register bytecode VM.
    Vm,
}

impl EngineId {
    /// The stable machine-readable label (`"tree-walker"` / `"vm"`).
    pub fn label(self) -> &'static str {
        match self {
            EngineId::TreeWalker => "tree-walker",
            EngineId::Vm => "vm",
        }
    }
}

/// The outcome of a STRICT engine run. `engine_ran` is the honest identity:
/// `Some(engine)` means that engine — and only that engine — executed the
/// program; `None` means NOTHING ran (a parse failure, or the typed refusal of
/// an engine that cannot run the program) and `error` says why. A strict run
/// never silently substitutes the tree-walker for the VM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineOutcome {
    /// The engine that executed the program, or `None` when nothing ran.
    pub engine_ran: Option<EngineId>,
    /// Every output line, newline-joined (both engines share this convention,
    /// so cross-engine outcomes compare byte-for-byte).
    pub output: String,
    /// The runtime error, the parse story, or the refusal reason.
    pub error: Option<String>,
}

impl EngineOutcome {
    /// True when nothing ran and `error` carries the reason — the typed
    /// refusal (or a parse failure, which refuses BOTH engines).
    pub fn refused(&self) -> bool {
        self.engine_ran.is_none()
    }
}

/// Engine-identity trace shared with the dispatching paths: with
/// `LOGOS_ENGINE_TRACE` set, say which engine a strict run used (stderr).
fn strict_trace(engine: &str) {
    if std::env::var_os("LOGOS_ENGINE_TRACE").is_some() {
        eprintln!("logos-engine: {engine}");
    }
}

/// Run `source` STRICTLY on the tree-walker. Identical pipeline to
/// [`tw_outcome`] (same front-end, same oracle engine — concurrency runs on
/// the tree-walker's deterministic scheduler), plus the honest engine
/// identity: `engine_ran` is `Some(TreeWalker)` whenever the program executed,
/// `None` for a parse failure (nothing ran on any engine).
pub fn run_strict_tw(source: &str) -> EngineOutcome {
    run_strict_tw_with_args(source, &[])
}

/// [`run_strict_tw`] with the program argument vector for the `args()` system
/// native (full argv; index 0 is the program name).
pub fn run_strict_tw_with_args(source: &str, program_args: &[String]) -> EngineOutcome {
    // Deterministic oracle: offline single node, matching `tw_outcome`.
    crate::concurrency::net_inbox::set_net_offline(true);
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => {
            let force_async = crate::interpreter::needs_async(stmts, interner);
            strict_trace("strict treewalker");
            let result = crate::ui_bridge::run_treewalker(
                stmts,
                types,
                policies,
                interner,
                force_async,
                program_args,
            );
            EngineOutcome {
                engine_ran: Some(EngineId::TreeWalker),
                output: result.lines.join("\n"),
                error: result.error,
            }
        }
        Err(advice) => EngineOutcome {
            engine_ran: None,
            output: String::new(),
            error: Some(advice),
        },
    })
}

/// Run `source` STRICTLY on the bytecode VM. Identical pipeline to
/// [`vm_outcome`] (same front-end, same oracle-range compile), with the
/// strict contract: a program the VM cannot run — one that needs the async
/// runtime, one that needs the scheduler driver, or one its compiler rejects —
/// is a TYPED REFUSAL (`engine_ran: None`, `error` says why). The tree-walker
/// is NEVER silently substituted.
pub fn run_strict_vm(source: &str) -> EngineOutcome {
    run_strict_vm_with_args(source, &[])
}

/// [`run_strict_vm`] with the program argument vector for the `args()` system
/// native. Honors the thread's armed fuel (the MCP `step_budget`) exactly as
/// the live sync path does.
pub fn run_strict_vm_with_args(source: &str, program_args: &[String]) -> EngineOutcome {
    let refusal = |why: String| {
        strict_trace("strict vm (refused)");
        EngineOutcome {
            engine_ran: None,
            output: String::new(),
            error: Some(why),
        }
    };
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => {
            // The strict twins of the dispatcher's tree-walker fallbacks: where
            // the live path would silently substitute the tree-walker, strict
            // mode refuses BEFORE anything runs.
            if crate::interpreter::needs_async(stmts, interner) {
                return refusal(
                    "strict vm: refused — this program needs the async runtime (file I/O, \
                     sleep, serve, or networking); the bytecode VM is sync-only and the \
                     tree-walker is never silently substituted"
                        .to_string(),
                );
            }
            if crate::concurrency::uses_scheduler(stmts) {
                return refusal(
                    "strict vm: refused — this program uses tasks/channels, which need the \
                     scheduler driver; the plain bytecode VM cannot run it and the \
                     tree-walker is never silently substituted"
                        .to_string(),
                );
            }
            let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
            let program = match crate::vm::Compiler::compile_with_oracle(
                stmts,
                interner,
                Some(types),
                Some(oracle),
            ) {
                Ok(p) => p,
                Err(e) => {
                    return refusal(format!(
                        "strict vm: refused — the VM compiler cannot compile this program: {e}"
                    ))
                }
            };
            strict_trace("strict vm");
            let mut vm = crate::vm::Vm::new(&program)
                .with_policy_ctx(&policies, interner)
                .with_program_args(program_args.to_vec());
            // Same tier rule as the live path: JIT-tiered native code executes
            // outside the STEPPED accounting, so a budgeted run stays on the
            // VM dispatch loop.
            if crate::interpreter::active_fuel().is_none() {
                if let Some(tier) = crate::vm::installed_native_tier() {
                    vm = vm.with_native_tier(tier);
                }
            }
            let error = crate::vm::drive_vm(&mut vm);
            EngineOutcome {
                engine_ran: Some(EngineId::Vm),
                output: vm.into_lines().join("\n"),
                error,
            }
        }
        Err(advice) => EngineOutcome {
            engine_ran: None,
            output: String::new(),
            error: Some(advice),
        },
    })
}

/// Run `source` on the bytecode VM in the OBSERVED profile (Wave 13): the VM
/// emits the same observation events (`Note`/`Warn`/`Record`/`During`) the
/// tree-walker does, into the `observe` channel — program output is byte-
/// identical to [`vm_outcome`]. Pair with `observe::install()`/`drain()` to
/// capture the emitted event stream.
pub fn vm_outcome_observed(source: &str) -> RunOutcome {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => {
            let (output, error) =
                crate::vm::run_to_outcome_observed(stmts, interner, Some(types), Some(&policies));
            RunOutcome { output, error }
        }
        Err(advice) => RunOutcome {
            output: String::new(),
            error: Some(advice),
        },
    })
}

/// The verdict of a cross-tier differential run (Wave 13, footgun F18). Program
/// output must be byte-identical across every execution tier; a divergence is
/// ALWAYS a compiler bug — the framework's liability, never the user's mistake —
/// and carries a filed-ready reproduction bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TierVerdict {
    /// Every tier agreed. The single shared outcome (the tree-walker oracle's).
    Agree {
        output: String,
        error: Option<String>,
    },
    /// Two tiers disagreed. `compiler_bug` is invariably `true` — that a tier
    /// divergence is the compiler's fault (never the user's) is the whole point
    /// of the classification. `bundle` is a ready-to-file reproduction.
    Divergence { compiler_bug: bool, bundle: String },
}

/// Run `source` on every always-available execution tier — the tree-walker
/// oracle and the register VM — and classify the result (Wave 13; the engine
/// behind `largo test --tiers all` and the F18 "works-on-my-tier" guard).
///
/// Output is compared modulo the one benign, documented tier convention (the VM
/// terminates output with a newline the tree-walker omits — `trim_end`); error
/// text is compared exactly. For sequential programs, as the differential
/// harness is; a concurrency program (which the plain VM entry declines) is out
/// of scope and simply reports `Agree` from the oracle.
pub fn tier_differential(source: &str) -> TierVerdict {
    let tw = tw_outcome(source);
    let vm = vm_outcome(source);

    // The VM declines scheduler ops ("requires the scheduler driver"); that is a
    // capability boundary, not a divergence — defer to the oracle (concurrency
    // has its own seeded differential).
    let vm_declined = vm
        .error
        .as_deref()
        .map(|e| e.contains("scheduler driver"))
        .unwrap_or(false);
    if vm_declined {
        return TierVerdict::Agree {
            output: tw.output,
            error: tw.error,
        };
    }

    let outputs_agree = tw.output.trim_end() == vm.output.trim_end();
    let errors_agree = tw.error == vm.error;
    if outputs_agree && errors_agree {
        TierVerdict::Agree {
            output: tw.output,
            error: tw.error,
        }
    } else {
        let bundle = format!(
            "TIER DIVERGENCE — a compiler bug (the framework's liability, never your program's).\n\
             This is footgun F18: a 5-tier language must guarantee every tier agrees.\n\
             --- source ---\n{source}\n\
             --- tree-walker (oracle) ---\noutput={:?}\nerror={:?}\n\
             --- register VM ---\noutput={:?}\nerror={:?}\n",
            tw.output, tw.error, vm.output, vm.error
        );
        TierVerdict::Divergence {
            compiler_bug: true,
            bundle,
        }
    }
}

/// The VM outcome for a CONCURRENT program — drives the bytecode VM on the cooperative scheduler
/// (`VmTask` on `run_with_seed`, seed 0) exactly as the browser/playground does, so a `Pipe`/task/
/// `Select` program runs on the VM byte-identical to the tree-walker's async path. The plain
/// [`vm_outcome`] errors on any concurrency op (`requires the scheduler driver`); this is the oracle
/// the AOT concurrency lock compares against so tw == VM == AOT is verifiable for the concurrent set.
#[cfg(not(target_arch = "wasm32"))]
pub fn vm_outcome_concurrent(source: &str) -> RunOutcome {
    let r = crate::ui_bridge::run_vm_concurrent(source);
    RunOutcome {
        output: r.lines.join("\n"),
        error: r.error,
    }
}

/// The VM outcome for a peer-NETWORKING program — drives the resumable bytecode VM and services each
/// `VmBlock::Net*` through the shared LOCAL `NetInbox` (the same inbox the tree-walker's offline
/// mode uses), so `Listen`/`Send`/`Sync`/`PeerAgent` run on the VM byte-identical to the tree-walker.
/// The AOT networking lock compares against this so tw == VM == AOT holds for the local-mode net set.
#[cfg(not(target_arch = "wasm32"))]
pub fn vm_outcome_net(source: &str) -> RunOutcome {
    let r = vm_lines_net(source);
    RunOutcome {
        output: r.lines.join("\n"),
        error: r.error,
    }
}

/// [`vm_outcome_net`] with the shown lines kept apart (see [`tw_lines`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn vm_lines_net(source: &str) -> crate::interpreter::InterpreterResult {
    // Deterministic oracle: offline single node, so `Connect` is a local no-op (matches `tw_outcome`).
    crate::concurrency::net_inbox::set_net_offline(true);
    futures::executor::block_on(crate::ui_bridge::run_vm_net_async(source))
}

/// Run an ALREADY-COMPILED (possibly HAND-AUTHORED) program through the reversible cooperative scheduler
/// (`RevCoop`) and return its main output. The scheduler-only ops that no LOGOS source emits — `ChanClose`,
/// `TaskAwait`, `SpawnHandle` — BLOCK on the plain VM (`requires the scheduler driver`), so their fabric
/// tile is proven by hand-building a raw `CompiledProgram` and differencing the fabric tape against THIS
/// oracle (the same seam `codegen_sva/rtl.rs`'s soft-core differential uses for those ops).
#[cfg(not(target_arch = "wasm32"))]
pub fn program_outcome_scheduled(program: &crate::vm::CompiledProgram) -> RunOutcome {
    match crate::vm::RevCoop::record(crate::vm::Vm::new(program)) {
        Ok(coop) => RunOutcome {
            output: coop.main_output(),
            error: None,
        },
        Err(e) => RunOutcome {
            output: String::new(),
            error: Some(e),
        },
    }
}

/// Run an ALREADY-COMPILED (possibly HAND-AUTHORED) program through the FULL cooperative scheduler (the same
/// `VmTask`/`run_with_seed` driver `run_vm_concurrent` uses, seed 0), returning its output. Unlike the
/// reversible `program_outcome_scheduled`, this services ops the reverse machinery does not yet cover
/// (`TaskAbort`) — so it is the oracle for the fabric's `TaskAbort` tile.
#[cfg(not(target_arch = "wasm32"))]
pub fn program_outcome_concurrent(program: &crate::vm::CompiledProgram) -> RunOutcome {
    let r = program_lines_concurrent(program);
    RunOutcome {
        output: r.lines.join("\n"),
        error: r.error,
    }
}

/// [`program_outcome_concurrent`] with the shown lines kept apart (see [`tw_lines`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn program_lines_concurrent(
    program: &crate::vm::CompiledProgram,
) -> crate::interpreter::InterpreterResult {
    use crate::concurrency::vm_driver::VmTask;
    use logicaffeine_runtime::{
        run_with_seed, RunOutcome as RtOutcome, SchedSeed, SchedulerConfig,
    };
    use std::cell::RefCell;
    use std::rc::Rc;

    let output: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let err_sink: crate::concurrency::driver::ErrSink = Rc::new(RefCell::new(None));
    let main_task = VmTask::new(
        crate::vm::Vm::new(program),
        output.clone(),
        Some(err_sink.clone()),
    );
    let (outcome, _trace) = run_with_seed(SchedulerConfig::default(), SchedSeed(0), move |sched| {
        sched.spawn_main(Box::new(main_task));
    });
    let mut error = err_sink.borrow().clone();
    if error.is_none() {
        if let RtOutcome::Deadlock = outcome {
            error = Some("deadlock: every task is blocked waiting".to_string());
        }
    }
    let lines = output.borrow().clone();
    crate::interpreter::InterpreterResult { lines, error }
}

/// Run an ALREADY-COMPILED `CompiledProgram` straight-line on the VM and return its outcome — the plain-
/// sequential counterpart to [`program_outcome_scheduled`]/[`program_outcome_concurrent`]. This is the
/// missing primitive that lets the source-of-truth invariants be expressed against the SAME bytecode IR every
/// backend lowers: `VM(opt P)` is `program_outcome(&optimized_program)`, so a backend's output can be
/// differenced against the exact IR it consumed. Partial output on error is preserved (matching the VM).
#[cfg(not(target_arch = "wasm32"))]
pub fn program_outcome(program: &crate::vm::CompiledProgram) -> RunOutcome {
    let mut vm = crate::vm::Vm::new(program);
    match vm.run() {
        Ok(()) => RunOutcome {
            output: vm.output(),
            error: None,
        },
        Err(e) => RunOutcome {
            output: vm.output(),
            error: Some(e),
        },
    }
}

/// [`tw_outcome`] with the program argument vector for the `args()` system
/// native (full argv; index 0 is the program name) — the seam the benchmark
/// corpus differential uses to run `main.lg` programs that read their size
/// from argv.
pub fn tw_outcome_with_args(source: &str, program_args: &[String]) -> RunOutcome {
    let result = tw_lines_with_args(source, program_args);
    RunOutcome {
        output: result.lines.join("\n"),
        error: result.error,
    }
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
        Err(advice) => RunOutcome {
            output: String::new(),
            error: Some(advice),
        },
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
            // `with_optimized_program_tiered` above owns the shared optimizer
            // stage. Bytecode selection and any native-tier work happen only
            // after that program reaches the VM, so keep their `mark_fired`
            // evidence in backend lowering. Without this boundary a JIT trace
            // falsely reports VM choices such as `unbox` and `fastdiv` as
            // extra shared optimizer passes.
            let (output, error) = crate::optimize::with_backend_lowering(|| {
                crate::vm::run_to_outcome_with_args(
                    stmts,
                    interner,
                    Some(types),
                    Some(&policies),
                    program_args,
                    native_tier,
                )
            });
            RunOutcome { output, error }
        }
        Err(advice) => RunOutcome {
            output: String::new(),
            error: Some(advice),
        },
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
        Err(advice) => RunOutcome {
            output: String::new(),
            error: Some(advice),
        },
    })
}

/// Refuse a program that touches a BROWSER surface on a tier that has none.
///
/// The DOM, the signal store and WebRTC arrive as ordinary builtin CALLS rather than opcodes of
/// their own, so a backend that never inspects builtins simply emits them and returns an
/// artifact. AOT-Rust produced 380 bytes for a page, AOT-C produced 121KB, and neither has a
/// document — `largo build --target c` on a `## Page` looked like a successful build and could
/// only ever render nothing, with the mistake surfacing far from the `--target` flag that caused
/// it. Fail closed instead, and NAME the DOM so the reader knows the tier is wrong for the
/// program rather than the program being wrong.
fn refuse_browser_surface(source: &str, tier: &str) -> Result<(), ParseError> {
    let Ok(program) = compile_to_program(source) else {
        return Ok(());
    };
    // REACHABLE, not merely present: the prelude loads standard-library functions this program
    // never calls, and `web.lg` holds the browser fetch wrappers beside the pure-text helpers.
    if crate::aot_unsupported::first_reachable(&program)
        == Some(crate::aot_unsupported::Offload::Dom)
    {
        return Err(ParseError {
            kind: crate::error::ParseErrorKind::Custom(format!(
                "{tier} cannot build a page: this program touches the DOM (a browser surface), \
                 and {tier} has no document to build. A `## Page` or a `## Component` runs on the \
                 tree-walker and in the browser through WebAssembly — build it for one of those, \
                 or move the part that is not about the page into a program of its own."
            )),
            span: crate::token::Span::default(),
        });
    }
    Ok(())
}

/// `stmts` with the root bindings its functions reach lifted into one lent record
/// ([`crate::program_bindings`]) — for a tier that compiles `## Main` into a function of its own —
/// and the registry that record is registered in. Unchanged when no function reaches one.
fn lift_program_bindings<'a>(
    stmts: Vec<Stmt<'a>>,
    registry: crate::analysis::TypeRegistry,
    interner: &mut Interner,
    arenas: crate::ast_map::Arenas<'a>,
) -> Result<(Vec<Stmt<'a>>, crate::analysis::TypeRegistry), ParseError> {
    match crate::program_bindings::lift_program_bindings(&stmts, &registry, interner, arenas) {
        Ok(Some(lifted)) => Ok((lifted.stmts, lifted.registry)),
        Ok(None) => Ok((stmts, registry)),
        Err(why) => Err(ParseError {
            kind: crate::error::ParseErrorKind::Custom(why),
            span: crate::token::Span::default(),
        }),
    }
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
/// - The program reaches a browser surface (the DOM, the signal store, WebRTC), which the AOT-Rust tier
///   does not have
///
/// # Example
///
/// ```
/// # use logicaffeine_compile::compile::compile_to_rust;
/// # use logicaffeine_compile::ParseError;
/// # fn main() -> Result<(), ParseError> {
/// let source = "## Main\nLet x be 5.\nShow x.";
/// let rust_code = compile_to_rust(source)?;
/// assert!(rust_code.contains("fn main()"));
/// assert!(!rust_code.contains("compile_error!"));
/// # Ok(())
/// # }
/// ```
pub fn compile_to_rust(source: &str) -> Result<String, ParseError> {
    refuse_browser_surface(source, "the AOT-Rust backend")?;
    compile_program_full(source).map(|o| o.rust_code)
}

/// Compile for **deterministic replay**: identical to [`compile_to_rust`] for a program
/// that never touches the scheduler; a program that does runs its tasks, pipes, timers and selects on
/// `logicaffeine_system::replay`, the interpreter's scheduler ported, so a binary run under
/// `LOGOS_SEED=…` prints what the interpreter prints at that seed. Used by the seeded differential
/// harness.
pub fn compile_to_rust_deterministic(source: &str) -> Result<String, ParseError> {
    crate::codegen::with_deterministic_replay(|| compile_to_rust(source))
}

/// Compile to Rust in the **OBSERVED profile** (Wave 13): the emitted program
/// EMITS observation events — `logicaffeine_system::obs::record`/`span_enter`/
/// `span_exit` calls for `Note`/`Warn`/`Record`/`During` — instead of eliding
/// them. The default [`compile_to_rust`] stays the elided (release) profile,
/// byte-identical to the observation-free program (§0.7).
pub fn compile_to_rust_observed(source: &str) -> Result<String, ParseError> {
    crate::codegen::with_observed_codegen(|| compile_to_rust(source))
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
            compile_to_rust(&decorated)
                .map(|r| r != base)
                .unwrap_or(false)
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

/// Compile the Rust AOT backend with stage-separated optimization evidence.
/// The ordinary compile path is unchanged; tracing only observes existing
/// `mark_fired` sites and classifies them as shared optimizer vs Rust lowering.
pub fn compile_program_backend_traced(
    source: &str,
) -> Result<(CompileOutput, crate::optimization::BackendOptimizationTrace), ParseError> {
    crate::optimize::begin_backend_trace(crate::optimization::OptimizationBackend::RustAot);
    let result = compile_program_full(source);
    let trace = crate::optimize::end_backend_trace().expect("backend trace was just started");
    result.map(|o| (o, trace))
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
    let fired = crate::optimize::end_fired_trace()
        .unwrap_or_default()
        .keywords();
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
        let off: BTreeSet<&'static str> = optimizations_fired(&decorate_source(source, &[dep]))
            .into_iter()
            .collect();
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
pub fn optimization_trace_run(source: &str) -> crate::optimization::BackendOptimizationTrace {
    crate::optimize::begin_backend_trace(crate::optimization::OptimizationBackend::TreeWalker);
    crate::ui_bridge::with_optimized_program(source, |_parsed, _interner| {});
    crate::optimize::end_backend_trace().expect("backend trace was just started")
}

pub fn optimizations_fired_run(source: &str) -> Vec<&'static str> {
    optimization_trace_run(source).all_fired().keywords()
}

/// The optimizations that fired while compiling `source` to VM bytecode — the
/// third path, after AOT codegen and the run-path optimizer. Surfaces the
/// VM-compile-time opts (constant-divisor magic division, VM `i32` narrowing)
/// the other two traces cannot see. Compiles only (no execution), so it is fast
/// and needs no program arguments.
pub fn optimization_trace_vm(source: &str) -> crate::optimization::BackendOptimizationTrace {
    crate::optimize::begin_backend_trace(crate::optimization::OptimizationBackend::Vm);
    crate::ui_bridge::with_optimized_program(source, |parsed, interner| {
        if let Ok((stmts, types, _policies)) = parsed {
            let _ = crate::optimize::with_backend_lowering(|| {
                // The oracle is consumed by VM bytecode selection after the shared
                // run-path optimizer has finished.  Keep facts it discovers in the
                // backend-lowering stage; otherwise an analysis-only `mark_fired`
                // looks like a VM-only optimizer transform and trips the parity
                // ratchet against the tree-walker/run path.
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(
                    stmts,
                    interner,
                    Some(types),
                    Some(oracle),
                );
                // This entry point traces the VM as the final backend, so
                // selecting an unchecked opcode is VM lowering evidence.  Do
                // the census here instead of in `Compiler::emit`: bytecode-C
                // uses the same compiler as an intermediate producer and must
                // earn its own `Unchecked` bit only when its C walker actually
                // consumes the opcode.  That separation is what lets the
                // cross-backend ratchet detect a silently reintroduced check.
                if let Ok(program) = &program {
                    if program.code.iter().any(|op| {
                        matches!(
                            op,
                            crate::vm::Op::IndexUnchecked { .. }
                                | crate::vm::Op::SetIndexUnchecked { .. }
                        )
                    }) {
                        crate::optimize::mark_fired(crate::optimization::Opt::Unchecked);
                    }
                }
                program
            });
        }
    });
    crate::optimize::end_backend_trace().expect("backend trace was just started")
}

pub fn optimizations_fired_vm(source: &str) -> Vec<&'static str> {
    optimization_trace_vm(source).all_fired().keywords()
}

/// Compile LOGOS source to C code.
///
/// Produces a self-contained C file with embedded runtime that can be
/// compiled with `gcc -O2 -o program output.c`.
pub fn compile_to_c(source: &str) -> Result<String, ParseError> {
    // A program the VM refuses before it runs is one whose every run fails, before any output, in the VM's
    // own words: the C is that program.
    if let Some(refusal) = vm_refusal(source) {
        return Ok(crate::codegen_c::refused_program(&refusal));
    }
    compile_to_c_tiered(source, crate::codegen_c::TaskTier::Coroutines)
}

/// Direct AST→C compilation with stage-separated optimization evidence.
/// The refusal probe runs before tracing so VM-only probe work cannot be
/// misreported as a C optimization.
pub fn compile_to_c_traced(
    source: &str,
) -> Result<(String, crate::optimization::BackendOptimizationTrace), ParseError> {
    let refusal = vm_refusal(source);
    crate::optimize::begin_backend_trace(crate::optimization::OptimizationBackend::CAst);
    let result = match refusal {
        Some(refusal) => Ok(crate::codegen_c::refused_program(&refusal)),
        None => compile_to_c_tiered(source, crate::codegen_c::TaskTier::Coroutines),
    };
    let trace = crate::optimize::end_backend_trace().expect("backend trace was just started");
    result.map(|c| (c, trace))
}

/// Why the VM refuses `source` before running any of it — its front end rejects the text, or its compiler the
/// program — in the words [`vm_outcome`] reports; `None` for a program it runs.
fn vm_refusal(source: &str) -> Option<String> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| match parsed {
        Err(advice) => Some(advice),
        Ok((stmts, _, _)) if main_drives_a_page(stmts, interner).is_some() => None,
        Ok((stmts, types, _)) => {
            let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
            crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                .err()
        }
    })
}

/// [`compile_to_c`] with the program's tasks as step functions over heap frames — the stepped tier, the one
/// a GPU thread can run and the host walker runs — instead of coroutines on stacks of their own.
pub fn compile_to_c_stepped(source: &str) -> Result<String, ParseError> {
    compile_to_c_tiered(source, crate::codegen_c::TaskTier::Stepped)
}

/// [`compile_to_c_stepped`] under an explicit [`SchedulerKnobs`](crate::codegen_c::concurrency::SchedulerKnobs):
/// the scheduler's policy, workers, seed and default pipe capacity, as `#define`s its scheduler reads.
#[cfg(not(target_arch = "wasm32"))]
pub fn compile_to_c_stepped_with_knobs(
    source: &str,
    knobs: &crate::codegen_c::concurrency::SchedulerKnobs,
) -> Result<String, ParseError> {
    Ok(format!(
        "{}{}",
        knobs.emit_defines(),
        compile_to_c_stepped(source)?
    ))
}

fn compile_to_c_tiered(
    source: &str,
    tier: crate::codegen_c::TaskTier,
) -> Result<String, ParseError> {
    refuse_browser_surface(source, "the AOT-C backend")?;
    // Auto-prepend the stdlib modules the program references, exactly as the Rust path does
    // (see `compile_program_full_with_proven`). Without this the C backend lexes the bare user
    // source, so a call to an auto-imported stdlib function (`gcd`, `binomial`, …) emits the call
    // but never its body — the generated C fails to link ("undefined reference"). No-op when the
    // program uses no stdlib vocabulary, so hand-written C corpora stay byte-identical.
    let prelude_src = crate::loader::apply_prelude(source);
    // Tree-shake unreachable functions before C codegen: the flat C namespace can't emit an
    // unused stdlib function that collides with a libc name or hits a C-codegen gap it is never
    // reached through. (C-only; the Rust backend tolerates dead functions, so it keeps the full
    // splice.)
    let pruned = crate::loader::keep_reachable_functions(source, prelude_src.as_ref());
    let source = pruned.as_str();
    let library = crate::loader::LibraryProvenance::of(source);

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    let (type_registry, policy_registry) = {
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
    let _compile_facts = crate::optimize::facts::scope();
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

    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
    );
    let stmts = parser.parse_program()?;
    // Honor file-level `## No`/`## Require` decorators on the C backend (`largo emit c`) via the
    // shared gate: merge `## No` into the active config and fail an unfired `## Require`. Captured
    // here (the last use of `parser`) so its `&mut interner` borrow ends before reuse.
    let (opt_config, gate) = crate::optimize::install_decorators(
        crate::optimization::OptimizationConfig::from_env(),
        &parser.program_opt_flags(),
        parser.required_optimizations(),
    );
    let codegen_registry = parser.into_type_registry();
    // Unreachable functions were tree-shaken above, so a native-kernel declaration that survives is
    // one the program calls — and C cannot link the Rust kernels.
    if let Some(kernel) = stmts.iter().find_map(|s| match s {
        Stmt::FunctionDef {
            name,
            is_native: true,
            ..
        } => crate::semantics::native_kernels::NativeKernel::from_name(interner.resolve(*name)),
        _ => None,
    }) {
        return Err(ParseError {
            kind: crate::error::ParseErrorKind::Custom(kernel.refusal("C")),
            span: crate::token::Span::default(),
        });
    }
    if let Some(kind) = codegen_registry
        .iter_types()
        .find_map(|(_, def)| match def {
            crate::analysis::TypeDef::Struct { fields, .. } => fields.iter().find_map(|f| {
                crate::semantics::crdt::CrdtKind::of_type_name(field_type_base_name(
                    &f.ty, &interner,
                ))
                .filter(|k| !k.lowered_by(Backend::CFromAst))
            }),
            _ => None,
        })
    {
        return Err(ParseError {
            kind: crate::error::ParseErrorKind::Custom(kind.refusal("C")),
            span: crate::token::Span::default(),
        });
    }
    // A pipe parameter declared by its element (`ch: Real`, as the interpreter accepts) becomes
    // `Pipe of Real`, so every site that types a parameter by its declaration sees a pipe.
    let stmts = crate::analysis::pipes::declare_pipe_params(
        &stmts,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
        &mut interner,
    );
    // Preserve source-level purity and recursion shape while inserting an explicit cache. The shared
    // optimizer and versioner must still see this source-shaped program, just as the Rust backend does;
    // C-only monomorphization runs after their final proof-bearing rewrite.
    let stmts = crate::optimize::memoize_for_explicit_lowering(
        stmts,
        &imperative_expr_arena,
        &stmt_arena,
        &mut interner,
        &opt_config,
    );
    // Type-directed division: rewrite `Divide → ExactDivide` in every Rational context (the integer
    // default stays floor), so `Let x: Rational be 7 / 2` compiles exact (`7/2`) instead of flooring to
    // `3` — matching the tree-walker/VM and the AOT-Rust path. The optimizer treats `ExactDivide` as
    // opaque (never floor-folds it, and never leaks an integer point-range from it), so this is the
    // single pass that keeps the exact numeric tower away from the integer-only folds. Purely additive.
    let resolved = crate::resolve_program::resolve_program(
        &stmts,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
        &mut interner,
        opt_config.is_on(crate::optimization::Opt::Comptime),
        library.functions(),
    );
    let stmts: Vec<Stmt> = match resolved {
        Some(rw) => rw.to_vec(),
        None => stmts,
    };
    let stmts = crate::optimize::optimize_program(
        stmts,
        &imperative_expr_arena,
        &stmt_arena,
        &mut interner,
        &opt_config,
    );
    // C fixes no order among an operator's operands; LOGOS evaluates them left to right. A lent element or field
    // becomes a binding the call points at, stored back after it.
    let effects = crate::codegen_c::call_effects(&stmts);
    let stmts = crate::optimize::sequence::sequence_operands(
        stmts,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
        &mut interner,
        effects,
    );
    // Consume the same guarded-region versioning pass Rust runs immediately before emission. It is
    // the last shared AST rewrite: the C-only monomorphizer below specializes this tree while
    // preserving the proof facts on the equivalent expressions it creates.
    let type_env = crate::analysis::check_program_located(&stmts, &interner, &codegen_registry)
        .map_err(|e| ParseError {
            kind: e.to_parse_error_kind(&interner, &stmts),
            span: crate::token::Span::default(),
        })?;
    let stmts = crate::optimize::version::version_stmts(
        stmts,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
        &type_env,
        &interner,
        &opt_config,
    );
    // C has no generics and no built-in `Option`: the proof-bearing program becomes the concrete one
    // it stands for — one function per (generic function, type arguments) it calls, one registered
    // type per applied generic, and `some`/`none` as instances of the core `Option` enum.
    let mono = crate::monomorphize::monomorphize(
        &stmts,
        &codegen_registry,
        &mut interner,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
        crate::monomorphize::Options { lower_option: true },
    )
    .map_err(|m| ParseError {
        kind: crate::error::ParseErrorKind::Custom(format!(
            "the C backend cannot compile this program: {m}"
        )),
        span: crate::token::Span::default(),
    })?;
    let (stmts, codegen_registry, instances) = (mono.stmts, mono.registry, mono.instances);
    if std::env::var_os("LOGOS_DEBUG_C_TILE").is_some() {
        use logicaffeine_language::ast_depth::{for_each_node, Visited};
        let mut tile_facts = 0usize;
        for_each_node(&stmts, |node| {
            if let Visited::Expr(expr) = node {
                tile_facts += usize::from(crate::optimize::facts::holds_expr(
                    crate::optimize::facts::Fact::TileLegal,
                    expr,
                ));
            }
        });
        eprintln!("C TileLegal expressions before emission: {tile_facts}");
    }
    let c_code = crate::optimize::with_backend_lowering(|| {
        crate::codegen_c::codegen_monomorphic_program_c(
            &stmts,
            &codegen_registry,
            &interner,
            &policy_registry,
            &instances,
            tier,
        )
    });
    gate.verify().map_err(|m| ParseError {
        kind: crate::error::ParseErrorKind::Custom(m),
        span: crate::token::Span::default(),
    })?;
    Ok(c_code)
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
/// Compile `source` to LOGOS Silicon ISA assembly (Tier 5, `--emit isa`) — the register-VM opcode set
/// AS the native chip ISA, every instruction annotated with its reversibility contract (see
/// [`crate::vm::isa`]). Uses the SAME Oracle-optimized bytecode the other tiers run, so the emitted
/// ISA is the exact program the interpreter/VM/JIT/AOT execute, now as silicon.
pub fn compile_to_isa(source: &str) -> Result<String, ParseError> {
    fn custom(msg: String) -> ParseError {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(msg),
            span: crate::token::Span::default(),
        }
    }
    refuse_exact(source, "Silicon ISA", "the LOGOS chip")?;
    crate::ui_bridge::with_parsed_program_checked(source, |parsed, interner| {
        let (stmts, types, _policies) = parsed?;
        let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
        let program = crate::vm::Compiler::compile_with_oracle_located(
            stmts,
            interner,
            Some(types),
            Some(oracle),
        )?;
        Ok(crate::vm::isa::emit_isa(&program))
    })
}

/// The sentence the `tier`, which holds no value of the exact extension, refuses `source` with when the extension
/// reaches it: the extension's values live in the value runtime a compiled program links beside it, which `holder`
/// cannot call. `None` for a program the extension never reaches, and for one that does not compile here: the
/// tier's own compile reports that, located, and checks the program's `## Require` against what fires in it.
pub(crate) fn exact_refusal(source: &str, tier: &str, holder: &str) -> Option<String> {
    let reaches = crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let Ok((stmts, types, _policies)) = parsed else {
            return false;
        };
        crate::vm::Compiler::compile_with_oracle_located(stmts, interner, Some(types), None)
            .is_ok_and(|program| crate::vm::tower::reaches(&program))
    });
    reaches.then(|| {
        format!("the {tier} does not run the exact extension: its values live in a runtime {holder} cannot call")
    })
}

/// [`exact_refusal`] as the error a compile entry stops with.
pub(crate) fn refuse_exact(source: &str, tier: &str, holder: &str) -> Result<(), ParseError> {
    match exact_refusal(source, tier, holder) {
        None => Ok(()),
        Some(sentence) => Err(ParseError {
            kind: crate::error::ParseErrorKind::Custom(sentence),
            span: crate::token::Span::default(),
        }),
    }
}

/// ★ THE CANONICAL `source → CompiledProgram` PATH ★ — the single bytecode-IR entry every backend and every
/// invariant lock reuses (no drift). This is the RAW-AST lowering (Oracle-analyzed, but WITHOUT the AOT
/// `optimize_program` lattice) — identical to what `vm_outcome`/`compile_to_riscv`/`compile_to_wasm` compile
/// today, so `program_outcome(compile_to_program(src)) == vm_outcome(src)` by construction. The optimized
/// variant (`optimize_program` applied → the finalized source-of-truth IR every backend will lower) is added
/// with the optimizer-soundness lock; see the refactor plan.
// ★ NOT HOST-ONLY, AND THE GATE HERE BROKE THE WEB BUILD. This was `#[cfg(not(target_arch = "wasm32"))]`,
// but nothing in the body needs a host: `ui_bridge` → `optimize` → `vm::Compiler` is pure computation, and
// all three modules build for wasm32 (the `vm` module IS the browser's engine). Meanwhile
// `refuse_browser_surface` — which is NOT gated, and must not be, because `compile_to_rust` and
// `compile_to_c` are what the web app's roadmap page and benchmarks call — invokes this function. So on
// wasm32 the crate referenced a function that did not exist and `apps/logicaffeine_web` could not build.
//
// Gating the CALLERS instead would compile, and would be wrong: it deletes `compile_to_rust` from the web
// build (the site uses it) and silently drops the DOM refusal in the browser, so the guard would hold on
// the host and vanish exactly where a `## Page` program is most likely to be compiled.
pub fn compile_to_program(source: &str) -> Result<crate::vm::CompiledProgram, ParseError> {
    fn custom(msg: String) -> ParseError {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(msg),
            span: crate::token::Span::default(),
        }
    }
    crate::ui_bridge::with_parsed_program_checked(source, |parsed, interner| {
        let (stmts, types, _policies) = parsed?;
        let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
        Ok(crate::vm::Compiler::compile_with_oracle_located(
            stmts,
            interner,
            Some(types),
            Some(oracle),
        )?)
    })
}

/// ★ Phase 1 — compile to C by lowering the bytecode `CompiledProgram` directly (the `codegen_c::from_program`
/// Op-walker), the bytecode-IR-as-source-of-truth path that REPLACES the AST-based `compile_to_c`. It lowers
/// the OPTIMIZED IR (`compile_to_program_optimized`) — the SAME `optimize_program` the AST `compile_to_c` runs,
/// honoring the LOGOS source's `## No`/`## Require` opt decorators — so the whole AOT optimizer lattice
/// (constant-fold, SCCP, egraph, LICM, deforest, supercompile, closed-form, bound_version) reaches the C. One
/// pristine, optimal pipeline: source → (opt flags honored) optimized bytecode → Op-walker → C. Grown op-by-op
/// behind `c_bytecode_lock`; returns a typed `Err` on any op not yet lowered (callers fall back to the AST
/// emitter until the Op-walker covers the corpus).
#[cfg(not(target_arch = "wasm32"))]
pub fn compile_to_c_via_bytecode(source: &str) -> Result<String, ParseError> {
    compile_to_c_via_bytecode_with_knobs(
        source,
        &crate::codegen_c::concurrency::SchedulerKnobs::default(),
    )
}

/// Optimized-bytecode→C compilation with stage-separated optimization
/// evidence. Fallbacks to the syntax-tree C tiers are recorded explicitly.
#[cfg(not(target_arch = "wasm32"))]
pub fn compile_to_c_via_bytecode_traced(
    source: &str,
) -> Result<(String, crate::optimization::BackendOptimizationTrace), ParseError> {
    crate::optimize::begin_backend_trace(crate::optimization::OptimizationBackend::CBytecode);
    let result = compile_to_c_via_bytecode(source);
    let trace = crate::optimize::end_backend_trace().expect("backend trace was just started");
    result.map(|c| (c, trace))
}

/// The bytecode→C walker with an explicit [`SchedulerKnobs`](crate::codegen_c::concurrency::SchedulerKnobs) for the concurrency path — the tunable
/// deterministic scheduler (ready-queue `policy`, `workers`, `clock`, channel capacity, `seed`). A
/// non-concurrency program ignores the knobs (its sequential C is knob-independent). The default-knob
/// wrapper is [`compile_to_c_via_bytecode`]; a determinism lock drives this with each policy to prove the
/// knob is a real, output-preserving tunable on the determinate corpus.
///
/// [`SchedulerKnobs`]: crate::codegen_c::concurrency::SchedulerKnobs
#[cfg(not(target_arch = "wasm32"))]
pub fn compile_to_c_via_bytecode_with_knobs(
    source: &str,
    knobs: &crate::codegen_c::concurrency::SchedulerKnobs,
) -> Result<String, ParseError> {
    compile_to_c_via_bytecode_offloaded_with_knobs(source, knobs, |_, _| Vec::new()).map(|o| o.c)
}

/// The bytecode→C walker with an offload table: `offloads` chooses, from the optimized program and the interner that
/// names its functions, the functions a device may answer
/// ([`crate::codegen_c::from_program::compile_program_to_c_offloaded`]). An empty table is
/// [`compile_to_c_via_bytecode`].
#[cfg(not(target_arch = "wasm32"))]
pub fn compile_to_c_via_bytecode_offloaded(
    source: &str,
    offloads: impl FnOnce(
        &crate::vm::CompiledProgram,
        &Interner,
    ) -> Vec<crate::codegen_c::from_program::Offload>,
) -> Result<crate::codegen_c::from_program::OffloadedC, ParseError> {
    compile_to_c_via_bytecode_offloaded_with_knobs(
        source,
        &crate::codegen_c::concurrency::SchedulerKnobs::default(),
        offloads,
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn compile_to_c_via_bytecode_offloaded_with_knobs(
    source: &str,
    knobs: &crate::codegen_c::concurrency::SchedulerKnobs,
    offloads: impl FnOnce(
        &crate::vm::CompiledProgram,
        &Interner,
    ) -> Vec<crate::codegen_c::from_program::Offload>,
) -> Result<crate::codegen_c::from_program::OffloadedC, ParseError> {
    use crate::codegen_c::from_program::OffloadedC;
    fn custom(msg: String) -> ParseError {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(msg),
            span: crate::token::Span::default(),
        }
    }
    with_optimized_program(source, true, |stmts, interner, program, policies| {
        refuse_unlinkable(program, interner, Backend::C).map_err(custom)?;
        let table = offloads(program, interner);
        let on_the_host = |why: &str| {
            table
                .iter()
                .map(|o| (o.func, why.to_string()))
                .collect::<Vec<_>>()
        };
        // The Op-walker now lowers EVERY reachable bytecode op natively — the offline `Net*`/`Crdt*` ops,
        // runtime authorization (`CheckPolicy`), AND the last holdout, the real-socket TCP client (`Tcp`, over
        // the `C_TCP_RUNTIME` POSIX socket runtime). So this delegation gate is EMPTY: `program_needs_astc_
        // delegation` always returns `false` today. The guard is kept as the honest self-documenting seam — a
        // future op that genuinely cannot be lowered would re-open it (and the census locks would force it).
        if crate::codegen_c::program_needs_astc_delegation(program) {
            crate::optimize::mark_backend_fallback(
                crate::optimization::OptimizationBackend::CAst,
                "bytecode op outside the C Op-walker",
            );
            return Ok(OffloadedC {
                c: compile_to_c(source)?,
                kept: on_the_host("the program is lowered from its syntax tree"),
            });
        }
        // A concurrency program (channels / tasks / `Await the first of` / virtual-time `Sleep`) needs a
        // task to SUSPEND at a blocking op and RESUME later — which the flat Op-walker cannot express. So
        // it is lowered by the stepped tier (each task a step function over a heap frame, the scheduler core
        // the CUDA tier runs), tuned by `SchedulerKnobs` (default = the oracle's `Fifo` / `workers=1` /
        // `Logical`). The sequential surface stays on the bytecode walker.
        if crate::codegen_c::program_uses_concurrency(stmts) {
            crate::optimize::mark_backend_fallback(
                crate::optimization::OptimizationBackend::CAst,
                "concurrency requires the stepped syntax-tree C tier",
            );
            Ok(OffloadedC {
                c: compile_to_c_stepped_with_knobs(source, knobs)?,
                kept: on_the_host("a concurrent program runs on the stepped tier"),
            })
        } else {
            // The Op-walker lowers the whole sequential surface + the offline Net/CRDT ops + `CheckPolicy`
            // (the policy registry supplies each `Check`'s condition, as the VM's `policy_ctx` does).
            crate::optimize::with_backend_lowering(|| {
                crate::codegen_c::from_program::compile_program_to_c_offloaded(
                    program,
                    Some((interner, policies)),
                    &table,
                )
            })
            .map_err(custom)
        }
    })
}

/// ★ THE FINALIZED, OPTIMIZED SOURCE-OF-TRUTH IR ★ — `compile( optimize_program( resolve_divisions(AST) ) )`.
/// This is the exact front-end `compile_to_c`/`compile_to_rust` run (prelude + `## No`/`## Require` decorators
/// + division resolution + the full AOT optimizer lattice) — but it STOPS at the `CompiledProgram` instead of
/// emitting a target. It is the one IR every backend will lower in the source-of-truth architecture, and the
/// object the invariants pin: `VM(opt P) = program_outcome(compile_to_program_optimized(P))`. Today the bytecode
/// backends (riscv/wasm/fabric) compile the RAW AST and miss this whole lattice — so this is also the primitive
/// that lets them GAIN it. Mirrors `compile_to_c` (`src/compile.rs`) up to the codegen step.
#[cfg(not(target_arch = "wasm32"))]
pub fn compile_to_program_optimized(
    source: &str,
) -> Result<crate::vm::CompiledProgram, ParseError> {
    with_optimized_program(source, false, |_stmts, _interner, program, _policies| {
        Ok(program.clone())
    })
}

/// [`compile_to_program_optimized`] with each function's name, in function order: what a consumer that
/// reports on functions by name (the accelerator region discovery) reads beside the program.
#[cfg(not(target_arch = "wasm32"))]
pub fn compile_to_program_optimized_named(
    source: &str,
) -> Result<(crate::vm::CompiledProgram, Vec<String>), ParseError> {
    with_optimized_program(source, false, |_stmts, interner, program, _policies| {
        let names = program
            .functions
            .iter()
            .map(|f| interner.resolve(f.name).to_string())
            .collect();
        Ok((program.clone(), names))
    })
}

/// Run the full front-end (prelude + decorators + division resolution + the AOT optimizer lattice) to the
/// finalized `CompiledProgram`, then hand `finish` the OPTIMIZED `stmts`, the `interner`, and the `program`
/// — all borrowed while the front-end's arenas are still live. The one seam a backend needs when it lowers
/// from the bytecode IR yet still wants the optimized AST (e.g. the walker's concurrency branch, which asks it
/// whether the program has tasks). `compile_to_program_optimized` is the trivial `finish` that
/// just clones out the program.
#[cfg(not(target_arch = "wasm32"))]
fn with_optimized_program<T>(
    source: &str,
    require_static_types: bool,
    finish: impl FnOnce(
        &[Stmt],
        &Interner,
        &crate::vm::CompiledProgram,
        &crate::analysis::PolicyRegistry,
    ) -> Result<T, ParseError>,
) -> Result<T, ParseError> {
    fn custom(msg: String) -> ParseError {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(msg),
            span: crate::token::Span::default(),
        }
    }
    let prelude_src = crate::loader::apply_prelude(source);
    let pruned = crate::loader::keep_reachable_functions(source, prelude_src.as_ref());
    let source = pruned.as_str();
    let library = crate::loader::LibraryProvenance::of(source);

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    let (type_registry, policy_registry) = {
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
    let _compile_facts = crate::optimize::facts::scope();
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

    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
    );
    let stmts = parser.parse_program()?;
    let (opt_config, gate) = crate::optimize::install_decorators(
        crate::optimization::OptimizationConfig::from_env(),
        &parser.program_opt_flags(),
        parser.required_optimizations(),
    );
    let codegen_registry = parser.into_type_registry();
    // Memoization is an AST rewrite, so it must run before the shared
    // optimizer.  Optimizer facts are keyed to the exact arena nodes they
    // prove; rewriting the tree after optimization silently discarded
    // `InBounds`, `RawIntOp`, and `TileLegal` before bytecode/JIT lowering.
    // Direct C already uses this ordering.  Keeping the optimizer last makes
    // its proof-bearing output the source consumed by every bytecode tier.
    let stmts = crate::optimize::memoize_for_explicit_lowering(
        stmts,
        &imperative_expr_arena,
        &stmt_arena,
        &mut interner,
        &opt_config,
    );
    let resolved = crate::resolve_program::resolve_program(
        &stmts,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
        &mut interner,
        opt_config.is_on(crate::optimization::Opt::Comptime),
        library.functions(),
    );
    let stmts: Vec<Stmt> = match resolved {
        Some(rw) => rw.to_vec(),
        None => stmts,
    };
    let stmts = crate::optimize::optimize_program(
        stmts,
        &imperative_expr_arena,
        &stmt_arena,
        &mut interner,
        &opt_config,
    );
    // Rust runs the shared guarded-region versioner immediately before its
    // emitter.  Bytecode-derived backends must run the same proof pass before
    // compiling their final IR; otherwise TileLegal/InBounds reach Rust alone
    // and C-bytecode/JIT silently keep exact counters and bounds checks.
    // The VM retains its dynamic value semantics. A type environment is a
    // prerequisite for this proof-driven optimization, not for executing
    // bytecode. Typed backends still require the same static validation.
    let type_env = match crate::analysis::check_program_located(&stmts, &interner, &codegen_registry) {
        Ok(env) => Some(env),
        Err(e) if require_static_types => {
            return Err(ParseError {
                kind: e.to_parse_error_kind(&interner, &stmts),
                span: crate::token::Span::default(),
            });
        }
        Err(_) => None,
    };
    let stmts = if let Some(type_env) = type_env {
        crate::optimize::version::version_stmts(
            stmts,
            crate::ast_map::Arenas {
                exprs: &imperative_expr_arena,
                stmts: &stmt_arena,
                types: &type_expr_arena,
            },
            &type_env,
            &interner,
            &opt_config,
        )
    } else {
        stmts
    };
    // The optimized AST → the finalized bytecode IR. The `CompiledProgram` owns its op stream (no arena refs),
    // so it outlives this function's arenas.
    let oracle = crate::optimize::oracle_analyze_with(&stmts, &interner);
    let program = crate::optimize::with_backend_lowering(|| {
        crate::vm::Compiler::compile_with_oracle_located(
            &stmts,
            &interner,
            Some(&codegen_registry),
            Some(oracle),
        )
    })?;
    gate.verify().map_err(|m| ParseError {
        kind: crate::error::ParseErrorKind::Custom(m),
        span: crate::token::Span::default(),
    })?;
    finish(&stmts, &interner, &program, &policy_registry)
}

/// Compile to **RV64GC assembly text** — the LOGOS → RISC-V AOT tier. Lowers from the SAME
/// Oracle-optimized bytecode the VM/WASM tiers use (so DivPow2 / bounds-elim forms match), then
/// [`crate::codegen_riscv::codegen_program_riscv`] emits the assembly. A RISC-V GCC assembles+links it;
/// `qemu-user` runs it byte-for-byte identical to the VM. Returns a `Custom` `ParseError` when the
/// program uses an op/shape outside the currently-lowered fragment (a sound refusal, never wrong code).
pub fn compile_to_riscv(source: &str) -> Result<String, ParseError> {
    fn custom(msg: String) -> ParseError {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(msg),
            span: crate::token::Span::default(),
        }
    }
    crate::ui_bridge::with_parsed_program_checked(source, |parsed, interner| {
        let (stmts, types, policies) = parsed?;
        let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
        let mut program = crate::vm::Compiler::compile_with_oracle_located(
            stmts,
            interner,
            Some(types),
            Some(oracle),
        )?;
        // Coalesce the VM compiler's register-move cascades before lowering. RISC-V asm has no
        // downstream optimizing assembler (unlike the C tier, whose gcc does this), so without the
        // peephole it ships the `Set a to b. Set b to t.` five-move idiom verbatim every iteration.
        // Semantics-preserving IR→IR (peephole_soundness_lock proves `== VM`).
        crate::vm::peephole::peephole(&mut program);
        refuse_unlinkable(&program, interner, Backend::RiscV).map_err(custom)?;
        crate::codegen_riscv::codegen_program_riscv(&program, &policies, interner)
            .map_err(|e| custom(e.to_string()))
    })
}

/// Compile to a **single-Tensix-core TT-Metalium program** — the LOGOS → Tenstorrent
/// AOT tier. The compute body is the C translation ([`compile_to_c`], byte-for-byte ≡
/// the VM), wrapped by [`crate::codegen_tt`] as a data-movement kernel on one baby
/// RISC-V core (`RISCV_0`) plus a host orchestrator. Uses the SAME Oracle-optimized
/// bytecode to scan for ops outside the single-baby-core fragment — CONCURRENCY
/// (channels/tasks/select need a `ucontext` scheduler; the mesh tier lowers those) —
/// returning a `Custom` `ParseError`, a sound refusal, never wrong code. OFFLINE
/// networking rides the newlib-clean C net runtime on-core. Returns the emitted
/// [`crate::codegen_tt::TtArtifact`].
#[cfg(feature = "codegen")]
pub fn compile_to_tenstorrent(source: &str) -> Result<crate::codegen_tt::TtArtifact, ParseError> {
    fn custom(msg: String) -> ParseError {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(msg),
            span: crate::token::Span::default(),
        }
    }
    refuse_exact(source, "Tenstorrent tier", "a Tensix core")?;
    // Scan the Oracle-optimized bytecode for ops outside the single-baby-core fragment FIRST, so a
    // concurrency program gets the backend's specific refusal rather than the C tier's generic error
    // — a sound refusal, never wrong code. (Offline networking is served on-core, not refused.)
    let refusal = crate::ui_bridge::with_parsed_program_checked(source, |parsed, interner| {
        let (stmts, types, _policies) = parsed?;
        let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
        let program = crate::vm::Compiler::compile_with_oracle_located(
            stmts,
            interner,
            Some(types),
            Some(oracle),
        )?;
        Ok(crate::codegen_tt::unsupported_reason(&program))
    })?;
    if let Some(reason) = refusal {
        return Err(custom(format!(
            "tenstorrent AOT backend: unsupported {reason}"
        )));
    }
    // The C oracle is the proven ≡VM compute body the kernel runs.
    let oracle_c = compile_to_c(source)?;
    Ok(crate::codegen_tt::wrap_as_tt(&oracle_c))
}

/// Compile to a **mixed-signal analog artifact** — the LOGOS → analog spatial fabric tier
/// ([`crate::codegen_ana`]). Scans the Oracle-optimized bytecode through the fail-closed
/// [`crate::codegen_ana::analog_refusal`] gate FIRST: concurrency/networking, a program with no
/// float dataflow, or float inputs without declared ranges each get a NAMED `analog backend:`
/// refusal — a correct artifact or a typed refusal, never a silently-wrong circuit.
#[cfg(feature = "codegen")]
pub fn compile_to_analog(source: &str) -> Result<crate::codegen_ana::AnalogArtifact, ParseError> {
    fn custom(msg: String) -> ParseError {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(msg),
            span: crate::token::Span::default(),
        }
    }
    refuse_exact(source, "analog tier", "an analog fabric")?;
    let gate = crate::ui_bridge::with_parsed_program_checked(source, |parsed, interner| {
        let (stmts, types, _policies) = parsed?;
        let ranges = crate::codegen_ana::assumed_ranges_in(stmts, interner);
        let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
        let program = crate::vm::Compiler::compile_with_oracle_located(
            stmts,
            interner,
            Some(types),
            Some(oracle),
        )?;
        let refusal = crate::codegen_ana::analog_refusal(&program, &ranges);
        // The region unit is the FIRST function — its params are the DAC inputs.
        let region = stmts.iter().find_map(|s| match s {
            logicaffeine_language::ast::stmt::Stmt::FunctionDef { name, .. } => {
                Some(interner.resolve(*name).to_string())
            }
            _ => None,
        });
        Ok((refusal, region))
    })?;
    let target = crate::codegen_ana::AnalogTarget::sky130_fpaa();
    match gate {
        // Concurrency/networking (and float-without-ranges) refuse coarse; a NoAnalogRegion
        // verdict with a region function present falls through to the lane dispatcher, which
        // decides float vs certified-exact integer (panic-safe, typed refusals).
        (Some(crate::codegen_ana::AnaLowerError::NoAnalogRegion), Some(fn_name))
        | (None, Some(fn_name)) => crate::codegen_ana::try_region(source, &fn_name, &target)
            .map(|(_lane, art)| art)
            .map_err(|e| custom(format!("analog backend: {e}"))),
        (Some(e), _) => Err(custom(format!("analog backend: {e}"))),
        (None, None) => Err(custom(
            "analog backend: no region function found (the analog region unit is a float function)"
                .to_string(),
        )),
    }
}

/// Compile to the **Plasticine** target — the open RDU (Prabhakar et al., ISCA 2017), the LOGOS → Plasticine
/// lane. The region unit is the FIRST function, which must be a counted range loop (Spatial `Foreach` over
/// `lo..=hi`): it is lowered through the certified [`crate::codegen_sva::fabric_rtl::LoopFabric`], modeled as
/// one metapipeline stage under Spatial's II definition, placed on the ISCA'17 checkerboard, anchored to the
/// VM at sample inputs, SAT-certified when its body is ALU-only, and emitted as Spatial source. Any other
/// shape gets a NAMED `plasticine backend:` refusal — an artifact or a typed refusal, never a wrong model.
#[cfg(feature = "codegen")]
pub fn compile_to_plasticine(
    source: &str,
) -> Result<crate::codegen_sva::plasticine_report::PlasticineArtifact, ParseError> {
    fn custom(msg: String) -> ParseError {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(msg),
            span: crate::token::Span::default(),
        }
    }
    refuse_exact(source, "Plasticine tier", "a dataflow fabric")?;
    let region = crate::ui_bridge::with_parsed_program_checked(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed?;
        Ok(stmts.iter().find_map(|s| match s {
            Stmt::FunctionDef { name, params, .. } => {
                Some((interner.resolve(*name).to_string(), params.len()))
            }
            _ => None,
        }))
    })?;
    let Some((fn_name, arity)) = region else {
        return Err(custom("plasticine backend: no region function found (the Plasticine region unit is a counted-loop function)".to_string()));
    };
    // The VM anchor's sample inputs: small, distinct, positive.
    let params: Vec<i64> = (0..arity).map(|i| 8 + i as i64).collect();
    let target = crate::codegen_sva::plasticine::PlasticineTarget::isca17();
    crate::codegen_sva::plasticine_report::lower_counted_loop(source, &fn_name, &params, &target)
        .map_err(|e| custom(format!("plasticine backend: {e}")))
}

/// Which RDU lane an artifact came through — chosen by program shape ([`crate::spatial::lower::rdu_lanes::lane_of`]),
/// printed by name.
#[cfg(feature = "codegen")]
pub use crate::spatial::lower::rdu_lanes::RduLane;

/// What `largo emit rdu` writes: the `logos.spatial.neutral` v2 envelope, the placement text, the
/// certificate (rechecked against the program, carrying the envelope's digest), the MODELED report naming
/// the lane, and — when the lane produced one — the Spatial source.
#[cfg(feature = "codegen")]
#[derive(Clone, Debug)]
pub struct RduArtifact {
    pub chip: &'static str,
    pub lane: RduLane,
    pub neutral_json: String,
    pub placement_txt: String,
    pub certificate_json: String,
    pub report_json: String,
    pub spatial_source: Option<String>,
    /// The SDK-gated artifact itself, for a bridge to ingest (`None` for a lane whose envelope carries no
    /// spatial program).
    pub neutral: Option<crate::spatial::lower::NeutralArtifact>,
    /// What a lane could not do, by name (the Spatial emitter's refusal, …).
    pub notes: Vec<String>,
}

pub const RDU_EMIT_REPORT_SCHEMA: &str = "logos.rdu.emit_report";

/// A streamed model bundle's manifest, the model constants it binds from files no checkpoint holds — each at the
/// path the manifest binds it at, relative to the bundle — which are written with it, and the LOGOS text of the whole
/// cut of the model's one program the bundle deploys (`program.lg`), which the manifest's `program` line identifies.
pub struct RduBundle {
    pub manifest: String,
    pub constants: Vec<(String, Vec<u8>)>,
    pub program: String,
}

/// One lowered program of a streamed model bundle, rendered to the files a bundle directory holds.
pub struct RduProgramFiles {
    pub dir: String,
    pub neutral_json: String,
    pub placement_txt: String,
    pub certificate_json: String,
}

pub const RDU_STREAMED_MANIFEST_SCHEMA: &str = "logos.rdu.streamed_manifest";

/// A step deployment's shape: a KV cache of `ctx` positions per sequence, `batch` sequences per step, over `tp`
/// chips of tensor parallelism — and, with `prefill: Some(chunk)`, a step that PREFILLS `chunk` tokens of one sequence
/// (their hidden states stacked, the sequence's state taken row by row: the model's prefill cut) instead of decoding one
/// token per sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepShape {
    pub ctx: usize,
    pub batch: usize,
    pub tp: usize,
    pub prefill: Option<usize>,
}

/// A model the RDU bundle is compiled from: its spec, the name the manifest gives it, which of the weights the author
/// declares Q8_0 its checkpoint stores Q8_0, and the per-frequency factors of a rotation that reads them
/// (`rope_freqs.weight`).
pub struct RduModel<'a> {
    pub spec: crate::model::spec::ModelSpec,
    pub name: String,
    pub storage: crate::spatial::streamed::Storage<'a>,
    pub rope_factors: Option<Vec<f64>>,
}

impl<'a> RduModel<'a> {
    /// A built-in configuration of Qwen3.8, whose checkpoint stores every projection Q8_0.
    pub fn qwen38(cfg: &crate::codegen_cuda::qwen38::Qwen38Cfg) -> RduModel<'static> {
        RduModel { spec: cfg.spec(), name: "Qwen3.8".to_string(), storage: crate::spatial::streamed::Storage::AllQ8, rope_factors: None }
    }

    /// The model a checkpoint holds, named `name`: its spec, each weight read as the checkpoint stores it, and the
    /// frequency factors its rotations read (`rope_freqs.weight`) when it carries them.
    pub fn from_checkpoint(name: &str, checkpoint: &'a mut dyn crate::model::Checkpoint) -> Result<RduModel<'a>, String> {
        use crate::model::bind::{values, Binding, View};
        use crate::model::reference::WeightSource;
        let spec = crate::model::spec::ModelSpec::from_source(&*checkpoint).map_err(|e| match e {
            crate::model::spec::SpecError::UnknownArchitecture(_) => format!("{name}: {}", crate::model::triage::refusal(&*checkpoint)),
            e => format!("{name}: {e:?}"),
        })?;
        let rope_factors = if WeightSource::has(&*checkpoint, "rope_freqs.weight") {
            let b = Binding { tensor: "rope_freqs.weight".to_string(), view: View::Same };
            Some(values(&b, &mut *checkpoint).map_err(|e| format!("{name}: rope_freqs.weight: {e}"))?)
        } else {
            None
        };
        let source: &'a dyn WeightSource = checkpoint;
        Ok(RduModel { spec, name: name.to_string(), storage: crate::spatial::streamed::Storage::Checkpoint(source), rope_factors })
    }
}

/// Lower a model to the RDU as the chip would run it: ONE program per layer kind plus the head, each lowered,
/// certified and handed to `sink` in turn, and a manifest binding every layer to its kind. Returns the
/// manifest JSON and the model constants the bundle ships: a decode deployment's rotary caches
/// (`{weights_dir}/Rcos.f`, `{weights_dir}/Rsin.f`, one pair per rotation the layers turn by —
/// [`rope_tables`](crate::model::bind::rope_tables) at `ctx` positions, little-endian f32), which every step reads at
/// its positions so the host hands in only tokens and positions.
///
/// Layers with equal specs lower to one dataflow (`spatial::streamed::layer_kinds`), so the whole forward —
/// however deep — is a program per kind and the head. Programs are rendered and sunk one at a time, so the caller
/// never holds more than one program's JSON. The weights stay outside the programs: the manifest names, for
/// every layer, which file each of its kind's tensors is read from, and each tensor's role.
///
/// `decode: Some(StepShape { ctx, batch, tp })` lowers the DECODE deployment instead of the authored forward: an
/// embedding program, layer kinds whose attention appends to and reads a KV cache of `ctx` positions, and a head that
/// emits the next token (see `streamed::lower_decode_streamed`) — for `batch` sequences per step, every
/// weight read once for all of them, when `batch > 1` (`streamed::lower_batched_decode_streamed`); over
/// `tp` chips, each layer kind one program with every chip its own heads and a slice of every weight, when `tp > 1`
/// (`streamed::lower_tp_decode_streamed`, the manifest naming each chip tensor's slice).
pub fn compile_model_to_rdu_streamed(
    rdu: &RduModel,
    chip: crate::spatial::rdu_chip::RduChip,
    weights_dir: &str,
    decode: Option<StepShape>,
    mut sink: impl FnMut(RduProgramFiles) -> std::io::Result<()>,
) -> Result<RduBundle, String> {
    use crate::model::program::{written_in, written_split, Cut};
    use crate::spatial::lower::rdu::{certificate_summary, RduSpatialBackend};
    use crate::spatial::lower::{SpatialArtifact, SpatialBackend};
    use crate::spatial::streamed::rebind;
    use crate::spatial::streamed::{
        lower_batched_decode_streamed, lower_batched_tp_decode_streamed, lower_decode_streamed,
        lower_forward_streamed, lower_prefill_streamed, lower_tp_decode_streamed, StreamedReject,
    };
    let (spec, storage) = (&rdu.spec, rdu.storage);
    crate::model::author::check(spec).map_err(|e| format!("{}: {e}", rdu.name))?;

    let facts = chip.facts();
    let target = facts.spatial_target();
    let model = match decode {
        None => lower_forward_streamed(spec, storage, &target, |s| s.to_string()),
        Some(StepShape { ctx, batch: 1, tp: 1, prefill: Some(chunk) }) => lower_prefill_streamed(spec, storage, ctx, chunk, &target, |s| s.to_string()),
        Some(StepShape { batch, tp, prefill: Some(chunk), .. }) => Err(StreamedReject {
            program: "bundle".into(),
            why: format!("a prefill chunk ({chunk} tokens) is one sequence on one chip — not {batch} sequences over {tp} chips"),
        }),
        Some(StepShape { ctx, batch: 1, tp: 1, prefill: None }) => lower_decode_streamed(spec, storage, ctx, &target, |s| s.to_string()),
        Some(StepShape { ctx, batch, tp: 1, prefill: None }) => lower_batched_decode_streamed(spec, storage, ctx, batch, &target, |s| s.to_string()),
        Some(StepShape { ctx, batch: 1, tp, prefill: None }) => lower_tp_decode_streamed(spec, storage, ctx, tp, &target, |s| s.to_string()),
        Some(StepShape { ctx, batch, tp, prefill: None }) => lower_batched_tp_decode_streamed(spec, storage, ctx, batch, tp, &target, |s| s.to_string()),
    }
    .map_err(|e| format!("rdu backend: {}: {e}", rdu.name))?;
    // The weight files: the tensors the model declares with one. Carried state (the KV caches, the Gated-DeltaNet
    // memories and conv rings) and the compiler's own tensors have none; a chip's slice is read from its full
    // tensor's file.
    let ((_, f32w, q8w), _) = match decode {
        None => written_in(spec, Cut::Forward, weights_dir),
        Some(StepShape { ctx, batch: 1, .. }) => written_in(spec, Cut::Decode { ctx }, weights_dir),
        Some(StepShape { ctx, batch, .. }) => written_in(spec, Cut::Batch { rows: batch, ctx }, weights_dir),
    };
    // The whole cut of the model's one program the bundle deploys, the program its embedding, layers and head are
    // cuts of: the identity a CUDA lane or an engine program compiled from the same cut records too.
    let (whole_text, whole) = match decode {
        None => {
            let ((text, _, _), id) = written_in(spec, Cut::Forward, weights_dir);
            (text, id)
        }
        Some(StepShape { ctx, batch: 1, tp: 1, prefill: Some(rows) }) => {
            let ((text, _, _), id) = written_in(spec, Cut::Prefill { rows, ctx }, weights_dir);
            (text, id)
        }
        Some(StepShape { ctx, batch: 1, tp: 1, prefill: None }) => {
            let ((text, _, _), id) = written_in(spec, Cut::Decode { ctx }, weights_dir);
            (text, id)
        }
        Some(StepShape { ctx, batch, tp: 1, .. }) => {
            let ((text, _, _), id) = written_in(spec, Cut::Batch { rows: batch, ctx }, weights_dir);
            (text, id)
        }
        Some(StepShape { ctx, batch, tp, .. }) => {
            let cut = if batch == 1 { Cut::Decode { ctx } } else { Cut::Batch { rows: batch, ctx } };
            let ((text, _, _, _), id) =
                written_split(spec, cut, weights_dir, tp).map_err(|e| format!("rdu backend: {} over {tp} chips: {e}", rdu.name))?;
            (text, id)
        }
    };
    let file_of = |var: &str| -> Option<String> {
        f32w.get(var)
            .map(|(path, _)| path.clone())
            .or_else(|| q8w.get(var).cloned())
    };
    // A tensor a section updates in place is `state` only when a step carries it to the next — what a deployment
    // allocates once for the session; a table the step assembles whole is one of the step's activations.
    let role_of = |t: &crate::spatial::ir::TensorDecl,
                   carried: &std::collections::BTreeSet<String>| match t.role {
        crate::spatial::ir::TensorRole::State if !carried.contains(&t.name) => {
            "activation".to_string()
        }
        r => format!("{r:?}").to_lowercase(),
    };
    let none = std::collections::BTreeSet::new();
    // A chip's tensor names its chip and, when it is a slice of a checkpoint or carried-state tensor, the full tensor,
    // its file, and the rows or columns of it the chip loads.
    let bindings = |program: &crate::spatial::ir::SpatialProgram,
                    rename: &dyn Fn(&str) -> String,
                    tp: Option<&crate::spatial::tensor_parallel::TpManifest>,
                    carried: &std::collections::BTreeSet<String>|
     -> Vec<serde_json::Value> {
        program
            .tensors
            .iter()
            .map(|t| {
                let to = rename(&t.name);
                // A weight read as it is stored Q8_0 is two tensors of its checkpoint file: the int8 values (`K × N`) and one
                // fp32 scale per 32 rows of each column (`K/32 × N`), which the host repacks from the file's blocks.
                let (stored, repack) = match crate::spatial::passes::from_q8::stored_part(&to) {
                    Some((w, part)) => (w.to_string(), Some(part.repack())),
                    None => (to.clone(), None),
                };
                let mut entry = serde_json::json!({ "program": t.name, "layer": to, "file": file_of(&stored), "role": role_of(t, carried) });
                if let Some(r) = repack {
                    entry["repack"] = serde_json::json!(r);
                }
                if let Some(chip) = t.chip {
                    entry["chip"] = serde_json::json!(chip.0);
                }
                if let Some(s) = tp.and_then(|m| m.slices.get(&t.name)) {
                    let of = rename(&s.of);
                    entry["slice"] = serde_json::json!({ "of": of, "file": file_of(&of), "axis": s.axis, "ranges": s.ranges, "of_shape": s.of_shape });
                }
                // A chip's Q8_0 plane is the same rows or columns of the FULL weight's plane: the values at the weight's
                // own coordinates, the scales at one row per 32 of them (a row slice folds only in whole Q8_0 blocks).
                let plane = crate::spatial::passes::from_q8::stored_part(&t.name).and_then(|(w, part)| tp.and_then(|m| m.slices.get(w)).map(|s| (s, part)));
                if let Some((s, part)) = plane {
                    use crate::spatial::passes::from_q8::{stored_name, Q8Part, Q8_BLOCK};
                    use crate::spatial::tensor_parallel::SliceAxis;
                    let of = rename(&s.of);
                    let (ranges, of_shape): (Vec<(usize, usize)>, (usize, usize)) = match (part, s.axis) {
                        (Q8Part::Values, _) => (s.ranges.clone(), s.of_shape),
                        (Q8Part::Scales, SliceAxis::Rows) => (s.ranges.iter().map(|(a, n)| (a / Q8_BLOCK, n / Q8_BLOCK)).collect(), (s.of_shape.0 / Q8_BLOCK, s.of_shape.1)),
                        (Q8Part::Scales, SliceAxis::Cols) => (s.ranges.clone(), (s.of_shape.0 / Q8_BLOCK, s.of_shape.1)),
                    };
                    entry["slice"] = serde_json::json!({ "of": stored_name(&of, part), "file": file_of(&of), "repack": part.repack(), "axis": s.axis, "ranges": ranges, "of_shape": of_shape });
                }
                entry
            })
            .collect()
    };

    let mut render = |dir: &str,
                      program: &crate::spatial::ir::SpatialProgram|
     -> Result<(u64, bool), String> {
        let artifact = match RduSpatialBackend::with_facts(facts.clone()).lower(program) {
            Ok(SpatialArtifact::SdkGated(a)) => a,
            other => {
                return Err(format!(
                "rdu backend: {dir}: the backend did not produce an SDK-gated artifact: {other:?}"
            ))
            }
        };
        // The backend proved the certificate by rechecking the one the program carries, or derived it; a derived one is
        // rechecked here, so every certificate a bundle ships has been rechecked against its program.
        let rechecked = artifact.rechecked || artifact.certificate.recheck(program);
        let mut cert = certificate_summary(&artifact.certificate);
        cert["digest_hex"] =
            serde_json::json!(format!("{:016x}", artifact.certificate.schedule_digest));
        cert["rechecked"] = serde_json::json!(rechecked);
        let digest = artifact.certificate.schedule_digest;
        sink(RduProgramFiles {
            dir: dir.to_string(),
            neutral_json: artifact.json,
            placement_txt: artifact.pretty,
            certificate_json: format!("{}\n", serde_json::to_string_pretty(&cert).expect("JSON")),
        })
        .map_err(|e| format!("rdu backend: writing {dir}: {e}"))?;
        Ok((digest, rechecked))
    };

    let embed = match &model.embed {
        Some(p) => {
            let (digest, rechecked) = render("embed", p)?;
            let program = model.embed_identity.map(|id| id.line());
            Some(
                serde_json::json!({ "dir": "embed", "program": program, "schedule_digest": format!("{digest:016x}"), "rechecked": rechecked, "tensors": bindings(p, &|n| n.to_string(), None, &none) }),
            )
        }
        None => None,
    };
    let mut kinds = Vec::new();
    for k in &model.kinds {
        let dir = k.name.replace(' ', "_");
        let (digest, rechecked) = render(&dir, &k.program)?;
        kinds.push(serde_json::json!({
            "name": k.name, "dir": dir, "representative_layer": k.representative, "program": k.identity.line(),
            "schedule_digest": format!("{digest:016x}"), "rechecked": rechecked,
            "sections": k.program.sections.len(), "stages": k.program.stages.len(),
        }));
    }
    let (head_digest, head_rechecked) = render("head", &model.head)?;
    let layers: Vec<serde_json::Value> = model
        .layers
        .iter()
        .map(|b| {
            let kind = &model.kinds[b.kind];
            let mut map: HashMap<&str, &str> = HashMap::with_capacity(b.tensors.len());
            for (from, to) in &b.tensors {
                map.entry(from.as_str()).or_insert(to.as_str());
            }
            let (rep, layer) = (kind.representative, b.layer);
            let rename = |n: &str| {
                map.get(n)
                    .map(|to| to.to_string())
                    .unwrap_or_else(|| rebind(n, rep, layer))
            };
            let tensors = bindings(&kind.program, &rename, kind.tp.as_ref(), &kind.carried);
            serde_json::json!({ "layer": b.layer, "kind": b.kind, "program": b.identity.line(), "tensors": tensors })
        })
        .collect();
    let mut manifest = serde_json::json!({
        "schema": RDU_STREAMED_MANIFEST_SCHEMA,
        "version": 1,
        "chip": chip.as_str(),
        "target": target.name,
        "model": { "name": rdu.name, "arch": spec.arch, "layers": spec.layers.len(), "d": spec.d, "vocab": spec.vocab },
        "form": if decode.is_some() { "decode" } else { "forward" },
        "program": whole.line(),
        "provenance": crate::spatial::fit::PROVENANCE_MODELED,
        "no_speed_claim": true,
        "kinds": kinds,
        "head": { "dir": "head", "program": model.head_identity.line(), "schedule_digest": format!("{head_digest:016x}"), "rechecked": head_rechecked, "tensors": bindings(&model.head, &|n| n.to_string(), None, &none) },
        "layers": layers,
    });
    // Where the host allocates each carried state: the SRAM each chip keeps for it beside the largest program's own
    // footprint, or HBM/DDR — per state tensor of each program, once for every layer that runs the program.
    let r = &model.residency;
    let tier_name = |t: &crate::spatial::ir::TensorSpace| match t {
        crate::spatial::ir::TensorSpace::Sram => "sram",
        crate::spatial::ir::TensorSpace::Hbm => "hbm",
        crate::spatial::ir::TensorSpace::Dram => "ddr",
    };
    let program_of = |name: &str| -> Option<&crate::spatial::ir::SpatialProgram> {
        match name {
            "embed" => model.embed.as_ref(),
            "head" => Some(&model.head),
            kind => model
                .kinds
                .iter()
                .find(|k| k.name == kind)
                .map(|k| &k.program),
        }
    };
    let role_name = |c: &crate::spatial::residency::StateClass| {
        let t = program_of(&c.program).and_then(|p| p.tensors.iter().find(|t| t.name == c.tensor));
        match t.map(|t| {
            (
                t.role,
                file_of(&c.tensor).is_some()
                    || crate::spatial::passes::from_q8::stored_part(&c.tensor).is_some(),
            )
        }) {
            Some((
                crate::spatial::ir::TensorRole::Param | crate::spatial::ir::TensorRole::Input,
                true,
            )) => "weight",
            Some((r, _)) => match r {
                crate::spatial::ir::TensorRole::Input => "input",
                crate::spatial::ir::TensorRole::Output => "output",
                crate::spatial::ir::TensorRole::Param => "constant",
                crate::spatial::ir::TensorRole::Activation => "activation",
                crate::spatial::ir::TensorRole::State => "table",
            },
            None => "tensor",
        }
    };
    manifest["residency"] = serde_json::json!({
        "provenance": crate::spatial::fit::PROVENANCE_MODELED,
        "sram_bytes_per_chip": r.sram_bytes,
        "hbm_bytes_per_chip": r.hbm_bytes,
        "reserve_bytes": r.reserve.iter().map(|(c, b)| (c.to_string(), *b)).collect::<std::collections::BTreeMap<_, _>>(),
        "states": r.placed.iter().map(|(c, tier)| serde_json::json!({
            "program": c.program, "tensor": c.tensor, "chip": c.chip, "instances": c.instances,
            "bytes": c.bytes, "moved_per_step": c.moved, "tier": tier_name(tier),
        })).collect::<Vec<_>>(),
        "off_chip": r.off_chip.iter().map(|(c, tier)| serde_json::json!({
            "program": c.program, "tensor": c.tensor, "role": role_name(c), "chip": c.chip, "instances": c.instances,
            "bytes": c.bytes, "moved_per_step": c.moved, "tier": tier_name(tier),
        })).collect::<Vec<_>>(),
    });
    // A decode bundle serves every step: the host supplies each sequence's token and position, and allocates the
    // carried state (every `state` tensor, one set per sequence) once, zeroed, for the session.
    if let (
        Some(StepShape {
            ctx,
            batch,
            tp,
            prefill,
        }),
        Some(embed),
    ) = (decode, embed)
    {
        manifest["ctx"] = serde_json::json!(ctx);
        manifest["batch"] = serde_json::json!(batch);
        manifest["tensor_parallel"] = serde_json::json!(tp);
        // What a host hands this bundle, or takes from it, when another chip prefills or decodes the sequence.
        manifest["handoff"] = crate::spatial::handoff::contract(spec, ctx).map_err(|e| e.to_string())?.to_json();
        // A prefill bundle fills one sequence's state — the decode bundle's, by name — from a chunk of its prompt.
        if let Some(chunk) = prefill {
            manifest["form"] = serde_json::json!("prefill");
            manifest["chunk"] = serde_json::json!(chunk);
        }
        // The plan the chips and the model imply at this context, beside the shape this bundle was compiled at.
        manifest["serving_plan"] = match crate::codegen_cuda::qwen38::Qwen38Cfg::from_spec(spec) {
            Ok(cfg) => match crate::spatial::fit::qwen38_serving_plan(&cfg, &facts, ctx) {
                Ok(p) => serde_json::to_value(p).expect("JSON"),
                Err(why) => serde_json::json!({ "refused": why }),
            },
            Err(_) => serde_json::json!({ "refused": format!("the serving plan sizes the Qwen3.5/3.8 hybrid, not {} (CHIP-5)", spec.arch) }),
        };
        // A stacked step's token ids are one vector; a batch's sequences each bring a position, a chunk's rows the first.
        manifest["runtime_inputs"] = match (prefill, batch) {
            (None, 1) => serde_json::json!(["tok", "pos"]),
            (Some(_), _) => serde_json::json!(["toks", "pos_s0"]),
            (None, rows) => serde_json::json!(std::iter::once("toks".to_string())
                .chain((0..rows).map(|b| format!("pos_s{b}")))
                .collect::<Vec<_>>()),
        };
        manifest["embed"] = embed;
    }
    let constants = match decode {
        None => Vec::new(),
        Some(StepShape { ctx, .. }) => {
            use crate::model::bind::{rope_tables, rotations, RotaryTable};
            use crate::model::spec::RopeScaling;
            let le = |v: Vec<f64>| v.into_iter().flat_map(|x| (x as f32).to_le_bytes()).collect::<Vec<u8>>();
            let mut constants = Vec::new();
            for (rotation, rope) in rotations(spec).into_iter().enumerate() {
                let factors = match rope.scaling {
                    RopeScaling::Factors => Some(rdu.rope_factors.as_deref().ok_or_else(|| {
                        format!("rdu backend: {}: rotation {rotation} reads the checkpoint's frequency factors, and none were given", rdu.name)
                    })?),
                    _ => None,
                };
                let (cos, sin) = rope_tables(rope, ctx, factors);
                for (is_cos, values) in [(true, cos), (false, sin)] {
                    let name = RotaryTable { rotation, cached: true, cos: is_cos }.name();
                    constants.push((format!("{weights_dir}/{name}.f"), le(values)));
                }
            }
            // The other tables the host computes: a weightless norm's ones, the attention temperature's rotation.
            for table in crate::model::bind::computed_tables(spec) {
                let values = crate::model::bind::computed_values(spec, table, ctx).expect("a table the spec gives");
                constants.push((format!("{weights_dir}/{}.f", table.name()), le(values)));
            }
            constants
        }
    };
    Ok(RduBundle {
        manifest: format!(
            "{}\n",
            serde_json::to_string_pretty(&manifest).expect("JSON")
        ),
        constants,
        program: whole_text,
    })
}

/// Compile to the **SambaNova RDU** — ONE front door, three lanes chosen by program shape: the **tensor lane**
/// (a tensor program lifted through the spatial IR, placed on the chip's descriptor and certified), the
/// **loop lane** (a counted-range-loop function through the Plasticine metapipeline model on the RDU mesh
/// descriptor), and the **whole-program lane** (any construct the fabric's decision datapath covers, as
/// `Scalar` stages). Every lane ends in the same certified neutral envelope, honestly SDK-gated for the
/// vendor config bytes. A tensor program the chip cannot host is refused BY NAME on the tensor lane, never
/// handed on; a program no lane hosts gets a `rdu backend:` refusal naming every lane's verdict.
#[cfg(feature = "codegen")]
pub fn compile_to_rdu(
    source: &str,
    chip: crate::spatial::rdu_chip::RduChip,
) -> Result<RduArtifact, ParseError> {
    use crate::codegen_sva::plasticine_report::counted_loop_fabric;
    use crate::spatial::lower::rdu_lanes::{
        first_function, lane_of, lower_loop_lane, lower_whole_program_lane,
    };
    use crate::spatial::passes::SpatialReject;
    use crate::spatial::pipeline::compile_program_to_spatial;
    fn custom(msg: String) -> ParseError {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(msg),
            span: crate::token::Span::default(),
        }
    }
    refuse_exact(source, "RDU tier", "a dataflow fabric")?;
    let facts = chip.facts();
    let target = facts.spatial_target();
    match lane_of(source) {
        // ★ THE WHOLE TENSOR PROGRAM, OR A REFUSAL THAT NAMES THE FRACTION.
        //
        // This arm used to call `template_from_source`, which lifts the FIRST recognized op and lowers
        // THAT — so a 183-statement transformer forward came back exit-0 with a four-file bundle whose
        // program was `matmul_1x4x8`, one q-projection, with nothing saying so. A partial lowering
        // reported as a whole one is the single failure this campaign cannot ship: the people reading
        // these bundles design the chip, and they will ask what it covers.
        RduLane::Tensor => match compile_program_to_spatial(source, &target) {
            Ok(lowering) => match lowering.program.as_ref() {
                Some(program) => {
                    let coverage = serde_json::json!({
                        "whole_program": true,
                        "statements_total": lowering.statements,
                        "statements_lowered": lowering.lowered,
                        "sections": lowering.sections,
                        "state_tensors": lowering.state,
                    });
                    lower_rdu_spatial(program, chip, &facts, RduLane::Tensor, Some(coverage)).map_err(custom)
                }
                None => {
                    let first: Vec<String> = lowering
                        .missing
                        .iter()
                        .take(3)
                        .map(|m| format!("stmt {} `{}` [{}]: {}", m.stmt_index, m.binder, m.kind, m.why))
                        .collect();
                    Err(custom(format!(
                        "rdu backend: tensor lane: the {} cannot host {} of this program's {} tensor statements — REFUSING rather than reporting a fraction as the whole program ({} lowered into {} section(s)). Missing: {}",
                        target.name,
                        lowering.missing.len(),
                        lowering.statements,
                        lowering.lowered,
                        lowering.sections,
                        if first.is_empty() { "(none listed)".to_string() } else { first.join(" | ") },
                    )))
                }
            },
            Err(SpatialReject::Parse(e)) => Err(custom(format!("rdu backend: tensor lane: the program does not parse: {e}"))),
            Err(SpatialReject::Unrecognized { matmul, elementwise, reduction }) => Err(custom(format!(
                "rdu backend: tensor lane: the program declares tensors but no recognizer of the whole-program walk lifts an op from it (matmul: {matmul:?}; elementwise: {elementwise:?}; reduction: {reduction:?})",
            ))),
            Err(other) => Err(custom(format!("rdu backend: tensor lane: the {} cannot host this tensor program: {other:?}", target.name))),
        },
        RduLane::Loop => {
            let art = lower_loop_lane(source, chip).map_err(|e| custom(format!("rdu backend: loop lane: {e}")))?;
            Ok(loop_lane_artifact(art, chip, &facts))
        }
        RduLane::WholeProgram => match lower_whole_program_lane(source, chip) {
            Ok(program) => lower_rdu_spatial(&program, chip, &facts, RduLane::WholeProgram, None).map_err(custom),
            Err(reject) => {
                let loop_verdict = match first_function(source) {
                    Some((name, _)) => match counted_loop_fabric(source, &name) {
                        Ok(_) => "a counted loop (would have taken the loop lane)".to_string(),
                        Err(why) => format!("`{name}` is not a counted range loop ({why})"),
                    },
                    None => "no function to take (the loop lane's unit is a counted-loop function)".to_string(),
                };
                Err(custom(format!(
                    "rdu backend: no lane hosts this program on the {} — lane by shape: whole-program; tensor lane: no tensor declared and no tensor op recognized; loop lane: {loop_verdict}; whole-program lane: {reject:?}",
                    target.name
                )))
            }
        },
    }
}

/// The loop lane's pieces as the four files: the placement text is the lane's one-screen summary, the
/// certificate is the SAT certificate summary bound by digest, the report wraps the modeled Plasticine row,
/// and the envelope carries the placement, the row, the Spatial source and the certificate under
/// `lane: "loop"` in the v2 shape (the target block from the same facts as every other envelope).
#[cfg(feature = "codegen")]
fn loop_lane_artifact(
    art: crate::spatial::lower::rdu_lanes::LoopLaneArtifact,
    chip: crate::spatial::rdu_chip::RduChip,
    facts: &crate::spatial::rdu_chip::RduChipFacts,
) -> RduArtifact {
    use crate::spatial::fit::{ChipBlock, HONESTY, PROVENANCE_MODELED};
    use crate::spatial::lower::rdu::{
        NeutralTarget, GATED_REASON, NEUTRAL_SCHEMA, NEUTRAL_VERSION,
    };
    let target = facts.spatial_target();
    let parse =
        |s: &str| serde_json::from_str::<serde_json::Value>(s).expect("the loop lane renders JSON");
    let pretty =
        |v: &serde_json::Value| format!("{}\n", serde_json::to_string_pretty(v).expect("JSON"));
    let (placement, row, certificate) = (
        parse(&art.placement_json),
        parse(&art.report_json),
        parse(&art.certificate_json),
    );
    let envelope = serde_json::json!({
        "schema": NEUTRAL_SCHEMA,
        "version": NEUTRAL_VERSION,
        "lane": RduLane::Loop,
        "target": NeutralTarget::of(&target, facts),
        "sections": [],
        "state": [],
        "gathers": [],
        "ii_report": {},
        "loop": {
            "function": art.plasticine.fn_name,
            "plasticine_target": art.plasticine.target,
            "placement": placement,
            "row": row,
            "spatial_source": art.spatial_source,
        },
        "certificate": certificate,
    });
    let report = serde_json::json!({
        "schema": RDU_EMIT_REPORT_SCHEMA,
        "version": 1,
        "provenance": PROVENANCE_MODELED,
        "no_speed_claim": true,
        "honesty": HONESTY,
        "lane": RduLane::Loop,
        "chip": chip.as_str(),
        "target": target.name,
        "gated": GATED_REASON,
        "program": { "name": art.plasticine.fn_name, "kind": "counted_loop", "plasticine_target": art.plasticine.target },
        "certificate": certificate,
        "plasticine_row": row,
        "spatial_source": true,
        "notes": [],
        "chip_facts": ChipBlock::of(facts),
    });
    RduArtifact {
        chip: chip.as_str(),
        lane: RduLane::Loop,
        neutral_json: serde_json::to_string(&envelope).expect("the loop envelope serializes"),
        placement_txt: art.pretty,
        certificate_json: art.certificate_json,
        report_json: pretty(&report),
        spatial_source: Some(art.spatial_source),
        neutral: None,
        notes: Vec::new(),
    }
}

/// A verified spatial program through the RDU lane: the envelope, the placement text, the rechecked
/// certificate, the report with the II view, and the Spatial source when the emitter spells the program.
#[cfg(feature = "codegen")]
fn lower_rdu_spatial(
    program: &crate::spatial::ir::SpatialProgram,
    chip: crate::spatial::rdu_chip::RduChip,
    facts: &crate::spatial::rdu_chip::RduChipFacts,
    lane: RduLane,
    coverage: Option<serde_json::Value>,
) -> Result<RduArtifact, String> {
    use crate::spatial::fit::{
        CertificateSummary, ChipBlock, IiBlock, ProgramSummary, HONESTY, PROVENANCE_MODELED,
    };
    use crate::spatial::lower::rdu::RduSpatialBackend;
    use crate::spatial::lower::spatial_lang::emit_spatial_program;
    use crate::spatial::lower::{SpatialArtifact, SpatialBackend};
    use crate::spatial::rdu_chip::rdu_ii_report;
    let target = facts.spatial_target();
    let artifact = match RduSpatialBackend::with_facts(facts.clone()).lower(program) {
        Ok(SpatialArtifact::SdkGated(a)) => a,
        Ok(other) => {
            return Err(format!(
                "rdu backend: {} lane: the {} lane is SDK-gated, got {other:?}",
                lane.as_str(),
                target.name
            ))
        }
        Err(e) => {
            return Err(format!(
                "rdu backend: {} lane: the {} refused the verified program {}: {e:?}",
                lane.as_str(),
                target.name,
                program.name
            ))
        }
    };
    // ★ READ THE CERTIFICATE FROM THE CERTIFICATE, NOT FROM ITS OWN SERIALIZATION. The envelope's
    // `certificate` block is `certificate_summary(&artifact.certificate)` — built from the value this
    // artifact already carries. Recovering it by parsing the whole envelope back into a
    // `serde_json::Value` cost 67.7 GB on ONE real-width Qwen3.8 layer (a 2.1 GB neutral string becomes
    // a tree ~30x its size), and was the entire reason `largo emit rdu` died at two layers while the
    // lowering underneath it handled eight. Measured per phase: lowering 11.6 GB, backend 17.0 GB,
    // reparse 84.7 GB. The block is byte-identical either way; only the detour is gone.
    let summary = CertificateSummary::of(&artifact, program);
    let mut certificate = crate::spatial::lower::rdu::certificate_summary(&artifact.certificate);
    certificate["digest_hex"] = serde_json::json!(summary.digest_hex);
    certificate["rechecked"] = serde_json::json!(summary.rechecked);
    certificate["provenance"] = serde_json::json!(PROVENANCE_MODELED);
    let mut notes = Vec::new();
    let spatial_source = match emit_spatial_program(program, &target, None) {
        Ok(s) => Some(s),
        Err(e) => {
            notes.push(format!(
                "spatial: the Spatial emitter refused {}: {e:?}",
                program.name
            ));
            None
        }
    };
    let pretty =
        |v: &serde_json::Value| format!("{}\n", serde_json::to_string_pretty(v).expect("JSON"));
    let report = serde_json::json!({
        "schema": RDU_EMIT_REPORT_SCHEMA,
        "version": 1,
        "provenance": PROVENANCE_MODELED,
        "no_speed_claim": true,
        "honesty": HONESTY,
        "lane": lane,
        "chip": chip.as_str(),
        "target": target.name,
        "gated": artifact.reason,
        "program": ProgramSummary::of(program, &artifact.census),
        "certificate": summary,
        "ii_report": IiBlock::of(&rdu_ii_report(program, facts), format!("the emitted program {} on the {}", program.name, target.name)),
        // ★ THE COVERAGE BLOCK IS THE HONESTY SURFACE. A bundle that lowered part of a program must SAY
        // so in the artifact a vendor engineer reads, not only in the refusal a caller might discard.
        "coverage": coverage.unwrap_or(serde_json::json!({ "whole_program": true, "statements_total": 1, "statements_lowered": 1, "sections": 1 })),
        "spatial_source": spatial_source.is_some(),
        "notes": notes,
        "chip_facts": ChipBlock::of(facts),
    });
    Ok(RduArtifact {
        chip: chip.as_str(),
        lane,
        neutral_json: artifact.json.clone(),
        placement_txt: artifact.pretty.clone(),
        certificate_json: pretty(&certificate),
        report_json: pretty(&report),
        spatial_source,
        neutral: Some(artifact),
        notes,
    })
}

/// Compile to a **single-file CUDA program** — the LOGOS → CUDA AOT tier. Lifts the
/// C tier's program body ([`compile_to_c`], byte-for-byte ≡ the VM) onto one GPU thread
/// as `__device__` code, over a `printf`-based `Show` preamble, plus a host launcher.
/// Uses the SAME Oracle-optimized bytecode to refuse anything outside the integer/bool
/// scalar fragment (a GPU thread has no dynamic C runtime), returning a `Custom`
/// `ParseError` — a sound refusal, never wrong code. Returns the emitted
/// [`crate::codegen_cuda::CudaArtifact`].
#[cfg(feature = "codegen")]
pub fn compile_to_cuda(source: &str) -> Result<crate::codegen_cuda::CudaArtifact, ParseError> {
    fn custom(msg: String) -> ParseError {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(msg),
            span: crate::token::Span::default(),
        }
    }
    // The C oracle is the proven ≡VM compute body; its program region is lifted to the device. A concurrent
    // program's is the stepped tier's — its tasks as step functions under the scheduler core, which a GPU
    // thread runs as the host does; a program with no tasks lowers to the same C on either tier.
    // `codegen_program_cuda` refuses (typed) any op/value outside the fragment.
    refuse_exact(source, "CUDA tier", "a GPU thread")?;
    let oracle_c = compile_to_c_stepped(source)?;
    crate::ui_bridge::with_parsed_program_checked(source, |parsed, interner| {
        let (stmts, types, _policies) = parsed?;
        let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
        let program = crate::vm::Compiler::compile_with_oracle_located(
            stmts,
            interner,
            Some(types),
            Some(oracle),
        )?;
        crate::codegen_cuda::codegen_program_cuda(&program, &oracle_c)
            .map_err(|e| custom(e.to_string()))
    })
}

/// Emit an **int8 weight-stationary GEMM** CUDA program for `O(m×n) = A(m×k)·W(k×n)` — the DP4A
/// kernel path of the CUDA tier. Uses a deterministic int8 sample ([`crate::codegen_cuda::sample_gemm_data`])
/// baked into the `.cu`; its numerics are anchored to the certified `eval_systolic` reference by
/// `cuda_matrix_lock`. Refuses (typed) if the RDU-shaped target lacks int8 weight-stationary GEMM.
#[cfg(feature = "codegen")]
pub fn compile_to_cuda_gemm(
    m: usize,
    k: usize,
    n: usize,
) -> Result<crate::codegen_cuda::CudaGemmArtifact, ParseError> {
    fn custom(msg: String) -> ParseError {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(msg),
            span: crate::token::Span::default(),
        }
    }
    let (weights, acts) = crate::codegen_cuda::sample_gemm_data(m, k, n);
    crate::codegen_cuda::emit_int8_gemm(m, k, n, &weights, &acts).map_err(|e| custom(e.to_string()))
}

/// Genuinely LOWER an ordinary LOGOS matmul program (a triple-loop `C[i][j] += A[i][k]*B[k][j]`) to a tiled
/// Tensix matrix-engine compute kernel — the compiler *recognizes* the GEMM idiom (affine index analysis)
/// and *generates* the kernel from the program's dimensions, or REFUSES (typed) if the program is not a
/// matmul. This is the honest counterpart to the hand-written template kernels: the kernel is a function of
/// the source, not a canned string. Returns the recognized `(spec, kernel_cpp)`.
pub fn compile_to_tenstorrent_matmul(
    source: &str,
) -> Result<(crate::codegen_tt::matmul_lower::MatmulSpec, String), String> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed?;
        let spec = crate::tensor_recognize::recognize_matmul(stmts, interner)
            .map_err(|r| format!("not a tile-matmul the compiler can lower: {r:?}"))?;
        // The kernel comes from the spatial IR, like every other tensor lowering — see
        // `compile_to_tenstorrent_tensor_op` for why the templates this replaced could not be trusted.
        // The SPEC is still returned from the recognizer: it is what the source said, and callers
        // assert on it directly rather than on the kernel's text.
        let target = crate::spatial::target::SpatialTarget::wormhole();
        let program = crate::spatial::pipeline::compile_to_spatial(source, &target)
            .map_err(|e| format!("not a tile-matmul the compiler can lower: {e:?}"))?;
        let kernel = crate::spatial::lower::tensix::TensixBackend::new(target)
            .compute_kernel_source(&program)
            .map_err(|e| format!("the Tensix printer refused the recognized matmul: {e:?}"))?;
        Ok((spec, kernel))
    })
}

/// Genuinely LOWER an ordinary LOGOS matmul program to an optimized **CUDA** GEMM kernel — the CUDA analog
/// of [`compile_to_tenstorrent_matmul`]. Recognizes the GEMM idiom with the SAME backend-agnostic
/// [`crate::tensor_recognize::recognize_matmul`], extracts `(M,N,K)` from the source, and emits a
/// CUDA program whose dims are the program's (a function of the source, not a canned template); a non-matmul
/// is REFUSED (typed). Returns the recognized `(spec, kernel_cu)`; the emitted program's checksum equals the
/// VM's `Show` byte-for-byte.
#[cfg(feature = "codegen")]
pub fn compile_to_cuda_matmul(
    source: &str,
) -> Result<(crate::codegen_tt::matmul_lower::MatmulSpec, String), String> {
    compile_to_cuda_matmul_with(source, &crate::codegen_cuda::knobs::CudaKnobs::default())
}

/// Knob-driven counterpart of [`compile_to_cuda_matmul`] — recognize the matmul, then lower it under an
/// explicit [`crate::codegen_cuda::knobs::CudaKnobs`] point in the optimization lattice (e.g. the `Int8Dp4a`
/// precision fast path). This is the user-facing surface the CLI/API drive to select a trade-off; a
/// non-matmul is refused (typed) regardless of the knobs.
#[cfg(feature = "codegen")]
pub fn compile_to_cuda_matmul_with(
    source: &str,
    knobs: &crate::codegen_cuda::knobs::CudaKnobs,
) -> Result<(crate::codegen_tt::matmul_lower::MatmulSpec, String), String> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed?;
        let spec = crate::tensor_recognize::recognize_matmul(stmts, interner)
            .map_err(|r| format!("not a matmul the CUDA backend can lower: {r:?}"))?;
        let cu = crate::codegen_cuda::matrix::lower_matmul_to_cuda_with(&spec, knobs);
        Ok((spec, cu))
    })
}

/// Recognize ANY supported tensor op — `matmul` OR elementwise (`plus`/`hadamard`) — in a LOGOS program and
/// lower it to CUDA. The unified accelerator entrypoint: tries the GEMM idiom first, then the elementwise
/// idiom, and REFUSES (typed) only if neither is present. This is what closes the tensor-op × backend matrix
/// to zero gaps on the CUDA cell.
#[cfg(feature = "codegen")]
pub fn compile_to_cuda_tensor_op(source: &str) -> Result<String, String> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed?;
        // ── GUARD: composite blocks must not be claimed by the simple recognizers ──
        // A composite tensor op CONTAINS the simpler idioms. `moe` has a softmax over its router logits;
        // `attention` has a score GEMV. The recognizers below match those FRAGMENTS, so without this guard a
        // Mixture-of-Experts program compiled successfully to a bare `fused_row` softmax kernel over the
        // router's E logits — a silent miscompile that reported success and computed almost none of the block.
        // `recognize_stages` avoids this by trying the composite recognizers first; this entry point had no
        // such ordering. Fail closed: composites lower through the forward path
        // (`compile_to_cuda_forward_q8_moe*`), which emits the grouped-expert / attention kernels.
        if crate::codegen_cuda::qwen_emit::recognize_moe(stmts).is_some() {
            return Err("`moe` is a composite block — lower it with compile_to_cuda_forward_q8_moe*, not the single-op path".to_string());
        }
        if crate::codegen_cuda::qwen_emit::recognize_attention(stmts).is_some() {
            return Err("`attention` is a composite block — lower it through the forward path, not the single-op path".to_string());
        }
        if let Ok(spec) = crate::tensor_recognize::recognize_matmul(stmts, interner) {
            return Ok(crate::codegen_cuda::matrix::lower_matmul_to_cuda(&spec));
        }
        if let Ok(spec) = crate::tensor_recognize::recognize_elementwise(stmts, interner) {
            return Ok(crate::codegen_cuda::matrix::lower_elementwise_to_cuda(
                &spec,
            ));
        }
        if let Ok(spec) = crate::tensor_recognize::recognize_swiglu(stmts, interner) {
            return Ok(crate::codegen_cuda::fused_row::emit_swiglu(
                &spec,
                &crate::codegen_cuda::knobs::CudaKnobs::default(),
            ));
        }
        if let Ok(spec) = crate::tensor_recognize::recognize_fused_row(stmts, interner) {
            return Ok(crate::codegen_cuda::fused_row::emit_fused_row(
                &spec,
                &crate::codegen_cuda::knobs::CudaKnobs::default(),
            ));
        }
        Err("not a tensor op the CUDA backend can lower (matmul / elementwise / reduction / swiglu)".to_string())
    })
}

/// Compile a LOGOS fused-row program straight into a **SOL-ExecBench** submission — recognize the op and emit
/// the `solution.json` + torch-binding CUDA kernel the benchmark evaluates on the B200. `definition` is the
/// problem's Definition name; `inputs`/`output` are the tensor names in Definition order. This closes the loop
/// from English LOGOS to a submittable, compiler-generated kernel.
#[cfg(feature = "codegen")]
pub fn compile_to_sol_solution(
    source: &str,
    definition: &str,
    inputs: &[&str],
    output: &str,
) -> Result<crate::codegen_cuda::SolSolution, String> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed?;
        let spec = crate::tensor_recognize::recognize_fused_row(stmts, interner)
            .map_err(|r| format!("not a fused-row op the SOL-ExecBench bridge can lower: {r:?}"))?;
        Ok(crate::codegen_cuda::emit_fused_row_solution(
            &spec,
            &crate::codegen_cuda::knobs::CudaKnobs::default(),
            definition,
            inputs,
            output,
        ))
    })
}

/// Recognize the ordered tensor-op stages of a MULTI-STAGE forward and lower it to ONE CUDA program that
/// chains their parallel kernels through shared device buffers — the whole-forward orchestrator
/// ([`crate::codegen_cuda::qwen_emit`]). Where [`compile_to_cuda_tensor_op`] emits ONE op's kernel, this
/// composes a sequence (`rmsnorm → matmul → softmax → …`). Refuses (typed) if no stage is recognized.
#[cfg(feature = "codegen")]
pub fn compile_to_cuda_forward(source: &str) -> Result<String, String> {
    compile_to_cuda_forward_weighted(source, &std::collections::HashMap::new())
}

/// [`compile_to_cuda_forward`] with a WEIGHT MANIFEST: `var name → (file path, float count)`. Any tensor
/// variable named in the manifest is loaded from that file by the emitted host at runtime instead of being
/// inlined — the mechanism the real-8B forward needs, since the Qwen weights are gigabytes (extracted from the
/// GGUF checkpoint). Every other input still uses its literal `Set` values / the deterministic fill.
#[cfg(feature = "codegen")]
pub fn compile_to_cuda_forward_weighted(
    source: &str,
    weights: &std::collections::HashMap<String, (String, usize)>,
) -> Result<String, String> {
    compile_to_cuda_forward_q8(source, weights, &std::collections::HashMap::new())
}

/// [`compile_to_cuda_forward_weighted`] with an INT8 (Q8_0) weight manifest: any tensor variable named in `q8`
/// is loaded resident as int8 quants + f32 block scales, and a matmul against it becomes a dequant-in-kernel
/// GEMV. This is what fits real 8B weights on a 12 GB card (8 GB int8, not 32 GB f32) and the DP4A path that
/// beats llama.cpp — see [`crate::codegen_cuda::qwen_emit::Q8Ref`].
#[cfg(feature = "codegen")]
pub fn compile_to_cuda_forward_q8(
    source: &str,
    weights: &std::collections::HashMap<String, (String, usize)>,
    q8: &std::collections::HashMap<String, crate::codegen_cuda::qwen_emit::Q8Ref>,
) -> Result<String, String> {
    compile_to_cuda_forward_q8_impl(
        source,
        weights,
        q8,
        &std::collections::HashMap::new(),
        crate::codegen_cuda::qwen_emit::Q8MatKind::Dequant,
    )
}

/// [`compile_to_cuda_forward_q8`] emitting the INT8-**activation** DP4A MMQ kernel for each Q8 matmul: the
/// activation is quantized to int8 per 32-block and `__dp4a`'d against the int8 weight (int32 accumulate) —
/// the same kernel shape llama.cpp's fast path uses, and the shipped batch-1 default. Numerically the
/// int8-activation approximation of the dequant GEMV (small quantization error), not bit-identical; the
/// `TwoPassInt8` mode recovers ~14-bit activation exactness at two `__dp4a` per block.
#[cfg(feature = "codegen")]
pub fn compile_to_cuda_forward_q8_dp4a(
    source: &str,
    weights: &std::collections::HashMap<String, (String, usize)>,
    q8: &std::collections::HashMap<String, crate::codegen_cuda::qwen_emit::Q8Ref>,
) -> Result<String, String> {
    compile_to_cuda_forward_q8_impl(
        source,
        weights,
        q8,
        &std::collections::HashMap::new(),
        crate::codegen_cuda::qwen_emit::Q8MatKind::Int8Act,
    )
}

#[cfg(feature = "codegen")]
fn compile_to_cuda_forward_q8_impl(
    source: &str,
    weights: &std::collections::HashMap<String, (String, usize)>,
    q8: &std::collections::HashMap<String, crate::codegen_cuda::qwen_emit::Q8Ref>,
    moe: &std::collections::HashMap<String, crate::codegen_cuda::qwen_emit::MoeWeights>,
    kind: crate::codegen_cuda::qwen_emit::Q8MatKind,
) -> Result<String, String> {
    compile_to_cuda_forward_holding(source, weights, q8, moe, kind, &[])
}

/// The older forward of `source` at `kind`'s numerics with the optimizer's steps `held` held back: each kernel it
/// would fuse emitted apart, the program the fused one is held equal to.
#[cfg(feature = "codegen")]
pub fn compile_to_cuda_forward_holding(
    source: &str,
    weights: &std::collections::HashMap<String, (String, usize)>,
    q8: &std::collections::HashMap<String, crate::codegen_cuda::qwen_emit::Q8Ref>,
    moe: &std::collections::HashMap<String, crate::codegen_cuda::qwen_emit::MoeWeights>,
    kind: crate::codegen_cuda::qwen_emit::Q8MatKind,
    held: &[&str],
) -> Result<String, String> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed?;
        let graph = crate::codegen_cuda::qwen_emit::forward_graph_holding(stmts, interner, q8, moe, kind, held)
            .map_err(|e| e.to_string())?;
        let inits = crate::codegen_cuda::qwen_emit::collect_inits(stmts, interner);
        crate::codegen_cuda::qwen_emit::emit_forward(
            &graph, &inits, weights, q8, moe, kind, interner,
        )
        .map_err(|e| e.to_string())
    })
}

/// [`compile_to_cuda_forward_q8`] with a **MoE weight manifest**: any `moe` op in the source lowers to
/// grouped-expert CUDA (router GEMV → top-K route → per-expert SwiGLU → weighted combine), its router/gate/up/
/// down weights loaded int8-resident from the files named in `moe` (keyed by the block's output variable). This
/// is the Qwen3-30B-A3B FFN — the SAME emitter as the dense 8B forward, plus the MoE stage.
#[cfg(feature = "codegen")]
pub fn compile_to_cuda_forward_q8_moe(
    source: &str,
    weights: &std::collections::HashMap<String, (String, usize)>,
    q8: &std::collections::HashMap<String, crate::codegen_cuda::qwen_emit::Q8Ref>,
    moe: &std::collections::HashMap<String, crate::codegen_cuda::qwen_emit::MoeWeights>,
) -> Result<String, String> {
    compile_to_cuda_forward_q8_impl(
        source,
        weights,
        q8,
        moe,
        crate::codegen_cuda::qwen_emit::Q8MatKind::Dequant,
    )
}

/// [`compile_to_cuda_forward_q8_moe`] emitting the **sparse top-K, deeply-fused, coalesced int8-activation**
/// MoE path — THE SHIPPED DEFAULT: only the K selected experts' weights are gathered per token (the "A3B = 3B
/// active" sparsity), and the FFN collapses to three kernels — router+topk, gate+up+SwiGLU fused, down+combine
/// fused — so every intermediate stays off HBM. One `__dp4a` per 32-block (single-pass int8 activation) is the
/// shortest dependency chain, the fastest batch-1 path at llama.cpp-matched activation precision (the A100
/// head-to-head beat: identical greedy token, more tok/s). For logits within ~1e-3 of the f64 reference — the
/// correctness mode llama.cpp does not have — use [`compile_to_cuda_forward_q8_moe_exact`].
#[cfg(feature = "codegen")]
pub fn compile_to_cuda_forward_q8_moe_dp4a(
    source: &str,
    weights: &std::collections::HashMap<String, (String, usize)>,
    q8: &std::collections::HashMap<String, crate::codegen_cuda::qwen_emit::Q8Ref>,
    moe: &std::collections::HashMap<String, crate::codegen_cuda::qwen_emit::MoeWeights>,
) -> Result<String, String> {
    compile_to_cuda_forward_q8_impl(
        source,
        weights,
        q8,
        moe,
        crate::codegen_cuda::qwen_emit::Q8MatKind::Int8Act,
    )
}

/// [`compile_to_cuda_forward_q8_moe_dp4a`]'s LOGIT-EXACT sibling: the same sparse top-K deeply-fused kernel
/// shape, but the activation is quantized TWO-PASS (hi byte + lo residual byte ⇒ ~14-bit) and every block runs
/// two `__dp4a` (hi and lo) — ~2× the activation math for logits within ~1e-3 of the f64 reference, so a
/// near-tie greedy token does NOT flip (single int8 activation flips it — llama.cpp included). Weights are
/// still read once, so the memory-bound cost is unchanged; the measured price of exactness is ~6% tok/s.
#[cfg(feature = "codegen")]
pub fn compile_to_cuda_forward_q8_moe_exact(
    source: &str,
    weights: &std::collections::HashMap<String, (String, usize)>,
    q8: &std::collections::HashMap<String, crate::codegen_cuda::qwen_emit::Q8Ref>,
    moe: &std::collections::HashMap<String, crate::codegen_cuda::qwen_emit::MoeWeights>,
) -> Result<String, String> {
    compile_to_cuda_forward_q8_impl(
        source,
        weights,
        q8,
        moe,
        crate::codegen_cuda::qwen_emit::Q8MatKind::TwoPassInt8,
    )
}

/// The Tenstorrent analog of [`compile_to_cuda_tensor_op`]: recognize `matmul` OR elementwise and lower to a
/// Tensix compute kernel, refusing (typed) if neither is present. Closes the Tenstorrent cell of the matrix.
#[cfg(feature = "codegen")]
pub fn compile_to_tenstorrent_tensor_op(source: &str) -> Result<String, String> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed?;
        // ★ THROUGH THE SPATIAL IR, NOT THE TEMPLATES — AND THIS IS A CORRECTNESS FIX, NOT A TIDY-UP.
        //
        // The four `lower_*_to_tensix` templates this replaced were not all real. `lower_fused_row_to_tensix`
        // declared `cb_w`, `cb_res` and `cb_sq`, included `sqrt.h`, and promised
        // `Y = (X+R)/sqrt(mean+eps)·W[i]` in its header comment — while its body did one `reduce_tile` and a
        // `pack_tile`. No sqrt, no rsqrt, no multiply, no weight, no residual. One of its format arguments
        // was literally `if residual { "" } else { "" }`. Nothing in the tree called it except this function,
        // and no test checked its NUMBERS — the row-op locks assert kernel text, so a kernel that computed
        // the wrong thing passed them for as long as it contained the word `reduce_tile`.
        //
        // The IR path is checked the other way round: `spatial_algebra_semantics_lock` compares it against an
        // independent statement of the mathematics, and `spatial_tensix_p4_ttsim_lock` runs it on
        // Tenstorrent's own simulator against a golden. Six workloads (matmul, elementwise, swiglu, rmsnorm,
        // softmax, layernorm) lower this way, from ONE general compute form.
        let _ = (stmts, interner);
        let target = crate::spatial::target::SpatialTarget::wormhole();
        let program = crate::spatial::pipeline::compile_to_spatial(source, &target).map_err(|e| {
            format!("not a tensor op the Tenstorrent backend can lower (matmul / elementwise / reduction / swiglu): {e:?}")
        })?;
        // Kernels only: this returns a KERNEL, and the host's golden would additionally demand fills the
        // compiler can reproduce — a requirement that belongs to a runnable bundle, not to a kernel.
        crate::spatial::lower::tensix::TensixBackend::new(target)
            .compute_kernel_source(&program)
            .map_err(|e| format!("the Tensix printer refused the recognized tensor op: {e:?}"))
    })
}

/// Lower a recognized concurrency pattern's value stream to a spatial program: the sends become the input
/// tensor's init, the fold becomes a reduction, and the Tensix printer emits the kernels. This is the one
/// place the concurrency tier turns a recognized pattern into a PROGRAM — every pattern below reduces to a
/// stream of values and a fold over it, which is why none of them needs an emitter of its own any more.
#[cfg(feature = "codegen")]
fn mesh_program(values: &[i64]) -> Result<crate::spatial::ir::SpatialProgram, String> {
    let target = crate::spatial::target::SpatialTarget::wormhole();
    let tpl = crate::spatial::passes::from_tensor_op::from_pipe(values, &target)
        .map_err(|e| format!("the pattern's value stream does not lower: {e:?}"))?;
    crate::spatial::pipeline::run_passes(tpl, &target)
        .map_err(|e| format!("the lowered pattern did not survive the passes: {e:?}"))
}

/// Recognize a producer/consumer pipe program and lower its CONCURRENCY onto the Tensix mesh — the task on
/// one baby RISC-V, main on the other, the pipe as a circular buffer. Returns the recognized spec + the
/// complete 2-core program, or an error if the program is not the recognized pattern (the caller then falls
/// back to the sound single-core refusal). The mesh output is ≡ the VM's deterministic interleaving.
pub fn compile_to_tenstorrent_concurrency(
    source: &str,
) -> Result<
    crate::codegen_tt::concurrency_mesh::MeshLowering<
        crate::codegen_tt::concurrency_mesh::ConcurrencySpec,
    >,
    String,
> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed?;
        let spec =
            crate::codegen_tt::concurrency_mesh::recognize_producer_consumer(stmts, interner)
                .ok_or_else(|| {
                    "not a producer/consumer pipe program the mesh tier can lower".to_string()
                })?;
        let program = Some(mesh_program(&spec.sends)?);
        Ok(crate::codegen_tt::concurrency_mesh::MeshLowering { spec, program })
    })
}

/// Recognize a **fan-out** program (N distinct tasks each send one value into a shared pipe; main folds all
/// N) and lower its concurrency onto the Tensix mesh — one core per task, main folding via a NoC + semaphore
/// barrier. The commutative fold makes the mesh's arrival order irrelevant, so it is ≡VM. Errors if the
/// program is not the fan-out pattern (the caller falls back to the sound refusal).
pub fn compile_to_tenstorrent_fanout(
    source: &str,
) -> Result<
    crate::codegen_tt::concurrency_mesh::MeshLowering<
        crate::codegen_tt::concurrency_mesh::FanoutSpec,
    >,
    String,
> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed?;
        let spec = crate::codegen_tt::concurrency_mesh::recognize_fanout(stmts, interner)
            .ok_or_else(|| "not a fan-out program the mesh tier can lower".to_string())?;
        let program = Some(mesh_program(&spec.values)?);
        Ok(crate::codegen_tt::concurrency_mesh::MeshLowering { spec, program })
    })
}

/// Recognize a **select** (`Await the first of { Receive … / After … }`) on a ready channel and lower it to
/// a single-core program whose non-blocking readiness check reproduces the VM's deterministic arm choice.
pub fn compile_to_tenstorrent_select(
    source: &str,
) -> Result<
    crate::codegen_tt::concurrency_mesh::MeshLowering<
        crate::codegen_tt::concurrency_mesh::SelectSpec,
    >,
    String,
> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed?;
        let spec = crate::codegen_tt::concurrency_mesh::recognize_select(stmts, interner)
            .ok_or_else(|| "not a select program the mesh tier can lower".to_string())?;
        // The channel is READY (main pre-sends before the await), so the VM's deterministic select takes
        // the receive arm — and the compiler resolves it the same way rather than emitting a device-side
        // readiness check whose answer is already a literal of the source.
        let program = Some(mesh_program(&[spec.presend])?);
        Ok(crate::codegen_tt::concurrency_mesh::MeshLowering { spec, program })
    })
}

/// Recognize a **racy select** (a task sleeps then sends; main awaits a receive-or-timeout) and lower it to
/// a coordinator + task program. The coordinator compares the reported send virtual-time against the
/// timeout — the VM's deterministic scheduler rule — so the outcome is ≡VM on both branches, independent of
/// real hardware timing.
pub fn compile_to_tenstorrent_racy_select(
    source: &str,
) -> Result<
    crate::codegen_tt::concurrency_mesh::MeshLowering<
        crate::codegen_tt::concurrency_mesh::RacySelectSpec,
    >,
    String,
> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed?;
        let spec = crate::codegen_tt::concurrency_mesh::recognize_racy_select(stmts, interner)
            .ok_or_else(|| "not a racy select program the mesh tier can lower".to_string())?;
        // NOT a real-time race: the VM fires whichever event has the smaller VIRTUAL time, and both
        // times are literals of the source. When the send wins, the program is a one-value pipe; when the
        // timeout wins the answer is a constant string, which has no dataflow to place.
        let program = if spec.send_vtime_ms < spec.timeout_ms {
            Some(mesh_program(&[spec.value])?)
        } else {
            None
        };
        Ok(crate::codegen_tt::concurrency_mesh::MeshLowering { spec, program })
    })
}

/// Recognize a **dynamic-channel** program (a loop-driven producer feeding the pipe with computed values)
/// and lower it to a 2-core program: the producer loops pushing `f(i)` onto a bounded circular buffer; the
/// consumer folds the stream. Order-deterministic FIFO ⇒ ≡VM.
pub fn compile_to_tenstorrent_dynamic(
    source: &str,
) -> Result<
    crate::codegen_tt::concurrency_mesh::MeshLowering<
        crate::codegen_tt::concurrency_mesh::DynamicSpec,
    >,
    String,
> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed?;
        let spec = crate::codegen_tt::concurrency_mesh::recognize_dynamic_producer(stmts, interner)
            .ok_or_else(|| "not a dynamic loop producer the mesh tier can lower".to_string())?;
        let program = Some(mesh_program(&spec.values)?);
        Ok(crate::codegen_tt::concurrency_mesh::MeshLowering { spec, program })
    })
}

/// Recognize a **multi-task race** (≥2 tasks each sleep then send; main awaits receive-or-timeout) and lower
/// it to a coordinator + N-task program. The coordinator collects every task's `(value, send-vtime)`, picks
/// the global-minimum send-vtime, and compares it to the timeout — the VM's deterministic winner. ≡VM.
pub fn compile_to_tenstorrent_multi_racy(
    source: &str,
) -> Result<
    crate::codegen_tt::concurrency_mesh::MeshLowering<
        crate::codegen_tt::concurrency_mesh::RacyMultiSpec,
    >,
    String,
> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed?;
        let spec = crate::codegen_tt::concurrency_mesh::recognize_multi_racy(stmts, interner)
            .ok_or_else(|| "not a multi-task racy select the mesh tier can lower".to_string())?;
        // The winner is the globally smallest send virtual time, and it only fires if it beats the
        // timeout — the same rule the VM's scheduler applies, evaluated here because every term is a
        // literal of the source.
        let winner = spec.tasks.iter().min_by_key(|(_, vt)| *vt).copied();
        let program = match winner {
            Some((value, vt)) if vt < spec.timeout_ms => Some(mesh_program(&[value])?),
            _ => None,
        };
        Ok(crate::codegen_tt::concurrency_mesh::MeshLowering { spec, program })
    })
}

pub fn compile_to_wasm(source: &str) -> Result<Vec<u8>, ParseError> {
    compile_to_wasm_with(source, &crate::vm::reach::HostEntries::default())
}

/// [`compile_to_wasm`] for a host that enters the module by name beyond `main`: the components a
/// test mounts at a statement of its own ([`crate::vm::reach::HostEntries`]). A shipped page's
/// entries are the default, and its module is exactly the one [`compile_to_wasm`] writes.
pub fn compile_to_wasm_with(
    source: &str,
    entries: &crate::vm::reach::HostEntries,
) -> Result<Vec<u8>, ParseError> {
    compile_to_wasm_module(source, entries, false)
}

/// [`compile_to_wasm`] as a check-mode module: the same program through the same pipeline, with the
/// heap census exported ([`crate::vm::wasm::assemble_program_checked`]) for a heap test to read
/// every true count back through. A shipped page never carries it.
pub fn compile_to_wasm_checked(source: &str) -> Result<Vec<u8>, ParseError> {
    compile_to_wasm_module(source, &crate::vm::reach::HostEntries::default(), true)
}

fn compile_to_wasm_module(
    source: &str,
    entries: &crate::vm::reach::HostEntries,
    checked: bool,
) -> Result<Vec<u8>, ParseError> {
    fn custom(msg: String) -> ParseError {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(msg),
            span: crate::token::Span::default(),
        }
    }
    // The module lowers concrete code only: the program arrives monomorphic (the VM's own `Option`
    // stays built in).
    let concrete = crate::monomorphize::Options {
        lower_option: false,
    };
    crate::ui_bridge::with_monomorphic_program_checked(source, concrete, |parsed, interner| {
        let (stmts, types, policies) = parsed?;
        // Compile with the SAME Oracle the VM/native tier use (`run_to_outcome`), so the AOT receives
        // the optimized bytecode — DivPow2 / MagicDivU (magic-reciprocal division) and the Oracle-proven
        // bounds-check-elimination forms (IndexUnchecked / SetIndexUnchecked / RegionBoundsGuard). The
        // AOT lowers each (division bit-exact to the VM's `magic_eval`; the unchecked forms keep the
        // bounds check, a safe superset), so WASM == VM stays exact while the two share one bytecode.
        let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
        // Located: a refusal of the author's construct reaches the caller AT that construct.
        let mut program = crate::vm::Compiler::compile_with_oracle_located(
            stmts,
            interner,
            Some(types),
            Some(oracle),
        )?;
        // Coalesce register-move cascades before lowering — the direct-WASM backend emits final code
        // with no downstream optimizing assembler. Semantics-preserving (peephole_soundness_lock ≡ VM).
        crate::vm::peephole::peephole(&mut program);
        refuse_unlinkable_with(&program, interner, Backend::WebAssembly, entries)
            .map_err(custom)?;
        let assemble = if checked {
            crate::vm::wasm::assemble_program_checked
        } else {
            crate::vm::wasm::assemble_program_with
        };
        assemble(&program, &policies, interner, entries).map_err(|e| custom(e.to_string()))
    })
}

/// Compile to WebAssembly in the **OBSERVED profile** (Wave 13): `Note`/`Warn`/
/// `Record`/`During` lower to `Op::Obs`, which the WASM backend emits as calls to
/// the host `obs(site_id)` import — the browser-tracing analog of the VM's
/// observation emission. Returns the module bytes AND the `observation_sites`
/// table: the host (which compiled the program) resolves each `site_id` to its
/// category / message / span kind. The default [`compile_to_wasm`] stays the
/// elided profile, byte-identical to the observation-free program (§0.7).
pub fn compile_to_wasm_observed(
    source: &str,
) -> Result<(Vec<u8>, Vec<crate::vm::ObsSiteDesc>), ParseError> {
    fn custom(msg: String) -> ParseError {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(msg),
            span: crate::token::Span::default(),
        }
    }
    let concrete = crate::monomorphize::Options {
        lower_option: false,
    };
    crate::ui_bridge::with_monomorphic_program(source, concrete, |parsed, interner| {
        let (stmts, types, policies) = parsed.map_err(custom)?;
        let mut program =
            crate::vm::Compiler::compile_observed_located(stmts, interner, Some(types))?;
        let sites = program.observation_sites.clone();
        // The observed profile keeps `Op::Obs` (its args stay live; `site` indexes the untouched
        // observation_sites table), so the peephole only coalesces the move cascades around them.
        crate::vm::peephole::peephole(&mut program);
        refuse_unlinkable(&program, interner, Backend::WebAssembly).map_err(custom)?;
        let wasm = crate::vm::wasm::assemble_program(&program, &policies, interner)
            .map_err(|e| custom(e.to_string()))?;
        Ok((wasm, sites))
    })
}

/// Compile LOGOS source to a LINKED WebAssembly module that uses the real `logicaffeine_base::BigInt`
/// runtime for arbitrary-precision integer arithmetic: an overflowing integer expression
/// (`Show 99999999999 * 99999999999`, `Show 2 to the power of 200`) computes the EXACT big number —
/// matching the VM's promote-on-overflow — instead of trapping, and `Show` prints its decimal.
///
/// Unlike [`compile_to_wasm`] (fully self-contained, no toolchain), this is TOOLCHAIN-DEPENDENT: the
/// BigInt-aware emitter (`crate::vm::wasm::assemble_program_linked`) produces a RELOCATABLE object that
/// is linked against `logicaffeine_base` (compiled once to `wasm32-unknown-unknown`) with the Rust
/// toolchain's `rust-lld`. The runtime object is cached per process, so the amortized cost is one link.
/// Returns the bytes of a `.wasm` file, or a [`ParseError`] carrying the reason the backend or linker
/// declined (an unsupported program shape, or an unavailable toolchain / base wasm32 build).
#[cfg(all(feature = "wasm-jit", not(target_arch = "wasm32")))]
pub fn compile_to_wasm_linked(source: &str) -> Result<Vec<u8>, ParseError> {
    let relocatable = compile_to_wasm_relocatable(source)?;
    crate::vm::wasm::link_relocatable_bigint(&relocatable).map_err(|e| ParseError {
        kind: crate::error::ParseErrorKind::Custom(e.to_string()),
        span: crate::token::Span::default(),
    })
}

/// The RELOCATABLE object [`compile_to_wasm_linked`] hands the linker: the program monomorphized,
/// compiled with the optimizer's oracle, peephole-coalesced, checked linkable, lowered by the
/// BigInt-aware emitter (`crate::vm::wasm::assemble_program_linked`) and carried with its
/// relocations — not yet linked, so a caller can link it against a runtime of its own.
#[cfg(all(feature = "wasm-jit", not(target_arch = "wasm32")))]
pub fn compile_to_wasm_relocatable(source: &str) -> Result<Vec<u8>, ParseError> {
    fn custom(msg: String) -> ParseError {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(msg),
            span: crate::token::Span::default(),
        }
    }
    let concrete = crate::monomorphize::Options {
        lower_option: false,
    };
    crate::ui_bridge::with_monomorphic_program(
        source,
        concrete,
        |parsed, interner| -> Result<Vec<u8>, ParseError> {
            let (stmts, types, policies) = parsed.map_err(custom)?;
            let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
            let mut program = crate::vm::Compiler::compile_with_oracle_located(
                stmts,
                interner,
                Some(types),
                Some(oracle),
            )?;
            // Same coalescing before the BigInt-linked WASM lowering (no downstream optimizer). ≡ VM.
            crate::vm::peephole::peephole(&mut program);
            refuse_unlinkable(&program, interner, Backend::WebAssembly).map_err(custom)?;
            let module = crate::vm::wasm::assemble_program_linked(&program, &policies, interner)
                .map_err(|e| custom(e.to_string()))?;
            crate::vm::wasm::module_to_relocatable(&module).map_err(|e| custom(e.to_string()))
        },
    )
}

/// Format an `f64` exactly as a `Show` of a `Float` would — i.e. byte-identical to the
/// tree-walker / VM (`RuntimeValue::to_display_string`). The direct-WASM host's `print_f64`
/// sink uses this so a `Show` of a float matches every other engine. Delegating to the real
/// formatter (rather than re-deriving it) makes drift impossible.
pub fn display_float_like_logos(f: f64) -> String {
    crate::interpreter::RuntimeValue::Float(f).to_display_string()
}

/// Classify the determinacy of a LOGOS program.
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
    let _compile_facts = crate::optimize::facts::scope();
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

    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
    );
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
    let _compile_facts = crate::optimize::facts::scope();
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

    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
    );
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
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
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

/// Run the Send/escape analysis over a program.
///
/// Parses `source` and returns the Send/escape diagnostics (empty = the program
/// respects the message-passing + CRDT discipline). Pure analysis — no codegen.
pub fn send_check_source(
    source: &str,
) -> Result<Vec<crate::concurrency::SendDiagnostic>, ParseError> {
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
    let _compile_facts = crate::optimize::facts::scope();
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

    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
    );
    let stmts = parser.parse_program()?;
    Ok(crate::concurrency::check_send_escape(&stmts))
}

/// [`compile_program_full`] for **deterministic replay** — the full
/// output (rust + dependencies) with every scheduler operation lowered onto
/// `logicaffeine_system::replay`. The seeded differential harness uses this so it
/// gets the same dependency set as the default build.
pub fn compile_program_full_deterministic(source: &str) -> Result<CompileOutput, ParseError> {
    crate::codegen::with_deterministic_replay(|| compile_program_full(source))
}

/// Refuse the first type the parsed program names that nothing supplies
/// (`analysis::check::first_undeclared_type`), located at the top-level statement that names it —
/// `spans` are the parser's statement spans, aligned with `parsed` as the parser returned it; a
/// declaration's field type has no statement, and the refusal names the type alone.
fn refuse_undeclared_types(
    parsed: &[Stmt],
    spans: &[crate::token::Span],
    registry: &crate::analysis::TypeRegistry,
    interner: &Interner,
) -> Result<(), ParseError> {
    match crate::analysis::check::first_undeclared_type(parsed, registry, interner) {
        Some((index, name)) => Err(ParseError {
            kind: crate::error::ParseErrorKind::Custom(format!(
                "`{}` is not a type this program declares or imports",
                interner.resolve(name)
            )),
            span: index
                .and_then(|i| spans.get(i))
                .copied()
                .unwrap_or_default(),
        }),
        None => Ok(()),
    }
}

/// Compile LOGOS source and return full output including dependency metadata.
///
/// This is the primary compilation entry point that returns both the generated
/// Rust code and any crate dependencies declared in `## Requires` blocks.
pub fn compile_program_full(source: &str) -> Result<CompileOutput, ParseError> {
    // Mixed-document aware: a source interleaving imperative code with Coq-style math
    // (`Definition`/`## Theorem:`/…) routes the math to the Forge and bundles it as
    // `mod proven`. Pure imperative programs partition to `(source, None)` — a no-op.
    let (imperative_src, math_src) = crate::ui_bridge::partition_mixed(source);
    let proven = match math_src.as_deref() {
        Some(math) => crate::ui_bridge::mixed_proven_module(math)?,
        None => None,
    };
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
pub fn compile_program_full_with_proven(
    source: &str,
    proven: Option<&str>,
) -> Result<CompileOutput, ParseError> {
    // Phase 10: auto-prepend the stdlib modules the program references (no-op when
    // it uses no stdlib vocabulary, so the benchmark corpus stays byte-identical).
    let prelude_src = crate::loader::apply_prelude(source);
    // Drop the DEAD prelude functions the whole-module splice dragged in but the program never
    // reaches (e.g. a postgres program pulls all of crypto.lg for md5/sha256/pbkdf2, but never the
    // ML-DSA signature functions). This never touches a user function and is byte-identical when
    // nothing is dead, so live programs are unchanged; it removes stdlib code the AOT-Rust backend
    // would otherwise have to compile — including any latent codegen gap it is never reached through.
    let pruned = crate::loader::prune_unreachable_prelude_from(source, prelude_src.as_ref());
    let source = pruned.as_str();
    let library = crate::loader::LibraryProvenance::of(source);

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    // Pass 1: Discovery - scan for type definitions and policies
    let (type_registry, policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };
    let codegen_policies = policy_registry.clone();

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let _compile_facts = crate::optimize::facts::scope();
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
    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
    );
    // Note: Don't call process_block_headers() - parse_program handles blocks itself

    let stmts = parser.parse_program()?;
    let (parsed, parsed_spans) = (stmts.clone(), parser.stmt_spans().to_vec());

    // §0.7 — in the ELIDED (release) profile, observation is ERASED at lowering,
    // before the optimizer and codegen: Note/Warn/Record vanish and During spans unwrap to
    // their bodies, so observation cannot perturb one instruction of the emitted
    // code (not even through the optimizer's overflow/range analysis). No-op for
    // programs that observe nothing. Locked by `obs_zero_cost_codegen_lock`. In
    // the OBSERVED profile the statements survive and the AOT backend emits
    // `logicaffeine_system::obs` calls for them (Wave 13).
    let stmts: Vec<Stmt> = if crate::codegen::observed_codegen_enabled() {
        stmts
    } else {
        match crate::elide_observation::elide_observation(&stmts, &stmt_arena) {
            Some(elided) => elided.to_vec(),
            None => stmts,
        }
    };

    // The ONE optimization config for this compile: env baseline merged with the
    // file-level `## No <X>` decorators (program-wide), normalized. Threaded to
    // BOTH the optimizer and codegen so they never disagree. Captured here (the
    // last use of `parser`) so its `&mut interner` borrow ends before reuse.
    // `## No` / `## Require` decorators via the shared gate: merge `## No` into the active
    // config (so the optimizer AND codegen honor it), and open a firing trace so an unfired
    // `## Require` (Ray Myers' wish) fails the compile. Captured here (last use of `parser`)
    // so its `&mut interner` borrow ends before reuse; verified after codegen.
    let (opt_config, gate) = crate::optimize::install_decorators(
        crate::optimization::OptimizationConfig::from_env(),
        &parser.program_opt_flags(),
        parser.required_optimizations(),
    );
    let codegen_registry = parser.into_type_registry();
    refuse_undeclared_types(&parsed, &parsed_spans, &codegen_registry, &interner)?;

    // Escape analysis reads the program as written, before observation was elided.
    check_as_written(&parsed, &interner, AsWritten::Escapes)?;

    // Type-directed division: rewrite `Divide → ExactDivide` in every Rational context
    // (the integer default stays floor), so a `Let x: Rational be 7 / 2` compiles exact
    // (`7/2`) instead of flooring to `3` — matching the tree-walker and VM. The optimizer
    // treats `ExactDivide` as opaque (never floor-folds it), so this is purely additive.
    let resolved = crate::resolve_program::resolve_program(
        &stmts,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
        &mut interner,
        opt_config.is_on(crate::optimization::Opt::Comptime),
        library.functions(),
    );
    let stmts: Vec<Stmt> = match resolved {
        Some(rw) => rw.to_vec(),
        None => stmts,
    };
    let (stmts, codegen_registry) = lift_program_bindings(
        stmts,
        codegen_registry,
        &mut interner,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
    )?;

    // Pass 2.5: Optimization - constant folding and dead code elimination
    let stmts = crate::optimize::optimize_program(
        stmts,
        &imperative_expr_arena,
        &stmt_arena,
        &mut interner,
        &opt_config,
    );

    // Extract dependencies
    let mut dependencies = extract_dependencies(&stmts, &interner)?;

    // FFI: Auto-inject wasm-bindgen dependency if any function is exported for WASM
    let needs_wasm_bindgen = stmts.iter().any(|stmt| {
        if let Stmt::FunctionDef {
            is_exported: true,
            export_target: Some(target),
            ..
        } = stmt
        {
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

    // Note: Static verification is available when the `verification` feature is enabled,
    // but must be explicitly invoked via compile_to_rust_verified().

    // A call to a name that exists nowhere in the post-prelude program is a
    // check error here — not a rustc E0425 after codegen. The bundled proven
    // module's `pub fn`s are in scope via `use proven::*;`, so they count.
    let proven_exports = proven
        .map(crate::analysis::proven_module_exports)
        .unwrap_or_default();
    crate::analysis::check_unknown_calls_with_exports(
        &stmts,
        &interner,
        &codegen_registry,
        &proven_exports,
    )
    .map_err(|e| ParseError {
        kind: e.to_parse_error_kind(&interner),
        span: crate::token::Span::default(),
    })?;

    let type_env = crate::analysis::check_program_located(&stmts, &interner, &codegen_registry)
        .map_err(|e| ParseError {
            kind: e.to_parse_error_kind(&interner, &stmts),
            span: crate::token::Span::default(),
        })?;
    let rust_code = crate::optimize::with_backend_lowering(|| {
        crate::codegen::codegen_program_with_proven(
            &stmts,
            &codegen_registry,
            &codegen_policies,
            &interner,
            &type_env,
            &opt_config,
            "proven",
            proven,
        )
    });

    // `## Require <opt>`: every demanded optimization must have fired for this
    // program, or the compile fails loudly — the inverse of `## No`, and strictly
    // more general than Zig's tail-call-only `@call(.always_tail)`.
    gate.verify().map_err(|m| ParseError {
        kind: crate::error::ParseErrorKind::Custom(m),
        span: crate::token::Span::default(),
    })?;

    // Universal ABI: Generate C header + bindings if any C exports exist
    let has_c = stmts.iter().any(|stmt| {
        if let Stmt::FunctionDef {
            is_exported: true,
            export_target,
            ..
        } = stmt
        {
            match export_target {
                None => true,
                Some(t) => interner.resolve(*t).eq_ignore_ascii_case("c"),
            }
        } else {
            false
        }
    });

    let c_header = if has_c {
        Some(generate_c_header(
            &stmts,
            "module",
            &interner,
            &codegen_registry,
        ))
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
        Some(generate_python_bindings(
            &stmts,
            "module",
            &interner,
            &codegen_registry,
        ))
    } else {
        None
    };

    let (typescript_bindings, typescript_types) = if has_c {
        let (js, dts) =
            generate_typescript_bindings(&stmts, "module", &interner, &codegen_registry);
        (Some(js), Some(dts))
    } else {
        (None, None)
    };

    Ok(CompileOutput {
        rust_code,
        dependencies,
        c_header,
        python_bindings,
        typescript_types,
        typescript_bindings,
    })
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
    let library = crate::loader::LibraryProvenance::of(source);
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    let (type_registry, policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };
    let codegen_policies = policy_registry.clone();

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let _compile_facts = crate::optimize::facts::scope();
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

    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
    );
    let stmts = parser.parse_program()?;
    let (parsed, parsed_spans) = (stmts.clone(), parser.stmt_spans().to_vec());
    let opt_config = {
        let mut c =
            crate::optimization::OptimizationConfig::from_env().merged(&parser.program_opt_flags());
        c.normalize();
        c
    };
    let codegen_registry = parser.into_type_registry();
    refuse_undeclared_types(&parsed, &parsed_spans, &codegen_registry, &interner)?;
    let resolved = crate::resolve_program::resolve_program(
        &stmts,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
        &mut interner,
        opt_config.is_on(crate::optimization::Opt::Comptime),
        library.functions(),
    );
    let stmts: Vec<Stmt> = match resolved {
        Some(rw) => rw.to_vec(),
        None => stmts,
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
        Stmt::FunctionDef {
            name,
            params,
            return_type,
            ..
        } if interner.resolve(*name) == target => {
            let ps: Vec<(crate::intern::Symbol, &TypeExpr)> =
                params.iter().map(|(s, t)| (*s, *t)).collect();
            Some((*name, ps, *return_type))
        }
        _ => None,
    });
    let Some((target_sym, params, return_type)) = target_sig else {
        return Ok(None);
    };
    // Native code that stops partway hands the call back to bytecode, which re-runs it and raises
    // the exact error — sound only for a call with nothing observable to repeat. A function with an
    // effect (a `Show`, IO, concurrency) stays on VM+JIT.
    if !crate::optimize::effects::EffectEnv::from_stmts(&stmts, &interner)
        .function_is_replayable(target)
    {
        return Ok(None);
    }
    // The shim only exists for the sound scalar subset; otherwise no AOT-native fn.
    let Some(shim) =
        crate::codegen::codegen_native_tier_export(target_sym, &params, return_type, &interner)
    else {
        return Ok(None);
    };

    let slice = crate::codegen::function_slice(&stmts, target_sym, &interner);
    let type_env = crate::analysis::check_program_located(&slice, &interner, &codegen_registry)
        .map_err(|e| ParseError {
            kind: e.to_parse_error_kind(&interner, &slice),
            span: crate::token::Span::default(),
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

    // The artifact is read back from `proj/target`, so the build writes there whatever the caller's
    // environment says: an inherited `CARGO_TARGET_DIR` would put it somewhere this code never looks.
    let out = Command::new("cargo")
        .args([
            "build",
            "--release",
            "--lib",
            "--target",
            "wasm32-unknown-unknown",
        ])
        .arg("--target-dir")
        .arg(proj.join("target"))
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

    // The `.so` is read back from `proj/target/release`, so the build writes there whatever the
    // caller's environment says: an inherited `CARGO_TARGET_DIR` would put the library somewhere this
    // code never looks, and the native tier would silently never load.
    let out = Command::new("cargo")
        .args(["build", "--release", "--lib"])
        .arg("--target-dir")
        .arg(proj.join("target"))
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
    let so = cache_dir
        .join(&crate_name)
        .join("target")
        .join("release")
        .join(format!(
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
    let module = compile_function_to_native_rust(source, fn_name)
        .ok()
        .flatten()?;
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
                Stmt::FunctionDef {
                    name,
                    is_exported: true,
                    export_target: Some(t),
                    ..
                } if interner.resolve(*t).eq_ignore_ascii_case("native") => {
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
fn extract_dependencies(
    stmts: &[Stmt],
    interner: &Interner,
) -> Result<Vec<CrateDependency>, ParseError> {
    use std::collections::HashMap;

    let mut seen: HashMap<String, String> = HashMap::new(); // name → version
    let mut deps: Vec<CrateDependency> = Vec::new();

    for stmt in stmts {
        if let Stmt::Require {
            crate_name,
            version,
            features,
            span,
        } = stmt
        {
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
                    features: features
                        .iter()
                        .map(|f| interner.resolve(*f).to_string())
                        .collect(),
                });
            }
        }
    }

    Ok(deps)
}

/// What an entry point judges on the program as written.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AsWritten {
    /// Zone escapes — every compile refuses one.
    Escapes,
    /// Zone escapes and uses after `Give` — the checked compile.
    EscapesAndOwnership,
}

/// Judge the program as its author wrote it: the freshly-parsed statements, before observation
/// elision, division resolution or the optimizer rewrite them. Those rewrites can fold away the
/// very names the checks follow — once `x` is a known constant, propagation turns `Give x to p.`
/// into `Give 5 to p.` and `Show x.` into `Show 5.`, and folding flattens a constant `If` — so a
/// check run afterwards passes whatever the optimizer happened to fold, and disagrees with the
/// editor, which judges the parse. Run it before `install_decorators`, so a refused program never
/// opens a `## Require` trace.
fn check_as_written(
    stmts: &[Stmt],
    interner: &Interner,
    checks: AsWritten,
) -> Result<(), ParseError> {
    fn refusal(message: String, span: crate::token::Span) -> ParseError {
        ParseError {
            kind: crate::error::ParseErrorKind::Custom(message),
            span,
        }
    }
    EscapeChecker::new(interner)
        .check_program(stmts)
        .map_err(|e| refusal(e.to_string(), e.span))?;
    if checks == AsWritten::EscapesAndOwnership {
        OwnershipChecker::new(interner)
            .check_program(stmts)
            .map_err(|e| refusal(e.to_string(), e.span))?;
    }
    Ok(())
}

/// Compile LOGOS source to Rust with ownership checking enabled.
///
/// This runs the lightweight ownership analysis pass that catches use-after-move
/// errors with control flow awareness. The analysis is fast enough to run on
/// every keystroke in an IDE. It judges the program as written, before the
/// optimizer runs: constant propagation would otherwise fold a given-away name
/// into its value and hide the use after it.
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
    let library = crate::loader::LibraryProvenance::of(source);
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    // Pass 1: Discovery - scan for type definitions and policies
    let (type_registry, policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };
    let codegen_policies = policy_registry.clone();

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let _compile_facts = crate::optimize::facts::scope();
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
    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
    );
    let stmts = parser.parse_program()?;
    let (parsed, parsed_spans) = (stmts.clone(), parser.stmt_spans().to_vec());

    // Pass 2.5: Optimization - constant folding, propagation, and dead code elimination.
    // Honor `## No`/`## Require` decorators (merge parser flags; a bare `from_env()` drops
    // them) and open a firing trace so an unfired `## Require` fails this path too.
    // `## No`/`## Require` decorators via the shared gate (merge into active config; verify
    // required opts fired after codegen). Last use of `parser` before `&mut interner` is reused.
    let (opt_config, gate) = crate::optimize::install_decorators(
        crate::optimization::OptimizationConfig::from_env(),
        &parser.program_opt_flags(),
        parser.required_optimizations(),
    );
    let codegen_registry = parser.into_type_registry();
    refuse_undeclared_types(&parsed, &parsed_spans, &codegen_registry, &interner)?;

    // Pass 3: Escape and ownership analysis — use-after-move with control-flow awareness —
    // on the program as written.
    check_as_written(&stmts, &interner, AsWritten::EscapesAndOwnership)?;

    let resolved = crate::resolve_program::resolve_program(
        &stmts,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
        &mut interner,
        opt_config.is_on(crate::optimization::Opt::Comptime),
        library.functions(),
    );
    let stmts: Vec<Stmt> = match resolved {
        Some(rw) => rw.to_vec(),
        None => stmts,
    };
    let (stmts, codegen_registry) = lift_program_bindings(
        stmts,
        codegen_registry,
        &mut interner,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
    )?;
    let stmts = crate::optimize::optimize_program(
        stmts,
        &imperative_expr_arena,
        &stmt_arena,
        &mut interner,
        &opt_config,
    );

    let type_env = crate::analysis::check_program_located(&stmts, &interner, &codegen_registry)
        .map_err(|e| ParseError {
            kind: e.to_parse_error_kind(&interner, &stmts),
            span: crate::token::Span::default(),
        })?;
    let rust_code = codegen_program(
        &stmts,
        &codegen_registry,
        &codegen_policies,
        &interner,
        &type_env,
        &opt_config,
    );

    gate.verify().map_err(|m| ParseError {
        kind: crate::error::ParseErrorKind::Custom(m),
        span: crate::token::Span::default(),
    })?;

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
    let library = crate::loader::LibraryProvenance::of(source);

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    // Pass 1: Discovery - scan for type definitions and policies
    let (type_registry, policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };
    let codegen_policies = policy_registry.clone();

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let _compile_facts = crate::optimize::facts::scope();
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
    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
    );
    let stmts = parser.parse_program()?;
    let (parsed, parsed_spans) = (stmts.clone(), parser.stmt_spans().to_vec());

    // Honor `## No`/`## Require` decorators on the verified path too. Captured here (the last
    // use of `parser`) so its `&mut interner` borrow ends before the immutable `&interner`
    // passes below; a bare `from_env()` at codegen would silently drop them.
    // `## No`/`## Require` decorators via the shared gate (merge into active config; verify
    // required opts fired after codegen). Last use of `parser` before `&mut interner` is reused.
    let (opt_config, gate) = crate::optimize::install_decorators(
        crate::optimization::OptimizationConfig::from_env(),
        &parser.program_opt_flags(),
        parser.required_optimizations(),
    );
    let codegen_registry = parser.into_type_registry();
    refuse_undeclared_types(&parsed, &parsed_spans, &codegen_registry, &interner)?;

    // Pass 3: Escape analysis, on the program as written
    check_as_written(&stmts, &interner, AsWritten::Escapes)?;

    let resolved = crate::resolve_program::resolve_program(
        &stmts,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
        &mut interner,
        opt_config.is_on(crate::optimization::Opt::Comptime),
        library.functions(),
    );
    let stmts: Vec<Stmt> = match resolved {
        Some(rw) => rw.to_vec(),
        None => stmts,
    };

    // Pass 4: Static verification
    let mut verifier = VerificationPass::new(&interner);
    verifier.verify_program(&stmts).map_err(|e| ParseError {
        kind: crate::error::ParseErrorKind::Custom(format!("Verification Failed:\n\n{}", e)),
        span: crate::token::Span::default(),
    })?;

    let type_env = crate::analysis::check_program_located(&stmts, &interner, &codegen_registry)
        .map_err(|e| ParseError {
            kind: e.to_parse_error_kind(&interner, &stmts),
            span: crate::token::Span::default(),
        })?;
    let rust_code = codegen_program(
        &stmts,
        &codegen_registry,
        &codegen_policies,
        &interner,
        &type_env,
        &opt_config,
    );

    gate.verify().map_err(|m| ParseError {
        kind: crate::error::ParseErrorKind::Custom(m),
        span: crate::token::Span::default(),
    })?;

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

/// The offset in the author's own text that byte `pos` of the prelude-expanded text came from,
/// through the runs [`crate::loader::apply_prelude_mapped`] reports. A position inside spliced
/// stdlib text came from no byte the author wrote and maps to `None`; a run's END maps too, so an
/// exclusive span end and the end of the file translate.
pub fn prelude_user_offset(
    segments: &[crate::loader::PreludeSegment],
    pos: usize,
) -> Option<usize> {
    let mut at_end = None;
    for seg in segments {
        if pos >= seg.out_start && pos < seg.out_start + seg.len {
            return seg.user_start.map(|us| us + (pos - seg.out_start));
        }
        if pos == seg.out_start + seg.len {
            at_end = seg.user_start.map(|us| us + seg.len);
        }
    }
    at_end
}

/// A span of the prelude-expanded text as a span of the author's own text — `None` when either
/// end lies in spliced stdlib text. The one inversion of the prelude splice: every surface that
/// reports a location against the author's file translates through this.
pub fn prelude_user_span(
    segments: &[crate::loader::PreludeSegment],
    span: crate::token::Span,
) -> Option<crate::token::Span> {
    match (
        prelude_user_offset(segments, span.start),
        prelude_user_offset(segments, span.end),
    ) {
        (Some(start), Some(end)) if start <= end => Some(crate::token::Span::new(start, end)),
        _ => None,
    }
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

    // The stdlib prelude is spliced in on demand — AFTER any leading
    // title/comment prose, and `## NoPrelude` strips its marker — so spans
    // from the parse are offsets into the expanded text and translate back
    // through the exact edit `apply_prelude` reports.
    let (prelude_src, prelude_segments) = crate::loader::apply_prelude_mapped(user_source);
    // Prune the DEAD prelude — the stdlib functions the whole-module splice pulled in
    // but the program never reaches — EXACTLY as `largo build` does before it parses
    // (`compile_project` = prune ∘ apply_prelude). Deep check then checks only what the
    // program executes and what the shipped binary contains: an UNREACHABLE stdlib
    // helper can never surface a finding (or a parse error) on code that never runs.
    let (pruned_src, kept_ranges) =
        crate::loader::prune_unreachable_prelude_mapped_from(user_source, prelude_src.as_ref());
    // Map an offset in the pre-prune (expanded) text to its offset in the pruned text:
    // the count of kept bytes lying before it. Only defined for kept offsets — every
    // user run is kept, which is all the span translation needs.
    let to_pruned = |x: usize| -> usize {
        let mut kept = 0usize;
        for &(s, e) in &kept_ranges {
            if e <= x {
                kept += e - s;
            } else if s < x {
                kept += x - s;
                break;
            } else {
                break;
            }
        }
        kept
    };
    // Rebuild the span segments in PRUNED coordinates. Only USER runs carry a mapping
    // (a spliced — or now pruned — prelude run maps to nothing); each user run's bytes
    // are intact and contiguous, so it shifts uniformly by the prelude removed before
    // it. Reuses the exact `user_pos` translation below, just over the pruned text.
    let prelude_segments: Vec<crate::loader::PreludeSegment> = prelude_segments
        .into_iter()
        .filter(|seg| seg.user_start.is_some())
        .map(|seg| crate::loader::PreludeSegment {
            out_start: to_pruned(seg.out_start),
            len: seg.len,
            user_start: seg.user_start,
        })
        .collect();
    let expanded: &str = pruned_src.as_str();
    let library = crate::loader::LibraryProvenance::of(expanded);
    for seg in &prelude_segments {
        if let Some(user_start) = seg.user_start {
            debug_assert_eq!(
                &expanded[seg.out_start..seg.out_start + seg.len],
                &user_source[user_start..user_start + seg.len],
                "prelude prune must keep user text byte-intact; span translation depends on it"
            );
        }
    }

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(expanded, &mut interner);
    let tokens = lexer.tokenize();

    let (type_registry, policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };
    let codegen_policies = policy_registry.clone();

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let _compile_facts = crate::optimize::facts::scope();
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

    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
    );
    let stmts = parser.parse_program()?;

    // Translate spans back into the USER buffer; spliced-prelude statements
    // become zero-width sentinels that mapped codegen skips.
    let stmt_spans: Vec<crate::token::Span> = parser
        .stmt_spans()
        .iter()
        .map(|span| prelude_user_span(&prelude_segments, *span).unwrap_or_default())
        .collect();
    let codegen_registry = parser.into_type_registry();

    // Exact division still resolves (type semantics, statement-count
    // preserving); the optimizer stays OFF so spans align 1:1.
    let resolved = crate::resolve_program::resolve_program(
        &stmts,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
        &mut interner,
        false,
        library.functions(),
    );
    let stmts: Vec<Stmt> = match resolved {
        Some(rw) => rw.to_vec(),
        None => stmts,
    };
    let lifted_from = stmts.len();
    let (stmts, codegen_registry) = lift_program_bindings(
        stmts,
        codegen_registry,
        &mut interner,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
    )?;
    // The lift opens the program with the record's own `Let`, which no source line wrote.
    let stmt_spans: Vec<crate::token::Span> =
        std::iter::repeat_n(crate::token::Span::default(), stmts.len() - lifted_from)
            .chain(stmt_spans)
            .collect();

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

    // The cache dir's own target, whatever the caller's environment says: under a shared
    // `CARGO_TARGET_DIR` a check of one program finds another's `logos_output` fingerprint fresh and
    // replays that program's diagnostics.
    let check_output = Command::new("cargo")
        .arg("check")
        .arg("--message-format=json")
        .current_dir(cache_dir)
        .env("CARGO_TARGET_DIR", cache_dir.join("target"))
        .output()
        .map_err(|e| CompileError::Io(e.to_string()))?;

    let stdout = String::from_utf8_lossy(&check_output.stdout);
    let diagnostics = parse_rustc_json(&stdout);
    let findings = crate::diagnostic::translate_diagnostics_all(
        &diagnostics,
        &artifacts.source_map,
        &artifacts.interner,
    );

    if !check_output.status.success() && findings.is_empty() {
        let stderr = String::from_utf8_lossy(&check_output.stderr);
        return Err(CompileError::Build(stderr.to_string()));
    }

    Ok(findings)
}

/// Remap every rustc error over a project's GENERATED Rust back to the LOGOS
/// source line it came from — the build path uses this so a generated-code
/// failure points at the LOGOS the user wrote, not `main.rs`. Compiles with the
/// source-mapped codegen (whose line numbers the map indexes), runs
/// `cargo check --message-format=json`, and remaps each diagnostic's primary
/// span. Returns `Ok(vec![])` when nothing maps (caller falls back to raw cargo
/// output).
pub fn remap_project_errors(
    source: &str,
    cache_dir: &Path,
    file_map: Option<&crate::analysis::SourceMap>,
) -> Result<Vec<crate::diagnostic::RemappedError>, CompileError> {
    let artifacts = rustc_check_artifacts(source).map_err(CompileError::Parse)?;
    write_cargo_project(&artifacts.rust_code, &artifacts.dependencies, cache_dir)?;

    // The cache dir's own target, whatever the caller's environment says: under a shared
    // `CARGO_TARGET_DIR` a check of one program finds another's `logos_output` fingerprint fresh and
    // replays that program's diagnostics.
    let check_output = Command::new("cargo")
        .arg("check")
        .arg("--message-format=json")
        .current_dir(cache_dir)
        .env("CARGO_TARGET_DIR", cache_dir.join("target"))
        .output()
        .map_err(|e| CompileError::Io(e.to_string()))?;

    let stdout = String::from_utf8_lossy(&check_output.stdout);
    let diagnostics = parse_rustc_json(&stdout);

    // Attribute each LOGOS byte offset to its originating `.lg` file (multi-file)
    // via the assembler's byte-range→file map, when one is supplied.
    let file_of = |offset: usize| -> Option<String> {
        file_map
            .and_then(|m| m.file_for(offset))
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .map(String::from)
    };
    Ok(crate::diagnostic::remap_rustc_diagnostics(
        &diagnostics,
        &artifacts.source_map,
        &file_of,
    ))
}

pub fn compile_to_dir(source: &str, output_dir: &Path) -> Result<(), CompileError> {
    let output = compile_program_full(source).map_err(CompileError::Parse)?;
    write_cargo_project(&output.rust_code, &output.dependencies, output_dir)
}

/// How a generated project resolves the `logicaffeine` runtime crates.
///
/// Inside the monorepo we stage the crate *sources* beside the generated
/// program and path-depend on them (hackable, offline). An installed `largo`
/// has no checkout to stage from, so it version-depends on the published
/// crates.io releases pinned to this compiler's OWN version — the generated
/// Rust and the runtime API can then never drift.
pub enum RuntimeCrateSource {
    /// Monorepo checkout: stage sources under `./crates` and path-depend.
    Workspace,
    /// Standalone install: depend on the crates.io releases at `version`.
    Registry { version: String },
}

impl RuntimeCrateSource {
    /// A monorepo checkout stages sources; anywhere else (an installed `largo`)
    /// pulls the matching crates.io release at this compiler's own version.
    pub fn detect() -> Self {
        match find_workspace_root() {
            Ok(_) => RuntimeCrateSource::Workspace,
            Err(_) => RuntimeCrateSource::Registry {
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        }
    }

    /// True inside a checkout (stage sources + path-depend); false for an
    /// installed `largo` (nothing to copy — depend on crates.io).
    pub fn is_workspace(&self) -> bool {
        matches!(self, RuntimeCrateSource::Workspace)
    }

    /// The `logicaffeine-data` dependency line for this source.
    pub fn data_dep(&self) -> String {
        match self {
            RuntimeCrateSource::Workspace => {
                "logicaffeine-data = { path = \"./crates/logicaffeine_data\" }".to_string()
            }
            // Exact-pin (`=`) so the generated Rust always links the runtime
            // whose API produced it — a caret would let a newer 0.x patch drift.
            RuntimeCrateSource::Registry { version } => {
                format!("logicaffeine-data = \"={version}\"")
            }
        }
    }

    /// The `logicaffeine-system` dependency line carrying `features`.
    pub fn system_dep(&self, features: &[&str]) -> String {
        let feats = features
            .iter()
            .map(|f| format!("\"{f}\""))
            .collect::<Vec<_>>()
            .join(", ");
        match self {
            RuntimeCrateSource::Workspace => format!(
                "logicaffeine-system = {{ path = \"./crates/logicaffeine_system\", features = [{feats}] }}"
            ),
            RuntimeCrateSource::Registry { version } => {
                format!("logicaffeine-system = {{ version = \"={version}\", features = [{feats}] }}")
            }
        }
    }

    /// The target section every generated project ends its dependencies with: on Linux the runtime
    /// carries `io-uring`, which cargo unifies with the features its `[dependencies]` line names,
    /// so a compiled server has the thread-per-core io_uring engine and the `UringVfs`, and every
    /// other target builds without them.
    pub fn linux_engine_section(&self) -> String {
        format!(
            "[target.'cfg(target_os = \"linux\")'.dependencies]\n{}\n",
            self.system_dep(&["full", "io-uring"])
        )
    }
}

/// A declared field type's base name (`SharedMap` for `SharedMap from Text to Int`).
fn field_type_base_name<'i>(ty: &crate::analysis::FieldType, interner: &'i Interner) -> &'i str {
    use crate::analysis::FieldType;
    match ty {
        FieldType::Primitive(s) | FieldType::Named(s) | FieldType::TypeParam(s) => {
            interner.resolve(*s)
        }
        FieldType::Generic { base, .. } => interner.resolve(*base),
        FieldType::Tuple(_) => "Tuple",
        FieldType::Record(_) => "Record",
    }
}

/// A backend that lowers compiled bytecode WITHOUT the Rust runtime — the tiers
/// [`refuse_unlinkable`] gates. What each one lowers is answered per construct against this type
/// (see [`crate::semantics::crdt::CrdtKind::lowered_by`]), so adding a backend fails to compile until
/// every such table classifies it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backend {
    /// The bytecode → C op-walker (`compile_to_c_via_bytecode`).
    C,
    /// The AST → C backend (`compile_to_c`), which gates a program's declared field types itself.
    CFromAst,
    RiscV,
    WebAssembly,
    SoftCore,
}

impl Backend {
    pub const ALL: [Backend; 5] = [
        Backend::C,
        Backend::CFromAst,
        Backend::RiscV,
        Backend::WebAssembly,
        Backend::SoftCore,
    ];

    /// The backend's name in a refusal sentence.
    pub const fn name(self) -> &'static str {
        match self {
            Backend::C | Backend::CFromAst => "C",
            Backend::RiscV => "RISC-V",
            Backend::WebAssembly => "WebAssembly",
            Backend::SoftCore => "soft-core",
        }
    }
}

/// The entry gate of every backend that lowers bytecode WITHOUT the Rust runtime (C, RISC-V,
/// WebAssembly, the soft core): refuse — by name — what only the interpreter, the VM, and the Rust tier
/// provide, rather than lower it as something else. A standard-library native kernel is Rust code; a
/// CRDT kind the backend does not lower is refused by its type name. Only code that can run is judged
/// ([`crate::vm::reach`]): the loader splices whole standard-library modules, and a function nothing
/// calls refuses nothing.
pub fn refuse_unlinkable(
    program: &crate::vm::CompiledProgram,
    interner: &Interner,
    backend: Backend,
) -> Result<(), String> {
    refuse_unlinkable_with(
        program,
        interner,
        backend,
        &crate::vm::reach::HostEntries::default(),
    )
}

/// [`refuse_unlinkable`], judging what the host's own `entries` reach as well as Main — the same
/// roots a module built with those entries lowers.
pub fn refuse_unlinkable_with(
    program: &crate::vm::CompiledProgram,
    interner: &Interner,
    backend: Backend,
    entries: &crate::vm::reach::HostEntries,
) -> Result<(), String> {
    crate::semantics::native_kernels::refuse_kernel_calls_with(
        program,
        interner,
        backend.name(),
        entries,
    )?;
    for (_, op) in crate::vm::reach::running_ops_with(program, interner, entries) {
        if let crate::vm::Op::NewCrdt { kind, .. } = *op {
            if let Some(k) = crate::semantics::crdt::CrdtKind::from_new_crdt_code(kind) {
                if !k.lowered_by(backend) {
                    return Err(k.refusal(backend.name()));
                }
            }
        }
    }
    Ok(())
}

/// The third-party crates the generated Rust names DIRECTLY (not through the runtime crates), each as
/// its `[dependencies]` line: declared iff the code uses it, and never when a `## Requires` already
/// brought it in (a duplicate key is a manifest error). The ONE list every generated manifest — the
/// `largo build` project and the staged checking project — takes its implied dependencies from.
pub fn implied_crate_deps(rust_code: &str, requires: &[CrateDependency]) -> Vec<&'static str> {
    const IMPLIED: &[(&str, &str, &str)] = &[
        // The parallel-reduction codegen (`rayon::prelude::*`) and `Simultaneously:` (`rayon::join`).
        ("rayon", "rayon::", "rayon = \"1\""),
        // The stdlib text-search intrinsic's SIMD `memchr`/`memmem`.
        ("memchr", "memchr::", "memchr = \"2\""),
        // `Shared`/`Portable` types derive `serde::Serialize`/`Deserialize` (sync + wire).
        (
            "serde",
            "serde::",
            "serde = { version = \"1\", features = [\"derive\"] }",
        ),
    ];
    IMPLIED
        .iter()
        .filter(|(name, marker, _)| {
            rust_code.contains(marker) && !requires.iter().any(|d| d.name == *name)
        })
        .map(|(_, _, line)| *line)
        .collect()
}

/// Render the generated project's `Cargo.toml`. `source` selects staged path
/// deps (in-workspace) vs. crates.io version deps (standalone install); the
/// feature sets, tokio, user `## Requires` deps, and release profile are
/// identical either way.
fn render_cargo_manifest(
    rust_code: &str,
    dependencies: &[CrateDependency],
    source: &RuntimeCrateSource,
) -> String {
    let data_dep = source.data_dep();
    // A `Serve` program's generated code names `logicaffeine_system::http`;
    // the staged project must carry the same runtime feature the real build
    // manifest (`project/build.rs`) does, or checking a web handler drowns
    // the user in phantom findings.
    let system_features: &[&str] = if rust_code.contains("logicaffeine_system::http::") {
        &["full", "http"]
    } else {
        &["full"]
    };
    let system_dep = source.system_dep(system_features);

    let mut cargo_toml = format!(
        "[package]\n\
         name = \"logos_output\"\n\
         version = \"0.1.0\"\n\
         edition = \"2021\"\n\
         \n\
         [dependencies]\n\
         {data_dep}\n\
         {system_dep}\n\
         tokio = {{ version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }}\n"
    );

    // User-declared `## Requires` deps — inside [dependencies], before any later
    // section header (a dep after a [target.*]/[profile.*] header changes meaning).
    for dep in dependencies {
        if dep.features.is_empty() {
            let _ = writeln!(cargo_toml, "{} = \"{}\"", dep.name, dep.version);
        } else {
            let feats = dep
                .features
                .iter()
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

    for line in implied_crate_deps(rust_code, dependencies) {
        let _ = writeln!(cargo_toml, "{line}");
    }

    let _ = write!(cargo_toml, "\n{}", source.linux_engine_section());

    let panic_strategy = crate::codegen::release_panic_strategy(rust_code);
    let _ = write!(cargo_toml, "\n[profile.release]\nlto = true\nopt-level = 3\ncodegen-units = 1\npanic = \"{panic_strategy}\"\nstrip = true\n");
    cargo_toml
}

/// Write a generated program as a runnable cargo project: `src/main.rs`,
/// `Cargo.toml` (runtime deps + user `## Requires`), `.cargo/config.toml`, and
/// — only inside a checkout — the staged runtime crates. Shared by
/// [`compile_to_dir`] and [`rustc_check`].
pub fn write_cargo_project(
    rust_code: &str,
    dependencies: &[CrateDependency],
    output_dir: &Path,
) -> Result<(), CompileError> {
    let source = RuntimeCrateSource::detect();

    // Create output directory structure
    let src_dir = output_dir.join("src");
    fs::create_dir_all(&src_dir).map_err(|e| CompileError::Io(e.to_string()))?;

    // Write main.rs (codegen already includes the use statements)
    let main_path = src_dir.join("main.rs");
    let mut file = fs::File::create(&main_path).map_err(|e| CompileError::Io(e.to_string()))?;
    file.write_all(rust_code.as_bytes())
        .map_err(|e| CompileError::Io(e.to_string()))?;

    // Write Cargo.toml with runtime crate dependencies
    let cargo_toml = render_cargo_manifest(rust_code, dependencies, &source);
    let cargo_path = output_dir.join("Cargo.toml");
    let mut file = fs::File::create(&cargo_path).map_err(|e| CompileError::Io(e.to_string()))?;
    file.write_all(cargo_toml.as_bytes())
        .map_err(|e| CompileError::Io(e.to_string()))?;

    // Write .cargo/config.toml with target-cpu=native for optimal codegen.
    // Enables SIMD auto-vectorization and CPU-specific instruction selection.
    let cargo_config_dir = output_dir.join(".cargo");
    fs::create_dir_all(&cargo_config_dir).map_err(|e| CompileError::Io(e.to_string()))?;
    let config_content = "[build]\nrustflags = [\"-C\", \"target-cpu=native\"]\n";
    let config_path = cargo_config_dir.join("config.toml");
    fs::write(&config_path, config_content).map_err(|e| CompileError::Io(e.to_string()))?;

    // Stage the runtime sources only inside a checkout; a standalone install
    // pulls them from crates.io, so there is nothing to copy.
    if source.is_workspace() {
        copy_runtime_crates(output_dir)?;
    }

    Ok(())
}

/// Copy the runtime crates to the output directory.
/// Copies logicaffeine_data and logicaffeine_system.
pub fn copy_runtime_crates(output_dir: &Path) -> Result<(), CompileError> {
    let crates_dir = output_dir.join("crates");
    fs::create_dir_all(&crates_dir).map_err(|e| CompileError::Io(e.to_string()))?;

    // Find workspace root
    let workspace_root = find_workspace_root()?;
    let workspace = WorkspacePackage::read(&workspace_root)?;

    // Copy logicaffeine_data
    let data_src = workspace_root.join(CRATES_DATA_PATH);
    let data_dest = crates_dir.join("logicaffeine_data");
    copy_dir_recursive(&data_src, &data_dest)?;
    deworkspace_cargo_toml(&data_dest.join("Cargo.toml"), &workspace)?;

    // Copy logicaffeine_system
    let system_src = workspace_root.join(CRATES_SYSTEM_PATH);
    let system_dest = crates_dir.join("logicaffeine_system");
    copy_dir_recursive(&system_src, &system_dest)?;
    deworkspace_cargo_toml(&system_dest.join("Cargo.toml"), &workspace)?;

    // Also need to copy logicaffeine_base since both crates depend on it
    let base_src = workspace_root.join("crates/logicaffeine_base");
    let base_dest = crates_dir.join("logicaffeine_base");
    copy_dir_recursive(&base_src, &base_dest)?;
    deworkspace_cargo_toml(&base_dest.join("Cargo.toml"), &workspace)?;

    // ★ AND THE WORKSPACE'S LOCKFILE. A generated project has no `Cargo.lock`, so cargo resolves
    // every transitive dependency to the newest version its requirement ALLOWS — which is not the
    // set this workspace has ever built. The copied crates carry unpinned minors by design
    // (`rustls = { version = "0.23", … }`), and on 2026-09-11 a fresh resolve paired rustls 0.23.44
    // with a `ring` that no longer satisfies it: 16 errors inside rustls itself
    // (`SystemRandom: SecureRandom` unsatisfied, `UnixTime::now` gone), none of them in generated
    // code, failing `hotswap_p17_wasm::scalar_function_builds_to_a_wasm_module` on all 3 attempts.
    //
    // Seeding the lock makes a generated build resolve to exactly the versions the workspace has
    // verified, so crates.io moving underneath us can no longer break a build nobody changed.
    // Cargo still updates it for anything genuinely new, so this pins without freezing.
    let lock_src = workspace_root.join("Cargo.lock");
    let lock_dest = output_dir.join("Cargo.lock");
    if lock_src.exists() && !lock_dest.exists() {
        fs::copy(&lock_src, &lock_dest).map_err(|e| CompileError::Io(e.to_string()))?;
    }

    Ok(())
}

/// The settings a staged crate inherits from the workspace, read from `[workspace.package]` in the
/// root `Cargo.toml`, so a standalone copy declares exactly the edition and minimum Rust the
/// workspace does.
struct WorkspacePackage {
    edition: String,
    rust_version: String,
}

impl WorkspacePackage {
    fn read(workspace_root: &Path) -> Result<WorkspacePackage, CompileError> {
        let manifest = workspace_root.join("Cargo.toml");
        let text = fs::read_to_string(&manifest)
            .map_err(|e| CompileError::Io(format!("{}: {e}", manifest.display())))?;
        Self::parse(&text).ok_or_else(|| {
            CompileError::Io(format!(
                "{} declares no `edition` and `rust-version` under [workspace.package]",
                manifest.display()
            ))
        })
    }

    /// The quoted `edition` and `rust-version` values of the `[workspace.package]` table.
    fn parse(root_manifest: &str) -> Option<WorkspacePackage> {
        let mut in_table = false;
        let (mut edition, mut rust_version) = (None, None);
        for line in root_manifest.lines() {
            let t = line.trim();
            if t.starts_with('[') {
                in_table = t == "[workspace.package]";
                continue;
            }
            let Some((key, value)) = t.split_once('=').filter(|_| in_table) else {
                continue;
            };
            let Some(quoted) = value
                .trim()
                .strip_prefix('"')
                .and_then(|v| v.split_once('"'))
                .map(|(v, _)| v.to_string())
            else {
                continue;
            };
            match key.trim() {
                "edition" => edition = Some(quoted),
                "rust-version" => rust_version = Some(quoted),
                _ => {}
            }
        }
        Some(WorkspacePackage {
            edition: edition?,
            rust_version: rust_version?,
        })
    }
}

/// Resolve workspace-inherited fields in a copied crate's Cargo.toml.
///
/// When runtime crates are copied to a standalone project, any fields using
/// `*.workspace = true` won't resolve because there's no parent workspace.
/// This rewrites them with the workspace's own values.
fn deworkspace_cargo_toml(
    cargo_toml_path: &Path,
    workspace: &WorkspacePackage,
) -> Result<(), CompileError> {
    let content =
        fs::read_to_string(cargo_toml_path).map_err(|e| CompileError::Io(e.to_string()))?;

    let mut result = String::with_capacity(content.len());
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "edition.workspace = true" {
            result.push_str(&format!("edition = \"{}\"", workspace.edition));
        } else if trimmed == "rust-version.workspace = true" {
            result.push_str(&format!("rust-version = \"{}\"", workspace.rust_version));
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

    fs::write(cargo_toml_path, result).map_err(|e| CompileError::Io(e.to_string()))?;

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
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
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
pub(crate) fn find_workspace_root() -> Result<std::path::PathBuf, CompileError> {
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
    let mut current = std::env::current_dir().map_err(|e| CompileError::Io(e.to_string()))?;

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
                    fs::write(&dst_path, filtered).map_err(|e| CompileError::Io(e.to_string()))?;
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

    // Every generated package is named `logos_output`, so the project builds into ITS OWN target
    // whatever the caller's environment says. Under an inherited `CARGO_TARGET_DIR` two projects
    // share one `target/debug/logos_output`: the second build finds the first's fingerprint fresh
    // and `cargo run` starts whichever program linked last.
    let target_dir = output_dir.join("target");

    // Run cargo build with JSON message format for structured error parsing
    let build_output = Command::new("cargo")
        .arg("build")
        .arg("--message-format=json")
        .current_dir(output_dir)
        .env("CARGO_TARGET_DIR", &target_dir)
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
        .env("CARGO_TARGET_DIR", &target_dir)
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
    compile_project_with_deps(entry_file, Vec::new())
}

/// [`compile_project`], additionally linking the declared dependency roots (see
/// [`assemble_project_with_deps`](crate::analysis::assemble_project_with_deps)).
/// `dep_roots` empty is byte-for-byte identical to [`compile_project`].
pub fn compile_project_with_deps(
    entry_file: &Path,
    dep_roots: Vec<crate::loader::DepRoot>,
) -> Result<CompileOutput, CompileError> {
    use crate::analysis::{assemble_project_with_deps, discover_with_imports};
    use crate::loader::Loader;

    let root_path = entry_file.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut loader = Loader::with_dep_roots(root_path, dep_roots.clone());
    let mut interner = Interner::new();

    // Assemble the whole project into ONE compilation unit: the entry with every
    // imported module's code (functions + type defs) spliced in, dependency-first
    // and cycle-safe. This is the seam that makes cross-file FUNCTION calls resolve
    // — imported `## To` definitions now reach the parser, call graph, and codegen
    // exactly as a single-file program's do.
    let assembled = assemble_project_with_deps(entry_file, dep_roots).map_err(CompileError::Io)?;

    // Auto-import the demand-driven stdlib prelude (uuid.lg, io/net/crdt/time/…) — the SAME seam the
    // string-compile and interpreter paths use (see `apply_prelude`), so `largo build` resolves stdlib
    // functions like `md5`/`sha1`/`uuidV3`/`uuidParse`/`flush`. Demand-driven + idempotent: a program
    // that references no stdlib name compiles byte-identically to before. Then drop the DEAD prelude
    // functions the whole-module splice pulled in but the project never reaches (a postgres app pulls all
    // of crypto.lg for the SCRAM/MD5 auth path, but never the ML-DSA signature functions) — never touching
    // a user function, byte-identical when nothing is dead, so the AOT-Rust backend compiles only what the
    // program executes, exactly as `compile_program_full` does for the single-source path.
    let source = crate::loader::prune_unreachable_prelude_from(
        &assembled.source,
        crate::loader::apply_prelude(&assembled.source).as_ref(),
    );

    // Discover types from the entry file and all imports (namespaced `Alias::Type`).
    let type_registry = discover_with_imports(entry_file, &source, &mut loader, &mut interner)
        .map_err(|e| CompileError::Io(e))?;

    // Now compile with the discovered types, enforcing per-file scoping. A parse
    // error is attributed back to the SOURCE FILE it came from (not an offset in
    // the merged source the user never wrote).
    compile_to_rust_with_registry_full(
        &source,
        type_registry,
        &mut interner,
        Some(&assembled.scope),
    )
    .map_err(|pe| attribute_parse_error(pe, &assembled, &source))
}

/// Attribute a parse error in a multi-file build back to the file it came from,
/// naming that file in the message. Best-effort: if the location can't be mapped,
/// the original error is returned unchanged.
///
/// The parse ran over `final_source` — the assembled program with the stdlib prelude spliced in as
/// ONE run after any leading prose (pruning keeps it one run) — so its span is moved back into the
/// assembled program's own coordinates, the ones every surface locates against: past the run it
/// moves back by the run's length, and a span inside the run names text the author never wrote and
/// carries no place (`Span::default()`).
fn attribute_parse_error(
    pe: ParseError,
    assembled: &crate::analysis::AssembledProgram,
    final_source: &str,
) -> CompileError {
    let prelude_len = final_source.len().saturating_sub(assembled.source.len());
    let insert_at = if assembled.source.starts_with("##") {
        0
    } else {
        assembled.source.find("\n##").map(|i| i + 1).unwrap_or(0)
    };
    let to_assembled = |at: usize| -> Option<usize> {
        if prelude_len == 0 || at <= insert_at {
            Some(at)
        } else if at >= insert_at + prelude_len {
            Some(at - prelude_len)
        } else {
            None
        }
    };
    let span = match (to_assembled(pe.span.start), to_assembled(pe.span.end)) {
        (Some(start), Some(end)) if start <= end => crate::token::Span::new(start, end),
        _ => crate::token::Span::default(),
    };
    let pe = ParseError { span, ..pe };
    // Scope/collision diagnostics are already well-formed (they name their files);
    // only raw syntax errors need location attribution.
    if matches!(pe.kind, crate::error::ParseErrorKind::Custom(_)) {
        return CompileError::Parse(pe);
    }

    // Only IMPORTED-module errors need re-attribution — an error in the entry
    // file already reports correctly, and re-wrapping it would change the
    // single-file diagnostic format the CLI pins.
    if let Some(file) = assembled.source_map.file_for(pe.span.start) {
        if file != assembled.scope.entry() {
            if let Some(name) = file.file_name().and_then(|s| s.to_str()) {
                return CompileError::Parse(ParseError {
                    kind: crate::error::ParseErrorKind::Custom(format!(
                        "in `{name}`: {}",
                        describe_parse_error(&pe)
                    )),
                    span: pe.span,
                });
            }
        }
    }
    CompileError::Parse(pe)
}

/// The author-facing sentence for a parse error in an imported module: the same explanation an
/// entry-file error carries (`ParseError::diagnostic_parts`). `socratic_explanation` matches every
/// `ParseErrorKind` with no wildcard arm, so a new kind cannot reach the author unexplained.
pub(crate) fn describe_parse_error(pe: &ParseError) -> String {
    crate::error::socratic_explanation(pe, &Interner::new())
}

/// Enforce per-file scoping for a project WITHOUT codegen. The interpreter and
/// other lightweight tiers call this so transitive leakage is rejected uniformly
/// with `largo build` — a program that fails scoping on one tier fails on all.
/// Returns `Ok(())` when every reference stays within its file's scope, else the
/// clean scope-violation message.
pub fn enforce_project_scope(entry_file: &Path) -> Result<(), String> {
    enforce_project_scope_with_deps(entry_file, Vec::new()).map_err(|e| e.to_string())
}

/// Why a project fails [`enforce_project_scope_with_deps`].
#[derive(Debug)]
pub enum ProjectScopeError {
    /// The assembled program does not parse — the parser's own error, so a surface can show it
    /// where it is rather than as a sentence with no place.
    Parse(ParseError),
    /// The project does not assemble, or a file reaches a name it neither defines nor imports.
    Refused(String),
}

impl std::fmt::Display for ProjectScopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectScopeError::Parse(pe) => f.write_str(&describe_parse_error(pe)),
            ProjectScopeError::Refused(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for ProjectScopeError {}

/// [`enforce_project_scope`], additionally linking the declared dependency roots.
/// `dep_roots` empty is identical to [`enforce_project_scope`].
pub fn enforce_project_scope_with_deps(
    entry_file: &Path,
    dep_roots: Vec<crate::loader::DepRoot>,
) -> Result<(), ProjectScopeError> {
    use crate::analysis::{assemble_project_with_deps, check_scopes, discover_with_imports};
    use crate::loader::Loader;

    let root_path = entry_file.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut loader = Loader::with_dep_roots(root_path, dep_roots.clone());
    let mut interner = Interner::new();

    let assembled =
        assemble_project_with_deps(entry_file, dep_roots).map_err(ProjectScopeError::Refused)?;
    let source = crate::loader::apply_prelude(&assembled.source).into_owned();
    let type_registry = discover_with_imports(entry_file, &source, &mut loader, &mut interner)
        .map_err(ProjectScopeError::Refused)?;

    let mut lexer = Lexer::new(&source, &mut interner);
    let mut tokens = lexer.tokenize();
    let functions = crate::analysis::assemble::defined_function_names(&source);
    crate::analysis::assemble::resolve_qualified_names(&mut tokens, &mut interner, &functions);

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let _compile_facts = crate::optimize::facts::scope();
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

    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
    );
    let stmts = parser.parse_program().map_err(ProjectScopeError::Parse)?;

    check_scopes(&stmts, &assembled.scope, &interner).map_err(ProjectScopeError::Refused)
}

/// Compile LOGOS source with a pre-populated type registry, returning full output.
/// Returns both generated Rust code and extracted dependencies. `scope`, when
/// present (multi-file `largo` builds), enforces per-file scoping over the
/// assembled program — a file may only reference what it defines or imports.
fn compile_to_rust_with_registry_full(
    source: &str,
    type_registry: crate::analysis::TypeRegistry,
    interner: &mut Interner,
    scope: Option<&crate::analysis::ScopeInfo>,
) -> Result<CompileOutput, ParseError> {
    let library = crate::loader::LibraryProvenance::of(source);
    let mut lexer = Lexer::new(source, interner);
    let mut tokens = lexer.tokenize();

    // Resolve the qualified escape hatch for cross-module calls: an `Alias::fn`
    // token whose tail is a known function is rewritten to the bare `fn` symbol
    // (types stay namespaced). No-op for single-file programs.
    let functions = crate::analysis::assemble::defined_function_names(source);
    crate::analysis::assemble::resolve_qualified_names(&mut tokens, interner, &functions);

    // Discovery pass for policies (types already discovered)
    let policy_registry = {
        let mut discovery = DiscoveryPass::new(&tokens, interner);
        discovery.run_full().policies
    };

    let codegen_policies = policy_registry.clone();

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let _compile_facts = crate::optimize::facts::scope();
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
    let (parsed, parsed_spans) = (stmts.clone(), parser.stmt_spans().to_vec());

    // The ONE optimization config for this (largo/production) compile: env baseline
    // merged with the file-level `## No <X>` decorators, normalized — threaded to BOTH
    // the optimizer and codegen so `largo build` / `largo emit` honor them exactly like
    // the traced `compile_program_full` path does. This path previously dropped them
    // (bare `from_env()`), silently ignoring every `## No` / `## Require` decorator for
    // real users. Captured here (last use of `parser`) so its interner borrow ends.
    let (opt_config, gate) = crate::optimize::install_decorators(
        crate::optimization::OptimizationConfig::from_env(),
        &parser.program_opt_flags(),
        parser.required_optimizations(),
    );
    let codegen_registry = parser.into_type_registry();
    refuse_undeclared_types(&parsed, &parsed_spans, &codegen_registry, interner)?;

    // Per-file scoping: forbid a file from referencing a name it neither defines
    // nor imports (no transitive leakage). Runs on the freshly-parsed program,
    // before optimization can inline calls away.
    if let Some(scope) = scope {
        crate::analysis::check_scopes(&stmts, scope, interner).map_err(|msg| ParseError {
            kind: crate::error::ParseErrorKind::Custom(msg),
            span: crate::token::Span::default(),
        })?;
    }
    check_as_written(&stmts, interner, AsWritten::Escapes)?;

    let resolved = crate::resolve_program::resolve_program(
        &stmts,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
        interner,
        opt_config.is_on(crate::optimization::Opt::Comptime),
        library.functions(),
    );
    let stmts: Vec<Stmt> = match resolved {
        Some(rw) => rw.to_vec(),
        None => stmts,
    };

    // Run the AST optimizer in the production (largo) compile path so
    // compiled binaries get the same optimizations the test path
    // (compile_program_full) already validates: closed-form / modulus
    // deferral, partial evaluation, constant folding, GVN, LICM,
    // deforestation, CTFE, supercompilation. Previously skipped here, which
    // left the whole optimize layer off for compiled programs. Confirmed a
    // clean win (geomean 1.388x -> 1.735x C-speed, all benchmarks verify
    // correct, no regressions).
    let (stmts, codegen_registry) = lift_program_bindings(
        stmts,
        codegen_registry,
        interner,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
    )?;
    let stmts = crate::optimize::optimize_program(
        stmts,
        &imperative_expr_arena,
        &stmt_arena,
        interner,
        &opt_config,
    );

    // Extract dependencies
    let mut dependencies = extract_dependencies(&stmts, interner)?;

    // FFI: Auto-inject wasm-bindgen dependency if any function is exported for WASM
    let needs_wasm_bindgen = stmts.iter().any(|stmt| {
        if let Stmt::FunctionDef {
            is_exported: true,
            export_target: Some(target),
            ..
        } = stmt
        {
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

    // A call to a name that exists nowhere in the post-prelude program is a
    // check error here — not a rustc E0425 after codegen (`largo check` runs
    // exactly this path).
    crate::analysis::check_unknown_calls(&stmts, interner, &codegen_registry).map_err(|e| {
        ParseError {
            kind: e.to_parse_error_kind(interner),
            span: crate::token::Span::default(),
        }
    })?;

    let type_env = crate::analysis::check_program_located(&stmts, interner, &codegen_registry)
        .map_err(|e| ParseError {
            kind: e.to_parse_error_kind(interner, &stmts),
            span: crate::token::Span::default(),
        })?;
    let rust_code = codegen_program(
        &stmts,
        &codegen_registry,
        &codegen_policies,
        interner,
        &type_env,
        &opt_config,
    );

    // `## Require <opt>`: every demanded optimization must have fired, or the compile
    // fails loudly — the inverse of `## No`, now enforced on the real `largo` path too.
    gate.verify().map_err(|m| ParseError {
        kind: crate::error::ParseErrorKind::Custom(m),
        span: crate::token::Span::default(),
    })?;

    // Universal ABI: Generate C header + bindings if any C exports exist
    let has_c = stmts.iter().any(|stmt| {
        if let Stmt::FunctionDef {
            is_exported: true,
            export_target,
            ..
        } = stmt
        {
            match export_target {
                None => true,
                Some(t) => interner.resolve(*t).eq_ignore_ascii_case("c"),
            }
        } else {
            false
        }
    });

    let c_header = if has_c {
        Some(generate_c_header(
            &stmts,
            "module",
            interner,
            &codegen_registry,
        ))
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
        Some(generate_python_bindings(
            &stmts,
            "module",
            interner,
            &codegen_registry,
        ))
    } else {
        None
    };

    let (typescript_bindings, typescript_types) = if has_c {
        let (js, dts) = generate_typescript_bindings(&stmts, "module", interner, &codegen_registry);
        (Some(js), Some(dts))
    } else {
        (None, None)
    };

    Ok(CompileOutput {
        rust_code,
        dependencies,
        c_header,
        python_bindings,
        typescript_types,
        typescript_bindings,
    })
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
            CompileError::Parse(e) => write!(f, "Parse error: {}", describe_parse_error(e)),
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

/// Encode a LOGOS program (given as source) into CProgram construction source.
///
/// Takes LOGOS source code (with or without `## Main` header) and returns
/// LOGOS source code that constructs the equivalent CProgram data structure.
/// The result defines a variable `prog` of type CProgram.
/// `program` as the language runs it: bare statements become its `## Main`, and the standard library functions it reaches
/// are spliced in, as the tree-walker and a build splice them (`loader::apply_prelude`, pruned to what the program
/// reaches). Every encoding of a user program reads this, so the Core IR holds every function the program calls.
pub fn user_program_source(program: &str) -> String {
    let full = if program.contains("## Main") || program.contains("## To ") {
        program.to_string()
    } else {
        format!("## Main\n{}", program)
    };
    let spliced = crate::loader::apply_prelude(&full);
    crate::loader::prune_unreachable_prelude_from(&full, &spliced)
}

/// `stmts` as every tier runs the program: `resolve_program`'s resolutions (exact or floored division, the real constants,
/// each supertask to the statements that compute its end state, each `Pop` behind its check), read from `assembled`, the
/// text they were parsed from. Every Core IR encoding goes through it, so the self-interpreter and the specializers
/// receive the program the tree-walker runs.
fn resolved_as_every_tier<'a>(
    stmts: Vec<Stmt<'a>>,
    assembled: &str,
    opt_flags: &crate::optimization::OptimizationConfig,
    interner: &mut Interner,
    arenas: crate::ast_map::Arenas<'a>,
) -> Vec<Stmt<'a>> {
    let library = crate::loader::LibraryProvenance::of(assembled);
    let mut run_cfg = crate::optimization::OptimizationConfig::from_env().merged(opt_flags);
    run_cfg.normalize();
    match crate::resolve_program::resolve_program(
        &stmts,
        arenas,
        interner,
        run_cfg.is_on(crate::optimization::Opt::Comptime),
        library.functions(),
    ) {
        Some(resolved) => resolved.to_vec(),
        None => stmts,
    }
}

pub fn encode_program_source(source: &str) -> Result<String, ParseError> {
    let full_source = user_program_source(source);

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(&full_source, &mut interner);
    let tokens = lexer.tokenize();

    let (type_registry, policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };

    // Collect variant constructors before the parser takes ownership of type_registry
    let variant_constructors = variant_ctors(&type_registry, &policy_registry, &interner);

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let _compile_facts = crate::optimize::facts::scope();
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

    let type_shapes = crate::core_ir_scope::type_shapes(&type_registry);
    let mut parser = crate::parser::Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
    );
    let stmts = parser.parse_program()?;
    let opt_flags = parser.program_opt_flags();
    drop(parser);
    let stmts = resolved_as_every_tier(
        stmts,
        &full_source,
        &opt_flags,
        &mut interner,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
    );
    let stmts = crate::core_ir_scope::for_core_ir(
        &stmts,
        &mut interner,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
        &type_shapes,
    );

    let mut functions: Vec<(String, Vec<String>, Vec<String>, String, Vec<&Stmt>)> = Vec::new();
    let mut main_stmts: Vec<&Stmt> = Vec::new();

    for stmt in &stmts {
        if let Stmt::FunctionDef {
            name,
            params,
            body,
            return_type,
            is_native,
            ..
        } = stmt
        {
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

    // Build the funcMap directly (Map of Text to CFunc) with fixed name, plus a parallel
    // `encodedFuncSeq` (Seq of CFunc, declaration order) — the map has no meaning-preserving
    // iteration order, so the wire serializer (which ships a `CProg with funcs …`) needs the seq.
    output.push_str("Let encodedFuncMap be a new Map of Text to CFunc.\n");
    output.push_str("Let encodedFuncSeq be a new Seq of CFunc.\n");

    for (fn_name, params, param_types, ret_type, body) in &functions {
        let body_var = encode_stmt_list_src(
            body,
            &mut counter,
            &mut output,
            &interner,
            &variant_constructors,
        );

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
    let main_var = encode_stmt_list_src(
        &main_stmts,
        &mut counter,
        &mut output,
        &interner,
        &variant_constructors,
    );
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

    let (type_registry, policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };

    let variant_constructors = variant_ctors(&type_registry, &policy_registry, &interner);

    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let _compile_facts = crate::optimize::facts::scope();
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

    let type_shapes = crate::core_ir_scope::type_shapes(&type_registry);
    let mut parser = crate::parser::Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
    );
    let stmts = parser.parse_program()?;
    let opt_flags = parser.program_opt_flags();
    drop(parser);
    let stmts = resolved_as_every_tier(
        stmts,
        &full_source,
        &opt_flags,
        &mut interner,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
    );
    let stmts = crate::core_ir_scope::for_core_ir(
        &stmts,
        &mut interner,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
        &type_shapes,
    );

    let mut functions: Vec<(String, Vec<String>, Vec<String>, String, Vec<&Stmt>)> = Vec::new();
    let mut main_stmts: Vec<&Stmt> = Vec::new();

    for stmt in &stmts {
        if let Stmt::FunctionDef {
            name,
            params,
            body,
            return_type,
            is_native,
            ..
        } = stmt
        {
            if *is_native {
                continue;
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

    output.push_str("Let encodedFuncMap be a new Map of Text to CFunc.\n");

    for (fn_name, params, param_types, ret_type, body) in &functions {
        let body_var = encode_stmt_list_compact(
            body,
            &mut counter,
            &mut output,
            &interner,
            &variant_constructors,
        );

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

    let main_var = encode_stmt_list_compact(
        &main_stmts,
        &mut counter,
        &mut output,
        &interner,
        &variant_constructors,
    );
    output.push_str(&format!("Let encodedMain be {}.\n", main_var));

    Ok(output)
}

/// Returns an inline expression string for simple expressions (no Let variable needed).
/// Returns None for complex expressions that require a Let variable.
fn try_inline_expr(expr: &Expr, interner: &Interner) -> Option<String> {
    if let Some((code, awaits)) = spelled_in_core_ir(expr, interner) {
        return Some(format!("(a new CSpelled with code {} and awaits {})", text_literal(&code), text_literal(awaits)));
    }
    match expr {
        Expr::Literal(lit) => match lit {
            Literal::Number(n) => Some(format!("(a new CInt with value {})", n)),
            Literal::Boolean(b) => Some(format!("(a new CBool with value {})", b)),
            Literal::Text(s) => Some(format!(
                "(a new CText with value {})",
                text_literal(interner.resolve(*s))
            )),
            Literal::Float(f) => {
                let fs = format!("{}", f);
                let fs = if fs.contains('.') {
                    fs
                } else {
                    format!("{}.0", fs)
                };
                Some(format!("(a new CFloat with value {})", fs))
            }
            Literal::Nothing => Some("(a new CNothing)".to_string()),
            other => CoreAtom::of_literal(other).map(|atom| format!("({})", atom.construction())),
        },
        Expr::Identifier(sym) => {
            let name = interner.resolve(*sym);
            match clock_read(name) {
                Some(read) => Some(format!("(a new {read})")),
                None => Some(format!("(a new CVar with name \"{}\")", name)),
            }
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

fn encode_expr_compact(
    expr: &Expr,
    counter: &mut usize,
    output: &mut String,
    interner: &Interner,
    variants: &VCtors,
) -> String {
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
            let op_str = crate::semantics::operator_overloads::core_op_text(*op);
            output.push_str(&format!(
                "Let {} be a new CBinOp with op \"{}\" and left {} and right {}.\n",
                var, op_str, left_var, right_var
            ));
        }
        Expr::Call { function, args } => {
            let fn_name = interner.resolve(*function);
            if let Some(field_names) = variants.get(fn_name).map(|ctor| &ctor.fields) {
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
            output.push_str(&format!(
                "Let {} be a new CLen with target {}.\n",
                var, coll_var
            ));
        }
        Expr::FieldAccess { object, field } => {
            let obj_var = encode_expr_compact(object, counter, output, interner, variants);
            let field_name = interner.resolve(*field);
            output.push_str(&format!(
                "Let {} be a new CMapGet with target {} and key (a new CText with value \"{}\").\n",
                var, obj_var, field_name
            ));
        }
        Expr::NewVariant {
            variant, fields, ..
        } => {
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
                let field_var =
                    encode_expr_compact(field_expr, counter, output, interner, variants);
                output.push_str(&format!("Push {} to {}.\n", field_var, vals_var));
            }
            output.push_str(&format!(
                "Let {} be a new CNewVariant with tag \"{}\" and fnames {} and fvals {}.\n",
                var, variant_name, names_var, vals_var
            ));
        }
        Expr::New {
            type_name,
            type_args,
            init_fields,
        } => {
            let tn = interner.resolve(*type_name);
            let typed = collection_type_text(tn, type_args, interner);
            if tn == "Seq" || tn == "List" {
                output.push_str(&format!("Let {} be a new CNewSeq with elementType {}.\n", var, text_literal(&typed)));
            } else if tn == "Set" {
                output.push_str(&format!("Let {} be a new CNewSet with elementType {}.\n", var, text_literal(&typed)));
            } else {
                let names_var = format!("nvNames_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of Text.\n", names_var));
                let vals_var = format!("nvVals_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of CExpr.\n", vals_var));
                let is_map = tn == "Map" || tn.starts_with("Map ");
                if !is_map {
                    let written: Vec<String> =
                        init_fields.iter().map(|(f, _)| interner.resolve(*f).to_string()).collect();
                    for field in held_fields(tn, &written, variants) {
                        let (fname, field_var) = match field {
                            HeldField::Written(i) => {
                                let field_var =
                                    encode_expr_compact(init_fields[i].1, counter, output, interner, variants);
                                (written[i].as_str(), field_var)
                            }
                            HeldField::Slot((fname, ftype)) => {
                                (fname.as_str(), declared_default_src(ftype, counter, output))
                            }
                        };
                        output.push_str(&format!("Push \"{}\" to {}.\n", fname, names_var));
                        output.push_str(&format!("Push {} to {}.\n", field_var, vals_var));
                    }
                }
                if is_map {
                    output.push_str(&format!(
                        "Let {} be a new CNew with typeName {} and fieldNames {} and fields {}.\n",
                        var,
                        text_literal(&typed),
                        names_var,
                        vals_var
                    ));
                } else if init_fields.is_empty() && variants.struct_fields(tn).is_none() {
                    output.push_str(&format!(
                        "Let {} be a new CNewVariant with tag \"{}\" and fnames {} and fvals {}.\n",
                        var, tn, names_var, vals_var
                    ));
                } else {
                    output.push_str(&format!(
                        "Let {} be a new CNew with typeName \"{}\" and fieldNames {} and fields {}.\n",
                        var, tn, names_var, vals_var
                    ));
                }
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
                            let text = text_literal(interner.resolve(*sym));
                            output.push_str(&format!(
                                "Push a new CLiteralPart with value {} to {}.\n",
                                text, parts_var
                            ));
                        }
                        StringPart::Expr { value, format_spec, .. } => {
                            let pv =
                                encode_expr_compact(value, counter, output, interner, variants);
                            output.push_str(&string_part_src(&pv, *format_spec, &parts_var, interner));
                        }
                    }
                }
                output.push_str(&format!(
                    "Let {} be a new CInterpolatedString with parts {}.\n",
                    var, parts_var
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
            output.push_str(&format!(
                "Let {} be a new CCopy with target {}.\n",
                var, inner_var
            ));
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
                "Let {} be a new COptionSome with inner {}.\n",
                var, inner_var
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
                "Let {} be a new CTuple with items {}.\n",
                var, items_var
            ));
        }
        Expr::Closure { params, body, .. } => {
            let params_var = format!("clp_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of Text.\n", params_var));
            for (sym, _) in params {
                let name = interner.resolve(*sym);
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
                    output.push_str(&format!(
                        "Let {} be a new CReturn with expr {}.\n",
                        ret_var, ret_expr
                    ));
                    output.push_str(&format!("Push {} to {}.\n", ret_var, body_var));
                }
                ClosureBody::Block(stmts) => {
                    for s in spliced_spans(&stmts.iter().collect::<Vec<_>>()) {
                        let sv = encode_stmt_compact(s, counter, output, interner, variants);
                        output.push_str(&format!("Push {} to {}.\n", sv, body_var));
                    }
                }
            }
            let free = closure_captures(expr, interner);
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
        Expr::Slice {
            collection,
            start,
            end,
        } => {
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
                var,
                code_str.replace('\"', "\\\"")
            ));
        }
        _ => return encode_expr_src(expr, counter, output, interner, variants),
    }

    var
}

fn encode_stmt_compact(
    stmt: &Stmt,
    counter: &mut usize,
    output: &mut String,
    interner: &Interner,
    variants: &VCtors,
) -> String {
    let Some((root, path)) = write_place(stmt, interner) else {
        return encode_written_stmt_compact(stmt, counter, output, interner, variants);
    };
    let inner = encode_written_stmt_compact(stmt, counter, output, interner, variants);
    at_path_src(&root, &path, &inner, counter, output)
}

/// One statement as compact construction text, a write through a field path as the bare write (its place wraps it in
/// [`encode_stmt_compact`]).
fn encode_written_stmt_compact(
    stmt: &Stmt,
    counter: &mut usize,
    output: &mut String,
    interner: &Interner,
    variants: &VCtors,
) -> String {
    let var = format!("s_{}", *counter);
    *counter += 1;

    match stmt {
        Stmt::Let {
            var: name, value, ..
        } => {
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
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            let cond_var = encode_expr_compact(cond, counter, output, interner, variants);
            let then_stmts: Vec<&Stmt> = then_block.iter().collect();
            let then_var =
                encode_stmt_list_compact(&then_stmts, counter, output, interner, variants);
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
            let body_var =
                encode_stmt_list_compact(&body_stmts, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CWhile with cond {} and body {}.\n",
                var, cond_var, body_var
            ));
        }
        Stmt::Return { value } => {
            if let Some(val) = value {
                let expr_var = encode_expr_compact(val, counter, output, interner, variants);
                output.push_str(&format!(
                    "Let {} be a new CReturn with expr {}.\n",
                    var, expr_var
                ));
            } else {
                output.push_str(&format!("Let {} be a new CReturn with expr (a new CNothing).\n", var));
            }
        }
        Stmt::Show { object, .. } => {
            let expr_var = encode_expr_compact(object, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CShow with expr {}.\n",
                var, expr_var
            ));
        }
        Stmt::Repeat {
            pattern,
            iterable,
            body,
        } => {
            let (var_str, components) = repeat_binders(pattern, interner);
            let coll_var = encode_expr_compact(iterable, counter, output, interner, variants);
            let body_stmts: Vec<&Stmt> = body.iter().collect();
            let body_var = encode_repeat_body_compact(
                &components,
                &var_str,
                &body_stmts,
                counter,
                output,
                interner,
                variants,
            );
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
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            let target_str = extract_ident_name(collection, interner);
            let idx_var = encode_expr_compact(index, counter, output, interner, variants);
            let val_var = encode_expr_compact(value, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CSetIdx with target \"{}\" and idx {} and val {}.\n",
                var, target_str, idx_var, val_var
            ));
        }
        Stmt::SetField {
            object,
            field,
            value,
        } => {
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
                    let body_var =
                        encode_stmt_list_compact(&body_stmts, counter, output, interner, variants);
                    let arm_var = format!("arm_{}", *counter);
                    *counter += 1;
                    output.push_str(&format!(
                        "Let {} be a new CWhen with variantName \"{}\" and bindings {} and body {}.\n",
                        arm_var, vname, bindings_var, body_var
                    ));
                    output.push_str(&format!("Push {} to {}.\n", arm_var, arms_var));
                } else {
                    let body_stmts: Vec<&Stmt> = arm.body.iter().collect();
                    let body_var =
                        encode_stmt_list_compact(&body_stmts, counter, output, interner, variants);
                    let arm_var = format!("arm_{}", *counter);
                    *counter += 1;
                    output.push_str(&format!(
                        "Let {} be a new COtherwise with body {}.\n",
                        arm_var, body_var
                    ));
                    output.push_str(&format!("Push {} to {}.\n", arm_var, arms_var));
                }
            }
            for (variant, report) in uncovered_variants(arms, interner, variants) {
                push_unhandled_arm_src(&variant, &report, &arms_var, counter, output);
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

fn encode_stmt_list_compact(
    stmts: &[&Stmt],
    counter: &mut usize,
    output: &mut String,
    interner: &Interner,
    variants: &VCtors,
) -> String {
    encode_repeat_body_compact(&[], "", stmts, counter, output, interner, variants)
}

/// [`encode_stmt_list_compact`] for a loop body: the statements that bind a tuple pattern's components (see
/// [`repeat_binders`]) come first.
fn encode_repeat_body_compact(
    components: &[String],
    loop_var: &str,
    stmts: &[&Stmt],
    counter: &mut usize,
    output: &mut String,
    interner: &Interner,
    variants: &VCtors,
) -> String {
    let list_var = format!("stmts_{}", *counter);
    *counter += 1;
    output.push_str(&format!("Let {} be a new Seq of CStmt.\n", list_var));
    for binder in tuple_binder_stmts(components, loop_var) {
        output.push_str(&format!("Push {} to {}.\n", binder, list_var));
    }
    for s in spliced_spans(stmts) {
        let sv = encode_stmt_compact(s, counter, output, interner, variants);
        output.push_str(&format!("Push {} to {}.\n", sv, list_var));
    }
    list_var
}

/// `stmts` with each `During <span>:`, `Splice` and `Reversibly:` replaced by its body, which runs where it stands.
fn spliced_spans<'s>(stmts: &[&'s Stmt<'s>]) -> Vec<&'s Stmt<'s>> {
    let mut out = Vec::with_capacity(stmts.len());
    for s in stmts {
        match s {
            Stmt::During { body, .. } | Stmt::Splice { body } | Stmt::Reversibly { body } => {
                out.extend(spliced_spans(&body.iter().collect::<Vec<_>>()))
            }
            _ => out.push(*s),
        }
    }
    out
}

/// How a `Repeat` pattern binds: the Core IR loop variable, and for a tuple pattern the names its components bind in
/// order. A tuple pattern's loop variable holds the whole element (a map iterates as `(key, value)`), and the body
/// opens by binding each component from it, so every name the pattern wrote is bound.
fn repeat_binders(pattern: &Pattern, interner: &Interner) -> (String, Vec<String>) {
    match pattern {
        Pattern::Identifier(sym) => (interner.resolve(*sym).to_string(), Vec::new()),
        Pattern::Tuple(syms) => {
            let names: Vec<String> = syms.iter().map(|s| interner.resolve(*s).to_string()).collect();
            (format!("{}__pair", names.join("_")), names)
        }
    }
}

/// The Core IR statements, as construction text, that bind a tuple pattern's components from its loop variable.
fn tuple_binder_stmts(components: &[String], loop_var: &str) -> Vec<String> {
    components
        .iter()
        .enumerate()
        .map(|(i, name)| {
            format!(
                "(a new CLet with name \"{}\" and expr (a new CIndex with coll (a new CVar with name \"{}\") and idx (a new CInt with value {})))",
                name,
                loop_var,
                i + 1
            )
        })
        .collect()
}

/// The names a closure captures, sorted so every encoding of one closure is byte-identical: the engines' own
/// capture set ([`crate::interpreter::Interpreter::free_vars_in_closure`]), so the Core IR captures exactly what the
/// tree-walker and the VM capture.
fn closure_captures(expr: &Expr, interner: &Interner) -> Vec<String> {
    let Expr::Closure { params, body, .. } = expr else {
        return Vec::new();
    };
    let mut free: Vec<String> = crate::interpreter::Interpreter::free_vars_in_closure(params, body)
        .into_iter()
        .map(|sym| interner.resolve(sym).to_string())
        .collect();
    free.sort();
    free.dedup();
    free
}

fn encode_expr_src(
    expr: &Expr,
    counter: &mut usize,
    output: &mut String,
    interner: &Interner,
    variants: &VCtors,
) -> String {
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
                output.push_str(&format!(
                    "Let {} be a new CText with value {}.\n",
                    var,
                    text_literal(interner.resolve(*s))
                ));
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
            Literal::Nothing => {
                output.push_str(&format!("Let {} be a new CNothing.\n", var));
            }
            Literal::Duration(_)
            | Literal::Char(_)
            | Literal::Date(_)
            | Literal::Moment(_)
            | Literal::Span { .. }
            | Literal::Time(_) => {
                let atom = CoreAtom::of_literal(lit).expect("a temporal or character literal is an atom");
                output.push_str(&format!("Let {var} be {}.\n", atom.construction()));
            }
        },
        Expr::Identifier(sym) => {
            let name = interner.resolve(*sym);
            match clock_read(name) {
                Some(read) => output.push_str(&format!("Let {} be a new {}.\n", var, read)),
                None => output.push_str(&format!("Let {} be a new CVar with name \"{}\".\n", var, name)),
            }
        }
        Expr::BinaryOp { op, left, right } => {
            let left_var = encode_expr_src(left, counter, output, interner, variants);
            let right_var = encode_expr_src(right, counter, output, interner, variants);
            let op_str = crate::semantics::operator_overloads::core_op_text(*op);
            output.push_str(&format!(
                "Let {} be a new CBinOp with op \"{}\" and left {} and right {}.\n",
                var, op_str, left_var, right_var
            ));
        }
        Expr::Not { operand } => {
            let inner_var = encode_expr_src(operand, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CNot with inner {}.\n",
                var, inner_var
            ));
        }
        Expr::Call { function, args } => {
            let fn_name = interner.resolve(*function);
            if let Some(field_names) = variants.get(fn_name).map(|ctor| &ctor.fields) {
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
            output.push_str(&format!(
                "Let {} be a new CLen with target {}.\n",
                var, coll_var
            ));
        }
        Expr::FieldAccess { object, field } => {
            let obj_var = encode_expr_src(object, counter, output, interner, variants);
            let field_name = interner.resolve(*field);
            let key_var = format!("e_{}", *counter);
            *counter += 1;
            output.push_str(&format!(
                "Let {} be a new CText with value \"{}\".\n",
                key_var, field_name
            ));
            output.push_str(&format!(
                "Let {} be a new CMapGet with target {} and key {}.\n",
                var, obj_var, key_var
            ));
        }
        Expr::NewVariant {
            variant, fields, ..
        } => {
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
        Expr::New {
            type_name,
            type_args,
            init_fields,
        } => {
            let tn = interner.resolve(*type_name);
            let typed = collection_type_text(tn, type_args, interner);
            if tn == "Seq" || tn == "List" {
                output.push_str(&format!("Let {} be a new CNewSeq with elementType {}.\n", var, text_literal(&typed)));
            } else if tn == "Set" {
                output.push_str(&format!("Let {} be a new CNewSet with elementType {}.\n", var, text_literal(&typed)));
            } else if tn == "Map" || tn.starts_with("Map ") {
                // Empty map creation — encode as CNew with "Map" type
                let names_var = format!("nvNames_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of Text.\n", names_var));
                let vals_var = format!("nvVals_{}", *counter);
                *counter += 1;
                output.push_str(&format!("Let {} be a new Seq of CExpr.\n", vals_var));
                output.push_str(&format!(
                    "Let {} be a new CNew with typeName {} and fieldNames {} and fields {}.\n",
                    var,
                    text_literal(&typed),
                    names_var,
                    vals_var
                ));
            } else if init_fields.is_empty() && variants.struct_fields(tn).is_none() {
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
                let written: Vec<String> =
                    init_fields.iter().map(|(f, _)| interner.resolve(*f).to_string()).collect();
                for field in held_fields(tn, &written, variants) {
                    let (fname, field_var) = match field {
                        HeldField::Written(i) => {
                            let field_var = encode_expr_src(init_fields[i].1, counter, output, interner, variants);
                            (written[i].as_str(), field_var)
                        }
                        HeldField::Slot((fname, ftype)) => (fname.as_str(), declared_default_src(ftype, counter, output)),
                    };
                    output.push_str(&format!("Push \"{}\" to {}.\n", fname, names_var));
                    output.push_str(&format!("Push {} to {}.\n", field_var, vals_var));
                }
                output.push_str(&format!(
                    "Let {} be a new CNew with typeName \"{}\" and fieldNames {} and fields {}.\n",
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
                            let text = text_literal(interner.resolve(*sym));
                            output.push_str(&format!(
                                "Push a new CLiteralPart with value {} to {}.\n",
                                text, parts_var
                            ));
                        }
                        StringPart::Expr { value, format_spec, .. } => {
                            let pv = encode_expr_src(value, counter, output, interner, variants);
                            output.push_str(&string_part_src(&pv, *format_spec, &parts_var, interner));
                        }
                    }
                }
                output.push_str(&format!(
                    "Let {} be a new CInterpolatedString with parts {}.\n",
                    var, parts_var
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
        Expr::Slice {
            collection,
            start,
            end,
        } => {
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
            output.push_str(&format!(
                "Let {} be a new CCopy with target {}.\n",
                var, inner_var
            ));
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
            for (sym, _) in params {
                let name = interner.resolve(*sym);
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
                    output.push_str(&format!(
                        "Let {} be a new CReturn with expr {}.\n",
                        ret_var, ret_expr
                    ));
                    output.push_str(&format!("Push {} to {}.\n", ret_var, body_var));
                }
                ClosureBody::Block(stmts) => {
                    for s in stmts.iter() {
                        for sv in encode_stmts_src(s, counter, output, interner, variants) {
                            output.push_str(&format!("Push {} to {}.\n", sv, body_var));
                        }
                    }
                }
            }
            let free = closure_captures(expr, interner);
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
                var,
                code_str.replace('\"', "\\\"")
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
        Expr::View(_) | Expr::Dom(_) => spelled_src(expr, &var, output, interner),
    }

    var
}

fn encode_stmt_src(
    stmt: &Stmt,
    counter: &mut usize,
    output: &mut String,
    interner: &Interner,
    variants: &VCtors,
) -> String {
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
        Stmt::Swap { a, b } => {
            // No dedicated CSwap in the PE self-representation; encode the observationally-equivalent
            // temp-mediated swap as an always-taken CIf, whose block holds the temp: a fresh
            // `tmp := a; a := b; b := tmp`, all in existing CLet/CSet/CVar nodes.
            let (an, bn) = (interner.resolve(*a), interner.resolve(*b));
            let tmp = swap_temp(an, bn);
            output.push_str(&format!("Let swapBlock_{var} be a new Seq of CStmt.\n"));
            output.push_str(&format!("Let swapVarA_{var} be a new CVar with name \"{an}\".\n"));
            output.push_str(&format!("Let swapS1_{var} be a new CLet with name \"{tmp}\" and expr swapVarA_{var}.\n"));
            output.push_str(&format!("Push swapS1_{var} to swapBlock_{var}.\n"));
            output.push_str(&format!("Let swapVarB_{var} be a new CVar with name \"{bn}\".\n"));
            output.push_str(&format!("Let swapS2_{var} be a new CSet with name \"{an}\" and expr swapVarB_{var}.\n"));
            output.push_str(&format!("Push swapS2_{var} to swapBlock_{var}.\n"));
            output.push_str(&format!("Let swapVarT_{var} be a new CVar with name \"{tmp}\".\n"));
            output.push_str(&format!("Let swapS3_{var} be a new CSet with name \"{bn}\" and expr swapVarT_{var}.\n"));
            output.push_str(&format!("Push swapS3_{var} to swapBlock_{var}.\n"));
            output.push_str(&format!("Let swapCond_{var} be a new CBool with value true.\n"));
            output.push_str(&format!("Let swapEmpty_{var} be a new Seq of CStmt.\n"));
            output.push_str(&format!(
                "Let {var} be a new CIf with cond swapCond_{var} and thenBlock swapBlock_{var} and elseBlock swapEmpty_{var}.\n"
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
        Stmt::Return { value } => {
            if let Some(expr) = value {
                let expr_var = encode_expr_src(expr, counter, output, interner, variants);
                output.push_str(&format!("Let {} be a new CReturn with expr {}.\n", var, expr_var));
            } else {
                output.push_str(&format!("Let {} be a new CReturn with expr (a new CNothing).\n", var));
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
            let target_name = extract_ident_name(object, interner);
            let field_name = interner.resolve(*field);
            let val_var = encode_expr_src(value, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CSetField with target \"{}\" and field \"{}\" and val {}.\n",
                var, target_name, field_name, val_var
            ));
        }
        Stmt::Pop { collection, into } => {
            let coll_name = extract_ident_name(collection, interner);
            match into {
                Some(into) => output.push_str(&format!(
                    "Let {} be a new CPopInto with target \"{}\" and into \"{}\".\n",
                    var,
                    coll_name,
                    interner.resolve(*into)
                )),
                None => output.push_str(&format!("Let {} be a new CPop with target \"{}\".\n", var, coll_name)),
            }
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
        Stmt::RuntimeAssert { condition, hard, dim: _, reason } => {
            let cond_var = encode_expr_src(condition, counter, output, interner, variants);
            let msg_var = match reason {
                Some(r) => encode_expr_src(r, counter, output, interner, variants),
                None => {
                    let msg_var = format!("e_{}", *counter);
                    *counter += 1;
                    output.push_str(&format!("Let {} be a new CText with value \"Assertion failed\".\n", msg_var));
                    msg_var
                }
            };
            // `Require that` (hard) encodes as a distinct CStmt variant `CHardAssert`
            // (NOT `CRequire`, which is the `## Requires` dependency directive) so the
            // self-encoding round-trip preserves the enforced/dev distinction.
            let variant = if *hard { "CHardAssert" } else { "CRuntimeAssert" };
            output.push_str(&format!(
                "Let {} be a new {} with cond {} and msg {}.\n",
                var, variant, cond_var, msg_var
            ));
        }
        Stmt::Fail { reason } => {
            let reason_var = encode_expr_src(reason, counter, output, interner, variants);
            output.push_str(&format!("Let {} be a new CFail with reason {}.\n", var, reason_var));
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
        Stmt::Check { .. } => {
            let (subject, object, capability, rule, source) = check_parts(stmt, interner).expect("a Check");
            let alts_var = format!("alts_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of CPolicyAlt.\n", alts_var));
            for (type_name, node) in variants.policies.get(&(rule.clone(), capability)).into_iter().flatten() {
                let condition = policy_src(node, counter, output);
                output.push_str(&format!(
                    "Push a new CPolicyCase with typeName {} and condition {} to {}.\n",
                    text_literal(type_name),
                    condition,
                    alts_var
                ));
            }
            output.push_str(&format!(
                "Let {} be a new CCheckPolicy with subject {} and object {} and capability {} and rule {} and alternatives {} and source {}.\n",
                var,
                text_literal(&subject),
                text_literal(&object),
                capability,
                text_literal(&rule),
                alts_var,
                text_literal(&source)
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
        // The IDEMPOTENT sibling of `Merge` — encoded as a distinct `CJoin` core node so the self-hosted PE
        // round-trips `Converge` back to `Converge` (an idempotent join), NOT to `Merge` (which double-counts).
        Stmt::JoinCrdt { source, target } => {
            let source_var = encode_expr_src(source, counter, output, interner, variants);
            let target_name = extract_ident_name(target, interner);
            output.push_str(&format!(
                "Let {} be a new CJoin with target \"{}\" and other {}.\n",
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
        Stmt::ResolveConflict { object, field, value } => {
            let value_var = encode_expr_src(value, counter, output, interner, variants);
            let target_name =
                format!("{}'s {}", extract_ident_name(object, interner), interner.resolve(*field));
            output.push_str(&format!(
                "Let {} be a new CResolveTo with target \"{}\" and value {}.\n",
                var, target_name, value_var
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
                for sv in encode_stmts_src(stmt, counter, output, interner, variants) {
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
                for sv in encode_stmts_src(stmt, counter, output, interner, variants) {
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
        // `Close ch` and `Await h` are OPAQUE deterministic-scheduler concurrency statements the
        // partial-evaluator does not model beyond preserving them (the self-interpreter treats channel
        // close / task join as scheduling no-ops that never alter a specialized value). To keep the
        // Futamura projection COMPLETE — never silently dropping an executable statement, the invariant
        // this exhaustive match guards — each rides an existing catalog node that carries its read-set:
        // `Close ch` is a single-handle terminal control like `CStopTask`; `Await h into r` reads a handle
        // and binds a target exactly like `CReceivePipe`. Their dedicated on-chip/AOT lowering lives in the
        // real backends (VM `Op::ChanClose`/`Op::TaskAwait`, `codegen_c`), not in this meta-circular tier.
        Stmt::CloseChannel { channel } => {
            let chan_var = encode_expr_src(channel, counter, output, interner, variants);
            output.push_str(&format!(
                "Let {} be a new CStopTask with handle {}.\n",
                var, chan_var
            ));
        }
        Stmt::AwaitTask { handle, result } => {
            let chan_name = match handle {
                Expr::Identifier(sym) => interner.resolve(*sym).to_string(),
                _ => "task".to_string(),
            };
            let target_name = result.map(|r| interner.resolve(r).to_string()).unwrap_or_else(|| "_await".to_string());
            output.push_str(&format!(
                "Let {} be a new CReceivePipe with chan \"{}\" and target \"{}\".\n",
                var, chan_name, target_name
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
                            for sv in encode_stmts_src(stmt, counter, output, interner, variants) {
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
                            for sv in encode_stmts_src(stmt, counter, output, interner, variants) {
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
        // A `Serve` is a live-networking terminal, not a partial-evaluation target;
        // encode its port like a Listen address so no variable read is dropped.
        Stmt::Serve { port, handler: _ } => {
            let addr_var = encode_expr_src(port, counter, output, interner, variants);
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
                for sv in encode_stmts_src(stmt, counter, output, interner, variants) {
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
        | Stmt::Ui(_)
        | Stmt::Fetch { .. }
        | Stmt::GivenFetch { .. }
        | Stmt::ServeFetches
        | Stmt::Replay { .. }
        | Stmt::ComponentDef(_)
        | Stmt::DirectiveDef { .. }
        | Stmt::RouteDef { .. }
        | Stmt::MiddlewareDef { .. }
        | Stmt::RedirectDef { .. }
        | Stmt::Theorem(_)
        | Stmt::Definition(_)
        | Stmt::Axiom(_)
        | Stmt::Theory(_)
        // A failure-site marker only ever appears inside an LGX action/handler body — the SAME
        // `FrontendSurface` corpus `Stmt::ComponentDef`/`Stmt::Ui` are already, correctly, excluded
        // from here: a program containing one is not a Jones corpus program at all
        // (`jones_whole_language_lock.rs`), so this arm is defensive, not reachable in practice.
        | Stmt::Site(_) => {
            return String::new();
        }
        // `## Test` blocks (and the test-only statements the parser confines to
        // their bodies) are declarations to the projection: the runner executes
        // them on the interpreter tier; a specialized program never runs them.
        Stmt::TestDef { .. }
        | Stmt::Expect { .. }
        | Stmt::ExpectFail { .. }
        | Stmt::ExpectOutput { .. }
        | Stmt::AdvanceTime { .. }
        | Stmt::GivenFile { .. }
        // Observation emission is not yet part of the projection (Wave 7/13). `During`, `Splice` and `Reversibly:`
        // run their bodies in the block they stand in, which every list walk splices them into, so none has an
        // encoding of its own.
        | Stmt::Observe { .. }
        | Stmt::During { .. }
        | Stmt::Splice { .. }
        | Stmt::Reversibly { .. }
        | Stmt::ForAll { .. } => {
            return String::new();
        }
    }

    var
}

/// An arm for an `Inspect`'s `variant` that no arm of the program covers, as construction text: it stops with the
/// language's `report` (see [`uncovered_variants`]). Pushed onto `arms_var`.
fn push_unhandled_arm_src(variant: &str, report: &str, arms_var: &str, counter: &mut usize, output: &mut String) {
    let body_var = format!("stmtList_{}", *counter);
    *counter += 1;
    output.push_str(&format!("Let {} be a new Seq of CStmt.\n", body_var));
    output.push_str(&format!(
        "Push (a new CFail with reason (a new CText with value \"{}\")) to {}.\n",
        report, body_var
    ));
    let arm_var = format!("arm_{}", *counter);
    *counter += 1;
    output.push_str(&format!(
        "Let {} be a new CWhen with variantName \"{}\" and bindings (a new Seq of Text) and body {}.\n",
        arm_var, variant, body_var
    ));
    output.push_str(&format!("Push {} to {}.\n", arm_var, arms_var));
}

fn encode_stmts_src(
    stmt: &Stmt,
    counter: &mut usize,
    output: &mut String,
    interner: &Interner,
    variants: &VCtors,
) -> Vec<String> {
    match stmt {
        Stmt::Inspect { target, arms, .. } => {
            let target_var = encode_expr_src(target, counter, output, interner, variants);
            let arms_var = format!("arms_{}", *counter);
            *counter += 1;
            output.push_str(&format!("Let {} be a new Seq of CMatchArm.\n", arms_var));
            for arm in arms {
                let body_stmts: Vec<&Stmt> = arm.body.iter().collect();
                let body_var = encode_stmt_list_src(&body_stmts, counter, output, interner, variants);
                let arm_var = format!("arm_{}", *counter);
                *counter += 1;
                match arm.variant {
                    Some(variant_sym) => {
                        let bindings_var = format!("bindings_{}", *counter);
                        *counter += 1;
                        output.push_str(&format!("Let {} be a new Seq of Text.\n", bindings_var));
                        for (_, binding_name) in &arm.bindings {
                            output.push_str(&format!(
                                "Push \"{}\" to {}.\n",
                                interner.resolve(*binding_name),
                                bindings_var
                            ));
                        }
                        output.push_str(&format!(
                            "Let {} be a new CWhen with variantName \"{}\" and bindings {} and body {}.\n",
                            arm_var,
                            interner.resolve(variant_sym),
                            bindings_var,
                            body_var
                        ));
                    }
                    None => output.push_str(&format!(
                        "Let {} be a new COtherwise with body {}.\n",
                        arm_var, body_var
                    )),
                }
                output.push_str(&format!("Push {} to {}.\n", arm_var, arms_var));
            }
            for (variant, report) in uncovered_variants(arms, interner, variants) {
                push_unhandled_arm_src(&variant, &report, &arms_var, counter, output);
            }
            let inspect_var = format!("s_{}", *counter);
            *counter += 1;
            output.push_str(&format!(
                "Let {} be a new CInspect with target {} and arms {}.\n",
                inspect_var, target_var, arms_var
            ));
            vec![inspect_var]
        }
        Stmt::Repeat {
            pattern,
            iterable,
            body,
            ..
        } => {
            let (loop_var_name, components) = repeat_binders(pattern, interner);

            // Range-based repeat: encode as CRepeatRange
            if let Expr::Range { start, end } = iterable {
                let start_var = encode_expr_src(start, counter, output, interner, variants);
                let end_var = encode_expr_src(end, counter, output, interner, variants);
                let body_stmts: Vec<&Stmt> = body.iter().collect();
                let body_var = encode_repeat_body_src(
                    &components,
                    &loop_var_name,
                    &body_stmts,
                    counter,
                    output,
                    interner,
                    variants,
                );
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
            let body_var = encode_repeat_body_src(
                &components,
                &loop_var_name,
                &body_stmts,
                counter,
                output,
                interner,
                variants,
            );
            let rep = format!("s_{}", *counter);
            *counter += 1;
            output.push_str(&format!(
                "Let {} be a new CRepeat with var \"{}\" and coll {} and body {}.\n",
                rep, loop_var_name, coll_var, body_var
            ));
            vec![rep]
        }
        // `During <span>:` only names a span, and a `Splice` (a desugaring's statements) and a `Reversibly:` block
        // add no scope: each body runs where it stands, in the enclosing block.
        Stmt::During { body, .. } | Stmt::Splice { body } | Stmt::Reversibly { body } => {
            let mut vars = Vec::new();
            for inner in body.iter() {
                vars.extend(encode_stmts_src(inner, counter, output, interner, variants));
            }
            vars
        }
        // A write through a field path (`Push x to b's items`, `Increase c's points by 1`) is the write wrapped in
        // the place it writes: `CAtPath` (see `write_place`).
        _ if write_place(stmt, interner).is_some() => {
            let (root, path) = write_place(stmt, interner).expect("a field path");
            let inner = encode_stmt_src(stmt, counter, output, interner, variants);
            vec![at_path_src(&root, &path, &inner, counter, output)]
        }
        // A merge into a whole struct (`Merge b into a`) — force the struct's root variable dynamic first, so the PE
        // doesn't fold a register it then mutates.
        Stmt::MergeCrdt { target: object, .. } | Stmt::JoinCrdt { target: object, .. } => {
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

fn encode_stmt_list_src(
    stmts: &[&Stmt],
    counter: &mut usize,
    output: &mut String,
    interner: &Interner,
    variants: &VCtors,
) -> String {
    encode_repeat_body_src(&[], "", stmts, counter, output, interner, variants)
}

/// [`encode_stmt_list_src`] for a loop body: the statements that bind a tuple pattern's components (see
/// [`repeat_binders`]) come first.
fn encode_repeat_body_src(
    components: &[String],
    loop_var: &str,
    stmts: &[&Stmt],
    counter: &mut usize,
    output: &mut String,
    interner: &Interner,
    variants: &VCtors,
) -> String {
    let list_var = format!("stmtList_{}", *counter);
    *counter += 1;
    output.push_str(&format!("Let {} be a new Seq of CStmt.\n", list_var));
    for binder in tuple_binder_stmts(components, loop_var) {
        output.push_str(&format!("Push {} to {}.\n", binder, list_var));
    }

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
            format!(
                "{}'s {}",
                extract_ident_name(object, interner),
                interner.resolve(*field)
            )
        }
        _ => "unknown".to_string(),
    }
}

/// The clock read the name `name` is, whatever binds it: the language reads `now` and `today` as the clock even where a
/// binding shadows them.
fn clock_read(name: &str) -> Option<&'static str> {
    match name {
        "now" => Some("CTimeNow"),
        "today" => Some("CDateToday"),
        _ => None,
    }
}

/// Where a write statement writes when that is a field path — `Push x to b's items`, `Set p's pos's x to 3`,
/// `Increase c's points by 1` — as its root variable and the fields from the root to the place; `None` for a write to
/// a plain variable, and for every other statement. The Core IR wraps such a write in `CAtPath`: the write keeps the
/// place's spelling (`b's items`) as its target, the self-interpreter binds the place's value under that spelling,
/// runs the write and writes the value back along the path, and a specializer forces the root dynamic. Computed here
/// so the self-interpreter — which must compile to statically-typed Rust for self-application — never parses a path.
fn write_place(stmt: &Stmt, interner: &Interner) -> Option<(String, Vec<String>)> {
    let (target, last) = match stmt {
        Stmt::Push { collection, .. }
        | Stmt::Pop { collection, .. }
        | Stmt::Add { collection, .. }
        | Stmt::Remove { collection, .. }
        | Stmt::SetIndex { collection, .. } => (*collection, None),
        Stmt::AppendToSequence { sequence, .. } => (*sequence, None),
        Stmt::MergeCrdt { target, .. } | Stmt::JoinCrdt { target, .. } => (*target, None),
        Stmt::SetField { object, .. } => (*object, None),
        Stmt::IncreaseCrdt { object, field, .. }
        | Stmt::DecreaseCrdt { object, field, .. }
        | Stmt::ResolveConflict { object, field, .. } => (*object, Some(*field)),
        _ => return None,
    };
    let (root, mut path) = field_path(target, interner)?;
    path.extend(last.map(|f| interner.resolve(f).to_string()));
    (!path.is_empty()).then_some((root, path))
}

/// A variable or a chain of field reads from one, as the variable and the fields in order: `p's pos's x` is `p` and
/// `[pos, x]`.
fn field_path(expr: &Expr, interner: &Interner) -> Option<(String, Vec<String>)> {
    match expr {
        Expr::Identifier(sym) => Some((interner.resolve(*sym).to_string(), Vec::new())),
        Expr::FieldAccess { object, field } => {
            let (root, mut path) = field_path(object, interner)?;
            path.push(interner.resolve(*field).to_string());
            Some((root, path))
        }
        _ => None,
    }
}

/// Emit a `CForceDynamic` node binding `base` and return its node var, prepended before a merge into a whole
/// struct so the PE invalidates the binding before it can fold it.
fn emit_force_dynamic_named(base: &str, counter: &mut usize, output: &mut String) -> String {
    let v = format!("s_{}", *counter);
    *counter += 1;
    output.push_str(&format!(
        "Let {} be a new CForceDynamic with name \"{}\".\n",
        v, base
    ));
    v
}

/// `CForceDynamic` for a merge into a whole struct (`Merge b into a`): the whole struct is mutated, so its root
/// variable is always forced dynamic.
fn emit_force_dynamic_struct(
    object: &Expr,
    counter: &mut usize,
    output: &mut String,
    interner: &Interner,
) -> Option<String> {
    match object {
        Expr::Identifier(sym) => Some(emit_force_dynamic_named(interner.resolve(*sym), counter, output)),
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
pub fn projection1_source(
    _core_types: &str,
    _interpreter: &str,
    program: &str,
) -> Result<String, String> {
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
    let _compile_facts = crate::optimize::facts::scope();
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

    let mut parser = crate::parser::Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
    );
    let stmts = parser
        .parse_program()
        .map_err(|e| format!("Parse error: {}", describe_parse_error(&e)))?;

    // Honor `## No <opt>` decorators here too — captured before the optimizer reuses
    // `&mut interner` (a bare `from_env()` would drop them).
    let opt_config = {
        let mut c =
            crate::optimization::OptimizationConfig::from_env().merged(&parser.program_opt_flags());
        c.normalize();
        c
    };
    let resolved = crate::resolve_program::resolve_program(
        &stmts,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
        &mut interner,
        opt_config.is_on(crate::optimization::Opt::Comptime),
        &crate::loader::library_owned_functions(&full_source),
    );
    let stmts: Vec<Stmt> = match resolved {
        Some(rw) => rw.to_vec(),
        None => stmts,
    };

    // First Futamura Projection: PE(interpreter, program) = compiled_program.
    // Use the projection-safe optimizer: fold + propagate + PE + CTFE.
    // This preserves control-flow structure (If/While) while still folding
    // constants, propagating values, and specializing function calls.
    let optimized = crate::optimize::optimize_for_projection(
        stmts,
        &imperative_expr_arena,
        &stmt_arena,
        &mut interner,
        &opt_config,
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
        Stmt::ComponentDef(def) => decompile_component(def, interner, out),
        Stmt::DirectiveDef {
            name,
            params,
            body,
            removed,
        } => {
            out.push_str(&format!(
                "## Directive {} {}:\n",
                interner.resolve(*name),
                decompile_params(params, interner)
            ));
            for inner in body.iter() {
                decompile_stmt(inner, interner, out, 1);
            }
            if let Some(r) = removed {
                out.push_str("    When removed:\n");
                for inner in r.iter() {
                    decompile_stmt(inner, interner, out, 2);
                }
            }
        }
        Stmt::RouteDef {
            method,
            path,
            params,
            return_type,
            body,
            ..
        } => {
            let ret = return_type
                .map(|t| format!(" -> {}", decompile_type_expr(t, interner)))
                .unwrap_or_default();
            let ps = decompile_params(params, interner);
            out.push_str(&format!(
                "## Route {} {}{}{}:\n",
                interner.resolve(*method),
                interner.resolve(*path),
                if ps.is_empty() {
                    String::new()
                } else {
                    format!(" {ps}")
                },
                ret
            ));
            match &**body {
                logicaffeine_language::ast::component::RouteBody::Statements(b) => {
                    for inner in b.iter() {
                        decompile_stmt(inner, interner, out, 1);
                    }
                }
                logicaffeine_language::ast::component::RouteBody::View(view) => {
                    for line in decompile_view(view, interner).lines() {
                        out.push_str(&format!("    {line}\n"));
                    }
                }
            }
        }
        Stmt::MiddlewareDef { param, body } => {
            out.push_str(&format!(
                "## Before each request ({}: {}) -> Option of Response:\n",
                interner.resolve(param.0),
                decompile_type_expr(param.1, interner)
            ));
            for inner in body.iter() {
                decompile_stmt(inner, interner, out, 1);
            }
        }
        Stmt::RedirectDef { from, to } => {
            out.push_str(&format!(
                "## Redirect {} -> {}.\n",
                interner.resolve(*from),
                interner.resolve(*to)
            ));
        }
        Stmt::FunctionDef {
            name,
            params,
            body,
            return_type,
            ..
        } => {
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
            out.push_str(&format!(
                "{}## To {} ({}){}:\n",
                pad,
                fn_name,
                param_strs.join(", "),
                ret_str
            ));
            for s in body.iter() {
                decompile_stmt(s, interner, out, indent + 1);
            }
        }
        Stmt::Let {
            var,
            ty,
            value,
            mutable,
        } => {
            let name = interner.resolve(*var);
            let declared = ty
                .map(|t| format!(": {}", decompile_type_expr(t, interner)))
                .unwrap_or_default();
            let expr_str = decompile_expr(value, interner);
            let mutable = if *mutable { "mutable " } else { "" };
            out.push_str(&format!("{pad}Let {mutable}{name}{declared} be {expr_str}.\n"));
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
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
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
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            let coll_str = decompile_expr(collection, interner);
            let idx_str = decompile_expr(index, interner);
            let val_str = decompile_expr(value, interner);
            out.push_str(&format!(
                "{}Set item {} of {} to {}.\n",
                pad, idx_str, coll_str, val_str
            ));
        }
        Stmt::SetField {
            object,
            field,
            value,
        } => {
            let obj_str = decompile_expr(object, interner);
            let field_name = interner.resolve(*field);
            let val_str = decompile_expr(value, interner);
            out.push_str(&format!(
                "{}Set {} of {} to {}.\n",
                pad, field_name, obj_str, val_str
            ));
        }
        Stmt::Repeat {
            pattern,
            iterable,
            body,
            ..
        } => {
            let var_name = match pattern {
                Pattern::Identifier(sym) => interner.resolve(*sym).to_string(),
                Pattern::Tuple(syms) => format!(
                    "({})",
                    syms.iter().map(|s| interner.resolve(*s).to_string()).collect::<Vec<_>>().join(", ")
                ),
            };
            let source = match iterable {
                Expr::Range { start, end } => {
                    format!("from {} to {}", decompile_expr(start, interner), decompile_expr(end, interner))
                }
                other => format!("in {}", decompile_expr(other, interner)),
            };
            out.push_str(&format!("{}Repeat for {} {}:\n", pad, var_name, source));
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
                    let bindings: Vec<String> = arm
                        .bindings
                        .iter()
                        .map(|(_, b)| interner.resolve(*b).to_string())
                        .collect();
                    if bindings.is_empty() {
                        out.push_str(&format!("{}    When {}:\n", pad, variant_name));
                    } else {
                        out.push_str(&format!(
                            "{}    When {}({}):\n",
                            pad,
                            variant_name,
                            bindings.join(", ")
                        ));
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
                out.push_str(&format!(
                    "{}Pop from {} into {}.\n",
                    pad, coll_str, target_name
                ));
            } else {
                out.push_str(&format!("{}Pop from {}.\n", pad, coll_str));
            }
        }
        Stmt::Break => {
            out.push_str(&format!("{}Break.\n", pad));
        }
        Stmt::RuntimeAssert {
            condition,
            hard,
            dim: _,
            reason,
        } => {
            let cond_str = decompile_expr(condition, interner);
            // Preserve the enforced/dev distinction: `Require that` (hard) survives the
            // round-trip rather than silently becoming the dev-only `Assert that`.
            let kw = if *hard { "Require that" } else { "Assert that" };
            match reason {
                Some(r) => out.push_str(&format!(
                    "{}{} {}, otherwise {}.\n",
                    pad,
                    kw,
                    cond_str,
                    decompile_expr(r, interner)
                )),
                None => out.push_str(&format!("{}{} {}.\n", pad, kw, cond_str)),
            }
        }
        Stmt::Fail { reason } => {
            out.push_str(&format!(
                "{}Fail with {}.\n",
                pad,
                decompile_expr(reason, interner)
            ));
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
            out.push_str(&format!(
                "{}Inside a new zone called \"{}\":\n",
                pad, zone_name
            ));
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
            out.push_str(&format!(
                "{}Write {} to file {}.\n",
                pad, content_str, path_str
            ));
        }
        Stmt::Sleep { milliseconds } => {
            let ms = decompile_expr(milliseconds, interner);
            out.push_str(&format!("{}Sleep {}.\n", pad, ms));
        }
        // CRDT mutations: a Shared struct the program mutates survives the
        // projection unfolded (the optimizer keeps the binding dynamic), so the
        // residual MUST re-emit these or it silently changes the program's value.
        Stmt::IncreaseCrdt {
            object,
            field,
            amount,
        } => {
            let obj = decompile_expr(object, interner);
            let fld = interner.resolve(*field);
            let amt = decompile_expr(amount, interner);
            out.push_str(&format!("{}Increase {}'s {} by {}.\n", pad, obj, fld, amt));
        }
        Stmt::DecreaseCrdt {
            object,
            field,
            amount,
        } => {
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
        Stmt::JoinCrdt { source, target } => {
            let src = decompile_expr(source, interner);
            let tgt = decompile_expr(target, interner);
            out.push_str(&format!("{}Converge {} into {}.\n", pad, src, tgt));
        }
        Stmt::AppendToSequence { sequence, value } => {
            let seq = decompile_expr(sequence, interner);
            let val = decompile_expr(value, interner);
            out.push_str(&format!("{}Append {} to {}.\n", pad, val, seq));
        }
        Stmt::ResolveConflict {
            object,
            field,
            value,
        } => {
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

/// An operand of a binary operator as the program writes it: parenthesized when it is itself an operation, or a
/// constructor ending in its type (`a new Seq of Int`), whose type would take the operator that follows.
fn decompile_operand(expr: &Expr, interner: &Interner) -> String {
    let construction = matches!(expr, Expr::New { .. } | Expr::NewVariant { .. });
    let spelled_in_words =
        matches!(expr, Expr::Literal(Literal::Moment(_) | Literal::Span { .. }) | Expr::Dom(_));
    if matches!(expr, Expr::BinaryOp { .. }) || construction || spelled_in_words {
        format!("({})", decompile_expr(expr, interner))
    } else {
        decompile_expr(expr, interner)
    }
}

fn decompile_expr(expr: &Expr, interner: &Interner) -> String {
    match expr {
        Expr::Dom(q) => match q.selector() {
            Some(sel) => format!("the {} of {}", q.noun(), decompile_expr(sel, interner)),
            None => format!("the {}", q.noun()),
        },
        Expr::View(v) => decompile_view(v, interner),
        Expr::Literal(lit) => match lit {
            Literal::Number(n) => n.to_string(),
            Literal::Float(f) => format!("{}", f),
            Literal::Boolean(b) => {
                if *b {
                    "true".to_string()
                } else {
                    "false".to_string()
                }
            }
            Literal::Text(s) => text_literal(interner.resolve(*s)),
            Literal::Nothing => "nothing".to_string(),
            Literal::Char(_)
            | Literal::Duration(_)
            | Literal::Date(_)
            | Literal::Moment(_)
            | Literal::Span { .. }
            | Literal::Time(_) => CoreAtom::of_literal(lit).expect("a temporal or character literal is an atom").expression(),
        },
        Expr::Identifier(sym) => interner.resolve(*sym).to_string(),
        Expr::BinaryOp { op, left, right } => {
            let l = decompile_operand(left, interner);
            let r = decompile_operand(right, interner);
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
            let op_str = crate::semantics::operator_overloads::surface_text(*op);
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
        Expr::New {
            type_name,
            type_args,
            init_fields,
        } => {
            let name = interner.resolve(*type_name);
            let mut out = format!("a new {name}");
            if !type_args.is_empty() {
                let args: Vec<String> = type_args
                    .iter()
                    .map(|t| decompile_type_expr(t, interner))
                    .collect();
                out.push_str(&format!(" of {}", args.join(generic_separator(name))));
            }
            if !init_fields.is_empty() {
                let fields: Vec<String> = init_fields
                    .iter()
                    .map(|(name, value)| {
                        format!("{} {}", interner.resolve(*name), decompile_expr(value, interner))
                    })
                    .collect();
                out.push_str(&format!(" with {}", fields.join(" and ")));
            }
            out
        }
        Expr::NewVariant {
            variant, fields, ..
        } => {
            let vn = interner.resolve(*variant);
            if fields.is_empty() {
                format!("a new {}", vn)
            } else {
                let parts: Vec<String> = fields
                    .iter()
                    .map(|(name, val)| {
                        let n = interner.resolve(*name);
                        let v = decompile_expr(val, interner);
                        format!("{} {}", n, v)
                    })
                    .collect();
                format!("a new {} with {}", vn, parts.join(" and "))
            }
        }
        Expr::InterpolatedString(parts) => {
            let mut result = String::new();
            for part in parts {
                match part {
                    StringPart::Literal(sym) => {
                        result.push_str(&text_literal_body(interner.resolve(*sym)));
                    }
                    StringPart::Expr { value, debug, format_spec } => {
                        let expr_str = decompile_expr(value, interner);
                        let spec = format_spec.map(|s| format!(":{}", interner.resolve(s))).unwrap_or_default();
                        let label = if *debug { "=" } else { "" };
                        result.push_str(&format!("{{{expr_str}{label}{spec}}}"));
                    }
                }
            }
            format!("\"{}\"", result)
        }
        Expr::Slice {
            collection,
            start,
            end,
        } => {
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
        Expr::Closure {
            params,
            body,
            return_type,
        } => {
            let param_strs: Vec<String> = params
                .iter()
                .map(|(name, ty)| {
                    let n = interner.resolve(*name);
                    let t = decompile_type_expr(ty, interner);
                    format!("{}: {}", n, t)
                })
                .collect();
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

/// What separates a generic type's parameters as the language writes them: a map's key `to` its
/// value (`Map of Text to Int`), any other pair `and` (`Result of Int and Text`).
fn generic_separator(base: &str) -> &'static str {
    match base {
        "Map" | "HashMap" | "SharedMap" | "ORMap" => " to ",
        _ => " and ",
    }
}

/// What `a new Seq of T`, `a new Set of T` and `a new Map of K to V` build, for the Core IR: a sequence's or a set's
/// element type, and a map's whole type (`Map of Text to Int`), each as LOGOS writes it. A collection whose type the
/// program leaves open is `Any`.
fn collection_type_text(type_name: &str, type_args: &[TypeExpr], interner: &Interner) -> String {
    match type_name {
        "Seq" | "List" | "Set" => type_args
            .first()
            .map(|t| decompile_type_expr(t, interner))
            .unwrap_or_else(|| "Any".to_string()),
        _ => {
            if type_args.len() == 2 {
                format!(
                    "Map of {} to {}",
                    decompile_type_expr(&type_args[0], interner),
                    decompile_type_expr(&type_args[1], interner)
                )
            } else {
                type_name.to_string()
            }
        }
    }
}

fn decompile_type_expr(ty: &TypeExpr, interner: &Interner) -> String {
    match ty {
        TypeExpr::Primitive(sym) => interner.resolve(*sym).to_string(),
        TypeExpr::Named(sym) => interner.resolve(*sym).to_string(),
        TypeExpr::Generic { base, params } => {
            let base_str = interner.resolve(*base);
            let param_strs: Vec<String> = params
                .iter()
                .map(|p| decompile_type_expr(p, interner))
                .collect();
            format!("{} of {}", base_str, param_strs.join(generic_separator(base_str)))
        }
        TypeExpr::Tuple(params) => {
            let elems: Vec<String> = params
                .iter()
                .map(|p| decompile_type_expr(p, interner))
                .collect();
            format!("({})", elems.join(", "))
        }
        TypeExpr::Record {
            field_names,
            field_types,
        } => {
            let elems: Vec<String> = field_names
                .iter()
                .zip(field_types.iter())
                .map(|(n, t)| {
                    format!(
                        "{}: {}",
                        interner.resolve(*n),
                        decompile_type_expr(t, interner)
                    )
                })
                .collect();
            format!("({})", elems.join(", "))
        }
        TypeExpr::Function { inputs, output } => {
            let in_strs: Vec<String> = inputs
                .iter()
                .map(|t| decompile_type_expr(t, interner))
                .collect();
            let out_str = decompile_type_expr(output, interner);
            format!("fn({}) -> {}", in_strs.join(", "), out_str)
        }
        TypeExpr::Refinement { base, .. } => decompile_type_expr(base, interner),
        TypeExpr::Persistent { inner } => {
            format!("Persistent {}", decompile_type_expr(inner, interner))
        }
        TypeExpr::Mutable { inner } => {
            format!("mutable {}", decompile_type_expr(inner, interner))
        }
    }
}

/// Verify that a LOGOS program has no interpretive overhead.
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
    let _compile_facts = crate::optimize::facts::scope();
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

    let mut parser = crate::parser::Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
    );
    let stmts = parser
        .parse_program()
        .map_err(|e| format!("Parse error: {}", describe_parse_error(&e)))?;

    verify_no_overhead_stmts(&stmts, &interner)
}

const CORE_VARIANT_NAMES: &[&str] = &[
    "CInt",
    "CBool",
    "CText",
    "CVar",
    "CBinOp",
    "CNot",
    "CCall",
    "CIndex",
    "CLen",
    "CMapGet",
    "CLet",
    "CSet",
    "CIf",
    "CWhile",
    "CReturn",
    "CShow",
    "CCallS",
    "CPush",
    "CSetIdx",
    "CMapSet",
    "CFuncDef",
    "CProg",
    "CManifestOf",
    "CChunkAt",
    "VInt",
    "VBool",
    "VText",
    "VSeq",
    "VMap",
    "VError",
    "VNothing",
];

/// Interpreter *carrier* type names — the umbrella data types the self-interpreter/PE thread
/// through their state. A Jones-optimal residual holds none of them: constructing
/// `a new Map of Text to CVal` (the environment), or binding a `CExpr`-typed variable, is
/// surviving interpreter state, not dissolved computation.
const CORE_TYPE_NAMES: &[&str] = &[
    "CVal",
    "CExpr",
    "CStmt",
    "CFunc",
    "CFuncDef",
    "CProgram",
    "CProg",
    "CoreOut",
    "CoreNext",
    "CoreReturn",
    "CoreBreak",
    "PEState",
    "PEStateR",
    "PEMiniState",
    "PEMiniR",
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
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
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
                    "Interpretive overhead: Core type constructor 'new {}'",
                    tn
                ));
            }
        }
        Expr::NewVariant { variant, .. } => {
            let vn = interner.resolve(*variant);
            if CORE_VARIANT_NAMES.contains(&vn) {
                return Err(format!(
                    "Interpretive overhead: Core variant constructor '{}'",
                    vn
                ));
            }
        }
        Expr::Call { function, args } => {
            let fn_name = interner.resolve(*function);
            if CORE_VARIANT_NAMES.contains(&fn_name) {
                return Err(format!(
                    "Interpretive overhead: Core variant call '{}'",
                    fn_name
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
    "peExpr",
    "peBlock",
    "coreEval",
    "coreExecBlock",
    "applyBinOp",
];

/// The Jones-optimality oracle: count the units of interpreter dispatch remaining
/// in a residual program.
///
/// A residual that is genuinely Jones-optimal has dissolved the interpreter entirely
/// and returns zero. One unit is counted for each of:
/// - an `Inspect` arm dispatching on a Core IR variant (`CInt`, `CIf`, …);
/// - an environment/function-map lookup `item <literal> of env` / `... of funcs`;
/// - a Core IR constructor (`new CInt`, a `CInt(...)` call, a `VInt` variant, …);
/// - a call to a known dispatch function (`DISPATCH_FN_NAMES`).
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
    let _compile_facts = crate::optimize::facts::scope();
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

    let mut parser = crate::parser::Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
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
/// `(a: T) and (b: U)` for a parameter list, or the empty string.
fn decompile_params(
    params: &[(logicaffeine_base::Symbol, &TypeExpr)],
    interner: &Interner,
) -> String {
    params
        .iter()
        .map(|(n, t)| {
            format!(
                "({}: {})",
                interner.resolve(*n),
                decompile_type_expr(t, interner)
            )
        })
        .collect::<Vec<_>>()
        .join(" and ")
}

/// The source spelling of a type expression (the common shapes; anything else prints its
/// debug form so a decompile never silently drops a type).
/// A component, page, store or error page back to its LGX source: header, `Use`, and the
/// `State:` / `Actions:` / `View:` / `Style:` sections it carries.
fn decompile_component(
    def: &logicaffeine_language::ast::component::ComponentDef,
    interner: &Interner,
    out: &mut String,
) {
    use logicaffeine_language::ast::component::{ComponentKind, StoreMode, Trigger};
    let props = decompile_params(&def.props, interner);
    match &def.kind {
        ComponentKind::Component => out.push_str(&format!(
            "## Component {}{}\n",
            interner.resolve(def.name),
            if props.is_empty() {
                String::new()
            } else {
                format!(" {props}")
            }
        )),
        ComponentKind::Page { path, params } => {
            let ps = decompile_params(params, interner);
            out.push_str(&format!(
                "## Page {}{}\n",
                interner.resolve(*path),
                if ps.is_empty() {
                    String::new()
                } else {
                    format!(" {ps}")
                }
            ));
        }
        ComponentKind::Store { mode } => {
            let tail = match mode {
                StoreMode::Local => String::new(),
                StoreMode::Persisted { key } => {
                    format!(" persisted as {}", decompile_expr(key, interner))
                }
                StoreMode::Shared { topic } => {
                    format!(" shared on {}", decompile_expr(topic, interner))
                }
                StoreMode::Preserved => " preserved across reloads".to_string(),
            };
            out.push_str(&format!(
                "## Store {}{}\n",
                interner.resolve(def.name),
                tail
            ));
        }
        ComponentKind::ErrorPage { status } => out.push_str(&format!("## Error page {status}\n")),
    }
    for u in &def.uses {
        out.push_str(&format!("Use {}.\n", interner.resolve(*u)));
    }
    if !def.state.is_empty() {
        out.push_str("State:\n");
        for f in &def.state {
            let ty =
                f.ty.map(|t| format!(": {}", decompile_type_expr(t, interner)))
                    .unwrap_or_default();
            let verb = if f.derived { "always be" } else { "be" };
            out.push_str(&format!(
                "    Let {}{} {} {}.\n",
                interner.resolve(f.name),
                ty,
                verb,
                decompile_expr(f.init, interner)
            ));
        }
    }
    if !def.events.is_empty() || !def.actions.is_empty() {
        out.push_str("Actions:\n");
        for ev in &def.events {
            let head = match &ev.trigger {
                Trigger::PageLoads => "When the page loads".to_string(),
                Trigger::Appears => "When this component appears".to_string(),
                Trigger::Disappears => "When this component disappears".to_string(),
                Trigger::RouteChanges => "When the route changes".to_string(),
                Trigger::Online => "When the app comes back online".to_string(),
                Trigger::Offline => "When the app goes offline".to_string(),
                Trigger::Selector {
                    selector,
                    event,
                    key: None,
                } => format!(
                    "When {} is {}",
                    decompile_expr(selector, interner),
                    interner.resolve(*event)
                ),
                Trigger::Selector {
                    selector,
                    event: _,
                    key: Some(k),
                } => format!(
                    "When the user presses {} in {}",
                    interner.resolve(*k),
                    decompile_expr(selector, interner)
                ),
                Trigger::Key { key } => format!("When the user presses {}", interner.resolve(*key)),
                Trigger::FieldChanges { store: None, field } => {
                    format!("When {} changes", interner.resolve(*field))
                }
                Trigger::FieldChanges {
                    store: Some(st),
                    field,
                } => format!(
                    "When {}'s {} changes",
                    interner.resolve(*st),
                    interner.resolve(*field)
                ),
                Trigger::Every {
                    seconds,
                    name: None,
                } => format!("Every {} seconds", decompile_expr(seconds, interner)),
                Trigger::Every {
                    seconds,
                    name: Some(n),
                } => format!(
                    "Every {} seconds as {}",
                    decompile_expr(seconds, interner),
                    interner.resolve(*n)
                ),
                Trigger::After {
                    seconds,
                    name: None,
                } => format!("After {} seconds", decompile_expr(seconds, interner)),
                Trigger::After {
                    seconds,
                    name: Some(n),
                } => format!(
                    "After {} seconds as {}",
                    decompile_expr(seconds, interner),
                    interner.resolve(*n)
                ),
                Trigger::Message { topic, param } => format!(
                    "When a message arrives on {} ({}: {})",
                    decompile_expr(topic, interner),
                    interner.resolve(param.0),
                    decompile_type_expr(param.1, interner)
                ),
                Trigger::ActionFails { param } => {
                    format!("When an action fails ({}: Text)", interner.resolve(*param))
                }
                Trigger::FetchFails { url, error } => format!(
                    "When a fetch fails ({}: Text) and ({}: Text)",
                    interner.resolve(*url),
                    interner.resolve(*error)
                ),
                Trigger::WindowResizes => "When the window resizes".to_string(),
                Trigger::ScrollsToBottom { selector: None } => {
                    "When the user scrolls to the bottom".to_string()
                }
                Trigger::ScrollsToBottom {
                    selector: Some(sel),
                } => format!(
                    "When the user scrolls to the bottom of {}",
                    decompile_expr(sel, interner)
                ),
                Trigger::FileDropped { selector, param } => format!(
                    "When a file is dropped on {} ({}: {})",
                    decompile_expr(selector, interner),
                    interner.resolve(param.0),
                    decompile_type_expr(param.1, interner)
                ),
            };
            out.push_str(&format!("    {head}:\n"));
            for inner in ev.body.iter() {
                decompile_stmt(inner, interner, out, 2);
            }
        }
        for a in &def.actions {
            let ps = decompile_params(&a.params, interner);
            out.push_str(&format!(
                "    To {}{}:\n",
                interner.resolve(a.name),
                if ps.is_empty() {
                    String::new()
                } else {
                    format!(" {ps}")
                }
            ));
            for inner in a.body.iter() {
                decompile_stmt(inner, interner, out, 2);
            }
        }
    }
    if let Some(view) = &def.view {
        out.push_str("View:\n");
        for line in decompile_view(view, interner).lines() {
            out.push_str(&format!("    {line}\n"));
        }
    }
    if let Some(style) = &def.style {
        out.push_str("Style:\n");
        for line in interner.resolve(style.css).lines() {
            out.push_str(&format!("    {line}\n"));
        }
    }
}

/// A view tree back to LGX template source (holes through `decompile_expr`).
fn decompile_view(view: &logicaffeine_language::ast::view::ViewDef, interner: &Interner) -> String {
    use logicaffeine_language::ast::view::{Attr, ViewNode};
    fn esc(s: &str, out: &mut String) {
        for c in s.chars() {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '{' => out.push_str("&#123;"),
                '}' => out.push_str("&#125;"),
                _ => out.push(c),
            }
        }
    }
    fn go(nodes: &[ViewNode], interner: &Interner, out: &mut String) {
        for n in nodes {
            match n {
                ViewNode::Element {
                    tag,
                    attrs,
                    children,
                    ..
                } => {
                    out.push('<');
                    out.push_str(interner.resolve(*tag));
                    for a in attrs {
                        match a {
                            Attr::Plain { name, value } => {
                                out.push(' ');
                                out.push_str(interner.resolve(*name));
                                out.push_str("={");
                                out.push_str(&decompile_expr(value, interner));
                                out.push('}');
                            }
                            Attr::Bind { field } => {
                                out.push_str(" bind={");
                                out.push_str(interner.resolve(*field));
                                out.push('}');
                            }
                            Attr::Class { name, cond } => {
                                out.push_str(&format!(
                                    " class:{}={{{}}}",
                                    interner.resolve(*name),
                                    decompile_expr(cond, interner)
                                ));
                            }
                            Attr::Directive { name, value } => {
                                out.push_str(&format!(
                                    " {}={{{}}}",
                                    interner.resolve(*name),
                                    decompile_expr(value, interner)
                                ));
                            }
                        }
                    }
                    out.push('>');
                    if logicaffeine_language::lgx::html::is_void(interner.resolve(*tag)) {
                        continue;
                    }
                    go(children, interner, out);
                    out.push_str("</");
                    out.push_str(interner.resolve(*tag));
                    out.push('>');
                }
                ViewNode::Text { text, .. } => {
                    out.push('{');
                    out.push_str(&decompile_expr(text, interner));
                    out.push('}');
                }
                ViewNode::Component {
                    name,
                    props,
                    children,
                    ..
                } => {
                    out.push('<');
                    out.push_str(interner.resolve(*name));
                    for (k, v) in props {
                        out.push(' ');
                        out.push_str(interner.resolve(*k));
                        out.push_str("={");
                        out.push_str(&decompile_expr(v, interner));
                        out.push('}');
                    }
                    if children.is_empty() {
                        out.push_str("/>");
                    } else {
                        out.push('>');
                        go(children, interner, out);
                        out.push_str("</");
                        out.push_str(interner.resolve(*name));
                        out.push('>');
                    }
                }
                ViewNode::Each {
                    pattern,
                    iterable,
                    key,
                    index,
                    body,
                    empty,
                    ..
                } => {
                    let pat = match pattern {
                        logicaffeine_language::ast::stmt::Pattern::Identifier(s) => {
                            interner.resolve(*s).to_string()
                        }
                        logicaffeine_language::ast::stmt::Pattern::Tuple(items) => format!(
                            "({})",
                            items
                                .iter()
                                .map(|s| interner.resolve(*s))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    };
                    out.push_str(&format!(
                        "{{for each {pat} in {}",
                        decompile_expr(iterable, interner)
                    ));
                    if let Some(k) = key {
                        out.push_str(&format!(", keyed by {}", decompile_expr(k, interner)));
                    }
                    if let Some(i) = index {
                        out.push_str(&format!(", at index {}", interner.resolve(*i)));
                    }
                    out.push_str(":}");
                    go(body, interner, out);
                    if !empty.is_empty() {
                        out.push_str("{else:}");
                        go(empty, interner, out);
                    }
                    out.push_str("{end}");
                }
                ViewNode::If {
                    arms, otherwise, ..
                } => {
                    for (i, (c, b)) in arms.iter().enumerate() {
                        out.push_str(if i == 0 { "{if " } else { "{else if " });
                        out.push_str(&decompile_expr(c, interner));
                        out.push_str(":}");
                        go(b, interner, out);
                    }
                    if let Some(o) = otherwise {
                        out.push_str("{else:}");
                        go(o, interner, out);
                    }
                    out.push_str("{end}");
                }
                ViewNode::Handler {
                    event,
                    modifiers,
                    body,
                    ..
                } => {
                    let _ = modifiers;
                    // Legacy positional handler (the indentation DSL): its statements have no
                    // attribute form; render the intent as a template comment.
                    out.push_str(&format!(
                        "{{# on {}: {} statement(s) #}}",
                        interner.resolve(*event),
                        body.len()
                    ));
                }
                ViewNode::Raw { html, .. } => {
                    out.push_str("{raw ");
                    out.push_str(&decompile_expr(html, interner));
                    out.push('}');
                }
                ViewNode::Inspect {
                    scrutinee, arms, ..
                } => {
                    out.push_str(&format!(
                        "{{inspect {}:}}",
                        decompile_expr(scrutinee, interner)
                    ));
                    for arm in arms {
                        let head = match arm.variant {
                            None => "Otherwise".to_string(),
                            Some(v) => {
                                let bs: Vec<&str> = arm
                                    .bindings
                                    .iter()
                                    .map(|(_, b)| interner.resolve(*b))
                                    .collect();
                                if bs.is_empty() {
                                    interner.resolve(v).to_string()
                                } else {
                                    format!("{} ({})", interner.resolve(v), bs.join(", "))
                                }
                            }
                        };
                        out.push_str(&format!("{{when {head}:}}"));
                        go(&arm.body, interner, out);
                    }
                    out.push_str("{end}");
                }
                ViewNode::Let { var, value, .. } => {
                    out.push_str(&format!(
                        "{{let {} be {}}}",
                        interner.resolve(*var),
                        decompile_expr(value, interner)
                    ));
                }
                ViewNode::Extends { layout, .. } => {
                    out.push_str(&format!("{{extends {}}}", interner.resolve(*layout)));
                }
                ViewNode::Block { name, default, .. } => match default {
                    None => out.push_str(&format!("{{block {}}}", interner.resolve(*name))),
                    Some(d) => {
                        out.push_str(&format!("{{block {}:}}", interner.resolve(*name)));
                        go(d, interner, out);
                        out.push_str("{end}");
                    }
                },
                ViewNode::Children { .. } => out.push_str("{children}"),
            }
        }
    }
    let mut out = String::new();
    go(&view.nodes, interner, &mut out);
    out
}

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
        TypeExpr::Tuple(params) => params.iter().any(|p| type_expr_mentions_core(p, interner)),
        TypeExpr::Record { field_types, .. } => field_types
            .iter()
            .any(|p| type_expr_mentions_core(p, interner)),
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
        Stmt::ComponentDef(def) => {
            for f in &def.state {
                count_expr_dispatch(f.init, interner, count);
            }
            for ev in &def.events {
                for inner in ev.body.iter() {
                    count_stmt_dispatch(inner, interner, count);
                }
            }
            for a in &def.actions {
                for inner in a.body.iter() {
                    count_stmt_dispatch(inner, interner, count);
                }
            }
            if let Some(view) = &def.view {
                let (view_exprs, view_blocks) = view.exprs_and_blocks();
                for e in view_exprs {
                    count_expr_dispatch(e, interner, count);
                }
                for b in view_blocks {
                    for inner in b.iter() {
                        count_stmt_dispatch(inner, interner, count);
                    }
                }
            }
        }
        Stmt::DirectiveDef { body, removed, .. } => {
            for inner in body.iter() {
                count_stmt_dispatch(inner, interner, count);
            }
            if let Some(r) = removed {
                for inner in r.iter() {
                    count_stmt_dispatch(inner, interner, count);
                }
            }
        }
        Stmt::RouteDef { body, .. } => match &**body {
            logicaffeine_language::ast::component::RouteBody::Statements(b) => {
                for inner in b.iter() {
                    count_stmt_dispatch(inner, interner, count);
                }
            }
            logicaffeine_language::ast::component::RouteBody::View(view) => {
                let (view_exprs, view_blocks) = view.exprs_and_blocks();
                for e in view_exprs {
                    count_expr_dispatch(e, interner, count);
                }
                for b in view_blocks {
                    for inner in b.iter() {
                        count_stmt_dispatch(inner, interner, count);
                    }
                }
            }
        },
        Stmt::MiddlewareDef { body, .. } => {
            for inner in body.iter() {
                count_stmt_dispatch(inner, interner, count);
            }
        }
        Stmt::RedirectDef { .. } => {}
        Stmt::ServeFetches | Stmt::Replay { .. } => {}
        Stmt::GivenFetch { url, status, body, .. } => {
            count_expr_dispatch(url, interner, count);
            count_expr_dispatch(status, interner, count);
            count_expr_dispatch(body, interner, count);
        }
        Stmt::Fetch { url, body, .. } => {
            count_expr_dispatch(url, interner, count);
            if let Some(b) = body {
                count_expr_dispatch(b, interner, count);
            }
        }
        Stmt::Ui(u) => {
            for x in u.exprs() {
                count_expr_dispatch(x, interner, count);
            }
        }
        Stmt::Splice { body } | Stmt::Reversibly { body } => {
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
        // `Swap a and b` reads two bare variables — no interpreted sub-expression, no dispatch.
        Stmt::Swap { .. } => {}
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
        Stmt::RuntimeAssert { condition, reason, .. } => {
            count_expr_dispatch(condition, interner, count);
            if let Some(r) = reason {
                count_expr_dispatch(r, interner, count);
            }
        }
        Stmt::Fail { reason } => count_expr_dispatch(reason, interner, count),
        Stmt::Give { object, recipient } | Stmt::Show { object, recipient } => {
            count_expr_dispatch(object, interner, count);
            count_expr_dispatch(recipient, interner, count);
        }
        Stmt::SetField { object, value, .. } => {
            count_expr_dispatch(object, interner, count);
            count_expr_dispatch(value, interner, count);
        }
        Stmt::FunctionDef { body, .. } => count_block_dispatch(body, interner, count),
        Stmt::TestDef { body, .. } => count_block_dispatch(body, interner, count),
        Stmt::During { body, .. } => count_block_dispatch(body, interner, count),
        Stmt::ForAll { body, constraint, .. } => {
            if let Some(c) = constraint {
                count_expr_dispatch(c, interner, count);
            }
            count_block_dispatch(body, interner, count);
        }
        Stmt::Observe { payloads, .. } => {
            for (_, e) in payloads {
                count_expr_dispatch(e, interner, count);
            }
        }
        Stmt::Expect { condition } => count_expr_dispatch(condition, interner, count),
        Stmt::ExpectFail { body, .. } => count_block_dispatch(body, interner, count),
        Stmt::ExpectOutput { .. } | Stmt::GivenFile { .. } => {}
        Stmt::AdvanceTime { millis } => count_expr_dispatch(millis, interner, count),
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
        Stmt::MergeCrdt { source, target } | Stmt::JoinCrdt { source, target } => {
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
        Stmt::Serve { port, handler } => {
            count_expr_dispatch(port, interner, count);
            count_expr_dispatch(handler, interner, count);
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
        Stmt::CloseChannel { channel } => count_expr_dispatch(channel, interner, count),
        Stmt::AwaitTask { handle, .. } => count_expr_dispatch(handle, interner, count),
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
        | Stmt::Require { .. }
        // An opaque failure-site marker: an id, no interpreted sub-expression — same shape as
        // `Swap` above, no dispatch.
        | Stmt::Site(_) => {}
    }
}

/// Exhaustive by construction — NO `_` arm. Every sub-expression is recursed, so a Core
/// constructor or dispatch call cannot hide inside a container (`List`/`Tuple`), an
/// operator, a constructor field, an interpolant, or a closure body.
fn count_expr_dispatch(expr: &Expr, interner: &Interner, count: &mut usize) {
    match expr {
        Expr::Dom(q) => {
            if let Some(sel) = q.selector() {
                count_expr_dispatch(sel, interner, count);
            }
        }
        Expr::View(v) => {
            let (view_exprs, view_blocks) = v.exprs_and_blocks();
            for e in view_exprs {
                count_expr_dispatch(e, interner, count);
            }
            for b in view_blocks {
                for inner in b.iter() {
                    count_stmt_dispatch(inner, interner, count);
                }
            }
        }
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
        Expr::Slice {
            collection,
            start,
            end,
        } => {
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
        Expr::New {
            type_name,
            type_args,
            init_fields,
        } => {
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
        Expr::NewVariant {
            variant, fields, ..
        } => {
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

/// Returns the source text of the partial evaluator written in LOGOS.
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
    A CNewSeq with elementType Text.
    A CNewVariant with tag Text and fnames Seq of Text and fvals Seq of CExpr.
    A CList with items Seq of CExpr.
    A CRange with start CExpr and end CExpr.
    A CSlice with coll CExpr and startIdx CExpr and endIdx CExpr.
    A CCopy with target CExpr.
    A CNewSet with elementType Text.
    A CContains with coll CExpr and elem CExpr.
    A CUnion with left CExpr and right CExpr.
    A CIntersection with left CExpr and right CExpr.
    A COptionSome with inner CExpr.
    A COptionNone.
    A CNothing.
    A CDeclaredDefault with typeName Text.
    A CSpelled with code Text and awaits Text.
    A CTuple with items Seq of CExpr.
    A CNew with typeName Text and fieldNames Seq of Text and fields Seq of CExpr.
    A CFieldAccess with target CExpr and field Text.
    A CClosure with params Seq of Text and body Seq of CStmt and captured Seq of Text.
    A CCallExpr with target CExpr and args Seq of CExpr.
    A CInterpolatedString with parts Seq of CStringPart.
    A CDuration with value Duration.
    A CDate with value Date.
    A CMoment with value Moment.
    A CSpan with value Span.
    A CTime with value Time.
    A CChar with value Char.
    A CTimeNow.
    A CDateToday.
    A CEscExpr with code Text.
    A CManifestOf with zn CExpr.
    A CChunkAt with idx CExpr and zn CExpr.

## A CStringPart is one of:
    A CLiteralPart with value Text.
    A CExprPart with expr CExpr.
    A CFormattedPart with expr CExpr and spec Text and form Text and size Int.

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
    A CPopInto with target Text and into Text.
    A CRepeat with var Text and coll CExpr and body Seq of CStmt.
    A CRepeatRange with var Text and start CExpr and end CExpr and body Seq of CStmt.
    A CBreak.
    A CAdd with elem CExpr and target Text.
    A CRemove with elem CExpr and target Text.
    A CForceDynamic with name Text.
    A CSetField with target Text and field Text and val CExpr.
    A CAtPath with target Text and path Seq of Text and body Seq of CStmt.
    A CStructDef with name Text and fieldNames Seq of Text.
    A CInspect with target CExpr and arms Seq of CMatchArm.
    A CEnumDef with name Text and variants Seq of Text.
    A CRuntimeAssert with cond CExpr and msg CExpr.
    A CHardAssert with cond CExpr and msg CExpr.
    A CFail with reason CExpr.
    A CGive with expr CExpr and target Text.
    A CEscStmt with code Text.
    A CSleep with duration CExpr.
    A CReadConsole with target Text.
    A CReadFile with path CExpr and target Text.
    A CWriteFile with path CExpr and content CExpr.
    A CCheck with predicate CExpr and msg CExpr.
    A CCheckPolicy with subject Text and object Text and capability Bool and rule Text and alternatives Seq of CPolicyAlt and source Text.
    A CAssert with proposition CExpr.
    A CTrust with proposition CExpr and justification Text.
    A CRequire with dependency Text.
    A CMerge with target Text and other CExpr.
    A CJoin with target Text and other CExpr.
    A CIncrease with target Text and amount CExpr.
    A CDecrease with target Text and amount CExpr.
    A CAppendToSeq with target Text and value CExpr.
    A CResolve with target Text.
    A CResolveTo with target Text and value CExpr.
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

## A CPolicyAlt is one of:
    A CPolicyCase with typeName Text and condition CPolicy.

## A CPolicy is one of:
    A PFieldEquals with field Text and expected Text and literal Bool and asInt Int and isInt Bool and asBool Bool and isBool Bool.
    A PFieldBool with field Text and value Bool.
    A PFieldsEqual with subjectField Text and objectField Text.
    A PAnd with left CPolicy and right CPolicy.
    A POr with left CPolicy and right CPolicy.
    A PFalse.
    A PRecursive with predicate Text.

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
    A VDuration with value Duration.
    A VDate with value Date.
    A VMoment with value Moment.
    A VSpan with value Span.
    A VTime with value Time.
    A VChar with value Char.
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
    let encoded = encode_program_source(&full_source)
        .map_err(|e| format!("Failed to encode PE: {}", describe_parse_error(&e)))?;
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

    let compiler_source = replace_word(
        &replace_word(&pe_source, "peExpr", "compileExpr"),
        "peBlock",
        "compileBlock",
    );

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

    let cogen_source = replace_word(
        &replace_word(&pe_source, "peExpr", "cogenExpr"),
        "peBlock",
        "cogenBlock",
    );

    Ok(format!("{}\n{}", CORE_TYPES_FOR_PE, cogen_source))
}

/// Compile and run LOGOS source, returning stdout.
///
/// Uses the full compilation pipeline: LOGOS → Rust → binary → execute.
/// This is the library-side equivalent of the test infrastructure's `run_logos()`.
pub fn run_logos_source(source: &str) -> Result<String, String> {
    let compile_output = compile_program_full(source)
        .map_err(|e| format!("Compilation failed: {}", describe_parse_error(&e)))?;

    // Create temp directory using std (no tempfile crate needed)
    let temp_base = std::env::temp_dir().join("logos_run_source");
    std::fs::create_dir_all(&temp_base).map_err(|e| format!("mkdir failed: {}", e))?;

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

    std::fs::create_dir_all(project_dir.join("src")).map_err(|e| format!("mkdir failed: {}", e))?;
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
    std::fs::create_dir_all(&target_dir).map_err(|e| format!("mkdir target failed: {}", e))?;

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
                    if n.starts_with(&format!("{stem}-")) || n.starts_with(&format!("lib{stem}-")) {
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
fn discover_entry_points(
    residual: &str,
    block_prefix: &str,
    expr_prefix: &str,
) -> (String, Option<String>) {
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
pub fn projection1_source_real(
    core_types: &str,
    _interpreter: &str,
    program: &str,
) -> Result<String, String> {
    let full_source = if program.contains("## Main") || program.contains("## To ") {
        program.to_string()
    } else {
        format!("## Main\n{}", program)
    };

    // Step 1: Encode the program as CProgram construction source
    let encoded = encode_program_source(&full_source)
        .map_err(|e| format!("Failed to encode program: {}", describe_parse_error(&e)))?;

    // Step 2: Get PE source and decompile source
    let pe_source = pe_source_text();
    let decompile_source = decompile_source_text();

    // Step 3: Build the combined source
    let actual_core_types = if core_types.is_empty() {
        CORE_TYPES_FOR_PE
    } else {
        core_types
    };

    let driver = r#"
    Let state be makePeState(a new Map of Text to CVal, encodedFuncMap, 200).
    Let residual be peBlock(encodedMain, state).
    Let source be decompileBlock(residual, 0).
    Show source.
"#;

    let combined = format!(
        "{}\n{}\n{}\n## Main\n{}\n{}",
        actual_core_types, pe_source, decompile_source, encoded, driver,
    );

    // Step 4: Compile and run to get the decompiled residual
    let raw_residual = with_reference_semantics(|| run_logos_source(&combined))?;
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
pub fn projection1_source_real_fast(
    core_types: &str,
    _interpreter: &str,
    program: &str,
) -> Result<String, String> {
    let combined = pe_combined_source(core_types, program)?;

    let raw_residual = interpret_bootstrap(&combined)
        .map_err(|e| format!("PE execution failed: {}", describe_parse_error(&e)))?;
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

/// The offline specializer: a two-level program (`crate::optimize::mix_bta`) specialized by following its
/// annotations alone, its static parts evaluated by [`core_interp_source`]. Its decisions are static in every
/// self-application, so it is the specializer the textbook projections apply to itself.
pub fn mix_source_text() -> &'static str {
    include_str!("optimize/mix.logos")
}

/// The catalog the binding-time analysis reads constructors' fields from: the Core IR's, then `mix`'s.
fn mix_catalog() -> String {
    format!("{}\n{}", CORE_TYPES_FOR_PE, mix_source_text())
}

/// The type definitions of the programs a projection's residual is specialized from — `core_interp`'s outcomes, the
/// decompiler's, `mix`'s two-level catalog, jobs and block outcomes — whose values the residual builds.
fn mix_type_definitions() -> String {
    let mut out = String::new();
    for source in [core_interp_source(), decompile_source_text(), mix_source_text()] {
        let mut in_type = false;
        for line in source.lines() {
            if line.starts_with("## A ") || line.starts_with("## An ") {
                in_type = true;
            } else if line.starts_with("## ") {
                in_type = false;
            }
            if in_type {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out
}

/// Where the offline specializer runs: compiled once to native code, as the production specializer runs, or on the
/// tree-walker, the semantic ground truth the native engine is held to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MixEngine {
    Native,
    TreeWalker,
}

impl MixEngine {
    /// Native wherever a native engine can be built and loaded; the tree-walker in the browser.
    pub fn default_engine() -> MixEngine {
        if cfg!(target_arch = "wasm32") {
            MixEngine::TreeWalker
        } else {
            MixEngine::Native
        }
    }
}

/// A specialization's static inputs: each name `mix` binds while specializing, with its value.
type MixStore = Vec<(&'static str, crate::optimize::mix_bta::StoreEntry)>;

/// The offline specializer applied, `[[mix]](program, store)`: `mix` specializes the two-level `program` with its
/// static inputs bound by `store`, on `engine`. The residual is its functions, `## Main`, then its main block.
fn mix_apply(
    program: &crate::optimize::mix_bta::TwoLevelProgram,
    store: &MixStore,
    engine: MixEngine,
) -> Result<String, String> {
    let raw = match engine {
        MixEngine::Native => mix_apply_native(program, store)?,
        MixEngine::TreeWalker => {
            let source = format!(
                "{}\n{}\n{}\n{}\n{}\n{}\n## Main\nLet mixResidual be mixProgram(mixBuildMain(), mixBuildStore(), mixBuildFuncs(), mixBuildBFuncs()).\n{}",
                CORE_TYPES_FOR_PE,
                core_interp_source(),
                decompile_source_text(),
                mix_source_text(),
                program.builders()?,
                crate::optimize::mix_bta::store_builder("mixBuildStore", store, &mix_catalog())?,
                MIX_PRINT_RESIDUAL
            );
            interpret_bootstrap(&source).map_err(|e| format!("mix failed: {}", describe_parse_error(&e)))?
        }
    };
    Ok(raw.trim_end().to_string())
}

#[cfg(target_arch = "wasm32")]
fn mix_apply_native(_: &crate::optimize::mix_bta::TwoLevelProgram, _: &MixStore) -> Result<String, String> {
    Err("the native offline specializer is not available in the browser".to_string())
}

/// [`mix_apply`] on the offline specializer compiled once to an in-process cdylib ([`build_native_mix_cdylib`]): the
/// program and its store cross as wire bytes, the residual comes back as text.
#[cfg(not(target_arch = "wasm32"))]
fn mix_apply_native(program: &crate::optimize::mix_bta::TwoLevelProgram, store: &MixStore) -> Result<String, String> {
    let bytes = program.mix_input_wire(store)?;
    MIX_LIB.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(NativeLib::load(&build_native_mix_cdylib()?, b"logos_mix_specialize")?);
        }
        slot.as_ref().expect("loaded").call(&bytes).map_err(|e| format!("mix failed: {e}"))
    })
}

/// The driver a specialization run as a program ends with: the residual as `mixResidualText` prints it.
const MIX_PRINT_RESIDUAL: &str = "Show mixResidualText(mixResidual).\n";

/// The offline specializer as an in-process cdylib entry: `core_interp`, the decompiler and `mix`, with
/// `mixSpecializeOnce`, exported for native, taking a two-level program and its store as one `MixInput` and
/// returning the residual's text.
pub fn mix_cdylib_source() -> String {
    format!(
        "{}\n{}\n{}\n{}\n## A MixInput is one of:\n    A MixIn with main Seq of BStmt and store Map of Text to CVal and funcs Map of Text to CFunc and bfuncs Map of Text to BFunc.\n\n## To mixSpecializeOnce (mixInput: MixInput) -> Text is exported for native:\n    Inspect mixInput:\n        When MixIn (inMain, inStore, inFuncs, inBFuncs):\n            Let inResidual be mixProgram(inMain, inStore, inFuncs, inBFuncs).\n            Return mixResidualText(inResidual).\n        Otherwise:\n            Return \"\".\n\n## Main\n    Let __x be 0.\n",
        CORE_TYPES_FOR_PE,
        core_interp_source(),
        decompile_source_text(),
        mix_source_text()
    )
}

/// The FFI entry appended to the generated offline specializer: decode the `MixInput`, specialize, hand back the text.
#[cfg(not(target_arch = "wasm32"))]
fn mix_cdylib_shim() -> String {
    native_engine_shim("logos_mix_specialize", "MixInput", "mixSpecializeOnce")
}

/// The offline specializer cdylib's `lib.rs`: [`mix_cdylib_source`] through the Rust emitter with the wire codec,
/// under the reference collection semantics, followed by its FFI entry.
#[cfg(not(target_arch = "wasm32"))]
pub fn native_mix_cdylib_lib_rs() -> Result<String, String> {
    let generated = compile_bootstrap_engine(&mix_cdylib_source())
        .map_err(|e| format!("native mix cdylib compile: {}", describe_parse_error(&e)))?
        .rust_code;
    Ok(format!("{generated}\n{}", mix_cdylib_shim()))
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static NATIVE_MIX_CDYLIB: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
    static MIX_LIB: std::cell::RefCell<Option<NativeLib>> = const { std::cell::RefCell::new(None) };
}

/// Build (and cache) the offline specializer as an in-process cdylib exporting `logos_mix_specialize`,
/// content-addressed on its source, shim and collection semantics.
#[cfg(not(target_arch = "wasm32"))]
pub fn build_native_mix_cdylib() -> Result<std::path::PathBuf, String> {
    if let Some(p) = NATIVE_MIX_CDYLIB.with(|c| c.borrow().clone()) {
        if p.exists() {
            return Ok(p);
        }
    }
    let key = aot_cache_key(&format!("{}\n{}\n{BOOTSTRAP_SEMANTICS}", mix_cdylib_source(), mix_cdylib_shim()));
    let so = build_logos_cdylib("logos_native_mix_cdylib", "logos_native_mix_lib", &format!("mix_{key}"), EngineBuild::Interpretive, native_mix_cdylib_lib_rs)?;
    NATIVE_MIX_CDYLIB.with(|c| *c.borrow_mut() = Some(so.clone()));
    Ok(so)
}

/// `definitions` followed by the lift types of their values ([`crate::optimize::mix_lift_types`]): the functions `mix`
/// asks when it lifts a static value of the program they define.
fn with_lift_types(definitions: String) -> String {
    let types = crate::optimize::mix_lift_types::lift_types_source(&definitions);
    format!("{definitions}\n{types}")
}

/// The self-interpreter's run of a program it reads from `progStmts`/`progFuncs`, with its lift types.
fn self_interpreter_open_source() -> String {
    format!(
        "{}\n## Main\ncoreRun(progStmts, progFuncs, progArgs).\n",
        with_lift_types(format!("{}\n{}", CORE_TYPES_FOR_PE, core_interp_source()))
    )
}

/// `sint²`: the self-interpreter's run ([`self_interpreter_open_source`]) as a two-level program, the program it
/// runs static.
pub fn self_interpreter_two_level() -> Result<crate::optimize::mix_bta::TwoLevelProgram, String> {
    let tree = program_to_core_tree(&self_interpreter_open_source())
        .ok_or_else(|| "the self-interpreter is outside the native builder's coverage".to_string())?;
    crate::optimize::mix_bta::annotate(&tree, &mix_catalog(), &["progStmts", "progFuncs", "progArgs"], &[])
}

/// `mix²`: the specializer's run on a two-level program it reads from `mixMain`, `mixFuncs` and `mixBFuncs` as a
/// two-level program, that program static and the store it specializes with, `pStore`, dynamic.
pub fn specializer_two_level() -> Result<crate::optimize::mix_bta::TwoLevelProgram, String> {
    let run = format!(
        "{}\n## Main\nLet mixResidual be mixProgram(mixMain, pStore, mixFuncs, mixBFuncs).\n{}",
        with_lift_types(format!(
            "{}\n{}\n{}\n{}",
            CORE_TYPES_FOR_PE,
            core_interp_source(),
            decompile_source_text(),
            mix_source_text()
        )),
        MIX_PRINT_RESIDUAL
    );
    let tree = program_to_core_tree(&run)
        .ok_or_else(|| "the specializer's run is outside the native builder's coverage".to_string())?;
    crate::optimize::mix_bta::annotate(&tree, &mix_catalog(), &["mixMain", "mixFuncs", "mixBFuncs"], &["pStore"])
}

/// A first projection of a program (`tree`, its `CProg`) that reads an input only when it runs is refused: the
/// projection would read it while specializing. Its first projection keeps that input dynamic.
fn refuse_run_time_input(tree: &CoreNode, arguments_dynamic: bool) -> Result<(), String> {
    let functions: std::collections::HashSet<String> = match tree {
        CoreNode::Ind(c, parts) if c == "CProg" => match parts.first() {
            Some(CoreNode::List(fs)) => fs
                .iter()
                .filter_map(|f| match f {
                    CoreNode::Ind(fc, fa) if fc == "CFuncDef" => match fa.first() {
                        Some(CoreNode::Text(name)) => Some(name.clone()),
                        _ => None,
                    },
                    _ => None,
                })
                .collect(),
            _ => Default::default(),
        },
        _ => Default::default(),
    };
    match run_time_input(tree, &functions, arguments_dynamic) {
        Some(input) => Err(format!(
            "the program reads {input} when it runs: its first projection keeps that input dynamic, which the \
             self-interpreter's specialization does not carry yet"
        )),
        None => Ok(()),
    }
}

/// The first input a program reads only when it runs, in words, if it reads one: its arguments (unless
/// `arguments_dynamic`: the specialization reads them as a run-time `args()`), the clock, the console or a file, or any
/// builtin whose answer is not a function of its arguments (a random draw, the environment, the page, storage). The
/// self-interpreter makes such a read where the program makes it, so specializing it to the program alone would make
/// the read while specializing. A call by the name of one of the program's `functions` is the program's own. The offline
/// binding-time analysis reads the same reads as dynamic in a program with a dynamic input.
pub(crate) fn run_time_input(node: &CoreNode, functions: &std::collections::HashSet<String>, arguments_dynamic: bool) -> Option<String> {
    use crate::semantics::builtins::{builtin_from_name, system_native, BuiltinEffect};
    match node {
        CoreNode::Ind(c, args) => {
            let read = match c.as_str() {
                "CTimeNow" | "CDateToday" => Some("the clock".to_string()),
                "CReadConsole" => Some("the console".to_string()),
                "CReadFile" => Some("a file".to_string()),
                "CCall" | "CCallS" => match args.first() {
                    Some(CoreNode::Text(name)) if functions.contains(name) => None,
                    Some(CoreNode::Text(name)) if name == "args" => {
                        (!arguments_dynamic).then(|| "its arguments (`args()`)".to_string())
                    }
                    Some(CoreNode::Text(name)) => {
                        let arity = match args.get(1) {
                            Some(CoreNode::List(a)) => a.len(),
                            _ => 0,
                        };
                        builtin_from_name(name)
                            .or_else(|| system_native(name, arity))
                            .filter(|b| matches!(b.effect(), BuiltinEffect::Nondeterministic | BuiltinEffect::Effectful))
                            .map(|_| format!("`{name}`"))
                    }
                    _ => None,
                },
                _ => None,
            };
            read.or_else(|| args.iter().find_map(|a| run_time_input(a, functions, arguments_dynamic)))
        }
        CoreNode::List(items) => items.iter().find_map(|a| run_time_input(a, functions, arguments_dynamic)),
        _ => None,
    }
}

/// `program` as the self-interpreter reads it: `progStmts`, its main block, and `progFuncs`, its functions by name,
/// as its Core IR.
fn program_store(program: &str) -> Result<MixStore, String> {
    use crate::optimize::mix_bta::StoreEntry;
    let tree = program_to_core_tree(&user_program_source(program))
        .ok_or_else(|| "the program is outside the native builder's coverage".to_string())?;
    refuse_run_time_input(&tree, false)?;
    let CoreNode::Ind(c, mut parts) = tree else {
        return Err("the program's Core IR is not a CProg".to_string());
    };
    if c != "CProg" || parts.len() != 2 {
        return Err(format!("the program's Core IR is a `{c}`, not a CProg"));
    }
    let main = parts.pop().expect("two parts");
    let CoreNode::List(funcs) = parts.pop().expect("two parts") else {
        return Err("the program's functions are not a list".to_string());
    };
    let funcs: Vec<(String, CoreNode)> = funcs
        .into_iter()
        .filter_map(|f| match &f {
            CoreNode::Ind(fc, fa) if fc == "CFuncDef" => match fa.first() {
                Some(CoreNode::Text(name)) => Some((name.clone(), f.clone())),
                _ => None,
            },
            _ => None,
        })
        .collect();
    Ok(vec![
        ("progStmts", StoreEntry::Value(main)),
        ("progFuncs", StoreEntry::Map(funcs)),
        ("progArgs", StoreEntry::Value(CoreNode::List(Vec::new()))),
    ])
}

/// The first Futamura projection through the offline specializer, `[[mix]](sint², p)`: the self-interpreter
/// specialized to `program`. The residual is decompiled Core IR.
pub fn mix_projection1(program: &str) -> Result<String, String> {
    mix_projection1_on(MixEngine::default_engine(), program)
}

/// [`mix_projection1`] with `mix` on `engine`.
pub fn mix_projection1_on(engine: MixEngine, program: &str) -> Result<String, String> {
    let residual = mix_apply(&self_interpreter_two_level()?, &program_store(program)?, engine)?;
    Ok(prepend_type_definitions(program, residual))
}

/// The second Futamura projection through the offline specializer, `compiler = [[mix]](mix², sint²)`: the
/// specializer specialized to the self-interpreter. The compiler's main reads the program to compile from `pStore`
/// ([`run_mix_compiler`]).
pub fn mix_projection2() -> Result<String, String> {
    mix_projection2_on(MixEngine::default_engine())
}

/// [`mix_projection2`] with `mix` on `engine`.
pub fn mix_projection2_on(engine: MixEngine) -> Result<String, String> {
    mix_apply(&specializer_two_level()?, &self_interpreter_two_level()?.store_entries(), engine)
}

/// The third Futamura projection through the offline specializer, `cogen = [[mix]](mix², mix²)`: the specializer
/// specialized to itself. Given a two-level program in `pStore` it prints that program's compiler
/// ([`run_mix_cogen`]).
pub fn mix_projection3() -> Result<String, String> {
    mix_projection3_on(MixEngine::default_engine())
}

/// [`mix_projection3`] with `mix` on `engine`.
pub fn mix_projection3_on(engine: MixEngine) -> Result<String, String> {
    let mix2 = specializer_two_level()?;
    mix_apply(&mix2, &mix2.store_entries(), engine)
}

/// A projection's residual whose main reads `pStore`, as the program it runs as with `store`: the type definitions
/// of the programs it was specialized from, its functions, the store, then its main.
fn mix_residual_run_source(residual: &str, store: &MixStore) -> Result<String, String> {
    let (functions, main) = residual
        .split_once("## Main\n")
        .ok_or_else(|| "the residual has no `## Main`".to_string())?;
    Ok(format!(
        "{}\n{}\n{}\n{}\n## Main\nLet pStore be mixBuildStore().\n{}",
        CORE_TYPES_FOR_PE,
        mix_type_definitions(),
        functions,
        crate::optimize::mix_bta::store_builder("mixBuildStore", store, &mix_catalog())?,
        main
    ))
}

/// A projection's residual whose main reads `pStore`, run on the tree-walker with `store`.
fn run_mix_residual(residual: &str, store: &MixStore) -> Result<String, String> {
    let run = mix_residual_run_source(residual, store)?;
    let raw = interpret_bootstrap(&run).map_err(|e| format!("the residual failed: {}", describe_parse_error(&e)))?;
    Ok(raw.trim_end().to_string())
}

/// The program the compiler [`mix_projection2`] made runs as on `program`: any engine that runs it prints the
/// program's first projection ([`run_mix_compiler`] runs it on the tree-walker).
pub fn mix_compiler_run_source(compiler: &str, program: &str) -> Result<String, String> {
    mix_residual_run_source(compiler, &program_store(program)?)
}

/// The compiler [`mix_projection2`] made, run on `program`: it prints the program's first projection.
pub fn run_mix_compiler(compiler: &str, program: &str) -> Result<String, String> {
    let residual = run_mix_residual(compiler, &program_store(program)?)?;
    Ok(prepend_type_definitions(program, residual))
}

/// The compiler generator [`mix_projection3`] made, run on the self-interpreter's two-level program: it prints the
/// self-interpreter's compiler.
pub fn run_mix_cogen(cogen: &str) -> Result<String, String> {
    run_mix_residual(cogen, &self_interpreter_two_level()?.store_entries())
}

/// The input a residual engine ([`residual_engine_source`]) takes: the store its main reads as `pStore`.
const RESIDUAL_INPUT: &str = "## A ResidualInput is one of:\n    A ResidualIn with store Map of Text to CVal.\n";

/// A projection's residual whose main reads `pStore` and shows its one answer, as an engine to compile once: the type
/// definitions of the programs it was specialized from, its functions, and its main as `residualAnswer`, exported for
/// native, which takes the store and returns the answer.
pub fn residual_engine_source(residual: &str) -> Result<String, String> {
    let (functions, main) = residual
        .split_once("## Main\n")
        .ok_or_else(|| "the residual has no `## Main`".to_string())?;
    let statements: Vec<&str> = main.trim_end().lines().collect();
    let (last, before) = statements.split_last().ok_or_else(|| "the residual's main is empty".to_string())?;
    let answer = last
        .strip_prefix("Show ")
        .and_then(|a| a.strip_suffix('.'))
        .ok_or_else(|| format!("the residual's main ends with `{last}`, not the `Show` of its answer"))?;
    if let Some(shown) = before.iter().find(|l| l.trim_start().starts_with("Show ")) {
        return Err(format!("the residual's main shows `{}` besides its answer", shown.trim()));
    }
    let mut body = String::new();
    for statement in before {
        body.push_str(&format!("            {statement}\n"));
    }
    body.push_str(&format!("            Return {answer}.\n"));
    Ok(format!(
        "{}\n{}\n{}\n{RESIDUAL_INPUT}\n## To residualAnswer (residualInput: ResidualInput) -> Text is exported for native:\n    Inspect residualInput:\n        When ResidualIn (pStore):\n{body}        Otherwise:\n            Return \"\".\n\n## Main\n    Let __x be 0.\n",
        CORE_TYPES_FOR_PE,
        mix_type_definitions(),
        functions
    ))
}

/// A projection's residual compiled once to native code ([`residual_engine_source`]) and loaded in process: each run
/// hands it a store as wire bytes and takes back its answer.
#[cfg(not(target_arch = "wasm32"))]
struct NativeResidual {
    lib: NativeLib,
}

#[cfg(not(target_arch = "wasm32"))]
impl NativeResidual {
    /// `residual`'s engine, built once and cached, content-addressed on its source, its FFI entry and the collection
    /// semantics it compiles under.
    fn build(residual: &str) -> Result<NativeResidual, String> {
        let source = residual_engine_source(residual)?;
        let shim = native_engine_shim("logos_residual_answer", "ResidualInput", "residualAnswer");
        let key = aot_cache_key(&format!("{source}\n{shim}\n{BOOTSTRAP_SEMANTICS}"));
        let so = build_logos_cdylib("logos_native_residual_cdylib", "logos_native_residual_lib", &format!("residual_{key}"), EngineBuild::Residual, || {
            let generated = compile_bootstrap_engine(&source)
                .map_err(|e| format!("native residual compile: {}", describe_parse_error(&e)))?
                .rust_code;
            Ok(format!("{generated}\n{shim}"))
        })?;
        Ok(NativeResidual { lib: NativeLib::load(&so, b"logos_residual_answer")? })
    }

    /// The residual's answer with `store` as `pStore`, as [`run_mix_residual`] gives it on the tree-walker.
    fn run(&self, store: &MixStore) -> Result<String, String> {
        let bytes = crate::optimize::mix_bta::residual_input_wire(store, &mix_catalog())?;
        Ok(self.lib.call(&bytes).map_err(|e| format!("the residual failed: {e}"))?.trim_end().to_string())
    }
}

/// The compiler [`mix_projection2`] made, compiled once to native code: [`NativeMixCompiler::compile`] runs it on a
/// program as [`run_mix_compiler`] runs it on the tree-walker.
#[cfg(not(target_arch = "wasm32"))]
pub struct NativeMixCompiler(NativeResidual);

#[cfg(not(target_arch = "wasm32"))]
impl NativeMixCompiler {
    /// `compiler` compiled to native code, built once and cached.
    pub fn build(compiler: &str) -> Result<NativeMixCompiler, String> {
        NativeResidual::build(compiler).map(NativeMixCompiler)
    }

    /// The compiler run on `program`: the program's first projection.
    pub fn compile(&self, program: &str) -> Result<String, String> {
        let residual = self.0.run(&program_store(program)?)?;
        Ok(prepend_type_definitions(program, residual))
    }
}

/// The compiler generator [`mix_projection3`] made, compiled to native code and run on the self-interpreter's
/// two-level program, as [`run_mix_cogen`] runs it on the tree-walker: it prints the self-interpreter's compiler.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_native_mix_cogen(cogen: &str) -> Result<String, String> {
    NativeResidual::build(cogen)?.run(&self_interpreter_two_level()?.store_entries())
}

/// The driver shared by the textbook first projection, the resident-server PE (`pe_native_source`, terminal statement
/// writes the residual to stdout) and the in-process cdylib PE (`pe_cdylib_source`, terminal statement `Return`s it):
/// specialize `encodedMain` against `encodedFuncMap`, then `peEmitProgram` emits the residual — every function it
/// reaches once, each body and the main block through the specializer's finishing pass. Written at 0-base indent; the
/// caller re-indents it under its enclosing arm.
const PE_DRIVER_CORE: &str = r#"Let state be makePeState(a new Map of Text to CVal, encodedFuncMap, 200).
Let residual be peBlock(encodedMain, state).
Let output be peEmitProgram(residual, state)."#;

/// The PE driver body with `last` appended as its terminal statement — the only line that differs
/// between the stdout-writing server, the value-returning cdylib entry and the textbook projection.
fn pe_driver_body(last: &str) -> String {
    format!("{PE_DRIVER_CORE}\n{last}")
}

/// The compile-once native partial evaluator's source: the PE engine + decompiler + a driver that reads its input
/// `CProgram` from stdin (over the fast wire codec) instead of a baked-in constructor. Compiled once (content-addressed
/// on this text) with `WireEncode`/`WireDecode` emitted, it specializes any program handed to it as
/// `program_to_core_wire_bytes` — the Futamura "compiler as an artifact", at native speed. Its driver body is
/// [`PE_DRIVER_CORE`]; only `encodedMain`/`encodedFuncMap` come from `readWireProgram()` rather than the encoded program
/// text.
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
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("                {l}")
            }
        })
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
/// round-trip is gone. The driver `is exported for native`: host code calls it, so it keeps the
/// signature it declares, its program taken by value, whatever the emitter would lend an internal
/// function. `## Main` is a trivial stub (a cdylib never runs it).
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
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("            {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let func = format!(
        "## To peSpecializeOnce (__prog: CProgram) -> Text is exported for native:\n    Inspect __prog:\n        When CProg (encFuncs, encMain):\n            Let encodedMain be encMain.\n            Let mutable encodedFuncMap be a new Map of Text to CFunc.\n            Repeat for __f in encFuncs:\n                Inspect __f:\n                    When CFuncDef (__nm, __ps, __pt, __rt, __bd):\n                        Set item __nm of encodedFuncMap to __f.\n                    Otherwise:\n                        Let __skip be true.\n{indented}\n        Otherwise:\n            Return \"\".\n"
    );
    format!(
        "{}\n{}\n{}\n{}\n## Main\n    Let __x be 0.\n",
        core, pe, decompile, func
    )
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

/// The Core-IR enum a constructor belongs to, read from the catalog the encoders and the partial evaluator share
/// ([`CORE_TYPES_FOR_PE`]) — so the built value's `inductive_type` is the one the interpreter records (`a new CInt` →
/// `CExpr`) and the native builder's wire bytes match the interpreter path byte for byte. The native decoder ignores
/// this field; matching it keeps every byte-level lock exact.
fn cv_enum(ctor: &str) -> &'static str {
    static TABLE: std::sync::OnceLock<HashMap<&'static str, &'static str>> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = HashMap::new();
        let mut current = "";
        for line in CORE_TYPES_FOR_PE.lines() {
            if let Some(rest) = line.strip_prefix("## A ") {
                current = rest.split_whitespace().next().unwrap_or("");
            } else if let Some(rest) = line.strip_prefix("    A ") {
                let name = rest
                    .split(|c: char| c == ' ' || c == '.')
                    .next()
                    .unwrap_or("");
                table.insert(name, current);
            }
        }
        table
    });
    table
        .get(ctor)
        .copied()
        .unwrap_or_else(|| panic!("`{ctor}` is not a constructor of the Core IR catalog"))
}

fn cv_ind(ctor: &str, args: Vec<RuntimeValue>) -> RuntimeValue {
    RuntimeValue::Inductive(std::rc::Rc::new(crate::interpreter::InductiveValue {
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

/// One of a program's enum variants as the encoders read it: the fields its constructor takes (a constructor call
/// names them), its enum, and every variant of that enum.
struct VariantCtor {
    fields: Vec<String>,
    enum_name: String,
    siblings: Vec<String>,
}

/// A program's declarations as the encoders read them: its enum variants by name, each struct's fields with the type
/// name each declares (`interpreter::declared_field_type`) in declaration order, and its policies.
struct VCtors {
    ctors: HashMap<String, VariantCtor>,
    structs: HashMap<String, Vec<(String, String)>>,
    /// For each predicate (`false`) or capability (`true`) a `## Policy` block names, every subject type that defines it
    /// with its condition there, by type name.
    policies: HashMap<(String, bool), Vec<(String, PolicyNode)>>,
}

/// A policy condition as the Core IR holds it for a subject of one type (`CPolicy`): each predicate it names resolved
/// to that type's own condition, as `semantics::policy::evaluate_policy_condition` resolves it when it runs.
#[derive(Clone)]
enum PolicyNode {
    /// The subject's field compared with the policy's text: as text, or — when the policy wrote no string literal —
    /// as the Int or Bool the text reads as.
    FieldEquals { field: String, expected: String, literal: bool, as_int: Option<i64>, as_bool: Option<bool> },
    FieldBool { field: String, value: bool },
    /// A subject field and an object field, compared as the language compares values.
    FieldsEqual { subject_field: String, object_field: String },
    And(Box<PolicyNode>, Box<PolicyNode>),
    Or(Box<PolicyNode>, Box<PolicyNode>),
    /// A predicate the subject's type does not define: it does not hold.
    False,
    /// A predicate met again inside its own condition.
    Recursive(String),
}

/// `condition` for a subject of type `subject_type`, the predicates it names resolved through `policies`; `visiting`
/// holds the predicates being resolved.
fn policy_node(
    condition: &crate::analysis::PolicyCondition,
    policies: &crate::analysis::PolicyRegistry,
    interner: &Interner,
    subject_type: crate::intern::Symbol,
    visiting: &mut Vec<crate::intern::Symbol>,
) -> PolicyNode {
    use crate::analysis::PolicyCondition as C;
    match condition {
        C::FieldEquals { field, value, is_string_literal } => {
            let expected = interner.resolve(*value).to_string();
            PolicyNode::FieldEquals {
                field: interner.resolve(*field).to_string(),
                as_int: expected.parse::<i64>().ok(),
                as_bool: expected.parse::<bool>().ok(),
                expected,
                literal: *is_string_literal,
            }
        }
        C::FieldBool { field, value } => PolicyNode::FieldBool { field: interner.resolve(*field).to_string(), value: *value },
        C::ObjectFieldEquals { subject, field, .. } => PolicyNode::FieldsEqual {
            subject_field: interner.resolve(*subject).to_string(),
            object_field: interner.resolve(*field).to_string(),
        },
        C::SubjectFieldEqualsObjectField { subject_field, object_field, .. } => PolicyNode::FieldsEqual {
            subject_field: interner.resolve(*subject_field).to_string(),
            object_field: interner.resolve(*object_field).to_string(),
        },
        C::Or(left, right) => PolicyNode::Or(
            Box::new(policy_node(left, policies, interner, subject_type, visiting)),
            Box::new(policy_node(right, policies, interner, subject_type, visiting)),
        ),
        C::And(left, right) => PolicyNode::And(
            Box::new(policy_node(left, policies, interner, subject_type, visiting)),
            Box::new(policy_node(right, policies, interner, subject_type, visiting)),
        ),
        C::Predicate { predicate, .. } => {
            if visiting.contains(predicate) {
                return PolicyNode::Recursive(interner.resolve(*predicate).to_string());
            }
            let found = policies
                .get_predicates(subject_type)
                .and_then(|defs| defs.iter().find(|d| d.predicate_name == *predicate));
            match found {
                Some(def) => {
                    visiting.push(*predicate);
                    let node = policy_node(&def.condition, policies, interner, subject_type, visiting);
                    visiting.pop();
                    node
                }
                None => PolicyNode::False,
            }
        }
    }
}

/// Every predicate and capability `policies` defines, each with the subject types defining it and their conditions
/// (see [`VCtors::policies`]); a type's first definition of a name is the one a `Check` reads, as the language reads it.
fn policy_alternatives(
    policies: &crate::analysis::PolicyRegistry,
    interner: &Interner,
) -> HashMap<(String, bool), Vec<(String, PolicyNode)>> {
    let mut out: HashMap<(String, bool), Vec<(String, PolicyNode)>> = HashMap::new();
    for (ty, defs) in policies.iter_predicates() {
        let mut named: Vec<crate::intern::Symbol> = Vec::new();
        for def in defs {
            if named.contains(&def.predicate_name) {
                continue;
            }
            named.push(def.predicate_name);
            let node = policy_node(&def.condition, policies, interner, *ty, &mut vec![def.predicate_name]);
            out.entry((interner.resolve(def.predicate_name).to_string(), false))
                .or_default()
                .push((interner.resolve(*ty).to_string(), node));
        }
    }
    for (ty, defs) in policies.iter_capabilities() {
        let mut named: Vec<crate::intern::Symbol> = Vec::new();
        for def in defs {
            if named.contains(&def.action) {
                continue;
            }
            named.push(def.action);
            let node = policy_node(&def.condition, policies, interner, *ty, &mut Vec::new());
            out.entry((interner.resolve(def.action).to_string(), true))
                .or_default()
                .push((interner.resolve(*ty).to_string(), node));
        }
    }
    for alternatives in out.values_mut() {
        alternatives.sort_by(|a, b| a.0.cmp(&b.0));
    }
    out
}

/// A policy condition as construction text: its variable.
fn policy_src(node: &PolicyNode, counter: &mut usize, output: &mut String) -> String {
    let built = match node {
        PolicyNode::FieldEquals { field, expected, literal, as_int, as_bool } => format!(
            "a new PFieldEquals with field {} and expected {} and literal {} and asInt {} and isInt {} and asBool {} and isBool {}",
            text_literal(field),
            text_literal(expected),
            literal,
            as_int.unwrap_or(0),
            as_int.is_some(),
            as_bool.unwrap_or(false),
            as_bool.is_some()
        ),
        PolicyNode::FieldBool { field, value } => format!("a new PFieldBool with field {} and value {value}", text_literal(field)),
        PolicyNode::FieldsEqual { subject_field, object_field } => format!(
            "a new PFieldsEqual with subjectField {} and objectField {}",
            text_literal(subject_field),
            text_literal(object_field)
        ),
        PolicyNode::And(left, right) | PolicyNode::Or(left, right) => {
            let l = policy_src(left, counter, output);
            let r = policy_src(right, counter, output);
            let word = if matches!(node, PolicyNode::And(..)) { "PAnd" } else { "POr" };
            format!("a new {word} with left {l} and right {r}")
        }
        PolicyNode::False => "a new PFalse".to_string(),
        PolicyNode::Recursive(name) => format!("a new PRecursive with predicate {}", text_literal(name)),
    };
    let var = format!("pol_{}", *counter);
    *counter += 1;
    output.push_str(&format!("Let {var} be {built}.\n"));
    var
}

/// [`policy_src`] on a [`CoreSink`].
fn policy_core<S: CoreSink>(node: &PolicyNode) -> S::Val {
    match node {
        PolicyNode::FieldEquals { field, expected, literal, as_int, as_bool } => S::inductive(
            "PFieldEquals",
            vec![
                S::text(field),
                S::text(expected),
                S::boolean(*literal),
                S::int(as_int.unwrap_or(0)),
                S::boolean(as_int.is_some()),
                S::boolean(as_bool.unwrap_or(false)),
                S::boolean(as_bool.is_some()),
            ],
        ),
        PolicyNode::FieldBool { field, value } => S::inductive("PFieldBool", vec![S::text(field), S::boolean(*value)]),
        PolicyNode::FieldsEqual { subject_field, object_field } => {
            S::inductive("PFieldsEqual", vec![S::text(subject_field), S::text(object_field)])
        }
        PolicyNode::And(left, right) => S::inductive("PAnd", vec![policy_core::<S>(left), policy_core::<S>(right)]),
        PolicyNode::Or(left, right) => S::inductive("POr", vec![policy_core::<S>(left), policy_core::<S>(right)]),
        PolicyNode::False => S::inductive("PFalse", vec![]),
        PolicyNode::Recursive(name) => S::inductive("PRecursive", vec![S::text(name)]),
    }
}

/// The parts of a `Check` as the Core IR's `CCheckPolicy` holds them: its subject's and object's names, whether it asks
/// a capability, the predicate or action it names, and its source text.
fn check_parts(stmt: &Stmt, interner: &Interner) -> Option<(String, String, bool, String, String)> {
    let Stmt::Check { subject, predicate, is_capability, object, source_text, .. } = stmt else {
        return None;
    };
    Some((
        interner.resolve(*subject).to_string(),
        object.map(|o| interner.resolve(o).to_string()).unwrap_or_default(),
        *is_capability,
        interner.resolve(*predicate).to_string(),
        source_text.to_string(),
    ))
}

impl VCtors {
    /// The enum variant named `variant`.
    fn get(&self, variant: &str) -> Option<&VariantCtor> {
        self.ctors.get(variant)
    }

    /// The fields of the struct named `name`, with their declared type names.
    fn struct_fields(&self, name: &str) -> Option<&[(String, String)]> {
        self.structs.get(name).map(Vec::as_slice)
    }
}

/// Every enum variant and struct `registry` declares and every policy `policies` defines (see [`VCtors`]).
fn variant_ctors(
    registry: &crate::analysis::TypeRegistry,
    policies: &crate::analysis::PolicyRegistry,
    interner: &Interner,
) -> VCtors {
    let mut ctors = HashMap::new();
    let mut structs = HashMap::new();
    for (enum_name, def) in registry.iter_types() {
        if let crate::analysis::TypeDef::Struct { fields, .. } = def {
            structs.insert(
                interner.resolve(*enum_name).to_string(),
                fields
                    .iter()
                    .map(|f| {
                        let ty = crate::interpreter::declared_field_type(&f.ty);
                        (interner.resolve(f.name).to_string(), interner.resolve(ty).to_string())
                    })
                    .collect(),
            );
        }
        if let crate::analysis::TypeDef::Enum { variants, .. } = def {
            let siblings: Vec<String> = variants.iter().map(|v| interner.resolve(v.name).to_string()).collect();
            for v in variants {
                ctors.insert(
                    interner.resolve(v.name).to_string(),
                    VariantCtor {
                        fields: v.fields.iter().map(|f| interner.resolve(f.name).to_string()).collect(),
                        enum_name: interner.resolve(*enum_name).to_string(),
                        siblings: siblings.clone(),
                    },
                );
            }
        }
    }
    VCtors { ctors, structs, policies: policy_alternatives(policies, interner) }
}

/// The fields of the struct a construction of `type_name` builds that it leaves out, with the type name each declares:
/// each starts at its type's default, as the tree-walker starts it (`interpreter::default_for_declared_type`).
fn omitted_fields<'v>(type_name: &str, given: &[String], ctors: &'v VCtors) -> Vec<&'v (String, String)> {
    ctors
        .struct_fields(type_name)
        .map(|fields| fields.iter().filter(|(name, _)| !given.contains(name)).collect())
        .unwrap_or_default()
}

/// `CAtPath` of the place `root`'s `path` around the encoded write `inner`, as construction text: its variable. The
/// write is the place's block, so every engine runs it as it runs any block the program holds.
fn at_path_src(root: &str, path: &[String], inner: &str, counter: &mut usize, output: &mut String) -> String {
    let path_var = format!("path_{}", *counter);
    *counter += 1;
    output.push_str(&format!("Let {} be a new Seq of Text.\n", path_var));
    for field in path {
        output.push_str(&format!("Push \"{}\" to {}.\n", field, path_var));
    }
    let body_var = format!("body_{}", *counter);
    *counter += 1;
    output.push_str(&format!("Let {} be a new Seq of CStmt.\nPush {} to {}.\n", body_var, inner, body_var));
    let at = format!("s_{}", *counter);
    *counter += 1;
    output.push_str(&format!(
        "Let {} be a new CAtPath with target \"{}\" and path {} and body {}.\n",
        at, root, path_var, body_var
    ));
    at
}

/// A format spec as the Core IR holds it beside its text: its form — `$` for currency, `.` for a precision, `>`, `<` or
/// `^` for an alignment, a bare width aligning right as `>` — and its number of digits or width (0 for currency), as
/// `semantics::format::apply_format_spec` reads the spec the parser accepted.
fn spec_form(spec: &str) -> (&'static str, i64) {
    let number = |digits: &str| digits.parse::<i64>().unwrap_or(0);
    match spec.chars().next() {
        Some('$') => ("$", 0),
        Some('.') => (".", number(&spec[1..])),
        Some('>') => (">", number(&spec[1..])),
        Some('<') => ("<", number(&spec[1..])),
        Some('^') => ("^", number(&spec[1..])),
        _ => (">", number(spec)),
    }
}

/// The construction text pushing an interpolation hole's part, its value `value_var`, to `parts_var`: a
/// `CFormattedPart` when the hole names a format spec (`{x:.2}`), else a `CExprPart`.
fn string_part_src(
    value_var: &str,
    format_spec: Option<crate::intern::Symbol>,
    parts_var: &str,
    interner: &Interner,
) -> String {
    match format_spec {
        Some(spec) => {
            let spec = interner.resolve(spec);
            let (form, size) = spec_form(spec);
            format!(
                "Push a new CFormattedPart with expr {value_var} and spec {} and form {} and size {size} to {parts_var}.\n",
                text_literal(spec),
                text_literal(form)
            )
        }
        None => format!("Push a new CExprPart with expr {value_var} to {parts_var}.\n"),
    }
}

/// What the self-interpreter awaits to evaluate a view template or a question about the rendered page.
pub const PAGE_AWAITS: &str = "a rendered page";

/// What the Core IR holds as the program spelled it — an expression of a family the self-interpreter carries no
/// value of yet — with what the self-interpreter awaits to evaluate it: `CSpelled with code and awaits`. A
/// specializer keeps it residual and the decompiler prints its code, so a residual means what the program meant; the
/// self-interpreter stops on it with what it awaits.
fn spelled_in_core_ir(expr: &Expr, interner: &Interner) -> Option<(String, &'static str)> {
    let awaits = match expr {
        Expr::View(_) | Expr::Dom(_) => PAGE_AWAITS,
        _ => return None,
    };
    Some((decompile_expr(expr, interner), awaits))
}

/// [`spelled_in_core_ir`] as construction text bound to `var`.
fn spelled_src(expr: &Expr, var: &str, output: &mut String, interner: &Interner) {
    let (code, awaits) = spelled_in_core_ir(expr, interner).expect("a spelled expression is one the Core IR spells");
    output.push_str(&format!(
        "Let {var} be a new CSpelled with code {} and awaits {}.\n",
        text_literal(&code),
        text_literal(awaits)
    ));
}

/// A field of a construction as the Core IR holds it: the construction's own field at this position, or a slot it
/// leaves to its declared type's default (`omitted_fields`: field name, declared type).
enum HeldField<'v> {
    Written(usize),
    Slot(&'v (String, String)),
}

/// A construction's fields in the order the Core IR holds them: the written ones in their own order, each slot the
/// construction leaves out placed before the first written field whose name follows its own — a record holds its
/// fields by name, the order the language prints them in and `core_ir_scope` holds the written ones in.
fn held_fields<'v>(type_name: &str, written: &[String], ctors: &'v VCtors) -> Vec<HeldField<'v>> {
    let mut slots = omitted_fields(type_name, written, ctors);
    slots.sort_by(|a, b| a.0.cmp(&b.0));
    let mut slots = slots.into_iter().peekable();
    let mut held = Vec::with_capacity(written.len() + slots.len());
    for (i, name) in written.iter().enumerate() {
        while let Some(slot) = slots.next_if(|slot| slot.0 < *name) {
            held.push(HeldField::Slot(slot));
        }
        held.push(HeldField::Written(i));
    }
    held.extend(slots.map(HeldField::Slot));
    held
}

/// The Core IR expression a slot of declared type `type_name` starts at, as construction text: `CDeclaredDefault`, which
/// every engine reads by the one table (`interpreter::declared_default`) and the decompiler leaves out, as the program
/// did. Its variable.
fn declared_default_src(type_name: &str, counter: &mut usize, output: &mut String) -> String {
    let var = format!("e_{}", *counter);
    *counter += 1;
    output.push_str(&format!("Let {var} be a new CDeclaredDefault with typeName {}.\n", text_literal(type_name)));
    var
}

/// [`declared_default_src`] on a [`CoreSink`].
fn declared_default_core<S: CoreSink>(type_name: &str) -> S::Val {
    S::inductive("CDeclaredDefault", vec![S::text(type_name)])
}

/// The variants an `Inspect` with these arms leaves to no arm, each with the language's report
/// (`builtins::inspect_unhandled_message`): every variant of the enum its arms name that no arm names, when no arm is
/// `Otherwise`. An encoder adds an arm per variant that stops with its report, as the VM compiler does, so every
/// engine running the Core IR stops where the language does.
fn uncovered_variants(arms: &[crate::ast::stmt::MatchArm], interner: &Interner, ctors: &VCtors) -> Vec<(String, String)> {
    if arms.iter().any(|arm| arm.variant.is_none()) {
        return Vec::new();
    }
    let named: Vec<String> = arms.iter().filter_map(|arm| arm.variant.map(|v| interner.resolve(v).to_string())).collect();
    let Some(ctor) = named.iter().find_map(|n| ctors.get(n)) else {
        return Vec::new();
    };
    ctor.siblings
        .iter()
        .filter(|v| !named.contains(v))
        .map(|v| (v.clone(), crate::semantics::builtins::inspect_unhandled_message(&ctor.enum_name, v)))
        .collect()
}

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
    fn atom(a: CoreAtom) -> Self::Val;
    fn list(items: Vec<Self::Val>) -> Self::Val;
    fn inductive(ctor: &str, args: Vec<Self::Val>) -> Self::Val;
}

/// A value of a family the Core IR holds as the language's own value, in the field `value` of its constant
/// ([`CoreAtom::constructor`]): a duration, a calendar date, an instant, a calendar span, a time of day, a character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreAtom {
    Duration(i64),
    Date(i32),
    Moment(i64),
    Span { months: i32, days: i32 },
    Time(i64),
    Char(char),
}

impl CoreAtom {
    /// The atom a literal spells, `None` for a literal of another family.
    pub fn of_literal(lit: &Literal) -> Option<CoreAtom> {
        Some(match lit {
            Literal::Duration(ns) => CoreAtom::Duration(*ns),
            Literal::Date(days) => CoreAtom::Date(*days),
            Literal::Moment(ns) => CoreAtom::Moment(*ns),
            Literal::Span { months, days } => CoreAtom::Span { months: *months, days: *days },
            Literal::Time(ns) => CoreAtom::Time(*ns),
            Literal::Char(c) => CoreAtom::Char(*c),
            _ => return None,
        })
    }

    /// The Core IR constant that holds it.
    pub fn constructor(self) -> &'static str {
        match self {
            CoreAtom::Duration(_) => "CDuration",
            CoreAtom::Date(_) => "CDate",
            CoreAtom::Moment(_) => "CMoment",
            CoreAtom::Span { .. } => "CSpan",
            CoreAtom::Time(_) => "CTime",
            CoreAtom::Char(_) => "CChar",
        }
    }

    /// The value as the tree-walker holds it.
    pub fn runtime_value(self) -> RuntimeValue {
        match self {
            CoreAtom::Duration(ns) => RuntimeValue::Duration(ns),
            CoreAtom::Date(days) => RuntimeValue::Date(days),
            CoreAtom::Moment(ns) => RuntimeValue::Moment(ns),
            CoreAtom::Span { months, days } => RuntimeValue::Span { months, days },
            CoreAtom::Time(ns) => RuntimeValue::Time(ns),
            CoreAtom::Char(c) => RuntimeValue::Char(c),
        }
    }

    /// The language's expression for the value: its literal when one spells it, else literals and the language's own
    /// arithmetic that evaluate to exactly it — a negative duration as one subtracted from `0ns`, an instant between
    /// minutes as a minute and the nanoseconds from it, a time of day between minutes as the time of such an instant,
    /// a date past a four-digit year by `makeDate`. A span's parts are spelled as the parser reads a count, modulo 2³².
    pub fn expression(self) -> String {
        const MINUTE: i64 = 60_000_000_000;
        match self {
            CoreAtom::Char(c) => char_literal(c),
            CoreAtom::Duration(ns) => duration_literal(ns).unwrap_or_else(|| match ns.checked_neg() {
                Some(magnitude) => format!("(0ns - {magnitude}ns)"),
                None => format!("(0ns - {}ns - 1ns)", i64::MAX),
            }),
            CoreAtom::Date(days) => date_literal(days).unwrap_or_else(|| {
                let (y, m, d) = logicaffeine_base::temporal::civil_from_days(i64::from(days));
                format!("makeDate({y}, {m}, {d})")
            }),
            CoreAtom::Moment(ns) => moment_literal(ns).unwrap_or_else(|| {
                let at_minute = |minute: i64| {
                    moment_literal(minute).expect("every instant an i64 of nanoseconds holds falls in a four-digit year")
                };
                let past = ns.rem_euclid(MINUTE);
                match ns.checked_sub(past) {
                    Some(minute) => format!("({} + {past}ns)", at_minute(minute)),
                    None => format!("({} - {}ns)", at_minute(ns + (MINUTE - past)), MINUTE - past),
                }
            }),
            CoreAtom::Time(ns) => time_literal(ns).unwrap_or_else(|| {
                let of_day = ns.rem_euclid(logicaffeine_base::temporal::NANOS_PER_DAY);
                format!("time_of(1970-01-01 at 12:00am + {of_day}ns)")
            }),
            CoreAtom::Span { months, days } => span_literal(months, days).unwrap_or_else(|| match (months as u32, days as u32) {
                (0, d) => format!("{d} days"),
                (m, 0) => format!("{m} months"),
                (m, d) => format!("{m} months and {d} days"),
            }),
        }
    }

    /// The self-interpreter's value holding it.
    pub fn value_variant(self) -> &'static str {
        match self {
            CoreAtom::Duration(_) => "VDuration",
            CoreAtom::Date(_) => "VDate",
            CoreAtom::Moment(_) => "VMoment",
            CoreAtom::Span { .. } => "VSpan",
            CoreAtom::Time(_) => "VTime",
            CoreAtom::Char(_) => "VChar",
        }
    }

    /// Appends its wire bytes, as the peer codec writes the tree-walker's value of it.
    pub fn write_wire(self, out: &mut Vec<u8>) {
        use logicaffeine_data::wire::{write_uvarint, zigzag, T_CHAR, T_DATE, T_DURATION, T_MOMENT, T_SPAN, T_TIME};
        match self {
            CoreAtom::Duration(ns) => {
                out.push(T_DURATION);
                write_uvarint(zigzag(ns), out);
            }
            CoreAtom::Date(days) => {
                out.push(T_DATE);
                write_uvarint(zigzag(i64::from(days)), out);
            }
            CoreAtom::Moment(ns) => {
                out.push(T_MOMENT);
                write_uvarint(zigzag(ns), out);
            }
            CoreAtom::Span { months, days } => {
                out.push(T_SPAN);
                write_uvarint(zigzag(i64::from(months)), out);
                write_uvarint(zigzag(i64::from(days)), out);
            }
            CoreAtom::Time(ns) => {
                out.push(T_TIME);
                write_uvarint(zigzag(ns), out);
            }
            CoreAtom::Char(c) => {
                out.push(T_CHAR);
                write_uvarint(u64::from(c), out);
            }
        }
    }

    /// The Core IR constant as construction text: `a new CMoment with value (<expression>)`.
    pub fn construction(self) -> String {
        format!("a new {} with value ({})", self.constructor(), self.expression())
    }
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
    fn atom(a: CoreAtom) -> RuntimeValue {
        a.runtime_value()
    }
    fn list(items: Vec<RuntimeValue>) -> RuntimeValue {
        cv_list(items)
    }
    fn inductive(ctor: &str, args: Vec<RuntimeValue>) -> RuntimeValue {
        cv_ind(ctor, args)
    }
}

/// A program's Core IR as a plain tree, for analyses written in Rust (the binding-time analysis the offline
/// specializer reads): the same constructors, in the same order, as the other backends build.
#[derive(Clone, Debug, PartialEq)]
pub enum CoreNode {
    Int(i64),
    Bool(bool),
    Float(f64),
    Text(String),
    Atom(CoreAtom),
    List(Vec<CoreNode>),
    Ind(String, Vec<CoreNode>),
}

struct NodeSink;
impl CoreSink for NodeSink {
    type Val = CoreNode;
    fn int(n: i64) -> CoreNode {
        CoreNode::Int(n)
    }
    fn boolean(b: bool) -> CoreNode {
        CoreNode::Bool(b)
    }
    fn float(f: f64) -> CoreNode {
        CoreNode::Float(f)
    }
    fn text(s: &str) -> CoreNode {
        CoreNode::Text(s.to_string())
    }
    fn atom(a: CoreAtom) -> CoreNode {
        CoreNode::Atom(a)
    }
    fn list(items: Vec<CoreNode>) -> CoreNode {
        CoreNode::List(items)
    }
    fn inductive(ctor: &str, args: Vec<CoreNode>) -> CoreNode {
        CoreNode::Ind(ctor.to_string(), args)
    }
}

/// `program`'s Core IR as a [`CoreNode`] tree (`CProg(funcs, main)`), built by the one traversal the wire and
/// tree encoders share. `None` if a construct is outside the native builder's coverage.
pub fn program_to_core_tree(program: &str) -> Option<CoreNode> {
    program_to_core::<NodeSink>(program)
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
        vec![if b {
            logicaffeine_data::wire::T_TRUE
        } else {
            logicaffeine_data::wire::T_FALSE
        }]
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
    fn atom(a: CoreAtom) -> Vec<u8> {
        let mut v = Vec::with_capacity(11);
        a.write_wire(&mut v);
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
        logicaffeine_data::wire::write_inductive_header(
            &mut v,
            cv_enum(ctor),
            ctor,
            args.len() as u64,
        );
        for a in &args {
            v.extend_from_slice(a);
        }
        v
    }
}

fn exprs_to_core<S: CoreSink>(
    args: &[&Expr],
    interner: &Interner,
    vctors: &VCtors,
) -> Option<Vec<S::Val>> {
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
        Expr::Literal(Literal::Text(s)) => {
            S::inductive("CText", vec![S::text(interner.resolve(*s))])
        }
        Expr::Literal(Literal::Float(f)) => S::inductive("CFloat", vec![S::float(*f)]),
        Expr::Literal(
            lit @ (Literal::Duration(_)
            | Literal::Char(_)
            | Literal::Date(_)
            | Literal::Moment(_)
            | Literal::Span { .. }
            | Literal::Time(_)),
        ) => {
            let atom = CoreAtom::of_literal(lit)?;
            S::inductive(atom.constructor(), vec![S::atom(atom)])
        }
        Expr::Literal(Literal::Nothing) => S::inductive("CNothing", vec![]),
        Expr::Identifier(sym) => match clock_read(interner.resolve(*sym)) {
            Some(read) => S::inductive(read, Vec::new()),
            None => S::inductive("CVar", vec![S::text(interner.resolve(*sym))]),
        },
        Expr::BinaryOp { op, left, right } => S::inductive(
            "CBinOp",
            vec![
                S::text(crate::semantics::operator_overloads::core_op_text(*op)),
                expr_to_core::<S>(left, interner, vctors)?,
                expr_to_core::<S>(right, interner, vctors)?,
            ],
        ),
        Expr::Not { operand } => {
            S::inductive("CNot", vec![expr_to_core::<S>(operand, interner, vctors)?])
        }
        Expr::Call { function, args } => {
            let fn_name = interner.resolve(*function);
            if let Some(field_names) = vctors.get(fn_name).map(|ctor| &ctor.fields) {
                // Variant constructor call → CNewVariant (field names from the type registry).
                let mut fnames: Vec<S::Val> = Vec::with_capacity(args.len());
                let mut fvals: Vec<S::Val> = Vec::with_capacity(args.len());
                for (i, a) in args.iter().enumerate() {
                    fnames.push(S::text(
                        field_names.get(i).map(|s| s.as_str()).unwrap_or("value"),
                    ));
                    fvals.push(expr_to_core::<S>(a, interner, vctors)?);
                }
                S::inductive(
                    "CNewVariant",
                    vec![S::text(fn_name), S::list(fnames), S::list(fvals)],
                )
            } else {
                let avals = exprs_to_core::<S>(args, interner, vctors)?;
                S::inductive("CCall", vec![S::text(fn_name), S::list(avals)])
            }
        }
        Expr::Index { collection, index } => S::inductive(
            "CIndex",
            vec![
                expr_to_core::<S>(collection, interner, vctors)?,
                expr_to_core::<S>(index, interner, vctors)?,
            ],
        ),
        Expr::Length { collection } => S::inductive(
            "CLen",
            vec![expr_to_core::<S>(collection, interner, vctors)?],
        ),
        Expr::FieldAccess { object, field } => S::inductive(
            "CMapGet",
            vec![
                expr_to_core::<S>(object, interner, vctors)?,
                S::inductive("CText", vec![S::text(interner.resolve(*field))]),
            ],
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
        Expr::Range { start, end } => S::inductive(
            "CRange",
            vec![
                expr_to_core::<S>(start, interner, vctors)?,
                expr_to_core::<S>(end, interner, vctors)?,
            ],
        ),
        Expr::Slice {
            collection,
            start,
            end,
        } => S::inductive(
            "CSlice",
            vec![
                expr_to_core::<S>(collection, interner, vctors)?,
                expr_to_core::<S>(start, interner, vctors)?,
                expr_to_core::<S>(end, interner, vctors)?,
            ],
        ),
        Expr::Copy { expr } => {
            S::inductive("CCopy", vec![expr_to_core::<S>(expr, interner, vctors)?])
        }
        Expr::Contains { collection, value } => S::inductive(
            "CContains",
            vec![
                expr_to_core::<S>(collection, interner, vctors)?,
                expr_to_core::<S>(value, interner, vctors)?,
            ],
        ),
        Expr::Union { left, right } => S::inductive(
            "CUnion",
            vec![
                expr_to_core::<S>(left, interner, vctors)?,
                expr_to_core::<S>(right, interner, vctors)?,
            ],
        ),
        Expr::Intersection { left, right } => S::inductive(
            "CIntersection",
            vec![
                expr_to_core::<S>(left, interner, vctors)?,
                expr_to_core::<S>(right, interner, vctors)?,
            ],
        ),
        Expr::OptionSome { value } => S::inductive(
            "COptionSome",
            vec![expr_to_core::<S>(value, interner, vctors)?],
        ),
        Expr::OptionNone => S::inductive("COptionNone", vec![]),
        Expr::InterpolatedString(parts) => {
            if parts.is_empty() {
                S::inductive("CText", vec![S::text("")])
            } else {
                let mut pvals: Vec<S::Val> = Vec::with_capacity(parts.len());
                for part in parts {
                    pvals.push(match part {
                        StringPart::Literal(sym) => {
                            S::inductive("CLiteralPart", vec![S::text(interner.resolve(*sym))])
                        }
                        StringPart::Expr { value, format_spec: None, .. } => S::inductive(
                            "CExprPart",
                            vec![expr_to_core::<S>(value, interner, vctors)?],
                        ),
                        StringPart::Expr { value, format_spec: Some(spec), .. } => {
                            let spec = interner.resolve(*spec);
                            let (form, size) = spec_form(spec);
                            S::inductive(
                                "CFormattedPart",
                                vec![expr_to_core::<S>(value, interner, vctors)?, S::text(spec), S::text(form), S::int(size)],
                            )
                        }
                    });
                }
                S::inductive("CInterpolatedString", vec![S::list(pvals)])
            }
        }
        Expr::NewVariant {
            variant, fields, ..
        } => {
            let mut fnames: Vec<S::Val> = Vec::with_capacity(fields.len());
            let mut fvals: Vec<S::Val> = Vec::with_capacity(fields.len());
            for (fname, fexpr) in fields {
                fnames.push(S::text(interner.resolve(*fname)));
                fvals.push(expr_to_core::<S>(fexpr, interner, vctors)?);
            }
            S::inductive(
                "CNewVariant",
                vec![
                    S::text(interner.resolve(*variant)),
                    S::list(fnames),
                    S::list(fvals),
                ],
            )
        }
        Expr::New {
            type_name,
            type_args,
            init_fields,
        } => {
            let tn = interner.resolve(*type_name);
            let typed = collection_type_text(tn, type_args, interner);
            if tn == "Seq" || tn == "List" {
                S::inductive("CNewSeq", vec![S::text(&typed)])
            } else if tn == "Set" {
                S::inductive("CNewSet", vec![S::text(&typed)])
            } else if tn == "Map" || tn.starts_with("Map ") {
                S::inductive(
                    "CNew",
                    vec![S::text(&typed), S::list(vec![]), S::list(vec![])],
                )
            } else if init_fields.is_empty() && vctors.struct_fields(tn).is_none() {
                S::inductive(
                    "CNewVariant",
                    vec![S::text(tn), S::list(vec![]), S::list(vec![])],
                )
            } else {
                let written: Vec<String> =
                    init_fields.iter().map(|(f, _)| interner.resolve(*f).to_string()).collect();
                let mut fnames: Vec<S::Val> = Vec::with_capacity(init_fields.len());
                let mut fvals: Vec<S::Val> = Vec::with_capacity(init_fields.len());
                for field in held_fields(tn, &written, vctors) {
                    match field {
                        HeldField::Written(i) => {
                            fnames.push(S::text(&written[i]));
                            fvals.push(expr_to_core::<S>(init_fields[i].1, interner, vctors)?);
                        }
                        HeldField::Slot((fname, ftype)) => {
                            fnames.push(S::text(fname));
                            fvals.push(declared_default_core::<S>(ftype));
                        }
                    }
                }
                S::inductive("CNew", vec![S::text(tn), S::list(fnames), S::list(fvals)])
            }
        }
        // Closure: params (Seq Text), body (Seq CStmt), captured names ([`closure_captures`], the
        // reference encoder's set). An expression body wraps as `CReturn e`.
        Expr::Closure { params, body, .. } => {
            let mut param_vals: Vec<S::Val> = Vec::with_capacity(params.len());
            for (sym, _) in params {
                param_vals.push(S::text(interner.resolve(*sym)));
            }
            let body_vals: Vec<S::Val> = match body {
                ClosureBody::Expression(e) => {
                    vec![S::inductive(
                        "CReturn",
                        vec![expr_to_core::<S>(e, interner, vctors)?],
                    )]
                }
                ClosureBody::Block(stmts) => stmt_list_to_vec::<S>(&stmts.iter().collect::<Vec<_>>(), interner, vctors)?,
            };
            let free = closure_captures(expr, interner);
            let cap_vals: Vec<S::Val> = free.iter().map(|fv| S::text(fv)).collect();
            S::inductive(
                "CClosure",
                vec![S::list(param_vals), S::list(body_vals), S::list(cap_vals)],
            )
        }
        // A callee-expression call (`(f)(x)`) → `CCallExpr{target, args}`.
        Expr::CallExpr { callee, args } => {
            let mut avals: Vec<S::Val> = Vec::with_capacity(args.len());
            for a in args.iter() {
                avals.push(expr_to_core::<S>(a, interner, vctors)?);
            }
            S::inductive(
                "CCallExpr",
                vec![expr_to_core::<S>(callee, interner, vctors)?, S::list(avals)],
            )
        }
        Expr::ChunkAt { index, zone } => S::inductive(
            "CChunkAt",
            vec![
                expr_to_core::<S>(index, interner, vctors)?,
                expr_to_core::<S>(zone, interner, vctors)?,
            ],
        ),
        Expr::ManifestOf { zone } => S::inductive(
            "CManifestOf",
            vec![expr_to_core::<S>(zone, interner, vctors)?],
        ),
        Expr::Escape { code, .. } => {
            S::inductive("CEscExpr", vec![S::text(interner.resolve(*code))])
        }
        // `Give`/`WithCapacity` are transparent: the reference encodes the inner value with no node
        // (an ownership move / an allocation hint the interpreter erases).
        Expr::Give { value } => expr_to_core::<S>(value, interner, vctors)?,
        Expr::WithCapacity { value, .. } => expr_to_core::<S>(value, interner, vctors)?,
        Expr::View(_) | Expr::Dom(_) => {
            let (code, awaits) = spelled_in_core_ir(expr, interner)?;
            S::inductive("CSpelled", vec![S::text(&code), S::text(awaits)])
        }
    })
}

fn stmt_to_core<S: CoreSink>(stmt: &Stmt, interner: &Interner, vctors: &VCtors) -> Option<S::Val> {
    Some(match stmt {
        Stmt::Let { var, value, .. } => S::inductive(
            "CLet",
            vec![
                S::text(interner.resolve(*var)),
                expr_to_core::<S>(value, interner, vctors)?,
            ],
        ),
        Stmt::Set { target, value } => S::inductive(
            "CSet",
            vec![
                S::text(interner.resolve(*target)),
                expr_to_core::<S>(value, interner, vctors)?,
            ],
        ),
        Stmt::Show { object, .. } => {
            S::inductive("CShow", vec![expr_to_core::<S>(object, interner, vctors)?])
        }
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => S::inductive(
            "CSetIdx",
            vec![
                S::text(&extract_ident_name(collection, interner)),
                expr_to_core::<S>(index, interner, vctors)?,
                expr_to_core::<S>(value, interner, vctors)?,
            ],
        ),
        Stmt::SetField {
            object,
            field,
            value,
        } => S::inductive(
            "CSetField",
            vec![
                S::text(&extract_ident_name(object, interner)),
                S::text(interner.resolve(*field)),
                expr_to_core::<S>(value, interner, vctors)?,
            ],
        ),
        Stmt::Pop { collection, into: None } => S::inductive(
            "CPop",
            vec![S::text(&extract_ident_name(collection, interner))],
        ),
        Stmt::Pop { collection, into: Some(into) } => S::inductive(
            "CPopInto",
            vec![S::text(&extract_ident_name(collection, interner)), S::text(interner.resolve(*into))],
        ),
        Stmt::Give { object, recipient } => S::inductive(
            "CGive",
            vec![
                expr_to_core::<S>(object, interner, vctors)?,
                S::text(&extract_ident_name(recipient, interner)),
            ],
        ),
        Stmt::Sleep { milliseconds } => S::inductive(
            "CSleep",
            vec![expr_to_core::<S>(milliseconds, interner, vctors)?],
        ),
        Stmt::RuntimeAssert {
            condition,
            hard,
            dim: _,
            reason,
        } => S::inductive(
            if *hard {
                "CHardAssert"
            } else {
                "CRuntimeAssert"
            },
            vec![
                expr_to_core::<S>(condition, interner, vctors)?,
                match reason {
                    Some(r) => expr_to_core::<S>(r, interner, vctors)?,
                    None => S::inductive("CText", vec![S::text("Assertion failed")]),
                },
            ],
        ),
        Stmt::Fail { reason } => S::inductive("CFail", vec![expr_to_core::<S>(reason, interner, vctors)?]),
        Stmt::Return { value } => {
            let e = match value {
                Some(v) => expr_to_core::<S>(v, interner, vctors)?,
                None => S::inductive("CNothing", vec![]),
            };
            S::inductive("CReturn", vec![e])
        }
        Stmt::Break => S::inductive("CBreak", vec![]),
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
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
            S::inductive(
                "CWhile",
                vec![cond_v, stmt_list_to_core::<S>(&body_s, interner, vctors)?],
            )
        }
        Stmt::Call { function, args } => {
            let mut avals: Vec<S::Val> = Vec::with_capacity(args.len());
            for a in args {
                avals.push(expr_to_core::<S>(a, interner, vctors)?);
            }
            S::inductive(
                "CCallS",
                vec![S::text(interner.resolve(*function)), S::list(avals)],
            )
        }
        Stmt::Push { value, collection } => S::inductive(
            "CPush",
            vec![
                expr_to_core::<S>(value, interner, vctors)?,
                S::text(&extract_ident_name(collection, interner)),
            ],
        ),
        Stmt::Repeat {
            pattern,
            iterable,
            body,
            ..
        } => {
            let (loop_var, components) = repeat_binders(pattern, interner);
            let body_s: Vec<&Stmt> = body.iter().collect();
            let mut body_vals: Vec<S::Val> = components
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    S::inductive(
                        "CLet",
                        vec![
                            S::text(name),
                            S::inductive(
                                "CIndex",
                                vec![
                                    S::inductive("CVar", vec![S::text(&loop_var)]),
                                    S::inductive("CInt", vec![S::int((i + 1) as i64)]),
                                ],
                            ),
                        ],
                    )
                })
                .collect();
            body_vals.extend(stmt_list_to_vec::<S>(&body_s, interner, vctors)?);
            let body_v = S::list(body_vals);
            if let Expr::Range { start, end } = iterable {
                S::inductive(
                    "CRepeatRange",
                    vec![
                        S::text(&loop_var),
                        expr_to_core::<S>(start, interner, vctors)?,
                        expr_to_core::<S>(end, interner, vctors)?,
                        body_v,
                    ],
                )
            } else {
                S::inductive(
                    "CRepeat",
                    vec![
                        S::text(&loop_var),
                        expr_to_core::<S>(iterable, interner, vctors)?,
                        body_v,
                    ],
                )
            }
        }
        // CRDT ops (mirror `encode_stmt_src`; declaration order from CORE_TYPES_FOR_PE).
        Stmt::Add { value, collection } => S::inductive(
            "CAdd",
            vec![
                expr_to_core::<S>(value, interner, vctors)?,
                S::text(&extract_ident_name(collection, interner)),
            ],
        ),
        Stmt::Remove { value, collection } => S::inductive(
            "CRemove",
            vec![
                expr_to_core::<S>(value, interner, vctors)?,
                S::text(&extract_ident_name(collection, interner)),
            ],
        ),
        Stmt::MergeCrdt { source, target } => S::inductive(
            "CMerge",
            vec![
                S::text(&extract_ident_name(target, interner)),
                expr_to_core::<S>(source, interner, vctors)?,
            ],
        ),
        // The IDEMPOTENT sibling — a distinct `CJoin` core node so the self-hosted PE preserves the join.
        Stmt::JoinCrdt { source, target } => S::inductive(
            "CJoin",
            vec![
                S::text(&extract_ident_name(target, interner)),
                expr_to_core::<S>(source, interner, vctors)?,
            ],
        ),
        Stmt::IncreaseCrdt {
            object,
            field,
            amount,
        } => S::inductive(
            "CIncrease",
            vec![
                S::text(&format!(
                    "{}'s {}",
                    extract_ident_name(object, interner),
                    interner.resolve(*field)
                )),
                expr_to_core::<S>(amount, interner, vctors)?,
            ],
        ),
        Stmt::DecreaseCrdt {
            object,
            field,
            amount,
        } => S::inductive(
            "CDecrease",
            vec![
                S::text(&format!(
                    "{}'s {}",
                    extract_ident_name(object, interner),
                    interner.resolve(*field)
                )),
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
        // Concurrency: one branch (Seq of CStmt) per task, the task's statements as the list walk encodes them.
        Stmt::Concurrent { tasks } => {
            let mut branches: Vec<S::Val> = Vec::with_capacity(tasks.len());
            for task in tasks.iter() {
                branches.push(S::list(stmt_list_to_vec::<S>(&[task], interner, vctors)?));
            }
            S::inductive("CConcurrent", vec![S::list(branches)])
        }
        Stmt::Parallel { tasks } => {
            let mut branches: Vec<S::Val> = Vec::with_capacity(tasks.len());
            for task in tasks.iter() {
                branches.push(S::list(stmt_list_to_vec::<S>(&[task], interner, vctors)?));
            }
            S::inductive("CParallel", vec![S::list(branches)])
        }
        // Sequence append (`Append x to d's lines`) — the base struct is forced dynamic at the list
        // level (see `stmt_list_to_vec`); here just the op. `target` round-trips as surface syntax.
        Stmt::AppendToSequence { sequence, value } => S::inductive(
            "CAppendToSeq",
            vec![
                S::text(&extract_ident_name(sequence, interner)),
                expr_to_core::<S>(value, interner, vctors)?,
            ],
        ),
        // Pipes / channels. Capacity is a `CInt` expression node (default 32, matching the reference).
        Stmt::CreatePipe {
            var: pipe_var,
            capacity,
            ..
        } => {
            let cap = capacity.unwrap_or(32);
            S::inductive(
                "CCreatePipe",
                vec![
                    S::text(interner.resolve(*pipe_var)),
                    S::inductive("CInt", vec![S::int(cap as i64)]),
                ],
            )
        }
        Stmt::SendPipe { value, pipe } => S::inductive(
            "CSendPipe",
            vec![
                S::text(&pipe_ident_name(pipe, interner)),
                expr_to_core::<S>(value, interner, vctors)?,
            ],
        ),
        Stmt::ReceivePipe {
            var: recv_var,
            pipe,
        } => S::inductive(
            "CReceivePipe",
            vec![
                S::text(&pipe_ident_name(pipe, interner)),
                S::text(interner.resolve(*recv_var)),
            ],
        ),
        Stmt::TrySendPipe { value, pipe, .. } => S::inductive(
            "CTrySendPipe",
            vec![
                S::text(&pipe_ident_name(pipe, interner)),
                expr_to_core::<S>(value, interner, vctors)?,
            ],
        ),
        Stmt::TryReceivePipe {
            var: recv_var,
            pipe,
        } => S::inductive(
            "CTryReceivePipe",
            vec![
                S::text(&pipe_ident_name(pipe, interner)),
                S::text(interner.resolve(*recv_var)),
            ],
        ),
        // Select over channels: one `CSelectRecv{chan,var,body}` or `CSelectTimeout{duration,body}`
        // per branch. Branch bodies use the single-stmt encoder (fall back on Inspect/Repeat).
        Stmt::Select { branches } => {
            let mut bvals: Vec<S::Val> = Vec::with_capacity(branches.len());
            for branch in branches.iter() {
                match branch {
                    SelectBranch::Receive {
                        var: recv_var,
                        pipe,
                        body,
                    } => bvals.push(S::inductive(
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
        Stmt::Assert { .. } => S::inductive(
            "CAssert",
            vec![S::inductive("CBool", vec![S::boolean(true)])],
        ),
        Stmt::Check { .. } => {
            let (subject, object, capability, rule, source) = check_parts(stmt, interner)?;
            let alternatives = vctors
                .policies
                .get(&(rule.clone(), capability))
                .into_iter()
                .flatten()
                .map(|(type_name, node)| S::inductive("CPolicyCase", vec![S::text(type_name), policy_core::<S>(node)]))
                .collect();
            S::inductive(
                "CCheckPolicy",
                vec![
                    S::text(&subject),
                    S::text(&object),
                    S::boolean(capability),
                    S::text(&rule),
                    S::list(alternatives),
                    S::text(&source),
                ],
            )
        }
        Stmt::Trust { justification, .. } => S::inductive(
            "CTrust",
            vec![
                S::inductive("CBool", vec![S::boolean(true)]),
                S::text(interner.resolve(*justification)),
            ],
        ),
        Stmt::Require { crate_name, .. } => {
            S::inductive("CRequire", vec![S::text(interner.resolve(*crate_name))])
        }
        // Inline-native escape hatch.
        Stmt::Escape { code, .. } => {
            S::inductive("CEscStmt", vec![S::text(interner.resolve(*code))])
        }
        // Task / actor control.
        Stmt::StopTask { handle } => S::inductive(
            "CStopTask",
            vec![expr_to_core::<S>(handle, interner, vctors)?],
        ),
        Stmt::Spawn { agent_type, name } => S::inductive(
            "CSpawn",
            vec![
                S::text(interner.resolve(*agent_type)),
                S::text(interner.resolve(*name)),
            ],
        ),
        // Structured-concurrency zone (kind fixed to "heap"); body via the single-stmt encoder.
        Stmt::Zone { name, body, .. } => S::inductive(
            "CZone",
            vec![
                S::text(interner.resolve(*name)),
                S::text("heap"),
                select_body_to_core::<S>(body, interner, vctors)?,
            ],
        ),
        // CRDT conflict resolution (base struct forced dynamic at the list level, like Increase/Merge).
        Stmt::ResolveConflict { object, field, value } => S::inductive(
            "CResolveTo",
            vec![
                S::text(&format!("{}'s {}", extract_ident_name(object, interner), interner.resolve(*field))),
                expr_to_core::<S>(value, interner, vctors)?,
            ],
        ),
        // Sync / mount.
        Stmt::Sync {
            var: sync_var,
            topic,
        } => S::inductive(
            "CSync",
            vec![
                S::text(interner.resolve(*sync_var)),
                expr_to_core::<S>(topic, interner, vctors)?,
            ],
        ),
        Stmt::Mount {
            var: mount_var,
            path,
        } => S::inductive(
            "CMount",
            vec![
                S::text(interner.resolve(*mount_var)),
                expr_to_core::<S>(path, interner, vctors)?,
            ],
        ),
        // IO.
        Stmt::ReadFrom {
            var: read_var,
            source,
        } => {
            let var_name = interner.resolve(*read_var);
            match source {
                ReadSource::Console => S::inductive("CReadConsole", vec![S::text(var_name)]),
                ReadSource::File(path_expr) => S::inductive(
                    "CReadFile",
                    vec![
                        expr_to_core::<S>(path_expr, interner, vctors)?,
                        S::text(var_name),
                    ],
                ),
            }
        }
        Stmt::WriteFile { content, path } => S::inductive(
            "CWriteFile",
            vec![
                expr_to_core::<S>(path, interner, vctors)?,
                expr_to_core::<S>(content, interner, vctors)?,
            ],
        ),
        // Networking / messaging. (`LetPeerAgent` encodes as `CConnectTo` with the agent as target.)
        Stmt::ConnectTo { address, .. } => S::inductive(
            "CConnectTo",
            vec![
                expr_to_core::<S>(address, interner, vctors)?,
                S::text("conn"),
            ],
        ),
        Stmt::Listen { address, .. } => S::inductive(
            "CListen",
            vec![
                expr_to_core::<S>(address, interner, vctors)?,
                S::text("default"),
            ],
        ),
        Stmt::SendMessage {
            message,
            destination,
            ..
        } => S::inductive(
            "CSendMessage",
            vec![
                expr_to_core::<S>(destination, interner, vctors)?,
                expr_to_core::<S>(message, interner, vctors)?,
            ],
        ),
        Stmt::AwaitMessage { into, .. } => {
            S::inductive("CAwaitMessage", vec![S::text(interner.resolve(*into))])
        }
        Stmt::StreamMessage {
            values,
            destination,
        } => S::inductive(
            "CStreamMessage",
            vec![
                expr_to_core::<S>(destination, interner, vctors)?,
                expr_to_core::<S>(values, interner, vctors)?,
            ],
        ),
        Stmt::LetPeerAgent {
            var: pa_var,
            address,
        } => S::inductive(
            "CConnectTo",
            vec![
                expr_to_core::<S>(address, interner, vctors)?,
                S::text(interner.resolve(*pa_var)),
            ],
        ),
        Stmt::AwaitTask { handle, result } => S::inductive(
            "CReceivePipe",
            vec![
                S::text(&match handle {
                    Expr::Identifier(sym) => interner.resolve(*sym).to_string(),
                    _ => "task".to_string(),
                }),
                S::text(&result.map(|r| interner.resolve(r).to_string()).unwrap_or_else(|| "_await".to_string())),
            ],
        ),
        Stmt::CloseChannel { channel } => {
            S::inductive("CStopTask", vec![expr_to_core::<S>(channel, interner, vctors)?])
        }
        Stmt::Serve { port, handler: _ } => S::inductive(
            "CListen",
            vec![expr_to_core::<S>(port, interner, vctors)?, S::text("default")],
        ),
        Stmt::Swap { a, b } => {
            let (an, bn) = (interner.resolve(*a), interner.resolve(*b));
            let tmp = swap_temp(an, bn);
            let var = |name: &str| S::inductive("CVar", vec![S::text(name)]);
            let exchange = vec![
                S::inductive("CLet", vec![S::text(&tmp), var(an)]),
                S::inductive("CSet", vec![S::text(an), var(bn)]),
                S::inductive("CSet", vec![S::text(bn), var(&tmp)]),
            ];
            S::inductive(
                "CIf",
                vec![S::inductive("CBool", vec![S::boolean(true)]), S::list(exchange), S::list(Vec::new())],
            )
        }
        // A list walk encodes these as nothing before it asks (`encodes_to_nothing`), splices a `During` and wraps an
        // `Inspect` itself; a caller holding one where no list walk does (a closure body, a task) has no encoding.
        Stmt::FunctionDef { .. }
        | Stmt::StructDef { .. }
        | Stmt::Ui(_)
        | Stmt::Fetch { .. }
        | Stmt::GivenFetch { .. }
        | Stmt::ServeFetches
        | Stmt::Replay { .. }
        | Stmt::ComponentDef(_)
        | Stmt::DirectiveDef { .. }
        | Stmt::RouteDef { .. }
        | Stmt::MiddlewareDef { .. }
        | Stmt::RedirectDef { .. }
        | Stmt::Theorem(_)
        | Stmt::Definition(_)
        | Stmt::Axiom(_)
        | Stmt::Theory(_)
        | Stmt::Site(_)
        | Stmt::TestDef { .. }
        | Stmt::Expect { .. }
        | Stmt::ExpectFail { .. }
        | Stmt::ExpectOutput { .. }
        | Stmt::AdvanceTime { .. }
        | Stmt::GivenFile { .. }
        | Stmt::Observe { .. }
        | Stmt::ForAll { .. }
        | Stmt::During { .. }
        | Stmt::Splice { .. }
        | Stmt::Reversibly { .. }
        | Stmt::Inspect { .. } => return None,
    })
}

/// The statements a Core IR body holds nothing for, as `encode_stmt_src` drops them: declarations (functions and
/// types are encoded at the program level; theorems, routes, components and the like have no run-time effect to
/// specialize), `## Test` blocks and the test-only statements, observation, and property tests.
fn encodes_to_nothing(stmt: &Stmt) -> bool {
    matches!(
        stmt,
        Stmt::FunctionDef { .. }
            | Stmt::StructDef { .. }
            | Stmt::Ui(_)
            | Stmt::Fetch { .. }
            | Stmt::GivenFetch { .. }
            | Stmt::ServeFetches
            | Stmt::Replay { .. }
            | Stmt::ComponentDef(_)
            | Stmt::DirectiveDef { .. }
            | Stmt::RouteDef { .. }
            | Stmt::MiddlewareDef { .. }
            | Stmt::RedirectDef { .. }
            | Stmt::Theorem(_)
            | Stmt::Definition(_)
            | Stmt::Axiom(_)
            | Stmt::Theory(_)
            | Stmt::Site(_)
            | Stmt::TestDef { .. }
            | Stmt::Expect { .. }
            | Stmt::ExpectFail { .. }
            | Stmt::ExpectOutput { .. }
            | Stmt::AdvanceTime { .. }
            | Stmt::GivenFile { .. }
            | Stmt::Observe { .. }
            | Stmt::ForAll { .. }
    )
}

/// The name a `Swap a and b` holds `a`'s value under while it exchanges the two, the same in every encoder: there
/// is no `CSwap`, so the swap is `tmp := a; a := b; b := tmp` in an always-taken `CIf`.
fn swap_temp(a: &str, b: &str) -> String {
    format!("swaptmp_{a}_{b}")
}

/// A channel name: a plain identifier, else the literal `"pipe"` (mirrors `encode_stmt_src`).
fn pipe_ident_name(pipe: &Expr, interner: &Interner) -> String {
    match pipe {
        Expr::Identifier(sym) => interner.resolve(*sym).to_string(),
        _ => "pipe".to_string(),
    }
}

/// A `Select` branch's or a zone's body, by the list walk.
fn select_body_to_core<S: CoreSink>(
    body: &[Stmt],
    interner: &Interner,
    vctors: &VCtors,
) -> Option<S::Val> {
    stmt_list_to_core::<S>(&body.iter().collect::<Vec<_>>(), interner, vctors)
}

fn stmt_list_to_core<S: CoreSink>(
    stmts: &[&Stmt],
    interner: &Interner,
    vctors: &VCtors,
) -> Option<S::Val> {
    Some(S::list(stmt_list_to_vec::<S>(stmts, interner, vctors)?))
}

/// Flatten a statement list into Core-IR nodes. Most statements map 1:1 via `stmt_to_core`; a write through a field
/// path is wrapped in the place it writes (`CAtPath`), and a merge into a whole struct is preceded by the
/// `CForceDynamic` of its root, as `encode_stmts_src` emits them.
fn stmt_list_to_vec<S: CoreSink>(
    stmts: &[&Stmt],
    interner: &Interner,
    vctors: &VCtors,
) -> Option<Vec<S::Val>> {
    let mut out: Vec<S::Val> = Vec::with_capacity(stmts.len());
    for s in stmts {
        match s {
            Stmt::Inspect { .. } => out.push(inspect_to_core::<S>(s, interner, vctors)?),
            Stmt::During { body, .. } | Stmt::Splice { body } | Stmt::Reversibly { body } => {
                let inner: Vec<&Stmt> = body.iter().collect();
                out.extend(stmt_list_to_vec::<S>(&inner, interner, vctors)?);
            }
            _ if encodes_to_nothing(s) => {}
            _ if write_place(s, interner).is_some() => {
                let (root, path) = write_place(s, interner).expect("a field path");
                let inner = stmt_to_core::<S>(s, interner, vctors)?;
                out.push(S::inductive(
                    "CAtPath",
                    vec![S::text(&root), S::list(path.iter().map(|f| S::text(f)).collect()), S::list(vec![inner])],
                ));
            }
            Stmt::MergeCrdt { target, .. } | Stmt::JoinCrdt { target, .. } => {
                if let Expr::Identifier(root) = target {
                    out.push(S::inductive("CForceDynamic", vec![S::text(interner.resolve(*root))]));
                }
                out.push(stmt_to_core::<S>(s, interner, vctors)?);
            }
            _ => out.push(stmt_to_core::<S>(s, interner, vctors)?),
        }
    }
    Some(out)
}

/// `Inspect target: When Variant (bindings): body … Otherwise: rest` as one `CInspect`, its arms in source order and
/// each arm's bindings as the pattern wrote them — the value `encode_stmts_src` builds.
fn inspect_to_core<S: CoreSink>(stmt: &Stmt, interner: &Interner, vctors: &VCtors) -> Option<S::Val> {
    let Stmt::Inspect { target, arms, .. } = stmt else {
        return None;
    };
    let mut core_arms: Vec<S::Val> = Vec::with_capacity(arms.len());
    for arm in arms {
        let body_refs: Vec<&Stmt> = arm.body.iter().collect();
        let body = stmt_list_to_core::<S>(&body_refs, interner, vctors)?;
        core_arms.push(match arm.variant {
            Some(variant) => S::inductive(
                "CWhen",
                vec![
                    S::text(interner.resolve(variant)),
                    S::list(
                        arm.bindings
                            .iter()
                            .map(|(_, binding)| S::text(interner.resolve(*binding)))
                            .collect(),
                    ),
                    body,
                ],
            ),
            None => S::inductive("COtherwise", vec![body]),
        });
    }
    for (variant, report) in uncovered_variants(arms, interner, vctors) {
        let fail = S::inductive("CFail", vec![S::inductive("CText", vec![S::text(&report)])]);
        core_arms.push(S::inductive("CWhen", vec![S::text(&variant), S::list(Vec::new()), S::list(vec![fail])]));
    }
    Some(S::inductive(
        "CInspect",
        vec![expr_to_core::<S>(target, interner, vctors)?, S::list(core_arms)],
    ))
}

/// Whether the native Core-IR value builder covers `program` (the fast marshal path is taken, not
/// the interpreter fallback). For test coverage assertions.
pub fn program_covered_by_native_builder(program: &str) -> bool {
    program_to_core_value(&user_program_source(program)).is_some()
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
    let full_source = if program.contains("## Main") || program.contains("## To ") {
        program.to_string()
    } else {
        format!("## Main\n{}", program)
    };
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(&full_source, &mut interner);
    let tokens = lexer.tokenize();
    let (type_registry, policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };
    let vctors = variant_ctors(&type_registry, &policy_registry, &interner);
    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let _compile_facts = crate::optimize::facts::scope();
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
    let type_shapes = crate::core_ir_scope::type_shapes(&type_registry);
    let mut parser = crate::parser::Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
    );
    let stmts = parser.parse_program().ok()?;
    let opt_flags = parser.program_opt_flags();
    drop(parser);
    let stmts = resolved_as_every_tier(
        stmts,
        &full_source,
        &opt_flags,
        &mut interner,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
    );
    let stmts = crate::core_ir_scope::for_core_ir(
        &stmts,
        &mut interner,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
        &type_shapes,
    );
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
        if let Stmt::FunctionDef {
            name,
            params,
            body,
            return_type,
            is_native,
            ..
        } = stmt
        {
            if *is_native {
                continue;
            }
            let fname = interner.resolve(*name).to_string();
            let pnames: Vec<S::Val> = params
                .iter()
                .map(|(n, _)| S::text(interner.resolve(*n)))
                .collect();
            let ptypes: Vec<S::Val> = params
                .iter()
                .map(|(_, ty)| S::text(&decompile_type_expr(ty, interner)))
                .collect();
            let ret = return_type
                .map(|rt| decompile_type_expr(rt, interner))
                .unwrap_or_else(|| "Nothing".to_string());
            let body_s: Vec<&Stmt> = body.iter().collect();
            let body_v = stmt_list_to_core::<S>(&body_s, interner, vctors)?;
            funcs.push(S::inductive(
                "CFuncDef",
                vec![
                    S::text(&fname),
                    S::list(pnames),
                    S::list(ptypes),
                    S::text(&ret),
                    body_v,
                ],
            ));
        } else if matches!(
            stmt,
            Stmt::StructDef { .. }
                | Stmt::Theorem(_)
                | Stmt::Definition(_)
                | Stmt::Axiom(_)
                | Stmt::Theory(_)
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
/// `program_to_core` on `WireSink` emits the wire bytes in ONE pass straight from the AST — no
/// intermediate `RuntimeValue` tree and no second `encode_value_raw` walk (byte-for-byte identical to
/// the old two-pass form; locked by `native_builder_is_byte_identical_to_the_interpreter`). FALLBACK:
/// for a construct the native builder does not cover, run the constructor source on the tree-walker
/// (`wireBytes`). Both are decoded identically by the native binary's generated `CProgram::wire_decode`.
pub fn program_to_core_wire_bytes(program: &str) -> Result<Vec<u8>, String> {
    if let Some(bytes) = program_to_core::<WireSink>(&user_program_source(program)) {
        return Ok(bytes);
    }
    program_to_core_wire_bytes_via_interpreter(program)
}

/// The PARSE-FREE marshal: emit a program's Core-IR wire bytes directly from an ALREADY-PARSED AST,
/// skipping the lex + discovery + `parse_program` that dominates [`program_to_core_wire_bytes`] (~90%
/// of a specialization). An integrated caller that parsed the program once for compilation reuses that
/// AST here to feed the native PE at a fraction of the cost. `registry` and `policies` are the program's type and
/// policy registries, whose declarations the encoding reads as `program_to_core` does. `None` if a construct is
/// uncovered. Byte-identical to [`program_to_core_wire_bytes`] on the same program — same `build_core`/`WireSink` spine.
pub fn stmts_to_core_wire_bytes(
    stmts: &[Stmt],
    interner: &mut Interner,
    registry: &crate::analysis::TypeRegistry,
    policies: &crate::analysis::PolicyRegistry,
) -> Option<Vec<u8>> {
    let vctors = variant_ctors(registry, policies, interner);
    let exprs: Arena<Expr> = Arena::new();
    let stmt_nodes: Arena<Stmt> = Arena::new();
    let _facts = crate::optimize::facts::scope();
    let types: Arena<TypeExpr> = Arena::new();
    let scoped = crate::core_ir_scope::for_core_ir(
        stmts,
        interner,
        crate::ast_map::Arenas {
            exprs: &exprs,
            stmts: &stmt_nodes,
            types: &types,
        },
        &crate::core_ir_scope::TypeShapes::default(),
    );
    build_core::<WireSink>(&scoped, interner, &vctors)
}

/// The TWO-PASS marshal that [`program_to_core_wire_bytes`] replaced: build the full `RuntimeValue`
/// Core-IR tree (`TreeSink`) then `encode_value_raw` it. Kept as the correctness ORACLE and the
/// speed BASELINE for the single-pass (`WireSink`) form — the two are byte-identical by construction
/// (`WireSink` emits exactly what `encode_value_raw` would), and the single pass avoids the tree
/// allocation + the second encode walk. `None` if a construct is uncovered.
pub fn program_to_core_wire_bytes_two_pass(program: &str) -> Option<Vec<u8>> {
    program_to_core::<TreeSink>(&user_program_source(program)).and_then(|v| encode_value_raw(&v).ok())
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
    let encoded = encode_program_source(&full_source)
        .map_err(|e| format!("Failed to encode program: {}", describe_parse_error(&e)))?;
    let driver = "    Let prog be a new CProg with funcs encodedFuncSeq and main encodedMain.\n    Let wb be wireBytes(prog).\n    Repeat for b in wb:\n        Show \"{b}\".\n";
    let combined = format!("{}\n## Main\n{}\n{}", CORE_TYPES_FOR_PE, encoded, driver);
    let out = interpret_bootstrap(&combined)
        .map_err(|e| format!("wire-encode run failed: {}", describe_parse_error(&e)))?;
    let mut bytes = Vec::new();
    for line in out.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let n: u16 = t
            .parse()
            .map_err(|_| format!("wireBytes emitted a non-byte line: {:?}", t))?;
        bytes.push(n as u8);
    }
    Ok(bytes)
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    /// The already-built native PE binary path for this process. Caching it avoids re-running
    /// `aot_cache_key` on every specialization — which spawns `rustc --version` (~15 ms) and
    /// re-hashes the 167 KB source. The path is stable for the process (the PE source doesn't
    /// change at runtime), so one lookup per process suffices.
    static NATIVE_PE_BIN: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// The semantics every bootstrap program — the partial evaluator, its dialects, the self-interpreter, the decompiler,
/// the wire encoder — compiles under ([`compile_bootstrap_engine`]): reference collections, so the state it threads
/// through `mutable` parameters is mutated in place ([`crate::semantics::collections::with_reference_semantics`]), and
/// blocking waits ([`crate::codegen::with_blocking_waits`]). Part of every cache key of a natively compiled bootstrap
/// program, since the Rust it compiles to depends on it.
#[cfg(not(target_arch = "wasm32"))]
const BOOTSTRAP_SEMANTICS: &str = "collections: reference; waits: block";

/// A bootstrap program's Rust, with the wire codec, under [`BOOTSTRAP_SEMANTICS`].
#[cfg(not(target_arch = "wasm32"))]
fn compile_bootstrap_engine(source: &str) -> Result<CompileOutput, ParseError> {
    with_reference_semantics(|| crate::codegen::with_blocking_waits(|| compile_program_full_with_wire(source)))
}

/// Build (once, content-addressed) the compile-once native PE binary and return its path. Keyed
/// on the generated Rust of [`pe_native_source`] + toolchain, so the one-time rustc compile of the
/// whole PE engine happens exactly once per PE version and is reused across every program and
/// session. This is the amortization that turns AOT from "recompile per program" into the 210x win.
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
    // Key on the (stable) PE SOURCE text + toolchain + the collection semantics it compiles under
    // — NOT the generated Rust: computing the Rust means recompiling the whole 167 KB engine,
    // which would defeat the cache on every warm call.
    let key = aot_cache_key(&format!("{src}\n{BOOTSTRAP_SEMANTICS}"));
    let base = std::env::temp_dir().join("logos_native_pe");
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let proj = base.join(format!("pe_{key}"));
    let bin = proj.join("target").join("release").join("logos_native_pe");
    if bin.exists() {
        NATIVE_PE_BIN.with(|c| *c.borrow_mut() = Some(bin.clone()));
        return Ok(bin);
    }
    let rust = compile_bootstrap_engine(&src)
        .map_err(|e| format!("native PE compile: {}", describe_parse_error(&e)))?
        .rust_code;
    std::fs::create_dir_all(proj.join("src")).map_err(|e| e.to_string())?;
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().unwrap().parent().unwrap();
    std::fs::write(
        proj.join("Cargo.toml"),
        format!(
            "[package]\nname=\"logos_native_pe\"\nversion=\"0.1.0\"\nedition=\"2021\"\n[dependencies]\nlogicaffeine-data={{path=\"{r}/crates/logicaffeine_data\"}}\nlogicaffeine-system={{path=\"{r}/crates/logicaffeine_system\",features=[\"full\"]}}\ntokio={{version=\"1\",features=[\"rt-multi-thread\",\"macros\"]}}\nserde={{version=\"1\",features=[\"derive\"]}}\nrayon=\"1\"\n{profile}",
            r = root.display(),
            profile = EngineBuild::Interpretive.profile()
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
                *slot = Some(PeServer {
                    child,
                    stdin,
                    stdout,
                });
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

/// The in-process FFI entry appended to the generated PE engine to form the cdylib's `lib.rs`. It lives in the SAME
/// crate-root module as the generated `pub enum CProgram` and the exported `pub fn peSpecializeOnce(CProgram) ->
/// String`, so it calls both directly.
#[cfg(not(target_arch = "wasm32"))]
fn pe_cdylib_shim() -> String {
    native_engine_shim("logos_pe_specialize", "CProgram", "peSpecializeOnce")
}

/// The PE cdylib's `lib.rs`: [`pe_cdylib_source`] through the Rust emitter, under the reference
/// collection semantics, followed by its FFI entry.
#[cfg(not(target_arch = "wasm32"))]
pub fn native_pe_cdylib_lib_rs() -> Result<String, String> {
    let generated = compile_bootstrap_engine(&pe_cdylib_source())
        .map_err(|e| format!("native PE cdylib compile: {}", describe_parse_error(&e)))?
        .rust_code;
    Ok(format!("{generated}\n{}", pe_cdylib_shim()))
}

/// Build (and cache) the PE compiled as an IN-PROCESS cdylib exporting `logos_pe_specialize`.
/// Mirrors [`build_native_pe_binary`] but emits a `cdylib` whose `lib.rs` is the generated PE engine
/// ([`pe_cdylib_source`]) plus its `extern "C"` entry ([`native_engine_shim`]) that decodes the
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
    // Key on the PE source, the FFI shim and the collection semantics the engine compiles under,
    // so changing any of them invalidates the on-disk cache.
    let key = aot_cache_key(&format!("{src}\n{}\n{BOOTSTRAP_SEMANTICS}", pe_cdylib_shim()));
    let so = build_logos_cdylib("logos_native_pe_cdylib", "logos_native_pe_lib", &format!("pe_{key}"), EngineBuild::Interpretive, native_pe_cdylib_lib_rs)?;
    NATIVE_PE_CDYLIB.with(|c| *c.borrow_mut() = Some(so.clone()));
    Ok(so)
}

/// What a generated engine is, which decides how it is built. An interpretive engine (the offline specializer, the online
/// one) runs its loops hot over every program it is handed, so it is optimized whole. A residual program the
/// projections emit (the second projection's compiler, the third's compiler generator) is large straight-line code
/// run once per program, so its build spreads over many codegen units at `opt-level=1` without link-time
/// optimization: on the 245k-line compiler a quarter of the build's CPU, the compiler within 7% of its speed.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum EngineBuild {
    Interpretive,
    Residual,
}

#[cfg(not(target_arch = "wasm32"))]
impl EngineBuild {
    /// The `[profile.release]` section of the engine's manifest.
    pub(crate) fn profile(self) -> &'static str {
        match self {
            EngineBuild::Interpretive => "[profile.release]\nopt-level=3\n",
            EngineBuild::Residual => "[profile.release]\nopt-level=1\ncodegen-units=256\nlto=\"off\"\n",
        }
    }
}

/// Build a generated LOGOS engine as a release cdylib named `crate_name`, in `<temp>/<dir>/<project>`, with the profile
/// `build` gives it: `lib_rs` writes its `lib.rs` (the generated engine and its FFI shim) the first time; a project
/// already built is reused.
#[cfg(not(target_arch = "wasm32"))]
fn build_logos_cdylib(
    dir: &str,
    crate_name: &str,
    project: &str,
    build: EngineBuild,
    lib_rs: impl FnOnce() -> Result<String, String>,
) -> Result<std::path::PathBuf, String> {
    let base = std::env::temp_dir().join(dir);
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let proj = base.join(project);
    let so = proj.join("target").join("release").join(format!(
        "{}{crate_name}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    if so.exists() {
        return Ok(so);
    }
    let lib_rs = lib_rs()?;
    std::fs::create_dir_all(proj.join("src")).map_err(|e| e.to_string())?;
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().unwrap().parent().unwrap();
    std::fs::write(
        proj.join("Cargo.toml"),
        format!(
            "[package]\nname=\"{crate_name}\"\nversion=\"0.1.0\"\nedition=\"2021\"\n[lib]\ncrate-type=[\"cdylib\"]\npath=\"src/lib.rs\"\n[dependencies]\nlogicaffeine-data={{path=\"{r}/crates/logicaffeine_data\"}}\nlogicaffeine-system={{path=\"{r}/crates/logicaffeine_system\",features=[\"full\"]}}\ntokio={{version=\"1\",features=[\"rt-multi-thread\",\"macros\"]}}\nserde={{version=\"1\",features=[\"derive\"]}}\nrayon=\"1\"\n{profile}",
            r = root.display(),
            profile = build.profile()
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
        return Err(format!("{crate_name} build failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    if so.exists() {
        Ok(so)
    } else {
        Err(format!("{crate_name} missing after a successful build"))
    }
}

/// What every in-process engine's cdylib carries beside its entry: the outcome a call hands the host — a status byte
/// (0 the text, 1 the failure), then the text — and the free that reclaims it. The engine runs on a thread of its own
/// with the stack its recursion needs; a failure reaches the host as its text, so the engine's panic hook stays
/// silent. `into_boxed_slice` gives an allocation of capacity exactly its length, so `logos_native_free` reconstructs
/// the identical `Box<[u8]>` (reconstructing a `Vec` with cap == len would be UB if the allocator over-allocated).
#[cfg(not(target_arch = "wasm32"))]
const NATIVE_ENGINE_SHIM: &str = r#"
fn logos_native_outcome(outcome: std::thread::Result<String>, out_len: *mut usize) -> *mut u8 {
    let mut bytes = match outcome {
        Ok(text) => {
            let mut b = vec![0u8];
            b.extend_from_slice(text.as_bytes());
            b
        }
        Err(payload) => {
            let msg = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "the native engine failed".to_string());
            let mut b = vec![1u8];
            b.extend_from_slice(msg.as_bytes());
            b
        }
    };
    bytes.shrink_to_fit();
    let boxed: Box<[u8]> = bytes.into_boxed_slice();
    let n = boxed.len();
    unsafe { *out_len = n; }
    Box::into_raw(boxed) as *mut u8
}

#[no_mangle]
pub extern "C" fn logos_native_free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        unsafe { drop(Box::from_raw(std::slice::from_raw_parts_mut(ptr, len) as *mut [u8])); }
    }
}
"#;

/// The FFI entry of an engine generated from LOGOS, appended to its generated Rust: `symbol` decodes its wire bytes
/// as one `input`, calls the engine's exported `function` on it and hands back the text it answers, with
/// [`NATIVE_ENGINE_SHIM`].
#[cfg(not(target_arch = "wasm32"))]
fn native_engine_shim(symbol: &str, input: &str, function: &str) -> String {
    format!(
        r#"
#[no_mangle]
pub extern "C" fn {symbol}(ptr: *const u8, len: usize, out_len: *mut usize) -> *mut u8 {{
    static QUIET: std::sync::Once = std::sync::Once::new();
    QUIET.call_once(|| std::panic::set_hook(Box::new(|_| {{}})));
    let bytes = unsafe {{ std::slice::from_raw_parts(ptr, len) }}.to_vec();
    let outcome = std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {{
            let input = <{input} as logicaffeine_data::wire::WireDecode>::wire_decode(&bytes, &mut 0usize)
                .expect("{symbol}: decode {input}");
            let out: String = {function}(input);
            out
        }})
        .map_err(|e| Box::new(e.to_string()) as Box<dyn std::any::Any + Send>)
        .and_then(|engine| engine.join());
    logos_native_outcome(outcome, out_len)
}}
{NATIVE_ENGINE_SHIM}"#
    )
}

/// A generated engine's cdylib loaded for this thread: the `Library` kept alive alongside the raw entry-point
/// pointers derived from it (valid for as long as the `Library` is not unloaded, which is exactly this struct's
/// lifetime). The entry takes wire bytes and returns a status byte (0 the text, 1 the failure it caught) and a text,
/// in a buffer it owns until `free`.
#[cfg(not(target_arch = "wasm32"))]
struct NativeLib {
    _lib: libloading::Library,
    entry: unsafe extern "C" fn(*const u8, usize, *mut usize) -> *mut u8,
    free: unsafe extern "C" fn(*mut u8, usize),
}

#[cfg(not(target_arch = "wasm32"))]
impl NativeLib {
    fn load(so: &std::path::Path, entry: &[u8]) -> Result<NativeLib, String> {
        let free: &[u8] = b"logos_native_free";
        let name = |sym: &[u8]| String::from_utf8_lossy(sym).into_owned();
        // SAFETY: the cdylib was built from our own generated source; its exported symbols have exactly the
        // signatures declared here, and the raw pointers are stored alongside the owning `Library` so they never
        // outlive it.
        unsafe {
            let lib = libloading::Library::new(so).map_err(|e| format!("cdylib load {}: {e}", so.display()))?;
            let entry_fn = *lib
                .get::<unsafe extern "C" fn(*const u8, usize, *mut usize) -> *mut u8>(entry)
                .map_err(|e| format!("dlsym {}: {e}", name(entry)))?;
            let free_fn = *lib
                .get::<unsafe extern "C" fn(*mut u8, usize)>(free)
                .map_err(|e| format!("dlsym {}: {e}", name(free)))?;
            Ok(NativeLib { _lib: lib, entry: entry_fn, free: free_fn })
        }
    }

    fn call(&self, bytes: &[u8]) -> Result<String, String> {
        let mut out_len: usize = 0;
        // SAFETY: `bytes` is a live slice for the duration of the call; the callee writes `out_len` and returns a
        // buffer of that length we own until `free`.
        let p = unsafe { (self.entry)(bytes.as_ptr(), bytes.len(), &mut out_len as *mut usize) };
        if p.is_null() {
            return Err("the native engine returned a null result".into());
        }
        let out = unsafe { std::slice::from_raw_parts(p, out_len).to_vec() };
        unsafe { (self.free)(p, out_len) };
        let (status, text) = out.split_first().ok_or("the native engine returned no status")?;
        let text = String::from_utf8(text.to_vec()).map_err(|e| format!("the native engine's result is not UTF-8: {e}"))?;
        match status {
            0 => Ok(text),
            _ => Err(text),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static PE_LIB: std::cell::RefCell<Option<NativeLib>> = const { std::cell::RefCell::new(None) };
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
            *slot = Some(NativeLib::load(&build_native_pe_cdylib()?, b"logos_pe_specialize")?);
        }
        slot.as_ref().expect("loaded").call(&bytes)
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

    let raw_residual = with_reference_semantics(|| vm_run_source(&combined))
        .map_err(|e| format!("PE VM execution failed: {e}"))?;
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
        .map_err(|e| format!("Failed to encode program: {}", describe_parse_error(&e)))?;

    let pe_source = pe_source_text();
    let decompile_source = decompile_source_text();

    let actual_core_types = if core_types.is_empty() {
        CORE_TYPES_FOR_PE
    } else {
        core_types
    };

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
        actual_core_types, pe_source, decompile_source, encoded, driver,
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

/// The self-interpreter's run of `program`: [`core_interp_source`] and one `coreRun` of the program's encoding on the
/// arguments the run is given. Run as it stands it is `[[sint]](p, args)`.
pub fn self_interpreter_run_source(program: &str) -> Result<String, String> {
    self_interpreter_run_on(program, "args()")
}

/// [`self_interpreter_run_source`] with the program's arguments `arguments`, a `Seq of Text` expression.
fn self_interpreter_run_on(program: &str, arguments: &str) -> Result<String, String> {
    let full_source = if program.contains("## Main") || program.contains("## To ") {
        program.to_string()
    } else {
        format!("## Main\n{}", program)
    };
    let target = encode_program_source(&full_source).map_err(|e| describe_parse_error(&e))?;
    Ok(format!(
        "{}\n{}\n## Main\n{}\ncoreRun(encodedMain, encodedFuncMap, {arguments}).\n",
        CORE_TYPES_FOR_PE,
        core_interp_source(),
        target
    ))
}

/// The self-interpreter's run of `program` with `program_args` (the full argument vector, as [`tw_lines_with_args`]
/// takes it): the encoded program executed by `core_interp` on the tree-walker in the bootstrap scope, every line it
/// showed and the report it ended with. A program the encoder cannot read ends with the encoder's words.
pub fn self_interpreter_lines(program: &str, program_args: &[String]) -> crate::interpreter::InterpreterResult {
    let subject = match self_interpreter_run_source(program) {
        Ok(subject) => subject,
        Err(why) => {
            return crate::interpreter::InterpreterResult {
                lines: Vec::new(),
                error: Some(why),
            }
        }
    };
    let program_args = program_args.to_vec();
    on_bootstrap_stack(move || {
        crate::concurrency::net_inbox::set_net_offline(true);
        crate::ui_bridge::with_parsed_program(&subject, |parsed, interner| match parsed {
            Ok((stmts, types, policies)) => {
                let force_async = crate::interpreter::needs_async(stmts, interner);
                with_reference_semantics(|| crate::ui_bridge::run_treewalker(stmts, types, policies, interner, force_async, &program_args))
            }
            Err(advice) => crate::interpreter::InterpreterResult {
                lines: Vec::new(),
                error: Some(advice),
            },
        })
    })
}

/// The program [`textbook_projection1`] runs: the partial evaluator and the decompiler, given the self-interpreter's run
/// of `program` on its run-time arguments (`args()`) as data, so the residual reads them when it runs. A program that
/// reads another input only when it runs — the clock, the console, a file, a builtin whose answer is not a function of
/// its arguments — is refused.
pub fn textbook_projection1_subject_source(program: &str) -> Result<String, String> {
    let tree = program_to_core_tree(&user_program_source(program))
        .ok_or_else(|| "the program is outside the native builder's coverage".to_string())?;
    refuse_run_time_input(&tree, true)?;
    let subject = self_interpreter_run_on(program, "args()")?;
    let encoded_subject = encode_program_source_compact(&subject).map_err(|e| {
        format!(
            "Failed to encode the interpreter's run of the program: {}",
            describe_parse_error(&e)
        )
    })?;
    Ok(format!(
        "{}\n{}\n{}\n## Main\n{}\n{}",
        CORE_TYPES_FOR_PE,
        pe_source_text(),
        decompile_source_text(),
        encoded_subject,
        pe_driver_body("Show output.")
    ))
}

/// The first Futamura projection as Futamura defined it: the partial evaluator specializes the self-interpreter
/// ([`core_interp_source`]) to `program`. The subject the specializer reads is the interpreter together with one call
/// of it on the program's encoding, a constant, so every dispatch the interpreter makes on the program is a static one.
/// The residual is the decompiled specialization: Jones-optimal when [`count_dispatch`] finds nothing of the
/// interpreter in it and it prints what the program prints.
pub fn textbook_projection1(program: &str) -> Result<String, String> {
    let combined = textbook_projection1_subject_source(program)?;
    let raw = interpret_bootstrap(&combined)
        .map_err(|e| format!("PE execution failed: {}", describe_parse_error(&e)))?;
    let residual = prepend_type_definitions(program, finish_projection1_residual(raw)?);
    Ok(if holds_catalog_values(&residual) {
        format!("{}\n{residual}", self_interpreter_catalog())
    } else {
        residual
    })
}

/// The types the self-interpreter's values are made of: the Core IR catalog, and the sections `core_interp.logos`
/// declares for itself (`CoreOut`).
fn self_interpreter_catalog() -> &'static str {
    static CATALOG: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut catalog = core_types_for_pe_source().trim().to_string();
        let mut keep = false;
        for line in core_interp_source().lines() {
            if line.starts_with("## ") {
                keep = line.starts_with("## A ");
                if keep {
                    catalog.push_str("\n\n");
                }
            }
            if keep && !line.trim().is_empty() {
                catalog.push_str(line);
                catalog.push('\n');
            }
        }
        catalog
    })
}

/// Every type and constructor the self-interpreter's catalog declares.
fn catalog_names() -> &'static std::collections::HashSet<&'static str> {
    static NAMES: std::sync::OnceLock<std::collections::HashSet<&'static str>> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        self_interpreter_catalog()
            .lines()
            .filter_map(|line| line.trim().strip_prefix("## A ").or_else(|| line.trim().strip_prefix("A ")))
            .filter_map(|rest| rest.split(|c: char| !(c.is_alphanumeric() || c == '_')).next())
            .filter(|name| !name.is_empty())
            .collect()
    })
}

/// Whether a residual still names a value of the self-interpreter's catalog (a `CVal`, a `CoreOut`): a specialization
/// that kept some of the interpreter around a value it knew only in part, and so needs the catalog's types to run.
fn holds_catalog_values(residual: &str) -> bool {
    let names = catalog_names();
    residual.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|word| names.contains(word))
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
pub fn run_genuine_p2_on_target(
    program: &str,
    core_types: &str,
    interpreter: &str,
) -> Result<String, String> {
    let combined = genuine_p2_on_target_source(program, core_types, interpreter)?;
    with_reference_semantics(|| run_logos_source(&combined))
}

/// The whole program [`run_genuine_p2_on_target`] runs, so a harness can run it on any backend.
pub fn genuine_p2_on_target_source(
    program: &str,
    core_types: &str,
    interpreter: &str,
) -> Result<String, String> {
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
        .map_err(|e| format!("Failed to encode target: {}", describe_parse_error(&e)))?;
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
    let encoded = encode_program_source_compact(&pe_mini_prog).map_err(|e| {
        format!(
            "Failed to encode pe_mini+target for P2: {}",
            describe_parse_error(&e)
        )
    })?;

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
    Ok(format!(
        "{}\n{}\n{}\n## Main\n{}\n{}",
        CORE_TYPES_FOR_PE, pe, interpreter, encoded, driver
    ))
}

/// Run genuine P3 on a specific target: PE(pe_source, pe_bti(target))
pub fn run_genuine_p3_on_target(
    program: &str,
    core_types: &str,
    interpreter: &str,
) -> Result<String, String> {
    let combined = genuine_p3_on_target_source(program, core_types, interpreter)?;
    with_reference_semantics(|| run_logos_source(&combined))
}

/// The whole program [`run_genuine_p3_on_target`] runs, so a harness can run it on any backend.
pub fn genuine_p3_on_target_source(
    program: &str,
    core_types: &str,
    interpreter: &str,
) -> Result<String, String> {
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
        .map_err(|e| format!("Failed to encode target: {}", describe_parse_error(&e)))?;
    let pe_bti_prog = format!(
        "{}\n{}\n{}\n## Main\n{}\n\
         Let compileEnv be a new Map of Text to CVal.\n\
         Let compileState be makePeState(compileEnv, encodedFuncMap, 200).\n\
         Let compiled be peBlockB(encodedMain, compileState).\n\
         Let runEnv be a new Map of Text to CVal.\n\
         coreExecBlock(compiled, runEnv, encodedFuncMap).\n",
        bti_types, pe_bti, interpreter, target_encoded
    );

    let encoded = encode_program_source_compact(&pe_bti_prog).map_err(|e| {
        format!(
            "Failed to encode pe_bti+target for P3: {}",
            describe_parse_error(&e)
        )
    })?;

    // Execute residual directly — no decompilation needed
    let driver = r#"    Let state be makePeState(a new Map of Text to CVal, encodedFuncMap, 200).
    Let residual be peBlock(encodedMain, state).
    Let runEnv be a new Map of Text to CVal.
    coreExecBlock(residual, runEnv, encodedFuncMap).
"#;
    Ok(format!(
        "{}\n{}\n{}\n## Main\n{}\n{}",
        CORE_TYPES_FOR_PE, pe, interpreter, encoded, driver
    ))
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
pub fn projection2_source_real(
    _core_types: &str,
    _interpreter: &str,
) -> Result<GenuineProjectionResult, String> {
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
    let encoded = encode_program_source_compact(&program).map_err(|e| {
        format!(
            "Failed to encode pe_mini for P2: {}",
            describe_parse_error(&e)
        )
    })?;

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
    let combined = format!(
        "{}\n{}\n{}\n## Main\n{}\n{}",
        CORE_TYPES_FOR_PE, pe, decompile, encoded, driver
    );

    let result = with_reference_semantics(|| run_logos_source(&combined))?;

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
        .map_err(|e| format!("Failed to encode pe_mini: {}", describe_parse_error(&e)))?;

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
    let combined = format!(
        "{}\n{}\n{}\n## Main\n{}\n{}",
        CORE_TYPES_FOR_PE, pe, decompile, encoded, driver
    );

    let result = with_reference_semantics(|| run_logos_source(&combined))?;
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
        .map_err(|e| format!("Failed to encode pe_bti: {}", describe_parse_error(&e)))?;

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
    let combined = format!(
        "{}\n{}\n{}\n## Main\n{}\n{}",
        CORE_TYPES_FOR_PE, pe, decompile, encoded, driver
    );

    let result = with_reference_semantics(|| run_logos_source(&combined))?;
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
    let encoded = encode_program_source_compact(&program).map_err(|e| {
        format!(
            "Failed to encode pe_bti for P3: {}",
            describe_parse_error(&e)
        )
    })?;

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
    let combined = format!(
        "{}\n{}\n{}\n## Main\n{}\n{}",
        CORE_TYPES_FOR_PE, pe, decompile, encoded, driver
    );

    let result = with_reference_semantics(|| run_logos_source(&combined))?;

    // Fix decompiled types for pe_bti specialized functions (B suffix)
    // Fix decompiled types for pe_bti specialized functions (B suffix)
    let result = fix_decompiled_types(
        &result,
        &[
            ("peExprB_", "(e: CExpr) -> CExpr:"),
            ("peBlockB_", "(stmts: Seq of CStmt) -> Seq of CStmt:"),
            ("isStatic_", "(e: CExpr) -> Bool:"),
            ("isLiteral_", "(e: CExpr) -> Bool:"),
            ("allStatic_", "(args: Seq of CExpr) -> Bool:"),
            ("exprToVal_", "(e: CExpr) -> CVal:"),
            ("valToExpr_", "(v: CVal) -> CExpr:"),
            (
                "evalBinOp_",
                "(binOp: Text) and (lv: CVal) and (rv: CVal) -> CVal:",
            ),
            ("isCopyPropSafe_", "(e: CExpr) -> Bool:"),
            ("isVNothing_", "(v: CVal) -> Bool:"),
            ("hasReturn_", "(stmts: Seq of CStmt) -> Bool:"),
            ("extractReturnB_", "(stmts: Seq of CStmt) -> CExpr:"),
            (
                "makeKey_",
                "(fnName: Text) and (args: Seq of CExpr) -> Text:",
            ),
            ("exprToKeyPartB_", "(e: CExpr) -> Text:"),
            ("collectSetVars_", "(stmts: Seq of CStmt) -> Seq of Text:"),
        ],
    );

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
        let before = if pos > 0 {
            remaining.as_bytes()[pos - 1]
        } else {
            b' '
        };
        let after_pos = pos + from.len();
        let after = if after_pos < remaining.len() {
            remaining.as_bytes()[after_pos]
        } else {
            b' '
        };
        let is_word = !before.is_ascii_alphanumeric()
            && before != b'_'
            && !after.is_ascii_alphanumeric()
            && after != b'_';
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
#[path = "../../logicaffeine_tests/unit/logicaffeine_compile/compile/tests.rs"]
mod tests;
