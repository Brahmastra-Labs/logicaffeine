//! wpe_floorplan_lock — the whole-program engine's memory, planned against a REAL process before a
//! single hour of place-and-route is spent on it.
//!
//! WHY THIS EXISTS. The WPE is the shuttle payload, and it has never seen a standard-cell library —
//! only `synth_xilinx`. Its deployed `stream` profile asks for 3.65 Mbit of on-chip memory against a
//! Caravel user area of 10.28 mm^2, which is not close. Worse, the profile that asks for it was not
//! sized by what programs need: `HEAP=32768` is a POWER OF TWO chosen to land on exactly one
//! RAMB36E1 tile and dodge a yosys depth-cascade mis-map, and `NREG=512` is a clean power-of-two
//! register file for the same reason. Both are Xilinx facts. An ASIC has no RAMB tiles, so carrying
//! those numbers onto silicon would buy nothing and cost square millimetres — and `sfile` is
//! `DEPTH x NREG`, so the inflated NREG is multiplied by 32 before it is paid for.
//!
//! So the profile must be SOLVED, from measured program demands against a measured process, and the
//! solution must be checked rather than asserted. That is what this file locks:
//!
//!   1. the array model mirrors the RTL the emitter actually produces — parsed out of it, so the
//!      area model cannot drift away from the Verilog it claims to describe;
//!   2. the deployed `stream` profile is honestly reported as NOT fittable on a Caravel sky130 die,
//!      with a typed refusal rather than a number nobody checks;
//!   3. the solved ASIC profile covers every measured demand, fits, and leaves room for the logic.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::pdk_target::PdkTarget;
use logicaffeine_compile::codegen_sva::wpe_floorplan::{
    plan_wpe, ArrayRole, Residence, WpeDemands, WpeProfile, WpeUnplaceable,
};
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::WPE_STREAM_CAPS;

/// The MEASURED ceiling across the whole lowering benchmark suite, from
/// `wprog_all_benchmarks_lane::print_max_dims_across_all_benchmarks` (35 programs; the driver for
/// each dimension is named). DEPTH is the one that must be RUN rather than read off the config.
///
/// These are demands, not caps: the profile solver adds the headroom, and
/// `the_solved_asic_profile_covers_every_measured_demand` proves it did.
fn measured_suite_demands() -> WpeDemands {
    WpeDemands {
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
    }
}

#[test]
fn the_array_model_mirrors_the_emitted_rtl() {
    let profile = WpeProfile::from_caps(WPE_STREAM_CAPS);
    // `file0` is the block-entry register snapshot, and it is EXCISED under CFG_BRAM=1 (the streamed
    // engine replaces it with the read-before-write buffer). The shipped artifact is the CFG_BRAM
    // build, so the model must exclude it — counting a dead array would inflate the area estimate.
    let modelled = profile.arrays(true);

    let rtl = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../benchmarks/silicon/wpe_soc_stream.v"),
    )
    .expect("the emitted streamed engine");

    let mut declared: Vec<(String, u32, u32)> = Vec::new();
    for line in rtl.lines() {
        // `reg [31:0] name [0:EXPR-1];`, with an optional `signed` and an optional attribute prefix.
        let Some(open) = line.find("reg ") else {
            continue;
        };
        let rest = &line[open + 4..];
        let rest = rest.strip_prefix("signed ").unwrap_or(rest);
        let Some(hi) = rest.strip_prefix('[') else {
            continue;
        };
        let Some(colon) = hi.find(":0]") else {
            continue;
        };
        let Ok(width) = hi[..colon].parse::<u32>() else {
            continue;
        };
        let after = hi[colon + 3..].trim();
        let Some(sp) = after.find(|c: char| c.is_whitespace() || c == '[') else {
            continue;
        };
        let (name, tail) = after.split_at(sp);
        let tail = tail.trim();
        let Some(depth_expr) = tail.strip_prefix("[0:") else {
            continue;
        };
        let Some(end) = depth_expr.find(']') else {
            continue;
        };
        declared.push((name.to_string(), width + 1, {
            // Evaluate the depth expression against the profile's own localparams.
            let e = depth_expr[..end].trim();
            profile
                .eval_depth(e)
                .unwrap_or_else(|| panic!("cannot evaluate RTL depth `{e}` for `{name}`"))
        }));
    }
    assert!(
        declared.len() >= 39,
        "the RTL parse found only {} addressed arrays — the declaration syntax changed and this \
         lock is no longer reading the file it claims to check",
        declared.len()
    );

    for (name, width, depth) in &declared {
        if name == "file0" {
            continue; // excised under CFG_BRAM=1, asserted below
        }
        let m = modelled.iter().find(|a| a.name == name).unwrap_or_else(|| {
            panic!(
                "`{name}` [{depth} x {width}] is declared in wpe_soc_stream.v but the floorplan \
                 model does not know about it — the area estimate is missing a real array"
            )
        });
        assert_eq!(
            (m.words, m.bits),
            (*depth, *width),
            "`{name}`: the model says {} x {} but the RTL declares {depth} x {width}",
            m.words,
            m.bits
        );
    }
    for a in &modelled {
        assert!(
            declared.iter().any(|(n, ..)| n == a.name),
            "the floorplan model carries `{}`, which the emitted RTL does NOT declare — the model \
             is charging area for an array that does not exist",
            a.name
        );
    }
    assert!(
        modelled.iter().all(|a| a.name != "file0"),
        "`file0` is dead under CFG_BRAM=1 and must not be charged against the die"
    );
}

