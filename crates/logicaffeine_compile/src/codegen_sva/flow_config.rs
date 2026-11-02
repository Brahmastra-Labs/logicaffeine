//! `flow_config` — the compiler emits its own physical-design configuration.
//!
//! WHY THIS EXISTS. Closing a block's electrical gates was, until now, a person editing environment
//! variables in front of `scripts/openlane-run.sh` — a slew margin here, an antenna threshold
//! there, a density sweep — and the knowledge of WHICH values work lived in that person's head and
//! in a scratch directory. Every new block re-learned it. The whole-program engine, which does not
//! exist as a hardened macro yet, would have re-learned it again.
//!
//! The compiler already knows the design and (via [`super::pdk_target::PdkTarget`]) the process.
//! That is everything needed to choose the flow configuration, so it should choose it — and the
//! measured reason for each value should live next to the value, in code, where a test can read it.
//!
//! EVERY NUMBER BELOW IS MEASURED, on this box, and the measurement is quoted at its field. Two of
//! them are NEGATIVE results kept deliberately, because the expensive mistake is re-trying a lever
//! that has already been shown not to work.

use super::pdk_target::{HarnessClock, PdkTarget, PdkUnsupported};
use std::fmt::Write as _;

/// What the compiler knows about a design that changes how it should be built.
#[derive(Debug, Clone, PartialEq)]
pub struct DesignProfile {
    /// Top-level module name; also the OpenLane `DESIGN_NAME`.
    pub name: String,
    pub clock_port: String,
    pub clock_period_ns: f64,
    /// Top-level I/O count. A design with many wide buses is PAD-LIMITED — its die is set by the
    /// perimeter its pins need, not by its logic — and must be given an explicit die area or the
    /// floorplanner sizes it too small for the pins to fit.
    pub port_bits: u32,
    /// Hardened macros this design instantiates (SRAM, or another of our blocks).
    pub macros: Vec<String>,
    /// Set for a fixed-die shuttle vehicle wrapper, where the die is not ours to choose.
    pub fixed_die: bool,
}

impl DesignProfile {
    pub fn new(name: &str, clock_port: &str, clock_period_ns: f64) -> Self {
        Self {
            name: name.to_string(),
            clock_port: clock_port.to_string(),
            clock_period_ns,
            port_bits: 0,
            macros: Vec::new(),
            fixed_die: false,
        }
    }
    pub fn with_ports(mut self, bits: u32) -> Self {
        self.port_bits = bits;
        self
    }
    /// A design is pad-limited when its perimeter, not its logic, sets the die. The threshold is
    /// the point at which the observed blocks needed an explicit `DIE_AREA`: the infinichip
    /// subsystem (~386 pins) did, everything smaller did not.
    pub fn is_pad_limited(&self) -> bool {
        self.port_bits >= 256
    }
}

/// How antenna violations are to be closed. These are NOT interchangeable, and choosing wrongly
/// costs slew.
///
/// MEASURED on `coproc_core`: its max-slew stuck at exactly 126 whether the resizer margin was 30
/// or 45 — the lever could not reach the cause. Half the slew violators were `ANTENNA_*/DIODE`
/// pins. A diode is extra CAPACITANCE on its net, and the diode passes run AFTER the resizer, so
/// slew repair never sees the load they add. Diode-based antenna closure and slew closure fight.
///
/// The SAT jog-router fixes antenna with METAL instead — it bridges a segment one layer up, adding
/// no cell and no capacitance — so on a block where slew is tight it is strictly the better tool.
/// It only became safe to rely on alone once its free-window search was taught the layer spacing
/// rule; before that it placed bridges flush against foreign metal (359 Magic violations on the
/// Caravel wrapper, while TritonRoute reported 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AntennaStrategy {
    /// OpenLane's diode passes plus the jog ECO. The default: it closed `gpu_simt` and
    /// `fabric_soc` outright, and diodes are cheap when slew has headroom.
    DiodesAndJog,
    /// Jog-router only — no diode is inserted anywhere. For a block whose slew is tight, where the
    /// diodes' added load is the thing keeping slew from closing.
    JogOnly,
    /// The PRE-ROUTE heuristic diode pass plus the jog ECO, with OpenLane's POST-route
    /// `GRT_REPAIR_ANTENNAS` off.
    ///
    /// This exists because the other two variants are not the only choices, and treating them as
    /// such hid the best one. OpenLane runs TWO independent diode passes — a pre-route heuristic
    /// pass driven by `HEURISTIC_ANTENNA_THRESHOLD`, and the global router's own post-route repair
    /// — and an enum with only "both" and "neither" cannot express the third combination. Every
    /// `coproc_core` experiment for two sessions moved them together:
    ///
    /// | heuristic | GRT repair | result |
    /// |---|---|---|
    /// | on  | on  | 17,716 diodes, max-slew 126-282 |
    /// | off | off | antenna 538-770 violating nets |
    /// | on  | off | **slew 120, antenna 27, and it ROUTES** |
    ///
    /// That third row is this variant: DRC 0, LVS 0, XOR 0, routing 0, setup +32.54, hold +0.27 on
    /// a block previously recorded as a dead end needing an emitter-side rewrite. What made the
    /// difference is not the diode COUNT — that is essentially unchanged at 17,548 — but that
    /// GRT's repair RE-ROUTES as well as inserting, and it was that re-route, not the diodes,
    /// that hit GRT-0119 congestion at every utilisation and die size tried.
    ///
    /// It also corrected a wrong attribution: the diodes were believed to come from GRT because
    /// they "survived" a threshold of 300. Surviving a threshold does not identify a source. With
    /// GRT off there are still 17,548 of them, so they come from the heuristic pass — which makes
    /// `heuristic_antenna_threshold` the live lever rather than an exhausted one. (Though not a
    /// wide one: at 1500 the count drops far enough that antenna explodes back to 510 nets. It is
    /// a cliff, not a gradient.)
    HeuristicDiodesAndJog,
}

