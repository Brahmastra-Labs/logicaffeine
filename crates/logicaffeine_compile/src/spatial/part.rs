//! What a programmable part holds, as facts with sources: its logic, its flip-flops, its multipliers and every
//! kind of on-chip memory it carries, counted the way its vendor's data sheet counts them.
//!
//! A part is the budget a lowering spends and the capacity a pricer reads. Stated once, from the data sheet,
//! the same numbers let a compiler refuse a fabric whose memories or multipliers the part cannot hold before a
//! line of RTL is printed, and let the decode pricer read on-chip capacity and port bandwidth instead of
//! restating them. Each figure is a [`Fact`]: published when the table prints it, derived when it is the
//! table's arithmetic (four LUTs to a 7-series slice, 64 bits to a LUT), with the figure quoted in its cite.

use serde::Serialize;

use super::rdu_chip::{Fact, Provenance};

pub const DS180: &str = "DS180 (v1.17) 7 Series FPGAs Data Sheet: Overview";
pub const UG473: &str = "UG473 (v1.14) 7 Series FPGAs Memory Resources";
pub const UG474: &str = "UG474 (v1.8) 7 Series FPGAs Configurable Logic Block";
pub const UG479: &str = "UG479 (v1.10) 7 Series DSP48E1 Slice";
pub const DS891: &str = "DS891 (v1.11.1) Zynq UltraScale+ MPSoC Data Sheet: Overview";
pub const UG573: &str = "UG573 (v1.9) UltraScale Architecture Memory Resources";
pub const UG574: &str = "UG574 (v1.5) UltraScale Architecture Configurable Logic Block";
pub const UG579: &str = "UG579 (v1.7) UltraScale Architecture DSP Slice";
pub const S10_OVERVIEW: &str = "S10-OVERVIEW (2020.09.28) Intel Stratix 10 GX/SX Device Overview";
pub const S10_PRODUCT_TABLE: &str = "GEN-1023-1.0 (May 2015) Stratix 10 Product Table";

/// The unit a part's logic is counted in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum LogicUnit {
    /// A Xilinx six-input look-up table.
    Lut6,
    /// An Intel adaptive logic module.
    Alm,
}

/// A kind of on-chip memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum MemoryKind {
    /// Look-up tables a memory slice can run as RAM, counted in LUTs.
    LutRam,
    /// Intel's memory logic array block.
    Mlab,
    /// A Xilinx 36 Kb block RAM (RAMB36E1/E2), which also runs as two independent 18 Kb halves.
    Block36,
    /// Intel's 20 Kb block.
    M20k,
    /// A Xilinx UltraRAM (URAM288).
    Ultra,
}

impl MemoryKind {
    /// Whether the memory is a dedicated block, rather than logic the fabric would otherwise spend as gates.
    pub fn is_block(self) -> bool {
        matches!(self, MemoryKind::Block36 | MemoryKind::M20k | MemoryKind::Ultra)
    }
}

/// One kind of on-chip memory a part carries.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Memory {
    pub kind: MemoryKind,
    /// How many the part carries.
    pub count: Fact<u64>,
    /// Storage bits in one, parity bits included: a 72-bit word holds nine bytes.
    pub bits: Fact<u64>,
    /// Bits one can read in a cycle, over all of its read ports.
    pub read_bits_per_cycle: Fact<u64>,
    /// Whether the configuration bitstream sets its contents.
    pub preloaded: Fact<bool>,
}

/// What a programmable part holds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct PartResources {
    /// The device, as its vendor's tools name it without package or speed grade.
    pub part: &'static str,
    pub logic_unit: LogicUnit,
    pub logic: Fact<u64>,
    pub flops: Fact<u64>,
    pub dsp: Fact<u64>,
    /// The two's-complement operand widths one DSP block multiplies, in bits.
    pub multiplier: Fact<(u32, u32)>,
    pub memories: &'static [Memory],
    pub sources: &'static [&'static str],
}

impl PartResources {
    /// Every part the compiler knows.
    pub fn catalog() -> Vec<PartResources> {
        vec![PartResources::xc7a100t(), PartResources::xc7a200t(), PartResources::xczu7ev(), PartResources::sx2800()]
    }

    pub fn by_name(part: &str) -> Option<PartResources> {
        PartResources::catalog().into_iter().find(|p| p.part == part)
    }

