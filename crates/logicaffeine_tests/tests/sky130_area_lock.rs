//! sky130_area_lock — the areas the Sky130 recorder (`scripts/record-sky130-signoff.py`) writes are the
//! design's own, so blocks of different floorplans compare.
//!
//! A ledger's die and core areas are set by the floorplan (`FP_CORE_UTIL`), not by the design, so the
//! comparative-synthesis bench compares two others, each re-derived by `--verify`:
//!
//! A. `std_cell_area_um2` — the Liberty area of every instance in the final netlist, the physical-only cells
//!    (well taps, decaps, fill, antenna diodes) left out; a cell the library does not know makes it -1, never a
//!    smaller sum;
//! B. `synth_module_areas_um2` — every module's area from the last synthesis pass's `stat -liberty` report, so a
//!    candidate kept whole inside its fold wrapper reports its own area apart from the wrapper's, and
//!    `synth_area_um2` is the top's.
//!
//! C. `synth_findings` — what OpenLane's synthesis checks find, by kind: undriven wires, conflicting drivers (a tristate
//!    bus excepted, as OpenLane excepts it) and logic loops from yosys's CHECK pass, latches, out-of-range selects and
//!    resized ports from the synthesis log. A signed-off block's flow stops on any of them; a comparative block's
//!    completes, and its row carries them.
//!
//! Each is exercised on a run directory built in a sandbox with known cells, areas and findings.

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

const LIBERTY: &str = r#"library (sandbox) {
  cell ("sky130_fd_sc_hd__nand2_1") {
    area : 3.7536;
    pin ("Y") { direction : output; }
  }
  cell ("sky130_fd_sc_hd__dfxtp_1") {
    area : 20.0192;
  }
  cell ("sky130_fd_sc_hd__tapvpwrvgnd_1") {
    area : 1.2512;
  }
  cell ("sky130_fd_sc_hd__decap_4") {
    area : 5.0048;
  }
  cell ("sky130_fd_sc_hd__fill_2") {
    area : 2.5024;
  }
  cell ("sky130_fd_sc_hd__diode_2") {
    area : 2.5024;
  }
}
"#;

const STAT: &str = "\n=== qorsyn_toy ===\n\n   Number of cells: 12\n\n   Chip area for module '\\qorsyn_toy': 150.250000\n\n\
=== toy ===\n\n   Number of cells: 3\n\n   Chip area for module '\\toy': 27.526400\n";

/// A sandbox run directory: config.tcl naming the library, a final netlist of `cells`, a synthesis report.
fn sandbox_run(dir: &Path, cells: &[&str]) -> PathBuf {
    let run = dir.join("logos_qorsyn_toy/runs/RUN_X");
    std::fs::create_dir_all(run.join("results/final/verilog/gl")).unwrap();
    std::fs::create_dir_all(run.join("reports/synthesis")).unwrap();
    let lib = dir.join("sandbox.lib");
    std::fs::write(&lib, LIBERTY).unwrap();
    std::fs::write(run.join("config.tcl"), format!("set ::env(LIB_SYNTH_COMPLETE) \"{}\"\n", lib.display())).unwrap();
    let body: String = cells.iter().enumerate().map(|(k, c)| format!(" {c} _{k}_ (\n );\n")).collect();
    std::fs::write(run.join("results/final/verilog/gl/qorsyn_toy.v"), format!("module qorsyn_toy (clk);\n input clk;\n{body}endmodule\n")).unwrap();
    std::fs::write(run.join("reports/synthesis/1-synthesis.AREA_0.stat.rpt"), "stale").unwrap();
    std::fs::write(run.join("reports/synthesis/2-synthesis.AREA_0.stat.rpt"), STAT).unwrap();
    run
}

/// The recorder's area fields for a run, as JSON.
fn areas(run: &Path) -> serde_json::Value {
    let out = std::process::Command::new("python3")
        .arg("-c")
        .arg(
            "import importlib.util, json, sys\n\
             spec = importlib.util.spec_from_file_location('rec', sys.argv[1])\n\
             rec = importlib.util.module_from_spec(spec); spec.loader.exec_module(rec)\n\
             print(json.dumps({'std': rec.std_cell_area(sys.argv[2], 'qorsyn_toy'), 'modules': rec.synth_areas(sys.argv[2])}))",
        )
        .arg(root().join("scripts/record-sky130-signoff.py"))
        .arg(run)
        .output()
        .expect("python3 runs");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("the areas as JSON")
}

