//! Prove a module of a design's Verilog the same function as another, some clocks late, over units each proven apart
//! (`logicaffeine_proof::units`): each side synthesized with every instance of its units cut out
//! (`codegen_sva::netlist::synthesize_cut`), the two decided round by round, every round's derivation checked.
//!
//!     rtl_equiv [<verilog>...] [--library <name>]... --left <top>[@<clocks>][:<NAME>=<VALUE>,...]
//!               --right <top>[@<clocks>][:<NAME>=<VALUE>,...] [--unit <left module>[@<clocks>]=<right module>[@<clocks>]]...
//!               [NAME=VALUE ...] [--zero]
//!
//! The Verilog is the files named and the decode engine's libraries `--library` names, as its generator writes them
//! now (`array`, `fp32`, `act_quant`, `attend`, `matvec`, `column`, `gdn`, `gdn_stream`, `tile_run`, `act_stream_p`,
//! `vector`, `vector_side`, `lane`, `bridge`, `weights`, `simd`, `simd_p`), each module once among them.
//! A side's clocks are the clock its outputs are read at (0, a combinational module, unless given), a unit's the
//! clocks its results follow its operands by. `NAME=VALUE` sets a parameter of both sides, a side's own follow its `:`.
//! With no unit the two are decided whole. `--zero` reads every undefined bit as 0, as a circuit that fixed them at 0
//! computes, rather than as any value. It prints the search's cost as it goes, each round's identifications, the
//! verdicts and whether every round's derivation checks; it exits 0 when every output is proven equal, 1 when one is
//! not, and 2 on a usage or synthesis error.

use std::sync::Mutex;
use std::time::Instant;

use logicaffeine_compile::codegen_sva::netlist::{synthesize_cut, Undefined};
use logicaffeine_compile::spatial::decode_engine::rtl;
use logicaffeine_proof::fraig::{self, Progress, Stats};
use logicaffeine_proof::sweep::Verdict;
use logicaffeine_proof::units::{self, Side};

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("rtl_equiv: {why}");
    std::process::exit(2)
}

static START: Mutex<Option<Instant>> = Mutex::new(None);

fn elapsed() -> f64 {
    START.lock().ok().and_then(|t| *t).map_or(0.0, |t| t.elapsed().as_secs_f64())
}

fn progress(s: &Stats) {
    eprintln!(
        "  {:.1} s: round {}, swept {} of {} ({} nodes), {} structural, {} queries, {} merges, {} counterexamples, {} unknown, {} conflicts",
        elapsed(),
        s.rounds,
        s.swept,
        s.cone,
        s.nodes,
        s.structural,
        s.queries,
        s.merges,
        s.counterexamples,
        s.unknown,
        s.conflicts
    );
}

/// The decode engine's library `name` as its generator writes it.
fn library(name: &str) -> Option<String> {
    Some(match name {
        "array" => rtl::array::array_verilog().to_string(),
        "fp32" => rtl::fp32::fp32_verilog().to_string(),
        "act_quant" => rtl::act_quant::act_quant_verilog(),
        "attend" => rtl::attend::attend_verilog(),
        "matvec" => rtl::matvec::matvec_verilog(),
        "column" => rtl::column::column_verilog(),
        "gdn" => rtl::gdn::gdn_verilog(),
        "gdn_stream" => rtl::gdn::gdn_stream_verilog(),
        "tile_run" => rtl::streaming::tile_run_verilog(),
        "act_stream_p" => rtl::streaming::act_stream_p_verilog(),
        "vector" => rtl::vector::vector_verilog(),
        "vector_side" => rtl::vector_side::vector_side_verilog(),
        "lane" => rtl::lane::lane_verilog(),
        "bridge" => rtl::engine::bridge_verilog(),
        "weights" => rtl::weights::weights_verilog().to_string(),
        "simd" => rtl::simd::simd_verilog(),
        "simd_p" => rtl::simd::simd_p_verilog(),
        _ => return None,
    })
}

/// `<name>[@<clocks>]`.
fn clocked(spec: &str) -> (String, usize) {
    match spec.split_once('@') {
        None => (spec.to_string(), 0),
        Some((name, k)) => (name.to_string(), k.parse().unwrap_or_else(|_| fail(format!("{spec}: clocks are a number")))),
    }
}

/// A side: `<top>[@<clocks>][:<NAME>=<VALUE>,...]`.
struct Spec {
    top: String,
    latency: usize,
    params: Vec<(String, String)>,
}

