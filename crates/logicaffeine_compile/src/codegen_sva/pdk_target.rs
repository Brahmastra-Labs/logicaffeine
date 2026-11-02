//! `PdkTarget` — the PROCESS a design is built out of, as data.
//!
//! [`super::fabric_target::FabricTarget`] describes an ARCHITECTURE (the grid you place onto).
//! This describes an IMPLEMENTATION (what the grid is made of): the standard-cell library, the
//! metal stack, the shuttle vehicle's fixed geometry, the SRAM macros actually available, and the
//! golden DRC deck. They are deliberately separate types — the same fabric is placeable on several
//! processes, and the same process carries many fabrics.
//!
//! WHY THIS EXISTS. Every PDK fact in this project used to be a literal in a shell script:
//! `sky130A_mr.drc` in one, `"0 0 2920 3520"` in another, `"0.99,$X,0,0,0,0"` (a six-column
//! `li1,met1..met5` vector) in a third, and `contains("sky130")` inside four Rust locks that would
//! have actively REJECTED a GF180 ledger. Bringing up a second process meant editing eight files
//! and hoping. The acceptance test for this module is blunt: **adding a PDK must be adding one
//! value here, and nothing else.** If a script or a lock has to change, the abstraction leaked.
//!
//! The shape follows [`crate::codegen_ana::target::AnalogTarget`]: every quantity carries its
//! `provenance`, and asking for something the process cannot do returns a typed refusal rather
//! than a silently-wrong number.

use std::fmt;

/// A standard-cell SRAM macro the PDK actually ships, with the facts that decide whether a design
/// can use it.
///
/// `corners` is not decoration. Of sky130's four macros, three ship **only** `TT_1p8V_25C`, and
/// the signoff recorder gates on `multi_corner` — so a design built from those three cannot
/// honestly claim a multi-corner signoff no matter how clean its logic is. That fact belongs next
/// to the macro, not in someone's memory.
// No `Eq`: these carry LEF dimensions as `f64`, and `f64` is `PartialEq` but not `Eq`. The values
// are exact decimals in the LEF (683.10 x 416.54), so integer nanometres would be the more honest
// representation and would restore `Eq` — worth doing if these ever need to be a map key or
// deduplicated. Nothing needs that today, and float equality on a spec sheet is not a comparison
// anyone should be making.
#[derive(Debug, Clone, PartialEq)]
pub struct SramMacro {
    pub name: &'static str,
    /// Addressable words.
    pub words: u32,
    /// Bits per word — the macro's data width. A design whose arrays are this wide needs no
    /// width-banking, which is most of what makes a memory plan cheap or expensive.
    pub bits: u32,
    /// Physical size in micrometres, from the macro's own LEF `SIZE` line.
    pub width_um: f64,
    pub height_um: f64,
    /// Read/write ports, e.g. "1rw1r". An array read every cycle against writes in the same block
    /// needs a dual-port macro; a single-port one cannot express it at any speed.
    pub ports: &'static str,
    /// Liberty corners shipped. Fewer than a full set caps what the design can claim.
    pub corners: &'static [&'static str],
}

impl SramMacro {
    pub fn area_mm2(&self) -> f64 {
        self.width_um * self.height_um / 1.0e6
    }
    pub fn capacity_bits(&self) -> u64 {
        u64::from(self.words) * u64::from(self.bits)
    }
    /// Whether this macro can carry a multi-corner signoff claim. A single shipped corner cannot:
    /// slow and fast silicon are simply not characterized.
    pub fn is_multi_corner(&self) -> bool {
        self.corners.len() > 1
    }
}

/// A memory the design needs, expressed the way RTL expresses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryRequest {
    pub words: u32,
    pub bits: u32,
    /// Set when the array is read and written in the same cycle (the common case for an engine
    /// that reads unconditionally at the top of its always block).
    pub dual_port: bool,
    /// Set when the design intends to claim multi-corner signoff over this memory.
    pub needs_multi_corner: bool,
}