#[test]
fn the_deployed_stream_profile_does_not_fit_a_caravel_die() {
    let sky = PdkTarget::by_name("sky130A").expect("sky130A");
    let profile = WpeProfile::from_caps(WPE_STREAM_CAPS);
    assert!(
        profile.total_bits(true) > 3_500_000,
        "the streamed profile should be ~3.6 Mbit; got {}",
        profile.total_bits(true)
    );
    // Nothing off-chip: this is the honest statement of where the engine is today.
    match plan_wpe(sky, &profile, &[], 140_000) {
        Err(WpeUnplaceable::ExceedsUserArea {
            memory_mm2,
            available_mm2,
            ..
        }) => {
            assert!(
                memory_mm2 > available_mm2,
                "a refusal must actually be over the area it refuses for"
            );
        }
        Err(other) => panic!("expected an area refusal, got {other:?}"),
        Ok(f) => panic!(
            "the 3.65 Mbit stream profile is reported as FITTING a {:.2} mm^2 user area in \
             {:.2} mm^2 of memory — the area model is wrong, and believing it would cost a \
             shuttle slot",
            sky.user_area.unwrap().mm2(),
            f.memory_mm2
        ),
    }
}

#[test]
fn the_solved_asic_profile_covers_every_measured_demand() {
    let d = measured_suite_demands();
    let p = d.smallest_covering_profile();
    assert!(
        p.nreg >= d.registers,
        "NREG {} < measured {}",
        p.nreg,
        d.registers
    );
    assert!(
        p.maxblk >= d.blocks,
        "MAXBLK {} < measured {}",
        p.maxblk,
        d.blocks
    );
    assert!(
        p.maxcell >= d.cells,
        "MAXCELL {} < measured {}",
        p.maxcell,
        d.cells
    );
    assert!(
        p.maxv >= d.cells_per_block,
        "MAXV {} < measured {}",
        p.maxv,
        d.cells_per_block
    );
    assert!(
        p.maxwr >= d.writes,
        "MAXWR {} < measured {}",
        p.maxwr,
        d.writes
    );
    assert!(
        p.maxliv >= d.liveins,
        "MAXLIV {} < measured {}",
        p.maxliv,
        d.liveins
    );
    assert!(
        p.maxarg >= d.call_args,
        "MAXARG {} < measured {}",
        p.maxarg,
        d.call_args
    );
    assert!(
        p.maxk >= d.konsts,
        "MAXK {} < measured {}",
        p.maxk,
        d.konsts
    );
    assert!(
        p.depth >= d.depth,
        "DEPTH {} < measured {}",
        p.depth,
        d.depth
    );
    assert!(
        p.heap >= d.heap_words,
        "HEAP {} < measured {}",
        p.heap,
        d.heap_words
    );

    // The whole point of solving rather than inheriting: the FPGA profile's Xilinx-driven
    // oversizing must actually be GONE, dimension by dimension. `NREG` and `HEAP` are the two the
    // RAMB tile mapping chose (clean powers of two, so a 15-bit address lands on one tile with no
    // depth-cascade); `DEPTH x NREG` is `sfile`, so the NREG one is paid for 32 times over.
    let stream = WpeProfile::from_caps(WPE_STREAM_CAPS);
    for (dim, solved, fpga, demand) in [
        ("NREG", p.nreg, stream.nreg, d.registers),
        ("HEAP", p.heap, stream.heap, d.heap_words),
        ("MAXBLK", p.maxblk, stream.maxblk, d.blocks),
        ("MAXWR", p.maxwr, stream.maxwr, d.writes),
    ] {
        assert!(
            solved < fpga,
            "{dim}: the solved profile kept the FPGA value {fpga} even though the suite only \
             demands {demand} — that number came from a RAMB tile, not from a program, and \
             carrying it onto silicon buys nothing and costs area"
        );
    }
    assert!(
        p.total_bits(true) * 3 < stream.total_bits(true) * 2,
        "the solved profile is {} bits against the FPGA profile's {} — under a third smaller is \
         not a solve",
        p.total_bits(true),
        stream.total_bits(true)
    );
}

