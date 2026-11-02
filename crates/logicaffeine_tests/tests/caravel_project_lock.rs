//! caravel_project_lock — every Caravel user project is one entry of `codegen_sva::caravel_project`, and the
//! chain that turns it into a shuttle submission reads that entry rather than naming a macro itself.
//!
//! WHY. The chain — macro harden, wrapper harden, golden-deck DRC, submission, `mpw_precheck` — was written
//! around one macro, `dram_user_project`, whose name, die, placement, clock, design directories and ledgers
//! were spelled out in four scripts. A second project either forks those scripts or overwrites the first
//! project's evidence: both wrappers are `user_project_wrapper`, so a golden-deck verdict filed under the
//! top cell's name is overwritten by whichever wrapper ran last.
//!
//! What this file locks:
//!   1. projects are distinct in everything that must not be shared — designs, wrapper RTL, ledgers,
//!      golden-deck keys, submission directories, overlays — and the first project keeps the paths its
//!      evidence was recorded under;
//!   2. every project fits sky130's Caravel, runs on a harness clock the chip is signed for (the core clock no
//!      faster than its signoff period, a faster project on `user_clock2`), and its macro is signed off at no
//!      slower than the chip's clock; every refusal is reachable; the wrapper is timed at every clock the chip
//!      runs, and its ledger records them;
//!   3. the scripts' view of a project is complete and made of bare words, and every script of the chain
//!      takes the project from the registry and names no macro of its own;
//!   4. every wrapper RTL is its emitter's output, and every overlay carries the files a submission needs;
//!   5. a recorded wrapper's golden-deck verdict is filed under its project's key and checked the GDS the
//!      wrapper ledger ships;
//!   6. an assembled submission is the recorded pair and the repo's overlay: the wrapper and macro GDS the
//!      ledger names, the macro's RTL hash, a netlist manifest that resolves `mprj`, device LVS against the
//!      macro's netlist, and every overlay file as the overlay holds it.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(caravel_project_lock)'`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::caravel_project::{
    CaravelProject, CaravelProjectUnplaceable, CARAVEL_PROJECTS, DRAM_PROJECT, WPE_PROJECT,
};
use logicaffeine_compile::codegen_sva::caravel_rtl::emit_user_project_wrapper_verilog;
use logicaffeine_compile::codegen_sva::flow_config::{MacroGrid, SynthStrategy};
use logicaffeine_compile::codegen_sva::pdk_target::{HarnessClock, PdkTarget, UserArea, SKY130A};
use logicaffeine_compile::codegen_sva::signoff_blocks::{registered_period_ns, SIGNOFF_BLOCKS};
use logicaffeine_compile::codegen_sva::wpe_caravel::{WPE_CARAVEL, WPE_CLOCK};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

// ── 1: distinct projects ───────────────────────────────────────────────────────────────────────────────────