/// Measured standard-cell density on this flow: 173,895 cells/mm^2, the median across the twelve
/// sky130 blocks in `benchmarks/silicon/sky130_signoff*.json` (range 121k-224k).
pub const MEASURED_CELLS_PER_MM2: f64 = 173_895.0;

/// Silicon area per stored bit when an array is built from FLIP-FLOPS, in um^2.
///
/// Measured from the sky130 library's own LEF, not estimated: `dfxtp_1` is 20.019 um^2, plus the
/// access logic every addressed array needs — a `mux2_1` read path at 11.261 and roughly a
/// `nand2_1` of write decode at 3.754 — is 35.03 um^2 of CELL per bit. Standard cells do not fill
/// their area: these blocks place at FP_CORE_UTIL 30-40, so the die cost is that over ~0.40.
///
/// The comparison that matters: this is ~87 um^2/bit against a macro's 17.4-20.5 um^2/bit, where
/// the macro figure is its true LEF footprint with its own decode already inside. **SRAM is about
/// four times denser than flops.** An earlier version of this constant was expressed in generic
/// "cells" and made flops look CHEAPER than macros, which put a 512 kbit heap in flip-flops at
/// 36 mm^2 and called it the smaller option.
pub const FLOP_BIT_UM2: f64 = (20.019 + 11.261 + 3.754) / 0.40;

/// How a [`MemoryRequest`] is best built. NOT always a macro — and getting this wrong is
/// expensive in both directions.
///
/// An SRAM macro has a MINIMUM depth (256 words on sky130's smallest), so a shallow array pays for
/// a whole macro to store a fraction of one. The WPE's `blk_*` arrays are 13 x 160 words: as
/// macros that is 13 instances and 2.18 mm^2 to hold 66 kbit, which is worse than flops AND drags
/// in port constraints the flops would not have. Conversely the heap at 2.1 Mbit would be ~36 mm^2
/// of flops and has to be a macro. The planner must pick, not assume.
#[derive(Debug, Clone, PartialEq)]
pub enum Implementation {
    /// Flip-flops and their access logic — no macro, no port limit, no corner problem.
    Flops { cells: u64 },
    /// One or more SRAM macro instances.
    Macros {
        macro_name: &'static str,
        depth_banks: u32,
        width_banks: u32,
        instances: u32,
    },
}

/// How a [`MemoryRequest`] maps onto real macros.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryPlan {
    /// Flops or macros, whichever is smaller on this process.
    pub implementation: Implementation,
    pub macro_name: &'static str,
    /// Instances along the address axis.
    pub depth_banks: u32,
    /// Instances along the data axis. More than 1 means the array is wider than the macro and must
    /// be split — every access then drives several macros.
    pub width_banks: u32,
    pub instances: u32,
    pub area_mm2: f64,
}

/// Why a process cannot build what was asked for. Typed, so a caller cannot mistake a refusal for
/// a plan — the mistake that produces a design nobody can fabricate.
// No `Eq` — `ExceedsUserArea` carries `f64` areas. See the note on `SramMacro`.
#[derive(Debug, Clone, PartialEq)]
pub enum PdkUnsupported {
    /// The process ships no SRAM macro at all; the memory would have to be flip-flops.
    NoSramMacros { pdk: &'static str },
    /// No macro offers the required port count.
    NoDualPortMacro { pdk: &'static str },
    /// A multi-corner claim was requested but every usable macro ships a single corner.
    NoMultiCornerMacro {
        pdk: &'static str,
        best: &'static str,
    },
    /// The plan does not fit the vehicle's user area.
    ExceedsUserArea { needed_mm2: f64, available_mm2: f64 },
    /// A user macro for this process's vehicle was requested, but the vehicle's power nets and the
    /// layer ceiling a macro must stay under have not been read here.
    VehicleUncharacterized { pdk: &'static str },
}

impl fmt::Display for PdkUnsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSramMacros { pdk } => {
                write!(
                    f,
                    "{pdk} ships no SRAM macro — this memory can only be flip-flops"
                )
            }
            Self::NoDualPortMacro { pdk } => write!(
                f,
                "{pdk} ships no dual-port SRAM macro, but the array is read and written in the \
                 same cycle"
            ),
            Self::NoMultiCornerMacro { pdk, best } => write!(
                f,
                "{pdk}: a multi-corner signoff was requested, but the widest usable macro ({best}) \
                 ships a single Liberty corner — slow and fast silicon are not characterized"
            ),
            Self::ExceedsUserArea {
                needed_mm2,
                available_mm2,
            } => write!(
                f,
                "the memory plan needs {needed_mm2:.2} mm² of macro against a {available_mm2:.2} \
                 mm² user area, before any logic"
            ),
            Self::VehicleUncharacterized { pdk } => write!(
                f,
                "{pdk}: the shuttle vehicle's user-macro power nets and routing ceiling have not \
                 been read — refusing to guess them"
            ),
        }
    }
}

