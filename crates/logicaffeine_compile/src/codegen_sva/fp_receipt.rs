//! The card's measurement of its floating-point DSP block as a receipt (`benchmarks/silicon/d5005/fp_hard_receipts.json`):
//! what the oracle persona ([`super::fp_oracle`]) measured on the card, tied to the build that compiled it and the image
//! the card ran, with what each mode's block is for the engine ([`super::fp_hard`]). The block is the vendor's and
//! encrypted, so a receipt is a measurement recorded as one; the proofs hold everything around it.
//!
//! A receipt reads ([`receipt`]) only when:
//! - the build is a row of the build ledger that passed, naming the engine it compiled (`hw/rtl/logos_engine.v`, by
//!   SHA-256);
//! - the card's run configured the image (`gbs.sha256`, the run's own hash of what it loaded), partial reconfiguration
//!   reported no error (`PR_ERR` all zero), the user clock was set, and the run finished no earlier than it started;
//! - the measurement finished.
//!
//! Each mode is its depths, the one class that agreed with the block on every step (or how many did, and the steps no
//! class explains), the classes that differed on the fewest steps, and the engine's unit at its latency on that block:
//! the wrapper's build ([`plan`]) or why there is none. The multiply-add is its depth and its rounding.
//!
//! [`record`] refuses besides a receipt whose discriminator ledger does not regenerate the engine the build compiled
//! (the persona's oracle is that ledger's classes and vectors), and a measurement the ledger already holds. Its inputs
//! are kept beside the ledger ([`KEPT`] in [`kept_dir`], each as [`kept`] keeps it), so every receipt reads again from
//! them.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::fp_hard::{plan, Block, MeasureError, Measurement, Op, Registering, Rounding};
use super::fp_oracle::{Class, Oracle, OracleError};
use crate::crypto_accel::sha256;

/// How many of the classes nearest the block a mode keeps.
pub const NEAREST: usize = 3;

/// The inputs a receipt keeps beside the ledger, each under the name the card's run wrote it (the discriminator ledger
/// as `discriminators.json`), in [`kept_dir`].
pub const KEPT: [&str; 8] = ["measurement.json", "discriminators.json", "gbs.sha256", "pr.log", "userclk.log", "started", "finished", "logos_engine.v"];

/// The receipt that records `measurement`, by its SHA-256: the card run a hard-float build takes its blocks from, read
/// against the discriminator ledger kept beside it (`fp_receipts/<kept_dir>/discriminators.json`).
pub fn receipt_of<'a>(ledger: &'a Ledger, measurement: &str) -> Option<&'a Receipt> {
    let sha = hex(&sha256(measurement.as_bytes()));
    ledger.receipts.iter().find(|r| r.measurement_sha256 == sha)
}

/// A card run's file `name` as the ledger keeps it: as the run wrote it, but for a `sha256sum` line (`gbs.sha256`),
/// which names its image by its file name alone, so no kept input names one machine's directories.
pub fn kept(name: &str, text: &str) -> String {
    if name != "gbs.sha256" {
        return text.to_string();
    }
    let lines: Vec<String> = text
        .lines()
        .map(|l| match l.split_once("  ") {
            Some((hash, path)) => format!("{hash}  {}", path.rsplit('/').next().unwrap_or(path)),
            None => l.to_string(),
        })
        .collect();
    lines.join("\n") + if text.ends_with('\n') { "\n" } else { "" }
}

/// Where a receipt's inputs are kept, under `fp_receipts/` beside the ledger: the build and the measurement's first
/// twelve hex digits, one directory a measurement.
pub fn kept_dir(receipt: &Receipt) -> String {
    format!("{}-{}", receipt.build, &receipt.measurement_sha256[..12])
}

/// What the card's run left beside its measurement, each file as the run wrote it.
#[derive(Clone, Copy, Debug)]
pub struct CardRun<'a> {
    /// `gbs.sha256`: `sha256sum` of the image the run configured.
    pub gbs_sha256: &'a str,
    /// `pr.log`: the FME's partial-reconfiguration status and error registers after configuring.
    pub pr: &'a str,
    /// `userclk.log`: the user clock the run set.
    pub user_clock: &'a str,
    /// `started` and `finished`: the measurement's start and end, `YYYY-MM-DDTHH:MM:SSZ`.
    pub started: &'a str,
    pub finished: &'a str,
    /// `measurement.json`: the host program's output ([`super::fp_oracle::HOST_PROGRAM`]).
    pub measurement: &'a str,
}

