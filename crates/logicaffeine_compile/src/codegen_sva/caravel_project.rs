//! A Caravel user project as data.
//!
//! Caravel carries exactly one user macro, placed as `mprj` inside the fixed `user_project_wrapper`. Everything
//! that differs between two user projects along the chain — macro harden, wrapper harden, golden-deck DRC,
//! submission, `mpw_precheck` — is a field of [`CaravelProject`]. The flow scripts read their project through
//! `emit_flow_config --caravel-project <key>` instead of naming a macro themselves, so a second user project is
//! one more entry in [`CARAVEL_PROJECTS`], never a second copy of the scripts; and the locks walk the same list,
//! so an entry is held to every gate from the moment it exists.

use super::caravel_rtl::emit_user_project_wrapper_around;
use super::flow_config::{MacroGrid, SynthStrategy};
use super::pdk_target::{HarnessClock, HarnessTiming, PdkTarget};
use super::signoff_blocks::registered_period_ns;
use super::wpe_caravel::{WPE_CARAVEL, WPE_CLOCK};

/// One user project on the Caravel harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaravelProject {
    /// The name the flow scripts are invoked with.
    pub key: &'static str,
    /// The user macro's module. Its RTL is `benchmarks/silicon/<macro_module>.v`.
    pub macro_module: &'static str,
    /// What the macro carries, as the wrapper's header names it.
    pub carries: &'static str,
    /// The macro's die, and the lower-left corner the wrapper places it at, in µm of the user area.
    pub macro_die_um: (u32, u32),
    pub macro_origin_um: (u32, u32),
    /// The harness clock the macro's logic runs on, and its period: the clock the chip runs the project at. On the
    /// core clock the period is no shorter than the chip signs that clock off at; `user_clock2` takes its own.
    pub clock: HarnessClock,
    pub clock_period_ps: u32,
    /// The macro's technology-mapping script, when OpenLane's default is not the one it closes with
    /// (see `flow_config::ClosureSettings::synth_strategy`).
    pub synth_strategy: Option<SynthStrategy>,
    /// The hold slack every path of the macro is closed to (`flow_config::VehicleMacro::hold_margin_ns`), in ps.
    pub macro_hold_margin_ps: u32,
    /// The macro's global placement density target, in thousandths, when the closure default does not route it.
    pub macro_density_permille: Option<u16>,
    /// The macro's met1 global-route penalty, in thousandths, when the closure recipe's
    /// (`flow_config::ClosureSettings::met1_route_penalty`) leaves it too little met1 to route.
    pub macro_met1_penalty_permille: Option<u16>,
    /// The macro's global-route overflow iterations, when OpenLane's own 50 do not route it
    /// (`flow_config::ClosureSettings::grt_overflow_iters`).
    pub macro_grt_overflow_iters: Option<u16>,
    /// The share of each gcell's capacity the macro's global route holds back, in thousandths, when OpenLane's own
    /// 0.3 leaves it over capacity (`flow_config::ClosureSettings::grt_adjustment`).
    pub macro_grt_adjustment_permille: Option<u16>,
    /// The share past each antenna ratio the macro's global router repairs to, in percent, when the closure default
    /// (`flow_config::ClosureSettings::grt_antenna_margin_pct`) inserts more diodes than the route has room for.
    pub macro_grt_antenna_margin_pct: Option<u8>,
    /// How many passes the macro's global router makes to repair antennas, when the closure default
    /// (`flow_config::ClosureSettings::grt_antenna_iters`) reroutes the block past what its route holds.
    pub macro_grt_antenna_iters: Option<u8>,
    /// The span, in µm, past which the macro's heuristic pass gives a net a protection diode before routing, when the
    /// closure default (`flow_config::ClosureSettings::heuristic_antenna_threshold`) floods a dense patch with them.
    pub macro_antenna_threshold_um: Option<u16>,
    /// The sites a side the macro's legalization keeps around every cell, when abutting cells leave a dense patch's
    /// pins no room (`flow_config::ClosureSettings::dpl_cell_padding_per_side`).
    pub macro_dpl_padding_per_side: Option<u8>,
    /// The setup slack, in ps, the macro's placement and global-route timing repairs close every path to, when its
    /// routed paths at the max-RC corner outgrow the estimates the repairs judge
    /// (`flow_config::ClosureSettings::setup_slack_margin_ns`).
    pub macro_setup_margin_ps: Option<u32>,
    /// How far below the slew limit the macro's post-diode upsize detects, in percent, at every corner
    /// (`flow_config::ClosureSettings::post_diode_upsize_margin_pct`). The global-route estimate it detects on reads
    /// below the extracted slew signoff judges, by an amount that depends on the design.
    pub macro_upsize_margin_pct: u8,
    /// How the macro takes power from the wrapper, and so how high it routes.
    pub macro_grid: MacroGrid,
    /// The OpenLane designs, under `tools/OpenLane/designs/`, the macro and the wrapper are hardened in.
    pub macro_design: &'static str,
    pub wrapper_design: &'static str,
    /// The wrapper RTL under `benchmarks/silicon/`, staged into the wrapper design under the same name. Its module
    /// is always `user_project_wrapper`, the harness's name for the user area.
    pub wrapper_rtl: &'static str,
    /// The ledger the macro's own block signoff is recorded in.
    pub macro_ledger: &'static str,
    /// The wrapper's signoff record, the key its golden-deck verdict is filed under in `golden_precheck.json`, the
    /// precheck record, and the directory the submission is assembled in — regenerated OpenLane output, kept
    /// beside the runs it is assembled from under the ignored `tools/`.
    pub wrapper_ledger: &'static str,
    pub golden_key: &'static str,
    pub precheck_ledger: &'static str,
    pub submission_dir: &'static str,
    /// The submission files that are this project's own — README, pad configuration, bring-up bench — laid over
    /// the efabless scaffold at the same relative paths.
    pub overlay: &'static str,
}

