//! Time a module before Quartus does: how deep its logic is, the deepest path into each register and output (each
//! stage of a pipeline), the instances each path spends its delay in, the deepest instances, and the clock its loops
//! allow, from its Verilog through `codegen_sva::timing_graph`.
//!
//!     timing_report <top> <verilog>... [NAME=VALUE ...] [--box <module>[=memory]]... [--box-memories] [--delays L,C,D,M] [--ns <ns per LUT delay>] [--ends <n>] [--instances <n>] [--within <levels>] [--path <end>]...
//!
//! Delays are in the timing model's unit: `Delays::LEVELS` (a LUT is 100) unless `--delays` gives the LUT, carry, DSP
//! and memory delays. `--ns` prints each figure in nanoseconds as well, at that many per LUT delay. A path's delay is
//! told by instance `--within` levels down (1 unless given). An argument that is not a file and holds `=` sets a
//! parameter of the top. Each `--box` module is timed apart: its instances' inputs end paths and their outputs start
//! them, a memory's after its read, and its own logic is never elaborated. `--box-memories` boxes every memory the
//! engine's RAM generator writes (`spatial::decode_engine::rtl::ram::memory_boxes`, the `_part`s a stack or the
//! pipelined store is built of included) as one: a block RAM timed as its read, without the minutes yosys spends
//! elaborating a wide one's byte lanes. Each `--path` endpoint's deepest path is printed node by node, with the
//! depth each is reached at.

use std::collections::HashMap;
use std::path::Path;
use std::process::exit;

use logicaffeine_compile::codegen_sva::timing_graph::{synthesize, Area, Cost, Delays, End, TimingGraph};
use logicaffeine_compile::spatial::decode_engine::rtl::ram::memory_boxes;
use logicaffeine_proof::cycle_ratio::Bound;

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("timing_report: {why}");
    exit(1)
}

fn number<T: std::str::FromStr>(flag: &str, v: Option<&String>) -> T {
    let v = v.unwrap_or_else(|| fail(format!("{flag} takes a value")));
    v.parse().unwrap_or_else(|_| fail(format!("{flag}: not a number: {v}")))
}

