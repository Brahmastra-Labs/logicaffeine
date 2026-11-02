//! ★ ANALOG SPICE LOCK ★ — the behavioral ngspice tier agrees with the behavioral evaluator,
//! with the comparison term DERIVED from the tolerances PINNED IN THE EMITTED NETLIST.
//!
//! Gating: skip-if-ngspice-absent (the nvcc-gate pattern — ngspice is one `apt install` away,
//! unlike hardware; `#[ignore]` stays reserved for real silicon). The netlist under test is the
//! compiler's own emission: DC sources at the block scale, one controlled source per tile, the
//! output node written via `wrdata`. The assertion is `|V(spice) − s·eval| ≤ reltol·|V| + vntol`
//! where reltol/vntol are read back from the netlist's own `.options` line — no hand-typed
//! simulator tolerance anywhere.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE.

use logicaffeine_compile::codegen_ana::{
    analog_netlist_from_source, assumed_ranges, emit_spice_dc, eval_analog_dc, scale_pass,
    AnalogTarget, SPICE_RELTOL, SPICE_VNTOL,
};

const AFFINE: &str = "## To f (x: Float) -> Float:\n    Assume x is between -1.0 and 1.0.\n    Return x * 0.5 + 0.25.\n\n## Main\n    Show f(0.5).\n";
const DOT3: &str = "## To dot3 (a: Float, b: Float, c: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Assume c is between -1.0 and 1.0.\n    Return a * 0.25 + b * 0.5 + c * 0.125.\n\n## Main\n    Show dot3(0.5, 0.25, 0.75).\n";
const VARMUL: &str = "## To vm2 (a: Float, b: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Return a * b + 0.25.\n\n## Main\n    Show vm2(0.5, 0.5).\n";

const CORPUS: &[(&str, &str, &[f64])] = &[
    ("affine", AFFINE, &[0.5]),
    ("dot3", DOT3, &[0.5, 0.25, 0.75]),
    ("varmul", VARMUL, &[0.5, 0.5]),
];
const FN_NAMES: &[&str] = &["f", "dot3", "vm2"];

fn have_ngspice() -> bool {
    std::process::Command::new("ngspice")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn ngspice_operating_points_match_the_behavioral_evaluator() {
    if !have_ngspice() {
        eprintln!("SKIP: ngspice not installed (apt install ngspice)");
        return;
    }
    let target = AnalogTarget::ideal();
    for (i, (label, src, inputs)) in CORPUS.iter().enumerate() {
        let netlist = analog_netlist_from_source(src, FN_NAMES[i], &target).expect("lowers");
        let ranges = assumed_ranges(src);
        let (schedule, _cert) = scale_pass(&netlist, &ranges, &target).expect("schedules");

        let dir = std::env::temp_dir().join(format!("logos_spice_{}_{label}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tmpdir");
        let out_path = dir.join("out.txt");
        let sp = emit_spice_dc(&schedule, &target, inputs, out_path.to_str().unwrap());
        // The comparison tolerances come from the NETLIST ITSELF — read them back.
        assert!(
            sp.contains(&format!("reltol={SPICE_RELTOL}"))
                && sp.contains(&format!("vntol={SPICE_VNTOL}")),
            "`{label}`: the netlist must pin its own simulator options:\n{sp}"
        );
        let sp_path = dir.join("netlist.sp");
        std::fs::write(&sp_path, &sp).expect("write netlist");

        let run = std::process::Command::new("ngspice")
            .arg("-b")
            .arg(&sp_path)
            .current_dir(&dir)
            .output()
            .expect("ngspice runs");
        assert!(
            run.status.success(),
            "`{label}`: ngspice failed:\n{}",
            String::from_utf8_lossy(&run.stderr)
        );

        let data = std::fs::read_to_string(&out_path).expect("wrdata output exists");
        let v_spice: f64 = data
            .split_whitespace()
            .last()
            .and_then(|t| t.parse().ok())
            .unwrap_or_else(|| panic!("`{label}`: unparseable wrdata output: {data:?}"));

        let s = f64::powi(2.0, schedule.scale_log2);
        let v_eval = s * eval_analog_dc(&schedule.quantized, inputs);
        let sim_term = SPICE_RELTOL * v_eval.abs().max(1.0e-12) + SPICE_VNTOL;
        assert!(
            (v_spice - v_eval).abs() <= sim_term,
            "`{label}`: |spice {v_spice} − eval {v_eval}| exceeds the netlist-derived term {sim_term:e}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
