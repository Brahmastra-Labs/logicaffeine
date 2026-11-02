//! flow_config_lock — the compiler's physical-design configuration is pinned to the measurements
//! that justify it, and the PDK is proven to be a parameter rather than a literal.
//!
//! Before this, closing a block's electrical gates meant a person setting environment variables in
//! front of `scripts/openlane-run.sh`, and the knowledge of which values work lived in that
//! person's head. Two consequences, both real: every new block re-learned it from scratch, and a
//! block could be signed off having silently missed a setting that was measured to be necessary.
//!
//! `codegen_sva::flow_config` moves that decision into the compiler. This locks it there.

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::flow_config::{
    openlane_config, AntennaStrategy, ClosureSettings, DesignProfile,
};
use logicaffeine_compile::codegen_sva::pdk_target::{
    HarnessClock, MemoryRequest, PdkTarget, PdkUnsupported, GF180MCUC, SKY130A,
};

fn cfg_for(pdk: &PdkTarget) -> String {
    let d = DesignProfile::new("probe", "clk", 10.0);
    openlane_config(&d, pdk, &ClosureSettings::default())
}

/// The settings that were MEASURED to be necessary must actually appear. Each value here has its
/// measurement quoted at its definition in `flow_config.rs`; this asserts the emitter still emits
/// them, so a refactor cannot quietly drop one and leave a block built without it.
/// The two resizer margins must stay INDEPENDENT.
///
/// They were one field, and that coupling is why three `systolic_pipe16_realbist` runs died:
/// the GLB (routing-stage) margin is what repairs slew — 89 -> 0 on `systolic_os_pipe8_realbist`
/// at 45 — while the PL (placement-stage) margin is the one with the memory cliff, reaching a
/// TRUE peak of 114 GB (VmHWM) on a 125 GB box at 457k cells and being OOM-killed. Driving both
/// from one number means every attempt to get the repair also arms the cliff.
#[test]
fn the_placement_and_routing_slew_margins_are_separate_levers() {
    let cfg = logicaffeine_compile::codegen_sva::flow_config::ClosureSettings {
        slew_margin_pct: 45,
        ..Default::default()
    };
    assert_eq!(
        cfg.pl_slew_margin_pct, 30,
        "raising the routing-stage margin must NOT raise the placement-stage one. The placement \
         resizer is where the OOM lives; the routing resizer is where the slew repair lives."
    );
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/logicaffeine_compile/src/codegen_sva/flow_config.rs"),
    )
    .expect("read flow_config.rs");
    assert!(
        src.contains("\"PL_RESIZER_MAX_SLEW_MARGIN\\\": {},\", c.pl_slew_margin_pct")
            || src.contains("c.pl_slew_margin_pct"),
        "PL_RESIZER_MAX_SLEW_MARGIN must be emitted from its OWN field, not from slew_margin_pct"
    );
}

#[test]
fn the_emitted_flow_config_carries_every_measured_closure_setting() {
    let c = cfg_for(&SKY130A);
    for needle in [
        // slew 6 -> 0 on gpu_simt, and fanout 3999 -> 847 with it
        "\"PL_RESIZER_MAX_SLEW_MARGIN\": 30",
        "\"GLB_RESIZER_MAX_SLEW_MARGIN\": 30",
        "\"PL_RESIZER_MAX_CAP_MARGIN\": 30",
        "\"GLB_RESIZER_MAX_CAP_MARGIN\": 30",
        // a HIGHER diode threshold gives a LOWER antenna residual (4 -> 2)
        "\"HEURISTIC_ANTENNA_THRESHOLD\": 300",
        // the SAT jog-router, which is what clears met1/met2/met3 side-area violations
        "\"DRT_ANTENNA_ECO\": 1",
        "\"ECO_JOG\": 1",
        // the lever that finally took gpu_simt to antenna 0/0 — a met1 COST penalty, after
        // forbidding met1 outright failed to route and a density sweep made it worse
        "\"GRT_LAYER_ADJUSTMENTS\"",
        // reproducible under load: the divergence is thread oversubscription, not load
        "\"ROUTING_CORES\": 4",
    ] {
        assert!(
            c.contains(needle),
            "the emitted flow config is missing `{needle}` — that value is not a preference, it \
             is a measurement (see flow_config.rs for the number it came from).\n{c}"
        );
    }
}

/// Hold is a GATE, not a report. This is the setting whose absence let a Caravel wrapper ship with
/// 97 hold-violating paths under a waiver whose stated basis ("harness pin to harness pin, no user
/// logic between") was disproven: 97 of 97 ran from a Wishbone input port to a flip-flop INSIDE the
/// user macro, every one through standard cells.
#[test]
fn hold_violations_stop_the_flow_rather_than_being_recorded() {
    let c = cfg_for(&SKY130A);
    assert!(
        c.contains("\"QUIT_ON_HOLD_VIOLATIONS\": 1"),
        "the flow must REFUSE to emit a hold-violating block. Producing one and recording it is \
         how a waiver gets written.\n{c}"
    );
}

/// The PDK is a parameter. Two processes must produce configs that differ in exactly the
/// process-dependent places — if they come out identical, something is hardcoded.
#[test]
fn the_config_is_derived_from_the_pdk_not_hardcoded() {
    let sky = cfg_for(&SKY130A);
    let gf = cfg_for(&GF180MCUC);

    assert!(
        sky.contains("sky130_fd_sc_hd"),
        "sky130 config must name its own cell library"
    );
    assert!(
        gf.contains("gf180mcu_fd_sc_mcu7t5v0"),
        "gf180 config must name ITS cell library"
    );
    assert!(
        !gf.contains("sky130"),
        "a gf180 config must not mention sky130 anywhere:\n{gf}"
    );

    // The metal stacks are genuinely different — sky130 tops out at met5, gf180 at Metal4 — and a
    // config that got this wrong would route on a layer the vehicle does not offer.
    assert!(
        sky.contains("\"RT_MAX_LAYER\": \"met5\""),
        "sky130 top layer"
    );
    assert!(
        gf.contains("\"RT_MAX_LAYER\": \"Metal4\""),
        "gf180 top layer"
    );
    assert_ne!(
        sky, gf,
        "two different processes produced an identical config"
    );
}

/// A fixed-die vehicle gets its die from the PDK's own user area, not from a literal. sky130's
/// Caravel is 2920x3520; gf180's is 3000x3000 — a different harness repo entirely.
#[test]
fn a_fixed_die_wrapper_takes_its_geometry_from_the_vehicle() {
    let mut d = DesignProfile::new("user_project_wrapper", "wb_clk_i", 25.0);
    d.fixed_die = true;
    let sky = openlane_config(&d, &SKY130A, &ClosureSettings::default());
    let gf = openlane_config(&d, &GF180MCUC, &ClosureSettings::default());
    assert!(
        sky.contains("\"DIE_AREA\": \"0 0 2920 3520\""),
        "sky130 Caravel user area:\n{sky}"
    );
    assert!(
        gf.contains("\"DIE_AREA\": \"0 0 3000 3000\""),
        "gf180 Caravel user area:\n{gf}"
    );
}

/// The memory planner must REFUSE rather than return a plan that cannot be built. This is the
/// decision that governs the whole-program engine: its streamed profile wants a 32,768-word x
/// 64-bit heap, which is 2.1 Mbit against a 10.28 mm^2 user area.
#[test]
fn a_memory_that_cannot_fit_the_vehicle_is_refused_not_planned() {
    let wpe_heap = MemoryRequest {
        words: 32768,
        bits: 64,
        dual_port: true,
        needs_multi_corner: true,
    };
    match SKY130A.plan_memory(wpe_heap) {
        Err(PdkUnsupported::ExceedsUserArea {
            needed_mm2,
            available_mm2,
        }) => {
            assert!(
                needed_mm2 > available_mm2,
                "the refusal must carry the numbers that justify it"
            );
        }
        other => panic!(
            "a 2.1 Mbit heap must be refused on a 10.28 mm^2 user area, got {other:?} — silently \
             planning it is how a design gets built that no shuttle can carry"
        ),
    }
}

/// …and a memory that DOES fit must plan onto a macro that can actually carry the claim being
/// made about it. sky130's widest SRAM ships a single Liberty corner, so a design intending a
/// multi-corner signoff cannot use it — the planner must reach for the full-corner part instead.
#[test]
fn a_multi_corner_claim_forces_a_fully_characterized_macro() {
    let heap = MemoryRequest {
        words: 2048,
        bits: 32,
        dual_port: true,
        needs_multi_corner: true,
    };
    let plan = SKY130A
        .plan_memory(heap)
        .expect("a 2048x32 heap fits the Caravel user area");
    let m = SKY130A
        .sram_macros
        .iter()
        .find(|m| m.name == plan.macro_name)
        .expect("the plan names a macro the PDK actually ships");
    assert!(
        m.is_multi_corner(),
        "planned onto {} which ships corners {:?} — a multi-corner signoff over a single-corner \
         macro is a claim the characterization does not support",
        m.name,
        m.corners
    );
    assert_eq!(
        plan.width_banks, 1,
        "a 32-bit array on a 32-bit macro needs no width banking"
    );
}

/// GF180's SRAM IP is 8 bits wide and, in this descriptor, uncharacterized — so the planner must
/// refuse a multi-corner claim there rather than invent one. This is the check that would catch
/// someone porting the engine to gf180 and assuming the memory story carries over.
#[test]
fn gf180_refuses_a_multi_corner_memory_it_cannot_characterize() {
    let heap = MemoryRequest {
        words: 2048,
        bits: 32,
        dual_port: true,
        needs_multi_corner: true,
    };
    assert!(
        matches!(
            GF180MCUC.plan_memory(heap),
            Err(PdkUnsupported::NoDualPortMacro { .. })
                | Err(PdkUnsupported::NoMultiCornerMacro { .. })
        ),
        "gf180's SRAM IP is single-port 8-bit and uncharacterized in this descriptor; planning a \
         dual-port multi-corner memory onto it must be a typed refusal, not a number"
    );
}

/// The layer-adjustment vector must have one column per routing layer AND assign them by ROLE.
///
/// Column count alone is not enough, and this test exists because the first version got the count
/// right and the meaning wrong. sky130's bottom layer is `li1`, local interconnect, which is not
/// for routing and is hard-penalised at 0.99; the antenna lever belongs on met1 above it. GF180
/// has no local interconnect, so its bottom layer IS the first real metal — and the name-matched
/// version put 0.99 on it (strangling the router) while silently dropping the antenna penalty
/// entirely. Found by emitting for both processes and reading the output, not by a compiler error.
#[test]
fn the_layer_adjustment_vector_is_assigned_by_role_not_by_name() {
    let sky = SKY130A.layer_adjustments(0.99, 0.7);
    let gf = GF180MCUC.layer_adjustments(0.99, 0.7);

    assert_eq!(
        sky.split(',').count(),
        6,
        "sky130 is li1 + met1..met5 (got {sky:?})"
    );
    assert_eq!(
        gf.split(',').count(),
        5,
        "gf180 is Metal1..Metal5, no li1 (got {gf:?})"
    );

    // sky130: local interconnect hard-penalised, the antenna lever one layer up.
    assert_eq!(sky, "0.99,0.7,0,0,0,0", "sky130 li1=0.99 then met1=0.7");
    // gf180: no local interconnect, so the antenna lever lands on the bottom layer and NOTHING
    // gets the 0.99 — applying it here would be penalising the process's only low metal.
    assert_eq!(
        gf, "0.7,0,0,0,0",
        "gf180 Metal1 takes the antenna lever, not the li1 penalty"
    );
    assert!(
        !gf.contains("0.99"),
        "gf180 has no local-interconnect layer to hard-penalise: {gf}"
    );

    assert_eq!(
        SKY130A.first_signal_layer(),
        "met1",
        "sky130 signals start above li1"
    );
    assert_eq!(
        GF180MCUC.first_signal_layer(),
        "Metal1",
        "gf180 signals start at the bottom"
    );
}

