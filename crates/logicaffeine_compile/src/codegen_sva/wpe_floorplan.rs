//! The whole-program engine's memory, planned against a real process — before place-and-route.
//!
//! The WPE runs arbitrary LOGOS bytecode streamed over a wire, and it has run on two Artix-7
//! boards for months. Every number that describes its memory, though, was chosen for an FPGA:
//!
//!   * `HEAP = 32768` is a power of two because a RAMB36E1 is 32768x1, so a 15-bit address lands
//!     on exactly one tile with no depth-cascade — dodging a yosys `memory_libmap` mis-map that
//!     silently aliased every heap access above 4096 and made the transformer return a wrong
//!     answer on silicon. Nothing about any program wanted 32768; the measured working set is
//!     ~18,300 words.
//!   * `NREG = 512` is a power of two for the same reason, against a measured ceiling of 175.
//!     That one is paid for `DEPTH` times over, because the frame stack `sfile` is `DEPTH x NREG`.
//!   * The `_lo`/`_hi` split of every 64-bit array exists to force TDP-W36 and dodge a parity-lane
//!     mis-wire.
//!
//! An ASIC has no RAMB tiles, so the first two buy nothing and cost square millimetres. (The third
//! is a stroke of luck: it makes every array exactly 32 bits wide, which is exactly the width of
//! sky130's SRAM macros, so no width-banking is needed.)
//!
//! This module therefore SOLVES the profile instead of inheriting it: measured program demands in,
//! a cap profile and a full floorplan out, or a typed refusal. It answers the question that
//! actually gates the tapeout — *what does this engine have to give up to fit a shuttle die, and
//! what is left over for the logic* — with arithmetic over the real macro inventory in
//! [`crate::codegen_sva::pdk_target`], rather than with a guess that place-and-route discovers
//! eight hours later.
//!
//! The array table is checked against the emitted Verilog by `wpe_floorplan_lock`, so an area
//! model that has drifted away from the RTL it describes fails a test instead of a tapeout.

use crate::codegen_sva::pdk_target::{
    Implementation, MemoryRequest, PdkTarget, PdkUnsupported, MEASURED_CELLS_PER_MM2,
};
use crate::codegen_sva::wprog_engine_rtl::{
    eval_wprog_config_run, wprog_config_image_unchecked, WProgConfig,
};

/// What an array is FOR — which is what decides whether it can live off-chip.
///
/// Off-chip residence is not a property of size. It is a property of access pattern: an array that
/// is read every cycle by combinational logic cannot cross a package boundary at any capacity,
/// while one the FSM already reads through an ISSUE/WAIT/LATCH triplet can absorb an N-cycle
/// transaction as a latency change to a structure that already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrayRole {
    /// The program's runtime state plane (`heap_*`). Bulk, latency-tolerant, already registered.
    Heap,
    /// The call-frame spill area (`sfile_*`), touched only at Call and Return.
    FrameStack,
    /// The program image itself (`cell_*`) — what the loader streams in and the interpreter walks.
    ConfigImage,
    /// Per-block metadata (`blk_*`, `wr_*`, `liv_reg`, `arg_op`): small, random-access, hot.
    BlockTable,
    /// The register file and its write buffer (`file_*`, `param_buf`, `wbuf_*`).
    RegisterFile,
    /// Per-block value scratch (`v_*`), the constant table, and the tiny return stacks.
    Scratch,
}

/// One addressed array, exactly as the emitted RTL declares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WpeArray {
    pub name: &'static str,
    pub words: u32,
    pub bits: u32,
    /// Read and written in the same cycle, so a single-port macro cannot express it.
    pub dual_port: bool,
    /// Read COMBINATIONALLY — the value is consumed in the same cycle the address is presented.
    /// An SRAM macro cannot serve this at any speed, because a macro read is registered by
    /// construction. Such an array is flip-flops, or the FSM grows a wait state. Marking it here
    /// is what stops the planner from producing a floorplan that cannot be built.
    pub combinational_read: bool,
    pub role: ArrayRole,
}

impl WpeArray {
    pub fn bits_total(&self) -> u64 {
        u64::from(self.words) * u64::from(self.bits)
    }
}

/// The engine's cap profile — the ten numbers that size every memory in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WpeProfile {
    pub nreg: usize,
    pub maxblk: usize,
    pub maxcell: usize,
    pub maxwr: usize,
    pub maxarg: usize,
    pub maxliv: usize,
    pub maxk: usize,
    pub heap: usize,
    pub depth: usize,
    pub maxv: usize,
}

