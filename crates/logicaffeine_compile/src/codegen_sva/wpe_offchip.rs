//! The whole-program engine with its bulk memories off the die: the same loader and interpret FSM, whose
//! heap, frame stack and config image — any of them — live in a QSPI PSRAM behind one
//! [`psram_edge`](super::psram::emit_psram_edge_verilog), each through its own
//! [`psram_cache`](super::psram::emit_psram_cache_verilog), the caches sharing the edge through a
//! [`psram_arbiter`](super::psram::emit_psram_arbiter_verilog).
//!
//! Every cache keeps the port of the block RAM it replaces — a registered read one cycle after its address,
//! refreshed every cycle — and while any of them serves a miss it raises `mstall`, which freezes the whole
//! loader/interpret FSM. A program's Show records, its result and its counted run cycles are therefore the
//! on-chip engine's exactly; a miss costs only wall-clock time. The one input that does not wait is the
//! UART: the receiver holds one byte while the FSM is stalled, so no byte is lost as long as every stall
//! the loader can meet is shorter than a byte ([`OffchipMemory::check_uart`]).
//!
//! The memories an engine sends off-chip are named by the floorplan's own [`ArrayRole`]s, so the plan that
//! decides what leaves the die ([`plan_wpe`](super::wpe_floorplan::plan_wpe)) and the RTL that takes it
//! away speak one vocabulary.

use super::psram::{
    emit_psram_arbiter_verilog, emit_psram_cache_hit_function, emit_psram_cache_hit_wires,
    emit_psram_cache_verilog, emit_psram_edge_verilog, emit_psram_model_verilog,
    psram_cache_hit_ports, PsramCache, PsramEdge, EDGE_WORD_ADDR_BITS,
};
use super::wpe_floorplan::ArrayRole;
use super::wprog_engine_rtl::{
    emit_wprog_engine_soc_offchip_body, emit_wprog_engine_soc_testbench_board, MemBackend, WImage,
};
use std::fmt::Write as _;

/// The engine memories behind one PSRAM edge, each with its own cache; `None` keeps a memory on the die.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OffchipMemory {
    pub edge: PsramEdge,
    pub heap: Option<PsramCache>,
    pub frames: Option<PsramCache>,
    /// The config image, as three lanes of one word per cell — {code, kind}, {op1, op0}, {op3, op2} — each
    /// behind a cache of this geometry.
    pub cells: Option<PsramCache>,
}

/// Why an engine cannot have these memories off-chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OffchipUnplaceable {
    /// Nothing is off-chip; the on-chip emitters build that engine.
    NothingOffChip,
    /// The walk reads the config image through a registered port only under [`MemBackend::ConfigBram`];
    /// the other backends read it combinationally, which no cache can serve.
    CellsNeedConfigBram { backend: MemBackend },
    /// The regions the memories need do not fit the part.
    RegionsLargerThanPart { words: u64, part_words: u64 },
    /// A stall the loader can meet outlasts one UART byte, so a byte could complete while the one before it
    /// still waits.
    StallOutlastsByte { stall_cycles: u64, byte_cycles: u64 },
    /// The edge could still be initializing the part when the loader first touches it.
    InitOutlastsLoad { init_cycles: u64, load_cycles: u64 },
}

/// One memory's place in the part: a naturally aligned power-of-two block of words at `base`, so its
/// address is the region's high bits over the low `bits` bits of its own index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub name: &'static str,
    pub base: u32,
    pub bits: u32,
}

/// The fewest bytes the loader takes before it can touch the part. A program whose every config memory
/// and parameter list is empty streams exactly these, ending in the go byte that starts the heap clear;
/// every other stream writes its first cell, its first preloaded word or its heap later still.
pub fn wire_bytes_before_first_part_access() -> u64 {
    WImage::default().wire_bytes(&[]).len() as u64
}

/// System cycles of one UART byte at `clks_per_bit`: a start bit, eight data bits and a stop bit.
pub fn uart_byte_cycles(clks_per_bit: u64) -> u64 {
    10 * clks_per_bit
}

