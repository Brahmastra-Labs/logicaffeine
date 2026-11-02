//! Emit a block's OpenLane `config.json` from the compiler's own knowledge of the design and the
//! process, so the physical-design settings that were MEASURED to be necessary cannot be missed by
//! whoever runs the flow next.
//!
//!   cargo run -q -p logicaffeine-compile --example emit_flow_config -- \
//!       <block> <clock_port> <period_ns> [--pdk sky130A] [--fixed-die] [--ports N] \
//!       [--density D] [--met1-penalty P] [--grt-overflow-iters N] [--grt-adjustment A] [--dpl-padding-per-side N] [--clock-max-wire UM|openroad] [--max-wire UM|openroad] [--post-antenna-upsize] [--slew-margin P] [--pl-slew-margin P] [--antenna-threshold N] [--eco-iters N] [--grt-antenna-iters N] [--grt-antenna-margin PCT] [--pdn-vpitch UM] [--hold-slack-margin NS] [--setup-slack-margin NS] [--cores N] [--antenna jog-only|diodes+jog|heuristic+jog]
//!   ... --registered-period <block>              the block's registered signoff period
//!   ... --caravel-project <key> [--pdk NAME]     a Caravel user project as shell assignments
//!   ... --vehicle-macro-sdc <clock port> [--pdk NAME]   the SDC a vehicle user macro on that harness clock is timed with
//!
//! Overrides exist for deliberate experiments. They are NOT the normal path: the defaults carry
//! their measurements in `codegen_sva::flow_config`, and a block built with something else should
//! have a recorded reason.

