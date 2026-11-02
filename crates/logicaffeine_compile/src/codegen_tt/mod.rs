//! LOGOS → **Tenstorrent** (TT-Metalium) AOT codegen tier. Emits a complete
//! single-Tensix-core TT-Metalium program — a **host** orchestrator plus a
//! **data-movement compute kernel** that runs on one baby RISC-V core
//! (`DataMovementProcessor::RISCV_0`) — for the scalar-compute fragment of the
//! benchmark suite.
//!
//! ## Why the C backend is the compute source
//! A Tensix baby core is a RISC-V CPU and a TT-Metalium kernel is C++; the
//! canonical `add_2_integers_in_riscv` kernel is literally `dat0[0] = dat0[0] +
//! dat1[0]`. So the kernel's compute body is the SAME self-contained C the
//! [`crate::codegen_c`] backend already emits — which is byte-for-byte ≡ the VM
//! (locked by `assert_c_output`). The Tenstorrent tier does not re-lower
//! arithmetic; it *wraps* that proven compute in the TT-Metalium host/kernel
//! split and routes `Show` to the host DPRINT stream. See `wrap`.
//!
//! ## What is verified, and when
//! - **Pre-hardware (now, no card):** the compute is executably ≡ VM via the C
//!   oracle ([`TtArtifact::oracle_c`], run through gcc exactly like the C tier),
//!   and the emitted host/kernel/CMake carry the correct TT-Metalium API surface.
//! - **On-hardware (Phase 5, gated on a Wormhole/Blackhole card + tt-metal SDK):**
//!   the host+kernel build against tt-metal and run on the Tensix core, DPRINT
//!   output compared to the VM. That path is `#[ignore]`d until `TT_METAL_HOME`
//!   is present, so nothing here fabricates an on-silicon result.
//!
//! ## Totality discipline (mirrors the RISC-V / WASM tiers)
//! Any program using an op outside the single-baby-core fragment — CONCURRENCY
//! (channels/tasks/select, which need a `ucontext` scheduler + multiple contexts;
//! the genuine multi-baby-core mesh tier in [`concurrency_mesh`] lowers those) —
//! returns [`TtLowerError::Unsupported`]: a SOUND refusal, never a miscompile.
//! OFFLINE networking ([`TT_NET_OPS`]) is served on-core through the newlib-clean
//! C net runtime (wrapped verbatim). The `tenstorrent_op_coverage_lock` ratchet
//! forces the off-core set onto a shrink-only ledger so nothing is silently dropped.

pub mod concurrency_mesh;
pub mod matmul_lower;
mod runtime;
mod wrap;

pub use runtime::{TT_HOST_TEMPLATE, TT_KERNEL_PREAMBLE};
pub use wrap::wrap_as_tt;

use crate::vm::CompiledProgram;

/// An op / shape outside the current phase's single-baby-core fragment. The
/// `&str` names what is not yet lowered. A refusal — the backend declines to
/// emit rather than emit wrong code (mirrors [`crate::codegen_riscv::RiscVLowerError`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TtLowerError {
    Unsupported(&'static str),
}

impl std::fmt::Display for TtLowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TtLowerError::Unsupported(what) => {
                write!(f, "tenstorrent AOT backend: unsupported {what}")
            }
        }
    }
}

impl std::error::Error for TtLowerError {}

/// A complete single-Tensix-core TT-Metalium program emitted from a LOGOS program.
#[derive(Debug, Clone)]
pub struct TtArtifact {
    /// The TT-Metalium compute kernel (runs on `DataMovementProcessor::RISCV_0`).
    pub kernel_cpp: String,
    /// The host orchestrator: open device, create program + kernel, launch, wait, close.
    pub host_cpp: String,
    /// CMake glue so `host_cpp` builds against a tt-metal checkout (`$TT_METAL_HOME`).
    pub cmake: String,
    /// The pre-hardware ≡VM correctness oracle: the self-contained C program from
    /// [`crate::codegen_c`], whose compute is what the kernel runs. Compiled with
    /// gcc and run on the host, its output is byte-identical to the VM.
    pub oracle_c: String,
}