#[test]
fn projects_share_nothing_that_would_overwrite_another() {
    let fields: [(&str, fn(&CaravelProject) -> &'static str); 10] = [
        ("key", |p| p.key),
        ("macro_module", |p| p.macro_module),
        ("macro_design", |p| p.macro_design),
        ("wrapper_design", |p| p.wrapper_design),
        ("wrapper_rtl", |p| p.wrapper_rtl),
        ("wrapper_ledger", |p| p.wrapper_ledger),
        ("golden_key", |p| p.golden_key),
        ("precheck_ledger", |p| p.precheck_ledger),
        ("submission_dir", |p| p.submission_dir),
        ("overlay", |p| p.overlay),
    ];
    for (name, field) in fields {
        let values: BTreeSet<&str> = CARAVEL_PROJECTS.iter().map(field).collect();
        assert_eq!(
            values.len(),
            CARAVEL_PROJECTS.len(),
            "two projects share a {name}"
        );
    }
    let designs: BTreeSet<&str> = CARAVEL_PROJECTS
        .iter()
        .flat_map(|p| [p.macro_design, p.wrapper_design])
        .collect();
    assert_eq!(
        designs.len(),
        2 * CARAVEL_PROJECTS.len(),
        "a macro design doubles as another project's wrapper design"
    );
    assert_eq!(CaravelProject::by_key("wpe"), Some(&WPE_PROJECT));
    assert_eq!(CaravelProject::by_key("nope"), None);
}

/// The first project was hardened, recorded and precheck-passed before the registry existed; its entry names
/// exactly the paths that evidence lives under.
#[test]
fn the_dram_project_names_the_paths_its_evidence_lives_under() {
    assert_eq!(DRAM_PROJECT.macro_module, "dram_user_project");
    assert_eq!(
        (DRAM_PROJECT.macro_design, DRAM_PROJECT.wrapper_design),
        ("logos_upm", "logos_upw")
    );
    assert_eq!(
        DRAM_PROJECT.wrapper_ledger,
        "benchmarks/silicon/caravel_signoff.json"
    );
    assert_eq!(
        DRAM_PROJECT.precheck_ledger,
        "benchmarks/silicon/mpw_precheck.json"
    );
    assert_eq!(DRAM_PROJECT.golden_key, "user_project_wrapper");

    let led: serde_json::Value = serde_json::from_str(&read(DRAM_PROJECT.wrapper_ledger)).unwrap();
    assert_eq!(led["user_project_wrapper"]["macro"], "dram_user_project");
    assert_eq!(
        led["user_project_wrapper"]["clock_period_ns"].as_f64(),
        Some(DRAM_PROJECT.clock_period_ns())
    );
    assert_eq!(led["user_project_wrapper"]["die_area"], "0 0 2920 3520");
}

// ── 2: the vehicle and the clock ───────────────────────────────────────────────────────────────────────────

#[test]
fn every_project_fits_the_vehicle_and_its_macro_guarantees_the_chip_clock() {
    for p in CARAVEL_PROJECTS {
        p.check(&SKY130A)
            .unwrap_or_else(|e| panic!("{}: {e}", p.key));
        let period = p.macro_period_ns();
        match registered_period_ns(p.macro_module) {
            Some(reg) => assert_eq!(
                period, reg,
                "{}: a registered macro is hardened at its registered period",
                p.key
            ),
            None => assert_eq!(
                period,
                p.clock_period_ns(),
                "{}: the first build is at the chip's clock",
                p.key
            ),
        }
        assert!(
            period <= p.clock_period_ns(),
            "{}: the macro is signed off at no slower than the chip runs",
            p.key
        );
    }
    assert_eq!(
        WPE_PROJECT.clock_period_ps, WPE_CARAVEL.clk_period_ps,
        "the WPE runs at the clock its UART is timed for"
    );
}

#[test]
fn every_refusal_is_reachable() {
    let ua = SKY130A
        .user_area
        .expect("sky130's vehicle is characterized");
    let past = CaravelProject {
        macro_origin_um: (200, 80),
        ..WPE_PROJECT
    };
    assert_eq!(
        past.check(&SKY130A),
        Err(CaravelProjectUnplaceable::MacroOutsideUserArea {
            corner_um: (3000, 3440),
            user_area_um: (ua.width_um, ua.height_um)
        })
    );
    let fast_chip = CaravelProject {
        clock_period_ps: 10_000,
        ..DRAM_PROJECT
    };
    assert_eq!(
        fast_chip.check(&SKY130A),
        Err(CaravelProjectUnplaceable::MacroSlowerThanChip {
            macro_period_ns: 20.0,
            chip_period_ns: 10.0
        })
    );
    let standalone = PdkTarget {
        user_area: None,
        ..SKY130A.clone()
    };
    assert_eq!(
        DRAM_PROJECT.check(&standalone),
        Err(CaravelProjectUnplaceable::NoVehicle { pdk: "sky130A" })
    );
    let untimed = PdkTarget {
        user_area: Some(UserArea {
            harness: None,
            ..ua
        }),
        ..SKY130A.clone()
    };
    assert_eq!(
        DRAM_PROJECT.check(&untimed),
        Err(CaravelProjectUnplaceable::VehicleUntimed { pdk: "sky130A" })
    );
    assert!(
        DRAM_PROJECT.flow_shell(&untimed).is_err(),
        "an untimed vehicle gives the scripts nothing to time with"
    );
}

/// Caravel gives a user project two clocks (`pdk_target::HarnessClock`): `wb_clk_i`, the core clock, which also
/// runs the management SoC and the Wishbone bus and which Caravel's own chip signoff times at 25 ns; and
/// `user_clock2`, the PLL's 90-degree phase through a divider of its own (`caravel_clocking.v`, housekeeping's
/// `pll90_sel`). A project on the core clock runs no faster than the chip signs that clock for; a faster one runs on
/// `user_clock2`. The whole-program engine runs at 16 ns, so it is on `user_clock2`, and the same project on the
/// core clock is refused by name. Wherever Caravel is fetched, the core period is its signoff SDC's.
#[test]
fn each_project_runs_on_a_clock_the_chip_is_signed_for() {
    let h = SKY130A
        .user_area
        .unwrap()
        .harness
        .expect("sky130's Caravel harness is characterized");
    assert_eq!(
        h.core_clock_period_ns, 25.0,
        "Caravel's chip signoff times its core clock at 25 ns"
    );
    assert_eq!(
        (HarnessClock::Core.port(), HarnessClock::User2.port()),
        ("wb_clk_i", "user_clock2")
    );
    for c in [HarnessClock::Core, HarnessClock::User2] {
        assert_eq!(HarnessClock::by_port(c.port()), Some(c), "{c:?} round-trips through its port");
    }
    assert_eq!(HarnessClock::by_port("clk"), None);
    assert_eq!(DRAM_PROJECT.clock, HarnessClock::Core);
    assert_eq!(WPE_PROJECT.clock, HarnessClock::User2);
    assert_eq!(
        WPE_PROJECT.clock, WPE_CLOCK,
        "the project runs the engine on the clock its macro is wired to"
    );
    for p in CARAVEL_PROJECTS {
        if p.clock == HarnessClock::Core {
            assert!(
                p.clock_period_ns() >= h.core_clock_period_ns,
                "{}: {} ns on the core clock, which the chip signs off at {} ns",
                p.key,
                p.clock_period_ns(),
                h.core_clock_period_ns
            );
        }
    }
    let on_core = CaravelProject {
        clock: HarnessClock::Core,
        ..WPE_PROJECT
    };
    assert_eq!(
        on_core.check(&SKY130A),
        Err(CaravelProjectUnplaceable::CoreClockPastSignoff {
            period_ns: WPE_PROJECT.clock_period_ns(),
            signoff_ns: h.core_clock_period_ns
        }),
        "the engine's period is a frequency the chip does not sign its core clock for"
    );
    assert!(
        on_core.flow_shell(&SKY130A).is_err(),
        "and the scripts get nothing to harden it with"
    );
    if !common::require_tool("eda-tools") {
        return;
    }
    let chip = std::fs::read_to_string(root().join("tools/caravel/signoff/caravel/caravel.sdc"))
        .expect("the fetched Caravel carries its chip signoff SDC");
    let period: f64 = chip
        .lines()
        .find_map(|l| l.strip_prefix("set clk_period ")?.trim().parse().ok())
        .expect("the chip signoff SDC's clock period");
    assert!(
        chip.contains("create_clock -name clk -period $clk_period [get_ports {clock}]"),
        "the period is the clock pad's, which is the core clock"
    );
    assert_eq!(h.core_clock_period_ns, period, "the chip signoff SDC's core clock");
}

/// The wrapper is timed at every clock the chip runs: `clk` on `wb_clk_i`, at the project's period when the project
/// runs there and at the chip's signoff period when it does not, and for a project on `user_clock2`, `usr_clk` there
/// at the project's period: asynchronous to `clk`, with the scaffold's `usr_*` source latency and transition, created
/// before the propagation and the uncertainty that apply to every clock. The scripts' view carries both periods, and
/// the wrapper harden writes its clocks from them and from nothing else.
#[test]
fn the_wrapper_is_timed_at_every_clock_the_chip_runs() {
    let h = SKY130A.user_area.unwrap().harness.unwrap();
    assert_eq!(DRAM_PROJECT.wrapper_clocks_ns(&h), (25.0, None));
    assert_eq!(WPE_PROJECT.wrapper_clocks_ns(&h), (25.0, Some(21.0)));
    for p in CARAVEL_PROJECTS {
        let (core, user2) = p.wrapper_clocks_ns(&h);
        let sh = p.flow_shell(&SKY130A).unwrap();
        let tail = format!(
            "CW_CORE_PERIOD_NS={core}\nCW_USER_CLOCK2_PERIOD_NS={}\n",
            user2.map_or("none".to_string(), |u| format!("{u}"))
        );
        assert!(sh.ends_with(&tail), "{}: the wrapper's clocks close the view:\n{sh}", p.key);
        match p.clock {
            HarnessClock::Core => {
                assert_eq!((core, user2), (p.clock_period_ns(), None), "{}", p.key)
            }
            HarnessClock::User2 => assert_eq!(
                (core, user2),
                (h.core_clock_period_ns, Some(p.clock_period_ns())),
                "{}",
                p.key
            ),
        }
    }

    let harden = read("scripts/caravel-harden.sh");
    for line in [
        "\"CLOCK_PERIOD\": $CW_CORE_PERIOD_NS,",
        "\"CLOCK_PORT\": \"wb_clk_i\",",
        "\"CLOCK_NET\": \"mprj.wb_clk_i\",",
    ] {
        assert_eq!(harden.matches(line).count(), 1, "the wrapper config sets {line}");
    }
    assert!(
        !harden.contains("$CP_CLOCK_PERIOD_NS"),
        "the wrapper's clocks come from CW_*, never the project's own period"
    );
    let user = harden
        .lines()
        .filter(|l| l.contains("LOGOS_USER_CLOCK2_PERIOD"))
        .collect::<Vec<_>>();
    assert_eq!(user.len(), 1, "one line writes the user clock: {user:?}");
    assert!(
        user[0].contains("\\\"LOGOS_USER_CLOCK2_PERIOD\\\": $CW_USER_CLOCK2_PERIOD_NS,")
            && user[0].contains("[ \"$CW_USER_CLOCK2_PERIOD_NS\" != none ]"),
        "only a project on user_clock2 writes it, at its period: {}",
        user[0]
    );
    let config = &harden[harden.find("cat > \"$D/config.json\" <<EOF\n").expect("the wrapper config")..];
    let config = &config[..config.find("\nEOF\n").expect("its end")];
    assert_eq!(config.matches("\n${USER_CLOCK_LINES}  \"").count(), 1, "the config carries the user clock's line");
    let recorder = read("scripts/record-caravel-signoff.py");
    assert!(
        recorder.contains("user_clock2 = sky._tcl_env(run_dir).get(\"LOGOS_USER_CLOCK2_PERIOD\")")
            && recorder.contains(
                "sig[\"user_clock2_period_ns\"] = float(user_clock2) if user_clock2 is not None else None"
            ),
        "the wrapper ledger records the user clock the run was configured with"
    );

    let sdc = read("benchmarks/silicon/caravel/base_user_project_wrapper.sdc");
    let code: Vec<&str> = sdc
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .collect();
    let at = |line: &str| {
        let hits: Vec<usize> = code
            .iter()
            .enumerate()
            .filter(|(_, l)| **l == line)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(hits.len(), 1, "the wrapper SDC holds {line:?} once");
        hits[0]
    };
    let create = at("create_clock [get_ports user_clock2] -name usr_clk -period $::env(LOGOS_USER_CLOCK2_PERIOD)");
    let groups = at("set_clock_groups -asynchronous -group [get_clocks {clk}] -group [get_clocks {usr_clk}]");
    let latency = at("set_clock_latency -source $usr_clk_max_latency [get_clocks {usr_clk}]");
    let tran = at("set_input_transition $usr_clk_tran [get_ports user_clock2]");
    let guarded = |i: usize| {
        code[..i].iter().rposition(|l| l.ends_with('{')).is_some_and(|open| {
            let close = open + code[open..].iter().position(|l| *l == "}").unwrap();
            code[open] == "if {[info exists ::env(LOGOS_USER_CLOCK2_PERIOD)]} {" && i < close
        })
    };
    for (name, i) in [("the clock", create), ("its groups", groups), ("its latency", latency), ("its transition", tran)] {
        assert!(guarded(i), "{name} is set only for a project on user_clock2 (line {i})");
    }
    for (i, l) in code.iter().enumerate() {
        if (l.contains("usr_clk") || l.contains("user_clock2"))
            && !l.starts_with("set usr_clk_")
            && !l.starts_with("puts ")
        {
            assert!(guarded(i), "every constraint on the user clock sits under its guard: {l}");
        }
    }
    let propagated = at("set_propagated_clock [all_clocks]");
    let uncertainty = at("set_clock_uncertainty $::env(SYNTH_CLOCK_UNCERTAINTY) [all_clocks]");
    let transition = at("set_clock_transition $::env(SYNTH_CLOCK_TRANSITION) [all_clocks]");
    assert!(
        create < propagated && create < uncertainty && create < transition,
        "usr_clk exists before the constraints on every clock are applied"
    );
}

/// A recorded wrapper was timed at its project's clocks: the ledger's `clock_period_ns` is its `clk`'s, and a project
/// on `user_clock2` records that clock's period beside it (`scripts/record-caravel-signoff.py` reads both from the
/// run's own config).
#[test]
fn a_recorded_wrapper_was_timed_at_its_projects_clocks() {
    let h = SKY130A.user_area.unwrap().harness.unwrap();
    for p in CARAVEL_PROJECTS {
        let Ok(raw) = std::fs::read_to_string(root().join(p.wrapper_ledger)) else {
            continue;
        };
        let w = &serde_json::from_str::<serde_json::Value>(&raw).unwrap()["user_project_wrapper"];
        let (core, user2) = p.wrapper_clocks_ns(&h);
        assert_eq!(
            w["clock_period_ns"].as_f64(),
            Some(core),
            "{}: the wrapper's clk was timed at {core} ns",
            p.key
        );
        assert_eq!(
            w["user_clock2_period_ns"].as_f64(),
            user2,
            "{}: the wrapper's user clock",
            p.key
        );
    }
}

/// Every harden of the chain is timed with the clock model the chip is signed off with. The macro carries it in
/// its SDC (`flow_config::vehicle_macro_sdc`); the wrapper's scaffold SDC reads it from `SYNTH_CLOCK_UNCERTAINTY`
/// and `SYNTH_TIMING_DERATE`, which the wrapper harden sets from the vehicle — the same values, from the same
/// place. The scripts' view of a project carries the vehicle's clock after the project's own variables, and the
/// clocks its wrapper is timed at after that.
#[test]
fn every_harden_of_the_chain_is_timed_with_the_chips_signoff_clock() {
    let h = SKY130A
        .user_area
        .unwrap()
        .harness
        .expect("sky130's Caravel harness is characterized");
    for p in CARAVEL_PROJECTS {
        let sh = p
            .flow_shell(&SKY130A)
            .unwrap_or_else(|e| panic!("{}: {e}", p.key));
        let (project, rest) = sh.split_at(p.shell().len());
        assert_eq!(
            project,
            p.shell(),
            "{}: the project's own variables first",
            p.key
        );
        let (vehicle, wrapper) = rest.split_at(h.shell().len());
        assert_eq!(
            vehicle,
            format!(
                "CV_CLOCK_UNCERTAINTY_NS={}\nCV_TIMING_DERATE={}\nCV_CORE_CLOCK_PERIOD_NS={}\n",
                h.clock_uncertainty_ns, h.timing_derate, h.core_clock_period_ns
            ),
            "{}: then the vehicle's signoff clock",
            p.key
        );
        assert!(
            wrapper.starts_with("CW_CORE_PERIOD_NS="),
            "{}: then the clocks its wrapper is timed at:\n{wrapper}",
            p.key
        );
    }
    let emitter = std::fs::read_to_string(
        root().join("crates/logicaffeine_compile/examples/emit_flow_config.rs"),
    )
    .unwrap();
    assert!(
        emitter.contains(".flow_shell(pdk)"),
        "the emitter prints the scripts' view this lock checks"
    );
    let harden = std::fs::read_to_string(root().join("scripts/caravel-harden.sh")).unwrap();
    for line in [
        "\"SYNTH_CLOCK_UNCERTAINTY\": $CV_CLOCK_UNCERTAINTY_NS,",
        "\"SYNTH_TIMING_DERATE\": $CV_TIMING_DERATE,",
    ] {
        assert_eq!(
            harden.matches(line).count(),
            1,
            "the wrapper config sets {line}"
        );
    }
    let sdc = std::fs::read_to_string(
        root().join("benchmarks/silicon/caravel/base_user_project_wrapper.sdc"),
    )
    .unwrap();
    let code: Vec<&str> = sdc
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .collect();
    let uncertainty: Vec<&&str> = code
        .iter()
        .filter(|l| l.starts_with("set_clock_uncertainty"))
        .collect();
    assert_eq!(
        uncertainty,
        [&"set_clock_uncertainty $::env(SYNTH_CLOCK_UNCERTAINTY) [all_clocks]"],
        "the wrapper SDC's only uncertainty is the configured one, on every clock it creates"
    );
    let derate: Vec<&&str> = code
        .iter()
        .filter(|l| l.starts_with("set_timing_derate"))
        .collect();
    assert_eq!(
        derate,
        [
            &"set_timing_derate -early [expr {1-$::env(SYNTH_TIMING_DERATE)}]",
            &"set_timing_derate -late [expr {1+$::env(SYNTH_TIMING_DERATE)}]"
        ],
        "the wrapper SDC's only derate is the configured one"
    );
}

/// A project's met1 route penalty is its own. The command player was hardened on the recipe's 0.7; the engine's
/// 300,507 cells route on met1-met3 under the wrapper's grid, and at 0.7 met1 keeps 20% of its tracks — global
/// routing came up two gcells short, both horizontal (RUN_2026.09.26_05.24.57). The scripts' view carries the
/// penalty and the macro harden passes it on.
#[test]
fn each_project_carries_its_own_met1_penalty() {
    assert_eq!(
        DRAM_PROJECT.macro_met1_penalty_permille, None,
        "the command player keeps the recipe it was signed off with"
    );
    assert_eq!(WPE_PROJECT.macro_met1_penalty_permille, Some(400));
    for p in CARAVEL_PROJECTS {
        let sh = p.shell();
        let v = sh
            .lines()
            .find_map(|l| l.strip_prefix("CP_MET1_PENALTY="))
            .expect("CP_MET1_PENALTY");
        match p.macro_met1_penalty_permille {
            Some(pm) => assert_eq!(v.parse::<f64>().unwrap(), pm as f64 / 1000.0, "{}", p.key),
            None => assert_eq!(v, "default", "{}", p.key),
        }
    }
    let sh = std::fs::read_to_string(root().join("scripts/caravel-macro-harden.sh")).unwrap();
    assert!(
        sh.contains(
            "[ \"$CP_MET1_PENALTY\" = default ] || MET1_ARGS=(--met1-penalty \"$CP_MET1_PENALTY\")"
        ),
        "the macro harden passes a project's penalty and leaves the recipe's otherwise"
    );
    assert!(
        sh.contains("${MET1_ARGS[@]+\"${MET1_ARGS[@]}\"}"),
        "and hands it to the emitter"
    );
}

/// A project's global-route effort is its own: the overflow iterations and the capacity adjustment
/// (`flow_config::ClosureSettings::{grt_overflow_iters, grt_adjustment}`) reach the macro's config only when the
/// project names them. The command player keeps the route it was signed off with. The engine's route needs both: at
/// OpenLane's 0.3 adjustment its global route stopped at GRT-0119 at 50, 150 and 400 iterations alike; at 0.2 with
/// 150 iterations the first route (step 15) was clean, and the route detailed routing takes (step 25) overflowed in
/// 72 gcells by one track each once the fanout step had split 18,096 nets (the private tree's
/// RUN_2026.10.20_17.57.25, x6, met1 penalty 0.3). The scripts' view carries both and the macro harden passes each on.
#[test]
fn each_project_carries_its_own_global_route_effort() {
    assert_eq!(
        (
            DRAM_PROJECT.macro_grt_overflow_iters,
            DRAM_PROJECT.macro_grt_adjustment_permille
        ),
        (None, None),
        "the command player keeps the route it was signed off with"
    );
    assert_eq!(
        (
            WPE_PROJECT.macro_grt_overflow_iters,
            WPE_PROJECT.macro_grt_adjustment_permille
        ),
        (Some(150), Some(50)),
        "the engine routes with 150 iterations, holding back a twentieth of every gcell"
    );
    let chosen = CaravelProject {
        macro_grt_overflow_iters: Some(150),
        macro_grt_adjustment_permille: Some(200),
        ..WPE_PROJECT
    };
    let sh = chosen.shell();
    for want in ["CP_GRT_OVERFLOW_ITERS=150\n", "CP_GRT_ADJUSTMENT=0.2\n"] {
        assert!(sh.contains(want), "a chosen effort reaches the scripts as {want:?}:\n{sh}");
    }
    for p in CARAVEL_PROJECTS.iter().chain([&chosen]) {
        let sh = p.shell();
        let get = |k: &str| {
            sh.lines()
                .find_map(|l| l.strip_prefix(&format!("{k}=")))
                .unwrap_or_else(|| panic!("{}: {k}", p.key))
                .to_string()
        };
        match p.macro_grt_overflow_iters {
            Some(n) => assert_eq!(get("CP_GRT_OVERFLOW_ITERS").parse::<u32>().unwrap(), u32::from(n), "{}", p.key),
            None => assert_eq!(get("CP_GRT_OVERFLOW_ITERS"), "default", "{}", p.key),
        }
        match p.macro_grt_adjustment_permille {
            Some(pm) => {
                assert!(pm < 1000, "{}: an adjustment holds back less than a whole gcell", p.key);
                assert_eq!(get("CP_GRT_ADJUSTMENT").parse::<f64>().unwrap(), pm as f64 / 1000.0, "{}", p.key)
            }
            None => assert_eq!(get("CP_GRT_ADJUSTMENT"), "default", "{}", p.key),
        }
    }
    let sh = std::fs::read_to_string(root().join("scripts/caravel-macro-harden.sh")).unwrap();
    for (var, flag) in [
        ("CP_GRT_OVERFLOW_ITERS", "--grt-overflow-iters"),
        ("CP_GRT_ADJUSTMENT", "--grt-adjustment"),
    ] {
        assert!(
            sh.contains(&format!("[ \"${var}\" = default ] || GRT_ARGS+=({flag} \"${var}\")")),
            "the macro harden passes a project's {flag} and leaves OpenLane's otherwise"
        );
    }
    assert!(
        sh.contains("GRT_ARGS=()") && sh.contains("${GRT_ARGS[@]+\"${GRT_ARGS[@]}\"}"),
        "and hands them to the emitter"
    );
}

/// A project's legalization padding is its own: the sites a side every cell keeps free of its neighbours when
/// placement is legalized (`flow_config::ClosureSettings::dpl_cell_padding_per_side`) reach the macro's config only when
/// the project names them. Abutting cells leave a dense patch's met1 pins no room, and detailed routing then stalls
/// on shorts it cannot rip up. The command player keeps the placement it was signed off with.
#[test]
fn each_project_carries_its_own_legalization_padding() {
    assert_eq!(DRAM_PROJECT.macro_dpl_padding_per_side, None, "the command player keeps the placement it was signed off with");
    assert_eq!(
        WPE_PROJECT.macro_dpl_padding_per_side,
        Some(1),
        "the engine keeps a site free a side: abutting, its met1 shorts stalled detailed routing at 74"
    );
    let chosen = CaravelProject { macro_dpl_padding_per_side: Some(2), ..WPE_PROJECT };
    assert!(chosen.shell().contains("CP_DPL_PADDING=2\n"), "a chosen padding reaches the scripts:\n{}", chosen.shell());
    for p in CARAVEL_PROJECTS.iter().chain([&chosen]) {
        let sh = p.shell();
        let got = sh.lines().find_map(|l| l.strip_prefix("CP_DPL_PADDING=")).unwrap_or_else(|| panic!("{}: CP_DPL_PADDING", p.key));
        match p.macro_dpl_padding_per_side {
            Some(n) => assert_eq!(got.parse::<u8>().unwrap(), n, "{}", p.key),
            None => assert_eq!(got, "default", "{}", p.key),
        }
    }
    let harden = std::fs::read_to_string(root().join("scripts/caravel-macro-harden.sh")).unwrap();
    assert!(
        harden.contains("[ \"$CP_DPL_PADDING\" = default ] || GRT_ARGS+=(--dpl-padding-per-side \"$CP_DPL_PADDING\")"),
        "the macro harden passes a project's padding and leaves the emitter's otherwise"
    );
}

/// A project's heuristic diode pass is its own: the span past which a net takes a protection diode before routing
/// (`flow_config::ClosureSettings::heuristic_antenna_threshold`, OpenLane's `HEURISTIC_ANTENNA_THRESHOLD`) reaches the
/// macro's config only when the project names one. A low threshold floods a dense patch with diodes that take the
/// sites and the tracks its routes need. The command player keeps the pass it was signed off with.
#[test]
fn each_project_carries_its_own_diode_threshold() {
    assert_eq!(DRAM_PROJECT.macro_antenna_threshold_um, None, "the command player keeps the pass it was signed off with");
    assert_eq!(
        WPE_PROJECT.macro_antenna_threshold_um,
        Some(700),
        "the engine diodes nets past 700 um: at 300 the pass's 57,637 diodes were a third of its congested patch"
    );
    let chosen = CaravelProject { macro_antenna_threshold_um: Some(500), ..WPE_PROJECT };
    assert!(chosen.shell().contains("CP_ANTENNA_THRESHOLD=500\n"), "a chosen threshold reaches the scripts:\n{}", chosen.shell());
    for p in CARAVEL_PROJECTS.iter().chain([&chosen]) {
        let sh = p.shell();
        let got = sh
            .lines()
            .find_map(|l| l.strip_prefix("CP_ANTENNA_THRESHOLD="))
            .unwrap_or_else(|| panic!("{}: CP_ANTENNA_THRESHOLD", p.key));
        match p.macro_antenna_threshold_um {
            Some(um) => assert_eq!(got.parse::<u16>().unwrap(), um, "{}", p.key),
            None => assert_eq!(got, "default", "{}", p.key),
        }
    }
    let harden = std::fs::read_to_string(root().join("scripts/caravel-macro-harden.sh")).unwrap();
    assert!(
        harden.contains("[ \"$CP_ANTENNA_THRESHOLD\" = default ] || GRT_ARGS+=(--antenna-threshold \"$CP_ANTENNA_THRESHOLD\")"),
        "the macro harden passes a project's threshold and leaves the emitter's otherwise"
    );
}

/// A project's antenna repair is its own: how far past each antenna ratio the global router repairs
/// (`flow_config::ClosureSettings::grt_antenna_margin_pct`, OpenLane's `GRT_ANT_MARGIN`) and how many passes it makes
/// (`grt_antenna_iters`, `GRT_ANT_ITERS`) reach the macro's config only when the project names them. Each pass inserts
/// diodes and reroutes the nets it touched, and a margin that flags more nets, or a pass more, inserts diodes where the
/// route is already full. The command player keeps the repair it was signed off with.
#[test]
fn each_project_carries_its_own_antenna_repair() {
    assert_eq!(
        (DRAM_PROJECT.macro_grt_antenna_margin_pct, DRAM_PROJECT.macro_grt_antenna_iters),
        (None, None),
        "the command player keeps the repair it was signed off with"
    );
    assert_eq!(
        WPE_PROJECT.macro_grt_antenna_margin_pct,
        Some(20),
        "the engine repairs to a fifth past each ratio: at half, the first pass's 5,981 diodes left its reroute no room"
    );
    assert_eq!(
        WPE_PROJECT.macro_grt_antenna_iters,
        Some(3),
        "the engine repairs in three passes: from the fourth the count oscillates while each reroute slows"
    );
    let chosen = CaravelProject { macro_grt_antenna_margin_pct: Some(20), macro_grt_antenna_iters: Some(3), ..WPE_PROJECT };
    let sh = chosen.shell();
    for want in ["CP_GRT_ANT_MARGIN=20\n", "CP_GRT_ANT_ITERS=3\n"] {
        assert!(sh.contains(want), "a chosen repair reaches the scripts as {want:?}:\n{sh}");
    }
    for p in CARAVEL_PROJECTS.iter().chain([&chosen]) {
        let sh = p.shell();
        let get = |k: &str| {
            sh.lines()
                .find_map(|l| l.strip_prefix(&format!("{k}=")))
                .unwrap_or_else(|| panic!("{}: {k}", p.key))
                .to_string()
        };
        match p.macro_grt_antenna_margin_pct {
            Some(pct) => {
                assert!(pct <= 100, "{}: a margin is a share of the ratio", p.key);
                assert_eq!(get("CP_GRT_ANT_MARGIN").parse::<u8>().unwrap(), pct, "{}", p.key)
            }
            None => assert_eq!(get("CP_GRT_ANT_MARGIN"), "default", "{}", p.key),
        }
        match p.macro_grt_antenna_iters {
            Some(n) => {
                assert!(n >= 1, "{}: a repair makes at least one pass", p.key);
                assert_eq!(get("CP_GRT_ANT_ITERS").parse::<u8>().unwrap(), n, "{}", p.key)
            }
            None => assert_eq!(get("CP_GRT_ANT_ITERS"), "default", "{}", p.key),
        }
    }
    let harden = std::fs::read_to_string(root().join("scripts/caravel-macro-harden.sh")).unwrap();
    for (var, flag) in [("CP_GRT_ANT_MARGIN", "--grt-antenna-margin"), ("CP_GRT_ANT_ITERS", "--grt-antenna-iters")] {
        assert!(
            harden.contains(&format!("[ \"${var}\" = default ] || GRT_ARGS+=({flag} \"${var}\")")),
            "the macro harden passes a project's {flag} and leaves the emitter's otherwise"
        );
    }
}

/// A project's setup repair margin is its own: the slack the placement and global-route timing repairs close every
/// path to (`flow_config::ClosureSettings::setup_slack_margin_ns`, OpenLane's `PL_`/`GLB_RESIZER_SETUP_SLACK_MARGIN`)
/// reaches the macro's config only when the project names it. The repairs judge setup on global-route estimates at the
/// nominal RC corner and stop once those close; signoff times the routed design extracted at the max-RC corner. The
/// command player keeps the repair it was signed off with.
#[test]
fn each_project_carries_its_own_setup_margin() {
    assert_eq!(DRAM_PROJECT.macro_setup_margin_ps, None, "the command player keeps the repair it was signed off with");
    assert_eq!(
        WPE_PROJECT.macro_setup_margin_ps,
        Some(1250),
        "the engine closes setup 1.25 ns past its estimate: routed, it read 1.16 ns slower at the max-RC corner"
    );
    let chosen = CaravelProject { macro_setup_margin_ps: Some(500), ..WPE_PROJECT };
    assert!(chosen.shell().contains("CP_SETUP_MARGIN=0.5\n"), "a chosen margin reaches the scripts in ns:\n{}", chosen.shell());
    for p in CARAVEL_PROJECTS.iter().chain([&chosen]) {
        let sh = p.shell();
        let got = sh.lines().find_map(|l| l.strip_prefix("CP_SETUP_MARGIN=")).unwrap_or_else(|| panic!("{}: CP_SETUP_MARGIN", p.key));
        match p.macro_setup_margin_ps {
            Some(ps) => assert_eq!(got.parse::<f64>().unwrap(), ps as f64 / 1000.0, "{}", p.key),
            None => assert_eq!(got, "default", "{}", p.key),
        }
    }
    let harden = std::fs::read_to_string(root().join("scripts/caravel-macro-harden.sh")).unwrap();
    assert!(
        harden.contains("[ \"$CP_SETUP_MARGIN\" = default ] || GRT_ARGS+=(--setup-slack-margin \"$CP_SETUP_MARGIN\")"),
        "the macro harden passes a project's setup margin and leaves OpenLane's otherwise"
    );
}

/// A project's slew-detection margin is its own, and never looser than the all-corner 20% that closed
/// `systolic_pipe16_realbist` and `systolic_os_pipe8_realbist`. The engine needs more: on its RUN_2026.09.27_18.14.14 the
/// upsize detected at 0.600 ns and converged to zero, yet net `_117618_` (773 um, driven by `a21o_4`, the top of its
/// family) signed off at 0.757 ns at max RC — its global-route estimate read 0.635 on a re-route of the same netlist.
/// Over the whole engine, every pin signing off above 0.70 ns estimates at 0.56 ns or more; 28% (0.540 ns) catches all
/// of them with 0.05 ns to spare for that route-to-route noise, flagging 1,752 pins where 20% flagged 21.
#[test]
fn each_project_detects_slew_at_its_own_measured_margin() {
    assert_eq!(
        DRAM_PROJECT.macro_upsize_margin_pct, 20,
        "the command player keeps the all-corner margin"
    );
    assert_eq!(
        WPE_PROJECT.macro_upsize_margin_pct, 28,
        "the engine's measured margin"
    );
    for p in CARAVEL_PROJECTS {
        assert!(
            p.macro_upsize_margin_pct >= 20,
            "{}: detection looser than the all-corner floor",
            p.key
        );
        assert!(
            p.macro_upsize_margin_pct < 100,
            "{}: a margin of 100% detects every pin",
            p.key
        );
        let sh = p.shell();
        let v = sh
            .lines()
            .find_map(|l| l.strip_prefix("CP_UPSIZE_MARGIN="))
            .expect("CP_UPSIZE_MARGIN");
        assert_eq!(
            v.parse::<u8>().unwrap(),
            p.macro_upsize_margin_pct,
            "{}",
            p.key
        );
    }
    let sh = std::fs::read_to_string(root().join("scripts/caravel-macro-harden.sh")).unwrap();
    assert!(
        sh.contains("--upsize-margin \"$CP_UPSIZE_MARGIN\""),
        "the macro harden passes the project's margin"
    );
    assert!(
        !sh.contains("--upsize-margin 20"),
        "and no longer a literal that overrides it"
    );
}

/// Each macro takes power the way its evidence was hardened. The command player carries its own grid — the run it
/// is signed off on has `DESIGN_IS_CORE` 1 and routes to met3; the engine is fed by the wrapper and routes to
/// met4. Where the signed-off run is on disk, its own config says so; the scripts' view carries the grid, and the
/// macro harden passes it to the emitter.
#[test]
fn each_macro_takes_power_the_way_its_evidence_was_hardened() {
    assert_eq!(DRAM_PROJECT.macro_grid, MacroGrid::Own);
    assert_eq!(WPE_PROJECT.macro_grid, MacroGrid::WrapperFed);
    let ua = SKY130A.user_area.unwrap();
    let eda_tools = common::require_tool("eda-tools");
    for p in CARAVEL_PROJECTS {
        let sh = p.shell();
        let flag = sh
            .lines()
            .find_map(|l| l.strip_prefix("CP_MACRO_GRID="))
            .expect("CP_MACRO_GRID");
        assert_eq!(
            MacroGrid::parse(flag),
            Some(p.macro_grid),
            "{}: the grid round-trips",
            p.key
        );
        if !eda_tools {
            continue;
        }
        let Some(block) = SIGNOFF_BLOCKS.iter().find(|b| b.name == p.macro_module) else {
            continue;
        };
        let run = block.basis.rsplit(", ").next().unwrap();
        let cfg = root()
            .join("tools/OpenLane/designs")
            .join(p.macro_design)
            .join("runs")
            .join(run)
            .join("config.tcl");
        let Ok(cfg) = std::fs::read_to_string(&cfg) else {
            continue;
        };
        let get = |k: &str| {
            cfg.lines()
                .find_map(|l| {
                    l.strip_prefix(&format!("set ::env({k}) \""))
                        .map(|r| r.trim_end_matches('"').to_string())
                })
                .unwrap_or_else(|| panic!("{}: {run} has no {k}", p.key))
        };
        let (core, top) = match p.macro_grid {
            MacroGrid::Own => ("1", ua.macro_max_layer.unwrap()),
            MacroGrid::WrapperFed => ("0", ua.fed_macro_max_layer.unwrap()),
        };
        assert_eq!(
            get("DESIGN_IS_CORE"),
            core,
            "{}: the signed-off run's grid",
            p.key
        );
        assert_eq!(
            get("RT_MAX_LAYER"),
            top,
            "{}: the signed-off run's routing ceiling",
            p.key
        );
    }
    let sh = std::fs::read_to_string(root().join("scripts/caravel-macro-harden.sh")).unwrap();
    assert!(
        sh.contains("--macro-grid \"$CP_MACRO_GRID\""),
        "the macro harden passes its project's grid"
    );
}

/// The wrapper-level special-net geometry over a rectangle of the user area, from a DEF: per net, the vertical
/// straps on `strap` crossing the rectangle and the via stacks on `strap` inside it.
fn grid_over(
    def: &str,
    strap: &str,
    rect: (f64, f64, f64, f64),
) -> std::collections::BTreeMap<String, (usize, usize)> {
    let dbu: f64 = def
        .lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("UNITS DISTANCE MICRONS ")?
                .trim_end_matches(" ;")
                .trim()
                .parse()
                .ok()
        })
        .expect("DEF units");
    let (x0, y0, x1, y1) = rect;
    let body = &def
        [def.find("\nSPECIALNETS").expect("SPECIALNETS")..def.find("\nEND SPECIALNETS").unwrap()];
    let mut out = std::collections::BTreeMap::new();
    let mut net = String::new();
    for l in body.lines() {
        if let Some(rest) = l.strip_prefix("    - ") {
            net = rest.split_whitespace().next().unwrap().to_string();
        }
        let Some(rest) = l
            .split(&format!("{strap} "))
            .nth(1)
            .filter(|r| r.contains("+ SHAPE STRIPE ( "))
        else {
            continue;
        };
        let width: u64 = rest.split_whitespace().next().unwrap().parse().unwrap_or(0);
        let points: Vec<Vec<&str>> = rest
            .split('(')
            .skip(1)
            .map(|g| {
                g.split(')')
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .collect::<Vec<_>>()
            })
            .filter(|p| p.len() == 2)
            .collect();
        let coord = |s: &str, prev: f64| {
            if s == "*" {
                prev
            } else {
                s.parse::<f64>().unwrap() / dbu
            }
        };
        let entry = out.entry(net.clone()).or_insert((0, 0));
        match points.as_slice() {
            [a, b] if width > 0 => {
                let (ax, ay) = (coord(a[0], f64::NAN), coord(a[1], f64::NAN));
                let (bx, by) = (coord(b[0], ax), coord(b[1], ay));
                if ax == bx && ax > x0 && ax < x1 && ay.min(by) < y1 && ay.max(by) > y0 {
                    entry.0 += 1;
                }
            }
            [a] if width == 0 => {
                let (x, y) = (coord(a[0], f64::NAN), coord(a[1], f64::NAN));
                if x > x0 && x < x1 && y > y0 && y < y1 {
                    entry.1 += 1;
                }
            }
            _ => {}
        }
    }
    out
}