/// The met1 route penalty is a lever, not a constant: the recipe's 0.7 is what closed `gpu_simt`'s last side-area
/// antenna violator, and it is also most of met1's capacity — a block whose ceiling is met3 can need it back. The
/// penalty reaches the vector through the metal stack's roles at any value, and the emitter takes it as a flag
/// that refuses what is not a penalty rather than falling back to the default.
#[test]
fn the_met1_route_penalty_is_a_lever_through_the_metal_stack() {
    let d = DesignProfile::new("probe", "clk", 10.0);
    let adj = |c: &ClosureSettings, pdk: &PdkTarget| -> String {
        let v: serde_json::Value = serde_json::from_str(&openlane_config(&d, pdk, c)).unwrap();
        v["GRT_LAYER_ADJUSTMENTS"].as_str().unwrap().to_string()
    };
    assert_eq!(
        ClosureSettings::default().met1_route_penalty,
        0.7,
        "the recipe's penalty stays the default"
    );
    assert_eq!(
        adj(&ClosureSettings::default(), &SKY130A),
        "0.99,0.7,0,0,0,0"
    );
    let eased = ClosureSettings {
        met1_route_penalty: 0.5,
        ..ClosureSettings::default()
    };
    assert_eq!(
        adj(&eased, &SKY130A),
        "0.99,0.5,0,0,0,0",
        "li1 keeps its hard penalty; met1 takes the lever"
    );
    assert_eq!(
        adj(&eased, &GF180MCUC),
        "0.5,0,0,0,0",
        "gf180's bottom layer takes the lever"
    );
    let em = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/logicaffeine_compile/examples/emit_flow_config.rs"),
    )
    .unwrap();
    assert!(
        em.contains("\"--met1-penalty\" =>"),
        "the emitter exposes the penalty"
    );
    assert!(
        !em.contains(".unwrap_or(c.target_density)"),
        "a bad --density is refused, not replaced by the default"
    );
    assert!(
        !em.contains(".unwrap_or(c.post_diode_upsize_margin_pct)"),
        "a bad --upsize-margin is refused too"
    );
}

/// The wire from a clock input to the tree's root is buffered to a length the slow corner meets. OpenROAD's own
/// limit came out at 3,331 um, and on `wpe_user_project`'s 2.8 x 3.4 mm die the 1.7 mm run from `wb_clk_i` to the
/// root buffer reached it unbuffered at 1.125 ns against the 0.75 ns limit. Every block's config carries the
/// recipe's limit, the CTS step hands it to `repair_clock_nets`, and the emitter exposes it.
#[test]
fn the_clock_input_wire_is_buffered_to_a_length_the_slow_corner_meets() {
    assert_eq!(
        ClosureSettings::default().clock_max_wire_um,
        800,
        "the measured limit"
    );
    let d = DesignProfile::new("probe", "clk", 10.0);
    assert_eq!(
        cfg_for(&SKY130A)
            .matches("\"CTS_CLK_MAX_WIRE_LENGTH\": 800,")
            .count(),
        1
    );
    let v = ClosureSettings {
        clock_max_wire_um: 1000,
        ..ClosureSettings::default()
    };
    assert!(openlane_config(&d, &SKY130A, &v).contains("\"CTS_CLK_MAX_WIRE_LENGTH\": 1000,"));
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cts =
        std::fs::read_to_string(root.join("scripts/openlane-overlay/scripts/openroad/cts.tcl"))
            .unwrap();
    assert!(
        cts.contains("repair_clock_nets -max_wire_length $::env(CTS_CLK_MAX_WIRE_LENGTH)"),
        "the CTS step applies it"
    );
    let em = std::fs::read_to_string(
        root.join("crates/logicaffeine_compile/examples/emit_flow_config.rs"),
    )
    .unwrap();
    assert!(
        em.contains("\"--clock-max-wire\" =>"),
        "the emitter exposes it"
    );
}

/// A long signal wire is buffered to a length the slow corner meets, as the clock's is. OpenROAD's own limit,
/// 3,331 um, left `wpe_user_project`'s `buf_12`-driven nets at 1,447-1,814 um whole, and they signed off at
/// 0.76-0.87 ns against the 0.75 ns limit at ss_100C_1v60, max RC — beyond what the post-diode upsize's global-route
/// estimate detects (RUN_2026.09.27_06.33.43). Both resizers take the recipe's limit.
#[test]
fn a_long_signal_wire_is_buffered_to_a_length_the_slow_corner_meets() {
    assert_eq!(
        ClosureSettings::default().signal_max_wire_um,
        800,
        "the measured limit"
    );
    let c = cfg_for(&SKY130A);
    assert_eq!(
        c.matches("\"PL_RESIZER_MAX_WIRE_LENGTH\": 800,").count(),
        1,
        "the placement resizer:\n{c}"
    );
    assert_eq!(
        c.matches("\"GLB_RESIZER_MAX_WIRE_LENGTH\": 800,").count(),
        1,
        "the routing resizer:\n{c}"
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rd = std::fs::read_to_string(
        root.join("scripts/openlane-overlay/scripts/openroad/resizer_routing_design.tcl"),
    )
    .unwrap();
    assert!(
        rd.contains("-max_wire_length $::env(GLB_RESIZER_MAX_WIRE_LENGTH)"),
        "the routing resizer applies it"
    );
    let em = std::fs::read_to_string(
        root.join("crates/logicaffeine_compile/examples/emit_flow_config.rs"),
    )
    .unwrap();
    assert!(em.contains("\"--max-wire\" =>"), "the emitter exposes it");
}

/// A block may keep OpenROAD's own wire limits — what every block hardened before the 800 um limits were measured was
/// built with — by naming them: `--max-wire openroad` / `--clock-max-wire openroad` emit 0, which OpenLane reads as
/// unset (the resizers pass `-max_wire_length` only for a nonzero value; OpenLane's own default is 0). A bare 0 stays
/// refused, so no typo silently drops a limit. MEASURED: `fabric_horner_seq` at 800 um died in global-route setup
/// repair (RUN_2026.09.28_20.36.35): `rsz::makeBufferedNet` recursed 10,460 frames deep under `RepairSetup::rebuffer`
/// on an 8 MB stack and, given an unlimited one, grew until the kernel killed it — a cycle in one net's route tree,
/// OpenROAD 41a51eaf. Its recorded recipe, at OpenROAD's limits, passes that step.
#[test]
fn a_block_may_keep_openroads_own_wire_limits_by_naming_them() {
    use logicaffeine_compile::codegen_sva::flow_config::parse_wire_limit_um;
    assert_eq!(
        parse_wire_limit_um("openroad"),
        Some(0),
        "the named tool limit"
    );
    assert_eq!(parse_wire_limit_um("800"), Some(800));
    assert_eq!(
        parse_wire_limit_um("0"),
        None,
        "a bare 0 is refused — name the tool's limit instead"
    );
    assert_eq!(parse_wire_limit_um("-5"), None);
    assert_eq!(parse_wire_limit_um("default"), None);
    let d = DesignProfile::new("probe", "clk", 10.0);
    let tool = ClosureSettings {
        signal_max_wire_um: 0,
        clock_max_wire_um: 0,
        ..ClosureSettings::default()
    };
    let c = openlane_config(&d, &SKY130A, &tool);
    for k in [
        "CTS_CLK_MAX_WIRE_LENGTH",
        "PL_RESIZER_MAX_WIRE_LENGTH",
        "GLB_RESIZER_MAX_WIRE_LENGTH",
    ] {
        assert_eq!(
            c.matches(&format!("\"{k}\": 0,")).count(),
            1,
            "{k} must be emitted as 0:\n{c}"
        );
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rd = std::fs::read_to_string(
        root.join("scripts/openlane-overlay/scripts/openroad/resizer_routing_design.tcl"),
    )
    .unwrap();
    assert!(
        rd.contains("&& $::env(GLB_RESIZER_MAX_WIRE_LENGTH) } {"),
        "the routing resizer skips the limit at 0"
    );
    let em = std::fs::read_to_string(
        root.join("crates/logicaffeine_compile/examples/emit_flow_config.rs"),
    )
    .unwrap();
    assert_eq!(
        em.matches("parse_wire_limit_um(need(i))").count(),
        2,
        "both flags parse through the library"
    );
}

/// The upsize sizes once more after global routing's antenna repair, with that repair's diodes present. The repair
/// hangs them on nets the post-diode upsize has already sized — up to 20 on a single-sink net of `wpe_user_project`
/// (ANT-0009: more than 10 diodes per gate), and 13 of its 21 slow-corner slew violators were such nets
/// (RUN_2026.09.27_06.33.43). Replayed on that run's post-repair route, the upsize found and repaired all 13. Global
/// routing then runs again without antenna repair, so detailed routing follows guides for the netlist the upsize
/// produced, and the repair's own switch is restored.
#[test]
fn the_upsize_sizes_again_for_the_diodes_global_routing_adds() {
    let c = ClosureSettings::default();
    assert!(!c.post_antenna_upsize, "off unless chosen: it re-routes");
    let on = ClosureSettings {
        post_antenna_upsize: true,
        ..ClosureSettings::default()
    };
    let cfg = openlane_config(&DesignProfile::new("probe", "clk", 10.0), &SKY130A, &on);
    assert!(
        cfg.contains("\"LOGOS_POST_ANTENNA_UPSIZE\": 1,"),
        "the lever reaches config.json"
    );
    assert!(
        cfg_for(&SKY130A).contains("\"LOGOS_POST_ANTENNA_UPSIZE\": 0,"),
        "the default is emitted explicitly"
    );

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rt = std::fs::read_to_string(
        root.join("scripts/openlane-overlay/scripts/tcl_commands/routing.tcl"),
    )
    .unwrap();
    let body = &rt[rt.find("proc run_routing {").expect("run_routing proc")..];
    let body = &body[..body.find("\n}\n").expect("run_routing end")];
    let first_gr = body
        .find("\n    global_routing\n")
        .expect("the global route");
    let gate = body
        .find("$::env(LOGOS_POST_ANTENNA_UPSIZE) == 1")
        .expect("the post-antenna gate");
    let drt = body
        .find("\n        detailed_routing\n")
        .expect("detailed routing");
    assert!(
        first_gr < gate && gate < drt,
        "after global routing's antenna repair, before detailed routing"
    );
    let block = &body[gate..drt];
    let up = block
        .find("run_logos_post_diode_upsize")
        .expect("the upsize runs");
    let off = block
        .find("set ::env(GRT_REPAIR_ANTENNAS) 0")
        .expect("antenna repair off for the re-route");
    let regr = block[off..]
        .find("global_routing")
        .map(|i| off + i)
        .expect("the guides are re-routed");
    let restore = block
        .find("set ::env(GRT_REPAIR_ANTENNAS) $")
        .expect("the switch is restored");
    assert!(
        up < off && off < regr && regr < restore,
        "upsize, repair off, re-route, restore:\n{block}"
    );

    let sh = std::fs::read_to_string(root.join("scripts/caravel-macro-harden.sh")).unwrap();
    assert!(
        sh.contains("--post-antenna-upsize"),
        "the Caravel macro harden sizes for the repair's diodes"
    );
    let em = std::fs::read_to_string(
        root.join("crates/logicaffeine_compile/examples/emit_flow_config.rs"),
    )
    .unwrap();
    assert!(
        em.contains("\"--post-antenna-upsize\" =>"),
        "the emitter exposes it"
    );
}

/// The quiet object lookups in `logos_post_diode_upsize.tcl` that the script tests for absence, and do not pass
/// through `logos_obj` — OpenSTA answers a lookup that finds nothing with the string `NULL`, not an empty one.
fn unguarded_quiet_lookups(tcl: &str) -> Vec<String> {
    let mut bad = Vec::new();
    for (n, line) in tcl.lines().enumerate() {
        let code = line.split('#').next().unwrap();
        let mut rest = code;
        while let Some(at) = rest.find("[get_") {
            let tail = &rest[at..];
            let call = &tail[..tail.find(']').map_or(tail.len(), |e| e + 1)];
            let is_object = ["[get_nets ", "[get_cells ", "[get_pins ", "[get_ports "]
                .iter()
                .any(|k| call.starts_with(k));
            if is_object && call.contains(" -quiet") && !rest[..at].ends_with("[logos_obj ") {
                bad.push(format!("{}: {}", n + 1, line.trim()));
            }
            rest = &rest[at + 1..];
        }
    }
    bad
}

/// A slew violator the STA reports can be a top-level port, which is a pin with no net of its own: the engine's
/// `wb_clk_i` was one, and the upsize died on `get_full_name NULL` before it had changed anything
/// (RUN_2026.09.26_17.51.01). Every quiet lookup passes through `logos_obj`, which reads `NULL` as absent, and a
/// port resolves to its term's net.
#[test]
fn the_post_diode_upsize_reads_a_port_as_its_net_and_null_as_absent() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let up = std::fs::read_to_string(
        root.join("scripts/openlane-overlay/scripts/openroad/logos_post_diode_upsize.tcl"),
    )
    .unwrap();
    assert!(
        up.contains("proc logos_obj {o}"),
        "the NULL-reading helper exists"
    );
    let bad = unguarded_quiet_lookups(&up);
    assert!(
        bad.is_empty(),
        "quiet lookups that read a missing object as present:\n{}",
        bad.join("\n")
    );
    let nets = &up[up
        .find("proc logos_slew_violating_nets")
        .expect("the violator collector")..];
    let nets = &nets[..nets.find("\n}\n").unwrap()];
    assert!(
        nets.contains("[logos_obj [$p term]]") && nets.contains("[logos_obj [$term net]]"),
        "a port resolves to its term's net:\n{nets}"
    );

    assert_eq!(
        unguarded_quiet_lookups("set n [get_nets -quiet -of_objects $p]\n").len(),
        1,
        "a bare lookup is caught"
    );
    assert!(
        unguarded_quiet_lookups("set n [logos_obj [get_nets -quiet -of_objects $p]]\n").is_empty()
    );
    assert!(
        unguarded_quiet_lookups("set n [get_nets $name]\n").is_empty(),
        "a loud lookup errors on its own"
    );
    assert_eq!(
        unguarded_quiet_lookups("if {[get_cells -quiet -of_objects $d] eq \"\"} {}\n").len(),
        1
    );
}

/// The post-diode upsize chooses only cells the flow may place. `DONT_USE_CELLS` carries the process's DRC-excluded
/// cells; OpenROAD's `replace_cell` does not consult it, so a chooser that skips it grows a `buf_12` into
/// `buf_16` — which it did 226 times on `wpe_user_project` (RUN_2026.09.26_17.51.01, replayed). Every cell the
/// script picks from the library passes `logos_usable`.
#[test]
fn the_post_diode_upsize_never_chooses_a_cell_the_flow_forbids() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let up = std::fs::read_to_string(
        root.join("scripts/openlane-overlay/scripts/openroad/logos_post_diode_upsize.tcl"),
    )
    .unwrap();
    let usable = &up[up
        .find("proc logos_usable {name}")
        .expect("the dont-use predicate")..];
    let usable = &usable[..usable.find("\n}\n").unwrap()];
    assert!(
        usable.contains("DONT_USE_CELLS"),
        "it reads the flow's dont-use list:\n{usable}"
    );
    let mut choosers = 0;
    for (at, _) in up.match_indices("[get_lib_cells ") {
        let proc_start = up[..at]
            .rfind("\nproc ")
            .expect("a lib-cell scan inside a proc");
        let body = &up[proc_start..];
        let body = &body[..body.find("\n}\n").unwrap()];
        choosers += 1;
        assert!(
            body.contains("logos_usable $name"),
            "a chooser that ignores DONT_USE_CELLS:\n{body}"
        );
    }
    assert!(
        choosers >= 2,
        "the upsize and the isolation buffer both choose from the library ({choosers} found)"
    );
}