#[test]
fn the_standard_cell_area_is_the_logics_and_every_module_reports_its_own() {
    let tmp = tempfile::tempdir().unwrap();
    let run = sandbox_run(
        tmp.path(),
        &[
            "sky130_fd_sc_hd__nand2_1",
            "sky130_fd_sc_hd__nand2_1",
            "sky130_fd_sc_hd__dfxtp_1",
            "sky130_fd_sc_hd__tapvpwrvgnd_1",
            "sky130_fd_sc_hd__decap_4",
            "sky130_fd_sc_hd__fill_2",
            "sky130_fd_sc_hd__diode_2",
            "sky130_ef_sc_hd__decap_12",
        ],
    );
    let a = areas(&run);
    let logic = 2.0 * 3.7536 + 20.0192;
    assert!((a["std"].as_f64().unwrap() - logic).abs() < 1e-9, "the logic's area, nothing physical: {a}");
    assert_eq!(a["modules"], serde_json::json!({ "qorsyn_toy": 150.25, "toy": 27.5264 }), "the last synthesis pass, every module");
}

#[test]
fn a_cell_the_library_does_not_know_makes_the_area_unknown_never_smaller() {
    let tmp = tempfile::tempdir().unwrap();
    let run = sandbox_run(tmp.path(), &["sky130_fd_sc_hd__nand2_1", "sky130_fd_sc_hd__xor3_4"]);
    assert_eq!(areas(&run)["std"].as_f64(), Some(-1.0));
}

#[test]
fn the_area_fields_are_reverified_with_every_other_field() {
    let rec = std::fs::read_to_string(root().join("scripts/record-sky130-signoff.py")).unwrap();
    let fields = rec.split("VERIFY_FIELDS = (").nth(1).and_then(|s| s.split(')').next()).expect("VERIFY_FIELDS");
    for f in ["\"std_cell_area_um2\"", "\"synth_area_um2\"", "\"synth_module_areas_um2\""] {
        assert!(fields.contains(f), "{f} is re-derived by --verify");
    }
}

#[test]
fn every_synthesis_finding_is_counted_by_kind() {
    let dir = tempfile::Builder::new().prefix("sky130_area_lock").tempdir().unwrap();
    let run = dir.path().join("RUN_X");
    std::fs::create_dir_all(run.join("reports/synthesis")).unwrap();
    std::fs::create_dir_all(run.join("logs/synthesis")).unwrap();
    std::fs::write(
        run.join("reports/synthesis/1-synthesis_pre_synth.chk.rpt"),
        "21. Executing CHECK pass (checking for obvious problems).\n\
         Checking module main...\n\
         Warning: Wire main.\\a_write_data [1] is used but has no driver.\n\
         Warning: Wire main.\\a_write_data [0] is used but has no driver.\n\
         Warning: multiple conflicting drivers for main.\\y:\n    port Y[0] of cell $t1 ($tribuf)\n    module input y[0]\n\
         Warning: multiple conflicting drivers for main.\\z:\n    module input f[0]\n    module input z[0]\n\
         Warning: found logic loop in module main:\n    wire \\k\n\
         Found and reported 5 problems.\n",
    )
    .unwrap();
    std::fs::write(
        run.join("reports/synthesis/1-synthesis.AREA_0.chk.rpt"),
        "Warning: Wire main.\\later is used but has no driver.\n",
    )
    .unwrap();
    std::fs::write(
        run.join("logs/synthesis/1-synthesis.log"),
        "poly.v:179: Warning: Range [0:-1] select out of bounds on signal `\\in2': Setting 1 LSB bits to undef.\n\
         Creating $LATCH cell for q\nResizing cell port main.u.a from 8 bits to 4 bits.\n",
    )
    .unwrap();
    let out = std::process::Command::new("python3")
        .arg("-c")
        .arg(
            "import importlib.util, json, sys\n\
             spec = importlib.util.spec_from_file_location('rec', sys.argv[1])\n\
             rec = importlib.util.module_from_spec(spec); spec.loader.exec_module(rec)\n\
             print(json.dumps([rec.synth_findings(sys.argv[2]), rec.synth_findings(sys.argv[3]), 'synth_findings' in rec.VERIFY_FIELDS], sort_keys=True))",
        )
        .arg(root().join("scripts/record-sky130-signoff.py"))
        .arg(&run)
        .arg(dir.path().join("no-run"))
        .output()
        .expect("python3 runs");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(
        v[0],
        serde_json::json!({"conflicting_drivers": 1, "latches": 1, "logic_loops": 1, "out_of_bounds": 1, "port_resizing": 1, "undriven": 2}),
        "the pre-synthesis check's own findings, the tristate bus excepted, and the log's"
    );
    assert_eq!(
        v[1],
        serde_json::json!({"conflicting_drivers": 0, "latches": 0, "logic_loops": 0, "out_of_bounds": 0, "port_resizing": 0, "undriven": 0}),
        "a run without the reports finds nothing"
    );
    assert_eq!(v[2], true, "--verify re-derives them");
}

