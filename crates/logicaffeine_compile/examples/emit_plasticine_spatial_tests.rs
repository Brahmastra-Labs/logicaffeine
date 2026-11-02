//! Write every canonical counted-loop workload as a Spatial REGRESSION-HARNESS app (`class Logos_<name>
//! extends SpatialTest`, 64-bit `Long`, the VM's answer as the gold) into a directory — the public
//! toolchain's `regression/` dir, so `gate.sh Logos_<name>` runs OUR loop through Spatial → PIR → plastiroute →
//! Tungsten. Also prints the ledger `add` invocation for each app. Usage:
//!   cargo run -p logicaffeine-compile --example emit_plasticine_spatial_tests -- <out_dir>

use logicaffeine_compile::codegen_sva::plasticine::PlasticineTarget;
use logicaffeine_compile::codegen_sva::plasticine_report::{
    canonical_workloads, lower_counted_loop, WorkloadKind,
};

fn main() {
    let out = std::env::args()
        .nth(1)
        .expect("usage: emit_plasticine_spatial_tests <out_dir>");
    std::fs::create_dir_all(&out).expect("out dir");
    let t = PlasticineTarget::sara_20x20();
    for w in canonical_workloads() {
        let WorkloadKind::CountedLoop {
            source,
            fn_name,
            params,
        } = &w.kind
        else {
            continue;
        };
        let class = format!("Logos_{}", w.name.replace('-', "_"));
        let mut art = lower_counted_loop(source, fn_name, params, &t)
            .unwrap_or_else(|e| panic!("{}: {e}", w.name));
        // The artifact names the class after the function; the regression dir needs one class per workload.
        art.spatial_test = art
            .spatial_test
            .replace(&format!("Logos_{fn_name}"), &class);
        let path = std::path::Path::new(&out).join(format!("{class}.scala"));
        std::fs::write(&path, &art.spatial_test)
            .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
        let src_path = std::path::Path::new(&out).join(format!("{class}.lg"));
        std::fs::write(&src_path, source).unwrap();
        println!(
            "{} → {} (iterations {}, modeled II {}, modeled cycles {})",
            w.name,
            path.display(),
            art.row.iterations,
            art.row.pipeline_ii,
            art.row.modeled_cycles
        );
        println!("  python3 scripts/accel/plasticine-gate-ledger.py add --kind counted_loop --app {class} --gen-dir target/plasticine-toolchain/spatial/gen/Tst_pirTest/{class} --logos-source {} --fn {fn_name} --params \"{}\" --toolchain \"…\"", src_path.display(), params.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(" "));
    }
}
