//! Packing EVERY signed-off macro into ONE Caravel shuttle submission.
//!
//! A shuttle slot buys one 2920 x 3520 um user area, and until now exactly one block has ever
//! occupied it. The other fifteen signoff-clean macros were "manufacturable" only in the sense
//! that each *could* be packaged — none was, and sixteen slots is not a plan.
//!
//! They do not need sixteen slots. Measured from their hardened LEFs, the sixteen clean macros
//! total **4.771 mm^2** against the **10.278 mm^2** user area: 46% occupancy, all of them, in one
//! submission. Every one already has a hardened `.lef`/`.gds`/`.lib` from the run its signoff
//! ledger entry vouches for, so nothing has to be re-hardened to do this — only placed, powered
//! and wired.
//!
//! WIRING IS THE REAL WORK. Caravel speaks Wishbone; these macros do not. Their ports are small
//! and regular though (3 to 18, mostly `clk`/`rst` plus a couple of data words), so the bridge is
//! mechanical: give each macro an address window, map its inputs to write registers and its
//! outputs to read registers, and share the clock and reset. That is generated here rather than
//! hand-written sixteen times, because hand-writing it sixteen times is how a submission ends up
//! instantiating a module nothing defines.

use crate::codegen_sva::pdk_target::UserArea;

/// One port of a macro, as its RTL declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacroPort {
    pub name: &'static str,
    pub width: u32,
    pub is_input: bool,
}

/// A hardened, signoff-clean macro and the footprint its LEF reports.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShuttleMacro {
    pub name: &'static str,
    /// Footprint in micrometres, from the macro's own LEF `SIZE` line — never estimated.
    pub width_um: f64,
    pub height_um: f64,
    /// Data ports, EXCLUDING `clk` and `rst`, which are driven from the Caravel harness.
    pub ports: &'static [MacroPort],
    /// Whether the macro takes a reset (some are free-running).
    pub has_rst: bool,
}

impl ShuttleMacro {
    pub fn area_mm2(&self) -> f64 {
        self.width_um * self.height_um / 1.0e6
    }
    /// 32-bit Wishbone words needed to carry every input, and every output.
    pub fn in_words(&self) -> u32 {
        self.ports
            .iter()
            .filter(|p| p.is_input)
            .map(|p| p.width.div_ceil(32))
            .sum()
    }
    pub fn out_words(&self) -> u32 {
        self.ports
            .iter()
            .filter(|p| !p.is_input)
            .map(|p| p.width.div_ceil(32))
            .sum()
    }
}

/// Where a macro sits in the user area, in LEF orientation terms.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub name: &'static str,
    pub x_um: f64,
    pub y_um: f64,
    /// Wishbone base address for this macro's register window.
    pub base: u32,
}