/// The electrical-closure settings, separated from the rest because they are the ones that were
/// actually measured and the ones a future change is most likely to get wrong.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClosureSettings {
    /// The ABC script technology mapping runs, or OpenLane's default (`AREA 0`) when `None`.
    ///
    /// MEASURED on `wpe_user_project` (191,641 generic gates between 32,779 inputs and 32,773 outputs, the
    /// on-die tables' read multiplexers most of them): `AREA 0` opens with `fx; mfs`, and its ABC pass had
    /// run 2 h 15 min without finishing when it was stopped (RUN_2026.09.25_19.58.43). `AREA 3`
    /// (`strash; dch; map`) mapped the same extracted netlist in 6 min 8 s, to 240,032 cells and
    /// 1.86 mm² of logic with an 8.56 ns critical path against a 40 ns clock.
    pub synth_strategy: Option<SynthStrategy>,
    /// Percent margin `repair_design` targets below the slew limit.
    ///
    /// MEASURED on `gpu_simt` (79,965 cells): at OpenLane's defaults (PL 20 / GLB 10) the block
    /// signed off carrying **6 max-slew violations**. At **30** it reached slew 0 — and the design
    /// got better in every other respect at the same time: max-fanout 3,999 -> 847, setup slack
    /// +3.75 -> +4.48 ns, cell count 79,965 -> 69,080. Buffering for slew shortens the electrical
    /// paths that were forcing large drivers elsewhere.
    ///
    /// IT INTERACTS WITH THE CLOCK-DRIVER DECLARATION, and the interaction is large. Declaring
    /// `SYNTH_CLK_DRIVING_CELL` (see `PdkTarget::clk_driving_cell`) makes the clock port's assumed
    /// edge FAST, which removes the pessimism that was inflating every downstream transition — and
    /// with it the resizer's reason to buffer. On `systolic_os_pipe8_realbist` (392k cells) the two
    /// nearly-identical runs differ only in that setting: slew went 3 -> 89, all 89 of them
    /// MARGINAL (0.93-1.01 against a 0.75 limit) and the design 3k cells SMALLER. At margin 45 the
    /// same configuration reached slew 0, with setup +2.03 and XOR back to 0. So the clock fix is
    /// right and needs the margin raised alongside it, not reverted.
    ///
    /// 45 IS NOT A SAFE DEFAULT. The placement resizer destabilises at high margin in a
    /// size-dependent way: measured OK at 45 on 417k and 392k cells, CRASHES at 45 on 819k and
    /// 204k, and crashes at 60 on 417k. Raise it per block against a measured failure, not
    /// globally.
    ///
    /// AND THE FAILURE MODE AT SIZE IS MEMORY, NOT A CRASH — which matters because it does not
    /// look like a failure. On `systolic_pipe16_realbist` (457k cells) at margin 45 the
    /// PLACEMENT RESIZER alone reached 92.9 GB resident after 38 minutes, still at step 9, with
    /// detailed routing — the real peak — ahead of it. The kernel had already taken one openroad
    /// at 98.7 GB on this 125 GB box that day, and an OOM kill leaves an EMPTY `.errors` file and
    /// a log that simply stops, so it reads as a mystery crash rather than as this setting. Three
    /// runs were lost that way before the cause was identified. At margin 30 the same block
    /// completes.
    ///
    /// So the size dependence is a memory curve with a cliff, and 457k cells sits just past the
    /// safe edge: 392k and 417k are fine at 45, 457k is not. Between "works" and "OOM" there is
    /// no warning, because the resizer gives none.
    ///
    /// MARGIN 35 ALSO OOMs ON 457k. The cliff is not between 35 and 45; it is below 35, and only
    /// the default 30 completes on that block. Measured with a 45-SECOND SAMPLER the peak read
    /// 83 GB — comfortably under the wall — while the kernel still took the process, because a
    /// sampler misses the spike between polls. Read `/proc/<pid>/status` VmHWM for the true
    /// high-water mark; a sampled peak is an underestimate and will tell you a run was safe when
    /// it was not.
    /// THIS DRIVES THE ROUTING-STAGE (GLB) RESIZER ONLY. It used to drive the placement (PL)
    /// resizer too, and that coupling is a trap: the two have DIFFERENT failure modes, so
    /// raising the one that helps necessarily raised the one that explodes.
    ///   GLB (step 15, `repair_design` after global route) is where the slew repair that closed
    ///   `systolic_os_pipe8_realbist` happened — 89 -> 0 at margin 45.
    ///   PL (step 9, before CTS) is where the memory goes. On `systolic_pipe16_realbist` (457k
    ///   cells) it reached a TRUE peak of 114 GB (VmHWM) at margin 35 on a 125 GB box and was
    ///   OOM-killed; margin 45 and margin 35-with-fewer-diodes did the same.
    /// Split so a block can take the slew repair it needs without the placement resizer that
    /// cannot afford it. See `pl_slew_margin_pct`.
    pub slew_margin_pct: u32,
    /// Margin for the PLACEMENT resizer (step 9). Kept at the safe default independently of
    /// `slew_margin_pct` because this is the one with the memory cliff — measured OK at 45 on
    /// 392k and 417k cells, OOM at 35 on 457k, OOM at 45 on 819k.
    ///
    /// A NOTE ON MEASURING IT: a sampled peak is not a peak. A 45-second sampler read 83 GB on a
    /// run whose true VmHWM was 114 GB, i.e. it reported "comfortably under the wall" about a
    /// process the kernel was killing. Read /proc/<pid>/status VmHWM.
    pub pl_slew_margin_pct: u32,
    /// Percent margin below the capacitance limit. Same mechanism, same call
    /// (`repair_design -cap_margin`); `fabric_horner_seq` is the only block with nonzero max-cap
    /// (7), so this is carried at the same value rather than tuned separately.
    pub cap_margin_pct: u32,
    /// Fanout above which the heuristic diode pass inserts a protection diode.
    ///
    /// THE RELATIONSHIP IS NOT MONOTONIC, and reading it as monotonic wastes runs in both
    /// directions. It is a U-curve whose optimum depends on how large the residual already is.
    ///
    /// AT A LARGE RESIDUAL, RAISE IT. A low threshold floods the design with diodes that crowd
    /// out the ones that would have mattered — post-fill there is no free site left for the
    /// diode the ARC check actually wants. Measured: `gpu_simt` 10 -> 300 took antenna 4 -> 2;
    /// `coproc_core` residual 15 at 300 against 23 at 160; `systolic_os_pipe8_realbist` built at
    /// 10 came out 536,104 cells with 8 max-slew violations against 392,221 cells with 3 at 300.
    /// The flood is not subtle at the bottom end: `soft_core_scalar` at threshold 10 inserted
    /// 7,527 diodes into a 14,700-cell design — over half the instances — and died at GRT-0119.
    ///
    /// AT A SMALL RESIDUAL, LOWER IT. Once the count is down to single digits the remaining
    /// violators are often nets the pass simply never selected, and there is no flooding risk
    /// left to avoid. Measured the same afternoon, on the same blocks that produced the numbers
    /// above: `systolic_os_pipe8_realbist` 300 -> 150 took antenna 3 -> 0 and CLOSED the block;
    /// `coproc_core` 300 -> 150 took 5 -> 2 with max-slew still 0.
    ///
    /// So the default stays 300 because a block's FIRST run is always in the large-residual
    /// regime, and 150 is the endgame move once the count is small and a diode-less violator is
    /// visible in the report. Both directions are real; neither generalises.
    ///
    /// AND IT CANNOT FIX EVERYTHING. A violator whose pin ALREADY carries a diode is beyond this
    /// lever entirely — measured on `coproc_core`, where all 7 survivors had one. That case needs
    /// `grt_antenna_margin_pct` (which acts before the blocking geometry exists), a base-route
    /// change, or narrower RTL.
    pub heuristic_antenna_threshold: u32,
    /// Iterations of the SAT jog-router antenna ECO.
    ///
    /// MEASURED on `gpu_simt`: the ECO converges 8 -> 2 -> 1 in its first THREE iterations and
    /// then sits at 1 for twenty-eight more. So this is sized for convergence, not for grinding;
    /// raising it further buys nothing. A residual that survives here is a side-area violation,
    /// which no number of diode iterations can fix.
    pub antenna_eco_iters: u32,
    /// Placement density.
    ///
    /// NEGATIVE RESULT, recorded so it is not re-tried: 0.40 is WORSE than 0.45 for `gpu_simt`
    /// (antenna 4 vs 2, cells 43,952 vs 43,564) even though 0.40 is exactly what finally cleared
    /// `coproc_core`'s last violator. The difference is the violation's KIND — coproc's residual
    /// was a met5 routing detour that a different base route removed, gpu_simt's is a met1
    /// side-area violation that a density change does not touch. Sweep density only against an
    /// upper-layer detour.
    pub target_density: f64,
    /// Global-route cost penalty on met1, 0..1 (higher = the router avoids met1 more).
    ///
    /// MEASURED — this is what finally closed `gpu_simt`. After slew/cap margin and the diode
    /// threshold, one met1 SIDE-AREA antenna violator survived at ratio 1.14. It could not be
    /// dioded (a diode addresses gate area; side area is perimeter with no diffusion term), the
    /// jog router plateaued on it (8 -> 2 -> 1 in three ECO iterations, then 1 for twenty-eight
    /// more), and a density sweep made it WORSE. Forbidding met1 for signals outright
    /// (`RT_MIN_SIGNAL_LAYER=met2`) FAILED TO ROUTE even at 25% core utilisation — the design
    /// needs met1 capacity. A cost penalty is the version that works: met1 stays available so it
    /// cannot congest, but the router stops building long met1 runs. At **0.7** the block reached
    /// antenna 0/0 with every other gate still at zero.
    pub met1_route_penalty: f64,
    /// The rip-up iterations global routing spends removing overflow (`GRT_OVERFLOW_ITERS`), or OpenLane's own 50 when
    /// `None`. Every global route of the flow takes it, the resizers' re-routes and the fanout step's included.
    ///
    /// MEASURED on `wpe_user_project` at 16 ns (the private tree's RUN_2026.10.19_19.38.51 and its variants): at 50 the
    /// route stopped at GRT-0119 with gcells at the die center over by one to three tracks; at 150 and at 400, every
    /// other setting held, it stopped the same way, the iterations ending once the overflow could no longer grow
    /// (GRT-0230; RUN_2026.10.19_20.34.27, RUN_2026.10.19_22.07.52). Iterations alone do not route a block whose
    /// gcells are over capacity; with capacity given back ([`ClosureSettings::grt_adjustment`] or
    /// [`ClosureSettings::met1_route_penalty`]) the same block routed at 150 with no overflow.
    pub grt_overflow_iters: Option<u32>,
    /// The share of every gcell's capacity global routing holds back, 0..1 (`GRT_ADJUSTMENT`), or OpenLane's own 0.3
    /// when `None`. The reserve is room detailed routing has for vias, pins and blockages the global grid does not see;
    /// a block that cannot route under it trades some of that room for capacity.
    ///
    /// MEASURED on `wpe_user_project` at 16 ns, 150 overflow iterations: at 0.3 the global route stopped at GRT-0119
    /// (RUN_2026.10.19_20.34.27); at 0.2 it routed with no overflow, 18,902,039 um (RUN_2026.10.20_17.57.25, x5).
    pub grt_adjustment: Option<f64>,
    /// The sites a side every cell keeps free of its neighbours when placement is legalized (`DPL_CELL_PADDING`, which
    /// OpenLane takes as a total and halves to a side), or OpenLane's none when `None`. Abutting cells in a dense patch
    /// leave detailed routing no way to reach their met1 pins; global placement is left unpadded, so its density target
    /// still holds.
    pub dpl_cell_padding_per_side: Option<u32>,
    /// The longest wire `repair_clock_nets` leaves between a clock input and the tree's root, in um
    /// (`CTS_CLK_MAX_WIRE_LENGTH`).
    ///
    /// MEASURED on `wpe_user_project` (2.8 x 3.4 mm, RUN_2026.09.26_17.51.01, the CTS libraries at every corner):
    /// OpenROAD's own limit, 3,331 um, left the 1.7 mm run from `wb_clk_i` to the root buffer whole, 1.125 ns at
    /// its end against the 0.75 ns limit; at 800 um it takes four buffers and no point on it exceeds 0.353 ns —
    /// under the post-diode upsize's detection limit too. At 1,000 um it reaches 0.429 ns; at 500 um it rewires
    /// fourteen of the tree's own nets.
    ///
    /// 0 keeps OpenROAD's own limit, as every block hardened before this was measured was built
    /// ([`parse_wire_limit_um`]).
    pub clock_max_wire_um: u32,
    /// The longest wire either resizer leaves on a signal net, in um (`PL_RESIZER_MAX_WIRE_LENGTH`,
    /// `GLB_RESIZER_MAX_WIRE_LENGTH`).
    ///
    /// MEASURED on `wpe_user_project` (RUN_2026.09.27_06.33.43): under OpenROAD's own 3,331 um, nets of 1,447 to
    /// 1,814 um driven by `buf_12` — the strongest buffer the flow may place — signed off at 0.76-0.87 ns against the
    /// 0.75 ns limit at ss_100C_1v60, max RC, below what the post-diode upsize's global-route estimate detects. 800 um,
    /// the clock's limit, splits the 2,709 of 310,043 nets routed longer (0.9%).
    ///
    /// 0 keeps OpenROAD's own limit ([`parse_wire_limit_um`]).
    pub signal_max_wire_um: u32,
    /// Ratio margin for the GLOBAL-ROUTE antenna repair (`repair_antennas -ratio_margin`),
    /// as a percent. The PDK default is 10.
    ///
    /// WHY THIS LEVER IS DIFFERENT FROM EVERY OTHER ANTENNA LEVER, and why it was worth adding:
    /// it is the ONLY antenna repair that runs BEFORE detailed routing. Everything else in this
    /// struct acts afterwards, and by then TritonRoute has patched nets with via-enclosure RECTs
    /// that the jog router cannot bridge (odb exposes no way to round-trip that geometry from
    /// Python — `getRect` needs out-params, there are no SWIG pointer helpers, and
    /// `dbWireShapeItr` is unbound; all three checked). A net that reaches detailed routing
    /// already violating, and already carrying a diode, has NO repair left. This one gets in
    /// first.
    ///
    /// MEASURED on `coproc_core` (180k cells, narrow-multiply RTL): raising 10 -> 50 with
    /// `grt_antenna_iters` 15 -> 25 took the antenna residual 11 -> 5 with max-slew still 0 and
    /// every other gate at zero. Nothing else in the run changed. The repair itself is cheap —
    /// 3 diodes placed at margin 50 against 1 at margin 10 — because global route works from an
    /// ESTIMATE and only sees the worst offenders; that is also why it cannot be the whole fix.
    ///
    /// IT HAS AN OPTIMUM, NOT A DIRECTION. Pushing further on the same block — 50/25 -> 100/40,
    /// every other setting held — took the residual from 1 net back up to 8, including a new met5
    /// violator (ratio 1.10 against a 3486.68 requirement). Over-repairing at global route inserts
    /// diodes and layer changes against an ESTIMATE, and detailed routing then lands elsewhere.
    /// Measured points on `coproc_core`: 10 -> 11 nets, 50 -> 5 (then 1 with threshold 100),
    /// 100 -> 8. Stay at 50.
    pub grt_antenna_margin_pct: u32,
    /// Iterations for the same pre-detailed-route repair. PDK default 15.
    pub grt_antenna_iters: u32,
    /// Insert antenna diodes BEFORE the slew/cap resizer instead of after it
    /// (`LOGOS_DIODES_BEFORE_RESIZE`, implemented in `scripts/tcl_commands/routing.tcl`).
    ///
    /// The base order sizes every driver and THEN hangs diodes on its net, so the resizer never
    /// sees the load it is sizing for. Measured on two blocks: 25 of `coproc_core`'s 69 slew
    /// violators were diode pins, and every violating driver net on `systolic_pipe16_realbist`
    /// carried diodes. Both symptom-side fixes failed — re-repairing after the diodes congested
    /// or backfired (3,587 buffers for 7 violations, slew 69 -> 181), and raising the resizer
    /// margins hit a 114 GB OOM in the placement resizer and a 40-minute stall in the routing one.
    /// Heuristic diode insertion reads placement, not routing, so it can legally run first.
    ///
    /// MEASURED NEGATIVE, which is why it stays off. On `systolic_pipe16_realbist` (margins 30/30,
    /// everything else held) it made max-slew 27 -> 123, antenna 18 -> 25 nets, and squeezed setup
    /// to +0.42 ns. Sizing drivers for diodes that are already present does not help when the
    /// diodes themselves are the problem: most of the design's ~18k heuristic diodes are joined to
    /// their nets above the violating layer and are not credited there at all (see
    /// `eco_diode_fallback`), so they are load with no antenna benefit.
    pub diodes_before_resize: bool,
    /// At the jog router's fixpoint, add ONE more diode at each still-violating pin (the
    /// manual-stub ECO in `routing.tcl`, `LOGOS_ECO_DIODE_FALLBACK`) instead of stopping.
    ///
    /// The jog and the diode ECO were written either/or: with the jog enabled the diode ECO was
    /// unreachable code, so the nets the jog cannot bridge were exactly the nets nothing repaired.
    /// Every remaining violator on `coproc_core` and `systolic_pipe16_realbist` had that shape —
    /// one heuristic diode already on the pin, ratio still 1.0-1.4x, and a via-enclosure RECT the
    /// jog cannot round-trip. One diode's diffusion is not enough and no pass ever added a second.
    /// The stub ECO runs no router, so it cannot strand a net; any geometry problem it caused
    /// would surface at the fail-closed DRC/LVS/XOR gates rather than as a false clean.
    pub eco_diode_fallback: bool,
    /// After the diodes are in, grow the driver of each net that violates max slew WITH its diodes
    /// present, or buffer that one net when its driver is already the strongest of its function
    /// (`LOGOS_POST_DIODE_UPSIZE`, `scripts/openroad/logos_post_diode_upsize.tcl`).
    ///
    /// The surgical successor to the whole-design post-diode repair (3,587 buffers for 7
    /// violations, slew 69 -> 181). Every surviving slew violator on `systolic_pipe16_realbist`
    /// was driven by an `a21oi_4`/`a31oi_4` — the top of its family, so no upsize reaches it.
    /// Measured offline on that block's post-diode netlist: 22 nets selected, 28 drivers grown,
    /// 15 nets rebuffered, +26 cells of 457k, 0 residual after 7 rounds, about five minutes; the
    /// three culprit drivers went from 0.41-0.43 ns estimated slew to at most 0.13 ns.
    pub post_diode_upsize: bool,
    /// How far below the real limit the upsize DETECTS, in percent. The global-route estimate is
    /// about 2x optimistic on the nets signoff flags (0.37-0.43 ns estimated vs 0.75-0.85 ns
    /// signed off) but ranks them correctly: at 52% (0.36 ns) all 15 of pipe16's signoff-violating
    /// pins are selected among 103 pins on 22 nets; at 0.40 ns two are missed; 0.34 ns selects
    /// 442 pins and 0.30 ns 2,081. Signoff still judges against the real limit.
    pub post_diode_upsize_margin_pct: u32,
    /// Detect the upsize's violators at EVERY library corner (ss / tt / ff), not the typical one
    /// alone (`LOGOS_UPSIZE_ALL_CORNERS`). Signoff judges slew at ss_100C_1v60; every surviving
    /// violator on `systolic_pipe16_realbist` and `systolic_os_pipe8_realbist`
    /// (RUN_2026.09.23_14.05.55 / _11.27.15) was slow-corner only, typical slew 0.
    pub post_diode_upsize_all_corners: bool,
    /// Upsize again after global routing's antenna repair, with that repair's diodes present, then re-route without
    /// it (`LOGOS_POST_ANTENNA_UPSIZE`, `scripts/tcl_commands/routing.tcl`).
    ///
    /// `repair_antennas` runs after the post-diode upsize and hangs diodes on nets it has already sized. MEASURED on
    /// `wpe_user_project` (RUN_2026.09.27_06.33.43): up to 20 on a single-sink net of 255-299 um (ANT-0009, more than
    /// 10 diodes per gate), and 13 of its 21 slow-corner slew violators were such nets, each driven by the strongest
    /// cell of its function. Replayed on that run's post-repair route, the upsize found and repaired all 13 (+151
    /// cells). Off unless chosen: it adds a global route.
    pub post_antenna_upsize: bool,
    /// Build the clock tree within the SDC's `set_max_fanout` (`LOGOS_CTS_FANOUT`, `scripts/openroad/cts.tcl` and
    /// `logos_cts.py`): leaf clusters of at most the limit's sinks, H-tree splits balanced, and the tree deep enough
    /// that a balanced leaf averages under the limit.
    ///
    /// At OpenLane's defaults a leaf cluster may hold 25 sinks and each H-tree split may hand one branch 60% of its
    /// parent's clusters, so the imbalance compounds level by level. MEASURED on `gpu_simt`'s placement
    /// (RUN_2026.09.23_09.37.32, CTS replayed): 33 clock nets over a limit of 10 at the defaults; 3 with clusters
    /// held to 10, every one an H-tree leaf driving 11-12 cluster buffers; 0 with the splits balanced too, from the
    /// same 317 clock buffers, skew 0.54 -> 0.48 ns. A shallower or deeper unbalanced tree is worse, not better:
    /// at a maximum of 7 or 5 leaf sinks, 4 nets stayed over (up to 16) and 5 added 160 buffers. Every signed-off
    /// block's clock violators were these leaves: 703 on `wpe_user_project`, 295 on `systolic_pipe16_realbist`.
    pub cts_within_fanout: bool,
    /// Split every net the diode passes take over `set_max_fanout` by whole gates, each with the diodes placed for
    /// it (`LOGOS_DIODE_FANOUT`, `scripts/openroad/logos_diode_fanout.tcl`), after the pre-route diodes and again
    /// after global routing's antenna repair; and hold the post-route diode ECO under the limit.
    ///
    /// The resizer meets the limit on every logic net — all 23 signed-off blocks had 0 logic nets over it — and
    /// the liberty counts a diode as a load, so every non-clock violator signoff reported was a diode count:
    /// 5,052 of `wpe_user_project`'s 5,755 (ten mux2 inputs with a heuristic diode each read as 20; global
    /// routing's repair put up to 26 on one net), 2,384 on `systolic_pipe16_realbist`, 1,101 on `coproc_core`.
    /// OpenROAD's own single-net repair clears them but groups pins, not gates: replayed on `gpu_simt`'s
    /// post-diode placement it moved 295 of 1,907 diodes off their gate's net. Grouping whole gates cleared all
    /// 151 signal nets there with 283 buffers and every diode still on its gate's net.
    pub diode_fanout_repair: bool,
    /// Loads every net is built under the SDC's `set_max_fanout` (`LOGOS_FANOUT_MARGIN`,
    /// `scripts/openroad/logos_fanout_limit.tcl`): CTS and the diode-aware fanout step build to the limit less this,
    /// and every report and signoff judge the limit itself.
    ///
    /// It is room for the one load a pass after routing can add. The post-route antenna ECO adds a diode at a pin the
    /// checker still flags; on a net built to the limit it has nowhere to put it. MEASURED on `gpu_simt`
    /// (RUN_2026.10.03_07.00.23, built to the limit): pin `_13114_/C`, met1 ratio 1.02, on a net of ten gates and no
    /// diode — refused, and the block missed antenna by that one pin.
    ///
    /// The margin is kept by the steps that know which loads are critical, never by the SDC the resizers repair
    /// against. MEASURED NEGATIVE, the SDC way (RUN_2026.10.03_07.23.27): `repair_design` split every ten-load net
    /// without regard to slack, the slow-corner critical path went from 2 to 6 fanout-buffer pins, and setup slack
    /// from +0.27 to -0.01 ns, past what a full setup repair could recover. Kept by the fanout step instead — a net at
    /// the limit split like a diode-laden one, its least-slack gates left on the driver — the same block signed off
    /// with every gate at zero and setup +0.19 ns (RUN_2026.10.03_07.35.09; 493 nets split, 644 buffers, 2.6% of its
    /// nets sit at exactly the limit).
    pub fanout_margin_loads: u32,
    /// Run global-route timing repair without pin swapping (`LOGOS_GLB_SKIP_PIN_SWAP`). OpenROAD
    /// 41a51eaf's post-route `repair_timing` lost one net's driver from its route tree on
    /// `soft_core_realbist` (RSZ-0073 x51 on `_19008_/Y`, every signal net routed) and died on SIGSEGV
    /// under both antenna recipes; the same step with `-skip_pin_swap` completes. Sizing, buffering
    /// and hold repair are untouched.
    pub glb_skip_pin_swap: bool,
    /// Whether antenna is closed with diodes, or with the jog-router alone. See [`AntennaStrategy`].
    pub antenna: AntennaStrategy,
    /// Pin the router's thread count so a run reproduces regardless of what else the box is doing.
    /// The divergence this prevents (a design converging 25 -> 0 DRC reaching 22,090 -> 27,653
    /// under load) is thread oversubscription, not load itself.
    pub routing_cores: u32,
    /// The vertical power-strap pitch in um (`FP_PDN_VPITCH`), or OpenLane's own (153.6 on sky130) when `None`.
    ///
    /// A standard cell is fed along its met1 rail from the nearest vertical strap, so a cell midway between two
    /// straps sees the longest rail, and the drop along it grows with the square of that run. MEASURED on
    /// `fabric_horner_seq` (RUN_2026.10.03_14.54.29, anchored area sources on the strap pitch): worst 90.13 mV
    /// against the 90 mV gate, at x 98.6-101.9 um — midway between its straps at x 21.8/25.1 and 175.4/178.7 —
    /// while the block's average was 9.36 mV. Only the pitch shortens the rail; placement density and die size do not.
    /// The gate is each cell's droop, its supply's drop plus its ground's rise at the cell: the same block's worst cell
    /// drooped 106.45 mV at 76.8 um and 66.50 mV at 38.4 um (RUN_2026.10.20_17.57.25: supply 33.88 mV, ground 34.06 mV).
    pub pdn_vpitch_um: Option<f64>,
    /// The hold repair's slack margin in ns, for the placement and the global-route repair both
    /// (`PL_`/`GLB_RESIZER_HOLD_SLACK_MARGIN`), or OpenLane's own (0.1 and 0.05) when `None`.
    ///
    /// The repair judges hold on estimated parasitics and signoff on extracted ones, so the margin must cover the
    /// difference. MEASURED on `systolic_pipe16_realbist` (RUN_2026.10.03_18.45.27): the global-route repair met
    /// its 0.05 ns, and signoff read -0.01, -0.03 and -0.04 ns at the fastest corner on the min, nominal and max RC
    /// corners, after routing took the clock skew from 0.52 ns at CTS to 1.05 ns at max RC — an estimate 0.09 ns
    /// optimistic.
    pub hold_slack_margin_ns: Option<f64>,
    /// The setup repair's slack margin in ns, for the placement and the global-route repair both
    /// (`PL_`/`GLB_RESIZER_SETUP_SLACK_MARGIN`), or OpenLane's own (0.05 and 0.025) when `None`.
    ///
    /// The repairs judge setup on global-route estimates at the nominal RC corner and stop once those close; signoff
    /// times the routed design extracted at the max-RC corner. MEASURED on `wpe_user_project` at 18.5 ns
    /// (RUN_2026.10.25_06.23.41, every net routed): the repairs closed to +0.03 ns on their estimates, and the routed
    /// design read -0.37 ns extracted at the nominal corner and -1.13 ns at the max-RC corner, at ss_100C_1v60.
    pub setup_slack_margin_ns: Option<f64>,
    /// The block is measured beside others (the comparative-synthesis bench), not signed off. A run whose typical
    /// corner misses setup completes and records the slack it missed by, so a period search can read the period the
    /// block needs; yosys's check findings (latches, out-of-range selects, undriven wires) are left to the recorder,
    /// which records them, rather than stopping the flow. Hold stops it either way. Antenna closes with every repair
    /// the flow has, [`ClosureSettings::eco_diode_fallback`] among them: one flow serves every candidate of a
    /// comparison, and the bench's first attempts met timing and ended on the residual it repairs
    /// (`qorsyn_k1_bambu_pipe_c_a7p10_p32_5`, RUN_2026.10.17_01.53.18: one met4 net at 1.02 times its limit). A
    /// signoff block never sets it. It is synthesized for delay ([`COMPARATIVE_SYNTH_STRATEGY`]) unless its caller
    /// names a strategy.
    pub comparative: bool,
}