/// The DRAM command-stream executor behind a Wishbone target: the project submitted and precheck-passed first.
/// Every path is the one its evidence was recorded under.
pub const DRAM_PROJECT: CaravelProject = CaravelProject {
    key: "dram",
    macro_module: "dram_user_project",
    carries: "certified command player",
    macro_die_um: (2800, 1760),
    macro_origin_um: (60, 880),
    clock: HarnessClock::Core,
    clock_period_ps: 25_000,
    synth_strategy: None,
    macro_hold_margin_ps: 700,
    macro_density_permille: None,
    macro_met1_penalty_permille: None,
    macro_grt_overflow_iters: None,
    macro_grt_adjustment_permille: None,
    macro_grt_antenna_margin_pct: None,
    macro_grt_antenna_iters: None,
    macro_antenna_threshold_um: None,
    macro_dpl_padding_per_side: None,
    macro_setup_margin_ps: None,
    // The all-corner margin that closed `systolic_pipe16_realbist` and `systolic_os_pipe8_realbist`
    // (RUN_2026.09.24_16.37.47 / _19.16.31), on which their slew violators were all slow-corner only.
    macro_upsize_margin_pct: 20,
    macro_grid: MacroGrid::Own,
    macro_design: "logos_upm",
    wrapper_design: "logos_upw",
    wrapper_rtl: "user_project_wrapper.v",
    macro_ledger: "benchmarks/silicon/caravel_macro_signoff.json",
    wrapper_ledger: "benchmarks/silicon/caravel_signoff.json",
    golden_key: "user_project_wrapper",
    precheck_ledger: "benchmarks/silicon/mpw_precheck.json",
    submission_dir: "tools/caravel_submissions/dram",
    overlay: "benchmarks/silicon/caravel/dram",
};