fn ceil_log2(n: usize) -> u32 {
    n.max(1).next_power_of_two().trailing_zeros()
}

impl OffchipMemory {
    /// The floorplan roles this engine takes off the die.
    pub fn roles(&self) -> Vec<ArrayRole> {
        [
            (self.heap, ArrayRole::Heap),
            (self.frames, ArrayRole::FrameStack),
            (self.cells, ArrayRole::ConfigImage),
        ]
        .into_iter()
        .filter_map(|(c, r)| c.map(|_| r))
        .collect()
    }

    /// The part's words within the edge's word address.
    pub fn part_words(&self) -> u64 {
        (self.edge.part.capacity_bytes as u64 / 8).min(1 << EDGE_WORD_ADDR_BITS)
    }

    /// Each off-chip memory's region for the cap profile `caps`, largest first so every region is aligned
    /// to its own size: the three cell lanes (`MAXCELL` words each), the frame stack (`DEPTH x NREG`) and
    /// the heap (`HEAP`), each rounded up to a power of two. An index past its memory's cap wraps inside
    /// its own region and never reaches another memory's.
    pub fn regions(&self, caps: [usize; 10]) -> Vec<Region> {
        let [nreg, _, maxcell, _, _, _, _, heap, depth, _] = caps;
        let mut r: Vec<(&'static str, u32)> = Vec::new();
        if self.cells.is_some() {
            let bits = ceil_log2(maxcell);
            r.extend([
                ("cell_lane0", bits),
                ("cell_lane1", bits),
                ("cell_lane2", bits),
            ]);
        }
        if self.frames.is_some() {
            r.push(("frames", ceil_log2(depth * nreg)));
        }
        if self.heap.is_some() {
            r.push(("heap", ceil_log2(heap)));
        }
        r.sort_by(|a, b| b.1.cmp(&a.1));
        let mut base = 0u32;
        r.into_iter()
            .map(|(name, bits)| {
                let region = Region { name, base, bits };
                base += 1 << bits;
                region
            })
            .collect()
    }

    /// Refuse a combination the engine cannot build: nothing off-chip, the config image off-chip without
    /// the registered config port, or regions the part cannot hold.
    pub fn check(&self, caps: [usize; 10], backend: MemBackend) -> Result<(), OffchipUnplaceable> {
        if self.roles().is_empty() {
            return Err(OffchipUnplaceable::NothingOffChip);
        }
        if self.cells.is_some() && backend != MemBackend::ConfigBram {
            return Err(OffchipUnplaceable::CellsNeedConfigBram { backend });
        }
        let words: u64 = self.regions(caps).iter().map(|r| 1u64 << r.bits).sum();
        if words > self.part_words() {
            return Err(OffchipUnplaceable::RegionsLargerThanPart {
                words,
                part_words: self.part_words(),
            });
        }
        Ok(())
    }

    /// The longest stall the loader can meet while bytes are still arriving. Until it takes the go byte
    /// the loader touches the part only to write — one config-image field per four bytes, one preloaded
    /// heap word per eight — and the walk fetches no cells, so the worst is one write miss in the heap or
    /// cell caches that evicts a dirty line. Stalls in the run are not bounded by the UART: the host sends
    /// nothing until the result frame is back.
    pub fn loading_stall_cycles(&self) -> u64 {
        [self.heap, self.cells]
            .into_iter()
            .flatten()
            .map(|c| c.miss_stall_cycles(&self.edge))
            .max()
            .unwrap_or(0)
    }

    /// Refuse a UART that could lose a byte. The receiver holds one byte while the FSM is stalled, so a byte
    /// is lost only if another completes before the stall ends; requiring every loading stall to be shorter
    /// than a byte means no byte ever waits at all. The first access to the part comes no sooner than
    /// [`wire_bytes_before_first_part_access`] bytes after reset, and the edge must have initialized the
    /// part by then, or that access would wait out the power-up too.
    pub fn check_uart(&self, clks_per_bit: u64) -> Result<(), OffchipUnplaceable> {
        let byte_cycles = uart_byte_cycles(clks_per_bit);
        let stall_cycles = self.loading_stall_cycles();
        if stall_cycles >= byte_cycles {
            return Err(OffchipUnplaceable::StallOutlastsByte {
                stall_cycles,
                byte_cycles,
            });
        }
        let load_cycles = wire_bytes_before_first_part_access() * byte_cycles;
        let init_cycles = self.edge.init_cycles();
        if init_cycles >= load_cycles {
            return Err(OffchipUnplaceable::InitOutlastsLoad {
                init_cycles,
                load_cycles,
            });
        }
        Ok(())
    }

    /// The fastest UART (fewest system cycles per bit) [`check_uart`](Self::check_uart) accepts.
    pub fn min_clks_per_bit(&self) -> u64 {
        let by_stall = self.loading_stall_cycles() / 10 + 1;
        let by_init = self.edge.init_cycles() / (10 * wire_bytes_before_first_part_access()) + 1;
        by_stall.max(by_init)
    }

    /// The Verilog that wires the off-chip memories to the part: a cache per memory (three for the config
    /// image), the arbiter, the edge on the PSRAM pins, and `mstall`, the OR of every cache's `busy`.
    fn glue(&self, caps: [usize; 10]) -> String {
        let aw = EDGE_WORD_ADDR_BITS;
        let regions = self.regions(caps);
        let addr = |name: &str, index: &str| {
            let r = regions
                .iter()
                .find(|r| r.name == name)
                .expect("a region per off-chip memory");
            match aw - r.bits {
                0 => format!("{index}[{}:0]", aw - 1),
                high => format!(
                    "{{{high}'d{}, {index}[{}:0]}}",
                    r.base >> r.bits,
                    r.bits - 1
                ),
            }
        };
        struct Port<'a> {
            inst: String,
            cache: &'a PsramCache,
            req_r: String,
            ra: String,
            rhit: String,
            req_w: String,
            wa: String,
            whit: String,
            wd: &'static str,
            wm: &'static str,
            rd: String,
        }
        // The template's tasks register each address's hit through one function per memory, `<q>_hit_at(a)` of the
        // engine address `a`: the lookup in that memory's cache (or, for the config image, each lane's) of the
        // address the cache sees for it.
        let mut hit_at = String::new();
        let mut ports: Vec<Port> = Vec::new();
        let mut cell_wires = String::new();
        if let Some(c) = &self.heap {
            ports.push(Port {
                inst: "heap_cache".into(),
                cache: c,
                req_r: "hq_r".into(),
                ra: addr("heap", "heap_ra"),
                rhit: "hq_rhit".into(),
                req_w: "hq_w".into(),
                wa: addr("heap", "hq_wa"),
                whit: "hq_whit".into(),
                wd: "hq_wd",
                wm: "2'b11",
                rd: "heap_rd".into(),
            });
            let _ = write!(
                hit_at,
                "  function hq_hit_at(input [31:0] a); hq_hit_at = heap_cache_hit({}); endfunction\n",
                addr("heap", "a")
            );
        }
        if let Some(c) = &self.frames {
            ports.push(Port {
                inst: "frame_cache".into(),
                cache: c,
                req_r: "sq_r".into(),
                ra: addr("frames", "sf_ra"),
                rhit: "sq_rhit".into(),
                req_w: "sq_w".into(),
                wa: addr("frames", "sq_wa"),
                whit: "sq_whit".into(),
                wd: "sq_wd",
                wm: "2'b11",
                rd: "sf_rd".into(),
            });
            let _ = write!(
                hit_at,
                "  function sq_hit_at(input [31:0] a); sq_hit_at = frame_cache_hit({}); endfunction\n",
                addr("frames", "a")
            );
        }
        if let Some(c) = &self.cells {
            cell_wires.push_str(
                "  // The walk reads the current cell every cycle of the run, as the on-chip registered read does.\n\
                 \x20 wire cell_fetch = (ph == P_RUN);\n\
                 \x20 wire [31:0] cell_at = c_cellptr + ci;\n\
                 \x20 wire [63:0] cell_rd0, cell_rd1, cell_rd2;\n\
                 \x20 assign cc_kind_r = cell_rd0[31:0]; assign cc_code_r = cell_rd0[63:32];\n\
                 \x20 assign cc_op0_r = cell_rd1[31:0];  assign cc_op1_r = cell_rd1[63:32];\n\
                 \x20 assign cc_op2_r = cell_rd2[31:0];  assign cc_op3_r = cell_rd2[63:32];\n",
            );
            let mut lanes = Vec::new();
            for k in 0..3 {
                let lane = format!("cell_lane{k}");
                lanes.push(format!("{lane}_hit({})", addr(&lane, "a")));
                ports.push(Port {
                    inst: lane.clone(),
                    cache: c,
                    req_r: "cell_fetch".into(),
                    ra: addr(&lane, "cell_at"),
                    rhit: format!("cq_rhit[{k}]"),
                    req_w: format!("cq_w[{k}]"),
                    wa: addr(&lane, "cq_wa"),
                    whit: format!("cq_whit[{k}]"),
                    wd: "cq_wd",
                    wm: "cq_wm",
                    rd: format!("cell_rd{k}"),
                });
            }
            lanes.reverse();
            let _ = write!(
                hit_at,
                "  function [2:0] cq_hit_at(input [31:0] a); cq_hit_at = {{{}}}; endfunction\n",
                lanes.join(", ")
            );
        }
        let n = ports.len();
        let mut g = format!(
            "  // The off-chip memories, one cache each, sharing the PSRAM edge through the arbiter. Each lives in its\n\
             \x20 // own naturally aligned power-of-two region of the part: its address is the region's high bits over\n\
             \x20 // its own index.\n\
             \x20 wire [{n1}:0] mem_busy;\n\
             \x20 wire mstall = |mem_busy;\n\
             \x20 wire [{n1}:0] mc_valid, mc_ready, mc_we;\n\
             \x20 wire [{awn}:0] mc_addr; wire [{lw}:0] mc_len; wire [{dw}:0] mc_wdata;\n\
             \x20 wire me_valid, me_ready, me_we, me_take, me_rvalid;\n\
             \x20 wire [{am}:0] me_addr; wire [7:0] me_len; wire [63:0] me_wdata, me_rdata;\n\
             {cell_wires}",
            n1 = n - 1,
            awn = n as u32 * aw - 1,
            lw = n * 8 - 1,
            dw = n * 64 - 1,
            am = aw - 1,
        );
        for (k, p) in ports.iter().enumerate() {
            g.push_str(&emit_psram_cache_hit_wires(&p.inst, p.cache));
            let _ = write!(
                g,
                "  psram_cache #(.IW({iw}), .OW({ow})) {inst} (\n\
                 \x20   .clk(clk), .rst(rst), .req_r({rq}), .ra({ra}), .req_w({wq}), .wa({wa}), .wd({wd}), .wm({wm}),\n\
                 \x20   {hits},\n\
                 \x20   .rd({rd}), .busy(mem_busy[{k}]), .e_valid(mc_valid[{k}]), .e_ready(mc_ready[{k}]), .e_we(mc_we[{k}]),\n\
                 \x20   .e_addr(mc_addr[{a} +: {aw}]), .e_len(mc_len[{l} +: 8]), .e_wdata(mc_wdata[{d} +: 64]),\n\
                 \x20   .e_wdata_take(me_take), .e_rdata(me_rdata), .e_rdata_valid(me_rvalid));\n",
                iw = p.cache.index_bits,
                ow = p.cache.offset_bits,
                inst = p.inst,
                hits = psram_cache_hit_ports(&p.inst, &p.rhit, &p.whit),
                rq = p.req_r,
                ra = p.ra,
                wq = p.req_w,
                wa = p.wa,
                wd = p.wd,
                wm = p.wm,
                rd = p.rd,
                a = k as u32 * aw,
                l = k * 8,
                d = k * 64,
            );
        }
        for p in &ports {
            g.push_str(&emit_psram_cache_hit_function(&p.inst, p.cache));
        }
        g.push_str(&hit_at);
        let _ = write!(
            g,
            "  psram_arbiter #(.N({n})) mem_arb (\n\
             \x20   .clk(clk), .rst(rst), .c_valid(mc_valid), .c_ready(mc_ready), .c_we(mc_we), .c_addr(mc_addr), .c_len(mc_len),\n\
             \x20   .c_wdata(mc_wdata), .e_valid(me_valid), .e_ready(me_ready), .e_we(me_we), .e_addr(me_addr), .e_len(me_len),\n\
             \x20   .e_wdata(me_wdata));\n\
             \x20 psram_edge mem_edge (\n\
             \x20   .clk(clk), .rst(rst), .req_valid(me_valid), .req_ready(me_ready), .req_we(me_we), .req_addr(me_addr),\n\
             \x20   .req_len(me_len), .wdata(me_wdata), .wdata_take(me_take), .rdata(me_rdata), .rdata_valid(me_rvalid),\n\
             \x20   .ce_n(psram_ce_n), .sclk(psram_sclk), .sio_out(psram_sio_out), .sio_oe(psram_sio_oe), .sio_in(psram_sio_in));"
        );
        g
    }
}

