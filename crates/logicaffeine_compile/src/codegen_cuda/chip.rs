//! The GPU **chip / family** knob — which NVIDIA silicon the CUDA backend targets.
//!
//! This is the CUDA analog of the Tenstorrent backend's per-part [`crate::spatial::target::SpatialTarget`]
//! (wormhole / blackhole): a described target part whose SM architecture, shared-memory budget, and realized
//! numeric formats make the fastest kernel *chip-specific*. The SAME recognized LOGOS tensor op lowers to a
//! kernel tuned for the local **RTX 3060** (Ampere, sm_86) — where we prove correctness — or for a cloud
//! **B200** (Blackwell, sm_100), the target we submit to NVIDIA's SOL-ExecBench against.
//!
//! Choosing the chip is a genuine *trade-off* (hence a knob, not a constant): it trades **portability** against
//! **hardware exploitation**. A kernel compiled for `sm_100` won't run on an `sm_86` host; a kernel compiled
//! for `sm_86` leaves the B200's 5th-gen tensor cores, fp8/nvfp4, and larger shared memory unused.

use crate::codegen_sva::fabric_target::Precision;

/// An NVIDIA GPU architecture family — fixes the tensor-core generation and which numeric formats exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuFamily {
    /// sm_80 / sm_86 — 3rd-gen tensor cores: bf16 / fp16 / tf32, int8 IMMA. A100, RTX 30xx.
    Ampere,
    /// sm_89 — 4th-gen tensor cores + fp8 (e4m3/e5m2). Ada: RTX 40xx / L40.
    Ada,
    /// sm_90 — 4th-gen tensor cores, wgmma / TMA, fp8. Hopper: H100 / H200.
    Hopper,
    /// sm_100 — 5th-gen tensor cores, tcgen05, fp8 + NVFP4 16-element block scaling. Blackwell: B200 / GB200.
    Blackwell,
}

/// A described CUDA target part — the hardware limits and realized numeric formats that make the fastest
/// kernel chip-specific, plus the SM arch its kernels are compiled for. Construct with a named part
/// ([`CudaChip::rtx3060`], [`CudaChip::b200`], …); the emitter reads it to pick the arch flag, the shared-memory
/// budget, and which precisions are legal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CudaChip {
    /// Human-readable part name — carried into the emitted kernel's provenance header.
    pub name: &'static str,
    /// The architecture family (tensor-core generation).
    pub family: GpuFamily,
    /// SM architecture number (80 / 86 / 89 / 90 / 100) — the `sm_XX` the kernel compiles for.
    pub sm_arch: u32,
    /// Opt-in dynamic shared-memory ceiling per block, in KiB (bounds the resident row/tile a kernel stages).
    pub smem_kib_per_block: usize,
    /// Threads per block ceiling.
    pub max_threads_per_block: usize,
    /// Streaming multiprocessors: the blocks a launch must offer to fill the part.
    pub sms: usize,
    /// The part's device memory, in bytes: what a lane plans its program's weights, caches and arrays on, less what the
    /// driver and the program's context take (`lane::residence::CONTEXT_RESERVE`).
    pub mem_bytes: usize,
    /// The numeric formats this part's tensor / CUDA cores realize — the precision legality gate (a lowering
    /// may pick a precision only if the target chip provides it).
    pub precisions: &'static [Precision],
}

/// Ampere/Ada realize bf16/fp16/fp32 (+ the int formats, int32 the CUDA cores' own word) but no fp8; the tensor path
/// picks among these.
const AMPERE_PRECISIONS: &[Precision] = &[
    Precision::Int8,
    Precision::Int16,
    Precision::Int32,
    Precision::Int64,
    Precision::Bf16,
    Precision::Fp16,
    Precision::Fp32,
];
/// Hopper/Blackwell add fp8. (NVFP4 joins here in Phase 5, once `Precision::Nvfp4` and its witness ship.)
const HOPPER_PLUS_PRECISIONS: &[Precision] = &[
    Precision::Int8,
    Precision::Int16,
    Precision::Int32,
    Precision::Int64,
    Precision::Bf16,
    Precision::Fp16,
    Precision::Fp32,
    Precision::Fp8,
];

impl CudaChip {
    /// **RTX 3060** — the local dev GPU: Ampere, sm_86, 28 SMs, ~99 KiB opt-in shared memory, 12 GiB. Where we prove
    /// correctness and relative speed before spending B200 time.
    pub fn rtx3060() -> Self {
        CudaChip {
            name: "rtx3060",
            family: GpuFamily::Ampere,
            sm_arch: 86,
            smem_kib_per_block: 99,
            max_threads_per_block: 1024,
            sms: 28,
            mem_bytes: 12 << 30,
            precisions: AMPERE_PRECISIONS,
        }
    }

    /// **A100** — Ampere data-center part, sm_80, 108 SMs, ~164 KiB opt-in shared memory, the 80 GB part.
    pub fn a100() -> Self {
        CudaChip {
            name: "a100",
            family: GpuFamily::Ampere,
            sm_arch: 80,
            smem_kib_per_block: 164,
            max_threads_per_block: 1024,
            sms: 108,
            mem_bytes: 80 << 30,
            precisions: AMPERE_PRECISIONS,
        }
    }

    /// **H100** (SXM / NVL) — Hopper, sm_90, 132 SMs, ~228 KiB opt-in shared memory, 80 GB (the SXM part), fp8.
    pub fn h100() -> Self {
        CudaChip {
            name: "h100",
            family: GpuFamily::Hopper,
            sm_arch: 90,
            smem_kib_per_block: 228,
            max_threads_per_block: 1024,
            sms: 132,
            mem_bytes: 80 << 30,
            precisions: HOPPER_PLUS_PRECISIONS,
        }
    }

    /// **B200** — Blackwell, sm_100, 148 SMs, ~228 KiB opt-in shared memory, 192 GB, fp8 (+ NVFP4 in Phase 5). The
    /// SOL-ExecBench target.
    pub fn b200() -> Self {
        CudaChip {
            name: "b200",
            family: GpuFamily::Blackwell,
            sm_arch: 100,
            smem_kib_per_block: 228,
            max_threads_per_block: 1024,
            sms: 148,
            mem_bytes: 192 << 30,
            precisions: HOPPER_PLUS_PRECISIONS,
        }
    }

    /// The `nvcc` arch flag that targets this part. Hopper/Blackwell get the architecture-specific `a` suffix
    /// (required to unlock wgmma/TMA on Hopper and tcgen05 on Blackwell); Ampere/Ada use the plain form.
    pub fn nvcc_arch_flag(&self) -> String {
        let suffix = match self.family {
            GpuFamily::Hopper | GpuFamily::Blackwell => "a",
            GpuFamily::Ampere | GpuFamily::Ada => "",
        };
        format!("-arch=sm_{}{}", self.sm_arch, suffix)
    }

    /// Whether this part's tensor / CUDA path realizes a numeric format.
    pub fn supports(&self, p: Precision) -> bool {
        self.precisions.contains(&p)
    }

    /// Can a kernel built for THIS chip be *numerically* executed on a host GPU of SM arch `host_sm`? Kernels
    /// are forward-compatible: a lower-arch kernel runs on a higher-arch host, never the reverse. This routes
    /// the run-vs-compile-check split — everything sm_86-legal is proven on the local 3060; a B200 kernel can
    /// only be compile-checked locally and is numerically validated on the cloud B200.
    pub fn runs_numerically_on(&self, host_sm: u32) -> bool {
        self.sm_arch <= host_sm
    }
}