#[test]
fn the_solved_asic_profile_fits_a_caravel_die_with_room_for_the_engine() {
    let sky = PdkTarget::by_name("sky130A").expect("sky130A");
    let area = sky.user_area.expect("Caravel user area").mm2();
    let p = measured_suite_demands().smallest_covering_profile();

    // `wpe_soc_fpga` WITH_FPU=1 is 203,034 generic cells. That is the logic the die must also hold,
    // and a floorplan that fits the memory by leaving no room for the engine has not fit anything.
    const ENGINE_CELLS: u64 = 203_034;

    // Heap, frame stack AND config image. The first two are obvious (bulk state, latency-tolerant,
    // already read through the FSM's ISSUE/WAIT/LATCH triplets). The third is the one the area
    // model forces: `cell_*` is six 32-bit arrays indexed by one address, and at the solved
    // MAXCELL it is the single largest on-chip term — more than every other config array put
    // together. It is also the most natural thing to move: it IS the program image, it already
    // arrives over the wire, and a block's cell walk is a sequential `ci` increment, which is the
    // most prefetchable access pattern there is.
    let off = [
        ArrayRole::Heap,
        ArrayRole::FrameStack,
        ArrayRole::ConfigImage,
    ];
    let f = plan_wpe(sky, &p, &off, ENGINE_CELLS).unwrap_or_else(|e| {
        panic!(
            "the solved ASIC profile with heap+frame-stack off-chip STILL does not fit a \
             {area:.2} mm^2 user area: {e:?}"
        )
    });

    assert!(
        f.memory_mm2 + f.logic_mm2 <= area,
        "memory {:.2} + logic {:.2} = {:.2} mm^2 over a {area:.2} mm^2 user area",
        f.memory_mm2,
        f.logic_mm2,
        f.memory_mm2 + f.logic_mm2
    );
    assert!(
        f.off_chip_bits > 0,
        "the plan put nothing off-chip, yet the heap alone is {} bits — the off-chip policy was \
         not applied",
        p.heap * 64
    );
    // Every array the policy did NOT send off-chip must have a real on-chip home.
    for pl in &f.placements {
        match pl.residence {
            Residence::OffChip => assert!(
                off.contains(&pl.array.role),
                "`{}` was sent off-chip but its role is not in the policy",
                pl.array.name
            ),
            Residence::Macros { .. } => assert!(
                !pl.array.combinational_read,
                "`{}` is read COMBINATIONALLY and cannot be an SRAM macro at any speed — a macro \
                 read is registered, so this placement is unimplementable",
                pl.array.name
            ),
            Residence::Flops { .. } => {}
        }
    }
}
