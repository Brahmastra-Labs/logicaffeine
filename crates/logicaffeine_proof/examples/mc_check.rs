//! Decide the safety of AIGER circuits, certified: by the race of PDR and bounded model checking
//! ([`logicaffeine_proof::mc`], the default), by property-directed reachability alone ([`logicaffeine_proof::pdr`]), by
//! PDR on localization abstractions ([`logicaffeine_proof::localize`], `--engine loc`) or by interpolation
//! ([`logicaffeine_proof::itp`]), or look for a run to a bad state of at most `--max-depth` clocks by
//! bounded model checking alone ([`logicaffeine_proof::itp::bmc_interleaved`]: a clock at a time and a window at a time
//! by turns; `--shortest` a clock at a time alone, [`logicaffeine_proof::itp::bmc`], `--window <n>` windows of `n` clocks
//! alone, [`logicaffeine_proof::itp::bmc_windowed`]): a line per file with the verdict (`safe`, `unsafe`,
//! or `unknown`, and why when the engine says), its certificate's size (the invariant's clauses for PDR, the depth of
//! its induction, or the run's clocks), the engine that answered, the latches searched over, what the search cost, and
//! the seconds. PDR asks the induction step up to depth 4 unless `--induction` says otherwise (0 asks none), and
//! `--engine loc` decides each cone reduced by its signal correspondence ([`logicaffeine_proof::scorr`]) unless
//! `--no-correspond`.
//!
//!     mc_check [--engine mc|pdr|loc|itp|bmc] [--max-frames <n>] [--max-obligations <n>] [--ctgs <n>] [--strict-lifting]
//!              [--no-correspond] [--induction <n>] [--induction-conflicts <n>] [--max-depth <n>]
//!              [--shortest | --window <n>] [--max-rounds <n>] <file.aig>...

use std::time::Instant;

use logicaffeine_proof::aig::Aig;
use logicaffeine_proof::itp::{self, ItpOptions, ItpVerdict};
use logicaffeine_proof::localize;
use logicaffeine_proof::mc;
use logicaffeine_proof::pdr::{self, Stats, Verdict};

/// A PDR verdict and its search's cost as the line reports them.
fn pdr_line(verdict: &Verdict, s: &Stats) -> String {
    let verdict = match verdict {
        Verdict::Safe(_) => format!("safe invariant={}", s.invariant),
        Verdict::Induction(k) => format!("safe induction={k}"),
        Verdict::Unsafe(trace) => format!("unsafe clocks={}", trace.inputs.len()),
        Verdict::Unknown(why) => format!("unknown ({why})"),
    };
    let cost = format!("frames={} obligations={} queries={}", s.frames, s.obligations, s.queries);
    format!("{verdict} searched={} {cost}", s.latches)
}

/// The induction step PDR asks unless told otherwise: up to depth 4 within 100,000 conflicts a depth, which on the
/// HWMCC'24 bit-level track at 60 s of CPU decides 121 of the 319 designs against 114 without it.
const INDUCTION: usize = 4;
const INDUCTION_CONFLICTS: u64 = 100_000;

/// Whether `--engine loc` reduces a cone by its signal correspondence first unless told otherwise.
const CORRESPOND: bool = true;

/// `text`, the value of `flag`, as a number, or the usage error.
fn number(flag: &str, text: &str) -> usize {
    text.parse().unwrap_or_else(|_| {
        eprintln!("mc_check: {flag} takes a number");
        std::process::exit(2)
    })
}

fn main() {
    let mut pdr_opts =
        pdr::Options { induction: INDUCTION, induction_conflicts: INDUCTION_CONFLICTS, ..pdr::Options::default() };
    let mut itp_opts = ItpOptions::default();
    let (mut shortest, mut window) = (false, 1);
    let mut correspond = CORRESPOND;
    let (mut engine, mut files) = (String::from("mc"), Vec::new());
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = |flag: &str| {
            args.next().unwrap_or_else(|| {
                eprintln!("mc_check: {flag} takes a value");
                std::process::exit(2)
            })
        };
        match a.as_str() {
            "--engine" => engine = value("--engine"),
            "--max-frames" => pdr_opts.max_frames = number("--max-frames", &value("--max-frames")),
            "--max-obligations" => pdr_opts.max_obligations = number("--max-obligations", &value("--max-obligations")),
            "--ctgs" => pdr_opts.ctgs = number("--ctgs", &value("--ctgs")),
            "--strict-lifting" => pdr_opts.assume_constraints = false,
            "--no-correspond" => correspond = false,
            "--induction" => pdr_opts.induction = number("--induction", &value("--induction")),
            "--induction-conflicts" => {
                pdr_opts.induction_conflicts = number("--induction-conflicts", &value("--induction-conflicts")) as u64
            }
            "--max-depth" => itp_opts.max_depth = number("--max-depth", &value("--max-depth")),
            "--shortest" => shortest = true,
            "--window" => window = number("--window", &value("--window")).max(1),
            "--max-rounds" => itp_opts.max_rounds = number("--max-rounds", &value("--max-rounds")),
            _ => files.push(a),
        }
    }
    if files.is_empty() || !matches!(engine.as_str(), "mc" | "pdr" | "loc" | "itp" | "bmc") {
        eprintln!(
            "usage: mc_check [--engine mc|pdr|loc|itp|bmc] [--max-frames <n>] [--max-obligations <n>] [--ctgs <n>] \
             [--strict-lifting] [--no-correspond] [--induction <n>] [--induction-conflicts <n>] [--max-depth <n>] \
             [--shortest | --window <n>] [--max-rounds <n>] <file.aig>..."
        );
        std::process::exit(2);
    }
    for path in &files {
        let read = std::fs::read(path).map_err(|e| e.to_string());
        let aig = match read.and_then(|b| Aig::parse(&b).map_err(|e| format!("{e:?}"))) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("{path}: {e}");
                continue;
            }
        };
        let start = Instant::now();
        let line = if engine == "mc" {
            let out = mc::prove_safe(&aig, &mc::Options { pdr: pdr_opts, ..mc::Options::default() });
            format!("{} engine={:?}", pdr_line(&out.verdict, &out.stats), out.engine)
        } else if engine == "pdr" {
            let report = pdr::prove_safe(&aig, &pdr_opts);
            pdr_line(&report.verdict, &report.stats)
        } else if engine == "loc" {
            let report =
                localize::prove_safe(&aig, &localize::Options { pdr: pdr_opts, correspond, ..localize::Options::default() });
            format!("{} kept={} refinements={}", pdr_line(&report.verdict, &report.stats), report.kept, report.refinements)
        } else if engine == "bmc" {
            let found = if shortest {
                itp::bmc(&aig, itp_opts.max_depth)
            } else if window > 1 {
                itp::bmc_windowed(&aig, itp_opts.max_depth, window)
            } else {
                itp::bmc_interleaved(&aig, itp_opts.max_depth)
            };
            match found {
                Some(trace) => format!("unsafe clocks={}", trace.inputs.len()),
                None => format!("unknown (no run within {} clocks)", itp_opts.max_depth),
            }
        } else {
            match itp::prove_safe(&aig, &itp_opts) {
                ItpVerdict::Safe(_) => "safe".to_string(),
                ItpVerdict::Unsafe(trace) => format!("unsafe clocks={}", trace.inputs.len()),
                ItpVerdict::Unknown => "unknown".to_string(),
            }
        };
        println!(
            "{path} {line} seconds={:.3} latches={} ands={}",
            start.elapsed().as_secs_f64(),
            aig.latches().len(),
            aig.num_ands()
        );
    }
}
