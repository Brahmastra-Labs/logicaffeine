//! dump_fabric_grid — the PLACED-AND-ROUTED tile arrays the spatial fabric compiler actually produces, as JSON,
//! so a technical drawing of the fabric is generated from the placer's own output rather than drawn by hand.
//!
//!   cargo run -q -p logicaffeine-compile --example dump_fabric_grid > benchmarks/silicon/fabric_grids.json
//!
//! The data is `fabric_soc::fabric_grids_json` (the five `config_fabric` islands + the Horner cubic on both
//! placers); `fabric_grids_lock` pins the checked-in file to it, so the recorded grids can never show a placement
//! the compiler no longer produces.
use logicaffeine_compile::codegen_sva::fabric_soc::fabric_grids_json;

fn main() {
    println!(
        "{}",
        serde_json::to_string_pretty(&fabric_grids_json()).expect("json")
    );
}
