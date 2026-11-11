//! LOGOS → **CUDA** AOT codegen tier. Emits ONE self-contained CUDA program — a
//! `__global__` kernel that runs a whole LOGOS program on a single GPU thread, plus a
//! host orchestrator — for the integer/bool scalar fragment of the language.
//!
//! ## Why this tier tree-shakes, rather than wrapping the whole runtime
//! A Tenstorrent baby core is a RISC-V CPU, so [`crate::codegen_tt`] reuses the entire
//! glibc-backed C body verbatim. A GPU thread is not a CPU: device code has no
//! `realloc`/`calloc`/`strdup`/`snprintf`/`strtod`, which the C runtime leans on for
//! every string, map, and big integer. And `codegen_c` emits the WHOLE ~1900-line runtime
//! for every program, so lifting it wholesale would need device bignum + `vsnprintf` even
//! for a float one-liner. So the CUDA tier TREE-SHAKES the C oracle (via
//! [`crate::codegen_c::cshake`]): it lifts only the runtime slice a program actually
//! references (see `device_lift`), `__device__`-annotating each unit, applying device
//! OVERRIDES for the irreducibly-host ones (`logos_fmt_f64` → a from-scratch Dragon4
//! shortest-float formatter, `device_float`), and REFUSING (typed) any reachable unit
//! that still needs a device-absent facility. A program is accepted iff BOTH pass:
//!   1. every op is in the scalar fragment ([`CUDA_SCALAR_OPS`]); and
//!   2. every reachable runtime unit is device-liftable (verbatim, shimmed, or overridden).
//!
//! ## What is verified, and when
//! - **Pre-toolkit (no nvcc):** the artifact is structurally a well-formed CUDA program,
//!   and `oracle_c` is byte-for-byte the C tier's proven ≡VM body.
//! - **On-GPU (gated on `nvcc` + a device):** the emitted `.cu` compiles with `nvcc` and
//!   its output equals the tree-walker over the corpus (`cuda_aot_lock` / `cuda_float_lock`,
//!   skipped when the toolkit is absent — never a failure, mirroring the RISC-V/qemu gate).
//!
//! ## Totality discipline (mirrors the RISC-V / Tenstorrent tiers)
//! Every refusal is [`CudaLowerError::Unsupported`] — a SOUND decline, never a
//! miscompile. The `cuda_aot_lock` ratchet holds the supported-op floor and a shrink-only
//! gap list so the fragment can only grow.

pub mod chip;
pub(crate) mod device_float;
mod device_lift;
mod device_string;
pub mod forward_shim;
pub mod fused_row;
pub mod gdn;
pub mod gemm_tile;
pub mod knobs;
pub mod lane;
pub mod lane_link;
pub mod lane_shim;
pub mod matrix;
pub mod provenance;
pub mod qwen38;
pub mod qwen38_batched;
pub mod qwen_batched;
pub mod qwen_emit;
mod runtime;
pub mod solbench;
mod wrap;

pub use chip::{CudaChip, GpuFamily};
pub use device_float::LOGOS_DEVICE_FLOAT_FMT;
pub use fused_row::{emit_fused_row, emit_swiglu};
pub use knobs::{CachePrecision, CudaKnobs, KnobTradeoff, LoadWidth, Logits, MatmulPrecision, StatePrecision, STRICTLY_BETTER};
pub use matrix::{
    emit_int8_gemm, lower_elementwise_to_cuda, lower_matmul_to_cuda, lower_matmul_to_cuda_with,
    lower_reduction_to_cuda, sample_gemm_data, CudaGemmArtifact,
};
pub use runtime::{CUDA_MAKEFILE_TEMPLATE, KERNEL_FILE_NAME};
pub use solbench::{emit_fused_row_solution, SolSolution};

use crate::vm::{CompiledProgram, Op};

/// An op / value outside the CUDA scalar fragment. The `&str` names what is not lifted.
/// A refusal — the backend declines rather than emit wrong code (mirrors
/// [`crate::codegen_riscv::RiscVLowerError`] / [`crate::codegen_tt::TtLowerError`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CudaLowerError {
    Unsupported(&'static str),
    /// The C tier refuses the program: its C carries an `#error` naming what it does not lower, and the
    /// device lowers the same C.
    CRefused(String),
}

impl std::fmt::Display for CudaLowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CudaLowerError::Unsupported(what) => write!(f, "cuda AOT backend: unsupported {what}"),
            CudaLowerError::CRefused(why) => write!(f, "cuda AOT backend: {why}"),
        }
    }
}