    /// The Artix-7 100T: Board B's Nexys A7-100T.
    pub fn xc7a100t() -> PartResources {
        PartResources {
            part: "xc7a100t",
            logic_unit: LogicUnit::Lut6,
            logic: Fact::derived(
                63_400,
                "DS180 (v1.17) 7 Series FPGAs Data Sheet: Overview, Table 2: XC7A100T \"Slices 15,850\"; note 1 \"Each 7 series \
                 FPGA slice contains four LUTs and eight flip-flops\": 15,850 × 4",
            ),
            flops: Fact::derived(
                126_800,
                "DS180 (v1.17) 7 Series FPGAs Data Sheet: Overview, Table 2: XC7A100T \"Slices 15,850\"; note 1 \"four LUTs and \
                 eight flip-flops\": 15,850 × 8",
            ),
            dsp: Fact::published(240, "DS180 (v1.17) 7 Series FPGAs Data Sheet: Overview, Table 2: XC7A100T \"DSP48E1 Slices 240\""),
            multiplier: Fact::published((25, 18), "UG479 (v1.10) 7 Series DSP48E1 Slice: \"25 × 18 two's-complement multiplier\""),
            memories: XC7A100T_MEMORIES,
            sources: XC7_SOURCES,
        }
    }

    /// The Artix-7 200T: Board A's Nexys Video.
    pub fn xc7a200t() -> PartResources {
        PartResources {
            part: "xc7a200t",
            logic_unit: LogicUnit::Lut6,
            logic: Fact::derived(
                134_600,
                "DS180 (v1.17) 7 Series FPGAs Data Sheet: Overview, Table 2: XC7A200T \"Slices 33,650\"; note 1 \"Each 7 series \
                 FPGA slice contains four LUTs and eight flip-flops\": 33,650 × 4",
            ),
            flops: Fact::derived(
                269_200,
                "DS180 (v1.17) 7 Series FPGAs Data Sheet: Overview, Table 2: XC7A200T \"Slices 33,650\"; note 1 \"four LUTs and \
                 eight flip-flops\": 33,650 × 8",
            ),
            dsp: Fact::published(740, "DS180 (v1.17) 7 Series FPGAs Data Sheet: Overview, Table 2: XC7A200T \"DSP48E1 Slices 740\""),
            multiplier: Fact::published((25, 18), "UG479 (v1.10) 7 Series DSP48E1 Slice: \"25 × 18 two's-complement multiplier\""),
            memories: XC7A200T_MEMORIES,
            sources: XC7_SOURCES,
        }
    }

    /// The Zynq UltraScale+ ZU7EV: the ZCU104's programmable logic.
    pub fn xczu7ev() -> PartResources {
        PartResources {
            part: "xczu7ev",
            logic_unit: LogicUnit::Lut6,
            logic: Fact::published(
                230_400,
                "DS891 (v1.11.1) Zynq UltraScale+ MPSoC Data Sheet: Overview, Table 5: EV Device Feature Summary, ZU7EV \"CLB LUTs \
                 230,400\"",
            ),
            flops: Fact::published(
                460_800,
                "DS891 (v1.11.1) Zynq UltraScale+ MPSoC Data Sheet: Overview, Table 5: EV Device Feature Summary, ZU7EV \"CLB \
                 Flip-Flops 460,800\"",
            ),
            dsp: Fact::published(
                1_728,
                "DS891 (v1.11.1) Zynq UltraScale+ MPSoC Data Sheet: Overview, Table 5: EV Device Feature Summary, ZU7EV \"DSP Slices \
                 1,728\"",
            ),
            multiplier: Fact::published(
                (27, 18),
                "UG579 (v1.7) UltraScale Architecture DSP Slice: \"27 × 18 two's complement multiplier with dynamic bypass\"",
            ),
            memories: XCZU7EV_MEMORIES,
            sources: ZYNQ_US_PLUS_SOURCES,
        }
    }

