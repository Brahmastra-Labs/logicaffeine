//! Register bytecode VM.
//!
//! A fast, portable interpreter that runs the same `RuntimeValue` semantics as
//! the tree-walker. It is the browser/WASM execution engine and the substrate
//! the native copy-and-patch JIT tiers up from. Built incrementally, RED-first,
//! differential-tested against the tree-walker.

pub mod compiler;
// Off-thread native compilation (HOTSWAP §6). Native-only: it needs std::thread and
// the forge backend, neither of which exists on wasm (which never installs a tier).
#[cfg(not(target_arch = "wasm32"))]
mod bg_compile;
// AOT-native tier loader (HOTSWAP §Axis-3): dlopen a rustc-built cdylib. Native-only.
#[cfg(not(target_arch = "wasm32"))]
pub mod aot_tier;
// Background AOT-native compilation (HOTSWAP §Axis-3 / P18): build+load off-thread.
#[cfg(not(target_arch = "wasm32"))]
pub mod bg_aot;
// Relocatable per-function bytecode units (HOTSWAP §7) — the producer behind the
// Axis-1 warm side-table and the OPFS tier cache. Pure data; available on all targets.
pub mod fn_bytecode;
// LOGOS_TIER_TRACE observability (HOTSWAP §P13): one stderr line per tier hot-swap.
pub mod tier_trace;
// LOGOS_PERF_MAP: name the JIT's machine code in /tmp/perf-<pid>.map. Native-only (no JIT on wasm).
#[cfg(not(target_arch = "wasm32"))]
pub mod perf_map;
// Tier cache (HOTSWAP §P12): persist a compiled FnBytecode keyed by source+config+tier.
pub mod binisa;
/// Bytecode disassembler — readable text for the Studio debug drawer and any
/// future `largo --disasm`. Pure, headless, no VM state.
pub mod disasm;
/// Which integers of a region of bytecode are held exact: one decision for every backend that lowers the bytecode.
pub(crate) mod exactness;
/// Which registers hold a value of the exact extension, for every backend that lowers the bytecode onto the
/// linked value runtime.
pub(crate) mod tower;
/// The variables of a region of bytecode: its definitions, joined where one read is reached by several.
pub(crate) mod variables;
/// The machine form a number variable is held in when its definitions make numbers of different forms.
pub(crate) mod numbers;
#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/fuzz.rs"]
mod fuzz;
mod instruction;
pub mod isa;
pub mod journal;
mod machine;
/// The narrow (8-byte) NaN-boxed register-file
/// value (see the module docs). Under `feature = "narrow-value"` it backs
/// [`value::Value`] and the VM register file; the default build keeps the fat
/// `RuntimeValue` path, so the scalar accessors/arith here are exercised only by
/// the in-module tests then. `allow(dead_code)` keeps the default build quiet
/// about the methods only the narrow path calls.
#[cfg_attr(not(feature = "narrow-value"), allow(dead_code))]
mod nanbox;
mod native_tier;
pub mod pebble;
pub mod peephole;
pub mod reach;
/// Reversibility classification of the ISA — the static substrate for running the whole language
/// backward (`forward ; reverse ≡ identity`), the software prerequisite for the adiabatic /
/// sub-Landauer chip. Locked by `tests/reverse_op_lock.rs`.
pub mod reverse;
pub mod session;
pub mod tier_cache;
mod value;

/// WebAssembly code generation from VM bytecode: the shared byte emitter (`wasm::encode` +
/// `wasm::func`, always built) plus the WS6 browser WASM-JIT tier (`wasm::region_jit`, behind
/// the `wasm-jit` feature). The direct AOT backend (`largo build --emit wasm`) consumes the
/// same emitter.
/// The "field name" a `Some` arm's binding carries, in place of a real field.
///
/// LOGOS encodes Options transparently — `some x` IS x — so a `Some` arm binds the WHOLE matched
/// value rather than one of its fields. A struct arm and a `Some` arm reach `BindArm` looking
/// identical otherwise, and guessing between them (by binding index, say) silently changes what a
/// real struct arm does. A marker no identifier can spell tells them apart exactly.
pub const SOME_BINDING_MARKER: &str = "\u{0}some";

pub mod wasm;

/// Back-compat facade: the WS6 browser WASM-JIT tier moved into the shared `wasm` codegen
/// submodule — `wasm::func` for the byte emitter, `wasm::region_jit` for the tier + host.
/// `machine.rs` (`super::wasm_jit::WasmTier`) and the `wasm_jit_*` differential/browser test
/// suites still reach it here, unchanged.
#[cfg(feature = "wasm-jit")]
pub mod wasm_jit {
    pub use super::wasm::func::{compile_function_to_wasm, compile_region_to_wasm};
    #[cfg(target_arch = "wasm32")]
    pub use super::wasm::region_jit::run_on_host;
    pub use super::wasm::region_jit::WasmTier;
}