/// Everything a receipt is read from.
#[derive(Clone, Copy, Debug)]
pub struct Inputs<'a> {
    /// The build's name, a row of `builds`.
    pub build: &'a str,
    /// The build ledger (`benchmarks/silicon/d5005/quartus_builds.json`).
    pub builds: &'a str,
    /// The discriminator ledger the persona's oracle was made from.
    pub discriminators: &'a str,
    /// The SHA-256 of the image, hex.
    pub image_sha256: &'a str,
    /// The engine's multiply and add latencies the units are planned at.
    pub latencies: [usize; 2],
    pub card: CardRun<'a>,
}

/// The engine's unit on one mode's block.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    /// The wrapper at `latency`: the block registered fully or with its output register alone, the registers after it,
    /// and what the logic around it corrects.
    Hard { latency: usize, registering: String, pads: usize, flush_operands: bool, flush_results: bool, canonical_nan: bool },
    /// No wrapper at `latency`, and why.
    Refused { latency: usize, why: String },
}

/// One class and the steps on which it differed from the block over every run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Near {
    pub class: String,
    pub differing: u64,
}

/// One mode of the block as the card measured it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mode {
    /// Its depth fully registered and with its output register alone.
    pub full: usize,
    pub least: usize,
    /// How many classes agreed with the block on every step, and the members of the one that did (empty unless one).
    pub agreeing: usize,
    pub class: Vec<String>,
    /// The steps no class explains.
    pub unexplained: u64,
    /// The [`NEAREST`] classes that differed on the fewest steps, fewest first, ties in the family's order; each by its
    /// first member.
    pub nearest: Vec<Near>,
    pub unit: Unit,
}

/// The steps of each run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Steps {
    pub vectors: u64,
    pub random: u64,
    pub near: u64,
}

/// A receipt: the run's identity, the measurement's size, and each mode.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    pub recorded: String,
    pub build: String,
    pub gbs_sha256: String,
    pub engine_sha256: String,
    pub discriminators_sha256: String,
    pub measurement_sha256: String,
    pub started: String,
    pub finished: String,
    pub user_clock_mhz: f64,
    pub seed: u64,
    pub steps: Steps,
    pub product_classes: usize,
    pub sum_classes: usize,
    pub multiply: Mode,
    pub add: Mode,
    pub multiply_add_latency: usize,
    pub multiply_add_rounding: String,
}

/// The receipts ledger.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ledger {
    #[serde(rename = "_what")]
    pub what: String,
    pub receipts: Vec<Receipt>,
}

/// What the receipts ledger says of itself when it starts.
pub const LEDGER_WHAT: &str = "The D5005's floating-point DSP block as the card measured it: each receipt one run of the oracle \
persona (codegen_sva::fp_oracle) on the image the build compiled, recorded by the d5005_fp_receipt example only when the \
build passed, its discriminator ledger regenerates the engine it compiled, the card configured that image without error \
and the measurement finished. Its inputs are kept beside it in fp_receipts/<build>-<measurement>/.";

/// Why a receipt is not read or not recorded.
#[derive(Clone, Debug, PartialEq)]
pub enum ReceiptError {
    /// The build ledger or the receipts ledger is not one.
    Ledger(String),
    /// The build ledger has no row of this name.
    NoBuild(String),
    /// The build did not pass.
    BuildFailed(String),
    /// The build names no one engine it compiled.
    NoEngine(String),
    /// The engine given is not the one the build compiled: its SHA-256 and the build's.
    Engine { build: String, compiled: String, kept: String },
    /// A module the oracle writes itself that the discriminator ledger does not regenerate as the build compiled it.
    Regenerated { build: String, module: String },
    Oracle(OracleError),
    /// The run configured another image than the one given.
    Image { configured: String, given: String },
    /// Partial reconfiguration reported an error, or the run left no error register.
    Reconfiguration(String),
    /// The run set no user clock.
    UserClock,
    /// A time not `YYYY-MM-DDTHH:MM:SSZ`, or a run that finished before it started.
    Times { started: String, finished: String },
    Measure(MeasureError),
    /// The ledger already holds a receipt of this measurement.
    Duplicate(String),
}