/// The whole-program engine ([`wpe_caravel`](super::wpe_caravel)): its UART and its PSRAM on user GPIOs, clocked
/// on `user_clock2` at the clock its UART bit period was derived from, faster than the chip signs its core clock
/// for. Its ports are held to the harness's minimum input delays
/// (`flow_config::vehicle_macro_sdc`), so its hold margin is OpenLane's placement default, 0.1 ns: the 0.7 ns the
/// command player carries buffered 35,656 of the engine's paths.
pub const WPE_PROJECT: CaravelProject = CaravelProject {
    key: "wpe",
    macro_module: "wpe_user_project",
    carries: "whole-program engine",
    macro_die_um: (2800, 3360),
    macro_origin_um: (60, 80),
    clock: WPE_CLOCK,
    clock_period_ps: WPE_CARAVEL.clk_period_ps,
    synth_strategy: Some(SynthStrategy::Area(3)),
    macro_hold_margin_ps: 100,
    // MEASURED: at the default 0.45 the placer packed 31% utilization into the center-right of the die and raised
    // its own target to 0.51 for routability; global routing under the met3 ceiling then overflowed by 1,188 in a
    // 1.6 x 1.2 mm patch with the die's left third empty (RUN_2026.09.26_01.14.22, GRT-0119). At 0.38 (raised to
    // 0.416) it overflowed in 55 gcells beside 32,005 hold buffers (RUN_2026.09.26_03.38.30), and in 2 gcells,
    // each by one track, around the FPU multiplier once the chip's signoff clock left 16
    // (RUN_2026.09.26_05.24.57). Spreading further does not help: at 0.33 the wirelength grew 6% and 230 gcells
    // overflowed (RUN_2026.09.26_07.55.00).
    macro_density_permille: Some(380),
    // MEASURED: the recipe's 0.7 leaves met1 20% of its tracks (785,461 of 3,936,600 after the rails), and with met2
    // the only vertical layer under the met3 ceiling, the engine's global route came up two gcells short, both
    // horizontal, by one track each (RUN_2026.09.26_05.24.57). The recipe's penalty closed one met1 side-area
    // antenna violator on `gpu_simt`; this block's antenna stays with the diodes and the jog ECO. MEASURED at 18.5 ns,
    // the route detailed routing follows (step 30) replayed from RUN_2026.10.24_03.18.57's step-29 database: at 0.5 it
    // overflowed 248 gcells by a track in one 270 x 390 um patch over the integer multiplier, at 0.4 53, and at 0.4
    // with the gcell adjustment at 0.05 none.
    macro_met1_penalty_permille: Some(400),
    // MEASURED at 16 ns in the private tree: at OpenLane's 0.3 adjustment the first global route (step 15) stopped at
    // GRT-0119 at 50, 150 and 400 iterations alike, the iterations ending once the overflow could grow no further
    // (RUN_2026.10.19_19.38.51, RUN_2026.10.19_20.34.27, RUN_2026.10.19_22.07.52); at 0.2 with 150 iterations it was
    // clean (RUN_2026.10.20_17.57.25, x5 and x6).
    macro_grt_overflow_iters: Some(150),
    // MEASURED: at 0.2 the route detailed routing takes (step 25) overflowed in 72 gcells, by one track each over a
    // ~19-track edge, all in one 400 x 190 um patch, once the fanout step had split 18,096 nets with 22,708 buffers and
    // the timing repair had resized 7,575 cells (RUN_2026.10.20_17.57.25, x6, with the met1 penalty at 0.3 besides).
    // 0.1 gives every layer an eighth more capacity than 0.2, against the twentieth that patch lacked. MEASURED at
    // 18.5 ns on the same step-30 replay as the met1 penalty: at 0.1 the patch overflowed 248 gcells, at 0.05 83, and at
    // 0.05 with met1 at 0.4 none.
    macro_grt_adjustment_permille: Some(50),
    // MEASURED at 18.5 ns on the registered cache-hit RTL, the global route replayed from RUN_2026.10.22_16.11.23's
    // step-24 database: at the closure default of 50% the antenna repair's first pass found 1,412 violations and
    // inserted 5,981 diodes, and its reroute of 11,725 nets stopped at GRT-0232; at 20% the first pass found 599 and
    // inserted 1,537, and the next found 46 and 26. At either margin the repair stalls near twenty nets that each need
    // more than ten diodes a gate, as at 48 ns, where the passes left 16 and 17 (RUN_2026.09.27_18.14.14,
    // RUN_2026.09.28_20.20.58) and the block signed off antenna-clean.
    macro_grt_antenna_margin_pct: Some(20),
    // MEASURED on the same route at 20%: the passes found 599, 46, 26, 21, 16, 24 and 17 violations
    // (RUN_2026.10.23_21.04.53), oscillating from the fourth on, as the earlier RTL's 18 passes at 50% oscillated
    // between 27 and 44 (RUN_2026.10.21_09.00.47). Each pass reroutes the ~150 nets it touched, ever more slowly: the
    // fourth took 100 minutes, the seventh had not ended after 14 attempts, and at 50% with a met1 adjustment of 0.4
    // the thirteenth stopped at GRT-0232. Three passes route in 1 h 38 m and leave the same score of nets to the
    // repair after detailed routing.
    macro_grt_antenna_iters: Some(3),
    // MEASURED at 18.5 ns: at the closure default of 300 um the heuristic pass inserted 57,637 diodes
    // (RUN_2026.10.24_03.18.57), and the congested patch held 4,844 of its 14,681 cells as diodes, at 0.552 utilization
    // against the core's 0.345. With both route levers the step-30 route then ran clean, but the slew rounds' buffers
    // filled the patch until a reroute stopped at GRT-0232. The pass on the same placement inserts 39,821 at 400 um
    // (2,536 of the patch's 3,798 before legalization), 28,338 at 500 and 12,226 at 700. All three then routed steps 25
    // and 30, but detailed routing could not clear the met1 shorts the remaining diodes crowd in: 930 violations left
    // after 21 passes at 400 um (RUN_2026.10.24_12.25.42) and 409, still falling by a few a pass, after 26 at 500
    // (logos_wpe_upm_t500's RUN_2026.10.24_12.26.00), where 700 started its first pass with the fewest, 251,562
    // against 265,814 and 273,753.
    macro_antenna_threshold_um: Some(700),
    // MEASURED at 18.5 ns, detailed routing replayed from RUN_2026.10.24_18.23.48's step-30 database (the recipe above)
    // after legalizing it again: with the cells abutting, its met1 shorts stalled at 74 violations from the 38th pass
    // to the 46th, where it was stopped; with a site kept free a side the passes left 591 against 909 at the 11th, 80
    // against 179 at the 21st, 18 (10 met1 shorts, 5 met1 spacings, 3 on met2) from the 39th, 5 at the 50th, and none
    // at the 56th; with two sites a side the patch over the multiplier filled, and the passes left 3,777 against one
    // site's 2,171 at the 9th and stalled at 201 from the 22nd. The full flow with a site a side at every legalization
    // (RUN_2026.10.25_06.23.41) held 78 from its 46th pass to its 51st, every one in a 170 x 90 um patch over the
    // integer multiplier, met1 shorts at the pins of its nand2_1, a22o_1 and o21ai_1 gates.
    macro_dpl_padding_per_side: Some(1),
    // MEASURED at 18.5 ns on RUN_2026.10.25_06.23.41, every net routed (its detailed routing replayed to the 36th pass)
    // and timed at ss_100C_1v60: the repairs had closed setup to +0.03 ns on their global-route estimates, and the
    // routed design read -0.37 ns extracted at the nominal RC corner and -1.13 ns at the max-RC corner signoff judges,
    // 1.16 ns past the estimate. At 21 ns, on RUN_2026.10.26_16.02.11, the repairs had nothing to do: the estimate
    // read +0.757 ns before the fanout step's repair and after it.
    macro_setup_margin_ps: Some(1250),
    // MEASURED: at 20% (0.600 ns) the upsize converged to zero, yet net `_117618_` — 773 um, driven by an `a21o_4`,
    // the top of its family — signed off at 0.757 ns at max RC (RUN_2026.09.27_18.14.14); a re-route of the same
    // netlist estimated it at 0.635. Every pin of the engine signing off above 0.70 ns estimates at 0.56 ns or more,
    // so 0.540 ns catches all of them with 0.05 ns for that route-to-route noise: 1,752 pins, where 0.600 caught 21.
    macro_upsize_margin_pct: 28,
    // MEASURED: with the penalty eased, met1 held 1,423,779 tracks and the misses moved to met2, the only vertical
    // layer under the met3 ceiling: 59 of 89 overflowing gcells vertical (RUN_2026.09.26_09.04.50). Fed by the
    // wrapper, the engine routes on met4 as well.
    macro_grid: MacroGrid::WrapperFed,
    macro_design: "logos_wpe_upm",
    wrapper_design: "logos_wpe_upw",
    wrapper_rtl: "wpe_user_project_wrapper.v",
    macro_ledger: "benchmarks/silicon/caravel_macro_signoff.json",
    wrapper_ledger: "benchmarks/silicon/caravel_signoff_wpe.json",
    golden_key: "user_project_wrapper_wpe",
    precheck_ledger: "benchmarks/silicon/mpw_precheck_wpe.json",
    submission_dir: "tools/caravel_submissions/wpe",
    overlay: "benchmarks/silicon/caravel/wpe",
};