/// The synthesis script of a comparative block ([`ClosureSettings::comparative`]) whose caller names none. OpenLane
/// hands abc the clock period as its target, and an area script recovers area down to it, so a run at a period longer
/// than the design needs closes a netlist slower than the design: the period it closes at says more about the period
/// it was given than about the design. A delay script maps every candidate for speed. MEASURED by the clock-push search
/// over the signed blocks under DELAY 0 (logs/clock-push, 10-17): decision_hft_bist 15.0 -> 4.4 ns, dram_executor
/// 10.5 -> 5.0, cgra_sumto 18 -> 6.4, rev_alu_core 14.5 -> 7.7.
pub const COMPARATIVE_SYNTH_STRATEGY: SynthStrategy = SynthStrategy::Delay(0);

/// The hold repair's margin for a comparative block ([`ClosureSettings::comparative`]) whose caller gives none. The
/// repair judges hold at the typical corner on estimated parasitics, and signoff checks every corner on extracted ones.
/// MEASURED on the bench's first fold-wrapped attempts: `qorsyn_k1_bambu_pipe_c_a7p10_p32_5`
/// (RUN_2026.10.17_01.20.19) met hold by +0.09 ns at the typical corner after OpenLane's 0.05 ns repair, and missed it
/// by 0.04, 0.05 and 0.04 ns at the fastest corner on the min, max and nominal RC corners, a 0.14 ns gap;
/// `qorsyn_k1_bambu_fsm_c_a7p10_p20_5` missed it by 0.03 ns the same way. One margin serves every candidate of a
/// comparison, so it covers that gap with room: 0.19 ns would close it exactly.
pub const COMPARATIVE_HOLD_MARGIN_NS: f64 = 0.3;