impl WpeProfile {
    /// Every dimension of `d` this profile does not cover, as (the engine parameter, the demand, the cap). Empty
    /// exactly when a program with those demands loads and runs without a refusal or a trap.
    pub fn shortfalls(&self, d: &WpeDemands) -> Vec<(&'static str, usize, usize)> {
        [
            ("NREG", d.registers, self.nreg),
            ("MAXBLK", d.blocks, self.maxblk),
            ("MAXCELL", d.cells, self.maxcell),
            ("MAXWR", d.writes, self.maxwr),
            ("MAXARG", d.call_args, self.maxarg),
            ("MAXLIV", d.liveins, self.maxliv),
            ("MAXK", d.konsts, self.maxk),
            ("HEAP", d.heap_words, self.heap),
            ("DEPTH", d.depth, self.depth),
            ("MAXV", d.cells_per_block, self.maxv),
        ]
        .into_iter()
        .filter(|&(_, need, cap)| need > cap)
        .collect()
    }

    /// The `[usize; 10]` order `emit_wprog_engine_soc_verilog_backend` expects.
    pub fn from_caps(c: [usize; 10]) -> Self {
        Self {
            nreg: c[0],
            maxblk: c[1],
            maxcell: c[2],
            maxwr: c[3],
            maxarg: c[4],
            maxliv: c[5],
            maxk: c[6],
            heap: c[7],
            depth: c[8],
            maxv: c[9],
        }
    }

    pub fn caps(&self) -> [usize; 10] {
        [
            self.nreg,
            self.maxblk,
            self.maxcell,
            self.maxwr,
            self.maxarg,
            self.maxliv,
            self.maxk,
            self.heap,
            self.depth,
            self.maxv,
        ]
    }

    /// `HBANK0`/`HBANK1` exactly as `wpe_soc_template.v` computes them. The split is an FPGA
    /// artifact — it exists so each half maps to a 14-bit-indexed RAMB tile instead of the
    /// least-fuzzed 32K depth-cascade corner, which prjxray mis-encodes BELOW the netlist. It is
    /// mirrored here because the model's job is to describe the RTL that exists, not the RTL that
    /// ought to; an ASIC backend should elide the dummy bank when `HEAP <= 16384`.
    fn heap_banks(&self) -> (u32, u32) {
        if self.heap > 16384 {
            (16384, (self.heap - 16384) as u32)
        } else {
            (self.heap as u32, 1)
        }
    }

    /// Evaluate an RTL array bound (`[0:<expr>]`) to a WORD COUNT — so `MAXBLK-1` yields `MAXBLK`.
    /// Supports the forms the template actually uses: an identifier, a product of two, and a
    /// trailing `-1`. Returns `None` for anything it does not understand, so a new declaration
    /// shape fails the lock loudly rather than being silently skipped.
    pub fn eval_depth(&self, expr: &str) -> Option<u32> {
        let (hb0, hb1) = self.heap_banks();
        let lookup = |t: &str| -> Option<u64> {
            let t = t.trim();
            if let Ok(n) = t.parse::<u64>() {
                return Some(n);
            }
            Some(match t {
                "NREG" => self.nreg as u64,
                "MAXBLK" => self.maxblk as u64,
                "MAXCELL" => self.maxcell as u64,
                "MAXWR" => self.maxwr as u64,
                "MAXARG" => self.maxarg as u64,
                "MAXLIV" => self.maxliv as u64,
                "MAXK" => self.maxk as u64,
                "HEAP" => self.heap as u64,
                "DEPTH" => self.depth as u64,
                "MAXV" => self.maxv as u64,
                "HBANK0" => u64::from(hb0),
                "HBANK1" => u64::from(hb1),
                _ => return None,
            })
        };
        // `<product> - 1`, where `<product>` is one or two identifiers joined by `*`.
        let (head, minus) = match expr.rsplit_once('-') {
            Some((h, t)) => (h, t.trim().parse::<u64>().ok()?),
            None => (expr, 0),
        };
        let mut v = 1u64;
        for f in head.split('*') {
            v = v.checked_mul(lookup(f)?)?;
        }
        u32::try_from(v.checked_sub(minus)? + 1).ok()
    }

