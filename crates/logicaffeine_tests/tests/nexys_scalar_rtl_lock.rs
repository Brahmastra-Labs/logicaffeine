//! nexys_scalar_rtl_lock — PIT-OF-SUCCESS invariants for the board-runnable scalar LOGOS SoC that is
//! PROVEN on the real Nexys A7-100T: a LOGOS program (`sumsq.lg`) compiles through the register bytecode +
//! `codegen_sva::scalar_asm` encoder, streams to the FPGA over UART, and executes on the Artix-7 with a
//! result and cycle count that match the VM exactly.
//!
//! DESIGN (deliberately NOT a brittle linter): the PRIMARY safety net is BEHAVIORAL and lives elsewhere —
//! `encode_scalar_matches_vm` (the .lg->chip encoder ≡ VM) and `scripts/verify-nexys-soc.sh` (the RTL runs
//! the demos ≡ VM in iverilog). Those catch any wrong answer robustly, regardless of how the RTL is
//! refactored. THIS lock only guards the handful of HARDWARE-ONLY failure classes that simulation and the
//! model physically cannot see — the open flow silently dropping LUT-RAM writes, and a missing power-on
//! reset — by asserting each FIX PATTERN is still PRESENT. It intentionally does NOT pin sizes, clock
//! ratios, register widths, or framing bytes: deepen the imem, speed up the clock, or extend the protocol
//! freely — only removing a load-bearing fix trips this. Prevent the mistake; don't tie our hands.

#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
/// File contents with whitespace runs collapsed, so a fix pattern is matched by its load-bearing tokens
/// (not incidental formatting).
fn squished(rel: &str) -> String {
    let raw = fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"));
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}
/// True iff a `reg`-array memory named `mem` is declared flip-flop-backed (`ram_style="registers"`). This
/// is the open-flow fix: nextpnr-xilinx/prjxray silently drop writes to LUT-RAM (init'd or back-to-back),
/// so any write-ported memory must be FF-backed or the program never reaches the core / a value is lost —
/// a failure iverilog and the VM both PASS. Robust to depth/width changes; only removal of the attribute
/// on that memory trips it.
fn memory_is_ff_backed(file: &str, mem: &str) -> bool {
    let s = squished(file);
    // find the `ram_style = "registers"` attribute that immediately precedes the `... <mem> [` declaration
    s.split(&format!("reg [31:0] {mem} ["))
        .next()
        .map(|before| {
            before
                .trim_end()
                .ends_with("(* ram_style = \"registers\" *)")
        })
        .unwrap_or(false)
        || s.contains(&format!(
            "(* ram_style = \"registers\" *) reg [31:0] {mem} ["
        ))
}

#[test]
fn write_ported_memories_are_flip_flop_backed() {
    // The two memories a program's bytes flow THROUGH on the way to producing output. Both are written on
    // the hot path and MUST be FF-backed for the open FPGA flow (see fn doc).
    assert!(
        memory_is_ff_backed("benchmarks/silicon/soft_core_scalar.v", "imem"),
        "soft_core_scalar's imem is not `(* ram_style=\"registers\" *)` — the open flow drops LUT-RAM \
         writes, so the loaded program never lands and the core runs all-zeros: a SILENT board that \
         simulation does not catch. Re-add the attribute (any depth is fine)."
    );
    assert!(
        memory_is_ff_backed("benchmarks/silicon/nexys_soc_scalar.v", "show_mem"),
        "nexys_soc_scalar's show_mem is not FF-backed — the open flow drops the 2nd of two consecutive \
         writes, so a program with back-to-back `Show`s reads 0 for the second value on silicon."
    );
}

#[test]
fn scalar_core_keeps_its_uart_load_port() {
    let s = squished("benchmarks/silicon/soft_core_scalar.v");
    assert!(
        s.contains("`ifdef SOFT_CORE_SCALAR_PROG_LOAD")
            && s.contains("input prog_we,")
            && s.contains("if (prog_we) imem[prog_addr] <= prog_data;"),
        "soft_core_scalar lost its UART program-load port — the SoC can no longer stream a program in"
    );
    // The boot-ROM $readmemh must stay guarded OUT of the loadable build (it fights the writable imem).
    assert!(
        s.contains("`ifndef SOFT_CORE_SCALAR_PROG_LOAD"),
        "the boot-ROM $readmemh is no longer guarded out of the program-load build (it would fight the \
         write port and re-break imem)"
    );
}

#[test]
fn board_top_keeps_a_power_on_reset() {
    let s = squished("benchmarks/silicon/nexys_top_scalar.v");
    // After configuration there is no reset EDGE, so a POR pulse is required or core-hold / uart_tx power
    // up wrong and the board is silent. Match the pattern (an internal reset ORed into the SoC reset),
    // not any specific counter width.
    assert!(
        s.contains("~por") && s.contains("soc_rst = rst |"),
        "the power-on reset is gone — without a reset pulse after configuration the board comes up silent \
         (simulation pulses reset; hardware does not). Keep the POR (its width/shape is free to change)."
    );
    // The core runs on a divided global clock (its real path is slower than the open timing model claims);
    // keep the global buffer. The exact ratio and the matching UART divisor are free — the sim + hardware
    // round-trip verify them.
    assert!(
        s.contains("BUFG"),
        "the divided core clock lost its global clock buffer (BUFG)"
    );
}

#[test]
fn the_chip_runnable_artifacts_exist() {
    for hex in ["sum_loop.hex", "expr_13.hex", "const_7_42.hex", "sumsq.hex"] {
        assert!(
            root()
                .join("benchmarks/silicon/programs")
                .join(hex)
                .exists(),
            "chip demo program {hex} is missing"
        );
    }
    assert!(
        root().join("benchmarks/silicon/programs/sumsq.lg").exists(),
        "sumsq.lg — the compiled-from-source flagship — is missing"
    );
    assert!(
        fs::read_to_string(root().join("benchmarks/silicon/scalar_timing.json"))
            .map(|t| t.contains("cycles"))
            .unwrap_or(false),
        "scalar_timing.json lost its measured on-chip cycle counts"
    );
}
