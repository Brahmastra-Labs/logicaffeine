//! Size the pipeline a combinational module needs at each of several periods: the least latency, the register bits
//! at it, and the named nets to cut, from its Verilog through `codegen_sva::timing_graph` and `codegen_sva::autopipe`.
//!
//!     autopipe_report <verilog> <top> <period>,... [NAME=VALUE ...] [--delays L,C,D,M] [--cuts named|nets]
//!                     [--outputs registered] [--emit <dir>] [--enable <port>] [--prove [--sweep]] [--aiger <dir>]
//!
//! A period is in the timing model's unit: `Delays::LEVELS` (a LUT is 100) unless `--delays` gives the LUT, carry,
//! DSP and memory delays. `--cuts` says where registers may go:
//! on the nets the source names (the default, the cuts to write into it by hand) or on any net. `--outputs
//! registered` puts one more register on every output, a clock later, so the outputs are read straight from
//! registers. A module that is a pipeline already is cut afresh at its own latency (`autopipe::unpipelined`), its
//! outputs kept on registers when they were, or told the latency the period needs past it. `--emit` writes each
//! pipeline as `<dir>/<top>_p<latency>.v`, its registers stepping under `--enable` when one is named; `--prove` then
//! proves it the module that many clocks late (`logicaffeine_proof::seq_equiv`, a pipeline against the pipeline it was
//! cut from, each from any state, the source's undefined bits read as 0), and prints the proof's verdicts, its
//! certificate's check and both texts' SHA-256. The proof is by incremental SAT sweeping (`fraig`), its progress
//! printed as it goes; `--sweep` puts it to the cut-by-cut engine (`sweep`) instead, with its queries by the cut they
//! were put at and its hardest queries. `--aiger` writes the two circuits a proof decides as binary AIGER,
//! `<dir>/<module>.aig` and `<dir>/<top>.aig`, the input `aig_equiv` decides them from.

use std::path::PathBuf;
use std::process::exit;
use std::sync::Mutex;
use std::time::Instant;

use logicaffeine_compile::codegen_sva::autopipe::{check, emit, min_latency, pipeline, unpipelined, Cuts};
use logicaffeine_compile::codegen_sva::netlist::{self, Undefined};
use logicaffeine_compile::codegen_sva::timing_graph::{synthesize, Delays};
use logicaffeine_compile::crypto_accel::sha256;
use logicaffeine_proof::fraig;
use logicaffeine_proof::seq_equiv::{aligned, aligned_by_fraig};
use logicaffeine_proof::sweep::{Options, Progress, Stats, Verdict, CUT_DEPTHS};

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("autopipe_report: {why}");
    exit(1)
}

/// When the proof under way began, for its progress lines.
static PROOF_START: Mutex<Option<Instant>> = Mutex::new(None);

/// The queries by the cut they were put at: count, conflicts and how many refuted, each kind in turn.
fn by_cut(s: &Stats) -> String {
    let kinds = CUT_DEPTHS.iter().map(|d| format!("depth {d}")).chain(["meeting".to_string(), "whole".to_string()]);
    kinds
        .zip(&s.by_cut)
        .map(|(k, c)| format!("{k} {}q/{}c/{}r/{}m/{}b", c.queries, c.conflicts, c.refuted, c.models, c.budget))
        .collect::<Vec<_>>()
        .join(", ")
}

fn progress(s: &Stats) {
    let at = PROOF_START.lock().ok().and_then(|t| *t).map_or(0.0, |t| t.elapsed().as_secs_f64());
    eprintln!(
        "    {at:.0} s: swept {} of {}, {} candidates, {} merges, {} differences, {} unknown; {}",
        s.swept,
        s.cone,
        s.sat_calls,
        s.merges,
        s.counterexamples,
        s.unknown,
        by_cut(s)
    );
}

fn fraig_progress(s: &fraig::Stats) {
    let at = PROOF_START.lock().ok().and_then(|t| *t).map_or(0.0, |t| t.elapsed().as_secs_f64());
    eprintln!(
        "    {at:.0} s: swept {} of {}, {} structural, {} queries, {} merges, {} counterexamples, {} unknown, {} conflicts, log {}",
        s.swept, s.cone, s.structural, s.queries, s.merges, s.counterexamples, s.unknown, s.conflicts, s.log
    );
}

