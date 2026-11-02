//! `Roofs` — the rates a chip sustains, each one cited: the roofline a program's work is held against.
//!
//! A [`SpatialTarget`](super::target::SpatialTarget) says what a chip HOLDS: sites, local memory, the
//! accumulator. This says how fast it MOVES and COMPUTES: bytes per second off the chip, bytes per cycle
//! across one network link, operations per cycle of a site's matrix engine. A program's work over a rate is
//! a lower bound on its time; the largest of those bounds is the roof that binds it; and a measured or
//! predicted time below the bound is a wrong byte count or a wrong roof, never a fast program (Williams,
//! Waterman & Patterson, *Roofline: an insightful visual performance model for multicore architectures*,
//! CACM 52(4), 2009).
//!
//! Every rate is a [`Fact`] in the unit its source states it in. A vendor's GB/s stays per second, a link's
//! width stays per cycle, and [`Roofs::per_cycle`] / [`Roofs::per_second`] convert through the clock with
//! the weaker of the two provenances, so a published bandwidth never borrows a modeled clock's authority. A
//! rate the record does not give is absent rather than guessed: [`Roofs::mem`] and [`Roofs::compute`]
//! answer `None`, and nothing downstream may price a resource without a roof.
//!
//! An operation is the vendors' unit: a multiply-add counts two.

use serde::Serialize;

use super::rdu_chip::{Fact, Provenance};
use crate::codegen_sva::fabric_target::Precision;

/// Where bytes move.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum MemLevel {
    /// The off-chip memory every site shares: DRAM, GDDR or HBM.
    Dram,
    /// One link of the on-chip network between sites.
    Noc,
    /// A site's own memory: L1, shared memory, a scratchpad.
    Local,
}

/// The access mix a bandwidth holds for — the four STREAM kernels (McCalpin, *Memory Bandwidth and Machine
/// Balance in Current High Performance Computers*, IEEE TCCA Newsletter, 1995) — or `Any`: a peak that
/// bounds every mix, the way a bus's data rate bounds whatever crosses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum StreamMix {
    Any,
    Read,
    Write,
    Copy,
    Triad,
}

/// What one rate is counted over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum Scope {
    /// The whole chip, shared by every site.
    Chip,
    /// Each site, independently of the others.
    Site,
    /// Each link of a network, independently of the others.
    Link,
}

/// The engine an operation rate belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum Engine {
    /// The tile / tensor-core / systolic matrix engine.
    Matrix,
    /// The SIMD or elementwise datapath.
    Vector,
}

/// What a memory rate counts. A bus moves bytes whatever they hold; an element-serial lane carries one
/// element per cycle whatever its width, so its traffic is counted in elements.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, serde::Deserialize,
)]
pub enum Transfer {
    Bytes,
    Elements,
}

/// A rate in the unit its source states it in.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum Rate {
    PerSecond(f64),
    PerCycle(f64),
}

/// A bandwidth at one level of the hierarchy, for one access mix, over one scope, in the unit the transfer
/// counts.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct MemRoof {
    pub level: MemLevel,
    pub mix: StreamMix,
    pub scope: Scope,
    pub unit: Transfer,
    pub rate: Fact<Rate>,
}

/// A throughput: operations of one engine at one precision, over one scope. For an engine whose math
/// fidelity is a number of passes over the operands, this is the rate of ONE pass: the work model already
/// counts each pass as work ([`super::work::fidelity_passes`]), so a roof that divided by them again would
/// charge the fidelity twice.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ComputeRoof {
    pub engine: Engine,
    pub precision: Precision,
    pub scope: Scope,
    pub ops: Fact<Rate>,
}

/// A rate converted to the unit asked for, with the provenance of the weaker of the rate and the clock.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Converted {
    pub value: f64,
    pub provenance: Provenance,
}

/// A chip's roofs.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Roofs {
    /// The clock the per-cycle rates run at, in GHz.
    pub clock_ghz: Fact<f64>,
    pub mem: Vec<MemRoof>,
    pub compute: Vec<ComputeRoof>,
    /// The documents every published or derived roof is read from; each such cite names one of them.
    pub sources: &'static [&'static str],
}

impl Roofs {
    /// The bandwidth of `level` for `mix`: a roof measured for that very mix when there is one, else the peak
    /// that bounds every mix.
    pub fn mem(&self, level: MemLevel, mix: StreamMix) -> Option<&MemRoof> {
        let at = |m: StreamMix| self.mem.iter().find(|r| r.level == level && r.mix == m);
        at(mix).or_else(|| at(StreamMix::Any))
    }

