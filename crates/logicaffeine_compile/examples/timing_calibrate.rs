//! Hold the timing model's depth and area against Quartus's: every synthesis in the D5005 build ledger that ran to the
//! end, mapped again from its own Verilog (checked by SHA-256 against the ledger) through `codegen_sva::timing_graph`,
//! beside the Max Depth, ALUTs, registers, DSP blocks and memory bits Quartus reported for each instance in that build's
//! synthesis report, and the ALUTs fitted to the model's LUTs and carry bits over every instance of at least 10,000
//! (`timing_graph::fit_area`). Every memory the engine's RAM generator writes, its `_part`s included
//! (`spatial::decode_engine::rtl::ram::memory_boxes`), is a box timed as the block RAM it is, so a wide one's byte
//! lanes are never elaborated. A build whose Verilog the model cannot map, whose memory Quartus built in its fabric, or
//! whose top is too shallow to measure the delays by (`DEPTH_FLOOR`), is recorded as refused, with why, and the rest
//! calibrate without it.
//!
//!     timing_calibrate <quartus_builds.json> <timing_calibration.json> [--delays <lut>,<carry>,<dsp>,<memory>] [--netlists <dir>] [--fit]
//!
//! `--delays` holds the model to other delays than `Delays::LEVELS`. `--netlists <dir>` keeps each yosys netlist
//! there by the Verilog's SHA-256 and that of the script that made it (which names the top and the parameters), or the
//! refusal yosys gave it, and reads it back on the next run instead of elaborating again. `--fit` fits the delays to
//! the builds' top-level Max Depth (`timing_graph::fit`), searching one delay at a time against a LUT's, each in turn
//! until none moves: the carry from 0 to 30 hundredths of a LUT, the DSP block's and the memory read's from none to
//! three LUTs in tenths of one; and records every build at the delays fitted.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::exit;

use logicaffeine_compile::codegen_sva::quartus::{parse_hierarchy, Ledger};
use logicaffeine_compile::codegen_sva::timing_graph::{
    fit, fit_area, netlist, read, script, CalibratedInstance, Calibration, CalibrationRow, Delays, InstanceArea,
    QuartusArea, RefusedBuild, TimingGraph, AREA_CARRIES,
};
use logicaffeine_compile::spatial::decode_engine::rtl::ram::memory_boxes;
use logicaffeine_compile::crypto_accel::sha256;

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("timing_calibrate: {why}");
    exit(1)
}

/// Why a build is outside the model, when Quartus built some of the memory bits the model maps as memories in its
/// fabric instead: the model times and sizes every memory as the block it asks for, and a memory Quartus made of
/// registers and multiplexers (declared in a generate block, or read in MLABs without a register and without
/// `no_rw_check`, or asking for logic) is neither its timing nor its area.
fn built_in_logic(graph: &TimingGraph, quartus: &BTreeMap<String, (f64, QuartusArea)>) -> Option<String> {
    let model = graph.area_under("").memory_bits;
    let held = quartus.get("").map_or(0, |(_, area)| area.memory_bits);
    (held < model).then(|| {
        format!("Quartus built {} of the {model} memory bits the model maps as memories in its fabric, holding {held} in memory blocks", model - held)
    })
}

/// The least logic levels a build's top has to reach to measure the delays by: below it a level either way is most of
/// its depth, as a few LUTs either way are most of an instance below [`AREA_FLOOR`].
const DEPTH_FLOOR: f64 = 10.0;

/// Why a build is no measure of the delays, when its top is shallower than [`DEPTH_FLOOR`].
fn too_shallow(quartus: &BTreeMap<String, (f64, QuartusArea)>) -> Option<String> {
    let depth = quartus.get("").map_or(0.0, |(d, _)| *d);
    (depth < DEPTH_FLOOR).then(|| {
        format!("its top is {depth} logic levels deep, under the {DEPTH_FLOOR} the delays are measured over: a level either way is most of it")
    })
}

/// The least ALUTs an instance has to be drawn into the area fit: below it a few LUTs either way is most of one.
const AREA_FLOOR: u64 = 10_000;

fn hex(bytes: &[u8]) -> String {
    sha256(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The top entity and the parameters a project's settings file sets.
fn settings(qsf: &str) -> (Option<String>, Vec<(String, String)>) {
    let mut top = None;
    let mut params = Vec::new();
    for line in qsf.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["set_global_assignment", "-name", "TOP_LEVEL_ENTITY", name] => top = Some(name.to_string()),
            ["set_parameter", "-name", name, value] => params.push((name.to_string(), value.to_string())),
            _ => {}
        }
    }
    (top, params)
}

