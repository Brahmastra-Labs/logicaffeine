//! Record where the D5005's memory channels' banks are (`codegen_sva::floorplan::anchor`): each memory channel of a
//! fitted build (`…|shell|ch[k].channel`, every k its place report's entity table has), the clock its paths run on and
//! the site of the IO tile control that clock leaves, read off the build's reports in `<output files>`
//! (`<revision>.fit.place.rpt`, `timing_report/clocks.rpt` and its slow 100 °C setup report), and written to `<ledger>`
//! as the anchors of `<build>`. It exits 2 naming a channel it cannot anchor.
//!
//!     d5005_anchors <output files> <revision> <build> <ledger>

use std::path::Path;
use std::process::exit;
use std::time::{SystemTime, UNIX_EPOCH};

use logicaffeine_base::temporal::civil_from_days;
use logicaffeine_compile::codegen_sva::floorplan::{anchor, instance_need, Anchors};

const WHAT: &str = "Where each memory channel's bank is on the D5005: the channel, the clock its paths run on and the site \
of the IO tile control that clock leaves, read off a fitted build's reports by d5005_anchors (codegen_sva::floorplan::anchor). \
The unit floorplan (d5005_unit_floorplan) wires each channel to its anchor.";

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("d5005_anchors: {why}");
    exit(2)
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())))
}

/// Today, `YYYY-MM-DD`.
fn today() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).expect("after 1970").as_secs() as i64;
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [dir, revision, build, ledger] = &args[..] else { fail("usage: d5005_anchors <output files> <revision> <build> <ledger>") };
    let dir = Path::new(dir);
    let place = read(&dir.join(format!("{revision}.fit.place.rpt")));
    let clocks = read(&dir.join("timing_report/clocks.rpt"));
    let setup = read(&dir.join(format!("timing_report/{revision}_2_slow_900mv_100c_setup.rpt")));
    let channel = |k: usize| format!("afu_top|port_gasket|pr_slot|afu_main|ofs_plat_afu|shell|ch[{k}].channel");
    let anchors: Vec<_> = (0..)
        .map(channel)
        .take_while(|c| instance_need(&place, c).is_some())
        .map(|c| anchor(&place, &clocks, &setup, &c).unwrap_or_else(|e| fail(e)))
        .collect();
    if anchors.is_empty() {
        fail(format!("{}: no row in the place report's entity table", channel(0)));
    }
    for a in &anchors {
        eprintln!("{} on {} from {:?}", a.instance, a.clock, a.site);
    }
    let record = Anchors { what: WHAT.into(), build: build.clone(), recorded: today(), anchors };
    let text = serde_json::to_string_pretty(&record).expect("anchors serialize") + "\n";
    std::fs::write(ledger, text).unwrap_or_else(|e| fail(format!("{ledger}: {e}")));
}
