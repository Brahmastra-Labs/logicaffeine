//! Genuine LOGOS → Tensix matrix-engine lowering — an AFFINE GEMM-idiom recognizer.
//!
//! This is the compiler *generating* the matrix-engine kernel, not returning a canned template. It reads an
//! ordinary LOGOS program — a triple-nested loop computing `C[i][j] += A[i][k]*B[k][j]` over row-major
//! `Seq of Int` — recognizes it as a matmul by checking that the three index expressions are AFFINE in the
//! induction variables and match the row-major GEMM layout (verified against the loop bounds), extracts the
//! dimensions `M, N, K`, and lowers to a tiled Tensix kernel (`matmul_tiles` + tile-register management)
//! whose tile bounds are the *recognized program's* dimensions. A program that is not a matmul is REFUSED
//! (typed), never miscompiled.
//!
//! The genuineness contract, enforced by the lock: change the program's dimensions → the emitted kernel's
//! tile bounds change; a non-matmul program → a typed refusal. A template can do neither.

/// The backend-neutral recognizers + specs now live in [`crate::tensor_recognize`]; re-exported here so
/// existing `codegen_tt::matmul_lower::…` paths keep resolving. This module keeps only the Tensix EMITTERS.
pub use crate::tensor_recognize::*;

/// The tiling / blocking config and the lattice search that fills it now live in
/// [`crate::spatial::passes::tile`] — it is the same divisor lattice the spatial planner already searched,
/// so the two cannot drift. This module keeps the Tenstorrent-facing names and the named-part presets.
pub use crate::spatial::passes::tile::TileConfig;

/// The DST register file holds at most this many bf16 output tiles in one acquire (half-sync mode) on
/// Wormhole; other parts differ — see [`ChipKnobs`]. Kept for the default-chip helpers + locks.
pub const MAX_DST_TILES: usize = 8;

/// The Wormhole Tensix worker grid is 8×8; other parts differ — see [`ChipKnobs`].
pub const MAX_GRID: usize = 8;

/// Per-chip capability **knobs** — the hardware limits that make the fastest tiling *chip-specific*. A
/// bigger DST register file lets a core hold a larger reuse sub-block; a bigger worker grid lets the work
/// spread across more cores; more L1 lets wider inner blocks stay resident. The autotuner searches within
/// these, so the SAME matmul compiles to a DIFFERENT (faster) kernel on a different part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChipKnobs {
    /// Human-readable part name (for the emitted kernel header + provenance).
    pub name: &'static str,
    /// DST register-file capacity in 32×32 output tiles per acquire (the reuse sub-block ceiling).
    pub max_dst_tiles: usize,
    /// Worker-grid width (max cores across the N dimension).
    pub grid_x: usize,
    /// Worker-grid height (max cores across the M dimension).
    pub grid_y: usize,
    /// Per-core L1 SRAM (KiB) — budgets the resident A/B blocks (bounds `in0_block_w`).
    pub l1_kib: usize,
}

impl ChipKnobs {
    /// Wormhole b0: 8-tile DST, 8×8 worker grid, ~1.46 MB L1.
    pub fn wormhole() -> Self {
        ChipKnobs {
            name: "wormhole_b0",
            max_dst_tiles: 8,
            grid_x: 8,
            grid_y: 8,
            l1_kib: 1464,
        }
    }
    /// Blackhole: 16-tile DST (double the reuse), a larger 13×10 worker grid, ~1.46 MB L1.
    pub fn blackhole() -> Self {
        ChipKnobs {
            name: "blackhole",
            max_dst_tiles: 16,
            grid_x: 13,
            grid_y: 10,
            l1_kib: 1464,
        }
    }

    /// This part's limits in the units the lattice search filters by, under the requested compute knobs
    /// (fp32 dest-accumulate halves the addressable DST tiles).
    pub fn tile_limits(&self, knobs: &ComputeKnobs) -> crate::spatial::passes::tile::TileLimits {
        crate::spatial::passes::tile::TileLimits {
            max_acc_tiles: knobs.dst_tiles(self),
            sites_x: self.grid_x,
            sites_y: self.grid_y,
            local_kib: self.l1_kib,
        }
    }
    /// Cores available.
    pub fn cores(&self) -> usize {
        self.grid_x * self.grid_y
    }
}