pub use compiler::Compiler;
pub use disasm::{disassemble, format_constant, op_io, DisasmLine, OpIo};
pub use instruction::{
    BoundaryType, ChanElem, CompiledProgram, Constant, EnumTypeDef, EnumVariantDef, ObsSiteDesc,
    ObsSiteKind, Op, Reg, StructTypeDef, NO_SITE,
};
pub use journal::{HistoryTape, JournalEntry};
pub use machine::Vm;
pub(crate) use machine::{DebugView, DebugVmState, HeapObjView};
pub(crate) use machine::{VmBlock, VmStep};
pub use reverse::{reversibility, EffectKind, InverseSpec, JournalKind, RevClass};
// The reversible cooperative scheduler + the compiled-function shape — the in-crate soft-core differential
// (`codegen_sva/rtl.rs`) hand-builds bytecode for the scheduler-only ops (`ChanClose`/`TaskAwait`) that no
// source emits, and drives them through this scheduler for its VM oracle. `CompiledFunction` is also public
// so the fabric coverage lock can hand-build a SPAWN witness (a producer child) the same way.
pub use instruction::CompiledFunction;
pub(crate) use machine::RevCoop;
pub use native_tier::{
    install_native_tier, installed_native_tier, ArrayPin, CalleeSig, FnTable, HoistGuard,
    NativeCtx, NativeFn, NativeFrame, NativeOutcome, NativeRet, NativeTier, NativeWriteback,
    ObservedKind, ParamKind, PinElem, RegBox, RegionFn, RegionOutcome, RegionReturn,
    RegionReturnKind, SlotKind, NATIVE_TIER_THRESHOLD, REGION_TIER_THRESHOLD,
};
pub use value::Value;

use crate::ast::stmt::Stmt;
use crate::intern::Interner;

/// Hard cap on registers a single frame may claim. Generous for real programs;
/// prevents a pathological frame from claiming the whole register file.
pub const MAX_REGISTERS_PER_FRAME: usize = 16_384;
/// Hard cap on the total register file across all live frames. Deep recursion
/// hits this and errors instead of consuming unbounded memory.
pub const MAX_REGISTER_FILE: usize = 1 << 20;
/// Maximum expression nesting depth the compiler will descend before erroring
/// (guards the compiler's own recursion against adversarial inputs). Debug
/// builds allocate every match arm's locals in `compile_expr_into_inner`'s
/// frame — measured in the tens of KiB per nesting level as the compiler
/// grows — so 64 keeps the worst case well inside a default 2 MiB
/// test-thread stack while still exceeding any realistic expression depth.
/// (An explicit-work-stack compiler would remove the native-stack coupling
/// entirely; tracked as future work.)
pub const MAX_EXPR_DEPTH: usize = 64;

/// Shannon entropy of a two-outcome branch profile, in bits (EXODIA 3.3).
/// 0.0 = perfectly predictable (the hardware branch predictor wins; Tier-1
/// code is fine), 1.0 = a coin flip (Tier-2 should restructure the branch
/// away). Zero samples are zero entropy.
pub fn branch_entropy(taken: u64, not_taken: u64) -> f64 {
    let total = taken + not_taken;
    if total == 0 || taken == 0 || not_taken == 0 {
        return 0.0;
    }
    let p = taken as f64 / total as f64;
    -(p * p.log2() + (1.0 - p) * (1.0 - p).log2())
}

/// Compile a statement block to bytecode and run it, returning captured output.
pub fn compile_and_run(stmts: &[Stmt], interner: &Interner) -> Result<String, String> {
    let program = Compiler::compile(stmts, interner)?;
    let mut vm = Vm::new(&program);
    if let Some(tier) = installed_native_tier() {
        vm = vm.with_native_tier(tier);
    }
    vm.run()?;
    Ok(vm.output().to_string())
}

/// Compile and run, preserving partial output alongside any error — the shape
/// the differential harness compares against the tree-walker (which also keeps
/// the lines emitted before a runtime error). `types` carries the discovery
/// pass's struct definitions (for default-fill on construction).
pub fn run_to_outcome(
    stmts: &[Stmt],
    interner: &Interner,
    types: Option<&crate::analysis::TypeRegistry>,
    policies: Option<&crate::analysis::PolicyRegistry>,
) -> (String, Option<String>) {
    let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
    let program = match Compiler::compile_with_oracle(stmts, interner, types, Some(oracle)) {
        Ok(p) => p,
        Err(e) => return (String::new(), Some(e)),
    };
    let mut vm = Vm::new(&program);
    if let Some(tier) = installed_native_tier() {
        vm = vm.with_native_tier(tier);
    }
    if let Some(registry) = policies {
        vm = vm.with_policy_ctx(registry, interner);
    }
    match vm.run() {
        Ok(()) => (vm.output().to_string(), None),
        Err(e) => (vm.output().to_string(), Some(e)),
    }
}