    /// The throughput of `engine` at `precision`.
    pub fn compute(&self, engine: Engine, precision: Precision) -> Option<&ComputeRoof> {
        self.compute
            .iter()
            .find(|r| r.engine == engine && r.precision == precision)
    }

    /// `rate` per cycle of this chip's clock.
    pub fn per_cycle(&self, rate: &Fact<Rate>) -> Converted {
        match rate.value {
            Rate::PerCycle(v) => Converted {
                value: v,
                provenance: rate.provenance,
            },
            Rate::PerSecond(v) => Converted {
                value: v / (self.clock_ghz.value * 1e9),
                provenance: rate.provenance.max(self.clock_ghz.provenance),
            },
        }
    }

    /// `rate` per second at this chip's clock.
    pub fn per_second(&self, rate: &Fact<Rate>) -> Converted {
        match rate.value {
            Rate::PerSecond(v) => Converted {
                value: v,
                provenance: rate.provenance,
            },
            Rate::PerCycle(v) => Converted {
                value: v * self.clock_ghz.value * 1e9,
                provenance: rate.provenance.max(self.clock_ghz.provenance),
            },
        }
    }

    /// Every fact the roofs carry, the clock first, each with the path a lock names it by.
    pub fn facts(&self) -> Vec<(String, Provenance, &'static str)> {
        let mut out = vec![(
            "clock_ghz".to_string(),
            self.clock_ghz.provenance,
            self.clock_ghz.cite,
        )];
        for r in &self.mem {
            out.push((
                format!("mem.{:?}.{:?}.{:?}", r.level, r.mix, r.scope),
                r.rate.provenance,
                r.rate.cite,
            ));
        }
        for r in &self.compute {
            out.push((
                format!("compute.{:?}.{:?}.{:?}", r.engine, r.precision, r.scope),
                r.ops.provenance,
                r.ops.cite,
            ));
        }
        out
    }
}

// ── The presets ──────────────────────────────────────────────────────────────────────────────────────────

const TT_WORMHOLE_SPECS: &str =
    "docs.tenstorrent.com/docs-test/core/latest/aibs/wormhole/specifications.html";
const TT_BLACKHOLE_SPECS: &str = "docs.tenstorrent.com/aibs/blackhole/index.html";
const TT_GEMM_FLOPS: &str =
    "github.com/tenstorrent/tt-metal/blob/main/tech_reports/GEMM_FLOPS/GEMM_FLOPS.md";
const TT_WORMHOLE_NOC: &str =
    "github.com/tenstorrent/tt-isa-documentation/blob/main/WormholeB0/NoC/README.md";
const TT_BLACKHOLE_NOC: &str =
    "github.com/tenstorrent/tt-isa-documentation/blob/main/BlackholeA0/NoC/README.md";
const NV_RTX3060_SPECS: &str = "nvidia.com/en-us/geforce/graphics-cards/30-series/rtx-3060-3060ti";
const NV_GA102_WHITEPAPER: &str =
    "nvidia.com/content/PDF/nvidia-ampere-ga-102-gpu-architecture-whitepaper-v2.pdf";
pub(crate) const NV_H100_PAGE: &str = "nvidia.com/en-us/data-center/h100";
pub(crate) const NV_H100_NVL_BRIEF: &str =
    "nvidia.com/content/dam/en-zz/Solutions/Data-Center/h100/PB-11773-001_v01.pdf";
pub(crate) const NV_A100_PAGE: &str = "nvidia.com/en-us/data-center/a100";
pub(crate) const NV_DGX_B200_PAGE: &str = "nvidia.com/en-us/data-center/dgx-b200";
pub(crate) const NV_HGX_PAGE: &str = "nvidia.com/en-us/data-center/hgx";
const FABRIC_TWIN: &str = "spatial::lower::fabric::fabric_cycle_model, cycle-exact against the RTL by spatial_fabric_lowering_lock";
const SKY130_SYSTOLIC_SIGNOFF: &str = "benchmarks/silicon/systolic_pipelined_signoff.json";
const DIGILENT_NEXYS_CLOCK: &str = "digilent.com/reference/programmable-logic/nexys-a7/reference-manual and \
    digilent.com/reference/programmable-logic/nexys-video/reference-manual: a 100 MHz crystal oscillator on the FPGA's \
    clock-capable input";