    /// The Stratix 10 SX 2800: the Intel PAC D5005's 1SX280HN2F43E2VG. The device overview counts its logic in
    /// thousands of logic elements; the product table prints its ALMs and their registers.
    pub fn sx2800() -> PartResources {
        PartResources {
            part: "1sx280",
            logic_unit: LogicUnit::Alm,
            logic: Fact::published(
                933_120,
                "GEN-1023-1.0 (May 2015) Stratix 10 Product Table: GX 2800 / SX 2800 \"Adaptive logic modules (ALMs) 933,120\"",
            ),
            flops: Fact::published(
                3_732_480,
                "GEN-1023-1.0 (May 2015) Stratix 10 Product Table: GX 2800 / SX 2800 \"ALM registers 3,732,480\"; S10-OVERVIEW \
                 (2020.09.28) Intel Stratix 10 GX/SX Device Overview: \"4 Registers per ALM\"",
            ),
            dsp: Fact::published(
                5_760,
                "GEN-1023-1.0 (May 2015) Stratix 10 Product Table: GX 2800 / SX 2800 \"Variable-precision digital signal processing \
                 (DSP) blocks 5,760\", \"18 x 19 multipliers 11,520\"",
            ),
            multiplier: Fact::published(
                (18, 19),
                "S10-OVERVIEW (2020.09.28) Intel Stratix 10 GX/SX Device Overview: \"Each DSP block can be independently configured \
                 at compile time as either dual 18x19 or a single 27x27 multiply accumulate\"; the array multiplies on the two \
                 18x19 halves (Table 4: \"18x19 Multipliers 11,520\", note 2: \"The number of 27x27 multipliers is one-half the \
                 number of 18x19 multipliers\")",
            ),
            memories: SX2800_MEMORIES,
            sources: S10_SOURCES,
        }
    }

    /// The memory of `kind` the part carries.
    pub fn memory(&self, kind: MemoryKind) -> Option<&'static Memory> {
        self.memories.iter().find(|m| m.kind == kind)
    }

    /// Bytes the part's dedicated memory blocks store, all of their bits counted.
    pub fn block_bytes(&self) -> u64 {
        self.memories.iter().filter(|m| m.kind.is_block()).map(|m| m.count.value * m.bits.value / 8).sum()
    }

    /// Bytes the part's dedicated memory blocks read in one cycle, every read port busy.
    pub fn block_read_bytes_per_cycle(&self) -> u64 {
        self.memories
            .iter()
            .filter(|m| m.kind.is_block())
            .map(|m| m.count.value * m.read_bits_per_cycle.value / 8)
            .sum()
    }

    /// Every fact the part carries, with the path a lock names it by.
    pub fn facts(&self) -> Vec<(String, Provenance, &'static str)> {
        let mut out = vec![
            ("logic".to_string(), self.logic.provenance, self.logic.cite),
            ("flops".to_string(), self.flops.provenance, self.flops.cite),
            ("dsp".to_string(), self.dsp.provenance, self.dsp.cite),
            ("multiplier".to_string(), self.multiplier.provenance, self.multiplier.cite),
        ];
        for m in self.memories {
            let at = |field: &str| format!("memory.{:?}.{field}", m.kind);
            out.push((at("count"), m.count.provenance, m.count.cite));
            out.push((at("bits"), m.bits.provenance, m.bits.cite));
            out.push((at("read_bits_per_cycle"), m.read_bits_per_cycle.provenance, m.read_bits_per_cycle.cite));
            out.push((at("preloaded"), m.preloaded.provenance, m.preloaded.cite));
        }
        out
    }
}

const XC7_SOURCES: &[&str] = &[DS180, UG473, UG474, UG479];
const S10_SOURCES: &[&str] = &[S10_OVERVIEW, S10_PRODUCT_TABLE];
const ZYNQ_US_PLUS_SOURCES: &[&str] = &[DS891, UG573, UG574, UG579];

const RAMB36E1_BITS: Fact<u64> = Fact::derived(
    36_864,
    "UG473 (v1.14) 7 Series FPGAs Memory Resources: a 36 Kb block RAM is \"32K x 1, 16K x 2, 8K x 4, 4K x 9, 2K x 18, 1K x 36, \
     or 512 x 72 in simple dual-port mode\": 512 × 72",
);
const RAMB36E1_READ: Fact<u64> = Fact::derived(
    72,
    "UG473 (v1.14) 7 Series FPGAs Memory Resources: \"512 x 72 in simple dual-port mode\", or two 1K x 36 ports in true \
     dual-port mode: 72 bits a cycle either way",
);
const RAMB36E1_PRELOADED: Fact<bool> = Fact::published(
    true,
    "UG473 (v1.14) 7 Series FPGAs Memory Resources: \"memory content can be initialized or cleared by the configuration \
     bitstream\"",
);
const XC7_LUT_BITS: Fact<u64> = Fact::published(
    64,
    "UG474 (v1.8) 7 Series FPGAs Configurable Logic Block: SLICEMs \"can also use their LUTs as distributed 64-bit RAM\"",
);
const XC7_LUT_READ: Fact<u64> = Fact::derived(
    1,
    "UG474 (v1.8) 7 Series FPGAs Configurable Logic Block: a LUT of \"distributed 64-bit RAM\" read as 64 x 1 is one bit a \
     cycle per read port",
);
const XC7_LUT_PRELOADED: Fact<bool> = Fact::derived(
    true,
    "UG474 (v1.8) 7 Series FPGAs Configurable Logic Block: distributed RAM is the LUT's own 64 configuration bits, which the \
     bitstream writes",
);