/// Every lever the shell scripts used to expose must still be reachable through the compiler.
///
/// This test exists because moving the config generation into the compiler SILENTLY DROPPED one.
/// The emitter hardcoded `RUN_HEURISTIC_DIODE_INSERTION: 1` and `GRT_REPAIR_ANTENNAS: 1`, so
/// `HEUR_DIODE=0 GRT_REPAIR=0` — which the old shell honoured — became a no-op. An experiment run
/// specifically to measure the design WITHOUT antenna diodes in fact ran with 106 of them, and a
/// conclusion was drawn from it before anyone noticed. Nothing failed; the config was simply not
/// what the caller asked for.
///
/// A partial shift-left is worse than none: the new owner must expose every lever the old one had.
#[test]
fn the_antenna_strategy_is_a_real_lever_in_both_directions() {
    let d = DesignProfile::new("probe", "clk", 10.0);

    let jog = openlane_config(
        &d,
        &SKY130A,
        &ClosureSettings {
            antenna: AntennaStrategy::JogOnly,
            ..Default::default()
        },
    );
    assert!(
        jog.contains("\"RUN_HEURISTIC_DIODE_INSERTION\": 0")
            && jog.contains("\"GRT_REPAIR_ANTENNAS\": 0"),
        "JogOnly must actually disable BOTH diode passes — a diode is extra capacitance on its net \
         and the diode passes run AFTER the resizer, so slew repair never sees the load they add. \
         On coproc_core half the max-slew violators were ANTENNA_*/DIODE pins.\n{jog}"
    );
    assert!(
        jog.contains("\"ECO_JOG\": 1"),
        "JogOnly still needs the jog router — it is the thing doing the antenna work"
    );

    let diodes = openlane_config(
        &d,
        &SKY130A,
        &ClosureSettings {
            antenna: AntennaStrategy::DiodesAndJog,
            ..Default::default()
        },
    );
    assert!(
        diodes.contains("\"RUN_HEURISTIC_DIODE_INSERTION\": 1")
            && diodes.contains("\"GRT_REPAIR_ANTENNAS\": 1"),
        "DiodesAndJog must actually enable them:\n{diodes}"
    );
    assert_ne!(
        jog, diodes,
        "the two strategies produced an identical config — the lever is dead"
    );

    // The THIRD cell of the table: pre-route heuristic diodes ON, post-route GRT repair OFF.
    // OpenLane runs two INDEPENDENT diode passes, and while this enum had only "both" and
    // "neither" the best combination for coproc_core was unreachable — two sessions of
    // experiments moved them together and concluded the block needed an emitter rewrite. It did
    // not: on/off gave slew 282 -> 120, antenna 27, DRC/LVS/XOR/routing 0, AND it routes, where
    // on/on congested and off/off gave 538-770 violating nets.
    let heur = openlane_config(
        &d,
        &SKY130A,
        &ClosureSettings {
            antenna: AntennaStrategy::HeuristicDiodesAndJog,
            ..Default::default()
        },
    );
    assert!(
        heur.contains("\"RUN_HEURISTIC_DIODE_INSERTION\": 1")
            && heur.contains("\"GRT_REPAIR_ANTENNAS\": 0"),
        "HeuristicDiodesAndJog must enable the PRE-route heuristic pass and disable the POST-route          GRT repair. Emitting both from one shared flag is exactly what made this combination          impossible to express.\n{heur}"
    );
    assert!(
        heur.contains("\"ECO_JOG\": 1"),
        "HeuristicDiodesAndJog still runs the jog router for the residue"
    );
    for (name, other) in [("JogOnly", &jog), ("DiodesAndJog", &diodes)] {
        assert_ne!(
            &heur, other,
            "HeuristicDiodesAndJog emits the same config as {name} — the third variant is not a              real lever, which is the defect it was added to fix"
        );
    }
}

/// The CLOCK input port is declared as driven by the process's own clock-root buffer, never left
/// to OpenLane's default.
///
/// `SYNTH_CLK_DRIVING_CELL` DEFAULTS TO `SYNTH_DRIVING_CELL`, which is the minimum DATA-input
/// inverter — `sky130_fd_sc_hd__inv_2`. Nothing on a real die drives a clock with a minimum logic
/// inverter: it arrives from a pad, a PLL, or (on Caravel) the management SoC's clock tree. The
/// resulting pessimism lands on the one net no resizer can repair, because an input port's
/// transition is an ASSUMPTION rather than something buffering can improve.
///
/// It is not a theoretical concern. `systolic_os_pipe8_realbist` closed DRC, LVS, XOR, routing,
/// antenna, setup and hold, and signed off carrying exactly three max-slew violations — and all
/// three were that single net: the port `clk`, the first buffer's pin `clkbuf_0_clk/A`, and a
/// diode sitting on the same net, each measured at 0.97 against a 0.75 limit with identical
/// −0.22 slack. `systolic_pipe16_realbist` shows the same triple at 1.25.
#[test]
fn the_clock_port_is_driven_by_the_processs_own_clock_buffer() {
    let c = cfg_for(&SKY130A);
    let (cell, pin) = SKY130A
        .clk_driving_cell
        .expect("sky130A declares a clock-root buffer");
    assert!(
        c.contains(&format!("\"SYNTH_CLK_DRIVING_CELL\": \"{cell}\"")),
        "the clock port's driver is not declared, so OpenLane falls back to SYNTH_DRIVING_CELL — \
         the minimum data inverter — and charges the clock input a transition no design can \
         repair.\n{c}"
    );
    assert!(
        c.contains(&format!("\"SYNTH_CLK_DRIVING_CELL_PIN\": \"{pin}\"")),
        "the driving cell is declared without its output pin; OpenLane needs both.\n{c}"
    );
    assert!(
        cell.contains("clkbuf"),
        "`{cell}` is not a clock buffer — this field must carry the PDK's own ROOT_CLK_BUFFER, \
         which is the process's statement of what drives a clock root, not a logic cell"
    );
    assert!(
        !c.contains("\"SYNTH_CLK_DRIVING_CELL\": \"sky130_fd_sc_hd__inv_2\""),
        "the clock port is declared as driven by the DATA-input inverter, which is exactly the \
         default this setting exists to displace"
    );
}

/// A process whose clock-root buffer has not been read from a real PDK tree declares NOTHING,
/// rather than a plausible guess.
///
/// gf180mcuC is not installed here (`ciel ls --pdk-family gf180mcu` is empty), so its
/// `ROOT_CLK_BUFFER` has never been read. A guessed cell name is the worst outcome available: a
/// wrong one fails the flow immediately, but a plausible wrong one builds a block against a cell
/// that is not the process's clock driver and nobody finds out. Emitting nothing leaves OpenLane
/// on its own default, which is at least a documented behaviour.
#[test]
fn an_unread_process_declares_no_clock_driver_rather_than_guessing() {
    assert!(
        GF180MCUC.clk_driving_cell.is_none(),
        "gf180mcuC now claims a clock-root buffer. That is only legitimate if the PDK has been \
         installed and the value READ from its libs.tech/openlane/<scl>/config.tcl — if it was \
         typed from memory, delete it"
    );
    let c = cfg_for(&GF180MCUC);
    assert!(
        !c.contains("SYNTH_CLK_DRIVING_CELL"),
        "a process with no declared clock-root buffer must emit no clock-driver setting at all, \
         not an empty or guessed one.\n{c}"
    );
}