    /// Every addressed array the engine declares, mirroring `wpe_soc_template.v`.
    ///
    /// `config_bram` selects the streamed (`MemBackend::ConfigBram`) build, which is the one that
    /// would be taped out: it EXCISES `file0` (the block-entry register snapshot) in favour of the
    /// read-before-write buffer. Charging a dead array against the die would inflate the estimate.
    pub fn arrays(&self, config_bram: bool) -> Vec<WpeArray> {
        use ArrayRole::*;
        let (hb0, hb1) = self.heap_banks();
        let (nreg, maxblk) = (self.nreg as u32, self.maxblk as u32);
        let (maxcell, maxwr) = (self.maxcell as u32, self.maxwr as u32);
        let (maxarg, maxliv, maxk) = (self.maxarg as u32, self.maxliv as u32, self.maxk as u32);
        let (depth, maxv) = (self.depth as u32, self.maxv as u32);
        let sfile = depth.saturating_mul(nreg);

        let mut a: Vec<WpeArray> = Vec::new();
        let mut push = |name, words, bits, comb, role| {
            a.push(WpeArray {
                name,
                words,
                bits,
                dual_port: true,
                combinational_read: comb,
                role,
            });
        };

        for n in [
            "blk_livein_ptr",
            "blk_n_livein",
            "blk_cell_ptr",
            "blk_n_cells",
            "blk_write_ptr",
            "blk_n_writes",
            "blk_term_kind",
            "blk_t0",
            "blk_t1",
            "blk_top",
            "blk_cdst",
            "blk_cargc",
            "blk_cargp",
        ] {
            push(n, maxblk, 32, false, BlockTable);
        }
        for n in [
            "cell_kind",
            "cell_code",
            "cell_op0",
            "cell_op1",
            "cell_op2",
            "cell_op3",
        ] {
            push(n, maxcell, 32, false, ConfigImage);
        }
        push("wr_op", maxwr, 32, false, BlockTable);
        push("wr_reg", maxwr, 32, false, BlockTable);
        // Deliberately flip-flops in the RTL (`ram_style = "registers"`) and read in the same
        // cycle the address is formed — these are the five the audit found, and they are small
        // enough that keeping them as flops costs less than the wait state would.
        push("arg_op", maxarg, 32, true, BlockTable);
        push("liv_reg", maxliv, 32, true, BlockTable);
        push("konst", maxk, 64, true, Scratch);
        push("sret", depth, 16, true, Scratch);
        push("sdst", depth, 16, true, Scratch);

        push("file_lo", nreg, 32, false, RegisterFile);
        push("file_hi", nreg, 32, false, RegisterFile);
        if !config_bram {
            push("file0", nreg, 64, true, RegisterFile);
        }
        push("param_buf", nreg, 64, false, RegisterFile);
        push("wbuf_val", nreg, 64, true, RegisterFile);
        push("wbuf_tgt", nreg, 16, true, RegisterFile);

        push("v_lo", maxv, 32, false, Scratch);
        push("v_hi", maxv, 32, false, Scratch);
        push("sfile_lo", sfile, 32, false, FrameStack);
        push("sfile_hi", sfile, 32, false, FrameStack);
        push("heap_b0_lo", hb0, 32, false, Heap);
        push("heap_b0_hi", hb0, 32, false, Heap);
        push("heap_b1_lo", hb1, 32, false, Heap);
        push("heap_b1_hi", hb1, 32, false, Heap);
        a
    }

    pub fn total_bits(&self, config_bram: bool) -> u64 {
        self.arrays(config_bram)
            .iter()
            .map(WpeArray::bits_total)
            .sum()
    }
}

/// What the corpus actually demands — every field a MEASURED ceiling across the lowering benchmark
/// suite, never a guess. `depth` and `heap_words` are the two that cannot be read off a config and
/// must be obtained by RUNNING the program; they are also the two the hardware gets silently wrong
/// (the engine advances `sp` past `DEPTH` with no bound check), which is how a DEPTH=4 engine once
/// shipped against DEPTH=8 programs and answered `fib` with 8 instead of 21.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WpeDemands {
    pub blocks: usize,
    pub registers: usize,
    pub cells: usize,
    pub cells_per_block: usize,
    pub writes: usize,
    pub liveins: usize,
    pub call_args: usize,
    pub konsts: usize,
    pub depth: usize,
    pub heap_words: usize,
}

/// Headroom over a measured demand: +15%, then rounded up to a multiple of 16.
///
/// Enough that a program somewhat larger than anything in the suite still runs; small enough that
/// the headroom is not itself the thing that does not fit. There are deliberately NO powers of two
/// here — that constraint came from RAMB tile mapping, and on an ASIC the only granularity that
/// costs anything is the SRAM macro's minimum depth, which the planner accounts for directly.
const fn with_headroom(n: usize) -> usize {
    let raised = n * 115 / 100;
    (raised + 15) / 16 * 16
}

