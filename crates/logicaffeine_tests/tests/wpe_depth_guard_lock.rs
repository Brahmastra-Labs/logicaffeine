//! wpe_depth_guard_lock — the frame stack holds EXACTLY `DEPTH` frames, and the overflow guard fires on the
//! `DEPTH + 1`-th push, not one earlier.
//!
//! The engine's `DEPTH` is what the fit check compares a program's MEASURED peak call depth against
//! (`eval_wprog_config_run(..).peak_depth <= DEPTH` ⇒ it fits). If the RTL refuses one frame short of that, a
//! program the model and the fit check both accept comes back `0xE1` from silicon — measured on Board B: the
//! sumdown recursion `f(8)` (8 frames, `wpe_rec8.wire`) answered 36 on the guard-less image and OVERFLOW on the
//! guarded one at `DEPTH = 8`. The guard and the model must agree on the same number, so this lock runs the
//! XC7A100T netlist (WPE_HW, `DEPTH = 8`) under iverilog at exactly DEPTH frames (must answer) and at DEPTH + 1
//! (must report `0xE1`).

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    emit_wprog_engine_soc_testbench, emit_wprog_engine_soc_verilog_hw, eval_wprog_config_run,
    program_calls_fabric, wprog_config_fits_hw_engine, wprog_config_image, WPE_HW_DEPTH,
};

/// A two-call recursion the compiler cannot turn into a loop (a sumdown is loop-converted and never pushes a
/// frame): `fib(n)` peaks at `n - 1` frames on the engine model.
const FIB: &str = "## To f (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return f(n - 1) + f(n - 2).\n";
const CLKS_PER_BIT: usize = 4;

/// The smallest argument at which `fib(n)` peaks at exactly `frames` call frames on the engine model.
fn arg_peaking_at(
    cfg: &logicaffeine_compile::codegen_sva::wprog_engine_rtl::WProgConfig,
    frames: usize,
) -> i64 {
    (1..64)
        .find(|&n| eval_wprog_config_run(cfg, &[n], &[]).peak_depth == frames)
        .unwrap_or_else(|| panic!("no fib argument peaks at {frames} frames"))
}

/// Stream `f(n)` into the hw netlist under iverilog; return the testbench's verdict lines.
fn run_hw(name: &str, n: i64) -> Option<String> {
    if !common::require_tool("iverilog") {
        return None;
    }
    let cfg = program_calls_fabric(FIB, "f");
    wprog_config_fits_hw_engine(&cfg).expect("fib fits the hw caps");
    let dir = std::env::temp_dir().join("logos_wpe_depth_guard");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let im = wprog_config_image(&cfg);
    let bytes = im.wire_bytes(&[n]);
    let oracle = eval_wprog_config_run(&cfg, &[n], &[]).result;
    std::fs::write(
        dir.join(format!("wire_{name}.hex")),
        bytes
            .iter()
            .map(|b| format!("{b:02x}\n"))
            .collect::<String>(),
    )
    .unwrap();
    std::fs::write(
        dir.join(format!("lens_{name}.hex")),
        format!("{:08x}\n", bytes.len()),
    )
    .unwrap();
    std::fs::write(
        dir.join(format!("oracle_{name}.hex")),
        format!("{:016x}\n", oracle as u64),
    )
    .unwrap();
    let soc = emit_wprog_engine_soc_verilog_hw(name);
    let tb = emit_wprog_engine_soc_testbench(name, 1, CLKS_PER_BIT, bytes.len());
    let srcv = dir.join(format!("wpe_soc_{name}.v"));
    let tbv = dir.join(format!("tb_wpe_soc_{name}.v"));
    let sim = dir.join(format!("sim_{name}.vvp"));
    std::fs::write(&srcv, &soc).unwrap();
    std::fs::write(&tbv, &tb).unwrap();
    let build = std::process::Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&sim)
        .arg(&srcv)
        .arg(&tbv)
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "iverilog failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .unwrap();
    Some(String::from_utf8_lossy(&run.stdout).into_owned())
}

#[test]
fn a_recursion_exactly_depth_frames_deep_runs_and_one_deeper_is_refused() {
    let cfg = program_calls_fabric(FIB, "f");
    assert!(
        eval_wprog_config_run(&cfg, &[5], &[]).peak_depth > 0,
        "fib pushes real frames on the model"
    );
    // The model measures these at exactly DEPTH and DEPTH + 1 frames; the fit check accepts the first, so the
    // chip must answer it, and must refuse the second with the overflow terminator.
    let at = arg_peaking_at(&cfg, WPE_HW_DEPTH);
    let over = arg_peaking_at(&cfg, WPE_HW_DEPTH + 1);
    assert_eq!(over, at + 1);

    if let Some(out) = run_hw("at_depth", at) {
        assert!(
            out.contains("PASS:"),
            "fib({at}) fits DEPTH={WPE_HW_DEPTH} by the model's measure but the netlist did not answer it:\n{out}"
        );
    }
    if let Some(out) = run_hw("over_depth", over) {
        assert!(
            !out.contains("PASS:"),
            "fib({over}) exceeds DEPTH but the netlist answered:\n{out}"
        );
        assert!(
            out.contains("end=e1"),
            "the over-deep run must report the 0xE1 overflow terminator:\n{out}"
        );
    }
}