/// The Tensix matrix engine: one 8x16 by 16x16 product per cycle per pass, the same engine on both parts.
const TENSIX_MATRIX: &str = "github.com/tenstorrent/tt-metal/blob/main/tech_reports/GEMM_FLOPS/GEMM_FLOPS.md: \
    \"The matrix engine performs 8x16 x 16x16 = 8x16 in a single cycle. This is 2*8*16*16 = 4096 multiply-adds in a \
    single cycle\" (4096 operations: 2048 multiply-adds); \"LoFi ~4 TFLOPS, HiFi2 ~2 TFLOPS, HiFi4 ~1 TFLOPS\" per engine \
    at 1 GHz, so one pass per cycle";

/// Matrix roofs from a vendor table that states rates "with sparsity": the dense rate is half of each.
fn dense_from_sparse(rows: &[(Precision, f64, &'static str)]) -> Vec<ComputeRoof> {
    rows.iter()
        .map(|&(precision, sparse, cite)| ComputeRoof {
            engine: Engine::Matrix,
            precision,
            scope: Scope::Chip,
            ops: Fact::derived(Rate::PerSecond(sparse / 2.0), cite),
        })
        .collect()
}

fn tensix_matrix(precisions: &[Precision]) -> Vec<ComputeRoof> {
    precisions
        .iter()
        .map(|&precision| ComputeRoof {
            engine: Engine::Matrix,
            precision,
            scope: Scope::Site,
            ops: Fact::published(Rate::PerCycle(4096.0), TENSIX_MATRIX),
        })
        .collect()
}

impl Roofs {
    /// Tenstorrent Wormhole n150.
    pub fn wormhole() -> Roofs {
        Roofs {
            clock_ghz: Fact::published(1.0, "docs.tenstorrent.com/docs-test/core/latest/aibs/wormhole/specifications.html: n150s \"AI Clock: 1 GHz\""),
            mem: vec![
                MemRoof {
                    level: MemLevel::Dram,
                    mix: StreamMix::Any,
                    scope: Scope::Chip,
                    unit: Transfer::Bytes,
                    rate: Fact::published(
                        Rate::PerSecond(288e9),
                        "docs.tenstorrent.com/docs-test/core/latest/aibs/wormhole/specifications.html: n150s \"Memory: 12GB GDDR6\", \
                         \"Memory Speed: 12 GT/sec\", \"Memory Bandwidth: 288 GB/sec\"",
                    ),
                },
                MemRoof {
                    level: MemLevel::Noc,
                    mix: StreamMix::Any,
                    scope: Scope::Link,
                    unit: Transfer::Bytes,
                    rate: Fact::published(
                        Rate::PerCycle(32.0),
                        "github.com/tenstorrent/tt-isa-documentation/blob/main/WormholeB0/NoC/README.md: \"Each flit consists of exactly \
                         256 bits (32 bytes)\", \"One flit (256 bits) per cycle\"",
                    ),
                },
            ],
            compute: tensix_matrix(&[Precision::Bf16, Precision::Fp16]),
            sources: &[TT_WORMHOLE_SPECS, TT_GEMM_FLOPS, TT_WORMHOLE_NOC],
        }
    }

    /// Tenstorrent Blackhole p150.
    pub fn blackhole() -> Roofs {
        Roofs {
            clock_ghz: Fact::published(1.35, "docs.tenstorrent.com/aibs/blackhole/index.html: p150a/p150b \"AI Clock: Up to 1.35 GHz\""),
            mem: vec![
                MemRoof {
                    level: MemLevel::Dram,
                    mix: StreamMix::Any,
                    scope: Scope::Chip,
                    unit: Transfer::Bytes,
                    rate: Fact::published(
                        Rate::PerSecond(512e9),
                        "docs.tenstorrent.com/aibs/blackhole/index.html: p150a/p150b \"32 GB GDDR6\", \"Memory Speed: 16 GT/sec\", \
                         \"Memory Bandwidth: 512 GB/sec\"",
                    ),
                },
                MemRoof {
                    level: MemLevel::Noc,
                    mix: StreamMix::Any,
                    scope: Scope::Link,
                    unit: Transfer::Bytes,
                    rate: Fact::published(
                        Rate::PerCycle(64.0),
                        "github.com/tenstorrent/tt-isa-documentation/blob/main/BlackholeA0/NoC/README.md: \"Each flit consists of exactly \
                         512 bits (64 bytes)\", \"One flit (512 bits) per cycle per axis\"",
                    ),
                },
            ],
            compute: tensix_matrix(&[Precision::Bf16, Precision::Fp16]),
            sources: &[TT_BLACKHOLE_SPECS, TT_GEMM_FLOPS, TT_BLACKHOLE_NOC],
        }
    }

    /// The local RTX 3060. NVIDIA's table gives the SM count (through the CUDA cores) and the boost clock but no
    /// tensor rate and no memory data rate, so the tensor rates are derived from the GA10x SM the Ampere
    /// whitepaper tabulates, and the DRAM rate is modeled until the box measures it.
    pub fn rtx3060() -> Roofs {
        Roofs {
            clock_ghz: Fact::published(1.78, "nvidia.com/en-us/geforce/graphics-cards/30-series/rtx-3060-3060ti: RTX 3060 \"Boost Clock: 1.78 GHz\""),
            mem: vec![
                MemRoof {
                    level: MemLevel::Dram,
                    mix: StreamMix::Any,
                    scope: Scope::Chip,
                    unit: Transfer::Bytes,
                    rate: Fact::modeled(
                        Rate::PerSecond(360e9),
                        "not published by NVIDIA: its RTX 3060 table gives \"12 GB GDDR6\" on a \"192-bit\" interface and no data \
                         rate; the board partners' 15 Gbps GDDR6 gives 192 x 15 / 8 = 360 GB/s",
                    ),
                },
                MemRoof {
                    level: MemLevel::Local,
                    mix: StreamMix::Any,
                    scope: Scope::Chip,
                    unit: Transfer::Bytes,
                    rate: Fact::derived(
                        Rate::PerCycle(28.0 * 128.0),
                        "nvidia.com/content/PDF/nvidia-ampere-ga-102-gpu-architecture-whitepaper-v2.pdf: GA10x shared memory \
                         \"128 bytes/clock per SM\"; nvidia.com/en-us/geforce/graphics-cards/30-series/rtx-3060-3060ti: \"CUDA Cores: \
                         3584\" at the whitepaper's \"CUDA Cores / SM 128\" = 28 SMs; 28 x 128 B per cycle",
                    ),
                },
            ],
            compute: [Precision::Bf16, Precision::Fp16]
                .into_iter()
                .map(|precision| ComputeRoof {
                    engine: Engine::Matrix,
                    precision,
                    scope: Scope::Chip,
                    ops: Fact::derived(
                        Rate::PerCycle(28.0 * 512.0),
                        "nvidia.com/content/PDF/nvidia-ampere-ga-102-gpu-architecture-whitepaper-v2.pdf: GA10x \"Total FP16 FMA \
                         operations per SM Dense: 512\", 1024 operations per SM per cycle with FP16 accumulate and half that with FP32 \
                         accumulate (RTX 3080: \"Peak FP16 Tensor TFLOPS with FP16 Accumulate 119\", \"with FP32 Accumulate 59.5\"); \
                         nvidia.com/en-us/geforce/graphics-cards/30-series/rtx-3060-3060ti: \"CUDA Cores: 3584\" = 28 SMs at the \
                         whitepaper's \"CUDA Cores / SM 128\"; 28 x 512 at FP32 accumulate",
                    ),
                })
                .chain(std::iter::once(ComputeRoof {
                    engine: Engine::Matrix,
                    precision: Precision::Int8,
                    scope: Scope::Chip,
                    ops: Fact::derived(
                        Rate::PerCycle(28.0 * 2048.0),
                        "nvidia.com/content/PDF/nvidia-ampere-ga-102-gpu-architecture-whitepaper-v2.pdf: INT8 tensor at twice the \
                         FP16-accumulate rate (RTX 3080: \"Peak INT8 Tensor TOPS 238\" against FP16 119, 68 SMs at 1710 MHz = 2048 \
                         per SM per cycle); nvidia.com/en-us/geforce/graphics-cards/30-series/rtx-3060-3060ti: \"CUDA Cores: 3584\" = \
                         28 SMs; 28 x 2048",
                    ),
                }))
                .collect(),
            sources: &[NV_RTX3060_SPECS, NV_GA102_WHITEPAPER],
        }
    }

    /// NVIDIA H100 SXM. The product table gives memory bandwidth and tensor rates "With sparsity"; a dense rate
    /// is half of it, which is what a dense matrix-vector product can use. The table gives no clock, and every
    /// rate here is per second, so nothing is converted through the one recorded.
    pub fn h100_sxm() -> Roofs {
        Roofs {
            clock_ghz: Fact::modeled(
                1.98,
                "not published on NVIDIA's H100 product page; inherited from the third-party spec databases' 1980 MHz \
                 boost — every rate below is per second, so no rate is converted through it",
            ),
            mem: vec![MemRoof {
                level: MemLevel::Dram,
                mix: StreamMix::Any,
                scope: Scope::Chip,
                unit: Transfer::Bytes,
                rate: Fact::published(
                    Rate::PerSecond(3.35e12),
                    "nvidia.com/en-us/data-center/h100: H100 SXM \"GPU Memory Bandwidth 3.35TB/s\"",
                ),
            }],
            compute: dense_from_sparse(&[
                (Precision::Int8, 3958e12, "nvidia.com/en-us/data-center/h100: H100 SXM \"INT8 Tensor Core* 3,958 TOPS\", \"* With sparsity\"; dense is half"),
                (Precision::Fp8, 3958e12, "nvidia.com/en-us/data-center/h100: H100 SXM \"FP8 Tensor Core* 3,958 teraFLOPS\", \"* With sparsity\"; dense is half"),
                (Precision::Bf16, 1979e12, "nvidia.com/en-us/data-center/h100: H100 SXM \"BFLOAT16 Tensor Core* 1,979 teraFLOPS\", \"* With sparsity\"; dense is half"),
                (Precision::Fp16, 1979e12, "nvidia.com/en-us/data-center/h100: H100 SXM \"FP16 Tensor Core* 1,979 teraFLOPS\", \"* With sparsity\"; dense is half"),
            ]),
            sources: &[NV_H100_PAGE],
        }
    }

    /// NVIDIA H100 NVL: the PCIe card the Qwen3.8-27B decode runs were measured on. Its product brief gives the
    /// clock and the exact memory bandwidth; the tensor rates are the product table's, halved from sparse.
    pub fn h100_nvl() -> Roofs {
        Roofs {
            clock_ghz: Fact::published(
                1.785,
                "nvidia.com/content/dam/en-zz/Solutions/Data-Center/h100/PB-11773-001_v01.pdf: Table 1 \"GPU clocks Base: \
                 1,080 MHz Boost: 1,785 MHz\"",
            ),
            mem: vec![MemRoof {
                level: MemLevel::Dram,
                mix: StreamMix::Any,
                scope: Scope::Chip,
                unit: Transfer::Bytes,
                rate: Fact::published(
                    Rate::PerSecond(3938e9),
                    "nvidia.com/content/dam/en-zz/Solutions/Data-Center/h100/PB-11773-001_v01.pdf: Table 2 \"Peak memory \
                     bandwidth 3,938 GB/s\" (the product page rounds it to \"3.9TB/s\")",
                ),
            }],
            compute: dense_from_sparse(&[
                (Precision::Int8, 3341e12, "nvidia.com/en-us/data-center/h100: H100 NVL \"INT8 Tensor Core* 3,341 TOPS\", \"* With sparsity\"; dense is half"),
                (Precision::Fp8, 3341e12, "nvidia.com/en-us/data-center/h100: H100 NVL \"FP8 Tensor Core* 3,341 teraFLOPS\", \"* With sparsity\"; dense is half"),
                (Precision::Bf16, 1671e12, "nvidia.com/en-us/data-center/h100: H100 NVL \"BFLOAT16 Tensor Core* 1,671 teraFLOPS\", \"* With sparsity\"; dense is half"),
                (Precision::Fp16, 1671e12, "nvidia.com/en-us/data-center/h100: H100 NVL \"FP16 Tensor Core* 1,671 teraFLOPS\", \"* With sparsity\"; dense is half"),
            ]),
            sources: &[NV_H100_PAGE, NV_H100_NVL_BRIEF],
        }
    }

    /// NVIDIA A100 80GB SXM. Its product table states the dense rate first and the sparse rate after it.
    pub fn a100_sxm4_80gb() -> Roofs {
        Roofs {
            clock_ghz: Fact::modeled(
                1.41,
                "not published on NVIDIA's A100 product page; inherited from the A100 whitepaper's 1410 MHz boost — every \
                 rate below is per second, so no rate is converted through it",
            ),
            mem: vec![MemRoof {
                level: MemLevel::Dram,
                mix: StreamMix::Any,
                scope: Scope::Chip,
                unit: Transfer::Bytes,
                rate: Fact::published(
                    Rate::PerSecond(2039e9),
                    "nvidia.com/en-us/data-center/a100: A100 80GB SXM \"GPU Memory Bandwidth 2,039 GB/s\"",
                ),
            }],
            compute: [
                (Precision::Int8, 624e12, "nvidia.com/en-us/data-center/a100: A100 80GB SXM \"INT8 Tensor Core 624 TOPS | 1248 TOPS*\", \"* With sparsity\""),
                (Precision::Bf16, 312e12, "nvidia.com/en-us/data-center/a100: A100 80GB SXM \"BFLOAT16 Tensor Core 312 TFLOPS | 624 TFLOPS*\", \"* With sparsity\""),
                (Precision::Fp16, 312e12, "nvidia.com/en-us/data-center/a100: A100 80GB SXM \"FP16 Tensor Core 312 TFLOPS | 624 TFLOPS*\", \"* With sparsity\""),
            ]
            .into_iter()
            .map(|(precision, ops, cite)| ComputeRoof {
                engine: Engine::Matrix,
                precision,
                scope: Scope::Chip,
                ops: Fact::published(Rate::PerSecond(ops), cite),
            })
            .collect(),
            sources: &[NV_A100_PAGE],
        }
    }

    /// One GPU of the NVIDIA DGX/HGX B200. NVIDIA tabulates the 8-GPU system, so each rate is the system's over
    /// eight, and each tensor rate the sparse figure halved per the table's own footnote.
    pub fn b200() -> Roofs {
        Roofs {
            clock_ghz: Fact::modeled(
                1.965,
                "not published on NVIDIA's DGX B200 or HGX pages; inherited from third-party spec databases — every rate \
                 below is per second, so no rate is converted through it",
            ),
            mem: vec![MemRoof {
                level: MemLevel::Dram,
                mix: StreamMix::Any,
                scope: Scope::Chip,
                unit: Transfer::Bytes,
                rate: Fact::derived(
                    Rate::PerSecond(64e12 / 8.0),
                    "nvidia.com/en-us/data-center/dgx-b200: \"8x NVIDIA Blackwell GPUs\", \"1,440 GB total, 64 TB/s HBM3e \
                     bandwidth\"; 64 TB/s over 8 GPUs",
                ),
            }],
            compute: [
                (Precision::Int8, 72e15, "nvidia.com/en-us/data-center/hgx: HGX B200 \"8x NVIDIA Blackwell SXM\", \"INT8 Tensor Core 72 POPS\", footnote \"Specification in Sparse. Dense is ½ sparse spec shown.\"; halved, over 8 GPUs"),
                (Precision::Fp8, 72e15, "nvidia.com/en-us/data-center/hgx: HGX B200 \"8x NVIDIA Blackwell SXM\", \"FP8/FP6 Tensor Core 72 PFLOPS\", footnote \"Specification in Sparse. Dense is ½ sparse spec shown.\"; halved, over 8 GPUs"),
                (Precision::Bf16, 36e15, "nvidia.com/en-us/data-center/hgx: HGX B200 \"8x NVIDIA Blackwell SXM\", \"FP16/BF16 Tensor Core 36 PFLOPS\", footnote \"Specification in Sparse. Dense is ½ sparse spec shown.\"; halved, over 8 GPUs"),
                (Precision::Fp16, 36e15, "nvidia.com/en-us/data-center/hgx: HGX B200 \"8x NVIDIA Blackwell SXM\", \"FP16/BF16 Tensor Core 36 PFLOPS\", footnote \"Specification in Sparse. Dense is ½ sparse spec shown.\"; halved, over 8 GPUs"),
            ]
            .into_iter()
            .map(|(precision, sparse_system, cite)| ComputeRoof {
                engine: Engine::Matrix,
                precision,
                scope: Scope::Chip,
                ops: Fact::derived(Rate::PerSecond(sparse_system / 2.0 / 8.0), cite),
            })
            .collect(),
            sources: &[NV_DGX_B200_PAGE, NV_HGX_PAGE],
        }
    }

    /// Every roofs preset, by name: the chips a decode is priced on and the spatial targets' own.
    pub fn catalog() -> Vec<(&'static str, Roofs)> {
        vec![
            ("wormhole", Roofs::wormhole()),
            ("blackhole", Roofs::blackhole()),
            ("rtx3060", Roofs::rtx3060()),
            ("sky130-fabric", Roofs::sky_fabric()),
            ("a100-sxm4-80gb", Roofs::a100_sxm4_80gb()),
            ("h100-sxm", Roofs::h100_sxm()),
            ("h100-nvl", Roofs::h100_nvl()),
            ("b200", Roofs::b200()),
        ]
    }

    /// Our own fabric. Its matrix engine is not a fixed part: each program instantiates a `k` x `n` systolic
    /// array sized by its plan's block, and the cycle twin times that array exactly, so the fabric carries no
    /// matrix roof. What every program on it shares is the address lane, which carries one element per cycle
    /// whatever its width. The clock is the largest signed-off array's.
    pub fn sky_fabric() -> Roofs {
        Roofs {
            clock_ghz: Fact::modeled(
                1.0 / 19.5,
                "inherited: the largest signed-off systolic array, systolic_pipe16_realbist, closes at clock_period_ns 19.5 with \
                 setup_worst_slack_ns 0.25 under OpenLane Sky130 STA (benchmarks/silicon/systolic_pipelined_signoff.json); a \
                 program's own array is not signed off",
            ),
            mem: vec![MemRoof {
                level: MemLevel::Dram,
                mix: StreamMix::Any,
                scope: Scope::Site,
                unit: Transfer::Elements,
                rate: Fact::derived(
                    Rate::PerCycle(1.0),
                    "spatial::lower::fabric::fabric_cycle_model, cycle-exact against the RTL by spatial_fabric_lowering_lock: \
                     ReadTiles and WriteTiles take one cycle per element whatever its width",
                ),
            }],
            compute: Vec::new(),
            sources: &[FABRIC_TWIN, SKY130_SYSTOLIC_SIGNOFF],
        }
    }

    /// The fabric on a 7-series FPGA: the same element-serial lanes as [`sky_fabric`](Self::sky_fabric), clocked from
    /// the board.
    pub fn xc7_fabric() -> Roofs {
        Roofs {
            clock_ghz: Fact::modeled(
                0.1,
                "the Nexys boards' 100 MHz system oscillator (Digilent Nexys A7 and Nexys Video reference manuals), the \
                 clock an image divides down from; a program's routed Fmax is recorded beside its bitstream",
            ),
            sources: &[FABRIC_TWIN, DIGILENT_NEXYS_CLOCK],
            ..Roofs::sky_fabric()
        }
    }
}

// ── A program's roofline ─────────────────────────────────────────────────────────────────────────────────

/// A resource a program's work is bounded by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum Resource {
    /// The memory every site shares, over the program's whole DRAM traffic.
    Dram,
    /// One network link, over the busiest movement engine's traffic: an engine injects at most one flit
    /// per cycle into the network it drives.
    NocLink,
    /// The matrix engine, over the busiest site's arithmetic for a per-site rate, the chip's for a
    /// chip-wide one.
    Matrix,
}

/// One resource's lower bound on a program's time.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Bound {
    pub resource: Resource,
    /// The work the rate divides: bytes for a memory or a link, operations for an engine.
    pub work: f64,
    /// The rate, per cycle of the target's clock.
    pub per_cycle: f64,
    /// `work / per_cycle`.
    pub cycles: f64,
    /// The weaker of the roof's provenance and the clock's, when the roof was stated per second.
    pub provenance: Provenance,
}