/// A path's delay instance by instance, consecutive edges of one instance `levels` down summed: where a stage spends it.
fn spent(g: &TimingGraph, path: &[usize], levels: usize) -> Vec<(String, i64)> {
    let mut steps: HashMap<(usize, usize), usize> = path.windows(2).map(|w| ((w[0], w[1]), usize::MAX)).collect();
    for (k, e) in g.edges.iter().enumerate() {
        if e.registers == 0 {
            if let Some(best) = steps.get_mut(&(e.from, e.to)) {
                if *best == usize::MAX || g.edges[*best].delay < e.delay {
                    *best = k;
                }
            }
        }
    }
    let mut out: Vec<(String, i64)> = Vec::new();
    for w in path.windows(2) {
        let k = steps[&(w[0], w[1])];
        if k == usize::MAX || g.edges[k].delay == 0 {
            continue;
        }
        let delay = g.edges[k].delay;
        let instance = g.instances[g.owner[k]].split('|').take(levels).collect::<Vec<_>>().join("|");
        let instance = if instance.is_empty() { "(top)".to_string() } else { instance };
        match out.last_mut() {
            Some((last, total)) if *last == instance => *total += delay,
            _ => out.push((instance, delay)),
        }
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: timing_report <top> <verilog>... [NAME=VALUE ...] [--box <module>[=memory]]... [--box-memories] [--delays L,C,D,M] [--ns <ns per LUT delay>] [--ends <n>] [--instances <n>] [--within <levels>] [--path <end>]...");
        exit(2);
    }
    let top = &args[0];
    let (mut verilog, mut params, mut delays, mut ns) = (String::new(), Vec::new(), Delays::LEVELS, None);
    let (mut ends, mut deepest, mut within) = (50usize, 12usize, 1usize);
    let (mut boxes, mut paths): (Vec<(String, Cost, Area)>, Vec<String>) = (Vec::new(), Vec::new());
    let mut memories = false;
    let mut rest = args[1..].iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
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
            "--ns" => ns = Some(number::<f64>("--ns", rest.next())),
            "--box" => {
                let b = rest.next().unwrap_or_else(|| fail("--box takes a module"));
                boxes.push(match b.split_once('=') {
                    None => (b.clone(), Cost::NONE, Area::default()),
                    Some((m, "memory")) => (m.to_string(), Cost::MEMORY, Area::default()),
                    Some((_, other)) => fail(format!("--box {b}: a box's outputs are plain or =memory, not {other}")),
                });
            }
            "--box-memories" => memories = true,
            "--path" => paths.push(rest.next().unwrap_or_else(|| fail("--path takes an endpoint's name")).clone()),
            "--ends" => ends = number("--ends", rest.next()),
            "--instances" => deepest = number("--instances", rest.next()),
            "--within" => within = number("--within", rest.next()),
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
    if verilog.is_empty() {
        fail("no Verilog file named");
    }
    let shown = |units: f64| -> String {
        let levels = units / delays.lut as f64;
        match ns {
            Some(per) => format!("{units} ({levels:.2} LUT delays, {:.1} ns)", levels * per),
            None => format!("{units} ({levels:.2} LUT delays)"),
        }
    };
    let pairs: Vec<(&str, &str)> = params.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    if memories {
        for (m, cost, area) in memory_boxes(&verilog, top) {
            if !boxes.iter().any(|(b, _, _)| *b == m) {
                boxes.push((m, cost, area));
            }
        }
    }
    let boxed: Vec<(&str, Cost, Area)> = boxes.iter().map(|(b, c, a)| (b.as_str(), *c, *a)).collect();
    let g = synthesize(&verilog, top, &pairs, &boxed, &delays).unwrap_or_else(|e| fail(e));
    let depth = g.depth().unwrap_or_else(|e| fail(e));
    println!("{top}{}: {} nodes, {} edges, {} flip-flops, delays {delays:?}", if params.is_empty() { String::new() } else { format!(" {params:?}") }, g.nodes.len(), g.edges.len(), g.flops);
    for b in &g.boxes {
        println!("box {} ({}): {} bits in, {} bits out, timed apart", b.cell, b.module, b.inputs.len(), b.outputs.len());
    }
    println!("depth {}", shown(depth.depth as f64));
    match g.loop_bound() {
        Ok(Bound::Acyclic) => println!("loops: none"),
        Ok(b @ Bound::Ratio { .. }) => {
            let (p, q) = b.period().unwrap_or((0, 1));
            println!("loop bound {} a register: {}", shown(p as f64 / q as f64), g.name_loop(&b).join(" -> "));
        }
        Ok(b @ Bound::CombinationalLoop(_)) => println!("a combinational loop: {}", g.name_loop(&b).join(" -> ")),
        Err(e) => println!("loops: {e:?}"),
    }
    let all = g.endpoints().unwrap_or_else(|e| fail(e));
    println!("deepest path into each register and output, deepest first ({} of {}):", all.len().min(ends), all.len());
    for e in all.iter().take(ends) {
        let kind = match e.end {
            End::Register => "register",
            End::Output => "output",
            End::Box => "box input",
        };
        let from = e.path.first().map(|&n| g.nodes[n].as_str()).unwrap_or("");
        println!("  {} {kind} {} ×{} from {from}", shown(e.depth as f64), e.name, e.bits);
        let by: Vec<String> = spent(&g, &e.path, within).into_iter().map(|(i, d)| format!("{i} {d}")).collect();
        if !by.is_empty() {
            println!("      {}", by.join(" -> "));
        }
    }
    if !paths.is_empty() {
        let arrival = g.arrivals().unwrap_or_else(|e| fail(e));
        for name in &paths {
            let found: Vec<_> = all.iter().filter(|e| &e.name == name).collect();
            if found.is_empty() {
                fail(format!("--path {name}: no endpoint of that name"));
            }
            for e in found {
                println!("deepest path into {name} ({:?}), {}:", e.end, shown(e.depth as f64));
                for &n in &e.path {
                    println!("  {:>8}  {}", arrival[n], g.nodes[n]);
                }
            }
        }
    }
    let mut instances = depth.instances.clone();
    instances.sort_by(|a, b| b.own.cmp(&a.own).then_with(|| a.path.cmp(&b.path)));
    println!("deepest instances by their own cells ({} of {}):", instances.len().min(deepest), instances.len());
    for i in instances.iter().take(deepest) {
        println!("  {} own, {} total  {}", shown(i.own as f64), shown(i.total as f64), if i.path.is_empty() { "(top)" } else { &i.path });
    }
}