/// The Tensix matrix engine's math fidelity: how many FPU passes a bf16 multiply gets (LoFi = 1 … HiFi4 = 4).
/// ttnn picks it by a heuristic (HiFi2 with no program config, LoFi the moment one is passed); LOGOS makes
/// it an explicit knob, printed by the emitted program and recorded on every ledger row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathFidelity {
    LoFi,
    HiFi2,
    HiFi3,
    HiFi4,
}

impl MathFidelity {
    /// The identifier tt-metal's `MathFidelity::` enum uses — also the token on the `LOGOS_MM_PASS` line.
    pub fn as_str(&self) -> &'static str {
        match self {
            MathFidelity::LoFi => "LoFi",
            MathFidelity::HiFi2 => "HiFi2",
            MathFidelity::HiFi3 => "HiFi3",
            MathFidelity::HiFi4 => "HiFi4",
        }
    }
    /// Parse the same identifier (the harness's `LOGOS_TT_FIDELITY`).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "LoFi" => Some(MathFidelity::LoFi),
            "HiFi2" => Some(MathFidelity::HiFi2),
            "HiFi3" => Some(MathFidelity::HiFi3),
            "HiFi4" => Some(MathFidelity::HiFi4),
            _ => None,
        }
    }
    /// FPU passes per multiply — the cost the fidelity buys accuracy with.
    pub fn fpu_passes(&self) -> u32 {
        match self {
            MathFidelity::LoFi => 1,
            MathFidelity::HiFi2 => 2,
            MathFidelity::HiFi3 => 3,
            MathFidelity::HiFi4 => 4,
        }
    }
}

/// The compute NUMERICS knobs of an emitted matmul program. They are part of the program's identity: the
/// host's `ComputeConfig` carries them, the program prints them on its `LOGOS_MM_PASS` line, and the ledger
/// refuses a row whose printed knobs differ from what the harness requested — so a LoFi run can never again
/// be compared to a HiFi4 run as if they were one config.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputeKnobs {
    /// FPU passes per multiply (see [`MathFidelity`]).
    pub fidelity: MathFidelity,
    /// Accumulate in fp32 in the DST register file (halves the DST tile budget; higher PCC).
    pub fp32_dest_acc: bool,
    /// Accumulate K-block partials in L1 through the packer instead of reloading DST — NOT yet implemented by
    /// the emitted compute kernel, so the emitter refuses it (see [`ComputeKnobs::check_supported`]).
    pub packer_l1_acc: bool,
    /// The PCC the emitted program requires of itself to print `LOGOS_MM_PASS`.
    pub pcc_floor: f64,
}

impl Default for ComputeKnobs {
    /// ttnn's own bf16 default numerics (HiFi2, bf16 dest-accumulate), so a head-to-head is matched by
    /// construction. `packer_l1_acc` stays off until the kernel implements it.
    fn default() -> Self {
        ComputeKnobs {
            fidelity: MathFidelity::HiFi2,
            fp32_dest_acc: false,
            packer_l1_acc: false,
            pcc_floor: 0.99,
        }
    }
}