/// The first refusal the C tier wrote into `c` — the words of its `#error` line.
fn c_refusal(c: &str) -> Option<String> {
    c.lines().find_map(|line| {
        let words = line.trim_start().strip_prefix("#error")?.trim();
        Some(
            words
                .strip_prefix('"')
                .and_then(|w| w.strip_suffix('"'))
                .unwrap_or(words)
                .to_string(),
        )
    })
}

impl std::error::Error for CudaLowerError {}

/// A complete single-file CUDA program emitted from a LOGOS program.
#[derive(Debug, Clone)]
pub struct CudaArtifact {
    /// The self-contained `.cu`: device preamble + the lifted `__device__` program +
    /// the `__global__` entry + the host orchestrator. Built with `nvcc`.
    pub kernel_cu: String,
    /// Makefile glue (`nvcc -arch=sm_86 …`).
    pub makefile: String,
    /// The pre-GPU ≡VM correctness oracle: the self-contained C program from
    /// [`crate::codegen_c`], whose integer/bool compute the kernel runs. Compiled with
    /// gcc on the host, its output is byte-identical to the VM.
    pub oracle_c: String,
}

/// The op names the CUDA scalar fragment lowers today — integer/bool arithmetic,
/// comparisons, control flow, calls, globals, and `Show`. EXHAUSTIVELY enforced against
/// the VM ISA by `cuda_aot_lock` (floor + shrink-only gaps), so this set can only grow.
/// A scalar op whose value escapes to BigInt/Rational/Float/Text is still caught, by the
/// device-cleanliness backstop — this list is necessary, not sufficient, for support.
pub const CUDA_SCALAR_OPS: &[&str] = &[
    "LoadConst",
    "Move",
    "RevSwap",
    "RevXor",
    "RevNeg",
    "RevAdd",
    "RevSub",
    "Add",
    "AddAssign",
    "Sub",
    "Mul",
    "Div",
    "ExactDiv",
    "FloorDiv",
    "Mod",
    "DivPow2",
    "MagicDivU",
    "Lt",
    "Gt",
    "LtEq",
    "GtEq",
    "Eq",
    "ApproxEq",
    "NotEq",
    "Not",
    "Pow",
    "BitXor",
    "BitAnd",
    "BitOr",
    "Shl",
    "Shr",
    "Jump",
    "JumpIfFalse",
    "JumpIfTrue",
    "Call",
    "CallBuiltin",
    "GlobalGet",
    "GlobalSet",
    "Return",
    "ReturnNothing",
    "Show",
    "Halt",
    // Phase 1 — the runtime-error path: a fail site (`FailWith`, a non-exhaustive `Inspect` fallthrough,
    // a div-by-zero guard) becomes a device trap (CUDA_DEVICE_FAULT_SHIM) → nonzero host exit ≡ the VM's
    // error. The stderr diagnostic is not part of the ≡VM comparison.
    "FailWith",
    // Phase 2 — collections / heap (device allocator shim). A program `Show`ing a whole collection
    // still needs the device string runtime (Phase 3) and is refused at the value-slice level; these
    // ops themselves run ≡VM whenever the reachable slice is device-clean.
    "NewList",
    "NewEmptyList",
    "NewEmptyListWithCapacity",
    "NewEmptyListFixed",
    "NewEmptyListI32",
    "NewEmptySet",
    "NewEmptyMap",
    "NewEmptyMapWithCapacity",
    "NewRange",
    "ListPush",
    "ListPop",
    "ListPushField",
    "SetAdd",
    "RemoveFrom",
    "SetIndex",
    "SetIndexUnchecked",
    "Index",
    "IndexUnchecked",
    "RegionBoundsGuard",
    "Length",
    "Contains",
    "SliceOp",
    "UnionOp",
    "IntersectOp",
    "SeqConcat",
    "DeepClone",
    "EnsureOwned",
    "IterPrepare",
    "IterNext",
    "IterPop",
    // Phase 3 — text: string concat + interpolation/format specs (device vsnprintf/str runtime).
    "Concat",
    "FormatValue",
    // Phase 4 — aggregates: struct + tuple literals (synthesized C types lift as device structs).
    // `DestructureTuple` (Phase 4b) rides the same synthesized `LTuple_…` device structs: a
    // `Repeat for (a, b) in pairs` over a Seq-of-tuple binds each pattern var to a positional field.
    // Enum (NewInductive/TestArm/BindArm) is still Phase 4b (see cuda_aot_lock).
    "NewStruct",
    "StructInsert",
    "GetField",
    "NewTuple",
    "DestructureTuple",
    // Phase 4b — enums: `new <Variant>` + `Inspect`/`When` match over a device tagged-union.
    "NewInductive",
    "TestArm",
    "BindArm",
    // Phase 2b — closures: a non-capturing lambda lambda-lifts to a top-level `__device__` function;
    // `CallValue` is an indirect call through a device function pointer (nvcc supports these).
    "MakeClosure",
    "CallValue",
    // Phase 4c — policy PREDICATE check: `Check that <subj> is <pred>` lowers to an inline guard
    // `if (!(<condition>)) trap` where the condition is the `## Policy` predicate emitted over the subject's
    // device struct fields (`FieldEquals`→strcmp/`==`, `FieldBool`, nested `Predicate`, `And`/`Or`) —
    // byte-identical to `evaluate_policy_condition`. A denied check traps (CUDA_DEVICE_FAULT_SHIM) → nonzero
    // exit ≡ the VM's `Security Check Failed`. Capability (`can <action> the <object>`) checks resolve their
    // object by TYPE and error in BOTH oracles through this path, so they are not ≡VM-exercisable; codegen_c
    // refuses any not-yet-lowerable condition form LOUDLY (`#error`), never a silent skip.
    "CheckPolicy",
    // Phase 6 — temporal: `today`/`now` read the deterministic fixed clock the HOST injects into device
    // globals before launch (CUDA_DEVICE_CLOCK); `Show` renders the date/moment via device calendar math.
    "LoadToday",
    "LoadNow",
    // Phase 6 — argv: `args()` reads the device `_logos_argc/_logos_argv` globals the HOST marshals its own
    // argv into (cudaMalloc/cudaMemcpy + cudaMemcpyToSymbol) before launch; `logos_args` builds the Seq.
    "Args",
    // Phase 6 — counter CRDT on the single device thread: `NewCrdt` default-fills a `ConvergentCount`/
    // `Tally` field, `CrdtBump` is `Increase`/`Decrease`, `CrdtMerge` is the per-replica-MAX lattice join
    // (idempotent). The `logos_crdt` runtime is a device-clean integer count-map (malloc shim + arithmetic),
    // so the slice lifts verbatim. `CrdtAppend` (Append to a shared `Seq` field) + `CrdtResolve` (a LWW
    // register — last write wins) are now codegen_c-faithful (both were `/* unsupported stmt */` stubs) and
    // device-clean. `CrdtJoin` stays Deferred (untested).
    "NewCrdt",
    "CrdtBump",
    "CrdtMerge",
    "CrdtAppend",
    "CrdtResolve",
    // Phase 6a — CONCURRENCY: every task runs on the stepped scheduler — the C family's scheduler core under
    // step functions over heap frames (`codegen_c::stepped`), placed on the device whole — so pipes, tasks,
    // selects and the logical clock schedule on the GPU exactly as on the host C tier, at every seed.
    "ChanNew",
    "ChanSend",
    "ChanRecv",
    "ChanTrySend",
    "ChanTryRecv",
    "ChanClose",
    "Spawn",
    "SpawnHandle",
    "TaskAwait",
    "TaskAbort",
    "Sleep",
    "SelectArmRecv",
    "SelectArmTimeout",
    "SelectWait",
    // Phase 6c — OFFLINE NETWORKING (the loopback transport). `Connect`/`Listen`/`Sync` no-op, `MakePeer` is
    // identity, `Send`/`Stream` enqueue a 64-bit message into a per-address FIFO, `Await` dequeues it — the
    // device-lifted `C_NET_RUNTIME` (strcmp/strdup shimmed). Single-node round-trip ≡ the VM's offline model.
    "NetConnect",
    "NetListen",
    "NetSend",
    "NetStream",
    "NetAwait",
    "NetMakePeer",
    "NetSync",
];

