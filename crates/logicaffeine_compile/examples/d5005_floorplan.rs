//! Print a floorplan for a D5005 build from a fitted build's reports (`codegen_sva::floorplan`): each memory channel
//! (`…|shell|ch[k].channel`) in the least square that holds it at `--fill` (0.5 unless given) of the persona region's
//! density, against the IO tile control its clock comes from, inside the persona's region, or the whole device with
//! `--flat`. `<output files>` is a compile's `output_files` directory and `<revision>` its revision (`iofs_pr_afu`): the
//! fit's place report, the timing analyzer's clock table and its slow 100 °C setup report are read there. It exits 2
//! naming any channel it cannot place, and refuses two regions that overlap.
//!
//!     d5005_floorplan <output files> <revision> [--flat] [--fill <fraction>]

use std::path::Path;
use std::process::exit;

use logicaffeine_compile::codegen_sva::floorplan::{
    clock_site, clock_targets, device, fast_signal_sites, instance_clock, instance_need, parent_region, region, tcl, Assignment,
};

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("d5005_floorplan: {why}");
    exit(2)
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [dir, revision, rest @ ..] = &args[..] else { fail("usage: d5005_floorplan <output files> <revision> [--flat] [--fill <fraction>]") };
    let (mut flat, mut fill) = (false, 0.5);
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--flat" => flat = true,
            "--fill" => fill = it.next().and_then(|f| f.parse().ok()).unwrap_or_else(|| fail("--fill takes a fraction")),
            other => fail(format!("unknown argument {other}")),
        }
    }
    let dir = Path::new(dir);
    let place = read(&dir.join(format!("{revision}.fit.place.rpt")));
    let clocks = read(&dir.join("timing_report/clocks.rpt"));
    let setup = read(&dir.join(format!("timing_report/{revision}_2_slow_900mv_100c_setup.rpt")));
    let (persona, density) = parent_region(&place).unwrap_or_else(|| fail("the place report has no one Logic Lock region to read the density of"));
    let parent = if flat { vec![device(&place).unwrap_or_else(|| fail("the place report states no route region"))] } else { persona };
    let fast = fast_signal_sites(&place);
    let targets = clock_targets(&clocks);
    let channel = |k: usize| format!("afu_top|port_gasket|pr_slot|afu_main|ofs_plat_afu|shell|ch[{k}].channel");
    let mut plan: Vec<Assignment> = Vec::new();
    for k in 0.. {
        let instance = channel(k);
        let Some(need) = instance_need(&place, &instance) else {
            if k == 0 {
                fail(format!("{instance}: no row in the place report's entity table"));
            }
            break;
        };
        let clock = instance_clock(&setup, &instance).unwrap_or_else(|| fail(format!("{instance}: no one clock its paths run on")));
        let on = &targets.iter().find(|(c, _)| *c == clock).unwrap_or_else(|| fail(format!("{clock}: not in the clock table"))).1;
        let site = clock_site(on, &fast).unwrap_or_else(|| fail(format!("{clock}: no one fast signal it comes from")));
        let rect = region(site, need, density, fill, &parent).unwrap_or_else(|| fail(format!("{instance}: no square holds it")));
        if let Some(other) = plan.iter().find(|a| a.rect.x0 <= rect.x1 && rect.x0 <= a.rect.x1 && a.rect.y0 <= rect.y1 && rect.y0 <= a.rect.y1) {
            fail(format!("{instance}'s region {rect:?} overlaps {}'s {:?}", other.instances[0], other.rect));
        }
        eprintln!("ch[{k}]: {need:?} on {clock} from {site:?}: {rect:?}");
        plan.push(Assignment { instances: vec![instance], name: format!("logos_ch{k}"), rect });
    }
    print!("{}", tcl(&plan));
}
