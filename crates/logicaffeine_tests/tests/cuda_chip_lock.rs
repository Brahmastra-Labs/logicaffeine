//! cuda_chip_lock — the GPU **chip/family** is a first-class knob of the CUDA backend.
//!
//! The CUDA emitters are not hard-wired to one part. Which silicon we target — its SM architecture,
//! shared-memory budget, and the numeric formats its tensor path realizes — is a knob, exactly like the
//! Tenstorrent backend's [`ChipKnobs`] (wormhole / blackhole). This is what lets the SAME recognized LOGOS
//! tensor op lower to a kernel tuned for the local RTX 3060 (Ampere, sm_86) OR for a cloud B200 (Blackwell,
//! sm_100) — the target we submit to NVIDIA's SOL-ExecBench against.
//!
//! Why it is a genuine *trade-off* knob (and not a baked-in constant): choosing the chip trades **portability**
//! against **hardware exploitation**. A kernel compiled for `sm_100` won't run on an `sm_86` host; a kernel
//! compiled for `sm_86` leaves the B200's 5th-gen tensor cores and larger shared memory unused. That win/lose
//! structure is precisely what makes it a knob under the backend's design principle.
//!
//! This lock pins the contract:
//!   • every chip carries its real (family, sm_arch);
//!   • the nvcc arch flag targets the chip (Hopper/Blackwell get the arch-specific `a` suffix);
//!   • which numeric precisions are legal is CHIP-DRIVEN (Ampere: bf16/fp16/fp32; Blackwell adds fp8);
//!   • a kernel for a newer arch than the host is compile-check-only locally (drives run-vs-compile gating);
//!   • the default chip is the local dev GPU;
//!   • `chip` is a real knob with a non-empty win/lose trade-off row.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_chip_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::{CudaChip, CudaKnobs, GpuFamily};
use logicaffeine_compile::codegen_sva::fabric_target::Precision;

#[test]
fn every_chip_carries_its_real_family_and_arch() {
    // Real parts, real SM architectures — the descriptor is a fact about the silicon, not a placeholder.
    assert_eq!(
        (CudaChip::rtx3060().family, CudaChip::rtx3060().sm_arch),
        (GpuFamily::Ampere, 86)
    );
    assert_eq!(
        (CudaChip::a100().family, CudaChip::a100().sm_arch),
        (GpuFamily::Ampere, 80)
    );
    assert_eq!(
        (CudaChip::h100().family, CudaChip::h100().sm_arch),
        (GpuFamily::Hopper, 90)
    );
    assert_eq!(
        (CudaChip::b200().family, CudaChip::b200().sm_arch),
        (GpuFamily::Blackwell, 100)
    );
    // Each part's device memory, as its datasheet states it: the 3060's 12 GiB, the 80 GB A100 and H100 SXM, the
    // B200's 192 GB.
    assert_eq!(
        [CudaChip::rtx3060(), CudaChip::a100(), CudaChip::h100(), CudaChip::b200()].map(|c| c.mem_bytes),
        [12 << 30, 80 << 30, 80 << 30, 192 << 30]
    );
    // Names are distinct and non-empty (they land in the emitted kernel's provenance header).
    for c in [
        CudaChip::rtx3060(),
        CudaChip::a100(),
        CudaChip::h100(),
        CudaChip::b200(),
    ] {
        assert!(
            !c.name.is_empty(),
            "a chip must name itself for the kernel header"
        );
    }
}

#[test]
fn the_nvcc_arch_flag_targets_the_chip() {
    // Ampere/Ada: plain `sm_XX`. Hopper/Blackwell: the arch-specific `a` suffix (required to unlock
    // wgmma/tma on Hopper and tcgen05 on Blackwell) — so the flag is a function of BOTH arch and family.
    assert_eq!(CudaChip::rtx3060().nvcc_arch_flag(), "-arch=sm_86");
    assert_eq!(CudaChip::a100().nvcc_arch_flag(), "-arch=sm_80");
    assert_eq!(CudaChip::h100().nvcc_arch_flag(), "-arch=sm_90a");
    assert_eq!(CudaChip::b200().nvcc_arch_flag(), "-arch=sm_100a");
}

#[test]
fn precision_legality_is_chip_driven() {
    // Ampere realizes bf16/fp16/fp32 in its tensor/CUDA cores, but NOT fp8. Blackwell adds fp8 (and, in a
    // later phase, nvfp4). Which precision the fused-row / GEMM emitter may pick is gated on THIS, per chip.
    let amp = CudaChip::rtx3060();
    assert!(
        amp.supports(Precision::Bf16)
            && amp.supports(Precision::Fp16)
            && amp.supports(Precision::Fp32)
    );
    assert!(
        !amp.supports(Precision::Fp8),
        "Ampere has no fp8 tensor path"
    );

    let bw = CudaChip::b200();
    assert!(
        bw.supports(Precision::Bf16)
            && bw.supports(Precision::Fp16)
            && bw.supports(Precision::Fp32)
    );
    assert!(bw.supports(Precision::Fp8), "Blackwell realizes fp8");
}

#[test]
fn a_newer_arch_kernel_is_compile_check_only_on_an_older_host() {
    // The run-vs-compile-check gate: a B200 kernel cannot be NUMERICALLY validated on an sm_86 host — it can
    // only be compiled. A 3060 kernel runs on the 3060. This is what routes fp8/nvfp4 numeric checks to the
    // cloud B200 while everything sm_86-legal is proven locally.
    assert!(
        CudaChip::rtx3060().runs_numerically_on(86),
        "an sm_86 kernel runs on an sm_86 host"
    );
    assert!(
        !CudaChip::b200().runs_numerically_on(86),
        "an sm_100 kernel cannot run on an sm_86 host"
    );
    assert!(
        CudaChip::b200().runs_numerically_on(100),
        "on a real B200 host it runs"
    );
    // Forward-compatible: a lower-arch kernel runs on a higher-arch host.
    assert!(
        CudaChip::rtx3060().runs_numerically_on(100),
        "sm_86 kernel runs on an sm_100 host"
    );
}

#[test]
fn the_default_chip_is_the_local_dev_gpu() {
    // We develop and prove correctness locally on the RTX 3060; targeting the B200 is an explicit knob flip.
    assert_eq!(CudaKnobs::default().chip, CudaChip::rtx3060());
}

#[test]
fn chip_is_a_real_knob_with_a_documented_tradeoff() {
    // It must appear in the trade-off table with a real win AND a real loss (portability vs exploitation) —
    // that's why it's a knob and not a constant. (cuda_knobs_lock enforces one row per CudaKnobs field.)
    let row = CudaKnobs::tradeoffs()
        .iter()
        .find(|t| t.knob == "chip")
        .expect("chip must be a documented knob");
    assert!(
        !row.wins.trim().is_empty(),
        "chip knob must state where targeting a newer chip WINS"
    );
    assert!(
        !row.loses.trim().is_empty(),
        "chip knob must state where it LOSES (portability)"
    );
}

#[test]
fn toggling_the_chip_knob_is_observable() {
    // Flipping the chip must change something the emitter consumes — here, the arch flag the build uses.
    let local = CudaKnobs::default();
    let cloud = CudaKnobs {
        chip: CudaChip::b200(),
        ..Default::default()
    };
    assert_ne!(
        local.chip.nvcc_arch_flag(),
        cloud.chip.nvcc_arch_flag(),
        "different chips ⇒ different build target"
    );
    assert_ne!(local.chip, cloud.chip);
}