/// The MEASURED ceiling across the whole lowering benchmark suite, from
/// `wprog_all_benchmarks_lane::print_max_dims_across_all_benchmarks` (35 programs). `depth` and
/// `heap_words` are the two that cannot be read off a config and must be obtained by RUNNING the
/// program — and `depth` is the one the hardware gets silently wrong, because the engine advances
/// `sp` past `DEPTH` with no bound check. That is how a DEPTH=4 engine once shipped against
/// DEPTH=8 programs and answered `fib` with 8 instead of 21.
pub const MEASURED_SUITE: WpeDemands = WpeDemands {
    blocks: 38,           // sha256
    registers: 175,       // nbody
    cells: 920,           // sha256
    cells_per_block: 450, // sha256
    writes: 395,          // sha256
    liveins: 258,         // the streamed transformer
    call_args: 24,        // the recursion demo
    konsts: 91,           // the postsynth 90-value sum
    depth: 26,            // fib, MEASURED by running it
    heap_words: 18_300,   // the transformer's working set
};

/// The ASIC cap profile, DERIVED from [`MEASURED_SUITE`] rather than written down.
///
/// A second hand-maintained cap table is exactly the thing that rots: the one in
/// `examples/wpe_memory_plan.rs` had already drifted from the RTL it claimed to describe, missing
/// `liv_reg`/`arg_op`/`konst`/`sret`/`sdst` entirely. This one cannot drift from the demands,
/// because it is computed from them at compile time.
pub const ASIC_PROFILE: WpeProfile = MEASURED_SUITE.solve();

impl WpeDemands {
    /// What one program demands of an engine: every table the loader receives, read off the image it is sent
    /// as, and the frame depth and heap words its runs over `args` reach — the two a config cannot tell, and
    /// the two the engine traps on at run time rather than refusing at load.
    pub fn of(cfg: &WProgConfig, args: &[&[i64]]) -> WpeDemands {
        let runs: Vec<_> = args
            .iter()
            .map(|a| eval_wprog_config_run(cfg, a, &[]))
            .collect();
        WpeDemands {
            depth: runs.iter().map(|r| r.peak_depth).max().unwrap_or(0),
            heap_words: runs.iter().map(|r| r.heap.len()).max().unwrap_or(0),
            ..WpeDemands::loaded(cfg)
        }
    }

    /// The demands a host can read off a config without running it: every table the loader receives. Depth
    /// and heap words are zero — the engine traps those on die.
    pub fn loaded(cfg: &WProgConfig) -> WpeDemands {
        let img = wprog_config_image_unchecked(cfg);
        WpeDemands {
            blocks: img.blk_term_kind.len(),
            registers: img.nreg,
            cells: img.cell_kind.len(),
            cells_per_block: img.blk_n_cells.iter().copied().max().unwrap_or(0) as usize,
            writes: img.wr_reg.len(),
            liveins: img.liv_reg.len(),
            call_args: img.arg_op.len(),
            konsts: img.konst.len(),
            depth: 0,
            heap_words: 0,
        }
    }

    /// The smallest profile that covers every measured demand with stated headroom.
    ///
    /// `const` so [`ASIC_PROFILE`] is derived rather than transcribed.
    pub const fn solve(&self) -> WpeProfile {
        WpeProfile {
            nreg: with_headroom(self.registers),
            maxblk: with_headroom(self.blocks),
            maxcell: with_headroom(self.cells),
            maxwr: with_headroom(self.writes),
            maxarg: with_headroom(self.call_args),
            maxliv: with_headroom(self.liveins),
            maxk: with_headroom(self.konsts),
            heap: with_headroom(self.heap_words),
            depth: with_headroom(self.depth),
            maxv: with_headroom(self.cells_per_block),
        }
    }

    /// Non-const alias kept for callers that read better this way.
    pub fn smallest_covering_profile(&self) -> WpeProfile {
        self.solve()
    }
}

/// Where one array ends up.
#[derive(Debug, Clone, PartialEq)]
pub enum Residence {
    Macros {
        macro_name: &'static str,
        instances: u32,
    },
    Flops {
        cells: u64,
    },
    /// Behind the memory edge, off the die entirely.
    OffChip,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub array: WpeArray,
    pub residence: Residence,
    pub area_mm2: f64,
}

/// A solved floorplan: where every array lives, what it costs, and what is left for the logic.
#[derive(Debug, Clone, PartialEq)]
pub struct WpeFloorplan {
    pub profile: WpeProfile,
    pub placements: Vec<Placement>,
    pub macro_mm2: f64,
    pub flop_mm2: f64,
    /// Total on-die memory area — macros plus flop-backed arrays.
    pub memory_mm2: f64,
    /// Die area the requested logic needs at the measured standard-cell density.
    pub logic_mm2: f64,
    pub logic_cells: u64,
    pub macro_instances: u32,
    pub off_chip_bits: u64,
}