/// A program's roofline on a target: one lower bound per resource the target has a roof for, the largest
/// of which binds. A resource with no roof has no bound; nothing is priced at a guessed rate.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Roofline {
    pub bounds: Vec<Bound>,
    /// The matrix operations the program performs — its arithmetic, `2 · M · N · K` for a GEMM, however many
    /// fidelity passes the engine makes over it.
    pub ops: f64,
    /// The bytes it moves to and from the memory every site shares.
    pub dram_bytes: f64,
}

impl Roofline {
    /// The bound that binds: the resource the program waits on longest.
    pub fn binding(&self) -> Option<&Bound> {
        self.bounds
            .iter()
            .max_by(|a, b| a.cycles.total_cmp(&b.cycles))
    }

    /// No schedule of this work finishes in fewer cycles.
    pub fn cycles(&self) -> f64 {
        self.binding().map_or(0.0, |b| b.cycles)
    }

    /// Operations per DRAM byte, the arithmetic intensity: where the program sits on the roofline's horizontal
    /// axis.
    pub fn intensity(&self) -> Option<f64> {
        (self.dram_bytes > 0.0).then(|| self.ops / self.dram_bytes)
    }
}

/// One resource's rate per cycle, with what vouches for it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct PerCycle {
    pub value: f64,
    pub provenance: Provenance,
}