use logicaffeine_compile::codegen_sva::caravel_project::{CaravelProject, CARAVEL_PROJECTS};
use logicaffeine_compile::codegen_sva::flow_config::{
    openlane_config, parse_wire_limit_um, vehicle_macro_config, vehicle_macro_sdc, AntennaStrategy,
    ClosureSettings, DesignProfile, MacroGrid, SynthStrategy, VehicleMacro,
};
use logicaffeine_compile::codegen_sva::pdk_target::{HarnessClock, PdkTarget};
use logicaffeine_compile::codegen_sva::signoff_blocks::{registered_period_ns, resolve_synth_strategy};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    // `--registered-period <block>`: print the block's signoff period and nothing else, or exit 3
    // when it is not registered. Scripts read the period from here instead of probing the emitter
    // with a wrong one and scraping the refusal — which, under `set -euo pipefail`, killed the
    // Caravel macro harden with a silent exit 2 before it started.
    if a.first().map(String::as_str) == Some("--registered-period") {
        match a.get(1).and_then(|b| registered_period_ns(b)) {
            Some(p) => {
                println!("{p}");
                return;
            }
            None => {
                eprintln!(
                    "{} has no registered signoff period (codegen_sva::signoff_blocks)",
                    a.get(1).map(String::as_str).unwrap_or("<missing block>")
                );
                std::process::exit(3)
            }
        }
    }
    // `--vehicle-macro-sdc <clock port> [--pdk NAME]`: print the SDC a vehicle user macro whose logic runs on that
    // harness clock is timed with (the process's base constraints, the harness's minimum input delays, and the
    // harness's bus on its own clock when the macro runs on another), or refuse, exit 2, for a port that is no
    // harness clock or an uncharacterized vehicle.
    if a.first().map(String::as_str) == Some("--vehicle-macro-sdc") {
        let usage = || -> ! {
            eprintln!("usage: emit_flow_config --vehicle-macro-sdc <wb_clk_i|user_clock2> [--pdk NAME]");
            std::process::exit(2)
        };
        let clock = a.get(1).and_then(|p| HarnessClock::by_port(p)).unwrap_or_else(|| usage());
        let pdk_name = match a.get(2..).unwrap_or_default() {
            [] => "sky130A",
            [flag, name] if flag == "--pdk" => name.as_str(),
            _ => usage(),
        };
        let pdk = PdkTarget::by_name(pdk_name).unwrap_or_else(|| {
            eprintln!("unknown PDK {pdk_name:?}");
            std::process::exit(2)
        });
        match vehicle_macro_sdc(pdk, clock) {
            Ok(sdc) => print!("{sdc}"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2)
            }
        }
        return;
    }
    // `--caravel-project <key> [--pdk NAME]`: print the Caravel user project, then its vehicle's signoff clock, as
    // `NAME=value` lines for the flow scripts to `eval` (codegen_sva::caravel_project), or refuse, exit 2, when the
    // key is unknown or the project does not fit the process's vehicle.
    if a.first().map(String::as_str) == Some("--caravel-project") {
        let key = a.get(1).map(String::as_str).unwrap_or("<missing key>");
        let project = CaravelProject::by_key(key).unwrap_or_else(|| {
            eprintln!(
                "no Caravel project {key:?}; the projects are {:?} (codegen_sva::caravel_project)",
                CARAVEL_PROJECTS.iter().map(|p| p.key).collect::<Vec<_>>()
            );
            std::process::exit(2)
        });
        let pdk_name = match a.get(2..).unwrap_or_default() {
            [] => "sky130A",
            [flag, name] if flag == "--pdk" => name.as_str(),
            _ => {
                eprintln!("usage: emit_flow_config --caravel-project <key> [--pdk NAME]");
                std::process::exit(2)
            }
        };
        let pdk = PdkTarget::by_name(pdk_name).unwrap_or_else(|| {
            eprintln!("unknown PDK {pdk_name:?}");
            std::process::exit(2)
        });
        match project.flow_shell(pdk) {
            Ok(sh) => print!("{sh}"),
            Err(e) => {
                eprintln!("Caravel project {key}: {e}");
                std::process::exit(2)
            }
        }
        return;
    }
    if a.len() < 3 {
        eprintln!(
            "usage: emit_flow_config <block> <clock_port> <period_ns> [--pdk NAME] [--fixed-die] \
             [--ports N] [--density D] [--met1-penalty P] [--grt-overflow-iters N] [--grt-adjustment A] [--dpl-padding-per-side N] [--clock-max-wire UM|openroad] [--max-wire UM|openroad] [--post-antenna-upsize] [--slew-margin P] [--pl-slew-margin P] [--antenna-threshold N] \
             [--eco-iters N] [--cores N] [--antenna jog-only|diodes+jog|heuristic+jog] [--eco-diode-fallback] \
             [--diodes-before-resize] [--post-diode-upsize] [--upsize-margin P] [--upsize-all-corners] \
             [--no-cts-fanout] [--no-diode-fanout] [--fanout-margin N] \
             [--vehicle-macro W H [--hold-margin NS] [--macro-grid own|fed]] \
             [--synth-strategy area0..3|delay0..4|default]"
        );
        std::process::exit(2);
    }
    let period: f64 = a[2].parse().unwrap_or_else(|_| {
        eprintln!("period_ns must be a number, got {:?}", a[2]);
        std::process::exit(2)
    });
    // A registered block is built at its registered period and nothing else: that value is derived
    // from the block's measured SLOW corner, and any other period is a frequency the part does not
    // guarantee (codegen_sva::signoff_blocks).
    if let Some(reg) = registered_period_ns(&a[0]) {
        if (reg - period).abs() > 1e-9 {
            eprintln!(
                "{period} ns is not the registered signoff period for {}: it is {reg} ns, derived from \
                 the block's measured slow corner (codegen_sva::signoff_blocks)",
                a[0]
            );
            std::process::exit(2);
        }
    }
    let mut d = DesignProfile::new(&a[0], &a[1], period);
    let mut c = ClosureSettings::default();
    let mut strategy_given = false;
    let mut pdk_name = "sky130A".to_string();
    let mut vehicle: Option<VehicleMacro> = None;

    let mut i = 3;
    while i < a.len() {
        let need = |i: usize| -> &String {
            a.get(i + 1).unwrap_or_else(|| {
                eprintln!("{} needs a value", a[i]);
                std::process::exit(2)
            })
        };
        match a[i].as_str() {
            // A user macro placed inside the shuttle vehicle's wrapper: its own die (W H, um), the
            // vehicle's power nets and routing ceiling. Hold margin: see VehicleMacro.
            "--vehicle-macro" => {
                let dim = |k: usize| -> f64 {
                    a.get(i + k)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or_else(|| {
                            eprintln!("--vehicle-macro needs a die width and height in um");
                            std::process::exit(2)
                        })
                };
                vehicle = Some(VehicleMacro {
                    die_um: (dim(1), dim(2)),
                    hold_margin_ns: 0.7,
                    grid: MacroGrid::Own,
                });
                i += 3;
            }
            "--macro-grid" => {
                let g = MacroGrid::parse(need(i)).unwrap_or_else(|| {
                    eprintln!("--macro-grid is own or fed, got {:?}", a[i + 1]);
                    std::process::exit(2)
                });
                match vehicle.as_mut() {
                    Some(v) => v.grid = g,
                    None => {
                        eprintln!("--macro-grid applies to --vehicle-macro, which must come first");
                        std::process::exit(2)
                    }
                }
                i += 2;
            }
            "--hold-margin" => {
                let hm: f64 = need(i).parse().unwrap_or_else(|_| {
                    eprintln!("--hold-margin needs a number of ns");
                    std::process::exit(2)
                });
                match vehicle.as_mut() {
                    Some(v) => v.hold_margin_ns = hm,
                    None => {
                        eprintln!(
                            "--hold-margin applies to --vehicle-macro, which must come first"
                        );
                        std::process::exit(2)
                    }
                }
                i += 2;
            }
            "--fixed-die" => {
                d.fixed_die = true;
                i += 1;
            }
            "--pdk" => {
                pdk_name = need(i).clone();
                i += 2;
            }
            "--ports" => {
                d.port_bits = need(i).parse().unwrap_or(0);
                i += 2;
            }
            "--macro" => {
                d.macros.push(need(i).clone());
                i += 2;
            }
            "--density" => {
                c.target_density = need(i)
                    .parse()
                    .ok()
                    .filter(|d| *d > 0.0 && *d <= 1.0)
                    .unwrap_or_else(|| {
                        eprintln!(
                            "--density needs a placement density in (0, 1], got {:?}",
                            a[i + 1]
                        );
                        std::process::exit(2)
                    });
                i += 2;
            }
            // A signal net's longest unbuffered wire (ClosureSettings::signal_max_wire_um), in um.
            "--max-wire" => {
                c.signal_max_wire_um = parse_wire_limit_um(need(i)).unwrap_or_else(|| {
                    eprintln!("--max-wire needs a length in um, or `openroad` for its own limit, got {:?}", a[i + 1]);
                    std::process::exit(2)
                });
                i += 2;
            }
            // Upsize again after global routing's antenna repair (ClosureSettings::post_antenna_upsize).
            "--post-antenna-upsize" => {
                c.post_antenna_upsize = true;
                i += 1;
            }
            // The clock input's longest unbuffered wire (ClosureSettings::clock_max_wire_um), in um.
            "--clock-max-wire" => {
                c.clock_max_wire_um = parse_wire_limit_um(need(i)).unwrap_or_else(|| {
                    eprintln!("--clock-max-wire needs a length in um, or `openroad` for its own limit, got {:?}", a[i + 1]);
                    std::process::exit(2)
                });
                i += 2;
            }
            // The met1 global-route penalty (ClosureSettings::met1_route_penalty), for a block the recipe's
            // penalty leaves too little met1 to route.
            "--met1-penalty" => {
                c.met1_route_penalty = need(i)
                    .parse()
                    .ok()
                    .filter(|p| (0.0..1.0).contains(p))
                    .unwrap_or_else(|| {
                        eprintln!(
                            "--met1-penalty needs a penalty in [0, 1), got {:?}",
                            a[i + 1]
                        );
                        std::process::exit(2)
                    });
                i += 2;
            }
            // Global routing's overflow iterations (ClosureSettings::grt_overflow_iters).
            "--grt-overflow-iters" => {
                let n = need(i).parse::<u32>().ok().filter(|n| *n > 0).unwrap_or_else(|| {
                    eprintln!("--grt-overflow-iters needs a positive number of iterations, got {:?}", a[i + 1]);
                    std::process::exit(2)
                });
                c.grt_overflow_iters = Some(n);
                i += 2;
            }
            // The share of each gcell's capacity global routing holds back (ClosureSettings::grt_adjustment).
            "--grt-adjustment" => {
                let adj = need(i).parse::<f64>().ok().filter(|v| (0.0..1.0).contains(v)).unwrap_or_else(|| {
                    eprintln!("--grt-adjustment needs an adjustment in [0, 1), got {:?}", a[i + 1]);
                    std::process::exit(2)
                });
                c.grt_adjustment = Some(adj);
                i += 2;
            }
            // The sites a side legalization keeps around every cell (ClosureSettings::dpl_cell_padding_per_side).
            "--dpl-padding-per-side" => {
                let n = need(i).parse::<u32>().ok().filter(|n| *n >= 1).unwrap_or_else(|| {
                    eprintln!("--dpl-padding-per-side needs a positive number of sites, got {:?}", a[i + 1]);
                    std::process::exit(2)
                });
                c.dpl_cell_padding_per_side = Some(n);
                i += 2;
            }
            // Raises the ROUTING-stage resizer only. The placement resizer keeps its own
            // default because that is the one with the memory cliff (114 GB true peak, OOM, on
            // a 457k-cell block at margin 35). Use --pl-slew-margin to raise it deliberately.
            "--slew-margin" => {
                c.slew_margin_pct = need(i).parse().unwrap_or(c.slew_margin_pct);
                c.cap_margin_pct = c.slew_margin_pct;
                i += 2;
            }
            "--eco-diode-fallback" => {
                c.eco_diode_fallback = true;
                i += 1;
            }
            "--post-diode-upsize" => {
                c.post_diode_upsize = true;
                i += 1;
            }
            "--upsize-all-corners" => {
                c.post_diode_upsize_all_corners = true;
                i += 1;
            }
            "--glb-skip-pin-swap" => {
                c.glb_skip_pin_swap = true;
                i += 1;
            }
            "--comparative" => {
                c.comparative = true;
                i += 1;
            }
            "--no-cts-fanout" => {
                c.cts_within_fanout = false;
                i += 1;
            }
            "--no-diode-fanout" => {
                c.diode_fanout_repair = false;
                i += 1;
            }
            "--fanout-margin" => {
                c.fanout_margin_loads = need(i).parse().unwrap_or_else(|_| {
                    eprintln!(
                        "--fanout-margin needs a number of loads, got {:?}",
                        a[i + 1]
                    );
                    std::process::exit(2)
                });
                i += 2;
            }
            "--upsize-margin" => {
                c.post_diode_upsize_margin_pct = need(i)
                    .parse()
                    .ok()
                    .filter(|m| *m < 100)
                    .unwrap_or_else(|| {
                        eprintln!(
                            "--upsize-margin needs a percentage below 100, got {:?}",
                            a[i + 1]
                        );
                        std::process::exit(2)
                    });
                i += 2;
            }
            "--diodes-before-resize" => {
                c.diodes_before_resize = true;
                i += 1;
            }
            "--pl-slew-margin" => {
                c.pl_slew_margin_pct = need(i).parse().unwrap_or(c.pl_slew_margin_pct);
                i += 2;
            }
            "--antenna-threshold" => {
                c.heuristic_antenna_threshold = need(i).parse::<u32>().ok().filter(|um| *um >= 1).unwrap_or_else(|| {
                    eprintln!("--antenna-threshold needs a positive span in um, got {:?}", a[i + 1]);
                    std::process::exit(2)
                });
                i += 2;
            }
            "--eco-iters" => {
                c.antenna_eco_iters = need(i).parse().unwrap_or_else(|_| {
                    eprintln!(
                        "--eco-iters needs a number of iterations, got {:?}",
                        a[i + 1]
                    );
                    std::process::exit(2)
                });
                i += 2;
            }
            "--hold-slack-margin" => {
                c.hold_slack_margin_ns = match need(i).parse::<f64>() {
                    Ok(m) if m >= 0.0 => Some(m),
                    _ => {
                        eprintln!(
                            "--hold-slack-margin needs a margin in ns, got {:?}",
                            a[i + 1]
                        );
                        std::process::exit(2)
                    }
                };
                i += 2;
            }
            "--setup-slack-margin" => {
                c.setup_slack_margin_ns = match need(i).parse::<f64>() {
                    Ok(m) if m >= 0.0 => Some(m),
                    _ => {
                        eprintln!("--setup-slack-margin needs a margin in ns, got {:?}", a[i + 1]);
                        std::process::exit(2)
                    }
                };
                i += 2;
            }
            "--pdn-vpitch" => {
                c.pdn_vpitch_um = match need(i).parse::<f64>() {
                    Ok(p) if p > 0.0 => Some(p),
                    _ => {
                        eprintln!("--pdn-vpitch needs a strap pitch in um, got {:?}", a[i + 1]);
                        std::process::exit(2)
                    }
                };
                i += 2;
            }
            "--grt-antenna-iters" => {
                c.grt_antenna_iters = need(i).parse().unwrap_or_else(|_| {
                    eprintln!(
                        "--grt-antenna-iters needs a number of iterations, got {:?}",
                        a[i + 1]
                    );
                    std::process::exit(2)
                });
                i += 2;
            }
            "--grt-antenna-margin" => {
                c.grt_antenna_margin_pct = need(i).parse().unwrap_or_else(|_| {
                    eprintln!(
                        "--grt-antenna-margin needs a percentage, got {:?}",
                        a[i + 1]
                    );
                    std::process::exit(2)
                });
                i += 2;
            }
            "--antenna" => {
                c.antenna = match need(i).as_str() {
                    "jog-only" => AntennaStrategy::JogOnly,
                    "diodes+jog" => AntennaStrategy::DiodesAndJog,
                    "heuristic+jog" => AntennaStrategy::HeuristicDiodesAndJog,
                    other => {
                        eprintln!("--antenna must be jog-only, diodes+jog or heuristic+jog, got {other:?}");
                        std::process::exit(2)
                    }
                };
                i += 2;
            }
            // `area0..=3` / `delay0..=4`, or `default` for OpenLane's own (the Caravel project mode prints `default`
            // for a project that sets none). See ClosureSettings::synth_strategy for the measurement.
            "--synth-strategy" => {
                c.synth_strategy = match need(i).as_str() {
                    "default" => None,
                    other => Some(SynthStrategy::parse(other).unwrap_or_else(|| {
                        eprintln!("--synth-strategy must be area0..area3, delay0..delay4 or default, got {other:?}");
                        std::process::exit(2)
                    })),
                };
                strategy_given = true;
                i += 2;
            }
            "--cores" => {
                c.routing_cores = need(i).parse().unwrap_or(c.routing_cores);
                i += 2;
            }
            other => {
                eprintln!("unknown flag {other:?}");
                std::process::exit(2);
            }
        }
    }

    // Refuse loudly on an unknown process rather than silently defaulting to sky130 — building a
    // block against the wrong PDK is not a mistake that shows up until signoff.
    let pdk = PdkTarget::by_name(&pdk_name).unwrap_or_else(|| {
        eprintln!(
            "unknown PDK {pdk_name:?}; this flow supports: {:?}",
            PdkTarget::all().iter().map(|p| p.name).collect::<Vec<_>>()
        );
        std::process::exit(2)
    });

    // A registered block is synthesized under the strategy its period was measured with: the period is abc's delay
    // target, and what abc does with it is the strategy's, so another strategy rebuilds a part that does not meet the
    // period (codegen_sva::signoff_blocks).
    let asked = strategy_given.then_some(c.synth_strategy);
    c.synth_strategy = resolve_synth_strategy(&a[0], asked).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2)
    });
    if vehicle.is_some() && c.hold_slack_margin_ns.is_some() {
        eprintln!(
            "--hold-slack-margin is a block's; a vehicle macro's hold margin is --hold-margin"
        );
        std::process::exit(2)
    }
    match vehicle {
        Some(v) => match vehicle_macro_config(&d, &v, pdk, &c) {
            Ok(cfg) => print!("{cfg}"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2)
            }
        },
        None => print!("{}", openlane_config(&d, pdk, &c)),
    }
}
