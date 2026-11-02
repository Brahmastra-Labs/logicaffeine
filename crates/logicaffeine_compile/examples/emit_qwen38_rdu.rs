//! Regenerate `benchmarks/silicon/qwen38_sn50.json` — Qwen3.8-27B on the SN50, MODELED: the weights by
//! storage, per-layer MACs and arithmetic intensity, the KV cache, the Gated-DeltaNet memory and conv rings,
//! the peak live set of activations, residency tiers, and tensor parallel over one SambaRack at the model's
//! full context — and `benchmarks/silicon/qwen38_vllm_plan.json`, the deployment plan the fit implies. Every
//! number derives from `Qwen38Cfg::real()`, the authored forward and the chip facts. The only writer of both;
//! `rdu_fit_report_lock` pins them byte-equal. Usage (from the repo root):
//!   cargo run -p logicaffeine-compile --example emit_qwen38_rdu [-- <fit_path> [<plan_path>]]

use logicaffeine_compile::codegen_cuda::qwen38::Qwen38Cfg;
use logicaffeine_compile::spatial::fit::{
    default_tensor_parallel, qwen38_fit, render, vllm_plan, QWEN38_FIT_PATH, VLLM_PLAN_PATH,
};
use logicaffeine_compile::spatial::rdu_chip::RduChipFacts;

fn main() {
    let fit_out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| QWEN38_FIT_PATH.to_string());
    let plan_out = std::env::args()
        .nth(2)
        .unwrap_or_else(|| VLLM_PLAN_PATH.to_string());
    let cfg = Qwen38Cfg::real();
    let facts = RduChipFacts::sn50();
    let tp = default_tensor_parallel(&facts);
    let fit = render(
        &qwen38_fit(&cfg, &facts, tp, cfg.ctx_len)
            .unwrap_or_else(|e| panic!("the Qwen3.8 fit on the {}: {e}", facts.part)),
    );
    std::fs::write(&fit_out, &fit).unwrap_or_else(|e| panic!("write {fit_out}: {e}"));
    eprintln!(
        "wrote {fit_out} ({} bytes; TP {tp}, ctx {}; provenance modeled)",
        fit.len(),
        cfg.ctx_len
    );
    let plan =
        render(&vllm_plan(&cfg, &facts, tp).unwrap_or_else(|e| panic!("the vLLM plan: {e}")));
    std::fs::write(&plan_out, &plan).unwrap_or_else(|e| panic!("write {plan_out}: {e}"));
    eprintln!(
        "wrote {plan_out} ({} bytes; provenance modeled)",
        plan.len()
    );
}