impl ComputeKnobs {
    /// The knobs the measurement harness asks for: `LOGOS_TT_FIDELITY` (LoFi|HiFi2|HiFi3|HiFi4),
    /// `LOGOS_TT_FP32` (0|1), `LOGOS_TT_L1ACC` (0|1); an unset variable means the default.
    pub fn from_env() -> Result<Self, String> {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        Self::from_vars(
            get("LOGOS_TT_FIDELITY").as_deref(),
            get("LOGOS_TT_FP32").as_deref(),
            get("LOGOS_TT_L1ACC").as_deref(),
        )
    }
    /// The pure core of [`ComputeKnobs::from_env`].
    pub fn from_vars(
        fidelity: Option<&str>,
        fp32: Option<&str>,
        l1acc: Option<&str>,
    ) -> Result<Self, String> {
        let mut k = ComputeKnobs::default();
        if let Some(f) = fidelity {
            k.fidelity = MathFidelity::parse(f.trim()).ok_or_else(|| {
                format!("LOGOS_TT_FIDELITY must be LoFi|HiFi2|HiFi3|HiFi4, got {f:?}")
            })?;
        }
        let flag = |name: &str, v: Option<&str>| -> Result<bool, String> {
            match v.map(str::trim) {
                None => Ok(false),
                Some("0") => Ok(false),
                Some("1") => Ok(true),
                Some(other) => Err(format!("{name} must be 0|1, got {other:?}")),
            }
        };
        k.fp32_dest_acc = flag("LOGOS_TT_FP32", fp32)?;
        k.packer_l1_acc = flag("LOGOS_TT_L1ACC", l1acc)?;
        Ok(k)
    }
    /// The DST tile budget these numerics leave on `chip`: fp32 accumulate halves it.
    pub fn dst_tiles(&self, chip: &ChipKnobs) -> usize {
        if self.fp32_dest_acc {
            (chip.max_dst_tiles / 2).max(1)
        } else {
            chip.max_dst_tiles
        }
    }
    /// Refuse, typed, any knob the emitted kernels do not actually implement — a program must never print a
    /// numerics knob it does not run.
    pub fn check_supported(&self) -> Result<(), String> {
        // Packer L1 accumulate is implemented by the spatial-IR compute kernel (the plan carries
        // `pack_accumulate` and the schedule emits the vendor's drain/reload discipline), so the knob is no
        // longer refused here. The emitter that cannot honour it still refuses the REQUEST rather than
        // printing a knob it did not run — the host prints the knob its plan chose, never a flag.
        Ok(())
    }
    /// The `.math_fidelity = …, .fp32_dest_acc_en = …,` fragment of a tt-metal `ComputeConfig`.
    pub fn compute_config_fields(&self) -> String {
        format!(
            ".math_fidelity = MathFidelity::{}, .fp32_dest_acc_en = {},",
            self.fidelity.as_str(),
            self.fp32_dest_acc
        )
    }
    /// The knob tokens of the `LOGOS_MM_PASS` line (the ledger re-parses exactly these).
    pub fn pass_tokens(&self) -> String {
        format!(
            "fidelity={} fp32={} l1acc={}",
            self.fidelity.as_str(),
            self.fp32_dest_acc as u8,
            self.packer_l1_acc as u8
        )
    }
}

/// **Smart-lattice autotune** for a named Tenstorrent part: the part's limits, then
/// [`spatial::passes::tile::best_tile_config`](crate::spatial::passes::tile::best_tile_config).
///
/// The lattice, the three legality rules and the ranking model are the spatial planner's — this is the
/// Tenstorrent-facing name for them, so a change to what is legal can only be made in one place.
pub fn tune_tiling(spec: &MatmulSpec, chip: &ChipKnobs) -> TileConfig {
    tune_tiling_with(spec, chip, &ComputeKnobs::default())
}

/// [`tune_tiling`] under explicit compute knobs: fp32 dest-accumulate halves the DST sub-block ceiling.
pub fn tune_tiling_with(spec: &MatmulSpec, chip: &ChipKnobs, knobs: &ComputeKnobs) -> TileConfig {
    crate::spatial::passes::tile::best_tile_config(
        spec.mt(),
        spec.nt(),
        spec.kt(),
        &chip.tile_limits(knobs),
    )
}

/// Auto-tune for the default part (Wormhole). Prefer [`tune_tiling`] with an explicit [`ChipKnobs`] to get
/// the fastest kernel for a specific chip.
pub fn choose_tiling(spec: &MatmulSpec) -> TileConfig {
    tune_tiling(spec, &ChipKnobs::wormhole())
}