/// [`run_to_outcome`], plus the failure-site id in force at the point an `Err` propagated from
/// (Wave 3: "runtime failures carry a location") — `None` on success, or when the failure happened
/// outside any `Stmt::Site` marker (`## Main`/`## Test`'s own top-level statements carry none).
/// `compile.rs::locate_runtime_failure` is the one place this id becomes a rendered `file:line:col`.
pub fn run_to_outcome_located(
    stmts: &[Stmt],
    interner: &Interner,
    types: Option<&crate::analysis::TypeRegistry>,
    policies: Option<&crate::analysis::PolicyRegistry>,
) -> (String, Option<String>, Option<u32>) {
    let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
    let program = match Compiler::compile_with_oracle(stmts, interner, types, Some(oracle)) {
        Ok(p) => p,
        Err(e) => return (String::new(), Some(e), None),
    };
    let mut vm = Vm::new(&program);
    if let Some(tier) = installed_native_tier() {
        vm = vm.with_native_tier(tier);
    }
    if let Some(registry) = policies {
        vm = vm.with_policy_ctx(registry, interner);
    }
    match vm.run() {
        Ok(()) => (vm.output().to_string(), None, None),
        Err(e) => {
            let site = vm.failing_site();
            (vm.output().to_string(), Some(e), site)
        }
    }
}

/// RUN-ONLY wall-time — the MIN nanoseconds over `iters` executions of the CHIP bytecode on the register
/// VM, compiled ONCE (so parse / compile / stdlib-prelude load are excluded). This is the fair host
/// counterpart to the FPGA soft-core's on-chip cycle counter, which also measures execution only — and it
/// runs the SAME `compile_for_chip` bytecode the silicon executes, so VM-vs-FPGA is one instruction stream,
/// host CPU vs custom silicon. `None` if the program does not compile to the chip subset.
pub fn run_nanos(
    stmts: &[Stmt],
    interner: &Interner,
    types: Option<&crate::analysis::TypeRegistry>,
    iters: usize,
) -> Option<u128> {
    let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
    let program = Compiler::compile_for_chip(stmts, interner, types, Some(oracle)).ok()?;
    {
        let mut vm = Vm::new(&program);
        let _ = vm.run(); // warm up
    }
    let mut best = u128::MAX;
    for _ in 0..iters {
        let mut vm = Vm::new(&program);
        let t = std::time::Instant::now();
        let _ = vm.run();
        best = best.min(t.elapsed().as_nanos());
    }
    Some(best)
}

/// [`run_to_outcome`] in the OBSERVED profile (Wave 13): `Note`/`Warn`/`Record`/
/// `During` lower to `Op::Obs`, so the VM emits the SAME observation event stream
/// the tree-walker does (into the `observe` channel — separate from program
/// output). The dev/test differential entry; the compiled/release tiers keep the
/// elided profile. Program output is byte-identical to [`run_to_outcome`].
pub fn run_to_outcome_observed(
    stmts: &[Stmt],
    interner: &Interner,
    types: Option<&crate::analysis::TypeRegistry>,
    policies: Option<&crate::analysis::PolicyRegistry>,
) -> (String, Option<String>) {
    let program = match Compiler::compile_observed(stmts, interner, types) {
        Ok(p) => p,
        Err(e) => return (String::new(), Some(e)),
    };
    let mut vm = Vm::new(&program);
    if let Some(registry) = policies {
        vm = vm.with_policy_ctx(registry, interner);
    }
    match vm.run() {
        Ok(()) => (vm.output().to_string(), None),
        Err(e) => (vm.output().to_string(), Some(e)),
    }
}

/// Run a `## Test` body's COMPUTATION on the bytecode VM for the `--tiers all`
/// differential (Wave 13): the test-only statements (`Expect`/`ForAll`/…) compile
/// as no-ops, so the VM executes exactly the surrounding computation.
///
/// STRICT shape: `Err(reason)` means the VM never ran the program (the compiler
/// refused it — a typed refusal, distinguishable from a runtime failure);
/// `Ok((output, error))` means the VM genuinely executed, with its output as
/// newline-joined lines (the interpreter's convention, so the tier comparison
/// is byte-for-byte) and any runtime error alongside the partial output.
pub fn run_tiers_strict(
    stmts: &[Stmt],
    interner: &Interner,
    types: Option<&crate::analysis::TypeRegistry>,
    policies: Option<&crate::analysis::PolicyRegistry>,
) -> Result<(String, Option<String>), String> {
    let program = Compiler::compile_tiers(stmts, interner, types)?;
    let mut vm = Vm::new(&program);
    if let Some(registry) = policies {
        vm = vm.with_policy_ctx(registry, interner);
    }
    let error = vm.run().err();
    Ok((vm.into_lines().join("\n"), error))
}