/// The vertical straps on `layer` in a DEF's special nets, as `(net, x0, y0, x1, y1)` rectangles of their drawn width.
fn vertical_straps(def: &str, layer: &str) -> Vec<(String, (f64, f64, f64, f64))> {
    let dbu: f64 = def
        .lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("UNITS DISTANCE MICRONS ")?
                .trim_end_matches(" ;")
                .trim()
                .parse()
                .ok()
        })
        .expect("DEF units");
    let body = &def
        [def.find("\nSPECIALNETS").expect("SPECIALNETS")..def.find("\nEND SPECIALNETS").unwrap()];
    let mut out = Vec::new();
    let mut net = String::new();
    for l in body.lines() {
        if let Some(rest) = l.strip_prefix("    - ") {
            net = rest.split_whitespace().next().unwrap().to_string();
        }
        let Some(rest) = l
            .split(&format!("{layer} "))
            .nth(1)
            .filter(|r| r.contains("+ SHAPE STRIPE ( "))
        else {
            continue;
        };
        let width = rest
            .split_whitespace()
            .next()
            .unwrap()
            .parse::<f64>()
            .unwrap_or(0.0)
            / dbu;
        let points: Vec<Vec<&str>> = rest
            .split('(')
            .skip(1)
            .map(|g| {
                g.split(')')
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .collect::<Vec<_>>()
            })
            .filter(|p| p.len() == 2)
            .collect();
        let coord = |s: &str, prev: f64| {
            if s == "*" {
                prev
            } else {
                s.parse::<f64>().unwrap() / dbu
            }
        };
        if let [a, b] = points.as_slice() {
            let (ax, ay) = (coord(a[0], f64::NAN), coord(a[1], f64::NAN));
            let (bx, by) = (coord(b[0], ax), coord(b[1], ay));
            if width > 0.0 && ax == bx {
                out.push((
                    net.clone(),
                    (ax - width / 2.0, ay.min(by), ax + width / 2.0, ay.max(by)),
                ));
            }
        }
    }
    out
}