/// A synthesis the ledger records: its top, its Verilog's SHA-256, the model's graph of it, and Quartus's Max Depth and
/// area of each instance.
struct Build {
    name: String,
    top: String,
    sha: String,
    graph: TimingGraph,
    quartus: BTreeMap<String, (f64, QuartusArea)>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: timing_calibrate <quartus_builds.json> <timing_calibration.json> [--delays L,C,D,M] [--netlists <dir>] [--fit]");
        exit(2);
    }
    let (ledger_path, out_path) = (Path::new(&args[0]), Path::new(&args[1]));
    let mut delays = Delays::LEVELS;
    let mut cache: Option<PathBuf> = None;
    let mut fitting = false;
    let mut rest = &args[2..];
    while let [flag, tail @ ..] = rest {
        if flag == "--fit" {
            fitting = true;
            rest = tail;
            continue;
        }
        let [value, tail @ ..] = tail else { fail(format!("{flag} takes a value")) };
        match flag.as_str() {
            "--delays" => {
                let v: Vec<i64> = value.split(',').map(|x| x.trim().parse().unwrap_or_else(|_| fail(format!("a delay is a number: {x}")))).collect();
                let [lut, carry, dsp, memory] = v[..] else { fail("--delays takes four: lut,carry,dsp,memory") };
                delays = Delays { lut, carry, dsp, memory };
            }
            "--netlists" => cache = Some(PathBuf::from(value)),
            _ => fail(format!("unknown flag {flag}")),
        }
        rest = tail;
    }
    let ledger = Ledger::load(ledger_path).unwrap_or_else(|e| fail(e));
    let mut builds: Vec<Build> = Vec::new();
    let mut refused: Vec<RefusedBuild> = Vec::new();
    for row in &ledger.rows {
        let Some((v, q)) = row.synthesis_sources() else { continue };
        let (verilog_path, qsf_path) = (PathBuf::from(&v.path), PathBuf::from(&q.path));
        let verilog = std::fs::read(&verilog_path).unwrap_or_else(|e| fail(format!("{}: {e}", verilog_path.display())));
        let qsf = std::fs::read(&qsf_path).unwrap_or_else(|e| fail(format!("{}: {e}", qsf_path.display())));
        for (path, bytes) in [(&verilog_path, &verilog), (&qsf_path, &qsf)] {
            let recorded = row.inputs.iter().find(|i| Path::new(&i.path) == path.as_path()).map(|i| i.sha256.as_str());
            if recorded != Some(hex(bytes).as_str()) {
                fail(format!("{}: {} is not the file {} was compiled from", row.name, path.display(), row.name));
            }
        }
        let (top, params) = settings(&String::from_utf8_lossy(&qsf));
        let top = top.unwrap_or_else(|| fail(format!("{}: no TOP_LEVEL_ENTITY", qsf_path.display())));
        let revision = qsf_path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
        let report = qsf_path.with_file_name(format!("{revision}.syn.rpt"));
        let report = std::fs::read_to_string(&report).unwrap_or_else(|e| fail(format!("{}: {e}", report.display())));
        let mut quartus: BTreeMap<String, (f64, QuartusArea)> = BTreeMap::new();
        for n in parse_hierarchy(&report) {
            if let Some(d) = n.max_depth {
                let area = QuartusArea { aluts: n.aluts, registers: n.registers, dsp: n.dsp, memory_bits: n.memory_bits };
                quartus.insert(if n.path == "|" { String::new() } else { n.path.clone() }, (d, area));
            }
        }
        let sha = hex(&verilog);
        let pairs: Vec<(&str, &str)> = params.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let text = String::from_utf8_lossy(&verilog);
        let memories = memory_boxes(&text, &top);
        let boxes: Vec<(&str, _, _)> = memories.iter().map(|(m, c, a)| (m.as_str(), *c, *a)).collect();
        let made_by = hex(script(&top, &pairs, &boxes).as_bytes());
        let key = format!("{}-{}-{top}.json", &sha[..16], &made_by[..16]);
        let refusal = cache.as_ref().map(|d| d.join(format!("{key}.refused")));
        let json = match (cache.as_ref().map(|d| d.join(&key)), &refusal) {
            (Some(file), _) if file.exists() => Ok(std::fs::read_to_string(&file).unwrap_or_else(|e| fail(format!("{}: {e}", file.display())))),
            (_, Some(file)) if file.exists() => Err(std::fs::read_to_string(file).unwrap_or_else(|e| fail(format!("{}: {e}", file.display())))),
            (cached, _) => netlist(&text, &top, &pairs, &boxes)
                .inspect(|text| {
                    if let Some(file) = cached {
                        std::fs::create_dir_all(file.parent().expect("a directory")).unwrap_or_else(|e| fail(e));
                        std::fs::write(&file, text).unwrap_or_else(|e| fail(format!("{}: {e}", file.display())));
                    }
                })
                .map_err(|e| {
                    let said = e.to_string();
                    if let Some(file) = &refusal {
                        std::fs::create_dir_all(file.parent().expect("a directory")).unwrap_or_else(|e| fail(e));
                        std::fs::write(file, &said).unwrap_or_else(|e| fail(format!("{}: {e}", file.display())));
                    }
                    said
                }),
        };
        match json.and_then(|json| read(&json, &delays).map_err(|e| e.to_string())) {
            Ok(graph) => match built_in_logic(&graph, &quartus).or_else(|| too_shallow(&quartus)) {
                Some(why) => {
                    println!("{}: refused: {why}", row.name);
                    refused.push(RefusedBuild { build: row.name.clone(), verilog_sha256: sha, why });
                }
                None => builds.push(Build { name: row.name.clone(), top, sha, graph, quartus }),
            },
            Err(said) => {
                let why = said.lines().find(|l| l.contains("ERROR")).or(said.lines().next()).unwrap_or_default().trim().to_string();
                println!("{}: refused: {why}", row.name);
                refused.push(RefusedBuild { build: row.name.clone(), verilog_sha256: sha, why });
            }
        }
    }
    if fitting {
        let mut samples: Vec<(TimingGraph, f64)> = builds
            .iter_mut()
            .map(|b| {
                let q = b.quartus.get("").unwrap_or_else(|| fail(format!("{}: no top-level Max Depth", b.name))).0;
                (std::mem::take(&mut b.graph), q)
            })
            .collect();
        let ratio = |d: &Delays, x: i64| (x as f64 * 100.0 / d.lut as f64).round() as i64;
        let mut at = Delays { lut: 100, carry: 0, dsp: 100, memory: 100 };
        let mut seen = vec![at];
        let mut fitted = None;
        loop {
            let carries: Vec<Delays> = (0..=30).map(|carry| Delays { carry, ..at }).collect();
            let by_carry = fit(&mut samples, &carries);
            let carry = ratio(&by_carry.delays, by_carry.delays.carry);
            let dsps: Vec<Delays> = (0..=30).map(|k| Delays { carry, dsp: 10 * k, ..at }).collect();
            let by_dsp = fit(&mut samples, &dsps);
            let dsp = ratio(&by_dsp.delays, by_dsp.delays.dsp);
            let memories: Vec<Delays> = (0..=30).map(|k| Delays { carry, dsp, memory: 10 * k, ..at }).collect();
            let by_memory = fit(&mut samples, &memories);
            let memory = ratio(&by_memory.delays, by_memory.delays.memory);
            println!("fit: {:?}, error {:.4}", by_memory.delays, by_memory.error);
            let next = Delays { lut: 100, carry, dsp, memory };
            fitted = Some(by_memory);
            if seen.contains(&next) {
                break;
            }
            seen.push(next);
            at = next;
        }
        delays = fitted.expect("a round of the search").delays;
        for (b, (graph, _)) in builds.iter_mut().zip(samples) {
            b.graph = graph;
        }
    }
    let mut calibration = Calibration { refused, ..Calibration::default() };
    for b in builds {
        let depth = b.graph.depth().unwrap_or_else(|e| fail(format!("{}: {e}", b.name)));
        let instances: Vec<CalibratedInstance> = depth
            .instances
            .iter()
            .filter_map(|i| {
                b.quartus.get(&i.path).map(|&(q, area)| CalibratedInstance {
                    path: i.path.clone(),
                    quartus: q,
                    predicted: i.total,
                    area: Some(InstanceArea { quartus: area, model: b.graph.area_under(&i.path) }),
                })
            })
            .collect();
        let top_line = instances.iter().find(|i| i.path.is_empty()).map_or("-".into(), |i| format!("{} vs {}", i.quartus, i.predicted as f64 / 100.0));
        println!("{}: top {top_line} levels, {} instances matched", b.name, instances.len());
        calibration.rows.push(CalibrationRow { build: b.name, top: b.top, verilog_sha256: b.sha, delays, instances });
    }
    let samples: Vec<(_, u64)> =
        calibration.rows.iter().flat_map(|r| r.instances.iter()).filter_map(|i| i.area.map(|a| (a.model, a.quartus.aluts))).collect();
    let area = fit_area(&samples, AREA_FLOOR, &AREA_CARRIES);
    println!("area: ALUTs = {:.3} × (LUTs + {:.2} × carry bits) over {} instances of {}+ ALUTs, error {:.4}", area.scale, area.carry, area.instances, AREA_FLOOR, area.error);
    calibration.area = Some(area);
    let mut text = serde_json::to_string_pretty(&calibration).unwrap_or_else(|e| fail(e));
    text.push('\n');
    std::fs::write(out_path, text).unwrap_or_else(|e| fail(format!("{}: {e}", out_path.display())));
}