/// Drive a prepared VM to completion, honoring the thread's armed fuel (`largo
/// run --max-steps`, the MCP `step_budget`). A budgeted run stays on the
/// resumable STEPPED loop and stops at the deterministic op where the shared
/// cell runs dry; an unbudgeted run is the untouched hot loop. The ONE place
/// this dance lives — the live sync path and the strict engine runs share it.
pub(crate) fn drive_vm(vm: &mut Vm) -> Option<String> {
    match crate::interpreter::active_fuel() {
        Some(fuel) => {
            logicaffeine_base::money::clear_ambient_rates();
            let before = vm.steps_run();
            let step = vm.run_steps(fuel.remaining.get());
            let spent = vm.steps_run() - before;
            fuel.remaining.set(fuel.remaining.get().saturating_sub(spent));
            match step {
                Ok(VmStep::Paused) => {
                    fuel.remaining.set(0);
                    fuel.exhausted.set(true);
                    Some(fuel.exhaust_message())
                }
                Ok(VmStep::Blocked) => {
                    Some("vm: concurrency op requires the scheduler driver".to_string())
                }
                Ok(VmStep::Done(_)) => None,
                Err(e) => Some(e),
            }
        }
        None => vm.run().err(),
    }
}

/// [`run_to_outcome`] with the program argument vector for the `args()`
/// system native (full argv; index 0 is the program name) and an optional
/// caller-supplied native tier. A `Some(tier)` overrides the process-wide
/// installed tier — differential suites use a private tier per program so
/// its compile counters are isolated from every other test in the binary.
pub fn run_to_outcome_with_args(
    stmts: &[Stmt],
    interner: &Interner,
    types: Option<&crate::analysis::TypeRegistry>,
    policies: Option<&crate::analysis::PolicyRegistry>,
    program_args: &[String],
    tier: Option<&dyn NativeTier>,
) -> (String, Option<String>) {
    let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
    let program = match Compiler::compile_with_oracle(stmts, interner, types, Some(oracle)) {
        Ok(p) => p,
        Err(e) => return (String::new(), Some(e)),
    };
    let mut vm = Vm::new(&program).with_program_args(program_args.to_vec());
    let chosen: Option<&dyn NativeTier> = match tier {
        Some(t) => Some(t),
        None => installed_native_tier().map(|t| t as &dyn NativeTier),
    };
    if let Some(t) = chosen {
        vm = vm.with_native_tier(t);
    }
    if let Some(registry) = policies {
        vm = vm.with_policy_ctx(registry, interner);
    }
    match vm.run() {
        Ok(()) => (vm.output().to_string(), None),
        Err(e) => (vm.output().to_string(), Some(e)),
    }
}

/// [`run_to_outcome_with_args`] with BACKGROUND native compilation (HOTSWAP §6): hot
/// functions compile on a worker thread while the interpreter keeps running bytecode,
/// then their chains are drained + published. Requires a process-installed tier (the
/// worker needs a `&'static` backend); with none installed it runs pure bytecode.
/// Drains outstanding compiles before returning so the native tier engages
/// deterministically for the differential gates.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_to_outcome_bg(
    stmts: &[Stmt],
    interner: &Interner,
    types: Option<&crate::analysis::TypeRegistry>,
    policies: Option<&crate::analysis::PolicyRegistry>,
    program_args: &[String],
) -> (String, Option<String>) {
    let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
    let program = match Compiler::compile_with_oracle(stmts, interner, types, Some(oracle)) {
        Ok(p) => p,
        Err(e) => return (String::new(), Some(e)),
    };
    let mut vm = Vm::new(&program).with_program_args(program_args.to_vec());
    if let Some(t) = installed_native_tier() {
        vm = vm.with_bg_native_tier(t);
    }
    if let Some(registry) = policies {
        vm = vm.with_policy_ctx(registry, interner);
    }
    let result = match vm.run() {
        Ok(()) => (vm.output().to_string(), None),
        Err(e) => (vm.output().to_string(), Some(e)),
    };
    vm.drain_pending_compiles();
    result
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/tests.rs"]
mod tests;
