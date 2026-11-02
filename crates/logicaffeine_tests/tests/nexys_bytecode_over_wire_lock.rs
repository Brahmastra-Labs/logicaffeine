//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! BYTECODE-OVER-WIRE LOCK — the CI-runnable (no-board) proof that we can literally send LOGOS bytecode
//! down the wire to the chip and it runs ≡ the VM.
//!
//! This is the simulation companion to `fpga_silicon_lock::logos_bytecode_runs_on_the_artix7_matching_the_vm`
//! (which is BOARD-GATED — it needs the physical Nexys A7-100T on Board B). That hardware lock proves the
//! real silicon; THIS lock proves the same end-to-end path in iverilog so it runs in the ordinary test suite
//! with no board attached — closing the gap where the wire proof lived only in `scripts/verify-nexys-soc.sh`
//! (a shell script), never in the cargo suite.
//!
//! The path proven, from LOGOS source to result, every hop the real board takes:
//!   1. `compile_to_scalar_words` — the SAME encoder that produces the `.hex` streamed to the FPGA
//!      (locked ≡ VM by `rtl.rs::encode_scalar_matches_vm`) — turns each program into UART words.
//!   2. Those words are streamed over the UART load protocol into the BOARD-RUNNABLE scalar SoC RTL
//!      (`nexys_soc_scalar.v` + `soft_core_scalar.v` + `alu.v` — the open-flow design proven on real
//!      Artix-7) under iverilog, via `tb_nexys_run.v` (`+HEX=<file> +N=<nwords>` → `RESULT: <shows>`).
//!   3. The Show values read back over the wire must equal `vm_outcome` — bit for bit.
//!
//! A green run means: encoder → UART frame → on-chip decode/execute → readback all agree with the VM, in
//! the exact RTL that flashes to the board. iverilog-gated: skips cleanly (with a loud note) where the
//! simulator is unavailable, exactly like the fabric/board locks.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

use std::path::PathBuf;
use std::process::Command;

use logicaffeine_compile::codegen_sva::scalar_asm::{compile_to_scalar_words, to_hex};
use logicaffeine_compile::compile::vm_outcome;

#[path = "common/mod.rs"]
mod common;

/// The repo root (this crate is `crates/logicaffeine_tests`).
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// The VM's Show stream for a source, one value per line → the space-joined form the SoC's `RESULT:` line
/// prints (so the two are directly comparable).
fn vm_shows(src: &str) -> String {
    let out = vm_outcome(src);
    assert!(
        out.error.is_none(),
        "VM errored on the corpus program: {:?}\n{src}",
        out.error
    );
    out.output.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// SCALAR-CORE-COMPATIBLE corpus: integer arithmetic, comparisons, branches and counted/while loops with
/// `Show` — no division, no functions, no collections (the scalar core's honest ISA). Each is small enough
/// (≤ 64 words) to fit `tb_nexys_run.v`'s program memory. Every one is compiled fresh, streamed over the
/// wire into the RTL, and checked ≡ VM.
const CORPUS: &[(&str, &str)] = &[
    ("const_pair", "## Main\n    Show 7.\n    Show 42.\n"),
    ("expr", "## Main\n    Show 2 + 3 * 4 - 1.\n"),
    ("branch", "## Main\n    Let x be 9.\n    If x is greater than 5:\n        Show 1.\n    Otherwise:\n        Show 0.\n"),
    (
        "loop_sum",
        "## Main\n    Let mutable sum be 0.\n    Let mutable i be 1.\n    While i is at most 10:\n        Set sum to sum + i.\n        Set i to i + 1.\n    Show sum.\n",
    ),
    (
        "countdown",
        "## Main\n    Let mutable n be 5.\n    Let mutable acc be 0.\n    While n is greater than 0:\n        Set acc to acc + n.\n        Set n to n - 1.\n    Show acc.\n",
    ),
];

#[test]
fn logos_bytecode_over_wire_matches_vm_in_simulation() {
    if !common::require_tool("iverilog") {
        return;
    }

    let sil = root().join("benchmarks/silicon");
    let tmp = std::env::temp_dir().join("logos_nexys_wire_lock");
    std::fs::create_dir_all(&tmp).expect("create temp dir");

    // Build the scalar-SoC sim ONCE: the board-runnable RTL (nexys_soc_scalar + soft_core_scalar + alu),
    // driven by tb_nexys_run.v which streams a +HEX program in over the UART loader and prints the Shows.
    let sim = tmp.join("nexys_scalar_run.vvp");
    let build = Command::new("iverilog")
        .current_dir(&sil)
        .args(["-g2012", "-s", "tb", "-DSOC_MODULE=nexys_soc_scalar", "-o"])
        .arg(&sim)
        .args([
            "nexys_soc_scalar.v",
            "soft_core_scalar.v",
            "alu.v",
            "tb_nexys_run.v",
        ])
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog failed to build the scalar SoC sim:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let mut proven = 0usize;
    for (name, src) in CORPUS {
        // 1) LOGOS source → scalar bytecode words (the SAME encoder that produces the FPGA .hex).
        let words = compile_to_scalar_words(src).unwrap_or_else(|e| {
            panic!("{name}: scalar encode failed (outside the scalar ISA?): {e}")
        });
        assert!(
            words.len() <= 64,
            "{name}: {} words exceeds tb_nexys_run.v's 64-word program memory — shrink the corpus program",
            words.len()
        );

        // 2) Write the .hex and stream it over the wire into the RTL.
        let hex = tmp.join(format!("{name}.hex"));
        std::fs::write(&hex, to_hex(&words)).expect("write hex");
        let run = Command::new("vvp")
            .current_dir(&sil)
            .arg(&sim)
            .arg(format!("+HEX={}", hex.display()))
            .arg(format!("+N={}", words.len()))
            .output()
            .expect("run vvp");
        let stdout = String::from_utf8_lossy(&run.stdout);

        // 3) Parse the `RESULT: <shows>  CYCLES: <n>` line the SoC prints back over the wire.
        let got = stdout
            .lines()
            .find_map(|l| l.trim().strip_prefix("RESULT:"))
            .map(|rest| {
                rest.split("CYCLES")
                    .next()
                    .unwrap_or("")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_else(|| panic!("{name}: no RESULT line from the SoC sim:\n{stdout}"));

        // 4) The wire readback must equal the VM, bit for bit.
        let want = vm_shows(src);
        assert_eq!(
            got, want,
            "{name}: the SoC readback [{got}] != VM [{want}] — bytecode streamed over the wire diverged \
             from the VM.\n  words = {words:?}\n  sim stdout = {stdout}"
        );
        proven += 1;
    }

    assert_eq!(
        proven,
        CORPUS.len(),
        "every corpus program must run over the wire ≡ VM"
    );
    println!(
        "bytecode-over-wire: {proven} LOGOS programs compiled → streamed over the UART protocol into \
         nexys_soc_scalar.v under iverilog → Shows read back ≡ VM (the no-board twin of the Artix-7 lock)."
    );
}
