//! The clock period every hardened block is signed off at — held here, derived from its measured
//! SLOW corner, and the only period `emit_flow_config` accepts for a registered block.
//!
//! Signoff means every corner closes (ss_100C_1v60 / tt_025C_1v80 / ff_n40C_1v95 x min/nom/max RC),
//! at a period set to what the slow corner actually meets. A period chosen at the typical corner is a
//! frequency the part does not guarantee: measured 2026-09-23, 16 of 18 blocks then recorded clean
//! failed setup at the slow corner, `fabric_soc` by 121.40 ns on a 300 ns clock.

use super::flow_config::SynthStrategy;

/// One hardened block's signoff clock.
#[derive(Debug, Clone, Copy)]
pub struct SignoffBlock {
    pub name: &'static str,
    pub period_ns: f64,
    /// The synthesis strategy the period was measured under, `None` for OpenLane's default (AREA 0). abc takes the
    /// period as its delay target, and the strategy decides what it does with it: a period found under DELAY 0 is
    /// not met by an AREA 0 rebuild, so the period holds only under this strategy.
    pub synth_strategy: Option<SynthStrategy>,
    /// The measurement this period was derived from.
    pub basis: &'static str,
}

/// Headroom kept above the measured slow-corner requirement. Lengthening the period relaxes
/// timing-driven synthesis and resizing, so the rebuilt critical path can come out a little slower
/// than the one measured; 5% absorbs that without a second round.
pub const SLOW_CORNER_MARGIN: f64 = 1.05;

/// The period a block must be built at, given the period it was measured at and the worst setup
/// slack across every corner of that run. A block that met every corner keeps its period; one that
/// missed gets the miss plus [`SLOW_CORNER_MARGIN`], rounded UP to the 0.5 ns grid.
///
/// A slack reported as `-0.00` is a miss smaller than the report's resolution, not a pass, so the
/// sign — not the magnitude — decides.
pub fn honest_period_ns(period_ns: f64, slowest_setup_slack_ns: f64) -> f64 {
    if !slowest_setup_slack_ns.is_sign_negative() {
        return period_ns;
    }
    let required = (period_ns - slowest_setup_slack_ns) * SLOW_CORNER_MARGIN;
    (required * 2.0 - 1e-9).ceil() / 2.0
}

macro_rules! block {
    ($name:literal, $was:literal, $slack:literal, $run:literal) => {
        block!($name, $was, $slack, $run, None)
    };
    ($name:literal, $was:literal, $slack:literal, $run:literal, $strategy:expr) => {
        SignoffBlock {
            name: $name,
            period_ns: honest_period_ns($was, $slack),
            synth_strategy: $strategy,
            basis: concat!(
                "worst setup across every corner ",
                stringify!($slack),
                " ns at ",
                stringify!($was),
                " ns, ",
                $run
            ),
        }
    };
}

/// A block not yet built takes its first period from a sibling built from the same core — the
/// sibling's run, period and worst slack, named as such. Once the block's own run is recorded, its entry
/// becomes a [`block!`] on that run — so between new blocks no entry uses it.
#[allow(unused_macros)]
macro_rules! seeded {
    ($name:literal from $sibling:literal, $was:literal, $slack:literal, $run:literal) => {
        SignoffBlock {
            name: $name,
            period_ns: honest_period_ns($was, $slack),
            synth_strategy: None,
            basis: concat!(
                "seeded from ",
                $sibling,
                ": worst setup across every corner ",
                stringify!($slack),
                " ns at ",
                stringify!($was),
                " ns, ",
                $run
            ),
        }
    };
}