/// Diodes-before-resize must be a real lever end to end: the compiler emits it, and when it is set
/// `routing.tcl` runs the heuristic diode pass BEFORE the slew/cap resizer, not after.
///
/// The order is the entire content of this fix. The resizer sizes drivers for the load it can
/// see; with diodes added afterwards it never sees theirs, and 25 of `coproc_core`'s 69 slew
/// violators were diode pins while every violating net on `systolic_pipe16_realbist` carried one.
/// A flag that is emitted but checked after the resizer has already run would read as configured
/// and change nothing — the failure shape of the three `lappend -flag` bugs.
#[test]
fn diodes_before_resize_is_emitted_and_actually_precedes_the_resizer() {
    assert!(
        !logicaffeine_compile::codegen_sva::flow_config::ClosureSettings::default()
            .diodes_before_resize,
        "off by default until measured across the suite: it changes every block's netlist"
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fc = std::fs::read_to_string(
        root.join("crates/logicaffeine_compile/src/codegen_sva/flow_config.rs"),
    )
    .unwrap();
    assert!(
        fc.contains("LOGOS_DIODES_BEFORE_RESIZE") && fc.contains("c.diodes_before_resize"),
        "flow_config must emit LOGOS_DIODES_BEFORE_RESIZE from its own field"
    );
    let rt = std::fs::read_to_string(
        root.join("scripts/openlane-overlay/scripts/tcl_commands/routing.tcl"),
    )
    .unwrap();
    let body = &rt[rt.find("proc run_routing {").expect("run_routing proc")..];
    let gate = body
        .find("if { $_logos_dbr && $_logos_have_diode }")
        .expect("the DBR gate is gone");
    let resize = body
        .find("\n    run_resizer_design_routing\n")
        .expect("resizer call");
    let early_diode = body[gate..]
        .find("heuristic_diode_insertion")
        .map(|i| gate + i)
        .expect("early diode call");
    assert!(
        gate < resize && early_diode < resize,
        "with LOGOS_DIODES_BEFORE_RESIZE set, heuristic_diode_insertion must run BEFORE \
         run_resizer_design_routing — otherwise the resizer still sizes for a load it cannot see"
    );
    assert!(
        body.contains("if { !$_logos_dbr && $_logos_have_diode }"),
        "the base-order path must be skipped when DBR is on, or every net gets diodes twice"
    );
}

/// The diode fallback must be REACHABLE with the jog enabled.
///
/// The manual-stub diode ECO existed for months and could never run: the jog branch always
/// `continue`d or `return`ed before it. That is the failure this lock exists for — code that reads
/// as a repair path and executes as dead code. So assert the fallback sits INSIDE the jog branch,
/// at its zero-bridged fixpoint, ahead of the `return` that used to end the ECO there.
#[test]
fn the_diode_fallback_is_reachable_at_the_jog_fixpoint() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fc = std::fs::read_to_string(
        root.join("crates/logicaffeine_compile/src/codegen_sva/flow_config.rs"),
    )
    .unwrap();
    assert!(
        fc.contains("LOGOS_ECO_DIODE_FALLBACK") && fc.contains("c.eco_diode_fallback"),
        "flow_config must emit LOGOS_ECO_DIODE_FALLBACK from its own field"
    );
    let rt = std::fs::read_to_string(
        root.join("scripts/openlane-overlay/scripts/tcl_commands/routing.tcl"),
    )
    .unwrap();
    let eco = &rt[rt.find("proc run_antenna_eco").expect("run_antenna_eco")..];
    let jog = eco.find("$::env(ECO_JOG) == 1").expect("jog branch");
    assert!(
        eco.contains("[llength $_nets] < $_prev_nets"),
        "the jog fixpoint must be measured in VIOLATORS: a pass that bridges gates but removes no \
         violator is not progress (pipe16 re-bridged 2 of 972 gates for three passes at 657)"
    );
    let fixpoint = eco[jog..]
        .find("if { $_jog_progress }")
        .map(|i| jog + i)
        .expect("fixpoint test");
    let fallback = eco[fixpoint..]
        .find("LOGOS_ECO_DIODE_FALLBACK")
        .map(|i| fixpoint + i)
        .expect("the diode fallback is not inside the jog branch — it would be unreachable");
    let stop = eco[fixpoint..]
        .find("\n                return\n")
        .map(|i| fixpoint + i)
        .expect("fixpoint return");
    assert!(
        fallback < stop,
        "the fallback must run BEFORE the fixpoint `return`, or the jog still ends the ECO first"
    );
    assert!(
        eco[fallback..stop].contains("--manual-stub")
            && eco[fallback..stop].contains("--violator-pins"),
        "the fallback must target the exact violating PINS with the no-router manual stub"
    );
    assert!(
        eco.contains("set _fallback_last [llength $_nets]")
            && eco.contains("[llength $_nets] < $_fallback_last"),
        "the fallback may only repeat while each pass REDUCES the violator count — otherwise an \
         uncreditable pin loops to the iteration cap"
    );
}

/// The post-diode driver upsize must be a real lever end to end, placed where it can act, and
/// built so that it cannot report a false "nothing to do".
///
/// Every surviving max-slew violator on `systolic_pipe16_realbist` (15 pins, 0.75-0.85 ns against
/// 0.75) sits on a net whose driver is ALREADY the strongest of its function (`a21oi_4`,
/// `a31oi_4`), and the global-route estimate sees those nets at only 0.37-0.43 ns. Its first
/// version failed in three silent ways, each asserted against here:
///   * it read violators through `sta::with_output_to_variable`, which in this OpenROAD captures
///     NOTHING — so it reported "0 nets violating" at every margin and looked like a clean no-op;
///   * it could only upsize, so the drivers that actually cause the violations were skipped;
///   * it re-estimated on stale routes, where a buffer inserted a round earlier has no route yet.
#[test]
fn the_post_diode_upsize_is_emitted_placed_and_cannot_report_a_false_zero() {
    let c = ClosureSettings::default();
    assert!(
        !c.post_diode_upsize,
        "off by default until measured across the suite: it changes every block's netlist"
    );
    assert_eq!(c.post_diode_upsize_margin_pct, 52,
        "52% is the MEASURED detection margin: at 0.36 ns every pipe16 signoff violator is selected \
         (103 pins, 22 nets); 0.40 misses two, 0.34 selects 442 pins");
    let on = ClosureSettings {
        post_diode_upsize: true,
        ..ClosureSettings::default()
    };
    let cfg = openlane_config(&DesignProfile::new("probe", "clk", 10.0), &SKY130A, &on);
    assert!(
        cfg.contains("\"LOGOS_POST_DIODE_UPSIZE\": 1,"),
        "the lever must reach config.json"
    );
    assert!(
        cfg.contains("\"LOGOS_UPSIZE_MARGIN\": 52,"),
        "the detection margin must reach config.json"
    );
    assert!(
        cfg_for(&SKY130A).contains("\"LOGOS_POST_DIODE_UPSIZE\": 0,"),
        "the default must be emitted explicitly, never left to an unset variable"
    );

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rt = std::fs::read_to_string(
        root.join("scripts/openlane-overlay/scripts/tcl_commands/routing.tcl"),
    )
    .unwrap();
    let body = &rt[rt.find("proc run_routing {").expect("run_routing proc")..];
    let body = &body[..body.find("\n}\n").expect("run_routing end")];
    let late_diodes = body
        .find("if { !$_logos_dbr && $_logos_have_diode }")
        .expect("base-order diode pass");
    let gate = body
        .find("$::env(LOGOS_POST_DIODE_UPSIZE) == 1")
        .expect("the upsize gate is gone");
    let call = body[gate..]
        .find("run_logos_post_diode_upsize")
        .map(|i| gate + i)
        .expect("upsize call");
    let grt = body
        .find("\n    global_routing\n")
        .expect("global_routing call");
    assert!(
        late_diodes < gate && call < grt,
        "the upsize must run AFTER the diodes exist (it sizes for their load) and BEFORE global \
         routing, so the routers route the netlist it produced"
    );
    assert!(
        rt.contains("proc run_logos_post_diode_upsize")
            && rt.contains("openroad/logos_post_diode_upsize.tcl"),
        "the gate must invoke the real OpenROAD step"
    );

    let up = std::fs::read_to_string(
        root.join("scripts/openlane-overlay/scripts/openroad/logos_post_diode_upsize.tcl"),
    )
    .unwrap();
    assert!(
        !up.contains("with_output_to_variable"),
        "violators must come from the STA API, not a captured report: capture returns empty in \
         this OpenROAD and turns every run into a false 'nothing to repair'"
    );
    assert!(
        up.contains("sta::check_slew_limits"),
        "violators must come from sta::check_slew_limits"
    );
    assert!(
        up.contains("rsz::repair_net_cmd"),
        "a driver at maximum drive must be rebuffered, or the nets that cause the signoff \
         violations are exactly the ones skipped"
    );
    let grid = up
        .find("\ndetailed_placement\n")
        .expect("grid-building legalization");
    let repair = up.find("rsz::repair_net_cmd").unwrap();
    assert!(
        grid < repair,
        "repair_net places through the detailed placer's grid, which exists only after a \
         legalization (DPL-0043 otherwise)"
    );
    let measure = up.find("proc logos_measure").expect("logos_measure");
    assert!(
        up[measure..]
            .split("\n}")
            .next()
            .unwrap()
            .contains("global_route"),
        "each round must re-route before it measures: a buffer inserted last round has no route"
    );
    let lp = up.find("for {set round 1}").expect("round loop");
    assert!(
        up[lp..].contains("logos_measure") && up[lp..].contains("if { $changed == 0 } { break }"),
        "rounds continue while they change something and stop only when nothing changes"
    );
    assert!(
        up.contains("residual net"),
        "whatever is left over must be named in the log, never hidden"
    );

    // A driver at maximum drive that the single-net repair cannot help is ISOLATED: a strong buffer
    // at the driver takes over the wire and every sink, and the driver sees one short hop.
    // MEASURED on systolic_pipe16_realbist's post-upsize state (RUN_2026.09.24_06.10.32, all corners,
    // 0.60 ns detection): rsz::repair_net_cmd inserted NOTHING on 12 of 14 violating nets — every
    // a21oi_4 / nor2_8 / xnor2_4 / dfxtp_4 driver — and downsized one (o21a_4 -> o21a_1). Explicit
    // isolation cleared all 12; the loop's re-measure then found the next few, converging 14 -> 10 -> 5.
    let iso = up
        .find("proc logos_isolate_driver")
        .expect("the isolation step is gone");
    let iso_body = up[iso..].split("\n}\n").next().unwrap();
    assert!(
        iso_body.contains("make_instance")
            && iso_body.contains("connect_pin")
            && iso_body.contains("disconnect_pin"),
        "isolation re-wires the net: driver -> new net -> buffer -> the original net and its sinks"
    );
    assert!(
        iso_body.contains("getSigType] eq \"CLOCK\"") && iso_body.contains("*clkbuf*"),
        "a clock net is never isolated — a buffer there moves clock arrival, which CTS owns"
    );
    assert!(iso_body.contains("logos_iso_"),
        "a net already driven by an isolation buffer is not isolated again (a chain of buffers never \
         shortens the wire)");
    let rp = up[lp..]
        .find("rsz::repair_net_cmd")
        .map(|i| lp + i)
        .expect("repair inside the loop");
    assert!(
        up[rp..].contains("logos_isolate_driver"),
        "isolation is the fallback AFTER the single-net repair inserted nothing"
    );
}

/// Every round of the post-diode upsize measures the placement the flow will route: a round that changed something
/// legalizes before the next one measures, so the cells it placed on top of their drivers (isolation buffers,
/// rebuffered nets) are where detailed routing will find them.
///
/// MEASURED on `systolic_pipe16_realbist` (RUN_2026.09.29_00.40.21, replayed from its `19-diodes_legalized.odb`): the
/// last round measured net `_005642_` (a `nor2_4` into a flop D pin, 243 um) at 0.488 ns, under the 0.540 ns
/// detection limit, and left it alone; the closing legalization then moved the 379 cells the rounds had placed, and
/// the same estimate on the placement actually routed read 0.618 ns. It signed off at 0.76 ns at ss_100C_1v60, max RC.
/// `wpe_user_project`'s `_117618_` (RUN_2026.09.27_18.14.14) was the same: under 0.600 inside the step, 0.635 on the
/// placement it wrote.
#[test]
fn the_upsize_measures_the_placement_that_will_be_routed() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let up = std::fs::read_to_string(
        root.join("scripts/openlane-overlay/scripts/openroad/logos_post_diode_upsize.tcl"),
    )
    .unwrap();
    let lp = up.find("for {set round 1}").expect("round loop");
    let body = &up[lp..lp + up[lp..].find("\n}\n").expect("the loop closes")];
    let brk = body
        .find("if { $changed == 0 } { break }")
        .expect("the loop stops when a round changes nothing");
    let legal = body[brk..]
        .find("detailed_placement")
        .map(|i| brk + i)
        .expect("a round that changed something legalizes before the next round measures");
    assert!(
        body.find("logos_measure").unwrap() < brk && brk < legal,
        "measure, repair, stop if nothing changed, legalize: the next measurement sees legal cells"
    );
    let after = &up[lp + body.len()..];
    let resid = after
        .find("set residual_nets [logos_slew_violating_nets]")
        .expect("the residual is measured after the loop");
    assert!(
        after[..resid].contains("logos_measure"),
        "the residual is measured on a fresh route of the legal placement"
    );
}

/// The pre-route parasitic estimate is CALIBRATED against our own extraction, per layer.
///
/// OpenLane ships no `LAYERS_RC` for sky130, so every resizer and pre-route STA step estimated
/// wire capacitance from uncalibrated defaults. MEASURED on `systolic_pipe16_realbist`
/// (RUN_2026.09.23_03.24.05, 109k nets, estimate vs OpenRCX SPEF): on nets whose true load is
/// 30-80 fF — the ones that fail slew — the estimate was optimistic by a median 1.39x and 2.02x at
/// p90, so the slew/timing repairs sized drivers for about half the real load, and signoff found
/// what they missed. Fitting per-layer capacitance to the routed lengths of 23,557 nets reproduced
/// extraction with a 7.3% median error; with it, the same heavy nets estimate at a median 0.95x
/// (p10 0.86, p90 1.07) — slightly pessimistic, the safe side.
#[test]
fn the_pre_route_estimate_uses_the_calibrated_layer_rc() {
    let cfg = cfg_for(&SKY130A);
    let line = cfg
        .lines()
        .find(|l| l.contains("\"LAYERS_RC\""))
        .expect("sky130 must emit LAYERS_RC");
    for (layer, cap) in [
        ("met1", "1.6116e-4"),
        ("met2", "1.4460e-4"),
        ("met3", "1.8581e-4"),
        ("met4", "1.6682e-4"),
        ("met5", "2.4831e-4"),
    ] {
        assert!(
            line.contains(&format!("{layer} {cap} ")),
            "{layer} must carry its FITTED capacitance {cap} pF/um: {line}"
        );
    }
    for (layer, res) in [
        ("met1", "8.9286e-4"),
        ("met3", "1.5667e-4"),
        ("met5", "1.7812e-5"),
    ] {
        assert!(
            line.contains(&format!(" {res}")) && line.contains(layer),
            "{layer} resistance must be the LEF's RPERSQ / width ({res} kohm/um): {line}"
        );
    }
    for l in SKY130A.estimator_layer_rc.expect("sky130 is calibrated") {
        assert!(
            SKY130A.metal_stack.contains(&l.layer),
            "{} is not a sky130 routing layer",
            l.layer
        );
    }
    assert!(
        !cfg_for(&GF180MCUC).contains("LAYERS_RC"),
        "an UNCALIBRATED process must emit nothing rather than borrow another process's numbers"
    );
}