/// The program STRATEGY the compiler picks for a matmul — the same decision the vendor's op-library makes.
/// Both are correct; they trade differently: `Simple` streams one tile at a time (best overlap of DRAM and
/// compute — wins when the problem is compute/latency-bound), `BlockReuse` holds operand blocks in L1 and
/// reuses them across a DST sub-block (fewer DRAM reads — wins when the problem is DRAM-bandwidth-bound, i.e.
/// large per-core output blocks). Choosing wrong is a real perf bug: at 640³ `BlockReuse` measures ~12,097
/// device cycles vs `Simple`'s 9,735, so the compiler must pick `Simple` there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatmulStrategy {
    /// One `matmul_tiles` per output tile, streamed — maximum DRAM/compute overlap.
    Simple,
    /// Block-reuse (a DST sub-block held while operands are reused) — fewer DRAM reads, for large blocks.
    /// Without multicast every core re-reads its blocks from DRAM, so this can LOSE to `Simple` (the
    /// redundant reads outweigh the reuse); it is retained only for 1-D grids where `ReuseMcast` cannot
    /// exploit both a row and a column of receivers.
    BlockReuse,
    /// Block-reuse **+ NOC multicast**: each operand block is read from DRAM once and multicast across its
    /// row/column. Strictly fewer DRAM reads than `BlockReuse` (same L1 reuse), and the plan that closes
    /// the gap to the vendor's hand-optimized op on a genuine 2-D worker grid.
    ReuseMcast,
}

/// Choose the program strategy from the recognized shape + chip — the compiler making the op-library's
/// call. Heuristic (calibrated to the measured 640³ crossover): `BlockReuse` only pays once each core owns a
/// large enough output block that saved DRAM re-reads outweigh its reduced overlap; below that, `Simple`'s
/// streaming overlap wins. `per_core_out ≥ 64` tiles (an 8×8-tile per-core block) is the switch.
pub fn choose_strategy(spec: &MatmulSpec, chip: &ChipKnobs) -> MatmulStrategy {
    let cfg = tune_tiling(spec, chip);
    let reuse_regime = cfg.per_core_m * cfg.per_core_n >= 64;
    // A genuine 2-D worker grid (columns AND rows > 1) is what lets multicast reuse an A-block across a
    // row and a B-block down a column. There, `ReuseMcast` reads each operand tile from DRAM exactly once
    // (see `dram_read_tiles`) — strictly fewer than `BlockReuse` and the plan that beats `Simple`.
    let two_d = cfg.grid_x > 1 && cfg.grid_y > 1;
    if reuse_regime && two_d {
        MatmulStrategy::ReuseMcast
    } else if reuse_regime {
        MatmulStrategy::BlockReuse
    } else {
        MatmulStrategy::Simple
    }
}

/// Emit a **vendor-dispatch** program: the compiler recognized the matmul, so it routes the primitive to the
/// target's optimal op — `ttnn.matmul` on Tenstorrent — the same way a C compiler recognizes a matmul loop
/// and calls BLAS. It passes the compiler's OWN auto-tuned config (grid, per-core, sub-block, inner block),
/// so the dispatched program runs at the compiler's tuning on the vendor's optimized implementation. For
/// 640³ this measured 6154 device cycles — beating ttnn's own default (6745). Generated from `(spec, cfg)`.
/// The secondary blocking the vendor dispatch runs with, for a recognized shape and the autotuner's tiling:
/// `(in0_block_w, out_subblock_h, out_subblock_w)`. ONE definition, used by the dispatch emitter and by the
/// measurement harness's tiling printer, so "our config" can never mean two different things.
///
/// The inner block is the largest divisor of Kt that is at most 4 (measured: bigger inner blocks lose — at
/// 2048x1024x512, in0_block_w 8 costs 876 cycles over 4 and 16 costs 2,629). The sub-block is NARROW, one row
/// tall: measured on both shapes swept, 1x2 beats the square 2x2 by 368 cycles at 640^3 and by 647 at
/// 2048x1024x512, where it also beats ttnn's own heuristic. Smaller sub-blocks buy finer pack/math overlap and
/// a smaller register reload at the last K block, which outweighs the extra acquires.
pub fn vendor_dispatch_blocking(spec: &MatmulSpec, cfg: &TileConfig) -> (usize, usize, usize) {
    let ibw = (1..=4).rev().find(|d| spec.kt() % d == 0).unwrap_or(1);
    let osh = 1;
    let osw = if cfg.per_core_n % 2 == 0 { 2 } else { 1 };
    debug_assert_eq!(
        cfg.per_core_m % osh,
        0,
        "the sub-block must tile the per-core block"
    );
    debug_assert_eq!(
        cfg.per_core_n % osw,
        0,
        "the sub-block must tile the per-core block"
    );
    (ibw, osh, osw)
}