/// The single canonical name of the generated kernel source (referenced by the
/// host `CreateKernel` call and the CMake glue).
pub const KERNEL_FILE_NAME: &str = "logos_compute_kernel.cpp";

/// The OFFLINE NETWORKING ops the single Tensix baby core serves via the loopback transport
/// ([`crate::codegen_c`]'s `C_NET_RUNTIME`, wrapped verbatim). `Connect`/`Listen`/`Sync` are
/// no-ops, `MakePeer` is identity, `Send`/`Stream` enqueue a 64-bit message into a per-address
/// FIFO, `Await` dequeues it. That runtime is newlib-clean (`strcmp`/`strdup` + static arrays —
/// no `ucontext`, unlike the concurrency runtime), so it rides the SAME wrap the scalar body does
/// and a single-node `Send X to peer; Await …` round-trips ≡ the VM's offline `NetInbox`. Every op
/// here is [`crate::aot_unsupported::Offload::Net`]; excluded from the refusal below (device-served,
/// not host-only). The CUDA analogue is `codegen_cuda::CUDA_NET_OPS`; bound by
/// `tenstorrent_op_coverage_lock`.
pub const TT_NET_OPS: &[&str] = &[
    "NetConnect",
    "NetListen",
    "NetSend",
    "NetStream",
    "NetAwait",
    "NetMakePeer",
    "NetSync",
];

/// Returns `Some(reason)` if `program` uses an op that cannot run on a single
/// Tensix baby core in this phase — concurrency (a Tensix mesh is SPMD, not a
/// shared-memory thread pool with a scheduler; the concurrency runtime needs
/// `ucontext`, which a baby core has no libc for). Everything else — including
/// single-replica CRDT counters (in-memory maps in the C body) and the OFFLINE
/// networking loopback ([`TT_NET_OPS`], the newlib-clean `C_NET_RUNTIME`) —
/// lowers through the proven C compute body. `Sleep` is virtual-time (a
/// scheduler primitive) and is refused with the concurrency set.
pub fn unsupported_reason(program: &CompiledProgram) -> Option<&'static str> {
    use crate::aot_unsupported::Offload;
    // The offload categories (channel / task / virtual-time / net) are refused EXCEPT the offline
    // networking loopback the C net runtime serves ([`TT_NET_OPS`]). So find the first offload op
    // that is NOT net-served; a `Send`/`Await` on a peer passes through to the wrapped C body.
    let op = program.code.iter().find(|op| {
        crate::aot_unsupported::classify(op).is_some() && !TT_NET_OPS.contains(&op.name())
    })?;
    let o = crate::aot_unsupported::classify(op).expect("filtered to Some");
    Some(match o {
        Offload::Channel => "channel op (no shared-memory channels on a single Tensix core)",
        Offload::Task => "task spawn/await (Tensix cores are SPMD, not a thread pool)",
        Offload::VirtualTime => "virtual-time / select (needs a cooperative scheduler)",
        Offload::Net => "networking op (host-only IO)",
        Offload::Dom => "DOM / browser op (a Tensix baby core has no document to build)",
    })
}

/// Emit `program` as a single-Tensix-core TT-Metalium program. `oracle_c` is the
/// self-contained C translation of the SAME program (from [`crate::compile::compile_to_c`]);
/// its compute body becomes the kernel. Returns [`TtLowerError::Unsupported`] if the
/// program uses any op outside the single-baby-core fragment.
pub fn codegen_program_tt(
    program: &CompiledProgram,
    oracle_c: &str,
) -> Result<TtArtifact, TtLowerError> {
    if let Some(reason) = unsupported_reason(program) {
        return Err(TtLowerError::Unsupported(reason));
    }
    Ok(wrap_as_tt(oracle_c))
}