/// The fixed geometry of a shuttle vehicle. `None` for a standalone die, where the user chooses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UserArea {
    pub width_um: f64,
    pub height_um: f64,
    /// The power and ground nets the vehicle's harness provides to a user macro, when read.
    pub macro_power: Option<(&'static str, &'static str)>,
    /// The highest layer a user macro with its own grid may route on, so the wrapper's power grid can pass over it.
    pub macro_max_layer: Option<&'static str>,
    /// The highest layer a user macro fed by the wrapper's grid may route on: its own strap layer.
    pub fed_macro_max_layer: Option<&'static str>,
    /// How the harness times a user macro, when read.
    pub harness: Option<HarnessTiming>,
}

/// The vehicle's timing as efabless characterizes it: what a user macro's ports see from the harness, and the clock
/// model the whole chip is signed off with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HarnessTiming {
    /// The earliest the harness presents each synchronous user-area input after the clock edge, in ns: the
    /// `set_input_delay -min` a user macro's own ports are held to, so its hold repair closes the port-to-flop
    /// paths the wrapper would otherwise see violated.
    pub input_min_ns: &'static [(&'static str, f64)],
    /// The clock uncertainty the chip is signed off with, in ns.
    pub clock_uncertainty_ns: f64,
    /// The on-chip-variation derate the chip is signed off with: early paths at `1 - d`, late paths at `1 + d`.
    pub timing_derate: f64,
    /// The period the chip signs its core clock off at, in ns: [`HarnessClock::Core`], which also runs the
    /// management SoC and the bus.
    pub core_clock_period_ns: f64,
    /// The user-area ports the harness times against its core clock, inputs and outputs: the bus, its reset and the
    /// logic analyzer. A macro whose logic runs on another clock still times these against [`HarnessClock::Core`].
    pub core_clock_inputs: &'static [&'static str],
    pub core_clock_outputs: &'static [&'static str],
}

impl HarnessTiming {
    /// The chip's signoff clock as `NAME=value` lines for the flow scripts to `eval`.
    pub fn shell(&self) -> String {
        format!(
            "CV_CLOCK_UNCERTAINTY_NS={}\nCV_TIMING_DERATE={}\nCV_CORE_CLOCK_PERIOD_NS={}\n",
            self.clock_uncertainty_ns, self.timing_derate, self.core_clock_period_ns
        )
    }
}

/// The clocks the vehicle's harness gives a user macro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessClock {
    /// `wb_clk_i`: the core clock, which also runs the management SoC and the bus, signed off with the chip at
    /// [`HarnessTiming::core_clock_period_ns`].
    Core,
    /// `user_clock2`: Caravel's PLL 90-degree phase through a divider of its own (`caravel_clocking.v`, housekeeping's
    /// `pll90_sel`), set apart from the core clock's (`pll_sel`), so a macro on it runs at a period of its own.
    User2,
}

impl HarnessClock {
    /// The user-area port the clock enters on.
    pub fn port(self) -> &'static str {
        match self {
            HarnessClock::Core => "wb_clk_i",
            HarnessClock::User2 => "user_clock2",
        }
    }

    pub fn by_port(port: &str) -> Option<HarnessClock> {
        [HarnessClock::Core, HarnessClock::User2]
            .into_iter()
            .find(|c| c.port() == port)
    }
}