/// The obstruction rectangles a macro's abstract declares on `layer`, in the macro's own coordinates.
fn lef_obstruction(lef: &str, layer: &str) -> Vec<(f64, f64, f64, f64)> {
    let Some(at) = lef.find("\n  OBS") else {
        return Vec::new();
    };
    let obs = &lef[at..];
    let obs = &obs[..obs.find("\n  END").unwrap_or(obs.len())];
    let mut on = false;
    let mut out = Vec::new();
    for l in obs.lines() {
        let t = l.trim();
        if let Some(name) = t.strip_prefix("LAYER ") {
            on = name.trim_end_matches(" ;").trim() == layer;
        } else if let Some(r) = t.strip_prefix("RECT ").filter(|_| on) {
            let v: Vec<f64> = r
                .trim_end_matches(" ;")
                .split_whitespace()
                .filter_map(|n| n.parse().ok())
                .collect();
            if let [a, b, c, d] = v[..] {
                out.push((a, b, c, d));
            }
        }
    }
    out
}

fn overlaps(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> bool {
    a.0 < b.2 && a.2 > b.0 && a.1 < b.3 && a.3 > b.1
}

#[test]
fn the_wrapper_grid_reader_counts_what_crosses_and_what_feeds() {
    let def = r#"UNITS DISTANCE MICRONS 1000 ;
SPECIALNETS 2 ;
    - vccd1 ( PIN vccd1 ) + USE POWER
      + ROUTED met4 3100 + SHAPE STRIPE ( 500000 -38270 ) ( 500000 3557950 )
      NEW met4 3100 + SHAPE STRIPE ( 20000 -38270 ) ( * 3557950 )
      NEW met4 3100 + SHAPE STRIPE ( 700000 2000000 ) ( 700000 3557950 )
      NEW met5 3100 + SHAPE STRIPE ( -43630 1000000 ) ( 2963250 1000000 )
      NEW met4 0 + SHAPE STRIPE ( 900000 1000000 ) via5_6_1600_3100_2_1_1600_1600
      NEW met4 0 + SHAPE STRIPE ( 20000 1000000 ) via5_6_1600_3100_2_1_1600_1600 ;
    - vssd1 ( PIN vssd1 ) + USE GROUND
      + ROUTED met4 3100 + SHAPE STRIPE ( 600000 3000000 ) ( * 3557950 ) ;
END SPECIALNETS
"#;
    let g = grid_over(def, "met4", (60.0, 80.0, 1000.0, 1800.0));
    assert_eq!(
        g["vccd1"],
        (1, 1),
        "the strap at x=500 crosses; x=20 lies outside, x=700 starts above; one via inside"
    );
    assert_eq!(
        g["vssd1"],
        (0, 0),
        "a strap above the macro does not cross it"
    );
    assert!(
        grid_over(def, "met5", (60.0, 80.0, 1000.0, 1800.0))["vccd1"].0 == 0,
        "a horizontal strap is not a vertical crossing"
    );

    let lef = "MACRO m\n  SIZE 900 BY 1700 ;\n  OBS\n      LAYER met3 ;\n        RECT 0 0 900 1700 ;\n      LAYER met4 ;\n        RECT 400.0 25.0 460.0 1690.0 ;\n  END\nEND m\n";
    let obs = lef_obstruction(lef, "met4");
    assert_eq!(
        obs,
        vec![(400.0, 25.0, 460.0, 1690.0)],
        "only the met4 rectangles, in macro coordinates"
    );
    let placed: Vec<_> = obs
        .iter()
        .map(|&(a, b, c, d)| (a + 60.0, b + 80.0, c + 60.0, d + 80.0))
        .collect();
    let straps = vertical_straps(def, "met4");
    let hit: Vec<&String> = straps
        .iter()
        .filter(|(_, r)| placed.iter().any(|o| overlaps(*r, *o)))
        .map(|(n, _)| n)
        .collect();
    assert_eq!(
        hit,
        vec!["vccd1"],
        "the x=500 strap runs through the placed obstruction at x 460-520; the others clear it"
    );
    let stub = "UNITS DISTANCE MICRONS 1000 ;\nSPECIALNETS 1 ;\n    - vccd1 ( PIN vccd1 ) + USE POWER\n      + ROUTED met4 3100 + SHAPE STRIPE ( 500000 -38270 ) ( 500000 94640 ) ;\nEND SPECIALNETS\n";
    assert!(
        vertical_straps(stub, "met4")
            .iter()
            .all(|(_, r)| !placed.iter().any(|o| overlaps(*r, *o))),
        "a strap that stops below the obstruction, inside the macro's outline, clears it"
    );
}

/// The wrapper around each recorded macro built the grid the macro takes. Around a macro with its own grid the
/// wrapper's met4 straps run over it, every supply; around a wrapper-fed macro none touches the met4 the macro's
/// abstract obstructs — they stop at its routing, which lies inside its outline — and met5 feeds its supplies
/// through via stacks inside it. Read from the final DEF of the wrapper run the ledger records — found by
/// its GDS bytes, as the submission assembler finds it — wherever that run is on disk.
#[test]
fn each_recorded_wrapper_built_the_grid_its_macro_takes() {
    if !common::require_tool("eda-tools") {
        return;
    }
    let ua = SKY130A.user_area.unwrap();
    let (vdd, gnd) = ua.macro_power.unwrap();
    for p in CARAVEL_PROJECTS {
        let Ok(ledger) = std::fs::read_to_string(root().join(p.wrapper_ledger)) else {
            continue;
        };
        let ledger: serde_json::Value = serde_json::from_str(&ledger).unwrap();
        let Some(bytes) = ledger["user_project_wrapper"]["gds_bytes"].as_u64() else {
            continue;
        };
        let runs = root()
            .join("tools/OpenLane/designs")
            .join(p.wrapper_design)
            .join("runs");
        let Ok(entries) = std::fs::read_dir(&runs) else {
            continue;
        };
        let Some(run) = entries.flatten().map(|e| e.path()).find(|r| {
            std::fs::metadata(r.join("results/final/gds/user_project_wrapper.gds"))
                .is_ok_and(|m| m.len() == bytes)
        }) else {
            continue;
        };
        let def = std::fs::read_to_string(run.join("results/final/def/user_project_wrapper.def"))
            .unwrap();
        let (x, y) = (p.macro_origin_um.0 as f64, p.macro_origin_um.1 as f64);
        let rect = (
            x,
            y,
            x + p.macro_die_um.0 as f64,
            y + p.macro_die_um.1 as f64,
        );
        let met4 = grid_over(&def, "met4", rect);
        let crossing: usize = met4.values().map(|(s, _)| s).sum();
        match p.macro_grid {
            MacroGrid::Own => assert!(
                crossing > 0,
                "{}: the wrapper's met4 grid passes over a macro with its own",
                p.key
            ),
            MacroGrid::WrapperFed => {
                let lef = root()
                    .join("tools/OpenLane/designs")
                    .join(p.wrapper_design)
                    .join("macro")
                    .join(format!("{}.lef", p.macro_module));
                let lef = std::fs::read_to_string(&lef)
                    .unwrap_or_else(|e| panic!("{}: {}: {e}", p.key, lef.display()));
                let obs: Vec<_> = lef_obstruction(&lef, "met4")
                    .into_iter()
                    .map(|(a, b, c, d)| (a + x, b + y, c + x, d + y))
                    .collect();
                assert!(
                    !obs.is_empty(),
                    "{}: a wrapper-fed macro routes met4, so its abstract obstructs met4",
                    p.key
                );
                let through: Vec<_> = vertical_straps(&def, "met4")
                    .into_iter()
                    .filter(|(_, r)| obs.iter().any(|o| overlaps(*r, *o)))
                    .collect();
                assert!(
                    through.is_empty(),
                    "{}: wrapper met4 straps through the macro's met4: {through:?}",
                    p.key
                );
                for net in [vdd, gnd] {
                    let fed = met4.get(net).map_or(0, |(_, v)| *v);
                    assert!(
                        fed > 0,
                        "{}: no met4 via stack feeds {net} inside the macro",
                        p.key
                    );
                }
            }
        }
    }
}

// ── 3: the scripts ─────────────────────────────────────────────────────────────────────────────────────────

const SHELL_VARS: [&str; 31] = [
    "CP_KEY",
    "CP_MACRO",
    "CP_MACRO_DIE_W",
    "CP_MACRO_DIE_H",
    "CP_MACRO_X",
    "CP_MACRO_Y",
    "CP_CLOCK_PORT",
    "CP_CLOCK_PERIOD_NS",
    "CP_MACRO_PERIOD_NS",
    "CP_SYNTH_STRATEGY",
    "CP_HOLD_MARGIN_NS",
    "CP_DENSITY",
    "CP_MET1_PENALTY",
    "CP_GRT_OVERFLOW_ITERS",
    "CP_GRT_ADJUSTMENT",
    "CP_GRT_ANT_MARGIN",
    "CP_GRT_ANT_ITERS",
    "CP_ANTENNA_THRESHOLD",
    "CP_DPL_PADDING",
    "CP_SETUP_MARGIN",
    "CP_UPSIZE_MARGIN",
    "CP_MACRO_GRID",
    "CP_MACRO_DESIGN",
    "CP_WRAPPER_DESIGN",
    "CP_WRAPPER_RTL",
    "CP_MACRO_LEDGER",
    "CP_WRAPPER_LEDGER",
    "CP_GOLDEN_KEY",
    "CP_PRECHECK_LEDGER",
    "CP_SUBMISSION",
    "CP_OVERLAY",
];

#[test]
fn the_scripts_view_of_a_project_is_complete_and_bare() {
    for p in CARAVEL_PROJECTS {
        let sh = p.shell();
        let vars: Vec<(&str, &str)> = sh
            .lines()
            .map(|l| l.split_once('=').expect("NAME=value"))
            .collect();
        assert_eq!(
            vars.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            SHELL_VARS,
            "{}: the variables the scripts read",
            p.key
        );
        for (k, v) in &vars {
            assert!(
                !v.is_empty()
                    && v.chars()
                        .all(|c| c.is_ascii_alphanumeric() || "_./-".contains(c)),
                "{}: {k}={v:?}",
                p.key
            );
        }
        let get = |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| *v).unwrap();
        assert_eq!(get("CP_MACRO"), p.macro_module);
        assert_eq!(get("CP_CLOCK_PORT"), p.clock.port());
        assert_eq!(
            get("CP_MACRO_PERIOD_NS").parse::<f64>().unwrap(),
            p.macro_period_ns()
        );
        assert_eq!(
            get("CP_CLOCK_PERIOD_NS").parse::<f64>().unwrap(),
            p.clock_period_ns()
        );
        assert_eq!(
            get("CP_HOLD_MARGIN_NS").parse::<f64>().unwrap(),
            p.macro_hold_margin_ps as f64 / 1000.0
        );
        match p.macro_density_permille {
            Some(d) => assert_eq!(get("CP_DENSITY").parse::<f64>().unwrap(), d as f64 / 1000.0),
            None => assert_eq!(get("CP_DENSITY"), "default"),
        }
        match p.synth_strategy {
            Some(st) => assert_eq!(
                SynthStrategy::parse(get("CP_SYNTH_STRATEGY")),
                Some(st),
                "{}: the strategy round-trips",
                p.key
            ),
            None => assert_eq!(
                get("CP_SYNTH_STRATEGY"),
                "default",
                "{}: no strategy is OpenLane's default",
                p.key
            ),
        }
        assert_eq!(
            (get("CP_MACRO_X"), get("CP_MACRO_Y")),
            (
                &*p.macro_origin_um.0.to_string(),
                &*p.macro_origin_um.1.to_string()
            )
        );
    }
}