/// The rate a matrix engine sustains, and what one issue the work model counts is worth against it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct MatrixRate {
    pub per_cycle: PerCycle,
    /// The rate holds for the whole chip, so it bounds the whole program's arithmetic; otherwise each site's.
    pub chip_wide: bool,
    /// Operations one counted issue performs, one fidelity pass each — what the per-pass rate divides.
    pub ops_per_issue: f64,
    /// Operations of the program's arithmetic one counted issue stands for, whatever the fidelity.
    pub arithmetic_per_issue: f64,
}

/// A memory's rate per cycle and what it counts.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct LaneRate {
    pub per_cycle: PerCycle,
    pub unit: Transfer,
}

/// The per-cycle rates a target's roofs give for a program whose operands are one element type: what a
/// roofline divides the work by. A resource without a roof has no rate.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct Rates {
    pub dram: Option<LaneRate>,
    pub link: Option<LaneRate>,
    pub matrix: Option<MatrixRate>,
    /// Operations of the program's arithmetic one counted issue stands for, priced or not.
    pub arithmetic_per_issue: f64,
    /// Elements in one of the target's tiles: what a tile moved is worth to a lane counted in elements.
    pub elements_per_tile: f64,
}

impl Rates {
    /// The rates of `t`'s roofs for a program whose operands are `element`s.
    ///
    /// The work model counts arithmetic as `MVMUL_PER_TILE_MATMUL_PER_PASS` issues per tile-matmul pass; one
    /// pass of a tile-matmul is `2 · rows · cols · cols` operations on the target's tile.
    pub fn of(t: &super::target::SpatialTarget, element: Precision) -> Rates {
        let r = &t.roofs;
        let per = |rate: &Fact<Rate>| {
            let c = r.per_cycle(rate);
            PerCycle {
                value: c.value,
                provenance: c.provenance,
            }
        };
        let ops_per_pass = 2.0 * (t.tile.rows * t.tile.cols * t.tile.cols) as f64;
        let per_tile = super::work::mvmul_per_tile_matmul(
            t.matmul_fidelity()
                .unwrap_or(super::target::Fidelity::HiFi4),
        );
        let arithmetic_per_issue = ops_per_pass / per_tile as f64;
        let lane = |m: &MemRoof| LaneRate {
            per_cycle: per(&m.rate),
            unit: m.unit,
        };
        Rates {
            dram: r.mem(MemLevel::Dram, StreamMix::Any).map(lane),
            link: r.mem(MemLevel::Noc, StreamMix::Any).map(lane),
            matrix: r.compute(Engine::Matrix, element).map(|c| MatrixRate {
                per_cycle: per(&c.ops),
                chip_wide: c.scope == Scope::Chip,
                ops_per_issue: ops_per_pass / super::work::MVMUL_PER_TILE_MATMUL_PER_PASS as f64,
                arithmetic_per_issue,
            }),
            arithmetic_per_issue,
            elements_per_tile: t.tile.elems() as f64,
        }
    }