/// The shuttle's innermost artifact — the user macro the Caravel wrapper places — is configured by
/// the compiler like every other block, not by a hand-written heredoc.
///
/// It was the one block the compiler never touched: `caravel-macro-harden.sh` wrote its own
/// config.json, so none of the measured closure settings reached it. Re-read at every corner on
/// 2026-09-23 its latest run (RUN_2026.09.21_18.38.24) carried 33 slow-corner max-slew violations,
/// built without the calibrated estimator or the slew recipe that closed every other block.
#[test]
fn the_vehicle_user_macro_is_configured_by_the_compiler() {
    use logicaffeine_compile::codegen_sva::flow_config::{
        vehicle_macro_config, MacroGrid, VehicleMacro,
    };
    let d = DesignProfile::new("dram_user_project", "wb_clk_i", 20.0);
    let m = VehicleMacro {
        die_um: (2800.0, 1760.0),
        hold_margin_ns: 0.7,
        grid: MacroGrid::Own,
    };
    let cfg = vehicle_macro_config(&d, &m, &SKY130A, &ClosureSettings::default())
        .expect("sky130's Caravel vehicle is characterized");
    let v: serde_json::Value =
        serde_json::from_str(&cfg).unwrap_or_else(|e| panic!("not JSON ({e}):\n{cfg}"));
    assert_eq!(v["FP_SIZING"], "absolute");
    assert_eq!(v["DIE_AREA"], "0 0 2800 1760");
    assert_eq!(v["VDD_NETS"], serde_json::json!(["vccd1"]));
    assert_eq!(v["GND_NETS"], serde_json::json!(["vssd1"]));
    assert_eq!(v["SYNTH_USE_PG_PINS_DEFINES"], "USE_POWER_PINS");
    assert_eq!(v["FP_PDN_ENABLE_RAILS"], 1);
    assert_eq!(
        v["RT_MAX_LAYER"], "met3",
        "the macro stays below the wrapper's power grid"
    );
    assert_eq!(v["FP_PIN_ORDER_CFG"], "dir::pin_order.cfg");
    assert_eq!(v["PL_RESIZER_HOLD_SLACK_MARGIN"], 0.7);
    assert_eq!(v["GLB_RESIZER_HOLD_SLACK_MARGIN"], 0.7);
    assert_eq!(v["DIODE_ON_PORTS"], "both");
    assert_eq!(v["QUIT_ON_HOLD_VIOLATIONS"], 1);
    assert!(
        v["LAYERS_RC"].is_string(),
        "the calibrated estimator must reach the macro too"
    );
    for key in ["RT_MAX_LAYER", "DIODE_ON_PORTS", "DIE_AREA", "FP_SIZING"] {
        assert_eq!(
            cfg.matches(&format!("\"{key}\":")).count(),
            1,
            "{key} emitted more than once — a JSON duplicate key silently keeps only one value"
        );
    }
    assert!(
        vehicle_macro_config(&d, &m, &GF180MCUC, &ClosureSettings::default()).is_err(),
        "a process whose vehicle power nets and macro layer ceiling have not been read must refuse"
    );

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sh = std::fs::read_to_string(root.join("scripts/caravel-macro-harden.sh")).unwrap();
    assert!(
        sh.contains("--vehicle-macro"),
        "the macro harden must take its config from the emitter"
    );
    assert!(sh.contains("--caravel-project \"$PROJECT\"") && sh.contains("\"$CP_MACRO\" \"$CP_CLOCK_PORT\" \"$CP_MACRO_PERIOD_NS\""),
        "the macro, its clock and its period come from the project registry (codegen_sva::caravel_project), which \
         reads the registered period — probing the emitter with a wrong period and scraping its refusal \
         killed this script with a silent exit 2 under set -euo pipefail");
    assert!(
        !sh.contains("\"DESIGN_NAME\": \"dram_user_project\""),
        "a hand-written config would bypass every measured setting again"
    );
}

/// A vehicle user macro takes power one of two ways, and how high it may route follows from which. With its own
/// grid it is a core: straps on both of the process's PDN layers and routing below them. Fed by the wrapper it is
/// efabless's reference user macro — `DESIGN_IS_CORE` 0, straps on met4 only, routing up to met4 — read back from
/// the scaffold's own config where it is fetched. What the wrapper then builds around each is held by
/// `caravel_project_lock::each_recorded_wrapper_built_the_grid_its_macro_takes`.
#[test]
fn a_wrapper_fed_macro_routes_up_to_the_strap_layer_the_wrapper_feeds() {
    use logicaffeine_compile::codegen_sva::flow_config::{
        vehicle_macro_config, MacroGrid, VehicleMacro,
    };
    let d = DesignProfile::new("probe", "wb_clk_i", 40.0);
    let cfg = |grid: MacroGrid, pdk: &PdkTarget| {
        vehicle_macro_config(
            &d,
            &VehicleMacro {
                die_um: (2800.0, 3360.0),
                hold_margin_ns: 0.1,
                grid,
            },
            pdk,
            &ClosureSettings::default(),
        )
    };
    let ua = SKY130A.user_area.unwrap();
    assert_eq!(
        ua.macro_max_layer,
        Some("met3"),
        "a macro with its own grid stays below both PDN layers"
    );
    assert_eq!(
        ua.fed_macro_max_layer,
        Some("met4"),
        "a wrapper-fed macro routes up to its strap layer"
    );

    let own = cfg(MacroGrid::Own, &SKY130A).unwrap();
    let v: serde_json::Value = serde_json::from_str(&own).unwrap();
    assert_eq!(v["RT_MAX_LAYER"], "met3");
    assert!(
        v.get("DESIGN_IS_CORE").is_none(),
        "its own grid is OpenLane's core default:\n{own}"
    );

    let fed = cfg(MacroGrid::WrapperFed, &SKY130A).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&fed).unwrap_or_else(|e| panic!("not JSON ({e}):\n{fed}"));
    assert_eq!(v["RT_MAX_LAYER"], "met4");
    assert_eq!(
        v["DESIGN_IS_CORE"], 0,
        "straps on the vertical PDN layer only"
    );
    for key in ["RT_MAX_LAYER", "DESIGN_IS_CORE"] {
        assert_eq!(
            fed.matches(&format!("\"{key}\":")).count(),
            1,
            "{key} emitted once:\n{fed}"
        );
    }
    assert!(
        cfg(MacroGrid::WrapperFed, &GF180MCUC).is_err(),
        "an unread vehicle's fed ceiling is refused, not guessed"
    );
    for g in [MacroGrid::Own, MacroGrid::WrapperFed] {
        assert_eq!(MacroGrid::parse(g.flag()), Some(g));
    }
    assert_eq!(MacroGrid::parse("core"), None);

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if !common::require_tool("eda-tools") {
        return;
    }
    if let Ok(text) = std::fs::read_to_string(
        root.join("tools/caravel_user_project/openlane/user_proj_example/config.json"),
    ) {
        let r: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            r["DESIGN_IS_CORE"], 0,
            "efabless's reference user macro is wrapper-fed"
        );
        assert_eq!(
            r["pdk::sky130*"]["RT_MAX_LAYER"],
            ua.fed_macro_max_layer.unwrap(),
            "and routes up to its strap layer"
        );
    }
}