fn hex(text: &str) -> String {
    sha256(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: autopipe_report <verilog> <top> <period>,... [NAME=VALUE ...] [--delays L,C,D,M] [--cuts named|nets] [--outputs registered] [--emit <dir>] [--enable <port>] [--prove [--sweep]] [--aiger <dir>]");
        exit(2);
    }
    let verilog = std::fs::read_to_string(&args[0]).unwrap_or_else(|e| fail(format!("{}: {e}", args[0])));
    let top = &args[1];
    let periods: Vec<i64> = args[2].split(',').map(|p| p.trim().parse().unwrap_or_else(|_| fail(format!("a period is a number: {p}")))).collect();
    let (mut params, mut dir, mut enable, mut policy, mut delays) = (Vec::new(), None, None, Cuts::Named, Delays::LEVELS);
    let (mut registered, mut prove, mut by_sweep, mut aiger) = (false, false, false, None);
    let mut rest = args[3..].iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "--emit" => dir = Some(PathBuf::from(rest.next().unwrap_or_else(|| fail("--emit takes a directory")))),
            "--enable" => enable = Some(rest.next().unwrap_or_else(|| fail("--enable takes a port")).clone()),
            "--delays" => {
                let v: Vec<i64> = rest
                    .next()
                    .unwrap_or_else(|| fail("--delays takes lut,carry,dsp,memory"))
                    .split(',')
                    .map(|x| x.trim().parse().unwrap_or_else(|_| fail(format!("a delay is a number: {x}"))))
                    .collect();
                let [lut, carry, dsp, memory] = v[..] else { fail("--delays takes four: lut,carry,dsp,memory") };
                delays = Delays { lut, carry, dsp, memory };
            }
            "--outputs" => match rest.next().map(String::as_str) {
                Some("registered") => registered = true,
                other => fail(format!("--outputs takes registered, not {other:?}")),
            },
            "--prove" => prove = true,
            "--sweep" => by_sweep = true,
            "--aiger" => aiger = Some(PathBuf::from(rest.next().unwrap_or_else(|| fail("--aiger takes a directory")))),
            "--cuts" => {
                policy = match rest.next().map(String::as_str) {
                    Some("named") => Cuts::Named,
                    Some("nets") => Cuts::Nets,
                    other => fail(format!("--cuts takes named or nets, not {other:?}")),
                }
            }
            kv => match kv.split_once('=') {
                Some((k, v)) => params.push((k.to_string(), v.to_string())),
                None => fail(format!("a parameter is NAME=VALUE: {kv}")),
            },
        }
    }
    if prove && dir.is_none() {
        fail("--prove proves the pipeline --emit writes");
    }
    if aiger.is_some() && !prove {
        fail("--aiger writes the circuits --prove decides");
    }
    if by_sweep && !prove {
        fail("--sweep says how --prove decides");
    }
    let pairs: Vec<(&str, &str)> = params.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let source = synthesize(&verilog, top, &pairs, &[], &delays).unwrap_or_else(|e| fail(e));
    let depth = source.depth().unwrap_or_else(|e| fail(e)).depth;
    println!("{top}: depth {depth} ({} LUTs' worth), {} nodes, {} edges, delays {delays:?}", depth as f64 / delays.lut as f64, source.nodes.len(), source.edges.len());
    // A pipeline already: its logic, its latency, and whether its outputs are registers.
    let (graph, kept) = if source.edges.iter().any(|e| e.registers > 0) {
        let registered_into: std::collections::HashSet<usize> = source.edges.iter().filter(|e| e.registers > 0).map(|e| e.to).collect();
        let on_registers = source.outputs.iter().all(|o| registered_into.contains(o));
        let (logic, latency) = unpipelined(&source).unwrap_or_else(|e| fail(e));
        let depth = logic.depth().unwrap_or_else(|e| fail(e)).depth;
        println!(
            "{top} is a pipeline {latency} clocks long{}, its deepest stage {} deep; its logic, {depth} deep, is cut afresh at that latency",
            if on_registers { ", its outputs registers" } else { "" },
            source.depth().unwrap_or_else(|e| fail(e)).depth
        );
        (logic, Some((latency, on_registers)))
    } else {
        (source, None)
    };
    let registered = registered || kept.is_some_and(|(_, r)| r);
    for period in periods {
        let least = match min_latency(&graph, period, policy) {
            Ok(l) => l,
            Err(e) => {
                println!("  period {period}: {e}");
                continue;
            }
        };
        let latency = match kept {
            Some((l, _)) => {
                let target = l - registered as i64;
                if least > target {
                    println!("  period {period}: needs {} clocks, past the pipeline's {l}", least + registered as i64);
                    continue;
                }
                target
            }
            None => least,
        };
        let plan = pipeline(&graph, period, latency, policy).unwrap_or_else(|e| fail(e));
        let plan = if registered { plan.with_outputs_registered(&graph) } else { plan };
        if !check(&graph, &plan) {
            fail(format!("period {period}: the plan does not check against the graph"));
        }
        let cuts = plan.cuts(&graph);
        println!("  period {period}: latency {}, {} register bits on {} nets", plan.latency, plan.bits, cuts.len());
        let mut by_bits: Vec<&(String, i64, Vec<usize>)> = cuts.iter().collect();
        by_bits.sort_by_key(|(_, count, bits)| std::cmp::Reverse(*count as usize * bits.len()));
        for (net, count, bits) in by_bits.iter().take(24) {
            println!("    {net}: {} bit{} × {count}", bits.len(), if bits.len() == 1 { "" } else { "s" });
        }
        if let Some(dir) = &dir {
            let module = format!("{top}_p{}", plan.latency);
            let v = emit(&graph, &plan, &module, enable.as_deref()).unwrap_or_else(|e| fail(e));
            std::fs::create_dir_all(dir).unwrap_or_else(|e| fail(e));
            let file = dir.join(format!("{module}.v"));
            std::fs::write(&file, &v).unwrap_or_else(|e| fail(format!("{}: {e}", file.display())));
            println!("    wrote {}", file.display());
            if prove {
                if enable.is_some() {
                    fail("--prove reads every register as stepping each clock; leave --enable out");
                }
                let start = Instant::now();
                if let Ok(mut began) = PROOF_START.lock() {
                    *began = Some(start);
                }
                let piped = netlist::synthesize(&v, &module, &[]).unwrap_or_else(|e| fail(format!("{module}: {e}"))).aig;
                let original = netlist::synthesize_reading(&verilog, top, &pairs, Undefined::Zero).unwrap_or_else(|e| fail(format!("{top}: {e}"))).aig;
                if let Some(at) = &aiger {
                    std::fs::create_dir_all(at).unwrap_or_else(|e| fail(format!("{}: {e}", at.display())));
                    for (name, aig) in [(&module, &piped), (top, &original)] {
                        let file = at.join(format!("{name}.aig"));
                        std::fs::write(&file, aig.to_binary()).unwrap_or_else(|e| fail(format!("{}: {e}", file.display())));
                        println!("    wrote {}", file.display());
                    }
                }
                let reference = kept.map_or(0, |(l, _)| l as usize);
                let (verdicts, checks) = if by_sweep {
                    let opts = Options { progress: Some(Progress { every: 2000, report: progress }), ..Options::default() };
                    let report = aligned(&piped, plan.latency as usize, &original, reference, &opts).unwrap_or_else(|e| fail(format!("{e:?}")));
                    let checks = report.check();
                    println!(
                        "    sweep: {} SAT calls, {} merges; queries by cut (queries/conflicts/refuted/models/budget): {}",
                        report.stats.sat_calls,
                        report.stats.merges,
                        by_cut(&report.stats)
                    );
                    for h in report.stats.hardest.iter().take(5) {
                        println!("    hardest: {} conflicts over {} variables, cut of {}, {}", h.conflicts, h.vars, h.cut, h.verdict);
                    }
                    (report.verdicts, checks)
                } else {
                    let opts = fraig::Options { progress: Some(fraig::Progress { every: 10_000, report: fraig_progress }), ..fraig::Options::default() };
                    let report = aligned_by_fraig(&piped, plan.latency as usize, &original, reference, &opts).unwrap_or_else(|e| fail(format!("{e:?}")));
                    let checks = report.check();
                    let s = &report.stats;
                    println!(
                        "    fraig: {} structural, {} queries, {} merges, {} counterexamples, {} unknown, {} conflicts, derivation of {} steps",
                        s.structural, s.queries, s.merges, s.counterexamples, s.unknown, s.conflicts, s.proof
                    );
                    (report.verdicts, checks)
                };
                let equal = verdicts.iter().filter(|(_, v)| *v == Verdict::Equal).count();
                println!(
                    "    proof: {equal} of {} outputs equal {} clocks late, certificate {}, {:.1} s",
                    verdicts.len(),
                    plan.latency,
                    if checks { "checks" } else { "DOES NOT CHECK" },
                    start.elapsed().as_secs_f64()
                );
                println!("    sha256 {top} {} (source), {module} {}", hex(&verilog), hex(&v));
                if equal != verdicts.len() || !checks {
                    let open: Vec<&String> = verdicts.iter().filter(|(_, v)| *v != Verdict::Equal).map(|(n, _)| n).collect();
                    fail(format!("{module} is not proven {top} {} clocks late: {open:?}", plan.latency));
                }
            }
        }
    }
}
