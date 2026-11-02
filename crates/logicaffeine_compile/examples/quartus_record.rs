//! Record one Quartus Prime compile as a row of a build ledger (`codegen_sva::quartus::Ledger`), judged on what it
//! ran.
//!
//!     quartus_record [evidence] <reports dir> <revision> <name> <ledger.json> [<input file>...]
//!     quartus_record --again [evidence] <reports dir> <revision> <name> <ledger.json>
//!     quartus_record --judge [evidence] <name> <ledger.json>
//!
//! `<reports dir>` holds `<revision>.flow.rpt` and whatever other reports the compile wrote: a project's
//! `output_files`, or the project directory itself. The row carries the SHA-256 of each input file named and the
//! time it was recorded. A compile that failed is recorded like one that passed; a name already in the ledger is
//! refused.
//!
//! `--again` reads a recorded compile's reports anew, under its name and with its inputs, which must hash as they did
//! when it was recorded. `--judge` weighs a recorded compile's reports, as recorded, against new evidence.
//!
//! The evidence, each optional: `--exit <status>` the flow command's exit status; `--log <file>` the flow's log;
//! `--expect <module>,...` the flow modules the job runs, as the flow report names them; `--started <seconds>` when
//! the job started, in seconds since the epoch, against which the flow report's own write time is held.

use std::path::Path;
use std::process::exit;
use std::time::{SystemTime, UNIX_EPOCH};

use logicaffeine_compile::codegen_sva::quartus::{self, judge, Evidence, Input, Ledger, Row};
use logicaffeine_compile::crypto_accel::sha256;

const USAGE: &str = "usage: quartus_record [evidence] <reports dir> <revision> <name> <ledger.json> [<input file>...]
       quartus_record --again [evidence] <reports dir> <revision> <name> <ledger.json>
       quartus_record --judge [evidence] <name> <ledger.json>
evidence: --exit <status> --log <file> --expect <module>,... --started <seconds since the epoch>";

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("quartus_record: {why}");
    exit(1)
}

fn usage() -> ! {
    eprintln!("{USAGE}");
    exit(2)
}

fn hash(path: &str) -> String {
    let bytes = std::fs::read(path).unwrap_or_else(|e| fail(format!("{path}: {e}")));
    sha256(&bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The evidence flags at the front of `args`, the compile the job resumed (`--resumed`), and the arguments after them.
fn evidence(mut args: &[String]) -> (Evidence, Option<String>, &[String]) {
    let mut e = Evidence::default();
    let mut resumed = None;
    while let [flag, value, rest @ ..] = args {
        match flag.as_str() {
            "--resumed" => resumed = Some(value.clone()),
            "--exit" => e.exit = Some(value.parse().unwrap_or_else(|_| fail(format!("an exit status is a number: {value}")))),
            "--log" => e.log = Some(String::from_utf8_lossy(&std::fs::read(value).unwrap_or_else(|err| fail(format!("{value}: {err}")))).into_owned()),
            "--expect" => e.expected = value.split(',').map(str::trim).filter(|m| !m.is_empty()).map(str::to_string).collect(),
            "--started" => e.started = Some(value.parse().unwrap_or_else(|_| fail(format!("a start time is seconds: {value}")))),
            _ => break,
        }
        args = rest;
    }
    (e, resumed, args)
}

/// When `<revision>.flow.rpt` under `dir` was last written.
fn flow_written(dir: &Path, revision: &str) -> Option<u64> {
    let modified = std::fs::metadata(dir.join(format!("{revision}.flow.rpt"))).and_then(|m| m.modified()).ok()?;
    modified.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

fn summary(row: &Row) -> String {
    let b = &row.build;
    let verdict = if row.verdict.passed { "passed".to_string() } else { format!("FAILED: {}", row.verdict.failures.join("; ")) };
    format!(
        "{}: {verdict} | ALMs {} | DSP {} | worst setup {} | fitter {} | peak {}",
        row.name,
        b.fit.as_ref().and_then(|f| f.alms).map_or("-".into(), |r| format!("{}/{}", r.used, r.total)),
        b.fit.as_ref().and_then(|f| f.dsp_blocks).map_or("-".into(), |r| format!("{}/{}", r.used, r.total)),
        b.worst_setup().map_or("-".into(), |s| format!("{:.3} ns on {}", s.slack_ns, s.clock)),
        b.flow.phase("Fitter").map_or("-".into(), |p| format!("{} s", p.seconds)),
        b.flow.phases.iter().filter_map(|p| p.peak_mb).max().map_or("-".into(), |m| format!("{m} MB")),
    )
}

fn main() {
    let all: Vec<String> = std::env::args().skip(1).collect();
    let (mode, rest) = match all.first().map(String::as_str) {
        Some("--again") => ("again", &all[1..]),
        Some("--judge") => ("judge", &all[1..]),
        _ => ("record", &all[..]),
    };
    let (mut evidence, resumed, args) = evidence(rest);
    let row = match (mode, args) {
        ("judge", [name, ledger_path]) => {
            let ledger = Ledger::load(Path::new(ledger_path)).unwrap_or_else(|e| fail(e));
            let mut row = ledger.row(name).cloned().unwrap_or_else(|| fail(format!("no compile named {name} is recorded")));
            row.verdict = judge(&row.build, &evidence);
            (row, ledger_path, true)
        }
        ("again", [dir, revision, name, ledger_path]) => {
            let ledger = Ledger::load(Path::new(ledger_path)).unwrap_or_else(|e| fail(e));
            let old = ledger.row(name).unwrap_or_else(|| fail(format!("no compile named {name} is recorded")));
            for input in &old.inputs {
                if hash(&input.path) != input.sha256 {
                    fail(format!("{} has changed since {name} was recorded; its reports are another compile's", input.path));
                }
            }
            let build = quartus::read(Path::new(dir), revision).unwrap_or_else(|e| fail(e));
            evidence.flow_written = flow_written(Path::new(dir), revision);
            let verdict = judge(&build, &evidence);
            let resumed = resumed.or_else(|| old.resumed.clone());
            (Row { name: name.clone(), recorded: quartus::utc(now()), inputs: old.inputs.clone(), build, verdict, resumed }, ledger_path, true)
        }
        ("record", [dir, revision, name, ledger_path, inputs @ ..]) => {
            let build = quartus::read(Path::new(dir), revision).unwrap_or_else(|e| fail(e));
            evidence.flow_written = flow_written(Path::new(dir), revision);
            let verdict = judge(&build, &evidence);
            let inputs = inputs.iter().map(|p| Input { path: p.clone(), sha256: hash(p) }).collect();
            (Row { name: name.clone(), recorded: quartus::utc(now()), inputs, build, verdict, resumed }, ledger_path, false)
        }
        _ => usage(),
    };
    let (row, ledger_path, replace) = row;
    let mut ledger = Ledger::load(Path::new(ledger_path)).unwrap_or_else(|e| fail(e));
    let line = summary(&row);
    if replace { ledger.replace(row) } else { ledger.record(row) }.unwrap_or_else(|e| fail(e));
    ledger.save(Path::new(ledger_path)).unwrap_or_else(|e| fail(e));
    println!("{line}");
}
