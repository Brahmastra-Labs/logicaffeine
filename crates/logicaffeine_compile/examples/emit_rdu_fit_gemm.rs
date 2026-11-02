//! Regenerate `benchmarks/silicon/rdu_fit_gemm.json` — the int8 GEMM fit on the SN50, MODELED: the workload
//! in the chip's tiles, its arithmetic and working set against the SRAM, the VERIFIED program's census, the
//! certificate that rechecks, and the II view on the chip's unit model. The only writer; `rdu_fit_report_lock`
//! pins it byte-equal. Usage (from the repo root):
//!   cargo run -p logicaffeine-compile --example emit_rdu_fit_gemm [-- <path>]

use logicaffeine_compile::spatial::fit::{gemm_fit, gemm_fit_shape, render, GEMM_FIT_PATH};
use logicaffeine_compile::spatial::rdu_chip::RduChipFacts;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| GEMM_FIT_PATH.to_string());
    let facts = RduChipFacts::sn50();
    let (m, n, k) = gemm_fit_shape(&facts.spatial_target());
    let report = render(
        &gemm_fit(m, n, k, &facts)
            .unwrap_or_else(|e| panic!("the GEMM fit on the {}: {e}", facts.part)),
    );
    std::fs::write(&out, &report).unwrap_or_else(|e| panic!("write {out}: {e}"));
    eprintln!(
        "wrote {out} ({} bytes; {m}x{n}x{k} int8 on the {}; provenance modeled)",
        report.len(),
        facts.part
    );
}
