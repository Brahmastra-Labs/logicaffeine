//! Hold a persona's design to the D5005's partial-reconfiguration region and its clock before Quartus runs
//! (`codegen_sva::preflight`): its Verilog through the timing model, its ALUTs and ALMs as the timing calibration's area
//! fit and the ledger's syntheses predict them, its DSP blocks and registers, its deepest path and its loop bound at the
//! card's delay a level, and the fitter's time, each held to what the build ledger's personas measured.
//!
//!     d5005_preflight <top> <verilog>... [NAME=VALUE ...] --period <ns> [--ns <ns per LUT delay>] [--delays L,C,D,M]
//!                     [--part <module>[=<cycles>]]... [--box <module>[=memory]]... [--box-memories]
//!                     [--ledger <quartus_builds.json>] [--calibration <timing_calibration.json>]
//!
//! The delays default to fit27's (LUT 100, carry 7, DSP 460, memory 330), at the 0.55 ns a LUT delay that flat fit of
//! the 27B engine measured, unless `--delays` and `--ns` say otherwise. The ledger and the calibration default to the
//! repository's (`benchmarks/silicon/d5005`). The top is timed whole a period a path unless a `--part` names a module
//! timed on its own, its paths `<cycles>` periods (1 unless given), as a multicycle constraint gives them; the top's area
//! is the whole design's either way. `--box` and `--box-memories` are `timing_report`'s. It exits 0 when the design goes
//! (tight or not) and 1 when anything is refused.

use std::path::{Path, PathBuf};
use std::process::exit;

use logicaffeine_compile::codegen_sva::preflight::{device_library, preflight_domains, Design, Target, Timed};
use logicaffeine_compile::codegen_sva::quartus::Ledger;
use logicaffeine_compile::codegen_sva::timing_graph::{modules_numbered, synthesize, Calibration, Cost, Delays};
use logicaffeine_compile::spatial::decode_engine::rtl::ram::{bram_name, bram_re_name, ram_ce_name, ram_name};

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("d5005_preflight: {why}");
    exit(2)
}

fn number<T: std::str::FromStr>(flag: &str, v: Option<&String>) -> T {
    let v = v.unwrap_or_else(|| fail(format!("{flag} takes a value")));
    v.parse().unwrap_or_else(|_| fail(format!("{flag}: not a number: {v}")))
}