/// Global-route timing repair can run without pin swapping, and when told to, BOTH of its
/// `repair_timing` calls skip it.
///
/// MEASURED 2026-09-24 on `soft_core_realbist` (RUN_2026.09.24_21.34.30 and _21.40.26, then offline
/// from the step-17 database): OpenROAD 41a51eaf's post-route `repair_timing` lost one net's driver
/// from its route tree (RSZ-0073 x51, `_19008_/Y`) and died on SIGSEGV, under both antenna recipes;
/// every signal net had global-route guides, so it is not an unrouted net. With `-skip_pin_swap` the
/// same step completes (exit 0).
#[test]
fn global_route_timing_repair_can_skip_pin_swapping() {
    let on = ClosureSettings {
        glb_skip_pin_swap: true,
        ..ClosureSettings::default()
    };
    let cfg = openlane_config(&DesignProfile::new("probe", "clk", 10.0), &SKY130A, &on);
    assert!(
        cfg.contains("\"LOGOS_GLB_SKIP_PIN_SWAP\": 1,"),
        "the lever must reach config.json"
    );
    assert!(
        cfg_for(&SKY130A).contains("\"LOGOS_GLB_SKIP_PIN_SWAP\": 0,"),
        "the default must be emitted explicitly"
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tcl = std::fs::read_to_string(
        root.join("scripts/openlane-overlay/scripts/openroad/resizer_routing_timing.tcl"),
    )
    .unwrap();
    let gate = tcl
        .find("$::env(LOGOS_GLB_SKIP_PIN_SWAP) == 1")
        .expect("the pin-swap gate");
    let calls: Vec<usize> = tcl.match_indices("repair_timing").map(|(i, _)| i).collect();
    assert!(
        calls.len() >= 2 && calls.iter().all(|&c| c > gate),
        "the gate must precede both repair_timing calls"
    );
    assert_eq!(
        tcl.matches("-skip_pin_swap").count(),
        2,
        "both calls — setup and hold — must receive the flag"
    );
    let sh = std::fs::read_to_string(root.join("scripts/openlane-run.sh")).unwrap();
    assert!(
        sh.contains("CFG_ARGS+=( --glb-skip-pin-swap )"),
        "the runner must forward the lever"
    );
    let emit = std::fs::read_to_string(
        root.join("crates/logicaffeine_compile/examples/emit_flow_config.rs"),
    )
    .unwrap();
    assert!(
        emit.contains("\"--glb-skip-pin-swap\""),
        "the emitter must accept the lever"
    );
}

/// The post-diode upsize can detect at EVERY library corner, and when told to, it really loads the
/// slow and fast libraries.
///
/// MEASURED 2026-09-23: every surviving max-slew violator on `systolic_pipe16_realbist` (252 pins)
/// and `systolic_os_pipe8_realbist` (38) was at ss_100C_1v60 only — typical-corner slew was 0 on
/// both — while the step read `-lib_typical` alone, so it repaired a corner signoff was not failing
/// on and never saw the one it was.
#[test]
fn the_post_diode_upsize_can_see_every_corner() {
    let on = ClosureSettings {
        post_diode_upsize: true,
        post_diode_upsize_all_corners: true,
        ..ClosureSettings::default()
    };
    let cfg = openlane_config(&DesignProfile::new("probe", "clk", 10.0), &SKY130A, &on);
    assert!(
        cfg.contains("\"LOGOS_UPSIZE_ALL_CORNERS\": 1,"),
        "the lever must reach config.json"
    );
    assert!(
        cfg_for(&SKY130A).contains("\"LOGOS_UPSIZE_ALL_CORNERS\": 0,"),
        "the default must be emitted explicitly"
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let up = std::fs::read_to_string(
        root.join("scripts/openlane-overlay/scripts/openroad/logos_post_diode_upsize.tcl"),
    )
    .unwrap();
    let gate = up
        .find("$::env(LOGOS_UPSIZE_ALL_CORNERS) == 1")
        .expect("the all-corners gate");
    let rd = &up[gate..];
    assert!(rd[..rd.find("} else {").unwrap()].contains("-lib_slowest $::env(RSZ_LIB_SLOWEST)")
            && rd[..rd.find("} else {").unwrap()].contains("-lib_fastest $::env(RSZ_LIB_FASTEST)"),
        "with the lever set, the step must load the slow and fast libraries, not the typical one alone");
    assert!(
        up.contains("sta::check_slew_limits \"NULL\" 1 \"NULL\" max"),
        "violators are read across ALL corners (corner NULL), not the command corner"
    );
    let sh = std::fs::read_to_string(root.join("scripts/openlane-run.sh")).unwrap();
    assert!(
        sh.contains("CFG_ARGS+=( --upsize-all-corners )")
            && sh.contains("CFG_ARGS+=( --antenna \"$ANTENNA\" )"),
        "the runner must forward the all-corners lever and the antenna strategy"
    );
}

/// A Caravel user macro repairs slew where signoff judges it: at every corner. The block recorder takes the worst
/// max-slew count across the typical report and all three RC corners' multi-corner reports, and a macro whose upsize
/// detected at the typical corner alone left only slow-corner violators — `wpe_user_project` came out with typical
/// slew 0 and 16 / 73 / 112 at min / nom / max RC, every other gate clean (RUN_2026.09.26_20.13.06). Detection at
/// every corner takes a margin of at least the 20% that closed `systolic_pipe16_realbist` and
/// `systolic_os_pipe8_realbist` on the same slow-corner-only pattern (RUN_2026.09.24_16.37.47 / _19.16.31); the
/// typical-corner 52% would select most of the design at the slow corner. Each project carries its own measured margin
/// (`caravel_project_lock::each_project_detects_slew_at_its_own_measured_margin`).
#[test]
fn a_caravel_macro_repairs_slew_at_every_corner_signoff_judges() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sh = std::fs::read_to_string(root.join("scripts/caravel-macro-harden.sh")).unwrap();
    assert!(
        sh.contains(
            "--post-diode-upsize --upsize-all-corners --upsize-margin \"$CP_UPSIZE_MARGIN\""
        ),
        "the macro harden detects slew at every corner, at the project's all-corner margin"
    );
    let rec = std::fs::read_to_string(root.join("scripts/record-sky130-signoff.py")).unwrap();
    assert!(
        rec.contains("_sta_reports(signoff, \"checks\")") && rec.contains("_mca_dirs(signoff)"),
        "the recorder judges slew across the multi-corner reports, which is why detection must too"
    );
}

/// The technology-mapping script is a lever with a measurement behind it: OpenLane's default `AREA 0` never
/// finished mapping the whole-program engine (2 h 15 min in `fx; mfs`), `AREA 3` mapped it in 6 min. It is
/// emitted only when chosen, spelled as OpenLane reads it, and the WPE's Caravel macro chooses it.
#[test]
fn the_synthesis_strategy_is_a_lever_emitted_only_when_chosen() {
    use logicaffeine_compile::codegen_sva::caravel_project::{DRAM_PROJECT, WPE_PROJECT};
    use logicaffeine_compile::codegen_sva::flow_config::SynthStrategy;
    for (flag, st, ol) in [
        ("area0", SynthStrategy::Area(0), "AREA 0"),
        ("area3", SynthStrategy::Area(3), "AREA 3"),
        ("delay4", SynthStrategy::Delay(4), "DELAY 4"),
    ] {
        assert_eq!(SynthStrategy::parse(flag), Some(st));
        assert_eq!(st.flag(), flag);
        assert_eq!(st.openlane(), ol);
    }
    for bad in ["area4", "delay5", "area", "3", "fast1", "default"] {
        assert_eq!(
            SynthStrategy::parse(bad),
            None,
            "{bad:?} is not an OpenLane strategy"
        );
    }
    let d = DesignProfile::new("probe", "clk", 10.0);
    assert!(
        !cfg_for(&SKY130A).contains("SYNTH_STRATEGY"),
        "the default leaves OpenLane's own strategy in place"
    );
    let chosen = openlane_config(
        &d,
        &SKY130A,
        &ClosureSettings {
            synth_strategy: Some(SynthStrategy::Area(3)),
            ..ClosureSettings::default()
        },
    );
    assert_eq!(
        chosen.matches("\"SYNTH_STRATEGY\": \"AREA 3\",").count(),
        1,
        "{chosen}"
    );
    assert_eq!(
        WPE_PROJECT.synth_strategy,
        Some(SynthStrategy::Area(3)),
        "the engine's macro maps with AREA 3"
    );
    assert_eq!(
        DRAM_PROJECT.synth_strategy, None,
        "the command player keeps the strategy it was signed off with"
    );
    let sh = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/caravel-macro-harden.sh"),
    )
    .unwrap();
    assert!(
        sh.contains("--synth-strategy \"$CP_SYNTH_STRATEGY\""),
        "the macro harden passes its project's strategy"
    );
}

/// A vehicle user macro's ports are held to the harness's real minimum input delays, read from efabless's own
/// characterization — the `-min` input delays of the scaffold's wrapper SDC — so the macro's hold repair closes
/// the port-to-flop paths the wrapper would see, and no global margin has to buffer every internal path. The
/// characterization is the SDC's, value for value; an uncharacterized vehicle refuses; the config points at the
/// SDC and the macro harden writes it.
#[test]
fn a_vehicle_macro_is_timed_against_the_harness_input_delays() {
    use logicaffeine_compile::codegen_sva::flow_config::{
        vehicle_macro_config, vehicle_macro_sdc, MacroGrid, VehicleMacro,
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let wrapper_sdc = std::fs::read_to_string(
        root.join("benchmarks/silicon/caravel/base_user_project_wrapper.sdc"),
    )
    .unwrap();
    let mut scaffold: Vec<(String, f64)> = Vec::new();
    // The unconditional characterization is the top-level `-min` lines; the indented ones sit inside
    // `if { $::env(IO_SYNC) }`, which the wrapper leaves off because its GPIOs are asynchronous.
    for line in wrapper_sdc.lines() {
        let Some(rest) = line.strip_prefix("set_input_delay -min ") else {
            continue;
        };
        let value: f64 = rest.split_whitespace().next().unwrap().parse().unwrap();
        let ports = rest
            .split("[get_ports {")
            .nth(1)
            .and_then(|r| r.split('}').next())
            .unwrap()
            .to_string();
        scaffold.push((ports, value));
    }
    let ours = SKY130A
        .user_area
        .unwrap()
        .harness
        .expect("sky130's Caravel harness is characterized")
        .input_min_ns;
    let mut ours: Vec<(String, f64)> = ours.iter().map(|(p, v)| (p.to_string(), *v)).collect();
    scaffold.sort_by(|a, b| a.0.cmp(&b.0));
    ours.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        ours, scaffold,
        "the harness's minimum input delays are the scaffold SDC's, value for value"
    );

    let sdc = vehicle_macro_sdc(&SKY130A, HarnessClock::Core).expect("sky130's vehicle is characterized");
    assert!(
        sdc.starts_with("source $::env(SCRIPTS_DIR)/base.sdc\n"),
        "the process's base constraints first"
    );
    for (ports, ns) in &ours {
        assert_eq!(sdc.matches(&format!("set_input_delay -min {ns} -clock [get_clocks $::env(CLOCK_PORT)] [get_ports {{{ports}}}]")).count(), 1, "{ports}");
    }
    assert!(
        vehicle_macro_sdc(&GF180MCUC, HarnessClock::Core).is_err(),
        "an uncharacterized vehicle refuses rather than guessing its harness"
    );
    let d = DesignProfile::new("wpe_user_project", "wb_clk_i", 40.0);
    let cfg = vehicle_macro_config(
        &d,
        &VehicleMacro {
            die_um: (2800.0, 3360.0),
            hold_margin_ns: 0.1,
            grid: MacroGrid::WrapperFed,
        },
        &SKY130A,
        &ClosureSettings::default(),
    )
    .unwrap();
    assert_eq!(
        cfg.matches("\"BASE_SDC_FILE\": \"dir::vehicle_macro.sdc\",")
            .count(),
        1,
        "{cfg}"
    );
    let sh = std::fs::read_to_string(root.join("scripts/caravel-macro-harden.sh")).unwrap();
    assert!(
        sh.contains("--vehicle-macro-sdc \"$CP_CLOCK_PORT\"") && sh.contains("> \"$D/vehicle_macro.sdc\""),
        "the macro harden writes the SDC its config names, for the clock its project runs on"
    );
    assert!(
        sh.contains("--hold-margin \"$CP_HOLD_MARGIN_NS\""),
        "the hold margin is the project's"
    );
}

/// A vehicle user macro's clock is timed the way the chip it sits in is signed off. Caravel's chip-level signoff
/// SDC holds `clk` to 0.1 ns of uncertainty and a 3.75 % on-chip-variation derate; OpenLane's `base.sdc` defaults
/// to 0.25 ns and 5 %. Under those defaults the engine's 31,706 recirculating flops sat between 0 and 0.1 ns of
/// hold slack — met, but inside the repair margin — and got a buffer each. The macro SDC sets the chip's values
/// after `base.sdc`, so they are the ones in force; wherever Caravel is fetched, they are its SDC's, value for value.
#[test]
fn a_vehicle_macro_is_timed_with_the_chips_signoff_clock() {
    use logicaffeine_compile::codegen_sva::flow_config::vehicle_macro_sdc;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let h = SKY130A
        .user_area
        .unwrap()
        .harness
        .expect("sky130's Caravel harness is characterized");
    assert_eq!(
        (h.clock_uncertainty_ns, h.timing_derate),
        (0.1, 0.0375),
        "Caravel's chip signoff clock"
    );
    let sdc = vehicle_macro_sdc(&SKY130A, HarnessClock::Core).unwrap();
    let base_at = sdc
        .find("source $::env(SCRIPTS_DIR)/base.sdc\n")
        .expect("the process's base constraints");
    for line in [
        "set_clock_uncertainty 0.1 [get_clocks $::env(CLOCK_PORT)]\n",
        "set_timing_derate -early [expr {1 - 0.0375}]\n",
        "set_timing_derate -late [expr {1 + 0.0375}]\n",
    ] {
        assert_eq!(sdc.matches(line).count(), 1, "{line:?} in\n{sdc}");
        assert!(
            sdc.find(line).unwrap() > base_at,
            "{line:?} must follow base.sdc, or base.sdc's default wins"
        );
    }
    if !common::require_tool("eda-tools") {
        return;
    }
    let Ok(chip) = std::fs::read_to_string(root.join("tools/caravel/signoff/caravel/caravel.sdc"))
    else {
        return;
    };
    let uncertainty: f64 = chip
        .lines()
        .find_map(|l| {
            let rest = l.strip_prefix("set_clock_uncertainty ")?;
            rest.contains("[get_clocks {clk ")
                .then(|| rest.split_whitespace().next()?.parse().ok())?
        })
        .expect("the chip signoff SDC's clk uncertainty");
    let derate: f64 = chip
        .lines()
        .find_map(|l| l.strip_prefix("set derate ")?.trim().parse().ok())
        .expect("the chip signoff SDC's derate");
    assert_eq!(
        (h.clock_uncertainty_ns, h.timing_derate),
        (uncertainty, derate),
        "the chip signoff SDC's values"
    );
}

/// The ports Caravel's harness times against its core clock are the scaffold SDC's: every port its top level delays
/// against `clk`, inputs and outputs. The indented ones sit inside `if { $::env(IO_SYNC) }`, which the wrapper leaves
/// off because its GPIOs are asynchronous. A macro whose logic runs on another clock times exactly these against
/// `wb_clk_i`.
#[test]
fn the_harness_core_clock_ports_are_the_scaffold_sdcs() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sdc = std::fs::read_to_string(
        root.join("benchmarks/silicon/caravel/base_user_project_wrapper.sdc"),
    )
    .unwrap();
    let (mut ins, mut outs) = (
        std::collections::BTreeSet::new(),
        std::collections::BTreeSet::new(),
    );
    for line in sdc.lines() {
        let set = if line.starts_with("set_input_delay ") {
            &mut ins
        } else if line.starts_with("set_output_delay ") {
            &mut outs
        } else {
            continue;
        };
        assert!(line.contains("-clock [get_clocks {clk}]"), "a harness delay is against clk: {line}");
        let ports = line
            .split("[get_ports {")
            .nth(1)
            .and_then(|r| r.split('}').next())
            .unwrap();
        set.insert(ports.to_string());
    }
    let h = SKY130A.user_area.unwrap().harness.unwrap();
    let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<std::collections::BTreeSet<_>>();
    assert_eq!(names(h.core_clock_inputs), ins, "the inputs the harness times against its core clock");
    assert_eq!(names(h.core_clock_outputs), outs, "the outputs it times against it");
    assert!(ins.contains("wb_rst_i"), "the management SoC's reset among them");
    for (ports, _) in h.input_min_ns {
        assert!(ins.contains(*ports), "{ports}: a minimum input delay on a core-clock port");
    }
}