impl UserArea {
    pub fn mm2(&self) -> f64 {
        self.width_um * self.height_um / 1.0e6
    }
    /// The OpenLane `DIE_AREA` string for this vehicle.
    pub fn die_area(&self) -> String {
        format!("0 0 {} {}", self.width_um, self.height_um)
    }
}

/// One process, as data.
#[derive(Debug, Clone, PartialEq)]
pub struct PdkTarget {
    /// The PDK variant exactly as `ciel`/open_pdks names it — this is the string that must appear
    /// in a run's `config.tcl` and therefore in the signoff ledger.
    pub name: &'static str,
    /// Drawn feature size in nanometres. Useful for first-order area scaling and nothing else.
    pub node_nm: u32,
    pub std_cell_library: &'static str,
    /// Routing layers bottom-to-top, in the names the tools use. sky130 says `li1,met1..met5`;
    /// GF180 says `Metal1..Metal5` and has no `li1`. A six-column layer-adjustment vector written
    /// for one is silently wrong on the other, which is exactly the bug this field removes.
    pub metal_stack: &'static [&'static str],
    /// The LOCAL-INTERCONNECT layer, when the process has one. sky130 has `li1` below met1; GF180
    /// does not. This is a ROLE, not a name, and conflating the two is a live bug: a layer vector
    /// that hard-penalises "the lowest layer" is right on sky130 (li1 is not for routing) and
    /// wrong on GF180, where the lowest layer IS the first real metal and hard-penalising it
    /// strangles the router. Caught by emitting for both processes and reading the result.
    pub local_interconnect: Option<&'static str>,
    /// Highest layer signal routing may use on this vehicle.
    pub top_routing_layer: &'static str,
    /// This process's clock-root buffer and its output pin — the cell that should be declared as
    /// the CLOCK input port's driver, when the process has been characterized here.
    ///
    /// OpenLane models the clock port's drive with `SYNTH_CLK_DRIVING_CELL`, which DEFAULTS TO
    /// `SYNTH_DRIVING_CELL` — the minimum data-input inverter (`sky130_fd_sc_hd__inv_2`). A clock
    /// port is not driven by a minimum logic inverter on any real die; it arrives from a pad, a
    /// PLL, or (on Caravel) the management SoC's clock distribution. The pessimism lands on the
    /// ONE net a design cannot repair, because an input port's transition is an ASSUMPTION rather
    /// than something a resizer can buffer. Measured: `systolic_os_pipe8_realbist` closed every
    /// other gate and signed off with exactly three max-slew violations, and all three were that
    /// single net — `clk`, `clkbuf_0_clk/A`, and a diode sitting on it, each 0.97 against a 0.75
    /// limit. `systolic_pipe16_realbist` shows the identical triple at 1.25.
    ///
    /// The value is not invented: it is the process's OWN `ROOT_CLK_BUFFER`, read from its
    /// `libs.tech/openlane/<scl>/config.tcl`. `None` means this process has not been installed and
    /// read here, and the flow then emits nothing rather than guessing a cell name that may not
    /// exist in the library — a wrong name fails the flow, and a plausible wrong one is worse.
    ///
    /// (This cannot be expressed as `ref::$ROOT_CLK_BUFFER` in config.json: design-config
    /// references resolve BEFORE the PDK config is sourced, so the variable is not yet defined and
    /// OpenLane exits with `Referenced variable 'ROOT_CLK_BUFFER' not found`.)
    pub clk_driving_cell: Option<(&'static str, &'static str)>,
    /// The efabless/foundry maximal-rule deck `mpw_precheck` accepts a submission against.
    pub golden_drc_deck: &'static str,
    /// The shuttle vehicle's fixed user area, when there is one.
    pub user_area: Option<UserArea>,
    /// SRAM macros the PDK ships, widest-capacity first.
    pub sram_macros: &'static [SramMacro],
    /// How many of `mpw_precheck`'s checks are scheduled for this PDK. It is NOT the same number
    /// for every process — gf180 drops six that sky130 runs (magic_drc, zeroarea, consistency,
    /// illegal_cellname, pin_label, metalcheck) and adds pdnmulti. "19/19" and "13/13" are not the
    /// same assurance, and a claim built on one must not be read as the other.
    pub precheck_checks: u32,
    /// Per-layer wire RC for the PRE-ROUTE parasitic estimate (`LAYERS_RC`), calibrated against
    /// this project's own extraction — or `None` where no process measurement exists, in which
    /// case nothing is emitted rather than another process's numbers borrowed.
    ///
    /// Every resizer and pre-route STA step sizes drivers against this estimate, so its error
    /// becomes the gap signoff later finds. See [`SKY130A_LAYER_RC`] for the measurement.
    pub estimator_layer_rc: Option<&'static [LayerRc]>,
    /// Where these numbers come from, so a reader can check them rather than trust them.
    pub provenance: &'static str,
}

