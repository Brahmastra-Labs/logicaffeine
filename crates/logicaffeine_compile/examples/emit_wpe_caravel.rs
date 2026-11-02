//! Writes the whole-program engine's Caravel user project for the configuration `wpe_caravel::WPE_CARAVEL`
//! names: `benchmarks/silicon/wpe_user_project.v` (engine, caches, arbiter, PSRAM edge and the
//! `wpe_user_project` macro), `benchmarks/silicon/wpe_user_project_wrapper.v` (Caravel's `user_project_wrapper`
//! around it), and the emitted files of the submission overlay `benchmarks/silicon/caravel/wpe/` — the GPIOs'
//! power-on modes and the bring-up bench with its programs. Refuses a configuration whose PSRAM read window the
//! harness round trip does not fit. With `--psram-read-sta <lib> <netlist> <spef>` it prints the OpenSTA script
//! that closes the read window through a hardened macro instead.
//!
//! Run: `cargo run -p logicaffeine-compile --example emit_wpe_caravel`

use logicaffeine_compile::codegen_sva::caravel_project::WPE_PROJECT;
use logicaffeine_compile::codegen_sva::wpe_caravel::{
    emit_wpe_user_project_verilog, wpe_caravel_memory, wpe_caravel_overlay_files,
    wpe_psram_read_sta_tcl, WPE_CARAVEL, WPE_CARAVEL_BACKEND, WPE_CARAVEL_CAPS, WPE_CARAVEL_NAME,
};
use std::path::Path;

fn main() {
    // `--psram-read-sta <lib> <netlist> <spef>`: print the OpenSTA script that closes a PSRAM read through the
    // hardened macro (wpe_caravel::wpe_psram_read_sta_tcl), for scripts/check-wpe-psram-timing.sh.
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.first().map(String::as_str) == Some("--psram-read-sta") {
        let [lib, netlist, spef] = [1, 2, 3].map(|i| {
            a.get(i).cloned().unwrap_or_else(|| {
                eprintln!("usage: emit_wpe_caravel --psram-read-sta <lib> <netlist> <spef>");
                std::process::exit(2)
            })
        });
        print!(
            "{}",
            wpe_psram_read_sta_tcl(&WPE_CARAVEL, &wpe_caravel_memory(), &lib, &netlist, &spef)
        );
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let silicon = root.join("benchmarks/silicon");
    let memory = wpe_caravel_memory();
    WPE_CARAVEL
        .check_harness(&memory)
        .unwrap_or_else(|e| panic!("the shipped Caravel engine cannot be built: {e:?}"));
    let rtl = emit_wpe_user_project_verilog(
        WPE_CARAVEL_NAME,
        WPE_CARAVEL_CAPS,
        WPE_CARAVEL_BACKEND,
        &memory,
        &WPE_CARAVEL,
    )
    .unwrap_or_else(|e| panic!("the shipped Caravel engine does not wire: {e:?}"));
    std::fs::write(silicon.join(format!("{}.v", WPE_PROJECT.macro_module)), rtl)
        .expect("write the macro RTL");
    std::fs::write(
        silicon.join(WPE_PROJECT.wrapper_rtl),
        WPE_PROJECT.wrapper_verilog(),
    )
    .expect("write the wrapper RTL");
    let overlay = root.join(WPE_PROJECT.overlay);
    for (path, body) in wpe_caravel_overlay_files() {
        let at = overlay.join(&path);
        std::fs::create_dir_all(at.parent().expect("an overlay file has a directory"))
            .expect("create the overlay directory");
        std::fs::write(&at, body).unwrap_or_else(|e| panic!("write {}: {e}", at.display()));
    }
    println!(
        "wrote {}.v, {} and the overlay under {}",
        WPE_PROJECT.macro_module,
        WPE_PROJECT.wrapper_rtl,
        overlay.display()
    );
}