/// A macro whose logic runs on `user_clock2` is timed on that clock (`CLOCK_PORT`), and the harness's bus on its
/// own: `wb_clk_i` at the chip's signoff period, with the chip's uncertainty and `base.sdc`'s ideal transition,
/// asynchronous to the macro's clock. Every port the harness times against the core clock has `base.sdc`'s delay
/// against the macro's clock taken off and its own put on against `wb_clk_i`: `IO_PCT` of the core period, as
/// `base.sdc` gives the ports of its own clock, refined by the harness's minimum input delays. `wb_clk_i` is a clock
/// now, so the data delay `base.sdc` gave it comes off and it takes the clock driving cell. The removals name the
/// clock they remove: MEASURED in the pinned OpenSTA, `unset_input_delay` without `-clock` removes nothing (it
/// removes the delays that have no clock), and `write_sdc` still showed `wb_clk_i` delayed against `user_clock2`.
/// A macro on the core clock gets none of this: its SDC is the one the command player was signed off with.
#[test]
fn a_macro_on_the_user_clock_times_the_harness_bus_on_the_core_clock() {
    use logicaffeine_compile::codegen_sva::flow_config::vehicle_macro_sdc;
    let h = SKY130A.user_area.unwrap().harness.unwrap();
    let core = vehicle_macro_sdc(&SKY130A, HarnessClock::Core).unwrap();
    let user = vehicle_macro_sdc(&SKY130A, HarnessClock::User2).unwrap();
    for word in ["wb_clk_i", "set_clock_groups", "unset_"] {
        assert!(!core.contains(word), "a macro on the core clock has one clock: {word} in\n{core}");
    }
    let lines: Vec<&str> = user.lines().collect();
    let at = |line: &str| -> usize {
        let hits: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| **l == line)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(hits.len(), 1, "{line:?} once in\n{user}");
        hits[0]
    };
    let p = h.core_clock_period_ns;
    let off = "unset_input_delay -clock [get_clocks $::env(CLOCK_PORT)]";
    let off_out = "unset_output_delay -clock [get_clocks $::env(CLOCK_PORT)]";
    let on = format!("[expr {{{p} * $::env(IO_PCT)}}] -clock [get_clocks wb_clk_i]");
    let create = at(&format!("create_clock [get_ports wb_clk_i] -name wb_clk_i -period {p}"));
    assert!(at("source $::env(SCRIPTS_DIR)/base.sdc") < create, "after base.sdc, whose delays it moves");
    for line in [
        format!("set_clock_uncertainty {} [get_clocks wb_clk_i]", h.clock_uncertainty_ns),
        "set_clock_transition $::env(SYNTH_CLOCK_TRANSITION) [get_clocks wb_clk_i]".to_string(),
        "set_clock_groups -asynchronous -group [get_clocks $::env(CLOCK_PORT)] -group [get_clocks wb_clk_i]".to_string(),
        format!("{off} [get_ports wb_clk_i]"),
        "set_driving_cell -lib_cell $::env(SYNTH_CLK_DRIVING_CELL) -pin $::env(SYNTH_CLK_DRIVING_CELL_PIN) [get_ports wb_clk_i]"
            .to_string(),
    ] {
        assert!(at(&line) > create, "{line:?} once wb_clk_i is a clock");
    }
    for port in h.core_clock_inputs {
        let un = at(&format!("{off} [get_ports {{{port}}}]"));
        let set = at(&format!("set_input_delay {on} [get_ports {{{port}}}]"));
        assert!(create < un && un < set, "{port}: off the macro's clock, then onto wb_clk_i");
        if let Some((_, ns)) = h.input_min_ns.iter().find(|(ports, _)| ports == port) {
            let min = at(&format!("set_input_delay -min {ns} -clock [get_clocks wb_clk_i] [get_ports {{{port}}}]"));
            assert!(set < min, "{port}: the harness's minimum refines the delay it replaces");
        }
    }
    for port in h.core_clock_outputs {
        let un = at(&format!("{off_out} [get_ports {{{port}}}]"));
        let set = at(&format!("set_output_delay {on} [get_ports {{{port}}}]"));
        assert!(create < un && un < set, "{port}: off the macro's clock, then onto wb_clk_i");
    }
    for (ports, ns) in h.input_min_ns {
        assert!(
            !user.contains(&format!("set_input_delay -min {ns} -clock [get_clocks $::env(CLOCK_PORT)] [get_ports {{{ports}}}]")),
            "{ports}: its minimum is against wb_clk_i, not the macro's clock"
        );
    }
    for line in core.lines().filter(|l| !l.starts_with("set_input_delay -min ")) {
        assert!(lines.contains(&line), "the user-clock SDC keeps the core-clock one's {line:?}");
    }
    let emit = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../logicaffeine_compile/examples/emit_flow_config.rs"),
    )
    .unwrap();
    let arm = &emit[emit
        .find("if a.first().map(String::as_str) == Some(\"--vehicle-macro-sdc\")")
        .expect("the emitter prints the SDC")..];
    let arm = &arm[..arm.find("return;").unwrap()];
    assert!(
        arm.contains("a.get(1).and_then(|p| HarnessClock::by_port(p)).unwrap_or_else(|| usage())")
            && arm.contains("std::process::exit(2)")
            && arm.contains("vehicle_macro_sdc(pdk, clock)"),
        "the SDC is for the clock the macro harden names, and a port that is no harness clock is refused, \
         never defaulted:\n{arm}"
    );
}

/// The vertical power-strap pitch is a lever emitted only when chosen: a design whose cells draw current midway
/// between two straps loses it along its met1 rails, and only a narrower pitch shortens the rail. The default keeps
/// OpenLane's own grid, so every block built without it is the grid it was signed off on.
#[test]
fn the_power_strap_pitch_is_a_lever_emitted_only_when_chosen() {
    assert!(
        !cfg_for(&SKY130A).contains("FP_PDN_VPITCH"),
        "the default leaves OpenLane's own strap pitch in place"
    );
    let d = DesignProfile::new("probe", "clk", 10.0);
    let chosen = openlane_config(
        &d,
        &SKY130A,
        &ClosureSettings {
            pdn_vpitch_um: Some(76.8),
            ..ClosureSettings::default()
        },
    );
    assert_eq!(
        chosen.matches("\"FP_PDN_VPITCH\": 76.8,").count(),
        1,
        "{chosen}"
    );
    let emit = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../logicaffeine_compile/examples/emit_flow_config.rs"),
    )
    .unwrap();
    let arm = &emit[emit
        .find("\"--pdn-vpitch\" =>")
        .expect("the emitter takes the pitch")..];
    let arm = &arm[..arm.find("i += 2;").unwrap()];
    assert!(
        arm.contains("std::process::exit(2)") && arm.contains("> 0.0"),
        "a pitch that is not a positive length is refused, never ignored:\n{arm}"
    );
}

/// The setup repair's margin is a lever emitted only when chosen, for a block and a Caravel macro alike. The placement
/// and global-route repairs judge setup on estimated parasitics at the nominal RC corner, and signoff on parasitics
/// extracted at the max-RC corner, so a block whose routed paths outgrow the estimate needs the difference as margin.
/// The default keeps OpenLane's own margins (0.05 and 0.025 ns), so every block built without it is the repair it was
/// signed off with.
#[test]
fn the_setup_repair_margin_is_a_lever_emitted_only_when_chosen() {
    use logicaffeine_compile::codegen_sva::flow_config::{vehicle_macro_config, MacroGrid, VehicleMacro};
    let plain = cfg_for(&SKY130A);
    assert!(
        !plain.contains("PL_RESIZER_SETUP_SLACK_MARGIN") && !plain.contains("GLB_RESIZER_SETUP_SLACK_MARGIN"),
        "the default leaves OpenLane's own setup margins in place"
    );
    assert_eq!(ClosureSettings::default().setup_slack_margin_ns, None);
    let chosen = ClosureSettings { setup_slack_margin_ns: Some(1.25), ..ClosureSettings::default() };
    let d = DesignProfile::new("probe", "clk", 10.0);
    let m = VehicleMacro { die_um: (2800.0, 3360.0), hold_margin_ns: 0.1, grid: MacroGrid::WrapperFed };
    for s in [
        openlane_config(&d, &SKY130A, &chosen),
        vehicle_macro_config(&d, &m, &SKY130A, &chosen).expect("sky130A takes a vehicle macro"),
    ] {
        let v: serde_json::Value = serde_json::from_str(&s).unwrap_or_else(|e| panic!("{e}:\n{s}"));
        assert_eq!(v["PL_RESIZER_SETUP_SLACK_MARGIN"], 1.25, "{s}");
        assert_eq!(v["GLB_RESIZER_SETUP_SLACK_MARGIN"], 1.25, "{s}");
        assert_eq!(s.matches("\"GLB_RESIZER_SETUP_SLACK_MARGIN\"").count(), 1, "{s}");
    }
    let em = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile/examples/emit_flow_config.rs"),
    )
    .unwrap();
    let arm = &em[em.find("\"--setup-slack-margin\" =>").expect("the emitter takes the margin")..];
    let arm = &arm[..arm.find("i += 2;").unwrap()];
    assert!(
        arm.contains("std::process::exit(2)")
            && arm.contains("Ok(m) if m >= 0.0 => Some(m)")
            && arm.contains("setup_slack_margin_ns = "),
        "a margin that is not a non-negative time is refused, never ignored:\n{arm}"
    );
}

/// The hold repair's margin is a lever emitted only when chosen. The repair judges hold on estimated parasitics and
/// signoff on extracted ones, so a block whose clock skew grows once routed needs the difference as margin. The
/// default keeps OpenLane's own margins, so every block built without it is the repair it was signed off with.
#[test]
fn the_hold_repair_margin_is_a_lever_emitted_only_when_chosen() {
    let plain = cfg_for(&SKY130A);
    assert!(
        !plain.contains("PL_RESIZER_HOLD_SLACK_MARGIN")
            && !plain.contains("GLB_RESIZER_HOLD_SLACK_MARGIN"),
        "the default leaves OpenLane's own hold margins in place"
    );
    let d = DesignProfile::new("probe", "clk", 10.0);
    let chosen = openlane_config(
        &d,
        &SKY130A,
        &ClosureSettings {
            hold_slack_margin_ns: Some(0.15),
            ..ClosureSettings::default()
        },
    );
    assert_eq!(
        chosen
            .matches("\"PL_RESIZER_HOLD_SLACK_MARGIN\": 0.15,")
            .count(),
        1,
        "{chosen}"
    );
    assert_eq!(
        chosen
            .matches("\"GLB_RESIZER_HOLD_SLACK_MARGIN\": 0.15,")
            .count(),
        1,
        "{chosen}"
    );
    let emit = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../logicaffeine_compile/examples/emit_flow_config.rs"),
    )
    .unwrap();
    let arm = &emit[emit
        .find("\"--hold-slack-margin\" =>")
        .expect("the emitter takes the margin")..];
    let arm = &arm[..arm.find("i += 2;").unwrap()];
    assert!(
        arm.contains("std::process::exit(2)") && arm.contains(">= 0.0"),
        "a margin that is not a non-negative time is refused, never ignored:\n{arm}"
    );
}

/// Legalization's padding is a lever emitted only when chosen, in sites a side (`DPL_CELL_PADDING`, which OpenLane
/// takes as a total and halves to a side, so a total of one pads nothing). The default leaves OpenLane's none, so every
/// block built without it is the placement it was signed off with.
#[test]
fn the_legalization_padding_is_a_lever_emitted_only_when_chosen() {
    let plain = cfg_for(&SKY130A);
    assert!(!plain.contains("\"DPL_CELL_PADDING\""), "the default leaves OpenLane's own padding:\n{plain}");
    assert_eq!(ClosureSettings::default().dpl_cell_padding_per_side, None);
    let d = DesignProfile::new("probe", "clk", 10.0);
    for (side, total) in [(1u32, 2u32), (2, 4)] {
        let s = openlane_config(&d, &SKY130A, &ClosureSettings { dpl_cell_padding_per_side: Some(side), ..ClosureSettings::default() });
        let v: serde_json::Value = serde_json::from_str(&s).unwrap_or_else(|e| panic!("{e}:\n{s}"));
        assert_eq!(v["DPL_CELL_PADDING"], total, "{side} site(s) a side is a total of {total}:\n{s}");
        assert_eq!(s.matches("\"DPL_CELL_PADDING\"").count(), 1, "{s}");
    }
    let em = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile/examples/emit_flow_config.rs"),
    )
    .unwrap();
    let arm = &em[em.find("\"--dpl-padding-per-side\" =>").expect("the emitter takes the padding")..];
    let arm = &arm[..arm.find("i += 2;").unwrap()];
    assert!(
        arm.contains("std::process::exit(2)") && arm.contains("*n >= 1") && arm.contains("dpl_cell_padding_per_side = Some(n)"),
        "a padding that is not a positive number of sites is refused, never ignored:\n{arm}"
    );
}