impl fmt::Display for ReceiptError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ReceiptError::Ledger(e) => write!(f, "a ledger: {e}"),
            ReceiptError::NoBuild(b) => write!(f, "the build ledger has no row {b}"),
            ReceiptError::BuildFailed(b) => write!(f, "the build {b} did not pass"),
            ReceiptError::NoEngine(b) => write!(f, "the build {b} names no one hw/rtl/logos_engine.v among its inputs"),
            ReceiptError::Engine { build, compiled, kept } => write!(
                f,
                "the engine given is {}, the build {build} compiled {}",
                &kept[..12.min(kept.len())],
                &compiled[..12.min(compiled.len())]
            ),
            ReceiptError::Regenerated { build, module } => {
                write!(f, "the discriminator ledger does not regenerate {module} as the build {build} compiled it")
            }
            ReceiptError::Oracle(e) => write!(f, "{e}"),
            ReceiptError::Image { configured, given } => write!(f, "the card configured the image {configured}, not {given}"),
            ReceiptError::Reconfiguration(why) => write!(f, "partial reconfiguration: {why}"),
            ReceiptError::UserClock => write!(f, "the run set no user clock"),
            ReceiptError::Times { started, finished } => write!(f, "the run started {started:?} and finished {finished:?}"),
            ReceiptError::Measure(e) => write!(f, "{e}"),
            ReceiptError::Duplicate(sha) => write!(f, "the ledger already holds a receipt of the measurement {}", &sha[..12.min(sha.len())]),
        }
    }
}

/// The receipt `inputs` make, as recorded at `recorded`.
pub fn receipt(inputs: &Inputs, recorded: &str) -> Result<Receipt, ReceiptError> {
    let engine_sha256 = compiled(inputs.builds, inputs.build)?;
    let oracle = Oracle::from_ledger(inputs.discriminators).map_err(ReceiptError::Oracle)?;
    let card = &inputs.card;
    let image = configured(card.gbs_sha256);
    if image != Some(inputs.image_sha256) {
        let configured = image.map_or_else(|| card.gbs_sha256.lines().next().unwrap_or("").to_string(), str::to_string);
        return Err(ReceiptError::Image { configured, given: inputs.image_sha256.to_string() });
    }
    reconfiguration(card.pr).map_err(ReceiptError::Reconfiguration)?;
    let user_clock_mhz = user_clock_mhz(card.user_clock).ok_or(ReceiptError::UserClock)?;
    let (started, finished) = (card.started.trim(), card.finished.trim());
    if !time(started) || !time(finished) || finished < started {
        return Err(ReceiptError::Times { started: started.to_string(), finished: finished.to_string() });
    }
    let m = card.measurement;
    let (seed, [vectors, random, near]) = Measurement::read_runs(m).map_err(ReceiptError::Measure)?;
    let [multiply, add] = [(Op::Multiply, inputs.latencies[0]), (Op::Add, inputs.latencies[1])].map(|(op, latency)| mode(&oracle, m, op, latency));
    let (multiply_add_latency, rounding) = Measurement::read_multiply_add(m).map_err(ReceiptError::Measure)?;
    Ok(Receipt {
        recorded: recorded.to_string(),
        build: inputs.build.to_string(),
        gbs_sha256: inputs.image_sha256.to_string(),
        engine_sha256,
        discriminators_sha256: hex(&sha256(inputs.discriminators.as_bytes())),
        measurement_sha256: hex(&sha256(m.as_bytes())),
        started: started.to_string(),
        finished: finished.to_string(),
        user_clock_mhz,
        seed,
        steps: Steps { vectors, random, near },
        product_classes: oracle.products.len(),
        sum_classes: oracle.sums.len(),
        multiply: multiply?,
        add: add?,
        multiply_add_latency,
        multiply_add_rounding: match rounding {
            Rounding::Once => "once",
            Rounding::Twice => "twice",
            Rounding::Other => "other",
        }
        .to_string(),
    })
}

