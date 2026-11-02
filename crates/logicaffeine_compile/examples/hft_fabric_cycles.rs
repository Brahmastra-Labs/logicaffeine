//! hft_fabric_cycles — measure the REAL whole-program-fabric cycle count (≡VM, via
//! `eval_wprog_config_counted`) for the pieces of the HFT tick-to-trade pipeline that the one-pager's
//! projection glossed over: the O(LEVELS) best-of-book scan LOOP and the microprice DIVISION. These do
//! NOT collapse to one clock edge the way the branch-free decision select-DAG does, so this quantifies
//! how optimistic the "carry the decision's per-op reduction" projection was.

use logicaffeine_compile::codegen_sva::fabric_rtl::program_cfg_fabric;
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    eval_wprog_config_counted, serialize_program,
};

fn measure(name: &str, src: &str, func: &str, params: &[i64]) {
    let pf = program_cfg_fabric(src, func);
    let cfg = serialize_program(&pf);
    let (res, cyc) = eval_wprog_config_counted(&cfg, params);
    println!("{name:22} result={res:<12} fabric_cycles(block-visits)={cyc}");
}

fn main() {
    // The best-of-book scan: find the highest populated tick over a 16-level ladder (exactly the
    // hft_model.py inner loop). One While pass, LEVELS iterations.
    let scan = "\
## To scan () -> Int:
    Let mutable a be a new Seq of Int.
    Let mutable z be 0.
    While z is less than 16:
        Push 0 to a.
        Set z to z + 1.
    Set item 4 of a to 30.
    Set item 8 of a to 50.
    Set item 15 of a to 40.
    Let mutable best be 0.
    Let mutable t be 1.
    While t is at most 16:
        If item t of a is greater than 0:
            If t is greater than best:
                Set best to t.
        Set t to t + 1.
    Return best.
";
    measure("scan16 (book scan)", scan, "scan", &[]);

    // The microprice: (bid*ask_qty + ask*bid_qty) * 100 / denom — one integer division.
    let divide = "\
## To micro () -> Int:
    Let bidp be 100.
    Let askp be 101.
    Let bidq be 50.
    Let askq be 20.
    Let denom be bidq + askq.
    Return (bidp * askq + askp * bidq) * 100 / denom.
";
    measure("microprice (divide)", divide, "micro", &[]);

    // A trivial straight-line block for calibration (how many cycles is "one basic block"?).
    let stline = "\
## To lin () -> Int:
    Let a be 7.
    Let b be a + 3.
    Return b * 2.
";
    measure("straight-line calib", stline, "lin", &[]);

    grounded_full_pipeline_estimate();
}

/// Compose a GROUNDED per-message fabric cycle + energy estimate for the FULL tick-to-trade pipeline
/// from the measured components above. (The monolithic pipeline as one LOGOS function does not lower on
/// today's whole-program fabric: its dynamically-indexed collection ops — compound `item (i+k) of feed`,
/// clamped-index book writes — emit bounds-check `FailWith` ops the fabric lowering rejects. Supporting
/// those is the honest "future work". The scan loop — the dominant cost — IS measured directly, ==VM.)
fn grounded_full_pipeline_estimate() {
    // per-message fabric cycles (block-visits), each = 1 clock on the sequenced fabric
    let scan = 91.0_f64; //  MEASURED above (the O(16) best-of-book scan, ==VM)
    let rest = 20.0_f64; //  decode(~4) + book update(~4) + microprice numerator(~2) + decision(~6) + risk(~4)
    let per_msg = scan + rest; //  ~111 cycles / message
    let soft_core = 866.0_f64; //  MEASURED (tb_hft.v, full pipeline)

    // energy: anchor the sequenced fabric's active power to the MEASURED decision-fabric power
    // (conservative — most cycles are a small scan block, fewer tiles than the whole decision), each
    // block at its own signoff clock. Both figures are read from the power ledgers.
    let load = |rel: &str| -> serde_json::Value {
        let path = format!(
            "{}/../../benchmarks/silicon/{rel}",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(
            &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}")),
        )
        .unwrap_or_else(|e| panic!("bad JSON in {path}: {e}"))
    };
    let dfp = load("decision_fabric_power.json");
    let sky = load("sky130_power.json");
    let fab_mw = dfp["signoff_clean"]["power_mw"]["decision_selectdag_comb"]
        .as_f64()
        .expect("fabric mW");
    let f_clock = dfp["signoff_clean"]["clock_mhz"]
        .as_f64()
        .expect("fabric clock")
        * 1e6;
    let fab_fmax = dfp["signoff_clean"]["fmax_mhz"]
        .as_f64()
        .expect("fabric fmax");
    let soft_mw = sky["power_w"]["total"].as_f64().expect("soft-core W") * 1e3;
    let soft_clock = sky["clock_mhz"].as_f64().expect("soft-core clock") * 1e6;
    let e_fab_nj = fab_mw * 1e-3 * per_msg / f_clock * 1e9; //  nJ / message
    let e_i9_nj = 199.0_f64; //  MEASURED (12 W * 16.6 ns)
    let e_soft_nj = soft_mw * 1e-3 * soft_core / soft_clock * 1e9; //  soft-core power x 866 cycles

    println!(
        "\nFULL PIPELINE — grounded from the measured components (NOT a monolithic lowering):"
    );
    println!(
        "  fabric cycles / message   ~ {per_msg:.0}   (scan {scan:.0} MEASURED + rest ~{rest:.0})"
    );
    println!(
        "  soft-core cycles / message  = {soft_core:.0}  (measured)  ->  ~{:.0}x fewer cycles",
        soft_core / per_msg
    );
    println!("  fabric energy / message   ~ {e_fab_nj:.2} nJ   (at the measured {fab_mw} mW decision-fabric power)");
    println!(
        "  vs i9 ({e_i9_nj:.0} nJ)   ~ {:.0}x less energy",
        e_i9_nj / e_fab_nj
    );
    println!(
        "  vs soft-core ({e_soft_nj:.0} nJ) ~ {:.0}x less energy",
        e_soft_nj / e_fab_nj
    );
    println!("  LATENCY: {per_msg:.0} sequential cycles at the {fab_fmax} MHz every-corner fmax = {:.0} ns/msg — SLOWER than the", per_msg / (fab_fmax * 1e6) * 1e9);
    println!("           i9's 16.6 ns. The 1-cycle 'beats a gigahertz' story is the DECISION kernel, not");
    println!("           the loop-dominated full pipeline; the full pipeline wins on ENERGY (~100x), not latency.");
}