/// Every hardened block, with the run its measurement came from: the period that run was built at
/// and its worst setup slack across every corner, so the derivation is visible, not just its result.
/// A block iterates: when a run at the derived period still misses, its entry moves to THAT run.
pub static SIGNOFF_BLOCKS: std::sync::LazyLock<Vec<SignoffBlock>> =
    std::sync::LazyLock::new(|| {
        vec![
            block!(
                "rev_alu_core",
                7.7,
                0.14,
                "RUN_2026.10.17_18.20.14",
                Some(SynthStrategy::Delay(0))
            ),
            block!("soft_core_scalar", 15.0, 0.41, "RUN_2026.10.15_17.14.37"),
            block!(
                "cgra_sumto",
                6.4,
                0.11,
                "RUN_2026.10.17_17.32.11",
                Some(SynthStrategy::Delay(0))
            ),
            block!("nested_tri", 16.5, 0.07, "RUN_2026.10.15_08.18.56"),
            block!(
                "dram_executor",
                5.0,
                0.03,
                "RUN_2026.10.17_16.50.21",
                Some(SynthStrategy::Delay(0))
            ),
            block!(
                "infinichip",
                8.1,
                0.01,
                "RUN_2026.10.18_18.34.30",
                Some(SynthStrategy::Delay(0))
            ),
            block!(
                "dram_executor_wb",
                4.8,
                0.12,
                "RUN_2026.10.17_17.19.18",
                Some(SynthStrategy::Delay(0))
            ),
            block!("dpu_ingest", 14.5, 1.66, "RUN_2026.10.15_16.56.32"),
            block!("coproc_core", 67.0, 0.31, "RUN_2026.10.15_18.29.32"),
            block!("gpu_simt", 19.5, 0.28, "RUN_2026.10.15_16.58.45"),
            block!(
                "decision_hft_bist",
                4.4,
                0.06,
                "RUN_2026.10.17_17.04.21",
                Some(SynthStrategy::Delay(0))
            ),
            block!(
                "decision_hft_realbist",
                10.8,
                0.02,
                "RUN_2026.10.18_16.36.28",
                Some(SynthStrategy::Delay(0))
            ),
            block!("fabric_horner_seq", 41.0, 0.98, "RUN_2026.10.20_17.57.25"),
            block!("fabric_soc", 468.0, 6.89, "RUN_2026.10.15_17.51.57"),
            block!(
                "gemm_mac_realbist",
                9.4,
                0.06,
                "RUN_2026.10.18_19.21.23",
                Some(SynthStrategy::Delay(0))
            ),
            block!(
                "systolic_tile_realbist",
                21.0,
                2.62,
                "RUN_2026.10.15_17.21.09"
            ),
            block!(
                "systolic_pipe8_realbist",
                15.0,
                0.34,
                "RUN_2026.10.15_17.27.18"
            ),
            block!(
                "systolic_os_pipe8_realbist",
                20.5,
                1.86,
                "RUN_2026.10.20_18.30.02"
            ),
            block!("dram_user_project", 20.0, 7.15, "RUN_2026.10.16_07.41.44"),
            block!(
                "systolic_pipe16_realbist",
                19.5,
                0.47,
                "RUN_2026.10.17_05.49.26"
            ),
            block!("soft_core_realbist", 24.5, 0.4, "RUN_2026.10.15_17.39.59"),
            block!(
                "psram_edge",
                5.8,
                0.05,
                "RUN_2026.10.17_18.00.41",
                Some(SynthStrategy::Delay(0))
            ),
        ]
    });

/// The registered signoff period for `name`, if the block is registered.
pub fn registered_period_ns(name: &str) -> Option<f64> {
    SIGNOFF_BLOCKS
        .iter()
        .find(|b| b.name == name)
        .map(|b| b.period_ns)
}

/// The synthesis strategy `name`'s period was measured under (`Some(None)` for OpenLane's default), if the block is
/// registered.
pub fn registered_synth_strategy(name: &str) -> Option<Option<SynthStrategy>> {
    SIGNOFF_BLOCKS
        .iter()
        .find(|b| b.name == name)
        .map(|b| b.synth_strategy)
}

/// The strategy `name` is synthesized under, given the one asked for (`None`: none asked; `Some(None)`: OpenLane's
/// default). A registered block takes its registered strategy and refuses any other; an unregistered block takes
/// what was asked.
pub fn resolve_synth_strategy(
    name: &str,
    asked: Option<Option<SynthStrategy>>,
) -> Result<Option<SynthStrategy>, String> {
    let spell = |s: Option<SynthStrategy>| s.map_or_else(|| "default".to_string(), |s| s.flag());
    match (registered_synth_strategy(name), asked) {
        (Some(reg), Some(a)) if a != reg => Err(format!(
            "{} is not the registered synthesis strategy for {name}: it is {}, the strategy its signoff period was \
             measured under (codegen_sva::signoff_blocks)",
            spell(a),
            spell(reg)
        )),
        (Some(reg), _) => Ok(reg),
        (None, a) => Ok(a.flatten()),
    }
}