/// Whether `engine` is the engine the build compiled and was made of `discriminators`: its SHA-256 the build row's
/// `hw/rtl/logos_engine.v`, and every module of it the oracle writes itself ([`Oracle::own_verilog`]) the text the oracle
/// of `discriminators` writes now, byte for byte. A receipt is recorded under it; kept, the engine is held by its SHA-256
/// thereafter, so the oracle's code may move on. The library the engine embeds (the engine's crossing and ports, the
/// store's memory) is held by the build's SHA-256 alone, so the library may move on between a build and its receipt; an
/// own module that does not regenerate is refused by name.
pub fn engine_held(build: &str, builds: &str, discriminators: &str, engine: &str) -> Result<(), ReceiptError> {
    let compiled = compiled(builds, build)?;
    let kept = hex(&sha256(engine.as_bytes()));
    if kept != compiled {
        return Err(ReceiptError::Engine { build: build.to_string(), compiled, kept });
    }
    let oracle = Oracle::from_ledger(discriminators).map_err(ReceiptError::Oracle)?;
    let own = oracle.own_verilog();
    let built = super::netlist::modules(engine).unwrap_or_default();
    for (module, text) in super::netlist::modules(&own).unwrap_or_default() {
        if !built.iter().any(|(m, t)| *m == module && *t == text) {
            return Err(ReceiptError::Regenerated { build: build.to_string(), module });
        }
    }
    Ok(())
}

/// [`receipt`], refused unless `engine` is held ([`engine_held`]) and `ledger` (none: a new ledger) holds no receipt of
/// the same measurement; with the ledger it is appended to, as written.
pub fn record(ledger: Option<&str>, inputs: &Inputs, engine: &str, recorded: &str) -> Result<(Receipt, String), ReceiptError> {
    let receipt = receipt(inputs, recorded)?;
    engine_held(inputs.build, inputs.builds, inputs.discriminators, engine)?;
    let mut ledger = match ledger {
        Some(text) => serde_json::from_str::<Ledger>(text).map_err(|e| ReceiptError::Ledger(e.to_string()))?,
        None => Ledger { what: LEDGER_WHAT.to_string(), receipts: Vec::new() },
    };
    if ledger.receipts.iter().any(|r| r.measurement_sha256 == receipt.measurement_sha256) {
        return Err(ReceiptError::Duplicate(receipt.measurement_sha256));
    }
    ledger.receipts.push(receipt.clone());
    let mut text = Vec::new();
    let mut writer = serde_json::Serializer::with_formatter(&mut text, serde_json::ser::PrettyFormatter::with_indent(b" "));
    ledger.serialize(&mut writer).map_err(|e| ReceiptError::Ledger(e.to_string()))?;
    text.push(b'\n');
    Ok((receipt, String::from_utf8(text).expect("JSON is UTF-8")))
}

