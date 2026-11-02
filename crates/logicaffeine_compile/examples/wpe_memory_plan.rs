//! What of the whole-program engine actually fits a shuttle die — solved, not guessed.
//!
//! The WPE's deployed `stream` profile wants 3.65 Mbit of on-chip memory against a 10.28 mm^2
//! Caravel user area. This asks [`plan_wpe`] — the same code `wpe_floorplan_lock` pins — what that
//! costs in real macros on a real process, what has to move off-chip, and what the ASIC profile
//! solved from MEASURED program demands looks like instead.
//!
//!   cargo run -q -p logicaffeine-compile --example wpe_memory_plan [-- --pdk sky130A]

use logicaffeine_compile::codegen_sva::pdk_target::PdkTarget;
use logicaffeine_compile::codegen_sva::wpe_floorplan::{
    plan_wpe, ArrayRole, Residence, WpeDemands, WpeFloorplan, WpeProfile,
};
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::WPE_STREAM_CAPS;

/// The MEASURED ceiling across the whole lowering benchmark suite, from
/// `wprog_all_benchmarks_lane::print_max_dims_across_all_benchmarks`. `depth` and `heap_words` are
/// the two that cannot be read off a config and must be obtained by RUNNING the program.
const SUITE: WpeDemands = WpeDemands {
    blocks: 38,
    registers: 175,
    cells: 920,
    cells_per_block: 450,
    writes: 395,
    liveins: 258,
    call_args: 24,
    konsts: 91,
    depth: 26,
    heap_words: 18_300,
};

/// `wpe_soc_fpga` WITH_FPU=1 is 203,034 generic cells. A floorplan that fits the memory by leaving
/// no room for the engine has not fit anything.
const ENGINE_CELLS: u64 = 203_034;

fn show(label: &str, pdk: &'static PdkTarget, p: &WpeProfile, off: &[ArrayRole]) {
    let area = pdk.user_area.map(|a| a.mm2()).unwrap_or(f64::NAN);
    println!("\n=== {label} ===");
    println!(
        "  caps NREG {} MAXBLK {} MAXCELL {} MAXWR {} MAXV {} MAXK {} DEPTH {} HEAP {}",
        p.nreg, p.maxblk, p.maxcell, p.maxwr, p.maxv, p.maxk, p.depth, p.heap
    );
    println!(
        "  {} bits of memory declared by the RTL",
        p.total_bits(true)
    );
    if off.is_empty() {
        println!("  off-chip policy: NOTHING (everything on die)");
    } else {
        println!("  off-chip policy: {off:?}");
    }
    match plan_wpe(pdk, p, off, ENGINE_CELLS) {
        Err(e) => println!("  REFUSED: {e:?}"),
        Ok(f) => {
            print_fit(&f, area);
        }
    }
}

fn print_fit(f: &WpeFloorplan, area: f64) {
    println!(
        "  FITS: macro {:.2} + flops {:.2} = memory {:.2} mm^2, logic {:.2} mm^2, \
         {:.2} mm^2 spare of {area:.2}",
        f.macro_mm2,
        f.flop_mm2,
        f.memory_mm2,
        f.logic_mm2,
        area - f.memory_mm2 - f.logic_mm2
    );
    println!(
        "  {} macro instances, {} bits off-chip",
        f.macro_instances, f.off_chip_bits
    );
    let mut on: Vec<_> = f
        .placements
        .iter()
        .filter(|p| !matches!(p.residence, Residence::OffChip))
        .collect();
    on.sort_by(|a, b| b.area_mm2.total_cmp(&a.area_mm2));
    println!("  largest on-die arrays:");
    for p in on.iter().take(6) {
        let how = match &p.residence {
            Residence::Macros {
                macro_name,
                instances,
            } => format!("{instances} x {macro_name}"),
            Residence::Flops { cells } => format!("{cells} flop-cells"),
            Residence::OffChip => unreachable!(),
        };
        println!(
            "    {:<14} {:>6} x {:>2}  {:>7.3} mm^2  {how}",
            p.array.name, p.array.words, p.array.bits, p.area_mm2
        );
    }
}

fn main() {
    let name = std::env::args()
        .skip_while(|a| a != "--pdk")
        .nth(1)
        .unwrap_or_else(|| "sky130A".into());
    let pdk = PdkTarget::by_name(&name).expect("a supported PDK");
    println!(
        "WPE on {} — user area {:.2} mm^2",
        pdk.name,
        pdk.user_area.expect("a vehicle").mm2()
    );

    let stream = WpeProfile::from_caps(WPE_STREAM_CAPS);
    show(
        "DEPLOYED FPGA `stream` profile, everything on die",
        pdk,
        &stream,
        &[],
    );
    show(
        "DEPLOYED profile, heap + frame stack off-chip",
        pdk,
        &stream,
        &[ArrayRole::Heap, ArrayRole::FrameStack],
    );

    let asic = SUITE.smallest_covering_profile();
    show(
        "SOLVED ASIC profile, heap + frame stack off-chip",
        pdk,
        &asic,
        &[ArrayRole::Heap, ArrayRole::FrameStack],
    );
    show(
        "SOLVED ASIC profile, heap + frame stack + CONFIG IMAGE off-chip",
        pdk,
        &asic,
        &[
            ArrayRole::Heap,
            ArrayRole::FrameStack,
            ArrayRole::ConfigImage,
        ],
    );

    println!(
        "\nThe FPGA profile's big numbers are Xilinx artifacts, not program demands: HEAP {} is a\n\
         power of two so a 15-bit address lands on ONE RAMB36E1 tile, and NREG {} likewise — and\n\
         `sfile` is DEPTH x NREG, so the inflated NREG is paid for {} times over. An ASIC has no\n\
         RAMB tiles. Solved against the measured suite: HEAP {}, NREG {}.",
        stream.heap, stream.nreg, stream.depth, asic.heap, asic.nreg
    );
}
