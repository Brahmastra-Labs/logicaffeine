//! Emit the BATCHED Qwen3-30B decode .cu (B parallel sequences) for the beat-llama batched lane.
//!   LOGOS_BATCH=32 cargo run -p logicaffeine-compile --example emit_qwen30b_batched -- /tmp/gen30b_batched.cu

use logicaffeine_compile::codegen_cuda::qwen_batched::emit_qwen30b_generate_batched;
use logicaffeine_compile::codegen_cuda::qwen_emit::Qwen30bCfg;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/gen30b_batched.cu".to_string());
    let batch: usize = std::env::var("LOGOS_BATCH")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(32);
    let wdir = std::env::var("LOGOS_Q30_WDIR").unwrap_or_else(|_| "/root/w".to_string());
    let cfg = Qwen30bCfg::real();
    let prompt = vec![785usize, 6722, 315, 9625, 374];
    let cu = emit_qwen30b_generate_batched(&cfg, &wdir, &prompt, 24, batch);
    std::fs::write(&out, &cu).unwrap();
    eprintln!(
        "wrote {out} ({} bytes, batch={batch}, wdir={wdir})",
        cu.len()
    );
}