pub const CARAVEL_PROJECTS: &[CaravelProject] = &[DRAM_PROJECT, WPE_PROJECT];

/// Why a project cannot be hardened as described.
#[derive(Debug, Clone, PartialEq)]
pub enum CaravelProjectUnplaceable {
    /// The process has no shuttle vehicle characterized here.
    NoVehicle { pdk: &'static str },
    /// The vehicle's harness timing — its input delays and the clock the chip is signed off with — has not been
    /// read, so nothing on it can be timed.
    VehicleUntimed { pdk: &'static str },
    /// The macro, at its origin, does not lie inside the user area.
    MacroOutsideUserArea {
        corner_um: (u32, u32),
        user_area_um: (f64, f64),
    },
    /// The macro is signed off at a period longer than the clock the chip runs at, so the chip's clock is a
    /// frequency the macro does not guarantee.
    MacroSlowerThanChip {
        macro_period_ns: f64,
        chip_period_ns: f64,
    },
    /// The project runs on the core clock faster than the chip signs that clock off at, so the management SoC and
    /// the bus on the same clock would run at a frequency the chip does not guarantee.
    CoreClockPastSignoff { period_ns: f64, signoff_ns: f64 },
}

impl std::fmt::Display for CaravelProjectUnplaceable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoVehicle { pdk } => write!(f, "{pdk} has no shuttle vehicle characterized (PdkTarget::user_area)"),
            Self::VehicleUntimed { pdk } => {
                write!(f, "{pdk}'s shuttle vehicle has no harness timing characterized (UserArea::harness)")
            }
            Self::MacroOutsideUserArea { corner_um, user_area_um } => write!(
                f,
                "the macro's far corner ({}, {}) um lies outside the {} x {} um user area",
                corner_um.0, corner_um.1, user_area_um.0, user_area_um.1
            ),
            Self::MacroSlowerThanChip { macro_period_ns, chip_period_ns } => write!(
                f,
                "the macro is signed off at {macro_period_ns} ns but the chip runs at {chip_period_ns} ns"
            ),
            Self::CoreClockPastSignoff { period_ns, signoff_ns } => write!(
                f,
                "the project runs the core clock (wb_clk_i) at {period_ns} ns, which the chip signs off at {signoff_ns} ns; \
                 a faster project runs on user_clock2"
            ),
        }
    }
}