/// The memories of `verilog` the engine's RAM generator writes, each a module per width, but `top`.
fn memory_modules(verilog: &str, top: &str) -> Vec<String> {
    let names = [ram_name(8), bram_name(8), bram_re_name(8), ram_ce_name(8)];
    let prefixes: Vec<&str> = names.iter().map(|n| n.trim_end_matches(|c: char| c.is_ascii_digit())).collect();
    modules_numbered(verilog, &prefixes).into_iter().filter(|m| m != top).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((top, rest)) = args.split_first() else {
        eprintln!("usage: d5005_preflight <top> <verilog>... [NAME=VALUE ...] --period <ns> [--ns <ns per LUT delay>] [--delays L,C,D,M] [--box <module>[=memory]]... [--box-memories] [--ledger <file>] [--calibration <file>]");
        exit(2);
    };
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/d5005");
    let (mut ledger_path, mut calibration_path) = (data.join("quartus_builds.json"), data.join("timing_calibration.json"));
    let mut delays = Delays { lut: 100, carry: 7, dsp: 460, memory: 330 };
    let (mut ns, mut period) = (0.55, None::<f64>);
    let (mut verilog, mut params, mut boxes, mut memories) = (String::new(), Vec::new(), Vec::<(String, Cost)>::new(), false);
    let mut parts: Vec<(String, u32)> = Vec::new();
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--period" => period = Some(number("--period", it.next())),
            "--ns" => ns = number("--ns", it.next()),
            "--delays" => {
                let v: Vec<i64> = it
                    .next()
                    .unwrap_or_else(|| fail("--delays takes lut,carry,dsp,memory"))
                    .split(',')
                    .map(|x| x.trim().parse().unwrap_or_else(|_| fail(format!("a delay is a number: {x}"))))
                    .collect();
                let [lut, carry, dsp, memory] = v[..] else { fail("--delays takes four: lut,carry,dsp,memory") };
                delays = Delays { lut, carry, dsp, memory };
            }
            "--box" => {
                let b = it.next().unwrap_or_else(|| fail("--box takes a module"));
                boxes.push(match b.split_once('=') {
                    None => (b.clone(), Cost::NONE),
                    Some((m, "memory")) => (m.to_string(), Cost::MEMORY),
                    Some((_, other)) => fail(format!("--box {b}: a box's outputs are plain or =memory, not {other}")),
                });
            }
            "--box-memories" => memories = true,
            "--part" => {
                let p = it.next().unwrap_or_else(|| fail("--part takes a module"));
                parts.push(match p.split_once('=') {
                    None => (p.clone(), 1),
                    Some((m, c)) => (m.to_string(), c.parse().unwrap_or_else(|_| fail(format!("--part {p}: cycles are a number")))),
                });
            }
            "--ledger" => ledger_path = PathBuf::from(it.next().unwrap_or_else(|| fail("--ledger takes a file"))),
            "--calibration" => calibration_path = PathBuf::from(it.next().unwrap_or_else(|| fail("--calibration takes a file"))),
            file if Path::new(file).is_file() => {
                verilog.push_str(&std::fs::read_to_string(file).unwrap_or_else(|e| fail(format!("{file}: {e}"))));
                verilog.push('\n');
            }
            kv => match kv.split_once('=') {
                Some((k, v)) => params.push((k.to_string(), v.to_string())),
                None => fail(format!("neither a file nor NAME=VALUE: {kv}")),
            },
        }
    }
    let period = period.unwrap_or_else(|| fail("--period gives the clock the design must close"));
    if verilog.is_empty() {
        fail("no Verilog file named");
    }
    let library = device_library(&verilog);
    verilog.push_str(&library);
    let ledger = Ledger::load(&ledger_path).unwrap_or_else(|e| fail(e));
    let text = std::fs::read_to_string(&calibration_path).unwrap_or_else(|e| fail(format!("{}: {e}", calibration_path.display())));
    let calibration: Calibration = serde_json::from_str(&text).unwrap_or_else(|e| fail(format!("{}: {e}", calibration_path.display())));
    if memories {
        for m in memory_modules(&verilog, top) {
            if !boxes.iter().any(|(b, _)| *b == m) {
                boxes.push((m, Cost::MEMORY));
            }
        }
    }
    let pairs: Vec<(&str, &str)> = params.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let boxed: Vec<(&str, Cost)> = boxes.iter().map(|(b, c)| (b.as_str(), *c)).collect();
    let design_of = |module: &str| -> Design {
        let graph = synthesize(&verilog, module, &pairs, &boxed, &delays).unwrap_or_else(|e| fail(format!("{module}: {e}")));
        Design::of(&graph).unwrap_or_else(|e| fail(format!("{module}: {e}")))
    };
    let design = design_of(top);
    let timed: Vec<(String, Design, u32)> =
        if parts.is_empty() { vec![(String::new(), design.clone(), 1)] } else { parts.iter().map(|(m, c)| (m.clone(), design_of(m), *c)).collect() };
    let timed: Vec<Timed> = timed.iter().map(|(name, design, cycles)| Timed { name, design, cycles: *cycles }).collect();
    let target = Target { period_ns: period, ns_per_unit: ns / delays.lut as f64 };
    let p = preflight_domains(&design.area, &timed, &ledger, &calibration, &target);
    let a = &design.area;
    println!("{top}: {} LUTs, {} carry bits, {} DSP halves, {} flip-flops, {} memory bits as mapped", a.luts, a.carry, a.half_dsps, a.flops, a.memory_bits);
    let of = |used: f64, total: u64| if total > 0 { format!("{used:.0} of {total} ({:.0}%)", 100.0 * used / total as f64) } else { format!("{used:.0}") };
    let c = p.capacity.as_ref();
    println!("  ALUTs {:.0}, ALMs {}", p.predicted.aluts, of(p.predicted.alms, c.map_or(0, |c| c.alms)));
    println!("  DSP blocks at most {}, registers {}", of(p.predicted.dsp as f64, c.map_or(0, |c| c.dsp)), of(p.predicted.registers as f64, c.map_or(0, |c| c.registers)));
    for t in &p.predicted.timing {
        println!(
            "  {}deepest path {:.2} ns, loop bound {}, against {} ns",
            if t.name.is_empty() { String::new() } else { format!("{}: ", t.name) },
            t.depth_ns,
            t.loop_ns.map_or("none".into(), |n| format!("{n:.2} ns")),
            period * t.cycles as f64
        );
    }
    if let Some(s) = p.predicted.fitter_seconds {
        println!("  fitter about {:.1} h", s / 3_600.0);
    }
    if let Some(c) = c {
        println!("  held to the PR region {} was placed in", c.row);
    }
    for t in &p.tight {
        println!("  tight: {t}");
    }
    for r in &p.refusals {
        println!("  REFUSED: {r}");
    }
    println!("{}", if p.refusals.is_empty() { "go" } else { "refused" });
    if !p.refusals.is_empty() {
        exit(1);
    }
}
