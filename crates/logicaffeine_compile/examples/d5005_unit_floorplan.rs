//! Print a floorplan of a D5005 build's units (`codegen_sva::floorplan::unit_regions`): each `--unit` (an instance path
//! under the module `--top`, the shell unless given) and each `--group <name>=<path>,<path>...` (instances sharing one
//! region, `<name>` its name, its needs theirs summed) in a Logic Lock region of whole tiles of the device's geometry,
//! placed in the order given where its wires are shortest, grown round its seed (`floorplan::grow`), or with
//! `--rectangles` the least rectangle of whole tiles that holds it (`floorplan::unit_floorplan`).
//!
//! - The geometry is what the geometry probe's fit measured, its row `<probe job>` in the build ledger.
//! - A unit's needs are its synthesis (`<syn report>`, `floorplan::synthesized_needs`) at the packing the ledger's
//!   syntheses show.
//! - The wires are the bits between the instances of `--top` in the tree's Verilog (every `.v` file `<rtl dir>/sources.txt`
//!   names), with the modules `--through` names (the engine and its fast side unless given) dissolved into it
//!   (`netlist::module_graph`).
//! - A unit the anchors ledger names (a memory channel) is wired to its bank's site.
//!
//! `--fill <logic>,<m20k>,<dsp>` is each unit's share of a region's logic, M20Ks and DSP blocks (0.5, 0.9 and 0.9 unless
//! given), and `--aspect` the longest side a rectangle may have over its shortest (3 unless given). The settings go to
//! stdout, each unit's region and what it takes of it to stderr, and with `--wires` first every wire the units are
//! placed on (`floorplan::unit_wires`), heaviest first, then each instance no unit holds by the bits it carries to the
//! units (`floorplan::loose`): a path through one is no wire until its module is dissolved by `--through`. It exits 2
//! naming what it cannot read or place.
//!
//!     d5005_unit_floorplan <build ledger> <probe job> <anchors ledger> <syn report> <rtl dir> [--unit <path>]...
//!         [--group <name>=<path>,<path>...]... [--top <module>] [--through <module>,...] [--fill <logic>,<m20k>,<dsp>]
//!         [--rectangles [--aspect <ratio>]] [--wires]

use std::path::Path;
use std::process::exit;

use logicaffeine_compile::codegen_sva::floorplan::{
    loose, region_name, regions_tcl, synthesized_needs, tcl, unit_floorplan, unit_regions, unit_wires, Anchors, Capacity, Fill, Geometry, Needs, Rect, Unit, Wire,
};
use logicaffeine_compile::codegen_sva::netlist::module_graph;
use logicaffeine_compile::codegen_sva::preflight::packing;
use logicaffeine_compile::codegen_sva::quartus::{parse_hierarchy, Ledger};