/// A wire limit as the flow scripts spell it: a length in um, or `openroad` for OpenROAD's own limit, emitted as 0,
/// which OpenLane reads as unset. A bare 0 is refused, so no typo drops a limit.
///
/// MEASURED: `fabric_horner_seq` at 800 um died in global-route setup repair (RUN_2026.09.28_20.36.35):
/// `rsz::makeBufferedNet` recursed without end under `RepairSetup::rebuffer` — 10,460 frames on an 8 MB stack, and
/// with an unlimited one it grew until the kernel killed it; a cycle in one net's route tree, OpenROAD 41a51eaf. Its
/// recorded recipe, at OpenROAD's limits, passes that step.
pub fn parse_wire_limit_um(s: &str) -> Option<u32> {
    if s == "openroad" {
        return Some(0);
    }
    s.parse().ok().filter(|l: &u32| *l > 0)
}

impl Default for ClosureSettings {
    fn default() -> Self {
        Self {
            slew_margin_pct: 30,
            pl_slew_margin_pct: 30,
            cap_margin_pct: 30,
            heuristic_antenna_threshold: 300,
            grt_antenna_margin_pct: 50,
            grt_antenna_iters: 25,
            diodes_before_resize: false,
            eco_diode_fallback: false,
            post_diode_upsize: false,
            post_diode_upsize_margin_pct: 52,
            post_diode_upsize_all_corners: false,
            glb_skip_pin_swap: false,
            cts_within_fanout: true,
            diode_fanout_repair: true,
            fanout_margin_loads: 1,
            antenna_eco_iters: 30,
            target_density: 0.45,
            met1_route_penalty: 0.7,
            grt_overflow_iters: None,
            grt_adjustment: None,
            dpl_cell_padding_per_side: None,
            clock_max_wire_um: 800,
            signal_max_wire_um: 800,
            post_antenna_upsize: false,
            antenna: AntennaStrategy::DiodesAndJog,
            routing_cores: 4,
            synth_strategy: None,
            pdn_vpitch_um: None,
            hold_slack_margin_ns: None,
            setup_slack_margin_ns: None,
            comparative: false,
        }
    }
}