const XC7A100T_MEMORIES: &[Memory] = &[
    Memory {
        kind: MemoryKind::LutRam,
        count: Fact::derived(
            19_008,
            "DS180 (v1.17) 7 Series FPGAs Data Sheet: Overview, Table 2: XC7A100T \"Max Distributed RAM (Kb) 1,188\": 1,188 × 1024 \
             / 64 bits a LUT",
        ),
        bits: XC7_LUT_BITS,
        read_bits_per_cycle: XC7_LUT_READ,
        preloaded: XC7_LUT_PRELOADED,
    },
    Memory {
        kind: MemoryKind::Block36,
        count: Fact::published(
            135,
            "DS180 (v1.17) 7 Series FPGAs Data Sheet: Overview, Table 2: XC7A100T \"Block RAM Blocks 18 Kb 270, 36 Kb 135\"",
        ),
        bits: RAMB36E1_BITS,
        read_bits_per_cycle: RAMB36E1_READ,
        preloaded: RAMB36E1_PRELOADED,
    },
];

const XC7A200T_MEMORIES: &[Memory] = &[
    Memory {
        kind: MemoryKind::LutRam,
        count: Fact::derived(
            46_208,
            "DS180 (v1.17) 7 Series FPGAs Data Sheet: Overview, Table 2: XC7A200T \"Max Distributed RAM (Kb) 2,888\": 2,888 × 1024 \
             / 64 bits a LUT",
        ),
        bits: XC7_LUT_BITS,
        read_bits_per_cycle: XC7_LUT_READ,
        preloaded: XC7_LUT_PRELOADED,
    },
    Memory {
        kind: MemoryKind::Block36,
        count: Fact::published(
            365,
            "DS180 (v1.17) 7 Series FPGAs Data Sheet: Overview, Table 2: XC7A200T \"Block RAM Blocks 18 Kb 730, 36 Kb 365\"",
        ),
        bits: RAMB36E1_BITS,
        read_bits_per_cycle: RAMB36E1_READ,
        preloaded: RAMB36E1_PRELOADED,
    },
];

