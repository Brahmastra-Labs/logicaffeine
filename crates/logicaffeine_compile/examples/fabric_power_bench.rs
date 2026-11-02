//! fabric_power_bench — per-benchmark FABRIC size + power estimate, on the CFG/loop fabric.
//!
//! The spatial fabric runs ITERATIVE compute (branches + counted/while loops, no recursion, no unbounded
//! heap). Most benchmarks put that compute in a named `## To <fn>` function, which lowers via
//! `program_cfg_fabric` (the same path `wprog_engine_lock` proves ≡ VM). For each benchmark we lower its
//! first function, count the spatial compute TILES (`sum of CfgBlock.body`), and scale the MEASURED
//! `decision_hft` power density (OpenLane Sky130 signoff-clean, post-route RCX OpenSTA, read from
//! decision_fabric_power.json) to a per-benchmark fabric power + energy/eval estimate. Recursive / heap / Main-only
//! benchmarks REFUSE — those are soft-core programs, not spatial kernels. Tile-scaling is a first-order
//! estimate (a multiply tile ≫ an add tile); a gold number is a yosys+OpenSTA synth of the emitted RTL.
//!
//!   cargo run -p logicaffeine-compile --example fabric_power_bench

use std::panic;

use logicaffeine_compile::codegen_sva::fabric_rtl::program_cfg_fabric;

/// The measured anchor: the decision fabric's tile count, combinational power, energy per decision
/// and placed cells, read from the power ledger (benchmarks/silicon/decision_fabric_power.json), which
/// `scripts/record-silicon-power.py` writes from the netlist the signoff ledger ships.
struct Anchor {
    tiles: f64,
    mw: f64,
    pj: f64,
    cells: f64,
}

fn anchor() -> Anchor {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../benchmarks/silicon/decision_fabric_power.json"
    );
    let raw = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let j: serde_json::Value =
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("bad JSON in {path}: {e}"));
    let num = |v: &serde_json::Value, what: &str| {
        v.as_f64()
            .unwrap_or_else(|| panic!("{path}: {what} missing"))
    };
    let s = &j["signoff_clean"];
    Anchor {
        tiles: num(&j["tiles"]["total"], "tiles.total"),
        mw: num(
            &s["power_mw"]["decision_selectdag_comb"],
            "signoff_clean.power_mw.decision_selectdag_comb",
        ),
        pj: num(
            &s["energy_per_trade_pJ"]["decision"],
            "signoff_clean.energy_per_trade_pJ.decision",
        ),
        cells: num(&s["std_cells_placed"], "signoff_clean.std_cells_placed"),
    }
}

/// The first `## To <name>` in a source, if any (the benchmark's compute function).
fn first_fn(src: &str) -> Option<String> {
    src.lines().find_map(|l| {
        let l = l.trim_start();
        l.strip_prefix("## To ").map(|rest| {
            rest.split(|c: char| c == ' ' || c == '(')
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        })
    })
}

fn main() {
    let a = anchor();
    panic::set_hook(Box::new(|_| {})); // silence refusal panics; we catch + classify them

    let root = env!("CARGO_MANIFEST_DIR");
    let progs = format!("{root}/../../benchmarks/programs");
    let mut names: Vec<String> = std::fs::read_dir(&progs)
        .expect("benchmarks/programs")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| std::path::Path::new(&format!("{progs}/{n}/interp.lg")).exists())
        .collect();
    names.sort();

    println!(
        "{:<16} {:>14} {:>7} {:>7} {:>9} {:>11} {:>11}",
        "benchmark", "fn", "blocks", "tiles", "~cells", "~power_mW", "~energy_pJ"
    );
    println!("{}", "-".repeat(82));

    let mut lowered = 0u32;
    let mut refused: Vec<String> = Vec::new();
    let (mut sum_tiles, mut sum_mw) = (0.0f64, 0.0f64);
    for n in &names {
        let src = std::fs::read_to_string(format!("{progs}/{n}/interp.lg")).unwrap();
        let Some(fname) = first_fn(&src) else {
            refused.push(format!("{n}(Main-only)"));
            continue;
        };
        let res = panic::catch_unwind(panic::AssertUnwindSafe(|| program_cfg_fabric(&src, &fname)));
        match res {
            Ok(pf) => {
                let blocks = pf.blocks.len();
                let tiles: usize = pf.blocks.iter().map(|b| b.body.len()).sum();
                let t = tiles as f64;
                let cells = t * (a.cells / a.tiles);
                let mw = t * (a.mw / a.tiles);
                let pj = t * (a.pj / a.tiles);
                println!(
                    "{:<16} {:>14} {:>7} {:>7} {:>9.0} {:>11.3} {:>11.1}",
                    n, fname, blocks, tiles, cells, mw, pj
                );
                lowered += 1;
                sum_tiles += t;
                sum_mw += mw;
            }
            Err(_) => refused.push(format!("{n}({fname})")),
        }
    }

    println!("{}", "-".repeat(82));
    println!(
        "{lowered} benchmark functions lowered to the spatial fabric  (total {:.0} tiles, {:.2} mW aggregate)",
        sum_tiles, sum_mw
    );
    println!("refused ({}): {}", refused.len(), refused.join(", "));
    println!(
        "\nscaling anchor (MEASURED, decision_hft Sky130 signoff-clean, post-route RCX OpenSTA): \
         {:.0} tiles = {} mW comb = {} pJ/eval, {:.0} cells.",
        a.tiles, a.mw, a.pj, a.cells
    );
}