/// One routing layer's wire RC per unit length, as `set_layer_rc` takes it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerRc {
    pub layer: &'static str,
    pub cap_pf_per_um: f64,
    pub res_kohm_per_um: f64,
}

/// sky130A signal-layer RC for the pre-route estimate.
///
/// CAPACITANCE is FITTED: per-net routed length on each layer (the detailed route of
/// `systolic_pipe16_realbist`, RUN_2026.09.23_03.24.05) regressed against that net's OpenRCX wire
/// capacitance (nominal corner, `rules.openrcx.sky130A.nom.calibre`), over 23,557 nets of 20 um
/// or more. The fit reproduces extraction with a 7.3% median error and 0.8% in total. OpenLane
/// ships no `LAYERS_RC`, and the uncalibrated estimate it fell back to was optimistic by a median
/// 1.39x (2.02x at p90) on the 30-80 fF nets that fail slew; with this table those estimate at
/// 0.95x (p10 0.86, p90 1.07).
///
/// RESISTANCE is the tech LEF's own `RESISTANCE RPERSQ` over the layer's minimum `WIDTH`
/// (`sky130_fd_sc_hd__nom.tlef`), e.g. met1 0.125 ohm/sq / 0.14 um.
pub const SKY130A_LAYER_RC: &[LayerRc] = &[
    LayerRc {
        layer: "met1",
        cap_pf_per_um: 1.6116e-4,
        res_kohm_per_um: 8.9286e-4,
    },
    LayerRc {
        layer: "met2",
        cap_pf_per_um: 1.4460e-4,
        res_kohm_per_um: 8.9286e-4,
    },
    LayerRc {
        layer: "met3",
        cap_pf_per_um: 1.8581e-4,
        res_kohm_per_um: 1.5667e-4,
    },
    LayerRc {
        layer: "met4",
        cap_pf_per_um: 1.6682e-4,
        res_kohm_per_um: 1.5667e-4,
    },
    LayerRc {
        layer: "met5",
        cap_pf_per_um: 2.4831e-4,
        res_kohm_per_um: 1.7812e-5,
    },
];