fn side(spec: &str) -> Spec {
    let (head, params) = spec.split_once(':').unwrap_or((spec, ""));
    let (top, latency) = clocked(head);
    let params = params
        .split(',')
        .filter(|p| !p.is_empty())
        .map(|p| p.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())).unwrap_or_else(|| fail(format!("{spec}: a parameter is NAME=VALUE"))))
        .collect();
    Spec { top, latency, params }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        fail("usage: rtl_equiv [<verilog>...] [--library <name>]... --left <top>[@<clocks>][:NAME=VALUE,...] --right <top>[@<clocks>][:NAME=VALUE,...] [--unit <left>[@<clocks>]=<right>[@<clocks>]]... [NAME=VALUE ...] [--zero]");
    }
    let (mut verilog, mut shared, mut pairs) = (String::new(), Vec::<(String, String)>::new(), Vec::<(String, usize, String, usize)>::new());
    let (mut left, mut right, mut undefined) = (None, None, Undefined::Free);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--left" => left = Some(side(it.next().unwrap_or_else(|| fail("--left takes a module")))),
            "--right" => right = Some(side(it.next().unwrap_or_else(|| fail("--right takes a module")))),
            "--unit" => {
                let u = it.next().unwrap_or_else(|| fail("--unit takes <left>[@<clocks>]=<right>[@<clocks>]"));
                let (l, r) = u.split_once('=').unwrap_or_else(|| fail(format!("--unit {u}: <left>[@<clocks>]=<right>[@<clocks>]")));
                let ((lm, lk), (rm, rk)) = (clocked(l), clocked(r));
                pairs.push((lm, lk, rm, rk));
            }
            "--zero" => undefined = Undefined::Zero,
            "--library" => {
                let name = it.next().unwrap_or_else(|| fail("--library takes a name"));
                verilog.push_str(&library(name).unwrap_or_else(|| fail(format!("no library {name}"))));
                verilog.push('\n');
            }
            file if std::path::Path::new(file).is_file() => {
                verilog.push_str(&std::fs::read_to_string(file).unwrap_or_else(|e| fail(format!("{file}: {e}"))));
                verilog.push('\n');
            }
            kv => match kv.split_once('=') {
                Some((k, v)) => shared.push((k.to_string(), v.to_string())),
                None => fail(format!("neither a file nor NAME=VALUE: {kv}")),
            },
        }
    }
    let (left, right) = (left.unwrap_or_else(|| fail("--left names a side")), right.unwrap_or_else(|| fail("--right names a side")));
    if verilog.is_empty() {
        fail("no Verilog file or library named");
    }
    *START.lock().unwrap() = Some(Instant::now());
    let cut = |spec: &Spec, modules: Vec<&str>| {
        let mut modules = modules;
        modules.sort_unstable();
        modules.dedup();
        let params: Vec<(&str, &str)> = shared.iter().chain(&spec.params).map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let netlist = synthesize_cut(&verilog, &spec.top, &params, undefined, &modules).unwrap_or_else(|e| fail(format!("{}: {e}", spec.top)));
        eprintln!(
            "{:.1} s: {} synthesized: {} inputs, {} latches, {} outputs, {} units cut out",
            elapsed(),
            spec.top,
            netlist.netlist.aig.inputs().len(),
            netlist.netlist.aig.latches().len(),
            netlist.netlist.aig.outputs().len(),
            netlist.units.len()
        );
        netlist
    };
    let lc = cut(&left, pairs.iter().map(|p| p.0.as_str()).collect());
    let rc = cut(&right, pairs.iter().map(|p| p.2.as_str()).collect());
    let left_late = |m: &str| pairs.iter().find(|p| p.0 == m).map_or(0, |p| p.1);
    let right_late = |m: &str| pairs.iter().find(|p| p.2 == m).map_or(0, |p| p.3);
    let (lu, ru) = (lc.units(&left_late, &["clk"]), rc.units(&right_late, &["clk"]));
    let functions: Vec<(String, String)> = pairs.iter().map(|p| (p.0.clone(), p.2.clone())).collect();
    let ls = Side { aig: &lc.netlist.aig, latency: left.latency, units: &lu };
    let rs = Side { aig: &rc.netlist.aig, latency: right.latency, units: &ru };
    let opts = fraig::Options { progress: Some(Progress { every: 200_000, report: progress }), ..fraig::Options::default() };
    let report = units::decide(&ls, &rs, &functions, &opts).unwrap_or_else(|e| fail(format!("the two sides do not pair: {e:?}")));
    let decided = elapsed();
    let at = |a: &units::Application| format!("{}:{}@{}", ["left", "right"][a.side], [&lu, &ru][a.side][a.unit].name, a.frame);
    for (k, round) in report.rounds.iter().enumerate() {
        let names: Vec<String> = round.substituted.iter().map(|(kept, made)| format!("{} = {}", at(made), at(kept))).collect();
        println!("round {}: {} identified, derivation {} steps{}", k + 1, names.len(), round.proof.steps.len(), if names.is_empty() { String::new() } else { format!(": {}", names.join(", ")) });
    }
    let equal = report.verdicts.iter().filter(|(_, v)| *v == Verdict::Equal).count();
    println!("{equal} of {} outputs equal in {decided:.1} s", report.verdicts.len());
    for (name, v) in report.verdicts.iter().filter(|(_, v)| *v != Verdict::Equal) {
        println!("  {name}: {v:?}");
    }
    let checked = report.check(&ls, &rs, &functions);
    println!("every round's derivation and identification {} ({:.1} s)", if checked { "checks" } else { "DOES NOT CHECK" }, elapsed() - decided);
    if !checked || equal < report.verdicts.len() {
        std::process::exit(1);
    }
}