/// The engine the build compiled, by its row in the build ledger: a row that passed, naming one `hw/rtl/logos_engine.v`.
fn compiled(builds: &str, build: &str) -> Result<String, ReceiptError> {
    let builds: serde_json::Value = serde_json::from_str(builds).map_err(|e| ReceiptError::Ledger(e.to_string()))?;
    let rows = builds["rows"].as_array().ok_or_else(|| ReceiptError::Ledger("the build ledger has no rows".to_string()))?;
    let row = rows.iter().find(|r| r["name"] == build).ok_or_else(|| ReceiptError::NoBuild(build.to_string()))?;
    if row["verdict"]["passed"] != true {
        return Err(ReceiptError::BuildFailed(build.to_string()));
    }
    let engines: Vec<&str> = row["inputs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| i["path"].as_str().is_some_and(|p| p.ends_with("/hw/rtl/logos_engine.v")))
        .filter_map(|i| i["sha256"].as_str())
        .collect();
    match engines[..] {
        [engine] => Ok(engine.to_string()),
        _ => Err(ReceiptError::NoEngine(build.to_string())),
    }
}

/// One mode as the measurement found it, and the engine's unit at `latency` on its block.
fn mode(oracle: &Oracle, m: &str, op: Op, latency: usize) -> Result<Mode, ReceiptError> {
    let tally = Measurement::read_tally(oracle, m, op).map_err(ReceiptError::Measure)?;
    let (full, least) = Measurement::read_depths(m, op).map_err(ReceiptError::Measure)?;
    let classes = match op {
        Op::Multiply => &oracle.products,
        Op::Add => &oracle.sums,
    };
    let read = match op {
        Op::Multiply => Measurement::read_product(oracle, m),
        Op::Add => Measurement::read_sum(oracle, m),
    };
    let (class, unit) = match read {
        Ok(block) => {
            let class = classes.iter().find(|c| c.semantics == block.semantics).map(|c| c.members.clone()).unwrap_or_default();
            (class, unit(op, &block, latency))
        }
        Err(e @ MeasureError::Class { .. }) => (Vec::new(), Unit::Refused { latency, why: e.to_string() }),
        Err(e) => return Err(ReceiptError::Measure(e)),
    };
    Ok(Mode {
        full,
        least,
        agreeing: tally.differing.iter().filter(|&&d| d == 0).count(),
        class,
        unexplained: tally.unexplained,
        nearest: nearest(classes, &tally.differing),
        unit,
    })
}

/// The engine's unit at `latency` on `block`.
fn unit(op: Op, block: &Block, latency: usize) -> Unit {
    match plan(op, block, latency) {
        Ok(p) => Unit::Hard {
            latency,
            registering: match p.registering {
                Registering::Full => "full",
                Registering::Least => "least",
            }
            .to_string(),
            pads: p.pads,
            flush_operands: p.flush_operands,
            flush_results: p.flush_results,
            canonical_nan: p.canonical_nan,
        },
        Err(e) => Unit::Refused { latency, why: e.to_string() },
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The image a run configured: the first word of its `sha256sum` line, 64 lower-case hex digits.
fn configured(line: &str) -> Option<&str> {
    let word = line.split_whitespace().next()?;
    (word.len() == 64 && word.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))).then_some(word)
}

/// Whether partial reconfiguration reported no error: every `PR_ERR` line's register and each of its fields zero.
fn reconfiguration(pr: &str) -> Result<(), String> {
    let lines: Vec<&str> = pr.lines().filter(|l| l.starts_with("PR_ERR")).collect();
    if lines.is_empty() {
        return Err("the run left no PR_ERR register".to_string());
    }
    for line in lines {
        let (register, fields) = line["PR_ERR".len()..].split_once(':').ok_or_else(|| format!("unread: {line}"))?;
        let raw = register.trim().strip_prefix("0x").and_then(|h| u64::from_str_radix(h, 16).ok()).ok_or_else(|| format!("unread: {line}"))?;
        let words: Vec<&str> = fields.split_whitespace().collect();
        if words.is_empty() || words.len() % 2 != 0 || words.chunks(2).any(|f| f[1].parse::<u64>().is_err()) {
            return Err(format!("unread: {line}"));
        }
        let set: Vec<String> = words.chunks(2).filter(|f| f[1].parse::<u64>() != Ok(0)).map(|f| f.join(" ")).collect();
        if raw != 0 || !set.is_empty() {
            return Err(format!("PR_ERR {raw:#x}: {}", set.join(", ")));
        }
    }
    Ok(())
}

/// The user clock a run left set: the last `High clock = <MHz> MHz` it reported.
fn user_clock_mhz(log: &str) -> Option<f64> {
    log.lines()
        .filter_map(|l| l.trim().strip_prefix("High clock"))
        .filter_map(|r| r.trim_start().strip_prefix('=')?.trim().strip_suffix("MHz")?.trim().parse::<f64>().ok())
        .last()
}

/// Whether `t` is `YYYY-MM-DDTHH:MM:SSZ`.
fn time(t: &str) -> bool {
    let b = t.as_bytes();
    b.len() == 20
        && b.iter().enumerate().all(|(i, &c)| match i {
            4 | 7 => c == b'-',
            10 => c == b'T',
            13 | 16 => c == b':',
            19 => c == b'Z',
            _ => c.is_ascii_digit(),
        })
}

/// The [`NEAREST`] classes with the fewest differing steps, fewest first, ties in the classes' order.
fn nearest(classes: &[Class], differing: &[u64]) -> Vec<Near> {
    let mut order: Vec<usize> = (0..classes.len().min(differing.len())).collect();
    order.sort_by_key(|&k| (differing[k], k));
    order.into_iter().take(NEAREST).map(|k| Near { class: classes[k].members[0].clone(), differing: differing[k] }).collect()
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/fp_receipt/tests.rs"]
mod tests;