/// The OFFLINE NETWORKING ops the device serves via the loopback transport (`codegen_c`'s `C_NET_RUNTIME`,
/// device-lifted). `Connect`/`Listen`/`Sync` are no-ops, `MakePeer` is identity, `Send`/`Stream` enqueue a
/// 64-bit message into a per-address FIFO, `Await` dequeues it — so a single-node `Send X to peer; Await …`
/// round-trips ≡ the VM's offline `NetInbox`. Every op here is also in [`CUDA_SCALAR_OPS`]; excluded from the
/// offload refusal below (they are now device-served, not host-only). Bound by `cuda_aot_lock`.
pub const CUDA_NET_OPS: &[&str] = &[
    "NetConnect",
    "NetListen",
    "NetSend",
    "NetStream",
    "NetAwait",
    "NetMakePeer",
    "NetSync",
];

/// `Some(reason)` if `program` uses an op outside the CUDA scalar fragment. Host-only IO and the
/// DOM get the shared single-device reason ([`crate::aot_unsupported`]); everything else that is not
/// a scalar op is named directly.
pub fn unsupported_reason(program: &CompiledProgram) -> Option<&'static str> {
    use crate::aot_unsupported::Offload;
    // Of the offload categories, the device serves every channel / task / virtual-time op (the stepped
    // scheduler) and the loopback networking ops (`CUDA_NET_OPS`); the first op it does not serve is refused.
    if let Some(o) = program
        .code
        .iter()
        .find_map(|op| match crate::aot_unsupported::classify(op) {
            Some(o @ (Offload::Net | Offload::Dom)) if !CUDA_NET_OPS.contains(&op.name()) => {
                Some(o)
            }
            _ => None,
        })
    {
        return Some(match o {
            Offload::Net => "networking op (host-only IO)",
            _ => "DOM / browser op (a GPU kernel has no document to build)",
        });
    }
    program
        .code
        .iter()
        .find(|op| !CUDA_SCALAR_OPS.contains(&op.name()))
        .map(op_refusal)
}