/// One of OpenLane's `SYNTH_STRATEGY` scripts: `AREA 0..=3` or `DELAY 0..=4`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynthStrategy {
    Area(u8),
    Delay(u8),
}

impl SynthStrategy {
    /// `area3` / `delay4`, or `None` for anything OpenLane does not define.
    pub fn parse(s: &str) -> Option<SynthStrategy> {
        let (kind, n) = s.split_at(s.find(|c: char| c.is_ascii_digit())?);
        let n: u8 = n.parse().ok()?;
        match kind {
            "area" if n <= 3 => Some(SynthStrategy::Area(n)),
            "delay" if n <= 4 => Some(SynthStrategy::Delay(n)),
            _ => None,
        }
    }

    /// The flag spelling [`SynthStrategy::parse`] reads back.
    pub fn flag(&self) -> String {
        match self {
            SynthStrategy::Area(n) => format!("area{n}"),
            SynthStrategy::Delay(n) => format!("delay{n}"),
        }
    }

    /// The `SYNTH_STRATEGY` value OpenLane reads.
    pub fn openlane(&self) -> String {
        match self {
            SynthStrategy::Area(n) => format!("AREA {n}"),
            SynthStrategy::Delay(n) => format!("DELAY {n}"),
        }
    }
}

/// Emit the OpenLane `config.json` for a design on a process.
///
/// The point of generating this rather than hand-writing it: a block cannot silently be built
/// without the settings that were measured to be necessary, and the settings cannot drift from the
/// evidence, because both live here and a lock reads them.
pub fn openlane_config(d: &DesignProfile, pdk: &PdkTarget, c: &ClosureSettings) -> String {
    emit(d, pdk, c, None)
}