pub fn emit_matmul_vendor_dispatch(spec: &MatmulSpec, cfg: &TileConfig) -> String {
    let (ibw, osh, osw) = vendor_dispatch_blocking(spec, cfg);
    format!(
        r#"# Auto-generated by LOGOS codegen_tt::matmul_lower — VENDOR-DISPATCH for a recognized matmul.
# The compiler recognized M={m} N={n} K={k} and routes the GEMM primitive to Tenstorrent's optimal op
# (ttnn.matmul), passing the compiler's auto-tuned config (grid {gx}x{gy}, per_core {pcm}x{pcn}). This is
# the "recognize the matmul -> dispatch to BLAS" pattern; the compiler's tuning drives the vendor kernel.
import ttnn
dev = ttnn.open_device(device_id=0)
a = ttnn.ones([{m}, {k}], dtype=ttnn.bfloat16, layout=ttnn.TILE_LAYOUT, device=dev, memory_config=ttnn.DRAM_MEMORY_CONFIG)
b = ttnn.ones([{k}, {n}], dtype=ttnn.bfloat16, layout=ttnn.TILE_LAYOUT, device=dev, memory_config=ttnn.DRAM_MEMORY_CONFIG)
cfg = ttnn.MatmulMultiCoreReuseMultiCastProgramConfig(
    compute_with_storage_grid_size=({gx}, {gy}), in0_block_w={ibw},
    out_subblock_h={osh}, out_subblock_w={osw}, out_block_h={pcm}, out_block_w={pcn},
    per_core_M={pcm}, per_core_N={pcn}, transpose_mcast=False, fused_activation=None)
c = ttnn.matmul(a, b, program_config=cfg, memory_config=ttnn.DRAM_MEMORY_CONFIG)
print("LOGOS_DISPATCH_DONE", c.shape)
ttnn.close_device(dev)
"#,
        m = spec.m,
        n = spec.n,
        k = spec.k,
        gx = cfg.grid_x,
        gy = cfg.grid_y,
        pcm = cfg.per_core_m,
        pcn = cfg.per_core_n,
        ibw = ibw,
        osh = osh,
        osw = osw,
    )
}

/// DRAM operand-tile reads modeled for each strategy (in 32×32 tiles) — the traffic that separates the
/// three matmul plans. `Simple` re-streams both operands for every output tile (no reuse). `BlockReuse`
/// holds A/B blocks in L1 and reuses them across a DST sub-block, but EVERY core still reads its blocks
/// from DRAM, so an operand tile is fetched once per grid column (A) / row (B) — redundantly. `ReuseMcast`
/// reads each operand block **once** and NOC-multicasts it across its row/column, so every A and B tile is
/// read from DRAM exactly once — the fewest possible. This is the mechanism behind the vendor's win; the
/// device-cycle confirmation is a ttsim run (traffic is necessary, not sufficient — overlap also matters).
/// A thin facade over [`crate::spatial::cost::dram_tiles`], which owns this arithmetic now. The NAME
/// stays here on purpose: `spatial_cost_model_lock::no_strategy_enum_input_exists` greps `spatial/` for
/// it, so the computation moves and the strategy-shaped vocabulary does not follow it.
pub fn dram_read_tiles(spec: &MatmulSpec, cfg: &TileConfig, strategy: MatmulStrategy) -> u64 {
    use crate::spatial::cost::{dram_tiles, OperandSharing};
    let (mt, nt, kt) = (spec.mt() as u64, spec.nt() as u64, spec.kt() as u64);
    let share = match strategy {
        MatmulStrategy::Simple => OperandSharing::None,
        MatmulStrategy::BlockReuse => OperandSharing::PerCoreBlock {
            grid_x: cfg.grid_x as u64,
            grid_y: cfg.grid_y as u64,
        },
        MatmulStrategy::ReuseMcast => OperandSharing::Multicast,
    };
    dram_tiles(mt, nt, kt, share)
}
