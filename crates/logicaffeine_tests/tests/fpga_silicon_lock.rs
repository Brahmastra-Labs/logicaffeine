//! fpga_silicon_lock — the REPRODUCIBLE physical proof: LOGOS bytecode runs on the real Artix-7.
//!
//! "Send the bytes down the wire and the tiled interpreter runs them." This lock is the board-gated
//! acceptance step of the spatial-interpreter campaign (Phase F): a `.lg` program is compiled to
//! register-bytecode chip words, streamed over USB-UART to a Nexys A7-100T (a Xilinx Artix-7 running the
//! soft-core interpreter bitstream, built entirely with the open flow — yosys + nextpnr-xilinx + prjxray,
//! no Vivado), executed on silicon, and its `Show` output MUST equal the VM golden (`lgvm`, the same
//! `vm_outcome` the whole suite locks against).
//!
//! BOARD-GATED: this runs only when `LOGOS_FPGA_BOARD=1` (a board on Board B, `/dev/ttyUSB3`, sudo
//! available). Everywhere else — CI, a dev box without hardware — it SKIPS cleanly, because the rest of
//! the campaign (P0-P5) already proves the fabric ≡ VM in sim/SAT without a simulator or a board. The
//! physical run is the final "it really works on the metal" acceptance.
//!
//!   LOGOS_FPGA_BOARD=1 cargo test -p logicaffeine-tests --test fpga_silicon_lock -- --nocapture
//!
//! Corpus note: on-chip arrays must fit the 2048-word soft-core heap; the 10000-element benchmark
//! variants overflow it (a documented resource limit, NOT a logic bug — they pass bit-exact at n<=~1000,
//! see scripts/validate-on-nexys-full.sh with a smaller n). This corpus is sized to fit.

use std::process::Command;

#[test]
fn logos_bytecode_runs_on_the_artix7_matching_the_vm() {
    if std::env::var("LOGOS_FPGA_BOARD").as_deref() != Ok("1") {
        eprintln!(
            "LOGOS_FPGA_BOARD != 1 — skipping the physical Artix-7 proof (needs a Nexys A7 on Board B). \
             Reproduce with: LOGOS_FPGA_BOARD=1 cargo test -p logicaffeine-tests --test fpga_silicon_lock"
        );
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // Heap-fitting programs spanning arithmetic, division, recursion, nested loops, collections (push /
    // index / setindex / length / `followed by`), and a full quicksort.
    let corpus = [
        "benchmarks/silicon/fpga/two_sum.lg",
        "benchmarks/silicon/fpga/quicksort.lg",
        "benchmarks/silicon/fpga/histogram.lg",
        "benchmarks/silicon/fpga/wire_primes.lg",
        "benchmarks/silicon/fpga/wire_lists.lg",
        "benchmarks/silicon/fpga/wire_fib.lg",
    ];
    // The script self-sudoes the board ops (openFPGALoader + the UART host); it re-flashes the bitstream
    // before each program (the loader latches at L_DONE) and diffs the on-chip Show output against `lgvm`.
    let status = Command::new("bash")
        .arg("scripts/validate-on-nexys-full.sh")
        .args(corpus)
        .current_dir(&root)
        .status()
        .expect("failed to launch scripts/validate-on-nexys-full.sh");
    assert!(
        status.success(),
        "silicon ≡ VM validation FAILED on the Artix-7 (see the per-program table above)"
    );
}
