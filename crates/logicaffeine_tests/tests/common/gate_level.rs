//! Gate-level simulation of a synthesized netlist: yosys's generic Xilinx sim cells with its two block-RAM models
//! stripped, so `assets/ramb_behav.v` — every TDP width and SDP-72/36 over the device's own data and parity layout,
//! loud on what it does not implement — provides them.

#![allow(dead_code)]

/// The faithful 7-series block-RAM model (`RAMB18E1`, `RAMB36E1`).
pub const RAMB_BEHAV_V: &str = include_str!("../assets/ramb_behav.v");

/// yosys's `xilinx/cells_sim.v` with `RAMB18E1` and `RAMB36E1` cut away (the last two modules in it, contiguous),
/// or None when yosys's data directory is not where a distro or a source install keeps it.
pub fn cells_sim_without_ramb() -> Option<String> {
    for p in ["/usr/share/yosys/xilinx/cells_sim.v", "/usr/local/share/yosys/xilinx/cells_sim.v"] {
        if let Ok(full) = std::fs::read_to_string(p) {
            let mut out = String::with_capacity(full.len());
            for line in full.lines() {
                if line.starts_with("module RAMB18E1 (") {
                    return Some(out);
                }
                out.push_str(line);
                out.push('\n');
            }
            panic!("{p} has no `module RAMB18E1 (` to cut the block-RAM models at");
        }
    }
    None
}