impl PdkTarget {
    /// Every process this project can currently build against.
    pub fn all() -> &'static [PdkTarget] {
        &[SKY130A, GF180MCUC]
    }

    /// Look a process up by the name a run's `config.tcl` records.
    pub fn by_name(name: &str) -> Option<&'static PdkTarget> {
        Self::all().iter().find(|p| p.name == name)
    }

    /// The first layer signal routing should actually use: the one above local interconnect where
    /// the process has it, otherwise the bottom of the stack.
    pub fn first_signal_layer(&self) -> &'static str {
        match self.local_interconnect {
            Some(li) => self
                .metal_stack
                .iter()
                .find(|l| **l != li)
                .copied()
                .unwrap_or(li),
            None => self.metal_stack[0],
        }
    }

    /// The `GRT_LAYER_ADJUSTMENTS` vector — one column per routing layer, so the string can never
    /// have the wrong count for the process, and assigned by ROLE so it can never have the right
    /// count with the wrong meaning.
    ///
    /// `local_penalty` applies to local interconnect (which is not for routing at all), and
    /// `first_metal_penalty` to the lowest real signal layer (the antenna lever). On a process
    /// with no local interconnect only the second applies — the bug this replaced put the
    /// local-interconnect penalty on GF180's `Metal1` and silently dropped the antenna one.
    pub fn layer_adjustments(&self, local_penalty: f64, first_metal_penalty: f64) -> String {
        let first = self.first_signal_layer();
        self.metal_stack
            .iter()
            .map(|l| {
                let v = if Some(*l) == self.local_interconnect {
                    local_penalty
                } else if *l == first {
                    first_metal_penalty
                } else {
                    0.0
                };
                format!("{v}")
            })
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Plan a memory onto this process's macros, or refuse with a reason.
    ///
    /// Refusing is the point. A design that silently "fits" because nobody checked the port count
    /// or the corner set is a design that fails at signoff, or worse, on silicon.
    pub fn plan_memory(&self, req: MemoryRequest) -> Result<MemoryPlan, PdkUnsupported> {
        // Flops are always available: no macro, no port limit, no corner gap. For a shallow array
        // they are also SMALLER, because a macro cannot be shorter than its minimum depth.
        let bits = u64::from(req.words) * u64::from(req.bits);
        let flop_area = bits as f64 * FLOP_BIT_UM2 / 1.0e6;
        let flop_cells = (flop_area * MEASURED_CELLS_PER_MM2) as u64;
        let flop_plan = MemoryPlan {
            implementation: Implementation::Flops { cells: flop_cells },
            macro_name: "(flops)",
            depth_banks: 0,
            width_banks: 0,
            instances: 0,
            area_mm2: flop_area,
        };

        if self.sram_macros.is_empty() {
            // Not a refusal: an array with no macro available is simply built from flops, and
            // only the VEHICLE can refuse it.
            return self.fits_vehicle(flop_plan);
        }
        let dual = |m: &SramMacro| m.ports.contains("1rw1r") || m.ports.contains("2rw");
        let usable: Vec<&SramMacro> = self
            .sram_macros
            .iter()
            .filter(|m| !req.dual_port || dual(m))
            .collect();
        if usable.is_empty() {
            return Err(PdkUnsupported::NoDualPortMacro { pdk: self.name });
        }
        let corner_ok: Vec<&&SramMacro> = usable
            .iter()
            .filter(|m| !req.needs_multi_corner || m.is_multi_corner())
            .collect();
        if corner_ok.is_empty() {
            // Flops have no Liberty corner problem — they are characterized with the standard
            // cells — so an uncharacterized macro library is a reason to use flops, not to refuse.
            return self.fits_vehicle(flop_plan);
        }

        // Choose by smallest resulting area, not by largest macro: a wide-but-shallow array can be
        // cheaper on a narrower part, and width-banking multiplies instance count.
        let best = corner_ok
            .iter()
            .map(|m| {
                let width_banks = req.bits.div_ceil(m.bits);
                let depth_banks = req.words.div_ceil(m.words);
                let instances = width_banks * depth_banks;
                MemoryPlan {
                    implementation: Implementation::Macros {
                        macro_name: m.name,
                        depth_banks,
                        width_banks,
                        instances,
                    },
                    macro_name: m.name,
                    depth_banks,
                    width_banks,
                    instances,
                    area_mm2: f64::from(instances) * m.area_mm2(),
                }
            })
            .min_by(|a, b| a.area_mm2.total_cmp(&b.area_mm2))
            .expect("corner_ok is non-empty");

        // Whichever is actually smaller. A shallow array that "fits" a macro is still the wrong
        // build if flops cost less silicon.
        let best = if flop_plan.area_mm2 < best.area_mm2 {
            flop_plan
        } else {
            best
        };
        self.fits_vehicle(best)
    }

    /// A plan is only real if the vehicle can carry it.
    fn fits_vehicle(&self, p: MemoryPlan) -> Result<MemoryPlan, PdkUnsupported> {
        if let Some(ua) = self.user_area {
            if p.area_mm2 > ua.mm2() {
                return Err(PdkUnsupported::ExceedsUserArea {
                    needed_mm2: p.area_mm2,
                    available_mm2: ua.mm2(),
                });
            }
        }
        Ok(p)
    }
}