/// How a vehicle user macro takes power from the wrapper around it, which decides how high it may route.
///
/// The wrapper's grid generator keeps its straps off a macro's obstructions and feeds the macro's own power pins
/// where the two cross. MEASURED (wrapper PDN spike, 2026-09-26: the DRAM wrapper around a copy of its macro's
/// abstract with the met5 power ports removed and met4 obstructed across the die): every supply's met4 straps
/// were cut at the macro — 0 of 257 segments over it, against 127 over the unmodified macro
/// (RUN_2026.09.23_12.53.02) — and met5 crossed it and fed vccd1 and vssd1 through 190 via stacks each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacroGrid {
    /// Its own grid, as a core: straps on both of the process's PDN layers, and routing below them
    /// ([`UserArea::macro_max_layer`](super::pdk_target::UserArea::macro_max_layer)); the wrapper's grid passes
    /// over it on both layers.
    Own,
    /// Straps on the vertical PDN layer only, crossed and fed by the wrapper's horizontal straps, and routing up
    /// to that layer ([`UserArea::fed_macro_max_layer`](super::pdk_target::UserArea::fed_macro_max_layer)); the
    /// wrapper's vertical straps stop at the macro. efabless's reference user macro.
    WrapperFed,
}

impl MacroGrid {
    /// `own` / `fed`.
    pub fn parse(s: &str) -> Option<MacroGrid> {
        match s {
            "own" => Some(MacroGrid::Own),
            "fed" => Some(MacroGrid::WrapperFed),
            _ => None,
        }
    }

    /// The flag spelling [`MacroGrid::parse`] reads back.
    pub fn flag(&self) -> &'static str {
        match self {
            MacroGrid::Own => "own",
            MacroGrid::WrapperFed => "fed",
        }
    }
}

/// A macro hardened to sit INSIDE a shuttle vehicle's wrapper — the Caravel user macro.
///
/// Its die is ours to choose, but its power nets and routing ceiling are the vehicle's: it takes the harness's
/// supplies through power pins, and its routing ceiling follows from how the wrapper's grid feeds it
/// ([`MacroGrid`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VehicleMacro {
    pub die_um: (f64, f64),
    /// Extra hold slack the macro closes every path to. The paths the wrapper would see violated start at a
    /// harness input port and end at a flop in here; the wrapper is a cell-free routing shell (cells in it need
    /// taps and fill, which broke its LVS: 0 -> 7,755 with input buffering), so they are closed on THIS side —
    /// by [`vehicle_macro_sdc`], which holds the macro's ports to the harness's real minimum input delays.
    /// `dram_user_project` was hardened before that SDC existed and closed its 97 such paths (the pure shell
    /// ran hold -0.57 ns over them) with a 0.7 ns margin here; a margin that size applies to every internal
    /// path as well, which a macro with tens of thousands of flops cannot afford.
    pub hold_margin_ns: f64,
    pub grid: MacroGrid,
}

struct VehicleCtx<'a> {
    m: &'a VehicleMacro,
    power: (&'static str, &'static str),
    max_layer: &'static str,
}

/// The config for a vehicle user macro, or a refusal when the process's vehicle has not been read.
pub fn vehicle_macro_config(
    d: &DesignProfile,
    m: &VehicleMacro,
    pdk: &PdkTarget,
    c: &ClosureSettings,
) -> Result<String, PdkUnsupported> {
    let ua = pdk
        .user_area
        .ok_or(PdkUnsupported::VehicleUncharacterized { pdk: pdk.name })?;
    let ceiling = match m.grid {
        MacroGrid::Own => ua.macro_max_layer,
        MacroGrid::WrapperFed => ua.fed_macro_max_layer,
    };
    match (ua.macro_power, ceiling) {
        (Some(power), Some(max_layer)) => Ok(emit(
            d,
            pdk,
            c,
            Some(VehicleCtx {
                m,
                power,
                max_layer,
            }),
        )),
        _ => Err(PdkUnsupported::VehicleUncharacterized { pdk: pdk.name }),
    }
}

/// The constraints a vehicle user macro is timed with: OpenLane's own `base.sdc`, then the vehicle's timing as it
/// characterizes it ([`HarnessTiming`](super::pdk_target::HarnessTiming)) — the clock model the chip is signed
/// off with, and every synchronous harness input's minimum delay.
///
/// The inputs: `base.sdc` gives every input one delay, `IO_PCT` of the period, as both its earliest and latest
/// arrival — 8 ns at 40 ns — so a port-to-flop path looks hold-safe inside the macro and is violated one level
/// up, where the harness delivers the Wishbone bus under 2 ns after the edge. With the real minimum on the ports,
/// the macro's own hold repair closes exactly those paths, and nothing else needs a margin. MEASURED on
/// `wpe_user_project` (RUN_2026.09.25_22.53.44): the global 0.7 ns hold margin that closed `dram_user_project`'s
/// 97 port-to-flop paths found 35,656 hold endpoints in the engine's 32,773 flops and inserted 81,316 buffers,
/// 0.81 mm²; the macro went from 32% to 41% utilization and global routing failed on congestion (GRT-0119).
///
/// The clock: `base.sdc`'s 0.25 ns uncertainty and 5 % derate are OpenLane's defaults, not the chip's; Caravel
/// signs itself off at 0.1 ns and 3.75 %, and those are set after `base.sdc` so they are the ones in force.
/// MEASURED on the engine's post-CTS placement (RUN_2026.09.26_03.38.30, all three corners, OpenLane's 0.1 ns
/// repair margin): under the defaults 31,793 of its 35,747 flops fell inside the margin and took a buffer each
/// (32,005), leaving global routing 55 gcells short; 31,089 of them were flops holding their own value through
/// an enable mux — launch and capture on the same flop and the same edge, a path skew cannot reach — and only
/// 87 had negative slack. Under the chip's model 6 endpoints fall inside the margin and 1 is negative: the
/// `wbs_cyc_i` port path, a real violation the repair closes either way.
///
/// `clock` is the harness clock the macro's logic runs on, its `CLOCK_PORT`. On [`HarnessClock::User2`] the harness's
/// bus keeps its own clock: `wb_clk_i` becomes a clock at the chip's core period, asynchronous to the macro's, and
/// every port the harness times against it (`HarnessTiming::core_clock_inputs` and `core_clock_outputs`) moves from
/// `base.sdc`'s delay against `CLOCK_PORT` to one against `wb_clk_i`. Each removal names the clock it removes:
/// MEASURED in the pinned OpenSTA, `unset_input_delay` without `-clock` removes only the delays that have no clock,
/// and `write_sdc` still showed `wb_clk_i` delayed against `user_clock2`.
pub fn vehicle_macro_sdc(pdk: &PdkTarget, clock: HarnessClock) -> Result<String, PdkUnsupported> {
    let h = pdk
        .user_area
        .and_then(|ua| ua.harness)
        .ok_or(PdkUnsupported::VehicleUncharacterized { pdk: pdk.name })?;
    let mut s = String::from("source $::env(SCRIPTS_DIR)/base.sdc\n");
    let _ = writeln!(
        s,
        "set_clock_uncertainty {} [get_clocks $::env(CLOCK_PORT)]",
        h.clock_uncertainty_ns
    );
    let _ = writeln!(
        s,
        "set_timing_derate -early [expr {{1 - {}}}]",
        h.timing_derate
    );
    let _ = writeln!(
        s,
        "set_timing_derate -late [expr {{1 + {}}}]",
        h.timing_derate
    );
    let bus_clock = match clock {
        HarnessClock::Core => "$::env(CLOCK_PORT)",
        HarnessClock::User2 => {
            let (core, p) = (HarnessClock::Core.port(), h.core_clock_period_ns);
            let off = "-clock [get_clocks $::env(CLOCK_PORT)]";
            let on = format!("[expr {{{p} * $::env(IO_PCT)}}] -clock [get_clocks {core}]");
            let _ = writeln!(s, "create_clock [get_ports {core}] -name {core} -period {p}");
            let _ = writeln!(s, "set_clock_uncertainty {} [get_clocks {core}]", h.clock_uncertainty_ns);
            let _ = writeln!(s, "set_clock_transition $::env(SYNTH_CLOCK_TRANSITION) [get_clocks {core}]");
            let _ = writeln!(
                s,
                "set_clock_groups -asynchronous -group [get_clocks $::env(CLOCK_PORT)] -group [get_clocks {core}]"
            );
            let _ = writeln!(s, "unset_input_delay {off} [get_ports {core}]");
            let _ = writeln!(
                s,
                "set_driving_cell -lib_cell $::env(SYNTH_CLK_DRIVING_CELL) -pin $::env(SYNTH_CLK_DRIVING_CELL_PIN) [get_ports {core}]"
            );
            for port in h.core_clock_inputs {
                let _ = writeln!(s, "unset_input_delay {off} [get_ports {{{port}}}]");
                let _ = writeln!(s, "set_input_delay {on} [get_ports {{{port}}}]");
            }
            for port in h.core_clock_outputs {
                let _ = writeln!(s, "unset_output_delay {off} [get_ports {{{port}}}]");
                let _ = writeln!(s, "set_output_delay {on} [get_ports {{{port}}}]");
            }
            core
        }
    };
    for (ports, ns) in h.input_min_ns {
        let _ = writeln!(s, "set_input_delay -min {ns} -clock [get_clocks {bus_clock}] [get_ports {{{ports}}}]");
    }
    Ok(s)
}