const USAGE: &str = "usage: d5005_unit_floorplan <build ledger> <probe job> <anchors ledger> <syn report> <rtl dir> [--unit <path>]... \
[--group <name>=<path>,<path>...]... [--top <module>] [--through <module>,...] [--fill <logic>,<m20k>,<dsp>] [--rectangles [--aspect <ratio>]] [--wires]";

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("d5005_unit_floorplan: {why}");
    exit(2)
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [builds, probe, anchors, syn, rtl, rest @ ..] = &args[..] else { fail(USAGE) };
    let (mut units, mut top, mut through) = (Vec::new(), "logos_d5005_shell".to_string(), vec!["logos_engine".to_string(), "logos_engine_fast".to_string()]);
    let (mut fill, mut aspect, mut rectangles, mut wires) = (Fill { logic: 0.5, m20k: 0.9, dsp: 0.9 }, 3.0, false, false);
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| fail(format!("{a} takes a value")));
        match a.as_str() {
            "--unit" => {
                let path = value().clone();
                units.push((region_name(&path), vec![path]));
            }
            "--group" => {
                let v = value();
                let Some((name, paths)) = v.split_once('=') else { fail(format!("--group {v}: <name>=<path>,<path>...")) };
                let members: Vec<String> = paths.split(',').filter(|p| !p.is_empty()).map(str::to_string).collect();
                if name.is_empty() || members.is_empty() {
                    fail(format!("--group {v}: a name and one instance or more"));
                }
                units.push((name.to_string(), members));
            }
            "--top" => top = value().clone(),
            "--through" => through = value().split(',').filter(|m| !m.is_empty()).map(str::to_string).collect(),
            "--fill" => {
                let v = value();
                let shares: Vec<f64> = v.split(',').map(|s| s.parse().unwrap_or_else(|_| fail(format!("--fill {v}: three fractions")))).collect();
                let [logic, m20k, dsp] = shares[..] else { fail(format!("--fill {v}: <logic>,<m20k>,<dsp>")) };
                if ![logic, m20k, dsp].iter().all(|s| *s > 0.0 && *s <= 1.0) {
                    fail(format!("--fill {v}: each share above 0 and at most 1"));
                }
                fill = Fill { logic, m20k, dsp };
            }
            "--rectangles" => rectangles = true,
            "--wires" => wires = true,
            "--aspect" => {
                let v = value();
                aspect = v.parse().ok().filter(|r: &f64| *r >= 1.0).unwrap_or_else(|| fail(format!("--aspect {v}: a ratio of at least 1")));
            }
            other => fail(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    if units.is_empty() {
        fail(format!("no --unit or --group to place\n{USAGE}"));
    }
    let ledger = Ledger::load(Path::new(builds)).unwrap_or_else(|e| fail(e));
    let row = ledger.rows.iter().find(|r| r.name == *probe).unwrap_or_else(|| fail(format!("{builds}: no row {probe}")));
    let geometry = Geometry::of(&row.build.regions).unwrap_or_else(|| fail(format!("{probe}: no geometry probe's regions, every tile counted")));
    let packing = packing(&ledger).unwrap_or_else(|| fail(format!("{builds}: no synthesis to read a packing off")));
    let anchors: Anchors = serde_json::from_str(&read(Path::new(anchors))).unwrap_or_else(|e| fail(format!("{anchors}: {e}")));
    let syn = read(Path::new(syn));
    let nodes = parse_hierarchy(&syn);
    let under = match nodes.iter().filter(|n| n.entity == top).map(|n| n.path.as_str()).collect::<Vec<_>>()[..] {
        [path] => path.to_string(),
        [] => fail(format!("the synthesis has no instance of {top}")),
        _ => fail(format!("the synthesis has several instances of {top}")),
    };
    let rtl = Path::new(rtl);
    let verilog: String = read(&rtl.join("sources.txt"))
        .lines()
        .map(str::trim)
        .filter(|l| l.ends_with(".v") && !l.contains(':'))
        .map(|f| read(&rtl.join(f)) + "\n")
        .collect();
    let through: Vec<&str> = through.iter().map(String::as_str).collect();
    let graph = module_graph(&verilog, &top, &through).unwrap_or_else(|e| fail(e));
    let units: Vec<Unit> = units
        .into_iter()
        .map(|(name, instances)| {
            let paths: Vec<String> = instances.iter().map(|u| format!("{under}|{u}")).collect();
            let needs = synthesized_needs(&syn, &paths, &packing).unwrap_or_else(|path| fail(format!("{path}: no row in the synthesis's entity table")));
            Unit { instances, name, needs }
        })
        .collect();
    if wires {
        let (_, mut all) = unit_wires(&graph, &under, &units, &anchors.anchors).unwrap_or_else(|e| fail(e));
        let bits = |w: &Wire| match w {
            Wire::Units(_, _, b) | Wire::Site(_, _, b) => *b,
        };
        all.sort_by_key(|w| std::cmp::Reverse(bits(w)));
        for w in &all {
            match *w {
                Wire::Units(a, b, n) => eprintln!("wire {} - {}: {n} bits", units[a].name, units[b].name),
                Wire::Site(a, (x, y), n) => eprintln!("wire {} - its anchor at X{x} Y{y}: {n} bits", units[a].name),
            }
        }
        for (path, n) in loose(&graph, &units) {
            eprintln!("loose {path}: {n} bits to the units");
        }
    }
    let report = |name: &str, rects: &[Rect], n: &Needs| {
        let c = rects.iter().fold(Capacity::default(), |c, r| {
            let t = geometry.capacity(*r).expect("a region of whole tiles");
            Capacity { alms: c.alms + t.alms, registers: c.registers + t.registers, m20k: c.m20k + t.m20k, dsp: c.dsp + t.dsp }
        });
        let share = |need: f64, have: u64| if have == 0 { 0.0 } else { 100.0 * need / have as f64 };
        let place: Vec<String> = rects.iter().map(|r| format!("X{} Y{} X{} Y{}", r.x0, r.y0, r.x1, r.y1)).collect();
        eprintln!(
            "{name}: {}: ALMs {:.0}/{} ({:.0}%), registers {:.0}/{} ({:.0}%), M20Ks {}/{} ({:.0}%), DSPs {}/{} ({:.0}%)",
            place.join(";"),
            n.alms,
            c.alms,
            share(n.alms, c.alms),
            n.registers,
            c.registers,
            share(n.registers, c.registers),
            n.m20k,
            c.m20k,
            share(n.m20k as f64, c.m20k),
            n.dsp,
            c.dsp,
            share(n.dsp as f64, c.dsp)
        );
    };
    if rectangles {
        let plan = unit_floorplan(&geometry, &graph, &under, &units, &anchors.anchors, fill, aspect, &[]).unwrap_or_else(|e| fail(e));
        plan.iter().zip(&units).for_each(|(a, u)| report(&a.name, &[a.rect], &u.needs));
        print!("{}", tcl(&plan));
    } else {
        let plan = unit_regions(&geometry, &graph, &under, &units, &anchors.anchors, fill, &[]).unwrap_or_else(|e| fail(e));
        plan.iter().zip(&units).for_each(|(r, u)| report(&r.name, &r.rects(), &u.needs));
        print!("{}", regions_tcl(&plan));
    }
}