/// SkyWater 130 nm, the process every block in this repo has been hardened on.
pub const SKY130A: PdkTarget = PdkTarget {
    name: "sky130A",
    node_nm: 130,
    std_cell_library: "sky130_fd_sc_hd",
    metal_stack: &["li1", "met1", "met2", "met3", "met4", "met5"],
    local_interconnect: Some("li1"),
    top_routing_layer: "met5",
    clk_driving_cell: Some(("sky130_fd_sc_hd__clkbuf_16", "X")),
    golden_drc_deck: "sky130A_mr.drc",
    // Caravel: user macros take vccd1/vssd1 from the harness. One with its own grid stays on met3 and below, under
    // the wrapper's met4/met5 power straps — how the shipped submission was hardened and precheck-passed (19/19,
    // 2026-09-17). One fed by the wrapper is efabless's reference user macro (caravel_user_project,
    // openlane/user_proj_example/config.json: `DESIGN_IS_CORE` 0, `RT_MAX_LAYER` met4): met4 straps only, crossed
    // by the wrapper's met5.
    user_area: Some(UserArea {
        width_um: 2920.0,
        height_um: 3520.0,
        macro_power: Some(("vccd1", "vssd1")),
        macro_max_layer: Some("met3"),
        fed_macro_max_layer: Some("met4"),
        harness: Some(HarnessTiming {
            // efabless's characterization of the harness in the scaffold's `base_user_project_wrapper.sdc`
            // (generated from Caravel's own STA by get_cup_sdc.py): the `-min` input delays of the logic
            // analyzer and the Wishbone bus. The GPIOs carry none — `IO_SYNC` 0 declares them asynchronous.
            input_min_ns: &[
                ("la_data_in[*]", 0.18),
                ("la_oenb[*]", 0.3),
                ("wbs_adr_i[*]", 0.79),
                ("wbs_dat_i[*]", 1.04),
                ("wbs_sel_i[*]", 1.19),
                ("wbs_we_i", 1.65),
                ("wbs_cyc_i", 1.69),
                ("wbs_stb_i", 1.86),
            ],
            // Caravel's chip-level signoff SDC (caravel/signoff/caravel/caravel.sdc, Rev 1, 2023-12-02):
            // `set_clock_uncertainty 0.1000` on `clk`, `set derate 0.0375`, and `set clk_period 25` on the clock pad,
            // which `caravel_clocking.v` hands the user area as `wb_clk_i`.
            clock_uncertainty_ns: 0.1,
            timing_derate: 0.0375,
            core_clock_period_ns: 25.0,
            // Every port the scaffold's wrapper SDC delays against `clk` outside its `IO_SYNC` budget.
            core_clock_inputs: &[
                "wb_rst_i",
                "la_data_in[*]",
                "la_oenb[*]",
                "wbs_sel_i[*]",
                "wbs_we_i",
                "wbs_adr_i[*]",
                "wbs_stb_i",
                "wbs_dat_i[*]",
                "wbs_cyc_i",
            ],
            core_clock_outputs: &["user_irq[*]", "la_data_out[*]", "wbs_dat_o[*]", "wbs_ack_o"],
        }),
    }),
    sram_macros: &[
        SramMacro {
            name: "sky130_sram_2kbyte_1rw1r_32x512_8",
            words: 512,
            bits: 32,
            width_um: 683.10,
            height_um: 416.54,
            ports: "1rw1r",
            corners: &["TT_1p8V_25C"],
        },
        SramMacro {
            name: "sram_1rw1r_32_256_8_sky130",
            words: 256,
            bits: 32,
            width_um: 376.48,
            height_um: 446.235,
            ports: "1rw1r",
            corners: &[
                "FF_1p8V_25C",
                "SS_1p8V_25C",
                "TT_1p7V_25C",
                "TT_1p8V_0C",
                "TT_1p8V_25C",
                "TT_1p8V_100C",
                "TT_1p9V_25C",
            ],
        },
        SramMacro {
            name: "sky130_sram_1kbyte_1rw1r_32x256_8",
            words: 256,
            bits: 32,
            width_um: 479.78,
            height_um: 397.50,
            ports: "1rw1r",
            corners: &["TT_1p8V_25C"],
        },
        SramMacro {
            name: "sky130_sram_1kbyte_1rw1r_8x1024_8",
            words: 1024,
            bits: 8,
            width_um: 455.30,
            height_um: 446.46,
            ports: "1rw1r",
            corners: &["TT_1p8V_25C"],
        },
    ],
    precheck_checks: 19,
    estimator_layer_rc: Some(SKY130A_LAYER_RC),
    provenance:
        "open_pdks 0fe599b2afb6708d281543108caf8310912f54af via ciel; macro sizes read from \
                 each macro's own LEF SIZE line under ~/.ciel/sky130A/libs.ref/sky130_sram_macros; \
                 user area is the Caravel fixed 2920x3520 um; precheck count measured from a real \
                 mpw_precheck run (2026-09-17, 19/19 passed)",
};