impl CaravelProject {
    pub fn by_key(key: &str) -> Option<&'static CaravelProject> {
        CARAVEL_PROJECTS.iter().find(|p| p.key == key)
    }

    pub fn clock_period_ns(&self) -> f64 {
        self.clock_period_ps as f64 / 1000.0
    }

    /// The period the macro is hardened at: its registered signoff period once a run has measured one, and the
    /// chip's clock for the build that takes the first measurement.
    pub fn macro_period_ns(&self) -> f64 {
        registered_period_ns(self.macro_module).unwrap_or_else(|| self.clock_period_ns())
    }

    /// The wrapper RTL: the harness's `user_project_wrapper` around this project's macro.
    pub fn wrapper_verilog(&self) -> String {
        emit_user_project_wrapper_around(self.macro_module, self.carries)
    }

    /// The project fits the process's vehicle, the vehicle's timing is known, the macro guarantees the chip's
    /// clock, and a project on the core clock runs it no faster than the chip signs it off at.
    pub fn check(&self, pdk: &PdkTarget) -> Result<(), CaravelProjectUnplaceable> {
        let ua = pdk
            .user_area
            .ok_or(CaravelProjectUnplaceable::NoVehicle { pdk: pdk.name })?;
        let harness = ua
            .harness
            .ok_or(CaravelProjectUnplaceable::VehicleUntimed { pdk: pdk.name })?;
        let corner = (
            self.macro_origin_um.0 + self.macro_die_um.0,
            self.macro_origin_um.1 + self.macro_die_um.1,
        );
        if f64::from(corner.0) > ua.width_um || f64::from(corner.1) > ua.height_um {
            return Err(CaravelProjectUnplaceable::MacroOutsideUserArea {
                corner_um: corner,
                user_area_um: (ua.width_um, ua.height_um),
            });
        }
        if let Some(p) = registered_period_ns(self.macro_module) {
            if p > self.clock_period_ns() {
                return Err(CaravelProjectUnplaceable::MacroSlowerThanChip {
                    macro_period_ns: p,
                    chip_period_ns: self.clock_period_ns(),
                });
            }
        }
        if self.clock == HarnessClock::Core && self.clock_period_ns() < harness.core_clock_period_ns {
            return Err(CaravelProjectUnplaceable::CoreClockPastSignoff {
                period_ns: self.clock_period_ns(),
                signoff_ns: harness.core_clock_period_ns,
            });
        }
        Ok(())
    }

    /// The clocks the wrapper is timed at, in ns: `clk` on the core clock, at this project's period when it runs
    /// there and at the chip's signoff period when it does not, and `usr_clk` on `user_clock2` at this project's
    /// period when it runs there.
    pub fn wrapper_clocks_ns(&self, harness: &HarnessTiming) -> (f64, Option<f64>) {
        match self.clock {
            HarnessClock::Core => (self.clock_period_ns(), None),
            HarnessClock::User2 => (harness.core_clock_period_ns, Some(self.clock_period_ns())),
        }
    }

    /// What the flow scripts `eval`: the project, then the clock the vehicle's chip is signed off with, which every
    /// harden of the chain is timed at, then the clocks the wrapper is timed at ([`CaravelProject::wrapper_clocks_ns`],
    /// `none` for a clock it does not have) — or the refusal, when the project does not fit the vehicle.
    pub fn flow_shell(&self, pdk: &PdkTarget) -> Result<String, CaravelProjectUnplaceable> {
        self.check(pdk)?;
        let harness = pdk
            .user_area
            .and_then(|ua| ua.harness)
            .ok_or(CaravelProjectUnplaceable::VehicleUntimed { pdk: pdk.name })?;
        let (core, user2) = self.wrapper_clocks_ns(&harness);
        Ok(format!(
            "{}{}CW_CORE_PERIOD_NS={core}\nCW_USER_CLOCK2_PERIOD_NS={}\n",
            self.shell(),
            harness.shell(),
            user2.map_or("none".to_string(), |u| format!("{u}"))
        ))
    }

    /// The project as `NAME=value` lines for the flow scripts to `eval`. Every value is a bare word — an
    /// identifier, a number or a repo-relative path — so the assignment needs no quoting.
    pub fn shell(&self) -> String {
        let vars: [(&str, String); 31] = [
            ("CP_KEY", self.key.to_string()),
            ("CP_MACRO", self.macro_module.to_string()),
            ("CP_MACRO_DIE_W", self.macro_die_um.0.to_string()),
            ("CP_MACRO_DIE_H", self.macro_die_um.1.to_string()),
            ("CP_MACRO_X", self.macro_origin_um.0.to_string()),
            ("CP_MACRO_Y", self.macro_origin_um.1.to_string()),
            ("CP_CLOCK_PORT", self.clock.port().to_string()),
            ("CP_CLOCK_PERIOD_NS", format!("{}", self.clock_period_ns())),
            ("CP_MACRO_PERIOD_NS", format!("{}", self.macro_period_ns())),
            (
                "CP_SYNTH_STRATEGY",
                self.synth_strategy
                    .map_or("default".to_string(), |st| st.flag()),
            ),
            (
                "CP_HOLD_MARGIN_NS",
                format!("{}", self.macro_hold_margin_ps as f64 / 1000.0),
            ),
            (
                "CP_DENSITY",
                self.macro_density_permille
                    .map_or("default".to_string(), |d| format!("{}", d as f64 / 1000.0)),
            ),
            (
                "CP_MET1_PENALTY",
                self.macro_met1_penalty_permille
                    .map_or("default".to_string(), |p| format!("{}", p as f64 / 1000.0)),
            ),
            (
                "CP_GRT_OVERFLOW_ITERS",
                self.macro_grt_overflow_iters
                    .map_or("default".to_string(), |n| n.to_string()),
            ),
            (
                "CP_GRT_ADJUSTMENT",
                self.macro_grt_adjustment_permille
                    .map_or("default".to_string(), |a| format!("{}", a as f64 / 1000.0)),
            ),
            (
                "CP_GRT_ANT_MARGIN",
                self.macro_grt_antenna_margin_pct
                    .map_or("default".to_string(), |m| m.to_string()),
            ),
            (
                "CP_GRT_ANT_ITERS",
                self.macro_grt_antenna_iters
                    .map_or("default".to_string(), |n| n.to_string()),
            ),
            (
                "CP_ANTENNA_THRESHOLD",
                self.macro_antenna_threshold_um
                    .map_or("default".to_string(), |um| um.to_string()),
            ),
            (
                "CP_DPL_PADDING",
                self.macro_dpl_padding_per_side
                    .map_or("default".to_string(), |n| n.to_string()),
            ),
            (
                "CP_SETUP_MARGIN",
                self.macro_setup_margin_ps
                    .map_or("default".to_string(), |ps| format!("{}", ps as f64 / 1000.0)),
            ),
            ("CP_UPSIZE_MARGIN", self.macro_upsize_margin_pct.to_string()),
            ("CP_MACRO_GRID", self.macro_grid.flag().to_string()),
            ("CP_MACRO_DESIGN", self.macro_design.to_string()),
            ("CP_WRAPPER_DESIGN", self.wrapper_design.to_string()),
            ("CP_WRAPPER_RTL", self.wrapper_rtl.to_string()),
            ("CP_MACRO_LEDGER", self.macro_ledger.to_string()),
            ("CP_WRAPPER_LEDGER", self.wrapper_ledger.to_string()),
            ("CP_GOLDEN_KEY", self.golden_key.to_string()),
            ("CP_PRECHECK_LEDGER", self.precheck_ledger.to_string()),
            ("CP_SUBMISSION", self.submission_dir.to_string()),
            ("CP_OVERLAY", self.overlay.to_string()),
        ];
        let mut s = String::new();
        for (k, v) in vars {
            assert!(
                !v.is_empty()
                    && v.chars()
                        .all(|c| c.is_ascii_alphanumeric() || "_./-".contains(c)),
                "{k}={v:?} is not a bare word"
            );
            s.push_str(&format!("{k}={v}\n"));
        }
        s
    }
}