/// Why a set of macros cannot share one user area.
#[derive(Debug, Clone, PartialEq)]
pub enum ShuttleUnplaceable {
    /// Total footprint exceeds the vehicle even before spacing.
    AreaExceeded { needed_mm2: f64, available_mm2: f64 },
    /// One macro is individually wider or taller than the user area.
    MacroTooLarge { name: &'static str, w: f64, h: f64 },
    /// Shelf packing ran out of vertical room at the requested keep-out.
    DoesNotPack {
        placed: usize,
        total: usize,
        keepout_um: f64,
    },
    /// The register windows would overflow the address space allotted per macro.
    AddressOverflow {
        name: &'static str,
        words: u32,
        per_macro_words: u32,
    },
}

/// Micrometres of keep-out around every macro.
///
/// Macros need room for their power hookup and for signal routing to escape; abutting them makes
/// the wrapper unroutable no matter how much total area is free. 50 um is the spacing the single
/// -macro wrapper already uses successfully (`macro.cfg` places `mprj` at 60,880 inside a
/// 2920x3520 area) and is kept here rather than tightened, because a packing failure costs a
/// multi-hour route to discover.
pub const MACRO_KEEPOUT_UM: f64 = 50.0;

/// Address window per macro. 0x100 gives 64 registers — far more than the widest macro here
/// needs (18 ports), and keeps each base a readable multiple in a memory map a human debugs.
pub const MACRO_ADDR_STRIDE: u32 = 0x100;
const WB_BASE: u32 = 0x3000_0000;

/// Shelf-pack the macros into the user area, tallest first.
///
/// Tallest-first shelf packing is the right shape here and not merely convenient: the macros vary
/// by two orders of magnitude in area (0.019 to 1.140 mm^2), and packing the tall ones first keeps
/// each shelf's wasted height bounded by the shortest macro on it. At 46% occupancy this has
/// enormous slack; the planner still REFUSES rather than overlapping if it runs out.
pub fn plan_shuttle(
    macros: &[ShuttleMacro],
    area: UserArea,
) -> Result<Vec<Placement>, ShuttleUnplaceable> {
    let avail = area.mm2();
    let needed: f64 = macros.iter().map(|m| m.area_mm2()).sum();
    if needed > avail {
        return Err(ShuttleUnplaceable::AreaExceeded {
            needed_mm2: needed,
            available_mm2: avail,
        });
    }
    let (aw, ah) = (area.width_um, area.height_um);
    for m in macros {
        if m.width_um + 2.0 * MACRO_KEEPOUT_UM > aw || m.height_um + 2.0 * MACRO_KEEPOUT_UM > ah {
            return Err(ShuttleUnplaceable::MacroTooLarge {
                name: m.name,
                w: m.width_um,
                h: m.height_um,
            });
        }
    }
    for (i, m) in macros.iter().enumerate() {
        let words = m.in_words().max(m.out_words());
        let per = MACRO_ADDR_STRIDE / 4;
        if words > per {
            return Err(ShuttleUnplaceable::AddressOverflow {
                name: m.name,
                words,
                per_macro_words: per,
            });
        }
        let _ = i;
    }

    let mut order: Vec<&ShuttleMacro> = macros.iter().collect();
    order.sort_by(|a, b| b.height_um.total_cmp(&a.height_um).then(a.name.cmp(b.name)));

    let mut out = Vec::new();
    let (mut x, mut y, mut shelf_h) = (MACRO_KEEPOUT_UM, MACRO_KEEPOUT_UM, 0.0f64);
    for m in &order {
        let (w, h) = (m.width_um, m.height_um);
        if x + w + MACRO_KEEPOUT_UM > aw {
            x = MACRO_KEEPOUT_UM;
            y += shelf_h + MACRO_KEEPOUT_UM;
            shelf_h = 0.0;
        }
        if y + h + MACRO_KEEPOUT_UM > ah {
            return Err(ShuttleUnplaceable::DoesNotPack {
                placed: out.len(),
                total: macros.len(),
                keepout_um: MACRO_KEEPOUT_UM,
            });
        }
        // Address by the macro's position in the CALLER's order, so the memory map is stable
        // against any future change to the packing heuristic.
        let idx = macros
            .iter()
            .position(|c| c.name == m.name)
            .expect("macro in the input set");
        out.push(Placement {
            name: m.name,
            x_um: x,
            y_um: y,
            base: WB_BASE + (idx as u32) * MACRO_ADDR_STRIDE,
        });
        x += w + MACRO_KEEPOUT_UM;
        shelf_h = shelf_h.max(h);
    }
    Ok(out)
}

/// OpenLane `macro.cfg` lines: `<instance> <x> <y> <orientation>`.
pub fn emit_macro_cfg(placed: &[Placement]) -> String {
    let mut s = String::new();
    for p in placed {
        s.push_str(&format!("{}_i {:.0} {:.0} N\n", p.name, p.x_um, p.y_um));
    }
    s
}

/// `FP_PDN_MACRO_HOOKS` — every macro must be hooked to the user-area supply, or the power
/// connections check fails the submission outright.
pub fn emit_pdn_hooks(placed: &[Placement]) -> String {
    placed
        .iter()
        .map(|p| format!("{}_i vccd1 vssd1 vccd1 vssd1", p.name))
        .collect::<Vec<_>>()
        .join(", ")
}

const fn i(name: &'static str, width: u32) -> MacroPort {
    MacroPort {
        name,
        width,
        is_input: true,
    }
}
const fn o(name: &'static str, width: u32) -> MacroPort {
    MacroPort {
        name,
        width,
        is_input: false,
    }
}

/// Every signoff-clean macro, with the footprint its own hardened LEF reports.
///
/// Dimensions are read from `results/final/lef/<macro>.lef` of the run whose `gds_bytes` the
/// signoff ledger vouches for — the same race-free resolution the submission assembler uses, so
/// a placement can never be planned against a build that is not the one signed off.
/// `shuttle_wrapper_lock` re-derives every number here from those LEFs.
pub const CLEAN_MACROS: &[ShuttleMacro] = &[
    ShuttleMacro {
        name: "systolic_pipe8_realbist",
        width_um: 1062.6,
        height_um: 1073.3,
        ports: &[o("sig", 1)],
        has_rst: true,
    },
    ShuttleMacro {
        name: "fabric_horner_seq",
        width_um: 1028.8,
        height_um: 1039.5,
        ports: &[i("in0", 64), o("result", 64), o("done", 1)],
        has_rst: true,
    },
    ShuttleMacro {
        name: "fabric_soc",
        width_um: 783.2,
        height_um: 793.9,
        ports: &[i("in0", 64), o("result", 64), o("done", 1)],
        has_rst: true,
    },
    ShuttleMacro {
        name: "soft_core_scalar",
        width_um: 649.0,
        height_um: 659.7,
        ports: &[i("start", 1), o("result", 64), o("done", 1)],
        has_rst: true,
    },
    ShuttleMacro {
        name: "systolic_tile_realbist",
        width_um: 602.0,
        height_um: 612.7,
        ports: &[o("sig", 1)],
        has_rst: true,
    },
    ShuttleMacro {
        name: "gpu_simt",
        width_um: 591.0,
        height_um: 601.7,
        ports: &[i("in0", 64), o("result", 64), o("done", 1)],
        has_rst: true,
    },
    ShuttleMacro {
        name: "gemm_mac_realbist",
        width_um: 529.5,
        height_um: 540.2,
        ports: &[o("sig", 1)],
        has_rst: true,
    },
    ShuttleMacro {
        name: "infinichip",
        width_um: 350.0,
        height_um: 350.0,
        ports: &[i("in0", 64), o("result", 64), o("done", 1)],
        has_rst: true,
    },
    ShuttleMacro {
        name: "dpu_ingest",
        width_um: 320.0,
        height_um: 320.0,
        ports: &[i("in0", 64), o("result", 64), o("done", 1)],
        has_rst: true,
    },
    ShuttleMacro {
        name: "dram_executor_wb",
        width_um: 300.0,
        height_um: 300.0,
        ports: &[i("cmd", 64), o("la", 64)],
        has_rst: true,
    },
    ShuttleMacro {
        name: "nested_tri",
        width_um: 204.8,
        height_um: 215.5,
        ports: &[i("in0", 64), o("result", 64), o("done", 1)],
        has_rst: true,
    },
    ShuttleMacro {
        name: "cgra_sumto",
        width_um: 187.2,
        height_um: 198.0,
        ports: &[i("in0", 64), o("result", 64), o("done", 1)],
        has_rst: true,
    },
    ShuttleMacro {
        name: "decision_hft_realbist",
        width_um: 178.6,
        height_um: 189.3,
        ports: &[o("sig", 1)],
        has_rst: true,
    },
    ShuttleMacro {
        name: "decision_hft_bist",
        width_um: 159.1,
        height_um: 169.8,
        ports: &[o("sig", 1)],
        has_rst: true,
    },
    ShuttleMacro {
        name: "rev_alu_core",
        width_um: 150.9,
        height_um: 161.6,
        ports: &[i("op", 3), o("a", 32), o("b", 32)],
        has_rst: false,
    },
    ShuttleMacro {
        name: "dram_executor",
        width_um: 132.4,
        height_um: 143.1,
        ports: &[
            i("cmd_word", 64),
            i("cmd_valid", 1),
            o("issue", 1),
            o("op", 3),
            o("bank", 5),
            o("row", 32),
        ],
        has_rst: true,
    },
];