/// The heuristic diode pass protects every net whose span passes its threshold (`HEURISTIC_ANTENNA_THRESHOLD`, in um)
/// before the route, and a project names its own (`CaravelProject::macro_antenna_threshold_um`). The emitter takes it
/// as chosen and refuses one that is no positive length: a threshold it could not read would otherwise harden the
/// block at the default 300 and record that run under the project's recipe.
#[test]
fn a_diode_threshold_that_is_no_positive_length_is_refused() {
    let d = DesignProfile::new("probe", "clk", 10.0);
    let s = openlane_config(&d, &SKY130A, &ClosureSettings { heuristic_antenna_threshold: 700, ..ClosureSettings::default() });
    assert_eq!(s.matches("\"HEURISTIC_ANTENNA_THRESHOLD\": 700,").count(), 1, "{s}");
    let em = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile/examples/emit_flow_config.rs"),
    )
    .unwrap();
    let arm = &em[em.find("\"--antenna-threshold\" =>").expect("the emitter takes the threshold")..];
    let arm = &arm[..arm.find("i += 2;").unwrap()];
    assert!(
        arm.contains("std::process::exit(2)")
            && arm.contains("*um >= 1")
            && arm.contains("heuristic_antenna_threshold =")
            && !arm.contains("unwrap_or(c.heuristic_antenna_threshold)"),
        "a threshold that is not a positive length is refused, never replaced by the default:\n{arm}"
    );
}

/// Global routing's effort is a lever emitted only when chosen: the rip-up iterations it spends removing overflow
/// (`GRT_OVERFLOW_ITERS`) and the share of every gcell's capacity it holds back (`GRT_ADJUSTMENT`). Each is emitted on
/// its own, as the value chosen, beside the per-layer adjustments; the default keeps OpenLane's own 50 iterations and
/// 0.3, so every block built without them is the route it was signed off with.
#[test]
fn the_global_route_effort_is_a_lever_emitted_only_when_chosen() {
    let plain = cfg_for(&SKY130A);
    assert!(
        !plain.contains("\"GRT_OVERFLOW_ITERS\"") && !plain.contains("\"GRT_ADJUSTMENT\""),
        "the default leaves OpenLane's own global-route effort in place:\n{plain}"
    );
    assert_eq!(ClosureSettings::default().grt_overflow_iters, None);
    assert_eq!(ClosureSettings::default().grt_adjustment, None);
    let d = DesignProfile::new("probe", "clk", 10.0);
    let emit = |c: &ClosureSettings| -> (String, serde_json::Value) {
        let s = openlane_config(&d, &SKY130A, c);
        let v = serde_json::from_str(&s).unwrap_or_else(|e| panic!("{e}:\n{s}"));
        (s, v)
    };
    let (iters, v) = emit(&ClosureSettings {
        grt_overflow_iters: Some(150),
        ..ClosureSettings::default()
    });
    assert_eq!(iters.matches("\"GRT_OVERFLOW_ITERS\": 150,").count(), 1, "{iters}");
    assert!(v["GRT_ADJUSTMENT"].is_null(), "the iterations alone leave the adjustment OpenLane's");
    let (adj, v) = emit(&ClosureSettings {
        grt_adjustment: Some(0.2),
        ..ClosureSettings::default()
    });
    assert_eq!(adj.matches("\"GRT_ADJUSTMENT\": 0.2,").count(), 1, "{adj}");
    assert!(v["GRT_OVERFLOW_ITERS"].is_null(), "the adjustment alone leaves the iterations OpenLane's");
    let (both, v) = emit(&ClosureSettings {
        grt_overflow_iters: Some(400),
        grt_adjustment: Some(0.0),
        ..ClosureSettings::default()
    });
    assert_eq!(v["GRT_OVERFLOW_ITERS"], 400, "{both}");
    assert_eq!(v["GRT_ADJUSTMENT"], 0.0, "{both}");
    let at = |k: &str| both.find(&format!("\"{k}\"")).unwrap_or_else(|| panic!("{k} in:\n{both}"));
    assert!(
        at("GRT_LAYER_ADJUSTMENTS") < at("GRT_OVERFLOW_ITERS")
            && at("GRT_OVERFLOW_ITERS") < at("GRT_ADJUSTMENT")
            && at("GRT_ADJUSTMENT") < at("CTS_CLK_MAX_WIRE_LENGTH"),
        "the global-route effort sits with the per-layer adjustments:\n{both}"
    );
    let em = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../logicaffeine_compile/examples/emit_flow_config.rs"),
    )
    .unwrap();
    let arm_of = |flag: &str| -> String {
        let a = &em[em
            .find(&format!("\"{flag}\" =>"))
            .unwrap_or_else(|| panic!("the emitter takes {flag}"))..];
        a[..a.find("i += 2;").unwrap()].to_string()
    };
    let arm = arm_of("--grt-overflow-iters");
    assert!(
        arm.contains("std::process::exit(2)") && arm.contains("> 0") && arm.contains("grt_overflow_iters = Some("),
        "an iteration count that is not a positive number is refused, never ignored:\n{arm}"
    );
    let arm = arm_of("--grt-adjustment");
    assert!(
        arm.contains("std::process::exit(2)")
            && arm.contains("(0.0..1.0).contains")
            && arm.contains("grt_adjustment = Some("),
        "an adjustment outside [0, 1) is refused, never ignored:\n{arm}"
    );
}

/// A block the comparative-synthesis bench measures records what a signoff block refuses: a run whose typical corner
/// misses setup completes, so a period search reads the slack it missed by, and yosys's check findings (latches,
/// out-of-range selects, undriven wires) leave the flow to the recorder rather than stopping it. Hold stops both, and a
/// signoff block keeps OpenLane's own quitting on setup and on the checks.
#[test]
fn a_comparative_block_records_a_setup_miss_and_its_synthesis_findings_and_still_stops_on_hold() {
    let d = DesignProfile::new("probe", "clk", 10.0);
    let signoff: serde_json::Value = serde_json::from_str(&openlane_config(&d, &SKY130A, &ClosureSettings::default())).unwrap();
    let measured = ClosureSettings { comparative: true, ..ClosureSettings::default() };
    let bench: serde_json::Value = serde_json::from_str(&openlane_config(&d, &SKY130A, &measured)).unwrap();
    for key in ["QUIT_ON_SETUP_VIOLATIONS", "QUIT_ON_SYNTH_CHECKS"] {
        assert!(signoff[key].is_null(), "a signoff block quits as OpenLane does on {key}");
        assert_eq!(bench[key], 0, "a comparative block records what {key} would stop");
    }
    assert_eq!(signoff["QUIT_ON_HOLD_VIOLATIONS"], 1);
    assert_eq!(bench["QUIT_ON_HOLD_VIOLATIONS"], 1, "hold stops a comparative block too");
    assert!(!ClosureSettings::default().comparative, "a block is signed off unless it says otherwise");
}

/// A comparative block repairs hold with the bench's margin, so the fastest corner closes the hold its typical-corner
/// repair met; a margin the caller gives is the one used, and a signoff block keeps OpenLane's own unless given one.
#[test]
fn a_comparative_block_repairs_hold_with_a_margin_for_every_corner() {
    use logicaffeine_compile::codegen_sva::flow_config::COMPARATIVE_HOLD_MARGIN_NS;
    let d = DesignProfile::new("probe", "clk", 10.0);
    let cfg = |c: &ClosureSettings| -> serde_json::Value { serde_json::from_str(&openlane_config(&d, &SKY130A, c)).unwrap() };
    let signoff = cfg(&ClosureSettings::default());
    assert!(signoff["GLB_RESIZER_HOLD_SLACK_MARGIN"].is_null() && signoff["PL_RESIZER_HOLD_SLACK_MARGIN"].is_null());
    let bench = cfg(&ClosureSettings { comparative: true, ..ClosureSettings::default() });
    for key in ["PL_RESIZER_HOLD_SLACK_MARGIN", "GLB_RESIZER_HOLD_SLACK_MARGIN"] {
        assert_eq!(bench[key].as_f64(), Some(COMPARATIVE_HOLD_MARGIN_NS), "{key}");
    }
    assert!(COMPARATIVE_HOLD_MARGIN_NS >= 0.25, "the measured typical-to-fast gap is 0.14 ns past OpenLane's 0.05");
    let given = cfg(&ClosureSettings { comparative: true, hold_slack_margin_ns: Some(0.4), ..ClosureSettings::default() });
    assert_eq!(given["GLB_RESIZER_HOLD_SLACK_MARGIN"].as_f64(), Some(0.4), "the caller's margin wins");
}

/// A comparative block is synthesized for delay (`SYNTH_STRATEGY "DELAY 0"`), every candidate alike, so the period each
/// closes at is its design's, not the area-recovered netlist an area script leaves at whatever period a run is given:
/// OpenLane hands abc the clock period as its target, and an area script recovers area down to it. MEASURED by the
/// clock-push search on the signed blocks under DELAY 0 (logs/clock-push, 10-17): decision_hft_bist 15.0 -> 4.4 ns,
/// dram_executor 10.5 -> 5.0, cgra_sumto 18 -> 6.4. A strategy the caller names is the one used; a signoff block keeps
/// its recipe's.
#[test]
fn a_comparative_block_is_synthesized_for_delay() {
    use logicaffeine_compile::codegen_sva::flow_config::{SynthStrategy, COMPARATIVE_SYNTH_STRATEGY};
    let d = DesignProfile::new("probe", "clk", 10.0);
    let cfg = |c: &ClosureSettings| -> serde_json::Value { serde_json::from_str(&openlane_config(&d, &SKY130A, c)).unwrap() };
    assert_eq!(COMPARATIVE_SYNTH_STRATEGY, SynthStrategy::Delay(0));
    assert!(cfg(&ClosureSettings::default())["SYNTH_STRATEGY"].is_null(), "a signoff block's recipe chooses it");
    assert_eq!(cfg(&ClosureSettings { comparative: true, ..ClosureSettings::default() })["SYNTH_STRATEGY"], "DELAY 0");
    let named = ClosureSettings { comparative: true, synth_strategy: Some(SynthStrategy::Area(3)), ..ClosureSettings::default() };
    assert_eq!(cfg(&named)["SYNTH_STRATEGY"], "AREA 3", "the caller's strategy wins");
}

/// A comparative block closes antenna with every repair the flow has: at the jog router's fixpoint a pin that still
/// violates gets one more diode (`LOGOS_ECO_DIODE_FALLBACK`). The bench's first fold-wrapped attempts met setup and hold
/// and ended on exactly that residual — `qorsyn_k1_bambu_pipe_c_a7p10_p32_5` (RUN_2026.10.17_01.53.18) one met4 net at
/// 1.02 times its limit, `qorsyn_k1_bambu_fsm_c_a7p10_p20_5` two pins — so no period could close them. One flow serves
/// every candidate of a comparison, so it is on for all of them; a signoff block keeps the repairs its recipe chose.
#[test]
fn a_comparative_block_gives_a_still_violating_antenna_pin_one_more_diode() {
    let d = DesignProfile::new("probe", "clk", 10.0);
    let cfg = |c: &ClosureSettings| -> serde_json::Value { serde_json::from_str(&openlane_config(&d, &SKY130A, c)).unwrap() };
    assert_eq!(cfg(&ClosureSettings::default())["LOGOS_ECO_DIODE_FALLBACK"], 0, "a signoff block's recipe chooses it");
    assert_eq!(cfg(&ClosureSettings { comparative: true, ..ClosureSettings::default() })["LOGOS_ECO_DIODE_FALLBACK"], 1);
    assert_eq!(
        cfg(&ClosureSettings { eco_diode_fallback: true, ..ClosureSettings::default() })["LOGOS_ECO_DIODE_FALLBACK"],
        1,
        "a recipe that chooses it has it"
    );
}
