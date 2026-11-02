//! Write the D5005's geometry probe (`codegen_sva::floorplan::probe`): the device a fitted build's place report states
//! (its route region) cut into tiles of `<width>` columns by `<height>` rows (16 by 24 unless given), a one-register cell
//! a tile pinned to it by its own Logic Lock region. `probe.v` is the design (top `logos_geometry_probe`) and
//! `floorplan.tcl` its regions, for `QUARTUS_FLOORPLAN=floorplan.tcl scripts/d5005/quartus.sh fit <job> probe.v
//! logos_geometry_probe <period>`; the fit's region usage, recorded in the build ledger, is every tile's capacity
//! (`floorplan::Geometry::of`). `--without <fit log>` leaves out every tile a probe's fit found no flip-flop in
//! (`floorplan::fabricless`): a tile with no logic, which no region of a floorplan takes, so the map lacks it.
//!
//!     d5005_geometry_probe <place report> <out dir> [--tile <width>x<height>] [--without <fit log>]...

use std::path::Path;
use std::process::exit;

use logicaffeine_compile::codegen_sva::floorplan::{device, fabricless, probe, tiles};

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("d5005_geometry_probe: {why}");
    exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [place, out, rest @ ..] = &args[..] else {
        fail("usage: d5005_geometry_probe <place report> <out dir> [--tile <width>x<height>] [--without <fit log>]...")
    };
    let (mut width, mut height) = (16, 24);
    let mut without = Vec::new();
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--tile" => {
                let t = it.next().unwrap_or_else(|| fail("--tile takes <width>x<height>"));
                let parsed = t.split_once('x').and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)));
                (width, height) = parsed.filter(|&(w, h)| w > 0 && h > 0).unwrap_or_else(|| fail(format!("--tile {t}: <width>x<height>")));
            }
            "--without" => {
                let log = it.next().unwrap_or_else(|| fail("--without takes a fit log"));
                let found = fabricless(&std::fs::read_to_string(log).unwrap_or_else(|e| fail(format!("{log}: {e}"))));
                if found.is_empty() {
                    fail(format!("{log}: no tile the fit found no flip-flop in"));
                }
                without.extend(found);
            }
            other => fail(format!("unknown argument {other}")),
        }
    }
    let text = std::fs::read_to_string(place).unwrap_or_else(|e| fail(format!("{place}: {e}")));
    let whole = device(&text).unwrap_or_else(|| fail(format!("{place}: the place report states no route region")));
    let every = tiles(whole, width, height);
    if let Some(stray) = without.iter().find(|w| !every.contains(w)) {
        fail(format!("{stray:?} is no tile of {width}x{height} over {whole:?}"));
    }
    let grid: Vec<_> = every.into_iter().filter(|t| !without.contains(t)).collect();
    let (verilog, settings) = probe(&grid);
    let out = Path::new(out);
    std::fs::create_dir_all(out).unwrap_or_else(|e| fail(format!("{}: {e}", out.display())));
    std::fs::write(out.join("probe.v"), verilog).unwrap_or_else(|e| fail(e));
    std::fs::write(out.join("floorplan.tcl"), settings).unwrap_or_else(|e| fail(e));
    println!("d5005_geometry_probe: {} tiles of {width}x{height} over {whole:?}, {} left out, in {}", grid.len(), without.len(), out.display());
}