/// The engine with `memory`'s memories off the die: the template's off-chip regions for each of them and
/// the generated glue, then the FPU modules, the cache, the arbiter and the edge, in one file.
pub fn emit_wprog_engine_soc_verilog_offchip(
    name: &str,
    caps: [usize; 10],
    backend: MemBackend,
    memory: &OffchipMemory,
) -> Result<String, OffchipUnplaceable> {
    memory.check(caps, backend)?;
    let body = emit_wprog_engine_soc_offchip_body(name, caps, backend, &memory.roles());
    assert_eq!(
        body.matches("//__OFFCHIP_GLUE__").count(),
        1,
        "the template carries one glue site"
    );
    let cache = memory
        .heap
        .or(memory.frames)
        .or(memory.cells)
        .expect("checked: something is off-chip");
    Ok(format!(
        "{}{}\n{}\n{}",
        body.replace("//__OFFCHIP_GLUE__", &memory.glue(caps)),
        emit_psram_cache_verilog(&cache),
        emit_psram_arbiter_verilog(),
        emit_psram_edge_verilog(&memory.edge),
    ))
}

/// The over-the-wire host for the off-chip engine, on a board carrying the part: the same stimulus and
/// checks as the on-chip testbench, the clock at the edge's system period, the part's pin model on the SIO
/// pads, and a monitor that fails the run the moment a received byte is overwritten before the engine takes
/// it. Prints a `FRAME` line per program, then `PSRAM-VIOLATIONS` and `PSRAM-STALL-CYCLES`.
pub fn emit_wprog_engine_soc_offchip_testbench(
    name: &str,
    n_vectors: usize,
    clks_per_bit: usize,
    max_bytes: usize,
    memory: &OffchipMemory,
) -> String {
    emit_wprog_engine_soc_offchip_testbench_clocked(
        name,
        n_vectors,
        clks_per_bit,
        max_bytes,
        memory,
        memory.edge.sys_period_ps,
    )
}

/// [`emit_wprog_engine_soc_offchip_testbench`] with the clock at `clk_period_ps` — any period in the edge's
/// clock range, where its cycle counts meet every timing of the part.
pub fn emit_wprog_engine_soc_offchip_testbench_clocked(
    name: &str,
    n_vectors: usize,
    clks_per_bit: usize,
    max_bytes: usize,
    memory: &OffchipMemory,
    clk_period_ps: u32,
) -> String {
    let half_ns = clk_period_ps as f64 / 2000.0;
    format!(
        "{}\n{}",
        emit_wprog_engine_soc_testbench_board(name, n_vectors, clks_per_bit, max_bytes, true)
            .replace("__HALF_NS__", &format!("{half_ns}")),
        emit_psram_model_verilog(&memory.edge.part),
    )
}