/// GlobalFoundries 180 nm MCU. The variant this project's vendored OpenLane supports is C.
///
/// Note the two asymmetries against sky130 that matter for planning: the SRAM IP is far smaller
/// and only 8 bits wide, and `mpw_precheck` schedules SIX FEWER checks — so a passing gf180
/// submission is a weaker statement than a passing sky130 one, and should never be reported as
/// the same thing.
pub const GF180MCUC: PdkTarget = PdkTarget {
    name: "gf180mcuC",
    node_nm: 180,
    std_cell_library: "gf180mcu_fd_sc_mcu7t5v0",
    metal_stack: &["Metal1", "Metal2", "Metal3", "Metal4", "Metal5"],
    local_interconnect: None,
    top_routing_layer: "Metal4",
    clk_driving_cell: None,
    golden_drc_deck: "gf180mcuC_mr.drc",
    user_area: Some(UserArea {
        width_um: 3000.0,
        height_um: 3000.0,
        macro_power: None,
        macro_max_layer: None,
        fed_macro_max_layer: None,
        harness: None,
    }),
    sram_macros: &[
        SramMacro {
            name: "gf180mcu_fd_ip_sram__sram512x8m8wm1",
            words: 512,
            bits: 8,
            width_um: 0.0,
            height_um: 0.0,
            ports: "1rw",
            corners: &[],
        },
        SramMacro {
            name: "gf180mcu_fd_ip_sram__sram256x8m8wm1",
            words: 256,
            bits: 8,
            width_um: 0.0,
            height_um: 0.0,
            ports: "1rw",
            corners: &[],
        },
    ],
    precheck_checks: 13,
    estimator_layer_rc: None,
    provenance: "PDK NOT YET INSTALLED — `ciel ls --pdk-family gf180mcu` is empty; install with \
                 `cd tools/OpenLane && make pdk PDK=gf180mcuC`. Library/stack/die/deck names are \
                 read from the vendored tools (tools/caravel_user_project/openlane/\
                 user_project_wrapper/config.json pdk::gf180mcuC block; \
                 tools/mpw_precheck/checks/tech-files/gf180mcuC_mr.drc; the caravel-gf180mcu \
                 harness is a DIFFERENT repo from tools/caravel). SRAM macro names come from \
                 tools/mpw_precheck lvs_config files; their SIZE and corners are UNMEASURED (zeroed \
                 here on purpose) and must be filled from the installed PDK's LEF before any \
                 gf180 memory plan is trusted.",
};