/// Each script of the chain takes its project from the registry and, outside its comments, names no macro,
/// design directory or ledger of its own.
#[test]
fn every_script_of_the_chain_takes_the_project_from_the_registry() {
    for script in [
        "caravel-macro-harden.sh",
        "caravel-harden.sh",
        "caravel-submission.sh",
        "run-mpw-precheck.sh",
    ] {
        let sh = read(&format!("scripts/{script}"));
        assert!(
            sh.contains("--caravel-project"),
            "{script} must read its project from emit_flow_config --caravel-project"
        );
        let code: Vec<&str> = sh
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .collect();
        for p in CARAVEL_PROJECTS {
            for name in [
                p.macro_module,
                p.macro_design,
                p.wrapper_design,
                p.wrapper_ledger,
                p.precheck_ledger,
                p.submission_dir,
            ] {
                let hits: Vec<&&str> = code.iter().filter(|l| l.contains(name)).collect();
                assert!(
                    hits.is_empty(),
                    "{script} names {name} itself:\n{}",
                    hits.iter()
                        .map(|l| format!("  {l}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
            }
        }
    }
    let golden = read("scripts/caravel-harden.sh");
    assert!(
        golden.contains("run-golden-precheck-drc.sh\" user_project_wrapper \"$GDS\" \"$CP_GOLDEN_KEY\""),
        "the wrapper's golden verdict is filed under its project's key, never under the shared top-cell name"
    );
}

// ── 4: RTL and overlays ────────────────────────────────────────────────────────────────────────────────────

#[test]
fn every_wrapper_rtl_is_its_emitter_output() {
    for p in CARAVEL_PROJECTS {
        let rel = format!("benchmarks/silicon/{}", p.wrapper_rtl);
        let w = p.wrapper_verilog();
        assert_eq!(read(&rel), w, "{rel} is stale");
        assert_eq!(
            w.matches(&format!("  {} mprj (", p.macro_module)).count(),
            1,
            "{}: the macro is `mprj`, once",
            p.key
        );
        assert!(
            w.contains("module user_project_wrapper"),
            "{}: the harness's top-cell name",
            p.key
        );
    }
    assert_eq!(
        DRAM_PROJECT.wrapper_verilog(),
        emit_user_project_wrapper_verilog(),
        "the DRAM wrapper is the emitter's"
    );
}

#[test]
fn every_overlay_carries_what_a_submission_needs() {
    for p in CARAVEL_PROJECTS {
        let o = |rel: &str| read(&format!("{}/{rel}", p.overlay));
        for rel in [
            "README.md",
            "verilog/rtl/user_defines.v",
            "verilog/dv/logos_bringup/README.md",
            "verilog/dv/logos_bringup/tb_user_project_wrapper.v",
        ] {
            assert!(
                o(rel).contains("SPDX-License-Identifier: MIT"),
                "{}/{rel} carries no MIT SPDX header",
                p.overlay
            );
        }
        assert!(
            o("verilog/rtl/user_defines.v").contains("SPDX-FileCopyrightText: 2022 Efabless Corporation"),
            "{}: user_defines.v carries the scaffold's GPIO mode table without the scaffold's copyright line",
            p.key
        );
        assert!(
            o("README.md").contains(&format!("`{}`", p.macro_module)),
            "{}: the README names the macro",
            p.key
        );
        assert!(
            o("verilog/dv/logos_bringup/README.md").contains(&format!("`{}`", p.macro_module)),
            "{}: the bring-up README names the macro",
            p.key
        );
        assert!(
            o("verilog/dv/logos_bringup/tb_user_project_wrapper.v")
                .contains("user_project_wrapper dut"),
            "{}: the bench drives the top cell",
            p.key
        );
        let defines = o("verilog/rtl/user_defines.v");
        for gpio in 5..38 {
            let line = defines
                .lines()
                .find(|l| l.starts_with(&format!("`define USER_CONFIG_GPIO_{gpio}_INIT ")))
                .unwrap_or_else(|| panic!("{}: GPIO {gpio} has no power-on mode", p.key));
            assert!(
                !line.contains("INVALID"),
                "{}: GPIO {gpio} left INVALID",
                p.key
            );
        }
        for readme in ["README.md", "verilog/dv/logos_bringup/README.md"] {
            assert!(
                !o(readme).contains("${"),
                "{}/{readme}: an unexpanded shell expression",
                p.overlay
            );
        }
    }
}

// ── 5: golden-deck binding ─────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_recorded_wrappers_golden_verdict_is_filed_under_its_key_on_its_gds() {
    let golden: serde_json::Value =
        serde_json::from_str(&read("benchmarks/silicon/golden_precheck.json")).unwrap();
    let mut bound = 0;
    for p in CARAVEL_PROJECTS {
        let Ok(raw) = std::fs::read_to_string(root().join(p.wrapper_ledger)) else {
            continue;
        };
        let w = &serde_json::from_str::<serde_json::Value>(&raw).unwrap()["user_project_wrapper"];
        let g = &golden["blocks"][p.golden_key];
        assert!(
            !g.is_null(),
            "{}: a recorded wrapper with no golden verdict under {}",
            p.key,
            p.golden_key
        );
        assert_eq!(
            g["gds_bytes"], w["gds_bytes"],
            "{}: the golden deck checked a different GDS than the wrapper ledger ships",
            p.key
        );
        assert_eq!(
            g["golden_drc_violations"], w["golden_drc_violations"],
            "{}: the two records of one deck run disagree",
            p.key
        );
        bound += 1;
    }
    assert!(
        bound >= 1,
        "no project's wrapper is recorded — the binding would pass vacuously"
    );
}

// ── 6: assembled submissions ───────────────────────────────────────────────────────────────────────────────

fn sha256(path: &Path) -> String {
    let out = std::process::Command::new("sha256sum")
        .arg(path)
        .output()
        .expect("run sha256sum");
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string()
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d)
            .unwrap_or_else(|e| panic!("read {}: {e}", d.display()))
            .flatten()
        {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// A project's submission directory is gitignored, so a clean checkout has none to inspect; where one is
/// assembled, it must be exactly the recorded pair and the repo's overlay.
#[test]
fn an_assembled_submission_is_the_recorded_pair_and_the_overlay() {
    if !common::require_tool("eda-tools") {
        return;
    }
    for p in CARAVEL_PROJECTS {
        let sub = root().join(p.submission_dir);
        if !sub.is_dir() {
            continue;
        }
        let led: serde_json::Value = serde_json::from_str(&read(p.wrapper_ledger)).unwrap();
        let w = &led["user_project_wrapper"];
        let size = |rel: &str| {
            std::fs::metadata(sub.join(rel))
                .map(|m| m.len() as i64)
                .unwrap_or(-1)
        };
        assert_eq!(
            Some(size("gds/user_project_wrapper.gds")),
            w["gds_bytes"].as_i64(),
            "{}: the wrapper GDS is not the recorded one",
            p.key
        );
        assert_eq!(
            Some(size(&format!("gds/{}.gds", p.macro_module))),
            w["macro_gds_bytes"].as_i64(),
            "{}: the macro GDS is not the one the wrapper embeds",
            p.key
        );

        let rtl = root().join(format!("benchmarks/silicon/{}.v", p.macro_module));
        let shipped = std::fs::read_to_string(sub.join("logos_rtl.sha256")).unwrap_or_default();
        let recorded = shipped
            .lines()
            .find(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
            .and_then(|l| l.split_whitespace().next());
        assert_eq!(
            recorded,
            Some(sha256(&rtl).as_str()),
            "{}: the shipped macro was hardened from other RTL than the repo holds",
            p.key
        );

        let manifest =
            std::fs::read_to_string(sub.join("verilog/rtl/uprj_netlists.v")).unwrap_or_default();
        for include in [
            format!("`include \"gl/{}.v\"", p.macro_module),
            format!("`include \"{}.v\"", p.macro_module),
        ] {
            assert!(
                manifest.contains(&include),
                "{}: uprj_netlists.v lacks {include}, so `mprj` is undefined in one branch",
                p.key
            );
        }
        assert!(
            !manifest.contains("user_proj_example"),
            "{}: the manifest still names the scaffold's example",
            p.key
        );
        let lvs = std::fs::read_to_string(sub.join("lvs/user_project_wrapper/lvs_config.json"))
            .unwrap_or_default();
        assert!(
            lvs.contains(&format!("verilog/gl/{}.v", p.macro_module)),
            "{}: device LVS would extract the macro as a black box",
            p.key
        );

        let overlay = root().join(p.overlay);
        for f in files_under(&overlay) {
            let rel = f.strip_prefix(&overlay).unwrap();
            assert_eq!(
                std::fs::read(sub.join(rel)).ok(),
                std::fs::read(&f).ok(),
                "{}: {} in the submission is not the overlay's — re-assemble with scripts/caravel-submission.sh {}",
                p.key,
                rel.display(),
                p.key
            );
        }
    }
}

/// Every project's assembled submission is regenerated output, hundreds of megabytes of it, and the repo tracks its
/// ledgers instead: a root-anchored `.gitignore` rule must cover each project's submission directory, so a release's
/// `git add -A` cannot sweep one in.
#[test]
fn every_submission_directory_is_ignored() {
    let rules: Vec<String> = read(".gitignore")
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('/') && !l.starts_with('#'))
        .map(|l| l.trim_start_matches('/').trim_end_matches('/').to_string())
        .collect();
    // A root-anchored rule names a path, `*` matching within one component; it covers that path and everything
    // under it.
    let covers = |rule: &str, dir: &str| -> bool {
        let rule_parts: Vec<&str> = rule.split('/').collect();
        let dir_parts: Vec<&str> = dir.split('/').collect();
        let component = |pat: &str, name: &str| -> bool {
            let pieces: Vec<&str> = pat.split('*').collect();
            let Some(mut rest) = name.strip_prefix(pieces[0]) else {
                return false;
            };
            for (i, piece) in pieces[1..].iter().enumerate() {
                if i + 2 == pieces.len() {
                    return rest.ends_with(piece);
                }
                match rest.find(piece) {
                    Some(at) => rest = &rest[at + piece.len()..],
                    None => return false,
                }
            }
            rest.is_empty()
        };
        rule_parts.len() <= dir_parts.len()
            && rule_parts
                .iter()
                .zip(&dir_parts)
                .all(|(r, d)| component(r, d))
    };
    for p in CARAVEL_PROJECTS {
        assert!(
            rules.iter().any(|r| covers(r, p.submission_dir)),
            "{}: no root-anchored .gitignore rule covers {}",
            p.key,
            p.submission_dir
        );
    }
    assert!(
        covers("tools", "tools/caravel_submissions/wpe"),
        "a directory rule covers what is under it"
    );
    assert!(
        !covers("tools/caravel_submission", "tools/caravel_submission_wpe"),
        "and not a sibling it prefixes"
    );
    assert!(covers(
        "tools/caravel_submission*",
        "tools/caravel_submission_wpe"
    ));
}

/// A batch of re-signs assembles and prechecks a submission beside its staging ledgers (`SUBMISSION_OUT`), and lands
/// it with them, so the project's own submission directory never holds a pair the live ledgers do not record. MEASURED
/// 2026-10-16: the DRAM chain assembled its re-signed pair into `tools/caravel_submissions/dram` while its ledgers
/// waited in staging, and `an_assembled_submission_is_the_recorded_pair_and_the_overlay` read 57,732,922 bytes against
/// the 57,593,442 the live ledger recorded until they landed. The precheck reads the submission where it stands and
/// records it where it lands.
#[test]
fn a_staged_submission_is_checked_where_it_stands_and_recorded_where_it_lands() {
    let sub = read("scripts/caravel-submission.sh");
    assert!(
        sub.contains("OUT=\"${2:-${SUBMISSION_OUT:-$ROOT/$CP_SUBMISSION}}\""),
        "caravel-submission.sh assembles into SUBMISSION_OUT when it is set"
    );
    let pre = read("scripts/run-mpw-precheck.sh");
    assert!(
        pre.contains("SUBDIR=\"${SUBMISSION_OUT:-$CP_SUBMISSION}\"")
            && pre.contains("SUB=$(cd \"$SUBDIR\" && pwd)")
            && pre.contains("\"$ROOT/$CP_SUBMISSION\""),
        "run-mpw-precheck.sh checks SUBMISSION_OUT when it is set and records the project's own directory"
    );

    let dir = tempfile::tempdir().expect("a scratch directory");
    let staged = dir.path().join("staged");
    std::fs::create_dir_all(staged.join("gds")).unwrap();
    std::fs::write(staged.join("gds/user_project_wrapper.gds"), vec![7u8; 1234]).unwrap();
    let log = dir.path().join("precheck.log");
    std::fs::write(
        &log,
        "Executing Check 1 of 1: License\n{{License CHECK PASSED}}\n{{SUCCESS}} All Checks Passed\n",
    )
    .unwrap();
    let record = |extra: &[&str]| -> serde_json::Value {
        let out = dir.path().join("precheck.json");
        let st = std::process::Command::new("python3")
            .arg(root().join("scripts/record-mpw-precheck.py"))
            .arg(&log)
            .arg(&staged)
            .arg(&out)
            .args(extra)
            .output()
            .expect("run python3");
        assert!(
            st.status.success(),
            "{}",
            String::from_utf8_lossy(&st.stderr)
        );
        serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap()
    };
    let landed = root().join(CARAVEL_PROJECTS[0].submission_dir);
    let at = record(&[landed.to_str().unwrap()]);
    assert_eq!(
        at["meta"]["submission"].as_str(),
        Some(&*format!(
            "{} (assembled by scripts/caravel-submission.sh)",
            CARAVEL_PROJECTS[0].submission_dir
        )),
        "the record names the directory the submission lands in"
    );
    assert_eq!(
        at["submitted_gds_bytes"].as_u64(),
        Some(1234),
        "the bytes are the checked submission's"
    );
    assert!(at["all_checks_passed"].as_bool() == Some(true));
    let here = record(&[]);
    assert!(
        here["meta"]["submission"]
            .as_str()
            .is_some_and(|s| s.contains("staged (assembled")),
        "with no landing directory the record names the one it checked: {}",
        here["meta"]["submission"]
    );
}