/// A stable reason string for a non-scalar op (so the message is meaningful without
/// leaking the whole op zoo). Grouped by the runtime surface it would require.
fn op_refusal(op: &Op) -> &'static str {
    match op.name() {
        "NewList"
        | "NewEmptyList"
        | "NewEmptyListWithCapacity"
        | "NewEmptyListFixed"
        | "NewEmptyListI32"
        | "NewEmptySet"
        | "NewEmptyMap"
        | "NewEmptyMapWithCapacity"
        | "NewRange"
        | "ListPush"
        | "ListPop"
        | "ListPushField"
        | "SetAdd"
        | "RemoveFrom"
        | "SetIndex"
        | "SetIndexUnchecked"
        | "Index"
        | "IndexUnchecked"
        | "RegionBoundsGuard"
        | "Length"
        | "Contains"
        | "SliceOp"
        | "UnionOp"
        | "IntersectOp"
        | "SeqConcat" => "collection op (needs the device heap runtime, not yet lifted)",
        "Concat" | "FormatValue" => "text/format op (needs the device string runtime)",
        "NewStruct" | "StructInsert" | "GetField" | "NewInductive" | "TestArm" | "BindArm"
        | "NewTuple" | "DestructureTuple" => {
            "struct/enum/tuple op (aggregate values not yet lifted)"
        }
        "CrdtBump" | "CrdtMerge" | "CrdtJoin" | "NewCrdt" | "CrdtAppend" | "CrdtResolve" => {
            "CRDT op (replicated state, off a single kernel)"
        }
        "BigIntWide" => "wide-integer op (256-bit lane not yet lifted)",
        "IterPrepare" | "IterNext" | "IterPop" => {
            "snapshot iteration (needs the device heap runtime)"
        }
        "LoadToday" | "LoadNow" => "temporal op (host clock, off the datapath)",
        "MakeClosure" | "CallValue" => "closure op (heap-allocated captures not yet lifted)",
        "Obs" => "host-side op (observation, off the datapath)",
        "FailWith" => "runtime-error path (host exit, not yet a device trap)",
        "DeepClone" | "EnsureOwned" => "ownership op (implies a heap collection, not yet lifted)",
        _ => "op outside the CUDA scalar fragment",
    }
}

/// Emit `program` as a single-file CUDA program. `oracle_c` is the self-contained C
/// translation of the SAME program (from [`crate::compile::compile_to_c_stepped`], which is
/// [`crate::compile::compile_to_c`]'s C for a program with no tasks); its program-region
/// compute is lifted onto the device. Returns [`CudaLowerError::Unsupported`]
/// if the program leaves the scalar fragment (op-level or value-escape).
pub fn codegen_program_cuda(
    program: &CompiledProgram,
    oracle_c: &str,
) -> Result<CudaArtifact, CudaLowerError> {
    if let Some(reason) = unsupported_reason(program) {
        return Err(CudaLowerError::Unsupported(reason));
    }
    if let Some(why) = c_refusal(oracle_c) {
        return Err(CudaLowerError::CRefused(why));
    }
    // Tree-shake the C oracle and lift only the reachable runtime slice onto the device. A reachable
    // unit that still needs a device-absent facility (bignum, string builder, …) is a typed refusal —
    // never a miscompile. This REPLACES the old whole-oracle string scan with a precise per-slice check.
    let kernel_cu =
        device_lift::lift_program_to_device(oracle_c).map_err(CudaLowerError::Unsupported)?;
    Ok(CudaArtifact {
        kernel_cu,
        makefile: CUDA_MAKEFILE_TEMPLATE.replace("{KERNEL_FILE}", KERNEL_FILE_NAME),
        oracle_c: oracle_c.to_string(),
    })
}