/// Why an engine cannot be placed. Typed, so a caller cannot mistake a refusal for a plan.
#[derive(Debug, Clone, PartialEq)]
pub enum WpeUnplaceable {
    /// The memory plus the logic is larger than the vehicle's user area.
    ExceedsUserArea {
        memory_mm2: f64,
        logic_mm2: f64,
        available_mm2: f64,
    },
    /// The vehicle has no fixed user area, so "fits" is not a question this can answer.
    NoUserArea { pdk: &'static str },
    /// The process cannot build one of the arrays at all.
    Process {
        array: &'static str,
        why: PdkUnsupported,
    },
}

/// Plan every array of `profile` onto `pdk`, sending the roles in `off_chip` behind the memory
/// edge, and check the result against the vehicle's user area alongside `logic_cells` of engine.
///
/// An array whose role is off-chip costs no die area and contributes to `off_chip_bits`. An array
/// that is read combinationally is never given a macro, because a macro read is registered and the
/// placement would be unbuildable — the planner refuses to produce a floorplan it knows is a lie.
pub fn plan_wpe(
    pdk: &'static PdkTarget,
    profile: &WpeProfile,
    off_chip: &[ArrayRole],
    logic_cells: u64,
) -> Result<WpeFloorplan, WpeUnplaceable> {
    let Some(ua) = pdk.user_area else {
        return Err(WpeUnplaceable::NoUserArea { pdk: pdk.name });
    };

    let (mut macro_mm2, mut flop_mm2, mut off_chip_bits, mut macro_instances) = (0.0, 0.0, 0u64, 0);
    let mut placements = Vec::new();

    for array in profile.arrays(true) {
        if off_chip.contains(&array.role) {
            off_chip_bits += array.bits_total();
            placements.push(Placement {
                array,
                residence: Residence::OffChip,
                area_mm2: 0.0,
            });
            continue;
        }

        // A combinationally-read array is flops by construction — asking `plan_memory` would let
        // it answer "macro", which is smaller and unbuildable.
        let (residence, area) = if array.combinational_read {
            let a = flop_area_mm2(array.bits_total());
            (
                Residence::Flops {
                    cells: (a * MEASURED_CELLS_PER_MM2) as u64,
                },
                a,
            )
        } else {
            let req = MemoryRequest {
                words: array.words,
                bits: array.bits,
                dual_port: array.dual_port,
                needs_multi_corner: true,
            };
            match pdk.plan_memory(req) {
                Ok(p) => match p.implementation {
                    Implementation::Flops { cells } => (Residence::Flops { cells }, p.area_mm2),
                    Implementation::Macros {
                        macro_name,
                        instances,
                        ..
                    } => (
                        Residence::Macros {
                            macro_name,
                            instances,
                        },
                        p.area_mm2,
                    ),
                },
                // One array being larger than the whole user area is an AREA fact about this
                // floorplan, not a process refusal — keep its cost and let the aggregate check
                // below report the real total, which is what a reader needs to see.
                Err(PdkUnsupported::ExceedsUserArea { needed_mm2, .. }) => (
                    Residence::Macros {
                        macro_name: "(over-area)",
                        instances: 0,
                    },
                    needed_mm2,
                ),
                Err(why) => {
                    return Err(WpeUnplaceable::Process {
                        array: array.name,
                        why,
                    });
                }
            }
        };
        match &residence {
            Residence::Macros { instances, .. } => {
                macro_mm2 += area;
                macro_instances += instances;
            }
            Residence::Flops { .. } => flop_mm2 += area,
            Residence::OffChip => {}
        }
        placements.push(Placement {
            array,
            residence,
            area_mm2: area,
        });
    }

    let memory_mm2 = macro_mm2 + flop_mm2;
    let logic_mm2 = logic_cells as f64 / MEASURED_CELLS_PER_MM2;
    if memory_mm2 + logic_mm2 > ua.mm2() {
        return Err(WpeUnplaceable::ExceedsUserArea {
            memory_mm2,
            logic_mm2,
            available_mm2: ua.mm2(),
        });
    }
    Ok(WpeFloorplan {
        profile: *profile,
        placements,
        macro_mm2,
        flop_mm2,
        memory_mm2,
        logic_mm2,
        logic_cells,
        macro_instances,
        off_chip_bits,
    })
}

fn flop_area_mm2(bits: u64) -> f64 {
    bits as f64 * crate::codegen_sva::pdk_target::FLOP_BIT_UM2 / 1.0e6
}