fn emit(d: &DesignProfile, pdk: &PdkTarget, c: &ClosureSettings, vm: Option<VehicleCtx>) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "{{");
    let _ = writeln!(s, "  \"DESIGN_NAME\": \"{}\",", d.name);
    let _ = writeln!(s, "  \"VERILOG_FILES\": \"dir::src/*.v\",");
    let _ = writeln!(s, "  \"VERILOG_INCLUDE_DIRS\": \"dir::src\",");
    let _ = writeln!(s, "  \"CLOCK_PORT\": \"{}\",", d.clock_port);
    let _ = writeln!(s, "  \"CLOCK_PERIOD\": {},", d.clock_period_ns);

    // The process, never assumed. A run records these into its own config.tcl, which is where the
    // signoff recorder reads the PDK back from — so the ledger can never disagree with the build.
    let _ = writeln!(s, "  \"STD_CELL_LIBRARY\": \"{}\",", pdk.std_cell_library);
    let top = vm.as_ref().map_or(pdk.top_routing_layer, |v| v.max_layer);
    let _ = writeln!(s, "  \"RT_MAX_LAYER\": \"{top}\",");

    if let Some(st) = c.synth_strategy.or(c.comparative.then_some(COMPARATIVE_SYNTH_STRATEGY)) {
        let _ = writeln!(s, "  \"SYNTH_STRATEGY\": \"{}\",", st.openlane());
    }

    // Electrical closure.
    let _ = writeln!(
        s,
        "  \"PL_RESIZER_MAX_SLEW_MARGIN\": {},",
        c.pl_slew_margin_pct
    );
    let _ = writeln!(
        s,
        "  \"GLB_RESIZER_MAX_SLEW_MARGIN\": {},",
        c.slew_margin_pct
    );
    let _ = writeln!(s, "  \"PL_RESIZER_MAX_CAP_MARGIN\": {},", c.cap_margin_pct);
    let _ = writeln!(s, "  \"GLB_RESIZER_MAX_CAP_MARGIN\": {},", c.cap_margin_pct);
    let _ = writeln!(s, "  \"PL_RESIZER_DESIGN_OPTIMIZATIONS\": 1,");
    let _ = writeln!(s, "  \"GLB_RESIZER_DESIGN_OPTIMIZATIONS\": 1,");
    let _ = writeln!(s, "  \"PL_RESIZER_TIMING_OPTIMIZATIONS\": 1,");
    let _ = writeln!(s, "  \"GLB_RESIZER_TIMING_OPTIMIZATIONS\": 1,");

    // The calibrated pre-route parasitic estimate (see `pdk_target::SKY130A_LAYER_RC`). Every
    // resizer and pre-route STA step sizes against it; uncalibrated, it was optimistic by 1.39x
    // (2.02x at p90) on exactly the heavy nets that then failed slew at signoff.
    if let Some(rc) = pdk.estimator_layer_rc {
        let table: Vec<String> = rc
            .iter()
            .map(|l| {
                format!(
                    "{} {:.4e} {:.4e}",
                    l.layer, l.cap_pf_per_um, l.res_kohm_per_um
                )
            })
            .collect();
        let _ = writeln!(s, "  \"LAYERS_RC\": \"{}\",", table.join(","));
    }

    // The CLOCK input port's assumed driver. OpenLane defaults `SYNTH_CLK_DRIVING_CELL` to
    // `SYNTH_DRIVING_CELL`, so the clock port is modelled as driven by the minimum DATA-input
    // inverter (`inv_2` on sky130). That is wrong in the pessimistic direction on the one net no
    // resizer can fix — an input port's own transition is an assumption, not something buffering
    // can improve — and it is measurably what remains after everything else closes:
    // `systolic_os_pipe8_realbist` reached 3 max-slew violations and ALL THREE were that one net
    // (`clk`, `clkbuf_0_clk/A`, and a diode on it, each at 0.97 against the 0.75 limit);
    // `systolic_pipe16_realbist` showed the identical triple at 1.25.
    //
    // The cell is the PDK's OWN `ROOT_CLK_BUFFER`, read from its config.tcl and carried in
    // `PdkTarget`. A process that has not been installed and read declares `None`, and then
    // nothing is emitted — OpenLane keeps its default rather than being handed a guessed cell
    // name. (It cannot be a `ref::$ROOT_CLK_BUFFER` reference: design-config references resolve
    // before the PDK config is sourced, so the variable does not exist yet.)
    if let Some((cell, pin)) = pdk.clk_driving_cell {
        let _ = writeln!(s, "  \"SYNTH_CLK_DRIVING_CELL\": \"{cell}\",");
        let _ = writeln!(s, "  \"SYNTH_CLK_DRIVING_CELL_PIN\": \"{pin}\",");
    }

    // Hold is a GATE, not a report: the flow refuses to emit a hold-violating block rather than
    // producing one for a recorder to excuse. This is the setting whose absence let a wrapper ship
    // with 97 hold-violating paths under a waiver whose stated basis was false.
    let _ = writeln!(s, "  \"QUIT_ON_HOLD_VIOLATIONS\": 1,");
    if c.comparative {
        let _ = writeln!(s, "  \"QUIT_ON_SETUP_VIOLATIONS\": 0,");
        let _ = writeln!(s, "  \"QUIT_ON_SYNTH_CHECKS\": 0,");
    }

    // Antenna. The diode passes are a CHOICE, not a constant — see AntennaStrategy for the
    // measurement. Emitting them unconditionally (as an earlier version of this function did)
    // silently disabled the caller's ability to turn them off, and an experiment that believed it
    // was running without diodes was in fact running with 106 of them.
    // The two diode passes are emitted INDEPENDENTLY, because they are independent passes with
    // different effects. Tying them to one flag is what made `HeuristicDiodesAndJog` unreachable.
    let (heuristic, grt_repair) = match c.antenna {
        AntennaStrategy::DiodesAndJog => (1, 1),
        AntennaStrategy::JogOnly => (0, 0),
        AntennaStrategy::HeuristicDiodesAndJog => (1, 0),
    };
    let _ = writeln!(s, "  \"GRT_REPAIR_ANTENNAS\": {grt_repair},");
    let _ = writeln!(s, "  \"GRT_ANT_MARGIN\": {},", c.grt_antenna_margin_pct);
    let _ = writeln!(s, "  \"GRT_ANT_ITERS\": {},", c.grt_antenna_iters);
    let _ = writeln!(
        s,
        "  \"LOGOS_DIODES_BEFORE_RESIZE\": {},",
        u8::from(c.diodes_before_resize)
    );
    let _ = writeln!(
        s,
        "  \"LOGOS_ECO_DIODE_FALLBACK\": {},",
        u8::from(c.eco_diode_fallback || c.comparative)
    );
    let _ = writeln!(
        s,
        "  \"LOGOS_POST_DIODE_UPSIZE\": {},",
        u8::from(c.post_diode_upsize)
    );
    let _ = writeln!(
        s,
        "  \"LOGOS_POST_ANTENNA_UPSIZE\": {},",
        u8::from(c.post_antenna_upsize)
    );
    let _ = writeln!(
        s,
        "  \"LOGOS_UPSIZE_MARGIN\": {},",
        c.post_diode_upsize_margin_pct
    );
    let _ = writeln!(
        s,
        "  \"LOGOS_UPSIZE_ALL_CORNERS\": {},",
        u8::from(c.post_diode_upsize_all_corners)
    );
    let _ = writeln!(
        s,
        "  \"LOGOS_GLB_SKIP_PIN_SWAP\": {},",
        u8::from(c.glb_skip_pin_swap)
    );
    // Fanout: every net within the SDC's set_max_fanout, counting the diodes the liberty counts.
    let _ = writeln!(
        s,
        "  \"LOGOS_CTS_FANOUT\": {},",
        u8::from(c.cts_within_fanout)
    );
    let _ = writeln!(
        s,
        "  \"LOGOS_DIODE_FANOUT\": {},",
        u8::from(c.diode_fanout_repair)
    );
    let _ = writeln!(s, "  \"LOGOS_FANOUT_MARGIN\": {},", c.fanout_margin_loads);
    let _ = writeln!(s, "  \"RUN_HEURISTIC_DIODE_INSERTION\": {heuristic},");
    // A vehicle macro's ports face the wrapper's long harness nets, whose metal this flow never
    // sees, so its inputs are protected at the pin; a standalone block's ports are not exposed that way.
    let port_diodes = if vm.is_some() { "both" } else { "none" };
    let _ = writeln!(s, "  \"DIODE_ON_PORTS\": \"{port_diodes}\",");
    let _ = writeln!(
        s,
        "  \"HEURISTIC_ANTENNA_THRESHOLD\": {},",
        c.heuristic_antenna_threshold
    );
    let _ = writeln!(s, "  \"DRT_ANTENNA_ECO\": 1,");
    let _ = writeln!(s, "  \"ECO_JOG\": 1,");
    let _ = writeln!(s, "  \"DRT_ECO_ITERS\": {},", c.antenna_eco_iters);

    // Antenna, continued: the met1 penalty is expressed through the PDK's own metal stack, so the
    // column count can never be wrong for the process (sky130 has six columns including li1;
    // gf180 has five and no li1 — a hand-written vector silently mismatches).
    let _ = writeln!(
        s,
        "  \"GRT_LAYER_ADJUSTMENTS\": \"{}\",",
        pdk.layer_adjustments(0.99, c.met1_route_penalty)
    );
    if let Some(n) = c.grt_overflow_iters {
        let _ = writeln!(s, "  \"GRT_OVERFLOW_ITERS\": {n},");
    }
    if let Some(a) = c.grt_adjustment {
        let _ = writeln!(s, "  \"GRT_ADJUSTMENT\": {a},");
    }
    if let Some(n) = c.dpl_cell_padding_per_side {
        let _ = writeln!(s, "  \"DPL_CELL_PADDING\": {},", 2 * n);
    }

    // Clock tree: the wire from the clock input to the root is buffered to what the slow corner meets.
    let _ = writeln!(s, "  \"CTS_CLK_MAX_WIRE_LENGTH\": {},", c.clock_max_wire_um);
    let _ = writeln!(
        s,
        "  \"PL_RESIZER_MAX_WIRE_LENGTH\": {},",
        c.signal_max_wire_um
    );
    let _ = writeln!(
        s,
        "  \"GLB_RESIZER_MAX_WIRE_LENGTH\": {},",
        c.signal_max_wire_um
    );

    // Floorplan.
    let _ = writeln!(s, "  \"PL_TARGET_DENSITY\": {},", c.target_density);
    if let Some(p) = c.pdn_vpitch_um {
        let _ = writeln!(s, "  \"FP_PDN_VPITCH\": {p},");
    }
    if let Some(m) = c.hold_slack_margin_ns.or(c.comparative.then_some(COMPARATIVE_HOLD_MARGIN_NS)) {
        let _ = writeln!(s, "  \"PL_RESIZER_HOLD_SLACK_MARGIN\": {m},");
        let _ = writeln!(s, "  \"GLB_RESIZER_HOLD_SLACK_MARGIN\": {m},");
    }
    if let Some(m) = c.setup_slack_margin_ns {
        let _ = writeln!(s, "  \"PL_RESIZER_SETUP_SLACK_MARGIN\": {m},");
        let _ = writeln!(s, "  \"GLB_RESIZER_SETUP_SLACK_MARGIN\": {m},");
    }
    if d.fixed_die {
        if let Some(ua) = pdk.user_area {
            let _ = writeln!(s, "  \"FP_SIZING\": \"absolute\",");
            let _ = writeln!(s, "  \"DIE_AREA\": \"{}\",", ua.die_area());
        }
    }
    if let Some(v) = &vm {
        let (w, h) = v.m.die_um;
        let _ = writeln!(s, "  \"FP_SIZING\": \"absolute\",");
        let _ = writeln!(s, "  \"DIE_AREA\": \"0 0 {w} {h}\",");
        let _ = writeln!(s, "  \"VDD_NETS\": [\"{}\"],", v.power.0);
        let _ = writeln!(s, "  \"GND_NETS\": [\"{}\"],", v.power.1);
        let _ = writeln!(s, "  \"SYNTH_USE_PG_PINS_DEFINES\": \"USE_POWER_PINS\",");
        if v.m.grid == MacroGrid::WrapperFed {
            let _ = writeln!(s, "  \"DESIGN_IS_CORE\": 0,");
        }
        let _ = writeln!(s, "  \"FP_PDN_ENABLE_RAILS\": 1,");
        // The pin order MUST mirror the wrapper DEF, or the wrapper's Wishbone nets route ~2.4 mm
        // to reach the pins and come back as antenna violators (scripts/gen-caravel-macro-pinorder.py).
        let _ = writeln!(s, "  \"FP_PIN_ORDER_CFG\": \"dir::pin_order.cfg\",");
        let _ = writeln!(s, "  \"RT_CLOCK_MIN_LAYER\": \"met2\",");
        if pdk.user_area.is_some_and(|ua| ua.harness.is_some()) {
            let _ = writeln!(s, "  \"BASE_SDC_FILE\": \"dir::vehicle_macro.sdc\",");
        }
        let _ = writeln!(
            s,
            "  \"PL_RESIZER_HOLD_SLACK_MARGIN\": {},",
            v.m.hold_margin_ns
        );
        let _ = writeln!(
            s,
            "  \"GLB_RESIZER_HOLD_SLACK_MARGIN\": {},",
            v.m.hold_margin_ns
        );
        let _ = writeln!(s, "  \"MAGIC_WRITE_FULL_LEF\": 0,");
        let _ = writeln!(s, "  \"RUN_CVC\": 0,");
    }
    if !d.macros.is_empty() {
        let lefs: Vec<String> = d
            .macros
            .iter()
            .map(|m| format!("dir::macro/{m}.lef"))
            .collect();
        let _ = writeln!(s, "  \"EXTRA_LEFS\": \"{}\",", lefs.join(" "));
    }

    // Reproducibility, last so it reads as the closing invariant it is.
    let _ = writeln!(s, "  \"ROUTING_CORES\": {}", c.routing_cores);
    let _ = writeln!(s, "}}");
    s
}