const XCZU7EV_MEMORIES: &[Memory] = &[
    Memory {
        kind: MemoryKind::LutRam,
        count: Fact::derived(
            102_400,
            "DS891 (v1.11.1) Zynq UltraScale+ MPSoC Data Sheet: Overview, Table 5: ZU7EV \"Distributed RAM (Mb) 6.2\", an upper \
             bound from the top of its rounding: 6.25 × 2^20 / 64 bits a LUT (UG574 (v1.5) UltraScale Architecture Configurable \
             Logic Block: \"distributed 64-bit RAM\")",
        ),
        bits: Fact::published(
            64,
            "UG574 (v1.5) UltraScale Architecture Configurable Logic Block: \"A SLICEM (memory) can also use the LUTs as \
             distributed 64-bit RAM\"",
        ),
        read_bits_per_cycle: Fact::derived(
            1,
            "UG574 (v1.5) UltraScale Architecture Configurable Logic Block: \"The LUT can be configured as either a 64 x 1 or 32 \
             x 2 memory\": one bit a cycle per read port at full depth",
        ),
        preloaded: Fact::derived(
            true,
            "UG574 (v1.5) UltraScale Architecture Configurable Logic Block: distributed RAM is the LUT's own 64 configuration bits, \
             which the bitstream writes",
        ),
    },
    Memory {
        kind: MemoryKind::Block36,
        count: Fact::published(
            312,
            "DS891 (v1.11.1) Zynq UltraScale+ MPSoC Data Sheet: Overview, Table 5: EV Device Feature Summary, ZU7EV \"Block RAM \
             Blocks 312\"",
        ),
        bits: Fact::derived(
            36_864,
            "UG573 (v1.9) UltraScale Architecture Memory Resources: \"a 36 Kb block RAM can additionally be configured with a port \
             width of 512 x 72 bits\": 512 × 72",
        ),
        read_bits_per_cycle: Fact::derived(
            72,
            "UG573 (v1.9) UltraScale Architecture Memory Resources: \"512 x 72 bits (when used as simple dual-port (SDP) memory)\", \
             or two 1K x 36 read ports in true dual-port mode: 72 bits a cycle either way",
        ),
        preloaded: Fact::published(
            true,
            "UG573 (v1.9) UltraScale Architecture Memory Resources: \"The memory content can be initialized or cleared by the \
             configuration bitstream\"",
        ),
    },
    Memory {
        kind: MemoryKind::Ultra,
        count: Fact::published(
            96,
            "DS891 (v1.11.1) Zynq UltraScale+ MPSoC Data Sheet: Overview, Table 5: EV Device Feature Summary, ZU7EV \"UltraRAM \
             Blocks 96\"",
        ),
        bits: Fact::derived(
            294_912,
            "UG573 (v1.9) UltraScale Architecture Memory Resources: \"Each UltraRAM block can store up to 288K bits of data and is \
             configured as a 4K x 72 memory block\": 4096 × 72",
        ),
        read_bits_per_cycle: Fact::derived(
            144,
            "UG573 (v1.9) UltraScale Architecture Memory Resources: \"UltraRAM has two ports, both of which address all 4K x 72 \
             bits. Each port can independently perform either one read or one write operation per clock cycle\": 2 × 72",
        ),
        preloaded: Fact::published(
            false,
            "UG573 (v1.9) UltraScale Architecture Memory Resources: \"There is no user defined INIT attribute and therefore the \
             content of the SRAM array cannot be initialized to user defined values\"",
        ),
    },
];

const SX2800_MEMORIES: &[Memory] = &[
    Memory {
        kind: MemoryKind::Mlab,
        count: Fact::published(
            23_796,
            "S10-OVERVIEW (2020.09.28) Intel Stratix 10 GX/SX Device Overview, Table 4: GX 2800/ SX 2800 \"MLAB Counts 23,796\"",
        ),
        bits: Fact::published(
            640,
            "S10-OVERVIEW (2020.09.28) Intel Stratix 10 GX/SX Device Overview: \"two types of embedded memory blocks: M20K (20 \
             Kb) and MLAB (640 bit)\"",
        ),
        read_bits_per_cycle: Fact::derived(
            20,
            "S10-OVERVIEW (2020.09.28) Intel Stratix 10 GX/SX Device Overview, Table 10: MLAB \"32 x 20\", its widest port",
        ),
        preloaded: Fact::derived(
            true,
            "S10-OVERVIEW (2020.09.28) Intel Stratix 10 GX/SX Device Overview: \"Both M20K and MLAB embedded memory blocks can be \
             configured as a single-port or dual-port RAM, FIFO, ROM, or shift register\"; a ROM's contents come from the \
             bitstream",
        ),
    },
    Memory {
        kind: MemoryKind::M20k,
        count: Fact::published(
            11_721,
            "S10-OVERVIEW (2020.09.28) Intel Stratix 10 GX/SX Device Overview, Table 4: GX 2800/ SX 2800 \"M20K Blocks 11,721\", \
             \"M20K Mbits 229\"",
        ),
        bits: Fact::derived(
            20_480,
            "S10-OVERVIEW (2020.09.28) Intel Stratix 10 GX/SX Device Overview, Table 10: M20K (20 Kb) \"512 x 40 (or x32)\": 512 \
             × 40",
        ),
        read_bits_per_cycle: Fact::derived(
            40,
            "S10-OVERVIEW (2020.09.28) Intel Stratix 10 GX/SX Device Overview, Table 10: M20K \"2K x 10 (or x8)\", \"1K x 20 (or \
             x16)\", \"512 x 40 (or x32)\": 40 bits a cycle at its widest",
        ),
        preloaded: Fact::derived(
            true,
            "S10-OVERVIEW (2020.09.28) Intel Stratix 10 GX/SX Device Overview: \"Both M20K and MLAB embedded memory blocks can be \
             configured as a single-port or dual-port RAM, FIFO, ROM, or shift register\"; a ROM's contents come from the \
             bitstream",
        ),
    },
];