    /// The roofline of `w` at these rates.
    pub fn roofline(&self, w: &super::work::Work) -> Roofline {
        let mut bounds = Vec::new();
        let mut bound = |resource: Resource, work: f64, rate: PerCycle| {
            bounds.push(Bound {
                resource,
                work,
                per_cycle: rate.value,
                cycles: work / rate.value,
                provenance: rate.provenance,
            });
        };
        if let Some(lane) = self.dram {
            let work = match lane.unit {
                Transfer::Bytes => w.dram_bytes as f64,
                Transfer::Elements => w.dram_tiles as f64 * self.elements_per_tile,
            };
            bound(Resource::Dram, work, lane.per_cycle);
        }
        // The work model counts the busiest movement engine's traffic in bytes only, so a link counted in
        // elements is not priced.
        if let Some(lane) = self.link.filter(|l| l.unit == Transfer::Bytes) {
            bound(Resource::NocLink, w.engine_noc_bytes as f64, lane.per_cycle);
        }
        if let Some(m) = self.matrix {
            let issues = if m.chip_wide {
                w.matmul_retired
            } else {
                w.site_matmul_retired
            };
            bound(
                Resource::Matrix,
                issues as f64 * m.ops_per_issue,
                m.per_cycle,
            );
        }
        Roofline {
            bounds,
            ops: w.matmul_retired as f64 * self.arithmetic_per_issue,
            dram_bytes: w.dram_bytes as f64,
        }
    }
}

/// The roofline of `w` on `t`, for a program whose operands are `element`s.
pub fn roofline(
    w: &super::work::Work,
    t: &super::target::SpatialTarget,
    element: Precision,
) -> Roofline {
    Rates::of(t, element).roofline(w)
}
